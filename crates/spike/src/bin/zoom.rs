//! What high-quality sampling at high zoom is worth, and what it costs
//! (issue #11).
//!
//! Six questions, all of them answered against real footage through the app's
//! own pass, and the first one decides whether the rest matter.
//!
//! ```sh
//! cargo run --release -p kyerag-spike --bin zoom -- <file.insv> \
//!   [frame=n | time=s] [yaw=deg] [pitch=deg] [fov=deg] [size=px] [shot=px] [lock=0]
//! ```
//!
//! - **ratio**: how far the view magnifies the source, where, and which plane
//!   notices first. The number the shader decides on is the local Jacobian of
//!   the backward map (`Reframe::texels_per_pixel`), swept across the zoom
//!   range and across the picture, because a fisheye's angular density is not
//!   uniform and neither is a rectilinear output's.
//! - **delta**: the same view rendered three ways, bilinear against the
//!   shipped luma upgrade against both planes, as PNGs and as the differences
//!   between them. The detail metric is the mean absolute Laplacian of the
//!   luma, which is what "crisper" has to show up as if it is real; the PNGs
//!   are what says whether it rang while doing it.
//! - **wide**: what is not magnified has to be untouched. "Wide field of
//!   view" turns out not to be the same question, so the claim is made per
//!   pixel: every pixel that moved is checked against the ratio it sits at.
//! - **sweep**: scrolling the zoom through the threshold must not pop. How
//!   much of the difference between the settings arrives in the worst single
//!   step, and whether the picture itself steps harder sharp than soft.
//! - **shot**: a still is the same pipeline drawn into a bigger target
//!   (issue #15), so it magnifies harder than the window it came off and has
//!   to work that out for itself. This takes one through `Scene::capture` and
//!   compares it against the same view drawn straight into a target of the
//!   still's own size.
//! - **cost**: the pass alone, the settings interleaved render by render.
//!   `--bin playback` is where dropped and starved come from; this is where
//!   the milliseconds do.
//!
//! PNGs land in ./scratch/, which is gitignored: frames of real footage are
//! personal video and this repo is public.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use kyerag_media::Fallible;
use kyerag_meta::{CalibrationSet, Lens};
use kyerag_render::{
    Camera, Cue, Held, Horizon, Reframe, Request, Sampling, Scene, ScenePipeline, Size, sampling,
};
use kyerag_spike::{Difference, FORMAT, Gpu, Offscreen, Picture, Render, aspect};

/// The zoom range, in degrees: `Camera`'s own limits, the default, and the
/// places between them where the two planes cross their thresholds.
const FOVS: [f32; 7] = [20.0, 25.0, 35.0, 60.0, 90.0, 100.0, 110.0];

/// Where in the output the ratio is read. A rectilinear view's own angular
/// density falls off with the cosine squared of the angle from the axis, so
/// the corner of a wide view is magnified harder than its middle.
const PLACES: [(&str, [f32; 2]); 3] = [
    ("centre", [0.5, 0.5]),
    ("edge", [0.98, 0.5]),
    ("corner", [0.98, 0.98]),
];

/// Fields of view the pop sweep walks, in degrees. It has to cross both
/// planes' thresholds, which on this footage sit inside it.
const SWEEP: (f32, f32, usize) = (40.0, 110.0, 71);

/// The window the player's own cost numbers are taken at, and the output
/// every picture here is rendered at unless `size=` says otherwise. It is
/// half the measurement: magnification is texels per **output pixel**, so a
/// view rendered small is a view that is not magnifying.
const WINDOW: Size = Size {
    width: 2560,
    height: 1440,
};

/// Output widths the byte-identity check walks, at the window's aspect. The
/// small ones are not a window anyone uses; they are the only way to put the
/// whole picture past 1:1 on a camera whose delivered frame is this dense.
const WIDTHS: [u32; 5] = [640, 960, 1280, 1920, 2560];

struct Options {
    input: PathBuf,
    camera: Camera,
    at: Cue,
    output: Size,
    shot: u32,
    horizon: Horizon,
}

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    std::fs::create_dir_all("scratch")?;

    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);

    let (lenses, frame) = ratios(&options)?;

    // An instrument has no stored calibration to read: the app keeps that in
    // its own config, and this is not the app. So the seam is fitted off this
    // file, which is what every instrument did before the calibration moved
    // to the camera (issue #48).
    let scene = Scene::still(&options.input, options.at)?;
    scene.fit_seam();
    scene.set_horizon(options.horizon);
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let render = Render {
        gpu: &gpu,
        scene: &scene,
        pipeline: &mut pipeline,
    };
    let mut render = render;

    delta(&mut render, &options)?;
    untouched(&mut render, &options, &lenses, frame)?;
    sweep(&mut render, &options)?;
    shot(&mut render, &options, &lenses, frame)?;
    cost(&mut render, &options)
}

