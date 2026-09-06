//! Bitwise division regression independent of the image solver fixtures.

use wgpu::util::DeviceExt;

#[test]
fn corrected_division_matches_binary32_edges_random_inputs_and_fallback() {
    let (device, queue, adapter) = match super::tests::gpu() {
        Ok(gpu) => gpu,
        Err(error) => {
            assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none(), "{error}");
            eprintln!("division regression skipped without GPU: {error}");
            return;
        }
    };
    let edges = [
        0,
        1,
        2,
        3,
        0x007f_ffff,
        0x0080_0000,
        0x0080_0001,
        0x00ff_ffff,
        0x3eff_ffff,
        0x3f00_0000,
        0x3f7f_ffff,
        0x3f80_0000,
        0x3f80_0001,
        0x3fff_ffff,
        0x4000_0000,
        0x4000_0001,
        0x7f00_0000,
        0x7f7f_ffff,
        0x7f80_0000,
        0x7f80_0001,
        0x7fc0_0000,
    ];
    let mut pairs = Vec::new();
    for a in edges {
        for b in edges {
            for signs in 0..4 {
                pairs.push([a | ((signs & 1) << 31), b | ((signs >> 1) << 31)]);
            }
        }
    }
    let mut state = 0x7123_abcdu32;
    let mut random = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    for _ in 0..65_536 {
        pairs.push([random(), random()]);
    }
    let expected: Vec<_> = pairs.iter().map(|&[a, b]| reference(a, b)).collect();
    let start = super::SHADER.find("fn div_f32_bits").unwrap();
    let end = super::SHADER.find("\nfn div_rn").unwrap();
    let division = &super::SHADER[start..end];
    // A deliberately bad estimate must exercise the restoring fallback, not
    // silently turn the exact divider into approximate arithmetic.
    let fallback = division.replacen(
        "u32((f32(numerator) / f32(mb)) * 8388608.0)",
        "u32((f32(numerator) / f32(mb)) * 8388608.0) + 32u",
        1,
    );
    assert_ne!(division, fallback);
    for (label, division) in [("hardware estimate", division), ("fallback", &fallback)] {
        let actual = divide(&device, &queue, division, &pairs);
        for (index, (&actual, &expected)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!(
                actual, expected,
                "{adapter} {label}, pair {index} {:08x?}: got {actual:08x}, expected {expected:08x}",
                pairs[index]
            );
        }
        eprintln!("{label}: {} bit-exact divisions on {adapter}", pairs.len());
    }
}

fn reference(a: u32, b: u32) -> u32 {
    let aa = a & 0x7fff_ffff;
    let bb = b & 0x7fff_ffff;
    if aa > 0x7f80_0000 {
        return a | 0x0040_0000;
    }
    if bb > 0x7f80_0000 {
        return b | 0x0040_0000;
    }
    if (aa == 0 && bb == 0) || (aa == 0x7f80_0000 && bb == 0x7f80_0000) {
        return 0xffc0_0000;
    }
    (f32::from_bits(a) / f32::from_bits(b)).to_bits()
}

fn divide(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    division: &str,
    pairs: &[[u32; 2]],
) -> Vec<u32> {
    let source = format!(
        r#"
{division}
@group(0) @binding(0) var<storage, read> inputs: array<vec2<u32>>;
@group(0) @binding(1) var<storage, read_write> outputs: array<u32>;
@compute @workgroup_size(64)
fn run(@builtin(global_invocation_id) id: vec3<u32>) {{
    if id.x < arrayLength(&inputs) {{
        outputs[id.x] = div_f32_bits(inputs[id.x].x, inputs[id.x].y);
    }}
}}
"#
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("corrected division regression"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("corrected division regression"),
        layout: None,
        module: &module,
        entry_point: Some("run"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bytes: Vec<_> = pairs
        .iter()
        .flatten()
        .flat_map(|word| word.to_ne_bytes())
        .collect();
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: &bytes,
        usage: wgpu::BufferUsages::STORAGE,
    });
    let size = (pairs.len() * 4) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
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
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups((pairs.len() as u32).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    receiver.recv().unwrap().unwrap();
    let bytes = readback.slice(..).get_mapped_range();
    bytes
        .chunks_exact(4)
        .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
        .collect()
}
