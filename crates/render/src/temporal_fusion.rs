//! Motion-compensated fusion after stitching, not gradual color-map updates.
//!
//! The normalized law and explicit inputs are documented in
//! `docs/research/studio-image-fusion-temporal-602.md`. This module owns no
//! history policy: callers supply the current image, ordered references,
//! motion/confidence textures and effective parameters. In particular it
//! cannot substitute a stale map for a new source or infer a seek history.
//!
//! Two ordinary render passes store Y/R8 and UV/RG8, preserving normalized
//! 8-bit output without requiring optional narrow storage-texture formats.
//! Studio uses compute dispatches; this is a Kjerag execution choice. Encoding
//! never submits, waits, maps buffers or reads pixels back to the CPU.

use wgpu::util::DeviceExt;

/// Readable CPU motion-packing reference, not a selected playback path.
pub mod motion;

/// Readable CPU pyramid reference, with explicitly supplied base images.
pub mod pyramid;

/// Effective values supplied by the calibration/history producer, not defaults.
pub struct Parameters {
    pub noise: f32,
    pub limit: f32,
    pub y_limits: [f32; 256],
    pub uv_limits: [f32; 256],
    pub current_layer: u32,
    pub reference_layers: Vec<u32>,
}

/// Prepared full-range source planes and current-to-reference motion.
///
/// Y and UV are arrays sharing layer order. Flow is RGBA16Sint, one layer
/// per reference in parameter order: signed xy displacement in Y pixels,
/// Y confidence in z, UV confidence in w. Luma is an R8Uint table index grid.
pub struct Inputs<'a> {
    pub y: &'a wgpu::Texture,
    pub uv: &'a wgpu::Texture,
    pub flow: &'a wgpu::Texture,
    pub luma: &'a wgpu::Texture,
}

/// GPU-owned result for the requested even-aligned region, with local origin.
/// Submission completion and the source-frame stamp remain the caller's job.
pub struct Output {
    pub y: wgpu::Texture,
    pub uv: wgpu::Texture,
}

pub struct GpuFuse {
    layout: wgpu::BindGroupLayout,
    y: wgpu::RenderPipeline,
    uv: wgpu::RenderPipeline,
}

impl GpuFuse {
    pub fn new(device: &wgpu::Device) -> Self {
        let sampled = |binding, sample_type, view_dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension,
                multisampled: false,
            },
            count: None,
        };
        let float = wgpu::TextureSampleType::Float { filterable: false };
        let array = wgpu::TextureViewDimension::D2Array;
        let buffer = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("temporal pixel fusion"),
            entries: &[
                sampled(0, float, array),
                sampled(1, float, array),
                sampled(2, wgpu::TextureSampleType::Sint, array),
                sampled(
                    3,
                    wgpu::TextureSampleType::Uint,
                    wgpu::TextureViewDimension::D2,
                ),
                buffer(4, wgpu::BufferBindingType::Uniform),
                buffer(5, wgpu::BufferBindingType::Storage { read_only: true }),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("temporal pixel fusion"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("temporal pixel fusion"),
            source: wgpu::ShaderSource::Wgsl(include_str!("temporal_fusion.wgsl").into()),
        });
        let pipeline = |entry, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("triangle"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
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
        let y = pipeline("fuse_y", wgpu::TextureFormat::R8Unorm);
        let uv = pipeline("fuse_uv", wgpu::TextureFormat::Rg8Unorm);
        Self { layout, y, uv }
    }

    /// Encode one supplied temporal window. No frame scheduling or CPU wait.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        inputs: Inputs<'_>,
        params: &Parameters,
        roi: [u32; 4],
    ) -> Result<Output, String> {
        validate(&inputs, params, roi)?;
        // Two vec4 layer-index groups keep the uniform's alignment explicit.
        let mut words = [0u32; 16];
        words[..4].copy_from_slice(&roi);
        words[4] = params.noise.to_bits();
        words[5] = params.limit.to_bits();
        words[6] = params.current_layer;
        words[7] = params.reference_layers.len() as u32;
        words[8..8 + params.reference_layers.len()].copy_from_slice(&params.reference_layers);
        let config: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let tables: Vec<u8> = params
            .y_limits
            .iter()
            .chain(&params.uv_limits)
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let config = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("temporal fusion parameters"),
            contents: &config,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let tables = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("temporal fusion limits"),
            contents: &tables,
            usage: wgpu::BufferUsages::STORAGE,
        });
        let array_view = |texture: &wgpu::Texture| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let y = array_view(inputs.y);
        let uv = array_view(inputs.uv);
        let flow = array_view(inputs.flow);
        let luma = inputs.luma.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("temporal fusion supplied window"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&y),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&uv),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&flow),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&luma),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: config.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: tables.as_entire_binding(),
                },
            ],
        });
        let target = |label, width, height, format| {
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
        };
        let output = Output {
            y: target(
                "temporally fused Y",
                roi[2],
                roi[3],
                wgpu::TextureFormat::R8Unorm,
            ),
            uv: target(
                "temporally fused UV",
                roi[2] / 2,
                roi[3] / 2,
                wgpu::TextureFormat::Rg8Unorm,
            ),
        };
        for (texture, pipeline) in [(&output.y, &self.y), (&output.uv, &self.uv)] {
            let view = texture.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("temporal pixel fusion"),
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
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        Ok(output)
    }
}

