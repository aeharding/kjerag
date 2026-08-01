//! What the two lenses' pictures of the **same content** differ by in
//! brightness and in colour, measured across the overlap band (issue #103,
//! stage 3).
//!
//! ```sh
//! # the measurement and every one of its controls, on one stretch
//! cargo run --release -p kjerag-spike --bin expose -- <file.insv> from=488.855 count=8
//! # the same estimator the format study's 6.3 used, for comparison
//! cargo run --release -p kjerag-spike --bin expose -- <file.insv> mode=annulus from=488.855
//! # what a rendered view's luma does as it crosses the seam
//! cargo run --release -p kjerag-spike --bin expose -- <file.insv> mode=render \
//!   from=488.855 yaw=67.24 pitch=2.56 fov=218.99 out=scratch/stage3-proof
//! # the pooled gain frame by frame: does it pump
//! cargo run --release -p kjerag-spike --bin expose -- <file.insv> mode=trace count=120
//! ```
//!
//! **Nothing in the project's earlier exposure corpus is used here, or
//! checked against.** It was audited and refused in full (issue #103): three
//! populations in one column, rows that do not divide, a flat-sky claim with
//! no sky measurement behind it, and a shutter-ratio correction measured to
//! make the artifact four to twenty times worse. This binary re-measures from
//! zero and carries its own controls.
//!
//! **What is new here, and it is the whole method.** An exposure step and a
//! misregistration are the same picture unless the two lenses are sampled on
//! content that has been *lined up first*: two annuli of "the same world
//! directions" are the same directions only if the calibration is exact, and
//! it is not - it is out by degrees before the fit (6.8) and by parallax after
//! it (6.9). So every reading below is taken **after** the alignment the
//! project now measures per direction, and the estimator that finds that
//! alignment is normalized cross-correlation, which is invariant to exactly
//! the affine brightness change being measured. The two questions are
//! therefore orthogonal by construction rather than by hope, and
//! [`Trial`] is how that claim is checked rather than asserted: the same
//! measurement runs on one lens against itself, at the same shift, where the
//! true answer is known to be 1.
//!
//! PNGs land in gitignored `scratch/`: these are frames of somebody's real
//! flights and this repo is public.

use std::path::PathBuf;

use kjerag_media::{Fallible, Pair, Plane, Walk};
use kjerag_meta::{CalibrationSet, Lens};
use kjerag_render::seam::{self, Probe, Refused, Where};
use kjerag_render::{Camera, Cue, Horizon, Reframe, Sampling, Scene, ScenePipeline, Size};
use kjerag_spike::{FORMAT, Gpu, Picture, Render};

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    match options.mode {
        Mode::Field => field(&options),
        Mode::Annulus => annulus(&options),
        Mode::Render => render(&options),
        Mode::Trace => trace(&options),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// The measurement and its controls.
    Field,
    /// The estimator 6.3 used, so the two can be told apart.
    Annulus,
    /// One view, and what its luma does as it crosses the seam.
    Render,
    /// What the shipped pass's own pooled gain does frame to frame.
    Trace,
}

// ------------------------------------------------------------ the sampling

/// How wide a photometric patch is along the seam, in degrees.
///
/// The band's own patch is 2.0 degrees ([`kjerag_render::band`]), and this
/// matches it so that what this instrument measures is what the shipped pass
/// can measure, not a more generous version of it.
const ALONG_DEG: f64 = 2.0;

/// How finely a patch is sampled along the seam, in degrees.
const ALONG_STEP_DEG: f64 = 0.1;

/// How far either side of the seam the columns reach, in degrees.
///
/// Bounded by the optics and not chosen: the two lenses of the fixture
/// overlap by 14.4 degrees, 7.2 a side, and a column further out than that is
/// a column one lens has no picture of. 4 degrees leaves room for the
/// alignment shift on top, which is added to lens 1's sampling direction.
const ACROSS_DEG: f64 = 4.0;

/// How far apart the columns are, in degrees.
const ACROSS_STEP_DEG: f64 = 0.25;

/// Which columns count as "at the seam" for the gain, in degrees either side.
///
/// The band's patch reaches 1 degree either way, so this is the same content
/// the shipped pass will pool over, and the gain reported here is the gain it
/// can reach.
const AT_SEAM_DEG: f64 = 1.0;

/// The code either side of which a sample is not a measurement of brightness.
///
/// A clipped highlight has no ratio: if one lens is at the ceiling and the
/// other is not, their difference is the ceiling and not the exposure. The
/// pair is dropped together, so nothing is biased by dropping it. The sun is
/// in shot at the owner's own reference view, which is why this exists.
const CEILING: f64 = 252.0;
const FLOOR: f64 = 2.0;

/// One column of one azimuth: the two lenses' pictures of one across-seam
/// offset, pooled over the along-seam samples.
#[derive(Clone, Copy, Default)]
struct Column {
    /// How far past the seam, in degrees, positive towards lens 0.
    delta: f64,
    count: f64,
    sum0: f64,
    sum1: f64,
    /// For the affine fit `lens1 = gain * lens0 + offset`.
    sum00: f64,
    sum01: f64,
    /// Cb and Cr, each lens, in signed codes.
    chroma: [f64; 4],
    chroma_count: f64,
}

impl Column {
    fn mean0(&self) -> f64 {
        self.sum0 / self.count
    }

    fn mean1(&self) -> f64 {
        self.sum1 / self.count
    }

    /// The natural log of lens 1's brightness over lens 0's.
    ///
    /// In logs because a gain is multiplicative and because the correction is
    /// a symmetric split, which is a halving of this number and not of a
    /// ratio.
    fn log_ratio(&self) -> f64 {
        (self.mean1() / self.mean0()).ln()
    }

    fn add(&mut self, other: &Self) {
        self.count += other.count;
        self.sum0 += other.sum0;
        self.sum1 += other.sum1;
        self.sum00 += other.sum00;
        self.sum01 += other.sum01;
        self.chroma_count += other.chroma_count;
        for (held, more) in self.chroma.iter_mut().zip(other.chroma) {
            *held += more;
        }
    }
}

/// What one run of the photometry is run on.
///
/// Every control in this instrument is one of these rather than a second code
/// path, which is the point: a control that runs different code proves the
/// control works.
#[derive(Clone, Copy)]
struct Trial {
    /// Which lens the second side is sampled from. 1 is the measurement; 0 is
    /// the null, where the answer has to be exactly 1 because it is the same
    /// picture of the same directions.
    back: usize,
    /// Whether the alignment the correlation found is applied to the second
    /// side's sampling directions. Off is what every exposure measurement
    /// before this one could do.
    aligned: bool,
    /// Added to that alignment, in degrees along and across: the sensitivity
    /// probe.
    nudge: (f64, f64),
    /// What the second side's samples are multiplied by: the positive
    /// control. A known gain has to come back as itself.
    inject: f64,
}

impl Trial {
    /// The measurement itself.
    const TRUTH: Self = Self {
        back: 1,
        aligned: true,
        nudge: (0.0, 0.0),
        inject: 1.0,
    };
}

