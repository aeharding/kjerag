//! The **epipolar** axis of the seam, read per moment per azimuth across the
//! corpus, and asked the one question a static correction turns on: does its
//! structure reproduce between flights of one camera (issue #103, the epi
//! fork).
//!
//! ```sh
//! # the six X4 Air flights, the reduction the along study settled on
//! cargo run --release -p kjerag-spike --bin epi -- scratch/epi/corpus/x4-*.csv \
//!   middle=trimmed seam=roll:0.795,yaw:-2.310,pitch:-0.936,cx:-3.28,cy:-11.91
//! # the control column: what the estimator #155 used would have said
//! cargo run --release -p kjerag-spike --bin epi -- scratch/epi/corpus/x4-*.csv \
//!   middle=mean seam=<the same pose>
//! ```
//!
//! **This is `--bin corpus`, one axis over.** Same dumps, same reduction, same
//! held-out arithmetic; what changes is which column of a reading is the
//! observation. `--bin corpus` takes `along_deg` and the pose's along-seam
//! shift; this takes `across_deg` and the pose's across-seam shift, which is
//! [`seam::moved`]'s second component.
//!
//! **Why the axis needs its own instrument and not a flag.** Along the seam a
//! leftover is the camera by construction, because no distance can reach that
//! axis. Across it a leftover is the camera *plus the scene*: parallax lives
//! here and reaches it one-signed at every azimuth
//! (docs/research/seam-two-axis.md 1). So every table below carries a sign
//! column, and a positive answer here has to survive the question "is this a
//! near field" in a way an along-seam answer never had to.
//!
//! **One fixed pose for every capture of a camera**, never `seam=file`, for
//! `--bin corpus`'s reason: a fit off a capture's own frames absorbs that
//! capture's scene into the pose, and two such fits do not leave the same
//! quantity behind. The pose enters this axis as one number per azimuth, the
//! same number in every capture of a camera, so every difference and every
//! held-out residual below is **invariant** to which pose is named; only the
//! `factory`/`under pose` columns move. `pose=` sweeps a second one to show
//! that.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kjerag_media::Fallible;
use kjerag_meta::CalibrationSet;
use kjerag_render::{Leftover, SeamFit, Size, seam};
use kjerag_spike::seam_fit;
use kjerag_spike::settled::{self, At, Middle};

/// Which of the seam's two axes is under study.
///
/// The along-seam arm is not a feature, it is **the control**. A negative
/// result on this axis is worth nothing from an instrument that could not
/// have found a positive one, and the along-seam field of the same corpus is
/// a positive this pipeline is known to have to find: reduced and sampled
/// this way it reproduces on 18 of 18 pairs and predicts a held-out flight to
/// 0.021 degrees (docs/research/stage9.md 4.5). So every table below is run on
/// both axes off the same dumps, and the along column is what says the
/// arithmetic works.
#[derive(Clone, Copy, PartialEq)]
enum Axis {
    /// Across the seam: `Ring::epi`, `Cell::disparity`, where parallax lives.
    Epi,
    /// Along the seam: `Ring::perp`, `Cell::off_epi`, where nothing but the
    /// camera can reach.
    Along,
}

impl Axis {
    /// Which component of [`seam::moved`] this axis is.
    fn column(self) -> usize {
        match self {
            Self::Epi => 1,
            Self::Along => 0,
        }
    }

    /// This axis's reading out of a reduced direction, in degrees, with no
    /// pose off it.
    fn of(self, at: &At) -> f64 {
        match self {
            Self::Epi => at.across,
            Self::Along => at.along,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Epi => "ACROSS the seam (epipolar)",
            Self::Along => "ALONG the seam (the control axis)",
        }
    }
}

/// How many harmonic orders the shape ladder reports.
const ORDERS: usize = 8;

/// The moment counts the density sweep thins to. The along study's threshold
/// was about ten readings per azimuth and its own instrument sampled two, so
/// the list has to straddle both.
const DEPTHS: [usize; 6] = [12, 24, 60, 120, 300, 0];

/// The harmonic orders the held-out ladder reports, as term counts:
/// `1 + 2 * order`. Order 2 is the five terms `band::Along` fits per session.
const LADDER: [usize; 6] = [0, 1, 2, 3, 5, 7];

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let mut captures = Vec::new();
    for path in &options.inputs {
        captures.push(Capture::read(path, &options)?);
    }
    if captures.is_empty() {
        return Err(USAGE.into());
    }
    report(&mut captures, &options)
}

// ------------------------------------------------------------ one capture

/// One capture's epi ring, reduced, and what a pose leaves on it.
struct Capture {
    name: String,
    lenses: Vec<kjerag_meta::Lens>,
    frame: Size,
    ring: Vec<seam::Where>,
    rows: Vec<settled::Row>,
    /// Every azimuth this capture answered, reduced, with no pose off it.
    read: Vec<At>,
    /// The same with the pose taken off and the plausibility gate applied;
    /// `At::left` holds the **epi** leftover, in degrees.
    left: Vec<At>,
    before: f64,
    after: f64,
    refused: usize,
    tolerance: f64,
    moments: usize,
    /// Source pixels per degree across the seam, median round the ring.
    scale: f64,
}

