//! Sealed GPU prerequisite for the mature warm small-disparity row update.
//!
//! The stage is deliberately not reachable from the cold loop or Scene. A
//! later warm post-L1 owner can give it the exact post-median sparse buffer,
//! common A-side block mask and installed-prior row state, append both passes
//! to that owner's command encoder, and retain the returned typed result.

use std::num::NonZeroU64;

use wgpu::util::DeviceExt;

use crate::Fallible;
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
use crate::flow::one_xs::pis::gpu::GpuPisFlight;
use crate::flow::one_xs_belt_gpu::resident_frame_gpu::GpuResidentIdentity;

const PATCH_ROWS: usize = 178;
const PATCH_COLS: usize = 8;
const PATCHES: usize = PATCH_ROWS * PATCH_COLS;
const FILTERED_WORDS: usize = 2 * PATCHES * 2;
const ROW_WORDS: usize = 2 * PATCH_ROWS;
const CONFIG_WORDS: usize = 3;

/// Arithmetic projection used by constructor qualification and, later, by one
/// exact paused post-L1 owner. It has no provenance fields, so qualification
/// cannot mint a frame or a resident successor.
pub(super) trait GpuSmallRowArithmeticInput: input_owner::Sealed {
    fn context(&self) -> &OneXsGpuContext;
    fn filtered(&self) -> &wgpu::Buffer;
    fn common_a_block_mask(&self) -> &wgpu::Buffer;
    fn prior_rows(&self) -> &wgpu::Buffer;
    fn prior_present(&self) -> bool;
    fn pre_increment_counts(&self) -> [i32; 2];
}

/// Production frame input projection. Only the future exact paused post-L1
/// owner may implement this sealed extension and reach ordinary encoding.
pub(super) trait GpuSmallRowInputOwner: GpuSmallRowArithmeticInput {
    fn producer_flight(&self) -> &GpuPisFlight;
    fn capture_root(&self) -> &GpuResidentIdentity;
}

pub(super) mod input_owner {
    pub(in super::super) trait Sealed {}
}

struct QualificationSmallRowOwner<'a> {
    context: OneXsGpuContext,
    filtered: &'a wgpu::Buffer,
    mask: &'a wgpu::Buffer,
    prior: &'a wgpu::Buffer,
    prior_present: bool,
    counts: [i32; 2],
}

impl input_owner::Sealed for QualificationSmallRowOwner<'_> {}

impl GpuSmallRowArithmeticInput for QualificationSmallRowOwner<'_> {
    fn context(&self) -> &OneXsGpuContext {
        &self.context
    }

    fn filtered(&self) -> &wgpu::Buffer {
        self.filtered
    }

    fn common_a_block_mask(&self) -> &wgpu::Buffer {
        self.mask
    }

    fn prior_rows(&self) -> &wgpu::Buffer {
        self.prior
    }

    fn prior_present(&self) -> bool {
        self.prior_present
    }

    fn pre_increment_counts(&self) -> [i32; 2] {
        self.counts
    }
}

#[cfg(test)]
struct TestSmallRowOwner<'a> {
    arithmetic: QualificationSmallRowOwner<'a>,
    flight: GpuPisFlight,
    root: GpuResidentIdentity,
}

#[cfg(test)]
impl input_owner::Sealed for TestSmallRowOwner<'_> {}

#[cfg(test)]
impl GpuSmallRowArithmeticInput for TestSmallRowOwner<'_> {
    fn context(&self) -> &OneXsGpuContext {
        self.arithmetic.context()
    }

    fn filtered(&self) -> &wgpu::Buffer {
        self.arithmetic.filtered()
    }

    fn common_a_block_mask(&self) -> &wgpu::Buffer {
        self.arithmetic.common_a_block_mask()
    }

    fn prior_rows(&self) -> &wgpu::Buffer {
        self.arithmetic.prior_rows()
    }

    fn prior_present(&self) -> bool {
        self.arithmetic.prior_present()
    }

    fn pre_increment_counts(&self) -> [i32; 2] {
        self.arithmetic.pre_increment_counts()
    }
}

