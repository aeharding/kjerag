//! The one shader pass the player draws.
//!
//! With no frame it draws nothing at all: every ray misses every lens, which
//! is the transparent room the ball floats in (issue #100), so what the pilot
//! sees between opening a file and its first frame is the backdrop the shell
//! paints behind this widget. With a frame it reprojects a real VA-API frame
//! imported by [`super::dmabuf`]: for every output pixel, a view ray
//! through the camera's yaw, pitch and field of view, rotated into each
//! lens's frame and pushed through the Mei/UCM model in
//! [`super::projection`], sampled from the NV12 planes of whichever lens
//! wins and converted to RGB. One pass, no intermediate target.
//!
//! The frames move (issue #4): a [`Player`] decodes both lenses on its own
//! thread and this file asks it, on every redraw, which pair belongs on
//! screen; [`ScenePipeline::prepare`] imports that pair and binds it. Both
//! lenses are sampled as well as imported, so the picture is the whole
//! sphere (issue #27), and where the two overlap the pass mixes them by the
//! weight field in [`super::projection`] rather than picking one (issue #7).
//! Outside the overlap that weight is exactly 1 and the fetch is the single
//! fetch it always was.
//!
//! [`Scene::pump`] takes `&self` and keeps the clock behind a [`RefCell`],
//! which is not how a player would be written on its own. It is how iced's
//! `shader::Program` is shaped: `update` and `draw` both borrow the program
//! immutably, and the pump has to happen inside the redraw pass, before the
//! draw, or the picture is always one refresh behind the clock. The cell is
//! touched from the UI thread only; the decode thread never sees it.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kjerag_media::{Accuracy, Cue, FrameStamp, Frames, Player, PresentationPolicy, Reader, Stats};
use kjerag_meta::{
    CalibrationSet, ExposureTrack, Filter, Format, Lens, OrientationTrack, Quat, Readout,
};

use super::band::{self, Table};
use super::capture::{self, Order, Pending, Request, Shutter, Stamp};
use super::chroma;
use super::direct_type2::DirectMapDraw;
use super::flow::one_xs::gpu_context::OneXsGpuContext;
use super::flow::one_xs::pis::gpu::{
    GpuPisFlight, GpuPisPipeline, GpuPisStageOutput, GpuPisStageReceipt,
};
#[cfg(test)]
use super::flow::one_xs::player::FrameOwnerError;
use super::flow::one_xs::player::{FrameCommitError, FrameOwner, FrameResult, PreparedFrame};
use super::flow::one_xs::scalar::{
    ColdInputs, ColdPreparedSchedule, CpuPisOracleInputs, PairedControlInputs, PairedPatchGrids,
    PairedPisSolver, PairedSolveRequest,
};
use super::flow::one_xs::temporal::BlurredBelts;
use super::flow::one_xs_belt_gpu::{GpuSolverBeltPipeline, PendingBlurredBelts, SourceTextures};
use super::flow::{Cadence, Estimate};
use super::one_xs_luma::{self, LumaReadbackPipeline, PendingOneXsLuma};
use super::projection::{self, Held, MAX_LENSES, Reframe, Rolling, SeamAnchor};
use super::sampling::{self, Sampling};
use super::seam::{Correction, SeamFit};
use super::stall::{Stall, Stalled};
use super::studio_type2::{
    MapBindError, OneXsMapFrame, OneXsMapRaster, PisBackend, PreparedPicture,
};
use super::{Camera, Extent, Fallible, Nudge, Planes, Size, Viewpoint, dmabuf};

/// The sampler binding, which sits after every lens's two planes.
const SAMPLER_BINDING: u32 = 1 + 2 * MAX_LENSES as u32;

/// Frames kept alive behind the one being drawn.
///
/// An imported texture aliases the decoder's surface: dropping the
/// [`Frames`] hands that surface back to the decoder, which will write the
/// next picture into it. The GPU may still be reading it, because iced
/// submits after `prepare` returns and presents later still, so a frame is
/// released only once this many newer ones have been bound.
const RETAINED: usize = 3;

/// When the widget should come back, which is the whole of frame pacing:
/// the shell sleeps until the instant the next frame is due rather than
/// polling, so 29.97 fps content costs 29.97 redraws a second.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// Whenever the compositor will take a frame: playback that is still
    /// waiting for its first decoded frame, and a seek that has not landed.
    Refresh,
    /// At this instant, when the frame after the one just taken is due.
    At(Instant),
    /// Nothing changes by itself: paused, ended, or a still frame.
    Never,
    /// The picture is gone and the file has been stopped, sound and all
    /// (issue #124). Nothing changes by itself here either, and the
    /// difference is that somebody has to be told: this arm is how a failure
    /// in the pass reaches the shell's alert, and there is no other way out
    /// of this crate for one.
    Stopped(Stall),
}

/// Which clock a decoded frame's instant is read on, before the camera's
/// orientation is looked up at that instant (issue #8).
///
/// The player uses [`Self::Exposure`] and this exists so the losing
/// hypothesis stays measurable: `kjerag-spike --bin horizon` renders the same
/// frames both ways and reports what the difference is worth in degrees of
/// horizon tilt. Nothing in the shell offers the choice.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FrameClock {
    /// The exposure record's own timestamp for this frame, which is the
    /// camera's clock and the one `pts_type = 2` names
    /// ([`ExposureTrack::frame_time_us`]).
    #[default]
    Exposure,
    /// The container's PTS, which is a nominal 30000/1001 grid.
    Container,
}

/// Whether the picture is held against the world or against the camera body.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Horizon {
    /// The world stays put: the body's roll, pitch and heading are all taken
    /// out, so the view is pointed at a direction in the world and the
    /// aircraft turns underneath it (owner ruling, 2026-08-06). Until then
    /// the heading was high passed and a deliberate turn carried the picture
    /// round with it; what is left of the lock's own motion is the
    /// gyroscope's yaw drift, which nothing in these files can bound.
    #[default]
    Locked,
    /// The view rides the camera, as it did before issue #8.
    Free,
}

/// The widget's state, owned by the shell.
pub struct Scene {
    show: Option<Show>,
    /// Where a capture waits for the redraw that takes it (issue #15).
    shutter: Shutter,
    /// Set while the shell has hidden the controls, which is when the pointer
    /// goes with them: `mouse_interaction` answers `Hidden` instead of `Grab`
    /// (docs/UI.md, "The cursor"). One bit of shell state, and the only one
    /// this crate carries.
    cursor_hidden: bool,
    /// Where the view points, and any drag that has hold of it.
    ///
    /// Here rather than in the shader widget's own `State`, which is where
    /// iced would keep it and where issue #77 found it. Widget state lives in
    /// the widget tree, and the tree is rebuilt from the shell's `view`
    /// whenever the window changes shape. The header bar coming and going is
    /// one of those changes -- libcosmic pushes it into the same column as
    /// the content (`src/app/mod.rs:775`), so hiding it moves the content up
    /// a place -- and it goes on entering fullscreen, on leaving it, and two
    /// seconds after the pointer stops while a file plays. Measured
    /// 2026-07-31 through the headless harness: with the bar pinned up every
    /// one of those transitions held the view, and with it free each one put
    /// the camera back to [`Camera::default`].
    ///
    /// The [`Scene`] is the shell's own, and the shell's own state outlives
    /// its view. A [`Cell`] for the same reason the clock is one:
    /// `shader::Program` hands out `&self` and nothing else.
    viewpoint: Cell<Viewpoint>,
    /// A [`Nudge`] the `View` menu left for the widget. Read once, by the
    /// next redraw, which is where the output's shape is known.
    nudge: Cell<Option<Nudge>>,
    /// `View > Lock horizon`. Read on every redraw rather than taken, because
    /// it is a state and not an event.
    horizon: Cell<Horizon>,
    /// Which clock the orientation is looked up on. The instruments move it;
    /// the shell does not.
    clock: Cell<FrameClock>,
    /// An orientation the harness has forced in place of the file's own.
    forced: Cell<Option<Quat>>,
    /// A sensor readout the harness has forced in place of the file's own,
    /// including a zero one, which is the correction switched off.
    readout: Cell<Option<Readout>>,
    /// How the pass samples where the view magnifies the source (issue #11).
    /// The instruments move it; the shell leaves it alone.
    sampling: Cell<Sampling>,
    /// The available Studio-derived optical-flow seam correction on or off,
    /// the player's runtime toggle and the "Optical Flow" arm of the stitching
    /// control. **DEFAULT OFF**:
    /// off, the pass is byte-identical to the shipped player; on, the pipeline
    /// estimates the flow on a BACKGROUND WORKER on Studio's ~30-frame cadence and
    /// applies the last field the worker returned, held between updates
    /// ([`ScenePipeline::flow_step`], §35) — the render thread never runs the DIS.
    /// A cell for the reason the toggles above are: `shader::Program` hands out
    /// `&self`.
    flow: Cell<bool>,
    /// Where the pass leaves word that it cannot draw this file any more
    /// (issue #124). It belongs to the open capture rather than to the
    /// pipeline, which outlives every file it draws.
    stalled: Stalled,
    /// And what it last managed to draw of this file, for the same reason.
    shown: Shown,
}

/// How the picture is to be held for one redraw: the shell's own toggle, and
/// the three overrides the headless instruments reach for.
#[derive(Clone, Copy, Debug)]
struct Holding {
    horizon: Horizon,
    clock: FrameClock,
    forced: Option<Quat>,
    readout: Option<Readout>,
}

fn supports_player_flow(lenses: Option<&[Lens]>) -> bool {
    lenses.is_none_or(|lenses| !projection::is_one_xs_lens_pair(lenses))
}

/// The generated static resources and direct renderer have passed their
/// authenticated substitution and rendered-pixel gates. Keep the activation
/// explicit and reviewable; it is not a user-facing quality switch.
const ONE_XS_PLAYBACK_ENABLED: bool = true;

fn one_xs_playback_selected(enabled: bool, capture_owned: bool) -> bool {
    enabled && capture_owned
}

/// Whether the player must keep offering its current selected frame.
///
/// `None` is the initial state before frame zero has been offered, so it must
/// not close the gate. Once there is a current frame, only an exact ready-map
/// acknowledgement lets the EveryFrame player hand over its successor.
fn one_xs_frame_waiting(enabled: bool, capture_owned: bool, current_ready: Option<bool>) -> bool {
    one_xs_playback_selected(enabled, capture_owned) && current_ready == Some(false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReplayStart {
    Continue,
    FrameZero,
}

/// Decide whether the causal lineage can reach `target` by continuing the
/// decoder exactly where it stands. `exact_ready` binds the offered surfaces
/// to the completed map by full opaque identity; the adjacency arm covers the
/// one offered successor which has not completed its transaction yet.
fn one_xs_replay_start(
    ready: Option<u64>,
    offered: Option<u64>,
    exact_ready: bool,
    same_epoch: bool,
    target: u64,
) -> ReplayStart {
    let Some(ready) = ready else {
        return ReplayStart::FrameZero;
    };
    let Some(offered) = offered else {
        return ReplayStart::FrameZero;
    };
    let safe_head =
        exact_ready || (same_epoch && ready.checked_add(1).is_some_and(|next| next == offered));
    if safe_head && target >= offered {
        ReplayStart::Continue
    } else {
        ReplayStart::FrameZero
    }
}

fn selected_replay_start(
    proposed: ReplayStart,
    allow_forward: bool,
    replaying: bool,
    offered: Option<u64>,
    target: u64,
) -> ReplayStart {
    if replaying {
        return ReplayStart::FrameZero;
    }
    match (proposed, allow_forward, offered) {
        (ReplayStart::Continue, true, _) => ReplayStart::Continue,
        (ReplayStart::Continue, false, Some(offered)) if offered == target => ReplayStart::Continue,
        _ => ReplayStart::FrameZero,
    }
}

fn replay_step_base(requested: Option<u64>, decoded: Option<u64>) -> Option<u64> {
    requested.or(decoded)
}

fn retire_replay<T>(replay: &RefCell<Option<T>>) {
    replay.borrow_mut().take();
}

fn next_after_pump(replaying: bool, playing: bool, seeking: bool, due: Option<Instant>) -> Next {
    match (replaying, playing, seeking, due) {
        // Decoder landing is deliberately earlier than selected transaction
        // completion. Keep one redraw in flight so prepare can build the
        // target map and the following pump can observe its acknowledgement.
        (true, _, _, _) => Next::Refresh,
        (false, false, true, _) => Next::Refresh,
        (false, false, false, _) => Next::Never,
        (false, true, _, Some(due)) => Next::At(due),
        (false, true, _, None) => Next::Refresh,
    }
}

fn exact_selected_display<T: Eq>(current: &T, shown: Option<&T>, same_capture: bool) -> bool {
    same_capture && shown == Some(current)
}

/// A file on screen: its calibration, and where its frames come from.
struct Show {
    /// Containers in the exact decoder lane order admitted at open.
    files: Arc<[PathBuf]>,
    /// The size of one lens's decoded frame.
    frame: Size,
    /// One per decoded stream, in stream order, as the camera calibrated
    /// them.
    lenses: Arc<[Lens]>,
    /// What names the camera these came off, serial-free
    /// ([`CalibrationSet::camera_key`]). The seam calibration is stored under
    /// it.
    camera: u64,
    /// The lenses with a seam correction in them, or the factory calibration
    /// where there is none. A manual [`Scene::use_seam`] lands a fit here for RE
    /// and testing; the player never does, so what it draws is the factory
    /// parity base (the per-capture fit of issue #48 was removed 2026-08-15).
    /// Shared because it is read on every redraw.
    corrected: Arc<Correction>,
    /// The along-seam table this camera has been read at, landed at open
    /// (issue #103, stage 9). A cell because it is set once from outside and
    /// read on every redraw, exactly like the toggles above.
    table: Cell<Table>,
    /// Where the camera body was, over the whole file, and the camera's own
    /// timestamp for each frame. Both come out of the trailer at open
    /// (issue #8); both are empty for a file with no IMU record, and then
    /// horizon lock is a no-op rather than an error.
    held: Arc<Motion>,
    /// Sequential selected ONE X2 state for ordinary live playback only.
    one_xs: Option<Arc<OneXsCapture>>,
    /// Factory input for a genuinely fresh causal lineage after a restart.
    one_xs_calibration: Option<Arc<CalibrationSet>>,
    /// A requested target remains a seek until its exact map, not merely its
    /// decoded surfaces, has completed the capture transaction.
    replay: RefCell<Option<OneXsReplay>>,
    /// The clock and the frame it is showing. See the module docs for why
    /// this is a cell.
    playing: RefCell<Playing>,
}

/// What the trailer says about how the camera moved.
struct Motion {
    orientation: OrientationTrack,
    /// Lens 0's shutter track, read for its timestamps rather than its
    /// shutters: `pts_type = 2` makes it the camera's own frame clock.
    exposure: ExposureTrack,
    /// How long one frame takes to come off the sensor and which way it
    /// comes, which is what issue #9's correction is measured against.
    readout: Readout,
}

/// The selected stitch state belonging to one open live capture.
///
/// iced owns [`ScenePipeline`] independently from [`Scene`], so the sequential
/// CPU owner cannot live only in either one. A live [`View`] carries this
/// shared capture identity across that boundary. The mutex protects only the
/// short reservation and installation boundaries. A reservation moves the
/// sequential owner out, so GPU waits and estimator work hold no capture lock;
/// its generation keeps a recreated pipeline from restarting or duplicating
/// the capture's numeric lineage.
struct OneXsCapture {
    state: Mutex<OneXsCaptureState>,
}

#[derive(Clone, Copy, Debug)]
struct OneXsReplay {
    target: u64,
    position: Duration,
    playing: bool,
}

struct OneXsCaptureState {
    /// Ordinary playback. The last completed resources are retained so a
    /// redraw of the exact same delivered pair does not consume the
    /// sequential owner twice.
    owner: Option<Box<FrameOwner>>,
    ready: Option<Arc<OneXsMapFrame>>,
    generation: u64,
    in_flight: Option<GpuPisFlight>,
    /// A transaction that could not be restored or installed makes this
    /// lineage terminal. The old ready display remains available, but it is
    /// not a truthful base for another successor transaction.
    terminal: bool,
    /// Owners from stale tokens are quarantined rather than allowed to
    /// overwrite the owner leased to a different current flight.
    quarantined_owners: Vec<FrameOwner>,
}

enum OneXsPreparation {
    Ready(Arc<OneXsMapFrame>),
    Reserved(OneXsReservation),
    InFlight(Option<Arc<OneXsMapFrame>>),
}

/// The exact old owner and prepared geometry leased out of one capture.
///
/// Future staged GPU PIS work may retain this value across all of its waits.
/// Until [`Self::commit_prepared_with_solver`] succeeds, aborting it restores
/// the exact box that was installed before the reservation; no estimator clone
/// is involved.
struct OneXsReservation {
    capture: Arc<OneXsCapture>,
    flight: GpuPisFlight,
    previous_ready: Option<Arc<OneXsMapFrame>>,
    owner: Option<Box<FrameOwner>>,
    prepared: Option<Box<PreparedFrame>>,
}

struct CompletedOneXsReservation {
    capture: Arc<OneXsCapture>,
    flight: GpuPisFlight,
    previous_ready: Option<Arc<OneXsMapFrame>>,
    owner: Option<Box<FrameOwner>>,
    result: Option<FrameResult>,
    pis_backend: PisBackend,
}

#[cfg(test)]
struct RejectedOneXsReservation {
    reservation: OneXsReservation,
    error: FrameOwnerError,
}

struct RejectedOneXsSolverReservation<E> {
    reservation: OneXsReservation,
    error: FrameCommitError<E>,
}

struct RejectedOneXsAbort {
    reservation: OneXsReservation,
    reason: String,
}

struct RejectedOneXsInstall {
    completion: CompletedOneXsReservation,
    reason: String,
}

impl std::fmt::Debug for RejectedOneXsInstall {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output
            .debug_struct("RejectedOneXsInstall")
            .field("flight", &self.completion.flight)
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for RejectedOneXsInstall {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(&self.reason)
    }
}

impl std::error::Error for RejectedOneXsInstall {}

impl std::fmt::Debug for RejectedOneXsAbort {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output
            .debug_struct("RejectedOneXsAbort")
            .field("flight", &self.reservation.flight)
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for RejectedOneXsAbort {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(&self.reason)
    }
}

impl std::error::Error for RejectedOneXsAbort {}

#[derive(Debug)]
struct OneXsRollbackFailure {
    primary: Box<dyn std::error::Error + Send + Sync>,
    rollback: Box<RejectedOneXsAbort>,
}

impl std::fmt::Display for OneXsRollbackFailure {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            output,
            "{}; restoring the ONE X2 reservation also failed: {}",
            self.primary, self.rollback
        )
    }
}

impl std::error::Error for OneXsRollbackFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.primary.as_ref())
    }
}

fn abort_one_xs_after_error(
    reservation: OneXsReservation,
    primary: Box<dyn std::error::Error + Send + Sync>,
) -> Box<dyn std::error::Error + Send + Sync> {
    match reservation.abort() {
        Ok(()) => primary,
        Err(rollback) => Box::new(OneXsRollbackFailure { primary, rollback }),
    }
}

/// One submitted compact solver input and the exact delivery it sampled.
///
/// [`PendingBlurredBelts`] retains the decoder surfaces themselves. This outer
/// token retains their opaque numeric identity as well, so the readback cannot
/// be committed to geometry prepared for another delivery.
struct PendingOneXsBlurredBelts {
    frame: FrameStamp,
    pending: PendingBlurredBelts<Arc<Frames>>,
}

/// Failure before a GPU PIS result can re-enter its capture reservation.
#[derive(Debug)]
enum GpuPisSolverError {
    Pipeline(Box<dyn std::error::Error + Send + Sync>),
    Receipt {
        expected: Box<GpuPisStageReceipt>,
        actual: Box<GpuPisStageReceipt>,
    },
}

impl std::fmt::Display for GpuPisSolverError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pipeline(source) => source.fmt(output),
            Self::Receipt { expected, actual } => write!(
                output,
                "ONE X2 GPU PIS completion receipt is {actual:?}, expected {expected:?}"
            ),
        }
    }
}

impl std::error::Error for GpuPisSolverError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Pipeline(source) => Some(source.as_ref()),
            Self::Receipt { .. } => None,
        }
    }
}

/// The only production adapter from one reservation into the GPU kernel.
struct ReservationGpuPisSolver<'a> {
    flight: GpuPisFlight,
    pipeline: &'a GpuPisPipeline,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    completed_stages: &'a mut u64,
    prepared: CpuPisOracleInputs,
}

impl PairedPisSolver for ReservationGpuPisSolver<'_> {
    type Error = GpuPisSolverError;
    const BACKEND: super::studio_type2::PisBackend = super::studio_type2::PisBackend::Gpu;

    fn solve(&mut self, request: PairedSolveRequest) -> Result<PairedPatchGrids, Self::Error> {
        let expected = GpuPisStageReceipt {
            flight: self.flight.clone(),
            stage: request.stage,
        };
        let output = self
            .pipeline
            .solve_request(
                self.device,
                self.queue,
                expected.clone(),
                &self.prepared,
                request,
            )
            .map_err(GpuPisSolverError::Pipeline)?;
        *self.completed_stages = self
            .completed_stages
            .checked_add(1)
            .expect("ONE X2 GPU PIS completed-stage counter is exhausted");
        finish_gpu_pis_stage(&expected, output)
    }
}

fn finish_gpu_pis_stage(
    expected: &GpuPisStageReceipt,
    output: GpuPisStageOutput,
) -> Result<PairedPatchGrids, GpuPisSolverError> {
    validate_gpu_pis_receipt(expected, &output.receipt)?;
    Ok(output.grids)
}

fn validate_gpu_pis_receipt(
    expected: &GpuPisStageReceipt,
    actual: &GpuPisStageReceipt,
) -> Result<(), GpuPisSolverError> {
    if actual == expected {
        Ok(())
    } else {
        Err(GpuPisSolverError::Receipt {
            expected: Box::new(expected.clone()),
            actual: Box::new(actual.clone()),
        })
    }
}

impl PendingOneXsBlurredBelts {
    fn read(self, prepared: &PreparedFrame) -> Fallible<BlurredBelts> {
        if &self.frame != prepared.frame() {
            return Err(crate::studio_type2::FrameMapMismatch::new(
                "GPU solver belts",
                &self.frame,
                "prepared geometry",
                prepared.frame(),
            )
            .into());
        }
        self.pending.read()
    }
}

impl std::fmt::Debug for OneXsCapture {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str("OneXsCapture")
    }
}

fn same_ready_map(left: Option<&Arc<OneXsMapFrame>>, right: Option<&Arc<OneXsMapFrame>>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        _ => false,
    }
}

impl OneXsReservation {
    fn prepared(&self) -> &PreparedFrame {
        self.prepared
            .as_deref()
            .expect("a live ONE X2 reservation owns prepared geometry")
    }

    /// Restore the exact pre-transaction owner after submission or readback
    /// fails. The ready map is validated by allocation identity and is never
    /// replaced by rollback.
    fn abort(mut self) -> Result<(), Box<RejectedOneXsAbort>> {
        self.prepared.take();
        let capture = self.capture.clone();
        if let Err(error) =
            capture.restore(&self.flight, self.previous_ready.as_ref(), &mut self.owner)
        {
            return Err(Box::new(RejectedOneXsAbort {
                reservation: self,
                reason: error.to_string(),
            }));
        }
        Ok(())
    }

    /// Run today's scalar estimator without the capture mutex.
    ///
    /// `FrameOwner::commit` leaves its owner usable on every current error.
    /// Returning the reservation on rejection preserves that fact at the
    /// capture boundary instead of dropping the leased owner.
    #[cfg(test)]
    fn commit_scalar(
        mut self,
        blurred_belts: BlurredBelts,
    ) -> Result<CompletedOneXsReservation, RejectedOneXsReservation> {
        let prepared = self
            .prepared
            .take()
            .expect("a live ONE X2 reservation owns prepared geometry");
        let owner = self
            .owner
            .as_deref_mut()
            .expect("a live ONE X2 reservation owns the old estimator");
        match owner.commit(*prepared, blurred_belts) {
            Ok(result) => Ok(CompletedOneXsReservation {
                capture: self.capture.clone(),
                flight: self.flight.clone(),
                previous_ready: self.previous_ready.clone(),
                owner: Some(
                    self.owner
                        .take()
                        .expect("the successful scalar transaction retains its next owner"),
                ),
                result: Some(result),
                pis_backend:
                    <super::flow::one_xs::scalar::CpuPairedPisSolver as PairedPisSolver>::BACKEND,
            }),
            Err(error) => Err(RejectedOneXsReservation {
                reservation: self,
                error,
            }),
        }
    }

    /// Run a fallible paired solver while retaining the outer reservation.
    ///
    /// `FrameOwner::commit_prepared_with_solver` restores the exact old
    /// estimator on every solver and stamp error. Returning this reservation
    /// lets the capture restore that owner and its allocation-identical ready
    /// map.
    fn commit_prepared_with_solver<S: PairedPisSolver>(
        mut self,
        controls: PairedControlInputs,
        solver: &mut S,
    ) -> Result<CompletedOneXsReservation, Box<RejectedOneXsSolverReservation<S::Error>>> {
        let prepared = self
            .prepared
            .take()
            .expect("a live ONE X2 reservation owns prepared geometry");
        let owner = self
            .owner
            .as_deref_mut()
            .expect("a live ONE X2 reservation owns the old estimator");
        match owner.commit_prepared_with_solver(*prepared, controls, solver) {
            Ok(result) => Ok(CompletedOneXsReservation {
                capture: self.capture.clone(),
                flight: self.flight.clone(),
                previous_ready: self.previous_ready.clone(),
                owner: Some(
                    self.owner
                        .take()
                        .expect("the successful solver transaction retains its next owner"),
                ),
                result: Some(result),
                pis_backend: S::BACKEND,
            }),
            Err(error) => Err(Box::new(RejectedOneXsSolverReservation {
                reservation: self,
                error,
            })),
        }
    }
}

impl Drop for OneXsReservation {
    fn drop(&mut self) {
        if self.owner.is_none() {
            return;
        }
        let capture = self.capture.clone();
        if capture
            .restore(&self.flight, self.previous_ready.as_ref(), &mut self.owner)
            .is_err()
        {
            capture.retain_failed_reservation(self);
        }
    }
}

impl CompletedOneXsReservation {
    fn install(mut self) -> Result<Arc<OneXsMapFrame>, Box<RejectedOneXsInstall>> {
        let capture = self.capture.clone();
        match capture.install(&mut self) {
            Ok(map) => Ok(map),
            Err(reason) => Err(Box::new(RejectedOneXsInstall {
                completion: self,
                reason,
            })),
        }
    }
}

impl Drop for CompletedOneXsReservation {
    fn drop(&mut self) {
        if self.owner.is_none() {
            return;
        }
        let capture = self.capture.clone();
        if capture.install(self).is_err() {
            // A successful scalar transition cannot reconstruct its old
            // estimator without cloning the multi-megabyte state. If corrupt
            // completion metadata prevents publication, retain the advanced
            // owner in the terminal capture instead of silently dropping the
            // only usable estimator. The ready map remains the old display.
            capture.retain_failed_completion(self);
        }
    }
}

impl OneXsCapture {
    fn replay_start(&self, offered: Option<&FrameStamp>, target: u64) -> Fallible<ReplayStart> {
        let state = self.state.lock().map_err(
            |_| "ONE X2 stitch state is unavailable after its owner stopped unexpectedly",
        )?;
        let ready = state.ready.as_deref().map(OneXsMapFrame::frame);
        Ok(one_xs_replay_start(
            ready.map(FrameStamp::index),
            offered.map(FrameStamp::index),
            ready
                .zip(offered)
                .is_some_and(|(ready, offered)| ready == offered),
            ready
                .zip(offered)
                .is_some_and(|(ready, offered)| ready.same_decode_epoch(offered)),
            target,
        ))
    }

    fn new(calibration: &CalibrationSet) -> Fallible<Self> {
        Ok(Self {
            state: Mutex::new(OneXsCaptureState {
                owner: Some(Box::new(FrameOwner::new(calibration)?)),
                ready: None,
                generation: 0,
                in_flight: None,
                terminal: false,
                quarantined_owners: Vec::new(),
            }),
        })
    }

    fn acknowledged(&self, frame: &FrameStamp) -> Fallible<bool> {
        let state = self.state.lock().map_err(
            |_| "ONE X2 stitch state is unavailable after its owner stopped unexpectedly",
        )?;
        Ok(state
            .ready
            .as_ref()
            .is_some_and(|ready| ready.frame() == frame))
    }

    fn ready(&self, frame: &FrameStamp) -> Fallible<Option<Arc<OneXsMapFrame>>> {
        let state = self.state.lock().map_err(
            |_| "ONE X2 stitch state is unavailable after its owner stopped unexpectedly",
        )?;
        Ok(state
            .ready
            .as_ref()
            .filter(|ready| ready.frame() == frame)
            .cloned())
    }

    /// Return an existing exact result or reserve its successor atomically.
    ///
    /// The owner and prepared geometry move into the reservation. A recreated
    /// pipeline therefore observes `InFlight` and cannot submit the same or an
    /// ABA delivery while the original pipeline waits or computes.
    fn reserve(self: &Arc<Self>, frame: &FrameStamp, size: Size) -> Fallible<OneXsPreparation> {
        let mut state = self.state.lock().map_err(
            |_| "ONE X2 stitch state is unavailable after its owner stopped unexpectedly",
        )?;
        if let Some(ready) = state.ready.as_ref().filter(|ready| ready.frame() == frame) {
            return Ok(OneXsPreparation::Ready(ready.clone()));
        }
        if state.terminal {
            return Err(
                "ONE X2 stitch capture stopped after a transaction could not be published".into(),
            );
        }
        if state.in_flight.is_some() {
            return Ok(OneXsPreparation::InFlight(state.ready.clone()));
        }
        let prepared = state
            .owner
            .as_ref()
            .ok_or("ONE X2 stitch owner is absent without an active reservation")?
            .prepare(frame, size)?;
        let generation = state
            .generation
            .checked_add(1)
            .ok_or("ONE X2 stitch reservation generation is exhausted")?;
        let flight = GpuPisFlight {
            generation,
            frame: frame.clone(),
        };
        state.generation = generation;
        state.in_flight = Some(flight.clone());
        let previous_ready = state.ready.clone();
        let owner = state
            .owner
            .take()
            .expect("the checked idle capture owns its estimator");
        Ok(OneXsPreparation::Reserved(OneXsReservation {
            capture: self.clone(),
            flight,
            previous_ready,
            owner: Some(owner),
            prepared: Some(Box::new(prepared)),
        }))
    }

    fn restore(
        &self,
        flight: &GpuPisFlight,
        previous_ready: Option<&Arc<OneXsMapFrame>>,
        owner: &mut Option<Box<FrameOwner>>,
    ) -> Fallible<()> {
        let mut state = self.state.lock().map_err(
            |_| "ONE X2 stitch state is unavailable after its owner stopped unexpectedly",
        )?;
        if state.in_flight.as_ref() != Some(flight) {
            return Err("ONE X2 stitch rollback names a stale reservation generation".into());
        }
        if state.owner.is_some() {
            return Err("ONE X2 stitch rollback found another installed owner".into());
        }
        if !same_ready_map(state.ready.as_ref(), previous_ready) {
            return Err("ONE X2 stitch rollback found a changed ready map".into());
        }
        state.owner = Some(
            owner
                .take()
                .expect("a restored ONE X2 reservation owns its estimator"),
        );
        state.in_flight = None;
        Ok(())
    }

