//! GPU expansion and confidence packing for captured temporal motion grids.
//!
//! This implementation records one render pass. Raw search results are supplied
//! as CPU records or a validated GPU refinement output. Luma remains an explicit
//! CPU upload; no history selection, submission, wait or readback is implied.

use std::fmt;

use super::{Parameters, fcvtzs_i32, validate_count};
use crate::temporal_fusion::parallel_refine;
use wgpu::util::DeviceExt;

const MAX_FULL_DIMENSION: u32 = 32_768;
const MAX_RAW_COST: i32 = 65_280;
const MAX_THRESHOLD: i32 = 46_340;

#[derive(Debug, Eq, PartialEq)]
pub enum Error {
    Reference(String),
    ForeignDevice,
    FullDimensions,
    RawDisplacement,
    RawCost,
    Threshold,
    RefinementLayout,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reference(message) => formatter.write_str(message),
            Self::ForeignDevice => {
                formatter.write_str("motion builder belongs to a different graphics device")
            }
            Self::FullDimensions => formatter.write_str(
                "GPU motion packing supports full image dimensions no larger than 32768",
            ),
            Self::RawDisplacement => {
                formatter.write_str("GPU motion packing needs raw displacements that fit i16")
            }
            Self::RawCost => formatter
                .write_str("GPU motion packing needs nonnegative raw costs no larger than 65280"),
            Self::Threshold => formatter.write_str(
                "GPU motion packing needs each positive confidence threshold no larger than 46340",
            ),
            Self::RefinementLayout => formatter
                .write_str("refined motion reference or dimensions do not match the packing grid"),
        }
    }
}

impl std::error::Error for Error {}

