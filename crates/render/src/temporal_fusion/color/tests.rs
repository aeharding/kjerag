use super::*;
use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

const X4: MatrixCoefficients =
    MatrixCoefficients::from_source_rgb([1.5748, 0.1873, 0.4681, 1.8556]);

#[test]
fn x4_inverse_round_trips_gamma_rgb() {
    for rgb in [
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        [0.18, 0.42, 0.73],
        [0.91, 0.27, 0.08],
    ] {
        let decoded = ycbcr_to_rgb(rgb_to_ycbcr(rgb, X4), X4);
        for channel in 0..3 {
            assert!(
                (decoded[channel] - rgb[channel]).abs() < 2.0e-6,
                "channel {channel}: {decoded:?} from {rgb:?}"
            );
        }
    }
}

#[test]
fn neutral_chroma_preserves_full_range_luma() {
    for y in [0.0, 0.25, 0.5, 1.0] {
        assert_eq!(ycbcr_to_rgb([y, 0.0, 0.0], X4), [y, y, y]);
        let encoded = rgb_to_ycbcr([y, y, y], X4);
        assert!((encoded[0] - y).abs() < 1.0e-7);
        assert!(encoded[1].abs() < 1.0e-7);
        assert!(encoded[2].abs() < 1.0e-7);
    }
    assert_eq!(128.0_f32 / 255.0, 0.5019608_f32);
}

#[test]
fn chroma_of_a_footprint_is_the_conversion_of_its_rgb_average() {
    let footprint = [
        [0.1, 0.2, 0.3],
        [0.8, 0.1, 0.4],
        [0.3, 0.9, 0.2],
        [0.6, 0.4, 1.0],
    ];
    let average = [0, 1, 2].map(|channel| {
        footprint.iter().map(|rgb| rgb[channel]).sum::<f32>() / footprint.len() as f32
    });
    let from_average = rgb_to_ycbcr(average, X4);
    let average_converted = [0, 1, 2].map(|channel| {
        footprint
            .iter()
            .map(|&rgb| rgb_to_ycbcr(rgb, X4)[channel])
            .sum::<f32>()
            / footprint.len() as f32
    });
    for channel in 0..3 {
        assert!((from_average[channel] - average_converted[channel]).abs() < 1.0e-7);
    }
}

#[test]
fn rgb_cube_converts_to_finite_clamped_unorm_values() {
    for bits in 0..8 {
        let rgb = [
            (bits & 1) as f32,
            ((bits >> 1) & 1) as f32,
            ((bits >> 2) & 1) as f32,
        ];
        let [y, cb, cr] = rgb_to_ycbcr(rgb, X4);
        for encoded in [y, cb + 128.0 / 255.0, cr + 128.0 / 255.0] {
            let unorm = encoded.clamp(0.0, 1.0);
            assert!(unorm.is_finite() && (0.0..=1.0).contains(&unorm));
        }
    }
}

#[test]
fn rejects_noninvertible_or_nonfinite_coefficients() {
    for matrix in [
        MatrixCoefficients::from_source_rgb([0.0, 0.1873, 0.4681, 1.8556]),
        MatrixCoefficients::from_source_rgb([1.5748, f32::NAN, 0.4681, 1.8556]),
        MatrixCoefficients::from_source_rgb([1.0, -1.0, 0.0, 1.0]),
    ] {
        assert_eq!(matrix.validate(), Err(Error::Coefficients));
    }
    assert_eq!(X4.validate(), Ok(()));
}

#[test]
fn diagnostic_conversion_wgsl_validates() {
    for (label, source) in [
        ("RGB to NV12", RGB_TO_NV12_WGSL),
        ("NV12 to RGB", NV12_TO_RGB_WGSL),
    ] {
        let module = wgpu::naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|error| panic!("{label} WGSL parse failed: {error}"));
        Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|error| panic!("{label} WGSL validation failed: {error}"));
    }
}

