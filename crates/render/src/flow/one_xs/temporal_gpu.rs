//! Unselected GPU-resident warm image history for the selected ONE X2 path.
//!
//! This stage owns only image-temporal state: the exact base motion mask, its
//! recursive L1/L2 reductions and the next physical A/B reference planes.  A
//! pending result is a second state slot.  It becomes the capture's committed
//! slot only when an enclosing whole-frame transaction calls `commit`.

use std::sync::{Arc, Mutex};

#[cfg(test)]
use super::super::GpuBlurredBelts;
use super::GpuGeometryBelts;
use crate::Fallible;
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
use crate::flow::one_xs::pis::Level;
use crate::flow::one_xs::pis::gpu::GpuPisFlight;
use crate::flow::one_xs::post_update::MotionPyramid;
use crate::flow::one_xs::temporal::{BlurredBelts, MotionMask, next_warm_references};
use crate::flow::one_xs::{COLS, LensPair, ROWS};
use crate::flow::one_xs_belt::SolverBelts;

const CODES_PER_WORD: usize = 4;
const BASE_BYTES: usize = ROWS * COLS;
const L1_BYTES: usize = Level::One.pixels();
const L2_BYTES: usize = Level::Two.pixels();
const BELT_BYTES: usize = SolverBelts::BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GpuMotionHistory {
    Cold,
    Warm,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GpuMotionReceipt {
    pub(crate) flight: GpuPisFlight,
    pub(crate) temporal_generation: u64,
    pub(crate) history: GpuMotionHistory,
}

struct CaptureState {
    generation: u64,
    committed_references: Option<Arc<wgpu::Buffer>>,
    committed_flight: Option<GpuPisFlight>,
    pending: Option<(u64, GpuPisFlight)>,
}

/// Capture-owned committed reference slot.  It is intentionally non-cloneable;
/// pending tokens share only its private lock so drop can roll a flight back.
pub(crate) struct GpuMotionState {
    inner: Arc<Mutex<CaptureState>>,
}

/// One private, qualified GPU context matched to a solver-belt producer.
pub(crate) struct GpuMotionStage {
    context: OneXsGpuContext,
    threshold: wgpu::ComputePipeline,
    promote: wgpu::ComputePipeline,
    reduce_l1: wgpu::ComputePipeline,
    reduce_l2: wgpu::ComputePipeline,
    cold_refs: wgpu::ComputePipeline,
    next_refs: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

struct GpuMotionCandidate {
    state: Arc<Mutex<CaptureState>>,
    receipt: GpuMotionReceipt,
    current: wgpu::Buffer,
    raw_base: wgpu::Buffer,
    base: wgpu::Buffer,
    level_one: wgpu::Buffer,
    level_two: wgpu::Buffer,
    next_references: Arc<wgpu::Buffer>,
    _resources: wgpu::BindGroup,
}

/// Purpose-specific temporal allocation, reservation and command. Only the
/// resident belt owner can submit it and join it to the inherited lease.
struct EncodedGpuMotion {
    command: Option<wgpu::CommandBuffer>,
    candidate: Option<GpuMotionCandidate>,
}

/// Candidate second slot. Drop is rollback; transfer only seals the successor.
#[must_use = "the GPU motion transaction must enter a frame candidate or roll back"]
pub(crate) struct GpuMotionTransaction<C> {
    candidate: Option<GpuMotionCandidate>,
    carrier: Option<C>,
    transferred: bool,
}

/// Opaque pending successor. Publication belongs only to the capture owner's
/// final atomic ready-draw install, after every downstream stage succeeds.
#[must_use = "the pending GPU motion frame must reach final install or roll back"]
pub(crate) struct GpuMotionFrame<C> {
    candidate: Option<GpuMotionCandidate>,
    carrier: C,
    installed: bool,
}

impl GpuMotionStage {
    pub(crate) fn new(context: OneXsGpuContext) -> Fallible<Self> {
        Self::from_shader(context, SHADER, true)
    }

    fn from_shader(context: OneXsGpuContext, shader: &str, qualify: bool) -> Fallible<Self> {
        let device = context.device();
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ONE X2 GPU temporal image state"),
            entries: &[
                storage(0, true),
                storage(1, true),
                storage(2, false),
                storage(3, false),
                storage(4, false),
                storage(5, false),
                storage(6, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 GPU temporal image state"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 GPU temporal image state"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ONE X2 GPU temporal image state"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let stage = Self {
            context: context.clone(),
            threshold: pipeline("threshold_base"),
            promote: pipeline("promote_base"),
            reduce_l1: pipeline("reduce_l1"),
            reduce_l2: pipeline("reduce_l2"),
            cold_refs: pipeline("cold_references"),
            next_refs: pipeline("next_references"),
            layout,
        };
        if qualify {
            stage.qualify()?;
        }
        Ok(stage)
    }

    pub(crate) fn empty_state(&self) -> GpuMotionState {
        GpuMotionState {
            inner: Arc::new(Mutex::new(CaptureState {
                generation: 0,
                committed_references: None,
                committed_flight: None,
                pending: None,
            })),
        }
    }

    fn context_for_resident_transition(&self) -> &OneXsGpuContext {
        &self.context
    }

    /// Reserve one temporal generation and encode its exact arithmetic. The
    /// resident belt owner remains responsible for the sole lease advancement.
    fn encode_resident_transition(
        &self,
        state: &GpuMotionState,
        current: &wgpu::Buffer,
        flight: GpuPisFlight,
    ) -> Fallible<EncodedGpuMotion> {
        let (generation, reference, history) = {
            let mut guard = state.inner.lock().expect("GPU motion state mutex poisoned");
            if guard.pending.is_some() {
                return Err("ONE X2 GPU motion state already has an in-flight frame".into());
            }
            let history = if guard.committed_references.is_some() {
                GpuMotionHistory::Warm
            } else {
                GpuMotionHistory::Cold
            };
            guard.pending = Some((guard.generation, flight.clone()));
            (
                guard.generation,
                guard.committed_references.clone(),
                history,
            )
        };
        let reference = reference.unwrap_or_else(|| Arc::new(current.clone()));
        let device = self.context.device();
        let raw_base = buffer(device, "ONE X2 raw motion", BASE_BYTES);
        let base = buffer(device, "ONE X2 promoted motion", BASE_BYTES);
        let level_one = buffer(device, "ONE X2 L1 motion", L1_BYTES);
        let level_two = buffer(device, "ONE X2 L2 motion", L2_BYTES);
        let next_references = Arc::new(buffer(device, "ONE X2 next references", BELT_BYTES));
        let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 GPU temporal image state"),
            layout: &self.layout,
            entries: &[
                entry(0, current),
                entry(1, &reference),
                entry(2, &raw_base),
                entry(3, &base),
                entry(4, &level_one),
                entry(5, &level_two),
                entry(6, &next_references),
            ],
        });
        let command = self.encode(device, &resources, history);
        Ok(EncodedGpuMotion {
            command: Some(command),
            candidate: Some(GpuMotionCandidate {
                state: Arc::clone(&state.inner),
                receipt: GpuMotionReceipt {
                    flight,
                    temporal_generation: generation,
                    history,
                },
                current: current.clone(),
                raw_base,
                base,
                level_one,
                level_two,
                next_references,
                _resources: resources,
            }),
        })
    }

    fn encode(
        &self,
        device: &wgpu::Device,
        resources: &wgpu::BindGroup,
        history: GpuMotionHistory,
    ) -> wgpu::CommandBuffer {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 GPU temporal image state"),
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ONE X2 GPU temporal image state"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, resources, &[]);
        if history == GpuMotionHistory::Warm {
            for (pipeline, bytes) in [
                (&self.threshold, BASE_BYTES),
                (&self.promote, BASE_BYTES),
                (&self.reduce_l1, L1_BYTES),
                (&self.reduce_l2, L2_BYTES),
            ] {
                pass.set_pipeline(pipeline);
                pass.dispatch_workgroups(words(bytes).div_ceil(64) as u32, 1, 1);
            }
        }
        pass.set_pipeline(match history {
            GpuMotionHistory::Cold => &self.cold_refs,
            GpuMotionHistory::Warm => &self.next_refs,
        });
        pass.dispatch_workgroups(words(BELT_BYTES).div_ceil(64) as u32, 1, 1);
        drop(pass);
        encoder.finish()
    }

    fn qualify(&self) -> Fallible<()> {
        let device = self.context.device();
        let queue = self.context.queue();
        let (reference, current) = qualification_pair();
        let expected_mask = MotionMask::between(&current, &reference);
        let expected_pyramid = MotionPyramid::from_base(&expected_mask);
        let expected_refs = next_warm_references(&reference, &current);
        let current_buffer = upload(device, queue, "motion current", current.bytes());
        let reference_buffer = upload(device, queue, "motion reference", reference.bytes());
        let raw = buffer(device, "motion raw qualification", BASE_BYTES);
        let base = buffer(device, "motion base qualification", BASE_BYTES);
        let l1 = buffer(device, "motion L1 qualification", L1_BYTES);
        let l2 = buffer(device, "motion L2 qualification", L2_BYTES);
        let next = buffer(device, "motion reference qualification", BELT_BYTES);
        let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 motion qualification"),
            layout: &self.layout,
            entries: &[
                entry(0, &current_buffer),
                entry(1, &reference_buffer),
                entry(2, &raw),
                entry(3, &base),
                entry(4, &l1),
                entry(5, &l2),
                entry(6, &next),
            ],
        });
        queue.submit([self.encode(device, &resources, GpuMotionHistory::Warm)]);
        for (name, gpu, expected) in [
            ("base", &base, expected_mask.bytes()),
            ("level one", &l1, expected_pyramid.bytes(Level::One)),
            ("level two", &l2, expected_pyramid.bytes(Level::Two)),
            ("next references", &next, expected_refs.bytes()),
        ] {
            let actual = read(device, queue, gpu, expected.len())?;
            if let Some(index) = actual.iter().zip(expected).position(|(a, b)| a != b) {
                return Err(format!("ONE X2 GPU temporal arithmetic is not exact on this graphics device: {name} byte {index} is {}, expected {}", actual[index], expected[index]).into());
            }
        }
        Ok(())
    }
}

