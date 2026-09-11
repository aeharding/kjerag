//! GPU-owned copy of one exact imported lens pair.
//!
//! Imported textures alias decoder surfaces and therefore cannot outlive their
//! `Frames`. This module samples each plane once into ordinary wgpu textures.
//! The sealed result keeps the exact decoded source stamp and graphics
//! context, but no decoder owner or resident production carrier.

use kjerag_media::{Samples, Size};

use super::{DirectType2Pipeline, ImportedOneXsPicture};
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
#[cfg(test)]
use crate::flow::one_xs::one_xs_belt_gpu::ResidentSourceIdentity;
use crate::{Extent, Fallible, FrameStamp, Planes};

const COPY_SHADER: &str = r#"
struct VertexOutput {
  @builtin(position) position: vec4<f32>,
}

@group(0) @binding(0) var source_a: texture_2d<f32>;
@group(0) @binding(1) var source_b: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) vertex: u32) -> VertexOutput {
  let points = array(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>( 3.0, -1.0),
    vec2<f32>(-1.0,  3.0),
  );
  var out: VertexOutput;
  out.position = vec4<f32>(points[vertex], 0.0, 1.0);
  return out;
}

struct LumaOutput {
  @location(0) a: f32,
  @location(1) b: f32,
}

@fragment
fn luma_fs(in: VertexOutput) -> LumaOutput {
  let at = vec2<i32>(in.position.xy);
  var out: LumaOutput;
  out.a = textureLoad(source_a, at, 0).r;
  out.b = textureLoad(source_b, at, 0).r;
  return out;
}

struct ChromaOutput {
  @location(0) a: vec2<f32>,
  @location(1) b: vec2<f32>,
}

@fragment
fn chroma_fs(in: VertexOutput) -> ChromaOutput {
  let at = vec2<i32>(in.position.xy);
  var out: ChromaOutput;
  out.a = textureLoad(source_a, at, 0).rg;
  out.b = textureLoad(source_b, at, 0).rg;
  return out;
}
"#;

/// Cached copier shared by every source snapshot made through one direct
/// pipeline. It records commands only; submission and retirement stay with the
/// installed resident transaction.
pub(super) struct SnapshotPipeline {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    luma: wgpu::RenderPipeline,
    chroma: wgpu::RenderPipeline,
}

