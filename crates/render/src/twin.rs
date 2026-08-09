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

use kjerag_meta::Lens;

use crate::projection::{Held, MAX_LENSES, Reframe, Rolling};
use crate::sampling::Sampling;
use crate::{Camera, Size, dmabuf};

/// Six `vec4`s of answer per probe, which is how the storage buffer is read
/// back. A struct would need WGSL's own alignment rules agreed on twice; a
/// lane of four floats needs none.
///
/// Four were the map's; the fifth and sixth are the belt's - where each lens
/// is actually sampled once the flow has moved it, and what the field read
/// answered for this ray.
const LANES: usize = 6;

/// The compute half of this file: the probe entry the shipped map is
/// concatenated in front of.
///
/// Its bindings are group 1, so group 0 is exactly what
/// `super::projection::wgsl` declares and the uniform is bound the way the
/// draw binds it.
const PROBE: &str = r#"
@group(1) @binding(0) var<storage, read> probes: array<vec4<f32>>;
@group(1) @binding(1) var<storage, read_write> answers: array<vec4<f32>>;
// One planted flow per probe ray, in strip pixels, with `z` saying whether
// this ray is to be treated as on the strip.
//
// **Planted rather than read, and that is what makes the belt's arithmetic
// comparable at all.** `blend` reads its flow out of a 4096x128 texture; the
// Rust mirror has no texture and a CPU-side copy of one would be a second
// thing to keep in step. So the guard splits the question in two: `blended`
// is asked about a flow both halves are simply HANDED, and `belt_look` - the
// texture read itself - is asked separately against a field planted at values
// f16 holds exactly, so that only the bilinear arithmetic is under test.
@group(1) @binding(2) var<storage, read> flows: array<vec4<f32>>;

@compute @workgroup_size(64)
fn twin(@builtin(global_invocation_id) id: vec3<u32>) {
  let index = id.x;
  if index >= arrayLength(&probes) {
    return;
  }
  let ray = probes[index].xyz;
  let plant = flows[index];
  let out = blended(ray, plant.xy, plant.z > 0.5);
  let base = index * 6u;
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
  // Where each lens is SAMPLED, which is the belt's whole consuming path:
  // `belt_seat`, `belt_side`, `belt_gain`, `belt_ray` and the second
  // projection through them.
  answers[base + 4u] = vec4<f32>(out.moved[0], out.moved[1]);
  // And the field read itself, against the planted texture.
  let look = belt_look(ray);
  answers[base + 5u] = vec4<f32>(look.x, look.y, look.z, 0.0);
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
    /// Where each lens is sampled once the belt has moved it.
    moved: [[f32; 2]; MAX_LENSES],
    /// What the field read answered: the flow in strip pixels, and whether the
    /// ray is on the strip.
    look: ([f32; 2], bool),
}