/// One azimuth's columns, or `None` where the alignment did not correlate or
/// one of the two lenses has no picture of the patch.
fn columns(
    reframe: &Reframe,
    planes: &[Plane],
    at: &Where,
    found: (f64, f64),
    trial: Trial,
) -> Option<Vec<Column>> {
    let along = (ALONG_DEG / 2.0 / ALONG_STEP_DEG).round() as isize;
    let across = (ACROSS_DEG / ACROSS_STEP_DEG).round() as isize;
    let shift = match trial.aligned {
        true => (found.0 + trial.nudge.0, found.1 + trial.nudge.1),
        false => trial.nudge,
    };
    let mut out = Vec::with_capacity((2 * across + 1) as usize);
    for column in -across..=across {
        let delta = column as f64 * ACROSS_STEP_DEG;
        let mut held = Column {
            delta,
            ..Column::default()
        };
        for row in -along..=along {
            let a = row as f64 * ALONG_STEP_DEG;
            // A direction one of the two lenses has no picture of is not a
            // pair. Dropped rather than refusing the azimuth, and dropped on
            // BOTH sides at once, so what is left is still the same content
            // in both and nothing is biased by what went.
            let (Some(front), Some(back)) = (
                look(reframe, planes, 0, at, (a, delta)),
                look(
                    reframe,
                    planes,
                    trial.back,
                    at,
                    (a + shift.0, delta + shift.1),
                ),
            ) else {
                continue;
            };
            let (one, two) = (front.0, back.0 * trial.inject);
            if !(FLOOR..=CEILING).contains(&one) || !(FLOOR..=CEILING).contains(&two) {
                continue;
            }
            held.count += 1.0;
            held.sum0 += one;
            held.sum1 += two;
            held.sum00 += one * one;
            held.sum01 += one * two;
            if let (Some(a), Some(b)) = (front.1, back.1) {
                held.chroma_count += 1.0;
                held.chroma[0] += a.0;
                held.chroma[1] += a.1;
                held.chroma[2] += b.0;
                held.chroma[3] += b.1;
            }
        }
        // A column that clipping emptied is not a column. Kept rather than
        // refused, so a patch with the sun in a corner still reports the
        // columns that are pictures.
        if held.count > 0.0 {
            out.push(held);
        }
    }
    (out.len() > 2).then_some(out)
}

/// One lens's luma and chroma at one direction off the seam, or `None` where
/// that lens has no picture there.
fn look(
    reframe: &Reframe,
    planes: &[Plane],
    lens: usize,
    at: &Where,
    offset: (f64, f64),
) -> Option<(f64, Option<(f64, f64)>)> {
    let (a, b) = (offset.0.to_radians(), offset.1.to_radians());
    let ray = seam::unit(std::array::from_fn(|axis| {
        at.centre[axis] + at.along[axis] * a + at.across[axis] * b
    }));
    let landing = reframe.project(lens, ray.map(|c| c as f32));
    if !landing.inside {
        return None;
    }
    let plane = planes.get(lens)?;
    let (x, y) = (f64::from(landing.pixel[0]), f64::from(landing.pixel[1]));
    Some((plane.at(x, y)?, plane.chroma_at(x, y)))
}

// ------------------------------------------------------------ the pooling

/// Every azimuth's columns over a run, plus what was refused.
#[derive(Default)]
struct Field {
    /// One entry per azimuth-frame that read, with the azimuth it was read
    /// at: the same azimuth on two consecutive frames is very nearly the same
    /// measurement, so a spread that treats them as independent reads the
    /// standard error too small by the square root of the frame count.
    seen: Vec<(usize, Vec<Column>)>,
    /// How far lens 1's picture of each of those had to be moved ACROSS the
    /// seam to become the same content, in degrees.
    ///
    /// Across and not the whole shift, because across is the axis a distance
    /// displaces content along and along-seam is what the calibration left
    /// (6.8). This is the same quantity the shipped pass gates on, so the cut
    /// below is the pass's own cut and not a stricter one that would score a
    /// different set of directions.
    shifts: Vec<f64>,
    frames: usize,
    refused: usize,
}

impl Field {
    /// The pooled log gain over the columns at the seam, and how many
    /// azimuths were behind it.
    ///
    /// Pooled as a mean of per-azimuth log ratios rather than as a ratio of
    /// pooled sums, because an azimuth looking at bright sky would otherwise
    /// outweigh thirty looking at soil, and what is wanted is the gain of the
    /// pair of lenses and not the gain of the brightest thing they can see.
    fn gain(&self) -> Reading {
        Reading::of(
            self.seen
                .iter()
                .filter_map(|(index, columns)| Some((*index, at_seam(columns)?))),
        )
    }

    /// The same content pooled as **totals** rather than as an average of
    /// ratios: the log of every azimuth's lens-1 light over every azimuth's
    /// lens-0 light.
    ///
    /// Not a taste between two averages. A window shifted by an alignment
    /// error `e` reports a brightness that is wrong by `e` times the mean log
    /// gradient across the window, which is a **boundary** term: it falls as
    /// the window widens, and it changes sign with whichever way the content
    /// happens to slope. Averaging ratios keeps each patch's own boundary term
    /// at full weight; pooling totals makes the ring one window, where those
    /// terms are 128 numbers of either sign over one denominator. Which of the
    /// two is actually less sensitive is not an argument, it is the
    /// `d gain / d nudge` column below.
    /// Each azimuth-frame's two means and its sample count, which is what the
    /// three [`Model`]s are fitted to.
    fn points(&self, shift: (f64, f64)) -> Vec<(f64, f64, f64)> {
        self.seen
            .iter()
            .zip(&self.shifts)
            .filter(|(_, moved)| (shift.0..shift.1).contains(*moved))
            .filter_map(|((_, columns), _)| {
                let held = pooled(columns.iter().filter(|c| c.delta.abs() <= AT_SEAM_DEG))?;
                Some((held.mean0(), held.mean1(), held.count))
            })
            .collect()
    }

    fn totals(&self) -> f64 {
        let Some(held) = pooled(
            self.seen
                .iter()
                .flat_map(|(_, columns)| columns.iter())
                .filter(|c| c.delta.abs() <= AT_SEAM_DEG),
        ) else {
            return f64::NAN;
        };
        held.log_ratio()
    }

    /// The same for the radial term: how the log ratio slopes across the
    /// band, per degree, one number per azimuth.
    ///
    /// This is the whole of the vignetting question. Vignetting is radial and
    /// the two lenses look at the seam from opposite sides, so a direction one
    /// degree towards lens 0 is one degree further **out** in lens 1's picture
    /// and one degree further **in** in lens 0's: a rolloff shows up as a
    /// slope and a gain does not. Scene content also slopes, so what settles
    /// it is whether the slope agrees across azimuths, which is what the
    /// spread of this reading says.
    fn radial(&self) -> Reading {
        Reading::of(
            self.seen
                .iter()
                .filter_map(|(index, columns)| Some((*index, slope(columns)?))),
        )
    }

    /// Lens 1's Cb and Cr minus lens 0's, in codes, at the seam.
    fn chroma(&self) -> [Reading; 2] {
        let step = |channel: usize| {
            Reading::of(self.seen.iter().filter_map(|(index, columns)| {
                let held = pooled(columns.iter().filter(|c| c.delta.abs() <= AT_SEAM_DEG))?;
                (held.chroma_count > 0.0).then(|| {
                    (
                        *index,
                        (held.chroma[2 + channel] - held.chroma[channel]) / held.chroma_count,
                    )
                })
            }))
        };
        [step(0), step(1)]
    }
}