    /// Atomically publish the already-complete scalar owner and map.
    ///
    /// Every fallible estimator operation precedes this lock. Generation,
    /// full delivery identity and the previous ready allocation are checked
    /// before either shared slot changes.
    fn install(
        &self,
        completion: &mut CompletedOneXsReservation,
    ) -> Result<Arc<OneXsMapFrame>, String> {
        let mut state = self.state.lock().map_err(
            |_| "ONE X2 stitch state is unavailable after its owner stopped unexpectedly",
        )?;
        if state.in_flight.as_ref() != Some(&completion.flight) {
            return Err("ONE X2 stitch completion names a stale reservation generation".into());
        }
        if state.owner.is_some() {
            return Err("ONE X2 stitch completion found another installed owner".into());
        }
        if !same_ready_map(state.ready.as_ref(), completion.previous_ready.as_ref()) {
            return Err("ONE X2 stitch completion found a changed ready map".to_owned());
        }
        let FrameResult {
            map,
            phase,
            camera_mask,
            invalid_nodes,
            weighted_rows,
            lens_a_census,
            lens_b_census,
        } = completion
            .result
            .as_ref()
            .expect("a pending ONE X2 completion owns its frame result");
        if map.frame() != &completion.flight.frame {
            return Err(crate::studio_type2::FrameMapMismatch::new(
                "reservation",
                &completion.flight.frame,
                "map",
                map.frame(),
            )
            .to_string());
        }
        if map.pis_backend() != completion.pis_backend {
            return Err(format!(
                "ONE X2 stitch completion map backend is {}, expected {}",
                map.pis_backend().as_str(),
                completion.pis_backend.as_str()
            ));
        }
        // Preserve access to the complete transaction diagnostics without
        // making a pooled number a picture verdict. They remain available for
        // the exact-frame regression and do not gate drawing.
        let _ = (
            phase,
            camera_mask,
            invalid_nodes,
            weighted_rows,
            lens_a_census,
            lens_b_census,
        );
        let FrameResult { map, .. } = completion
            .result
            .take()
            .expect("the validated ONE X2 completion retains its frame result");
        let map = Arc::new(map);
        state.owner = Some(
            completion
                .owner
                .take()
                .expect("the validated ONE X2 completion retains its estimator"),
        );
        state.ready = Some(map.clone());
        state.in_flight = None;
        Ok(map)
    }

    fn retain_failed_completion(&self, completion: &mut CompletedOneXsReservation) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.terminal = true;
        let exact_flight = state.in_flight.as_ref() == Some(&completion.flight);
        let exact_ready = same_ready_map(state.ready.as_ref(), completion.previous_ready.as_ref());
        if exact_flight && exact_ready && state.owner.is_none() {
            state.owner = completion.owner.take();
            state.in_flight = None;
        } else if let Some(owner) = completion.owner.take() {
            state.quarantined_owners.push(*owner);
        }
    }

    fn retain_failed_reservation(&self, reservation: &mut OneXsReservation) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.terminal = true;
        let exact_flight = state.in_flight.as_ref() == Some(&reservation.flight);
        let exact_ready = same_ready_map(state.ready.as_ref(), reservation.previous_ready.as_ref());
        if exact_flight && exact_ready && state.owner.is_none() {
            state.owner = reservation.owner.take();
            state.in_flight = None;
        } else if let Some(owner) = reservation.owner.take() {
            state.quarantined_owners.push(*owner);
        }
    }
}

impl Motion {
    /// The camera's own instant for this frame, in media time.
    fn instant(&self, frames: &Frames, clock: FrameClock) -> i64 {
        let container = || i64::try_from(frames.timestamp.as_micros()).unwrap_or(i64::MAX);
        match clock {
            // The camera's own timestamp, or the container's where the
            // exposure record does not reach: a file whose record is short is
            // a file that still plays.
            FrameClock::Exposure => self
                .exposure
                .frame_time_us(frames.index)
                .unwrap_or_else(container),
            FrameClock::Container => container(),
        }
    }

    /// How the body moved while this frame came off the sensor (issue #9).
    ///
    /// `None` where there is nothing to correct with, or nothing known to
    /// correct: a file with no IMU record, a trailer with no readout time,
    /// and any camera whose readout direction has not been measured
    /// (`kjerag_meta::Sweep::Unknown`, which today is everything except the
    /// measured X4 and ONE X2 families).
    /// The pass is then what it was before issue #9, and the picture with it.
    fn rolling(&self, at: i64, readout: Readout) -> Option<Rolling> {
        let span = (readout.seconds * 1e6) as i64;
        let axis = readout.sweep.axis();
        if self.orientation.is_empty() || span <= 0 || axis == [0.0; 2] {
            return None;
        }
        Some(Rolling {
            // Centred on the frame's own instant, so the middle row is the
            // instant the rest of the pipeline already believes in and the
            // two ends of the window are where the turn is exact.
            turn: self.orientation.turn(at - span / 2, at + span / 2),
            axis,
        })
    }
}

struct Playing {
    frames: Option<Arc<Frames>>,
    source: Source,
}

enum Source {
    /// Playing, or paused mid-play: a decode thread and a clock. Boxed
    /// because the other arm carries nothing.
    Live(Box<Player>),
    /// Frames pulled on this thread, no clock and no thread. What the
    /// headless instruments use; the reader is kept so an instrument that
    /// walks a run of frames pays the container open and the trailer parse
    /// once rather than once per frame.
    Stepped(Box<Reader>),
}

impl Scene {
    /// No file: nothing on the pane but the backdrop behind it.
    pub fn blank() -> Self {
        Self {
            show: None,
            shutter: Shutter::default(),
            cursor_hidden: false,
            viewpoint: Cell::new(Viewpoint::default()),
            nudge: Cell::new(None),
            horizon: Cell::new(Horizon::default()),
            clock: Cell::new(FrameClock::default()),
            forced: Cell::new(None),
            readout: Cell::new(None),
            sampling: Cell::new(Sampling::default()),
            flow: Cell::new(false),
            stalled: Stalled::default(),
            shown: Shown::default(),
        }
    }

    /// Opens a file and starts playing it. Returns as soon as the container
    /// is parsed; the first frames arrive on the decode thread.
    pub fn open(path: &Path) -> Fallible<Self> {
        Self::open_with(path, &[])
    }

    /// The same, told about the other files the pilot picked alongside this
    /// one. A capture written one lens per file finds its other half beside
    /// itself, except when a sandbox's file chooser hands over a document
    /// with nothing beside it, and then the pilot's own second pick is the
    /// only place it can come from (issue #123).
    pub fn open_with(path: &Path, alongside: &[PathBuf]) -> Fallible<Self> {
        ours(path)?;
        Self::open_live(Player::open_with(path, alongside)?)
    }

    /// Opens an already authenticated two-file capture for live playback in
    /// explicit lens order. The caller may pass retained descriptor aliases;
    /// neither media layer rediscovers or reopens a mutable sibling name.
    pub fn open_pair(first: &Path, second: &Path) -> Fallible<Self> {
        ours(first)?;
        Self::open_live(Player::open_pair(first, second)?)
    }

    fn open_live(mut player: Player) -> Fallible<Self> {
        let files: Arc<[PathBuf]> = player.paths().into();
        // The trailer is the capture's rather than the picked file's, and on a
        // camera that writes one lens per file only lens 0 carries one
        // (`kjerag_meta::pair`). The pilot picks whichever half his file
        // manager listed first, and a `_10_` document has no trailer and
        // nothing beside it to borrow one from, so reading it from the file
        // the reader put first is the difference between a capture that opens
        // either way round and one that opens only if it was picked in the
        // camera's own order (issue #123).
        let calibrated = calibrated(&files[0], player.size(), player.lenses())?;
        let selected_one_xs = projection::is_one_xs_lens_pair(&calibrated.lenses);
        println!(
            "media:  {}{}, {}x{}, {:.3} fps, {} frames, {:.1} s",
            match player.lenses() {
                1 => "1 lens stream".to_owned(),
                n => format!("{n} lens streams"),
            },
            // Two files is a capture the camera wrote one lens per file and
            // the player paired at open (issue #79). Printed because it is
            // the one thing about an open file the pilot cannot otherwise
            // see: half a sphere and a whole one look the same until the
            // view is turned round.
            match player.files() {
                1 => String::new(),
                n => format!(" from {n} files"),
            },
            player.size().width,
            player.size().height,
            player.timing().fps(),
            player.timing().frames,
            player.timing().duration().as_secs_f64(),
        );
        // Opening a file plays it, which is what every player does. Space
        // and the control row's button pause it (issue #16).
        let frame = player.size();
        if one_xs_playback_selected(ONE_XS_PLAYBACK_ENABLED, selected_one_xs) {
            // The selected estimator is causal: every aligned decoded pair is
            // part of the next pair's state. Presentation therefore cannot
            // discard a late frame before the stitch owner consumes it.
            if !player.set_presentation_policy(PresentationPolicy::EveryFrame) {
                return Err(
                    "ONE X2 playback could not preserve every source frame before starting".into(),
                );
            }
        }
        player.play();
        Ok(Self {
            show: Some(Show::new(
                files,
                frame,
                calibrated,
                None,
                Source::Live(Box::new(player)),
            )),
            ..Self::blank()
        })
    }

    /// One frame of a file, decoded on this thread. The headless
    /// instruments render with this, and it takes a [`Cue`] rather than
    /// always giving frame 0 because #8's Studio-diff harness needs to name
    /// the frame it is checking.
    pub fn still(path: &Path, at: Cue) -> Fallible<Self> {
        ours(path)?;
        let reader = Reader::open(path)?;
        Self::still_from_reader(reader, at)
    }

    /// One frame from an explicitly selected pair, in lens order.
    ///
    /// Offline evidence consumers use this with retained-descriptor paths so
    /// container decode, trailer parsing and calibration all stay bound to
    /// the authenticated leaves instead of reopening mutable names.
    pub fn still_pair(first: &Path, second: &Path, at: Cue) -> Fallible<Self> {
        ours(first)?;
        let reader = Reader::open_pair(first, second)?;
        Self::still_from_reader(reader, at)
    }

    fn still_from_reader(mut reader: Reader, at: Cue) -> Fallible<Self> {
        let files: Arc<[PathBuf]> = reader.paths().into();
        let calibrated = calibrated(&files[0], reader.size(), reader.lenses())?;
        let frame = reader.size();
        let frames = reader.frame(at)?;
        println!(
            "frame:  {} at {:.3} s",
            frames.index,
            frames.timestamp.as_secs_f64()
        );
        for frame in &frames.lenses {
            println!("drm:    {}", frame.describe());
        }
        Ok(Self {
            show: Some(Show::new(
                files,
                frame,
                calibrated,
                Some(Arc::new(frames)),
                Source::Stepped(Box::new(reader)),
            )),
            ..Self::blank()
        })
    }

    /// What names the camera this file came off, which is what its seam
    /// calibration is stored under (issue #48). `None` with nothing open.
    pub fn camera_key(&self) -> Option<u64> {
        Some(self.show.as_ref()?.camera)
    }

    /// Whether this file has a seam at all: two lens streams, sampled from
    /// one body. A one-stream capture has nothing to hand over and nothing to
    /// calibrate.
    pub fn has_seam(&self) -> bool {
        self.show
            .as_ref()
            .is_some_and(|show| show.lenses.len() >= 2)
    }

    /// How wide this file hands the picture over, in degrees, or `None` for a
    /// capture with one lens stream, which has no seam to hand over at.
    ///
    /// **The width is the camera's and not the build's** since 2026-08-05: the
    /// projection asks for one number and this file's own overlap clamps it
    /// ([`Reframe::handover_width`], [`Reframe::afforded`]). Since the flat
    /// seam every camera in the corpus takes the 8 asked for, the ONE X2
    /// included, because the bound is the bare overlap and the X2 overlaps by
    /// 9.19; it drew 4.18 while the bend it carried had to fit in the same
    /// margin.
    ///
    /// Read off the lenses the pass will draw with **now**, correction and all,
    /// because a seam fit moves the principal point, which moves each lens's
    /// coverage boundary, which moves the overlap. So this is a reading and not
    /// a property of the file, and a fit landing later can move it - which is
    /// why [`fit_into`] says it again when one does.
    pub fn handover_deg(&self) -> Option<f32> {
        let show = self.show.as_ref()?;
        handover_deg(&show.lenses(), show.frame)
    }

    /// Draw this file with what the pool knows about its camera. Applied here
    /// and now, with no walk, so it is in the first frame.
    pub fn use_seam(&self, fit: SeamFit) {
        let Some(show) = &self.show else {
            return;
        };
        show.corrected.land(fit);
    }

    /// Draw this file with the along-seam table its camera has been read at
    /// (issue #103, stage 9).
    ///
    /// Landed rather than walked. A table is a calibration and the caller is
    /// expected to set it before the first frame, where there is no picture
    /// for it to jump.
    ///
    /// **That is a discipline and not a property of this function.** Called
    /// mid-play it lands whatever it is given in the next frame, and the
    /// picture steps by the whole of it. A later stage that re-answered a table
    /// while a file is up would need to ease it in rather than land it here.
    pub fn use_table(&self, table: Table) {
        if let Some(show) = &self.show {
            show.table.set(table);
        }
    }