impl Capture {
    fn read(path: &Path, options: &Options) -> Fallible<Self> {
        let mut dump = settled::load(path)?;
        dump.rows = thinned(dump.rows, options.moments);
        plant(&mut dump.rows, options);
        let calibration = CalibrationSet::from_insv(&dump.source)?;
        let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
        let ring = seam::ring(dump.patches);
        let read = match options.stages {
            None => settled::field(&dump.rows, 0.0, f64::INFINITY, options.middle, false),
            Some((stages, quantile)) => staged(&dump.rows, stages, quantile, options),
        };
        if read.is_empty() {
            return Err(format!("{} holds no readings", path.display()).into());
        }
        let moments = moments_in(&dump.rows);
        let lenses = calibration.lenses.clone();
        let scale = scale_of(&seam::mapped(&lenses, frame), &ring, options.axis);
        Ok(Self {
            name: short(path),
            before: seam::rms(read.iter().map(|at| options.axis.of(at))),
            ring,
            lenses,
            frame,
            rows: dump.rows,
            read,
            left: Vec::new(),
            after: f64::NAN,
            refused: 0,
            tolerance: f64::NAN,
            moments,
            scale,
        })
    }

    /// What one azimuth of this capture's ring would move by if lens 1 were
    /// turned by `fit`, on the axis under study, in degrees. One number per
    /// azimuth and the same number in every capture of a camera.
    fn shift(&self, fit: &SeamFit, axis: Axis) -> Vec<Option<f64>> {
        let base = seam::mapped(&self.lenses, self.frame);
        let corrected = seam::mapped(&fit.applied(&self.lenses), self.frame);
        self.ring
            .iter()
            .map(|at| seam::moved(&base, &corrected, 1, at).map(|axes| axes[axis.column()]))
            .collect()
    }

    /// The pose taken off this capture's epi ring, and the gate applied to
    /// what is left.
    fn subtract(&mut self, fit: &SeamFit, gate: bool, axis: Axis) {
        let shift = self.shift(fit, axis);
        let mut moved: Vec<At> = self
            .read
            .iter()
            .filter_map(|at| {
                Some(At {
                    left: axis.of(at) + (*shift.get(at.index)?)?,
                    ..*at
                })
            })
            .collect();
        self.after = seam::rms(moved.iter().map(|at| at.left));
        if gate {
            let (kept, tolerance) = settled::gated(&settled::leftovers(&moved));
            let survived: Vec<f32> = kept.iter().map(|l| l.phi).collect();
            let before = moved.len();
            moved.retain(|at| survived.contains(&at.leftover().phi));
            self.refused = before - moved.len();
            self.tolerance = f64::from(tolerance.to_degrees());
            self.after = seam::rms(moved.iter().map(|at| at.left));
        }
        self.left = moved;
    }

    /// The same reduction over one stretch of the session only, with the same
    /// pose off it and no gate: the within-flight arm.
    fn stretch(&self, from: f64, to: f64, options: &Options) -> Vec<At> {
        let shift = self.shift(&options.seam, options.axis);
        let field = settled::field(&self.rows, from, to, options.middle, false)
            .into_iter()
            .filter_map(|at| {
                Some(At {
                    left: options.axis.of(&at) + (*shift.get(at.index)?)?,
                    ..at
                })
            })
            .collect();
        // The same gate the cross-flight rings pass, or the two tables are not
        // the same statistic and the within-flight one is the looser of them.
        gate_of(field, options.gate)
    }
}

