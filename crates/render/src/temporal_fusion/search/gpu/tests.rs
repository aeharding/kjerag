use super::{Builder, Error};
use crate::temporal_fusion::pyramid::Level;
use crate::temporal_fusion::search::{self, FinestInput};
use crate::temporal_fusion::tests::gpu;
use std::time::Instant;
use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

#[test]
fn shader_validates_without_optional_capabilities() {
    let module = wgpu::naga::front::wgsl::parse_str(include_str!("../gpu.wgsl")).unwrap();
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap();
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, level: &Level) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("finest search test luma"),
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

fn bytes(records: &[[i32; 3]]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|record| record.iter().flat_map(|value| value.to_le_bytes()))
        .collect()
}

fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    builder: &Builder,
    current: &wgpu::Texture,
    references: [&wgpu::Texture; 6],
    inputs: &[FinestInput; 6],
) -> Vec<u8> {
    let started = Instant::now();
    let mut encoder = device.create_command_encoder(&Default::default());
    let output = builder
        .encode_finest(
            device,
            &mut encoder,
            current,
            references,
            inputs.each_ref().map(|input| input.seeds.as_slice()),
            inputs.each_ref().map(|input| input.global),
        )
        .unwrap();
    let copy = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("finest search fixture readback"),
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
    let data = copy.slice(..).get_mapped_range().to_vec();
    copy.unmap();
    eprintln!(
        "finest GPU diagnostic: {}x{} blocks, encode/upload preparation {encode_ms:.3}ms, submit+completion+readback {execute_readback_ms:.3}ms, {} bytes",
        output.blocks[0],
        output.blocks[1],
        data.len(),
    );
    data
}

fn assert_outputs(actual: &[u8], expected: &[Vec<u8>; 6]) {
    let per_reference = expected[0].len();
    assert_eq!(actual.len(), per_reference * 6);
    for (ordinal, expected) in expected.iter().enumerate() {
        let actual = &actual[ordinal * per_reference..(ordinal + 1) * per_reference];
        if actual != expected {
            let differing: Vec<_> = actual
                .chunks_exact(12)
                .zip(expected.chunks_exact(12))
                .enumerate()
                .filter_map(|(index, (a, b))| (a != b).then_some(index))
                .collect();
            let first = differing[0];
            let lanes = |record: &[u8]| -> [i32; 3] {
                std::array::from_fn(|at| {
                    i32::from_le_bytes(record[at * 4..at * 4 + 4].try_into().unwrap())
                })
            };
            panic!(
                "reference {ordinal}: {} different records; first {first} actual {:?} expected {:?}",
                differing.len(),
                lanes(&actual[first * 12..first * 12 + 12]),
                lanes(&expected[first * 12..first * 12 + 12])
            );
        }
    }
}

#[test]
fn six_reference_order_and_ties_match_the_serial_controller() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    let current = Level {
        width: 1024,
        height: 1024,
        pixels: vec![73; 1024 * 1024],
    };
    let references = [10, 42, 73, 94, 121, 255].map(|value| Level {
        width: 1024,
        height: 1024,
        pixels: vec![value; 1024 * 1024],
    });
    let inputs = std::array::from_fn(|ordinal| FinestInput {
        seeds: (0..4096)
            .map(|at| {
                [
                    ((at * 13 + ordinal) % 31) as i32 - 15,
                    (at % 19) as i32 - 9,
                    0,
                ]
            })
            .collect(),
        global: [ordinal as i32 - 3, 2 - ordinal as i32],
    });
    let expected = std::array::from_fn(|ordinal| {
        bytes(&search::tests::finish_finest(
            &current,
            &references[ordinal],
            &inputs[ordinal],
        ))
    });
    let current = upload(&device, &queue, &current);
    let references = references
        .each_ref()
        .map(|level| upload(&device, &queue, level));
    let actual = run(
        &device,
        &queue,
        &builder,
        &current,
        references.each_ref(),
        &inputs,
    );
    assert_outputs(&actual, &expected);
}

#[test]
fn adaptive_umh_and_partial_edge_blocks_match_the_serial_controller() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    // Third block must enter UMH; its match is beyond the fixed range's
    // largest horizontal candidate and needs adaptive Hex movement.
    let current = Level {
        width: 1031,
        height: 1027,
        pixels: vec![255; 1031 * 1027],
    };
    let references = std::array::from_fn::<_, 6, _>(|ordinal| {
        let mut pixels = vec![0; 1031 * 1027];
        for y in 0..16 {
            for x in 61 + ordinal..77 + ordinal {
                pixels[y * 1031 + x] = 255;
            }
        }
        Level {
            width: 1031,
            height: 1027,
            pixels,
        }
    });
    let inputs = std::array::from_fn(|_| FinestInput {
        seeds: vec![[0, 0, 0]; 4096],
        global: [0, 0],
    });
    let expected_records = std::array::from_fn::<_, 6, _>(|ordinal| {
        search::tests::finish_finest(&current, &references[ordinal], &inputs[ordinal])
    });
    assert_eq!(expected_records[0][2], [29, 0, 0]);
    let expected = expected_records.each_ref().map(|records| bytes(records));
    let current = upload(&device, &queue, &current);
    let references = references
        .each_ref()
        .map(|level| upload(&device, &queue, level));
    let actual = run(
        &device,
        &queue,
        &builder,
        &current,
        references.each_ref(),
        &inputs,
    );
    assert_outputs(&actual, &expected);
}

