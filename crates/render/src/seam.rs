//! What one file's two lenses disagree by at the seam, and the correction
//! that flattens it (issue #48).
//!
//! The camera's own calibration is out by degrees on the unit the owner
//! flies: measured round the whole seam circle on a capture from a camera
//! that was not moving, the two lenses' pictures of the same directions sit
//! -2.4 to +2.7 degrees apart across the seam, which is 43 px of the
//! delivered frame and is what draws a tree trunk twice. The structure of it
//! round the circle names a relative **lens tilt**, and a static correction to
//! lens 1's block of the calibration takes it out. Method, controls and every
//! number: docs/research/insv-format.md 6.8.
//!
//! **The correction belongs to the camera, not to the file** (6.8). One
//! five-knob answer fitted on a capture from a camera that was **not moving**
//! goes into five flights spanning three and a half months, and re-read on
//! the pixels with it applied it takes their seams from 0.74 to 0.96 degrees
//! along and 1.69 to 2.18 across down to 0.12 to 0.36 and 0.57 to 0.84.
//!
//! A fit off a flight scores better on that flight's own readings, and it is
//! still not a calibration, because it does not agree with itself. Fitted
//! file by file the same glued pair of lenses asks for yaws from -1.69 to
//! -2.58 degrees and principal points from -1.3 to -9.5 px: the yaw alone
//! spans 0.9 degrees, which at the seam is 15 px of a 1920-wide 90-degree
//! view. What moves between the files is the scene. Each capture's own
//! readings put the still one's content at 580 m and the flights' at 2.6 to
//! 4.2 m, and a fit taken through a seam that close absorbs the parallax into
//! its answer and then applies it to the whole sphere.
//!
//! So the fit runs once, against a capture the pilot points it at, and the
//! answer is stored under [`CalibrationSet::camera_key`], which is the model
//! and the factory calibration and is not the serial. Nothing is decoded at
//! open and nothing lands later: five stored numbers are in the first frame.
//!
//! A file whose camera has no stored calibration is still corrected, from its
//! own frames, best effort ([`Scene::fit_seam`](crate::Scene::fit_seam)).
//! That is the weaker path and it says so in the line it prints.
//!
//! The fit is phase 1's own measurement, in the shipped map's units. Both
//! lenses are sampled on the **same angular grid** around directions on the
//! seam circle, so what best correlates between them is a disagreement in
//! degrees of world angle with no rotation to undo; each calibration field is
//! then turned by a probe amount and the map is asked what that does to the
//! same patches, which is a column of the design matrix in the units
//! `offset_v3` writes. `kyerag-spike --bin seam` is the same core with the
//! attribution, the harmonics and the controls printed round it.
//!
//! Nothing here decides what a picture looks like: the answer is a patch to a
//! [`Lens`], and the pass runs on the patched calibration exactly as it ran on
//! the factory one.

use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use kyerag_media::{Fallible, Plane, Walk};
use kyerag_meta::Lens;

use super::projection::{Held, Reframe};
use super::sampling::Sampling;
use super::{Camera, Size};

/// The knobs the shipped fit turns: a relative rotation, and the principal
/// point that reaches the one term a rotation cannot.
///
/// A rotation alone leaves about 0.4 degrees of **along-seam one cycle**,
/// which is a principal-point signature and nothing else: a lens tilt reaches
/// the across-seam column at 1.0000 degrees per degree and the along-seam one
/// at 0.0000, while a principal-point shift reaches both. Fitting the pair
/// takes the along-seam residual on the owner's static capture from 0.384
/// degrees to 0.022 and on his five flights from 0.35-0.49 to 0.11-0.42,
/// while the across-seam column does not move at all (6.8).
///
/// The reason three shipped first was that the pair runs away on a file whose
/// seam has little far-field content: five knobs on the owner's seven
/// near-field deck patches ask for a -55 px principal point and a yaw of the
/// opposite sign to every other capture from that camera. [`RIDGE`] and
/// [`PATCHES_NEEDED`] are what make five safe, and neither is a guess: both
/// are measured in 6.8.
pub const KNOBS: [Knob; 5] = [Knob::Roll, Knob::Yaw, Knob::Pitch, Knob::Cx, Knob::Cy];

/// How hard the principal point is held towards zero, in degrees of penalty
/// per pixel of shift.
///
/// The data's own weight on the principal point is its leverage squared times
/// the patch count, which at 0.032 degrees per pixel over fifty patches is
/// 0.05: a ridge above about 0.2 has already won and one below about 0.02
/// does nothing. Scanned over 0.05, 0.10 and 0.20 on the owner's captures
/// (6.8). At 0.05 the thin deck capture's runaway is pulled from -55 px to
/// -21 while the static capture's own answer moves 1.6 px, from -4.19 to
/// -2.55, and its along-seam residual improves from 0.030 to 0.022. It is a
/// prior, not a limit: a capture with content round the whole circle
/// overrules it, and it is not the guard either. [`PATCHES_NEEDED`] is what
/// refuses the deck capture outright.
pub const RIDGE: f64 = 0.05;

/// The widest correction that is a calibration rather than a fit running
/// away, in degrees of relative rotation.
///
/// The error being corrected is a factory extrinsic, and the two captures
/// that pinned it read 2.44 degrees of tilt against sub-degree recorded
/// extrinsics. Ten degrees is four times that and still nowhere near what a
/// correlation locked onto the wrong content would produce; past it the file
/// keeps the calibration the camera wrote.
const RUNAWAY_DEG: f64 = 10.0;

/// How many azimuths have to correlate before a fit is believed: twice the
/// knob count.
///
/// The two captures that fall below it are the two that misbehave. The
/// owner's deck capture correlates 7 azimuths of near-field decking, and the
/// five knobs on those ask for a -55 px principal point and a yaw of the
/// opposite sign to every other capture from the same camera; a ONE X2 clip
/// correlates 3, on which the five are singular outright. Neither is short,
/// both are wrong, and a count catches them before the numbers do.
const PATCHES_NEEDED: usize = 2 * KNOBS.len();

// ------------------------------------------------------------ the ring

/// One direction on the seam great circle, and the two axes of the sphere
/// there: along the circle towards increasing azimuth, and across it towards
/// the front lens.
#[derive(Clone, Copy)]
pub struct Where {
    pub phi: f64,
    pub centre: [f64; 3],
    pub along: [f64; 3],
    pub across: [f64; 3],
}

/// `patches` directions evenly round the seam circle, from the body's +x.
pub fn ring(patches: usize) -> Vec<Where> {
    (0..patches)
        .map(|index| {
            let phi = index as f64 / patches as f64 * std::f64::consts::TAU;
            let (sin, cos) = phi.sin_cos();
            Where {
                phi,
                centre: [cos, sin, 0.0],
                along: [-sin, cos, 0.0],
                across: [0.0, 0.0, 1.0],
            }
        })
        .collect()
}

