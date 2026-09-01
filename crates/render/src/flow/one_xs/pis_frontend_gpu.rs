//! GPU-resident image-owned PIS preparation checkpoint.
//!
//! This first vertical slice consumes the exact packed post-Gaussian solver
//! belts without mapping them into CPU memory and produces the complete
//! prepared-source model for A-to-B level two. The readable CPU `Input` and
//! `PairedPisSolver` remain the oracle. Scene does not select this incomplete
//! path.

use std::marker::PhantomData;
use std::sync::mpsc;

use super::pis::gpu::GpuPisFlight;
use super::pis::{AtoB, Level, PisDirection};
use super::scalar::{ColdInputs, LevelInputs, MaskPyramid};
use super::temporal::BlurredBelts;
use super::{COLS, Direction, LensPair, ROWS};
use crate::Fallible;
use crate::flow::one_xs_belt::SolverBelts;
use crate::flow::one_xs_belt_gpu::GpuBlurredBelts;

const MODEL_WORDS_PER_PATCH: usize = 5;
const OUTPUT_WORDS: usize = Level::Two.patches() * MODEL_WORDS_PER_PATCH;
const OUTPUT_BYTES: u64 = (OUTPUT_WORDS * size_of::<u32>()) as u64;
const MASK_WORDS: usize = (ROWS * COLS).div_ceil(4);

/// Exact identity of one GPU-resident image-owned preprocessing result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GpuPisFrontEndReceipt {
    pub(crate) flight: GpuPisFlight,
    pub(crate) direction: Direction,
    pub(crate) level: Level,
}

/// Complete source model for one exact direction and level, still resident on
/// the device and ready for a later GPU PIS adapter.
#[must_use = "the GPU-resident prepared source model has not been consumed"]
pub(crate) struct GpuPreparedSourceModel<K, D: PisDirection> {
    receipt: GpuPisFrontEndReceipt,
    models: wgpu::Buffer,
    mask: wgpu::Buffer,
    resources: wgpu::BindGroup,
    belts: GpuBlurredBelts<K>,
    submission: wgpu::SubmissionIndex,
    direction: PhantomData<D>,
}

impl<K, D: PisDirection> GpuPreparedSourceModel<K, D> {
    pub(crate) fn receipt(&self) -> &GpuPisFrontEndReceipt {
        &self.receipt
    }

    pub(crate) fn models(&self) -> &wgpu::Buffer {
        &self.models
    }

    pub(crate) fn submission(&self) -> &wgpu::SubmissionIndex {
        &self.submission
    }
}

