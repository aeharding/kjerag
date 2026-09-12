//! Private capture root for the selected resident ONE X2 frame chain.
//!
//! A capture is constructed once. Seeking or reopening constructs another
//! capture; there is deliberately no reset operation. The mutex owns the one
//! monotonic generation, exact pending seal, committed successor, one future
//! draw and the ready capability. GPU work happens only after a linear
//! reservation has taken an immutable snapshot of the committed successor.

use std::sync::{Arc, Mutex, Weak};

use crate::Fallible;
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
use crate::flow::one_xs::pis::gpu::GpuPisFlight;
use kjerag_media::FrameStamp;

use super::geometry_gpu::temporal_gpu::GpuPairedCadence;
use super::pis_frontend_gpu::{
    GpuWorkModeBinding, GpuWorkModePipeline, RetainedL2DirectionPixelVec2Buffer,
};
use super::{InstalledOneXsDraw, InstalledOneXsPass, InstalledOneXsReady, ResidentSourceIdentity};
use crate::draw_retirement::{DrawRetirementError, IcedDrawRetirements};
use crate::flow::one_xs::pis::Level;

/// Storage installed only after a whole resident frame succeeds.
///
/// Motion owns the first field. Later post-L1 composition can extend this
/// aggregate without exposing a raw allocation through the capture root.
pub(super) struct ResidentSuccessor {
    flight: GpuPisFlight,
    motion_references: wgpu::Buffer,
    post_l1: Option<ResidentPostL1Storage>,
}

pub(super) struct ResidentPostL1Storage {
    public: wgpu::Buffer,
    retained_l2_direction_pixel_vec2: RetainedL2DirectionPixelVec2Buffer,
    histogram: wgpu::Buffer,
    fifo: wgpu::Buffer,
    hints: wgpu::Buffer,
    lack_rows: wgpu::Buffer,
    small_rows: wgpu::Buffer,
    small_present: bool,
    cadence: GpuPairedCadence,
    calculation: u8,
}

impl ResidentPostL1Storage {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        public: wgpu::Buffer,
        retained_l2_direction_pixel_vec2: RetainedL2DirectionPixelVec2Buffer,
        histogram: wgpu::Buffer,
        fifo: wgpu::Buffer,
        hints: wgpu::Buffer,
        lack_rows: wgpu::Buffer,
        small_rows: wgpu::Buffer,
        small_present: bool,
        cadence: GpuPairedCadence,
        calculation: u8,
    ) -> Self {
        Self {
            public,
            retained_l2_direction_pixel_vec2,
            histogram,
            fifo,
            hints,
            lack_rows,
            small_rows,
            small_present,
            cadence,
            calculation,
        }
    }
}

/// Allocation-identical immutable view of one installed predecessor.
///
/// The private constructor lives on [`GpuResidentReservation`], whose `prior`
/// field is cloned from the capture root while its pending seal is created.
/// No sibling can assemble this capability from handles that merely happen to
/// have the right sizes.
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct InstalledResidentPrior {
    successor: Arc<ResidentSuccessor>,
    context: OneXsGpuContext,
}

