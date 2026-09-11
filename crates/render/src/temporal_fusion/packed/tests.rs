use std::time::{Duration, Instant};

use super::*;
use crate::temporal_fusion::HorizontalBoundary;
use crate::temporal_fusion::tests::{array_texture, copy_texture, gpu, read_copy, write_layer};
use crate::temporal_fusion::{GpuFuse, Inputs, Parameters};

const SMALL_SIZE: [u32; 2] = [38, 26];
const SMALL_ROI: [u32; 4] = [2, 4, 34, 20];

#[test]
fn packed_quartets_match_original_planes_exactly() {
    // The fixture combines current/current+1 full-confidence samples (exact
    // half-code results), signed odd motion, and zero/negative confidences.
    let Some((device, queue)) = gpu() else {
        return;
    };
    let original = GpuFuse::new(&device);
    let packed = Encoder::new(&device);
    for references in [1, 3, 6] {
        let fixture = Fixture::new(&device, &queue, SMALL_SIZE, references);
        assert_outputs_equal(&device, &queue, &original, &packed, &fixture, SMALL_ROI);
    }
}

#[test]
fn periodic_packed_fusion_wraps_y_and_uv_at_both_horizontal_edges() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let size = [32, 4];
    let uv_size = [16, 2];
    let y = array_texture(
        &device,
        "periodic packed fusion Y",
        size,
        2,
        wgpu::TextureFormat::R8Unorm,
    );
    let uv = array_texture(
        &device,
        "periodic packed fusion UV",
        uv_size,
        2,
        wgpu::TextureFormat::Rg8Unorm,
    );
    let flow = array_texture(
        &device,
        "periodic packed fusion flow",
        [2, 1],
        1,
        wgpu::TextureFormat::Rgba16Sint,
    );
    let luma = array_texture(
        &device,
        "periodic packed fusion luma",
        [2, 1],
        1,
        wgpu::TextureFormat::R8Uint,
    );

    let reference_y: Vec<_> = (0..size[1])
        .flat_map(|_| {
            (0..size[0]).map(|x| {
                if x < 2 {
                    40
                } else if x >= 30 {
                    200
                } else {
                    100
                }
            })
        })
        .collect();
    let current_y = vec![100; (size[0] * size[1]) as usize];
    write_layer(&queue, &y, 0, [0, 0], size, size[0], &reference_y);
    write_layer(&queue, &y, 1, [0, 0], size, size[0], &current_y);

    let reference_uv: Vec<_> = (0..uv_size[1])
        .flat_map(|_| {
            (0..uv_size[0]).flat_map(|x| match x {
                0 => [40, 60],
                15 => [200, 220],
                _ => [100, 120],
            })
        })
        .collect();
    let current_uv: Vec<_> = (0..uv_size[0] * uv_size[1])
        .flat_map(|_| [100, 120])
        .collect();
    write_layer(
        &queue,
        &uv,
        0,
        [0, 0],
        uv_size,
        uv_size[0] * 2,
        &reference_uv,
    );
    write_layer(&queue, &uv, 1, [0, 0], uv_size, uv_size[0] * 2, &current_uv);
    let flow_bytes: Vec<_> = [[-2i16, 0, 255, 255], [2i16, 0, 255, 255]]
        .into_iter()
        .flatten()
        .flat_map(i16::to_le_bytes)
        .collect();
    write_layer(&queue, &flow, 0, [0, 0], [2, 1], 16, &flow_bytes);
    write_layer(&queue, &luma, 0, [0, 0], [2, 1], 2, &[0, 0]);

    let inputs = Inputs {
        y: &y,
        uv: &uv,
        flow: &flow,
        luma: &luma,
    };
    let params = Parameters {
        noise: 1.0,
        limit: 1.0,
        y_limits: [1.0; 256],
        uv_limits: [1.0; 256],
        current_layer: 1,
        reference_layers: vec![0],
    };
    let packed = Encoder::with_boundary(&device, HorizontalBoundary::Periodic);
    let mut encoder = device.create_command_encoder(&Default::default());
    let output = packed
        .encode(
            &device,
            &mut encoder,
            inputs,
            &params,
            [0, 0, size[0], size[1]],
        )
        .unwrap();
    let y_copy = copy_texture(&device, &mut encoder, &output.y, [16, 2], 4);
    let uv_copy = copy_texture(&device, &mut encoder, &output.uv, uv_size, 2);
    queue.submit([encoder.finish()]);

    let y_result = unpack_y(&read_copy(&device, &y_copy, [16, 2], 4), size);
    assert_eq!(&y_result[0..2], &[150, 150]);
    assert_eq!(&y_result[30..32], &[70, 70]);
    let uv_result = read_copy(&device, &uv_copy, uv_size, 2);
    assert_eq!(&uv_result[0..2], &[150, 170]);
    assert_eq!(&uv_result[30..32], &[70, 90]);
}

