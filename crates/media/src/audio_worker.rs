//! Audio refill has its own producer: a full video delivery queue cannot stop
//! sound. Only bounded ring capacity paces it. Seek acknowledgments are waited
//! for on the video decoder thread, never on the UI or audio callback.

use std::sync::{Arc, Condvar, Mutex};
use std::task::Waker;
use std::time::Duration;

use crate::Fallible;
use crate::audio::{AudioEpoch, Pipe};
use crate::track::{HEADROOM, Track};

trait Source: Send + 'static {
    fn epoch(&self) -> AudioEpoch;
    fn pump_one(&mut self) -> Fallible<bool>;
    fn seek_in(&mut self, to: i64, epoch: AudioEpoch) -> Fallible<()>;
}

impl Source for Track {
    fn epoch(&self) -> AudioEpoch {
        Track::epoch(self)
    }

    fn pump_one(&mut self) -> Fallible<bool> {
        Track::pump_one(self)
    }

    fn seek_in(&mut self, to: i64, epoch: AudioEpoch) -> Fallible<()> {
        Track::seek_in(self, to, epoch)
    }
}

#[derive(Default)]
struct State {
    pending: Option<Seek>,
    active: Option<Arc<Reply>>,
    failure: Option<String>,
    stopped: bool,
    failure_wake: Option<Waker>,
    #[cfg(test)]
    waiting_for_epoch: bool,
}

struct Seek {
    to: i64,
    epoch: AudioEpoch,
    reply: Arc<Reply>,
}

#[derive(Default)]
struct Reply {
    result: Mutex<Option<Result<(), String>>>,
    ready: Condvar,
}

impl Reply {
    fn complete(&self, result: Result<(), String>) {
        let mut held = self.result.lock().unwrap_or_else(|e| e.into_inner());
        if held.is_none() {
            *held = Some(result);
        }
        drop(held);
        self.ready.notify_all();
    }

    fn wait(&self) -> Fallible<()> {
        let mut result = self.result.lock().unwrap_or_else(|e| e.into_inner());
        while result.is_none() {
            result = self.ready.wait(result).unwrap_or_else(|e| e.into_inner());
        }
        result
            .take()
            .expect("acknowledged audio seek")
            .map_err(Into::into)
    }
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    pipe: Pipe,
}

/// The clone is a control handle, not another producer or shutdown owner.
#[derive(Clone)]
pub(crate) struct AudioControl(Arc<Shared>);

pub(crate) struct AudioWorker {
    control: AudioControl,
}

impl AudioWorker {
    pub(crate) fn new(track: Track, pipe: Pipe) -> Fallible<Self> {
        Self::spawn(track, pipe)
    }

    fn spawn(source: impl Source, pipe: Pipe) -> Fallible<Self> {
        let epoch = source.epoch();
        let control = AudioControl(Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            pipe,
        }));
        let running = control.clone();
        std::thread::Builder::new()
            .name("kjerag-audio".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(source, &running, epoch)
                }));
                let error = match outcome {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(error.to_string()),
                    Err(payload) => Some(
                        payload
                            .downcast_ref::<&str>()
                            .map(|s| (*s).to_owned())
                            .or_else(|| payload.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "audio producer panicked".into()),
                    ),
                };
                if let Some(error) = error {
                    running.fail(error);
                }
            })?;
        Ok(Self { control })
    }

    pub(crate) fn control(&self) -> AudioControl {
        self.control.clone()
    }

    pub(crate) fn seek(&self, to: i64, epoch: Option<AudioEpoch>) -> Fallible<()> {
        let epoch = epoch.unwrap_or_else(|| self.control.0.pipe.invalidate());
        self.control.request_seek(to, epoch)?.wait()
    }

    pub(crate) fn check(&self) -> Fallible<()> {
        self.control.check()
    }
}

impl Drop for AudioWorker {
    fn drop(&mut self) {
        self.control.stop();
    }
}