/// One direction's session reduced in two steps instead of one: the middle of
/// each of `stages` equal stretches of media time, and then the `quantile` of
/// those middles.
///
/// **The estimator a one-signed contaminant needs, and the one-stage
/// reductions cannot be.** Parallax only ever displaces a reading towards the
/// front lens, so the camera's own number is the *far end* of a direction's
/// population and not its middle. Taking that end off the raw readings does
/// not work: one reading carries about half a degree of correlation noise
/// (the plant control), so a low quantile of raw readings is a quantile of the
/// noise. Two steps separate the two: the middle of a stretch averages the
/// noise down over its hundred-odd readings, and the quantile over stretches
/// then picks the stretch whose content was furthest away, which is the one
/// with least parallax in it.
///
/// If the wander between stretches is a scene, this reproduces between
/// captures and the one-stage reductions do not. If it reproduces no better,
/// the wander is not one-signed and is therefore not a distance.
fn staged(rows: &[settled::Row], stages: usize, quantile: f64, options: &Options) -> Vec<At> {
    let seconds: Vec<f64> = rows.iter().map(|row| row.seconds).collect();
    let (Some(first), Some(last)) = (
        seconds.iter().copied().reduce(f64::min),
        seconds.iter().copied().reduce(f64::max),
    ) else {
        return Vec::new();
    };
    let width = (last - first) / stages as f64;
    let mut per: std::collections::BTreeMap<usize, Vec<At>> = std::collections::BTreeMap::new();
    for stage in 0..stages {
        let from = first + width * stage as f64;
        for at in settled::field(rows, from, from + width + 1e-6, options.middle, false) {
            per.entry(at.index).or_default().push(at);
        }
    }
    per.into_values()
        .filter(|all| all.len() >= 2)
        .map(|all| {
            let pick = |of: fn(&At) -> f64| {
                settled::quantile(&all.iter().map(of).collect::<Vec<_>>(), quantile)
            };
            At {
                along: pick(|at| at.along),
                across: pick(|at| at.across),
                readings: all.iter().map(|at| at.readings).sum(),
                across_error: seam::rms(all.iter().map(|at| at.across_error))
                    / (all.len() as f64).sqrt(),
                ..all[0]
            }
        })
        .collect()
}

/// The moments `--bin table`'s twelve-place plan would have read, and no
/// others. `--bin corpus`'s own thinning, so the density sweeps compare.
fn thinned(rows: Vec<settled::Row>, places: usize) -> Vec<settled::Row> {
    if places == 0 {
        return rows;
    }
    let mut moments: Vec<f64> = rows.iter().map(|row| row.seconds).collect();
    moments.sort_by(f64::total_cmp);
    moments.dedup();
    let (Some(first), Some(last)) = (moments.first(), moments.last()) else {
        return rows;
    };
    let wanted: Vec<f64> = (0..places)
        .map(|place| {
            let at = first + (last - first) * (place as f64 + 0.5) / places as f64;
            *moments
                .iter()
                .min_by(|one, other| (*one - at).abs().total_cmp(&(*other - at).abs()))
                .unwrap_or(&at)
        })
        .collect();
    rows.into_iter()
        .filter(|row| wanted.iter().any(|at| (row.seconds - at).abs() < 1e-6))
        .collect()
}

