//! Bounded retirement bound to the exact render command buffer.
//!
//! Admission happens before a candidate is installed. Drawing consumes that
//! linear permit and clones the complete installed draw payload here before a
//! binding or draw command can be encoded. The guarded draw closure borrows
//! that stored payload, so Scene and every in-flight redraw can share one
//! `Arc` without separating its resources. The submitted-work callback is
//! attached to that live [`wgpu::RenderPass`], so it cannot accidentally prove
//! an earlier compute prefix. Callbacks only publish generation numbers;
//! ordinary nonblocking polling performs every payload release.
//!
//! The eventual `InstalledOneXsDraw` payload must transitively own the exact
//! `Arc<Frames>` whose imported surfaces the draw samples. Retaining cloned
//! wgpu textures or bind groups is insufficient: duplicating a dmabuf file
//! descriptor does not stop VA-API from reusing and rewriting its surface.

use std::collections::VecDeque;
use std::error::Error;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::Fallible;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawRetirementError {
    Full,
    Quarantined,
    GenerationExhausted,
}

impl std::fmt::Display for DrawRetirementError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(match self {
            Self::Full => "ONE X2 draw retirement is full",
            Self::Quarantined => "ONE X2 draw retirement is quarantined",
            Self::GenerationExhausted => "ONE X2 draw retirement generation was exhausted",
        })
    }
}

impl Error for DrawRetirementError {}

struct Admission {
    available: AtomicUsize,
    capacity: usize,
    closed: AtomicBool,
}

/// One linear reservation made before a draw candidate becomes visible.
///
/// Dropping an uninstalled candidate returns its reservation immediately; it
/// never needs a GPU completion proof because no sampling draw used it.
#[must_use = "a reserved ONE X2 draw permit must be installed or dropped"]
pub(crate) struct DrawPermit {
    admission: Arc<Admission>,
    generation: u64,
    active: bool,
}

impl DrawPermit {
    fn consume(mut self) -> u64 {
        self.active = false;
        self.generation
    }
}

impl Drop for DrawPermit {
    fn drop(&mut self) {
        if self.active && !self.admission.closed.load(Ordering::Acquire) {
            let previous = self.admission.available.fetch_add(1, Ordering::Release);
            debug_assert!(previous < self.admission.capacity);
        }
    }
}

struct Retiring<P> {
    completed_generation: Arc<AtomicU64>,
    generation: u64,
    payload: Option<P>,
}

enum Poller {
    Device(wgpu::Device),
    #[cfg(test)]
    Injected,
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum InjectedPoll {
    Error,
    Panic,
}

/// Private bounded owner for payloads sampled by submitted render passes.
///
/// The owning production adapter consumes a live render pass and returns it
/// only after callback registration succeeds. [`IcedDrawRetirements`] wraps
/// the borrowed adapter with the interior mutability required by iced's
/// `Primitive::draw(&Pipeline, &mut RenderPass)` boundary. Destruction with
/// uncertain pending work is fail-closed: payloads and proof state are
/// retained for process life.
pub(crate) struct DrawRetirements<P> {
    poller: Poller,
    admission: Arc<Admission>,
    pending: VecDeque<Retiring<Arc<P>>>,
    free_signals: Vec<Arc<AtomicU64>>,
    next_generation: u64,
    failed: bool,
    #[cfg(test)]
    injected_poll: Option<InjectedPoll>,
    #[cfg(test)]
    injected_arm_panic: bool,
}

/// Interior-mutability adapter for iced's shared pipeline draw callback.
///
/// A panic poisons the mutex only after the inner owner has quarantined every
/// uncertain payload. Later calls deliberately recover that guard and report
/// the inner terminal error; poison never becomes a route around quarantine.
pub(crate) struct IcedDrawRetirements<P> {
    inner: Mutex<DrawRetirements<P>>,
}

impl<P> IcedDrawRetirements<P> {
    pub(crate) fn new(device: &wgpu::Device, capacity: usize) -> Self {
        Self {
            inner: Mutex::new(DrawRetirements::new(device, capacity)),
        }
    }

