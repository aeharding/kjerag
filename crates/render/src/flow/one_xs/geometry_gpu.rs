//! GPU-resident selected ONE X2 retained geometry.
//!
//! The CPU remains responsible for the small pose/orientation calculation.
//! This stage consumes the resident two-lens 100-by-200 parent token, then
//! performs both 1,080-by-60 periodic `mapMerge`
//! kernels, the selected directional continuity filters and the physical
//! mask seed, 9-by-9 erosion, camera support and A/B unification. Static
//! coordinates and conditioned camera support are uploaded once at
//! construction. The result has no ordinary readback.

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
use crate::flow::one_xs_belt::{CAMERA_MASK_SIZE, CameraMaskSupport, RetainedBaseMaps};

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
    parent: wgpu::Buffer,
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

impl GpuGeometryFrameOwner<crate::direct_type2::ImportedOneXsPicture> {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn ensure_final_map_sources(
        &self,
        frame: &kjerag_media::FrameStamp,
    ) -> Fallible<()> {
        self._source_owner.ensure_resident_frame(frame)?;
        if !matches!(self._geometry._parents, ParentRetention::Resident(_)) {
            return Err("ONE X2 final map requires the exact resident parent allocation".into());
        }
        for (name, actual, expected) in [
            ("parent", self._geometry.parent.size(), PARENT_BYTES),
            ("base", self._geometry.retained.size(), RETAINED_BYTES),
        ] {
            if actual != expected {
                return Err(format!(
                    "ONE X2 final-map {name} buffer is {actual} bytes, expected {expected}"
                )
                .into());
            }
        }
        Ok(())
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn copy_final_map_dynamic_inputs(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: super::map_patch_gpu::resident::DynamicInputTarget<'_>,
        public: &wgpu::Buffer,
    ) {
        use super::map_patch_gpu::resident::{DynamicBufferCopy, DynamicSide};

        const PREIMAGE_WORDS: u64 = 40_000;
        const SIDE_FLOW_WORDS: u64 = 129_600;
        const WORD_BYTES: u64 = size_of::<u32>() as u64;
        target.copy_side(
            encoder,
            DynamicSide::LensA,
            DynamicBufferCopy::new(&self._geometry.parent, 0),
            DynamicBufferCopy::new(&self._geometry.retained, 0),
            DynamicBufferCopy::new(public, SIDE_FLOW_WORDS * WORD_BYTES),
        );
        target.copy_side(
            encoder,
            DynamicSide::LensB,
            DynamicBufferCopy::new(&self._geometry.parent, PREIMAGE_WORDS * WORD_BYTES),
            DynamicBufferCopy::new(&self._geometry.retained, SIDE_FLOW_WORDS * WORD_BYTES),
            DynamicBufferCopy::new(public, 0),
        );
    }
}

/// Source owner carried by the sole submission lease after geometry enters
/// belt sampling. Its fields cannot be separated from that lease.
pub(crate) struct GpuGeometryFrameOwner<K> {
    _source_owner: K,
    _geometry: GpuRetainedGeometry,
}

impl GpuGeometryFrameOwner<crate::direct_type2::ImportedOneXsPicture> {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn into_installed_parts(
        self,
    ) -> (
        crate::direct_type2::ImportedOneXsPicture,
        GpuGeometryFrameOwner<()>,
    ) {
        (
            self._source_owner,
            GpuGeometryFrameOwner {
                _source_owner: (),
                _geometry: self._geometry,
            },
        )
    }
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
    #[cfg(test)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn physical_masks_for_test(&self) -> wgpu::Buffer {
        self.masks.clone()
    }

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
    camera_support: wgpu::Buffer,
}

impl GpuGeometryPipeline {
    pub(crate) fn new(
        context: OneXsGpuContext,
        coordinates: &LensPair<StaticLineCoordinates>,
        support: &CameraMaskSupport,
    ) -> Fallible<Self> {
        Self::from_shader(context, coordinates, support, SHADER)
    }

