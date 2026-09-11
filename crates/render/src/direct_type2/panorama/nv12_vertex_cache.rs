//! Test-only compact panorama with a per-map native-vertex cache.
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
                    "{}\n{}",
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
                "{}\n{}",
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
}