#[cfg(test)]
impl GpuSmallRowInputOwner for TestSmallRowOwner<'_> {
    fn producer_flight(&self) -> &GpuPisFlight {
        &self.flight
    }

    fn capture_root(&self) -> &GpuResidentIdentity {
        &self.root
    }
}

struct GpuSmallRowAllocations {
    rows: wgpu::Buffer,
    present: bool,
    candidates: wgpu::Buffer,
    config: wgpu::Buffer,
    resources: wgpu::BindGroup,
}

/// Exact bilateral successor row owner. `present` remains separate from the
/// allocation because present-all-zero and absent have different native
/// topology even though both produce unweighted work rows.
#[allow(dead_code)]
pub(super) struct GpuResidentSmallRows {
    context: OneXsGpuContext,
    producer_flight: GpuPisFlight,
    root: GpuResidentIdentity,
    rows: wgpu::Buffer,
    present: bool,
    _candidates: wgpu::Buffer,
    _config: wgpu::Buffer,
    _resources: wgpu::BindGroup,
}

#[allow(dead_code)]
impl GpuResidentSmallRows {
    /// Verify the producing frame before a future atomic-install owner accepts
    /// this output. This method does not install it or authorize later use.
    pub(super) fn ensure_producer_identity(
        &self,
        context: &OneXsGpuContext,
        flight: &GpuPisFlight,
        root: &GpuResidentIdentity,
    ) -> Fallible<()> {
        context.ensure_same(&self.context)?;
        if &self.producer_flight != flight {
            return Err(
                "ONE X2 resident successor small rows belong to another producer flight".into(),
            );
        }
        if !self.root.matches(root) {
            return Err("ONE X2 resident small rows belong to another capture root".into());
        }
        Ok(())
    }
}

#[allow(dead_code)]
pub(super) struct GpuSmallRowPipeline {
    context: OneXsGpuContext,
    layout: wgpu::BindGroupLayout,
    classify: wgpu::ComputePipeline,
    merge: wgpu::ComputePipeline,
}

#[allow(dead_code)]
impl GpuSmallRowPipeline {
    pub(super) fn new(context: OneXsGpuContext) -> Fallible<Self> {
        Self::qualified_from_shader(context, SHADER)
    }

    fn qualified_from_shader(context: OneXsGpuContext, shader: &str) -> Fallible<Self> {
        let pipeline = Self::from_shader(context, shader);
        pipeline.qualify().map_err(|error| error.to_string())?;
        Ok(pipeline)
    }

    fn from_shader(context: OneXsGpuContext, shader: &str) -> Self {
        let device = context.device().clone();
        let storage = |binding, read_only, words| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(super::super::super::words_bytes(words)),
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ONE X2 resident mature small rows"),
            entries: &[
                storage(0, true, FILTERED_WORDS),
                storage(1, true, PATCHES),
                storage(2, true, ROW_WORDS),
                storage(3, true, CONFIG_WORDS),
                storage(4, false, ROW_WORDS),
                storage(5, false, ROW_WORDS),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 resident mature small rows"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 resident mature small rows"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let make = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ONE X2 resident mature small rows"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            context,
            layout,
            classify: make("classify_rows"),
            merge: make("merge_rows"),
        }
    }

