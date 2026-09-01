//! Resident GPU arithmetic and ownership for the three selected cold tails.
//!
//! Each tail keeps the imported-frame lease, source owner, root reservation,
//! validity allocation and temporal successor together. Cold2 constructs the
//! typed completed-cold checkpoint; ordinary warm execution remains excluded.

use std::num::NonZeroU64;

use super::super::super::geometry_gpu::GpuGeometryFrameOwner;
use super::super::{GpuPreparedTerminal, GpuResidentValidity, words_bytes};
use super::{GpuL1PreparedTerminal, GpuResidentLevelTwoPost};
use crate::Fallible;
use crate::direct_type2::ImportedOneXsPicture;
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
use crate::flow::one_xs::one_xs_belt_gpu::geometry_gpu::temporal_gpu::{
    GpuColdPriorPublicLevelTwo, GpuMotionResidentL2Post,
};
use crate::flow::one_xs::one_xs_belt_gpu::map_patch_gpu::resident;
use crate::flow::one_xs::pis::Level;
use crate::flow::one_xs::scalar::PairSolveStage;

const L1_ROWS: usize = 540;
const L1_COLS: usize = 30;
const L2_ROWS: usize = 270;
const L2_COLS: usize = 15;
const PUB_ROWS: usize = 1080;
const PUB_COLS: usize = 60;
const PATCH_ROWS: usize = 178;
const PATCH_COLS: usize = 8;
const PATCHES: usize = PATCH_ROWS * PATCH_COLS;
const HIST_BINS: usize = 159;
const HIST_WORDS: usize = 2 * PATCHES * HIST_BINS;
const FIFO_WORDS: usize = 2 * PATCHES * 5;
const PUBLIC_WORDS: usize = 2 * PUB_ROWS * PUB_COLS * 2;
const HINT_WORDS: usize = 275_400;
const PATCH_WORDS: usize = 2 * PATCHES * 2;
const L1_WORDS: usize = 2 * L1_ROWS * L1_COLS * 2;
const PACKED_L1_MOTION_WORDS: usize = (L1_ROWS * L1_COLS).div_ceil(4);
const RETAINED_L2_WORDS: usize = 2 * L2_ROWS * L2_COLS * 2;
const HORIZONTAL_WORDS: usize = 2 * L1_ROWS * PUB_COLS * 2;
const HORIZONTAL_PRODUCT_WORDS: usize = 2 * HORIZONTAL_WORDS;

pub(super) struct GpuColdPostL1Pipeline {
    context: OneXsGpuContext,
    layout: wgpu::BindGroupLayout,
    passes: [wgpu::ComputePipeline; 8],
    quantized_values: wgpu::Buffer,
}

impl GpuColdPostL1Pipeline {
    pub(super) fn new(context: OneXsGpuContext) -> Fallible<Self> {
        Self::from_shader(context, SHADER)
    }

    fn from_shader(context: OneXsGpuContext, shader: &str) -> Fallible<Self> {
        let device = context.device().clone();
        let storage = |binding, read_only, words| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(words_bytes(words)),
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ONE X2 resident cold post-L1"),
            entries: &[
                storage(0, true, PATCH_WORDS),
                storage(1, true, 2 * (L1_ROWS * L1_COLS + L2_ROWS * L2_COLS)),
                storage(3, false, HIST_WORDS),
                storage(4, false, FIFO_WORDS),
                storage(7, false, HINT_WORDS),
                storage(9, false, PATCH_WORDS),
                storage(11, false, L1_WORDS),
                storage(12, false, HORIZONTAL_PRODUCT_WORDS),
                storage(13, false, PUBLIC_WORDS),
                storage(14, true, 2 * HIST_BINS),
                storage(16, false, RETAINED_L2_WORDS),
                storage(17, false, 1),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 resident cold post-L1"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 resident cold post-L1"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let make = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ONE X2 resident cold post-L1"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let mut quantized_values = Vec::with_capacity(2 * HIST_BINS);
        for (base, scale) in [
            (f32::from_bits(0x3f7f_fc66), -10.0f32),
            (f32::from_bits(0xbf7f_fc66), 10.0f32),
        ] {
            quantized_values.extend((0..HIST_BINS).map(|bin| {
                let offset = bin as f32 / scale;
                (base + offset).to_bits()
            }));
        }
        let quantized_values = upload_words(
            context.device(),
            context.queue(),
            "ONE X2 exact temporal quantized values",
            &quantized_values,
        );
        Ok(Self {
            context,
            layout,
            passes: [
                make("make_hint_l1"),
                make("make_hint_l2"),
                make("temporal_median"),
                make("resize_horizontal"),
                make("resize_vertical"),
                make("repair_periodic"),
                make("densify_cold"),
                make("make_retained_l2"),
            ],
            quantized_values,
        })
    }
}

/// Arithmetic-only warm continuation. The resident frame owner does not yet
/// construct its input; keeping this beside the cold tail makes the shared
/// shader and ABI one private contract.
#[allow(
    dead_code,
    reason = "qualified prerequisite for the later warm owner join"
)]
pub(super) struct GpuWarmPostL1Pipeline {
    context: OneXsGpuContext,
    layout: wgpu::BindGroupLayout,
    passes: [wgpu::ComputePipeline; 9],
    quantized_values: wgpu::Buffer,
}

pub(super) mod classified_rows {
    pub(in super::super) trait Sealed {}
}

/// Sealed handoff implemented only by the later classifier's exact owned
/// successor-state output. Post-L1 never receives or returns its raw buffer.
#[allow(
    dead_code,
    reason = "the small-row classifier is a parallel prerequisite"
)]
pub(super) trait GpuWarmClassifiedRows: classified_rows::Sealed {}

