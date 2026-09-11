//! Body-equirect materialization and locked-view reprojection.
//!
//! This path stays RGB. It deliberately does not choose the
//! still-unread RGB-to-NV12 conversion needed by the temporal filter.

use wgpu::util::DeviceExt;

use super::{DirectType2Pipeline, draw_wgsl_with_fusion_mode};
use crate::{Extent, Fallible, FrameStamp, Size};

pub(super) mod nv12;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// One gamma-RGB body panorama inseparably named by its decoded delivery.
///
/// Construction is private to the exact map draw. Scene may retain this owner
/// through a temporal window without separately pairing a texture and stamp.
pub(crate) struct BodyPanorama {
    #[cfg(test)]
    device: wgpu::Device,
    texture: wgpu::Texture,
    frame: FrameStamp,
}

impl BodyPanorama {
    pub(super) fn new(device: &wgpu::Device, frame: FrameStamp, size: Size) -> Fallible<Self> {
        if size.width == 0 || size.height == 0 || size.width != size.height.saturating_mul(2) {
            return Err(format!(
                "body panorama must be nonzero 2:1, got {} by {}",
                size.width, size.height
            )
            .into());
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("source-stamped gamma RGB body panorama"),
            size: size.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Ok(Self {
            #[cfg(test)]
            device: device.clone(),
            texture,
            frame,
        })
    }

    pub(crate) fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub(crate) fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    #[cfg(test)]
    pub(crate) fn belongs_to(&self, device: &wgpu::Device) -> bool {
        self.device == *device
    }
}

/// The source/map consumer with only its output projection replaced.
pub(super) struct BodyPanoramaPipeline {
    pipeline: wgpu::RenderPipeline,
}

impl BodyPanoramaPipeline {
    pub(super) fn new(device: &wgpu::Device, direct: &DirectType2Pipeline) -> Self {
        let hardware_fusion = direct.fusion_sampler.is_some();
        let fusion = direct.fusion_layout.is_some();
        let source = format!(
            "{}\n{}",
            draw_wgsl_with_fusion_mode(fusion, hardware_fusion),
            BODY_PANORAMA_WGSL
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gamma RGB body panorama"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let mut layouts = vec![&direct.picture_layout, &direct.map_layout];
        layouts.extend(direct.fusion_layout.as_ref());
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gamma RGB body panorama"),
            bind_group_layouts: &layouts,
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gamma RGB body panorama"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("panorama_fs"),
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
        Self { pipeline }
    }

    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        picture: &wgpu::BindGroup,
        map: &wgpu::BindGroup,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, picture, &[]);
        pass.set_bind_group(1, map, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Reproject one gamma-RGB panorama through an exact `Reframe`. Longitude
/// repeats and latitude clamps in the texture sampler.
pub(crate) struct PanoramaProjector {
    device: wgpu::Device,
    #[cfg(test)]
    pipeline: wgpu::RenderPipeline,
    module: wgpu::ShaderModule,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    draw_pipelines: std::sync::Mutex<Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>>,
}

impl PanoramaProjector {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gamma RGB panorama projection"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<
                            crate::Reframe,
                        >()
                            as u64),
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
        #[cfg(test)]
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gamma RGB panorama projection"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gamma RGB panorama projection"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{}\n{}", crate::projection::wgsl(), PROJECT_WGSL).into(),
            ),
        });
        #[cfg(test)]
        let pipeline = projection_pipeline(
            device,
            &pipeline_layout,
            &module,
            FORMAT,
            "panorama_project_fs",
            None,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("periodic body panorama sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Self {
            device: device.clone(),
            #[cfg(test)]
            pipeline,
            module,
            layout,
            sampler,
            draw_pipelines: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Bind one exact source-stamped body panorama for an existing render
    /// pass. Preparing allocates only the immutable view uniform/bind group;
    /// the target-format pipeline is cached and redraws never rematerialize the
    /// panorama. The caller still owns presentation and submission.
    #[cfg(test)]
    pub(crate) fn prepare(
        &self,
        device: &wgpu::Device,
        panorama: &BodyPanorama,
        reframe: &crate::Reframe,
        output_format: wgpu::TextureFormat,
    ) -> Fallible<PreparedPanoramaDraw> {
        if !panorama.belongs_to(device) {
            return Err("body panorama belongs to a different graphics device".into());
        }
        self.prepare_source(
            device,
            panorama.texture(),
            panorama.frame(),
            reframe,
            output_format,
        )
    }

    /// Bind one completed temporal-filter output without separating its
    /// texture from the history center that names it.
    pub(crate) fn prepare_filtered(
        &self,
        device: &wgpu::Device,
        panorama: &crate::temporal_fusion::stream::FilteredPanorama,
        reframe: &crate::Reframe,
        output_format: wgpu::TextureFormat,
    ) -> Fallible<PreparedPanoramaDraw> {
        if !panorama.belongs_to(device) {
            return Err("filtered panorama belongs to a different graphics device".into());
        }
        self.prepare_source(
            device,
            panorama.texture(),
            panorama.frame(),
            reframe,
            output_format,
        )
    }

    /// Shared implementation for other private source-stamped panorama
    /// owners. Typed public entry points must validate their own sealed owner
    /// before passing its inseparable texture and stamp here.
    fn prepare_source(
        &self,
        device: &wgpu::Device,
        texture: &wgpu::Texture,
        frame: &FrameStamp,
        reframe: &crate::Reframe,
        output_format: wgpu::TextureFormat,
    ) -> Fallible<PreparedPanoramaDraw> {
        if self.device != *device {
            return Err("panorama projector and source must belong to this graphics device".into());
        }
        validate_panorama(texture)?;
        if reframe.linearizes_output() != output_format.is_srgb() {
            return Err(
                "panorama projection transfer does not match the render target format".into(),
            );
        }
        if !matches!(
            output_format,
            wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Rgba8UnormSrgb
                | wgpu::TextureFormat::Bgra8Unorm
                | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            return Err(format!(
                "panorama projection target format {output_format:?} is not a renderable RGBA surface format"
            )
            .into());
        }

        let pipeline = {
            let mut pipelines = self
                .draw_pipelines
                .lock()
                .map_err(|_| "panorama projection pipeline cache is unavailable")?;
            if let Some((_, pipeline)) = pipelines
                .iter()
                .find(|(format, _)| *format == output_format)
            {
                pipeline.clone()
            } else {
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("gamma RGB panorama surface projection"),
                    bind_group_layouts: &[&self.layout],
                    immediate_size: 0,
                });
                let pipeline = projection_pipeline(
                    device,
                    &layout,
                    &self.module,
                    output_format,
                    "panorama_draw_fs",
                    Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                );
                pipelines.push((output_format, pipeline.clone()));
                pipeline
            }
        };
        Ok(PreparedPanoramaDraw {
            pipeline,
            binding: self.binding(device, texture, reframe),
            frame: frame.clone(),
        })
    }

    /// Append projection into a newly allocated non-sRGB texture.
    ///
    /// The caller owns source-stamp validation before passing the exact
    /// `Reframe`, so original and derived RGB panoramas use the same primitive
    /// without minting a false source owner. Several same-source arms can be
    /// appended to one command buffer without an intermediate submit.
    #[cfg(test)]
    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        panorama: &wgpu::Texture,
        reframe: &crate::Reframe,
        output_size: Size,
    ) -> Fallible<wgpu::Texture> {
        if self.device != *device {
            return Err("panorama projector belongs to a different graphics device".into());
        }
        if output_size.width == 0 || output_size.height == 0 {
            return Err("panorama projection target must be nonzero".into());
        }
        validate_panorama(panorama)?;
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("non-sRGB gamma RGB projected panorama view"),
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
        self.encode_to_view(device, encoder, &target, panorama, reframe);
        Ok(output)
    }

    #[cfg(test)]
    fn encode_to_view(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        panorama: &wgpu::Texture,
        reframe: &crate::Reframe,
    ) {
        let binding = self.binding(device, panorama, reframe);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gamma RGB panorama projection"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
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
    }

    fn binding(
        &self,
        device: &wgpu::Device,
        panorama: &wgpu::Texture,
        reframe: &crate::Reframe,
    ) -> wgpu::BindGroup {
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("exact panorama projection Reframe"),
            contents: reframe.bytes(),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let source = panorama.create_view(&Default::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("source-stamped gamma RGB panorama projection"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&source),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }
}