#[test]
fn gpu_matches_varying_footprints_and_centred_chroma_reconstruction() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    for (label, matrix) in [
        (
            "BT.601",
            MatrixCoefficients::from_source_rgb([1.402, 0.344, 0.714, 1.772]),
        ),
        ("BT.709", X4),
    ] {
        gpu_round_trip(&device, &queue, label, matrix);
    }
}

fn gpu_round_trip(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    matrix: MatrixCoefficients,
) {
    let size = wgpu::Extent3d {
        width: 4,
        height: 4,
        depth_or_array_layers: 1,
    };
    let source = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("diagnostic color round-trip source"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let source_pixels = [
        [16, 32, 48, 255],
        [64, 80, 96, 255],
        [100, 120, 140, 255],
        [160, 150, 130, 255],
        [20, 200, 40, 255],
        [220, 30, 100, 255],
        [80, 90, 240, 255],
        [250, 200, 10, 255],
        [5, 180, 230, 255],
        [190, 60, 70, 255],
        [110, 210, 30, 255],
        [40, 20, 220, 255],
        [240, 120, 180, 255],
        [70, 240, 160, 255],
        [200, 40, 250, 255],
        [130, 170, 60, 255],
    ];
    let source_bytes: Vec<u8> = source_pixels.into_iter().flatten().collect();
    queue.write_texture(
        source.as_image_copy(),
        &source_bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(16),
            rows_per_image: Some(4),
        },
        size,
    );

    let conversion = GpuColorConversion::new(device);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("diagnostic color round-trip"),
    });
    let nv12 = conversion
        .encode_rgb_to_nv12(&mut encoder, &source, matrix)
        .unwrap();
    let wrong = if matrix == X4 {
        MatrixCoefficients::from_source_rgb([1.402, 0.344, 0.714, 1.772])
    } else {
        X4
    };
    assert_eq!(
        conversion
            .encode_nv12_to_rgb(&mut encoder, &nv12, wrong)
            .err(),
        Some(Error::MatrixMismatch)
    );
    let output = conversion
        .encode_nv12_to_rgb(&mut encoder, &nv12, matrix)
        .unwrap();
    let external_output = conversion
        .encode_planes_to_rgb(&mut encoder, &nv12.y, &nv12.uv, matrix)
        .unwrap();
    assert!(
        conversion
            .encode_planes_to_rgb_scissored(&mut encoder, &nv12.y, &nv12.uv, matrix, &[])
            .is_err()
    );
    assert!(
        conversion
            .encode_planes_to_rgb_scissored(
                &mut encoder,
                &nv12.y,
                &nv12.uv,
                matrix,
                &[[1, 0, 2, 2]],
            )
            .is_err()
    );
    let scissored_output = conversion
        .encode_planes_to_rgb_scissored(
            &mut encoder,
            &nv12.y,
            &nv12.uv,
            matrix,
            &[[0, 0, 2, 2], [2, 2, 2, 2]],
        )
        .unwrap();
    assert_eq!(
        conversion
            .encode_planes_to_rgb(
                &mut encoder,
                &nv12.y,
                &nv12.uv,
                MatrixCoefficients::from_source_rgb([f32::NAN, 0.0, 0.0, 1.0]),
            )
            .err(),
        Some(Error::Coefficients)
    );
    let wrong_shape_uv = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("diagnostic color wrong-shape UV"),
        size: wgpu::Extent3d {
            width: 1,
            height: 2,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    assert_eq!(
        conversion
            .encode_planes_to_rgb(&mut encoder, &nv12.y, &wrong_shape_uv, matrix)
            .err(),
        Some(Error::Nv12Textures)
    );
    let y_copy = copy_texture(device, &mut encoder, &nv12.y, 4, 4, 1);
    let uv_copy = copy_texture(device, &mut encoder, &nv12.uv, 2, 2, 2);
    let rgb_copy = copy_texture(device, &mut encoder, &output, 4, 4, 4);
    let external_rgb_copy = copy_texture(device, &mut encoder, &external_output, 4, 4, 4);
    let scissored_rgb_copy = copy_texture(device, &mut encoder, &scissored_output, 4, 4, 4);
    queue.submit([encoder.finish()]);
    let actual_y = read_copy(device, y_copy, 4, 4, 1);
    let actual_uv = read_copy(device, uv_copy, 2, 2, 2);
    let actual_rgb = read_copy(device, rgb_copy, 4, 4, 4);
    let actual_external_rgb = read_copy(device, external_rgb_copy, 4, 4, 4);
    let actual_scissored_rgb = read_copy(device, scissored_rgb_copy, 4, 4, 4);
    for y in 0..4 {
        for x in 0..4 {
            let at = (y * 4 + x) * 4;
            let selected = (x < 2 && y < 2) || (x >= 2 && y >= 2);
            let expected = if selected {
                &actual_external_rgb[at..at + 4]
            } else {
                &[0, 0, 0, 255]
            };
            assert_eq!(
                &actual_scissored_rgb[at..at + 4],
                expected,
                "{label} scissor at {x},{y}"
            );
        }
    }

    let (expected_y, expected_uv) = reference_nv12(&source_bytes, matrix);
    assert_codes(label, "Y", &actual_y, &expected_y);
    assert_codes(label, "UV", &actual_uv, &expected_uv);
    let expected_rgb = reference_rgb(&actual_y, &actual_uv, matrix);
    assert_codes(label, "RGB", &actual_rgb, &expected_rgb);
    assert_codes(label, "external RGB", &actual_external_rgb, &expected_rgb);
    for alpha in actual_rgb.iter().skip(3).step_by(4) {
        assert_eq!(*alpha, 255, "{label} reverse alpha");
    }
}

