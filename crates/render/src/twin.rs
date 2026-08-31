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
//! **What it does not do.** The fragment half - the plane sampling, the colour
//! transform, the write to the target - is not covered: it needs decoded
//! planes, which means real footage, which means the null instrument. The
//! boundary this guards is the one the seam work keeps moving, which is every
//! function of the map: `blend`, `handover`, `crossover`, `claim`, `share`,
//! `within`, `axis_of`, `project`, `readout_share`, `turned`, `lens_pixel` and
//! **both of the models it dispatches to**, plus the layout of the uniform
//! block they all read.
//!
//! **Both models, because a model is only guarded at a fixture that selects
//! it.** `lens_pixel` branches on the block's own `model` field, so a camera
//! whose blocks say `MEI` never runs a line of `theta` on either half, and the
//! `.OSV` support could have shipped a WGSL `theta` that disagreed with its
//! Rust twin with every test in the workspace green. The comparison below is
//! therefore run twice over, on an Insta360 X4 Air's pair and on a DJI Osmo
//! 360's, and each arm asserts that the blocks it built name the model it
//! meant to test - which is the same lesson the rolling fixture below records,
//! learned once and applied before it could be learned again.
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

use crate::Blend;
use kjerag_meta::{Lens, Sweep};

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
    // **A fine ring, and it is here because the coarse sweep below misses most
    // of the belt.** Stepping azimuth by 15 degrees lands `floor(turn)` on 24
    // index pairs out of 128, so 80 of the belt's lanes were never compared
    // between the halves at all - a whole-lane offset slip that happened to
    // miss the probed indices passed. Seven degrees is coprime with the ring's
    // 2.8125-degree spacing, so this walks onto every entry.
    for phi in 0..(360 * 3) {
        let phi = phi as f32 / 3.0 * 7.0 % 360.0;
        push(90.0, phi);
        push(88.0, phi);
        push(92.0, phi);
    }
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
/// 2026-08-09 over these 5930 rays: every weight agrees to **4.3e-6**, every
/// landing to **7.3e-4 px**, every coverage depth to **8.5e-4 px** and every
/// landing axis to **1.2e-7**. The weight figure is the WORSE of the two
/// cameras - `theta` reads 4.232e-6 where `mei` reads 1.848e-6 - because a bar
/// quoted from the better arm is a bar that does not cover the run.
///
/// **Ray counts are deliberately not quoted here.** They were, twice, and went
/// stale twice: the width move to 6 degrees and each move of the 50/50 surface
/// changes them, and a number in a doc comment has no way to notice. The test
/// prints its own counts on every run and asserts floors on them, which is the
/// check those numbers were standing in for.
///
/// The bars are roughly ten times that, which
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
/// **The theta arm calibrated the same way**, 2026-08-09, and both of these
/// were caught by the DJI arm while the Insta360 arm stayed green to the last
/// digit above - which is the check that the two arms are independent and not
/// one fixture reported twice:
///
/// | mutation, WGSL `theta` only | worst weight | of the bar |
/// | --- | ---: | ---: |
/// | the tail coefficient read twice, `c5` written `c4` - the copy-paste a five-slot chain invites | 3.5e-1 | 17700x |
/// | the leading coefficient out by a **tenth of a percent**, `c1 * 1.001` | 4.3e-4 | 22x |
///
/// The second is the one worth reading: an error far too small to see in a
/// picture, on the term that does the most work, is still twenty times the
/// bar. Both were reverted.
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
struct MapArm {
    camera: &'static str,
    theta: bool,
    one_xs: bool,
    frame: Size,
    sweep: Sweep,
    lenses: Vec<Lens>,
}

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

    // One device, three map arms. The X4 Air and ONE X2 both select `mei` and
    // both read down their delivered frames, but only lens type 0x29 selects
    // the recovered ONE X2 mounting and alpha law. The DJI pair selects
    // `theta`; its readout is a synthetic horizontal control because no Osmo
    // readout has been measured. Keeping that arm horizontal makes both row
    // axis lanes live without mislabelling either Insta360 camera.
    for arm in [
        MapArm {
            camera: "insta360 x4 air (mei, down readout)",
            theta: false,
            one_xs: false,
            frame: crate::projection::tests::FRAME,
            sweep: Sweep::Down,
            lenses: crate::projection::tests::fixture_lenses(),
        },
        MapArm {
            camera: "insta360 one x2 (mei, OneXS alpha, down readout)",
            theta: false,
            one_xs: true,
            frame: crate::projection::tests::ONE_XS_FRAME,
            sweep: Sweep::Down,
            lenses: crate::projection::tests::one_xs_lenses(),
        },
        MapArm {
            camera: "dji osmo 360 (theta, horizontal readout control)",
            theta: true,
            one_xs: false,
            frame: crate::projection::tests::FRAME,
            sweep: Sweep::Right,
            lenses: crate::projection::tests::osmo_pair(),
        },
    ] {
        compare(&device, &queue, arm);
    }
}

