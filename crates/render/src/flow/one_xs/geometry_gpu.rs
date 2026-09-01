//! GPU-resident selected ONE X2 retained geometry.
//!
//! The CPU remains responsible for the small pose/orientation calculation.
//! This stage consumes the resident two-lens 100-by-200 parent token, then
//! performs both 1,080-by-60 periodic `mapMerge`
//! kernels, the selected directional continuity filters and the physical
//! mask seed, 9-by-9 erosion and A/B unification. Static coordinates are
//! uploaded once at construction. The result has no ordinary readback.

use super::super::base_map::{
    FlowstateRoi, SELECTED_FLOWSTATE_COLS, SELECTED_FLOWSTATE_ROWS, SELECTED_LINE_COLS,
    SELECTED_LINE_ROWS, StaticLineCoordinates, filter_fisheye_line_pair, map_merge,
};
use super::super::gpu_context::OneXsGpuContext;
use super::super::pis::gpu::GpuPisFlight;
use super::super::{COLS, LensPair, ROWS};
use super::parent_gpu::{EncodedGpuParentMaps, ResidentGpuParentMaps};
use super::pis_frontend_gpu::{GpuPisFrontEnd, GpuPreparedFrame};
use super::resident_frame_gpu::GpuResidentReservation;
use super::{GpuBlurredBelts, GpuSolverBeltPipeline, SourceTextures};
use crate::Fallible;
use crate::flow::one_xs_belt::{RetainedBaseMaps, base_support_masks};

/// Temporal image state is private beneath the geometry owner. Its production
/// entry can consume only the complete geometry/belt aggregate.
#[path = "temporal_gpu.rs"]
#[allow(dead_code)]
pub(in crate::flow::one_xs::one_xs_belt_gpu) mod temporal_gpu;

const PARENT_NODES_PER_LENS: usize = SELECTED_FLOWSTATE_ROWS * SELECTED_FLOWSTATE_COLS;
const RETAINED_NODES_PER_LENS: usize = SELECTED_LINE_ROWS * SELECTED_LINE_COLS;
const PARENT_BYTES: u64 = (2 * PARENT_NODES_PER_LENS * size_of::<[f32; 2]>()) as u64;
const RETAINED_BYTES: u64 = (2 * RETAINED_NODES_PER_LENS * size_of::<[f32; 2]>()) as u64;
pub(in crate::flow) const MASK_WORDS_PER_LENS: usize = RETAINED_NODES_PER_LENS.div_ceil(4);
const MASK_WORDS: usize = 2 * MASK_WORDS_PER_LENS + 1;
const MASK_BYTES: u64 = (MASK_WORDS * size_of::<u32>()) as u64;
const WORKGROUP: u32 = 64;
const _: () = assert!(COLS == SELECTED_LINE_COLS && ROWS == SELECTED_LINE_ROWS);

/// A complete frame-bound geometry allocation admitted by the one source
/// submission lease. It remains owned through belt sampling, PIS and final
/// materialization.
#[must_use = "the GPU-resident ONE X2 geometry has not been consumed"]
pub(crate) struct GpuRetainedGeometry {
    context: OneXsGpuContext,
    flight: Option<GpuPisFlight>,
    _parents: ParentRetention,
    retained: wgpu::Buffer,
    masks: wgpu::Buffer,
    _resources: wgpu::BindGroup,
    reservation: Option<GpuResidentReservation>,
}

enum ParentRetention {
    Uploaded(wgpu::Buffer),
    Resident(ResidentGpuParentMaps),
}

impl GpuRetainedGeometry {
    // Intentionally no ordinary accessors. Only the two concrete consuming
    // transitions below can bind its retained map and packed masks.
}

/// Source owner carried by the sole submission lease after geometry enters
/// belt sampling. Its fields cannot be separated from that lease.
pub(crate) struct GpuGeometryFrameOwner<K> {
    _source_owner: K,
    _geometry: GpuRetainedGeometry,
}

/// Exact geometry-to-belt product. The mask handle is private and can only
/// enter the matching prepared-source front end; the geometry allocation and
/// imported source owner remain inside the belt submission lease.
#[must_use = "the geometry-backed ONE X2 solver belts have not been consumed"]
pub(crate) struct GpuGeometryBelts<K> {
    pub(super) belts: GpuBlurredBelts<GpuGeometryFrameOwner<K>>,
    pub(super) masks: wgpu::Buffer,
    pub(super) reservation: Option<GpuResidentReservation>,
}

