//! GPU-resident continuation from a sealed paired level-two PIS terminal.
//!
//! The ordinary boundary consumes the terminal and advances its prepared
//! frame's one submission lease. The terminal, source images, output planes
//! and validity word remain resident. CPU uploads and readbacks exist only in
//! this module's qualification helpers.

#![allow(dead_code)] // This checkpoint is intentionally not selected or Scene-wired.

use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::sync::mpsc;

use super::{
    GpuLevelOne, GpuLevelTwo, GpuPreparedFrame, GpuPreparedTerminal, GpuResidentValidity,
    SealedPreparedPair, pis_direction_header_base, write_pis_bases,
};
use crate::Fallible;
use crate::flow::one_xs::dense::PublicDenseField;
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
use crate::flow::one_xs::one_xs_belt_gpu::geometry_gpu::temporal_gpu::GpuAdmittedSnapshot;
#[cfg(test)]
use crate::flow::one_xs::pis::Flow;
use crate::flow::one_xs::pis::gpu::{GpuPisPipeline, GpuPisStageReceipt, ResidentPisDispatch};
use crate::flow::one_xs::pis::{AtoB, BtoA, CostMode, DisparityInterval, Level, PisDirection};
use crate::flow::one_xs::post_update::RetainedPublicPyramids;
use crate::flow::one_xs::scalar::PairSolveStage;

#[path = "post_l1.rs"]
mod post_l1;
pub(in crate::flow::one_xs::one_xs_belt_gpu) use post_l1::GpuCold0Terminal;

const L2_ROWS: usize = 270;
const L2_COLS: usize = 15;
const L2_PIXELS: usize = L2_ROWS * L2_COLS;
const L2_PATCHES: usize = 88 * 3;
const L1_PATCHES: usize = 178 * 8;
const PLANES: usize = 4;

/// Exact PIS producer identity accepted by the bridge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GpuL2BridgeReceipt(GpuPisStageReceipt);

impl GpuL2BridgeReceipt {
    pub(crate) fn from_terminal(receipt: GpuPisStageReceipt) -> Result<Self, BridgeInputError> {
        if receipt.stage.level() != Level::Two {
            return Err(BridgeInputError::WrongReceiptLevel(receipt.stage.level()));
        }
        Ok(Self(receipt))
    }
}

/// CPU-uploaded terminal component planes used only by qualification.
#[derive(Clone, Debug)]
pub(crate) struct CpuUploadedTerminalL2<D: PisDirection> {
    dcol: Box<[f32]>,
    drow: Box<[f32]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> CpuUploadedTerminalL2<D> {
    pub(crate) fn new(dcol: Vec<f32>, drow: Vec<f32>) -> Result<Self, BridgeInputError> {
        check_len("terminal dcol", dcol.len(), L2_PATCHES)?;
        check_len("terminal drow", drow.len(), L2_PATCHES)?;
        if dcol.iter().chain(&drow).any(|value| !value.is_finite()) {
            return Err(BridgeInputError::NonFiniteTerminal);
        }
        Ok(Self {
            dcol: dcol.into_boxed_slice(),
            drow: drow.into_boxed_slice(),
            direction: PhantomData,
        })
    }
}

/// Physical A/B level-two luma planes used only by qualification.
#[derive(Clone)]
pub(crate) struct LevelTwoImages {
    a: Box<[u8]>,
    b: Box<[u8]>,
}

impl LevelTwoImages {
    pub(crate) fn new(a: Vec<u8>, b: Vec<u8>) -> Result<Self, BridgeInputError> {
        check_len("image A", a.len(), L2_PIXELS)?;
        check_len("image B", b.len(), L2_PIXELS)?;
        Ok(Self {
            a: a.into_boxed_slice(),
            b: b.into_boxed_slice(),
        })
    }
}

/// Direction-owned retained field at the exact level consumed by this stage.
#[derive(Clone)]
pub(crate) struct RetainedLevelTwo<D: PisDirection> {
    dcol: Box<[f32]>,
    drow: Box<[f32]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> RetainedLevelTwo<D> {
    pub(crate) fn new(dcol: Vec<f32>, drow: Vec<f32>) -> Result<Self, BridgeInputError> {
        check_len("retained dcol", dcol.len(), L2_PIXELS)?;
        check_len("retained drow", drow.len(), L2_PIXELS)?;
        Ok(Self {
            dcol: dcol.into_boxed_slice(),
            drow: drow.into_boxed_slice(),
            direction: PhantomData,
        })
    }
}

/// Immutable cold/warm inputs. Motion is the selected shared level-two mask.
pub(crate) enum LevelTwoPostUpdate {
    Cold,
    Warm {
        a_to_b: RetainedLevelTwo<AtoB>,
        b_to_a: RetainedLevelTwo<BtoA>,
        motion: Box<[u8]>,
    },
}

/// Sealed resident post-update input. Implementations create the complete bind
/// group themselves, so neither retained state nor motion buffers can detach
/// from their owner at this boundary.
pub(in crate::flow::one_xs::one_xs_belt_gpu) trait GpuResidentLevelTwoPost:
    super::resident_l2_post_seal::Sealed + Sized
{
    fn resident_identity(
        &self,
    ) -> Fallible<
        Option<crate::flow::one_xs::one_xs_belt_gpu::resident_frame_gpu::GpuResidentIdentity>,
    > {
        Ok(None)
    }
    fn context(&self) -> &OneXsGpuContext;
    fn is_warm(&self) -> bool;
    fn admissions(&self) -> GpuAdmittedSnapshot;
    fn initialize(&self, encoder: &mut wgpu::CommandEncoder);
    #[allow(clippy::too_many_arguments)]
    fn bind(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        config: &wgpu::Buffer,
        images: &wgpu::Buffer,
        terminal: &wgpu::Buffer,
        output: &wgpu::Buffer,
        validity: &wgpu::Buffer,
    ) -> wgpu::BindGroup;
    fn bind_l1_hint_fill(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        config: &wgpu::Buffer,
        dynamic: &wgpu::Buffer,
    ) -> Option<wgpu::BindGroup>;

    fn bind_l2_hint_fill(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        config: &wgpu::Buffer,
        dynamic: &wgpu::Buffer,
    ) -> Option<wgpu::BindGroup> {
        self.bind_l1_hint_fill(device, layout, config, dynamic)
    }
}

/// Cold frames still bind real resident allocations. Their initialization is
/// encoded as GPU clears in the same submission, never as host zero uploads.
struct ColdResidentLevelTwoPost {
    context: OneXsGpuContext,
    retained: wgpu::Buffer,
    motion: wgpu::Buffer,
}

/// Constructor-only uploaded oracle input. It is never accepted by the
/// ordinary consuming transition.
struct QualificationResidentLevelTwoPost {
    context: OneXsGpuContext,
    warm: bool,
    retained: wgpu::Buffer,
    motion: wgpu::Buffer,
    hints: Option<wgpu::Buffer>,
}

/// Qualification-only adapter that binds the exact packed allocation emitted
/// by the production temporal kernel. It cannot escape a test build.
#[cfg(test)]
struct BorrowedProductionMotionPost<'a> {
    context: OneXsGpuContext,
    retained: wgpu::Buffer,
    motion: &'a wgpu::Buffer,
}

impl super::resident_l2_post_seal::Sealed for ColdResidentLevelTwoPost {}
impl super::resident_l2_post_seal::Sealed for QualificationResidentLevelTwoPost {}
#[cfg(test)]
impl super::resident_l2_post_seal::Sealed for BorrowedProductionMotionPost<'_> {}

/// L1 controls deliberately omit the initial grid. The only initial accepted
/// by this transition is the resident seed allocation produced by L2.
#[derive(Clone)]
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuL1DirectionControls<D: PisDirection> {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) cost_modes: Box<[CostMode]>,
    pub(in crate::flow::one_xs::one_xs_belt_gpu) disparity: DisparityInterval,
    direction: PhantomData<D>,
}

#[derive(Clone)]
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuL1Controls {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) a_to_b: GpuL1DirectionControls<AtoB>,
    pub(in crate::flow::one_xs::one_xs_belt_gpu) b_to_a: GpuL1DirectionControls<BtoA>,
}

/// Direction controls whose exact L2 stage is derived by the temporal owner.
#[derive(Clone)]
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuL2Controls {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) a_to_b: GpuL1DirectionControls<AtoB>,
    pub(in crate::flow::one_xs::one_xs_belt_gpu) b_to_a: GpuL1DirectionControls<BtoA>,
}

#[derive(Clone)]
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuColdLoopControls {
    l2: GpuL2Controls,
    l1: GpuL1Controls,
}

impl GpuColdLoopControls {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn new(
        l2: GpuL2Controls,
        l1: GpuL1Controls,
    ) -> Self {
        Self { l2, l1 }
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn l2(&self) -> GpuL2Controls {
        self.l2.clone()
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn l1(&self) -> GpuL1Controls {
        self.l1.clone()
    }
}

impl GpuL2Controls {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn resident(
        a_cost_modes: Box<[CostMode]>,
        a_disparity: DisparityInterval,
        b_cost_modes: Box<[CostMode]>,
        b_disparity: DisparityInterval,
    ) -> Self {
        Self {
            a_to_b: GpuL1DirectionControls {
                cost_modes: a_cost_modes,
                disparity: a_disparity,
                direction: PhantomData,
            },
            b_to_a: GpuL1DirectionControls {
                cost_modes: b_cost_modes,
                disparity: b_disparity,
                direction: PhantomData,
            },
        }
    }

    fn into_dispatch<'a>(
        self,
        solver: &'a GpuPisPipeline,
        stage: PairSolveStage,
        admissions: GpuAdmittedSnapshot,
    ) -> Fallible<ResidentPisDispatch<'a>> {
        solver.prepare_resident_grid_dispatch(
            stage,
            self.a_to_b.cost_modes,
            admissions.a_to_b(),
            Some(self.a_to_b.disparity),
            self.b_to_a.cost_modes,
            admissions.b_to_a(),
            Some(self.b_to_a.disparity),
        )
    }
}

impl GpuL1Controls {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn resident(
        a_cost_modes: Box<[CostMode]>,
        a_disparity: DisparityInterval,
        b_cost_modes: Box<[CostMode]>,
        b_disparity: DisparityInterval,
    ) -> Self {
        Self {
            a_to_b: GpuL1DirectionControls {
                cost_modes: a_cost_modes,
                disparity: a_disparity,
                direction: PhantomData,
            },
            b_to_a: GpuL1DirectionControls {
                cost_modes: b_cost_modes,
                disparity: b_disparity,
                direction: PhantomData,
            },
        }
    }

    fn into_dispatch<'a>(
        self,
        solver: &'a GpuPisPipeline,
        stage: PairSolveStage,
        admissions: GpuAdmittedSnapshot,
    ) -> Fallible<ResidentPisDispatch<'a>> {
        solver.prepare_resident_grid_dispatch(
            stage,
            self.a_to_b.cost_modes,
            admissions.a_to_b(),
            Some(self.a_to_b.disparity),
            self.b_to_a.cost_modes,
            admissions.b_to_a(),
            Some(self.b_to_a.disparity),
        )
    }
}

/// Linear cold-call identity derived only from the preceding sealed L2
/// receipt. No caller can construct an integer ordinal or change its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GpuL1Ordinal {
    Warm,
    Cold0,
    Cold1,
    Cold2,
}

impl GpuL1Ordinal {
    fn after_l2(stage: PairSolveStage) -> Fallible<Self> {
        match stage {
            PairSolveStage::Warm { level: Level::Two } => Ok(Self::Warm),
            PairSolveStage::Cold {
                calculation: 0,
                level: Level::Two,
            } => Ok(Self::Cold0),
            PairSolveStage::Cold {
                calculation: 1,
                level: Level::Two,
            } => Ok(Self::Cold1),
            PairSolveStage::Cold {
                calculation: 2,
                level: Level::Two,
            } => Ok(Self::Cold2),
            _ => Err("ONE X2 resident L2 receipt has no valid next L1 ordinal".into()),
        }
    }

    fn l1_stage(self) -> PairSolveStage {
        match self {
            Self::Warm => PairSolveStage::Warm { level: Level::One },
            Self::Cold0 => PairSolveStage::Cold {
                calculation: 0,
                level: Level::One,
            },
            Self::Cold1 => PairSolveStage::Cold {
                calculation: 1,
                level: Level::One,
            },
            Self::Cold2 => PairSolveStage::Cold {
                calculation: 2,
                level: Level::One,
            },
        }
    }
}

impl GpuResidentLevelTwoPost for ColdResidentLevelTwoPost {
    fn context(&self) -> &OneXsGpuContext {
        &self.context
    }

    fn is_warm(&self) -> bool {
        false
    }

    fn admissions(&self) -> GpuAdmittedSnapshot {
        GpuAdmittedSnapshot::qualification_every_patch()
    }

    fn initialize(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.clear_buffer(&self.retained, 0, None);
        encoder.clear_buffer(&self.motion, 0, None);
    }

    fn bind(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        config: &wgpu::Buffer,
        images: &wgpu::Buffer,
        terminal: &wgpu::Buffer,
        output: &wgpu::Buffer,
        validity: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("L2 bridge cold resident bind group"),
            layout,
            entries: &[
                binding(0, config),
                binding(1, images),
                binding(2, terminal),
                binding(3, &self.retained),
                binding(4, &self.motion),
                binding(5, output),
                binding(6, validity),
            ],
        })
    }

    fn bind_l1_hint_fill(
        &self,
        _device: &wgpu::Device,
        _layout: &wgpu::BindGroupLayout,
        _config: &wgpu::Buffer,
        _dynamic: &wgpu::Buffer,
    ) -> Option<wgpu::BindGroup> {
        None
    }
}

impl GpuResidentLevelTwoPost for QualificationResidentLevelTwoPost {
    fn context(&self) -> &OneXsGpuContext {
        &self.context
    }

    fn is_warm(&self) -> bool {
        self.warm
    }

    fn admissions(&self) -> GpuAdmittedSnapshot {
        GpuAdmittedSnapshot::qualification_every_patch()
    }

    fn initialize(&self, _encoder: &mut wgpu::CommandEncoder) {}

    fn bind(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        config: &wgpu::Buffer,
        images: &wgpu::Buffer,
        terminal: &wgpu::Buffer,
        output: &wgpu::Buffer,
        validity: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("L2 bridge qualification bind group"),
            layout,
            entries: &[
                binding(0, config),
                binding(1, images),
                binding(2, terminal),
                binding(3, &self.retained),
                binding(4, &self.motion),
                binding(5, output),
                binding(6, validity),
            ],
        })
    }

    fn bind_l1_hint_fill(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        config: &wgpu::Buffer,
        dynamic: &wgpu::Buffer,
    ) -> Option<wgpu::BindGroup> {
        self.hints.as_ref().map(|hints| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("L2 bridge qualification hint fill"),
                layout,
                entries: &[binding(0, config), binding(1, hints), binding(2, dynamic)],
            })
        })
    }
}

#[cfg(test)]
impl GpuResidentLevelTwoPost for BorrowedProductionMotionPost<'_> {
    fn context(&self) -> &OneXsGpuContext {
        &self.context
    }

    fn is_warm(&self) -> bool {
        true
    }

    fn admissions(&self) -> GpuAdmittedSnapshot {
        GpuAdmittedSnapshot::qualification_every_patch()
    }

    fn initialize(&self, _encoder: &mut wgpu::CommandEncoder) {}

    fn bind(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        config: &wgpu::Buffer,
        images: &wgpu::Buffer,
        terminal: &wgpu::Buffer,
        output: &wgpu::Buffer,
        validity: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("L2 bridge production-motion qualification bind group"),
            layout,
            entries: &[
                binding(0, config),
                binding(1, images),
                binding(2, terminal),
                binding(3, &self.retained),
                binding(4, self.motion),
                binding(5, output),
                binding(6, validity),
            ],
        })
    }

    fn bind_l1_hint_fill(
        &self,
        _device: &wgpu::Device,
        _layout: &wgpu::BindGroupLayout,
        _config: &wgpu::Buffer,
        _dynamic: &wgpu::Buffer,
    ) -> Option<wgpu::BindGroup> {
        None
    }
}

