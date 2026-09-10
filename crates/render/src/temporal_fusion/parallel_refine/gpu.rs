//! GPU host boundary for Kjerag's independently parallel finest refinement.
//!
//! This is an explicit Kjerag-specific candidate, not parity with the selected
//! serial search. Coarse seeds and global predictors are immutable inputs, and
//! every finest block writes a separate output record. The builder only
//! records work; it does not submit, wait, read back, or select frame history.

use std::fmt;
use std::sync::OnceLock;

use super::coarse::prepare::Prepared;

use crate::temporal_fusion::pyramid::gpu::PackedGray;
use wgpu::util::DeviceExt;

const MAX_REFERENCES: usize = 6;
const BLOCK: u32 = 16;
const MIN_DIMENSION: u32 = 1_024;
const MAX_DIMENSION_EXCLUSIVE: u32 = 8_192;
const MIN_DISPLACEMENT: i32 = -8_192;
const MAX_DISPLACEMENT_EXCLUSIVE: i32 = 8_192;
const MAX_SAD: i32 = 65_280;
const RECORD_BYTES: u64 = 3 * size_of::<i32>() as u64;

/// GPU-owned raw motion records. The supplied-reference slices are contiguous
/// and each record is `(dx, dy, unpenalized SAD)` as three `i32`s.
pub struct Output {
    pub(crate) raw: wgpu::Buffer,
    pub(crate) device: wgpu::Device,
    pub(crate) blocks: [u32; 2],
    pub(crate) references: u32,
}

impl Output {
    pub fn blocks(&self) -> [u32; 2] {
        self.blocks
    }

    pub fn reference_count(&self) -> u32 {
        self.references
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    ForeignDevice,
    ReferenceCount {
        references: usize,
        seeds: usize,
        globals: usize,
    },
    Geometry,
    CoarseGeometry,
    ResidentSeeds,
    SeedCount {
        ordinal: usize,
        expected: usize,
        actual: usize,
    },
    SeedValue {
        ordinal: usize,
        index: usize,
    },
    GlobalValue {
        ordinal: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ForeignDevice => formatter.write_str(
                "parallel-refine builder or packed image belongs to a different graphics device",
            ),
            Self::ReferenceCount {
                references,
                seeds,
                globals,
            } => write!(
                formatter,
                "parallel finest refinement needs 1 through 6 matching references, seeds and globals; got {references}, {seeds} and {globals}"
            ),
            Self::Geometry => formatter.write_str(
                "parallel finest refinement needs equal image dimensions from 1024 through 8191",
            ),
            Self::CoarseGeometry => formatter.write_str(
                "parallel coarse search needs equal image dimensions from 16 through 4095",
            ),
            Self::ResidentSeeds => formatter.write_str(
                "resident motion seeds differ from the search geometry or reference count",
            ),
            Self::SeedCount {
                ordinal,
                expected,
                actual,
            } => write!(
                formatter,
                "parallel-refine reference {ordinal} has {actual} seeds but needs {expected}"
            ),
            Self::SeedValue { ordinal, index } => write!(
                formatter,
                "parallel-refine reference {ordinal} seed {index} is outside the selected safe range"
            ),
            Self::GlobalValue { ordinal } => write!(
                formatter,
                "parallel-refine reference {ordinal} global predictor is outside the selected safe range"
            ),
        }
    }
}

impl std::error::Error for Error {}

pub struct Builder {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    coarse: OnceLock<[wgpu::ComputePipeline; 2]>,
}

#[derive(Clone, Copy)]
enum Mode {
    Finest,
    Coarse,
    Smallest,
}

struct Inputs<'a> {
    seeds: &'a wgpu::Buffer,
    globals: &'a wgpu::Buffer,
    mode: Mode,
}

