//! Detached sampling of Studio's two photometric source bands.
//!
//! The shared encoder composes one externally associated type-2 map through
//! the fixed native four-row lookup, then samples both exact source pictures.
//! The detached submit/read transaction below remains diagnostic; live
//! playback appends the same encoder directly to its resident command stream.

use std::sync::{Arc, mpsc};

use kjerag_media::{FrameStamp, Frames};
use wgpu::util::DeviceExt;

use crate::direct_type2;
use crate::flow::one_xs::LensPair;
use crate::stitch_camera::StitchCamera;
use crate::studio_type2::{MAP_WIDTH, OneXsMapFrame, PACKED_BYTES};
use crate::{Fallible, Planes, Reframe};

const MAP_ROWS: usize = 4;
const COMPOSED_NODES: usize = MAP_WIDTH * MAP_ROWS;
const COMPOSED_BYTES: u64 = (COMPOSED_NODES * size_of::<[f32; 4]>()) as u64;
const BAND_WIDTH: usize = 800;
const BAND_HEIGHT: usize = 16;
const BAND_PIXELS: usize = BAND_WIDTH * BAND_HEIGHT;
const BAND_BYTES: u64 = (BAND_PIXELS * 2 * size_of::<[f32; 4]>()) as u64;
const PACKED_BAND_BYTES: u64 = (BAND_PIXELS * size_of::<u32>()) as u64;
const READBACK_BYTES: u64 = COMPOSED_BYTES + BAND_BYTES;
const EXTENSION: usize = 6;
const EXTENDED_WIDTH: usize = MAP_WIDTH + 2 * EXTENSION;
const INVALID_BYTES: u64 = (EXTENDED_WIDTH * MAP_ROWS * size_of::<u32>()) as u64;

#[cfg(test)]
mod gpu_tests;

pub struct FusionInputs {
    frame: FrameStamp,
    camera: StitchCamera,
    bands: LensPair<Vec<u8>>,
    invalid: Vec<u8>,
    #[cfg(test)]
    coarse_uv: LensPair<Vec<[f32; 2]>>,
}

/// Encoded, not necessarily complete, GPU source-band inputs.
///
/// This owner retains every allocation and binding used by the commands
/// appended by [`FusionInputPipeline::encode`]. It never submits, polls, or
/// maps them. Coordinate invalidity is local to these source bands and is not
/// the producer's separate global observation-failure signal.
pub(crate) struct GpuBandInputs {
    _map: wgpu::BindGroup,
    _output_binding: wgpu::BindGroup,
    composed: wgpu::Buffer,
    float_bands: wgpu::Buffer,
    bands: [wgpu::Buffer; 2],
    invalid: wgpu::Buffer,
}

impl GpuBandInputs {
    pub(crate) fn bands(&self) -> [&wgpu::Buffer; 2] {
        [&self.bands[0], &self.bands[1]]
    }

    pub(crate) fn invalid(&self) -> &wgpu::Buffer {
        &self.invalid
    }
}

impl FusionInputs {
    pub fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub fn new_reference(&self) -> super::spatial::Reference {
        super::spatial::Reference::for_camera(self.camera)
    }

    /// Native fusion ordinals zero and one, as 800-by-16 BGR8 bands.
    ///
    /// They are delivered streams zero and one on ONE X2, and streams one
    /// and zero on the calibrated X4 path.
    pub fn bands(&self) -> [&[u8]; 2] {
        [&self.bands.a, &self.bands.b]
    }

    /// Four rows by 212 columns, including the six-pixel periodic extension.
    pub fn invalid(&self) -> &[u8] {
        &self.invalid
    }

    /// Test-only lens-local four-row coordinates already present in this
    /// diagnostic's readback. This performs no additional GPU work.
    #[cfg(test)]
    pub fn coarse_uv(&self) -> [&[[f32; 2]]; 2] {
        [&self.coarse_uv.a, &self.coarse_uv.b]
    }
}

#[must_use = "the submitted fusion input sample has not been consumed"]
pub struct PendingOneXsFusionInputs {
    _picture: wgpu::BindGroup,
    _uniforms: wgpu::Buffer,
    _map_buffer: wgpu::Buffer,
    _gpu: GpuBandInputs,
    readback: wgpu::Buffer,
    device: wgpu::Device,
    frame: FrameStamp,
    camera: StitchCamera,
    submission: wgpu::SubmissionIndex,
    // Last: decoder storage must outlive every bind group which samples it.
    frames: Arc<Frames>,
}

