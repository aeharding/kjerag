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

pub(crate) struct Producer {
    device: wgpu::Device,
    workspace: wgpu::Buffer,
    prepass_layout: wgpu::BindGroupLayout,
    render_map_layout: wgpu::BindGroupLayout,
    prepass: wgpu::ComputePipeline,
    render: wgpu::RenderPipeline,
    fusion: bool,
}

/// One per-map binding over the producer's reusable workspace.
///
/// Its prepass and draw must stay contiguous in one encoder. A later
/// preparation may overwrite the workspace only after the preceding draw;
/// submissions on the producer's one device queue retain that ordering.
pub(crate) struct Prepared {
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
        let workspace = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("type-2 native vertex workspace"),
            size: CACHE_BYTES,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        Ok(Self {
            device: device.clone(),
            workspace,
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

        let prepass_binding = bind_cache(
            device,
            &self.prepass_layout,
            "type-2 native vertex cache prepass",
            packed,
            alpha,
            &self.workspace,
        );
        let render_binding = bind_cache(
            device,
            &self.render_map_layout,
            "vertex-cached compact NV12 map",
            packed,
            alpha,
            &self.workspace,
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
        Ok(Prepared { render_binding })
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

    mod workspace_reuse {
        use super::*;
        use std::time::Duration;
        use wgpu::util::DeviceExt;

        #[test]
        fn reused_workspace_keeps_two_pending_maps_distinct() {
            let (device, queue) = match crate::direct_type2::tests::gpu() {
                Ok(gpu) => gpu,
                Err(error) => {
                    assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none(), "{error}");
                    eprintln!("skipping vertex-cache workspace reuse test: {error}");
                    return;
                }
            };
            let picture_layout = crate::scene::bind_group_layout(&device);
            let direct =
                DirectType2Pipeline::new(&device, &picture_layout, wgpu::TextureFormat::Rgba8Unorm);
            let producer = direct.vertex_cached_compact_nv12();
            let output = direct.compact_nv12();
            let source_size = Size::new(2, 2);
            let full = Size::new(4, 2);
            let reframe = crate::Reframe::new(
                &[],
                source_size,
                crate::Camera::default(),
                crate::Held::default(),
                2.0,
                false,
                crate::Sampling::Bilinear,
            )
            .with_samples(kjerag_media::Samples {
                wide: false,
                limited: false,
                matrix: kjerag_media::ColorMatrix::Bt709,
            });
            let coefficients = MatrixCoefficients::from_source_rgb(reframe.source_color_matrix());
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vertex-cache workspace test Reframe"),
                contents: reframe.bytes(),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let planes = test_planes(&device, &queue);
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                min_filter: wgpu::FilterMode::Linear,
                mag_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            let picture = crate::direct_type2::bind_picture(
                &device,
                &picture_layout,
                &uniform,
                [&planes[0], &planes[1]],
                &sampler,
            );
            let alpha_values = vec![1.0f32; crate::studio_type2::MAP_NODES];
            let alpha = test_buffer(
                &device,
                &queue,
                "vertex-cache workspace alpha",
                &float_bytes(&alpha_values),
            );
            let maps = [
                vec![[0.25f32, 0.5, 0.25, 0.5]; crate::studio_type2::MAP_NODES],
                vec![[0.75f32, 0.5, 0.75, 0.5]; crate::studio_type2::MAP_NODES],
            ];
            let packed = maps.each_ref().map(|map| {
                test_buffer(
                    &device,
                    &queue,
                    "vertex-cache workspace packed map",
                    &float4_bytes(map),
                )
            });
            let first = crate::FrameStamp::for_test(0, Duration::ZERO, None);
            let second = crate::FrameStamp::for_test(1, Duration::from_millis(1), Some(&first));

            let isolated = [(&packed[0], &first), (&packed[1], &second)].map(|(map, frame)| {
                let mut encoder = device.create_command_encoder(&Default::default());
                let panorama = encode_test_panorama(
                    producer,
                    output,
                    &device,
                    &mut encoder,
                    &picture,
                    map,
                    &alpha,
                    frame.clone(),
                    full,
                    coefficients,
                );
                let copies = copy_panorama(&device, &mut encoder, &panorama);
                let submission = queue.submit([encoder.finish()]);
                read_panorama(&device, copies, submission)
            });
            assert_ne!(
                isolated[0], isolated[1],
                "the two map fixtures do not distinguish workspace contamination"
            );

            let mut encoder = device.create_command_encoder(&Default::default());
            let pending_a = encode_test_panorama(
                producer,
                output,
                &device,
                &mut encoder,
                &picture,
                &packed[0],
                &alpha,
                first,
                full,
                coefficients,
            );
            let copies_a = copy_panorama(&device, &mut encoder, &pending_a);
            let pending_b = encode_test_panorama(
                producer,
                output,
                &device,
                &mut encoder,
                &picture,
                &packed[1],
                &alpha,
                second,
                full,
                coefficients,
            );
            let copies_b = copy_panorama(&device, &mut encoder, &pending_b);
            let submission = queue.submit([encoder.finish()]);
            let together_a = read_panorama(&device, copies_a, submission.clone());
            let together_b = read_panorama(&device, copies_b, submission);
            assert_eq!(together_a, isolated[0]);
            assert_eq!(together_b, isolated[1]);
            assert_ne!(
                together_a, isolated[1],
                "first pending draw was contaminated by the second map"
            );
            assert_ne!(
                together_b, isolated[0],
                "second pending draw retained the first map"
            );
        }

        #[allow(clippy::too_many_arguments)]
        fn encode_test_panorama(
            producer: &Producer,
            output: &nv12::Producer,
            device: &wgpu::Device,
            encoder: &mut wgpu::CommandEncoder,
            picture: &wgpu::BindGroup,
            packed: &wgpu::Buffer,
            alpha: &wgpu::Buffer,
            frame: FrameStamp,
            full: Size,
            coefficients: MatrixCoefficients,
        ) -> nv12::CompactNv12Panorama {
            let output = output
                .prepare(device, frame, full, coefficients, false)
                .unwrap();
            let cached = producer.prepare(device, encoder, packed, alpha).unwrap();
            let [y, uv] = output.views();
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("vertex-cache workspace test panorama"),
                    color_attachments: &[Some(nv12::attachment(&y)), Some(nv12::attachment(&uv))],
                    ..Default::default()
                });
                producer.draw(&mut pass, picture, &cached, None);
            }
            output.into_output()
        }

