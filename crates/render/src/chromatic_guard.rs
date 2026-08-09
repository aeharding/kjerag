//! **The chromatic mechanism's own GPU guard** (issue #103, stage 10): the
//! shipped WGSL against its Rust twins, on a real device, at fixtures that
//! carry a NON-ZERO field and a NON-ZERO plant - because a guarded function
//! is only guarded at a fixture that reaches it, which is the lesson
//! [`super::twin`] records being learned twice before this file was written.
//!
//! Two tests, two boundaries:
//!
//! - **The lookup**: `chromatic_cell`, `chromatic_kernel`, `chromatic_reach`
//!   and the `view_to_body` transform, compiled from the same
//!   `chromatic::wgsl()` emission the draw compiles, probed over rays that
//!   cross the seam at every azimuth, against [`chromatic::pull`].
//! - **The estimator**: the whole band module - `measure`'s chroma
//!   photometry at zero shift, `settle_chroma`'s conversion and gates, and
//!   `pool_chroma`'s assembly and ease - run on PLANTED synthetic planes
//!   whose two lenses disagree by a known per-channel ripple at a known
//!   azimuth, read back and held against the planted truth and against
//!   [`chromatic::field_target`], the pooling's Rust twin.
//!
//! Like the twin guard, this needs a GPU and says so: `KJERAG_REQUIRE_GPU`
//! makes the skip a failure, and `scripts/uitest.sh` sets it.

#![cfg(test)]

use crate::projection::{Held, Reframe};
use crate::sampling::Sampling;
use crate::twin::gpu;
use crate::{Camera, Size, band, chromatic};

/// The emitted WGSL the probe rides behind: exactly what the draw compiles,
/// so the functions under test are the shipped strings and not copies.
fn probe_module() -> String {
    format!(
        "{}\n{}\n{PROBE}",
        crate::projection::wgsl(),
        chromatic::wgsl()
    )
}

/// The probe: group 0 is the uniform block the way the draw binds it, group
/// 1 carries the rays (xyz the view ray, w the arm's scale), the answers,
/// and a planted field. The glue here is line for line the `chromatic_half`
/// lookup in `band::LOOKUP`, reading the probe's own field buffer where the
/// lookup reads the band state - the array indexing is the one thing this
/// cannot borrow from the shipped string, and it is held to the same Rust
/// twin the shipped glue is.
const PROBE: &str = r#"
@group(1) @binding(0) var<storage, read> probes: array<vec4<f32>>;
@group(1) @binding(1) var<storage, read_write> answers: array<vec4<f32>>;
@group(1) @binding(2) var<storage, read> planted: array<vec4<f32>>;

@compute @workgroup_size(64)
fn chroma_twin(@builtin(global_invocation_id) id: vec3<u32>) {
  let index = id.x;
  if index >= arrayLength(&probes) {
    return;
  }
  let scale = probes[index].w;
  var out = vec3<f32>(0.0);
  if scale != 0.0 {
    let at = chromatic_cell(reframe.view_to_body * probes[index].xyz);
    if at.z < CHROMATIC_EDGE {
      let low = u32(i32(at.x) % i32(CHROMATIC_CELLS) + i32(CHROMATIC_CELLS)) % CHROMATIC_CELLS;
      let e0 = planted[low];
      let e1 = planted[(low + 1u) % CHROMATIC_CELLS];
      out = chromatic_reach(e0.xyz, e1.xyz, at.y, at.z, scale);
    }
  }
  answers[index] = vec4<f32>(out, 0.0);
}
"#;

/// A map at a deliberately turned camera, like the twin guard's: the
/// `view_to_body` the lookup transforms through has to be doing something,
/// or the transform is compared at the identity.
fn fixture() -> Reframe {
    Reframe::new(
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
        Held::default(),
        16.0 / 9.0,
        true,
        Sampling::default(),
    )
}

