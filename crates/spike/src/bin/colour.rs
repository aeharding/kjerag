//! What the two lenses' pictures of the **same content** differ by in each
//! colour channel, and how that difference behaves across the overlap band
//! (issue #103, stage 7).
//!
//! ```sh
//! # the decomposition round the ring: per channel, per content class, with every control
//! cargo run --release -p kjerag-spike --bin colour -- <file.insv> from=488.855 count=8
//! # what a drawn view's three channels do as they cross the seam - the acceptance evidence
//! cargo run --release -p kjerag-spike --bin colour -- <file.insv> mode=profile \
//!   from=488.855 yaw=67.24 pitch=2.56 fov=60 out=scratch/stage7
//! # the same statistic on somebody else's stitch, from an equirectangular export
//! cargo run --release -p kjerag-spike --bin colour -- <export.mp4> mode=studio at=12.0
//! ```
//!
//! **Stage 3 measured brightness; this measures colour.** The two are not the
//! same question and the difference is the whole charter: stage 3's correction
//! is one number applied to all three channels, so whatever the two lenses
//! disagree about that is not common to R, G and B survives it exactly, and
//! what survives a brightness correction is a **hue** step. Stage 3 measured
//! that residue at 2.3 codes in one channel and declined it as under the chroma
//! plane's own resolution. The owner's eye has since named it the worst thing
//! left at the seam.
//!
//! **Two things here are new and both are about content the earlier instrument
//! could not read.**
//!
//! - **Flat content.** The band correlates on texture and refuses a patch with
//!   under [`CONTRAST`] codes of standard deviation in it, so a seam that is
//!   mostly sky is a seam the pass measures almost nothing on - and sky is
//!   where the owner sees the defect. A photometric reading needs an alignment
//!   only in proportion to the content's own gradient: what a displacement of
//!   `e` degrees costs is `e` times the gradient across the window, so on the
//!   flattest content it costs the least. That is not an argument to be taken
//!   on trust, and it is not taken on trust here: [`Trial`]'s nulls read one
//!   lens against its own picture displaced by exactly the residual the shipped
//!   pass leaves, on the very same patches, and report per channel what that
//!   displacement is worth.
//! - **Per channel, in the space the correction is applied in.** The samples
//!   are decoded to gamma-coded R, G and B through the fragment shader's own
//!   BT.709 matrix, because that is what the pass multiplies and what an eye
//!   reads. Cb and Cr are reported beside them for continuity with stage 3.
//!
//! PNGs land in gitignored `scratch/`: these are frames of somebody's real
//! flights and this repo is public.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use kjerag_media::{Fallible, Pair, Plane, Walk};
use kjerag_meta::{CalibrationSet, Lens};
use kjerag_render::seam::{self, Probe, Refused, Where};
use kjerag_render::{Camera, Cue, Horizon, Reframe, Sampling, Scene, ScenePipeline, Size};
use kjerag_spike::{FORMAT, Gpu, Picture, Render};

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    match options.mode {
        Mode::Field => field(&options),
        Mode::Profile => profile(&options),
        Mode::Studio => studio(&options),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// The per-channel decomposition round the seam ring, and its controls.
    Field,
    /// One drawn view, and what each channel does as it crosses the seam.
    Profile,
    /// Somebody else's stitch, measured across their own seam.
    Studio,
}

// ------------------------------------------------------------ the sampling

/// How wide a photometric patch is along the seam, in degrees, and how finely
/// it is sampled. The band's own patch, so what this reads is what the shipped
/// pass could read and not a more generous version of it.
const ALONG_DEG: f64 = 2.0;
const ALONG_STEP_DEG: f64 = 0.1;

/// How far either side of the seam the columns reach, and how far apart they
/// are, in degrees.
///
/// Bounded by the optics: the fixture's two lenses overlap by 14.4 degrees,
/// 7.2 a side, and a column further out than that is one a lens has no picture
/// of. 5 leaves room for the alignment shift on top of it, and it covers the
/// whole of the widest crossover the pass can open
/// ([`kjerag_render::band::WIDEST_DEG`] is 2.9).
const ACROSS_DEG: f64 = 5.0;
const ACROSS_STEP_DEG: f64 = 0.25;

/// Which columns count as "at the seam", in degrees either side. The band's
/// patch reaches one degree either way.
const AT_SEAM_DEG: f64 = 1.0;

/// The codes either side of which a sample is not a measurement of brightness.
/// A clipped highlight has no ratio, and the pair is dropped together so that
/// nothing is biased by dropping it.
const CEILING: f64 = 252.0;
const FLOOR: f64 = 2.0;

/// How much picture a patch needs before the band will correlate on it, in
/// codes of standard deviation.
///
/// [`kjerag_render::band`]'s own gate, imported as a number here because it is
/// what divides this instrument's two content classes: a patch under it is one
/// the shipped pass measures **nothing** on, and on a real seam most of the
/// ring is that patch.
const CONTRAST: f64 = 6.0;

/// The three channels, in the order everything below reports them.
const CHANNELS: [&str; 3] = ["R", "G", "B"];

/// One lens's picture of one direction: gamma-coded R, G and B in codes, and
/// the luma they were decoded from.
#[derive(Clone, Copy, Default)]
struct Look {
    rgb: [f64; 3],
    luma: f64,
    chroma: [f64; 2],
}

/// One across-seam column of one azimuth: both lenses' pictures of one offset
/// from the seam, pooled over the along-seam samples.
#[derive(Clone, Copy, Default)]
struct Column {
    /// How far past the seam, in degrees, positive towards lens 0.
    delta: f64,
    count: f64,
    /// Per channel, per lens: the sum of the samples.
    sum: [[f64; 3]; 2],
    /// The same for luma and for the two chroma channels, lens 0 then lens 1.
    luma: [f64; 2],
    /// Sum of squares of lens 0's luma, which is what the texture test reads.
    luma_square: f64,
    chroma: [[f64; 2]; 2],
}

impl Column {
    fn mean(&self, lens: usize, channel: usize) -> f64 {
        self.sum[lens][channel] / self.count
    }

    /// Lens 0's own standard deviation over this column's samples, in codes:
    /// how much picture there is here to line two lenses up on.
    fn texture(&self) -> f64 {
        let mean = self.luma[0] / self.count;
        (self.luma_square / self.count - mean * mean)
            .max(0.0)
            .sqrt()
    }

    fn add(&mut self, other: &Self) {
        self.count += other.count;
        self.luma_square += other.luma_square;
        for lens in 0..2 {
            self.luma[lens] += other.luma[lens];
            for channel in 0..3 {
                self.sum[lens][channel] += other.sum[lens][channel];
            }
            for channel in 0..2 {
                self.chroma[lens][channel] += other.chroma[lens][channel];
            }
        }
    }
}

