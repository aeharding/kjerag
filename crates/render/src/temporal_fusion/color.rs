//! Diagnostic gamma-RGB/full-range-NV12 conversion for temporal A/A checks.
//!
//! This is an explicit Kjerag representation choice, not a recovered Studio
//! conversion. Chroma is the arithmetic mean of each exact 2x2 RGB footprint
//! and is reconstructed by bilinear interpolation between those footprint
//! centres. Encoding records render passes only; submission and frame identity
//! remain the caller's responsibility.

use std::fmt;
use wgpu::util::DeviceExt;

/// The four coefficients consumed by Kjerag's `source_rgb` conversion.
///
/// For centred `[Cb, Cr]`, the forward source law is
/// `R=Y+r_cr*Cr`, `G=Y-g_cb*Cb-g_cr*Cr`, `B=Y+b_cb*Cb`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatrixCoefficients {
    pub r_cr: f32,
    pub g_cb: f32,
    pub g_cr: f32,
    pub b_cb: f32,
}

impl MatrixCoefficients {
    pub const fn from_source_rgb(values: [f32; 4]) -> Self {
        Self {
            r_cr: values[0],
            g_cb: values[1],
            g_cr: values[2],
            b_cb: values[3],
        }
    }

    fn words(self) -> [f32; 4] {
        [self.r_cr, self.g_cb, self.g_cr, self.b_cb]
    }

    fn validate(self) -> Result<(), Error> {
        let [r_cr, g_cb, g_cr, b_cb] = self.words();
        let denominator = 1.0 + g_cb / b_cb + g_cr / r_cr;
        if [r_cr, g_cb, g_cr, b_cb, denominator]
            .into_iter()
            .all(f32::is_finite)
            && r_cr != 0.0
            && b_cb != 0.0
            && denominator != 0.0
        {
            Ok(())
        } else {
            Err(Error::Coefficients)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Coefficients,
    RgbTexture,
    Nv12Textures,
    ForeignNv12,
    MatrixMismatch,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Coefficients => "RGB/NV12 matrix coefficients must have a finite inverse",
            Self::RgbTexture => {
                "RGB/NV12 conversion needs one even-sized sampled Rgba8Unorm texture"
            }
            Self::Nv12Textures => {
                "RGB/NV12 conversion needs matching sampled R8Unorm and Rg8Unorm planes"
            }
            Self::ForeignNv12 => "NV12 planes belong to a different color converter device",
            Self::MatrixMismatch => "NV12 reverse conversion needs its originating source matrix",
        })
    }
}

impl std::error::Error for Error {}

/// GPU-owned full-range NV12 planes produced on one converter device.
pub struct Nv12 {
    pub y: wgpu::Texture,
    pub uv: wgpu::Texture,
    device: wgpu::Device,
    coefficients: MatrixCoefficients,
}

impl Nv12 {
    pub fn size(&self) -> wgpu::Extent3d {
        self.y.size()
    }
}

/// Render-pass implementation of the disclosed diagnostic conversion.
pub struct GpuColorConversion {
    device: wgpu::Device,
    rgb_layout: wgpu::BindGroupLayout,
    nv12_layout: wgpu::BindGroupLayout,
    y_pipeline: wgpu::RenderPipeline,
    uv_pipeline: wgpu::RenderPipeline,
    rgb_pipeline: wgpu::RenderPipeline,
}

