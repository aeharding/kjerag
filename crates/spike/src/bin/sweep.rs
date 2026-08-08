//! A stretch of playback drawn offscreen, frame by frame, with how much the
//! picture MOVES inside the seam's fade zone measured on every step.
//!
//! The question the seam-anchor experiment asks is not what one frame looks
//! like. It is whether the handover line swims across world content while the
//! body turns under a world-locked view. That is a difference between
//! consecutive frames and nowhere else, so this plays a segment, draws every
//! frame at ONE camera, and reports the mean absolute luma step from the frame
//! before - inside the fade corridor, and in a control ring well outside it
//! that no handover touches.
//!
//! Two runs of this, one with `KJERAG_ANCHOR=1` and one without, are the
//! measurement. The control ring is what says the two runs saw the same
//! content: it has to come out equal.
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin sweep -- <file.insv> \
//!   open=65.666 oyaw=179.00 opitch=-36.97 ofov=20.00 \
//!   at=66.0 frames=100 yaw=179.00 pitch=-36.97 fov=20.00 \
//!   lock=1 w=1280 h=720 out=scratch/sweep/off
//! ```
//!
//! Nothing here opens a window.

use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_render::{Camera, Cue, Horizon, Reframe, Sampling, Scene, ScenePipeline, Size};
use kjerag_spike::{Gpu, Render, Seam};

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    println!(
        "arm:    KJERAG_ANCHOR={:?}",
        std::env::var("KJERAG_ANCHOR").unwrap_or_else(|_| "<unset>".to_owned())
    );
    println!(
        "sweep:  {} from {:.3} s, {} frames at yaw={} pitch={} fov={} ({}x{}, lock={})",
        options.input.display(),
        options.at,
        options.frames,
        options.yaw,
        options.pitch,
        options.fov,
        options.w,
        options.h,
        u8::from(options.lock),
    );

    let mut pipeline = ScenePipeline::new(&gpu.device, kjerag_spike::FORMAT);
    let mut scene = Scene::still(&options.input, Cue::Time(secs(options.open)))?;
    scene.set_horizon(match options.lock {
        true => Horizon::Locked,
        false => Horizon::Free,
    });
    options.seam.hold(&scene);

    let size = Size::new(options.w, options.h);
    let aspect = options.w as f32 / options.h as f32;
    let target = secs(options.at);
    std::fs::create_dir_all(&options.out)?;

    let mut previous: Option<Vec<f32>> = None;
    let mut rows: Vec<Row> = Vec::new();
    let mut kept = 0usize;
    while let Some((_, now)) = scene.frame() {
        let arrived = now >= target;
        let camera = match arrived {
            true => options.look(),
            false => options.opening(),
        };
        let picture = Render {
            gpu: &gpu,
            scene: &scene,
            pipeline: &mut pipeline,
        }
        .frame(camera, Sampling::default(), size)?;
        if arrived {
            let luma = picture.luma();
            let map = scene
                .mapped(camera, aspect)
                .ok_or("no map at that view")?;
            let zones = zones(&map, size);
            let row = Row {
                index: kept,
                at: now.as_secs_f64(),
                seam: previous
                    .as_ref()
                    .map(|was| step(&luma, was, &zones, Zone::Seam)),
                off: previous
                    .as_ref()
                    .map(|was| step(&luma, was, &zones, Zone::Off)),
            };
            picture.save(&gpu, &options.out.join(format!("f{kept:04}.png")))?;
            rows.push(row);
            previous = Some(luma);
            kept += 1;
            if kept >= options.frames {
                break;
            }
        }
        if !scene.advance()? {
            break;
        }
    }
    if rows.is_empty() {
        return Err(format!("never reached {:.3} s", options.at).into());
    }

    println!("\n  frame        at s     seam d/frame      off d/frame");
    let mut csv = String::from("frame,at,seam,off\n");
    for row in &rows {
        let show = |v: Option<f64>| match v {
            Some(v) => format!("{v:>15.5}"),
            None => "              -".to_owned(),
        };
        println!(
            "{:>7} {:>11.3} {} {}",
            row.index,
            row.at,
            show(row.seam),
            show(row.off)
        );
        csv.push_str(&format!(
            "{},{:.4},{},{}\n",
            row.index,
            row.at,
            row.seam.map_or(String::new(), |v| format!("{v:.6}")),
            row.off.map_or(String::new(), |v| format!("{v:.6}")),
        ));
    }
    std::fs::write(options.out.join("stats.csv"), csv)?;
    let mean = |pick: fn(&Row) -> Option<f64>| {
        let taken: Vec<f64> = rows.iter().filter_map(pick).collect();
        taken.iter().sum::<f64>() / taken.len().max(1) as f64
    };
    println!(
        "\nmean:   seam {:.5} codes a frame, control ring {:.5}, {} frames written to {}",
        mean(|r| r.seam),
        mean(|r| r.off),
        rows.len(),
        options.out.display(),
    );
    Ok(())
}

