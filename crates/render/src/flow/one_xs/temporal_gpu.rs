//! Unselected GPU-resident warm image history for the selected ONE X2 path.
//!
//! This stage owns only image-temporal state: the exact base motion mask, its
//! recursive L1/L2 reductions and the next physical A/B reference planes.  A
//! pending result remains inside the capture root. It can become the committed
//! successor only at the future whole-frame atomic install boundary.

#[cfg(test)]
use super::super::GpuBlurredBelts;
use super::super::pis_frontend_gpu::{
    GpuCold0Terminal, GpuColdLoopControls, GpuL1Controls, GpuL1PreparedTerminal, GpuL2Controls,
    GpuL2PostPisBridge, GpuPisFrontEnd, GpuResidentLevelTwoPost, GpuWorkModeBinding,
    GpuWorkModePipeline, RetainedL2DirectionPixelVec2Buffer,
};
use super::super::resident_frame_gpu::{
    GpuResidentCandidate, GpuResidentCapture, GpuResidentReservation, ResidentPostL1Storage,
    ResidentSuccessor,
};
use super::GpuGeometryBelts;
use super::GpuGeometryFrameOwner;
use crate::Fallible;
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
use crate::flow::one_xs::pis::gpu::GpuPisFlight;
use crate::flow::one_xs::pis::gpu::GpuPisPipeline;
use crate::flow::one_xs::pis::{DescentAdmission, Level};
use crate::flow::one_xs::post_update::MotionPyramid;
use crate::flow::one_xs::scalar::PairSolveStage;
use crate::flow::one_xs::temporal::{BlurredBelts, MotionMask, next_warm_references};
use crate::flow::one_xs::warm::EmptyOverrideCadence;
use crate::flow::one_xs::{COLS, LensPair, ROWS};
use crate::flow::one_xs_belt::SolverBelts;

