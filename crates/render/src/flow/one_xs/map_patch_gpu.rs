//! Resident GPU final-map materializer for the selected ONE X2 route.
//!
//! The production entry consumes a sealed upstream resident token, advances
//! that token's existing submission lease and leaves the packed 200-by-100
//! type-2 map resident. A four-byte asynchronous validity gate is the only
//! ordinary CPU observation. Scene reaches it through the capture facade.

use std::num::NonZeroU64;
use std::sync::{Arc, mpsc};
use std::{error::Error, fmt};

use kjerag_media::FrameStamp;

use super::super::gpu_context::OneXsGpuContext;
use super::super::l2_seed::SeedError;
#[cfg(test)]
use super::super::map_patch::{
    self, BaseMap, BilateralInputs, CoordinateMap, FlowMap, GateMap, PreimageMap, SideInputs,
};
use super::super::resources::OneXsResources;
use crate::Fallible;
#[cfg(test)]
use crate::studio_type2::AlphaMap;
use crate::studio_type2::{ALPHA_BYTES, MAP_NODES, PACKED_BYTES, PackedMap};

const SHADER: &str = include_str!("map_patch_gpu.wgsl");
const WORKGROUP_SIZE: u32 = 64;
const RETAINED_NODES: usize = super::super::ROWS * super::super::COLS;
const PREIMAGE_WORDS: usize = MAP_NODES * 2;
const BASE_WORDS: usize = RETAINED_NODES * 2;
const FLOW_WORDS: usize = RETAINED_NODES * 2;
const GATE_WORDS: usize = MAP_NODES;
const COORDINATE_WORDS: usize = MAP_NODES * 2;
const DYNAMIC_SIDE_WORDS: usize = PREIMAGE_WORDS + BASE_WORDS + FLOW_WORDS;
const STATIC_SIDE_WORDS: usize = GATE_WORDS + COORDINATE_WORDS;
const SIDE_WORDS: usize = DYNAMIC_SIDE_WORDS + STATIC_SIDE_WORDS;
const INPUT_WORDS: usize = 2 * SIDE_WORDS;
const INPUT_BYTES: u64 = (INPUT_WORDS * size_of::<u32>()) as u64;
#[cfg(test)]
const DYNAMIC_INPUT_WORDS: usize = 2 * DYNAMIC_SIDE_WORDS;
const STATIC_INPUT_WORDS: usize = 2 * STATIC_SIDE_WORDS;
#[allow(dead_code, reason = "legacy final-map binding oracle ABI")]
const STATIC_INPUT_BYTES: u64 = (STATIC_INPUT_WORDS * size_of::<u32>()) as u64;
const ACTION_BYTES: u64 = (MAP_NODES * 2 * size_of::<u32>()) as u64;
const VALIDITY_BYTES: u64 = size_of::<u32>() as u64;
const L1_PATCH_COLS: usize = 8;
const L1_PATCHES: usize = 1424;
const PIS_VALIDITY_CODES: u32 = (4 * L1_PATCHES) as u32;
const GENERATED_HINT_FAILURE_TAG: u32 = 1 << 31;
const GENERATED_HINT_LEVEL_BIT: u32 = 1 << 16;
const GENERATED_HINT_DIRECTION_BIT: u32 = 1 << 15;
const GENERATED_HINT_COMPONENT_BIT: u32 = 1 << 14;
const GENERATED_HINT_SITE_MASK: u32 = GENERATED_HINT_COMPONENT_BIT - 1;
const GENERATED_HINT_RESERVED_MASK: u32 = 0x7ffe_0000;
const L1_DENSE_ROWS: usize = super::super::ROWS / 2;
const L1_DENSE_COLS: usize = super::super::COLS / 2;
const L2_DENSE_ROWS: usize = L1_DENSE_ROWS / 2;
const L2_DENSE_COLS: usize = L1_DENSE_COLS / 2;

/// Semantic level used by the post-L1 validity encoder. The raw bit layout
/// remains owned and decoded here so sibling producers cannot invent another
/// namespace for the same resident status word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GeneratedHintLevel {
    One,
    Two,
}

/// Semantic component used by the post-L1 validity encoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GeneratedHintComponent {
    Dcol,
    Drow,
}

/// Encode one exact generated-successor-hint failure for the shared resident
/// validity word. Invalid dense sites have no representation.
#[allow(dead_code, reason = "final-map qualification oracle")]
pub(super) const fn generated_hint_validity_word(
    level: GeneratedHintLevel,
    direction: super::super::Direction,
    component: GeneratedHintComponent,
    dense_site: u32,
) -> Option<u32> {
    let sites = match level {
        GeneratedHintLevel::One => (L1_DENSE_ROWS * L1_DENSE_COLS) as u32,
        GeneratedHintLevel::Two => (L2_DENSE_ROWS * L2_DENSE_COLS) as u32,
    };
    if dense_site >= sites {
        return None;
    }
    let level = match level {
        GeneratedHintLevel::One => 0,
        GeneratedHintLevel::Two => GENERATED_HINT_LEVEL_BIT,
    };
    let direction = match direction {
        super::super::Direction::AtoB => 0,
        super::super::Direction::BtoA => GENERATED_HINT_DIRECTION_BIT,
    };
    let component = match component {
        GeneratedHintComponent::Dcol => 0,
        GeneratedHintComponent::Drow => GENERATED_HINT_COMPONENT_BIT,
    };
    Some(GENERATED_HINT_FAILURE_TAG | level | direction | component | dense_site)
}

/// Sealed handoff implemented by the upstream resident ONE X2 chain.
///
/// The methods are visible only within `one_xs`: no crate caller can supply a
/// free buffer, frame or queue. Static gate, coordinate and alpha resources
/// are deliberately absent. `submit_after` must advance the token's one
/// existing source-owner lease to the supplied command buffer.
pub(super) mod resident {
    use super::*;

    pub(in crate::flow::one_xs::one_xs_belt_gpu) trait Sealed {}

    #[derive(Clone, Copy)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) enum DynamicSide {
        LensA,
        LensB,
    }

    /// One source range for a purpose-specific dynamic input copy. Its fields
    /// are private so it cannot reveal a retained buffer after the call.
    #[derive(Clone, Copy)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) struct DynamicBufferCopy<'a> {
        buffer: &'a wgpu::Buffer,
        offset: u64,
    }

    impl<'a> DynamicBufferCopy<'a> {
        pub(in crate::flow::one_xs::one_xs_belt_gpu) fn new(
            buffer: &'a wgpu::Buffer,
            offset: u64,
        ) -> Self {
            Self { buffer, offset }
        }
    }

    /// Write-only view of the frame-varying slots in the combined shader
    /// input. The upstream owner can name its exact sources, but never the
    /// target buffer or the capture-static destination ranges.
    #[derive(Clone, Copy)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) struct DynamicInputTarget<'a> {
        target: &'a wgpu::Buffer,
    }

    impl<'a> DynamicInputTarget<'a> {
        pub(super) fn new(target: &'a wgpu::Buffer) -> Self {
            Self { target }
        }

        pub(in crate::flow::one_xs::one_xs_belt_gpu) fn copy_side(
            self,
            encoder: &mut wgpu::CommandEncoder,
            side: DynamicSide,
            preimage: DynamicBufferCopy<'_>,
            base: DynamicBufferCopy<'_>,
            public_flow: DynamicBufferCopy<'_>,
        ) {
            let side = match side {
                DynamicSide::LensA => 0,
                DynamicSide::LensB => 1,
            };
            let target = side * SIDE_WORDS;
            for (source, target, words) in [
                (preimage, target, PREIMAGE_WORDS),
                (base, target + PREIMAGE_WORDS, BASE_WORDS),
                (
                    public_flow,
                    target + PREIMAGE_WORDS + BASE_WORDS,
                    FLOW_WORDS,
                ),
            ] {
                encoder.copy_buffer_to_buffer(
                    source.buffer,
                    source.offset,
                    self.target,
                    byte_offset(target),
                    byte_offset(words),
                );
            }
        }
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) trait Operands:
        Sealed + Sized
    {
        fn context(&self) -> &OneXsGpuContext;
        fn frame(&self) -> &FrameStamp;
        fn encode_dynamic_input_copy(
            &self,
            encoder: &mut wgpu::CommandEncoder,
            target: DynamicInputTarget<'_>,
        );
        fn encode_validity_copy(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Buffer);
        fn submit_after(
            &mut self,
            producer: &OneXsGpuContext,
            command: wgpu::CommandBuffer,
        ) -> Fallible<()>;
        fn acknowledge_mapped_completion(&mut self) -> Fallible<()>;
    }
}

use super::geometry_gpu::temporal_gpu::GpuPriorPublicLevelTwo;
#[cfg(test)]
use super::pis_frontend_gpu::{GpuCompletedColdCheckpoint, validate_completed_cold_final};
use super::pis_frontend_gpu::{GpuFinalDrawCarrier, GpuFinalOperands};
use crate::direct_type2::ImportedOneXsPicture;
use resident::Operands;

/// Capture-static final-map resources derived from one validated ONE X2
/// calibration and uploaded on the exact resident context.
///
/// No constructor or buffer accessor leaves this private owner. The only
/// operations copy its gate and coordinate payload into the materializer's
/// fixed layout and lend its alpha while constructing the final binding.
struct GpuFinalMapStatics {
    context: OneXsGpuContext,
    gate_coordinate: wgpu::Buffer,
    alpha: wgpu::Buffer,
}

impl GpuFinalMapStatics {
    fn new(context: OneXsGpuContext, resources: &OneXsResources) -> Arc<Self> {
        let words = pack_static_resources(resources);
        Self::from_words(context, &words, resources.alpha().bytes())
    }

    fn from_words(context: OneXsGpuContext, words: &[u32], alpha: &[u8]) -> Arc<Self> {
        assert_eq!(words.len(), STATIC_INPUT_WORDS);
        assert_eq!(alpha.len(), crate::studio_type2::ALPHA_BYTES);
        let gate_coordinate = upload_bytes(
            context.device(),
            context.queue(),
            "ONE X2 capture-static final-map gate and coordinates",
            u32_slice_bytes(words),
        );
        let alpha = upload_bytes(
            context.device(),
            context.queue(),
            "ONE X2 capture-static final-map alpha",
            alpha,
        );
        Arc::new(Self {
            context,
            gate_coordinate,
            alpha,
        })
    }

    fn encode_input_copy(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Buffer) {
        for side in 0..2 {
            encoder.copy_buffer_to_buffer(
                &self.gate_coordinate,
                byte_offset(side * STATIC_SIDE_WORDS),
                target,
                byte_offset(side * SIDE_WORDS + DYNAMIC_SIDE_WORDS),
                byte_offset(STATIC_SIDE_WORDS),
            );
        }
    }

    fn alpha_binding(&self) -> wgpu::BufferBinding<'_> {
        self.alpha.as_entire_buffer_binding()
    }
}

fn pack_static_resources(resources: &OneXsResources) -> Vec<u32> {
    let mut words = Vec::with_capacity(STATIC_INPUT_WORDS);
    append_static_side(
        &mut words,
        resources.gates().a.values(),
        resources.coordinates().a.values(),
    );
    append_static_side(
        &mut words,
        resources.gates().b.values(),
        resources.coordinates().b.values(),
    );
    words
}

fn append_static_side(words: &mut Vec<u32>, gates: &[f32], coordinates: &[[f32; 2]]) {
    debug_assert_eq!(gates.len(), GATE_WORDS);
    debug_assert_eq!(coordinates.len(), MAP_NODES);
    words.extend(gates.iter().map(|value| value.to_bits()));
    append_f32x2_words(words, coordinates);
}

fn append_f32x2_words(words: &mut Vec<u32>, values: &[[f32; 2]]) {
    words.extend(
        values
            .iter()
            .flat_map(|value| value.iter().map(|component| component.to_bits())),
    );
}

const fn byte_offset(words: usize) -> u64 {
    (words * size_of::<u32>()) as u64
}

fn upload_bytes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    bytes: &[u8],
) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytes);
    buffer
}

fn u32_slice_bytes(words: &[u32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), std::mem::size_of_val(words)) }
}

