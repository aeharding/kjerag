//! **The twin guard: the shader and the mirror, asked the same question on
//! the same GPU.**
//!
//! Every number the seam is made of exists twice - once in
//! [`super::projection`] as Rust the tests can reach, and once in the WGSL
//! that file emits, which is what actually draws. The two are kept in step by
//! hand, and until 2026-08-09 nothing checked that they were: a review planted
//! a bend inside the WGSL `blend` alone, left the Rust twin untouched, and the
//! whole workspace stayed green while the rendered picture changed (the null
//! instrument's md5s moved, which is how it was caught, and the null needs
//! real footage and a second build of the whole tree to say so).
//!
//! **What this does.** It compiles the shipped `projection::wgsl()` - the same
//! string `Scene` hands wgpu, not a copy - with a compute entry point after it
//! that calls `blend` on a list of probe rays and writes what came back. Then
//! it asks [`Reframe::blend`] the same rays on the CPU and compares. A change
//! to one side and not the other fails here, in a unit test, with no footage.
//!
//! **What it does not do.** The fragment half - the NV12 sampling, the colour
//! transform, the write to the target - is not covered: it needs decoded
//! planes, which means real footage, which means the null instrument. The
//! boundary this guards is the one the seam work keeps moving, which is every
//! function of the map: `blend`, `handover`, `crossover`, `claim`, `share`,
//! `within`, `axis_of`, `project` and `mei`, plus the layout of the uniform
//! block they all read.
//!
//! **It needs a GPU and says so.** `cargo test --workspace` on a box with a
//! Vulkan device runs it; CI has no `/dev/dri/renderD128` and every runner
//! would skip, so `KJERAG_REQUIRE_GPU` makes the skip a failure instead and
//! `scripts/uitest.sh` sets it. That is the same seat the harness itself
//! occupies (AGENTS.md, UI verification): the gate CI cannot run, run on the
//! box that can, and not skippable on the way to a tag.

#![cfg(test)]

use crate::projection::{Held, MAX_LENSES, Reframe};
use crate::sampling::Sampling;
use crate::{Camera, Size, dmabuf};

/// Four `vec4`s of answer per probe, which is how the storage buffer is read
/// back. A struct would need WGSL's own alignment rules agreed on twice; a
/// lane of four floats needs none.
const LANES: usize = 4;

/// The compute half of this file: the probe entry the shipped map is
/// concatenated in front of.
///
/// Its bindings are group 1, so group 0 is exactly what
/// `super::projection::wgsl` declares and the uniform is bound the way the
/// draw binds it.
const PROBE: &str = r#"
@group(1) @binding(0) var<storage, read> probes: array<vec4<f32>>;
@group(1) @binding(1) var<storage, read_write> answers: array<vec4<f32>>;

@compute @workgroup_size(64)
fn twin(@builtin(global_invocation_id) id: vec3<u32>) {
  let index = id.x;
  if index >= arrayLength(&probes) {
    return;
  }
  let out = blend(probes[index].xyz);
  let base = index * 4u;
  answers[base + 0u] = vec4<f32>(out.weights[0], out.weights[1], 0.0, 0.0);
  answers[base + 1u] = vec4<f32>(out.landings[0].pixel, out.landings[1].pixel);
  answers[base + 2u] = vec4<f32>(
    out.landings[0].depth,
    out.landings[1].depth,
    out.landings[0].axis,
    out.landings[1].axis,
  );
  answers[base + 3u] = vec4<f32>(
    f32(out.landings[0].inside),
    f32(out.landings[1].inside),
    0.0,
    0.0,
  );
}
"#;

/// Waits for one of wgpu's futures on this thread.
///
/// Twelve lines rather than a dependency: `pollster` is already in the lock
/// file for `kjerag-spike`, but a dev-dependency here would still be a change
/// to `Cargo.lock`, and a change to `Cargo.lock` is a change to
/// `flatpak/cargo-sources.json` (AGENTS.md) for a test that never leaves this
/// box. Enumerating adapters on native is ready on the first poll; the yield
/// is there so that a backend which is not cannot turn this into a spin.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(answer) => return answer,
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}

/// A Vulkan device with nothing on it, or why there is none.
fn gpu() -> Result<(wgpu::Device, wgpu::Queue, String), String> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
        .into_iter()
        .next()
        .ok_or("no Vulkan adapter")?;
    let name = adapter.get_info().name;
    let (device, queue) = dmabuf::open_device(&adapter).map_err(|e| e.to_string())?;
    Ok((device, queue, name))
}

/// What the shader answered for one ray.
#[derive(Clone, Copy, Debug)]
struct Answer {
    weights: [f32; MAX_LENSES],
    pixel: [[f32; 2]; MAX_LENSES],
    depth: [f32; MAX_LENSES],
    axis: [f32; MAX_LENSES],
    inside: [bool; MAX_LENSES],
}