    fn from_shader(
        context: OneXsGpuContext,
        coordinates: &LensPair<StaticLineCoordinates>,
        support: &CameraMaskSupport,
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
                storage(4, true),
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
        use wgpu::util::DeviceExt;
        let mut support_bytes = super::super::Lens::ALL
            .into_iter()
            .flat_map(|lens| support.conditioned_pixels(lens))
            .flat_map(|value| value.to_ne_bytes())
            .collect::<Vec<_>>();
        // A runtime zero prevents contraction across the scalar sampler's
        // separately rounded operations, as in the qualified PIS frontend.
        support_bytes.extend(0u32.to_ne_bytes());
        support_bytes.extend(u32::from(support.applies_all_rows()).to_ne_bytes());
        assert_eq!(
            support_bytes.len(),
            (2 * CAMERA_MASK_SIZE * CAMERA_MASK_SIZE + 2) * 4
        );
        let camera_support = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ONE X2 capture-static conditioned camera support"),
            contents: &support_bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });
        let built = Self {
            context: context.clone(),
            merge: pipeline("merge_maps"),
            filter: pipeline("filter_rows"),
            mask: pipeline("build_masks"),
            layout,
            static_coordinates,
            camera_support,
        };
        built.qualify(coordinates, support)?;
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
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.camera_support.as_entire_binding(),
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
                (MASK_WORDS_PER_LENS + 1) as u32
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
                parent: parent_buffer,
                retained,
                masks,
                _resources: resources,
                reservation,
            },
        })
    }

    fn qualify(
        &self,
        coordinates: &LensPair<StaticLineCoordinates>,
        support: &CameraMaskSupport,
    ) -> Fallible<()> {
        let parents = qualification_parents(coordinates);
        let expected = cpu_oracle(coordinates, &parents, support)?;
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
        self.compare_qualification(&parents, &expected)?;
        // The original merge fixture lies largely inside camera support.
        // Valid corner UVs make the camera stage clear the outer rows while
        // preserving the middle, independently of validity and erosion.
        let corner_parents = qualification_corner_parents();
        let expected = cpu_oracle(coordinates, &corner_parents, support)?;
        self.compare_qualification(&corner_parents, &expected)
    }

    fn compare_qualification(
        &self,
        parents: &LensPair<FlowstateRoi>,
        expected: &(Vec<u32>, Vec<u32>),
    ) -> Fallible<()> {
        let encoded = self.encode_inner(parents, None)?;
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

fn qualification_corner_parents() -> LensPair<FlowstateRoi> {
    let make = || {
        FlowstateRoi::from_row_major_values(
            SELECTED_FLOWSTATE_ROWS,
            SELECTED_FLOWSTATE_COLS,
            vec![[0.001, 0.001]; PARENT_NODES_PER_LENS],
        )
        .unwrap()
    };
    LensPair {
        a: make(),
        b: make(),
    }
}

fn cpu_oracle(
    coordinates: &LensPair<StaticLineCoordinates>,
    parents: &LensPair<FlowstateRoi>,
    support: &CameraMaskSupport,
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
    let (masks, _) = support.apply(&retained);
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
@group(0) @binding(4) var<storage, read> camera_support: array<u32>;

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

fn support_rn(value: f32) -> f32 {
  return bitcast<f32>(bitcast<u32>(value) ^ camera_support[320000u]);
}

fn support_lerp(a: f32, b: f32, t: f32) -> f32 {
  let delta = support_rn(fma(b, 1.0, -a));
  let product = support_rn(fma(delta, t, -0.0));
  return support_rn(fma(a, 1.0, product));
}

fn support_at(lens: u32, row: u32, col: u32) -> f32 {
  return bitcast<f32>(camera_support[lens * 160000u + row * 400u + col]);
}

fn camera_clears(lens: u32, uv: vec2<f32>) -> bool {
  let col = clamp(support_rn(uv.x * 400.0), 0.0, 399.0);
  let row = clamp(support_rn(uv.y * 400.0), 0.0, 399.0);
  let c0 = u32(col);
  let r0 = u32(row);
  let c1 = min(c0 + 1u, 399u);
  let r1 = min(r0 + 1u, 399u);
  let dx = support_rn(col - f32(c0));
  let dy = support_rn(row - f32(r0));
  let top = support_lerp(support_at(lens, r0, c0), support_at(lens, r0, c1), dx);
  let bottom = support_lerp(support_at(lens, r1, c0), support_at(lens, r1, c1), dx);
  let sample = support_lerp(top, bottom, dy);
  // CPU promotes the f32 sample to f64 for `< 1e-8`. The nearest
  // f32 lies below that literal, so equality with these bits clears too.
  return sample <= bitcast<f32>(0x322bcc77u);
}

@compute @workgroup_size(64)
fn build_masks(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (gid.x == MASK_WORDS_PER_LENS) {
    masks[2u * MASK_WORDS_PER_LENS] = 0u;
    return;
  }
  if (gid.x > MASK_WORDS_PER_LENS) { return; }
  let word = gid.x;
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
    if (both && (camera_support[320001u] != 0u || row < 216u || row >= 864u)) {
      both = !camera_clears(0u, retained[local]) && !camera_clears(1u, retained[RETAINED_NODES + local]);
    }
    packed = packed | (select(0u, 255u, both) << (8u * byte));
  }
  // The mask is bilateral. This invocation uniquely owns the same word in
  // both lens sections; the sentinel above has its own disjoint owner.
  masks[word] = packed;
  masks[MASK_WORDS_PER_LENS + word] = packed;
}
"#;

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::time::Duration;

    use super::*;
    use crate::flow::one_xs::base_map::one_xs_static_coordinates;
    use crate::flow::one_xs::one_xs_belt_gpu::pis_frontend_gpu::{
        GpuColdLoopControls, GpuL1Controls, GpuL2Controls, GpuL2PostPisBridge,
    };
    use crate::flow::one_xs::pis::gpu::GpuPisPipeline;
    use crate::flow::one_xs::pis::{CostMode, DisparityInterval, Level};
    use crate::flow::one_xs::scalar::propagate_work_modes;
    use kjerag_media::FrameStamp;
    use temporal_gpu::{GpuColdPriorPublicLevelTwo, GpuMotionStage};

    #[test]
    fn camera_support_threshold_and_bilateral_mask_match_cpu() {
        let (context, _, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(reason) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "GPU required: {reason}"
                );
                eprintln!("skipping camera support GPU test: {reason}");
                return;
            }
        };
        let coordinates = one_xs_static_coordinates();
        let below = f32::from_bits(0x322b_cc76);
        let nearest = f32::from_bits(0x322b_cc77);
        let above = f32::from_bits(0x322b_cc78);
        for values in [
            [below, above],
            [nearest, above],
            [above, nearest],
            [above, above],
        ] {
            let support = CameraMaskSupport::filled_for_test(values);
            GpuGeometryPipeline::new(context.clone(), &coordinates, &support).unwrap_or_else(
                |error| panic!("camera threshold {values:?} on {adapter}: {error}"),
            );
            let (_, masks) =
                cpu_oracle(&coordinates, &qualification_corner_parents(), &support).unwrap();
            let cleared = values.iter().any(|v| f64::from(*v) < 1e-8_f64);
            assert_eq!(masks[0], if cleared { 0 } else { u32::MAX });
            assert_eq!(masks[216 * COLS / 4], u32::MAX);
            assert_eq!(
                masks[..MASK_WORDS_PER_LENS],
                masks[MASK_WORDS_PER_LENS..2 * MASK_WORDS_PER_LENS]
            );
        }
        let support = CameraMaskSupport::filled_for_test([nearest, above]);
        let changed = SHADER.replace("sample <= bitcast<f32>", "sample < bitcast<f32>");
        let error = GpuGeometryPipeline::from_shader(context, &coordinates, &support, &changed)
            .err()
            .expect("rounded-f32 threshold mutation passed");
        assert!(error.to_string().contains("physical mask"));
    }

    #[test]
    fn calibrated_camera_gpu_masks_clip_middle_rows_too() {
        let (context, _, _) = match gpu() {
            Ok(gpu) => gpu,
            Err(reason) => {
                assert!(std::env::var("KJERAG_REQUIRE_GPU").is_err(), "{reason}");
                eprintln!("skipping calibrated camera GPU mask test: {reason}");
                return;
            }
        };
        let reframe = crate::Reframe::new(
            &crate::projection::tests::fixture_lenses(),
            crate::projection::tests::FRAME,
            crate::Camera::default(),
            crate::Held::default(),
            1.0,
            false,
            crate::Sampling::default(),
        );
        let support = CameraMaskSupport::for_camera(
            crate::stitch_camera::StitchCamera::CalibratedMei,
            &reframe,
        )
        .unwrap();
        let coordinates = one_xs_static_coordinates();
        GpuGeometryPipeline::new(context.clone(), &coordinates, &support).unwrap();
        let (_, masks) =
            cpu_oracle(&coordinates, &qualification_corner_parents(), &support).unwrap();
        assert_eq!(masks[540 * COLS / 4], 0);
        let changed = SHADER.replace("camera_support[320001u] != 0u", "false");
        assert_ne!(changed, SHADER);
        let error = GpuGeometryPipeline::from_shader(context, &coordinates, &support, &changed)
            .err()
            .expect("a calibrated camera admitted the native pole-only mask mutation");
        assert!(error.to_string().contains("physical mask"));
    }

    #[test]
    fn retained_geometry_matches_cpu_and_rejects_semantic_mutations() {
        let (context, _, adapter) = match gpu() {
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
        let support = CameraMaskSupport::for_test();
        let geometry_pipeline = GpuGeometryPipeline::new(context.clone(), &coordinates, &support)
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
            (
                "second bilateral lens write",
                "masks[MASK_WORDS_PER_LENS + word] = packed;",
                "masks[MASK_WORDS_PER_LENS + word] = packed ^ 255u;",
            ),
            (
                "trailing validity sentinel",
                "masks[2u * MASK_WORDS_PER_LENS] = 0u;",
                "masks[2u * MASK_WORDS_PER_LENS] = 1u;",
            ),
            (
                "camera support omission",
                "if (both && (camera_support[320001u] != 0u || row < 216u || row >= 864u))",
                "if (false)",
            ),
            ("camera top range", "row < 216u", "row < 215u"),
            ("camera bottom range", "row >= 864u", "row >= 865u"),
        ];
        for (name, from, to) in mutations {
            let changed = SHADER.replacen(from, to, 1);
            assert_ne!(changed, SHADER, "mutation {name} did not alter the shader");
            let error =
                GpuGeometryPipeline::from_shader(context.clone(), &coordinates, &support, &changed)
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
        let (context, foreign_context, adapter) = match gpu() {
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
        let session = crate::flow::one_xs_belt_gpu::ResidentSourceIdentity::for_test();
        let capture =
            crate::flow::one_xs_belt_gpu::resident_frame_gpu::GpuResidentCapture::new_bound(
                context.clone(),
                session.clone(),
            );
        let reservation = capture
            .reserve(FrameStamp::for_test(71, Duration::from_millis(71), None))
            .unwrap();
        let flight = reservation.flight().clone();
        let resources = crate::flow::one_xs::resources::OneXsResources::new(
            &crate::projection::tests::one_xs_lenses(),
        )
        .unwrap();
        let coordinates = resources.static_coordinates().clone();
        let support = CameraMaskSupport::for_test();
        let geometry = GpuGeometryPipeline::new(context.clone(), &coordinates, &support).unwrap();
        let parents = qualification_parents(&coordinates);
        let expected_base = cpu_oracle(&coordinates, &parents, &support).unwrap().0;
        let parent_bytes = pair_bytes(&parents);
        let parent_words = parent_bytes
            .chunks_exact(4)
            .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
            .collect::<Vec<_>>();
        let parent = context.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 final bridge resident parent"),
            size: PARENT_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        context.queue().write_buffer(&parent, 0, &parent_bytes);
        let encoder = context
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ONE X2 final bridge resident parent"),
            });
        let encoded = geometry
            .encode_buffer(
                parent.clone(),
                ParentRetention::Resident(ResidentGpuParentMaps::for_geometry_test(
                    &context, parent,
                )),
                Some(flight.clone()),
                Some(reservation),
                encoder,
            )
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
        let picture_layout = crate::scene::bind_group_layout(context.device());
        let imported = crate::direct_type2::ImportedOneXsPicture::resident_test_owner(
            &context,
            session.clone(),
            flight.frame.clone(),
        );
        let belts = encoded
            .submit_belts(
                &GpuSolverBeltPipeline::new(context.clone()).unwrap(),
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                imported,
            )
            .unwrap();
        let front = GpuPisFrontEnd::new(context.clone()).unwrap();
        let solver = GpuPisPipeline::new(context.clone()).unwrap();
        let bridge = GpuL2PostPisBridge::new(context.clone()).unwrap();
        let disparity = DisparityInterval::new([-8.0, -8.0], [8.0, 8.0]);
        let controls = GpuColdLoopControls::new(
            GpuL2Controls::resident(disparity, disparity),
            GpuL1Controls::resident(disparity, disparity),
        );
        let cold0 = belts
            .prepare_motion(&motion)
            .unwrap()
            .submit_resident_cold0(
                &front,
                &solver,
                &bridge,
                GpuColdPriorPublicLevelTwo::new(context.clone()),
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
        let mut successor = cold1
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
        let completion = Arc::new(AtomicU8::new(0));
        let submissions = Arc::new(AtomicU8::new(0));
        let expected_public = successor.public_words_for_test().unwrap();
        successor.observe_final_lease(Arc::clone(&completion), Arc::clone(&submissions));
        let foreign_materializer =
            crate::flow::one_xs::one_xs_belt_gpu::map_patch_gpu::GpuMapMaterializer::new(
                foreign_context,
                &resources,
            )
            .unwrap();
        let error = foreign_materializer
            .validate_completed_cold_for_test(&successor)
            .unwrap_err();
        assert!(error.to_string().contains("different device or queue"));
        assert_eq!(submissions.load(Ordering::SeqCst), 0);
        assert_eq!(completion.load(Ordering::SeqCst), 0);
        assert!(capture.snapshot().pending);
        let materializer =
            crate::flow::one_xs::one_xs_belt_gpu::map_patch_gpu::GpuMapMaterializer::new(
                context.clone(),
                &resources,
            )
            .unwrap();

        // Equal readable frame values do not authorize a different frame
        // allocation. Refuse the ABA before allocating or submitting final
        // work, then restore the exact flight for the joined success path.
        let forged_frame =
            FrameStamp::for_test(flight.frame.index(), flight.frame.timestamp(), None);
        let original_frame = successor.replace_frame_for_test(forged_frame);
        let error = materializer
            .validate_completed_cold_for_test(&successor)
            .unwrap_err();
        assert!(
            error.to_string().contains("different root flight"),
            "unexpected frame ABA refusal: {error}"
        );
        successor.replace_frame_for_test(original_frame);
        assert_eq!(submissions.load(Ordering::SeqCst), 0);
        assert_eq!(completion.load(Ordering::SeqCst), 0);
        assert!(capture.snapshot().pending);

        // The same flight value in a different capture root is likewise not
        // the resident root. Identity is allocation based, not generation or
        // frame-value based.
        let foreign_capture = motion.new_capture();
        let foreign_reservation = foreign_capture.reserve(flight.frame.clone()).unwrap();
        let original_root = successor
            .replace_validity_root_for_test(foreign_reservation.identity())
            .expect("completed Cold2 validity retains its resident root");
        let error = materializer
            .validate_completed_cold_for_test(&successor)
            .unwrap_err();
        assert!(error.to_string().contains("different capture root"));
        successor.replace_validity_root_for_test(original_root);
        drop(foreign_reservation);
        assert_eq!(submissions.load(Ordering::SeqCst), 0);
        assert_eq!(completion.load(Ordering::SeqCst), 0);
        assert!(capture.snapshot().pending);

        // Install an asymmetric pre-increment cadence while Cold2 still owns
        // its successor uniquely. A must consume this next frame's current
        // L1 lack rows; B must retain the planted predecessor rows. The
        // complement makes an accidental A-prior read observably different.
        let rows = Level::One.patch_rows();
        let mut installed_lack = vec![0; 2 * rows];
        for row in 0..rows {
            installed_lack[row] = u32::from(cold2_state.lack_rows[row] == 0);
            installed_lack[rows + row] = u32::from(row % 7 == 0 || (53..=60).contains(&row));
        }
        successor
            .replace_installed_work_state_for_test([0, 7], &installed_lack)
            .unwrap();

        let operands = crate::flow::one_xs_belt_gpu::pis_frontend_gpu::admit_completed_cold_final(
            successor, &context,
        )
        .unwrap();
        let mut pending = materializer.materialize_final(operands).unwrap();
        assert_eq!(submissions.load(Ordering::SeqCst), 1);
        assert_eq!(completion.load(Ordering::SeqCst), 0);
        let ready = loop {
            pending = match pending.poll().unwrap() {
                crate::flow::one_xs::one_xs_belt_gpu::map_patch_gpu::ValidityPoll::Pending(
                    pending,
                ) => pending,
                crate::flow::one_xs::one_xs_belt_gpu::map_patch_gpu::ValidityPoll::Ready(ready) => {
                    break ready;
                }
            };
        };
        let diagnostic = ready.diagnostic_readback().unwrap();
        assert_eq!(diagnostic.frame, flight.frame);
        assert_eq!(
            diagnostic.packed.nodes().len(),
            crate::studio_type2::MAP_NODES
        );
        const PREIMAGE_WORDS: usize = 40_000;
        const FLOW_WORDS: usize = 129_600;
        const SIDE_WORDS: usize = 359_200;
        assert_eq!(
            &diagnostic.input[..PREIMAGE_WORDS],
            &parent_words[..PREIMAGE_WORDS]
        );
        assert_eq!(
            &diagnostic.input[PREIMAGE_WORDS..PREIMAGE_WORDS + FLOW_WORDS],
            &expected_base[..FLOW_WORDS]
        );
        assert_eq!(
            &diagnostic.input[PREIMAGE_WORDS + FLOW_WORDS..PREIMAGE_WORDS + 2 * FLOW_WORDS],
            &expected_public[FLOW_WORDS..2 * FLOW_WORDS]
        );
        assert_eq!(
            &diagnostic.input[SIDE_WORDS..SIDE_WORDS + PREIMAGE_WORDS],
            &parent_words[PREIMAGE_WORDS..2 * PREIMAGE_WORDS]
        );
        assert_eq!(
            &diagnostic.input
                [SIDE_WORDS + PREIMAGE_WORDS..SIDE_WORDS + PREIMAGE_WORDS + FLOW_WORDS],
            &expected_base[FLOW_WORDS..2 * FLOW_WORDS]
        );
        assert_eq!(
            &diagnostic.input[SIDE_WORDS + PREIMAGE_WORDS + FLOW_WORDS
                ..SIDE_WORDS + PREIMAGE_WORDS + 2 * FLOW_WORDS],
            &expected_public[..FLOW_WORDS]
        );
        drop(diagnostic);
        let direct = Arc::new(crate::direct_type2::DirectType2Pipeline::new(
            context.device(),
            &picture_layout,
            wgpu::TextureFormat::Rgba8Unorm,
        ));
        let _recreated_pipeline = Arc::new(crate::direct_type2::DirectType2Pipeline::new(
            context.device(),
            &picture_layout,
            wgpu::TextureFormat::Rgba8Unorm,
        ));
        let retirement_owner = Arc::new(crate::draw_retirement::IcedDrawRetirements::new(
            context.device(),
            2,
        ));
        let iced_draw =
            crate::flow::one_xs_belt_gpu::IcedInstalledDrawAdapter::with_shared_retirements(
                Arc::clone(&retirement_owner),
            );
        let retirements = retirement_owner.as_ref();
        let witness = Arc::new(AtomicU8::new(0));
        let install = crate::flow::one_xs_belt_gpu::prepare_resident_install(
            ready,
            Arc::clone(&direct),
            retirements,
        )
        .unwrap();
        let installed = install.install().unwrap();
        let reframe = crate::Reframe::blank(1.0, false);
        let installed_snapshot = capture.snapshot();
        assert!(installed_snapshot.ready);
        assert!(!installed_snapshot.pending);
        assert!(installed_snapshot.committed.is_some());
        assert_eq!(
            completion.load(Ordering::SeqCst),
            0,
            "mapped final validity must disarm the exact lease without waiting again"
        );

        // The next real resident source frame must derive every warm input
        // from the exact Cold2 successor just installed above. It traverses
        // motion, warm L2, the retained bridge and warm L1 on that imported
        // source's one SubmissionLease; no detached prior or loose buffer can
        // enter this call.
        let warm_reservation = capture
            .reserve(FrameStamp::for_test(72, Duration::from_millis(72), None))
            .unwrap();
        let installed_cold2 = Arc::clone(installed_snapshot.committed.as_ref().unwrap());
        assert!(
            warm_reservation
                .installed_prior()
                .unwrap()
                .same_successor(&installed_cold2)
        );
        let warm_flight = warm_reservation.flight().clone();
        let warm_parent = context.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 installed Cold2 to warm resident parent"),
            size: PARENT_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        context.queue().write_buffer(&warm_parent, 0, &parent_bytes);
        let warm_encoder =
            context
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("ONE X2 installed Cold2 to warm resident parent"),
                });
        let warm_geometry = geometry
            .encode_buffer(
                warm_parent.clone(),
                ParentRetention::Resident(ResidentGpuParentMaps::for_geometry_test(
                    &context,
                    warm_parent,
                )),
                Some(warm_flight.clone()),
                Some(warm_reservation),
                warm_encoder,
            )
            .unwrap();
        let warm_source = crate::direct_type2::ImportedOneXsPicture::resident_test_owner(
            &context,
            session.clone(),
            warm_flight.frame.clone(),
        );
        let warm_terminal = warm_geometry
            .submit_belts(
                &GpuSolverBeltPipeline::new(context.clone()).unwrap(),
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                warm_source,
            )
            .unwrap()
            .prepare_motion(&motion)
            .unwrap()
            .submit_resident_warm(
                &front,
                &solver,
                &bridge,
                GpuL2Controls::resident(disparity, disparity),
                GpuL1Controls::resident(disparity, disparity),
            )
            .unwrap_or_else(|error| {
                panic!("installed Cold2 to warm L2/L1 failed on {adapter}: {error}")
            });
        assert_eq!(warm_terminal.receipt_for_test().flight, warm_flight);
        assert_eq!(
            warm_terminal.receipt_for_test().stage,
            crate::flow::one_xs::scalar::PairSolveStage::Warm {
                level: crate::flow::one_xs::pis::Level::One,
            }
        );
        assert!(
            warm_terminal
                .post_for_test()
                .installed_prior_matches_for_test(&installed_cold2)
        );
        let current_lack = warm_terminal.current_l1_lack_for_test().unwrap();
        let actual_l1 = warm_terminal.work_modes_for_test(Level::One).unwrap();
        let actual_l2 = warm_terminal.work_modes_for_test(Level::Two).unwrap();
        let expected_l1 = [
            current_lack[0]
                .iter()
                .map(|word| u32::from(*word != 0))
                .collect::<Vec<_>>(),
            installed_lack[rows..]
                .iter()
                .map(|word| u32::from(*word != 0))
                .collect::<Vec<_>>(),
        ];
        let planted_a = installed_lack[..rows]
            .iter()
            .map(|word| u32::from(*word != 0))
            .collect::<Vec<_>>();
        assert_ne!(
            expected_l1[0], planted_a,
            "same-flight A lack rows must differ from planted installed prior"
        );
        assert_eq!(actual_l1, expected_l1, "warm L1 chose the wrong lack owner");
        let expected_l2 = expected_l1.clone().map(|modes| {
            propagate_work_modes(
                &modes
                    .iter()
                    .map(|word| {
                        if *word == 0 {
                            CostMode::Unweighted
                        } else {
                            CostMode::Weighted
                        }
                    })
                    .collect::<Vec<_>>(),
            )
            .into_iter()
            .map(|mode| u32::from(mode == CostMode::Weighted))
            .collect::<Vec<_>>()
        });
        assert_eq!(actual_l2, expected_l2, "warm L2 chose the wrong lack owner");
        let warm_operands = warm_terminal
            .complete_warm_final(&bridge)
            .unwrap_or_else(|error| panic!("first warm post-L1 failed on {adapter}: {error}"));
        let mut warm_pending = materializer.materialize_final(warm_operands).unwrap();
        let warm_ready = loop {
            warm_pending = match warm_pending.poll().unwrap() {
                crate::flow::one_xs::one_xs_belt_gpu::map_patch_gpu::ValidityPoll::Pending(
                    pending,
                ) => pending,
                crate::flow::one_xs::one_xs_belt_gpu::map_patch_gpu::ValidityPoll::Ready(ready) => {
                    break ready;
                }
            };
        };
        warm_ready.assert_cpu_twin_for_test().unwrap();
        warm_ready
            .assert_alpha_for_test(resources.alpha().bytes())
            .unwrap();
        let warm_install = crate::flow::one_xs_belt_gpu::prepare_resident_install(
            warm_ready,
            Arc::clone(&direct),
            retirements,
        )
        .unwrap();
        let warm_installed = warm_install.install().unwrap();
        let warm1_snapshot = capture.snapshot();
        assert!(!warm1_snapshot.pending);
        let installed_warm1 = Arc::clone(warm1_snapshot.committed.as_ref().unwrap());
        assert!(!Arc::ptr_eq(&installed_warm1, &installed_cold2));
        assert_eq!(installed_warm1.calculation_for_test(), 3);
        assert_eq!(installed_warm1.cadence_for_test(), [1, 8]);
        let warm1_fingerprint = installed_warm1
            .successor_fingerprint_for_test(&context)
            .unwrap();

        // A later warm frame must consume the exact first-warm allocation,
        // including its newly classified rows, and install another complete
        // map through the same generic final path.
        drop(installed);
        let later_reservation = capture
            .reserve(FrameStamp::for_test(73, Duration::from_millis(73), None))
            .unwrap();
        assert!(
            later_reservation
                .installed_prior()
                .unwrap()
                .same_successor(&installed_warm1)
        );
        let later_flight = later_reservation.flight().clone();
        let later_parent = context.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 installed warm to later warm resident parent"),
            size: PARENT_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        context
            .queue()
            .write_buffer(&later_parent, 0, &parent_bytes);
        let later_encoder =
            context
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("ONE X2 installed warm to later warm resident parent"),
                });
        let later_geometry = geometry
            .encode_buffer(
                later_parent.clone(),
                ParentRetention::Resident(ResidentGpuParentMaps::for_geometry_test(
                    &context,
                    later_parent,
                )),
                Some(later_flight.clone()),
                Some(later_reservation),
                later_encoder,
            )
            .unwrap();
        let later_source = crate::direct_type2::ImportedOneXsPicture::resident_test_owner(
            &context,
            session.clone(),
            later_flight.frame.clone(),
        );
        let later_terminal = later_geometry
            .submit_belts(
                &GpuSolverBeltPipeline::new(context.clone()).unwrap(),
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                later_source,
            )
            .unwrap()
            .prepare_motion(&motion)
            .unwrap()
            .submit_resident_warm(
                &front,
                &solver,
                &bridge,
                GpuL2Controls::resident(disparity, disparity),
                GpuL1Controls::resident(disparity, disparity),
            )
            .unwrap_or_else(|error| panic!("later warm L2/L1 failed on {adapter}: {error}"));
        assert!(
            later_terminal
                .post_for_test()
                .installed_prior_matches_for_test(&installed_warm1)
        );
        let later_operands = later_terminal
            .complete_warm_final(&bridge)
            .unwrap_or_else(|error| panic!("later warm post-L1 failed on {adapter}: {error}"));
        let mut later_pending = materializer.materialize_final(later_operands).unwrap();
        let later_ready = loop {
            later_pending = match later_pending.poll().unwrap() {
                crate::flow::one_xs::one_xs_belt_gpu::map_patch_gpu::ValidityPoll::Pending(
                    pending,
                ) => pending,
                crate::flow::one_xs::one_xs_belt_gpu::map_patch_gpu::ValidityPoll::Ready(ready) => {
                    break ready;
                }
            };
        };
        later_ready.assert_cpu_twin_for_test().unwrap();
        later_ready
            .assert_alpha_for_test(resources.alpha().bytes())
            .unwrap();
        let mut later_install = crate::flow::one_xs_belt_gpu::prepare_resident_install(
            later_ready,
            Arc::clone(&direct),
            retirements,
        )
        .unwrap();
        later_install.observe_draw_drop(Arc::clone(&witness));
        let later_installed = later_install.install().unwrap();
        let later_snapshot = capture.snapshot();
        let installed_warm2 = Arc::clone(later_snapshot.committed.as_ref().unwrap());
        assert!(!Arc::ptr_eq(&installed_warm2, &installed_warm1));
        assert_eq!(installed_warm2.calculation_for_test(), 3);
        assert_eq!(installed_warm2.cadence_for_test(), [2, 9]);
        assert_eq!(
            installed_warm1
                .successor_fingerprint_for_test(&context)
                .unwrap(),
            warm1_fingerprint,
            "later warm mutated its installed predecessor"
        );

        let prepare_additional_warm = |frame_index: u64| {
            let prior_snapshot = capture.snapshot();
            let prior = Arc::clone(prior_snapshot.committed.as_ref().unwrap());
            let reservation = capture
                .reserve(FrameStamp::for_test(
                    frame_index,
                    Duration::from_millis(frame_index),
                    None,
                ))
                .unwrap();
            assert!(
                reservation
                    .installed_prior()
                    .unwrap()
                    .same_successor(&prior)
            );
            let flight = reservation.flight().clone();
            let parent = context.device().create_buffer(&wgpu::BufferDescriptor {
                label: Some("ONE X2 additional warm resident parent"),
                size: PARENT_BYTES,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            context.queue().write_buffer(&parent, 0, &parent_bytes);
            let encoder =
                context
                    .device()
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("ONE X2 additional warm resident parent"),
                    });
            let geometry = geometry
                .encode_buffer(
                    parent.clone(),
                    ParentRetention::Resident(ResidentGpuParentMaps::for_geometry_test(
                        &context, parent,
                    )),
                    Some(flight.clone()),
                    Some(reservation),
                    encoder,
                )
                .unwrap();
            let source = crate::direct_type2::ImportedOneXsPicture::resident_test_owner(
                &context,
                session.clone(),
                flight.frame.clone(),
            );
            let terminal = geometry
                .submit_belts(
                    &GpuSolverBeltPipeline::new(context.clone()).unwrap(),
                    SourceTextures {
                        a: &texture_a,
                        b: &texture_b,
                    },
                    source,
                )
                .unwrap()
                .prepare_motion(&motion)
                .unwrap()
                .submit_resident_warm(
                    &front,
                    &solver,
                    &bridge,
                    GpuL2Controls::resident(disparity, disparity),
                    GpuL1Controls::resident(disparity, disparity),
                )
                .unwrap_or_else(|error| {
                    panic!("additional warm L2/L1 failed on {adapter}: {error}")
                });
            assert!(
                terminal
                    .post_for_test()
                    .installed_prior_matches_for_test(&prior)
            );
            terminal
        };

        // A failing status planted on the exact warm L1 terminal must remain
        // the same four-byte validity owner through post-L1 and final map. Its
        // refusal rolls the pending root back without touching warm2.
        let warm2_fingerprint = installed_warm2
            .successor_fingerprint_for_test(&context)
            .unwrap();
        let invalid_terminal = prepare_additional_warm(74);
        invalid_terminal
            .inject_inherited_validity_for_test(0)
            .unwrap();
        let invalid_completion = Arc::new(AtomicU8::new(0));
        let mut invalid_operands = invalid_terminal.complete_warm_final(&bridge).unwrap();
        invalid_operands.observe_final_completion(Arc::clone(&invalid_completion));
        let mut invalid_pending = materializer.materialize_final(invalid_operands).unwrap();
        let invalid_error = loop {
            match invalid_pending.poll() {
                Ok(crate::flow::one_xs::one_xs_belt_gpu::map_patch_gpu::ValidityPoll::Pending(
                    pending,
                )) => invalid_pending = pending,
                Ok(crate::flow::one_xs::one_xs_belt_gpu::map_patch_gpu::ValidityPoll::Ready(_)) => {
                    panic!("inherited invalid warm validity became installable")
                }
                Err(error) => break error,
            }
        };
        assert!(invalid_error.to_string().contains("is not finite"));
        assert_eq!(
            invalid_completion.load(Ordering::SeqCst),
            0,
            "semantic validity refusal must disarm mapped completion without waiting"
        );
        let invalid_snapshot = capture.snapshot();
        assert!(!invalid_snapshot.pending);
        assert!(later_snapshot.same_ready(&invalid_snapshot));
        assert!(Arc::ptr_eq(
            later_snapshot.committed.as_ref().unwrap(),
            invalid_snapshot.committed.as_ref().unwrap()
        ));
        assert_eq!(
            installed_warm2
                .successor_fingerprint_for_test(&context)
                .unwrap(),
            warm2_fingerprint,
            "invalid inherited validity mutated its installed predecessor"
        );

        // Both successful installed draws still hold the two retirement
        // permits. Drive another real warm frame all the way through post-L1,
        // classification and final mapping, then prove atomic install refuses
        // that fully materialized candidate and preserves warm2.
        let full_terminal = prepare_additional_warm(75);
        let full_operands = full_terminal.complete_warm_final(&bridge).unwrap();
        let mut full_pending = materializer.materialize_final(full_operands).unwrap();
        let full_ready = loop {
            full_pending = match full_pending.poll().unwrap() {
                crate::flow::one_xs::one_xs_belt_gpu::map_patch_gpu::ValidityPoll::Pending(
                    pending,
                ) => pending,
                crate::flow::one_xs::one_xs_belt_gpu::map_patch_gpu::ValidityPoll::Ready(ready) => {
                    break ready;
                }
            };
        };
        full_ready.assert_cpu_twin_for_test().unwrap();
        full_ready
            .assert_alpha_for_test(resources.alpha().bytes())
            .unwrap();
        let error = match crate::flow::one_xs_belt_gpu::prepare_resident_install(
            full_ready,
            Arc::clone(&direct),
            retirements,
        ) {
            Ok(_) => panic!("full retirement admitted a materialized warm install"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "ONE X2 draw retirement is full");
        let full_snapshot = capture.snapshot();
        assert!(!full_snapshot.pending);
        assert!(later_snapshot.same_ready(&full_snapshot));
        assert!(Arc::ptr_eq(
            later_snapshot.committed.as_ref().unwrap(),
            full_snapshot.committed.as_ref().unwrap()
        ));
        assert_eq!(
            installed_warm2
                .successor_fingerprint_for_test(&context)
                .unwrap(),
            warm2_fingerprint,
            "retirement-full warm candidate mutated its installed predecessor"
        );

        let target = context.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("resident installed draw target"),
            size: wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let draw_once = |iced_draw: &crate::flow::one_xs_belt_gpu::IcedInstalledDrawAdapter| {
            let mut encoder =
                context
                    .device()
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("resident installed draw"),
                    });
            let view = target.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("resident installed draw"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            assert!(iced_draw.arm_and_draw(&mut pass));
            drop(pass);
            context.queue().submit([encoder.finish()])
        };
        iced_draw.prepare_installed(warm_installed, &reframe);
        let first_draw_uniform = iced_draw.staged_uniform_for_test();
        assert_eq!(read_uniform(&context, &first_draw_uniform), reframe.bytes());
        let _first_draw = draw_once(&iced_draw);
        drop(iced_draw);
        let iced_draw =
            crate::flow::one_xs_belt_gpu::IcedInstalledDrawAdapter::with_shared_retirements(
                Arc::clone(&retirement_owner),
            );
        let redraw_reframe = crate::Reframe::blank(0.5, true);
        iced_draw.prepare_installed(later_installed, &redraw_reframe);
        let second_draw_uniform = iced_draw.staged_uniform_for_test();
        assert_eq!(
            read_uniform(&context, &second_draw_uniform),
            redraw_reframe.bytes()
        );
        assert_eq!(read_uniform(&context, &first_draw_uniform), reframe.bytes());
        let second_draw = draw_once(&iced_draw);
        let error = iced_draw
            .prepare_redraw(&capture, &reframe)
            .expect_err("full iced retirement admitted a third draw");
        assert_eq!(error, crate::draw_retirement::DrawRetirementError::Full);
        let mut refused_encoder =
            context
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("resident refused iced draw"),
                });
        let refused_view = target.create_view(&Default::default());
        let mut refused_pass = refused_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("resident refused iced draw"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &refused_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        assert!(!iced_draw.arm_and_draw(&mut refused_pass));
        drop(refused_pass);
        drop(refused_encoder);
        context
            .device()
            .poll(wgpu::PollType::Wait {
                submission_index: Some(second_draw),
                timeout: None,
            })
            .unwrap();
        assert_eq!(iced_draw.poll_prepare().unwrap(), 2);
        assert!(iced_draw.prepare_redraw(&capture, &reframe).unwrap());
        let screen_uniform = iced_draw.staged_uniform_for_test();
        assert_eq!(read_uniform(&context, &screen_uniform), reframe.bytes());
        let screenshot_reframe = crate::Reframe::blank(1.75, true);
        let mut screenshot = capture
            .ready_for_draw(retirements)
            .unwrap()
            .expect("installed root has a screenshot ready");
        screenshot.write_reframe(&screenshot_reframe);
        let screenshot_uniform = screenshot.uniform_for_test();
        assert_eq!(
            read_uniform(&context, &screenshot_uniform),
            screenshot_reframe.bytes()
        );
        assert_eq!(
            read_uniform(&context, &screen_uniform),
            reframe.bytes(),
            "screenshot preparation changed the staged window Reframe"
        );
        let mut screenshot_encoder =
            context
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("resident screenshot before window submit"),
                });
        let screenshot_view = target.create_view(&Default::default());
        let mut screenshot_pass =
            screenshot_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("resident screenshot before window submit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &screenshot_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        screenshot.arm_and_draw(retirements, &mut screenshot_pass);
        drop(screenshot_pass);
        context.queue().submit([screenshot_encoder.finish()]);
        let third_draw = draw_once(&iced_draw);
        assert_eq!(read_uniform(&context, &screen_uniform), reframe.bytes());
        assert_eq!(
            read_uniform(&context, &screenshot_uniform),
            screenshot_reframe.bytes()
        );
        context
            .device()
            .poll(wgpu::PollType::Wait {
                submission_index: Some(third_draw),
                timeout: None,
            })
            .unwrap();
        assert_eq!(iced_draw.poll_prepare().unwrap(), 2);
        let redraw_snapshot = capture.snapshot();
        assert!(later_snapshot.same_ready(&redraw_snapshot));
        assert!(Arc::ptr_eq(
            later_snapshot.committed.as_ref().unwrap(),
            redraw_snapshot.committed.as_ref().unwrap()
        ));

        // Remove the root's final Arc only through a purpose-specific test
        // capability, then prove an install refusal drops that whole carrier
        // before the candidate begins reservation rollback.
        let ready = capture.take_ready_for_drop_order_test(retirements);
        let refusal = capture
            .reserve(FrameStamp::for_test(76, Duration::from_millis(76), None))
            .unwrap();
        let next_references = context.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("resident drop-order successor"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let successor =
            crate::flow::one_xs_belt_gpu::resident_frame_gpu::ResidentSuccessor::from_motion(
                refusal.flight().clone(),
                next_references,
            );
        let mut refusal = crate::flow::one_xs_belt_gpu::ResidentInstallCandidate {
            draw: Some(ready.draw),
            root: Some(refusal.seal(successor).unwrap()),
            panic_before_root_install: false,
            permit: Some(ready.permit),
        };
        refusal.observe_root_rollback(Arc::clone(&witness));
        let error = refusal.probe_install_refusal().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("different imported picture allocation")
        );
        assert_eq!(witness.load(Ordering::SeqCst), 0);

        refusal.inject_stale_root();
        let error = refusal.probe_install_refusal().unwrap_err();
        assert!(error.to_string().contains("candidate quarantined"));
        assert_eq!(witness.load(Ordering::SeqCst), 0);

        refusal.inject_install_panic();
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = refusal.install();
        }));
        assert!(unwind.is_err(), "injected resident install did not unwind");
        assert_eq!(witness.load(Ordering::SeqCst), 2);
    }

    fn read_uniform(context: &OneXsGpuContext, source: &wgpu::Buffer) -> Vec<u8> {
        let size = std::mem::size_of::<crate::Reframe>() as u64;
        let readback = context.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("resident installed uniform readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder =
            context
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("resident installed uniform readback"),
                });
        encoder.copy_buffer_to_buffer(source, 0, &readback, 0, size);
        let submission = context.queue().submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (sent, received) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            sent.send(result).unwrap()
        });
        context
            .device()
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        received.recv().unwrap().unwrap();
        slice.get_mapped_range().to_vec()
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

    fn gpu() -> Result<(OneXsGpuContext, OneXsGpuContext, String), String> {
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
        let (foreign_device, foreign_queue) =
            block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("foreign exact ONE X2 GPU geometry"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            }))
            .map_err(|error| error.to_string())?;
        Ok((
            OneXsGpuContext::new(&device, &queue),
            OneXsGpuContext::new(&foreign_device, &foreign_queue),
            name,
        ))
    }
}
