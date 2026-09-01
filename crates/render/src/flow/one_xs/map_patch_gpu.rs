//! Unselected resident GPU final-map materializer for the selected ONE X2 route.
//!
//! The production entry consumes a sealed upstream resident token, advances
//! that token's existing submission lease and leaves the packed 200-by-100
//! type-2 map resident. It never maps, polls or reconstructs an operand on the
//! CPU. Scene wiring remains a later boundary.

#![allow(
    dead_code,
    reason = "standalone qualification checkpoint is deliberately not selected by Scene"
)]

use std::num::NonZeroU64;
#[cfg(test)]
use std::sync::mpsc;

use kjerag_media::FrameStamp;

use super::gpu_context::OneXsGpuContext;
#[cfg(test)]
use super::map_patch::{
    self, BaseMap, BilateralInputs, CoordinateMap, FlowMap, GateMap, PreimageMap, SideInputs,
};
use crate::Fallible;
#[cfg(test)]
use crate::studio_type2::{ALPHA_BYTES, AlphaMap, PackedMap};
use crate::studio_type2::{MAP_NODES, PACKED_BYTES};

const SHADER: &str = include_str!("map_patch_gpu.wgsl");
const WORKGROUP_SIZE: u32 = 64;
const RETAINED_NODES: usize = super::ROWS * super::COLS;
const INPUT_WORDS: usize =
    2 * (MAP_NODES * 2 + RETAINED_NODES * 2 + RETAINED_NODES * 2 + MAP_NODES + MAP_NODES * 2);
const INPUT_BYTES: u64 = (INPUT_WORDS * size_of::<u32>()) as u64;
const ACTION_BYTES: u64 = (MAP_NODES * 2 * size_of::<u32>()) as u64;

/// Sealed handoff implemented by the upstream resident ONE X2 chain.
///
/// The methods are visible only within `one_xs`: no crate caller can supply a
/// free buffer, frame or queue. `submit_after` must advance the token's one
/// existing source-owner lease to the supplied command buffer.
pub(super) mod resident {
    use super::*;

    pub(in crate::flow::one_xs) trait Sealed {}

    pub(in crate::flow::one_xs) trait Operands: Sealed + Sized {
        fn context(&self) -> &OneXsGpuContext;
        fn frame(&self) -> &FrameStamp;
        fn alpha_binding(&self) -> wgpu::BufferBinding<'_>;
        fn encode_input_copy(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Buffer);
        fn submit_after(
            &mut self,
            producer: &OneXsGpuContext,
            command: wgpu::CommandBuffer,
        ) -> Fallible<()>;
    }
}

use resident::Operands;

/// GPU-resident packed map bound to the exact delivery that produced it.
///
/// This deliberately exposes no ordinary CPU byte view. Evidence code must
/// call [`Self::diagnostic_readback`] and wait for an explicit copy.
pub(super) struct GpuPackedMapFrame<O: Operands> {
    frame: FrameStamp,
    packed: wgpu::Buffer,
    _actions: wgpu::Buffer,
    context: OneXsGpuContext,
    upstream: O,
}

impl<O: Operands> GpuPackedMapFrame<O> {
    #[cfg(test)]
    fn diagnostic_readback(&self) -> Fallible<DiagnosticPackedMap> {
        let packed = readback_packed(self.context.device(), self.context.queue(), &self.packed)?;
        Ok(DiagnosticPackedMap {
            frame: self.frame.clone(),
            packed,
        })
    }