/// GPU-resident packed map bound to the exact delivery that produced it.
///
/// This deliberately exposes no ordinary CPU byte view. Evidence code must
/// call [`Self::diagnostic_readback`] and wait for an explicit copy.
pub(super) struct GpuPackedMapFrame<O: Operands> {
    // The inherited submission carrier is first so every refusal, unwind and
    // ordinary drop retires or quarantines it before any downstream owner.
    upstream: O,
    frame: FrameStamp,
    packed: wgpu::Buffer,
    _actions: wgpu::Buffer,
    context: OneXsGpuContext,
    statics: Arc<GpuFinalMapStatics>,
    fusion: Option<GpuFusionFrame>,
    #[cfg(test)]
    input_readback: wgpu::Buffer,
}

/// A resident final map whose four-byte fail-closed status has not completed.
///
/// This type has no Scene-binding operation. It retains the complete upstream
/// owner until a successful poll converts it into [`GpuPackedMapFrame`].
#[must_use = "the pending resident final-map validity has not been polled"]
pub(super) struct PendingGpuPackedMapFrame<O: Operands> {
    // See `GpuPackedMapFrame`: mapping failure must retire the inherited lease
    // before the exact statics or candidate allocations can be released.
    upstream: O,
    frame: FrameStamp,
    packed: wgpu::Buffer,
    actions: wgpu::Buffer,
    context: OneXsGpuContext,
    statics: Arc<GpuFinalMapStatics>,
    fusion: Option<GpuFusionFrame>,
    validity: wgpu::Buffer,
    mapped: mpsc::Receiver<Result<(), String>>,
    #[cfg(test)]
    input_readback: wgpu::Buffer,
}

/// Carrier-first owner spanning the only fallible submit. It becomes the
/// pending validity owner only after the existing lease accepts the command.
struct UnsubmittedGpuPackedMapFrame<O: Operands> {
    upstream: O,
    frame: FrameStamp,
    packed: wgpu::Buffer,
    actions: wgpu::Buffer,
    context: OneXsGpuContext,
    statics: Arc<GpuFinalMapStatics>,
    fusion: Option<GpuFusionFrame>,
    validity: wgpu::Buffer,
    #[cfg(test)]
    input_readback: wgpu::Buffer,
}

struct EncodedFinalMap {
    packed: wgpu::Buffer,
    actions: wgpu::Buffer,
    command: wgpu::CommandBuffer,
    fusion: Option<GpuFusionFrame>,
    #[cfg(test)]
    input_readback: wgpu::Buffer,
}

/// Color output belongs to the same source lease and delivery as its UV map.
/// The inputs remain owned through asynchronous completion; immutable ratios
/// then travel with the installed draw, never with a renderer-global history.
struct GpuFusionFrame {
    frame: FrameStamp,
    output: crate::image_fusion::gpu::Output,
    _inputs: crate::direct_type2::ResidentGpuBandInputs,
    _validity: wgpu::Buffer,
}

pub(super) enum ValidityPoll<O: Operands> {
    Pending(PendingGpuPackedMapFrame<O>),
    Ready(GpuPackedMapFrame<O>),
}

pub(super) enum ClassifiedValidityPoll<O: Operands> {
    Pending(PendingGpuPackedMapFrame<O>),
    Ready(GpuPackedMapFrame<O>),
    Refused(Box<dyn Error + Send + Sync>),
    Quarantined(Box<dyn Error + Send + Sync>),
}

impl<O: Operands> PendingGpuPackedMapFrame<O> {
    /// Drive callbacks once without waiting and consume the result if ready.
    #[allow(dead_code, reason = "legacy final-map oracle polling")]
    pub(super) fn poll(self) -> Fallible<ValidityPoll<O>> {
        if let Err(error) = self
            .context
            .device()
            .poll(wgpu::PollType::Poll)
            .map_err(Box::<dyn std::error::Error + Send + Sync>::from)
        {
            self.quarantine_uncertain();
            return Err(error);
        }
        match self.finish_after_poll_classified() {
            ClassifiedValidityPoll::Pending(pending) => Ok(ValidityPoll::Pending(pending)),
            ClassifiedValidityPoll::Ready(ready) => Ok(ValidityPoll::Ready(ready)),
            ClassifiedValidityPoll::Refused(error) | ClassifiedValidityPoll::Quarantined(error) => {
                Err(error)
            }
        }
    }

    /// Consume the callback state after the capture façade has already driven
    /// this exact device once for the redraw.
    #[allow(dead_code, reason = "explicit final-map oracle completion boundary")]
    pub(super) fn finish_after_poll(self) -> Fallible<ValidityPoll<O>> {
        match self.finish_after_poll_classified() {
            ClassifiedValidityPoll::Pending(pending) => Ok(ValidityPoll::Pending(pending)),
            ClassifiedValidityPoll::Ready(ready) => Ok(ValidityPoll::Ready(ready)),
            ClassifiedValidityPoll::Refused(error) | ClassifiedValidityPoll::Quarantined(error) => {
                Err(error)
            }
        }
    }

    pub(super) fn finish_after_poll_classified(self) -> ClassifiedValidityPoll<O> {
        match self.mapped.try_recv() {
            Err(mpsc::TryRecvError::Empty) => ClassifiedValidityPoll::Pending(self),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.quarantine_uncertain();
                ClassifiedValidityPoll::Quarantined(
                    "ONE X2 GPU validity mapping callback disconnected".into(),
                )
            }
            Ok(Err(error)) => {
                self.quarantine_uncertain();
                ClassifiedValidityPoll::Quarantined(error.into())
            }
            Ok(Ok(())) => self.finish_classified(),
        }
    }

    fn finish_classified(mut self) -> ClassifiedValidityPoll<O> {
        if let Err(error) = self.upstream.acknowledge_mapped_completion() {
            self.quarantine_uncertain();
            return ClassifiedValidityPoll::Quarantined(error);
        }
        let bytes = self.validity.slice(..).get_mapped_range();
        let status = u32::from_ne_bytes(bytes[..size_of::<u32>()].try_into().unwrap());
        drop(bytes);
        self.validity.unmap();
        let validity = decode_validity(status);
        if !matches!(validity, ResidentValidity::Success) {
            return ClassifiedValidityPoll::Refused(Box::new(validity));
        }
        ClassifiedValidityPoll::Ready(GpuPackedMapFrame {
            upstream: self.upstream,
            frame: self.frame,
            packed: self.packed,
            _actions: self.actions,
            context: self.context,
            statics: self.statics,
            fusion: self.fusion,
            #[cfg(test)]
            input_readback: self.input_readback,
        })
    }

    /// A failed device poll or map callback cannot prove whether the latest
    /// submission still references the decoder owner. Retain the complete
    /// carrier/root aggregate for process life without invoking its blocking
    /// exceptional Drop path.
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn quarantine_uncertain(self) {
        std::mem::forget(self);
    }

    #[cfg(test)]
    fn inject_mapping_failure(&mut self, message: &str) {
        let (sender, receiver) = mpsc::channel();
        sender.send(Err(message.to_owned())).unwrap();
        self.mapped = receiver;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResidentValidity {
    Success,
    Seed(SeedError),
    GeneratedHint {
        direction: super::super::Direction,
        level: GeneratedHintLevel,
        component: GeneratedHintComponent,
        dense_row: usize,
        dense_col: usize,
    },
    InvalidStatus(u32),
}

fn decode_validity(status: u32) -> ResidentValidity {
    if status == u32::MAX {
        return ResidentValidity::Success;
    }
    if status < PIS_VALIDITY_CODES {
        return ResidentValidity::Seed(pis_validity_error(status));
    }
    if status & GENERATED_HINT_FAILURE_TAG != 0 && status & GENERATED_HINT_RESERVED_MASK == 0 {
        let level = if status & GENERATED_HINT_LEVEL_BIT == 0 {
            GeneratedHintLevel::One
        } else {
            GeneratedHintLevel::Two
        };
        let direction = if status & GENERATED_HINT_DIRECTION_BIT == 0 {
            super::super::Direction::AtoB
        } else {
            super::super::Direction::BtoA
        };
        let component = if status & GENERATED_HINT_COMPONENT_BIT == 0 {
            GeneratedHintComponent::Dcol
        } else {
            GeneratedHintComponent::Drow
        };
        let site = (status & GENERATED_HINT_SITE_MASK) as usize;
        let (rows, cols) = match level {
            GeneratedHintLevel::One => (L1_DENSE_ROWS, L1_DENSE_COLS),
            GeneratedHintLevel::Two => (L2_DENSE_ROWS, L2_DENSE_COLS),
        };
        if site < rows * cols {
            return ResidentValidity::GeneratedHint {
                direction,
                level,
                component,
                dense_row: site / cols,
                dense_col: site % cols,
            };
        }
    }
    ResidentValidity::InvalidStatus(status)
}

fn pis_validity_error(status: u32) -> SeedError {
    let plane = status as usize / L1_PATCHES;
    let patch = status as usize % L1_PATCHES;
    let direction = if plane < 2 {
        super::super::Direction::AtoB
    } else {
        super::super::Direction::BtoA
    };
    let component = if plane.is_multiple_of(2) {
        "dcol"
    } else {
        "drow"
    };
    let patch_row = patch / L1_PATCH_COLS;
    let patch_col = patch % L1_PATCH_COLS;
    SeedError::NonFiniteCenter {
        direction,
        component,
        patch_row,
        patch_col,
        dense_row: patch_row * 3 + 4,
        dense_col: patch_col * 3 + 4,
    }
}

impl fmt::Display for GeneratedHintLevel {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(match self {
            Self::One => "level-one",
            Self::Two => "level-two",
        })
    }
}

impl fmt::Display for GeneratedHintComponent {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(match self {
            Self::Dcol => "dcol",
            Self::Drow => "drow",
        })
    }
}

impl fmt::Display for ResidentValidity {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Success => output.write_str("ONE X2 GPU resident validity succeeded"),
            Self::Seed(error) => error.fmt(output),
            Self::GeneratedHint {
                direction,
                level,
                component,
                dense_row,
                dense_col,
            } => write!(
                output,
                "ONE X2 {direction} generated {level} {component} successor hint at dense row {dense_row} column {dense_col} is not finite",
            ),
            Self::InvalidStatus(status) => write!(
                output,
                "ONE X2 GPU resident validity returned unknown status word {status:#010x}",
            ),
        }
    }
}

impl Error for ResidentValidity {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Seed(error) => Some(error),
            Self::Success | Self::GeneratedHint { .. } | Self::InvalidStatus(_) => None,
        }
    }
}

impl<O: Operands> GpuPackedMapFrame<O> {
    #[cfg(test)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn diagnostic_readback(
        &self,
    ) -> Fallible<DiagnosticPackedMap> {
        let packed = readback_packed(self.context.device(), self.context.queue(), &self.packed)?;
        Ok(DiagnosticPackedMap {
            frame: self.frame.clone(),
            packed,
            input: readback_u32(
                self.context.device(),
                self.context.queue(),
                &self.input_readback,
                INPUT_BYTES,
            )?,
        })
    }

    #[cfg(test)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn assert_cpu_twin_for_test(
        &self,
    ) -> Fallible<()> {
        let diagnostic = self.diagnostic_readback()?;
        let expected = materialize_words(&diagnostic.input)?;
        compare_bits(diagnostic.packed.nodes(), &expected)
    }

    #[cfg(test)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn assert_alpha_for_test(
        &self,
        expected: &[u8],
    ) -> Fallible<()> {
        if expected.len() != ALPHA_BYTES {
            return Err("ONE X2 final-map alpha oracle has the wrong byte count".into());
        }
        let actual = readback_u32(
            self.context.device(),
            self.context.queue(),
            &self.statics.alpha,
            ALPHA_BYTES as u64,
        )?;
        let expected = expected
            .chunks_exact(4)
            .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
            .collect::<Vec<_>>();
        if actual != expected {
            return Err("ONE X2 final-map alpha differs from its frozen CPU resource".into());
        }
        Ok(())
    }

    /// Validate exact context and delivery identity, then build the two-buffer
    /// binding consumed by the existing direct type-2 Scene shader. No raw
    /// buffer leaves this module.
    #[allow(dead_code, reason = "legacy final-map binding oracle")]
    pub(super) fn bind_for_scene(
        self,
        context: &OneXsGpuContext,
        expected_frame: &FrameStamp,
        layout: &wgpu::BindGroupLayout,
    ) -> Result<GpuMapBinding<O>, BindingError> {
        self.context
            .ensure_same(context)
            .map_err(|_| BindingError::Context)?;
        if self.frame != *expected_frame {
            return Err(BindingError::Frame);
        }
        let read = context
            .device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ONE X2 resident native type-2 resources"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.packed.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(self.statics.alpha_binding()),
                    },
                ],
            });
        Ok(GpuMapBinding {
            frame: self.frame.clone(),
            read,
            _resident: self,
        })
    }
}