/// One immutable view of one exact stamped body panorama. Construction stays
/// behind [`PanoramaProjector::prepare`], so a caller cannot pair arbitrary
/// texture bytes with a frame identity.
pub(crate) struct PreparedPanoramaDraw {
    pipeline: wgpu::RenderPipeline,
    binding: wgpu::BindGroup,
    frame: FrameStamp,
}

impl PreparedPanoramaDraw {
    pub(crate) fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn validate_panorama(panorama: &wgpu::Texture) -> Fallible<()> {
    if panorama.format() != FORMAT
        || panorama.dimension() != wgpu::TextureDimension::D2
        || panorama.depth_or_array_layers() != 1
        || panorama.mip_level_count() != 1
        || panorama.sample_count() != 1
        || !panorama
            .usage()
            .contains(wgpu::TextureUsages::TEXTURE_BINDING)
    {
        return Err(
            "panorama projector requires a single-sampled sampleable 2D Rgba8Unorm texture".into(),
        );
    }
    Ok(())
}

fn projection_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    fragment: &'static str,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("gamma RGB panorama projection"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("panorama_project_vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

const BODY_PANORAMA_WGSL: &str = r#"
@fragment
fn panorama_fs(in: Type2VsOut) -> @location(0) vec4<f32> {
  let phi = TYPE2_PI * in.uv.y;
  let theta = TYPE2_TAU * in.uv.x;
  let body = vec3<f32>(sin(phi) * sin(theta), cos(phi),
    -sin(phi) * cos(theta));
  let map = type2_mesh(body);
  if map.covered <= 0.5 { return vec4<f32>(0.0); }
  return type2_gamma_color(map);
}
"#;

const PROJECT_WGSL: &str = r#"
@group(0) @binding(1) var panorama: texture_2d<f32>;
@group(0) @binding(2) var panorama_sampler: sampler;

struct PanoramaProjectVsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn panorama_project_vs(@builtin(vertex_index) index: u32) -> PanoramaProjectVsOut {
  let x = f32((index << 1u) & 2u);
  let y = f32(index & 2u);
  var out: PanoramaProjectVsOut;
  out.uv = vec2<f32>(x, y);
  out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  return out;
}

fn body_panorama_uv(body_unscaled: vec3<f32>) -> vec2<f32> {
  let body = normalize(body_unscaled);
  var theta = atan2(body.x, -body.z);
  if theta < 0.0 { theta += 6.28318530717958647692; }
  return vec2<f32>(theta / 6.28318530717958647692,
    acos(clamp(body.y, -1.0, 1.0)) / 3.14159265358979323846);
}

fn panorama_project_sample(in: PanoramaProjectVsOut) -> vec4<f32> {
  let view = view_ray(in.uv);
  if view.w <= 0.0 { return vec4<f32>(0.0); }
  let uv = body_panorama_uv(reframe.view_to_body * view.xyz);
  return textureSample(panorama, panorama_sampler, uv);
}

@fragment
fn panorama_project_fs(in: PanoramaProjectVsOut) -> @location(0) vec4<f32> {
  return panorama_project_sample(in);
}

@fragment
fn panorama_draw_fs(in: PanoramaProjectVsOut) -> @location(0) vec4<f32> {
  let gamma = panorama_project_sample(in);
  let linear = select(
    gamma.rgb / 12.92,
    pow((gamma.rgb + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)),
    gamma.rgb > vec3<f32>(0.04045),
  );
  return vec4<f32>(select(gamma.rgb, linear, reframe.linearize > 0.5), gamma.a);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{PI, TAU};
    use std::time::Duration;

    use crate::{Camera, Held, Reframe, Sampling};

    fn body(uv: [f32; 2]) -> [f32; 3] {
        let phi = PI * uv[1];
        let theta = TAU * uv[0];
        [phi.sin() * theta.sin(), phi.cos(), -phi.sin() * theta.cos()]
    }

    fn uv(ray: [f32; 3]) -> [f32; 2] {
        let reach = ray.iter().map(|v| v * v).sum::<f32>().sqrt();
        let ray = ray.map(|v| v / reach);
        let theta = ray[0].atan2(-ray[2]).rem_euclid(TAU);
        [theta / TAU, ray[1].clamp(-1.0, 1.0).acos() / PI]
    }

    fn periodic_error(a: f32, b: f32) -> f32 {
        (a - b + 0.5).rem_euclid(1.0) - 0.5
    }

    #[test]
    fn body_raster_and_projector_are_inverse_at_texel_centres() {
        for size in [[16u32, 8u32], [30, 15]] {
            for y in 0..size[1] {
                for x in 0..size[0] {
                    let expected = [
                        (x as f32 + 0.5) / size[0] as f32,
                        (y as f32 + 0.5) / size[1] as f32,
                    ];
                    let actual = uv(body(expected));
                    assert!(periodic_error(actual[0], expected[0]).abs() < 2e-6);
                    assert!((actual[1] - expected[1]).abs() < 2e-6);
                }
            }
        }
    }

    #[test]
    fn body_raster_cardinals_fix_seam_and_direction() {
        let near =
            |a: [f32; 3], b: [f32; 3]| a.into_iter().zip(b).all(|(a, b)| (a - b).abs() < 2e-6);
        assert!(near(body([0.0, 0.5]), [0.0, 0.0, -1.0]));
        assert!(near(body([0.25, 0.5]), [1.0, 0.0, 0.0]));
        assert!(near(body([0.5, 0.5]), [0.0, 0.0, 1.0]));
        assert!(near(body([0.75, 0.5]), [-1.0, 0.0, 0.0]));
        assert!(near(body([0.5, 0.0]), [0.0, 1.0, 0.0]));
        assert!(near(body([0.5, 1.0]), [0.0, -1.0, 0.0]));
    }

    #[test]
    fn shaders_keep_gamma_rgb_and_explicit_boundary_modes() {
        use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

        let materializer = format!(
            "{}\n{}",
            draw_wgsl_with_fusion_mode(false, false),
            BODY_PANORAMA_WGSL
        );
        let projector = format!("{}\n{}", crate::projection::wgsl(), PROJECT_WGSL);
        for (name, source) in [("materializer", &materializer), ("projector", &projector)] {
            let module = wgpu::naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|error| panic!("panorama {name} WGSL did not parse: {error}"));
            Validator::new(ValidationFlags::all(), Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|error| panic!("panorama {name} WGSL did not validate: {error}"));
        }
        assert!(BODY_PANORAMA_WGSL.contains("type2_gamma_color(map)"));
        assert!(!BODY_PANORAMA_WGSL.contains("reframe.view_to_body"));
        assert!(PROJECT_WGSL.contains("atan2(body.x, -body.z)"));
        assert!(PROJECT_WGSL.contains("textureSample(panorama, panorama_sampler, uv)"));
        assert!(PROJECT_WGSL.contains("reframe.linearize > 0.5"));
    }

