//! Offline compositor for reviewing a low-resolution temporal correction.
//!
//! This deliberately changes the full-resolution filtering semantics. The two
//! low-resolution inputs are caller-supplied gamma-RGB panoramas from the same
//! RGB-to-NV12-to-RGB round trip: `low_current` is the unfiltered control and
//! `low_filtered` is the existing temporal result. No conversion, source
//! association, timestamp ownership, interpolation between updates or gradual
//! colour policy is hidden in this stateless test primitive.

use wgpu::util::DeviceExt;

use super::{FORMAT, PROJECT_WGSL};
use crate::{Extent, Fallible, Reframe, Size};

/// Add a low-resolution gamma-RGB temporal residual to an exact directly
/// projected high-resolution viewport for offline visual comparison.
pub(crate) struct CorrectionReview {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

impl CorrectionReview {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("offline low-resolution temporal correction"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<Reframe>() as u64
                        ),
                    },
                    count: None,
                },
                texture_layout(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                texture_layout(3),
                texture_layout(4),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("offline low-resolution temporal correction"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        // Reuse the projector's view ray and body-panorama UV functions. This
        // review must not grow a second convention for the same camera view.
        let source = format!(
            "{}\n{}\n{}",
            crate::projection::wgsl(),
            PROJECT_WGSL,
            CORRECTION_WGSL
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("offline low-resolution temporal correction"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("offline low-resolution temporal correction"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("panorama_project_vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("correction_review_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("periodic low-resolution correction sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Self {
            device: device.clone(),
            layout,
            pipeline,
            sampler,
        }
    }

    /// Encode one review viewport. The caller retains and authenticates the
    /// exact source/map/Reframe association for all three inputs.
    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        high_direct: &wgpu::Texture,
        low_current: &wgpu::Texture,
        low_filtered: &wgpu::Texture,
        reframe: &Reframe,
    ) -> Fallible<wgpu::Texture> {
        if self.device != *device {
            return Err("temporal correction review belongs to a different graphics device".into());
        }
        if reframe.linearizes_output() {
            return Err("temporal correction review requires gamma RGB inputs".into());
        }
        let output_size = validate_viewport(high_direct)?;
        let low_size = validate_low_panorama(low_current, "current")?;
        if validate_low_panorama(low_filtered, "filtered")? != low_size {
            return Err("temporal correction review low panoramas have different geometry".into());
        }

        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("offline temporal correction Reframe"),
            contents: reframe.bytes(),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let high = high_direct.create_view(&Default::default());
        let current = low_current.create_view(&Default::default());
        let filtered = low_filtered.create_view(&Default::default());
        let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("offline low-resolution temporal correction"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&high),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&current),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&filtered),
                },
            ],
        });
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offline temporally corrected viewport"),
            size: output_size.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target = output.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("offline low-resolution temporal correction"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &binding, &[]);
            pass.draw(0..3, 0..1);
        }
        Ok(output)
    }
}