impl EncodedGpuMotion {
    fn take_command(&mut self) -> wgpu::CommandBuffer {
        self.command
            .take()
            .expect("encoded GPU motion transition submits exactly once")
    }
}

impl Drop for EncodedGpuMotion {
    fn drop(&mut self) {
        if let Some(candidate) = &self.candidate {
            clear_pending(
                &candidate.state,
                candidate.receipt.temporal_generation,
                &candidate.receipt.flight,
            );
        }
    }
}

impl<C> GpuMotionTransaction<C> {
    fn from_resident_transition(mut encoded: EncodedGpuMotion, carrier: C) -> Self {
        debug_assert!(encoded.command.is_none());
        Self {
            candidate: encoded.candidate.take(),
            carrier: Some(carrier),
            transferred: false,
        }
    }

    fn candidate(&self) -> &GpuMotionCandidate {
        self.candidate
            .as_ref()
            .expect("GPU motion transaction lost its candidate")
    }

    #[cfg(test)]
    fn receipt(&self) -> &GpuMotionReceipt {
        &self.candidate().receipt
    }
    #[cfg(test)]
    fn base(&self) -> Option<&wgpu::Buffer> {
        (self.receipt().history == GpuMotionHistory::Warm).then_some(&self.candidate().base)
    }
    #[cfg(test)]
    fn level(&self, level: Level) -> Option<&wgpu::Buffer> {
        (self.receipt().history == GpuMotionHistory::Warm).then_some(match level {
            Level::One => &self.candidate().level_one,
            Level::Two => &self.candidate().level_two,
        })
    }