#[test]
#[ignore]
fn benchmark_full_frame_packed_quartets_against_original_planes() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let original = GpuFuse::new(&device);
    let packed = Encoder::new(&device);
    let mut mismatch_count = 0;
    for size in [[7680, 3840], [5760, 2880]] {
        let fixture = Fixture::new(&device, &queue, size, 6);
        let roi = [0, 0, size[0], size[1]];

        drop(encode_original(&device, &queue, &original, &fixture, roi));
        drop(encode_packed(&device, &queue, &packed, &fixture, roi));

        let mut original_times = Vec::with_capacity(8);
        let mut packed_times = Vec::with_capacity(8);
        let mut original_output = None;
        let mut packed_output = None;
        for pair in 0..8 {
            if pair % 2 == 0 {
                let (output, elapsed) = encode_original(&device, &queue, &original, &fixture, roi);
                original_output = Some(output);
                original_times.push(elapsed);
                let (output, elapsed) = encode_packed(&device, &queue, &packed, &fixture, roi);
                packed_output = Some(output);
                packed_times.push(elapsed);
            } else {
                let (output, elapsed) = encode_packed(&device, &queue, &packed, &fixture, roi);
                packed_output = Some(output);
                packed_times.push(elapsed);
                let (output, elapsed) = encode_original(&device, &queue, &original, &fixture, roi);
                original_output = Some(output);
                original_times.push(elapsed);
            }
        }
        eprintln!(
            "temporal quartet fusion {size:?}: original_ms={:?} packed_ms={:?}",
            milliseconds(&original_times),
            milliseconds(&packed_times)
        );
        let [y_difference, uv_difference] = compare_outputs(
            &device,
            &queue,
            original_output.as_ref().unwrap(),
            packed_output.as_ref().unwrap(),
            [roi[2], roi[3]],
        );
        eprintln!(
            "temporal quartet fusion {size:?}: Y count={} max={} first(x,y,c,old,new)={:?}; UV count={} max={} first(x,y,c,old,new)={:?}",
            y_difference.count,
            y_difference.max,
            y_difference.first,
            uv_difference.count,
            uv_difference.max,
            uv_difference.first
        );
        mismatch_count += y_difference.count + uv_difference.count;
    }
    assert_eq!(mismatch_count, 0, "packed fusion output differs");
}

fn assert_outputs_equal(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    original: &GpuFuse,
    packed: &Encoder,
    fixture: &Fixture,
    roi: [u32; 4],
) {
    let mut encoder = device.create_command_encoder(&Default::default());
    let original_output = original
        .encode(device, &mut encoder, fixture.inputs(), &fixture.params, roi)
        .unwrap();
    let packed_output = packed
        .encode(device, &mut encoder, fixture.inputs(), &fixture.params, roi)
        .unwrap();
    let original_y = copy_texture(
        device,
        &mut encoder,
        &original_output.y,
        [roi[2], roi[3]],
        1,
    );
    let original_uv = copy_texture(
        device,
        &mut encoder,
        &original_output.uv,
        [roi[2] / 2, roi[3] / 2],
        2,
    );
    let packed_y = copy_texture(
        device,
        &mut encoder,
        &packed_output.y,
        [roi[2] / 2, roi[3] / 2],
        4,
    );
    let packed_uv = copy_texture(
        device,
        &mut encoder,
        &packed_output.uv,
        [roi[2] / 2, roi[3] / 2],
        2,
    );
    queue.submit([encoder.finish()]);
    assert_eq!(
        unpack_y(
            &read_copy(device, &packed_y, [roi[2] / 2, roi[3] / 2], 4),
            [roi[2], roi[3]],
        ),
        read_copy(device, &original_y, [roi[2], roi[3]], 1)
    );
    assert_eq!(
        read_copy(device, &packed_uv, [roi[2] / 2, roi[3] / 2], 2),
        read_copy(device, &original_uv, [roi[2] / 2, roi[3] / 2], 2)
    );
}