impl LevelTwoPostUpdate {
    pub(crate) fn warm(
        a_to_b: RetainedLevelTwo<AtoB>,
        b_to_a: RetainedLevelTwo<BtoA>,
        motion: Vec<u8>,
    ) -> Result<Self, BridgeInputError> {
        check_len("motion", motion.len(), L2_PIXELS)?;
        Ok(Self::Warm {
            a_to_b,
            b_to_a,
            motion: motion.into_boxed_slice(),
        })
    }
}

/// Direction-owned L1 initial flows read back only for qualification today.
#[derive(Debug, PartialEq)]
#[cfg(test)]
pub(crate) struct DiagnosticLevelOneInitial<D: PisDirection> {
    flows: Box<[Flow]>,
    direction: PhantomData<D>,
}

#[cfg(test)]
impl<D: PisDirection> DiagnosticLevelOneInitial<D> {
    pub(crate) fn flows(&self) -> &[Flow] {
        &self.flows
    }
}

/// Paired output retaining the exact terminal transaction identity.
#[must_use = "the resident L2 continuation has not been consumed"]
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuL2BridgeOutput<K, P: GpuResidentLevelTwoPost>
{
    receipt: GpuL2BridgeReceipt,
    seeds: GpuPairedLevelOneInitialBuffer,
    validity: GpuResidentValidity,
    admissions: GpuAdmittedSnapshot,
    _terminal: wgpu::Buffer,
    _config: wgpu::Buffer,
    _resources: wgpu::BindGroup,
    // Drop the prepared carrier and its SubmissionLease before the post owner
    // can roll back the root reservation or release successor allocations.
    _prepared: GpuPreparedFrame<K>,
    _post: P,
}

/// Join the terminal carrier and resident post before any fallible check.
/// Field order is load-bearing: the prepared SubmissionLease retires before
/// the post may roll back its root reservation or release a successor.
struct TerminalPostGuard<K, P> {
    terminal: Option<GpuPreparedTerminal<K>>,
    post: Option<P>,
}

impl<K, P> TerminalPostGuard<K, P> {
    fn new(terminal: GpuPreparedTerminal<K>, post: P) -> Self {
        Self {
            terminal: Some(terminal),
            post: Some(post),
        }
    }

    fn terminal(&self) -> &GpuPreparedTerminal<K> {
        self.terminal.as_ref().expect("L2 join lost its terminal")
    }

    fn terminal_mut(&mut self) -> &mut GpuPreparedTerminal<K> {
        self.terminal.as_mut().expect("L2 join lost its terminal")
    }

    fn post(&self) -> &P {
        self.post.as_ref().expect("L2 join lost its post")
    }

    fn into_parts(mut self) -> (GpuPreparedTerminal<K>, P) {
        let terminal = self.terminal.take().expect("L2 join lost its terminal");
        let post = self.post.take().expect("L2 join lost its post");
        (terminal, post)
    }
}

/// Join the prepared carrier to the resident owner before resident L2 setup.
/// The optional fields permit a successful transfer without changing the
/// carrier-first drop order on errors or unwind.
struct PreparedPostGuard<K, P> {
    prepared: Option<GpuPreparedFrame<K>>,
    post: Option<P>,
}

impl<K, P> PreparedPostGuard<K, P> {
    fn new(prepared: GpuPreparedFrame<K>, post: P) -> Self {
        Self {
            prepared: Some(prepared),
            post: Some(post),
        }
    }

    fn into_parts(mut self) -> (GpuPreparedFrame<K>, P) {
        let prepared = self.prepared.take().expect("L2 PIS join lost its carrier");
        let post = self.post.take().expect("L2 PIS join lost its post");
        (prepared, post)
    }
}

impl<K> GpuPreparedFrame<K> {
    /// Purpose-specific resident L2 PIS. Dynamic grids are GPU-cleared and
    /// mandatory hint centres come only from the sealed resident post owner.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn submit_resident_l2_bridge<
        P: GpuResidentLevelTwoPost,
    >(
        self,
        solver: &GpuPisPipeline,
        bridge: &GpuL2PostPisBridge,
        controls: GpuL2Controls,
        stage: PairSolveStage,
        post: P,
        inherited_validity: Option<GpuResidentValidity>,
    ) -> Fallible<GpuL2BridgeOutput<K, P>> {
        let mut joined = PreparedPostGuard::new(self, post);
        let prepared = joined
            .prepared
            .as_ref()
            .expect("L2 PIS join lost its carrier");
        solver.validate_terminal_context(&prepared.context)?;
        bridge.context.ensure_same(&prepared.context)?;
        bridge.context.ensure_same(
            joined
                .post
                .as_ref()
                .expect("L2 PIS join lost its post")
                .context(),
        )?;
        if let Some(validity) = inherited_validity.as_ref() {
            let resident_identity = joined
                .post
                .as_ref()
                .expect("L2 PIS join lost its post")
                .resident_identity()?;
            validity.ensure_identity(
                &prepared.context,
                &prepared.flight,
                resident_identity.as_ref(),
            )?;
        }
        let admissions = joined
            .post
            .as_ref()
            .expect("L2 PIS join lost its post")
            .admissions();
        let mut dispatch = controls.into_dispatch(solver, stage, admissions)?;
        if dispatch.stage.level() != Level::Two {
            return Err("ONE X2 resident L2 dispatch did not prepare L2 PIS".into());
        }
        let pair = SealedPreparedPair::new::<GpuLevelTwo>(prepared);
        let flight = pair.flight.clone();
        let a_base = pis_direction_header_base(&dispatch.u32s, 2)?;
        let b_base = pis_direction_header_base(&dispatch.u32s, 5)?;
        write_pis_bases(
            &mut dispatch.u32s,
            a_base,
            pair.level,
            crate::flow::one_xs::Direction::AtoB,
            pair.a,
        )?;
        write_pis_bases(
            &mut dispatch.u32s,
            b_base,
            pair.level,
            crate::flow::one_xs::Direction::BtoA,
            pair.b,
        )?;

        let device = bridge.context.device();
        let queue = bridge.context.queue();
        let dynamic_u32 = super::upload_words(
            device,
            queue,
            "ONE X2 resident L2 PIS dynamic u32 metadata",
            &dispatch.u32s,
        );
        let dynamic_f32 = resident_buffer(
            device,
            "ONE X2 resident L2 PIS GPU-filled grids",
            dispatch.float_words,
        );
        let output = super::storage_buffer(
            device,
            "ONE X2 resident L2 PIS terminal bits",
            dispatch.output_words,
        );
        let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 resident L2 PIS sealed resources"),
            layout: dispatch.layout,
            entries: &[
                binding(0, &dynamic_u32),
                binding(1, &dynamic_f32),
                binding(2, &output),
                binding(3, pair.shared_images),
                binding(4, pair.shared_masks),
                binding(5, pair.gradients),
                binding(6, pair.raw_weights),
                binding(7, pair.patch_weight_sums),
                binding(8, pair.models),
            ],
        });
        let hint_config = upload(
            device,
            "ONE X2 resident dense-to-L2 hint config",
            &u32_bytes(&[
                129_600,
                133_650,
                267_300,
                271_350,
                2 * L2_PATCHES as u32,
                u32::try_from(dispatch.direction_float_words)? + 2 * L2_PATCHES as u32,
                L2_PATCHES as u32,
                Level::Two.patch_cols() as u32,
                Level::Two.cols() as u32,
            ]),
        );
        let hint_resources = joined
            .post
            .as_ref()
            .expect("L2 PIS join lost its post")
            .bind_l2_hint_fill(device, &bridge.hint_layout, &hint_config, &dynamic_f32);
        if joined
            .post
            .as_ref()
            .expect("L2 PIS join lost its post")
            .is_warm()
            && hint_resources.is_none()
        {
            return Err("ONE X2 warm resident L2 PIS has no sealed hint producer".into());
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 resident L2 PIS"),
        });
        encoder.clear_buffer(&dynamic_f32, 0, None);
        encode_disparities(&dispatch, device, queue, &dynamic_f32, &mut encoder)?;
        if let Some(hint_resources) = &hint_resources {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 resident dense-to-L2 hint fill"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&bridge.hint_pipeline);
            pass.set_bind_group(0, hint_resources, &[]);
            pass.dispatch_workgroups(L2_PATCHES.div_ceil(64) as u32, 2, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 resident paired L2 GPU PIS"),
                timestamp_writes: None,
            });
            pass.set_pipeline(dispatch.pipeline);
            pass.set_bind_group(0, &resources, &[]);
            pass.dispatch_workgroups(2, 1, 1);
        }
        joined
            .prepared
            .as_mut()
            .expect("L2 PIS join lost its carrier")
            .belts
            .lease
            .submit_after(&bridge.context, |_| encoder.finish())?;
        let (prepared, post) = joined.into_parts();
        GpuPreparedTerminal {
            receipt: GpuPisStageReceipt { flight, stage },
            context: bridge.context.clone(),
            _output: output,
            _output_span_words: dispatch.output_span_words,
            _b_output_base_words: dispatch.b_output_base_words,
            resident_validity: inherited_validity,
            prepared,
        }
        .submit_l2_bridge_admitted(bridge, post, admissions)
    }
}

impl<K, P: GpuResidentLevelTwoPost> GpuL2BridgeOutput<K, P> {
    #[cfg(test)]
    fn read_l2_terminal_words_for_test(&self) -> Result<Vec<u32>, Box<dyn Error>> {
        read_buffer_words(
            &self.seeds.context,
            &self._terminal,
            4 * self.receipt.0.stage.level().patches(),
        )
    }

    /// Consume the complete L2 carrier into paired L1 PIS. The resident seed
    /// planes are interleaved into the private dynamic allocation by GPU work
    /// in the same submission; callers cannot supply or recover an L1 initial.
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn submit_l1_pis(
        mut self,
        bridge: &GpuL2PostPisBridge,
        solver: &GpuPisPipeline,
        controls: GpuL1Controls,
    ) -> Fallible<GpuL1PreparedTerminal<K, P>> {
        // Keep the carrier-first aggregate intact through every fallible
        // validation, allocation, encode and submission. Its field order
        // guarantees lease retirement before post rollback on every exit.
        bridge.context.ensure_same(&self.seeds.context)?;
        solver.validate_terminal_context(&self.seeds.context)?;
        let ordinal = GpuL1Ordinal::after_l2(self.receipt.0.stage)?;
        let admissions = self.admissions;
        let mut dispatch = controls.into_dispatch(solver, ordinal.l1_stage(), admissions)?;
        if dispatch.stage.level() != Level::One {
            return Err("ONE X2 resident L2 continuation did not prepare L1 PIS".into());
        }
        let pair = SealedPreparedPair::new::<GpuLevelOne>(&self._prepared);
        if pair.flight != &self.receipt.0.flight {
            return Err("ONE X2 resident L2-to-L1 flight does not match its prepared frame".into());
        }
        let flight = pair.flight.clone();
        let a_base = pis_direction_header_base(&dispatch.u32s, 2)?;
        let b_base = pis_direction_header_base(&dispatch.u32s, 5)?;
        write_pis_bases(
            &mut dispatch.u32s,
            a_base,
            pair.level,
            crate::flow::one_xs::Direction::AtoB,
            pair.a,
        )?;
        write_pis_bases(
            &mut dispatch.u32s,
            b_base,
            pair.level,
            crate::flow::one_xs::Direction::BtoA,
            pair.b,
        )?;

        let device = bridge.context.device();
        let queue = bridge.context.queue();
        let dynamic_u32 = super::upload_words(
            device,
            queue,
            "ONE X2 resident L1 PIS dynamic u32 input",
            &dispatch.u32s,
        );
        let dynamic_f32 = resident_buffer(
            device,
            "ONE X2 resident L1 PIS dynamic f32 input",
            dispatch.float_words,
        );
        let output = super::storage_buffer(
            device,
            "ONE X2 resident L1 PIS terminal bits",
            dispatch.output_words,
        );
        let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 resident L1 PIS sealed resources"),
            layout: dispatch.layout,
            entries: &[
                binding(0, &dynamic_u32),
                binding(1, &dynamic_f32),
                binding(2, &output),
                binding(3, pair.shared_images),
                binding(4, pair.shared_masks),
                binding(5, pair.gradients),
                binding(6, pair.raw_weights),
                binding(7, pair.patch_weight_sums),
                binding(8, pair.models),
            ],
        });
        let seed_config = upload(
            device,
            "ONE X2 resident L2-to-L1 seed config",
            &u32_bytes(&[
                u32::try_from(self.seeds.plane_stride / 4)?,
                dispatch.u32s[6],
            ]),
        );
        let seed_resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 resident L2-to-L1 seed injection"),
            layout: &bridge.seed_layout,
            entries: &[
                binding(0, &seed_config),
                binding(1, &self.seeds.buffer),
                binding(2, &dynamic_f32),
            ],
        });
        let hint_config = upload(
            device,
            "ONE X2 resident dense-to-L1 hint config",
            &u32_bytes(&[
                0,
                64_800,
                137_700,
                202_500,
                2 * L1_PATCHES as u32,
                u32::try_from(dispatch.direction_float_words)? + 2 * L1_PATCHES as u32,
                L1_PATCHES as u32,
                Level::One.patch_cols() as u32,
                Level::One.cols() as u32,
            ]),
        );
        let hint_resources =
            self._post
                .bind_l1_hint_fill(device, &bridge.hint_layout, &hint_config, &dynamic_f32);
        if ordinal == GpuL1Ordinal::Warm && hint_resources.is_none() {
            return Err("ONE X2 warm resident L1 PIS has no sealed hint producer".into());
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 resident L2-to-L1 PIS continuation"),
        });
        encoder.clear_buffer(&dynamic_f32, 0, None);
        encode_disparities(&dispatch, device, queue, &dynamic_f32, &mut encoder)?;
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 resident L2-to-L1 seed injection"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&bridge.seed_pipeline);
            pass.set_bind_group(0, &seed_resources, &[]);
            pass.dispatch_workgroups(L1_PATCHES.div_ceil(64) as u32, 2, 1);
        }
        if let Some(hint_resources) = &hint_resources {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 resident dense-to-L1 hint fill"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&bridge.hint_pipeline);
            pass.set_bind_group(0, hint_resources, &[]);
            pass.dispatch_workgroups(L1_PATCHES.div_ceil(64) as u32, 2, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 resident paired L1 GPU PIS"),
                timestamp_writes: None,
            });
            pass.set_pipeline(dispatch.pipeline);
            pass.set_bind_group(0, &resources, &[]);
            pass.dispatch_workgroups(2, 1, 1);
        }
        self._prepared
            .belts
            .lease
            .submit_after(&bridge.context, |_| encoder.finish())?;
        let GpuL2BridgeOutput {
            receipt: _,
            seeds: _,
            validity,
            admissions: _,
            _terminal,
            _config,
            _resources,
            _prepared,
            _post: post,
        } = self;
        let terminal = GpuPreparedTerminal {
            receipt: GpuPisStageReceipt {
                flight,
                stage: dispatch.stage,
            },
            context: bridge.context.clone(),
            _output: output,
            _output_span_words: dispatch.output_span_words,
            _b_output_base_words: dispatch.b_output_base_words,
            resident_validity: Some(validity),
            prepared: _prepared,
        };
        Ok(GpuL1PreparedTerminal {
            terminal,
            post,
            ordinal,
        })
    }

    #[cfg(test)]
    pub(crate) fn direction<D: PisDirection>(&self) -> GpuLevelOneInitialPlanes<'_, D> {
        self.seeds.direction()
    }
}