/// A smooth, everywhere-non-zero field of code-sized amplitudes: one and two
/// cycles plus a DC on each channel, the shapes the mechanism exists to
/// carry.
fn planted_field() -> [[f32; 3]; band::AZIMUTHS] {
    std::array::from_fn(|index| {
        let phi = index as f32 / band::AZIMUTHS as f32 * std::f32::consts::TAU;
        [
            (2.0 * phi.cos() + 0.5) / 255.0,
            (-0.8 * (2.0 * phi).sin() - 0.3) / 255.0,
            (-2.0 * phi.cos() - 1.0) / 255.0,
        ]
    })
}

/// **The lookup and its Rust twin answer the same pull**, at a turned
/// camera, over rays that cross the seam at every azimuth, at both arms and
/// at the off-equality.
///
/// Bars calibrated the twin guard's way: measured on RADV (worst observed
/// 6.0e-8 of a code across 8786 answered rays), bar set roughly ten times
/// over, still orders under the smallest planted amplitude.
#[test]
fn the_chromatic_lookup_and_its_rust_twin_answer_the_same_pull() {
    let (device, queue, name) = match gpu() {
        Ok(open) => open,
        Err(why) => {
            assert!(
                std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}",
            );
            eprintln!("chromatic guard: skipped, no GPU on this box ({why})");
            return;
        }
    };
    eprintln!("chromatic guard: {name}");
    let reframe = fixture();
    let field = planted_field();
    // A term left at its default takes the code that reads it out of the
    // comparison entirely: the fixture must carry a field.
    assert!(
        field.iter().flatten().all(|value| *value != 0.0),
        "the planted field has a zero in it, so part of the lookup is compared against nothing",
    );

    // Rays in the body frame, where the seam stands still, handed over in the
    // view's: a walk across the seam at every 5 degrees of azimuth, past the
    // kernel's edge on both sides, at the off arm, the on arm and the double.
    let mut rays: Vec<([f32; 3], f32)> = Vec::new();
    for arm in [0.0f32, 1.0, 2.0] {
        for step in 0..72 {
            let phi = (step as f32 * 5.0 + 1.3).to_radians();
            for off in -45..=45 {
                let off = (off as f32).to_radians();
                let body = [off.cos() * phi.cos(), off.cos() * phi.sin(), off.sin()];
                rays.push((reframe.view_ray_from_body(body), arm));
            }
        }
    }

    let answers = on_the_gpu(&device, &queue, &reframe, &field, &rays);
    let mut worst = 0.0f32;
    let mut answered = 0usize;
    for ((ray, arm), answer) in rays.iter().zip(&answers) {
        let mirror = chromatic::pull(&reframe, &field, *arm, *ray);
        if *arm == 0.0 {
            // The off arm is an equality, on both halves.
            assert_eq!(*answer, [0.0; 3], "the off arm moved a probe");
            assert_eq!(mirror, [0.0; 3]);
            continue;
        }
        if answer.iter().any(|value| *value != 0.0) {
            answered += 1;
        }
        for channel in 0..3 {
            worst = worst.max((answer[channel] - mirror[channel]).abs());
        }
    }
    // The walk has to actually land inside the kernel, or this compares
    // nothing but the refusals.
    assert!(
        answered > 4000,
        "only {answered} probes landed in the kernel"
    );
    assert!(
        worst < 1e-6,
        "the two halves disagree by {worst} of full scale ({} codes)",
        worst * 255.0,
    );
    eprintln!(
        "chromatic guard: lookup, {} rays, {answered} inside the kernel, worst {worst:.3e}",
        rays.len(),
    );
}

