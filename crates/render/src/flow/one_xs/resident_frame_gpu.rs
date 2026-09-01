//! Private capture root for the unselected resident ONE X2 frame chain.
//!
//! A capture is constructed once. Seeking or reopening constructs another
//! capture; there is deliberately no reset operation. The mutex owns the one
//! monotonic generation, exact pending seal, committed successor and future
//! ready capability. GPU work happens only after a linear reservation has
//! taken an immutable snapshot of the committed successor.

use std::sync::{Arc, Mutex, Weak};

use crate::Fallible;
use crate::flow::one_xs::pis::gpu::GpuPisFlight;
use kjerag_media::FrameStamp;

use super::pis_frontend_gpu::RetainedL2DirectionPixelVec2Buffer;

/// Storage installed only after a whole resident frame succeeds.
///
/// Motion owns the first field. Later post-L1 composition can extend this
/// aggregate without exposing a raw allocation through the capture root.
pub(super) struct ResidentSuccessor {
    flight: GpuPisFlight,
    motion_references: wgpu::Buffer,
    _post_l1: Option<ResidentPostL1Storage>,
}

pub(super) struct ResidentPostL1Storage {
    _public: wgpu::Buffer,
    _retained_l2_direction_pixel_vec2: RetainedL2DirectionPixelVec2Buffer,
    _histogram: wgpu::Buffer,
    _fifo: wgpu::Buffer,
    _hints: wgpu::Buffer,
    _lack_rows: wgpu::Buffer,
    _small_rows: wgpu::Buffer,
    _small_present: bool,
    _cadence_counts: [i32; 2],
}

impl ResidentPostL1Storage {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn after_cold(
        public: wgpu::Buffer,
        retained_l2_direction_pixel_vec2: RetainedL2DirectionPixelVec2Buffer,
        histogram: wgpu::Buffer,
        fifo: wgpu::Buffer,
        hints: wgpu::Buffer,
        lack_rows: wgpu::Buffer,
        small_rows: wgpu::Buffer,
        small_present: bool,
        cadence_counts: [i32; 2],
    ) -> Self {
        Self {
            _public: public,
            _retained_l2_direction_pixel_vec2: retained_l2_direction_pixel_vec2,
            _histogram: histogram,
            _fifo: fifo,
            _hints: hints,
            _lack_rows: lack_rows,
            _small_rows: small_rows,
            _small_present: small_present,
            _cadence_counts: cadence_counts,
        }
    }
}

impl ResidentSuccessor {
    pub(super) fn from_motion(flight: GpuPisFlight, motion_references: wgpu::Buffer) -> Self {
        Self {
            flight,
            motion_references,
            _post_l1: None,
        }
    }

    /// Purpose-specific binding for the motion child. The capture API never
    /// exposes this allocation to Scene or another flow sibling.
    pub(super) fn motion_reference(&self) -> &wgpu::Buffer {
        &self.motion_references
    }

    pub(super) fn attach_post_l1(&mut self, storage: ResidentPostL1Storage) -> Fallible<()> {
        if self._post_l1.is_some() {
            return Err("ONE X2 resident successor already owns post-L1 state".into());
        }
        self._post_l1 = Some(storage);
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingSeal {
    generation: u64,
    flight: GpuPisFlight,
}

/// Placeholder for the future resident direct-draw capability. No current
/// production transition can construct or install one.
struct ResidentReadyPlaceholder {
    _private: (),
}

struct RootState {
    generation: u64,
    pending: Option<PendingSeal>,
    committed: Option<Arc<ResidentSuccessor>>,
    ready: Option<ResidentReadyPlaceholder>,
    quarantined: bool,
    quarantined_successors: Vec<Arc<ResidentSuccessor>>,
}

struct SharedRoot {
    state: Mutex<RootState>,
}

#[derive(Clone)]
pub(super) struct GpuResidentIdentity {
    shared: Weak<SharedRoot>,
}

impl GpuResidentIdentity {
    pub(super) fn matches(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.shared, &other.shared)
    }
}

/// Non-cloneable root owner for one open capture.
pub(super) struct GpuResidentCapture {
    shared: Arc<SharedRoot>,
}

/// Linear reservation with the allocation-identical prior snapshot.
#[must_use = "the resident frame reservation must be sealed or aborted"]
pub(super) struct GpuResidentReservation {
    shared: Arc<SharedRoot>,
    seal: PendingSeal,
    prior: Option<Arc<ResidentSuccessor>>,
    active: bool,
}

/// Sealed but unpublished whole-frame successor.
#[must_use = "the resident successor must reach future atomic install or roll back"]
pub(super) struct GpuResidentCandidate {
    reservation: Option<GpuResidentReservation>,
    successor: Arc<ResidentSuccessor>,
    disarmed: bool,
}

impl GpuResidentCapture {
    pub(super) fn new() -> Self {
        Self {
            shared: Arc::new(SharedRoot {
                state: Mutex::new(RootState {
                    generation: 0,
                    pending: None,
                    committed: None,
                    ready: None,
                    quarantined: false,
                    quarantined_successors: Vec::new(),
                }),
            }),
        }
    }

