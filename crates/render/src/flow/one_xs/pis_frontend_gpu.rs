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

mod level_marker {
    pub trait Sealed {}
}

/// Compile-time identity of one prepared recursive level.
pub(crate) trait GpuPreparedLevelMarker: level_marker::Sealed + 'static {
    const LEVEL: Level;
}

pub(crate) struct GpuLevelOne;
pub(crate) struct GpuLevelTwo;

impl level_marker::Sealed for GpuLevelOne {}
impl level_marker::Sealed for GpuLevelTwo {}
impl GpuPreparedLevelMarker for GpuLevelOne {
    const LEVEL: Level = Level::One;
}
impl GpuPreparedLevelMarker for GpuLevelTwo {
    const LEVEL: Level = Level::Two;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PisPreparedBaseWords {
    source_image: u32,
    target_image: u32,
    source_mask: u32,
    target_mask: u32,
    gradient: u32,
    weight: u32,
    patch_sum: u32,
    model: u32,
    lack_rows: Option<u32>,
    block_mask: Option<u32>,
}

/// Borrow-tied, typed binding contract for one exact frame/direction/level.
///
/// Every storage binding covers its complete buffer at byte offset zero. The
/// shader receives these logical word bases in its private dynamic header, so
/// no unaligned storage-buffer slice can escape this bundle.
pub(crate) struct PisPreparedBinding<'a, D: PisDirection, L: GpuPreparedLevelMarker> {
    flight: &'a GpuPisFlight,
    shared_images: &'a wgpu::Buffer,
    shared_masks: &'a wgpu::Buffer,
    gradients: &'a wgpu::Buffer,
    raw_weights: &'a wgpu::Buffer,
    patch_weight_sums: &'a wgpu::Buffer,
    models: &'a wgpu::Buffer,
    l1_lack_rows: &'a wgpu::Buffer,
    l1_block_mask: &'a wgpu::Buffer,
    bases: PisPreparedBaseWords,
    marker: PhantomData<(D, L)>,
}

impl<'a, D: PisDirection, L: GpuPreparedLevelMarker> PisPreparedBinding<'a, D, L> {
    pub(crate) fn flight(&self) -> &'a GpuPisFlight {
        self.flight
    }
    pub(crate) fn level(&self) -> Level {
        L::LEVEL
    }
    pub(crate) fn direction(&self) -> Direction {
        D::DIRECTION
    }
    pub(crate) fn shared_images(&self) -> &'a wgpu::Buffer {
        self.shared_images
    }
    pub(crate) fn shared_masks(&self) -> &'a wgpu::Buffer {
        self.shared_masks
    }
    pub(crate) fn gradients(&self) -> &'a wgpu::Buffer {
        self.gradients
    }
    pub(crate) fn raw_weights(&self) -> &'a wgpu::Buffer {
        self.raw_weights
    }
    pub(crate) fn patch_weight_sums(&self) -> &'a wgpu::Buffer {
        self.patch_weight_sums
    }
    pub(crate) fn models(&self) -> &'a wgpu::Buffer {
        self.models
    }
    pub(crate) fn l1_lack_rows(&self) -> &'a wgpu::Buffer {
        self.l1_lack_rows
    }
    pub(crate) fn l1_block_mask(&self) -> &'a wgpu::Buffer {
        self.l1_block_mask
    }
    pub(crate) fn source_image_base_words(&self) -> u32 {
        self.bases.source_image
    }
    pub(crate) fn target_image_base_words(&self) -> u32 {
        self.bases.target_image
    }
    pub(crate) fn source_mask_base_words(&self) -> u32 {
        self.bases.source_mask
    }
    pub(crate) fn target_mask_base_words(&self) -> u32 {
        self.bases.target_mask
    }
    pub(crate) fn gradient_base_words(&self) -> u32 {
        self.bases.gradient
    }
    pub(crate) fn weight_base_words(&self) -> u32 {
        self.bases.weight
    }
    pub(crate) fn patch_sum_base_words(&self) -> u32 {
        self.bases.patch_sum
    }
    pub(crate) fn model_base_words(&self) -> u32 {
        self.bases.model
    }
    pub(crate) fn lack_row_base_words(&self) -> Option<u32> {
        self.bases.lack_rows
    }
    pub(crate) fn block_mask_base_words(&self) -> Option<u32> {
        self.bases.block_mask
    }
}

