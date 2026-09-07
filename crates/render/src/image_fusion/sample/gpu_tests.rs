//! GPU qualification for the detached working-chart source sampler.

use super::*;
use crate::{Camera, Held, Sampling, Size};
use kjerag_media::{ColorMatrix, Samples};
use std::sync::mpsc;
use wgpu::util::DeviceExt;

const FRAME: Size = Size {
    width: 32,
    height: 16,
};
const LEFT_Y: u8 = 64;
const RIGHT_Y: u8 = 192;

#[test]
fn gpu_sampler_recovers_original_chart_half_turn_and_both_nv12_lenses() {
    let (device, queue) = match gpu() {
        Ok(gpu) => gpu,
        Err(error) => {
            assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none(), "{error}");
            eprintln!("skipping fusion input GPU sampler: {error}");
            return;
        }
    };

    let good = sample_north_pole(&device, &queue, SAMPLE_WGSL);
    assert_eq!(good.covered, 1);
    near(good.left[0], LEFT_Y as f32 / 255.0, 2.0e-5);
    near(good.right[0], RIGHT_Y as f32 / 255.0, 2.0e-5);
    for channel in 1..3 {
        near(good.left[channel], good.left[0], 2.0e-5);
        near(good.right[channel], good.right[0], 2.0e-5);
    }

    // Working north becomes original-chart +X. The mesh locates longitude PI,
    // whose +0.5 varying wraps to original longitude zero. The map stores its
    // local source U in both packed atlas halves.
    near(
        good.packed[0],
        0.5 * (0.25 + 0.5 * (0.5 / MAP_WIDTH as f32)),
        3.0e-4,
    );
    near(
        good.packed[2],
        0.5 * (1.25 + 0.5 * (0.5 / MAP_WIDTH as f32)),
        3.0e-4,
    );

    let body = "type2_mesh(vec3<f32>(-q_final.y, q_final.z, q_final.x))";
    assert_eq!(SAMPLE_WGSL.matches(body).count(), 1);
    let wrong = SAMPLE_WGSL.replacen(
        body,
        "type2_mesh(vec3<f32>(q_final.y, q_final.z, -q_final.x))",
        1,
    );
    let shifted = sample_north_pole(&device, &queue, &wrong);
    assert_eq!(shifted.covered, 1);
    near(
        shifted.packed[0],
        0.5 * (0.25 + 0.5 * (100.5 / MAP_WIDTH as f32)),
        3.0e-4,
    );
    near(
        shifted.packed[2],
        0.5 * (1.25 + 0.5 * (100.5 / MAP_WIDTH as f32)),
        3.0e-4,
    );
    assert!(
        (shifted.packed[0] - good.packed[0]).abs() > 0.12,
        "half-turn corruption did not move the sampled chart: good {:?}, corrupt {:?}",
        good.packed,
        shifted.packed
    );
}

#[derive(Debug)]
struct Answer {
    left: [f32; 4],
    right: [f32; 4],
    packed: [f32; 4],
    covered: u32,
}

