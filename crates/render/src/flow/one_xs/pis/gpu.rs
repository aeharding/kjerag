//! Exact paired GPU boundary for the selected ONE X2 patch inverse search.
//!
//! CPU code admits [`Input`], [`InitialGrid`], [`HintGrid`] and
//! [`DescentAdmission`] at their existing typed boundary, then serializes an
//! explicit `u32`/`f32` storage contract. One 32-lane workgroup owns each
//! direction, advances the native in-place passes by anti-diagonal, and assigns
//! four lanes to each active patch's native candidate reduction. Construction
//! qualifies this same paired production entry against both CPU directions on
//! the actual adapter.

use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::sync::mpsc;

use kjerag_media::FrameStamp;

use super::{
    AtoB, BtoA, CostMode, DescentAdmission, Direction, DisparityInterval, Flow, HintGrid,
    InitialGrid, Input, Level, PisDirection, solve_with_descent_admission,
};
use crate::Fallible;
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
use crate::flow::one_xs::pis_frontend_gpu::{
    GpuPreparedFrame, GpuPreparedTerminal, PreparedPisDispatch,
};
use crate::flow::one_xs::scalar::{
    CpuPisOracleInputs, PairSolveStage, PairedPatchGrids as ScalarPairedPatchGrids,
    PairedSolveRequest, SolveStamp, StampedPatchGrid,
};

const HEADER_WORDS: usize = 32;
const OUTPUT_WORDS_PER_PATCH: usize = 2;
const DIVISION_PROBES: &[(u32, u32, u32)] = &[
    (0x0000_0001, 0x4000_0000, 0x0000_0000),
    (0x0000_0003, 0x4000_0000, 0x0000_0002),
    (0x0080_0000, 0x4000_0000, 0x0040_0000),
    (0x00ff_ffff, 0x4000_0000, 0x0080_0000),
    (0x7f7f_ffff, 0x3f00_0000, 0x7f80_0000),
    (0xff7f_ffff, 0x3f00_0000, 0xff80_0000),
    (0x0080_0000, 0x7f7f_ffff, 0x0000_0000),
    (0x7f7f_ffff, 0x0080_0000, 0x7f80_0000),
    (0x478f_e475, 0x3a83_126f, 0x4c8c_851a),
    (0xc54c_efee, 0x3a83_126f, 0xca48_224e),
    (0x4547_e588, 0x3a83_126f, 0x4a43_3626),
    (0x42a1_7a6c, 0x4200_0000, 0x4021_7a6c),
    (0xc327_2b0f, 0x4280_0000, 0xc027_2b0f),
    (0x3f80_0000, 0x4774_2400, 0x3786_37bd),
    (0x0000_0000, 0x0000_0000, 0xffc0_0000),
    (0x8000_0000, 0x0000_0000, 0xffc0_0000),
    (0x0000_0000, 0x8000_0000, 0xffc0_0000),
    (0x8000_0000, 0x8000_0000, 0xffc0_0000),
];

// Each tuple is (dx bits, dy bits, reject). These are the exact f32
// subtraction/f64-hypot rounding discriminators from the boundary audit.
const DISTANCE_PROBES: &[(u32, u32, u32)] = &[
    (0x4100_0000, 0x3400_0000, 0),
    (0x4100_0000, 0x3400_0001, 1),
    (0x4100_0000, 0x0000_0001, 0),
    (0x4100_0001, 0x0000_0000, 1),
    (0x40ff_ffff, 0x3b35_04f3, 0),
    (0x40ff_ffff, 0x3b35_04f4, 1),
    (0x7f80_0000, 0x0000_0000, 1),
    (0x7fc0_0001, 0x0000_0000, 0),
];

const DISPARITY_PROBES: usize = 5;
const PROBE_WORDS: usize = 1 + DIVISION_PROBES.len() + DISTANCE_PROBES.len() + DISPARITY_PROBES;

/// One capture-owned GPU sparse-solver transaction.
///
/// The full decoded-pair identity prevents index/time ABA, and the monotonic
/// generation prevents a delayed completion from impersonating a later
/// reservation of that same delivery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GpuPisFlight {
    pub(crate) generation: u64,
    pub(crate) frame: FrameStamp,
}

/// Identity of one paired solve inside a reserved GPU transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GpuPisStageReceipt {
    pub(crate) flight: GpuPisFlight,
    pub(crate) stage: PairSolveStage,
}

/// Typed terminal grids and the exact reservation/stage that submitted them.
pub(crate) struct GpuPisStageOutput {
    pub(crate) receipt: GpuPisStageReceipt,
    pub(crate) grids: ScalarPairedPatchGrids,
}

/// The only frame-varying inputs to one direction of a direct-bound stage.
///
/// Image-owned terms are deliberately absent. They remain sealed in the one
/// [`GpuPreparedFrame`] shared by every cold and warm stage for this flight.
pub(crate) struct GpuPisDynamicDirection<D: PisDirection> {
    pub(crate) cost_modes: Box<[CostMode]>,
    pub(crate) initial: InitialGrid<D>,
    pub(crate) hint: Option<HintGrid<D>>,
    pub(crate) admission: DescentAdmission,
    pub(crate) disparity: Option<DisparityInterval>,
}

#[cfg(test)]
impl<D: PisDirection> GpuPisDynamicDirection<D> {
    /// Test-only extraction of the dynamic half from the readable CPU oracle.
    /// Production direct binding has no conversion from [`Input`].
    pub(crate) fn from_oracle(
        input: &Input<D>,
        initial: InitialGrid<D>,
        hint: Option<HintGrid<D>>,
        admission: DescentAdmission,
    ) -> Self {
        Self {
            cost_modes: input.cost_modes.clone(),
            initial,
            hint,
            admission,
            disparity: input.disparity,
        }
    }
}

/// Both direction-specific dynamic inputs for one direct-bound stage.
pub(crate) struct GpuPisDynamicStage {
    pub(crate) stage: PairSolveStage,
    pub(crate) a_to_b: GpuPisDynamicDirection<AtoB>,
    pub(crate) b_to_a: GpuPisDynamicDirection<BtoA>,
}

/// One direct-bound terminal result and the same linear prepared-frame token.
///
/// Owning rather than borrowing the frame is intentional: a caller must take
/// this exact token into the next cold/warm stage, and the resident submission
/// lease can later be advanced and acknowledged only at the terminal CPU
/// boundary without inventing a second lifetime owner.
#[must_use = "the direct-bound GPU PIS terminal and prepared frame have not been consumed"]
#[cfg(test)]
pub(crate) struct GpuPreparedStageOutput<K> {
    pub(crate) receipt: GpuPisStageReceipt,
    pub(crate) grids: ScalarPairedPatchGrids,
    prepared: GpuPreparedFrame<K>,
}

