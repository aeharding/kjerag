//! The owner's own picture, reproduced offscreen: open at one view, PLAY to a
//! named instant, then draw the view he was on when he complained, and say
//! what the live across-seam field is holding at the azimuths that are in it.
//!
//! ```sh
//! # the ghost A/B's down1 arm, played through to his seek point
//! cargo run --release -p kjerag-spike --bin look -- <file.insv> \
//!   open=65.666 oyaw=179.00 opitch=-36.97 ofov=20.00 \
//!   at=79.880 yaw=-179.60 pitch=-2.30 fov=38.24 \
//!   lock=1 seam=pool w=1920 h=1080 out=scratch/look/field.png
//! ```
//!
//! The arm is the environment's, exactly as `ghost-ab.sh` sets it: this calls
//! neither `use_ghost` nor `plant_ghost`, so `KJERAG_GHOST=field` is on and
//! unset is `main`'s picture. Nothing here opens a window.

use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_render::{AZIMUTHS, Camera, Cue, Horizon, KEEP, Reframe, Sampling, Scene, ScenePipeline, Size,
                    Table};
use kjerag_spike::{Gpu, Render, Seam};

/// How many places along the seam's own centre line the report reads.
const PROBES: usize = 25;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    println!(
        "arm:    KJERAG_GHOST={:?}",
        std::env::var("KJERAG_GHOST").unwrap_or_else(|_| "<unset>".to_owned())
    );
    println!(
        "open:   {} at {:.3} s, yaw={} pitch={} fov={}",
        options.input.display(),
        options.open,
        options.oyaw,
        options.opitch,
        options.ofov
    );
    println!(
        "look:   at {:.3} s, yaw={} pitch={} fov={}  ({}x{})",
        options.at, options.yaw, options.pitch, options.fov, options.w, options.h
    );

    let mut pipeline = ScenePipeline::new(&gpu.device, kjerag_spike::FORMAT);
    let mut scene = Scene::still(&options.input, Cue::Time(secs(options.open)))?;
    scene.set_horizon(match options.lock {
        true => Horizon::Locked,
        false => Horizon::Free,
    });
    options.seam.hold(&scene);

    let size = Size::new(options.w, options.h);
    let target = secs(options.at);
    let mut frames = 0usize;
    let mut last = Duration::ZERO;
    let mut drawn = None;
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
        frames += 1;
        last = now;
        if arrived {
            drawn = Some(picture);
            break;
        }
        if !scene.advance()? {
            break;
        }
    }
    let Some(picture) = drawn else {
        return Err(format!("never reached {:.3} s (stopped at {:.3})", options.at, last.as_secs_f64()).into());
    };
    println!(
        "\nplayed:  {frames} frames, {:.3} s to {:.3} s of media ({:.3} s of accumulation)",
        options.open,
        last.as_secs_f64(),
        last.as_secs_f64() - options.open,
    );
    if let Some(path) = &options.out {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        picture.save(&gpu, path)?;
        println!("wrote:   {}", path.display());
    }

    let field = pipeline
        .ghost()
        .map(kjerag_render::Ghost::table)
        .unwrap_or(Table::REST);
    report_field(&field);

    let aspect = options.w as f32 / options.h as f32;
    let Some(plain) = scene.mapped(options.look(), aspect) else {
        return Err("no map at that view".into());
    };
    let live = plain.with_epi(field);
    let (along, cells) = pipeline.band_state(&gpu.device, &gpu.queue)?;
    // The OTHER axis, which the two arms do not hold fixed: the band measures
    // through the field, so what it reports along the seam is a different
    // number on each arm, and lens 1 takes that one whole as well.
    println!("\nthe band's along-seam fit at the end of the run (the same lens-1-whole channel):");
    println!("  {along:?}");
    let live_cells = cells.iter().filter(|cell| cell.confidence >= KEEP).count();
    println!("  cells above KEEP: {live_cells} of {AZIMUTHS}");
    grid(&plain, &live, &cells, size);
    Ok(())
}

fn secs(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds.max(0.0))
}

/// What the field holds, direction by direction, and how sharply it varies
/// between neighbours - which is the quantity a displacement that is applied
/// per ray stretches the picture by.
fn report_field(field: &Table) {
    let entries = field.entries();
    let drawn = entries.iter().filter(|v| **v != 0.0).count();
    println!("\nthe field, {AZIMUTHS} directions, {drawn} of them nonzero:");
    println!("   phi deg      field deg   step to next deg");
    for index in 0..AZIMUTHS {
        let next = entries[(index + 1) % AZIMUTHS];
        println!(
            "{:>10.2} {:>14.4} {:>18.4}",
            index as f64 / AZIMUTHS as f64 * 360.0,
            f64::from(entries[index].to_degrees()),
            f64::from((next - entries[index]).to_degrees()),
        );
    }
}

