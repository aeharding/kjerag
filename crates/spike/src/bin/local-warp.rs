//! Stage 9's observation-only raw-lens registration.
//!
//! `warm` follows the same rendered traversal as `step`, but only to decide
//! which physical seam contour the named view exposes.  Registration then
//! starts again from the synchronized decoded lens planes; no composited,
//! blended, colour-corrected, or warped output is matched.

use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_meta::CalibrationSet;
use kjerag_render::{Camera, Cue, Horizon, Sampling, Scene, ScenePipeline, Size};
use kjerag_spike::{FORMAT, Gpu, Render, Walk, raw_register};

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let baseline = calibration
        .lenses
        .get(1)
        .map_or([0.0; 3], |lens| lens.pose.translation_m);
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let gpu = Gpu::open()?;
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let mut scene = Scene::still(&options.input, options.start())?;
    scene.fit_seam(true);
    scene.set_horizon(if options.lock {
        Horizon::Locked
    } else {
        Horizon::Free
    });

    // Keep this rendered traversal even though its pixels are discarded: the
    // pipeline's media-time band state and held camera pose are exactly what
    // makes `warm` mean the same thing here as it does in `step`.
    let mut rendered = 0usize;
    while let Some((_, at)) = scene.frame() {
        let _ = Render {
            gpu: &gpu,
            scene: &scene,
            pipeline: &mut pipeline,
        }
        .frame(options.camera(), Sampling::default(), options.size())?;
        rendered += 1;
        if at.as_secs_f64() >= options.time || !scene.advance()? {
            break;
        }
    }
    let (_, at) = scene.frame().ok_or("no frame decoded at that instant")?;
    let map = scene
        .mapped(options.camera(), 1.0)
        .ok_or("no frame to map")?;
    let candidates = raw_register::visible_candidates(&map, options.size, options.size, baseline);
    println!(
        "played: {rendered} frame(s), ending at {:.3} s; {} visible seam candidates",
        at.as_secs_f64(),
        candidates.len()
    );

    // `Walk` is deliberately independent of `Scene`: it returns delivered
    // raw planes and cannot accidentally hand this instrument an output pixel.
    let mut walk = Walk::open(&options.input, at.as_secs_f64(), frame)?;
    let pair = walk
        .next_pair()?
        .ok_or("no synchronized raw lens pair at that instant")?;
    if (pair.at.as_secs_f64() - at.as_secs_f64()).abs() > 0.050 {
        println!(
            "warning: raw pair landed at {:.3} s, not rendered {:.3} s",
            pair.at.as_secs_f64(),
            at.as_secs_f64()
        );
    }
    match raw_register::select(&map, &pair.lenses, &candidates) {
        Ok(reading) => {
            let sigma = [
                reading.covariance_rad2[0][0].max(0.0).sqrt().to_degrees(),
                reading.covariance_rad2[1][1].max(0.0).sqrt().to_degrees(),
            ];
            println!(
                "selected: view ({}, {}), body phi {:.2} deg\nraw:      shift [perp {:.4}, epi {:.4}] deg; 1σ [{:.4}, {:.4}] deg\nquality:  r {:.4}, condition {:.2}, selector {:.4}",
                reading.candidate.view_pixel[0],
                reading.candidate.view_pixel[1],
                reading.candidate.node.phi.to_degrees(),
                reading.shift_rad[0].to_degrees(),
                reading.shift_rad[1].to_degrees(),
                sigma[0],
                sigma[1],
                reading.correlation,
                reading.condition,
                reading.score,
            );
            println!(
                "meaning: raw-lens local registration only; it neither proves a warp model nor changes the renderer."
            );
        }
        Err(reason) => println!("refused: {reason:?}; no two-axis registration was inferred"),
    }
    Ok(())
}

struct Options {
    input: PathBuf,
    time: f64,
    warm: f64,
    yaw: f64,
    pitch: f64,
    fov: f64,
    size: u32,
    lock: bool,
}
impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut out = Self {
            input: PathBuf::new(),
            time: 0.0,
            warm: 0.0,
            yaw: 90.0,
            pitch: 0.0,
            fov: 20.0,
            size: 1024,
            lock: true,
        };
        for arg in args {
            match arg.split_once('=') {
                None => out.input = PathBuf::from(arg),
                Some(("time", v)) => out.time = v.parse()?,
                Some(("warm", v)) => out.warm = v.parse()?,
                Some(("yaw", v)) => out.yaw = v.parse()?,
                Some(("pitch", v)) => out.pitch = v.parse()?,
                Some(("fov", v)) => out.fov = v.parse()?,
                Some(("size", v)) => out.size = v.parse()?,
                Some(("lock", v)) => out.lock = v.parse::<u32>()? != 0,
                Some((key, _)) => return Err(format!("no argument called {key}. {USAGE}").into()),
            }
        }
        if out.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        Ok(out)
    }
    fn start(&self) -> Cue {
        Cue::Time(Duration::from_secs_f64((self.time - self.warm).max(0.0)))
    }
    fn camera(&self) -> Camera {
        Camera {
            yaw: self.yaw.to_radians() as f32,
            pitch: self.pitch.to_radians() as f32,
            fov: self.fov.to_radians() as f32,
        }
    }
    fn size(&self) -> Size {
        Size::new(self.size, self.size)
    }
}
const USAGE: &str = "usage: local-warp <file.insv> time=seconds warm=seconds yaw=deg pitch=deg fov=deg [size=px] [lock=0]";