impl Builder {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Uint,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
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
        let mut entries: Vec<_> = (0..=1).map(texture).collect();
        entries.extend([
            storage(2, true),
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
            storage(5, true),
        ]);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("parallel finest temporal refinement"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("parallel finest temporal refinement"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("parallel finest temporal refinement"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("parallel finest temporal refinement"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("refine_blocks"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            device: device.clone(),
            layout,
            pipeline,
            coarse: OnceLock::new(),
        }
    }

    /// Record one independent finest-block refinement for one through six references.
    /// Images come from `pyramid::gpu::Builder::encode_packed_base`; logical
    /// dimensions, not packed texture width, determine search bounds. As with
    /// the producer, all resources must belong to the same wgpu Instance.
    pub fn encode_finest(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        current: &PackedGray,
        references: &[&PackedGray],
        seeds: &[&[[i32; 3]]],
        globals: &[[i32; 2]],
    ) -> Result<Output, Error> {
        let reference_count =
            validate_reference_count(references.len(), seeds.len(), globals.len())?;
        let blocks = self.validate_images(device, current, references, Mode::Finest)?;
        let records_per_reference = usize::try_from(u64::from(blocks[0]) * u64::from(blocks[1]))
            .map_err(|_| Error::Geometry)?;
        for ordinal in 0..reference_count {
            if seeds[ordinal].len() != records_per_reference {
                return Err(Error::SeedCount {
                    ordinal,
                    expected: records_per_reference,
                    actual: seeds[ordinal].len(),
                });
            }
            for (index, &[dx, dy, sad]) in seeds[ordinal].iter().enumerate() {
                if !safe_displacement(dx) || !safe_displacement(dy) || !(0..=MAX_SAD).contains(&sad)
                {
                    return Err(Error::SeedValue { ordinal, index });
                }
            }
            if globals[ordinal]
                .iter()
                .any(|&value| !safe_displacement(value))
            {
                return Err(Error::GlobalValue { ordinal });
            }
        }

        let seed_bytes: Vec<u8> = seeds
            .iter()
            .flat_map(|records| {
                records
                    .iter()
                    .flat_map(|record| record.iter().flat_map(|value| value.to_le_bytes()))
            })
            .collect();
        let seed_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("parallel finest temporal seeds"),
            contents: &seed_bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });
        let global_bytes: Vec<u8> = globals
            .iter()
            .flat_map(|global| global.iter().flat_map(|value| value.to_le_bytes()))
            .collect();
        let global_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("parallel finest temporal global predictors"),
            contents: &global_bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });

