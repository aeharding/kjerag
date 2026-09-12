use super::*;
use crate::temporal_fusion::{parallel_refine, pyramid as cpu_pyramid, tests::gpu};

#[test]
fn level_count_gate_accepts_only_matching_five_through_seven_level_inputs() {
    assert_eq!(matching_level_count(5, [5, 5]).unwrap(), 5);
    assert_eq!(matching_level_count(6, [6, 6]).unwrap(), 6);
    assert_eq!(matching_level_count(7, [7, 7]).unwrap(), 7);
    assert!(matching_level_count(4, [4]).is_err());
    assert!(matching_level_count(8, [8]).is_err());
    assert!(matching_level_count(5, [6]).is_err());
    assert!(matching_level_count(6, [6, 7]).is_err());
    assert!(matching_level_count(7, [6]).is_err());
}

fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    levels: &[cpu_pyramid::Level],
) -> MotionPyramid {
    let textures = levels
        .iter()
        .map(|level| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("resident coarse oracle gray"),
                size: wgpu::Extent3d {
                    width: level.width as u32,
                    height: level.height as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Uint,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                &level.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(level.width as u32),
                    rows_per_image: None,
                },
                texture.size(),
            );
            texture
        })
        .collect();
    let mut encoder = device.create_command_encoder(&Default::default());
    let packed = MotionPyramid::encode_with_levels(
        device,
        &mut encoder,
        &pyramid::Builder::new(device),
        &pyramid::Output { levels: textures },
        levels.len(),
    )
    .unwrap();
    queue.submit([encoder.finish()]);
    packed
}

#[test]
fn five_level_review_runs_real_quarter_field_motion_bases_without_padding() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let builder = Builder::new(&device);
    for [width, height] in [[960, 480], [704, 352]] {
        let pixels: Vec<_> = (0..width * height)
            .map(|at| {
                let x = (at % width) as u32;
                let y = (at / width) as u32;
                x.wrapping_mul(1_664_525)
                    .wrapping_add(y.wrapping_mul(1_013_904_223))
                    .rotate_left((x ^ y) & 15) as u8
            })
            .collect();
        let levels = cpu_pyramid::build(&pixels, width, height, FIVE_LEVEL_REVIEW_LEVELS).unwrap();
        assert_eq!(levels.len(), FIVE_LEVEL_REVIEW_LEVELS);
        assert_eq!(
            [levels[4].width, levels[4].height],
            [width / 16, height / 16]
        );
        assert_eq!(width % 16, 0);
        assert_eq!(height % 16, 0);

        let current = upload(&device, &queue, &levels);
        let reference = upload(&device, &queue, &levels);
        let mut encoder = device.create_command_encoder(&Default::default());
        let output = builder
            .encode_motion(&device, &mut encoder, &current, &[&reference])
            .unwrap();
        assert_eq!(output.blocks(), [(width / 16) as u32, (height / 16) as u32]);
        assert_eq!(output.reference_count(), 1);
        queue.submit([encoder.finish()]);
        assert!(
            read_buffer(&device, &queue, &output.raw)
                .chunks_exact(3)
                .all(|record| record == [0, 0, 0]),
            "identical five-level {width} by {height} source produced motion"
        );
    }
}

fn read_buffer(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer) -> Vec<i32> {
    let copy = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("resident motion oracle readback"),
        size: buffer.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &copy, 0, buffer.size());
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    copy.slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap()
        });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    receive.recv().unwrap().unwrap();
    let values = copy
        .slice(..)
        .get_mapped_range()
        .chunks_exact(4)
        .map(|value| i32::from_le_bytes(value.try_into().unwrap()))
        .collect();
    copy.unmap();
    values
}

