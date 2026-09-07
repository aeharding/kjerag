//! Detached sampling of the two exact source charts named by one type-2 map.
//!
//! This is a diagnostic transaction. The caller associates a captured or
//! computed map with a source frame; this module checks that association but
//! does not authenticate the map's provenance and is never selected by live
//! playback.

use std::sync::{Arc, mpsc};

use kjerag_media::{FrameStamp, Frames};

use crate::direct_type2;
use crate::flow::one_xs::{Lens, LensPair};
use crate::flow::one_xs_belt_gpu::ResidentCameraProfile;
use crate::map_oracle::local_uv;
use crate::studio_type2::{
    ALPHA_BYTES, MAP_HEIGHT, MAP_NODES, MAP_WIDTH, OneXsMapFrame, PACKED_BYTES,
};
use crate::{Fallible, Planes, Reframe};

const OUTPUT_WORDS: usize = 16;
const OUTPUT_BYTES: u64 = (MAP_NODES * OUTPUT_WORDS * size_of::<u32>()) as u64;
const VALIDITY_ROWS: usize = 4;
const VALIDITY_TOP: usize = 48;
const EXTENSION: usize = 6;
const EXTENDED_WIDTH: usize = MAP_WIDTH + 2 * EXTENSION;

#[cfg(test)]
mod gpu_tests;

/// Exact-frame diagnostic source charts in renderer lens order.
pub struct FusionInputs {
    frame: FrameStamp,
    images: LensPair<Vec<u8>>,
    invalid: Vec<u8>,
}

impl FusionInputs {
    pub fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    /// Left (`packed.xy`) and right (`packed.zw`) 200-by-100 BGR8 charts.
    pub fn images(&self) -> [&[u8]; 2] {
        [&self.images.a, &self.images.b]
    }

    /// Four rows by 212 columns, including the six-pixel periodic extension.
    pub fn invalid(&self) -> &[u8] {
        &self.invalid
    }
}

/// One submitted diagnostic sample. Reading consumes its exact source owner.
#[must_use = "the submitted ONE X2 fusion input sample has not been consumed"]
pub struct PendingOneXsFusionInputs {
    _picture: wgpu::BindGroup,
    _map: wgpu::BindGroup,
    _output_binding: wgpu::BindGroup,
    _uniforms: wgpu::Buffer,
    _map_buffers: [wgpu::Buffer; 2],
    _output: wgpu::Buffer,
    readback: wgpu::Buffer,
    device: wgpu::Device,
    profile: Arc<ResidentCameraProfile>,
    frame: FrameStamp,
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
            return Err("ONE X2 fusion input source owner changed frame identity".into());
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
        let (images, invalid) = decode(&view, &self.profile)?;
        drop(view);
        self.readback.unmap();
        Ok(FusionInputs {
            frame: self.frame,
            images,
            invalid,
        })
    }
}

pub(crate) struct FusionInputPipeline {
    pipeline: wgpu::ComputePipeline,
    map_layout: wgpu::BindGroupLayout,
    output_layout: wgpu::BindGroupLayout,
}