impl GpuColorConversion {
    pub fn new(device: &wgpu::Device) -> Self {
        let sampled = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let rgb_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("diagnostic RGB to NV12"),
            entries: &[sampled(0), uniform(1)],
        });
        let nv12_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("diagnostic NV12 to RGB"),
            entries: &[sampled(0), sampled(1), uniform(2)],
        });
        let rgb_to_nv12 = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("diagnostic RGB to NV12"),
            source: wgpu::ShaderSource::Wgsl(RGB_TO_NV12_WGSL.into()),
        });
        let nv12_to_rgb = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("diagnostic NV12 to RGB"),
            source: wgpu::ShaderSource::Wgsl(NV12_TO_RGB_WGSL.into()),
        });
        let make_pipeline =
            |label, layout: &wgpu::BindGroupLayout, shader: &wgpu::ShaderModule, entry, format| {
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(label),
                        bind_group_layouts: &[layout],
                        immediate_size: 0,
                    });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: shader,
                        entry_point: Some("triangle"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: shader,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
            };
        let y_pipeline = make_pipeline(
            "diagnostic RGB to Y",
            &rgb_layout,
            &rgb_to_nv12,
            "rgb_to_y",
            wgpu::TextureFormat::R8Unorm,
        );
        let uv_pipeline = make_pipeline(
            "diagnostic RGB to UV",
            &rgb_layout,
            &rgb_to_nv12,
            "rgb_to_uv",
            wgpu::TextureFormat::Rg8Unorm,
        );
        let rgb_pipeline = make_pipeline(
            "diagnostic NV12 to RGB",
            &nv12_layout,
            &nv12_to_rgb,
            "nv12_to_rgb",
            wgpu::TextureFormat::Rgba8Unorm,
        );
        Self {
            device: device.clone(),
            rgb_layout,
            nv12_layout,
            y_pipeline,
            uv_pipeline,
            rgb_pipeline,
        }
    }

    /// Record full-range gamma-RGB to NV12 conversion without submitting it.
    ///
    /// wgpu validates that the externally owned RGB texture belongs to this
    /// device when the bind group is created; wgpu exposes no portable device
    /// identity on a texture for an earlier Rust-side check.
    pub fn encode_rgb_to_nv12(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        rgb: &wgpu::Texture,
        coefficients: MatrixCoefficients,
    ) -> Result<Nv12, Error> {
        coefficients.validate()?;
        validate_rgb(rgb)?;
        let size = rgb.size();
        let parameters = uniform(&self.device, coefficients, "diagnostic RGB to NV12 matrix");
        let rgb_view = rgb.create_view(&Default::default());
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("diagnostic RGB to NV12"),
            layout: &self.rgb_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&rgb_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: parameters.as_entire_binding(),
                },
            ],
        });
        let output = Nv12 {
            y: target(
                &self.device,
                "diagnostic full-range Y",
                size.width,
                size.height,
                wgpu::TextureFormat::R8Unorm,
            ),
            uv: target(
                &self.device,
                "diagnostic full-range UV",
                size.width / 2,
                size.height / 2,
                wgpu::TextureFormat::Rg8Unorm,
            ),
            device: self.device.clone(),
            coefficients,
        };
        draw(encoder, &output.y, &self.y_pipeline, &group);
        draw(encoder, &output.uv, &self.uv_pipeline, &group);
        Ok(output)
    }

    /// Record NV12 to gamma-RGB conversion using centred-2x2 bilinear chroma.
    pub fn encode_nv12_to_rgb(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        nv12: &Nv12,
        coefficients: MatrixCoefficients,
    ) -> Result<wgpu::Texture, Error> {
        coefficients.validate()?;
        if self.device != nv12.device {
            return Err(Error::ForeignNv12);
        }
        if coefficients != nv12.coefficients {
            return Err(Error::MatrixMismatch);
        }
        self.encode_planes_to_rgb(encoder, &nv12.y, &nv12.uv, coefficients)
    }

    /// Record explicit external NV12 planes to gamma-RGB without submitting.
    ///
    /// This is the same disclosed centered-2x2 reconstruction used for this
    /// converter's owned [`Nv12`], but carries no frame-stamp or history claim.
    /// wgpu validates external texture and encoder device ownership when the
    /// resources are bound and the pass is recorded; texture device identity
    /// is not exposed for an earlier portable Rust-side check.
    pub fn encode_planes_to_rgb(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        y: &wgpu::Texture,
        uv: &wgpu::Texture,
        coefficients: MatrixCoefficients,
    ) -> Result<wgpu::Texture, Error> {
        coefficients.validate()?;
        validate_planes(y, uv)?;
        Ok(self.encode_validated_planes_to_rgb(encoder, y, uv, coefficients))
    }

    fn encode_validated_planes_to_rgb(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        y: &wgpu::Texture,
        uv: &wgpu::Texture,
        coefficients: MatrixCoefficients,
    ) -> wgpu::Texture {
        let parameters = uniform(&self.device, coefficients, "diagnostic NV12 to RGB matrix");
        let y_view = y.create_view(&Default::default());
        let uv_view = uv.create_view(&Default::default());
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("diagnostic NV12 to RGB"),
            layout: &self.nv12_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&y_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&uv_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: parameters.as_entire_binding(),
                },
            ],
        });
        let size = y.size();
        let output = target(
            &self.device,
            "diagnostic NV12 round-trip RGB",
            size.width,
            size.height,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        draw(encoder, &output, &self.rgb_pipeline, &group);
        output
    }
}