    /// Validate exact context and delivery identity, then build the two-buffer
    /// binding consumed by the existing direct type-2 Scene shader. No raw
    /// buffer leaves this module.
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
                        resource: wgpu::BindingResource::Buffer(self.upstream.alpha_binding()),
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

pub(super) struct GpuMapBinding<O: Operands> {
    frame: FrameStamp,
    read: wgpu::BindGroup,
    _resident: GpuPackedMapFrame<O>,
}

impl<O: Operands> GpuMapBinding<O> {
    pub(super) fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub(super) fn read(&self) -> &wgpu::BindGroup {
        &self.read
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BindingError {
    Context,
    Frame,
}

#[cfg(test)]
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

#[cfg(test)]
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
struct DiagnosticPackedMap {
    frame: FrameStamp,
    packed: PackedMap,
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

/// Render-private, unselected final-map compute pipeline.
pub(crate) struct GpuMapMaterializer {
    context: OneXsGpuContext,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl GpuMapMaterializer {
    pub(crate) fn new(context: OneXsGpuContext) -> Self {
        Self::from_shader(context, SHADER)
    }

    fn from_shader(context: OneXsGpuContext, shader: &str) -> Self {
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
        Self {
            context,
            pipeline,
            layout,
        }
    }

    /// Consume one sealed upstream token and emit an opaque resident map.
    ///
    /// Submission goes through the upstream token so its existing source-owner
    /// lease advances to this dispatch. There is no second lease, map, poll or
    /// readback on this path.
    pub(super) fn materialize<O: Operands>(
        &self,
        mut operands: O,
    ) -> Fallible<GpuPackedMapFrame<O>> {
        self.context.ensure_same(operands.context())?;
        let frame = operands.frame().clone();
        let input = self
            .context
            .device()
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("ONE X2 resident GPU final-map inputs"),
                size: INPUT_BYTES,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        let (packed, actions, command) = self.encode(&operands, &input);
        operands.submit_after(&self.context, command)?;
        Ok(GpuPackedMapFrame {
            frame,
            packed,
            _actions: actions,
            context: self.context.clone(),
            upstream: operands,
        })
    }

    fn encode<O: Operands>(
        &self,
        operands: &O,
        input: &wgpu::Buffer,
    ) -> (wgpu::Buffer, wgpu::Buffer, wgpu::CommandBuffer) {
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
        operands.encode_input_copy(&mut encoder, input);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 GPU final-map materializer"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &resources, &[]);
            pass.dispatch_workgroups((MAP_NODES as u32).div_ceil(WORKGROUP_SIZE), 1, 1);
        }
        (packed, actions, encoder.finish())
    }

    #[cfg(test)]
    fn qualify(&self) -> Fallible<()> {
        let fixture = QualificationFixture::new();
        fixture.verify_cpu_coverage();
        let inputs = fixture.cpu_inputs();
        let expected = map_patch::materialize(inputs).packed;
        let words = pack_inputs(inputs);
        let operands = TestResidentOperands::new(
            self.context.clone(),
            FrameStamp::for_test(0, std::time::Duration::ZERO, None),
            AlphaMap::new(vec![0.5; MAP_NODES]).unwrap(),
            &words,
        );
        let result = self.materialize(operands)?;
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
}

#[cfg(test)]
fn pack_inputs(inputs: BilateralInputs<'_>) -> Vec<u32> {
    let mut words = Vec::with_capacity(INPUT_WORDS);
    append_untyped_side(&mut words, inputs.b_to_a);
    append_untyped_side(&mut words, inputs.a_to_b);
    words
}

#[cfg(test)]
fn append_untyped_side(words: &mut Vec<u32>, side: SideInputs<'_>) {
    append_bound_side(
        words,
        side.preimage,
        side.base,
        side.flow,
        side.gate,
        side.coordinate,
    );
}

#[cfg(test)]
fn append_bound_side(
    words: &mut Vec<u32>,
    preimage: &PreimageMap,
    base: &BaseMap,
    flow: &FlowMap,
    gate: &GateMap,
    coordinate: &CoordinateMap,
) {
    append_f32x2(words, preimage.values());
    append_f32x2(words, base.values());
    append_f32x2(words, flow.values());
    words.extend(gate.values().iter().map(|value| value.to_bits()));
    append_f32x2(words, coordinate.values());
}

#[cfg(test)]
fn append_f32x2(words: &mut Vec<u32>, values: &[[f32; 2]]) {
    words.extend(
        values
            .iter()
            .flat_map(|value| value.iter().map(|component| component.to_bits())),
    );
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
fn upload(device: &wgpu::Device, queue: &wgpu::Queue, label: &str, bytes: &[u8]) -> wgpu::Buffer {
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

#[cfg(test)]
fn u32_bytes(words: &[u32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), std::mem::size_of_val(words)) }
}

#[cfg(test)]
struct TestResidentOperands {
    context: OneXsGpuContext,
    frame: FrameStamp,
    alpha: wgpu::Buffer,
    input: wgpu::Buffer,
}

#[cfg(test)]
impl TestResidentOperands {
    fn new(context: OneXsGpuContext, frame: FrameStamp, alpha: AlphaMap, words: &[u32]) -> Self {
        assert_eq!(words.len(), INPUT_WORDS);
        let input = upload(
            context.device(),
            context.queue(),
            "ONE X2 diagnostic resident final-map operands",
            u32_bytes(words),
        );
        let alpha = upload(
            context.device(),
            context.queue(),
            "ONE X2 diagnostic resident alpha",
            alpha.bytes(),
        );
        Self {
            context,
            frame,
            alpha,
            input,
        }
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

    fn alpha_binding(&self) -> wgpu::BufferBinding<'_> {
        self.alpha.as_entire_buffer_binding()
    }

    fn encode_input_copy(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Buffer) {
        encoder.copy_buffer_to_buffer(&self.input, 0, target, 0, INPUT_BYTES);
    }

    fn submit_after(
        &mut self,
        producer: &OneXsGpuContext,
        command: wgpu::CommandBuffer,
    ) -> Fallible<()> {
        self.context.ensure_same(producer)?;
        self.context.queue().submit([command]);
        Ok(())
    }
}

#[cfg(test)]
struct QualificationFixture {
    preimage: super::LensPair<PreimageMap>,
    base: super::LensPair<BaseMap>,
    flow: super::LensPair<FlowMap>,
    gate: super::LensPair<GateMap>,
    coordinate: super::LensPair<CoordinateMap>,
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
            preimage: super::LensPair {
                a: PreimageMap::new(left.preimage).unwrap(),
                b: PreimageMap::new(right.preimage).unwrap(),
            },
            base: super::LensPair {
                a: BaseMap::new(left.base).unwrap(),
                b: BaseMap::new(right.base).unwrap(),
            },
            flow: super::LensPair {
                a: FlowMap::new(left.flow).unwrap(),
                b: FlowMap::new(right.flow).unwrap(),
            },
            gate: super::LensPair {
                a: GateMap::new(left.gate).unwrap(),
                b: GateMap::new(right.gate).unwrap(),
            },
            coordinate: super::LensPair {
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
        TestResidentOperands::new(
            context,
            frame,
            AlphaMap::new(vec![0.5; MAP_NODES]).unwrap(),
            &pack_inputs(self.cpu_inputs()),
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
            let row = node / super::COLS;
            let col = node % super::COLS;
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
            let col = ((node * 37 + seed as usize * 11) % (super::COLS - 2)) as f32;
            let row = ((node * 53 + seed as usize * 101) % (super::ROWS - 2)) as f32;
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
        row * super::COLS + col,
        row * super::COLS + col + 1,
        (row + 1) * super::COLS + col,
        (row + 1) * super::COLS + col + 1,
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
        let pipeline = GpuMapMaterializer::new(context.clone());
        pipeline.qualify().unwrap_or_else(|error| {
            panic!("GPU final-map qualification failed on {adapter}: {error}")
        });
        let fixture = QualificationFixture::new();
        let frame = FrameStamp::for_test(0, std::time::Duration::ZERO, None);
        let gpu = pipeline
            .materialize(fixture.resident_operands(context.clone(), frame.clone()))
            .unwrap();
        assert_eq!(gpu.frame, frame);
        assert_eq!(gpu.packed.size(), PACKED_BYTES as u64);
        let read = gpu.diagnostic_readback().unwrap();
        assert_eq!(read.frame, frame);
        let layout = scene_layout(&device);
        let binding = gpu.bind_for_scene(&context, &frame, &layout).unwrap();
        assert_eq!(binding.frame(), &frame);
        assert_eq!(binding._resident.upstream.alpha.size(), ALPHA_BYTES as u64);
        let _ = binding.read();
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
        let pipeline = GpuMapMaterializer::new(context.clone());
        pipeline.qualify().unwrap();
        let fixture = QualificationFixture::new();
        let frame = FrameStamp::for_test(7, std::time::Duration::ZERO, None);
        let foreign_frame = FrameStamp::for_test(7, std::time::Duration::ZERO, None);
        let layout = scene_layout(&device);

        let foreign_context = OneXsGpuContext::new(&foreign_device, &foreign_queue);

        let gpu = pipeline
            .materialize(fixture.resident_operands(context.clone(), frame.clone()))
            .unwrap();
        match gpu.bind_for_scene(&foreign_context, &frame, &layout) {
            Err(error) => assert_eq!(error, BindingError::Context),
            Ok(_) => panic!("foreign GPU context was accepted"),
        }
        let gpu = pipeline
            .materialize(fixture.resident_operands(context.clone(), frame.clone()))
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
        GpuMapMaterializer::new(context.clone())
            .qualify()
            .unwrap_or_else(|error| {
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
            let error = GpuMapMaterializer::from_shader(context.clone(), &broken)
                .qualify()
                .expect_err(&format!(
                    "changed GPU final-map {name} was accepted on {adapter}"
                ));
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
