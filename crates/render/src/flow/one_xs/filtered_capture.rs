//! Typed worker bridge from decoded resident sources to filtered panoramas.
//!
//! This owner is one decode epoch. Admission and exact-stamp presentation are
//! nonblocking. The existing stitch worker owns ordered panorama preparation;
//! one capture-local temporal worker owns the seven-source stream and its GPU
//! completion polling. A stitch actor drains the source queue independently of
//! shell messages. CPU work admission is separate from GPU lifetime slots.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, mpsc};

use kjerag_media::{FrameStamp, Frames};
use kjerag_meta::OrientationTrack;

use super::corrected::{CorrectedFrame, CorrectionSequence};
use super::panorama_ingest::{prepare_correction_input, validate_reframe};
use super::{
    ResidentCameraProfile, ResidentCaptureSession, native_lifecycle_event,
    native_lifecycle_probe_enabled,
};
use crate::draw_retirement::{DrawPermit, DrawRetirementError};
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
use crate::ready_wake::ReadyWake;
use crate::temporal_fusion::settings::Provider;
use crate::{Fallible, Reframe, Size};

use super::temporal_worker::{TemporalEpoch, TemporalJob, TemporalWorker};

const READY_CAPACITY: usize = 4;
const SOURCE_CAPACITY: usize = READY_CAPACITY;
const FINISH_OUTPUT_CAPACITY: usize = 3;

fn completed_prefix(readiness: impl IntoIterator<Item = Fallible<bool>>) -> Fallible<usize> {
    let mut completed = 0;
    for ready in readiness {
        if !ready? {
            break;
        }
        completed += 1;
    }
    Ok(completed)
}

/// Quarter-field review candidate. Retain the whole 2:1 sphere while rounding
/// raster height down to a complete motion block footprint. Source planes,
/// map/colour cadence and the direct viewport stay at their original sizes.
fn quarter_correction_review_size(source_height: u32) -> [u32; 2] {
    let height = source_height / 4 / 32 * 32;
    [height * 2, height]
}

pub(super) struct FilteredSession {
    resident: Arc<ResidentCaptureSession>,
    temporal: Arc<TemporalWorker>,
    epoch: Arc<TemporalEpoch>,
}

enum Pending {
    Source {
        stamp: FrameStamp,
        previous: Option<FrameStamp>,
        output_capacity: usize,
    },
    Finish,
}

enum TemporalPending {
    Source {
        stamp: FrameStamp,
        output_capacity: usize,
    },
    Finish,
}

struct State {
    session: Option<Arc<FilteredSession>>,
    // Queued decoded work reserves output capacity, not a GPU lifetime slot.
    // The worker acquires that slot only when it begins the source transaction.
    stitch_pending: VecDeque<Pending>,
    queued: VecDeque<Work>,
    worker_running: bool,
    temporal_pending: VecDeque<TemporalPending>,
    accepted: Option<FrameStamp>,
    accepted_sources: usize,
    expected: VecDeque<FrameStamp>,
    ready: VecDeque<Arc<CorrectedFrame>>,
    installed: Option<Arc<CorrectedFrame>>,
    retired: bool,
    finish_requested: bool,
    finished: bool,
    failure: Option<String>,
    due_waiter: Option<(FrameStamp, ReadyWake)>,
    progress_wake: Option<ReadyWake>,
}

impl State {
    fn new() -> Self {
        Self {
            session: None,
            stitch_pending: VecDeque::with_capacity(SOURCE_CAPACITY),
            queued: VecDeque::with_capacity(SOURCE_CAPACITY),
            worker_running: false,
            temporal_pending: VecDeque::with_capacity(SOURCE_CAPACITY),
            accepted: None,
            accepted_sources: 0,
            expected: VecDeque::new(),
            ready: VecDeque::with_capacity(READY_CAPACITY),
            installed: None,
            retired: false,
            finish_requested: false,
            finished: false,
            failure: None,
            due_waiter: None,
            progress_wake: None,
        }
    }
}

struct FilteredSource {
    frames: Arc<Frames>,
    reframe: Reframe,
    stamp: FrameStamp,
    size: Size,
}

enum Work {
    Source(Box<FilteredSource>),
    Finish,
}

/// One capture actor, not one channel job per camera frame. Payloads in State
/// hold no Arc back to their owner, so an idle facade cannot retain itself.
pub(super) struct FilteredJob {
    owner: Arc<FilteredCaptureInner>,
    session: Arc<FilteredSession>,
}

impl FilteredJob {
    pub(super) fn owner(&self) -> Arc<FilteredCaptureInner> {
        Arc::clone(&self.owner)
    }
}

pub(super) struct FilteredCaptureInner {
    profile: Arc<ResidentCameraProfile>,
    orientation: OrientationTrack,
    provider: Provider,
    field_size: [u32; 2],
    state: Mutex<State>,
}

/// One source-ordered filtered capture. Clones share chronology and outputs.
#[derive(Clone)]
pub(crate) struct FilteredCaptureFacade {
    inner: Arc<FilteredCaptureInner>,
}

impl FilteredCaptureFacade {
    pub(crate) const READY_CAPACITY: usize = READY_CAPACITY;

    pub(crate) fn is_attached(&self) -> Fallible<bool> {
        let state = self.state()?;
        self.ensure_healthy(&state)?;
        Ok(state.session.is_some())
    }

    pub(crate) fn set_progress_wake(&self, wake: &ReadyWake) -> Fallible<()> {
        let session = {
            let mut state = self.state()?;
            self.ensure_healthy(&state)?;
            state.progress_wake = Some(wake.clone());
            state.session.clone()
        };
        if let Some(session) = session {
            session.resident.worker.set_progress_wake(wake);
            session.temporal.set_progress_wake(wake);
        }
        Ok(())
    }