/// What the upgrade costs the pass, with nothing else in the measurement.
///
/// A still frame, so nothing decodes, nothing imports, and no clock runs:
/// one process, one file, one frame, and the settings interleaved render by
/// render, so a box that throttles throttles all of them together. What is
/// reported is the **least** of the repetitions, which is the pass with
/// nothing else on the machine, and the median beside it, which says how
/// much else there was. `kyerag-spike --bin playback` is where dropped and
/// starved come from; this is where the milliseconds do.
fn cost(render: &mut Render, options: &Options) -> Fallible<()> {
    let target = Offscreen::new(&render.gpu.device, options.output, FORMAT);
    println!(
        "\ncost:   the pass alone at {}x{}, {REPETITIONS} renders a cell, interleaved. \
         ms/redraw, least (median)",
        options.output.width, options.output.height,
    );
    println!("        fov | bilinear         luma            both planes");

    for fov in FOVS {
        let camera = Camera {
            fov: fov.to_radians(),
            ..options.camera
        };
        let mut runs = [const { Vec::new() }; 3];
        for repetition in 0..REPETITIONS {
            for (slot, sampling) in [Sampling::Bilinear, Sampling::Luma, Sampling::Sharp]
                .into_iter()
                .enumerate()
            {
                render.scene.set_sampling(sampling);
                let primitive = render.scene.primitive(camera);
                render.pipeline.prepare(
                    &primitive,
                    &render.gpu.device,
                    &render.gpu.queue,
                    aspect(options.output),
                );
                let began = std::time::Instant::now();
                target.render(&render.gpu.device, &render.gpu.queue, render.pipeline)?;
                // The first of each cell warms the pipeline and the target.
                if repetition > 0 {
                    runs[slot].push(began.elapsed().as_secs_f64() * 1000.0);
                }
            }
        }
        let cell = |took: &mut Vec<f64>| {
            took.sort_by(f64::total_cmp);
            format!("{:5.2} ({:5.2})", took[0], took[took.len() / 2])
        };
        println!(
            "        {fov:3.0} | {}  {}  {}",
            cell(&mut runs[0]),
            cell(&mut runs[1]),
            cell(&mut runs[2]),
        );
    }
    Ok(())
}

/// Renders a cell of the cost table, one of which is thrown away warming up.
const REPETITIONS: usize = 40;

/// One question: how far is the source magnified, and where.
///
/// No pixels in this one. It is the model's own Jacobian at the fixture the
/// file carries, which is what the shader reads off the hardware's quad
/// derivatives per fragment. The body is held level, so what varies down the
/// table is the view and the lens geometry under it.
fn ratios(options: &Options) -> Fallible<(Vec<Lens>, Size)> {
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let chroma = frame.halved();
    println!(
        "lens:   {} {}, frames {}x{}, chroma {}x{}",
        calibration.camera_model,
        calibration.firmware,
        frame.width,
        frame.height,
        chroma.width,
        chroma.height,
    );
    println!(
        "\nratio:  delivered-frame texels per output pixel at {}x{}, and how far each plane's \
         kernel engages at the centre. Under 1 is magnifying.",
        options.output.width, options.output.height,
    );
    println!("        fov | centre    edge    corner | luma   chroma");
    for fov in FOVS {
        let camera = Camera {
            fov: fov.to_radians(),
            ..options.camera
        };
        let reframe = Reframe::new(
            &calibration.lenses,
            frame,
            camera,
            Held::default(),
            aspect(options.output),
            false,
            Sampling::default(),
        );
        let at = |uv| lit(&reframe, uv, options.output);
        let middle = at(PLACES[0].1);
        println!(
            "        {fov:3.0} | {:6.3}  {:6.3}   {:6.3} | {:5.2}  {:5.2}",
            middle,
            at(PLACES[1].1),
            at(PLACES[2].1),
            engaged(middle, frame.width as f32, frame.width as f32),
            engaged(middle, chroma.width as f32, frame.width as f32),
        );
    }
    Ok((calibration.lenses, frame))
}

