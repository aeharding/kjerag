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
        Mode::Trace => trace(&options),
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
    /// What the shipped pass's own colour state does frame to frame.
    Trace,
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
/// of. 5 leaves room for the alignment shift on top of it.
///
/// What that covers has changed under it. It used to reach past everything the
/// handover could touch; since 2026-08-05 the crossover is 8 degrees wide, so
/// these columns cover the whole doubled band (4 either side, with a degree to
/// spare) but **not** the 6.6 degrees the band plus the bend it carries reaches
/// to ([`kjerag_render::band::reach`]). The outermost columns are inside the
/// handover now, and the optics are what stops this being widened to match.
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
    let files = [options.input.clone()];
    let Some(fitted) = seam::fit_reported(&files, &lenses, frame, &seam::Plan::default()) else {
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
         \twhat that is worth end to end over the {CROSSOVER_DEG:.1} degree crossover the pass \n\
         \tasks for. a step that is one number everywhere across the overlap is reachable by \n\
         \tone correction; one that slopes is not, and needs a wider handover or a field.\n"
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
            read[2].mean * CROSSOVER_DEG,
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
            terms[1].hypot(terms[2]),
            terms[3].hypot(terms[4]),
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
fn ring_fit(by_azimuth: &[(f64, f64, f64)], terms: usize) -> ([f64; 5], f64) {
    let mut normal = [[0.0f64; 5]; 5];
    let mut right = [0.0f64; 5];
    for (phi, value, weight) in by_azimuth {
        let held = basis(*phi);
        for row in 0..5 {
            for column in 0..5 {
                let inside = row < terms && column < terms;
                normal[row][column] +=
                    f64::from(u8::from(inside)) * weight * held[row] * held[column];
            }
            right[row] += f64::from(u8::from(row < terms)) * weight * held[row] * value;
        }
    }
    // The ridge keeps the untouched rows invertible and shrinks a term nothing
    // supports, which is what `band::Along` uses it for as well.
    for (term, row) in normal.iter_mut().enumerate() {
        row[term] += 1.0;
    }
    let fitted = solve5(normal, right);
    let mut error = 0.0;
    let mut weight = 0.0;
    for (phi, value, held) in by_azimuth {
        let at: f64 = basis(*phi)
            .iter()
            .zip(fitted)
            .map(|(term, coefficient)| term * coefficient)
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
                terms[1].hypot(terms[2]),
                terms[3].hypot(terms[4]),
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

    let draw = |held: bool| -> Fallible<(Picture, Reframe, kjerag_render::Tone)> {
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
        let (_, cells) = pipeline.band_state(&gpu.device, &gpu.queue)?;
        // What the shipped state holds, which since this PR re-scoped is main's:
        // one gain over the whole ring, and how much of the ring is behind it.
        // The instrument reports it so a picture can be read beside the number
        // that drew it; it no longer reports a per-direction anything, because
        // the pass no longer has one.
        let seen =
            cells.iter().filter(|cell| cell.confidence > 0.0).count() as f32 / cells.len() as f32;
        println!(
            "band:   the shipped gain is {:+.5} ln, evidence {:.3}, {:.0} percent of the ring \n\
             \tis correlating.",
            tone.log_gain,
            tone.evidence,
            100.0 * seen,
        );
        Ok((
            shot.ok_or("no frame decoded at that instant")?,
            mapped,
            tone,
        ))
    };
    let (before, mapped, _) = draw(true)?;
    let (after, _, tone) = draw(false)?;

    let stem = format!("{}-{}", options.stem(), options.tag);
    before.save(&gpu, &out.join(format!("{stem}-1-held.png")))?;
    after.save(&gpu, &out.join(format!("{stem}-2-drawn.png")))?;
    after
        .amplified(&before)
        .save(&gpu, &out.join(format!("{stem}-3-what-moved.png")))?;
    marked(&after, &mapped, size).save(&gpu, &out.join(format!("{stem}-4-marked.png")))?;
    let split = tone.split();
    println!(
        "\nwrote four pictures into {} at yaw {:.2}, pitch {:.2}, fov {:.2}, {} frames in.\n\
         {}\ngain:   {:+.5} ln; lens 0 is multiplied by {:.5} and lens 1 by {:.5}.",
        out.display(),
        options.yaw,
        options.pitch,
        options.fov,
        options.count.max(1),
        after.against(&before).report(),
        tone.log_gain,
        split[0],
        split[1],
    );
    // THE FIELD'S OWN INTERIOR, which is what the owner rejected the branch
    // over and what nothing here could see (issue #103, stage 8).
    println!(
        "\n=== the applied field, {} to {} degrees off the seam ===",
        INTERIOR.0, INTERIOR.1,
    );
    match interior(&before, &after, &mapped, size, 0.0) {
        Some(read) => println!("  what is drawn        {}", read.report()),
        None => println!("  not enough of the interior is in this view"),
    }
    println!("  controls:");
    if let Some(read) = interior(&before, &before, &mapped, size, 0.0) {
        println!(
            "    nothing applied at all                {}",
            read.report()
        );
    }
    for planted in [0.5f64, 2.0] {
        if let Some(read) = interior(&before, &before, &mapped, size, planted) {
            println!(
                "    a {planted:.1} code ripple, 8 cycles round the ring   {}",
                read.report(),
            );
        }
    }
    for (name, picture) in [("the band held off", &before), ("as it draws", &after)] {
        println!("\n=== {name} ===");
        across_seam(&mapped, picture, size, options.window);
        eye(&mapped, picture, size, options.window, options.reach);
    }
    Ok(())
}

/// The drawn picture with the handover drawn on it, so a defect can be
/// pointed at rather than described.
///
/// Three marks, and each is a claim the eye can check against the picture
/// under it: the seam plane itself, where the two lenses meet; the crossover,
/// which is what the pass mixes them over and therefore how sharp any residual
/// difference is allowed to be; and the edge of the overlap, past which only
/// one lens has a picture at all. A step that sits inside the crossover is the
/// handover's; one that does not is the scene's.
///
/// Both edges are asked of the map this render was drawn with rather than
/// written down: since 2026-08-05 the crossover is the camera's own width and
/// the overlap always was.
fn marked(picture: &Picture, reframe: &Reframe, size: Size) -> Picture {
    let seam = distances(reframe, size, 2, (0.0, 0.0, 1.0, 1.0));
    let crossover = f64::from(reframe.crossover_at(0.0).to_degrees()) / 2.0;
    let overlap = reframe.overlap().map_or(0.0, |o| f64::from(o.to_degrees())) / 2.0;
    let rgba = picture
        .rgba
        .chunks_exact(4)
        .zip(&seam)
        .flat_map(|(pixel, at)| {
            let Some(degrees) = at else {
                return [pixel[0], pixel[1], pixel[2], 255];
            };
            let away = degrees.abs();
            // Lines and not bands: what is under them is the evidence, so the
            // marks have to be narrow enough to leave it visible. Each is one
            // twentieth of a degree wide, which at any view this player offers
            // is a few pixels.
            let at = |edge: f64| (away - edge).abs() < 0.025;
            if away < 0.025 {
                return [255, 40, 40, 255];
            }
            if at(crossover) {
                return [60, 220, 255, 255];
            }
            if at(overlap) {
                return [255, 230, 60, 255];
            }
            [pixel[0], pixel[1], pixel[2], 255]
        })
        .collect();
    Picture {
        rgba,
        size: picture.size,
    }
}

// ------------------------------------------------------------ the eye

/// The lags the local contrast is read over, in pixels of the delivered view.
///
/// A step and a ramp are the same number of codes and not the same artifact,
/// and the only thing that separates them is the distance the codes are spread
/// over. One pixel is the sharpest thing a display can show; 32 is a quarter of
/// a degree at the view the owner complained at, which is around where a
/// gradient stops being an edge and starts being shading.
const LAGS: [usize; 8] = [1, 2, 4, 8, 16, 32, 64, 128];

/// The contrast an eye is held to. Weber, so it is a ratio and not a count of
/// codes: 1 percent is the standard just-noticeable difference on a large flat
/// field, and it is the bar stage 8 is scored against.
const JND: f64 = 0.01;

/// What a seam is worth to an eye, at one view: the steepest local Weber
/// contrast anywhere across the handover (issue #103, stage 8).
///
/// **Why this replaces a step in codes.** A step of 6.5 codes is 31 percent of
/// 21-code soil and 3.4 percent of 190-code sky, and an eye reads the second
/// one as a tenth of the first. Every acceptance number before stage 8 was
/// counted in codes, which is a loss that spends its whole budget on bright
/// content, and it is why the owner's wide view could be scored as improved
/// while he was looking at the artifact (docs/research/seam-blending.md 4).
///
/// **Why LOCAL, and why in pixels.** A correction that spreads a difference
/// over enough of the picture is a difference an eye cannot find, because the
/// contrast sensitivity of the eye falls away at low spatial frequency: what a
/// seam shows is not the total change but the steepest part of it. So the
/// profile is binned at one pixel of the DELIVERED view, and the statistic is
/// the largest change between two bins a given number of pixels apart. The same
/// residual reads five times sharper at the owner's fov 114 than at the fov 20
/// stage 5 was judged on, and nothing about the correction changed between them
/// (6.5).
#[derive(Clone, Copy, Debug, Default)]
struct Eye {
    /// The steepest local Weber contrast at each of [`LAGS`], worst channel.
    steepest: [f64; LAGS.len()],
    /// The whole step across the handover as a ratio: each side's own trend
    /// extrapolated to the seam, over the mean of the two. The statistic
    /// stage 3 and stage 7 reported in codes, in the space an eye reads.
    step: f64,
    /// How many degrees of view one pixel is, at the seam, which is what turns
    /// the two into each other.
    degrees_per_pixel: f64,
    /// How many one-pixel bins the profile was read over.
    bins: usize,
}

impl Eye {
    /// The one number the bar is set on: the steepest contrast an eye can find
    /// at any of the lags, worst channel.
    fn worst(&self) -> f64 {
        self.steepest.iter().copied().fold(0.0, f64::max)
    }

    fn report(&self) -> String {
        let lags = LAGS
            .iter()
            .zip(&self.steepest)
            .map(|(lag, held)| format!("{lag}px {:.2}%", 100.0 * held))
            .collect::<Vec<_>>()
            .join("  ");
        format!(
            "step {:+.2}%  steepest {lags}  ({:.4} deg/px over {} bins)",
            100.0 * self.step,
            self.degrees_per_pixel,
            self.bins,
        )
    }
}

/// How many degrees of the seam's own axis one output pixel covers, near the
/// seam: the median of what neighbouring pixels' distances differ by.
///
/// Measured off the picture rather than derived from the field of view, because
/// the output projection bends past [`FOV_FLAT`](kjerag_render) and the rate
/// at the seam is not the rate at the middle of the frame.
fn degrees_per_pixel(distance: &[Option<f64>], size: Size) -> Option<f64> {
    let width = size.width as usize;
    let mut steps: Vec<f64> = Vec::new();
    for index in 0..distance.len() {
        if index % width + 1 >= width {
            continue;
        }
        let (Some(here), Some(next)) = (distance[index], distance[index + 1]) else {
            continue;
        };
        if here.abs() > 2.0 {
            continue;
        }
        let step = (next - here).abs();
        if step > 0.0 {
            steps.push(step);
        }
    }
    if steps.is_empty() {
        return None;
    }
    steps.sort_by(f64::total_cmp);
    Some(steps[steps.len() / 2])
}

/// The picture's profile across the seam, one bin per pixel of the delivered
/// view: how far from the seam the bin is in degrees, its mean code per
/// channel, and how many pixels answered.
fn binned(
    planes: &[Vec<f64>; 3],
    distance: &[Option<f64>],
    rate: f64,
    reach: f64,
) -> Vec<(f64, [f64; 3], f64)> {
    let bins = (reach / rate).round().max(1.0) as usize;
    // Sums first and means after: one pass over the picture rather than one
    // pass per bin, because a wide view is a megapixel and the bins are
    // thousands of it.
    let mut held = vec![([0.0f64; 3], 0.0f64); 2 * bins + 1];
    for (index, at) in distance.iter().enumerate() {
        let Some(at) = at else { continue };
        let bin = (at / rate).round();
        if bin.abs() > bins as f64 {
            continue;
        }
        if (0..3).any(|channel| planes[channel][index] <= 0.0) {
            continue;
        }
        let slot = &mut held[(bin as isize + bins as isize) as usize];
        for (channel, plane) in planes.iter().enumerate() {
            slot.0[channel] += plane[index];
        }
        slot.1 += 1.0;
    }
    held.into_iter()
        .enumerate()
        .filter(|(_, (_, count))| *count > 0.0)
        .map(|(slot, (sums, count))| {
            (
                (slot as f64 - bins as f64) * rate,
                std::array::from_fn(|channel| sums[channel] / count),
                count,
            )
        })
        .collect()
}

/// The metric itself, over one picture and one great circle.
///
/// `reach` is how far either side of the circle the profile is read, in
/// degrees: the whole overlap and a little more, because past there the two
/// lenses have no common picture and there is nothing a handover could still
/// be doing.
///
/// **Every pair the maximum is taken over STRADDLES the circle**, which is
/// what makes this a statistic about a handover rather than about a scene. A
/// pair of bins both on one side is the scene's own texture, and on a wide
/// view of ploughed soil that is larger than the artifact; the same pair
/// measured across the seam is the handover plus that texture, and the decoy
/// circle is what says how much of it is which.
fn eye_at(
    planes: &[Vec<f64>; 3],
    distance: &[Option<f64>],
    size: Size,
    reach: f64,
    centre: f64,
) -> Option<Eye> {
    let rate = degrees_per_pixel(distance, size)?;
    let bins = binned(planes, distance, rate, reach);
    if bins.len() < 8 {
        return None;
    }
    let mut out = Eye {
        degrees_per_pixel: rate,
        bins: bins.len(),
        ..Eye::default()
    };
    for (slot, lag) in LAGS.iter().enumerate() {
        let mut worst = 0.0f64;
        for low in 0..bins.len().saturating_sub(*lag) {
            let high = low + lag;
            // Straddling, and by the degrees rather than by the index: a bin
            // with no pixels in it is not in the list at all.
            // Straddling THE LINE BEING ASKED ABOUT, by the degrees rather
            // than by the index: a bin with no pixels in it is not in the list
            // at all. `centre` is zero for the seam and a few degrees either
            // way for the controls that say what this content reads anywhere
            // (issue #103, stage 8, the line decomposition).
            if bins[low].0 > centre || bins[high].0 < centre {
                continue;
            }
            for channel in 0..3 {
                let (a, b) = (bins[low].1[channel], bins[high].1[channel]);
                let mean = 0.5 * (a + b);
                if mean <= 0.0 {
                    continue;
                }
                worst = worst.max((b - a).abs() / mean);
            }
        }
        out.steepest[slot] = worst;
    }
    let at = |from: f64, to: f64| -> Option<(f64, f64)> {
        let rows: Vec<(f64, f64)> = bins
            .iter()
            .filter(|(degrees, _, _)| (from..=to).contains(degrees))
            .map(|(degrees, codes, _)| {
                (
                    *degrees,
                    LUMA[0] * codes[0] + LUMA[1] * codes[1] + LUMA[2] * codes[2],
                )
            })
            .collect();
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
        (variance > 0.0).then(|| (mean_y - covariance / variance * mean_x, mean_y))
    };
    if let (Some((low, level_low)), Some((high, level_high))) = (at(-reach, -1.5), at(1.5, reach)) {
        let mean = 0.5 * (level_low + level_high);
        if mean > 0.0 {
            out.step = (high - low) / mean;
        }
    }
    Some(out)
}

/// BT.709's luma weights, which is what the three channels are pooled into
/// where the statistic wants a brightness rather than a colour.
const LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722];

/// A flat field of one level, with a change of a known ratio put across the
/// circle over a known number of pixels: the positive control for [`eye_at`].
///
/// A control and not a second code path. The metric is run over the same
/// geometry the picture was measured through - the pass's own distances, at
/// the pass's own view - and the only thing that changes is what is written
/// into the pixels. Three things have to come back, and each of them is a
/// property the whole stage rests on:
///
/// - a ratio of 1 reads **zero** at every lag;
/// - a ratio of `r` spread over ONE pixel reads `(r - 1) / ((r + 1) / 2)` at
///   every lag, because a step is the same size however far apart the two bins
///   reading it are;
/// - the same ratio spread over `n` pixels reads the same whole step and a
///   `lag / n` share of it at each lag, which is the entire claim stage 8
///   makes about spreading a difference out.
fn flat(distance: &[Option<f64>], rate: f64, level: f64, ratio: f64, pixels: f64) -> [Vec<f64>; 3] {
    let ramp = |at: f64| -> f64 {
        let t = ((at / rate / pixels) + 0.5).clamp(0.0, 1.0);
        level * (1.0 + (ratio - 1.0) * t)
    };
    std::array::from_fn(|_| distance.iter().map(|at| at.map_or(0.0, ramp)).collect())
}

/// What one view is worth to an eye, with its nulls and its plants under it.
fn eye(reframe: &Reframe, picture: &Picture, size: Size, window: (f64, f64, f64, f64), reach: f64) {
    let planes = channels(picture);
    let seam = distances(reframe, size, 2, window);
    // The whole frame for the decoy and the window for the seam: the window
    // is there to hold one kind of content across the HANDOVER, and the decoy
    // circle is a quarter turn away from it.
    let decoy = distances(reframe, size, 0, (0.0, 0.0, 1.0, 1.0));
    let Some(here) = eye_at(&planes, &seam, size, reach, 0.0) else {
        println!("\n  the eye: not enough of the seam is inside the window to profile");
        return;
    };
    println!("\n  the eye, in Weber contrast across the seam, worst channel:");
    println!("    the seam            {}", here.report());
    println!(
        "    THE BAR             the worst lag is {:.2}%, against a {:.0}% just-noticeable \
         difference: {}",
        100.0 * here.worst(),
        100.0 * JND,
        match here.worst() <= JND {
            true => "AT OR UNDER",
            false => "OVER",
        },
    );
    // THE LINE'S AUTHOR (issue #103, stage 8). The owner's report after the
    // wide matching landed was that it "still effectively looks like a line",
    // and a line at one pixel has two possible authors: a photometric STEP,
    // which is a difference in level and shows on content with no gradient at
    // all, or a MISREGISTRATION, which is a difference in position and shows
    // only where there is content to draw twice. The same statistic straddling
    // a line a few degrees away, in the same window and the same content, is
    // what separates them: a photometric step is at the seam and nowhere else,
    // and texture is everywhere.
    println!(
        "\n  the line's author: the same statistic straddling a line a few degrees\n\
         \x20 off the seam, in the same window and the same content.\n\n\
         \x20   where{:>10}{:>10}{:>10}{:>10}",
        "1px", "2px", "8px", "32px",
    );
    let show = |name: &str, read: &Eye| {
        println!(
            "    {name:<14}{:>9.2}%{:>9.2}%{:>9.2}%{:>9.2}%",
            100.0 * read.steepest[0],
            100.0 * read.steepest[1],
            100.0 * read.steepest[3],
            100.0 * read.steepest[5],
        );
    };
    let mut away: Vec<Eye> = Vec::new();
    for centre in [-12.0, -6.0, 6.0, 12.0] {
        if let Some(read) = eye_at(&planes, &seam, size, reach, centre) {
            show(&format!("{centre:+.0} deg off"), &read);
            away.push(read);
        }
    }
    show("THE SEAM", &here);
    if !away.is_empty() {
        let mean = |slot: usize| {
            away.iter().map(|read| read.steepest[slot]).sum::<f64>() / away.len() as f64
        };
        let excess: Vec<f64> = [0usize, 1, 3, 5]
            .iter()
            .map(|slot| here.steepest[*slot] - mean(*slot))
            .collect();
        println!(
            "    {:<14}{:>+9.2}%{:>+9.2}%{:>+9.2}%{:>+9.2}%\n\
             \x20   ^ what the seam has that this content does not have anywhere. A\n\
             \x20     PHOTOMETRIC step is a difference in LEVEL and shows on content with\n\
             \x20     no gradient at all; a MISREGISTRATION is a difference in POSITION and\n\
             \x20     shows only where there is content to draw twice, at the lag its own\n\
             \x20     size in pixels puts it at, and no photometry moves it.",
            "the excess",
            100.0 * excess[0],
            100.0 * excess[1],
            100.0 * excess[2],
            100.0 * excess[3],
        );
    }
    println!("\n  controls, the same statistic through the same geometry:");
    match eye_at(&planes, &decoy, size, reach, 0.0) {
        Some(there) => println!(
            "    a circle with no handover on it, which is what the scene contributes\n      {}",
            there.report()
        ),
        None => println!("    the decoy circle is outside the window on this view"),
    }
    let rate = here.degrees_per_pixel;
    for (ratio, pixels) in [(1.0, 1.0), (1.02, 1.0), (1.05, 1.0), (1.05, 64.0)] {
        let Some(read) = eye_at(
            &flat(&seam, rate, 100.0, ratio, pixels),
            &seam,
            size,
            reach,
            0.0,
        ) else {
            continue;
        };
        let want = (ratio - 1.0) / ((ratio + 1.0) / 2.0);
        let lags = LAGS
            .iter()
            .zip(&read.steepest)
            .map(|(lag, held)| format!("{lag}px {:.3}%", 100.0 * held))
            .collect::<Vec<_>>()
            .join("  ");
        println!(
            "    a flat field with {ratio:.2} across it over {pixels:.0} px: step {:+.3}% \
             against {:+.3}% planted, {lags}",
            100.0 * read.step,
            100.0 * want,
        );
    }
}

// -------------------------------------------------- the field's own interior

/// How many azimuth bins the applied correction is read over, round the seam
/// circle.
///
/// Twice the [`kjerag_render::AZIMUTHS`] the field is measured at, so a stripe
/// one cell wide has two bins to be seen in and the statistic cannot alias the
/// very spacing it is looking for.
const SWEEP: usize = 256;

/// The crossover the projection asks for, in degrees.
///
/// Mirrored here rather than imported: it is private to its own module, and an
/// instrument that reaches into a shipped crate's internals is one that cannot
/// be run against a second build of that crate - which is exactly what this
/// instrument is for.
///
/// **Nothing checks this copy**, and that cost a wrong number the day the
/// width moved: the two lines it is left in scaled it by a leftover 2, which
/// printed "the 16.0 degree crossover" - wider than the whole 14.4 degree
/// overlap - and multiplied the `B end to end` column by sixteen instead of by
/// eight. It is the width itself in both places now. What the picture actually
/// hands over across is the camera's since 2026-08-05 and is asked of the map
/// wherever it decides anything ([`marked`]).
const CROSSOVER_DEG: f64 = 8.0;

/// A small symmetric positive definite system, by Gaussian elimination with no
/// pivoting.
///
/// The instrument's own, for the same reason.
fn solve5(mut normal: [[f64; 5]; 5], mut right: [f64; 5]) -> [f64; 5] {
    for pivot in 0..5 {
        let leading = normal[pivot];
        for row in (pivot + 1)..5 {
            let factor = normal[row][pivot] / leading[pivot];
            for (column, above) in leading.iter().enumerate().skip(pivot) {
                normal[row][column] -= factor * above;
            }
            right[row] -= factor * right[pivot];
        }
    }
    let mut out = [0.0f64; 5];
    for row in (0..5).rev() {
        let mut total = right[row];
        for column in (row + 1)..5 {
            total -= normal[row][column] * out[column];
        }
        out[row] = total / normal[row][row];
    }
    out
}

/// How far off the seam the interior is sampled, in degrees: away from the
/// handover itself, out where a wide correction is the only thing that can be
/// changing the picture.
///
/// The near end is past everything the handover reaches, which is half the
/// crossover plus the whole bend it carries: 6.6 degrees at the 8 the pass
/// asks for ([`kjerag_render::band::reach`]). It was 4.0 while the crossover
/// was 2, and 4.0 is inside the handover now.
const INTERIOR: (f64, f64) = (7.0, 60.0);

/// How dark "dark content" is, in codes of 255.
///
/// An ADDITIVE correction is a ratio of whatever it is added to, so a code on
/// 18-code soil is five percent and the same code on 190-code sky is a half of
/// one. The owner's streaks are on ploughed soil at sunset and every one of his
/// rejections has been on dark content; a statistic that averages the two
/// together is the same mistake stage 3 made in the other direction.
const DARK: f64 = 64.0;

/// **What the whole acceptance layer was blind to, by construction** (issue
/// #103, stage 8, after the owner rejected the branch).
///
/// Every statistic in this file straddles the seam. That measures the handover
/// and says nothing at all about what the correction does to the picture it is
/// painted over, and the owner's rejection was exactly that: *"there's weird
/// artifacts extending down and up"* - dark streaks across the soil, running
/// away from the seam. A per-direction field applied over a wide support paints
/// each direction's own value along the whole sweep of that direction, so a
/// difference between neighbouring directions that is noise becomes a STRIPE.
/// It is stage 5's scalloping, reborn on the photometric axis, and nothing here
/// could see it.
///
/// This reads the applied correction itself - the drawn picture minus the same
/// picture with the photometry held off, which is the field and nothing else -
/// at a band of angles AWAY from the handover, binned by the azimuth the field
/// is indexed by. What it reports is how much of that field is **not** smooth
/// round the ring: the rms of what a five-term harmonic cannot describe,
/// divided by the brightness it sits on, in Weber percent.
///
/// A smooth field reads zero however large it is. A striped one reads its
/// stripes.
#[derive(Clone, Copy, Debug, Default)]
struct Interior {
    /// The applied correction's mean size over the band, in codes.
    applied: f64,
    /// What is smooth round the ring, as Weber percent: the five-term fit.
    smooth: f64,
    /// What is NOT, as Weber percent. **The number.**
    rough: f64,
    /// The largest single step between neighbouring azimuth bins, Weber.
    step: f64,
    /// How many azimuth bins had any picture in them.
    bins: usize,
}

impl Interior {
    fn report(&self) -> String {
        format!(
            "applied {:.2} codes; smooth {:.2}%, ROUGH {:.2}%, worst neighbour step {:.2}%              ({} bins)",
            self.applied,
            100.0 * self.smooth,
            100.0 * self.rough,
            100.0 * self.step,
            self.bins,
        )
    }
}

/// The interior statistic over one pair of pictures.
///
/// `ripple` plants a known azimuthal ripple of that amplitude in codes into the
/// applied field before it is measured, which is the positive control: a
/// correction that is smooth round the ring plus a ripple has to read the
/// ripple back, and a run with no ripple and no correction has to read zero.
fn interior(
    before: &Picture,
    after: &Picture,
    reframe: &Reframe,
    size: Size,
    ripple: f64,
) -> Option<Interior> {
    let width = size.width as usize;
    // Sums per azimuth bin: the applied correction, the level it sits on, and
    // how many pixels answered.
    let mut held = vec![(0.0f64, 0.0f64, 0.0f64); SWEEP];
    for index in 0..(size.width * size.height) as usize {
        let uv = [
            (index % width) as f32 / size.width as f32,
            (index / width) as f32 / size.height as f32,
        ];
        let Some(ray) = reframe.view_ray(uv) else {
            continue;
        };
        let body = reframe.body_ray(ray);
        let length = (body[0] * body[0] + body[1] * body[1] + body[2] * body[2]).sqrt();
        if length <= 0.0 {
            continue;
        }
        let off = f64::from((body[2] / length).asin().to_degrees()).abs();
        if !(INTERIOR.0..=INTERIOR.1).contains(&off) {
            continue;
        }
        let phi = f64::from(body[1].atan2(body[0]));
        let bin = ((phi / std::f64::consts::TAU + 1.0) * SWEEP as f64) as usize % SWEEP;
        // Luma of what was applied, and of what it was applied to. Dark content
        // is where an additive correction is a large ratio and where the owner
        // is looking, and the level in the denominator is what makes this a
        // Weber number rather than a count of codes.
        let (mut lift, mut level) = (0.0, 0.0);
        for (channel, weight) in LUMA.iter().enumerate() {
            let a = f64::from(before.rgba[4 * index + channel]);
            let b = f64::from(after.rgba[4 * index + channel]);
            lift += weight * (b - a);
            level += weight * a;
        }
        if level <= 0.0 || level > DARK {
            continue;
        }
        let planted = ripple * (8.0 * phi).cos();
        // The FIELD in the numerator and the content in the denominator, each
        // averaged over the bin before they are divided. Dividing per pixel
        // instead puts the content's own roughness into the numerator, and the
        // statistic then reads the soil rather than the correction painted over
        // it - measured: it reported 0.89 percent of roughness for a field that
        // is smooth by construction.
        held[bin].0 += lift + planted;
        held[bin].1 += level;
        held[bin].2 += 1.0;
    }
    let seen: Vec<(f64, f64, f64)> = held
        .iter()
        .enumerate()
        .filter(|(_, bin)| bin.2 > 16.0)
        .map(|(index, bin)| {
            (
                index as f64 / SWEEP as f64 * std::f64::consts::TAU,
                bin.0 / bin.2,
                bin.1 / bin.2,
            )
        })
        .collect();
    if seen.len() < 16 {
        return None;
    }
    // The five terms a field CAN have and stay smooth: a constant, one cycle
    // and two. The same basis the geometry is fitted through, and the same one
    // stage 7's colour field used. Anything outside it is a stripe.
    let mut normal = [[0.0f64; 5]; 5];
    let mut right = [0.0f64; 5];
    for (phi, codes, _) in &seen {
        let basis = [
            1.0,
            phi.cos(),
            phi.sin(),
            (2.0 * phi).cos(),
            (2.0 * phi).sin(),
        ];
        for row in 0..5 {
            for column in 0..5 {
                normal[row][column] += basis[row] * basis[column];
            }
            right[row] += basis[row] * codes;
        }
    }
    let fitted = solve5(normal, right);
    let smooth_at = |phi: f64| -> f64 {
        let basis = [
            1.0,
            phi.cos(),
            phi.sin(),
            (2.0 * phi).cos(),
            (2.0 * phi).sin(),
        ];
        (0..5).map(|term| fitted[term] * basis[term]).sum()
    };
    let count = seen.len() as f64;
    let mut applied = 0.0;
    let mut smooth = 0.0;
    let mut rough = 0.0;
    for (phi, codes, level) in &seen {
        applied += codes.abs();
        smooth += (smooth_at(*phi) / level).powi(2);
        rough += ((codes - smooth_at(*phi)) / level).powi(2);
    }
    let mut step: f64 = 0.0;
    for pair in seen.windows(2) {
        let level = 0.5 * (pair[0].2 + pair[1].2);
        if level > 0.0 {
            step = step.max((pair[1].1 - pair[0].1).abs() / level);
        }
    }
    Some(Interior {
        applied: applied / count,
        smooth: (smooth / count).sqrt(),
        rough: (rough / count).sqrt(),
        step,
        bins: seen.len(),
    })
}

/// Each channel's step across the seam, and the same at the decoy.
fn across_seam(reframe: &Reframe, picture: &Picture, size: Size, window: (f64, f64, f64, f64)) {
    let planes = channels(picture);
    let seam = distances(reframe, size, 2, window);
    let decoy = distances(reframe, size, 0, window);
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
fn distances(
    reframe: &Reframe,
    size: Size,
    axis: usize,
    box_: (f64, f64, f64, f64),
) -> Vec<Option<f64>> {
    let width = size.width as usize;
    (0..(size.width * size.height) as usize)
        .map(|index| {
            let uv = [
                (index % width) as f32 / size.width as f32,
                (index / width) as f32 / size.height as f32,
            ];
            // A window on the picture, so a profile can be taken where ONE
            // kind of content crosses the seam. Over a whole wide view the
            // strips at a given distance from the seam run from sky to soil,
            // and their mean is an average of the scene rather than a reading
            // of the handover (issue #103, stage 7).
            if f64::from(uv[0]) < box_.0
                || f64::from(uv[0]) > box_.2
                || f64::from(uv[1]) < box_.1
                || f64::from(uv[1]) > box_.3
            {
                return None;
            }
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

// ------------------------------------------------------------ the flicker

/// What the shipped pass's own colour state does frame to frame.
///
/// **A pumping colour is worse than a step.** A step is still, and an eye stops
/// seeing what does not move; a hue that breathes is motion where the scene has
/// none. So the shipped numbers are watched over a run rather than sampled, and
/// the column means nothing without the positive control under it: a known step
/// put in with alternating sign has to come back at twice its size.
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
    // Per frame: the three gains, then the field evaluated at four azimuths a
    // quarter turn apart, which is what a view sees one of.
    let mut held: Vec<[f64; 16]> = Vec::new();
    while scene.frame().is_some() {
        Render {
            gpu: &gpu,
            scene: &scene,
            pipeline: &mut pipeline,
        }
        .frame(options.camera(), Sampling::default(), size)?;
        let tone = pipeline.band_tone(&gpu.device, &gpu.queue)?;
        // The shipped gain, which since this PR re-scoped is the only
        // photometric state the pass has. The columns that watched a
        // per-direction field went with the field.
        let mut row = [0.0f64; 16];
        row[0] = f64::from(tone.log_gain);
        held.push(row);
        if held.len() >= options.count || !scene.advance()? {
            break;
        }
    }
    println!(
        "\ntrace:  the colour state the shipped pass drew each of {} frames with, from {:.3} s.",
        held.len(),
        options.from,
    );
    // One code at a mid grey of 128 is ln(129/128): the smallest change an
    // 8-bit picture can carry, and what every number below is measured against.
    let one_code = (129.0f64 / 128.0).ln();
    println!(
        "\n  {:<34} {:>12} {:>12} {:>14}",
        "what", "ln rms/frame", "worst step", "codes under one"
    );
    let stepped = |column: usize, shake: f64| {
        let steps: Vec<f64> = held
            .windows(2)
            .enumerate()
            .map(|(index, pair)| {
                let shaken = |at: usize, value: f64| match at % 2 {
                    0 => value + shake,
                    _ => value - shake,
                };
                (shaken(index + 1, pair[1][column]) - shaken(index, pair[0][column])).abs()
            })
            .collect();
        let rms = (steps.iter().map(|s| s * s).sum::<f64>() / steps.len().max(1) as f64).sqrt();
        (rms, steps.iter().fold(0.0, |worst: f64, s| worst.max(*s)))
    };
    let mut columns: Vec<(String, usize)> = CHANNELS
        .iter()
        .enumerate()
        .map(|(channel, name)| (format!("the gain, {name}"), channel))
        .collect();
    for turn in 0..4 {
        for (channel, name) in CHANNELS.iter().enumerate() {
            columns.push((
                format!("the offset at {} deg, {name}", 90 * turn),
                3 + 3 * turn + channel,
            ));
        }
    }
    columns.push(("the openness at 0 deg".to_owned(), 15));
    let mut worst_rms: f64 = 0.0;
    for (name, column) in &columns {
        let (rms, worst) = stepped(*column, 0.0);
        worst_rms = worst_rms.max(rms);
        println!(
            "  {name:<34} {rms:>12.6} {worst:>12.6} {:>14.0}",
            one_code / rms.max(f64::MIN_POSITIVE),
        );
    }
    println!(
        "\n  one code at a mid grey of 128 is {one_code:.4} ln, so the worst column above is \n\
         \t{:.0}x under what an 8-bit picture can carry. a state that cannot move one code \n\
         \tbetween two frames cannot pump.",
        one_code / worst_rms.max(f64::MIN_POSITIVE),
    );
    println!(
        "\n  the positive control, which those columns mean nothing without: a known step \n\
         \tput into the G gain each frame with alternating sign has to come back at 2s, in \n\
         \tquadrature with what was already there.\n\n\
         \x20            step        read    expected"
    );
    let (flicker, _) = stepped(1, 0.0);
    for step in [0.002f64, 0.010] {
        println!(
            "         {step:>9.4} {:>11.5} {:>11.5}",
            stepped(1, step).0,
            flicker.hypot(2.0 * step),
        );
    }
    Ok(())
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
    /// Which corner-to-corner window of a DRAWN view the across-seam profile
    /// is read over, as fractions: left, top, right, bottom.
    window: (f64, f64, f64, f64),
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
            window: (0.0, 0.0, 1.0, 1.0),
        };
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("mode", value)) => {
                    options.mode = match value {
                        "field" => Mode::Field,
                        "profile" => Mode::Profile,
                        "studio" => Mode::Studio,
                        "trace" => Mode::Trace,
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
                Some(("box", value)) => {
                    let mut edges = value.split(':').map(str::parse::<f64>);
                    let mut next = || {
                        edges
                            .next()
                            .ok_or("box wants left:top:right:bottom")?
                            .map_err(|e| e.to_string())
                    };
                    options.window = (next()?, next()?, next()?, next()?);
                }
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

const USAGE: &str = "usage: colour <file.insv|export.mp4> [mode=field|profile|studio|trace] \
     [from=seconds] [count=frames] [places=n] [patches=n] [keep=r] [seam=factory] [verbose=1] \
     [yaw=deg] [pitch=deg] [fov=deg] [size=px] [lock=0] [out=dir] [tag=name] [reach=deg] \
     [rows=lo:hi] [cols=lo:hi] [box=left:top:right:bottom]";
