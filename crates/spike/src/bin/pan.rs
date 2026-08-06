//! What the view does after a pan, with the file actually playing.
//!
//! The reproduction harness for issue #44. Everything else that has measured
//! the horizon lock drove the filter or the composition directly; this drives
//! the **player**: a real file open and playing on its own decode thread, a
//! drag through the same [`Viewpoint`] calls the shader widget makes, and
//! then frames arriving in real time while nothing touches the mouse.
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin pan -- <file.insv> from=300
//! cargo run --release -p kjerag-spike --bin pan -- <file.insv> from=300 png=1
//! ```
//!
//! `from` is where to start in seconds, `pan` is how far the drag turns the
//! view in degrees, `watch` is how many seconds to watch it afterwards, and
//! `every` is how often to read it. `png=1` writes the sequence into
//! `scratch/`, which is gitignored: these are frames of somebody's real
//! flights and this repo is public.
//!
//! ## What it reads, and why that number
//!
//! **Where the view points in the camera body's own frame.** The pilot's
//! report is that the view returns to where the camera is pointing, and that
//! is a statement about the body, so it is the body the answer is measured
//! in: the bearing of the middle of the output after the lock's own rotation
//! is undone, which is `body_from_world * camera_rotation` applied to
//! straight ahead. Zero is the camera's own nose. A drag to 60 degrees that
//! reads 60 and stays there is a view held by a camera that is not turning;
//! one that steps to zero between two reads has been reset by an event, which
//! is the defect this was written for, and telling that apart from the rest
//! is the whole point of reading it every quarter second rather than twice.
//!
//! **This number wanders on its own now, and that is the lock working.** With
//! the lock world-fixed since 2026-08-06 the view holds a direction in the
//! world, so the moment the aircraft turns, where the view sits relative to
//! its nose turns with it: on real footage this column walks by whatever the
//! flying did and does not come back. What it cannot do any more is decay
//! smoothly towards zero, which is what the heading follow used to look like
//! here, so a smooth decay is now a finding rather than the design.
//!
//! Nothing is stubbed: the frames are decoded, the orientation comes off the
//! trailer, and the composition is the one the shader is handed.

use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use kjerag_media::Fallible;
use kjerag_meta::{CalibrationSet, Filter, OrientationTrack};
use kjerag_render::{Camera, Scene, ScenePipeline, Size, Viewpoint};
use kjerag_spike::{Gpu, Offscreen};

/// Not sRGB, so a written frame is what the window shows.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The output the drag is scripted against. Only its aspect reaches the
/// camera, but a drag is in fractions of a widget and wants a real one.
const BOUNDS: Size = Size {
    width: 1600,
    height: 900,
};

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    // An instrument has no stored calibration to read: the app keeps that in
    // its own config, and this is not the app. So the seam is fitted off this
    // file, which is what every instrument did before the calibration moved
    // to the camera (issue #48).
    let mut scene = Scene::open(&options.input)?;
    scene.fit_seam(true);
    let aspect = BOUNDS.width as f32 / BOUNDS.height as f32;

    // Everything the window would build, because the picture has to be
    // rendered for the sequence to mean anything to anybody.
    let gpu = Gpu::open()?;
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let target = Offscreen::new(&gpu.device, BOUNDS, FORMAT);
    if options.png {
        std::fs::create_dir_all("scratch")?;
    }

    scene.seek(
        Duration::from_secs_f64(options.from),
        kjerag_render::Accuracy::Exact,
    );
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let held = Held {
        track: calibration.orientation(Filter::default()),
        exposure: calibration.exposure[0].clone(),
    };
    let mut viewpoint = Viewpoint::default();
    let started = Instant::now();
    settle(&mut scene, &options)?;

    println!(
        "\npanning the view {:.0} degrees, then watching it for {:.0} s with nothing \
         touching the mouse",
        options.pan, options.watch,
    );
    let before = bearing(&scene, &held, viewpoint.camera());
    drag(&mut viewpoint, &options, aspect);
    let after = bearing(&scene, &held, viewpoint.camera());
    println!(
        "the drag moved the camera's own yaw to {:.1} deg, and the view from {:.1} to \
         {:.1} deg off the camera's nose\n",
        viewpoint.camera().yaw.to_degrees(),
        before.unwrap_or(f64::NAN),
        after.unwrap_or(f64::NAN),
    );

    println!("{:>9} {:>14} {:>10}", "seconds", "off the nose", "moved");
    let mut previous = after;
    let mut shots = 0;
    let watching = Instant::now();
    while watching.elapsed().as_secs_f64() < options.watch {
        scene.pump(Instant::now());
        let now = watching.elapsed().as_secs_f64();
        let at = bearing(&scene, &held, viewpoint.camera());
        let step = match (at, previous) {
            (Some(at), Some(previous)) => wrap(at - previous),
            _ => f64::NAN,
        };
        println!(
            "{now:>9.2} {:>14.2} {step:>10.2}",
            at.unwrap_or(f64::NAN),
            step = step,
        );
        previous = at.or(previous);
        if options.png {
            let primitive = scene.primitive(viewpoint.camera());
            pipeline.prepare(&primitive, &gpu.device, &gpu.queue, aspect);
            target.render(&gpu.device, &gpu.queue, &pipeline)?;
            let pixels = target.read(&gpu.device, &gpu.queue)?;
            let name = format!("pan-{shots:02}-{:.2}s.png", now);
            target.write_png(&pixels, &PathBuf::from("scratch").join(name))?;
            shots += 1;
        }
        sleep(Duration::from_secs_f64(options.every));
    }
    println!(
        "\noff the nose is where the middle of the output looks in the camera body's own \n\
         frame, in degrees: 0 is the camera's own nose and the drag put it at {:.0}. The \n\
         lock is world-fixed, so this number walks by whatever the aircraft turns and does \n\
         not come back; what it must not do is step between two reads, which is a view \n\
         reset by an event. Elapsed {:.0} s of wall clock.",
        after.unwrap_or(f64::NAN),
        started.elapsed().as_secs_f64(),
    );
    Ok(())
}