    /// Seal the complete pending successor into the opaque frame candidate.
    /// This deliberately does not publish temporal state: later resident L2,
    /// post, final-map, bind and draw installation may still fail.
    pub(crate) fn into_frame_candidate(mut self) -> GpuMotionFrame<C> {
        let candidate = self
            .candidate
            .take()
            .expect("GPU motion transaction lost its candidate");
        self.transferred = true;
        GpuMotionFrame {
            candidate: Some(candidate),
            carrier: self
                .carrier
                .take()
                .expect("motion transaction lost its submission lease"),
            installed: false,
        }
    }
}

impl<K> GpuGeometryBelts<K> {
    /// Consume the entire resident producer token into the temporal stage.
    /// No buffer, flight, context, command or lease component crosses this
    /// private ownership boundary separately.
    pub(crate) fn prepare_motion(
        mut self,
        stage: &GpuMotionStage,
        state: &GpuMotionState,
    ) -> Fallible<GpuMotionTransaction<Self>> {
        let context = stage.context_for_resident_transition();
        self.belts.lease.validate_provenance(context)?;
        let flight = self
            .belts
            .flight
            .as_ref()
            .expect("GPU-resident belts retain their flight until consumption")
            .clone();
        let mut encoded = stage.encode_resident_transition(state, &self.belts.packed, flight)?;
        self.belts
            .lease
            .submit_after(context, |_| encoded.take_command())?;
        self.belts
            .flight
            .take()
            .expect("GPU-resident belts transfer their flight exactly once");
        Ok(GpuMotionTransaction::from_resident_transition(
            encoded, self,
        ))
    }
}

