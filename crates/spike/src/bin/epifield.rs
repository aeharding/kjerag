//! One session's OWN across-seam field, far-gated, written down (issue #103,
//! the epi fork).
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin epifield -- <file.insv> \
//!   seam=roll:0.795,yaw:-2.310,pitch:-0.936,cx:-3.28,cy:-11.91 \
//!   places=24 frames=6 out=scratch/epiramp/fields/jul14.txt
//! ```
//!
//! **Why a session's own and not a corpus's.** The pooled static table was
//! refused in the delivered picture: the six X4 Air flights disagree at a given
//! azimuth by 0.597 degrees at the median against a pooled amplitude of 0.229
//! rms, so the mean reconstructs no member of the population
//! (docs/research/stage9.md 10.12). One flight that happened to sit near the
//! pooled answer collapsed its delivered ramp by 89 percent, which is the
//! existence proof that a per-flight-correct field is the right thing and the
//! pooled one is the wrong number in it.
//!
//! **What is written is `read_deg`, not the term.** The file carries what this
//! session reads across the seam through the FACTORY map, per azimuth, in
//! degrees. `seam::epi_term` adds the drawn pose's own displacement to it at
//! open, exactly as it does for the pooled table. So the pooled arm and the
//! session arm differ in **one input and nothing else**, which is what makes
//! the two columns of the table comparable.
//!
//! **The far gate, and it is the smallest honest one.** Parallax on this axis
//! is one-signed - `band::Cell::metres` is `reach_m / disparity` and exists only
//! where the disparity is positive, because a negative one is not a distance -
//! so near content can only push a reading ONE WAY.
//!
//! It is applied to the **excursion** and not to the reading. A first pass takes
//! each direction's own middle; a moment whose excursion above that middle
//! implies a distance nearer than [`Options::far_m`], on this capture's own
//! baseline and by `metres`' own arithmetic, is near content and is dropped;
//! what survives is reduced again by the trimmed middle the corpus study
//! settled on.
//!
//! **Applied to the reading itself it would be nonsense**, and that is measured:
//! the factory calibration's own across-seam error reaches two and a half
//! degrees and is positive over half the ring, so an absolute gate at 60 metres
//! throws away 1829 moments of 3205 on the May-01 flight and calls a
//! calibration a hedge. The excursion is what a distance can move; the middle is
//! what the camera is.
//!
//! **What this gate cannot do**, said plainly: the camera's own term and a far
//! object's parallax are the same sign and the same axis, and a session whose
//! near content never moves would have that content in its middle. The gate
//! removes what *wanders* nearby - the wing, the lines, the pilot, the prop cage
//! swinging through a direction - and leaves the rest alone. Whether what is
//! left is camera or scene is the question the delivered table answers, not
//! this file.

use std::path::PathBuf;

use kjerag_media::Fallible;
use kjerag_meta::CalibrationSet;
use kjerag_render::{Size, band, seam};
use kjerag_spike::seam_fit;