    pub(super) fn reserve(&self, frame: FrameStamp) -> Fallible<GpuResidentReservation> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| "ONE X2 resident capture root is poisoned and quarantined")?;
        if state.quarantined {
            return Err("ONE X2 resident capture root is quarantined".into());
        }
        if state.pending.is_some() {
            return Err("ONE X2 resident capture already has an in-flight frame".into());
        }
        let generation = state
            .generation
            .checked_add(1)
            .ok_or("ONE X2 resident capture generation space is exhausted")?;
        state.generation = generation;
        let seal = PendingSeal {
            generation,
            flight: GpuPisFlight { generation, frame },
        };
        state.pending = Some(seal.clone());
        let prior = state.committed.clone();
        drop(state);
        Ok(GpuResidentReservation {
            shared: Arc::clone(&self.shared),
            seal,
            prior,
            active: true,
        })
    }

    #[cfg(test)]
    pub(super) fn snapshot(&self) -> TestSnapshot {
        let state = self.shared.state.lock().unwrap();
        TestSnapshot {
            generation: state.generation,
            pending: state.pending.is_some(),
            pending_flight: state.pending.as_ref().map(|seal| seal.flight.clone()),
            committed: state.committed.clone(),
            ready: state.ready.is_some(),
            quarantined: state.quarantined,
        }
    }
}

impl GpuResidentReservation {
    pub(super) fn identity(&self) -> GpuResidentIdentity {
        GpuResidentIdentity {
            shared: Arc::downgrade(&self.shared),
        }
    }
    pub(super) fn generation(&self) -> u64 {
        self.seal.generation
    }

    pub(super) fn flight(&self) -> &GpuPisFlight {
        &self.seal.flight
    }

    /// Immutable allocation snapshot used to select cold/warm motion and bind
    /// the exact prior reference. It cannot change while GPU work is encoded.
    pub(super) fn prior(&self) -> &Option<Arc<ResidentSuccessor>> {
        &self.prior
    }

    pub(super) fn seal(self, successor: ResidentSuccessor) -> Fallible<GpuResidentCandidate> {
        if successor.flight != self.seal.flight {
            return Err("ONE X2 resident successor does not match its root reservation".into());
        }
        Ok(GpuResidentCandidate {
            reservation: Some(self),
            successor: Arc::new(successor),
            disarmed: false,
        })
    }

    pub(super) fn abort(mut self) -> Fallible<()> {
        let result = self.rollback();
        self.active = false;
        result
    }

    fn rollback(&mut self) -> Fallible<()> {
        let mut state = match self.shared.state.lock() {
            Ok(state) => state,
            Err(_) => {
                // The root cannot prove any state transition after poison.
                // Retain the prior snapshot for process life and refuse every
                // later reservation through the poisoned mutex.
                if let Some(prior) = self.prior.take() {
                    std::mem::forget(prior);
                }
                return Err("ONE X2 resident capture rollback found a poisoned root; reservation quarantined".into());
            }
        };
        if state.pending.as_ref() == Some(&self.seal) {
            state.pending = None;
            return Ok(());
        }
        // A stale token must never clear a newer pending seal or touch the
        // committed/ready allocations. Quarantine the capture explicitly.
        state.quarantined = true;
        Err(
            "ONE X2 resident capture rollback names a stale reservation; capture quarantined"
                .into(),
        )
    }
}