    pub(super) fn encode<O: GpuSmallRowInputOwner>(
        &self,
        input: &O,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Fallible<GpuResidentSmallRows> {
        let allocations = self.encode_arithmetic(input, encoder, false)?;
        Ok(GpuResidentSmallRows {
            context: self.context.clone(),
            producer_flight: input.producer_flight().clone(),
            root: input.capture_root().clone(),
            rows: allocations.rows,
            present: allocations.present,
            _candidates: allocations.candidates,
            _config: allocations.config,
            _resources: allocations.resources,
        })
    }

    fn encode_arithmetic<I: GpuSmallRowArithmeticInput>(
        &self,
        input: &I,
        encoder: &mut wgpu::CommandEncoder,
        qualification_readback: bool,
    ) -> Fallible<GpuSmallRowAllocations> {
        self.validate(input)?;
        let device = self.context.device();
        let pre_increment_counts = input.pre_increment_counts();
        let prior_present = input.prior_present();
        let config = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ONE X2 resident mature small-row state"),
            contents: &pre_increment_counts
                .map(|count| count.to_ne_bytes())
                .into_iter()
                .flatten()
                .chain(u32::from(prior_present).to_ne_bytes())
                .collect::<Vec<_>>(),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let candidates = storage_buffer(
            device,
            "ONE X2 small-row direction candidates",
            ROW_WORDS,
            false,
        );
        let rows = storage_buffer(
            device,
            "ONE X2 bilateral resident small rows",
            ROW_WORDS,
            qualification_readback,
        );
        let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 resident mature small rows"),
            layout: &self.layout,
            entries: &[
                binding(0, input.filtered()),
                binding(1, input.common_a_block_mask()),
                binding(2, input.prior_rows()),
                binding(3, &config),
                binding(4, &candidates),
                binding(5, &rows),
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 classify mature small rows"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.classify);
            pass.set_bind_group(0, &resources, &[]);
            pass.dispatch_workgroups(PATCH_ROWS as u32, 2, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 merge bilateral small rows"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.merge);
            pass.set_bind_group(0, &resources, &[]);
            pass.dispatch_workgroups(PATCH_ROWS as u32, 1, 1);
        }
        let present = prior_present || pre_increment_counts.into_iter().any(|count| count >= 3);
        Ok(GpuSmallRowAllocations {
            rows,
            present,
            candidates,
            config,
            resources,
        })
    }

    fn validate<I: GpuSmallRowArithmeticInput>(&self, input: &I) -> Fallible<()> {
        self.context.ensure_same(input.context())?;
        for (name, actual, expected) in [
            (
                "post-median sparse field",
                input.filtered().size(),
                super::super::super::words_bytes(FILTERED_WORDS),
            ),
            (
                "common A-side block mask",
                input.common_a_block_mask().size(),
                super::super::super::words_bytes(PATCHES),
            ),
            (
                "prior small rows",
                input.prior_rows().size(),
                super::super::super::words_bytes(ROW_WORDS),
            ),
        ] {
            if actual != expected {
                return Err(
                    format!("ONE X2 {name} buffer is {actual} bytes, expected {expected}").into(),
                );
            }
        }
        Ok(())
    }

    fn qualify(&self) -> Result<(), Box<dyn std::error::Error>> {
        for case in qualification_cases() {
            let actual = self.run_qualification(&case)?;
            let expected = cpu_rows(&case);
            if actual != expected.rows || case.expected_present != expected.present {
                return Err(format!(
                    "ONE X2 mature small-row qualifier disagreed for {}",
                    case.label
                )
                .into());
            }
        }
        Ok(())
    }

    fn run_qualification(
        &self,
        case: &QualificationCase,
    ) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
        let device = self.context.device();
        let filtered = upload_words(
            device,
            "ONE X2 small-row filtered qualifier",
            &case.filtered,
        );
        let mask = upload_words(device, "ONE X2 small-row mask qualifier", &case.mask);
        let prior = upload_words(device, "ONE X2 small-row prior qualifier", &case.prior);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 small-row qualification"),
        });
        let owner = QualificationSmallRowOwner {
            context: self.context.clone(),
            filtered: &filtered,
            mask: &mask,
            prior: &prior,
            prior_present: case.prior_present,
            counts: case.counts,
        };
        let output = self
            .encode_arithmetic(&owner, &mut encoder, true)
            .map_err(|error| error.to_string())?;
        let words = read_words(&self.context, encoder.finish(), &output.rows, ROW_WORDS)?;
        if output.present != case.expected_present {
            return Err(format!("ONE X2 small-row topology disagreed for {}", case.label).into());
        }
        Ok(words)
    }
}

fn binding(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

fn storage_buffer(
    device: &wgpu::Device,
    label: &'static str,
    words: usize,
    copy_src: bool,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: super::super::super::words_bytes(words),
        usage: wgpu::BufferUsages::STORAGE
            | if copy_src {
                wgpu::BufferUsages::COPY_SRC
            } else {
                wgpu::BufferUsages::empty()
            },
        mapped_at_creation: false,
    })
}

