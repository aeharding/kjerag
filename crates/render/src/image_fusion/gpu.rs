//! GPU producer for the selected photometric image-fusion byte boundary.
//!
//! This starts with two already-aligned 800 by 16 packed BGR8 bands and the
//! 212 by 4 invalid-coordinate words. Source geometry and cadence are outside
//! this module. A producer is one capture/session's retained state; construct
//! another producer to reset it.
//!
//! The outer predicate deliberately compares integer sums with
//! `abs(new-old) > 3*66*16`. This disclosed GPU policy is not binary64
//! equivalent: the CPU reference's baseline sum 9 to 3177 case admits due to
//! binary64 mean rounding, while this producer does not.

use wgpu::util::DeviceExt;

use crate::stitch_camera::StitchCamera;

const MAP_NODES: u64 = 200 * 100;
const STATE_WORDS: usize = 1_728;

/// One immutable renderer-ordinal publication of 200 by 100 ratio maps.
pub struct Output {
    pub textures: [wgpu::Texture; 2],
}

/// Retained state for one capture/session. Construct a fresh value to reset.
pub struct Producer {
    output_streams: [usize; 2],
    pipelines: Vec<wgpu::ComputePipeline>,
    layout: wgpu::BindGroupLayout,
    state: wgpu::Buffer,
    working: wgpu::Buffer,
    field: wgpu::Buffer,
    cg: wgpu::Buffer,
    ratios: wgpu::Buffer,
    blurred: wgpu::Buffer,
    fixed: wgpu::Buffer,
}

impl Producer {
    pub fn new(device: &wgpu::Device, camera: StitchCamera) -> Result<Self, String> {
        const STORAGE_BINDINGS: u32 = 11;
        const LARGEST_BINDING: u32 = (2 * MAP_NODES * 16) as u32;
        let limits = device.limits();
        if limits.max_storage_buffers_per_shader_stage < STORAGE_BINDINGS {
            return Err(format!(
                "image fusion needs {STORAGE_BINDINGS} GPU storage buffers, but this device supports {}",
                limits.max_storage_buffers_per_shader_stage
            ));
        }
        if limits.max_storage_textures_per_shader_stage < 2 {
            return Err(format!(
                "image fusion needs 2 GPU storage textures, but this device supports {}",
                limits.max_storage_textures_per_shader_stage
            ));
        }
        if limits.max_storage_buffer_binding_size < LARGEST_BINDING {
            return Err(format!(
                "image fusion needs a {LARGEST_BINDING}-byte GPU storage binding, but this device supports {} bytes",
                limits.max_storage_buffer_binding_size
            ));
        }
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                // Let wgpu derive the shader's real array minimum. In
                // particular vec4 buffers require 16-byte alignment.
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image fusion GPU producer"),
            entries: &[
                storage(0, true),
                storage(1, true),
                storage(2, true),
                storage(3, true),
                storage(4, false),
                storage(5, false),
                storage(6, false),
                storage(7, false),
                storage(8, false),
                storage(9, false),
                storage(10, true),
                storage_texture(11),
                storage_texture(12),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("image fusion GPU producer"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("image fusion GPU producer"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu.wgsl").into()),
        });
        let pipelines = [
            "admit",
            "prepare",
            "measure",
            "solve",
            "make_ratios",
            "blur",
            "retain_blur",
            "remap",
        ]
        .into_iter()
        .map(|entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        })
        .collect();
        let buffer = |label, words: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: words * 4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };

        let mut initial_state = vec![0u32; STATE_WORDS];
        initial_state[22] = 20; // retained metric
        initial_state[23] = 1; // retained budget
        initial_state[24] = 1; // first valid observation
        let state = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("image fusion retained control"),
            contents: words_as_bytes(&initial_state),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let initial_ratios = vec![[1.0f32, 1.0, 1.0, 0.0]; 2 * MAP_NODES as usize];
        let ratios = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("image fusion retained ratios"),
            contents: float4_as_bytes(&initial_ratios),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let zero_fields = vec![0.0f32; 3 * 5_088];
        let field = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("image fusion warm fields"),
            contents: floats_as_bytes(&zero_fields),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let blurred = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("image fusion blurred ratio scratch"),
            contents: float4_as_bytes(&initial_ratios),
            usage: wgpu::BufferUsages::STORAGE,
        });
        // The retained metric transition is generated from the readable CPU
        // law, not independently reimplemented in WGSL. Fixed remap
        // coordinates likewise use the recovered CPU producer once.
        let mut fixed = Vec::with_capacity(81 * 81 + 2 * MAP_NODES as usize);
        for retained in 20..=100 {
            for current in 20..=100 {
                let mut metric = crate::chromatic::Metric::seed(retained as f32);
                metric.advance(current as f32);
                fixed.push(metric.level() as u32);
            }
        }
        for coordinate in super::coordinates::for_camera_output(camera) {
            fixed.extend(coordinate.map(f32::to_bits));
        }
        let fixed = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("image fusion fixed metric and remap data"),
            contents: words_as_bytes(&fixed),
            usage: wgpu::BufferUsages::STORAGE,
        });
        Ok(Self {
            output_streams: camera.fusion_streams(),
            pipelines,
            layout,
            state,
            working: buffer("image fusion working images", 2 * 212 * 100),
            field,
            cg: buffer("image fusion CG work", 3 * 4 * 5_088),
            ratios,
            blurred,
            fixed,
        })
    }

    /// Encode one observation without submitting or reading it back.
    ///
    /// Each band is 12,800 u32 packed BGR pixels. `invalid` is 848 u32.
    /// `global_validity[0] == u32::MAX` is the resident final-map success
    /// sentinel. Every other value makes the observation a no-commit copy.
    /// Buffer shape and storage usage are checked before any output or bind
    /// group is created. The surrounding resident session guarantees that all
    /// buffers and this producer belong to the same device.
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        bands: [&wgpu::Buffer; 2],
        invalid: &wgpu::Buffer,
        global_validity: &wgpu::Buffer,
    ) -> Result<Output, String> {
        self.encode_inner(device, encoder, bands, invalid, global_validity, None)
    }

    #[cfg(test)]
    fn encode_profiled(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        bands: [&wgpu::Buffer; 2],
        invalid: &wgpu::Buffer,
        global_validity: &wgpu::Buffer,
        timestamps: &wgpu::QuerySet,
    ) -> Result<Output, String> {
        self.encode_inner(
            device,
            encoder,
            bands,
            invalid,
            global_validity,
            Some(timestamps),
        )
    }

    fn encode_inner(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        bands: [&wgpu::Buffer; 2],
        invalid: &wgpu::Buffer,
        global_validity: &wgpu::Buffer,
        timestamps: Option<&wgpu::QuerySet>,
    ) -> Result<Output, String> {
        for (name, buffer, size) in [
            ("left band", bands[0], 800 * 16 * 4),
            ("right band", bands[1], 800 * 16 * 4),
            ("coordinate validity", invalid, 4 * 212 * 4),
            ("global validity", global_validity, 4),
        ] {
            if buffer.size() != size {
                return Err(format!(
                    "image fusion {name} buffer has {} bytes, expected {size}",
                    buffer.size()
                ));
            }
            if !buffer.usage().contains(wgpu::BufferUsages::STORAGE) {
                return Err(format!(
                    "image fusion {name} buffer is not usable as GPU storage"
                ));
            }
        }
        let textures = std::array::from_fn(|lens| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(if lens == 0 {
                    "image fusion left ratio texture"
                } else {
                    "image fusion right ratio texture"
                }),
                size: wgpu::Extent3d {
                    width: 200,
                    height: 100,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        });
        let texture_views = textures
            .each_ref()
            .map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()));
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image fusion GPU observation"),
            layout: &self.layout,
            entries: &[
                entry(0, bands[0]),
                entry(1, bands[1]),
                entry(2, invalid),
                entry(3, global_validity),
                entry(4, &self.state),
                entry(5, &self.working),
                entry(6, &self.field),
                entry(7, &self.cg),
                entry(8, &self.ratios),
                entry(9, &self.blurred),
                entry(10, &self.fixed),
                // The solve stays in native fusion order. The returned
                // textures, like the stitcher's packed map, stay in delivered
                // stream order. Rebind here without another pass or copy.
                texture_entry(11, &texture_views[self.output_streams[0]]),
                texture_entry(12, &texture_views[self.output_streams[1]]),
            ],
        });
        // Skips republish retained ratios; global failures publish neutral
        // output without changing history. Both still reach every dispatch.
        let groups = [1, 27, 1, 3, 625, 625, 625, 313];
        for (stage, (pipeline, groups)) in self.pipelines.iter().zip(groups).enumerate() {
            let timestamp_writes = timestamps.map(|query_set| wgpu::ComputePassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(2 * stage as u32),
                end_of_pass_write_index: Some(2 * stage as u32 + 1),
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("image fusion GPU producer"),
                timestamp_writes,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(groups, 1, 1);
        }
        Ok(Output { textures })
    }
}

fn storage_texture(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba32Float,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn entry<'a>(binding: u32, buffer: &'a wgpu::Buffer) -> wgpu::BindGroupEntry<'a> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn texture_entry<'a>(binding: u32, view: &'a wgpu::TextureView) -> wgpu::BindGroupEntry<'a> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

fn words_as_bytes(words: &[u32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(words.as_ptr().cast(), std::mem::size_of_val(words)) }
}

fn float4_as_bytes(words: &[[f32; 4]]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(words.as_ptr().cast(), std::mem::size_of_val(words)) }
}

fn floats_as_bytes(words: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(words.as_ptr().cast(), std::mem::size_of_val(words)) }
}

#[cfg(test)]
mod tests;