#[test]
fn future_seed_and_searched_left_up_dependencies_match_the_serial_controller() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    let pixels = (0..1024usize * 1024)
        .map(|at| {
            let value = (at as u32)
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            ((value ^ (value >> 13)) >> 8) as u8
        })
        .collect();
    let current = Level {
        width: 1024,
        height: 1024,
        pixels,
    };
    let references = std::array::from_fn::<_, 6, _>(|ordinal| {
        let shift = 5 + ordinal;
        let pixels = (0..1024usize * 1024)
            .map(|at| current.pixels[(at / 1024) * 1024 + (at % 1024 + 1024 - shift) % 1024])
            .collect();
        Level {
            width: 1024,
            height: 1024,
            pixels,
        }
    });
    let inputs = std::array::from_fn(|ordinal| {
        let mut seeds = vec![[0, 0, 0]; 4096];
        // Row0/block0 reads row1/block1 before that seed is searched.
        seeds[65] = [5 + ordinal as i32, 0, 0];
        FinestInput {
            seeds,
            global: [0, 0],
        }
    });
    let expected_records = std::array::from_fn::<_, 6, _>(|ordinal| {
        search::tests::finish_finest(&current, &references[ordinal], &inputs[ordinal])
    });
    let unseeded = FinestInput {
        seeds: vec![[0, 0, 0]; 4096],
        global: [0, 0],
    };
    assert_ne!(
        search::tests::finish_finest(&current, &references[0], &unseeded)[0],
        expected_records[0][0]
    );
    for (ordinal, records) in expected_records.iter().enumerate() {
        let wanted = [5 + ordinal as i32, 0, 0];
        assert_eq!(records[0], wanted, "future diagonal seed");
        assert_eq!(records[1], wanted, "searched left neighbor");
        assert_eq!(
            records[64], wanted,
            "searched up neighbor across row dispatches"
        );
    }
    let expected = expected_records.each_ref().map(|records| bytes(records));
    let current = upload(&device, &queue, &current);
    let references = references
        .each_ref()
        .map(|level| upload(&device, &queue, level));
    assert_outputs(
        &run(
            &device,
            &queue,
            &builder,
            &current,
            references.each_ref(),
            &inputs,
        ),
        &expected,
    );
}

#[test]
fn invalid_seed_is_refused_before_a_valid_encode() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    let level = Level {
        width: 1024,
        height: 1024,
        pixels: vec![0; 1024 * 1024],
    };
    let texture = upload(&device, &queue, &level);
    let mut inputs = std::array::from_fn::<_, 6, _>(|_| FinestInput {
        seeds: vec![[0, 0, 0]; 4096],
        global: [0, 0],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    inputs[4].seeds[17][2] = 65281;
    assert!(matches!(
        builder.encode_finest(
            &device,
            &mut encoder,
            &texture,
            [&texture; 6],
            inputs.each_ref().map(|input| input.seeds.as_slice()),
            [[0, 0]; 6],
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
            [&texture; 6],
            inputs.each_ref().map(|input| input.seeds.as_slice()),
            [[0, 0]; 6],
        )
        .unwrap();
    queue.submit([encoder.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
}

#[test]
#[ignore = "requires sealed private inputs and combined adapter outputs"]
fn matches_saved_six_reference_search_and_reports_timings() {
    let Some((device, queue)) = gpu() else { return };
    let fixture = search::tests::native_fixture();
    let started = Instant::now();
    let inputs = fixture
        .references
        .each_ref()
        .map(|reference| search::prepare_finest(&fixture.current, reference).unwrap());
    eprintln!(
        "serial CPU coarse preparation for six references: {:.3}ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    let current = upload(&device, &queue, &fixture.current[0]);
    let references = fixture
        .references
        .each_ref()
        .map(|levels| upload(&device, &queue, &levels[0]));
    let started = Instant::now();
    let builder = Builder::new(&device);
    eprintln!(
        "GPU finest pipeline construction: {:.3}ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    for repeat in 0..3 {
        eprintln!("saved six-reference repeat {repeat}");
        let actual = run(
            &device,
            &queue,
            &builder,
            &current,
            references.each_ref(),
            &inputs,
        );
        assert_outputs(&actual, &fixture.expected);
    }
    eprintln!(
        "all 172,800 raw vectors match the sealed combined-adapter outputs in all three runs"
    );
}
