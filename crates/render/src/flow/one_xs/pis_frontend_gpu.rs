//! Complete immutable GPU-resident image-owned PIS front end.
//!
//! One frame token owns the shared recursive image and physical-mask pyramids,
//! plus the direction-owned L1/L2 gradients, exact weights, rolling patch
//! denominators and five-word source models. L1 additionally owns the native
//! lack-of-texture rows and physical block mask. The readable CPU `Input`
//! remains the oracle. Scene does not select this path.

use std::marker::PhantomData;
use std::sync::mpsc;

use super::pis::gpu::GpuPisFlight;
use super::pis::{AtoB, BtoA, Level, PisDirection};
use super::scalar::{ColdInputs, LevelInputs, MaskPyramid};
use super::temporal::BlurredBelts;
use super::{COLS, Direction, LensPair, ROWS};
use crate::Fallible;
use crate::flow::one_xs_belt::SolverBelts;
use crate::flow::one_xs_belt_gpu::GpuBlurredBelts;

const MODEL_WORDS_PER_PATCH: usize = 5;
const L1_PIXELS: usize = Level::One.pixels();
const L2_PIXELS: usize = Level::Two.pixels();
const SHARED_PIXELS: usize = 2 * (L1_PIXELS + L2_PIXELS);
const STAGE_PIXELS: usize = 2 * (L1_PIXELS + L2_PIXELS);
const STAGE_PATCHES: usize = 2 * (Level::One.patches() + Level::Two.patches());
const L1_LACK_ROWS: usize = 2 * Level::One.patch_rows();
const L1_BLOCKS: usize = Level::One.patches();
const MASK_WORDS_PER_LENS: usize = (ROWS * COLS).div_ceil(4);

/// Exact identity of one GPU-resident image-owned preprocessing result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GpuPisFrontEndReceipt {
    pub(crate) flight: GpuPisFlight,
    pub(crate) direction: Direction,
    pub(crate) level: Level,
}