/// The probe pass itself: buffers up, dispatch, read back.
fn on_the_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    reframe: &Reframe,
    field: &[[f32; 3]; band::AZIMUTHS],
    rays: &[([f32; 3], f32)],
) -> Vec<[f32; 3]> {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("chromatic twin"),
        source: wgpu::ShaderSource::Wgsl(probe_module().into()),
    });
    let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("chromatic twin block"),
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
    let probe_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("chromatic twin probes"),
        entries: &[storage(0, true), storage(1, false), storage(2, true)],
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("chromatic twin"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("chromatic twin"),
                bind_group_layouts: &[&uniform_layout, &probe_layout],
                immediate_size: 0,
            }),
        ),
        module: &module,
        entry_point: Some("chroma_twin"),
        compilation_options: Default::default(),
        cache: None,
    });

    let buffer = |label: &str, contents: &[u8], usage: wgpu::BufferUsages| {
        let made = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: contents.len() as u64,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&made, 0, contents);
        made
    };
    let uniform = buffer(
        "chromatic block",
        reframe.bytes(),
        wgpu::BufferUsages::UNIFORM,
    );
    let padded: Vec<[f32; 4]> = rays
        .iter()
        .map(|(ray, arm)| [ray[0], ray[1], ray[2], *arm])
        .collect();
    let probes = buffer(
        "chromatic rays",
        floats_of(&padded),
        wgpu::BufferUsages::STORAGE,
    );
    let entries: Vec<[f32; 4]> = field.iter().map(|e| [e[0], e[1], e[2], 0.0]).collect();
    let planted = buffer(
        "chromatic field",
        floats_of(&entries),
        wgpu::BufferUsages::STORAGE,
    );
    let answer_bytes = (rays.len() * 16) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("chromatic answers"),
        size: answer_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("chromatic readback"),
        size: answer_bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let uniform_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("chromatic twin block"),
        layout: &uniform_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    let probe_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("chromatic twin probes"),
        layout: &probe_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: probes.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: planted.as_entire_binding(),
            },
        ],
    });

    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &uniform_group, &[]);
        pass.set_bind_group(1, &probe_group, &[]);
        pass.dispatch_workgroups(rays.len().div_ceil(64) as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, answer_bytes);
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the chromatic probe pass did not finish");
    let mapped = readback.slice(..).get_mapped_range();
    let floats: Vec<f32> = mapped
        .chunks_exact(4)
        .map(|word| f32::from_le_bytes([word[0], word[1], word[2], word[3]]))
        .collect();
    drop(mapped);
    readback.unmap();
    (0..rays.len())
        .map(|probe| {
            [
                floats[probe * 4],
                floats[probe * 4 + 1],
                floats[probe * 4 + 2],
            ]
        })
        .collect()
}

fn floats_of(rows: &[[f32; 4]]) -> &[u8] {
    // `[f32; 4]` has no padding and no invalid pattern; the same cast
    // `Reframe::bytes` makes.
    unsafe { std::slice::from_raw_parts(rows.as_ptr().cast::<u8>(), std::mem::size_of_val(rows)) }
}

// ------------------------------------------------------------ the estimator

/// The synthetic pair: flat mid-grey luma on both lenses - which makes every
/// direction the FLAT population, the zero-shift path that carries 77 to 89
/// percent of a real ring (M-5) - and a chroma plane on lens 1 that differs
/// from lens 0 by a known per-channel ripple: two raw codes of Cr and minus
/// two of Cb at the crest, walking round the ring as one cycle of azimuth.
/// Through BT.709 that is `+3.15 R, -0.56 G, -3.71 B` codes at the crest,
/// three channels, three different sizes, two signs.
struct Plant {
    luma: Vec<u8>,
    chroma0: Vec<u8>,
    chroma1: Vec<u8>,
    frame: u32,
}

/// The ripple's raw chroma amplitude at the crest, in texture codes.
const PLANT_RAW: f32 = 2.0;

impl Plant {
    fn new(frame: u32) -> Self {
        let half = frame / 2;
        let luma = vec![102u8; (frame * frame) as usize]; // 0.4 of full scale
        let chroma0 = vec![128u8; (half * half * 2) as usize];
        let mut chroma1 = chroma0.clone();
        let centre = half as f32 / 2.0;
        for y in 0..half {
            for x in 0..half {
                let eta = ((y as f32 + 0.5) - centre).atan2((x as f32 + 0.5) - centre);
                let swing = PLANT_RAW * eta.cos();
                let at = ((y * half + x) * 2) as usize;
                chroma1[at] = (128.0 - swing).round().clamp(0.0, 255.0) as u8; // Cb down
                chroma1[at + 1] = (128.0 + swing).round().clamp(0.0, 255.0) as u8; // Cr up
            }
        }
        Self {
            luma,
            chroma0,
            chroma1,
            frame,
        }
    }

