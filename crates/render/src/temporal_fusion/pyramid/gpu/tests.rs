use super::Builder;
use crate::temporal_fusion::pyramid::{self, Level};
use crate::temporal_fusion::tests::{copy_texture, gpu, read_copy};
use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

#[test]
fn shader_validates_without_optional_capabilities() {
    let module = wgpu::naga::front::wgsl::parse_str(include_str!("../gpu.wgsl")).unwrap();
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap();
}

#[test]
fn half_luma_preserves_every_byte_and_rounds_every_possible_sum() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut constant_blocks = vec![0; 512 * 2];
    for y in 0..2 {
        for x in 0..512 {
            constant_blocks[y * 512 + x] = (x / 2) as u8;
        }
    }
    let source = upload(
        &device,
        &queue,
        &constant_blocks,
        [512, 2],
        wgpu::TextureFormat::R8Unorm,
    );
    let preserved = builder
        .encode_luma(&device, &mut encoder, &source, 1)
        .unwrap();
    let preserved_copy = copy_texture(&device, &mut encoder, &preserved.levels[0], [256, 1], 1);

    // Every possible sum of four bytes, placed in its own nonoverlapping
    // footprint. In particular sums 1 and 2 distinguish one final rounding
    // from independently rounded vertical/horizontal pair averages.
    let mut source_bytes = vec![0; 2042 * 2];
    for sum in 0..=1020_u16 {
        let mut remaining = sum;
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let value = remaining.min(255);
            source_bytes[dy * 2042 + usize::from(sum) * 2 + dx] = value as u8;
            remaining -= value;
        }
    }
    let source = upload(
        &device,
        &queue,
        &source_bytes,
        [2042, 2],
        wgpu::TextureFormat::R8Unorm,
    );
    let rounded = builder
        .encode_luma(&device, &mut encoder, &source, 1)
        .unwrap();
    let rounded_copy = copy_texture(&device, &mut encoder, &rounded.levels[0], [1021, 1], 1);
    // Multiple encodes on one builder before a submission must not replace
    // uniforms or outputs belonging to the preceding encode.
    queue.submit([encoder.finish()]);
    assert_eq!(
        read_copy(&device, &preserved_copy, [256, 1], 1),
        (0..=255).collect::<Vec<u8>>()
    );
    assert_eq!(
        read_copy(&device, &rounded_copy, [1021, 1], 1),
        (0..=1020_u16)
            .map(|sum| ((sum + 2) >> 2) as u8)
            .collect::<Vec<_>>()
    );
}

#[test]
fn reductions_preserve_separate_rounding_borders_and_floor_halve_odd_levels() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    for (width, height, count) in [(2, 2, 2), (8, 8, 4), (64, 32, 6), (5, 3, 1), (90, 45, 3)] {
        let bytes: Vec<u8> = (0..height)
            .flat_map(|y| (0..width).map(move |x| ((37 * x + 61 * y + 13 * x * y) % 256) as u8))
            .collect();
        let expected = pyramid::build(&bytes, width, height, count).unwrap();
        if width == 8 {
            assert_eq!(
                expected[1].pixels,
                [
                    53, 124, 115, 122, 124, 143, 134, 141, 131, 166, 129, 128, 138, 125, 112, 99
                ]
            );
        }
        if width == 90 {
            assert_eq!((expected[1].width, expected[1].height), (45, 22));
            assert_eq!((expected[2].width, expected[2].height), (22, 11));
        }
        check_base(&device, &queue, &builder, &expected, "synthetic");
    }
    let expected = pyramid::build(&[0, 1, 2, 3], 2, 2, 2).unwrap();
    assert_eq!(expected[1].pixels, [2]);
    check_base(&device, &queue, &builder, &expected, "rounded pair edges");
}

#[test]
fn full_luma_to_all_levels_matches_the_readable_reference() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    let [width, height] = [256usize, 128];
    let bytes: Vec<u8> = (0..width * height)
        .map(|index| {
            let mut value = (index as u32)
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            value ^= value >> 13;
            (value >> 8) as u8
        })
        .collect();
    let mut base = Vec::new();
    for y in (0..height).step_by(2) {
        for x in (0..width).step_by(2) {
            let sum: u16 = [
                y * width + x,
                y * width + x + 1,
                (y + 1) * width + x,
                (y + 1) * width + x + 1,
            ]
            .map(|at| u16::from(bytes[at]))
            .into_iter()
            .sum();
            base.push(((sum + 2) >> 2) as u8);
        }
    }
    let expected = pyramid::build(&base, width / 2, height / 2, 7).unwrap();
    let source = upload(
        &device,
        &queue,
        &bytes,
        [width as u32, height as u32],
        wgpu::TextureFormat::R8Unorm,
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    let output = builder
        .encode_luma(&device, &mut encoder, &source, 7)
        .unwrap();
    check_output(
        &device,
        &queue,
        encoder,
        &output.levels,
        &expected,
        "full Y bridge",
    );
}