/// One camera's worth of the comparison above: build the map, run it on both
/// halves, and hold them to the bars.
fn compare(device: &wgpu::Device, queue: &wgpu::Queue, arm: MapArm) {
    let MapArm {
        camera,
        theta,
        one_xs,
        frame,
        sweep,
        lenses,
    } = arm;
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
            axis: sweep.axis(),
        }),
        ..Held::default()
    };
    let reframe = Reframe::new(
        &lenses,
        frame,
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
    .with_shift(2.5f32.to_radians());
    // The block the shader is handed carries the held line, or this test would
    // be run on the one field the seam anchor added.
    assert!(reframe.handover_width() > 0.0, "{camera}");
    // And it carries the readout, or `project`'s second half is dead on both
    // halves and the boundary this file claims to guard is half a boundary.
    assert!(
        reframe.is_rolling(),
        "{camera}: the block's row axis is zero, so the readout branch of `project` runs on \
         neither side",
    );
    // A nonzero axis alone would let Down silently regress to Right again.
    // These two mid-edge shares distinguish the uniform's x and y lanes and
    // leave the horizontal control live beside both cameras' real Down arm.
    let axis = sweep.axis();
    assert_eq!(
        reframe.readout_share([frame.width as f32, frame.height as f32 / 2.0]),
        0.5 * axis[0] as f32,
        "{camera}: the readout's horizontal lane is not the selected sweep",
    );
    assert_eq!(
        reframe.readout_share([frame.width as f32 / 2.0, frame.height as f32]),
        0.5 * axis[1] as f32,
        "{camera}: the readout's vertical lane is not the selected sweep",
    );
    // And it says the model this arm exists to reach, or `lens_pixel` takes the
    // other branch on both halves and this arm is the other arm again.
    assert_eq!(
        crate::projection::tests::runs_theta(&reframe),
        theta,
        "{camera}: the blocks do not name the model this arm is meant to run",
    );
    assert_eq!(
        crate::projection::tests::runs_one_xs(&reframe),
        one_xs,
        "{camera}: the block does not select the base-alpha law this arm is meant to run",
    );
    if one_xs {
        assert_eq!(
            reframe.handover_shift(),
            0.0,
            "{camera}: ONE X2 retained a seam-anchor shift",
        );
    } else {
        assert!(
            reframe.handover_shift() > 0.0,
            "{camera}: the legacy arm did not carry its seam-anchor shift",
        );
    }

    let mut rays = probe_rays(&reframe);
    let target = one_xs.then(|| {
        rays.push(reframe.view_ray_from_body(crate::projection::tests::ONE_XS_TARGET_BODY_RAY));
        rays.len() - 1
    });
    let compare = |arm: &str, answers: &[Answer], mirror_of: &dyn Fn(&[f32; 3]) -> Blend| {
        assert_eq!(answers.len(), rays.len());
        let (mut worst_weight, mut worst_pixel) = (0.0f32, 0.0f32);
        let (mut worst_depth, mut worst_axis) = (0.0f32, 0.0f32);
        let (mut mixed, mut landings) = (0usize, 0usize);
        for (ray, answer) in rays.iter().zip(answers) {
            let mirror = mirror_of(ray);
            if mirror.weights.iter().all(|weight| *weight > 0.0) {
                mixed += 1;
            }
            for lens in 0..MAX_LENSES {
                // The weight is compared everywhere, and it is the number that
                // carries `inside`: `claim` answers zero for a landing that is
                // outside, so a disagreement about coverage shows up here as a
                // disagreement about the weight.
                worst_weight =
                    worst_weight.max((answer.weights[lens] - mirror.weights[lens]).abs());
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
                    "{camera}, {arm}: lens {lens} carries weight {} on the mirror and is \
                     outside on the shader at {ray:?}",
                    mirror.weights[lens],
                );
                for axis in 0..2 {
                    worst_pixel = worst_pixel
                        .max((answer.pixel[lens][axis] - mirror.landings[lens].pixel[axis]).abs());
                }
                worst_depth =
                    worst_depth.max((answer.depth[lens] - mirror.landings[lens].depth).abs());
                worst_axis = worst_axis.max((answer.axis[lens] - mirror.landings[lens].axis).abs());
            }
        }
        // The rays have to actually cross the seam, or the whole comparison is
        // about the far field where one lens takes everything at a weight of
        // one.
        assert!(
            mixed > 500,
            "{camera}, {arm}: only {mixed} of {} probes are inside the handover, so this \
             compares the far field",
            rays.len(),
        );
        assert!(
            landings > 4000,
            "{camera}, {arm}: only {landings} landings reach a pixel, so this compares almost \
             nothing",
        );
        assert!(
            worst_weight < 2e-5,
            "{camera}, {arm}: the two halves disagree by {worst_weight} of a whole weight",
        );
        assert!(
            worst_pixel < 1e-2,
            "{camera}, {arm}: the two halves land {worst_pixel} px apart",
        );
        assert!(
            worst_depth < 1e-2,
            "{camera}, {arm}: the two halves read coverage depths {worst_depth} px apart",
        );
        assert!(
            worst_axis < 1e-6,
            "{camera}, {arm}: the two halves read the landing's own axis cosine {worst_axis} \
             apart",
        );
        eprintln!(
            "twin: {camera}, {arm}: {} rays, {mixed} inside the handover, {landings} landings \
             compared; worst weight {worst_weight:.3e}, worst landing {worst_pixel:.3e} px, \
             worst depth {worst_depth:.3e} px, worst axis {worst_axis:.3e}",
            rays.len(),
        );
    };

    let answers = on_the_gpu(device, queue, &reframe, &rays);
    compare("plain blend", &answers, &|ray| reframe.blend(*ray));
    if let Some(target) = target {
        assert_eq!(
            reframe.blend(rays[target]).weights[0],
            1.0,
            "{camera}: the Rust map does not give the reported mount/riser ray to lens 0",
        );
        assert_eq!(
            answers[target].weights[0], 1.0,
            "{camera}: the shader does not give the reported mount/riser ray to lens 0",
        );
    }
}