    /// A bilinear read of one chroma plane at a full-resolution pixel
    /// coordinate: the CPU mirror of what the GPU sampler answers at
    /// `frame_uv`, quantization included, so the truth the estimator is held
    /// to carries everything the plant's own storage did to it.
    fn chroma_at(&self, plane: &[u8], pixel: [f32; 2]) -> [f32; 2] {
        let half = (self.frame / 2) as f32;
        let uv = [
            (pixel[0] + 0.5) / self.frame as f32,
            (pixel[1] + 0.5) / self.frame as f32,
        ];
        let x = (uv[0] * half - 0.5).clamp(0.0, half - 1.0);
        let y = (uv[1] * half - 0.5).clamp(0.0, half - 1.0);
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let texel = |tx: u32, ty: u32, channel: usize| {
            let tx = tx.min(self.frame / 2 - 1);
            let ty = ty.min(self.frame / 2 - 1);
            plane[((ty * self.frame / 2 + tx) * 2) as usize + channel] as f32 / 255.0
        };
        std::array::from_fn(|channel| {
            let top = texel(x0, y0, channel) * (1.0 - fx) + texel(x0 + 1, y0, channel) * fx;
            let low = texel(x0, y0 + 1, channel) * (1.0 - fx) + texel(x0 + 1, y0 + 1, channel) * fx;
            top * (1.0 - fy) + low * fy
        })
    }

    /// What the estimator should read at one ring cell: the patch's own
    /// per-channel means through the same taps, the same pairing and the
    /// same conversion, mirrored in Rust off the planted bytes. `baseline`
    /// is [`band::baseline`] of the same lenses the map was built from,
    /// which is what `Reframe::new` stored and what `ring_of` reads back.
    fn expected_cell(
        &self,
        reframe: &Reframe,
        baseline: [f32; 3],
        index: usize,
    ) -> chromatic::ChromaCell {
        let step = 0.10f32.to_radians();
        let half_patch = 10i32;
        let ring = band::Ring::cell(index, baseline);
        let mut sums = ([0.0f64; 3], [0.0f64; 3]);
        let mut count = 0.0f64;
        for row in -half_patch..=half_patch {
            for column in -half_patch..=half_patch {
                let body = std::array::from_fn(|axis| {
                    ring.centre[axis]
                        + column as f32 * step * ring.perp[axis]
                        + row as f32 * step * ring.epi[axis]
                });
                let view = reframe.view_ray_from_body(body);
                let mix = reframe.blend(view);
                if mix.landings.iter().any(|landing| !landing.inside) {
                    continue;
                }
                let y = 102.0 / 255.0;
                let c0 = self.chroma_at(&self.chroma0, mix.landings[0].pixel);
                let c1 = self.chroma_at(&self.chroma1, mix.landings[1].pixel);
                for (sum, raw) in [
                    (&mut sums.0, [y, c0[0], c0[1]]),
                    (&mut sums.1, [y, c1[0], c1[1]]),
                ] {
                    for channel in 0..3 {
                        sum[channel] += f64::from(raw[channel]);
                    }
                }
                count += 1.0;
            }
        }
        if count <= 0.0 {
            return chromatic::ChromaCell::default();
        }
        let total = f64::from(((2 * half_patch + 1) * (2 * half_patch + 1)) as u32);
        let mean = |sum: [f64; 3]| std::array::from_fn(|c| (sum[c] / count) as f32);
        chromatic::ChromaCell {
            m0: chromatic::rgb_of(mean(sums.0), false),
            m1: chromatic::rgb_of(mean(sums.1), false),
            lit: 102.0 / 255.0,
            evidence: (count / total).min(1.0) as f32,
            population: chromatic::POP_ZERO_SHIFT,
            _pad: [0.0; 3],
        }
    }
}