// ------------------------------------------------------------ the patches

/// How finely the seam is read: the sampling grid, the search, and what is
/// too flat or too unlike to keep.
///
/// The defaults are the numbers 6.8's measurement was taken with. The search
/// is wider across the seam than along it because parallax only reaches one
/// of them, and reaches it by degrees in the near field.
#[derive(Clone, Copy, Debug)]
pub struct Probe {
    pub patches: usize,
    /// How wide a patch is, in degrees of world angle.
    pub span: f64,
    /// How finely the correlation is stepped, in degrees.
    pub step: f64,
    pub along: f64,
    pub across: f64,
    /// The correlation a patch has to reach to be kept.
    pub keep: f64,
    /// How much picture a patch needs, in 8-bit codes of standard deviation.
    /// Flat sky correlates with anything.
    pub contrast: f64,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            patches: 72,
            span: 3.7,
            step: 0.08,
            along: 2.0,
            across: 4.0,
            keep: 0.80,
            contrast: 6.0,
        }
    }
}

/// One lens's picture of a rectangle of the sphere, sampled on a grid of
/// **directions** rather than of pixels: `2 * along + 1` by `2 * across + 1`,
/// `step` radians apart, laid out along then across.
struct Grid {
    along: isize,
    across: isize,
    luma: Vec<f64>,
}

impl Grid {
    fn at(&self, i: isize, j: isize) -> f64 {
        self.luma[((i + self.along) * (2 * self.across + 1) + (j + self.across)) as usize]
    }

    /// How much picture there is to correlate, in 8-bit codes.
    fn contrast(&self) -> f64 {
        let count = self.luma.len() as f64;
        let mean = self.luma.iter().sum::<f64>() / count;
        (self.luma.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count).sqrt()
    }

    /// Zero-mean normalized cross-correlation against `other` shifted by
    /// `(di, dj)`, over every `stride`-th sample of this grid.
    fn correlation(&self, other: &Grid, di: isize, dj: isize, stride: isize) -> f64 {
        let (mut sum_a, mut sum_b, mut count) = (0.0, 0.0, 0.0);
        let mut pairs: Vec<(f64, f64)> = Vec::with_capacity(self.luma.len());
        let mut i = -self.along;
        while i <= self.along {
            let mut j = -self.across;
            while j <= self.across {
                let (a, b) = (self.at(i, j), other.at(i + di, j + dj));
                sum_a += a;
                sum_b += b;
                count += 1.0;
                pairs.push((a, b));
                j += stride;
            }
            i += stride;
        }
        let (mean_a, mean_b) = (sum_a / count, sum_b / count);
        let (mut covariance, mut var_a, mut var_b) = (0.0, 0.0, 0.0);
        for (a, b) in pairs {
            let (a, b) = (a - mean_a, b - mean_b);
            covariance += a * b;
            var_a += a * a;
            var_b += b * b;
        }
        match var_a > 0.0 && var_b > 0.0 {
            true => covariance / (var_a * var_b).sqrt(),
            false => 0.0,
        }
    }
}

/// One lens's picture of the sphere around `at`. `None` where any corner of
/// the rectangle is outside this lens's picture: the two lenses have to be
/// answering about the same directions or the correlation means nothing.
fn sample(
    reframe: &Reframe,
    plane: &Plane,
    lens: usize,
    at: &Where,
    half: (isize, isize),
    step: f64,
) -> Option<Grid> {
    let mut luma = Vec::with_capacity(((2 * half.0 + 1) * (2 * half.1 + 1)) as usize);
    for i in -half.0..=half.0 {
        for j in -half.1..=half.1 {
            let (a, b) = (i as f64 * step, j as f64 * step);
            let ray = unit(std::array::from_fn(|axis| {
                at.centre[axis] + at.along[axis] * a + at.across[axis] * b
            }));
            let landing = reframe.project(lens, ray.map(|c| c as f32));
            if !landing.inside {
                return None;
            }
            luma.push(plane.at(f64::from(landing.pixel[0]), f64::from(landing.pixel[1]))?);
        }
    }
    Some(Grid {
        along: half.0,
        across: half.1,
        luma,
    })
}

/// The shift, in grid steps, that lines `back`'s picture up with `front`'s,
/// and how well it correlates there.
///
/// Coarse then fine then parabolic. The coarse pass strides both the search
/// and the samples it scores on, which is what makes an across-seam search
/// wide enough to hold near-field parallax affordable; the fine pass is every
/// step within one coarse cell of the winner; and the peak is then
/// interpolated between whole steps, because a residual of a third of a step
/// is exactly the size this is trying to resolve.
fn best_shift(front: &Grid, back: &Grid, search: (isize, isize)) -> Option<(f64, f64, f64)> {
    let stride = (search.0.max(search.1) / 12).max(1);
    // How far apart the shifts are tried and how far apart the samples are
    // scored are two different strides, and tying them together is how a
    // coarse pass over a wide search ends up correlating sixteen pixels
    // against sixteen pixels and finding a peak in the noise.
    let coarse = stride.min(3);
    let score = |di, dj, stride| front.correlation(back, di, dj, stride);
    let mut best: Option<(isize, isize, f64)> = None;
    let mut di = -search.0;
    while di <= search.0 {
        let mut dj = -search.1;
        while dj <= search.1 {
            let r = score(di, dj, coarse);
            if best.is_none_or(|(_, _, held)| r > held) {
                best = Some((di, dj, r));
            }
            dj += stride;
        }
        di += stride;
    }
    let (coarse_i, coarse_j, _) = best?;
    let mut best: Option<(isize, isize, f64)> = None;
    for di in (coarse_i - stride).max(-search.0)..=(coarse_i + stride).min(search.0) {
        for dj in (coarse_j - stride).max(-search.1)..=(coarse_j + stride).min(search.1) {
            let r = score(di, dj, 1);
            if best.is_none_or(|(_, _, held)| r > held) {
                best = Some((di, dj, r));
            }
        }
    }
    let (i, j, r) = best?;
    let peak = |minus: f64, here: f64, plus: f64| {
        let curve = minus - 2.0 * here + plus;
        match curve < 0.0 {
            true => (0.5 * (minus - plus) / curve).clamp(-1.0, 1.0),
            false => 0.0,
        }
    };
    let (mut refined_i, mut refined_j) = (0.0, 0.0);
    if i.abs() < search.0 {
        refined_i = peak(score(i - 1, j, 1), r, score(i + 1, j, 1));
    }
    if j.abs() < search.1 {
        refined_j = peak(score(i, j - 1, 1), r, score(i, j + 1, 1));
    }
    Some((i as f64 + refined_i, j as f64 + refined_j, r))
}

