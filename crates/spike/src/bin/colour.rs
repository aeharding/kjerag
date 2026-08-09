//! What the two lenses' pictures of the **same content** differ by in each
//! colour channel, and how that difference behaves across the overlap band
//! (issue #103, stage 7).
//!
//! ```sh
//! # the decomposition round the ring: per channel, per content class, with every control
//! cargo run --release -p kjerag-spike --bin colour -- <file.insv> from=488.855 count=8
//! # the chromatic line's measuring phase, M1 to M5 in one run (chromatic.md 7.1)
//! cargo run --release -p kjerag-spike --bin colour -- <file.insv> mode=chroma \
//!   from=488.855 count=8 places=3
//! # the arm-internal per-channel discontinuity statistic on one drawn view
//! cargo run --release -p kjerag-spike --bin colour -- <file.insv> mode=arm \
//!   from=488.855 yaw=-5.17 pitch=2.56 fov=60 lock=1
//! # what a drawn view's three channels do as they cross the seam - the acceptance evidence
//! cargo run --release -p kjerag-spike --bin colour -- <file.insv> mode=profile \
//!   from=488.855 yaw=-5.17 pitch=2.56 fov=60 lock=1 out=scratch/stage7
//! # the same statistic on somebody else's stitch, from an equirectangular export
//! cargo run --release -p kjerag-spike --bin colour -- <export.mp4> mode=studio at=12.0
//! ```
//!
//! **`lock=1` is written out because it is the default and a bare `yaw=` does
//! not say which frame it is in.** Since 2026-08-06 that frame is world-fixed:
//! its zero is the file's opening heading rather than the followed one, so a
//! `yaw` copied from before that date points somewhere else and runs without a
//! word. The soil view above said `yaw=67.24` until that date and is the same
//! picture at `-5.17`. `new_yaw = old_yaw + carried(t)`, computed per line by
//! `--bin carried`, rule and re-derived registry in
//! docs/research/reference-views.md.
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
        Mode::Chroma => chroma(&options),
        Mode::Arm => arm(&options),
        Mode::Profile => profile(&options),
        Mode::Studio => studio(&options),
        Mode::Trace => trace(&options),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// The per-channel decomposition round the seam ring, and its controls.
    Field,
    /// The two lenses' own per-channel FAR-FIELD ratio round the ring, pooled
    /// the way the shipped pass pools its gain, decomposed into the achromatic
    /// term stage 3 already owns and the two chromatic degrees of freedom the
    /// chromatic line would estimate (docs/research/chromatic.md 7.1, M1 to
    /// M5).
    Chroma,
    /// The arm-internal per-channel discontinuity statistic on a drawn view:
    /// the band region against its own surrounding content, which is
    /// stretch-proof and is the instrument the oracle's own number was read
    /// with.
    Arm,
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
    /// Which frame of the run this was read on, counting from zero. The
    /// temporal question needs it: a reading that moves frame to frame and a
    /// reading that moves between sessions want different filters.
    frame: usize,
    /// Which of the run's places in the file this frame came from. Frames
    /// inside one place are consecutive; places are minutes apart. It is the
    /// difference between "does this reproduce over a second" and "does this
    /// reproduce over a flight", and those are two different questions about
    /// the same number.
    place: usize,
    columns: Vec<Column>,
    /// How far across the seam the alignment moved lens 1, in degrees. This is
    /// the quantity the shipped pass gates the exposure on.
    across: f64,
    /// Whether this direction's patch correlated at all on this frame.
    ///
    /// **Load-bearing, and it was measured to be** (docs/research/chromatic.md
    /// 4.2, which inherits it from 6.11's own defect). A direction that did not
    /// correlate is sampled at a shift of zero, so reading [`Self::across`]
    /// alone calls it far field and pools it with the horizon. That is the far
    /// field cut read as "the last reading a direction ever took" rather than
    /// as "the disparity the pass is drawing with", and it is worth 40 percent
    /// of this ring's population at the owner's dirt reference. The three
    /// populations are kept apart instead.
    correlated: bool,
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
    /// What the shipped pass actually pools: directions the alignment barely
    /// had to move, which is everything at infinity as far as a 33 mm baseline
    /// is concerned. [`kjerag_render::band::NEAR_KNEE_DEG`], imported rather
    /// than copied, so this cut is the pass's own and not a second opinion.
    Far,
    /// The rest, which is the wing, the lines and the cage: the darkest and
    /// hardest-to-align content on a flight, and where an apparent additive
    /// term was measured to come from (stage 3).
    Near,
    /// Directions whose patch did not correlate at all, and which are
    /// therefore read at a shift of zero. Sky and flat soil, which is where
    /// the owner sees the defect and where the shipped pass reads nothing
    /// (docs/research/chromatic.md 4.3).
    Blind,
}

/// The far-field knee, in degrees of across-seam disparity.
fn knee() -> f64 {
    f64::from(kjerag_render::band::NEAR_KNEE_DEG)
}