/// Complete per-frame front end, still resident and deliberately unselected.
///
#[must_use = "the GPU-resident PIS frame front end has not been consumed"]
pub(crate) struct GpuPreparedFrame<K> {
    flight: GpuPisFlight,
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
    belts: GpuBlurredBelts<K>,
}

impl<K> GpuPreparedFrame<K> {
    pub(crate) fn bind_pis_level<D, L>(&self) -> PisPreparedBinding<'_, D, L>
    where
        D: PisDirection,
        L: GpuPreparedLevelMarker,
    {
        PisPreparedBinding {
            flight: &self.flight,
            shared_images: &self.shared_images,
            shared_masks: &self.shared_masks,
            gradients: &self.gradients,
            raw_weights: &self.raw_weights,
            patch_weight_sums: &self.patch_weight_sums,
            models: &self.models,
            l1_lack_rows: &self.l1_lack_rows,
            l1_block_mask: &self.l1_block_mask,
            bases: prepared_bases::<D, L>(),
            marker: PhantomData,
        }
    }

    pub(crate) fn record_consumer_submission(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submission: wgpu::SubmissionIndex,
    ) -> Fallible<()> {
        self.belts
            .record_consumer_submission(device, queue, submission)
    }

    /// Consume the resident frame at its explicit CPU re-entry boundary.
    /// Success proves the latest same-queue consumer and every earlier stage,
    /// releases the imported source owner once and disarms cancellation Drop.
    pub(crate) fn acknowledge_terminal(mut self) -> Fallible<()> {
        self.belts.complete()
    }

    #[cfg(test)]
    fn observe_completion(&mut self, state: std::sync::Arc<std::sync::atomic::AtomicU8>) {
        self.belts.observe_completion(state);
    }

    #[cfg(test)]
    fn qualification_sections(&self) -> [(wgpu::Buffer, usize); 9] {
        let counts = OutputBuffers::section_word_counts();
        [
            (self.shared_images.clone(), counts[0]),
            (self.shared_masks.clone(), counts[1]),
            (self.gradients.clone(), counts[2]),
            (self._weight_horizontal.clone(), counts[3]),
            (self.raw_weights.clone(), counts[4]),
            (self.patch_weight_sums.clone(), counts[5]),
            (self.models.clone(), counts[6]),
            (self.l1_lack_rows.clone(), counts[7]),
            (self.l1_block_mask.clone(), counts[8]),
        ]
    }
}