impl SnapshotPipeline {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("resident source snapshot inputs"),
            entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("resident source snapshot"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("resident source snapshot"),
            source: wgpu::ShaderSource::Wgsl(COPY_SHADER.into()),
        });
        let make = |label, entry_point, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry_point),
                    compilation_options: Default::default(),
                    targets: &[0, 1].map(|_| {
                        Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })
                    }),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            device: device.clone(),
            layout,
            luma: make(
                "resident luma source snapshot",
                "luma_fs",
                wgpu::TextureFormat::R8Unorm,
            ),
            chroma: make(
                "resident chroma source snapshot",
                "chroma_fs",
                wgpu::TextureFormat::Rg8Unorm,
            ),
        }
    }

    fn encode_pair(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        sources: [&wgpu::Texture; 2],
        targets: [&wgpu::Texture; 2],
        label: &'static str,
    ) {
        let source_views = sources.map(|texture| texture.create_view(&Default::default()));
        let target_views = targets.map(|texture| texture.create_view(&Default::default()));
        let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source_views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&source_views[1]),
                },
            ],
        });
        let attachments = target_views.each_ref().map(|view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &attachments,
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &binding, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// One immutable, GPU-owned pair of lens planes and its unforgeable source
/// association. Plane handles never cross this owner boundary.
pub(crate) struct SourceSnapshot {
    planes: [Planes; 2],
    frame: FrameStamp,
    context: OneXsGpuContext,
    size: Size,
    samples: Samples,
    source_matrix: [f32; 4],
}

impl SourceSnapshot {
    pub(crate) fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub(crate) fn ensure_context(&self, expected: &OneXsGpuContext) -> Fallible<()> {
        self.context.ensure_same(expected)
    }

    pub(crate) fn source_size(&self) -> [f32; 2] {
        [self.size.width as f32, self.size.height as f32]
    }

    pub(crate) fn source_matrix(&self) -> [f32; 4] {
        self.source_matrix
    }

    pub(crate) fn prepare_correction_picture(
        &self,
        pipeline: &super::correction::CorrectionPipeline,
        reframe: &crate::Reframe,
        correction: &crate::temporal_fusion::correction_stream::CorrectionFrame,
    ) -> Fallible<super::correction::CorrectionPictureBinding> {
        if &self.frame != correction.frame() {
            return Err("temporal correction names a different source snapshot".into());
        }
        if !correction.belongs_to(self.context.device()) {
            return Err("temporal correction belongs to a different graphics device".into());
        }
        let reframe = self.exact_reframe(reframe)?;
        pipeline.prepare_picture(&reframe, [&self.planes[0], &self.planes[1]], correction)
    }

    fn exact_reframe(&self, reframe: &crate::Reframe) -> Fallible<crate::Reframe> {
        let expected_size = self.source_size();
        if reframe.frame_size() != expected_size {
            let actual = reframe.frame_size();
            return Err(format!(
                "resident source snapshot reframe names {} by {} pixels, expected {} by {}",
                actual[0], actual[1], self.size.width, self.size.height
            )
            .into());
        }
        let expected_matrix = self.source_matrix();
        if reframe.source_color_matrix() != expected_matrix {
            return Err(
                "resident source snapshot reframe has different source colour coefficients".into(),
            );
        }
        let reframe = (*reframe).with_samples(self.samples);
        Ok(reframe)
    }
}

fn source_matrix(size: Size, samples: Samples) -> [f32; 4] {
    crate::Reframe::new(
        &[],
        size,
        crate::Camera::default(),
        crate::Held::default(),
        1.0,
        false,
        crate::Sampling::Bilinear,
    )
    .with_samples(samples)
    .source_color_matrix()
}

pub(super) fn encode(
    source: &ImportedOneXsPicture,
    producer: &DirectType2Pipeline,
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
) -> Fallible<SourceSnapshot> {
    producer.ensure_device(&source.context)?;
    if source.context.device() != device {
        return Err("resident source snapshot belongs to a different graphics device".into());
    }
    if source.frames.samples.wide {
        return Err("resident source snapshot requires 8-bit R8/Rg8 source planes".into());
    }
    let size = source.frames.size;
    if size.width == 0
        || size.height == 0
        || !size.width.is_multiple_of(2)
        || !size.height.is_multiple_of(2)
    {
        return Err(format!(
            "resident source snapshot needs positive even dimensions, got {} by {}",
            size.width, size.height
        )
        .into());
    }
    for (lens, planes) in source.planes.iter().enumerate() {
        validate_plane(
            &planes.luma,
            size,
            wgpu::TextureFormat::R8Unorm,
            lens,
            "luma",
        )?;
        validate_plane(
            &planes.chroma,
            size.halved(),
            wgpu::TextureFormat::Rg8Unorm,
            lens,
            "chroma",
        )?;
    }

    let make = |label, size: Size, format| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: size.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | if cfg!(test) {
                    wgpu::TextureUsages::COPY_SRC
                } else {
                    wgpu::TextureUsages::empty()
                },
            view_formats: &[],
        })
    };
    let planes = std::array::from_fn(|lens| Planes {
        luma: make(
            if lens == 0 {
                "snapshot lens A luma"
            } else {
                "snapshot lens B luma"
            },
            size,
            wgpu::TextureFormat::R8Unorm,
        ),
        chroma: make(
            if lens == 0 {
                "snapshot lens A chroma"
            } else {
                "snapshot lens B chroma"
            },
            size.halved(),
            wgpu::TextureFormat::Rg8Unorm,
        ),
    });
    let snapshot = producer.source_snapshot();
    snapshot.encode_pair(
        encoder,
        &snapshot.luma,
        [&source.planes[0].luma, &source.planes[1].luma],
        [&planes[0].luma, &planes[1].luma],
        "resident luma source snapshot",
    );
    snapshot.encode_pair(
        encoder,
        &snapshot.chroma,
        [&source.planes[0].chroma, &source.planes[1].chroma],
        [&planes[0].chroma, &planes[1].chroma],
        "resident chroma source snapshot",
    );
    Ok(SourceSnapshot {
        planes,
        frame: source.frames.stamp(),
        context: source.context.clone(),
        size,
        samples: source.frames.samples,
        source_matrix: source_matrix(size, source.frames.samples),
    })
}

