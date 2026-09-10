use super::{Builder, Error, validate_reference_count};
use crate::temporal_fusion::{
    motion::{self, Geometry, Parameters},
    parallel_refine,
    pyramid::{
        Level,
        gpu::{Builder as Packer, PackedGray},
    },
    search::{self, FinestInput},
    tests::{copy_texture, gpu, gpu_pair, read_copy},
};
use std::time::Instant;
use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

#[test]
fn shader_validates_without_optional_capabilities() {
    let module = wgpu::naga::front::wgsl::parse_str(include_str!("../gpu.wgsl")).unwrap();
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap();
}

fn patterned_level(width: usize, height: usize, salt: u32) -> Level {
    let pixels = (0..width * height)
        .map(|at| {
            let x = (at % width) as u32;
            let y = (at / width) as u32;
            x.wrapping_mul(1_664_525)
                .wrapping_add(y.wrapping_mul(1_013_904_223))
                .wrapping_add(salt)
                .rotate_left((x ^ y) & 15) as u8
        })
        .collect();
    Level {
        width,
        height,
        pixels,
    }
}

fn inputs(width: usize, height: usize) -> [FinestInput; 6] {
    let count = (width / 16) * (height / 16);
    std::array::from_fn(|ordinal| FinestInput {
        seeds: (0..count)
            .map(|index| {
                [
                    ((index * 13 + ordinal * 7) % 31) as i32 - 15,
                    ((index * 17 + ordinal * 3) % 29) as i32 - 14,
                    ((index * 97 + ordinal * 101) % 65_281) as i32,
                ]
            })
            .collect(),
        global: [ordinal as i32 - 3, 2 - ordinal as i32],
    })
}

fn upload_gray(device: &wgpu::Device, queue: &wgpu::Queue, level: &Level) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("parallel finest test luma"),
        size: wgpu::Extent3d {
            width: level.width as u32,
            height: level.height as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Uint,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        &level.pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(level.width as u32),
            rows_per_image: Some(level.height as u32),
        },
        texture.size(),
    );
    texture
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, level: &Level) -> PackedGray {
    let texture = upload_gray(device, queue, level);
    let mut encoder = device.create_command_encoder(&Default::default());
    let packed = Packer::new(device)
        .encode_packed_base(device, &mut encoder, &texture)
        .unwrap();
    // The source is packed once and reused across refinement repeats. Queue
    // ordering makes it available without a host wait or per-output repack.
    queue.submit([encoder.finish()]);
    packed
}

fn raw_bytes(records: &[[i32; 3]]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|record| record.iter().flat_map(|value| value.to_le_bytes()))
        .collect()
}

fn packed_bytes(records: &[[i16; 4]]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|record| record.iter().flat_map(|value| value.to_le_bytes()))
        .collect()
}

fn expected(current: &Level, references: &[Level; 6], inputs: &[FinestInput; 6]) -> [Vec<u8>; 6] {
    std::array::from_fn(|ordinal| {
        raw_bytes(
            &parallel_refine::finest(current, &references[ordinal], &inputs[ordinal]).unwrap(),
        )
    })
}

struct Run {
    bytes: Vec<u8>,
    blocks: [u32; 2],
    references: u32,
    encode_ms: f64,
    execute_readback_ms: f64,
}

fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    builder: &Builder,
    current: &PackedGray,
    references: &[&PackedGray],
    inputs: &[FinestInput],
) -> Run {
    let started = Instant::now();
    let mut encoder = device.create_command_encoder(&Default::default());
    let seeds: Vec<_> = inputs.iter().map(|input| input.seeds.as_slice()).collect();
    let globals: Vec<_> = inputs.iter().map(|input| input.global).collect();
    let output = builder
        .encode_finest(device, &mut encoder, current, references, &seeds, &globals)
        .unwrap();
    let copy = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("parallel finest test readback"),
        size: output.raw.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(&output.raw, 0, &copy, 0, output.raw.size());
    let command = encoder.finish();
    let encode_ms = started.elapsed().as_secs_f64() * 1000.0;
    let submitted = Instant::now();
    queue.submit([command]);
    let (send, receive) = std::sync::mpsc::channel();
    copy.slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap()
        });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    receive.recv().unwrap().unwrap();
    let execute_readback_ms = submitted.elapsed().as_secs_f64() * 1000.0;
    let mapped = copy.slice(..).get_mapped_range();
    let bytes = mapped.to_vec();
    drop(mapped);
    copy.unmap();
    Run {
        bytes,
        blocks: output.blocks,
        references: output.reference_count(),
        encode_ms,
        execute_readback_ms,
    }
}

#[test]
fn reference_count_rejects_empty_oversized_and_unpaired_inputs() {
    assert!(matches!(
        validate_reference_count(0, 0, 0),
        Err(Error::ReferenceCount { .. })
    ));
    assert!(matches!(
        validate_reference_count(7, 7, 7),
        Err(Error::ReferenceCount { .. })
    ));
    assert!(matches!(
        validate_reference_count(3, 2, 3),
        Err(Error::ReferenceCount { .. })
    ));
    assert!(matches!(
        validate_reference_count(3, 3, 2),
        Err(Error::ReferenceCount { .. })
    ));
    for count in 1..=6 {
        assert_eq!(
            validate_reference_count(count, count, count).unwrap(),
            count
        );
    }
}

#[test]
fn every_reduced_reference_count_matches_the_same_six_reference_prefix() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let current_level = patterned_level(1024, 1024, 0x1234_5678);
    let reference_levels: [Level; 6] = std::array::from_fn(|ordinal| {
        patterned_level(1024, 1024, 0x9e37_79b9_u32.wrapping_mul(ordinal as u32 + 1))
    });
    let inputs = inputs(1024, 1024);
    let current = upload(&device, &queue, &current_level);
    let references = reference_levels
        .each_ref()
        .map(|reference| upload(&device, &queue, reference));
    let reference_views = references.each_ref();
    let builder = Builder::new(&device);
    let full = run(
        &device,
        &queue,
        &builder,
        &current,
        &reference_views,
        &inputs,
    );
    assert_eq!(full.references, 6);
    let bytes_per_reference = full.bytes.len() / 6;

    for count in 1..=6 {
        let reduced = run(
            &device,
            &queue,
            &builder,
            &current,
            &reference_views[..count],
            &inputs[..count],
        );
        assert_eq!(reduced.references, count as u32);
        assert_eq!(
            reduced.bytes,
            full.bytes[..count * bytes_per_reference],
            "reduced reference count {count}"
        );
    }
}

fn assert_outputs(actual: &[u8], expected: &[Vec<u8>; 6]) {
    let per_reference = expected[0].len();
    assert!(
        expected
            .iter()
            .all(|records| records.len() == per_reference)
    );
    assert_eq!(actual.len(), per_reference * 6);
    for (ordinal, expected) in expected.iter().enumerate() {
        let actual = &actual[ordinal * per_reference..(ordinal + 1) * per_reference];
        if actual != expected {
            let differing: Vec<_> = actual
                .chunks_exact(12)
                .zip(expected.chunks_exact(12))
                .enumerate()
                .filter_map(|(index, (got, wanted))| (got != wanted).then_some(index))
                .take(8)
                .collect();
            panic!(
                "parallel-refine reference {ordinal} differs; first record indices {differing:?}"
            );
        }
    }
}

#[test]
fn patterned_tail_geometry_matches_the_cpu_oracle() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let current = patterned_level(1031, 1027, 0x1234_5678);
    let references = std::array::from_fn(|ordinal| {
        patterned_level(1031, 1027, 0x9e37_79b9_u32.wrapping_mul(ordinal as u32 + 1))
    });
    let inputs = inputs(current.width, current.height);
    let expected = expected(&current, &references, &inputs);
    let current = upload(&device, &queue, &current);
    let references = references
        .each_ref()
        .map(|reference| upload(&device, &queue, reference));
    let run = run(
        &device,
        &queue,
        &Builder::new(&device),
        &current,
        &references.each_ref(),
        &inputs,
    );
    assert_eq!(run.blocks, [64, 64]);
    assert_outputs(&run.bytes, &expected);
}

