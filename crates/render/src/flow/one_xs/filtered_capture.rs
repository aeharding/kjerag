//! Typed worker bridge from decoded resident sources to filtered panoramas.
//!
//! This owner is one decode epoch. Admission and exact-stamp presentation are
//! nonblocking; the existing stitch worker owns panorama preparation, temporal
//! processing and GPU completion polling.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, mpsc};

use kjerag_media::{FrameStamp, Frames};
use kjerag_meta::OrientationTrack;

use super::panorama_ingest::{prepare_panorama, validate_reframe};
use super::{ResidentCameraProfile, ResidentCaptureSession};
use crate::draw_retirement::{DrawPermit, DrawRetirementError};
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
use crate::ready_wake::ReadyWake;
use crate::temporal_fusion::color::MatrixCoefficients;
use crate::temporal_fusion::settings::Provider;
use crate::temporal_fusion::stream::{FilteredPanorama, Stream};
use crate::{Fallible, Reframe, Size};

const READY_CAPACITY: usize = 4;
const FINISH_OUTPUT_CAPACITY: usize = 3;

pub(super) struct FilteredSession {
    resident: Arc<ResidentCaptureSession>,
    stream: Mutex<Stream>,
}

enum Pending {
    Source {
        stamp: FrameStamp,
        previous: Option<FrameStamp>,
    },
    Finish,
}

struct State {
    session: Option<Arc<FilteredSession>>,
    pending: Option<Pending>,
    accepted: Option<FrameStamp>,
    expected: VecDeque<FrameStamp>,
    ready: VecDeque<Arc<FilteredPanorama>>,
    installed: Option<Arc<FilteredPanorama>>,
    finish_requested: bool,
    finished: bool,
    failure: Option<String>,
    due_waiter: Option<(FrameStamp, ReadyWake)>,
}