fn uniform(
    device: &wgpu::Device,
    coefficients: MatrixCoefficients,
    label: &'static str,
) -> wgpu::Buffer {
    let bytes: Vec<u8> = coefficients
        .words()
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: &bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    })
}

fn target(
    device: &wgpu::Device,
    label: &'static str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn draw(
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::Texture,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
) {
    let view = target.create_view(&Default::default());
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("diagnostic RGB/NV12 conversion"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
}

fn sampled_2d(texture: &wgpu::Texture, format: wgpu::TextureFormat) -> bool {
    texture.format() == format
        && texture.dimension() == wgpu::TextureDimension::D2
        && texture.mip_level_count() == 1
        && texture.sample_count() == 1
        && texture.depth_or_array_layers() == 1
        && texture
            .usage()
            .contains(wgpu::TextureUsages::TEXTURE_BINDING)
}

fn validate_rgb(rgb: &wgpu::Texture) -> Result<(), Error> {
    let size = rgb.size();
    if sampled_2d(rgb, wgpu::TextureFormat::Rgba8Unorm)
        && size.width.is_multiple_of(2)
        && size.height.is_multiple_of(2)
    {
        Ok(())
    } else {
        Err(Error::RgbTexture)
    }
}

fn validate_planes(y: &wgpu::Texture, uv: &wgpu::Texture) -> Result<(), Error> {
    let y_size = y.size();
    let uv_size = uv.size();
    if sampled_2d(y, wgpu::TextureFormat::R8Unorm)
        && sampled_2d(uv, wgpu::TextureFormat::Rg8Unorm)
        && y_size.width.is_multiple_of(2)
        && y_size.height.is_multiple_of(2)
        && uv_size.width == y_size.width / 2
        && uv_size.height == y_size.height / 2
    {
        Ok(())
    } else {
        Err(Error::Nv12Textures)
    }
}

#[cfg(test)]
fn rgb_to_ycbcr(rgb: [f32; 3], matrix: MatrixCoefficients) -> [f32; 3] {
    let denominator = 1.0 + matrix.g_cb / matrix.b_cb + matrix.g_cr / matrix.r_cr;
    let y = (rgb[1] + matrix.g_cb * rgb[2] / matrix.b_cb + matrix.g_cr * rgb[0] / matrix.r_cr)
        / denominator;
    [y, (rgb[2] - y) / matrix.b_cb, (rgb[0] - y) / matrix.r_cr]
}

#[cfg(test)]
fn ycbcr_to_rgb(ycbcr: [f32; 3], matrix: MatrixCoefficients) -> [f32; 3] {
    let [y, cb, cr] = ycbcr;
    [
        y + matrix.r_cr * cr,
        y - matrix.g_cb * cb - matrix.g_cr * cr,
        y + matrix.b_cb * cb,
    ]
}

const RGB_TO_NV12_WGSL: &str = r#"
struct Matrix { value: vec4<f32>, }
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> matrix: Matrix;

struct Vertex { @builtin(position) position: vec4<f32>, }
@vertex fn triangle(@builtin(vertex_index) vertex: u32) -> Vertex {
  let positions = array<vec2<f32>, 3>(
    vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
  return Vertex(vec4(positions[vertex], 0.0, 1.0));
}

fn convert(rgb: vec3<f32>) -> vec3<f32> {
  let m = matrix.value;
  let y = (rgb.g + m.y * rgb.b / m.w + m.z * rgb.r / m.x) /
    (1.0 + m.y / m.w + m.z / m.x);
  return vec3(y, (rgb.b - y) / m.w, (rgb.r - y) / m.x);
}

@fragment fn rgb_to_y(@builtin(position) position: vec4<f32>) -> @location(0) f32 {
  let rgb = textureLoad(source, vec2<i32>(position.xy), 0).rgb;
  return clamp(convert(rgb).x, 0.0, 1.0);
}

@fragment fn rgb_to_uv(@builtin(position) position: vec4<f32>) -> @location(0) vec2<f32> {
  let base = vec2<i32>(position.xy) * 2;
  let rgb = (textureLoad(source, base, 0).rgb +
    textureLoad(source, base + vec2(1, 0), 0).rgb +
    textureLoad(source, base + vec2(0, 1), 0).rgb +
    textureLoad(source, base + vec2(1, 1), 0).rgb) * 0.25;
  return clamp(convert(rgb).yz + vec2(0.50196081399917603), vec2(0.0), vec2(1.0));
}
"#;

const NV12_TO_RGB_WGSL: &str = r#"
struct Matrix { value: vec4<f32>, }
@group(0) @binding(0) var source_y: texture_2d<f32>;
@group(0) @binding(1) var source_uv: texture_2d<f32>;
@group(0) @binding(2) var<uniform> matrix: Matrix;

struct Vertex { @builtin(position) position: vec4<f32>, }
@vertex fn triangle(@builtin(vertex_index) vertex: u32) -> Vertex {
  let positions = array<vec2<f32>, 3>(
    vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
  return Vertex(vec4(positions[vertex], 0.0, 1.0));
}

fn centred_chroma(position: vec2<f32>) -> vec2<f32> {
  // A UV texel represents the centre of a 2x2 luma footprint. In UV texel
  // coordinates that centre is an integer, hence q = luma_position/2 - 1/2.
  let q = position * 0.5 - vec2(0.5);
  let low = vec2<i32>(floor(q));
  let fraction = fract(q);
  let limit = vec2<i32>(textureDimensions(source_uv)) - vec2(1);
  let at = clamp(low, vec2(0), limit);
  let right = clamp(low + vec2(1, 0), vec2(0), limit);
  let down = clamp(low + vec2(0, 1), vec2(0), limit);
  let diagonal = clamp(low + vec2(1, 1), vec2(0), limit);
  let top = mix(textureLoad(source_uv, at, 0).rg,
    textureLoad(source_uv, right, 0).rg, fraction.x);
  let bottom = mix(textureLoad(source_uv, down, 0).rg,
    textureLoad(source_uv, diagonal, 0).rg, fraction.x);
  return mix(top, bottom, fraction.y) - vec2(0.50196081399917603);
}

@fragment fn nv12_to_rgb(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let y = textureLoad(source_y, vec2<i32>(position.xy), 0).r;
  let c = centred_chroma(position.xy);
  let m = matrix.value;
  let rgb = vec3(y + m.x * c.y, y - m.y * c.x - m.z * c.y, y + m.w * c.x);
  return vec4(clamp(rgb, vec3(0.0), vec3(1.0)), 1.0);
}
"#;

#[cfg(test)]
mod tests;