impl Drop for GpuResidentReservation {
    fn drop(&mut self) {
        if self.active && self.rollback().is_err() {
            // Fail closed. The root is poisoned or quarantined and cannot be
            // reused; retain this allocation snapshot rather than pretending
            // rollback succeeded.
            if let Some(prior) = self.prior.take() {
                std::mem::forget(prior);
            }
        }
    }
}

impl GpuResidentCandidate {
    pub(super) fn successor(&self) -> &Arc<ResidentSuccessor> {
        &self.successor
    }

    pub(super) fn flight(&self) -> &GpuPisFlight {
        &self
            .reservation
            .as_ref()
            .expect("resident candidate retains its reservation")
            .seal
            .flight
    }

    pub(super) fn generation(&self) -> u64 {
        self.reservation
            .as_ref()
            .expect("resident candidate retains its reservation")
            .seal
            .generation
    }

    pub(super) fn abort(mut self) -> Fallible<()> {
        let result = self
            .reservation
            .take()
            .expect("resident candidate retains its reservation")
            .abort();
        self.disarmed = true;
        if result.is_err() {
            std::mem::forget(Arc::clone(&self.successor));
        }
        result
    }

    /// Test-only stand-in for the future atomic successor plus ready install.
    /// It intentionally installs no ready capability.
    #[cfg(test)]
    pub(super) fn install_successor_only_for_test(mut self) -> Fallible<Self> {
        let reservation = self
            .reservation
            .as_mut()
            .expect("resident candidate retains its reservation");
        let mut state = reservation.shared.state.lock().map_err(
            |_| "ONE X2 resident capture install found a poisoned root; candidate quarantined",
        )?;
        let exact_seal = state.pending.as_ref() == Some(&reservation.seal);
        let exact_prior = same_successor(state.committed.as_ref(), reservation.prior.as_ref());
        if !exact_seal || !exact_prior || state.quarantined {
            state.quarantined = true;
            state
                .quarantined_successors
                .push(Arc::clone(&self.successor));
            reservation.active = false;
            self.disarmed = true;
            return Err("ONE X2 resident capture install does not match its seal and prior allocation; candidate quarantined".into());
        }
        state.committed = Some(Arc::clone(&self.successor));
        state.pending = None;
        // `ready` deliberately remains untouched until the future atomic
        // resident draw install exists.
        drop(state);
        reservation.active = false;
        self.disarmed = true;
        Ok(self)
    }
}

impl Drop for GpuResidentCandidate {
    fn drop(&mut self) {
        if self.disarmed {
            return;
        }
        let Some(reservation) = self.reservation.take() else {
            return;
        };
        if reservation.abort().is_err() {
            // A successor whose rollback could not be authenticated remains
            // quarantined for process life; it must not be released as if the
            // root had accepted the rollback.
            std::mem::forget(Arc::clone(&self.successor));
        }
    }
}

fn same_successor(
    left: Option<&Arc<ResidentSuccessor>>,
    right: Option<&Arc<ResidentSuccessor>>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => Arc::ptr_eq(left, right),
        _ => false,
    }
}

#[cfg(test)]
pub(super) struct TestSnapshot {
    pub(super) generation: u64,
    pub(super) pending: bool,
    pub(super) pending_flight: Option<GpuPisFlight>,
    pub(super) committed: Option<Arc<ResidentSuccessor>>,
    pub(super) ready: bool,
    pub(super) quarantined: bool,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use kjerag_media::FrameStamp;

    use super::*;

    #[test]
    fn root_identity_distinguishes_equal_flights_from_distinct_captures() {
        let stamp = frame(7);
        let a = GpuResidentCapture::new().reserve(stamp.clone()).unwrap();
        let b = GpuResidentCapture::new().reserve(stamp).unwrap();
        assert_eq!(a.flight(), b.flight());
        assert!(a.identity().matches(&a.identity()));
        assert!(!a.identity().matches(&b.identity()));
    }