    pub(crate) fn reserve(&self) -> Result<DrawPermit, DrawRetirementError> {
        self.lock().reserve()
    }

    pub(crate) fn poll(&self) -> Fallible<usize> {
        self.lock().poll()
    }

    /// Collect callbacks after another owner has already driven this exact
    /// device once. This is used by the capture-wide resident transaction so
    /// its validity word and draw retirements share one nonblocking device
    /// poll per redraw.
    pub(crate) fn collect_after_external_poll(&self) -> Fallible<usize> {
        self.lock().collect_completed()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lock().pending.is_empty()
    }

    /// A shared device poll failed while resident draws may still be in
    /// flight. Keep every uncertain payload for process life without replacing
    /// the failure site's original error.
    pub(crate) fn quarantine_after_external_poll_failure(&self) {
        self.lock().quarantine_all();
    }

    pub(crate) fn arm_and_draw<'pass>(
        &self,
        permit: DrawPermit,
        pass: &mut wgpu::RenderPass<'pass>,
        payload: Arc<P>,
        draw: impl FnOnce(&P, &mut wgpu::RenderPass<'pass>),
    ) {
        self.lock()
            .arm_and_draw_borrowed(permit, pass, payload, draw);
    }

    fn lock(&self) -> MutexGuard<'_, DrawRetirements<P>> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

impl<P> DrawRetirements<P> {
    pub(crate) fn new(device: &wgpu::Device, capacity: usize) -> Self {
        Self::with_poller(Poller::Device(device.clone()), capacity)
    }

    fn with_poller(poller: Poller, capacity: usize) -> Self {
        assert!(
            capacity > 0,
            "ONE X2 draw retirement capacity must be nonzero"
        );
        let mut free_signals = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            free_signals.push(Arc::new(AtomicU64::new(0)));
        }
        Self {
            poller,
            admission: Arc::new(Admission {
                available: AtomicUsize::new(capacity),
                capacity,
                closed: AtomicBool::new(false),
            }),
            pending: VecDeque::with_capacity(capacity),
            free_signals,
            next_generation: 1,
            failed: false,
            #[cfg(test)]
            injected_poll: None,
            #[cfg(test)]
            injected_arm_panic: false,
        }
    }