fn reference_nv12(source: &[u8], matrix: MatrixCoefficients) -> (Vec<u8>, Vec<u8>) {
    let rgb = |x: usize, y: usize| {
        let at = 4 * (y * 4 + x);
        [0, 1, 2].map(|channel| source[at + channel] as f32 / 255.0)
    };
    let mut y = Vec::with_capacity(16);
    for row in 0..4 {
        for column in 0..4 {
            y.push(quantize(reference_inverse(rgb(column, row), matrix).0));
        }
    }
    let mut uv = Vec::with_capacity(8);
    for row in 0..2 {
        for column in 0..2 {
            let mut average = [0.0; 3];
            for y_offset in 0..2 {
                for x_offset in 0..2 {
                    let sample = rgb(2 * column + x_offset, 2 * row + y_offset);
                    for channel in 0..3 {
                        average[channel] += sample[channel] * 0.25;
                    }
                }
            }
            let (_, cb, cr) = reference_inverse(average, matrix);
            uv.extend([quantize(cb + 128.0 / 255.0), quantize(cr + 128.0 / 255.0)]);
        }
    }
    (y, uv)
}

fn reference_rgb(y: &[u8], uv: &[u8], matrix: MatrixCoefficients) -> Vec<u8> {
    let chroma = |x: i32, y: i32| {
        let x = x.clamp(0, 1) as usize;
        let y = y.clamp(0, 1) as usize;
        let at = 2 * (y * 2 + x);
        [
            uv[at] as f32 / 255.0 - 128.0 / 255.0,
            uv[at + 1] as f32 / 255.0 - 128.0 / 255.0,
        ]
    };
    let mut rgb = Vec::with_capacity(64);
    for row in 0..4 {
        for column in 0..4 {
            let qx = (column as f32 + 0.5) * 0.5 - 0.5;
            let qy = (row as f32 + 0.5) * 0.5 - 0.5;
            let low_x = qx.floor() as i32;
            let low_y = qy.floor() as i32;
            // WGSL `fract` is x-floor(x), including for the negative edge site.
            let fx = qx - qx.floor();
            let fy = qy - qy.floor();
            let top = mix2(chroma(low_x, low_y), chroma(low_x + 1, low_y), fx);
            let bottom = mix2(chroma(low_x, low_y + 1), chroma(low_x + 1, low_y + 1), fx);
            let [cb, cr] = mix2(top, bottom, fy);
            let luminance = y[row * 4 + column] as f32 / 255.0;
            let decoded = [
                luminance + matrix.r_cr * cr,
                luminance - matrix.g_cb * cb - matrix.g_cr * cr,
                luminance + matrix.b_cb * cb,
            ];
            rgb.extend(decoded.map(quantize));
            rgb.push(255);
        }
    }
    rgb
}