    /// Take the next frame of a stepped scene, on this thread. `false` at the
    /// end of the file.
    ///
    /// The instrument that measures the horizon over a run of frames
    /// (issue #8) reads consecutive frames, and a seek per frame would cost
    /// it a keyframe walk each time.
    pub fn advance(&mut self) -> Fallible<bool> {
        let Some(show) = self.show.as_mut() else {
            return Ok(false);
        };
        let Playing { frames, source } = show.playing.get_mut();
        let Source::Stepped(reader) = source else {
            return Ok(false);
        };
        match reader.next_frames()? {
            Some(taken) => {
                *frames = Some(Arc::new(taken));
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// The frame currently offered by the source, for instruments that drive
    /// or measure that delivery boundary.
    ///
    /// This can be newer than the picture the pass has committed to display.
    /// Use [`Self::displayed_frame`] when reporting what the pilot can see.
    pub fn frame(&self) -> Option<(u64, Duration)> {
        let show = self.show.as_ref()?;
        let playing = show.playing.borrow();
        let frames = playing.frames.as_ref()?;
        Some((frames.index, frames.timestamp))
    }

    /// The frame the pass most recently committed to display.
    ///
    /// Unlike [`Self::frame`], this reads the retained full [`View`] written
    /// at the presentation boundary. It therefore stays on the visible frame
    /// while a newer offered delivery is still waiting for import or its
    /// selected stitch map.
    pub fn displayed_frame(&self) -> Option<(u64, Duration)> {
        self.shown.frame()
    }

    /// Opaque identity of the exact aligned lens pair currently held by this
    /// scene. Unlike [`Self::frame`], this does not alias after a seek or
    /// across captures.
    pub fn frame_stamp(&self) -> Option<FrameStamp> {
        let show = self.show.as_ref()?;
        let playing = show.playing.borrow();
        Some(playing.frames.as_ref()?.stamp())
    }

    /// Containers in the decoder's admitted lane order, for evidence
    /// instruments which must bind the exact paired source rather than infer a
    /// sibling from a filename.
    pub fn source_paths(&self) -> Option<Arc<[PathBuf]>> {
        Some(self.show.as_ref()?.files.clone())
    }

    /// Return the selected ONE X2 map for an instrument inspecting the exact
    /// delivery currently held by this scene.
    ///
    /// This is deliberately narrower than exposing the capture owner or its
    /// retained state. A map is returned only when selected ONE X2 playback
    /// owns this capture and its ready transaction carries the complete opaque
    /// [`FrameStamp`] of the scene's current aligned pair. An index and time
    /// match after a seek or reopen is therefore not enough.
    pub fn diagnostic_one_xs_map(&self) -> Fallible<Option<OneXsMapFrame>> {
        let Some(show) = self.show.as_ref() else {
            return Ok(None);
        };
        let playing = show.playing.borrow();
        let Some(capture) = show.one_xs.as_ref() else {
            return Ok(None);
        };
        let Some(frames) = playing.frames.as_ref() else {
            return Ok(None);
        };
        let current = frames.stamp();
        let Some(shown) = self.shown.get() else {
            return Ok(None);
        };
        let shown_stamp = shown.frames.stamp();
        let same_capture = shown
            .one_xs
            .as_ref()
            .is_some_and(|shown_capture| Arc::ptr_eq(shown_capture, capture));
        if !exact_selected_display(&current, Some(&shown_stamp), same_capture) {
            return Ok(None);
        }
        Ok(capture.ready(&current)?.map(|map| map.as_ref().clone()))
    }

    /// Takes whichever frame belongs on screen at `now`, and says when to
    /// come back. Call it on every redraw: this is the presentation clock's
    /// only tick.
    pub fn pump(&self, now: Instant) -> Next {
        // Nothing open is nothing that changes by itself: no clock, no decode
        // thread, and a pane the shell paints. This asked for a redraw on
        // every compositor refresh while the pass carried an animation, which
        // was a window kept awake to draw a test pattern.
        let Some(show) = &self.show else {
            return Next::Never;
        };
        // Out of the cell in one step: the borrow checker splits the fields
        // of a `&mut Playing`, but not those of a `RefMut`.
        let Playing { frames, source } = &mut *show.playing.borrow_mut();
        // The pass has been unable to put a frame on screen for long enough
        // that it has given up (issue #124). Pausing is what stops the sound
        // as well as the clock, because the sound follows the clock
        // (`kjerag_media`'s `Beat`), and a picture that died while the audio
        // played on is the whole of what that issue was.
        if let Some(stall) = self.stalled.take() {
            if let Source::Live(player) = source {
                player.pause(now);
            }
            retire_replay(&show.replay);
            return Next::Stopped(self.finish_observed_terminal_stop(stall));
        }
        let Source::Live(player) = source else {
            return Next::Never;
        };
        // EveryFrame protects the decode queue from dropping a pair, but the
        // scene still has to acknowledge when the currently offered pair has
        // crossed the separate iced pipeline lifetime and completed its
        // capture-owned stitch transaction. Gate on `playing.frames` itself,
        // not `Shown`: selected output is deliberately not marked shown until
        // the matching map is ready, so `Shown` cannot acknowledge the input
        // whose replacement this controls. The initial `None` remains open so
        // frame zero can be offered.
        let capture_owned = show.one_xs.is_some();
        let current_ready = match (show.one_xs.as_ref(), frames.as_ref()) {
            (Some(capture), Some(frame)) => match capture.acknowledged(&frame.stamp()) {
                Ok(ready) => Some(ready),
                Err(error) => {
                    // Capture-state loss is deterministic. Stop immediately
                    // with its raw error and never ask the player for a frame
                    // that the sequential owner can no longer consume.
                    retire_replay(&show.replay);
                    self.stalled.fail_now(&error);
                    player.pause(now);
                    self.fail_terminal_shutter_without_display();
                    return self.stalled.take().map_or(Next::Never, Next::Stopped);
                }
            },
            _ => None,
        };
        if current_ready == Some(true)
            && frames.as_ref().is_some_and(|frame| {
                show.replay
                    .borrow()
                    .is_some_and(|replay| replay.target == frame.index)
            })
        {
            // Decoder landing is not completion. Retire the exposed seek only
            // after the exact target source has its exact capture-owned map.
            if show
                .replay
                .borrow_mut()
                .take()
                .is_some_and(|replay| replay.playing)
            {
                player.play();
            }
        }
        if one_xs_frame_waiting(ONE_XS_PLAYBACK_ENABLED, capture_owned, current_ready) {
            return Next::Refresh;
        }
        match player.pump(now) {
            Ok(None) => {}
            Ok(Some(taken)) => *frames = Some(taken),
            // The decode thread has stopped and will deliver nothing more, so
            // the answer is the same one: stop cleanly, and say so. This
            // printed a line and paused in silence until issue #124.
            Err(e) => {
                player.pause(now);
                retire_replay(&show.replay);
                self.stalled.fail_now(&e);
                // This redraw may not reach preparation after publishing the
                // stop. With no completed display there is nothing for that
                // preparation to capture anyway, so resolve the independent
                // one-shot here as part of the same terminal transition.
                self.fail_terminal_shutter_without_display();
                return self.stalled.take().map_or(Next::Never, Next::Stopped);
            }
        }
        // The end of the file stops the clock rather than leaving it running
        // against frames that will never arrive.
        if player.is_ended() {
            player.pause(now);
            return Next::Never;
        }
        next_after_pump(
            show.replay.borrow().is_some(),
            player.is_playing(),
            player.is_seeking(),
            player.next_due(),
        )
    }

    pub fn toggle_play(&mut self, now: Instant) {
        if self.is_playing() {
            self.pause(now);
        } else {
            self.play();
        }
    }

    pub fn play(&mut self) {
        if let Some(show) = &self.show
            && let Some(replay) = show.replay.borrow_mut().as_mut()
        {
            replay.playing = true;
            return;
        }
        if let Some(player) = self.player_mut() {
            player.play();
        }
    }

    pub fn pause(&mut self, now: Instant) {
        if let Some(show) = &self.show
            && let Some(replay) = show.replay.borrow_mut().as_mut()
        {
            replay.playing = false;
            return;
        }
        if let Some(player) = self.player_mut() {
            player.pause(now);
        }
    }

    /// Move the picture, to a keyframe while a drag is still going and to the
    /// frame itself when it ends (issue #5).
    pub fn seek(&mut self, to: Duration, accuracy: Accuracy) {
        if self.stalled.stopped() {
            return;
        }
        let selected = self
            .show
            .as_mut()
            .map(|show| show.replay_to(Cue::Time(to), false))
            .transpose();
        match selected {
            Ok(Some(true)) => return,
            Ok(_) => {}
            Err(error) => {
                self.stalled.fail_now(error);
                return;
            }
        }
        if let Some(player) = self.player_mut() {
            player.seek(Cue::Time(to), accuracy);
        }
    }

    /// One frame forward or back.
    pub fn step(&mut self, now: Instant, frames: i64) {
        if self.stalled.stopped() {
            return;
        }
        let selected = self
            .show
            .as_mut()
            .map(|show| show.replay_step(now, frames))
            .transpose();
        match selected {
            Ok(Some(true)) => return,
            Ok(_) => {}
            Err(error) => {
                self.stalled.fail_now(error);
                return;
            }
        }
        if let Some(player) = self.player_mut() {
            player.step(now, frames);
        }
    }

    /// A seek has been asked for and has not landed. The shell keeps the
    /// picture redrawing while this is true.
    pub fn is_seeking(&self) -> bool {
        if self
            .show
            .as_ref()
            .is_some_and(|show| show.replay.borrow().is_some())
        {
            return true;
        }
        self.player(Player::is_seeking).unwrap_or(false)
    }

    pub fn is_playing(&self) -> bool {
        if let Some(replay) = self.show.as_ref().and_then(|show| *show.replay.borrow()) {
            return replay.playing;
        }
        self.player(Player::is_playing).unwrap_or(false)
    }

    pub fn position(&self, now: Instant) -> Duration {
        if let Some(replay) = self.show.as_ref().and_then(|show| *show.replay.borrow()) {
            return replay.position;
        }
        self.player(|player| player.position(now))
            .or_else(|| {
                self.show
                    .as_ref()?
                    .playing
                    .borrow()
                    .frames
                    .as_ref()
                    .map(|frames| frames.timestamp)
            })
            .unwrap_or_default()
    }

    /// How long the file runs, from the container: the frame count and the
    /// rational frame rate, divided.
    pub fn duration(&self) -> Duration {
        let Some(show) = self.show.as_ref() else {
            return Duration::ZERO;
        };
        let playing = show.playing.borrow();
        match &playing.source {
            Source::Live(player) => player.timing().duration(),
            Source::Stepped(reader) => reader.timing().duration(),
        }
    }

    /// How many lenses the open capture is read as: two for a whole sphere,
    /// whether they came out of one file or two, one for half of one.
    ///
    /// The shell asks because half a sphere is the one thing about an open
    /// file the pilot cannot see. It looks like a whole one until the view is
    /// turned round (issue #123).
    pub fn lenses(&self) -> usize {
        self.show.as_ref().map_or(0, |show| show.lenses.len())
    }

    /// Whether this file has a sound track that a device took (issue #13).
    /// `false` is a file with no sound in it, or a box with no working output;
    /// the control row draws its volume button disabled for both.
    pub fn has_sound(&self) -> bool {
        self.player(Player::has_sound).unwrap_or(false)
    }

    /// Loudness, 0 to 1. A file with no sound takes it and does nothing.
    pub fn set_volume(&self, volume: f32) {
        self.player(|player| player.set_volume(volume));
    }

    /// Silence without stopping: the sound keeps running under a mute, so
    /// unmuting lands where the picture is rather than where it was.
    pub fn set_muted(&self, muted: bool) {
        self.player(|player| player.set_muted(muted));
    }

    /// Hide the pointer along with the controls, or bring both back.
    pub fn hide_cursor(&mut self, hidden: bool) {
        self.cursor_hidden = hidden;
    }

    pub fn is_cursor_hidden(&self) -> bool {
        self.cursor_hidden
    }

    /// Where the view points, and whether a drag has hold of it.
    pub fn viewpoint(&self) -> Viewpoint {
        self.viewpoint.get()
    }

    /// Move the view, and hand back whatever the move answered. The widget's
    /// mouse handling is the only caller: everything else asks through a
    /// [`Nudge`].
    pub(crate) fn steer<T>(&self, steer: impl FnOnce(&mut Viewpoint) -> T) -> T {
        let mut viewpoint = self.viewpoint.get();
        let answer = steer(&mut viewpoint);
        self.viewpoint.set(viewpoint);
        answer
    }

    /// Leave a view change for the widget to apply on its next redraw.
    pub fn nudge(&self, nudge: Nudge) {
        self.nudge.set(Some(nudge));
    }

    pub(crate) fn take_nudge(&self) -> Option<Nudge> {
        self.nudge.take()
    }

    /// Hold the picture against the world, or let it ride the camera
    /// (issue #8). Takes effect on the next redraw.
    pub fn set_horizon(&self, horizon: Horizon) {
        self.horizon.set(horizon);
    }

    pub fn horizon(&self) -> Horizon {
        self.horizon.get()
    }

    /// Whether this file carries the IMU record horizon lock needs. A file
    /// without one plays with the toggle on and the picture unheld.
    pub fn has_orientation(&self) -> bool {
        self.show
            .as_ref()
            .is_some_and(|show| !show.held.orientation.is_empty())
    }

    /// Whether the player's optical-flow toggle may use the available legacy
    /// route for this file. With nothing open it remains a persisted
    /// preference; an open ONE X2 refuses the legacy solver because its
    /// selected route runs automatically.
    pub fn supports_optical_flow(&self) -> bool {
        supports_player_flow(self.show.as_ref().map(|show| show.lenses.as_ref()))
    }

    /// Which clock a frame's orientation is looked up on. The instrument that
    /// measured the choice moves this; the shell leaves it alone
    /// ([`FrameClock`]).
    pub fn set_frame_clock(&self, clock: FrameClock) {
        self.clock.set(clock);
    }

    /// Hold the picture at this orientation rather than the one the file's
    /// own IMU solves to, until it is set back to `None`.
    ///
    /// The harness's hook, and the reason it is here rather than in the
    /// harness: a deliberately wrong answer has to travel the **same** path
    /// to the shader as the right one, or what fails is the harness's own
    /// copy of the composition and not the thing under test (issue #8's
    /// negative control). Nothing in the shell calls this.
    pub fn hold_at(&self, world_from_body: Option<Quat>) {
        self.forced.set(world_from_body);
    }

    /// Read the sensor this way rather than the way the file describes, until
    /// it is set back to `None`. A [`Readout`] with a zero span is the
    /// rolling-shutter correction switched off.
    ///
    /// The same hook as [`Self::hold_at`] and for the same reason: issue #9's
    /// answer is which way the sensor reads, and the three wrong answers have
    /// to reach the shader by the path the right one takes, or what is
    /// measured is the harness. Nothing in the shell calls this.
    pub fn set_readout(&self, readout: Option<Readout>) {
        self.readout.set(readout);
    }

    /// How one frame comes off this file's sensor, for an instrument that has
    /// to say what it corrected for. `None` before a file is open.
    pub fn readout(&self) -> Option<Readout> {
        self.show.as_ref().map(|show| show.held.readout)
    }

    /// Sample the magnified picture this way rather than the way the player
    /// ships (issue #11). The same hook as [`Self::hold_at`], and the same
    /// reason: what a quality change is worth is the difference between two
    /// pictures, and the losing one has to come out of the same pass.
    /// Nothing in the shell calls this.
    pub fn set_sampling(&self, sampling: Sampling) {
        self.sampling.set(sampling);
    }

    /// Request the available Studio-derived optical-flow seam correction
    /// (default off). Takes effect on the next redraw. The selected ONE X2
    /// camera refuses this legacy route in [`ScenePipeline::prepare`]; supported
    /// cameras estimate on a background worker at the recovered cadence and
    /// hold the last completed field. Off draws the ordinary pass and runs none
    /// of that machinery.
    pub fn set_flow(&self, on: bool) {
        self.flow.set(on);
    }

    pub fn flow(&self) -> bool {
        self.flow.get()
    }

    pub fn stats(&self) -> Option<Stats> {
        self.player(Player::stats)
    }

    /// Reading the player needs the cell, so this hands it to a closure
    /// rather than handing out a reference into it.
    fn player<T>(&self, read: impl FnOnce(&Player) -> T) -> Option<T> {
        let show = self.show.as_ref()?;
        let playing = show.playing.borrow();
        match &playing.source {
            Source::Live(player) => Some(read(player)),
            Source::Stepped(_) => None,
        }
    }

    /// The player, for the calls that drive it: play, pause, seek, step.
    ///
    /// A capture the pass has given up on has none to hand out (issue #124).
    /// The sound follows the clock, so a play press that got through here
    /// would be sound over a picture that is not coming back, which is the
    /// symptom this whole issue is about. The transport goes quiet with the
    /// file it belongs to, and opening a file is the way on.
    fn player_mut(&mut self) -> Option<&mut Player> {
        if self.stalled.stopped() {
            return None;
        }
        match &mut self.show.as_mut()?.playing.get_mut().source {
            Source::Live(player) => Some(player),
            Source::Stepped(_) => None,
        }
    }

    /// Resolves an armed still as part of a terminal transition when this
    /// capture has no complete display to offer it. Called from every stop
    /// discovered in [`Self::pump`], because publishing that stop may end the
    /// redraw before the renderer's preparation half gets another turn.
    fn fail_terminal_shutter_without_display(&self) {
        if self.shown.get().is_none()
            && let Some(error) = self.stalled.terminal()
        {
            self.shutter.fail(error);
        }
    }

    /// Completes the one-shot work belonging to a terminal stop the shell is
    /// about to observe. The caller pauses playback and retires any replay
    /// first; this final step preserves the raw retained error for a still
    /// whose redraw will never arrive.
    fn finish_observed_terminal_stop(&self, stall: Stall) -> Stall {
        self.fail_terminal_shutter_without_display();
        stall
    }

    /// Asks for a still of whatever the next redraw draws, at the size the
    /// request names. The pixels come back on a worker thread, through the
    /// request's own `then`; nothing here waits.
    pub fn capture(&self, request: Request) {
        // Arm before checking terminal state. Together with every terminal
        // transition checking after it stores the reason, this closes both
        // sides of the cross-thread race: whichever operation happens last
        // observes and resolves the shutter. A complete shown display stays
        // armed for the pipeline to restore and capture after failure.
        self.shutter.arm(request);
        self.fail_terminal_shutter_without_display();
    }

    /// The map this scene would draw one view through, for an instrument that
    /// wants to ask where a pixel is looking without opening a window.
    ///
    /// The same `Reframe` `prepare` builds, minus the frame it would be bound
    /// to: what it answers about is geometry, which the pictures do not
    /// change.
    pub fn mapped(&self, camera: Camera, aspect: f32) -> Option<Reframe> {
        let view = self.primitive(camera).view?;
        Some(
            Reframe::new(
                &view.lenses,
                view.frames.size,
                camera,
                view.held,
                aspect,
                false,
                self.sampling.get(),
            )
            .with_table(view.table),
        )
    }

    pub fn primitive(&self, camera: Camera) -> ScenePrimitive {
        let held = Holding {
            horizon: self.horizon.get(),
            clock: self.clock.get(),
            forced: self.forced.get(),
            readout: self.readout.get(),
        };
        ScenePrimitive {
            camera,
            view: self.show.as_ref().and_then(|show| show.view(held)),
            sampling: self.sampling.get(),
            flow: self.flow.get(),
            shutter: self.shutter.clone(),
            stalled: self.stalled.clone(),
            shown: self.shown.clone(),
        }
    }
}

impl Show {
    fn new(
        files: Arc<[PathBuf]>,
        frame: Size,
        calibrated: Calibrated,
        frames: Option<Arc<Frames>>,
        source: Source,
    ) -> Self {
        // Stepped scenes are forensic inputs which can start at any requested
        // frame and use explicit type-2 APIs. Only an ordinary live decode is
        // the cold-from-frame-zero production transaction.
        let one_xs = matches!(&source, Source::Live(_))
            .then(|| calibrated.one_xs.clone())
            .flatten();
        let one_xs_calibration = matches!(&source, Source::Live(_))
            .then(|| calibrated.one_xs_calibration.clone())
            .flatten();
        Self {
            files,
            frame,
            corrected: Arc::new(Correction::none(&calibrated.lenses)),
            table: Cell::new(Table::REST),
            lenses: calibrated.lenses,
            camera: calibrated.camera,
            held: calibrated.held,
            one_xs,
            one_xs_calibration,
            replay: RefCell::new(None),
            playing: RefCell::new(Playing { frames, source }),
        }
    }

    /// What the pass runs on this redraw: the corrected lenses as they stand,
    /// which since the seam fit landed at open never change under a viewer.
    fn lenses(&self) -> Arc<[Lens]> {
        self.corrected.lenses()
    }

    fn view(&self, held: Holding) -> Option<View> {
        let frames = self.playing.borrow().frames.clone()?;
        let at = self.held.instant(&frames, held.clock);
        let world_from_body = held.forced.unwrap_or_else(|| self.held.orientation.at(at));
        Some(View {
            held: Held {
                body_from_world: match held.horizon {
                    Horizon::Locked => world_from_body.conjugate(),
                    Horizon::Free => Quat::IDENTITY,
                },
                // Not under the horizon toggle: the readout is the camera's
                // own motion during the frame, and a view that rides the body
                // has the same skew in it as one that does not.
                rolling: self
                    .held
                    .rolling(at, held.readout.unwrap_or(self.held.readout)),
            },
            lenses: self.lenses(),
            table: self.table.get(),
            frames,
            one_xs: self.one_xs.clone(),
        })
    }

    /// Route an exposed selected-capture jump through the causal replay
    /// state machine. All callers of `Scene::seek` arrive here, including
    /// scrub updates, releases, relative jumps and pasted views.
    fn replay_to(&mut self, to: Cue, allow_forward: bool) -> Fallible<bool> {
        let Some(capture) = self.one_xs.clone() else {
            return Ok(false);
        };
        let Playing { frames, source } = self.playing.get_mut();
        let Source::Live(player) = source else {
            return Ok(false);
        };
        let target = to
            .index(player.timing())
            .min(player.timing().frames.saturating_sub(1));
        let replaying = self.replay.borrow().is_some();
        if self
            .replay
            .borrow()
            .is_some_and(|replay| replay.target == target)
        {
            return Ok(true);
        }
        let playing = self
            .replay
            .borrow()
            .map_or_else(|| player.is_playing(), |replay| replay.playing);
        let offered = frames.as_ref().map(|frames| frames.stamp());
        let proposed = capture.replay_start(offered.as_ref(), target)?;
        // Arbitrary seeks restart so the independent sound ring can be
        // positioned at the requested target without consuming stale audio.
        // A single forward step retains the already-proven next-audio splice.
        let start = selected_replay_start(
            proposed,
            allow_forward,
            replaying,
            offered.as_ref().map(FrameStamp::index),
            target,
        );
        if start == ReplayStart::FrameZero {
            let calibration = self
                .one_xs_calibration
                .as_ref()
                .ok_or("ONE X2 playback lost its capture calibration")?;
            // Replace the Arc. The retained old View continues to name the
            // old completed capture and can never submit its nonzero frame to
            // this fresh frame-zero owner.
            self.one_xs = Some(Arc::new(OneXsCapture::new(calibration)?));
            // Do not let the acknowledgement gate wait for a surface from
            // the lineage just retired. The last complete display remains in
            // `Shown` and may be restored until new frame zero completes.
            *frames = None;
        }
        self.replay.replace(Some(OneXsReplay {
            target,
            position: player.timing().time_of(target),
            playing,
        }));
        if let Err(error) = player.replay_to(Cue::Index(target), start == ReplayStart::FrameZero) {
            retire_replay(&self.replay);
            return Err(error);
        }
        Ok(true)
    }

    fn replay_step(&mut self, now: Instant, by: i64) -> Fallible<bool> {
        if self.one_xs.is_none() {
            return Ok(false);
        }
        let Playing { source, .. } = self.playing.get_mut();
        let Source::Live(player) = source else {
            return Ok(false);
        };
        player.pause(now);
        let was_replaying = self.replay.borrow().is_some();
        if let Some(replay) = self.replay.borrow_mut().as_mut() {
            replay.playing = false;
        }
        let index = replay_step_base(
            self.replay.borrow().map(|replay| replay.target),
            player.index(),
        );
        let Some(index) = index else {
            return Ok(true);
        };
        let target = index
            .saturating_add_signed(by)
            .min(player.timing().frames.saturating_sub(1));
        self.replay_to(Cue::Index(target), by == 1 && !was_replaying)
    }
}

/// How wide a camera with these lenses hands the picture over, in degrees, or
/// `None` where there is no seam to hand over at.
///
/// Read by the shell at open ([`Scene::handover_deg`]). It reads the same
/// [`Reframe::handover_width`] the pass reads, off the lenses it is handed, and
/// the aspect and the camera it builds the map with do not reach the answer.
fn handover_deg(lenses: &[Lens], frame: Size) -> Option<f32> {
    if lenses.len() < 2 {
        return None;
    }
    let mapped = Reframe::new(
        lenses,
        frame,
        Camera::default(),
        Held::default(),
        1.0,
        false,
        Sampling::default(),
    );
    Some(mapped.handover_width().to_degrees())
}

/// Everything the trailer contributes to one open capture: the calibration
/// for the lenses the shader samples, checked against the streams they will
/// be sampled from, and where the camera body went while it recorded.
///
/// One lens entry per decoded stream, and in the same order: the trailer
/// writes its lens blocks in the order the container carries the streams,
/// and a paired per-lens capture opens lens 0's file first, so the two
/// orders are the same one (`kjerag_meta::lens_index`).
///
/// **The calibration belongs to the capture, not to the file** (issue #79).
/// A camera that writes one lens per file writes one trailer for the pair
/// and keeps it with lens 0, so opening the second file reads the first
/// file's trailer. Before that, opening it failed outright with "file has no
/// Insta360 trailer". A per-lens file whose sibling is not on the card still
/// calibrates two lenses and decodes one, and then this is lens 0 alone and
/// the picture is one hemisphere, exactly as it was.
///
/// The orientation is integrated here, once, at open: a 30-minute X4 Air
/// capture is 1.8 million IMU samples and costs about a fifth of a second to
/// read and integrate, against 70 ms to open the container. Doing it per
/// frame would be 30 times a second for a track that does not change.
/// Another camera's 360 format is refused here, before the decoder is asked
/// for anything (issue #107).
///
/// The file it means opens perfectly well: a GoPro `.360` is a valid MP4 with
/// two HEVC tracks in it, so nothing downstream fails until the trailer read
/// finds no trailer, and "file has no Insta360 trailer" is what a corrupt
/// file says too. The shell turns this error into a line that names the
/// format instead.
///
/// A file nothing recognizes is not refused: the second file of an X2-class
/// pair carries no trailer and no maker's mark, and it is a file Kjerag plays
/// (`kjerag_meta::sibling`).
fn ours(path: &Path) -> Fallible<()> {
    match Format::sniff(path) {
        Format::Foreign(foreign) => Err(Box::new(foreign)),
        Format::Insta360 | Format::Osmo | Format::Unknown => Ok(()),
    }
}

fn calibrated(path: &Path, size: Size, streams: usize) -> Fallible<Calibrated> {
    let calibration = CalibrationSet::from_capture(path)?;
    // The calibration's pixel numbers are already in delivered-frame
    // coordinates, so they describe this texture only if the stream is the
    // size the trailer says it is. A mismatch reprojects at the wrong scale,
    // which reads as a mild lens error rather than a bug.
    if (calibration.dimension.width, calibration.dimension.height) != (size.width, size.height) {
        return Err(format!(
            "trailer says lens frames are {}x{} but the stream decodes {}x{}",
            calibration.dimension.width, calibration.dimension.height, size.width, size.height
        )
        .into());
    }
    let sampled = streams.min(MAX_LENSES);
    let lenses = calibration
        .lenses
        .get(..sampled)
        .ok_or_else(|| {
            format!(
                "file decodes {streams} lens streams but the trailer calibrates {}",
                calibration.lenses.len()
            )
        })?
        .to_vec();
    // The camera key is printed because it is what a seam calibration is
    // filed under, and a pilot with two cameras or a bug report to write has
    // no other way to see which one this file came off. It names the unit
    // without naming it: model and factory calibration, hashed, no serial.
    println!(
        "lens:   {} {}, sampling {sampled} of {} calibrated, camera {:016x}",
        calibration.camera_model,
        calibration.firmware,
        calibration.lenses.len(),
        calibration.camera_key(),
    );

    let orientation = calibration.orientation(Filter::default());
    // What this line reports is whatever the file turned out to have, and a
    // file can have none: a capture with no inertial record at all, and a DJI
    // `.OSV` whose own gravity refused the mounting its orientations would have
    // been read with (`kjerag_meta::osmo`, which has already said so and why).
    // Neither has a sample count, a rate or an axis convention worth printing,
    // and printing the first branch's zeros for them reads as a broken IMU
    // rather than an absent one. The `level:` line below is what those two get.
    if !calibration.imu.samples().is_empty() {
        println!(
            "imu:    {} samples at {:.0} Hz, {} orientations, axes {}",
            calibration.imu.samples().len(),
            calibration.imu.rate_hz(),
            orientation.samples().len(),
            calibration.gyro.imu_orientation,
        );
    } else if !orientation.is_empty() {
        // A camera that solved its own has no axis convention to name and no
        // filter to have run: what it wrote is what is held.
        println!(
            "imu:    {} orientations the camera solved for itself, one a frame",
            orientation.samples().len()
        );
    }
    // Said out loud, because the alternative is a menu item that does nothing
    // and a pilot who cannot tell that from a broken one. A capture whose
    // telemetry carries no orientation at all is the case: the shell draws the
    // item disabled off [`Scene::has_orientation`], which is the same fact.
    if orientation.is_empty() {
        println!(
            "level:  this capture carries no orientation record Kjerag can use, so horizon \
             lock does nothing on it and the view is yours to pan"
        );
    }
    let held = Motion {
        orientation,
        exposure: calibration.exposure[0].clone(),
        readout: calibration.readout(),
    };
    let camera = calibration.camera_key();
    let (one_xs, one_xs_calibration) =
        if ONE_XS_PLAYBACK_ENABLED && projection::is_one_xs_lens_pair(&lenses) {
            let calibration = Arc::new(calibration);
            (
                Some(Arc::new(OneXsCapture::new(&calibration)?)),
                Some(calibration),
            )
        } else {
            (None, None)
        };
    Ok(Calibrated {
        lenses: lenses.into(),
        camera,
        held: Arc::new(held),
        one_xs,
        one_xs_calibration,
    })
}

/// Everything one open capture's trailer contributes, in one piece so that
/// opening a file hands it over in one piece.
struct Calibrated {
    lenses: Arc<[Lens]>,
    camera: u64,
    held: Arc<Motion>,
    one_xs: Option<Arc<OneXsCapture>>,
    one_xs_calibration: Option<Arc<CalibrationSet>>,
}

/// What the shell hands the renderer for one frame.
#[derive(Debug)]
pub struct ScenePrimitive {
    camera: Camera,
    view: Option<View>,
    /// How the pass samples a magnified picture, which is a property of the
    /// redraw rather than of the frame in it.
    sampling: Sampling,
    /// Whether the Studio optical-flow correction is on for this redraw, read
    /// from the [`Scene`]'s runtime toggle ([`Scene::set_flow`]). Default off,
    /// so the shipped player carries `false` and draws the byte-identical pass.
    flow: bool,
    /// A handle on the [`Scene`]'s shutter, not a copy of it: the request
    /// is taken by whichever redraw reaches [`ScenePipeline::prepare`]
    /// first, and one that never does is still armed for the next.
    shutter: Shutter,
    /// A handle on the [`Scene`]'s stall slot, the same way and for the same
    /// reason, in the other direction: the pass writes and the shell reads
    /// (issue #124).
    stalled: Stalled,
    /// And on the slot the pass keeps the last frame it drew of this capture
    /// in, which it both writes and reads.
    shown: Shown,
}

/// A pair of decoded lenses and the calibration that reprojects them. Both
/// halves are shared, so a redraw that changes nothing but the camera costs
/// two atomic increments.
#[derive(Clone, Debug)]
struct View {
    lenses: Arc<[Lens]>,
    /// What the along-seam axis still disagrees by after the pose, direction
    /// by direction (issue #103, stage 9). Part of this camera's calibration
    /// and carried with it, like the lenses above.
    table: Table,
    frames: Arc<Frames>,
    /// Where the body was when these frames were taken, already inverted for
    /// the pass. Identity with the lock off.
    held: Held,
    /// Capture-owned sequential stitch state. `None` for every other camera
    /// and for stepped diagnostic scenes.
    one_xs: Option<Arc<OneXsCapture>>,
}

/// Resolves the selected route's shutter at its final display boundary.
/// A complete exact display wins even after a terminal successor failure;
/// without one, only a terminal error consumes the request. A merely pending
/// frame therefore leaves it armed for the redraw that completes the map.
fn resolve_selected_shutter(
    shutter: &Shutter,
    stalled: &Stalled,
    complete_display: bool,
) -> Option<Request> {
    if complete_display {
        return shutter.take();
    }
    if let Some(error) = stalled.terminal() {
        shutter.fail(error);
    }
    None
}

/// The last view the pass actually presented of one capture, which is what the
/// pane holds while a newer one cannot be imported (issue #124).
///
/// It belongs to the capture rather than to the pipeline, for the reason the
/// whole issue is about: iced keeps one pipeline for the life of the window,
/// so whatever it remembers about this file it remembers about the next one.
/// Kept on the pipeline for one commit, this opened a new file onto the last
/// frame of the file before it, until the first frame of the new one arrived.
/// Issue #125's check is what caught it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Shown(Arc<Mutex<Option<View>>>);

impl Shown {
    fn keep(&self, view: &View) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(view.clone());
        }
    }

    fn get(&self) -> Option<View> {
        self.0.lock().ok()?.clone()
    }

    fn frame(&self) -> Option<(u64, Duration)> {
        let slot = self.0.lock().ok()?;
        let frames = &slot.as_ref()?.frames;
        Some((frames.index, frames.timestamp))
    }
}

/// The GPU state behind the widget. iced builds one of these per primitive
/// type and keeps it for the life of the renderer.
pub struct ScenePipeline {
    /// The authoritative iced device and queue pair for every selected ONE X2
    /// resident stage owned by this renderer pipeline.
    one_xs_gpu: OneXsGpuContext,
    pipeline: wgpu::RenderPipeline,
    /// The same draw with the Studio optical-flow apply compiled in, chosen per
    /// draw when the runtime flow toggle is on ([`ScenePipeline::draw`]). Built
    /// unconditionally beside the plain pipeline so the toggle needs neither the
    /// `KJERAG_FLOW` env nor a pipeline rebuild; off, it is never bound and the
    /// plain `pipeline` above draws the byte-identical shipped picture.
    flow_pipeline: wgpu::RenderPipeline,
    /// The same draw with the selected native ONE X2 1080-by-60 retained-field
    /// apply compiled in. This remains instrument-only: normal selected
    /// playback uses the direct type-2 draw, while the V6 oracle can upload
    /// captured fields and exercise this retained-field pass.
    one_xs_flow_pipeline: wgpu::RenderPipeline,
    /// A one-shot dense captured-map draw installed only by older headless
    /// oracles. The typed frame-bound consumer owns its draw separately, so it
    /// cannot become pipeline or playback state.
    map_oracle: Option<MapOracleDraw>,
    /// Exact final projection and delivered pair written by the latest
    /// typed preparation, including selected playback. A fresh identity on
    /// every such preparation keeps a map from surviving a changed view of
    /// the same frame. Other ordinary routes leave this empty.
    prepared_picture: Option<PreparedPicture>,
    /// Lazy access to the exact bound R8 source pair, used by selected
    /// diagnostics. Production selected playback does not construct it.
    one_xs_luma: Option<Box<LumaReadbackPipeline>>,
    /// Exact GPU sampler/reducer for production selected playback. It reads
    /// the imported R8 pair and returns only the two compact 1080-by-60 U8
    /// solver inputs; retained estimator state remains capture-owned on CPU.
    one_xs_belts: Option<Box<GpuSolverBeltPipeline>>,
    /// Lazily built, device-qualified paired PIS kernel. CPU preparation of
    /// each source model remains the explicit producer boundary.
    one_xs_pis: Option<Box<GpuPisPipeline>>,
    /// Frame-path instrumentation counts paired GPU stages that returned typed
    /// grids, not queue submissions, reservations or map installs.
    one_xs_gpu_pis_completed_stages: u64,
    /// Lazily built production/direct type-2 consumer. Its pipeline and
    /// exact-size buffers are reused; only the two map payloads and their
    /// CPU-side frame association change between frames and diagnostics.
    direct_one_xs_map: Option<DirectMapDraw>,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    /// The seam band, measured on the frame the draw is about to sample and
    /// dispatched from `prepare` (issue #103).
    band: Band,
    /// One black pixel, bound wherever a lens has no stream: before a file is
    /// open, and in the second slot of a file that carries one lens.
    blank: Planes,
    bind_group: wgpu::BindGroup,
    /// The frame the bind group points at, and the ones still in flight
    /// behind it. Newest first.
    live: VecDeque<Live>,
    /// The target this pass was built for, which is iced's surface format.
    /// A capture renders into a texture of the same format, so that what it
    /// reads back is what the compositor would have been handed.
    format: wgpu::TextureFormat,
    reported: bool,
    /// Where the drawn handover line is being held ([`SeamAnchor`]), carried
    /// from one redraw to the next because a held line is a thing with a
    /// history and the block that carries it to the GPU is rebuilt from
    /// nothing every frame.
    ///
    /// `None` until the first redraw.
    anchor: Option<SeamAnchor>,
    /// Which displacement contract the next draw reads. [`Self::prepare`]
    /// selects plain or legacy for other cameras and direct ONE X2 or nothing
    /// for selected playback. The V6 instrument can replace only its own
    /// one-draw choice after preparation with [`Self::upload_one_xs_flow`]; a
    /// later prepare restores normal playback selection, so captured corpus
    /// data can never become held player state accidentally.
    flow_draw: FlowDraw,
    /// The exact selected ONE X2 delivery whose source, projection uniform and
    /// native map completed as one display transaction. The GPU objects stay
    /// in their existing owners; this stamp is the admission proof used to
    /// restore that tuple after preparation of its successor fails.
    one_xs_display: ExactDisplay<FrameStamp>,
    /// The player's asynchronous optical-flow estimator: Studio's ~30-frame
    /// cadence, the background DIS worker, and the async strip readback that
    /// feeds it ([`Self::flow_step`]). None of it runs while the toggle is off,
    /// and when it does the render thread never blocks on the DIS.
    flow: Flow,
}

/// The three shader contracts that can draw one prepared frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlowDraw {
    /// Selected ONE X2 has no exact map for the bound source yet. Drawing
    /// nothing leaves the pane's freshly painted backdrop visible and cannot
    /// expose either an unstitched source pair or a prior map on a new pair.
    Nothing,
    Plain,
    Legacy,
    OneXs,
    DirectOneXs,
    MapOracle,
}

/// Identity of the last complete selected display transaction.
///
/// A successor is recorded only when its source and map carry the same opaque
/// delivery identity. Until then, including while it is pending or after it
/// fails, the previous identity remains recoverable.
#[derive(Debug)]
struct ExactDisplay<T> {
    complete: Option<T>,
}

impl<T> Default for ExactDisplay<T> {
    fn default() -> Self {
        Self { complete: None }
    }
}

impl<T: Clone + Eq> ExactDisplay<T> {
    fn commit(&mut self, source: &T, map: &T) -> bool {
        if source != map {
            return false;
        }
        self.complete = Some(source.clone());
        true
    }

    fn recovery_index(
        &self,
        shown: &T,
        map: Option<&T>,
        retained: impl IntoIterator<Item = (usize, bool)>,
    ) -> Option<usize> {
        if self.complete.as_ref() != Some(shown) || map != Some(shown) {
            return None;
        }
        retained
            .into_iter()
            .find_map(|(index, matches)| matches.then_some(index))
    }

    fn clear(&mut self) {
        self.complete = None;
    }
}

fn exact_direct_one_xs_draw<'a, T: Eq>(
    flow_draw: FlowDraw,
    complete: Option<&T>,
    bound: Option<&'a T>,
) -> Option<&'a T> {
    let bound = (flow_draw == FlowDraw::DirectOneXs).then_some(bound)??;
    (complete == Some(bound)).then_some(bound)
}

struct MapOracleDraw {
    pipeline: wgpu::RenderPipeline,
    _buffer: wgpu::Buffer,
    read: wgpu::BindGroup,
}

impl FlowDraw {
    /// The only selection normal frame preparation may make. Keeping this
    /// transition in one place makes the instrument override explicitly
    /// one-draw state rather than another playback mode.
    fn prepared(active: bool) -> Self {
        if active { Self::Legacy } else { Self::Plain }
    }

    fn selected_map(ready: bool) -> Self {
        if ready {
            Self::DirectOneXs
        } else {
            Self::Nothing
        }
    }
}

/// The player may run the legacy solver only for a route it actually owns.
fn player_flow(requested: bool, selected_one_xs: bool) -> bool {
    requested && !selected_one_xs
}

/// Neither the player nor the environment-driven research instrument may
/// select the legacy draw contract for the selected ONE X2 route. The
/// environment arm remains separate because it uploads its own field and must
/// not start the player's worker.
fn legacy_flow_draw(requested: bool, environment: bool, selected_one_xs: bool) -> bool {
    (requested || environment) && !selected_one_xs
}

/// One frame on the GPU. The mapped frames must outlive the textures
/// imported from them, and both must outlive the passes that read them,
/// which is what [`RETAINED`] is about.
struct Live {
    frames: Arc<Frames>,
    planes: Vec<Planes>,
}

/// The compute half of the seam: the pipeline that measures the overlap band,
/// the state it accumulates into, and where in the film the state is.
///
/// It is dispatched from [`ScenePipeline::prepare`] and never from `draw`,
/// which is handed a render pass it cannot leave. The submit order is what
/// makes that correct and it is the same rule the capture already relies on:
/// a submit from `prepare` lands before iced's own submit of the pass that
/// reads its result.
struct Band {
    pipeline: wgpu::ComputePipeline,
    /// The second dispatch of the same pass: what the ring just read, pooled
    /// into one exposure for the picture (issue #103, stage 3).
    pool: wgpu::ComputePipeline,
    /// The along-seam field fitted over the whole ring, dispatched beside the
    /// exposure pooling and over the same cells (issue #103, stage 5).
    pool_along: wgpu::ComputePipeline,
    /// Studio's grid, in the order the frame runs it: read the evidence band,
    /// admit and weigh it, solve the 5,088-node system, then blur, ramp and
    /// ease what came out (docs/research/chromatic.md P).
    grid_gate: wgpu::ComputePipeline,
    grid_read: wgpu::ComputePipeline,
    grid_blur: wgpu::ComputePipeline,
    grid_admit: wgpu::ComputePipeline,
    grid_solve: wgpu::ComputePipeline,
    grid_finish: wgpu::ComputePipeline,
    /// The two rectified seam line-image strips, and the pass that fills them
    /// (docs/research/studio-seam-re.md §35). An INSTRUMENT: dispatched only by
    /// [`ScenePipeline::band_strips`], never by [`ScenePipeline::measure`], so
    /// the shipped picture never sees it.
    strip: wgpu::ComputePipeline,
    strips: wgpu::Buffer,
    /// The two DIS flow fields the draw applies (§38/§40): `4 × STRIP_W × STRIP_H`
    /// floats, the `u` plane (along-seam) then the `v` plane (across-seam), in
    /// belt/sample pixels. Created zeroed (which displaces nothing) and written
    /// by [`ScenePipeline::upload_flow`]. Only bound into the draw's read group
    /// when [`flow_on`]; off, it is an unread buffer.
    flow: wgpu::Buffer,
    /// The selected ONE X2 1080-by-60 displacement, held separately from the
    /// incompatible legacy layout. A later normal prepare may select the
    /// legacy pipeline before its worker publishes another field, so sharing
    /// one allocation would let captured oracle bytes leak into playback.
    one_xs_flow: wgpu::Buffer,
    /// The grid's control block, kept so an instrument can read how often the
    /// content gate said no.
    control: wgpu::Buffer,
    /// The grid's applied field, kept for the same reason.
    shown: wgpu::Buffer,
    /// One [`band::Cell`] per direction, read by the draw and written here.
    state: wgpu::Buffer,
    watch: wgpu::Buffer,
    /// The same buffer twice: writable for the dispatch, read-only for the
    /// draw. Two groups over one buffer, and never both in one pass.
    group: wgpu::BindGroup,
    read: wgpu::BindGroup,
    /// The read group the flow draw variant takes: the same state and chromatic
    /// bindings as `read`, plus the composed flow displacement on
    /// [`FLOW_BINDING`]. Chosen by [`ScenePipeline::draw`] when the runtime
    /// toggle is on; otherwise `read` above is used and this is never bound.
    flow_read: wgpu::BindGroup,
    /// The same read group shape with the dedicated selected ONE X2 field on
    /// binding 3. Only [`FlowDraw::OneXs`] selects it.
    one_xs_flow_read: wgpu::BindGroup,
    /// Set by an instrument to stop measuring (`ScenePipeline::hold_band`).
    held: bool,
    /// Set by an instrument to leave the exposure alone
    /// (`ScenePipeline::hold_tone`). The ring is still measured and the bend
    /// is still applied; only the pooling is not dispatched, so the header
    /// stays at the zero it was created in and `tone_split` returns exactly
    /// one on both sides. That is the picture stage 2 drew, and it is how a
    /// before and after differ by this stage and by nothing else.
    tone_held: bool,
    /// Which round of the circle the next frame reads.
    /// How many times the measurement is dispatched per redraw. One, except
    /// under [`ScenePipeline::band_repeats`].
    repeats: u32,
    slice: u32,
    /// Where the last measured frame sat in the film, so the next one knows
    /// how much media time the state has aged by, and whether what happened
    /// in between was play or a seek.
    at: Option<Duration>,
}

/// Everything one readback of the band's buffer yields: the pooled tone, the
/// along-seam fit, the ring of cells, and the chromatic field.
type BandState = (band::Tone, band::Along, Vec<band::Cell>, band::Field);

