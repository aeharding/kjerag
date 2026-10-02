//! Compare the selected final transfer against the pre-optimization expression.
//! This isolates the transfer, not camera sampling, stitching or whole-frame quality.

use wgpu::util::DeviceExt;

fn probe_source() -> String {
    // Take the actual selected function tail, not a duplicate candidate formula.
    let selected = super::CORRECTION_WGSL
        .split_once("  let gamma = clamp(high + (filtered - current)")
        .unwrap()
        .1
        .split_once("\n}\n")
        .unwrap()
        .0;
    format!(
        r#"
struct Reframe {{ linearize: f32 }};
@group(0) @binding(0) var<storage, read> inputs: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> outputs: array<vec4<f32>>;
fn selected(high: vec3<f32>, current: vec3<f32>, filtered: vec3<f32>, flag: f32) -> vec4<f32> {{
  let reframe = Reframe(flag);
  let gamma = clamp(high + (filtered - current){selected}
}}
fn previous(high: vec3<f32>, current: vec3<f32>, filtered: vec3<f32>, flag: f32) -> vec4<f32> {{
  let gamma = clamp(high + (filtered - current), vec3<f32>(0.0), vec3<f32>(1.0));
  let linear = select(
    gamma / 12.92,
    pow((gamma + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)),
    gamma > vec3<f32>(0.04045),
  );
  return vec4<f32>(select(gamma, linear, flag > 0.5), 1.0);
}}
@compute @workgroup_size(64)
fn compare(@builtin(global_invocation_id) at: vec3<u32>) {{
  let count = arrayLength(&inputs) / 3u;
  if at.x >= count {{ return; }}
  let high = inputs[at.x * 3u];
  let current = inputs[at.x * 3u + 1u].xyz;
  let filtered = inputs[at.x * 3u + 2u].xyz;
  outputs[at.x * 2u] = selected(high.xyz, current, filtered, high.w);
  outputs[at.x * 2u + 1u] = previous(high.xyz, current, filtered, high.w);
}}
"#
    )
}

#[test]
fn selected_transfer_probe_validates() {
    let module = wgpu::naga::front::wgsl::parse_str(&probe_source()).unwrap();
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
}

#[test]
fn gpu_selected_transfer_matches_previous_for_encoded_and_linear_output() {
    let Ok((device, queue)) = crate::direct_type2::tests::gpu() else {
        assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none());
        return;
    };
    let mut inputs = Vec::<[f32; 4]>::new();
    // Entire RGB8 range, clamped positive/negative residuals, and adjacent
    // binary32 values around the piecewise transfer's threshold. Both modes
    // are used by real output formats; no new transfer approximation is tested.
    let threshold = 0.04045_f32;
    let edge = [
        0.0,
        f32::from_bits(threshold.to_bits() - 1),
        threshold,
        f32::from_bits(threshold.to_bits() + 1),
        1.0,
    ];
    for flag in [0.0, 1.0] {
        for value in (0..=255).map(|v| v as f32 / 255.0).chain(edge) {
            for residual in [-1.0, -0.05, 0.0, 0.05, 1.0] {
                inputs.extend([
                    [value, 1.0 - value, threshold, flag],
                    [0.5, 0.5, 0.5, 0.0],
                    [0.5 + residual, 0.5 - residual, 0.5, 0.0],
                ]);
            }
        }
    }
    let count = inputs.len() as u32 / 3;
    let input_bytes = inputs
        .iter()
        .flatten()
        .flat_map(|value| value.to_ne_bytes())
        .collect::<Vec<_>>();
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("final transfer regression inputs"),
        contents: &input_bytes,
        usage: wgpu::BufferUsages::STORAGE,
    });
    let bytes = u64::from(count) * 2 * 16;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("paired final transfer outputs"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("final transfer regression readback"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("selected versus previous final transfer"),
        source: wgpu::ShaderSource::Wgsl(probe_source().into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("final transfer regression"),
        layout: None,
        module: &shader,
        entry_point: Some("compare"),
        compilation_options: Default::default(),
        cache: None,
    });
    let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("final transfer regression"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &binding, &[]);
        pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
    let submission = queue.submit([encoder.finish()]);
    let (sent, received) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sent.send(result).unwrap();
        });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    received.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range();
    for (case, pair) in mapped.chunks_exact(32).enumerate() {
        assert_eq!(
            &pair[..16],
            &pair[16..],
            "final transfer differs at case {case}"
        );
        assert_eq!(&pair[12..16], &1.0_f32.to_ne_bytes());
    }
    drop(mapped);
    readback.unmap();
    eprintln!("final transfer: {count} paired cases, encoded and linear output, zero changed bits");
}
