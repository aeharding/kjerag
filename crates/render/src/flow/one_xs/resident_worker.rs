//! One bounded stitch worker shared by a capture and its seek restarts.
//!
//! Admission/import and publication remain on the caller. Once admitted, a
//! capture actor owns source execution, final validity acknowledgement and
//! temporal-future commit. It may run the second admitted source without a UI
//! poll. A completed second result parks in capture state when the one future
//! slot is occupied; the actor then ends until publication kicks it again.

use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use super::filtered_capture::{FilteredJob, service_filtered};
use super::panorama_ingest::{ResidentPanoramaIngestInner, service_panorama};
use super::{
    ResidentCaptureFacadeInner, ResidentPendingMap, ResidentPoll, ResidentReadyMap,
    native_lifecycle_event, native_lifecycle_probe_enabled, prepare_resident_bound,
};
use crate::Fallible;
use crate::ready_wake::ReadyWake;

/// Shared across seek epochs. Channel space released by an old epoch must
/// wake the current source consumer too, not just the old exact-frame waiter.
#[derive(Default)]
pub(super) struct WorkerProgressWake(Mutex<Option<ReadyWake>>);

impl WorkerProgressWake {
    pub(super) fn set(&self, wake: &ReadyWake) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(wake.clone());
    }

    pub(super) fn notify(&self) {
        let wake = self.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(wake) = wake {
            wake.notify();
        }
    }
}

#[cfg(test)]
mod progress_wake_tests {
    use super::*;
    use std::task::{Context, Waker};

    #[test]
    fn shared_executor_progress_wake_survives_multiple_epoch_turns() {
        let wake = ReadyWake::default();
        let mut listener = wake.listen();
        let progress = WorkerProgressWake::default();
        let mut cx = Context::from_waker(Waker::noop());
        progress.set(&wake);
        assert!(listener.poll_ready(&mut cx).is_pending());
        // Dequeue/finish notifications coalesce, but registration persists
        // after consumption for the next (possibly superseded) epoch.
        progress.notify();
        progress.notify();
        assert!(listener.poll_ready(&mut cx).is_ready());
        assert!(listener.poll_ready(&mut cx).is_pending());
        progress.notify();
        assert!(listener.poll_ready(&mut cx).is_ready());
        progress.set(&wake);
        assert!(
            listener.poll_ready(&mut cx).is_pending(),
            "registration cannot self-wake"
        );
    }
}

pub(super) struct PanoramaJob {
    pub(super) ingest: Arc<ResidentPanoramaIngestInner>,
    pub(super) frames: Arc<kjerag_media::Frames>,
    pub(super) reframe: crate::Reframe,
    pub(super) stamp: kjerag_media::FrameStamp,
    pub(super) size: crate::Size,
    pub(super) permit: crate::draw_retirement::DrawPermit,
}

enum Job {
    Capture(Arc<ResidentCaptureFacadeInner>),
    Panorama(Box<PanoramaJob>),
    Filtered(FilteredJob),
    #[cfg(test)]
    Pause {
        entered: mpsc::SyncSender<()>,
        release: mpsc::Receiver<()>,
    },
}

#[cfg(test)]
pub(crate) struct StitchPause(mpsc::SyncSender<()>);

