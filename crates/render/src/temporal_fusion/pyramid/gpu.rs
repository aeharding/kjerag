//! GPU preparation for the selected temporal luma pyramid arithmetic.
//!
//! This module only records render passes. The caller owns submission,
//! completion, source association and history. Narrow integer render targets
//! preserve the CPU reference's separately rounded vertical and horizontal
//! reductions.

use std::fmt;

/// GPU-owned logical pyramid levels, beginning with the supplied or derived base.
pub struct Output {
    pub levels: Vec<wgpu::Texture>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    ForeignDevice,
    InputTexture,
    EmptyPyramid,
    OddLumaDimensions,
    OddReduction {
        level: usize,
        width: u32,
        height: u32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ForeignDevice => {
                formatter.write_str("pyramid builder belongs to a different graphics device")
            }
            Self::InputTexture => formatter
                .write_str("pyramid input must be a nonempty, single-layer sampled R8 texture"),
            Self::EmptyPyramid => formatter.write_str("pyramid level count must be nonzero"),
            Self::OddLumaDimensions => formatter.write_str(
                "full-resolution luma dimensions must be even for the half-size gray base",
            ),
            Self::OddReduction {
                level,
                width,
                height,
            } => write!(
                formatter,
                "pyramid level {level} is {width}x{height}; both dimensions must be even to reduce it"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Render-pass encoder for half-resolution gray preparation and its pyramid.
pub struct Builder {
    device: wgpu::Device,
    float_layout: wgpu::BindGroupLayout,
    uint_layout: wgpu::BindGroupLayout,
    half_y: wgpu::RenderPipeline,
    copy: wgpu::RenderPipeline,
    vertical: wgpu::RenderPipeline,
    horizontal: wgpu::RenderPipeline,
}

impl Builder {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture_entry = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let float_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("temporal pyramid full luma"),
            entries: &[texture_entry(
                0,
                wgpu::TextureSampleType::Float { filterable: false },
            )],
        });
        let uint_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("temporal pyramid gray level"),
            entries: &[texture_entry(1, wgpu::TextureSampleType::Uint)],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("temporal luma pyramid"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu.wgsl").into()),
        });
        let pipeline = |label, layout: &wgpu::BindGroupLayout, entry| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[layout],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
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
                        format: wgpu::TextureFormat::R8Uint,
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
        let half_y = pipeline("temporal half-size luma", &float_layout, "half_y");
        let copy = pipeline("temporal pyramid base copy", &uint_layout, "copy_gray");
        let vertical = pipeline(
            "temporal pyramid vertical reduction",
            &uint_layout,
            "reduce_vertical",
        );
        let horizontal = pipeline(
            "temporal pyramid horizontal reduction",
            &uint_layout,
            "reduce_horizontal",
        );
        Self {
            device: device.clone(),
            float_layout,
            uint_layout,
            half_y,
            copy,
            vertical,
            horizontal,
        }
    }

    /// Record an exact 2x2 rounded average from full-resolution R8Unorm Y,
    /// followed by the requested logical gray pyramid.
    pub fn encode_luma(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        luma: &wgpu::Texture,
        level_count: usize,
    ) -> Result<Output, Error> {
        self.ensure_device(device)?;
        validate_input(luma, wgpu::TextureFormat::R8Unorm)?;
        let size = luma.size();
        if !size.width.is_multiple_of(2) || !size.height.is_multiple_of(2) {
            return Err(Error::OddLumaDimensions);
        }
        let base = [size.width / 2, size.height / 2];
        validate_geometry(base, level_count)?;

        let first = target(device, "temporal half-size gray", base[0], base[1]);
        self.draw_float(encoder, luma, &first, &self.half_y);
        self.finish_pyramid(device, encoder, first, level_count)
    }

    /// Copy an explicit R8Uint half-resolution base, then record its pyramid.
    pub fn encode_base(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        base: &wgpu::Texture,
        level_count: usize,
    ) -> Result<Output, Error> {
        self.ensure_device(device)?;
        validate_input(base, wgpu::TextureFormat::R8Uint)?;
        let size = base.size();
        validate_geometry([size.width, size.height], level_count)?;

        let first = target(
            device,
            "temporal pyramid copied base",
            size.width,
            size.height,
        );
        self.draw_uint(encoder, base, &first, &self.copy);
        self.finish_pyramid(device, encoder, first, level_count)
    }

    fn ensure_device(&self, device: &wgpu::Device) -> Result<(), Error> {
        if self.device == *device {
            Ok(())
        } else {
            Err(Error::ForeignDevice)
        }
    }

    fn finish_pyramid(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        first: wgpu::Texture,
        level_count: usize,
    ) -> Result<Output, Error> {
        let mut levels = Vec::with_capacity(level_count);
        levels.push(first);
        while levels.len() < level_count {
            let source = levels.last().expect("the pyramid base was inserted");
            let size = source.size();
            let intermediate = target(
                device,
                "temporal pyramid vertical intermediate",
                size.width,
                size.height / 2,
            );
            self.draw_uint(encoder, source, &intermediate, &self.vertical);
            let reduced = target(
                device,
                "temporal pyramid reduced level",
                size.width / 2,
                size.height / 2,
            );
            self.draw_uint(encoder, &intermediate, &reduced, &self.horizontal);
            levels.push(reduced);
        }
        Ok(Output { levels })
    }

    fn draw_float(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        destination: &wgpu::Texture,
        pipeline: &wgpu::RenderPipeline,
    ) {
        let source_view = source.create_view(&Default::default());
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("temporal pyramid full luma"),
            layout: &self.float_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&source_view),
            }],
        });
        draw(encoder, destination, pipeline, &group);
    }

    fn draw_uint(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        destination: &wgpu::Texture,
        pipeline: &wgpu::RenderPipeline,
    ) {
        let source_view = source.create_view(&Default::default());
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("temporal pyramid gray source"),
            layout: &self.uint_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&source_view),
            }],
        });
        draw(encoder, destination, pipeline, &group);
    }
}

fn validate_input(texture: &wgpu::Texture, format: wgpu::TextureFormat) -> Result<(), Error> {
    let size = texture.size();
    if texture.format() != format
        || texture.dimension() != wgpu::TextureDimension::D2
        || texture.mip_level_count() != 1
        || texture.sample_count() != 1
        || size.width == 0
        || size.height == 0
        || size.depth_or_array_layers != 1
        || !texture
            .usage()
            .contains(wgpu::TextureUsages::TEXTURE_BINDING)
    {
        Err(Error::InputTexture)
    } else {
        Ok(())
    }
}

fn validate_geometry(base: [u32; 2], level_count: usize) -> Result<(), Error> {
    if level_count == 0 {
        return Err(Error::EmptyPyramid);
    }
    let [mut width, mut height] = base;
    for level in 0..level_count - 1 {
        if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(Error::OddReduction {
                level,
                width,
                height,
            });
        }
        width /= 2;
        height /= 2;
    }
    Ok(())
}

fn target(device: &wgpu::Device, label: &'static str, width: u32, height: u32) -> wgpu::Texture {
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
        format: wgpu::TextureFormat::R8Uint,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn draw(
    encoder: &mut wgpu::CommandEncoder,
    destination: &wgpu::Texture,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
) {
    let view = destination.create_view(&Default::default());
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("temporal pyramid pass"),
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

#[cfg(test)]
mod tests;