impl InstalledResidentPrior {
    fn post_l1(&self) -> &ResidentPostL1Storage {
        self.successor
            .post_l1
            .as_ref()
            .expect("installed-prior constructor checked post-L1 state")
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn validate_post_l1(&self) -> Fallible<()> {
        let post = self.post_l1();
        const EXPECTED: [(&str, u64); 7] = [
            ("public", (2 * 1080 * 60 * 2 * 4) as u64),
            ("retained L2", (2 * 270 * 15 * 2 * 4) as u64),
            ("histogram", (2 * 178 * 8 * 159 * 4) as u64),
            ("FIFO", (2 * 178 * 8 * 5 * 4) as u64),
            ("hints", (275_400 * 4) as u64),
            ("lack rows", (2 * 178 * 4) as u64),
            ("small rows", (2 * 178 * 4) as u64),
        ];
        let actual = [
            post.public.size(),
            post.retained_l2_direction_pixel_vec2.buffer().size(),
            post.histogram.size(),
            post.fifo.size(),
            post.hints.size(),
            post.lack_rows.size(),
            post.small_rows.size(),
        ];
        for ((name, expected), actual) in EXPECTED.into_iter().zip(actual) {
            if actual != expected {
                return Err(format!(
                    "ONE X2 installed warm {name} buffer is {actual} bytes, expected {expected}"
                )
                .into());
            }
        }
        Ok(())
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn context(&self) -> &OneXsGpuContext {
        &self.context
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn cadence(&self) -> GpuPairedCadence {
        self.post_l1().cadence
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn calculation(&self) -> u8 {
        self.post_l1().calculation
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn small_present(&self) -> bool {
        self.post_l1().small_present
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn bind_warm_post_l1(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        terminal: &wgpu::Buffer,
        images: &wgpu::Buffer,
        motion_l1: &wgpu::Buffer,
        histogram: &wgpu::Buffer,
        fifo: &wgpu::Buffer,
        hints: &wgpu::Buffer,
        filtered: &wgpu::Buffer,
        retained_l1: &wgpu::Buffer,
        dense_l1: &wgpu::Buffer,
        horizontal: &wgpu::Buffer,
        public: &wgpu::Buffer,
        quantized_values: &wgpu::Buffer,
        retained_l2: &wgpu::Buffer,
        validity: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let post = self.post_l1();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 installed warm post-L1 owner"),
            layout,
            entries: &[
                buffer_entry(0, terminal),
                buffer_entry(1, images),
                buffer_entry(3, histogram),
                buffer_entry(4, fifo),
                buffer_entry(5, &post.public),
                buffer_entry(6, motion_l1),
                buffer_entry(7, hints),
                buffer_entry(9, filtered),
                buffer_entry(10, retained_l1),
                buffer_entry(11, dense_l1),
                buffer_entry(12, horizontal),
                buffer_entry(13, public),
                buffer_entry(14, quantized_values),
                buffer_entry(16, retained_l2),
                buffer_entry(17, validity),
            ],
        })
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn encode_warm_history_copies(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        histogram: &wgpu::Buffer,
        fifo: &wgpu::Buffer,
    ) {
        let post = self.post_l1();
        encoder.copy_buffer_to_buffer(&post.histogram, 0, histogram, 0, post.histogram.size());
        encoder.copy_buffer_to_buffer(&post.fifo, 0, fifo, 0, post.fifo.size());
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn bind_small_row_classifier(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        filtered: &wgpu::Buffer,
        common_a_block_mask: &wgpu::Buffer,
        config: &wgpu::Buffer,
        candidates: &wgpu::Buffer,
        rows: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 installed warm small-row owner"),
            layout,
            entries: &[
                buffer_entry(0, filtered),
                buffer_entry(1, common_a_block_mask),
                buffer_entry(2, &self.post_l1().small_rows),
                buffer_entry(3, config),
                buffer_entry(4, candidates),
                buffer_entry(5, rows),
            ],
        })
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn encode_same_flight_lack_rows(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        current_l1_lack: &wgpu::Buffer,
        successor_lack: &wgpu::Buffer,
    ) {
        let post = self.post_l1();
        let counts = post.cadence.counts();
        let direction_bytes = (Level::One.patch_rows() * size_of::<u32>()) as u64;
        for (direction, count) in counts.into_iter().enumerate() {
            let offset = direction as u64 * direction_bytes;
            encoder.copy_buffer_to_buffer(
                if count == 0 {
                    current_l1_lack
                } else {
                    &post.lack_rows
                },
                offset,
                successor_lack,
                offset,
                direction_bytes,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn bind_l2_bridge(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        config: &wgpu::Buffer,
        images: &wgpu::Buffer,
        terminal: &wgpu::Buffer,
        motion_l2: &wgpu::Buffer,
        output: &wgpu::Buffer,
        validity: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let post = self.post_l1();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 installed warm prior-public L2 owner"),
            layout,
            entries: &[
                buffer_entry(0, config),
                buffer_entry(1, images),
                buffer_entry(2, terminal),
                buffer_entry(3, post.retained_l2_direction_pixel_vec2.buffer()),
                buffer_entry(4, motion_l2),
                buffer_entry(5, output),
                buffer_entry(6, validity),
            ],
        })
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn bind_hints(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        config: &wgpu::Buffer,
        dynamic: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 installed warm prior-public hint owner"),
            layout,
            entries: &[
                buffer_entry(0, config),
                buffer_entry(1, &self.post_l1().hints),
                buffer_entry(2, dynamic),
            ],
        })
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn bind_warm_work_modes(
        &self,
        pipeline: &GpuWorkModePipeline,
        dynamic: &wgpu::Buffer,
        successor_lack: &wgpu::Buffer,
        level: Level,
        flight: &GpuPisFlight,
    ) -> GpuWorkModeBinding {
        pipeline.bind_warm(
            dynamic,
            successor_lack,
            &self.post_l1().small_rows,
            level,
            flight,
        )
    }

    #[cfg(test)]
    pub(super) fn same_successor(&self, successor: &Arc<ResidentSuccessor>) -> bool {
        Arc::ptr_eq(&self.successor, successor)
    }
}

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

impl ResidentSuccessor {
    pub(super) fn from_motion(flight: GpuPisFlight, motion_references: wgpu::Buffer) -> Self {
        Self {
            flight,
            motion_references,
            post_l1: None,
        }
    }

    /// Purpose-specific binding for the motion child. The capture API never
    /// exposes this allocation to Scene or another flow sibling.
    pub(super) fn motion_reference(&self) -> &wgpu::Buffer {
        &self.motion_references
    }

    pub(super) fn flight(&self) -> &GpuPisFlight {
        &self.flight
    }

    pub(super) fn attach_post_l1(&mut self, storage: ResidentPostL1Storage) -> Fallible<()> {
        if self.post_l1.is_some() {
            return Err("ONE X2 resident successor already owns post-L1 state".into());
        }
        self.post_l1 = Some(storage);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn cadence_for_test(&self) -> [i32; 2] {
        self.post_l1
            .as_ref()
            .expect("installed successor retains post-L1 state")
            .cadence
            .counts()
    }

    #[cfg(test)]
    pub(super) fn calculation_for_test(&self) -> u8 {
        self.post_l1
            .as_ref()
            .expect("installed successor retains post-L1 state")
            .calculation
    }

    #[cfg(test)]
    pub(super) fn successor_fingerprint_for_test(
        &self,
        context: &OneXsGpuContext,
    ) -> Fallible<TestSuccessorFingerprint> {
        let post = self
            .post_l1
            .as_ref()
            .ok_or("installed successor has no post-L1 state")?;
        Ok(TestSuccessorFingerprint {
            motion_references: read_words(context, &self.motion_references)?,
            public: read_words(context, &post.public)?,
            retained_l2: read_words(context, post.retained_l2_direction_pixel_vec2.buffer())?,
            histogram: read_words(context, &post.histogram)?,
            fifo: read_words(context, &post.fifo)?,
            hints: read_words(context, &post.hints)?,
            lack_rows: read_words(context, &post.lack_rows)?,
            small_rows: read_words(context, &post.small_rows)?,
            small_present: post.small_present,
            cadence: post.cadence.counts(),
            calculation: post.calculation,
        })
    }

    #[cfg(test)]
    pub(super) fn replace_installed_cadence_for_test(
        &mut self,
        cadence: GpuPairedCadence,
    ) -> Fallible<()> {
        self.post_l1
            .as_mut()
            .ok_or("resident successor has no installed post-L1 state")?
            .cadence = cadence;
        Ok(())
    }
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
pub(super) struct TestSuccessorFingerprint {
    motion_references: Vec<u32>,
    public: Vec<u32>,
    retained_l2: Vec<u32>,
    histogram: Vec<u32>,
    fifo: Vec<u32>,
    hints: Vec<u32>,
    lack_rows: Vec<u32>,
    small_rows: Vec<u32>,
    small_present: bool,
    cadence: [i32; 2],
    calculation: u8,
}

#[cfg(test)]
fn read_words(context: &OneXsGpuContext, source: &wgpu::Buffer) -> Fallible<Vec<u32>> {
    use std::sync::mpsc;
    let readback = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("ONE X2 installed post-L1 fingerprint"),
        size: source.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 installed post-L1 fingerprint"),
        });
    encoder.copy_buffer_to_buffer(source, 0, &readback, 0, source.size());
    context.queue().submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (sender, receiver) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    context.device().poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    })?;
    receiver.recv()??;
    let bytes = slice.get_mapped_range();
    let words = bytes
        .chunks_exact(4)
        .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
        .collect();
    drop(bytes);
    readback.unmap();
    Ok(words)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingSeal {
    generation: u64,
    flight: GpuPisFlight,
}

struct RootState {
    generation: u64,
    pending: Option<PendingSeal>,
    committed: Option<Arc<ResidentSuccessor>>,
    future: Option<Arc<InstalledOneXsDraw>>,
    ready: Option<Arc<InstalledOneXsDraw>>,
    quarantined: bool,
    quarantined_successors: Vec<Arc<ResidentSuccessor>>,
    context: Option<OneXsGpuContext>,
    session: Option<ResidentSourceIdentity>,
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
    #[cfg(test)]
    carrier_drop_witness: Option<Arc<std::sync::atomic::AtomicU8>>,
}

impl GpuResidentCapture {
    pub(super) fn new() -> Self {
        Self {
            shared: Arc::new(SharedRoot {
                state: Mutex::new(RootState {
                    generation: 0,
                    pending: None,
                    committed: None,
                    future: None,
                    ready: None,
                    quarantined: false,
                    quarantined_successors: Vec::new(),
                    context: None,
                    session: None,
                }),
            }),
        }
    }

    pub(super) fn new_bound(context: OneXsGpuContext, session: ResidentSourceIdentity) -> Self {
        let mut capture = Self::new();
        let shared = Arc::get_mut(&mut capture.shared).expect("new root is uniquely owned");
        let state = shared
            .state
            .get_mut()
            .expect("new root mutex is not poisoned");
        state.context = Some(context);
        state.session = Some(session);
        capture
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

    pub(super) fn has_installed_successor(&self) -> Fallible<bool> {
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| "ONE X2 resident capture root is poisoned and quarantined")?;
        if state.quarantined {
            return Err("ONE X2 resident capture root is quarantined".into());
        }
        Ok(state.committed.is_some())
    }

    pub(super) fn has_future(&self) -> Fallible<bool> {
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| "ONE X2 resident capture root is poisoned and quarantined")?;
        if state.quarantined {
            return Err("ONE X2 resident capture root is quarantined".into());
        }
        Ok(state.future.is_some())
    }

    pub(super) fn future_stamp(&self) -> Fallible<Option<FrameStamp>> {
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| "ONE X2 resident capture root is poisoned and quarantined")?;
        if state.quarantined {
            return Err("ONE X2 resident capture root is quarantined".into());
        }
        Ok(state.future.as_ref().map(|draw| draw.frame()))
    }

    /// Publish the one completed future only for its exact delivered frame.
    /// A mismatch leaves both the displayed draw and future untouched.
    pub(super) fn publish_future(&self, due: &FrameStamp) -> Fallible<bool> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| "ONE X2 resident capture root is poisoned and quarantined")?;
        if state.quarantined {
            return Err("ONE X2 resident capture root is quarantined".into());
        }
        let matches = state
            .future
            .as_ref()
            .is_some_and(|draw| draw.frame() == *due);
        if !matches {
            return Ok(false);
        }
        state.ready = state.future.take();
        Ok(true)
    }

    /// Snapshot the complete installed draw and reserve a fresh retirement
    /// slot. This does not touch committed history and never exposes a piece of
    /// the payload.
    pub(super) fn ready_for_draw(
        &self,
        retirements: &IcedDrawRetirements<InstalledOneXsPass>,
    ) -> Result<Option<InstalledOneXsReady>, DrawRetirementError> {
        let permit = retirements.reserve()?;
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| DrawRetirementError::Quarantined)?;
        Ok(state.ready.as_ref().map(|draw| InstalledOneXsReady {
            draw: Arc::clone(draw),
            permit,
            pass: None,
        }))
    }

