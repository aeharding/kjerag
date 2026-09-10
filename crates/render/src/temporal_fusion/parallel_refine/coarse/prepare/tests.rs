use super::*;
use crate::temporal_fusion::{
    parallel_refine::coarse,
    tests::{gpu as test_gpu, gpu_pair},
};
use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};
use wgpu::util::DeviceExt;

#[test]
fn shader_validates_without_optional_capabilities() {
    let module = wgpu::naga::front::wgsl::parse_str(include_str!("../prepare.wgsl")).unwrap();
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap();
}

#[test]
fn cpu_oracle_distinguishes_lowest_ties_joint_outliers_and_signed_truncation() {
    let disjoint = [[-4, 20, 1], [-4, 20, 2], [4, -6, 3], [4, -6, 4]];
    let result = coarse::prepare_next(&disjoint, [2, 2], [4, 4]).unwrap();
    assert_eq!(result.global, [-8, -12]);

    let negative = [[-2, -2, 11], [-1, -2, 12], [-1, -1, 13]];
    let result = coarse::prepare_next(&negative, [3, 1], [7, 3]).unwrap();
    assert_eq!(result.global, [-2, -3]);
}

#[test]
fn gpu_matches_cpu_for_odd_edges_and_all_six_references() {
    let Some((device, queue)) = test_gpu() else {
        return;
    };
    let blocks = [3, 2];
    let next = [7, 5];
    let records = reference_records();
    let expected: Vec<_> = records
        .iter()
        .map(|records| coarse::prepare_next(records, [3, 2], [7, 5]).unwrap())
        .collect();
    assert_eq!(expected[0].global, [6, -4]);
    assert!(
        expected[0]
            .seeds
            .iter()
            .all(|record| *record == [6, -4, 160])
    );
    let input = input(&device, blocks, &records);
    let builder = Builder::new(&device);
    let actual = run(&device, &queue, &builder, &input, next);

    assert_eq!(actual.blocks, next);
    assert_eq!(actual.references, 6);
    let expected_seeds: Vec<_> = expected
        .iter()
        .flat_map(|input| input.seeds.iter().copied())
        .collect();
    let expected_globals: Vec<_> = expected.iter().map(|input| input.global).collect();
    assert_eq!(decode_records(&actual.seeds), expected_seeds);
    assert_eq!(decode_globals(&actual.globals), expected_globals);
}

#[test]
fn rejects_foreign_device_bad_reference_count_raw_size_and_next_geometry() {
    let Some([(first_device, _first_queue), (second_device, _second_queue)]) = gpu_pair() else {
        return;
    };
    let builder = Builder::new(&first_device);
    let records = vec![vec![[0, 0, 0]; 4]];
    let foreign = input(&second_device, [2, 2], &records);
    let mut encoder = first_device.create_command_encoder(&Default::default());
    assert!(matches!(
        builder.encode(&first_device, &mut encoder, &foreign, [4, 4]),
        Err(Error::ForeignDevice)
    ));

    let empty = input(&first_device, [2, 2], &[]);
    assert!(matches!(
        builder.encode(&first_device, &mut encoder, &empty, [4, 4]),
        Err(Error::ReferenceCount(0))
    ));

    let malformed = gpu::Output {
        raw: first_device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("malformed coarse records"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }),
        device: first_device.clone(),
        blocks: [2, 2],
        references: 1,
    };
    assert!(matches!(
        builder.encode(&first_device, &mut encoder, &malformed, [4, 4]),
        Err(Error::UnsupportedSize)
    ));

    let valid = input(&first_device, [2, 2], &records);
    for next in [[3, 4], [6, 4], [4, 6]] {
        assert!(matches!(
            builder.encode(&first_device, &mut encoder, &valid, next),
            Err(Error::Geometry { .. })
        ));
    }

    let oversized = input(&first_device, [256, 1], &[vec![[0, 0, 0]; 256]]);
    assert!(matches!(
        builder.encode(&first_device, &mut encoder, &oversized, [512, 2]),
        Err(Error::Geometry { .. })
    ));
}

fn reference_records() -> Vec<Vec<[i32; 3]>> {
    vec![
        vec![[3, -2, 160]; 6],
        vec![
            [-4, 20, 1],
            [-4, 20, 2],
            [4, -6, 3],
            [4, -6, 4],
            [-4, 20, 5],
            [4, -6, 6],
        ],
        vec![
            [-2, -2, 11],
            [-1, -2, 12],
            [-1, -1, 13],
            [-2, -2, 14],
            [-1, -2, 15],
            [-1, -1, 16],
        ],
        vec![
            [0, 0, 0],
            [1, 2, 65_280],
            [2, 4, 17],
            [3, 6, 33],
            [4, 8, 49],
            [5, 10, 65],
        ],
        vec![
            [-100, 95, 91],
            [-99, 94, 92],
            [-98, 93, 93],
            [-97, 92, 94],
            [-96, 91, 95],
            [-95, 90, 96],
        ],
        vec![
            [17, -11, 101],
            [-5, 3, 102],
            [17, 3, 103],
            [-5, -11, 104],
            [17, -11, 105],
            [-5, 3, 106],
        ],
    ]
}

fn input(device: &wgpu::Device, blocks: [u32; 2], records: &[Vec<[i32; 3]>]) -> gpu::Output {
    let bytes: Vec<_> = records
        .iter()
        .flat_map(|records| {
            records
                .iter()
                .flat_map(|record| record.iter().flat_map(|value| value.to_le_bytes()))
        })
        .collect();
    let raw = if bytes.is_empty() {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("empty coarse records"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    } else {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("coarse preparation test records"),
            contents: &bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        })
    };
    gpu::Output {
        raw,
        device: device.clone(),
        blocks,
        references: records.len() as u32,
    }
}

struct Run {
    blocks: [u32; 2],
    references: u32,
    seeds: Vec<u8>,
    globals: Vec<u8>,
}

fn run(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    builder: &Builder,
    input: &gpu::Output,
    next: [u32; 2],
) -> Run {
    let mut encoder = device.create_command_encoder(&Default::default());
    let output = builder.encode(device, &mut encoder, input, next).unwrap();
    let seed_copy = readback(device, &mut encoder, output.seeds());
    let global_copy = readback(device, &mut encoder, output.globals());
    queue.submit([encoder.finish()]);
    Run {
        blocks: output.blocks(),
        references: output.reference_count(),
        seeds: read(device, &seed_copy),
        globals: read(device, &global_copy),
    }
}

fn readback(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    source: &wgpu::Buffer,
) -> wgpu::Buffer {
    let copy = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("coarse preparation test readback"),
        size: source.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(source, 0, &copy, 0, source.size());
    copy
}

fn read(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u8> {
    let (send, receive) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap()
        });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    receive.recv().unwrap().unwrap();
    let mapped = buffer.slice(..).get_mapped_range();
    let bytes = mapped.to_vec();
    drop(mapped);
    buffer.unmap();
    bytes
}

fn decode_records(bytes: &[u8]) -> Vec<[i32; 3]> {
    bytes
        .chunks_exact(12)
        .map(|record| {
            std::array::from_fn(|component| {
                let at = component * 4;
                i32::from_le_bytes(record[at..at + 4].try_into().unwrap())
            })
        })
        .collect()
}

fn decode_globals(bytes: &[u8]) -> Vec<[i32; 2]> {
    bytes
        .chunks_exact(8)
        .map(|record| {
            std::array::from_fn(|component| {
                let at = component * 4;
                i32::from_le_bytes(record[at..at + 4].try_into().unwrap())
            })
        })
        .collect()
}