fn texture_layout(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn validate_texture(texture: &wgpu::Texture, domain: &str) -> Fallible<Size> {
    if texture.format() != FORMAT
        || texture.dimension() != wgpu::TextureDimension::D2
        || texture.depth_or_array_layers() != 1
        || texture.mip_level_count() != 1
        || texture.sample_count() != 1
        || !texture
            .usage()
            .contains(wgpu::TextureUsages::TEXTURE_BINDING)
    {
        return Err(format!(
            "temporal correction review {domain} must be a single-sampled sampleable 2D Rgba8Unorm texture"
        )
        .into());
    }
    Ok(Size::new(texture.width(), texture.height()))
}

fn validate_viewport(texture: &wgpu::Texture) -> Fallible<Size> {
    let size = validate_texture(texture, "high direct viewport")?;
    if size.width == 0 || size.height == 0 {
        return Err("temporal correction review viewport must be nonzero".into());
    }
    Ok(size)
}

fn validate_low_panorama(texture: &wgpu::Texture, role: &str) -> Fallible<Size> {
    let size = validate_texture(texture, &format!("low {role} panorama"))?;
    if size.width == 0 || size.height == 0 || size.width != size.height.saturating_mul(2) {
        return Err(format!(
            "temporal correction review low {role} panorama must be nonzero 2:1, got {} by {}",
            size.width, size.height
        )
        .into());
    }
    Ok(size)
}

const CORRECTION_WGSL: &str = r#"
@group(0) @binding(3) var low_current: texture_2d<f32>;
@group(0) @binding(4) var low_filtered: texture_2d<f32>;

@fragment
fn correction_review_fs(in: PanoramaProjectVsOut) -> @location(0) vec4<f32> {
  // `panorama` is already the exact high-resolution projected viewport. Do
  // not resample it or repeat the source/map projection in this experiment.
  let high = textureLoad(panorama, vec2<i32>(in.position.xy), 0).rgb;
  let view = view_ray(in.uv);
  if view.w <= 0.0 { return vec4<f32>(high, 1.0); }
  let uv = body_panorama_uv(reframe.view_to_body * view.xyz);
  let current = textureSample(low_current, panorama_sampler, uv).rgb;
  let filtered = textureSample(low_filtered, panorama_sampler, uv).rgb;
  return vec4<f32>(clamp(high + (filtered - current), vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_low_resolution_correction_preserves_direct_viewport_exactly() {
        let Ok((device, queue)) = crate::direct_type2::tests::gpu() else {
            eprintln!("skipping correction review test without a Vulkan adapter");
            return;
        };
        let viewport = Size::new(64, 32);
        let low = Size::new(8, 4);
        let high_bytes: Vec<u8> = (0..viewport.height)
            .flat_map(|y| {
                (0..viewport.width).flat_map(move |x| {
                    [
                        x.wrapping_mul(11).wrapping_add(y.wrapping_mul(3)) as u8,
                        x.wrapping_mul(5).wrapping_add(y.wrapping_mul(17)) as u8,
                        (x ^ y).wrapping_mul(7) as u8,
                        255,
                    ]
                })
            })
            .collect();
        let low_bytes: Vec<u8> = (0..low.height)
            .flat_map(|y| {
                (0..low.width).flat_map(move |x| {
                    [
                        x.wrapping_mul(29).wrapping_add(y.wrapping_mul(7)) as u8,
                        x.wrapping_mul(13).wrapping_add(y.wrapping_mul(31)) as u8,
                        (x ^ y).wrapping_mul(37) as u8,
                        255,
                    ]
                })
            })
            .collect();
        let actual = render(&device, &queue, &high_bytes, &low_bytes, &low_bytes);
        assert_eq!(actual, high_bytes);
    }

    #[test]
    fn signed_low_resolution_correction_adds_subtracts_and_clamps() {
        let Ok((device, queue)) = crate::direct_type2::tests::gpu() else {
            eprintln!("skipping signed correction review test without a Vulkan adapter");
            return;
        };
        let high = [240, 16, 80, 255]
            .into_iter()
            .cycle()
            .take(64 * 32 * 4)
            .collect::<Vec<_>>();
        let current = [64, 128, 192, 255]
            .into_iter()
            .cycle()
            .take(8 * 4 * 4)
            .collect::<Vec<_>>();
        let filtered = [96, 96, 255, 255]
            .into_iter()
            .cycle()
            .take(8 * 4 * 4)
            .collect::<Vec<_>>();
        let expected = [255, 0, 143, 255]
            .into_iter()
            .cycle()
            .take(high.len())
            .collect::<Vec<_>>();
        assert_eq!(
            render(&device, &queue, &high, &current, &filtered),
            expected
        );
    }

    fn render(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        high_bytes: &[u8],
        current_bytes: &[u8],
        filtered_bytes: &[u8],
    ) -> Vec<u8> {
        let viewport = Size::new(64, 32);
        let low = Size::new(8, 4);
        let high = upload(device, queue, viewport, high_bytes, "direct viewport");
        let current = upload(device, queue, low, current_bytes, "low current round trip");
        let filtered = upload(
            device,
            queue,
            low,
            filtered_bytes,
            "low filtered round trip",
        );
        let review = CorrectionReview::new(device);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let output = review
            .encode(
                device,
                &mut encoder,
                &high,
                &current,
                &filtered,
                &Reframe::blank(viewport.width as f32 / viewport.height as f32, false),
            )
            .unwrap();
        let copy = copy_for_readback(device, &mut encoder, &output, viewport);
        queue.submit([encoder.finish()]);
        read_copy(device, &copy)
    }

    fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: Size,
        bytes: &[u8],
        label: &str,
    ) -> wgpu::Texture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: size.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.width * 4),
                rows_per_image: Some(size.height),
            },
            size.extent(),
        );
        texture
    }

    fn copy_for_readback(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        size: Size,
    ) -> wgpu::Buffer {
        let bytes_per_row = size.width * 4;
        assert!(bytes_per_row.is_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT));
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offline temporal correction readback"),
            size: u64::from(bytes_per_row) * u64::from(size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(size.height),
                },
            },
            size.extent(),
        );
        buffer
    }

    fn read_copy(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u8> {
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = buffer.slice(..).get_mapped_range().to_vec();
        buffer.unmap();
        bytes
    }
}
