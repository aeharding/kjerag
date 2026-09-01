//! Exact GPU derivation of the row modes consumed by resident PIS.
//!
//! Native classifies only L1. Cold uses each direction's retained lack row;
//! warm ORs that row with the retained bilateral small-disparity row. L2 is
//! the geometric propagation of each maximal L1 run, never a second
//! classifier. One serial invocation owns each direction so the readable CPU
//! schedule is preserved without a cross-workgroup barrier.

use super::{BridgeGpuError, binding, read_buffer_words, scoped_gpu, storage_entry};
use crate::Fallible;
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
use crate::flow::one_xs::pis::Level;
use crate::flow::one_xs::pis::gpu::GpuPisFlight;
use std::error::Error;

const PAIR_SCHEMA: u32 = 0x5049_5301;
const PAIR_HEADER_WORDS: usize = 8;
const DIRECTION_HEADER_WORDS: usize = 32;
const L1_ROWS: usize = 178;
const L2_ROWS: usize = 88;
const ROW_WORDS: usize = 2 * L1_ROWS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkModeKind {
    Cold,
    Warm,
}

enum WorkModeAssociation {
    Flight(GpuPisFlight),
    Qualification,
}

/// Complete private binding for one exact frame and PIS stage.
///
/// It exposes neither its bind group nor any of the three buffers. Ordinary
/// encoding additionally requires the allocation-identical flight carried by
/// the prepared frame.
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuWorkModeBinding {
    context: OneXsGpuContext,
    resources: wgpu::BindGroup,
    kind: WorkModeKind,
    level: Level,
    association: WorkModeAssociation,
}

/// Bridge-owned exact row-mode pipeline.
pub(in crate::flow::one_xs::one_xs_belt_gpu) struct GpuWorkModePipeline {
    context: OneXsGpuContext,
    layout: wgpu::BindGroupLayout,
    cold_l1: wgpu::ComputePipeline,
    cold_l2: wgpu::ComputePipeline,
    warm_l1: wgpu::ComputePipeline,
    warm_l2: wgpu::ComputePipeline,
}

impl GpuWorkModePipeline {
    pub(super) fn new(context: OneXsGpuContext) -> Result<Self, BridgeGpuError> {
        Self::from_shader(context, SHADER)
    }

