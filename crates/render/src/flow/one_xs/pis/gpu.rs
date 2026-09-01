//! Exact paired GPU boundary for the selected ONE X2 patch inverse search.
//!
//! CPU code admits [`Input`], [`InitialGrid`], [`HintGrid`] and
//! [`DescentAdmission`] at their existing typed boundary, then serializes an
//! explicit `u32`/`f32` storage contract. One 32-lane workgroup owns each
//! direction, advances the native in-place passes by anti-diagonal, and assigns
//! four lanes to each active patch's native candidate reduction. Construction
//! qualifies this same paired production entry against both CPU directions on
//! the actual adapter.

use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::sync::mpsc;

use super::{
    AtoB, BtoA, CostMode, DescentAdmission, Direction, Flow, HintGrid, InitialGrid, Input, Level,
    PisDirection, solve_with_descent_admission,
};
use crate::Fallible;

const HEADER_WORDS: usize = 32;
const OUTPUT_WORDS_PER_PATCH: usize = 2;
const DIAGNOSTIC_WORDS: usize = 0;

/// Exact terminal component bits for one direction and selected level.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TerminalBits<D: PisDirection> {
    level: Level,
    dcol: Box<[u32]>,
    drow: Box<[u32]>,
    diagnostics: Box<[u32]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> TerminalBits<D> {
    pub(crate) const fn level(&self) -> Level {
        self.level
    }

    pub(crate) fn dcol(&self) -> &[u32] {
        &self.dcol
    }

    pub(crate) fn drow(&self) -> &[u32] {
        &self.drow
    }

    /// Re-enter the typed CPU boundary without changing any terminal bits.
    /// Pass reports are intentionally empty because this production handoff
    /// retains only the native terminal grid.
    pub(crate) fn into_patch_grid(self) -> Result<super::PatchGrid<D>, TerminalGridError> {
        let expected = self.level.patches();
        for (component, actual) in [("dcol", self.dcol.len()), ("drow", self.drow.len())] {
            if actual != expected {
                return Err(TerminalGridError::Shape {
                    direction: D::DIRECTION,
                    level: self.level,
                    component,
                    expected,
                    actual,
                });
            }
        }
        let patches = self
            .dcol
            .iter()
            .copied()
            .zip(self.drow.iter().copied())
            .enumerate()
            .map(|(patch, (dcol, drow))| {
                for (component, bits) in [("dcol", dcol), ("drow", drow)] {
                    if !f32::from_bits(bits).is_finite() {
                        return Err(TerminalGridError::NonFinite {
                            direction: D::DIRECTION,
                            level: self.level,
                            patch,
                            component,
                            bits,
                        });
                    }
                }
                let flow = Flow::new(f32::from_bits(dcol), f32::from_bits(drow)).ok_or(
                    TerminalGridError::NonFinite {
                        direction: D::DIRECTION,
                        level: self.level,
                        patch,
                        component: "flow",
                        bits: dcol,
                    },
                )?;
                Ok(super::Patch::seeded(flow))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice();
        Ok(super::PatchGrid {
            level: self.level,
            patches,
            direction: PhantomData,
        })
    }
}

/// A terminal GPU bit grid could not safely re-enter the typed CPU boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalGridError {
    Shape {
        direction: Direction,
        level: Level,
        component: &'static str,
        expected: usize,
        actual: usize,
    },
    NonFinite {
        direction: Direction,
        level: Level,
        patch: usize,
        component: &'static str,
        bits: u32,
    },
}

impl fmt::Display for TerminalGridError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shape {
                direction,
                level,
                component,
                expected,
                actual,
            } => write!(
                out,
                "ONE X2 GPU PIS {direction:?} {level} terminal {component} has {actual} patches, expected {expected}"
            ),
            Self::NonFinite {
                direction,
                level,
                patch,
                component,
                bits,
            } => write!(
                out,
                "ONE X2 GPU PIS {direction:?} {level} patch {patch} terminal {component} bits {bits:#010x} are not finite"
            ),
        }
    }
}

impl Error for TerminalGridError {}