/// What one run of the photometry is run on.
///
/// Every control is one of these rather than a second code path, which is
/// stage 3's rule and the reason it is kept: a control that runs different code
/// proves the control works.
#[derive(Clone, Copy)]
struct Trial {
    /// Which lens the second side is sampled from. 1 is the measurement; 0 is
    /// the null, where the answer is exactly zero in every channel because it
    /// is the same picture of the same directions.
    back: usize,
    /// Whether the alignment the correlation found is applied to the second
    /// side's sampling directions.
    aligned: bool,
    /// Added to that alignment, in degrees along and across.
    nudge: (f64, f64),
    /// Per channel, what the second side's samples are multiplied by and then
    /// added to: the positive controls. A known gain and a known offset have to
    /// come back as themselves, in the channel they were put in and in no
    /// other.
    gain: [f64; 3],
    offset: [f64; 3],
}

impl Trial {
    const TRUTH: Self = Self {
        back: 1,
        aligned: true,
        nudge: (0.0, 0.0),
        gain: [1.0; 3],
        offset: [0.0; 3],
    };
}

/// One azimuth's columns, or `None` where one of the two lenses has no picture
/// of the patch.
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
            // A direction one lens has no picture of is not a pair. Dropped on
            // both sides at once, so what is left is still the same content in
            // both and nothing is biased by what went.
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
            if !(FLOOR..=CEILING).contains(&front.luma) || !(FLOOR..=CEILING).contains(&back.luma) {
                continue;
            }
            held.count += 1.0;
            held.luma[0] += front.luma;
            held.luma[1] += back.luma;
            held.luma_square += front.luma * front.luma;
            for channel in 0..3 {
                held.sum[0][channel] += front.rgb[channel];
                held.sum[1][channel] +=
                    back.rgb[channel] * trial.gain[channel] + trial.offset[channel];
            }
            for channel in 0..2 {
                held.chroma[0][channel] += front.chroma[channel];
                held.chroma[1][channel] += back.chroma[channel];
            }
        }
        if held.count > 0.0 {
            out.push(held);
        }
    }
    (out.len() > 2).then_some(out)
}

/// One lens's colour at one direction off the seam, or `None` where that lens
/// has no picture there or the frame carries no chroma plane.
///
/// Decoded through the fragment shader's own BT.709 full-range matrix, in the
/// video's own gamma-coded space, because that is the space the correction is
/// applied in and the space an eye reads. No transfer function is assumed at
/// either end.
fn look(
    reframe: &Reframe,
    planes: &[Plane],
    lens: usize,
    at: &Where,
    offset: (f64, f64),
) -> Option<Look> {
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
    let luma = plane.at(x, y)?;
    let (cb, cr) = plane.chroma_at(x, y)?;
    Some(Look {
        rgb: [
            luma + 1.5748 * cr,
            luma - 0.1873 * cb - 0.4681 * cr,
            luma + 1.8556 * cb,
        ],
        luma,
        chroma: [cb, cr],
    })
}

// ------------------------------------------------------------ the pooling

/// One azimuth-frame that read: its columns, how far the alignment had to move
/// lens 1, and what kind of content it is.
struct Seen {
    azimuth: usize,
    columns: Vec<Column>,
    /// How far across the seam the alignment moved lens 1, in degrees. This is
    /// the quantity the shipped pass gates the exposure on.
    across: f64,
    /// Lens 0's standard deviation over the at-seam columns, in codes.
    texture: f64,
    /// Which lens, if either, had the sun in it on this frame.
    sun: Option<usize>,
}

impl Seen {
    /// The at-seam columns as one.
    fn at_seam(&self) -> Option<Column> {
        pooled(self.columns.iter().filter(|c| c.delta.abs() <= AT_SEAM_DEG))
    }

    /// Whether this is content the band can measure on at all.
    fn textured(&self) -> bool {
        self.texture >= CONTRAST
    }
}

/// Every azimuth-frame a run read, plus what was refused.
#[derive(Default)]
struct Field {
    seen: Vec<Seen>,
    frames: usize,
    refused: usize,
    /// How many directions round the seam were tried, so an azimuth index can
    /// be turned back into the angle a ring fit needs.
    azimuths: usize,
}

impl Field {
    /// One channel's readings over a class of content: lens 0's mean, lens 1's
    /// mean, and how many samples are behind them.
    fn points(&self, class: Class, channel: usize) -> Vec<(f64, f64, f64)> {
        self.seen
            .iter()
            .filter(|seen| class.holds(seen))
            .filter_map(|seen| {
                let held = seen.at_seam()?;
                Some((held.mean(0, channel), held.mean(1, channel), held.count))
            })
            .collect()
    }

    /// The step at the seam in one channel, in codes, one reading per
    /// azimuth-frame.
    fn step(&self, class: Class, channel: usize) -> Reading {
        Reading::of(
            self.seen
                .iter()
                .filter(|s| class.holds(s))
                .filter_map(|seen| {
                    let held = seen.at_seam()?;
                    Some((seen.azimuth, held.mean(1, channel) - held.mean(0, channel)))
                }),
        )
    }

    /// One channel's step averaged per azimuth: the azimuth in radians, the
    /// step in codes, and how many samples are behind it.
    ///
    /// Per azimuth rather than per reading, because the same direction on two
    /// consecutive frames is one measurement made twice and a ring fit weighted
    /// by readings would weigh the frames rather than the circle.
    fn by_azimuth(&self, class: Class, channel: usize) -> Vec<(f64, f64, f64)> {
        let mut held: Vec<(usize, f64, f64)> = Vec::new();
        for seen in self.seen.iter().filter(|s| class.holds(s)) {
            let Some(column) = seen.at_seam() else {
                continue;
            };
            let step = column.mean(1, channel) - column.mean(0, channel);
            if !step.is_finite() {
                continue;
            }
            match held.iter_mut().find(|entry| entry.0 == seen.azimuth) {
                Some(entry) => {
                    entry.1 += step * column.count;
                    entry.2 += column.count;
                }
                None => held.push((seen.azimuth, step * column.count, column.count)),
            }
        }
        held.into_iter()
            .map(|(azimuth, total, weight)| {
                (
                    azimuth as f64 / self.azimuths as f64 * std::f64::consts::TAU,
                    total / weight,
                    weight,
                )
            })
            .collect()
    }

    /// What the same azimuth reads on consecutive frames, in codes rms: this
    /// instrument's own noise, which every fit above has to beat before it
    /// means anything.
    fn repeatability(&self, class: Class, channel: usize) -> f64 {
        let mut groups: Vec<(usize, Vec<f64>)> = Vec::new();
        for seen in self.seen.iter().filter(|s| class.holds(s)) {
            let Some(column) = seen.at_seam() else {
                continue;
            };
            let step = column.mean(1, channel) - column.mean(0, channel);
            if !step.is_finite() {
                continue;
            }
            match groups.iter_mut().find(|entry| entry.0 == seen.azimuth) {
                Some(entry) => entry.1.push(step),
                None => groups.push((seen.azimuth, vec![step])),
            }
        }
        let mut error = 0.0;
        let mut count = 0.0;
        for (_, values) in groups.iter().filter(|group| group.1.len() > 1) {
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            for value in values {
                error += (value - mean).powi(2);
            }
            count += (values.len() - 1) as f64;
        }
        match count > 0.0 {
            true => (error / count).sqrt(),
            false => f64::NAN,
        }
    }