pub(super) struct GpuBoundFinalMap {
    pub(super) source: ImportedOneXsPicture,
    pub(super) binding: InstalledGpuMapBinding,
    pub(super) candidate: super::resident_frame_gpu::GpuResidentCandidate,
}

pub(super) struct InstalledGpuMapBinding {
    frame: FrameStamp,
    read: wgpu::BindGroup,
    fusion_read: Option<wgpu::BindGroup>,
    fusion: Option<GpuFusionFrame>,
    packed: wgpu::Buffer,
    _actions: wgpu::Buffer,
    context: OneXsGpuContext,
    statics: Arc<GpuFinalMapStatics>,
    #[cfg(test)]
    input_readback: wgpu::Buffer,
    carrier: Box<dyn InstalledFinalCarrier>,
}

/// Sample-only installed map resources detached from the heavyweight final-map
/// carrier. The bind groups retain their immutable wgpu buffers and ratio
/// textures; unlike imported source textures, none aliases decoder memory.
pub(super) struct MapSnapshot {
    frame: FrameStamp,
    read: wgpu::BindGroup,
    fusion_read: Option<wgpu::BindGroup>,
    context: OneXsGpuContext,
}

impl MapSnapshot {
    pub(super) fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub(super) fn read(&self) -> &wgpu::BindGroup {
        &self.read
    }

    pub(super) fn fusion_read(&self) -> Option<&wgpu::BindGroup> {
        self.fusion_read.as_ref()
    }

    pub(super) fn ensure_context(&self, expected: &OneXsGpuContext) -> Fallible<()> {
        self.context.ensure_same(expected)
    }
}

#[cfg(test)]
pub(crate) struct DiagnosticFinalInputs {
    pub frame: FrameStamp,
    pub lens_a_preimage: Vec<u32>,
    pub lens_b_preimage: Vec<u32>,
    pub lens_a_base: Vec<u32>,
    pub lens_b_base: Vec<u32>,
    pub lens_a_public: Vec<u32>,
    pub lens_b_public: Vec<u32>,
    words: Vec<u32>,
}

#[cfg(test)]
impl DiagnosticFinalInputs {
    fn from_words(frame: FrameStamp, words: Vec<u32>) -> Fallible<Self> {
        if words.len() != INPUT_WORDS {
            return Err("ONE X2 diagnostic final inputs have the wrong word count".into());
        }
        let side = |side: usize| {
            let start = side * SIDE_WORDS;
            let preimage = words[start..start + PREIMAGE_WORDS].to_vec();
            let base_start = start + PREIMAGE_WORDS;
            let base = words[base_start..base_start + BASE_WORDS].to_vec();
            let public_start = base_start + BASE_WORDS;
            let public = words[public_start..public_start + FLOW_WORDS].to_vec();
            (preimage, base, public)
        };
        let (lens_a_preimage, lens_a_base, lens_a_public) = side(0);
        let (lens_b_preimage, lens_b_base, lens_b_public) = side(1);
        Ok(Self {
            frame,
            lens_a_preimage,
            lens_b_preimage,
            lens_a_base,
            lens_b_base,
            lens_a_public,
            lens_b_public,
            words,
        })
    }

    /// Materialize this frame's exact preimage, base and capture-static inputs
    /// with another completed frame's public correction field.
    ///
    /// This is an offline diagnostic only. It neither submits GPU work nor
    /// mutates either frame's resident temporal history.
    pub(crate) fn materialize_with_public_from(&self, correction: &Self) -> Fallible<PackedMap> {
        if self.words.len() != INPUT_WORDS {
            return Err("ONE X2 diagnostic current final inputs have the wrong word count".into());
        }
        for (name, flow) in [
            ("lens A", correction.lens_a_public.as_slice()),
            ("lens B", correction.lens_b_public.as_slice()),
        ] {
            if flow.len() != FLOW_WORDS {
                return Err(format!(
                    "ONE X2 diagnostic {name} carried public flow has {} words, expected {FLOW_WORDS}",
                    flow.len()
                )
                .into());
            }
        }

        let mut words = self.words.clone();
        let flow_offset = PREIMAGE_WORDS + BASE_WORDS;
        for (side, flow) in [
            correction.lens_a_public.as_slice(),
            correction.lens_b_public.as_slice(),
        ]
        .into_iter()
        .enumerate()
        {
            let start = side * SIDE_WORDS + flow_offset;
            words[start..start + FLOW_WORDS].copy_from_slice(flow);
        }
        Ok(PackedMap::new(materialize_words(&words)?)?)
    }
}

/// Type-erased only after the complete typed post owner has crossed the
/// final-map boundary. The installed draw needs root comparison and lifetime
/// retention, not access to cold- or warm-specific state.
trait InstalledFinalCarrier: Send + Sync {
    fn matches_root(&self, root: &super::resident_frame_gpu::GpuResidentIdentity) -> bool;
}

impl<P> InstalledFinalCarrier for GpuFinalDrawCarrier<P>
where
    P: GpuPriorPublicLevelTwo + Send + Sync,
{
    fn matches_root(&self, root: &super::resident_frame_gpu::GpuResidentIdentity) -> bool {
        self.matches_root(root)
    }
}

impl InstalledGpuMapBinding {
    pub(super) fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub(super) fn read(&self) -> &wgpu::BindGroup {
        &self.read
    }

    pub(super) fn fusion_read(&self) -> Option<&wgpu::BindGroup> {
        self.fusion_read.as_ref()
    }

    /// Detach only the immutable resources sampled by a later display draw.
    /// Frame and device are checked before any handle is cloned; production
    /// carriers, mutable receipts and decoder-backed inputs do not cross.
    pub(super) fn snapshot(
        &self,
        expected: &FrameStamp,
        context: &OneXsGpuContext,
    ) -> Fallible<MapSnapshot> {
        if &self.frame != expected {
            return Err("resident map snapshot names a different source frame".into());
        }
        self.context.ensure_same(context)?;
        Ok(MapSnapshot {
            frame: self.frame.clone(),
            read: self.read.clone(),
            fusion_read: self.fusion_read.clone(),
            context: self.context.clone(),
        })
    }

    /// Populate one compact-panorama vertex cache from this exact installed
    /// map. The packed and alpha buffers remain private to their installed
    /// owner; only the opaque cache result crosses this boundary.
    pub(super) fn prepare_compact_vertex_cache(
        &self,
        expected_frame: &FrameStamp,
        pipeline: &crate::direct_type2::DirectType2Pipeline,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Fallible<crate::direct_type2::panorama::nv12_vertex_cache::Prepared> {
        if &self.frame != expected_frame {
            return Err("resident compact vertex cache map names a different source frame".into());
        }
        if self.context.device() != device {
            return Err(
                "resident compact vertex cache belongs to a different graphics device".into(),
            );
        }
        pipeline.ensure_device(&self.context)?;
        pipeline.vertex_cached_compact_nv12().prepare(
            device,
            encoder,
            &self.packed,
            &self.statics.alpha,
        )
    }

    pub(super) fn matches_root(
        &self,
        root: &super::resident_frame_gpu::GpuResidentIdentity,
    ) -> bool {
        self.carrier.matches_root(root)
    }

    pub(super) fn diagnostic_readback(&self) -> Fallible<crate::OneXsMapFrame> {
        let packed = readback_packed(self.context.device(), self.context.queue(), &self.packed)?;
        let alpha = readback_u32(
            self.context.device(),
            self.context.queue(),
            &self.statics.alpha,
            ALPHA_BYTES as u64,
        )?
        .into_iter()
        .map(f32::from_bits)
        .collect();
        let map = crate::OneXsMapFrame::new(
            self.frame.clone(),
            packed,
            crate::studio_type2::AlphaMap::new(alpha)?,
            crate::studio_type2::PisBackend::Gpu,
        );
        match &self.fusion {
            None => Ok(map),
            Some(fusion) => {
                let read = |texture| {
                    readback_fusion_texture(self.context.device(), self.context.queue(), texture)
                };
                Ok(map.with_fusion(crate::image_fusion::RatioPair {
                    left: read(&fusion.output.textures[0])?,
                    right: read(&fusion.output.textures[1])?,
                }))
            }
        }
    }

    #[cfg(test)]
    pub(super) fn diagnostic_final_inputs(&self) -> Fallible<DiagnosticFinalInputs> {
        let words = readback_u32(
            self.context.device(),
            self.context.queue(),
            &self.input_readback,
            INPUT_BYTES,
        )?;
        DiagnosticFinalInputs::from_words(self.frame.clone(), words)
    }
}

impl<P> GpuPackedMapFrame<GpuFinalOperands<P>>
where
    P: GpuPriorPublicLevelTwo + Send + Sync + 'static,
{
    #[allow(dead_code, reason = "explicit final-map oracle identity inspection")]
    pub(super) fn frame_stamp(&self) -> &FrameStamp {
        &self.frame
    }

    pub(super) fn install_context(&self) -> OneXsGpuContext {
        self.context.clone()
    }

    /// Consume the validated map into a root-free binding plus the one exact
    /// root installation capability. Nothing returned exposes a raw resource.
    pub(super) fn bind_for_install(
        self,
        context: &OneXsGpuContext,
        pipeline: &crate::direct_type2::DirectType2Pipeline,
    ) -> Fallible<GpuBoundFinalMap> {
        self.context.ensure_same(context)?;
        pipeline.ensure_device(context)?;
        if self.fusion.is_some() != pipeline.fusion_layout().is_some() {
            return Err(
                "image fusion map and draw pipeline disagree about color correction".into(),
            );
        }
        let fusion_read = self
            .fusion
            .as_ref()
            .map(|fusion| {
                if fusion.frame != self.frame {
                    return Err(
                        "image fusion ratios name a different source frame than the stitch map"
                            .into(),
                    );
                }
                pipeline.bind_fusion_textures(
                    context.device(),
                    [&fusion.output.textures[0], &fusion.output.textures[1]],
                )
            })
            .transpose()?;
        let read = context
            .device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ONE X2 installed resident native type-2 resources"),
                layout: pipeline.map_layout(),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.packed.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(self.statics.alpha_binding()),
                    },
                ],
            });
        let parts = self.upstream.into_install_parts()?;
        let binding = InstalledGpuMapBinding {
            frame: self.frame.clone(),
            read,
            fusion_read,
            fusion: self.fusion,
            packed: self.packed,
            _actions: self._actions,
            context: self.context,
            statics: self.statics,
            #[cfg(test)]
            input_readback: self.input_readback,
            carrier: Box::new(parts.carrier),
        };
        Ok(GpuBoundFinalMap {
            source: parts.source,
            binding,
            candidate: parts.candidate,
        })
    }
}

#[allow(dead_code, reason = "legacy final-map binding oracle")]
pub(super) struct GpuMapBinding<O: Operands> {
    frame: FrameStamp,
    read: wgpu::BindGroup,
    _resident: GpuPackedMapFrame<O>,
}

#[allow(dead_code, reason = "legacy final-map binding oracle")]
impl<O: Operands> GpuMapBinding<O> {
    pub(super) fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub(super) fn read(&self) -> &wgpu::BindGroup {
        &self.read
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code, reason = "legacy final-map binding oracle error surface")]
pub(super) enum BindingError {
    Context,
    Frame,
}

impl fmt::Display for BindingError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Context => {
                output.write_str("ONE X2 resident map belongs to a different GPU context")
            }
            Self::Frame => output.write_str("ONE X2 resident map names a different frame"),
        }
    }
}

impl Error for BindingError {}

fn readback_packed(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    packed: &wgpu::Buffer,
) -> Fallible<PackedMap> {
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ONE X2 diagnostic GPU final-map readback"),
        size: PACKED_BYTES as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ONE X2 diagnostic GPU final-map readback"),
    });
    encoder.copy_buffer_to_buffer(packed, 0, &readback, 0, PACKED_BYTES as u64);
    let submission = queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (mapped, answer) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = mapped.send(result);
    });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    answer.recv()??;
    let bytes = slice.get_mapped_range();
    let nodes = bytes
        .chunks_exact(size_of::<[f32; 4]>())
        .map(|node| {
            std::array::from_fn(|component| {
                let start = component * size_of::<f32>();
                f32::from_ne_bytes(node[start..start + 4].try_into().unwrap())
            })
        })
        .collect::<Vec<_>>();
    drop(bytes);
    readback.unmap();
    Ok(PackedMap::new(nodes).expect("fixed GPU output has the type-2 map shape"))
}

