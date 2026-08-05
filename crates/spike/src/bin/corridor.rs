//! Where the seam corridor lands in the picture, frame by frame.
//!
//! The band bends content along a corridor centred on the body's own seam
//! great circle, and every question about a shimmer there starts with which
//! part of the rendered view the corridor is under. That is a question about
//! geometry rather than about pixels, so this reads it out of the shipped map
//! itself (`kjerag_render::Reframe`, the shader's own Rust twin) with no GPU
//! and no render: the angle off the seam plane is a smooth function of the
//! output pixel, and the seam is its zero contour, so a walk down its own
//! gradient from the picture's centre lands on the seam whichever way it
//! runs.
//!
//! ```sh
//! # where the seam sits in one view, and whether it holds still for 3 s
//! cargo run --release -p kjerag-spike --bin corridor -- <file.insv> \
//!   from=36.303 count=90 yaw=3.78 pitch=5.44 fov=20 lock=1
//! ```
//!
//! The horizon lock is why this is per frame rather than per view: the body
//! turns under a locked view, so the corridor walks across the picture at
//! whatever rate the pilot's heading is changing, and a column that is on the
//! seam in the first frame need not be on it in the last.
//!
//! Nothing is written anywhere. The columns are the report.

use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_render::{Camera, Cue, Horizon, Reframe, Scene, SeamFit, Size};
use kjerag_spike::seam_fit;

/// How many Newton steps the walk from the picture's centre onto the seam
/// takes. The angle is smooth and very nearly linear over a picture this
/// wide, so three is convergence and a fourth would move nothing.
const STEPS: usize = 3;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let mut scene = Scene::still(&options.input, options.at())?;
    scene.set_horizon(options.horizon());
    options.seam.hold(&scene);

    let size = options.size();
    println!(
        "\nview:   yaw {:.2}, pitch {:.2}, fov {:.2} degrees, {} px square, horizon {:?}",
        options.yaw,
        options.pitch,
        options.fov,
        size.width,
        options.horizon(),
    );
    println!(
        "\nwhere the seam runs, in pixels of the rendered picture. the point is the one \n\
         nearest the picture's centre; `tilt` is the seam's own direction, 0 for a seam \n\
         lying along the rows and 90 for one standing up the columns; `thick` is how many \n\
         pixels wide the crossover is across it, and `centre` is how far off the seam \n\
         plane the middle pixel of the picture is looking, which says which side of the \n\
         handover the view is sitting on.\n"
    );
    println!("  frame       time         x         y      tilt     thick    centre");

    let mut track = Vec::new();
    while let Some((_, at)) = scene.frame() {
        let mapped = scene
            .mapped(options.camera(), 1.0)
            .ok_or("no frame to map")?;
        let found = seam(&mapped, size);
        let line = match found {
            Some(line) => format!(
                "{:>10.1}{:>10.1}{:>10.1}{:>10.0}{:>10.2}",
                line.x,
                line.y,
                line.tilt_deg,
                f64::from(mapped.crossover_at(0.0).to_degrees()) / line.deg_per_px,
                line.centre_deg,
            ),
            None => format!("{:>10}{:>10}{:>10}{:>10}{:>10}", "-", "-", "-", "-", "-"),
        };
        println!("{:>7} {:>10.3}{line}", track.len(), at.as_secs_f64());
        track.push(found);
        if track.len() >= options.count || !scene.advance()? {
            break;
        }
    }

    let walked: Vec<Line> = track.into_iter().flatten().collect();
    if let (Some(first), Some(last)) = (walked.first(), walked.last()) {
        println!(
            "\nover {} frames the seam moved {:+.1} px across the picture and {:+.1} px \
             along it,\nwhich is {:+.2} degrees of world angle across.",
            walked.len(),
            last.x - first.x,
            last.y - first.y,
            (last.centre_deg - first.centre_deg),
        );
    }
    Ok(())
}

/// Where the seam runs at one frame, in the picture's own pixels.
#[derive(Clone, Copy)]
struct Line {
    x: f64,
    y: f64,
    /// The seam's direction in the picture, in degrees off the rows.
    tilt_deg: f64,
    /// How fast the angle off the seam plane changes across the seam, which
    /// is what a displacement in degrees is worth in pixels here.
    deg_per_px: f64,
    /// What the picture's middle pixel is looking at, in degrees off the seam
    /// plane.
    centre_deg: f64,
}

