//! One bounded stitch worker shared by a capture and its seek restarts.
//!
//! Admission/import and publication remain on the caller. The worker owns the
//! imported source while executing the existing typed GPU chain. A pending
//! result stays in its capture's Working transaction until normal publication
//! or replacement draining collects it; dropping a receiver is exceptional
//! cancellation, never permission to recycle an uncertain source surface.

use std::sync::{Arc, mpsc};

use super::{
    ImportedOneXsPicture, OneXsGpuContext, ResidentCaptureSession, ResidentPendingMap,
    ResidentTransaction,
};
use crate::Fallible;

struct Job {
    session: Arc<ResidentCaptureSession>,
    source: ImportedOneXsPicture,
    result: mpsc::SyncSender<Fallible<ResidentPendingMap>>,
}

pub(super) struct ResidentStitchWorker {
    jobs: mpsc::SyncSender<Job>,
}

pub(super) struct ResidentWork {
    result: mpsc::Receiver<Fallible<ResidentPendingMap>>,
}

impl ResidentStitchWorker {
    pub(super) fn new(context: &OneXsGpuContext) -> Fallible<Self> {
        // One executing job and one waiting replacement, across all restarts.
        // A full channel is ordinary backpressure, never a UI-thread wait.
        let (jobs, incoming) = mpsc::sync_channel::<Job>(1);
        let thread = std::thread::Builder::new()
            .name("kjerag-stitch".into())
            .spawn(move || {
                for job in incoming {
                    let result = catch_worker_panic(|| {
                        if !job.session.context.is_worker_thread() {
                            return Err("ONE X2 stitch job reached a different worker".into());
                        }
                        job.session.submit(job.source)
                    });
                    if let Err(unsent) = job.result.send(result)
                        && let Ok(pending) = unsent.0
                    {
                        // Normal seek/reopen keeps the receiver until drained.
                        // An abandoned receiver must not free a source without
                        // the final validity/publication completion proof.
                        ResidentTransaction::Pending(pending).quarantine_uncertain();
                    }
                }
            })?;
        // No caller can enqueue until construction returns. Registering here
        // makes pacing apply only to this thread, never constructor qualifiers
        // or the UI even though they use clones of the same device and queue.
        context.register_worker_thread(thread.thread().id())?;
        // No join in Drop: the last sender closes the channel, and the worker
        // finishes its owned job before exiting without blocking the UI.
        drop(thread);
        Ok(Self { jobs })
    }

    pub(super) fn try_submit(
        &self,
        session: Arc<ResidentCaptureSession>,
        source: ImportedOneXsPicture,
    ) -> Fallible<Option<ResidentWork>> {
        let (result, received) = mpsc::sync_channel(1);
        match self.jobs.try_send(Job {
            session,
            source,
            result,
        }) {
            Ok(()) => Ok(Some(ResidentWork { result: received })),
            // This job has not run or acquired a GPU submission lease; its
            // imported source can be dropped normally and retried by Scene.
            Err(mpsc::TrySendError::Full(_)) => Ok(None),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                Err("ONE X2 stitch worker stopped before accepting the frame".into())
            }
        }
    }
}

impl ResidentWork {
    pub(super) fn collect(self) -> Fallible<ResidentTransaction> {
        match self.result.try_recv() {
            Ok(result) => result.map(ResidentTransaction::Pending),
            Err(mpsc::TryRecvError::Empty) => Ok(ResidentTransaction::Working(self)),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("ONE X2 stitch worker stopped before returning the frame".into())
            }
        }
    }

    #[cfg(test)]
    pub(super) fn waiting_for_test() -> (mpsc::SyncSender<Fallible<ResidentPendingMap>>, Self) {
        let (result, received) = mpsc::sync_channel(1);
        (result, Self { result: received })
    }
}

fn catch_worker_panic(
    submit: impl FnOnce() -> Fallible<ResidentPendingMap>,
) -> Fallible<ResidentPendingMap> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(submit)).unwrap_or_else(|payload| {
        Err(payload
            .downcast_ref::<&str>()
            .map(|message| (*message).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "ONE X2 stitch worker panicked".to_owned())
            .into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_poll_is_nonblocking_and_keeps_the_exact_result_channel() {
        let (sent, result) = mpsc::sync_channel(1);
        let transaction = ResidentWork { result }.collect().unwrap();
        let ResidentTransaction::Working(work) = transaction else {
            panic!("an unfinished worker was published");
        };
        sent.send(Err("exact underlying worker error".into()))
            .unwrap_or_else(|_| panic!("the result receiver was discarded"));
        let error = work.collect().err().expect("worker failure was lost");
        assert_eq!(error.to_string(), "exact underlying worker error");
    }

    #[test]
    fn disconnected_worker_is_a_failure_not_an_infinite_pending_frame() {
        let (sent, result) = mpsc::sync_channel(1);
        drop(sent);
        assert_eq!(
            ResidentWork { result }
                .collect()
                .err()
                .expect("disconnected worker was ignored")
                .to_string(),
            "ONE X2 stitch worker stopped before returning the frame"
        );
    }

    #[test]
    fn worker_unwind_preserves_the_underlying_failure() {
        let error = catch_worker_panic(|| panic!("exact GPU panic"))
            .err()
            .expect("worker unwind was ignored");
        assert_eq!(error.to_string(), "exact GPU panic");
    }
}