impl PendingOneXsFusionInputs {
    pub fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub fn read(self) -> Fallible<FusionInputs> {
        if self.frames.stamp() != self.frame {
            return Err("fusion input source owner changed frame identity".into());
        }
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(self.submission),
            timeout: None,
        })?;
        let (mapped, answer) = mpsc::channel();
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = mapped.send(result);
            });
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        answer.recv()??;
        let view = self.readback.slice(..).get_mapped_range();
        #[cfg(test)]
        let coarse_uv = decode_coarse_uv(&view, self.camera)?;
        let (bands, invalid) = decode(&view)?;
        drop(view);
        self.readback.unmap();
        Ok(FusionInputs {
            frame: self.frame,
            camera: self.camera,
            bands,
            invalid,
            #[cfg(test)]
            coarse_uv,
        })
    }
}

pub(crate) struct FusionInputPipeline {
    camera: StitchCamera,
    compose_pipeline: wgpu::ComputePipeline,
    sample_pipeline: wgpu::ComputePipeline,
    map_layout: wgpu::BindGroupLayout,
    output_layout: wgpu::BindGroupLayout,
    picture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    lookup: wgpu::Buffer,
}

impl FusionInputPipeline {
    pub(crate) fn new(
        device: &wgpu::Device,
        picture_layout: &wgpu::BindGroupLayout,
        camera: StitchCamera,
    ) -> Self {
        let storage = |binding, bytes, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: std::num::NonZeroU64::new(bytes),
            },
            count: None,
        };
        let map_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fusion input map composition"),
            entries: &[
                storage(0, PACKED_BYTES as u64, true),
                storage(1, (COMPOSED_NODES * size_of::<[f32; 2]>()) as u64, true),
                storage(2, COMPOSED_BYTES, false),
            ],
        });
        let output_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fusion source-band output"),
            entries: &[
                storage(0, BAND_BYTES, false),
                storage(1, PACKED_BAND_BYTES, false),
                storage(2, PACKED_BAND_BYTES, false),
                storage(3, INVALID_BYTES, false),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fusion source-band sampler"),
            bind_group_layouts: &[picture_layout, &map_layout, &output_layout],
            immediate_size: 0,
        });
        let native_lens_zero_is_delivered_one = camera.fusion_streams() == [1, 0];
        let shader = format!(
            "{}\nconst native_lens_zero_is_delivered_one = {};\n{}",
            direct_type2::source_wgsl(),
            native_lens_zero_is_delivered_one,
            SAMPLE_WGSL
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fusion source-band sampler"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let make_pipeline = |label, entry_point| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry_point),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let coordinates = super::coordinates::for_camera_band(camera);
        assert_eq!(coordinates.len(), COMPOSED_NODES);
        let lookup = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fusion fixed native band lookup"),
            contents: bytes_of(&coordinates),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fusion resident source-band sampler"),
            min_filter: wgpu::FilterMode::Linear,
            mag_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            camera,
            compose_pipeline: make_pipeline("fusion packed-map composition", "compose_fusion_map"),
            sample_pipeline: make_pipeline("fusion source-band sampling", "sample_fusion_bands"),
            map_layout,
            output_layout,
            picture_layout: picture_layout.clone(),
            sampler,
            lookup,
        }
    }

    pub(crate) fn camera(&self) -> StitchCamera {
        self.camera
    }

    /// Bind a caller-owned exact source pair without exposing its planes.
    /// The caller must retain both the returned binding and its source owner
    /// through submission; this helper performs no identity inference.
    pub(crate) fn bind_source(
        &self,
        device: &wgpu::Device,
        uniforms: &wgpu::Buffer,
        planes: [&Planes; 2],
    ) -> wgpu::BindGroup {
        let planes = self.fusion_planes(planes);
        direct_type2::bind_picture(
            device,
            &self.picture_layout,
            uniforms,
            planes,
            &self.sampler,
        )
    }

    fn fusion_planes<'a>(&self, planes: [&'a Planes; 2]) -> [&'a Planes; 2] {
        let streams = self.camera.fusion_streams();
        [planes[streams[0]], planes[streams[1]]]
    }

    /// Append the two source-band passes without submitting or waiting.
    ///
    /// `picture` must be the caller's draw-private exact source binding and
    /// `packed` its already-sealed map. Their identity is deliberately not
    /// inferred here; the lease-bound caller owns that proof.
    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        picture: &wgpu::BindGroup,
        packed: &wgpu::Buffer,
    ) -> GpuBandInputs {
        self.encode_inner(device, encoder, picture, packed, None)
    }

    #[cfg(test)]
    fn encode_profiled(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        picture: &wgpu::BindGroup,
        packed: &wgpu::Buffer,
        timestamps: &wgpu::QuerySet,
    ) -> GpuBandInputs {
        self.encode_inner(device, encoder, picture, packed, Some(timestamps))
    }

    fn encode_inner(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        picture: &wgpu::BindGroup,
        packed: &wgpu::Buffer,
        timestamps: Option<&wgpu::QuerySet>,
    ) -> GpuBandInputs {
        let composed = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fusion composed packed band"),
            size: COMPOSED_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let map = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fusion input map composition"),
            layout: &self.map_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: packed.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.lookup.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: composed.as_entire_binding(),
                },
            ],
        });
        let float_bands = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fusion source-band output"),
            size: BAND_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let bands = std::array::from_fn(|lens| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(if lens == 0 {
                    "fusion packed left source band"
                } else {
                    "fusion packed right source band"
                }),
                size: PACKED_BAND_BYTES,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        });
        let invalid = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fusion coordinate-invalid band"),
            size: INVALID_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let output_binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fusion source-band output"),
            layout: &self.output_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: float_bands.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: bands[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: bands[1].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: invalid.as_entire_binding(),
                },
            ],
        });
        {
            let timestamp_writes = timestamps.map(|query_set| wgpu::ComputePassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index: Some(1),
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fusion packed-map composition"),
                timestamp_writes,
            });
            pass.set_pipeline(&self.compose_pipeline);
            pass.set_bind_group(0, picture, &[]);
            pass.set_bind_group(1, &map, &[]);
            pass.set_bind_group(2, &output_binding, &[]);
            pass.dispatch_workgroups((COMPOSED_NODES as u32).div_ceil(64), 1, 1);
        }
        {
            let timestamp_writes = timestamps.map(|query_set| wgpu::ComputePassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(2),
                end_of_pass_write_index: Some(3),
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fusion source-band sampling"),
                timestamp_writes,
            });
            pass.set_pipeline(&self.sample_pipeline);
            pass.set_bind_group(0, picture, &[]);
            pass.set_bind_group(1, &map, &[]);
            pass.set_bind_group(2, &output_binding, &[]);
            pass.dispatch_workgroups(
                (BAND_WIDTH as u32).div_ceil(8),
                (BAND_HEIGHT as u32).div_ceil(8),
                1,
            );
        }
        GpuBandInputs {
            _map: map,
            _output_binding: output_binding,
            composed,
            float_bands,
            bands,
            invalid,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn submit(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        picture_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        planes: [&Planes; 2],
        frames: Arc<Frames>,
        reframe: Reframe,
        map_frame: &OneXsMapFrame,
    ) -> PendingOneXsFusionInputs {
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fusion input private source metadata"),
            size: size_of::<Reframe>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniforms, 0, reframe.bytes());
        let picture = direct_type2::bind_picture(
            device,
            picture_layout,
            &uniforms,
            self.fusion_planes(planes),
            sampler,
        );
        let map_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fusion input packed map"),
            size: PACKED_BYTES as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&map_buffer, 0, map_frame.packed().bytes());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fusion input readback"),
            size: READBACK_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("fusion source-band sampler"),
        });
        let gpu = self.encode(device, &mut encoder, &picture, &map_buffer);
        encoder.copy_buffer_to_buffer(&gpu.composed, 0, &readback, 0, COMPOSED_BYTES);
        encoder.copy_buffer_to_buffer(&gpu.float_bands, 0, &readback, COMPOSED_BYTES, BAND_BYTES);
        let frame = frames.stamp();
        PendingOneXsFusionInputs {
            _picture: picture,
            _uniforms: uniforms,
            _map_buffer: map_buffer,
            _gpu: gpu,
            readback,
            device: device.clone(),
            frame,
            camera: self.camera,
            submission: queue.submit([encoder.finish()]),
            frames,
        }
    }
}