/// The two direction-typed terminal grids from one paired level solve.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PairedTerminalBits {
    pub(crate) a_to_b: TerminalBits<AtoB>,
    pub(crate) b_to_a: TerminalBits<BtoA>,
}

#[derive(Debug, PartialEq, Eq)]
enum QualificationError {
    Direction {
        direction: &'static str,
        level: Level,
        patch: usize,
        component: &'static str,
        actual: u32,
        expected: u32,
        actual_diagnostics: Box<[u32]>,
        expected_candidates: Box<[u32]>,
    },
}

impl fmt::Display for QualificationError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direction {
                direction,
                level,
                patch,
                component,
                actual,
                expected,
                actual_diagnostics,
                expected_candidates,
            } => write!(
                out,
                "ONE X2 GPU PIS arithmetic is not exact on this graphics device: {direction} {level} patch {patch} {component} bits are {actual:#010x}, expected {expected:#010x}; GPU patch-1 diagnostics {actual_diagnostics:#010x?}; CPU pass-0 candidate score bits {expected_candidates:#010x?}",
            ),
        }
    }
}

impl Error for QualificationError {}

/// Render-internal paired PIS compute state.
pub(crate) struct GpuPisPipeline {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl GpuPisPipeline {
    /// Build and qualify the actual production shader entry on this device.
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
            label: Some("ONE X2 paired GPU PIS"),
            entries: &[storage(0, true), storage(1, true), storage(2, false)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 paired GPU PIS"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 paired GPU PIS"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ONE X2 paired GPU PIS"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("solve_pis_wavefront"),
            compilation_options: Default::default(),
            cache: None,
        });
        let built = Self { pipeline, layout };
        if qualify {
            built.qualify(device, queue)?;
        }
        Ok(built)
    }

    /// Execute both native directions at one level. Inputs have already passed
    /// the CPU constructor's shape, finite, mask and rolling-denominator gates.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn solve_pair(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        a_to_b: &Input<AtoB>,
        a_initial: InitialGrid<AtoB>,
        a_hint: Option<&HintGrid<AtoB>>,
        b_to_a: &Input<BtoA>,
        b_initial: InitialGrid<BtoA>,
        b_hint: Option<&HintGrid<BtoA>>,
        a_admission: DescentAdmission,
        b_admission: DescentAdmission,
    ) -> Fallible<PairedTerminalBits> {
        if a_to_b.level != b_to_a.level {
            return Err(format!(
                "ONE X2 paired GPU PIS directions have different levels: {} and {}",
                a_to_b.level, b_to_a.level
            )
            .into());
        }
        let a = PackedInput::new(a_to_b, a_initial, a_hint, a_admission)?;
        let b = PackedInput::new(b_to_a, b_initial, b_hint, b_admission)?;
        self.dispatch_pair(device, queue, PackedPair::new(a, b)?)
    }

    fn dispatch_pair(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        packed: PackedPair,
    ) -> Fallible<PairedTerminalBits> {
        let u32_buffer = upload(
            device,
            queue,
            "ONE X2 GPU PIS u32 input",
            u32_bytes(&packed.u32s),
        );
        let f32_buffer = upload(
            device,
            queue,
            "ONE X2 GPU PIS f32 input",
            f32_bytes(&packed.f32s),
        );
        let output_size = (packed.output_words * 4) as u64;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 GPU PIS terminal bits"),
            size: output_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 GPU PIS terminal readback"),
            size: output_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 GPU PIS resources"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: u32_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: f32_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 paired GPU PIS"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 paired GPU PIS direction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &resources, &[]);
            pass.dispatch_workgroups(2, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output_size);
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
            .chunks_exact(4)
            .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
            .collect::<Vec<_>>();
        drop(bytes);
        readback.unmap();
        Ok(PairedTerminalBits {
            a_to_b: decode_terminal(
                packed.level,
                &words[packed.a_output_base..packed.a_output_base + packed.output_span],
            )?,
            b_to_a: decode_terminal(
                packed.level,
                &words[packed.b_output_base..packed.b_output_base + packed.output_span],
            )?,
        })
    }

    fn qualify(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Fallible<()> {
        self.qualify_case(
            device,
            queue,
            Level::Two,
            true,
            DescentAdmission::EveryPatch,
            DescentAdmission::EveryPatch,
        )?;
        self.qualify_case(
            device,
            queue,
            Level::One,
            true,
            DescentAdmission::NoPatches,
            DescentAdmission::NoPatches,
        )?;
        self.qualify_case(
            device,
            queue,
            Level::Two,
            false,
            DescentAdmission::NoPatches,
            DescentAdmission::EveryPatch,
        )
    }

    fn qualify_case(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        level: Level,
        use_hint: bool,
        a_admission: DescentAdmission,
        b_admission: DescentAdmission,
    ) -> Fallible<()> {
        let (a_input, b_input, a_initial, b_initial, a_hint, b_hint) = qualification_fixture(level);
        let a_hint_ref = use_hint.then_some(&a_hint);
        let b_hint_ref = use_hint.then_some(&b_hint);
        let expected_a = solve_with_descent_admission(
            &a_input,
            clone_initial(&a_initial),
            a_hint_ref,
            a_admission,
        )?;
        let expected_b = solve_with_descent_admission(
            &b_input,
            clone_initial(&b_initial),
            b_hint_ref,
            b_admission,
        )?;
        let actual = self.solve_pair(
            device,
            queue,
            &a_input,
            a_initial,
            a_hint_ref,
            &b_input,
            b_initial,
            b_hint_ref,
            a_admission,
            b_admission,
        )?;
        let actual_a = actual.a_to_b.into_patch_grid()?;
        let actual_b = actual.b_to_a.into_patch_grid()?;
        compare("A-to-B", &actual_a, &expected_a)?;
        compare("B-to-A", &actual_b, &expected_b)?;
        Ok(())
    }
}