/// What one model of the difference between the two lenses leaves.
///
/// Three models are fitted to the same points and the one that leaves least
/// is the one the correction should be. A **gain** is what an exposure
/// difference is: the two lenses' pictures of the same content are
/// proportional. An **offset** is what veiling glare, a black-level pedestal
/// or a difference in the toe of the tone curve is: the two differ by a fixed
/// number of codes whatever the content. They are told apart by having
/// patches of every brightness from soil at 17 codes to sky at 180 in the
/// same fit, which is the one thing a single patch cannot do.
struct Model {
    name: &'static str,
    gain: f64,
    offset: f64,
    /// Root mean square of what the model does not explain, in codes.
    residual: f64,
}

impl Model {
    /// What this model leaves when it is applied as a **symmetric split**,
    /// which is how the correction is actually applied: lens 0 takes half of
    /// it one way and lens 1 half the other, so neither hemisphere carries the
    /// whole change.
    ///
    /// Returned in codes and in percent of the local brightness, because those
    /// are two different questions and the answer is different: an eye catches
    /// a step in proportion to what it is a step of, and a correction that
    /// halves the codes on bright sky while doubling the percent on dark soil
    /// has not helped.
    fn leaves(&self, points: &[(f64, f64, f64)]) -> (f64, f64, f64) {
        let root = self.gain.max(f64::MIN_POSITIVE).sqrt();
        let mut codes = 0.0;
        let mut relative = 0.0;
        let mut weight = 0.0;
        let mut worst: f64 = 0.0;
        for (m0, m1, n) in points {
            let low = root * m0 + self.offset / 2.0;
            let high = (m1 - self.offset / 2.0) / root;
            let step = high - low;
            let middle = 0.5 * (low + high);
            codes += n * step * step;
            if middle > 0.0 {
                relative += n * (step / middle).powi(2);
            }
            weight += n;
            worst = worst.max(step.abs());
        }
        match weight > 0.0 {
            true => (
                (codes / weight).sqrt(),
                100.0 * (relative / weight).sqrt(),
                worst,
            ),
            false => (0.0, 0.0, 0.0),
        }
    }

    /// The three models, fitted to the same weighted points.
    ///
    /// `points` is one entry per azimuth-frame: lens 0's mean, lens 1's mean,
    /// and how many samples were behind them.
    fn all(points: &[(f64, f64, f64)]) -> Vec<Self> {
        let sum = |f: &dyn Fn(&(f64, f64, f64)) -> f64| points.iter().map(f).sum::<f64>();
        let n = sum(&|p| p.2);
        if n <= 0.0 {
            return Vec::new();
        }
        let x = sum(&|p| p.2 * p.0);
        let y = sum(&|p| p.2 * p.1);
        let xx = sum(&|p| p.2 * p.0 * p.0);
        let xy = sum(&|p| p.2 * p.0 * p.1);
        let leftover = |gain: f64, offset: f64| {
            (points
                .iter()
                .map(|p| p.2 * (p.1 - gain * p.0 - offset).powi(2))
                .sum::<f64>()
                / n)
                .sqrt()
        };
        let mut all = vec![
            Self {
                name: "nothing at all",
                gain: 1.0,
                offset: 0.0,
                residual: leftover(1.0, 0.0),
            },
            Self {
                name: "gain, least squares in codes",
                gain: xy / xx,
                offset: 0.0,
                residual: leftover(xy / xx, 0.0),
            },
            Self {
                name: "gain, equal weight in logs",
                gain: {
                    let logs = points
                        .iter()
                        .filter(|p| p.0 > 0.0 && p.1 > 0.0)
                        .map(|p| (p.1 / p.0).ln())
                        .collect::<Vec<f64>>();
                    match logs.is_empty() {
                        true => 1.0,
                        false => (logs.iter().sum::<f64>() / logs.len() as f64).exp(),
                    }
                },
                offset: 0.0,
                residual: 0.0,
            },
            Self {
                name: "gain, ratio of totals",
                gain: y / x,
                offset: 0.0,
                residual: leftover(y / x, 0.0),
            },
            Self {
                name: "offset alone (lens1 = lens0 + o)",
                gain: 1.0,
                offset: (y - x) / n,
                residual: leftover(1.0, (y - x) / n),
            },
        ];
        let spread = xx - x * x / n;
        if spread > 0.0 {
            let gain = (xy - x * y / n) / spread;
            let offset = (y - gain * x) / n;
            all.push(Self {
                name: "gain and offset together",
                gain,
                offset,
                residual: leftover(gain, offset),
            });
        }
        for model in &mut all {
            model.residual = leftover(model.gain, model.offset);
        }
        all
    }
}

/// One pooled number, its spread, and what it was pooled over.
///
/// **Pooled per azimuth first.** The same direction of the seam read on two
/// consecutive frames is very nearly the same measurement of the same content,
/// so counting them as two independent readings divides a standard error by
/// the square root of the frame count for free and turns any bias at all into
/// a significant result. What is independent here is an azimuth, and barely
/// that.
#[derive(Clone, Copy, Default)]
struct Reading {
    mean: f64,
    /// Root mean square of the deviation from the mean, over the azimuths.
    spread: f64,
    /// How many azimuths, which is the count the standard error is taken over.
    count: usize,
    /// How many azimuth-frames went into them.
    readings: usize,
}

impl Reading {
    fn of(values: impl Iterator<Item = (usize, f64)>) -> Self {
        let values: Vec<(usize, f64)> = values.filter(|(_, v)| v.is_finite()).collect();
        let readings = values.len();
        let mut azimuths: Vec<(usize, f64, f64)> = Vec::new();
        for (index, value) in values {
            match azimuths.iter_mut().find(|held| held.0 == index) {
                Some(held) => {
                    held.1 += value;
                    held.2 += 1.0;
                }
                None => azimuths.push((index, value, 1.0)),
            }
        }
        let held: Vec<f64> = azimuths.iter().map(|(_, sum, n)| sum / n).collect();
        let count = held.len();
        if count == 0 {
            return Self::default();
        }
        let mean = held.iter().sum::<f64>() / count as f64;
        let spread = (held.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count as f64).sqrt();
        Self {
            mean,
            spread,
            count,
            readings,
        }
    }

    /// How far the mean is from zero in units of its own standard error: the
    /// one statistic that says whether a term is there at all.
    fn signal(&self) -> f64 {
        match self.spread > 0.0 && self.count > 1 {
            true => self.mean.abs() / (self.spread / (self.count as f64).sqrt()),
            false => 0.0,
        }
    }
}

/// The columns within [`AT_SEAM_DEG`] of the seam, as one log ratio.
fn at_seam(columns: &[Column]) -> Option<f64> {
    let held = pooled(columns.iter().filter(|c| c.delta.abs() <= AT_SEAM_DEG))?;
    Some(held.log_ratio())
}

/// Several columns as one.
fn pooled<'a>(columns: impl Iterator<Item = &'a Column>) -> Option<Column> {
    let mut held = Column::default();
    for column in columns {
        held.add(column);
    }
    (held.count > 0.0).then_some(held)
}