/// Purpose-specific paired L1 terminal. Besides the sealed PIS allocation it
/// owns the resident L2 post input, which may itself be the pending temporal
/// successor. It has no direct acknowledgement path.
#[must_use = "the resident L1 terminal must enter post-L1 processing"]
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuL1PreparedTerminal<
    K,
    P: GpuResidentLevelTwoPost,
> {
    terminal: GpuPreparedTerminal<K>,
    post: P,
    ordinal: GpuL1Ordinal,
}

#[cfg(test)]
impl<K, P: GpuResidentLevelTwoPost> GpuL1PreparedTerminal<K, P> {
    fn read_terminal_words_for_test(&self) -> Result<Vec<u32>, Box<dyn Error>> {
        read_buffer_words(
            &self.terminal.context,
            &self.terminal._output,
            self.terminal._b_output_base_words + self.terminal._output_span_words,
        )
    }

    fn acknowledge_qualified_for_test(mut self) -> Fallible<()> {
        if self.terminal.receipt.stage.level() != Level::One
            || self.terminal.resident_validity.take().is_none()
        {
            return Err("qualified resident L1 terminal lost its validity token".into());
        }
        let GpuL1PreparedTerminal {
            terminal,
            post,
            ordinal: _,
        } = self;
        let result = terminal.prepared.acknowledge_terminal();
        drop(post);
        result
    }
}

/// Device-resident paired L1 seed planes. The buffer is direction-major then
/// component-major: A-to-B dcol/drow, B-to-A dcol/drow.
pub(crate) struct GpuPairedLevelOneInitialBuffer {
    buffer: wgpu::Buffer,
    plane_stride: u64,
    context: OneXsGpuContext,
}

#[cfg(test)]
pub(crate) struct DiagnosticPairedLevelOneInitial {
    pub(crate) a_to_b: DiagnosticLevelOneInitial<AtoB>,
    pub(crate) b_to_a: DiagnosticLevelOneInitial<BtoA>,
}

/// Typed offsets into one direction's two contiguous L1 seed planes.
pub(crate) struct GpuLevelOneInitialPlanes<'a, D: PisDirection> {
    buffer: &'a wgpu::Buffer,
    dcol_offset: u64,
    drow_offset: u64,
    binding_size: std::num::NonZeroU64,
    direction: PhantomData<D>,
}

impl GpuPairedLevelOneInitialBuffer {
    pub(crate) fn direction<D: PisDirection>(&self) -> GpuLevelOneInitialPlanes<'_, D> {
        let direction = match D::DIRECTION {
            crate::flow::one_xs::Direction::AtoB => 0,
            crate::flow::one_xs::Direction::BtoA => 1,
        };
        let plane_bytes = self.plane_stride;
        GpuLevelOneInitialPlanes {
            buffer: &self.buffer,
            dcol_offset: direction * 2 * plane_bytes,
            drow_offset: (direction * 2 + 1) * plane_bytes,
            binding_size: std::num::NonZeroU64::new((L1_PATCHES * 4) as u64).unwrap(),
            direction: PhantomData,
        }
    }

    /// Explicit seed readback for qualification and diagnostics only.
    #[cfg(test)]
    pub(crate) fn readback_diagnostic(
        &self,
    ) -> Result<DiagnosticPairedLevelOneInitial, Box<dyn Error>> {
        let words = self.readback_words()?;
        let stride = (self.plane_stride / 4) as usize;
        let direction = |base: usize| {
            [
                &words[base..base + L1_PATCHES],
                &words[base + stride..base + stride + L1_PATCHES],
            ]
            .concat()
        };
        Ok(DiagnosticPairedLevelOneInitial {
            a_to_b: decode::<AtoB>(&direction(0))?,
            b_to_a: decode::<BtoA>(&direction(2 * stride))?,
        })
    }

    fn readback_words(&self) -> Result<Vec<u32>, Box<dyn Error>> {
        let output_size = PLANES as u64 * self.plane_stride;
        let device = self.context.device();
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("L2 bridge diagnostic readback"),
            size: output_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("L2 bridge diagnostic readback"),
        });
        encoder.copy_buffer_to_buffer(&self.buffer, 0, &readback, 0, output_size);
        let submission = self.context.queue().submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (sent, received) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sent.send(result);
        });
        device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        received.recv()??;
        let mapped = slice.get_mapped_range();
        let words = mapped
            .chunks_exact(4)
            .map(|v| u32::from_ne_bytes(v.try_into().unwrap()))
            .collect();
        drop(mapped);
        readback.unmap();
        Ok(words)
    }
}

impl<'a, D: PisDirection> GpuLevelOneInitialPlanes<'a, D> {
    pub(crate) fn dcol(&self) -> wgpu::BufferBinding<'a> {
        wgpu::BufferBinding {
            buffer: self.buffer,
            offset: self.dcol_offset,
            size: Some(self.binding_size),
        }
    }
    pub(crate) fn drow(&self) -> wgpu::BufferBinding<'a> {
        wgpu::BufferBinding {
            buffer: self.buffer,
            offset: self.drow_offset,
            size: Some(self.binding_size),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BridgeInputError {
    Shape {
        part: &'static str,
        expected: usize,
        actual: usize,
    },
    NonFiniteTerminal,
    WrongReceiptLevel(Level),
    NonFiniteCenter {
        direction: super::Direction,
        component: &'static str,
        patch_row: usize,
        patch_col: usize,
        dense_row: usize,
        dense_col: usize,
    },
}

impl fmt::Display for BridgeInputError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shape {
                part,
                expected,
                actual,
            } => write!(
                out,
                "ONE X2 GPU L2 bridge {part} has {actual} values, expected {expected}"
            ),
            Self::NonFiniteTerminal => {
                write!(out, "ONE X2 GPU L2 bridge terminal flow is not finite")
            }
            Self::WrongReceiptLevel(level) => write!(
                out,
                "ONE X2 GPU L2 bridge received {level}, expected level two"
            ),
            Self::NonFiniteCenter {
                direction,
                component,
                patch_row,
                patch_col,
                dense_row,
                dense_col,
            } => write!(
                out,
                "ONE X2 {direction} level-one initial {component} at patch row {patch_row} column {patch_col}, dense row {dense_row} column {dense_col}, is not finite"
            ),
        }
    }
}
impl Error for BridgeInputError {}

fn check_len(part: &'static str, actual: usize, expected: usize) -> Result<(), BridgeInputError> {
    (actual == expected)
        .then_some(())
        .ok_or(BridgeInputError::Shape {
            part,
            expected,
            actual,
        })
}

#[derive(Debug)]
struct QualificationError {
    case: &'static str,
    word: usize,
    expected: u32,
    actual: u32,
}
impl fmt::Display for QualificationError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "ONE X2 GPU L2 bridge {} qualification word {} was {:#010x}, expected {:#010x}",
            self.case, self.word, self.actual, self.expected
        )
    }
}
impl Error for QualificationError {}

#[derive(Debug)]
enum BridgeGpuError {
    Scoped(String),
    Panic,
}
impl fmt::Display for BridgeGpuError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scoped(error) => write!(out, "ONE X2 GPU L2 bridge GPU error: {error}"),
            Self::Panic => write!(out, "ONE X2 GPU L2 bridge GPU operation panicked"),
        }
    }
}
impl Error for BridgeGpuError {}

fn scoped_gpu<T>(
    device: &wgpu::Device,
    operation: impl FnOnce() -> T,
) -> Result<T, BridgeGpuError> {
    let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation));
    let errors = [validation.pop(), internal.pop(), oom.pop()]
        .into_iter()
        .filter_map(|future| block_on_gpu(device, future))
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(BridgeGpuError::Scoped(errors.join("; ")));
    }
    result.map_err(|_| BridgeGpuError::Panic)
}

fn block_on_gpu<F: std::future::Future>(device: &wgpu::Device, future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(answer) => return answer,
            std::task::Poll::Pending => {
                let _ = device.poll(wgpu::PollType::Poll);
                std::thread::yield_now();
            }
        }
    }
}

/// The unselected render-private pipeline. Construction qualifies this exact
/// shader before it may process a caller's buffers.
pub(crate) struct GpuL2PostPisBridge {
    context: OneXsGpuContext,
    plane_stride: u64,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    seed_layout: wgpu::BindGroupLayout,
    seed_pipeline: wgpu::ComputePipeline,
    hint_layout: wgpu::BindGroupLayout,
    hint_pipeline: wgpu::ComputePipeline,
    post_l1: post_l1::GpuColdPostL1Pipeline,
}

impl GpuL2PostPisBridge {
    pub(crate) fn new(context: OneXsGpuContext) -> Result<Self, Box<dyn Error>> {
        Self::new_qualified_all(
            context,
            SHADER,
            SEED_INJECTION_SHADER,
            HINT_INJECTION_SHADER,
        )
    }

    fn new_qualified(context: OneXsGpuContext, shader: &str) -> Result<Self, Box<dyn Error>> {
        Self::new_qualified_all(
            context,
            shader,
            SEED_INJECTION_SHADER,
            HINT_INJECTION_SHADER,
        )
    }

    fn new_qualified_all(
        context: OneXsGpuContext,
        shader: &str,
        seed_shader: &str,
        hint_shader: &str,
    ) -> Result<Self, Box<dyn Error>> {
        let bridge = Self::from_shaders(context, shader, seed_shader, hint_shader)?;
        bridge.qualify()?;
        Ok(bridge)
    }

    fn from_shaders(
        context: OneXsGpuContext,
        shader: &str,
        seed_shader: &str,
        hint_shader: &str,
    ) -> Result<Self, BridgeGpuError> {
        let device = context.device();
        let alignment = u64::from(device.limits().min_storage_buffer_offset_alignment);
        let plane_bytes = (L1_PATCHES * 4) as u64;
        let plane_stride = plane_bytes.div_ceil(alignment) * alignment;
        let (layout, pipeline, seed_layout, seed_pipeline, hint_layout, hint_pipeline) =
            scoped_gpu(device, || {
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("ONE X2 L2 post-PIS bridge"),
                    source: wgpu::ShaderSource::Wgsl(shader.into()),
                });
                let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("ONE X2 L2 post-PIS bridge resources"),
                    entries: &[
                        storage_entry(0, true),
                        storage_entry(1, true),
                        storage_entry(2, true),
                        storage_entry(3, true),
                        storage_entry(4, true),
                        storage_entry(5, false),
                        storage_entry(6, false),
                    ],
                });
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("ONE X2 L2 post-PIS bridge layout"),
                        bind_group_layouts: &[&layout],
                        immediate_size: 0,
                    });
                let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("ONE X2 L2 post-PIS bridge"),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                });
                let seed_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("ONE X2 resident L2-to-L1 seed injection"),
                    source: wgpu::ShaderSource::Wgsl(seed_shader.into()),
                });
                let seed_layout =
                    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("ONE X2 resident L2-to-L1 seed injection"),
                        entries: &[
                            storage_entry(0, true),
                            storage_entry(1, true),
                            storage_entry(2, false),
                        ],
                    });
                let seed_pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("ONE X2 resident L2-to-L1 seed injection"),
                        bind_group_layouts: &[&seed_layout],
                        immediate_size: 0,
                    });
                let seed_pipeline =
                    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some("ONE X2 resident L2-to-L1 seed injection"),
                        layout: Some(&seed_pipeline_layout),
                        module: &seed_module,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        cache: None,
                    });
                let hint_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("ONE X2 resident dense-to-L1 hint fill"),
                    source: wgpu::ShaderSource::Wgsl(hint_shader.into()),
                });
                let hint_layout =
                    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("ONE X2 resident dense-to-L1 hint fill"),
                        entries: &[
                            storage_entry(0, true),
                            storage_entry(1, true),
                            storage_entry(2, false),
                        ],
                    });
                let hint_pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("ONE X2 resident dense-to-L1 hint fill"),
                        bind_group_layouts: &[&hint_layout],
                        immediate_size: 0,
                    });
                let hint_pipeline =
                    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some("ONE X2 resident dense-to-L1 hint fill"),
                        layout: Some(&hint_pipeline_layout),
                        module: &hint_module,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        cache: None,
                    });
                (
                    layout,
                    pipeline,
                    seed_layout,
                    seed_pipeline,
                    hint_layout,
                    hint_pipeline,
                )
            })?;
        let post_l1 = post_l1::GpuColdPostL1Pipeline::new(context.clone())
            .map_err(|error| BridgeGpuError::Scoped(error.to_string()))?;
        Ok(Self {
            context,
            plane_stride,
            layout,
            pipeline,
            seed_layout,
            seed_pipeline,
            hint_layout,
            hint_pipeline,
            post_l1,
        })
    }

    fn cold_post(&self) -> ColdResidentLevelTwoPost {
        let device = self.context.device();
        ColdResidentLevelTwoPost {
            context: self.context.clone(),
            retained: resident_buffer(device, "L2 bridge cold retained", 4 * L2_PIXELS),
            motion: resident_buffer(
                device,
                "L2 bridge cold packed motion",
                L2_PIXELS.div_ceil(4),
            ),
        }
    }

    fn allocate_outputs(&self) -> (wgpu::Buffer, wgpu::Buffer) {
        let device = self.context.device();
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("L2 bridge seeds"),
            size: PLANES as u64 * self.plane_stride,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        use wgpu::util::DeviceExt;
        let validity = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("L2 bridge resident finite-center status"),
            contents: &u32::MAX.to_ne_bytes(),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        (output, validity)
    }

    fn encode<P: GpuResidentLevelTwoPost>(
        &self,
        post: &P,
        config: &wgpu::Buffer,
        images: &wgpu::Buffer,
        terminal: &wgpu::Buffer,
        output: &wgpu::Buffer,
        validity: &wgpu::Buffer,
    ) -> (wgpu::CommandBuffer, wgpu::BindGroup) {
        let device = self.context.device();
        let bind = post.bind(
            device,
            &self.layout,
            config,
            images,
            terminal,
            output,
            validity,
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 L2 post-PIS bridge"),
        });
        post.initialize(&mut encoder);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 L2 post-PIS bridge"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(L1_PATCHES.div_ceil(64) as u32, 2, 1);
        }
        (encoder.finish(), bind)
    }
}