impl Class {
    fn holds(self, seen: &Seen) -> bool {
        match self {
            Self::All => true,
            Self::Flat => !seen.textured(),
            Self::Textured => seen.textured(),
            Self::Sun => seen.sun.is_some(),
            Self::NoSun => seen.sun.is_none(),
            Self::Far => seen.correlated && seen.across < knee(),
            Self::Near => seen.correlated && seen.across >= knee(),
            Self::Blind => !seen.correlated,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::All => "every reading",
            Self::Flat => "flat: under the band's own contrast gate",
            Self::Textured => "textured: what the band can correlate on",
            Self::Sun => "the sun in one lens",
            Self::NoSun => "no sun in either lens",
            Self::Far => "far field: what the shipped pass pools",
            Self::Near => "near field: over the pass's own knee",
            Self::Blind => "never correlated: read at zero shift",
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

/// The two facts about a frame that a READING needs and the sampling does not:
/// where in the file it came from, and what was in it.
///
/// One value rather than two arguments, because [`harvest`] already takes
/// everything else a frame is made of and a seventh loose scalar is how a
/// signature stops being readable.
#[derive(Clone, Copy)]
struct Whence {
    place: usize,
    sun: Option<usize>,
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
            let whence = Whence {
                place,
                sun: sun(&pair),
            };
            for (trial, field) in trials.iter().zip(&mut fields) {
                field.azimuths = ring.len();
                harvest(&reframe, &pair, &ring, &found, whence, *trial, field);
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
    whence: Whence,
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
            frame: field.frames - 1,
            place: whence.place,
            texture: held.texture(),
            columns,
            across: shift.1.abs(),
            correlated: hit.is_some(),
            sun: whence.sun,
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

// ------------------------------------- the chromatic line's own measurements

/// How bright a channel's mean has to be before its ratio is a colour reading
/// rather than a division of noise, in codes of 255.
///
/// **Stated rather than left to `lit` squared to imply**
/// (docs/research/chromatic.md 4.2). A weight makes a dark direction count for
/// little; a floor says a 3-code patch is not a measurement at all, and the
/// two are different claims. 8 codes is where the chroma plane's own
/// quantisation, which is one code about 128 on a quarter-resolution plane,
/// stops being a tenth of the signal and starts being a half of it.
const LEVEL_FLOOR: f64 = 8.0;

/// One azimuth-frame reduced to what the chromatic line estimates from.
///
/// **The two chroma coordinates are arm-internal by construction, and that is
/// the discipline rather than a convenience.** `d[R] - d[G]` is a difference
/// of two log ratios taken on the SAME patch of the SAME frame through the
/// SAME two lenses, so everything common to the three channels at that
/// direction cancels exactly: the scene's own level, the shading, the shutter,
/// the auto-exposure loop, and stage 3's pooled gain itself. What survives is
/// a difference of hue between the two lenses and nothing else. A per-channel
/// step read against an absolute is not that, and stage 7 measured what
/// reading against an absolute costs.
struct Read {
    azimuth: usize,
    frame: usize,
    /// Each lens's per-channel mean over the at-seam columns, in codes.
    m0: [f64; 3],
    m1: [f64; 3],
    count: f64,
    /// Lens 0's BT.709 luma, in codes: the `lit` the shipped pool weighs by.
    lit: f64,
    /// `ln(lens 1 / lens 0)` per channel.
    d: [f64; 3],
    /// The BT.709 achromatic part of `d`, which is stage 3's own quantity.
    luma: f64,
    /// `d` minus that, which carries no luminance by construction.
    c: [f64; 3],
    across: f64,
    textured: bool,
    place: usize,
    sun: Option<usize>,
}

impl Read {
    /// The green-magenta coordinate, in natural log. The oracle's own axis at
    /// the dirt end of the seam.
    fn warm(&self) -> f64 {
        self.d[0] - self.d[1]
    }

    /// The blue-amber coordinate, in natural log. The oracle's axis at the sky
    /// end, where it turns.
    fn cool(&self) -> f64 {
        self.d[2] - self.d[1]
    }
}

/// Every azimuth-frame that clears the level floor, reduced to a [`Read`].
fn reads(field: &Field, class: Class) -> Vec<Read> {
    field
        .seen
        .iter()
        .filter(|seen| class.holds(seen))
        .filter_map(|seen| {
            let held = seen.at_seam()?;
            let m0: [f64; 3] = std::array::from_fn(|channel| held.mean(0, channel));
            let m1: [f64; 3] = std::array::from_fn(|channel| held.mean(1, channel));
            if m0
                .iter()
                .chain(&m1)
                .any(|v| !v.is_finite() || *v < LEVEL_FLOOR)
            {
                return None;
            }
            let d: [f64; 3] = std::array::from_fn(|channel| (m1[channel] / m0[channel]).ln());
            if d.iter().any(|v| !v.is_finite()) {
                return None;
            }
            let luma: f64 = LUMA.iter().zip(d).map(|(w, v)| w * v).sum();
            Some(Read {
                azimuth: seen.azimuth,
                frame: seen.frame,
                m0,
                m1,
                count: held.count,
                lit: LUMA.iter().zip(m0).map(|(w, v)| w * v).sum(),
                d,
                luma,
                c: std::array::from_fn(|channel| d[channel] - luma),
                across: seen.across,
                textured: seen.textured(),
                place: seen.place,
                sun: seen.sun,
            })
        })
        .collect()
}

/// How a direction's reading is priced when the ring is pooled: the fork the
/// memo pre-registers rather than chooses (docs/research/chromatic.md 4.1, M3).
///
/// Only the weight changes between the three. The estimator underneath is one
/// estimator, so what the columns compare is the weighting and not three
/// different ideas.
#[derive(Clone, Copy, PartialEq)]
enum Weigh {
    /// What stage 3 shipped, and it was measured rather than chosen: of three
    /// poolings over nine captures, brightness squared left the smallest step
    /// at the seam on all nine. It is also the inverse-variance weight for a
    /// log ratio, because the noise on `ln(mean)` goes as one over the mean,
    /// so it is the statistically efficient answer to the achromatic question.
    Lit2,
    /// One direction, one vote. The mean of log ratios.
    Equal,
    /// The deliberate inverse of the shipped weight, and the reason the fork
    /// exists. `lit` squared prices a direction by how many photons it has;
    /// this prices it by how VISIBLE a fixed error there would be, which is
    /// Weber's law and is the axis every one of the owner's rejections has
    /// been on. A code on 18-code soil is 5.6 percent and the same code on
    /// 190-code sky is 0.5, and the shipped weight puts the soil at about one
    /// percent of the total (seam-blending.md's TL;DR, measured as one of the
    /// three reasons stage 7 could not reach the artifact).
    Weber,
}

impl Weigh {
    const ALL: [Self; 3] = [Self::Lit2, Self::Equal, Self::Weber];

    fn of(self, lit: f64) -> f64 {
        let lit = lit.max(LEVEL_FLOOR);
        match self {
            Self::Lit2 => lit * lit,
            Self::Equal => 1.0,
            Self::Weber => 1.0 / (lit * lit),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Lit2 => "lit squared (shipped)",
            Self::Equal => "equal weight",
            Self::Weber => "Weber (1 / lit squared)",
        }
    }
}

/// What the whole ring agrees the two lenses differ by, per channel, split
/// into the achromatic term stage 3 already owns and the two chromatic degrees
/// of freedom the chromatic line would estimate.
#[derive(Clone, Copy, Default)]
struct Pooled {
    /// `ln(lens 1 / lens 0)` per channel, pooled.
    d: [f64; 3],
    /// The BT.709 achromatic part of that.
    luma: f64,
    /// What is left: `d` minus the achromatic part, which sums to zero under
    /// the BT.709 weights by construction.
    c: [f64; 3],
    /// The scatter of the green-magenta coordinate over the ring's own
    /// directions, which is what a standard error is made of.
    warm: Reading,
    /// The same for the blue-amber coordinate.
    cool: Reading,
    /// How many readings were behind it, before and after the trim.
    readings: usize,
}

impl Pooled {
    fn warm(&self) -> f64 {
        self.d[0] - self.d[1]
    }

    fn cool(&self) -> f64 {
        self.d[2] - self.d[1]
    }

    /// The widest chroma coordinate, which is what a runaway guard is derived
    /// from (M4).
    fn widest(&self) -> f64 {
        self.warm().abs().max(self.cool().abs())
    }

    /// The standard error of the wider of the two coordinates.
    fn error(&self) -> f64 {
        (self.warm.spread / (self.warm.count.max(1) as f64).sqrt())
            .max(self.cool.spread / (self.cool.count.max(1) as f64).sqrt())
    }

    fn row(&self, label: &str) -> String {
        format!(
            "  {:<30} {:>9.5} {:>9.5} {:>9.5} {:>9.5} {:>9.5} {:>9.5} {:>8.5} {:>8.5} {:>6}",
            label,
            self.luma,
            self.c[0],
            self.c[1],
            self.c[2],
            self.warm(),
            self.cool(),
            self.warm.spread / (self.warm.count.max(1) as f64).sqrt(),
            self.cool.spread / (self.cool.count.max(1) as f64).sqrt(),
            self.readings,
        )
    }

    fn header() -> String {
        format!(
            "  {:<30} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>8} {:>8} {:>6}\n  \
             the first column is stage 3's own quantity and must not move; the next three \n  \
             are the chromatic term, and sum to zero under the BT.709 weights by construction.",
            "pooling", "achromatic", "cR", "cG", "cB", "R-G", "B-G", "se R-G", "se B-G", "n"
        )
    }
}

/// The pooled per-channel ratio, in the shipped estimator's own shape.
///
/// A **weighted ratio of means, then logged**, which is what `pooled_gain`
/// does and is a ratio of means rather than a mean of ratios because what a
/// correction inverts is the ratio of means. The only thing the fork changes
/// is the weight.
fn pool(reads: &[&Read], weigh: Weigh) -> Option<Pooled> {
    let mut weight = 0.0;
    let mut total = [0.0f64; 3];
    for read in reads {
        let held = weigh.of(read.lit);
        weight += held;
        for (channel, sum) in total.iter_mut().enumerate() {
            *sum += held * (read.m1[channel] / read.m0[channel]);
        }
    }
    if weight <= 0.0 {
        return None;
    }
    let d: [f64; 3] = std::array::from_fn(|channel| (total[channel] / weight).ln());
    if d.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let luma: f64 = LUMA.iter().zip(d).map(|(w, v)| w * v).sum();
    Some(Pooled {
        d,
        luma,
        c: std::array::from_fn(|channel| d[channel] - luma),
        warm: Reading::of(reads.iter().map(|r| (r.azimuth, r.warm()))),
        cool: Reading::of(reads.iter().map(|r| (r.azimuth, r.cool()))),
        readings: reads.len(),
    })
}

/// Drop the most extreme readings at each tail of each chroma coordinate.
///
/// **Per reading and not per direction**, because what this is defending
/// against is one frame's patch landing on content the two lenses do not
/// actually share - a bird, a rotor, a specular highlight in one lens only -
/// and that is a property of the reading. A direction that is genuinely odd
/// should survive as a direction, which is what the ring fit below is for.
fn trimmed<'a>(reads: &[&'a Read], share: f64) -> Vec<&'a Read> {
    if reads.len() < 10 {
        return reads.to_vec();
    }
    let cut = |values: &mut Vec<f64>| -> (f64, f64) {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let drop = ((values.len() as f64 * share).floor() as usize).max(1);
        (values[drop], values[values.len() - 1 - drop])
    };
    let (warm_low, warm_high) = cut(&mut reads.iter().map(|r| r.warm()).collect());
    let (cool_low, cool_high) = cut(&mut reads.iter().map(|r| r.cool()).collect());
    reads
        .iter()
        .copied()
        .filter(|r| (warm_low..=warm_high).contains(&r.warm()))
        .filter(|r| (cool_low..=cool_high).contains(&r.cool()))
        .collect()
}

/// What share of a reading's tails the trim takes off each end.
const TRIM: f64 = 0.10;

/// The chroma coordinates averaged per azimuth, weighted, for a ring fit.
///
/// Weights are normalized to sum to the direction count, because
/// [`ring_fit`]'s ridge is a fixed 1.0 and a weight of four million or of a
/// ten-thousandth would make that ridge either invisible or the whole answer.
fn by_direction(
    reads: &[&Read],
    weigh: Weigh,
    coordinate: usize,
    azimuths: usize,
) -> Vec<(f64, f64, f64)> {
    let mut held: Vec<(usize, f64, f64)> = Vec::new();
    for read in reads {
        let value = match coordinate {
            0 => read.warm(),
            _ => read.cool(),
        };
        let weight = weigh.of(read.lit);
        match held.iter_mut().find(|entry| entry.0 == read.azimuth) {
            Some(entry) => {
                entry.1 += value * weight;
                entry.2 += weight;
            }
            None => held.push((read.azimuth, value * weight, weight)),
        }
    }
    let total: f64 = held.iter().map(|entry| entry.2).sum();
    let scale = match total > 0.0 {
        true => held.len() as f64 / total,
        false => 1.0,
    };
    held.into_iter()
        .map(|(azimuth, sum, weight)| {
            (
                azimuth as f64 / azimuths.max(1) as f64 * std::f64::consts::TAU,
                sum / weight,
                weight * scale,
            )
        })
        .collect()
}

/// M1 to M5 over one capture: the whole measuring phase in one run.
fn chroma(options: &Options) -> Fallible<()> {
    // Five trials, and every control is a trial rather than a second code
    // path, which is stage 3's rule and the reason it is kept.
    let trials = [
        Trial::TRUTH,
        // THE NOISE FLOOR. One lens against ITSELF at the very shift the
        // alignment found, so the true chroma is exactly zero in every
        // channel and what this reads is what the instrument invents. M1's
        // pre-registered refusal is decided against this row.
        Trial {
            back: 0,
            ..Trial::TRUTH
        },
        // The same with no alignment at all, which is what a flat direction
        // gets.
        Trial {
            back: 0,
            aligned: false,
            ..Trial::TRUTH
        },
        // A 2 percent gain in R alone: a hue plant of a known size, which has
        // to come back as +0.019803 ln in R and 0.0 in G and B.
        Trial {
            gain: [1.02, 1.0, 1.0],
            ..Trial::TRUTH
        },
        // A green-magenta plant, which is the oracle's own axis: R and B up
        // together against G.
        Trial {
            gain: [1.01, 0.99, 1.01],
            ..Trial::TRUTH
        },
        // An additive plant in R, which a multiplicative estimator has to read
        // as a level-dependent gain and not as a constant one. M2's own
        // control.
        Trial {
            offset: [4.0, 0.0, 0.0],
            ..Trial::TRUTH
        },
    ];
    let fields = sweep(options, &trials)?;
    let truth = &fields[0];
    println!(
        "\nchroma: {} azimuth-frames read of {} tried, over {} frames from {:.3} s at {} \n\
         \tplace(s), {} azimuths round the seam. the two chroma coordinates below are \n\
         \tARM-INTERNAL: R-G is a difference of two log ratios on the same patch of the \n\
         \tsame frame, so the scene's own level, the shading and stage 3's pooled gain all \n\
         \tcancel exactly and what is left is a difference of hue between the lenses.",
        truth.seen.len(),
        truth.seen.len() + truth.refused,
        truth.frames,
        options.from,
        options.places.max(1),
        options.patches,
    );

    yields(truth, options);
    // TWO populations, and reporting both is the finding rather than a
    // hedge. What the shipped pass pools is the correlated far field, and on
    // these captures that is two to thirteen percent of the ring and almost
    // all of it is sky. What a chromatic estimator COULD read, if 4.3's
    // flat-content rule changed, is the whole ring at zero shift. M1's gate is
    // asked of both, because a split that exists on one and not the other is a
    // different answer to increment 1 than a split that exists on both.
    let mut floor = 0.0f64;
    for class in [Class::Far, Class::All] {
        let held = reads(truth, class);
        let kept: Vec<&Read> = held.iter().collect();
        if kept.is_empty() {
            println!("\nM1 ({}): nothing cleared the level floor.", class.name());
            continue;
        }
        floor = floor.max(split(&fields, truth, &kept, class));
    }
    // Everything downstream is asked of the WHOLE ring, because the far field
    // on these captures is sky and a weighting fork read on sky alone cannot
    // discriminate anything.
    let whole = reads(truth, Class::All);
    let kept: Vec<&Read> = whole.iter().collect();
    if kept.is_empty() {
        return Ok(());
    }
    local(&kept);
    fork(&kept);
    separate(truth);
    runaway(&kept, floor);
    temporal(truth, &kept);
    plants(&fields);
    Ok(())
}

/// M5: what the ring refuses, and to whom.
///
/// The defect lives on sky and on flat soil and the band refuses to correlate
/// on either, so what this counts is how much of the evidence the shipped
/// acceptance rule throws away before the estimator sees it. It decides
/// whether the flat-content rule changes in increment 1 or waits
/// (docs/research/chromatic.md 4.3).
fn yields(field: &Field, options: &Options) {
    let tried = field.seen.len() + field.refused;
    let count = |pick: &dyn Fn(&Seen) -> bool| field.seen.iter().filter(|s| pick(s)).count();
    let flat = count(&|s| !s.textured());
    let far = count(&|s| Class::Far.holds(s));
    let near = count(&|s| Class::Near.holds(s));
    let blind = count(&|s| Class::Blind.holds(s));
    let blind_flat = count(&|s| Class::Blind.holds(s) && !s.textured());
    let far_flat = count(&|s| Class::Far.holds(s) && !s.textured());
    let sunny = count(&|s| s.sun.is_some());
    let floored = reads(field, Class::All).len();
    let far_floored = reads(field, Class::Far).len();
    let blind_floored = reads(field, Class::Blind).len();
    let share = |part: usize| -> f64 { 100.0 * part as f64 / (tried as f64).max(1.0) };
    println!(
        "\nM5 (the ring's yield, at {} azimuths over {} frames). every line is a share of \n\
         \tthe {tried} azimuth-frames TRIED, so the refusals add up rather than nesting.\n",
        options.patches, field.frames,
    );
    println!("  {:<52} {:>8} {:>9}", "population", "count", "of tried");
    for (name, held) in [
        ("tried: azimuths times frames", tried),
        (
            "no pair at all (one lens has no picture, or clipped)",
            field.refused,
        ),
        ("read", field.seen.len()),
        ("CORRELATED and far field: what the pass pools", far),
        ("CORRELATED and near field", near),
        ("NEVER CORRELATED: the pass reads nothing here", blind),
        ("read AND flat: under the band's 6 code gate", flat),
        ("far AND flat", far_flat),
        ("blind AND flat: the defect's own content", blind_flat),
        ("read AND the sun in one lens", sunny),
        ("read AND over the 8 code level floor", floored),
        ("far AND over the level floor: M1's population", far_floored),
        (
            "blind AND over the level floor: what M5 would add",
            blind_floored,
        ),
    ] {
        println!("  {name:<52} {held:>8} {:>8.1}%", share(held));
    }
    println!(
        "\n  the NEVER CORRELATED share is the number 4.3 asks for: those directions are \n\
         \tsampled at a shift of zero and the shipped pass pools none of them, and they are \n\
         \twhere the owner sees the defect. reading them as far field because their shift \n\
         \thappens to be zero is the 6.11 defect and is refused here."
    );
}

/// M1: is there a hemisphere-scale chroma split at all, is it above the
/// instrument's own noise, does it reproduce, and how much of it is a constant.
///
/// Returns the noise floor it measured, in natural log, so M4 can quote it.
fn split(fields: &[Field], truth: &Field, kept: &[&Read], class: Class) -> f64 {
    println!(
        "\nM1 on `{}`. the two lenses' own per-channel ratio round the ring, pooled the way \n\
         \tthe shipped pass pools its gain. if it is inside the instrument's noise the \n\
         \tincrement is REFUSED before it is built, and the memo says so in advance \n\
         \t(docs/research/chromatic.md 3.2).\n",
        class.name(),
    );
    // What the population M1 pools actually is, so the pooled number below is
    // read beside the evidence under it rather than on its own.
    let samples: f64 = kept.iter().map(|r| r.count).sum();
    let flat = kept.iter().filter(|r| !r.textured).count();
    let sunny = kept.iter().filter(|r| r.sun.is_some()).count();
    let mut disparity: Vec<f64> = kept.iter().map(|r| r.across).collect();
    disparity.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut level: Vec<f64> = kept.iter().map(|r| r.lit).collect();
    level.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let achromatic = Reading::of(kept.iter().map(|r| (r.azimuth, r.luma)));
    println!(
        "  the population: {} readings over {samples:.0} paired samples, {flat} of them on \n\
         \tflat content and {sunny} with the sun in one lens. median across-seam disparity \n\
         \t{:.3} deg (the pass's own knee is {:.2}); level runs {:.0} to {:.0} codes, median \n\
         \t{:.0}. the achromatic term over the same readings is {:+.5} ln, spread {:.5}.\n",
        kept.len(),
        disparity[disparity.len() / 2],
        knee(),
        level[0],
        level[level.len() - 1],
        level[level.len() / 2],
        achromatic.mean,
        achromatic.spread,
    );
    println!("{}", Pooled::header());
    let whole = pool(kept, Weigh::Lit2);
    if let Some(read) = whole {
        println!("{}", read.row("untrimmed"));
    }
    let cut = trimmed(kept, TRIM);
    let trim = pool(&cut, Weigh::Lit2);
    if let Some(read) = trim {
        println!("{}", read.row("10% trimmed"));
    }
    // The three sub-populations pooled the same way, which is what M5's
    // decision turns on: if the directions the pass reads NOTHING on agree
    // with the ones it pools, then reading them buys evidence and no bias, and
    // if they do not then the flat-content rule is a change of population and
    // not a change of yield.
    for held in [Class::Far, Class::Near, Class::Blind] {
        let rows = reads(truth, held);
        let rows: Vec<&Read> = rows.iter().collect();
        if let Some(read) = pool(&trimmed(&rows, TRIM), Weigh::Lit2) {
            println!("{}", read.row(&format!("  of which {}", held.name())));
        }
    }
    // THE FLOOR, on this very population. The null trial is one lens against
    // its own picture displaced by the shift the alignment found, so its true
    // chroma is zero by arithmetic and everything it reads is the instrument.
    // Measured on the same class as the signal, because a floor read on a
    // different population is a floor for a different measurement.
    let mut floor = 0.0f64;
    for (index, label) in [
        (1usize, "FLOOR: lens 0 vs itself, aligned"),
        (2, "FLOOR: the same, unaligned"),
    ] {
        let null = reads(&fields[index], class);
        let held: Vec<&Read> = null.iter().collect();
        if let Some(read) = pool(&trimmed(&held, TRIM), Weigh::Lit2) {
            println!("{}", read.row(label));
            // Both halves of the floor: the bias the null pools to, and its
            // own scatter. On directions that never correlated the null is
            // sampled at a shift of zero, so it is EXACTLY zero by
            // arithmetic and the bias term is not a floor at all; what is
            // left there is the chroma plane's own quantisation, which the
            // standard error measures.
            floor = floor.max(read.widest()).max(read.error());
        }
    }
    let Some(trim) = trim else {
        println!("\n  nothing pooled. No verdict.");
        return floor;
    };
    // Does it reproduce WITHIN the capture? The run's places are minutes apart
    // in the same file, which is the cheapest hold-out there is and the one
    // 3.3's first row asks for by name.
    let places = kept.iter().map(|r| r.place).max().unwrap_or(0) + 1;
    let mut per_place: Vec<(f64, f64)> = Vec::new();
    for place in 0..places {
        let held: Vec<&Read> = kept.iter().copied().filter(|r| r.place == place).collect();
        if let Some(read) = pool(&trimmed(&held, TRIM), Weigh::Lit2) {
            println!("{}", read.row(&format!("place {place} of the capture")));
            per_place.push((read.warm(), read.cool()));
        }
    }
    let reproduces = match per_place.len() > 1 {
        true => {
            let span = |pick: fn(&(f64, f64)) -> f64| -> f64 {
                per_place.iter().map(pick).fold(f64::MIN, f64::max)
                    - per_place.iter().map(pick).fold(f64::MAX, f64::min)
            };
            Some((span(|p| p.0), span(|p| p.1)))
        }
        false => None,
    };

    // How much of the split is a CONSTANT, which is the only thing increment
    // 1's two degrees of freedom can reach, and how much is local. The five
    // terms are the same basis the geometry is fitted through.
    println!(
        "\n  how much of it a CONSTANT reaches. the readings are averaged per direction and \n\
         \tfitted round the ring; each row is what the fit LEAVES, in natural log rms over \n\
         \tthe directions. the drop from `nothing` to `a constant` is the DC term's share, \n\
         \tand whatever the five terms cannot describe is local to the seam and out of \n\
         \treach of any hemisphere-wide model.\n"
    );
    println!(
        "  {:<12} {:>10} {:>10} {:>10} {:>10} {:>12}",
        "coordinate", "nothing", "constant", "+1 cycle", "+2 cycles", "DC value"
    );
    for (coordinate, name) in [(0usize, "R-G"), (1, "B-G")] {
        let points = by_direction(kept, Weigh::Lit2, coordinate, truth.azimuths);
        if points.len() < 5 {
            println!("  {name:<12} too few directions to fit");
            continue;
        }
        let leaves: Vec<f64> = [0usize, 1, 3, 5]
            .iter()
            .map(|terms| ring_fit(&points, *terms).1)
            .collect();
        let (fitted, _) = ring_fit(&points, 1);
        println!(
            "  {name:<12} {:>10.5} {:>10.5} {:>10.5} {:>10.5} {:>12.5}   ({} directions)",
            leaves[0],
            leaves[1],
            leaves[2],
            leaves[3],
            fitted[0],
            points.len(),
        );
    }

    // The pre-registered verdict, computed rather than narrated.
    let signal = trim.widest();
    let se = trim.error();
    println!(
        "\n  VERDICT. widest chroma coordinate {signal:.5} ln ({:.2}% of level). instrument \n\
         \tfloor {floor:.5} ln{}. standard error {se:.5} ln, so the reading is {:.1} se from \n\
         \tzero.{}",
        100.0 * (signal.exp() - 1.0),
        match floor > 1e-5 {
            true => format!(", ratio {:.1}x", signal / floor),
            false => ", which is the null pooling to zero BY ARITHMETIC on directions read \n\
                      \tat a shift of zero: there the standard error is the whole floor"
                .to_owned(),
        },
        signal / se.max(f64::MIN_POSITIVE),
        match reproduces {
            Some((warm, cool)) => format!(
                "\n\tover the run's places, minutes apart in the same file, the reading spans \n\
                 \t{warm:.5} in R-G and {cool:.5} in B-G, against a signal of {signal:.5}.",
            ),
            None => String::new(),
        },
    );
    floor
}

/// Is the per-direction structure REAL, or is it the noise stage 8 painted?
///
/// **The decisive question for any correction whose support is local**, and it
/// is not in the memo above because the memo was scoped to a constant. The ring
/// leaves per-direction structure that no smooth model reaches (M1.5), and a
/// seam-local field is exactly a thing that would fit it. Whether fitting it is
/// estimation or is stage 5's scalloping reborn on the photometric axis turns
/// on one measurement: **does the same direction read the same thing twice.**
///
/// Three numbers, and the third is the one that decides:
///
/// - **within**: the spread of one direction's readings over consecutive frames
///   inside one place, where the content is the same and the two lenses have
///   not moved. That is the instrument, in full.
/// - **between**: the spread over directions of each direction's own mean,
///   after the ring's constant is taken out. That is what a per-direction field
///   would fit, and it contains the instrument's noise as well as any structure.
/// - **corrected**: `between` with `within` divided out of it, which is the
///   structure that is left when the noise is accounted for. A field can only
///   honestly reach this much.
///
/// And then the test that noise cannot pass: the same directions read at two
/// PLACES minutes apart in the same file, correlated against each other. Local
/// structure that belongs to the lens pair reproduces; local structure that is
/// the scene or the correlator does not.
fn local(kept: &[&Read]) {
    println!(
        "\nlocal structure. what a smooth ring model leaves is fitted by anything with local \n\
         \tsupport, so the question is whether it is real. `within` is one direction read on \n\
         \tconsecutive frames of the same place, which is the instrument and nothing else; \n\
         \t`between` is the spread over directions after the constant is removed; `corrected` \n\
         \tis what survives dividing the first out of the second.\n"
    );
    println!(
        "  {:<12} {:>10} {:>10} {:>11} {:>9} {:>26}",
        "coordinate", "within", "between", "corrected", "real %", "same directions, 2 places"
    );
    for (coordinate, name) in [(0usize, "R-G"), (1, "B-G")] {
        let value = |read: &Read| match coordinate {
            0 => read.warm(),
            _ => read.cool(),
        };
        // Group by place and direction, which is the only grouping where the
        // content is genuinely the same thing read twice.
        let mut groups: Vec<((usize, usize), Vec<f64>)> = Vec::new();
        for read in kept {
            let key = (read.place, read.azimuth);
            match groups.iter_mut().find(|held| held.0 == key) {
                Some(held) => held.1.push(value(read)),
                None => groups.push((key, vec![value(read)])),
            }
        }
        let mut error = 0.0;
        let mut freedom = 0.0;
        let mut sizes = 0.0;
        let mut counted = 0.0;
        for (_, values) in groups.iter().filter(|group| group.1.len() > 1) {
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            for held in values {
                error += (held - mean).powi(2);
            }
            freedom += (values.len() - 1) as f64;
            sizes += values.len() as f64;
            counted += 1.0;
        }
        if freedom < 1.0 || counted < 4.0 {
            println!("  {name:<12} too few repeats to say");
            continue;
        }
        let within = (error / freedom).sqrt();
        let per_group = sizes / counted;
        // The per-direction means, per place, with that place's own constant
        // removed so what is measured is the SHAPE round the ring and not the
        // DC M1 already reported.
        let mut places: Vec<(usize, Vec<(usize, f64)>)> = Vec::new();
        for ((place, azimuth), values) in &groups {
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            match places.iter_mut().find(|held| held.0 == *place) {
                Some(held) => held.1.push((*azimuth, mean)),
                None => places.push((*place, vec![(*azimuth, mean)])),
            }
        }
        let mut spread = 0.0;
        let mut directions = 0.0;
        let mut centred: Vec<(usize, Vec<(usize, f64)>)> = Vec::new();
        for (place, rows) in &places {
            if rows.len() < 4 {
                continue;
            }
            let mean = rows.iter().map(|r| r.1).sum::<f64>() / rows.len() as f64;
            let rows: Vec<(usize, f64)> = rows.iter().map(|r| (r.0, r.1 - mean)).collect();
            for (_, held) in &rows {
                spread += held * held;
                directions += 1.0;
            }
            centred.push((*place, rows));
        }
        if directions < 4.0 {
            println!("  {name:<12} too few directions to say");
            continue;
        }
        let between = (spread / directions).sqrt();
        let corrected = (between * between - within * within / per_group)
            .max(0.0)
            .sqrt();
        // The test noise cannot pass: the same directions at two places.
        let mut pairs: Vec<(f64, f64)> = Vec::new();
        for first in 0..centred.len() {
            for second in (first + 1)..centred.len() {
                for (azimuth, held) in &centred[first].1 {
                    if let Some((_, other)) = centred[second].1.iter().find(|r| r.0 == *azimuth) {
                        pairs.push((*held, *other));
                    }
                }
            }
        }
        let agreement = match pairs.len() > 8 {
            true => {
                let n = pairs.len() as f64;
                let mx = pairs.iter().map(|p| p.0).sum::<f64>() / n;
                let my = pairs.iter().map(|p| p.1).sum::<f64>() / n;
                let mut sxy = 0.0;
                let mut sxx = 0.0;
                let mut syy = 0.0;
                for (x, y) in &pairs {
                    sxy += (x - mx) * (y - my);
                    sxx += (x - mx).powi(2);
                    syy += (y - my).powi(2);
                }
                match sxx > 0.0 && syy > 0.0 {
                    true => format!(
                        "r {:+.3} over {} pairs",
                        sxy / (sxx * syy).sqrt(),
                        pairs.len()
                    ),
                    false => "degenerate".to_owned(),
                }
            }
            false => "too few shared directions".to_owned(),
        };
        println!(
            "  {name:<12} {within:>10.5} {between:>10.5} {corrected:>11.5} {:>8.0}% {agreement:>26}",
            100.0 * corrected / between.max(f64::MIN_POSITIVE),
        );
    }
    println!(
        "\n  a correlation near zero between two places says the per-direction structure is \n\
         \tNOT a property of the lens pair, and a field that fits it paints the scene's own \n\
         \tnoise along each direction's whole sweep. that is stage 8, and it is what the \n\
         \towner rejected."
    );
}

/// M3: the weighting fork, three columns, pre-registered rather than chosen.
fn fork(kept: &[&Read]) {
    println!(
        "\nM3 (the weighting fork). `lit` squared was fitted on the ACHROMATIC question and \n\
         \tis measured to put the directions where the defect is visible at about one \n\
         \tpercent of the weight. so the chromatic term's weighting is a decision of its \n\
         \town, and all three columns are reported (docs/research/chromatic.md 4.1).\n"
    );
    println!("{}", Pooled::header());
    for weigh in Weigh::ALL {
        if let Some(read) = pool(&trimmed(kept, TRIM), weigh) {
            println!("{}", read.row(weigh.name()));
        }
    }
    // And the same three on the DARK half of the ring alone, which is the
    // content the owner's every rejection has been on. If the three columns
    // agree there, the fork does not matter and that is itself the finding.
    let mut levels: Vec<f64> = kept.iter().map(|r| r.lit).collect();
    levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = levels[levels.len() / 2];
    let dark: Vec<&Read> = kept.iter().copied().filter(|r| r.lit <= median).collect();
    let light: Vec<&Read> = kept.iter().copied().filter(|r| r.lit > median).collect();
    println!(
        "\n  and split at the ring's own median level of {median:.0} codes, because a \n\
         \tweighting only matters where the two halves disagree:\n"
    );
    for (name, half) in [("dark half", &dark), ("light half", &light)] {
        for weigh in Weigh::ALL {
            if let Some(read) = pool(&trimmed(half, TRIM), weigh) {
                println!("{}", read.row(&format!("{name}, {}", weigh.name())));
            }
        }
    }
}

/// M2: gain or offset, over the ring's own dynamic range.
fn separate(field: &Field) {
    println!(
        "\nM2 (gain or offset). 6.11 could not separate them on one view's flat patches; the \n\
         \tring's own 20-to-190 code range can. a multiplicative correction cannot fix an \n\
         \tadditive defect, and the last column is what says which this is."
    );
    models(field, Class::Far);
    models(field, Class::Near);
    models(field, Class::Blind);
}

/// M4: the runaway guard, per channel, re-derived rather than copied.
fn runaway(kept: &[&Read], floor: f64) {
    // The FITTED value and not the widest single reading, because that is how
    // LIMIT_LN itself was derived: `--bin expose` fitted the achromatic ratio
    // over whole captures and took the widest of those fits, times four. A
    // guard sized on the widest single reading a dark patch ever produced
    // would be a guard sized on the noise it exists to catch.
    let mut fitted = 0.0f64;
    for weigh in Weigh::ALL {
        if let Some(read) = pool(&trimmed(kept, TRIM), weigh) {
            fitted = fitted.max(read.widest());
        }
    }
    let mut per_reading: Vec<f64> = kept
        .iter()
        .map(|r| r.c.iter().fold(0.0f64, |held, v| held.max(v.abs())))
        .collect();
    per_reading.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let widest = *per_reading.last().unwrap_or(&0.0);
    let p99 = per_reading[(per_reading.len() * 99 / 100).min(per_reading.len() - 1)];
    println!(
        "\nM4 (the runaway guard). LIMIT_LN is 0.25 because the ACHROMATIC ratio was measured \n\
         \tat 0.946 to 1.004 over seven captures and the guard is the widest FIT, times four. \n\
         \t0.25 ln of hue is 28 percent, which is far outside anything 6.11 measured.\n\
         \n  this capture: widest FITTED chroma coordinate over the three weightings \n\
         \t{fitted:.5} ln, which asks for {:.4} at four times. instrument floor {floor:.5}.\n\
         \tfor scale, the widest SINGLE reading is {widest:.5} ln and the 99th percentile \n\
         \t{p99:.5}, which is what a guard would be sized at if it were sized on the noise \n\
         \tit exists to catch. the corpus number is the widest fit over every flight and is \n\
         \tassembled in the results section, not here.",
        4.0 * fitted,
    );
}

/// The temporal question: how fast does the term move, and what filter class
/// does that make it?
fn temporal(field: &Field, kept: &[&Read]) {
    println!(
        "\ntemporal. the band's own reasoning applies twice over: a gain that flickers \n\
         \tchanges the brightness of everything, and a white balance that flickers changes \n\
         \tthe COLOUR of everything. what decides the filter is whether the reading moves \n\
         \tframe to frame or only between sessions.\n"
    );
    println!("  {:>7} {:>10} {:>10} {:>8}", "frame", "R-G", "B-G", "n");
    let mut series: Vec<(usize, f64, f64)> = Vec::new();
    for frame in 0..field.frames {
        let held: Vec<&Read> = kept.iter().copied().filter(|r| r.frame == frame).collect();
        if held.len() < 4 {
            continue;
        }
        if let Some(read) = pool(&held, Weigh::Lit2) {
            println!(
                "  {frame:>7} {:>10.5} {:>10.5} {:>8}",
                read.warm(),
                read.cool(),
                held.len()
            );
            series.push((frame, read.warm(), read.cool()));
        }
    }
    if series.len() < 2 {
        println!("  too few frames answered to say anything about time.");
        return;
    }
    let mut worst = (0.0f64, 0.0f64);
    let mut rms = (0.0f64, 0.0f64);
    for pair in series.windows(2) {
        let warm = (pair[1].1 - pair[0].1).abs();
        let cool = (pair[1].2 - pair[0].2).abs();
        worst = (worst.0.max(warm), worst.1.max(cool));
        rms.0 += warm * warm;
        rms.1 += cool * cool;
    }
    let steps = (series.len() - 1) as f64;
    let span = |pick: fn(&(usize, f64, f64)) -> f64| -> f64 {
        let values: Vec<f64> = series.iter().map(pick).collect();
        values.iter().copied().fold(f64::MIN, f64::max)
            - values.iter().copied().fold(f64::MAX, f64::min)
    };
    println!(
        "\n  frame to frame: worst {:.5} / {:.5} ln, rms {:.5} / {:.5}, over {} steps. \n\
         \tthe whole run's span is {:.5} / {:.5} ln. a reading whose frame-to-frame noise \n\
         \tis the same size as its whole span is a CONSTANT seen through noise, and wants \n\
         \tthe tone gain's filter class: a first-order ease at TAU_GAIN_S, no events, no \n\
         \tstates, no thresholds.",
        worst.0,
        worst.1,
        (rms.0 / steps).sqrt(),
        (rms.1 / steps).sqrt(),
        steps as usize,
        span(|s| s.1),
        span(|s| s.2),
    );
}

/// Every plant, beside the number it has to produce.
///
/// A per-channel reading is a negative result until it is shown able to read a
/// positive one, and each of these runs the SAME code on the SAME frames: only
/// one multiplier and one addend change.
fn plants(fields: &[Field]) {
    println!(
        "\nplants. every trial runs the same code on the same frames. the expected column is \n\
         \tarithmetic, not a previous run.\n"
    );
    println!(
        "  {:<40} {:>10} {:>10} {:>26}",
        "trial", "R-G", "B-G", "expected R-G / B-G"
    );
    // Read on the WHOLE ring rather than on the far field, because on these
    // captures the far field is two to thirteen percent of it and on hard
    // mode it is one reading, and a control read on one reading has cleared
    // nothing.
    let base = reads(&fields[0], Class::All);
    let held: Vec<&Read> = base.iter().collect();
    let truth = pool(&trimmed(&held, TRIM), Weigh::Lit2);
    let (warm, cool) = truth.map_or((0.0, 0.0), |read| (read.warm(), read.cool()));
    for (index, label, expect) in [
        (
            1usize,
            "lens 0 vs itself at the found shift",
            Some((0.0, 0.0)),
        ),
        (2, "lens 0 vs itself, no alignment", Some((0.0, 0.0))),
        (
            3,
            "lens 1 times 1.02 in R alone",
            Some((warm + 1.02f64.ln(), cool)),
        ),
        (
            4,
            "lens 1 times 1.01 / 0.99 / 1.01",
            Some((warm + (1.01f64 / 0.99).ln(), cool + (1.01f64 / 0.99).ln())),
        ),
        (5, "lens 1 plus 4 codes in R alone", None),
    ] {
        let held = reads(&fields[index], Class::All);
        let rows: Vec<&Read> = held.iter().collect();
        let Some(read) = pool(&trimmed(&rows, TRIM), Weigh::Lit2) else {
            continue;
        };
        println!(
            "  {label:<40} {:>10.5} {:>10.5} {:>26}",
            read.warm(),
            read.cool(),
            match expect {
                Some((w, c)) => format!("{w:+.5} / {c:+.5}"),
                None => "level-dependent, see M2".to_owned(),
            },
        );
    }
}

// ---------------------------------------------- the arm-internal instrument

/// How far off the seam the arm's own surrounding content starts and stops,
/// in degrees, measured from the edge of the band region.
///
/// Close enough that it is the SAME content the band sits on - the oracle's
/// number is a band of dirt against the dirt round it - and far enough out
/// that the handover itself is not in it.
const SURROUND_DEG: (f64, f64) = (1.0, 5.0);

/// The band region against its own surrounding content, per channel.
///
/// **This is the instrument the oracle's own number was read with**, and the
/// memo's own complaint was that it was a thing one session measured once
/// (docs/research/chromatic.md 7.2). It is a mode now.
///
/// Why it is the valid one: it is **stretch-proof**. Every number below is a
/// difference taken inside one picture between two regions of the same
/// content, so a tone curve, a display stretch, a global gain or a JPEG
/// encode's own gamma moves both regions together and cancels out of the
/// difference. A reading of the band against an absolute does not have that
/// property and stage 7 measured what reading against an absolute costs. And
/// the Weber law it serves is that dark content is judged relative: the split
/// is reported against the warmth it sits on as well as in codes, because 5
/// codes on 11-code warmth is 45 percent and the same 5 codes on the sky end
/// is under 2.
#[derive(Clone, Copy, Default)]
struct Arm {
    band: [f64; 3],
    surround: [f64; 3],
    band_pixels: usize,
    surround_pixels: usize,
}

impl Arm {
    fn warmth(region: [f64; 3]) -> f64 {
        region[0] - region[1]
    }

    fn coolth(region: [f64; 3]) -> f64 {
        region[2] - region[1]
    }

    /// The number: how much warmer the band is than the content round it, in
    /// codes.
    fn warm_split(&self) -> f64 {
        Self::warmth(self.band) - Self::warmth(self.surround)
    }

    fn cool_split(&self) -> f64 {
        Self::coolth(self.band) - Self::coolth(self.surround)
    }

    /// The same split as a log ratio of ratios, which is exactly invariant to
    /// any per-channel gain applied to the whole picture.
    fn stretch_proof(&self) -> (f64, f64) {
        let ratio = |channel: usize| (self.band[channel] / self.surround[channel]).ln();
        (ratio(0) - ratio(1), ratio(2) - ratio(1))
    }

    fn level(&self) -> f64 {
        LUMA.iter().zip(self.surround).map(|(w, v)| w * v).sum()
    }

    fn row(&self, label: &str) -> String {
        let (warm, cool) = self.stretch_proof();
        format!(
            "  {label:<34} {:>7.1} {:>8.2} {:>8.2} {:>9.2} {:>9.2} {:>8.1} {:>8.1} {:>8.4} {:>8.4} {:>7}/{}",
            self.level(),
            Self::warmth(self.surround),
            Self::coolth(self.surround),
            self.warm_split(),
            self.cool_split(),
            100.0 * self.warm_split() / Self::warmth(self.surround).abs().max(f64::MIN_POSITIVE),
            100.0 * self.cool_split() / Self::coolth(self.surround).abs().max(f64::MIN_POSITIVE),
            warm,
            cool,
            self.band_pixels,
            self.surround_pixels,
        )
    }

    fn header() -> String {
        format!(
            "  {:<34} {:>7} {:>8} {:>8} {:>9} {:>9} {:>8} {:>8} {:>8} {:>8} {:>7}",
            "region",
            "level",
            "R-G out",
            "B-G out",
            "R-G split",
            "B-G split",
            "rel %",
            "rel %",
            "ln R-G",
            "ln B-G",
            "px in/out"
        )
    }
}

/// The arm-internal statistic over one picture about one great circle.
///
/// `plant` is added to the BAND region alone, per channel, which is a chroma
/// stripe laid on the seam of a known size: the positive control.
fn arm_read(
    planes: &[Vec<f64>; 3],
    distance: &[Option<f64>],
    half: f64,
    plant: [f64; 3],
) -> Option<Arm> {
    let mut band = [0.0f64; 3];
    let mut surround = [0.0f64; 3];
    let (mut inside, mut outside) = (0usize, 0usize);
    for index in 0..planes[0].len() {
        let Some(at) = distance[index] else {
            continue;
        };
        let off = at.abs();
        let level: f64 = LUMA
            .iter()
            .enumerate()
            .map(|(channel, weight)| weight * planes[channel][index])
            .sum();
        if level <= 0.0 {
            continue;
        }
        if off <= half {
            inside += 1;
            for channel in 0..3 {
                band[channel] += planes[channel][index] + plant[channel];
            }
        } else if (half + SURROUND_DEG.0..=half + SURROUND_DEG.1).contains(&off) {
            outside += 1;
            for channel in 0..3 {
                surround[channel] += planes[channel][index];
            }
        }
    }
    if inside < 256 || outside < 256 {
        return None;
    }
    Some(Arm {
        band: std::array::from_fn(|channel| band[channel] / inside as f64),
        surround: std::array::from_fn(|channel| surround[channel] / outside as f64),
        band_pixels: inside,
        surround_pixels: outside,
    })
}

fn arm(options: &Options) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let size = Size::new(options.size, options.size);
    let (before, mapped, _) = drawn(options, &gpu, size, true)?;
    let (after, _, tone) = drawn(options, &gpu, size, false)?;
    let half = f64::from(mapped.handover_width().to_degrees()) / 2.0;
    println!(
        "\narm: the band region is the handover's own half-width, {half:.2} degrees either \n\
         \tside of the seam, asked of the map this render was drawn with. its surrounding \n\
         \tcontent is {:.1} to {:.1} degrees past that edge. the split columns are the \n\
         \tband's warmth minus the surround's, which is a difference taken inside one \n\
         \tpicture and is therefore stretch-proof; `rel %` is the same against the warmth \n\
         \tit sits on, which is Weber and is how dark content is judged.\n",
        SURROUND_DEG.0, SURROUND_DEG.1,
    );
    println!("{}", Arm::header());
    let seam = distances(&mapped, size, 2, options.window);
    let decoy = distances(&mapped, size, 0, options.window);
    for (label, picture) in [("band held off", &before), ("as it draws", &after)] {
        let planes = channels(picture);
        if let Some(read) = arm_read(&planes, &seam, half, [0.0; 3]) {
            println!("{}", read.row(&format!("{label}: at the seam")));
        }
        // P.1's own control: the same statistic about a circle the correction
        // has no business touching. If the decoy moves, the change belongs to
        // the scene and not to the correction.
        if let Some(read) = arm_read(&planes, &decoy, half, [0.0; 3]) {
            println!("{}", read.row(&format!("{label}: at the DECOY")));
        }
    }
    println!("\n  controls:");
    let planes = channels(&before);
    for (label, plant) in [
        ("nothing planted", [0.0f64; 3]),
        ("+2 codes of R on the band alone", [2.0, 0.0, 0.0]),
        ("a 2 code green-magenta stripe", [2.0, -0.7963, 2.0]),
    ] {
        if let Some(read) = arm_read(&planes, &seam, half, plant) {
            println!("{}", read.row(label));
        }
    }
    println!(
        "\n  gain: the shipped pass drew with {:+.5} ln. a pooled LUMA gain moves both \n\
         \tregions' channels together and must leave every split column above unmoved, \n\
         \twhich is what the two `band held off` / `as it draws` rows check.",
        tone.log_gain,
    );
    Ok(())
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
/// One drawn view, with the band's photometry held off or let run.
///
/// Lifted out of [`profile`] so that [`arm`] draws the very same picture
/// through the very same code: two modes reading one render is the point of
/// the arm-internal instrument, and a second copy of this block is a second
/// thing to get out of step.
fn drawn(
    options: &Options,
    gpu: &Gpu,
    size: Size,
    held: bool,
) -> Fallible<(Picture, Reframe, kjerag_render::Tone)> {
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
    scene.use_table(options.table);
    let mut shot = None;
    for _ in 0..options.count.max(1) {
        shot = Some(
            Render {
                gpu,
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
}

fn profile(options: &Options) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let out = options.out();
    std::fs::create_dir_all(&out)?;
    let size = Size::new(options.size, options.size);

    let (before, mapped, _) = drawn(options, &gpu, size, true)?;
    let (after, _, tone) = drawn(options, &gpu, size, false)?;

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
    println!("{}", Interior::header());
    match interior(&before, &after, &mapped, size, Plant::NOTHING) {
        Some(read) => print!("{}", read.report("what is drawn")),
        None => println!("  not enough of the interior is in this view"),
    }
    // Every plant runs against a picture with NOTHING applied, so what comes
    // back is the plant and not the plant plus a correction. A control that
    // has never been shown able to read a positive is a control that has
    // cleared nothing.
    for (label, plant) in [
        ("nothing applied at all", Plant::NOTHING),
        ("a 0.5 code LUMA ripple", Plant::luma(0.5)),
        ("a 2.0 code LUMA ripple", Plant::luma(2.0)),
        ("a 0.5 code CHROMA ripple", Plant::chroma(0.5)),
        ("a 2.0 code CHROMA ripple", Plant::chroma(2.0)),
    ] {
        if let Some(read) = interior(&before, &before, &mapped, size, plant) {
            print!("{}", read.report(&format!("{label}: {}", plant.name())),);
        }
    }
    println!(
        "  the two CHROMA rows are the finding: they carry zero luminance by construction, \n\
         \tso the luma row reads its own noise while R, G and B read the stripe. Until \n\
         \t2026-08-09 the luma row was the whole statistic."
    );
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
    let crossover = f64::from(reframe.handover_width().to_degrees()) / 2.0;
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
    // Eight and twelve degrees off, and not the six this used until
    // 2026-08-06: a decoy has to straddle content the handover never touched,
    // and the handover plus the bend it carries reaches 6.60 degrees now that
    // the crossover is 8 (`kjerag_render::band::reach`). A control inside the
    // thing it is a control for reads the artifact and subtracts it from the
    // excess line below.
    for centre in [-12.0, -8.0, 8.0, 12.0] {
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
///
/// **And until 2026-08-09 it read all of that in LUMA and nothing else**
/// (docs/research/chromatic.md 6.2). The applied field was reduced through
/// [`LUMA`] before it was binned, so a chroma-only stripe with no luminance
/// lift in it read exactly 0.00 percent, however violent it was, on the one
/// anti-acceptance metric this campaign's whole photometric line is gated on.
/// The chromatic line is the mechanism most able to produce exactly that
/// artifact, so extending this was a gate on that build and not a step inside
/// it. It reads four channels now: luma first, so every number already
/// published still compares, then R, G and B, each with its own plant.
///
/// What names the four is [`INTERIOR_CHANNELS`].
#[derive(Clone, Copy, Debug, Default)]
struct Coherence {
    /// The applied correction's mean size over the band, in codes.
    applied: f64,
    /// What is smooth round the ring, as Weber percent: the five-term fit.
    smooth: f64,
    /// What is NOT, as Weber percent. **The number.**
    rough: f64,
    /// The largest single step between neighbouring azimuth bins, Weber.
    step: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Interior {
    /// Luma, then R, G and B.
    read: [Coherence; 4],
    /// How many azimuth bins had any picture in them.
    bins: usize,
}

/// What [`Interior`]'s four readings are called, in order. Luma is first
/// because it is the one every earlier number in this campaign was.
const INTERIOR_CHANNELS: [&str; 4] = ["luma", "R", "G", "B"];

impl Interior {
    /// One row per channel under a label that is printed once, so four
    /// readings do not cost four labels.
    fn report(&self, label: &str) -> String {
        let mut out = String::new();
        for (index, name) in INTERIOR_CHANNELS.iter().enumerate() {
            let read = self.read[index];
            out.push_str(&format!(
                "  {:<40} {:>5} {:>10.3} {:>9.2} {:>9.2} {:>9.2}{}\n",
                match index {
                    0 => label,
                    _ => "",
                },
                name,
                read.applied,
                100.0 * read.smooth,
                100.0 * read.rough,
                100.0 * read.step,
                match index {
                    0 => format!("   ({} bins)", self.bins),
                    _ => String::new(),
                },
            ));
        }
        out
    }

    /// The header the rows above want over them.
    fn header() -> String {
        format!(
            "  {:<40} {:>5} {:>10} {:>9} {:>9} {:>9}",
            "field", "ch", "applied", "smooth %", "ROUGH %", "step %"
        )
    }
}

/// A known azimuthal ripple planted into the applied field before it is
/// measured, in codes, **per channel**: the positive control.
///
/// Per channel and not one number, and that is this memo's second finding
/// made runnable. A luma plant can only ever exercise a luma statistic. The
/// plant that decides whether this instrument can see the chromatic line's
/// own artifact is [`Plant::chroma`], which carries exactly zero luminance:
/// under the statistic as it stood it reads 0.00 percent however large it is,
/// and a chroma-only stripe round the ring is precisely what a per-channel
/// correction estimated per direction would paint.
#[derive(Clone, Copy, Default)]
struct Plant([f64; 3]);

impl Plant {
    const NOTHING: Self = Self([0.0; 3]);

    /// The same codes in all three channels: a luminance ripple, which is what
    /// this control has always planted. 0.5 and 2.0 read 2.07 and 8.27 percent
    /// on the rejected build's own view and those numbers still stand.
    fn luma(codes: f64) -> Self {
        Self([codes; 3])
    }

    /// A ripple on the green-magenta axis with **no luminance in it at all**:
    /// R and B up together, G down by exactly the BT.709 weight that cancels
    /// them. It is the oracle's own axis at the dirt end of the seam
    /// (docs/research/chromatic.md 2.2), and [`Plant::lifts`] is the equality
    /// that says it is neutral rather than nearly so.
    fn chroma(codes: f64) -> Self {
        Self([codes, -codes * (LUMA[0] + LUMA[2]) / LUMA[1], codes])
    }

    /// What this plant lifts the luma by, in codes. Zero for [`Self::chroma`]
    /// by construction, and printed rather than assumed.
    fn lifts(&self) -> f64 {
        LUMA.iter().zip(self.0).map(|(w, c)| w * c).sum()
    }

    fn name(&self) -> String {
        format!(
            "R {:+.2}, G {:+.2}, B {:+.2} codes (luma {:+.3})",
            self.0[0],
            self.0[1],
            self.0[2],
            self.lifts(),
        )
    }
}

/// The interior statistic over one pair of pictures, per channel.
///
/// `plant` plants a known azimuthal ripple into the applied field before it is
/// measured, which is the positive control: a correction that is smooth round
/// the ring plus a ripple has to read the ripple back **in the channels it was
/// planted in and in no others**, and a run with no ripple and no correction
/// has to read zero in all four.
fn interior(
    before: &Picture,
    after: &Picture,
    reframe: &Reframe,
    size: Size,
    plant: Plant,
) -> Option<Interior> {
    let width = size.width as usize;
    // Sums per azimuth bin: the applied correction and the level it sits on,
    // each in four channels, and how many pixels answered.
    let mut held = vec![([0.0f64; 4], [0.0f64; 4], 0.0f64); SWEEP];
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
        // What was applied and what it was applied to, in four channels: the
        // BT.709 luma first, so the number this campaign has published all
        // along is still index 0, then R, G and B themselves. Dark content is
        // where an additive correction is a large ratio and where the owner is
        // looking, and the level in the denominator is what makes this a Weber
        // number rather than a count of codes.
        let (mut lift, mut level) = ([0.0f64; 4], [0.0f64; 4]);
        for channel in 0..3 {
            let a = f64::from(before.rgba[4 * index + channel]);
            let b = f64::from(after.rgba[4 * index + channel]);
            lift[0] += LUMA[channel] * (b - a);
            level[0] += LUMA[channel] * a;
            lift[1 + channel] = b - a;
            level[1 + channel] = a;
        }
        // The gate is the LUMA level, unchanged, so which pixels this reads is
        // exactly the set it always read and the four numbers are four
        // statistics over one population rather than four populations.
        if level[0] <= 0.0 || level[0] > DARK {
            continue;
        }
        let wave = (8.0 * phi).cos();
        let planted = [
            plant.lifts() * wave,
            plant.0[0] * wave,
            plant.0[1] * wave,
            plant.0[2] * wave,
        ];
        // The FIELD in the numerator and the content in the denominator, each
        // averaged over the bin before they are divided. Dividing per pixel
        // instead puts the content's own roughness into the numerator, and the
        // statistic then reads the soil rather than the correction painted over
        // it - measured: it reported 0.89 percent of roughness for a field that
        // is smooth by construction.
        for channel in 0..4 {
            held[bin].0[channel] += lift[channel] + planted[channel];
            held[bin].1[channel] += level[channel].max(0.0);
        }
        held[bin].2 += 1.0;
    }
    let seen: Vec<(f64, [f64; 4], [f64; 4])> = held
        .iter()
        .enumerate()
        .filter(|(_, bin)| bin.2 > 16.0)
        .map(|(index, bin)| {
            (
                index as f64 / SWEEP as f64 * std::f64::consts::TAU,
                std::array::from_fn(|channel| bin.0[channel] / bin.2),
                std::array::from_fn(|channel| bin.1[channel] / bin.2),
            )
        })
        .collect();
    if seen.len() < 16 {
        return None;
    }
    let count = seen.len() as f64;
    let read = std::array::from_fn(|channel| {
        // The five terms a field CAN have and stay smooth: a constant, one
        // cycle and two. The same basis the geometry is fitted through, and the
        // same one stage 7's colour field used. Anything outside it is a
        // stripe.
        let mut normal = [[0.0f64; 5]; 5];
        let mut right = [0.0f64; 5];
        for (phi, codes, _) in &seen {
            let held = basis(*phi);
            for row in 0..5 {
                for column in 0..5 {
                    normal[row][column] += held[row] * held[column];
                }
                right[row] += held[row] * codes[channel];
            }
        }
        let fitted = solve5(normal, right);
        let smooth_at = |phi: f64| -> f64 {
            basis(phi)
                .iter()
                .zip(fitted)
                .map(|(term, coefficient)| term * coefficient)
                .sum()
        };
        let mut applied = 0.0;
        let mut smooth = 0.0;
        let mut rough = 0.0;
        for (phi, codes, level) in &seen {
            let level = level[channel].max(f64::MIN_POSITIVE);
            applied += codes[channel].abs();
            smooth += (smooth_at(*phi) / level).powi(2);
            rough += ((codes[channel] - smooth_at(*phi)) / level).powi(2);
        }
        let mut step: f64 = 0.0;
        for pair in seen.windows(2) {
            let level = 0.5 * (pair[0].2[channel] + pair[1].2[channel]);
            if level > 0.0 {
                step = step.max((pair[1].1[channel] - pair[0].1[channel]).abs() / level);
            }
        }
        Coherence {
            applied: applied / count,
            smooth: (smooth / count).sqrt(),
            rough: (rough / count).sqrt(),
            step,
        }
    });
    Some(Interior {
        read,
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
    scene.use_table(options.table);
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
    /// The along-seam table the picture is drawn with (issue #103, stage 9).
    /// `Table::REST` unless a run names one, so a run that does not is the
    /// picture this instrument has always measured, byte for byte.
    table: kjerag_render::Table,
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
            table: kjerag_render::Table::REST,
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
                        "chroma" => Mode::Chroma,
                        "arm" => Mode::Arm,
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
                Some(("table", value)) => options.table = kjerag_spike::seam_table(value)?,
                // Two paths only, and anything else is refused rather than
                // read as one of them: `value != "factory"` took `seam=pool`
                // and every typo for `file` and drew a pose nobody asked for.
                Some(("seam", value)) => {
                    options.fit = match value {
                        "factory" => false,
                        "file" => true,
                        _ => {
                            return Err(format!(
                                "this instrument fits the file or leaves the factory \
                                 calibration alone: seam=file or seam=factory, not {value}"
                            )
                            .into());
                        }
                    }
                }
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

const USAGE: &str = "usage: colour <file.insv|export.mp4> \
     [mode=field|chroma|arm|profile|studio|trace] \
     [from=seconds] [count=frames] [places=n] [patches=n] [keep=r] [seam=file|factory] [verbose=1] \
     [yaw=deg] [pitch=deg] [fov=deg] [size=px] [lock=0] [out=dir] [tag=name] [reach=deg] \
     [rows=lo:hi] [cols=lo:hi] [box=left:top:right:bottom] [table=table.txt]";