    /// Snapshot the completed unpublished draw and reserve its one retirement
    /// slot before the facade prepares a binding or publishes the future.
    pub(super) fn future_for_draw(
        &self,
        retirements: &IcedDrawRetirements<InstalledOneXsPass>,
    ) -> Result<Option<InstalledOneXsReady>, DrawRetirementError> {
        let permit = retirements.reserve()?;
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| DrawRetirementError::Quarantined)?;
        if state.quarantined {
            return Err(DrawRetirementError::Quarantined);
        }
        Ok(state.future.as_ref().map(|draw| InstalledOneXsReady {
            draw: Arc::clone(draw),
            permit,
            pass: None,
        }))
    }

    pub(super) fn diagnostic_ready(&self) -> Fallible<Option<Arc<InstalledOneXsDraw>>> {
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| "ONE X2 resident capture root is poisoned and quarantined")?;
        if state.quarantined {
            return Err("ONE X2 resident capture root is quarantined".into());
        }
        Ok(state.ready.clone())
    }

    #[cfg(test)]
    pub(super) fn take_ready_for_drop_order_test(
        &self,
        retirements: &IcedDrawRetirements<InstalledOneXsPass>,
    ) -> InstalledOneXsReady {
        let permit = retirements.reserve().unwrap();
        let draw = self
            .shared
            .state
            .lock()
            .unwrap()
            .ready
            .take()
            .expect("drop-order test requires installed ready draw");
        InstalledOneXsReady {
            draw,
            permit,
            pass: None,
        }
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
            ready_allocation: state.ready.as_ref().map(Arc::downgrade),
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

    /// Mint the sole warm prior capability from this reservation's exact root
    /// snapshot. A cold reservation has no installed predecessor and refuses.
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn installed_prior(
        &self,
    ) -> Fallible<InstalledResidentPrior> {
        let successor = self
            .prior
            .as_ref()
            .ok_or("ONE X2 warm transition has no installed predecessor")?;
        if successor.post_l1.is_none() {
            return Err("ONE X2 installed predecessor has no post-L1 state".into());
        }
        let context = self
            .shared
            .state
            .lock()
            .map_err(|_| "ONE X2 resident capture root is poisoned and quarantined")?
            .context
            .clone()
            .ok_or("ONE X2 resident root has no capture context")?;
        Ok(InstalledResidentPrior {
            successor: Arc::clone(successor),
            context,
        })
    }

    pub(super) fn seal(self, successor: ResidentSuccessor) -> Fallible<GpuResidentCandidate> {
        if successor.flight != self.seal.flight {
            return Err("ONE X2 resident successor does not match its root reservation".into());
        }
        Ok(self.seal_validated(successor))
    }

    /// Construct after the owning post state has compared the successor and
    /// reservation without extracting either one. This path cannot fail and
    /// therefore cannot roll the root back ahead of an outer source carrier.
    pub(super) fn seal_validated(self, successor: ResidentSuccessor) -> GpuResidentCandidate {
        GpuResidentCandidate {
            reservation: Some(self),
            successor: Arc::new(successor),
            disarmed: false,
            #[cfg(test)]
            carrier_drop_witness: None,
        }
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
    #[cfg(test)]
    pub(super) fn observe_carrier_drop(&mut self, witness: Arc<std::sync::atomic::AtomicU8>) {
        self.carrier_drop_witness = Some(witness);
    }

    #[cfg(test)]
    pub(super) fn clear_pending_for_stale_test(&self) {
        let reservation = self
            .reservation
            .as_ref()
            .expect("test candidate retains reservation");
        reservation.shared.state.lock().unwrap().pending = None;
    }
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

    /// Atomically publish the exact successor and whole installed draw. Every
    /// identity check occurs under the capture root's one mutex, and failure
    /// drops the offered source/map carrier before reservation rollback.
    pub(super) fn install(&mut self, draw: &Arc<InstalledOneXsDraw>) -> Fallible<()> {
        let reservation = self
            .reservation
            .as_mut()
            .expect("resident candidate retains its reservation");
        let mut state =
            match reservation.shared.state.lock() {
                Ok(state) => state,
                Err(_) => return Err(
                    "ONE X2 resident capture install found a poisoned root; candidate quarantined"
                        .into(),
                ),
            };
        let exact_seal = state.pending.as_ref() == Some(&reservation.seal);
        let exact_prior = same_successor(state.committed.as_ref(), reservation.prior.as_ref());
        if !exact_seal || !exact_prior || state.quarantined {
            state.quarantined = true;
            state
                .quarantined_successors
                .push(Arc::clone(&self.successor));
            return Err("ONE X2 resident capture install does not match its seal and prior allocation; candidate quarantined".into());
        }
        let root_identity = reservation.identity();
        let identity = state
            .context
            .as_ref()
            .zip(state.session.as_ref())
            .ok_or("ONE X2 resident root has no capture context or session");
        let (context, session) = identity?;
        draw.ensure_install_identity(context, session, &root_identity, &reservation.seal.flight)?;
        state.committed = Some(Arc::clone(&self.successor));
        state.ready = Some(Arc::clone(draw));
        state.pending = None;
        reservation.active = false;
        self.disarmed = true;
        Ok(())
    }

    /// Commit one whole completed successor without publishing it for draw.
    ///
    /// The future owns the exact source/map associated with this successor.
    /// The previously published draw remains available until an exact due
    /// delivery moves this future into the ready slot.
    pub(super) fn commit_future(&mut self, draw: &Arc<InstalledOneXsDraw>) -> Fallible<()> {
        let reservation = self
            .reservation
            .as_mut()
            .expect("resident candidate retains its reservation");
        let mut state = match reservation.shared.state.lock() {
            Ok(state) => state,
            Err(_) => {
                return Err(
                    "ONE X2 resident future commit found a poisoned root; candidate quarantined"
                        .into(),
                );
            }
        };
        let exact_seal = state.pending.as_ref() == Some(&reservation.seal);
        let exact_prior = same_successor(state.committed.as_ref(), reservation.prior.as_ref());
        if !exact_seal || !exact_prior || state.future.is_some() || state.quarantined {
            state.quarantined = true;
            state
                .quarantined_successors
                .push(Arc::clone(&self.successor));
            return Err("ONE X2 resident future commit does not match its seal and prior allocation, or the future slot is occupied; candidate quarantined".into());
        }
        let root_identity = reservation.identity();
        let identity = state
            .context
            .as_ref()
            .zip(state.session.as_ref())
            .ok_or("ONE X2 resident root has no capture context or session");
        let (context, session) = identity?;
        draw.ensure_install_identity(context, session, &root_identity, &reservation.seal.flight)?;
        state.committed = Some(Arc::clone(&self.successor));
        state.future = Some(Arc::clone(draw));
        state.pending = None;
        reservation.active = false;
        self.disarmed = true;
        Ok(())
    }

    /// Commit one whole completed successor for later processing without
    /// making its raw source/map carrier a display future or ready draw.
    pub(super) fn commit_processing(&mut self, draw: &Arc<InstalledOneXsDraw>) -> Fallible<()> {
        self.commit_processing_with(|context, session, root, flight| {
            draw.ensure_install_identity(context, session, root, flight)
        })
    }

    fn commit_processing_with(
        &mut self,
        ensure_identity: impl FnOnce(
            &OneXsGpuContext,
            &ResidentSourceIdentity,
            &GpuResidentIdentity,
            &GpuPisFlight,
        ) -> Fallible<()>,
    ) -> Fallible<()> {
        let reservation = self
            .reservation
            .as_mut()
            .expect("resident candidate retains its reservation");
        let mut state = match reservation.shared.state.lock() {
            Ok(state) => state,
            Err(_) => {
                return Err(
                    "ONE X2 resident processing commit found a poisoned root; candidate quarantined"
                        .into(),
                );
            }
        };
        let exact_seal = state.pending.as_ref() == Some(&reservation.seal);
        let exact_prior = same_successor(state.committed.as_ref(), reservation.prior.as_ref());
        if !exact_seal || !exact_prior || state.future.is_some() || state.quarantined {
            state.quarantined = true;
            state
                .quarantined_successors
                .push(Arc::clone(&self.successor));
            return Err("ONE X2 resident processing commit does not match its seal and prior allocation, or the future slot is occupied; candidate quarantined".into());
        }
        let root_identity = reservation.identity();
        let identity = state
            .context
            .as_ref()
            .zip(state.session.as_ref())
            .ok_or("ONE X2 resident root has no capture context or session");
        let (context, session) = identity?;
        ensure_identity(context, session, &root_identity, &reservation.seal.flight)?;
        state.committed = Some(Arc::clone(&self.successor));
        state.pending = None;
        reservation.active = false;
        self.disarmed = true;
        Ok(())
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
        #[cfg(test)]
        if let Some(witness) = &self.carrier_drop_witness {
            assert_eq!(
                witness.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "resident root rollback preceded installed source/map carrier drop"
            );
            witness.store(2, std::sync::atomic::Ordering::SeqCst);
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
    ready_allocation: Option<Weak<InstalledOneXsDraw>>,
    pub(super) quarantined: bool,
}

#[cfg(test)]
impl TestSnapshot {
    pub(super) fn same_ready(&self, other: &Self) -> bool {
        match (&self.ready_allocation, &other.ready_allocation) {
            (None, None) => true,
            (Some(left), Some(right)) => Weak::ptr_eq(left, right),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use kjerag_media::FrameStamp;

    use super::*;

    #[test]
    fn empty_future_cannot_replace_the_ready_slot() {
        let capture = GpuResidentCapture::new();
        let due = frame(1);

        assert!(!capture.has_future().unwrap());
        assert_eq!(capture.future_stamp().unwrap(), None);
        assert!(!capture.publish_future(&due).unwrap());
        assert!(!capture.snapshot().ready);
    }

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
    fn processing_commit_advances_the_exact_prior_without_a_display_slot() {
        let (device, queue) = match gpu() {
            Ok(gpu) => gpu,
            Err(reason) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {reason}"
                );
                eprintln!("skipping resident processing-commit test: {reason}");
                return;
            }
        };
        let context = OneXsGpuContext::new(&device, &queue);
        let session = ResidentSourceIdentity::for_test();
        let capture = GpuResidentCapture::new_bound(context.clone(), session.clone());
        let first_flight = flight(1);
        let mut first = capture
            .reserve(first_flight.frame.clone())
            .unwrap()
            .seal(successor(&device, first_flight.clone()))
            .unwrap();
        let expected_root = first
            .reservation
            .as_ref()
            .expect("candidate retains its reservation")
            .identity();
        first
            .commit_processing_with(|actual_context, actual_session, root, actual_flight| {
                actual_context.ensure_same(&context)?;
                actual_session.ensure_matches(&session)?;
                assert!(root.matches(&expected_root));
                assert_eq!(actual_flight, &first_flight);
                Ok(())
            })
            .unwrap();

        let committed = Arc::clone(&first.successor);
        let state = capture.snapshot();
        assert!(!state.pending);
        assert!(!state.ready);
        assert!(!capture.has_future().unwrap());
        assert!(Arc::ptr_eq(state.committed.as_ref().unwrap(), &committed));

        let next = capture.reserve(frame(2)).unwrap();
        assert!(Arc::ptr_eq(next.prior.as_ref().unwrap(), &committed));
        next.abort().unwrap();
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