impl ScenePipeline {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let one_xs_gpu = OneXsGpuContext::new(device, queue);
        let layout = bind_group_layout(device);
        // Two groups: the pictures and the map, then the band's state. iced's
        // device is asked for a limit of exactly two (`iced_wgpu`), so this is
        // all of them, and there is nowhere for a third to go.
        //
        // Three draw pipelines, built unconditionally: the plain shipped pass,
        // the audited legacy-flow pass, and the selected ONE X2 retained-flow
        // pass used only by its headless oracle. Selected playback additionally
        // builds the direct type-2 map draw lazily. With legacy flow off, other
        // cameras bind only the plain pipeline. The two retained-flow variants
        // use the same binding number but separate typed storage allocations
        // and shaders.
        let (pipeline, flow_pipeline, one_xs_flow_pipeline) = {
            let build = |label: &'static str, source: String, flow_bytes: Option<u64>| {
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                });
                let reading = read_layout(device, flow_bytes);
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("scene"),
                        bind_group_layouts: &[&layout, &reading],
                        immediate_size: 0,
                    });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("scene"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs"),
                        compilation_options: Default::default(),
                        // Blending, which the pass did without until issue #100:
                        // the picture writes alpha 1 and replaces exactly what a
                        // replacing pipeline replaced, and the room around the
                        // ball writes alpha 0 and leaves whatever the shell drew
                        // behind the widget exactly as it found it.
                        // Premultiplied, which is what the room's own colour
                        // already is: black at alpha 0.
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    // iced's own pass has one sample and no depth attachment
                    // (`iced_wgpu/src/lib.rs`, "iced_wgpu render pass"); a
                    // pipeline that disagrees fails to draw rather than looking
                    // wrong.
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
            };
            (
                build("scene", draw_wgsl_flow(false), None),
                build(
                    "scene legacy flow",
                    draw_wgsl_flow(true),
                    Some(band::FLOW_BYTES),
                ),
                build(
                    "scene ONE X2 flow",
                    draw_wgsl_one_xs_flow(),
                    Some(crate::flow::one_xs::Displacement::BYTES as u64),
                ),
            )
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene"),
            size: std::mem::size_of::<Reframe>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let blank = blank_planes(device);
        let band = Band::new(device, &layout);
        let bind_group = bind(device, &layout, &uniforms, [&blank; MAX_LENSES], &sampler);

        Self {
            one_xs_gpu,
            pipeline,
            flow_pipeline,
            one_xs_flow_pipeline,
            map_oracle: None,
            prepared_picture: None,
            one_xs_luma: None,
            one_xs_belts: None,
            one_xs_pis: None,
            one_xs_gpu_pis_completed_stages: 0,
            direct_one_xs_map: None,
            layout,
            sampler,
            uniforms,
            band,
            blank,
            bind_group,
            live: VecDeque::new(),
            format,
            reported: false,
            anchor: None,
            // No draw contract exists before the first prepare. That prepare
            // admits the environment instrument only for a non-ONE-X2 route.
            flow_draw: FlowDraw::Plain,
            one_xs_display: ExactDisplay::default(),
            flow: Flow::new(),
        }
    }

    /// The band, measured on the pair the bind group points at, before the
    /// draw that will read what it wrote.
    ///
    /// One dispatch, one submit, no readback: the state lives on the GPU for
    /// its whole life, so nothing here waits on a fence and nothing stalls the
    /// pipeline. The reason it is here and not in `draw` is that `draw` is
    /// handed a render pass and cannot open a compute one; the reason that is
    /// **correct** is the same rule the capture already relies on, written at
    /// the uniform write above: a submit from `prepare` lands before iced's
    /// own submit of the pass that reads its result.
    fn measure(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, view: Option<&View>) {
        // A file with one lens stream has no seam, and a redraw with no new
        // frame has nothing new to read. The state stays where it is, which
        // for a one-lens file is the zero it was created in.
        let Some(view) =
            view.filter(|view| !self.band.held && view.lenses.len() > 1 && self.is_bound(view))
        else {
            return;
        };
        let Some(watch) = self.band.aged(view.frames.timestamp) else {
            return;
        };
        queue.write_buffer(&self.band.watch, 0, watch.bytes());
        let mut encoder = device.create_command_encoder(&Default::default());
        // Only ever more than one under `band_repeats`, and then each in a pass
        // of its own: dispatches inside one pass have no barrier between them
        // and a device is free to overlap them, which measures throughput where
        // what is wanted is one redraw's worth of latency.
        for _ in 1..self.band.repeats {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("band repeat"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.band.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(1, &self.band.group, &[]);
            pass.dispatch_workgroups(watch.groups(), 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("band"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.band.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(1, &self.band.group, &[]);
            pass.dispatch_workgroups(watch.groups(), 1, 1);
            // One workgroup over the state the dispatch above just wrote.
            // Two dispatches of one pass are ordered against each other by
            // WebGPU itself, so what this pools is this frame's readings and
            // no barrier has to be asked for.
            if !self.band.tone_held {
                pass.set_pipeline(&self.band.pool);
                pass.dispatch_workgroups(1, 1, 1);
            }
            pass.set_pipeline(&self.band.pool_along);
            pass.dispatch_workgroups(1, 1, 1);
            // And the chromatic field, on the readings the same dispatch
            // wrote. Warm-started from what it left last frame, which is why
            // it is in the state buffer and not a scratch one.
            // And Studio's grid, on the same frame's pictures. Four dispatches
            // in order, and WebGPU orders them against each other, so the
            // admission sees every difference and the solve sees every weight
            // without a barrier being asked for.
            pass.set_pipeline(&self.band.grid_gate);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&self.band.grid_read);
            pass.dispatch_workgroups(chroma::READ_GROUPS, 1, 1);
            pass.set_pipeline(&self.band.grid_blur);
            pass.dispatch_workgroups(chroma::READ_GROUPS, 1, 1);
            pass.set_pipeline(&self.band.grid_admit);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&self.band.grid_solve);
            pass.dispatch_workgroups(1, 1, 1);
            pass.set_pipeline(&self.band.grid_finish);
            pass.dispatch_workgroups(1, 1, 1);
        }
        queue.submit([encoder.finish()]);
    }

    /// iced picks an sRGB surface when it gamma-corrects, and the GPU then
    /// encodes whatever the shader writes. Video is already gamma-encoded,
    /// so it has to be decoded back to linear first or the picture washes
    /// out.
    fn linearize(&self) -> bool {
        self.format.is_srgb()
    }

    /// `aspect` is the output's width over its height, which is what decides
    /// the vertical field of view. The widget reads it from its bounds.
    pub fn prepare(
        &mut self,
        primitive: &ScenePrimitive,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        aspect: f32,
    ) {
        let selected_one_xs = one_xs_playback_selected(
            ONE_XS_PLAYBACK_ENABLED,
            primitive
                .view
                .as_ref()
                .is_some_and(|view| view.one_xs.is_some())
                || primitive
                    .shown
                    .get()
                    .is_some_and(|view| view.one_xs.is_some()),
        );
        if selected_one_xs {
            let gpu = self.one_xs_gpu.clone();
            if let Err(error) = gpu.ensure_same(&OneXsGpuContext::new(device, queue)) {
                // A foreign pair cannot touch retained imports, bind groups,
                // uniforms or recovery state. Preserve the failure site's raw
                // identity error and leave the last complete display owned by
                // the authoritative context.
                self.flow_draw = FlowDraw::Nothing;
                primitive.stalled.fail_now(error);
                return;
            }
            let device = gpu.device();
            let queue = gpu.queue();
            if primitive.stalled.stopped() {
                self.restore_one_xs_display(primitive, device, queue, aspect);
                return;
            }
            if let Err(error) = self.prepare_one_xs_playback(primitive, aspect) {
                primitive.stalled.fail_now(error);
                self.restore_one_xs_display(primitive, device, queue, aspect);
            } else if self.flow_draw != FlowDraw::DirectOneXs {
                // Initial decode and any future asynchronous implementation
                // may have a source without an exact completed map yet.
                self.restore_one_xs_display(primitive, device, queue, aspect);
            }
        } else {
            self.one_xs_display.clear();
            let _ = self.prepare_inner(primitive, device, queue, aspect, false);
        }
    }

    /// Opaque identity of the native map bound for the next production
    /// selected ONE X2 draw.
    ///
    /// This is an allocation-free instrument observable. It returns a stamp
    /// only when the selected route is [`FlowDraw::DirectOneXs`], its direct
    /// type-2 resource owns a bound map, and that map is the same complete
    /// display transaction the pipeline admitted. It exposes no map payload.
    pub fn diagnostic_one_xs_direct_frame(&self) -> Option<&FrameStamp> {
        exact_direct_one_xs_draw(
            self.flow_draw,
            self.one_xs_display.complete.as_ref(),
            self.direct_one_xs_map
                .as_ref()
                .and_then(DirectMapDraw::bound_frame),
        )
    }

    /// Prepare through the ordinary picture path and return a diagnostic
    /// capability for the exact final projection it wrote.
    ///
    /// Selected playback also creates preparation identity internally. Other
    /// ordinary routes allocate none.
    pub fn prepare_one_xs_picture(
        &mut self,
        primitive: &ScenePrimitive,
        aspect: f32,
    ) -> Option<PreparedPicture> {
        let gpu = self.one_xs_gpu.clone();
        let device = gpu.device();
        let queue = gpu.queue();
        let _ = self.prepare_inner(primitive, device, queue, aspect, true);
        self.prepared_picture.clone()
    }

    /// Correctness-first live selected ONE X2 transaction.
    ///
    /// The GPU samples retained maps, performs exact 3-by-3 reduction on the
    /// imported R8 pair, then executes every selected paired PIS stage. CPU
    /// construction and upload of each prepared source model remain explicit.
    /// Source bindings, prepared geometry, sequential retained state, solver
    /// receipts, uploaded map and draw all name the same full [`FrameStamp`].
    fn prepare_one_xs_playback(&mut self, primitive: &ScenePrimitive, aspect: f32) -> Fallible<()> {
        let gpu = self.one_xs_gpu.clone();
        let device = gpu.device();
        let queue = gpu.queue();
        let Some(frames) = self.prepare_inner(primitive, device, queue, aspect, true) else {
            self.flow_draw = FlowDraw::Nothing;
            return Ok(());
        };
        // `prepare_inner` may deliberately hold the last successfully bound
        // frame when a newer import fails. Select the View for those exact
        // bindings, never merely the newest source offered by the player.
        let view = primitive
            .view
            .as_ref()
            .filter(|view| self.is_bound(view))
            .cloned()
            .or_else(|| primitive.shown.get().filter(|view| self.is_bound(view)))
            .ok_or("ONE X2 playback has bound source textures without their capture view")?;
        if view.frames.stamp() != frames.stamp() {
            return Err("ONE X2 playback source view differs from its bound lens pair".into());
        }
        let capture = view
            .one_xs
            .as_ref()
            .ok_or("ONE X2 playback lost its capture-owned stitch state")?;

        let map = match capture.reserve(&frames.stamp(), frames.size)? {
            OneXsPreparation::Ready(map) => map,
            OneXsPreparation::InFlight(ready) => {
                self.flow_draw = FlowDraw::Nothing;
                // `prepare_inner` may already have made the offered successor
                // frontmost. Seed a recreated pipeline from the capture-owned
                // completed map, then put the last exact source/map/uniform
                // tuple back without making a second estimator submission.
                self.seed_one_xs_display(primitive, device, queue, ready.as_deref());
                self.restore_one_xs_display(primitive, device, queue, aspect);
                return Ok(());
            }
            OneXsPreparation::Reserved(reservation) => {
                // Qualify every pipeline required by this transaction before
                // submitting any per-frame GPU work. `InFlight` returned
                // above without constructing it.
                if self.one_xs_pis.is_none() {
                    let pipeline = match GpuPisPipeline::new(device, queue) {
                        Ok(pipeline) => pipeline,
                        Err(error) => {
                            return Err(abort_one_xs_after_error(reservation, error));
                        }
                    };
                    self.one_xs_pis = Some(Box::new(pipeline));
                }
                let pending =
                    match self.submit_one_xs_solver_belts(frames.clone(), reservation.prepared()) {
                        Ok(pending) => pending,
                        Err(error) => {
                            return Err(abort_one_xs_after_error(reservation, error));
                        }
                    };
                // The presentation policy admits only one frame at a time.
                // Waiting occurs outside the capture mutex, and retained CPU
                // history is leased only after this exact readback succeeds.
                let blurred_belts = match pending.read(reservation.prepared()) {
                    Ok(blurred_belts) => blurred_belts,
                    Err(error) => {
                        return Err(abort_one_xs_after_error(reservation, error));
                    }
                };
                let input = ColdInputs::from_blurred_belts_and_masks(
                    blurred_belts,
                    reservation.prepared().masks().clone(),
                );
                let (controls, prepared) = ColdPreparedSchedule::from_cpu(&input).into_parts();
                let mut solver = ReservationGpuPisSolver {
                    flight: reservation.flight.clone(),
                    pipeline: self
                        .one_xs_pis
                        .as_deref()
                        .expect("the selected reservation initialized GPU PIS"),
                    device,
                    queue,
                    completed_stages: &mut self.one_xs_gpu_pis_completed_stages,
                    prepared,
                };
                match reservation.commit_prepared_with_solver(controls, &mut solver) {
                    Ok(completed) => completed
                        .install()
                        .map_err(|rejected| rejected as Box<dyn std::error::Error + Send + Sync>)?,
                    Err(rejected) => {
                        let RejectedOneXsSolverReservation { reservation, error } = *rejected;
                        return Err(abort_one_xs_after_error(reservation, error.into()));
                    }
                }
            }
        };
        MapBindError::require_frame(map.frame(), self.prepared_picture.as_ref())?;
        let source = frames.stamp();
        if !self.one_xs_display.commit(&source, map.frame()) {
            return Err(crate::studio_type2::FrameMapMismatch::new(
                "source",
                &source,
                "map",
                map.frame(),
            )
            .into());
        }
        let draw = self
            .direct_one_xs_map
            .get_or_insert_with(|| DirectMapDraw::new(device, &self.layout, self.format));
        if draw.bound_frame() != Some(map.frame()) {
            draw.upload(queue, &map);
        }
        debug_assert_eq!(draw.bound_frame(), Some(map.frame()));
        self.flow_draw = FlowDraw::selected_map(true);
        primitive.shown.keep(&view);

        // A still request waits behind the same exact map rather than taking
        // an unstitched or blank picture while the transaction is pending.
        if let Some(request) = primitive.shutter.take() {
            self.shoot(device, queue, request, aspect, Some(&view));
        }
        Ok(())
    }

    /// Restore the last complete selected display after preparation of its
    /// successor waits or fails.
    ///
    /// `show` may already have imported the successor and replaced the source
    /// bind group, and `prepare_inner` may already have written its uniform.
    /// Merely retaining [`FlowDraw::DirectOneXs`] would therefore pair the old
    /// map with the new source. Recovery is admitted only when the Scene-owned
    /// shown view, retained imported planes, completed transaction and native
    /// map all name the same opaque [`FrameStamp`]. It then makes that retained
    /// source frontmost again and rebuilds the uniform for the current camera
    /// and target aspect before selecting the direct draw.
    fn seed_one_xs_display(
        &mut self,
        primitive: &ScenePrimitive,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ready: Option<&OneXsMapFrame>,
    ) {
        let Some(ready) = ready else {
            return;
        };
        let Some(view) = primitive
            .shown
            .get()
            .filter(|view| view.frames.stamp() == *ready.frame())
        else {
            return;
        };

        // A genuinely recreated iced pipeline has no retained import or map
        // resource of its own. Both are reconstructible from capture-owned
        // state without touching the in-flight estimator reservation.
        self.show(device, &view, primitive);
        if !self.is_bound(&view) {
            return;
        }
        let draw = self
            .direct_one_xs_map
            .get_or_insert_with(|| DirectMapDraw::new(device, &self.layout, self.format));
        if draw.bound_frame() != Some(ready.frame()) {
            draw.upload(queue, ready);
        }
        let source = view.frames.stamp();
        let _ = self.one_xs_display.commit(&source, ready.frame());
    }

    fn restore_one_xs_display(
        &mut self,
        primitive: &ScenePrimitive,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        aspect: f32,
    ) {
        self.flow_draw = FlowDraw::Nothing;
        let Some(view) = primitive.shown.get().filter(|view| view.one_xs.is_some()) else {
            resolve_selected_shutter(&primitive.shutter, &primitive.stalled, false);
            return;
        };
        let frame = view.frames.stamp();
        let map = self
            .direct_one_xs_map
            .as_ref()
            .and_then(|draw| draw.bound_frame());
        let retained = self.one_xs_display.recovery_index(
            &frame,
            map,
            self.live
                .iter()
                .enumerate()
                .map(|(index, live)| (index, Arc::ptr_eq(&live.frames, &view.frames))),
        );
        let Some(retained) = retained else {
            resolve_selected_shutter(&primitive.shutter, &primitive.stalled, false);
            return;
        };

        // `is_bound` deliberately means `live.front`, so moving the exact
        // retained import is part of restoration rather than bookkeeping.
        let live = self
            .live
            .remove(retained)
            .expect("retained source position came from this queue");
        self.live.push_front(live);
        let live = self
            .live
            .front()
            .expect("restored selected source was pushed to the front");
        self.bind_group = bind(
            device,
            &self.layout,
            &self.uniforms,
            std::array::from_fn(|lens| live.planes.get(lens).unwrap_or(&self.blank)),
            &self.sampler,
        );

        let reframe = Reframe::new(
            &view.lenses,
            view.frames.size,
            primitive.camera,
            view.held,
            aspect,
            self.linearize(),
            primitive.sampling,
        )
        .with_samples(view.frames.samples)
        .with_table(view.table);
        self.anchor = None;
        self.map_oracle = None;
        self.prepared_picture = Some(PreparedPicture::new(frame, reframe, aspect));
        queue.write_buffer(&self.uniforms, 0, reframe.bytes());
        self.flow_draw = FlowDraw::DirectOneXs;

        if let Some(request) =
            resolve_selected_shutter(&primitive.shutter, &primitive.stalled, true)
        {
            self.shoot(device, queue, request, aspect, Some(&view));
        }
    }

    /// Diagnostic wrapper around exact full R8 readback for the lens pair the
    /// picture actually bound.
    ///
    /// Selection and submission are one transaction. If a newer offered frame
    /// cannot be imported, this captures the held frame the draw still samples,
    /// not the newer offer. Production selected playback instead submits the
    /// compact GPU solver-belt producer directly.
    pub fn prepare_one_xs_luma(
        &mut self,
        primitive: &ScenePrimitive,
        aspect: f32,
    ) -> Fallible<Option<PendingOneXsLuma>> {
        let gpu = self.one_xs_gpu.clone();
        let device = gpu.device();
        let queue = gpu.queue();
        let Some(frames) = self.prepare_inner(primitive, device, queue, aspect, false) else {
            return Ok(None);
        };
        Ok(Some(self.submit_one_xs_luma(frames)?))
    }

    /// Submit source extraction for the exact pair already bound by this
    /// preparation. Keeping this separate lets live playback prepare once,
    /// then wait for and consume that same binding without another import or
    /// uniform write between source and map ownership.
    fn submit_one_xs_luma(&mut self, frames: Arc<Frames>) -> Fallible<PendingOneXsLuma> {
        let gpu = self.one_xs_gpu.clone();
        let device = gpu.device();
        let queue = gpu.queue();
        let shape = {
            let live = self
                .live
                .front()
                .ok_or("ONE X2 source readback has no bound lens pair")?;
            if !Arc::ptr_eq(&live.frames, &frames) {
                return Err(
                    "ONE X2 source readback frame differs from the picture bindings".into(),
                );
            }
            one_xs_luma::validate(&live.planes, &frames)?
        };
        let readback = self
            .one_xs_luma
            .get_or_insert_with(|| Box::new(LumaReadbackPipeline::new(device, &self.layout)));
        Ok(readback.submit(device, queue, &self.bind_group, frames, shape))
    }

    /// Submit exact retained-map sampling and area reduction for the bound
    /// source pair. The returned token owns both the imported decoder surfaces
    /// and their opaque delivery stamp until its compact readback is consumed.
    fn submit_one_xs_solver_belts(
        &mut self,
        frames: Arc<Frames>,
        prepared: &PreparedFrame,
    ) -> Fallible<PendingOneXsBlurredBelts> {
        let frame = frames.stamp();
        if prepared.frame() != &frame {
            return Err(crate::studio_type2::FrameMapMismatch::new(
                "prepared geometry",
                prepared.frame(),
                "bound source",
                &frame,
            )
            .into());
        }
        let live = self
            .live
            .front()
            .ok_or("ONE X2 GPU solver belts have no bound lens pair")?;
        if !Arc::ptr_eq(&live.frames, &frames) {
            return Err("ONE X2 GPU solver-belt source differs from the picture bindings".into());
        }
        one_xs_luma::validate_source_pair(&live.planes, &frames, "GPU solver belts")?;
        let sources = SourceTextures {
            a: &live.planes[0].luma,
            b: &live.planes[1].luma,
        };
        if self.one_xs_belts.is_none() {
            self.one_xs_belts = Some(Box::new(GpuSolverBeltPipeline::new(
                self.one_xs_gpu.clone(),
            )?));
        }
        let producer = self
            .one_xs_belts
            .as_ref()
            .expect("the exact GPU solver-belt producer was just qualified");
        let pending = producer.submit_retained(sources, prepared.retained_base_maps(), frames)?;
        Ok(PendingOneXsBlurredBelts { frame, pending })
    }

    /// How many imported frame pairs the inactive source oracle can currently
    /// see, and the production retention bound that limits that queue.
    ///
    /// Normal playback never calls this. The source oracle uses it to prove
    /// that submitted readbacks survive eviction of their imported frame from
    /// the draw pipeline, rather than merely completing while still retained.
    pub fn one_xs_luma_retention(&self) -> (usize, usize) {
        (self.live.len(), RETAINED)
    }

    fn prepare_inner(
        &mut self,
        primitive: &ScenePrimitive,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        aspect: f32,
        bind_map_picture: bool,
    ) -> Option<Arc<Frames>> {
        // Every preparation is a new picture transaction, even if it happens
        // to draw the same delivered pair through identical values.
        self.prepared_picture = None;
        if !self.reported {
            self.reported = true;
            println!("device: {}", dmabuf::device_report(device));
        }
        if let Some(view) = &primitive.view {
            self.show(device, view, primitive);
        }

        // The pane holds the last frame this pipeline actually presented
        // whenever the one it is offered is not on the GPU (issue #124).
        // Frames keep arriving while imports fail, so drawing strictly by
        // what the shell offers takes the picture away for as long as the
        // failures last and leaves it away once the capture is stopped, which
        // is a second failure on top of the first from where the pilot sits.
        // Owner: "Why does the video disappear instead of just freezing on
        // current frame though? Its jarring".
        //
        // Every gap rather than only the stopped one, because a hiccup that
        // costs frames should cost frames: a squeeze under the bound took the
        // pane to the backdrop and back before this.
        let showing = match &primitive.view {
            Some(view) if self.is_bound(view) => primitive.view.clone(),
            // Nothing ever presented is the one case with nothing to hold,
            // and the pane is the backdrop. `Stalled` says so in the terminal
            // line when it gives up, because on screen it looks like the
            // other kind of failure.
            _ => primitive.shown.get(),
        };
        let selected_one_xs = one_xs_playback_selected(
            ONE_XS_PLAYBACK_ENABLED,
            primitive
                .view
                .as_ref()
                .is_some_and(|view| view.one_xs.is_some())
                || showing.as_ref().is_some_and(|view| view.one_xs.is_some()),
        );

        let reframe = match &showing {
            Some(view) if self.is_bound(view) => Reframe::new(
                &view.lenses,
                view.frames.size,
                primitive.camera,
                view.held,
                aspect,
                self.linearize(),
                primitive.sampling,
            )
            .with_samples(view.frames.samples)
            .with_table(view.table),
            // No frame yet, or none this pipeline has managed to bind: the
            // pane is all room, which the shell's backdrop shows through.
            _ => Reframe::blank(aspect, self.linearize()),
        };
        // The one place the drawn handover line's own offset becomes a number
        // the shader can read. AFTER the block is built, because the follow is
        // measured against the very pose and lenses the draw will use, and
        // BEFORE the write, because that is the copy the GPU sees.
        //
        // The clock is the frame's own presentation time - not a wall clock
        // and not a count of redraws. The follow is charged in FILM, so a
        // redraw that arrives with the same frame behind it advances nothing,
        // and a run at 30 or at 300 fps follows over the same seconds of
        // picture.
        let reframe = if selected_one_xs {
            // The selected packed map already owns the complete handover.
            // Legacy held-line history is neither an input nor a fallback.
            self.anchor = None;
            reframe
        } else {
            let held = showing.as_ref().map_or(Held::default(), |view| view.held);
            let at = showing
                .as_ref()
                .map_or(0.0, |view| view.frames.timestamp.as_secs_f64());
            let anchor = SeamAnchor::hold(self.anchor, &reframe, held, at);
            self.anchor = Some(anchor);
            reframe.with_shift(anchor.shift())
        };
        let bound_frames = showing
            .as_ref()
            .filter(|view| self.is_bound(view))
            .map(|view| view.frames.clone());
        self.prepared_picture = if bind_map_picture {
            bound_frames
                .as_ref()
                .map(|frames| PreparedPicture::new(frames.stamp(), reframe, aspect))
        } else {
            None
        };
        queue.write_buffer(&self.uniforms, 0, reframe.bytes());
        // After the uniform write, because the band reads the same block: the
        // calibration it measures against has to be the one the draw will use,
        // or the two would disagree.
        if !selected_one_xs {
            self.measure(device, queue, showing.as_ref());
        }

        // The legacy optical-flow estimate and apply, behind the player's
        // runtime toggle (default OFF, [`ScenePrimitive`]'s `flow`). The
        // selected ONE X2 route is refused here: feeding that camera through
        // this legacy solver would substitute a different Studio mechanism.
        // ASYNCHRONOUS
        // (§35): the render thread never runs the DIS. Each frame it polls the
        // async strip readback and the background worker, uploads a finished
        // field, and — on Studio's ~30-frame cadence, only when nothing is in
        // flight — kicks the next estimate; the draw below picks the flow variant
        // and reads whatever field the worker last returned, HELD between updates
        // ([`Self::flow_step`]). The env path (`kjerag-spike --bin band`) uploads
        // its own field and leaves the toggle off, so the machinery is gated on
        // the toggle alone. The environment arm may select the legacy draw for
        // other routes, but never for selected ONE X2.
        if selected_one_xs {
            self.flow_step(device, queue, false, None);
            self.flow_draw = FlowDraw::Nothing;
        } else {
            let selected_camera = reframe.is_one_xs_pair();
            let player_flow = player_flow(primitive.flow, selected_camera);
            self.flow_step(device, queue, player_flow, showing.as_ref());
            self.flow_draw =
                FlowDraw::prepared(legacy_flow_draw(primitive.flow, flow_on(), selected_camera));
        }
        // A captured dense map is one prepared-frame instrument state. Any
        // ordinary prepare restores playback's own selection.
        self.map_oracle = None;

        // After the uniform write, and only after it: the write lands at the
        // next submit on this queue, and the capture's own submit is that
        // one. Taken here rather than in `draw` because this is the call
        // that has a device to render with.
        if !selected_one_xs && let Some(request) = primitive.shutter.take() {
            self.shoot(device, queue, request, aspect, showing.as_ref());
        }
        bound_frames
    }

    /// One frame's worth of the ASYNCHRONOUS optical-flow estimate and apply
    /// (§35), behind the player's runtime toggle. **The render thread never runs
    /// the DIS.** Called from [`Self::prepare`] after the band is measured.
    ///
    /// Per frame, in order:
    /// - **collect**: if the strip readback started earlier has landed (checked
    ///   with a non-blocking [`wgpu::PollType::Poll`], no stall), take its two
    ///   strips off the mapped buffer and hand them to the background worker;
    /// - **upload**: if the worker has returned a finished [`crate::flow::compose::Displacement`],
    ///   [`Band::upload_flow`] it so the draw applies it — this is the only flow
    ///   work on the render/queue thread, and it is a cheap `write_buffer`;
    /// - **kick**: tick Studio's cadence ([`Cadence`], period 30) once per new
    ///   frame, and only when it is due AND nothing is already in flight, START a
    ///   new async strip readback ([`Band::strips_kickoff`]) — never a blocking
    ///   `band_strips`.
    ///
    /// The draw reads whatever field was last uploaded, HELD between updates. Off,
    /// or on a frame with no bound seam pair, none of this runs and the flow
    /// buffer keeps its held (or zeroed) field.
    ///
    /// A toggle in either direction is an edge that clears the in-flight work and
    /// restarts the cadence, so a just-enabled seam re-estimates on its first
    /// frame ([`Cadence::start`]) rather than holding an empty field for a second.
    fn flow_step(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        on: bool,
        showing: Option<&View>,
    ) {
        // A toggle edge (either direction): clean slate. Drop the in-flight strip
        // readback, drain any stale field the worker already returned, and reset
        // the cadence so an off→on enable re-estimates at once.
        if on != self.flow.enabled_last {
            self.flow.cadence = Cadence::start();
            self.flow.readback = None;
            self.flow.estimating = false;
            self.flow.ticked_at = None;
            if let Some(worker) = &self.flow.worker {
                worker.drain();
            }
            self.flow.enabled_last = on;
        }
        if !on {
            return;
        }

        // collect: has the async strip readback landed? A non-blocking poll drives
        // the map callback; the render thread does not wait on it.
        if self.flow.readback.is_some() {
            let _ = device.poll(wgpu::PollType::Poll);
            match self.flow.readback.as_ref().and_then(FlowReadback::poll) {
                Some(Ok(())) => {
                    let readback = self.flow.readback.take().unwrap();
                    let (strip0, strip1) = readback.take_strips();
                    let worker = self.flow.worker.get_or_insert_with(FlowWorker::spawn);
                    self.flow.estimating = worker.submit(strip0, strip1);
                }
                // The map failed (a lost device): abandon this readback and let
                // the cadence start another. Nothing panics.
                Some(Err(e)) => {
                    eprintln!("kjerag: optical-flow strip readback failed: {e}");
                    self.flow.readback = None;
                }
                // Still mapping — hold the last field and try again next frame.
                None => {}
            }
        }

        // upload: a finished field from the worker. The only flow work on the
        // queue thread, and a cheap `write_buffer`. Held until the next one lands.
        if let Some(disp) = self.flow.worker.as_ref().and_then(FlowWorker::try_recv) {
            self.band.upload_flow(queue, &disp);
            self.flow.estimating = false;
            self.flow.uploaded += 1;
        }

        // kick: Studio's cadence, ticked once per NEW frame, and only starting an
        // estimate when the previous one has fully drained (readback done AND the
        // worker idle). A due frame that arrives with work still in flight simply
        // holds the last field — the compute-limited case, expected while the CPU
        // DIS costs more than one cadence period.
        let seam_bound = showing.is_some_and(|view| view.lenses.len() > 1 && self.is_bound(view));
        if seam_bound {
            let ts = showing.map(|view| view.frames.timestamp);
            let due = if self.flow.ticked_at != ts {
                self.flow.ticked_at = ts;
                self.flow.cadence.tick(true) == Estimate::Reestimate
            } else {
                false
            };
            if due && self.flow.readback.is_none() && !self.flow.estimating {
                self.flow.readback =
                    Some(self.band.strips_kickoff(device, queue, &self.bind_group));
            }
        }
    }

    /// How many finished optical-flow fields the worker has produced and this
    /// pipeline has uploaded, for a headless instrument to confirm the field
    /// updates on the cadence. Nothing in the player reads it.
    pub fn flow_uploads(&self) -> u64 {
        self.flow.uploaded
    }

    /// Draws the view a second time, offscreen, at the size the capture
    /// asked for. Everything after the submit is the worker thread's
    /// (`super::capture`).
    fn shoot(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        request: Request,
        aspect: f32,
        view: Option<&View>,
    ) {
        let at = view.map_or_else(Stamp::default, |view| Stamp {
            index: view.frames.index,
            time: view.frames.timestamp,
        });
        capture::deliver(
            self.expose(device, queue, request.width, aspect, at),
            request.then,
        );
    }

    /// The render-thread half: a target, one pass into it, and the copy that
    /// will be read back. The frame it samples is the one the bind group
    /// already points at, and `RETAINED` is what keeps that frame's decoder
    /// surfaces alive long enough for this pass to finish: three frames of
    /// slack against a pass that costs a few milliseconds.
    fn expose(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        aspect: f32,
        at: Stamp,
    ) -> Fallible<Pending> {
        let order = Order::of(self.format)?;
        let size = capture::fitted(width, aspect)?;
        let stride = capture::stride(size.width);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture"),
            size: size.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture"),
            size: u64::from(stride) * u64::from(size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let view = texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("capture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.draw(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(size.height),
                },
            },
            size.extent(),
        );

        Ok(Pending {
            device: device.clone(),
            _texture: texture,
            readback,
            submission: queue.submit([encoder.finish()]),
            size,
            stride,
            order,
            at,
        })
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        // Plain and both typed flow variants share the picture bindings. Only
        // flow draws bind the displacement buffer; their separate shader
        // pipelines keep the incompatible legacy and ONE X2 coordinate laws
        // from becoming a runtime reinterpretation of the same bytes.
        let (pipeline, read) = match self.flow_draw {
            FlowDraw::Nothing => return,
            FlowDraw::Plain => (&self.pipeline, &self.band.read),
            FlowDraw::Legacy => (&self.flow_pipeline, &self.band.flow_read),
            FlowDraw::OneXs => (&self.one_xs_flow_pipeline, &self.band.one_xs_flow_read),
            FlowDraw::DirectOneXs => {
                let draw = self
                    .direct_one_xs_map
                    .as_ref()
                    .expect("direct ONE X2 draw must own exact map resources");
                (&draw.pipeline, &draw.read)
            }
            FlowDraw::MapOracle => {
                let oracle = self
                    .map_oracle
                    .as_ref()
                    .expect("map-oracle selection must own its draw resources");
                (&oracle.pipeline, &oracle.read)
            }
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(1, read, &[]);
        pass.draw(0..3, 0..1);
    }

    fn is_bound(&self, view: &View) -> bool {
        self.live
            .front()
            .is_some_and(|live| Arc::ptr_eq(&live.frames, &view.frames))
    }

    /// Stop measuring the band, which leaves its state where it is and, on a
    /// pipeline that has never measured, leaves it at the zero that bends
    /// nothing: exactly the picture stage 1 drew.
    ///
    /// **Nothing in the player calls this** and no key reaches it (AGENTS.md,
    /// zero-config playback). It exists so `kjerag-spike --bin band` can draw
    /// the same frame both ways through the same pipeline, which is the only
    /// way a before and after differ by the band and by nothing else.
    pub fn hold_band(&mut self, held: bool) {
        self.band.held = held;
    }

    /// Stop pooling the exposure, which leaves the gain at exactly one and
    /// draws the picture stage 2 drew (issue #103, stage 3).
    ///
    /// The same instrument-only switch as [`Self::hold_band`] and for the same
    /// reason: a before and after have to differ by one thing. The band is
    /// still measured and the bend still applied, so what moves between the
    /// two renders is the exposure alone.
    pub fn hold_tone(&mut self, held: bool) {
        self.band.tone_held = held;
    }

    /// Dispatch the measurement this many times per redraw instead of once,
    /// so its cost can be read as a SLOPE (issue #103, stage 6).
    ///
    /// The same instrument-only switch as [`Self::hold_band`] and for a reason
    /// of the same kind. A redraw's wall time on a box with other work on it is
    /// the pass plus whatever else ran, and on this box that second term is
    /// wider than the first: six alternating runs of two builds under a load
    /// average of 21 came back 5.1 to 20.3 ms with the builds interleaved. The
    /// noise is ADDITIVE and the pass is not, so `n` dispatches of it minus one
    /// dispatch of it, over `n - 1`, is the pass with the box divided out.
    ///
    /// Nothing in the player calls this and no key reaches it.
    pub fn band_repeats(&mut self, times: u32) {
        self.band.repeats = times.max(1);
    }

    /// The band as it stands, for an instrument. `None` where the device
    /// cannot map a buffer back, which is not a case the player has.
    ///
    /// Nothing in the player calls this: the state's whole life is on the GPU
    /// and a readback would be a stall. It exists so `kjerag-spike --bin band`
    /// can print what the pass is drawing with.
    pub fn band_state(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Fallible<(band::Along, Vec<band::Cell>)> {
        let (_, along, cells, _) = self.band.read(device, queue)?;
        Ok((along, cells))
    }

    /// The solved chromatic field and the retained band metric, for an
    /// instrument. Same readback and the same caveat as [`Self::band_state`]:
    /// it stalls the queue, and no shipped path takes it.
    pub fn band_chroma(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Fallible<band::Field> {
        Ok(self.band.read(device, queue)?.3)
    }

    /// The pooled exposure the pass is drawing with, for an instrument
    /// (issue #103, stage 3). Same readback, same caveat: a stall, and no
    /// shipped path takes it.
    pub fn band_tone(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Fallible<band::Tone> {
        Ok(self.band.read(device, queue)?.0)
    }

    /// The two rectified seam line-image strips, lens 0 then lens 1, each
    /// [`band::STRIP_W`] × [`band::STRIP_H`] luma samples in row-major order,
    /// with a negative sentinel where that lens has no picture
    /// (docs/research/studio-seam-re.md §35).
    ///
    /// **An instrument, and dispatched only here.** It rectifies the pair the
    /// bind group currently points at — the last frame `prepare` bound — through
    /// the same calibration the picture is drawn with, in its own encoder, and
    /// never runs in [`Self::measure`] or the draw. So the shipped picture is
    /// untouched: nothing but this method reaches the `strip` entry point.
    ///
    /// It stalls the queue on the readback, like [`Self::band_state`], and no
    /// shipped path takes it.
    pub fn band_strips(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Fallible<(Vec<f32>, Vec<f32>)> {
        self.band.strips(device, queue, &self.bind_group)
    }

    /// Upload the composed per-lens flow displacement maps so the next draw
    /// applies them (chunk 4, §37).
    ///
    /// Only meaningful when the flow is compiled in (`KJERAG_FLOW` set): off,
    /// the buffer this writes is not bound into the draw, so the write is inert
    /// and the picture is the byte-identical shipped one. The instrument sets
    /// the knob, composes the field, and calls this before it draws.
    pub fn upload_flow(&self, queue: &wgpu::Queue, disp: &crate::flow::compose::Displacement) {
        self.band.upload_flow(queue, disp);
    }

    /// Upload native selected ONE X2 retained fields and make the next draw use
    /// their 1080-by-60 coordinate contract.
    ///
    /// This is a headless-oracle boundary, not player production wiring. The
    /// caller first prepares the exact media frame, then calls this method and
    /// draws without another [`Self::prepare`]. A later prepare deliberately
    /// restores the ordinary plain/legacy selection, so an external captured
    /// field cannot leak into playback state.
    pub fn upload_one_xs_flow(
        &mut self,
        queue: &wgpu::Queue,
        disp: &crate::flow::one_xs::Displacement,
    ) {
        self.band.upload_one_xs_flow(queue, disp);
        self.flow_draw = FlowDraw::OneXs;
    }

    /// Install one dense owner-view captured-map product for the next draw.
    ///
    /// The caller must first call [`Self::prepare`] for the decoded frame it
    /// wants to reuse, then upload a map rasterized for the target's exact
    /// extent, and draw without preparing again. This is instrument-only and
    /// deliberately has no `Scene` or playback selector.
    pub fn upload_map_oracle(&mut self, device: &wgpu::Device, map: &crate::map_oracle::DenseMap) {
        self.upload_dense_map(
            device,
            map,
            "owner-view captured map oracle",
            draw_wgsl_map_oracle(map.size.width),
        );
    }

    /// Submit the readable selected type-2 consumer for the exact picture most
    /// recently prepared into one complete origin-zero target.
    ///
    /// The raster must have been derived through [`crate::OneXsMapFrame`] for
    /// this exact preparation. The actual texture supplies extent, shape,
    /// format and usage; validation precedes every type-2 map-resource
    /// allocation. This method creates and submits the command buffer itself,
    /// so a caller cannot substitute a nonzero viewport/scissor or re-prepare
    /// the shared uniform before submission. It never changes the pipeline's
    /// ordinary draw state, and normal playback has no route that calls it.
    pub fn submit_one_xs_map(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::Texture,
        map: &OneXsMapRaster,
    ) -> Result<wgpu::SubmissionIndex, MapBindError> {
        MapBindError::require(map.prepared(), self.prepared_picture.as_ref())?;
        if map.uncovered() != 0 {
            return Err(MapBindError::Uncovered {
                count: map.uncovered(),
            });
        }
        MapBindError::require_target(map.size(), target, self.format)?;

        let draw = self.dense_map_draw(
            device,
            map.dense(),
            "frame-bound ONE X2 type-2 map",
            draw_wgsl_map_oracle(map.size().width),
        );
        let view = target.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("frame-bound ONE X2 type-2 map"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&draw.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(1, &draw.read, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        Ok(queue.submit([encoder.finish()]))
    }

    /// Submit the selected type-2 consumer directly from its native 200 by
    /// 100 resources. No output-sized dense map is constructed or uploaded.
    ///
    /// This is an instrument boundary only. It retains the exact delivered
    /// [`FrameStamp`] check, replaces the legacy band read group for this draw
    /// with the two native map buffers, and uses interpolated fullscreen UV so
    /// the result does not depend on an iced widget's framebuffer origin.
    /// Alternate target extents with the prepared aspect are valid because
    /// the native sphere map is independent of output resolution.
    pub fn submit_one_xs_map_direct(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::Texture,
        map: &crate::OneXsMapFrame,
    ) -> Result<wgpu::SubmissionIndex, MapBindError> {
        MapBindError::require_frame(map.frame(), self.prepared_picture.as_ref())?;
        let prepared = self
            .prepared_picture
            .as_ref()
            .expect("frame requirement established a prepared picture");
        MapBindError::require_direct_target(prepared, target, self.format)?;
        let draw = self
            .direct_one_xs_map
            .get_or_insert_with(|| DirectMapDraw::new(device, &self.layout, self.format));
        draw.upload(queue, map);
        debug_assert_eq!(draw.bound_frame(), Some(map.frame()));
        let view = target.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ONE X2 direct type-2 map"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&draw.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(1, &draw.read, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        Ok(queue.submit([encoder.finish()]))
    }

    /// Install one dense owner-view map while retaining Kjerag's normal
    /// decoded-source and colour path for the next draw.
    ///
    /// This differs from [`Self::upload_map_oracle`] only after the map has
    /// supplied the two source coordinates and colour weight: the ordinary
    /// `picture` function still performs Kjerag's NV12 sampling, range
    /// conversion and chromatic lookup. It is instrument-only and has no
    /// `Scene` or playback selector.
    pub fn upload_map_geometry_oracle(
        &mut self,
        device: &wgpu::Device,
        map: &crate::map_oracle::DenseMap,
    ) {
        self.upload_dense_map(
            device,
            map,
            "owner-view map geometry oracle",
            draw_wgsl_map_geometry_oracle(map.size.width),
        );
    }

    fn upload_dense_map(
        &mut self,
        device: &wgpu::Device,
        map: &crate::map_oracle::DenseMap,
        label: &'static str,
        shader: String,
    ) {
        self.map_oracle = Some(self.dense_map_draw(device, map, label, shader));
        self.flow_draw = FlowDraw::MapOracle;
    }

    fn dense_map_draw(
        &self,
        device: &wgpu::Device,
        map: &crate::map_oracle::DenseMap,
        label: &'static str,
        shader: String,
    ) -> MapOracleDraw {
        use wgpu::util::DeviceExt as _;

        let bytes = map.bytes();
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });
        let reading = read_layout(device, Some(bytes.len() as u64));
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[&self.layout, &reading],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let read = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &reading,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: band::STATE_BINDING,
                    resource: self.band.state.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.band.shown.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: FLOW_BINDING,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        MapOracleDraw {
            pipeline,
            _buffer: buffer,
            read,
        }
    }

    /// Imports a newly delivered pair and points the bind group at it. A
    /// redraw that shows the same pair again does nothing here.
    ///
    /// A failed import costs this frame and no more (issue #124). The next
    /// redraw tries again; what gives up is [`Stalled`], on a run of failures
    /// that lasts, and the pilot hears about it from the shell rather than
    /// from a terminal.
    ///
    /// Once it has given up, this stops trying, and that is not an
    /// optimisation. The view that failed is never bound, so every redraw
    /// after it would try the same import again, and each two seconds of that
    /// raised another alert: the owner met five of them in one sitting.
    fn show(&mut self, device: &wgpu::Device, view: &View, primitive: &ScenePrimitive) {
        if primitive.stalled.stopped() || self.is_bound(view) {
            return;
        }
        match self.import(device, view) {
            Ok(planes) => {
                primitive.stalled.landed();
                self.bind_group = bind(
                    device,
                    &self.layout,
                    &self.uniforms,
                    // A file with one lens stream leaves the second slot on
                    // the blank pixel. Nothing samples it: `Reframe`'s lens
                    // count says one, and a bind group still needs an entry
                    // for every binding the layout declares.
                    std::array::from_fn(|lens| planes.get(lens).unwrap_or(&self.blank)),
                    &self.sampler,
                );
                self.live.push_front(Live {
                    frames: view.frames.clone(),
                    planes,
                });
                self.live.truncate(RETAINED);
                // Selected ONE X2 is not "shown" merely because its source
                // imported. The exact map upload below is the presentation
                // boundary; recording it here would let an unstitched View
                // replace the last successfully presented one.
                if !ONE_XS_PLAYBACK_ENABLED || view.one_xs.is_none() {
                    primitive.shown.keep(view);
                }
            }
            Err(e) => {
                primitive
                    .stalled
                    .failed(Instant::now(), e, primitive.shown.get().is_some());
            }
        }
    }

    /// Every lens of the pair: both are sampled, one per output pixel.
    fn import(&self, device: &wgpu::Device, view: &View) -> Fallible<Vec<Planes>> {
        view.frames
            .lenses
            .iter()
            .map(|frame| dmabuf::import(device, frame.descriptor(), view.frames.size))
            .collect()
    }
}

impl Band {
    fn new(device: &wgpu::Device, scene: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("band"),
            // The same map the draw runs, so the band correlates directions
            // through the calibration the picture is drawn with rather than
            // through a second copy of it.
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}\n{}",
                    projection::wgsl(),
                    band::wgsl(),
                    chroma::wgsl(),
                )
                .into(),
            ),
        });
        let layout = band_layout(device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("band"),
            bind_group_layouts: &[scene, &layout],
            immediate_size: 0,
        });
        let reading = read_layout(device, None);
        let reading_flow = read_layout(device, Some(band::FLOW_BYTES));
        let reading_one_xs = read_layout(
            device,
            Some(crate::flow::one_xs::Displacement::BYTES as u64),
        );
        let compute = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("band"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let pipeline = compute("measure");
        let pool = compute("pool");
        let pool_along = compute("pool_along");
        // Studio's grid: read the evidence band, admit and weigh it, solve the
        // 5,088-node system, then blur, ramp and ease what it produced.
        let grid_gate = compute("chroma_gate");
        let grid_read = compute("chroma_read");
        let grid_blur = compute("chroma_blur");
        let grid_admit = compute("chroma_admit");
        let grid_solve = compute("chroma_solve");
        let grid_finish = compute("chroma_finish");
        // Studio's line-image strip, an instrument on its own entry point.
        let strip = compute("strip");
        let state = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("band"),
            size: band::BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            // Zeroed, and zero is the state that bends nothing: a file's first
            // frame is drawn exactly as stage 1 drew it.
            mapped_at_creation: false,
        });
        let buffer = |label: &str, size: u64| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                // Zeroed, and zero is the field that corrects nothing: a
                // file's first frame draws exactly as it drew before this arm.
                mapped_at_creation: false,
            })
        };
        let evidence = buffer("chroma evidence", chroma::EVIDENCE_BYTES);
        let field = buffer("chroma field", chroma::FIELD_BYTES);
        let work = buffer("chroma work", chroma::WORK_BYTES);
        let control = buffer("chroma control", chroma::CONTROL_BYTES);
        let shown = buffer("chroma shown", chroma::FIELD_BYTES);
        // The strips buffer: STORAGE for the pass to write, COPY_SRC for the
        // instrument to read back. Nothing shipped reads or dispatches it.
        let strips = buffer("band strips", band::STRIP_BYTES);
        // The composed flow displacement, uploaded from the CPU (COPY_DST) and
        // read by the draw (STORAGE). Zeroed, which displaces nothing.
        let flow = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("band flow"),
            size: band::FLOW_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let one_xs_flow = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 flow oracle"),
            size: crate::flow::one_xs::Displacement::BYTES as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let watch = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("band"),
            size: std::mem::size_of::<band::Watch>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("band"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: band::STATE_BINDING,
                    resource: state.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: band::WATCH_BINDING,
                    resource: watch.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: evidence.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: field.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: work.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: control.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: shown.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: band::STRIP_BINDING,
                    resource: strips.as_entire_binding(),
                },
            ],
        });
        // Three read groups over the shared band/chromatic state: the plain one
        // the shipped draw takes, the legacy-flow one with the estimator's
        // displacement, and the instrument-only ONE X2 one with its own typed
        // displacement allocation.
        let read = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("band read"),
            layout: &reading,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: band::STATE_BINDING,
                    resource: state.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: shown.as_entire_binding(),
                },
            ],
        });
        let flow_read = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("band read flow"),
            layout: &reading_flow,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: band::STATE_BINDING,
                    resource: state.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: shown.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: FLOW_BINDING,
                    resource: flow.as_entire_binding(),
                },
            ],
        });
        let one_xs_flow_read = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("band read ONE X2 flow"),
            layout: &reading_one_xs,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: band::STATE_BINDING,
                    resource: state.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: shown.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: FLOW_BINDING,
                    resource: one_xs_flow.as_entire_binding(),
                },
            ],
        });
        Self {
            pipeline,
            pool,
            pool_along,
            control,
            shown,
            grid_gate,
            grid_read,
            grid_blur,
            grid_admit,
            grid_solve,
            grid_finish,
            strip,
            strips,
            flow,
            one_xs_flow,
            state,
            watch,
            group,
            read,
            flow_read,
            one_xs_flow_read,
            held: false,
            tone_held: false,
            repeats: 1,
            slice: 0,
            at: None,
        }
    }

    /// How much media time the state has aged by, or `None` for a frame it has
    /// already read.
    ///
    /// Media time and not wall clock, so a paused window does not age the
    /// state, a slow box does not smooth harder than a fast one, and the same
    /// second of film settles the same way at 24 fps and at 60. A gap that is
    /// not a play forward is a reset: what the state holds is an average over
    /// what the seam has been showing, and after a seek that is somewhere
    /// else.
    fn aged(&mut self, now: Duration) -> Option<band::Watch> {
        let before = self.at.replace(now);
        let seconds = before.map(|then| now.as_secs_f32() - then.as_secs_f32());
        let slice = self.slice;
        self.slice = (slice + 1) % band::ROUNDS;
        match seconds {
            Some(0.0) => None,
            Some(seconds) if (0.0..band::Watch::GAP_S).contains(&seconds) => {
                Some(band::Watch::track(seconds, slice))
            }
            // The first frame of a file, and every landing after a seek. The
            // step it is given is one frame's worth, so a direction with
            // content in it starts moving immediately rather than waiting a
            // frame for a gap to exist, and it sweeps the WHOLE ring rather
            // than the slice it happened to land on, because what it is
            // throwing away is per direction (`band::Watch::stride`).
            _ => Some(band::Watch::start(1.0 / 30.0)),
        }
    }

    /// The state copied back to the CPU. For instruments only: see
    /// [`ScenePipeline::band_state`].
    fn read(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Fallible<BandState> {
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("band"),
            size: band::BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&self.state, 0, &readback, 0, band::BYTES);
        let submission = queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        let mapped = slice.get_mapped_range();
        let float = |at: usize| {
            f32::from_ne_bytes([mapped[at], mapped[at + 1], mapped[at + 2], mapped[at + 3]])
        };
        let tone = band::Tone::read(float(0), float(4));
        let along = band::Along::read(
            std::array::from_fn(|term| float(band::ALONG_AT + 4 * term)),
            float(band::ALONG_AT + 20),
        );
        let cells = (0..band::AZIMUTHS)
            .map(|index| {
                let at = band::CELLS_AT + index * std::mem::size_of::<band::Cell>();
                band::Cell {
                    disparity: float(at),
                    confidence: float(at + 4),
                    reach_m: float(at + 8),
                    off_epi: float(at + 12),
                    off_conf: float(at + 16),
                    tone: float(at + 20),
                    lit: float(at + 24),
                    trust: float(at + 28),
                }
            })
            .collect();
        // The solved chromatic field, and the metric that set the solve's
        // weight scale and its budget. Read out because an arm nobody can see
        // the numbers of is an arm that gets reasoned about instead of
        // measured, which is how it shipped painting the whole sphere.
        // The grid's control block, on its own small readback.
        let gate = {
            let back = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("chroma control"),
                size: chroma::CONTROL_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&self.control, 0, &back, 0, chroma::CONTROL_BYTES);
            let done = queue.submit([encoder.finish()]);
            let slice = back.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::Wait {
                submission_index: Some(done),
                timeout: None,
            })?;
            let held = slice.get_mapped_range();
            let at = |i: usize| {
                f32::from_ne_bytes([
                    held[4 * i],
                    held[4 * i + 1],
                    held[4 * i + 2],
                    held[4 * i + 3],
                ])
            };
            // `by_row` is declared before `baseline`, so it starts right after
            // the eight scalars. Reading it past the baselines returned the
            // gate's own strip means as residuals - negative lengths and
            // 150-code values, which is what caught it.
            let rows: Vec<f32> = (0..chroma::WINDOW_ROWS).map(|row| at(8 + row)).collect();
            let out = (at(3), at(4), at(5), at(0), at(6), at(7), rows);
            drop(held);
            back.unmap();
            out
        };
        // The grid's applied field, one [R, G, B] per node.
        let grid = {
            let back = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("chroma shown"),
                size: chroma::FIELD_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&self.shown, 0, &back, 0, chroma::FIELD_BYTES);
            let done = queue.submit([encoder.finish()]);
            let slice = back.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::Wait {
                submission_index: Some(done),
                timeout: None,
            })?;
            let held = slice.get_mapped_range();
            let at = |i: usize| {
                f32::from_ne_bytes([
                    held[4 * i],
                    held[4 * i + 1],
                    held[4 * i + 2],
                    held[4 * i + 3],
                ])
            };
            let out: Vec<[f32; 3]> = (0..chroma::NODES)
                .map(|node| std::array::from_fn(|channel| at(channel * chroma::NODES + node)))
                .collect();
            drop(held);
            back.unmap();
            out
        };
        let field = band::Field {
            // The APPLIED field, which is the eased one: what the picture took
            // rather than what the solve last produced. An instrument that
            // reported the raw iterate would report a flicker the draw never
            // showed, and miss one it did.
            // The GRID's applied field, out of the grid's own buffer. This
            // used to walk the RING's arrays and report them as the
            // correction, which is an instrument lying about the one thing it
            // exists to watch - and it was still doing it while the grid was
            // already drawing.
            per_direction: grid,
            level: gate.3,
            triggers: gate.0,
            frames: gate.1,
            motion: gate.2,
            step: gate.4,
            residual: gate.5,
            by_row: gate.6,
        };
        drop(mapped);
        readback.unmap();
        Ok((tone, along, cells, field))
    }

    /// Dispatch the strip pass once over the pair `scene` points at, then read
    /// the two strips back. For instruments only: see
    /// [`ScenePipeline::band_strips`].
    ///
    /// One dispatch, one submit, one map: it runs in an encoder of its own, so
    /// nothing about the measurement or the draw is on the same submit and the
    /// shipped path is not so much as reordered by it.
    /// Write the composed flow displacement into the buffer the draw's read
    /// group binds (chunk 4, §37). `write_buffer` and no submit of its own: the
    /// upload rides the next frame's queue.
    fn upload_flow(&self, queue: &wgpu::Queue, disp: &crate::flow::compose::Displacement) {
        queue.write_buffer(&self.flow, 0, disp.bytes());
    }

    /// Write the selected native ONE X2 retained-field layout into its exact
    /// dedicated allocation. [`ScenePipeline::upload_one_xs_flow`] switches
    /// the matching buffer and shader together as one instrument operation.
    fn upload_one_xs_flow(&self, queue: &wgpu::Queue, disp: &crate::flow::one_xs::Displacement) {
        debug_assert_eq!(disp.bytes().len(), crate::flow::one_xs::Displacement::BYTES);
        queue.write_buffer(&self.one_xs_flow, 0, disp.bytes());
    }

    fn strips(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &wgpu::BindGroup,
    ) -> Fallible<(Vec<f32>, Vec<f32>)> {
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("band strip"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.strip);
            pass.set_bind_group(0, scene, &[]);
            pass.set_bind_group(1, &self.group, &[]);
            pass.dispatch_workgroups(band::STRIP_GROUPS, 1, 1);
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("band strip"),
            size: band::STRIP_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&self.strips, 0, &readback, 0, band::STRIP_BYTES);
        let submission = queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        let mapped = slice.get_mapped_range();
        let samples: Vec<f32> = mapped
            .chunks_exact(4)
            .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        drop(mapped);
        readback.unmap();
        let per_strip = band::STRIP_W * band::STRIP_H;
        Ok((
            samples[..per_strip].to_vec(),
            samples[per_strip..2 * per_strip].to_vec(),
        ))
    }

    /// Dispatch the strip pass over the pair `scene` points at and START an
    /// asynchronous readback of the result: submit the copy, then `map_async`
    /// **without waiting**. The player's [`ScenePipeline::flow_step`] polls the
    /// returned [`FlowReadback`] on later frames (a non-blocking device poll) and
    /// pulls the strips off once the map lands, so the render thread never stalls
    /// on the GPU→CPU readback — the one difference from the synchronous
    /// [`Self::strips`] instrument, which is otherwise the same dispatch.
    fn strips_kickoff(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &wgpu::BindGroup,
    ) -> FlowReadback {
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("band strip async"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.strip);
            pass.set_bind_group(0, scene, &[]);
            pass.set_bind_group(1, &self.group, &[]);
            pass.dispatch_workgroups(band::STRIP_GROUPS, 1, 1);
        }
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("band strip async"),
            size: band::STRIP_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&self.strips, 0, &buffer, 0, band::STRIP_BYTES);
        queue.submit([encoder.finish()]);
        // The callback fires from a device poll (the non-blocking `Poll` in
        // `flow_step`), not from a `Wait`, so nothing here blocks. The channel
        // carries the map result to the render thread.
        let (done_tx, done) = mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = done_tx.send(result);
            });
        FlowReadback {
            buffer,
            done: Mutex::new(done),
        }
    }
}