/// Least squares of the log ratio against the across-seam offset, per degree.
fn slope(columns: &[Column]) -> Option<f64> {
    let rows: Vec<(f64, f64)> = columns
        .iter()
        .filter(|c| c.count > 0.0)
        .map(|c| (c.delta, c.log_ratio()))
        .filter(|(_, y)| y.is_finite())
        .collect();
    if rows.len() < 4 {
        return None;
    }
    let n = rows.len() as f64;
    let mean_x = rows.iter().map(|r| r.0).sum::<f64>() / n;
    let mean_y = rows.iter().map(|r| r.1).sum::<f64>() / n;
    let mut covariance = 0.0;
    let mut variance = 0.0;
    for (x, y) in rows {
        covariance += (x - mean_x) * (y - mean_y);
        variance += (x - mean_x).powi(2);
    }
    (variance > 0.0).then(|| covariance / variance)
}

// ------------------------------------------------------------ the run

/// The calibration this file is drawn through, corrected the way the app
/// corrects it.
fn calibrated(options: &Options) -> Fallible<(CalibrationSet, Vec<Lens>, Size)> {
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = calibration.lenses.clone();
    if !options.fit {
        println!("seam:   factory calibration, uncorrected");
        return Ok((calibration, lenses, frame));
    }
    let Some(fitted) = seam::fit_file(&options.input, &lenses, frame, &seam::Plan::default())
    else {
        return Ok((calibration, lenses, frame));
    };
    println!("seam:   {}", fitted.describe(0.0));
    let corrected = fitted.fit.applied(&lenses);
    Ok((calibration, corrected, frame))
}

/// Every trial's field over the same frames, so the controls are read off the
/// very same pixels the measurement is.
fn sweep(options: &Options, trials: &[Trial]) -> Fallible<Vec<Field>> {
    let (_, lenses, frame) = calibrated(options)?;
    let reframe = seam::mapped(&lenses, frame);
    let ring = seam::ring(options.patches);
    let mut walk = Walk::open(&options.input, options.from, frame)?;
    if walk.streams() < 2 {
        return Err("this file carries one lens stream, so it has no seam".into());
    }
    let mut fields: Vec<Field> = (0..trials.len()).map(|_| Field::default()).collect();
    let mut refused = Refused::default();
    // Spread over the file rather than taken from one instant, because the
    // sun moves round an aircraft and a single stretch of film is a single
    // lighting. `places=1` is one stretch, which is what a reference view
    // wants.
    let duration = walk.duration().as_secs_f64();
    for place in 0..options.places.max(1) {
        if place > 0 {
            let at = options.from
                + (duration - options.from) * place as f64 / options.places.max(1) as f64;
            walk.jump(at)?;
        }
        for _ in 0..options.count {
            let Some(pair) = walk.next_pair()? else {
                break;
            };
            let found = seam::read_ring(
                &reframe,
                &pair.lenses,
                &ring,
                &options.probe(),
                &mut refused,
            );
            for (trial, field) in trials.iter().zip(&mut fields) {
                harvest(&reframe, &pair, &ring, &found, *trial, options, field);
            }
        }
    }
    if fields[0].frames == 0 {
        return Err("no frame at that instant had a correlating seam".into());
    }
    Ok(fields)
}

/// One frame's worth of columns, into one trial's field.
fn harvest(
    reframe: &Reframe,
    pair: &Pair,
    ring: &[Where],
    found: &[Option<seam::Found>],
    trial: Trial,
    options: &Options,
    field: &mut Field,
) {
    field.frames += 1;
    for (index, at) in ring.iter().enumerate() {
        let Some(hit) = found[index].filter(|hit| hit.r >= options.keep) else {
            field.refused += 1;
            continue;
        };
        let Some(columns) = columns(reframe, &pair.lenses, at, (hit.along, hit.across), trial)
        else {
            field.refused += 1;
            continue;
        };
        field.seen.push((index, columns));
        field.shifts.push(hit.across.abs());
    }
}

// ------------------------------------------------------------ the field

fn field(options: &Options) -> Fallible<()> {
    let probes = [-0.5, -0.2, 0.2, 0.5];
    let mut trials = vec![
        Trial::TRUTH,
        // What every exposure measurement before stage 2 could do: the same
        // patches, not lined up first.
        Trial {
            aligned: false,
            ..Trial::TRUTH
        },
        // The null. One lens against its own picture of the same directions,
        // where the answer is 1 by arithmetic.
        Trial {
            back: 0,
            aligned: false,
            ..Trial::TRUTH
        },
        // The null that matters: one lens against ITSELF, displaced by the
        // very shift the alignment found. No exposure difference exists here,
        // so whatever it reads is what a misregistration of that size
        // contributes to a gain, which is the confound this whole method is
        // built to keep out.
        Trial {
            back: 0,
            ..Trial::TRUTH
        },
    ];
    for inject in [0.95, 1.05] {
        trials.push(Trial {
            inject,
            ..Trial::TRUTH
        });
    }
    for across in probes {
        trials.push(Trial {
            nudge: (0.0, across),
            ..Trial::TRUTH
        });
    }
    let fields = sweep(options, &trials)?;
    let truth = &fields[0];

    println!(
        "\nfield:  {} azimuth-frames read of {} tried, over {} frames from {:.3} s, \n\
         \tat {} azimuths round the seam. every reading below is taken AFTER the two \n\
         \tlenses were lined up on the same content.",
        truth.seen.len(),
        truth.seen.len() + truth.refused,
        truth.frames,
        options.from,
        options.patches,
    );
    table(truth, options);

    let gain = truth.gain();
    println!(
        "\ngain:   {:+.4} ln, which is {:+.2} percent, spread {:.4} over {} azimuth-frames, \n\
         \t{:.1} standard errors from zero. this is lens 1's picture over lens 0's picture of \n\
         \tthe same content, in the video's own gamma-coded luma, which is the space the \n\
         \tcorrection is applied in as well, so no transfer function is assumed anywhere.",
        gain.mean,
        100.0 * (gain.mean.exp() - 1.0),
        gain.spread,
        gain.count,
        gain.signal(),
    );
    println!(
        "        pooled from {} azimuth-frames at {} distinct azimuths; the spread and the \n\
         \tstandard error are taken over the azimuths, because the same direction on two \n\
         \tconsecutive frames is the same measurement twice.",
        gain.readings, gain.count,
    );

    let radial = truth.radial();
    println!(
        "\nradial: {:+.5} ln per degree across the band, spread {:.5} over {} azimuth-frames, \n\
         \t{:.1} standard errors from zero. over the {:.0} degrees the crossover can ever \n\
         \treach that is {:+.2} percent end to end, against the {:+.2} percent step above. \n\
         \tvignetting is radial and the two lenses see the band from opposite sides, so it \n\
         \tCANNOT hide in the gain and the gain cannot hide in it.",
        radial.mean,
        radial.spread,
        radial.count,
        radial.signal(),
        2.0 * kjerag_render::band::WIDEST_DEG,
        100.0 * ((radial.mean * 2.0 * f64::from(kjerag_render::band::WIDEST_DEG)).exp() - 1.0),
        100.0 * (gain.mean.exp() - 1.0),
    );

    models(truth, options.gain);
    let [cb, cr] = truth.chroma();
    println!(
        "\nchroma: Cb {:+.2} codes (spread {:.2}, {:.1} se), Cr {:+.2} codes (spread {:.2}, \n\
         \t{:.1} se), over {} azimuth-frames. a chroma plane is 8 bit and signed about 128, \n\
         \tso a step under about 1 code is under the plane's own quantisation.",
        cb.mean,
        cb.spread,
        cb.signal(),
        cr.mean,
        cr.spread,
        cr.signal(),
        cb.count,
    );

    controls(&fields, &probes);
    Ok(())
}

