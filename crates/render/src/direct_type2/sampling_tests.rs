//! Compare the actual texture-filtering entry with the previous manual atlas,
//! including the split between the two independent decoder textures.

use super::*;

#[test]
fn hardware_atlas_sampling_keeps_join_and_clamp_edges() {
    let (device, queue) = match super::tests::gpu() {
        Ok(gpu) => gpu,
        Err(error) => {
            assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none(), "{error}");
            eprintln!("skipping hardware atlas sampler: {error}");
            return;
        }
    };
    // The original box law, using the original four-load linear sampler.
    // Extract the unchanged law so the probe also covers every inner sample.
    let reference_box = DRAW
        .split_once("fn type2_box(")
        .unwrap()
        .1
        .split_once("fn type2_ycbcr(")
        .unwrap()
        .0;
    // Keep the previous eager select in the oracle. The optimized path only
    // fetches the fallback sample if the box has no usable area.
    let lazy_return = "if area > 0.0010000000474974513 { return sum / area; }\n  return type2_atlas_linear(a, b, uv);";
    assert!(reference_box.contains(lazy_return));
    let reference_box = reference_box.replace(
        lazy_return,
        "return select(type2_atlas_linear(a, b, uv), sum / area, area > 0.0010000000474974513);",
    );
    let source = format!(
        "{}\nfn reference_box({}\n{}",
        draw_wgsl(),
        reference_box.replace("type2_atlas_linear(", "reference_linear("),
        PROBE,
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("hardware atlas sampler regression"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("hardware atlas sampler regression"),
        layout: None,
        module: &module,
        entry_point: Some("compare_atlas"),
        compilation_options: Default::default(),
        cache: None,
    });
    let texture = |lens: u32| {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas sampler high-contrast fixture"),
            size: wgpu::Extent3d {
                width: 32,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let bytes: Vec<u8> = (0..1024u32)
            .map(|i| ((i * 137 + (i / 64) * 47 + lens * 173) % 256) as u8)
            .collect();
        queue.write_texture(
            texture.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(64),
                rows_per_image: Some(16),
            },
            texture.size(),
        );
        texture.create_view(&Default::default())
    };
    let a = texture(0);
    let b = texture(1);
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        min_filter: wgpu::FilterMode::Linear,
        mag_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let pictures = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("atlas sampler fixtures"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&a),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&b),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    let buffer = |usage| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("atlas sampler comparison"),
            size: 257 * 33 * 16,
            usage,
            mapped_at_creation: false,
        })
    };
    let answers = buffer(wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
    let readback = buffer(wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ);
    let output = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("atlas sampler differences"),
        layout: &pipeline.get_bind_group_layout(1),
        entries: &[wgpu::BindGroupEntry {
            binding: 2,
            resource: answers.as_entire_binding(),
        }],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &pictures, &[]);
        pass.set_bind_group(1, &output, &[]);
        pass.dispatch_workgroups((257u32 * 33).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&answers, 0, &readback, 0, answers.size());
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    let mut worst = 0.0f32;
    for (index, bytes) in bytes.chunks_exact(4).enumerate() {
        let difference = f32::from_le_bytes(bytes.try_into().unwrap()).abs();
        // Filtering may quantize subtexel weights. This is a numeric guard,
        // not owner acceptance of the resulting video.
        assert!(
            difference.is_finite() && difference <= 1.01 / 255.0,
            "atlas sample {index} differs by {difference}"
        );
        worst = worst.max(difference);
    }
    eprintln!(
        "hardware atlas sampler worst difference: {} code values",
        worst * 255.0
    );
}

const PROBE: &str = r#"
fn reference_linear(a: texture_2d<f32>, b: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
  let dims = textureDimensions(a);
  let p = uv * vec2<f32>(f32(2u * dims.x), f32(dims.y)) - vec2<f32>(0.5);
  let base = vec2<i32>(floor(p));
  let f = fract(p);
  let top = mix(type2_atlas_load(a, b, base), type2_atlas_load(a, b, base + vec2<i32>(1, 0)), f.x);
  let bottom = mix(type2_atlas_load(a, b, base + vec2<i32>(0, 1)), type2_atlas_load(a, b, base + vec2<i32>(1, 1)), f.x);
  return mix(top, bottom, f.y);
}
@group(1) @binding(2) var<storage, read_write> answers: array<vec4<f32>>;
@compute @workgroup_size(64)
fn compare_atlas(@builtin(global_invocation_id) id: vec3<u32>) {
  if id.x >= 257u * 33u { return; }
  let x = id.x % 257u;
  let y = id.x / 257u;
  // Includes samples outside all four edges and all sides of the atlas join.
  // The middle row traverses both lenses at subpixel spacing.
  let uv = vec2<f32>(f32(x) / 256.0 * 1.2 - 0.1, f32(y) / 32.0 * 1.2 - 0.1);
  let linear = type2_atlas_linear(type2_luma0, type2_luma1, uv);
  let expected_linear = reference_linear(type2_luma0, type2_luma1, uv);
  let box = type2_box(type2_luma0, type2_luma1, uv, vec2<f32>(32.0, 16.0));
  let expected_box = reference_box(type2_luma0, type2_luma1, uv, vec2<f32>(32.0, 16.0));
  answers[id.x] = vec4<f32>(linear.rg - expected_linear.rg, box.rg - expected_box.rg);
}
"#;