    /// Reserve capacity before history or successor state is committed.
    ///
    /// A full or quarantined owner refuses here, before a candidate payload is
    /// moved. Repeated redraws reserve a fresh permit without implying another
    /// history transition.
    pub(crate) fn reserve(&mut self) -> Result<DrawPermit, DrawRetirementError> {
        if self.failed {
            return Err(DrawRetirementError::Quarantined);
        }
        if self.next_generation == 0 {
            return Err(DrawRetirementError::GenerationExhausted);
        }
        self.admission
            .available
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |available| {
                available.checked_sub(1)
            })
            .map_err(|_| DrawRetirementError::Full)?;
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1);
        Ok(DrawPermit {
            admission: Arc::clone(&self.admission),
            generation,
            active: true,
        })
    }

    /// Install proof on this exact render command buffer before any sampling
    /// bind or draw, retain the opaque installed-draw payload, then let the
    /// closure bind and draw through that stored payload.
    pub(crate) fn arm_and_draw<'pass>(
        &mut self,
        permit: DrawPermit,
        mut pass: wgpu::RenderPass<'pass>,
        payload: Arc<P>,
        draw: impl FnOnce(&P, &mut wgpu::RenderPass<'pass>),
    ) -> wgpu::RenderPass<'pass> {
        self.arm_and_draw_borrowed(permit, &mut pass, payload, draw);
        pass
    }

    /// Iced-compatible form of [`Self::arm_and_draw`] for the render pass its
    /// shader primitive receives by mutable reference.
    ///
    /// Registration still precedes the guarded draw closure on this exact
    /// pass. A registration or draw panic quarantines every uncertain owner
    /// before resuming the panic. Unlike the owning adapter, this method
    /// cannot consume the caller's pass, so fail-closed retention is what
    /// makes any outer `catch_unwind` safe: even a later encoded draw cannot
    /// release or reuse its exact source owner.
    pub(crate) fn arm_and_draw_borrowed<'pass>(
        &mut self,
        permit: DrawPermit,
        pass: &mut wgpu::RenderPass<'pass>,
        payload: Arc<P>,
        draw: impl FnOnce(&P, &mut wgpu::RenderPass<'pass>),
    ) {
        if self.failed
            || !permit.active
            || !Arc::ptr_eq(&permit.admission, &self.admission)
            || self.free_signals.is_empty()
        {
            // No draw has been encoded through the consumed pass. Retain the
            // offered owner conservatively and close this malformed owner.
            std::mem::forget(payload);
            let _ = permit.consume();
            self.quarantine_all();
            panic!("ONE X2 draw retirement received an invalid permit");
        }

        let completed_generation = self
            .free_signals
            .pop()
            .expect("validated draw permit has one reserved callback slot");
        completed_generation.store(0, Ordering::Relaxed);
        let generation = permit.consume();
        let retiring = Retiring {
            completed_generation: Arc::clone(&completed_generation),
            generation,
            payload: Some(payload),
        };
        #[cfg(test)]
        let injected_arm_panic = std::mem::take(&mut self.injected_arm_panic);
        let armed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            #[cfg(test)]
            if injected_arm_panic {
                panic!("injected ONE X2 draw callback registration panic");
            }
            pass.on_submitted_work_done(move || {
                completed_generation.store(generation, Ordering::Release);
            });
        }));
        if let Err(panic_payload) = armed {
            // Registration may have partially succeeded. The offered payload
            // and all older uncertain payloads enter one fail-closed set before
            // panic resumes. In the borrowed adapter the pass remains with its
            // caller, but every source it could sample is retained forever.
            self.pending.push_back(retiring);
            self.quarantine_all();
            std::panic::resume_unwind(panic_payload);
        }
        self.pending.push_back(retiring);
        let payload = self
            .pending
            .back()
            .and_then(|retiring| retiring.payload.as_deref())
            .expect("armed draw retirement retains its payload");
        let encoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            draw(payload, pass);
        }));
        if let Err(panic_payload) = encoded {
            // The pass may contain a prefix of the requested draw, and its
            // command buffer may never be submitted after unwind. No callback
            // can safely release any owner in that state.
            self.quarantine_all();
            std::panic::resume_unwind(panic_payload);
        }
    }

    /// Drive callbacks without waiting, then release only exact generations
    /// whose render-command callback has run.
    pub(crate) fn poll(&mut self) -> Fallible<usize> {
        if self.failed {
            return Err("ONE X2 draw retirement is quarantined".into());
        }
        if self.pending.is_empty() {
            return Ok(0);
        }
        let polled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            #[cfg(test)]
            if let Some(outcome) = self.injected_poll.take() {
                return match outcome {
                    InjectedPoll::Error => Err("injected ONE X2 draw poll failure".into()),
                    InjectedPoll::Panic => panic!("injected ONE X2 draw poll panic"),
                };
            }
            match &self.poller {
                Poller::Device(device) => device
                    .poll(wgpu::PollType::Poll)
                    .map(|_| ())
                    .map_err(Box::<dyn Error + Send + Sync>::from),
                #[cfg(test)]
                Poller::Injected => Ok(()),
            }
        }));
        match polled {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.quarantine_all();
                return Err(error);
            }
            Err(panic_payload) => {
                self.quarantine_all();
                let message = panic_payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| panic_payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("ONE X2 draw retirement poll panicked");
                return Err(message.to_owned().into());
            }
        }

        self.collect_completed()
    }

    fn collect_completed(&mut self) -> Fallible<usize> {
        if self.failed {
            return Err("ONE X2 draw retirement is quarantined".into());
        }
        let mut released = 0;
        let count = self.pending.len();
        for _ in 0..count {
            let mut item = self
                .pending
                .pop_front()
                .expect("ONE X2 draw retirement count came from this queue");
            if item.completed_generation.load(Ordering::Acquire) == item.generation {
                drop(item.payload.take());
                item.completed_generation.store(0, Ordering::Relaxed);
                self.free_signals.push(item.completed_generation);
                self.admission.available.fetch_add(1, Ordering::Release);
                released += 1;
            } else {
                self.pending.push_back(item);
            }
        }
        Ok(released)
    }

    fn quarantine_all(&mut self) {
        self.admission.closed.store(true, Ordering::Release);
        for mut item in self.pending.drain(..) {
            if let Some(payload) = item.payload.take() {
                std::mem::forget(payload);
            }
            std::mem::forget(item);
        }
        self.free_signals.clear();
        self.failed = true;
    }

    #[cfg(test)]
    fn injected(capacity: usize) -> Self {
        Self::with_poller(Poller::Injected, capacity)
    }

    #[cfg(test)]
    fn arm_injected(&mut self, permit: DrawPermit, payload: Arc<P>) {
        assert!(Arc::ptr_eq(&permit.admission, &self.admission));
        let completed_generation = self
            .free_signals
            .pop()
            .expect("reserved injected draw has a callback slot");
        completed_generation.store(0, Ordering::Relaxed);
        self.pending.push_back(Retiring {
            completed_generation,
            generation: permit.consume(),
            payload: Some(payload),
        });
    }

    #[cfg(test)]
    fn prove_for_test(&self, index: usize) {
        let item = &self.pending[index];
        item.completed_generation
            .store(item.generation, Ordering::Release);
    }

    #[cfg(test)]
    fn inject_poll(&mut self, outcome: InjectedPoll) {
        self.injected_poll = Some(outcome);
    }

    #[cfg(test)]
    fn inject_arm_panic(&mut self) {
        self.injected_arm_panic = true;
    }
}