/// Runs the shipped map on the GPU over `rays` and reads every landing and
/// weight back.
fn on_the_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    reframe: &Reframe,
    rays: &[[f32; 3]],
) -> Vec<Answer> {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("twin"),
        source: wgpu::ShaderSource::Wgsl(format!("{}\n{PROBE}", crate::projection::wgsl()).into()),
    });
    let block = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("twin block"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Reframe>() as u64),
            },
            count: None,
        }],
    });
    let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let probes = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("twin probes"),
        entries: &[storage(0, true), storage(1, false)],
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("twin"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("twin"),
                bind_group_layouts: &[&block, &probes],
                immediate_size: 0,
            }),
        ),
        module: &module,
        entry_point: Some("twin"),
        compilation_options: Default::default(),
        cache: None,
    });

    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("twin block"),
        size: std::mem::size_of::<Reframe>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&uniform, 0, reframe.bytes());

    let padded: Vec<[f32; 4]> = rays.iter().map(|r| [r[0], r[1], r[2], 0.0]).collect();
    let ray_bytes = bytes_of(&padded);
    let input = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("twin rays"),
        size: ray_bytes.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&input, 0, ray_bytes);

    let answers_bytes = (rays.len() * LANES * 16) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("twin answers"),
        size: answers_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("twin readback"),
        size: answers_bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let block_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("twin block"),
        layout: &block,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    let probe_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("twin probes"),
        layout: &probes,
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
        pass.set_bind_group(0, &block_group, &[]);
        pass.set_bind_group(1, &probe_group, &[]);
        pass.dispatch_workgroups(rays.len().div_ceil(64) as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, answers_bytes);
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the probe pass did not finish");

    let mapped = readback.slice(..).get_mapped_range();
    let floats: Vec<f32> = mapped
        .chunks_exact(4)
        .map(|word| f32::from_le_bytes([word[0], word[1], word[2], word[3]]))
        .collect();
    drop(mapped);
    readback.unmap();

    (0..rays.len())
        .map(|probe| {
            let lane = &floats[probe * LANES * 4..(probe + 1) * LANES * 4];
            Answer {
                weights: [lane[0], lane[1]],
                pixel: [[lane[4], lane[5]], [lane[6], lane[7]]],
                depth: [lane[8], lane[9]],
                axis: [lane[10], lane[11]],
                inside: [lane[12] != 0.0, lane[13] != 0.0],
            }
        })
        .collect()
}

fn bytes_of(rows: &[[f32; 4]]) -> &[u8] {
    // `[f32; 4]` has no padding and no invalid pattern, and this is the same
    // cast `Reframe::bytes` makes for the block itself.
    unsafe { std::slice::from_raw_parts(rows.as_ptr().cast::<u8>(), std::mem::size_of_val(rows)) }
}

/// The rays the two halves are compared on: a fine walk straight across the
/// seam at every fifteen degrees of azimuth, plus the far field of each lens
/// and both poles, so a difference that only shows where one lens has the ray
/// is not missed.
///
/// Laid out in the camera BODY's frame, where the seam circle stands still,
/// and handed to the map in the view's, which is the frame it reads
/// ([`Reframe::view_ray_from_body`]). Written the other way round the walk
/// would be a walk across the seam only at yaw zero, and this fixture is
/// deliberately turned.
fn probe_rays(reframe: &Reframe) -> Vec<[f32; 3]> {
    let mut rays = Vec::new();
    let mut push = |theta_deg: f32, phi_deg: f32| {
        let (theta, phi) = (theta_deg.to_radians(), phi_deg.to_radians());
        rays.push(reframe.view_ray_from_body([
            theta.sin() * phi.cos(),
            theta.sin() * phi.sin(),
            theta.cos(),
        ]));
    };
    for phi in (0..360).step_by(15) {
        let phi = phi as f32;
        for step in -120..=120 {
            push(90.0 + step as f32 * 0.1, phi);
        }
        for theta in [5.0f32, 30.0, 60.0, 120.0, 150.0, 175.0] {
            push(theta, phi);
        }
    }
    push(0.0, 0.0);
    push(180.0, 0.0);
    rays
}