impl FusionInputPipeline {
    pub(crate) fn new(device: &wgpu::Device, picture_layout: &wgpu::BindGroupLayout) -> Self {
        let storage = |binding, bytes| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: std::num::NonZeroU64::new(bytes),
            },
            count: None,
        };
        let map_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ONE X2 fusion input map"),
            entries: &[
                storage(0, PACKED_BYTES as u64),
                storage(1, ALPHA_BYTES as u64),
            ],
        });
        let output_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ONE X2 fusion input output"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(OUTPUT_BYTES),
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 fusion input sampler"),
            bind_group_layouts: &[picture_layout, &map_layout, &output_layout],
            immediate_size: 0,
        });
        let shader = format!("{}\n{}", direct_type2::draw_wgsl(), SAMPLE_WGSL);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 fusion input sampler"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ONE X2 fusion input sampler"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("sample_fusion_inputs"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            pipeline,
            map_layout,
            output_layout,
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
        profile: Arc<ResidentCameraProfile>,
    ) -> PendingOneXsFusionInputs {
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 fusion input private projection"),
            size: size_of::<Reframe>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniforms, 0, reframe.bytes());
        let picture =
            direct_type2::bind_picture(device, picture_layout, &uniforms, planes, sampler);

        let map_buffers = [
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ONE X2 fusion input packed map"),
                size: PACKED_BYTES as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ONE X2 fusion input alpha map"),
                size: ALPHA_BYTES as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        ];
        queue.write_buffer(&map_buffers[0], 0, map_frame.packed().bytes());
        queue.write_buffer(&map_buffers[1], 0, map_frame.alpha().bytes());
        let map = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 fusion input map"),
            layout: &self.map_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: map_buffers[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: map_buffers[1].as_entire_binding(),
                },
            ],
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 fusion input output"),
            size: OUTPUT_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 fusion input readback"),
            size: OUTPUT_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let output_binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 fusion input output"),
            layout: &self.output_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: output.as_entire_binding(),
            }],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 fusion input sampler"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 fusion input sampler"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &picture, &[]);
            pass.set_bind_group(1, &map, &[]);
            pass.set_bind_group(2, &output_binding, &[]);
            pass.dispatch_workgroups(
                (MAP_WIDTH as u32).div_ceil(8),
                (MAP_HEIGHT as u32).div_ceil(8),
                1,
            );
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, OUTPUT_BYTES);
        let frame = frames.stamp();
        PendingOneXsFusionInputs {
            _picture: picture,
            _map: map,
            _output_binding: output_binding,
            _uniforms: uniforms,
            _map_buffers: map_buffers,
            _output: output,
            readback,
            device: device.clone(),
            profile,
            frame,
            submission: queue.submit([encoder.finish()]),
            frames,
        }
    }
}

fn decode(
    mapped: &[u8],
    profile: &ResidentCameraProfile,
) -> Fallible<(LensPair<Vec<u8>>, Vec<u8>)> {
    if mapped.len() != OUTPUT_BYTES as usize {
        return Err(format!(
            "ONE X2 fusion input readback mapped {} bytes, expected {OUTPUT_BYTES}",
            mapped.len()
        )
        .into());
    }
    let mut images = LensPair {
        a: Vec::with_capacity(MAP_NODES * 3),
        b: Vec::with_capacity(MAP_NODES * 3),
    };
    let mut validity = Vec::with_capacity(MAP_NODES);
    for node in mapped.chunks_exact(OUTPUT_WORDS * size_of::<u32>()) {
        let word = |at: usize| u32::from_le_bytes(node[at * 4..at * 4 + 4].try_into().unwrap());
        let packed = std::array::from_fn(|component| f32::from_bits(word(8 + component)));
        let mut source_covered = [false; 2];
        for (lens, image, first) in [
            (Lens::A, &mut images.a, 0usize),
            (Lens::B, &mut images.b, 4usize),
        ] {
            let rgb = [
                f32::from_bits(word(first)),
                f32::from_bits(word(first + 1)),
                f32::from_bits(word(first + 2)),
            ];
            image.extend(rgb.into_iter().rev().map(diagnostic_byte));
            source_covered[lens.index()] =
                profile.source_is_covered(lens, local_uv(packed, lens.index()));
        }
        validity.push([source_covered[0], source_covered[1], word(12) != 0]);
    }
    Ok((images, extend_invalid(&validity)))
}

fn extend_invalid(validity: &[[bool; 3]]) -> Vec<u8> {
    debug_assert_eq!(validity.len(), MAP_NODES);
    let mut invalid = Vec::with_capacity(EXTENDED_WIDTH * VALIDITY_ROWS);
    for row in VALIDITY_TOP..VALIDITY_TOP + VALIDITY_ROWS {
        for extended_col in 0..EXTENDED_WIDTH {
            let col = (extended_col + MAP_WIDTH - EXTENSION) % MAP_WIDTH;
            invalid.push(u8::from(
                !validity[row * MAP_WIDTH + col]
                    .into_iter()
                    .all(|value| value),
            ));
        }
    }
    invalid
}