        fn test_planes(device: &wgpu::Device, queue: &wgpu::Queue) -> [crate::Planes; 2] {
            [([32], [80, 160]), ([220], [180, 40])].map(|(luma, chroma)| crate::Planes {
                luma: test_texture(
                    device,
                    queue,
                    "vertex-cache workspace luma",
                    wgpu::TextureFormat::R8Unorm,
                    &luma,
                ),
                chroma: test_texture(
                    device,
                    queue,
                    "vertex-cache workspace chroma",
                    wgpu::TextureFormat::Rg8Unorm,
                    &chroma,
                ),
            })
        }

        fn test_texture(
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            label: &'static str,
            format: wgpu::TextureFormat,
            bytes: &[u8],
        ) -> wgpu::Texture {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes.len() as u32),
                    rows_per_image: Some(1),
                },
                texture.size(),
            );
            texture
        }

        fn test_buffer(
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            label: &'static str,
            bytes: &[u8],
        ) -> wgpu::Buffer {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes.len() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, bytes);
            buffer
        }

        fn float_bytes(values: &[f32]) -> Vec<u8> {
            values
                .iter()
                .flat_map(|value| value.to_ne_bytes())
                .collect()
        }

        fn float4_bytes(values: &[[f32; 4]]) -> Vec<u8> {
            values
                .iter()
                .flatten()
                .flat_map(|value| value.to_ne_bytes())
                .collect()
        }

        struct Copies {
            y: Copy,
            uv: Copy,
        }

        #[derive(Debug, PartialEq, Eq)]
        struct PanoramaBytes {
            y: Vec<u8>,
            uv: Vec<u8>,
        }

        fn copy_panorama(
            device: &wgpu::Device,
            encoder: &mut wgpu::CommandEncoder,
            panorama: &nv12::CompactNv12Panorama,
        ) -> Copies {
            Copies {
                y: Copy::encode(device, encoder, panorama.packed_y(), 4),
                uv: Copy::encode(device, encoder, panorama.uv(), 2),
            }
        }

        fn read_panorama(
            device: &wgpu::Device,
            copies: Copies,
            submission: wgpu::SubmissionIndex,
        ) -> PanoramaBytes {
            PanoramaBytes {
                y: copies.y.read(device, submission.clone()),
                uv: copies.uv.read(device, submission),
            }
        }

        struct Copy {
            buffer: wgpu::Buffer,
            packed_row: u32,
            padded_row: u32,
            height: u32,
        }

        impl Copy {
            fn encode(
                device: &wgpu::Device,
                encoder: &mut wgpu::CommandEncoder,
                texture: &wgpu::Texture,
                bytes_per_pixel: u32,
            ) -> Self {
                let packed_row = texture.width() * bytes_per_pixel;
                let padded_row = packed_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                    * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
                let height = texture.height();
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("vertex-cache workspace readback"),
                    size: u64::from(padded_row) * u64::from(height),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                encoder.copy_texture_to_buffer(
                    texture.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(padded_row),
                            rows_per_image: Some(height),
                        },
                    },
                    texture.size(),
                );
                Self {
                    buffer,
                    packed_row,
                    padded_row,
                    height,
                }
            }

            fn read(self, device: &wgpu::Device, submission: wgpu::SubmissionIndex) -> Vec<u8> {
                self.buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(submission),
                        timeout: None,
                    })
                    .unwrap();
                let mapped = self.buffer.slice(..).get_mapped_range();
                let mut bytes = Vec::with_capacity((self.packed_row * self.height) as usize);
                for row in mapped.chunks_exact(self.padded_row as usize) {
                    bytes.extend_from_slice(&row[..self.packed_row as usize]);
                }
                drop(mapped);
                self.buffer.unmap();
                bytes
            }
        }
    }
}