/// The signed angle off the seam plane one pixel is looking at, in degrees.
///
/// Zero on the seam, and its sign says which lens the ray belongs to, so the
/// picture's own handover is the zero contour of this function.
fn past(mapped: &Reframe, size: Size, at: [f64; 2]) -> Option<f64> {
    let uv = [
        (at[0] / f64::from(size.width)) as f32,
        (at[1] / f64::from(size.height)) as f32,
    ];
    let ray = mapped.view_ray(uv)?;
    let body = mapped.body_ray(ray);
    let length = (body[0] * body[0] + body[1] * body[1] + body[2] * body[2]).sqrt();
    match length > 0.0 {
        true => Some(f64::from((body[2] / length).asin().to_degrees())),
        false => None,
    }
}

/// The seam point nearest the picture's centre, walked onto rather than
/// scanned for.
///
/// A scan along a row finds nothing when the seam lies along the rows and
/// finds a distant, badly conditioned crossing when it nearly does, which is
/// the case this instrument was written for. The gradient does not care which
/// way the seam runs: the angle off the seam plane is smooth, so one step
/// down its own gradient lands on the contour whatever its direction.
fn seam(mapped: &Reframe, size: Size) -> Option<Line> {
    let centre = [f64::from(size.width) / 2.0, f64::from(size.height) / 2.0];
    let centre_deg = past(mapped, size, centre)?;
    let mut at = centre;
    let mut gradient = [0.0, 0.0];
    for _ in 0..STEPS {
        let here = past(mapped, size, at)?;
        gradient = [
            (past(mapped, size, [at[0] + 1.0, at[1]])? - past(mapped, size, [at[0] - 1.0, at[1]])?)
                / 2.0,
            (past(mapped, size, [at[0], at[1] + 1.0])? - past(mapped, size, [at[0], at[1] - 1.0])?)
                / 2.0,
        ];
        let steepness = gradient[0].hypot(gradient[1]);
        if steepness <= 0.0 {
            return None;
        }
        at = [
            at[0] - here * gradient[0] / (steepness * steepness),
            at[1] - here * gradient[1] / (steepness * steepness),
        ];
    }
    let steepness = gradient[0].hypot(gradient[1]);
    Some(Line {
        x: at[0],
        y: at[1],
        // The seam runs across its own gradient, so a gradient straight down
        // the columns is a seam lying along the rows.
        tilt_deg: gradient[0].atan2(gradient[1]).to_degrees().abs(),
        deg_per_px: steepness,
        centre_deg,
    })
}

/// Which seam correction the map is built with, exactly as `reframe` and
/// `band` take it, so a corridor read here is the corridor those two draw.
enum Seam {
    Factory,
    File,
    Stored(SeamFit),
}

impl Seam {
    fn hold(&self, scene: &Scene) {
        match self {
            Self::Factory => println!("seam:   factory calibration, no correction"),
            Self::File => scene.fit_seam(true),
            Self::Stored(fit) => scene.use_seam(*fit),
        }
    }
}

struct Options {
    input: PathBuf,
    from: f64,
    count: usize,
    yaw: f64,
    pitch: f64,
    fov: f64,
    size: u32,
    lock: bool,
    seam: Seam,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            from: 0.0,
            count: 1,
            yaw: 0.0,
            pitch: 0.0,
            fov: 60.0,
            size: 1024,
            lock: true,
            seam: Seam::File,
        };
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("from", value)) => options.from = value.parse()?,
                Some(("count", value)) => options.count = value.parse()?,
                Some(("yaw", value)) => options.yaw = value.parse()?,
                Some(("pitch", value)) => options.pitch = value.parse()?,
                Some(("fov", value)) => options.fov = value.parse()?,
                Some(("size", value)) => options.size = value.parse()?,
                Some(("lock", value)) => options.lock = value.parse::<u32>()? != 0,
                Some(("seam", value)) => {
                    options.seam = match value {
                        "factory" => Seam::Factory,
                        "file" => Seam::File,
                        _ => Seam::Stored(seam_fit(value)?),
                    }
                }
                Some((key, _)) => return Err(format!("no argument called {key}. {USAGE}").into()),
            }
        }
        if options.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        Ok(options)
    }

    fn at(&self) -> Cue {
        Cue::Time(Duration::from_secs_f64(self.from.max(0.0)))
    }

    fn size(&self) -> Size {
        Size::new(self.size, self.size)
    }

    fn camera(&self) -> Camera {
        Camera {
            yaw: (self.yaw.to_radians()) as f32,
            pitch: (self.pitch.to_radians()) as f32,
            fov: (self.fov.to_radians()) as f32,
        }
    }

    fn horizon(&self) -> Horizon {
        match self.lock {
            true => Horizon::Locked,
            false => Horizon::Free,
        }
    }
}

const USAGE: &str = "usage: corridor <file.insv> [from=seconds] [count=frames] [yaw=deg] \
     [pitch=deg] [fov=deg] [size=px] [lock=0] \
     [seam=factory|file|roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9]";