/// Which description of the difference the data actually supports.
///
/// The whole of the design decision is in this table. A gain and an offset
/// look identical on any one patch and are told apart only by a fit that spans
/// brightnesses, so a correction chosen without this table is a correction
/// chosen by assumption.
fn models(field: &Field, shipped: Option<f64>) {
    println!(
        "\nmodels: what the two lenses' difference actually looks like, and what each \n\
         \tcandidate correction LEAVES of it. an exposure difference is a GAIN; veiling \n\
         \tglare, a black-level pedestal and a difference in the toe of the tone curve are \n\
         \tan OFFSET. the two are indistinguishable on any one patch and are told apart only \n\
         \tby a fit that spans brightnesses, which is what the last column is for.\n\
         \x20\tthe cut is by how far the alignment had to move lens 1 to make the content the \n\
         \tsame. it is not decoration: near-field content moves by degrees, is what the \n\
         \tdarkest patches are made of, and is where an alignment is hardest, so a term that \n\
         \tlives only in the near-field rows is a boot and not a camera."
    );
    // The shipped pass's own knee, imported rather than copied: 0.19 degrees
    // is 10 m at this baseline, it is what the band already switches time
    // constants at, and a cut here means this table is scoring the directions
    // the pass actually pools.
    let knee = f64::from(kjerag_render::band::NEAR_KNEE_DEG);
    for (name, cut) in [
        (
            format!("far field, under {knee:.2} deg - what the pass pools"),
            (0.0, knee),
        ),
        (
            format!("near field, over {knee:.2} deg"),
            (knee, f64::INFINITY),
        ),
    ] {
        let points = field.points(cut);
        let all = Model::all(&points);
        if all.is_empty() {
            println!("\n  {name}: nothing correlated in this cut.");
            continue;
        }
        let range = points.iter().fold((f64::MAX, f64::MIN), |held, p| {
            (held.0.min(p.0), held.1.max(p.0))
        });
        println!(
            "\n  {name}: {} azimuth-frames spanning {:.0} to {:.0} codes of lens 0.\n",
            points.len(),
            range.0,
            range.1,
        );
        println!(
            "  {:<30} {:>9} {:>9} {:>11} {:>10} {:>9}",
            "correction", "gain", "offset", "step codes", "step pct", "worst"
        );
        let mut all = all;
        if let Some(shipped) = shipped {
            // The number the GPU pass actually drew with, scored beside the
            // fits made here, so "the instrument and the pass agree" is a row
            // of this table rather than a claim about two separate runs.
            all.push(Model {
                name: "what the shipped pass drew with",
                gain: shipped.exp(),
                offset: 0.0,
                residual: 0.0,
            });
        }
        for model in &all {
            let (codes, percent, worst) = model.leaves(&points);
            println!(
                "  {:<30} {:>9.5} {:>9.3} {:>11.3} {:>10.2} {:>9.2}",
                model.name, model.gain, model.offset, codes, percent, worst,
            );
        }
    }
}

/// What each azimuth read, so a pooled number can be checked against the
/// things it was pooled from.
fn table(field: &Field, options: &Options) {
    if !options.verbose {
        println!("\n        (verbose=1 prints every azimuth's own reading.)");
        return;
    }
    println!(
        "\n    phi    mean0     mean1     ln ratio    percent      slope/deg      Cb       Cr   samples"
    );
    for (azimuth, columns) in &field.seen {
        let Some(held) = pooled(columns.iter().filter(|c| c.delta.abs() <= AT_SEAM_DEG)) else {
            continue;
        };
        println!(
            "{azimuth:>6} {:>8.2} {:>9.2} {:>12.4} {:>10.2} {:>14} {:>7.2} {:>8.2} {:>9.0}",
            held.mean0(),
            held.mean1(),
            held.log_ratio(),
            100.0 * (held.log_ratio().exp() - 1.0),
            slope(columns).map_or_else(|| "-".to_owned(), |s| format!("{s:+.5}")),
            (held.chroma[2] - held.chroma[0]) / held.chroma_count.max(1.0),
            (held.chroma[3] - held.chroma[1]) / held.chroma_count.max(1.0),
            held.count,
        );
    }
}

/// Every control, beside the number each one has to produce.
///
/// The columns are two **estimators** of the same thing, side by side, and
/// which of them ships is decided by the last block rather than by taste.
/// `mean` is the average of each azimuth's own log ratio; `totals` is the log
/// of all the lens-1 light over all the lens-0 light, which makes the whole
/// ring one window.
fn controls(fields: &[Field], probes: &[f64]) {
    let truth = fields[0].gain();
    println!(
        "\ncontrols. a gain column is a negative result until it is shown able to read a \n\
         positive one, and it is a confounded one until the confound is measured on its own. \n\
         every trial below runs the SAME code on the SAME frames: only the sampling \n\
         directions and one multiplier change.\n"
    );
    println!(
        "  {:<44} {:>10} {:>10} {:>12}",
        "trial", "mean ln", "totals ln", "expected"
    );
    let line = |name: &str, field: &Field, expected: &str| {
        println!(
            "  {name:<44} {:>10.4} {:>10.4} {:>12}",
            field.gain().mean,
            field.totals(),
            expected,
        );
    };
    line("the measurement", &fields[0], "-");
    line("the same patches, NOT lined up first", &fields[1], "-");
    line(
        "null: lens 0 on itself, same directions",
        &fields[2],
        "0.0000",
    );
    line(
        "null: lens 0 on itself, at the found shift",
        &fields[3],
        "0.0000",
    );
    for (index, inject) in [0.95f64, 1.05].iter().enumerate() {
        line(
            &format!("injected gain of {inject:.2} into lens 1"),
            &fields[4 + index],
            &format!("{:+.4}", inject.ln()),
        );
    }
    for (index, nudge) in probes.iter().enumerate() {
        line(
            &format!("alignment nudged {nudge:+.1} deg across the seam"),
            &fields[6 + index],
            "the measurement",
        );
    }
    sensitivity(fields, probes, truth);
}