fn moments_in(rows: &[settled::Row]) -> usize {
    rows.iter()
        .map(|row| row.seconds.to_bits())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

/// The positive control: a field of known amplitude and known cycles put into
/// every reading of every capture, at that reading's own azimuth.
///
/// The same field in all of them, which is what *static* means. It is added
/// to the raw across-seam column before any reduction, so what it exercises is
/// the whole chain the verdict rests on - the trim, the gate, the pose
/// subtraction, the pairing and the held-out fit - and not just the
/// arithmetic at the end.
fn plant(rows: &mut [settled::Row], options: &Options) {
    let Some((amplitude, cycles)) = options.plant else {
        return;
    };
    for row in rows {
        let planted = amplitude * (cycles * row.phi.to_radians()).cos();
        match options.axis {
            Axis::Epi => row.across += planted,
            Axis::Along => row.along += planted,
        }
    }
}

/// How many source pixels of the delivered frame one degree of the axis under
/// study is, at the front lens, as a median round the ring.
fn scale_of(base: &kjerag_render::Reframe, ring: &[seam::Where], axis: Axis) -> f64 {
    let probe = 0.01f64.to_radians();
    let mut all: Vec<f64> = ring
        .iter()
        .filter_map(|at| {
            let tangent = match axis {
                Axis::Epi => at.across,
                Axis::Along => at.along,
            };
            let step = |sign: f64| {
                let ray = seam::unit(std::array::from_fn(|c| {
                    at.centre[c] + sign * probe * tangent[c]
                }));
                let landing = base.project(0, ray.map(|c| c as f32));
                landing.inside.then_some(landing.pixel)
            };
            let (plus, minus) = (step(1.0)?, step(-1.0)?);
            Some(
                f64::from(plus[0] - minus[0]).hypot(f64::from(plus[1] - minus[1]))
                    / (2.0 * probe).to_degrees(),
            )
        })
        .collect();
    all.sort_by(f64::total_cmp);
    all.get(all.len() / 2).copied().unwrap_or(f64::NAN)
}

// ------------------------------------------------------------ the report

fn report(captures: &mut [Capture], options: &Options) -> Fallible<()> {
    println!("axis:   {}", options.axis.name());
    if let Some((stages, quantile)) = options.stages {
        println!(
            "stages: each session cut into {stages} stretches, each reduced on its own, then the \
             {:.0}th\n\x20       percentile taken over the stretches.",
            quantile * 100.0,
        );
    }
    println!(
        "reduce: one direction's readings become one number by {}{}{}",
        options.middle.name(),
        match options.moments {
            0 => String::new(),
            places => format!(", off {places} moments of each dump and no more"),
        },
        match options.gate {
            true => ", then the ring passes the plausibility gate",
            false => ", and NO gate is applied (control)",
        },
    );
    println!(
        "seam:   one stored pose for every capture, roll {:+.3} yaw {:+.3} pitch {:+.3} cx {:+.2} \
         cy {:+.2}",
        options.seam.roll_deg,
        options.seam.yaw_deg,
        options.seam.pitch_deg,
        options.seam.cx_px,
        options.seam.cy_px,
    );
    if let Some((amplitude, cycles)) = options.plant {
        println!(
            "PLANT:  {amplitude:+.3} deg at {cycles:.0} cycles round the ring, added to every \
             reading of every capture."
        );
    }
    for capture in captures.iter_mut() {
        capture.subtract(&options.seam, options.gate, options.axis);
    }
    per_capture_rings(captures, options);
    let pooled: Vec<Leftover> = captures
        .iter()
        .flat_map(|capture| settled::leftovers(&capture.left))
        .collect();
    if pooled.is_empty() {
        return Err("no capture had a reading on its seam".into());
    }
    structure(&pooled);
    reproduces(captures);
    density(captures, options);
    held_out(captures, options);
    within(captures, options);
    drift(captures, options);
    spill(captures, options)
}

fn per_capture_rings(captures: &[Capture], options: &Options) {
    println!(
        "\n{:<12} {:>8} {:>9} {:>9} {:>10} {:>8} {:>9} {:>9}",
        "capture", "moments", "azimuths", "factory", "under pose", "refused", "tolerance", "px/deg"
    );
    for capture in captures {
        println!(
            "{:<12} {:>8} {:>9} {:>9.4} {:>10.4} {:>8} {:>9.3} {:>9.2}",
            capture.name,
            capture.moments,
            capture.left.len(),
            capture.before,
            capture.after,
            capture.refused,
            capture.tolerance,
            capture.scale,
        );
    }
    depth(captures, options);
}

/// The depth control, and the only question this axis has that the along-seam
/// axis does not.
///
/// Parallax displaces a near subject one way only, at every azimuth. So if a
/// scene is in these readings at all, every azimuth's population leans the
/// same way, and `lean` counts the azimuths that lean positive. A ring that
/// splits near half and half is not reading a distance; a ring that is
/// one-signed is, and then the middle of the population is the wrong
/// estimator for a camera and the far end of it is the right one.
fn depth(captures: &[Capture], options: &Options) {
    if options.axis != Axis::Epi {
        return;
    }
    println!(
        "\ndepth: how each azimuth's own readings lean, before any pose. `lean+` is how many \n\
         \x20       azimuths have a longer tail on the positive side (positive disparity is \n\
         \x20       near: band::Cell::disparity). `skew` is that tail's excess in degrees, \n\
         \x20       pooled as a median over the azimuths."
    );
    println!(
        "{:<12} {:>10} {:>10} {:>12} {:>12}",
        "capture", "azimuths", "lean+", "skew deg", "spread deg"
    );
    for capture in captures {
        let leaning = capture
            .read
            .iter()
            .filter(|at| at.across_skew > 0.0)
            .count();
        let mut skews: Vec<f64> = capture.read.iter().map(|at| at.across_skew).collect();
        skews.sort_by(f64::total_cmp);
        println!(
            "{:<12} {:>10} {:>4}/{:<5} {:>12.4} {:>12.4}",
            capture.name,
            capture.read.len(),
            leaning,
            capture.read.len(),
            settled::median(&skews),
            seam::rms(capture.read.iter().map(|at| at.across_error)),
        );
    }
}

/// How much of the pooled leftover each harmonic order can describe: the
/// shape question, measured on the readings themselves rather than held out.
fn structure(pooled: &[Leftover]) {
    println!(
        "\nstructure: what each harmonic order leaves on the {} pooled readings, degrees rms \n\
         \x20       across the seam. fitted on the same readings it is measured on, so this is \n\
         \x20       the ceiling and `held out` below is the honest column.",
        pooled.len(),
    );
    print!("order  {:>8}", "none");
    for order in 0..ORDERS {
        print!("{order:>8}");
    }
    println!();
    print!(
        "left   {:>8.4}",
        seam::rms(pooled.iter().map(|l| f64::from(l.perp.to_degrees())))
    );
    for order in 0..ORDERS {
        print!("{:>8.4}", harmonic_left(pooled, order));
    }
    println!();
}

fn harmonic_left(pooled: &[Leftover], order: usize) -> f64 {
    let rows = design(pooled, order);
    let Some(fitted) = seam::least_squares(&rows) else {
        return f64::NAN;
    };
    seam::rms(rows.iter().map(|(row, value)| {
        value
            - row
                .iter()
                .zip(&fitted.params)
                .map(|(a, b)| a * b)
                .sum::<f64>()
    }))
}

fn design(left: &[Leftover], order: usize) -> Vec<(Vec<f64>, f64)> {
    left.iter()
        .map(|l| {
            (
                settled::harmonics(f64::from(l.phi), order),
                f64::from(l.perp.to_degrees()),
            )
        })
        .collect()
}

/// Whether two captures read the same thing at the same azimuth, which is the
/// premise a per-camera static term rests on.
fn reproduces(captures: &[Capture]) {
    if captures.len() < 2 {
        return;
    }
    println!(
        "\nagreement: the same azimuth read on two captures, degrees across the seam. `apart` \n\
         \x20       under `spread` is a camera; `apart` over it is a scene. the last column is \n\
         \x20       the same difference with each capture's own five terms taken off first."
    );
    println!(
        "{:<12} {:<12} {:>7} {:>10} {:>10} {:>12} {:>7}",
        "capture", "against", "shared", "apart rms", "spread", "5 terms off", "passes"
    );
    let (mut apart_all, mut spread_all, mut passed) = (Vec::new(), Vec::new(), 0);
    for (index, one) in captures.iter().enumerate() {
        for other in captures.iter().skip(index + 1) {
            let shared = settled::shared(&one.left, &other.left);
            let apart = seam::rms(shared.iter().map(|(a, b)| a.left - b.left));
            let spread = seam::rms(shared.iter().map(|(a, _)| a.left));
            passed += usize::from(apart < spread);
            apart_all.push(apart);
            spread_all.push(spread);
            println!(
                "{:<12} {:<12} {:>7} {:>10.4} {:>10.4} {:>12.4} {:>7}",
                one.name,
                other.name,
                shared.len(),
                apart,
                spread,
                levelled_apart(one, other),
                match apart < spread {
                    true => "yes",
                    false => "NO",
                },
            );
        }
    }
    let (apart, spread) = (
        seam::rms(apart_all.iter().copied()),
        seam::rms(spread_all.iter().copied()),
    );
    println!(
        "{:<12} {:<12} {:>7} {:>10.4} {:>10.4} {:>12} {:>4}/{:<2}",
        "all pairs",
        "",
        apart_all.len(),
        apart,
        spread,
        "",
        passed,
        apart_all.len(),
    );
    // Two captures of one camera are the same field plus their own scenes. If
    // the scenes are independent, `apart` is the two scenes added in
    // quadrature and `spread` is the field and one scene, so what the two
    // captures actually share comes out of them by subtraction. It is a
    // decomposition and not a measurement: it assumes exactly that
    // independence, and it is reported because the held-out column below tests
    // the same quantity without assuming it.
    let shared = spread * spread - apart * apart / 2.0;
    println!(
        "        shared component, if the two scenes are independent: {} deg rms",
        match shared > 0.0 {
            true => format!("{:.4}", shared.sqrt()),
            false => "none, apart is past the independent limit".into(),
        },
    );
}

fn levelled_apart(one: &Capture, other: &Capture) -> f64 {
    let mine = settled::five(&settled::leftovers(&one.left));
    let theirs = settled::five(&settled::leftovers(&other.left));
    let shared = settled::shared(&one.left, &other.left);
    seam::rms(shared.iter().map(|(a, b)| {
        let phi = a.phi.to_radians();
        (a.left - settled::at_phi(&mine, phi)) - (b.left - settled::at_phi(&theirs, phi))
    }))
}

/// How deep the sampling has to be before the agreement above is reachable.
/// The along axis needed about ten readings per azimuth; this asks the same
/// question of this one, by thinning the same dumps.
fn density(captures: &[Capture], options: &Options) {
    println!(
        "\ndensity: the same corpus thinned to fewer moments and reduced again. `per az` is \n\
         \x20       readings per azimuth after thinning."
    );
    println!(
        "{:>9} {:>9} {:>10} {:>10} {:>10}",
        "moments", "per az", "apart", "spread", "passing"
    );
    for depth in DEPTHS {
        let thin: Vec<(Vec<At>, f64)> = captures
            .iter()
            .map(|capture| {
                let rows = thinned(capture.rows.clone(), depth);
                let shift = capture.shift(&options.seam, options.axis);
                let field: Vec<At> =
                    settled::field(&rows, 0.0, f64::INFINITY, options.middle, false)
                        .into_iter()
                        .filter_map(|at| {
                            Some(At {
                                left: options.axis.of(&at) + (*shift.get(at.index)?)?,
                                ..at
                            })
                        })
                        .collect();
                let per = match field.is_empty() {
                    true => 0.0,
                    false => {
                        field.iter().map(|at| at.readings).sum::<usize>() as f64
                            / field.len() as f64
                    }
                };
                (gate_of(field, options.gate), per)
            })
            .collect();
        let (mut apart_all, mut spread_all, mut passed, mut pairs) = (Vec::new(), Vec::new(), 0, 0);
        for (index, (one, _)) in thin.iter().enumerate() {
            for (other, _) in thin.iter().skip(index + 1) {
                let shared = settled::shared(one, other);
                if shared.is_empty() {
                    continue;
                }
                let apart = seam::rms(shared.iter().map(|(a, b)| a.left - b.left));
                let spread = seam::rms(shared.iter().map(|(a, _)| a.left));
                passed += usize::from(apart < spread);
                pairs += 1;
                apart_all.push(apart);
                spread_all.push(spread);
            }
        }
        let per = thin.iter().map(|(_, per)| per).sum::<f64>() / thin.len() as f64;
        println!(
            "{:>9} {:>9.1} {:>10.4} {:>10.4} {:>6}/{:<3}",
            match depth {
                0 => "all".to_string(),
                depth => depth.to_string(),
            },
            per,
            seam::rms(apart_all.into_iter()),
            seam::rms(spread_all.into_iter()),
            passed,
            pairs,
        );
    }
}

/// Where the whole ring sits at each stage of the session: the diagnostic
/// that says whether within-flight movement is a drift or a scatter.
///
/// A scene term moves the ring bodily, because a change of distance moves
/// every azimuth that sees the ground the same way; noise does not. So this
/// prints one number per stretch - the ring's own median leftover - and how
/// many azimuths moved the same way as it did between one stretch and the
/// next. A run of one-signed moves is a distance changing; a sign that flips
/// every stretch is not.
fn drift(captures: &[Capture], options: &Options) {
    println!(
        "\ndrift: the ring's median leftover over {} equal stretches of each session, degrees. \n\
         \x20       `agree` is the share of azimuths that moved the same way the ring's median \n\
         \x20       did, stretch to stretch, pooled: 1.00 is the whole ring moving together.",
        options.drifts,
    );
    for capture in captures {
        let Some(parts) = stretches(capture, options, options.drifts) else {
            continue;
        };
        let middles: Vec<f64> = parts
            .iter()
            .map(|part| settled::median(&part.iter().map(|at| at.left).collect::<Vec<_>>()))
            .collect();
        let (mut together, mut counted) = (0, 0);
        for (index, one) in parts.iter().enumerate().skip(1) {
            let step = middles[index] - middles[index - 1];
            for (a, b) in settled::shared(one, &parts[index - 1]) {
                counted += 1;
                together += usize::from(((a.left - b.left) * step) > 0.0);
            }
        }
        print!("{:<12}", capture.name);
        for middle in &middles {
            print!("{middle:>8.3}");
        }
        println!(
            "   agree {:.2}",
            match counted {
                0 => f64::NAN,
                counted => together as f64 / counted as f64,
            }
        );
    }
}

/// One capture cut into `parts` equal stretches of media time, each reduced
/// and gated on its own.
fn stretches(capture: &Capture, options: &Options, parts: usize) -> Option<Vec<Vec<At>>> {
    let span: Vec<f64> = capture.rows.iter().map(|row| row.seconds).collect();
    let first = span.iter().copied().reduce(f64::min)?;
    let last = span.iter().copied().reduce(f64::max)?;
    let width = (last - first) / parts as f64;
    Some(
        (0..parts)
            .map(|part| {
                let from = first + width * part as f64;
                capture.stretch(from, from + width + 1e-6, options)
            })
            .collect(),
    )
}

fn gate_of(field: Vec<At>, gate: bool) -> Vec<At> {
    if !gate {
        return field;
    }
    let (kept, _) = settled::gated(&settled::leftovers(&field));
    let survived: Vec<f32> = kept.iter().map(|l| l.phi).collect();
    field
        .into_iter()
        .filter(|at| survived.contains(&at.leftover().phi))
        .collect()
}

/// Each capture predicted by a field fitted on the OTHER captures: the column
/// that decides the fork, and the shape ladder that decides how many terms
/// the static term would need.
fn held_out(captures: &[Capture], options: &Options) {
    if captures.len() < 2 {
        return;
    }
    println!(
        "\nheld out: each capture predicted by a field fitted on the OTHER captures, degrees \n\
         \x20       rms across the seam. nothing below is measured on its own data. `pose` is \n\
         \x20       the leftover itself; `o<n>` is a harmonic field of order n fitted \n\
         \x20       elsewhere; `table` is a {:.0} deg kernel per-azimuth field fitted elsewhere.",
        options.smooth,
    );
    print!("{:<12} {:>9}", "capture", "azimuths");
    for order in LADDER {
        print!("{:>9}", format!("o{order}"));
    }
    println!("{:>9} {:>8}", "table", "better");
    let mut totals: Vec<Vec<(usize, f64)>> = vec![Vec::new(); LADDER.len() + 2];
    for (index, held) in captures.iter().enumerate() {
        let train: Vec<Leftover> = captures
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .flat_map(|(_, capture)| settled::leftovers(&capture.left))
            .collect();
        let test = settled::leftovers(&held.left);
        let arms = predict(&train, &test, options.smooth);
        for (all, arm) in totals.iter_mut().zip(&arms) {
            all.push((test.len(), *arm));
        }
        print!("{:<12} {:>9}", held.name, test.len());
        for arm in &arms {
            print!("{arm:>9.4}");
        }
        println!(
            " {:>8}",
            match arms[3] < arms[0] {
                true => "yes",
                false => "NO",
            }
        );
    }
    print!(
        "{:<12} {:>9}",
        "all",
        totals[0].iter().map(|(n, _)| n).sum::<usize>()
    );
    for all in &totals {
        print!("{:>9.4}", pooled_rms(all));
    }
    println!();
}

fn pooled_rms(all: &[(usize, f64)]) -> f64 {
    let count: usize = all.iter().map(|(n, _)| n).sum();
    (all.iter().map(|(n, v)| *n as f64 * v * v).sum::<f64>() / count as f64).sqrt()
}

/// What the held-out capture reads with each arm taken off it, in degrees rms:
/// the harmonic ladder, then the per-azimuth kernel field.
fn predict(train: &[Leftover], test: &[Leftover], smooth: f64) -> Vec<f64> {
    let value = |l: &Leftover| f64::from(l.perp.to_degrees());
    let mut arms = Vec::new();
    for order in LADDER {
        let terms = seam::least_squares(&design(train, order)).map(|fit| fit.params);
        arms.push(match terms {
            Some(terms) => seam::rms(
                test.iter()
                    .map(|l| value(l) - settled::at_phi(&terms, f64::from(l.phi))),
            ),
            None => f64::NAN,
        });
    }
    let kernel = smoothed(train, smooth);
    arms.push(seam::rms(
        test.iter()
            .map(|l| value(l) - at_kernel(&kernel, f64::from(l.phi))),
    ));
    arms
}

/// A per-azimuth field over 128 directions, raised-cosine weighted, with one
/// reading's worth of ridge, in degrees.
///
/// `band::Table`'s recipe without its two along-seam-only steps: it does not
/// level the five terms out first (on this axis nothing else applies them) and
/// it does not clamp at half a degree (this axis's field is larger than that).
fn smoothed(left: &[Leftover], half_deg: f64) -> [f64; 128] {
    let half = half_deg.to_radians();
    std::array::from_fn(|index| {
        let phi = index as f64 / 128.0 * std::f64::consts::TAU;
        let (mut sum, mut weight) = (0.0, 0.0);
        for l in left {
            let mut apart = (f64::from(l.phi) - phi).abs() % std::f64::consts::TAU;
            if apart > std::f64::consts::PI {
                apart = std::f64::consts::TAU - apart;
            }
            if apart >= half {
                continue;
            }
            let w = 0.5 * (1.0 + (std::f64::consts::PI * apart / half).cos());
            sum += w * f64::from(l.perp.to_degrees());
            weight += w;
        }
        sum / (weight + 1.0)
    })
}

fn at_kernel(entries: &[f64; 128], phi: f64) -> f64 {
    let turn = phi / std::f64::consts::TAU * 128.0;
    let low = turn.floor();
    let mix = turn - low;
    let entry = |step: i64| entries[(low as i64 + step).rem_euclid(128) as usize];
    entry(0) + (entry(1) - entry(0)) * mix
}

/// Whether one flight's own epi field holds still while it plays: the #155
/// within-flight claim, re-asked at full density under this reduction.
fn within(captures: &[Capture], options: &Options) {
    println!(
        "\nwithin flight: each capture cut into {} equal stretches of media time, reduced \n\
         \x20       separately, and compared azimuth by azimuth. `swing` is the largest \n\
         \x20       difference any azimuth shows between two stretches.",
        options.parts,
    );
    println!(
        "{:<12} {:>8} {:>10} {:>10} {:>10} {:>10}",
        "capture", "shared", "rms apart", "swing deg", "swing px", "field rms"
    );
    for capture in captures {
        let Some(parts) = stretches(capture, options, options.parts) else {
            continue;
        };
        let (mut apart, mut swing, mut shared_count) = (Vec::new(), 0.0f64, 0);
        for (index, one) in parts.iter().enumerate() {
            for other in parts.iter().skip(index + 1) {
                for (a, b) in settled::shared(one, other) {
                    apart.push(a.left - b.left);
                    swing = swing.max((a.left - b.left).abs());
                    shared_count += 1;
                }
            }
        }
        println!(
            "{:<12} {:>8} {:>10.4} {:>10.4} {:>10.2} {:>10.4}",
            capture.name,
            shared_count,
            seam::rms(apart.into_iter()),
            swing,
            swing * capture.scale,
            capture.after,
        );
    }
}

/// Every leftover behind every number above, so a claim about this corpus can
/// be re-checked without a second decode.
fn spill(captures: &[Capture], options: &Options) -> Fallible<()> {
    let Some(dump) = &options.dump else {
        return Ok(());
    };
    if let Some(parent) = dump.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = format!(
        "# source: kjerag-spike --bin epi ({})\n# args: {}\n# axis: across the seam (epipolar)\n\
         # reduction: {}, gate {}\n# seam: roll:{} yaw:{} pitch:{} cx:{} cy:{}\n\
         # left_deg is across_deg with the pose's own across-seam shift added.\n",
        env!("CARGO_PKG_VERSION"),
        std::env::args().skip(1).collect::<Vec<_>>().join(" "),
        options.middle.name(),
        match options.gate {
            true => "on",
            false => "off",
        },
        options.seam.roll_deg,
        options.seam.yaw_deg,
        options.seam.pitch_deg,
        options.seam.cx_px,
        options.seam.cy_px,
    );
    text.push_str("capture,phi_deg,read_deg,left_deg,readings,negative,error_deg,skew_deg\n");
    for capture in captures {
        for at in &capture.left {
            let _ = writeln!(
                text,
                "{},{:.4},{:.6},{:.6},{},{},{:.6},{:.6}",
                capture.name,
                at.phi,
                options.axis.of(at),
                at.left,
                at.readings,
                at.across_negative,
                at.across_error,
                at.across_skew,
            );
        }
    }
    std::fs::write(dump, text)?;
    println!("\nwrote:  {}", dump.display());
    Ok(())
}

fn short(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into()
}

// ------------------------------------------------------------ the options

struct Options {
    inputs: Vec<PathBuf>,
    axis: Axis,
    seam: SeamFit,
    middle: Middle,
    gate: bool,
    /// How many moments of each dump are read, spread over it, or 0 for all.
    moments: usize,
    smooth: f64,
    parts: usize,
    drifts: usize,
    stages: Option<(usize, f64)>,
    plant: Option<(f64, f64)>,
    dump: Option<PathBuf>,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut inputs = Vec::new();
        let mut axis = Axis::Epi;
        let mut seam = None;
        let mut middle = Middle::Trimmed;
        let mut gate = true;
        let mut moments = 0;
        let mut smooth = f64::from(kjerag_render::band::SMOOTH_DEG);
        let mut parts = 2;
        let mut drifts = 8;
        let mut stages = None;
        let mut plant = None;
        let mut dump = None;
        for arg in args {
            match arg.split_once('=') {
                Some(("axis", "epi")) => axis = Axis::Epi,
                Some(("axis", "along")) => axis = Axis::Along,
                Some(("seam", value)) => {
                    if value == "file" || value == "factory" || value == "corpus" {
                        return Err(USAGE_SEAM.into());
                    }
                    seam = Some(seam_fit(value)?);
                }
                Some(("middle", value)) => {
                    middle = Middle::parse(value).ok_or("middle is mean, median or trimmed")?;
                }
                Some(("gate", value)) => gate = value != "0",
                Some(("moments", value)) => moments = value.parse()?,
                Some(("smooth", value)) => smooth = value.parse()?,
                Some(("parts", value)) => parts = value.parse::<usize>()?.max(2),
                Some(("drifts", value)) => drifts = value.parse::<usize>()?.max(2),
                Some(("stages", value)) => {
                    let (count, quantile) =
                        value.split_once(':').ok_or("stages is count:quantile")?;
                    stages = Some((count.parse::<usize>()?.max(2), quantile.parse()?));
                }
                Some(("plant", value)) => {
                    let (amplitude, cycles) =
                        value.split_once(':').ok_or("plant is amplitude:cycles")?;
                    plant = Some((amplitude.parse()?, cycles.parse()?));
                }
                Some(("dump", value)) => dump = Some(PathBuf::from(value)),
                Some(_) => return Err(format!("{USAGE}\n\nunknown: {arg}").into()),
                None => inputs.push(PathBuf::from(arg)),
            }
        }
        Ok(Self {
            inputs,
            axis,
            seam: seam.ok_or(USAGE_SEAM)?,
            middle,
            gate,
            moments,
            smooth,
            parts,
            drifts,
            stages,
            plant,
            dump,
        })
    }
}

const USAGE: &str = "usage: epi <settle-dump.csv> [<settle-dump.csv> ...] \
seam=roll:0.8,yaw:-2.3,pitch:-0.9,cx:-3.3,cy:-11.9 [axis=epi|along] \
[middle=mean|median|trimmed|far:0.1] [gate=0] \
[moments=n] [smooth=deg] [parts=n] [drifts=n] [stages=count:quantile] \
[plant=amplitude:cycles] [dump=path.csv]";

const USAGE_SEAM: &str = "this instrument needs one stored pose for every capture: a fit off each \
capture's own frames absorbs that scene into the pose, and two such fits do not leave the same \
quantity behind. seam=roll:..,yaw:..,pitch:..,cx:..,cy:..";
