//! GPU qualification for explicit captured image-fusion map consumption.

use super::*;
use crate::image_fusion::{RatioMap, RatioPair, correct};
use crate::studio_type2::{MAP_HEIGHT, MAP_NODES, MAP_WIDTH};

#[test]
fn optional_fusion_shader_declares_only_its_enabled_group() {
    use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

    let enabled = draw_wgsl_with_fusion(true);
    let filtered = draw_wgsl_with_fusion_mode(true, true);
    let disabled = draw_wgsl_with_fusion(false);
    for source in [&enabled, &filtered, &disabled] {
        let module = wgpu::naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|error| panic!("direct fusion WGSL did not parse: {error}"));
        Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|error| panic!("direct fusion WGSL did not validate: {error}"));
    }

    assert!(enabled.contains("@group(2) @binding(0) var fusion_left: texture_2d<f32>"));
    assert!(enabled.contains("@group(2) @binding(1) var fusion_right: texture_2d<f32>"));
    assert!(enabled.contains("textureLoad(fusion_left, at, 0).xyz"));
    assert!(enabled.contains("textureLoad(fusion_right, at, 0).xyz"));
    assert!(!enabled.contains("fusion_linear"));
    assert!(filtered.contains("@group(2) @binding(2) var fusion_linear: sampler"));
    assert!(filtered.contains("textureSampleLevel(fusion_left, fusion_linear, uv, 0.0).xyz"));
    assert!(filtered.contains("textureSampleLevel(fusion_right, fusion_linear, uv, 0.0).xyz"));
    assert!(enabled.contains("let ratio = mix(mix(a, b, q.z), mix(c, d, q.z), q.w);"));
    assert!(enabled.contains("if map.alpha == 0.0"));
    assert!(enabled.contains("} else if map.alpha == 1.0"));
    assert!(enabled.contains("let b = type2_correct("));
    assert!(enabled.contains("let a = type2_correct("));
    assert!(enabled.contains("rgb = mix(b, a, map.alpha);"));
    assert!(!disabled.contains("@group(2)"));
    assert!(!disabled.contains("fusion_left"));
    assert!(disabled.contains("{ return color; }"));
}

#[test]
fn gpu_fusion_sampling_and_preblend_match_the_scalar_consumer() {
    qualify_gpu_fusion(false);
}

#[test]
fn gpu_hardware_fusion_sampling_stays_within_half_an_output_code() {
    qualify_gpu_fusion(true);
}

