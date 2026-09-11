//! Compact panorama with a per-map native-vertex cache.
//!
//! A compute prepass evaluates the shared type-2 position and packed-map laws
//! for the complete 51 by 101 endpoint-preserving vertex grid. The following
//! compact panorama draw retains the original cell search, triangle test,
//! interpolation, alpha sampling, fusion coordinates and source sampling.

use std::num::NonZeroU64;

#[cfg(test)]
use super::nv12;
use crate::Fallible;
use crate::direct_type2::{
    DirectType2Pipeline, vertex_cache_prepass_wgsl, vertex_cached_draw_wgsl_with_fusion_mode,
};
use crate::studio_type2::{ALPHA_BYTES, PACKED_BYTES};
#[cfg(test)]
use crate::temporal_fusion::color::MatrixCoefficients;
#[cfg(test)]
use crate::{FrameStamp, Size};

const ROWS: u64 = 51;
const COLUMNS: u64 = 101;
const RECORD_BYTES: u64 = std::mem::size_of::<[f32; 8]>() as u64;
const CACHE_BYTES: u64 = ROWS * COLUMNS * RECORD_BYTES;
const WORKGROUP_SIZE: u32 = 64;
const VERTICES: u32 = (ROWS * COLUMNS) as u32;

const DIRECT_BODY_SELECTOR: &str = r#"
fn panorama_type2_mesh(body: vec3<f32>, sample_uv: vec2<f32>) -> Type2Sample {
  let ray = normalize(vec3<f32>(-body.x, body.y, -body.z));
  return type2_mesh_seeded(ray, type2_body_seed(ray, sample_uv));
}
"#;

pub(crate) struct Producer {
    device: wgpu::Device,
    prepass_layout: wgpu::BindGroupLayout,
    render_map_layout: wgpu::BindGroupLayout,
    prepass: wgpu::ComputePipeline,
    render: wgpu::RenderPipeline,
    fusion: bool,
}

/// One per-map cache and the binding which reads it during the following draw.
/// The buffer remains owned until both passes have been recorded.
pub(crate) struct Prepared {
    _cache: wgpu::Buffer,
    render_binding: wgpu::BindGroup,
}

impl Producer {
    pub(in crate::direct_type2) fn new(
        device: &wgpu::Device,
        direct: &DirectType2Pipeline,
    ) -> Fallible<Self> {
        if direct.device != *device {
            return Err(
                "vertex-cached compact NV12 producer belongs to a different graphics device".into(),
            );
        }

        let prepass_layout = cache_layout(device, wgpu::ShaderStages::COMPUTE, false);
        let render_map_layout = cache_layout(
            device,
            wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            true,
        );
        let prepass_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("type-2 native vertex cache prepass"),
            source: wgpu::ShaderSource::Wgsl(vertex_cache_prepass_wgsl().into()),
        });
        let prepass_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("type-2 native vertex cache prepass"),
                bind_group_layouts: &[&prepass_layout],
                immediate_size: 0,
            });
        let prepass = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("type-2 native vertex cache prepass"),
            layout: Some(&prepass_pipeline_layout),
            module: &prepass_module,
            entry_point: Some("cache_type2_vertices"),
            compilation_options: Default::default(),
            cache: None,
        });

        let fusion = direct.fusion_layout.is_some();
        let hardware_fusion = direct.fusion_sampler.is_some();
        let render_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vertex-cached compact NV12 body panorama"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{DIRECT_BODY_SELECTOR}\n{}",
                    vertex_cached_draw_wgsl_with_fusion_mode(fusion, hardware_fusion),
                    include_str!("nv12.wgsl")
                )
                .into(),
            ),
        });
        let mut layouts = vec![&direct.picture_layout, &render_map_layout];
        layouts.extend(direct.fusion_layout.as_ref());
        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("vertex-cached compact NV12 body panorama"),
                bind_group_layouts: &layouts,
                immediate_size: 0,
            });
        let render = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vertex-cached compact NV12 body panorama"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &render_module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &render_module,
                entry_point: Some("panorama_nv12_fs"),
                compilation_options: Default::default(),
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rg8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        Ok(Self {
            device: device.clone(),
            prepass_layout,
            render_map_layout,
            prepass,
            render,
            fusion,
        })
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(in crate::direct_type2) fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        picture: &wgpu::BindGroup,
        packed: &wgpu::Buffer,
        alpha: &wgpu::Buffer,
        fusion: Option<&wgpu::BindGroup>,
        output: &nv12::Producer,
        frame: FrameStamp,
        full: Size,
        coefficients: MatrixCoefficients,
    ) -> Fallible<super::nv12::CompactNv12Panorama> {
        if self.device != *device {
            return Err(
                "vertex-cached compact NV12 producer belongs to a different graphics device".into(),
            );
        }
        let prepared = output.prepare(device, frame, full, coefficients, fusion.is_some())?;
        if self.fusion != fusion.is_some() {
            return Err(
                "vertex-cached compact NV12 map and photometric binding presence differ".into(),
            );
        }

        let cached = self.prepare(device, encoder, packed, alpha)?;

        let [y_view, uv_view] = prepared.views();
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vertex-cached compact NV12 body panorama"),
                color_attachments: &[
                    Some(nv12::attachment(&y_view)),
                    Some(nv12::attachment(&uv_view)),
                ],
                ..Default::default()
            });
            self.draw(&mut pass, picture, &cached, fusion);
        }
        Ok(prepared.into_output())
    }

    /// Allocate and populate one complete native vertex grid. The caller owns
    /// the exact installed map buffers and records its compact draw afterward
    /// in this same encoder.
    pub(crate) fn prepare(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        packed: &wgpu::Buffer,
        alpha: &wgpu::Buffer,
    ) -> Fallible<Prepared> {
        if self.device != *device {
            return Err(
                "vertex-cached compact NV12 producer belongs to a different graphics device".into(),
            );
        }

        let cache = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("type-2 native vertex cache"),
            size: CACHE_BYTES,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let prepass_binding = bind_cache(
            device,
            &self.prepass_layout,
            "type-2 native vertex cache prepass",
            packed,
            alpha,
            &cache,
        );
        let render_binding = bind_cache(
            device,
            &self.render_map_layout,
            "vertex-cached compact NV12 map",
            packed,
            alpha,
            &cache,
        );

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("type-2 native vertex cache prepass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.prepass);
            pass.set_bind_group(0, &prepass_binding, &[]);
            pass.dispatch_workgroups(VERTICES.div_ceil(WORKGROUP_SIZE), 1, 1);
        }
        Ok(Prepared {
            _cache: cache,
            render_binding,
        })
    }

    pub(crate) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        picture: &wgpu::BindGroup,
        cached: &Prepared,
        fusion: Option<&wgpu::BindGroup>,
    ) {
        assert_eq!(
            self.fusion,
            fusion.is_some(),
            "vertex-cached compact NV12 map and photometric binding presence differ"
        );
        pass.set_pipeline(&self.render);
        pass.set_bind_group(0, picture, &[]);
        pass.set_bind_group(1, &cached.render_binding, &[]);
        if let Some(fusion) = fusion {
            pass.set_bind_group(2, fusion, &[]);
        }
        pass.draw(0..3, 0..1);
    }
}

