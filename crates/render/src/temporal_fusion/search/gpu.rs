//! Host-side encoder for the selected finest-level GPU motion search.
//!
//! The supplied seeds and global predictors are explicit products of the
//! coarser CPU search. This builder records the six independent references in
//! source order, one row dispatch at a time. It does not submit, wait, read
//! back, select history, or change the selected search policy.

use std::fmt;

use wgpu::util::DeviceExt;

const REFERENCES: usize = 6;
const BLOCK: u32 = 16;
const MIN_DIMENSION: u32 = 1_024;
const MAX_DIMENSION_EXCLUSIVE: u32 = 8_192;
const MIN_DISPLACEMENT: i32 = -8_192;
const MAX_DISPLACEMENT_EXCLUSIVE: i32 = 8_192;
const MAX_SAD: i32 = 65_280;

/// GPU-owned raw motion records, six contiguous reference slices in supplied
/// order. Each record is three little-endian `i32` values: dx, dy and SAD.
pub struct Output {
    pub raw: wgpu::Buffer,
    pub blocks: [u32; 2],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    ForeignDevice,
    Texture {
        ordinal: usize,
        message: &'static str,
    },
    Geometry,
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
            Self::ForeignDevice => {
                formatter.write_str("motion-search builder belongs to a different graphics device")
            }
            Self::Texture { ordinal, message } => {
                write!(formatter, "motion-search texture {ordinal} {message}")
            }
            Self::Geometry => formatter.write_str(
                "finest motion search needs equal image dimensions from 1024 through 8191",
            ),
            Self::SeedCount {
                ordinal,
                expected,
                actual,
            } => write!(
                formatter,
                "motion-search reference {ordinal} has {actual} seeds but needs {expected}"
            ),
            Self::SeedValue { ordinal, index } => write!(
                formatter,
                "motion-search reference {ordinal} seed {index} is outside the selected safe range"
            ),
            Self::GlobalValue { ordinal } => write!(
                formatter,
                "motion-search reference {ordinal} global predictor is outside the selected safe range"
            ),
        }
    }
}

impl std::error::Error for Error {}

pub struct Builder {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
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
        let mut entries: Vec<_> = (0..=6).map(texture).collect();
        entries.extend([
            storage(7, false),
            storage(8, false),
            wgpu::BindGroupLayoutEntry {
                binding: 9,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            storage(10, true),
        ]);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("finest temporal motion search"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("finest temporal motion search"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("finest temporal motion search"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("finest temporal motion search"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("search_row"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            device: device.clone(),
            layout,
            pipeline,
        }
    }

    /// Record the selected finest-level search for six references.
    pub fn encode_finest(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        current: &wgpu::Texture,
        references: [&wgpu::Texture; REFERENCES],
        seeds: [&[[i32; 3]]; REFERENCES],
        globals: [[i32; 2]; REFERENCES],
    ) -> Result<Output, Error> {
        if self.device != *device {
            return Err(Error::ForeignDevice);
        }
        validate_texture(current, 0)?;
        let size = current.size();
        if !(MIN_DIMENSION..MAX_DIMENSION_EXCLUSIVE).contains(&size.width)
            || !(MIN_DIMENSION..MAX_DIMENSION_EXCLUSIVE).contains(&size.height)
        {
            return Err(Error::Geometry);
        }
        for (index, reference) in references.iter().enumerate() {
            validate_texture(reference, index + 1)?;
            if reference.size() != size {
                return Err(Error::Geometry);
            }
        }

        let blocks = [size.width / BLOCK, size.height / BLOCK];
        let record_count = usize::try_from(u64::from(blocks[0]) * u64::from(blocks[1]))
            .map_err(|_| Error::Geometry)?;
        for ordinal in 0..REFERENCES {
            if seeds[ordinal].len() != record_count {
                return Err(Error::SeedCount {
                    ordinal,
                    expected: record_count,
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

        let raw_bytes: Vec<u8> = seeds
            .iter()
            .flat_map(|records| {
                records
                    .iter()
                    .flat_map(|record| record.iter().flat_map(|value| value.to_le_bytes()))
            })
            .collect();
        let raw = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("finest temporal motion records"),
            contents: &raw_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let bad_counts = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("finest temporal motion bad counts"),
            contents: &[0; REFERENCES * size_of::<u32>()],
            usage: wgpu::BufferUsages::STORAGE,
        });
        let global_bytes: Vec<u8> = globals
            .iter()
            .flat_map(|global| global.iter().flat_map(|value| value.to_le_bytes()))
            .collect();
        let global_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("finest temporal motion global predictors"),
            contents: &global_bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });
        let current_view = current.create_view(&Default::default());
        let reference_views = references.map(|texture| texture.create_view(&Default::default()));

        for row in 0..blocks[1] {
            let parameters = [size.width, size.height, blocks[0], blocks[1], row, 0, 0, 0];
            let parameter_bytes: Vec<u8> = parameters
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect();
            let parameter_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("finest temporal motion row"),
                contents: &parameter_bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let mut entries = vec![wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&current_view),
            }];
            entries.extend(reference_views.iter().enumerate().map(|(index, view)| {
                wgpu::BindGroupEntry {
                    binding: index as u32 + 1,
                    resource: wgpu::BindingResource::TextureView(view),
                }
            }));
            entries.extend([
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: raw.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: bad_counts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: parameter_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: global_buffer.as_entire_binding(),
                },
            ]);
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("finest temporal motion row"),
                layout: &self.layout,
                entries: &entries,
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("finest temporal motion row"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(REFERENCES as u32, 1, 1);
        }

        Ok(Output { raw, blocks })
    }
}

fn validate_texture(texture: &wgpu::Texture, ordinal: usize) -> Result<(), Error> {
    if texture.format() != wgpu::TextureFormat::R8Uint {
        return Err(Error::Texture {
            ordinal,
            message: "must use R8Uint",
        });
    }
    let size = texture.size();
    if texture.dimension() != wgpu::TextureDimension::D2
        || size.depth_or_array_layers != 1
        || texture.mip_level_count() != 1
        || texture.sample_count() != 1
    {
        return Err(Error::Texture {
            ordinal,
            message: "must be a single-sampled two-dimensional texture with one layer and mip",
        });
    }
    if !texture
        .usage()
        .contains(wgpu::TextureUsages::TEXTURE_BINDING)
    {
        return Err(Error::Texture {
            ordinal,
            message: "must have sampled-texture usage",
        });
    }
    Ok(())
}

fn safe_displacement(value: i32) -> bool {
    (MIN_DISPLACEMENT..MAX_DISPLACEMENT_EXCLUSIVE).contains(&value)
}

#[cfg(test)]
mod tests;