impl<K> GpuPreparedTerminal<K> {
    /// Consume this exact sealed L2 PIS terminal into its resident bridge.
    /// Encoding and submission stay inside the frame owner, so no raw buffer,
    /// command buffer or separable lease component crosses the boundary.
    fn submit_l2_bridge_admitted<P: GpuResidentLevelTwoPost>(
        self,
        bridge: &GpuL2PostPisBridge,
        post: P,
        admissions: GpuAdmittedSnapshot,
    ) -> Fallible<GpuL2BridgeOutput<K, P>> {
        let mut joined = TerminalPostGuard::new(self, post);
        bridge.context.ensure_same(&joined.terminal().context)?;
        bridge.context.ensure_same(joined.post().context())?;
        let receipt = GpuL2BridgeReceipt::from_terminal(joined.terminal().receipt.clone())?;
        let stage_is_warm = matches!(receipt.0.stage, PairSolveStage::Warm { .. });
        if stage_is_warm != joined.post().is_warm() {
            return Err("ONE X2 resident L2 PIS stage does not match its sealed history".into());
        }
        check_len(
            "terminal span",
            joined.terminal()._output_span_words,
            2 * L2_PATCHES,
        )?;
        check_len(
            "terminal B base",
            joined.terminal()._b_output_base_words,
            2 * L2_PATCHES,
        )?;
        joined
            .terminal()
            .context
            .ensure_same(&joined.terminal().prepared.context)?;
        let pair = SealedPreparedPair::new::<GpuLevelTwo>(&joined.terminal().prepared);
        let config = [
            joined.post().is_warm() as u32,
            0,
            (bridge.plane_stride / 4) as u32,
            1,
            0,
            u32::try_from(joined.terminal()._b_output_base_words)?,
            pair.a.source_image,
            pair.b.source_image,
        ];
        let config = upload(
            bridge.context.device(),
            "L2 bridge config",
            &u32_bytes(&config),
        );
        let (seeds, fresh_validity) = bridge.allocate_outputs();
        let resident_identity = joined.post().resident_identity()?;
        let validity = joined
            .terminal_mut()
            .resident_validity
            .take()
            .unwrap_or_else(|| {
                GpuResidentValidity::new(
                    bridge.context.clone(),
                    receipt.0.flight.clone(),
                    resident_identity.clone(),
                    fresh_validity,
                )
            });
        validity.ensure_identity(
            &bridge.context,
            &receipt.0.flight,
            resident_identity.as_ref(),
        )?;
        let (command, resources) = bridge.encode(
            joined.post(),
            &config,
            &joined.terminal().prepared.shared_images,
            &joined.terminal()._output,
            &seeds,
            &validity.buffer,
        );
        let context = joined.terminal().context.clone();
        joined
            .terminal_mut()
            .prepared
            .belts
            .lease
            .submit_after(&context, |_| command)?;
        let (terminal, post) = joined.into_parts();
        let GpuPreparedTerminal {
            receipt: _,
            context: _,
            _output: terminal_buffer,
            _output_span_words: _,
            _b_output_base_words: _,
            resident_validity,
            prepared,
        } = terminal;
        debug_assert!(resident_validity.is_none());
        Ok(GpuL2BridgeOutput {
            receipt,
            seeds: GpuPairedLevelOneInitialBuffer {
                buffer: seeds,
                plane_stride: bridge.plane_stride,
                context: context.clone(),
            },
            validity,
            admissions,
            _terminal: terminal_buffer,
            _config: config,
            _resources: resources,
            _prepared: prepared,
            _post: post,
        })
    }

    #[cfg(test)]
    fn submit_l2_bridge<P: GpuResidentLevelTwoPost>(
        self,
        bridge: &GpuL2PostPisBridge,
        post: P,
    ) -> Fallible<GpuL2BridgeOutput<K, P>> {
        let admissions = post.admissions();
        self.submit_l2_bridge_admitted(bridge, post, admissions)
    }
}

impl GpuL2PostPisBridge {
    fn submit_qualification(
        &self,
        a_terminal: &CpuUploadedTerminalL2<AtoB>,
        b_terminal: &CpuUploadedTerminalL2<BtoA>,
        images: &LevelTwoImages,
        post: &LevelTwoPostUpdate,
    ) -> Result<GpuPairedLevelOneInitialBuffer, Box<dyn Error>> {
        let device = self.context.device();
        let (retained, motion) = pack_post(post);
        let resident = QualificationResidentLevelTwoPost {
            context: self.context.clone(),
            warm: matches!(post, LevelTwoPostUpdate::Warm { .. }),
            retained: upload(device, "L2 bridge retained", &f32_bytes(&retained)),
            motion: upload(device, "L2 bridge motion", &u32_bytes(&motion)),
            hints: None,
        };
        self.submit_qualification_resident(a_terminal, b_terminal, images, resident)
    }

    fn submit_qualification_resident<P: GpuResidentLevelTwoPost>(
        &self,
        a_terminal: &CpuUploadedTerminalL2<AtoB>,
        b_terminal: &CpuUploadedTerminalL2<BtoA>,
        images: &LevelTwoImages,
        post: P,
    ) -> Result<GpuPairedLevelOneInitialBuffer, Box<dyn Error>> {
        let device = self.context.device();
        let config = upload(
            device,
            "L2 bridge config",
            &u32_bytes(&[
                post.is_warm() as u32,
                0,
                (self.plane_stride / 4) as u32,
                1,
                0,
                (2 * L2_PATCHES) as u32,
                0,
                L2_PIXELS as u32,
            ]),
        );
        let image_words = images
            .a
            .iter()
            .chain(images.b.iter())
            .map(|&v| u32::from(v))
            .collect::<Vec<_>>();
        let terminal_values = [
            a_terminal
                .dcol
                .iter()
                .zip(&a_terminal.drow)
                .flat_map(|(&dcol, &drow)| [dcol, drow])
                .collect::<Vec<_>>(),
            b_terminal
                .dcol
                .iter()
                .zip(&b_terminal.drow)
                .flat_map(|(&dcol, &drow)| [dcol, drow])
                .collect::<Vec<_>>(),
        ]
        .concat();
        let (_image_buffer, _terminal_buffer, _post, _resources, output, status) =
            scoped_gpu(device, || {
                let image_buffer = upload(device, "L2 bridge images", &u32_bytes(&image_words));
                let terminal_buffer =
                    upload(device, "L2 bridge terminal", &f32_bytes(&terminal_values));
                let (output, status) = self.allocate_outputs();
                let (command, resources) = self.encode(
                    &post,
                    &config,
                    &image_buffer,
                    &terminal_buffer,
                    &output,
                    &status,
                );
                self.context.queue().submit([command]);
                (
                    image_buffer,
                    terminal_buffer,
                    post,
                    resources,
                    output,
                    status,
                )
            })?;
        let status_word = read_one_word(device, self.context.queue(), &status)?;
        if status_word != u32::MAX {
            let plane = status_word as usize / L1_PATCHES;
            let patch = status_word as usize % L1_PATCHES;
            let direction = if plane < 2 {
                crate::flow::one_xs::Direction::AtoB
            } else {
                crate::flow::one_xs::Direction::BtoA
            };
            let component = if plane.is_multiple_of(2) {
                "dcol"
            } else {
                "drow"
            };
            let patch_row = patch / 8;
            let patch_col = patch % 8;
            return Err(Box::new(BridgeInputError::NonFiniteCenter {
                direction,
                component,
                patch_row,
                patch_col,
                dense_row: patch_row * 3 + 4,
                dense_col: patch_col * 3 + 4,
            }));
        }
        Ok(GpuPairedLevelOneInitialBuffer {
            buffer: output,
            plane_stride: self.plane_stride,
            context: self.context.clone(),
        })
    }

    /// Bind the exact packed level-two motion allocation emitted by the
    /// production temporal kernel and compare its bridge result to the CPU
    /// contract. This is deliberately a qualification-only raw-buffer seam.
    #[cfg(test)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn qualify_production_motion_buffer_for_test(
        &self,
        motion: &wgpu::Buffer,
        expected_motion: &[u8],
    ) -> Result<(), Box<dyn Error>> {
        check_len("production motion", expected_motion.len(), L2_PIXELS)?;
        let fixture = fixture()?;
        let LevelTwoPostUpdate::Warm {
            a_to_b,
            b_to_a,
            motion: _,
        } = &fixture.warm
        else {
            unreachable!("qualification warm fixture changed shape")
        };
        let retained = a_to_b
            .dcol
            .iter()
            .chain(&a_to_b.drow)
            .chain(&b_to_a.dcol)
            .chain(&b_to_a.drow)
            .copied()
            .collect::<Vec<_>>();
        let resident = BorrowedProductionMotionPost {
            context: self.context.clone(),
            retained: upload(
                self.context.device(),
                "L2 bridge production-motion retained fixture",
                &f32_bytes(&retained),
            ),
            motion,
        };
        let seeds = self.submit_qualification_resident(
            &fixture.a_terminal,
            &fixture.b_terminal,
            &fixture.images,
            resident,
        )?;
        let padded = seeds.readback_words()?;
        let stride = usize::try_from(self.plane_stride / 4)?;
        let actual = (0..PLANES)
            .flat_map(|plane| padded[plane * stride..][..L1_PATCHES].iter().copied())
            .collect::<Vec<_>>();
        let expected_post =
            LevelTwoPostUpdate::warm(a_to_b.clone(), b_to_a.clone(), expected_motion.to_vec())?;
        compare_words(
            "production packed motion",
            &actual,
            &cpu_words(
                &fixture.a_terminal,
                &fixture.b_terminal,
                &fixture.images,
                &expected_post,
            ),
        )?;
        Ok(())
    }

    fn qualify(&self) -> Result<(), Box<dyn Error>> {
        let fixture = fixture()?;
        for (case, post) in [("cold", &fixture.cold), ("warm", &fixture.warm)] {
            let seeds = self.submit_qualification(
                &fixture.a_terminal,
                &fixture.b_terminal,
                &fixture.images,
                post,
            )?;
            self.qualify_direction_spans(&seeds)?;
            let padded = seeds.readback_words()?;
            let stride = (self.plane_stride / 4) as usize;
            let actual = (0..PLANES)
                .flat_map(|plane| padded[plane * stride..][..L1_PATCHES].iter().copied())
                .collect::<Vec<_>>();
            let expected = cpu_words(
                &fixture.a_terminal,
                &fixture.b_terminal,
                &fixture.images,
                post,
            );
            compare_words(case, &actual, &expected)?;
        }
        self.qualify_grid_injection()?;
        Ok(())
    }

    fn qualify_grid_injection(&self) -> Result<(), Box<dyn Error>> {
        let device = self.context.device();
        let stride = usize::try_from(self.plane_stride / 4)?;
        let mut seed_words = vec![0; PLANES * stride];
        for plane in 0..PLANES {
            for patch in 0..L1_PATCHES {
                seed_words[plane * stride + patch] =
                    0x3f00_0000u32.wrapping_add((plane * L1_PATCHES + patch) as u32);
            }
        }
        let seeds = upload(
            device,
            "ONE X2 seed injection qualifier",
            &u32_bytes(&seed_words),
        );
        let dynamic = resident_buffer(device, "ONE X2 grid injection qualifier", 11_400);
        let config = upload(
            device,
            "ONE X2 seed injection qualifier config",
            &u32_bytes(&[u32::try_from(stride)?, 5_700]),
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 seed injection qualifier"),
            layout: &self.seed_layout,
            entries: &[
                binding(0, &config),
                binding(1, &seeds),
                binding(2, &dynamic),
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.clear_buffer(&dynamic, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.seed_pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(L1_PATCHES.div_ceil(64) as u32, 2, 1);
        }
        self.context.queue().submit([encoder.finish()]);
        let actual = read_buffer_words(&self.context, &dynamic, 11_400)?;
        for direction in 0..2 {
            for patch in 0..L1_PATCHES {
                let dst = direction * 5_700 + 2 * patch;
                let src = direction * 2 * stride + patch;
                if actual[dst] != seed_words[src] || actual[dst + 1] != seed_words[src + stride] {
                    return Err(format!(
                        "ONE X2 L1 seed injection is not exact at direction {direction} patch {patch}"
                    )
                    .into());
                }
            }
        }

        let dense_words = (0..275_400u32)
            .map(|word| 0x4000_0000u32.wrapping_add(word))
            .collect::<Vec<_>>();
        let dense = upload(
            device,
            "ONE X2 hint injection qualifier",
            &u32_bytes(&dense_words),
        );
        let hint_config = upload(
            device,
            "ONE X2 hint injection qualifier config",
            &u32_bytes(&[
                0,
                64_800,
                137_700,
                202_500,
                2_848,
                8_548,
                L1_PATCHES as u32,
                Level::One.patch_cols() as u32,
                Level::One.cols() as u32,
            ]),
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 hint injection qualifier"),
            layout: &self.hint_layout,
            entries: &[
                binding(0, &hint_config),
                binding(1, &dense),
                binding(2, &dynamic),
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.hint_pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(L1_PATCHES.div_ceil(64) as u32, 2, 1);
        }
        self.context.queue().submit([encoder.finish()]);
        let actual = read_buffer_words(&self.context, &dynamic, 11_400)?;
        for direction in 0..2 {
            let (col_base, row_base, dst_base) = if direction == 0 {
                (0, 64_800, 2_848)
            } else {
                (137_700, 202_500, 8_548)
            };
            for patch in 0..L1_PATCHES {
                let row = patch / 8;
                let col = patch % 8;
                let center = (3 * row + 4) * 30 + 3 * col + 4;
                let dst = dst_base + 2 * patch;
                if actual[dst] != dense_words[col_base + center]
                    || actual[dst + 1] != dense_words[row_base + center]
                {
                    return Err(format!(
                        "ONE X2 L1 hint injection is not exact at direction {direction} patch {patch}"
                    )
                    .into());
                }
            }
            let tail = if direction == 0 { 5_696 } else { 11_396 };
            if actual[tail..tail + 4] != [0; 4] {
                return Err(format!(
                    "ONE X2 L1 hint injection wrote direction {direction} disparity tail"
                )
                .into());
            }
        }

        let l2_config = upload(
            device,
            "ONE X2 L2 hint injection qualifier config",
            &u32_bytes(&[
                129_600,
                133_650,
                267_300,
                271_350,
                2 * L2_PATCHES as u32,
                1_060 + 2 * L2_PATCHES as u32,
                L2_PATCHES as u32,
                Level::Two.patch_cols() as u32,
                Level::Two.cols() as u32,
            ]),
        );
        let l2_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 L2 hint injection qualifier"),
            layout: &self.hint_layout,
            entries: &[
                binding(0, &l2_config),
                binding(1, &dense),
                binding(2, &dynamic),
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.clear_buffer(&dynamic, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.hint_pipeline);
            pass.set_bind_group(0, &l2_bind, &[]);
            pass.dispatch_workgroups(L2_PATCHES.div_ceil(64) as u32, 2, 1);
        }
        self.context.queue().submit([encoder.finish()]);
        let actual = read_buffer_words(&self.context, &dynamic, 2_120)?;
        for direction in 0..2 {
            let (col_base, row_base, dst_base) = if direction == 0 {
                (129_600, 133_650, 2 * L2_PATCHES)
            } else {
                (267_300, 271_350, 1_060 + 2 * L2_PATCHES)
            };
            for patch in 0..L2_PATCHES {
                let row = patch / Level::Two.patch_cols();
                let col = patch % Level::Two.patch_cols();
                let center = (3 * row + 4) * Level::Two.cols() + 3 * col + 4;
                let dst = dst_base + 2 * patch;
                if actual[dst] != dense_words[col_base + center]
                    || actual[dst + 1] != dense_words[row_base + center]
                {
                    return Err(format!(
                        "ONE X2 L2 hint injection is not exact at direction {direction} patch {patch}"
                    )
                    .into());
                }
            }
        }
        Ok(())
    }

    fn qualify_direction_spans(
        &self,
        seeds: &GpuPairedLevelOneInitialBuffer,
    ) -> Result<(), BridgeGpuError> {
        let device = self.context.device();
        scoped_gpu(device, || {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("ONE X2 L1 downstream-span qualifier"),
                source: wgpu::ShaderSource::Wgsl(DOWNSTREAM_BIND_SHADER.into()),
            });
            let minimum = std::num::NonZeroU64::new((L1_PATCHES * 4) as u64);
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ONE X2 L1 downstream-span qualifier"),
                entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: minimum,
                    },
                    count: None,
                }),
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ONE X2 L1 downstream-span qualifier"),
                bind_group_layouts: &[&layout],
                immediate_size: 0,
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ONE X2 L1 downstream-span qualifier"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            let bind = |dcol: wgpu::BufferBinding<'_>, drow: wgpu::BufferBinding<'_>| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("ONE X2 L1 downstream direction span"),
                    layout: &layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Buffer(dcol),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Buffer(drow),
                        },
                    ],
                })
            };
            let a = seeds.direction::<AtoB>();
            let b = seeds.direction::<BtoA>();
            let a_bind = bind(a.dcol(), a.drow());
            let b_bind = bind(b.dcol(), b.drow());
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ONE X2 downstream-span qualification"),
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("ONE X2 downstream-span qualification"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &a_bind, &[]);
                pass.dispatch_workgroups(1, 1, 1);
                pass.set_bind_group(0, &b_bind, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            self.context.queue().submit([encoder.finish()]);
        })
    }
}