impl AudioControl {
    pub(crate) fn shortage_receipt(&self, due: Duration, lag: Duration) -> Option<(u64, Duration)> {
        self.0.pipe.shortage_receipt(due, lag)
    }

    #[cfg(test)]
    pub(crate) fn controlled_for_test(pipe: Pipe) -> Self {
        Self(Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            pipe,
        }))
    }

    pub(crate) fn invalidate(&self) -> AudioEpoch {
        self.0.pipe.invalidate()
    }

    pub(crate) fn check(&self) -> Fallible<()> {
        let state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        match &state.failure {
            Some(error) => Err(error.clone().into()),
            None => Ok(()),
        }
    }

    pub(crate) fn failure_wake(&self, wake: Waker) {
        let failed = {
            let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
            state.failure_wake = Some(wake.clone());
            state.failure.is_some()
        };
        if failed {
            wake.wake();
        }
    }

    fn request_seek(&self, to: i64, epoch: AudioEpoch) -> Fallible<Arc<Reply>> {
        let reply = Arc::new(Reply::default());
        let previous = {
            let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(error) = &state.failure {
                return Err(error.clone().into());
            }
            if state.stopped {
                return Err("audio producer stopped before accepting a seek".into());
            }
            state.pending.replace(Seek {
                to,
                epoch,
                reply: reply.clone(),
            })
        };
        // An overtaken request has no permission to write. Its decoder caller
        // can finish the obsolete video landing, whose own epoch rejects it.
        if let Some(previous) = previous {
            previous.reply.complete(Ok(()));
        }
        self.0.changed.notify_one();
        Ok(reply)
    }

    pub(crate) fn stop(&self) {
        let (pending, active) = {
            let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.stopped {
                return;
            }
            state.stopped = true;
            (state.pending.take(), state.active.take())
        };
        self.0.pipe.invalidate();
        if let Some(pending) = pending {
            pending
                .reply
                .complete(Err("audio producer stopped before completing a seek".into()));
        }
        if let Some(active) = active {
            active.complete(Err("audio producer stopped before completing a seek".into()));
        }
        self.0.changed.notify_all();
    }

    fn fail(&self, error: String) {
        let (pending, active, wake) = {
            let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.failure.is_some() {
                return;
            }
            state.failure = Some(error.clone());
            (
                state.pending.take(),
                state.active.take(),
                state.failure_wake.take(),
            )
        };
        if let Some(pending) = pending {
            pending.reply.complete(Err(error.clone()));
        }
        if let Some(active) = active {
            active.complete(Err(error));
        }
        if let Some(wake) = wake {
            wake.wake();
        }
        self.0.changed.notify_all();
    }
}

