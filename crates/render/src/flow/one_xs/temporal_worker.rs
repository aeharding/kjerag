//! One bounded temporal executor shared across a capture's seek epochs.
//!
//! The resident stitch worker produces source-ordered body panoramas. This
//! worker alone mutates [`Stream`], so its seven-layer history and center
//! ordering remain serial while the stitch worker prepares one successor.
//!
//! Across rapid product seeks the shared executor retains at most one active
//! and one queued temporal epoch. Its blocking handoff can retain one more
//! prepared epoch on the sole stitch worker, whose own capacity-one channel
//! can retain one unprepared job. Together with Scene's current and last-shown
//! facades, that is at most six distinct epoch histories rather than one per
//! seek. Intermediate idle restart facades have no job owner and drop.

use std::sync::{Arc, Mutex, mpsc};

use kjerag_media::FrameStamp;

use super::filtered_capture::FilteredCaptureInner;
use super::native_lifecycle_event;
use crate::Fallible;
use crate::direct_type2::BodyPanorama;
use crate::temporal_fusion::color::MatrixCoefficients;
use crate::temporal_fusion::stream::Stream;

pub(super) enum TemporalJob {
    Push {
        owner: Arc<FilteredCaptureInner>,
        epoch: Arc<TemporalEpoch>,
        panorama: BodyPanorama,
        stamp: FrameStamp,
        matrix: MatrixCoefficients,
        started: Option<std::time::Instant>,
    },
    Finish {
        owner: Arc<FilteredCaptureInner>,
        epoch: Arc<TemporalEpoch>,
    },
}

/// One decode epoch's private temporal state.
///
/// The mutex makes the state movable through the shared executor without
/// granting another caller access. Only [`service`] locks it.
pub(super) struct TemporalEpoch {
    stream: Mutex<Stream>,
}

pub(super) struct TemporalWorker {
    jobs: mpsc::SyncSender<ExecutorJob>,
}

impl TemporalWorker {
    pub(super) fn new() -> Fallible<Self> {
        // Every restarted epoch clones this sender. One job executes and one
        // waits here globally; a third handoff blocks the sole stitch worker,
        // which is deliberate cross-epoch backpressure rather than another
        // temporal thread or an unbounded collection of histories.
        let (jobs, incoming) = mpsc::sync_channel::<ExecutorJob>(1);
        let thread = std::thread::Builder::new()
            .name("kjerag-temporal".into())
            .spawn(move || {
                for job in incoming {
                    match job {
                        ExecutorJob::Run(job) => {
                            let owner = job.owner();
                            if let Err(error) = catch_temporal_panic(|| service(job)) {
                                owner.fail_worker(&error.to_string());
                            }
                        }
                        #[cfg(test)]
                        ExecutorJob::Probe { entered, release } => {
                            let _ = entered.send(());
                            let _ = release.recv();
                        }
                    }
                }
            })?;
        // The sender owns shutdown. Jobs retain their capture only while
        // queued or executing, so dropping the facade cannot form a cycle.
        drop(thread);
        Ok(Self { jobs })
    }

    pub(super) fn send(&self, job: TemporalJob) -> Fallible<()> {
        self.jobs
            .send(ExecutorJob::Run(job))
            .map_err(|_| "filtered temporal worker stopped before accepting a job".into())
    }
}

impl TemporalEpoch {
    pub(super) fn new(stream: Stream) -> Self {
        Self {
            stream: Mutex::new(stream),
        }
    }
}

enum ExecutorJob {
    Run(TemporalJob),
    #[cfg(test)]
    Probe {
        entered: mpsc::SyncSender<()>,
        release: mpsc::Receiver<()>,
    },
}

impl TemporalJob {
    fn owner(&self) -> Arc<FilteredCaptureInner> {
        match self {
            Self::Push { owner, .. } | Self::Finish { owner, .. } => Arc::clone(owner),
        }
    }
}

fn service(job: TemporalJob) -> Fallible<()> {
    match job {
        TemporalJob::Push {
            owner,
            epoch,
            panorama,
            stamp,
            matrix,
            started,
        } => {
            owner.ensure_temporal_source(&stamp)?;
            let mut stream = epoch
                .stream
                .lock()
                .map_err(|_| "filtered temporal epoch stream is poisoned")?;
            let outputs = stream.push(panorama, matrix)?;
            if let Some(started) = started {
                native_lifecycle_event("filtered-worker-complete", &stamp, Some(started.elapsed()));
            }
            owner.complete_source(&stamp, outputs)
        }
        TemporalJob::Finish { owner, epoch } => {
            owner.ensure_temporal_finish()?;
            let mut stream = epoch
                .stream
                .lock()
                .map_err(|_| "filtered temporal epoch stream is poisoned")?;
            let outputs = stream.finish()?;
            owner.complete_finish(outputs)
        }
    }
}

fn catch_temporal_panic(submit: impl FnOnce() -> Fallible<()>) -> Fallible<()> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(submit)).unwrap_or_else(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .map(|message| (*message).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "filtered temporal worker panicked".to_owned());
        Err(message.into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn probe() -> (ExecutorJob, mpsc::Receiver<()>, mpsc::SyncSender<()>) {
        let (entered_send, entered) = mpsc::sync_channel(1);
        let (release, release_receive) = mpsc::sync_channel(1);
        (
            ExecutorJob::Probe {
                entered: entered_send,
                release: release_receive,
            },
            entered,
            release,
        )
    }

    #[test]
    fn cloned_executor_has_one_global_active_and_queued_bound() {
        let worker = Arc::new(TemporalWorker::new().unwrap());
        let restarted = Arc::clone(&worker);
        let (first, first_entered, first_release) = probe();
        let (second, second_entered, second_release) = probe();
        let (third, third_entered, third_release) = probe();
        assert!(worker.jobs.send(first).is_ok());
        first_entered.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(restarted.jobs.send(second).is_ok());

        let (sent, sent_receive) = mpsc::sync_channel(1);
        let blocked = Arc::clone(&worker);
        let sender = std::thread::spawn(move || {
            assert!(blocked.jobs.send(third).is_ok());
            sent.send(()).unwrap();
        });
        assert!(
            sent_receive
                .recv_timeout(Duration::from_millis(20))
                .is_err()
        );

        first_release.send(()).unwrap();
        second_entered.recv_timeout(Duration::from_secs(1)).unwrap();
        sent_receive.recv_timeout(Duration::from_secs(1)).unwrap();
        second_release.send(()).unwrap();
        third_entered.recv_timeout(Duration::from_secs(1)).unwrap();
        third_release.send(()).unwrap();
        sender.join().unwrap();
    }
}