/// Explicit diagnostic copy of the same immutable f32 texture the draw samples.
/// Live playback neither allocates this staging buffer nor waits for its copy.
fn readback_fusion_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Texture,
) -> Fallible<crate::image_fusion::RatioMap> {
    const ROW_BYTES: u32 = (crate::studio_type2::MAP_WIDTH * size_of::<[f32; 4]>()) as u32;
    const PADDED_ROW_BYTES: u32 =
        ROW_BYTES.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    const HEIGHT: u32 = crate::studio_type2::MAP_HEIGHT as u32;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("image fusion diagnostic texture readback"),
        size: u64::from(PADDED_ROW_BYTES) * u64::from(HEIGHT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("image fusion diagnostic texture readback"),
    });
    encoder.copy_texture_to_buffer(
        source.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(PADDED_ROW_BYTES),
                rows_per_image: Some(HEIGHT),
            },
        },
        source.size(),
    );
    let submission = queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (mapped, answer) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = mapped.send(result);
    });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    answer.recv()??;
    let bytes = slice.get_mapped_range();
    let values = bytes
        .chunks_exact(PADDED_ROW_BYTES as usize)
        .flat_map(|row| row[..ROW_BYTES as usize].chunks_exact(size_of::<[f32; 4]>()))
        .map(|node| {
            std::array::from_fn(|channel| {
                let start = channel * size_of::<f32>();
                f32::from_ne_bytes(node[start..start + 4].try_into().unwrap())
            })
        })
        .collect();
    drop(bytes);
    readback.unmap();
    Ok(crate::image_fusion::RatioMap::new(values)?)
}

fn readback_u32(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Buffer,
    size: u64,
) -> Fallible<Vec<u32>> {
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ONE X2 diagnostic GPU final-map action readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ONE X2 diagnostic GPU final-map action readback"),
    });
    encoder.copy_buffer_to_buffer(source, 0, &readback, 0, size);
    let submission = queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (mapped, answer) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = mapped.send(result);
    });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    answer.recv()??;
    let bytes = slice.get_mapped_range();
    let words = bytes
        .chunks_exact(size_of::<u32>())
        .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
        .collect();
    drop(bytes);
    readback.unmap();
    Ok(words)
}

/// Explicit CPU diagnostic copy of one frame-bound GPU result.
#[cfg(test)]
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct DiagnosticPackedMap {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) frame: FrameStamp,
    pub(in crate::flow::one_xs::one_xs_belt_gpu) packed: PackedMap,
    pub(in crate::flow::one_xs::one_xs_belt_gpu) input: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg(test)]
struct QualificationError {
    node: usize,
    component: usize,
    actual: u32,
    expected: u32,
}

#[cfg(test)]
impl std::fmt::Display for QualificationError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.component == 4 {
            return write!(
                output,
                "ONE X2 GPU final-map branch receipt is not exact on this graphics device: node {} action is {}, expected {}",
                self.node, self.actual, self.expected,
            );
        }
        write!(
            output,
            "ONE X2 GPU final-map arithmetic is not exact on this graphics device: node {} component {} bits are {:#010x}, expected {:#010x}",
            self.node, self.component, self.actual, self.expected,
        )
    }
}

#[cfg(test)]
impl std::error::Error for QualificationError {}

/// Render-private final-map compute pipeline.
pub(super) struct GpuMapMaterializer {
    context: OneXsGpuContext,
    statics: Arc<GpuFinalMapStatics>,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl GpuMapMaterializer {
    pub(super) fn new(context: OneXsGpuContext, resources: &OneXsResources) -> Fallible<Self> {
        let statics = GpuFinalMapStatics::new(context.clone(), resources);
        Self::from_shader(context, statics, SHADER)
    }

    fn from_shader(
        context: OneXsGpuContext,
        statics: Arc<GpuFinalMapStatics>,
        shader: &str,
    ) -> Fallible<Self> {
        context.ensure_same(&statics.context)?;
        let device = context.device();
        let storage = |binding, read_only, bytes| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(bytes),
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ONE X2 GPU final-map materializer"),
            entries: &[
                storage(0, true, INPUT_BYTES),
                storage(1, false, PACKED_BYTES as u64),
                storage(2, false, ACTION_BYTES),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 GPU final-map materializer"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 GPU final-map materializer"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ONE X2 GPU final-map materializer"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("materialize_type2"),
            compilation_options: Default::default(),
            cache: None,
        });
        Ok(Self {
            context,
            statics,
            pipeline,
            layout,
        })
    }

    /// Consume one sealed upstream token and emit an opaque resident map.
    ///
    /// Submission goes through the upstream token so its existing source-owner
    /// lease advances to this dispatch. Only the resident validity word is
    /// copied for asynchronous CPU observation.
    pub(super) fn materialize_final<P>(
        &self,
        operands: GpuFinalOperands<P>,
    ) -> Fallible<PendingGpuPackedMapFrame<GpuFinalOperands<P>>>
    where
        P: GpuPriorPublicLevelTwo,
    {
        self.materialize_inner(operands)
    }

    /// Append color sampling and correction to the final-map command, before
    /// its existing completion word. All source reads therefore advance the
    /// same upstream lease; no post-install read can outlive decoder ownership.
    pub(super) fn materialize_final_fused<P>(
        &self,
        operands: GpuFinalOperands<P>,
        sampler: &crate::image_fusion::sample::FusionInputPipeline,
        producer: &mut crate::image_fusion::gpu::Producer,
    ) -> Fallible<PendingGpuPackedMapFrame<GpuFinalOperands<P>>>
    where
        P: GpuPriorPublicLevelTwo,
    {
        self.materialize_inner_with(operands, |operands, encoder, packed| {
            let inputs = operands.encode_fusion_inputs(
                &self.context,
                operands.frame(),
                encoder,
                sampler,
                packed,
            )?;
            let validity = self
                .context
                .device()
                .create_buffer(&wgpu::BufferDescriptor {
                    label: Some("image fusion resident validity"),
                    size: VALIDITY_BYTES,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            operands.encode_validity_copy(encoder, &validity);
            let output = producer.encode(
                self.context.device(),
                encoder,
                inputs.bands(),
                inputs.invalid(),
                &validity,
            )?;
            Ok(Some(GpuFusionFrame {
                frame: operands.frame().clone(),
                output,
                _inputs: inputs,
                _validity: validity,
            }))
        })
    }

    #[cfg(test)]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn validate_completed_cold_for_test(
        &self,
        checkpoint: &GpuCompletedColdCheckpoint<ImportedOneXsPicture>,
    ) -> Fallible<()> {
        validate_completed_cold_final(checkpoint, &self.context)
    }

    #[cfg(test)]
    fn materialize<O: Operands>(&self, operands: O) -> Fallible<PendingGpuPackedMapFrame<O>> {
        self.materialize_inner(operands)
    }

    fn materialize_inner<O: Operands>(&self, operands: O) -> Fallible<PendingGpuPackedMapFrame<O>> {
        self.materialize_inner_with(operands, |_, _, _| Ok(None))
    }

    fn materialize_inner_with<O: Operands>(
        &self,
        operands: O,
        encode_fusion: impl FnOnce(
            &O,
            &mut wgpu::CommandEncoder,
            &wgpu::Buffer,
        ) -> Fallible<Option<GpuFusionFrame>>,
    ) -> Fallible<PendingGpuPackedMapFrame<O>> {
        self.context.ensure_same(operands.context())?;
        let frame = operands.frame().clone();
        let input_usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        #[cfg(test)]
        let input_usage = input_usage | wgpu::BufferUsages::COPY_SRC;
        let input = self
            .context
            .device()
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("ONE X2 resident GPU final-map inputs"),
                size: INPUT_BYTES,
                usage: input_usage,
                mapped_at_creation: false,
            });
        let validity = self
            .context
            .device()
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("ONE X2 resident GPU final-map validity readback"),
                size: VALIDITY_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        let encoded = self.encode(&operands, &input, &validity, encode_fusion)?;
        let mut unsubmitted = UnsubmittedGpuPackedMapFrame {
            upstream: operands,
            frame,
            packed: encoded.packed,
            actions: encoded.actions,
            context: self.context.clone(),
            statics: Arc::clone(&self.statics),
            fusion: encoded.fusion,
            validity,
            #[cfg(test)]
            input_readback: encoded.input_readback,
        };
        unsubmitted
            .upstream
            .submit_after(&self.context, encoded.command)?;
        let slice = unsubmitted.validity.slice(..);
        let (sender, mapped) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result.map_err(|error| error.to_string()));
        });
        Ok(PendingGpuPackedMapFrame {
            upstream: unsubmitted.upstream,
            frame: unsubmitted.frame,
            packed: unsubmitted.packed,
            actions: unsubmitted.actions,
            context: unsubmitted.context,
            statics: unsubmitted.statics,
            fusion: unsubmitted.fusion,
            validity: unsubmitted.validity,
            mapped,
            #[cfg(test)]
            input_readback: unsubmitted.input_readback,
        })
    }

    fn encode<O: Operands>(
        &self,
        operands: &O,
        input: &wgpu::Buffer,
        validity: &wgpu::Buffer,
        encode_fusion: impl FnOnce(
            &O,
            &mut wgpu::CommandEncoder,
            &wgpu::Buffer,
        ) -> Fallible<Option<GpuFusionFrame>>,
    ) -> Fallible<EncodedFinalMap> {
        let device = self.context.device();
        let packed = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 GPU-resident packed type-2 map"),
            size: PACKED_BYTES as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let actions = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 GPU final-map action receipt"),
            size: ACTION_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 GPU final-map resources"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: packed.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: actions.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 GPU final-map materializer"),
        });
        operands.encode_dynamic_input_copy(&mut encoder, resident::DynamicInputTarget::new(input));
        self.statics.encode_input_copy(&mut encoder, input);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 GPU final-map materializer"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &resources, &[]);
            pass.dispatch_workgroups((MAP_NODES as u32).div_ceil(WORKGROUP_SIZE), 1, 1);
        }
        let fusion = encode_fusion(operands, &mut encoder, &packed)?;
        #[cfg(test)]
        let input_readback = {
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ONE X2 diagnostic final-map input readback"),
                size: INPUT_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            encoder.copy_buffer_to_buffer(input, 0, &readback, 0, INPUT_BYTES);
            readback
        };
        operands.encode_validity_copy(&mut encoder, validity);
        Ok(EncodedFinalMap {
            packed,
            actions,
            command: encoder.finish(),
            fusion,
            #[cfg(test)]
            input_readback,
        })
    }

    #[cfg(test)]
    fn qualify_fixture(&self, fixture: &QualificationFixture) -> Fallible<()> {
        fixture.verify_cpu_coverage();
        let inputs = fixture.cpu_inputs();
        let expected = map_patch::materialize(inputs).packed;
        let words = pack_dynamic_inputs(inputs);
        let operands = TestResidentOperands::new(
            self.context.clone(),
            FrameStamp::for_test(0, std::time::Duration::ZERO, None),
            &words,
        );
        let result = expect_ready(self.materialize(operands)?)?;
        let actual = result.diagnostic_readback()?.packed;
        compare_bits(actual.nodes(), &expected)?;
        let actions = readback_u32(
            self.context.device(),
            self.context.queue(),
            &result._actions,
            ACTION_BYTES,
        )?;
        fixture.verify_gpu_actions(&actions)?;
        Ok(())
    }

    #[cfg(test)]
    fn for_qualification(
        context: OneXsGpuContext,
        shader: &str,
        fixture: &QualificationFixture,
    ) -> Fallible<Self> {
        let statics = fixture.gpu_statics(context.clone());
        Self::from_shader(context, statics, shader)
    }

    #[cfg(test)]
    fn qualify_shader(context: OneXsGpuContext, shader: &str) -> Fallible<()> {
        let fixture = QualificationFixture::new();
        Self::for_qualification(context, shader, &fixture)?.qualify_fixture(&fixture)
    }
}

#[cfg(test)]
fn pack_dynamic_inputs(inputs: BilateralInputs<'_>) -> Vec<u32> {
    let mut words = Vec::with_capacity(DYNAMIC_INPUT_WORDS);
    append_dynamic_side(&mut words, inputs.b_to_a);
    append_dynamic_side(&mut words, inputs.a_to_b);
    words
}

#[cfg(test)]
fn pack_static_inputs(inputs: BilateralInputs<'_>) -> Vec<u32> {
    let mut words = Vec::with_capacity(STATIC_INPUT_WORDS);
    append_static_side(
        &mut words,
        inputs.b_to_a.gate.values(),
        inputs.b_to_a.coordinate.values(),
    );
    append_static_side(
        &mut words,
        inputs.a_to_b.gate.values(),
        inputs.a_to_b.coordinate.values(),
    );
    words
}