#[cfg(test)]
impl<K> GpuBlurredBelts<K> {
    fn prepare_motion(
        mut self,
        stage: &GpuMotionStage,
        state: &GpuMotionState,
    ) -> Fallible<GpuMotionTransaction<Self>> {
        let context = stage.context_for_resident_transition();
        self.lease.validate_provenance(context)?;
        let flight = self
            .flight
            .as_ref()
            .expect("GPU-resident belts retain their flight until consumption")
            .clone();
        let mut encoded = stage.encode_resident_transition(state, &self.packed, flight)?;
        self.lease
            .submit_after(context, |_| encoded.take_command())?;
        self.flight
            .take()
            .expect("GPU-resident belts transfer their flight exactly once");
        Ok(GpuMotionTransaction::from_resident_transition(
            encoded, self,
        ))
    }
}

impl<C> Drop for GpuMotionTransaction<C> {
    fn drop(&mut self) {
        if self.transferred {
            return;
        }
        if let Some(candidate) = &self.candidate {
            clear_pending(
                &candidate.state,
                candidate.receipt.temporal_generation,
                &candidate.receipt.flight,
            );
        }
    }
}

impl<C> GpuMotionFrame<C> {
    fn candidate(&self) -> &GpuMotionCandidate {
        self.candidate
            .as_ref()
            .expect("GPU motion frame lost its pending successor")
    }

    pub(crate) fn receipt(&self) -> &GpuMotionReceipt {
        &self.candidate().receipt
    }

    #[cfg(test)]
    fn current(&self) -> &wgpu::Buffer {
        &self.candidate().current
    }

    #[cfg(test)]
    fn base(&self) -> Option<&wgpu::Buffer> {
        (self.receipt().history == GpuMotionHistory::Warm).then_some(&self.candidate().base)
    }

    #[cfg(test)]
    fn level(&self, level: Level) -> Option<&wgpu::Buffer> {
        (self.receipt().history == GpuMotionHistory::Warm).then_some(match level {
            Level::One => &self.candidate().level_one,
            Level::Two => &self.candidate().level_two,
        })
    }
}

impl<C> Drop for GpuMotionFrame<C> {
    fn drop(&mut self) {
        if self.installed {
            return;
        }
        if let Some(candidate) = &self.candidate {
            clear_pending(
                &candidate.state,
                candidate.receipt.temporal_generation,
                &candidate.receipt.flight,
            );
        }
    }
}

#[cfg(test)]
impl<K> GpuMotionFrame<GpuBlurredBelts<K>> {
    /// Test-only stand-in for the future capture-owner atomic ready-draw
    /// install. Production deliberately has no publication method yet.
    fn publish_at_install_for_test(mut self) -> Fallible<Self> {
        let candidate = self.candidate();
        {
            let mut state = candidate
                .state
                .lock()
                .expect("GPU motion state mutex poisoned");
            if state.pending.as_ref()
                != Some(&(
                    candidate.receipt.temporal_generation,
                    candidate.receipt.flight.clone(),
                ))
            {
                return Err(
                    "ONE X2 GPU motion install does not match the sealed in-flight frame".into(),
                );
            }
            let next_generation = state
                .generation
                .checked_add(1)
                .ok_or("ONE X2 GPU motion generation space exhausted")?;
            state.committed_references = Some(Arc::clone(&candidate.next_references));
            state.committed_flight = Some(candidate.receipt.flight.clone());
            state.generation = next_generation;
            state.pending = None;
        }
        self.installed = true;
        Ok(self)
    }

    fn observe_completion(&mut self, state: std::sync::Arc<std::sync::atomic::AtomicU8>) {
        self.carrier.observe_completion(state);
    }
}

fn clear_pending(state: &Arc<Mutex<CaptureState>>, generation: u64, flight: &GpuPisFlight) {
    let mut state = state.lock().expect("GPU motion state mutex poisoned");
    if state.pending.as_ref() == Some(&(generation, flight.clone())) {
        state.pending = None;
    }
}