/// Typed immutable view of one direction and level in the shared frame token.
pub(crate) struct GpuPreparedLevel<D: PisDirection> {
    receipt: GpuPisFrontEndReceipt,
    gradient_bytes: std::ops::Range<u64>,
    weight_bytes: std::ops::Range<u64>,
    patch_weight_sum_bytes: std::ops::Range<u64>,
    model_bytes: std::ops::Range<u64>,
    lack_row_bytes: Option<std::ops::Range<u64>>,
    block_mask_bytes: Option<std::ops::Range<u64>>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> GpuPreparedLevel<D> {
    pub(crate) fn receipt(&self) -> &GpuPisFrontEndReceipt {
        &self.receipt
    }

    pub(crate) fn gradient_bytes(&self) -> std::ops::Range<u64> {
        self.gradient_bytes.clone()
    }

    pub(crate) fn weight_bytes(&self) -> std::ops::Range<u64> {
        self.weight_bytes.clone()
    }

    pub(crate) fn patch_weight_sum_bytes(&self) -> std::ops::Range<u64> {
        self.patch_weight_sum_bytes.clone()
    }

    pub(crate) fn model_bytes(&self) -> std::ops::Range<u64> {
        self.model_bytes.clone()
    }

    pub(crate) fn lack_row_bytes(&self) -> Option<std::ops::Range<u64>> {
        self.lack_row_bytes.clone()
    }

    pub(crate) fn block_mask_bytes(&self) -> Option<std::ops::Range<u64>> {
        self.block_mask_bytes.clone()
    }
}

/// Shared physical A/B image and mask ranges for one recursive level.
pub(crate) struct GpuSharedLevel {
    level: Level,
    image_a_bytes: std::ops::Range<u64>,
    image_b_bytes: std::ops::Range<u64>,
    mask_a_bytes: std::ops::Range<u64>,
    mask_b_bytes: std::ops::Range<u64>,
}

impl GpuSharedLevel {
    pub(crate) fn level(&self) -> Level {
        self.level
    }
    pub(crate) fn image_a_bytes(&self) -> std::ops::Range<u64> {
        self.image_a_bytes.clone()
    }
    pub(crate) fn image_b_bytes(&self) -> std::ops::Range<u64> {
        self.image_b_bytes.clone()
    }
    pub(crate) fn mask_a_bytes(&self) -> std::ops::Range<u64> {
        self.mask_a_bytes.clone()
    }
    pub(crate) fn mask_b_bytes(&self) -> std::ops::Range<u64> {
        self.mask_b_bytes.clone()
    }
}

/// Direction-owned L1/L2 views into one immutable frame allocation.
pub(crate) struct GpuPreparedDirection<D: PisDirection> {
    pub(crate) level_one: GpuPreparedLevel<D>,
    pub(crate) level_two: GpuPreparedLevel<D>,
}

/// Complete per-frame front end, still resident and deliberately unselected.
///
/// The embedded belt token preserves the checkpoint's existing lifetime
/// behavior. This type makes no stronger ownership or Scene-readiness claim.
#[must_use = "the GPU-resident PIS frame front end has not been consumed"]
pub(crate) struct GpuPreparedFrame<K> {
    pub(crate) shared_level_one: GpuSharedLevel,
    pub(crate) shared_level_two: GpuSharedLevel,
    pub(crate) a_to_b: GpuPreparedDirection<AtoB>,
    pub(crate) b_to_a: GpuPreparedDirection<BtoA>,
    shared_images: wgpu::Buffer,
    shared_masks: wgpu::Buffer,
    gradients: wgpu::Buffer,
    raw_weights: wgpu::Buffer,
    patch_weight_sums: wgpu::Buffer,
    models: wgpu::Buffer,
    l1_lack_rows: wgpu::Buffer,
    l1_block_mask: wgpu::Buffer,
    _weight_horizontal: wgpu::Buffer,
    _resources: wgpu::BindGroup,
    _belts: GpuBlurredBelts<K>,
    submission: wgpu::SubmissionIndex,
}

impl<K> GpuPreparedFrame<K> {
    pub(crate) fn shared_images(&self) -> &wgpu::Buffer {
        &self.shared_images
    }
    pub(crate) fn shared_masks(&self) -> &wgpu::Buffer {
        &self.shared_masks
    }
    pub(crate) fn gradients(&self) -> &wgpu::Buffer {
        &self.gradients
    }
    pub(crate) fn raw_weights(&self) -> &wgpu::Buffer {
        &self.raw_weights
    }
    pub(crate) fn patch_weight_sums(&self) -> &wgpu::Buffer {
        &self.patch_weight_sums
    }
    pub(crate) fn models(&self) -> &wgpu::Buffer {
        &self.models
    }
    pub(crate) fn l1_lack_rows(&self) -> &wgpu::Buffer {
        &self.l1_lack_rows
    }
    pub(crate) fn l1_block_mask(&self) -> &wgpu::Buffer {
        &self.l1_block_mask
    }
    pub(crate) fn submission(&self) -> &wgpu::SubmissionIndex {
        &self.submission
    }
}

/// Render-private production shader plus mandatory target-device CPU twin.
pub(crate) struct GpuPisFrontEnd {
    reduce: wgpu::ComputePipeline,
    gradient: wgpu::ComputePipeline,
    weight_horizontal: wgpu::ComputePipeline,
    weight_vertical: wgpu::ComputePipeline,
    patches: wgpu::ComputePipeline,
    l1_aux: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl GpuPisFrontEnd {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Fallible<Self> {
        Self::from_shader(device, queue, SHADER, true)
    }

    fn from_shader(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        shader: &str,
        qualify: bool,
    ) -> Fallible<Self> {
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
            label: Some("ONE X2 GPU PIS prepared-source front end"),
            entries: &[
                storage(0, true),
                storage(1, true),
                storage(2, false),
                storage(3, false),
                storage(4, false),
                storage(5, false),
                storage(6, false),
                storage(7, false),
                storage(8, false),
                storage(9, false),
                storage(10, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 GPU PIS prepared-source front end"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 GPU PIS prepared-source front end"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = |entry_point| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ONE X2 GPU PIS prepared-source front end"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry_point),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let built = Self {
            reduce: pipeline("reduce_shared"),
            gradient: pipeline("prepare_gradients"),
            weight_horizontal: pipeline("prepare_weight_horizontal"),
            weight_vertical: pipeline("prepare_weight_vertical"),
            patches: pipeline("prepare_patches"),
            l1_aux: pipeline("prepare_l1_aux"),
            layout,
        };
        if qualify {
            built.qualify(device, queue)?;
        }
        Ok(built)
    }

    /// Consume one exact no-readback belt token and enqueue the complete
    /// immutable two-direction, two-level front end. Same-queue ordering makes
    /// the producer visible without a CPU poll.
    pub(crate) fn prepare<K>(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        belts: GpuBlurredBelts<K>,
        physical_masks: &LensPair<Vec<u8>>,
    ) -> Fallible<GpuPreparedFrame<K>> {
        validate_masks(physical_masks)?;
        let mask = upload_masks(device, queue, physical_masks);
        let outputs = OutputBuffers::new(device);
        let resources = self.resources(device, belts.packed(), &mask, &outputs);
        let submission = self.dispatch(device, queue, &resources);
        let flight = belts.flight().clone();
        let (shared_level_one, shared_level_two) = shared_views();
        let views = prepared_views(&flight);
        Ok(GpuPreparedFrame {
            shared_level_one,
            shared_level_two,
            a_to_b: views.0,
            b_to_a: views.1,
            shared_images: outputs.shared_images,
            shared_masks: outputs.shared_masks,
            gradients: outputs.gradients,
            raw_weights: outputs.raw_weights,
            patch_weight_sums: outputs.patch_weight_sums,
            models: outputs.models,
            l1_lack_rows: outputs.l1_lack_rows,
            l1_block_mask: outputs.l1_block_mask,
            _weight_horizontal: outputs.weight_horizontal,
            _resources: resources,
            _belts: belts,
            submission,
        })
    }

    fn resources(
        &self,
        device: &wgpu::Device,
        belts: &wgpu::Buffer,
        mask: &wgpu::Buffer,
        outputs: &OutputBuffers,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 GPU PIS prepared-source resources"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: belts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: mask.as_entire_binding(),
                },
                entry(2, &outputs.shared_images),
                entry(3, &outputs.shared_masks),
                entry(4, &outputs.gradients),
                entry(5, &outputs.weight_horizontal),
                entry(6, &outputs.raw_weights),
                entry(7, &outputs.patch_weight_sums),
                entry(8, &outputs.models),
                entry(9, &outputs.l1_lack_rows),
                entry(10, &outputs.l1_block_mask),
            ],
        })
    }

    fn dispatch(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &wgpu::BindGroup,
    ) -> wgpu::SubmissionIndex {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 GPU PIS prepared-source front end"),
        });
        encode(&mut encoder, self, resources);
        queue.submit([encoder.finish()])
    }

    fn qualify(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Fallible<()> {
        let (blurred, masks) = qualification_fixture();
        let expected = cpu_outputs(&blurred, &masks);
        let packed = upload_belts(device, queue, &blurred);
        let mask = upload_masks(device, queue, &masks);
        let outputs = OutputBuffers::new(device);
        let resources = self.resources(device, &packed, &mask, &outputs);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 GPU PIS prepared-source qualification"),
        });
        encode(&mut encoder, self, &resources);
        let output_words = OutputBuffers::section_word_counts()
            .into_iter()
            .sum::<usize>();
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 GPU PIS prepared-source qualification readback"),
            size: words_bytes(output_words),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut byte_offset = 0;
        for (buffer, words) in outputs.sections() {
            encoder.copy_buffer_to_buffer(buffer, 0, &readback, byte_offset, words_bytes(words));
            byte_offset += words_bytes(words);
        }
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
        let actual = bytes
            .chunks_exact(4)
            .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
            .collect::<Vec<_>>();
        drop(bytes);
        readback.unmap();
        let mut offset = 0;
        for (name, expected) in expected {
            if let Some((word, (actual, expected))) = actual[offset..offset + expected.len()]
                .iter()
                .copied()
                .zip(expected.iter().copied())
                .enumerate()
                .find(|(_, (actual, expected))| actual != expected)
            {
                return Err(format!("ONE X2 GPU PIS front-end arithmetic is not exact on this graphics device: {name} word {word} bits are {actual:#010x}, expected {expected:#010x}").into());
            }
            offset += expected.len();
        }
        Ok(())
    }
}