/// The band's own shader compiles, including the chromatic solve.
///
/// Nothing else in the suite reaches it: the measure and pool pipelines are
/// built inside `Band::new`, which needs a device and so only runs when the
/// app does. A WGSL error in there is therefore a panic on launch rather than
/// a red test, which is the worst place to find one. This compiles the module
/// and creates every entry point the pass dispatches, which is where naga
/// checks the bodies rather than just the syntax.
#[test]
fn the_bands_shader_compiles_every_entry_the_pass_dispatches() {
    let (device, _queue, name) = match gpu() {
        Ok(open) => open,
        Err(why) => {
            assert!(
                std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}",
            );
            eprintln!("band shader: skipped, no GPU on this box ({why})");
            return;
        }
    };
    eprintln!("band shader: {name}");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("band twin"),
        // The same two halves the pass concatenates: the band's shader is
        // written against the map's, and neither compiles alone.
        source: wgpu::ShaderSource::Wgsl(
            format!("{}\n{}", crate::projection::wgsl(), crate::band::wgsl()).into(),
        ),
    });
    // The draw's own module too, assembled the way the pipeline assembles it —
    // the plain (flow-off) variant. Nothing else compiles this one either, and
    // the fragment is where the chromatic lookup lands.
    let _ = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scene twin"),
        source: wgpu::ShaderSource::Wgsl(crate::scene::draw_wgsl_flow(false).into()),
    });
    // The flow-on draw too (chunk 4, §37): the fragment recompute and the
    // appended `flow_shift`/flow buffer. The pipeline now builds BOTH variants
    // and the runtime toggle picks between them, so both are on a launch path;
    // compiling the on variant here makes a WGSL error in the flow apply a red
    // test rather than a crash the first time the toggle is flipped.
    let _ = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scene flow twin"),
        source: wgpu::ShaderSource::Wgsl(crate::scene::draw_wgsl_flow(true).into()),
    });
    // The detached V6 oracle's selected ONE X2 retained-field variant is a
    // third launch path. It deliberately has a different coordinate law and
    // field shape from the legacy flow shader, so compiling one says nothing
    // about the other.
    let _ = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scene ONE X2 flow twin"),
        source: wgpu::ShaderSource::Wgsl(crate::scene::draw_wgsl_one_xs_flow().into()),
    });
    for entry in ["measure", "pool", "pool_along", "strip"] {
        let _ = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: None,
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        });
    }

    // And the chromatic GRID's three passes, on the same two halves plus its
    // own: it is written against the band's `look`, `frame_uv` and plane
    // bindings, and against the map's `Reframe`, so it compiles with both or
    // with neither.
    //
    // This arm has already shipped a WGSL error to the owner once, as a crash
    // on launch, for exactly the reason the doc above gives: the pipeline is
    // built inside a constructor that only runs when the app does. Every entry
    // point the pass dispatches is created here, because that is where naga
    // checks the bodies rather than only the syntax.
    let grid = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("chroma twin"),
        source: wgpu::ShaderSource::Wgsl(
            format!(
                "{}\n{}\n{}",
                crate::projection::wgsl(),
                crate::band::wgsl(),
                crate::chroma::wgsl(),
            )
            .into(),
        ),
    });
    for entry in [
        "chroma_gate",
        "chroma_read",
        "chroma_blur",
        "chroma_admit",
        "chroma_solve",
        "chroma_finish",
    ] {
        let _ = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: None,
            module: &grid,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        });
    }
    // A validation failure raises on the spot rather than being collected:
    // wgpu's uncaptured handler panics, which is how the twin above surfaced
    // the last WGSL error in this crate.
}