/// What one patch's correlation found: where lens 1's picture of the same
/// directions sits relative to lens 0's, in degrees of world angle.
#[derive(Clone, Copy)]
pub struct Found {
    pub along: f64,
    pub across: f64,
    pub r: f64,
    pub contrast: f64,
}

/// Why a patch was not a patch, which on a ring that crosses a deck, a
/// treeline and a blank sky is most of them and is worth saying out loud.
#[derive(Clone, Copy, Debug, Default)]
pub struct Refused {
    /// One of the two lenses has no picture of the whole rectangle, which
    /// past about 6 degrees off the seam is every patch: the overlap band is
    /// only so wide, so near-field content that parallax has moved further
    /// than that is not in both pictures at all and no instrument can pair it.
    pub outside: usize,
    pub flat: usize,
    pub unlike: usize,
    pub pinned: usize,
}

/// Every patch round the seam of one frame, under one calibration, in patch
/// order. `None` where a lens has no usable picture of that patch, or where
/// there is nothing in it to correlate.
pub fn read_ring(
    reframe: &Reframe,
    planes: &[Plane],
    ring: &[Where],
    probe: &Probe,
    refused: &mut Refused,
) -> Vec<Option<Found>> {
    let step = probe.step.to_radians();
    let half = (probe.span.to_radians() / 2.0 / step) as isize;
    let search = (
        (probe.along / probe.step) as isize,
        (probe.across / probe.step) as isize,
    );
    ring.iter()
        .map(|at| {
            let Some(front) = sample(reframe, planes.first()?, 0, at, (half, half), step) else {
                refused.outside += 1;
                return None;
            };
            if front.contrast() < probe.contrast {
                refused.flat += 1;
                return None;
            }
            let Some(back) = sample(
                reframe,
                planes.get(1)?,
                1,
                at,
                (half + search.0, half + search.1),
                step,
            ) else {
                refused.outside += 1;
                return None;
            };
            let (along, across, r) = best_shift(&front, &back, search)?;
            if r < probe.keep {
                refused.unlike += 1;
            }
            // A peak against the edge of the search is not a peak, it is the
            // search running out. Near-field content at this seam moves
            // further across than the overlap band is wide, and a reading
            // pinned at the limit would report the limit.
            if along.abs() >= search.0 as f64 || across.abs() >= search.1 as f64 {
                refused.pinned += 1;
                return None;
            }
            Some(Found {
                along: (along * step).to_degrees(),
                across: (across * step).to_degrees(),
                r,
                contrast: front.contrast(),
            })
        })
        .collect()
}

// ------------------------------------------------------------ the knobs

/// One field of one lens's calibration, in the units `offset_v3` writes it in.
///
/// Everything here is applied to **lens 1**, because the seam sees only the
/// two lenses' disagreement and cannot say which of them is wrong: a
/// correction of `+x` on lens 1 and one of `-x` on lens 0 are the same picture
/// at the seam. Reported that way round, a fitted number is a patch to lens
/// 1's block of the string the camera wrote.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Knob {
    Roll,
    Yaw,
    Pitch,
    Cx,
    Cy,
    Fx,
    Fy,
    Xi,
}

impl Knob {
    pub const ALL: [Self; 8] = [
        Self::Roll,
        Self::Yaw,
        Self::Pitch,
        Self::Cx,
        Self::Cy,
        Self::Fx,
        Self::Fy,
        Self::Xi,
    ];

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|knob| knob.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Roll => "roll",
            Self::Yaw => "yaw",
            Self::Pitch => "pitch",
            Self::Cx => "cx",
            Self::Cy => "cy",
            Self::Fx => "fx",
            Self::Fy => "fy",
            Self::Xi => "xi",
        }
    }

    /// What one unit of this knob is. The two focal lengths are fractional
    /// because a focal error is a scale and quoting it in pixels hides that
    /// 1 px of 3666 is 0.016 degrees at the rim.
    pub fn unit(self) -> &'static str {
        match self {
            Self::Roll | Self::Yaw | Self::Pitch => "deg",
            Self::Cx | Self::Cy => "px",
            Self::Fx | Self::Fy => "ratio",
            Self::Xi => "abs",
        }
    }

    /// The step the Jacobian is taken over, in this knob's own units. Large
    /// enough that the projection's f32 arithmetic is not what is being
    /// measured, small enough to stay linear at a tenth of a degree.
    pub fn probe(self) -> f64 {
        match self {
            Self::Roll | Self::Yaw | Self::Pitch => 0.10,
            Self::Cx | Self::Cy => 4.0,
            Self::Fx | Self::Fy => 0.001,
            Self::Xi => 0.005,
        }
    }

    pub fn apply(self, lens: &mut Lens, amount: f64) {
        match self {
            Self::Roll => lens.pose.roll_deg += amount,
            Self::Yaw => lens.pose.yaw_deg += amount,
            Self::Pitch => lens.pose.pitch_deg += amount,
            Self::Cx => lens.intrinsics.cx += amount,
            Self::Cy => lens.intrinsics.cy += amount,
            Self::Fx => lens.intrinsics.fx *= 1.0 + amount,
            Self::Fy => lens.intrinsics.fy *= 1.0 + amount,
            Self::Xi => lens.intrinsics.xi += amount,
        }
    }
}

/// A calibration with one knob turned on lens 1.
pub fn turned(lenses: &[Lens], knob: Knob, amount: f64) -> Vec<Lens> {
    let mut lenses = lenses.to_vec();
    if let Some(lens) = lenses.get_mut(1) {
        knob.apply(lens, amount);
    }
    lenses
}

/// The map for one calibration: the camera left alone and the horizon
/// unlocked, so a view ray is a direction in the camera body's own frame and a
/// patch of the sphere is addressed by its angles.
///
/// No rolling shutter in it either ([`Held::default`]), and that is a
/// measurement rather than an omission: an X4 reads down the delivered frame,
/// which is the same world direction in both lenses, so its contribution at
/// the seam is 0.000 degrees (docs/research/insv-format.md 6.7). The
/// correction fitted here composes with the readout correction at draw time;
/// neither can see the other.
pub fn mapped(lenses: &[Lens], frame: Size) -> Reframe {
    Reframe::new(
        lenses,
        frame,
        Camera::default(),
        Held::default(),
        1.0,
        false,
        Sampling::default(),
    )
}