        Ok(self.encode_bound(
            device,
            encoder,
            current,
            references,
            Inputs {
                seeds: &seed_buffer,
                globals: &global_buffer,
                mode: Mode::Finest,
            },
        ))
    }

    /// Consume seeds produced entirely on the GPU, without a readback/upload.
    pub fn encode_finest_resident(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        current: &PackedGray,
        references: &[&PackedGray],
        prepared: &Prepared,
    ) -> Result<Output, Error> {
        let blocks = self.validate_images(device, current, references, Mode::Finest)?;
        validate_prepared(device, blocks, references.len(), prepared)?;
        Ok(self.encode_bound(
            device,
            encoder,
            current,
            references,
            Inputs {
                seeds: prepared.seeds(),
                globals: prepared.globals(),
                mode: Mode::Finest,
            },
        ))
    }

    /// Record one independently parallel coarse plane. Only the smallest
    /// plane may start without a completed coarser plane's prepared inputs.
    pub fn encode_coarse_resident(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        current: &PackedGray,
        references: &[&PackedGray],
        prepared: Option<&Prepared>,
    ) -> Result<Output, Error> {
        let mode = if prepared.is_some() {
            Mode::Coarse
        } else {
            Mode::Smallest
        };
        let blocks = self.validate_images(device, current, references, mode)?;
        if let Some(prepared) = prepared {
            validate_prepared(device, blocks, references.len(), prepared)?;
            return Ok(self.encode_bound(
                device,
                encoder,
                current,
                references,
                Inputs {
                    seeds: prepared.seeds(),
                    globals: prepared.globals(),
                    mode,
                },
            ));
        }
        // WebGPU initializes these private buffers to zero before their first
        // use. This is the smallest plane's explicit zero-seed law, not a
        // fallback when a predecessor failed or went missing.
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let seeds = buffer(
            "smallest motion zero seeds",
            u64::from(blocks[0]) * u64::from(blocks[1]) * references.len() as u64 * RECORD_BYTES,
        );
        let globals = buffer("smallest motion zero globals", references.len() as u64 * 8);
        Ok(self.encode_bound(
            device,
            encoder,
            current,
            references,
            Inputs {
                seeds: &seeds,
                globals: &globals,
                mode,
            },
        ))
    }

    fn validate_images(
        &self,
        device: &wgpu::Device,
        current: &PackedGray,
        references: &[&PackedGray],
        mode: Mode,
    ) -> Result<[u32; 2], Error> {
        validate_reference_count(references.len(), references.len(), references.len())?;
        if self.device != *device
            || current.device() != device
            || references.iter().any(|image| image.device() != device)
        {
            return Err(Error::ForeignDevice);
        }
        let [width, height] = current.logical_size();
        let (range, error) = match mode {
            Mode::Finest => (MIN_DIMENSION..MAX_DIMENSION_EXCLUSIVE, Error::Geometry),
            Mode::Coarse => (BLOCK..MAX_DIMENSION_EXCLUSIVE / 2, Error::CoarseGeometry),
            Mode::Smallest => (BLOCK..MAX_DIMENSION_EXCLUSIVE / 64, Error::CoarseGeometry),
        };
        if !range.contains(&width)
            || !range.contains(&height)
            || references
                .iter()
                .any(|image| image.logical_size() != [width, height])
        {
            return Err(error);
        }
        Ok([width / BLOCK, height / BLOCK])
    }

    fn pipeline(&self, mode: Mode) -> &wgpu::ComputePipeline {
        let at = match mode {
            Mode::Finest => return &self.pipeline,
            Mode::Coarse => 0,
            Mode::Smallest => 1,
        };
        &self.coarse.get_or_init(|| {
            let layout = self
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("independent coarse temporal search"),
                    bind_group_layouts: &[&self.layout],
                    immediate_size: 0,
                });
            let shader = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("independent coarse temporal search"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("gpu.wgsl").into()),
                });
            [false, true].map(|smallest| {
                self.device
                    .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some("independent coarse temporal search"),
                        layout: Some(&layout),
                        module: &shader,
                        entry_point: Some("refine_blocks"),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants: &[
                                ("COARSE", 1.0),
                                ("SMALLEST", if smallest { 1.0 } else { 0.0 }),
                            ],
                            ..Default::default()
                        },
                        cache: None,
                    })
            })
        })[at]
    }

    fn encode_bound(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        current: &PackedGray,
        references: &[&PackedGray],
        inputs: Inputs<'_>,
    ) -> Output {
        let [width, height] = current.logical_size();
        let blocks = [width / BLOCK, height / BLOCK];
        let reference_count = references.len();
        let raw = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("parallel temporal motion records"),
            size: u64::from(blocks[0])
                * u64::from(blocks[1])
                * reference_count as u64
                * RECORD_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let current_view = current.texture().create_view(&Default::default());
        let reference_views: Vec<_> = references
            .iter()
            .map(|image| image.texture().create_view(&Default::default()))
            .collect();
        let parameter_buffers: Vec<_> = (0..reference_count)
            .map(|ordinal| {
                let parameter_bytes: Vec<u8> =
                    [width, height, blocks[0], blocks[1], ordinal as u32, 0, 0, 0]
                        .iter()
                        .flat_map(|value| value.to_le_bytes())
                        .collect();
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("parallel finest temporal geometry and reference"),
                    contents: &parameter_bytes,
                    usage: wgpu::BufferUsages::UNIFORM,
                })
            })
            .collect();
        let groups: Vec<_> = reference_views
            .iter()
            .zip(&parameter_buffers)
            .map(|(reference_view, parameter_buffer)| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("parallel finest temporal refinement"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&current_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(reference_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: inputs.seeds.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: raw.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: parameter_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: inputs.globals.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("parallel finest temporal refinement"),
            timestamp_writes: None,
        });
        pass.set_pipeline(self.pipeline(inputs.mode));
        for group in &groups {
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(blocks[0], blocks[1], 1);
        }

        Output {
            raw,
            device: self.device.clone(),
            blocks,
            references: reference_count as u32,
        }
    }
}

fn validate_prepared(
    device: &wgpu::Device,
    blocks: [u32; 2],
    references: usize,
    prepared: &Prepared,
) -> Result<(), Error> {
    if prepared.device() != device {
        return Err(Error::ForeignDevice);
    }
    if prepared.blocks() != blocks || prepared.reference_count() as usize != references {
        return Err(Error::ResidentSeeds);
    }
    Ok(())
}

fn validate_reference_count(
    references: usize,
    seeds: usize,
    globals: usize,
) -> Result<usize, Error> {
    if !(1..=MAX_REFERENCES).contains(&references) || seeds != references || globals != references {
        return Err(Error::ReferenceCount {
            references,
            seeds,
            globals,
        });
    }
    Ok(references)
}

fn safe_displacement(value: i32) -> bool {
    (MIN_DISPLACEMENT..MAX_DISPLACEMENT_EXCLUSIVE).contains(&value)
}

#[cfg(test)]
mod tests;