fn encode(
    encoder: &mut wgpu::CommandEncoder,
    pipelines: &GpuPisFrontEnd,
    resources: &wgpu::BindGroup,
) {
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("ONE X2 GPU PIS prepared-source front end"),
        timestamp_writes: None,
    });
    pass.set_bind_group(0, resources, &[]);
    for (pipeline, words) in [
        (&pipelines.reduce, SHARED_PIXELS),
        (&pipelines.gradient, STAGE_PIXELS),
        (&pipelines.weight_horizontal, STAGE_PIXELS),
        (&pipelines.weight_vertical, STAGE_PIXELS),
        (&pipelines.patches, STAGE_PATCHES),
        (&pipelines.l1_aux, L1_BLOCKS.max(L1_LACK_ROWS)),
    ] {
        pass.set_pipeline(pipeline);
        pass.dispatch_workgroups(words.div_ceil(64) as u32, 1, 1);
    }
}

fn validate_masks(masks: &LensPair<Vec<u8>>) -> Fallible<()> {
    let expected = ROWS * COLS;
    for (lens, actual) in [('A', masks.a.len()), ('B', masks.b.len())] {
        if actual != expected {
            return Err(format!(
                "ONE X2 GPU PIS physical mask {lens} has {actual} bytes, expected {expected}"
            )
            .into());
        }
    }
    Ok(())
}

fn upload_masks(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    masks: &LensPair<Vec<u8>>,
) -> wgpu::Buffer {
    // The trailing runtime-zero word pins the CPU model's explicit binary32
    // round boundaries without changing the physical mask payload.
    let mut words = vec![0u32; 2 * MASK_WORDS_PER_LENS + 1];
    for (lens, mask) in [&masks.a, &masks.b].into_iter().enumerate() {
        for (index, value) in mask.iter().copied().enumerate() {
            let word = lens * MASK_WORDS_PER_LENS + index / 4;
            words[word] |= u32::from(value) << (8 * (index % 4));
        }
    }
    upload_words(device, queue, "ONE X2 GPU PIS physical masks", &words)
}

fn upload_belts(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    blurred: &BlurredBelts,
) -> wgpu::Buffer {
    let mut words = Vec::with_capacity(SolverBelts::BYTES / 4);
    for codes in blurred.bytes().chunks_exact(4) {
        words.push(u32::from_le_bytes(codes.try_into().unwrap()));
    }
    upload_words(device, queue, "ONE X2 GPU PIS qualification belts", &words)
}

fn upload_words(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    words: &[u32],
) -> wgpu::Buffer {
    let bytes = words
        .iter()
        .flat_map(|word| word.to_ne_bytes())
        .collect::<Vec<_>>();
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, &bytes);
    buffer
}

fn cpu_outputs(blurred: &BlurredBelts, masks: &LensPair<Vec<u8>>) -> Vec<(&'static str, Vec<u32>)> {
    let retained = ColdInputs::from_prepared(blurred.clone().into_lenses(), masks.clone())
        .expect("qualification fixture has the retained shape");
    let pyramid = MaskPyramid::build(&retained);
    let ab1 = LevelInputs::build::<AtoB>(&retained, &pyramid, Level::One)
        .front_end_oracle::<AtoB>(Level::One);
    let ba1 = LevelInputs::build::<BtoA>(&retained, &pyramid, Level::One)
        .front_end_oracle::<BtoA>(Level::One);
    let ab2 = LevelInputs::build::<AtoB>(&retained, &pyramid, Level::Two)
        .front_end_oracle::<AtoB>(Level::Two);
    let ba2 = LevelInputs::build::<BtoA>(&retained, &pyramid, Level::Two)
        .front_end_oracle::<BtoA>(Level::Two);
    assert_eq!(ab1.block_mask, ba1.block_mask);
    vec![
        (
            "shared images",
            join([&ab1.image_a, &ab1.image_b, &ab2.image_a, &ab2.image_b]),
        ),
        (
            "shared masks",
            join([&ab1.mask_a, &ab1.mask_b, &ab2.mask_a, &ab2.mask_b]),
        ),
        (
            "direction gradients",
            join([
                &ab1.gradients,
                &ba1.gradients,
                &ab2.gradients,
                &ba2.gradients,
            ]),
        ),
        (
            "weight horizontal scratch",
            join([
                &ab1.weight_horizontal,
                &ba1.weight_horizontal,
                &ab2.weight_horizontal,
                &ba2.weight_horizontal,
            ]),
        ),
        (
            "raw weights",
            join([
                &ab1.raw_weight,
                &ba1.raw_weight,
                &ab2.raw_weight,
                &ba2.raw_weight,
            ]),
        ),
        (
            "rolling patch weight sums",
            join([
                &ab1.patch_weight_sums,
                &ba1.patch_weight_sums,
                &ab2.patch_weight_sums,
                &ba2.patch_weight_sums,
            ]),
        ),
        (
            "five-word source models",
            join([&ab1.models, &ba1.models, &ab2.models, &ba2.models]),
        ),
        (
            "L1 lack-of-texture rows",
            join([&ab1.lack_rows, &ba1.lack_rows]),
        ),
        ("L1 physical block mask", ab1.block_mask),
    ]
}

fn join<const N: usize>(parts: [&Vec<u32>; N]) -> Vec<u32> {
    parts
        .into_iter()
        .flat_map(|part| part.iter().copied())
        .collect()
}