    /// How the step in one channel slopes across the band, in codes per
    /// degree.
    ///
    /// **This is the question a crossover width answers and a gain does not.**
    /// A difference that is one number everywhere across the overlap is a
    /// property of the two cameras and a single correction reaches it exactly.
    /// A difference that slopes is a rolloff, and a correction that is one
    /// number leaves the slope behind whatever it does to the mean.
    fn radial(&self, class: Class, channel: usize) -> Reading {
        Reading::of(
            self.seen
                .iter()
                .filter(|s| class.holds(s))
                .filter_map(|seen| Some((seen.azimuth, slope(&seen.columns, channel)?))),
        )
    }
}

/// Which content a reading is taken on.
///
/// The classes are measured properties of the patch and not a judgement about
/// the scene: whether there is enough picture in it for the band to correlate,
/// and whether either lens had the sun in it on that frame. Those are the two
/// axes the owner's complaint names.
#[derive(Clone, Copy, PartialEq)]
enum Class {
    All,
    /// Under the band's own contrast gate: the content the shipped pass reads
    /// nothing on, which on a real seam is most of the sky.
    Flat,
    Textured,
    /// Frames where one lens is looking at the sun and the other is not.
    Sun,
    NoSun,
}

impl Class {
    fn holds(self, seen: &Seen) -> bool {
        match self {
            Self::All => true,
            Self::Flat => !seen.textured(),
            Self::Textured => seen.textured(),
            Self::Sun => seen.sun.is_some(),
            Self::NoSun => seen.sun.is_none(),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::All => "every reading",
            Self::Flat => "flat: under the band's own contrast gate",
            Self::Textured => "textured: what the band can correlate on",
            Self::Sun => "the sun in one lens",
            Self::NoSun => "no sun in either lens",
        }
    }
}

/// What one description of the difference between the two lenses leaves.
///
/// A **gain** is what an exposure or a white-balance difference is: the two
/// lenses' pictures of the same content are proportional, per channel. An
/// **offset** is what veiling glare, a black-level pedestal and the toe of a
/// tone curve are: the two differ by a fixed number of codes whatever the
/// content. They are indistinguishable on any one patch and are told apart
/// only by a fit that spans brightnesses, which is why [`Model::all`] prints
/// the span it had.
struct Model {
    name: &'static str,
    gain: f64,
    offset: f64,
}

impl Model {
    /// What this model leaves when it is applied as a **symmetric split**,
    /// which is how the correction is applied: half of it to each lens, so
    /// neither hemisphere carries the whole change.
    fn leaves(&self, points: &[(f64, f64, f64)]) -> (f64, f64) {
        let root = self.gain.max(f64::MIN_POSITIVE).sqrt();
        let mut codes = 0.0;
        let mut weight = 0.0;
        let mut worst: f64 = 0.0;
        for (m0, m1, n) in points {
            let low = root * m0 + self.offset / 2.0;
            let high = (m1 - self.offset / 2.0) / root;
            let step = high - low;
            codes += n * step * step;
            weight += n;
            worst = worst.max(step.abs());
        }
        match weight > 0.0 {
            true => ((codes / weight).sqrt(), worst),
            false => (0.0, 0.0),
        }
    }

    /// The candidate corrections, fitted to the same weighted points.
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
        let mut all = vec![
            Self {
                name: "nothing at all",
                gain: 1.0,
                offset: 0.0,
            },
            Self {
                name: "gain, least squares in codes",
                gain: xy / xx,
                offset: 0.0,
            },
            Self {
                name: "offset alone",
                gain: 1.0,
                offset: (y - x) / n,
            },
        ];
        let spread = xx - x * x / n;
        if spread > 0.0 {
            let gain = (xy - x * y / n) / spread;
            all.push(Self {
                name: "gain and offset together",
                gain,
                offset: (y - gain * x) / n,
            });
        }
        all
    }
}

/// One pooled number, its spread, and what it was pooled over.
///
/// **Pooled per azimuth first**, because the same direction of the seam read on
/// two consecutive frames is very nearly the same measurement of the same
/// content: counting them as independent divides a standard error by the square
/// root of the frame count for free.
#[derive(Clone, Copy, Default)]
struct Reading {
    mean: f64,
    spread: f64,
    count: usize,
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

    /// How far the mean is from zero in units of its own standard error.
    fn signal(&self) -> f64 {
        match self.spread > 0.0 && self.count > 1 {
            true => self.mean.abs() / (self.spread / (self.count as f64).sqrt()),
            false => 0.0,
        }
    }
}

fn pooled<'a>(columns: impl Iterator<Item = &'a Column>) -> Option<Column> {
    let mut held = Column::default();
    for column in columns {
        held.add(column);
    }
    (held.count > 0.0).then_some(held)
}

/// Least squares of one channel's step against the across-seam offset, in
/// codes per degree.
fn slope(columns: &[Column], channel: usize) -> Option<f64> {
    let rows: Vec<(f64, f64)> = columns
        .iter()
        .filter(|c| c.count > 0.0)
        .map(|c| (c.delta, c.mean(1, channel) - c.mean(0, channel)))
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

/// What fraction of one lens's delivered frame is at the ceiling, over a
/// subsampled grid.
///
/// The sun in a lens is not a mood: it is a region of the picture at the top of
/// the sensor's range, and it is the one property of a frame that says which of
/// two hemispheres was pointed at it. Read on a coarse grid because what is
/// wanted is a fraction and not an edge.
fn clipped(plane: &Plane) -> f64 {
    let mut count = 0.0;
    let mut total = 0.0;
    let mut y = 0;
    while y < plane.size.height {
        let mut x = 0;
        while x < plane.size.width {
            if let Some(code) = plane.at(f64::from(x), f64::from(y)) {
                total += 1.0;
                count += f64::from(code >= CEILING);
            }
            x += 16;
        }
        y += 16;
    }
    match total > 0.0 {
        true => count / total,
        false => 0.0,
    }
}

/// Which lens had the sun in it on this frame, if either.
///
/// One lens clipping a measurable share of its picture while the other clips
/// far less. The ratio rather than a threshold on one of them, because a
/// bright scene clips a little in both and that is not the case this names.
fn sun(pair: &Pair) -> Option<usize> {
    let share: Vec<f64> = pair.lenses.iter().map(clipped).collect();
    if share.len() < 2 {
        return None;
    }
    let (high, low) = match share[1] > share[0] {
        true => (1, 0),
        false => (0, 1),
    };
    (share[high] > SUN_SHARE && share[high] > 4.0 * share[low]).then_some(high)
}

/// How much of a lens's picture has to be at the ceiling before the sun counts
/// as being in it.
///
/// The sun subtends half a degree and a lens covers a hemisphere, so the disc
/// itself is under a millionth of the picture; what clips around it is the
/// flare and the sky next to it, and a tenth of a percent is what that reaches
/// on the owner's own captures. Read as a share rather than as a count so it
/// does not depend on the frame size.
const SUN_SHARE: f64 = 0.001;

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
            let sun = sun(&pair);
            for (trial, field) in trials.iter().zip(&mut fields) {
                field.azimuths = ring.len();
                harvest(&reframe, &pair, &ring, &found, sun, *trial, field);
            }
        }
    }
    if fields[0].frames == 0 {
        return Err("no frame decoded at that instant".into());
    }
    Ok(fields)
}