/// The player's asynchronous optical-flow state (§35): Studio's cadence, the
/// background DIS worker, the in-flight strip readback, and the little phase the
/// render thread keeps so it ticks the cadence once per new frame and never
/// blocks. Default: idle — no worker thread until flow is first used, no field
/// ever produced, so flow-OFF costs nothing.
struct Flow {
    cadence: Cadence,
    /// The DIS worker, spawned the first time a strip readback lands and kept for
    /// the pipeline's life. `None` until then, so a player that never turns flow
    /// on never spawns a thread.
    worker: Option<FlowWorker>,
    /// The strip readback started on the last due frame, awaiting its map. `None`
    /// when nothing is in flight.
    readback: Option<FlowReadback>,
    /// Whether a job is currently with the worker — sent, result not yet taken.
    /// Gates the cadence so only one estimate is ever in flight.
    estimating: bool,
    /// The toggle's value last frame, to spot a toggle edge in either direction.
    enabled_last: bool,
    /// The frame timestamp the cadence last ticked on, so a redraw of the same
    /// frame does not advance Studio's period — the cadence counts frames.
    ticked_at: Option<Duration>,
    /// How many finished fields the worker has produced and the pipeline has
    /// uploaded, for the headless instrument ([`ScenePipeline::flow_uploads`]).
    uploaded: u64,
}

impl Flow {
    fn new() -> Self {
        Self {
            cadence: Cadence::start(),
            worker: None,
            readback: None,
            estimating: false,
            enabled_last: false,
            ticked_at: None,
            uploaded: 0,
        }
    }
}

/// One in-flight strip readback: the `MAP_READ` buffer and the channel the
/// `map_async` callback signals through. Dropping it before the map lands is
/// safe — destroying the buffer cancels the pending map and the callback's send
/// fails silently.
///
/// The `Mutex` around the receiver is only to make the pipeline `Sync` (iced's
/// `shader::Pipeline` requires it); the channel is touched from the render thread
/// alone, so the lock is always uncontended and never poisoned.
struct FlowReadback {
    buffer: wgpu::Buffer,
    done: Mutex<mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
}

impl FlowReadback {
    /// The map's outcome, or `None` while it is still pending: `Some(Ok)` mapped
    /// and ready to read, `Some(Err)` failed (a lost device).
    fn poll(&self) -> Option<Result<(), wgpu::BufferAsyncError>> {
        self.done.lock().ok()?.try_recv().ok()
    }

    /// The two strips off the mapped buffer, lens 0 then lens 1. Called once the
    /// map has landed; unmaps and lets the buffer drop after. Same read as the
    /// synchronous [`Band::strips`], minus the wait.
    fn take_strips(self) -> (Vec<f32>, Vec<f32>) {
        let slice = self.buffer.slice(..);
        let mapped = slice.get_mapped_range();
        let samples: Vec<f32> = mapped
            .chunks_exact(4)
            .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        drop(mapped);
        self.buffer.unmap();
        let per = band::STRIP_W * band::STRIP_H;
        (samples[..per].to_vec(), samples[per..2 * per].to_vec())
    }
}

/// The background thread that runs the CPU DIS. Given the two strips it runs
/// [`estimate_flow`] (mask + two DIS fields + compose) and returns the
/// [`crate::flow::compose::Displacement`], never touching the GPU. One job is
/// ever in flight ([`Flow::estimating`] gates it), so the unbounded channels
/// never back up. Dropped with the pipeline: the job sender is closed and the
/// thread joined, so nothing leaks and nothing deadlocks.
///
/// The two channels are behind `Mutex` for the same reason as [`FlowReadback`]'s:
/// to keep the pipeline `Sync`. Only the render thread touches them, so the locks
/// are uncontended.
struct FlowWorker {
    /// The job channel, in an `Option` so [`Drop`] can close it (drop the sender)
    /// before joining — the worker's blocking `recv` returns only once it is gone.
    jobs: Mutex<Option<mpsc::Sender<Strips>>>,
    /// Finished fields back from the worker.
    done: Mutex<mpsc::Receiver<crate::flow::compose::Displacement>>,
    handle: Option<JoinHandle<()>>,
}

/// The two rectified seam strips the render thread hands the worker, lens 0 then
/// lens 1, each [`band::STRIP_W`] × [`band::STRIP_H`] luma samples.
type Strips = (Vec<f32>, Vec<f32>);

impl FlowWorker {
    fn spawn() -> Self {
        let (jobs_tx, jobs_rx) = mpsc::channel::<Strips>();
        let (done_tx, done) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("kjerag-flow".to_owned())
            .spawn(move || {
                // Blocks until a job arrives and exits when the sender is dropped
                // (pipeline drop). One estimate at a time; a failed `send` means
                // the pipeline is gone, so stop.
                while let Ok((strip0, strip1)) = jobs_rx.recv() {
                    let disp = estimate_flow(&strip0, &strip1);
                    if done_tx.send(disp).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn the optical-flow worker thread");
        Self {
            jobs: Mutex::new(Some(jobs_tx)),
            done: Mutex::new(done),
            handle: Some(handle),
        }
    }

    /// Hand the worker a pair of strips to estimate. `false` only if the worker
    /// is gone (after a drop), which the caller reads as "nothing in flight".
    fn submit(&self, strip0: Vec<f32>, strip1: Vec<f32>) -> bool {
        match self.jobs.lock() {
            Ok(jobs) => jobs
                .as_ref()
                .is_some_and(|tx| tx.send((strip0, strip1)).is_ok()),
            Err(_) => false,
        }
    }

    /// A finished field, if the worker has one ready.
    fn try_recv(&self) -> Option<crate::flow::compose::Displacement> {
        self.done.lock().ok()?.try_recv().ok()
    }

    /// Discard any field the worker has already returned — a toggle edge's clean
    /// slate, so a stale field is never taken as the first one after an enable.
    fn drain(&self) {
        if let Ok(done) = self.done.lock() {
            while done.try_recv().is_ok() {}
        }
    }
}

impl Drop for FlowWorker {
    fn drop(&mut self) {
        // Close the job channel FIRST so the worker's `recv` returns and its loop
        // exits, THEN join. Struct fields drop after this body runs, so clearing
        // the sender here is what lets the join finish rather than deadlock against
        // a still-open channel. The join waits at most one in-progress estimate.
        if let Ok(jobs) = self.jobs.get_mut() {
            *jobs = None;
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// The pure-CPU optical-flow estimate the worker runs (§35/§59/§62): Studio's
/// per-lens coverage masks off the two rectified strips, the two separately-
/// estimated DIS fields through the faithful [`crate::flow::dis::DisFlow`], and
/// the composed [`crate::flow::compose::Displacement`] the draw applies. **No
/// GPU.** This is the sequence that used to run inline on the render thread and
/// the same one `kjerag-spike --bin band mode=flowrender` runs (§93); moving it
/// here, verbatim, is the whole of the async change on the estimate side.
fn estimate_flow(strip0: &[f32], strip1: &[f32]) -> crate::flow::compose::Displacement {
    // Studio's per-lens COVERAGE MASK (§59/§61/§62): a belt pixel is valid for a
    // lens iff that lens has real content there (the strip's own −1 no-picture
    // sentinel) AND it lies within the reliable seam band. The clip is the
    // faithful ONE-SIDED erode (§62's `erodeBeltMasksUsingFisheyeMask` 96°): lens
    // 0's far (high-θ) edge is clipped inward by `MASK_HALF_DEG`, lens 1's low-θ
    // edge the same, each near side kept full.
    let theta_of = |row: usize| row as f32 * 180.0 / (band::STRIP_H as f32 - 1.0);
    let mut mask0 = vec![false; band::STRIP_W * band::STRIP_H];
    let mut mask1 = vec![false; band::STRIP_W * band::STRIP_H];
    for row in 0..band::STRIP_H {
        let off = theta_of(row) - 90.0;
        let in0 = off <= MASK_HALF_DEG; // lens 0: clip its high-θ far edge
        let in1 = off >= -MASK_HALF_DEG; // lens 1: clip its low-θ far edge
        for col in 0..band::STRIP_W {
            let i = row * band::STRIP_W + col;
            mask0[i] = in0 && strip0[i] >= 0.0;
            mask1[i] = in1 && strip1[i] >= 0.0;
        }
    }
    // The RE'd FDSFlow parameter block (§44.3): `DisConfig::default` is
    // `finest_scale = 1` and the §47 reliability gate on. Two separately-estimated
    // fields, the belt images swapped (§38): l2r = DIS(lens0, lens1) is lens 1's
    // field, r2l the reverse; `IsInMask` gates each on both lens masks.
    let dis = crate::flow::dis::DisFlow::new(crate::flow::dis::DisConfig::default());
    let l2r = dis.calc(
        strip0,
        strip1,
        band::STRIP_W,
        band::STRIP_H,
        None,
        Some((&mask0, &mask1)),
    );
    let r2l = dis.calc(
        strip1,
        strip0,
        band::STRIP_W,
        band::STRIP_H,
        None,
        Some((&mask1, &mask0)),
    );
    crate::flow::compose::Displacement::compose(&l2r, &r2l)
}

/// The uniform block, then each lens's luma and chroma planes in lens order,
/// then the sampler they share. The shader names those textures rather than
/// indexing them, so the count is [`MAX_LENSES`] on both sides.
fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    lenses: [&Planes; MAX_LENSES],
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    // The views have to outlive the descriptor that borrows them, so they are
    // built before it rather than inside it.
    let views: Vec<wgpu::TextureView> = lenses
        .iter()
        .flat_map(|planes| [&planes.luma, &planes.chroma])
        .map(|texture| texture.create_view(&Default::default()))
        .collect();
    let mut entries = vec![wgpu::BindGroupEntry {
        binding: 0,
        resource: uniforms.as_entire_binding(),
    }];
    entries.extend(
        views
            .iter()
            .enumerate()
            .map(|(plane, view)| wgpu::BindGroupEntry {
                binding: 1 + plane as u32,
                resource: wgpu::BindingResource::TextureView(view),
            }),
    );
    entries.push(wgpu::BindGroupEntry {
        binding: SAMPLER_BINDING,
        resource: wgpu::BindingResource::Sampler(sampler),
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene"),
        layout,
        entries: &entries,
    })
}

fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    // The uniform block and the pictures are read by both passes: the draw
    // samples them and the band correlates them (issue #103). The state buffer
    // at the end is the draw's alone here, read-only; the compute pass reaches
    // the same buffer through a group of its own, where it is writable.
    let both = wgpu::ShaderStages::FRAGMENT.union(wgpu::ShaderStages::COMPUTE);
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: both,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: both,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            // The one place the Rust and WGSL definitions of the uniform
            // block are checked against each other: pipeline creation fails
            // if the shader's struct wants more bytes than `Reframe` has.
            min_binding_size: NonZeroU64::new(std::mem::size_of::<Reframe>() as u64),
        },
        count: None,
    }];
    entries.extend((1..SAMPLER_BINDING).map(texture));
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: SAMPLER_BINDING,
        visibility: both,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    });
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("scene"),
        entries: &entries,
    })
}