fn words_bytes(words: usize) -> u64 {
    (words * size_of::<u32>()) as u64
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn storage_buffer(device: &wgpu::Device, label: &'static str, words: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: words_bytes(words),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

struct OutputBuffers {
    shared_images: wgpu::Buffer,
    shared_masks: wgpu::Buffer,
    gradients: wgpu::Buffer,
    weight_horizontal: wgpu::Buffer,
    raw_weights: wgpu::Buffer,
    patch_weight_sums: wgpu::Buffer,
    models: wgpu::Buffer,
    l1_lack_rows: wgpu::Buffer,
    l1_block_mask: wgpu::Buffer,
}

impl OutputBuffers {
    fn new(device: &wgpu::Device) -> Self {
        Self {
            shared_images: storage_buffer(device, "ONE X2 GPU PIS shared images", SHARED_PIXELS),
            shared_masks: storage_buffer(device, "ONE X2 GPU PIS shared masks", SHARED_PIXELS),
            gradients: storage_buffer(device, "ONE X2 GPU PIS gradients", 2 * STAGE_PIXELS),
            weight_horizontal: storage_buffer(
                device,
                "ONE X2 GPU PIS horizontal weights",
                STAGE_PIXELS,
            ),
            raw_weights: storage_buffer(device, "ONE X2 GPU PIS raw weights", STAGE_PIXELS),
            patch_weight_sums: storage_buffer(
                device,
                "ONE X2 GPU PIS rolling patch sums",
                STAGE_PATCHES,
            ),
            models: storage_buffer(
                device,
                "ONE X2 GPU PIS source models",
                MODEL_WORDS_PER_PATCH * STAGE_PATCHES,
            ),
            l1_lack_rows: storage_buffer(device, "ONE X2 GPU PIS L1 lack rows", L1_LACK_ROWS),
            l1_block_mask: storage_buffer(device, "ONE X2 GPU PIS L1 block mask", L1_BLOCKS),
        }
    }

    const fn section_word_counts() -> [usize; 9] {
        [
            SHARED_PIXELS,
            SHARED_PIXELS,
            2 * STAGE_PIXELS,
            STAGE_PIXELS,
            STAGE_PIXELS,
            STAGE_PATCHES,
            MODEL_WORDS_PER_PATCH * STAGE_PATCHES,
            L1_LACK_ROWS,
            L1_BLOCKS,
        ]
    }

    fn sections(&self) -> [(&wgpu::Buffer, usize); 9] {
        let counts = Self::section_word_counts();
        [
            (&self.shared_images, counts[0]),
            (&self.shared_masks, counts[1]),
            (&self.gradients, counts[2]),
            (&self.weight_horizontal, counts[3]),
            (&self.raw_weights, counts[4]),
            (&self.patch_weight_sums, counts[5]),
            (&self.models, counts[6]),
            (&self.l1_lack_rows, counts[7]),
            (&self.l1_block_mask, counts[8]),
        ]
    }
}

fn prepared_views(
    flight: &GpuPisFlight,
) -> (GpuPreparedDirection<AtoB>, GpuPreparedDirection<BtoA>) {
    fn level<D: PisDirection>(
        flight: &GpuPisFlight,
        level: Level,
        pixel_start: usize,
        patch_start: usize,
        lack_start: Option<usize>,
    ) -> GpuPreparedLevel<D> {
        GpuPreparedLevel {
            receipt: GpuPisFrontEndReceipt {
                flight: flight.clone(),
                direction: D::DIRECTION,
                level,
            },
            gradient_bytes: words_bytes(2 * pixel_start)
                ..words_bytes(2 * (pixel_start + level.pixels())),
            weight_bytes: words_bytes(pixel_start)..words_bytes(pixel_start + level.pixels()),
            patch_weight_sum_bytes: words_bytes(patch_start)
                ..words_bytes(patch_start + level.patches()),
            model_bytes: words_bytes(MODEL_WORDS_PER_PATCH * patch_start)
                ..words_bytes(MODEL_WORDS_PER_PATCH * (patch_start + level.patches())),
            lack_row_bytes: lack_start
                .map(|start| words_bytes(start)..words_bytes(start + Level::One.patch_rows())),
            block_mask_bytes: (level == Level::One).then(|| 0..words_bytes(Level::One.patches())),
            direction: PhantomData,
        }
    }
    let ab1_patch = 0;
    let ba1_patch = Level::One.patches();
    let ab2_patch = 2 * Level::One.patches();
    let ba2_patch = ab2_patch + Level::Two.patches();
    (
        GpuPreparedDirection {
            level_one: level::<AtoB>(flight, Level::One, 0, ab1_patch, Some(0)),
            level_two: level::<AtoB>(flight, Level::Two, 2 * L1_PIXELS, ab2_patch, None),
        },
        GpuPreparedDirection {
            level_one: level::<BtoA>(
                flight,
                Level::One,
                L1_PIXELS,
                ba1_patch,
                Some(Level::One.patch_rows()),
            ),
            level_two: level::<BtoA>(
                flight,
                Level::Two,
                2 * L1_PIXELS + L2_PIXELS,
                ba2_patch,
                None,
            ),
        },
    )
}

fn shared_views() -> (GpuSharedLevel, GpuSharedLevel) {
    fn range(start: usize, words: usize) -> std::ops::Range<u64> {
        words_bytes(start)..words_bytes(start + words)
    }
    (
        GpuSharedLevel {
            level: Level::One,
            image_a_bytes: range(0, L1_PIXELS),
            image_b_bytes: range(L1_PIXELS, L1_PIXELS),
            mask_a_bytes: range(0, L1_PIXELS),
            mask_b_bytes: range(L1_PIXELS, L1_PIXELS),
        },
        GpuSharedLevel {
            level: Level::Two,
            image_a_bytes: range(2 * L1_PIXELS, L2_PIXELS),
            image_b_bytes: range(2 * L1_PIXELS + L2_PIXELS, L2_PIXELS),
            mask_a_bytes: range(2 * L1_PIXELS, L2_PIXELS),
            mask_b_bytes: range(2 * L1_PIXELS + L2_PIXELS, L2_PIXELS),
        },
    )
}

fn qualification_fixture() -> (BlurredBelts, LensPair<Vec<u8>>) {
    let image = |salt: usize| {
        let mut pixels = (0..ROWS * COLS)
            .map(|index| {
                let row = index / COLS;
                let col = index % COLS;
                ((29 * row + 47 * col + 13 * salt + (row ^ col)) & 255) as u8
            })
            .collect::<Vec<_>>();
        if salt == 1 {
            // A horizontal rank-one patch forces determinant clamping while
            // retaining a nonzero inverse-row-row numerator. Include the L2
            // Sobel halo around patch row 10, column 1.
            for row in 116..156 {
                for col in 8..48 {
                    pixels[row * COLS + col] = 20 + 7 * (col / 4) as u8;
                }
            }
        }
        pixels
    };
    let mut masks = LensPair {
        a: (0..ROWS * COLS)
            .map(|index| u8::from(!index.is_multiple_of(17) && index % COLS != 0))
            .collect::<Vec<_>>(),
        b: (0..ROWS * COLS)
            .map(|index| u8::from(!index.is_multiple_of(19) && index % COLS + 1 != COLS))
            .collect::<Vec<_>>(),
    };
    // The first L1 block has exactly seven selected top-left mask samples.
    // This distinguishes native's 0.1 valid-fraction threshold from an
    // adjacent integer boundary without changing either physical mask shape.
    for row in 0..8 {
        for col in 0..8 {
            masks.a[(2 * row) * COLS + 2 * col] = u8::from(row == 0 && col < 7);
        }
    }
    for row in 116..156 {
        for col in 8..48 {
            masks.a[row * COLS + col] = 1;
        }
    }
    (
        BlurredBelts::from_lenses(LensPair {
            a: image(1),
            b: image(2),
        })
        .expect("qualification fixture has the retained shape"),
        masks,
    )
}

const SHADER: &str = r#"
const RETAINED_ROWS = 1080u;
const RETAINED_COLS = 60u;
const RETAINED_PIXELS = RETAINED_ROWS * RETAINED_COLS;
const RETAINED_MASK_WORDS = (RETAINED_PIXELS + 3u) / 4u;
const L1_ROWS = 540u;
const L1_COLS = 30u;
const L1_PIXELS = L1_ROWS * L1_COLS;
const L2_ROWS = 270u;
const L2_COLS = 15u;
const L2_PIXELS = L2_ROWS * L2_COLS;
const SHARED_PIXELS = 2u * (L1_PIXELS + L2_PIXELS);
const STAGE_PIXELS = SHARED_PIXELS;
const L1_PATCH_ROWS = 178u;
const L1_PATCH_COLS = 8u;
const L1_PATCHES = L1_PATCH_ROWS * L1_PATCH_COLS;
const L2_PATCH_ROWS = 88u;
const L2_PATCH_COLS = 3u;
const L2_PATCHES = L2_PATCH_ROWS * L2_PATCH_COLS;
const STAGE_PATCHES = 2u * (L1_PATCHES + L2_PATCHES);
const PATCH_SIZE = 8u;
const PATCH_STRIDE = 3u;
const MODEL_WORDS = 5u;

@group(0) @binding(0) var<storage, read> blurred_words: array<u32>;
@group(0) @binding(1) var<storage, read> mask_words: array<u32>;
@group(0) @binding(2) var<storage, read_write> shared_images: array<u32>;
@group(0) @binding(3) var<storage, read_write> shared_masks: array<u32>;
@group(0) @binding(4) var<storage, read_write> gradient_bits: array<u32>;
@group(0) @binding(5) var<storage, read_write> weight_horizontal_bits: array<u32>;
@group(0) @binding(6) var<storage, read_write> raw_weight_bits: array<u32>;
@group(0) @binding(7) var<storage, read_write> patch_weight_sum_bits: array<u32>;
@group(0) @binding(8) var<storage, read_write> model_bits: array<u32>;
@group(0) @binding(9) var<storage, read_write> lack_rows: array<u32>;
@group(0) @binding(10) var<storage, read_write> block_mask: array<u32>;

fn code(lens: u32, index: u32) -> u32 {
    let at = lens * RETAINED_PIXELS + index;
    return (blurred_words[at / 4u] >> (8u * (at % 4u))) & 255u;
}

fn retained_mask(lens: u32, index: u32) -> u32 {
    let word = lens * RETAINED_MASK_WORDS + index / 4u;
    return (mask_words[word] >> (8u * (index % 4u))) & 255u;
}

fn rounded_average4(a: u32, b: u32, c: u32, d: u32) -> u32 {
    return (a + b + c + d + 2u) / 4u;
}

fn l1_code(lens: u32, row: u32, col: u32) -> u32 {
    let r = row * 2u;
    let c = col * 2u;
    return rounded_average4(
        code(lens, r * RETAINED_COLS + c),
        code(lens, r * RETAINED_COLS + c + 1u),
        code(lens, (r + 1u) * RETAINED_COLS + c),
        code(lens, (r + 1u) * RETAINED_COLS + c + 1u),
    );
}

fn l2_code(lens: u32, row: u32, col: u32) -> u32 {
    let r = row * 2u;
    let c = col * 2u;
    return rounded_average4(
        l1_code(lens, r, c),
        l1_code(lens, r, c + 1u),
        l1_code(lens, r + 1u, c),
        l1_code(lens, r + 1u, c + 1u),
    );
}

fn reflect_101(value: i32, extent: i32) -> u32 {
    if value < 0 { return u32(-value); }
    if value >= extent { return u32(2 * extent - value - 2); }
    return u32(value);
}

fn materialize(value: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(value) ^ mask_words[2u * RETAINED_MASK_WORDS]);
}

fn side() -> f32 { return bitcast<f32>(0x3e8c52b9u); }
fn centre() -> f32 { return bitcast<f32>(0x3ee75a8eu); }

fn mul_rn(a: f32, b: f32) -> f32 {
    return materialize(fma(a, b, -0.0));
}

fn add_rn(a: f32, b: f32) -> f32 {
    return materialize(fma(a, 1.0, b));
}

fn fma_rn(a: f32, b: f32, c: f32) -> f32 {
    return materialize(fma(a, b, c));
}

// WGSL permits implementation-defined division precision. The CPU model
// requires correctly-rounded binary32, so form quotient bits with the same
// 24-step restoring division used by the qualified GPU PIS kernel.
fn div_f32_bits(a: u32, b: u32) -> u32 {
    let sign = (a ^ b) & 0x80000000u;
    let a_abs = a & 0x7fffffffu;
    let b_abs = b & 0x7fffffffu;
    let a_exp = a_abs >> 23u;
    let b_exp = b_abs >> 23u;
    let a_frac = a_abs & 0x007fffffu;
    let b_frac = b_abs & 0x007fffffu;

    if a_exp == 0xffu && a_frac != 0u { return a | 0x00400000u; }
    if b_exp == 0xffu && b_frac != 0u { return b | 0x00400000u; }
    if (a_abs == 0u && b_abs == 0u) || (a_exp == 0xffu && b_exp == 0xffu) {
        return 0xffc00000u;
    }
    if b_abs == 0u || a_exp == 0xffu { return sign | 0x7f800000u; }
    if a_abs == 0u || b_exp == 0xffu { return sign; }

    var ma = a_frac;
    var mb = b_frac;
    var ea = i32(a_exp) - 127;
    var eb = i32(b_exp) - 127;
    if a_exp == 0u {
        let top = 31u - countLeadingZeros(a_frac);
        ma = a_frac << (23u - top);
        ea = i32(top) - 149;
    } else {
        ma |= 0x00800000u;
    }
    if b_exp == 0u {
        let top = 31u - countLeadingZeros(b_frac);
        mb = b_frac << (23u - top);
        eb = i32(top) - 149;
    } else {
        mb |= 0x00800000u;
    }

    var remainder = ma;
    var quotient_exponent = ea - eb;
    if remainder < mb {
        remainder <<= 1u;
        quotient_exponent -= 1;
    }
    var quotient = 0u;
    for (var step = 0u; step < 24u; step++) {
        let bit = 23u - step;
        if remainder >= mb {
            remainder -= mb;
            quotient |= 1u << bit;
        }
        if step != 23u { remainder <<= 1u; }
    }

    if quotient_exponent >= -126 {
        let twice_remainder = remainder << 1u;
        if twice_remainder > mb || (twice_remainder == mb && (quotient & 1u) != 0u) {
            quotient += 1u;
        }
        if quotient == 0x01000000u {
            quotient = 0x00800000u;
            quotient_exponent += 1;
        }
        if quotient_exponent > 127 { return sign | 0x7f800000u; }
        return sign | (u32(quotient_exponent + 127) << 23u) | (quotient & 0x007fffffu);
    }

    let shift = u32(-126 - quotient_exponent);
    if shift >= 25u { return sign; }
    var subnormal = quotient >> shift;
    let mask = (1u << shift) - 1u;
    let low = quotient & mask;
    let half = 1u << (shift - 1u);
    if low > half || (low == half && (remainder != 0u || (subnormal & 1u) != 0u)) {
        subnormal += 1u;
    }
    return sign | subnormal;
}

fn div_rn(a: f32, b: f32) -> f32 {
    return bitcast<f32>(div_f32_bits(bitcast<u32>(a), bitcast<u32>(b)));
}

struct Stage {
    base: u32,
    source_base: u32,
    mask_a_base: u32,
    local: u32,
    rows: u32,
    cols: u32,
    patch_rows: u32,
    patch_cols: u32,
};

fn stage_pixel(index: u32) -> Stage {
    if index < L1_PIXELS {
        return Stage(0u, 0u, 0u, index, L1_ROWS, L1_COLS, L1_PATCH_ROWS, L1_PATCH_COLS);
    }
    if index < 2u * L1_PIXELS {
        return Stage(L1_PIXELS, L1_PIXELS, 0u, index - L1_PIXELS, L1_ROWS, L1_COLS, L1_PATCH_ROWS, L1_PATCH_COLS);
    }
    if index < 2u * L1_PIXELS + L2_PIXELS {
        return Stage(2u * L1_PIXELS, 2u * L1_PIXELS, 2u * L1_PIXELS, index - 2u * L1_PIXELS, L2_ROWS, L2_COLS, L2_PATCH_ROWS, L2_PATCH_COLS);
    }
    return Stage(2u * L1_PIXELS + L2_PIXELS, 2u * L1_PIXELS + L2_PIXELS,
        2u * L1_PIXELS, index - 2u * L1_PIXELS - L2_PIXELS,
        L2_ROWS, L2_COLS, L2_PATCH_ROWS, L2_PATCH_COLS);
}

fn stage_patch(index: u32) -> Stage {
    if index < L1_PATCHES {
        return Stage(0u, 0u, 0u, index, L1_ROWS, L1_COLS, L1_PATCH_ROWS, L1_PATCH_COLS);
    }
    if index < 2u * L1_PATCHES {
        return Stage(L1_PIXELS, L1_PIXELS, 0u, index - L1_PATCHES, L1_ROWS, L1_COLS, L1_PATCH_ROWS, L1_PATCH_COLS);
    }
    if index < 2u * L1_PATCHES + L2_PATCHES {
        return Stage(2u * L1_PIXELS, 2u * L1_PIXELS, 2u * L1_PIXELS,
            index - 2u * L1_PATCHES, L2_ROWS, L2_COLS, L2_PATCH_ROWS, L2_PATCH_COLS);
    }
    return Stage(2u * L1_PIXELS + L2_PIXELS, 2u * L1_PIXELS + L2_PIXELS,
        2u * L1_PIXELS, index - 2u * L1_PATCHES - L2_PATCHES,
        L2_ROWS, L2_COLS, L2_PATCH_ROWS, L2_PATCH_COLS);
}

fn gradient(stage: Stage, row: u32, col: u32) -> vec2<f32> {
    if shared_masks[stage.mask_a_base + row * stage.cols + col] == 0u {
        return vec2<f32>(0.0);
    }
    var gx = 0.0;
    var gy = 0.0;
    for (var dr = 0u; dr < 3u; dr += 1u) {
        let rr = reflect_101(i32(row) + i32(dr) - 1, i32(stage.rows));
        let sy = select(1.0, 2.0, dr == 1u);
        let dy = f32(i32(dr) - 1);
        for (var dc = 0u; dc < 3u; dc += 1u) {
            let cc = reflect_101(i32(col) + i32(dc) - 1, i32(stage.cols));
            let sx = select(1.0, 2.0, dc == 1u);
            let dx = f32(i32(dc) - 1);
            let value = f32(shared_images[stage.source_base + rr * stage.cols + cc]);
            gx = add_rn(gx, mul_rn(mul_rn(value, dx), sy));
            gy = add_rn(gy, mul_rn(mul_rn(value, sx), dy));
        }
    }
    return vec2<f32>(gx, gy);
}

fn gradient_at(stage: Stage, row: u32, col: u32) -> vec2<f32> {
    let at = stage.base + row * stage.cols + col;
    return vec2<f32>(bitcast<f32>(gradient_bits[2u * at]), bitcast<f32>(gradient_bits[2u * at + 1u]));
}

fn weight_at(stage: Stage, row: u32, col: u32) -> f32 {
    return bitcast<f32>(raw_weight_bits[stage.base + row * stage.cols + col]);
}

fn horizontal_patch_sum(stage: Stage, row: u32, patch_col: u32) -> f32 {
    var sum = 0.0;
    for (var col = 0u; col < PATCH_SIZE; col += 1u) {
        sum = add_rn(sum, weight_at(stage, row, col));
    }
    let origin = patch_col * PATCH_STRIDE;
    for (var source_col = 1u; source_col <= origin; source_col += 1u) {
        let entering = weight_at(stage, row, source_col + PATCH_SIZE - 1u);
        let leaving = weight_at(stage, row, source_col - 1u);
        sum = add_rn(sum, add_rn(entering, -leaving));
    }
    return sum;
}

fn rolling_patch_sum(stage: Stage, patch_row: u32, patch_col: u32) -> f32 {
    var sum = 0.0;
    for (var row = 0u; row < PATCH_SIZE; row += 1u) {
        sum = add_rn(sum, horizontal_patch_sum(stage, row, patch_col));
    }
    let origin = patch_row * PATCH_STRIDE;
    for (var source_row = 1u; source_row <= origin; source_row += 1u) {
        let entering = horizontal_patch_sum(stage, source_row + PATCH_SIZE - 1u, patch_col);
        let leaving = horizontal_patch_sum(stage, source_row - 1u, patch_col);
        sum = add_rn(sum, add_rn(entering, -leaving));
    }
    return sum;
}

@compute @workgroup_size(64)
fn reduce_shared(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if index >= SHARED_PIXELS { return; }
    var lens = 0u;
    var level = 1u;
    var local = index;
    var rows = L1_ROWS;
    var cols = L1_COLS;
    if local >= L1_PIXELS { lens = 1u; local -= L1_PIXELS; }
    if index >= 2u * L1_PIXELS {
        level = 2u; rows = L2_ROWS; cols = L2_COLS;
        local = index - 2u * L1_PIXELS;
        lens = 0u;
        if local >= L2_PIXELS { lens = 1u; local -= L2_PIXELS; }
    }
    let row = local / cols;
    let col = local % cols;
    shared_images[index] = select(l1_code(lens, row, col), l2_code(lens, row, col), level == 2u);
    let scale = select(2u, 4u, level == 2u);
    shared_masks[index] = retained_mask(lens, (row * scale) * RETAINED_COLS + col * scale);
}

@compute @workgroup_size(64)
fn prepare_gradients(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= STAGE_PIXELS { return; }
    let stage = stage_pixel(id.x);
    let row = stage.local / stage.cols;
    let col = stage.local % stage.cols;
    let g = gradient(stage, row, col);
    gradient_bits[2u * id.x] = bitcast<u32>(g.x);
    gradient_bits[2u * id.x + 1u] = bitcast<u32>(g.y);
}

@compute @workgroup_size(64)
fn prepare_weight_horizontal(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= STAGE_PIXELS { return; }
    let stage = stage_pixel(id.x);
    let row = stage.local / stage.cols;
    let col = stage.local % stage.cols;
    let left_col = reflect_101(i32(col) - 1, i32(stage.cols));
    let right_col = reflect_101(i32(col) + 1, i32(stage.cols));
    let gl = gradient_at(stage, row, left_col);
    let gm = gradient_at(stage, row, col);
    let gr = gradient_at(stage, row, right_col);
    let left = add_rn(abs(gl.x), abs(gl.y));
    let middle = add_rn(abs(gm.x), abs(gm.y));
    let right = add_rn(abs(gr.x), abs(gr.y));
    let sides = add_rn(left, right);
    var value = fma_rn(middle, centre(), mul_rn(sides, side()));
    if stage.cols % 2u == 1u && col + 1u == stage.cols {
        value = fma_rn(sides, side(), mul_rn(middle, centre()));
    }
    weight_horizontal_bits[id.x] = bitcast<u32>(value);
}

@compute @workgroup_size(64)
fn prepare_weight_vertical(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= STAGE_PIXELS { return; }
    let stage = stage_pixel(id.x);
    let row = stage.local / stage.cols;
    let col = stage.local % stage.cols;
    let top = bitcast<f32>(weight_horizontal_bits[stage.base + reflect_101(i32(row) - 1, i32(stage.rows)) * stage.cols + col]);
    let middle = bitcast<f32>(weight_horizontal_bits[id.x]);
    let bottom = bitcast<f32>(weight_horizontal_bits[stage.base + reflect_101(i32(row) + 1, i32(stage.rows)) * stage.cols + col]);
    var value = fma_rn(add_rn(top, bottom), side(), mul_rn(middle, centre()));
    if shared_masks[stage.mask_a_base + stage.local] == 0u || value < 0.0 { value = 0.0; }
    raw_weight_bits[id.x] = bitcast<u32>(value);
}

@compute @workgroup_size(64)
fn prepare_patches(@builtin(global_invocation_id) id: vec3<u32>) {
    let patch_index = id.x;
    if patch_index >= STAGE_PATCHES { return; }
    let stage = stage_patch(patch_index);
    let patch_row = stage.local / stage.patch_cols;
    let patch_col = stage.local % stage.patch_cols;
    let origin_row = patch_row * PATCH_STRIDE;
    let origin_col = patch_col * PATCH_STRIDE;
    patch_weight_sum_bits[patch_index] = bitcast<u32>(rolling_patch_sum(stage, patch_row, patch_col));
    var sums = array<f32, 5>(0.0, 0.0, 0.0, 0.0, 0.0);
    for (var row = 0u; row < PATCH_SIZE; row += 1u) {
        for (var col = 0u; col < PATCH_SIZE; col += 1u) {
            let g = gradient_at(stage, origin_row + row, origin_col + col);
            sums[0] = add_rn(sums[0], g.x);
            sums[1] = add_rn(sums[1], g.y);
            sums[2] = add_rn(sums[2], mul_rn(g.x, g.x));
            sums[3] = add_rn(sums[3], mul_rn(g.x, g.y));
            sums[4] = add_rn(sums[4], mul_rn(g.y, g.y));
        }
    }
    let negative_cross_square = -mul_rn(sums[3], sums[3]);
    var determinant = fma_rn(sums[2], sums[4], negative_cross_square);
    if abs(determinant) < 0.001 { determinant = 0.001; }
    model_bits[patch_index * MODEL_WORDS] = bitcast<u32>(sums[0]);
    model_bits[patch_index * MODEL_WORDS + 1u] = bitcast<u32>(sums[1]);
    model_bits[patch_index * MODEL_WORDS + 2u] = bitcast<u32>(div_rn(sums[4], determinant));
    model_bits[patch_index * MODEL_WORDS + 3u] = bitcast<u32>(div_rn(-sums[3], determinant));
    model_bits[patch_index * MODEL_WORDS + 4u] = bitcast<u32>(div_rn(sums[2], determinant));
}

@compute @workgroup_size(64)
fn prepare_l1_aux(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x < 2u * L1_PATCH_ROWS {
        let direction = select(0u, 1u, id.x >= L1_PATCH_ROWS);
        let row = id.x % L1_PATCH_ROWS;
        let stage = stage_pixel(direction * L1_PIXELS);
        var sum = 0.0;
        var count = 0u;
        for (var patch_col = 0u; patch_col < L1_PATCH_COLS; patch_col += 1u) {
            var patch_value = 0.0;
            let origin_row = row * PATCH_STRIDE;
            let origin_col = patch_col * PATCH_STRIDE;
            for (var dr = 0u; dr < PATCH_SIZE; dr += 1u) {
                for (var dc = 0u; dc < PATCH_SIZE; dc += 1u) {
                    patch_value = add_rn(patch_value, weight_at(stage, origin_row + dr, origin_col + dc));
                }
            }
            if patch_value > 1.0 { sum = add_rn(sum, patch_value); count += 1u; }
        }
        let mean = select(div_rn(sum, f32(count)), 0.0, count == 0u);
        lack_rows[id.x] = u32(mean < 2000.0);
    }
    if id.x < L1_PATCHES {
        let patch_row = id.x / L1_PATCH_COLS;
        let patch_col = id.x % L1_PATCH_COLS;
        var nonzero = 0u;
        for (var dr = 0u; dr < PATCH_SIZE; dr += 1u) {
            for (var dc = 0u; dc < PATCH_SIZE; dc += 1u) {
                let row = patch_row * PATCH_STRIDE + dr;
                let col = patch_col * PATCH_STRIDE + dc;
                nonzero += u32(shared_masks[row * L1_COLS + col] != 0u);
            }
        }
        block_mask[id.x] = select(0u, 255u, nonzero >= 7u);
    }
}
"#;

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::*;

    #[test]
    fn fixture_exercises_every_source_model_component() {
        let (blurred, masks) = qualification_fixture();
        let outputs = cpu_outputs(&blurred, &masks);
        let model_words = &outputs
            .iter()
            .find(|(name, _)| *name == "five-word source models")
            .unwrap()
            .1;
        let ab2_start = 2 * Level::One.patches() * MODEL_WORDS_PER_PATCH;
        let models = model_words[ab2_start..ab2_start + Level::Two.patches() * 5]
            .chunks_exact(5)
            .collect::<Vec<_>>();
        for component in 0..MODEL_WORDS_PER_PATCH {
            assert!(
                models.iter().any(|model| model[component] != 0),
                "prepared-source model component {component} is inert"
            );
        }
        let rank_one = models[10 * Level::Two.patch_cols() + 1];
        assert_eq!(rank_one[2], 0, "rank-one inverse column term");
        assert_eq!(rank_one[3], (-0.0f32).to_bits(), "rank-one cross term");
        assert_ne!(rank_one[4], 0, "determinant clamp has no live numerator");
    }

    #[test]
    fn production_front_end_qualifies_on_the_actual_adapter() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping ONE X2 GPU prepared-source qualification: {why}");
                return;
            }
        };
        GpuPisFrontEnd::new(&device, &queue).unwrap_or_else(|error| {
            panic!("ONE X2 GPU prepared-source front end failed on {adapter}: {error}")
        });
    }

    #[test]
    fn qualification_refuses_front_end_semantic_mutations() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping ONE X2 GPU prepared-source mutation: {why}");
                return;
            }
        };
        let mutations = [
            (
                "recursive area rounding",
                "return (a + b + c + d + 2u) / 4u;",
                "return (a + b + c + d + 1u) / 4u;",
            ),
            (
                "direction-owned source lens",
                "return Stage(L1_PIXELS, L1_PIXELS, 0u, index - L1_PIXELS,",
                "return Stage(L1_PIXELS, 0u, 0u, index - L1_PIXELS,",
            ),
            (
                "recursive physical mask sampling",
                "let scale = select(2u, 4u, level == 2u);",
                "let scale = 2u;",
            ),
            (
                "physical mask A zeroing",
                "if shared_masks[stage.mask_a_base + row * stage.cols + col] == 0u {",
                "if shared_masks[stage.mask_a_base + row * stage.cols + col] == 255u {",
            ),
            (
                "reflect-101 Sobel border",
                "if value < 0 { return u32(-value); }",
                "if value < 0 { return 0u; }",
            ),
            (
                "L2 odd final-column Gaussian association",
                "value = fma_rn(sides, side(), mul_rn(middle, centre()));",
                "value = bitcast<f32>(bitcast<u32>(fma_rn(sides, side(), mul_rn(middle, centre()))) + 1u);",
            ),
            (
                "rolling patch sums",
                "sum = add_rn(sum, add_rn(entering, -leaving));",
                "sum = add_rn(sum, entering);",
            ),
            (
                "positive determinant clamp",
                "if abs(determinant) < 0.001 { determinant = 0.001; }",
                "if abs(determinant) < 0.001 { determinant = 0.002; }",
            ),
            (
                "L1 lack-of-texture rows",
                "lack_rows[id.x] = u32(mean < 2000.0);",
                "lack_rows[id.x] = u32(mean >= 2000.0);",
            ),
            (
                "L1 physical block mask",
                "block_mask[id.x] = select(0u, 255u, nonzero >= 7u);",
                "block_mask[id.x] = select(0u, 255u, nonzero >= 8u);",
            ),
            (
                "correctly-rounded divider",
                "return bitcast<f32>(div_f32_bits(bitcast<u32>(a), bitcast<u32>(b)));",
                "return bitcast<f32>(div_f32_bits(bitcast<u32>(a), bitcast<u32>(b)) + 1u);",
            ),
        ];
        for (name, before, after) in mutations {
            let mutated = SHADER.replacen(before, after, 1);
            assert_ne!(mutated, SHADER, "{name} mutation found no target");
            let error = match GpuPisFrontEnd::from_shader(&device, &queue, &mutated, true) {
                Ok(_) => {
                    panic!("changed ONE X2 GPU prepared-source {name} was accepted on {adapter}")
                }
                Err(error) => error,
            };
            assert!(
                error
                    .to_string()
                    .contains("front-end arithmetic is not exact"),
                "changed prepared-source {name} returned the wrong refusal on {adapter}: {error}"
            );
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
        let name = adapter.get_info().name;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("exact ONE X2 GPU prepared-source front end"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        Ok((device, queue, name))
    }
}