/// One frame's worth of columns, into one trial's field.
///
/// **A direction whose patch did not correlate is kept**, and that is the whole
/// difference from stage 3's harvest. Stage 3 was measuring an alignment-
/// sensitive quantity and refused what it could not line up; this is measuring
/// a difference whose sensitivity to alignment is the content's own gradient,
/// so a flat patch is the easiest reading on the ring rather than the one that
/// must be thrown away. What it takes for such a patch is the calibration's own
/// answer, which is a shift of zero, and the nulls below price it.
fn harvest(
    reframe: &Reframe,
    pair: &Pair,
    ring: &[Where],
    found: &[Option<seam::Found>],
    sun: Option<usize>,
    trial: Trial,
    field: &mut Field,
) {
    field.frames += 1;
    for (index, at) in ring.iter().enumerate() {
        let hit = found[index].filter(|hit| hit.r >= 0.80);
        let shift = hit.map_or((0.0, 0.0), |hit| (hit.along, hit.across));
        let Some(columns) = columns(reframe, &pair.lenses, at, shift, trial) else {
            field.refused += 1;
            continue;
        };
        let Some(held) = pooled(columns.iter().filter(|c| c.delta.abs() <= AT_SEAM_DEG)) else {
            field.refused += 1;
            continue;
        };
        field.seen.push(Seen {
            azimuth: index,
            texture: held.texture(),
            columns,
            across: shift.1.abs(),
            sun,
        });
    }
}

// ------------------------------------------------------------ the field

/// The alignment nudges the sensitivity is read off, in degrees across the
/// seam.
///
/// Sized to the regime rather than borrowed: what the shipped pass leaves on
/// the epipolar axis is 0.02 degrees far field, and what stage 5 leaves along
/// the seam is 0.05 to 0.20. A control has to be able to see the size of thing
/// it is clearing, so these bracket both.
const PROBES: [f64; 4] = [-0.5, -0.2, 0.2, 0.5];

fn field(options: &Options) -> Fallible<()> {
    let mut trials = vec![
        Trial::TRUTH,
        Trial {
            aligned: false,
            ..Trial::TRUTH
        },
        // The null. One lens against its own picture of the same directions,
        // where every channel is zero by arithmetic.
        Trial {
            back: 0,
            aligned: false,
            ..Trial::TRUTH
        },
        // The null that matters: one lens against ITSELF displaced by the very
        // shift the alignment found. No colour difference exists here, so what
        // it reads is what a misregistration of that size is worth per channel.
        Trial {
            back: 0,
            ..Trial::TRUTH
        },
        // The positive controls: a gain in one channel and an offset in
        // another, both of the size being argued about. Each has to come back
        // in its own channel and in no other.
        Trial {
            gain: [1.0, 1.0, 1.02],
            ..Trial::TRUTH
        },
        Trial {
            offset: [4.0, 0.0, 0.0],
            ..Trial::TRUTH
        },
    ];
    for across in PROBES {
        trials.push(Trial {
            nudge: (0.0, across),
            ..Trial::TRUTH
        });
    }
    // The null at the residual the shipped pass actually leaves, which is what
    // licenses reading a flat patch with no alignment at all.
    for across in PROBES {
        trials.push(Trial {
            back: 0,
            aligned: false,
            nudge: (0.0, across),
            ..Trial::TRUTH
        });
    }
    let fields = sweep(options, &trials)?;
    let truth = &fields[0];

    let flat = truth.seen.iter().filter(|s| !s.textured()).count();
    let sunny = truth.seen.iter().filter(|s| s.sun.is_some()).count();
    println!(
        "\nfield:  {} azimuth-frames read of {} tried, over {} frames from {:.3} s, at {} \n\
         \tazimuths round the seam. {} of them are FLAT - under the band's own {CONTRAST:.0} \n\
         \tcode contrast gate, which is content the shipped pass measures nothing on - and \n\
         \t{} were read on a frame with the sun in one lens.",
        truth.seen.len(),
        truth.seen.len() + truth.refused,
        truth.frames,
        options.from,
        options.patches,
        flat,
        sunny,
    );

    steps(truth, options);
    for class in [Class::All, Class::Flat, Class::Textured, Class::Sun] {
        rings(truth, class);
    }
    for class in [Class::All, Class::Flat, Class::Textured, Class::Sun] {
        models(truth, class);
    }
    controls(&fields);
    Ok(())
}

/// What the two lenses differ by at the seam, per channel, per content class.
///
/// The first table to read. Stage 3 corrects one number common to all three
/// channels, so **the spread between the channels is what survives it** and
/// the spread is the defect this stage exists for.
fn steps(field: &Field, options: &Options) {
    println!(
        "\nsteps:  lens 1 minus lens 0 at the seam, in codes of 255, on the same content. \n\
         \tthe last column is the SPREAD between the three channels, which is what a single \n\
         \tbrightness correction leaves behind however well it is fitted: a step common to \n\
         \tR, G and B is a brightness and one that is not is a hue.\n"
    );
    println!(
        "  {:<40} {:>8} {:>8} {:>8} {:>8} {:>8} {:>9}",
        "content", "R", "G", "B", "se R", "se B", "spread"
    );
    for class in [
        Class::All,
        Class::Flat,
        Class::Textured,
        Class::Sun,
        Class::NoSun,
    ] {
        let read: Vec<Reading> = (0..3).map(|c| field.step(class, c)).collect();
        if read[0].count == 0 {
            continue;
        }
        let values: Vec<f64> = read.iter().map(|r| r.mean).collect();
        let spread = values.iter().cloned().fold(f64::MIN, f64::max)
            - values.iter().cloned().fold(f64::MAX, f64::min);
        println!(
            "  {:<40} {:>8.2} {:>8.2} {:>8.2} {:>8.1} {:>8.1} {:>9.2}",
            format!(
                "{} ({} az, {})",
                class.name(),
                read[0].count,
                read[0].readings
            ),
            values[0],
            values[1],
            values[2],
            read[0].signal(),
            read[2].signal(),
            spread,
        );
    }
    println!(
        "\nradial: how each channel's step slopes ACROSS the band, in codes per degree, and \n\
         \twhat that is worth end to end over the {:.1} degree crossover the pass draws. a \n\
         \tstep that is one number everywhere across the overlap is reachable by one \n\
         \tcorrection; one that slopes is not, and needs a wider handover or a field.\n",
        2.0 * f64::from(kjerag_render::CROSSOVER_DEG),
    );
    println!(
        "  {:<40} {:>10} {:>10} {:>10} {:>12}",
        "content", "R /deg", "G /deg", "B /deg", "B end to end"
    );
    for class in [Class::All, Class::Flat, Class::Textured, Class::Sun] {
        let read: Vec<Reading> = (0..3).map(|c| field.radial(class, c)).collect();
        if read[0].count == 0 {
            continue;
        }
        println!(
            "  {:<40} {:>10.3} {:>10.3} {:>10.3} {:>12.2}",
            format!("{} ({})", class.name(), read[0].count),
            read[0].mean,
            read[1].mean,
            read[2].mean,
            read[2].mean * 2.0 * f64::from(kjerag_render::CROSSOVER_DEG),
        );
    }
    if options.verbose {
        table(field);
    } else {
        println!("\n        (verbose=1 prints every azimuth's own reading.)");
    }
}