/// How far a change to the calibration moves lens `lens`'s picture of one
/// direction, in degrees along and across the seam.
///
/// This is the model's own prediction of what the correlation will read, and
/// it is where the fit and the control both come from. If the change moves the
/// projection of a fixed direction by `dg` pixels, then the content that used
/// to correlate at shift `s` now correlates at `s - J^-1 dg`, where `J` is the
/// local Jacobian of the unchanged map from the two sphere axes to pixels.
pub fn moved(base: &Reframe, tweaked: &Reframe, lens: usize, at: &Where) -> Option<[f64; 2]> {
    let here = base.project(lens, at.centre.map(|c| c as f32));
    let there = tweaked.project(lens, at.centre.map(|c| c as f32));
    if !here.inside || !there.inside {
        return None;
    }
    // One hundredth of a degree, which is a sixth of a pixel at the rim: far
    // enough out of the f32 noise, near enough that the map is a plane.
    let probe = 0.01f64.to_radians();
    let column = |axis: [f64; 3]| {
        let step = |sign: f64| {
            let ray = unit(std::array::from_fn(|c| {
                at.centre[c] + sign * probe * axis[c]
            }));
            base.project(lens, ray.map(|c| c as f32)).pixel
        };
        let (plus, minus) = (step(1.0), step(-1.0));
        [
            f64::from(plus[0] - minus[0]) / (2.0 * probe),
            f64::from(plus[1] - minus[1]) / (2.0 * probe),
        ]
    };
    let (a, b) = (column(at.along), column(at.across));
    let determinant = a[0] * b[1] - a[1] * b[0];
    if determinant.abs() < 1e-9 {
        return None;
    }
    let d = [
        f64::from(there.pixel[0] - here.pixel[0]),
        f64::from(there.pixel[1] - here.pixel[1]),
    ];
    // -J^-1 d, in degrees.
    Some(
        [
            -(b[1] * d[0] - b[0] * d[1]) / determinant,
            -(a[0] * d[1] - a[1] * d[0]) / determinant,
        ]
        .map(f64::to_degrees),
    )
}

// ------------------------------------------------------------ the fit

/// One patch's reading, averaged over the frames it correlated on.
#[derive(Clone, Copy)]
pub struct Reading {
    pub at: Where,
    /// Degrees of world angle, lens 1's picture against lens 0's.
    pub along: f64,
    pub across: f64,
}

/// A correction to lens 1's calibration, in the units `offset_v3` writes.
///
/// Five fields, which is what the shipped fit turns ([`KNOBS`]) and what is
/// stored per camera. The instrument runs the same fitter with fewer, which
/// is how the choice between them was made and how it can be re-made.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeamFit {
    pub roll_deg: f64,
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    pub cx_px: f64,
    pub cy_px: f64,
}

impl SeamFit {
    /// The calibration with this correction on lens 1. A one-lens file is
    /// handed back unchanged: it has no seam to correct.
    pub fn applied(&self, lenses: &[Lens]) -> Vec<Lens> {
        let mut lenses = lenses.to_vec();
        if let Some(lens) = lenses.get_mut(1) {
            lens.pose.roll_deg += self.roll_deg;
            lens.pose.yaw_deg += self.yaw_deg;
            lens.pose.pitch_deg += self.pitch_deg;
            lens.intrinsics.cx += self.cx_px;
            lens.intrinsics.cy += self.cy_px;
        }
        lenses
    }

    /// How big a turn this is, in degrees: the length of the rotation it
    /// applies, which is what [`RUNAWAY_DEG`] bounds.
    pub fn turn_deg(&self) -> f64 {
        norm([self.roll_deg, self.yaw_deg, self.pitch_deg])
    }

    /// One more round's step on top of this one. The three numbers are the
    /// calibration's own fields, so a second correction to them adds; nothing
    /// here is composing two rotations.
    fn plus(self, step: Self) -> Self {
        Self {
            roll_deg: self.roll_deg + step.roll_deg,
            yaw_deg: self.yaw_deg + step.yaw_deg,
            pitch_deg: self.pitch_deg + step.pitch_deg,
            cx_px: self.cx_px + step.cx_px,
            cy_px: self.cy_px + step.cy_px,
        }
    }

    fn of(knobs: &[Knob], params: &[f64]) -> Self {
        let mut fit = Self::default();
        for (knob, amount) in knobs.iter().zip(params) {
            match knob {
                Knob::Roll => fit.roll_deg = *amount,
                Knob::Yaw => fit.yaw_deg = *amount,
                Knob::Pitch => fit.pitch_deg = *amount,
                Knob::Cx => fit.cx_px = *amount,
                Knob::Cy => fit.cy_px = *amount,
                // The focal lengths and the mirror parameter reach 300
                // percent corrections when they are let into this fit (6.8),
                // so nothing turns them and no field carries them.
                Knob::Fx | Knob::Fy | Knob::Xi => {}
            }
        }
        fit
    }
}

/// What a fit came to and how well it holds.
pub struct Fitted {
    pub fit: SeamFit,
    /// Root mean square of the readings before the correction and after it,
    /// in degrees: along the seam first, across it second.
    pub before: [f64; 2],
    pub after: [f64; 2],
    pub patches: usize,
}

impl Fitted {
    /// What this fit was measured on and what it left, for the report line.
    /// The seconds are the caller's: nearly all of a fit is decode, and only
    /// the caller knows when it started reading.
    pub fn describe(&self, seconds: f64) -> String {
        format!(
            "{} patches, {:.3} -> {:.3} along and {:.3} -> {:.3} across the seam, {seconds:.1} s",
            self.patches, self.before[0], self.after[0], self.before[1], self.after[1],
        )
    }
}

/// How many times the fit is re-linearized about its own answer.
///
/// One round is what 6.8 reports, and one round is 2 percent short at the
/// size being corrected: the design matrix is the map's Jacobian at a tenth
/// of a degree and the answer is two and a half degrees away, where the map
/// is no longer the plane a linear fit takes it for. Each round after the
/// first takes the same readings and asks the same question at the point the
/// round before reached, which lands inside a thousandth
/// (`the_fit_converges_on_the_error_it_was_given`).
const ROUNDS: usize = 3;

/// The correction that would flatten these readings, fitted through the map,
/// with nothing held towards zero.
///
/// This is the fitter an instrument reaches for, which is why it takes its
/// knobs and no ridge: what the shipped fit does to a set of readings is
/// [`fit_held`] with [`KNOBS`] and [`RIDGE`], and an instrument that wants to
/// compare that against something else needs the unregularized version to
/// compare it with.
pub fn fit(readings: &[Reading], lenses: &[Lens], frame: Size, knobs: &[Knob]) -> Option<Fitted> {
    fit_held(readings, lenses, frame, knobs, 0.0)
}