fn cache_layout(
    device: &wgpu::Device,
    visibility: wgpu::ShaderStages,
    cache_read_only: bool,
) -> wgpu::BindGroupLayout {
    let entry = |binding, bytes, read_only| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(bytes),
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("type-2 vertex-cached map resources"),
        entries: &[
            entry(0, PACKED_BYTES as u64, true),
            entry(1, ALPHA_BYTES as u64, true),
            entry(2, CACHE_BYTES, cache_read_only),
        ],
    })
}

fn bind_cache(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    label: &'static str,
    packed: &wgpu::Buffer,
    alpha: &wgpu::Buffer,
    cache: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: packed.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: alpha.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: cache.as_entire_binding(),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

    #[test]
    fn vertex_cache_has_the_bounded_native_grid_size() {
        assert_eq!(CACHE_BYTES, 164_832);
    }

    #[test]
    fn vertex_cache_shaders_validate_and_render_uses_at_most_three_groups() {
        let prepass = wgpu::naga::front::wgsl::parse_str(&vertex_cache_prepass_wgsl()).unwrap();
        Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&prepass)
            .unwrap();

        for (fusion, hardware) in [(false, false), (true, false), (true, true)] {
            let source = format!(
                "{}\n{DIRECT_BODY_SELECTOR}\n{}",
                vertex_cached_draw_wgsl_with_fusion_mode(fusion, hardware),
                include_str!("nv12.wgsl")
            );
            let module = wgpu::naga::front::wgsl::parse_str(&source).unwrap();
            Validator::new(ValidationFlags::all(), Capabilities::all())
                .validate(&module)
                .unwrap();
            let group_count = module
                .global_variables
                .iter()
                .filter_map(|(_, global)| global.binding.as_ref().map(|binding| binding.group))
                .max()
                .map_or(0, |maximum| maximum + 1);
            assert!(group_count <= 3, "shader uses {group_count} bind groups");
        }
    }

    #[test]
    fn vertex_cached_pipeline_fits_native_three_bind_group_device() {
        let (device, _) = match crate::direct_type2::tests::gpu() {
            Ok(gpu) => gpu,
            Err(error) => {
                assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none(), "{error}");
                eprintln!("skipping vertex-cache native-limit test: {error}");
                return;
            }
        };
        assert_eq!(device.limits().max_bind_groups, 3);
        let picture = crate::scene::bind_group_layout(&device);
        for fusion in [false, true] {
            let direct = DirectType2Pipeline::with_fusion(
                &device,
                &picture,
                wgpu::TextureFormat::Rgba8Unorm,
                fusion,
            );
            Producer::new(&device, &direct).unwrap();
        }
    }

    #[test]
    fn direct_body_seed_matches_inverse_at_every_compact_sample() {
        let (device, queue) = match crate::direct_type2::tests::gpu() {
            Ok(gpu) => gpu,
            Err(error) => {
                assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none(), "{error}");
                eprintln!("skipping direct body seed test: {error}");
                return;
            }
        };

        let control = Size::new(64, 32);
        assert_eq!(
            direct_seed_mismatches(&device, &queue, control, true),
            control.width * control.height,
            "the injected mismatch must count every rasterized quartet sample"
        );
        let mut all_match = true;
        for full in [Size::new(7680, 3840), Size::new(5760, 2880)] {
            let mismatches = direct_seed_mismatches(&device, &queue, full, false);
            eprintln!("body seed {full:?}: {mismatches} mismatches");
            all_match &= mismatches == 0;
        }
        assert!(all_match, "direct and inverse seeds differ");
    }

    fn direct_seed_mismatches(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        full: Size,
        force_mismatch: bool,
    ) -> u32 {
        let source = format!(
            "{}\n{}",
            crate::direct_type2::map_wgsl(),
            seed_comparison_wgsl(full, force_mismatch)
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("type-2 direct body seed comparison"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let counter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("type-2 seed mismatch counter layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(28),
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("type-2 seed comparison pipeline layout"),
            bind_group_layouts: &[&counter_layout],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("type-2 direct body seed comparison"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("seed_vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("seed_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let counter = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("type-2 seed mismatch counter"),
            size: 28,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        });
        counter.slice(..).get_mapped_range_mut().copy_from_slice(
            &[0u32, 0, 0, u32::MAX, u32::MAX, 0, 0]
                .into_iter()
                .flat_map(u32::to_ne_bytes)
                .collect::<Vec<_>>(),
        );
        counter.unmap();
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("type-2 seed mismatch readback"),
            size: 28,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let counter_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("type-2 seed mismatch counter"),
            layout: &counter_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: counter.as_entire_binding(),
            }],
        });
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("type-2 seed comparison raster"),
            size: wgpu::Extent3d {
                width: full.width / 2,
                height: full.height / 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let target_view = target.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("type-2 direct body seed comparison"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("type-2 direct body seed comparison"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &counter_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_buffer_to_buffer(&counter, 0, &readback, 0, 28);
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = readback.slice(..).get_mapped_range();
        let mismatches = u32::from_ne_bytes(mapped[..4].try_into().unwrap());
        let counts: Vec<_> = mapped
            .chunks_exact(4)
            .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
            .collect();
        eprintln!(
            "seed detail {full:?} forced={force_mismatch}: count={}, rows={}, cols={}, min=({},{}), max=({},{})",
            counts[0], counts[1], counts[2], counts[3], counts[4], counts[5], counts[6]
        );
        drop(mapped);
        readback.unmap();
        mismatches
    }

    fn seed_comparison_wgsl(full: Size, force_mismatch: bool) -> String {
        format!(
            r#"
struct SeedVsOut {{
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
}}

struct SeedMismatchCounter {{
  value: atomic<u32>, rows: atomic<u32>, cols: atomic<u32>,
  min_x: atomic<u32>, min_y: atomic<u32>, max_x: atomic<u32>, max_y: atomic<u32>,
}}
@group(0) @binding(0) var<storage, read_write> seed_mismatches: SeedMismatchCounter;

@vertex
fn seed_vs(@builtin(vertex_index) index: u32) -> SeedVsOut {{
  let x = f32((index << 1u) & 2u);
  let y = f32(index & 2u);
  var out: SeedVsOut;
  out.uv = vec2<f32>(x, y);
  out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  return out;
}}

fn compare_seed(sample_uv: vec2<f32>) {{
  let phi = TYPE2_PI * sample_uv.y;
  let theta = TYPE2_TAU * sample_uv.x;
  let body = vec3<f32>(sin(phi) * sin(theta), cos(phi),
    -sin(phi) * cos(theta));
  let ray = normalize(vec3<f32>(-body.x, body.y, -body.z));
  let different = type2_inverse_seed(ray) != type2_body_seed(ray, sample_uv);
  if {force_mismatch} || any(different) {{
    atomicAdd(&seed_mismatches.value, 1u);
    if different.x {{ atomicAdd(&seed_mismatches.rows, 1u); }}
    if different.y {{ atomicAdd(&seed_mismatches.cols, 1u); }}
    let pixel = vec2<u32>(sample_uv * vec2<f32>({}.0, {}.0));
    atomicMin(&seed_mismatches.min_x, pixel.x);
    atomicMin(&seed_mismatches.min_y, pixel.y);
    atomicMax(&seed_mismatches.max_x, pixel.x);
    atomicMax(&seed_mismatches.max_y, pixel.y);
  }}
}}

@fragment
fn seed_fs(in: SeedVsOut) -> @location(0) f32 {{
  let full_size = vec2<f32>({}.0, {}.0);
  let half_texel = vec2<f32>(0.5) / full_size;
  compare_seed(in.uv + vec2<f32>(-half_texel.x, -half_texel.y));
  compare_seed(in.uv + vec2<f32>( half_texel.x, -half_texel.y));
  compare_seed(in.uv + vec2<f32>(-half_texel.x,  half_texel.y));
  compare_seed(in.uv + vec2<f32>( half_texel.x,  half_texel.y));
  return 0.0;
}}
"#,
            full.width, full.height, full.width, full.height
        )
    }
}