/// What the field does to this view, across it: which body direction each
/// pixel is looking down, what the field holds there, and what that displaces
/// the picture by.
///
/// `field deg` is the entry the ray's own azimuth lands on. `dx px`/`dy px` is
/// what that displacement moves lens 1's sample by, in view pixels, measured
/// by differencing the map WITH the field against the same map without it and
/// resolving the answer onto the view's own two pixel axes - so it is the
/// picture's displacement and not a conversion of an angle.
///
/// `w0`/`w1` are the two lenses' weights at that pixel, so a reader can see
/// whether the picture there is one lens, the other, or the handover between
/// them. `elev` is how far past the seam plane the ray is: the corridor is
/// `+/-` half the crossover about zero.
fn grid(plain: &Reframe, live: &Reframe, cells: &[kjerag_render::Cell], size: Size) {
    println!(
        "\nthe view, {PROBES} columns at three rows. `elev` is degrees past the seam plane, so \n\
         the handover is within +/-{:.2} of zero. `dx px`/`dy px` is what the field alone \n\
         displaces lens 1's sample by, in view pixels; `d/dcol` is how much dy changes from the \n\
         column before, which is the STRETCH a per-ray displacement adds.",
        0.5 * f64::from(plain.crossover_at(0.0).to_degrees()),
    );
    for share in [0.25f32, 0.5, 0.75] {
        let y = (share * size.height as f32) as u32;
        println!(
            "\n  row {y} ({}% down the frame)",
            (share * 100.0) as u32
        );
        println!(
            "     col   elev deg    phi deg    field deg      dx px      dy px    d/dcol px      w0     w1   band deg   conf"
        );
        let mut previous: Option<f64> = None;
        for index in 0..PROBES {
            let x = ((index as f32 + 0.5) / PROBES as f32 * size.width as f32) as u32;
            let uv = [
                (x as f32 + 0.5) / size.width as f32,
                (y as f32 + 0.5) / size.height as f32,
            ];
            let Some(ray) = plain.view_ray(uv) else {
                println!("{x:>8}   (no ray)");
                continue;
            };
            let body = plain.body_ray(ray);
            let reach = body[0].hypot(body[1]);
            let length = (body[0] * body[0] + body[1] * body[1] + body[2] * body[2]).sqrt();
            let elev = f64::from((body[2] / length).asin().to_degrees());
            let phi = f64::from(body[1].atan2(body[0]).to_degrees()).rem_euclid(360.0);
            let field_deg =
                f64::from((live.epi().at(body[0] / reach, body[1] / reach)).to_degrees());
            let with = live.tabled(1, ray);
            let without = plain.tabled(1, ray);
            let moved: [f32; 3] = std::array::from_fn(|axis| with[axis] - without[axis]);
            let (dx, dy) = onto_pixels(plain, x, y, size, moved);
            let step = match previous.replace(dy) {
                Some(before) => format!("{:>12.2}", dy - before),
                None => format!("{:>12}", "-"),
            };
            let cell = (phi / 360.0 * AZIMUTHS as f64).floor() as usize % AZIMUTHS;
            let reading = plain.reading_at(ray, cells, kjerag_render::Along::default());
            let weights = live.blend_bent(ray, reading).weights;
            println!(
                "{x:>8} {elev:>10.3} {phi:>10.2} {field_deg:>12.4} {dx:>10.2} {dy:>10.2} {step} \
                 {:>7.3} {:>6.3} {:>10.4} {:>6.2}",
                weights[0],
                weights[1],
                f64::from(cells[cell].disparity.to_degrees()),
                cells[cell].confidence,
            );
        }
    }
}

/// A displacement of a view-frame ray, resolved onto the view's own pixel
/// axes at one pixel: how many pixels right and how many pixels down the
/// content that was drawn at that pixel has moved to.
fn onto_pixels(map: &Reframe, x: u32, y: u32, size: Size, moved: [f32; 3]) -> (f64, f64) {
    let uv = |dx: f32, dy: f32| {
        [
            (x as f32 + 0.5 + dx) / size.width as f32,
            (y as f32 + 0.5 + dy) / size.height as f32,
        ]
    };
    let (Some(here), Some(right), Some(down)) = (
        map.view_ray(uv(0.0, 0.0)),
        map.view_ray(uv(1.0, 0.0)),
        map.view_ray(uv(0.0, 1.0)),
    ) else {
        return (f64::NAN, f64::NAN);
    };
    let unit = |v: [f32; 3]| {
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / n, v[1] / n, v[2] / n]
    };
    let here = unit(here);
    let gx: [f32; 3] = {
        let r = unit(right);
        std::array::from_fn(|a| r[a] - here[a])
    };
    let gy: [f32; 3] = {
        let d = unit(down);
        std::array::from_fn(|a| d[a] - here[a])
    };
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    // The displacement is small and the two pixel axes are near enough
    // orthogonal that a projection onto each is the answer; both are printed
    // so a reader can see if one of them is doing all the work.
    (
        f64::from(dot(moved, gx) / dot(gx, gx)),
        f64::from(dot(moved, gy) / dot(gy, gy)),
    )
}

struct Options {
    input: PathBuf,
    open: f64,
    oyaw: f64,
    opitch: f64,
    ofov: f64,
    at: f64,
    yaw: f64,
    pitch: f64,
    fov: f64,
    w: u32,
    h: u32,
    lock: bool,
    seam: Seam,
    out: Option<PathBuf>,
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
            yaw: 90.0,
            pitch: 0.0,
            fov: 60.0,
            w: 1920,
            h: 1080,
            lock: true,
            seam: Seam::File,
            out: None,
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
                Some(("yaw", v)) => options.yaw = v.parse()?,
                Some(("pitch", v)) => options.pitch = v.parse()?,
                Some(("fov", v)) => options.fov = v.parse()?,
                Some(("w", v)) => options.w = v.parse()?,
                Some(("h", v)) => options.h = v.parse()?,
                Some(("lock", v)) => options.lock = v.parse::<u32>()? != 0,
                Some(("seam", v)) => seam = v.to_string(),
                Some(("out", v)) => options.out = Some(PathBuf::from(v)),
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

const USAGE: &str = "usage: look <file.insv> open=s oyaw=deg opitch=deg ofov=deg at=s yaw=deg \
     pitch=deg fov=deg [w=px] [h=px] [lock=0] [seam=factory|file|pool] [out=path.png]";
