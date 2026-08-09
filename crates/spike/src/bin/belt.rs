//! **The belt's controls: does the field read what was planted in it?**
//!
//! `crates/render/src/belt.rs` produces a dense flow field over the seam and
//! the picture consumes it. This asks the only questions that decide whether
//! any of that means anything, on the working shape, through the shipped code
//! rather than a copy of it.
//!
//! ```sh
//! # the plant table: +-0.05 and +-0.30 degrees, and the zero-shift null
//! cargo run --release -p kjerag-spike --bin belt -- <file.insv> mode=plant
//!
//! # the row gate: a textureless along-seam segment fails UPWARD, measured
//! cargo run --release -p kjerag-spike --bin belt -- <file.insv> mode=gate
//!
//! # temporal stability: how much the field moves frame to frame on real film
//! cargo run --release -p kjerag-spike --bin belt -- <file.insv> mode=steady
//! ```
//!
//! **Why the plant table is the gate on this whole increment.**
//! `docs/research/belt-rung0.md` 4 measured the field's along-seam median at
//! -2.1 to -4.5 strip PIXELS across sizes whose along sampling differs by 2.7
//! times, and said plainly that a physical angle is constant in degrees and
//! this is not: it read as an estimator bias from decimating the strip below
//! the source's own sampling, and it could not separate that from a real seam.
//! So before the field is believed about anything, it has to be shown to read
//! a KNOWN shift true, at the shape that ships, on both axes, with a zero-shift
//! null that reads zero.
//!
//! **The plant is a second rectification of lens 0**, displaced across the
//! seam by a known angle and matched against the unplanted lens 0
//! ([`kjerag_render::belt::Map::planted`]). The answer has to come back at
//! minus that many strip pixels across and at zero along.

use std::path::PathBuf;

use kjerag_media::{Fallible, Plane, Size, Walk};
use kjerag_meta::CalibrationSet;
use kjerag_render::{Extent, Reframe, band, belt, seam};
use kjerag_spike::Gpu;

/// The planted shifts, in degrees across the seam.
///
/// **The pair is the point and they answer differently.** 0.05 degrees is
/// inside a Lucas-Kanade patch's linearization radius at this shape, so the
/// kernel has to recover it from a standing start and its error is this
/// instrument's accuracy figure. 0.30 is a third of the owner's own -0.906
/// degree residual and is 3.4 strip pixels here, which is past what one level
/// captures cold: it is the measurement that says the ladder is load-bearing,
/// and it is read THROUGH the ladder because that is how the belt runs.
const PLANTS: [f64; 5] = [0.0, 0.05, -0.05, 0.30, -0.30];

const USAGE: &str = "usage: belt <file.insv> [mode=plant|gate|steady] [at=seconds] [frames=n] [seam=pool|file|factory]";

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);

    let set = CalibrationSet::from_insv(&options.input)?;
    let frame = Size::new(set.dimension.width, set.dimension.height);
    // The pose the app itself would draw with, or the camera's own. Printed,
    // because a reading is only a reading at the pose it was taken at
    // (docs/research/reference-views.md).
    let lenses = match options.seam.as_str() {
        "factory" => set.lenses.clone(),
        other => {
            let fit = kjerag_spike::fit_arg(other, Some(&options.input))?;
            println!(
                "seam:   roll:{:.3},yaw:{:.3},pitch:{:.3},cx:{:.2},cy:{:.2}",
                fit.roll_deg, fit.yaw_deg, fit.pitch_deg, fit.cx_px, fit.cy_px,
            );
            fit.applied(&set.lenses)
        }
    };
    println!(
        "lens:   delivered frame {}x{}, {} lenses",
        frame.width,
        frame.height,
        lenses.len(),
    );
    if lenses.len() < 2 {
        return Err("this capture has one lens stream; the belt needs two".into());
    }

    // The map the belt is built against: a body-frame object, so the camera it
    // is built at cancels and the readout is deliberately absent.
    let map = seam::mapped(&lenses, frame);
    let span = belt::span(&map, band::AZIMUTHS);
    let (cols, rows) = belt::patches(0);
    println!(
        "strip:  {}x{} over {:.3} deg across the seam, {cols}x{rows} patches at the finest level",
        belt::COLUMNS,
        belt::ROWS,
        span.to_degrees(),
    );
    println!(
        "scale:  {:.2} strip px per degree along the ring, {:.2} across the seam\n",
        f64::from(belt::COLUMNS) / 360.0,
        f64::from(belt::ROWS) / span.to_degrees(),
    );

    let planes = decode(&options, frame)?;
    let textures: Vec<[wgpu::Texture; 2]> = planes
        .iter()
        .map(|pair| {
            std::array::from_fn(|lens| {
                let texture = lens_texture(&gpu, frame);
                upload(&gpu, &pair[lens], &texture);
                texture
            })
        })
        .collect();

    match options.mode.as_str() {
        "plant" => plant(&gpu, &map, span, &textures, frame),
        "gate" => gate(&gpu, &map, span, &textures, frame),
        "steady" => steady(&gpu, &map, span, &textures, frame),
        other => Err(format!("unknown mode {other}. {USAGE}").into()),
    }
}