/// Runs the shipped map on the GPU over `rays` and reads every landing and
/// weight back.
fn on_the_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    reframe: &Reframe,
    rays: &[[f32; 3]],
    plants: &[([f32; 2], bool)],
    field: &[[f32; 2]],
) -> Vec<Answer> {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("twin"),
        source: wgpu::ShaderSource::Wgsl(format!("{}\n{PROBE}", crate::projection::wgsl()).into()),
    });
    // Group 0 is exactly what the draw binds, which now includes the belt's
    // field at binding 6: `blend` reaches it through `belt_look`, and a
    // pipeline whose layout is missing a binding its entry point reaches is
    // refused outright (which is how this guard found out it had to grow).
    let block = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("twin block"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Reframe>() as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: crate::scene::FIELD_BINDING,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
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
        entries: &[storage(0, true), storage(1, false), storage(2, true)],
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

    // The belt's field, uploaded as the shipped format holds it. Every value
    // planted is a multiple of 1/64 no larger than 16, which `Rg16Float` keeps
    // exactly, so both halves read the same numbers and what is compared is
    // the bilinear arithmetic over them and not a rounding.
    let texels: Vec<[u16; 2]> = field
        .iter()
        .map(|flow| [half(flow[0]), half(flow[1])])
        .collect();
    let belt = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("twin field"),
        size: wgpu::Extent3d {
            width: crate::belt::COLUMNS,
            height: crate::belt::ROWS,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg16Float,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &belt,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        // `[u16; 2]` has no padding and no invalid pattern.
        unsafe {
            std::slice::from_raw_parts(
                texels.as_ptr().cast::<u8>(),
                std::mem::size_of_val(&texels[..]),
            )
        },
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(crate::belt::COLUMNS * 4),
            rows_per_image: Some(crate::belt::ROWS),
        },
        wgpu::Extent3d {
            width: crate::belt::COLUMNS,
            height: crate::belt::ROWS,
            depth_or_array_layers: 1,
        },
    );
    let belt_view = belt.create_view(&Default::default());

    let planted: Vec<[f32; 4]> = plants
        .iter()
        .map(|(flow, on)| [flow[0], flow[1], f32::from(u8::from(*on)), 0.0])
        .collect();
    let plant_bytes = bytes_of(&planted);
    let plant_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("twin plants"),
        size: plant_bytes.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&plant_buffer, 0, plant_bytes);

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
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: crate::scene::FIELD_BINDING,
                resource: wgpu::BindingResource::TextureView(&belt_view),
            },
        ],
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
            wgpu::BindGroupEntry {
                binding: 2,
                resource: plant_buffer.as_entire_binding(),
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
                moved: [[lane[16], lane[17]], [lane[18], lane[19]]],
                look: ([lane[20], lane[21]], lane[22] != 0.0),
            }
        })
        .collect()
}

/// `f32` to the `Rg16Float` the field is held in.
///
/// Only ever handed a multiple of 1/64 no larger than 16, which is exactly
/// representable, so there is no rounding to get right - and
/// [`the_planted_field_survives_the_format`] is what says the plant stays
/// inside that.
fn half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    if value == 0.0 {
        return sign;
    }
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mantissa = ((bits & 0x007f_ffff) >> 13) as u16;
    sign | ((exponent as u16) << 10) | mantissa
}

/// The same the other way, so a test can prove the round trip rather than
/// assume it.
fn whole(bits: u16) -> f32 {
    let sign = f32::from_bits(u32::from(bits & 0x8000) << 16);
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = u32::from(bits & 0x03ff) << 13;
    if exponent == 0 && mantissa == 0 {
        return sign;
    }
    let out = f32::from_bits((((exponent - 15 + 127) as u32) << 23) | mantissa);
    match bits & 0x8000 {
        0 => out,
        _ => -out,
    }
}

fn bytes_of(rows: &[[f32; 4]]) -> &[u8] {
    // `[f32; 4]` has no padding and no invalid pattern, and this is the same
    // cast `Reframe::bytes` makes for the block itself.
    unsafe { std::slice::from_raw_parts(rows.as_ptr().cast::<u8>(), std::mem::size_of_val(rows)) }
}

/// The field the belt's own read is compared over.
///
/// **Every value is a multiple of 1/64 no larger than 16**, which `Rg16Float`
/// holds exactly, so the two halves read the same numbers and what is under
/// test is the bilinear arithmetic and not a rounding. It varies in both axes
/// and at both scales, so a read that fetched the right texel and mixed it the
/// wrong way, or wrapped the wrong axis, is a disagreement rather than a
/// coincidence.
fn planted_field() -> Vec<[f32; 2]> {
    let columns = crate::belt::COLUMNS as usize;
    let rows = crate::belt::ROWS as usize;
    let quantized = |value: f32| (value * 64.0).round() / 64.0;
    (0..columns * rows)
        .map(|index| {
            let x = (index % columns) as f32 / columns as f32;
            let y = (index / columns) as f32 / rows as f32;
            [
                quantized(6.0 * (x * std::f32::consts::TAU * 5.0).sin() + 2.0 * y),
                quantized(-4.0 * (x * std::f32::consts::TAU * 3.0).cos() + 3.0 * (y - 0.5)),
            ]
        })
        .collect()
}