    /// Construct the CPU owner. GPU resources remain lazy until [`Self::attach`].
    pub(crate) fn new(
        profile: Arc<ResidentCameraProfile>,
        orientation: OrientationTrack,
        provider: Provider,
    ) -> Fallible<Self> {
        let source = profile.source_size;
        if source.width == 0
            || source.height == 0
            || source.width != source.height
            || !source.height.is_multiple_of(2)
        {
            return Err(format!(
                "temporal correction needs an even square nonzero source, got {} by {}",
                source.width, source.height
            )
            .into());
        }
        Ok(Self {
            inner: Arc::new(FilteredCaptureInner {
                profile,
                orientation,
                provider,
                field_size: quarter_correction_review_size(source.height),
                state: Mutex::new(State::new()),
            }),
        })
    }

    /// Create a fresh decode epoch, sharing immutable GPU pipelines when the
    /// original owner was already attached.
    pub(crate) fn restart(&self) -> Fallible<Self> {
        let (old_session, progress_wake) = {
            let state = self.state()?;
            if let Some(error) = &state.failure {
                return Err(error.clone().into());
            }
            (state.session.clone(), state.progress_wake.clone())
        };
        let session = old_session
            .as_ref()
            .map(|old| {
                let resident = Arc::new(old.resident.restarted()?);
                let stream = CorrectionSequence::new(
                    resident.context.device(),
                    resident.context.queue(),
                    self.inner.field_size,
                    self.inner.provider.restarted(),
                )?;
                let epoch = Arc::new(TemporalEpoch::new(stream));
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Arc::new(FilteredSession {
                    resident,
                    temporal: Arc::clone(&old.temporal),
                    epoch,
                }))
            })
            .transpose()?;
        let mut state = State::new();
        state.session = session;
        state.progress_wake = progress_wake;
        // The installed old picture remains usable during seeking, but no
        // unpublished source/correction should accumulate across old epochs.
        // Cancellation never blocks the UI behind the temporal worker.
        {
            let mut previous = self.state()?;
            commit_restart(&mut previous)?;
        }
        if let Some(old) = old_session {
            old.epoch.cancel();
        }
        Ok(Self {
            inner: Arc::new(FilteredCaptureInner {
                profile: Arc::clone(&self.inner.profile),
                orientation: self.inner.orientation.clone(),
                provider: self.inner.provider.restarted(),
                field_size: self.inner.field_size,
                state: Mutex::new(state),
            }),
        })
    }

    /// Attach the exact Scene device/queue pair without admitting a source.
    pub(crate) fn attach(&self, context: OneXsGpuContext) -> Fallible<()> {
        let existing = {
            let state = self.state()?;
            self.ensure_healthy(&state)?;
            state.session.clone()
        };
        if let Some(session) = existing {
            return session
                .resident
                .ensure_renderer(&context, wgpu::TextureFormat::Rgba8Unorm);
        }
        let resident = Arc::new(ResidentCaptureSession::new(
            context,
            wgpu::TextureFormat::Rgba8Unorm,
            &self.inner.profile,
            self.inner.orientation.clone(),
        )?);
        let stream = CorrectionSequence::new(
            resident.context.device(),
            resident.context.queue(),
            self.inner.field_size,
            self.inner.provider.restarted(),
        )?;
        let temporal = Arc::new(TemporalWorker::new()?);
        let epoch = Arc::new(TemporalEpoch::new(stream));
        let candidate = Arc::new(FilteredSession {
            resident,
            temporal,
            epoch,
        });
        let mut state = self.state()?;
        self.ensure_healthy(&state)?;
        if let Some(session) = &state.session {
            session
                .resident
                .ensure_renderer(&candidate.resident.context, wgpu::TextureFormat::Rgba8Unorm)
        } else {
            state.session = Some(candidate.clone());
            let wake = state.progress_wake.clone();
            drop(state);
            if let Some(wake) = wake {
                candidate.resident.worker.set_progress_wake(&wake);
                candidate.temporal.set_progress_wake(&wake);
                wake.notify();
            }
            Ok(())
        }
    }

    /// Admit one contiguous decoded source without waiting for worker or GPU.
    /// The shell can query this before constructing a source's projection;
    /// try_submit still rechecks under the admission lock.
    pub(crate) fn wants_source(&self) -> Fallible<bool> {
        let state = self.state()?;
        self.ensure_healthy(&state)?;
        Ok(!state.finished && !state.finish_requested && source_admission_available(&state))
    }

    /// Admit one contiguous decoded source without waiting for worker or GPU.
    pub(crate) fn try_submit(&self, frames: Arc<Frames>, reframe: Reframe) -> Fallible<bool> {
        // Ordinary backpressure is a CPU queue decision. In particular, mouse
        // events never poll the graphics device or reserve GPU lifetimes.
        // The actor acquires those only when queued work begins execution.
        {
            let state = self.state()?;
            self.ensure_healthy(&state)?;
            if state.finished || state.finish_requested {
                return Err("filtered capture received a source after finish".into());
            }
            if !source_admission_available(&state) {
                return Ok(false);
            }
        }
        validate_reframe(&frames, &reframe)?;
        let stamp = frames.stamp();
        // The same source metadata must govern both panorama decoding and the
        // temporal representation. Do not maintain a second coefficient table.
        let reframe = reframe.with_samples(frames.samples);
        let session = self.attached_session()?;
        let kick = {
            let mut state = self.state()?;
            self.ensure_healthy(&state)?;
            if state.finished || state.finish_requested {
                return Err("filtered capture received a source after finish".into());
            }
            let output_capacity = source_output_capacity(state.accepted_sources);
            if !source_admission_available(&state) {
                return Ok(false);
            }
            if let Some(previous) = state.accepted.as_ref() {
                super::validate_resident_sequence(Some(previous), &stamp)?;
            }
            let accepted_sources = state
                .accepted_sources
                .checked_add(1)
                .ok_or("filtered source count overflowed")?;
            let previous = state.accepted.replace(stamp.clone());
            state.accepted_sources = accepted_sources;
            state.expected.push_back(stamp.clone());
            state.stitch_pending.push_back(Pending::Source {
                stamp: stamp.clone(),
                previous,
                output_capacity,
            });
            state
                .queued
                .push_back(Work::Source(Box::new(FilteredSource {
                    frames,
                    reframe,
                    stamp,
                    size: Size::new(self.inner.field_size[0], self.inner.field_size[1]),
                })));
            !std::mem::replace(&mut state.worker_running, true)
        };
        if !kick {
            return Ok(true);
        }
        let job = FilteredJob {
            owner: Arc::clone(&self.inner),
            session: Arc::clone(&session),
        };
        match session.resident.worker.try_kick_filtered(job) {
            Ok(()) => Ok(true),
            Err(mpsc::TrySendError::Full(_)) => {
                self.inner.cancel_kick()?;
                Ok(false)
            }
            Err(mpsc::TrySendError::Disconnected(job)) => {
                let owner = job.owner();
                let message = "ONE X2 stitch worker stopped before accepting filtered capture";
                owner.fail_worker(message);
                Err(message.into())
            }
        }
    }

    /// Schedule the temporal tail without blocking. Up to three outputs may
    /// be published, so the bounded FIFO must have room for all of them.
    pub(crate) fn try_finish(&self) -> Fallible<bool> {
        let session = self.attached_session()?;
        let kick = {
            let mut state = self.state()?;
            self.ensure_healthy(&state)?;
            if state.finished || state.finish_requested {
                return Ok(true);
            }
            if !state.stitch_pending.is_empty()
                || !state.temporal_pending.is_empty()
                || state.ready.len() + FINISH_OUTPUT_CAPACITY > READY_CAPACITY
            {
                return Ok(false);
            }
            state.finish_requested = true;
            state.stitch_pending.push_back(Pending::Finish);
            state.queued.push_back(Work::Finish);
            !std::mem::replace(&mut state.worker_running, true)
        };
        if !kick {
            return Ok(true);
        }
        let job = FilteredJob {
            owner: Arc::clone(&self.inner),
            session: Arc::clone(&session),
        };
        match session.resident.worker.try_kick_filtered(job) {
            Ok(()) => Ok(true),
            Err(mpsc::TrySendError::Full(_)) => {
                self.inner.cancel_kick()?;
                Ok(false)
            }
            Err(mpsc::TrySendError::Disconnected(job)) => {
                let owner = job.owner();
                let message = "ONE X2 stitch worker stopped before accepting filtered finish";
                owner.fail_worker(message);
                Err(message.into())
            }
        }
    }

    pub(crate) fn accepted_stamp(&self) -> Fallible<Option<FrameStamp>> {
        let state = self.state()?;
        self.ensure_healthy(&state)?;
        Ok(state.accepted.clone())
    }

    /// All real inputs for the pending output are admitted, not necessarily
    /// processed. This is not completion proof or restart lead. If the current
    /// output is complete, the missing work is its next source instead.
    pub(crate) fn has_output_inputs(
        &self,
        stamp: &FrameStamp,
        current_ready: bool,
    ) -> Fallible<bool> {
        let state = self.state()?;
        self.ensure_healthy(&state)?;
        Ok(output_inputs_admitted(&state, stamp, current_ready))
    }

    pub(crate) fn acknowledged(&self, stamp: &FrameStamp) -> Fallible<bool> {
        let state = self.state()?;
        self.ensure_healthy(&state)?;
        Ok(state
            .installed
            .as_ref()
            .is_some_and(|output| output.frame() == stamp))
    }

    /// Install only the exact FIFO-front result. A later due stamp cannot
    /// silently discard an earlier filtered picture.
    pub(crate) fn install_due(&self, stamp: &FrameStamp) -> Fallible<Option<Arc<CorrectedFrame>>> {
        let mut state = self.state()?;
        self.ensure_healthy(&state)?;
        if let Some(installed) = &state.installed
            && installed.frame() == stamp
        {
            return Ok(Some(Arc::clone(installed)));
        }
        let Some(front) = state.ready.front() else {
            if state.finished {
                return Err(format!(
                    "filtered capture finished without an output for frame {}",
                    stamp.index()
                )
                .into());
            }
            return Ok(None);
        };
        if front.frame() != stamp {
            return Err(format!(
                "filtered output frame {} precedes requested frame {}",
                front.frame().index(),
                stamp.index()
            )
            .into());
        }
        if !front.completion().ready()? {
            return Ok(None);
        }
        let output = state
            .ready
            .pop_front()
            .expect("checked filtered FIFO front");
        state.installed = Some(Arc::clone(&output));
        let progress = state.progress_wake.clone();
        drop(state);
        if let Some(progress) = progress {
            progress.notify();
        }
        Ok(Some(output))
    }

    pub(crate) fn installed_stamp(&self) -> Fallible<Option<FrameStamp>> {
        let state = self.state()?;
        self.ensure_healthy(&state)?;
        Ok(state
            .installed
            .as_ref()
            .map(|output| output.frame().clone()))
    }

    /// Completed contiguous successors of the exact installed owner. Pending
    /// source work is not playback lead, and another epoch cannot count.
    pub(crate) fn ready_successors(&self, stamp: &FrameStamp) -> Fallible<usize> {
        let state = self.state()?;
        self.ensure_healthy(&state)?;
        if state
            .installed
            .as_ref()
            .is_none_or(|output| output.frame() != stamp)
        {
            return Ok(0);
        }
        completed_prefix(state.ready.iter().map(|output| output.completion().ready()))
    }

    /// Preserve the last complete picture for a terminal-error screenshot.
    /// Unlike observed-state queries, this deliberately does not mask that
    /// already-installed resource with a later sticky worker failure.
    #[cfg(test)]
    pub(crate) fn installed(&self) -> Fallible<Option<Arc<CorrectedFrame>>> {
        Ok(self.state()?.installed.clone())
    }

    pub(crate) fn is_finished(&self) -> Fallible<bool> {
        let state = self.state()?;
        self.ensure_healthy(&state)?;
        Ok(state.finished)
    }

    /// Register one exact waiter only while worker progress is outstanding.
    pub(crate) fn wait_for_frame(&self, stamp: &FrameStamp, wake: &ReadyWake) -> Fallible<bool> {
        if !wake.listening() {
            return Ok(false);
        }
        let mut state = self.state()?;
        self.ensure_healthy(&state)?;
        if state
            .installed
            .as_ref()
            .is_some_and(|output| output.frame() == stamp)
        {
            return Ok(false);
        }
        if let Some(front) = state.ready.front() {
            if front.frame() == stamp {
                if front.completion().ready()? {
                    return Ok(false);
                }
                state.due_waiter = Some((stamp.clone(), wake.clone()));
                return Ok(true);
            }
            return Err(format!(
                "filtered output frame {} precedes requested frame {}",
                front.frame().index(),
                stamp.index()
            )
            .into());
        }
        if state.finished {
            return Err(format!(
                "filtered capture finished without an output for frame {}",
                stamp.index()
            )
            .into());
        }
        if (state.stitch_pending.is_empty() && state.temporal_pending.is_empty())
            || !state.expected.iter().any(|expected| expected == stamp)
        {
            return Ok(false);
        }
        state.due_waiter = Some((stamp.clone(), wake.clone()));
        Ok(true)
    }

    pub(crate) fn same_capture(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    #[cfg(test)]
    pub(crate) fn pause_stitch_for_test(&self) -> super::resident_worker::StitchPause {
        self.attached_session()
            .unwrap()
            .resident
            .worker
            .pause_for_test()
    }

    #[cfg(test)]
    pub(crate) fn work_idle_for_test(&self) -> Fallible<bool> {
        let state = self.state()?;
        self.ensure_healthy(&state)?;
        Ok(!state.worker_running
            && state.queued.is_empty()
            && state.stitch_pending.is_empty()
            && state.temporal_pending.is_empty())
    }

    #[cfg(test)]
    pub(crate) fn assert_restart_ownership(&self, previous: &Self) {
        let current = self.attached_session().unwrap();
        let previous = previous.attached_session().unwrap();
        assert!(Arc::ptr_eq(&current.temporal, &previous.temporal));
        assert!(!Arc::ptr_eq(&current.epoch, &previous.epoch));
        assert!(previous.epoch.is_canceled());
    }

    /// Force the actual terminal-worker path for Scene ownership regressions.
    #[cfg(test)]
    pub(crate) fn fail_for_test(&self, message: &str, poison_state: bool) {
        if poison_state {
            let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _state = self.inner.state.lock().unwrap();
                panic!("injected filtered capture state poison");
            }));
            assert!(
                poisoned.is_err(),
                "filtered state poison injection returned"
            );
        }
        self.inner.fail_worker(message);
    }

    /// Assert terminal cleanup without routing through the healthy-state API.
    #[cfg(test)]
    pub(crate) fn assert_history_released_for_test(&self) {
        let state = match self.inner.state.lock() {
            Ok(state) => state,
            Err(poison) => poison.into_inner(),
        };
        let session = state
            .session
            .as_ref()
            .expect("filtered history assertion needs an attached session");
        session.epoch.assert_released_for_test();
    }

    /// Prove a real Scene has unpublished work before a failure-release check.
    #[cfg(test)]
    pub(crate) fn assert_unpublished_history_for_test(&self) {
        let state = self
            .inner
            .state
            .lock()
            .expect("filtered history precondition state is poisoned");
        assert!(
            !state.expected.is_empty(),
            "filtered history precondition has no unpublished source"
        );
        let session = state
            .session
            .as_ref()
            .expect("filtered history precondition needs an attached session");
        session.epoch.assert_active_for_test();
    }

    /// Hold the actual populated epoch history across a concurrent state change.
    #[cfg(test)]
    pub(crate) fn with_locked_history_for_test<T>(&self, work: impl FnOnce() -> T) -> T {
        let epoch = {
            let state = match self.inner.state.lock() {
                Ok(state) => state,
                Err(poison) => poison.into_inner(),
            };
            Arc::clone(
                &state
                    .session
                    .as_ref()
                    .expect("filtered history lock needs an attached session")
                    .epoch,
            )
        };
        epoch.with_locked_history_for_test(work)
    }

    fn attached_session(&self) -> Fallible<Arc<FilteredSession>> {
        let state = self.state()?;
        self.ensure_healthy(&state)?;
        state
            .session
            .clone()
            .ok_or_else(|| "filtered capture has no attached GPU context".into())
    }

    fn ensure_healthy(&self, state: &State) -> Fallible<()> {
        if let Some(error) = &state.failure {
            Err(error.clone().into())
        } else {
            Ok(())
        }
    }

    fn state(&self) -> Fallible<std::sync::MutexGuard<'_, State>> {
        self.inner.state()
    }
}