fn validate_plane(
    texture: &wgpu::Texture,
    size: Size,
    format: wgpu::TextureFormat,
    lens: usize,
    role: &str,
) -> Fallible<()> {
    if texture.size() != size.extent()
        || texture.dimension() != wgpu::TextureDimension::D2
        || texture.sample_count() != 1
        || texture.format() != format
        || !texture
            .usage()
            .contains(wgpu::TextureUsages::TEXTURE_BINDING)
    {
        return Err(format!(
            "resident lens {lens} {role} snapshot source must be sampled single-layer {format:?} at {} by {}",
            size.width, size.height
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn snapshot_owner_has_no_decoder_or_resident_carrier() {
        let source = include_str!("source_snapshot.rs");
        let fields = source
            .split_once("pub(crate) struct SourceSnapshot")
            .unwrap()
            .1
            .split_once("impl SourceSnapshot")
            .unwrap()
            .0;
        assert!(!fields.contains("Frames"));
        assert!(!fields.contains("Carrier"));
        assert!(fields.contains("planes: [Planes; 2]"));
        assert!(fields.contains("frame: FrameStamp"));
    }

    // Root runs GPU tests serially; this test deliberately does no submission
    // or readback outside its own exact plane-copy comparison.
    #[test]
    fn gpu_snapshot_preserves_every_initialized_plane_byte() {
        let Ok((device, queue)) = super::super::tests::gpu() else {
            assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none());
            return;
        };
        let size = Size::new(16, 8);
        let stamp = FrameStamp::for_test(7, std::time::Duration::from_millis(233), None);
        let context = OneXsGpuContext::new(&device, &queue);
        let session = ResidentSourceIdentity::for_test();
        let bytes = |len: usize, salt: u8| {
            (0..len)
                .map(|at| (at as u8).wrapping_mul(73).wrapping_add(salt))
                .collect::<Vec<_>>()
        };
        let make = |label, size: Size, format, data: &[u8], bytes_per_pixel| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: size.extent(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size.width * bytes_per_pixel),
                    rows_per_image: Some(size.height),
                },
                size.extent(),
            );
            texture
        };
        let luma = [
            bytes((size.width * size.height) as usize, 0),
            bytes((size.width * size.height) as usize, 19),
        ];
        let half = size.halved();
        let chroma = [
            bytes((half.width * half.height * 2) as usize, 41),
            bytes((half.width * half.height * 2) as usize, 113),
        ];
        let planes = std::array::from_fn(|lens| Planes {
            luma: make(
                "snapshot source luma",
                size,
                wgpu::TextureFormat::R8Unorm,
                &luma[lens],
                1,
            ),
            chroma: make(
                "snapshot source chroma",
                half,
                wgpu::TextureFormat::Rg8Unorm,
                &chroma[lens],
                2,
            ),
        });
        let imported = ImportedOneXsPicture {
            planes,
            frames: Arc::new(kjerag_media::Frames::empty_for_test(stamp, size)),
            context: context.clone(),
            session,
        };
        let layout = crate::scene::bind_group_layout(&device);
        let producer = DirectType2Pipeline::new(&device, &layout, wgpu::TextureFormat::Rgba8Unorm);
        let mut encoder = device.create_command_encoder(&Default::default());
        let snapshot = imported
            .encode_source_snapshot(&producer, &device, &mut encoder)
            .unwrap();
        let expected = [&luma[0], &chroma[0], &luma[1], &chroma[1]];
        let textures = [
            &snapshot.planes[0].luma,
            &snapshot.planes[0].chroma,
            &snapshot.planes[1].luma,
            &snapshot.planes[1].chroma,
        ];
        let readbacks = textures.map(|texture| encode_readback(&device, &mut encoder, texture));
        let submission = queue.submit([encoder.finish()]);
        for ((readback, texture), expected) in readbacks.into_iter().zip(textures).zip(expected) {
            assert_eq!(
                readback_bytes(&device, submission.clone(), readback, texture),
                expected.as_slice()
            );
        }
    }

    fn encode_readback(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> wgpu::Buffer {
        let bytes_per_pixel = texture.format().block_copy_size(None).unwrap();
        let stride = (texture.width() * bytes_per_pixel)
            .next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("source snapshot readback"),
            size: u64::from(stride * texture.height()),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(texture.height()),
                },
            },
            texture.size(),
        );
        buffer
    }

    fn readback_bytes(
        device: &wgpu::Device,
        submission: wgpu::SubmissionIndex,
        buffer: wgpu::Buffer,
        texture: &wgpu::Texture,
    ) -> Vec<u8> {
        let bytes_per_pixel = texture.format().block_copy_size(None).unwrap();
        let row = texture.width() * bytes_per_pixel;
        let stride = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        let mapped = buffer.slice(..).get_mapped_range();
        let bytes = mapped
            .chunks_exact(stride as usize)
            .flat_map(|row_bytes| row_bytes[..row as usize].iter().copied())
            .collect();
        drop(mapped);
        buffer.unmap();
        bytes
    }
}