impl<K> GpuGeometryBelts<K> {
    /// Consume the complete geometry-backed belt token into the exact resident
    /// front end. The packed physical masks bind directly without reupload.
    pub(crate) fn prepare_front_end(
        self,
        front_end: &GpuPisFrontEnd,
    ) -> Fallible<GpuPreparedFrame<GpuGeometryFrameOwner<K>>> {
        if self.reservation.is_some() {
            return Err(
                "ONE X2 resident geometry must enter motion before the PIS front end".into(),
            );
        }
        front_end.prepare_geometry(self)
    }
}

/// Geometry commands and their inseparable frame allocation before the source
/// producer submits them under its existing [`SubmissionLease`].
#[must_use = "the encoded ONE X2 geometry has not entered the source submission"]
pub(crate) struct EncodedGpuGeometry {
    context: OneXsGpuContext,
    encoder: wgpu::CommandEncoder,
    geometry: GpuRetainedGeometry,
}

impl EncodedGpuGeometry {
    /// The only geometry-to-belt transition. Belt commands are appended to
    /// this producer encoder, then the belt owner performs the one submission
    /// and creates the chain's sole source-surface lease.
    pub(crate) fn submit_belts<K>(
        mut self,
        pipeline: &GpuSolverBeltPipeline,
        sources: SourceTextures<'_>,
        source_owner: K,
    ) -> Fallible<GpuGeometryBelts<K>> {
        let flight = self
            .geometry
            .flight
            .take()
            .expect("production GPU geometry carries one exact capture flight");
        let retained = self.geometry.retained.clone();
        let masks = self.geometry.masks.clone();
        pipeline.context.ensure_same(&self.context)?;
        sources.validate()?;
        let reservation = self.geometry.reservation.take();
        if let Some(reservation) = &reservation
            && reservation.flight() != &flight
        {
            return Err("ONE X2 resident geometry flight differs from its root reservation".into());
        }
        let owner = GpuGeometryFrameOwner {
            _source_owner: source_owner,
            _geometry: self.geometry,
        };
        let belts = pipeline
            .submit_inner_with_map(
                sources,
                super::MapInput::Resident(&retained),
                owner,
                super::SubmissionInput::Sampled {
                    qualify_intermediates: false,
                },
                false,
                Some(self.encoder),
            )?
            .into_resident(flight);
        Ok(GpuGeometryBelts {
            belts,
            masks,
            reservation,
        })
    }

    /// Qualification is the only local submission. Ordinary work leaves this
    /// token untouched for the replacement consuming lease transition.
    fn submit_for_qualification(self, context: &OneXsGpuContext) -> Fallible<GpuRetainedGeometry> {
        self.context.ensure_same(context)?;
        context.queue().submit([self.encoder.finish()]);
        Ok(self.geometry)
    }
}

/// Static-coordinate owner and exact dynamic geometry pipelines.
pub(crate) struct GpuGeometryPipeline {
    context: OneXsGpuContext,
    merge: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
    mask: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    static_coordinates: wgpu::Buffer,
}

impl GpuGeometryPipeline {
    pub(crate) fn new(
        context: OneXsGpuContext,
        coordinates: &LensPair<StaticLineCoordinates>,
    ) -> Fallible<Self> {
        Self::from_shader(context, coordinates, SHADER)
    }