impl fmt::Debug for FilteredCaptureFacade {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output
            .debug_struct("FilteredCaptureFacade")
            .field("identity", &Arc::as_ptr(&self.inner))
            .finish_non_exhaustive()
    }
}

impl FilteredCaptureInner {
    fn state(&self) -> Fallible<std::sync::MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| "filtered capture state is poisoned".into())
    }

    /// GPU completion wakes the exact waiter even if both CPU workers have
    /// become idle. It does not install or select the finished picture.
    pub(super) fn output_completed(&self) -> Fallible<()> {
        let (wake, progress) = {
            let mut state = self.state()?;
            (
                state.due_waiter.take().map(|(_, wake)| wake),
                state.progress_wake.clone(),
            )
        };
        if let Some(wake) = wake {
            wake.notify();
        }
        if let Some(progress) = progress {
            progress.notify();
        }
        Ok(())
    }

    fn ensure_stitch_source(&self, stamp: &FrameStamp) -> Fallible<()> {
        let state = self.state()?;
        ensure_healthy_state(&state)?;
        if matches!(
            state.stitch_pending.front(),
            Some(Pending::Source { stamp: pending, .. }) if pending == stamp
        ) {
            Ok(())
        } else {
            Err("filtered panorama worker source differs from its pending request".into())
        }
    }

    fn take_work(&self) -> Fallible<Option<Work>> {
        let mut state = self.state()?;
        ensure_healthy_state(&state)?;
        if state.retired {
            state.queued.clear();
        }
        let work = state.queued.pop_front();
        if work.is_none() {
            // Admission and actor exit use this same lock: a successor is
            // either already owned by this actor or kicks its replacement.
            state.worker_running = false;
        }
        Ok(work)
    }

    pub(super) fn ensure_temporal_source(&self, stamp: &FrameStamp) -> Fallible<()> {
        let state = self.state()?;
        ensure_healthy_state(&state)?;
        if matches!(
            state.temporal_pending.front(),
            Some(TemporalPending::Source { stamp: pending, .. }) if pending == stamp
        ) {
            Ok(())
        } else {
            Err("filtered temporal source differs from its pending request".into())
        }
    }

    pub(super) fn ensure_temporal_finish(&self) -> Fallible<()> {
        let state = self.state()?;
        ensure_healthy_state(&state)?;
        if matches!(
            state.temporal_pending.front(),
            Some(TemporalPending::Finish)
        ) {
            Ok(())
        } else {
            Err("filtered temporal finish differs from its pending request".into())
        }
    }

    fn handoff_source(&self, stamp: &FrameStamp) -> Fallible<Option<ReadyWake>> {
        let mut state = self.state()?;
        ensure_healthy_state(&state)?;
        let Some(Pending::Source {
            stamp: pending,
            output_capacity,
            ..
        }) = state.stitch_pending.pop_front()
        else {
            return Err("filtered stitch handoff has no pending source".into());
        };
        if &pending != stamp {
            return Err("filtered stitch handoff differs from its pending source".into());
        }
        state.temporal_pending.push_back(TemporalPending::Source {
            stamp: pending,
            output_capacity,
        });
        let wake = state.due_waiter.take().map(|(_, wake)| wake);
        let progress = state.progress_wake.clone();
        drop(state);
        if let Some(progress) = progress {
            progress.notify();
        }
        Ok(wake)
    }

    fn handoff_finish(&self) -> Fallible<Option<ReadyWake>> {
        let mut state = self.state()?;
        ensure_healthy_state(&state)?;
        if !matches!(state.stitch_pending.pop_front(), Some(Pending::Finish)) {
            return Err("filtered stitch handoff has no pending finish".into());
        }
        state.temporal_pending.push_back(TemporalPending::Finish);
        let wake = state.due_waiter.take().map(|(_, wake)| wake);
        let progress = state.progress_wake.clone();
        drop(state);
        if let Some(progress) = progress {
            progress.notify();
        }
        Ok(wake)
    }

    pub(super) fn complete_source(
        &self,
        stamp: &FrameStamp,
        outputs: Vec<CorrectedFrame>,
    ) -> Fallible<()> {
        self.complete_temporal(PendingKind::Source(stamp), outputs)
    }

    pub(super) fn complete_finish(&self, outputs: Vec<CorrectedFrame>) -> Fallible<()> {
        self.complete_temporal(PendingKind::Finish, outputs)
    }

    fn complete_temporal(
        &self,
        pending: PendingKind,
        outputs: Vec<CorrectedFrame>,
    ) -> Fallible<()> {
        let (wake, progress) = {
            let mut state = self.state()?;
            ensure_healthy_state(&state)?;
            if state.retired {
                // A seek won the race with completion. Drop these GPU-owned
                // outputs without publishing them into the old ready queue.
                return Ok(());
            }
            let matches = match (state.temporal_pending.front(), pending) {
                (Some(TemporalPending::Source { stamp, .. }), PendingKind::Source(done)) => {
                    stamp == done
                }
                (Some(TemporalPending::Finish), PendingKind::Finish) => true,
                _ => false,
            };
            if !matches {
                return Err("filtered worker completion differs from its pending request".into());
            }
            let capacity = match state.temporal_pending.front() {
                Some(TemporalPending::Source {
                    output_capacity, ..
                }) => *output_capacity,
                Some(TemporalPending::Finish) => FINISH_OUTPUT_CAPACITY,
                None => 0,
            };
            if outputs.len() > capacity || state.ready.len() + outputs.len() > READY_CAPACITY {
                return Err("filtered worker exceeded its four-picture ready FIFO".into());
            }
            for output in outputs {
                let expected = state
                    .expected
                    .pop_front()
                    .ok_or("filtered worker produced an unaccepted frame")?;
                if output.frame() != &expected {
                    return Err("filtered worker output order differs from accepted sources".into());
                }
                state.ready.push_back(Arc::new(output));
            }
            if matches!(pending, PendingKind::Finish) {
                state.finished = true;
            }
            state.temporal_pending.pop_front();
            (
                state.due_waiter.take().map(|(_, wake)| wake),
                state.progress_wake.clone(),
            )
        };
        if let Some(wake) = wake {
            wake.notify();
        }
        if let Some(progress) = progress {
            progress.notify();
        }
        Ok(())
    }

    fn cancel_kick(&self) -> Fallible<()> {
        let mut state = self.state()?;
        ensure_healthy_state(&state)?;
        // A refused channel send belongs to the latest admission, not to the
        // source already executing at the front of the worker's FIFO.
        state.worker_running = false;
        match (state.queued.pop_back(), state.stitch_pending.pop_back()) {
            (
                Some(Work::Source(job)),
                Some(Pending::Source {
                    stamp, previous, ..
                }),
            ) if stamp == job.stamp => {
                if state.expected.pop_back().as_ref() != Some(&stamp) {
                    return Err("filtered backpressure changed accepted source order".into());
                }
                state.accepted = previous;
                state.accepted_sources = state
                    .accepted_sources
                    .checked_sub(1)
                    .ok_or("filtered source count underflowed during backpressure")?;
            }
            (Some(Work::Finish), Some(Pending::Finish)) => {
                state.finish_requested = false;
            }
            _ => return Err("filtered backpressure changed its pending request".into()),
        }
        // This is a failed-admission rollback, not newly available capacity.
        // Waking here on channel Full would create a self-sustaining retry
        // loop. The shared worker notifies when it actually dequeues a job.
        Ok(())
    }

    pub(super) fn fail_worker(&self, message: &str) {
        let (wake, report, epoch) = record_worker_failure(&self.state, message);
        if let Some(epoch) = epoch {
            epoch.cancel();
        }
        if let Some(wake) = wake {
            wake.notify();
        }
        let progress = self
            .state()
            .ok()
            .and_then(|state| state.progress_wake.clone());
        if let Some(progress) = progress {
            progress.notify();
        }
        if report {
            eprintln!("{message}");
        }
    }
}