fn compare<D: PisDirection>(
    direction: &'static str,
    actual: &super::PatchGrid<D>,
    expected: &super::PatchGrid<D>,
) -> Fallible<()> {
    let expected_candidates = expected
        .patches()
        .get(1)
        .map(|patch| {
            patch.passes()[0]
                .candidates()
                .into_iter()
                .flatten()
                .map(|candidate| candidate.score().value().to_bits())
                .collect::<Vec<_>>()
                .into_boxed_slice()
        })
        .unwrap_or_default();
    for (patch, (actual_bits, expected_bits)) in actual
        .patches()
        .iter()
        .map(|patch| patch.flow().dcol().to_bits())
        .zip(
            expected
                .patches()
                .iter()
                .map(|patch| patch.flow().dcol().to_bits()),
        )
        .enumerate()
    {
        if actual_bits != expected_bits {
            return Err(QualificationError::Direction {
                direction,
                level: actual.level(),
                patch,
                component: "dcol",
                actual: actual_bits,
                expected: expected_bits,
                actual_diagnostics: Box::default(),
                expected_candidates: expected_candidates.clone(),
            }
            .into());
        }
    }
    for (patch, (actual_bits, expected_bits)) in actual
        .patches()
        .iter()
        .map(|patch| patch.flow().drow().to_bits())
        .zip(
            expected
                .patches()
                .iter()
                .map(|patch| patch.flow().drow().to_bits()),
        )
        .enumerate()
    {
        if actual_bits != expected_bits {
            return Err(QualificationError::Direction {
                direction,
                level: actual.level(),
                patch,
                component: "drow",
                actual: actual_bits,
                expected: expected_bits,
                actual_diagnostics: Box::default(),
                expected_candidates: expected_candidates.clone(),
            }
            .into());
        }
    }
    Ok(())
}

fn clone_initial<D: PisDirection>(initial: &InitialGrid<D>) -> InitialGrid<D> {
    InitialGrid {
        level: initial.level,
        flows: initial.flows.clone(),
        direction: PhantomData,
    }
}

struct PackedInput<D: PisDirection> {
    level: Level,
    u32s: Vec<u32>,
    f32s: Vec<f32>,
    direction: PhantomData<D>,
}

const PAIR_HEADER_WORDS: usize = 8;
const PAIR_SCHEMA: u32 = 0x5049_5301;