fn upload_words(device: &wgpu::Device, label: &'static str, words: &[u32]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: &words
            .iter()
            .flat_map(|word| word.to_ne_bytes())
            .collect::<Vec<_>>(),
        usage: wgpu::BufferUsages::STORAGE,
    })
}

fn read_words(
    context: &OneXsGpuContext,
    command: wgpu::CommandBuffer,
    source: &wgpu::Buffer,
    words: usize,
) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let readback = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("ONE X2 mature small-row qualification readback"),
        size: super::super::super::words_bytes(words),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut copy = context
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 mature small-row qualification copy"),
        });
    copy.copy_buffer_to_buffer(
        source,
        0,
        &readback,
        0,
        super::super::super::words_bytes(words),
    );
    context.queue().submit([command, copy.finish()]);
    let slice = readback.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    context.device().poll(wgpu::PollType::wait_indefinitely())?;
    receiver.recv()??;
    let bytes = slice.get_mapped_range();
    let answer = bytes
        .chunks_exact(4)
        .map(|bytes| u32::from_ne_bytes(bytes.try_into().unwrap()))
        .collect();
    drop(bytes);
    readback.unmap();
    Ok(answer)
}

struct CpuRows {
    rows: Vec<u32>,
    present: bool,
}

fn cpu_rows(case: &QualificationCase) -> CpuRows {
    let mut candidates = vec![0u32; ROW_WORDS];
    let mut present = [case.prior_present; 2];
    for direction in 0..2 {
        if case.counts[direction] >= 3 {
            present[direction] = true;
            for row in 0..PATCH_ROWS {
                let mut sum = 0.0f32;
                let mut count = 0i32;
                for col in 0..PATCH_COLS {
                    let patch = row * PATCH_COLS + col;
                    if case.mask[patch] != 0 {
                        sum +=
                            f32::from_bits(case.filtered[2 * (direction * PATCHES + patch)]).abs();
                        count += 1;
                    }
                }
                let mean = if count == 0 { 0.0 } else { sum / count as f32 };
                candidates[direction * PATCH_ROWS + row] = u32::from(mean < 5.0);
            }
        } else if case.prior_present {
            candidates[direction * PATCH_ROWS..(direction + 1) * PATCH_ROWS]
                .copy_from_slice(&case.prior[direction * PATCH_ROWS..(direction + 1) * PATCH_ROWS]);
        }
    }
    let mut rows = vec![0; ROW_WORDS];
    if present[0] || present[1] {
        for row in 0..PATCH_ROWS {
            let merged = u32::from(candidates[row] != 0 || candidates[PATCH_ROWS + row] != 0);
            rows[row] = merged;
            rows[PATCH_ROWS + row] = merged;
        }
    }
    CpuRows {
        rows,
        present: present[0] || present[1],
    }
}

struct QualificationCase {
    label: &'static str,
    filtered: Vec<u32>,
    mask: Vec<u32>,
    prior: Vec<u32>,
    prior_present: bool,
    counts: [i32; 2],
    expected_present: bool,
}