impl State {
    fn new() -> Self {
        Self {
            session: None,
            pending: None,
            accepted: None,
            expected: VecDeque::new(),
            ready: VecDeque::with_capacity(READY_CAPACITY),
            installed: None,
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
    matrix: MatrixCoefficients,
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
    full: [u32; 2],
    state: Mutex<State>,
}

/// One source-ordered filtered capture. Clones share chronology and outputs.
#[derive(Clone)]
pub(crate) struct FilteredCaptureFacade {
    inner: Arc<FilteredCaptureInner>,
}

impl FilteredCaptureFacade {
    /// Construct the CPU owner. GPU resources remain lazy until [`Self::attach`].
    /// Selection remains in real-Scene tests until the complete path is qualified.
    #[cfg(test)]
    pub(crate) fn new(
        profile: Arc<ResidentCameraProfile>,
        orientation: OrientationTrack,
        provider: Provider,
    ) -> Fallible<Self> {
        let source = profile.source_size;
        let width = source
            .width
            .checked_mul(2)
            .ok_or("filtered panorama width overflows u32")?;
        if source.width == 0 || source.height == 0 || source.width != source.height {
            return Err(format!(
                "filtered panorama needs a square nonzero source, got {} by {}",
                source.width, source.height
            )
            .into());
        }
        Ok(Self {
            inner: Arc::new(FilteredCaptureInner {
                profile,
                orientation,
                provider,
                full: [width, source.height],
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
            .map(|old| {
                let resident = Arc::new(old.resident.restarted()?);
                let stream = Stream::new(
                    resident.context.device(),
                    resident.context.queue(),
                    self.inner.full,
                    self.inner.provider.restarted(),
                )?;
                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Arc::new(FilteredSession {
                    resident,
                    stream: Mutex::new(stream),
                }))
            })
            .transpose()?;
        let mut state = State::new();
        state.session = session;
        Ok(Self {
            inner: Arc::new(FilteredCaptureInner {
                profile: Arc::clone(&self.inner.profile),
                orientation: self.inner.orientation.clone(),
                provider: self.inner.provider.restarted(),
                full: self.inner.full,
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
        let stream = Stream::new(
            resident.context.device(),
            resident.context.queue(),
            self.inner.full,
            self.inner.provider.restarted(),
        )?;
        let candidate = Arc::new(FilteredSession {
            resident,
            stream: Mutex::new(stream),
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
        let matrix = MatrixCoefficients::from_source_rgb(reframe.source_color_matrix());
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
            if state.pending.is_some() || state.ready.len() == READY_CAPACITY {
                return Ok(false);
            }
            if let Some(previous) = state.accepted.as_ref() {
                super::validate_resident_sequence(Some(previous), &stamp)?;
            }
            let previous = state.accepted.replace(stamp.clone());
            state.expected.push_back(stamp.clone());
            state.pending = Some(Pending::Source {
                stamp: stamp.clone(),
                previous,
            });
        }
        let job = FilteredJob::Push(Box::new(FilteredPanoramaJob {
            owner: Arc::clone(&self.inner),
            session: Arc::clone(&session),
            frames,
            reframe,
            stamp,
            size: Size::new(self.inner.full[0], self.inner.full[1]),
            matrix,
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
            if state.pending.is_some()
                || state.ready.len() + FINISH_OUTPUT_CAPACITY > READY_CAPACITY
            {
                return Ok(false);
            }
            state.finish_requested = true;
            state.pending = Some(Pending::Finish);
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
    pub(crate) fn install_due(
        &self,
        stamp: &FrameStamp,
    ) -> Fallible<Option<Arc<FilteredPanorama>>> {
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
    pub(crate) fn installed(&self) -> Fallible<Option<Arc<FilteredPanorama>>> {
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
        if state.pending.is_none() || !state.expected.iter().any(|expected| expected == stamp) {
            return Ok(false);
        }
        state.due_waiter = Some((stamp.clone(), wake.clone()));
        Ok(true)
    }

    pub(crate) fn same_capture(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
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

    fn ensure_pending_source(&self, stamp: &FrameStamp) -> Fallible<()> {
        if matches!(
            &self.state()?.pending,
            Some(Pending::Source { stamp: pending, .. }) if pending == stamp
        ) {
            Ok(())
        } else {
            Err("filtered panorama worker source differs from its pending request".into())
        }
    }

    fn complete(&self, pending: PendingKind, outputs: Vec<FilteredPanorama>) -> Fallible<()> {
        let wake = {
            let mut state = self.state()?;
            let matches = match (&state.pending, pending) {
                (Some(Pending::Source { stamp, .. }), PendingKind::Source(done)) => stamp == done,
                (Some(Pending::Finish), PendingKind::Finish) => true,
                _ => false,
            };
            if !matches {
                return Err("filtered worker completion differs from its pending request".into());
            }
            if state.ready.len() + outputs.len() > READY_CAPACITY {
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
            state.pending = None;
            state.due_waiter.take().map(|(_, wake)| wake)
        };
        if let Some(wake) = wake {
            wake.notify();
        }
        Ok(())
    }

    fn cancel(&self, job: FilteredJob) -> Fallible<()> {
        let mut state = self.state()?;
        match (job, state.pending.take()) {
            (FilteredJob::Push(job), Some(Pending::Source { stamp, previous }))
                if stamp == job.stamp =>
            {
                if state.expected.pop_back().as_ref() != Some(&stamp) {
                    return Err("filtered backpressure changed accepted source order".into());
                }
                state.accepted = previous;
            }
            (FilteredJob::Finish { .. }, Some(Pending::Finish)) => {
                state.finish_requested = false;
            }
            _ => return Err("filtered backpressure changed its pending request".into()),
        }
        Ok(())
    }

    pub(super) fn fail_worker(&self, message: &str) {
        let wake = match self.state.lock() {
            Ok(mut state) => {
                state.pending = None;
                state.ready.clear();
                state.failure = Some(message.to_owned());
                state.due_waiter.take().map(|(_, wake)| wake)
            }
            Err(poison) => {
                let mut state = poison.into_inner();
                state.pending = None;
                state.ready.clear();
                state.failure = Some(message.to_owned());
                state.due_waiter.take().map(|(_, wake)| wake)
            }
        };
        if let Some(wake) = wake {
            wake.notify();
        }
        eprintln!("{message}");
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
            job.owner.ensure_pending_source(&job.stamp)?;
            let panorama = prepare_panorama(
                &job.session.resident,
                job.frames,
                job.reframe,
                &job.stamp,
                job.size,
                job.permit,
            )?;
            let outputs = job
                .session
                .stream
                .lock()
                .map_err(|_| "filtered temporal stream is poisoned")?
                .push(panorama, job.matrix)?;
            job.owner.complete(PendingKind::Source(&job.stamp), outputs)
        }
        FilteredJob::Finish { owner, session } => {
            let outputs = session
                .stream
                .lock()
                .map_err(|_| "filtered temporal stream is poisoned")?
                .finish()?;
            owner.complete(PendingKind::Finish, outputs)
        }
    }
}