fn words(bytes: usize) -> usize {
    bytes.div_ceil(CODES_PER_WORD)
}
fn buffer(device: &wgpu::Device, label: &'static str, bytes: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (words(bytes) * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    bytes: &[u8],
) -> wgpu::Buffer {
    let buffer = buffer(device, label, bytes.len());
    let mut padded = vec![0; words(bytes.len()) * 4];
    padded[..bytes.len()].copy_from_slice(bytes);
    queue.write_buffer(&buffer, 0, &padded);
    buffer
}
fn fixture(seed: u32) -> BlurredBelts {
    BlurredBelts::from_lenses(LensPair {
        a: (0..BASE_BYTES)
            .map(|i| ((i as u32 * 37 + seed + (i / COLS) as u32 * 11) & 255) as u8)
            .collect(),
        b: (0..BASE_BYTES)
            .map(|i| ((i as u32 * 19 + seed * 3 + (i % COLS) as u32 * 7) & 255) as u8)
            .collect(),
    })
    .unwrap()
}

fn qualification_pair() -> (BlurredBelts, BlurredBelts) {
    let reference_lenses = LensPair {
        a: (0..BASE_BYTES)
            .map(|index| 32 + ((index * 37 + index / COLS * 11) % 160) as u8)
            .collect::<Vec<_>>(),
        b: (0..BASE_BYTES)
            .map(|index| 40 + ((index * 19 + index % COLS * 7) % 150) as u8)
            .collect::<Vec<_>>(),
    };
    let reference = BlurredBelts::from_lenses(reference_lenses.clone())
        .expect("qualification reference has retained shape");
    let mut current = LensPair {
        a: reference_lenses
            .a
            .iter()
            .map(|value| value + 3)
            .collect::<Vec<_>>(),
        b: reference_lenses
            .b
            .iter()
            .map(|value| value + 7)
            .collect::<Vec<_>>(),
    };
    // Isolated exact-threshold and B-only changes remain raw 1s because their
    // local populations are below the promotion threshold.
    current.a[12 * COLS + 13] = reference_lenses.a[12 * COLS + 13] + 10;
    current.b[37 * COLS + 29] = reference_lenses.b[37 * COLS + 29] + 11;
    // Exercise native's asymmetric low-edge bounds with a population that
    // straddles the first omitted row and column.
    for ordinal in 0..11 {
        let row = 1 + ordinal / 6;
        let col = 1 + ordinal % 6;
        let index = row * COLS + col;
        current.a[index] = reference_lenses.a[index] + 12;
    }
    for omitted_edge in [1, 2] {
        current.a[omitted_edge] = reference_lenses.a[omitted_edge] + 12;
    }
    // An interior zero byte sees exactly floor(0.2 * 20 * 12) changes.
    for row in 91..95 {
        for col in 15..27 {
            let index = row * COLS + col;
            current.a[index] = reference_lenses.a[index] + 12;
        }
    }
    (
        reference,
        BlurredBelts::from_lenses(current).expect("qualification pair has retained shape"),
    )
}
fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Buffer,
    bytes: usize,
) -> Fallible<Vec<u8>> {
    use std::sync::mpsc;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ONE X2 motion diagnostic readback"),
        size: (words(bytes) * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &staging, 0, (words(bytes) * 4) as u64);
    let copy = queue.submit([encoder.finish()]);
    let slice = staging.slice(..);
    let (sent, answer) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sent.send(result);
    });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(copy),
        timeout: None,
    })?;
    answer.recv()??;
    let mapped = slice.get_mapped_range();
    let result = mapped[..bytes].to_vec();
    drop(mapped);
    staging.unmap();
    Ok(result)
}

const SHADER: &str = r#"
const ROWS = 1080u; const COLS = 60u; const BASE = ROWS * COLS;
const L1_ROWS = 540u; const L1_COLS = 30u; const L1 = L1_ROWS * L1_COLS;
const L2_ROWS = 270u; const L2_COLS = 15u; const L2 = L2_ROWS * L2_COLS;
@group(0) @binding(0) var<storage, read> current: array<u32>;
@group(0) @binding(1) var<storage, read> reference: array<u32>;
@group(0) @binding(2) var<storage, read_write> raw_base: array<u32>;
@group(0) @binding(3) var<storage, read_write> motion_base: array<u32>;
@group(0) @binding(4) var<storage, read_write> motion_l1: array<u32>;
@group(0) @binding(5) var<storage, read_write> motion_l2: array<u32>;
@group(0) @binding(6) var<storage, read_write> next_ref: array<u32>;
fn current_byte(index: u32) -> u32 { return (current[index/4u] >> (8u*(index%4u))) & 255u; }
fn reference_byte(index: u32) -> u32 { return (reference[index/4u] >> (8u*(index%4u))) & 255u; }
fn raw(index: u32) -> u32 { return (raw_base[index/4u] >> (8u*(index%4u))) & 255u; }
fn promoted(index: u32) -> u32 { return (motion_base[index/4u] >> (8u*(index%4u))) & 255u; }
fn l1_at(index: u32) -> u32 { return (motion_l1[index/4u] >> (8u*(index%4u))) & 255u; }
fn pack4(a:u32,b:u32,c:u32,d:u32)->u32{return a|(b<<8u)|(c<<16u)|(d<<24u);}
fn threshold(index:u32)->u32{let a=abs(i32(current_byte(index))-i32(reference_byte(index)));
 let b=abs(i32(current_byte(BASE+index))-i32(reference_byte(BASE+index))); return u32(max(a,b)>=10);}