/// **The estimator reads the planted ripple, per channel, and the pooled
/// field closes it.** The whole chromatic compute path on a real GPU -
/// `measure`'s flat-path chroma photometry at zero shift, `settle_chroma`,
/// `pool` (which reads nothing here: flat content never correlates, so the
/// tone stays zero, itself a claim this fixture checks), and `pool_chroma` -
/// against the Rust mirrors and the planted truth.
#[test]
fn the_estimator_recovers_a_planted_ripple_and_the_correction_closes_it() {
    let (device, queue, name) = match gpu() {
        Ok(open) => open,
        Err(why) => {
            assert!(
                std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}",
            );
            eprintln!("chromatic guard: skipped, no GPU on this box ({why})");
            return;
        }
    };
    eprintln!("chromatic guard: estimator on {name}");
    let reframe = fixture();
    let baseline = band::baseline(&crate::projection::tests::fixture_lenses());
    let plant = Plant::new(3840);

    let state = run_band(&device, &queue, &reframe, &plant);
    let (head, cells, field) = state;

    // The arm reached the pass and the ring answered: the fixture is alive.
    assert_eq!(head.scale, 1.0);
    let read = cells.iter().filter(|cell| cell.evidence > 0.0).count();
    assert!(read > 100, "only {read} of the ring was read");

    // Every read direction is the zero-shift population: flat content
    // correlates with nothing, and the geometry's own refusal is untouched.
    for cell in cells.iter().filter(|cell| cell.evidence > 0.0) {
        assert_eq!(cell.population, chromatic::POP_ZERO_SHIFT);
    }

    // The reading against the planted bytes, mirrored through the same taps:
    // per channel, to a sixth of a code. This is the estimator's whole
    // chain - landings, pairing, quarter-resolution sampling, conversion -
    // against an independent CPU walk of the same numbers.
    let mut worst = 0.0f32;
    let mut compared = 0usize;
    for (index, cell) in cells.iter().enumerate() {
        if cell.evidence <= 0.0 {
            continue;
        }
        let expected = plant.expected_cell(&reframe, baseline, index);
        if expected.evidence <= 0.0 {
            continue;
        }
        compared += 1;
        for channel in 0..3 {
            worst = worst.max((cell.m0[channel] - expected.m0[channel]).abs());
            worst = worst.max((cell.m1[channel] - expected.m1[channel]).abs());
        }
    }
    assert!(compared > 100, "only {compared} cells compared");
    assert!(
        worst * 255.0 < 0.17,
        "the estimator and its mirror disagree by {} codes",
        worst * 255.0,
    );

    // The RECOVERY: where the ripple crests - the ring azimuth whose image
    // landing sits at the plant's own crest, found rather than assumed,
    // because the mounting decides how ring azimuth maps to image angle -
    // the per-channel disagreement is the plant: +3.15 R, -0.56 G, -3.71 B
    // codes, three channels, three sizes, two signs, within the plant's own
    // storage quantization and the patch's smoothing of a one-cycle ripple.
    let d = |cell: &chromatic::ChromaCell, channel: usize| {
        (cell.m1[channel] - cell.m0[channel]) * 255.0
    };
    let planted_rgb = [
        PLANT_RAW * 1.5748,
        PLANT_RAW * (0.1873 - 0.4681),
        -PLANT_RAW * 1.8556,
    ];
    let crest_index = (0..cells.len())
        .filter(|index| cells[*index].evidence > 0.0)
        .max_by(|a, b| d(&cells[*a], 0).total_cmp(&d(&cells[*b], 0)))
        .expect("no read cell");
    let crest = &cells[crest_index];
    for (channel, planted) in planted_rgb.iter().enumerate() {
        let read = d(crest, channel);
        assert!(
            (read - planted).abs() < 0.75,
            "channel {channel}: read {read} codes against {planted} planted",
        );
    }
    // And the trough carries the same plant negated: one cycle passes
    // through both, so a sign error anywhere in the chain fails here.
    let trough = (0..cells.len())
        .filter(|index| cells[*index].evidence > 0.0)
        .min_by(|a, b| d(&cells[*a], 0).total_cmp(&d(&cells[*b], 0)))
        .expect("no read cell");
    assert!(
        (d(&cells[trough], 0) + planted_rgb[0]).abs() < 0.75,
        "the trough reads {} codes against {} planted",
        d(&cells[trough], 0),
        -planted_rgb[0],
    );

    // The POOLED FIELD against its Rust twin, over the very cells the GPU
    // wrote: the same assembly, the same guard, the same shrink, the same
    // one ease step from zero on a reset frame.
    let target = chromatic::field_target(&cells, 0.0);
    let step = band::ease(1.0 / 30.0, band::TAU_GAIN_S);
    let mut worst_field = 0.0f32;
    for (entry, mirror) in field.iter().zip(&target) {
        for channel in 0..3 {
            worst_field = worst_field.max((entry[channel] - mirror[channel] * step).abs());
        }
    }
    assert!(
        worst_field < 1e-6,
        "pool_chroma and field_target disagree by {worst_field} of full scale",
    );

    // The CLOSURE: at the crest, applying the converged field - the target,
    // which the ease walks to over TAU_GAIN seconds of film - as `picture`
    // applies it moves the two lenses onto each other. The field is the
    // smoothed, evidence-shrunk estimate, so what it closes is bounded by
    // the shrink and by the kernel's smoothing of a one-cycle ripple; both
    // together still close over three quarters of the split, per channel.
    // Read at the middle of the crest rather than at its edge: the plant's
    // 8-bit storage staircases the cosine into plateaus, and a cell at a
    // plateau's END has half its smoothing window over the step down. The
    // smoothed field's own maximum is the plateau's middle, and the raw
    // reading there is still the full crest, which the assertion checks.
    let closing = (0..cells.len())
        .filter(|index| cells[*index].evidence > 0.0)
        .max_by(|a, b| target[*a][0].total_cmp(&target[*b][0]))
        .expect("no read cell");
    assert!(d(&cells[closing], 0) > planted_rgb[0] - 0.75);
    for channel in [0usize, 2] {
        let split = d(&cells[closing], channel);
        let corrected = (cells[closing].m1[channel] - 0.5 * target[closing][channel])
            - (cells[closing].m0[channel] + 0.5 * target[closing][channel]);
        let closed = 1.0 - (corrected * 255.0 / split).abs();
        assert!(
            closed > 0.75,
            "channel {channel}: only {:.0} percent of a {split:.2} code split closes",
            closed * 100.0,
        );
    }
    eprintln!(
        "chromatic guard: estimator, {compared} cells vs the mirror, worst {:.3} codes; \
         field vs twin worst {worst_field:.3e}; crest split R {:+.2} G {:+.2} B {:+.2} codes",
        worst * 255.0,
        d(crest, 0),
        d(crest, 1),
        d(crest, 2),
    );
}

