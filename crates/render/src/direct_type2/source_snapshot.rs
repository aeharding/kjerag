//! GPU-owned, full-resolution display snapshot of one imported lens pair.
//!
//! Imported textures alias decoder surfaces and therefore cannot outlive their
//! `Frames`. This module evaluates the native source box filter once at every
//! output texel centre into ordinary wgpu textures. Final views then sample
//! those immutable, prefiltered planes without retaining a decoder owner or
//! resident production carrier.
//!
//! This is deliberately approximate: filtering before R8/RG8 quantization and
//! later viewport interpolation is not algebraically identical to evaluating
//! the native box at every final view sample.

use kjerag_media::{Samples, Size};

use super::{DirectType2Pipeline, ImportedOneXsPicture};
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
#[cfg(test)]
use crate::flow::one_xs::one_xs_belt_gpu::ResidentSourceIdentity;
use crate::{Extent, Fallible, FrameStamp, Planes};

const FILTER_SHADER_PREFIX: &str = r#"
struct VertexOutput {
  @builtin(position) position: vec4<f32>,
}

@group(0) @binding(0) var source_a: texture_2d<f32>;
@group(0) @binding(1) var source_b: texture_2d<f32>;
@group(0) @binding(2) var type2_sampler: sampler;

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
"#;

const FILTER_SHADER_ENTRIES: &str = r#"
fn atlas_centres(position: vec2<f32>) -> array<vec2<f32>, 2> {
  let dimensions = vec2<f32>(textureDimensions(source_a));
  return array(
    vec2<f32>(position.x / (2.0 * dimensions.x), position.y / dimensions.y),
    vec2<f32>((dimensions.x + position.x) / (2.0 * dimensions.x), position.y / dimensions.y),
  );
}

struct LumaOutput {
  @location(0) a: f32,
  @location(1) b: f32,
}

@fragment
fn luma_fs(in: VertexOutput) -> LumaOutput {
  let centres = atlas_centres(in.position.xy);
  let logical = vec2<f32>(textureDimensions(source_a));
  var out: LumaOutput;
  out.a = type2_box(source_a, source_b, centres[0], logical).r;
  out.b = type2_box(source_a, source_b, centres[1], logical).r;
  return out;
}

struct ChromaOutput {
  @location(0) a: vec2<f32>,
  @location(1) b: vec2<f32>,
}

@fragment
fn chroma_fs(in: VertexOutput) -> ChromaOutput {
  let centres = atlas_centres(in.position.xy);
  let logical = vec2<f32>(textureDimensions(source_a));
  var out: ChromaOutput;
  out.a = type2_box(source_a, source_b, centres[0], logical).rg;
  out.b = type2_box(source_a, source_b, centres[1], logical).rg;
  return out;
}
"#;

/// Cached full-resolution prefilter shared by every display snapshot made
/// through one direct pipeline. It records commands only; submission and
/// retirement stay with the installed resident transaction.
pub(super) struct SnapshotPipeline {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    luma: wgpu::RenderPipeline,
    chroma: wgpu::RenderPipeline,
}