struct PackedPair {
    level: Level,
    u32s: Vec<u32>,
    f32s: Vec<f32>,
    output_words: usize,
    output_span: usize,
    a_output_base: usize,
    b_output_base: usize,
}

impl PackedPair {
    fn new(a: PackedInput<AtoB>, b: PackedInput<BtoA>) -> Fallible<Self> {
        debug_assert_eq!(a.level, b.level);
        let a_word_base = PAIR_HEADER_WORDS;
        let b_word_base = a_word_base
            .checked_add(a.u32s.len())
            .ok_or("ONE X2 paired GPU PIS u32 input is too large")?;
        let b_float_base = a.f32s.len();
        let output_span = a.level.patches() * OUTPUT_WORDS_PER_PATCH + DIAGNOSTIC_WORDS;
        let b_output_base = output_span;
        let output_words = output_span
            .checked_mul(2)
            .ok_or("ONE X2 paired GPU PIS output is too large")?;
        let as_u32 = |value: usize, part: &'static str| -> Fallible<u32> {
            u32::try_from(value)
                .map_err(move |_| format!("ONE X2 paired GPU PIS {part} exceeds u32").into())
        };
        let mut u32s = vec![0; PAIR_HEADER_WORDS];
        u32s[0] = PAIR_SCHEMA;
        u32s[1] = 2;
        u32s[2] = as_u32(a_word_base, "A word base")?;
        u32s[3] = 0;
        u32s[4] = 0;
        u32s[5] = as_u32(b_word_base, "B word base")?;
        u32s[6] = as_u32(b_float_base, "B float base")?;
        u32s[7] = as_u32(b_output_base, "B output base")?;
        u32s.extend(a.u32s);
        u32s.extend(b.u32s);
        let mut f32s = a.f32s;
        f32s.extend(b.f32s);
        Ok(Self {
            level: a.level,
            u32s,
            f32s,
            output_words,
            output_span,
            a_output_base: 0,
            b_output_base,
        })
    }
}

fn decode_terminal<D: PisDirection>(
    level: Level,
    words: &[u32],
) -> Result<TerminalBits<D>, TerminalGridError> {
    let expected_words = level.patches() * OUTPUT_WORDS_PER_PATCH + DIAGNOSTIC_WORDS;
    if words.len() != expected_words {
        return Err(TerminalGridError::Shape {
            direction: D::DIRECTION,
            level,
            component: "word span",
            expected: expected_words,
            actual: words.len(),
        });
    }
    let mut dcol = Vec::with_capacity(level.patches());
    let mut drow = Vec::with_capacity(level.patches());
    for pair in words[..level.patches() * 2].chunks_exact(2) {
        dcol.push(pair[0]);
        drow.push(pair[1]);
    }
    Ok(TerminalBits {
        level,
        dcol: dcol.into_boxed_slice(),
        drow: drow.into_boxed_slice(),
        diagnostics: words[level.patches() * 2..].into(),
        direction: PhantomData,
    })
}

