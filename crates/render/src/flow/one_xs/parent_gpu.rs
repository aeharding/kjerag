//! GPU-resident selected ONE X2 parent-map production.
//!
//! Host code retains the frozen pose/calibration preparation in `parent`; this
//! stage moves the complete per-node `CalcMap` arithmetic to the capture's one
//! render device. The output stays opaque and resident. Qualification-only
//! code may copy it back to compare every bit with the scalar oracle.

use std::time::Duration;

use kjerag_meta::{OrientationTrack, Readout};

use super::base_map::{SELECTED_FLOWSTATE_COLS, SELECTED_FLOWSTATE_ROWS};
use super::gpu_context::OneXsGpuContext;
use super::metal_calc_map::MetalCalcMapParams;
use super::parent::{ParentMapBuilder, ParentMapError, PreparedParentMap};
use crate::Fallible;

const PARAM_WORDS: usize = 33;
const POSE_WORDS: usize = 51 * 4;
const INPUT_WORDS: usize = 2 * PARAM_WORDS + POSE_WORDS;
pub(super) const LENS_OUTPUT_WORDS: usize = SELECTED_FLOWSTATE_ROWS * SELECTED_FLOWSTATE_COLS * 2;
const OUTPUT_WORDS: usize = 2 * LENS_OUTPUT_WORDS;

/// The resident pair produced by one exact parent-map dispatch.
///
/// Its storage and device identity are deliberately private. The GPU geometry
/// stage will consume this token directly once the shared submission lease is
/// available on the integration branch.
pub(crate) struct ResidentParentMaps {
    /// Visible only to the enclosing ONE X2 resident chain.
    pub(super) context: OneXsGpuContext,
    /// A-then-B row-major float2 bits, never exposed outside the chain.
    pub(super) storage: wgpu::Buffer,
}

/// Reusable pipeline for the selected parent arithmetic on one GPU context.
pub(crate) struct GpuParentMapPipeline {
    context: OneXsGpuContext,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl GpuParentMapPipeline {
    pub(crate) fn new(context: OneXsGpuContext) -> Fallible<Self> {
        let device = context.device();
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
            label: Some("ONE X2 GPU parent maps"),
            entries: &[storage(0, true), storage(1, false)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 GPU parent maps"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let shader = shader_source();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 GPU parent maps"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ONE X2 GPU parent maps"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("calc_parent_maps"),
            compilation_options: Default::default(),
            cache: None,
        });
        Ok(Self {
            context,
            pipeline,
            layout,
        })
    }

    /// Submit one pair without waiting, polling, mapping, or reading it back.
    pub(crate) fn produce(
        &self,
        builder: &ParentMapBuilder,
        orientation: &OrientationTrack,
        center: Duration,
        readout: Readout,
    ) -> Result<ResidentParentMaps, ParentMapError> {
        let prepared = builder.prepare(orientation, center, readout)?;
        Ok(self.dispatch(&prepared))
    }

    fn dispatch(&self, prepared: &PreparedParentMap) -> ResidentParentMaps {
        let words = pack(prepared);
        debug_assert_eq!(words.len(), INPUT_WORDS);
        let device = self.context.device();
        let input = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 GPU parent inputs"),
            size: (INPUT_WORDS * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.context
            .queue()
            .write_buffer(&input, 0, &words_to_bytes(&words));
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 resident parent maps"),
            size: (OUTPUT_WORDS * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 GPU parent resources"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 GPU parent maps"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 GPU parent maps"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(25, 13, 2);
        }
        self.context.queue().submit([encoder.finish()]);
        ResidentParentMaps {
            context: self.context.clone(),
            storage: output,
        }
    }
}

fn pack(prepared: &PreparedParentMap) -> Vec<u32> {
    let mut words = Vec::with_capacity(INPUT_WORDS);
    pack_parameters(&mut words, &prepared.parameters.a);
    pack_parameters(&mut words, &prepared.parameters.b);
    for pose in prepared.poses {
        words.extend(pose.map(f32::to_bits));
    }
    words
}

fn pack_parameters(words: &mut Vec<u32>, value: &MetalCalcMapParams) {
    words.extend(value.center.map(f32::to_bits));
    words.extend(value.focal.map(f32::to_bits));
    words.extend(value.src_size.map(f32::to_bits));
    words.extend(value.inv_src_size.map(f32::to_bits));
    words.extend(value.qci.map(f32::to_bits));
    words.extend(value.qwm.map(f32::to_bits));
    words.extend(value.q_c0_f0.map(f32::to_bits));
    words.extend(value.shift.map(f32::to_bits));
    words.push(value.xi.to_bits());
    words.push(u32::from(value.is_horizon_sweep));
    words.push(value.flip.to_bits());
    words.push(value.max_fov.to_bits());
    words.extend(value.distort_coeffs.map(f32::to_bits));
    words.extend(value.pos_scale.map(f32::to_bits));
}

fn words_to_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_ne_bytes()).collect()
}

const SHADER: &str = include_str!("parent_map.wgsl");

fn shader_source() -> String {
    let mut dst_x = Vec::with_capacity(SELECTED_FLOWSTATE_COLS);
    let mut phi_cos = Vec::with_capacity(SELECTED_FLOWSTATE_COLS);
    let mut phi_sin = Vec::with_capacity(SELECTED_FLOWSTATE_COLS);
    for column in 0..SELECTED_FLOWSTATE_COLS {
        let position =
            ((column as f32 * 2.0) * std::f32::consts::PI) / SELECTED_FLOWSTATE_COLS as f32;
        let phi = 2.0 * std::f32::consts::PI - position;
        dst_x.push(position.to_bits());
        phi_cos.push(phi.cos().to_bits());
        phi_sin.push(phi.sin().to_bits());
    }
    let mut dst_y = Vec::with_capacity(SELECTED_FLOWSTATE_ROWS);
    let mut theta_cos = Vec::with_capacity(SELECTED_FLOWSTATE_ROWS);
    let mut theta_sin = Vec::with_capacity(SELECTED_FLOWSTATE_ROWS);
    for row in 0..SELECTED_FLOWSTATE_ROWS {
        let position = (row as f32 * std::f32::consts::PI) / (SELECTED_FLOWSTATE_ROWS - 1) as f32;
        let theta = 0.5 * std::f32::consts::PI - position;
        dst_y.push(position.to_bits());
        theta_cos.push(theta.cos().to_bits());
        theta_sin.push(theta.sin().to_bits());
    }
    let mut tables = String::new();
    push_bits(&mut tables, "DST_X", &dst_x);
    push_bits(&mut tables, "PHI_COS", &phi_cos);
    push_bits(&mut tables, "PHI_SIN", &phi_sin);
    push_bits(&mut tables, "DST_Y", &dst_y);
    push_bits(&mut tables, "THETA_COS", &theta_cos);
    push_bits(&mut tables, "THETA_SIN", &theta_sin);
    tables.push_str(&format!(
        "const MAX_FOV_COS: u32 = 0x{:08x}u;\n",
        f32::from_bits(0x4006_0a92).cos().to_bits()
    ));
    SHADER.replace("// GENERATED_TABLES", &tables)
}

fn push_bits(output: &mut String, name: &str, values: &[u32]) {
    use std::fmt::Write as _;

    write!(
        output,
        "const {name}: array<u32, {}> = array<u32, {}>(",
        values.len(),
        values.len()
    )
    .unwrap();
    for value in values {
        write!(output, "0x{value:08x}u,").unwrap();
    }
    output.push_str(");\n");
}

#[cfg(test)]
#[path = "parent_gpu_tests.rs"]
mod tests;
