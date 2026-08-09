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
//! `within`, `axis_of`, `project`, `readout_share`, `turned` and `mei`, plus
//! the layout of the uniform block they all read.
//!
//! **A guarded function is only guarded at a fixture that reaches it, and
//! this file learned that on 2026-08-09.** The list above was written while
//! the fixture was built on `Held::default()`, whose `rolling` is `None`,
//! whose `row_axis` is therefore zero, and past whose uniform test the whole
//! readout half of `project` - `readout_share`, `turned`, and
//! `READOUT_STEPS` more rounds of `mei` - never ran on
//! either side. A review multiplied the WGSL `readout_share` by three, left
//! the Rust twin alone, and 226 of 226 tests passed while the rendered
//! picture moved (`down1` went `7d2200ea` to `10d51545`). The fixture rolls
//! now, and the test asserts that it does.
//!
//! **It needs a GPU and says so.** `cargo test --workspace` on a box with a
//! Vulkan device runs it; CI has no `/dev/dri/renderD128` and every runner
//! would skip, so `KJERAG_REQUIRE_GPU` makes the skip a failure instead and
//! `scripts/uitest.sh` sets it. That is the same seat the harness itself
//! occupies (AGENTS.md, UI verification): the gate CI cannot run, run on the
//! box that can, and not skippable on the way to a tag.

#![cfg(test)]