/// **The shipped shader and the Rust twin answer the same numbers.**
///
/// The map is built at a turned camera and with the handover line held off the
/// seam, because both of those are terms that reach the shader through the
/// uniform block: a block whose fields slid would fail here as loudly as a
/// function whose arithmetic did.
///
/// **The bars, and where they come from.** Measured on RADV Phoenix
/// 2026-08-09 over these 5930 rays, 1921 of them inside the handover and 7851
/// landings that reach a pixel: every weight agrees to **2.1e-6**, every
/// landing to **9.8e-4 px**, every coverage depth to **7.3e-4 px** and every
/// landing axis to **1.2e-7**. The
/// bars are ten times that, which still leaves them three orders under the
/// smallest change either half could make and be doing anything: the bend a
/// review planted in the WGSL `blend` alone moves a landing by whole pixels
/// and a weight by hundredths.
///
/// **Why the landings are compared where the weight is not zero, and not
/// everywhere.** A lens the ray cannot reach is never projected, and its
/// landing is whatever the slot held. WGSL says a `var` with no initializer is
/// zeroed, and on this box it is not re-zeroed per iteration of `blend`'s
/// loop: measured here 2026-08-09, a ray that only lens 0 has comes back with
/// **lens 0's landing in lens 1's slot**, at a weight of exactly zero. It
/// reaches no pixel - `fs` samples each lens behind `mix.weights[i] > 0.0`,
/// and `texel_ratio` of a stale landing feeds only those branches - so it is a
/// difference between the two halves that no picture can carry, and this test
/// is written about the picture. Recorded rather than worked around: nothing
/// in the shipped shader is changed for it, because changing the shader is
/// changing the arm the owner approved.
///
/// The residue is not zero and is not expected to be: WGSL's `normalize` is
/// allowed a couple of ulps where the twin divides by its own `norm3`, and the
/// two are compiled by different compilers for different machines. What the
/// test asserts is that the two halves compute the same function, not that
/// they round the same way.
#[test]
fn the_shader_and_its_rust_twin_answer_the_same_map() {
    let (device, queue, name) = match gpu() {
        Ok(open) => open,
        Err(why) => {
            // CI has no `/dev/dri/renderD128` and cannot answer this; the
            // owner's box and `scripts/uitest.sh` can, and the variable is
            // what makes the difference between the two visible rather than
            // silent.
            assert!(
                std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}",
            );
            eprintln!("twin: skipped, no GPU on this box ({why})");
            return;
        }
    };
    eprintln!("twin: {name}");

    let lenses = crate::projection::tests::fixture_lenses();
    let reframe = Reframe::new(
        &lenses,
        Size {
            width: 3840,
            height: 3840,
        },
        Camera {
            yaw: 74f32.to_radians(),
            pitch: -31f32.to_radians(),
            fov: 55f32.to_radians(),
        },
        Held::default(),
        16.0 / 9.0,
        true,
        Sampling::default(),
    )
    .with_shift(2.5f32.to_radians());
    // The block the shader is handed carries the held line, or this test would
    // be run on the one field the seam anchor added.
    assert!(reframe.handover_width() > 0.0);

    let rays = probe_rays(&reframe);
    let answers = on_the_gpu(&device, &queue, &reframe, &rays);
    assert_eq!(answers.len(), rays.len());

    let (mut worst_weight, mut worst_pixel) = (0.0f32, 0.0f32);
    let (mut worst_depth, mut worst_axis) = (0.0f32, 0.0f32);
    let (mut mixed, mut landings) = (0usize, 0usize);
    for (ray, answer) in rays.iter().zip(&answers) {
        let mirror = reframe.blend(*ray);
        if mirror.weights.iter().all(|weight| *weight > 0.0) {
            mixed += 1;
        }
        for lens in 0..MAX_LENSES {
            // The weight is compared everywhere, and it is the number that
            // carries `inside`: `claim` answers zero for a landing that is
            // outside, so a disagreement about coverage shows up here as a
            // disagreement about the weight.
            worst_weight = worst_weight.max((answer.weights[lens] - mirror.weights[lens]).abs());
            // The landing is compared where it reaches a pixel, which is
            // where its weight is not zero (`fs` samples each lens behind
            // `mix.weights[i] > 0.0`). See the doc above for the measured
            // reason that boundary is drawn here and not further out.
            if mirror.weights[lens] <= 0.0 {
                continue;
            }
            landings += 1;
            assert!(
                answer.inside[lens],
                "lens {lens} carries weight {} on the mirror and is outside on the shader at \
                 {ray:?}",
                mirror.weights[lens],
            );
            for axis in 0..2 {
                worst_pixel = worst_pixel
                    .max((answer.pixel[lens][axis] - mirror.landings[lens].pixel[axis]).abs());
            }
            worst_depth = worst_depth.max((answer.depth[lens] - mirror.landings[lens].depth).abs());
            worst_axis = worst_axis.max((answer.axis[lens] - mirror.landings[lens].axis).abs());
        }
    }
    // The rays have to actually cross the seam, or the whole comparison is
    // about the far field where one lens takes everything at a weight of one.
    assert!(
        mixed > 500,
        "only {mixed} of {} probes are inside the handover, so this compares the far field",
        rays.len(),
    );
    assert!(
        landings > 4000,
        "only {landings} landings reach a pixel, so this compares almost nothing",
    );
    assert!(
        worst_weight < 2e-5,
        "the two halves disagree by {worst_weight} of a whole weight",
    );
    assert!(
        worst_pixel < 1e-2,
        "the two halves land {worst_pixel} px apart",
    );
    assert!(
        worst_depth < 1e-2,
        "the two halves read coverage depths {worst_depth} px apart",
    );
    assert!(
        worst_axis < 1e-6,
        "the two halves read the landing's own axis cosine {worst_axis} apart",
    );
    eprintln!(
        "twin: {} rays, {mixed} inside the handover, {landings} landings compared; worst \
         weight {worst_weight:.3e}, worst landing {worst_pixel:.3e} px, worst depth \
         {worst_depth:.3e} px, worst axis {worst_axis:.3e}",
        rays.len(),
    );
}