    fn from_shader(context: OneXsGpuContext, shader: &str) -> Result<Self, BridgeGpuError> {
        let device = context.device();
        let (layout, cold_l1, cold_l2, warm_l1, warm_l2) = scoped_gpu(device, || {
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ONE X2 resident PIS work rows"),
                entries: &[
                    storage_entry(0, false),
                    storage_entry(1, true),
                    storage_entry(2, true),
                ],
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ONE X2 resident PIS work rows"),
                bind_group_layouts: &[&layout],
                immediate_size: 0,
            });
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("ONE X2 resident PIS work rows"),
                source: wgpu::ShaderSource::Wgsl(shader.into()),
            });
            let make = |entry| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("ONE X2 resident PIS work rows"),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            };
            (
                layout,
                make("cold_l1"),
                make("cold_l2"),
                make("warm_l1"),
                make("warm_l2"),
            )
        })?;
        Ok(Self {
            context,
            layout,
            cold_l1,
            cold_l2,
            warm_l1,
            warm_l2,
        })
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn bind_cold(
        &self,
        dynamic: &wgpu::Buffer,
        lack_rows: &wgpu::Buffer,
        level: Level,
        flight: &GpuPisFlight,
    ) -> GpuWorkModeBinding {
        // Cold never reads binding two. Binding the same allocation avoids a
        // dummy owner while the entry point itself keeps small absent.
        self.bind(
            dynamic,
            lack_rows,
            lack_rows,
            WorkModeKind::Cold,
            level,
            WorkModeAssociation::Flight(flight.clone()),
        )
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn bind_warm(
        &self,
        dynamic: &wgpu::Buffer,
        lack_rows: &wgpu::Buffer,
        small_rows: &wgpu::Buffer,
        level: Level,
        flight: &GpuPisFlight,
    ) -> GpuWorkModeBinding {
        self.bind(
            dynamic,
            lack_rows,
            small_rows,
            WorkModeKind::Warm,
            level,
            WorkModeAssociation::Flight(flight.clone()),
        )
    }

    fn bind(
        &self,
        dynamic: &wgpu::Buffer,
        lack_rows: &wgpu::Buffer,
        small_rows: &wgpu::Buffer,
        kind: WorkModeKind,
        level: Level,
        association: WorkModeAssociation,
    ) -> GpuWorkModeBinding {
        let resources = self
            .context
            .device()
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ONE X2 resident PIS work rows"),
                layout: &self.layout,
                entries: &[
                    binding(0, dynamic),
                    binding(1, lack_rows),
                    binding(2, small_rows),
                ],
            });
        GpuWorkModeBinding {
            context: self.context.clone(),
            resources,
            kind,
            level,
            association,
        }
    }

    /// Prove the private pack has the one canonical shape this pass can
    /// overwrite. The production shader has no malformed-input early return:
    /// an invalid pack is rejected before its buffer exists, while a valid
    /// pack always dispatches an exact level-specific writer.
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn validate_dynamic(
        &self,
        words: &[u32],
        level: Level,
    ) -> Fallible<()> {
        validate_work_mode_dynamic(words, level)
    }

    /// Append exact work-row construction to the same command that later
    /// dispatches PIS. No map, copy or CPU re-entry exists on this boundary.
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn encode(
        &self,
        binding: &GpuWorkModeBinding,
        flight: &GpuPisFlight,
        level: Level,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Fallible<()> {
        self.context.ensure_same(&binding.context)?;
        match &binding.association {
            WorkModeAssociation::Flight(bound) if bound == flight => {}
            WorkModeAssociation::Flight(_) => {
                return Err("ONE X2 resident PIS work rows belong to another flight".into());
            }
            WorkModeAssociation::Qualification => {
                return Err("ONE X2 qualification work rows cannot enter resident PIS".into());
            }
        }
        if binding.level != level {
            return Err("ONE X2 resident PIS work rows belong to another level".into());
        }
        self.encode_bound(binding, encoder);
        Ok(())
    }

    fn encode_bound(&self, binding: &GpuWorkModeBinding, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ONE X2 resident PIS work rows"),
            timestamp_writes: None,
        });
        pass.set_pipeline(match (binding.kind, binding.level) {
            (WorkModeKind::Cold, Level::One) => &self.cold_l1,
            (WorkModeKind::Cold, Level::Two) => &self.cold_l2,
            (WorkModeKind::Warm, Level::One) => &self.warm_l1,
            (WorkModeKind::Warm, Level::Two) => &self.warm_l2,
        });
        pass.set_bind_group(0, &binding.resources, &[]);
        pass.dispatch_workgroups(2, 1, 1);
    }

    /// Authenticate both entry points and the complete dynamic-slot contract
    /// once when the resident bridge is constructed. Ordinary frame work does
    /// not copy or map this allocation.
    pub(super) fn qualify(&self) -> Result<(), Box<dyn Error>> {
        for case in qualification_cases() {
            for level in [Level::One, Level::Two] {
                self.qualify_case(&case, level)?;
            }
        }
        Ok(())
    }

    fn qualify_case(&self, case: &QualificationCase, level: Level) -> Result<(), Box<dyn Error>> {
        let before = qualification_dynamic(level);
        let expected_modes = expected_modes(case, level);
        let expected =
            expected_dynamic(&before, &expected_modes).map_err(|error| error.to_string())?;
        let dynamic = qualification_buffer(
            &self.context,
            "ONE X2 resident work-row dynamic qualification",
            &before,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let lack = qualification_buffer(
            &self.context,
            "ONE X2 resident lack-row qualification",
            &case.lack,
            wgpu::BufferUsages::STORAGE,
        );
        let small = qualification_buffer(
            &self.context,
            "ONE X2 resident small-row qualification",
            &case.small,
            wgpu::BufferUsages::STORAGE,
        );
        let binding = self.bind(
            &dynamic,
            &lack,
            &small,
            case.kind,
            level,
            WorkModeAssociation::Qualification,
        );
        let mut encoder =
            self.context
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("ONE X2 resident work-row qualification"),
                });
        self.encode_bound(&binding, &mut encoder);
        self.context.queue().submit([encoder.finish()]);
        let actual = read_buffer_words(&self.context, &dynamic, before.len())?;
        if actual != expected {
            let word = actual
                .iter()
                .zip(&expected)
                .position(|(actual, expected)| actual != expected)
                .unwrap_or(usize::MAX);
            return Err(format!(
                "ONE X2 resident {} work-row qualifier differs at {level} dynamic word {word}",
                case.name
            )
            .into());
        }
        Ok(())
    }

    #[cfg(test)]
    fn qualified_from_shader(
        context: OneXsGpuContext,
        shader: &str,
    ) -> Result<Self, Box<dyn Error>> {
        let pipeline = Self::from_shader(context, shader)?;
        pipeline.qualify()?;
        Ok(pipeline)
    }
}