/// The same fit with the principal point held towards zero by `ridge`
/// degrees of penalty per pixel (see [`RIDGE`]).
///
/// Each knob is turned by its own probe amount and the map is asked what that
/// does to every patch, which is a column of the design matrix in the units
/// `offset_v3` writes. **Both** axes are in the fit: the across-seam column
/// carries parallax as well as calibration, and on far-field content that is
/// a tenth of a degree against the degrees being corrected (6.8), while the
/// tilt this is chasing reaches across the seam and barely reaches along it.
pub fn fit_held(
    readings: &[Reading],
    lenses: &[Lens],
    frame: Size,
    knobs: &[Knob],
    ridge: f64,
) -> Option<Fitted> {
    let base = mapped(lenses, frame);
    let mut fit = SeamFit::default();
    let mut patches = 0;
    for _ in 0..ROUNDS {
        let so_far = fit.applied(lenses);
        let here = mapped(&so_far, frame);
        // What the same patches would read with the correction so far in
        // place, which is what the next round is fitted to.
        let left: Vec<Reading> = readings
            .iter()
            .filter_map(|reading| {
                let shift = moved(&base, &here, 1, &reading.at)?;
                Some(Reading {
                    at: reading.at,
                    along: reading.along + shift[0],
                    across: reading.across + shift[1],
                })
            })
            .collect();
        let (step, kept) = round(&left, &so_far, frame, knobs, ridge)?;
        fit = fit.plus(step);
        patches = kept;
    }
    Some(Fitted {
        before: [
            rms(readings.iter().map(|r| r.along)),
            rms(readings.iter().map(|r| r.across)),
        ],
        after: residual(readings, &fit, lenses, frame),
        fit,
        patches,
    })
}

/// One linear round: the knobs that flatten these readings, taken about the
/// calibration they were read against.
fn round(
    readings: &[Reading],
    lenses: &[Lens],
    frame: Size,
    knobs: &[Knob],
    ridge: f64,
) -> Option<(SeamFit, usize)> {
    let base = mapped(lenses, frame);
    let probes: Vec<Reframe> = knobs
        .iter()
        .map(|knob| mapped(&turned(lenses, *knob, knob.probe()), frame))
        .collect();
    let mut rows: Vec<(Vec<f64>, f64)> = Vec::new();
    let mut kept = 0;
    for reading in readings {
        let mut along = Vec::with_capacity(knobs.len());
        let mut across = Vec::with_capacity(knobs.len());
        for (index, probe) in probes.iter().enumerate() {
            let Some(shift) = moved(&base, probe, 1, &reading.at) else {
                along.clear();
                break;
            };
            along.push(shift[0] / knobs[index].probe());
            across.push(shift[1] / knobs[index].probe());
        }
        if along.len() != knobs.len() {
            continue;
        }
        // The correction is what has to be ADDED to the calibration to bring
        // the disagreement to zero, so the target is the negative of it.
        rows.push((along, -reading.along));
        rows.push((across, -reading.across));
        kept += 1;
    }
    for (index, knob) in knobs.iter().enumerate() {
        if ridge <= 0.0 || matches!(knob, Knob::Roll | Knob::Yaw | Knob::Pitch) {
            continue;
        }
        // One row per held knob, asking it for zero. In units of the knob's
        // own probe step, so one ridge number means the same thing to a
        // principal point in pixels as it would to any other knob, and the
        // scale is the principal point's own probe, which is what the ridge
        // was scanned in.
        let mut basis = vec![0.0; knobs.len()];
        basis[index] = ridge * Knob::Cx.probe() / knob.probe();
        rows.push((basis, 0.0));
    }
    Some((SeamFit::of(knobs, &least_squares(&rows)?.params), kept))
}

/// What the readings would have been with the correction in place, predicted
/// through the map.
///
/// The honest check is to apply the correction and measure the pixels again,
/// which is what the instrument does and what 6.8 reports. This is the same
/// arithmetic the fit minimized, and it is here so a fit that did not improve
/// the thing it was fitted to can be thrown away without a second decode.
fn residual(readings: &[Reading], fit: &SeamFit, lenses: &[Lens], frame: Size) -> [f64; 2] {
    let base = mapped(lenses, frame);
    let corrected = mapped(&fit.applied(lenses), frame);
    let mut along = Vec::with_capacity(readings.len());
    let mut across = Vec::with_capacity(readings.len());
    for reading in readings {
        let Some(shift) = moved(&base, &corrected, 1, &reading.at) else {
            continue;
        };
        along.push(reading.along + shift[0]);
        across.push(reading.across + shift[1]);
    }
    [rms(along.iter().copied()), rms(across.iter().copied())]
}

// ------------------------------------------------------------ least squares

pub struct Fit {
    pub params: Vec<f64>,
    pub errors: Vec<f64>,
    pub residual: f64,
}

/// Ordinary least squares through the normal equations, with the standard
/// error of each parameter. Small systems only, which every fit here is.
pub fn least_squares(rows: &[(Vec<f64>, f64)]) -> Option<Fit> {
    let width = rows.first()?.0.len();
    if rows.len() <= width {
        return None;
    }
    let mut normal = vec![vec![0.0; width]; width];
    let mut right = vec![0.0; width];
    for (basis, value) in rows {
        for i in 0..width {
            right[i] += basis[i] * value;
            for j in 0..width {
                normal[i][j] += basis[i] * basis[j];
            }
        }
    }
    let inverse = invert(&normal)?;
    let params: Vec<f64> = (0..width)
        .map(|i| (0..width).map(|j| inverse[i][j] * right[j]).sum())
        .collect();
    let residual = (rows
        .iter()
        .map(|(basis, value)| {
            let modelled: f64 = basis.iter().zip(&params).map(|(b, p)| b * p).sum();
            (value - modelled).powi(2)
        })
        .sum::<f64>()
        / (rows.len() - width) as f64)
        .sqrt();
    Some(Fit {
        errors: (0..width)
            .map(|i| residual * inverse[i][i].max(0.0).sqrt())
            .collect(),
        params,
        residual: residual * ((rows.len() - width) as f64 / rows.len() as f64).sqrt(),
    })
}

/// Gauss-Jordan with partial pivoting, or `None` where the system is singular.
fn invert(matrix: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let width = matrix.len();
    let mut work: Vec<Vec<f64>> = (0..width)
        .map(|i| {
            let mut row = matrix[i].clone();
            row.extend((0..width).map(|j| f64::from(u8::from(i == j))));
            row
        })
        .collect();
    for column in 0..width {
        let pivot = (column..width)
            .max_by(|a, b| work[*a][column].abs().total_cmp(&work[*b][column].abs()))?;
        if work[pivot][column].abs() < 1e-12 {
            return None;
        }
        work.swap(column, pivot);
        let divisor = work[column][column];
        for value in &mut work[column] {
            *value /= divisor;
        }
        for row in 0..width {
            if row == column {
                continue;
            }
            let factor = work[row][column];
            let pivot = work[column].clone();
            for (value, above) in work[row].iter_mut().zip(&pivot) {
                *value -= factor * above;
            }
        }
    }
    Some(work.into_iter().map(|row| row[width..].to_vec()).collect())
}