/// One flow probe's two displaced rays, final blend weights, and lens-0 common
/// flow alpha.
struct FlowAnswer {
    shifted: [[f32; 3]; MAX_LENSES],
    weights: [f32; MAX_LENSES],
    alpha_a: f32,
}

/// The compute probe for the flow apply: `flow_shift` on both lenses plus the
/// final `blend_flow` weights. Group 1 holds the probes, the answers, and the
/// flow buffer on binding 3 — the same binding the draw's read group gives it.
const FLOW_PROBE: &str = r#"
@group(1) @binding(0) var<storage, read> fprobes: array<vec4<f32>>;
@group(1) @binding(1) var<storage, read_write> fanswers: array<vec4<f32>>;

@compute @workgroup_size(64)
fn flow_twin(@builtin(global_invocation_id) id: vec3<u32>) {
  let index = id.x;
  if index >= arrayLength(&fprobes) {
    return;
  }
  let ray = fprobes[index].xyz;
  // The per-lens share Studio splits the flow by — the WIDE coverage gate
  // (§42.1/§42.3), lens 0 by alpha_A, lens 1 by 1 - alpha_A. Driving the probe's
  // alpha from `gate_alpha` twins it too (a mismatch moves the displaced ray).
  // Rust twin: `Reframe::gate_alpha`.
  let alpha_a = gate_alpha(ray);
  let s0 = normalize(flow_shift(0u, ray, alpha_a));
  let s1 = normalize(flow_shift(1u, ray, 1.0 - alpha_a));
  let mixed = blend_flow(ray);
  fanswers[index * 3u + 0u] = vec4<f32>(s0, 0.0);
  fanswers[index * 3u + 1u] = vec4<f32>(s1, 0.0);
  fanswers[index * 3u + 2u] = vec4<f32>(mixed.weights[0], mixed.weights[1], alpha_a, 0.0);
}
"#;