struct QualificationCase {
    name: &'static str,
    kind: WorkModeKind,
    lack: Vec<u32>,
    small: Vec<u32>,
}

fn validate_work_mode_dynamic(words: &[u32], level: Level) -> Fallible<()> {
    let direction_words = DIRECTION_HEADER_WORDS + level.patch_rows() + 1;
    let expected_len = PAIR_HEADER_WORDS + 2 * direction_words;
    let a_header = PAIR_HEADER_WORDS;
    let b_header = a_header + direction_words;
    if words.len() != expected_len
        || words.first() != Some(&PAIR_SCHEMA)
        || words.get(1) != Some(&2)
        || words.get(2) != Some(&(a_header as u32))
        || words.get(5) != Some(&(b_header as u32))
    {
        return Err(format!(
            "ONE X2 resident {level} PIS work rows have a malformed paired dynamic layout"
        )
        .into());
    }
    for (name, header) in [("A-to-B", a_header), ("B-to-A", b_header)] {
        let mode_base = header + DIRECTION_HEADER_WORDS;
        let mode_end = mode_base + level.patch_rows();
        let relative_end = u32::try_from(mode_end - header)?;
        if words.get(header) != Some(&(level.rows() as u32))
            || words.get(header + 1) != Some(&(level.cols() as u32))
            || words.get(header + 2) != Some(&(level.patch_rows() as u32))
            || words.get(header + 3) != Some(&(level.patch_cols() as u32))
            || words.get(header + 13) != Some(&(DIRECTION_HEADER_WORDS as u32))
            || words.get(header + 22) != Some(&relative_end)
            || words.get(header + 23) != Some(&(relative_end + 1))
            || words.get(mode_end) != Some(&0)
        {
            return Err(format!(
                "ONE X2 resident {level} PIS {name} work rows have a malformed dynamic direction"
            )
            .into());
        }
        if words[mode_base..mode_end].iter().any(|word| *word != 0) {
            return Err(format!(
                "ONE X2 resident {level} PIS {name} work rows were supplied by the CPU"
            )
            .into());
        }
    }
    Ok(())
}

fn qualification_cases() -> Vec<QualificationCase> {
    let rows = |a: &[usize], b: &[usize]| {
        let mut answer = vec![0; ROW_WORDS];
        for &row in a {
            answer[row] = 1;
        }
        for &row in b {
            answer[L1_ROWS + row] = 1;
        }
        answer
    };
    let range = |first: usize, last: usize| (first..=last).collect::<Vec<_>>();
    let mut cold_a = range(18, 27);
    cold_a.extend([0, 79, 177]);
    let mut cold_b = range(53, 60);
    cold_b.extend([0, 84, 177]);
    let mut warm_small_a = range(18, 27);
    warm_small_a.extend((0..L1_ROWS).filter(|row| row % 2 == 0));
    let mut warm_small_b = range(53, 60);
    warm_small_b.extend((0..L1_ROWS).filter(|row| row % 2 == 1));
    vec![
        QualificationCase {
            name: "cold asymmetric",
            kind: WorkModeKind::Cold,
            lack: rows(&cold_a, &cold_b),
            // A planted small vector proves the cold entry never consumes it.
            small: vec![1; ROW_WORDS],
        },
        QualificationCase {
            name: "warm disjoint union",
            kind: WorkModeKind::Warm,
            lack: rows(&[79], &[84]),
            small: rows(&warm_small_a, &warm_small_b),
        },
        QualificationCase {
            name: "all unweighted",
            kind: WorkModeKind::Warm,
            lack: vec![0; ROW_WORDS],
            small: vec![0; ROW_WORDS],
        },
        QualificationCase {
            name: "all weighted",
            kind: WorkModeKind::Cold,
            lack: vec![1; ROW_WORDS],
            small: vec![0; ROW_WORDS],
        },
    ]
}