// ------------------------------------------------------------ the file

/// Where in the file the seam is read.
///
/// Spread rather than consecutive, because what one place has content at is
/// one set of azimuths and the tilt's axis is only pinned by covering the
/// circle (6.8, "the split of the tilt between yaw and pitch"). Frames at each
/// place because a seek costs a keyframe walk and the frames after it are
/// nearly free.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    pub places: usize,
    pub frames: usize,
    pub probe: Probe,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            places: 3,
            frames: 2,
            probe: Probe::default(),
        }
    }
}

/// Every patch this file's seam offers, pooled over the frames it was read on.
pub fn measure(path: &Path, lenses: &[Lens], frame: Size, plan: &Plan) -> Fallible<Vec<Reading>> {
    let base = mapped(lenses, frame);
    let ring = ring(plan.probe.patches);
    let mut walk = Walk::open(path, 0.0, frame)?;
    if walk.streams() < 2 {
        return Err("this file carries one lens stream, so it has no seam".into());
    }
    let duration = walk.duration().as_secs_f64();
    let mut sums: Vec<(usize, f64, f64)> = vec![(0, 0.0, 0.0); ring.len()];
    let mut refused = Refused::default();
    for place in 0..plan.places.max(1) {
        // Spread over the middle of the file: the first and last moments of a
        // flight are a camera on the ground with a hand in front of it.
        let at = duration * (place as f64 + 0.5) / plan.places.max(1) as f64;
        if place > 0 || at > 0.0 {
            walk.jump(at)?;
        }
        for _ in 0..plan.frames.max(1) {
            let Some(pair) = walk.next_pair()? else {
                break;
            };
            for (found, sum) in read_ring(&base, &pair.lenses, &ring, &plan.probe, &mut refused)
                .iter()
                .zip(&mut sums)
            {
                let Some(found) = found.filter(|found| found.r >= plan.probe.keep) else {
                    continue;
                };
                sum.0 += 1;
                sum.1 += found.along;
                sum.2 += found.across;
            }
        }
    }
    Ok(ring
        .iter()
        .zip(&sums)
        .filter(|(_, (frames, _, _))| *frames > 0)
        .map(|(at, (frames, along, across))| Reading {
            at: *at,
            along: along / *frames as f64,
            across: across / *frames as f64,
        })
        .collect())
}

/// One capture's correction, measured off its own frames, or why there is
/// none in words a pilot can read.
///
/// Every refusal is ordinary: a legacy camera that writes one lens per file, a
/// capture with no far-field content at the seam to correlate, a fit that came
/// out too big to be a calibration. Each of them leaves the factory
/// calibration in place, which is what the player did before this existed.
pub fn fit_capture(
    path: &Path,
    lenses: &[Lens],
    frame: Size,
    plan: &Plan,
) -> Result<Fitted, String> {
    let readings = measure(path, lenses, frame, plan).map_err(|e| e.to_string())?;
    if readings.len() < PATCHES_NEEDED {
        return Err(format!(
            "only {} of {} azimuths on the seam had content both lenses could be matched on, \
             which is too few to fit",
            readings.len(),
            plan.probe.patches,
        ));
    }
    let fitted = fit_held(&readings, lenses, frame, &KNOBS, RIDGE)
        .ok_or("the seam readings do not pin a correction")?;
    if fitted.fit.turn_deg() > RUNAWAY_DEG {
        return Err(format!(
            "the fit came to {:.1} deg of rotation, which is a fit running away rather than a \
             calibration",
            fitted.fit.turn_deg(),
        ));
    }
    // A correction is only a correction if it flattens what it was fitted to.
    if fitted.after[0] > fitted.before[0] || fitted.after[1] > fitted.before[1] {
        return Err("the fit does not flatten the seam".to_owned());
    }
    Ok(fitted)
}

/// The same fit, for a caller with a terminal rather than a toast: the reason
/// goes to stdout beside the rest of the app's report.
pub fn fit_file(path: &Path, lenses: &[Lens], frame: Size, plan: &Plan) -> Option<Fitted> {
    match fit_capture(path, lenses, frame, plan) {
        Ok(fitted) => Some(fitted),
        Err(why) => {
            println!("seam:   {why}; keeping the factory calibration");
            None
        }
    }
}

// ------------------------------------------------------------ at open

/// The lenses the pass runs on once a seam correction is in hand.
///
/// Empty until then, which is the factory calibration and is what the picture
/// was before this existed. This camera's stored calibration fills it at open,
/// before the first frame is drawn and with nothing to decode; the fallback
/// fills it from the fit's own thread, and the picture corrects itself
/// mid-playback.
pub type Corrected = Arc<OnceLock<Arc<[Lens]>>>;

/// Draw with this camera's stored calibration, from the first frame.
///
/// Nothing is decoded and no thread is started: the correction is five numbers
/// the pilot's config already holds, and applying them is arithmetic on the
/// calibration the trailer parsed.
pub fn known(into: &Corrected, lenses: &Arc<[Lens]>, fit: SeamFit) {
    if lenses.len() < 2 {
        return;
    }
    announce("this camera's calibration", &fit);
    let _ = into.set(fit.applied(lenses).into());
}

/// Fit this file's seam from its own frames, off the decode path.
///
/// The fallback, for a camera this box has no calibration for. The fit reads
/// its own frames through its own decoder, so nothing here waits on the player
/// and the player waits on nothing here; the file plays its first seconds on
/// the factory calibration and corrects itself when the fit lands, a second or
/// two in. What it is worth against a proper calibration, and what it costs,
/// are both in 6.8.
pub fn correct(into: &Corrected, path: &Path, lenses: &Arc<[Lens]>, frame: Size) {
    if lenses.len() < 2 {
        return;
    }
    best_effort_notice();
    let (path, lenses, into) = (path.to_path_buf(), lenses.clone(), into.clone());
    let spawned = std::thread::Builder::new()
        .name("seam fit".to_owned())
        .spawn(move || fit_into(&path, &lenses, frame, &into));
    if let Err(e) = spawned {
        eprintln!("kyerag: the seam fit did not start: {e}");
    }
}

/// The same fallback, on this thread.
///
/// What a **still** takes (issue #15, and every headless instrument): a
/// picture written to a file has no moment later to correct itself in.
pub fn correct_now(into: &Corrected, path: &Path, lenses: &Arc<[Lens]>, frame: Size) {
    if lenses.len() < 2 {
        return;
    }
    best_effort_notice();
    fit_into(path, lenses, frame, into);
}

