//! A one-shot wake for an empty decoder queue. Delivery still uses the bounded
//! channel; this signal neither buffers frames nor decides when to show them.

use std::sync::mpsc::{self, Receiver, SendError, SyncSender};
use std::sync::{Arc, Mutex};
use std::task::Waker;

#[derive(Default)]
pub(super) struct Arrival(Mutex<State>);

#[derive(Default)]
struct State {
    generation: u64,
    closed: bool,
    waiter: Option<Waker>,
}

impl Arrival {
    pub(super) fn generation(&self) -> u64 {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).generation
    }

    /// The caller observed Empty after reading `generation`. Publishing a
    /// note between that observation and registration must prevent sleeping.
    pub(super) fn wait_after(&self, generation: u64, waiter: Waker) -> bool {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || state.generation != generation {
            return false;
        }
        let previous = state.waiter.replace(waiter);
        drop(state);
        drop(previous);
        true
    }

    pub(super) fn cancel(&self) {
        let previous = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .waiter
            .take();
        drop(previous);
    }

    fn published(&self, closed: bool) {
        let waiter = {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            state.generation = state.generation.wrapping_add(1);
            state.closed |= closed;
            state.waiter.take()
        };
        // Waking may run arbitrary executor code. Never do so under our lock.
        if let Some(waiter) = waiter {
            waiter.wake();
        }
    }
}

/// Sole producer. Closing the channel precedes the exit wake, including when
/// the decoder unwinds: a woken consumer must be able to observe Disconnected.
pub(super) struct Delivery<T> {
    sender: Option<SyncSender<T>>,
    arrival: Arc<Arrival>,
}

pub(super) fn channel<T>(capacity: usize) -> (Delivery<T>, Receiver<T>, Arc<Arrival>) {
    let (sender, receiver) = mpsc::sync_channel(capacity);
    let arrival = Arc::new(Arrival::default());
    (
        Delivery {
            sender: Some(sender),
            arrival: arrival.clone(),
        },
        receiver,
        arrival,
    )
}

impl<T> Delivery<T> {
    pub(super) fn send(&self, note: T) -> Result<(), SendError<T>> {
        self.sender
            .as_ref()
            .expect("live decoder sender")
            .send(note)?;
        self.arrival.published(false);
        Ok(())
    }
}

impl<T> Drop for Delivery<T> {
    fn drop(&mut self) {
        drop(self.sender.take());
        self.arrival.published(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::TryRecvError;
    use std::task::Wake;

    #[derive(Default)]
    struct Counter(AtomicUsize);

    impl Wake for Counter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn a_delivery_between_empty_and_registration_prevents_sleep() {
        let (sender, notes, arrival) = channel(2);
        let generation = arrival.generation();
        assert_eq!(notes.try_recv(), Err(TryRecvError::Empty));
        sender.send(7).unwrap();
        assert!(!arrival.wait_after(generation, Waker::noop().clone()));
        assert_eq!(notes.try_recv(), Ok(7));
    }

    #[test]
    fn an_armed_wait_wakes_once_and_unarmed_delivery_does_not_wake() {
        let (sender, notes, arrival) = channel(2);
        let counter = Arc::new(Counter::default());
        let generation = arrival.generation();
        assert_eq!(notes.try_recv(), Err(TryRecvError::Empty));
        assert!(arrival.wait_after(generation, Waker::from(counter.clone())));
        sender.send(7).unwrap();
        sender.send(8).unwrap();
        assert_eq!(counter.0.load(Ordering::Relaxed), 1);
        assert_eq!(notes.try_recv(), Ok(7));
        assert_eq!(notes.try_recv(), Ok(8));
        assert!(!arrival.wait_after(generation, Waker::noop().clone()));
    }

    #[test]
    fn cancellation_does_not_leave_a_callback_for_the_old_wait() {
        let (sender, _notes, arrival) = channel(1);
        let counter = Arc::new(Counter::default());
        assert!(arrival.wait_after(arrival.generation(), Waker::from(counter.clone())));
        arrival.cancel();
        sender.send(1).unwrap();
        assert_eq!(counter.0.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn sender_exit_wakes_and_cannot_be_rearmed() {
        let (sender, notes, arrival) = channel::<u8>(1);
        let counter = Arc::new(Counter::default());
        assert!(arrival.wait_after(arrival.generation(), Waker::from(counter.clone())));
        drop(sender);
        assert_eq!(counter.0.load(Ordering::Relaxed), 1);
        assert_eq!(notes.try_recv(), Err(TryRecvError::Disconnected));
        assert!(!arrival.wait_after(arrival.generation(), Waker::noop().clone()));
    }

    #[test]
    fn disconnect_is_observable_inside_the_exit_callback() {
        struct ObserveClose(Mutex<Receiver<u8>>);
        impl Wake for ObserveClose {
            fn wake(self: Arc<Self>) {
                assert_eq!(
                    self.0.lock().unwrap().try_recv(),
                    Err(TryRecvError::Disconnected)
                );
            }
        }
        let (sender, notes, arrival) = channel::<u8>(1);
        let observer = Arc::new(ObserveClose(Mutex::new(notes)));
        assert!(arrival.wait_after(arrival.generation(), Waker::from(observer)));
        drop(sender);
    }

    #[test]
    fn unwinding_decoder_also_closes_and_wakes() {
        let (sender, notes, arrival) = channel::<u8>(1);
        let counter = Arc::new(Counter::default());
        assert!(arrival.wait_after(arrival.generation(), Waker::from(counter.clone())));
        assert!(
            std::panic::catch_unwind(move || {
                let _sender = sender;
                panic!("injected decoder unwind");
            })
            .is_err()
        );
        assert_eq!(counter.0.load(Ordering::Relaxed), 1);
        assert_eq!(notes.try_recv(), Err(TryRecvError::Disconnected));
        assert!(!arrival.wait_after(arrival.generation(), Waker::noop().clone()));
    }
}
