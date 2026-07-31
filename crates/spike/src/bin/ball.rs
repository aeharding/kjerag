//! What the zoom out to the tiny planet looks like, costs, and steps by
//! (issue #47).
//!
//! Four questions, all of them against real footage through the app's own
//! pass, because the projection is the pass and nothing about it is visible
//! in a number alone.
//!
//! ```sh
//! cargo run --release -p kyerag-spike --bin ball -- <file.insv> \
//!   [frame=n | time=s] [yaw=deg] [pitch=deg] [size=px] [lock=1]
//! ```
//!
//! - **sweep**: one scroll from the narrowest view to the far end, a notch at
//!   a time, rendered. A projection that popped would put one step far outside
//!   the trend of the steps either side of it, so what is reported is each
//!   step against the one before it as well as the largest of them; and then
//!   the same walk at a quarter of a notch, because a step that is a step
//!   stays the same size however finely the scroll is cut and a walk through
//!   a continuous map shrinks with it.
//! - **cost**: ms/redraw across the range, interleaved. The bend costs an
//!   `atan` and a `sin_cos` per fragment and the far end costs nearly a whole
//!   sphere's worth of both lenses; the narrow range is meant to cost exactly
//!   what it cost before, which is the other half of the table.
//! - **ratio**: how far the map magnifies or minifies at each end, which is
//!   what issue #11's kernel switches on. Out wide it is minification, and
//!   the kernel has to be off.
//! - **alias**: what that minification costs, measured against the same view
//!   supersampled: 4x4 samples a pixel, box averaged, which is the picture a
//!   prefilter would be trying to reach.
//!
//! PNGs land in ./scratch/, which is gitignored: frames of real footage are
//! personal video and this repo is public.

use std::path::PathBuf;
use std::time::Duration;

use kyerag_media::Fallible;
use kyerag_meta::CalibrationSet;
use kyerag_render::{
    Camera, Cue, Held, Horizon, Reframe, Sampling, Scene, ScenePipeline, Size, sampling,
};
use kyerag_spike::{FORMAT, Gpu, Offscreen, Picture, Render, aspect};

/// The window the player's own cost numbers are taken at, and what every
/// picture here is rendered at unless `size=` says otherwise. Its shape is
/// half the answer: the far end of the zoom is where the window's own corner
/// reaches `projection::CORNER_MAX`, so a 16:9 window zooms out further than a
/// square one and both stop at the same picture.
const WINDOW: Size = Size {
    width: 2560,
    height: 1440,
};

/// Field of view per scroll notch, as a ratio: `camera::ZOOM_PER_STEP` read
/// as what a wheel does with it, which is what makes the sweep below a scroll
/// rather than an arbitrary walk.
const NOTCH: f32 = 0.12;

/// Fields of view the cost and ratio tables are read at, in degrees: the
/// default view, the threshold the bend starts at, stereographic, and two
/// past it. The far end itself is added at the window's own ceiling, which on
/// a 16:9 window is 319.
const FOVS: [f32; 5] = [90.0, 110.0, 150.0, 220.0, 280.0];

/// Where in the output the ratio is read: the middle, and most of the way out
/// towards the corner.
const PLACES: [(&str, [f32; 2]); 3] = [
    ("centre", [0.5, 0.5]),
    ("halfway", [0.5, 0.28]),
    ("rim", [0.5, 0.11]),
];

/// How many samples a supersampled pixel is averaged from, per axis.
const SUPER: u32 = 4;

/// Renders a cell of the cost table, one of which is thrown away warming up.
const REPETITIONS: usize = 40;

struct Options {
    input: PathBuf,
    camera: Camera,
    at: Cue,
    output: Size,
    horizon: Horizon,
}

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    std::fs::create_dir_all("scratch")?;

    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);

    let scene = Scene::still(&options.input, options.at)?;
    scene.set_horizon(options.horizon);
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let mut render = Render {
        gpu: &gpu,
        scene: &scene,
        pipeline: &mut pipeline,
    };

    sweep(&mut render, &options)?;
    cost(&mut render, &options)?;
    ratios(&options)?;
    alias(&mut render, &options)
}