const CODES_PER_WORD: usize = 4;
const BASE_BYTES: usize = ROWS * COLS;
const L1_BYTES: usize = Level::One.pixels();
const L2_BYTES: usize = Level::Two.pixels();
const BELT_BYTES: usize = SolverBelts::BYTES;
const POST_HIST_WORDS: usize = 2 * Level::One.patches() * 159;
const POST_FIFO_WORDS: usize = 2 * Level::One.patches() * 5;
const POST_PUBLIC_WORDS: usize = 2 * ROWS * COLS * 2;
const POST_HINT_WORDS: usize = 275_400;
const POST_SMALL_WORDS: usize = 2 * Level::One.patch_rows();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GpuMotionHistory {
    Cold,
    Warm,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GpuMotionReceipt {
    pub(crate) flight: GpuPisFlight,
    pub(crate) temporal_generation: u64,
    pub(crate) history: GpuMotionHistory,
}

/// One immutable, pre-increment A/B admission snapshot. Its fields are
/// constructible only inside this temporal owner; downstream siblings may
/// carry and inspect it but cannot mint or alter an ordinary admitted call.
#[derive(Clone, Copy)]
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuAdmittedSnapshot {
    a_to_b: DescentAdmission,
    b_to_a: DescentAdmission,
}

impl GpuAdmittedSnapshot {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn a_to_b(self) -> DescentAdmission {
        self.a_to_b
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn b_to_a(self) -> DescentAdmission {
        self.b_to_a
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn qualification_every_patch() -> Self {
        Self {
            a_to_b: DescentAdmission::EveryPatch,
            b_to_a: DescentAdmission::EveryPatch,
        }
    }
}

/// One private, qualified GPU context matched to a solver-belt producer.
pub(crate) struct GpuMotionStage {
    context: OneXsGpuContext,
    threshold: wgpu::ComputePipeline,
    promote: wgpu::ComputePipeline,
    reduce_l1: wgpu::ComputePipeline,
    reduce_l2: wgpu::ComputePipeline,
    cold_refs: wgpu::ComputePipeline,
    next_refs: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuMotionCandidate {
    reservation: Option<GpuResidentReservation>,
    receipt: GpuMotionReceipt,
    current: wgpu::Buffer,
    raw_base: wgpu::Buffer,
    base: wgpu::Buffer,
    level_one: wgpu::Buffer,
    level_two: wgpu::Buffer,
    successor: Option<ResidentSuccessor>,
    _resources: wgpu::BindGroup,
}

impl Drop for GpuMotionCandidate {
    fn drop(&mut self) {
        let Some(reservation) = self.reservation.take() else {
            return;
        };
        if reservation.abort().is_err() {
            // A rollback that cannot be authenticated must retain the whole
            // successor allocation. The root is already fail-closed, so
            // releasing it here would only disguise a stale/poisoned state.
            if let Some(successor) = self.successor.take() {
                std::mem::forget(successor);
            }
        }
    }
}

/// Purpose-specific temporal allocation, reservation and command. Only the
/// resident belt owner can submit it and join it to the inherited lease.
struct EncodedGpuMotion {
    command: Option<wgpu::CommandBuffer>,
    candidate: Option<GpuMotionCandidate>,
}

struct MotionSubmissionGuard<C> {
    carrier: Option<C>,
    encoded: Option<EncodedGpuMotion>,
}

impl<C> MotionSubmissionGuard<C> {
    fn new(carrier: C, encoded: EncodedGpuMotion) -> Self {
        Self {
            carrier: Some(carrier),
            encoded: Some(encoded),
        }
    }
}

/// Candidate second slot. Drop is rollback; transfer only seals the successor.
#[must_use = "the GPU motion transaction must enter a frame candidate or roll back"]
pub(crate) struct GpuMotionTransaction<C> {
    // Field order is a safety invariant: the carrier's SubmissionLease waits
    // or quarantines submitted work before the reservation/prior can roll
    // back from `candidate`, including during unwind.
    carrier: Option<C>,
    candidate: Option<GpuMotionCandidate>,
}

/// Opaque pending successor. Publication belongs only to the capture owner's
/// final atomic ready-draw install, after every downstream stage succeeds.
#[must_use = "the pending GPU motion frame must reach final install or roll back"]
pub(crate) struct GpuMotionFrame<C> {
    // Keep this first and optional so explicit refusal can enforce the same
    // wait-before-root-rollback order as ordinary field drop and unwind.
    carrier: Option<C>,
    motion: GpuMotionCandidate,
    root_candidate: Option<GpuResidentCandidate>,
}

mod prior_public_l2 {
    pub trait Sealed {}
}

/// Exact private input from the capture-owned resident public L2 slot. It
/// creates complete bind groups, so neither retained state nor successor hint
/// storage can detach from the capture owner.
pub(in crate::flow::one_xs::one_xs_belt_gpu) trait GpuPriorPublicLevelTwo:
    prior_public_l2::Sealed + Sized
{
    fn context(&self) -> &OneXsGpuContext;
    fn is_warm(&self) -> bool;
    fn cadence(&self) -> GpuPairedCadence;
    fn initialize(&self, encoder: &mut wgpu::CommandEncoder);
    #[allow(clippy::too_many_arguments)]
    fn bind_l2_bridge(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        config: &wgpu::Buffer,
        images: &wgpu::Buffer,
        terminal: &wgpu::Buffer,
        motion_l2: &wgpu::Buffer,
        output: &wgpu::Buffer,
        validity: &wgpu::Buffer,
    ) -> wgpu::BindGroup;
    fn bind_l1_hint_fill(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        config: &wgpu::Buffer,
        dynamic: &wgpu::Buffer,
    ) -> wgpu::BindGroup;
    fn bind_work_mode_fill(
        &self,
        pipeline: &GpuWorkModePipeline,
        dynamic: &wgpu::Buffer,
        current_l1_lack: &wgpu::Buffer,
        level: Level,
        flight: &GpuPisFlight,
    ) -> Fallible<GpuWorkModeBinding>;
}

/// Allocation-owned A/B cadence snapshot for one admitted pair call. Both
/// levels consume these pre-increment values; only post-L1 may replace the
/// owner with `after_call`. Keeping the directions distinct is load-bearing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuPairedCadence {
    a_to_b: EmptyOverrideCadence,
    b_to_a: EmptyOverrideCadence,
}

impl GpuPairedCadence {
    fn new(a_to_b: EmptyOverrideCadence, b_to_a: EmptyOverrideCadence) -> Self {
        Self { a_to_b, b_to_a }
    }

    fn cold_root() -> Self {
        Self::new(
            EmptyOverrideCadence::new(0, 10).expect("fixed cold cadence is nonzero"),
            EmptyOverrideCadence::new(0, 10).expect("fixed cold cadence is nonzero"),
        )
    }

    fn admissions(self) -> GpuAdmittedSnapshot {
        GpuAdmittedSnapshot {
            a_to_b: self.a_to_b.admission(),
            b_to_a: self.b_to_a.admission(),
        }
    }

    #[allow(dead_code)] // consumed by the private post-L1 successor checkpoint
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn after_call(self) -> Self {
        Self::new(self.a_to_b.after_calc(), self.b_to_a.after_calc())
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn counts(self) -> [i32; 2] {
        [self.a_to_b.calc_count(), self.b_to_a.calc_count()]
    }
}

/// Real cold prior-public owner for the first selected calculation. Its zero
/// resident allocations are initialized by GPU clears in the inherited
/// submission; no CPU grid or raw handle crosses the transition.
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuColdPriorPublicLevelTwo {
    context: OneXsGpuContext,
    cadence: GpuPairedCadence,
    calculation: u8,
    retained_l2_direction_pixel_vec2: RetainedL2DirectionPixelVec2Buffer,
    public: wgpu::Buffer,
    hints: wgpu::Buffer,
    histogram: wgpu::Buffer,
    fifo: wgpu::Buffer,
    small_rows: wgpu::Buffer,
    small_present: bool,
    lack_rows: wgpu::Buffer,
}

impl GpuColdPriorPublicLevelTwo {
    pub(super) fn new(context: OneXsGpuContext) -> Self {
        Self {
            cadence: GpuPairedCadence::cold_root(),
            calculation: 0,
            retained_l2_direction_pixel_vec2: RetainedL2DirectionPixelVec2Buffer::new(buffer(
                context.device(),
                "ONE X2 cold prior-public retained L2 direction-pixel-vec2",
                4 * L2_BYTES * size_of::<f32>(),
            )),
            hints: buffer(
                context.device(),
                "ONE X2 cold resident successor hints",
                POST_HINT_WORDS * size_of::<u32>(),
            ),
            public: buffer(
                context.device(),
                "ONE X2 cold resident public fields",
                POST_PUBLIC_WORDS * size_of::<u32>(),
            ),
            histogram: buffer(
                context.device(),
                "ONE X2 cold resident temporal histograms",
                POST_HIST_WORDS * size_of::<u32>(),
            ),
            fifo: buffer(
                context.device(),
                "ONE X2 cold resident temporal FIFOs",
                POST_FIFO_WORDS * size_of::<u32>(),
            ),
            small_rows: buffer(
                context.device(),
                "ONE X2 cold resident small rows",
                POST_SMALL_WORDS * size_of::<u32>(),
            ),
            small_present: false,
            lack_rows: buffer(
                context.device(),
                "ONE X2 cold retained lack rows",
                POST_SMALL_WORDS * size_of::<u32>(),
            ),
            context,
        }
    }
}

impl prior_public_l2::Sealed for GpuColdPriorPublicLevelTwo {}

impl GpuMotionResidentL2Post<GpuColdPriorPublicLevelTwo> {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn ensure_final_reservation(
        &self,
        context: &OneXsGpuContext,
        flight: &GpuPisFlight,
        root: &super::super::resident_frame_gpu::GpuResidentIdentity,
    ) -> Fallible<()> {
        context.ensure_same(self.prior.context())?;
        let reservation = self
            .motion
            .reservation
            .as_ref()
            .ok_or("ONE X2 final map lost its root reservation")?;
        if reservation.flight() != flight {
            return Err("ONE X2 final-map post state names a different root flight".into());
        }
        if !reservation.identity().matches(root) {
            return Err("ONE X2 final-map post state belongs to a different capture root".into());
        }
        if self.motion.successor.is_none() {
            return Err("ONE X2 final map lost its resident successor allocation".into());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn cold_lifecycle_for_test(
        &self,
    ) -> (
        &wgpu::Buffer,
        &wgpu::Buffer,
        [i32; 2],
        u8,
        bool,
        super::super::resident_frame_gpu::GpuResidentIdentity,
    ) {
        (
            &self.prior.lack_rows,
            &self.prior.small_rows,
            self.prior.cadence.counts(),
            self.prior.calculation,
            self.prior.small_present,
            self.motion
                .reservation
                .as_ref()
                .expect("cold lifecycle retains its reservation")
                .identity(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn bind_cold_post_l1(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        terminal: &wgpu::Buffer,
        images: &wgpu::Buffer,
        histogram: &wgpu::Buffer,
        fifo: &wgpu::Buffer,
        hints: &wgpu::Buffer,
        filtered: &wgpu::Buffer,
        dense: &wgpu::Buffer,
        horizontal: &wgpu::Buffer,
        public: &wgpu::Buffer,
        quantized_values: &wgpu::Buffer,
        retained_l2: &wgpu::Buffer,
        validity: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 resident cold post-L1"),
            layout,
            entries: &[
                entry(0, terminal),
                entry(1, images),
                entry(3, histogram),
                entry(4, fifo),
                entry(7, hints),
                entry(9, filtered),
                entry(11, dense),
                entry(12, horizontal),
                entry(13, public),
                entry(14, quantized_values),
                entry(16, retained_l2),
                entry(17, validity),
            ],
        })
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn encode_cold_post_l1_copies(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        histogram: &wgpu::Buffer,
        fifo: &wgpu::Buffer,
        lack_rows: &wgpu::Buffer,
        captured_lack_rows: &wgpu::Buffer,
        calculation: u8,
    ) {
        encoder.copy_buffer_to_buffer(
            &self.prior.histogram,
            0,
            histogram,
            0,
            (POST_HIST_WORDS * size_of::<u32>()) as u64,
        );
        encoder.copy_buffer_to_buffer(
            &self.prior.fifo,
            0,
            fifo,
            0,
            (POST_FIFO_WORDS * size_of::<u32>()) as u64,
        );
        encoder.copy_buffer_to_buffer(
            if calculation == 0 {
                captured_lack_rows
            } else {
                &self.prior.lack_rows
            },
            0,
            lack_rows,
            0,
            (POST_SMALL_WORDS * size_of::<u32>()) as u64,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn commit_cold_post_l1(
        &mut self,
        histogram: wgpu::Buffer,
        fifo: wgpu::Buffer,
        hints: wgpu::Buffer,
        small_rows: wgpu::Buffer,
        lack_rows: wgpu::Buffer,
        public: wgpu::Buffer,
        retained_l2_direction_pixel_vec2: Option<RetainedL2DirectionPixelVec2Buffer>,
        calculation: u8,
    ) {
        self.prior.histogram = histogram;
        self.prior.fifo = fifo;
        self.prior.hints = hints;
        self.prior.small_rows = small_rows;
        self.prior.lack_rows = lack_rows;
        self.prior.public = public;
        if let Some(retained_l2_direction_pixel_vec2) = retained_l2_direction_pixel_vec2 {
            self.prior.retained_l2_direction_pixel_vec2 = retained_l2_direction_pixel_vec2;
        }
        self.prior.cadence = self.prior.cadence.after_call();
        self.prior.calculation = calculation + 1;
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn attach_cold_successor(
        &mut self,
    ) -> Fallible<()> {
        let storage = ResidentPostL1Storage::after_cold(
            self.prior.public.clone(),
            self.prior.retained_l2_direction_pixel_vec2.clone(),
            self.prior.histogram.clone(),
            self.prior.fifo.clone(),
            self.prior.hints.clone(),
            self.prior.lack_rows.clone(),
            self.prior.small_rows.clone(),
            self.prior.small_present,
            self.prior.cadence.counts(),
        );
        let successor = self
            .motion
            .successor
            .as_mut()
            .ok_or("resident cold loop lost its motion successor")?;
        successor
            .attach_post_l1(storage)
            .map_err(|error| error.to_string().into())
    }
}

impl GpuPriorPublicLevelTwo for GpuColdPriorPublicLevelTwo {
    fn context(&self) -> &OneXsGpuContext {
        &self.context
    }

    fn is_warm(&self) -> bool {
        false
    }

    fn cadence(&self) -> GpuPairedCadence {
        self.cadence
    }

    fn initialize(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.calculation == 0 {
            encoder.clear_buffer(self.retained_l2_direction_pixel_vec2.buffer(), 0, None);
            encoder.clear_buffer(&self.public, 0, None);
            encoder.clear_buffer(&self.hints, 0, None);
            encoder.clear_buffer(&self.histogram, 0, None);
            encoder.clear_buffer(&self.fifo, 0, None);
            encoder.clear_buffer(&self.small_rows, 0, None);
            encoder.clear_buffer(&self.lack_rows, 0, None);
        }
    }

    fn bind_l2_bridge(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        config: &wgpu::Buffer,
        images: &wgpu::Buffer,
        terminal: &wgpu::Buffer,
        motion_l2: &wgpu::Buffer,
        output: &wgpu::Buffer,
        validity: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 cold prior-public L2 owner"),
            layout,
            entries: &[
                entry(0, config),
                entry(1, images),
                entry(2, terminal),
                entry(3, self.retained_l2_direction_pixel_vec2.buffer()),
                entry(4, motion_l2),
                entry(5, output),
                entry(6, validity),
            ],
        })
    }

    fn bind_l1_hint_fill(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        config: &wgpu::Buffer,
        dynamic: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 cold prior-public hint owner"),
            layout,
            entries: &[entry(0, config), entry(1, &self.hints), entry(2, dynamic)],
        })
    }

    fn bind_work_mode_fill(
        &self,
        pipeline: &GpuWorkModePipeline,
        dynamic: &wgpu::Buffer,
        current_l1_lack: &wgpu::Buffer,
        level: Level,
        flight: &GpuPisFlight,
    ) -> Fallible<GpuWorkModeBinding> {
        Ok(pipeline.bind_cold(dynamic, current_l1_lack, level, flight))
    }
}

/// Pending temporal successor fused to the prior-public L2 owner. The root
/// reservation remains opaque and unsealed inside `motion` until a later
/// whole-frame candidate owns every downstream result.
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuMotionResidentL2Post<
    P: GpuPriorPublicLevelTwo,
> {
    motion: GpuMotionCandidate,
    prior: P,
}

impl<P: GpuPriorPublicLevelTwo> super::super::pis_frontend_gpu::resident_l2_post_seal::Sealed
    for GpuMotionResidentL2Post<P>
{
}

impl<P: GpuPriorPublicLevelTwo> GpuResidentLevelTwoPost for GpuMotionResidentL2Post<P> {
    fn resident_identity(
        &self,
    ) -> Fallible<Option<super::super::resident_frame_gpu::GpuResidentIdentity>> {
        self.motion
            .reservation
            .as_ref()
            .map(GpuResidentReservation::identity)
            .map(Some)
            .ok_or_else(|| "ONE X2 resident post lost its root reservation".into())
    }

    fn context(&self) -> &OneXsGpuContext {
        self.prior.context()
    }

    fn is_warm(&self) -> bool {
        self.prior.is_warm()
    }

    fn admissions(&self) -> GpuAdmittedSnapshot {
        self.prior.cadence().admissions()
    }

    fn initialize(&self, encoder: &mut wgpu::CommandEncoder) {
        self.prior.initialize(encoder);
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
        self.prior.bind_l2_bridge(
            device,
            layout,
            config,
            images,
            terminal,
            &self.motion.level_two,
            output,
            validity,
        )
    }

    fn bind_l1_hint_fill(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        config: &wgpu::Buffer,
        dynamic: &wgpu::Buffer,
    ) -> Option<wgpu::BindGroup> {
        Some(
            self.prior
                .bind_l1_hint_fill(device, layout, config, dynamic),
        )
    }

    fn bind_work_mode_fill(
        &self,
        pipeline: &GpuWorkModePipeline,
        dynamic: &wgpu::Buffer,
        current_l1_lack: &wgpu::Buffer,
        level: Level,
        flight: &GpuPisFlight,
    ) -> Fallible<GpuWorkModeBinding> {
        self.prior
            .bind_work_mode_fill(pipeline, dynamic, current_l1_lack, level, flight)
    }
}

impl GpuMotionStage {
    pub(crate) fn new(context: OneXsGpuContext) -> Fallible<Self> {
        Self::from_shader(context, SHADER, true)
    }

    fn from_shader(context: OneXsGpuContext, shader: &str, qualify: bool) -> Fallible<Self> {
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
            label: Some("ONE X2 GPU temporal image state"),
            entries: &[
                storage(0, true),
                storage(1, true),
                storage(2, false),
                storage(3, false),
                storage(4, false),
                storage(5, false),
                storage(6, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 GPU temporal image state"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 GPU temporal image state"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ONE X2 GPU temporal image state"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let stage = Self {
            context: context.clone(),
            threshold: pipeline("threshold_base"),
            promote: pipeline("promote_base"),
            reduce_l1: pipeline("reduce_l1"),
            reduce_l2: pipeline("reduce_l2"),
            cold_refs: pipeline("cold_references"),
            next_refs: pipeline("next_references"),
            layout,
        };
        if qualify {
            stage.qualify()?;
        }
        Ok(stage)
    }

    pub(crate) fn new_capture(&self) -> GpuResidentCapture {
        GpuResidentCapture::new()
    }

    fn context_for_resident_transition(&self) -> &OneXsGpuContext {
        &self.context
    }

    /// Reserve one temporal generation and encode its exact arithmetic. The
    /// resident belt owner remains responsible for the sole lease advancement.
    fn encode_resident_transition(
        &self,
        reservation: GpuResidentReservation,
        current: &wgpu::Buffer,
    ) -> Fallible<EncodedGpuMotion> {
        let history = if reservation.prior().is_some() {
            GpuMotionHistory::Warm
        } else {
            GpuMotionHistory::Cold
        };
        let reference = reservation
            .prior()
            .as_ref()
            .map(|successor| successor.motion_reference())
            .unwrap_or(current);
        let device = self.context.device();
        let raw_base = buffer(device, "ONE X2 raw motion", BASE_BYTES);
        let base = buffer(device, "ONE X2 promoted motion", BASE_BYTES);
        let level_one = buffer(device, "ONE X2 L1 motion", L1_BYTES);
        let level_two = buffer(device, "ONE X2 L2 motion", L2_BYTES);
        let next_references = buffer(device, "ONE X2 next references", BELT_BYTES);
        let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 GPU temporal image state"),
            layout: &self.layout,
            entries: &[
                entry(0, current),
                entry(1, reference),
                entry(2, &raw_base),
                entry(3, &base),
                entry(4, &level_one),
                entry(5, &level_two),
                entry(6, &next_references),
            ],
        });
        let command = self.encode(device, &resources, history);
        Ok(EncodedGpuMotion {
            command: Some(command),
            candidate: Some(GpuMotionCandidate {
                receipt: GpuMotionReceipt {
                    flight: reservation.flight().clone(),
                    temporal_generation: reservation.generation(),
                    history,
                },
                successor: Some(ResidentSuccessor::from_motion(
                    reservation.flight().clone(),
                    next_references,
                )),
                reservation: Some(reservation),
                current: current.clone(),
                raw_base,
                base,
                level_one,
                level_two,
                _resources: resources,
            }),
        })
    }

    fn encode(
        &self,
        device: &wgpu::Device,
        resources: &wgpu::BindGroup,
        history: GpuMotionHistory,
    ) -> wgpu::CommandBuffer {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 GPU temporal image state"),
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ONE X2 GPU temporal image state"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, resources, &[]);
        if history == GpuMotionHistory::Warm {
            for (pipeline, bytes) in [
                (&self.threshold, BASE_BYTES),
                (&self.promote, BASE_BYTES),
                (&self.reduce_l1, L1_BYTES),
                (&self.reduce_l2, L2_BYTES),
            ] {
                pass.set_pipeline(pipeline);
                pass.dispatch_workgroups(words(bytes).div_ceil(64) as u32, 1, 1);
            }
        }
        pass.set_pipeline(match history {
            GpuMotionHistory::Cold => &self.cold_refs,
            GpuMotionHistory::Warm => &self.next_refs,
        });
        pass.dispatch_workgroups(words(BELT_BYTES).div_ceil(64) as u32, 1, 1);
        drop(pass);
        encoder.finish()
    }

    fn qualify(&self) -> Fallible<()> {
        let device = self.context.device();
        let queue = self.context.queue();
        let (reference, current) = qualification_pair();
        let expected_mask = MotionMask::between(&current, &reference);
        let expected_pyramid = MotionPyramid::from_base(&expected_mask);
        let expected_refs = next_warm_references(&reference, &current);
        let current_buffer = upload(device, queue, "motion current", current.bytes());
        let reference_buffer = upload(device, queue, "motion reference", reference.bytes());
        let raw = buffer(device, "motion raw qualification", BASE_BYTES);
        let base = buffer(device, "motion base qualification", BASE_BYTES);
        let l1 = buffer(device, "motion L1 qualification", L1_BYTES);
        let l2 = buffer(device, "motion L2 qualification", L2_BYTES);
        let next = buffer(device, "motion reference qualification", BELT_BYTES);
        let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 motion qualification"),
            layout: &self.layout,
            entries: &[
                entry(0, &current_buffer),
                entry(1, &reference_buffer),
                entry(2, &raw),
                entry(3, &base),
                entry(4, &l1),
                entry(5, &l2),
                entry(6, &next),
            ],
        });
        queue.submit([self.encode(device, &resources, GpuMotionHistory::Warm)]);
        for (name, gpu, expected) in [
            ("base", &base, expected_mask.bytes()),
            ("level one", &l1, expected_pyramid.bytes(Level::One)),
            ("level two", &l2, expected_pyramid.bytes(Level::Two)),
            ("next references", &next, expected_refs.bytes()),
        ] {
            let actual = read(device, queue, gpu, expected.len())?;
            if let Some(index) = actual.iter().zip(expected).position(|(a, b)| a != b) {
                return Err(format!("ONE X2 GPU temporal arithmetic is not exact on this graphics device: {name} byte {index} is {}, expected {}", actual[index], expected[index]).into());
            }
        }
        Ok(())
    }
}

impl EncodedGpuMotion {
    fn take_command(&mut self) -> wgpu::CommandBuffer {
        self.command
            .take()
            .expect("encoded GPU motion transition submits exactly once")
    }
}

impl<C> GpuMotionTransaction<C> {
    fn from_resident_transition(mut encoded: EncodedGpuMotion, carrier: C) -> Self {
        debug_assert!(encoded.command.is_none());
        Self {
            carrier: Some(carrier),
            candidate: encoded.candidate.take(),
        }
    }

    fn candidate(&self) -> &GpuMotionCandidate {
        self.candidate
            .as_ref()
            .expect("GPU motion transaction lost its candidate")
    }

    #[cfg(test)]
    fn receipt(&self) -> &GpuMotionReceipt {
        &self.candidate().receipt
    }
    #[cfg(test)]
    fn base(&self) -> Option<&wgpu::Buffer> {
        (self.receipt().history == GpuMotionHistory::Warm).then_some(&self.candidate().base)
    }
    #[cfg(test)]
    fn level(&self, level: Level) -> Option<&wgpu::Buffer> {
        (self.receipt().history == GpuMotionHistory::Warm).then_some(match level {
            Level::One => &self.candidate().level_one,
            Level::Two => &self.candidate().level_two,
        })
    }

    /// Seal the complete pending successor into the opaque frame candidate.
    /// This deliberately does not publish temporal state: later resident L2,
    /// post, final-map, bind and draw installation may still fail.
    #[cfg(test)]
    pub(crate) fn into_frame_candidate(mut self) -> GpuMotionFrame<C> {
        let mut motion = self
            .candidate
            .take()
            .expect("GPU motion transaction lost its candidate");
        let reservation = motion
            .reservation
            .take()
            .expect("GPU motion transaction lost its root reservation");
        let successor = motion
            .successor
            .take()
            .expect("GPU motion transaction lost its successor");
        GpuMotionFrame {
            carrier: Some(
                self.carrier
                    .take()
                    .expect("motion transaction lost its submission lease"),
            ),
            motion,
            root_candidate: Some(
                reservation
                    .seal(successor)
                    .expect("motion successor was minted from this exact reservation"),
            ),
        }
    }

    /// Explicit refusal before the downstream frame candidate exists.
    pub(crate) fn abort(mut self) -> Fallible<()> {
        drop(
            self.carrier
                .take()
                .expect("GPU motion transaction lost its submission lease"),
        );
        let mut candidate = self
            .candidate
            .take()
            .expect("GPU motion transaction lost its candidate");
        let reservation = candidate
            .reservation
            .take()
            .expect("GPU motion transaction lost its root reservation");
        let result = reservation.abort();
        if result.is_err()
            && let Some(successor) = candidate.successor.take()
        {
            std::mem::forget(successor);
        }
        result
    }
}

impl<K> GpuMotionTransaction<GpuGeometryBelts<K>> {
    pub(super) fn submit_resident_cold0(
        self,
        front_end: &GpuPisFrontEnd,
        solver: &GpuPisPipeline,
        bridge: &GpuL2PostPisBridge,
        prior: GpuColdPriorPublicLevelTwo,
        controls: GpuColdLoopControls,
    ) -> Fallible<GpuCold0Terminal<K>> {
        let terminal = self.submit_resident_l2_l1(
            front_end,
            solver,
            controls.l2(),
            bridge,
            prior,
            controls.l1(),
        )?;
        Ok(GpuCold0Terminal::new(terminal, controls))
    }

    /// Consume the root-carried motion transaction directly through front end,
    /// L2 PIS, resident post-L2 and L1 PIS. The root reservation is neither
    /// cloned nor sealed into a separately publishable candidate here.
    pub(super) fn submit_resident_l2_l1<P: GpuPriorPublicLevelTwo>(
        mut self,
        front_end: &GpuPisFrontEnd,
        solver: &GpuPisPipeline,
        l2: GpuL2Controls,
        bridge: &GpuL2PostPisBridge,
        prior: P,
        l1: GpuL1Controls,
    ) -> Fallible<GpuL1PreparedTerminal<GpuGeometryFrameOwner<K>, GpuMotionResidentL2Post<P>>> {
        let motion_is_warm = self.candidate().receipt.history == GpuMotionHistory::Warm;
        if prior.is_warm() != motion_is_warm {
            return Err("ONE X2 prior-public L2 state does not match temporal history".into());
        }
        let admissions = prior.cadence().admissions();
        let l2_stage = if motion_is_warm {
            PairSolveStage::Warm { level: Level::Two }
        } else {
            if admissions.a_to_b() != DescentAdmission::EveryPatch
                || admissions.b_to_a() != DescentAdmission::EveryPatch
            {
                return Err("ONE X2 Cold0 admission is not EveryPatch".into());
            }
            PairSolveStage::Cold {
                calculation: 0,
                level: Level::Two,
            }
        };
        let post = GpuMotionResidentL2Post {
            motion: self
                .candidate
                .take()
                .expect("resident L2 transition lost its root-carried motion"),
            prior,
        };
        let carrier = self
            .carrier
            .take()
            .expect("resident L2 transition lost its source submission lease");
        carrier
            .prepare_front_end(front_end)?
            .submit_resident_l2_bridge(solver, bridge, l2, l2_stage, post, None)?
            .submit_l1_pis(bridge, solver, l1)
    }
}

impl<K> GpuGeometryBelts<K> {
    /// Consume the entire resident producer token into the temporal stage.
    /// No buffer, flight, context, command or lease component crosses this
    /// private ownership boundary separately.
    pub(crate) fn prepare_motion(
        mut self,
        stage: &GpuMotionStage,
    ) -> Fallible<GpuMotionTransaction<Self>> {
        let context = stage.context_for_resident_transition();
        self.belts.lease.validate_provenance(context)?;
        let flight = self
            .belts
            .flight
            .as_ref()
            .expect("GPU-resident belts retain their flight until consumption")
            .clone();
        let reservation = self
            .reservation
            .take()
            .ok_or("ONE X2 resident geometry reached motion without its root reservation")?;
        if reservation.flight() != &flight {
            return Err("ONE X2 resident motion flight differs from its root reservation".into());
        }
        let encoded = stage.encode_resident_transition(reservation, &self.belts.packed)?;
        let mut joined = MotionSubmissionGuard::new(self, encoded);
        let carrier = joined.carrier.as_mut().ok_or("motion join lost carrier")?;
        let encoded = joined.encoded.as_mut().ok_or("motion join lost command")?;
        carrier
            .belts
            .lease
            .submit_after(context, |_| encoded.take_command())?;
        let self_carrier = joined.carrier.take().expect("checked motion carrier");
        let encoded = joined.encoded.take().expect("checked motion command");
        // The exact flight remains inside the carrier. Motion adds resident
        // state to that aggregate; only the later PIS front end may take the
        // original token, so no clone can be reassociated with another frame.
        Ok(GpuMotionTransaction::from_resident_transition(
            encoded,
            self_carrier,
        ))
    }
}

#[cfg(test)]
impl<K> GpuBlurredBelts<K> {
    fn prepare_motion(
        self,
        stage: &GpuMotionStage,
        reservation: GpuResidentReservation,
    ) -> Fallible<GpuMotionTransaction<Self>> {
        let context = stage.context_for_resident_transition();
        self.lease.validate_provenance(context)?;
        let flight = self
            .flight
            .as_ref()
            .expect("GPU-resident belts retain their flight until consumption")
            .clone();
        if reservation.flight() != &flight {
            return Err("ONE X2 test motion flight differs from its root reservation".into());
        }
        let encoded = stage.encode_resident_transition(reservation, &self.packed)?;
        let mut joined = MotionSubmissionGuard::new(self, encoded);
        let carrier = joined.carrier.as_mut().ok_or("motion join lost carrier")?;
        let encoded = joined.encoded.as_mut().ok_or("motion join lost command")?;
        carrier
            .lease
            .submit_after(context, |_| encoded.take_command())?;
        let self_carrier = joined.carrier.take().expect("checked motion carrier");
        let encoded = joined.encoded.take().expect("checked motion command");
        // Qualification follows the production ownership shape: retain the
        // original flight in the carrier for the eventual consuming stage.
        Ok(GpuMotionTransaction::from_resident_transition(
            encoded,
            self_carrier,
        ))
    }
}

impl<C> GpuMotionFrame<C> {
    pub(crate) fn receipt(&self) -> &GpuMotionReceipt {
        &self.motion.receipt
    }

    #[cfg(test)]
    fn current(&self) -> &wgpu::Buffer {
        &self.motion.current
    }

    #[cfg(test)]
    fn base(&self) -> Option<&wgpu::Buffer> {
        (self.receipt().history == GpuMotionHistory::Warm).then_some(&self.motion.base)
    }

    #[cfg(test)]
    fn level(&self, level: Level) -> Option<&wgpu::Buffer> {
        (self.receipt().history == GpuMotionHistory::Warm).then_some(match level {
            Level::One => &self.motion.level_one,
            Level::Two => &self.motion.level_two,
        })
    }

    /// Explicit downstream refusal. Drop has the same exact rollback effect.
    pub(crate) fn abort(mut self) -> Fallible<()> {
        drop(
            self.carrier
                .take()
                .expect("GPU motion frame lost its submission lease"),
        );
        let candidate = self
            .root_candidate
            .take()
            .expect("GPU motion frame lost its root candidate");
        candidate.abort()
    }
}

#[cfg(test)]
impl<K> GpuMotionFrame<GpuBlurredBelts<K>> {
    /// Test-only stand-in for the future capture-owner atomic ready-draw
    /// install. Production deliberately has no publication method yet.
    fn publish_at_install_for_test(mut self) -> Fallible<Self> {
        let candidate = self
            .root_candidate
            .take()
            .expect("GPU motion frame lost its root candidate");
        self.root_candidate = Some(candidate.install_successor_only_for_test()?);
        Ok(self)
    }

    fn observe_completion(&mut self, state: std::sync::Arc<std::sync::atomic::AtomicU8>) {
        self.carrier
            .as_mut()
            .expect("GPU motion frame lost its submission lease")
            .observe_completion(state);
    }
}

fn words(bytes: usize) -> usize {
    bytes.div_ceil(CODES_PER_WORD)
}
fn buffer(device: &wgpu::Device, label: &'static str, bytes: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (words(bytes) * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    bytes: &[u8],
) -> wgpu::Buffer {
    let buffer = buffer(device, label, bytes.len());
    let mut padded = vec![0; words(bytes.len()) * 4];
    padded[..bytes.len()].copy_from_slice(bytes);
    queue.write_buffer(&buffer, 0, &padded);
    buffer
}
fn fixture(seed: u32) -> BlurredBelts {
    BlurredBelts::from_lenses(LensPair {
        a: (0..BASE_BYTES)
            .map(|i| ((i as u32 * 37 + seed + (i / COLS) as u32 * 11) & 255) as u8)
            .collect(),
        b: (0..BASE_BYTES)
            .map(|i| ((i as u32 * 19 + seed * 3 + (i % COLS) as u32 * 7) & 255) as u8)
            .collect(),
    })
    .unwrap()
}

fn qualification_pair() -> (BlurredBelts, BlurredBelts) {
    let reference_lenses = LensPair {
        a: (0..BASE_BYTES)
            .map(|index| 32 + ((index * 37 + index / COLS * 11) % 160) as u8)
            .collect::<Vec<_>>(),
        b: (0..BASE_BYTES)
            .map(|index| 40 + ((index * 19 + index % COLS * 7) % 150) as u8)
            .collect::<Vec<_>>(),
    };
    let reference = BlurredBelts::from_lenses(reference_lenses.clone())
        .expect("qualification reference has retained shape");
    let mut current = LensPair {
        a: reference_lenses
            .a
            .iter()
            .map(|value| value + 3)
            .collect::<Vec<_>>(),
        b: reference_lenses
            .b
            .iter()
            .map(|value| value + 7)
            .collect::<Vec<_>>(),
    };
    // Isolated exact-threshold and B-only changes remain raw 1s because their
    // local populations are below the promotion threshold.
    current.a[12 * COLS + 13] = reference_lenses.a[12 * COLS + 13] + 10;
    current.b[37 * COLS + 29] = reference_lenses.b[37 * COLS + 29] + 11;
    // Exercise native's asymmetric low-edge bounds with a population that
    // straddles the first omitted row and column.
    for ordinal in 0..11 {
        let row = 1 + ordinal / 6;
        let col = 1 + ordinal % 6;
        let index = row * COLS + col;
        current.a[index] = reference_lenses.a[index] + 12;
    }
    for omitted_edge in [1, 2] {
        current.a[omitted_edge] = reference_lenses.a[omitted_edge] + 12;
    }
    // An interior zero byte sees exactly floor(0.2 * 20 * 12) changes.
    for row in 91..95 {
        for col in 15..27 {
            let index = row * COLS + col;
            current.a[index] = reference_lenses.a[index] + 12;
        }
    }
    // Keep a fully populated changed block at the far physical edge. It
    // survives promotion and both area-half reductions, so the packed L2
    // producer/consumer qualification exercises a nonzero trailing lane
    // alongside robust storage bounds.
    for row in ROWS - 12..ROWS {
        for col in COLS - 12..COLS {
            let index = row * COLS + col;
            current.b[index] = reference_lenses.b[index] + 12;
        }
    }
    (
        reference,
        BlurredBelts::from_lenses(current).expect("qualification pair has retained shape"),
    )
}
fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Buffer,
    bytes: usize,
) -> Fallible<Vec<u8>> {
    use std::sync::mpsc;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ONE X2 motion diagnostic readback"),
        size: (words(bytes) * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &staging, 0, (words(bytes) * 4) as u64);
    let copy = queue.submit([encoder.finish()]);
    let slice = staging.slice(..);
    let (sent, answer) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sent.send(result);
    });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(copy),
        timeout: None,
    })?;
    answer.recv()??;
    let mapped = slice.get_mapped_range();
    let result = mapped[..bytes].to_vec();
    drop(mapped);
    staging.unmap();
    Ok(result)
}