fn sample_north_pole(device: &wgpu::Device, queue: &wgpu::Queue, sample_wgsl: &str) -> Answer {
    let shader = format!("{}\n{sample_wgsl}", crate::direct_type2::draw_wgsl());
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("fusion input GPU qualification"),
        source: wgpu::ShaderSource::Wgsl(shader.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("fusion input GPU qualification"),
        layout: None,
        module: &module,
        entry_point: Some("sample_fusion_inputs"),
        compilation_options: Default::default(),
        cache: None,
    });

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
        label: Some("fusion input qualification Reframe"),
        contents: reframe.bytes(),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let luma = [
        constant_texture(
            device,
            queue,
            "fusion input left luma",
            FRAME,
            wgpu::TextureFormat::R8Unorm,
            &[LEFT_Y],
        ),
        constant_texture(
            device,
            queue,
            "fusion input right luma",
            FRAME,
            wgpu::TextureFormat::R8Unorm,
            &[RIGHT_Y],
        ),
    ];
    let chroma_size = FRAME.halved();
    let chroma = [
        constant_texture(
            device,
            queue,
            "fusion input left chroma",
            chroma_size,
            wgpu::TextureFormat::Rg8Unorm,
            &[128, 128],
        ),
        constant_texture(
            device,
            queue,
            "fusion input right chroma",
            chroma_size,
            wgpu::TextureFormat::Rg8Unorm,
            &[128, 128],
        ),
    ];
    let views = [
        luma[0].create_view(&Default::default()),
        chroma[0].create_view(&Default::default()),
        luma[1].create_view(&Default::default()),
        chroma[1].create_view(&Default::default()),
    ];
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        min_filter: wgpu::FilterMode::Linear,
        mag_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let mut picture_entries = vec![wgpu::BindGroupEntry {
        binding: 0,
        resource: uniforms.as_entire_binding(),
    }];
    picture_entries.extend(
        views
            .iter()
            .enumerate()
            .map(|(index, view)| wgpu::BindGroupEntry {
                binding: 1 + index as u32,
                resource: wgpu::BindingResource::TextureView(view),
            }),
    );
    picture_entries.push(wgpu::BindGroupEntry {
        binding: 5,
        resource: wgpu::BindingResource::Sampler(&sampler),
    });
    let picture = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("fusion input qualification picture"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &picture_entries,
    });

    let packed: Vec<[f32; 4]> = (0..MAP_NODES)
        .map(|node| {
            // Keep every box-filter footprint away from the aggregate atlas
            // seam while retaining longitude as a discriminating gradient.
            let chart_u = (node % MAP_WIDTH) as f32 / MAP_WIDTH as f32 + 0.5 / MAP_WIDTH as f32;
            let local_u = 0.25 + 0.5 * chart_u;
            [0.5 * local_u, 0.5, 0.5 * (1.0 + local_u), 0.5]
        })
        .collect();
    let alpha = vec![0.5f32; MAP_NODES];
    let packed_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("fusion input longitude map"),
        contents: bytes_of(&packed),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let alpha_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("fusion input alpha map"),
        contents: bytes_of(&alpha),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let map = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("fusion input qualification map"),
        layout: &pipeline.get_bind_group_layout(1),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: packed_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: alpha_buffer.as_entire_binding(),
            },
        ],
    });

    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fusion input qualification output"),
        size: OUTPUT_BYTES,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fusion input qualification readback"),
        size: OUTPUT_BYTES,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let answer = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("fusion input qualification answer"),
        layout: &pipeline.get_bind_group_layout(2),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: output.as_entire_binding(),
        }],
    });

    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &picture, &[]);
        pass.set_bind_group(1, &map, &[]);
        pass.set_bind_group(2, &answer, &[]);
        pass.dispatch_workgroups(
            (MAP_WIDTH as u32).div_ceil(8),
            (MAP_HEIGHT as u32).div_ceil(8),
            1,
        );
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, OUTPUT_BYTES);
    queue.submit([encoder.finish()]);

    let (sent, received) = mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sent.send(result);
        });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    received.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range();
    let word = |at: usize| {
        u32::from_le_bytes(
            mapped[at * size_of::<u32>()..(at + 1) * size_of::<u32>()]
                .try_into()
                .unwrap(),
        )
    };
    let floats =
        |first: usize| std::array::from_fn(|channel| f32::from_bits(word(first + channel)));
    Answer {
        left: floats(0),
        right: floats(4),
        packed: floats(8),
        covered: word(12),
    }
}

fn constant_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    size: Size,
    format: wgpu::TextureFormat,
    pixel: &[u8],
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
    let bytes = pixel.repeat(size.width as usize * size.height as usize);
    queue.write_texture(
        texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size.width * pixel.len() as u32),
            rows_per_image: Some(size.height),
        },
        texture.size(),
    );
    texture
}

fn bytes_of<T>(values: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
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
        label: Some("fusion input sampler qualification"),
        required_features: wgpu::Features::empty(),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .map_err(|error| error.to_string())
}