    #[test]
    fn prepared_existing_pass_matches_offscreen_projection_and_keeps_stamp() {
        let Ok((device, queue)) = super::super::tests::gpu() else {
            eprintln!("skipping panorama prepared-draw test without a Vulkan adapter");
            return;
        };
        let panorama_size = Size::new(64, 32);
        let output_size = Size::new(64, 36);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("prepared panorama test source"),
            size: panorama_size.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let pixels: Vec<u8> = (0..panorama_size.height)
            .flat_map(|y| {
                (0..panorama_size.width).flat_map(move |x| {
                    [
                        x.wrapping_mul(17).wrapping_add(y.wrapping_mul(3)) as u8,
                        x.wrapping_mul(5).wrapping_add(y.wrapping_mul(19)) as u8,
                        (x ^ y).wrapping_mul(11) as u8,
                        255,
                    ]
                })
            })
            .collect();
        queue.write_texture(
            texture.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(panorama_size.width * 4),
                rows_per_image: Some(panorama_size.height),
            },
            panorama_size.extent(),
        );
        let stamp = FrameStamp::for_test(41, Duration::from_secs(2), None);
        let panorama = BodyPanorama {
            device: device.clone(),
            texture,
            frame: stamp.clone(),
        };
        let projector = PanoramaProjector::new(&device);