fn expected_modes(case: &QualificationCase, level: Level) -> [Vec<u32>; 2] {
    std::array::from_fn(|direction| {
        let base = direction * L1_ROWS;
        let l1 = (0..L1_ROWS)
            .map(|row| {
                u32::from(
                    case.lack[base + row] != 0
                        || (case.kind == WorkModeKind::Warm && case.small[base + row] != 0),
                )
            })
            .collect::<Vec<_>>();
        match level {
            Level::One => l1,
            Level::Two => propagate(&l1),
        }
    })
}

/// The READ scalar schedule from `scalar::propagate_work_modes`, expressed on
/// canonical 0/1 words so the construction oracle is independent of PIS.
fn propagate(finest: &[u32]) -> Vec<u32> {
    assert_eq!(finest.len(), L1_ROWS);
    let mut coarse = vec![0; L2_ROWS];
    let mut row = 0;
    while row < finest.len() {
        if finest[row] == 0 {
            row += 1;
            continue;
        }
        let first = row;
        while row + 1 < finest.len() && finest[row + 1] != 0 {
            row += 1;
        }
        let last = row;
        let source_top = (3 * first) as f32;
        let source_height = (3 * (last - first) + 8) as f32;
        let target_top = source_top / 2.0;
        let target_bottom = target_top + source_height / 2.0;
        let top_pixel = ((target_top - 8.0) + 4.0).clamp(0.0, 262.0);
        let bottom_pixel = (target_bottom - 4.0).clamp(0.0, 269.0);
        let first_coarse = ((top_pixel / 3.0).ceil() as usize).min(L2_ROWS - 1);
        let last_coarse = ((bottom_pixel / 3.0).floor() as usize).min(L2_ROWS - 1);
        coarse[first_coarse..=last_coarse].fill(1);
        row += 1;
    }
    coarse
}

fn qualification_dynamic(level: Level) -> Vec<u32> {
    let direction = || {
        let mut header = vec![0xa5a5_a5a5; DIRECTION_HEADER_WORDS];
        header[0] = level.rows() as u32;
        header[1] = level.cols() as u32;
        header[2] = level.patch_rows() as u32;
        header[3] = level.patch_cols() as u32;
        header[13] = DIRECTION_HEADER_WORDS as u32;
        header.extend(std::iter::repeat_n(0xdead_beef, level.patch_rows()));
        header[22] = header.len() as u32;
        header.push(0xc001_c0de);
        header[23] = header.len() as u32;
        header
    };
    let a = direction();
    let b = direction();
    let a_base = PAIR_HEADER_WORDS;
    let b_base = a_base + a.len();
    let mut words = vec![0x1357_9bdf; PAIR_HEADER_WORDS];
    words[0] = PAIR_SCHEMA;
    words[1] = 2;
    words[2] = a_base as u32;
    words[5] = b_base as u32;
    words.extend(a);
    words.extend(b);
    words
}

fn expected_dynamic(before: &[u32], modes: &[Vec<u32>; 2]) -> Fallible<Vec<u32>> {
    let mut expected = before.to_vec();
    for (direction, values) in modes.iter().enumerate() {
        let pair_slot = if direction == 0 { 2 } else { 5 };
        let header = usize::try_from(expected[pair_slot])?;
        let relative = usize::try_from(expected[header + 13])?;
        let first = header + relative;
        let last = first + values.len();
        if last > expected.len() {
            return Err("ONE X2 work-row qualification modes exceed dynamic state".into());
        }
        expected[first..last].copy_from_slice(values);
    }
    Ok(expected)
}

fn qualification_buffer(
    context: &OneXsGpuContext,
    label: &'static str,
    words: &[u32],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    let bytes = words
        .iter()
        .flat_map(|word| word.to_ne_bytes())
        .collect::<Vec<_>>();
    context
        .device()
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: &bytes,
            usage,
        })
}

const SHADER: &str = r#"
@group(0) @binding(0) var<storage, read_write> dynamic_words: array<u32>;
@group(0) @binding(1) var<storage, read> lack_rows: array<u32>;
@group(0) @binding(2) var<storage, read> small_rows: array<u32>;