/// How many surviving moments a direction needs before its value is believed.
///
/// Below it the direction is **identity** - zero, the picture unchanged - and
/// not its neighbour's value. A field with holes filled from next door is the
/// mechanism that made stage 5 scallop and stage 8 stripe, and it is why stage
/// 9's rules say an unmeasured direction is zero.
const MOMENTS_NEEDED: usize = 3;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    // Both halves of the capture, not the one file it is named by: a camera
    // that writes one lens per file has its seam between two paths, and a ring
    // read off one of them is a ring with one lens on it (issue #123).
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = calibration.lenses.clone();
    let files = kjerag_render::capture_set::resolve(&options.input).files;
    // The ring is the BAND's ring and not `Probe`'s default 72, so the file is
    // one entry per direction the term is looked up at and nothing here has to
    // resample anything.
    let plan = seam::Plan {
        places: options.places,
        frames: options.frames,
        probe: seam::Probe {
            patches: band::AZIMUTHS,
            ..seam::Probe::default()
        },
        ..seam::Plan::default()
    };
    println!(
        "plan:   {} places x {} frames, {} azimuths, far gate {} m",
        options.places, options.frames, plan.probe.patches, options.far_m,
    );

    let moments = seam::moments(&files, &lenses, frame, &plan)?;
    let baseline = band::baseline(&lenses);
    let mut entries = [0.0f64; band::AZIMUTHS];
    let mut evidence = [0usize; band::AZIMUTHS];
    let mut raw = 0usize;
    let mut near = 0usize;

    if moments.len() != band::AZIMUTHS {
        return Err(format!(
            "the ring came back {} long, not {}",
            moments.len(),
            band::AZIMUTHS
        )
        .into());
    }
    for (index, (_, seen)) in moments.iter().enumerate() {
        let cell = band::Ring::cell(index, baseline);
        let all: Vec<f64> = seen.iter().map(|axes| axes[1]).collect();
        raw += all.len();
        if all.len() < MOMENTS_NEEDED {
            continue;
        }
        // What a near object at the gate's own distance would add here, by
        // `Cell::metres`' arithmetic run backwards on this capture's baseline.
        let reach = (f64::from(cell.reach_m) / options.far_m).to_degrees();
        let (middle, _) = tolerated(&all);
        let far: Vec<f64> = all
            .into_iter()
            .filter(|across| {
                let keep = across - middle <= reach;
                near += usize::from(!keep);
                keep
            })
            .collect();
        if far.len() < MOMENTS_NEEDED {
            continue;
        }
        // The corpus study's own reduction, one level in: the middle of the
        // population and the moments that agree with it.
        let (middle, tolerance) = tolerated(&far);
        let kept: Vec<f64> = far
            .into_iter()
            .filter(|value| (value - middle).abs() <= tolerance)
            .collect();
        if kept.len() < MOMENTS_NEEDED {
            continue;
        }
        entries[index] = kept.iter().sum::<f64>() / kept.len() as f64;
        evidence[index] = kept.len();
    }

    // A direction whose reading departs from the ring's own smooth shape is a
    // correlation that found the wrong feature, not a camera. The factory
    // across-seam error is pose-order - one cycle of two and a half degrees
    // round this camera's ring - so five terms describe it and the residual is
    // what says which readings belong to it. `seam::tolerated`'s rule again,
    // one level out, with a floor wide enough that real structure above pose
    // order survives it.
    let wild = wild(&entries, &evidence);
    for index in &wild {
        evidence[*index] = 0;
        entries[*index] = 0.0;
    }
    let worst = entries.iter().fold(0.0f64, |worst, e| worst.max(e.abs()));
    let read = evidence.iter().filter(|n| **n > 0).count();
    println!(
        "shape:  {} direction(s) refused for departing from the ring's own five-term shape",
        wild.len(),
    );
    println!(
        "read:   {read} of {} directions have evidence; {raw} moments, {near} refused as near \
         content; worst entry {worst:.3} deg",
        band::AZIMUTHS,
    );
    if read == 0 {
        return Err("no direction of this capture's ring survived the far gate".into());
    }
    // No bound is checked here, and that is deliberate. What this file holds is
    // the FACTORY-map reading, which carries the factory calibration's own
    // across-seam error and reaches two and a half degrees on this camera. The
    // bound that matters is on the composed TERM - the reading plus the drawn
    // pose's own displacement - and `seam::epi_term` is where that composition
    // happens and where `EPI_LIMIT_RAD` refuses it.

    let mut text = format!(
        "# kjerag-spike --bin epifield ({}), one session's own across-seam field\n\
         # file: {}\n\
         # plan: {} places x {} frames, far gate {} m, {MOMENTS_NEEDED} moments needed\n\
         # read: {read} of {} directions, {raw} moments, {near} refused as near content\n\
         # UNITS: degrees, one per direction, read through the FACTORY map. The drawn pose's own\n\
         # across-seam displacement is added by `seam::epi_term` at open, not here.\n",
        env!("CARGO_PKG_VERSION"),
        options.input.display(),
        options.places,
        options.frames,
        options.far_m,
        band::AZIMUTHS,
    );
    // Two columns: the reading in degrees and how many moments are behind it.
    // The second is load-bearing and not a diagnostic - a direction with no
    // evidence must end up drawing NOTHING, and what makes that true is the
    // composed term being zero there, which only the consumer can arrange
    // (`seam::epi_term`). A file that wrote only values would have every
    // unread direction applying the whole of the pose's own displacement,
    // which is the arm that takes the band's eyes out.
    for (entry, seen) in entries.iter().zip(&evidence) {
        text.push_str(&format!("{entry:.6} {seen}\n"));
    }
    if let Some(parent) = options.out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&options.out, text)?;
    println!("wrote:  {}", options.out.display());
    Ok(())
}