    fn from_shader(
        context: OneXsGpuContext,
        coordinates: &LensPair<StaticLineCoordinates>,
        shader: &str,
    ) -> Fallible<Self> {
        validate_static(coordinates)?;
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
            label: Some("ONE X2 resident geometry"),
            entries: &[
                storage(0, true),
                storage(1, true),
                storage(2, false),
                storage(3, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 resident geometry"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 resident geometry"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ONE X2 resident geometry"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let static_coordinates = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 static line coordinates"),
            size: RETAINED_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        context
            .queue()
            .write_buffer(&static_coordinates, 0, &pair_bytes(coordinates));
        let built = Self {
            context: context.clone(),
            merge: pipeline("merge_maps"),
            filter: pipeline("filter_rows"),
            mask: pipeline("build_masks"),
            layout,
            static_coordinates,
        };
        built.qualify(coordinates)?;
        Ok(built)
    }

    /// Encode the complete producer at the temporary sealed CPU-parent upload
    /// boundary. The returned command encoder is deliberately unfinished: the
    /// source-belt producer appends its work and mints the transaction's one
    /// submission lease only after the combined command is submitted.
    ///
    /// The incoming resident-parent stage replaces only the private parent
    /// owner and binding below. None of the retained-map or mask arithmetic
    /// needs to change.
    pub(crate) fn encode_uploaded_parents(
        &self,
        parents: &LensPair<FlowstateRoi>,
        flight: GpuPisFlight,
    ) -> Fallible<EncodedGpuGeometry> {
        validate_parents(parents)?;
        self.encode_inner(parents, Some(flight))
    }

    fn encode_inner(
        &self,
        parents: &LensPair<FlowstateRoi>,
        flight: Option<GpuPisFlight>,
    ) -> Fallible<EncodedGpuGeometry> {
        let device = self.context.device();
        let parent_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 dynamic parent maps"),
            size: PARENT_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.context
            .queue()
            .write_buffer(&parent_buffer, 0, &pair_bytes(parents));
        let encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 resident geometry and source transaction"),
        });
        self.encode_buffer(
            parent_buffer.clone(),
            ParentRetention::Uploaded(parent_buffer),
            flight,
            None,
            encoder,
        )
    }

    /// Consume the only resident-parent token and append geometry to its
    /// unfinished encoder. Context refusal happens before any geometry
    /// allocation, binding, or encoding.
    pub(super) fn encode_resident_parents(
        &self,
        parents: EncodedGpuParentMaps,
    ) -> Fallible<EncodedGpuGeometry> {
        self.context.ensure_same(&parents.context)?;
        let EncodedGpuParentMaps {
            context: _,
            flight,
            encoder,
            resident,
            reservation,
        } = parents;
        self.encode_buffer(
            resident.storage.clone(),
            ParentRetention::Resident(resident),
            Some(flight),
            Some(reservation),
            encoder,
        )
    }

    fn encode_buffer(
        &self,
        parent_buffer: wgpu::Buffer,
        parent_retention: ParentRetention,
        flight: Option<GpuPisFlight>,
        reservation: Option<GpuResidentReservation>,
        mut encoder: wgpu::CommandEncoder,
    ) -> Fallible<EncodedGpuGeometry> {
        let device = self.context.device();
        let retained = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 resident retained maps"),
            size: RETAINED_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let masks = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 resident physical masks"),
            size: MASK_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 resident geometry"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.static_coordinates.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: parent_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: retained.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: masks.as_entire_binding(),
                },
            ],
        });
        for pipeline in [&self.merge, &self.filter, &self.mask] {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 resident geometry"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &resources, &[]);
            let work = if std::ptr::eq(pipeline, &self.filter) {
                (2 * ROWS) as u32
            } else if std::ptr::eq(pipeline, &self.mask) {
                MASK_WORDS as u32
            } else {
                (2 * ROWS * COLS) as u32
            };
            pass.dispatch_workgroups(work.div_ceil(WORKGROUP), 1, 1);
        }
        Ok(EncodedGpuGeometry {
            context: self.context.clone(),
            encoder,
            geometry: GpuRetainedGeometry {
                context: self.context.clone(),
                flight,
                _parents: parent_retention,
                retained,
                masks,
                _resources: resources,
                reservation,
            },
        })
    }

    fn qualify(&self, coordinates: &LensPair<StaticLineCoordinates>) -> Fallible<()> {
        let parents = qualification_parents(coordinates);
        let expected = cpu_oracle(coordinates, &parents)?;
        for (name, covered) in [
            (
                "NaN",
                expected.0.iter().any(|word| f32::from_bits(*word).is_nan()),
            ),
            (
                "infinity",
                expected
                    .0
                    .iter()
                    .any(|word| f32::from_bits(*word).is_infinite()),
            ),
            ("negative zero", expected.0.contains(&(-0.0_f32).to_bits())),
        ] {
            if !covered {
                return Err(format!(
                    "ONE X2 GPU geometry qualification fixture does not reach its {name} case"
                )
                .into());
            }
        }
        let encoded = self.encode_inner(&parents, None)?;
        let geometry = encoded.submit_for_qualification(&self.context)?;
        let (actual_maps, actual_masks) = diagnostic_readback(&self.context, &geometry)?;
        if let Some(index) = actual_maps
            .iter()
            .zip(&expected.0)
            .position(|(a, b)| a != b)
        {
            return Err(format!("ONE X2 GPU geometry is not bit exact at retained word {index}: 0x{:08x}, expected 0x{:08x}", actual_maps[index], expected.0[index]).into());
        }
        if let Some(index) = actual_masks
            .iter()
            .zip(&expected.1)
            .position(|(a, b)| a != b)
        {
            return Err(format!(
                "ONE X2 GPU geometry is not exact at physical mask {index}: {}, expected {}",
                actual_masks[index], expected.1[index]
            )
            .into());
        }
        Ok(())
    }
}

