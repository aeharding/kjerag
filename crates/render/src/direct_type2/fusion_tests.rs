//! GPU qualification for explicit captured image-fusion map consumption.

use super::*;
use crate::image_fusion::{RatioMap, RatioPair, correct};
use crate::studio_type2::{MAP_HEIGHT, MAP_NODES, MAP_WIDTH, PACKED_BYTES};

#[test]
fn optional_fusion_shader_declares_only_its_enabled_group() {
    use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

    let enabled = draw_wgsl_with_fusion(true);
    let disabled = draw_wgsl_with_fusion(false);
    for source in [&enabled, &disabled] {
        let module = wgpu::naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|error| panic!("direct fusion WGSL did not parse: {error}"));
        Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|error| panic!("direct fusion WGSL did not validate: {error}"));
    }

    assert!(enabled.contains("@group(2) @binding(0) var<storage, read> fusion_left"));
    assert!(enabled.contains("@group(2) @binding(1) var<storage, read> fusion_right"));
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
    let (device, queue) = match super::tests::gpu() {
        Ok(gpu) => gpu,
        Err(error) => {
            assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none(), "{error}");
            eprintln!("skipping direct fusion consumer: {error}");
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

    let source = format!("{}\n{PROBE}", draw_wgsl_with_fusion(true));
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
    let ratios = [
        buffer(
            "left fusion ratio fixture",
            PACKED_BYTES as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        ),
        buffer(
            "right fusion ratio fixture",
            PACKED_BYTES as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        ),
    ];
    queue.write_buffer(&ratios[0], 0, pair.left.bytes());
    queue.write_buffer(&ratios[1], 0, pair.right.bytes());
    let fusion = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("direct fusion ratio fixtures"),
        layout: &pipeline.get_bind_group_layout(2),
        entries: &std::array::from_fn::<_, 2, _>(|binding| wgpu::BindGroupEntry {
            binding: binding as u32,
            resource: ratios[binding].as_entire_binding(),
        }),
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
            let error = (actual[channel] - expected[channel]).abs();
            assert!(
                actual[channel].is_finite() && error <= 2.0e-6,
                "fusion sample {index} channel {channel}: GPU {} CPU {} error {error}",
                actual[channel],
                expected[channel]
            );
        }
        assert_eq!(actual[3].to_bits(), sample.alpha.to_bits());
    }
    assert!(saw_clamp, "fixture did not exercise correction clamping");
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