const SHADER: &str = r#"
const ROWS = 1080u; const COLS = 60u; const BASE = ROWS * COLS;
const L1_ROWS = 540u; const L1_COLS = 30u; const L1 = L1_ROWS * L1_COLS;
const L2_ROWS = 270u; const L2_COLS = 15u; const L2 = L2_ROWS * L2_COLS;
@group(0) @binding(0) var<storage, read> current: array<u32>;
@group(0) @binding(1) var<storage, read> reference: array<u32>;
@group(0) @binding(2) var<storage, read_write> raw_base: array<u32>;
@group(0) @binding(3) var<storage, read_write> motion_base: array<u32>;
@group(0) @binding(4) var<storage, read_write> motion_l1: array<u32>;
@group(0) @binding(5) var<storage, read_write> motion_l2: array<u32>;
@group(0) @binding(6) var<storage, read_write> next_ref: array<u32>;
fn current_byte(index: u32) -> u32 { return (current[index/4u] >> (8u*(index%4u))) & 255u; }
fn reference_byte(index: u32) -> u32 { return (reference[index/4u] >> (8u*(index%4u))) & 255u; }
fn raw(index: u32) -> u32 { return (raw_base[index/4u] >> (8u*(index%4u))) & 255u; }
fn promoted(index: u32) -> u32 { return (motion_base[index/4u] >> (8u*(index%4u))) & 255u; }
fn l1_at(index: u32) -> u32 { return (motion_l1[index/4u] >> (8u*(index%4u))) & 255u; }
fn pack4(a:u32,b:u32,c:u32,d:u32)->u32{return a|(b<<8u)|(c<<16u)|(d<<24u);}
fn threshold(index:u32)->u32{let a=abs(i32(current_byte(index))-i32(reference_byte(index)));
 let b=abs(i32(current_byte(BASE+index))-i32(reference_byte(BASE+index))); return u32(max(a,b)>=10);}