/// Allocation-identical prior public field carried from the resident
/// predecessor. It has no ordinary from-buffer constructor or accessor.
#[allow(
    dead_code,
    reason = "qualified prerequisite for the later warm owner join"
)]
pub(super) struct GpuWarmPriorPublic {
    buffer: wgpu::Buffer,
}

#[cfg(test)]
impl GpuWarmPriorPublic {
    fn for_qualification(buffer: wgpu::Buffer) -> Self {
        assert_eq!(buffer.size(), words_bytes(PUBLIC_WORDS));
        Self { buffer }
    }
}

#[allow(
    dead_code,
    reason = "qualified prerequisite for the later warm owner join"
)]
struct GpuWarmPostL1Inputs {
    context: OneXsGpuContext,
    terminal: wgpu::Buffer,
    images: wgpu::Buffer,
    prior_histogram: wgpu::Buffer,
    prior_fifo: wgpu::Buffer,
    prior_public: GpuWarmPriorPublic,
    motion_l1: wgpu::Buffer,
    validity: wgpu::Buffer,
}

/// Hints and median are already encoded, but the command encoder has not been
/// submitted. Only a sealed classifier result can resume the same encoder.
#[allow(
    dead_code,
    reason = "qualified prerequisite for the later warm owner join"
)]
#[must_use = "warm post-L1 must receive classified rows before submission"]
pub(super) struct GpuWarmPostL1Paused<'pipeline> {
    pipeline: &'pipeline GpuWarmPostL1Pipeline,
    encoder: wgpu::CommandEncoder,
    bind: wgpu::BindGroup,
    filtered: wgpu::Buffer,
    histogram: wgpu::Buffer,
    fifo: wgpu::Buffer,
    hints: wgpu::Buffer,
    retained_l1: wgpu::Buffer,
    dense_l1: wgpu::Buffer,
    horizontal: wgpu::Buffer,
    public: wgpu::Buffer,
    retained_l2_direction_pixel_vec2: super::RetainedL2DirectionPixelVec2Buffer,
    validity: wgpu::Buffer,
}

#[allow(
    dead_code,
    reason = "qualified prerequisite for the later warm owner join"
)]
#[must_use = "encoded warm post-L1 must remain inside its resident submission owner"]
pub(super) struct GpuWarmPostL1Encoded<C: GpuWarmClassifiedRows> {
    encoder: wgpu::CommandEncoder,
    _bind: wgpu::BindGroup,
    _filtered: wgpu::Buffer,
    _histogram: wgpu::Buffer,
    _fifo: wgpu::Buffer,
    _hints: wgpu::Buffer,
    _retained_l1: wgpu::Buffer,
    _dense_l1: wgpu::Buffer,
    _horizontal: wgpu::Buffer,
    public: wgpu::Buffer,
    retained_l2_direction_pixel_vec2: super::RetainedL2DirectionPixelVec2Buffer,
    validity: wgpu::Buffer,
    classified_rows: C,
}

#[allow(
    dead_code,
    reason = "qualified prerequisite for the later warm owner join"
)]
impl GpuWarmPostL1Pipeline {
    pub(super) fn new(context: OneXsGpuContext) -> Fallible<Self> {
        Self::from_shader(context, SHADER)
    }

