//! The one shader pass the player draws.
//!
//! With no file it is an animated gradient, which proves only that a custom
//! wgpu pass runs inside libcosmic. With a file it reprojects one lens of a
//! real VA-API frame imported by [`super::dmabuf`]: for every output pixel,
//! a view ray through the camera's yaw, pitch and field of view, rotated
//! into the lens frame and pushed through the Mei/UCM model in
//! [`super::projection`], sampled from the NV12 planes and converted to RGB.
//! One pass, no intermediate target.

use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use kyerag_media::{self as media, DrmFrame};
use kyerag_meta::{CalibrationSet, Lens};

use super::projection::{self, Reframe};
use super::{Camera, Extent, Fallible, Planes, Size, dmabuf};

/// Issue #3 reframes one lens. The second stream is the seam's business
/// (issue #7), and a view centred near a lens axis contains no seam at all.
const LENS: usize = 0;

/// A file the shell was asked to show. Decoding waits for the first
/// [`ScenePipeline::prepare`], because the import needs iced's device and
/// there is no earlier moment that has one.
#[derive(Debug)]
pub struct Frame {
    path: PathBuf,
}

impl Frame {
    pub fn pending(path: PathBuf) -> Self {
        Self { path }
    }
}

/// The widget's state, owned by the shell.
pub struct Scene {
    frame: Option<Arc<Frame>>,
    elapsed: Duration,
}

impl Scene {
    pub fn new(frame: Option<Arc<Frame>>) -> Self {
        Self {
            frame,
            elapsed: Duration::ZERO,
        }
    }

    pub fn advance(&mut self, step: Duration) {
        self.elapsed += step;
    }

    pub fn primitive(&self, camera: Camera) -> ScenePrimitive {
        ScenePrimitive {
            elapsed: self.elapsed.as_secs_f32(),
            camera,
            frame: self.frame.clone(),
        }
    }
}

/// What the shell hands the renderer for one frame.
#[derive(Debug)]
pub struct ScenePrimitive {
    elapsed: f32,
    camera: Camera,
    frame: Option<Arc<Frame>>,
}

/// The GPU state behind the widget. iced builds one of these per primitive
/// type and keeps it for the life of the renderer.
pub struct ScenePipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// `Some` once a file has been decoded and imported, or once the attempt
    /// has failed; either way it is tried exactly once.
    source: Option<Source>,
    linearize: bool,
    reported: bool,
}

enum Source {
    /// Boxed because the other arm carries nothing: an enum that is 300
    /// bytes wide whichever way the load went is what `clippy` objects to,
    /// and it is right that the failed case should cost a pointer.
    Imported(Box<Imported>),
    Failed,
}

/// One imported frame and the calibration that reprojects it. The mapped
/// frame must outlive the textures: dropping it returns the surface to the
/// decoder's pool.
struct Imported {
    lens: Lens,
    size: Size,
    _frame: DrmFrame,
    _planes: Planes,
}

impl ScenePipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(format!("{}\n{SHADER}", projection::wgsl()).into()),
        });
        let layout = bind_group_layout(device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            // iced's own pass has one sample and no depth attachment
            // (`iced_wgpu/src/lib.rs`, "iced_wgpu render pass"); a pipeline
            // that disagrees fails to draw rather than looking wrong.
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene"),
            size: std::mem::size_of::<Reframe>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Views keep their textures alive, so the blank pair needs no field.
        let bind_group = bind(device, &layout, &uniforms, &blank_planes(device), &sampler);

        Self {
            pipeline,
            layout,
            sampler,
            uniforms,
            bind_group,
            source: None,
            // iced picks an sRGB surface when it gamma-corrects, and the GPU
            // then encodes whatever the shader writes. Video is already
            // gamma-encoded, so it has to be decoded back to linear first or
            // the picture washes out.
            linearize: format.is_srgb(),
            reported: false,
        }
    }

    /// `aspect` is the output's width over its height, which is what decides
    /// the vertical field of view. The widget reads it from its bounds.
    pub fn prepare(
        &mut self,
        primitive: &ScenePrimitive,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        aspect: f32,
    ) {
        if !self.reported {
            self.reported = true;
            println!("device: {}", dmabuf::device_report(device));
        }
        if let Some(frame) = primitive.frame.as_ref() {
            self.load_once(frame, device);
        }

        let reframe = match &self.source {
            Some(Source::Imported(imported)) => Reframe::new(
                &imported.lens,
                imported.size,
                primitive.camera,
                aspect,
                self.linearize,
            ),
            _ => Reframe::gradient(primitive.elapsed, aspect, self.linearize),
        };
        queue.write_buffer(&self.uniforms, 0, reframe.bytes());
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn load_once(&mut self, frame: &Frame, device: &wgpu::Device) {
        if self.source.is_some() {
            return;
        }
        match self.load(frame, device) {
            Ok(source) => self.source = Some(source),
            Err(e) => {
                eprintln!("kyerag: {} not shown: {e}", frame.path.display());
                self.source = Some(Source::Failed);
            }
        }
    }

    fn load(&mut self, frame: &Frame, device: &wgpu::Device) -> Fallible<Source> {
        let calibration = CalibrationSet::from_insv(&frame.path)?;
        let lens = calibration
            .lenses
            .get(LENS)
            .ok_or("calibration describes no lens 0")?
            .clone();

        let (mapped, size) = media::first_frame(&frame.path, LENS)?;
        // The calibration's pixel numbers are already in delivered-frame
        // coordinates, so they describe this texture only if the stream is
        // the size the trailer says it is. A mismatch reprojects at the
        // wrong scale, which reads as a mild lens error rather than a bug.
        if (calibration.dimension.width, calibration.dimension.height) != (size.width, size.height)
        {
            return Err(format!(
                "trailer says lens frames are {}x{} but stream {LENS} decodes {}x{}",
                calibration.dimension.width, calibration.dimension.height, size.width, size.height
            )
            .into());
        }

        println!("drm:    {}", mapped.describe());
        let planes = dmabuf::import(device, mapped.descriptor(), size)?;
        self.bind_group = bind(device, &self.layout, &self.uniforms, &planes, &self.sampler);
        println!(
            "lens:   {} {}, lens {LENS} of {}, {} x {} imported",
            calibration.camera_model,
            calibration.firmware,
            calibration.lenses.len(),
            size.width,
            size.height
        );
        Ok(Source::Imported(Box::new(Imported {
            lens,
            size,
            _frame: mapped,
            _planes: planes,
        })))
    }
}

fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    planes: &Planes,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let view = |texture: &wgpu::Texture| texture.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view(&planes.luma)),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&view(&planes.chroma)),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("scene"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    // The one place the Rust and WGSL definitions of the
                    // uniform block are checked against each other: pipeline
                    // creation fails if the shader's struct wants more bytes
                    // than `Reframe` has.
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<Reframe>() as u64),
                },
                count: None,
            },
            texture(1),
            texture(2),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// The shader samples its planes unconditionally, so something has to be bound
/// before a file is. One black pixel each.
fn blank_planes(device: &wgpu::Device) -> Planes {
    let make = |format| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blank"),
            size: Size::new(1, 1).extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };
    Planes {
        luma: make(wgpu::TextureFormat::R8Unorm),
        chroma: make(wgpu::TextureFormat::Rg8Unorm),
    }
}

/// The half of the shader that belongs to this file. `projection::WGSL`
/// declares the uniform block, the view ray and the forward map, and is
/// concatenated ahead of this.
const SHADER: &str = r#"
struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  let x = f32((i << 1u) & 2u);
  let y = f32(i & 2u);
  var out: VsOut;
  out.uv = vec2<f32>(x, y);
  out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  return out;
}

@group(0) @binding(1) var luma: texture_2d<f32>;
@group(0) @binding(2) var chroma: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

fn gradient(uv: vec2<f32>, t: f32) -> vec3<f32> {
  let d = length(uv - vec2<f32>(0.5, 0.5));
  let wave = 0.5 + 0.5 * sin(d * 24.0 - t * 3.0);
  return vec3<f32>(wave * uv.x, wave * uv.y, wave);
}

// BT.709 full range: ffprobe reports bt709 and the camera writes yuvj420p.
// DRM_FORMAT_GR88 is little endian G:R, so .r is Cb and .g is Cr.
fn nv12(uv: vec2<f32>) -> vec3<f32> {
  let y = textureSample(luma, samp, uv).r;
  let c = textureSample(chroma, samp, uv).rg - vec2<f32>(0.5, 0.5);
  return vec3<f32>(
    y + 1.5748 * c.g,
    y - 0.1873 * c.r - 0.4681 * c.g,
    y + 1.8556 * c.r,
  );
}

fn linearize(c: vec3<f32>) -> vec3<f32> {
  let lo = c / 12.92;
  let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
  return select(lo, hi, c > vec3<f32>(0.04045));
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let landing = project(view_ray(in.uv));
  // Every branch is evaluated because `textureSample` needs uniform control
  // flow; picking afterwards is the WGSL-legal way to write this. A ray that
  // missed the lens would otherwise read a clamped edge texel and smear it
  // across the whole invalid region.
  let lens = select(OUTSIDE_GRAY, nv12(frame_uv(landing.pixel)), landing.inside);
  let rgb = select(gradient(in.uv, reframe.elapsed), lens, reframe.has_frame > 0.5);
  return vec4<f32>(select(rgb, linearize(rgb), reframe.linearize > 0.5), 1.0);
}
"#;