@compute @workgroup_size(64) fn threshold_base(@builtin(global_invocation_id) id:vec3<u32>){let first=id.x*4u;if(first>=BASE){return;}
 raw_base[id.x]=pack4(threshold(first),threshold(first+1u),threshold(first+2u),threshold(first+3u));}
fn promoted_code(index:u32)->u32{let row=index/COLS;let col=index%COLS;let y0=max(row,10u)-9u;let y1=min(row+10u,ROWS-1u)+1u;
 let x0=max(col,6u)-5u;let x1=min(col+6u,COLS-1u)+1u;var count=0u;for(var y=y0;y<y1;y++){for(var x=x0;x<x1;x++){count+=raw(y*COLS+x);}}
 let limit=u32(0.2*f32((y1-y0)*(x1-x0)));return select(raw(index),255u,count>=limit);}
@compute @workgroup_size(64) fn promote_base(@builtin(global_invocation_id) id:vec3<u32>){let first=id.x*4u;if(first>=BASE){return;}
 motion_base[id.x]=pack4(promoted_code(first),promoted_code(first+1u),promoted_code(first+2u),promoted_code(first+3u));}
fn average4(a:u32,b:u32,c:u32,d:u32)->u32{return(a+b+c+d+2u)/4u;}
fn half_base(index:u32)->u32{let r=(index/L1_COLS)*2u;let c=(index%L1_COLS)*2u;return average4(promoted(r*COLS+c),promoted(r*COLS+c+1u),promoted((r+1u)*COLS+c),promoted((r+1u)*COLS+c+1u));}
@compute @workgroup_size(64) fn reduce_l1(@builtin(global_invocation_id) id:vec3<u32>){let first=id.x*4u;if(first>=L1){return;}
 motion_l1[id.x]=pack4(half_base(first),half_base(first+1u),half_base(first+2u),half_base(first+3u));}