#[cfg(test)]
impl Drop for StitchPause {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

pub(super) struct ResidentStitchWorker {
    jobs: mpsc::SyncSender<Job>,
    progress: Arc<WorkerProgressWake>,
}

impl ResidentStitchWorker {
    pub(super) fn new(context: &super::OneXsGpuContext) -> Fallible<Self> {
        // One executing capture actor and one queued actor across seek
        // restarts. Each capture independently limits accepted sources to two.
        let (jobs, incoming) = mpsc::sync_channel::<Job>(1);
        let progress = Arc::new(WorkerProgressWake::default());
        let running_progress = progress.clone();
        let thread = std::thread::Builder::new()
            .name("kjerag-stitch".into())
            .spawn(move || {
                for job in incoming {
                    running_progress.notify();
                    match job {
                        Job::Capture(capture) => {
                            let owner = Arc::clone(&capture);
                            if let Err(error) = catch_capture_panic(|| service_capture(capture)) {
                                owner.fail_worker(error);
                            }
                        }
                        Job::Panorama(job) => {
                            let owner = Arc::clone(&job.ingest);
                            if let Err(error) = catch_capture_panic(|| service_panorama(job)) {
                                owner.fail(&error.to_string());
                            }
                        }
                        Job::Filtered(job) => {
                            let owner = job.owner();
                            if let Err(error) = catch_capture_panic(|| service_filtered(job)) {
                                owner.fail_worker(&error.to_string());
                            }
                        }
                        #[cfg(test)]
                        Job::Pause { entered, release } => {
                            let _ = entered.send(());
                            let _ = release.recv();
                        }
                    }
                    running_progress.notify();
                }
            })?;
        context.register_worker_thread(thread.thread().id())?;
        // The sender owns shutdown. The detached thread finishes any exact
        // active source before the channel closes and never blocks the UI.
        drop(thread);
        Ok(Self { jobs, progress })
    }

    pub(super) fn set_progress_wake(&self, wake: &ReadyWake) {
        self.progress.set(wake);
    }

    #[cfg(test)]
    pub(crate) fn pause_for_test(&self) -> StitchPause {
        let (entered, incoming) = mpsc::sync_channel(1);
        let (release, waiting) = mpsc::sync_channel(1);
        assert!(
            self.jobs
                .send(Job::Pause {
                    entered,
                    release: waiting
                })
                .is_ok()
        );
        let guard = StitchPause(release);
        incoming
            .recv_timeout(Duration::from_secs(10))
            .expect("stitch pause did not enter");
        guard
    }

    /// Schedule one capture actor without waiting. The caller has already set
    /// `worker_running`; a full channel must roll that marker back unless an
    /// existing actor still owns this capture's queued work.
    pub(super) fn try_kick(
        &self,
        capture: Arc<ResidentCaptureFacadeInner>,
    ) -> Result<bool, mpsc::TrySendError<Arc<ResidentCaptureFacadeInner>>> {
        match self.jobs.try_send(Job::Capture(capture)) {
            Ok(()) => Ok(true),
            Err(mpsc::TrySendError::Full(Job::Capture(capture))) => {
                Err(mpsc::TrySendError::Full(capture))
            }
            Err(mpsc::TrySendError::Disconnected(Job::Capture(capture))) => {
                Err(mpsc::TrySendError::Disconnected(capture))
            }
            Err(_) => unreachable!("capture kick returned another worker job"),
        }
    }

    pub(super) fn try_kick_panorama(
        &self,
        job: Box<PanoramaJob>,
    ) -> Result<(), mpsc::TrySendError<Box<PanoramaJob>>> {
        match self.jobs.try_send(Job::Panorama(job)) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(Job::Panorama(job))) => Err(mpsc::TrySendError::Full(job)),
            Err(mpsc::TrySendError::Disconnected(Job::Panorama(job))) => {
                Err(mpsc::TrySendError::Disconnected(job))
            }
            Err(_) => unreachable!("panorama kick returned another worker job"),
        }
    }

    pub(super) fn try_kick_filtered(
        &self,
        job: FilteredJob,
    ) -> Result<(), mpsc::TrySendError<FilteredJob>> {
        match self.jobs.try_send(Job::Filtered(job)) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(Job::Filtered(job))) => Err(mpsc::TrySendError::Full(job)),
            Err(mpsc::TrySendError::Disconnected(Job::Filtered(job))) => {
                Err(mpsc::TrySendError::Disconnected(job))
            }
            Err(_) => unreachable!("filtered kick returned another worker job"),
        }
    }
}

