//! GPU qualification for the direct NV12 source colour-matrix consumer.

use super::*;
use crate::{Camera, Held, Reframe, Sampling, Size};
use kjerag_media::{ColorMatrix, Samples};

const FRAME: Size = Size {
    width: 32,
    height: 16,
};
const UVS: [[f32; 2]; 4] = [[0.25, 0.25], [0.75, 0.25], [0.25, 0.75], [0.75, 0.75]];
const LUMA: [u8; 4] = [80, 120, 100, 140];
const CHROMA: [[u8; 2]; 4] = [[128, 128], [128, 128], [64, 192], [200, 32]];

#[test]
fn direct_nv12_source_uses_the_selected_matrix_on_both_atlas_halves() {
    let (device, queue) = match super::tests::gpu() {
        Ok(gpu) => gpu,
        Err(error) => {
            assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none(), "{error}");
            eprintln!("skipping direct colour-matrix consumer: {error}");
            return;
        }
    };

    let bt601 = sample_matrix(&device, &queue, ColorMatrix::Bt601);
    let bt709 = sample_matrix(&device, &queue, ColorMatrix::Bt709);
    for (label, actual, coefficients) in [
        ("BT.601", &bt601, [1.402, 0.344, 0.714, 1.772]),
        ("BT.709", &bt709, [1.5748, 0.1873, 0.4681, 1.8556]),
    ] {
        for index in 0..UVS.len() {
            let expected = source_rgb(LUMA[index], CHROMA[index], coefficients);
            for channel in 0..3 {
                let error = (actual[index][channel] - expected[channel]).abs();
                assert!(
                    actual[index][channel].is_finite() && error <= 2.0e-5,
                    "{label} sample {index} channel {channel}: GPU {} expected {} error {error}",
                    actual[index][channel],
                    expected[channel]
                );
            }
        }
    }

    // Neutral 128/255 chroma is matrix-independent on both native textures.
    for index in 0..2 {
        for channel in 0..3 {
            assert!((bt601[index][channel] - LUMA[index] as f32 / 255.0).abs() <= 2.0e-5);
            assert!((bt709[index][channel] - bt601[index][channel]).abs() <= 2.0e-5);
        }
    }
    let colored_difference = (2..4)
        .flat_map(|index| (0..3).map(move |channel| (index, channel)))
        .map(|(index, channel)| (bt709[index][channel] - bt601[index][channel]).abs())
        .fold(0.0f32, f32::max);
    assert!(
        colored_difference > 0.05,
        "fixture does not distinguish BT.709 from legacy BT.601: {colored_difference}"
    );
}

fn sample_matrix(device: &wgpu::Device, queue: &wgpu::Queue, matrix: ColorMatrix) -> Vec<[f32; 4]> {
    let source = format!("{}\n{PROBE}", draw_wgsl());
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("direct colour-matrix consumer"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("direct colour-matrix consumer"),
        layout: None,
        module: &module,
        entry_point: Some("sample_source_rgb"),
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
        matrix,
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("direct colour-matrix Reframe"),
        size: std::mem::size_of::<Reframe>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&uniform, 0, reframe.bytes());

    let luma = [
        luma_texture(device, queue, 80, 100),
        luma_texture(device, queue, 120, 140),
    ];
    let chroma = [
        chroma_texture(device, queue, [64, 192]),
        chroma_texture(device, queue, [200, 32]),
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
        resource: uniform.as_entire_binding(),
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
        label: Some("direct colour-matrix NV12 fixture"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &picture_entries,
    });

    let output_bytes = (UVS.len() * std::mem::size_of::<[f32; 4]>()) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("direct colour-matrix answers"),
        size: output_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("direct colour-matrix readback"),
        size: output_bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let answers = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("direct colour-matrix answers"),
        layout: &pipeline.get_bind_group_layout(1),
        entries: &[wgpu::BindGroupEntry {
            binding: 2,
            resource: output.as_entire_binding(),
        }],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &picture, &[]);
        pass.set_bind_group(1, &answers, &[]);
        pass.dispatch_workgroups(UVS.len() as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output_bytes);
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    bytes
        .chunks_exact(16)
        .map(|bytes| {
            std::array::from_fn(|channel| {
                f32::from_le_bytes(bytes[channel * 4..channel * 4 + 4].try_into().unwrap())
            })
        })
        .collect()
}

fn luma_texture(device: &wgpu::Device, queue: &wgpu::Queue, top: u8, bottom: u8) -> wgpu::Texture {
    let texture = texture(
        device,
        "direct colour luma",
        FRAME.width,
        FRAME.height,
        wgpu::TextureFormat::R8Unorm,
    );
    let bytes: Vec<u8> = (0..FRAME.height)
        .flat_map(|y| {
            std::iter::repeat_n(
                if y < FRAME.height / 2 { top } else { bottom },
                FRAME.width as usize,
            )
        })
        .collect();
    write_texture(queue, &texture, &bytes, FRAME.width);
    texture
}

fn chroma_texture(device: &wgpu::Device, queue: &wgpu::Queue, bottom: [u8; 2]) -> wgpu::Texture {
    let size = FRAME.halved();
    let texture = texture(
        device,
        "direct colour chroma",
        size.width,
        size.height,
        wgpu::TextureFormat::Rg8Unorm,
    );
    let bytes: Vec<u8> = (0..size.height)
        .flat_map(|y| {
            let sample = if y < size.height / 2 {
                [128, 128]
            } else {
                bottom
            };
            std::iter::repeat_n(sample, size.width as usize).flatten()
        })
        .collect();
    write_texture(queue, &texture, &bytes, size.width * 2);
    texture
}

fn texture(
    device: &wgpu::Device,
    label: &'static str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn write_texture(queue: &wgpu::Queue, texture: &wgpu::Texture, bytes: &[u8], bytes_per_row: u32) {
    queue.write_texture(
        texture.as_image_copy(),
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: Some(texture.height()),
        },
        texture.size(),
    );
}

fn source_rgb(y: u8, chroma: [u8; 2], matrix: [f32; 4]) -> [f32; 3] {
    let y = y as f32 / 255.0;
    let c = [
        chroma[0] as f32 / 255.0 - 128.0 / 255.0,
        chroma[1] as f32 / 255.0 - 128.0 / 255.0,
    ];
    [
        y + matrix[0] * c[1],
        y - matrix[1] * c[0] - matrix[2] * c[1],
        y + matrix[3] * c[0],
    ]
}

const PROBE: &str = r#"
@group(1) @binding(2) var<storage, read_write> colour_answers: array<vec4<f32>>;

@compute @workgroup_size(1)
fn sample_source_rgb(@builtin(global_invocation_id) id: vec3<u32>) {
  let uv = array<vec2<f32>, 4>(
    vec2<f32>(0.25, 0.25), vec2<f32>(0.75, 0.25),
    vec2<f32>(0.25, 0.75), vec2<f32>(0.75, 0.75));
  colour_answers[id.x] = vec4<f32>(type2_ycbcr(uv[id.x]), 1.0);
}
"#;