/// What an alignment error costs each estimator, and what that leaves of the
/// reading.
///
/// This is the block the design comes out of. A brightness read off a window
/// that has been displaced by `e` is wrong by `e` times the mean log gradient
/// across that window, and that error is indistinguishable from exposure in
/// any single reading. What is NOT indistinguishable is how fast it grows with
/// `e`, which is measured here by moving the alignment on purpose.
fn sensitivity(fields: &[Field], probes: &[f64], truth: Reading) {
    let slope = |read: &dyn Fn(&Field) -> f64| {
        let low = probes
            .iter()
            .position(|p| *p < 0.0)
            .map(|index| (probes[index], read(&fields[6 + index])));
        let high = probes
            .iter()
            .rposition(|p| *p > 0.0)
            .map(|index| (probes[index], read(&fields[6 + index])));
        match (low, high) {
            (Some(low), Some(high)) if high.0 > low.0 => (high.1 - low.1) / (high.0 - low.0),
            _ => f64::NAN,
        }
    };
    let by_mean = slope(&|field: &Field| field.gain().mean);
    let by_totals = slope(&|field: &Field| field.totals());
    let totals = fields[0].totals();
    println!(
        "\n  {:<44} {:>10} {:>10}",
        "how much an alignment error costs", "mean ln", "totals ln"
    );
    println!(
        "  {:<44} {by_mean:>10.4} {by_totals:>10.4}",
        "per degree of misalignment"
    );
    // The far-field flicker the band leaves, which is the alignment error the
    // shipped pass actually has: 0.02 deg rms (6.9's own table, worst column).
    let residual = 0.02;
    println!(
        "  {:<44} {:>10.5} {:>10.5}",
        format!("at the band's own {residual:.2} deg residual"),
        by_mean.abs() * residual,
        by_totals.abs() * residual,
    );
    println!(
        "  {:<44} {:>10.4} {:>10.4}",
        "the reading itself", truth.mean, totals,
    );
    println!(
        "  {:<44} {:>10.1} {:>10.1}",
        "reading over confound",
        truth.mean.abs() / (by_mean.abs() * residual).max(f64::MIN_POSITIVE),
        totals.abs() / (by_totals.abs() * residual).max(f64::MIN_POSITIVE),
    );
    println!(
        "\n  the second null above holds ALL of the misregistration and none of the exposure: \n\
         it is one lens against its own picture, displaced by the very shift the alignment \n\
         found. read beside the measurement it says how much of what is being called exposure \n\
         is the window having moved."
    );
}

// ------------------------------------------------------------ the annulus

/// The estimator the format study's 6.3 used, run again so that this stage's
/// answer and that one can be told apart rather than merely disagreed with.
///
/// 6.3 took the mean luma of a ring of **pixels** in each lens's delivered
/// frame, at radii where "both lenses have the ray", and called their ratio
/// the brightness step, on the argument that the two rings hold the same
/// world directions permuted by roll and that vignetting is radial and
/// identical in both so it cancels. Two things were not known when that was
/// written: the calibration is out by 2.4 degrees at the seam (issue #48,
/// merged after), so the two rings do NOT hold the same directions; and the
/// two radii quoted are different radii, so a rolloff does not cancel between
/// them either.
fn annulus(options: &Options) -> Fallible<()> {
    let (calibration, _, frame) = calibrated(options)?;
    let mut walk = Walk::open(&options.input, options.from, frame)?;
    if walk.streams() < 2 {
        return Err("this file carries one lens stream, so it has no seam".into());
    }
    println!(
        "\nannulus: the 6.3 estimator - mean luma of a ring of PIXELS in each lens, no \n\
         \talignment and no correspondence, on the same frames the field mode reads.\n"
    );
    println!(
        "  {:>7} {:>10} {:>10} {:>11} {:>10}",
        "frame", "mean0", "mean1", "ln ratio", "percent"
    );
    let radii = [(1680.0, 1913.0), (1670.0, 1905.0)];
    let mut readings = Vec::new();
    for index in 0..options.count {
        let Some(pair) = walk.next_pair()? else {
            break;
        };
        let means: Vec<f64> = (0..2)
            .map(|lens| ring_mean(&pair.lenses[lens], &calibration.lenses[lens], radii[lens]))
            .collect();
        let ratio = (means[1] / means[0]).ln();
        readings.push(ratio);
        println!(
            "  {index:>7} {:>10.3} {:>10.3} {:>11.4} {:>10.2}",
            means[0],
            means[1],
            ratio,
            100.0 * (ratio.exp() - 1.0),
        );
    }
    let held = Reading::of(readings.into_iter().enumerate());
    println!(
        "\n  pooled {:+.4} ln, {:+.2} percent, spread {:.4} over {} frames.",
        held.mean,
        100.0 * (held.mean.exp() - 1.0),
        held.spread,
        held.count,
    );
    Ok(())
}

/// Mean luma of one lens's annulus about its own principal point.
///
/// The principal point rather than the middle of the frame, because that is
/// what a radius about a lens means; `kjerag-meta` has already put both in
/// delivered-frame pixels, so nothing here rescales.
fn ring_mean(plane: &Plane, lens: &Lens, radii: (f64, f64)) -> f64 {
    let (cx, cy) = (lens.intrinsics.cx, lens.intrinsics.cy);
    let mut sum = 0.0;
    let mut count = 0.0;
    for step in 0..3600 {
        let phi = step as f64 / 3600.0 * std::f64::consts::TAU;
        let (sin, cos) = phi.sin_cos();
        let mut radius = radii.0;
        while radius <= radii.1 {
            if let Some(code) = plane.at(cx + radius * cos, cy + radius * sin) {
                sum += code;
                count += 1.0;
            }
            radius += 4.0;
        }
    }
    match count > 0.0 {
        true => sum / count,
        false => 0.0,
    }
}

// ------------------------------------------------------------ the picture

/// One view, drawn, with what its luma does as it crosses the seam.
///
/// The profile is the acceptance evidence: what the eye catches at a seam is
/// a step in a picture, and the picture is what this measures. A scene has its
/// own brightness gradient, so the seam's own step is read against a
/// **decoy**: the same profile taken about a great circle 90 degrees away,
/// where there is no handover at all and whatever the statistic reports is
/// the scene.
fn render(options: &Options) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let out = options.out();
    std::fs::create_dir_all(&out)?;
    let size = Size::new(options.size, options.size);

    // The same instant, the same view, the same run of frames, twice: the two
    // pictures differ by this stage and by nothing else.
    let draw = |held: bool| -> Fallible<(Picture, Reframe, kjerag_render::Tone, Scene)> {
        let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
        pipeline.hold_tone(held);
        let mut scene = Scene::still(
            &options.input,
            Cue::Time(std::time::Duration::from_secs_f64(options.from)),
        )?;
        scene.set_horizon(match options.lock {
            true => Horizon::Locked,
            false => Horizon::Free,
        });
        scene.fit_seam(true);
        let mut shot = None;
        for _ in 0..options.count.max(1) {
            shot = Some(
                Render {
                    gpu: &gpu,
                    scene: &scene,
                    pipeline: &mut pipeline,
                }
                .frame(options.camera(), Sampling::default(), size)?,
            );
            if !scene.advance()? {
                break;
            }
        }
        let mapped = scene
            .mapped(options.camera(), 1.0)
            .ok_or("no frame to map")?;
        let tone = pipeline.band_tone(&gpu.device, &gpu.queue)?;
        Ok((
            shot.ok_or("no frame decoded at that instant")?,
            mapped,
            tone,
            scene,
        ))
    };
    let (before, mapped, _, _) = draw(true)?;
    let (after, _, tone, scene) = draw(false)?;

    let stem = format!("{}-{}", options.stem(), options.tag);
    before.save(&gpu, &out.join(format!("{stem}-1-before.png")))?;
    after.save(&gpu, &out.join(format!("{stem}-2-after.png")))?;
    after
        .amplified(&before)
        .save(&gpu, &out.join(format!("{stem}-3-what-moved.png")))?;
    println!(
        "\nwrote three pictures into {} at yaw {:.2}, pitch {:.2}, fov {:.2}, {} frames in.\n\
         gain:   {:+.5} ln, {:+.3} percent, evidence {:.3}. lens 0 is multiplied by {:.5} and \n\
         \tlens 1 by {:.5}.\n{}",
        out.display(),
        options.yaw,
        options.pitch,
        options.fov,
        options.count.max(1),
        tone.log_gain,
        100.0 * (f64::from(tone.log_gain).exp() - 1.0),
        tone.evidence,
        tone.split()[0],
        tone.split()[1],
        after.against(&before).report(),
    );
    for (name, picture) in [("before", &before), ("after", &after)] {
        println!("\n=== {name} ===");
        profile(&mapped, picture, size);
    }
    seam_views(&scene, options);
    Ok(())
}

