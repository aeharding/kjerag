// SPDX-License-Identifier: GPL-3.0-or-later
// Author: Manao
// Copyright(c)2006 A.G.Balakhnin aka Fizick - global motion, overlap, mode, refineMVs
//! GPU preparation of one searched coarse grid for the next finer grid.
//!
//! The global-mode and interpolation arithmetic derive from MVTools commit
//! `17250aa979616ac48dfb0e18abfdcf2bd4e3afc0` (GPL-2.0-or-later); this
//! adaptation elects GPL-3.0-or-later. The complete elected license is in
//! `temporal_fusion/LICENSE-MVTOOLS`.
//!
//! This builder accepts only the sealed output of Kjerag's GPU refinement,
//! records histogram, global-estimation and interpolation work, and returns
//! sealed buffers for the next refinement. It never submits, maps or waits.

use std::fmt;

use wgpu::util::DeviceExt;

use super::super::gpu;

const MAX_REFERENCES: u32 = 6;
const MAX_COARSE_BLOCKS: u32 = 255;
const MAX_VECTOR_MAGNITUDE: i64 = 8_192;
const FREQUENCY_SIZE: u64 = 16_384;
const RECORD_BYTES: u64 = 3 * size_of::<i32>() as u64;
const GLOBAL_BYTES: u64 = 2 * size_of::<i32>() as u64;
const LANES: u32 = 256;

// The selected producer bounds every component to [-8192, 8192), so even the
// largest admitted grid's doubled signed sum fits the shader's i32 reduction.
const _: () = assert!(
    2 * MAX_VECTOR_MAGNITUDE * MAX_COARSE_BLOCKS as i64 * MAX_COARSE_BLOCKS as i64
        <= i32::MAX as i64
);

/// Sealed GPU predictor inputs for one finer refinement.
pub struct Prepared {
    device: wgpu::Device,
    blocks: [u32; 2],
    references: u32,
    seeds: wgpu::Buffer,
    globals: wgpu::Buffer,
}

impl Prepared {
    pub fn blocks(&self) -> [u32; 2] {
        self.blocks
    }

    pub fn reference_count(&self) -> u32 {
        self.references
    }

    pub(crate) fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub(crate) fn seeds(&self) -> &wgpu::Buffer {
        &self.seeds
    }

    pub(crate) fn globals(&self) -> &wgpu::Buffer {
        &self.globals
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    ForeignDevice,
    ReferenceCount(u32),
    Geometry { coarse: [u32; 2], next: [u32; 2] },
    UnsupportedSize,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ForeignDevice => formatter
                .write_str("coarse predictor preparation belongs to a different graphics device"),
            Self::ReferenceCount(count) => write!(
                formatter,
                "coarse predictor preparation needs 1 through 6 references, got {count}"
            ),
            Self::Geometry { coarse, next } => write!(
                formatter,
                "coarse predictor preparation cannot expand {} by {} blocks to {} by {}",
                coarse[0], coarse[1], next[0], next[1]
            ),
            Self::UnsupportedSize => formatter.write_str(
                "coarse predictor preparation exceeds graphics buffer or dispatch limits",
            ),
        }
    }
}

impl std::error::Error for Error {}

pub struct Builder {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    histogram: wgpu::ComputePipeline,
    globals: wgpu::ComputePipeline,
    interpolate: wgpu::ComputePipeline,
}

impl Builder {
    pub fn new(device: &wgpu::Device) -> Self {
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("coarse temporal predictor preparation"),
            entries: &[
                storage(0, true),
                storage(1, false),
                storage(2, false),
                storage(3, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
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
            label: Some("coarse temporal predictor preparation"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("coarse temporal predictor preparation"),
            source: wgpu::ShaderSource::Wgsl(include_str!("prepare.wgsl").into()),
        });
        let pipeline = |entry_point| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("coarse temporal predictor preparation"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry_point),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            device: device.clone(),
            layout,
            histogram: pipeline("build_histograms"),
            globals: pipeline("estimate_globals"),
            interpolate: pipeline("interpolate_seeds"),
        }
    }