#[cfg(test)]
fn decode_coarse_uv(mapped: &[u8], camera: StitchCamera) -> Fallible<LensPair<Vec<[f32; 2]>>> {
    if mapped.len() != READBACK_BYTES as usize {
        return Err(format!(
            "fusion input readback mapped {} bytes, expected {READBACK_BYTES}",
            mapped.len()
        )
        .into());
    }
    let mut uv = LensPair {
        a: Vec::with_capacity(COMPOSED_NODES),
        b: Vec::with_capacity(COMPOSED_NODES),
    };
    for node in mapped[..COMPOSED_BYTES as usize].chunks_exact(size_of::<[f32; 4]>()) {
        let component = |at: usize| {
            f32::from_bits(u32::from_le_bytes(
                node[at * 4..at * 4 + 4].try_into().unwrap(),
            ))
        };
        let delivered = [
            [component(0) * 2.0, component(1)],
            [component(2) * 2.0 - 1.0, component(3)],
        ];
        let streams = camera.fusion_streams();
        uv.a.push(delivered[streams[0]]);
        uv.b.push(delivered[streams[1]]);
    }
    Ok(uv)
}

fn decode(mapped: &[u8]) -> Fallible<(LensPair<Vec<u8>>, Vec<u8>)> {
    if mapped.len() != READBACK_BYTES as usize {
        return Err(format!(
            "fusion input readback mapped {} bytes, expected {READBACK_BYTES}",
            mapped.len()
        )
        .into());
    }
    let mut validity = Vec::with_capacity(COMPOSED_NODES);
    for node in mapped[..COMPOSED_BYTES as usize].chunks_exact(size_of::<[f32; 4]>()) {
        let component = |at: usize| {
            f32::from_bits(u32::from_le_bytes(
                node[at * 4..at * 4 + 4].try_into().unwrap(),
            ))
        };
        let packed = [component(0), component(1), component(2), component(3)];
        validity.push([
            ordered_unit([packed[0] * 2.0, packed[1]]),
            ordered_unit([packed[2] * 2.0 - 1.0, packed[3]]),
        ]);
    }
    let mut bands = LensPair {
        a: Vec::with_capacity(BAND_PIXELS * 3),
        b: Vec::with_capacity(BAND_PIXELS * 3),
    };
    for sample in mapped[COMPOSED_BYTES as usize..].chunks_exact(2 * size_of::<[f32; 4]>()) {
        for (image, first) in [(&mut bands.a, 0usize), (&mut bands.b, 4usize)] {
            let rgb = std::array::from_fn::<_, 3, _>(|channel| {
                let at = (first + channel) * 4;
                f32::from_bits(u32::from_le_bytes(sample[at..at + 4].try_into().unwrap()))
            });
            image.extend(rgb.into_iter().rev().map(diagnostic_byte));
        }
    }
    Ok((bands, extend_invalid(&validity)))
}