fn half_l1(index:u32)->u32{let r=(index/L2_COLS)*2u;let c=(index%L2_COLS)*2u;return average4(l1_at(r*L1_COLS+c),l1_at(r*L1_COLS+c+1u),l1_at((r+1u)*L1_COLS+c),l1_at((r+1u)*L1_COLS+c+1u));}
@compute @workgroup_size(64) fn reduce_l2(@builtin(global_invocation_id) id:vec3<u32>){let first=id.x*4u;if(first>=L2){return;}
 motion_l2[id.x]=pack4(half_l1(first),select(0u,half_l1(first+1u),first+1u<L2),select(0u,half_l1(first+2u),first+2u<L2),select(0u,half_l1(first+3u),first+3u<L2));}
fn ema(index:u32)->u32{let biased=fma(f32(reference_byte(index)),0.7,0.5);let blended=fma(f32(current_byte(index)),0.3,biased);return u32(clamp(trunc(blended),0.0,255.0));}
@compute @workgroup_size(64) fn cold_references(@builtin(global_invocation_id) id:vec3<u32>){if(id.x*4u>=2u*BASE){return;} next_ref[id.x]=current[id.x];}
@compute @workgroup_size(64) fn next_references(@builtin(global_invocation_id) id:vec3<u32>){let first=id.x*4u;if(first>=2u*BASE){return;}
 next_ref[id.x]=pack4(ema(first),ema(first+1u),ema(first+2u),ema(first+3u));}