/// Wait for the seek to land and for playback to be running, so that the drag
/// happens against a picture rather than against an empty scene.
fn settle(scene: &mut Scene, options: &Options) -> Fallible<()> {
    let until = Instant::now() + Duration::from_secs(30);
    scene.play();
    while Instant::now() < until {
        scene.pump(Instant::now());
        let landed = scene.position(Instant::now()).as_secs_f64();
        if scene.frame().is_some() && !scene.is_seeking() && landed >= options.from - 1.0 {
            println!("playing at {landed:.1} s");
            return Ok(());
        }
        sleep(Duration::from_millis(50));
    }
    Err("the file never started playing".into())
}

/// The drag the shader widget would make of a press, a sweep and a release.
///
/// Straight through [`Viewpoint`], which is what `kjerag_render::widget`'s
/// three mouse arms call and all they do: this crate cannot name an iced
/// event, and the arms themselves are covered by that file's own tests.
fn drag(viewpoint: &mut Viewpoint, options: &Options, aspect: f32) {
    const STEPS: usize = 24;
    // A pan of the whole view is a drag across most of the widget, so the
    // sweep is scaled to what the field of view makes of it: a drag from the
    // middle to the edge turns the view by about half the field of view.
    let across = (options.pan / 90.0 * 0.4).clamp(-0.45, 0.45);
    viewpoint.grab([0.5, 0.5], aspect);
    for step in 1..=STEPS {
        let along = across * step as f32 / STEPS as f32;
        viewpoint.drag_to([0.5 - along, 0.5], aspect);
    }
    viewpoint.release();
}

/// Where the middle of the output is looking in the camera body's own frame,
/// in degrees, or `None` before there is a frame to ask about.
///
/// Read off the primitive the shader is handed rather than recomposed here,
/// so that what this reports is what the window draws.
fn bearing(scene: &Scene, held: &Held, camera: Camera) -> Option<f64> {
    let (index, _) = scene.frame()?;
    let at = held.exposure.frame_time_us(index)?;
    // The lock's own rotation, undone: `body_from_world` is the inverse of
    // this, so what comes back is where the middle of the output looks in the
    // body's frame. The camera half is the shader's own `camera_rotation`,
    // which at pitch zero is a turn about the vertical.
    let ahead = [
        f64::from(camera.yaw.sin()) * f64::from(camera.pitch.cos()),
        -f64::from(camera.pitch.sin()),
        f64::from(camera.yaw.cos()) * f64::from(camera.pitch.cos()),
    ];
    let body = held.track.at(at).conjugate().rotate(ahead);
    Some(body[0].atan2(body[2]).to_degrees())
}

/// What the trailer says, which is how a bearing in the body's own frame is
/// read back out of a frame index.
struct Held {
    track: OrientationTrack,
    exposure: kjerag_meta::ExposureTrack,
}

fn wrap(degrees: f64) -> f64 {
    (degrees + 180.0).rem_euclid(360.0) - 180.0
}

struct Options {
    input: PathBuf,
    from: f64,
    pan: f32,
    watch: f64,
    every: f64,
    png: bool,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Fallible<Self> {
        let input = PathBuf::from(args.next().ok_or(USAGE)?);
        let mut options = Self {
            input,
            from: 300.0,
            pan: 60.0,
            watch: 12.0,
            every: 0.25,
            png: false,
        };
        for arg in args {
            let (key, value) = arg.split_once('=').ok_or(USAGE)?;
            match key {
                "from" => options.from = value.parse()?,
                "pan" => options.pan = value.parse()?,
                "watch" => options.watch = value.parse()?,
                "every" => options.every = value.parse()?,
                "png" => options.png = value.parse::<u32>()? != 0,
                _ => return Err(format!("unknown argument {key}. {USAGE}").into()),
            }
        }
        Ok(options)
    }
}

const USAGE: &str = "usage: pan <file.insv> [from=seconds] [pan=deg] [watch=seconds] \
     [every=seconds] [png=1]";