/// Render-private production shader plus mandatory target-device CPU twin.
pub(crate) struct GpuPisFrontEnd {
    pipeline: wgpu::ComputePipeline,
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
            entries: &[storage(0, true), storage(1, true), storage(2, false)],
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
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ONE X2 GPU PIS prepared-source front end"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("prepare_l2_a_to_b"),
            compilation_options: Default::default(),
            cache: None,
        });
        let built = Self { pipeline, layout };
        if qualify {
            built.qualify(device, queue)?;
        }
        Ok(built)
    }

    /// Consume one exact no-readback belt token and enqueue its complete
    /// A-to-B L2 prepared-source model. Same-queue ordering makes the producer
    /// visible without a CPU poll.
    pub(crate) fn prepare_l2_a_to_b<K>(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        belts: GpuBlurredBelts<K>,
        physical_masks: &LensPair<Vec<u8>>,
    ) -> Fallible<GpuPreparedSourceModel<K, AtoB>> {
        validate_masks(physical_masks)?;
        let mask = upload_mask(device, queue, &physical_masks.a);
        let models = output_buffer(device);
        let resources = self.resources(device, belts.packed(), &mask, &models);
        let submission = self.dispatch(device, queue, &resources);
        let receipt = GpuPisFrontEndReceipt {
            flight: belts.flight().clone(),
            direction: Direction::AtoB,
            level: Level::Two,
        };
        Ok(GpuPreparedSourceModel {
            receipt,
            models,
            mask,
            resources,
            belts,
            submission,
            direction: PhantomData,
        })
    }

    fn resources(
        &self,
        device: &wgpu::Device,
        belts: &wgpu::Buffer,
        mask: &wgpu::Buffer,
        models: &wgpu::Buffer,
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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: models.as_entire_binding(),
                },
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
        encode(&mut encoder, &self.pipeline, resources);
        queue.submit([encoder.finish()])
    }

    fn qualify(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Fallible<()> {
        let (blurred, masks) = qualification_fixture();
        let expected = cpu_models(&blurred, &masks);
        let packed = upload_belts(device, queue, &blurred);
        let mask = upload_mask(device, queue, &masks.a);
        let models = output_buffer(device);
        let resources = self.resources(device, &packed, &mask, &models);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 GPU PIS prepared-source qualification"),
        });
        encode(&mut encoder, &self.pipeline, &resources);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 GPU PIS prepared-source qualification readback"),
            size: OUTPUT_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&models, 0, &readback, 0, OUTPUT_BYTES);
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
        let mismatch = bytes
            .chunks_exact(4)
            .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
            .zip(expected.into_iter().flatten())
            .enumerate()
            .find(|(_, (actual, expected))| actual != expected);
        drop(bytes);
        readback.unmap();
        if let Some((word, (actual, expected))) = mismatch {
            return Err(format!(
                "ONE X2 GPU PIS prepared-source arithmetic is not exact on this graphics device: A-to-B level 2 patch {} component {} bits are {actual:#010x}, expected {expected:#010x}",
                word / MODEL_WORDS_PER_PATCH,
                word % MODEL_WORDS_PER_PATCH,
            )
            .into());
        }
        Ok(())
    }
}

fn encode(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::ComputePipeline,
    resources: &wgpu::BindGroup,
) {
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("ONE X2 GPU PIS prepared-source front end"),
        timestamp_writes: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, resources, &[]);
    pass.dispatch_workgroups(Level::Two.patches() as u32, 1, 1);
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