fn best_effort_notice() {
    println!(
        "seam:   no calibration stored for this camera, so it is fitted from this file, best \
         effort. View > Calibrate seam from this video, on a capture from a camera standing \
         still, is the one that transfers."
    );
}

fn fit_into(path: &Path, lenses: &Arc<[Lens]>, frame: Size, into: &Corrected) {
    let started = Instant::now();
    let Some(fitted) = fit_file(path, lenses, frame, &Plan::default()) else {
        return;
    };
    announce(
        &fitted.describe(started.elapsed().as_secs_f64()),
        &fitted.fit,
    );
    let _ = into.set(fitted.fit.applied(lenses).into());
}

fn announce(how: &str, fit: &SeamFit) {
    println!(
        "seam:   lens 1 roll {:+.3}, yaw {:+.3}, pitch {:+.3} deg, cx {:+.2}, cy {:+.2} px ({how})",
        fit.roll_deg, fit.yaw_deg, fit.pitch_deg, fit.cx_px, fit.cy_px,
    );
}

// ------------------------------------------------------------ calibrating

/// One calibration run, taken off the open file so it can be handed to a
/// worker thread.
///
/// It carries the calibration and the frame size rather than the whole scene
/// because that is all a fit reads, and because the shell must not stop for
/// one: a fit is a second or two of decode, and a window that stops for two
/// seconds is a window that has stopped.
pub struct Job {
    lenses: Arc<[Lens]>,
    frame: Size,
}

impl Job {
    /// `None` for a file with no seam, which is the one thing that cannot be
    /// calibrated from.
    pub fn new(lenses: &Arc<[Lens]>, frame: Size) -> Option<Self> {
        (lenses.len() >= 2).then(|| Self {
            lenses: lenses.clone(),
            frame,
        })
    }

    /// Fit this capture's seam, blocking for as long as it takes to read it.
    ///
    /// The whole answer, and what it left, go to the terminal; what comes back
    /// is the five numbers to store and, on a refusal, the reason in words a
    /// toast can carry.
    pub fn run(&self, path: &Path) -> Result<SeamFit, String> {
        let started = Instant::now();
        let fitted = fit_capture(path, &self.lenses, self.frame, &Plan::default())?;
        announce(
            &format!(
                "calibrated from this video, {}",
                fitted.describe(started.elapsed().as_secs_f64())
            ),
            &fitted.fit,
        );
        Ok(fitted.fit)
    }
}

// ------------------------------------------------------------ arithmetic

pub fn rms(values: impl Iterator<Item = f64>) -> f64 {
    let values: Vec<f64> = values.collect();
    match values.is_empty() {
        true => 0.0,
        false => (values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64).sqrt(),
    }
}