        for camera in [
            Camera::default(),
            Camera {
                yaw: 179.8_f32.to_radians(),
                pitch: 23.0_f32.to_radians(),
                fov: 96.0_f32.to_radians(),
            },
            Camera {
                yaw: -71.0_f32.to_radians(),
                pitch: -84.0_f32.to_radians(),
                fov: 72.0_f32.to_radians(),
            },
        ] {
            let reframe = Reframe::new(
                &[],
                panorama_size,
                camera,
                Held::default(),
                output_size.width as f32 / output_size.height as f32,
                false,
                Sampling::default(),
            );
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            let expected = projector
                .encode(
                    &device,
                    &mut encoder,
                    panorama.texture(),
                    &reframe,
                    output_size,
                )
                .unwrap();
            let actual = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("prepared panorama test target"),
                size: output_size.extent(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let draw = projector
                .prepare(&device, &panorama, &reframe, FORMAT)
                .unwrap();
            assert_eq!(draw.frame(), &stamp);
            {
                let view = actual.create_view(&Default::default());
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("prepared panorama existing pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                draw.draw(&mut pass);
            }
            let expected = copy_for_readback(&device, &mut encoder, &expected, output_size);
            let actual = copy_for_readback(&device, &mut encoder, &actual, output_size);
            queue.submit([encoder.finish()]);
            assert_eq!(
                read_copy(&device, &expected, output_size),
                read_copy(&device, &actual, output_size),
                "prepared draw differs for {camera:?}"
            );
        }
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
            label: Some("prepared panorama test readback"),
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

    fn read_copy(device: &wgpu::Device, buffer: &wgpu::Buffer, size: Size) -> Vec<u8> {
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = buffer.slice(..).get_mapped_range().to_vec();
        buffer.unmap();
        assert_eq!(bytes.len(), (size.width * size.height * 4) as usize);
        bytes
    }
}