fn output_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ONE X2 GPU PIS prepared-source models"),
        size: OUTPUT_BYTES,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn upload_mask(device: &wgpu::Device, queue: &wgpu::Queue, mask: &[u8]) -> wgpu::Buffer {
    // The trailing runtime-zero word pins the CPU model's explicit binary32
    // round boundaries without changing the physical mask payload.
    let mut words = vec![0u32; MASK_WORDS + 1];
    for (index, value) in mask.iter().copied().enumerate() {
        words[index / 4] |= u32::from(value) << (8 * (index % 4));
    }
    upload_words(device, queue, "ONE X2 GPU PIS physical mask A", &words)
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

fn cpu_models(blurred: &BlurredBelts, masks: &LensPair<Vec<u8>>) -> Vec<[u32; 5]> {
    let retained = ColdInputs::from_prepared(blurred.clone().into_lenses(), masks.clone())
        .expect("qualification fixture has the retained shape");
    let pyramid = MaskPyramid::build(&retained);
    LevelInputs::build::<AtoB>(&retained, &pyramid, Level::Two)
        .prepared_source_model_bits::<AtoB>(Level::Two)
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
const ROWS = 270u;
const COLS = 15u;
const PATCH_ROWS = 88u;
const PATCH_COLS = 3u;
const PATCH_SIZE = 8u;
const PATCH_STRIDE = 3u;
const MODEL_WORDS = 5u;

@group(0) @binding(0) var<storage, read> blurred_words: array<u32>;
@group(0) @binding(1) var<storage, read> mask_words: array<u32>;
@group(0) @binding(2) var<storage, read_write> model_bits: array<u32>;

fn code(index: u32) -> u32 {
    return (blurred_words[index / 4u] >> (8u * (index % 4u))) & 255u;
}

fn retained_mask(index: u32) -> u32 {
    return (mask_words[index / 4u] >> (8u * (index % 4u))) & 255u;
}

fn rounded_average4(a: u32, b: u32, c: u32, d: u32) -> u32 {
    return (a + b + c + d + 2u) / 4u;
}

fn l1_code(row: u32, col: u32) -> u32 {
    let r = row * 2u;
    let c = col * 2u;
    return rounded_average4(
        code(r * RETAINED_COLS + c),
        code(r * RETAINED_COLS + c + 1u),
        code((r + 1u) * RETAINED_COLS + c),
        code((r + 1u) * RETAINED_COLS + c + 1u),
    );
}

fn l2_code(row: u32, col: u32) -> u32 {
    let r = row * 2u;
    let c = col * 2u;
    return rounded_average4(
        l1_code(r, c),
        l1_code(r, c + 1u),
        l1_code(r + 1u, c),
        l1_code(r + 1u, c + 1u),
    );
}

fn reflect_101(value: i32, extent: i32) -> u32 {
    if value < 0 { return u32(-value); }
    if value >= extent { return u32(2 * extent - value - 2); }
    return u32(value);
}

fn materialize(value: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(value) ^ mask_words[RETAINED_MASK_WORDS]);
}

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

fn gradient(row: u32, col: u32) -> vec2<f32> {
    if retained_mask((row * 4u) * RETAINED_COLS + col * 4u) == 0u {
        return vec2<f32>(0.0);
    }
    var gx = 0.0;
    var gy = 0.0;
    for (var dr = 0u; dr < 3u; dr += 1u) {
        let rr = reflect_101(i32(row) + i32(dr) - 1, i32(ROWS));
        let sy = select(1.0, 2.0, dr == 1u);
        let dy = f32(i32(dr) - 1);
        for (var dc = 0u; dc < 3u; dc += 1u) {
            let cc = reflect_101(i32(col) + i32(dc) - 1, i32(COLS));
            let sx = select(1.0, 2.0, dc == 1u);
            let dx = f32(i32(dc) - 1);
            let value = f32(l2_code(rr, cc));
            gx = add_rn(gx, mul_rn(mul_rn(value, dx), sy));
            gy = add_rn(gy, mul_rn(mul_rn(value, sx), dy));
        }
    }
    return vec2<f32>(gx, gy);
}

@compute @workgroup_size(1)
fn prepare_l2_a_to_b(@builtin(global_invocation_id) id: vec3<u32>) {
    let patch_index = id.x;
    if patch_index >= PATCH_ROWS * PATCH_COLS { return; }
    let origin_row = (patch_index / PATCH_COLS) * PATCH_STRIDE;
    let origin_col = (patch_index % PATCH_COLS) * PATCH_STRIDE;
    var sums = array<f32, 5>(0.0, 0.0, 0.0, 0.0, 0.0);
    for (var row = 0u; row < PATCH_SIZE; row += 1u) {
        for (var col = 0u; col < PATCH_SIZE; col += 1u) {
            let g = gradient(origin_row + row, origin_col + col);
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
"#;

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::*;

    #[test]
    fn fixture_exercises_every_source_model_component() {
        let (blurred, masks) = qualification_fixture();
        let models = cpu_models(&blurred, &masks);
        assert_eq!(models.len(), Level::Two.patches());
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
                "physical source lens",
                "code(r * RETAINED_COLS + c),",
                "code(RETAINED_PIXELS + r * RETAINED_COLS + c),",
            ),
            (
                "physical mask A",
                "if retained_mask((row * 4u) * RETAINED_COLS + col * 4u) == 0u {",
                "if retained_mask((row * 4u) * RETAINED_COLS + col * 4u) == 255u {",
            ),
            (
                "reflect-101 Sobel border",
                "if value < 0 { return u32(-value); }",
                "if value < 0 { return 0u; }",
            ),
            (
                "positive determinant clamp",
                "if abs(determinant) < 0.001 { determinant = 0.001; }",
                "if abs(determinant) < 0.001 { determinant = 0.002; }",
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
                    .contains("prepared-source arithmetic is not exact"),
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
