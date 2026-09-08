//! One coalesced wake for a due stitch result, not a per-frame clock.

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

#[derive(Clone, Debug, Default)]
pub(crate) struct ReadyWake(Arc<Mutex<State>>);

#[derive(Debug, Default)]
struct State {
    listening: bool,
    pending: bool,
    waker: Option<Waker>,
}

impl Hash for ReadyWake {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

impl ReadyWake {
    pub(crate) fn listening(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .listening
    }

    /// Called only by the tracked widget subscription. Without that listener,
    /// the Scene retains its existing redraw retry, including in instruments.
    pub(crate) fn listen(&self) -> ReadyListener {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        assert!(!state.listening, "a stitch wake has only one listener");
        state.listening = true;
        ReadyListener(self.clone())
    }

    pub(crate) fn notify(&self) {
        let waker = {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            if !state.listening {
                return;
            }
            state.pending = true;
            state.waker.take()
        };
        // The executor may run arbitrary code. Never wake under a state lock.
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

pub(crate) struct ReadyListener(ReadyWake);

impl ReadyListener {
    pub(crate) fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        let mut state = self.0.0.lock().unwrap_or_else(|error| error.into_inner());
        if std::mem::take(&mut state.pending) {
            state.waker = None;
            Poll::Ready(())
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl Drop for ReadyListener {
    fn drop(&mut self) {
        let mut state = self.0.0.lock().unwrap_or_else(|error| error.into_inner());
        state.listening = false;
        state.pending = false;
        state.waker = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::Wake;

    #[derive(Default)]
    struct Counter(AtomicUsize);

    impl Wake for Counter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn readiness_before_poll_and_between_polls_is_retained_and_coalesced() {
        let wake = ReadyWake::default();
        let mut listener = wake.listen();
        let mut cx = Context::from_waker(Waker::noop());
        wake.notify();
        wake.notify();
        assert!(listener.poll_ready(&mut cx).is_ready());
        assert!(listener.poll_ready(&mut cx).is_pending());
        wake.notify();
        assert!(listener.poll_ready(&mut cx).is_ready());
        wake.notify();
        assert!(listener.poll_ready(&mut cx).is_ready());
        assert!(listener.poll_ready(&mut cx).is_pending());
    }

    #[test]
    fn notification_wakes_the_waiter_once_and_listener_drop_restores_polling() {
        let wake = ReadyWake::default();
        assert!(!wake.listening());
        let counter = Arc::new(Counter::default());
        let waker = Waker::from(counter.clone());
        let mut cx = Context::from_waker(&waker);
        let mut listener = wake.listen();
        assert!(wake.listening());
        assert!(listener.poll_ready(&mut cx).is_pending());
        wake.notify();
        wake.notify();
        assert_eq!(counter.0.load(Ordering::Relaxed), 1);
        assert!(listener.poll_ready(&mut cx).is_ready());
        drop(listener);
        assert!(!wake.listening());
        wake.notify();
        let mut replacement = wake.listen();
        assert!(replacement.poll_ready(&mut cx).is_pending());
    }
}