/// Render-private production shader plus mandatory target-device CPU twin.
pub(crate) struct GpuPisFrontEnd {
    device: wgpu::Device,
    queue: wgpu::Queue,
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
            device: device.clone(),
            queue: queue.clone(),
            reduce: pipeline("reduce_shared"),
            gradient: pipeline("prepare_gradients"),
            weight_horizontal: pipeline("prepare_weight_horizontal"),
            weight_vertical: pipeline("prepare_weight_vertical"),
            patches: pipeline("prepare_patches"),
            l1_aux: pipeline("prepare_l1_aux"),
            layout,
        };
        if qualify {
            built.qualify()?;
        }
        Ok(built)
    }

    /// Consume one exact no-readback belt token and enqueue the complete
    /// immutable two-direction, two-level front end. Same-queue ordering makes
    /// the producer visible without a CPU poll.
    pub(crate) fn prepare<K>(
        &self,
        mut belts: GpuBlurredBelts<K>,
        physical_masks: &LensPair<Vec<u8>>,
    ) -> Fallible<GpuPreparedFrame<K>> {
        belts.validate_provenance(&self.device, &self.queue)?;
        validate_masks(physical_masks)?;
        let device = &self.device;
        let queue = &self.queue;
        let mask = upload_masks(device, queue, physical_masks);
        let outputs = OutputBuffers::new(device);
        let resources = self.resources(device, belts.packed(), &mask, &outputs);
        let submission = self.dispatch(device, queue, &resources);
        belts.record_consumer_submission(device, queue, submission)?;
        let flight = belts.take_flight();
        Ok(GpuPreparedFrame {
            flight,
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
            belts,
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

    fn qualify(&self) -> Fallible<()> {
        let device = &self.device;
        let queue = &self.queue;
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

fn prepared_bases<D: PisDirection, L: GpuPreparedLevelMarker>() -> PisPreparedBaseWords {
    let level = L::LEVEL;
    let (image_a, image_b) = match level {
        Level::One => (0, L1_PIXELS),
        Level::Two => (2 * L1_PIXELS, 2 * L1_PIXELS + L2_PIXELS),
    };
    let (source_image, target_image) = match D::DIRECTION {
        Direction::AtoB => (image_a, image_b),
        Direction::BtoA => (image_b, image_a),
    };
    let pixel_start = match (D::DIRECTION, level) {
        (Direction::AtoB, Level::One) => 0,
        (Direction::BtoA, Level::One) => L1_PIXELS,
        (Direction::AtoB, Level::Two) => 2 * L1_PIXELS,
        (Direction::BtoA, Level::Two) => 2 * L1_PIXELS + L2_PIXELS,
    };
    let patch_start = match (D::DIRECTION, level) {
        (Direction::AtoB, Level::One) => 0,
        (Direction::BtoA, Level::One) => Level::One.patches(),
        (Direction::AtoB, Level::Two) => 2 * Level::One.patches(),
        (Direction::BtoA, Level::Two) => 2 * Level::One.patches() + Level::Two.patches(),
    };
    PisPreparedBaseWords {
        source_image: source_image as u32,
        target_image: target_image as u32,
        // Physical mask arguments remain A then B in both directions.
        source_mask: image_a as u32,
        target_mask: image_b as u32,
        gradient: (2 * pixel_start) as u32,
        weight: pixel_start as u32,
        patch_sum: patch_start as u32,
        model: (MODEL_WORDS_PER_PATCH * patch_start) as u32,
        lack_rows: match (D::DIRECTION, level) {
            (Direction::AtoB, Level::One) => Some(0),
            (Direction::BtoA, Level::One) => Some(Level::One.patch_rows() as u32),
            (_, Level::Two) => None,
        },
        block_mask: (level == Level::One).then_some(0),
    }
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::time::Duration;

    use super::*;
    use crate::flow::one_xs::pis::gpu::{
        DIRECT_TEST_SHADER, GpuPisDynamicDirection, GpuPisDynamicStage, GpuPisPipeline,
        GpuPisStageReceipt, validate_direct_stage_for_test,
    };
    use crate::flow::one_xs::pis::{CostMode, DescentAdmission, Flow, HintGrid, InitialGrid};
    use crate::flow::one_xs::scalar::PairSolveStage;
    use crate::flow::one_xs_belt::{RetainedBaseMaps, SourceImage, sample_source_belts};
    use crate::flow::one_xs_belt_gpu::{
        GpuSolverBeltPipeline, SourceTextures, resident_qualification_fixture,
    };
    use kjerag_media::FrameStamp;

    struct DropProbe {
        wait_state: Arc<AtomicU8>,
        dropped: mpsc::Sender<u8>,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            let _ = self.dropped.send(self.wait_state.load(Ordering::SeqCst));
        }
    }

    #[test]
    fn prepared_binding_bases_cover_each_whole_buffer_without_overlap() {
        let stages = [
            prepared_bases::<AtoB, GpuLevelOne>(),
            prepared_bases::<BtoA, GpuLevelOne>(),
            prepared_bases::<AtoB, GpuLevelTwo>(),
            prepared_bases::<BtoA, GpuLevelTwo>(),
        ];
        let pixels = [L1_PIXELS, L1_PIXELS, L2_PIXELS, L2_PIXELS];
        let patches = [
            Level::One.patches(),
            Level::One.patches(),
            Level::Two.patches(),
            Level::Two.patches(),
        ];

        assert_partition(
            "direction gradients",
            stages
                .iter()
                .zip(pixels)
                .map(|(base, words)| (base.gradient as usize, 2 * words))
                .collect(),
            2 * STAGE_PIXELS,
        );
        assert_partition(
            "raw weights",
            stages
                .iter()
                .zip(pixels)
                .map(|(base, words)| (base.weight as usize, words))
                .collect(),
            STAGE_PIXELS,
        );
        assert_partition(
            "rolling patch sums",
            stages
                .iter()
                .zip(patches)
                .map(|(base, words)| (base.patch_sum as usize, words))
                .collect(),
            STAGE_PATCHES,
        );
        assert_partition(
            "source models",
            stages
                .iter()
                .zip(patches)
                .map(|(base, words)| (base.model as usize, MODEL_WORDS_PER_PATCH * words))
                .collect(),
            MODEL_WORDS_PER_PATCH * STAGE_PATCHES,
        );

        let ab1 = stages[0];
        let ba1 = stages[1];
        let ab2 = stages[2];
        let ba2 = stages[3];
        assert_eq!((ab1.source_image, ab1.target_image), (0, L1_PIXELS as u32));
        assert_eq!((ba1.source_image, ba1.target_image), (L1_PIXELS as u32, 0));
        assert_eq!(
            (ab2.source_image, ab2.target_image),
            ((2 * L1_PIXELS) as u32, (2 * L1_PIXELS + L2_PIXELS) as u32)
        );
        assert_eq!(
            (ba2.source_image, ba2.target_image),
            (ab2.target_image, ab2.source_image)
        );
        assert_eq!((ab1.source_mask, ab1.target_mask), (0, L1_PIXELS as u32));
        assert_eq!((ba1.source_mask, ba1.target_mask), (0, L1_PIXELS as u32));
        assert_eq!(ab1.lack_rows, Some(0));
        assert_eq!(ba1.lack_rows, Some(Level::One.patch_rows() as u32));
        assert_eq!(ab2.lack_rows, None);
        assert_eq!(ba2.lack_rows, None);
        assert_eq!(ab1.block_mask, Some(0));
        assert_eq!(ba1.block_mask, Some(0));
        assert_eq!(ab2.block_mask, None);
        assert_eq!(ba2.block_mask, None);
    }

    #[test]
    fn composed_resident_front_end_waits_only_at_terminal_acknowledgement() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping composed ONE X2 GPU front end: {why}");
                return;
            }
        };
        let front = GpuPisFrontEnd::new(&device, &queue)
            .unwrap_or_else(|error| panic!("GPU front end failed on {adapter}: {error}"));
        let state = Arc::new(AtomicU8::new(0));
        let (dropped, answer) = mpsc::channel();
        let flight = GpuPisFlight {
            generation: 17,
            frame: FrameStamp::for_test(23, Duration::from_secs(3), None),
        };
        let (mut belts, expected_blurred) = resident_qualification_fixture(
            &device,
            &queue,
            DropProbe {
                wait_state: Arc::clone(&state),
                dropped,
            },
            flight.clone(),
        )
        .unwrap_or_else(|error| panic!("resident producer failed on {adapter}: {error}"));
        belts.observe_completion(Arc::clone(&state));
        assert_eq!(state.load(Ordering::SeqCst), 0, "producer handoff polled");

        let masks = qualification_fixture().1;
        let expected = cpu_outputs(&expected_blurred, &masks);
        let frame = front
            .prepare(belts, &masks)
            .unwrap_or_else(|error| panic!("resident front end failed on {adapter}: {error}"));
        assert_eq!(state.load(Ordering::SeqCst), 0, "front-end handoff polled");
        assert!(matches!(answer.try_recv(), Err(mpsc::TryRecvError::Empty)));
        let identities = [
            binding_identity(frame.bind_pis_level::<AtoB, GpuLevelOne>()),
            binding_identity(frame.bind_pis_level::<BtoA, GpuLevelOne>()),
            binding_identity(frame.bind_pis_level::<AtoB, GpuLevelTwo>()),
            binding_identity(frame.bind_pis_level::<BtoA, GpuLevelTwo>()),
        ];
        for ((direction, level, seen_flight), expected_identity) in identities.into_iter().zip([
            (Direction::AtoB, Level::One),
            (Direction::BtoA, Level::One),
            (Direction::AtoB, Level::Two),
            (Direction::BtoA, Level::Two),
        ]) {
            assert_eq!(seen_flight, flight);
            assert_eq!((direction, level), expected_identity);
        }
        assert_eq!(
            0 % u64::from(device.limits().min_storage_buffer_offset_alignment),
            0,
            "whole-buffer bindings must be aligned"
        );
        let sections = frame.qualification_sections();
        frame.acknowledge_terminal().unwrap();
        assert_eq!(
            state.load(Ordering::SeqCst),
            2,
            "terminal wait was not exact"
        );
        assert_eq!(answer.recv().unwrap(), 2, "owner preceded terminal wait");
        assert_exact_sections(&device, &queue, sections, expected, &adapter);
        assert_eq!(
            state.load(Ordering::SeqCst),
            2,
            "disarmed frame waited twice"
        );
    }

    #[test]
    fn prepared_frame_early_drop_waits_for_latest_front_end_submission() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping prepared-frame cancellation: {why}");
                return;
            }
        };
        let front = GpuPisFrontEnd::new(&device, &queue).unwrap();
        let state = Arc::new(AtomicU8::new(0));
        let (dropped, answer) = mpsc::channel();
        let (mut belts, _) = resident_qualification_fixture(
            &device,
            &queue,
            DropProbe {
                wait_state: Arc::clone(&state),
                dropped,
            },
            GpuPisFlight {
                generation: 31,
                frame: FrameStamp::for_test(41, Duration::ZERO, None),
            },
        )
        .unwrap_or_else(|error| panic!("resident producer failed on {adapter}: {error}"));
        belts.observe_completion(Arc::clone(&state));
        let masks = qualification_fixture().1;
        let frame = front.prepare(belts, &masks).unwrap();
        assert_eq!(state.load(Ordering::SeqCst), 0);
        drop(frame);
        assert_eq!(state.load(Ordering::SeqCst), 2);
        assert_eq!(
            answer.recv().unwrap(),
            2,
            "owner preceded latest front-end wait"
        );
    }

    fn binding_identity<D, L>(
        binding: PisPreparedBinding<'_, D, L>,
    ) -> (Direction, Level, GpuPisFlight)
    where
        D: PisDirection,
        L: GpuPreparedLevelMarker,
    {
        (
            binding.direction(),
            binding.level(),
            binding.flight().clone(),
        )
    }

    fn assert_partition(name: &str, mut ranges: Vec<(usize, usize)>, total: usize) {
        ranges.sort_unstable();
        let mut cursor = 0;
        for (start, words) in ranges {
            assert_eq!(
                start, cursor,
                "{name} has a gap or overlap at word {cursor}"
            );
            cursor += words;
            assert!(cursor <= total, "{name} exceeds its whole buffer");
        }
        assert_eq!(cursor, total, "{name} does not cover its whole buffer");
    }

    fn assert_exact_sections(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sections: [(wgpu::Buffer, usize); 9],
        expected: Vec<(&'static str, Vec<u32>)>,
        adapter: &str,
    ) {
        let total_words = sections.iter().map(|(_, words)| words).sum::<usize>();
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 composed resident front-end readback"),
            size: words_bytes(total_words),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 composed resident front-end readback"),
        });
        let mut byte_offset = 0;
        for (buffer, words) in &sections {
            encoder.copy_buffer_to_buffer(buffer, 0, &readback, byte_offset, words_bytes(*words));
            byte_offset += words_bytes(*words);
        }
        let submission = queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (mapped, answer) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = mapped.send(result);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        answer.recv().unwrap().unwrap();
        let bytes = slice.get_mapped_range();
        let actual = bytes
            .chunks_exact(4)
            .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
            .collect::<Vec<_>>();
        drop(bytes);
        readback.unmap();
        let mut offset = 0;
        for (name, expected) in expected {
            assert_eq!(
                &actual[offset..offset + expected.len()],
                expected,
                "composed resident {name} differs from the CPU oracle on {adapter}"
            );
            offset += expected.len();
        }
        assert_eq!(offset, actual.len());
    }

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
    fn direct_bound_pis_matches_both_levels_and_directions_on_one_frame() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping direct-bound ONE X2 GPU PIS twin: {why}");
                return;
            }
        };
        let rows = 128;
        let cols = 256;
        let source = |salt: usize| {
            SourceImage::from_compact(
                rows,
                cols,
                (0..rows * cols)
                    .map(|at| ((37 * at + 19 * (at / cols) + 53 * salt) & 255) as u8)
                    .collect(),
            )
            .unwrap()
        };
        let sources = LensPair {
            a: source(1),
            b: source(2),
        };
        let maps = RetainedBaseMaps::from_lenses(LensPair {
            a: (0..ROWS * COLS)
                .map(|at| {
                    let row = at / COLS;
                    let col = at % COLS;
                    [
                        (3.25 + 0.71 * col as f32) / cols as f32,
                        (2.75 + 0.093 * row as f32) / rows as f32,
                    ]
                })
                .collect(),
            b: (0..ROWS * COLS)
                .map(|at| {
                    let row = at / COLS;
                    let col = at % COLS;
                    [
                        (7.5 + 0.63 * col as f32) / cols as f32,
                        (5.0 + 0.087 * row as f32) / rows as f32,
                    ]
                })
                .collect(),
        })
        .unwrap();
        let texture = |label, source: &SourceImage| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: cols as u32,
                    height: rows as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                source.pixels(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(cols as u32),
                    rows_per_image: Some(rows as u32),
                },
                texture.size(),
            );
            texture
        };
        let texture_a = texture("direct-bound PIS A", &sources.a);
        let texture_b = texture("direct-bound PIS B", &sources.b);
        let (_, masks) = qualification_fixture();
        let blurred = super::super::temporal::gaussian_blur(
            &sample_source_belts(&sources, &maps).reduce_area_3x3(),
        );
        let retained = ColdInputs::from_blurred_belts_and_masks(blurred, masks.clone());
        let pyramid = MaskPyramid::build(&retained);
        let flight = GpuPisFlight {
            generation: 41,
            frame: FrameStamp::for_test(17, Duration::from_millis(567), None),
        };
        let belt_pipeline = GpuSolverBeltPipeline::new(&device, &queue)
            .unwrap_or_else(|error| panic!("GPU belt qualification failed on {adapter}: {error}"));
        let resident = belt_pipeline
            .submit_resident_retained(
                &device,
                &queue,
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                &maps,
                (),
                flight.clone(),
            )
            .unwrap();
        let front_end = GpuPisFrontEnd::new(&device, &queue).unwrap_or_else(|error| {
            panic!("GPU prepared-source qualification failed on {adapter}: {error}")
        });
        let mut prepared = front_end
            .prepare(&device, &queue, resident, &masks)
            .unwrap();
        GpuPisPipeline::validate_prepared_for_test(&prepared, &flight, Level::Two).unwrap();
        prepared.a_to_b.level_two.receipt.direction = Direction::BtoA;
        assert!(
            GpuPisPipeline::validate_prepared_for_test(&prepared, &flight, Level::Two).is_err(),
            "direction mutation entered the direct kernel"
        );
        prepared.a_to_b.level_two.receipt.direction = Direction::AtoB;
        let saved_flight = prepared.b_to_a.level_two.receipt.flight.clone();
        prepared.b_to_a.level_two.receipt.flight.generation += 1;
        assert!(
            GpuPisPipeline::validate_prepared_for_test(&prepared, &flight, Level::Two).is_err(),
            "flight mutation entered the direct kernel"
        );
        prepared.b_to_a.level_two.receipt.flight = saved_flight;
        prepared.a_to_b.level_two.model_bytes.start += 4;
        assert!(
            GpuPisPipeline::validate_prepared_for_test(&prepared, &flight, Level::Two).is_err(),
            "prepared range mutation entered the direct kernel"
        );
        prepared.a_to_b.level_two.model_bytes.start -= 4;
        assert!(
            validate_direct_stage_for_test(
                PairSolveStage::Cold {
                    calculation: 0,
                    level: Level::Two,
                },
                PairSolveStage::Cold {
                    calculation: 1,
                    level: Level::Two,
                },
            )
            .is_err(),
            "stage mutation entered the direct kernel"
        );
        let pis = GpuPisPipeline::new(&device, &queue)
            .unwrap_or_else(|error| panic!("GPU PIS qualification failed on {adapter}: {error}"));

        for (ordinal, level) in [Level::Two, Level::One].into_iter().enumerate() {
            let a_modes = (0..level.patch_rows())
                .map(|row| {
                    if row % 3 == 0 {
                        CostMode::Weighted
                    } else {
                        CostMode::Unweighted
                    }
                })
                .collect::<Vec<_>>();
            let b_modes = (0..level.patch_rows())
                .map(|row| {
                    if row % 4 < 2 {
                        CostMode::Unweighted
                    } else {
                        CostMode::Weighted
                    }
                })
                .collect::<Vec<_>>();
            let a_input = LevelInputs::build::<AtoB>(&retained, &pyramid, level)
                .input::<AtoB>(level, a_modes)
                .0;
            let b_input = LevelInputs::build::<BtoA>(&retained, &pyramid, level)
                .input::<BtoA>(level, b_modes)
                .0;
            let flows = |salt: usize| {
                (0..level.patches())
                    .map(|patch| {
                        Flow::new(
                            ((7 * patch + salt) % 17) as f32 / 8.0 - 1.0,
                            ((11 * patch + 3 * salt) % 19) as f32 / 9.0 - 1.0,
                        )
                        .unwrap()
                    })
                    .collect::<Vec<_>>()
            };
            let a_seed = flows(1 + ordinal);
            let b_seed = flows(3 + ordinal);
            let a_hint_flows = flows(5 + ordinal);
            let b_hint_flows = flows(7 + ordinal);
            let a_uses_hint = ordinal == 0;
            let b_uses_hint = ordinal != 0;
            let a_admission = if ordinal == 0 {
                DescentAdmission::EveryPatch
            } else {
                DescentAdmission::NoPatches
            };
            let b_admission = if ordinal == 0 {
                DescentAdmission::NoPatches
            } else {
                DescentAdmission::EveryPatch
            };
            let expected_a = super::super::pis::solve_with_descent_admission(
                &a_input,
                InitialGrid::from_test_row_major(level, a_seed.clone()).unwrap(),
                a_uses_hint
                    .then(|| HintGrid::from_row_major(level, a_hint_flows.clone()).unwrap())
                    .as_ref(),
                a_admission,
            )
            .unwrap();
            let expected_b = super::super::pis::solve_with_descent_admission(
                &b_input,
                InitialGrid::from_test_row_major(level, b_seed.clone()).unwrap(),
                b_uses_hint
                    .then(|| HintGrid::from_row_major(level, b_hint_flows.clone()).unwrap())
                    .as_ref(),
                b_admission,
            )
            .unwrap();
            let stage = PairSolveStage::Cold {
                calculation: ordinal,
                level,
            };
            if ordinal == 0 {
                let swapped_shader = DIRECT_TEST_SHADER.replacen(
                    "return f32(prepared_images[offset + row * cols() + col]);",
                    "return f32(prepared_masks[offset + row * cols() + col]);",
                    1,
                );
                assert_ne!(swapped_shader, DIRECT_TEST_SHADER);
                let swapped =
                    GpuPisPipeline::from_shader_for_direct_test(&device, &queue, &swapped_shader)
                        .unwrap();
                let output = swapped
                    .solve_prepared(
                        &device,
                        &queue,
                        prepared,
                        GpuPisStageReceipt {
                            flight: flight.clone(),
                            stage,
                        },
                        GpuPisDynamicStage {
                            stage,
                            a_to_b: GpuPisDynamicDirection::from_oracle(
                                &a_input,
                                InitialGrid::from_test_row_major(level, a_seed.clone()).unwrap(),
                                a_uses_hint.then(|| {
                                    HintGrid::from_row_major(level, a_hint_flows.clone()).unwrap()
                                }),
                                a_admission,
                            ),
                            b_to_a: GpuPisDynamicDirection::from_oracle(
                                &b_input,
                                InitialGrid::from_test_row_major(level, b_seed.clone()).unwrap(),
                                b_uses_hint.then(|| {
                                    HintGrid::from_row_major(level, b_hint_flows.clone()).unwrap()
                                }),
                                b_admission,
                            ),
                        },
                    )
                    .unwrap();
                let (_, swapped_bits, returned) = output.into_parts();
                let differs = swapped_bits
                    .a_to_b
                    .grid
                    .patches()
                    .iter()
                    .zip(expected_a.patches())
                    .any(|(actual, expected)| {
                        actual.flow().dcol().to_bits() != expected.flow().dcol().to_bits()
                            || actual.flow().drow().to_bits() != expected.flow().drow().to_bits()
                    })
                    || swapped_bits
                        .b_to_a
                        .grid
                        .patches()
                        .iter()
                        .zip(expected_b.patches())
                        .any(|(actual, expected)| {
                            actual.flow().dcol().to_bits() != expected.flow().dcol().to_bits()
                                || actual.flow().drow().to_bits()
                                    != expected.flow().drow().to_bits()
                        });
                assert!(differs, "prepared image/mask buffer swap was not detected");
                prepared = returned;
            }
            let output = pis
                .solve_prepared(
                    &device,
                    &queue,
                    prepared,
                    GpuPisStageReceipt {
                        flight: flight.clone(),
                        stage,
                    },
                    GpuPisDynamicStage {
                        stage,
                        a_to_b: GpuPisDynamicDirection::from_oracle(
                            &a_input,
                            InitialGrid::from_test_row_major(level, a_seed).unwrap(),
                            a_uses_hint
                                .then(|| HintGrid::from_row_major(level, a_hint_flows).unwrap()),
                            a_admission,
                        ),
                        b_to_a: GpuPisDynamicDirection::from_oracle(
                            &b_input,
                            InitialGrid::from_test_row_major(level, b_seed).unwrap(),
                            b_uses_hint
                                .then(|| HintGrid::from_row_major(level, b_hint_flows).unwrap()),
                            b_admission,
                        ),
                    },
                )
                .unwrap();
            let (_, actual, returned) = output.into_parts();
            fn assert_terminal<D: PisDirection>(
                direction: &str,
                level: Level,
                actual: &super::super::pis::PatchGrid<D>,
                expected: &super::super::pis::PatchGrid<D>,
            ) {
                for (patch, (actual, expected)) in
                    actual.patches().iter().zip(expected.patches()).enumerate()
                {
                    assert_eq!(
                        [
                            actual.flow().dcol().to_bits(),
                            actual.flow().drow().to_bits()
                        ],
                        [
                            expected.flow().dcol().to_bits(),
                            expected.flow().drow().to_bits()
                        ],
                        "{direction} {level} patch {patch} terminal bits"
                    );
                }
            }
            assert_terminal("A-to-B", level, &actual.a_to_b.grid, &expected_a);
            assert_terminal("B-to-A", level, &actual.b_to_a.grid, &expected_b);
            prepared = returned;
        }
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