    /// Record exact next-grid predictor preparation for every reference.
    ///
    /// A halved image can produce either twice the coarse block count or one
    /// extra block in each dimension after integer truncation. Those are the
    /// only admitted next-grid shapes. Device equality is meaningful within
    /// the caller's one wgpu Instance; encoder provenance remains the caller's
    /// responsibility and is validated by wgpu while recording.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        coarse: &gpu::Output,
        next_blocks: [u32; 2],
    ) -> Result<Prepared, Error> {
        if self.device != *device || coarse.device != *device {
            return Err(Error::ForeignDevice);
        }
        if !(1..=MAX_REFERENCES).contains(&coarse.references) {
            return Err(Error::ReferenceCount(coarse.references));
        }
        let coarse_blocks = coarse.blocks;
        if coarse_blocks
            .into_iter()
            .any(|value| value == 0 || value > MAX_COARSE_BLOCKS)
            || !coarse_blocks
                .into_iter()
                .zip(next_blocks)
                .all(|(coarse, next)| next == coarse * 2 || next == coarse * 2 + 1)
        {
            return Err(Error::Geometry {
                coarse: coarse_blocks,
                next: next_blocks,
            });
        }

        let coarse_count = checked_area(coarse_blocks)?;
        let next_count = checked_area(next_blocks)?;
        let references = u64::from(coarse.references);
        let raw_bytes = coarse_count
            .checked_mul(references)
            .and_then(|count| count.checked_mul(RECORD_BYTES))
            .ok_or(Error::UnsupportedSize)?;
        let histogram_bytes = references
            .checked_mul(2)
            .and_then(|count| count.checked_mul(FREQUENCY_SIZE))
            .and_then(|count| count.checked_mul(size_of::<u32>() as u64))
            .ok_or(Error::UnsupportedSize)?;
        let seed_bytes = next_count
            .checked_mul(references)
            .and_then(|count| count.checked_mul(RECORD_BYTES))
            .ok_or(Error::UnsupportedSize)?;
        let global_bytes = references
            .checked_mul(GLOBAL_BYTES)
            .ok_or(Error::UnsupportedSize)?;
        let limits = device.limits();
        let binding_limit = u64::from(limits.max_storage_buffer_binding_size);
        if coarse.raw.size() != raw_bytes
            || [raw_bytes, histogram_bytes, seed_bytes, global_bytes]
                .into_iter()
                .any(|size| size > binding_limit || size > limits.max_buffer_size)
            || coarse_count.div_ceil(u64::from(LANES))
                > u64::from(limits.max_compute_workgroups_per_dimension)
            || next_count.div_ceil(u64::from(LANES))
                > u64::from(limits.max_compute_workgroups_per_dimension)
        {
            return Err(Error::UnsupportedSize);
        }

        let histograms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("coarse temporal motion histograms"),
            size: histogram_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.clear_buffer(&histograms, 0, None);
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("coarse temporal doubled globals"),
            size: global_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let seeds = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("coarse temporal interpolated seeds"),
            size: seed_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let parameter_bytes: Vec<_> = [
            coarse_blocks[0],
            coarse_blocks[1],
            next_blocks[0],
            next_blocks[1],
            u32::try_from(coarse_count).map_err(|_| Error::UnsupportedSize)?,
            u32::try_from(next_count).map_err(|_| Error::UnsupportedSize)?,
            coarse.references,
            0,
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
        let parameters = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("coarse temporal predictor geometry"),
            contents: &parameter_bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("coarse temporal predictor preparation"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: coarse.raw.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: histograms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: seeds.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: parameters.as_entire_binding(),
                },
            ],
        });

        dispatch(
            encoder,
            &self.histogram,
            &group,
            [
                u32::try_from(coarse_count.div_ceil(u64::from(LANES)))
                    .map_err(|_| Error::UnsupportedSize)?,
                coarse.references,
                1,
            ],
            "coarse temporal histograms",
        );
        dispatch(
            encoder,
            &self.globals,
            &group,
            [coarse.references, 1, 1],
            "coarse temporal global estimates",
        );
        dispatch(
            encoder,
            &self.interpolate,
            &group,
            [
                u32::try_from(next_count.div_ceil(u64::from(LANES)))
                    .map_err(|_| Error::UnsupportedSize)?,
                coarse.references,
                1,
            ],
            "coarse temporal seed interpolation",
        );

        Ok(Prepared {
            device: device.clone(),
            blocks: next_blocks,
            references: coarse.references,
            seeds,
            globals,
        })
    }
}

fn checked_area(size: [u32; 2]) -> Result<u64, Error> {
    u64::from(size[0])
        .checked_mul(u64::from(size[1]))
        .ok_or(Error::UnsupportedSize)
}

fn dispatch(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::ComputePipeline,
    group: &wgpu::BindGroup,
    workgroups: [u32; 3],
    label: &'static str,
) {
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some(label),
        timestamp_writes: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.dispatch_workgroups(workgroups[0], workgroups[1], workgroups[2]);
}

#[cfg(test)]
mod tests;