fn read_one_word(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Buffer,
) -> Result<u32, Box<dyn Error>> {
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ONE X2 L2 bridge status readback"),
        size: 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ONE X2 L2 bridge status"),
    });
    encoder.copy_buffer_to_buffer(source, 0, &readback, 0, 4);
    let submission = queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (sent, received) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sent.send(result);
    });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    received.recv()??;
    let mapped = slice.get_mapped_range();
    let word = u32::from_ne_bytes(mapped[..4].try_into().unwrap());
    drop(mapped);
    readback.unmap();
    Ok(word)
}

fn read_buffer_words(
    context: &OneXsGpuContext,
    source: &wgpu::Buffer,
    words: usize,
) -> Result<Vec<u32>, Box<dyn Error>> {
    let device = context.device();
    let bytes = u64::try_from(words * 4)?;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ONE X2 constructor qualification readback"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &readback, 0, bytes);
    let submission = context.queue().submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (sent, received) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sent.send(result);
    });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    received.recv()??;
    let mapped = slice.get_mapped_range();
    let answer = mapped
        .chunks_exact(4)
        .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
        .collect();
    drop(mapped);
    readback.unmap();
    Ok(answer)
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn binding<'a>(binding: u32, buffer: &'a wgpu::Buffer) -> wgpu::BindGroupEntry<'a> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}
fn upload(device: &wgpu::Device, label: &str, bytes: &[u8]) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage: wgpu::BufferUsages::STORAGE,
    })
}

fn resident_buffer(device: &wgpu::Device, label: &str, words: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (words * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn encode_disparities(
    dispatch: &ResidentPisDispatch<'_>,
    device: &wgpu::Device,
    _queue: &wgpu::Queue,
    dynamic: &wgpu::Buffer,
    encoder: &mut wgpu::CommandEncoder,
) -> Fallible<()> {
    for (word_base, float_base, name, disparity) in [
        (
            usize::try_from(dispatch.u32s[2])?,
            0usize,
            "A-to-B",
            dispatch.a_disparity,
        ),
        (
            usize::try_from(dispatch.u32s[5])?,
            dispatch.direction_float_words,
            "B-to-A",
            dispatch.b_disparity,
        ),
    ] {
        let header = dispatch
            .u32s
            .get(word_base..word_base + 32)
            .ok_or("ONE X2 resident L1 PIS direction header exceeds dynamic state")?;
        if header[7] == 0 {
            if disparity.is_some() {
                return Err(format!(
                    "ONE X2 resident L1 PIS {name} disparity metadata is unexpected"
                )
                .into());
            }
            continue;
        }
        let disparity_base = float_base
            .checked_add(usize::try_from(header[20])?)
            .ok_or("ONE X2 resident L1 PIS disparity offset overflow")?;
        let values = disparity
            .map(|interval| {
                let (first, second) = interval.endpoints();
                [first[0], first[1], second[0], second[1]]
            })
            .ok_or("ONE X2 resident L1 PIS disparity metadata is absent")?;
        use wgpu::util::DeviceExt;
        let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ONE X2 resident L1 PIS disparity"),
            contents: &f32_bytes(&values),
            usage: wgpu::BufferUsages::COPY_SRC,
        });
        encoder.copy_buffer_to_buffer(&source, 0, dynamic, (disparity_base * 4) as u64, 16);
    }
    Ok(())
}
fn u32_bytes(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}
fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

fn pack_post(post: &LevelTwoPostUpdate) -> (Vec<f32>, Vec<u32>) {
    match post {
        LevelTwoPostUpdate::Cold => (
            vec![0.0; PLANES * L2_PIXELS],
            vec![0; L2_PIXELS.div_ceil(4)],
        ),
        LevelTwoPostUpdate::Warm {
            a_to_b,
            b_to_a,
            motion,
        } => (
            [&*a_to_b.dcol, &*a_to_b.drow, &*b_to_a.dcol, &*b_to_a.drow].concat(),
            motion
                .chunks(4)
                .map(|chunk| {
                    chunk.iter().enumerate().fold(0u32, |word, (byte, value)| {
                        word | (u32::from(*value) << (8 * byte))
                    })
                })
                .collect(),
        ),
    }
}

#[cfg(test)]
fn decode<D: PisDirection>(
    words: &[u32],
) -> Result<DiagnosticLevelOneInitial<D>, BridgeInputError> {
    let (dcol, drow) = words.split_at(L1_PATCHES);
    let flows = dcol
        .iter()
        .zip(drow)
        .map(|(&x, &y)| {
            Flow::new(f32::from_bits(x), f32::from_bits(y))
                .ok_or(BridgeInputError::NonFiniteTerminal)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DiagnosticLevelOneInitial {
        flows: flows.into_boxed_slice(),
        direction: PhantomData,
    })
}

fn compare_words(
    case: &'static str,
    actual: &[u32],
    expected: &[u32],
) -> Result<(), QualificationError> {
    for (word, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
        if actual != expected {
            return Err(QualificationError {
                case,
                word,
                expected,
                actual,
            });
        }
    }
    Ok(())
}

fn cpu_words(
    a_terminal: &CpuUploadedTerminalL2<AtoB>,
    b_terminal: &CpuUploadedTerminalL2<BtoA>,
    images: &LevelTwoImages,
    post: &LevelTwoPostUpdate,
) -> Vec<u32> {
    let terminals = [
        &*a_terminal.dcol,
        &*a_terminal.drow,
        &*b_terminal.dcol,
        &*b_terminal.drow,
    ];
    let (retained, motion) = pack_post(post);
    let warm = matches!(post, LevelTwoPostUpdate::Warm { .. });
    let mut answer = Vec::with_capacity(PLANES * L1_PATCHES);
    for direction in 0..2 {
        for component in 0..2 {
            for patch_row in 0..178 {
                let target_row = patch_row * 3 + 4;
                let (top, bottom, tw, bw) = vertical_sample(target_row);
                for patch_col in 0..8 {
                    let target_col = patch_col * 3 + 4;
                    let top = horizontal_cpu(
                        direction, component, top, target_col, terminals, images, warm, &retained,
                        &motion,
                    );
                    let bottom = horizontal_cpu(
                        direction, component, bottom, target_col, terminals, images, warm,
                        &retained, &motion,
                    );
                    answer.push((top.mul_add(tw, bottom * bw) * 2.0).to_bits());
                }
            }
        }
    }
    answer
}

fn vertical_sample(target: usize) -> (usize, usize, f32, f32) {
    if target.is_multiple_of(2) {
        (target / 2 - 1, target / 2, 0.25, 0.75)
    } else {
        (target / 2, target / 2 + 1, 0.75, 0.25)
    }
}

#[allow(clippy::too_many_arguments)]
fn horizontal_cpu(
    direction: usize,
    component: usize,
    row: usize,
    target: usize,
    terminals: [&[f32]; 4],
    images: &LevelTwoImages,
    warm: bool,
    retained: &[f32],
    motion: &[u32],
) -> f32 {
    let (left, right, lw, rw) = if target.is_multiple_of(2) {
        (target / 2 - 1, target / 2, 0.25, 0.75)
    } else {
        (target / 2, target / 2 + 1, 0.75, 0.25)
    };
    let left = dense_cpu(
        direction, component, row, left, terminals, images, warm, retained, motion,
    ) * lw;
    let right = dense_cpu(
        direction, component, row, right, terminals, images, warm, retained, motion,
    ) * rw;
    left + right
}

#[allow(clippy::too_many_arguments)]
fn dense_cpu(
    direction: usize,
    component: usize,
    row: usize,
    col: usize,
    terminals: [&[f32]; 4],
    images: &LevelTwoImages,
    warm: bool,
    retained: &[f32],
    motion: &[u32],
) -> f32 {
    let source = if direction == 0 { &images.a } else { &images.b };
    let target = if direction == 0 { &images.b } else { &images.a };
    let first_row = (row + 1).saturating_sub(8).div_ceil(3).min(87);
    let last_row = (row / 3).min(87);
    let first_col = (col + 1).saturating_sub(8).div_ceil(3).min(2);
    let last_col = (col / 3).min(2);
    let mut sums = [0.0f32; 2];
    let mut weight_sum = 0.0f32;
    for pr in first_row..=last_row {
        for pc in first_col..=last_col {
            let patch = pr * 3 + pc;
            let dcol = terminals[direction * 2][patch];
            let drow = terminals[direction * 2 + 1][patch];
            let sampled = bilinear(target, row as f32 + drow, col as f32 + dcol);
            let difference = (f32::from(source[row * L2_COLS + col]) - sampled).abs();
            let reciprocal = 1.0 / difference;
            let weight = if difference > 1.0 { reciprocal } else { 1.0 };
            sums[0] = weight.mul_add(dcol, sums[0]);
            sums[1] = weight.mul_add(drow, sums[1]);
            weight_sum += weight;
        }
    }
    let pixel = row * L2_COLS + col;
    let fresh = sums[component] / weight_sum;
    if warm {
        let motion_code = (motion[pixel / 4] >> (8 * (pixel % 4))) & 255;
        let weight = if motion_code == 0 {
            f32::from_bits(0x3ca3_d70a)
        } else {
            1.0
        };
        fresh.mul_add(
            weight,
            retained[(direction * 2 + component) * L2_PIXELS + pixel] * (1.0 - weight),
        )
    } else {
        fresh
    }
}

fn bilinear(image: &[u8], row: f32, col: f32) -> f32 {
    let epsilon = f32::from_bits(0xba83_126f);
    let row = row.max(0.0).min((L2_ROWS - 1) as f32 + epsilon);
    let col = col.max(0.0).min((L2_COLS - 1) as f32 + epsilon);
    let r0 = row as usize;
    let c0 = col as usize;
    let r1 = r0 + 1;
    let c1 = c0 + 1;
    let rf = row - r0 as f32;
    let cf = col - c0 as f32;
    let ri = r1 as f32 - row;
    let ci = c1 as f32 - col;
    let bl = (ci * rf) * f32::from(image[r1 * L2_COLS + c0]);
    let bottom = (cf * rf).mul_add(f32::from(image[r1 * L2_COLS + c1]), bl);
    let right = (cf * ri).mul_add(f32::from(image[r0 * L2_COLS + c1]), bottom);
    (ci * ri).mul_add(f32::from(image[r0 * L2_COLS + c0]), right)
}

struct Fixture {
    a_terminal: CpuUploadedTerminalL2<AtoB>,
    b_terminal: CpuUploadedTerminalL2<BtoA>,
    images: LevelTwoImages,
    cold: LevelTwoPostUpdate,
    warm: LevelTwoPostUpdate,
}

fn fixture() -> Result<Fixture, BridgeInputError> {
    let plane = |bias: f32| {
        (0..L2_PATCHES)
            .map(|i| bias + (i % 17) as f32 * 0.03125 - (i % 5) as f32 * 0.0078125)
            .collect()
    };
    let mut a_drow: Vec<f32> = plane(0.1875);
    let mut b_drow: Vec<f32> = plane(-0.21875);
    for value in &mut a_drow[258..] {
        *value = 3.25;
    }
    for value in &mut b_drow[258..] {
        *value = 2.75;
    }
    let a_terminal = CpuUploadedTerminalL2::new(plane(-0.375), a_drow)?;
    let b_terminal = CpuUploadedTerminalL2::new(plane(0.3125), b_drow)?;
    let a = (0..L2_PIXELS)
        .map(|i| ((i * 37 + i / L2_COLS * 11) % 251) as u8)
        .collect();
    let b = (0..L2_PIXELS)
        .map(|i| ((i * 19 + i / 7 * 23 + 17) % 253) as u8)
        .collect();
    let images = LevelTwoImages::new(a, b)?;
    let retained_a = selected_retained_level::<AtoB>(-1.25)?;
    let retained_b = selected_retained_level::<BtoA>(1.75)?;
    let motion = (0..L2_PIXELS)
        .map(|i| {
            if i % 11 == 0 {
                0
            } else if i % 3 == 0 {
                255
            } else {
                1
            }
        })
        .collect();
    let warm = LevelTwoPostUpdate::warm(retained_a, retained_b, motion)?;
    Ok(Fixture {
        a_terminal,
        b_terminal,
        images,
        cold: LevelTwoPostUpdate::Cold,
        warm,
    })
}

fn selected_retained_pyramids<D: PisDirection>(bias: f32) -> RetainedPublicPyramids<D> {
    let dcol = (0..super::ROWS * super::COLS)
        .map(|i| bias + (i % 37) as f32 * 0.00390625)
        .collect();
    let drow = (0..super::ROWS * super::COLS)
        .map(|i| -bias * 0.5 + (i % 31) as f32 * 0.0078125)
        .collect();
    RetainedPublicPyramids::from_public(
        PublicDenseField::from_row_major_components(dcol, drow).unwrap(),
    )
}

fn selected_retained_level<D: PisDirection>(
    bias: f32,
) -> Result<RetainedLevelTwo<D>, BridgeInputError> {
    let pyramids = selected_retained_pyramids::<D>(bias);
    let (dcol, drow) = pyramids.l2_bridge_qualification_components();
    RetainedLevelTwo::new(dcol.to_vec(), drow.to_vec())
}

const SEED_INJECTION_SHADER: &str = r#"
@group(0) @binding(0) var<storage, read> config: array<u32>;
@group(0) @binding(1) var<storage, read> seeds: array<u32>;
@group(0) @binding(2) var<storage, read_write> dynamic: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let patch_index = id.x;
    let direction = id.y;
    if (patch_index >= 1424u || direction >= 2u) { return; }
    let stride = config[0];
    let src_index = direction * 2u * stride + patch_index;
    let dst_index = select(0u, config[1], direction == 1u) + 2u * patch_index;
    dynamic[dst_index] = seeds[src_index];
    dynamic[dst_index + 1u] = seeds[src_index + stride];
}
"#;

const HINT_INJECTION_SHADER: &str = r#"
@group(0) @binding(0) var<storage, read> config: array<u32>;
@group(0) @binding(1) var<storage, read> dense_hints: array<u32>;
@group(0) @binding(2) var<storage, read_write> dynamic: array<u32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let patch_index = id.x;
    let direction = id.y;
    if (patch_index >= config[6] || direction >= 2u) { return; }
    let patch_row = patch_index / config[7];
    let patch_col = patch_index % config[7];
    let center = (3u * patch_row + 4u) * config[8] + 3u * patch_col + 4u;
    let config_base = 2u * direction;
    let dst = config[4u + direction] + 2u * patch_index;
    dynamic[dst] = dense_hints[config[config_base] + center];
    dynamic[dst + 1u] = dense_hints[config[config_base + 1u] + center];
}
"#;

const SHADER: &str = r#"
@group(0) @binding(0) var<storage, read> config: array<u32>;
@group(0) @binding(1) var<storage, read> images: array<u32>;
@group(0) @binding(2) var<storage, read> terminal: array<f32>;
@group(0) @binding(3) var<storage, read> retained: array<f32>;
@group(0) @binding(4) var<storage, read> motion: array<u32>;
@group(0) @binding(5) var<storage, read_write> seeds: array<u32>;
@group(0) @binding(6) var<storage, read_write> finite_status: array<atomic<u32>>;

fn materialize(value: f32) -> f32 { return bitcast<f32>(bitcast<u32>(value) ^ config[1]); }
fn mul_rn(a: f32, b: f32) -> f32 { return materialize(fma(a, b, -0.0)); }
fn add_rn(a: f32, b: f32) -> f32 { return materialize(fma(a, 1.0, b)); }
fn sub_rn(a: f32, b: f32) -> f32 { return materialize(fma(-1.0, b, a)); }
fn fma_rn(a: f32, b: f32, c: f32) -> f32 { return materialize(fma(a, b, c)); }