/// How much of the difference is one number round the whole seam, and how much
/// is a shape.
///
/// **The table the correction's own shape comes out of.** Stage 3 ships one
/// number for the ring and gives the reason: a gain that varied round the seam
/// would be a hemisphere whose brightness changes as the view pans. That
/// argument is about a correction applied to a whole hemisphere and it does not
/// reach a correction supported near the seam. So the question is a
/// measurement: does one number describe what the ring reads, and if not, does
/// the shape `band::Along` already fits for the geometry - a constant, one
/// cycle and two cycles of the azimuth - describe it?
///
/// The last column is what says any of it means anything. The same azimuth read
/// on consecutive frames is the same content twice, so the spread between those
/// readings is this instrument's own noise, and a fit that does not beat it is
/// fitting noise.
fn rings(field: &Field, class: Class) {
    println!(
        "\nrings:  {} - what a correction of each shape LEAVES round the ring, in codes rms \n\
         \tover the azimuths. the basis is the one `band::Along` already fits the geometry \n\
         \tthrough: a constant is a difference between two cameras, one cycle is a principal \n\
         \tpoint, two cycles is a focal aspect. the last column is the same azimuth read on \n\
         \tconsecutive frames, which is the noise any of these has to beat.\n",
        class.name(),
    );
    println!(
        "  {:>3} {:>9} {:>10} {:>11} {:>11} {:>11} {:>12} {:>9} {:>9} {:>9}",
        "ch",
        "azimuths",
        "nothing",
        "a constant",
        "one cycle",
        "two cycles",
        "frame noise",
        "const",
        "1 cyc",
        "2 cyc",
    );
    for (channel, name) in CHANNELS.iter().enumerate() {
        let by_azimuth = field.by_azimuth(class, channel);
        if by_azimuth.len() < 6 {
            continue;
        }
        let leaves = |terms: usize| ring_fit(&by_azimuth, terms).1;
        let (terms, _) = ring_fit(&by_azimuth, 5);
        println!(
            "  {name:>3} {:>9} {:>10.3} {:>11.3} {:>11.3} {:>11.3} {:>12.3} {:>9.2} {:>9.2} {:>9.2}",
            by_azimuth.len(),
            leaves(0),
            leaves(1),
            leaves(3),
            leaves(5),
            field.repeatability(class, channel),
            terms[0],
            f64::from(terms[1]).hypot(f64::from(terms[2])),
            f64::from(terms[3]).hypot(f64::from(terms[4])),
        );
    }
}

/// The five basis functions of the ring fit at one azimuth: the constant, then
/// one cycle, then two. `band::Along`'s own, and deliberately the same.
fn basis(phi: f64) -> [f64; 5] {
    let (sin, cos) = phi.sin_cos();
    [1.0, cos, sin, cos * cos - sin * sin, 2.0 * cos * sin]
}

/// A weighted least-squares fit of `terms` of that basis to the ring, and what
/// it leaves, in codes rms over the azimuths.
///
/// `terms` of 0 is no correction at all, which is what the readings themselves
/// are worth. The solver is [`kjerag_render::band::solve`], the shipped pass's
/// own, so what this scores is a fit the pass can actually make.
fn ring_fit(by_azimuth: &[(f64, f64, f64)], terms: usize) -> ([f32; 5], f64) {
    let mut normal = [[0.0f32; 5]; 5];
    let mut right = [0.0f32; 5];
    for (phi, value, weight) in by_azimuth {
        let held = basis(*phi);
        for row in 0..5 {
            for column in 0..5 {
                let inside = row < terms && column < terms;
                normal[row][column] +=
                    (f64::from(u8::from(inside)) * weight * held[row] * held[column]) as f32;
            }
            right[row] += (f64::from(u8::from(row < terms)) * weight * held[row] * value) as f32;
        }
    }
    // The ridge keeps the untouched rows invertible and shrinks a term nothing
    // supports, which is what `band::Along` uses it for as well.
    for (term, row) in normal.iter_mut().enumerate() {
        row[term] += 1.0;
    }
    let fitted = kjerag_render::band::solve(normal, right);
    let mut error = 0.0;
    let mut weight = 0.0;
    for (phi, value, held) in by_azimuth {
        let at: f64 = basis(*phi)
            .iter()
            .zip(fitted)
            .map(|(term, coefficient)| term * f64::from(coefficient))
            .sum();
        error += held * (value - at).powi(2);
        weight += held;
    }
    (fitted, (error / weight.max(f64::MIN_POSITIVE)).sqrt())
}

/// Which description of the difference the data supports, per channel.
fn models(field: &Field, class: Class) {
    println!(
        "\nmodels: {} - what each candidate correction LEAVES, in codes.",
        class.name(),
    );
    let mut any = false;
    for (channel, name) in CHANNELS.iter().enumerate() {
        let points = field.points(class, channel);
        let all = Model::all(&points);
        if all.is_empty() {
            continue;
        }
        let range = points.iter().fold((f64::MAX, f64::MIN), |held, p| {
            (held.0.min(p.0), held.1.max(p.0))
        });
        if !any {
            println!(
                "\n  {:>3} {:<30} {:>9} {:>9} {:>11} {:>9} {:>14}",
                "ch", "correction", "gain", "offset", "leaves", "worst", "span of codes"
            );
            any = true;
        }
        for model in &all {
            let (codes, worst) = model.leaves(&points);
            println!(
                "  {name:>3} {:<30} {:>9.5} {:>9.3} {:>11.3} {:>9.2} {:>14}",
                model.name,
                model.gain,
                model.offset,
                codes,
                worst,
                format!("{:.0} to {:.0}", range.0, range.1),
            );
        }
    }
    if !any {
        println!("  nothing read in this class.");
    }
}

