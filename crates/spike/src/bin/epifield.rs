//! One session's own across-seam field, and the walk that decides how much of
//! it the picture takes (issue #103, the epi fork; docs/research/stage9.md 11
//! and 12).
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin epifield -- <file.insv> seam=pool
//! # the guard's own control: the same field with its sign turned round
//! cargo run --release -p kjerag-spike --bin epifield -- <file.insv> seam=pool gain=-1
//! ```
//!
//! **The app's own functions and not a second copy of them.** `seam::harvest`
//! is what the player runs on every capture and `seam::walked` is what decides
//! how much of the answer reaches the picture; this binary is the two of them
//! with their working printed. A bench that re-implemented either would be
//! measuring itself.
//!
//! **`gain=` is a plant and not a setting.** A gain `k` on a field that is
//! right at 1 leaves the seam a residual of `(k - 1)` times its own
//! disagreement, so `gain=-1` is a field of the right size pointing the wrong
//! way: the walk has to refuse it, and a guard that has never been seen to
//! fire is a guard nobody has tested. It scales the composed term, which is
//! the same quantity the gain sweep in stage9.md 12.2 scaled.

use std::path::PathBuf;

use kjerag_media::Fallible;
use kjerag_meta::CalibrationSet;
use kjerag_render::{Size, band, seam};
use kjerag_spike::fit_arg;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    // Both halves of the capture, not the one file it is named by: a camera
    // that writes one lens per file has its seam between two paths, and a ring
    // read off one of them is a ring with one lens on it (issue #123).
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = calibration.lenses.clone();
    let files = kjerag_render::capture_set::resolve(&options.input).files;
    let plan = seam::epi_plan(options.places, options.frames);
    let walk = seam::epi_plan(options.walk_places, options.walk_frames);
    println!(
        "plan:   {} places x {} frames to read the field, {} x {} for each step of the walk, \
         {} directions",
        options.places,
        options.frames,
        options.walk_places,
        options.walk_frames,
        band::AZIMUTHS,
    );
    println!(
        "seam:   drawn at roll {:+.3}, yaw {:+.3}, pitch {:+.3}, cx {:+.2}, cy {:+.2}",
        options.fit.roll_deg,
        options.fit.yaw_deg,
        options.fit.pitch_deg,
        options.fit.cx_px,
        options.fit.cy_px,
    );

    let started = std::time::Instant::now();
    let field = seam::harvest(&files, &lenses, frame, &plan)?;
    println!(
        "read:   {}",
        field.describe(started.elapsed().as_secs_f64())
    );

    let term = seam::epi_term(&field, options.fit, &lenses, frame)
        .ok_or("the composed term is not a calibration, so there is nothing to walk")?;
    let planted = seam::part_of(term, options.gain);
    let worst = |table: band::Table| {
        table.entries().iter().fold(0.0f64, |worst, e| {
            worst.max(f64::from(e.abs()).to_degrees())
        })
    };
    println!(
        "term:   worst {:.3} deg, this session's reading plus the drawn pose's own across-seam \
         displacement{}",
        worst(planted),
        match options.gain == 1.0 {
            true => String::new(),
            false => format!(
                ", PLANTED at gain {:+.2}: a field of {:.3} deg pointing {}",
                options.gain,
                worst(term),
                match options.gain < 0.0 {
                    true => "the wrong way",
                    false => "its own way",
                },
            ),
        },
    );

    let (part, steps) = seam::walked(&files, &lenses, frame, options.fit, planted, &field, &walk)?;
    println!(
        "\n  {:>6} {:>7} {:>10} {:>9} {:>6}",
        "part", "read", "left deg", "worst", "kept"
    );
    for step in &steps {
        println!(
            "  {:>6.2} {:>7} {:>10.4} {:>9.3} {:>6}",
            step.part,
            step.read,
            step.left_deg,
            step.worst_deg,
            match (step.part == 0.0, step.kept) {
                (true, _) => "-",
                (_, true) => "yes",
                (_, false) => "NO",
            },
        );
    }
    println!(
        "took:   {:.0} percent of the field over {} step(s) of {}, {:.1} s all told",
        100.0 * part,
        steps.len() - 1,
        seam::EPI_STEPS,
        started.elapsed().as_secs_f64(),
    );

    if let Some(path) = &options.out {
        write(path, &options, &field)?;
        println!("wrote:  {}", path.display());
    }
    Ok(())
}

/// The field itself, one line per direction, so a harvest can be re-read
/// without a decode and two of them can be compared.
///
/// A derived table with no frame of anybody's footage in it and no capture
/// time, like `docs/research/stage9/along-seam-leftovers.csv`.
fn write(path: &PathBuf, options: &Options, field: &seam::Session) -> Fallible<()> {
    let mut text = format!(
        "# kjerag-spike --bin epifield ({}), one session's own across-seam field\n\
         # file: {}\n\
         # plan: {} places x {} frames, {} of {} directions read, {} moment(s) refused as near \
         content\n\
         # UNITS: degrees, one per direction, read through the FACTORY map. The drawn pose's own\n\
         # across-seam displacement is added by `seam::epi_term`, not here.\n",
        env!("CARGO_PKG_VERSION"),
        options.input.display(),
        options.places,
        options.frames,
        field.covered(),
        band::AZIMUTHS,
        field.near,
    );
    for (entry, seen) in field.read.iter().zip(&field.moments) {
        text.push_str(&format!("{entry:.6} {seen}\n"));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)?;
    Ok(())
}

struct Options {
    input: PathBuf,
    places: usize,
    frames: usize,
    walk_places: usize,
    walk_frames: usize,
    gain: f64,
    fit: kjerag_render::SeamFit,
    out: Option<PathBuf>,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            places: seam::EPI_PLACES,
            frames: seam::EPI_FRAMES,
            walk_places: seam::EPI_PLACES,
            walk_frames: seam::EPI_FRAMES,
            gain: 1.0,
            fit: kjerag_render::SeamFit::default(),
            out: None,
        };
        let mut fit = String::from("pool");
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("places", value)) => options.places = value.parse()?,
                Some(("frames", value)) => options.frames = value.parse()?,
                Some(("walkplaces", value)) => options.walk_places = value.parse()?,
                Some(("walkframes", value)) => options.walk_frames = value.parse()?,
                Some(("gain", value)) => options.gain = value.parse()?,
                Some(("out", value)) => options.out = Some(PathBuf::from(value)),
                Some(("seam", value)) => fit = value.to_string(),
                Some((key, _)) => return Err(format!("no argument called {key}. {USAGE}").into()),
            }
        }
        if options.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        // The pose the picture is drawn with, because the composed term is
        // this session's reading PLUS what that pose does to the across-seam
        // axis. Deferred out of the loop because `seam=pool` is resolved
        // against the file and the file may be named anywhere on the line.
        options.fit = fit_arg(&fit, Some(&options.input))?;
        Ok(options)
    }
}

const USAGE: &str = "usage: epifield <file.insv> [places=n] [frames=n] [walkplaces=n] \
     [walkframes=n] [gain=1] [seam=pool|roll:0.8,yaw:-2.3,pitch:-0.9,cx:-3.3,cy:-11.9] \
     [out=field.txt]";