impl SnapshotPipeline {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("resident source snapshot inputs"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("resident source snapshot"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let shader_source = format!(
            "{FILTER_SHADER_PREFIX}\n{}\n{FILTER_SHADER_ENTRIES}",
            super::SOURCE_FILTER_WGSL
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("resident source snapshot"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("resident source snapshot linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
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
            sampler,
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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
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

/// One immutable, GPU-owned pair of prefiltered display planes and its
/// unforgeable source association. Plane handles never cross this boundary.
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
        coordinates: &super::correction::CorrectionCoordinates,
    ) -> Fallible<super::correction::CorrectionPictureBinding> {
        if &self.frame != correction.frame() {
            return Err("temporal correction names a different source snapshot".into());
        }
        if !correction.belongs_to(self.context.device()) {
            return Err("temporal correction belongs to a different graphics device".into());
        }
        let reframe = self.exact_reframe(reframe)?;
        pipeline.prepare_picture(
            &reframe,
            [&self.planes[0], &self.planes[1]],
            correction,
            coordinates,
        )
    }

    /// Prepare the exact selected picture with its temporal contribution
    /// reduced to zero. This exists only for real-source diagnostics.
    #[cfg(test)]
    pub(crate) fn prepare_correction_picture_without_temporal_for_review(
        &self,
        pipeline: &super::correction::CorrectionPipeline,
        reframe: &crate::Reframe,
        correction: &crate::temporal_fusion::correction_stream::CorrectionFrame,
        coordinates: &super::correction::CorrectionCoordinates,
    ) -> Fallible<super::correction::CorrectionPictureBinding> {
        if &self.frame != correction.frame() {
            return Err("temporal correction names a different source snapshot".into());
        }
        if !correction.belongs_to(self.context.device()) {
            return Err("temporal correction belongs to a different graphics device".into());
        }
        let reframe = self.exact_reframe(reframe)?;
        pipeline.prepare_picture_without_temporal_for_review(
            &reframe,
            [&self.planes[0], &self.planes[1]],
            correction,
            coordinates,
        )
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

    // Root runs GPU tests serially. The CPU oracle independently evaluates the
    // native atlas box at every destination texel centre, including both sides
    // of the lens join and alternating source parity.
    #[test]
    fn gpu_snapshot_matches_native_box_at_every_plane_texel_centre() {
        let Ok((device, queue)) = super::super::tests::gpu() else {
            assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none());
            return;
        };
        let size = Size::new(16, 8);
        let stamp = FrameStamp::for_test(7, std::time::Duration::from_millis(233), None);
        let context = OneXsGpuContext::new(&device, &queue);
        let session = ResidentSourceIdentity::for_test();
        let bytes = |size: Size, channels: usize, lens: usize, salt: usize| {
            (0..size.height as usize)
                .flat_map(|y| {
                    (0..size.width as usize).flat_map(move |x| {
                        (0..channels).map(move |channel| {
                            ((x * 73
                                + y * 47
                                + channel * 113
                                + lens * 191
                                + ((x + y) & 1) * 59
                                + salt)
                                % 256) as u8
                        })
                    })
                })
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
        let luma = [bytes(size, 1, 0, 0), bytes(size, 1, 1, 19)];
        let half = size.halved();
        let chroma = [bytes(half, 2, 0, 41), bytes(half, 2, 1, 113)];
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
        let expected_luma = native_box_planes([&luma[0], &luma[1]], size, 1);
        let expected_chroma = native_box_planes([&chroma[0], &chroma[1]], half, 2);
        let expected = [
            &expected_luma[0],
            &expected_chroma[0],
            &expected_luma[1],
            &expected_chroma[1],
        ];
        let textures = [
            &snapshot.planes[0].luma,
            &snapshot.planes[0].chroma,
            &snapshot.planes[1].luma,
            &snapshot.planes[1].chroma,
        ];
        let readbacks = textures.map(|texture| encode_readback(&device, &mut encoder, texture));
        let submission = queue.submit([encoder.finish()]);
        let mut worst = 0u8;
        for ((readback, texture), expected) in readbacks.into_iter().zip(textures).zip(expected) {
            let actual = readback_bytes(&device, submission.clone(), readback, texture);
            assert_eq!(actual.len(), expected.len());
            for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
                let difference = actual.abs_diff(expected);
                // Hardware bilinear weights may have finite subtexel
                // precision before the render target performs UNORM rounding.
                // One code value bounds that numeric variance; it is not an
                // acceptance of the visual prefilter approximation.
                assert!(
                    difference <= 1,
                    "prefilter byte {index} is {actual}, CPU native box is {expected}"
                );
                worst = worst.max(difference);
            }
        }
        eprintln!("source snapshot native-box worst difference: {worst} code values");
    }

    const BOX_SIZE: f32 = 1.7881767;

    fn native_box_planes(sources: [&[u8]; 2], size: Size, channels: usize) -> [Vec<u8>; 2] {
        std::array::from_fn(|lens| {
            let mut output =
                Vec::with_capacity(size.width as usize * size.height as usize * channels);
            for y in 0..size.height {
                for x in 0..size.width {
                    let uv = [
                        (lens as f32 * size.width as f32 + x as f32 + 0.5)
                            / (2.0 * size.width as f32),
                        (y as f32 + 0.5) / size.height as f32,
                    ];
                    for channel in 0..channels {
                        output.push(
                            (native_box(sources, size, channels, channel, uv) * 255.0)
                                .round()
                                .clamp(0.0, 255.0) as u8,
                        );
                    }
                }
            }
            output
        })
    }

    fn native_box(
        sources: [&[u8]; 2],
        size: Size,
        channels: usize,
        channel: usize,
        uv: [f32; 2],
    ) -> f32 {
        let logical = [size.width as f32, size.height as f32];
        let start = [
            uv[0] * logical[0] - BOX_SIZE * 0.5,
            uv[1] * logical[1] - BOX_SIZE * 0.5,
        ];
        let end = [start[0] + BOX_SIZE, start[1] + BOX_SIZE];
        let mut sum = 0.0;
        let mut area = 0.0;
        let mut y = start[1].floor();
        while y < end[1] {
            let low_y = y.max(start[1]);
            let high_y = (y + 2.0).min(end[1]);
            let mut x = start[0].floor();
            while x < end[0] {
                let low_x = x.max(start[0]);
                let high_x = (x + 2.0).min(end[0]);
                let cell_area = (high_x - low_x) * (high_y - low_y);
                let sample_uv = [
                    (low_x + high_x) * 0.5 / logical[0],
                    (low_y + high_y) * 0.5 / logical[1],
                ];
                sum += atlas_linear(sources, size, channels, channel, sample_uv) * cell_area;
                area += cell_area;
                x += 2.0;
            }
            y += 2.0;
        }
        // The WGSL's recovered decimal rounds to this same binary32 value.
        if area > 0.001_f32 {
            sum / area
        } else {
            atlas_linear(sources, size, channels, channel, uv)
        }
    }

    fn atlas_linear(
        sources: [&[u8]; 2],
        size: Size,
        channels: usize,
        channel: usize,
        uv: [f32; 2],
    ) -> f32 {
        let p = [
            uv[0] * (2 * size.width) as f32 - 0.5,
            uv[1] * size.height as f32 - 0.5,
        ];
        let base = [p[0].floor() as i32, p[1].floor() as i32];
        let fraction = [p[0] - p[0].floor(), p[1] - p[1].floor()];
        let top = mix(
            atlas_load(sources, size, channels, channel, base),
            atlas_load(sources, size, channels, channel, [base[0] + 1, base[1]]),
            fraction[0],
        );
        let bottom = mix(
            atlas_load(sources, size, channels, channel, [base[0], base[1] + 1]),
            atlas_load(sources, size, channels, channel, [base[0] + 1, base[1] + 1]),
            fraction[0],
        );
        mix(top, bottom, fraction[1])
    }

    fn atlas_load(
        sources: [&[u8]; 2],
        size: Size,
        channels: usize,
        channel: usize,
        at: [i32; 2],
    ) -> f32 {
        let x = at[0].clamp(0, (2 * size.width) as i32 - 1) as u32;
        let y = at[1].clamp(0, size.height as i32 - 1) as u32;
        let lens = usize::from(x >= size.width);
        let local_x = x % size.width;
        let index = ((y * size.width + local_x) as usize) * channels + channel;
        sources[lens][index] as f32 / 255.0
    }

    fn mix(a: f32, b: f32, amount: f32) -> f32 {
        a * (1.0 - amount) + b * amount
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