fn validate_static(coordinates: &LensPair<StaticLineCoordinates>) -> Fallible<()> {
    for (name, map) in [('A', &coordinates.a), ('B', &coordinates.b)] {
        if (map.rows(), map.cols()) != (ROWS, COLS) {
            return Err(format!(
                "ONE X2 GPU static lens {name} is {} by {}, expected {ROWS} by {COLS}",
                map.rows(),
                map.cols()
            )
            .into());
        }
        if let Some((node, axis, bits)) =
            map.row_major_values()
                .iter()
                .enumerate()
                .find_map(|(node, value)| {
                    value
                        .iter()
                        .enumerate()
                        .find(|(_, v)| !v.is_finite())
                        .map(|(axis, v)| (node, axis, v.to_bits()))
                })
        {
            return Err(format!("ONE X2 GPU static lens {name} node {node} axis {axis} is not finite (bits 0x{bits:08x})").into());
        }
    }
    Ok(())
}

fn validate_parents(parents: &LensPair<FlowstateRoi>) -> Fallible<()> {
    for (name, map) in [('A', &parents.a), ('B', &parents.b)] {
        if (map.rows(), map.cols()) != (SELECTED_FLOWSTATE_ROWS, SELECTED_FLOWSTATE_COLS) {
            return Err(format!("ONE X2 GPU parent lens {name} is {} by {}, expected {SELECTED_FLOWSTATE_ROWS} by {SELECTED_FLOWSTATE_COLS}", map.rows(), map.cols()).into());
        }
    }
    Ok(())
}

trait Float2Map {
    fn values(&self) -> &[[f32; 2]];
}
impl Float2Map for StaticLineCoordinates {
    fn values(&self) -> &[[f32; 2]] {
        self.row_major_values()
    }
}
impl Float2Map for FlowstateRoi {
    fn values(&self) -> &[[f32; 2]] {
        self.row_major_values()
    }
}

fn pair_bytes<T: Float2Map>(pair: &LensPair<T>) -> Vec<u8> {
    pair.a
        .values()
        .iter()
        .chain(pair.b.values())
        .flat_map(|v| v.iter())
        .flat_map(|v| v.to_ne_bytes())
        .collect()
}