fn diagnostic_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round_ties_even() as u8
}

const SAMPLE_WGSL: &str = r#"
struct FusionInputSample {
  left: vec4<f32>,
  right: vec4<f32>,
  packed: vec4<f32>,
  covered: u32,
  padding: array<u32, 3>,
};

@group(2) @binding(0) var<storage, read_write> fusion_inputs: array<FusionInputSample>;

@compute @workgroup_size(8, 8, 1)
fn sample_fusion_inputs(@builtin(global_invocation_id) at: vec3<u32>) {
  if at.x >= 200u || at.y >= 100u { return; }
  let theta = f32(at.y) * TYPE2_PI / 99.0;
  let phi = f32(at.x) * TYPE2_TAU / 200.0;
  let q_working = vec3<f32>(sin(theta) * cos(phi), sin(theta) * sin(phi), cos(theta));
  let angle = TYPE2_PI * 0.5;
  let cb = cos(angle);
  let sb = sin(angle);
  let q_final = vec3<f32>(
    cb * q_working.x + sb * q_working.z,
    q_working.y,
    -sb * q_working.x + cb * q_working.z,
  );
  let map = type2_mesh(vec3<f32>(-q_final.y, q_final.z, q_final.x));
  let index = at.y * 200u + at.x;
  fusion_inputs[index].left = vec4<f32>(type2_ycbcr(map.packed.xy), 0.0);
  fusion_inputs[index].right = vec4<f32>(type2_ycbcr(map.packed.zw), 0.0);
  fusion_inputs[index].packed = map.packed;
  fusion_inputs[index].covered = u32(map.covered > 0.5);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_shape_and_periodic_validity_extension_are_fixed() {
        assert_eq!(OUTPUT_BYTES, 1_280_000);
        assert_eq!(EXTENDED_WIDTH * VALIDITY_ROWS, 848);
        let source = SAMPLE_WGSL;
        assert!(source.contains("f32(at.y) * TYPE2_PI / 99.0"));
        assert!(source.contains("f32(at.x) * TYPE2_TAU / 200.0"));
        assert!(source.contains("type2_ycbcr(map.packed.xy)"));
        assert!(source.contains("type2_ycbcr(map.packed.zw)"));
    }

    #[test]
    fn atlas_coordinates_become_lens_local_and_validity_wraps_six_columns() {
        let packed = [0.125, 0.25, 0.875, 0.75];
        assert_eq!(local_uv(packed, Lens::A.index()), [0.25, 0.25]);
        assert_eq!(local_uv(packed, Lens::B.index()), [0.75, 0.75]);

        let mut validity = vec![[true; 3]; MAP_NODES];
        for row in VALIDITY_TOP..VALIDITY_TOP + VALIDITY_ROWS {
            validity[row * MAP_WIDTH + MAP_WIDTH - EXTENSION] = [false, true, true];
            validity[row * MAP_WIDTH] = [true, false, true];
            validity[row * MAP_WIDTH + 1] = [true, true, false];
        }
        let invalid = extend_invalid(&validity);
        assert_eq!(invalid.len(), EXTENDED_WIDTH * VALIDITY_ROWS);
        for row in invalid.chunks_exact(EXTENDED_WIDTH) {
            assert_eq!(&row[..7], &[1, 0, 0, 0, 0, 0, 1]);
            assert_eq!(row[7], 1);
            assert!(row[8..MAP_WIDTH].iter().all(|&value| value == 0));
            assert_eq!(row[MAP_WIDTH], 1);
            assert!(
                row[MAP_WIDTH + 1..EXTENDED_WIDTH - EXTENSION]
                    .iter()
                    .all(|&value| value == 0)
            );
            assert_eq!(&row[EXTENDED_WIDTH - EXTENSION..], &[1, 1, 0, 0, 0, 0]);
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