/// Every control, beside the number each one has to produce.
fn controls(fields: &[Field]) {
    println!(
        "\ncontrols. every trial runs the SAME code on the SAME frames: only the sampling \n\
         directions, one multiplier and one addend change. a per-channel reading is a \n\
         negative result until it is shown able to read a positive one.\n"
    );
    println!(
        "  {:<46} {:>8} {:>8} {:>8} {:>14}",
        "trial", "R", "G", "B", "expected"
    );
    let line = |name: &str, field: &Field, expected: &str| {
        let read: Vec<f64> = (0..3).map(|c| field.step(Class::All, c).mean).collect();
        println!(
            "  {name:<46} {:>8.3} {:>8.3} {:>8.3} {:>14}",
            read[0], read[1], read[2], expected,
        );
    };
    line("the measurement", &fields[0], "-");
    line("the same patches, NOT lined up first", &fields[1], "-");
    line(
        "null: lens 0 on itself, same directions",
        &fields[2],
        "0 0 0",
    );
    line(
        "null: lens 0 on itself, at the found shift",
        &fields[3],
        "0 0 0",
    );
    line("a gain of 1.02 injected into B", &fields[4], "0 0 +B*0.02");
    line(
        "an offset of +4 codes injected into R",
        &fields[5],
        "+4 0 0",
    );
    for (index, nudge) in PROBES.iter().enumerate() {
        line(
            &format!("alignment nudged {nudge:+.1} deg across"),
            &fields[6 + index],
            "the measurement",
        );
    }
    println!(
        "\n  what a MISREGISTRATION of a given size is worth per channel, on this very \n\
         \tcontent: one lens against its own picture, displaced on purpose, where the true \n\
         \tanswer is zero in every channel. this is what says a reading is a colour \n\
         \tdifference and not a window that moved, and it is the whole licence for reading \n\
         \ta patch the band could not correlate on.\n"
    );
    println!(
        "  {:<46} {:>8} {:>8} {:>8} {:>10}",
        "displaced by", "R", "G", "B", "on flat"
    );
    for (index, nudge) in PROBES.iter().enumerate() {
        let held = &fields[10 + index];
        let read: Vec<f64> = (0..3).map(|c| held.step(Class::All, c).mean).collect();
        let flat = held.step(Class::Flat, 2).mean;
        println!(
            "  {:<46} {:>8.3} {:>8.3} {:>8.3} {:>10.3}",
            format!("{nudge:+.2} deg across the seam"),
            read[0],
            read[1],
            read[2],
            flat,
        );
    }
    // The ring fit's own control, and the one the finding stands or falls on.
    // A principal-point error displaces content once round the azimuth, so a
    // misregistration read over a scene with a gradient in it produces a
    // one-cycle photometric shape all by itself - the very term the fit above
    // is about to be believed for. This is that shape, measured: one lens
    // against its own picture at the found shift, where the true field is zero
    // in every channel and at every azimuth.
    for (name, field, class) in [
        (
            "lens 0 on itself, displaced +0.2 deg across",
            &fields[12],
            Class::All,
        ),
        (
            "the same, on the flat content only",
            &fields[12],
            Class::Flat,
        ),
        (
            "lens 0 on itself, displaced +0.5 deg across",
            &fields[13],
            Class::All,
        ),
    ] {
        println!("\n  the ring fit's own null - {name}:");
        println!(
            "  {:>3} {:>9} {:>10} {:>11} {:>11} {:>11} {:>9} {:>9}",
            "ch", "azimuths", "nothing", "a constant", "one cycle", "two cycles", "1 cyc", "2 cyc",
        );
        for (channel, label) in CHANNELS.iter().enumerate() {
            let by_azimuth = field.by_azimuth(class, channel);
            if by_azimuth.len() < 6 {
                continue;
            }
            let (terms, _) = ring_fit(&by_azimuth, 5);
            println!(
                "  {label:>3} {:>9} {:>10.3} {:>11.3} {:>11.3} {:>11.3} {:>9.2} {:>9.2}",
                by_azimuth.len(),
                ring_fit(&by_azimuth, 0).1,
                ring_fit(&by_azimuth, 1).1,
                ring_fit(&by_azimuth, 3).1,
                ring_fit(&by_azimuth, 5).1,
                f64::from(terms[1]).hypot(f64::from(terms[2])),
                f64::from(terms[3]).hypot(f64::from(terms[4])),
            );
        }
    }
    let truth = fields[0].step(Class::All, 2).mean;
    let confound = PROBES
        .iter()
        .enumerate()
        .map(|(index, nudge)| (fields[10 + index].step(Class::All, 2).mean / nudge).abs())
        .fold(0.0f64, f64::max);
    println!(
        "\n  the worst of those, per degree, is {confound:.3} codes in B. the shipped pass \n\
         \tleaves 0.02 deg far field on the epipolar axis and 0.05 to 0.20 along the seam \n\
         \t(stage 5), so at 0.20 deg the confound is {:.3} codes against a reading of \n\
         \t{truth:.3}: {:.1}x.",
        confound * 0.2,
        truth.abs() / (confound * 0.2).max(f64::MIN_POSITIVE),
    );
}

/// What each azimuth read, so a pooled number can be checked against the things
/// it was pooled from.
fn table(field: &Field) {
    println!("\n    phi   texture      lit0       dR       dG       dB    across   sun  samples");
    for seen in &field.seen {
        let Some(held) = seen.at_seam() else {
            continue;
        };
        println!(
            "{:>7} {:>9.2} {:>9.2} {:>8.2} {:>8.2} {:>8.2} {:>9.3} {:>5} {:>8.0}",
            seen.azimuth,
            seen.texture,
            held.mean(0, 0).max(held.mean(0, 1)),
            held.mean(1, 0) - held.mean(0, 0),
            held.mean(1, 1) - held.mean(0, 1),
            held.mean(1, 2) - held.mean(0, 2),
            seen.across,
            seen.sun
                .map_or_else(|| "-".to_owned(), |lens| lens.to_string()),
            held.count,
        );
    }
}

// ------------------------------------------------------------ the picture