/// The pop question, asked the way the owner asks it: scroll all the way out
/// and back, and does the picture ever jump.
///
/// Two ways round, because they answer different halves. The **trend** is the
/// step each notch makes against the step the notch before it made: a
/// projection that switched over rather than bending would put one step far
/// above its neighbours, wherever the switch was. The **refinement** is the
/// same walk cut four times finer: a discontinuity is the same size however
/// small the scroll that crosses it, and a continuous map's step falls with
/// the scroll.
fn sweep(render: &mut Render, options: &Options) -> Fallible<()> {
    let ceiling = ceiling(options.output);
    println!(
        "\nsweep:  one scroll from fov {:.0} to {:.0} at {}x{}, horizon {:?}",
        FOV_MIN,
        ceiling.to_degrees(),
        options.output.width,
        options.output.height,
        options.horizon,
    );
    println!("        fov | shrink | mean codes a step | against the step before");

    let notches = walk(render, options, ceiling, 1.0)?;
    for step in &notches {
        println!(
            "        {:3.0} | {:6.3} | {:17.3} | {}",
            step.fov.to_degrees(),
            shrink(step.fov),
            step.moved,
            match step.against {
                Some(ratio) => format!("{ratio:5.2}x"),
                None => "     -".to_owned(),
            },
        );
    }

    let largest = pick(&notches, |step| step.moved);
    let jumpiest = pick(&notches, |step| step.against.unwrap_or(0.0));
    let flat = pick(&notches, |step| match shrink(step.fov) == 1.0 {
        true => step.against.unwrap_or(0.0),
        false => 0.0,
    });
    println!(
        "        the largest single step is {:.3} codes, at fov {:.0}.\n        \
         The step that grows most against the one before it grows {:.2}x, at fov {:.0}; \
         inside\n        the flat range, which is the range this change did not touch, the \
         same number is\n        {:.2}x at fov {:.0}. The bend starts at 110 and \
         stereographic is 220.",
        largest.0,
        largest.1.to_degrees(),
        jumpiest.0,
        jumpiest.1.to_degrees(),
        flat.0,
        flat.1.to_degrees(),
    );

    // Four times finer, because a coarse walk can hide a jump inside a step
    // it was going to make anyway. What is read off it is the growth again
    // rather than the size: mean absolute difference saturates once two
    // pictures have moved past each other's detail, so a quarter of a scroll
    // is not a quarter of the codes and never was.
    let quarter = walk(render, options, ceiling, 0.25)?;
    let finest = pick(&quarter, |step| step.against.unwrap_or(0.0));
    println!(
        "        at a quarter of a notch the largest growth is {:.2}x, at fov {:.0}.",
        finest.0,
        finest.1.to_degrees(),
    );
    Ok(())
}

/// The narrowest view the player offers, in degrees: `camera::FOV_MIN`.
const FOV_MIN: f32 = 20.0;

/// One walk out along the zoom, `share` of a scroll notch at a time, and what
/// each step did to the picture.
fn walk(render: &mut Render, options: &Options, ceiling: f32, share: f32) -> Fallible<Vec<Step>> {
    let step = (NOTCH * share).exp();
    let mut fov = FOV_MIN.to_radians();
    let mut walked = Vec::new();
    let mut held: Option<Picture> = None;
    while fov < ceiling {
        let camera = Camera {
            fov: fov.min(ceiling),
            ..options.camera
        };
        let now = render.frame(camera, Sampling::default(), options.output)?;
        if let Some(before) = held {
            let moved = now.against(&before).mean;
            let against = walked
                .last()
                .map(|last: &Step| moved / last.moved.max(f64::MIN_POSITIVE));
            walked.push(Step {
                fov: camera.fov,
                moved,
                against,
            });
        }
        held = Some(now);
        fov *= step;
    }
    Ok(walked)
}

/// One notch of the sweep: where it landed, how far the picture moved to get
/// there, and how that compares with the notch before it.
struct Step {
    fov: f32,
    moved: f64,
    against: Option<f64>,
}

fn pick(walked: &[Step], of: impl Fn(&Step) -> f64) -> (f64, f32) {
    walked
        .iter()
        .map(|step| (of(step), step.fov))
        .fold((0.0, f32::NAN), |held, now| match now.0 > held.0 {
            true => now,
            false => held,
        })
}