/// **The flow apply's shader and its Rust twin displace and weigh the same rays**
/// (chunk 4, §37).
///
/// `Reframe::flow_shift` exists twice, like everything else the seam is made
/// of: once as Rust the tests reach and once as the WGSL the draw runs
/// ([`crate::scene::flow_wgsl`]). This compiles the shipped `flow_shift` with a
/// probe that calls it on both lenses, uploads a composed displacement field,
/// and asks [`Reframe::flow_shift`] and [`Reframe::blend_flow`] the same rays
/// on the CPU. A change to one side and not the other fails here, with no
/// footage.
#[test]
fn the_flow_apply_and_its_rust_twin_displace_the_same_rays() {
    let (device, queue, name) = match gpu() {
        Ok(open) => open,
        Err(why) => {
            assert!(
                std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}",
            );
            eprintln!("flow twin: skipped, no GPU on this box ({why})");
            return;
        }
    };
    eprintln!("flow twin: {name}");

    let reframe = Reframe::new(
        &crate::projection::tests::fixture_lenses(),
        Size {
            width: 3840,
            height: 3840,
        },
        Camera {
            yaw: 74f32.to_radians(),
            pitch: -31f32.to_radians(),
            fov: 55f32.to_radians(),
        },
        crate::projection::Held::default(),
        16.0 / 9.0,
        true,
        Sampling::default(),
    );

    // Two composed fields with DISTINCT structure across the belt (§38: the
    // apply samples lens 0 from r2l and lens 1 from l2r, two separate fields),
    // so a lens reading its wrong plane and a ray that lands on zero are both
    // exercised: a colatitude disparity that swings a few rows with azimuth.
    let l2r = flow_field(|col, row| {
        6.0 * (col as f32 / crate::band::STRIP_W as f32 * std::f32::consts::TAU).sin()
            + 3.0 * (row as f32 / crate::band::STRIP_H as f32 - 0.5)
    });
    let r2l = flow_field(|col, row| {
        -5.0 * (col as f32 / crate::band::STRIP_W as f32 * std::f32::consts::TAU).cos()
            + 2.5 * (row as f32 / crate::band::STRIP_H as f32 - 0.5)
    });
    let disp = crate::flow::compose::Displacement::compose(&l2r, &r2l);

    let rays = probe_rays(&reframe);
    let answers = flow_on_the_gpu(&device, &queue, &reframe, &disp, &rays);

    // Two boundaries the comparison steps around, both measure-zero and both
    // genuine discontinuities of the apply the two `atan2`/`acos` round across:
    //
    // 1. The GATE: the apply keeps the base ray outside it (`!(0 < alpha < 1)`),
    //    so a ray within a rounding of the edge is displaced on one half and
    //    kept on the other and disagrees by the whole displacement.
    // 2. The belt's LONGITUDE CUT (phi ≈ 0/2π, the back of the camera): the
    //    sample coordinate CLAMPS at the belt's first/last column (§40), and at
    //    the cut Rust's and the shader's `atan2` round `phi` to opposite signs,
    //    landing the clamp on opposite edges — a real seam of Studio's own
    //    clamp, far from the near content. A few columns either side are skipped.
    let tau = std::f32::consts::TAU;
    let cut_margin = 3.0 / crate::band::STRIP_W as f32 * tau; // ~3 belt columns
    let mut worst = 0.0f32;
    let mut worst_weight = 0.0f32;
    let mut worst_weight_at = ([0.0; 3], [0.0; 2], [0.0; 2], [0.0; 2]);
    let mut weighed = 0usize;
    let mut moved = 0usize;
    for (ray, answer) in rays.iter().zip(&answers) {
        let alpha_a = reframe.gate_alpha(*ray);
        let body = reframe.body_ray(*ray);
        let phi_mod = body[1].atan2(body[0]).rem_euclid(tau);
        if phi_mod < cut_margin || phi_mod > tau - cut_margin {
            continue;
        }
        if !(alpha_a > 1e-3 && alpha_a < 1.0 - 1e-3) {
            continue;
        }
        let mixed = reframe.blend_flow(*ray, &disp);
        // At the exact fisheye validity rim, a subpixel Rust/WGSL landing
        // difference is intentionally magnified by `claim` from zero coverage
        // depth. The main map twin owns that discontinuity. Compare flow's
        // final weights where both displaced samples have a stable interior.
        if mixed
            .landings
            .iter()
            .all(|landing| landing.inside && landing.depth > 2.0)
        {
            weighed += 1;
            for lens in 0..MAX_LENSES {
                let delta = (answer.weights[lens] - mixed.weights[lens]).abs();
                if delta > worst_weight {
                    worst_weight = delta;
                    worst_weight_at = (
                        *ray,
                        answer.weights,
                        mixed.weights,
                        [mixed.landings[0].depth, mixed.landings[1].depth],
                    );
                }
            }
        }
        for (lens, shader) in answer.shifted.iter().enumerate() {
            let alpha = if lens == 0 { alpha_a } else { 1.0 - alpha_a };
            let shifted = reframe.flow_shift(lens, *ray, &disp, alpha);
            let n = (shifted[0] * shifted[0] + shifted[1] * shifted[1] + shifted[2] * shifted[2])
                .sqrt();
            let mirror = [shifted[0] / n, shifted[1] / n, shifted[2] / n];
            if mirror != normalize(*ray) {
                moved += 1;
            }
            for axis in 0..3 {
                worst = worst.max((shader[axis] - mirror[axis]).abs());
            }
        }
    }
    // The rays actually have to be displaced, or this compares the identity map
    // on both halves.
    assert!(
        moved > 500,
        "flow twin: only {moved} of {} lens-rays were displaced, so this compares the far field",
        rays.len() * MAX_LENSES,
    );
    assert!(
        worst < 1e-3,
        "flow twin: the two halves displace a ray {worst} apart in direction",
    );
    assert!(
        weighed > 500,
        "flow twin: only {weighed} rays had stable displaced landings for the colour-weight twin",
    );
    // The flow-shift twin above admits 1e-3 of direction because Rust and WGSL
    // take different trig implementations. In the stable interior that becomes
    // at most 1.36e-4 of a normalized coverage-weight on this measured probe.
    assert!(
        worst_weight < 2e-4,
        "flow twin: the two halves disagree by {worst_weight} of a final colour weight at \
         {worst_weight_at:?}",
    );

    // Two fixed ONE X2 points guard both selected alpha resources. The reported
    // mount has a captured common flow alpha of 0.547828251 but a saturated
    // final Template alpha of one. The second has common alpha 0.479868661 and
    // final Template alpha 75/25; legacy `claim` would bias the latter to about
    // 80/20. The structured nonzero field makes the common alpha affect both
    // displaced rays, so this cannot pass through zero-displacement
    // cancellation. The explicit GPU alpha lane names a 16-vs-32-degree gate
    // mismatch directly.
    let one_xs = Reframe::new(
        &crate::projection::tests::one_xs_lenses(),
        crate::projection::tests::ONE_XS_FRAME,
        Camera::default(),
        Held::default(),
        1.0,
        false,
        Sampling::default(),
    );
    let cases = [
        (
            crate::projection::tests::ONE_XS_TARGET_BODY_RAY,
            0.547_828_26,
            [1.0, 0.0],
            "reported mount",
        ),
        (
            crate::projection::tests::ONE_XS_FLOW_BLEND_BODY_RAY,
            0.479_868_65,
            [0.75, 0.25],
            "unsaturated 75/25",
        ),
    ];
    let targets: Vec<[f32; 3]> = cases
        .iter()
        .map(|(body, _, _, _)| one_xs.view_ray_from_body(*body))
        .collect();
    let target_answers = flow_on_the_gpu(&device, &queue, &one_xs, &disp, &targets);
    for (((_, expected_alpha, expected, label), target), answer) in
        cases.iter().zip(&targets).zip(&target_answers)
    {
        let alpha_a = one_xs.gate_alpha(*target);
        assert!(
            (alpha_a - expected_alpha).abs() < 2e-5,
            "flow Rust twin selected the wrong ONE X2 common alpha at {label}: {alpha_a}",
        );
        assert!(
            (answer.alpha_a - expected_alpha).abs() < 2e-5,
            "flow shader selected the wrong ONE X2 common alpha at {label}: {}",
            answer.alpha_a,
        );
        let mirror = one_xs.blend_flow(*target, &disp);
        for (lens, expected_weight) in expected.iter().enumerate().take(MAX_LENSES) {
            let flow_alpha = if lens == 0 { alpha_a } else { 1.0 - alpha_a };
            let shifted = normalize(one_xs.flow_shift(lens, *target, &disp, flow_alpha));
            for axis in 0..3 {
                assert!(
                    (answer.shifted[lens][axis] - shifted[axis]).abs() < 1e-3,
                    "flow shader used the wrong ONE X2 common alpha at {label}, lens {lens}: \
                     {:?} versus {shifted:?}",
                    answer.shifted[lens],
                );
            }
            assert!(
                (mirror.weights[lens] - expected_weight).abs() < 2e-5,
                "flow Rust twin bypassed the ONE X2 final alpha at {label}: {:?}",
                mirror.weights,
            );
            assert!(
                (answer.weights[lens] - expected_weight).abs() < 2e-5,
                "flow shader bypassed the ONE X2 final alpha at {label}: {:?}",
                answer.weights,
            );
        }
        assert!(
            answer.shifted.iter().any(|shifted| {
                shifted
                    .iter()
                    .zip(normalize(*target))
                    .any(|(actual, base)| (actual - base).abs() > 1e-5)
            }),
            "flow gate discriminator did not displace either lens at {label}",
        );
    }

    eprintln!(
        "flow twin: {moved} lens-rays displaced; {weighed} stable weights; worst direction \
         {worst:.3e}; worst weight {worst_weight:.3e}",
    );
}