#[cfg(test)]
impl<K> GpuPreparedStageOutput<K> {
    pub(crate) fn into_parts(
        self,
    ) -> (
        GpuPisStageReceipt,
        ScalarPairedPatchGrids,
        GpuPreparedFrame<K>,
    ) {
        (self.receipt, self.grids, self.prepared)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct TerminalBits<D: PisDirection> {
    level: Level,
    dcol: Box<[u32]>,
    drow: Box<[u32]>,
    diagnostics: Box<[u32]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> TerminalBits<D> {
    /// Re-enter the typed CPU boundary without changing any terminal bits.
    /// Pass reports are intentionally empty because this production handoff
    /// retains only the native terminal grid.
    fn into_patch_grid(self) -> Result<super::PatchGrid<D>, TerminalGridError> {
        let expected = self.level.patches();
        for (component, actual) in [("dcol", self.dcol.len()), ("drow", self.drow.len())] {
            if actual != expected {
                return Err(TerminalGridError::Shape {
                    direction: D::DIRECTION,
                    level: self.level,
                    component,
                    expected,
                    actual,
                });
            }
        }
        let patches = self
            .dcol
            .iter()
            .copied()
            .zip(self.drow.iter().copied())
            .enumerate()
            .map(|(patch, (dcol, drow))| {
                let dcol_value = f32::from_bits(dcol);
                let drow_value = f32::from_bits(drow);
                let flow = Flow::new(dcol_value, drow_value).ok_or_else(|| {
                    let (component, bits) = if !dcol_value.is_finite() {
                        ("dcol", dcol)
                    } else {
                        ("drow", drow)
                    };
                    TerminalGridError::NonFinite {
                        direction: D::DIRECTION,
                        level: self.level,
                        patch,
                        component,
                        bits,
                    }
                })?;
                Ok(super::Patch::seeded(flow))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice();
        Ok(super::PatchGrid {
            level: self.level,
            patches,
            direction: PhantomData,
        })
    }
}

/// A terminal GPU bit grid could not safely re-enter the typed CPU boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalGridError {
    Span {
        direction: Direction,
        level: Level,
        expected_words: usize,
        actual_words: usize,
    },
    Shape {
        direction: Direction,
        level: Level,
        component: &'static str,
        expected: usize,
        actual: usize,
    },
    NonFinite {
        direction: Direction,
        level: Level,
        patch: usize,
        component: &'static str,
        bits: u32,
    },
}

impl fmt::Display for TerminalGridError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Span {
                direction,
                level,
                expected_words,
                actual_words,
            } => write!(
                out,
                "ONE X2 GPU PIS {direction:?} {level} terminal readback has {actual_words} words, expected {expected_words}"
            ),
            Self::Shape {
                direction,
                level,
                component,
                expected,
                actual,
            } => write!(
                out,
                "ONE X2 GPU PIS {direction:?} {level} terminal {component} has {actual} patches, expected {expected}"
            ),
            Self::NonFinite {
                direction,
                level,
                patch,
                component,
                bits,
            } => write!(
                out,
                "ONE X2 GPU PIS {direction:?} {level} patch {patch} terminal {component} bits {bits:#010x} are not finite"
            ),
        }
    }
}

impl Error for TerminalGridError {}

#[derive(Debug, PartialEq, Eq)]
struct PairedTerminalBits {
    a_to_b: TerminalBits<AtoB>,
    b_to_a: TerminalBits<BtoA>,
}

impl PairedTerminalBits {
    fn into_patch_grids(self) -> Result<PairedPatchGrids, TerminalGridError> {
        Ok(PairedPatchGrids {
            a_to_b: self.a_to_b.into_patch_grid()?,
            b_to_a: self.b_to_a.into_patch_grid()?,
        })
    }
}

/// Direction-typed terminal grids admitted back through the CPU boundary.
#[derive(Debug, PartialEq)]
pub(crate) struct PairedPatchGrids {
    pub(crate) a_to_b: super::PatchGrid<AtoB>,
    pub(crate) b_to_a: super::PatchGrid<BtoA>,
}

#[derive(Debug, PartialEq, Eq)]
enum QualificationError {
    Direction {
        direction: &'static str,
        level: Level,
        patch: usize,
        component: &'static str,
        actual: u32,
        expected: u32,
        expected_candidates: Box<[u32]>,
    },
    Probe {
        direction: &'static str,
        probe: usize,
        actual: u32,
        expected: u32,
    },
}

impl fmt::Display for QualificationError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direction {
                direction,
                level,
                patch,
                component,
                actual,
                expected,
                expected_candidates,
            } => write!(
                out,
                "ONE X2 GPU PIS arithmetic is not exact on this graphics device: {direction} {level} patch {patch} {component} bits are {actual:#010x}, expected {expected:#010x}; CPU pass-0 candidate score bits {expected_candidates:#010x?}",
            ),
            Self::Probe {
                direction,
                probe,
                actual,
                expected,
            } => write!(
                out,
                "ONE X2 GPU PIS qualification probe is not exact on this graphics device: {direction} probe {probe} bits are {actual:#010x}, expected {expected:#010x}",
            ),
        }
    }
}

impl Error for QualificationError {}

/// Render-internal paired PIS compute state.
pub(crate) struct GpuPisPipeline {
    context: OneXsGpuContext,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    /// Legacy CPU-oracle submissions do not read the prepared bindings.
    oracle_placeholder: wgpu::Buffer,
}