    fn from_shader(context: OneXsGpuContext, shader: &str) -> Fallible<Self> {
        let device = context.device().clone();
        let storage = |binding, read_only, words| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(words_bytes(words)),
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ONE X2 resident warm post-L1"),
            entries: &[
                storage(0, true, PATCH_WORDS),
                storage(1, true, 2 * (L1_ROWS * L1_COLS + L2_ROWS * L2_COLS)),
                storage(3, false, HIST_WORDS),
                storage(4, false, FIFO_WORDS),
                storage(5, true, PUBLIC_WORDS),
                storage(6, true, PACKED_L1_MOTION_WORDS),
                storage(7, false, HINT_WORDS),
                storage(9, false, PATCH_WORDS),
                storage(10, false, L1_WORDS),
                storage(11, false, L1_WORDS),
                storage(12, false, HORIZONTAL_PRODUCT_WORDS),
                storage(13, false, PUBLIC_WORDS),
                storage(14, true, 2 * HIST_BINS),
                storage(16, false, RETAINED_L2_WORDS),
                storage(17, false, 1),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 resident warm post-L1"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 resident warm post-L1"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let make = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ONE X2 resident warm post-L1"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let mut quantized_values = Vec::with_capacity(2 * HIST_BINS);
        for (base, scale) in [
            (f32::from_bits(0x3f7f_fc66), -10.0f32),
            (f32::from_bits(0xbf7f_fc66), 10.0f32),
        ] {
            quantized_values.extend((0..HIST_BINS).map(|bin| {
                let offset = bin as f32 / scale;
                (base + offset).to_bits()
            }));
        }
        let quantized_values = upload_words(
            context.device(),
            context.queue(),
            "ONE X2 exact warm temporal quantized values",
            &quantized_values,
        );
        Ok(Self {
            context,
            layout,
            passes: [
                make("make_hint_l1"),
                make("make_hint_l2"),
                make("temporal_median"),
                make("make_retained_l1"),
                make("densify_warm"),
                make("resize_horizontal"),
                make("resize_vertical"),
                make("repair_periodic"),
                make("make_retained_l2"),
            ],
            quantized_values,
        })
    }

    fn encode_until_classification(
        &self,
        inputs: GpuWarmPostL1Inputs,
    ) -> Fallible<GpuWarmPostL1Paused<'_>> {
        self.context.ensure_same(&inputs.context)?;
        for (part, resource, words) in [
            ("paired L1 terminal", &inputs.terminal, PATCH_WORDS),
            (
                "prepared images",
                &inputs.images,
                2 * (L1_ROWS * L1_COLS + L2_ROWS * L2_COLS),
            ),
            (
                "prior temporal histogram",
                &inputs.prior_histogram,
                HIST_WORDS,
            ),
            ("prior temporal FIFO", &inputs.prior_fifo, FIFO_WORDS),
            (
                "prior public flow",
                &inputs.prior_public.buffer,
                PUBLIC_WORDS,
            ),
            (
                "packed L1 motion",
                &inputs.motion_l1,
                PACKED_L1_MOTION_WORDS,
            ),
            ("resident validity", &inputs.validity, 1),
        ] {
            if resource.size() != words_bytes(words) {
                return Err(format!(
                    "ONE X2 warm post-L1 {part} buffer is {} bytes, expected {}",
                    resource.size(),
                    words_bytes(words)
                )
                .into());
            }
        }
        let device = self.context.device();
        let hints = buffer(device, "ONE X2 warm next planar hints", HINT_WORDS);
        let filtered = buffer(device, "ONE X2 warm filtered patches", PATCH_WORDS);
        let histogram = buffer(device, "ONE X2 warm successor histogram", HIST_WORDS);
        let fifo = buffer(device, "ONE X2 warm successor FIFO", FIFO_WORDS);
        let retained_l1 = buffer(device, "ONE X2 warm retained L1", L1_WORDS);
        let dense_l1 = buffer(device, "ONE X2 warm dense L1", L1_WORDS);
        let horizontal = buffer(
            device,
            "ONE X2 warm resize horizontal products",
            HORIZONTAL_PRODUCT_WORDS,
        );
        let public = buffer(device, "ONE X2 warm public flow", PUBLIC_WORDS);
        let retained_l2_direction_pixel_vec2 =
            super::RetainedL2DirectionPixelVec2Buffer::new(buffer(
                device,
                "ONE X2 warm successor retained L2 direction-pixel-vec2",
                RETAINED_L2_WORDS,
            ));
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 resident warm post-L1"),
            layout: &self.layout,
            entries: &[
                entry(0, &inputs.terminal),
                entry(1, &inputs.images),
                entry(3, &histogram),
                entry(4, &fifo),
                entry(5, &inputs.prior_public.buffer),
                entry(6, &inputs.motion_l1),
                entry(7, &hints),
                entry(9, &filtered),
                entry(10, &retained_l1),
                entry(11, &dense_l1),
                entry(12, &horizontal),
                entry(13, &public),
                entry(14, &self.quantized_values),
                entry(16, retained_l2_direction_pixel_vec2.buffer()),
                entry(17, &inputs.validity),
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 resident warm post-L1"),
        });
        encoder.copy_buffer_to_buffer(
            &inputs.prior_histogram,
            0,
            &histogram,
            0,
            words_bytes(HIST_WORDS),
        );
        encoder.copy_buffer_to_buffer(&inputs.prior_fifo, 0, &fifo, 0, words_bytes(FIFO_WORDS));
        encoder.clear_buffer(&hints, 0, None);
        dispatch(&mut encoder, &self.passes[0], &bind, 2 * L1_ROWS * L1_COLS);
        dispatch(&mut encoder, &self.passes[1], &bind, 2 * L2_ROWS * L2_COLS);
        dispatch(&mut encoder, &self.passes[2], &bind, 2 * PATCHES);
        Ok(GpuWarmPostL1Paused {
            pipeline: self,
            encoder,
            bind,
            filtered,
            histogram,
            fifo,
            hints,
            retained_l1,
            dense_l1,
            horizontal,
            public,
            retained_l2_direction_pixel_vec2,
            validity: inputs.validity,
        })
    }
}

#[allow(
    dead_code,
    reason = "qualified prerequisite for the later warm owner join"
)]
impl GpuWarmPostL1Paused<'_> {
    pub(super) fn continue_with<C: GpuWarmClassifiedRows>(
        mut self,
        classified_rows: C,
    ) -> GpuWarmPostL1Encoded<C> {
        for (index, count) in [
            (3, 2 * L1_ROWS * L1_COLS),
            (4, 2 * L1_ROWS * L1_COLS),
            (5, 2 * L1_ROWS * PUB_COLS * 2),
            (6, 2 * PUB_ROWS * PUB_COLS * 2),
            (7, 2 * 5 * PUB_COLS * 2),
            (8, 2 * L2_ROWS * L2_COLS),
        ] {
            dispatch(
                &mut self.encoder,
                &self.pipeline.passes[index],
                &self.bind,
                count,
            );
        }
        GpuWarmPostL1Encoded {
            encoder: self.encoder,
            _bind: self.bind,
            _filtered: self.filtered,
            _histogram: self.histogram,
            _fifo: self.fifo,
            _hints: self.hints,
            _retained_l1: self.retained_l1,
            _dense_l1: self.dense_l1,
            _horizontal: self.horizontal,
            public: self.public,
            retained_l2_direction_pixel_vec2: self.retained_l2_direction_pixel_vec2,
            validity: self.validity,
            classified_rows,
        }
    }
}

fn dispatch(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::ComputePipeline,
    bind: &wgpu::BindGroup,
    count: usize,
) {
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("ONE X2 resident warm post-L1"),
        timestamp_writes: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    pass.dispatch_workgroups((count as u32).div_ceil(64), 1, 1);
}

pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuCold0Terminal<K> {
    terminal: GpuL1PreparedTerminal<
        GpuGeometryFrameOwner<K>,
        GpuMotionResidentL2Post<GpuColdPriorPublicLevelTwo>,
    >,
    controls: super::GpuColdLoopControls,
}

pub(in crate::flow::one_xs::one_xs_belt_gpu) struct ColdAfter0;
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct ColdAfter1;

pub(in crate::flow::one_xs::one_xs_belt_gpu) trait ColdLoopOrdinal {
    const COMPLETED: u8;
    type Next;
}

impl ColdLoopOrdinal for ColdAfter0 {
    const COMPLETED: u8 = 0;
    type Next = ColdAfter1;
}

impl ColdLoopOrdinal for ColdAfter1 {
    const COMPLETED: u8 = 1;
    type Next = GpuCompletedColdMarker;
}

pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuCompletedColdMarker;

#[must_use = "the resident cold loop must resume on its owned next ordinal"]
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuColdLoop<K, O> {
    prepared: Option<super::super::GpuPreparedFrame<GpuGeometryFrameOwner<K>>>,
    validity: Option<GpuResidentValidity>,
    post: Option<GpuMotionResidentL2Post<GpuColdPriorPublicLevelTwo>>,
    controls: super::GpuColdLoopControls,
    _public_scratch: wgpu::Buffer,
    ordinal: std::marker::PhantomData<O>,
}

struct ColdResumeGuard<K> {
    prepared: Option<super::super::GpuPreparedFrame<GpuGeometryFrameOwner<K>>>,
    validity: Option<GpuResidentValidity>,
    post: Option<GpuMotionResidentL2Post<GpuColdPriorPublicLevelTwo>>,
}

#[must_use = "the resident completed-cold checkpoint has not been consumed"]
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuCompletedColdCheckpoint<K> {
    prepared: Option<super::super::GpuPreparedFrame<GpuGeometryFrameOwner<K>>>,
    validity: Option<GpuResidentValidity>,
    post: Option<GpuMotionResidentL2Post<GpuColdPriorPublicLevelTwo>>,
    public: wgpu::Buffer,
}

/// Concrete, nonconstructible final-map operand for the only production
/// Cold2/source-owner combination. It deliberately has no generic parameter.
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct CompletedColdFinalOperands {
    // Carrier first: every error, panic and unwind retires the imported source
    // lease before releasing the root reservation or capture-static owners.
    prepared: super::super::GpuPreparedFrame<GpuGeometryFrameOwner<ImportedOneXsPicture>>,
    validity: GpuResidentValidity,
    post: GpuMotionResidentL2Post<GpuColdPriorPublicLevelTwo>,
    public: wgpu::Buffer,
    context: OneXsGpuContext,
    flight: crate::flow::one_xs::pis::gpu::GpuPisFlight,
}

/// Root-free result of consuming the completed final-map operands. The source
/// is separate only long enough for the parent module to place both pieces in
/// one nonconstructible installed draw.
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct CompletedColdInstallParts {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) source: ImportedOneXsPicture,
    pub(in crate::flow::one_xs::one_xs_belt_gpu) carrier: CompletedColdDrawCarrier,
    pub(in crate::flow::one_xs::one_xs_belt_gpu) candidate:
        crate::flow::one_xs::one_xs_belt_gpu::resident_frame_gpu::GpuResidentCandidate,
}

/// All final-map producer state needed by the installed draw, after the root
/// reservation and successor have been consumed into a separate capability.
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct CompletedColdDrawCarrier {
    _prepared: super::super::GpuPreparedFrame<GpuGeometryFrameOwner<ImportedOneXsPicture>>,
    _geometry: GpuGeometryFrameOwner<()>,
    _validity: GpuResidentValidity,
    _post: GpuMotionResidentL2Post<GpuColdPriorPublicLevelTwo>,
    _public: wgpu::Buffer,
    _context: OneXsGpuContext,
    _flight: crate::flow::one_xs::pis::gpu::GpuPisFlight,
    root: crate::flow::one_xs::one_xs_belt_gpu::resident_frame_gpu::GpuResidentIdentity,
}

impl CompletedColdFinalOperands {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn into_install_parts(
        mut self,
    ) -> Fallible<CompletedColdInstallParts> {
        let root = self
            .post
            .resident_identity()?
            .ok_or("ONE X2 final install lost its capture root identity")?;
        let geometry = self.prepared.belts.lease.complete_into_owner()?;
        let (source, geometry) = geometry.into_installed_parts();
        // Every refusal retains the root inside `self`; the separately
        // extracted source local is then dropped before `self`. Success takes
        // the already-compared pair through a non-fallible constructor.
        let candidate = self.post.take_install_candidate()?;
        Ok(CompletedColdInstallParts {
            source,
            carrier: CompletedColdDrawCarrier {
                _prepared: self.prepared,
                _geometry: geometry,
                _validity: self.validity,
                _post: self.post,
                _public: self.public,
                _context: self.context,
                _flight: self.flight,
                root,
            },
            candidate,
        })
    }
}

impl CompletedColdDrawCarrier {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn matches_root(
        &self,
        root: &crate::flow::one_xs::one_xs_belt_gpu::resident_frame_gpu::GpuResidentIdentity,
    ) -> bool {
        self.root.matches(root)
    }
}