/// Finalize the old façade only after the replacement epoch is ready.
/// This is the restart transaction's failure-versus-retirement boundary.
fn commit_restart(previous: &mut State) -> Fallible<()> {
    ensure_healthy_state(previous)?;
    previous.retired = true;
    previous.queued.clear();
    previous.ready.clear();
    previous.due_waiter = None;
    Ok(())
}

fn record_worker_failure(
    state: &Mutex<State>,
    message: &str,
) -> (Option<ReadyWake>, bool, Option<Arc<TemporalEpoch>>) {
    let mut guard = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (wake, report) = record_failure(&mut guard, message);
    let epoch = guard
        .session
        .as_ref()
        .map(|session| Arc::clone(&session.epoch));
    // The state is terminal and normalized before another caller can acquire
    // it. Preserve the underlying worker error, not poison. This is a no-op
    // for an ordinarily acquired mutex.
    state.clear_poison();
    (wake, report, epoch)
}

fn record_failure(state: &mut State, message: &str) -> (Option<ReadyWake>, bool) {
    state.stitch_pending.clear();
    state.queued.clear();
    state.worker_running = false;
    state.temporal_pending.clear();
    state.ready.clear();
    let report = state.failure.is_none();
    if report {
        state.failure = Some(message.to_owned());
    }
    (state.due_waiter.take().map(|(_, wake)| wake), report)
}