fn qualification_parents(coordinates: &LensPair<StaticLineCoordinates>) -> LensPair<FlowstateRoi> {
    let make = |salt: u32| {
        (0..PARENT_NODES_PER_LENS)
            .map(|i| {
                let row = i / SELECTED_FLOWSTATE_COLS;
                let col = i % SELECTED_FLOWSTATE_COLS;
                let jitter = ((i as u32).wrapping_mul(0x9e37_79b9) ^ salt) & 0xff;
                [
                    0.2 + col as f32 * 0.001 + row as f32 * 0.0001 + jitter as f32 * 1.0e-7,
                    0.4 + col as f32 * 0.0002 + row as f32 * 0.0003,
                ]
            })
            .collect::<Vec<_>>()
    };
    let mut a = make(17);
    let b = make(91);
    // Column 30 is retained by lens A's directional filter. Fill all four
    // parent taps reached by three such nodes so exceptional arithmetic is
    // present in the compared output rather than merely in an uploaded input.
    for (row, value) in [
        (100, [f32::from_bits(0x7fc0_1234), f32::NAN]),
        (200, [f32::INFINITY, f32::INFINITY]),
        (300, [-0.0, -0.0]),
    ] {
        let [column, parent_row] = coordinates.a.row_major_values()[row * COLS + COLS / 2];
        let column0 = column as usize;
        let row0 = parent_row as usize;
        for (sample_row, sample_col) in [
            (row0, column0),
            (row0, (column0 + 1) % SELECTED_FLOWSTATE_COLS),
            ((row0 + 1) % SELECTED_FLOWSTATE_ROWS, column0),
            (
                (row0 + 1) % SELECTED_FLOWSTATE_ROWS,
                (column0 + 1) % SELECTED_FLOWSTATE_COLS,
            ),
        ] {
            a[sample_row * SELECTED_FLOWSTATE_COLS + sample_col] = value;
        }
    }
    LensPair {
        a: FlowstateRoi::from_row_major_values(SELECTED_FLOWSTATE_ROWS, SELECTED_FLOWSTATE_COLS, a)
            .unwrap(),
        b: FlowstateRoi::from_row_major_values(SELECTED_FLOWSTATE_ROWS, SELECTED_FLOWSTATE_COLS, b)
            .unwrap(),
    }
}

fn cpu_oracle(
    coordinates: &LensPair<StaticLineCoordinates>,
    parents: &LensPair<FlowstateRoi>,
) -> Fallible<(Vec<u32>, Vec<u32>)> {
    let maps = filter_fisheye_line_pair(LensPair {
        a: map_merge(&coordinates.a, &parents.a)?,
        b: map_merge(&coordinates.b, &parents.b)?,
    })?;
    let retained = RetainedBaseMaps::from_lenses(LensPair {
        a: maps.a.row_major_values().to_vec(),
        b: maps.b.row_major_values().to_vec(),
    })?;
    let map_words = retained
        .bytes()
        .chunks_exact(4)
        .map(|b| u32::from_ne_bytes(b.try_into().unwrap()))
        .collect();
    let masks = base_support_masks(&retained);
    let mut mask_words = vec![0u32; MASK_WORDS];
    for (lens, mask) in [&masks.a, &masks.b].into_iter().enumerate() {
        for (index, value) in mask.iter().copied().enumerate() {
            mask_words[lens * MASK_WORDS_PER_LENS + index / 4] |=
                u32::from(value) << (8 * (index % 4));
        }
    }
    Ok((map_words, mask_words))
}

fn diagnostic_readback(
    context: &OneXsGpuContext,
    geometry: &GpuRetainedGeometry,
) -> Fallible<(Vec<u32>, Vec<u32>)> {
    use std::sync::mpsc;
    let device = context.device();
    let map = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ONE X2 geometry diagnostic map readback"),
        size: RETAINED_BYTES,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mask = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ONE X2 geometry diagnostic mask readback"),
        size: MASK_BYTES,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&geometry.retained, 0, &map, 0, RETAINED_BYTES);
    encoder.copy_buffer_to_buffer(&geometry.masks, 0, &mask, 0, MASK_BYTES);
    let copy = context.queue().submit([encoder.finish()]);
    let read = |buffer: &wgpu::Buffer| -> Fallible<Vec<u32>> {
        let slice = buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device.poll(wgpu::PollType::Wait {
            submission_index: Some(copy.clone()),
            timeout: None,
        })?;
        rx.recv()??;
        let bytes = slice.get_mapped_range();
        let words = bytes
            .chunks_exact(4)
            .map(|b| u32::from_ne_bytes(b.try_into().unwrap()))
            .collect();
        drop(bytes);
        buffer.unmap();
        Ok(words)
    };
    Ok((read(&map)?, read(&mask)?))
}

const SHADER: &str = r#"
const PARENT_NODES: u32 = 20000u;
const RETAINED_NODES: u32 = 64800u;
const MASK_WORDS_PER_LENS: u32 = 16200u;
const ROWS: u32 = 1080u;
const COLS: u32 = 60u;
const PARENT_COLS: u32 = 200u;
const PARENT_ROWS: u32 = 100u;
const THRESHOLD: f32 = 0.005;
const SENTINEL: f32 = -20.0;