fn validate_direct_stage(receipt: PairSolveStage, request: PairSolveStage) -> Fallible<()> {
    if receipt != request {
        return Err(format!(
            "ONE X2 direct GPU PIS receipt names {receipt}, but its request names {request}"
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn validate_direct_stage_for_test(
    receipt: PairSolveStage,
    request: PairSolveStage,
) -> Fallible<()> {
    validate_direct_stage(receipt, request)
}

impl GpuPisPipeline {
    /// Build and qualify the actual production shader entry on this device.
    pub(crate) fn new(context: OneXsGpuContext) -> Fallible<Self> {
        Self::from_shader(context, SHADER, true)
    }

    #[cfg(test)]
    pub(crate) fn from_shader_for_direct_test(
        context: OneXsGpuContext,
        shader: &str,
    ) -> Fallible<Self> {
        Self::from_shader(context, shader, false)
    }

    /// Solve one scalar transaction stage while preserving its outer receipt.
    ///
    /// CPU construction of the prepared source models and their upload remain
    /// the explicit producer boundary. The receipt is not shader arithmetic;
    /// it binds the synchronous readback to the capture reservation that owns
    /// the request before typed grids can re-enter the scalar transaction.
    pub(crate) fn solve_request(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        receipt: GpuPisStageReceipt,
        prepared: &CpuPisOracleInputs,
        request: PairedSolveRequest,
    ) -> Fallible<GpuPisStageOutput> {
        if receipt.stage != request.stage {
            return Err(format!(
                "ONE X2 GPU PIS receipt names {}, but its request names {}",
                receipt.stage, request.stage
            )
            .into());
        }
        let PairedSolveRequest {
            stage,
            a_to_b,
            b_to_a,
        } = request;
        let level = stage.level();
        let a_to_b_input = prepared.a_to_b(level, a_to_b.cost_modes);
        let b_to_a_input = prepared.b_to_a(level, b_to_a.cost_modes);
        let terminal = self.solve_pair(
            device,
            queue,
            &a_to_b_input,
            a_to_b.initial,
            Some(&a_to_b.hint),
            &b_to_a_input,
            b_to_a.initial,
            Some(&b_to_a.hint),
            a_to_b.admission,
            b_to_a.admission,
        )?;
        let grids = ScalarPairedPatchGrids {
            a_to_b: StampedPatchGrid::new(
                SolveStamp {
                    direction: Direction::AtoB,
                    stage,
                },
                terminal.a_to_b,
            ),
            b_to_a: StampedPatchGrid::new(
                SolveStamp {
                    direction: Direction::BtoA,
                    stage,
                },
                terminal.b_to_a,
            ),
        };
        Ok(GpuPisStageOutput { receipt, grids })
    }

    /// Bind one immutable prepared frame directly into the paired kernel.
    ///
    /// This standalone entry is deliberately not selected by `Scene` yet.
    /// It consumes and returns the frame token so the same allocation and its
    /// single resident lease must cross every cold/warm stage. Only the
    /// terminal grids cross back into CPU memory.
    #[cfg(test)]
    pub(crate) fn solve_prepared_diagnostic<K>(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        prepared: GpuPreparedFrame<K>,
        receipt: GpuPisStageReceipt,
        request: GpuPisDynamicStage,
    ) -> Fallible<GpuPreparedStageOutput<K>> {
        validate_direct_stage(receipt.stage, request.stage)?;
        let level = request.stage.level();
        let stage = request.stage;
        let submitted = prepared.submit_pis_stage(self, request)?;
        let (actual_receipt, words, prepared) = submitted.readback_for_test()?;
        if actual_receipt != receipt {
            return Err(format!(
                "ONE X2 diagnostic GPU PIS returned receipt {:?}, expected {:?}",
                actual_receipt, receipt
            )
            .into());
        }
        let span = level.patches() * OUTPUT_WORDS_PER_PATCH;
        let terminal = PairedTerminalBits {
            a_to_b: decode_terminal(level, &words[..span], 0)?,
            b_to_a: decode_terminal(level, &words[span..2 * span], 0)?,
        }
        .into_patch_grids()?;
        let grids = ScalarPairedPatchGrids {
            a_to_b: StampedPatchGrid::new(
                SolveStamp {
                    direction: Direction::AtoB,
                    stage,
                },
                terminal.a_to_b,
            ),
            b_to_a: StampedPatchGrid::new(
                SolveStamp {
                    direction: Direction::BtoA,
                    stage,
                },
                terminal.b_to_a,
            ),
        };
        Ok(GpuPreparedStageOutput {
            receipt,
            grids,
            prepared,
        })
    }

    /// Submit one direct-bound paired stage without copying, mapping or
    /// polling its terminal buffer.
    ///
    /// The returned token keeps the exact output receipt and the same linear
    /// prepared-frame lease together. A later GPU stage may consume it back
    /// into that frame, or the exact terminal boundary may acknowledge it.
    pub(crate) fn submit_prepared<K>(
        &self,
        prepared: GpuPreparedFrame<K>,
        request: GpuPisDynamicStage,
    ) -> Fallible<GpuPreparedTerminal<K>> {
        prepared.submit_pis_stage(self, request)
    }

    pub(crate) fn prepare_resident_dispatch(
        &self,
        request: GpuPisDynamicStage,
    ) -> Fallible<PreparedPisDispatch<'_>> {
        let stage = request.stage;
        let level = stage.level();
        let a = PackedInput::new_prepared(level, request.a_to_b, false)?;
        let b = PackedInput::new_prepared(level, request.b_to_a, false)?;
        let packed = PackedPair::new(a, b)?;
        Ok(PreparedPisDispatch {
            stage,
            pipeline: &self.pipeline,
            layout: &self.layout,
            u32s: packed.u32s,
            f32s: packed.f32s,
            output_words: packed.output_words,
            output_span_words: packed.output_span,
            b_output_base_words: packed.b_output_base,
        })
    }

    pub(crate) fn validate_terminal_context(&self, context: &OneXsGpuContext) -> Fallible<()> {
        self.context.ensure_same(context)
    }

    fn from_shader(context: OneXsGpuContext, shader: &str, qualify: bool) -> Fallible<Self> {
        let device = context.device().clone();
        let queue = context.queue().clone();
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
            label: Some("ONE X2 paired GPU PIS"),
            entries: &[
                storage(0, true),
                storage(1, true),
                storage(2, false),
                storage(3, true),
                storage(4, true),
                storage(5, true),
                storage(6, true),
                storage(7, true),
                storage(8, true),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 paired GPU PIS"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 paired GPU PIS"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ONE X2 paired GPU PIS"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("solve_pis_wavefront"),
            compilation_options: Default::default(),
            cache: None,
        });
        let oracle_placeholder = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 GPU PIS oracle prepared-binding placeholder"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let built = Self {
            context,
            pipeline,
            layout,
            oracle_placeholder,
        };
        if qualify {
            built.qualify(&device, &queue)?;
        }
        Ok(built)
    }

    /// Execute both native directions at one level. Inputs have already passed
    /// the CPU constructor's shape, finite, mask and rolling-denominator gates.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn solve_pair(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        a_to_b: &Input<AtoB>,
        a_initial: InitialGrid<AtoB>,
        a_hint: Option<&HintGrid<AtoB>>,
        b_to_a: &Input<BtoA>,
        b_initial: InitialGrid<BtoA>,
        b_hint: Option<&HintGrid<BtoA>>,
        a_admission: DescentAdmission,
        b_admission: DescentAdmission,
    ) -> Fallible<PairedPatchGrids> {
        self.solve_pair_mode(
            device,
            queue,
            a_to_b,
            a_initial,
            a_hint,
            b_to_a,
            b_initial,
            b_hint,
            a_admission,
            b_admission,
            false,
        )?
        .into_patch_grids()
        .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn solve_pair_mode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        a_to_b: &Input<AtoB>,
        a_initial: InitialGrid<AtoB>,
        a_hint: Option<&HintGrid<AtoB>>,
        b_to_a: &Input<BtoA>,
        b_initial: InitialGrid<BtoA>,
        b_hint: Option<&HintGrid<BtoA>>,
        a_admission: DescentAdmission,
        b_admission: DescentAdmission,
        qualification_probes: bool,
    ) -> Fallible<PairedTerminalBits> {
        if a_to_b.level != b_to_a.level {
            return Err(format!(
                "ONE X2 paired GPU PIS directions have different levels: {} and {}",
                a_to_b.level, b_to_a.level
            )
            .into());
        }
        let a = PackedInput::new(a_to_b, a_initial, a_hint, a_admission, qualification_probes)?;
        let b = PackedInput::new(b_to_a, b_initial, b_hint, b_admission, qualification_probes)?;
        self.dispatch_pair(device, queue, PackedPair::new(a, b)?)
    }

    fn dispatch_pair(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        packed: PackedPair,
    ) -> Fallible<PairedTerminalBits> {
        let u32_buffer = upload(
            device,
            queue,
            "ONE X2 GPU PIS u32 input",
            u32_bytes(&packed.u32s),
        );
        let f32_buffer = upload(
            device,
            queue,
            "ONE X2 GPU PIS f32 input",
            f32_bytes(&packed.f32s),
        );
        let output_size = (packed.output_words * 4) as u64;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 GPU PIS terminal bits"),
            size: output_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 GPU PIS terminal readback"),
            size: output_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 GPU PIS resources"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: u32_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: f32_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.oracle_placeholder.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.oracle_placeholder.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.oracle_placeholder.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: self.oracle_placeholder.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: self.oracle_placeholder.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: self.oracle_placeholder.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 paired GPU PIS"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 paired GPU PIS direction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &resources, &[]);
            pass.dispatch_workgroups(2, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output_size);
        let submission = queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (mapped, answer) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = mapped.send(result);
        });
        device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        answer.recv()??;
        let bytes = slice.get_mapped_range();
        let words = bytes
            .chunks_exact(4)
            .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
            .collect::<Vec<_>>();
        drop(bytes);
        readback.unmap();
        Ok(PairedTerminalBits {
            a_to_b: decode_terminal(
                packed.level,
                &words[packed.a_output_base..packed.a_output_base + packed.output_span],
                packed.diagnostic_words,
            )?,
            b_to_a: decode_terminal(
                packed.level,
                &words[packed.b_output_base..packed.b_output_base + packed.output_span],
                packed.diagnostic_words,
            )?,
        })
    }

    fn qualify(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Fallible<()> {
        self.qualify_case(
            device,
            queue,
            Level::Two,
            true,
            true,
            true,
            DescentAdmission::EveryPatch,
            DescentAdmission::EveryPatch,
        )?;
        self.qualify_case(
            device,
            queue,
            Level::One,
            true,
            true,
            true,
            DescentAdmission::NoPatches,
            DescentAdmission::NoPatches,
        )?;
        self.qualify_case(
            device,
            queue,
            Level::Two,
            false,
            false,
            true,
            DescentAdmission::NoPatches,
            DescentAdmission::EveryPatch,
        )?;
        self.qualify_case(
            device,
            queue,
            Level::One,
            false,
            true,
            false,
            DescentAdmission::EveryPatch,
            DescentAdmission::NoPatches,
        )?;
        self.qualify_case(
            device,
            queue,
            Level::Two,
            true,
            false,
            true,
            DescentAdmission::NoPatches,
            DescentAdmission::EveryPatch,
        )?;
        self.qualify_survivor_boundary(device, queue)?;
        self.qualify_tie_boundary(device, queue)?;
        self.qualify_zero_survivor_descent(device, queue)?;
        assert_descent_fixture_coverage()
    }

    #[allow(clippy::too_many_arguments)]
    fn qualify_case(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        level: Level,
        use_disparity: bool,
        a_uses_hint: bool,
        b_uses_hint: bool,
        a_admission: DescentAdmission,
        b_admission: DescentAdmission,
    ) -> Fallible<()> {
        let (mut a_input, mut b_input, a_initial, b_initial, a_hint, b_hint) =
            qualification_fixture(level);
        if !use_disparity {
            a_input.disparity = None;
            b_input.disparity = None;
        }
        let a_hint_ref = a_uses_hint.then_some(&a_hint);
        let b_hint_ref = b_uses_hint.then_some(&b_hint);
        let expected_a = solve_with_descent_admission(
            &a_input,
            clone_initial(&a_initial),
            a_hint_ref,
            a_admission,
        )?;
        let expected_b = solve_with_descent_admission(
            &b_input,
            clone_initial(&b_initial),
            b_hint_ref,
            b_admission,
        )?;
        let actual = self.solve_pair_mode(
            device,
            queue,
            &a_input,
            a_initial,
            a_hint_ref,
            &b_input,
            b_initial,
            b_hint_ref,
            a_admission,
            b_admission,
            true,
        )?;
        compare_qualified(actual, &expected_a, &expected_b, use_disparity)
    }

    fn qualify_survivor_boundary(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Fallible<()> {
        let (a_input, b_input, a_initial, b_initial, a_hint, b_hint) = survivor_boundary_fixture();
        for (direction, current, hint) in [
            (
                "A-to-B",
                a_input.score(0, 0, Flow::new(1.0, 0.0).unwrap()),
                a_input.score(0, 0, Flow::ZERO),
            ),
            (
                "B-to-A",
                b_input.score(0, 0, Flow::new(1.0, 0.0).unwrap()),
                b_input.score(0, 0, Flow::ZERO),
            ),
        ] {
            if current.survivors() != 8 || hint.survivors() != 9 {
                return Err(format!(
                    "ONE X2 GPU PIS {direction} survivor qualifier is not discriminating: current has {}, hint has {}",
                    current.survivors(),
                    hint.survivors()
                )
                .into());
            }
        }
        let expected_a = solve_with_descent_admission(
            &a_input,
            clone_initial(&a_initial),
            Some(&a_hint),
            DescentAdmission::NoPatches,
        )?;
        let expected_b = solve_with_descent_admission(
            &b_input,
            clone_initial(&b_initial),
            Some(&b_hint),
            DescentAdmission::NoPatches,
        )?;
        let actual = self.solve_pair_mode(
            device,
            queue,
            &a_input,
            a_initial,
            Some(&a_hint),
            &b_input,
            b_initial,
            Some(&b_hint),
            DescentAdmission::NoPatches,
            DescentAdmission::NoPatches,
            true,
        )?;
        compare_qualified(actual, &expected_a, &expected_b, false)
    }

    fn qualify_tie_boundary(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Fallible<()> {
        let level = Level::Two;
        let (a_input, b_input) = constant_boundary_inputs(vec![1; level.pixels()]);
        let current = Flow::new(1.0, 0.0).unwrap();
        for (direction, input_score, hint_score) in [
            (
                "A-to-B",
                a_input.score(0, 0, current),
                a_input.score(0, 0, Flow::ZERO),
            ),
            (
                "B-to-A",
                b_input.score(0, 0, current),
                b_input.score(0, 0, Flow::ZERO),
            ),
        ] {
            if input_score.value().to_bits() != hint_score.value().to_bits() {
                return Err(format!(
                    "ONE X2 GPU PIS {direction} tie qualifier is not equal-score: current is {:#010x}, hint is {:#010x}",
                    input_score.value().to_bits(),
                    hint_score.value().to_bits()
                )
                .into());
            }
        }
        let initial_flows = vec![current; level.patches()];
        let hint_flows = vec![Flow::ZERO; level.patches()];
        let a_initial = initial_grid(level, &initial_flows);
        let b_initial = initial_grid(level, &initial_flows);
        let a_hint = HintGrid::from_row_major(level, hint_flows.clone()).unwrap();
        let b_hint = HintGrid::from_row_major(level, hint_flows).unwrap();
        let expected_a = solve_with_descent_admission(
            &a_input,
            clone_initial(&a_initial),
            Some(&a_hint),
            DescentAdmission::NoPatches,
        )?;
        let expected_b = solve_with_descent_admission(
            &b_input,
            clone_initial(&b_initial),
            Some(&b_hint),
            DescentAdmission::NoPatches,
        )?;
        let actual = self.solve_pair_mode(
            device,
            queue,
            &a_input,
            a_initial,
            Some(&a_hint),
            &b_input,
            b_initial,
            Some(&b_hint),
            DescentAdmission::NoPatches,
            DescentAdmission::NoPatches,
            true,
        )?;
        compare_qualified(actual, &expected_a, &expected_b, false)
    }

    fn qualify_zero_survivor_descent(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Fallible<()> {
        let level = Level::Two;
        let (a_input, b_input) = constant_boundary_inputs(vec![0; level.pixels()]);
        if a_input.score(0, 0, Flow::ZERO).survivors() != 0
            || b_input.score(0, 0, Flow::ZERO).survivors() != 0
        {
            return Err("ONE X2 GPU PIS zero-survivor descent qualifier has live taps".into());
        }
        let flows = vec![Flow::ZERO; level.patches()];
        let a_initial = initial_grid(level, &flows);
        let b_initial = initial_grid(level, &flows);
        let expected_a = solve_with_descent_admission(
            &a_input,
            clone_initial(&a_initial),
            None,
            DescentAdmission::EveryPatch,
        )?;
        let expected_b = solve_with_descent_admission(
            &b_input,
            clone_initial(&b_initial),
            None,
            DescentAdmission::EveryPatch,
        )?;
        let actual = self.solve_pair_mode(
            device,
            queue,
            &a_input,
            a_initial,
            None,
            &b_input,
            b_initial,
            None,
            DescentAdmission::EveryPatch,
            DescentAdmission::EveryPatch,
            true,
        )?;
        compare_qualified(actual, &expected_a, &expected_b, false)
    }
}

fn assert_descent_fixture_coverage() -> Fallible<()> {
    let (input, _, initial, _, hint, _) = qualification_fixture(Level::Two);
    let solved =
        solve_with_descent_admission(&input, initial, Some(&hint), DescentAdmission::EveryPatch)?;
    let (mut reaches_six, mut stops_after_write) = (false, false);
    for patch in solved.patches() {
        for pass in patch.passes() {
            reaches_six |= pass.descent_iterations() == 6;
            stops_after_write |= pass.stopped_on_no_improvement();
        }
    }
    if !reaches_six || !stops_after_write {
        return Err(format!(
            "ONE X2 GPU PIS descent qualifier is not discriminating: six steps={reaches_six}, write-before-stop={stops_after_write}"
        )
        .into());
    }
    Ok(())
}

fn expected_probes(use_disparity: bool) -> Vec<u32> {
    let mut expected = Vec::with_capacity(PROBE_WORDS);
    expected.push(0);
    expected.extend(DIVISION_PROBES.iter().map(|probe| probe.2));
    expected.extend(DISTANCE_PROBES.iter().map(|probe| probe.2));
    // Interior, then each of the four strict endpoints.
    expected.extend(if use_disparity {
        [0, 1, 1, 1, 1]
    } else {
        [0; DISPARITY_PROBES]
    });
    expected
}

fn compare_probes<D: PisDirection>(
    direction: &'static str,
    actual: &TerminalBits<D>,
    use_disparity: bool,
) -> Fallible<()> {
    for (probe, (actual, expected)) in actual
        .diagnostics
        .iter()
        .copied()
        .zip(expected_probes(use_disparity))
        .enumerate()
    {
        if actual != expected {
            return Err(QualificationError::Probe {
                direction,
                probe,
                actual,
                expected,
            }
            .into());
        }
    }
    Ok(())
}

fn compare_qualified(
    actual: PairedTerminalBits,
    expected_a: &super::PatchGrid<AtoB>,
    expected_b: &super::PatchGrid<BtoA>,
    use_disparity: bool,
) -> Fallible<()> {
    compare_probes("A-to-B", &actual.a_to_b, use_disparity)?;
    compare_probes("B-to-A", &actual.b_to_a, use_disparity)?;
    let actual = actual.into_patch_grids()?;
    compare("A-to-B", &actual.a_to_b, expected_a)?;
    compare("B-to-A", &actual.b_to_a, expected_b)
}

fn compare<D: PisDirection>(
    direction: &'static str,
    actual: &super::PatchGrid<D>,
    expected: &super::PatchGrid<D>,
) -> Fallible<()> {
    let expected_candidates = expected
        .patches()
        .get(1)
        .map(|patch| {
            patch.passes()[0]
                .candidates()
                .into_iter()
                .flatten()
                .map(|candidate| candidate.score().value().to_bits())
                .collect::<Vec<_>>()
                .into_boxed_slice()
        })
        .unwrap_or_default();
    for (patch, (actual_bits, expected_bits)) in actual
        .patches()
        .iter()
        .map(|patch| patch.flow().dcol().to_bits())
        .zip(
            expected
                .patches()
                .iter()
                .map(|patch| patch.flow().dcol().to_bits()),
        )
        .enumerate()
    {
        if actual_bits != expected_bits {
            return Err(QualificationError::Direction {
                direction,
                level: actual.level(),
                patch,
                component: "dcol",
                actual: actual_bits,
                expected: expected_bits,
                expected_candidates: expected_candidates.clone(),
            }
            .into());
        }
    }
    for (patch, (actual_bits, expected_bits)) in actual
        .patches()
        .iter()
        .map(|patch| patch.flow().drow().to_bits())
        .zip(
            expected
                .patches()
                .iter()
                .map(|patch| patch.flow().drow().to_bits()),
        )
        .enumerate()
    {
        if actual_bits != expected_bits {
            return Err(QualificationError::Direction {
                direction,
                level: actual.level(),
                patch,
                component: "drow",
                actual: actual_bits,
                expected: expected_bits,
                expected_candidates: expected_candidates.clone(),
            }
            .into());
        }
    }
    Ok(())
}

fn clone_initial<D: PisDirection>(initial: &InitialGrid<D>) -> InitialGrid<D> {
    InitialGrid {
        level: initial.level,
        flows: initial.flows.clone(),
        direction: PhantomData,
    }
}

struct PackedInput<D: PisDirection> {
    level: Level,
    u32s: Vec<u32>,
    f32s: Vec<f32>,
    qualification_probes: bool,
    direction: PhantomData<D>,
}

const PAIR_HEADER_WORDS: usize = 8;
const PAIR_SCHEMA: u32 = 0x5049_5301;

struct PackedPair {
    level: Level,
    u32s: Vec<u32>,
    f32s: Vec<f32>,
    output_words: usize,
    output_span: usize,
    diagnostic_words: usize,
    a_output_base: usize,
    b_output_base: usize,
}

impl PackedPair {
    fn new(a: PackedInput<AtoB>, b: PackedInput<BtoA>) -> Fallible<Self> {
        debug_assert_eq!(a.level, b.level);
        debug_assert_eq!(a.qualification_probes, b.qualification_probes);
        let a_word_base = PAIR_HEADER_WORDS;
        let b_word_base = a_word_base
            .checked_add(a.u32s.len())
            .ok_or("ONE X2 paired GPU PIS u32 input is too large")?;
        let b_float_base = a.f32s.len();
        let diagnostic_words = if a.qualification_probes {
            PROBE_WORDS
        } else {
            0
        };
        let output_span = a.level.patches() * OUTPUT_WORDS_PER_PATCH + diagnostic_words;
        let b_output_base = output_span;
        let output_words = output_span
            .checked_mul(2)
            .ok_or("ONE X2 paired GPU PIS output is too large")?;
        let as_u32 = |value: usize, part: &'static str| -> Fallible<u32> {
            u32::try_from(value)
                .map_err(move |_| format!("ONE X2 paired GPU PIS {part} exceeds u32").into())
        };
        let mut u32s = vec![0; PAIR_HEADER_WORDS];
        u32s[0] = PAIR_SCHEMA;
        u32s[1] = 2;
        u32s[2] = as_u32(a_word_base, "A word base")?;
        u32s[3] = 0;
        u32s[4] = 0;
        u32s[5] = as_u32(b_word_base, "B word base")?;
        u32s[6] = as_u32(b_float_base, "B float base")?;
        u32s[7] = as_u32(b_output_base, "B output base")?;
        u32s.extend(a.u32s);
        u32s.extend(b.u32s);
        let mut f32s = a.f32s;
        f32s.extend(b.f32s);
        Ok(Self {
            level: a.level,
            u32s,
            f32s,
            output_words,
            output_span,
            diagnostic_words,
            a_output_base: 0,
            b_output_base,
        })
    }
}

fn decode_terminal<D: PisDirection>(
    level: Level,
    words: &[u32],
    diagnostic_words: usize,
) -> Result<TerminalBits<D>, TerminalGridError> {
    let expected_words = level.patches() * OUTPUT_WORDS_PER_PATCH + diagnostic_words;
    if words.len() != expected_words {
        return Err(TerminalGridError::Span {
            direction: D::DIRECTION,
            level,
            expected_words,
            actual_words: words.len(),
        });
    }
    let mut dcol = Vec::with_capacity(level.patches());
    let mut drow = Vec::with_capacity(level.patches());
    for pair in words[..level.patches() * 2].chunks_exact(2) {
        dcol.push(pair[0]);
        drow.push(pair[1]);
    }
    Ok(TerminalBits {
        level,
        dcol: dcol.into_boxed_slice(),
        drow: drow.into_boxed_slice(),
        diagnostics: words[level.patches() * 2..].into(),
        direction: PhantomData,
    })
}

impl<D: PisDirection> PackedInput<D> {
    fn new_prepared(
        level: Level,
        dynamic: GpuPisDynamicDirection<D>,
        qualification_probes: bool,
    ) -> Fallible<Self> {
        let GpuPisDynamicDirection {
            cost_modes,
            initial,
            hint,
            admission,
            disparity,
        } = dynamic;
        if cost_modes.len() != level.patch_rows() {
            return Err(format!(
                "ONE X2 direct GPU PIS {} {level} has {} cost modes, expected {}",
                D::DIRECTION,
                cost_modes.len(),
                level.patch_rows()
            )
            .into());
        }
        if initial.level != level {
            return Err(format!(
                "ONE X2 direct GPU PIS {} initial grid is {}, expected {level}",
                D::DIRECTION,
                initial.level
            )
            .into());
        }
        if let Some(hint) = &hint
            && hint.level != level
        {
            return Err(format!(
                "ONE X2 direct GPU PIS {} hint grid is {}, expected {level}",
                D::DIRECTION,
                hint.level
            )
            .into());
        }

        let mut u32s = vec![0; HEADER_WORDS];
        u32s[0] = level.rows() as u32;
        u32s[1] = level.cols() as u32;
        u32s[2] = level.patch_rows() as u32;
        u32s[3] = level.patch_cols() as u32;
        u32s[4] = level.pixels() as u32;
        u32s[5] = level.patches() as u32;
        u32s[6] = u32::from(hint.is_some());
        u32s[7] = u32::from(disparity.is_some());
        u32s[8] = u32::from(admission.admits());
        u32s[13] = u32s.len() as u32;
        u32s.extend(cost_modes.iter().map(|mode| match mode {
            CostMode::Unweighted => 0,
            CostMode::Weighted => 1,
        }));
        u32s[22] = u32s.len() as u32;
        u32s.push(0);
        u32s[29] = u32::from(qualification_probes);
        u32s[30] = 1;

        let mut f32s = Vec::with_capacity(4 * level.patches() + 4);
        u32s[18] = f32s.len() as u32;
        for flow in &initial.flows {
            f32s.extend([flow.dcol(), flow.drow()]);
        }
        u32s[19] = f32s.len() as u32;
        if let Some(hint) = &hint {
            for flow in &hint.flows {
                f32s.extend([flow.dcol(), flow.drow()]);
            }
        } else {
            f32s.extend(std::iter::repeat_n(0.0, 2 * level.patches()));
        }
        u32s[20] = f32s.len() as u32;
        if let Some(disparity) = disparity {
            f32s.extend([
                disparity.first[0],
                disparity.first[1],
                disparity.second[0],
                disparity.second[1],
            ]);
        } else {
            f32s.extend([0.0; 4]);
        }
        u32s[23] = u32s.len() as u32;
        u32s[25] = f32s.len() as u32;
        u32s[27] = f32s.len() as u32;
        Ok(Self {
            level,
            u32s,
            f32s,
            qualification_probes,
            direction: PhantomData,
        })
    }

    fn new(
        input: &Input<D>,
        initial: InitialGrid<D>,
        hint: Option<&HintGrid<D>>,
        admission: DescentAdmission,
        qualification_probes: bool,
    ) -> Fallible<Self> {
        if initial.level != input.level {
            return Err(format!(
                "ONE X2 GPU PIS initial grid is {}, expected {}",
                initial.level, input.level
            )
            .into());
        }
        if let Some(hint) = hint
            && hint.level != input.level
        {
            return Err(format!(
                "ONE X2 GPU PIS hint grid is {}, expected {}",
                hint.level, input.level
            )
            .into());
        }

        let mut u32s = vec![0; HEADER_WORDS];
        u32s[0] = input.level.rows() as u32;
        u32s[1] = input.level.cols() as u32;
        u32s[2] = input.level.patch_rows() as u32;
        u32s[3] = input.level.patch_cols() as u32;
        u32s[4] = input.level.pixels() as u32;
        u32s[5] = input.level.patches() as u32;
        u32s[6] = u32::from(hint.is_some());
        u32s[7] = u32::from(input.disparity.is_some());
        u32s[8] = u32::from(admission.admits());
        append_u8_plane(&mut u32s, 9, &input.source);
        append_u8_plane(&mut u32s, 10, &input.target);
        append_u8_plane(&mut u32s, 11, &input.source_slot_mask);
        append_u8_plane(&mut u32s, 12, &input.target_slot_mask);
        u32s[13] = u32s.len() as u32;
        u32s.extend(input.cost_modes.iter().map(|mode| match mode {
            CostMode::Unweighted => 0,
            CostMode::Weighted => 1,
        }));
        u32s[22] = u32s.len() as u32;
        u32s.push(0);
        u32s[29] = u32::from(qualification_probes);
        u32s[23] = u32s.len() as u32;
        if qualification_probes {
            u32s[24] = DIVISION_PROBES.len() as u32;
            for &(numerator, denominator, _) in DIVISION_PROBES {
                u32s.extend([numerator, denominator]);
            }
        }

        let mut f32s = Vec::new();
        append_f32_plane(&mut f32s, &mut u32s, 14, &input.gradient_col);
        append_f32_plane(&mut f32s, &mut u32s, 15, &input.gradient_row);
        append_f32_plane(&mut f32s, &mut u32s, 16, &input.raw_weight);
        append_f32_plane(&mut f32s, &mut u32s, 17, &input.patch_weight_sums);
        u32s[18] = f32s.len() as u32;
        for flow in &initial.flows {
            f32s.extend([flow.dcol(), flow.drow()]);
        }
        u32s[19] = f32s.len() as u32;
        if let Some(hint) = hint {
            for flow in &hint.flows {
                f32s.extend([flow.dcol(), flow.drow()]);
            }
        } else {
            f32s.extend(std::iter::repeat_n(0.0, 2 * input.level.patches()));
        }
        u32s[20] = f32s.len() as u32;
        if let Some(disparity) = input.disparity {
            f32s.extend([
                disparity.first[0],
                disparity.first[1],
                disparity.second[0],
                disparity.second[1],
            ]);
        } else {
            f32s.extend([0.0; 4]);
        }
        u32s[21] = f32s.len() as u32;
        for patch in 0..input.level.patches() {
            let row = patch / input.level.patch_cols();
            let col = patch % input.level.patch_cols();
            let model = input.prepared_source_terms(row, col).inverted();
            f32s.extend([
                model.gradient_col_sum,
                model.gradient_row_sum,
                model.inverse_col_col,
                model.inverse_col_row,
                model.inverse_row_row,
            ]);
        }
        u32s[25] = f32s.len() as u32;
        if qualification_probes {
            u32s[26] = DISTANCE_PROBES.len() as u32;
            for &(dcol, drow, _) in DISTANCE_PROBES {
                f32s.extend([f32::from_bits(dcol), f32::from_bits(drow), 0.0, 0.0]);
            }
        }
        u32s[27] = f32s.len() as u32;
        if qualification_probes {
            u32s[28] = DISPARITY_PROBES as u32;
            let interval = input
                .disparity
                .unwrap_or(super::DisparityInterval::new([-2.0, -1.0], [4.0, 1.0]));
            let midpoint = [
                (interval.first[0] + interval.second[0]) * 0.5,
                (interval.first[1] + interval.second[1]) * 0.5,
            ];
            f32s.extend([
                midpoint[0],
                midpoint[1],
                interval.first[0],
                midpoint[1],
                interval.second[0],
                midpoint[1],
                midpoint[0],
                interval.first[1],
                midpoint[0],
                interval.second[1],
            ]);
        }
        Ok(Self {
            level: input.level,
            u32s,
            f32s,
            qualification_probes,
            direction: PhantomData,
        })
    }
}

fn append_u8_plane(target: &mut Vec<u32>, header: usize, values: &[u8]) {
    target[header] = target.len() as u32;
    target.extend(values.iter().copied().map(u32::from));
}

fn append_f32_plane(
    target: &mut Vec<f32>,
    header_words: &mut [u32],
    header: usize,
    values: &[f32],
) {
    header_words[header] = target.len() as u32;
    target.extend_from_slice(values);
}

fn u32_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_ne_bytes()).collect()
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_bits().to_ne_bytes())
        .collect()
}

fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    bytes: Vec<u8>,
) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, &bytes);
    buffer
}

#[allow(clippy::type_complexity)]
fn qualification_fixture(
    level: Level,
) -> (
    Input<AtoB>,
    Input<BtoA>,
    InitialGrid<AtoB>,
    InitialGrid<BtoA>,
    HintGrid<AtoB>,
    HintGrid<BtoA>,
) {
    let pixels = level.pixels();
    let image_a = (0..pixels)
        .map(|at| ((at * 29 + at / level.cols() * 17 + 3) & 255) as u8)
        .collect::<Vec<_>>();
    let image_b = (0..pixels)
        .map(|at| ((at * 11 + at / level.cols() * 43 + 197) & 255) as u8)
        .collect::<Vec<_>>();
    let mask_a = (0..pixels)
        .map(|at| u8::from(!at.is_multiple_of(17) && at % level.cols() != 0))
        .collect::<Vec<_>>();
    let mask_b = (0..pixels)
        .map(|at| u8::from(!at.is_multiple_of(19) && at % level.cols() + 1 != level.cols()))
        .collect::<Vec<_>>();
    let prepared = |salt: u32| {
        let mut gx = Vec::with_capacity(pixels);
        let mut gy = Vec::with_capacity(pixels);
        let mut weight = Vec::with_capacity(pixels);
        for (at, mask) in mask_a.iter().copied().enumerate() {
            if mask == 0 {
                gx.push(0.0);
                gy.push(0.0);
                weight.push(0.0);
            } else {
                let bits = (at as u32).wrapping_mul(0x9e37_79b9).rotate_left(salt);
                gx.push(((bits & 255) as f32 - 127.0) / 31.0);
                gy.push((((bits >> 8) & 255) as f32 - 127.0) / 29.0);
                weight.push((((bits >> 16) & 127) + 1) as f32 / 23.0);
            }
        }
        (gx, gy, weight)
    };
    let modes = (0..level.patch_rows())
        .map(|row| {
            if row.is_multiple_of(3) {
                CostMode::Unweighted
            } else {
                CostMode::Weighted
            }
        })
        .collect::<Vec<_>>();
    let make_input = |reverse: bool| {
        let (gx, gy, weight) = prepared(if reverse { 11 } else { 5 });
        let images = super::super::LensPair {
            a: image_a.clone(),
            b: image_b.clone(),
        };
        let masks = super::super::LensPair {
            a: mask_a.clone(),
            b: mask_b.clone(),
        };
        (images, masks, gx, gy, weight)
    };
    let (images, masks, gx, gy, weight) = make_input(false);
    let a = Input::<AtoB>::from_native_order(level, images, masks, gx, gy, weight, modes.clone())
        .unwrap()
        .with_disparity_interval(super::DisparityInterval::new([-7.75, -7.5], [7.75, 7.5]));
    let (images, masks, gx, gy, weight) = make_input(true);
    let b = Input::<BtoA>::from_native_order(level, images, masks, gx, gy, weight, modes)
        .unwrap()
        .with_disparity_interval(super::DisparityInterval::new([7.6, 7.8], [-7.6, -7.8]));
    let seeds = |salt: usize| {
        (0..level.patches())
            .map(|patch| {
                Flow::new(
                    ((patch * 7 + salt) % 17) as f32 / 8.0 - 1.0,
                    ((patch * 11 + salt) % 19) as f32 / 9.0 - 1.0,
                )
                .unwrap()
            })
            .collect::<Vec<_>>()
    };
    (
        a,
        b,
        InitialGrid {
            level,
            flows: seeds(1).into_boxed_slice(),
            direction: PhantomData,
        },
        InitialGrid {
            level,
            flows: seeds(2).into_boxed_slice(),
            direction: PhantomData,
        },
        HintGrid::from_row_major(level, seeds(3)).unwrap(),
        HintGrid::from_row_major(level, seeds(4)).unwrap(),
    )
}