#[test]
fn packed_tail_lanes_and_signed_edges_match_the_cpu_oracle() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let builder = Builder::new(&device);
    for tail in 1..=3 {
        let width = 1024 + tail;
        let current = patterned_level(width, width, 0x1234_5678);
        let mut references: [Level; 6] =
            std::array::from_fn(|ordinal| patterned_level(width, width, ordinal as u32 * 7919));
        let mut inputs = inputs(width, width);
        // Exact copied blocks force known unaligned negative starts and the
        // inclusive final row/column, independently of random search winners.
        let cases = [
            ([32usize, 32usize], [-3i32, -2i32]),
            ([63, 63], [tail as i32, tail as i32]),
        ];
        for (reference, input) in references.iter_mut().zip(&mut inputs) {
            for (block, shift) in cases {
                let [x, y] = block.map(|value| value * 16);
                let rx = (x as i32 + shift[0]) as usize;
                let ry = (y as i32 + shift[1]) as usize;
                for row in 0..16 {
                    reference.pixels[(ry + row) * width + rx..(ry + row) * width + rx + 16]
                        .copy_from_slice(
                            &current.pixels[(y + row) * width + x..(y + row) * width + x + 16],
                        );
                }
                input.seeds[block[1] * 64 + block[0]] = [shift[0], shift[1], 0];
            }
        }
        let expected = expected(&current, &references, &inputs);
        for plane in &expected {
            for (block, shift) in cases {
                let at = (block[1] * 64 + block[0]) * 12;
                assert_eq!(&plane[at..at + 12], raw_bytes(&[[shift[0], shift[1], 0]]));
            }
        }
        let current = upload(&device, &queue, &current);
        let references = references
            .each_ref()
            .map(|image| upload(&device, &queue, image));
        let result = run(
            &device,
            &queue,
            &builder,
            &current,
            &references.each_ref(),
            &inputs,
        );
        assert_outputs(&result.bytes, &expected);
    }
}

#[test]
fn packed_max_sad_keeps_start_order_at_the_inclusive_corner() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let width = 1025;
    let current = Level {
        width,
        height: width,
        pixels: vec![0; width * width],
    };
    let references = std::array::from_fn(|_| Level {
        width,
        height: width,
        pixels: vec![255; width * width],
    });
    let inputs = std::array::from_fn(|_| FinestInput {
        seeds: vec![[1008, 1008, 0]; 64 * 64],
        global: [1009, 1009],
    });
    let expected = expected(&current, &references, &inputs);
    for plane in &expected {
        // Global and own have the maximum raw SAD and no start penalty.
        // Global's earlier ordinal wins, even at the inclusive logical edge.
        assert_eq!(&plane[..12], raw_bytes(&[[1009, 1009, 65_280]]));
    }
    let current = upload(&device, &queue, &current);
    let references = references
        .each_ref()
        .map(|image| upload(&device, &queue, image));
    let result = run(
        &device,
        &queue,
        &Builder::new(&device),
        &current,
        &references.each_ref(),
        &inputs,
    );
    assert_outputs(&result.bytes, &expected);
}