/// The state buffer alone, as the draw sees it: read-only, on a group of its
/// own (see [`band::STATE_BINDING`]).
fn read_layout(device: &wgpu::Device, flow_bytes: Option<u64>) -> wgpu::BindGroupLayout {
    // The grid rides in this group and not one of its own: the app's device
    // reports max_bind_groups = 2, so zero and one are the whole budget. The
    // flow displacement rides here too, on binding 3, and only in a typed flow
    // layout built with that payload's exact size, so the plain layout the
    // shipped draw uses is untouched.
    let mut entries = vec![
        wgpu::BindGroupLayoutEntry {
            binding: band::STATE_BINDING,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(band::BYTES),
            },
            count: None,
        },
        chroma::read_entry(),
    ];
    if let Some(flow_bytes) = flow_bytes {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: FLOW_BINDING,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                // Each flow pipeline and bind group declare the lower bound of
                // their own payload contract. The selected ONE X2 allocation
                // can therefore be exact-sized without weakening the legacy
                // layout's validation.
                min_binding_size: NonZeroU64::new(flow_bytes),
            },
            count: None,
        });
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("band read"),
        entries: &entries,
    })
}

/// What the band writes and what it is told about this frame. A group of its
/// own so the same buffer can be read-only on the draw's side.
fn band_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("band"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: band::STATE_BINDING,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(band::BYTES),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: band::WATCH_BINDING,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<band::Watch>() as u64),
                },
                count: None,
            },
            chroma::entries()[0],
            chroma::entries()[1],
            chroma::entries()[2],
            chroma::entries()[3],
            chroma::entries()[4],
            // The strips buffer, on binding 7. Declared for every pipeline this
            // layout serves, but named only by the `strip` entry point: wgpu
            // validates the bindings a shader USES, so `measure`, `pool` and the
            // grid are unaffected by its presence.
            wgpu::BindGroupLayoutEntry {
                binding: band::STRIP_BINDING,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(band::STRIP_BYTES),
                },
                count: None,
            },
        ],
    })
}

/// The shader samples its planes unconditionally, so something has to be bound
/// before a file is. One black pixel each.
fn blank_planes(device: &wgpu::Device) -> Planes {
    let make = |format| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blank"),
            size: Size::new(1, 1).extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };
    Planes {
        luma: make(wgpu::TextureFormat::R8Unorm),
        chroma: make(wgpu::TextureFormat::Rg8Unorm),
    }
}

/// The draw's whole module, with the flow apply forced on or off. In dependency
/// order so nothing is used before it is declared: `projection::wgsl` declares
/// the uniform block, the view ray and the forward map; then the band's lookup
/// into it, the sampling, and this file's own entry points. The pipeline builds
/// it both ways ([`ScenePipeline::new`]) and the runtime toggle picks between
/// the two.
///
/// A function rather than an expression inside the pipeline, so a test can
/// compile it without a pipeline (the twin does, both ways): this is the module
/// the chromatic lookup lands in, and nothing reached it until one did.
///
/// **Off is byte-for-byte the shipped shader** (chunk 4, §37): the string is
/// returned unchanged, so the rendered picture is identical to the pass before
/// the flow existed. Only on does the fragment recompute each covered lens's
/// sample landing at its flow-displaced ray and the flow buffer get declared.
/// See [`flow_on`], [`flow_wgsl`].
pub(crate) fn draw_wgsl_flow(flow: bool) -> String {
    let core = draw_wgsl_core();
    if !flow {
        return core;
    }
    let injected = inject_flow_blend(core);
    format!("{injected}\n{}", flow_wgsl())
}

/// The selected native ONE X2 retained-field draw used by the detached V6
/// oracle. It is a separate shader module rather than a runtime branch over an
/// untyped buffer: the legacy and selected layouts disagree about dimensions,
/// axis order and the coordinate law that turns a grid sample back into a ray.
pub(crate) fn draw_wgsl_one_xs_flow() -> String {
    let injected = inject_flow_blend(draw_wgsl_core());
    format!("{injected}\n{}", one_xs_flow_wgsl())
}

/// Owner-view captured-map consumer. Geometry and map interpolation are
/// precomputed by the readable CPU oracle; this fragment performs the READ
/// selected aggregate-atlas, box filter and YUV-matrix source route.
fn draw_wgsl_map_oracle(width: u32) -> String {
    const PICTURE_CALL: &str = "  let lens = picture(mix, ratio, look.xyz);";
    const ORACLE_CALL: &str = "  let lens = oracle_picture(in.pos.xy);";
    let core = draw_wgsl_core();
    let injected = core.replace(PICTURE_CALL, ORACLE_CALL);
    assert_ne!(injected, core, "the map-oracle picture anchor moved");
    format!("{injected}\n{}", map_oracle_wgsl(width))
}

/// Dense recovered map geometry followed by Kjerag's ordinary source and
/// colour consumer. The only replacement in the core is the source-coordinate
/// producer that normally calls `blend`.
fn draw_wgsl_map_geometry_oracle(width: u32) -> String {
    const GEOMETRY_CALL: &str =
        "  if look.w > 0.0 {\n    mix = geometry_oracle_blend(in.pos.xy);\n  }";
    let core = draw_wgsl_core();
    let injected = core.replace(FS_BLEND, GEOMETRY_CALL);
    assert_ne!(injected, core, "the map-geometry blend anchor moved");
    format!("{injected}\n{}", map_geometry_oracle_wgsl(width))
}

fn map_geometry_oracle_wgsl(width: u32) -> String {
    format!(
        r#"@group(1) @binding({binding}) var<storage, read> geometry_map: array<f32>;
const GEOMETRY_WIDTH = {width}u;
const GEOMETRY_STRIDE = 8u;

fn geometry_oracle_blend(position: vec2<f32>) -> Blend {{
  let pixel = u32(position.y) * GEOMETRY_WIDTH + u32(position.x);
  let at = pixel * GEOMETRY_STRIDE;
  let covered = geometry_map[at + 5u];
  var out: Blend;
  if covered <= 0.5 {{ return out; }}

  // The recovered packed map stores lens A in the left half and lens B in
  // the right half of one virtual atlas. Convert each back to its local
  // normalized source coordinate, then to the pixel-centre convention that
  // Kjerag's ordinary `frame_uv` reverses inside `picture`.
  let local_a = vec2<f32>(2.0 * geometry_map[at], geometry_map[at + 1u]);
  let local_b = vec2<f32>(2.0 * geometry_map[at + 2u] - 1.0, geometry_map[at + 3u]);
  let frame = vec2<f32>(reframe.frame_width, reframe.frame_height);
  out.landings[0].pixel = local_a * frame - vec2<f32>(0.5);
  out.landings[1].pixel = local_b * frame - vec2<f32>(0.5);
  out.landings[0].inside = true;
  out.landings[1].inside = true;
  out.weights[0] = geometry_map[at + 4u];
  out.weights[1] = 1.0 - out.weights[0];
  return out;
}}
"#,
        binding = FLOW_BINDING,
    )
}

fn map_oracle_wgsl(width: u32) -> String {
    format!(
        r#"@group(1) @binding({binding}) var<storage, read> oracle_map: array<f32>;
const ORACLE_WIDTH = {width}u;
const ORACLE_STRIDE = 8u;

// Exact captured TextureParam bit anchors. The compiled selected helper uses
// 1.1 (0x3f8ccccd), not the 2.0 found in an unselected bundled generic shader.
const ORACLE_BOX_FAST: f32 = 1.1000000238418579;
const ORACLE_BOX_SIZE: f32 = 1.7881766557693481;
const ORACLE_BOX_FALLBACK_AREA: f32 = 0.0010000000474974513;
const ORACLE_CHROMA_OFFSET: f32 = 0.50196081399917603;
const ORACLE_R_CR: f32 = 1.4019999504089355;
const ORACLE_G_CB: f32 = -0.34400001168251038;
const ORACLE_G_CR: f32 = -0.71399998664855957;
const ORACLE_B_CB: f32 = 1.7719999551773071;

// Aggregate double-fisheye atlas tap routing over Kjerag's two decoded
// textures. The selected sampler is normalized, linear, clamp-to-edge, with
// no mip level. Clamp every tap against the full virtual atlas before routing
// it: a footprint crossing the internal split reads both physical lenses.
fn atlas_load(a: texture_2d<f32>, b: texture_2d<f32>, p: vec2<i32>) -> vec4<f32> {{
  let dims = textureDimensions(a);
  let x = clamp(p.x, 0, i32(2u * dims.x) - 1);
  let y = clamp(p.y, 0, i32(dims.y) - 1);
  if x < i32(dims.x) {{
    return textureLoad(a, vec2<i32>(x, y), 0);
  }}
  return textureLoad(b, vec2<i32>(x - i32(dims.x), y), 0);
}}

fn atlas_linear(a: texture_2d<f32>, b: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {{
  let dims = textureDimensions(a);
  let p = uv * vec2<f32>(f32(2u * dims.x), f32(dims.y)) - vec2<f32>(0.5);
  let base = vec2<i32>(floor(p));
  let f = fract(p);
  let top = mix(atlas_load(a, b, base), atlas_load(a, b, base + vec2<i32>(1, 0)), f.x);
  let bottom = mix(atlas_load(a, b, base + vec2<i32>(0, 1)), atlas_load(a, b, base + vec2<i32>(1, 1)), f.x);
  return mix(top, bottom, f.y);
}}

// Selected boxSampling: both-axis fast gate, otherwise a row-major walk from
// floor(start) in two-texel steps. Every cell is clipped to the box and sampled
// at its clipped midpoint, then area weighted. The final direct sample is only
// the READ <= 0.001-area fallback.
fn oracle_box_sample(
  a: texture_2d<f32>,
  b: texture_2d<f32>,
  uv: vec2<f32>,
  logical_size: vec2<f32>,
) -> vec4<f32> {{
  let box_size = vec2<f32>(ORACLE_BOX_SIZE);
  if box_size.x <= ORACLE_BOX_FAST && box_size.y <= ORACLE_BOX_FAST {{
    return atlas_linear(a, b, uv);
  }}
  let box_start = uv * logical_size - box_size * 0.5;
  let box_end = box_start + box_size;
  var weighted = vec4<f32>(0.0);
  var area = 0.0;
  var cell_y = floor(box_start.y);
  loop {{
    if cell_y >= box_end.y {{ break; }}
    let low_y = max(cell_y, box_start.y);
    let high_y = min(cell_y + 2.0, box_end.y);
    var cell_x = floor(box_start.x);
    loop {{
      if cell_x >= box_end.x {{ break; }}
      let low_x = max(cell_x, box_start.x);
      let high_x = min(cell_x + 2.0, box_end.x);
      let cell_area = (high_x - low_x) * (high_y - low_y);
      let sample_uv = vec2<f32>(
        (low_x + high_x) * 0.5 / logical_size.x,
        (low_y + high_y) * 0.5 / logical_size.y,
      );
      weighted += atlas_linear(a, b, sample_uv) * cell_area;
      area += cell_area;
      cell_x += 2.0;
    }}
    cell_y += 2.0;
  }}
  if area <= ORACLE_BOX_FALLBACK_AREA {{
    return atlas_linear(a, b, uv);
  }}
  return weighted / area;
}}

// Exact selected full-range matrix. Luma uses logical size 2880 by 2880;
// chroma uses 1440 by 1440 at the same normalized coordinate, with no
// half-texel chroma shift. There is no levels(), packed-wide-word path or
// validity suppression on this instrument route.
fn oracle_ycbcr(uv: vec2<f32>) -> vec3<f32> {{
  let l = oracle_box_sample(luma0, luma1, uv, vec2<f32>(2880.0, 2880.0));
  let ch = oracle_box_sample(chroma0, chroma1, uv, vec2<f32>(1440.0, 1440.0));
  let y = l.r;
  let c = ch.rg - vec2<f32>(ORACLE_CHROMA_OFFSET);
  return vec3<f32>(
    y + ORACLE_R_CR * c.g,
    y + ORACLE_G_CB * c.r + ORACLE_G_CR * c.g,
    y + ORACLE_B_CB * c.r,
  );
}}

fn oracle_picture(position: vec2<f32>) -> vec4<f32> {{
  let pixel = u32(position.y) * ORACLE_WIDTH + u32(position.x);
  let at = pixel * ORACLE_STRIDE;
  let atlas_a = vec2<f32>(oracle_map[at], oracle_map[at + 1u]);
  let atlas_b = vec2<f32>(oracle_map[at + 2u], oracle_map[at + 3u]);
  let alpha = oracle_map[at + 4u];
  let covered = oracle_map[at + 5u];
  // Preserve the selected unconditional two-source fetch and alpha mix.
  let color_b = oracle_ycbcr(atlas_b);
  let color_a = oracle_ycbcr(atlas_a);
  let rgb = mix(color_b, color_a, alpha);
  return select(vec4<f32>(0.0), vec4<f32>(rgb, 1.0), covered > 0.5);
}}
"#,
        binding = FLOW_BINDING,
    )
}

fn draw_wgsl_core() -> String {
    format!(
        "{}\n{}\n{}\n{}\n{SHADER}",
        projection::wgsl(),
        band::lookup_wgsl(),
        chroma::lookup_wgsl(),
        sampling::wgsl(),
    )
}

fn inject_flow_blend(core: String) -> String {
    let injected = core.replace(FS_BLEND, FS_BLEND_FLOW);
    assert_ne!(
        injected, core,
        "the flow apply anchor moved in the fragment shader"
    );
    injected
}

/// The `KJERAG_FLOW` env knob (chunk 4, §37), the instrument's flow switch.
/// It is NO LONGER a compile-time gate: the pipeline builds both draw variants
/// unconditionally and the runtime toggle ([`Scene::set_flow`]) picks between
/// them. This survives as an OR term in [`ScenePipeline::prepare`] for routes
/// allowed to use the legacy draw, so `kjerag-spike --bin band` stays armed
/// without starting the player's worker. Selected ONE X2 refuses it. Default
/// OFF: the shipped player leaves it unset, so its toggle governs alone and
/// starts off. Read once, cached.
pub(crate) fn flow_on() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("KJERAG_FLOW")
            .map(|value| value != "0" && !value.is_empty())
            .unwrap_or(false)
    })
}

/// The per-lens coverage mask half-width, in degrees of colatitude off the seam
/// (§59/§62): each lens's far vignette edge is eroded inward by this before the
/// DIS runs, clipping the belt to its reliable clean-content edge. Studio's
/// `erodeBeltMasksUsingFisheyeMask` 96° (= 90 + 6). RE'd, not tuned — the same
/// value the spike's `flowrender` masks with, mirrored here for [`estimate_flow`].
const MASK_HALF_DEG: f32 = 6.0;

/// The draw's read-only view of the composed per-lens flow displacement maps,
/// on the draw's group 1 beside the band state (binding 0) and the chromatic
/// field (binding 2). Bound into the flow read group ([`ScenePipeline::draw`]).
pub(crate) const FLOW_BINDING: u32 = 3;

/// The fragment's plain blend, the anchor the flow apply is injected around.
const FS_BLEND: &str = "  if look.w > 0.0 {\n    mix = blend(look.xyz);\n  }";

/// The same, with the flow apply: each covered lens samples at its
/// flow-displaced belt coordinate. Legacy routes also take the colour blend
/// weight from their 32-degree SphereAlpha gate (§42.1/§80). ONE X2 uses its
/// captured 16-degree common gate for displacement and keeps its separately
/// uploaded static alpha for colour. Rust twin:
/// [`projection::Reframe::blend_flow`].
const FS_BLEND_FLOW: &str = "  if look.w > 0.0 {\n    mix = blend_flow(look.xyz);\n  }";

/// The flow apply's own WGSL, appended to the draw only when [`flow_on`]: the
/// flow buffer, its bilinear sampler, and `flow_shift` — the twin of
/// [`projection::Reframe::flow_shift`], the same belt inversion, the same
/// sample-coordinate displacement by both flow components, and the same forward
/// strip law back to a ray (§40).
pub(crate) fn flow_wgsl() -> String {
    format!(
        "@group(1) @binding({binding}) var<storage, read> flow_disp: array<f32>;\n\
         const FLOW_W = {w}u;\n\
         const FLOW_H = {h}u;\n\
         const FLOW_TAU = {tau:?};\n\
         const FLOW_PI = {pi:?};\n\
         const FLOW_GATE_W = {gate_w:?};\n\
         const FLOW_ONE_XS_GATE_W = {one_xs_gate_w:?};\n\
         // Rust twin: Displacement::sample, over the field in the same layout\n\
         // compose writes: the u plane then the v plane, row-major.\n\
         fn flow_at(plane: u32, c: i32, r: i32) -> f32 {{\n\
         \x20 // Both axes clamp to belt dims — Studio's kernel clamps, not wraps (§40).\n\
         \x20 let cw = clamp(c, 0, i32(FLOW_W) - 1);\n\
         \x20 let rc = clamp(r, 0, i32(FLOW_H) - 1);\n\
         \x20 return flow_disp[plane * FLOW_W * FLOW_H + u32(rc) * FLOW_W + u32(cw)];\n\
         }}\n\
         fn flow_sample(lens: u32, col: f32, row: f32) -> vec2<f32> {{\n\
         \x20 let c0 = floor(col);\n\
         \x20 let r0 = floor(row);\n\
         \x20 let fc = col - c0;\n\
         \x20 let fr = row - r0;\n\
         \x20 let ci = i32(c0);\n\
         \x20 let ri = i32(r0);\n\
         \x20 var out = vec2<f32>(0.0, 0.0);\n\
         \x20 // Lens 0 reads planes 0,1 (r2l [0xae8]); lens 1 reads planes 2,3 (l2r [0xa88]).\n\
         \x20 let base = lens * 2u;\n\
         \x20 for (var comp = 0u; comp < 2u; comp += 1u) {{\n\
         \x20   let plane = base + comp;\n\
         \x20   let a = flow_at(plane, ci, ri);\n\
         \x20   let b = flow_at(plane, ci + 1, ri);\n\
         \x20   let cc = flow_at(plane, ci, ri + 1);\n\
         \x20   let dd = flow_at(plane, ci + 1, ri + 1);\n\
         \x20   let top = a + (b - a) * fc;\n\
         \x20   let bot = cc + (dd - cc) * fc;\n\
         \x20   out[comp] = top + (bot - top) * fr;\n\
         \x20 }}\n\
         \x20 return out;\n\
         }}\n\
         // Rust twin: Reframe::flow_shift.\n\
         fn flow_shift(lens: u32, ray: vec3<f32>, alpha: f32) -> vec3<f32> {{\n\
         \x20 if reframe.lens_count <= 1.0 {{ return ray; }}\n\
         \x20 if !(alpha > 0.0 && alpha < 1.0) {{ return ray; }}\n\
         \x20 let body = reframe.view_to_body * ray;\n\
         \x20 let len = length(body);\n\
         \x20 if len <= 0.0 {{ return ray; }}\n\
         \x20 let phi = atan2(body.y, body.x);\n\
         \x20 let phi_mod = phi - FLOW_TAU * floor(phi / FLOW_TAU);\n\
         \x20 // c = coordinate(ray): FULL colatitude belt law, no half-pixel (§42.3).\n\
         \x20 // CLAMP c to belt dims on both axes (§40/§41 MED: clamp, not wrap/cut).\n\
         \x20 let theta = acos(clamp(body.z / len, -1.0, 1.0));\n\
         \x20 let col = clamp(f32(FLOW_W) * (FLOW_TAU - phi_mod) / FLOW_TAU, 0.0, f32(FLOW_W) - 1.0);\n\
         \x20 let row = clamp(theta * (f32(FLOW_H) - 1.0) / FLOW_PI, 0.0, f32(FLOW_H) - 1.0);\n\
         \x20 // Lens 0 samples r2l, lens 1 samples l2r — two separate fields (§38/§40).\n\
         \x20 let f = flow_sample(lens, col, row);\n\
         \x20 if f.x == 0.0 && f.y == 0.0 {{ return ray; }}\n\
         \x20 // POSITIVE add for BOTH lenses; the direction is in the two fields (§40).\n\
         \x20 // CLAMP q to belt dims on both axes.\n\
         \x20 let w = 1.0 - alpha;\n\
         \x20 let qcol = clamp(col + w * f.x, 0.0, f32(FLOW_W) - 1.0);\n\
         \x20 let qrow = clamp(row + w * f.y, 0.0, f32(FLOW_H) - 1.0);\n\
         \x20 let nphi = FLOW_TAU - qcol / f32(FLOW_W) * FLOW_TAU;\n\
         \x20 let ntheta = qrow / (f32(FLOW_H) - 1.0) * FLOW_PI;\n\
         \x20 let nb = vec3<f32>(cos(nphi) * sin(ntheta), sin(nphi) * sin(ntheta), cos(ntheta));\n\
         \x20 return transpose(reframe.view_to_body) * nb;\n\
         }}\n\
         // Rust twin: Reframe::gate_alpha — selected common flow-coverage gate (§42.1).\n\
         fn gate_alpha(ray: vec3<f32>) -> f32 {{\n\
         \x20 if reframe.lens_count <= 1.0 {{ return 1.0; }}\n\
         \x20 let body = reframe.view_to_body * ray;\n\
         \x20 let len = length(body);\n\
         \x20 if len <= 0.0 {{ return 1.0; }}\n\
         \x20 let theta_deg = acos(clamp(body.z / len, -1.0, 1.0)) * (180.0 / FLOW_PI);\n\
         \x20 let gate_w = select(FLOW_GATE_W, FLOW_ONE_XS_GATE_W, reframe.one_xs > 0.5);\n\
         \x20 return clamp(((90.0 + gate_w) - theta_deg) / (2.0 * gate_w), 0.0, 1.0);\n\
         }}\n\
         // Rust twin: Reframe::blend_flow. The selected common SphereAlpha gate\n\
         // (32 degrees legacy, 16 ONE X2) drives displacement (§42.1/§80). It also\n\
         // drives legacy colour; ONE X2's\n\
         // final colour share is its separately uploaded static alpha map.\n\
         fn blend_flow(ray: vec3<f32>) -> Blend {{\n\
         \x20 var out: Blend;\n\
         \x20 var total = 0.0;\n\
         \x20 let reach = length(ray);\n\
         \x20 let axis0 = axis_of(reframe.lenses[0], ray);\n\
         \x20 let axis1 = axis_of(reframe.lenses[1], ray);\n\
         \x20 let flow_alpha_a = gate_alpha(ray);\n\
         \x20 var colour_alpha_a = flow_alpha_a;\n\
         \x20 if reframe.one_xs > 0.5 {{\n\
         \x20   colour_alpha_a = one_xs_alpha(normalize(reframe.lenses[0].view_to_lens * ray));\n\
         \x20 }}\n\
         \x20 for (var index = 0u; index < MAX_LENSES; index += 1u) {{\n\
         \x20   let lens = reframe.lenses[index];\n\
         \x20   var landing: Landing;\n\
         \x20   var claimed = 0.0;\n\
         \x20   if within(lens, select(axis1, axis0, index == 0u), reach) {{\n\
         \x20     let flow_alpha_lens = select(1.0 - flow_alpha_a, flow_alpha_a, index == 0u);\n\
         \x20     let colour_alpha_lens = select(1.0 - colour_alpha_a, colour_alpha_a, index == 0u);\n\
         \x20     landing = project(lens, flow_shift(index, ray, flow_alpha_lens));\n\
         \x20     // §80: displaced sample on an invalid/border fisheye UV -> keep base.\n\
         \x20     if (!landing.inside) {{ landing = project(lens, ray); }}\n\
         \x20     let one_xs_claim = select(0.0, colour_alpha_lens, landing.inside);\n\
         \x20     let lens_claim = select(claim(landing, colour_alpha_lens), one_xs_claim, reframe.one_xs > 0.5);\n\
         \x20     claimed = select(0.0, lens_claim, f32(index) < reframe.lens_count);\n\
         \x20   }}\n\
         \x20   out.landings[index] = landing;\n\
         \x20   out.weights[index] = claimed;\n\
         \x20   total += claimed;\n\
         \x20 }}\n\
         \x20 if total > 0.0 {{\n\
         \x20   for (var index = 0u; index < MAX_LENSES; index += 1u) {{\n\
         \x20     out.weights[index] = share(out.weights[index], total);\n\
         \x20   }}\n\
         \x20 }}\n\
         \x20 return out;\n\
         }}\n",
        binding = FLOW_BINDING,
        w = band::STRIP_W,
        h = band::STRIP_H,
        tau = std::f32::consts::TAU,
        pi = std::f32::consts::PI,
        gate_w = 0.5 * projection::GATE_WIDTH_DEG,
        one_xs_gate_w = 0.5 * projection::ONE_XS_FLOW_GATE_WIDTH_DEG,
    )
}

/// The selected ONE X2 retained-field apply. The storage planes have the same
/// semantic order as [`crate::flow::one_xs::Displacement`]: lens A/B, then
/// d-column/d-row. Rows unroll 400 degrees along the seam; columns are the
/// gnomonic across-seam coordinate. The scalar common SphereAlpha gate remains
/// separate from the final OneXS Template colour alpha.
pub(crate) fn one_xs_flow_wgsl() -> String {
    format!(
        r#"@group(1) @binding({binding}) var<storage, read> flow_disp: array<f32>;
const FLOW_W = {cols}u;
const FLOW_H = {rows}u;
const FLOW_PI = {pi:?};
const FLOW_DEG_PER_ROW = {degrees_per_row:?};
const FLOW_CENTRE_COL = {centre_col:?};
const FLOW_GNOMONIC_PIXELS = {gnomonic_pixels:?};
const FLOW_ONE_XS_GATE_W = {gate_w:?};

// Rust twin: one_xs::Displacement::sample. Both retained coordinates clamp.
fn flow_at(plane: u32, col: i32, row: i32) -> f32 {{
  let c = clamp(col, 0, i32(FLOW_W) - 1);
  let r = clamp(row, 0, i32(FLOW_H) - 1);
  return flow_disp[plane * FLOW_W * FLOW_H + u32(r) * FLOW_W + u32(c)];
}}

fn flow_sample(lens: u32, col: f32, row: f32) -> vec2<f32> {{
  let c0 = floor(col);
  let r0 = floor(row);
  let fc = col - c0;
  let fr = row - r0;
  let ci = i32(c0);
  let ri = i32(r0);
  var out = vec2<f32>(0.0);
  let base = lens * 2u;
  for (var component = 0u; component < 2u; component += 1u) {{
    let plane = base + component;
    let top = mix(flow_at(plane, ci, ri), flow_at(plane, ci + 1, ri), fc);
    let bottom = mix(flow_at(plane, ci, ri + 1), flow_at(plane, ci + 1, ri + 1), fc);
    out[component] = mix(top, bottom, fr);
  }}
  return out;
}}

// Rust twin: Reframe::one_xs_flow_shift. The shared +0x810 line coordinate
// is recovered from the body ray, displaced in final 1080x60 grid units, then
// converted back to the body direction consumed by the per-lens base map.
fn flow_shift(lens: u32, ray: vec3<f32>, alpha: f32) -> vec3<f32> {{
  if reframe.lens_count <= 1.0 || reframe.one_xs <= 0.5 {{ return ray; }}
  if !(alpha > 0.0 && alpha < 1.0) {{ return ray; }}

  let body = reframe.view_to_body * ray;
  let line = vec3<f32>(-body.x, body.z, body.y);
  let rho = length(line.xz);
  // `!(rho > 0)` rejects zero and NaN. The uniform matrix and fragment ray
  // are finite, so infinity is not reachable at this boundary.
  if !(rho > 0.0) {{ return ray; }}

  let phi_deg = atan2(line.x, line.z) * (180.0 / FLOW_PI);
  let row = clamp((phi_deg + 200.0) / FLOW_DEG_PER_ROW, 0.0, f32(FLOW_H) - 1.0);
  let col = clamp(
    FLOW_CENTRE_COL - FLOW_GNOMONIC_PIXELS * line.y / rho,
    0.0,
    f32(FLOW_W) - 1.0,
  );
  let displacement = flow_sample(lens, col, row);
  if displacement.x == 0.0 && displacement.y == 0.0 {{ return ray; }}

  let weight = 1.0 - alpha;
  let qcol = clamp(col + weight * displacement.x, 0.0, f32(FLOW_W) - 1.0);
  let qrow = clamp(row + weight * displacement.y, 0.0, f32(FLOW_H) - 1.0);

  let qphi = (-200.0 + qrow * FLOW_DEG_PER_ROW) * (FLOW_PI / 180.0);
  let tangent = (FLOW_CENTRE_COL - qcol) / FLOW_GNOMONIC_PIXELS;
  let qrho = 1.0 / sqrt(1.0 + tangent * tangent);
  let qline = vec3<f32>(sin(qphi) * qrho, tangent * qrho, cos(qphi) * qrho);
  let qbody = vec3<f32>(-qline.x, qline.z, qline.y);
  return transpose(reframe.view_to_body) * qbody;
}}

// The captured 16-degree common SphereAlpha, lens A's share.
fn gate_alpha(ray: vec3<f32>) -> f32 {{
  if reframe.lens_count <= 1.0 {{ return 1.0; }}
  let body = reframe.view_to_body * ray;
  let reach = length(body);
  if reach <= 0.0 {{ return 1.0; }}
  let theta_deg = acos(clamp(body.z / reach, -1.0, 1.0)) * (180.0 / FLOW_PI);
  return clamp(
    ((90.0 + FLOW_ONE_XS_GATE_W) - theta_deg) / (2.0 * FLOW_ONE_XS_GATE_W),
    0.0,
    1.0,
  );
}}

// Rust twin: Reframe::blend_one_xs_flow. Displacement and final colour use
// different captured alpha resources on this type-2 route.
fn blend_flow(ray: vec3<f32>) -> Blend {{
  var out: Blend;
  var total = 0.0;
  let reach = length(ray);
  let axis0 = axis_of(reframe.lenses[0], ray);
  let axis1 = axis_of(reframe.lenses[1], ray);
  let flow_alpha_a = gate_alpha(ray);
  let colour_alpha_a = one_xs_alpha(normalize(reframe.lenses[0].view_to_lens * ray));

  for (var index = 0u; index < MAX_LENSES; index += 1u) {{
    let lens = reframe.lenses[index];
    var landing: Landing;
    var claimed = 0.0;
    if within(lens, select(axis1, axis0, index == 0u), reach) {{
      let flow_alpha_lens = select(1.0 - flow_alpha_a, flow_alpha_a, index == 0u);
      let colour_alpha_lens = select(1.0 - colour_alpha_a, colour_alpha_a, index == 0u);
      landing = project(lens, flow_shift(index, ray, flow_alpha_lens));
      if !landing.inside {{ landing = project(lens, ray); }}
      claimed = select(0.0, colour_alpha_lens, landing.inside && f32(index) < reframe.lens_count);
    }}
    out.landings[index] = landing;
    out.weights[index] = claimed;
    total += claimed;
  }}
  if total > 0.0 {{
    for (var index = 0u; index < MAX_LENSES; index += 1u) {{
      out.weights[index] = share(out.weights[index], total);
    }}
  }}
  return out;
}}
"#,
        binding = FLOW_BINDING,
        cols = crate::flow::one_xs::COLS,
        rows = crate::flow::one_xs::ROWS,
        pi = std::f32::consts::PI,
        degrees_per_row = crate::flow::one_xs::DEGREES_PER_ROW,
        centre_col = crate::flow::one_xs::CENTRE_COL,
        gnomonic_pixels = crate::flow::one_xs::GNOMONIC_PIXELS,
        gate_w = 0.5 * projection::ONE_XS_FLOW_GATE_WIDTH_DEG,
    )
}

