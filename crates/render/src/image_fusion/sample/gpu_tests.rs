//! GPU qualification for both detached source-band stages.

use super::*;
use crate::{Camera, Held, Sampling, Size};
use kjerag_media::{ColorMatrix, Samples};
use std::sync::mpsc;
use wgpu::util::DeviceExt;

const FRAME: Size = Size {
    width: 32,
    height: 16,
};

#[test]
fn gpu_composes_four_packed_rows_then_samples_both_source_bands() {
    let (device, queue) = match gpu() {
        Ok(gpu) => gpu,
        Err(error) => {
            assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none(), "{error}");
            eprintln!("skipping fusion source-band GPU sampler: {error}");
            return;
        }
    };
    let picture_layout = crate::scene::bind_group_layout(&device);
    let pipeline = FusionInputPipeline::new(&device, &picture_layout);
    let reframe = Reframe::new(
        &[],
        FRAME,
        Camera::default(),
        Held::default(),
        2.0,
        false,
        Sampling::Bilinear,
    )
    .with_samples(Samples {
        wide: false,
        limited: false,
        matrix: ColorMatrix::Bt601,
    });
    let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("fusion band qualification Reframe"),
        contents: reframe.bytes(),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let left_luma: Vec<u8> = (0..FRAME.height)
        .flat_map(|y| (0..FRAME.width).map(move |x| (x * 4 + y * 7) as u8))
        .collect();
    let right_luma: Vec<u8> = (0..FRAME.height)
        .flat_map(|y| (0..FRAME.width).map(move |x| 255 - (x * 4 + y * 7) as u8))
        .collect();
    let luma = [
        texture(
            &device,
            &queue,
            "left luma",
            FRAME,
            wgpu::TextureFormat::R8Unorm,
            &left_luma,
        ),
        texture(
            &device,
            &queue,
            "right luma",
            FRAME,
            wgpu::TextureFormat::R8Unorm,
            &right_luma,
        ),
    ];
    let chroma = [
        texture(
            &device,
            &queue,
            "left chroma",
            FRAME.halved(),
            wgpu::TextureFormat::Rg8Unorm,
            &vec![128; FRAME.halved().width as usize * FRAME.halved().height as usize * 2],
        ),
        texture(
            &device,
            &queue,
            "right chroma",
            FRAME.halved(),
            wgpu::TextureFormat::Rg8Unorm,
            &vec![128; FRAME.halved().width as usize * FRAME.halved().height as usize * 2],
        ),
    ];
    let [luma0, luma1] = luma;
    let [chroma0, chroma1] = chroma;
    let planes = [
        crate::Planes {
            luma: luma0,
            chroma: chroma0,
        },
        crate::Planes {
            luma: luma1,
            chroma: chroma1,
        },
    ];
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        min_filter: wgpu::FilterMode::Linear,
        mag_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let picture = direct_type2::bind_picture(
        &device,
        &picture_layout,
        &uniforms,
        [&planes[0], &planes[1]],
        &sampler,
    );

    let packed: Vec<[f32; 4]> = (0..crate::studio_type2::MAP_NODES)
        .map(|node| {
            let x = (node % MAP_WIDTH) as f32 / (MAP_WIDTH - 1) as f32;
            let y = (node / MAP_WIDTH) as f32 / 99.0;
            [0.1 + 0.3 * x, 0.2 + 0.6 * y, 0.6 + 0.3 * x, 0.8 - 0.6 * y]
        })
        .collect();
    let lookup = super::super::coordinates::selected_x4_band();
    let expected_composed: Vec<[f32; 4]> = lookup
        .iter()
        .copied()
        .map(|coordinate| compose_cpu(&packed, coordinate))
        .collect();
    let inverse_chart = super::super::coordinates::selected_x4();
    let wrong_rotation: Vec<[f32; 4]> = inverse_chart[48 * MAP_WIDTH..52 * MAP_WIDTH]
        .iter()
        .copied()
        .map(|coordinate| compose_cpu(&packed, coordinate))
        .collect();
    assert!(
        expected_composed
            .iter()
            .zip(&wrong_rotation)
            .any(|(good, wrong)| (good[0] - wrong[0]).abs() > 0.01),
        "the packed-map gradient did not discriminate the opposite rotation"
    );
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("fusion band qualification packed map"),
        contents: bytes_of(&packed),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let composed = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fusion band qualification composed"),
        size: COMPOSED_BYTES,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let map = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("fusion band qualification map"),
        layout: &pipeline.map_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: pipeline.lookup.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: composed.as_entire_binding(),
            },
        ],
    });
    let bands = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fusion band qualification output"),
        size: BAND_BYTES,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let output = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("fusion band qualification output"),
        layout: &pipeline.output_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: bands.as_entire_binding(),
        }],
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fusion band qualification readback"),
        size: READBACK_BYTES,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline.compose_pipeline);
        pass.set_bind_group(0, &picture, &[]);
        pass.set_bind_group(1, &map, &[]);
        pass.set_bind_group(2, &output, &[]);
        pass.dispatch_workgroups((COMPOSED_NODES as u32).div_ceil(64), 1, 1);
    }
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline.sample_pipeline);
        pass.set_bind_group(0, &picture, &[]);
        pass.set_bind_group(1, &map, &[]);
        pass.set_bind_group(2, &output, &[]);
        pass.dispatch_workgroups(
            (BAND_WIDTH as u32).div_ceil(8),
            (BAND_HEIGHT as u32).div_ceil(8),
            1,
        );
    }
    encoder.copy_buffer_to_buffer(&composed, 0, &readback, 0, COMPOSED_BYTES);
    encoder.copy_buffer_to_buffer(&bands, 0, &readback, COMPOSED_BYTES, BAND_BYTES);
    queue.submit([encoder.finish()]);
    let mapped = map_read(&device, &readback);

    for (node, expected) in expected_composed.iter().enumerate() {
        let at = node * 16;
        let got: [f32; 4] = std::array::from_fn(|c| {
            f32::from_le_bytes(mapped[at + c * 4..at + c * 4 + 4].try_into().unwrap())
        });
        for c in 0..4 {
            near(got[c], expected[c], 1.0e-6);
        }
    }
    // Pick the strongest source-color distinction from the entire synthetic
    // fixture. An arbitrary interior coordinate can be locally flat despite
    // endpoint interpolation and nearest fourfold repetition being different.
    let (decoy_pixel, decoy_delta) = (0..BAND_HEIGHT)
        .flat_map(|y| (0..BAND_WIDTH).map(move |x| [x, y]))
        .map(|[x, y]| {
            let good = endpoint_cpu(&expected_composed, x, y);
            let wrong = expected_composed[(y / 4) * MAP_WIDTH + x / 4];
            let a = sample_luma(&left_luma, [good[0] * 2.0, good[1]]);
            let b = sample_luma(&left_luma, [wrong[0] * 2.0, wrong[1]]);
            ([x, y], (a - b).abs())
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    assert!(
        decoy_delta > 3.0 / 255.0,
        "source fixture cannot distinguish repeated UVs"
    );
    for [x, y] in [[0usize, 0usize], [317, 7], [799, 15], decoy_pixel] {
        let packed = endpoint_cpu(&expected_composed, x, y);
        let expected = [
            sample_luma(&left_luma, [packed[0] * 2.0, packed[1]]),
            sample_luma(&right_luma, [packed[2] * 2.0 - 1.0, packed[3]]),
        ];
        let first_word = (y * BAND_WIDTH + x) * 8;
        for (lens, &expected) in expected.iter().enumerate() {
            let actual = |channel| {
                let at = COMPOSED_BYTES as usize + (first_word + lens * 4 + channel) * 4;
                f32::from_le_bytes(mapped[at..at + 4].try_into().unwrap())
            };
            for channel in 0..3 {
                // Normalized texture filtering has implementation-specific
                // fractional precision. Qualify at the BGR producer's code
                // scale, not as bit identity with scalar f32 bilinear math.
                near(actual(channel), expected, 1.0 / 255.0);
            }
        }
    }
}

fn compose_cpu(map: &[[f32; 4]], coordinate: [f32; 2]) -> [f32; 4] {
    let q = coordinate.map(|v| (v * 32.0).round_ties_even() / 32.0);
    let base = q.map(|v| v.floor() as i32);
    let f = [q[0] - base[0] as f32, q[1] - base[1] as f32];
    let at = |x: i32, y: i32| {
        let source_x = (x + 199).rem_euclid(200) as usize;
        map[y.clamp(0, 99) as usize * MAP_WIDTH + source_x]
    };
    bilinear(
        at(base[0], base[1]),
        at(base[0] + 1, base[1]),
        at(base[0], base[1] + 1),
        at(base[0] + 1, base[1] + 1),
        f,
    )
}

fn endpoint_cpu(map: &[[f32; 4]], x: usize, y: usize) -> [f32; 4] {
    let p = [x as f32 * 199.0 / 799.0, y as f32 * 3.0 / 15.0];
    let base = p.map(|v| v.floor() as usize);
    let f = [p[0] - base[0] as f32, p[1] - base[1] as f32];
    let at = |x: usize, y: usize| map[y.min(3) * MAP_WIDTH + x.min(199)];
    bilinear(
        at(base[0], base[1]),
        at(base[0] + 1, base[1]),
        at(base[0], base[1] + 1),
        at(base[0] + 1, base[1] + 1),
        f,
    )
}

fn bilinear(a: [f32; 4], b: [f32; 4], c: [f32; 4], d: [f32; 4], f: [f32; 2]) -> [f32; 4] {
    let mix = |a, b, t| a * (1.0 - t) + b * t;
    std::array::from_fn(|i| mix(mix(a[i], b[i], f[0]), mix(c[i], d[i], f[0]), f[1]))
}

fn sample_luma(bytes: &[u8], uv: [f32; 2]) -> f32 {
    let p = [
        uv[0] * FRAME.width as f32 - 0.5,
        uv[1] * FRAME.height as f32 - 0.5,
    ];
    let base = p.map(|v| v.floor() as i32);
    let f = [p[0] - base[0] as f32, p[1] - base[1] as f32];
    let at = |x: i32, y: i32| {
        let x = x.clamp(0, FRAME.width as i32 - 1) as usize;
        let y = y.clamp(0, FRAME.height as i32 - 1) as usize;
        bytes[y * FRAME.width as usize + x] as f32 / 255.0
    };
    let mix = |a, b, t| a * (1.0 - t) + b * t;
    mix(
        mix(at(base[0], base[1]), at(base[0] + 1, base[1]), f[0]),
        mix(at(base[0], base[1] + 1), at(base[0] + 1, base[1] + 1), f[0]),
        f[1],
    )
}

fn map_read(device: &wgpu::Device, buffer: &wgpu::Buffer) -> wgpu::BufferView {
    let (sent, received) = mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sent.send(result);
        });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    received.recv().unwrap().unwrap();
    buffer.slice(..).get_mapped_range()
}

fn texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    size: Size,
    format: wgpu::TextureFormat,
    bytes: &[u8],
) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some((bytes.len() / size.height as usize) as u32),
            rows_per_image: Some(size.height),
        },
        texture.size(),
    );
    texture
}

fn near(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        actual.is_finite() && (actual - expected).abs() <= tolerance,
        "GPU value {actual} differs from {expected} by more than {tolerance}"
    );
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(answer) => return answer,
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn gpu() -> Result<(wgpu::Device, wgpu::Queue), String> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
        .into_iter()
        .next()
        .ok_or("no Vulkan adapter")?;
    block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("fusion source-band qualification"),
        required_features: wgpu::Features::empty(),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .map_err(|error| error.to_string())
}