fn qualify_gpu_fusion(hardware_fusion: bool) {
    // Hardware filtering quantizes interpolation weights. Its separate gate
    // permits at most half an 8-bit output code, not numerical identity with
    // explicit f32 mixes. Preserve the manual consumer's original bound.
    let tolerance = if hardware_fusion { 0.5 / 255.0 } else { 2.0e-6 };
    let (device, queue) = match fusion_gpu(hardware_fusion) {
        Ok(gpu) => gpu,
        Err(error) => {
            assert!(
                !hardware_fusion || !error.contains("advertised FLOAT32_FILTERABLE"),
                "{error}"
            );
            assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none(), "{error}");
            let mode = if hardware_fusion {
                "optional filtered"
            } else {
                "manual"
            };
            eprintln!("skipping {mode} direct fusion consumer: {error}");
            return;
        }
    };

    let pair = ratio_pair();
    let samples = [
        Sample::new([0.0, 0.0], 0.0, [0.2, 0.7, 1.2], [-0.2, 0.4, 0.9]),
        Sample::new([1.0, 1.0], 1.0, [0.8, -0.1, 0.3], [0.5, 0.9, 1.4]),
        Sample::new(
            [0.5 / MAP_WIDTH as f32, 0.5 / MAP_HEIGHT as f32],
            0.5,
            [0.1, 0.5, 0.9],
            [0.9, 0.5, 0.1],
        ),
        Sample::new(
            [199.5 / MAP_WIDTH as f32, 99.5 / MAP_HEIGHT as f32],
            0.25,
            [0.35, 0.65, 0.95],
            [0.95, 0.65, 0.35],
        ),
        // Moderately outside the chart: X repeats and Y clamps at each pole.
        Sample::new([-0.0025, -0.1], 0.75, [0.0, 0.4, 1.0], [1.0, 0.4, 0.0]),
        Sample::new([1.0025, 1.1], 0.125, [0.6, 0.2, 0.8], [0.2, 0.8, 0.6]),
        Sample::new([0.25125, 0.335], 0.625, [0.3, 0.6, 0.9], [0.9, 0.3, 0.6]),
        // Arbitrary f32 fractions exercise hardware filter-coordinate
        // rounding. The first two approach the periodic join from each side
        // and select one lens apiece; the others exercise pre-blend sampling.
        Sample::new(
            [-0.000617283, 0.503271],
            0.0,
            [0.17, 0.53, 0.91],
            [0.93, 0.41, 0.08],
        ),
        Sample::new(
            [1.0006173, 0.497193],
            1.0,
            [0.81, 0.27, 0.63],
            [0.11, 0.79, 0.37],
        ),
        Sample::new(
            [0.1234567, 0.2345679],
            0.37,
            [0.23, 0.61, 0.97],
            [0.89, 0.31, 0.07],
        ),
        Sample::new(
            [0.7312345, 0.8765432],
            0.83,
            [0.74, 0.19, 0.58],
            [0.16, 0.86, 0.43],
        ),
    ];
    let packed_inputs: Vec<[f32; 4]> = samples
        .iter()
        .flat_map(|sample| {
            [
                [sample.uv[0], sample.uv[1], sample.alpha, 0.0],
                [sample.left[0], sample.left[1], sample.left[2], 0.0],
                [sample.right[0], sample.right[1], sample.right[2], 0.0],
            ]
        })
        .collect();

    let source = format!(
        "{}\n{PROBE}",
        draw_wgsl_with_fusion_mode(true, hardware_fusion)
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("direct fusion consumer qualification"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("direct fusion consumer qualification"),
        layout: None,
        module: &module,
        entry_point: Some("qualify_fusion"),
        compilation_options: Default::default(),
        cache: None,
    });
    let buffer = |label, size, usage| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    let ratios = ["left fusion ratio fixture", "right fusion ratio fixture"].map(|label| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: MAP_WIDTH as u32,
                height: MAP_HEIGHT as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    });
    for (texture, ratio) in ratios.iter().zip([&pair.left, &pair.right]) {
        queue.write_texture(
            texture.as_image_copy(),
            ratio.bytes(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((MAP_WIDTH * std::mem::size_of::<[f32; 4]>()) as u32),
                rows_per_image: Some(MAP_HEIGHT as u32),
            },
            texture.size(),
        );
    }
    let ratio_views = ratios
        .each_ref()
        .map(|ratio| ratio.create_view(&Default::default()));
    let fusion_sampler = hardware_fusion.then(|| {
        device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("qualified image fusion full-f32 bilinear sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        })
    });
    let mut fusion_entries = ratio_views
        .iter()
        .enumerate()
        .map(|(binding, view)| wgpu::BindGroupEntry {
            binding: binding as u32,
            resource: wgpu::BindingResource::TextureView(view),
        })
        .collect::<Vec<_>>();
    if let Some(sampler) = fusion_sampler.as_ref() {
        fusion_entries.push(wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::Sampler(sampler),
        });
    }
    let fusion = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("direct fusion ratio fixtures"),
        layout: &pipeline.get_bind_group_layout(2),
        entries: &fusion_entries,
    });

    let input = buffer(
        "direct fusion sample inputs",
        std::mem::size_of_val(packed_inputs.as_slice()) as u64,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    );
    queue.write_buffer(&input, 0, bytes_of(&packed_inputs));
    let output_bytes = (samples.len() * std::mem::size_of::<[f32; 4]>()) as u64;
    let output = buffer(
        "direct fusion sample answers",
        output_bytes,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let readback = buffer(
        "direct fusion sample readback",
        output_bytes,
        wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
    );
    let probe = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("direct fusion probe inputs and answers"),
        layout: &pipeline.get_bind_group_layout(1),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 2,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let empty = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("unused picture group in fusion probe"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &empty, &[]);
        pass.set_bind_group(1, &probe, &[]);
        pass.set_bind_group(2, &fusion, &[]);
        pass.dispatch_workgroups(samples.len() as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output_bytes);
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = readback.slice(..).get_mapped_range();

    let mut saw_clamp = false;
    let mut worst = (0.0f32, 0usize, 0usize, 0.0f32, 0.0f32);
    let mode = if hardware_fusion {
        "hardware"
    } else {
        "manual"
    };
    for (index, (sample, actual)) in samples
        .iter()
        .zip(bytes.chunks_exact(16).map(|bytes| {
            std::array::from_fn::<_, 4, _>(|channel| {
                f32::from_le_bytes(bytes[channel * 4..channel * 4 + 4].try_into().unwrap())
            })
        }))
        .enumerate()
    {
        let left = correct(sample.left, pair.left.sample(sample.uv));
        let right = correct(sample.right, pair.right.sample(sample.uv));
        assert_ne!(
            left, right,
            "sample {index} did not distinguish the two maps"
        );
        saw_clamp |= left
            .into_iter()
            .chain(right)
            .any(|channel| channel == 0.0 || channel == 1.0);
        let expected = if sample.alpha == 0.0 {
            right
        } else if sample.alpha == 1.0 {
            left
        } else {
            std::array::from_fn(|channel| {
                right[channel] * (1.0 - sample.alpha) + left[channel] * sample.alpha
            })
        };
        for channel in 0..3 {
            let error = if actual[channel].is_finite() {
                (actual[channel] - expected[channel]).abs()
            } else {
                f32::INFINITY
            };
            if error > worst.0 {
                worst = (error, index, channel, actual[channel], expected[channel]);
            }
            if error > tolerance {
                eprintln!(
                    "{mode} fusion mismatch sample {index} channel {channel}: GPU {} CPU {} error {error}",
                    actual[channel], expected[channel]
                );
            }
        }
        assert_eq!(actual[3].to_bits(), sample.alpha.to_bits());
    }
    assert!(saw_clamp, "fixture did not exercise correction clamping");
    eprintln!(
        "{mode} fusion worst sample {} channel {}: GPU {} CPU {} error {}",
        worst.1, worst.2, worst.3, worst.4, worst.0
    );
    assert!(
        worst.0 <= tolerance,
        "{mode} fusion worst sample {} channel {}: GPU {} CPU {} error {}",
        worst.1,
        worst.2,
        worst.3,
        worst.4,
        worst.0
    );
}