@group(0) @binding(0) var<storage, read> coordinates: array<vec2<f32>>;
@group(0) @binding(1) var<storage, read> parents: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read_write> retained: array<vec2<f32>>;
@group(0) @binding(3) var<storage, read_write> masks: array<u32>;

@compute @workgroup_size(64)
fn merge_maps(@builtin(global_invocation_id) gid: vec3<u32>) {
  let index = gid.x;
  if (index >= 2u * RETAINED_NODES) { return; }
  let lens = index / RETAINED_NODES;
  let local = index % RETAINED_NODES;
  let p = coordinates[index];
  let c0 = u32(p.x);
  let r0 = u32(p.y);
  let c1 = select(c0 + 1u, 0u, c0 + 1u == PARENT_COLS);
  let r1 = select(r0 + 1u, 0u, r0 + 1u == PARENT_ROWS);
  let ci = (1.0 - p.x) + f32(c0);
  let ri = (1.0 - p.y) + f32(r0);
  let w00 = ci * ri;
  let w01 = ri - w00;
  let w10 = ci - w00;
  let w11 = ((1.0 - ci) - ri) + w00;
  let base = lens * PARENT_NODES;
  let v00 = parents[base + r0 * PARENT_COLS + c0];
  let v01 = parents[base + r0 * PARENT_COLS + c1];
  let v10 = parents[base + r1 * PARENT_COLS + c0];
  let v11 = parents[base + r1 * PARENT_COLS + c1];
  var sum = v01 * w01;
  sum = fma(v00, vec2(w00), sum);
  sum = fma(v10, vec2(w10), sum);
  retained[index] = fma(v11, vec2(w11), sum);
}

@compute @workgroup_size(64)
fn filter_rows(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (gid.x >= 2u * ROWS) { return; }
  let lens = gid.x / ROWS;
  let row = gid.x % ROWS;
  let base = lens * RETAINED_NODES + row * COLS;
  let pivot = COLS / 2u;
  var tripped = false;
  if (lens == 0u) {
    for (var col = pivot + 1u; col < COLS; col++) {
      if (!tripped) { tripped = abs(retained[base + col].x - retained[base + col - 1u].x) > THRESHOLD; }
      if (tripped) { retained[base + col] = vec2(SENTINEL); }
    }
  } else {
    for (var step = 0u; step <= pivot; step++) {
      let col = pivot - step;
      if (!tripped) { tripped = abs(retained[base + col + 1u].x - retained[base + col].x) > THRESHOLD; }
      if (tripped) { retained[base + col] = vec2(SENTINEL); }
    }
  }
}

fn valid(v: vec2<f32>) -> bool { return v.x > 0.0 && v.x <= 1.0 && v.y > 0.0 && v.y <= 1.0; }

