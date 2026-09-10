//! GPU expansion and confidence packing for captured temporal motion grids.
//!
//! This implementation records one render pass. Raw search results are supplied
//! as CPU records or a validated GPU refinement output. The resident entry also
//! consumes the typed coarse-motion pyramid's luma texture, avoiding its former
//! readback and re-upload. No history selection, submission or wait is implied.

use std::fmt;

use super::{
    Geometry, Parameters, ResidentParameters, fcvtzs_i32, validate_count, validate_resident_count,
};
use crate::temporal_fusion::parallel_refine::{self, coarse::gpu::MotionPyramid};
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
    ResidentLuma,
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
            Self::ResidentLuma => {
                formatter.write_str("resident motion luma does not match the packing output grid")
            }
        }
    }
}

impl std::error::Error for Error {}

pub struct Builder {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    resident_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    resident_pipeline: wgpu::RenderPipeline,
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
        let resident_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("resident temporal motion packing"),
            entries: &[
                storage(0),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("temporal motion packing"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu.wgsl").into()),
        });
        let make_pipeline = |label, layout: &wgpu::BindGroupLayout, entry_point| {
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
                    entry_point: Some(entry_point),
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
            })
        };
        let pipeline = make_pipeline("temporal motion packing", &layout, "pack_motion");
        let resident_pipeline = make_pipeline(
            "resident temporal motion packing",
            &resident_layout,
            "pack_motion_resident",
        );
        Self {
            device: device.clone(),
            layout,
            resident_layout,
            pipeline,
            resident_pipeline,
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
        if ordinal >= refined.references || refined.blocks != parameters.geometry.raw_grid {
            return Err(Error::RefinementLayout);
        }
        let count = refined.blocks[0]
            .checked_mul(refined.blocks[1])
            .ok_or(Error::RefinementLayout)?;
        if refined.raw.size() != u64::from(count) * u64::from(refined.references) * 12 {
            return Err(Error::RefinementLayout);
        }
        let mut prepared = Prepared::from_count(count as usize, parameters)?;
        prepared.uniform[6] = ordinal * count;
        self.encode_buffer(device, encoder, &refined.raw, parameters, prepared)
    }

    /// Pack one refined reference while its exact luma classification remains
    /// in the typed resident motion pyramid. This only records GPU work.
    pub fn encode_refined_resident(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        refined: &parallel_refine::gpu::Output,
        ordinal: u32,
        luma: &MotionPyramid,
        parameters: &ResidentParameters<'_>,
    ) -> Result<wgpu::Texture, Error> {
        if self.device != *device || refined.device != *device || luma.device() != device {
            return Err(Error::ForeignDevice);
        }
        if ordinal >= refined.references || refined.blocks != parameters.geometry.raw_grid {
            return Err(Error::RefinementLayout);
        }
        let count = refined.blocks[0]
            .checked_mul(refined.blocks[1])
            .ok_or(Error::RefinementLayout)?;
        if refined.raw.size() != u64::from(count) * u64::from(refined.references) * 12 {
            return Err(Error::RefinementLayout);
        }
        let resident = luma.luma();
        let size = resident.size();
        if [size.width, size.height] != parameters.geometry.output_grid
            || size.depth_or_array_layers != 1
            || resident.mip_level_count() != 1
            || resident.sample_count() != 1
            || resident.dimension() != wgpu::TextureDimension::D2
            || resident.format() != wgpu::TextureFormat::R8Uint
            || !resident
                .usage()
                .contains(wgpu::TextureUsages::TEXTURE_BINDING)
        {
            return Err(Error::ResidentLuma);
        }
        let mut prepared = Prepared::from_resident_count(count as usize, parameters)?;
        prepared.uniform[6] = ordinal * count;
        self.encode_with_luma(
            device,
            encoder,
            &refined.raw,
            parameters.geometry,
            prepared,
            Luma::Resident(resident),
        )
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
        let luma_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("temporal motion luma indices"),
            contents: &luma_bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });
        self.encode_with_luma(
            device,
            encoder,
            raw_buffer,
            parameters.geometry,
            prepared,
            Luma::Buffer(&luma_buffer),
        )
    }

    fn encode_with_luma(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        raw_buffer: &wgpu::Buffer,
        geometry: Geometry,
        prepared: Prepared,
        luma: Luma<'_>,
    ) -> Result<wgpu::Texture, Error> {
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
        let common = |binding| wgpu::BindGroupEntry {
            binding,
            resource: match binding {
                0 => raw_buffer.as_entire_binding(),
                2 => threshold_buffer.as_entire_binding(),
                3 => uniform_buffer.as_entire_binding(),
                _ => unreachable!("motion common binding"),
            },
        };
        let (group, pipeline) = match luma {
            Luma::Buffer(luma) => (
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("temporal motion packing"),
                    layout: &self.layout,
                    entries: &[
                        common(0),
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: luma.as_entire_binding(),
                        },
                        common(2),
                        common(3),
                    ],
                }),
                &self.pipeline,
            ),
            Luma::Resident(luma) => {
                let view = luma.create_view(&Default::default());
                (
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("resident temporal motion packing"),
                        layout: &self.resident_layout,
                        entries: &[
                            common(0),
                            common(2),
                            common(3),
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                        ],
                    }),
                    &self.resident_pipeline,
                )
            }
        };
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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        Ok(output)
    }
}

enum Luma<'a> {
    Buffer(&'a wgpu::Buffer),
    Resident(&'a wgpu::Texture),
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
        Self::thresholds(
            parameters.geometry,
            parameters.confidence_y,
            parameters.confidence_uv,
            parameters.scale_base,
            parameters.scale_extra,
            parameters.temporal,
            parameters.phase,
        )
    }

    fn from_count(count: usize, parameters: &Parameters<'_>) -> Result<Self, Error> {
        Self::validate_count(count, parameters)?;
        Self::thresholds(
            parameters.geometry,
            parameters.confidence_y,
            parameters.confidence_uv,
            parameters.scale_base,
            parameters.scale_extra,
            parameters.temporal,
            parameters.phase,
        )
    }

    fn from_resident_count(
        count: usize,
        parameters: &ResidentParameters<'_>,
    ) -> Result<Self, Error> {
        validate_resident_count(count, parameters).map_err(Error::Reference)?;
        if parameters
            .geometry
            .full
            .into_iter()
            .any(|value| value > MAX_FULL_DIMENSION)
        {
            return Err(Error::FullDimensions);
        }
        Self::thresholds(
            parameters.geometry,
            parameters.confidence_y,
            parameters.confidence_uv,
            parameters.scale_base,
            parameters.scale_extra,
            parameters.temporal,
            parameters.phase,
        )
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

    fn thresholds(
        geometry: Geometry,
        confidence_y: &[f32; 256],
        confidence_uv: &[f32; 256],
        scale_base: i32,
        scale_extra: i32,
        temporal: f32,
        phase: f64,
    ) -> Result<Self, Error> {
        // Keep these operations in the CPU oracle's exact scalar order. The
        // resulting thresholds are integer shader inputs, not GPU f64 work.
        let blend64 = (temporal as f64).mul_add(1.0 - phase, phase);
        let blend32 = blend64 as f32;
        let base = scale_base.wrapping_mul(scale_extra);
        let scale = fcvtzs_i32((base as f32).mul_add(blend32, 0.5), "confidence scale")
            .map_err(Error::Reference)?;
        let mut thresholds = Vec::with_capacity(512);
        for (name, table) in [
            ("Y threshold", confidence_y),
            ("UV threshold", confidence_uv),
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