const SHADER: &str = r#"
struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  let x = f32((i << 1u) & 2u);
  let y = f32(i & 2u);
  var out: VsOut;
  out.uv = vec2<f32>(x, y);
  out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  return out;
}

// Two planes per lens, in lens order, then the sampler they share. WGSL has
// no texture array to index here, so `picture` branches on the lens the ray
// picked; `SAMPLER_BINDING` is the Rust half of these numbers.
@group(0) @binding(1) var luma0: texture_2d<f32>;
@group(0) @binding(2) var chroma0: texture_2d<f32>;
@group(0) @binding(3) var luma1: texture_2d<f32>;
@group(0) @binding(4) var chroma1: texture_2d<f32>;
@group(0) @binding(5) var samp: sampler;



fn picture(mix: Blend, ratio: vec2<f32>, look: vec3<f32>) -> vec4<f32> {
  var rgb = vec3<f32>(0.0);
  var total = 0.0;
  // Stage 3's pooled luma gain is NOT applied any more (owner ruling
  // 2026-08-12). The pass is to carry exactly two modifications: the chromatic
  // correction below, and the mechanical hand-over between the lenses. A
  // brightness gain is a third, and while it was measured and it worked, it
  // was in the way of judging the second and third by eye.
  //
  // The measurement is still taken and still pooled - `--bin expose` and the
  // band's own trace read it - it simply does not reach the picture.
  // What the two lenses' COLOUR has to be brought together by at this
  // direction, split between them the same way the exposure is. Zero until
  // something has been measured, so a picture with no reading behind it is the
  // picture the pass drew before the chromatic arm existed.
  // Studio's grid, sampled per lens at this ray's own place in the fusion
  // window. Each lens carries its OWN correction over its own 12-row block -
  // they are separate nodes even where the blocks overlap - so this is two
  // lookups and not one field applied twice with opposite signs.
  let body = reframe.view_to_body * look;
  if mix.weights[0] > 0.0 {
    let fix = grid_fix(0u, body);
    rgb += mix.weights[0] * ycbcr(luma0, chroma0, frame_uv(mix.landings[0].pixel), ratio.x, fix);
    total += mix.weights[0];
  }
  if mix.weights[1] > 0.0 {
    let fix = grid_fix(1u, body);
    rgb += mix.weights[1] * ycbcr(luma1, chroma1, frame_uv(mix.landings[1].pixel), ratio.y, fix);
    total += mix.weights[1];
  }
  // The room around the ball, written rather than painted: transparent black,
  // which through the pass's premultiplied blend leaves what is under the
  // widget alone (issue #100). Black is what makes it premultiplied; what
  // fills the room is the shell's business and not this pass's.
  return select(vec4<f32>(0.0), vec4<f32>(rgb, 1.0), total > 0.0);
}

// BT.709, off whichever of the two plane layouts this capture decodes to.
// The chroma plane is little endian and its first component is Cb.
//
// The two planes are handed the same magnification and reach their own
// conclusions from it, because they are not the same size: `plane` scales the
// ratio by the grid it is sampling (`sampling::plane_ratio`), so the chroma
// plane upgrades an octave of zoom before the luma plane does.
fn ycbcr(luma: texture_2d<f32>, chroma: texture_2d<f32>, uv: vec2<f32>, ratio: f32, fix: vec3<f32>) -> vec3<f32> {
  let l = plane(luma, samp, uv, ratio, reframe.sharpen_luma);
  let ch = plane(chroma, samp, uv, ratio, reframe.sharpen_chroma);
  // NV12: one byte of luma, and a pair of bytes of chroma.
  var raw = vec3<f32>(l.r, ch.r, ch.g);
  if reframe.wide > 0.5 {
    // P010, aliased two bytes at a time because the device cannot make a
    // 16-bit normalized texture (`dmabuf::plane_format`).
    raw = vec3<f32>(plane_word(l.rg), plane_word(ch.rg), plane_word(ch.ba));
  }
  let range = levels();
  // The chromatic correction, in Studio's own space: an additive offset in
  // Y'CbCr CODES, applied before the matrix.
  //
  // **The pedestal ratio reduces to exactly this, and the algebra is worth
  // writing down.** Studio's map holds `r = (lens + S + 255) / (lens + 255)`
  // and applies it as `recoverData3f = (c + 1) * r - 1` on a normalized `c`.
  // Substituting `c = lens/255`:
  //
  //   (lens/255 + 1) * (lens + S + 255)/(lens + 255) - 1
  //     = (lens + 255)/255 * (lens + S + 255)/(lens + 255) - 1
  //     = (lens + S + 255)/255 - 1
  //     = (lens + S)/255
  //
  // So the ratio form is a pure ADDITIVE offset of `S` codes at the cell it
  // was computed from. It reads as gain-like only because the map is
  // low-resolution and resampled: a ratio formed at a map cell is applied to
  // full-resolution pixels near it whose values differ. Carrying `S` directly
  // is the same correction without the encode-and-decode, and it is why this
  // solves in codes rather than in a log ratio - Studio's `PENALTY = 0.1` and
  // `TOLERANCE = 1e-4` are numbers in codes, and the ring's log-RGB units made
  // them mean something else entirely.
  let y = raw.x * range.luma.x + range.luma.y + fix.x / 255.0;
  let c = raw.yz * range.chroma.x + vec2<f32>(range.chroma.y) + fix.yz / 255.0;
  return vec3<f32>(
    y + 1.5748 * c.g,
    y - 0.1873 * c.r - 0.4681 * c.g,
    y + 1.8556 * c.r,
  );
}