fn service_capture(capture: Arc<ResidentCaptureFacadeInner>) -> Fallible<()> {
    loop {
        let Some(next) = capture.take_worker_input()? else {
            return Ok(());
        };
        let (session, source, stamp) = match next {
            super::ResidentWorkerInput::Source {
                session,
                source,
                stamp,
            } => (session, source, stamp),
            super::ResidentWorkerInput::Ready { session, ready } => {
                if !commit_or_park(&capture, &session, ready)? {
                    return Ok(());
                }
                continue;
            }
        };
        if native_lifecycle_probe_enabled() {
            native_lifecycle_event("worker-start", &stamp, None);
        }
        let started = std::time::Instant::now();
        let pending = catch_submit_panic(|| session.submit(source));
        if native_lifecycle_probe_enabled() {
            native_lifecycle_event(
                if pending.is_ok() {
                    "worker-return-success"
                } else {
                    "worker-return-error"
                },
                &stamp,
                Some(started.elapsed()),
            );
        }
        let ready = finish_pending(&session, pending?)?;
        native_lifecycle_event("worker-validity-ready", &stamp, None);
        if !commit_or_park(&capture, &session, ready)? {
            return Ok(());
        }
    }
}

pub(super) fn finish_pending(
    session: &super::ResidentCaptureSession,
    mut pending: ResidentPendingMap,
) -> Fallible<ResidentReadyMap> {
    loop {
        let polled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.context.device().poll(wgpu::PollType::Poll)
        }));
        match polled {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                pending.quarantine_uncertain();
                return Err(error.into());
            }
            Err(payload) => {
                pending.quarantine_uncertain();
                return Err(payload
                    .downcast_ref::<&str>()
                    .map(|message| (*message).to_owned())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "ONE X2 GPU device poll panicked".to_owned())
                    .into());
            }
        }
        match pending.finish_after_poll_classified() {
            ResidentPoll::Pending(value) => {
                pending = value;
                std::thread::park_timeout(Duration::from_micros(100));
            }
            ResidentPoll::Ready(value) => return Ok(value),
            ResidentPoll::Refused(error) | ResidentPoll::Quarantined(error) => return Err(error),
        }
    }
}

fn commit_or_park(
    capture: &Arc<ResidentCaptureFacadeInner>,
    session: &Arc<super::ResidentCaptureSession>,
    ready: ResidentReadyMap,
) -> Fallible<bool> {
    let ready = match capture.take_commit_permission(ready)? {
        Some(ready) => ready,
        None => return Ok(false),
    };
    let completed = ready.frame().clone();
    // Binding creates immutable draw resources and must not hold capture
    // state. Only the short future commit takes façade then root locks.
    let bound = match ready {
        ResidentReadyMap::Cold(map) => prepare_resident_bound(*map, Arc::clone(&session.direct)),
        ResidentReadyMap::Warm(map) => prepare_resident_bound(*map, Arc::clone(&session.direct)),
    }?;
    if !capture.commit_worker_future(bound, &completed)? {
        return Ok(false);
    }
    native_lifecycle_event("worker-temporal-committed", &completed, None);
    Ok(true)
}

fn catch_capture_panic(submit: impl FnOnce() -> Fallible<()>) -> Fallible<()> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(submit))
        .unwrap_or_else(|payload| Err(panic_message(payload).into()))
}

fn catch_submit_panic(
    submit: impl FnOnce() -> Fallible<ResidentPendingMap>,
) -> Fallible<ResidentPendingMap> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(submit))
        .unwrap_or_else(|payload| Err(panic_message(payload).into()))
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "ONE X2 stitch worker panicked".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_unwind_preserves_the_underlying_failure() {
        let error = catch_capture_panic(|| panic!("exact GPU panic"))
            .expect_err("worker unwind was ignored");
        assert_eq!(error.to_string(), "exact GPU panic");
    }
}