/// One drawn view, and what each channel does as it crosses the seam.
///
/// **The acceptance statistic**, and it is deliberately the one an eye uses:
/// what is visible at a seam is a step in a picture, so the picture is what is
/// measured. Each side's own trend is fitted over the degrees where one lens is
/// drawing alone and the two are extrapolated to the seam, so a scene's own
/// gradient reports zero and a handover that changes colour does not. The
/// **decoy** is the same statistic about a great circle 90 degrees away where
/// there is no handover at all, which is what the scene contributes to a number
/// like this.
fn profile(options: &Options) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let out = options.out();
    std::fs::create_dir_all(&out)?;
    let size = Size::new(options.size, options.size);

    let draw = |held: bool| -> Fallible<(
        Picture,
        Reframe,
        kjerag_render::Tone,
        kjerag_render::Tint,
        f32,
    )> {
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
        let (_, tint, cells) = pipeline.band_state(&gpu.device, &gpu.queue)?;
        // How much of the ring answered about COLOUR, which is not how much
        // answered about geometry: the two are separate channels since stage 7
        // and most of a sky seam is only ever the first.
        let colours =
            cells.iter().filter(|cell| cell.hue_conf > 0.0).count() as f32 / cells.len() as f32;
        Ok((
            shot.ok_or("no frame decoded at that instant")?,
            mapped,
            tone,
            tint,
            colours,
        ))
    };
    let (before, mapped, _, _, _) = draw(true)?;
    let (after, _, tone, tint, colours) = draw(false)?;

    let stem = format!("{}-{}", options.stem(), options.tag);
    before.save(&gpu, &out.join(format!("{stem}-1-held.png")))?;
    after.save(&gpu, &out.join(format!("{stem}-2-drawn.png")))?;
    after
        .amplified(&before)
        .save(&gpu, &out.join(format!("{stem}-3-what-moved.png")))?;
    let split = tone.split();
    println!(
        "\nwrote three pictures into {} at yaw {:.2}, pitch {:.2}, fov {:.2}, {} frames in.\n\
         {}\ngain:   R {:+.5} G {:+.5} B {:+.5} ln, evidence {:.3}, {:.0} percent of the ring \n\
         \thad a colour to read. lens 0 is multiplied by {:.5} {:.5} {:.5} and lens 1 by \n\
         \t{:.5} {:.5} {:.5}.",
        out.display(),
        options.yaw,
        options.pitch,
        options.fov,
        options.count.max(1),
        after.against(&before).report(),
        tone.log_gain[0],
        tone.log_gain[1],
        tone.log_gain[2],
        tone.evidence,
        100.0 * colours,
        split[0][0],
        split[0][1],
        split[0][2],
        split[1][0],
        split[1][1],
        split[1][2],
    );
    println!(
        "field:  what the ring says ROUND the seam, on top of that, as the amplitude of each \n\
         \tcycle in codes at a mid grey of 128, evidence {:.1} directions:\n\
         \t  R one cycle {:.2}, two cycles {:.2}\n\
         \t  G one cycle {:.2}, two cycles {:.2}\n\
         \t  B one cycle {:.2}, two cycles {:.2}",
        tint.evidence,
        128.0 * f64::from(tint.terms[0]).hypot(f64::from(tint.terms[1])),
        128.0 * f64::from(tint.terms[2]).hypot(f64::from(tint.terms[3])),
        128.0 * f64::from(tint.terms[4]).hypot(f64::from(tint.terms[5])),
        128.0 * f64::from(tint.terms[6]).hypot(f64::from(tint.terms[7])),
        128.0 * f64::from(tint.terms[8]).hypot(f64::from(tint.terms[9])),
        128.0 * f64::from(tint.terms[10]).hypot(f64::from(tint.terms[11])),
    );
    for (name, picture) in [("the band held off", &before), ("as it draws", &after)] {
        println!("\n=== {name} ===");
        across_seam(&mapped, picture, size);
    }
    Ok(())
}

/// Each channel's step across the seam, and the same at the decoy.
fn across_seam(reframe: &Reframe, picture: &Picture, size: Size) {
    let planes = channels(picture);
    let seam = distances(reframe, size, 2);
    let decoy = distances(reframe, size, 0);
    println!(
        "\n  {:>10} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>8}",
        "degrees", "R", "G", "B", "decoy R", "decoy G", "decoy B", "pixels"
    );
    for step in -8..=8 {
        let band = (f64::from(step) - 0.5, f64::from(step) + 0.5);
        let here: Vec<(Option<f64>, usize)> =
            planes.iter().map(|p| strip(p, &seam, band)).collect();
        let there: Vec<(Option<f64>, usize)> =
            planes.iter().map(|p| strip(p, &decoy, band)).collect();
        let show = |held: &(Option<f64>, usize)| {
            held.0.map_or_else(|| "-".to_owned(), |v| format!("{v:.2}"))
        };
        println!(
            "  {step:>10} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>8}",
            show(&here[0]),
            show(&here[1]),
            show(&here[2]),
            show(&there[0]),
            show(&there[1]),
            show(&there[2]),
            here[0].1,
        );
    }
    let step = |plane: &[f64], at: &[Option<f64>]| {
        let low = trend(plane, at, (-8.0, -1.5))?;
        let high = trend(plane, at, (1.5, 8.0))?;
        Some(high - low)
    };
    println!("\n  step across the seam, each side's trend extrapolated to it, in codes of 255:");
    let mut worst: f64 = 0.0;
    let mut values = [0.0f64; 3];
    for (channel, name) in CHANNELS.iter().enumerate() {
        let here = step(&planes[channel], &seam);
        let there = step(&planes[channel], &decoy);
        if let Some(value) = here {
            worst = worst.max(value.abs());
            values[channel] = value;
        }
        println!(
            "    {name}: {:>10}   decoy {:>10}",
            here.map_or_else(|| "-".to_owned(), |v| format!("{v:+.3}")),
            there.map_or_else(|| "-".to_owned(), |v| format!("{v:+.3}")),
        );
    }
    let hue = values.iter().cloned().fold(f64::MIN, f64::max)
        - values.iter().cloned().fold(f64::MAX, f64::min);
    println!(
        "    worst channel {worst:.3} codes; the spread between them, which is the HUE step, \n\
         \x20   is {hue:.3} codes. one code of 255 is the floor of the medium: under it there \n\
         \x20   is no step left in the picture to see."
    );
}

/// The three channels of a drawn picture, as planes of codes.
fn channels(picture: &Picture) -> [Vec<f64>; 3] {
    std::array::from_fn(|channel| {
        picture
            .rgba
            .chunks_exact(4)
            .map(|p| f64::from(p[channel]))
            .collect()
    })
}

/// How far past a great circle each output pixel is, in degrees, or `None`
/// where no lens has the ray. `axis` 2 is the seam; 0 is the decoy.
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