#[test]
fn packed_images_from_another_device_are_refused_before_encoding() {
    let Some([(device, queue), (foreign, foreign_queue)]) = gpu_pair() else {
        return;
    };
    let level = patterned_level(1024, 1024, 7);
    let local = upload(&device, &queue, &level);
    let other = upload(&foreign, &foreign_queue, &level);
    let inputs = inputs(1024, 1024);
    let builder = Builder::new(&device);
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut references = [&local; 6];
    references[4] = &other;
    for (current, references) in [(&other, [&local; 6]), (&local, references)] {
        assert!(matches!(
            builder.encode_finest(
                &device,
                &mut encoder,
                current,
                &references,
                &inputs.each_ref().map(|input| input.seeds.as_slice()),
                &inputs.each_ref().map(|input| input.global)
            ),
            Err(Error::ForeignDevice)
        ));
    }
    builder
        .encode_finest(
            &device,
            &mut encoder,
            &local,
            &[&local; 6],
            &inputs.each_ref().map(|input| input.seeds.as_slice()),
            &inputs.each_ref().map(|input| input.global),
        )
        .unwrap();
    queue.submit([encoder.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    foreign.poll(wgpu::PollType::wait_indefinitely()).unwrap();
}

#[test]
fn invalid_input_is_refused_before_a_following_valid_encode() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let builder = Builder::new(&device);
    let level = patterned_level(1024, 1024, 7);
    let texture = upload(&device, &queue, &level);
    let wrong_size = upload(&device, &queue, &patterned_level(1025, 1024, 9));
    let mut inputs = inputs(1024, 1024);
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut references = [&texture; 6];
    references[3] = &wrong_size;
    assert!(matches!(
        builder.encode_finest(
            &device,
            &mut encoder,
            &texture,
            &references,
            &inputs.each_ref().map(|input| input.seeds.as_slice()),
            &inputs.each_ref().map(|input| input.global),
        ),
        Err(Error::Geometry)
    ));
    inputs[4].seeds[17][2] = 65_281;
    assert!(matches!(
        builder.encode_finest(
            &device,
            &mut encoder,
            &texture,
            &[&texture; 6],
            &inputs.each_ref().map(|input| input.seeds.as_slice()),
            &inputs.each_ref().map(|input| input.global),
        ),
        Err(Error::SeedValue {
            ordinal: 4,
            index: 17
        })
    ));
    inputs[4].seeds[17][2] = 0;
    builder
        .encode_finest(
            &device,
            &mut encoder,
            &texture,
            &[&texture; 6],
            &inputs.each_ref().map(|input| input.seeds.as_slice()),
            &inputs.each_ref().map(|input| input.global),
        )
        .unwrap();
    queue.submit([encoder.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
}

#[test]
fn duplicate_initial_sads_keep_each_candidates_penalty_and_tie_order() {
    // Zero is penalized, but its duplicate global/own starts are not. An
    // incorrectly retained zero penalty would let the +1 ring candidate win:
    // 2304 + floor(50*2304/256) = 2754, versus the correct unpenalized 2560.
    duplicate_sad_case(10, 0, [0, 0, 2560]);
}

#[test]
fn ring_sad_reuse_still_applies_the_new_candidates_penalty() {
    // The initial +1 start costs2560 and beats penalized zero (2304+450).
    // Ring W revisits zero: reusing its raw2304 without the ring penalty
    // would incorrectly replace +1. Matching raw costs do not share policy.
    duplicate_sad_case(-9, 1, [1, 0, 2560]);
}

fn duplicate_sad_case(bias: i32, seed_x: i32, center_expected: [i32; 3]) {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let width = 1024;
    let height = 1024;
    let reference = Level {
        width,
        height,
        pixels: (0..width * height)
            .map(|at| 64 + (at % width % 64) as u8)
            .collect(),
    };
    let current = Level {
        width,
        height,
        pixels: reference
            .pixels
            .iter()
            .map(|value| u8::try_from(i32::from(*value) + bias).unwrap())
            .collect(),
    };
    let references = std::array::from_fn(|_| reference.clone());
    let inputs = std::array::from_fn(|_| FinestInput {
        seeds: vec![[seed_x, 0, 0]; (width / 16) * (height / 16)],
        global: [seed_x, 0],
    });
    let expected = expected(&current, &references, &inputs);
    let center_index = 32 * (width / 16) + 32;
    let wanted = raw_bytes(&[center_expected]);
    assert_eq!(
        &expected[0][center_index * 12..(center_index + 1) * 12],
        wanted,
        "the independent oracle must exercise the intended penalty case"
    );
    // Compare every cell, including edges where inclusive start clipping
    // creates duplicates but the checked predictors/ring must stay invalid.
    let current = upload(&device, &queue, &current);
    let references = references
        .each_ref()
        .map(|reference| upload(&device, &queue, reference));
    let actual = run(
        &device,
        &queue,
        &Builder::new(&device),
        &current,
        &references.each_ref(),
        &inputs,
    );
    assert_outputs(&actual.bytes, &expected);
}

fn motion_parameters<'a>(
    raw_grid: [u32; 2],
    luma: &'a [u8],
    y: &'a [f32; 256],
    uv: &'a [f32; 256],
) -> Parameters<'a> {
    Parameters {
        geometry: Geometry {
            full: raw_grid.map(|value| value * 32),
            raw_grid,
            output_grid: raw_grid.map(|value| value * 2),
            block: [16, 16],
        },
        luma,
        confidence_y: y,
        confidence_uv: uv,
        scale_base: 1,
        scale_extra: 1,
        temporal: 1.0,
        phase: 0.0,
    }
}

#[test]
fn typed_refinement_handoff_packs_all_six_offsets_in_one_submission() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let current_level = patterned_level(1024, 1024, 0x1020_3040);
    let reference_levels: [Level; 6] =
        std::array::from_fn(|ordinal| patterned_level(1024, 1024, ordinal as u32 * 7919));
    let inputs = inputs(1024, 1024);
    let cpu_raw: [Vec<[i32; 3]>; 6] = std::array::from_fn(|ordinal| {
        parallel_refine::finest(&current_level, &reference_levels[ordinal], &inputs[ordinal])
            .unwrap()
    });
    for ordinal in 1..6 {
        assert_ne!(
            cpu_raw[ordinal], cpu_raw[0],
            "synthetic reference {ordinal} must exercise a distinct raw-buffer offset"
        );
    }
    let current = upload_gray(&device, &queue, &current_level);
    let references = reference_levels
        .each_ref()
        .map(|reference| upload_gray(&device, &queue, reference));
    let refine = Builder::new(&device);
    let motion = motion::gpu::Builder::new(&device);
    let luma = vec![0; 128 * 128];
    let y = [30_000.0; 256];
    let uv = [20_000.0; 256];
    let parameters = motion_parameters([64, 64], &luma, &y, &uv);
    let mut encoder = device.create_command_encoder(&Default::default());
    let packer = Packer::new(&device);
    let current = packer
        .encode_packed_base(&device, &mut encoder, &current)
        .unwrap();
    let references = references.each_ref().map(|image| {
        packer
            .encode_packed_base(&device, &mut encoder, image)
            .unwrap()
    });
    let reference_views = references.each_ref();
    let seeds = inputs.each_ref().map(|input| input.seeds.as_slice());
    let globals = inputs.each_ref().map(|input| input.global);
    let refined = refine
        .encode_finest(
            &device,
            &mut encoder,
            &current,
            &reference_views,
            &seeds,
            &globals,
        )
        .unwrap();
    let reduced = refine
        .encode_finest(
            &device,
            &mut encoder,
            &current,
            &reference_views[..2],
            &seeds[..2],
            &globals[..2],
        )
        .unwrap();
    let reduced_packed = motion
        .encode_refined(&device, &mut encoder, &reduced, 1, &parameters)
        .unwrap();
    let reduced_copy = copy_texture(
        &device,
        &mut encoder,
        &reduced_packed,
        parameters.geometry.output_grid,
        8,
    );
    assert!(
        motion
            .encode_refined(&device, &mut encoder, &reduced, 2, &parameters)
            .is_err()
    );
    let mut copies = Vec::new();
    let mut expected = Vec::new();
    for ordinal in 0u32..6 {
        let index = ordinal as usize;
        let packed = motion
            .encode_refined(&device, &mut encoder, &refined, ordinal, &parameters)
            .unwrap();
        copies.push(copy_texture(
            &device,
            &mut encoder,
            &packed,
            parameters.geometry.output_grid,
            8,
        ));
        expected.push(packed_bytes(
            &motion::pack_motion(&cpu_raw[index], &parameters).unwrap(),
        ));
    }
    assert!(
        expected.iter().skip(1).any(|value| value != &expected[0]),
        "synthetic packed references must exercise distinct raw-buffer offsets"
    );
    assert!(
        motion
            .encode_refined(&device, &mut encoder, &refined, 6, &parameters)
            .is_err()
    );
    let mismatched = motion_parameters([63, 64], &luma[..126 * 128], &y, &uv);
    assert!(
        motion
            .encode_refined(&device, &mut encoder, &refined, 0, &mismatched)
            .is_err()
    );

    queue.submit([encoder.finish()]);
    assert_eq!(
        read_copy(&device, &reduced_copy, parameters.geometry.output_grid, 8),
        packed_bytes(&motion::pack_motion(&cpu_raw[1], &parameters).unwrap())
    );
    for (ordinal, (copy, expected)) in copies.iter().zip(expected).enumerate() {
        let actual = read_copy(&device, copy, parameters.geometry.output_grid, 8);
        assert_eq!(actual.len(), expected.len());
        assert_eq!(
            actual.iter().zip(&expected).position(|(a, b)| a != b),
            None,
            "typed refinement handoff reference {ordinal}"
        );
    }
}