@compute @workgroup_size(64)
fn build_masks(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (gid.x == 2u * MASK_WORDS_PER_LENS) {
    masks[gid.x] = 0u;
    return;
  }
  if (gid.x > 2u * MASK_WORDS_PER_LENS) { return; }
  let lens = gid.x / MASK_WORDS_PER_LENS;
  let word = gid.x % MASK_WORDS_PER_LENS;
  var packed = 0u;
  for (var byte = 0u; byte < 4u; byte++) {
    let local = word * 4u + byte;
    let row = local / COLS;
    let col = local % COLS;
    let r0 = select(row - 4u, 0u, row < 4u);
    let r1 = min(row + 4u, ROWS - 1u);
    let c0 = select(col - 4u, 0u, col < 4u);
    let c1 = min(col + 4u, COLS - 1u);
    var both = true;
    for (var r = r0; r <= r1; r++) {
      for (var c = c0; c <= c1; c++) {
        let at = r * COLS + c;
        both = both && valid(retained[at]) && valid(retained[RETAINED_NODES + at]);
      }
    }
    packed = packed | (select(0u, 255u, both) << (8u * byte));
  }
  masks[lens * MASK_WORDS_PER_LENS + word] = packed;
}
"#;

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::flow::one_xs::base_map::one_xs_static_coordinates;
    use crate::flow::one_xs::one_xs_belt_gpu::pis_frontend_gpu::{
        GpuColdLoopControls, GpuL1Controls, GpuL2Controls, GpuL2PostPisBridge,
    };
    use crate::flow::one_xs::pis::gpu::GpuPisPipeline;
    use crate::flow::one_xs::pis::{CostMode, DisparityInterval, Level};
    use kjerag_media::FrameStamp;
    use temporal_gpu::{GpuColdPriorPublicLevelTwo, GpuMotionStage};

    #[test]
    fn retained_geometry_matches_cpu_and_rejects_semantic_mutations() {
        let (context, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(reason) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {reason}"
                );
                eprintln!("skipping ONE X2 GPU geometry test: {reason}");
                return;
            }
        };
        let coordinates = one_xs_static_coordinates();
        let geometry_pipeline = GpuGeometryPipeline::new(context.clone(), &coordinates)
            .unwrap_or_else(|error| {
                panic!("baseline ONE X2 GPU geometry failed on {adapter}: {error}")
            });
        eprintln!("baseline ONE X2 GPU geometry passed on {adapter}");

        let mutations = [
            (
                "merge FMA operand association",
                "sum = fma(v00, vec2(w00), sum);",
                "sum = fma(v11, vec2(w00), sum);",
            ),
            (
                "directional threshold",
                "const THRESHOLD: f32 = 0.005;",
                "const THRESHOLD: f32 = 0.0;",
            ),
            ("erosion radius", "row + 4u", "row + 3u"),
            (
                "lens unification",
                "both = both && valid(retained[at]) && valid(retained[RETAINED_NODES + at]);",
                "both = both && valid(retained[at]);",
            ),
        ];
        for (name, from, to) in mutations {
            let changed = SHADER.replacen(from, to, 1);
            assert_ne!(changed, SHADER, "mutation {name} did not alter the shader");
            let error = GpuGeometryPipeline::from_shader(context.clone(), &coordinates, &changed)
                .err()
                .unwrap_or_else(|| {
                    panic!("changed ONE X2 GPU geometry {name} was accepted on {adapter}")
                });
            assert!(
                error.to_string().contains("GPU geometry is not"),
                "mutation {name} returned the wrong refusal on {adapter}: {error}"
            );
            eprintln!("ONE X2 GPU geometry refused {name} mutation on {adapter}");
        }

        let texture = |label| {
            let texture = context.device().create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: 256,
                    height: 256,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            context.queue().write_texture(
                texture.as_image_copy(),
                &vec![137; 256 * 256],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(256),
                },
                texture.size(),
            );
            texture
        };
        let texture_a = texture("geometry transition A");
        let texture_b = texture("geometry transition B");
        let belt_pipeline = GpuSolverBeltPipeline::new(context.clone()).unwrap();
        let front_end = GpuPisFrontEnd::new(context.clone()).unwrap();
        let source_owner = Arc::new(());
        let encoded = geometry_pipeline
            .encode_uploaded_parents(
                &qualification_parents(&coordinates),
                GpuPisFlight {
                    generation: 1,
                    frame: FrameStamp::for_test(1, Duration::from_millis(17), None),
                },
            )
            .unwrap();
        let belts = encoded
            .submit_belts(
                &belt_pipeline,
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                Arc::clone(&source_owner),
            )
            .unwrap();
        assert_eq!(Arc::strong_count(&source_owner), 2);
        belts
            .prepare_front_end(&front_end)
            .unwrap()
            .acknowledge_terminal()
            .unwrap();
        assert_eq!(Arc::strong_count(&source_owner), 1);
        eprintln!("ONE X2 GPU geometry completed its sealed resident chain on {adapter}");
    }

    #[test]
    fn production_cold0_consumes_geometry_motion_prior_l2_and_l1_on_one_lease() {
        let (context, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(reason) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {reason}"
                );
                eprintln!("skipping ONE X2 resident Cold0 chain: {reason}");
                return;
            }
        };
        let motion = GpuMotionStage::new(context.clone()).unwrap();
        let capture = motion.new_capture();
        let reservation = capture
            .reserve(FrameStamp::for_test(71, Duration::from_millis(71), None))
            .unwrap();
        let flight = reservation.flight().clone();
        let coordinates = one_xs_static_coordinates();
        let geometry = GpuGeometryPipeline::new(context.clone(), &coordinates).unwrap();
        let encoded = geometry
            .encode_uploaded_parents(&qualification_parents(&coordinates), flight)
            .unwrap();
        let texture = |label| {
            context.device().create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: 256,
                    height: 256,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let texture_a = texture("resident Cold0 source A");
        let texture_b = texture("resident Cold0 source B");
        let mut belts = encoded
            .submit_belts(
                &GpuSolverBeltPipeline::new(context.clone()).unwrap(),
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                Arc::new(()),
            )
            .unwrap();
        belts.reservation = Some(reservation);
        let front = GpuPisFrontEnd::new(context.clone()).unwrap();
        let solver = GpuPisPipeline::new(context.clone()).unwrap();
        let bridge = GpuL2PostPisBridge::new(context.clone()).unwrap();
        let costs =
            |level: Level| vec![CostMode::Unweighted; level.patch_rows()].into_boxed_slice();
        let disparity = DisparityInterval::new([-8.0, -8.0], [8.0, 8.0]);
        let controls = GpuColdLoopControls::new(
            GpuL2Controls::resident(costs(Level::Two), disparity, costs(Level::Two), disparity),
            GpuL1Controls::resident(costs(Level::One), disparity, costs(Level::One), disparity),
        );
        let cold0 = belts
            .prepare_motion(&motion)
            .unwrap()
            .submit_resident_cold0(
                &front,
                &solver,
                &bridge,
                GpuColdPriorPublicLevelTwo::new(context),
                controls,
            )
            .unwrap_or_else(|error| panic!("resident Cold0 chain failed on {adapter}: {error}"))
            .complete(&bridge)
            .unwrap_or_else(|error| panic!("resident Cold0 post-L1 failed on {adapter}: {error}"));
        let cold0_state = cold0
            .snapshot_for_test()
            .unwrap_or_else(|error| panic!("resident Cold0 snapshot failed on {adapter}: {error}"));
        let cold1 = cold0
            .resume(&bridge, &solver)
            .unwrap_or_else(|error| panic!("resident Cold1 chain failed on {adapter}: {error}"));
        let cold1_state = cold1
            .snapshot_for_test()
            .unwrap_or_else(|error| panic!("resident Cold1 snapshot failed on {adapter}: {error}"));
        let successor = cold1
            .resume(&bridge, &solver)
            .unwrap_or_else(|error| panic!("resident Cold2 chain failed on {adapter}: {error}"));
        let cold2_state = successor
            .snapshot_for_test()
            .unwrap_or_else(|error| panic!("resident Cold2 snapshot failed on {adapter}: {error}"));
        assert_eq!(cold0_state.calculation, 1);
        assert_eq!(cold1_state.calculation, 2);
        assert_eq!(cold2_state.calculation, 3);
        assert_eq!(cold0_state.cadence, [1, 1]);
        assert_eq!(cold1_state.cadence, [2, 2]);
        assert_eq!(cold2_state.cadence, [3, 3]);
        assert_eq!(cold0_state.lack_rows.len(), 2 * 178);
        assert_eq!(cold0_state.lack_rows, cold1_state.lack_rows);
        assert_eq!(cold1_state.lack_rows, cold2_state.lack_rows);
        for state in [&cold0_state, &cold1_state, &cold2_state] {
            assert_eq!(state.small_rows, vec![0; 2 * 178]);
            assert!(!state.small_present);
            assert_eq!(state.validity, u32::MAX);
        }
        assert!(cold0_state.same_root(&cold1_state));
        assert!(cold1_state.same_root(&cold2_state));
        assert!(capture.snapshot().pending);
        drop(successor);
        assert!(!capture.snapshot().pending);
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            match future.as_mut().poll(&mut context) {
                std::task::Poll::Ready(answer) => return answer,
                std::task::Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn gpu() -> Result<(OneXsGpuContext, String), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
            .ok_or("no Vulkan adapter")?;
        let info = adapter.get_info();
        let name = format!("{} ({})", info.name, info.driver);
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("exact ONE X2 GPU geometry"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        Ok((OneXsGpuContext::new(&device, &queue), name))
    }
}