/// The ratio at one place of the output, from whichever lens has the ray.
fn lit(reframe: &Reframe, uv: [f32; 2], output: Size) -> f32 {
    let Some(ray) = reframe.view_ray(uv) else {
        // The room around the ball, which is not a magnification of anything.
        return f32::INFINITY;
    };
    let blend = reframe.blend(ray);
    let lens = (0..blend.weights.len())
        .max_by(|a, b| blend.weights[*a].total_cmp(&blend.weights[*b]))
        .unwrap_or(0);
    reframe.texels_per_pixel(lens, uv, output)
}

fn engaged(ratio: f32, plane_width: f32, frame_width: f32) -> f32 {
    sampling::sharpen(sampling::plane_ratio(ratio, plane_width, frame_width), 1.0)
}

/// The quality question: the same zoomed view, three ways.
///
/// Both planes are upgraded, then only the luma plane, so that what the
/// chroma half is worth is a difference between two pictures rather than an
/// argument. The detail metric is the mean absolute Laplacian of the luma:
/// resolving what bilinear smeared raises it, and so does ringing, which is
/// why the pictures are written out to be looked at as well.
fn delta(render: &mut Render, options: &Options) -> Fallible<()> {
    let size = options.output;
    println!(
        "\ndelta:  yaw {:.0}, pitch {:.0}, fov {:.0}, {}x{}, horizon {:?}",
        options.camera.yaw.to_degrees(),
        options.camera.pitch.to_degrees(),
        options.camera.fov.to_degrees(),
        size.width,
        size.height,
        options.horizon,
    );
    let soft = render.frame(options.camera, Sampling::Bilinear, size)?;
    let luma = render.frame(options.camera, Sampling::Luma, size)?;
    let sharp = render.frame(options.camera, Sampling::Sharp, size)?;
    println!(
        "        detail {:.3} codes bilinear, {:.3} luma only ({:+.1}%), {:.3} both planes \
         ({:+.1}%)",
        soft.detail(),
        luma.detail(),
        100.0 * (luma.detail() / soft.detail() - 1.0),
        sharp.detail(),
        100.0 * (sharp.detail() / soft.detail() - 1.0),
    );
    println!(
        "        the luma half is worth   {}",
        luma.against(&soft).report()
    );
    println!(
        "        the chroma half adds     {}",
        sharp.against(&luma).report()
    );
    println!(
        "        the two together are     {}",
        sharp.against(&soft).report()
    );
    for (picture, name) in [
        (&soft, "zoom-bilinear.png"),
        (&luma, "zoom-luma.png"),
        (&sharp, "zoom-sharp.png"),
    ] {
        picture.write(render.gpu, name)?;
    }
    println!("        wrote scratch/zoom-bilinear.png, zoom-luma.png, zoom-sharp.png");
    Ok(())
}

/// The other half of the quality question, and the harder one: what is not
/// magnified must be untouched.
///
/// "Wide field of view" is not the same question as "not magnifying", which
/// is the finding rather than a quibble. This camera delivers 3840 texels
/// across 195 degrees, so at a 2560 px window every field of view the player
/// offers still magnifies something: the middle of a 110-degree view is past
/// 1:1 and its own corners are not, because a rectilinear projection's
/// density rises towards the corner. So the claim is made per pixel instead:
/// **every pixel the upgrade moved is a pixel that is magnified**, and the
/// ratio each one sits at is read off the Rust mirror of the same map.
///
/// The horizon is let go for this one, so the mirror's `Held::default()` is
/// the pose the pass is running, and the two are the same map rather than
/// nearly the same.
fn untouched(render: &mut Render, options: &Options, lenses: &[Lens], frame: Size) -> Fallible<()> {
    let camera = Camera {
        fov: 110f32.to_radians(),
        ..options.camera
    };
    println!("\nwide:   both settings at the widest view the player offers, fov 110, horizon free");
    println!("        output      | shipped                  | both planes");
    render.scene.set_horizon(Horizon::Free);
    for width in WIDTHS {
        let size = Size::new(width, width * WINDOW.height / WINDOW.width);
        let soft = render.frame(camera, Sampling::Bilinear, size)?;
        let shipped = render.frame(camera, Sampling::default(), size)?;
        let both = render.frame(camera, Sampling::Sharp, size)?;
        println!(
            "        {:4}x{:<4}  | {:24} | {}",
            size.width,
            size.height,
            report(&shipped.against(&soft)),
            report(&both.against(&soft)),
        );
    }

    // And the claim itself, per pixel, at the window the player is used at.
    let soft = render.frame(camera, Sampling::Bilinear, options.output)?;
    let shipped = render.frame(camera, Sampling::default(), options.output)?;
    let reframe = Reframe::new(
        lenses,
        frame,
        camera,
        Held::default(),
        aspect(options.output),
        false,
        Sampling::default(),
    );
    let (mut moved, mut magnified, mut worst) = (0u64, 0u64, 0.0f32);
    for down in 0..options.output.height {
        for across in 0..options.output.width {
            let uv = [
                (across as f32 + 0.5) / options.output.width as f32,
                (down as f32 + 0.5) / options.output.height as f32,
            ];
            let ratio = lit(&reframe, uv, options.output);
            magnified += u64::from(ratio < 1.0);
            let at = 4 * (down as usize * options.output.width as usize + across as usize);
            if soft.rgba[at..at + 3] != shipped.rgba[at..at + 3] {
                moved += 1;
                worst = worst.max(ratio);
            }
        }
    }
    let pixels = u64::from(options.output.width) * u64::from(options.output.height);
    println!(
        "        of {pixels} pixels, {:.1}% are magnified and {:.1}% moved; the least magnified \
         pixel\n        that moved sits at {worst:.4} texels to the pixel, and 1.0 is where the \
         upgrade\n        switches off",
        100.0 * magnified as f64 / pixels as f64,
        100.0 * moved as f64 / pixels as f64,
    );
    render.scene.set_horizon(options.horizon);
    Ok(())
}