/// Mean luma against distance past the seam, and the **discontinuity** at it.
///
/// The statistic is not a difference of the two sides' means: at a wide view
/// one side of the seam can be sky and the other soil, and their difference is
/// twenty codes of scenery. What an eye catches at a seam is a step, and a
/// step is what a smooth profile does NOT have, so each side's own trend is
/// fitted over the degrees where only one lens is drawing and the two are
/// extrapolated to the seam. A pure scene gradient extrapolates to the same
/// number from both sides and reports zero; a handover that changes brightness
/// does not.
///
/// The **decoy** is the same statistic about a great circle 90 degrees away,
/// where the two lenses do not hand over at all. It is what the scene's own
/// curvature contributes to a number like this, measured rather than assumed,
/// and the seam's step means nothing without it.
fn profile(reframe: &Reframe, picture: &Picture, size: Size) {
    let luma = picture.luma();
    let seam = distances(reframe, size, 2);
    let decoy = distances(reframe, size, 0);
    println!(
        "\nprofile: mean luma of the drawn picture against how far past the seam it is, in \n\
         \tcodes of 255. the seam is at 0 and lens 0 is the positive side.\n"
    );
    println!(
        "  {:>10} {:>10} {:>9} {:>12} {:>9}",
        "degrees", "luma", "pixels", "decoy luma", "pixels"
    );
    for step in -8..=8 {
        let band = (f64::from(step) - 0.5, f64::from(step) + 0.5);
        let here = strip(&luma, &seam, band);
        let there = strip(&luma, &decoy, band);
        println!(
            "  {step:>10} {:>10} {:>9} {:>12} {:>9}",
            here.0.map_or_else(|| "-".to_owned(), |v| format!("{v:.2}")),
            here.1,
            there
                .0
                .map_or_else(|| "-".to_owned(), |v| format!("{v:.2}")),
            there.1,
        );
    }
    let step = |at: &[Option<f64>]| {
        let low = trend(&luma, at, (-8.0, -1.5))?;
        let high = trend(&luma, at, (1.5, 8.0))?;
        Some(high - low)
    };
    println!(
        "\n  step across the seam, each side's trend extrapolated to it: {}\n\
         \x20 the same statistic at the decoy circle:                     {}",
        step(&seam).map_or_else(
            || "no pixels either side".to_owned(),
            |codes| format!("{codes:+.3} codes"),
        ),
        step(&decoy).map_or_else(
            || "no pixels either side".to_owned(),
            |codes| format!("{codes:+.3} codes"),
        ),
    );
}

/// How far past a great circle each output pixel is, in degrees, or `None`
/// where no lens has the ray.
///
/// `axis` 2 is the seam, which is the plane the two lenses hand over across;
/// 0 is the decoy, a great circle through both lens axes where there is no
/// handover at all and the picture is one lens's on both sides of it.
fn distances(reframe: &Reframe, size: Size, axis: usize) -> Vec<Option<f64>> {
    let width = size.width as usize;
    (0..(size.width * size.height) as usize)
        .map(|index| {
            let uv = [
                (index % width) as f32 / size.width as f32,
                (index / width) as f32 / size.height as f32,
            ];
            let ray = reframe.view_ray(uv)?;
            let body = reframe.body_ray(ray);
            let length = (body[0] * body[0] + body[1] * body[1] + body[2] * body[2]).sqrt();
            (length > 0.0).then(|| f64::from((body[axis] / length).asin().to_degrees()))
        })
        .collect()
}

/// One side's mean luma trend, extrapolated to the circle, in codes.
///
/// A straight line through the per-degree means over `band`. Straight because
/// the extrapolation is only ever 1.5 degrees and because a curve fitted to
/// eight points would follow the scenery it is meant to be looking past.
fn trend(luma: &[f32], distance: &[Option<f64>], band: (f64, f64)) -> Option<f64> {
    let mut rows: Vec<(f64, f64)> = Vec::new();
    let mut at = band.0;
    while at <= band.1 {
        if let (Some(mean), _) = strip(luma, distance, (at - 0.25, at + 0.25)) {
            rows.push((at, mean));
        }
        at += 0.5;
    }
    if rows.len() < 4 {
        return None;
    }
    let n = rows.len() as f64;
    let mean_x = rows.iter().map(|r| r.0).sum::<f64>() / n;
    let mean_y = rows.iter().map(|r| r.1).sum::<f64>() / n;
    let mut covariance = 0.0;
    let mut variance = 0.0;
    for (x, y) in &rows {
        covariance += (x - mean_x) * (y - mean_y);
        variance += (x - mean_x).powi(2);
    }
    (variance > 0.0).then(|| mean_y - covariance / variance * mean_x)
}

/// Mean luma of the pixels whose distance falls inside `band`, and how many.
fn strip(luma: &[f32], distance: &[Option<f64>], band: (f64, f64)) -> (Option<f64>, usize) {
    let mut total = 0.0;
    let mut count = 0usize;
    for (index, value) in luma.iter().enumerate() {
        let Some(at) = distance[index] else {
            continue;
        };
        if !(band.0..=band.1).contains(&at) || *value <= 0.0 {
            continue;
        }
        total += f64::from(*value);
        count += 1;
    }
    match count > 0 {
        true => (Some(total / count as f64), count),
        false => (None, count),
    }
}

/// Views whose centre looks straight at the seam, as `reframe`'s own
/// arguments.
///
/// A wide view puts the seam somewhere across the frame and mixes it with
/// everything else in the sphere; a narrow one centred on it is where a step
/// is looked at rather than hunted for. The pitch is scanned rather than
/// solved because the horizon lock turns the body under the view, so which
/// pitch lands on the seam is a question about this frame.
fn seam_views(scene: &Scene, options: &Options) {
    println!(
        "\nviews:  narrow views centred on the seam at this instant, as reframe's own \n\
         \targuments. the yaw is swept and the pitch solved for by bisection on the \n\
         \tcentre ray's own distance from the handover plane.\n"
    );
    for turn in 0..8 {
        let yaw = options.yaw + f64::from(turn) * 45.0;
        let Some(pitch) = on_seam(scene, yaw) else {
            continue;
        };
        println!(
            "  time={:.3} yaw={:.2} pitch={:.2} fov=40 lock={}",
            options.from,
            yaw,
            pitch,
            u8::from(options.lock),
        );
    }
}

/// The pitch at which this yaw's centre ray lies on the seam, or `None` where
/// no pitch in reach does.
fn on_seam(scene: &Scene, yaw: f64) -> Option<f64> {
    let past = |pitch: f64| {
        let turned = scene.mapped(
            Camera {
                yaw: yaw.to_radians() as f32,
                pitch: pitch.to_radians() as f32,
                fov: 0.7,
            },
            1.0,
        )?;
        let body = turned.body_ray([0.0, 0.0, 1.0]);
        let length = (body[0] * body[0] + body[1] * body[1] + body[2] * body[2]).sqrt();
        Some(f64::from(body[2] / length))
    };
    let (mut low, mut high) = (-89.0f64, 89.0f64);
    if past(low)?.signum() == past(high)?.signum() {
        return None;
    }
    for _ in 0..40 {
        let middle = 0.5 * (low + high);
        match past(middle)?.signum() == past(low)?.signum() {
            true => low = middle,
            false => high = middle,
        }
    }
    Some(0.5 * (low + high))
}