pub(in crate::flow::one_xs::one_xs_belt_gpu) fn admit_completed_cold_final(
    mut checkpoint: GpuCompletedColdCheckpoint<ImportedOneXsPicture>,
    producer: &OneXsGpuContext,
) -> Fallible<CompletedColdFinalOperands> {
    validate_completed_cold_final(&checkpoint, producer)?;
    let flight = checkpoint
        .prepared
        .as_ref()
        .expect("validated Cold2 prepared owner")
        .flight
        .clone();
    Ok(CompletedColdFinalOperands {
        prepared: checkpoint
            .prepared
            .take()
            .expect("checked Cold2 prepared owner"),
        validity: checkpoint
            .validity
            .take()
            .expect("checked Cold2 validity owner"),
        post: checkpoint.post.take().expect("checked Cold2 post owner"),
        public: checkpoint.public,
        context: producer.clone(),
        flight,
    })
}

pub(in crate::flow::one_xs::one_xs_belt_gpu) fn validate_completed_cold_final(
    checkpoint: &GpuCompletedColdCheckpoint<ImportedOneXsPicture>,
    producer: &OneXsGpuContext,
) -> Fallible<()> {
    let prepared = checkpoint
        .prepared
        .as_ref()
        .ok_or("resident Cold2 final map lost its prepared frame")?;
    producer.ensure_same(&prepared.context)?;
    let flight = prepared.flight.clone();
    let validity = checkpoint
        .validity
        .as_ref()
        .ok_or("resident Cold2 final map lost its validity owner")?;
    let post = checkpoint
        .post
        .as_ref()
        .ok_or("resident Cold2 final map lost its post-L1 owner")?;
    producer.ensure_same(post.context())?;
    let root = post
        .resident_identity()?
        .ok_or("resident Cold2 final map lost its capture root identity")?;
    validity.ensure_identity(producer, &flight, Some(&root))?;
    post.ensure_final_reservation(producer, &flight, &root)?;
    prepared.belts.lease.validate_provenance(producer)?;
    prepared
        .belts
        .lease
        .ensure_final_map_sources(&flight.frame)?;
    if checkpoint.public.size() != words_bytes(PUBLIC_WORDS) {
        return Err(format!(
            "ONE X2 Cold2 public-flow buffer is {} bytes, expected {}",
            checkpoint.public.size(),
            words_bytes(PUBLIC_WORDS)
        )
        .into());
    }
    if validity.buffer.size() != words_bytes(1) {
        return Err(format!(
            "ONE X2 resident validity buffer is {} bytes, expected {}",
            validity.buffer.size(),
            words_bytes(1)
        )
        .into());
    }
    Ok(())
}

impl resident::Sealed for CompletedColdFinalOperands {}

impl resident::Operands for CompletedColdFinalOperands {
    fn context(&self) -> &OneXsGpuContext {
        &self.context
    }

    fn frame(&self) -> &kjerag_media::FrameStamp {
        &self.flight.frame
    }

    fn encode_dynamic_input_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: resident::DynamicInputTarget<'_>,
    ) {
        self.prepared
            .belts
            .lease
            .copy_final_map_dynamic_inputs(encoder, target, &self.public)
            .expect("Cold2 final-map sources were checked before encoding");
    }

    fn encode_validity_copy(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Buffer) {
        encoder.copy_buffer_to_buffer(&self.validity.buffer, 0, target, 0, words_bytes(1));
    }

    fn submit_after(
        &mut self,
        producer: &OneXsGpuContext,
        command: wgpu::CommandBuffer,
    ) -> Fallible<()> {
        self.prepared
            .belts
            .lease
            .submit_after(producer, |_| command)
    }
}

#[cfg(test)]
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct ColdLifecycleSnapshot {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) lack_rows: Vec<u32>,
    pub(in crate::flow::one_xs::one_xs_belt_gpu) small_rows: Vec<u32>,
    pub(in crate::flow::one_xs::one_xs_belt_gpu) cadence: [i32; 2],
    pub(in crate::flow::one_xs::one_xs_belt_gpu) calculation: u8,
    pub(in crate::flow::one_xs::one_xs_belt_gpu) small_present: bool,
    pub(in crate::flow::one_xs::one_xs_belt_gpu) validity: u32,
    root: crate::flow::one_xs::one_xs_belt_gpu::resident_frame_gpu::GpuResidentIdentity,
}

#[cfg(test)]
impl ColdLifecycleSnapshot {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn same_root(&self, other: &Self) -> bool {
        self.root.matches(&other.root)
    }
}

#[cfg(test)]
fn lifecycle_snapshot<K, O>(loop_owner: &GpuColdLoop<K, O>) -> Fallible<ColdLifecycleSnapshot> {
    lifecycle_snapshot_parts(
        loop_owner.post.as_ref().ok_or("cold snapshot lost post")?,
        loop_owner
            .validity
            .as_ref()
            .ok_or("cold snapshot lost validity")?,
    )
}

#[cfg(test)]
fn lifecycle_snapshot_parts(
    post: &GpuMotionResidentL2Post<GpuColdPriorPublicLevelTwo>,
    validity: &GpuResidentValidity,
) -> Fallible<ColdLifecycleSnapshot> {
    let (lack, small, cadence, calculation, small_present, root) = post.cold_lifecycle_for_test();
    let context = post.context();
    Ok(ColdLifecycleSnapshot {
        lack_rows: super::read_buffer_words(context, lack, 2 * PATCH_ROWS)
            .map_err(|error| error.to_string())?,
        small_rows: super::read_buffer_words(context, small, 2 * PATCH_ROWS)
            .map_err(|error| error.to_string())?,
        cadence,
        calculation,
        small_present,
        validity: super::read_buffer_words(context, &validity.buffer, 1)
            .map_err(|error| error.to_string())?[0],
        root,
    })
}