fn encode_original(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    fuse: &GpuFuse,
    fixture: &Fixture,
    roi: [u32; 4],
) -> (crate::temporal_fusion::Output, Duration) {
    let before = Instant::now();
    let mut encoder = device.create_command_encoder(&Default::default());
    let output = fuse
        .encode(device, &mut encoder, fixture.inputs(), &fixture.params, roi)
        .unwrap();
    queue.submit([encoder.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    (output, before.elapsed())
}

fn encode_packed(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    fuse: &Encoder,
    fixture: &Fixture,
    roi: [u32; 4],
) -> (Output, Duration) {
    let before = Instant::now();
    let mut encoder = device.create_command_encoder(&Default::default());
    let output = fuse
        .encode(device, &mut encoder, fixture.inputs(), &fixture.params, roi)
        .unwrap();
    queue.submit([encoder.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    (output, before.elapsed())
}

fn compare_outputs(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    original: &crate::temporal_fusion::Output,
    packed: &Output,
    size: [u32; 2],
) -> [Difference; 2] {
    let mut encoder = device.create_command_encoder(&Default::default());
    let original_y = copy_texture(device, &mut encoder, &original.y, size, 1);
    let original_uv = copy_texture(
        device,
        &mut encoder,
        &original.uv,
        [size[0] / 2, size[1] / 2],
        2,
    );
    let packed_y = copy_texture(
        device,
        &mut encoder,
        &packed.y,
        [size[0] / 2, size[1] / 2],
        4,
    );
    let packed_uv = copy_texture(
        device,
        &mut encoder,
        &packed.uv,
        [size[0] / 2, size[1] / 2],
        2,
    );
    queue.submit([encoder.finish()]);
    let original_y = read_copy(device, &original_y, size, 1);
    let packed_y = unpack_y(
        &read_copy(device, &packed_y, [size[0] / 2, size[1] / 2], 4),
        size,
    );
    let uv_size = [size[0] / 2, size[1] / 2];
    let original_uv = read_copy(device, &original_uv, uv_size, 2);
    let packed_uv = read_copy(device, &packed_uv, uv_size, 2);
    [
        differences(&original_y, &packed_y, size[0], 1),
        differences(&original_uv, &packed_uv, uv_size[0], 2),
    ]
}

struct Difference {
    count: usize,
    max: u8,
    first: Option<(u32, u32, u32, u8, u8)>,
}

fn differences(a: &[u8], b: &[u8], width: u32, components: u32) -> Difference {
    assert_eq!(a.len(), b.len());
    let mut difference = Difference {
        count: 0,
        max: 0,
        first: None,
    };
    for (index, (&a, &b)) in a.iter().zip(b).enumerate() {
        if a == b {
            continue;
        }
        difference.count += 1;
        difference.max = difference.max.max(a.abs_diff(b));
        if difference.first.is_none() {
            let texel = index as u32 / components;
            difference.first = Some((
                texel % width,
                texel / width,
                index as u32 % components,
                a,
                b,
            ));
        }
    }
    difference
}

fn unpack_y(packed: &[u8], size: [u32; 2]) -> Vec<u8> {
    let mut y = vec![0; (size[0] * size[1]) as usize];
    for row in 0..size[1] / 2 {
        for column in 0..size[0] / 2 {
            let source = ((row * (size[0] / 2) + column) * 4) as usize;
            let top = ((row * 2) * size[0] + column * 2) as usize;
            let bottom = top + size[0] as usize;
            y[top..top + 2].copy_from_slice(&packed[source..source + 2]);
            y[bottom..bottom + 2].copy_from_slice(&packed[source + 2..source + 4]);
        }
    }
    y
}

fn milliseconds(values: &[Duration]) -> Vec<f64> {
    values
        .iter()
        .map(|value| value.as_secs_f64() * 1e3)
        .collect()
}

struct Fixture {
    y: wgpu::Texture,
    uv: wgpu::Texture,
    flow: wgpu::Texture,
    luma: wgpu::Texture,
    params: Parameters,
}

impl Fixture {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, size: [u32; 2], references: u32) -> Self {
        let layers = references + 1;
        let blocks = [size[0].div_ceil(16), size[1].div_ceil(16)];
        let y = array_texture(
            device,
            "packed fusion Y",
            size,
            layers,
            wgpu::TextureFormat::R8Unorm,
        );
        let uv = array_texture(
            device,
            "packed fusion UV",
            [size[0] / 2, size[1] / 2],
            layers,
            wgpu::TextureFormat::Rg8Unorm,
        );
        let flow = array_texture(
            device,
            "packed fusion flow",
            blocks,
            references,
            wgpu::TextureFormat::Rgba16Sint,
        );
        let luma = array_texture(
            device,
            "packed fusion luma",
            blocks,
            1,
            wgpu::TextureFormat::R8Uint,
        );
        for layer in 0..layers {
            let y_bytes = y_layer(size, layer, references);
            write_layer(queue, &y, layer, [0, 0], size, size[0], &y_bytes);
            let uv_size = [size[0] / 2, size[1] / 2];
            let uv_bytes = uv_layer(uv_size, layer, references);
            write_layer(
                queue,
                &uv,
                layer,
                [0, 0],
                uv_size,
                uv_size[0] * 2,
                &uv_bytes,
            );
        }
        for ordinal in 0..references {
            let bytes = flow_layer(blocks, ordinal);
            write_layer(queue, &flow, ordinal, [0, 0], blocks, blocks[0] * 8, &bytes);
        }
        let luma_bytes: Vec<u8> = (0..blocks[0] * blocks[1])
            .map(|index| (index * 37 + 19) as u8)
            .collect();
        write_layer(queue, &luma, 0, [0, 0], blocks, blocks[0], &luma_bytes);
        let y_limits = std::array::from_fn(|index| ((index % 5 + 1) as f32) / 5.0);
        let uv_limits = std::array::from_fn(|index| ((index % 7 + 1) as f32) / 7.0);
        Self {
            y,
            uv,
            flow,
            luma,
            params: Parameters {
                noise: 8.0 / 255.0,
                limit: 12.0 / 255.0,
                y_limits,
                uv_limits,
                current_layer: references,
                reference_layers: (0..references).collect(),
            },
        }
    }

    fn inputs(&self) -> Inputs<'_> {
        Inputs {
            y: &self.y,
            uv: &self.uv,
            flow: &self.flow,
            luma: &self.luma,
        }
    }
}

fn y_layer(size: [u32; 2], layer: u32, current: u32) -> Vec<u8> {
    (0..size[1])
        .flat_map(|y| {
            (0..size[0]).map(move |x| {
                let base = 32 + ((x * 3 + y * 5) % 120) as u8;
                match layer {
                    l if l == current => base,
                    0 => base + 1,
                    _ => 24 + ((u32::from(base) + layer * 29 + x + 2 * y) % 180) as u8,
                }
            })
        })
        .collect()
}

fn uv_layer(size: [u32; 2], layer: u32, current: u32) -> Vec<u8> {
    (0..size[1])
        .flat_map(|y| {
            (0..size[0]).flat_map(move |x| {
                let base = [
                    48 + ((x * 7 + y * 3) % 100) as u8,
                    64 + ((x * 2 + y * 9) % 100) as u8,
                ];
                match layer {
                    l if l == current => base,
                    0 => [base[0] + 1, base[1] + 1],
                    _ => [
                        32 + ((u32::from(base[0]) + layer * 23 + y) % 180) as u8,
                        32 + ((u32::from(base[1]) + layer * 31 + x) % 180) as u8,
                    ],
                }
            })
        })
        .collect()
}

fn flow_layer(size: [u32; 2], ordinal: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((size[0] * size[1] * 8) as usize);
    for y in 0..size[1] {
        for x in 0..size[0] {
            let flow = if ordinal == 0 {
                match (x + y) % 3 {
                    0 => [0, 0, 255, 255],
                    1 => [-1, -3, 255, 128],
                    _ => [3, 1, 0, -1],
                }
            } else {
                let dx = [-3, -1, 1, 4][((x + ordinal) % 4) as usize];
                let dy = [-3, 2, -1, 3][((y + ordinal) % 4) as usize];
                let yc = [128, 64, -1, 255][((x + y + ordinal) % 4) as usize];
                let uvc = [0, 192, 255, -1][((x + 2 * y + ordinal) % 4) as usize];
                [dx, dy, yc, uvc]
            };
            bytes.extend(
                flow.into_iter()
                    .flat_map(|value| (value as i16).to_le_bytes()),
            );
        }
    }
    bytes
}