#[test]
fn resident_coarse_and_finest_match_the_independent_cpu_oracle() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let builder = Builder::new(&device);
    let finest = refine::Builder::new(&device);
    for [width, height] in [[1024, 1024], [1040, 1073]] {
        let pattern = |x: usize, y: usize| {
            let value = (x as u32)
                .wrapping_mul(1_664_525)
                .wrapping_add((y as u32).wrapping_mul(1_013_904_223));
            value.rotate_left(((x ^ y) & 15) as u32) as u8
        };
        let current: Vec<_> = (0..width * height)
            .map(|at| pattern(at % width, at / width))
            .collect();
        let current = cpu_pyramid::build(&current, width, height, FULL_RESOLUTION_LEVELS).unwrap();
        let references: Vec<_> = (0..6)
            .map(|ordinal| {
                let pixels: Vec<_> = (0..width * height)
                    .map(|at| {
                        let x = at % width;
                        let y = at / width;
                        if ordinal == 0 {
                            pattern(x, y)
                        } else {
                            pattern((x + ordinal * 3) % width, (y + ordinal * 2) % height)
                                .saturating_add(ordinal as u8)
                        }
                    })
                    .collect();
                cpu_pyramid::build(&pixels, width, height, FULL_RESOLUTION_LEVELS).unwrap()
            })
            .collect();
        let current_gpu = upload(&device, &queue, &current);
        let reference_gpu: Vec<_> = references
            .iter()
            .map(|reference| upload(&device, &queue, reference))
            .collect();
        let expected: Vec<_> = references
            .iter()
            .map(|reference| parallel_refine::coarse::prepare_finest(&current, reference).unwrap())
            .collect();
        for count in [1, 3, 6] {
            let refs: Vec<_> = reference_gpu[..count].iter().collect();
            let mut encoder = device.create_command_encoder(&Default::default());
            let prepared = builder
                .encode_finest_inputs(&device, &mut encoder, &current_gpu, &refs)
                .unwrap();
            let finest_refs: Vec<_> = refs.iter().map(|reference| reference.finest()).collect();
            let output = finest
                .encode_finest_resident(
                    &device,
                    &mut encoder,
                    current_gpu.finest(),
                    &finest_refs,
                    &prepared,
                )
                .unwrap();
            queue.submit([encoder.finish()]);
            let seeds: Vec<_> = expected[..count]
                .iter()
                .flat_map(|input| input.seeds.iter().flatten().copied())
                .collect();
            let globals: Vec<_> = expected[..count]
                .iter()
                .flat_map(|input| input.global)
                .collect();
            assert_eq!(
                read_buffer(&device, &queue, prepared.seeds()),
                seeds,
                "seeds {width}x{height}, {count} refs"
            );
            assert_eq!(
                read_buffer(&device, &queue, prepared.globals()),
                globals,
                "globals {width}x{height}, {count} refs"
            );
            let raw: Vec<_> = references[..count]
                .iter()
                .zip(&expected)
                .flat_map(|(reference, input)| {
                    parallel_refine::finest(&current[0], &reference[0], input)
                        .unwrap()
                        .into_iter()
                        .flatten()
                })
                .collect();
            assert_eq!(
                read_buffer(&device, &queue, &output.raw),
                raw,
                "finest {width}x{height}, {count} refs"
            );
        }
    }
}

#[test]
fn resident_coarse_rejects_bad_reference_count_before_encoding() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let levels =
        cpu_pyramid::build(&vec![0; 1024 * 1024], 1024, 1024, FULL_RESOLUTION_LEVELS).unwrap();
    let current = upload(&device, &queue, &levels);
    let builder = Builder::new(&device);
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(
        builder
            .encode_finest_inputs(&device, &mut encoder, &current, &[])
            .is_err()
    );
    assert!(
        builder
            .encode_finest_inputs(&device, &mut encoder, &current, &[&current; 7])
            .is_err()
    );
    builder
        .encode_finest_inputs(&device, &mut encoder, &current, &[&current])
        .unwrap();
    queue.submit([encoder.finish()]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
}