#[cfg(test)]
impl<K, O> GpuColdLoop<K, O> {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn snapshot_for_test(
        &self,
    ) -> Fallible<ColdLifecycleSnapshot> {
        lifecycle_snapshot(self)
    }
}

#[cfg(test)]
impl<K> GpuCompletedColdCheckpoint<K> {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn snapshot_for_test(
        &self,
    ) -> Fallible<ColdLifecycleSnapshot> {
        lifecycle_snapshot_parts(
            self.post.as_ref().ok_or("cold snapshot lost post")?,
            self.validity
                .as_ref()
                .ok_or("cold snapshot lost validity")?,
        )
    }
}

#[cfg(test)]
impl GpuCompletedColdCheckpoint<ImportedOneXsPicture> {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn observe_final_lease(
        &mut self,
        completion: std::sync::Arc<std::sync::atomic::AtomicU8>,
        submissions: std::sync::Arc<std::sync::atomic::AtomicU8>,
    ) {
        let lease = &mut self
            .prepared
            .as_mut()
            .expect("completed Cold2 test checkpoint retains prepared owner")
            .belts
            .lease;
        lease.observe(completion);
        lease.observe_submit(submissions);
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn public_words_for_test(
        &self,
    ) -> Fallible<Vec<u32>> {
        let context = &self
            .prepared
            .as_ref()
            .ok_or("completed Cold2 test checkpoint lost prepared owner")?
            .context;
        super::read_buffer_words(context, &self.public, PUBLIC_WORDS)
            .map_err(|error| error.to_string().into())
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn replace_frame_for_test(
        &mut self,
        frame: kjerag_media::FrameStamp,
    ) -> kjerag_media::FrameStamp {
        std::mem::replace(
            &mut self
                .prepared
                .as_mut()
                .expect("completed Cold2 test checkpoint retains prepared owner")
                .flight
                .frame,
            frame,
        )
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn replace_validity_root_for_test(
        &mut self,
        root: super::super::super::resident_frame_gpu::GpuResidentIdentity,
    ) -> Option<super::super::super::resident_frame_gpu::GpuResidentIdentity> {
        self.validity
            .as_mut()
            .expect("completed Cold2 test checkpoint retains validity owner")
            .resident
            .replace(root)
    }
}

impl<K> GpuCold0Terminal<K> {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn new(
        terminal: GpuL1PreparedTerminal<
            GpuGeometryFrameOwner<K>,
            GpuMotionResidentL2Post<GpuColdPriorPublicLevelTwo>,
        >,
        controls: super::GpuColdLoopControls,
    ) -> Self {
        Self { terminal, controls }
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn complete(
        self,
        bridge: &super::GpuL2PostPisBridge,
    ) -> Fallible<GpuColdLoop<K, ColdAfter0>> {
        let completed = complete_cold_tail(self.terminal, &bridge.post_l1, 0)?;
        Ok(GpuColdLoop {
            prepared: Some(completed.prepared),
            validity: Some(completed.validity),
            post: Some(completed.post),
            controls: self.controls,
            _public_scratch: completed.public,
            ordinal: std::marker::PhantomData,
        })
    }
}

impl<K, O: ColdLoopOrdinal> GpuColdLoop<K, O> {
    fn resume_terminal(
        mut self,
        bridge: &super::GpuL2PostPisBridge,
        solver: &crate::flow::one_xs::pis::gpu::GpuPisPipeline,
    ) -> Fallible<
        GpuL1PreparedTerminal<
            GpuGeometryFrameOwner<K>,
            GpuMotionResidentL2Post<GpuColdPriorPublicLevelTwo>,
        >,
    > {
        let calculation = O::COMPLETED + 1;
        let mut joined = ColdResumeGuard {
            prepared: self.prepared.take(),
            validity: self.validity.take(),
            post: self.post.take(),
        };
        if joined.prepared.is_none() || joined.validity.is_none() || joined.post.is_none() {
            return Err("resident cold loop lost its carrier-first resume owner".into());
        }
        let prepared = joined.prepared.take().expect("checked cold resume carrier");
        let validity = joined
            .validity
            .take()
            .expect("checked cold resume validity");
        let post = joined
            .post
            .take()
            .expect("checked cold resume temporal owner");
        prepared
            .submit_resident_l2_bridge(
                solver,
                bridge,
                self.controls.l2.clone(),
                PairSolveStage::Cold {
                    calculation: calculation.into(),
                    level: Level::Two,
                },
                post,
                Some(validity),
            )?
            .submit_l1_pis(bridge, solver, self.controls.l1.clone())
    }
}

impl<K> GpuColdLoop<K, ColdAfter0> {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn resume(
        self,
        bridge: &super::GpuL2PostPisBridge,
        solver: &crate::flow::one_xs::pis::gpu::GpuPisPipeline,
    ) -> Fallible<GpuColdLoop<K, ColdAfter1>> {
        let controls = self.controls.clone();
        let terminal = self.resume_terminal(bridge, solver)?;
        let completed = complete_cold_tail(terminal, &bridge.post_l1, 1)?;
        Ok(GpuColdLoop {
            prepared: Some(completed.prepared),
            validity: Some(completed.validity),
            post: Some(completed.post),
            controls,
            _public_scratch: completed.public,
            ordinal: std::marker::PhantomData,
        })
    }
}

impl<K> GpuColdLoop<K, ColdAfter1> {
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn resume(
        self,
        bridge: &super::GpuL2PostPisBridge,
        solver: &crate::flow::one_xs::pis::gpu::GpuPisPipeline,
    ) -> Fallible<GpuCompletedColdCheckpoint<K>> {
        let terminal = self.resume_terminal(bridge, solver)?;
        let completed = complete_cold_tail(terminal, &bridge.post_l1, 2)?;
        let mut boundary = GpuCompletedColdCheckpoint {
            prepared: Some(completed.prepared),
            validity: Some(completed.validity),
            post: Some(completed.post),
            public: completed.public,
        };
        boundary
            .post
            .as_mut()
            .ok_or("resident cold loop lost its temporal owner")?
            .attach_cold_successor()?;
        Ok(boundary)
    }
}

struct ColdTailGuard<K> {
    terminal: Option<GpuPreparedTerminal<GpuGeometryFrameOwner<K>>>,
    post: Option<GpuMotionResidentL2Post<GpuColdPriorPublicLevelTwo>>,
}

struct CompletedColdTail<K> {
    prepared: super::super::GpuPreparedFrame<GpuGeometryFrameOwner<K>>,
    validity: GpuResidentValidity,
    post: GpuMotionResidentL2Post<GpuColdPriorPublicLevelTwo>,
    public: wgpu::Buffer,
}

fn complete_cold_tail<K>(
    input: GpuL1PreparedTerminal<
        GpuGeometryFrameOwner<K>,
        GpuMotionResidentL2Post<GpuColdPriorPublicLevelTwo>,
    >,
    pipeline: &GpuColdPostL1Pipeline,
    calculation: u8,
) -> Fallible<CompletedColdTail<K>> {
    if input.ordinal.l1_stage()
        != (PairSolveStage::Cold {
            calculation: calculation.into(),
            level: Level::One,
        })
    {
        return Err("ONE X2 resident cold post-L1 ordinal is not its sealed successor".into());
    }
    pipeline.context.ensure_same(&input.terminal.context)?;
    pipeline.context.ensure_same(input.post.context())?;
    let resident_identity = input.post.resident_identity()?;
    input
        .terminal
        .resident_validity
        .as_ref()
        .ok_or("resident cold post-L1 lost inherited validity")?
        .ensure_identity(
            &input.terminal.context,
            &input.terminal.receipt.flight,
            resident_identity.as_ref(),
        )?;
    let GpuL1PreparedTerminal {
        terminal,
        post,
        ordinal: _,
    } = input;
    let mut guard = ColdTailGuard {
        terminal: Some(terminal),
        post: Some(post),
    };
    let terminal = guard.terminal.as_ref().expect("cold tail lost terminal");
    let post = guard.post.as_ref().expect("cold tail lost post owner");
    let prepared = &terminal.prepared;
    let device = pipeline.context.device();
    let histogram = buffer(device, "ONE X2 cold next temporal histograms", HIST_WORDS);
    let fifo = buffer(device, "ONE X2 cold next temporal FIFOs", FIFO_WORDS);
    let hints = buffer(device, "ONE X2 cold next planar hints", HINT_WORDS);
    let small_rows = buffer(
        device,
        "ONE X2 cold retained empty small rows",
        2 * PATCH_ROWS,
    );
    let lack_rows = buffer(device, "ONE X2 cold retained lack rows", 2 * PATCH_ROWS);
    let filtered = buffer(device, "ONE X2 cold filtered patches", PATCH_WORDS);
    let dense = buffer(device, "ONE X2 cold dense L1", L1_WORDS);
    let horizontal = buffer(
        device,
        "ONE X2 cold resize horizontal products",
        HORIZONTAL_PRODUCT_WORDS,
    );
    let public = buffer(device, "ONE X2 cold public flow", PUBLIC_WORDS);
    let retained_l2_direction_pixel_vec2 = super::RetainedL2DirectionPixelVec2Buffer::new(buffer(
        device,
        "ONE X2 cold successor retained L2 direction-pixel-vec2",
        RETAINED_L2_WORDS,
    ));
    let bind = post.bind_cold_post_l1(
        device,
        &pipeline.layout,
        &terminal._output,
        &prepared.shared_images,
        &histogram,
        &fifo,
        &hints,
        &filtered,
        &dense,
        &horizontal,
        &public,
        &pipeline.quantized_values,
        retained_l2_direction_pixel_vec2.buffer(),
        &terminal
            .resident_validity
            .as_ref()
            .expect("checked resident cold validity")
            .buffer,
    );
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ONE X2 resident cold post-L1"),
    });
    post.encode_cold_post_l1_copies(
        &mut encoder,
        &histogram,
        &fifo,
        &lack_rows,
        &prepared.l1_lack_rows,
        calculation,
    );
    encoder.clear_buffer(&hints, 0, None);
    encoder.clear_buffer(&small_rows, 0, None);
    encode_cold_passes(&mut encoder, pipeline, &bind, calculation == 2);
    guard
        .terminal
        .as_mut()
        .expect("cold tail lost terminal")
        .prepared
        .belts
        .lease
        .submit_after(&pipeline.context, |_| encoder.finish())?;
    let post = guard.post.as_mut().ok_or("cold tail lost post owner")?;
    post.commit_cold_post_l1(
        histogram,
        fifo,
        hints,
        small_rows,
        lack_rows,
        public.clone(),
        (calculation == 2).then_some(retained_l2_direction_pixel_vec2),
        calculation,
    );
    let validity = guard
        .terminal
        .as_mut()
        .ok_or("cold tail lost terminal")?
        .resident_validity
        .take()
        .ok_or("resident cold post-L1 lost inherited validity")?;
    let terminal = guard.terminal.take().expect("checked cold tail terminal");
    let post = guard.post.take().expect("checked cold tail post owner");
    let GpuPreparedTerminal {
        receipt: _,
        context: _,
        _output: _,
        _output_span_words: _,
        _b_output_base_words: _,
        resident_validity,
        prepared,
    } = terminal;
    debug_assert!(resident_validity.is_none());
    Ok(CompletedColdTail {
        prepared,
        validity,
        post,
        public,
    })
}

fn encode_cold_passes(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &GpuColdPostL1Pipeline,
    bind: &wgpu::BindGroup,
    final_call: bool,
) {
    let schedule = [
        (0usize, 2 * L1_ROWS * L1_COLS),
        (1, 2 * L2_ROWS * L2_COLS),
        (2, 2 * PATCHES),
        (6, 2 * L1_ROWS * L1_COLS),
        (3, 2 * L1_ROWS * PUB_COLS * 2),
        (4, 2 * PUB_ROWS * PUB_COLS * 2),
    ];
    for (index, count) in schedule {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ONE X2 resident cold post-L1"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline.passes[index]);
        pass.set_bind_group(0, bind, &[]);
        pass.dispatch_workgroups((count as u32).div_ceil(64), 1, 1);
    }
    if final_call {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ONE X2 resident cold periodic repair"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline.passes[5]);
        pass.set_bind_group(0, bind, &[]);
        pass.dispatch_workgroups(((2 * 5 * PUB_COLS * 2) as u32).div_ceil(64), 1, 1);
        drop(pass);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ONE X2 resident cold retained L2 successor"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline.passes[7]);
        pass.set_bind_group(0, bind, &[]);
        pass.dispatch_workgroups(((2 * L2_ROWS * L2_COLS) as u32).div_ceil(64), 1, 1);
    }
}

fn buffer(device: &wgpu::Device, label: &'static str, words: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: words_bytes(words),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
fn upload_words(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    words: &[u32],
) -> wgpu::Buffer {
    let result = buffer(device, label, words.len());
    let bytes = words
        .iter()
        .flat_map(|word| word.to_ne_bytes())
        .collect::<Vec<_>>();
    queue.write_buffer(&result, 0, &bytes);
    result
}
fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}
const SHADER: &str = include_str!("post_l1.wgsl");

#[cfg(test)]
#[path = "post_l1_oracle_tests.rs"]
mod oracle_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_cold_shader_contract(shader: &str) {
        for base in [
            "0u", "64800u", "129600u", "133650u", "137700u", "202500u", "267300u", "271350u",
        ] {
            assert!(shader.contains(base), "missing planar hint base {base}");
        }
        assert_eq!(HINT_WORDS, 275_400);
        assert!(shader.contains("fn densify_cold"));
        assert!(shader.contains("fn make_retained_l2"));
        assert!(!shader.contains("fn densify_and_retain"));
        assert!(!shader.contains("fn update_small_rows"));
        assert!(shader.contains("fn make_hint_l1"));
        assert!(shader.contains("fn make_hint_l2"));
        assert!(shader.contains("fn temporal_median"));
        assert!(shader.contains("if (nan(v)) { return 0u; }"));
    }

    #[test]
    fn cold_shader_pins_planar_abi_and_semantic_order() {
        assert_cold_shader_contract(SHADER);
    }

    #[test]
    fn cold_shader_contract_rejects_live_layout_and_order_mutations() {
        for mutated in [
            SHADER.replace("64800u", "64801u"),
            SHADER.replace("271350u", "271349u"),
            SHADER.replacen("fn make_hint_l2(", "fn mutated_hint_l2(", 1),
            SHADER.replacen("fn densify_cold", "fn densify_and_retain", 1),
        ] {
            assert!(std::panic::catch_unwind(|| assert_cold_shader_contract(&mutated)).is_err());
        }
    }

    #[test]
    fn warm_shader_contract_pins_order_association_and_exclusions() {
        assert!(SHADER.contains("resized=((tl+tr)*0.5+(bl+br)*0.5)*0.5;"));
        assert!(SHADER.contains("resized=(((tl+tr)+bl)+br)*0.25;"));
        assert!(SHADER.contains("let v=vote(dir,row,col,true);"));
        assert!(SHADER.contains("filtered[2u * id.x + 1u] = bitcast<u32>(raw_at"));
        assert!(SHADER.contains("fn densify_warm"));
        assert!(SHADER.contains("fn repair_periodic"));
        assert!(SHADER.contains("fn make_retained_l2"));
        assert!(!SHADER.contains("fn update_small_rows"));
    }

    #[test]
    fn warm_successor_state_pins_classifier_rows_and_retained_l2_abi() {
        assert_eq!(2 * PATCH_ROWS, 356);
        assert_eq!(2 * (PATCHES + PATCHES - 1) + 1, PATCH_WORDS - 1);
        assert_eq!(RETAINED_L2_WORDS, 16_200);
        assert_eq!(
            super::super::retained_l2_direction_pixel_vec2_index(1, 17, 1),
            2 * (L2_ROWS * L2_COLS + 17) + 1
        );
    }

    #[test]
    fn resident_cold_state_shapes_pin_full_hint_tails_and_history() {
        assert_eq!(PATCH_WORDS, 5_696);
        assert_eq!(HIST_WORDS, 452_832);
        assert_eq!(FIFO_WORDS, 14_240);
        assert_eq!(HINT_WORDS, 275_400);
        assert_eq!(L1_WORDS, 64_800);
        assert_eq!(RETAINED_L2_WORDS, 16_200);
        assert_eq!(PUBLIC_WORDS, 259_200);
        assert_eq!(64_800 - L1_ROWS * L1_COLS, 48_600);
    }
}