#[cfg(test)]
fn append_dynamic_side(words: &mut Vec<u32>, side: SideInputs<'_>) {
    append_f32x2_words(words, side.preimage.values());
    append_f32x2_words(words, side.base.values());
    append_f32x2_words(words, side.flow.values());
}

#[cfg(test)]
fn materialize_words(words: &[u32]) -> Fallible<Vec<[f32; 4]>> {
    if words.len() != INPUT_WORDS {
        return Err("ONE X2 final-map CPU twin input has the wrong word count".into());
    }
    struct OwnedSide {
        preimage: PreimageMap,
        base: BaseMap,
        flow: FlowMap,
        gate: GateMap,
        coordinate: CoordinateMap,
    }
    let vec2 = |start: usize, nodes: usize| {
        words[start..start + 2 * nodes]
            .chunks_exact(2)
            .map(|pair| [f32::from_bits(pair[0]), f32::from_bits(pair[1])])
            .collect::<Vec<_>>()
    };
    let scalar = |start: usize, nodes: usize| {
        words[start..start + nodes]
            .iter()
            .map(|word| f32::from_bits(*word))
            .collect::<Vec<_>>()
    };
    let side = |start: usize| -> Fallible<OwnedSide> {
        let preimage = start;
        let base = preimage + PREIMAGE_WORDS;
        let flow = base + BASE_WORDS;
        let gate = flow + FLOW_WORDS;
        let coordinate = gate + GATE_WORDS;
        Ok(OwnedSide {
            preimage: PreimageMap::new(vec2(preimage, MAP_NODES))?,
            base: BaseMap::new(vec2(base, RETAINED_NODES))?,
            flow: FlowMap::new(vec2(flow, RETAINED_NODES))?,
            gate: GateMap::new(scalar(gate, MAP_NODES))?,
            coordinate: CoordinateMap::new(vec2(coordinate, MAP_NODES))?,
        })
    };
    let left = side(0)?;
    let right = side(SIDE_WORDS)?;
    Ok(map_patch::materialize(BilateralInputs {
        b_to_a: SideInputs {
            preimage: &left.preimage,
            base: &left.base,
            flow: &left.flow,
            gate: &left.gate,
            coordinate: &left.coordinate,
        },
        a_to_b: SideInputs {
            preimage: &right.preimage,
            base: &right.base,
            flow: &right.flow,
            gate: &right.gate,
            coordinate: &right.coordinate,
        },
    })
    .packed)
}