fn report_serial_differences(actual: &[u8], serial: &[Vec<u8>; 6]) {
    let per_reference = actual.len() / 6;
    for (ordinal, old) in serial.iter().enumerate() {
        let new = &actual[ordinal * per_reference..(ordinal + 1) * per_reference];
        let different = new
            .chunks_exact(12)
            .zip(old.chunks_exact(12))
            .filter(|(new, old)| new != old)
            .count();
        eprintln!(
            "parallel candidate vs sealed selected serial reference {ordinal}: {different}/{} vectors differ (diagnostic, not failure)",
            new.len() / 12
        );
    }
}

#[test]
#[ignore = "requires sealed private search fixtures"]
fn native_six_reference_oracle_is_exact_and_timed_three_times() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let fixture = search::tests::native_fixture();
    let coarse_started = Instant::now();
    let inputs = fixture
        .references
        .each_ref()
        .map(|reference| search::prepare_finest(&fixture.current, reference).unwrap());
    eprintln!(
        "parallel candidate CPU coarse preparation for six references: {:.3}ms",
        coarse_started.elapsed().as_secs_f64() * 1000.0
    );
    let expected: [Vec<u8>; 6] = std::array::from_fn(|ordinal| {
        raw_bytes(
            &parallel_refine::finest(
                &fixture.current[0],
                &fixture.references[ordinal][0],
                &inputs[ordinal],
            )
            .unwrap(),
        )
    });
    let current = upload(&device, &queue, &fixture.current[0]);
    let references = fixture
        .references
        .each_ref()
        .map(|levels| upload(&device, &queue, &levels[0]));
    let pipeline_started = Instant::now();
    let builder = Builder::new(&device);
    eprintln!(
        "parallel candidate GPU pipeline construction: {:.3}ms",
        pipeline_started.elapsed().as_secs_f64() * 1000.0
    );
    for repeat in 1..=3 {
        let result = run(
            &device,
            &queue,
            &builder,
            &current,
            &references.each_ref(),
            &inputs,
        );
        eprintln!(
            "parallel candidate repeat {repeat}: encode/upload {:.3}ms; submit+completion+readback {:.3}ms",
            result.encode_ms, result.execute_readback_ms
        );
        assert_outputs(&result.bytes, &expected);
        report_serial_differences(&result.bytes, &fixture.expected);
    }
}