fn qualification_cases() -> Vec<QualificationCase> {
    let filtered = || vec![0.0f32.to_bits(); FILTERED_WORDS];
    let mask = || vec![255; PATCHES];
    let prior = || vec![0; ROW_WORDS];

    let mut threshold = filtered();
    for col in 0..PATCH_COLS {
        threshold[2 * col] = 5.0f32.to_bits();
        threshold[2 * (PATCHES + col)] = 5.0f32.to_bits();
    }
    let mut asymmetric = filtered();
    for col in 0..PATCH_COLS {
        asymmetric[2 * col] = 7.0f32.to_bits();
        asymmetric[2 * (PATCHES + PATCH_COLS + col)] = 7.0f32.to_bits();
    }
    let mut specials = filtered();
    specials[0] = (-0.0f32).to_bits();
    specials[2] = f32::NAN.to_bits();
    specials[2 * PATCH_COLS] = f32::NAN.to_bits();
    specials[2 * (PATCHES + PATCH_COLS)] = f32::NAN.to_bits();
    let mut specials_mask = mask();
    specials_mask[1] = 0;

    let mut prior_one_sided = prior();
    prior_one_sided[PATCH_ROWS + 9] = 1;
    let mut negative = filtered();
    let mut ordered = filtered();
    let ordered_values: [f32; PATCH_COLS] = [
        4.914_399,
        5.010_006_4,
        4.903_262_6,
        4.995_422_4,
        4.821_018_7,
        5.190_406_3,
        5.056_660_7,
        5.108_82,
    ];
    for direction in 0..2 {
        for col in 0..PATCH_COLS {
            negative[2 * (direction * PATCHES + 2 * PATCH_COLS + col)] = (-7.0f32).to_bits();
            ordered[2 * (direction * PATCHES + 3 * PATCH_COLS + col)] =
                ordered_values[col].to_bits();
        }
    }
    let mut mixed_maturity = filtered();
    for direction in 0..2 {
        for col in 0..PATCH_COLS {
            mixed_maturity[2 * (direction * PATCHES + 4 * PATCH_COLS + col)] = 7.0f32.to_bits();
        }
    }
    let mut mixed_prior = prior();
    mixed_prior[4] = 1;
    vec![
        QualificationCase {
            label: "count two absent",
            filtered: filtered(),
            mask: mask(),
            prior: prior(),
            prior_present: false,
            counts: [2, 2],
            expected_present: false,
        },
        QualificationCase {
            label: "count two present zero",
            filtered: filtered(),
            mask: mask(),
            prior: prior(),
            prior_present: true,
            counts: [2, 2],
            expected_present: true,
        },
        QualificationCase {
            label: "count three materializes",
            filtered: filtered(),
            mask: mask(),
            prior: prior(),
            prior_present: false,
            counts: [3, 3],
            expected_present: true,
        },
        QualificationCase {
            label: "strict threshold",
            filtered: threshold,
            mask: mask(),
            prior: prior(),
            prior_present: false,
            counts: [3, 3],
            expected_present: true,
        },
        QualificationCase {
            label: "direction asymmetry and bilateral merge",
            filtered: asymmetric,
            mask: mask(),
            prior: prior(),
            prior_present: false,
            counts: [3, 3],
            expected_present: true,
        },
        QualificationCase {
            label: "masked NaN active NaN and signed zero",
            filtered: specials,
            mask: specials_mask,
            prior: prior(),
            prior_present: false,
            counts: [3, 3],
            expected_present: true,
        },
        QualificationCase {
            label: "all masked positive-zero mean",
            filtered: filtered(),
            mask: vec![0; PATCHES],
            prior: prior(),
            prior_present: false,
            counts: [3, 3],
            expected_present: true,
        },
        QualificationCase {
            label: "one-sided retained merge",
            filtered: filtered(),
            mask: mask(),
            prior: prior_one_sided,
            prior_present: true,
            counts: [2, 2],
            expected_present: true,
        },
        QualificationCase {
            label: "asymmetric maturity",
            filtered: filtered(),
            mask: mask(),
            prior: prior(),
            prior_present: false,
            counts: [2, 3],
            expected_present: true,
        },
        QualificationCase {
            label: "negative disparity uses absolute magnitude",
            filtered: negative,
            mask: mask(),
            prior: prior(),
            prior_present: false,
            counts: [3, 3],
            expected_present: true,
        },
        QualificationCase {
            label: "serial column order",
            filtered: ordered,
            mask: mask(),
            prior: prior(),
            prior_present: false,
            counts: [3, 3],
            expected_present: true,
        },
        QualificationCase {
            label: "mixed maturity retains only the immature direction",
            filtered: mixed_maturity,
            mask: mask(),
            prior: mixed_prior,
            prior_present: true,
            counts: [2, 3],
            expected_present: true,
        },
        QualificationCase {
            label: "negative signed counts retain present zero",
            filtered: filtered(),
            mask: mask(),
            prior: prior(),
            prior_present: true,
            counts: [i32::MIN, -1],
            expected_present: true,
        },
        QualificationCase {
            label: "maximum signed counts are mature",
            filtered: filtered(),
            mask: mask(),
            prior: prior(),
            prior_present: false,
            counts: [i32::MAX, i32::MAX],
            expected_present: true,
        },
    ]
}