pub struct Builder {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl Builder {
    pub fn new(device: &wgpu::Device) -> Self {
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("temporal motion packing"),
            entries: &[
                storage(0),
                storage(1),
                storage(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("temporal motion packing"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("temporal motion packing"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("temporal motion packing"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("triangle"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("pack_motion"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Sint,
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
        Self {
            device: device.clone(),
            layout,
            pipeline,
        }
    }

    /// Record expansion of one supplied raw motion grid.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        raw: &[[i32; 3]],
        parameters: &Parameters<'_>,
    ) -> Result<wgpu::Texture, Error> {
        if self.device != *device {
            return Err(Error::ForeignDevice);
        }
        let prepared = Prepared::new(raw, parameters)?;
        let raw_bytes: Vec<u8> = raw
            .iter()
            .flat_map(|record| record.iter().flat_map(|value| value.to_le_bytes()))
            .collect();
        let raw_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("temporal raw motion"),
            contents: &raw_bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });
        self.encode_buffer(device, encoder, &raw_buffer, parameters, prepared)
    }

    /// Pack one reference directly from validated GPU refinement, without a
    /// readback. Only this typed producer guarantees bounded displacement/SAD;
    /// accepting an arbitrary raw buffer would bypass numeric validation.
    pub fn encode_refined(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        refined: &parallel_refine::gpu::Output,
        ordinal: u32,
        parameters: &Parameters<'_>,
    ) -> Result<wgpu::Texture, Error> {
        if self.device != *device || refined.device != *device {
            return Err(Error::ForeignDevice);
        }
        if ordinal >= 6 || refined.blocks != parameters.geometry.raw_grid {
            return Err(Error::RefinementLayout);
        }
        let count = refined.blocks[0]
            .checked_mul(refined.blocks[1])
            .ok_or(Error::RefinementLayout)?;
        if refined.raw.size() != u64::from(count) * 6 * 12 {
            return Err(Error::RefinementLayout);
        }
        let mut prepared = Prepared::from_count(count as usize, parameters)?;
        prepared.uniform[6] = ordinal * count;
        self.encode_buffer(device, encoder, &refined.raw, parameters, prepared)
    }

    fn encode_buffer(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        raw_buffer: &wgpu::Buffer,
        parameters: &Parameters<'_>,
        prepared: Prepared,
    ) -> Result<wgpu::Texture, Error> {
        let luma_bytes: Vec<u8> = parameters
            .luma
            .iter()
            .flat_map(|value| u32::from(*value).to_le_bytes())
            .collect();
        let threshold_bytes: Vec<u8> = prepared
            .thresholds
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let uniform_bytes: Vec<u8> = prepared
            .uniform
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let buffer = |label, contents: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        };
        let luma_buffer = buffer(
            "temporal motion luma indices",
            &luma_bytes,
            wgpu::BufferUsages::STORAGE,
        );
        let threshold_buffer = buffer(
            "temporal motion thresholds",
            &threshold_bytes,
            wgpu::BufferUsages::STORAGE,
        );
        let uniform_buffer = buffer(
            "temporal motion geometry",
            &uniform_bytes,
            wgpu::BufferUsages::UNIFORM,
        );
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("temporal motion packing"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: raw_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: luma_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: threshold_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });
        let geometry = parameters.geometry;
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("temporal packed motion"),
            size: wgpu::Extent3d {
                width: geometry.output_grid[0],
                height: geometry.output_grid[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Sint,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = output.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("temporal motion packing"),
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        Ok(output)
    }
}

struct Prepared {
    thresholds: Vec<i32>,
    uniform: [u32; 8],
}

impl Prepared {
    fn new(raw: &[[i32; 3]], parameters: &Parameters<'_>) -> Result<Self, Error> {
        Self::validate_count(raw.len(), parameters)?;
        if raw.iter().any(|record| {
            !(i32::from(i16::MIN)..=i32::from(i16::MAX)).contains(&record[0])
                || !(i32::from(i16::MIN)..=i32::from(i16::MAX)).contains(&record[1])
        }) {
            return Err(Error::RawDisplacement);
        }
        if raw
            .iter()
            .any(|record| !(0..=MAX_RAW_COST).contains(&record[2]))
        {
            return Err(Error::RawCost);
        }
        Self::thresholds(parameters)
    }

    fn from_count(count: usize, parameters: &Parameters<'_>) -> Result<Self, Error> {
        Self::validate_count(count, parameters)?;
        Self::thresholds(parameters)
    }

    fn validate_count(count: usize, parameters: &Parameters<'_>) -> Result<(), Error> {
        validate_count(count, parameters).map_err(Error::Reference)?;
        if parameters
            .geometry
            .full
            .into_iter()
            .any(|value| value > MAX_FULL_DIMENSION)
        {
            return Err(Error::FullDimensions);
        }
        Ok(())
    }

    fn thresholds(parameters: &Parameters<'_>) -> Result<Self, Error> {
        let geometry = parameters.geometry;

        // Keep these operations in the CPU oracle's exact scalar order. The
        // resulting thresholds are integer shader inputs, not GPU f64 work.
        let blend64 =
            (parameters.temporal as f64).mul_add(1.0 - parameters.phase, parameters.phase);
        let blend32 = blend64 as f32;
        let base = parameters.scale_base.wrapping_mul(parameters.scale_extra);
        let scale = fcvtzs_i32((base as f32).mul_add(blend32, 0.5), "confidence scale")
            .map_err(Error::Reference)?;
        let mut thresholds = Vec::with_capacity(512);
        for (name, table) in [
            ("Y threshold", parameters.confidence_y),
            ("UV threshold", parameters.confidence_uv),
        ] {
            for value in table {
                let threshold =
                    fcvtzs_i32(*value * scale as f32, name).map_err(Error::Reference)?;
                if threshold > MAX_THRESHOLD {
                    return Err(Error::Threshold);
                }
                thresholds.push(threshold);
            }
        }
        Ok(Self {
            thresholds,
            uniform: [
                geometry.raw_grid[0],
                geometry.raw_grid[1],
                geometry.output_grid[0],
                geometry.output_grid[1],
                geometry.full[0],
                geometry.full[1],
                0,
                0,
            ],
        })
    }
}

#[cfg(test)]
mod tests;