struct Row {
    index: usize,
    at: f64,
    seam: Option<f64>,
    off: Option<f64>,
}

#[derive(Clone, Copy, PartialEq)]
enum Zone {
    /// Outside every zone this run measures.
    None,
    /// Inside the drawn fade corridor: within half a crossover of the seam
    /// plane, which is where the handover is anything but 0 or 1.
    Seam,
    /// The control: far enough past the corridor that one lens has the whole
    /// picture, so nothing the handover does can reach it.
    Off,
}

/// Which zone each pixel of the view is in, in the body's own frame - which
/// is where the seam plane stands still, whatever the view is doing.
///
/// `elev` is how far past the seam plane a ray is. The corridor is half a
/// crossover either side of zero and the control is everything past it that
/// is still in the picture - which is one lens whole, so a handover that
/// moves cannot reach it, and it is populated at every field of view rather
/// than only at wide ones.
fn zones(map: &Reframe, size: Size) -> Vec<Zone> {
    let band = map.crossover_at(0.0);
    let mut out = vec![Zone::None; (size.width * size.height) as usize];
    for y in 0..size.height {
        for x in 0..size.width {
            let uv = [
                (x as f32 + 0.5) / size.width as f32,
                (y as f32 + 0.5) / size.height as f32,
            ];
            let Some(ray) = map.view_ray(uv) else {
                continue;
            };
            let body = map.body_ray(ray);
            let length = (body[0] * body[0] + body[1] * body[1] + body[2] * body[2]).sqrt();
            if length <= 0.0 {
                continue;
            }
            let elev = (body[2] / length).asin().abs();
            out[(y * size.width + x) as usize] = if elev <= 0.5 * band {
                Zone::Seam
            } else {
                Zone::Off
            };
        }
    }
    out
}

/// The mean absolute luma step from the frame before, over one zone.
fn step(now: &[f32], was: &[f32], zones: &[Zone], zone: Zone) -> f64 {
    let mut total = 0.0f64;
    let mut count = 0u64;
    for index in 0..zones.len().min(now.len()).min(was.len()) {
        if zones[index] == zone {
            total += f64::from((now[index] - was[index]).abs());
            count += 1;
        }
    }
    match count {
        0 => 0.0,
        _ => total / count as f64,
    }
}

fn secs(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds.max(0.0))
}

struct Options {
    input: PathBuf,
    open: f64,
    oyaw: f64,
    opitch: f64,
    ofov: f64,
    at: f64,
    frames: usize,
    yaw: f64,
    pitch: f64,
    fov: f64,
    w: u32,
    h: u32,
    lock: bool,
    seam: Seam,
    out: PathBuf,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            open: 0.0,
            oyaw: 90.0,
            opitch: 0.0,
            ofov: 60.0,
            at: 0.0,
            frames: 60,
            yaw: 90.0,
            pitch: 0.0,
            fov: 60.0,
            w: 1280,
            h: 720,
            lock: true,
            seam: Seam::File,
            out: PathBuf::from("scratch/sweep"),
        };
        let mut seam = String::from("file");
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("open", v)) => options.open = v.parse()?,
                Some(("oyaw", v)) => options.oyaw = v.parse()?,
                Some(("opitch", v)) => options.opitch = v.parse()?,
                Some(("ofov", v)) => options.ofov = v.parse()?,
                Some(("at", v)) => options.at = v.parse()?,
                Some(("frames", v)) => options.frames = v.parse()?,
                Some(("yaw", v)) => options.yaw = v.parse()?,
                Some(("pitch", v)) => options.pitch = v.parse()?,
                Some(("fov", v)) => options.fov = v.parse()?,
                Some(("w", v)) => options.w = v.parse()?,
                Some(("h", v)) => options.h = v.parse()?,
                Some(("lock", v)) => options.lock = v.parse::<u32>()? != 0,
                Some(("seam", v)) => seam = v.to_string(),
                Some(("out", v)) => options.out = PathBuf::from(v),
                Some((key, _)) => return Err(format!("no argument called {key}").into()),
            }
        }
        if options.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        options.seam = Seam::parse(&seam, &options.input)?;
        Ok(options)
    }

    fn opening(&self) -> Camera {
        Camera {
            yaw: self.oyaw.to_radians() as f32,
            pitch: self.opitch.to_radians() as f32,
            fov: self.ofov.to_radians() as f32,
        }
    }

    fn look(&self) -> Camera {
        Camera {
            yaw: self.yaw.to_radians() as f32,
            pitch: self.pitch.to_radians() as f32,
            fov: self.fov.to_radians() as f32,
        }
    }
}

const USAGE: &str = "usage: sweep <file.insv> open=s oyaw=deg opitch=deg ofov=deg at=s \
     frames=n yaw=deg pitch=deg fov=deg [w=px] [h=px] [lock=0] [seam=factory|file|pool] \
     [out=dir]";