fn validate(inputs: &Inputs<'_>, params: &Parameters, roi: [u32; 4]) -> Result<(), String> {
    for (texture, format) in [
        (inputs.y, wgpu::TextureFormat::R8Unorm),
        (inputs.uv, wgpu::TextureFormat::Rg8Unorm),
        (inputs.flow, wgpu::TextureFormat::Rgba16Sint),
        (inputs.luma, wgpu::TextureFormat::R8Uint),
    ] {
        if texture.format() != format
            || texture.dimension() != wgpu::TextureDimension::D2
            || texture.mip_level_count() != 1
            || texture.sample_count() != 1
            || !texture
                .usage()
                .contains(wgpu::TextureUsages::TEXTURE_BINDING)
        {
            return Err(
                "temporal fusion needs single-mip sampled Y, UV, motion and luma textures".into(),
            );
        }
    }
    let y = inputs.y.size();
    let uv = inputs.uv.size();
    let layers = y.depth_or_array_layers;
    if !y.width.is_multiple_of(2)
        || !y.height.is_multiple_of(2)
        || uv.width != y.width / 2
        || uv.height != y.height / 2
        || uv.depth_or_array_layers != layers
        || !(2..=7).contains(&layers)
        || inputs.luma.depth_or_array_layers() != 1
    {
        return Err("temporal fusion needs matching even-sized NV12 image layers".into());
    }
    let count = params.reference_layers.len();
    if !(1..=6).contains(&count)
        || inputs.flow.depth_or_array_layers() != count as u32
        || params.current_layer >= layers
        || params
            .reference_layers
            .iter()
            .enumerate()
            .any(|(i, &layer)| {
                layer >= layers
                    || layer == params.current_layer
                    || params.reference_layers[..i].contains(&layer)
            })
    {
        return Err(
            "temporal fusion needs distinct current and reference layers with matching motion"
                .into(),
        );
    }
    let fits = |start: u32, length: u32, full: u32| {
        length > 0 && start.checked_add(length).is_some_and(|end| end <= full)
    };
    if roi.iter().any(|v| !v.is_multiple_of(2))
        || !fits(roi[0], roi[2], y.width)
        || !fits(roi[1], roi[3], y.height)
    {
        return Err("temporal fusion needs an even-aligned region inside the source image".into());
    }
    if !params.noise.is_finite()
        || params.noise < 0.0
        || !params.limit.is_finite()
        || params.limit < 0.0
        || params
            .y_limits
            .iter()
            .chain(&params.uv_limits)
            .any(|v| !v.is_finite() || *v < 0.0)
    {
        return Err("temporal fusion parameters must be finite and nonnegative".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