fn report(against: &Difference) -> String {
    match against.is_identical() {
        true => "byte for byte identical".to_owned(),
        false => against.report(),
    }
}

/// The pop question, asked the way an eye asks it: does the picture jump.
///
/// Two numbers, because two things change along a zoom sweep and only one of
/// them is the kernel. The **difference** between the settings grows as the
/// engagement grows, and a kernel switched on rather than mixed in would put
/// the whole of that difference into whichever step crossed the threshold; so
/// the largest single step in it, against how much of it there is, is what
/// says the mix is a mix. The **picture** moves too, further with a sharp
/// kernel than with a soft one, because a sharper picture has more in it to
/// move; that shows up everywhere along the sweep rather than at a place, and
/// the ratio of the two settings' steps is what says so.
fn sweep(render: &mut Render, options: &Options) -> Fallible<()> {
    let (from, to, steps) = SWEEP;
    println!(
        "\nsweep:  {steps} steps of zoom from fov {to:.0} in to {from:.0} at {}x{}, {:.2} degrees \
         a step",
        options.output.width,
        options.output.height,
        (to - from) / (steps - 1) as f32,
    );

    let mut walk: Vec<Step> = Vec::with_capacity(steps);
    let mut held: Option<(Picture, Picture)> = None;
    for step in 0..steps {
        // Inwards, so the sweep starts where nothing is engaged.
        let fov = to - (to - from) * step as f32 / (steps - 1) as f32;
        let camera = Camera {
            fov: fov.to_radians(),
            ..options.camera
        };
        let soft = render.frame(camera, Sampling::Bilinear, options.output)?;
        let sharp = render.frame(camera, Sampling::default(), options.output)?;
        let apart = sharp.against(&soft).mean;
        let moved = held.map(|(was_soft, was_sharp): (Picture, Picture)| {
            (soft.against(&was_soft).mean, sharp.against(&was_sharp).mean)
        });
        walk.push(Step { fov, apart, moved });
        held = Some((soft, sharp));
    }

    let widest = walk.iter().map(|s| s.apart).fold(0.0, f64::max);
    let arriving = walk
        .windows(2)
        .map(|pair| ((pair[1].apart - pair[0].apart).abs(), pair[1].fov))
        .fold((0.0, 0.0), |held, now| match now.0 > held.0 {
            true => now,
            false => held,
        });
    println!(
        "        the shipped setting is {:.3} codes from bilinear at the widest view and \
         {widest:.3} at the\n        narrowest. The largest single step in that is {:.3} codes, \
         at fov {:.1}, which is {:.1}% of\n        it; a kernel switched on rather than mixed in \
         would put all {widest:.3} in one step.",
        walk.first().map_or(f64::NAN, |s| s.apart),
        arriving.0,
        arriving.1,
        100.0 * arriving.0 / widest.max(f64::MIN_POSITIVE),
    );

    let mut steps_apart: Vec<(f64, f32)> = walk
        .iter()
        .filter_map(|s| {
            let (soft, sharp) = s.moved?;
            (soft > 0.0).then_some((sharp / soft, s.fov))
        })
        .collect();
    steps_apart.sort_by(|a, b| a.0.total_cmp(&b.0));
    let worst = steps_apart.last().copied().unwrap_or((f64::NAN, f32::NAN));
    println!(
        "        Step for step the picture itself moves {:.3}x as far sharp as bilinear at the \
         median\n        and {:.3}x at the worst, at fov {:.1}: a sharper picture moving, spread \
         along the whole\n        sweep rather than arriving at one field of view.",
        steps_apart[steps_apart.len() / 2].0,
        worst.0,
        worst.1,
    );
    Ok(())
}