fn div_f32_bits(a: u32, b: u32) -> u32 {
    let sign = (a ^ b) & 0x80000000u;
    let a_abs = a & 0x7fffffffu; let b_abs = b & 0x7fffffffu;
    let a_exp = a_abs >> 23u; let b_exp = b_abs >> 23u;
    let a_frac = a_abs & 0x007fffffu; let b_frac = b_abs & 0x007fffffu;
    if a_exp == 0xffu && a_frac != 0u { return a | 0x00400000u; }
    if b_exp == 0xffu && b_frac != 0u { return b | 0x00400000u; }
    if (a_abs == 0u && b_abs == 0u) || (a_exp == 0xffu && b_exp == 0xffu) { return 0xffc00000u; }
    if b_abs == 0u || a_exp == 0xffu { return sign | 0x7f800000u; }
    if a_abs == 0u || b_exp == 0xffu { return sign; }
    var ma = a_frac; var mb = b_frac;
    var ea = i32(a_exp) - 127; var eb = i32(b_exp) - 127;
    if a_exp == 0u { let top = 31u - countLeadingZeros(a_frac); ma = a_frac << (23u - top); ea = i32(top) - 149; }
    else { ma |= 0x00800000u; }
    if b_exp == 0u { let top = 31u - countLeadingZeros(b_frac); mb = b_frac << (23u - top); eb = i32(top) - 149; }
    else { mb |= 0x00800000u; }
    var remainder = ma; var quotient_exponent = ea - eb;
    if remainder < mb { remainder <<= 1u; quotient_exponent -= 1; }
    var quotient = 0u;
    for (var step = 0u; step < 24u; step++) {
        let bit = 23u - step;
        if remainder >= mb { remainder -= mb; quotient |= 1u << bit; }
        if step != 23u { remainder <<= 1u; }
    }
    if quotient_exponent >= -126 {
        let twice_remainder = remainder << 1u;
        if twice_remainder > mb || (twice_remainder == mb && (quotient & 1u) != 0u) { quotient += 1u; }
        if quotient == 0x01000000u { quotient = 0x00800000u; quotient_exponent += 1; }
        if quotient_exponent > 127 { return sign | 0x7f800000u; }
        return sign | (u32(quotient_exponent + 127) << 23u) | (quotient & 0x007fffffu);
    }
    let shift = u32(-126 - quotient_exponent);
    if shift >= 25u { return sign; }
    var subnormal = quotient >> shift;
    let mask = (1u << shift) - 1u; let low = quotient & mask; let half = 1u << (shift - 1u);
    if low > half || (low == half && (remainder != 0u || (subnormal & 1u) != 0u)) { subnormal += 1u; }
    return sign | subnormal;
}
fn div_rn(a: f32, b: f32) -> f32 { return bitcast<f32>(div_f32_bits(bitcast<u32>(a), bitcast<u32>(b))); }

fn bilinear(direction: u32, row_in: f32, col_in: f32) -> f32 {
    let upper_row = add_rn(269.0, bitcast<f32>(0xba83126fu));
    let upper_col = add_rn(14.0, bitcast<f32>(0xba83126fu));
    var row = select(row_in, 0.0, row_in < 0.0);
    var col = select(col_in, 0.0, col_in < 0.0);
    row = select(row, upper_row, upper_row < row);
    col = select(col, upper_col, upper_col < col);
    let r0 = u32(row); let c0 = u32(col); let r1 = r0 + 1u; let c1 = c0 + 1u;
    let rf = sub_rn(row, f32(r0)); let cf = sub_rn(col, f32(c0));
    let ri = sub_rn(f32(r1), row); let ci = sub_rn(f32(c1), col);
    let base = select(config[6], config[7], direction == 0u);
    let bl_weight = mul_rn(ci, rf);
    let bl = mul_rn(bl_weight, f32(images[base + r1 * 15u + c0]));
    let br_weight = mul_rn(cf, rf);
    let bottom = fma_rn(br_weight, f32(images[base + r1 * 15u + c1]), bl);
    let tr_weight = mul_rn(cf, ri);
    let right = fma_rn(tr_weight, f32(images[base + r0 * 15u + c1]), bottom);
    let tl_weight = mul_rn(ci, ri);
    return fma_rn(tl_weight, f32(images[base + r0 * 15u + c0]), right);
}

fn dense(direction: u32, component: u32, row: u32, col: u32) -> f32 {
    let first_row = min(select(0u, (row + 1u - 8u + 2u) / 3u, row + 1u > 8u), 87u);
    let last_row = min(row / 3u, 87u);
    let first_col = min(select(0u, (col + 1u - 8u + 2u) / 3u, col + 1u > 8u), 2u);
    let last_col = min(col / 3u, 2u);
    let source_base = select(config[7], config[6], direction == 0u);
    let source = f32(images[source_base + row * 15u + col]);
    var sums = vec2<f32>(0.0); var weight_sum = 0.0;
    for (var pr = first_row; pr <= last_row; pr++) {
        for (var pc = first_col; pc <= last_col; pc++) {
            let patch_index = pr * 3u + pc;
            let terminal_base = select(config[5], config[4], direction == 0u);
            let dcol = terminal[terminal_base + 2u * patch_index];
            let drow = terminal[terminal_base + 2u * patch_index + 1u];
            let difference = abs(sub_rn(source, bilinear(direction, add_rn(f32(row), drow), add_rn(f32(col), dcol))));
            let reciprocal = div_rn(1.0, difference);
            let weight = select(1.0, reciprocal, difference > 1.0);
            sums.x = fma_rn(weight, dcol, sums.x);
            sums.y = fma_rn(weight, drow, sums.y);
            weight_sum = add_rn(weight_sum, weight);
        }
    }
    let pixel = row * 15u + col;
    var fresh = div_rn(select(sums.x, sums.y, component == 1u), weight_sum);
    if (config[0] != 0u) {
        let packed_motion = motion[pixel / 4u];
        let motion_code = (packed_motion >> (8u * (pixel % 4u))) & 255u;
        let weight = select(1.0, bitcast<f32>(0x3ca3d70au), motion_code == 0u);
        let old = retained[(direction * 2u + component) * 4050u + pixel];
        fresh = fma_rn(weight, fresh, mul_rn(old, sub_rn(1.0, weight)));
    }
    return fresh;
}

fn horizontal(direction: u32, component: u32, row: u32, target_col: u32) -> f32 {
    var left: u32; var right: u32; var lw: f32; var rw: f32;
    if (target_col % 2u == 0u) { left = target_col / 2u - 1u; right = target_col / 2u; lw = 0.25; rw = 0.75; }
    else { left = target_col / 2u; right = target_col / 2u + 1u; lw = 0.75; rw = 0.25; }
    let left_term = mul_rn(dense(direction, component, row, left), lw);
    let right_term = mul_rn(dense(direction, component, row, right), rw);
    return add_rn(left_term, right_term);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let patch_index = id.x; let direction = id.y;
    if (patch_index >= 1424u || direction >= 2u) { return; }
    let patch_row = patch_index / 8u; let patch_col = patch_index % 8u;
    let target_row = patch_row * 3u + 4u; let target_col = patch_col * 3u + 4u;
    var top: u32; var bottom: u32; var tw: f32; var bw: f32;
    if (target_row % 2u == 0u) { top = target_row / 2u - 1u; bottom = target_row / 2u; tw = 0.25; bw = 0.75; }
    else { top = target_row / 2u; bottom = target_row / 2u + 1u; tw = 0.75; bw = 0.25; }
    for (var component = 0u; component < 2u; component++) {
        let top_value = horizontal(direction, component, top, target_col);
        let bottom_value = horizontal(direction, component, bottom, target_col);
        let bottom_term = mul_rn(bottom_value, bw);
        let value = mul_rn(fma_rn(top_value, tw, bottom_term), 2.0);
        if ((bitcast<u32>(value) & 0x7f800000u) == 0x7f800000u) {
            atomicMin(&finite_status[0], (direction * 2u + component) * 1424u + patch_index);
        }
        seeds[(direction * 2u + component) * config[2] + patch_index] = bitcast<u32>(value);
    }
}
"#;

const DOWNSTREAM_BIND_SHADER: &str = r#"
@group(0) @binding(0) var<storage, read> dcol: array<f32>;
@group(0) @binding(1) var<storage, read> drow: array<f32>;
@compute @workgroup_size(1)
fn main() {
    let consumed = dcol[0] + dcol[1423] + drow[0] + drow[1423];
    if (consumed == bitcast<f32>(0x7fc00000u)) { return; }
}
"#;

#[cfg(test)]
mod tests {
    use super::super::{GpuPisFrontEnd, qualification_fixture};
    use super::*;
    use crate::flow::one_xs::pis::{HintGrid, InitialGrid, solve_with_descent_admission};
    use crate::flow::one_xs::scalar::{ColdInputs, LevelInputs, MaskPyramid};

    #[test]
    fn l1_ordinal_is_derived_only_from_the_exact_l2_receipt() {
        for (calculation, expected) in [
            (0, GpuL1Ordinal::Cold0),
            (1, GpuL1Ordinal::Cold1),
            (2, GpuL1Ordinal::Cold2),
        ] {
            let ordinal = GpuL1Ordinal::after_l2(PairSolveStage::Cold {
                calculation,
                level: Level::Two,
            })
            .unwrap();
            assert_eq!(ordinal, expected);
            assert_eq!(
                ordinal.l1_stage(),
                PairSolveStage::Cold {
                    calculation,
                    level: Level::One,
                }
            );
        }
        assert!(
            GpuL1Ordinal::after_l2(PairSolveStage::Cold {
                calculation: 3,
                level: Level::Two,
            })
            .is_err()
        );
        assert!(
            GpuL1Ordinal::after_l2(PairSolveStage::Cold {
                calculation: 0,
                level: Level::One,
            })
            .is_err()
        );
    }
    use crate::flow::one_xs::pis::PatchGrid;
    use crate::flow::one_xs::pis::gpu::GpuPisFlight;
    use crate::flow::one_xs::pis::gpu::{
        GpuPisDynamicDirection, GpuPisDynamicStage, GpuPisPipeline,
    };
    use crate::flow::one_xs::pis::{CostMode, DescentAdmission};
    use crate::flow::one_xs::post_update::MotionLevel;
    use crate::flow::one_xs::scalar::PairSolveStage;
    use crate::flow::one_xs::{LensPair, dense, l2_seed, post_update};
    use crate::flow::one_xs_belt_gpu::resident_qualification_fixture;
    use kjerag_media::FrameStamp;
    use std::future::Future;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::time::Duration;