const L1_ROWS = 178u;
const L2_ROWS = 88u;
const PATCH_SIZE = 8.0;
const PATCH_STRIDE = 3.0;

fn direction_header(direction: u32) -> u32 {
    return dynamic_words[select(2u, 5u, direction != 0u)];
}

fn direction_mode_base(direction: u32) -> u32 {
    let header = direction_header(direction);
    return header + dynamic_words[header + 13u];
}

fn l1_weighted(direction: u32, row: u32, use_small: bool) -> bool {
    let at = direction * L1_ROWS + row;
    return lack_rows[at] != 0u || (use_small && small_rows[at] != 0u);
}

fn write_l1(direction: u32, use_small: bool) {
    let mode_base = direction_mode_base(direction);
    for (var row = 0u; row < L1_ROWS; row += 1u) {
        dynamic_words[mode_base + row] = u32(l1_weighted(direction, row, use_small));
    }
}

fn write_l2(direction: u32, use_small: bool) {
    let mode_base = direction_mode_base(direction);
    for (var coarse = 0u; coarse < L2_ROWS; coarse += 1u) {
        dynamic_words[mode_base + coarse] = 0u;
    }
    var row = 0u;
    while row < L1_ROWS {
        if !l1_weighted(direction, row, use_small) {
            row += 1u;
            continue;
        }
        let first = row;
        while row + 1u < L1_ROWS && l1_weighted(direction, row + 1u, use_small) {
            row += 1u;
        }
        let last = row;

        let source_top = f32(3u * first);
        let source_height = f32(3u * (last - first) + 8u);
        let target_top = source_top / 2.0;
        let target_bottom = target_top + source_height / 2.0;
        let top_pixel = clamp((target_top - PATCH_SIZE) + PATCH_SIZE / 2.0, 0.0, 262.0);
        let bottom_pixel = clamp(target_bottom - PATCH_SIZE / 2.0, 0.0, 269.0);
        let first_coarse = min(u32(ceil(top_pixel / PATCH_STRIDE)), L2_ROWS - 1u);
        let last_coarse = min(u32(floor(bottom_pixel / PATCH_STRIDE)), L2_ROWS - 1u);
        for (var coarse = first_coarse; coarse <= last_coarse; coarse += 1u) {
            dynamic_words[mode_base + coarse] = 1u;
        }
        row += 1u;
    }
}

@compute @workgroup_size(1)
fn cold_l1(@builtin(workgroup_id) id: vec3<u32>) {
    write_l1(id.x, false);
}

@compute @workgroup_size(1)
fn cold_l2(@builtin(workgroup_id) id: vec3<u32>) {
    write_l2(id.x, false);
}

@compute @workgroup_size(1)
fn warm_l1(@builtin(workgroup_id) id: vec3<u32>) {
    write_l1(id.x, true);
}

