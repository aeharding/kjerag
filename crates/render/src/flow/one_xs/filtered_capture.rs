//! Typed worker bridge from decoded resident sources to filtered panoramas.
//!
//! This owner is one decode epoch. Admission and exact-stamp presentation are
//! nonblocking. The existing stitch worker owns ordered panorama preparation;
//! one capture-local temporal worker owns the seven-source stream and its GPU
//! completion polling.

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
const FINISH_OUTPUT_CAPACITY: usize = 3;

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
    stitch_pending: Option<Pending>,
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
}

impl State {
    fn new() -> Self {
        Self {
            session: None,
            stitch_pending: None,
            temporal_pending: VecDeque::with_capacity(2),
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
        }
    }
}

pub(super) struct FilteredPanoramaJob {
    owner: Arc<FilteredCaptureInner>,
    session: Arc<FilteredSession>,
    frames: Arc<Frames>,
    reframe: Reframe,
    stamp: FrameStamp,
    size: Size,
    permit: DrawPermit,
}

pub(super) enum FilteredJob {
    Push(Box<FilteredPanoramaJob>),
    Finish {
        owner: Arc<FilteredCaptureInner>,
        session: Arc<FilteredSession>,
    },
}

impl FilteredJob {
    pub(super) fn owner(&self) -> Arc<FilteredCaptureInner> {
        match self {
            Self::Push(job) => Arc::clone(&job.owner),
            Self::Finish { owner, .. } => Arc::clone(owner),
        }
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
                field_size: [source.width, source.height / 2],
                state: Mutex::new(State::new()),
            }),
        })
    }

    /// Create a fresh decode epoch, sharing immutable GPU pipelines when the
    /// original owner was already attached.
    pub(crate) fn restart(&self) -> Fallible<Self> {
        let old_session = {
            let state = self.state()?;
            if let Some(error) = &state.failure {
                return Err(error.clone().into());
            }
            state.session.clone()
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
        // The installed old picture remains usable during seeking, but no
        // unpublished source/correction should accumulate across old epochs.
        // Cancellation never blocks the UI behind the temporal worker.
        {
            let mut previous = self.state()?;
            previous.retired = true;
            previous.ready.clear();
            previous.due_waiter = None;
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
            state.session = Some(candidate);
            Ok(())
        }
    }

    /// Admit one contiguous decoded source without waiting for worker or GPU.
    pub(crate) fn try_submit(&self, frames: Arc<Frames>, reframe: Reframe) -> Fallible<bool> {
        validate_reframe(&frames, &reframe)?;
        let stamp = frames.stamp();
        // The same source metadata must govern both panorama decoding and the
        // temporal representation. Do not maintain a second coefficient table.
        let reframe = reframe.with_samples(frames.samples);
        let session = self.attached_session()?;
        if let Err(error) = session.resident.retirements.poll() {
            self.inner.fail_worker(&error.to_string());
            return Err(error);
        }
        let permit = match session.resident.retirements.reserve() {
            Ok(permit) => permit,
            Err(DrawRetirementError::Full) => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        {
            let mut state = self.state()?;
            self.ensure_healthy(&state)?;
            if state.finished || state.finish_requested {
                return Err("filtered capture received a source after finish".into());
            }
            let output_capacity = source_output_capacity(state.accepted_sources);
            if state.stitch_pending.is_some()
                || outstanding_source_count(&state) >= 2
                || state.ready.len() + reserved_output_capacity(&state) + output_capacity
                    > READY_CAPACITY
            {
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
            state.stitch_pending = Some(Pending::Source {
                stamp: stamp.clone(),
                previous,
                output_capacity,
            });
        }
        let job = FilteredJob::Push(Box::new(FilteredPanoramaJob {
            owner: Arc::clone(&self.inner),
            session: Arc::clone(&session),
            frames,
            reframe,
            stamp,
            size: Size::new(self.inner.field_size[0], self.inner.field_size[1]),
            permit,
        }));
        match session.resident.worker.try_kick_filtered(job) {
            Ok(()) => Ok(true),
            Err(mpsc::TrySendError::Full(job)) => {
                self.inner.cancel(job)?;
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
        {
            let mut state = self.state()?;
            self.ensure_healthy(&state)?;
            if state.finished || state.finish_requested {
                return Ok(true);
            }
            if state.stitch_pending.is_some()
                || !state.temporal_pending.is_empty()
                || state.ready.len() + FINISH_OUTPUT_CAPACITY > READY_CAPACITY
            {
                return Ok(false);
            }
            state.finish_requested = true;
            state.stitch_pending = Some(Pending::Finish);
        }
        let job = FilteredJob::Finish {
            owner: Arc::clone(&self.inner),
            session: Arc::clone(&session),
        };
        match session.resident.worker.try_kick_filtered(job) {
            Ok(()) => Ok(true),
            Err(mpsc::TrySendError::Full(job)) => {
                self.inner.cancel(job)?;
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
        let output = state
            .ready
            .pop_front()
            .expect("checked filtered FIFO front");
        state.installed = Some(Arc::clone(&output));
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

    /// Preserve the last complete picture for a terminal-error screenshot.
    /// Unlike observed-state queries, this deliberately does not mask that
    /// already-installed resource with a later sticky worker failure.
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
            || state
                .ready
                .front()
                .is_some_and(|output| output.frame() == stamp)
        {
            return Ok(false);
        }
        if let Some(front) = state.ready.front() {
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
        if (state.stitch_pending.is_none() && state.temporal_pending.is_empty())
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
    pub(crate) fn assert_restart_ownership(&self, previous: &Self) {
        let current = self.attached_session().unwrap();
        let previous = previous.attached_session().unwrap();
        assert!(Arc::ptr_eq(&current.temporal, &previous.temporal));
        assert!(!Arc::ptr_eq(&current.epoch, &previous.epoch));
        assert!(previous.epoch.is_canceled());
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

    fn ensure_stitch_source(&self, stamp: &FrameStamp) -> Fallible<()> {
        let state = self.state()?;
        ensure_healthy_state(&state)?;
        if matches!(
            &state.stitch_pending,
            Some(Pending::Source { stamp: pending, .. }) if pending == stamp
        ) {
            Ok(())
        } else {
            Err("filtered panorama worker source differs from its pending request".into())
        }
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
        }) = state.stitch_pending.take()
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
        Ok(state.due_waiter.take().map(|(_, wake)| wake))
    }

    fn handoff_finish(&self) -> Fallible<Option<ReadyWake>> {
        let mut state = self.state()?;
        ensure_healthy_state(&state)?;
        if !matches!(state.stitch_pending.take(), Some(Pending::Finish)) {
            return Err("filtered stitch handoff has no pending finish".into());
        }
        state.temporal_pending.push_back(TemporalPending::Finish);
        Ok(state.due_waiter.take().map(|(_, wake)| wake))
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
        let wake = {
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
            state.due_waiter.take().map(|(_, wake)| wake)
        };
        if let Some(wake) = wake {
            wake.notify();
        }
        Ok(())
    }

    fn cancel(&self, job: FilteredJob) -> Fallible<()> {
        let mut state = self.state()?;
        ensure_healthy_state(&state)?;
        match (job, state.stitch_pending.take()) {
            (
                FilteredJob::Push(job),
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
            (FilteredJob::Finish { .. }, Some(Pending::Finish)) => {
                state.finish_requested = false;
            }
            _ => return Err("filtered backpressure changed its pending request".into()),
        }
        Ok(())
    }

    pub(super) fn fail_worker(&self, message: &str) {
        let (wake, report) = match self.state.lock() {
            Ok(mut state) => record_failure(&mut state, message),
            Err(poison) => {
                let mut state = poison.into_inner();
                record_failure(&mut state, message)
            }
        };
        if let Some(wake) = wake {
            wake.notify();
        }
        if report {
            eprintln!("{message}");
        }
    }
}

fn record_failure(state: &mut State, message: &str) -> (Option<ReadyWake>, bool) {
    state.stitch_pending = None;
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
    match job {
        FilteredJob::Push(job) => {
            job.owner.ensure_stitch_source(&job.stamp)?;
            let started = native_lifecycle_probe_enabled().then(std::time::Instant::now);
            native_lifecycle_event("filtered-worker-start", &job.stamp, None);
            if job.session.epoch.is_canceled() {
                return Ok(());
            }
            let panorama = prepare_correction_input(
                &job.session.resident,
                job.frames,
                job.reframe,
                &job.stamp,
                job.size,
                job.permit,
            )?;
            if let Some(started) = started {
                // This ends at panorama submission, not GPU completion.
                native_lifecycle_event(
                    "filtered-panorama-submitted",
                    &job.stamp,
                    Some(started.elapsed()),
                );
            }
            let wake = job.owner.handoff_source(&job.stamp)?;
            let owner = Arc::clone(&job.owner);
            let temporal = TemporalJob::Push {
                owner,
                epoch: Arc::clone(&job.session.epoch),
                panorama: Box::new(panorama),
                started,
            };
            let sent = job.session.temporal.send(temporal);
            if let Some(wake) = wake {
                wake.notify();
            }
            sent
        }
        FilteredJob::Finish { owner, session } => {
            let wake = owner.handoff_finish()?;
            let sent = session.temporal.send(TemporalJob::Finish {
                owner: Arc::clone(&owner),
                epoch: Arc::clone(&session.epoch),
            });
            if let Some(wake) = wake {
                wake.notify();
            }
            sent
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
        + match state.stitch_pending.as_ref() {
            Some(Pending::Source {
                output_capacity, ..
            }) => *output_capacity,
            Some(Pending::Finish) => FINISH_OUTPUT_CAPACITY,
            None => 0,
        }
}

fn outstanding_source_count(state: &State) -> usize {
    usize::from(matches!(
        &state.stitch_pending,
        Some(Pending::Source { .. })
    )) + state
        .temporal_pending
        .iter()
        .filter(|pending| matches!(pending, TemporalPending::Source { .. }))
        .count()
}

#[cfg(test)]
mod stage_tests {
    use super::*;
    use std::time::Duration;

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
    fn stitch_and_temporal_stages_share_one_two_source_bound() {
        let first = stamp(0, None);
        let second = stamp(1, Some(&first));
        let mut state = State::new();
        state.temporal_pending.push_back(TemporalPending::Source {
            stamp: first,
            output_capacity: 1,
        });
        state.stitch_pending = Some(Pending::Source {
            stamp: second,
            previous: None,
            output_capacity: 1,
        });
        assert_eq!(outstanding_source_count(&state), 2);
        assert_eq!(reserved_output_capacity(&state), 2);
    }

    #[test]
    fn concurrent_stage_failures_preserve_the_first_error() {
        let mut state = State::new();
        let (_, first_reported) = record_failure(&mut state, "underlying GPU error");
        let (_, second_reported) = record_failure(&mut state, "secondary pending mismatch");
        assert!(first_reported);
        assert!(!second_reported);
        assert_eq!(state.failure.as_deref(), Some("underlying GPU error"));
    }
}