    #[test]
    fn stale_reservation_cannot_clear_a_newer_pending_seal() {
        let capture = GpuResidentCapture::new();
        let first = capture.reserve(frame(1)).unwrap();
        let stale = GpuResidentReservation {
            shared: Arc::clone(&first.shared),
            seal: first.seal.clone(),
            prior: first.prior.clone(),
            active: true,
        };
        first.abort().unwrap();
        let second = capture.reserve(frame(2)).unwrap();
        let second_flight = second.seal.flight.clone();
        drop(stale);
        let state = capture.snapshot();
        assert!(state.pending);
        assert_eq!(state.pending_flight, Some(second_flight));
        assert!(state.quarantined);
        drop(second);
    }

    #[test]
    fn explicit_abort_advances_generation_and_a_new_capture_starts_independently() {
        let capture = GpuResidentCapture::new();
        let first = capture.reserve(frame(1)).unwrap();
        assert_eq!(first.generation(), 1);
        first.abort().unwrap();
        let retry = capture.reserve(frame(1)).unwrap();
        assert_eq!(retry.generation(), 2);
        retry.abort().unwrap();

        let reopened_or_sought = GpuResidentCapture::new();
        let first_after_seek = reopened_or_sought.reserve(frame(1)).unwrap();
        assert_eq!(first_after_seek.generation(), 1);
        first_after_seek.abort().unwrap();
    }

    #[test]
    fn poisoned_root_refuses_reservation_and_abort_fails_closed() {
        let capture = GpuResidentCapture::new();
        let reservation = capture.reserve(frame(1)).unwrap();
        let shared = Arc::clone(&capture.shared);
        let _ = std::panic::catch_unwind(|| {
            let _guard = shared.state.lock().unwrap();
            panic!("poison resident root for test");
        });
        let error = reservation
            .abort()
            .expect_err("poisoned root accepted explicit abort");
        assert!(error.to_string().contains("poisoned root"), "{error}");
        let error = capture
            .reserve(frame(2))
            .err()
            .expect("poisoned root accepted another reservation");
        assert!(error.to_string().contains("poisoned"), "{error}");
    }

    #[test]
    fn install_requires_the_allocation_identical_prior_not_only_its_stamp() {
        let (device, _queue) = match gpu() {
            Ok(gpu) => gpu,
            Err(reason) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {reason}"
                );
                eprintln!("skipping resident allocation identity test: {reason}");
                return;
            }
        };
        let capture = GpuResidentCapture::new();
        let first_flight = flight(1);
        let first = capture
            .reserve(first_flight.frame.clone())
            .unwrap()
            .seal(successor(&device, first_flight.clone()))
            .unwrap();
        let first = first.install_successor_only_for_test().unwrap();
        drop(first);

        let second_flight = flight(2);
        let second = capture.reserve(second_flight.frame.clone()).unwrap();
        let exact_prior = second.prior.as_ref().unwrap().clone();
        let foreign_prior = Arc::new(successor(&device, first_flight));
        assert_eq!(foreign_prior.flight, exact_prior.flight);
        assert!(!Arc::ptr_eq(&foreign_prior, &exact_prior));
        capture.shared.state.lock().unwrap().committed = Some(Arc::clone(&foreign_prior));

        let error = second
            .seal(successor(&device, second_flight))
            .unwrap()
            .install_successor_only_for_test()
            .err()
            .expect("allocation-substituted prior was accepted");
        assert!(error.to_string().contains("prior allocation"), "{error}");
        let state = capture.snapshot();
        assert!(state.quarantined);
        assert!(state.pending);
        assert!(Arc::ptr_eq(
            state.committed.as_ref().unwrap(),
            &foreign_prior
        ));
        assert!(!state.ready);
    }

    fn successor(device: &wgpu::Device, flight: GpuPisFlight) -> ResidentSuccessor {
        ResidentSuccessor::from_motion(
            flight,
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("resident root allocation identity"),
                size: 4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            }),
        )
    }

    fn gpu() -> Result<(wgpu::Device, wgpu::Queue), String> {
        use std::future::Future;

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

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
            .ok_or("no Vulkan adapter")?;
        block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ONE X2 resident capture root identity"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())
    }

    fn flight(index: u64) -> GpuPisFlight {
        GpuPisFlight {
            generation: index,
            frame: FrameStamp::for_test(index, Duration::from_secs(index), None),
        }
    }

    fn frame(index: u64) -> FrameStamp {
        FrameStamp::for_test(index, Duration::from_secs(index), None)
    }
}