#[cfg(test)]
fn compare_bits(actual: &[[f32; 4]; MAP_NODES], expected: &[[f32; 4]]) -> Fallible<()> {
    for (node, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        for component in 0..4 {
            let actual = actual[component].to_bits();
            let expected = expected[component].to_bits();
            if actual != expected {
                return Err(QualificationError {
                    node,
                    component,
                    actual,
                    expected,
                }
                .into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
fn expect_ready<O: Operands>(
    mut pending: PendingGpuPackedMapFrame<O>,
) -> Fallible<GpuPackedMapFrame<O>> {
    for _ in 0..100 {
        match pending.poll()? {
            ValidityPoll::Ready(frame) => return Ok(frame),
            ValidityPoll::Pending(next) => {
                pending = next;
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    }
    Err("ONE X2 GPU validity remained pending after bounded safe polling".into())
}

#[cfg(test)]
fn expect_refusal<O: Operands>(
    mut pending: PendingGpuPackedMapFrame<O>,
) -> Box<dyn std::error::Error + Send + Sync> {
    for _ in 0..100 {
        match pending.poll() {
            Err(error) => return error,
            Ok(ValidityPoll::Ready(_)) => {
                panic!("invalid resident validity became Scene-bindable")
            }
            Ok(ValidityPoll::Pending(next)) => {
                pending = next;
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    }
    panic!("invalid validity remained pending after bounded safe polling")
}

#[cfg(test)]
fn upload(device: &wgpu::Device, queue: &wgpu::Queue, label: &str, bytes: &[u8]) -> wgpu::Buffer {
    upload_bytes(device, queue, label, bytes)
}

#[cfg(test)]
struct TestResidentOperands {
    context: OneXsGpuContext,
    frame: FrameStamp,
    dynamic: wgpu::Buffer,
    validity: wgpu::Buffer,
    submit_error: Option<String>,
    #[allow(dead_code, reason = "drop-order witness retained by oracle payload")]
    drop_witness: Option<TestDropWitness>,
}

#[cfg(test)]
struct TestDropWitness {
    count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    statics: Option<std::sync::Weak<GpuFinalMapStatics>>,
    observed_statics_strong_count: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
}

#[cfg(test)]
impl TestDropWitness {
    fn ordered(
        count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        statics: std::sync::Weak<GpuFinalMapStatics>,
        observed_statics_strong_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        Self {
            count,
            statics: Some(statics),
            observed_statics_strong_count: Some(observed_statics_strong_count),
        }
    }
}

#[cfg(test)]
impl Drop for TestDropWitness {
    fn drop(&mut self) {
        self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let (Some(statics), Some(observed)) =
            (&self.statics, &self.observed_statics_strong_count)
        {
            observed.store(statics.strong_count(), std::sync::atomic::Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
impl TestResidentOperands {
    fn new(context: OneXsGpuContext, frame: FrameStamp, words: &[u32]) -> Self {
        Self::with_validity(context, frame, words, u32::MAX)
    }

    fn with_validity(
        context: OneXsGpuContext,
        frame: FrameStamp,
        words: &[u32],
        validity_word: u32,
    ) -> Self {
        Self::with_validity_and_witness(context, frame, words, validity_word, None)
    }

    fn with_validity_and_witness(
        context: OneXsGpuContext,
        frame: FrameStamp,
        words: &[u32],
        validity_word: u32,
        drop_witness: Option<TestDropWitness>,
    ) -> Self {
        assert_eq!(words.len(), DYNAMIC_INPUT_WORDS);
        let dynamic = upload(
            context.device(),
            context.queue(),
            "ONE X2 diagnostic resident final-map operands",
            u32_slice_bytes(words),
        );
        let validity = upload(
            context.device(),
            context.queue(),
            "ONE X2 diagnostic resident validity",
            &validity_word.to_ne_bytes(),
        );
        Self {
            context,
            frame,
            dynamic,
            validity,
            submit_error: None,
            drop_witness,
        }
    }

    fn with_submit_failure(
        context: OneXsGpuContext,
        frame: FrameStamp,
        words: &[u32],
        message: &str,
        drop_witness: TestDropWitness,
    ) -> Self {
        let mut operands =
            Self::with_validity_and_witness(context, frame, words, u32::MAX, Some(drop_witness));
        operands.submit_error = Some(message.to_owned());
        operands
    }
}

#[cfg(test)]
impl resident::Sealed for TestResidentOperands {}

#[cfg(test)]
impl resident::Operands for TestResidentOperands {
    fn context(&self) -> &OneXsGpuContext {
        &self.context
    }

    fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    fn encode_dynamic_input_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: resident::DynamicInputTarget<'_>,
    ) {
        for (side_index, side) in [resident::DynamicSide::LensA, resident::DynamicSide::LensB]
            .into_iter()
            .enumerate()
        {
            let source = side_index * DYNAMIC_SIDE_WORDS;
            target.copy_side(
                encoder,
                side,
                resident::DynamicBufferCopy::new(&self.dynamic, byte_offset(source)),
                resident::DynamicBufferCopy::new(
                    &self.dynamic,
                    byte_offset(source + PREIMAGE_WORDS),
                ),
                resident::DynamicBufferCopy::new(
                    &self.dynamic,
                    byte_offset(source + PREIMAGE_WORDS + BASE_WORDS),
                ),
            );
        }
    }

    fn encode_validity_copy(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Buffer) {
        encoder.copy_buffer_to_buffer(&self.validity, 0, target, 0, VALIDITY_BYTES);
    }

    fn submit_after(
        &mut self,
        producer: &OneXsGpuContext,
        command: wgpu::CommandBuffer,
    ) -> Fallible<()> {
        self.context.ensure_same(producer)?;
        if let Some(error) = self.submit_error.take() {
            return Err(error.into());
        }
        self.context.queue().submit([command]);
        Ok(())
    }

    fn acknowledge_mapped_completion(&mut self) -> Fallible<()> {
        Ok(())
    }
}

#[cfg(test)]
struct QualificationFixture {
    preimage: super::super::LensPair<PreimageMap>,
    base: super::super::LensPair<BaseMap>,
    flow: super::super::LensPair<FlowMap>,
    gate: super::super::LensPair<GateMap>,
    coordinate: super::super::LensPair<CoordinateMap>,
    coverage: Vec<CoverageExpectation>,
}

#[cfg(test)]
impl QualificationFixture {
    fn new() -> Self {
        let mut left = qualification_values(0);
        let mut right = qualification_values(1);
        let mut coverage = Vec::new();
        plant_boundary_coverage(&mut left, &mut coverage);

        // Right-U packing keeps the exact native add-then-half expression.
        // The algebraically equivalent half-then-add expression is bitwise
        // identical for every finite binary32 value, so this fixture instead
        // discriminates the meaningful missing-offset packing fault.
        right.gate[0] = -0.0;
        right.preimage[0][0] = f32::from_bits(0xbf7f_ffff);

        Self {
            preimage: super::super::LensPair {
                a: PreimageMap::new(left.preimage).unwrap(),
                b: PreimageMap::new(right.preimage).unwrap(),
            },
            base: super::super::LensPair {
                a: BaseMap::new(left.base).unwrap(),
                b: BaseMap::new(right.base).unwrap(),
            },
            flow: super::super::LensPair {
                a: FlowMap::new(left.flow).unwrap(),
                b: FlowMap::new(right.flow).unwrap(),
            },
            gate: super::super::LensPair {
                a: GateMap::new(left.gate).unwrap(),
                b: GateMap::new(right.gate).unwrap(),
            },
            coordinate: super::super::LensPair {
                a: CoordinateMap::new(left.coordinate).unwrap(),
                b: CoordinateMap::new(right.coordinate).unwrap(),
            },
            coverage,
        }
    }

    fn cpu_inputs(&self) -> BilateralInputs<'_> {
        BilateralInputs {
            b_to_a: SideInputs {
                preimage: &self.preimage.a,
                base: &self.base.a,
                flow: &self.flow.a,
                gate: &self.gate.a,
                coordinate: &self.coordinate.a,
            },
            a_to_b: SideInputs {
                preimage: &self.preimage.b,
                base: &self.base.b,
                flow: &self.flow.b,
                gate: &self.gate.b,
                coordinate: &self.coordinate.b,
            },
        }
    }

    fn resident_operands(
        &self,
        context: OneXsGpuContext,
        frame: FrameStamp,
    ) -> TestResidentOperands {
        TestResidentOperands::new(context, frame, &pack_dynamic_inputs(self.cpu_inputs()))
    }

    fn gpu_statics(&self, context: OneXsGpuContext) -> Arc<GpuFinalMapStatics> {
        GpuFinalMapStatics::from_words(
            context,
            &pack_static_inputs(self.cpu_inputs()),
            AlphaMap::new(vec![0.5; MAP_NODES]).unwrap().bytes(),
        )
    }

    fn verify_cpu_coverage(&self) {
        let left =
            map_patch::classify(&self.base.a, &self.flow.a, &self.gate.a, &self.coordinate.a);
        for expected in &self.coverage {
            let actual = match left[expected.node] {
                map_patch::Action::KeepGate => CoverageBranch::KeepGate,
                map_patch::Action::KeepLookup => match expected.branch {
                    CoverageBranch::KeepTap | CoverageBranch::KeepFinal => expected.branch,
                    other => other,
                },
                map_patch::Action::Store(_) => CoverageBranch::Store,
            };
            assert_eq!(
                actual, expected.branch,
                "qualification case {} did not exercise its declared CPU branch",
                expected.name
            );
        }
    }

    fn verify_gpu_actions(&self, actions: &[u32]) -> Result<(), QualificationError> {
        assert_eq!(actions.len(), MAP_NODES * 2);
        for expected in &self.coverage {
            let actual = actions[expected.node * 2];
            let wanted = expected.branch.gpu_code();
            if actual != wanted {
                return Err(QualificationError {
                    node: expected.node,
                    component: 4,
                    actual,
                    expected: wanted,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
struct RawSide {
    preimage: Vec<[f32; 2]>,
    base: Vec<[f32; 2]>,
    flow: Vec<[f32; 2]>,
    gate: Vec<f32>,
    coordinate: Vec<[f32; 2]>,
}

#[cfg(test)]
struct CoverageExpectation {
    name: &'static str,
    node: usize,
    branch: CoverageBranch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(test)]
enum CoverageBranch {
    KeepGate,
    KeepTap,
    KeepFinal,
    Store,
}

#[cfg(test)]
impl CoverageBranch {
    const fn gpu_code(self) -> u32 {
        match self {
            Self::KeepGate => 1,
            Self::KeepTap => 2,
            Self::KeepFinal => 3,
            Self::Store => 4,
        }
    }
}

#[cfg(test)]
fn qualification_values(seed: u32) -> RawSide {
    let preimage = (0..map_patch::OUTPUT_NODES)
        .map(|node| {
            [
                f32::from_bits(0x3f00_0000 + ((node as u32 + seed * 17) & 0xffff)),
                if node.is_multiple_of(31) {
                    -0.0
                } else {
                    f32::from_bits(0x3e80_0000 + ((node as u32 + seed * 29) & 0xffff))
                },
            ]
        })
        .collect::<Vec<_>>();
    let base = (0..map_patch::RETAINED_NODES)
        .map(|node| {
            let row = node / super::super::COLS;
            let col = node % super::super::COLS;
            [
                0.125 + col as f32 * 0.0078125 + seed as f32 * 0.000_976_562_5,
                0.25 + (row % 97) as f32 * 0.00390625 + seed as f32 * 0.001_953_125,
            ]
        })
        .collect::<Vec<_>>();
    let flow = (0..map_patch::RETAINED_NODES)
        .map(|node| {
            let signed = (node as i32 % 23) - 11;
            [
                signed as f32 * 0.03125 + seed as f32 * 0.0625,
                ((node as i32 % 19) - 9) as f32 * -0.015625 - seed as f32 * 0.03125,
            ]
        })
        .collect::<Vec<_>>();
    let gate = (0..map_patch::OUTPUT_NODES)
        .map(|node| match node % 5 {
            0 => 0.125,
            1 => 0.25,
            2 => 0.5,
            3 => 0.75,
            _ => f32::from_bits(0x3f00_0001),
        })
        .collect::<Vec<_>>();
    let coordinate = (0..map_patch::OUTPUT_NODES)
        .map(|node| {
            let col = ((node * 37 + seed as usize * 11) % (super::super::COLS - 2)) as f32;
            let row = ((node * 53 + seed as usize * 101) % (super::super::ROWS - 2)) as f32;
            [col + 0.371_093_75, row + 0.613_281_25]
        })
        .collect::<Vec<_>>();
    RawSide {
        preimage,
        base,
        flow,
        gate,
        coordinate,
    }
}

#[cfg(test)]
fn plant_boundary_coverage(side: &mut RawSide, coverage: &mut Vec<CoverageExpectation>) {
    let epsilon = map_patch::POSITIVE_EPSILON;
    let below = f32::from_bits(epsilon.to_bits() - 1);
    let above = f32::from_bits(epsilon.to_bits() + 1);
    let mut node = 8;
    let mut row = 40;

    side.gate[0] = 0.0;
    side.gate[1] = 1.0;
    side.gate[2] = f32::from_bits(0x7fc0_1234);
    coverage.extend([
        CoverageExpectation {
            name: "gate zero equality",
            node: 0,
            branch: CoverageBranch::KeepGate,
        },
        CoverageExpectation {
            name: "gate one equality",
            node: 1,
            branch: CoverageBranch::KeepGate,
        },
        CoverageExpectation {
            name: "unordered gate continues",
            node: 2,
            branch: CoverageBranch::Store,
        },
    ]);

    for tap in 0..4 {
        for (label, value, branch) in [
            ("base tap next-down", below, CoverageBranch::KeepTap),
            ("base tap epsilon", epsilon, CoverageBranch::KeepTap),
            ("base tap next-up", above, CoverageBranch::Store),
        ] {
            let mut taps = [[0.25, 0.5]; 4];
            taps[tap][0] = value;
            plant_quad(side, node, row, 4, [0.371_093_75, 0.613_281_25], taps);
            coverage.push(CoverageExpectation {
                name: label,
                node,
                branch,
            });
            node += 1;
            row += 3;
        }
    }

    // Every U tap must itself pass the strict tap guard. These fixed
    // non-dyadic weight/operand witnesses make the accumulated final U land
    // exactly one bit below epsilon, at epsilon, and one bit above it.
    for (label, fraction, u_taps, branch) in [
        (
            "final U next-down",
            [f32::from_bits(0x3f7f_9e1c), f32::from_bits(0x3e2a_3b5b)],
            [0x322b_cc7f, 0x322b_cc78, 0x322b_cc7f, 0x322b_cc78],
            CoverageBranch::KeepFinal,
        ),
        (
            "final U epsilon",
            [f32::from_bits(0x3df1_9c2d), f32::from_bits(0x3f7d_eb4e)],
            [0x322b_cc78, 0x322b_cc7c, 0x322b_cc78, 0x322b_cc78],
            CoverageBranch::KeepFinal,
        ),
        (
            "final U next-up",
            [0.0, 0.0],
            [0x322b_cc78; 4],
            CoverageBranch::Store,
        ),
    ] {
        let taps = u_taps.map(|bits| [f32::from_bits(bits), 0.5]);
        plant_quad(side, node, row, 4, fraction, taps);
        coverage.push(CoverageExpectation {
            name: label,
            node,
            branch,
        });
        node += 1;
        row += 3;
    }
    for (label, value, branch) in [
        ("final V next-down", below, CoverageBranch::KeepFinal),
        ("final V epsilon", epsilon, CoverageBranch::KeepFinal),
        ("final V next-up", above, CoverageBranch::Store),
    ] {
        plant_quad(side, node, row, 4, [0.0, 0.0], [[0.5, value]; 4]);
        coverage.push(CoverageExpectation {
            name: label,
            node,
            branch,
        });
        node += 1;
        row += 3;
    }

    for tap in 0..4 {
        let mut taps = [[0.25, 0.5]; 4];
        taps[tap][0] = f32::from_bits(0x7fc0_2000 + tap as u32);
        plant_quad(side, node, row, 4, [0.371_093_75, 0.613_281_25], taps);
        coverage.push(CoverageExpectation {
            name: "unordered base U passes tap guard and fails final guard",
            node,
            branch: CoverageBranch::KeepFinal,
        });
        node += 1;
        row += 3;
    }
    for tap in 0..4 {
        let mut taps = [[0.25, 0.5]; 4];
        taps[tap][1] = f32::from_bits(0x7fc0_3000 + tap as u32);
        plant_quad(side, node, row, 4, [0.371_093_75, 0.613_281_25], taps);
        coverage.push(CoverageExpectation {
            name: "unordered base V fails final guard",
            node,
            branch: CoverageBranch::KeepFinal,
        });
        node += 1;
        row += 3;
    }

    for (name, taps, branch) in [
        (
            "negative U is retained at the tap guard",
            [[-0.25, 0.5]; 4],
            CoverageBranch::KeepTap,
        ),
        (
            "negative final V",
            [[0.5, -0.25]; 4],
            CoverageBranch::KeepFinal,
        ),
    ] {
        plant_quad(side, node, row, 4, [0.0, 0.0], taps);
        coverage.push(CoverageExpectation { name, node, branch });
        node += 1;
        row += 3;
    }

    // Non-dyadic column weights and very different positive operands expose
    // the recovered top-right-first FMA association directly in output U.
    plant_quad(
        side,
        node,
        row,
        4,
        [f32::from_bits(0x3ec9_34ab), 0.0],
        [
            [f32::from_bits(0x7e70_2a51), 0.5],
            [f32::from_bits(0x7ae6_5f7c), 0.5],
            [0.25, 0.5],
            [0.25, 0.5],
        ],
    );
    coverage.push(CoverageExpectation {
        name: "non-dyadic TR-first FMA association",
        node,
        branch: CoverageBranch::Store,
    });
    node += 1;
    row += 3;

    // All four weights are non-dyadic and every tap differs, binding the tap
    // construction associations as well as both-axis component order.
    plant_quad(
        side,
        node,
        row,
        4,
        [f32::from_bits(0x3f64_3bff), f32::from_bits(0x3ba5_cb2c)],
        [
            [f32::from_bits(0x5a02_651a), 0.25],
            [f32::from_bits(0x4193_f8fc), 0.75],
            [f32::from_bits(0x3f5a_b23a), 1.125],
            [f32::from_bits(0x6215_656c), 1.75],
        ],
    );
    coverage.push(CoverageExpectation {
        name: "non-dyadic two-axis tap weights",
        node,
        branch: CoverageBranch::Store,
    });
    node += 1;

    for (name, coordinate) in [
        ("column below range", [-100.25, 200.25]),
        ("column above range", [1000.25, 203.25]),
        ("row below range", [10.25, -100.25]),
        ("row above range", [13.25, 2000.25]),
        (
            "unordered column clamp",
            [f32::from_bits(0x7fc0_4000), 206.25],
        ),
        ("unordered row clamp", [16.25, f32::from_bits(0x7fc0_4001)]),
    ] {
        side.gate[node] = 0.5;
        side.coordinate[node] = coordinate;
        coverage.push(CoverageExpectation {
            name,
            node,
            branch: CoverageBranch::Store,
        });
        node += 1;
    }
}

#[cfg(test)]
fn plant_quad(
    side: &mut RawSide,
    node: usize,
    row: usize,
    col: usize,
    fraction: [f32; 2],
    taps: [[f32; 2]; 4],
) {
    let indices = [
        row * super::super::COLS + col,
        row * super::super::COLS + col + 1,
        (row + 1) * super::super::COLS + col,
        (row + 1) * super::super::COLS + col + 1,
    ];
    side.gate[node] = 0.5;
    side.coordinate[node] = [col as f32 + fraction[0], row as f32 + fraction[1]];
    side.preimage[node] = [-10.0 - node as f32, -20.0 - node as f32];
    for (index, value) in indices.into_iter().zip(taps) {
        side.base[index] = value;
        side.flow[index] = [0.0, 0.0];
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll, Wake, Waker};

    use super::*;

    #[test]
    fn display_map_snapshot_does_not_retain_production_carrier() {
        let source = include_str!("map_patch_gpu.rs");
        let fields = source
            .split_once("pub(super) struct MapSnapshot")
            .unwrap()
            .1
            .split_once("impl MapSnapshot")
            .unwrap()
            .0;
        assert!(fields.contains("read: wgpu::BindGroup"));
        assert!(fields.contains("fusion_read: Option<wgpu::BindGroup>"));
        for forbidden in [
            "carrier",
            "statics",
            "GpuFusionFrame",
            "ImportedOneXsPicture",
        ] {
            assert!(
                !fields.contains(forbidden),
                "display map snapshot retained {forbidden}"
            );
        }
    }

    fn diagnostic_fixture(
        fixture: &QualificationFixture,
        frame: FrameStamp,
    ) -> DiagnosticFinalInputs {
        let inputs = fixture.cpu_inputs();
        let mut words = Vec::with_capacity(INPUT_WORDS);
        for side in [inputs.b_to_a, inputs.a_to_b] {
            append_dynamic_side(&mut words, side);
            append_static_side(&mut words, side.gate.values(), side.coordinate.values());
        }
        DiagnosticFinalInputs::from_words(frame, words).unwrap()
    }

    #[test]
    fn carried_public_flow_keeps_every_current_non_flow_input() {
        let fixture = QualificationFixture::new();
        let current = diagnostic_fixture(
            &fixture,
            FrameStamp::for_test(10, std::time::Duration::ZERO, None),
        );

        let age_zero = current.materialize_with_public_from(&current).unwrap();
        let expected_age_zero =
            PackedMap::new(map_patch::materialize(fixture.cpu_inputs()).packed).unwrap();
        assert_eq!(
            age_zero, expected_age_zero,
            "age-zero recomposition changed the map"
        );

        let before = current.words.clone();
        let mut correction_words = current.words.clone();
        let flow_offset = PREIMAGE_WORDS + BASE_WORDS;
        for side in 0..2 {
            let start = side * SIDE_WORDS;
            correction_words[start] ^= 1;
            correction_words[start + PREIMAGE_WORDS] ^= 1;
            correction_words[start + DYNAMIC_SIDE_WORDS] = 0.0_f32.to_bits();
            correction_words[start + DYNAMIC_SIDE_WORDS + GATE_WORDS] ^= 1;
            let flow = start + flow_offset;
            for (component, word) in correction_words[flow..flow + FLOW_WORDS]
                .iter_mut()
                .enumerate()
            {
                *word = if component.is_multiple_of(2) {
                    0.25_f32.to_bits()
                } else {
                    (-0.125_f32).to_bits()
                };
            }
        }
        let correction = DiagnosticFinalInputs::from_words(
            FrameStamp::for_test(9, std::time::Duration::ZERO, None),
            correction_words,
        )
        .unwrap();

        let carried = current.materialize_with_public_from(&correction).unwrap();
        let mut expected_words = current.words.clone();
        for (side, flow) in [
            correction.lens_a_public.as_slice(),
            correction.lens_b_public.as_slice(),
        ]
        .into_iter()
        .enumerate()
        {
            let start = side * SIDE_WORDS + flow_offset;
            expected_words[start..start + FLOW_WORDS].copy_from_slice(flow);
        }
        let expected_carried = PackedMap::new(materialize_words(&expected_words).unwrap()).unwrap();
        assert_eq!(carried, expected_carried);
        assert_ne!(carried, age_zero, "changed carried flow was inert");
        assert_eq!(
            current.words, before,
            "recomposition mutated current inputs"
        );
    }

    #[test]
    fn qualification_fixture_covers_declared_cpu_branches_and_real_association() {
        QualificationFixture::new().verify_cpu_coverage();
        let left = f32::from_bits(0x7e70_2a51);
        let right = f32::from_bits(0x7ae6_5f7c);
        let col = f32::from_bits(0x3ec9_34ab);
        let left_weight = 1.0 - col;
        let right_weight = 1.0 - left_weight;
        assert_eq!(
            left.mul_add(left_weight, right * right_weight).to_bits(),
            0x7e12_7e10
        );
        assert_eq!(
            right.mul_add(right_weight, left * left_weight).to_bits(),
            0x7e12_7e0f
        );
    }

    #[test]
    fn dynamic_and_static_input_ranges_are_exact_and_disjoint() {
        assert_eq!(DYNAMIC_SIDE_WORDS, PREIMAGE_WORDS + BASE_WORDS + FLOW_WORDS);
        assert_eq!(STATIC_SIDE_WORDS, GATE_WORDS + COORDINATE_WORDS);
        assert_eq!(SIDE_WORDS, DYNAMIC_SIDE_WORDS + STATIC_SIDE_WORDS);
        for side in 0..2 {
            let start = side * SIDE_WORDS;
            let dynamic = start..start + DYNAMIC_SIDE_WORDS;
            let statics = dynamic.end..start + SIDE_WORDS;
            assert_eq!(dynamic.end, statics.start);
            assert!(dynamic.end <= statics.start);
            assert_eq!(statics.len(), STATIC_SIDE_WORDS);
        }
        assert_eq!(INPUT_WORDS, 2 * SIDE_WORDS);
    }

    #[test]
    fn generated_hint_validity_tags_decode_exact_semantics_and_reject_old_offsets() {
        assert_eq!(decode_validity(u32::MAX), ResidentValidity::Success);
        assert!(matches!(
            decode_validity(0),
            ResidentValidity::Seed(SeedError::NonFiniteCenter { .. })
        ));
        assert!(matches!(
            decode_validity(PIS_VALIDITY_CODES - 1),
            ResidentValidity::Seed(SeedError::NonFiniteCenter { .. })
        ));

        for (level, rows, cols) in [
            (GeneratedHintLevel::One, L1_DENSE_ROWS, L1_DENSE_COLS),
            (GeneratedHintLevel::Two, L2_DENSE_ROWS, L2_DENSE_COLS),
        ] {
            for direction in [
                crate::flow::one_xs::Direction::AtoB,
                crate::flow::one_xs::Direction::BtoA,
            ] {
                for component in [GeneratedHintComponent::Dcol, GeneratedHintComponent::Drow] {
                    for site in [0, rows * cols - 1] {
                        let word =
                            generated_hint_validity_word(level, direction, component, site as u32)
                                .expect("in-range generated hint site");
                        let dense_row = site / cols;
                        let dense_col = site % cols;
                        let decoded = decode_validity(word);
                        assert_eq!(
                            decoded,
                            ResidentValidity::GeneratedHint {
                                direction,
                                level,
                                component,
                                dense_row,
                                dense_col,
                            }
                        );
                        assert_eq!(
                            decoded.to_string(),
                            format!(
                                "ONE X2 {direction} generated {level} {component} successor hint at dense row {dense_row} column {dense_col} is not finite"
                            )
                        );
                    }
                    assert_eq!(
                        generated_hint_validity_word(
                            level,
                            direction,
                            component,
                            (rows * cols) as u32,
                        ),
                        None
                    );
                }
            }
        }

        let old_untyped_offset = PIS_VALIDITY_CODES;
        assert_eq!(
            decode_validity(old_untyped_offset),
            ResidentValidity::InvalidStatus(old_untyped_offset)
        );
        assert_eq!(
            decode_validity(old_untyped_offset).to_string(),
            "ONE X2 GPU resident validity returned unknown status word 0x00001640"
        );
        assert_eq!(
            decode_validity(PIS_VALIDITY_CODES | 1 << 20),
            ResidentValidity::InvalidStatus(PIS_VALIDITY_CODES | 1 << 20)
        );
        for bit in 17..=30 {
            let word = GENERATED_HINT_FAILURE_TAG | 1 << bit;
            assert_eq!(
                decode_validity(word),
                ResidentValidity::InvalidStatus(word),
                "reserved bit {bit} was accepted"
            );
        }
        for (level_bit, first_invalid_site) in [
            (0, (L1_DENSE_ROWS * L1_DENSE_COLS) as u32),
            (
                GENERATED_HINT_LEVEL_BIT,
                (L2_DENSE_ROWS * L2_DENSE_COLS) as u32,
            ),
        ] {
            let word = GENERATED_HINT_FAILURE_TAG | level_bit | first_invalid_site;
            assert_eq!(
                decode_validity(word),
                ResidentValidity::InvalidStatus(word),
                "out-of-range generated site was accepted"
            );
        }

        let first_generated = generated_hint_validity_word(
            GeneratedHintLevel::One,
            crate::flow::one_xs::Direction::AtoB,
            GeneratedHintComponent::Dcol,
            0,
        )
        .unwrap();
        assert!(
            PIS_VALIDITY_CODES - 1 < first_generated,
            "atomicMin must retain a PIS refusal over a generated-hint refusal"
        );
    }

    #[test]
    fn calibration_resources_upload_exact_gate_coordinate_and_alpha_bits() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(std::env::var("KJERAG_REQUIRE_GPU").is_err(), "{why}");
                eprintln!("skipping calibration-static GPU upload: {why}");
                return;
            }
        };
        let context = OneXsGpuContext::new(&device, &queue);
        let resources = OneXsResources::new(&crate::projection::tests::one_xs_lenses()).unwrap();
        let materializer = GpuMapMaterializer::new(context, &resources)
            .unwrap_or_else(|error| panic!("GPU final-map statics failed on {adapter}: {error}"));

        assert_eq!(
            materializer.statics.gate_coordinate.size(),
            STATIC_INPUT_BYTES
        );
        assert_eq!(materializer.statics.alpha.size(), ALPHA_BYTES as u64);
        let actual_static = readback_u32(
            &device,
            &queue,
            &materializer.statics.gate_coordinate,
            STATIC_INPUT_BYTES,
        )
        .unwrap();
        assert_eq!(actual_static, pack_static_resources(&resources));
        let actual_alpha = readback_u32(
            &device,
            &queue,
            &materializer.statics.alpha,
            ALPHA_BYTES as u64,
        )
        .unwrap();
        assert_eq!(
            actual_alpha,
            resources
                .alpha()
                .nodes()
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn static_gate_mutation_changes_output_without_entering_frame_operands() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(std::env::var("KJERAG_REQUIRE_GPU").is_err(), "{why}");
                eprintln!("skipping final-map static mutation test: {why}");
                return;
            }
        };
        let context = OneXsGpuContext::new(&device, &queue);
        let fixture = QualificationFixture::new();
        let baseline = GpuMapMaterializer::for_qualification(context.clone(), SHADER, &fixture)
            .unwrap_or_else(|error| panic!("baseline statics failed on {adapter}: {error}"));
        let mut changed_words = pack_static_inputs(fixture.cpu_inputs());
        let node = 2;
        changed_words[node] = 0.0_f32.to_bits();
        let changed_statics = GpuFinalMapStatics::from_words(
            context.clone(),
            &changed_words,
            AlphaMap::new(vec![0.5; MAP_NODES]).unwrap().bytes(),
        );
        let changed = GpuMapMaterializer::from_shader(context.clone(), changed_statics, SHADER)
            .unwrap_or_else(|error| panic!("changed statics failed on {adapter}: {error}"));
        let frame = FrameStamp::for_test(12, std::time::Duration::ZERO, None);

        let baseline_map = expect_ready(
            baseline
                .materialize(fixture.resident_operands(context.clone(), frame.clone()))
                .unwrap(),
        )
        .unwrap()
        .diagnostic_readback()
        .unwrap()
        .packed;
        let changed_map = expect_ready(
            changed
                .materialize(fixture.resident_operands(context, frame))
                .unwrap(),
        )
        .unwrap()
        .diagnostic_readback()
        .unwrap()
        .packed;

        assert_ne!(
            baseline_map.nodes()[node].map(f32::to_bits),
            changed_map.nodes()[node].map(f32::to_bits),
            "the materializer ignored its sealed gate resource"
        );
        assert_eq!(
            [
                changed_map.nodes()[node][0].to_bits(),
                changed_map.nodes()[node][1].to_bits(),
            ],
            [
                (fixture.preimage.a.values()[node][0] * 0.5_f32).to_bits(),
                fixture.preimage.a.values()[node][1].to_bits(),
            ],
            "zero gate must retain and exactly pack the lens-A preimage"
        );
    }

    #[test]
    fn completed_operands_retain_frame_and_alpha_as_one_resident_binding() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping exact GPU final-map twin: {why}");
                return;
            }
        };
        let context = OneXsGpuContext::new(&device, &queue);
        let fixture = QualificationFixture::new();
        let pipeline = GpuMapMaterializer::for_qualification(context.clone(), SHADER, &fixture)
            .expect("same-context qualification statics");
        pipeline.qualify_fixture(&fixture).unwrap_or_else(|error| {
            panic!("GPU final-map qualification failed on {adapter}: {error}")
        });
        let statics = Arc::downgrade(&pipeline.statics);
        let frame = FrameStamp::for_test(0, std::time::Duration::ZERO, None);
        let pending = pipeline
            .materialize(fixture.resident_operands(context.clone(), frame.clone()))
            .unwrap();
        assert!(Arc::ptr_eq(&pending.statics, &pipeline.statics));
        drop(pipeline);
        assert!(statics.upgrade().is_some());
        let gpu = expect_ready(pending).unwrap();
        assert_eq!(gpu.frame, frame);
        assert_eq!(gpu.packed.size(), PACKED_BYTES as u64);
        assert!(statics.upgrade().is_some());
        let read = gpu.diagnostic_readback().unwrap();
        assert_eq!(read.frame, frame);
        let alpha = readback_u32(&device, &queue, &gpu.statics.alpha, ALPHA_BYTES as u64).unwrap();
        assert_eq!(alpha, vec![0.5_f32.to_bits(); MAP_NODES]);
        let layout = scene_layout(&device);
        let binding = gpu.bind_for_scene(&context, &frame, &layout).unwrap();
        assert_eq!(binding.frame(), &frame);
        assert_eq!(binding._resident.statics.alpha.size(), ALPHA_BYTES as u64);
        assert!(statics.upgrade().is_some());
        let _ = binding.read();
        drop(binding);
        assert!(statics.upgrade().is_none());
    }

    #[test]
    fn invalid_resident_word_refuses_with_exact_identity_and_drops_upstream() {
        let (device, queue, _) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(std::env::var("KJERAG_REQUIRE_GPU").is_err(), "{why}");
                return;
            }
        };
        let context = OneXsGpuContext::new(&device, &queue);
        let fixture = QualificationFixture::new();
        let pipeline = GpuMapMaterializer::for_qualification(context.clone(), SHADER, &fixture)
            .expect("same-context qualification statics");
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_statics_strong_count =
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));
        let statics = Arc::downgrade(&pipeline.statics);
        let status = 3 * L1_PATCHES as u32 + 10;
        let operands = TestResidentOperands::with_validity_and_witness(
            context,
            FrameStamp::for_test(9, std::time::Duration::ZERO, None),
            &pack_dynamic_inputs(fixture.cpu_inputs()),
            status,
            Some(TestDropWitness::ordered(
                dropped.clone(),
                statics.clone(),
                observed_statics_strong_count.clone(),
            )),
        );
        let pending = pipeline.materialize(operands).unwrap();
        assert_eq!(pending.validity.size(), VALIDITY_BYTES);
        assert_eq!(dropped.load(std::sync::atomic::Ordering::SeqCst), 0);
        drop(pipeline);
        let error = expect_refusal(pending);
        assert_eq!(
            error.to_string(),
            "ONE X2 B-to-A level-one initial drow at patch row 1 column 2, dense row 7 column 10, is not finite"
        );
        assert_eq!(dropped.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            observed_statics_strong_count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "validity refusal released statics before the inherited submission carrier"
        );
        assert!(statics.upgrade().is_none());
    }

    #[test]
    fn submit_refusal_drops_the_upstream_owner_and_keeps_raw_error_text() {
        let (device, queue, _) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(std::env::var("KJERAG_REQUIRE_GPU").is_err(), "{why}");
                return;
            }
        };
        let context = OneXsGpuContext::new(&device, &queue);
        let fixture = QualificationFixture::new();
        let pipeline = GpuMapMaterializer::for_qualification(context.clone(), SHADER, &fixture)
            .expect("same-context qualification statics");
        let statics = Arc::downgrade(&pipeline.statics);
        let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_statics_strong_count =
            Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));
        let operands = TestResidentOperands::with_submit_failure(
            context,
            FrameStamp::for_test(11, std::time::Duration::ZERO, None),
            &pack_dynamic_inputs(fixture.cpu_inputs()),
            "injected raw final-map submit refusal",
            TestDropWitness::ordered(
                dropped.clone(),
                statics.clone(),
                observed_statics_strong_count.clone(),
            ),
        );

        let error = match pipeline.materialize(operands) {
            Err(error) => error,
            Ok(_) => panic!("injected final-map submit refusal was accepted"),
        };
        assert_eq!(error.to_string(), "injected raw final-map submit refusal");
        assert_eq!(dropped.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            observed_statics_strong_count.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "submit refusal released statics before the inherited submission carrier"
        );
        assert!(statics.upgrade().is_some());
        drop(pipeline);
        assert!(statics.upgrade().is_none());
    }

    #[test]
    fn mapping_failure_keeps_raw_underlying_text() {
        let (device, queue, _) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(std::env::var("KJERAG_REQUIRE_GPU").is_err(), "{why}");
                return;
            }
        };
        let context = OneXsGpuContext::new(&device, &queue);
        let fixture = QualificationFixture::new();
        let pipeline = GpuMapMaterializer::for_qualification(context.clone(), SHADER, &fixture)
            .expect("same-context qualification statics");
        let mut pending = pipeline
            .materialize(fixture.resident_operands(
                context,
                FrameStamp::for_test(10, std::time::Duration::ZERO, None),
            ))
            .unwrap();
        pending.inject_mapping_failure("injected raw GPU map failure");
        let error = match pending.poll() {
            Err(error) => error,
            Ok(_) => panic!("injected mapping failure was accepted"),
        };
        assert_eq!(error.to_string(), "injected raw GPU map failure");
    }

    #[test]
    fn frame_and_structural_gpu_context_provenance_cannot_be_mixed() {
        let (device, queue, foreign_device, foreign_queue, _) = match gpu_pair() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(std::env::var("KJERAG_REQUIRE_GPU").is_err(), "{why}");
                return;
            }
        };
        let context = OneXsGpuContext::new(&device, &queue);
        let fixture = QualificationFixture::new();
        let pipeline = GpuMapMaterializer::for_qualification(context.clone(), SHADER, &fixture)
            .expect("same-context qualification statics");
        pipeline.qualify_fixture(&fixture).unwrap();
        let frame = FrameStamp::for_test(7, std::time::Duration::ZERO, None);
        let foreign_frame = FrameStamp::for_test(7, std::time::Duration::ZERO, None);
        let layout = scene_layout(&device);

        let foreign_context = OneXsGpuContext::new(&foreign_device, &foreign_queue);

        let foreign_statics = fixture.gpu_statics(foreign_context.clone());
        let error = match GpuMapMaterializer::from_shader(context.clone(), foreign_statics, SHADER)
        {
            Err(error) => error,
            Ok(_) => panic!("foreign static-resource context was accepted"),
        };
        assert!(
            error
                .to_string()
                .contains("crossed a different device or queue")
        );

        let gpu = expect_ready(
            pipeline
                .materialize(fixture.resident_operands(context.clone(), frame.clone()))
                .unwrap(),
        )
        .unwrap();
        match gpu.bind_for_scene(&foreign_context, &frame, &layout) {
            Err(error) => assert_eq!(error, BindingError::Context),
            Ok(_) => panic!("foreign GPU context was accepted"),
        }
        let gpu = expect_ready(
            pipeline
                .materialize(fixture.resident_operands(context.clone(), frame.clone()))
                .unwrap(),
        )
        .unwrap();
        match gpu.bind_for_scene(&context, &foreign_frame, &layout) {
            Err(error) => assert_eq!(error, BindingError::Frame),
            Ok(_) => panic!("foreign frame was accepted"),
        }

        let operands = fixture.resident_operands(foreign_context, frame);
        let error = match pipeline.materialize(operands) {
            Err(error) => error,
            Ok(_) => panic!("foreign resident operand context was accepted"),
        };
        assert!(
            error
                .to_string()
                .contains("crossed a different device or queue")
        );
    }

    #[test]
    fn qualification_refuses_semantic_and_association_mutations() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping GPU final-map mutation refusal: {why}");
                return;
            }
        };
        let context = OneXsGpuContext::new(&device, &queue);
        GpuMapMaterializer::qualify_shader(context.clone(), SHADER).unwrap_or_else(|error| {
            panic!("baseline GPU final-map qualification failed on {adapter}: {error}")
        });
        let mutations = [
            ("gate equality", "gate <= 0.0", "gate < 0.0"),
            ("upper gate equality", "gate >= 1.0", "gate > 1.0"),
            (
                "unordered gate",
                "if gate <= 0.0 || gate >= 1.0 { return Materialized(retained, 1u); }",
                "if is_nan(gate) || gate <= 0.0 || gate >= 1.0 { return Materialized(retained, 1u); }",
            ),
            (
                "flow axes",
                "let q = fma(sampled_flow, vec2<f32>(weight), c);",
                "let q = fma(sampled_flow.yx, vec2<f32>(weight), c);",
            ),
            (
                "base axes",
                "let uv = interpolate_base(side, taps);",
                "let uv = interpolate_base(side, taps).yx;",
            ),
            (
                "lens ownership",
                "let left_result = materialize_side(0u, node);",
                "let left_result = materialize_side(1u, node);",
            ),
            (
                "keep preimage",
                "if gate <= 0.0 || gate >= 1.0 { return Materialized(retained, 1u); }",
                "if gate <= 0.0 || gate >= 1.0 { return Materialized(vec2<f32>(-1.0), 1u); }",
            ),
            (
                "top-left tap epsilon equality",
                "base_value(side, taps.indices.x).x <= positive_epsilon()",
                "base_value(side, taps.indices.x).x < positive_epsilon()",
            ),
            (
                "top-right tap epsilon equality",
                "base_value(side, taps.indices.y).x <= positive_epsilon()",
                "base_value(side, taps.indices.y).x < positive_epsilon()",
            ),
            (
                "bottom-left tap epsilon equality",
                "base_value(side, taps.indices.z).x <= positive_epsilon()",
                "base_value(side, taps.indices.z).x < positive_epsilon()",
            ),
            (
                "bottom-right tap epsilon equality",
                "base_value(side, taps.indices.w).x <= positive_epsilon()",
                "base_value(side, taps.indices.w).x < positive_epsilon()",
            ),
            (
                "unordered base tap",
                "if base_value(side, taps.indices.x).x <= positive_epsilon() ||",
                "if is_nan(base_value(side, taps.indices.x).x) ||\n     base_value(side, taps.indices.x).x <= positive_epsilon() ||",
            ),
            (
                "final U epsilon equality",
                "uv.x > positive_epsilon() &&",
                "uv.x >= positive_epsilon() &&",
            ),
            (
                "final V epsilon equality",
                "uv.y > positive_epsilon())",
                "uv.y >= positive_epsilon())",
            ),
            (
                "final component conjunction",
                "uv.x > positive_epsilon() && uv.y > positive_epsilon()",
                "uv.x > positive_epsilon() || uv.y > positive_epsilon()",
            ),
            (
                "unordered clamp semantics",
                "if is_nan(left) { return right; }",
                "if is_nan(left) { return left; }",
            ),
            (
                "upper coordinate clamp",
                "rust_min(c.x, f32(RETAINED_COLS) - 1.0)",
                "rust_min(c.x, f32(RETAINED_COLS) - 2.0)",
            ),
            (
                "tap-weight association",
                "let tmp = (1.0 - col_inverse) - row_inverse;\n  let br = tmp + tl;",
                "let br = (1.0 - col_inverse) - (row_inverse - tl);",
            ),
            (
                "base FMA association",
                "let top_right = base_value(side, taps.indices.y) * taps.weights.y;\n  let with_top_left = fma(base_value(side, taps.indices.x), vec2<f32>(taps.weights.x), top_right);",
                "let top_left = base_value(side, taps.indices.x) * taps.weights.x;\n  let with_top_left = fma(base_value(side, taps.indices.y), vec2<f32>(taps.weights.y), top_left);",
            ),
            (
                "right-lens U offset/packing",
                "let right_plus_one = right.x + 1.0;\n  let right_u = right_plus_one * 0.5;",
                "let right_u = right.x * 0.5;",
            ),
        ];
        for (name, from, to) in mutations {
            let broken = SHADER.replacen(from, to, 1);
            assert_ne!(broken, SHADER, "{name} mutation found no target");
            let error = GpuMapMaterializer::qualify_shader(context.clone(), &broken).expect_err(
                &format!("changed GPU final-map {name} was accepted on {adapter}"),
            );
            assert!(
                error.downcast_ref::<QualificationError>().is_some(),
                "{name} mutation returned the wrong refusal on {adapter}: {error}"
            );
        }
    }

    fn scene_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        let storage = |binding, bytes| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(bytes),
            },
            count: None,
        };
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ONE X2 diagnostic resident Scene binding"),
            entries: &[
                storage(0, PACKED_BYTES as u64),
                storage(1, ALPHA_BYTES as u64),
            ],
        })
    }

    fn gpu() -> Result<(wgpu::Device, wgpu::Queue, String), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .find(|adapter| adapter.get_info().driver.eq_ignore_ascii_case("radv"))
            .ok_or("no RADV Vulkan adapter")?;
        let info = adapter.get_info();
        let name = format!("{} / {} / {}", info.name, info.driver, info.driver_info);
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ONE X2 GPU final-map test"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|error| error.to_string())?;
        eprintln!("GPU final-map qualification adapter: {name}");
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
            .find(|adapter| adapter.get_info().driver.eq_ignore_ascii_case("radv"))
            .ok_or("no RADV Vulkan adapter")?;
        let info = adapter.get_info();
        let name = format!("{} / {} / {}", info.name, info.driver, info.driver_info);
        let descriptor = || wgpu::DeviceDescriptor {
            label: Some("ONE X2 GPU final-map context test"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        };
        let (device, queue) =
            block_on(adapter.request_device(&descriptor())).map_err(|error| error.to_string())?;
        let (foreign_device, foreign_queue) =
            block_on(adapter.request_device(&descriptor())).map_err(|error| error.to_string())?;
        eprintln!("GPU final-map qualification adapter: {name}");
        Ok((device, queue, foreign_device, foreign_queue, name))
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        struct ThreadWake(std::thread::Thread);
        impl Wake for ThreadWake {
            fn wake(self: std::sync::Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(std::sync::Arc::new(ThreadWake(std::thread::current())));
        let mut context = Context::from_waker(&waker);
        let mut future = Box::pin(future);
        loop {
            match Pin::new(&mut future).poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => std::thread::park(),
            }
        }
    }
}