/// One rung of the zoom sweep: how far the two settings are apart here, and
/// how far each of them moved from the rung before.
struct Step {
    fov: f32,
    apart: f64,
    moved: Option<(f64, f64)>,
}

/// The still question (issue #15): a capture is this pipeline drawn into a
/// bigger target, and nothing tells it so.
fn shot(render: &mut Render, options: &Options, lenses: &[Lens], frame: Size) -> Fallible<()> {
    let size = Size::new(
        options.shot,
        (options.shot as f32 / aspect(options.output)).round() as u32,
    );
    println!(
        "\nshot:   a still at {} px through Scene::capture, off a {} px view",
        size.width, options.output.width,
    );

    let (sent, taken) = mpsc::channel();
    render.scene.set_sampling(Sampling::default());
    render.scene.capture(Request {
        width: size.width,
        then: Box::new(move |shot| {
            let _ = sent.send(shot.map(|shot| Picture {
                rgba: shot.rgba,
                size: Size::new(shot.width, shot.height),
            }));
        }),
    });
    // The aspect the capture fits itself to is the one this redraw is drawn
    // at, so the still frames what the view frames.
    render.frame(options.camera, Sampling::default(), options.output)?;
    let still = taken.recv()??;

    let direct = render.frame(options.camera, Sampling::default(), size)?;
    let soft = render.frame(options.camera, Sampling::Bilinear, size)?;
    println!(
        "        against the same view drawn straight into a {} px target: {}",
        size.width,
        match still.against(&direct).is_identical() {
            true => "byte for byte identical".to_owned(),
            false => still.against(&direct).report(),
        }
    );
    println!(
        "        against a bilinear still of the same size: {}",
        still.against(&soft).report()
    );
    let reframe = Reframe::new(
        lenses,
        frame,
        options.camera,
        Held::default(),
        aspect(options.output),
        false,
        Sampling::default(),
    );
    println!(
        "        and it magnifies harder than the window it came off, which is what it has to \
         work\n        out for itself: {:.3} texels to the pixel at the middle of the still \
         against {:.3}\n        at the middle of the view",
        lit(&reframe, [0.5, 0.5], size),
        lit(&reframe, [0.5, 0.5], options.output),
    );
    println!(
        "        wrote {}",
        still.write(render.gpu, "zoom-shot.png")?.display()
    );
    Ok(())
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Fallible<Self> {
        let input = PathBuf::from(args.next().ok_or(USAGE)?);
        let mut options = Self {
            input,
            camera: Camera {
                fov: 25f32.to_radians(),
                ..Camera::default()
            },
            at: Cue::Index(0),
            output: WINDOW,
            shot: 3840,
            horizon: Horizon::Locked,
        };
        for arg in args {
            let (key, value) = arg.split_once('=').ok_or(USAGE)?;
            match key {
                "yaw" => options.camera.yaw = value.parse::<f32>()?.to_radians(),
                "pitch" => options.camera.pitch = value.parse::<f32>()?.to_radians(),
                "fov" => options.camera.fov = value.parse::<f32>()?.to_radians(),
                "frame" => options.at = Cue::Index(value.parse()?),
                "time" => options.at = Cue::Time(Duration::from_secs_f64(value.parse()?)),
                "size" => {
                    let width: u32 = value.parse()?;
                    options.output = Size::new(width, width * WINDOW.height / WINDOW.width);
                }
                "shot" => options.shot = value.parse()?,
                "lock" => {
                    options.horizon = match value.parse::<u32>()? {
                        0 => Horizon::Free,
                        _ => Horizon::Locked,
                    }
                }
                _ => return Err(format!("unknown argument {key}. {USAGE}").into()),
            }
        }
        Ok(options)
    }
}

const USAGE: &str = "usage: zoom <file.insv> [yaw=deg] [pitch=deg] [fov=deg] \
     [frame=n | time=seconds] [size=px] [shot=px] [lock=0]";
