//! The one shader pass the bring-up surface draws.
//!
//! With no file it is an animated gradient, which proves only that a custom
//! wgpu pass runs inside libcosmic. With a file it samples a real VA-API frame
//! imported by [`super::dmabuf`], which proves the import works on the device
//! iced created, inside iced's own render pass.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use kyerag_media::{self as media, DrmFrame};

use super::{Extent, Fallible, Planes, Size, dmabuf};

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

    pub fn primitive(&self) -> ScenePrimitive {
        ScenePrimitive {
            elapsed: self.elapsed.as_secs_f32(),
            frame: self.frame.clone(),
        }
    }
}

/// What the shell hands the renderer for one frame.
#[derive(Debug)]
pub struct ScenePrimitive {
    elapsed: f32,
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
    /// The mapped frame must outlive the textures: dropping it returns the
    /// surface to the decoder's pool.
    Imported {
        _frame: DrmFrame,
        _planes: Planes,
    },
    Failed,
}

impl ScenePipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
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
            size: std::mem::size_of::<[f32; 4]>() as u64,
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

    pub fn prepare(
        &mut self,
        primitive: &ScenePrimitive,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        if !self.reported {
            self.reported = true;
            println!("iced device: {}", dmabuf::device_report(device));
        }
        if let Some(frame) = primitive.frame.as_ref() {
            self.load_once(frame, device);
        }

        let has_frame = matches!(self.source, Some(Source::Imported { .. }));
        let params: [f32; 4] = [
            primitive.elapsed,
            f32::from(u8::from(has_frame)),
            f32::from(u8::from(self.linearize)),
            0.0,
        ];
        queue.write_buffer(&self.uniforms, 0, bytes_of(&params));
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
        let (mapped, size) = media::first_frame(&frame.path, 0)?;
        println!("drm:    {}", mapped.describe());
        let planes = dmabuf::import(device, mapped.descriptor(), size)?;
        self.bind_group = bind(device, &self.layout, &self.uniforms, &planes, &self.sampler);
        println!(
            "import: {} x {} imported into iced's device",
            size.width, size.height
        );
        Ok(Source::Imported {
            _frame: mapped,
            _planes: planes,
        })
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
                    min_binding_size: None,
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

fn bytes_of(params: &[f32; 4]) -> &[u8] {
    // `[f32; 4]` has no padding and no invalid bit patterns.
    unsafe {
        std::slice::from_raw_parts(params.as_ptr().cast::<u8>(), std::mem::size_of_val(params))
    }
}

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

// x: seconds since start, y: 1 when a frame is bound, z: 1 when the target
// is sRGB and the GPU will encode whatever this shader writes.
@group(0) @binding(0) var<uniform> params: vec4<f32>;
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
  // Both branches are evaluated because `textureSample` needs uniform control
  // flow; picking afterwards is the WGSL-legal way to write this.
  let rgb = select(gradient(in.uv, params.x), nv12(in.uv), params.y > 0.5);
  return vec4<f32>(select(rgb, linearize(rgb), params.z > 0.5), 1.0);
}
"#;