/// One planted flow per probe ray, in strip pixels.
///
/// Handed to `blended` on both halves rather than read out of the field, so
/// that the displacement arithmetic is compared against a number and not
/// against a second lookup. Every third ray is planted "off the strip", so the
/// branch that leaves a sample where the geometry put it is compared too.
fn planted_flow(index: usize) -> ([f32; 2], bool) {
    let turn = index as f32 * 0.37;
    (
        [7.5 * turn.sin(), 4.5 * (turn * 1.7).cos()],
        !index.is_multiple_of(3),
    )
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

    // One device, two cameras. The Insta360 pair selects `mei` inside
    // `lens_pixel` and the DJI pair selects `theta`; everything else about the
    // fixture, the rays and the bars is held identical, so an arm that fails
    // names a model rather than a setup.
    for (camera, theta, lenses) in [
        (
            "insta360 (mei)",
            false,
            crate::projection::tests::fixture_lenses(),
        ),
        (
            "dji osmo 360 (theta)",
            true,
            crate::projection::tests::osmo_pair(),
        ),
    ] {
        compare(&device, &queue, camera, theta, lenses);
    }
}

/// One camera's worth of the comparison above: build the map, run it on both
/// halves, and hold them to the bars.
fn compare(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    camera: &str,
    theta: bool,
    lenses: Vec<Lens>,
) {
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
    .with_shift(2.5f32.to_radians());
    // **And it carries the belt, or the whole consuming half is dead on both
    // halves.** This file has recorded that lesson twice - a fixture that did
    // not roll took the readout out of the comparison, and a fixture that
    // named one model took the other out - and the belt is the third place it
    // applies: `blend` reads `reframe.belt` before it does anything, so a
    // block that says zero compares a map with no belt in it and passes.
    let span = crate::belt::span(&reframe, crate::band::AZIMUTHS);
    let reframe = reframe.with_belt(span as f32);
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
    // And it says the model this arm exists to reach, or `lens_pixel` takes the
    // other branch on both halves and this arm is the other arm again.
    assert_eq!(
        crate::projection::tests::runs_theta(&reframe),
        theta,
        "{camera}: the blocks do not name the model this arm is meant to run",
    );
    // And it says the belt is running, over a strip wide enough to be one.
    assert!(
        reframe.is_belted(),
        "{camera}: the block says no belt, so its whole consuming half runs on neither side",
    );
    assert!(
        span.to_degrees() > 8.0,
        "{camera}: the strip is {} deg across, which is narrower than the handover",
        span.to_degrees(),
    );

    let rays = probe_rays(&reframe);
    let field = planted_field();
    let plants: Vec<([f32; 2], bool)> = (0..rays.len()).map(planted_flow).collect();
    let answers = on_the_gpu(device, queue, &reframe, &rays, &plants, &field);
    assert_eq!(answers.len(), rays.len());

    let (mut worst_weight, mut worst_pixel) = (0.0f32, 0.0f32);
    let (mut worst_depth, mut worst_axis) = (0.0f32, 0.0f32);
    let (mut worst_moved, mut worst_flow, mut biggest) = (0.0f32, 0.0f32, 0.0f32);
    let (mut mixed, mut landings) = (0usize, 0usize);
    let (mut moves, mut looked) = (0usize, 0usize);
    for ((ray, answer), plant) in rays.iter().zip(&answers).zip(&plants) {
        // The field read, compared on its own against the same planted texels.
        let (flow, on) = reframe.belt_look(*ray, &field);
        assert_eq!(
            on, answer.look.1,
            "{camera}: the two halves disagree about whether {ray:?} is on the strip",
        );
        if on {
            looked += 1;
            for (axis, read) in flow.iter().enumerate() {
                worst_flow = worst_flow.max((read - answer.look.0[axis]).abs());
            }
        }
        let mirror = reframe.blended(*ray, plant.0, plant.1);
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
                "{camera}: lens {lens} carries weight {} on the mirror and is outside on the \
                 shader at {ray:?}",
                mirror.weights[lens],
            );
            for axis in 0..2 {
                worst_pixel = worst_pixel
                    .max((answer.pixel[lens][axis] - mirror.landings[lens].pixel[axis]).abs());
            }
            worst_depth = worst_depth.max((answer.depth[lens] - mirror.landings[lens].depth).abs());
            worst_axis = worst_axis.max((answer.axis[lens] - mirror.landings[lens].axis).abs());
            // And the belt's own answer: where this lens is actually sampled.
            let shifted: f32 = (0..2)
                .map(|axis| (mirror.moved[lens][axis] - mirror.landings[lens].pixel[axis]).abs())
                .fold(0.0, f32::max);
            if shifted > 0.0 {
                moves += 1;
                biggest = biggest.max(shifted);
            }
            for axis in 0..2 {
                worst_moved =
                    worst_moved.max((answer.moved[lens][axis] - mirror.moved[lens][axis]).abs());
            }
        }
    }
    // The rays have to actually cross the seam, or the whole comparison is
    // about the far field where one lens takes everything at a weight of one.
    assert!(
        moves > 500,
        "{camera}: the belt moved only {moves} samples, so its consuming half is barely compared",
    );
    assert!(
        looked > 500,
        "{camera}: only {looked} rays are on the strip, so the field read is barely compared",
    );
    assert!(
        worst_moved < 1e-2,
        "{camera}: the two halves sample {worst_moved} px apart once the belt has moved them",
    );
    assert!(
        worst_flow < 2e-3,
        "{camera}: the two halves read the field {worst_flow} strip px apart",
    );
    assert!(
        biggest > 1.0,
        "{camera}: the largest displacement the belt applied is {biggest} px, which is not a test",
    );
    assert!(
        mixed > 500,
        "{camera}: only {mixed} of {} probes are inside the handover, so this compares the far \
         field",
        rays.len(),
    );
    assert!(
        landings > 4000,
        "{camera}: only {landings} landings reach a pixel, so this compares almost nothing",
    );
    assert!(
        worst_weight < 2e-5,
        "{camera}: the two halves disagree by {worst_weight} of a whole weight",
    );
    assert!(
        worst_pixel < 1e-2,
        "{camera}: the two halves land {worst_pixel} px apart",
    );
    assert!(
        worst_depth < 1e-2,
        "{camera}: the two halves read coverage depths {worst_depth} px apart",
    );
    assert!(
        worst_axis < 1e-6,
        "{camera}: the two halves read the landing's own axis cosine {worst_axis} apart",
    );
    eprintln!(
        "twin: {camera}: {} rays, {mixed} inside the handover, {landings} landings compared; \
         worst weight {worst_weight:.3e}, worst landing {worst_pixel:.3e} px, worst depth \
         {worst_depth:.3e} px, worst axis {worst_axis:.3e}",
        rays.len(),
    );
    eprintln!(
        "twin: {camera}: belt on over {:.3} deg, {looked} rays on the strip, {moves} samples \
         moved by up to {biggest:.2} px; worst moved landing {worst_moved:.3e} px, worst field \
         read {worst_flow:.3e} strip px",
        span.to_degrees(),
    );
}

/// **The plant survives the format it is stored in.**
///
/// The field comparison above is only worth something if both halves read the
/// same numbers, and one of them reads them back out of `Rg16Float`. Every
/// planted value is a multiple of 1/64 no larger than 16, which that format
/// holds exactly; this is what says so rather than assuming it, and it is a
/// control that can fail - widen the plant past 16 and it does.
#[test]
fn the_planted_field_survives_the_format() {
    let field = planted_field();
    let mut biggest = 0.0f32;
    for flow in &field {
        for value in flow {
            assert!(
                value.abs() <= 16.0,
                "{value} is past what f16 holds exactly"
            );
            assert_eq!(whole(half(*value)), *value, "{value} did not survive f16");
            biggest = biggest.max(value.abs());
        }
    }
    // And it is not a field of nothing.
    assert!(
        biggest > 4.0,
        "the largest planted flow is {biggest} strip px"
    );
}