#[allow(
    clippy::manual_range_contains,
    reason = "native ordered comparisons do not reject NaN"
)]
fn ordered_unit(uv: [f32; 2]) -> bool {
    !uv.into_iter().any(|value| value < 0.0 || value > 1.0)
}

fn extend_invalid(validity: &[[bool; 2]]) -> Vec<u8> {
    debug_assert_eq!(validity.len(), COMPOSED_NODES);
    let mut invalid = Vec::with_capacity(EXTENDED_WIDTH * MAP_ROWS);
    for row in 0..MAP_ROWS {
        for extended_col in 0..EXTENDED_WIDTH {
            let col = (extended_col + MAP_WIDTH - EXTENSION) % MAP_WIDTH;
            invalid.push(u8::from(
                !validity[row * MAP_WIDTH + col].into_iter().all(|v| v),
            ));
        }
    }
    invalid
}

fn diagnostic_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round_ties_even() as u8
}

fn bytes_of<T>(values: &[T]) -> &[u8] {
    // The source remains alive for the duration of DeviceExt's immediate copy.
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast(), std::mem::size_of_val(values)) }
}

const SAMPLE_WGSL: &str = r#"
struct FusionBandSample { left: vec4<f32>, right: vec4<f32> };
@group(1) @binding(0) var<storage, read> input_map: array<vec4<f32>>;
@group(1) @binding(1) var<storage, read> band_lookup: array<vec2<f32>>;
@group(1) @binding(2) var<storage, read_write> composed_map: array<vec4<f32>>;
@group(2) @binding(0) var<storage, read_write> fusion_bands: array<FusionBandSample>;
@group(2) @binding(1) var<storage, read_write> packed_left_band: array<u32>;
@group(2) @binding(2) var<storage, read_write> packed_right_band: array<u32>;
@group(2) @binding(3) var<storage, read_write> coordinate_invalid: array<u32>;