#[allow(clippy::type_complexity)]
fn survivor_boundary_fixture() -> (
    Input<AtoB>,
    Input<BtoA>,
    InitialGrid<AtoB>,
    InitialGrid<BtoA>,
    HintGrid<AtoB>,
    HintGrid<BtoA>,
) {
    let level = Level::Two;
    let pixels = level.pixels();
    let mut target_mask = vec![0; pixels];
    target_mask[..8].fill(1);
    target_mask[level.cols()] = 1;
    target_mask[2 * level.cols() + 8] = 1;
    let (a, b) = constant_boundary_inputs(target_mask);
    let initial = vec![Flow::new(1.0, 0.0).unwrap(); level.patches()];
    let mut hints = initial.clone();
    hints[0] = Flow::ZERO;
    (
        a,
        b,
        initial_grid(level, &initial),
        initial_grid(level, &initial),
        HintGrid::from_row_major(level, hints.clone()).unwrap(),
        HintGrid::from_row_major(level, hints).unwrap(),
    )
}

fn constant_boundary_inputs(target_mask: Vec<u8>) -> (Input<AtoB>, Input<BtoA>) {
    let level = Level::Two;
    let pixels = level.pixels();
    let images = super::super::LensPair {
        a: vec![10; pixels],
        b: vec![20; pixels],
    };
    let masks = super::super::LensPair {
        a: vec![1; pixels],
        b: target_mask,
    };
    let gradients = vec![0.0; pixels];
    let modes = vec![CostMode::Unweighted; level.patch_rows()];
    (
        Input::<AtoB>::from_native_order(
            level,
            images.clone(),
            masks.clone(),
            gradients.clone(),
            gradients.clone(),
            gradients.clone(),
            modes.clone(),
        )
        .unwrap(),
        Input::<BtoA>::from_native_order(
            level,
            images,
            masks,
            gradients.clone(),
            gradients.clone(),
            gradients,
            modes,
        )
        .unwrap(),
    )
}