/// One side's mean trend, extrapolated to the circle, in codes.
fn trend(plane: &[f64], distance: &[Option<f64>], band: (f64, f64)) -> Option<f64> {
    let mut rows: Vec<(f64, f64)> = Vec::new();
    let mut at = band.0;
    while at <= band.1 {
        if let (Some(mean), _) = strip(plane, distance, (at - 0.25, at + 0.25)) {
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

/// Mean of the pixels whose distance falls inside `band`, and how many.
fn strip(plane: &[f64], distance: &[Option<f64>], band: (f64, f64)) -> (Option<f64>, usize) {
    let mut total = 0.0;
    let mut count = 0usize;
    for (index, value) in plane.iter().enumerate() {
        let Some(at) = distance[index] else {
            continue;
        };
        if !(band.0..=band.1).contains(&at) || *value <= 0.0 {
            continue;
        }
        total += value;
        count += 1;
    }
    match count > 0 {
        true => (Some(total / count as f64), count),
        false => (None, count),
    }
}

// ------------------------------------------------------------ the competition

/// How wide the colour transition is across somebody else's seam, in an
/// equirectangular export.
///
/// **The one measurement that decides how much of this stage is correction and
/// how much is blending.** A stitcher that CORRECTS the two lenses to each
/// other leaves a flat profile with a narrow join in it; one that BLENDS the
/// difference away leaves a smooth ramp as wide as its blend, and the width of
/// that ramp is the width it decided the eye needs. Reading it costs nothing
/// but a frame of their output.
///
/// The frame arrives through ffmpeg as raw rgb24 on a pipe, because that is one
/// dependency the harness already has and no decoder in this repo opens a
/// finished mp4.
fn studio(options: &Options) -> Fallible<()> {
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-ss"])
        .arg(format!("{}", options.from))
        .arg("-i")
        .arg(&options.input)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdout(Stdio::piped())
        .spawn()?;
    let mut bytes = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        std::io::Read::read_to_end(&mut out, &mut bytes)?;
    }
    child.wait()?;
    let (width, height) = size_of(&options.input)?;
    if bytes.len() < width * height * 3 {
        return Err(format!(
            "ffmpeg gave {} bytes for a {width}x{height} rgb24 frame",
            bytes.len()
        )
        .into());
    }
    let (low, high) = options.rows(height);
    println!(
        "studio: {width}x{height} at {:.3} s, column means over rows {low} to {high}. \n\
         \tone column is {:.4} degrees at the {:.0} degree field this export was written at.",
        options.from,
        options.fov / width as f64,
        options.fov,
    );
    let mut column = vec![[0.0f64; 3]; width];
    for row in low..high {
        for (x, held) in column.iter_mut().enumerate() {
            let at = (row * width + x) * 3;
            for (channel, sum) in held.iter_mut().enumerate() {
                *sum += f64::from(bytes[at + channel]);
            }
        }
    }
    let rows = (high - low) as f64;
    for held in &mut column {
        for sum in held.iter_mut() {
            *sum /= rows;
        }
    }
    seam_transition(&column, options);
    Ok(())
}

/// The frame size ffprobe reports for a file.
fn size_of(path: &std::path::Path) -> Fallible<(usize, usize)> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0:s=x",
        ])
        .arg(path)
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (width, height) = text
        .trim()
        .split_once('x')
        .ok_or_else(|| format!("ffprobe said {text:?}"))?;
    Ok((width.trim().parse()?, height.trim().parse()?))
}

/// Where the seams are in a column profile, and how wide each transition is.
///
/// An equirectangular stitch hands over at two longitudes 180 degrees apart.
/// What marks them is not a step - the whole point of a good stitch is that
/// there is not one - but a **change of slope**: the second derivative of the
/// column profile has its two largest features there, because a blend's ramp
/// starts and stops. So the profile's own curvature is scanned rather than a
/// longitude being assumed, and the width is read off how far the ramp runs.
fn seam_transition(column: &[[f64; 3]], options: &Options) {
    let width = column.len();
    let degrees = options.fov / width as f64;
    let (from, to) = options.columns(width);
    println!(
        "\n  the profile, column by column. a stitcher that CORRECTS its two lenses to each \n\
         \tother leaves this flat with a narrow join in it; one that BLENDS the difference \n\
         \taway leaves a ramp as wide as its blend, and the width of that ramp is the width \n\
         \tit decided an eye needs.\n"
    );
    println!(
        "    {:>8} {:>9} {:>9} {:>9} {:>9} {:>11}",
        "column", "degrees", "R", "G", "B", "dR-dB /col"
    );
    let step = ((to - from) / 64).max(1);
    let mut at = from;
    while at < to {
        let ahead = (at + step).min(width - 1);
        println!(
            "    {at:>8} {:>9.2} {:>9.2} {:>9.2} {:>9.2} {:>11.3}",
            (at as f64 - width as f64 / 2.0) * degrees,
            column[at][0],
            column[at][1],
            column[at][2],
            ((column[ahead][0] - column[ahead][2]) - (column[at][0] - column[at][2])) / step as f64,
        );
        at += step;
    }
}

// ------------------------------------------------------------ options

struct Options {
    input: PathBuf,
    mode: Mode,
    from: f64,
    count: usize,
    places: usize,
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
    /// How far either side of a competitor's seam the transition is printed,
    /// in degrees.
    reach: f64,
    /// Which rows and columns of a competitor's export the profile is read
    /// over, as fractions of the frame.
    band: (f64, f64),
    span: (f64, f64),
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            mode: Mode::Field,
            from: 0.0,
            count: 8,
            places: 1,
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
            reach: 8.0,
            band: (0.375, 0.625),
            span: (0.0, 1.0),
        };
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("mode", value)) => {
                    options.mode = match value {
                        "field" => Mode::Field,
                        "profile" => Mode::Profile,
                        "studio" => Mode::Studio,
                        _ => return Err(format!("no mode called {value}").into()),
                    }
                }
                Some(("from" | "at", value)) => options.from = value.parse()?,
                Some(("count", value)) => options.count = value.parse()?,
                Some(("places", value)) => options.places = value.parse()?,
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
                Some(("reach", value)) => options.reach = value.parse()?,
                Some(("rows", value)) => options.band = pair(value)?,
                Some(("cols", value)) => options.span = pair(value)?,
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

    fn rows(&self, height: usize) -> (usize, usize) {
        let scale = |at: f64| ((at * height as f64) as usize).min(height.saturating_sub(1));
        (
            scale(self.band.0),
            scale(self.band.1).max(scale(self.band.0) + 1),
        )
    }

    fn columns(&self, width: usize) -> (usize, usize) {
        let scale = |at: f64| ((at * width as f64) as usize).min(width.saturating_sub(1));
        (
            scale(self.span.0),
            scale(self.span.1).max(scale(self.span.0) + 1),
        )
    }

    fn stem(&self) -> String {
        self.input
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    }
}

/// Two fractions of a frame, as `low:high`.
fn pair(value: &str) -> Fallible<(f64, f64)> {
    let (low, high) = value
        .split_once(':')
        .ok_or_else(|| format!("{value:?} is not low:high"))?;
    Ok((low.parse()?, high.parse()?))
}

const USAGE: &str = "usage: colour <file.insv|export.mp4> [mode=field|profile|studio] \
     [from=seconds] [count=frames] [places=n] [patches=n] [keep=r] [seam=factory] [verbose=1] \
     [yaw=deg] [pitch=deg] [fov=deg] [size=px] [lock=0] [out=dir] [tag=name] [reach=deg] \
     [rows=lo:hi] [cols=lo:hi]";