fn input_map_at(padded_x: i32, y: i32) -> vec4<f32> {
  let source_x = (padded_x + 199) % 200;
  return input_map[u32(clamp(y, 0, 99) * 200 + source_x)];
}

fn round_ties_even(value: f32) -> f32 {
  let low = floor(value);
  let fraction = value - low;
  let odd = (i32(low) & 1) != 0;
  return select(low, low + 1.0, fraction > 0.5 || (fraction == 0.5 && odd));
}

fn inter_linear_coordinate(value: f32) -> f32 {
  return round_ties_even(value * 32.0) / 32.0;
}

fn compose_at(coordinate: vec2<f32>) -> vec4<f32> {
  // OpenCV INTER_LINEAR tabulates each remap fraction at five bits.
  let q = vec2<f32>(inter_linear_coordinate(coordinate.x),
    inter_linear_coordinate(coordinate.y));
  let base = vec2<i32>(floor(q));
  let f = fract(q);
  let top = mix(input_map_at(base.x, base.y), input_map_at(base.x + 1, base.y), f.x);
  let bottom = mix(input_map_at(base.x, base.y + 1), input_map_at(base.x + 1, base.y + 1), f.x);
  return mix(top, bottom, f.y);
}

@compute @workgroup_size(64, 1, 1)
fn compose_fusion_map(@builtin(global_invocation_id) at: vec3<u32>) {
  if at.x >= 800u { return; }
  composed_map[at.x] = compose_at(band_lookup[at.x]);
}

fn composed_at(x: i32, y: i32) -> vec4<f32> {
  return composed_map[u32(clamp(y, 0, 3) * 200 + clamp(x, 0, 199))];
}

fn endpoint_packed(at: vec2<u32>) -> vec4<f32> {
  let p = vec2<f32>(f32(at.x) * 199.0 / 799.0, f32(at.y) * 3.0 / 15.0);
  let base = vec2<i32>(floor(p));
  let f = fract(p);
  let top = mix(composed_at(base.x, base.y), composed_at(base.x + 1, base.y), f.x);
  let bottom = mix(composed_at(base.x, base.y + 1), composed_at(base.x + 1, base.y + 1), f.x);
  return mix(top, bottom, f.y);
}

fn source_lens_rgb(lens: u32, uv: vec2<f32>) -> vec3<f32> {
  var luma: f32;
  var chroma: vec2<f32>;
  if lens == 0u {
    luma = textureSampleLevel(type2_luma0, type2_sampler, uv, 0.0).r;
    chroma = textureSampleLevel(type2_chroma0, type2_sampler, uv, 0.0).rg;
  } else {
    luma = textureSampleLevel(type2_luma1, type2_sampler, uv, 0.0).r;
    chroma = textureSampleLevel(type2_chroma1, type2_sampler, uv, 0.0).rg;
  }
  return source_rgb(luma, chroma - vec2<f32>(0.50196081399917603));
}

fn native_lens_uv(packed: vec4<f32>, lens: u32) -> vec2<f32> {
  if native_lens_zero_is_delivered_one {
    return select(packed.xy, packed.zw, lens == 0u);
  }
  return select(packed.zw, packed.xy, lens == 0u);
}

fn packed_bgr(rgb: vec3<f32>) -> u32 {
  let scaled = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0;
  let bytes = vec3<u32>(u32(round_ties_even(scaled.r)),
    u32(round_ties_even(scaled.g)), u32(round_ties_even(scaled.b)));
  return bytes.b | (bytes.g << 8u) | (bytes.r << 16u);
}