#[test]
fn packed_gray_preserves_lane_order_rows_maxima_and_zero_tail() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    let cases = [
        [1, 3],
        [2, 2],
        [3, 3],
        [4, 2],
        [5, 3],
        [7, 2],
        [8, 3],
        [13, 4],
    ];
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut pending = Vec::new();
    for [width, height] in cases {
        let mut bytes: Vec<u8> = (0..width * height)
            .map(|index| ((index * 73 + index / width * 29 + 11) % 256) as u8)
            .collect();
        bytes[0] = 0;
        bytes[(width * height - 1) as usize] = u8::MAX;
        let source = upload(
            &device,
            &queue,
            &bytes,
            [width, height],
            wgpu::TextureFormat::R8Uint,
        );
        let packed = builder
            .encode_packed_base(&device, &mut encoder, &source)
            .unwrap();
        assert_eq!(packed.logical_size(), [width, height]);
        assert_eq!(packed.device(), &device);
        assert_eq!(packed.texture().format(), wgpu::TextureFormat::Rgba8Uint);
        assert_eq!(
            [packed.texture().width(), packed.texture().height()],
            [width.div_ceil(4), height]
        );
        let packed_copy = copy_texture(
            &device,
            &mut encoder,
            packed.texture(),
            [width.div_ceil(4), height],
            4,
        );
        let source_copy = copy_texture(&device, &mut encoder, &source, [width, height], 1);
        pending.push((width, height, bytes, packed_copy, source_copy));
    }
    queue.submit([encoder.finish()]);

    for (width, height, source, packed, source_copy) in pending {
        assert_eq!(
            read_copy(&device, &source_copy, [width, height], 1),
            source,
            "packing changed its source"
        );
        let actual = read_copy(&device, &packed, [width.div_ceil(4), height], 4);
        let mut expected = Vec::with_capacity(actual.len());
        for row in source.chunks_exact(width as usize) {
            for group in row.chunks(4) {
                expected.extend_from_slice(group);
                expected.resize(expected.len() + (4 - group.len()), 0);
            }
        }
        assert_eq!(actual, expected, "packed bytes differ for {width}x{height}");
    }
}

#[test]
fn bad_input_is_refused_without_poisoning_the_encoder() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    let mut encoder = device.create_command_encoder(&Default::default());
    let too_small = upload(
        &device,
        &queue,
        &[0; 3],
        [1, 3],
        wgpu::TextureFormat::R8Uint,
    );
    assert!(
        builder
            .encode_base(&device, &mut encoder, &too_small, 2)
            .is_err()
    );
    assert!(
        builder
            .encode_base(&device, &mut encoder, &too_small, 0)
            .is_err()
    );
    assert!(
        builder
            .encode_base(&device, &mut encoder, &too_small, usize::MAX)
            .is_err()
    );
    assert!(
        builder
            .encode_luma(&device, &mut encoder, &too_small, 1)
            .is_err()
    );
    let odd = upload(
        &device,
        &queue,
        &[0; 15],
        [5, 3],
        wgpu::TextureFormat::R8Uint,
    );
    let unorm = upload(
        &device,
        &queue,
        &[0; 15],
        [5, 3],
        wgpu::TextureFormat::R8Unorm,
    );
    assert!(
        builder
            .encode_luma(&device, &mut encoder, &unorm, 1)
            .is_err()
    );
    assert!(
        builder
            .encode_base(&device, &mut encoder, &unorm, 1)
            .is_err()
    );
    assert!(
        builder
            .encode_packed_base(&device, &mut encoder, &unorm)
            .is_err()
    );
    let unsampled = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("unsampled pyramid input"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Uint,
        usage: wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    assert!(
        builder
            .encode_base(&device, &mut encoder, &unsampled, 1)
            .is_err()
    );
    assert!(
        builder
            .encode_packed_base(&device, &mut encoder, &unsampled)
            .is_err()
    );
    let valid = builder.encode_base(&device, &mut encoder, &odd, 1).unwrap();
    check_output(
        &device,
        &queue,
        encoder,
        &valid.levels,
        &pyramid::build(&[0; 15], 5, 3, 1).unwrap(),
        "after refusal",
    );
}

#[test]
#[ignore = "requires authenticated local Studio motion captures"]
fn matches_all_seven_native_pyramids_exactly() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    let mut checked = 0usize;
    for (name, expected) in pyramid::tests::native_captures() {
        checked += expected
            .iter()
            .map(|level| level.pixels.len())
            .sum::<usize>();
        check_base(&device, &queue, &builder, &expected, name);
    }
    assert_eq!(checked, 68_808_600);
    eprintln!("GPU temporal pyramid: {checked} native logical pixels exact");
}

fn check_base(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    builder: &Builder,
    expected: &[Level],
    label: &str,
) {
    let base = &expected[0];
    let source = upload(
        device,
        queue,
        &base.pixels,
        [base.width as u32, base.height as u32],
        wgpu::TextureFormat::R8Uint,
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    let output = builder
        .encode_base(device, &mut encoder, &source, expected.len())
        .unwrap();
    check_output(device, queue, encoder, &output.levels, expected, label);
}

fn check_output(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mut encoder: wgpu::CommandEncoder,
    output: &[wgpu::Texture],
    expected: &[Level],
    label: &str,
) {
    assert_eq!(output.len(), expected.len());
    let copies: Vec<_> = output
        .iter()
        .zip(expected)
        .map(|(actual, expected)| {
            assert_eq!(actual.format(), wgpu::TextureFormat::R8Uint);
            assert_eq!(
                [actual.width(), actual.height()],
                [expected.width as u32, expected.height as u32]
            );
            copy_texture(
                device,
                &mut encoder,
                actual,
                [actual.width(), actual.height()],
                1,
            )
        })
        .collect();
    queue.submit([encoder.finish()]);
    for (index, (copy, expected)) in copies.iter().zip(expected).enumerate() {
        let actual = read_copy(
            device,
            copy,
            [expected.width as u32, expected.height as u32],
            1,
        );
        assert_eq!(actual.len(), expected.pixels.len());
        let first = actual
            .iter()
            .zip(&expected.pixels)
            .position(|(a, b)| a != b);
        assert_eq!(first, None, "{label} level {index} first differing pixel");
    }
}

fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bytes: &[u8],
    [width, height]: [u32; 2],
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    assert_eq!(bytes.len(), (width * height) as usize);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("GPU temporal pyramid fixture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width),
            rows_per_image: Some(height),
        },
        texture.size(),
    );
    texture
}