fn run(mut source: impl Source, control: &AudioControl, mut epoch: AudioEpoch) -> Fallible<()> {
    let mut ended = false;
    loop {
        let next = {
            let mut state = control.0.state.lock().unwrap_or_else(|e| e.into_inner());
            while !state.stopped
                && state.pending.is_none()
                && (ended
                    || !control.0.pipe.is_current(&epoch)
                    || control.0.pipe.room() <= HEADROOM)
            {
                #[cfg(test)]
                {
                    state.waiting_for_epoch = !control.0.pipe.is_current(&epoch);
                }
                state = if ended || !control.0.pipe.is_current(&epoch) {
                    control
                        .0
                        .changed
                        .wait(state)
                        .unwrap_or_else(|e| e.into_inner())
                } else {
                    // Host refill scheduling, not a clock/correction law.
                    control
                        .0
                        .changed
                        .wait_timeout(state, HEADROOM / 2)
                        .unwrap_or_else(|e| e.into_inner())
                        .0
                };
            }
            if state.stopped {
                return Ok(());
            }
            #[cfg(test)]
            {
                state.waiting_for_epoch = false;
            }
            let next = state.pending.take();
            state.active = next.as_ref().map(|seek| seek.reply.clone());
            next
        };
        if let Some(seek) = next {
            let current = control.0.pipe.is_current(&seek.epoch);
            let result = if current {
                source
                    .seek_in(seek.to, seek.epoch.clone())
                    .map_err(|e| e.to_string())
            } else {
                Ok(())
            };
            if current && result.is_ok() {
                ended = false;
                epoch = seek.epoch;
            }
            seek.reply.complete(result.clone());
            control
                .0
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .active = None;
            result.map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
        } else {
            ended = source.pump_one()?;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::time::{Duration, Instant};

    struct Samples {
        pipe: Pipe,
        epoch: AudioEpoch,
        through: Duration,
        count: Arc<AtomicUsize>,
        once: bool,
    }

    impl Source for Samples {
        fn epoch(&self) -> AudioEpoch {
            self.epoch.clone()
        }

        fn pump_one(&mut self) -> Fallible<bool> {
            self.through += Duration::from_millis(20);
            self.pipe.write_in(&self.epoch, &[0.5; 20], self.through);
            self.count.fetch_add(1, Ordering::Release);
            Ok(self.once)
        }

        fn seek_in(&mut self, to: i64, epoch: AudioEpoch) -> Fallible<()> {
            self.through = Duration::from_micros(to as u64);
            self.epoch = epoch;
            Ok(())
        }
    }

    pub(crate) fn fixture(pipe: Pipe) -> (AudioWorker, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let source = Samples {
            epoch: pipe.epoch(),
            pipe: pipe.clone(),
            through: Duration::ZERO,
            count: count.clone(),
            once: false,
        };
        (AudioWorker::spawn(source, pipe).unwrap(), count)
    }

    pub(crate) fn failure_fixture(pipe: Pipe) -> AudioWorker {
        struct Failure(AudioEpoch);
        impl Source for Failure {
            fn epoch(&self) -> AudioEpoch {
                self.0.clone()
            }
            fn pump_one(&mut self) -> Fallible<bool> {
                Err("underlying independent audio decode failure".into())
            }
            fn seek_in(&mut self, _: i64, _: AudioEpoch) -> Fallible<()> {
                Ok(())
            }
        }
        AudioWorker::spawn(Failure(pipe.epoch()), pipe).unwrap()
    }

    pub(crate) fn until(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "audio producer did not make progress"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn flush_rejects_in_flight_samples_after_callback_clears_the_ring() {
        let pipe = Pipe::new(1000, 1, Duration::from_millis(500));
        let old = pipe.epoch();
        pipe.write_in(&old, &[0.5; 20], Duration::from_millis(20));
        let fresh = pipe.invalidate();
        assert_eq!(pipe.room(), Duration::ZERO, "flush must finish in callback");
        pipe.fill(&mut [0.0; 20], None);
        pipe.write_in(&old, &[0.9; 20], Duration::from_millis(20));
        assert_eq!(pipe.room(), Duration::from_millis(500));
        pipe.write_in(&fresh, &[0.25; 20], Duration::from_millis(20));
        let mut out = [0.0; 20];
        pipe.fill(&mut out, Some(Duration::ZERO));
        assert_eq!(out[19], 0.25);
        assert_eq!(pipe.health().dropped, 0, "stale writes are not overflow");
    }

    struct GatedSeek {
        samples: Samples,
        entered: Sender<()>,
        release: Receiver<()>,
    }

    impl Source for GatedSeek {
        fn epoch(&self) -> AudioEpoch {
            self.samples.epoch()
        }

        fn pump_one(&mut self) -> Fallible<bool> {
            self.samples.pump_one()
        }

        fn seek_in(&mut self, to: i64, epoch: AudioEpoch) -> Fallible<()> {
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
            // Model a packet completing inside an old seek, after newer hush.
            self.samples
                .pipe
                .write_in(&epoch, &[0.9; 20], Duration::from_millis(20));
            self.samples.seek_in(to, epoch)
        }
    }

    fn gated() -> (AudioWorker, Pipe, Receiver<()>, Sender<()>) {
        let pipe = Pipe::new(1000, 1, Duration::from_millis(500));
        let (entered, observing) = mpsc::channel();
        let (release, waiting) = mpsc::channel();
        let source = GatedSeek {
            samples: Samples {
                epoch: pipe.epoch(),
                pipe: pipe.clone(),
                through: Duration::ZERO,
                count: Arc::new(AtomicUsize::new(0)),
                once: false,
            },
            entered,
            release: waiting,
        };
        (
            AudioWorker::spawn(source, pipe.clone()).unwrap(),
            pipe,
            observing,
            release,
        )
    }

    #[test]
    fn obsolete_seek_cannot_authorize_writes_after_newer_ui_hush() {
        let (worker, pipe, entered, release) = gated();
        let old = pipe.invalidate();
        let reply = worker.control.request_seek(0, old).unwrap();
        entered.recv_timeout(Duration::from_secs(3)).unwrap();
        let fresh = pipe.invalidate();
        pipe.fill(&mut [0.0; 500], None);
        release.send(()).unwrap();
        reply.wait().unwrap();
        assert!(pipe.is_current(&fresh));
        assert_eq!(pipe.room(), Duration::from_millis(500));
        assert_eq!(pipe.health().dropped, 0);
    }

    #[test]
    fn shutdown_releases_active_seek_without_joining_its_io() {
        let (worker, pipe, entered, release) = gated();
        let epoch = pipe.invalidate();
        let reply = worker.control.request_seek(0, epoch).unwrap();
        entered.recv_timeout(Duration::from_secs(3)).unwrap();
        worker.control.stop();
        let error = reply.wait().unwrap_err();
        assert_eq!(
            error.to_string(),
            "audio producer stopped before completing a seek"
        );
        pipe.fill(&mut [0.0; 500], None);
        release.send(()).unwrap();
        assert_eq!(pipe.room(), Duration::from_millis(500));
    }

    #[test]
    fn startup_uses_the_sources_epoch_not_a_later_pipe_epoch() {
        let pipe = Pipe::new(1000, 1, Duration::from_millis(500));
        let count = Arc::new(AtomicUsize::new(0));
        let source = Samples {
            epoch: pipe.epoch(),
            pipe: pipe.clone(),
            through: Duration::ZERO,
            count: count.clone(),
            once: true,
        };
        let fresh = pipe.invalidate();
        pipe.fill(&mut [0.0; 20], None);
        let worker = AudioWorker::spawn(source, pipe.clone()).unwrap();
        until(|| worker.control.0.state.lock().unwrap().waiting_for_epoch);
        assert_eq!(count.load(Ordering::Acquire), 0);
        // The source cannot pump in its invalidated startup epoch. A valid
        // seek restores it and produces exactly one packet before EOF park.
        worker
            .control
            .request_seek(0, fresh)
            .unwrap()
            .wait()
            .unwrap();
        until(|| count.load(Ordering::Acquire) == 1);
        assert_eq!(pipe.room(), Duration::from_millis(480));
        worker.control.check().unwrap();
        let next = pipe.invalidate();
        pipe.fill(&mut [0.0; 20], None);
        worker
            .control
            .request_seek(0, next)
            .unwrap()
            .wait()
            .unwrap();
        until(|| count.load(Ordering::Acquire) == 2);
        assert_eq!(pipe.room(), Duration::from_millis(480));
    }

    #[test]
    fn terminal_failure_wakes_registration_even_after_worker_exit() {
        use std::task::Wake;
        struct Count(AtomicUsize);
        impl Wake for Count {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::Release);
            }
        }
        let pipe = Pipe::new(1000, 1, Duration::from_millis(500));
        let worker = failure_fixture(pipe);
        until(|| worker.control.check().is_err());
        let counter = Arc::new(Count(AtomicUsize::new(0)));
        worker.control.failure_wake(Waker::from(counter.clone()));
        assert_eq!(counter.0.load(Ordering::Acquire), 1);
        assert_eq!(
            worker.control.check().unwrap_err().to_string(),
            "underlying independent audio decode failure"
        );
    }
}