/// The middle of a population and how far from it a member may sit, which is
/// `seam::tolerated`'s rule at the depth this file needs it.
///
/// Four median absolute deviations, never tighter than a tenth of a degree.
/// The same two numbers `seam::left` gates a ring with and `seam::reduced`
/// gates one azimuth's frames with, and for the same physical argument: a
/// capture's calibration does not change while it plays.
fn tolerated(values: &[f64]) -> (f64, f64) {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted[sorted.len() / 2];
    let mut spread: Vec<f64> = sorted.iter().map(|v| (v - middle).abs()).collect();
    spread.sort_by(f64::total_cmp);
    (middle, (4.0 * spread[spread.len() / 2]).max(0.10))
}

/// Which directions do not belong to the ring's own five-term shape.
///
/// The factory calibration's across-seam error is pose-order on this camera - a
/// constant, a one-cycle and a two-cycle term, the same basis `band::Along` is
/// written in - so a reading that sits far off a least-squares fit of those
/// five over the whole ring is a correlation that locked onto something else.
/// Measured on the May-01 flight: two directions read -1.86 degrees where every
/// neighbour and every other flight reads +2.36, which is the search finding a
/// different feature and not a camera.
///
/// **A detector and not a smoother.** What survives keeps its own value,
/// including whatever it says above pose order, which is the entire thing a
/// per-session field exists to carry. The tolerance is four median absolute
/// deviations of the residual, never tighter than half a degree.
fn wild(entries: &[f64; band::AZIMUTHS], evidence: &[usize; band::AZIMUTHS]) -> Vec<usize> {
    let rows: Vec<(Vec<f64>, f64)> = (0..band::AZIMUTHS)
        .filter(|index| evidence[*index] > 0)
        .map(|index| {
            let phi = index as f64 / band::AZIMUTHS as f64 * std::f64::consts::TAU;
            (band::basis(phi).to_vec(), entries[index])
        })
        .collect();
    let Some(fit) = seam::least_squares(&rows) else {
        return Vec::new();
    };
    let modelled = |index: usize| {
        let phi = index as f64 / band::AZIMUTHS as f64 * std::f64::consts::TAU;
        let basis = band::basis(phi);
        (0..5)
            .map(|term| basis[term] * fit.params[term])
            .sum::<f64>()
    };
    let left: Vec<f64> = (0..band::AZIMUTHS)
        .filter(|index| evidence[*index] > 0)
        .map(|index| entries[index] - modelled(index))
        .collect();
    let (middle, tolerance) = wide(&left);
    (0..band::AZIMUTHS)
        .filter(|index| evidence[*index] > 0)
        .filter(|index| (entries[*index] - modelled(*index) - middle).abs() > tolerance)
        .collect()
}

/// [`tolerated`] with a wider floor, for the ring rather than for one
/// direction's frames: what is being refused here is a wrong feature and not
/// an unsteady reading, and half a degree is well above anything this axis
/// carries above pose order.
fn wide(values: &[f64]) -> (f64, f64) {
    let (middle, _) = tolerated(values);
    let mut spread: Vec<f64> = values.iter().map(|v| (v - middle).abs()).collect();
    spread.sort_by(f64::total_cmp);
    (middle, (4.0 * spread[spread.len() / 2]).max(0.50))
}

struct Options {
    input: PathBuf,
    places: usize,
    frames: usize,
    far_m: f64,
    out: PathBuf,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            places: 24,
            frames: 6,
            far_m: 60.0,
            out: PathBuf::from("scratch/epifield.txt"),
        };
        let mut fit = None;
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("places", value)) => options.places = value.parse()?,
                Some(("frames", value)) => options.frames = value.parse()?,
                Some(("far", value)) => options.far_m = value.parse()?,
                Some(("out", value)) => options.out = PathBuf::from(value),
                Some(("seam", value)) => fit = Some(seam_fit(value)?),
                Some((key, _)) => return Err(format!("no argument called {key}. {USAGE}").into()),
            }
        }
        if options.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        // The pose is named so a run says which calibration it belongs beside,
        // and it is NOT applied: the ring is read through the factory map on
        // every capture, for the life of the camera, which is the only thing
        // that makes two captures' readings the same quantity
        // (`seam::along_terms`' own argument, one axis over).
        if let Some(fit) = fit {
            println!(
                "seam:   named beside roll {:+.3}, yaw {:+.3}, pitch {:+.3}, cx {:+.2}, cy {:+.2}, \
                 and NOT applied: the ring is read through the factory map",
                fit.roll_deg, fit.yaw_deg, fit.pitch_deg, fit.cx_px, fit.cy_px,
            );
        }
        Ok(options)
    }
}

const USAGE: &str = "usage: epifield <file.insv> [places=n] [frames=n] [far=metres] \
     [seam=roll:0.8,yaw:-2.3,pitch:-0.9,cx:-3.3,cy:-11.9] out=field.txt";