fn linearize(c: vec3<f32>) -> vec3<f32> {
  let lo = c / 12.92;
  let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
  return select(lo, hi, c > vec3<f32>(0.04045));
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let look = view_ray(in.uv);
  // Zero, which is every weight zero: the room around the ball at the far end
  // of the zoom (issue #47) is a fragment no lens has, and `picture` already
  // paints that. Nothing is sampled for it and no model is run.
  var mix: Blend;
  if look.w > 0.0 {
    mix = blend(look.xyz);
  }
  // Here rather than inside the blend: a derivative has to be taken where
  // every lane of the quad is running, and the blend is all branches. What
  // the neighbouring lanes landed on is exactly what this asks about, so the
  // landings are read after the blend has answered for all of them.
  let ratio = vec2<f32>(
    texel_ratio(mix.landings[0].pixel),
    texel_ratio(mix.landings[1].pixel),
  );
  let lens = picture(mix, ratio, look.xyz);
  return vec4<f32>(
    select(lens.rgb, linearize(lens.rgb), reframe.linearize > 0.5),
    lens.a,
  );
}
"#;

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::mpsc;

    use super::*;
    use kjerag_meta::{
        CalibrationSet, ExposureTrack, Filter, GyroConfig, GyroEncoding, GyroSample, GyroTrack,
        OrientationSample, OrientationTrack, Quat, Sweep,
    };

    use crate::flow::one_xs::pis::Level;
    use crate::flow::one_xs::scalar::{
        CpuPairedPisSolver, PairSolveError, PairSolveStage, PairedPatchGrids, PairedSolveRequest,
    };
    use crate::flow::one_xs::{COLS, LensPair, ROWS};
    use crate::projection::tests::{ONE_XS_FRAME, one_xs_lenses};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct InjectedSolverFailure;

    impl std::fmt::Display for InjectedSolverFailure {
        fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            out.write_str("injected paired PIS failure")
        }
    }

    impl std::error::Error for InjectedSolverFailure {}

    #[derive(Clone, Copy)]
    enum SolverInjection {
        Failure,
        Stamp,
    }

    struct InjectingSolver {
        at: PairSolveStage,
        injection: SolverInjection,
        cpu: CpuPairedPisSolver,
    }

    struct PanickingSolver {
        at: PairSolveStage,
        cpu: CpuPairedPisSolver,
    }

    struct WrongReceiptSolver {
        flight: GpuPisFlight,
        cpu: CpuPairedPisSolver,
    }

    impl PairedPisSolver for InjectingSolver {
        type Error = InjectedSolverFailure;
        const BACKEND: crate::studio_type2::PisBackend = crate::studio_type2::PisBackend::Cpu;

        fn solve(
            &mut self,
            mut request: PairedSolveRequest,
        ) -> Result<PairedPatchGrids, Self::Error> {
            if request.stage == self.at {
                match self.injection {
                    SolverInjection::Failure => return Err(InjectedSolverFailure),
                    SolverInjection::Stamp => {
                        request.stage = mismatched_stage(request.stage);
                    }
                }
            }
            Ok(self.cpu.solve(request).unwrap())
        }
    }

    impl PairedPisSolver for PanickingSolver {
        type Error = InjectedSolverFailure;
        const BACKEND: crate::studio_type2::PisBackend = crate::studio_type2::PisBackend::Cpu;

        fn solve(&mut self, request: PairedSolveRequest) -> Result<PairedPatchGrids, Self::Error> {
            if request.stage == self.at {
                panic!("injected paired PIS panic at {}", request.stage);
            }
            Ok(self.cpu.solve(request).unwrap())
        }
    }

    impl PairedPisSolver for WrongReceiptSolver {
        type Error = GpuPisSolverError;
        const BACKEND: PisBackend = PisBackend::Gpu;

        fn solve(&mut self, request: PairedSolveRequest) -> Result<PairedPatchGrids, Self::Error> {
            let expected = GpuPisStageReceipt {
                flight: self.flight.clone(),
                stage: request.stage,
            };
            let grids = self.cpu.solve(request).unwrap();
            let mut actual = expected.clone();
            actual.flight.generation += 1;
            finish_gpu_pis_stage(
                &expected,
                GpuPisStageOutput {
                    receipt: actual,
                    grids,
                },
            )
        }
    }

    fn mismatched_stage(stage: PairSolveStage) -> PairSolveStage {
        match stage {
            PairSolveStage::Cold { calculation, level } => PairSolveStage::Cold {
                calculation: (calculation + 1) % 3,
                level,
            },
            PairSolveStage::Warm { level } => PairSolveStage::Cold {
                calculation: 0,
                level,
            },
        }
    }

    fn solver_stages() -> [PairSolveStage; 8] {
        [
            PairSolveStage::Cold {
                calculation: 0,
                level: Level::Two,
            },
            PairSolveStage::Cold {
                calculation: 0,
                level: Level::One,
            },
            PairSolveStage::Cold {
                calculation: 1,
                level: Level::Two,
            },
            PairSolveStage::Cold {
                calculation: 1,
                level: Level::One,
            },
            PairSolveStage::Cold {
                calculation: 2,
                level: Level::Two,
            },
            PairSolveStage::Cold {
                calculation: 2,
                level: Level::One,
            },
            PairSolveStage::Warm { level: Level::Two },
            PairSolveStage::Warm { level: Level::One },
        ]
    }

    fn reservation_calibration() -> CalibrationSet {
        CalibrationSet {
            camera_model: "Insta360 ONE X2".to_owned(),
            firmware: "synthetic".to_owned(),
            dimension: kjerag_meta::Size {
                width: ONE_XS_FRAME.width,
                height: ONE_XS_FRAME.height,
            },
            lenses: one_xs_lenses(),
            rolling_shutter_ms: 23.516_071_319_580_078,
            gyro: GyroConfig {
                encoding: GyroEncoding::Scaled,
                imu_orientation: "Zxy",
                first_frame_timestamp: 0,
                gyro_timestamp: None,
            },
            exposure: [ExposureTrack::default(), ExposureTrack::default()],
            imu: GyroTrack::default(),
            fused: OrientationTrack::from_samples(
                (1_900_000..=2_100_000)
                    .step_by(2_000)
                    .map(|offset_us| OrientationSample {
                        offset_us,
                        world_from_body: Quat::IDENTITY,
                    })
                    .collect(),
            ),
            calibration_canvas: kjerag_meta::Size {
                width: 6_080,
                height: 3_040,
            },
        }
    }

    fn reservation_capture() -> Arc<OneXsCapture> {
        Arc::new(OneXsCapture::new(&reservation_calibration()).unwrap())
    }

    fn reservation_stamp(index: u64, previous: Option<&FrameStamp>) -> FrameStamp {
        FrameStamp::for_test(index, Duration::from_secs(2), previous)
    }

    fn reservation_blurred(code: u8) -> BlurredBelts {
        BlurredBelts::from_lenses(LensPair {
            a: vec![code; ROWS * COLS],
            b: vec![code.wrapping_add(83); ROWS * COLS],
        })
        .unwrap()
    }

    fn reservation_cpu_schedule(reservation: &OneXsReservation, code: u8) -> ColdPreparedSchedule {
        let input = ColdInputs::from_blurred_belts_and_masks(
            reservation_blurred(code),
            reservation.prepared().masks().clone(),
        );
        ColdPreparedSchedule::from_cpu(&input)
    }

    fn reserve_frame(capture: &Arc<OneXsCapture>, frame: &FrameStamp) -> OneXsReservation {
        match capture
            .reserve(
                frame,
                Size {
                    width: ONE_XS_FRAME.width,
                    height: ONE_XS_FRAME.height,
                },
            )
            .unwrap()
        {
            OneXsPreparation::Reserved(reservation) => reservation,
            OneXsPreparation::Ready(_) => panic!("test frame was already ready"),
            OneXsPreparation::InFlight(_) => panic!("test frame was already reserved"),
        }
    }

    fn complete_frame(
        capture: &Arc<OneXsCapture>,
        frame: &FrameStamp,
        code: u8,
    ) -> Arc<OneXsMapFrame> {
        let reservation = reserve_frame(capture, frame);
        let completed = match reservation.commit_scalar(reservation_blurred(code)) {
            Ok(completed) => completed,
            Err(rejected) => panic!("test scalar commit failed: {}", rejected.error),
        };
        completed
            .install()
            .unwrap_or_else(|rejected| panic!("test install failed: {rejected}"))
    }

    fn assert_solver_reservation_recovers(stage: PairSolveStage, injection: SolverInjection) {
        let capture = reservation_capture();
        let first = reservation_stamp(0, None);
        let (offered, old_ready, code) = match stage {
            PairSolveStage::Cold { .. } => (first.clone(), None, 91),
            PairSolveStage::Warm { .. } => {
                let ready = complete_frame(&capture, &first, 89);
                let second = reservation_stamp(1, Some(&first));
                (second, Some(ready), 97)
            }
        };
        let reservation = reserve_frame(&capture, &offered);
        let owner_pointer = reservation.owner.as_deref().unwrap() as *const FrameOwner;
        let ColdPreparedSchedule {
            controls,
            solver: cpu,
        } = reservation_cpu_schedule(&reservation, code);
        let mut solver = InjectingSolver {
            at: stage,
            injection,
            cpu,
        };
        let rejected = match reservation.commit_prepared_with_solver(controls, &mut solver) {
            Ok(_) => panic!("injected {stage} transaction unexpectedly succeeded"),
            Err(rejected) => rejected,
        };
        match (&injection, &rejected.error) {
            (
                SolverInjection::Failure,
                FrameCommitError::Solver(PairSolveError::Solver {
                    stage: actual,
                    source,
                }),
            ) => {
                assert_eq!(*actual, stage);
                assert_eq!(*source, InjectedSolverFailure);
                assert_eq!(
                    rejected.error.to_string(),
                    format!("ONE X2 paired PIS failed at {stage}: injected paired PIS failure")
                );
            }
            (
                SolverInjection::Stamp,
                FrameCommitError::Solver(PairSolveError::Stamp { stage: actual, .. }),
            ) => {
                assert_eq!(*actual, stage);
                assert!(
                    rejected
                        .error
                        .to_string()
                        .starts_with(&format!("ONE X2 paired PIS stamp failed at {stage}:"))
                );
            }
            _ => panic!("injected {stage} returned the wrong typed failure"),
        }
        rejected.reservation.abort().unwrap();

        {
            let state = capture.state.lock().unwrap();
            assert_eq!(
                state.owner.as_deref().unwrap() as *const FrameOwner,
                owner_pointer,
                "{stage} did not restore the exact old owner allocation"
            );
            assert!(state.in_flight.is_none());
            match (&old_ready, &state.ready) {
                (None, None) => {}
                (Some(expected), Some(actual)) => assert!(
                    Arc::ptr_eq(expected, actual),
                    "{stage} replaced the last complete map allocation"
                ),
                _ => panic!("{stage} changed whether a ready map exists"),
            }
        }

        let retried = complete_frame(&capture, &offered, code);
        assert!(Arc::ptr_eq(
            &retried,
            &capture.ready(&offered).unwrap().unwrap()
        ));
        let successor = reservation_stamp(offered.index() + 1, Some(&offered));
        let successor_ready = complete_frame(&capture, &successor, code.wrapping_add(3));
        assert!(Arc::ptr_eq(
            &successor_ready,
            &capture.ready(&successor).unwrap().unwrap()
        ));
    }

    fn assert_warm_solver_panic_recovers(level: Level) {
        let capture = reservation_capture();
        let first = reservation_stamp(0, None);
        let second = reservation_stamp(1, Some(&first));
        let third = reservation_stamp(2, Some(&second));
        let ready_before = complete_frame(&capture, &first, 101);
        let reservation = reserve_frame(&capture, &second);
        let owner_pointer = reservation.owner.as_deref().unwrap() as *const FrameOwner;
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let ColdPreparedSchedule {
                controls,
                solver: cpu,
            } = reservation_cpu_schedule(&reservation, 103);
            let mut solver = PanickingSolver {
                at: PairSolveStage::Warm { level },
                cpu,
            };
            let _ = reservation.commit_prepared_with_solver(controls, &mut solver);
        }));
        assert!(unwound.is_err());

        {
            let state = capture.state.lock().unwrap();
            assert_eq!(
                state.owner.as_deref().unwrap() as *const FrameOwner,
                owner_pointer,
                "warm {level} panic did not retain the exact old owner allocation"
            );
            assert!(state.in_flight.is_none());
            assert!(Arc::ptr_eq(state.ready.as_ref().unwrap(), &ready_before));
        }

        let retried = complete_frame(&capture, &second, 103);
        assert!(Arc::ptr_eq(
            &retried,
            &capture.ready(&second).unwrap().unwrap()
        ));
        let successor = complete_frame(&capture, &third, 107);
        assert!(Arc::ptr_eq(
            &successor,
            &capture.ready(&third).unwrap().unwrap()
        ));
    }

    #[test]
    fn every_solver_stage_failure_restores_the_production_reservation() {
        for stage in solver_stages() {
            assert_solver_reservation_recovers(stage, SolverInjection::Failure);
        }
    }

    #[test]
    fn every_solver_stage_stamp_error_restores_the_production_reservation() {
        for stage in solver_stages() {
            assert_solver_reservation_recovers(stage, SolverInjection::Stamp);
        }
    }

    #[test]
    fn gpu_pis_receipt_requires_exact_generation_frame_and_stage() {
        let first = reservation_stamp(0, None);
        let expected = GpuPisStageReceipt {
            flight: GpuPisFlight {
                generation: 7,
                frame: first.clone(),
            },
            stage: PairSolveStage::Cold {
                calculation: 1,
                level: Level::Two,
            },
        };
        assert!(validate_gpu_pis_receipt(&expected, &expected).is_ok());

        let mut wrong_generation = expected.clone();
        wrong_generation.flight.generation += 1;
        assert!(validate_gpu_pis_receipt(&expected, &wrong_generation).is_err());

        let mut wrong_frame = expected.clone();
        wrong_frame.flight.frame = reservation_stamp(0, None);
        assert_ne!(wrong_frame.flight.frame, first);
        assert!(validate_gpu_pis_receipt(&expected, &wrong_frame).is_err());

        let mut wrong_stage = expected.clone();
        wrong_stage.stage = mismatched_stage(expected.stage);
        assert!(validate_gpu_pis_receipt(&expected, &wrong_stage).is_err());
    }

    #[test]
    fn rejected_gpu_receipt_restores_owner_and_ready_without_cpu_fallback() {
        let capture = reservation_capture();
        let first = reservation_stamp(0, None);
        let second = reservation_stamp(1, Some(&first));
        let ready = complete_frame(&capture, &first, 113);
        assert_eq!(ready.pis_backend(), PisBackend::Cpu);
        let reservation = reserve_frame(&capture, &second);
        let owner_pointer = reservation.owner.as_deref().unwrap() as *const FrameOwner;
        let ColdPreparedSchedule {
            controls,
            solver: cpu,
        } = reservation_cpu_schedule(&reservation, 127);
        let mut solver = WrongReceiptSolver {
            flight: reservation.flight.clone(),
            cpu,
        };
        let rejected = match reservation.commit_prepared_with_solver(controls, &mut solver) {
            Ok(_) => panic!("wrong GPU receipt committed a map"),
            Err(rejected) => rejected,
        };
        assert!(matches!(
            rejected.error,
            FrameCommitError::Solver(PairSolveError::Solver {
                source: GpuPisSolverError::Receipt { .. },
                ..
            })
        ));
        rejected.reservation.abort().unwrap();

        let state = capture.state.lock().unwrap();
        assert_eq!(
            state.owner.as_deref().unwrap() as *const FrameOwner,
            owner_pointer
        );
        assert!(state.in_flight.is_none());
        assert!(Arc::ptr_eq(state.ready.as_ref().unwrap(), &ready));
        assert_eq!(state.ready.as_ref().unwrap().frame(), &first);
        assert_eq!(state.ready.as_ref().unwrap().pis_backend(), PisBackend::Cpu);
    }

    #[test]
    fn solver_failure_and_failed_abort_preserve_both_raw_errors() {
        let capture = reservation_capture();
        let first = reservation_stamp(0, None);
        let reservation = reserve_frame(&capture, &first);
        let ColdPreparedSchedule {
            controls,
            solver: cpu,
        } = reservation_cpu_schedule(&reservation, 131);
        let mut solver = InjectingSolver {
            at: PairSolveStage::Cold {
                calculation: 1,
                level: Level::Two,
            },
            injection: SolverInjection::Failure,
            cpu,
        };
        let rejected = match reservation.commit_prepared_with_solver(controls, &mut solver) {
            Ok(_) => panic!("injected solver failure unexpectedly committed"),
            Err(rejected) => rejected,
        };
        let RejectedOneXsSolverReservation { reservation, error } = *rejected;
        let primary = error.to_string();
        {
            let mut state = capture.state.lock().unwrap();
            state.in_flight = Some(GpuPisFlight {
                generation: reservation.flight.generation + 1,
                frame: reservation.flight.frame.clone(),
            });
        }

        let combined = abort_one_xs_after_error(reservation, error.into());
        assert_eq!(combined.source().unwrap().to_string(), primary);
        assert!(combined.to_string().starts_with(&primary));
        assert!(
            combined
                .to_string()
                .contains("ONE X2 stitch rollback names a stale reservation generation")
        );
    }

    #[test]
    fn warm_solver_panics_leave_the_exact_production_reservation_retryable() {
        for level in [Level::Two, Level::One] {
            assert_warm_solver_panic_recovers(level);
        }
    }

    #[test]
    fn selected_type2_source_shader_parses_and_validates() {
        use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

        let source = draw_wgsl_map_oracle(2160);
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("selected type-2 source WGSL did not parse: {error}"));
        Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|error| {
                panic!("selected type-2 source WGSL did not validate: {error}")
            });

        let oracle = map_oracle_wgsl(2160);
        let yuv = oracle
            .split_once("fn oracle_ycbcr")
            .expect("selected source function exists")
            .1
            .split_once("fn oracle_picture")
            .expect("selected source function has a bounded body")
            .0;
        assert!(yuv.contains("vec2<f32>(2880.0, 2880.0)"));
        assert!(yuv.contains("vec2<f32>(1440.0, 1440.0)"));
        assert!(!yuv.contains("levels("));
        assert!(!yuv.contains("plane_word"));
        assert!(!yuv.contains("reframe.wide"));
        let picture = oracle
            .split_once("fn oracle_picture")
            .expect("selected picture function exists")
            .1;
        let b = picture.find("let color_b = oracle_ycbcr(atlas_b)").unwrap();
        let a = picture.find("let color_a = oracle_ycbcr(atlas_a)").unwrap();
        let blend = picture.find("mix(color_b, color_a, alpha)").unwrap();
        assert!(b < a && a < blend);
    }

    #[test]
    fn map_geometry_shader_keeps_kjerag_picture_consumer() {
        use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

        let source = draw_wgsl_map_geometry_oracle(2160);
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("map geometry WGSL did not parse: {error}"));
        Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|error| panic!("map geometry WGSL did not validate: {error}"));
        assert!(source.contains("mix = geometry_oracle_blend(in.pos.xy)"));
        assert!(source.contains("let lens = picture(mix, ratio, look.xyz)"));
        assert!(source.contains("ycbcr(luma0, chroma0"));
        assert!(source.contains("let range = levels()"));
        assert!(!source.contains("oracle_ycbcr"));
        assert!(!source.contains("oracle_box_sample"));
    }

    #[test]
    fn normal_prepare_clears_instrument_oracle_selections() {
        let mut draw = FlowDraw::OneXs;
        assert_eq!(draw, FlowDraw::OneXs);
        draw = FlowDraw::prepared(false);
        assert_eq!(draw, FlowDraw::Plain);

        draw = FlowDraw::OneXs;
        assert_eq!(draw, FlowDraw::OneXs);
        draw = FlowDraw::prepared(true);
        assert_eq!(draw, FlowDraw::Legacy);

        draw = FlowDraw::MapOracle;
        assert_eq!(draw, FlowDraw::MapOracle);
        draw = FlowDraw::prepared(false);
        assert_eq!(draw, FlowDraw::Plain);

        draw = FlowDraw::MapOracle;
        assert_eq!(draw, FlowDraw::MapOracle);
        draw = FlowDraw::prepared(true);
        assert_eq!(draw, FlowDraw::Legacy);

        draw = FlowDraw::DirectOneXs;
        assert_eq!(draw, FlowDraw::DirectOneXs);
        draw = FlowDraw::prepared(false);
        assert_eq!(draw, FlowDraw::Plain);

        draw = FlowDraw::Nothing;
        assert_eq!(draw, FlowDraw::Nothing);
        draw = FlowDraw::prepared(true);
        assert_eq!(draw, FlowDraw::Legacy);
    }

    #[test]
    fn direct_route_diagnostic_requires_route_resource_and_display_identity() {
        let complete = 17u64;
        let bound = 17u64;
        assert_eq!(
            exact_direct_one_xs_draw(FlowDraw::DirectOneXs, Some(&complete), Some(&bound),),
            Some(&bound)
        );

        assert_eq!(
            exact_direct_one_xs_draw(FlowDraw::Plain, Some(&complete), Some(&bound)),
            None
        );
        assert_eq!(
            exact_direct_one_xs_draw::<u64>(FlowDraw::DirectOneXs, Some(&complete), None),
            None
        );
        let other = 18u64;
        assert_eq!(
            exact_direct_one_xs_draw(FlowDraw::DirectOneXs, Some(&complete), Some(&other),),
            None
        );
    }

    #[test]
    fn selected_playback_requires_both_the_evidence_gate_and_capture_state() {
        assert!(!one_xs_playback_selected(false, false));
        assert!(!one_xs_playback_selected(false, true));
        assert!(!one_xs_playback_selected(true, false));
        assert!(one_xs_playback_selected(true, true));
    }

    #[test]
    fn selected_player_waits_only_for_an_unacknowledged_current_frame() {
        // No offered frame is the initial admission for frame zero.
        assert!(!one_xs_frame_waiting(true, true, None));
        // An exact ready-map acknowledgement admits the successor.
        assert!(!one_xs_frame_waiting(true, true, Some(true)));
        // The current selected frame remains offered until that exact map is
        // capture-owned, independently of whether it has ever been shown.
        assert!(one_xs_frame_waiting(true, true, Some(false)));

        for current_ready in [None, Some(false), Some(true)] {
            assert!(!one_xs_frame_waiting(false, true, current_ready));
            assert!(!one_xs_frame_waiting(true, false, current_ready));
        }
    }

    #[test]
    fn selected_seek_reuses_only_the_exact_current_transaction() {
        assert_eq!(
            one_xs_replay_start(Some(8), Some(8), true, true, 8),
            ReplayStart::Continue
        );
        assert_eq!(
            one_xs_replay_start(Some(8), Some(9), false, true, 9),
            ReplayStart::Continue
        );
        assert_eq!(
            one_xs_replay_start(Some(8), Some(9), false, true, 10),
            ReplayStart::Continue
        );
        for start in [
            one_xs_replay_start(None, Some(0), false, false, 9),
            one_xs_replay_start(Some(8), Some(8), false, true, 8),
            one_xs_replay_start(Some(8), Some(9), false, false, 9),
            one_xs_replay_start(Some(8), Some(9), false, true, 8),
            one_xs_replay_start(Some(9), Some(9), true, true, 4),
        ] {
            assert_eq!(start, ReplayStart::FrameZero);
        }

        assert_eq!(
            selected_replay_start(ReplayStart::Continue, false, false, Some(9), 10),
            ReplayStart::FrameZero,
            "an arbitrary forward seek restarts video and retargets audio"
        );
        assert_eq!(
            selected_replay_start(ReplayStart::Continue, true, false, Some(9), 10),
            ReplayStart::Continue,
            "one forward step keeps the proven adjacent audio splice"
        );
        assert_eq!(
            selected_replay_start(ReplayStart::Continue, false, false, Some(9), 9),
            ReplayStart::Continue,
            "the exact offered transaction needs no decoder seek"
        );
        assert_eq!(
            selected_replay_start(ReplayStart::Continue, false, true, Some(7), 7),
            ReplayStart::FrameZero,
            "retargeting an active replay must also retarget its audio"
        );
    }

    #[test]
    fn a_step_during_replay_is_relative_to_the_requested_target() {
        assert_eq!(replay_step_base(Some(100), Some(7)), Some(100));
        assert_eq!(replay_step_base(None, Some(7)), Some(7));
    }

    #[test]
    fn paused_target_landing_keeps_redrawing_until_its_map_is_acknowledged() {
        assert_eq!(
            next_after_pump(true, false, false, None),
            Next::Refresh,
            "the Player has landed, but the Scene transaction is still pending"
        );
        assert_eq!(
            next_after_pump(false, false, false, None),
            Next::Never,
            "only target-map acknowledgement retires the Scene replay"
        );
    }

    #[test]
    fn terminal_stop_retires_the_scene_owned_replay() {
        let replay = RefCell::new(Some(OneXsReplay {
            target: 100,
            position: Duration::from_secs(4),
            playing: true,
        }));
        retire_replay(&replay);
        assert!(replay.borrow().is_none());
    }

    #[test]
    fn diagnostic_display_requires_the_exact_shown_delivery_and_capture() {
        #[derive(PartialEq, Eq)]
        struct Stamp {
            index: u64,
            pair: u64,
        }
        let current = Stamp {
            index: 6_369,
            pair: 10,
        };
        let same_numbers_other_delivery = Stamp {
            index: 6_369,
            pair: 11,
        };

        assert!(exact_selected_display(&current, Some(&current), true));
        assert!(!exact_selected_display(
            &current,
            Some(&same_numbers_other_delivery),
            true
        ));
        assert!(!exact_selected_display(&current, Some(&current), false));
        assert!(!exact_selected_display(&current, None, true));
        assert!(Scene::blank().diagnostic_one_xs_map().unwrap().is_none());
    }

    #[test]
    fn selected_draw_stays_empty_until_an_exact_map_is_ready() {
        assert_eq!(FlowDraw::selected_map(false), FlowDraw::Nothing);
        assert_eq!(FlowDraw::selected_map(true), FlowDraw::DirectOneXs);
    }

    #[test]
    fn a_still_requested_after_an_empty_stop_fails_with_the_retained_raw_error() {
        let scene = Scene::blank();
        scene
            .stalled
            .fail_now("ONE X2 stitch rejected source frame 0");
        // The alert is an independent one-shot and may already have consumed
        // its copy before the pilot asks for a still.
        assert_eq!(
            scene.stalled.take(),
            Some(Stall::new("ONE X2 stitch rejected source frame 0"))
        );

        let (sent, received) = mpsc::channel();
        scene.capture(Request {
            width: 3840,
            then: Box::new(move |result| {
                sent.send(result.map(|_| ()).map_err(|error| error.to_string()))
                    .unwrap();
            }),
        });

        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err("ONE X2 stitch rejected source frame 0".to_owned())
        );
        assert!(scene.shutter.take().is_none());
    }

    #[test]
    fn an_armed_still_gets_the_acknowledgement_failure_once_off_thread() {
        let scene = Scene::blank();
        let caller = std::thread::current().id();
        let (sent, received) = mpsc::channel();
        scene.capture(Request {
            width: 3840,
            then: Box::new(move |result| {
                sent.send((
                    std::thread::current().id(),
                    result.map(|_| ()).map_err(|error| error.to_string()),
                ))
                .unwrap();
            }),
        });

        let error = "ONE X2 stitch state lost its acknowledgement for source frame 41";
        scene.stalled.fail_now(error);
        scene.fail_terminal_shutter_without_display();
        let (callback, result) = received.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_ne!(callback, caller);
        assert_eq!(result, Err(error.to_owned()));

        scene.fail_terminal_shutter_without_display();
        assert_eq!(
            received.recv_timeout(Duration::from_millis(20)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
    }

    #[test]
    fn an_observed_terminal_stop_resolves_an_armed_still_once() {
        let scene = Scene::blank();
        let (sent, received) = mpsc::channel();
        scene.capture(Request {
            width: 3840,
            then: Box::new(move |result| {
                sent.send(result.map(|_| ()).map_err(|error| error.to_string()))
                    .unwrap();
            }),
        });

        let error = "ONE X2 stitch rejected source frame 0";
        scene.stalled.fail_now(error);
        let stall = scene.stalled.take().expect("terminal stop was not raised");
        assert_eq!(
            scene.finish_observed_terminal_stop(stall),
            Stall::new(error)
        );
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err(error.to_owned())
        );
        assert!(scene.shutter.take().is_none());

        scene.finish_observed_terminal_stop(Stall::new(error));
        assert_eq!(
            received.recv_timeout(Duration::from_millis(20)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
    }

    #[test]
    fn a_pending_selected_still_waits_until_the_capture_is_terminal() {
        let shutter = Shutter::default();
        let stalled = Stalled::default();
        let (sent, received) = mpsc::channel();
        shutter.arm(Request {
            width: 3840,
            then: Box::new(move |result| {
                sent.send(result.map(|_| ()).map_err(|error| error.to_string()))
                    .unwrap();
            }),
        });

        assert!(resolve_selected_shutter(&shutter, &stalled, false).is_none());
        assert_eq!(
            received.recv_timeout(Duration::from_millis(20)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );

        stalled.fail_now("ONE X2 stitch rejected source frame 0");
        assert!(resolve_selected_shutter(&shutter, &stalled, false).is_none());
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err("ONE X2 stitch rejected source frame 0".to_owned())
        );
    }

    #[test]
    fn selected_display_replaces_only_after_an_exact_success() {
        #[derive(Clone, Debug, PartialEq, Eq)]
        struct Stamp {
            index: u64,
            epoch: u64,
            pair: u64,
        }

        let a = Stamp {
            index: 8,
            epoch: 1,
            pair: 80,
        };
        let b = Stamp {
            index: 9,
            epoch: 1,
            pair: 90,
        };
        let wrong_b = Stamp {
            index: 9,
            epoch: 1,
            pair: 91,
        };
        let mut display = ExactDisplay::default();

        assert!(display.commit(&a, &a));
        assert_eq!(display.recovery_index(&a, Some(&a), [(0, true)]), Some(0));

        // Merely staging B changes no completed display. A failed B whose map
        // names another delivery is refused before it can replace A.
        assert_eq!(display.recovery_index(&a, Some(&a), [(0, true)]), Some(0));
        assert!(!display.commit(&b, &wrong_b));
        assert_eq!(display.recovery_index(&a, Some(&a), [(0, true)]), Some(0));

        assert!(display.commit(&b, &b));
        assert_eq!(display.recovery_index(&a, Some(&a), [(0, true)]), None);
        assert_eq!(display.recovery_index(&b, Some(&b), [(0, true)]), Some(0));
    }

    #[test]
    fn selected_display_recovery_requires_the_complete_exact_tuple() {
        #[derive(Clone, Debug, PartialEq, Eq)]
        struct Stamp {
            index: u64,
            epoch: u64,
            pair: u64,
        }

        let complete = Stamp {
            index: 6_369,
            epoch: 4,
            pair: 10,
        };
        let same_numbers_other_delivery = Stamp {
            index: 6_369,
            epoch: 4,
            pair: 11,
        };
        let mut display = ExactDisplay::default();
        assert!(display.commit(&complete, &complete));

        assert_eq!(
            display.recovery_index(&complete, Some(&complete), [(0, false), (1, true)]),
            Some(1)
        );
        assert_eq!(
            display.recovery_index(&complete, Some(&complete), [(0, false), (1, false)]),
            None
        );
        assert_eq!(display.recovery_index(&complete, None, [(0, true)]), None);
        assert_eq!(
            display.recovery_index(&complete, Some(&same_numbers_other_delivery), [(0, true)],),
            None
        );
        assert_eq!(
            display.recovery_index(
                &same_numbers_other_delivery,
                Some(&same_numbers_other_delivery),
                [(0, true)],
            ),
            None
        );
        assert_eq!(
            display.recovery_index(&complete, Some(&complete), [(0, true)]),
            Some(0)
        );
    }

    #[test]
    fn an_exact_held_display_services_a_shutter_after_terminal_failure() {
        #[derive(Clone, Debug, PartialEq, Eq)]
        struct Stamp(u64);

        let complete = Stamp(6_369);
        let mut display = ExactDisplay::default();
        assert!(display.commit(&complete, &complete));
        let restorable = display
            .recovery_index(&complete, Some(&complete), [(0, true)])
            .is_some();

        let shutter = Shutter::default();
        let stalled = Stalled::default();
        stalled.fail_now("successor stitch failed");
        let (sent, received) = mpsc::channel();
        shutter.arm(Request {
            width: 3840,
            then: Box::new(move |result| {
                sent.send(result.map(|_| ()).map_err(|error| error.to_string()))
                    .unwrap();
            }),
        });

        let request = resolve_selected_shutter(&shutter, &stalled, restorable)
            .expect("the exact held display must service this request");
        (request.then)(Ok(capture::Shot {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
            index: 6_369,
            time: Duration::from_secs(212),
        }));
        assert_eq!(
            received.recv_timeout(Duration::from_secs(1)).unwrap(),
            Ok(())
        );
        assert!(shutter.take().is_none());
    }

    #[test]
    fn ordinary_prepare_never_constructs_the_source_readback_pipeline() {
        let Ok((device, queue)) = test_gpu() else {
            eprintln!("no GPU available for the source readback laziness test");
            return;
        };
        let mut pipeline = ScenePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        assert!(pipeline.one_xs_luma.is_none());
        assert!(pipeline.one_xs_belts.is_none());

        let scene = Scene::blank();
        pipeline.prepare(&scene.primitive(Camera::default()), &device, &queue, 1.0);
        assert!(pipeline.one_xs_luma.is_none());
        assert!(pipeline.one_xs_belts.is_none());
    }

    #[test]
    fn dropped_reservation_restores_the_exact_owner_and_advances_generation() {
        let capture = reservation_capture();
        let first = reservation_stamp(0, None);
        let owner_before = {
            let state = capture.state.lock().unwrap();
            std::ptr::from_ref(state.owner.as_deref().expect("fresh capture has its owner"))
        };

        let reservation = reserve_frame(&capture, &first);
        assert_eq!(reservation.flight.generation, 1);
        {
            let state = capture.state.lock().unwrap();
            assert!(state.owner.is_none());
            assert_eq!(state.in_flight.as_ref(), Some(&reservation.flight));
        }
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _reservation = reservation;
            panic!("injected panic while the reservation owns the estimator");
        }));
        assert!(unwound.is_err());

        let restored = capture.state.lock().unwrap();
        assert_eq!(
            std::ptr::from_ref(restored.owner.as_deref().expect("drop restored the owner"),),
            owner_before
        );
        assert!(restored.in_flight.is_none());
        drop(restored);

        let second_reservation = reserve_frame(&capture, &first);
        assert_eq!(second_reservation.flight.generation, 2);
        second_reservation.abort().unwrap();
    }

    #[test]
    fn completed_reservation_drop_during_panic_publishes_the_usable_owner_and_map() {
        let capture = reservation_capture();
        let first = reservation_stamp(0, None);
        let reservation = reserve_frame(&capture, &first);
        let completed = match reservation.commit_scalar(reservation_blurred(59)) {
            Ok(completed) => completed,
            Err(rejected) => panic!("cold scalar commit failed: {}", rejected.error),
        };

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _completed = completed;
            panic!("injected panic after scalar success and before install");
        }));
        assert!(unwound.is_err());

        let state = capture.state.lock().unwrap();
        assert!(state.owner.is_some());
        assert!(state.in_flight.is_none());
        assert_eq!(
            state.ready.as_deref().map(OneXsMapFrame::frame),
            Some(&first)
        );
    }

    #[test]
    fn concurrent_recreated_pipeline_and_aba_delivery_get_no_second_reservation() {
        let capture = reservation_capture();
        let first = reservation_stamp(0, None);
        let reservation = reserve_frame(&capture, &first);
        let imposter = reservation_stamp(0, None);
        assert_ne!(imposter, first);

        for offered in [&first, &imposter] {
            assert!(matches!(
                capture
                    .reserve(offered, Size::new(ONE_XS_FRAME.width, ONE_XS_FRAME.height))
                    .unwrap(),
                OneXsPreparation::InFlight(_)
            ));
        }
        let state = capture.state.lock().unwrap();
        assert_eq!(state.generation, 1);
        assert_eq!(state.in_flight.as_ref(), Some(&reservation.flight));
        assert!(state.owner.is_none());
        drop(state);
        reservation.abort().unwrap();
    }

    #[test]
    fn install_rejections_retain_the_completed_owner_result_and_original_flight() {
        let capture = reservation_capture();
        let first = reservation_stamp(0, None);
        let reservation = reserve_frame(&capture, &first);
        let mut completed = match reservation.commit_scalar(reservation_blurred(61)) {
            Ok(completed) => completed,
            Err(rejected) => panic!("cold scalar commit failed: {}", rejected.error),
        };
        let actual_flight = completed.flight.clone();

        completed.flight.generation += 1;
        let rejected = completed
            .install()
            .expect_err("wrong generation installed a completion");
        assert!(rejected.completion.owner.is_some());
        assert!(rejected.completion.result.is_some());
        assert!(rejected.reason.contains("stale reservation generation"));
        let mut completed = rejected.completion;
        completed.flight = actual_flight.clone();

        completed.flight.frame = reservation_stamp(0, None);
        let rejected = completed
            .install()
            .expect_err("wrong opaque frame installed a completion");
        assert!(rejected.completion.owner.is_some());
        assert!(rejected.completion.result.is_some());
        assert!(rejected.reason.contains("stale reservation generation"));
        let mut completed = rejected.completion;
        completed.flight = actual_flight.clone();

        completed.previous_ready = Some(Arc::new(
            completed
                .result
                .as_ref()
                .expect("rejected completion retained its result")
                .map
                .clone(),
        ));
        let rejected = completed
            .install()
            .expect_err("wrong ready allocation installed a completion");
        assert!(rejected.completion.owner.is_some());
        assert!(rejected.completion.result.is_some());
        assert!(rejected.reason.contains("changed ready map"));
        let mut completed = rejected.completion;
        completed.previous_ready = None;

        completed.pis_backend = PisBackend::Gpu;
        let rejected = completed
            .install()
            .expect_err("wrong solver backend installed a completion");
        assert!(rejected.completion.owner.is_some());
        assert!(rejected.completion.result.is_some());
        assert!(rejected.reason.contains("map backend is cpu, expected gpu"));
        let mut completed = rejected.completion;
        completed.pis_backend = PisBackend::Cpu;

        {
            let state = capture.state.lock().unwrap();
            assert_eq!(state.in_flight.as_ref(), Some(&actual_flight));
            assert!(state.owner.is_none());
            assert!(state.ready.is_none());
        }
        let installed = completed.install().unwrap();
        let ready = capture.ready(&first).unwrap().unwrap();
        assert!(Arc::ptr_eq(&installed, &ready));
        assert_eq!(installed.packed().bytes(), ready.packed().bytes());
    }

    #[test]
    fn dropped_install_rejection_retains_the_advanced_owner_and_old_ready_map() {
        let capture = reservation_capture();
        let first = reservation_stamp(0, None);
        let second = reservation_stamp(1, Some(&first));
        let first_ready = complete_frame(&capture, &first, 61);
        let reservation = reserve_frame(&capture, &second);
        let mut completed = match reservation.commit_scalar(reservation_blurred(67)) {
            Ok(completed) => completed,
            Err(rejected) => panic!("warm scalar commit failed: {}", rejected.error),
        };
        completed.flight.generation += 1;
        let rejected = completed
            .install()
            .expect_err("wrong generation installed a completion");
        drop(rejected);

        let state = capture.state.lock().unwrap();
        assert!(
            state.owner.is_none(),
            "a stale completion overwrote the owner slot for the recorded flight"
        );
        assert!(state.in_flight.is_some());
        assert!(state.terminal);
        assert_eq!(state.quarantined_owners.len(), 1);
        assert!(Arc::ptr_eq(
            state.ready.as_ref().expect("old ready map was lost"),
            &first_ready
        ));
        assert_eq!(state.ready.as_deref().unwrap().frame(), &first);
        drop(state);

        assert!(matches!(
            capture
                .reserve(
                    &first,
                    Size::new(ONE_XS_FRAME.width, ONE_XS_FRAME.height)
                )
                .unwrap(),
            OneXsPreparation::Ready(ready) if Arc::ptr_eq(&ready, &first_ready)
        ));
        let error =
            match capture.reserve(&second, Size::new(ONE_XS_FRAME.width, ONE_XS_FRAME.height)) {
                Err(error) => error,
                Ok(_) => panic!("terminal capture reserved another successor"),
            };
        assert!(error.to_string().contains("could not be published"));
    }

    #[test]
    fn stale_completed_drop_does_not_overwrite_or_clear_a_newer_flight() {
        let capture = reservation_capture();
        let first = reservation_stamp(0, None);
        let reservation = reserve_frame(&capture, &first);
        let completed = match reservation.commit_scalar(reservation_blurred(69)) {
            Ok(completed) => completed,
            Err(rejected) => panic!("cold scalar commit failed: {}", rejected.error),
        };
        let newer = GpuPisFlight {
            generation: completed.flight.generation + 1,
            frame: reservation_stamp(1, Some(&first)),
        };
        {
            let mut state = capture.state.lock().unwrap();
            state.in_flight = Some(newer.clone());
        }

        drop(completed);

        let state = capture.state.lock().unwrap();
        assert_eq!(state.in_flight.as_ref(), Some(&newer));
        assert!(state.owner.is_none());
        assert!(state.terminal);
        assert_eq!(state.quarantined_owners.len(), 1);
    }

    #[test]
    fn abort_error_and_poison_retain_the_exact_owner_without_double_panic() {
        let capture = reservation_capture();
        let first = reservation_stamp(0, None);
        let reservation = reserve_frame(&capture, &first);
        let owner_pointer = reservation.owner.as_deref().unwrap() as *const FrameOwner;

        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let capture = capture.clone();
            move || {
                let _state = capture.state.lock().unwrap();
                panic!("injected capture-lock poison");
            }
        }));
        assert!(poisoned.is_err());

        let rejected = reservation
            .abort()
            .expect_err("poisoned capture lock accepted rollback");
        assert!(rejected.reason.contains("owner stopped unexpectedly"));
        drop(rejected);

        let state = capture
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(state.terminal);
        assert!(state.in_flight.is_none());
        assert_eq!(state.quarantined_owners.len(), 0);
        assert_eq!(
            state.owner.as_deref().unwrap() as *const FrameOwner,
            owner_pointer
        );
    }

    #[test]
    fn rejected_scalar_commit_restores_ready_arc_and_exact_successor_state() {
        let control = reservation_capture();
        let trial = reservation_capture();
        let first = reservation_stamp(0, None);
        let second = reservation_stamp(1, Some(&first));
        let third = reservation_stamp(2, Some(&second));
        let control_first = complete_frame(&control, &first, 71);
        let trial_first = complete_frame(&trial, &first, 71);
        let ready_before = trial.ready(&first).unwrap().unwrap();
        assert!(Arc::ptr_eq(&trial_first, &ready_before));

        let mut reservation = reserve_frame(&trial, &second);
        match trial
            .reserve(&second, Size::new(ONE_XS_FRAME.width, ONE_XS_FRAME.height))
            .unwrap()
        {
            OneXsPreparation::InFlight(Some(ready)) => {
                assert!(Arc::ptr_eq(&ready_before, &ready));
            }
            _ => panic!("duplicate pipeline did not receive the retained ready allocation"),
        }
        reservation
            .prepared
            .as_deref_mut()
            .unwrap()
            .replace_frame_for_test(first.clone());
        let rejected = match reservation.commit_scalar(reservation_blurred(79)) {
            Ok(_) => panic!("duplicate prepared frame advanced the estimator"),
            Err(rejected) => rejected,
        };
        assert!(rejected.error.to_string().contains("repeats frame 0"));
        rejected.reservation.abort().unwrap();

        let ready_after = trial.ready(&first).unwrap().unwrap();
        assert!(Arc::ptr_eq(&ready_before, &ready_after));
        assert!(Arc::ptr_eq(&trial_first, &ready_after));
        assert_eq!(trial.state.lock().unwrap().generation, 2);

        let control_second = complete_frame(&control, &second, 79);
        let trial_second = complete_frame(&trial, &second, 79);
        assert_eq!(control_second.packed(), trial_second.packed());
        assert_eq!(control_second.alpha(), trial_second.alpha());

        let control_third = complete_frame(&control, &third, 83);
        let trial_third = complete_frame(&trial, &third, 83);
        assert_eq!(control_third.packed(), trial_third.packed());
        assert_eq!(control_third.alpha(), trial_third.alpha());
        assert_eq!(control_first.frame(), trial_first.frame());
    }

    /// Opt-in because this is the production dmabuf path: it needs a target
    /// Vulkan adapter, VA-API decode and an actual paired ONE X2 capture.
    /// `KJERAG_ONE_X2_TEST_MEDIA` names either half; media owns sibling
    /// discovery and exact lens ordering just as it does for ordinary opens.
    #[test]
    fn selected_one_x2_scene_keeps_exact_gpu_transaction_ownership() {
        let Some(path) = std::env::var_os("KJERAG_ONE_X2_TEST_MEDIA").map(PathBuf::from) else {
            eprintln!("skipping selected ONE X2 scene transaction: set KJERAG_ONE_X2_TEST_MEDIA");
            return;
        };
        let ((device, queue), (foreign_device, foreign_queue)) = test_import_gpu_and_foreign()
            .unwrap_or_else(|error| {
                panic!("could not open the target dmabuf Vulkan device: {error}")
            });
        let mut scene = Scene::open(&path)
            .unwrap_or_else(|error| panic!("could not open ONE X2 test capture: {error}"));
        // This is also required by the opt-in test's invocation contract: run
        // it through `scripts/quiet.sh`. Muting here closes the interval
        // between open and the first pause as well.
        scene.set_muted(true);
        let mut pipeline = ScenePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);

        let first = wait_for_new_scene_frame(&scene, None);
        let first_primitive = scene.primitive(Camera::default());
        assert!(
            first_primitive
                .view
                .as_ref()
                .is_some_and(|view| view.one_xs.is_some()),
            "KJERAG_ONE_X2_TEST_MEDIA is not a selected paired ONE X2 capture"
        );
        pipeline.prepare(&first_primitive, &device, &queue, 1.0);
        assert_eq!(pipeline.flow_draw, FlowDraw::DirectOneXs);
        assert_eq!(pipeline.diagnostic_one_xs_direct_frame(), Some(&first));
        assert_eq!(
            pipeline
                .direct_one_xs_map
                .as_ref()
                .and_then(DirectMapDraw::bound_frame),
            Some(&first)
        );
        assert_eq!(
            scene
                .diagnostic_one_xs_map()
                .expect("first selected map lookup failed")
                .expect("the first selected transaction did not commit")
                .frame(),
            &first
        );
        assert!(pipeline.one_xs_belts.is_some());
        assert!(pipeline.one_xs_pis.is_some());
        assert_eq!(pipeline.one_xs_gpu_pis_completed_stages, 6);
        assert!(pipeline.one_xs_luma.is_none());
        assert_eq!(
            scene
                .diagnostic_one_xs_map()
                .unwrap()
                .unwrap()
                .pis_backend(),
            PisBackend::Gpu
        );

        let second = wait_for_new_scene_frame(&scene, Some(&first));
        let second_primitive = scene.primitive(Camera::default());
        let frames = pipeline
            .prepare_inner(&second_primitive, &device, &queue, 1.0, true)
            .expect("frame one did not become the exact bound source");
        assert_eq!(frames.stamp(), second);
        assert_eq!(pipeline.flow_draw, FlowDraw::Nothing);

        let capture = second_primitive
            .view
            .as_ref()
            .and_then(|view| view.one_xs.as_ref())
            .expect("selected frame lost its capture owner");
        let reservation = match capture
            .reserve(&second, frames.size)
            .expect("could not reserve frame one")
        {
            OneXsPreparation::Reserved(reservation) => reservation,
            OneXsPreparation::Ready(_) => panic!("frame one was committed before its GPU belts"),
            OneXsPreparation::InFlight(_) => panic!("frame one already had a reservation"),
        };

        // iced may recreate its pipeline while the capture-owned transaction
        // is waiting. The new pipeline has neither the prior import nor its
        // direct-map resource, but it must reconstruct and retain that exact
        // completed display without making a second solver submission.
        let mut recreated = ScenePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        pipeline
            .one_xs_gpu
            .ensure_same(&recreated.one_xs_gpu)
            .expect("pipeline recreation changed the authoritative GPU context");
        recreated.prepare(&second_primitive, &device, &queue, 1.0);
        assert_eq!(recreated.flow_draw, FlowDraw::DirectOneXs);
        assert_eq!(recreated.diagnostic_one_xs_direct_frame(), Some(&first));
        assert_eq!(
            recreated
                .direct_one_xs_map
                .as_ref()
                .and_then(DirectMapDraw::bound_frame),
            Some(&first)
        );
        assert!(
            recreated.one_xs_belts.is_none(),
            "recreated pipeline submitted a second solver transaction"
        );
        assert!(recreated.one_xs_pis.is_none());
        assert_eq!(recreated.one_xs_gpu_pis_completed_stages, 0);

        let observed_frames = frames.clone();
        let mut pending = pipeline
            .submit_one_xs_solver_belts(frames, reservation.prepared())
            .expect("could not submit frame one's exact retained maps and bound source");
        let retained_with_pending = Arc::strong_count(&observed_frames);
        let completion = Arc::new(AtomicU8::new(0));
        pending.pending.observe_completion(Arc::clone(&completion));

        // Same report fields and decode epoch, different opaque delivered
        // pair. This is the ABA case an index/timestamp-only association
        // would accept. Mutating the private outer receipt adds no production
        // constructor or alternate route.
        let imposter = FrameStamp::for_test(second.index(), second.timestamp(), Some(&first));
        assert_ne!(imposter, second);
        let mismatch = crate::studio_type2::FrameMapMismatch::new(
            "GPU solver belts",
            &imposter,
            "prepared geometry",
            &second,
        );
        pending.frame = imposter;
        let error = pending
            .read(reservation.prepared())
            .expect_err("a different delivered pair impersonated prepared geometry");
        assert_eq!(
            error.downcast_ref::<crate::studio_type2::FrameMapMismatch>(),
            Some(&mismatch)
        );
        assert_eq!(
            completion.load(Ordering::SeqCst),
            2,
            "ABA refusal released its source without exact GPU completion"
        );
        assert_eq!(
            Arc::strong_count(&observed_frames),
            retained_with_pending - 1,
            "completed ABA refusal did not release exactly its pending FramePair Arc"
        );
        reservation
            .abort()
            .expect("rejected receipt did not restore the exact owner");

        assert!(capture.ready(&first).unwrap().is_some());
        assert!(capture.ready(&second).unwrap().is_none());
        assert_eq!(
            pipeline
                .direct_one_xs_map
                .as_ref()
                .and_then(DirectMapDraw::bound_frame),
            Some(&first)
        );
        assert_eq!(
            pipeline.flow_draw,
            FlowDraw::Nothing,
            "the failed successor selected a legacy or unstitched fallback"
        );
        assert!(pipeline.diagnostic_one_xs_direct_frame().is_none());
        assert_eq!(
            second_primitive
                .shown
                .get()
                .expect("failed successor lost the last complete display")
                .frames
                .stamp(),
            first
        );

        // The rejected receipt consumed no estimator history. The ordinary
        // production entry point can prepare, sample, commit, upload and
        // acknowledge the same real successor exactly once.
        pipeline.prepare(&second_primitive, &device, &queue, 1.0);
        assert_eq!(pipeline.flow_draw, FlowDraw::DirectOneXs);
        assert_eq!(pipeline.diagnostic_one_xs_direct_frame(), Some(&second));
        assert_eq!(pipeline.one_xs_gpu_pis_completed_stages, 8);
        assert_eq!(
            pipeline
                .direct_one_xs_map
                .as_ref()
                .and_then(DirectMapDraw::bound_frame),
            Some(&second)
        );
        assert_eq!(
            scene
                .diagnostic_one_xs_map()
                .expect("second selected map lookup failed")
                .expect("the recovered transaction did not commit")
                .frame(),
            &second
        );
        assert_eq!(
            scene
                .diagnostic_one_xs_map()
                .unwrap()
                .unwrap()
                .pis_backend(),
            PisBackend::Gpu
        );

        // Even a terminal selected redraw may recover the retained display.
        // Authenticate before that branch: a different valid pair must touch
        // no old resource, and the failure's raw context error must survive.
        assert_ne!(device, foreign_device);
        let retained_live = Arc::as_ptr(&pipeline.live.front().unwrap().frames);
        let retained_prepared = pipeline.prepared_picture.as_ref().unwrap().frame().clone();
        let foreign_scope = foreign_device.push_error_scope(wgpu::ErrorFilter::Validation);
        pipeline.prepare(&second_primitive, &foreign_device, &foreign_queue, 1.0);
        let foreign_gpu_error = block_on(foreign_scope.pop());
        assert!(
            foreign_gpu_error.is_none(),
            "foreign Scene recovery reached GPU validation: {foreign_gpu_error:?}"
        );
        assert_eq!(pipeline.flow_draw, FlowDraw::Nothing);
        assert_eq!(
            pipeline
                .direct_one_xs_map
                .as_ref()
                .and_then(DirectMapDraw::bound_frame),
            Some(&second)
        );
        assert_eq!(pipeline.one_xs_display.complete.as_ref(), Some(&second));
        assert_eq!(
            Arc::as_ptr(&pipeline.live.front().unwrap().frames),
            retained_live
        );
        assert_eq!(
            pipeline.prepared_picture.as_ref().unwrap().frame(),
            &retained_prepared
        );
        assert_eq!(
            second_primitive.stalled.take().unwrap().to_string(),
            "ONE X2 GPU submission crossed a different device or queue"
        );
        assert_eq!(
            second_primitive.stalled.terminal().unwrap().to_string(),
            "ONE X2 GPU submission crossed a different device or queue"
        );
        scene.pause(Instant::now());
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

    fn test_gpu() -> Result<(wgpu::Device, wgpu::Queue), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
            .ok_or("no Vulkan adapter")?;
        block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("source readback laziness test"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())
    }

    type GpuPair = (wgpu::Device, wgpu::Queue);

    fn test_import_gpu_and_foreign() -> Result<(GpuPair, GpuPair), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        let primary = dmabuf::open_device(&adapter).map_err(|error| error.to_string())?;
        let foreign = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("foreign selected ONE X2 Scene context"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        Ok((primary, foreign))
    }

    fn wait_for_new_scene_frame(scene: &Scene, previous: Option<&FrameStamp>) -> FrameStamp {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match scene.pump(Instant::now()) {
                Next::Stopped(error) => panic!("ONE X2 test playback stopped: {error}"),
                Next::Refresh | Next::At(_) | Next::Never => {}
            }
            if let Some(frame) = scene.frame_stamp()
                && previous.is_none_or(|previous| &frame != previous)
            {
                return frame;
            }
            assert!(
                Instant::now() < deadline,
                "ONE X2 test playback did not deliver its next frame"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn player_flow_refuses_the_selected_one_xs_route() {
        assert!(!player_flow(false, false));
        assert!(!player_flow(false, true));
        assert!(player_flow(true, false));
        assert!(!player_flow(true, true));

        for requested in [false, true] {
            for environment in [false, true] {
                assert!(
                    !legacy_flow_draw(requested, environment, true),
                    "selected ONE X2 admitted requested={requested}, environment={environment}",
                );
            }
        }
        assert!(!legacy_flow_draw(false, false, false));
        assert!(legacy_flow_draw(true, false, false));
        assert!(legacy_flow_draw(false, true, false));
        assert!(legacy_flow_draw(true, true, false));
    }

    #[test]
    fn scene_capability_uses_the_projection_owned_one_xs_identity() {
        assert!(supports_player_flow(None));

        let mut lenses = projection::tests::one_xs_lenses();
        let selected_type = lenses[0].lens_type;
        assert!(!supports_player_flow(Some(&lenses)));

        lenses[1].lens_type = 0;
        assert!(!supports_player_flow(Some(&lenses)));

        lenses[0].lens_type = 0;
        assert!(supports_player_flow(Some(&lenses)));

        lenses[1].lens_type = selected_type;
        assert!(supports_player_flow(Some(&lenses)));
    }

    /// A camera rolling at a constant rate, as an orientation track: enough
    /// for the one question this module owns, which is whether a frame's
    /// readout reaches the pass.
    fn turning(rate_dps: f64) -> OrientationTrack {
        let samples = (0..2_000)
            .map(|index| GyroSample {
                offset_us: index * 1_000,
                rate_dps: [0.0, 0.0, rate_dps],
                accel_g: [0.0, -1.0, 0.0],
            })
            .collect();
        Filter::default().solve(
            &GyroTrack::from_samples(samples),
            kjerag_meta::Mat3::IDENTITY,
        )
    }

    fn motion(orientation: OrientationTrack) -> Motion {
        Motion {
            orientation,
            exposure: ExposureTrack::default(),
            readout: Readout {
                seconds: 0.015_883,
                sweep: Sweep::Right,
            },
        }
    }

    /// The whole of issue #9's "and if there is no gyro": a file with no IMU
    /// record has nothing to correct with, so the pass is handed no readout at
    /// all rather than a zero one, and it runs as it did before.
    #[test]
    fn a_file_with_no_gyro_track_gets_no_readout() {
        let held = motion(OrientationTrack::default());

        assert_eq!(held.rolling(1_000_000, held.readout), None);
    }

    /// And a camera whose readout direction has not been measured is the same
    /// case. Cameras outside the measured X4 and ONE X2 families still use
    /// `Sweep::Unknown`, a zero axis with nothing to apply along.
    #[test]
    fn an_unknown_sweep_gets_no_readout() {
        let held = motion(turning(90.0));
        let unknown = Readout {
            sweep: Sweep::Unknown,
            ..held.readout
        };

        assert_eq!(held.rolling(1_000_000, unknown), None);
        assert!(held.rolling(1_000_000, held.readout).is_some());
    }

    /// With both, the turn handed to the pass is the one the body made across
    /// that frame's readout, centred on the frame's own instant: 90 deg/s
    /// through 15.883 ms is 1.43 degrees, about the body's forward axis.
    #[test]
    fn a_readout_carries_the_turn_the_body_made_during_it() {
        let held = motion(turning(90.0));
        let rolling = held.rolling(1_000_000, held.readout).expect("no readout");

        let turn = rolling.turn[2].to_degrees();
        assert!(
            (turn + 1.43).abs() < 0.05,
            "{turn} degrees across the readout"
        );
        assert_eq!(rolling.axis, [1.0, 0.0]);
    }
}