use crate::projection::{Held, MAX_LENSES, Reframe, Rolling};
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
  let split = exposure_split();
  answers[base + 3u] = vec4<f32>(
    f32(out.landings[0].inside),
    f32(out.landings[1].inside),
    split.x,
    split.y,
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
    /// What the deterministic exposure normalization multiplies each lens by
    /// (issue #103, stage 10 step P.1). Constant over the rays - it reads one
    /// uniform and no ray at all - which is exactly why it is carried in the
    /// two words lane 3 had spare rather than in a probe of its own.
    exposure: [f32; MAX_LENSES],
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
                exposure: [lane[14], lane[15]],
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
/// The map is built at a turned camera, with the handover line held off the
/// seam, and with the frame read out while the body turns, because all three
/// of those are terms that reach the shader through the uniform block: a block
/// whose fields slid would fail here as loudly as a function whose arithmetic
/// did, and a term left at its default takes the code that reads it out of the
/// comparison entirely (see the module doc, and the fixture below).
///
/// **The bars, and where they come from.** Measured on RADV Phoenix
/// 2026-08-09 over these 5930 rays, 1921 of them inside the handover and 7851
/// landings that reach a pixel: every weight agrees to **1.8e-6**, every
/// landing to **7.3e-4 px**, every coverage depth to **8.5e-4 px** and every
/// landing axis to **1.2e-7**. The bars are roughly ten times that, which
/// still leaves them orders under the smallest change either half could make
/// and be doing anything. Measured, both here on the same day:
///
/// | mutation, WGSL only | worst weight | of the bar | of the residue |
/// | --- | ---: | ---: | ---: |
/// | a bend inside `blend`, each lens sampled at a ray the other lens's share displaces | 4.4e-3 | 221x | 2400x |
/// | `readout_share` multiplied by three | 2.0e-2 | 984x | 10700x |
///
/// Each was the **only** failing test in the workspace, and each was reverted.
///
/// **Why the landings are compared where the weight is not zero, and not
/// everywhere.** A lens the ray cannot reach is never projected, and its
/// landing is whatever the slot held. WGSL says a `var` with no initializer is
/// zeroed, and on this box it is not re-zeroed per iteration of `blend`'s
/// loop: measured here 2026-08-09, a ray that only lens 0 has comes back with
/// **lens 0's landing in lens 1's slot**, at a weight of exactly zero.
///
/// **Why that reaches no pixel, in full, because the short version is not
/// enough.** `picture` samples each lens behind `mix.weights[i] > 0.0`, so the
/// stale landing is never a texture coordinate. It is not never READ, though:
/// `fs` computes `texel_ratio` for both lenses **outside** that gate
/// (`scene.rs`, `fs`), and deliberately, because `texel_ratio` is `dpdx`/`dpdy`
/// and a derivative has to be taken where every lane of the quad is running.
/// So a stale landing does feed a derivative, and the question is whether a
/// quad can straddle the boundary with some lanes sampling and some lanes
/// stale.
///
/// **It cannot, and `CAP_MARGIN_DEG` is why.** What decides whether a landing
/// is stale is `within`, and the cap it tests is the lens's own coverage
/// boundary widened by half a degree - thirty times the 0.016 degrees the
/// fixture's boundary actually needs
/// (`the_cap_is_tight_against_the_support`). The weight, meanwhile, has
/// already reached zero AT that coverage boundary, because `claim` multiplies
/// the share by the landing's own coverage depth and the depth is the distance
/// to the rim. So the ring where a landing is stale lies wholly outside the
/// ring where that lens's weight is non-zero, with half a degree of world
/// angle between the two, and a quad is two pixels of the view - 0.06 degrees
/// at this fixture's 55 degree field over a 1920 px window. A quad that
/// straddles the `within` boundary therefore has weight zero in every one of
/// its four lanes, and the ratio it computes is multiplied by nothing.
///
/// **Checked rather than argued**, by the review that raised it: the re-zero
/// this shader does not do was planted in it - `landing = Landing()` at the
/// top of each iteration - and the null rendered **byte-identical** summaries
/// at three views. Recorded rather than adopted: nothing in the shipped shader
/// is changed for it, because changing the shader is changing the arm the
/// owner approved.
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
    // **The frame is read out while the body turns, and that is not a
    // decoration.** `Held::default()` leaves `rolling` at `None`, which leaves
    // the block's `row_axis` at zero, which makes the WGSL test
    // `reframe.row_axis_x != 0.0 || reframe.row_axis_y != 0.0` false and the
    // Rust [`Reframe::is_rolling`] with it - and then the whole second half of
    // `project` (`readout_share`, `turned`, and `READOUT_STEPS` more rounds
    // of `mei`) runs on NEITHER side and this test compares a map that is not
    // the shipped one. A review proved that on 2026-08-09 by multiplying the
    // WGSL `readout_share` by three and nothing else: 226 of 226 tests passed
    // while the rendered picture moved (`down1` went 7d2200ea to 10d51545).
    let held = Held {
        rolling: Some(Rolling {
            // A roll about the lens axis with the other two turning as well,
            // so a `turn` that reached the shader transposed, negated or short
            // by a lane is a disagreement here rather than a coincidence. The
            // rate is the one the readout tests use: 90 deg/s across the X4
            // Air's 15.883 ms readout, 1.43 degrees from the first row of the
            // sensor to the last.
            turn: [
                0.3 * crate::projection::tests::READOUT_TURN,
                -0.6 * crate::projection::tests::READOUT_TURN,
                crate::projection::tests::READOUT_TURN,
            ],
            // `Sweep::Right`, which is how the X4 Air delivers: the leftmost
            // column is read first. Written out rather than imported so this
            // file names the two numbers the shader actually reads.
            axis: [1.0, 0.0],
        }),
        ..Held::default()
    };
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
        held,
        16.0 / 9.0,
        true,
        Sampling::default(),
    )
    .with_shift(2.5f32.to_radians())
    // **And the two lenses' shutters differ, or `exposure_split` returns its
    // literal `[1.0, 1.0]` on both sides and the comparison below is two
    // constants agreeing.** The value is a real one: 0.12 of a natural log is
    // the two lenses 12.7 percent apart in exposure time, which is inside the
    // 0.54-to-1.81 swing measured across two X4 Air captures
    // (docs/research/insv-format.md 6.3) and inside the `LIMIT_LN` clamp, so
    // this fixture reaches the arithmetic rather than the clamp.
    .with_exposure(0.12);
    // The block the shader is handed carries the held line, or this test would
    // be run on the one field the seam anchor added.
    assert!(reframe.handover_width() > 0.0);
    // And it carries the readout, or `project`'s second half is dead on both
    // halves and the boundary this file claims to guard is half a boundary.
    assert!(
        reframe.is_rolling(),
        "the block's row axis is zero, so the readout branch of `project` runs on neither side",
    );
    // And it carries a shutter difference, or `exposure_split` takes its exact
    // zero branch on both halves and a mutation of the arithmetic behind it
    // would pass. This is the same lesson `is_rolling` above is written for.
    assert!(
        reframe.exposure_split() != [1.0, 1.0],
        "the block's exposure ratio is zero, so `exposure_split` returns its literal on both          halves and the comparison is two constants agreeing",
    );

    let rays = probe_rays(&reframe);
    let answers = on_the_gpu(&device, &queue, &reframe, &rays);
    assert_eq!(answers.len(), rays.len());

    let (mut worst_weight, mut worst_pixel) = (0.0f32, 0.0f32);
    let (mut worst_depth, mut worst_axis) = (0.0f32, 0.0f32);
    let mut worst_exposure = 0.0f32;
    let (mut mixed, mut landings) = (0usize, 0usize);
    let split = reframe.exposure_split();
    for (ray, answer) in rays.iter().zip(&answers) {
        let mirror = reframe.blend(*ray);
        if mirror.weights.iter().all(|weight| *weight > 0.0) {
            mixed += 1;
        }
        // Checked on every ray rather than only where a lens draws: the split
        // reads one uniform and no ray at all, so a disagreement is a
        // disagreement everywhere, and the cheapest place to catch it is the
        // loop that is already running.
        for (drawn, mirrored) in answer.exposure.iter().zip(&split) {
            worst_exposure = worst_exposure.max((drawn - mirrored).abs());
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
    // A gain multiplies a code, so the bar is in codes: 1e-6 of a multiplier is
    // a four-thousandth of one code of 255 at full white, and the clean run
    // reads 0.0 exactly because both sides evaluate one `exp` of one uniform.
    assert!(
        worst_exposure < 1e-6,
        "the two halves normalize the exposure by multipliers {worst_exposure} apart",
    );
    eprintln!(
        "twin: {} rays, {mixed} inside the handover, {landings} landings compared; worst \
         weight {worst_weight:.3e}, worst landing {worst_pixel:.3e} px, worst depth \
         {worst_depth:.3e} px, worst axis {worst_axis:.3e}, worst exposure split \
         {worst_exposure:.3e}",
        rays.len(),
    );
}