@compute @workgroup_size(1)
fn warm_l2(@builtin(workgroup_id) id: vec3<u32>) {
    write_l2(id.x, true);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::one_xs::pis::CostMode;
    use crate::flow::one_xs::scalar::propagate_work_modes;

    #[test]
    fn scalar_transcription_matches_every_single_contiguous_run() {
        for first in 0..L1_ROWS {
            for last in first..L1_ROWS {
                let mut words = vec![0; L1_ROWS];
                words[first..=last].fill(1);
                let modes = words
                    .iter()
                    .map(|word| {
                        if *word == 0 {
                            CostMode::Unweighted
                        } else {
                            CostMode::Weighted
                        }
                    })
                    .collect::<Vec<_>>();
                let expected = propagate_work_modes(&modes)
                    .into_iter()
                    .map(|mode| u32::from(mode == CostMode::Weighted))
                    .collect::<Vec<_>>();
                assert_eq!(propagate(&words), expected, "run {first}..={last}");
            }
        }
    }

    #[test]
    fn dynamic_layout_preserves_headers_offsets_and_sentinels() {
        for (level, expected) in [
            (Level::One, (40, 218, 251, 429, 430)),
            (Level::Two, (40, 128, 161, 249, 250)),
        ] {
            let words = qualification_dynamic(level);
            let a_header = words[2] as usize;
            let b_header = words[5] as usize;
            let a_modes = a_header + words[a_header + 13] as usize;
            let b_modes = b_header + words[b_header + 13] as usize;
            assert_eq!(
                (
                    a_modes,
                    a_modes + level.patch_rows(),
                    b_modes,
                    b_modes + level.patch_rows(),
                    words.len(),
                ),
                expected
            );
            assert_eq!(words[a_modes + level.patch_rows()], 0xc001_c0de);
            assert_eq!(words[b_modes + level.patch_rows()], 0xc001_c0de);
        }
    }

    #[test]
    fn malformed_dynamic_layout_and_cpu_modes_are_rejected_before_upload() {
        for level in [Level::One, Level::Two] {
            let canonical = qualification_dynamic(level);
            let mut production = canonical.clone();
            for header in [production[2] as usize, production[5] as usize] {
                let first = header + DIRECTION_HEADER_WORDS;
                let last = first + level.patch_rows();
                production[first..last].fill(0);
                production[last] = 0;
            }
            assert!(validate_work_mode_dynamic(&production, level).is_ok());

            for (name, index, value) in [
                ("schema", 0, 0),
                ("direction count", 1, 1),
                ("A header", 2, 9),
                ("target rows", PAIR_HEADER_WORDS + 2, 177),
                ("mode offset", PAIR_HEADER_WORDS + 13, 31),
            ] {
                let mut malformed = production.clone();
                malformed[index] = value;
                assert!(
                    validate_work_mode_dynamic(&malformed, level).is_err(),
                    "accepted malformed {name} at {level}"
                );
            }

            let a_modes = PAIR_HEADER_WORDS + DIRECTION_HEADER_WORDS;
            let mut supplied = production;
            supplied[a_modes + level.patch_rows() / 2] = 1;
            assert!(
                validate_work_mode_dynamic(&supplied, level).is_err(),
                "accepted CPU-supplied work mode at {level}"
            );
        }
    }

    #[test]
    fn gpu_matches_cold_warm_and_exact_run_oracles() {
        let Some((context, adapter)) = test_gpu("resident work-row qualification") else {
            return;
        };
        eprintln!("ONE X2 work-row qualification adapter: {adapter}");
        let pipeline = GpuWorkModePipeline::new(context)
            .unwrap_or_else(|error| panic!("resident work-row pipeline failed: {error}"));
        pipeline
            .qualify()
            .unwrap_or_else(|error| panic!("resident work-row qualification failed: {error}"));
    }

    #[test]
    fn live_shader_mutations_are_refused() {
        let Some((context, _)) = test_gpu("resident work-row mutations") else {
            return;
        };
        for (name, from, to) in [
            (
                "warm OR",
                "lack_rows[at] != 0u || (use_small && small_rows[at] != 0u)",
                "lack_rows[at] != 0u && (use_small && small_rows[at] != 0u)",
            ),
            (
                "direction ownership",
                "let at = direction * L1_ROWS + row;",
                "let at = (1u - direction) * L1_ROWS + row;",
            ),
            (
                "mode header offset",
                "header + dynamic_words[header + 13u]",
                "header + dynamic_words[header + 13u] + 1u",
            ),
            (
                "run top geometry",
                "(target_top - PATCH_SIZE) + PATCH_SIZE / 2.0",
                "(target_top - PATCH_SIZE) + PATCH_SIZE / 2.0 + 3.0",
            ),
            (
                "run bottom geometry",
                "target_bottom - PATCH_SIZE / 2.0",
                "target_bottom - PATCH_SIZE / 2.0 - 3.0",
            ),
            (
                "target level",
                "fn cold_l2(@builtin(workgroup_id) id: vec3<u32>) {\n    write_l2(id.x, false);",
                "fn cold_l2(@builtin(workgroup_id) id: vec3<u32>) {\n    write_l1(id.x, false);",
            ),
        ] {
            let mutated = SHADER.replacen(from, to, 1);
            assert_ne!(mutated, SHADER, "mutation source exists: {name}");
            assert!(
                GpuWorkModePipeline::qualified_from_shader(context.clone(), &mutated).is_err(),
                "work-row qualifier accepted {name} mutation"
            );
        }
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
            label: Some("ONE X2 work-row qualifier"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        Ok((OneXsGpuContext::new(&device, &queue), name))
    }
}