/// The selected ONE X2 shader reads the 1080-by-60 retained layout through a
/// different coordinate law from the legacy pass. This keeps that new draw
/// path pinned to the readable Rust apply before the estimator is wired.
#[test]
fn the_one_xs_flow_apply_and_its_rust_twin_displace_the_same_rays() {
    use crate::flow::dis::FlowField;
    use crate::flow::one_xs::{AtoBField, BtoAField, COLS, DirectedFields, Layout, ROWS, Sample};

    let (device, queue, name) = match gpu() {
        Ok(open) => open,
        Err(why) => {
            assert!(
                std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}",
            );
            eprintln!("ONE X2 flow twin: skipped, no GPU on this box ({why})");
            return;
        }
    };
    eprintln!("ONE X2 flow twin: {name}");

    let reframe = Reframe::new(
        &crate::projection::tests::one_xs_lenses(),
        crate::projection::tests::ONE_XS_FRAME,
        Camera {
            yaw: 29f32.to_radians(),
            pitch: -17f32.to_radians(),
            fov: 74f32.to_radians(),
        },
        Held::default(),
        16.0 / 9.0,
        false,
        Sampling::default(),
    );

    let field = |a_to_b: bool| {
        let mut dcol = Vec::with_capacity(ROWS * COLS);
        let mut drow = Vec::with_capacity(ROWS * COLS);
        for row in 0..ROWS {
            for col in 0..COLS {
                let around = row as f32 / (ROWS - 1) as f32 * std::f32::consts::TAU;
                let across = col as f32 / (COLS - 1) as f32 - 0.5;
                if a_to_b {
                    dcol.push(0.8 + 0.6 * around.sin() + 0.3 * across);
                    drow.push(-1.2 + 0.7 * around.cos() - 0.2 * across);
                } else {
                    dcol.push(-0.5 + 0.4 * around.cos() - 0.25 * across);
                    drow.push(0.9 + 0.5 * around.sin() + 0.35 * across);
                }
            }
        }
        FlowField {
            width: COLS,
            height: ROWS,
            u: dcol,
            v: drow,
            valid: vec![true; ROWS * COLS],
            residual: vec![0.0; ROWS * COLS],
        }
    };
    let directed = DirectedFields::new(
        AtoBField::from_solver(field(true)).unwrap(),
        BtoAField::from_solver(field(false)).unwrap(),
    );
    let displacement = crate::flow::one_xs::Displacement::compose(&directed);

    // Stay inside the captured 16-degree gate while spanning the full seam
    // circle and non-integer retained coordinates on both grid axes.
    let columns = [12.25, 20.5, 28.75, 36.5, 46.75];
    let rays: Vec<[f32; 3]> = (20..ROWS - 20)
        .step_by(7)
        .flat_map(|row| {
            columns.into_iter().map(move |col| {
                let sample = Sample::new(row as f32 + 0.375, col).unwrap();
                reframe.view_ray_from_body(Layout.body_ray(sample).components())
            })
        })
        .collect();
    let answers = flow_bytes_on_the_gpu(
        &device,
        &queue,
        &reframe,
        displacement.bytes(),
        crate::scene::one_xs_flow_wgsl(),
        &rays,
    );

    let mut moved = 0usize;
    let mut weighed = 0usize;
    let mut worst_direction = 0.0f32;
    let mut worst_weight = 0.0f32;
    let mut worst_alpha = 0.0f32;
    for (ray, answer) in rays.iter().zip(&answers) {
        let alpha_a = reframe.gate_alpha(*ray);
        worst_alpha = worst_alpha.max((answer.alpha_a - alpha_a).abs());
        let mixed = reframe.blend_one_xs_flow(*ray, &displacement);
        if mixed.landings.iter().all(|landing| landing.inside) {
            weighed += 1;
            for lens in 0..MAX_LENSES {
                worst_weight = worst_weight.max((answer.weights[lens] - mixed.weights[lens]).abs());
            }
        }
        for lens in 0..MAX_LENSES {
            let alpha_lens = if lens == 0 { alpha_a } else { 1.0 - alpha_a };
            let shifted =
                normalize(reframe.one_xs_flow_shift(lens, *ray, &displacement, alpha_lens));
            if shifted != normalize(*ray) {
                moved += 1;
            }
            for (&gpu, rust) in answer.shifted[lens].iter().zip(shifted) {
                worst_direction = worst_direction.max((gpu - rust).abs());
            }
        }
    }

    assert!(
        moved > rays.len(),
        "ONE X2 flow twin displaced only {moved} of {} lens-rays",
        rays.len() * MAX_LENSES,
    );
    assert!(
        weighed > rays.len() / 2,
        "ONE X2 flow twin compared only {weighed} stable colour weights",
    );
    assert!(
        worst_direction < 1e-3,
        "ONE X2 flow twins differ by {worst_direction} in direction",
    );
    assert!(
        worst_weight < 2e-4,
        "ONE X2 flow twins differ by {worst_weight} in final colour weight",
    );
    // These probes are constructed by a retained-grid inverse and then pass
    // through a turned f32 body/view matrix before both sides evaluate acos.
    // Over the full 400-degree sweep the measured Rust/WGSL trig difference is
    // 2.13e-4 of alpha; the fixed native discriminators remain guarded at 2e-5
    // by the legacy flow twin above.
    assert!(
        worst_alpha < 3e-4,
        "ONE X2 flow twins differ by {worst_alpha} in common alpha",
    );
    eprintln!(
        "ONE X2 flow twin: {moved} lens-rays displaced; {weighed} stable weights; worst direction \
         {worst_direction:.3e}; weight {worst_weight:.3e}; alpha {worst_alpha:.3e}",
    );
}