const SHADER: &str = r#"
@group(0) @binding(0) var<storage, read> filtered: array<u32>;
@group(0) @binding(1) var<storage, read> common_a_block_mask: array<u32>;
@group(0) @binding(2) var<storage, read> prior_rows: array<u32>;
@group(0) @binding(3) var<storage, read> state: array<u32>;
@group(0) @binding(4) var<storage, read_write> candidates: array<u32>;
@group(0) @binding(5) var<storage, read_write> output_rows: array<u32>;

const PATCH_ROWS = 178u;
const PATCH_COLS = 8u;
const PATCHES = 1424u;

@compute @workgroup_size(1)
fn classify_rows(@builtin(workgroup_id) id: vec3<u32>) {
    let row = id.x;
    let direction = id.y;
    if (row >= PATCH_ROWS || direction >= 2u) { return; }
    let at = direction * PATCH_ROWS + row;
    let mature = bitcast<i32>(state[direction]) >= 3;
    if (!mature) {
        candidates[at] = select(0u, prior_rows[at], state[2] != 0u);
        return;
    }
    var sum = 0.0;
    var count = 0i;
    for (var column = 0u; column < PATCH_COLS; column += 1u) {
        let patch_index = row * PATCH_COLS + column;
        if (common_a_block_mask[patch_index] != 0u) {
            let dcol = bitcast<f32>(filtered[2u * (direction * PATCHES + patch_index)]);
            sum += abs(dcol);
            count += 1i;
        }
    }
    var mean = 0.0;
    if (count != 0i) {
        mean = sum / f32(count);
    }
    candidates[at] = u32(mean < 5.0);
}