fn initial_grid<D: PisDirection>(level: Level, flows: &[Flow]) -> InitialGrid<D> {
    InitialGrid {
        level,
        flows: flows.into(),
        direction: PhantomData,
    }
}

const SHADER: &str = include_str!("pis.wgsl");

#[cfg(test)]
pub(crate) const DIRECT_TEST_SHADER: &str = SHADER;

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::*;

    #[test]
    fn terminal_decoder_refuses_malformed_span() {
        let error = decode_terminal::<AtoB>(Level::Two, &[0; 3], 0).unwrap_err();
        assert_eq!(
            error,
            TerminalGridError::Span {
                direction: Direction::AtoB,
                level: Level::Two,
                expected_words: Level::Two.patches() * 2,
                actual_words: 3,
            }
        );
        assert!(error.to_string().contains("3 words"));
    }

    #[test]
    fn terminal_decoder_refuses_each_nonfinite_component() {
        for (component_index, component) in [(0, "dcol"), (1, "drow")] {
            for bits in [
                f32::NAN.to_bits(),
                f32::INFINITY.to_bits(),
                f32::NEG_INFINITY.to_bits(),
            ] {
                let mut words = vec![0; Level::Two.patches() * 2];
                words[component_index] = bits;
                let error = decode_terminal::<BtoA>(Level::Two, &words, 0)
                    .unwrap()
                    .into_patch_grid()
                    .unwrap_err();
                assert_eq!(
                    error,
                    TerminalGridError::NonFinite {
                        direction: Direction::BtoA,
                        level: Level::Two,
                        patch: 0,
                        component,
                        bits,
                    }
                );
            }
        }
    }

    #[test]
    fn terminal_decoder_preserves_signed_zero_and_subnormal_bits() {
        let mut words = vec![0; Level::Two.patches() * 2];
        words[0] = (-0.0_f32).to_bits();
        words[1] = 1;
        words[2] = 0x8000_0001;
        words[3] = 0.0_f32.to_bits();
        let grid = decode_terminal::<AtoB>(Level::Two, &words, 0)
            .unwrap()
            .into_patch_grid()
            .unwrap();
        assert_eq!(grid.patches()[0].flow().dcol().to_bits(), 0x8000_0000);
        assert_eq!(grid.patches()[0].flow().drow().to_bits(), 1);
        assert_eq!(grid.patches()[1].flow().dcol().to_bits(), 0x8000_0001);
        assert_eq!(grid.patches()[1].flow().drow().to_bits(), 0);
    }

    #[test]
    fn paired_gpu_matches_cpu_terminal_bits() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping exact paired GPU PIS twin: {why}");
                return;
            }
        };
        GpuPisPipeline::new(OneXsGpuContext::new(&device, &queue)).unwrap_or_else(|error| {
            panic!("paired GPU PIS qualification failed on {adapter}: {error}")
        });
    }

    #[test]
    fn qualification_refuses_production_entry_mutations() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping paired GPU PIS mutation refusal: {why}");
                return;
            }
        };
        let mutations = [
            (
                "candidate tie order",
                "if candidate == 0u || score < selected_scores[slot]",
                "if candidate == 0u || score <= selected_scores[slot]",
            ),
            (
                "eight/nine survivor boundary",
                "survivors >= 9u",
                "survivors >= 8u",
            ),
            (
                "forward scan propagation",
                "candidate_flow = stored_flow(cell - 1u);",
                "candidate_flow = stored_flow(cell);",
            ),
            (
                "hint absence",
                "present = word(6u) != 0u;",
                "present = true;",
            ),
            ("descent admission", "if word(8u) != 0u {", "if true {"),
            (
                "zero-survivor descent",
                "return DescentStep(vec2<f32>(0.0), SENTINEL);",
                "return DescentStep(vec2<f32>(1.0, 0.0), SENTINEL);",
            ),
            ("six-step descent limit", "descent < 6u", "descent < 5u"),
            (
                "write before stop",
                "if step.residual >= previous { break; }",
                "if step.residual >= previous { current = seed; break; }",
            ),
            (
                "runtime-zero materialization",
                "^ local_word(word(22u))",
                "^ 1u",
            ),
            (
                "disparity absence",
                "if word(7u) == 0u { return false; }",
                "if false { return false; }",
            ),
            (
                "strict disparity endpoints",
                "flow.x > first.x && flow.x < second.x",
                "flow.x >= first.x && flow.x < second.x",
            ),
            (
                "canonical signed-zero zero divide",
                "return 0xffc00000u;",
                "return 0x7fc00000u;",
            ),
            (
                "terminal distance discriminator",
                "threshold[7] = 1u << 28u;",
                "threshold[7] = 0u;",
            ),
        ];
        for (name, from, to) in mutations {
            let broken = SHADER.replacen(from, to, 1);
            assert_ne!(broken, SHADER, "{name} mutation found no target");
            let error = match GpuPisPipeline::from_shader(&device, &queue, &broken, true) {
                Ok(_) => panic!("changed paired GPU PIS {name} was accepted on {adapter}"),
                Err(error) => error,
            };
            assert!(
                error.downcast_ref::<QualificationError>().is_some(),
                "{name} mutation returned the wrong refusal on {adapter}: {error}"
            );
        }
    }

    #[test]
    fn terminal_decoder_refuses_nonfinite_bits() {
        let level = Level::Two;
        let mut words = vec![0; level.patches() * OUTPUT_WORDS_PER_PATCH];
        words[2 * 7] = f32::INFINITY.to_bits();
        let error = decode_terminal::<AtoB>(level, &words, 0)
            .unwrap()
            .into_patch_grid()
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ONE X2 GPU PIS AtoB level 2 patch 7 terminal dcol bits 0x7f800000 are not finite"
        );
    }

    #[test]
    fn ordinary_mode_skips_qualification_probes_without_changing_terminals() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping paired GPU PIS ordinary-mode probe gate: {why}");
                return;
            }
        };
        let mutated = SHADER.replacen(
            "terminal_bits[base] = local_word(word(22u));",
            "terminal_bits[base] = 0xdeadbeefu;",
            1,
        );
        assert_ne!(mutated, SHADER, "probe-write mutation found no target");
        let pipeline = GpuPisPipeline::from_shader(&device, &queue, &mutated, false)
            .unwrap_or_else(|error| panic!("ordinary paired GPU PIS failed on {adapter}: {error}"));
        let (a_input, b_input, a_initial, b_initial, a_hint, b_hint) =
            qualification_fixture(Level::Two);
        let expected_a = solve_with_descent_admission(
            &a_input,
            clone_initial(&a_initial),
            Some(&a_hint),
            DescentAdmission::NoPatches,
        )
        .unwrap();
        let expected_b = solve_with_descent_admission(
            &b_input,
            clone_initial(&b_initial),
            Some(&b_hint),
            DescentAdmission::NoPatches,
        )
        .unwrap();
        let actual = pipeline
            .solve_pair_mode(
                &device,
                &queue,
                &a_input,
                a_initial,
                Some(&a_hint),
                &b_input,
                b_initial,
                Some(&b_hint),
                DescentAdmission::NoPatches,
                DescentAdmission::NoPatches,
                false,
            )
            .unwrap();
        assert!(actual.a_to_b.diagnostics.iter().all(|word| *word == 0));
        assert!(actual.b_to_a.diagnostics.iter().all(|word| *word == 0));
        let actual = actual.into_patch_grids().unwrap();
        compare("A-to-B", &actual.a_to_b, &expected_a).unwrap();
        compare("B-to-A", &actual.b_to_a, &expected_b).unwrap();
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

    fn gpu() -> Result<(wgpu::Device, wgpu::Queue, String), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
            .ok_or("no Vulkan adapter")?;
        let name = adapter.get_info().name;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("exact paired ONE X2 GPU PIS"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        Ok((device, queue, name))
    }
}