/// A `FlowField` over the belt whose `v` is `of(col, row)` and whose `u` is a
/// DISTINCT along-seam structure, valid everywhere — so the twin exercises both
/// planes (the apply displaces the sample coordinate by BOTH components, §40).
fn flow_field(of: impl Fn(usize, usize) -> f32) -> crate::flow::dis::FlowField {
    let (w, h) = (crate::band::STRIP_W, crate::band::STRIP_H);
    let v = (0..w * h).map(|i| of(i % w, i / w)).collect();
    let u = (0..w * h)
        .map(|i| 4.0 * ((i % w) as f32 / w as f32 * std::f32::consts::TAU).cos())
        .collect();
    crate::flow::dis::FlowField {
        width: w,
        height: h,
        u,
        v,
        valid: vec![true; w * h],
        residual: vec![f32::INFINITY; w * h],
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / n, v[1] / n, v[2] / n]
}

/// Runs the flow probe on the GPU and reads the two displaced rays, final colour
/// weights, and common lens-0 flow alpha per input back. The flow buffer binds
/// on group 1 binding 3, the draw's own.
fn flow_on_the_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    reframe: &Reframe,
    disp: &crate::flow::compose::Displacement,
    rays: &[[f32; 3]],
) -> Vec<FlowAnswer> {
    flow_bytes_on_the_gpu(
        device,
        queue,
        reframe,
        disp.bytes(),
        crate::scene::flow_wgsl(),
        rays,
    )
}