@compute @workgroup_size(64) fn threshold_base(@builtin(global_invocation_id) id:vec3<u32>){let first=id.x*4u;if(first>=BASE){return;}
 raw_base[id.x]=pack4(threshold(first),threshold(first+1u),threshold(first+2u),threshold(first+3u));}
fn promoted_code(index:u32)->u32{let row=index/COLS;let col=index%COLS;let y0=max(row,10u)-9u;let y1=min(row+10u,ROWS-1u)+1u;
 let x0=max(col,6u)-5u;let x1=min(col+6u,COLS-1u)+1u;var count=0u;for(var y=y0;y<y1;y++){for(var x=x0;x<x1;x++){count+=raw(y*COLS+x);}}
 let limit=u32(0.2*f32((y1-y0)*(x1-x0)));return select(raw(index),255u,count>=limit);}
@compute @workgroup_size(64) fn promote_base(@builtin(global_invocation_id) id:vec3<u32>){let first=id.x*4u;if(first>=BASE){return;}
 motion_base[id.x]=pack4(promoted_code(first),promoted_code(first+1u),promoted_code(first+2u),promoted_code(first+3u));}
fn average4(a:u32,b:u32,c:u32,d:u32)->u32{return(a+b+c+d+2u)/4u;}
fn half_base(index:u32)->u32{let r=(index/L1_COLS)*2u;let c=(index%L1_COLS)*2u;return average4(promoted(r*COLS+c),promoted(r*COLS+c+1u),promoted((r+1u)*COLS+c),promoted((r+1u)*COLS+c+1u));}
@compute @workgroup_size(64) fn reduce_l1(@builtin(global_invocation_id) id:vec3<u32>){let first=id.x*4u;if(first>=L1){return;}
 motion_l1[id.x]=pack4(half_base(first),half_base(first+1u),half_base(first+2u),half_base(first+3u));}
fn half_l1(index:u32)->u32{let r=(index/L2_COLS)*2u;let c=(index%L2_COLS)*2u;return average4(l1_at(r*L1_COLS+c),l1_at(r*L1_COLS+c+1u),l1_at((r+1u)*L1_COLS+c),l1_at((r+1u)*L1_COLS+c+1u));}
@compute @workgroup_size(64) fn reduce_l2(@builtin(global_invocation_id) id:vec3<u32>){let first=id.x*4u;if(first>=L2){return;}
 motion_l2[id.x]=pack4(half_l1(first),select(0u,half_l1(first+1u),first+1u<L2),select(0u,half_l1(first+2u),first+2u<L2),select(0u,half_l1(first+3u),first+3u<L2));}
fn ema(index:u32)->u32{let biased=fma(f32(reference_byte(index)),0.7,0.5);let blended=fma(f32(current_byte(index)),0.3,biased);return u32(clamp(trunc(blended),0.0,255.0));}
@compute @workgroup_size(64) fn cold_references(@builtin(global_invocation_id) id:vec3<u32>){if(id.x*4u>=2u*BASE){return;} next_ref[id.x]=current[id.x];}
@compute @workgroup_size(64) fn next_references(@builtin(global_invocation_id) id:vec3<u32>){let first=id.x*4u;if(first>=2u*BASE){return;}
 next_ref[id.x]=pack4(ema(first),ema(first+1u),ema(first+2u),ema(first+3u));}