    struct DropProbe {
        state: Arc<AtomicU8>,
        dropped: mpsc::Sender<u8>,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            let _ = self.dropped.send(self.state.load(Ordering::SeqCst));
        }
    }

    struct OrderingPost {
        inner: ColdResidentLevelTwoPost,
        state: Arc<AtomicU8>,
        dropped: mpsc::Sender<u8>,
    }

    impl Drop for OrderingPost {
        fn drop(&mut self) {
            let _ = self.dropped.send(self.state.load(Ordering::SeqCst));
        }
    }

    impl super::super::resident_l2_post_seal::Sealed for OrderingPost {}

    impl GpuResidentLevelTwoPost for OrderingPost {
        fn context(&self) -> &OneXsGpuContext {
            self.inner.context()
        }

        fn is_warm(&self) -> bool {
            false
        }

        fn admissions(&self) -> GpuAdmittedSnapshot {
            GpuAdmittedSnapshot::qualification_every_patch()
        }

        fn initialize(&self, encoder: &mut wgpu::CommandEncoder) {
            self.inner.initialize(encoder);
        }

        fn bind(
            &self,
            device: &wgpu::Device,
            layout: &wgpu::BindGroupLayout,
            config: &wgpu::Buffer,
            images: &wgpu::Buffer,
            terminal: &wgpu::Buffer,
            output: &wgpu::Buffer,
            validity: &wgpu::Buffer,
        ) -> wgpu::BindGroup {
            self.inner
                .bind(device, layout, config, images, terminal, output, validity)
        }

        fn bind_l1_hint_fill(
            &self,
            device: &wgpu::Device,
            layout: &wgpu::BindGroupLayout,
            config: &wgpu::Buffer,
            dynamic: &wgpu::Buffer,
        ) -> Option<wgpu::BindGroup> {
            self.inner
                .bind_l1_hint_fill(device, layout, config, dynamic)
        }
    }

    fn ordering_terminal(
        context: &OneXsGpuContext,
        state: Arc<AtomicU8>,
        generation: u64,
    ) -> GpuPreparedTerminal<Arc<()>> {
        let flight = GpuPisFlight {
            generation,
            frame: FrameStamp::for_test(generation, Duration::from_millis(generation), None),
        };
        let (mut belts, _) =
            resident_qualification_fixture(context.device(), context.queue(), Arc::new(()), flight)
                .unwrap();
        belts.observe_completion(state);
        let masks = qualification_fixture().1;
        GpuPisFrontEnd::new(context.clone())
            .unwrap()
            .prepare(belts, &masks)
            .unwrap()
            .submit_pis_stage(
                &GpuPisPipeline::new(context.clone()).unwrap(),
                GpuPisDynamicStage {
                    stage: PairSolveStage::Cold {
                        calculation: 0,
                        level: Level::Two,
                    },
                    a_to_b: resident_direction(),
                    b_to_a: resident_direction(),
                },
            )
            .unwrap()
    }

    #[test]
    fn foreign_l2_and_l1_contexts_retire_carrier_before_post_release() {
        let (context, foreign, adapter) = gpu_context_pair()
            .expect("two Vulkan devices required for resident drop-order qualification");
        let bridge = GpuL2PostPisBridge::new(context.clone()).unwrap();
        let foreign_bridge = GpuL2PostPisBridge::new(foreign.clone()).unwrap();

        for (generation, foreign_part) in [(81, "bridge"), (82, "post")] {
            let state = Arc::new(AtomicU8::new(0));
            let (dropped, answer) = mpsc::channel();
            let terminal = ordering_terminal(&context, Arc::clone(&state), generation);
            let (chosen_bridge, inner) = if foreign_part == "bridge" {
                (&foreign_bridge, bridge.cold_post())
            } else {
                (&bridge, foreign_bridge.cold_post())
            };
            let post = OrderingPost {
                inner,
                state: Arc::clone(&state),
                dropped,
            };
            assert!(terminal.submit_l2_bridge(chosen_bridge, post).is_err());
            assert_eq!(
                answer.recv().unwrap(),
                2,
                "foreign {foreign_part} released post first"
            );
        }

        let state = Arc::new(AtomicU8::new(0));
        let (dropped, answer) = mpsc::channel();
        let output = ordering_terminal(&context, Arc::clone(&state), 83)
            .submit_l2_bridge(
                &bridge,
                OrderingPost {
                    inner: bridge.cold_post(),
                    state: Arc::clone(&state),
                    dropped,
                },
            )
            .unwrap();
        let foreign_solver = GpuPisPipeline::new(foreign).unwrap();
        let disparity = DisparityInterval::new([-8.0, -8.0], [8.0, 8.0]);
        let costs = || vec![CostMode::Unweighted; Level::One.patch_rows()].into_boxed_slice();
        assert!(
            output
                .submit_l1_pis(
                    &bridge,
                    &foreign_solver,
                    GpuL1Controls::resident(costs(), disparity, costs(), disparity),
                )
                .is_err()
        );
        assert_eq!(
            answer.recv().unwrap(),
            2,
            "foreign L1 solver released post first"
        );
        eprintln!("ONE X2 resident foreign-context drop order passed on {adapter}");
    }

    #[test]
    fn input_boundaries_are_typed_and_shape_checked() {
        assert!(matches!(
            CpuUploadedTerminalL2::<AtoB>::new(vec![0.0; 1], vec![0.0; L2_PATCHES]),
            Err(BridgeInputError::Shape { .. })
        ));
        assert!(matches!(
            CpuUploadedTerminalL2::<BtoA>::new(vec![f32::NAN; L2_PATCHES], vec![0.0; L2_PATCHES]),
            Err(BridgeInputError::NonFiniteTerminal)
        ));
        let receipt = receipt(Level::One);
        assert_eq!(
            GpuL2BridgeReceipt::from_terminal(receipt),
            Err(BridgeInputError::WrongReceiptLevel(Level::One))
        );
    }

    #[test]
    fn qualification_cpu_twin_matches_selected_cold_stages() {
        let fixture = fixture().unwrap();
        let expected = cpu_words(
            &fixture.a_terminal,
            &fixture.b_terminal,
            &fixture.images,
            &fixture.cold,
        );
        compare_selected_cold::<AtoB>(&fixture.a_terminal, &fixture.images, &expected, 0);
        compare_selected_cold::<BtoA>(
            &fixture.b_terminal,
            &fixture.images,
            &expected,
            2 * L1_PATCHES,
        );
    }

    #[test]
    fn qualification_cpu_twin_matches_selected_warm_stages() {
        let fixture = fixture().unwrap();
        let expected = cpu_words(
            &fixture.a_terminal,
            &fixture.b_terminal,
            &fixture.images,
            &fixture.warm,
        );
        let motion = match &fixture.warm {
            LevelTwoPostUpdate::Warm { motion, .. } => motion,
            LevelTwoPostUpdate::Cold => unreachable!(),
        };
        compare_selected_warm::<AtoB>(
            &fixture.a_terminal,
            &fixture.images,
            &expected,
            0,
            selected_retained_pyramids(-1.25),
            motion,
        );
        compare_selected_warm::<BtoA>(
            &fixture.b_terminal,
            &fixture.images,
            &expected,
            2 * L1_PATCHES,
            selected_retained_pyramids(1.75),
            motion,
        );
    }

    fn compare_selected_warm<D: PisDirection>(
        terminal: &CpuUploadedTerminalL2<D>,
        images: &LevelTwoImages,
        expected: &[u32],
        base: usize,
        retained: RetainedPublicPyramids<D>,
        motion: &[u8],
    ) {
        let directed = dense::DirectedImages::<D>::from_native_order(
            Level::Two,
            LensPair {
                a: &images.a,
                b: &images.b,
            },
        )
        .unwrap();
        let patches = PatchGrid::<D>::from_row_major_components(
            Level::Two,
            terminal.dcol.to_vec(),
            terminal.drow.to_vec(),
        )
        .unwrap();
        let field = dense::densify_coarse(&directed, patches).unwrap();
        let post = post_update::update_without_variational_with_retained(
            field,
            &retained,
            MotionLevel::from_bytes(Level::Two, motion.to_vec()).unwrap(),
        )
        .unwrap();
        let selected = l2_seed::into_l1_initial_grid(post).unwrap();
        for (patch, flow) in selected.flows().iter().enumerate() {
            assert_eq!(flow.dcol().to_bits(), expected[base + patch]);
            assert_eq!(flow.drow().to_bits(), expected[base + L1_PATCHES + patch]);
        }
    }

    fn compare_selected_cold<D: PisDirection>(
        terminal: &CpuUploadedTerminalL2<D>,
        images: &LevelTwoImages,
        expected: &[u32],
        base: usize,
    ) {
        let directed = dense::DirectedImages::<D>::from_native_order(
            Level::Two,
            LensPair {
                a: &images.a,
                b: &images.b,
            },
        )
        .unwrap();
        let patches = PatchGrid::<D>::from_row_major_components(
            Level::Two,
            terminal.dcol.to_vec(),
            terminal.drow.to_vec(),
        )
        .unwrap();
        let field = dense::densify_coarse(&directed, patches).unwrap();
        let post = post_update::preserve_without_variational_or_retained(field);
        let selected = l2_seed::into_l1_initial_grid(post).unwrap();
        for (patch, flow) in selected.flows().iter().enumerate() {
            assert_eq!(flow.dcol().to_bits(), expected[base + patch]);
            assert_eq!(flow.drow().to_bits(), expected[base + L1_PATCHES + patch]);
        }
    }

    fn assert_paired_terminal_words(label: &str, level: Level, actual: &[u32], expected: &[u32]) {
        assert_eq!(actual.len(), expected.len(), "{label} word count");
        if let Some(word) = actual
            .iter()
            .zip(expected)
            .position(|(actual, expected)| actual != expected)
        {
            let direction_words = 2 * level.patches();
            let direction = if word < direction_words {
                "AtoB"
            } else {
                "BtoA"
            };
            let local = word % direction_words;
            panic!(
                "{label} first differs at {direction} patch {} component {}: actual {:#010x}, expected {:#010x}",
                local / 2,
                if local.is_multiple_of(2) {
                    "dcol"
                } else {
                    "drow"
                },
                actual[word],
                expected[word],
            );
        }
    }

    #[test]
    fn constructor_qualifies_production_shader_cold_and_warm_bit_exact() {
        let (context, name) = gpu().expect("Vulkan GPU required for L2 bridge qualification");
        eprintln!("ONE X2 L2 bridge qualification adapter: {name}");
        let limits = context.device().limits();
        eprintln!(
            "ONE X2 L2 bridge limits: min_storage_buffer_offset_alignment={}, max_storage_buffer_binding_size={}",
            limits.min_storage_buffer_offset_alignment, limits.max_storage_buffer_binding_size,
        );
        let bridge = GpuL2PostPisBridge::new(context.clone()).expect("production shader builds");
        eprintln!(
            "ONE X2 L2 bridge qualified plane stride: {} bytes",
            bridge.plane_stride,
        );
        let fixture = fixture().unwrap();
        let output = bridge
            .submit_qualification(
                &fixture.a_terminal,
                &fixture.b_terminal,
                &fixture.images,
                &fixture.warm,
            )
            .expect("qualified production entry point runs");
        let diagnostic = output.readback_diagnostic().unwrap();
        assert_eq!(diagnostic.a_to_b.flows().len(), L1_PATCHES);
        assert_eq!(diagnostic.b_to_a.flows().len(), L1_PATCHES);
    }

    #[test]
    fn sealed_terminal_advances_the_one_lease_through_l2() {
        let (context, adapter) = gpu().expect("Vulkan GPU required for resident L2 chain");
        let front = GpuPisFrontEnd::new(context.clone()).unwrap();
        let pis = GpuPisPipeline::new(context.clone()).unwrap();
        let bridge = GpuL2PostPisBridge::new(context.clone()).unwrap();
        let state = Arc::new(AtomicU8::new(0));
        let (dropped, answer) = mpsc::channel();
        let flight = GpuPisFlight {
            generation: 29,
            frame: FrameStamp::for_test(31, Duration::from_millis(900), None),
        };
        let (mut belts, blurred) = resident_qualification_fixture(
            context.device(),
            context.queue(),
            DropProbe {
                state: Arc::clone(&state),
                dropped,
            },
            flight,
        )
        .unwrap_or_else(|error| panic!("resident L2 producer failed on {adapter}: {error}"));
        belts.observe_completion(Arc::clone(&state));
        let masks = qualification_fixture().1;
        let (a_input, b_input) = l1_oracle_inputs(&blurred, &masks);
        let prepared = front.prepare(belts, &masks).unwrap();
        let terminal = pis
            .submit_prepared(
                prepared,
                GpuPisDynamicStage {
                    stage: PairSolveStage::Cold {
                        calculation: 0,
                        level: Level::Two,
                    },
                    a_to_b: resident_direction(),
                    b_to_a: resident_direction(),
                },
            )
            .unwrap();
        assert_eq!(state.load(Ordering::SeqCst), 0, "PIS handoff polled");
        let output = terminal
            .submit_l2_bridge(&bridge, bridge.cold_post())
            .unwrap();
        assert_eq!(state.load(Ordering::SeqCst), 0, "L2 handoff polled");
        assert!(matches!(answer.try_recv(), Err(mpsc::TryRecvError::Empty)));
        let terminal = output
            .submit_l1_pis(
                &bridge,
                &pis,
                resident_l1_controls(
                    a_input.disparity_interval().unwrap(),
                    b_input.disparity_interval().unwrap(),
                ),
            )
            .unwrap();
        assert_eq!(state.load(Ordering::SeqCst), 0, "L1 handoff polled");
        terminal.acknowledge_qualified_for_test().unwrap();
        assert_eq!(
            state.load(Ordering::SeqCst),
            2,
            "terminal wait was not exact"
        );
        assert_eq!(answer.recv().unwrap(), 2, "owner preceded L2 completion");
    }

    #[test]
    fn resident_seed_injection_matches_actual_l1_cpu_terminal_bits() {
        let (context, adapter) = gpu().expect("Vulkan GPU required for resident L1 terminal twin");
        let front = GpuPisFrontEnd::new(context.clone()).unwrap();
        let pis = GpuPisPipeline::new(context.clone()).unwrap();
        let bridge = GpuL2PostPisBridge::new(context.clone()).unwrap();
        let flight = GpuPisFlight {
            generation: 41,
            frame: FrameStamp::for_test(43, Duration::from_millis(1_100), None),
        };
        let (belts, blurred) =
            resident_qualification_fixture(context.device(), context.queue(), Arc::new(()), flight)
                .unwrap_or_else(|error| {
                    panic!("resident L1 producer failed on {adapter}: {error}")
                });
        let masks = qualification_fixture().1;
        let retained = ColdInputs::from_blurred_belts_and_masks(blurred, masks.clone());
        let pyramid = MaskPyramid::build(&retained);
        let l2_modes = vec![CostMode::Unweighted; Level::Two.patch_rows()];
        let a_l2_input = LevelInputs::build::<AtoB>(&retained, &pyramid, Level::Two)
            .resident_l2_oracle_input::<AtoB>(Level::Two, l2_modes.clone());
        let b_l2_input = LevelInputs::build::<BtoA>(&retained, &pyramid, Level::Two)
            .resident_l2_oracle_input::<BtoA>(Level::Two, l2_modes.clone());
        let prepared = front.prepare(belts, &masks).unwrap();
        let output = prepared
            .submit_resident_l2_bridge(
                &pis,
                &bridge,
                GpuL2Controls::resident(
                    l2_modes.clone().into_boxed_slice(),
                    a_l2_input.disparity_interval().unwrap(),
                    l2_modes.into_boxed_slice(),
                    b_l2_input.disparity_interval().unwrap(),
                ),
                PairSolveStage::Cold {
                    calculation: 0,
                    level: Level::Two,
                },
                bridge.cold_post(),
                None,
            )
            .unwrap();
        let zero_l2_hint = vec![Flow::ZERO; L2_PATCHES];
        let expected_l2_a = solve_with_descent_admission(
            &a_l2_input,
            InitialGrid::<AtoB>::coarse_zeros(),
            Some(&HintGrid::<AtoB>::from_row_major(Level::Two, zero_l2_hint.clone()).unwrap()),
            DescentAdmission::EveryPatch,
        )
        .unwrap();
        let expected_l2_b = solve_with_descent_admission(
            &b_l2_input,
            InitialGrid::<BtoA>::coarse_zeros(),
            Some(&HintGrid::<BtoA>::from_row_major(Level::Two, zero_l2_hint).unwrap()),
            DescentAdmission::EveryPatch,
        )
        .unwrap();
        let expected_l2 = expected_l2_a
            .patches()
            .iter()
            .chain(expected_l2_b.patches())
            .flat_map(|patch| [patch.flow().dcol().to_bits(), patch.flow().drow().to_bits()])
            .collect::<Vec<_>>();
        assert_paired_terminal_words(
            "cold resident L2 positive-zero initial and hints",
            Level::Two,
            &output.read_l2_terminal_words_for_test().unwrap(),
            &expected_l2,
        );
        let seeds = output.seeds.readback_diagnostic().unwrap();

        let modes = vec![CostMode::Unweighted; Level::One.patch_rows()];
        let a_input = LevelInputs::build::<AtoB>(&retained, &pyramid, Level::One)
            .resident_l2_oracle_input::<AtoB>(Level::One, modes.clone());
        let b_input = LevelInputs::build::<BtoA>(&retained, &pyramid, Level::One)
            .resident_l2_oracle_input::<BtoA>(Level::One, modes);
        let zero_hint = vec![Flow::ZERO; L1_PATCHES];
        let a_hint = HintGrid::<AtoB>::from_row_major(Level::One, zero_hint.clone()).unwrap();
        let b_hint = HintGrid::<BtoA>::from_row_major(Level::One, zero_hint).unwrap();
        let a_initial =
            InitialGrid::<AtoB>::from_l1_row_major(seeds.a_to_b.flows().to_vec()).unwrap();
        let b_initial =
            InitialGrid::<BtoA>::from_l1_row_major(seeds.b_to_a.flows().to_vec()).unwrap();
        let expected_a = solve_with_descent_admission(
            &a_input,
            a_initial,
            Some(&a_hint),
            DescentAdmission::EveryPatch,
        )
        .unwrap();
        let expected_b = solve_with_descent_admission(
            &b_input,
            b_initial,
            Some(&b_hint),
            DescentAdmission::EveryPatch,
        )
        .unwrap();
        let terminal = output
            .submit_l1_pis(
                &bridge,
                &pis,
                resident_l1_controls(
                    a_input.disparity_interval().unwrap(),
                    b_input.disparity_interval().unwrap(),
                ),
            )
            .unwrap();
        let actual = terminal.read_terminal_words_for_test().unwrap();
        let expected = expected_a
            .patches()
            .iter()
            .chain(expected_b.patches())
            .flat_map(|patch| [patch.flow().dcol().to_bits(), patch.flow().drow().to_bits()])
            .collect::<Vec<_>>();
        assert_eq!(
            actual, expected,
            "actual resident L1 terminal differs from CPU"
        );
        terminal.acknowledge_qualified_for_test().unwrap();
    }

    #[test]
    fn resident_warm_hint_centres_reach_actual_l1_terminal_bits() {
        let (context, adapter) = gpu().expect("Vulkan GPU required for resident L1 hint twin");
        let front = GpuPisFrontEnd::new(context.clone()).unwrap();
        let pis = GpuPisPipeline::new(context.clone()).unwrap();
        let bridge = GpuL2PostPisBridge::new(context.clone()).unwrap();
        let flight = GpuPisFlight {
            generation: 47,
            frame: FrameStamp::for_test(49, Duration::from_millis(1_300), None),
        };
        let (belts, blurred) =
            resident_qualification_fixture(context.device(), context.queue(), Arc::new(()), flight)
                .unwrap_or_else(|error| {
                    panic!("resident warm L1 producer failed on {adapter}: {error}")
                });
        let masks = qualification_fixture().1;
        let retained = ColdInputs::from_blurred_belts_and_masks(blurred.clone(), masks.clone());
        let pyramid = MaskPyramid::build(&retained);
        let l2_modes = vec![CostMode::Unweighted; Level::Two.patch_rows()];
        let a_l2_input = LevelInputs::build::<AtoB>(&retained, &pyramid, Level::Two)
            .resident_l2_oracle_input::<AtoB>(Level::Two, l2_modes.clone());
        let b_l2_input = LevelInputs::build::<BtoA>(&retained, &pyramid, Level::Two)
            .resident_l2_oracle_input::<BtoA>(Level::Two, l2_modes.clone());
        let (a_input, b_input) = l1_oracle_inputs(&blurred, &masks);
        let prepared = front.prepare(belts, &masks).unwrap();

        let mut hint_words = vec![0u32; 275_400];
        for (plane, base, pixels) in [
            (0usize, 0usize, 64_800usize),
            (1, 64_800, 64_800),
            (2, 129_600, 4_050),
            (3, 133_650, 4_050),
            (4, 137_700, 64_800),
            (5, 202_500, 64_800),
            (6, 267_300, 4_050),
            (7, 271_350, 4_050),
        ] {
            for pixel in 0..pixels {
                hint_words[base + pixel] =
                    (plane as f32 * 0.125 + (pixel % 97) as f32 * 0.000_25).to_bits();
            }
        }
        let post = QualificationResidentLevelTwoPost {
            context: context.clone(),
            warm: true,
            retained: upload(
                context.device(),
                "L2 bridge warm terminal retained qualifier",
                &f32_bytes(&vec![0.0; PLANES * L2_PIXELS]),
            ),
            motion: upload(
                context.device(),
                "L2 bridge warm terminal packed motion qualifier",
                &u32_bytes(&vec![0; L2_PIXELS.div_ceil(4)]),
            ),
            hints: Some(upload(
                context.device(),
                "L2 bridge warm terminal hint qualifier",
                &u32_bytes(&hint_words),
            )),
        };
        let output = prepared
            .submit_resident_l2_bridge(
                &pis,
                &bridge,
                GpuL2Controls::resident(
                    l2_modes.clone().into_boxed_slice(),
                    a_l2_input.disparity_interval().unwrap(),
                    l2_modes.into_boxed_slice(),
                    b_l2_input.disparity_interval().unwrap(),
                ),
                PairSolveStage::Warm { level: Level::Two },
                post,
                None,
            )
            .unwrap();
        let l2_hint_grid = |col_base: usize, row_base: usize| {
            (0..L2_PATCHES)
                .map(|patch| {
                    let row = patch / Level::Two.patch_cols();
                    let col = patch % Level::Two.patch_cols();
                    let center = (3 * row + 4) * Level::Two.cols() + 3 * col + 4;
                    Flow::new(
                        f32::from_bits(hint_words[col_base + center]),
                        f32::from_bits(hint_words[row_base + center]),
                    )
                    .unwrap()
                })
                .collect::<Vec<_>>()
        };
        let expected_l2_a = solve_with_descent_admission(
            &a_l2_input,
            InitialGrid::<AtoB>::coarse_zeros(),
            Some(
                &HintGrid::<AtoB>::from_row_major(Level::Two, l2_hint_grid(129_600, 133_650))
                    .unwrap(),
            ),
            DescentAdmission::EveryPatch,
        )
        .unwrap();
        let expected_l2_b = solve_with_descent_admission(
            &b_l2_input,
            InitialGrid::<BtoA>::coarse_zeros(),
            Some(
                &HintGrid::<BtoA>::from_row_major(Level::Two, l2_hint_grid(267_300, 271_350))
                    .unwrap(),
            ),
            DescentAdmission::EveryPatch,
        )
        .unwrap();
        let expected_l2 = expected_l2_a
            .patches()
            .iter()
            .chain(expected_l2_b.patches())
            .flat_map(|patch| [patch.flow().dcol().to_bits(), patch.flow().drow().to_bits()])
            .collect::<Vec<_>>();
        assert_paired_terminal_words(
            "warm resident L2 nonzero hints",
            Level::Two,
            &output.read_l2_terminal_words_for_test().unwrap(),
            &expected_l2,
        );
        let seeds = output.seeds.readback_diagnostic().unwrap();

        let hint_grid = |col_base: usize, row_base: usize| {
            (0..L1_PATCHES)
                .map(|patch| {
                    let row = patch / 8;
                    let col = patch % 8;
                    let center = (3 * row + 4) * 30 + 3 * col + 4;
                    Flow::new(
                        f32::from_bits(hint_words[col_base + center]),
                        f32::from_bits(hint_words[row_base + center]),
                    )
                    .unwrap()
                })
                .collect::<Vec<_>>()
        };
        let a_hint = HintGrid::<AtoB>::from_row_major(Level::One, hint_grid(0, 64_800)).unwrap();
        let b_hint =
            HintGrid::<BtoA>::from_row_major(Level::One, hint_grid(137_700, 202_500)).unwrap();
        let expected_a = solve_with_descent_admission(
            &a_input,
            InitialGrid::<AtoB>::from_l1_row_major(seeds.a_to_b.flows().to_vec()).unwrap(),
            Some(&a_hint),
            DescentAdmission::EveryPatch,
        )
        .unwrap();
        let expected_b = solve_with_descent_admission(
            &b_input,
            InitialGrid::<BtoA>::from_l1_row_major(seeds.b_to_a.flows().to_vec()).unwrap(),
            Some(&b_hint),
            DescentAdmission::EveryPatch,
        )
        .unwrap();
        let terminal = output
            .submit_l1_pis(
                &bridge,
                &pis,
                resident_l1_controls(
                    a_input.disparity_interval().unwrap(),
                    b_input.disparity_interval().unwrap(),
                ),
            )
            .unwrap();
        let actual = terminal.read_terminal_words_for_test().unwrap();
        let expected = expected_a
            .patches()
            .iter()
            .chain(expected_b.patches())
            .flat_map(|patch| [patch.flow().dcol().to_bits(), patch.flow().drow().to_bits()])
            .collect::<Vec<_>>();
        assert_eq!(
            actual, expected,
            "actual warm resident L1 terminal ignored or misplaced hints"
        );
        terminal.acknowledge_qualified_for_test().unwrap();
    }

    fn resident_direction<D: PisDirection>() -> GpuPisDynamicDirection<D> {
        GpuPisDynamicDirection {
            cost_modes: vec![CostMode::Unweighted; Level::Two.patch_rows()].into_boxed_slice(),
            initial: InitialGrid::coarse_zeros(),
            hint: None,
            admission: DescentAdmission::EveryPatch,
            disparity: None,
        }
    }

    fn resident_l1_controls(
        a_disparity: DisparityInterval,
        b_disparity: DisparityInterval,
    ) -> GpuL1Controls {
        fn direction<D: PisDirection>(disparity: DisparityInterval) -> GpuL1DirectionControls<D> {
            GpuL1DirectionControls {
                cost_modes: vec![CostMode::Unweighted; Level::One.patch_rows()].into_boxed_slice(),
                disparity,
                direction: PhantomData,
            }
        }
        GpuL1Controls {
            a_to_b: direction::<AtoB>(a_disparity),
            b_to_a: direction::<BtoA>(b_disparity),
        }
    }

    fn l1_oracle_inputs(
        blurred: &crate::flow::one_xs::temporal::BlurredBelts,
        masks: &LensPair<Vec<u8>>,
    ) -> (
        crate::flow::one_xs::pis::Input<AtoB>,
        crate::flow::one_xs::pis::Input<BtoA>,
    ) {
        let retained = ColdInputs::from_blurred_belts_and_masks(blurred.clone(), masks.clone());
        let pyramid = MaskPyramid::build(&retained);
        let modes = vec![CostMode::Unweighted; Level::One.patch_rows()];
        (
            LevelInputs::build::<AtoB>(&retained, &pyramid, Level::One)
                .resident_l2_oracle_input::<AtoB>(Level::One, modes.clone()),
            LevelInputs::build::<BtoA>(&retained, &pyramid, Level::One)
                .resident_l2_oracle_input::<BtoA>(Level::One, modes),
        )
    }

    #[test]
    fn nonfinite_retained_times_zero_is_gated_only_when_it_reaches_a_center() {
        let (context, _) = gpu().expect("Vulkan GPU required for L2 bridge finite gate");
        let bridge = GpuL2PostPisBridge::new(context).unwrap();

        let mut safe = fixture().unwrap();
        if let LevelTwoPostUpdate::Warm {
            a_to_b,
            b_to_a,
            motion,
        } = &mut safe.warm
        {
            motion[0] = 1;
            a_to_b.dcol[0] = f32::NAN;
            b_to_a.drow[0] = f32::INFINITY;
        }
        let token = bridge
            .submit_qualification(&safe.a_terminal, &safe.b_terminal, &safe.images, &safe.warm)
            .expect("nonfinite retained values outside every sampled footprint are accepted");
        let alignment = u64::from(
            bridge
                .context
                .device()
                .limits()
                .min_storage_buffer_offset_alignment,
        );
        let a = token.direction::<AtoB>();
        let b = token.direction::<BtoA>();
        for offset in [a.dcol_offset, a.drow_offset, b.dcol_offset, b.drow_offset] {
            assert_eq!(offset % alignment, 0);
        }
        assert_eq!(a.binding_size.get(), (L1_PATCHES * 4) as u64);

        let reaching = fixture().unwrap();
        let retained_a = nonfinite_selected_retained::<AtoB>(f32::NAN, f32::INFINITY);
        let retained_b = nonfinite_selected_retained::<BtoA>(f32::NEG_INFINITY, f32::NAN);
        let motion = vec![1; L2_PIXELS];
        assert_selected_warm_nonfinite(
            &reaching.a_terminal,
            &reaching.images,
            &retained_a,
            &motion,
        );
        assert_selected_warm_nonfinite(
            &reaching.b_terminal,
            &reaching.images,
            &retained_b,
            &motion,
        );
        let (a_dcol, a_drow) = retained_a.l2_bridge_qualification_components();
        let (b_dcol, b_drow) = retained_b.l2_bridge_qualification_components();
        let post = LevelTwoPostUpdate::warm(
            RetainedLevelTwo::new(a_dcol.to_vec(), a_drow.to_vec()).unwrap(),
            RetainedLevelTwo::new(b_dcol.to_vec(), b_drow.to_vec()).unwrap(),
            motion,
        )
        .unwrap();
        let error = match bridge.submit_qualification(
            &reaching.a_terminal,
            &reaching.b_terminal,
            &reaching.images,
            &post,
        ) {
            Ok(_) => panic!("sampled nonfinite center minted a typed token"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("level-one initial"), "{error}");
        assert!(error.contains("is not finite"), "{error}");
    }

    fn nonfinite_selected_retained<D: PisDirection>(
        dcol: f32,
        drow: f32,
    ) -> RetainedPublicPyramids<D> {
        RetainedPublicPyramids::from_public(
            PublicDenseField::from_row_major_components(
                vec![dcol; crate::flow::one_xs::ROWS * crate::flow::one_xs::COLS],
                vec![drow; crate::flow::one_xs::ROWS * crate::flow::one_xs::COLS],
            )
            .unwrap(),
        )
    }

    fn assert_selected_warm_nonfinite<D: PisDirection>(
        terminal: &CpuUploadedTerminalL2<D>,
        images: &LevelTwoImages,
        retained: &RetainedPublicPyramids<D>,
        motion: &[u8],
    ) {
        let directed = dense::DirectedImages::<D>::from_native_order(
            Level::Two,
            LensPair {
                a: &images.a,
                b: &images.b,
            },
        )
        .unwrap();
        let patches = PatchGrid::from_row_major_components(
            Level::Two,
            terminal.dcol.to_vec(),
            terminal.drow.to_vec(),
        )
        .unwrap();
        let dense = dense::densify_coarse(&directed, patches).unwrap();
        let post = post_update::update_without_variational_with_retained(
            dense,
            retained,
            MotionLevel::from_bytes(Level::Two, motion.to_vec()).unwrap(),
        )
        .unwrap();
        assert!(l2_seed::into_l1_initial_grid(post).is_err());
    }

    fn receipt(level: Level) -> GpuPisStageReceipt {
        GpuPisStageReceipt {
            flight: GpuPisFlight {
                generation: 17,
                frame: FrameStamp::for_test(23, Duration::from_millis(750), None),
            },
            stage: PairSolveStage::Warm { level },
        }
    }

    #[test]
    fn focused_arithmetic_mutations_are_rejected() {
        let (context, _) = gpu().expect("Vulkan GPU required for L2 bridge mutation gate");
        GpuL2PostPisBridge::new_qualified_all(
            context.clone(),
            SHADER,
            SEED_INJECTION_SHADER,
            HINT_INJECTION_SHADER,
        )
        .expect("unmutated resident L2/L1 shaders qualify before negative controls");
        let mutations = [
            ("clamp epsilon", "0xba83126fu", "0x00000000u"),
            ("photo gate", "difference > 1.0", "difference >= 0.0"),
            ("still weight", "0x3ca3d70au", "0x3ca3d70bu"),
            ("motion law", "motion_code == 0u", "motion_code != 0u"),
            (
                "packed motion word",
                "motion[pixel / 4u]",
                "motion[pixel / 3u]",
            ),
            ("packed motion byte", "pixel % 4u", "pixel % 3u"),
            (
                "packed motion edge and robust OOB",
                "let pixel = row * 15u + col;",
                "let pixel = row * 15u + col + 1u;",
            ),
            ("packed motion lane mask", ") & 255u;", ");"),
            (
                "horizontal association",
                "return add_rn(left_term, right_term);",
                "return fma_rn(dense(direction, component, row, left), lw, right_term);",
            ),
            (
                "vector scale",
                "let value = mul_rn(fma_rn(top_value, tw, bottom_term), 2.0);",
                "let value = mul_rn(fma_rn(top_value, tw, bottom_term), 1.0);",
            ),
            ("patch center", "patch_col * 3u + 4u", "patch_col * 3u + 3u"),
        ];
        for (name, from, to) in mutations {
            let shader = SHADER.replacen(from, to, 1);
            assert_ne!(shader, SHADER, "mutation source exists: {name}");
            let rejected = GpuL2PostPisBridge::new_qualified(context.clone(), &shader).is_err();
            assert!(rejected, "fixture did not reject {name}");
        }

        for (name, from, to) in [
            (
                "seed direction plane",
                "direction * 2u * stride",
                "direction * stride",
            ),
            ("seed interleave", "+ 2u * patch_index", "+ patch_index"),
        ] {
            let shader = SEED_INJECTION_SHADER.replacen(from, to, 1);
            assert_ne!(
                shader, SEED_INJECTION_SHADER,
                "mutation source exists: {name}"
            );
            assert!(
                GpuL2PostPisBridge::new_qualified_all(
                    context.clone(),
                    SHADER,
                    &shader,
                    HINT_INJECTION_SHADER,
                )
                .is_err(),
                "fixture did not reject {name}"
            );
        }
        for (name, from, to) in [
            (
                "hint OOB guard",
                "patch_index >= config[6]",
                "patch_index > config[6]",
            ),
            (
                "hint center row",
                "3u * patch_row + 4u",
                "3u * patch_row + 3u",
            ),
            ("hint patch columns", "/ config[7]", "/ (config[7] + 1u)"),
            ("hint dense width", "* config[8] +", "* (config[8] - 1u) +"),
        ] {
            let shader = HINT_INJECTION_SHADER.replacen(from, to, 1);
            assert_ne!(
                shader, HINT_INJECTION_SHADER,
                "mutation source exists: {name}"
            );
            assert!(
                GpuL2PostPisBridge::new_qualified_all(
                    context.clone(),
                    SHADER,
                    SEED_INJECTION_SHADER,
                    &shader,
                )
                .is_err(),
                "fixture did not reject {name}"
            );
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            match future.as_mut().poll(&mut cx) {
                std::task::Poll::Ready(v) => return v,
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
        let name = format!("{:?}", adapter.get_info());
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ONE X2 L2 bridge qualifier"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        Ok((OneXsGpuContext::new(&device, &queue), name))
    }

    fn gpu_context_pair() -> Result<(OneXsGpuContext, OneXsGpuContext, String), String> {
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
        let request = || {
            block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("exact ONE X2 resident context provenance"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            }))
            .map_err(|error| error.to_string())
        };
        let (device, queue) = request()?;
        let (foreign_device, foreign_queue) = request()?;
        Ok((
            OneXsGpuContext::new(&device, &queue),
            OneXsGpuContext::new(&foreign_device, &foreign_queue),
            name,
        ))
    }
}