fn norm(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

pub fn unit(v: [f64; 3]) -> [f64; 3] {
    let length = norm(v).max(f64::MIN_POSITIVE);
    v.map(|c| c / length)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::projection::tests::{FRAME, fixture_lenses};

    /// The seam as a capture with content all the way round it would read it,
    /// if the camera's calibration were wrong by `error` on lens 1.
    ///
    /// This is the injected control of issue #45 and of 6.8, with the pixels
    /// taken out: what the correlation would have found is exactly what the
    /// map says a calibration error does to the patches, which is the same
    /// prediction the controls on real footage were scored against. It tests
    /// what a synthetic picture could not, because a synthetic picture would
    /// be drawn through this same map: that the fit inverts the map's own
    /// Jacobian into the units `offset_v3` writes, sign and all.
    fn readings_for(error: SeamFit, lenses: &[Lens], patches: usize) -> Vec<Reading> {
        let base = mapped(lenses, FRAME);
        let wrong = mapped(&error.applied(lenses), FRAME);
        ring(patches)
            .into_iter()
            .filter_map(|at| {
                let shift = moved(&base, &wrong, 1, &at)?;
                Some(Reading {
                    at,
                    along: -shift[0],
                    across: -shift[1],
                })
            })
            .collect()
    }

    /// One linear round is 2 percent short at this size, which is what
    /// [`ROUNDS`] exists for: the rounds are counted here rather than assumed,
    /// against the same injection the test above reads back.
    #[test]
    fn the_fit_converges_on_the_error_it_was_given() {
        let lenses = fixture_lenses();
        let error = SeamFit {
            roll_deg: 0.801,
            yaw_deg: -2.293,
            pitch_deg: -0.817,
            ..SeamFit::default()
        };
        let readings = readings_for(error, &lenses, 72);
        let base = mapped(&lenses, FRAME);
        let mut fit = SeamFit::default();
        let mut left = Vec::new();
        for _ in 0..ROUNDS {
            let so_far = fit.applied(&lenses);
            let here = mapped(&so_far, FRAME);
            let readings: Vec<Reading> = readings
                .iter()
                .filter_map(|reading| {
                    let shift = moved(&base, &here, 1, &reading.at)?;
                    Some(Reading {
                        at: reading.at,
                        along: reading.along + shift[0],
                        across: reading.across + shift[1],
                    })
                })
                .collect();
            fit = fit.plus(round(&readings, &so_far, FRAME, &KNOBS, 0.0).unwrap().0);
            left.push(norm([
                fit.roll_deg - error.roll_deg,
                fit.yaw_deg - error.yaw_deg,
                fit.pitch_deg - error.pitch_deg,
            ]));
        }
        assert!(
            left[0] > 0.01,
            "one round already lands: {:.4} deg",
            left[0]
        );
        assert!(
            *left.last().unwrap() < 0.001,
            "three rounds leave {:.4} deg",
            left[2]
        );
    }

    #[test]
    fn an_injected_calibration_error_is_read_back_as_itself() {
        let lenses = fixture_lenses();
        // The size and shape 6.8 fits on the owner's camera: a couple of
        // degrees of tilt, a fraction of a degree of roll under it, and a
        // principal point ten pixels or so off centre.
        let error = SeamFit {
            roll_deg: 0.801,
            yaw_deg: -2.293,
            pitch_deg: -0.817,
            cx_px: -4.18,
            cy_px: -13.91,
        };
        let readings = readings_for(error, &lenses, 72);
        let fitted = fit(&readings, &lenses, FRAME, &KNOBS).unwrap();
        for (fitted, truth) in [
            (fitted.fit.roll_deg, error.roll_deg),
            (fitted.fit.yaw_deg, error.yaw_deg),
            (fitted.fit.pitch_deg, error.pitch_deg),
            (fitted.fit.cx_px, error.cx_px),
            (fitted.fit.cy_px, error.cy_px),
        ] {
            assert!(
                (fitted / truth - 1.0).abs() < 0.02,
                "read back {fitted:.4} of an injected {truth:.4}"
            );
        }
        assert!(
            fitted.after[1] < 0.05 * fitted.before[1],
            "across the seam: {:.3} before, {:.3} after",
            fitted.before[1],
            fitted.after[1],
        );
    }

    /// What the fifth and fourth knobs are for: a principal-point error is
    /// **one cycle round the seam along it**, which is the term a rotation
    /// cannot reach at all (6.8). Fitted with three knobs it survives; fitted
    /// with five it does not.
    #[test]
    fn a_principal_point_error_needs_the_principal_point_to_come_out() {
        let lenses = fixture_lenses();
        let error = SeamFit {
            cx_px: -5.09,
            cy_px: -11.15,
            ..SeamFit::default()
        };
        let readings = readings_for(error, &lenses, 72);
        let rotation = fit(
            &readings,
            &lenses,
            FRAME,
            &[Knob::Roll, Knob::Yaw, Knob::Pitch],
        )
        .unwrap();
        let five = fit(&readings, &lenses, FRAME, &KNOBS).unwrap();
        assert!(
            five.after[0] < 0.1 * rotation.after[0],
            "along the seam: {:.4} deg left by a rotation, {:.4} by five knobs",
            rotation.after[0],
            five.after[0],
        );
    }

    /// The ridge is a prior on the principal point and nothing else: it pulls
    /// a shift the readings barely support towards zero, and leaves the
    /// angles where they were.
    #[test]
    fn the_ridge_holds_the_principal_point_and_not_the_angles() {
        let lenses = fixture_lenses();
        let error = SeamFit {
            roll_deg: 0.801,
            yaw_deg: -2.293,
            pitch_deg: -0.817,
            cx_px: -4.18,
            cy_px: -13.91,
        };
        // Six azimuths of one small arc, which is what a capture whose seam
        // has content in one place gives the fit.
        let readings: Vec<Reading> = readings_for(error, &lenses, 72)
            .into_iter()
            .take(6)
            .collect();
        let free = fit(&readings, &lenses, FRAME, &KNOBS).unwrap();
        let held = fit_held(&readings, &lenses, FRAME, &KNOBS, RIDGE).unwrap();
        assert!(
            held.fit.cx_px.hypot(held.fit.cy_px) < free.fit.cx_px.hypot(free.fit.cy_px),
            "the ridge did not hold the point: {:.2}, {:.2} free against {:.2}, {:.2} held",
            free.fit.cx_px,
            free.fit.cy_px,
            held.fit.cx_px,
            held.fit.cy_px,
        );
        assert!(
            (held.fit.turn_deg() - free.fit.turn_deg()).abs() < 0.5,
            "the ridge moved the rotation: {:.3} deg free against {:.3} held",
            free.fit.turn_deg(),
            held.fit.turn_deg(),
        );
    }

    /// A tilt is not a roll and the fit has to say which it saw: the along-seam
    /// column is the one only a roll reaches (6.8), so an injected roll must
    /// not come back as a tilt.
    #[test]
    fn a_roll_does_not_come_back_as_a_tilt() {
        let lenses = fixture_lenses();
        let error = SeamFit {
            roll_deg: -0.75,
            ..SeamFit::default()
        };
        let fitted = fit(&readings_for(error, &lenses, 72), &lenses, FRAME, &KNOBS).unwrap();
        assert!((fitted.fit.roll_deg + 0.75).abs() < 0.02);
        assert!(fitted.fit.yaw_deg.hypot(fitted.fit.pitch_deg) < 0.05);
    }

    /// Half a degree is the size of the residual this instrument was reporting
    /// before the degrees turned up, and #45's lesson is that a control has to
    /// be able to catch the failure it clears.
    #[test]
    fn half_a_degree_is_visible_to_the_fit() {
        let lenses = fixture_lenses();
        let error = SeamFit {
            yaw_deg: 0.5,
            ..SeamFit::default()
        };
        let fitted = fit(&readings_for(error, &lenses, 72), &lenses, FRAME, &KNOBS).unwrap();
        assert!((fitted.fit.yaw_deg / 0.5 - 1.0).abs() < 0.02);
    }

    /// A file whose content is at one azimuth cannot separate the knobs, and
    /// the guard for that is a count rather than a hope: a fit with fewer
    /// patches than knobs does not exist at all.
    #[test]
    fn too_few_patches_is_no_fit() {
        let lenses = fixture_lenses();
        let readings = readings_for(SeamFit::default(), &lenses, 72);
        assert!(fit(&readings[..1], &lenses, FRAME, &KNOBS).is_none());
    }

    /// A one-lens file has no seam, and the correction it is handed is the
    /// calibration it came with.
    #[test]
    fn a_one_lens_file_is_left_alone() {
        let lenses = fixture_lenses();
        let corrected = SeamFit {
            roll_deg: 1.0,
            yaw_deg: 1.0,
            pitch_deg: 1.0,
            cx_px: 1.0,
            cy_px: 1.0,
        }
        .applied(&lenses[..1]);
        assert_eq!(corrected.len(), 1);
        assert_eq!(corrected[0].pose.roll_deg, lenses[0].pose.roll_deg);
    }

    /// A stored calibration is in the picture before the first frame is: no
    /// decode, no thread, nothing to land later. That is the whole reason the
    /// correction moved off the file and onto the camera, and it is what the
    /// step two seconds into a first play used to be.
    #[test]
    fn a_stored_calibration_needs_no_file_and_no_wait() {
        let lenses: Arc<[Lens]> = fixture_lenses().into();
        let fit = SeamFit {
            roll_deg: 0.810,
            yaw_deg: -2.352,
            pitch_deg: -0.678,
            cx_px: -4.18,
            cy_px: -13.91,
        };
        let landed = Corrected::default();
        known(&landed, &lenses, fit);
        let corrected = landed.get().expect("the calibration is not in the picture");
        assert_eq!(corrected[1].pose.yaw_deg, lenses[1].pose.yaw_deg - 2.352);
        assert_eq!(corrected[1].intrinsics.cy, lenses[1].intrinsics.cy - 13.91);
        assert_eq!(corrected[0].pose.yaw_deg, lenses[0].pose.yaw_deg);
    }

    /// Nothing is spawned and nothing is applied for a file that cannot have
    /// a seam, which is what makes the legacy one-lens cameras cost nothing.
    #[test]
    fn a_one_lens_file_starts_no_fit() {
        let lenses: Arc<[Lens]> = fixture_lenses()[..1].into();
        let landed = Corrected::default();
        correct(&landed, Path::new("/nonexistent.insv"), &lenses, FRAME);
        known(&landed, &lenses, SeamFit::default());
        assert!(landed.get().is_none());
        assert!(Job::new(&lenses, FRAME).is_none());
    }
}