/// What the bend costs the pass, with nothing else in the measurement.
///
/// A still frame, so nothing decodes and no clock runs, and the fields of
/// view are interleaved render by render so that a box which throttles
/// throttles all of them together. The **least** of the repetitions is the
/// pass with nothing else on the machine and the median beside it says how
/// much else there was.
///
/// Read across the rows and not against another day's numbers: every cell of
/// this table moves together with whatever clock state the box is in, by a
/// factor of three between sessions on this one, and it is the difference
/// between the fields of view that the interleaving makes trustworthy.
fn cost(render: &mut Render, options: &Options) -> Fallible<()> {
    let target = Offscreen::new(&render.gpu.device, options.output, FORMAT);
    let fovs = fovs(options.output);
    println!(
        "\ncost:   the pass alone at {}x{}, {REPETITIONS} renders a cell, interleaved. \
         ms/redraw, least (median)",
        options.output.width, options.output.height,
    );

    let mut runs = vec![Vec::new(); fovs.len()];
    for repetition in 0..REPETITIONS {
        for (slot, fov) in fovs.iter().enumerate() {
            let camera = Camera {
                fov: *fov,
                ..options.camera
            };
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

    println!("        fov | shrink | ms/redraw");
    for (fov, took) in fovs.iter().zip(runs.iter_mut()) {
        took.sort_by(f64::total_cmp);
        println!(
            "        {:3.0} | {:6.3} | {:5.2} ({:5.2})",
            fov.to_degrees(),
            shrink(*fov),
            took[0],
            took[took.len() / 2],
        );
    }
    Ok(())
}

/// How far the map magnifies at each end of the range, and what the sampling
/// upgrade does about it.
///
/// No pixels in this one: it is the model's own Jacobian, which is what the
/// shader reads off the hardware's quad derivatives per fragment. Under 1 is
/// magnifying, which is what issue #11's kernel is for; over it the picture
/// has more texels than the output has pixels and bilinear is what the
/// upgrade falls back to.
fn ratios(options: &Options) -> Fallible<()> {
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    println!(
        "\nratio:  delivered-frame texels per output pixel at {}x{}, and how far the luma \
         kernel\n        engages there. Under 1 is magnifying.",
        options.output.width, options.output.height,
    );
    println!("        fov | centre           halfway          rim");

    for fov in fovs(options.output) {
        let reframe = Reframe::new(
            &calibration.lenses,
            frame,
            Camera {
                fov,
                ..options.camera
            },
            Held::default(),
            aspect(options.output),
            false,
            Sampling::default(),
        );
        let cell = |uv| {
            let ratio = lit(&reframe, uv, options.output);
            match ratio.is_finite() {
                true => format!(
                    "{ratio:7.2} ({:4.2})",
                    sampling::sharpen(
                        sampling::plane_ratio(ratio, frame.width as f32, frame.width as f32),
                        1.0,
                    )
                ),
                false => "  no ray     ".to_owned(),
            }
        };
        println!(
            "        {:3.0} | {}  {}  {}",
            fov.to_degrees(),
            cell(PLACES[0].1),
            cell(PLACES[1].1),
            cell(PLACES[2].1),
        );
    }
    Ok(())
}

/// The ratio at one place of the output, from whichever lens has the ray.
fn lit(reframe: &Reframe, uv: [f32; 2], output: Size) -> f32 {
    let blend = reframe.blend(reframe.view_ray(uv));
    let lens = (0..blend.weights.len())
        .max_by(|a, b| blend.weights[*a].total_cmp(&blend.weights[*b]))
        .unwrap_or(0);
    reframe.texels_per_pixel(lens, uv, output)
}

/// What the minification at the wide end costs, against the picture a
/// prefilter would be trying to reach.
///
/// One output pixel out there covers tens of source texels and the sampler
/// reads four of them, so what it writes depends on which four it landed on:
/// that is the shimmer a moving picture would show. Rendering the same view
/// at [`SUPER`] times the size and box averaging it back down is the same
/// pixel with its whole footprint in it, and the difference between the two
/// is the aliasing, in codes. The narrow end of the range is the control:
/// whatever number it answers is what this measurement calls nothing.
fn alias(render: &mut Render, options: &Options) -> Fallible<()> {
    println!(
        "\nalias:  one pass against {SUPER}x{SUPER} samples a pixel, box averaged, at {}x{}",
        options.output.width, options.output.height,
    );
    println!("        fov | against supersampled");

    for fov in fovs(options.output) {
        let camera = Camera {
            fov,
            ..options.camera
        };
        let plain = render.frame(camera, Sampling::default(), options.output)?;
        let dense = render.frame(
            camera,
            Sampling::default(),
            Size::new(options.output.width * SUPER, options.output.height * SUPER),
        )?;
        let against = plain.against(&boxed(&dense, SUPER));
        // Over the pixels that moved rather than over the frame: a pixel
        // the two agree on to the code has nothing in it to alias and would
        // divide the number down for the wrong reason.
        println!(
            "        {:3.0} | {:6.3} codes over the {:5.2}% that moved, {} worst",
            fov.to_degrees(),
            against.mean * against.pixels as f64 / against.moved.max(1) as f64,
            100.0 * against.moved as f64 / against.pixels as f64,
            against.worst,
        );
        if fov
            == *fovs(options.output)
                .last()
                .expect("the far end is in the table")
        {
            plain.write(render.gpu, "planet-plain.png")?;
            boxed(&dense, SUPER).write(render.gpu, "planet-supersampled.png")?;
            println!("        wrote scratch/planet-plain.png, planet-supersampled.png");
        }
    }
    Ok(())
}

/// A picture averaged down by a whole number of samples per axis.
fn boxed(dense: &Picture, by: u32) -> Picture {
    let size = Size::new(dense.size.width / by, dense.size.height / by);
    let mut rgba = Vec::with_capacity((size.width * size.height * 4) as usize);
    for down in 0..size.height {
        for across in 0..size.width {
            for channel in 0..4 {
                let mut total = 0u32;
                for dy in 0..by {
                    for dx in 0..by {
                        let at = ((down * by + dy) * dense.size.width + across * by + dx) * 4;
                        total += u32::from(dense.rgba[(at + channel) as usize]);
                    }
                }
                rgba.push((total / (by * by)) as u8);
            }
        }
    }
    Picture { rgba, size }
}

/// The fields of view the tables are read at: the fixed ones, and the tiny
/// planet at this window's own ceiling.
fn fovs(output: Size) -> Vec<f32> {
    let mut fovs: Vec<f32> = FOVS.iter().map(|fov| fov.to_radians()).collect();
    fovs.push(ceiling(output));
    fovs
}

/// The far end of the zoom for this window, read off the camera rather than
/// worked out here: `Camera::zoom` clamps to it, so scrolling out forever
/// lands on it.
fn ceiling(output: Size) -> f32 {
    let mut camera = Camera::default();
    for _ in 0..200 {
        camera.zoom(-1.0, aspect(output));
    }
    camera.fov
}

/// How much of a real angle the flat frame sees at this field of view, which
/// is `projection::Screen::shrink` read back from outside the crate: 1 is the
/// plain perspective view, 1/2 is stereographic.
fn shrink(fov: f32) -> f32 {
    (110f32.to_radians() / fov).min(1.0)
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Fallible<Self> {
        let input = PathBuf::from(args.next().ok_or(USAGE)?);
        let mut camera = Camera {
            pitch: -40f32.to_radians(),
            ..Camera::default()
        };
        let mut at = Cue::Index(0);
        let mut output = WINDOW;
        let mut horizon = Horizon::Free;

        for arg in args {
            let (key, value) = arg.split_once('=').ok_or(USAGE)?;
            match key {
                "yaw" => camera.yaw = value.parse::<f32>()?.to_radians(),
                "pitch" => camera.pitch = value.parse::<f32>()?.to_radians(),
                "frame" => at = Cue::Index(value.parse()?),
                "time" => at = Cue::Time(Duration::from_secs_f64(value.parse()?)),
                "size" => {
                    let width: u32 = value.parse()?;
                    output = Size::new(width, width * WINDOW.height / WINDOW.width);
                }
                "lock" => {
                    horizon = match value.parse::<u32>()? {
                        0 => Horizon::Free,
                        _ => Horizon::Locked,
                    }
                }
                _ => return Err(format!("unknown argument {key}. {USAGE}").into()),
            }
        }
        Ok(Self {
            input,
            camera,
            at,
            output,
            horizon,
        })
    }
}

const USAGE: &str = "usage: ball <file.insv> [yaw=deg] [pitch=deg] \
     [frame=n | time=seconds] [size=px] [lock=1]";