@compute @workgroup_size(1)
fn merge_rows(@builtin(workgroup_id) id: vec3<u32>) {
    let row = id.x;
    if (row >= PATCH_ROWS) { return; }
    let a_present = state[2] != 0u || bitcast<i32>(state[0]) >= 3;
    let b_present = state[2] != 0u || bitcast<i32>(state[1]) >= 3;
    var merged = 0u;
    if (a_present || b_present) {
        merged = u32(candidates[row] != 0u || candidates[PATCH_ROWS + row] != 0u);
    }
    output_rows[row] = merged;
    output_rows[PATCH_ROWS + row] = merged;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_oracle_pins_topology_special_values_and_bilateral_rows() {
        for case in qualification_cases() {
            let actual = cpu_rows(&case);
            assert_eq!(actual.present, case.expected_present, "{}", case.label);
            assert!(actual.rows.iter().all(|word| *word <= 1), "{}", case.label);
            assert_eq!(
                &actual.rows[..PATCH_ROWS],
                &actual.rows[PATCH_ROWS..],
                "{}",
                case.label
            );
        }
        let cases = qualification_cases();
        let case = |label| cases.iter().find(|case| case.label == label).unwrap();
        assert!(!cpu_rows(case("count two absent")).present);
        assert!(cpu_rows(case("count two present zero")).present);
        assert_eq!(
            cpu_rows(case("strict threshold")).rows[0],
            0,
            "mean exactly five is not small"
        );
        assert_eq!(
            cpu_rows(case("masked NaN active NaN and signed zero")).rows[0],
            1,
            "masked NaN is excluded"
        );
        assert_eq!(
            cpu_rows(case("masked NaN active NaN and signed zero")).rows[1],
            0,
            "active NaN is not small"
        );
        assert_eq!(
            cpu_rows(case("all masked positive-zero mean")).rows[0],
            1,
            "all masked has positive-zero mean"
        );
        assert_eq!(
            cpu_rows(case("one-sided retained merge")).rows[9],
            1,
            "one retained side reaches both outputs"
        );
        assert_eq!(
            cpu_rows(case("negative disparity uses absolute magnitude")).rows[2],
            0,
            "negative disparity magnitude is compared"
        );
        assert_eq!(
            cpu_rows(case("serial column order")).rows[3],
            0,
            "the CPU-order f32 sum lands exactly on the threshold"
        );
        assert_eq!(
            cpu_rows(case("mixed maturity retains only the immature direction")).rows[4],
            1,
            "the immature direction retains its prior row before bilateral merge"
        );
    }

    #[test]
    fn gpu_matches_exact_cpu_oracle() {
        let Some((context, adapter)) = test_gpu("mature small-row qualification") else {
            return;
        };
        eprintln!("ONE X2 mature small-row adapter: {adapter}");
        GpuSmallRowPipeline::qualified_from_shader(context, SHADER)
            .unwrap_or_else(|error| panic!("mature small-row qualification failed: {error}"));
    }

    #[test]
    fn live_semantic_mutations_are_refused() {
        let Some((context, _)) = test_gpu("mature small-row mutations") else {
            return;
        };
        for (name, from, to) in [
            ("non-strict threshold", "mean < 5.0", "mean <= 5.0"),
            (
                "wrong mask polarity",
                "common_a_block_mask[patch_index] != 0u",
                "common_a_block_mask[patch_index] == 0u",
            ),
            (
                "one-sided merge",
                "candidates[row] != 0u || candidates[PATCH_ROWS + row] != 0u",
                "candidates[row] != 0u",
            ),
            (
                "late maturity",
                "bitcast<i32>(state[direction]) >= 3",
                "bitcast<i32>(state[direction]) > 3",
            ),
            (
                "include drow",
                "filtered[2u * (direction * PATCHES + patch_index)]",
                "filtered[2u * (direction * PATCHES + patch_index) + 1u]",
            ),
            ("remove absolute value", "sum += abs(dcol);", "sum += dcol;"),
            (
                "swap maturity direction",
                "bitcast<i32>(state[direction]) >= 3",
                "bitcast<i32>(state[1u - direction]) >= 3",
            ),
            (
                "unsigned maturity",
                "bitcast<i32>(state[direction]) >= 3",
                "state[direction] >= 3u",
            ),
            (
                "reverse column traversal",
                "row * PATCH_COLS + column",
                "row * PATCH_COLS + (PATCH_COLS - 1u - column)",
            ),
        ] {
            let mutated = SHADER.replace(from, to);
            assert_ne!(mutated, SHADER, "mutation source exists: {name}");
            assert!(
                GpuSmallRowPipeline::qualified_from_shader(context.clone(), &mutated).is_err(),
                "small-row qualifier accepted {name} mutation"
            );
        }
    }

    #[test]
    fn sealed_owner_checks_pipeline_context_and_output_producer_identity() {
        let Some((context, foreign_context, _)) = test_gpu_pair("mature small-row provenance")
        else {
            return;
        };
        let pipeline = GpuSmallRowPipeline::new(context.clone()).unwrap();
        let device = context.device();
        let filtered = upload_words(
            device,
            "small-row provenance filtered",
            &vec![0; FILTERED_WORDS],
        );
        let mask = upload_words(device, "small-row provenance mask", &vec![255; PATCHES]);
        let prior = upload_words(device, "small-row provenance prior", &vec![0; ROW_WORDS]);
        let capture = crate::flow::one_xs_belt_gpu::resident_frame_gpu::GpuResidentCapture::new();
        let foreign_capture =
            crate::flow::one_xs_belt_gpu::resident_frame_gpu::GpuResidentCapture::new();
        let reservation = capture
            .reserve(kjerag_media::FrameStamp::for_test(
                21,
                std::time::Duration::from_millis(700),
                None,
            ))
            .unwrap();
        let foreign = foreign_capture
            .reserve(kjerag_media::FrameStamp::for_test(
                21,
                std::time::Duration::from_millis(700),
                None,
            ))
            .unwrap();
        let flight = reservation.flight().clone();
        let root = reservation.identity();
        let owner = TestSmallRowOwner {
            flight: flight.clone(),
            root: root.clone(),
            arithmetic: QualificationSmallRowOwner {
                context: context.clone(),
                filtered: &filtered,
                mask: &mask,
                prior: &prior,
                prior_present: false,
                counts: [3, 3],
            },
        };
        assert!(pipeline.validate(&owner).is_ok());
        let foreign_context_owner = QualificationSmallRowOwner {
            context: foreign_context,
            filtered: &filtered,
            mask: &mask,
            prior: &prior,
            prior_present: false,
            counts: [3, 3],
        };
        assert!(
            pipeline.validate(&foreign_context_owner).is_err(),
            "a foreign pipeline context must be refused before encoding"
        );

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 small-row sealed-output identity test"),
        });
        let output = pipeline.encode(&owner, &mut encoder).unwrap();
        assert!(
            output
                .ensure_producer_identity(&context, &flight, &root)
                .is_ok()
        );
        assert!(
            output
                .ensure_producer_identity(&context, foreign.flight(), &root)
                .is_err(),
            "future attachment must reject a foreign producer flight"
        );
        assert!(
            output
                .ensure_producer_identity(&context, &flight, &foreign.identity())
                .is_err(),
            "future attachment must reject a foreign capture root"
        );
        reservation.abort().unwrap();
        foreign.abort().unwrap();
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            match future.as_mut().poll(&mut context) {
                std::task::Poll::Ready(answer) => return answer,
                std::task::Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn test_gpu(label: &str) -> Option<(OneXsGpuContext, String)> {
        match gpu() {
            Ok(gpu) => Some(gpu),
            Err(why)
                if std::env::var_os("KJERAG_REQUIRE_GPU").is_none()
                    && std::env::var_os("KJERAG_REQUIRE_RADV").is_none() =>
            {
                eprintln!("skipping ONE X2 {label}: {why}");
                None
            }
            Err(why) => panic!("GPU required for ONE X2 {label}: {why}"),
        }
    }

    fn test_gpu_pair(label: &str) -> Option<(OneXsGpuContext, OneXsGpuContext, String)> {
        match gpu_pair() {
            Ok(gpu) => Some(gpu),
            Err(why)
                if std::env::var_os("KJERAG_REQUIRE_GPU").is_none()
                    && std::env::var_os("KJERAG_REQUIRE_RADV").is_none() =>
            {
                eprintln!("skipping ONE X2 {label}: {why}");
                None
            }
            Err(why) => panic!("GPU required for ONE X2 {label}: {why}"),
        }
    }

    fn gpu() -> Result<(OneXsGpuContext, String), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let require_radv = std::env::var_os("KJERAG_REQUIRE_RADV").is_some();
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.device_type != wgpu::DeviceType::Cpu
                    && (!require_radv || info.driver.eq_ignore_ascii_case("radv"))
            })
            .ok_or(if require_radv {
                "no non-software RADV Vulkan adapter"
            } else {
                "no non-software Vulkan adapter"
            })?;
        let info = adapter.get_info();
        let name = format!("{} / {} / {}", info.name, info.driver, info.driver_info);
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ONE X2 mature small-row qualifier"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        Ok((OneXsGpuContext::new(&device, &queue), name))
    }

    fn gpu_pair() -> Result<(OneXsGpuContext, OneXsGpuContext, String), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let require_radv = std::env::var_os("KJERAG_REQUIRE_RADV").is_some();
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .find(|adapter| {
                let info = adapter.get_info();
                info.device_type != wgpu::DeviceType::Cpu
                    && (!require_radv || info.driver.eq_ignore_ascii_case("radv"))
            })
            .ok_or(if require_radv {
                "no non-software RADV Vulkan adapter"
            } else {
                "no non-software Vulkan adapter"
            })?;
        let info = adapter.get_info();
        let name = format!("{} / {} / {}", info.name, info.driver, info.driver_info);
        let request = || {
            block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("ONE X2 mature small-row provenance pair"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            }))
            .map_err(|error| error.to_string())
        };
        let (device, queue) = request()?;
        let (foreign_device, foreign_queue) = request()?;
        Ok((
            OneXsGpuContext::new(&device, &queue),
            OneXsGpuContext::new(&foreign_device, &foreign_queue),
            name,
        ))
    }
}