impl<P> Drop for DrawRetirements<P> {
    fn drop(&mut self) {
        if !self.failed {
            // The exact render command buffer may never have been submitted,
            // so a wait cannot prove safety and may never return. Exceptional
            // destruction therefore retains every uncertain owner.
            self.quarantine_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};

    use super::*;

    struct SourceOwnerDrop {
        id: u8,
        dropped: mpsc::Sender<u8>,
    }

    impl Drop for SourceOwnerDrop {
        fn drop(&mut self) {
            let _ = self.dropped.send(self.id);
        }
    }

    struct InstalledDraw {
        _sampled_texture: Option<wgpu::Texture>,
        pipeline: Option<wgpu::RenderPipeline>,
        _exact_source_owner: Arc<SourceOwnerDrop>,
    }

    struct PreparedCandidate {
        permit: DrawPermit,
        payload: Arc<InstalledDraw>,
    }

    fn owner(id: u8, dropped: &mpsc::Sender<u8>) -> Arc<SourceOwnerDrop> {
        Arc::new(SourceOwnerDrop {
            id,
            dropped: dropped.clone(),
        })
    }

    fn installed(id: u8, dropped: &mpsc::Sender<u8>) -> Arc<InstalledDraw> {
        Arc::new(InstalledDraw {
            _sampled_texture: None,
            pipeline: None,
            _exact_source_owner: owner(id, dropped),
        })
    }

    #[test]
    fn bounded_permits_precede_install_and_unused_candidate_needs_no_proof() {
        let (dropped, answer) = mpsc::channel();
        let mut retirements = DrawRetirements::<InstalledDraw>::injected(1);
        let candidate = PreparedCandidate {
            permit: retirements.reserve().unwrap(),
            payload: installed(2, &dropped),
        };
        let offered = owner(1, &dropped);
        let error = match retirements.reserve() {
            Ok(_) => panic!("full draw retirement admitted a successor"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "ONE X2 draw retirement is full");
        assert!(matches!(answer.try_recv(), Err(mpsc::TryRecvError::Empty)));
        drop(offered);
        assert_eq!(answer.recv().unwrap(), 1, "refusal moved the payload");

        // A prepared candidate that was never drawn simply releases its source
        // and returns admission without asking for retirement proof.
        drop(candidate);
        assert_eq!(answer.recv().unwrap(), 2);
        let replacement = retirements.reserve().unwrap();
        drop(replacement);
    }

    #[test]
    fn generations_release_out_of_order_once_and_reject_stale_aba() {
        let (dropped, answer) = mpsc::channel();
        let mut retirements = DrawRetirements::injected(3);
        for id in 1..=3 {
            let permit = retirements.reserve().unwrap();
            retirements.arm_injected(permit, installed(id, &dropped));
        }
        let stale_signal = Arc::clone(&retirements.pending[1].completed_generation);
        let stale_generation = retirements.pending[1].generation;
        retirements.prove_for_test(1);
        assert_eq!(retirements.poll().unwrap(), 1);
        assert_eq!(answer.recv().unwrap(), 2);
        assert_eq!(retirements.poll().unwrap(), 0);

        let permit = retirements.reserve().unwrap();
        retirements.arm_injected(permit, installed(4, &dropped));
        let reused = retirements.pending.len() - 1;
        assert_eq!(
            Arc::as_ptr(&stale_signal),
            Arc::as_ptr(&retirements.pending[reused].completed_generation)
        );
        stale_signal.store(stale_generation, Ordering::Release);
        assert_eq!(retirements.poll().unwrap(), 0, "stale callback passed ABA");

        for index in 0..retirements.pending.len() {
            retirements.prove_for_test(index);
        }
        assert_eq!(retirements.poll().unwrap(), 3);
        let mut ids = [
            answer.recv().unwrap(),
            answer.recv().unwrap(),
            answer.recv().unwrap(),
        ];
        ids.sort();
        assert_eq!(ids, [1, 3, 4]);
    }

    #[test]
    fn poll_failure_and_panic_quarantine_source_owners() {
        for outcome in [InjectedPoll::Error, InjectedPoll::Panic] {
            let (dropped, answer) = mpsc::channel();
            let mut retirements = DrawRetirements::injected(1);
            let permit = retirements.reserve().unwrap();
            retirements.arm_injected(permit, installed(1, &dropped));
            retirements.inject_poll(outcome);
            let error = retirements.poll().unwrap_err();
            let expected = match outcome {
                InjectedPoll::Error => "injected ONE X2 draw poll failure",
                InjectedPoll::Panic => "injected ONE X2 draw poll panic",
            };
            assert_eq!(error.to_string(), expected);
            drop(retirements);
            assert!(matches!(answer.try_recv(), Err(mpsc::TryRecvError::Empty)));
        }
    }

    #[test]
    fn repeated_redraw_reserves_fresh_permit_without_recommitting_history() {
        let (dropped, answer) = mpsc::channel();
        let mut history_commits = 0;
        let mut retirements = DrawRetirements::injected(1);

        history_commits += 1;
        let mut scene_installed = installed(1, &dropped);
        let permit = retirements.reserve().unwrap();
        retirements.arm_injected(permit, Arc::clone(&scene_installed));
        retirements.prove_for_test(0);
        assert_eq!(retirements.poll().unwrap(), 1);
        assert!(matches!(answer.try_recv(), Err(mpsc::TryRecvError::Empty)));

        // A redraw clones the same installed transaction and reserves a new
        // permit. Replacing Scene's Arc cannot release that old source while
        // its repeat draw is still awaiting exact render-pass proof.
        let permit = retirements.reserve().unwrap();
        retirements.arm_injected(permit, Arc::clone(&scene_installed));
        assert_eq!(history_commits, 1);
        history_commits += 1;
        scene_installed = installed(2, &dropped);
        assert!(matches!(answer.try_recv(), Err(mpsc::TryRecvError::Empty)));
        retirements.prove_for_test(0);
        assert_eq!(retirements.poll().unwrap(), 1);
        assert_eq!(answer.recv().unwrap(), 1);
        assert_eq!(history_commits, 2);
        drop(scene_installed);
        assert_eq!(answer.recv().unwrap(), 2);
    }

    #[test]
    fn render_pass_callback_holds_exact_source_until_nonblocking_poll() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => {
                eprintln!("skipping render-pass retirement test: {error}");
                return;
            }
            Err(error) => panic!("Vulkan GPU required for draw retirement: {error}"),
        };
        eprintln!("draw retirement adapter: {adapter}");
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ONE X2 draw retirement target"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let (dropped, answer) = mpsc::channel();
        let exact_owner = owner(7, &dropped);
        let retained = Arc::downgrade(&exact_owner);
        let mut retirements = DrawRetirements::new(&device, 1);
        // Prepare admission precedes candidate installation.
        let candidate = PreparedCandidate {
            permit: retirements.reserve().unwrap(),
            payload: Arc::new(InstalledDraw {
                _sampled_texture: Some(target.clone()),
                pipeline: Some(test_pipeline(&device)),
                _exact_source_owner: Arc::clone(&exact_owner),
            }),
        };
        drop(exact_owner);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 exact draw retirement"),
        });
        let view = target.create_view(&Default::default());
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ONE X2 exact sampling draw"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        // Arm returns the pass only after its exact command-buffer callback is
        // installed. Binding and drawing are therefore necessarily later.
        let pass = retirements.arm_and_draw(
            candidate.permit,
            pass,
            candidate.payload,
            |installed, pass| {
                pass.set_pipeline(
                    installed
                        .pipeline
                        .as_ref()
                        .expect("real installed draw owns its pipeline"),
                );
                pass.draw(0..3, 0..1);
            },
        );
        drop(pass);
        queue.submit([encoder.finish()]);
        assert!(retained.upgrade().is_some());
        assert!(matches!(answer.try_recv(), Err(mpsc::TryRecvError::Empty)));

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            retirements.poll().unwrap();
            if retained.upgrade().is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            retained.upgrade().is_none(),
            "draw callback never retired source"
        );
        assert_eq!(answer.recv().unwrap(), 7);
    }

    #[test]
    fn registration_panic_quarantines_prior_and_offered_before_any_draw() {
        let (device, _queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => {
                eprintln!("skipping render-pass panic test: {error}");
                return;
            }
            Err(error) => panic!("Vulkan GPU required for draw panic: {error}"),
        };
        eprintln!("draw panic adapter: {adapter}");
        let (dropped, answer) = mpsc::channel();
        let mut retirements = DrawRetirements::new(&device, 3);
        let prior = retirements.reserve().unwrap();
        retirements.arm_injected(prior, installed(1, &dropped));
        let offered = retirements.reserve().unwrap();
        let retained_permit = retirements.reserve().unwrap();
        retirements.inject_arm_panic();

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ONE X2 panic target"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 panic before draw"),
        });
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ONE X2 panic before binding"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let encoded_draw = Arc::new(AtomicU8::new(0));
        let encoded_after_arm = Arc::clone(&encoded_draw);
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _pass = retirements.arm_and_draw(
                offered,
                pass,
                Arc::new(InstalledDraw {
                    _sampled_texture: None,
                    pipeline: None,
                    _exact_source_owner: owner(2, &dropped),
                }),
                |_installed, pass| {
                    encoded_after_arm.store(1, Ordering::SeqCst);
                    pass.draw(0..3, 0..1);
                },
            );
        }))
        .expect_err("injected draw callback registration did not panic");
        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&"injected ONE X2 draw callback registration panic")
        );
        assert_eq!(encoded_draw.load(Ordering::SeqCst), 0);
        assert!(retirements.failed);
        assert!(retirements.pending.is_empty());
        assert!(matches!(answer.try_recv(), Err(mpsc::TryRecvError::Empty)));
        assert_eq!(retirements.admission.available.load(Ordering::Acquire), 0);
        drop(retained_permit);
        assert_eq!(retirements.admission.available.load(Ordering::Acquire), 0);
        let error = match retirements.reserve() {
            Ok(_) => panic!("quarantined draw retirement admitted another permit"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "ONE X2 draw retirement is quarantined");
        drop(retirements);
        assert!(matches!(answer.try_recv(), Err(mpsc::TryRecvError::Empty)));
        // Finishing proves the failed arm dropped its pass borrow. No draw was
        // encoded, and this command buffer is deliberately not submitted.
        let _ = encoder.finish();
    }

    #[test]
    fn never_submitted_draw_keeps_backpressure_and_source_owner() {
        let (device, _queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => {
                eprintln!("skipping never-submitted draw test: {error}");
                return;
            }
            Err(error) => panic!("Vulkan GPU required for never-submitted draw: {error}"),
        };
        eprintln!("never-submitted draw adapter: {adapter}");
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ONE X2 never-submitted target"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let (dropped, answer) = mpsc::channel();
        let owner = owner(9, &dropped);
        let installed = Arc::new(InstalledDraw {
            _sampled_texture: Some(target.clone()),
            pipeline: Some(test_pipeline(&device)),
            _exact_source_owner: Arc::clone(&owner),
        });
        drop(owner);
        let retirements = IcedDrawRetirements::new(&device, 1);
        let permit = retirements.reserve().unwrap();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 never-submitted encoder"),
        });
        let view = target.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ONE X2 never-submitted pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        retirements.arm_and_draw(
            permit,
            &mut pass,
            Arc::clone(&installed),
            |installed, pass| {
                pass.set_pipeline(installed.pipeline.as_ref().unwrap());
                pass.draw(0..3, 0..1);
            },
        );
        drop(pass);
        let _never_submitted = encoder.finish();
        let error = match retirements.reserve() {
            Ok(_) => panic!("never-submitted draw released backpressure"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "ONE X2 draw retirement is full");
        drop(installed);
        drop(retirements);
        assert!(
            matches!(answer.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "unsubmitted draw returned its uncertain source owner"
        );
    }

    #[test]
    fn draw_panic_quarantines_stored_source_owner() {
        let (device, _queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(error) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => {
                eprintln!("skipping draw-panic quarantine test: {error}");
                return;
            }
            Err(error) => panic!("Vulkan GPU required for draw panic: {error}"),
        };
        eprintln!("draw-panic quarantine adapter: {adapter}");
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ONE X2 draw-panic target"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let (dropped, answer) = mpsc::channel();
        let installed = Arc::new(InstalledDraw {
            _sampled_texture: Some(target.clone()),
            pipeline: Some(test_pipeline(&device)),
            _exact_source_owner: owner(10, &dropped),
        });
        let retirements = IcedDrawRetirements::new(&device, 1);
        let permit = retirements.reserve().unwrap();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 draw-panic encoder"),
        });
        let view = target.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ONE X2 draw-panic pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            retirements.arm_and_draw(
                permit,
                &mut pass,
                Arc::clone(&installed),
                |installed, pass| {
                    pass.set_pipeline(installed.pipeline.as_ref().unwrap());
                    panic!("injected ONE X2 stored draw panic");
                },
            );
        }))
        .expect_err("injected stored draw did not panic");
        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&"injected ONE X2 stored draw panic")
        );
        let error = match retirements.reserve() {
            Ok(_) => panic!("poisoned iced draw retirement admitted another permit"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "ONE X2 draw retirement is quarantined");
        drop(pass);
        drop(installed);
        drop(retirements);
        assert!(
            matches!(answer.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "draw panic returned the stored uncertain source owner"
        );
        let _ = encoder.finish();
    }

    fn test_pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 draw retirement shader"),
            source: wgpu::ShaderSource::Wgsl(
                "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {\n\
                 var p = array(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));\n\
                 return vec4f(p[i], 0.0, 1.0);\n\
                 }\n\
                 @fragment fn fs() -> @location(0) vec4f { return vec4f(1.0); }"
                    .into(),
            ),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 draw retirement layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ONE X2 draw retirement pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

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

    fn gpu() -> Result<(wgpu::Device, wgpu::Queue, String), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
            .ok_or("no Vulkan adapter")?;
        let name = adapter.get_info().name;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("exact ONE X2 draw retirement"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        Ok((device, queue, name))
    }
}