/// One reset frame of the band module over the planted planes: measure the
/// whole ring, pool, pool the chromatic field, and read the state back.
fn run_band(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    reframe: &Reframe,
    plant: &Plant,
) -> (
    chromatic::Chromatic,
    Vec<chromatic::ChromaCell>,
    Vec<[f32; 4]>,
) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("chromatic band"),
        source: wgpu::ShaderSource::Wgsl(
            format!(
                "{}\n{}\n{}",
                crate::projection::wgsl(),
                chromatic::wgsl(),
                band::wgsl()
            )
            .into(),
        ),
    });
    // Group 0 the way the scene lays it out - uniform, four planes, sampler -
    // with compute visibility, and group 1 the band's own state and watch.
    let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Reframe>() as u64),
        },
        count: None,
    }];
    entries.extend((1..5).map(texture_entry));
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: 5,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    });
    let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("chromatic band scene"),
        entries: &entries,
    });
    let band_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("chromatic band state"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: band::STATE_BINDING,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(band::BYTES),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: band::WATCH_BINDING,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<band::Watch>() as u64
                    ),
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("chromatic band"),
        bind_group_layouts: &[&scene_layout, &band_layout],
        immediate_size: 0,
    });
    let compute = |entry: &str| {
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("chromatic band"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    let measure = compute("measure");
    let pool = compute("pool");
    let pool_chroma = compute("pool_chroma");

    let texture = |label: &str, format, width: u32, bytes: &[u8], stride: u32| {
        let made = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height: width,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &made,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width,
                height: width,
                depth_or_array_layers: 1,
            },
        );
        made
    };
    let frame = plant.frame;
    let luma0 = texture(
        "plant luma0",
        wgpu::TextureFormat::R8Unorm,
        frame,
        &plant.luma,
        frame,
    );
    let luma1 = texture(
        "plant luma1",
        wgpu::TextureFormat::R8Unorm,
        frame,
        &plant.luma,
        frame,
    );
    let chroma0 = texture(
        "plant chroma0",
        wgpu::TextureFormat::Rg8Unorm,
        frame / 2,
        &plant.chroma0,
        frame,
    );
    let chroma1 = texture(
        "plant chroma1",
        wgpu::TextureFormat::Rg8Unorm,
        frame / 2,
        &plant.chroma1,
        frame,
    );
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("chromatic band block"),
        size: std::mem::size_of::<Reframe>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&uniform, 0, reframe.bytes());
    let state = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("chromatic band state"),
        size: band::BYTES,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let mut watch = band::Watch::start(1.0 / 30.0);
    watch.chromatic = 1.0;
    let watch_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("chromatic band watch"),
        size: std::mem::size_of::<band::Watch>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&watch_buffer, 0, watch.bytes());

    let views: Vec<wgpu::TextureView> = [&luma0, &chroma0, &luma1, &chroma1]
        .iter()
        .map(|texture| texture.create_view(&Default::default()))
        .collect();
    let mut group_entries = vec![wgpu::BindGroupEntry {
        binding: 0,
        resource: uniform.as_entire_binding(),
    }];
    group_entries.extend(
        views
            .iter()
            .enumerate()
            .map(|(plane, view)| wgpu::BindGroupEntry {
                binding: 1 + plane as u32,
                resource: wgpu::BindingResource::TextureView(view),
            }),
    );
    group_entries.push(wgpu::BindGroupEntry {
        binding: 5,
        resource: wgpu::BindingResource::Sampler(&sampler),
    });
    let scene_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("chromatic band scene"),
        layout: &scene_layout,
        entries: &group_entries,
    });
    let band_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("chromatic band state"),
        layout: &band_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: band::STATE_BINDING,
                resource: state.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: band::WATCH_BINDING,
                resource: watch_buffer.as_entire_binding(),
            },
        ],
    });

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("chromatic band readback"),
        size: band::BYTES,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&measure);
        pass.set_bind_group(0, &scene_group, &[]);
        pass.set_bind_group(1, &band_group, &[]);
        pass.dispatch_workgroups(watch.groups(), 1, 1);
        pass.set_pipeline(&pool);
        pass.dispatch_workgroups(1, 1, 1);
        pass.set_pipeline(&pool_chroma);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&state, 0, &readback, 0, band::BYTES);
    queue.submit([encoder.finish()]);
    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the chromatic band pass did not finish");

    let mapped = readback.slice(..).get_mapped_range();
    let float = |at: usize| {
        f32::from_le_bytes([mapped[at], mapped[at + 1], mapped[at + 2], mapped[at + 3]])
    };
    let head = chromatic::Chromatic {
        scale: float(band::CHROMA_HEAD_AT),
        evidence: float(band::CHROMA_HEAD_AT + 4),
        _pad: [0.0; 2],
    };
    let cells: Vec<chromatic::ChromaCell> = (0..band::AZIMUTHS)
        .map(|index| {
            let at = band::CHROMA_CELLS_AT + index * std::mem::size_of::<chromatic::ChromaCell>();
            chromatic::ChromaCell {
                m0: [float(at), float(at + 4), float(at + 8)],
                m1: [float(at + 12), float(at + 16), float(at + 20)],
                lit: float(at + 24),
                evidence: float(at + 28),
                population: float(at + 32),
                _pad: [0.0; 3],
            }
        })
        .collect();
    let field: Vec<[f32; 4]> = (0..band::AZIMUTHS)
        .map(|index| {
            let at = band::FIELD_AT + index * 16;
            [float(at), float(at + 4), float(at + 8), float(at + 12)]
        })
        .collect();
    drop(mapped);
    readback.unmap();
    (head, cells, field)
}