fn outside_unit(value: f32) -> bool {
  // Ordered comparisons deliberately do not reject NaN, matching the native
  // coordinate scan. Public map construction admits only finite payloads.
  return value < 0.0 || value > 1.0;
}

@compute @workgroup_size(8, 8, 1)
fn sample_fusion_bands(@builtin(global_invocation_id) at: vec3<u32>) {
  if at.x < 212u && at.y < 4u {
    let column = (at.x + 194u) % 200u;
    let packed = composed_map[at.y * 200u + column];
    let left_packed = native_lens_uv(packed, 0u);
    let right_packed = native_lens_uv(packed, 1u);
    let left = vec2<f32>(left_packed.x * 2.0 - select(0.0, 1.0, native_lens_zero_is_delivered_one), left_packed.y);
    let right = vec2<f32>(right_packed.x * 2.0 - select(1.0, 0.0, native_lens_zero_is_delivered_one), right_packed.y);
    coordinate_invalid[at.y * 212u + at.x] = u32(
      outside_unit(left.x) || outside_unit(left.y) ||
      outside_unit(right.x) || outside_unit(right.y));
  }
  if at.x >= 800u || at.y >= 16u { return; }
  let packed = endpoint_packed(at.xy);
  let index = at.y * 800u + at.x;
  let left_packed = native_lens_uv(packed, 0u);
  let right_packed = native_lens_uv(packed, 1u);
  let left = source_lens_rgb(0u, vec2<f32>(left_packed.x * 2.0 - select(0.0, 1.0, native_lens_zero_is_delivered_one), left_packed.y));
  let right = source_lens_rgb(1u, vec2<f32>(right_packed.x * 2.0 - select(1.0, 0.0, native_lens_zero_is_delivered_one), right_packed.y));
  fusion_bands[index].left = vec4<f32>(left, 0.0);
  fusion_bands[index].right = vec4<f32>(right, 0.0);
  packed_left_band[index] = packed_bgr(left);
  packed_right_band[index] = packed_bgr(right);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_shapes_and_two_stage_entries_are_fixed() {
        assert_eq!(COMPOSED_NODES, 800);
        assert_eq!(BAND_PIXELS, 12_800);
        assert_eq!(READBACK_BYTES, 422_400);
        assert_eq!(EXTENDED_WIDTH * MAP_ROWS, 848);
        assert_eq!(SAMPLE_WGSL.matches("fn compose_fusion_map").count(), 1);
        assert_eq!(SAMPLE_WGSL.matches("fn sample_fusion_bands").count(), 1);
        assert!(SAMPLE_WGSL.contains("199.0 / 799.0"));
        assert!(SAMPLE_WGSL.contains("3.0 / 15.0"));
    }

    #[test]
    fn ordered_range_and_periodic_validity_match_native_boundary() {
        assert!(ordered_unit([0.0, 1.0]));
        assert!(!ordered_unit([-f32::EPSILON, 0.5]));
        assert!(!ordered_unit([0.5, 1.0 + f32::EPSILON]));
        assert!(ordered_unit([f32::NAN, 0.5]));
        let mut validity = vec![[true; 2]; COMPOSED_NODES];
        for row in 0..MAP_ROWS {
            validity[row * MAP_WIDTH + MAP_WIDTH - EXTENSION] = [false, true];
            validity[row * MAP_WIDTH] = [true, false];
        }
        let invalid = extend_invalid(&validity);
        for row in invalid.chunks_exact(EXTENDED_WIDTH) {
            assert_eq!(row[0], 1);
            assert_eq!(row[EXTENSION], 1);
            assert_eq!(row[MAP_WIDTH + EXTENSION], 1);
        }
    }

    #[test]
    fn diagnostic_bgr_conversion_clamps_and_rounds_ties_even() {
        assert_eq!(diagnostic_byte(-1.0), 0);
        assert_eq!(diagnostic_byte(300.0 / 255.0), 255);
        assert_eq!(diagnostic_byte(2.5 / 255.0), 2);
        assert_eq!(diagnostic_byte(3.5 / 255.0), 4);
    }
}