"#;

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::time::Duration;

    use kjerag_media::FrameStamp;

    use super::super::super::resident_blurred_fixture;
    use super::*;

    #[test]
    fn production_motion_kernel_is_bit_exact_on_the_actual_adapter() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping ONE X2 GPU temporal qualification: {why}");
                return;
            }
        };
        GpuMotionStage::new(OneXsGpuContext::new(&device, &queue)).unwrap_or_else(|error| {
            panic!("ONE X2 GPU temporal image state failed on {adapter}: {error}")
        });
    }

    #[test]
    fn qualification_refuses_live_motion_and_reference_mutations() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping ONE X2 GPU temporal mutations: {why}");
                return;
            }
        };
        let mutations = [
            (
                "both physical lenses",
                "return u32(max(a,b)>=10);",
                "return u32(a>=10);",
            ),
            (
                "inclusive threshold",
                "return u32(max(a,b)>=10);",
                "return u32(max(a,b)>10);",
            ),
            (
                "native low boundary",
                "let y0=max(row,10u)-9u;",
                "let y0=max(row,9u)-9u;",
            ),
            (
                "promotion population",
                "return select(raw(index),255u,count>=limit);",
                "return select(raw(index),255u,count>limit);",
            ),
            (
                "recursive area rounding",
                "return(a+b+c+d+2u)/4u;",
                "return(a+b+c+d+1u)/4u;",
            ),
            ("selected EMA weight", ",0.7,0.5);", ",0.6,0.5);"),
        ];
        for (name, old, new) in mutations {
            let mutated = SHADER.replacen(old, new, 1);
            assert_ne!(mutated, SHADER, "mutation {name} did not edit the shader");
            let error = match GpuMotionStage::from_shader(
                OneXsGpuContext::new(&device, &queue),
                &mutated,
                true,
            ) {
                Ok(_) => panic!("{name} mutation passed qualification on {adapter}"),
                Err(error) => error,
            };
            assert!(error.to_string().contains("not exact"), "{name}: {error}");
        }
    }

    #[test]
    fn final_install_publishes_and_later_stage_drop_is_all_or_nothing() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping ONE X2 GPU temporal rollback: {why}");
                return;
            }
        };
        let stage = GpuMotionStage::new(OneXsGpuContext::new(&device, &queue))
            .unwrap_or_else(|error| panic!("GPU qualification failed on {adapter}: {error}"));
        let state = stage.empty_state();
        let first_input = fixture(3);
        let first_flight = flight(1, 1);
        let first_owner = Arc::new(());
        let (first_belts, first) = resident_blurred_fixture(
            &device,
            &queue,
            Arc::clone(&first_owner),
            first_flight.clone(),
            &first_input,
        )
        .unwrap();
        let cold = first_belts.prepare_motion(&stage, &state).unwrap();
        assert_eq!(cold.receipt().history, GpuMotionHistory::Cold);
        assert!(cold.base().is_none());
        let cold_frame = cold.into_frame_candidate();
        assert_eq!(cold_frame.receipt().flight, first_flight);
        assert_eq!(Arc::strong_count(&first_owner), 2);
        {
            let state = state.inner.lock().unwrap();
            assert_eq!(state.generation, 0);
            assert!(state.committed_references.is_none());
            assert!(state.pending.is_some());
        }
        let cold_frame = cold_frame.publish_at_install_for_test().unwrap();
        let (generation, reference) = {
            let state = state.inner.lock().unwrap();
            (
                state.generation,
                Arc::clone(state.committed_references.as_ref().unwrap()),
            )
        };
        assert_eq!(
            read(&device, &queue, &reference, BELT_BYTES).unwrap(),
            first.bytes()
        );
        drop(cold_frame);
        assert_eq!(Arc::strong_count(&first_owner), 1);

        let second_input = fixture(211);
        let second_flight = flight(2, 2);
        let dropped_owner = Arc::new(());
        let (second_belts, second) = resident_blurred_fixture(
            &device,
            &queue,
            Arc::clone(&dropped_owner),
            second_flight.clone(),
            &second_input,
        )
        .unwrap();
        let warm = second_belts.prepare_motion(&stage, &state).unwrap();
        let warm_frame = warm.into_frame_candidate();
        assert_eq!(warm_frame.receipt().history, GpuMotionHistory::Warm);
        assert!(warm_frame.base().is_some());
        assert!(warm_frame.level(Level::One).is_some());
        assert!(warm_frame.level(Level::Two).is_some());
        drop(warm_frame);
        assert_eq!(Arc::strong_count(&dropped_owner), 1);
        {
            let state = state.inner.lock().unwrap();
            assert_eq!(state.generation, generation);
            assert!(Arc::ptr_eq(
                state.committed_references.as_ref().unwrap(),
                &reference
            ));
            assert!(state.pending.is_none());
        }

        let recovered_owner = Arc::new(());
        let (recovered_belts, recovered_second) = resident_blurred_fixture(
            &device,
            &queue,
            Arc::clone(&recovered_owner),
            second_flight.clone(),
            &second_input,
        )
        .unwrap();
        assert_eq!(recovered_second, second);
        let recovered = recovered_belts.prepare_motion(&stage, &state).unwrap();
        let frame = recovered.into_frame_candidate();
        assert_eq!(frame.receipt().flight, second_flight);
        assert_eq!(Arc::strong_count(&recovered_owner), 2);
        let expected_mask = MotionMask::between(&second, &first);
        let expected_pyramid = MotionPyramid::from_base(&expected_mask);
        let expected_reference = next_warm_references(&first, &second);
        assert_eq!(
            read(&device, &queue, frame.current(), BELT_BYTES).unwrap(),
            second.bytes()
        );
        assert_eq!(
            read(&device, &queue, frame.base().unwrap(), BASE_BYTES).unwrap(),
            expected_mask.bytes()
        );
        for (level, expected) in [
            (Level::One, expected_pyramid.bytes(Level::One)),
            (Level::Two, expected_pyramid.bytes(Level::Two)),
        ] {
            assert_eq!(
                read(&device, &queue, frame.level(level).unwrap(), expected.len()).unwrap(),
                expected
            );
        }
        {
            let state = state.inner.lock().unwrap();
            assert_eq!(state.generation, generation);
            assert!(Arc::ptr_eq(
                state.committed_references.as_ref().unwrap(),
                &reference
            ));
            assert!(state.pending.is_some());
        }
        let frame = frame.publish_at_install_for_test().unwrap();
        {
            let state = state.inner.lock().unwrap();
            assert_eq!(state.generation, generation + 1);
            assert!(!Arc::ptr_eq(
                state.committed_references.as_ref().unwrap(),
                &reference
            ));
            assert_eq!(
                read(
                    &device,
                    &queue,
                    state.committed_references.as_ref().unwrap(),
                    BELT_BYTES
                )
                .unwrap(),
                expected_reference.bytes()
            );
        }
        drop(frame);
        assert_eq!(Arc::strong_count(&recovered_owner), 1);
    }

    #[test]
    fn foreign_context_is_refused_before_temporal_reservation() {
        let (device, queue, foreign_device, foreign_queue, adapter) = match gpu_pair() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping ONE X2 GPU temporal provenance: {why}");
                return;
            }
        };
        let stage = GpuMotionStage::new(OneXsGpuContext::new(&foreign_device, &foreign_queue))
            .unwrap_or_else(|error| {
                panic!("foreign GPU motion stage failed on {adapter}: {error}")
            });
        let state = stage.empty_state();
        let owner = Arc::new(());
        let input = fixture(77);
        let (belts, _) =
            resident_blurred_fixture(&device, &queue, Arc::clone(&owner), flight(9, 9), &input)
                .unwrap();
        let error = match belts.prepare_motion(&stage, &state) {
            Ok(_) => panic!("foreign GPU motion context was accepted"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("crossed a different device or queue"),
            "{error}"
        );
        assert_eq!(Arc::strong_count(&owner), 1);
        let state = state.inner.lock().unwrap();
        assert_eq!(state.generation, 0);
        assert!(state.pending.is_none());
        assert!(state.committed_references.is_none());
    }

    fn flight(index: u64, seconds: u64) -> GpuPisFlight {
        GpuPisFlight {
            generation: index,
            frame: FrameStamp::for_test(index, Duration::from_secs(seconds), None),
        }
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
        let info = adapter.get_info();
        let name = format!("{} ({})", info.name, info.driver);
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("exact ONE X2 GPU temporal image state"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        Ok((device, queue, name))
    }

    fn gpu_pair() -> Result<(wgpu::Device, wgpu::Queue, wgpu::Device, wgpu::Queue, String), String>
    {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
            .ok_or("no Vulkan adapter")?;
        let info = adapter.get_info();
        let name = format!("{} ({})", info.name, info.driver);
        let request = || {
            block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("exact ONE X2 GPU temporal provenance"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            }))
            .map_err(|error| error.to_string())
        };
        let (device, queue) = request()?;
        let (foreign_device, foreign_queue) = request()?;
        Ok((device, queue, foreign_device, foreign_queue, name))
    }
}