fn fusion_gpu(hardware_fusion: bool) -> Result<(wgpu::Device, wgpu::Queue), String> {
    if !hardware_fusion {
        return super::tests::gpu();
    }
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
        .into_iter()
        .next()
        .ok_or("no Vulkan adapter")?;
    let feature = wgpu::Features::FLOAT32_FILTERABLE;
    let info = adapter.get_info();
    if !adapter.features().contains(feature) {
        return Err(format!(
            "adapter {} does not support optional FLOAT32_FILTERABLE",
            info.name
        ));
    }
    block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("direct type-2 filtered fusion qualification"),
        required_features: feature,
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .map_err(|error| {
        format!(
            "adapter {} advertised FLOAT32_FILTERABLE but its device request failed: {error}",
            info.name
        )
    })
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

#[derive(Clone, Copy)]
struct Sample {
    uv: [f32; 2],
    alpha: f32,
    left: [f32; 3],
    right: [f32; 3],
}

impl Sample {
    const fn new(uv: [f32; 2], alpha: f32, left: [f32; 3], right: [f32; 3]) -> Self {
        Self {
            uv,
            alpha,
            left,
            right,
        }
    }
}

fn ratio_pair() -> RatioPair {
    let map = |right: bool| {
        RatioMap::new(
            (0..MAP_NODES)
                .map(|index| {
                    let x = (index % MAP_WIDTH) as f32;
                    let y = (index / MAP_WIDTH) as f32;
                    if right {
                        [
                            1.5 - x / 512.0,
                            0.625 + y / 512.0,
                            0.875 + (x + y) / 1024.0,
                            -7.0,
                        ]
                    } else {
                        [
                            0.75 + x / 1024.0,
                            1.0 + y / 512.0,
                            1.25 - (x + y) / 2048.0,
                            9.0,
                        ]
                    }
                })
                .collect(),
        )
        .unwrap()
    };
    RatioPair {
        left: map(false),
        right: map(true),
    }
}

fn bytes_of<T>(values: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

const PROBE: &str = r#"
@group(1) @binding(2) var<storage, read> fusion_probe_inputs: array<vec4<f32>>;
@group(1) @binding(3) var<storage, read_write> fusion_probe_answers: array<vec4<f32>>;

@compute @workgroup_size(1)
fn qualify_fusion(@builtin(global_invocation_id) id: vec3<u32>) {
  let uv_alpha = fusion_probe_inputs[id.x * 3u];
  let left = type2_correct(fusion_probe_inputs[id.x * 3u + 1u].xyz, uv_alpha.xy, 0u);
  let right = type2_correct(fusion_probe_inputs[id.x * 3u + 2u].xyz, uv_alpha.xy, 1u);
  var color: vec3<f32>;
  if uv_alpha.z == 0.0 {
    color = right;
  } else if uv_alpha.z == 1.0 {
    color = left;
  } else {
    color = mix(right, left, uv_alpha.z);
  }
  fusion_probe_answers[id.x] = vec4<f32>(color, uv_alpha.z);
}
"#;