fn reference_inverse(rgb: [f32; 3], matrix: MatrixCoefficients) -> (f32, f32, f32) {
    let denominator = 1.0 + matrix.g_cb / matrix.b_cb + matrix.g_cr / matrix.r_cr;
    let y = (rgb[1] + matrix.g_cb * rgb[2] / matrix.b_cb + matrix.g_cr * rgb[0] / matrix.r_cr)
        / denominator;
    (y, (rgb[2] - y) / matrix.b_cb, (rgb[0] - y) / matrix.r_cr)
}

fn mix2(a: [f32; 2], b: [f32; 2], amount: f32) -> [f32; 2] {
    [a[0] + amount * (b[0] - a[0]), a[1] + amount * (b[1] - a[1])]
}

fn quantize(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn assert_codes(label: &str, plane: &str, actual: &[u8], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len());
    for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
        assert!(
            actual.abs_diff(expected) <= 1,
            "{label} {plane} code {index}: {actual}, expected {expected}; one code value is the explicit tolerance for UNorm rounding"
        );
    }
}

struct TextureCopy {
    buffer: wgpu::Buffer,
    padded_row: u32,
}

fn copy_texture(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    bytes_per_pixel: u32,
) -> TextureCopy {
    let packed_row = width * bytes_per_pixel;
    let padded_row = packed_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("diagnostic color plane readback"),
        size: u64::from(padded_row * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    TextureCopy { buffer, padded_row }
}

fn read_copy(
    device: &wgpu::Device,
    copy: TextureCopy,
    width: u32,
    height: u32,
    bytes_per_pixel: u32,
) -> Vec<u8> {
    copy.buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mapped = copy.buffer.slice(..).get_mapped_range();
    let packed_row = (width * bytes_per_pixel) as usize;
    let mut result = Vec::with_capacity(packed_row * height as usize);
    for row in mapped
        .chunks(copy.padded_row as usize)
        .take(height as usize)
    {
        result.extend_from_slice(&row[..packed_row]);
    }
    drop(mapped);
    copy.buffer.unmap();
    result
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let require_gpu = std::env::var_os("KJERAG_REQUIRE_GPU").is_some();
    let mut cpu = None;
    let mut selected = None;
    for adapter in block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN)) {
        if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
            if cpu.is_none() {
                cpu = Some(adapter);
            }
        } else {
            selected = Some(adapter);
            break;
        }
    }
    let adapter = selected.or(if require_gpu { None } else { cpu });
    let Some(adapter) = adapter else {
        assert!(
            !require_gpu,
            "diagnostic color GPU test has no non-CPU Vulkan adapter"
        );
        eprintln!("skipping diagnostic color GPU test: no Vulkan adapter");
        return None;
    };
    eprintln!("diagnostic color GPU adapter: {:?}", adapter.get_info());
    match block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("diagnostic color test"),
        required_features: wgpu::Features::empty(),
        required_limits: adapter.limits(),
        ..Default::default()
    })) {
        Ok(pair) => Some(pair),
        Err(error) => {
            assert!(!require_gpu, "diagnostic color GPU device: {error}");
            eprintln!("skipping diagnostic color GPU test: {error}");
            None
        }
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => return value,
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}