"#;

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    use kjerag_media::FrameStamp;

    use super::super::super::resident_blurred_fixture;
    use super::*;

    struct RootRollbackOrderProbe {
        wait_state: Arc<AtomicU8>,
        capture: Arc<GpuResidentCapture>,
        dropped: mpsc::Sender<(u8, bool, usize)>,
    }

    impl Drop for RootRollbackOrderProbe {
        fn drop(&mut self) {
            let snapshot = self.capture.snapshot();
            let prior_strong_count = snapshot.committed.as_ref().map_or(0, Arc::strong_count);
            let _ = self.dropped.send((
                self.wait_state.load(Ordering::SeqCst),
                snapshot.pending,
                prior_strong_count,
            ));
        }
    }

    #[test]
    fn paired_resident_cadence_authenticates_each_direction_pre_increment() {
        let admission = |count, cadence| {
            EmptyOverrideCadence::new(count, cadence)
                .unwrap()
                .admission()
        };
        for count in [3, 9, 10, 11, 19, 20] {
            let paired = GpuPairedCadence::new(
                EmptyOverrideCadence::new(count, 10).unwrap(),
                EmptyOverrideCadence::new(count, 10).unwrap(),
            );
            let actual = paired.admissions();
            assert_eq!(actual.a_to_b, admission(count, 10));
            assert_eq!(actual.b_to_a, admission(count, 10));
            assert_eq!(
                paired.after_call().a_to_b.calc_count(),
                count.wrapping_add(1)
            );
        }

        let asymmetric = GpuPairedCadence::new(
            EmptyOverrideCadence::new(10, 10).unwrap(),
            EmptyOverrideCadence::new(12, 10).unwrap(),
        );
        assert_eq!(asymmetric.admissions().a_to_b, DescentAdmission::EveryPatch);
        assert_eq!(asymmetric.admissions().b_to_a, DescentAdmission::NoPatches);

        let selected = GpuPairedCadence::new(
            EmptyOverrideCadence::new(6_370, 10).unwrap(),
            EmptyOverrideCadence::new(6_372, 10).unwrap(),
        );
        assert_ne!(selected.admissions().a_to_b, selected.admissions().b_to_a);

        let wrapped = GpuPairedCadence::new(
            EmptyOverrideCadence::new(i32::MAX, 10).unwrap(),
            EmptyOverrideCadence::new(i32::MIN, 10).unwrap(),
        )
        .after_call();
        assert_eq!(wrapped.a_to_b.calc_count(), i32::MIN);
        assert_eq!(wrapped.b_to_a.calc_count(), i32::MIN.wrapping_add(1));
    }

    #[test]
    fn production_motion_kernel_is_bit_exact_on_the_actual_adapter() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping ONE X2 GPU temporal qualification: {why}");
                return;
            }
        };
        GpuMotionStage::new(OneXsGpuContext::new(&device, &queue)).unwrap_or_else(|error| {
            panic!("ONE X2 GPU temporal image state failed on {adapter}: {error}")
        });
    }

    #[test]
    fn qualification_refuses_live_motion_and_reference_mutations() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping ONE X2 GPU temporal mutations: {why}");
                return;
            }
        };
        let mutations = [
            (
                "both physical lenses",
                "return u32(max(a,b)>=10);",
                "return u32(a>=10);",
            ),
            (
                "inclusive threshold",
                "return u32(max(a,b)>=10);",
                "return u32(max(a,b)>10);",
            ),
            (
                "native low boundary",
                "let y0=max(row,10u)-9u;",
                "let y0=max(row,9u)-9u;",
            ),
            (
                "promotion population",
                "return select(raw(index),255u,count>=limit);",
                "return select(raw(index),255u,count>limit);",
            ),
            (
                "recursive area rounding",
                "return(a+b+c+d+2u)/4u;",
                "return(a+b+c+d+1u)/4u;",
            ),
            ("selected EMA weight", ",0.7,0.5);", ",0.6,0.5);"),
        ];
        for (name, old, new) in mutations {
            let mutated = SHADER.replacen(old, new, 1);
            assert_ne!(mutated, SHADER, "mutation {name} did not edit the shader");
            let error = match GpuMotionStage::from_shader(
                OneXsGpuContext::new(&device, &queue),
                &mutated,
                true,
            ) {
                Ok(_) => panic!("{name} mutation passed qualification on {adapter}"),
                Err(error) => error,
            };
            assert!(error.to_string().contains("not exact"), "{name}: {error}");
        }
    }

    #[test]
    fn qualification_pair_l2_motion_covers_packed_zero_and_nonzero_lanes() {
        let (reference, current) = qualification_pair();
        let expected = MotionPyramid::from_base(&MotionMask::between(&current, &reference));
        let l2 = expected.bytes(Level::Two);
        let mut histogram = std::collections::BTreeMap::new();
        for &code in l2 {
            *histogram.entry(code).or_insert(0usize) += 1;
        }
        eprintln!("ONE X2 qualification L2 motion histogram: {histogram:?}");
        assert_eq!(l2.len(), L2_BYTES);
        assert!(l2.contains(&0), "qualification motion has no still branch");
        assert!(
            l2.iter().any(|&code| code != 0),
            "qualification motion has no changed branch"
        );
        for lane in 0..CODES_PER_WORD {
            let lane_values = l2.iter().skip(lane).step_by(CODES_PER_WORD);
            assert!(
                lane_values.clone().any(|&code| code == 0)
                    && lane_values.clone().any(|&code| code != 0),
                "packed lane {lane} does not cover both motion branches"
            );
        }
        let edge_words = 2 * Level::Two.cols();
        for (name, edge) in [
            ("leading", &l2[..edge_words]),
            ("trailing", &l2[l2.len() - edge_words..]),
        ] {
            assert!(
                edge.contains(&0) && edge.iter().any(|&code| code != 0),
                "{name} L2 edge does not cover both motion branches"
            );
        }
    }

    #[test]
    fn test_only_successor_install_leaves_ready_unpublished_and_drop_retries_exactly() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping ONE X2 GPU temporal rollback: {why}");
                return;
            }
        };
        let stage = GpuMotionStage::new(OneXsGpuContext::new(&device, &queue))
            .unwrap_or_else(|error| panic!("GPU qualification failed on {adapter}: {error}"));
        let capture = stage.new_capture();
        let (first_input, second_input) = qualification_pair();
        let first_flight = flight(1, 1);
        let first_reservation = capture.reserve(first_flight.frame.clone()).unwrap();
        let first_flight = first_reservation.flight().clone();
        let first_owner = Arc::new(());
        let (first_belts, first) = resident_blurred_fixture(
            &device,
            &queue,
            Arc::clone(&first_owner),
            first_flight.clone(),
            &first_input,
        )
        .unwrap();
        let cold = first_belts
            .prepare_motion(&stage, first_reservation)
            .unwrap();
        assert_eq!(cold.receipt().history, GpuMotionHistory::Cold);
        assert!(cold.base().is_none());
        let cold_frame = cold.into_frame_candidate();
        assert_eq!(cold_frame.receipt().flight, first_flight);
        assert_eq!(Arc::strong_count(&first_owner), 2);
        {
            let state = capture.snapshot();
            assert_eq!(state.generation, 1);
            assert!(state.committed.is_none());
            assert!(state.pending);
            assert!(!state.ready);
        }
        let cold_frame = cold_frame.publish_at_install_for_test().unwrap();
        let (generation, reference) = {
            let state = capture.snapshot();
            (
                state.generation,
                Arc::clone(state.committed.as_ref().unwrap()),
            )
        };
        assert_eq!(
            read(&device, &queue, reference.motion_reference(), BELT_BYTES).unwrap(),
            first.bytes()
        );
        drop(cold_frame);
        assert_eq!(Arc::strong_count(&first_owner), 1);

        let second_flight = flight(2, 2);
        let second_reservation = capture.reserve(second_flight.frame.clone()).unwrap();
        let second_flight = second_reservation.flight().clone();
        let dropped_owner = Arc::new(());
        let (second_belts, second) = resident_blurred_fixture(
            &device,
            &queue,
            Arc::clone(&dropped_owner),
            second_flight.clone(),
            &second_input,
        )
        .unwrap();
        let warm = second_belts
            .prepare_motion(&stage, second_reservation)
            .unwrap();
        let warm_frame = warm.into_frame_candidate();
        assert_eq!(warm_frame.receipt().history, GpuMotionHistory::Warm);
        assert!(warm_frame.base().is_some());
        assert!(warm_frame.level(Level::One).is_some());
        assert!(warm_frame.level(Level::Two).is_some());
        drop(warm_frame);
        assert_eq!(Arc::strong_count(&dropped_owner), 1);
        {
            let state = capture.snapshot();
            assert_eq!(state.generation, generation + 1);
            assert!(Arc::ptr_eq(state.committed.as_ref().unwrap(), &reference));
            assert!(!state.pending);
            assert!(!state.ready);
        }

        let recovered_owner = Arc::new(());
        let recovered_reservation = capture.reserve(second_flight.frame.clone()).unwrap();
        let recovered_flight = recovered_reservation.flight().clone();
        let (recovered_belts, recovered_second) = resident_blurred_fixture(
            &device,
            &queue,
            Arc::clone(&recovered_owner),
            recovered_flight.clone(),
            &second_input,
        )
        .unwrap();
        assert_eq!(recovered_second, second);
        let recovered = recovered_belts
            .prepare_motion(&stage, recovered_reservation)
            .unwrap();
        let frame = recovered.into_frame_candidate();
        assert_eq!(frame.receipt().flight, recovered_flight);
        assert_eq!(Arc::strong_count(&recovered_owner), 2);
        let expected_mask = MotionMask::between(&second, &first);
        let expected_pyramid = MotionPyramid::from_base(&expected_mask);
        let expected_reference = next_warm_references(&first, &second);
        let expected_l2_motion = expected_pyramid.bytes(Level::Two);
        assert_eq!(expected_l2_motion.len(), L2_BYTES);
        assert!(
            expected_l2_motion.contains(&0) && expected_l2_motion.iter().any(|&code| code != 0),
            "production-motion qualification needs both packed branch values"
        );
        GpuL2PostPisBridge::new(OneXsGpuContext::new(&device, &queue))
            .unwrap_or_else(|error| panic!("L2 bridge qualification failed on {adapter}: {error}"))
            .qualify_production_motion_buffer_for_test(
                frame.level(Level::Two).unwrap(),
                expected_l2_motion,
            )
            .unwrap_or_else(|error| {
                panic!("production temporal motion was decoded incorrectly on {adapter}: {error}")
            });
        assert_eq!(
            read(&device, &queue, frame.current(), BELT_BYTES).unwrap(),
            second.bytes()
        );
        assert_eq!(
            read(&device, &queue, frame.base().unwrap(), BASE_BYTES).unwrap(),
            expected_mask.bytes()
        );
        for (level, expected) in [
            (Level::One, expected_pyramid.bytes(Level::One)),
            (Level::Two, expected_pyramid.bytes(Level::Two)),
        ] {
            assert_eq!(
                read(&device, &queue, frame.level(level).unwrap(), expected.len()).unwrap(),
                expected
            );
        }
        {
            let state = capture.snapshot();
            assert_eq!(state.generation, generation + 2);
            assert!(Arc::ptr_eq(state.committed.as_ref().unwrap(), &reference));
            assert!(state.pending);
            assert!(!state.ready);
        }
        let frame = frame.publish_at_install_for_test().unwrap();
        {
            let state = capture.snapshot();
            assert_eq!(state.generation, generation + 2);
            assert!(!Arc::ptr_eq(state.committed.as_ref().unwrap(), &reference));
            assert_eq!(
                read(
                    &device,
                    &queue,
                    state.committed.as_ref().unwrap().motion_reference(),
                    BELT_BYTES
                )
                .unwrap(),
                expected_reference.bytes()
            );
            assert!(!state.ready);
        }
        drop(frame);
        assert_eq!(Arc::strong_count(&recovered_owner), 1);
    }

    #[test]
    fn every_motion_boundary_retires_the_carrier_before_root_rollback() {
        #[derive(Clone, Copy, Debug)]
        enum Boundary {
            TransactionDrop,
            TransactionAbort,
            FrameDrop,
            FrameAbort,
            TransactionUnwind,
        }

        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping ONE X2 GPU rollback ordering: {why}");
                return;
            }
        };
        let stage = GpuMotionStage::new(OneXsGpuContext::new(&device, &queue))
            .unwrap_or_else(|error| panic!("GPU qualification failed on {adapter}: {error}"));
        let capture = Arc::new(stage.new_capture());

        // Install one exact allocation so every boundary below is warm and
        // owns an immutable prior snapshot while submitted motion can read it.
        let initial = fixture(17);
        let initial_reservation = capture
            .reserve(FrameStamp::for_test(1, Duration::from_secs(1), None))
            .unwrap();
        let (initial_belts, _) = resident_blurred_fixture(
            &device,
            &queue,
            (),
            initial_reservation.flight().clone(),
            &initial,
        )
        .unwrap();
        let initial_frame = initial_belts
            .prepare_motion(&stage, initial_reservation)
            .unwrap()
            .into_frame_candidate()
            .publish_at_install_for_test()
            .unwrap();
        drop(initial_frame);

        for (ordinal, boundary) in [
            Boundary::TransactionDrop,
            Boundary::TransactionAbort,
            Boundary::FrameDrop,
            Boundary::FrameAbort,
            Boundary::TransactionUnwind,
        ]
        .into_iter()
        .enumerate()
        {
            let wait_state = Arc::new(AtomicU8::new(0));
            let (dropped, answer) = mpsc::channel();
            let reservation = capture
                .reserve(FrameStamp::for_test(
                    ordinal as u64 + 2,
                    Duration::from_secs(ordinal as u64 + 2),
                    None,
                ))
                .unwrap();
            let (mut belts, _) = resident_blurred_fixture(
                &device,
                &queue,
                RootRollbackOrderProbe {
                    wait_state: Arc::clone(&wait_state),
                    capture: Arc::clone(&capture),
                    dropped,
                },
                reservation.flight().clone(),
                &fixture(31 + ordinal as u32),
            )
            .unwrap();
            belts.observe_completion(Arc::clone(&wait_state));
            let transaction = belts.prepare_motion(&stage, reservation).unwrap();
            assert_eq!(transaction.receipt().history, GpuMotionHistory::Warm);

            match boundary {
                Boundary::TransactionDrop => drop(transaction),
                Boundary::TransactionAbort => transaction.abort().unwrap(),
                Boundary::FrameDrop => drop(transaction.into_frame_candidate()),
                Boundary::FrameAbort => transaction.into_frame_candidate().abort().unwrap(),
                Boundary::TransactionUnwind => {
                    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _owned = transaction;
                        panic!("injected downstream unwind");
                    }));
                    assert!(unwound.is_err());
                }
            }

            let (wait_observed, pending_observed, prior_strong_count) = answer.recv().unwrap();
            assert_eq!(
                wait_observed, 2,
                "{boundary:?} released its carrier before poll"
            );
            assert!(
                pending_observed,
                "{boundary:?} rolled the root back before retiring its carrier"
            );
            assert!(
                prior_strong_count >= 3,
                "{boundary:?} released its reservation's prior before carrier retirement"
            );
            assert_eq!(wait_state.load(Ordering::SeqCst), 2);
            assert!(!capture.snapshot().pending);
        }
    }

    #[test]
    fn foreign_context_is_refused_before_temporal_reservation() {
        let (device, queue, foreign_device, foreign_queue, adapter) = match gpu_pair() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping ONE X2 GPU temporal provenance: {why}");
                return;
            }
        };
        let stage = GpuMotionStage::new(OneXsGpuContext::new(&foreign_device, &foreign_queue))
            .unwrap_or_else(|error| {
                panic!("foreign GPU motion stage failed on {adapter}: {error}")
            });
        let capture = stage.new_capture();
        let owner = Arc::new(());
        let input = fixture(77);
        let reservation = capture.reserve(flight(9, 9).frame).unwrap();
        let reserved_flight = reservation.flight().clone();
        let (belts, _) =
            resident_blurred_fixture(&device, &queue, Arc::clone(&owner), reserved_flight, &input)
                .unwrap();
        let error = match belts.prepare_motion(&stage, reservation) {
            Ok(_) => panic!("foreign GPU motion context was accepted"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("crossed a different device or queue"),
            "{error}"
        );
        assert_eq!(Arc::strong_count(&owner), 1);
        let state = capture.snapshot();
        assert_eq!(state.generation, 1);
        assert!(!state.pending);
        assert!(state.committed.is_none());
        assert!(!state.ready);
    }

    fn flight(index: u64, seconds: u64) -> GpuPisFlight {
        GpuPisFlight {
            generation: index,
            frame: FrameStamp::for_test(index, Duration::from_secs(seconds), None),
        }
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
        let info = adapter.get_info();
        let name = format!("{} ({})", info.name, info.driver);
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("exact ONE X2 GPU temporal image state"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        Ok((device, queue, name))
    }

    fn gpu_pair() -> Result<(wgpu::Device, wgpu::Queue, wgpu::Device, wgpu::Queue, String), String>
    {
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
                label: Some("exact ONE X2 GPU temporal provenance"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            }))
            .map_err(|error| error.to_string())
        };
        let (device, queue) = request()?;
        let (foreign_device, foreign_queue) = request()?;
        Ok((device, queue, foreign_device, foreign_queue, name))
    }
}