// ---------------------------------------------------------------------------
// mode=plant: the subpixel estimator, against a shift nobody has to measure.
// ---------------------------------------------------------------------------

fn plant(
    gpu: &Gpu,
    map: &Reframe,
    span: f64,
    textures: &[[wgpu::Texture; 2]],
    frame: Size,
) -> Fallible<()> {
    let across_per_deg = f64::from(belt::ROWS) / span.to_degrees();
    let along_per_deg = f64::from(belt::COLUMNS) / 360.0;
    println!(
        "PLANT TABLE. Each row rectifies lens 0 twice, the second time displaced across the seam,\n\
         and reports what the field read back. Cold is the ladder from a standing start; seeded is\n\
         the ladder once and then the finest level from the previous frame's answer, which is how\n\
         the belt runs.\n"
    );
    println!(
        "{:>8}  {:>10}  {:>9}  {:>9}  {:>10}  {:>9}  {:>7}",
        "plant", "has to read", "cold", "err deg", "seeded", "err deg", "kept"
    );
    let mut worst = 0.0f64;
    for plant in PLANTS {
        let planted = belt::Map::planted(map, span, plant);
        let expected = -plant * across_per_deg;

        let mut belt = belt::Belt::new(&gpu.device);
        belt.load(&gpu.queue, &planted);
        // Frame 0 cold: the ladder from nothing, which is what a first frame
        // and a seek get.
        let cold = one(gpu, &mut belt, &textures[0], frame);
        // Frame 1 seeded from it, which is every other frame.
        let seeded = one(gpu, &mut belt, &textures[textures.len() - 1], frame);

        let cold_err = (cold.across - expected) / across_per_deg;
        let seed_err = (seeded.across - expected) / across_per_deg;
        worst = worst.max(cold_err.abs()).max(seed_err.abs());
        println!(
            "{plant:>+8.2}  {expected:>10.3}  {:>9.3}  {cold_err:>+9.4}  {:>10.3}  {seed_err:>+9.4}  {:>6.1}%",
            cold.across,
            seeded.across,
            100.0 * seeded.kept,
        );
        println!(
            "{:>8}  {:>10}  {:>9.3}  {:>+9.4}  {:>10.3}  {:>+9.4}",
            "",
            "0 along",
            cold.along,
            cold.along / along_per_deg,
            seeded.along,
            seeded.along / along_per_deg,
        );
    }
    println!(
        "\nworst error on either axis, either arm: {worst:.4} deg = {:.3} strip px across.",
        worst * across_per_deg,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// mode=gate: a textureless along-seam segment, and where its field comes from.
// ---------------------------------------------------------------------------

fn gate(
    gpu: &Gpu,
    map: &Reframe,
    span: f64,
    textures: &[[wgpu::Texture; 2]],
    frame: Size,
) -> Fallible<()> {
    // On top of a known plant, so the coarse answer in the blanked band is a
    // number rather than noise, and "failed upward" is distinguishable from
    // "snapped to zero" by reading it.
    const PLANT: f64 = 0.30;
    let across_per_deg = f64::from(belt::ROWS) / span.to_degrees();
    let planted = belt::Map::planted(map, span, PLANT);
    let (cols, _) = belt::patches(0);

    // 24 strip columns, which is three whole patch strides: wide enough that a
    // finest-level patch fits inside it with no content at all, and narrow
    // enough that two levels up - where it is 6 columns and a patch is 8 - a
    // patch straddling it keeps most of its own.
    let from = belt::COLUMNS / 4;
    let band_px = 24u32;
    let blanked: Vec<usize> = ((from / 3) as usize..((from + band_px) / 3) as usize).collect();

    let mut answers = Vec::new();
    for blank in [None, Some((from, from + band_px))] {
        let mut belt = belt::Belt::new(&gpu.device);
        belt.load(&gpu.queue, &planted);
        belt.blank(blank);
        one(gpu, &mut belt, &textures[0], frame);
        let read = one(gpu, &mut belt, &textures[textures.len() - 1], frame);
        let trust = belt.trust(&gpu.device, &gpu.queue);
        let coarse = belt.coarse(&gpu.device, &gpu.queue);
        answers.push((read, trust, coarse));
    }

    println!(
        "ROW GATE. {band_px} strip columns flattened after rectification on the second arm, on top\n\
         of a {PLANT:+.2} degree plant so the coarse answer there is a number. An untrusted segment\n\
         has to take {:.0}% of the coarse pyramid's answer and never zero.\n",
        100.0 * 0.98,
    );
    println!(
        "{:>10}  {:>8}  {:>9}  {:>10}  {:>10}",
        "arm", "trust", "failing", "coarse px", "field px"
    );
    for (label, (read, trust, coarse)) in ["untouched", "blanked"].iter().zip(&answers) {
        let mid = |values: &mut Vec<f64>| {
            values.sort_by(f64::total_cmp);
            match values.is_empty() {
                true => f64::NAN,
                false => values[values.len() / 2],
            }
        };
        let mut share: Vec<f64> = blanked.iter().map(|c| f64::from(trust[*c][0])).collect();
        let mut up: Vec<f64> = blanked.iter().map(|c| f64::from(trust[*c][2])).collect();
        // The coarse answer over the same columns, in finest-level pixels.
        let (ccols, crows) = belt::patches(2);
        let scale = (belt::COLUMNS / (belt::COLUMNS >> 2)) as f32;
        let mut coarse_px: Vec<f64> = blanked
            .iter()
            .flat_map(|c| {
                let cx = (*c as u32 / 4).min(ccols - 1);
                (0..crows).map(move |cy| f64::from(coarse[(cy * ccols + cx) as usize][1] * scale))
            })
            .collect();
        let mut field: Vec<f64> = blanked
            .iter()
            .flat_map(|c| {
                (0..read.rows).map(move |r| f64::from(read.raw[r * cols as usize + *c][1]))
            })
            .collect();
        println!(
            "{label:>10}  {:>8.3}  {:>9.3}  {:>10.3}  {:>10.3}",
            mid(&mut share),
            mid(&mut up),
            mid(&mut coarse_px),
            mid(&mut field),
        );
    }
    println!(
        "\nthe plant is {:.3} strip px across, so a field that reads about that has kept the\n\
         correction and a field that reads 0 has snapped to calibration.",
        -PLANT * across_per_deg,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// mode=steady: how much the field moves frame to frame on real film.
// ---------------------------------------------------------------------------

fn steady(
    gpu: &Gpu,
    map: &Reframe,
    span: f64,
    textures: &[[wgpu::Texture; 2]],
    frame: Size,
) -> Fallible<()> {
    let built = belt::Map::build(map, span);
    let mut belt = belt::Belt::new(&gpu.device);
    belt.load(&gpu.queue, &built);
    let across_per_deg = f64::from(belt::ROWS) / span.to_degrees();

    let mut previous: Option<Vec<[f32; 4]>> = None;
    let mut steps: Vec<f64> = Vec::new();
    let mut kept = Vec::new();
    for pair in textures {
        let read = one(gpu, &mut belt, pair, frame);
        if let Some(was) = &previous {
            let mut frame_steps: Vec<f64> = read
                .raw
                .iter()
                .zip(was)
                .filter(|(now, then)| now[2] < 0.06 && then[2] < 0.06)
                .map(|(now, then)| {
                    let dx = f64::from(now[0] - then[0]);
                    let dy = f64::from(now[1] - then[1]);
                    (dx * dx + dy * dy).sqrt()
                })
                .collect();
            frame_steps.sort_by(f64::total_cmp);
            if !frame_steps.is_empty() {
                steps.push(frame_steps[frame_steps.len() / 2]);
                kept.push(frame_steps[frame_steps.len() * 99 / 100]);
            }
        }
        previous = Some(read.raw);
    }
    // **The control that makes the number above mean something.** The same
    // frame pair, with the hint thrown away both times, has to give the same
    // field twice over: anything this reads is the estimator changing its mind
    // about content that did not change, and everything the column above reads
    // over and above it is the world going past the belt.
    let last = &textures[textures.len() - 1];
    belt.chill();
    let again = one(gpu, &mut belt, last, frame);
    belt.chill();
    let control = one(gpu, &mut belt, last, frame);
    let still = again
        .raw
        .iter()
        .zip(&control.raw)
        .map(|(a, b)| {
            let dx = f64::from(a[0] - b[0]);
            let dy = f64::from(a[1] - b[1]);
            (dx * dx + dy * dy).sqrt()
        })
        .fold(0.0, f64::max);
    // And what re-seeding from its own answer costs, on the patches the gate
    // trusted: a search at a fixed point moves by nothing when it is started
    // from where it stopped.
    let mut settle: Vec<f64> = again
        .raw
        .iter()
        .zip(&control.raw)
        .filter(|(a, b)| a[2] < 0.06 && b[2] < 0.06)
        .map(|(a, b)| {
            let dx = f64::from(a[0] - b[0]);
            let dy = f64::from(a[1] - b[1]);
            (dx * dx + dy * dy).sqrt()
        })
        .collect();
    settle.sort_by(f64::total_cmp);
    let _ = &settle;

    steps.sort_by(f64::total_cmp);
    kept.sort_by(f64::total_cmp);
    println!(
        "TEMPORAL STABILITY over {} frame pairs of real film, on the patches both frames trusted.\n",
        steps.len(),
    );
    println!(
        "median frame-to-frame change  {:.4} strip px = {:.5} deg",
        steps[steps.len() / 2],
        steps[steps.len() / 2] / across_per_deg,
    );
    println!(
        "worst of the per-frame medians {:.4} strip px = {:.5} deg",
        steps[steps.len() - 1],
        steps[steps.len() - 1] / across_per_deg,
    );
    println!(
        "worst per-frame p99            {:.4} strip px = {:.5} deg",
        kept[kept.len() - 1],
        kept[kept.len() - 1] / across_per_deg,
    );
    println!(
        "\ncontrol: the same pair, hint thrown away, run twice moves the field by {still:.6} strip\n\
         px, so everything above it is the world going past the belt and not the estimator\n\
         changing its mind.",
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// The plumbing.
// ---------------------------------------------------------------------------

struct Read {
    along: f64,
    across: f64,
    kept: f64,
    rows: usize,
    raw: Vec<[f32; 4]>,
}

/// One frame of the belt through the shipped pass, and the field it left.
fn one(gpu: &Gpu, belt: &mut belt::Belt, pair: &[wgpu::Texture; 2], frame: Size) -> Read {
    let views: Vec<wgpu::TextureView> = pair
        .iter()
        .map(|texture| texture.create_view(&Default::default()))
        .collect();
    belt.rebind(&gpu.device, [&views[0], &views[1]], false);
    belt.run(&gpu.device, &gpu.queue, frame, false);
    let raw = belt.answers(&gpu.device, &gpu.queue);
    let (cols, rows) = belt::patches(0);
    let mut along = Vec::new();
    let mut across = Vec::new();
    for entry in &raw {
        // The same gate the belt's own applies: a residual under the bar and a
        // Hessian that is not singular.
        if entry[2] < 0.06 && entry[3] > 1e-5 {
            along.push(f64::from(entry[0]));
            across.push(f64::from(entry[1]));
        }
    }
    let kept = along.len() as f64 / (cols as f64 * rows as f64);
    along.sort_by(f64::total_cmp);
    across.sort_by(f64::total_cmp);
    let mid = |values: &[f64]| match values.is_empty() {
        true => f64::NAN,
        false => values[values.len() / 2],
    };
    Read {
        along: mid(&along),
        across: mid(&across),
        kept,
        rows: rows as usize,
        raw,
    }
}

fn decode(options: &Options, frame: Size) -> Fallible<Vec<[Plane; 2]>> {
    let mut walk = Walk::open(&options.input, options.at, frame)?;
    let mut planes: Vec<[Plane; 2]> = Vec::new();
    while planes.len() < options.frames {
        let Some(mut pair) = walk.next_pair()? else {
            break;
        };
        if pair.lenses.len() < 2 {
            return Err("this capture has one lens stream; the belt needs two".into());
        }
        let second = pair.lenses.remove(1);
        let first = pair.lenses.remove(0);
        planes.push([first, second]);
    }
    if planes.len() < 2 {
        return Err("fewer than two frames decoded; the seeded arm needs a previous one".into());
    }
    println!(
        "decode: {} real consecutive frame pairs in hand",
        planes.len()
    );
    Ok(planes)
}

fn lens_texture(gpu: &Gpu, size: Size) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("belt lens"),
        size: size.extent(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn upload(gpu: &Gpu, plane: &Plane, into: &wgpu::Texture) {
    gpu.queue.write_texture(
        into.as_image_copy(),
        &plane.luma,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(plane.stride as u32),
            rows_per_image: Some(plane.size.height),
        },
        plane.size.extent(),
    );
}

struct Options {
    input: PathBuf,
    mode: String,
    at: f64,
    frames: usize,
    seam: String,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Fallible<Self> {
        let input = PathBuf::from(args.next().ok_or(USAGE)?);
        let mut out = Self {
            input,
            mode: "plant".to_owned(),
            at: 63.5,
            frames: 6,
            seam: "pool".to_owned(),
        };
        for arg in args {
            let (key, value) = arg.split_once('=').ok_or(USAGE)?;
            match key {
                "mode" => out.mode = value.to_owned(),
                "at" => out.at = value.parse()?,
                "frames" => out.frames = value.parse()?,
                "seam" => out.seam = value.to_owned(),
                _ => return Err(format!("unknown argument {key}. {USAGE}").into()),
            }
        }
        Ok(out)
    }
}