/// Run either typed flow shader against the bytes belonging to that exact
/// displacement contract. Both layouts fit the draw's legacy-sized storage
/// allocation, but only the supplied shader is allowed to interpret them.
fn flow_bytes_on_the_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    reframe: &Reframe,
    displacement: &[u8],
    flow_wgsl: String,
    rays: &[[f32; 3]],
) -> Vec<FlowAnswer> {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("flow twin"),
        source: wgpu::ShaderSource::Wgsl(
            format!("{}\n{}\n{FLOW_PROBE}", crate::projection::wgsl(), flow_wgsl,).into(),
        ),
    });
    let block = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("flow twin block"),
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
    let group1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("flow twin group 1"),
        entries: &[
            storage(0, true),
            storage(1, false),
            storage(crate::scene::FLOW_BINDING, true),
        ],
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("flow twin"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("flow twin"),
                bind_group_layouts: &[&block, &group1],
                immediate_size: 0,
            }),
        ),
        module: &module,
        entry_point: Some("flow_twin"),
        compilation_options: Default::default(),
        cache: None,
    });

    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("flow twin block"),
        size: std::mem::size_of::<Reframe>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&uniform, 0, reframe.bytes());

    let padded: Vec<[f32; 4]> = rays.iter().map(|r| [r[0], r[1], r[2], 0.0]).collect();
    let input = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("flow twin rays"),
        size: bytes_of(&padded).len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&input, 0, bytes_of(&padded));

    let flow = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("flow twin field"),
        size: crate::band::FLOW_BYTES,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    assert!(displacement.len() as u64 <= crate::band::FLOW_BYTES);
    queue.write_buffer(&flow, 0, displacement);

    let answers_bytes = (rays.len() * 3 * 16) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("flow twin answers"),
        size: answers_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("flow twin readback"),
        size: answers_bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let block_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("flow twin block"),
        layout: &block,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    let probe_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("flow twin group 1"),
        layout: &group1,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: crate::scene::FLOW_BINDING,
                resource: flow.as_entire_binding(),
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
        .expect("the flow probe pass did not finish");

    let mapped = readback.slice(..).get_mapped_range();
    let floats: Vec<f32> = mapped
        .chunks_exact(4)
        .map(|word| f32::from_le_bytes([word[0], word[1], word[2], word[3]]))
        .collect();
    drop(mapped);
    readback.unmap();

    (0..rays.len())
        .map(|probe| {
            let base = probe * 3 * 4;
            FlowAnswer {
                shifted: [
                    [floats[base], floats[base + 1], floats[base + 2]],
                    [floats[base + 4], floats[base + 5], floats[base + 6]],
                ],
                weights: [floats[base + 8], floats[base + 9]],
                alpha_a: floats[base + 10],
            }
        })
        .collect()
}