// ------------------------------------------------------------ the trace

/// What the shipped pass's own pooled gain does frame to frame.
///
/// A pumping exposure is worse than a step: a step is still and an eye stops
/// seeing it, and a brightness that breathes is motion where the scene has
/// none. So the shipped number is watched over a run rather than sampled.
fn trace(options: &Options) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let mut scene = Scene::still(
        &options.input,
        Cue::Time(std::time::Duration::from_secs_f64(options.from)),
    )?;
    scene.set_horizon(match options.lock {
        true => Horizon::Locked,
        false => Horizon::Free,
    });
    scene.fit_seam(true);
    let size = Size::new(256, 256);
    let mut held: Vec<(f64, f64, f64)> = Vec::new();
    while let Some((_, at)) = scene.frame() {
        Render {
            gpu: &gpu,
            scene: &scene,
            pipeline: &mut pipeline,
        }
        .frame(options.camera(), Sampling::default(), size)?;
        let tone = pipeline.band_tone(&gpu.device, &gpu.queue)?;
        held.push((
            at.as_secs_f64(),
            f64::from(tone.log_gain),
            f64::from(tone.evidence),
        ));
        if held.len() >= options.count || !scene.advance()? {
            break;
        }
    }
    println!(
        "\ntrace:  the pooled gain the shipped pass drew each frame with, over {} frames \n\
         \tfrom {:.3} s.\n",
        held.len(),
        options.from,
    );
    println!(
        "  {:>9} {:>11} {:>10} {:>11}",
        "at", "ln gain", "percent", "evidence"
    );
    for (at, gain, evidence) in &held {
        println!(
            "  {at:>8.2}s {gain:>11.5} {:>10.3} {evidence:>11.3}",
            100.0 * (gain.exp() - 1.0),
        );
    }
    let stepped = |shake: f64| {
        let steps: Vec<f64> = held
            .windows(2)
            .enumerate()
            .map(|(index, pair)| {
                let shaken = |at: usize, value: f64| match at % 2 {
                    0 => value + shake,
                    _ => value - shake,
                };
                (shaken(index + 1, pair[1].1) - shaken(index, pair[0].1)).abs()
            })
            .collect();
        let rms = (steps.iter().map(|s| s * s).sum::<f64>() / steps.len().max(1) as f64).sqrt();
        (rms, steps.iter().fold(0.0, |held: f64, s| held.max(*s)))
    };
    let (flicker, worst) = stepped(0.0);
    // One code at a mid grey of 128 is ln(129/128).
    let one_code = (129.0f64 / 128.0).ln();
    println!(
        "\n  frame to frame: {:.6} ln rms, which is {:.4} percent of brightness, and a worst \n\
         \x20 single step of {:.6} ln, {:.4} percent. one code at a mid grey of 128 is {:.4} ln, \n\
         \x20 so the rms is {:.0}x under what an 8-bit picture can carry and the worst single \n\
         \x20 step is {:.0}x under it. a gain that cannot move one code between two frames \n\
         \x20 cannot pump.",
        flicker,
        100.0 * flicker,
        worst,
        100.0 * worst,
        one_code,
        one_code / flicker.max(f64::MIN_POSITIVE),
        one_code / worst.max(f64::MIN_POSITIVE),
    );
    println!(
        "\n  the positive control, which this column means nothing without: a known step put \n\
         \x20 in each frame with alternating sign has to come back at 2s, in quadrature with \n\
         \x20 what was already there.\n\n\
         \x20            step        read    expected"
    );
    for step in [0.002f64, 0.010] {
        println!(
            "         {step:>9.4} {:>11.5} {:>11.5}",
            stepped(step).0,
            flicker.hypot(2.0 * step),
        );
    }
    Ok(())
}

// ------------------------------------------------------------ options

struct Options {
    input: PathBuf,
    mode: Mode,
    from: f64,
    count: usize,
    /// How many places in the file the run is spread over.
    places: usize,
    /// A gain to score beside the fitted ones, as a natural log: what the
    /// shipped pass read on the same footage.
    gain: Option<f64>,
    patches: usize,
    keep: f64,
    fit: bool,
    verbose: bool,
    yaw: f64,
    pitch: f64,
    fov: f64,
    size: u32,
    lock: bool,
    out: Option<PathBuf>,
    tag: String,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            mode: Mode::Field,
            from: 0.0,
            count: 8,
            places: 1,
            gain: None,
            patches: 72,
            keep: 0.80,
            fit: true,
            verbose: false,
            yaw: 90.0,
            pitch: 0.0,
            fov: 60.0,
            size: 1024,
            lock: true,
            out: None,
            tag: "view".to_owned(),
        };
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("mode", value)) => {
                    options.mode = match value {
                        "field" => Mode::Field,
                        "annulus" => Mode::Annulus,
                        "render" => Mode::Render,
                        "trace" => Mode::Trace,
                        _ => return Err(format!("no mode called {value}").into()),
                    }
                }
                Some(("from", value)) => options.from = value.parse()?,
                Some(("count", value)) => options.count = value.parse()?,
                Some(("places", value)) => options.places = value.parse()?,
                Some(("gain", value)) => options.gain = Some(value.parse()?),
                Some(("patches", value)) => options.patches = value.parse()?,
                Some(("keep", value)) => options.keep = value.parse()?,
                Some(("seam", value)) => options.fit = value != "factory",
                Some(("verbose", value)) => options.verbose = value.parse::<u32>()? != 0,
                Some(("yaw", value)) => options.yaw = value.parse()?,
                Some(("pitch", value)) => options.pitch = value.parse()?,
                Some(("fov", value)) => options.fov = value.parse()?,
                Some(("size", value)) => options.size = value.parse()?,
                Some(("lock", value)) => options.lock = value.parse::<u32>()? != 0,
                Some(("out", value)) => options.out = Some(PathBuf::from(value)),
                Some(("tag", value)) => options.tag = value.to_owned(),
                Some((key, _)) => return Err(format!("no argument called {key}").into()),
            }
        }
        if options.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        Ok(options)
    }

    fn probe(&self) -> Probe {
        Probe {
            patches: self.patches,
            keep: self.keep,
            ..Probe::default()
        }
    }

    fn camera(&self) -> Camera {
        Camera {
            yaw: self.yaw.to_radians() as f32,
            pitch: self.pitch.to_radians() as f32,
            fov: self.fov.to_radians() as f32,
        }
    }

    fn out(&self) -> PathBuf {
        self.out.clone().unwrap_or_else(|| PathBuf::from("scratch"))
    }

    fn stem(&self) -> String {
        self.input
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    }
}

const USAGE: &str = "usage: expose <file.insv> [mode=field|annulus|render|trace] [from=seconds] \
     [count=frames] [places=n] [gain=ln] [patches=n] [keep=r] [seam=factory] [verbose=1] [yaw=deg] [pitch=deg] \
     [fov=deg] [size=px] [lock=0] [out=dir] [tag=name]";