fn ensure_healthy_state(state: &State) -> Fallible<()> {
    if let Some(error) = &state.failure {
        Err(error.clone().into())
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum PendingKind<'a> {
    Source(&'a FrameStamp),
    Finish,
}

pub(super) fn service_filtered(job: FilteredJob) -> Fallible<()> {
    let FilteredJob { owner, session } = job;
    while let Some(work) = owner.take_work()? {
        if session.epoch.is_canceled() {
            return Ok(());
        }
        match work {
            Work::Source(job) => {
                owner.ensure_stitch_source(&job.stamp)?;
                let started = native_lifecycle_probe_enabled().then(std::time::Instant::now);
                native_lifecycle_event("filtered-worker-start", &job.stamp, None);
                let Some(permit) = reserve_source_permit(&session)? else {
                    return Ok(());
                };
                let panorama = prepare_correction_input(
                    &session.resident,
                    job.frames,
                    job.reframe,
                    &job.stamp,
                    job.size,
                    permit,
                )?;
                if let Some(started) = started {
                    // This ends at panorama submission, not GPU completion.
                    native_lifecycle_event(
                        "filtered-panorama-submitted",
                        &job.stamp,
                        Some(started.elapsed()),
                    );
                }
                let wake = owner.handoff_source(&job.stamp)?;
                let temporal = TemporalJob::Push {
                    owner: Arc::clone(&owner),
                    epoch: Arc::clone(&session.epoch),
                    panorama: Box::new(panorama),
                    started,
                };
                let sent = session.temporal.send(temporal);
                if let Some(wake) = wake {
                    wake.notify();
                }
                sent?;
            }
            Work::Finish => {
                let wake = owner.handoff_finish()?;
                let sent = session.temporal.send(TemporalJob::Finish {
                    owner: Arc::clone(&owner),
                    epoch: Arc::clone(&session.epoch),
                });
                if let Some(wake) = wake {
                    wake.notify();
                }
                sent?;
            }
        }
    }
    Ok(())
}

/// CPU admission does not consume either GPU lifetime slot. Only the actor
/// waits for source-snapshot retirement, with no UI mutex held or GPU fence
/// wait. Canceling the epoch interrupts this wait before another import.
fn reserve_source_permit(session: &FilteredSession) -> Fallible<Option<DrawPermit>> {
    loop {
        if session.epoch.is_canceled() {
            return Ok(None);
        }
        session.resident.retirements.poll()?;
        match session.resident.retirements.reserve() {
            Ok(permit) => return Ok(Some(permit)),
            Err(DrawRetirementError::Full) => {
                std::thread::park_timeout(std::time::Duration::from_micros(100));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn source_output_capacity(accepted_sources: usize) -> usize {
    match accepted_sources {
        0..=5 => 0,
        6 => READY_CAPACITY,
        _ => 1,
    }
}

fn output_inputs_admitted(state: &State, stamp: &FrameStamp, current_ready: bool) -> bool {
    use crate::temporal_fusion::stream::{CENTER, SOURCES};
    if state.accepted_sources < SOURCES {
        return false;
    }
    let Some(last) = state.accepted.as_ref() else {
        return false;
    };
    if !last.same_decode_epoch(stamp) {
        return false;
    }
    let Some(ahead) = last.index().checked_sub(stamp.index()) else {
        return false;
    };
    // A queued finish owns the complete, clipped EOF window. Ordinary work
    // needs the actual seven-source stream's three future sources, plus one
    // when it is the successor of an already-complete current output.
    state.finish_requested || ahead >= CENTER as u64 + u64::from(current_ready)
}

fn temporal_output_capacity(pending: &VecDeque<TemporalPending>) -> usize {
    pending
        .iter()
        .map(|pending| match pending {
            TemporalPending::Source {
                output_capacity, ..
            } => *output_capacity,
            TemporalPending::Finish => FINISH_OUTPUT_CAPACITY,
        })
        .sum()
}

fn reserved_output_capacity(state: &State) -> usize {
    temporal_output_capacity(&state.temporal_pending)
        + state
            .stitch_pending
            .iter()
            .map(|pending| match pending {
                Pending::Source {
                    output_capacity, ..
                } => *output_capacity,
                Pending::Finish => FINISH_OUTPUT_CAPACITY,
            })
            .sum::<usize>()
}

fn outstanding_source_count(state: &State) -> usize {
    state
        .stitch_pending
        .iter()
        .filter(|pending| matches!(pending, Pending::Source { .. }))
        .count()
        + state
            .temporal_pending
            .iter()
            .filter(|pending| matches!(pending, TemporalPending::Source { .. }))
            .count()
}

fn source_admission_available(state: &State) -> bool {
    outstanding_source_count(state) < SOURCE_CAPACITY
        && state.ready.len()
            + reserved_output_capacity(state)
            + source_output_capacity(state.accepted_sources)
            <= READY_CAPACITY
}

#[cfg(test)]
mod stage_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn output_input_ownership_requires_startup_and_exact_future_sources() {
        let first = stamp(10, None);
        let current = stamp(13, Some(&first));
        let mut state = State::new();
        state.accepted_sources = 6;
        state.accepted = Some(stamp(15, Some(&first)));
        assert!(!output_inputs_admitted(&state, &first, false));
        state.accepted_sources = 7;
        state.accepted = Some(stamp(16, Some(&first)));
        assert!(output_inputs_admitted(&state, &current, false));
        assert!(!output_inputs_admitted(&state, &current, true));
        assert!(
            state.ready.is_empty(),
            "input ownership is not GPU completion"
        );
        state.accepted_sources = 8;
        state.accepted = Some(stamp(17, Some(&first)));
        assert!(output_inputs_admitted(&state, &current, true));
        let another_epoch = stamp(13, None);
        assert!(!output_inputs_admitted(&state, &another_epoch, false));
        state.accepted = Some(stamp(12, Some(&first)));
        assert!(!output_inputs_admitted(&state, &current, false));
    }

    #[test]
    fn admitted_finish_owns_the_clipped_tail_but_not_another_epoch_or_future() {
        let first = stamp(10, None);
        let current = stamp(15, Some(&first));
        let mut state = State::new();
        state.accepted_sources = 7;
        state.accepted = Some(stamp(16, Some(&first)));
        assert!(!output_inputs_admitted(&state, &current, false));
        state.finish_requested = true;
        assert!(output_inputs_admitted(&state, &current, false));
        assert!(output_inputs_admitted(&state, &current, true));
        assert!(!output_inputs_admitted(&state, &stamp(15, None), false));
        assert!(!output_inputs_admitted(
            &state,
            &stamp(17, Some(&first)),
            false
        ));
    }

    #[test]
    fn recovery_lead_counts_only_the_completed_fifo_prefix() {
        use crate::gpu_completion::CompletionStatus;
        let statuses: Vec<_> = (0..4)
            .map(|_| CompletionStatus::pending_for_test())
            .collect();
        let count = || completed_prefix(statuses.iter().map(CompletionStatus::ready)).unwrap();
        assert_eq!(count(), 0, "queued work is not completed playback lead");
        statuses[2].finish_for_test(Ok(()));
        assert_eq!(
            count(),
            0,
            "a later completion cannot skip an unfinished predecessor"
        );
        statuses[0].finish_for_test(Ok(()));
        assert_eq!(count(), 1);
        statuses[1].finish_for_test(Ok(()));
        assert_eq!(count(), 3);
        statuses[3].finish_for_test(Ok(()));
        assert_eq!(count(), 4);
    }

    #[test]
    fn recovery_lead_preserves_the_raw_completion_error() {
        use crate::gpu_completion::CompletionStatus;
        let failed = CompletionStatus::pending_for_test();
        failed.finish_for_test(Err("exact recovery completion failure".into()));
        assert_eq!(
            completed_prefix([failed.ready()]).unwrap_err().to_string(),
            "exact recovery completion failure"
        );
    }

    fn stamp(index: u64, previous: Option<&FrameStamp>) -> FrameStamp {
        FrameStamp::for_test(index, Duration::from_millis(index), previous)
    }

    #[test]
    fn output_reservations_cover_startup_steady_and_finish_batches() {
        assert_eq!(
            (0..10).map(source_output_capacity).collect::<Vec<_>>(),
            [0, 0, 0, 0, 0, 0, 4, 1, 1, 1]
        );

        let first = stamp(0, None);
        let second = stamp(1, Some(&first));
        let pending = VecDeque::from([
            TemporalPending::Source {
                stamp: first,
                output_capacity: 0,
            },
            TemporalPending::Source {
                stamp: second,
                output_capacity: 4,
            },
        ]);
        assert_eq!(temporal_output_capacity(&pending), READY_CAPACITY);
    }

    #[test]
    fn ready_capacity_rejects_a_second_promised_steady_output() {
        let first = stamp(0, None);
        let pending = VecDeque::from([TemporalPending::Source {
            stamp: first,
            output_capacity: 1,
        }]);
        let ready = 3;
        assert_eq!(ready + temporal_output_capacity(&pending), READY_CAPACITY);
        assert!(
            ready + temporal_output_capacity(&pending) + source_output_capacity(8) > READY_CAPACITY
        );
    }

    #[test]
    fn stitch_and_temporal_stages_share_the_bounded_cpu_work_capacity() {
        let first = stamp(0, None);
        let second = stamp(1, Some(&first));
        let mut state = State::new();
        state.temporal_pending.push_back(TemporalPending::Source {
            stamp: first,
            output_capacity: 1,
        });
        state.stitch_pending.push_back(Pending::Source {
            stamp: second,
            previous: None,
            output_capacity: 1,
        });
        assert_eq!(outstanding_source_count(&state), 2);
        assert_eq!(reserved_output_capacity(&state), 2);
        assert!(source_admission_available(&state));
        let third = stamp(2, state.accepted.as_ref());
        let fourth = stamp(3, Some(&third));
        for stamp in [third, fourth] {
            state.stitch_pending.push_back(Pending::Source {
                stamp,
                previous: None,
                output_capacity: 1,
            });
        }
        assert_eq!(outstanding_source_count(&state), SOURCE_CAPACITY);
        assert_eq!(reserved_output_capacity(&state), READY_CAPACITY);
        assert!(!source_admission_available(&state));
    }

    #[test]
    fn actor_can_admit_a_successor_before_its_first_stitch_handoff() {
        let first = stamp(0, None);
        let second = stamp(1, Some(&first));
        let mut state = State::new();
        state.accepted_sources = 1;
        state.worker_running = true;
        state.stitch_pending.push_back(Pending::Source {
            stamp: first,
            previous: None,
            output_capacity: 0,
        });
        assert!(source_admission_available(&state));
        state.accepted_sources += 1;
        state.stitch_pending.push_back(Pending::Source {
            stamp: second,
            previous: None,
            output_capacity: 0,
        });
        assert_eq!(outstanding_source_count(&state), 2);
        assert!(source_admission_available(&state));
        for index in 2..SOURCE_CAPACITY as u64 {
            state.accepted_sources += 1;
            state.stitch_pending.push_back(Pending::Source {
                stamp: stamp(index, None),
                previous: None,
                output_capacity: 0,
            });
        }
        assert_eq!(outstanding_source_count(&state), SOURCE_CAPACITY);
        assert!(!source_admission_available(&state));
    }

    #[test]
    fn queued_startup_output_reserves_the_whole_ready_fifo() {
        let mut state = State::new();
        state.accepted_sources = 7;
        state.stitch_pending.push_back(Pending::Source {
            stamp: stamp(6, None),
            previous: None,
            output_capacity: READY_CAPACITY,
        });
        assert_eq!(outstanding_source_count(&state), 1);
        assert!(!source_admission_available(&state));
    }

    #[test]
    fn failure_retires_queued_actor_work() {
        let mut state = State::new();
        state.worker_running = true;
        state.queued.push_back(Work::Finish);
        state.stitch_pending.push_back(Pending::Finish);
        record_failure(&mut state, "exact queued worker error");
        assert!(!state.worker_running);
        assert!(state.queued.is_empty());
        assert!(state.stitch_pending.is_empty());
    }

    #[test]
    fn concurrent_stage_failures_preserve_the_first_error() {
        let mut state = State::new();
        let expected = stamp(4, None);
        state.due_waiter = Some((expected, ReadyWake::default()));
        let (first_wake, first_reported) = record_failure(&mut state, "underlying GPU error");
        let (second_wake, second_reported) =
            record_failure(&mut state, "secondary pending mismatch");
        assert!(first_reported);
        assert!(first_wake.is_some(), "the first failure lost its waiter");
        assert!(!second_reported);
        assert!(second_wake.is_none(), "the waiter was returned twice");
        assert_eq!(state.failure.as_deref(), Some("underlying GPU error"));
    }

    #[test]
    fn restart_commit_rejects_a_failure_recorded_after_its_snapshot() {
        let mut previous = State::new();
        assert!(
            previous.failure.is_none(),
            "the restart snapshot was not healthy"
        );

        record_failure(&mut previous, "exact worker failure during restart");
        let error = commit_restart(&mut previous)
            .expect_err("restart masked a worker failure that won before retirement");

        assert_eq!(error.to_string(), "exact worker failure during restart");
        assert!(!previous.retired, "failed restart retired its old façade");
    }

    #[test]
    fn restart_commit_retires_a_healthy_state_and_clears_its_waiter() {
        let mut previous = State::new();
        previous.due_waiter = Some((stamp(4, None), ReadyWake::default()));

        commit_restart(&mut previous).unwrap();

        assert!(previous.retired);
        assert!(previous.due_waiter.is_none());
    }

    #[test]
    fn worker_failure_normalizes_poison_before_exposing_the_raw_error() {
        let state = Mutex::new(State::new());
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _state = state.lock().unwrap();
            panic!("exact panic while filtered state was locked");
        }));
        assert!(poisoned.is_err(), "state poison injection returned");

        let (_, reported, _) = record_worker_failure(&state, "exact worker panic");
        assert!(reported);
        let state = state
            .lock()
            .expect("terminal normalization left filtered state poisoned");
        assert_eq!(state.failure.as_deref(), Some("exact worker panic"));
    }

    #[test]
    fn quarter_review_raster_preserves_aspect_and_complete_motion_blocks() {
        for (height, expected) in [(3_840, [1_920, 960]), (2_880, [1_408, 704])] {
            let field = quarter_correction_review_size(height);
            assert_eq!(field, expected);
            assert_eq!(field[0], field[1] * 2);
            assert!(field.into_iter().all(|value| value.is_multiple_of(32)));
        }
    }
}