impl<D: PisDirection> PackedInput<D> {
    fn new(
        input: &Input<D>,
        initial: InitialGrid<D>,
        hint: Option<&HintGrid<D>>,
        admission: DescentAdmission,
    ) -> Fallible<Self> {
        if initial.level != input.level {
            return Err(format!(
                "ONE X2 GPU PIS initial grid is {}, expected {}",
                initial.level, input.level
            )
            .into());
        }
        if let Some(hint) = hint
            && hint.level != input.level
        {
            return Err(format!(
                "ONE X2 GPU PIS hint grid is {}, expected {}",
                hint.level, input.level
            )
            .into());
        }

        let mut u32s = vec![0; HEADER_WORDS];
        u32s[0] = input.level.rows() as u32;
        u32s[1] = input.level.cols() as u32;
        u32s[2] = input.level.patch_rows() as u32;
        u32s[3] = input.level.patch_cols() as u32;
        u32s[4] = input.level.pixels() as u32;
        u32s[5] = input.level.patches() as u32;
        u32s[6] = u32::from(hint.is_some());
        u32s[7] = u32::from(input.disparity.is_some());
        u32s[8] = u32::from(admission.admits());
        append_u8_plane(&mut u32s, 9, &input.source);
        append_u8_plane(&mut u32s, 10, &input.target);
        append_u8_plane(&mut u32s, 11, &input.source_slot_mask);
        append_u8_plane(&mut u32s, 12, &input.target_slot_mask);
        u32s[13] = u32s.len() as u32;
        u32s.extend(input.cost_modes.iter().map(|mode| match mode {
            CostMode::Unweighted => 0,
            CostMode::Weighted => 1,
        }));
        u32s[22] = u32s.len() as u32;
        u32s.push(0);

        let mut f32s = Vec::new();
        append_f32_plane(&mut f32s, &mut u32s, 14, &input.gradient_col);
        append_f32_plane(&mut f32s, &mut u32s, 15, &input.gradient_row);
        append_f32_plane(&mut f32s, &mut u32s, 16, &input.raw_weight);
        append_f32_plane(&mut f32s, &mut u32s, 17, &input.patch_weight_sums);
        u32s[18] = f32s.len() as u32;
        for flow in &initial.flows {
            f32s.extend([flow.dcol(), flow.drow()]);
        }
        u32s[19] = f32s.len() as u32;
        if let Some(hint) = hint {
            for flow in &hint.flows {
                f32s.extend([flow.dcol(), flow.drow()]);
            }
        } else {
            f32s.extend(std::iter::repeat_n(0.0, 2 * input.level.patches()));
        }
        u32s[20] = f32s.len() as u32;
        if let Some(disparity) = input.disparity {
            f32s.extend([
                disparity.first[0],
                disparity.first[1],
                disparity.second[0],
                disparity.second[1],
            ]);
        } else {
            f32s.extend([0.0; 4]);
        }
        u32s[21] = f32s.len() as u32;
        for patch in 0..input.level.patches() {
            let row = patch / input.level.patch_cols();
            let col = patch % input.level.patch_cols();
            let model = input.prepared_source_terms(row, col).inverted();
            f32s.extend([
                model.gradient_col_sum,
                model.gradient_row_sum,
                model.inverse_col_col,
                model.inverse_col_row,
                model.inverse_row_row,
            ]);
        }
        Ok(Self {
            level: input.level,
            u32s,
            f32s,
            direction: PhantomData,
        })
    }
}

fn append_u8_plane(target: &mut Vec<u32>, header: usize, values: &[u8]) {
    target[header] = target.len() as u32;
    target.extend(values.iter().copied().map(u32::from));
}

fn append_f32_plane(
    target: &mut Vec<f32>,
    header_words: &mut [u32],
    header: usize,
    values: &[f32],
) {
    header_words[header] = target.len() as u32;
    target.extend_from_slice(values);
}

fn u32_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_ne_bytes()).collect()
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_bits().to_ne_bytes())
        .collect()
}

fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    bytes: Vec<u8>,
) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, &bytes);
    buffer
}

#[allow(clippy::type_complexity)]
fn qualification_fixture(
    level: Level,
) -> (
    Input<AtoB>,
    Input<BtoA>,
    InitialGrid<AtoB>,
    InitialGrid<BtoA>,
    HintGrid<AtoB>,
    HintGrid<BtoA>,
) {
    let pixels = level.pixels();
    let image_a = (0..pixels)
        .map(|at| ((at * 29 + at / level.cols() * 17 + 3) & 255) as u8)
        .collect::<Vec<_>>();
    let image_b = (0..pixels)
        .map(|at| ((at * 11 + at / level.cols() * 43 + 197) & 255) as u8)
        .collect::<Vec<_>>();
    let mask_a = (0..pixels)
        .map(|at| u8::from(!at.is_multiple_of(17) && at % level.cols() != 0))
        .collect::<Vec<_>>();
    let mask_b = (0..pixels)
        .map(|at| u8::from(!at.is_multiple_of(19) && at % level.cols() + 1 != level.cols()))
        .collect::<Vec<_>>();
    let prepared = |salt: u32| {
        let mut gx = Vec::with_capacity(pixels);
        let mut gy = Vec::with_capacity(pixels);
        let mut weight = Vec::with_capacity(pixels);
        for (at, mask) in mask_a.iter().copied().enumerate() {
            if mask == 0 {
                gx.push(0.0);
                gy.push(0.0);
                weight.push(0.0);
            } else {
                let bits = (at as u32).wrapping_mul(0x9e37_79b9).rotate_left(salt);
                gx.push(((bits & 255) as f32 - 127.0) / 31.0);
                gy.push((((bits >> 8) & 255) as f32 - 127.0) / 29.0);
                weight.push((((bits >> 16) & 127) + 1) as f32 / 23.0);
            }
        }
        (gx, gy, weight)
    };
    let modes = (0..level.patch_rows())
        .map(|row| {
            if row.is_multiple_of(3) {
                CostMode::Unweighted
            } else {
                CostMode::Weighted
            }
        })
        .collect::<Vec<_>>();
    let make_input = |reverse: bool| {
        let (gx, gy, weight) = prepared(if reverse { 11 } else { 5 });
        let images = super::super::LensPair {
            a: image_a.clone(),
            b: image_b.clone(),
        };
        let masks = super::super::LensPair {
            a: mask_a.clone(),
            b: mask_b.clone(),
        };
        (images, masks, gx, gy, weight)
    };
    let (images, masks, gx, gy, weight) = make_input(false);
    let a = Input::<AtoB>::from_native_order(level, images, masks, gx, gy, weight, modes.clone())
        .unwrap()
        .with_disparity_interval(super::DisparityInterval::new([-7.75, -7.5], [7.75, 7.5]));
    let (images, masks, gx, gy, weight) = make_input(true);
    let b = Input::<BtoA>::from_native_order(level, images, masks, gx, gy, weight, modes)
        .unwrap()
        .with_disparity_interval(super::DisparityInterval::new([7.6, 7.8], [-7.6, -7.8]));
    let seeds = |salt: usize| {
        (0..level.patches())
            .map(|patch| {
                Flow::new(
                    ((patch * 7 + salt) % 17) as f32 / 8.0 - 1.0,
                    ((patch * 11 + salt) % 19) as f32 / 9.0 - 1.0,
                )
                .unwrap()
            })
            .collect::<Vec<_>>()
    };
    (
        a,
        b,
        InitialGrid {
            level,
            flows: seeds(1).into_boxed_slice(),
            direction: PhantomData,
        },
        InitialGrid {
            level,
            flows: seeds(2).into_boxed_slice(),
            direction: PhantomData,
        },
        HintGrid::from_row_major(level, seeds(3)).unwrap(),
        HintGrid::from_row_major(level, seeds(4)).unwrap(),
    )
}

const SHADER: &str = include_str!("pis.wgsl");

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::*;

    #[test]
    fn paired_gpu_matches_cpu_terminal_bits() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping exact paired GPU PIS twin: {why}");
                return;
            }
        };
        GpuPisPipeline::new(&device, &queue).unwrap_or_else(|error| {
            panic!("paired GPU PIS qualification failed on {adapter}: {error}")
        });
    }

    #[test]
    fn qualification_refuses_candidate_mutation() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU: {why}"
                );
                eprintln!("skipping paired GPU PIS mutation refusal: {why}");
                return;
            }
        };
        let broken = SHADER.replacen(
            "if candidate == 0u || score < selected_scores[slot]",
            "if candidate == 0u || score > selected_scores[slot]",
            1,
        );
        assert_ne!(broken, SHADER, "candidate-order mutation found no target");
        let error = match GpuPisPipeline::from_shader(&device, &queue, &broken, true) {
            Ok(_) => panic!("changed paired GPU PIS candidate ties were accepted on {adapter}"),
            Err(error) => error,
        };
        assert!(
            error.downcast_ref::<QualificationError>().is_some(),
            "mutation returned the wrong refusal on {adapter}: {error}"
        );
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
            label: Some("exact paired ONE X2 GPU PIS"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        Ok((device, queue, name))
    }
}
