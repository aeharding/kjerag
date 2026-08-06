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
//! So the fit is pooled per camera rather than believed per file, under
//! [`CalibrationSet::camera_key`], which is the model and the factory
//! calibration and is not the serial. **Nothing here asks the pilot for
//! anything** (AGENTS.md, zero-config playback): the pool is filled by
//! watching, from fits the app makes while a file plays and gates on their own
//! quality, and a camera the pool already knows is in the first frame with
//! nothing decoded.
//!
//! A file whose camera the pool does not know yet is still corrected, from its
//! own frames, best effort ([`Scene::fit_seam`](crate::Scene::fit_seam)).
//! That is the weaker path and it says so in the line it prints.
//!
//! What the pass draws with is a [`Correction`], not a constant: it is asked
//! for an answer and walks towards it while the file plays, so a better
//! reading never arrives as a jump.
//!
//! The fit is phase 1's own measurement, in the shipped map's units. Both
//! lenses are sampled on the **same angular grid** around directions on the
//! seam circle, so what best correlates between them is a disagreement in
//! degrees of world angle with no rotation to undo; each calibration field is
//! then turned by a probe amount and the map is asked what that does to the
//! same patches, which is a column of the design matrix in the units
//! `offset_v3` writes. `kjerag-spike --bin seam` is the same core with the
//! attribution, the harmonics and the controls printed round it.
//!
//! Nothing here decides what a picture looks like: the answer is a patch to a
//! [`Lens`], and the pass runs on the patched calibration exactly as it ran on
//! the factory one.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use kjerag_media::{Fallible, Plane, Walk};
use kjerag_meta::Lens;

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
///
/// A direction this lens has no picture of is a **hole**: the sample sits
/// there and means nothing, and [`Grid::whole`] is what says which patches of
/// the grid are clear of them.
struct Grid {
    along: isize,
    across: isize,
    luma: Vec<f64>,
    /// How many holes lie between this grid's first sample and each sample,
    /// with a row and a column of zeros ahead of it: a summed-area table, so
    /// asking whether one patch has a hole in it is four reads rather than a
    /// sweep of the patch.
    holes: Vec<u32>,
}

impl Grid {
    /// A grid from one lens's answers about `2 * half + 1` directions each
    /// way, in the order [`sample`] asks for them: `None` where this lens has
    /// no picture of that direction.
    fn of(half: (isize, isize), taps: &[Option<f64>]) -> Self {
        let columns = 2 * half.1 + 1;
        let width = (columns + 1) as usize;
        let mut holes = vec![0; ((2 * half.0 + 2) * (columns + 1)) as usize];
        for (index, tap) in taps.iter().enumerate() {
            let (row, column) = (index / columns as usize + 1, index % columns as usize + 1);
            holes[row * width + column] = u32::from(tap.is_none())
                + holes[(row - 1) * width + column]
                + holes[row * width + column - 1]
                - holes[(row - 1) * width + column - 1];
        }
        Self {
            along: half.0,
            across: half.1,
            luma: taps.iter().map(|tap| tap.unwrap_or(0.0)).collect(),
            holes,
        }
    }

    fn at(&self, i: isize, j: isize) -> f64 {
        self.luma[((i + self.along) * (2 * self.across + 1) + (j + self.across)) as usize]
    }

    /// Whether the patch of `half` extent, centred `(di, dj)` steps from this
    /// grid's own centre, is wholly in this lens's picture.
    ///
    /// The two lenses have to be answering about the same directions or the
    /// correlation means nothing, and this is what asks that of **one
    /// candidate offset** rather than of the rectangle holding all of them.
    /// A patch that leaves the overlap band at one offset is still in both
    /// pictures at another, and on a camera whose factory extrinsic is degrees
    /// out there is no offset at all where the whole search stays inside: read
    /// as a rectangle, every candidate on such a file is refused for where
    /// some other candidate landed (issue #130).
    fn whole(&self, di: isize, dj: isize, half: (isize, isize)) -> bool {
        let width = 2 * self.across + 2;
        let sum = |row: isize, column: isize| self.holes[(row * width + column) as usize];
        let (top, bottom) = (di - half.0 + self.along, di + half.0 + self.along + 1);
        let (left, right) = (dj - half.1 + self.across, dj + half.1 + self.across + 1);
        sum(bottom, right) + sum(top, left) == sum(top, right) + sum(bottom, left)
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

/// One lens's picture of the sphere around `at`, holes and all. Which parts
/// of it this lens actually has a picture of is [`Grid::whole`]'s answer, per
/// patch, and never the whole rectangle's.
fn sample(
    reframe: &Reframe,
    plane: &Plane,
    lens: usize,
    at: &Where,
    half: (isize, isize),
    step: f64,
) -> Grid {
    let mut taps = Vec::with_capacity(((2 * half.0 + 1) * (2 * half.1 + 1)) as usize);
    let moved = lens == 1 && !reframe.table().is_rest();
    for i in -half.0..=half.0 {
        for j in -half.1..=half.1 {
            let (a, b) = (i as f64 * step, j as f64 * step);
            let ray = unit(std::array::from_fn(|axis| {
                at.centre[axis] + at.along[axis] * a + at.across[axis] * b
            }));
            // Through the table the picture is drawn with, if there is one:
            // a ring read past a correction that is already being applied
            // would ask for it a second time ([`Reframe::tabled`]). The test
            // for one is hoisted out of the loop: it compares every direction
            // of the table, and this loop runs thousands of times per patch.
            let ray = ray.map(|c| c as f32);
            let landing = match moved {
                true => reframe.project(lens, reframe.tabled(lens, ray)),
                false => reframe.project(lens, ray),
            };
            taps.push(
                landing
                    .inside
                    .then(|| plane.at(f64::from(landing.pixel[0]), f64::from(landing.pixel[1])))
                    .flatten(),
            );
        }
    }
    Grid::of(half, &taps)
}

/// Where one patch's correlation peaked, in grid steps, and how well it
/// correlated there.
struct Peak {
    along: f64,
    across: f64,
    r: f64,
    /// Whether the peak is against the edge of what could be tried, which is
    /// the search running out or the overlap band doing so. Either way the
    /// number is the edge's and not the content's.
    pinned: bool,
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
///
/// Each candidate offset is scored on **its own** patch of `back`, and only
/// where `back` has a picture of the whole of that patch ([`Grid::whole`]).
/// The rays are the same rays either way; what changes is that a candidate is
/// refused for where **it** landed rather than for where the widest one did.
///
/// The along-seam half of the search runs `search.0` either side of `centre`
/// rather than of nought, which is [`acquired`]'s answer for this ring.
fn best_shift(
    front: &Grid,
    back: &Grid,
    search: (isize, isize),
    half: (isize, isize),
    centre: isize,
) -> Option<Peak> {
    let (first, last) = (centre - search.0, centre + search.0);
    let stride = (search.0.max(search.1) / 12).max(1);
    // How far apart the shifts are tried and how far apart the samples are
    // scored are two different strides, and tying them together is how a
    // coarse pass over a wide search ends up correlating sixteen pixels
    // against sixteen pixels and finding a peak in the noise.
    let coarse = stride.min(3);
    let usable = |di: isize, dj: isize| {
        (first..=last).contains(&di) && dj.abs() <= search.1 && back.whole(di, dj, half)
    };
    let score = |di, dj, stride| front.correlation(back, di, dj, stride);
    let mut best: Option<(isize, isize, f64)> = None;
    let mut di = first;
    while di <= last {
        let mut dj = -search.1;
        while dj <= search.1 {
            if let Some(r) = usable(di, dj).then(|| score(di, dj, coarse))
                && best.is_none_or(|(_, _, held)| r > held)
            {
                best = Some((di, dj, r));
            }
            dj += stride;
        }
        di += stride;
    }
    let (coarse_i, coarse_j, _) = best?;
    let mut best: Option<(isize, isize, f64)> = None;
    for di in (coarse_i - stride).max(first)..=(coarse_i + stride).min(last) {
        for dj in (coarse_j - stride).max(-search.1)..=(coarse_j + stride).min(search.1) {
            if !usable(di, dj) {
                continue;
            }
            let r = score(di, dj, 1);
            if best.is_none_or(|(_, _, held)| r > held) {
                best = Some((di, dj, r));
            }
        }
    }
    let (i, j, r) = best?;
    // A winner with nothing measurable beside it has neighbours it was never
    // compared against, so the parabola has nothing to interpolate and the
    // reading is refused below rather than refined here.
    let hemmed = [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .any(|(a, b)| !usable(i + a, j + b));
    let peak = |minus: f64, here: f64, plus: f64| {
        let curve = minus - 2.0 * here + plus;
        match curve < 0.0 {
            true => (0.5 * (minus - plus) / curve).clamp(-1.0, 1.0),
            false => 0.0,
        }
    };
    let (mut refined_i, mut refined_j) = (0.0, 0.0);
    if !hemmed {
        refined_i = peak(score(i - 1, j, 1), r, score(i + 1, j, 1));
        refined_j = peak(score(i, j - 1, 1), r, score(i, j + 1, 1));
    }
    let (along, across) = (i as f64 + refined_i, j as f64 + refined_j);
    Some(Peak {
        along,
        across,
        r,
        pinned: hemmed
            || along <= first as f64
            || along >= last as f64
            || across.abs() >= search.1 as f64,
    })
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
    /// Lens 0 has no picture of the whole patch, or lens 1 has none of it at
    /// any offset the search could try: the overlap band is only so wide, so
    /// near-field content that parallax has moved further than that is not in
    /// both pictures at all and no instrument can pair it.
    pub outside: usize,
    pub flat: usize,
    pub unlike: usize,
    pub pinned: usize,
}

/// How far along the seam the acquiring pass looks, in degrees, and how much
/// coarser than the reading pass it looks.
///
/// It is answering "roughly where along the seam is lens 1's picture of this
/// ring", to the nearest degree, so it neither needs the fine grid nor the
/// whole of it: a quarter of the sampling rate and a third of the azimuths.
/// Six degrees is over twice the widest gross offset measured on any camera
/// here and under [`RUNAWAY_DEG`], and it costs what it costs only along the
/// seam, where the overlap band does not run out.
const ACQUIRE_DEG: f64 = 6.0;
const ACQUIRE_COARSER: f64 = 4.0;
const ACQUIRE_EVERY: usize = 3;

/// How many azimuths have to agree before this frame is allowed to answer at
/// all. Below it the frame says nothing and the next one is asked.
const ACQUIRE_NEEDED: usize = 5;

/// Where along the seam this ring's whole picture sits, in grid steps, so the
/// search can be centred there rather than on nought. `None` from a frame
/// with too little on its seam to say, which is a frame to ask again after
/// rather than an answer of nought.
///
/// **Parallax cannot reach the along-seam axis at any distance**
/// (docs/research/seam-two-axis.md 1): the baseline between the lenses is
/// perpendicular to every direction on the seam circle. So an offset the
/// whole ring shares along the seam is the factory extrinsic being out, and
/// on a camera far enough out it is larger than the window the search runs
/// in: the owner's ONE X2 reads 2.1 to 2.9 degrees along a window of 2.0, so
/// every azimuth peaks against the limit and the few that survive report the
/// limit rather than the camera (issue #130).
///
/// Across the seam the window stays where it is, because that is the axis
/// parallax owns: a gross offset there is the scene's distances, and moving
/// the search onto it would be searching for the near field rather than
/// around it.
///
/// The answer is a median, so one azimuth's false peak cannot move it, and a
/// ring the window already reaches is **not moved at all**: the search stays
/// exactly where the camera's own calibration puts it, which is where every
/// capture with a good factory extrinsic has always been read. Nothing here
/// is a widening, so nothing here lets a peak that was refused as false
/// through: the window keeps its width and only stops assuming the camera was
/// right about where its own lenses point.
pub fn acquired(
    reframe: &Reframe,
    planes: &[Plane],
    ring: &[Where],
    probe: &Probe,
) -> Option<isize> {
    let coarse = Probe {
        step: probe.step * ACQUIRE_COARSER,
        along: ACQUIRE_DEG,
        ..*probe
    };
    let thinned: Vec<Where> = ring.iter().step_by(ACQUIRE_EVERY).copied().collect();
    let along = read_ring_centred(
        reframe,
        planes,
        &thinned,
        &coarse,
        0,
        &mut Refused::default(),
    )
    .into_iter()
    .flatten()
    .filter(|found| found.r >= probe.keep)
    .map(|found| found.along)
    .collect();
    centred_on(along, probe)
}

/// The rule [`acquired`] applies to what it read: the median in whole degrees,
/// as grid steps, and nought for a ring the window already reaches.
fn centred_on(mut along: Vec<f64>, probe: &Probe) -> Option<isize> {
    if along.len() < ACQUIRE_NEEDED {
        return None;
    }
    along.sort_by(f64::total_cmp);
    let median = along[along.len() / 2];
    Some(match median.abs() < probe.along {
        true => 0,
        false => (median.round() / probe.step) as isize,
    })
}

/// Every patch round the seam of one frame, under one calibration, in patch
/// order. `None` where a lens has no usable picture of that patch, or where
/// there is nothing in it to correlate.
///
/// This one asks [`acquired`] where to centre the search on every frame it is
/// given. A caller reading several frames of one capture wants
/// [`read_ring_centred`] instead: what the acquiring pass measures is fixed in
/// the camera for the life of the file, so asking once and holding the answer
/// is both cheaper and steadier than asking per frame.
pub fn read_ring(
    reframe: &Reframe,
    planes: &[Plane],
    ring: &[Where],
    probe: &Probe,
    refused: &mut Refused,
) -> Vec<Option<Found>> {
    let centre = acquired(reframe, planes, ring, probe).unwrap_or(0);
    read_ring_centred(reframe, planes, ring, probe, centre, refused)
}

/// The same ring, with the along-seam search centred `centre` grid steps from
/// the calibration the camera wrote ([`acquired`]).
pub fn read_ring_centred(
    reframe: &Reframe,
    planes: &[Plane],
    ring: &[Where],
    probe: &Probe,
    centre: isize,
    refused: &mut Refused,
) -> Vec<Option<Found>> {
    let step = probe.step.to_radians();
    let half = (probe.span.to_radians() / 2.0 / step) as isize;
    let search = (
        (probe.along / probe.step) as isize,
        (probe.across / probe.step) as isize,
    );
    let patch = (half, half);
    ring.iter()
        .map(|at| {
            let front = sample(reframe, planes.first()?, 0, at, patch, step);
            if !front.whole(0, 0, patch) {
                refused.outside += 1;
                return None;
            }
            if front.contrast() < probe.contrast {
                refused.flat += 1;
                return None;
            }
            // The rectangle holding the search reaches the recentred window
            // and stays centred on the patch, so every candidate's picture is
            // the front's own sampling shifted by whole steps and nothing is
            // resampled to move the window.
            let back = sample(
                reframe,
                planes.get(1)?,
                1,
                at,
                (half + search.0 + centre.abs(), half + search.1),
                step,
            );
            // No candidate offset at all had lens 1's picture of the whole
            // patch, which is the overlap band being narrower than one patch.
            let Some(peak) = best_shift(&front, &back, search, patch, centre) else {
                refused.outside += 1;
                return None;
            };
            if peak.r < probe.keep {
                refused.unlike += 1;
            }
            // A peak against the edge of the search is not a peak, it is the
            // search running out. Near-field content at this seam moves
            // further across than the overlap band is wide, and a reading
            // pinned at the limit would report the limit.
            if peak.pinned {
                refused.pinned += 1;
                return None;
            }
            Some(Found {
                along: (peak.along * step).to_degrees(),
                across: (peak.across * step).to_degrees(),
                r: peak.r,
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

    /// A `fraction` of the way from this correction to `to`, knob by knob.
    ///
    /// A straight line in the five knobs rather than a rotation interpolated
    /// separately from a principal point: the knobs are a fit's own parameters
    /// and they trade against each other inside one, so the point between two
    /// fits that a walk should pass through is the one that keeps their
    /// proportions. At the sizes this walks over, degrees rather than turns,
    /// the difference from a slerp of the rotation part is below the
    /// arithmetic ([`the_walk_is_a_straight_line_between_two_fits`]).
    fn towards(self, to: Self, fraction: f64) -> Self {
        let step = |from: f64, to: f64| from + (to - from) * fraction;
        Self {
            roll_deg: step(self.roll_deg, to.roll_deg),
            yaw_deg: step(self.yaw_deg, to.yaw_deg),
            pitch_deg: step(self.pitch_deg, to.pitch_deg),
            cx_px: step(self.cx_px, to.cx_px),
            cy_px: step(self.cy_px, to.cy_px),
        }
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

/// A fit the app made by watching, with what it is worth beside it.
///
/// The pool keeps these rather than bare corrections, because a fit off a file
/// with seven near-field patches and a fit off one with fifty far-field ones
/// are not the same evidence and must not be averaged as if they were (6.8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Harvest {
    pub fit: SeamFit,
    /// How many azimuths round the seam circle correlated. The count, not the
    /// residual, is what caught both of 6.8's bad captures.
    pub patches: usize,
    /// What the fit left across the seam, in degrees, predicted through the
    /// map. Lower is a fit that flattened more of what it was given.
    pub residual_deg: f64,
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
    let left = predicted(readings, fit, lenses, frame);
    [
        rms(left.iter().map(|(_, axes)| axes[0])),
        rms(left.iter().map(|(_, axes)| axes[1])),
    ]
}

/// The same, kept per azimuth rather than reduced: where each patch would read
/// with the correction in place, along the seam first and across it second.
fn predicted(
    readings: &[Reading],
    fit: &SeamFit,
    lenses: &[Lens],
    frame: Size,
) -> Vec<(Where, [f64; 2])> {
    let base = mapped(lenses, frame);
    let corrected = mapped(&fit.applied(lenses), frame);
    readings
        .iter()
        .filter_map(|reading| {
            let shift = moved(&base, &corrected, 1, &reading.at)?;
            let axes = [reading.along + shift[0], reading.across + shift[1]];
            Some((reading.at, axes))
        })
        .collect()
}

/// What a pose leaves along the seam, azimuth by azimuth: the observation
/// [`super::band::Table`] is built out of (issue #103, stage 9).
///
/// The along-seam axis only, because that is the axis parallax cannot reach
/// and therefore the only one whose leftover is the camera rather than the
/// scene. Across the seam the same readings carry a scene's distances, and
/// what is left there is a per-session question the band already answers per
/// frame.
///
/// Every reading counts once. It is already the mean over the frames it
/// correlated on, the kernel that builds the table is far wider than the five
/// degrees between azimuths, and a table entry that rested on one reading
/// would be shrunk to nothing by the ridge whatever weight it carried.
pub fn left(
    readings: &[Reading],
    fit: &SeamFit,
    lenses: &[Lens],
    frame: Size,
    gate: Option<f64>,
) -> Left {
    let all: Vec<super::band::Leftover> = predicted(readings, fit, lenses, frame)
        .into_iter()
        .map(|(at, axes)| super::band::Leftover {
            phi: at.phi as f32,
            perp: axes[0].to_radians() as f32,
            weight: 1.0,
        })
        .collect();
    let Some(mads) = gate else {
        return Left {
            refused: 0,
            readings: all,
            tolerance: f32::INFINITY,
        };
    };
    let middle = middle_of(all.iter().map(|l| f64::from(l.perp)));
    let scatter = middle_of(all.iter().map(|l| f64::from(l.perp) - middle).map(f64::abs));
    let tolerance = (mads * scatter).max(GATE_FLOOR_DEG.to_radians()) as f32;
    let kept: Vec<super::band::Leftover> = all
        .iter()
        .copied()
        .filter(|l| f64::from((l.perp - middle as f32).abs()) <= f64::from(tolerance))
        .collect();
    Left {
        refused: all.len() - kept.len(),
        readings: kept,
        tolerance,
    }
}

/// What one pose left on one capture's ring, and what had to be thrown away
/// to say it.
pub struct Left {
    pub readings: Vec<super::band::Leftover>,
    /// How far from this capture's own middle a reading was allowed to sit,
    /// in radians.
    pub tolerance: f32,
    pub refused: usize,
}

/// How many times its own scatter a reading may sit from its capture's middle
/// before it is a correlation on the wrong feature rather than a camera.
///
/// The number [`left`]'s callers pass unless they are deliberately measuring
/// what the gate does, which `--bin table gate=0` is for: a conclusion that
/// turns on a filter has to be shown turning on it.
///
/// **A tolerance filter on a physical argument, not a classifier**, and the
/// same argument `--bin crossing`'s along-seam gate is built on: a capture's
/// calibration does not change while it plays and no distance can reach this
/// axis, so one capture's along-seam readings are one number plus a slow trend
/// round the ring. What is refused is not a tail: on the owner's six flights
/// the kept readings sit 0.054 degrees from their own middle on average and
/// the refused ones reach 2.5, which is past the window the search even runs
/// in (docs/research/stage9.md).
pub const GATE_MADS: f64 = 4.0;

/// The narrowest that tolerance may become, in degrees.
///
/// A capture whose readings happen to agree closely must not thereby refuse a
/// real one. It is the size of the along-seam residual itself - five of the
/// six flights in the corpus read 0.064 to 0.084 degrees rms under their pose
/// and the sixth reads 0.128 - so a reading this far from its own capture's
/// middle is a whole residual away from it, and a reading twice as far again
/// is what the gate is actually for (docs/research/stage9.md 5).
const GATE_FLOOR_DEG: f64 = 0.10;

/// The middle of a set of readings, which is a median and not a mean: one
/// correlation on the wrong feature moves a mean by its whole size and a
/// median not at all.
fn middle_of(values: impl Iterator<Item = f64>) -> f64 {
    let mut all: Vec<f64> = values.collect();
    if all.is_empty() {
        return 0.0;
    }
    all.sort_by(f64::total_cmp);
    all[all.len() / 2]
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
    /// The along-seam table the picture is already being drawn with
    /// ([`super::band::Table`]).
    ///
    /// A ring is read through the map the picture is drawn through, which is
    /// the same rule the band's own measurement follows: what a measurement
    /// answers is what is *still* wrong, and a correction the reading cannot
    /// see would be asked for twice. [`Table::REST`] on a camera nothing has
    /// been pooled for, which is every capture the first time it plays.
    pub table: super::band::Table,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            places: 3,
            frames: 2,
            probe: Probe::default(),
            table: super::band::Table::REST,
        }
    }
}

/// Every patch this capture's seam offers, pooled over the frames it was read
/// on.
///
/// A capture is its files in lens order, not the one path it was named by
/// ([`Walk::over`], issue #123). A capture picked in a sandbox's file chooser
/// has its second lens in the pilot's second pick and nowhere beside the
/// first, so a fit that starts from one path again reads half the capture and
/// then says the whole of it has one lens.
pub fn measure(
    files: &[PathBuf],
    lenses: &[Lens],
    frame: Size,
    plan: &Plan,
) -> Fallible<Vec<Reading>> {
    let base = mapped(lenses, frame).with_table(plan.table);
    let ring = ring(plan.probe.patches);
    let mut walk = Walk::over(files, 0.0, frame)?;
    if walk.streams() < 2 {
        return Err("this capture carries one lens stream, so it has no seam".into());
    }
    let duration = walk.duration().as_secs_f64();
    let mut sums: Vec<(usize, f64, f64)> = vec![(0, 0.0, 0.0); ring.len()];
    let mut refused = Refused::default();
    let mut centre = None;
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
            // Where the search is centred is the camera's own answer and not
            // this frame's ([`acquired`]), so it is asked for once and held.
            // A frame with too little on its seam to answer is asked past.
            if centre.is_none() {
                centre = acquired(&base, &pair.lenses, &ring, &plan.probe);
            }
            for (found, sum) in read_ring_centred(
                &base,
                &pair.lenses,
                &ring,
                &plan.probe,
                centre.unwrap_or(0),
                &mut refused,
            )
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
    files: &[PathBuf],
    lenses: &[Lens],
    frame: Size,
    plan: &Plan,
) -> Result<Fitted, String> {
    let readings = measure(files, lenses, frame, plan).map_err(|e| e.to_string())?;
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
pub fn fit_reported(
    files: &[PathBuf],
    lenses: &[Lens],
    frame: Size,
    plan: &Plan,
) -> Option<Fitted> {
    match fit_capture(files, lenses, frame, plan) {
        Ok(fitted) => Some(fitted),
        Err(why) => {
            println!("seam:   {why}; keeping the factory calibration");
            None
        }
    }
}

// ------------------------------------------------------------ the correction

/// How long a correction takes to walk in, whatever its size, in seconds.
///
/// A fixed duration and not a fixed rate, which is the owner's own number:
/// "self-fit lands as a refinement EASED IN over ~1 s". A rate was written
/// first and measured, and it is why this comment exists: at 0.25 deg/s the
/// cold-start fit of the owner's own camera, which is 26.8 probe steps of
/// correction, took **10.7 seconds** to walk in, and the headless harness
/// caught it as 803730 of 921600 pixels differing between two captures a
/// moment apart while the file was paused. Half the sphere sliding for ten
/// seconds is not below perception, it is the most visible thing in the
/// window.
///
/// Fixed duration inverts that: a big correction moves fast and is over, a
/// small one is imperceptible anyway. The worst case is the cold start, and it
/// is the landing step the owner measured at 39 to 52 view pixels; spread over
/// a second at 30 fps that is 1.3 to 1.7 pixels a frame.
const WALK_SECONDS: f64 = 1.0;

/// The seam correction the pass runs on, and where it is heading.
///
/// Not landed once (ROADMAP 2026-07-31, the revised seam architecture): the app
/// targets any 360 footage, near-field content generally moves, and readings
/// land while the file plays. Two corrections live here, the one the readings
/// ask for and the one the picture is drawn with, and the second walks towards
/// the first over [`WALK_SECONDS`].
///
/// The walk is the whole reason the second one exists. A correction that snaps
/// is a seam that jumps, and a jump is the one artifact an eye is built to
/// catch: motion where there was none reads as a fault even when it is a
/// picture getting better. What lands at open does not walk ([`Self::land`]),
/// because there is nothing to walk from.
pub struct Correction {
    /// The calibration the camera wrote, which every correction is a patch to.
    factory: Arc<[Lens]>,
    walking: Mutex<Walking>,
}

struct Walking {
    /// What the readings ask for.
    asked: SeamFit,
    /// Where the walk in progress started, so the ease is measured from a
    /// fixed point and takes the same time whatever it is crossing. Easing
    /// towards the target from wherever the picture currently is would make
    /// the last tenth take as long as the first, which is a different curve
    /// and a slower one.
    from: SeamFit,
    /// How far along that walk the picture is, 0 to 1.
    progress: f64,
    /// What the picture is drawn with.
    shown: SeamFit,
    /// `shown` applied to `factory`. Rebuilt only when `shown` moves, so a
    /// redraw of a settled correction costs one lock and one `Arc` clone.
    lenses: Arc<[Lens]>,
    /// When `shown` last moved, so the walk is per second rather than per
    /// redraw: a 144 Hz window must not correct five times faster than a
    /// 30 Hz one, and a paused window must not correct at all.
    walked: Option<Instant>,
}

impl Correction {
    /// The factory calibration, with nothing asked for. What a file with no
    /// seam keeps for good, and what every file draws its first frame with
    /// unless a stored calibration lands first.
    pub fn none(lenses: &Arc<[Lens]>) -> Self {
        Self {
            factory: lenses.clone(),
            walking: Mutex::new(Walking {
                asked: SeamFit::default(),
                from: SeamFit::default(),
                progress: 1.0,
                shown: SeamFit::default(),
                lenses: lenses.clone(),
                walked: None,
            }),
        }
    }

    /// Draw with this correction from the next frame, with no walk.
    ///
    /// What a stored calibration does at open: there is no picture yet to move
    /// under, so there is nothing to hide. Also what a still takes, which has
    /// no later moment to correct itself in.
    pub fn land(&self, fit: SeamFit) {
        let mut walking = self.walking.lock().unwrap_or_else(|e| e.into_inner());
        walking.asked = fit;
        walking.from = fit;
        walking.shown = fit;
        walking.progress = 1.0;
        walking.lenses = fit.applied(&self.factory).into();
        walking.walked = None;
    }

    /// Ask for this correction. The picture walks towards it from wherever it
    /// is now, and reaches it or is overtaken by a better answer first.
    pub fn ask(&self, fit: SeamFit) {
        let mut walking = self.walking.lock().unwrap_or_else(|e| e.into_inner());
        if walking.asked == fit {
            return;
        }
        walking.from = walking.shown;
        walking.asked = fit;
        walking.progress = 0.0;
        walking.walked = None;
    }

    /// What the pass runs on this redraw, having taken one step of the walk.
    ///
    /// The clock is read here rather than passed in because this is the only
    /// caller that needs it, and reading it costs less than threading an
    /// instant through the primitive that would only ever be used here.
    pub fn lenses(&self) -> Arc<[Lens]> {
        let mut walking = self.walking.lock().unwrap_or_else(|e| e.into_inner());
        if walking.progress >= 1.0 {
            return walking.lenses.clone();
        }
        let now = Instant::now();
        let since = walking.walked.replace(now);
        // The first redraw after an ask has no interval behind it, so it walks
        // nothing and the one after it walks from here.
        let Some(seconds) = since.map(|then| now.duration_since(then).as_secs_f64()) else {
            return walking.lenses.clone();
        };
        walking.progress = (walking.progress + seconds / WALK_SECONDS).min(1.0);
        walking.shown = walking.from.towards(walking.asked, walking.progress);
        walking.lenses = walking.shown.applied(&self.factory).into();
        walking.lenses.clone()
    }

    /// What is drawn and what is asked for, for a report line or an instrument.
    pub fn state(&self) -> (SeamFit, SeamFit) {
        let walking = self.walking.lock().unwrap_or_else(|e| e.into_inner());
        (walking.shown, walking.asked)
    }
}

/// How far apart two corrections are, in [`Knob::probe`] steps.
///
/// Probe steps rather than any one knob's own units, because the five are not
/// commensurable: a degree of yaw and a pixel of principal point are different
/// things, and the probe is the scale the fit itself already compares them on.
///
/// What the pool's answer is chosen by (`SeamPool::answer`) and what the tests
/// measure a walk with. The walk itself stopped needing it when it stopped
/// being a rate: a fixed-duration ease does not care how far it is going, which
/// is the point of it.
pub fn distance(from: SeamFit, to: SeamFit) -> f64 {
    let steps = [
        (to.roll_deg - from.roll_deg) / Knob::Roll.probe(),
        (to.yaw_deg - from.yaw_deg) / Knob::Yaw.probe(),
        (to.pitch_deg - from.pitch_deg) / Knob::Pitch.probe(),
        (to.cx_px - from.cx_px) / Knob::Cx.probe(),
        (to.cy_px - from.cy_px) / Knob::Cy.probe(),
    ];
    steps.iter().map(|s| s * s).sum::<f64>().sqrt()
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
    use std::time::Duration;

    use super::*;

    use crate::projection::tests::{FRAME, fixture_lenses};

    // ------------------------------------------ the patch and the search
    //
    // Issue #130: a camera whose factory extrinsic is degrees out could never
    // be fitted at all. These four are the two faults that made that true,
    // each in the smallest arrangement that shows it, with the projection and
    // the pictures taken out: a `Grid` is a rectangle of samples and a hole is
    // a sample a lens has no picture of, whatever the geometry that put it
    // there.

    /// One lens's rectangle: `half` steps either side of centre, filled from
    /// `luma`, with the samples `hole` marks missing.
    fn grid(
        half: (isize, isize),
        luma: impl Fn(isize, isize) -> f64,
        hole: impl Fn(isize, isize) -> bool,
    ) -> Grid {
        let mut taps = Vec::new();
        for i in -half.0..=half.0 {
            for j in -half.1..=half.1 {
                taps.push((!hole(i, j)).then(|| luma(i, j)));
            }
        }
        Grid::of(half, &taps)
    }

    /// Something with a correlation peak in it and no periodicity to find a
    /// second one at: two ramps and a hash, which is what a hillside and a
    /// treeline are to a correlator.
    fn texture(i: isize, j: isize) -> f64 {
        let (x, y) = (i as f64, j as f64);
        128.0
            + 9.0 * (x * 0.7).sin()
            + 7.0 * (y * 0.45).cos()
            + 3.0 * ((i * 7 + j * 13) % 11) as f64
    }

    /// A patch is refused for where **it** landed, not for where the widest
    /// candidate landed. The whole search rectangle leaves the picture here,
    /// which is the ONE X2's every azimuth: 157 of 432 tries, against 0 on a
    /// camera whose lenses point where it says.
    #[test]
    fn a_candidate_is_refused_for_its_own_landing_and_not_its_neighbours() {
        let patch = (3, 3);
        let search = (0, 8);
        let half = (patch.0 + search.0, patch.1 + search.1);
        // The overlap band runs out five steps past the patch, which no
        // rectangle covering the whole search can stay inside.
        let back = grid(half, texture, |_, j| j > 5);
        assert!(
            !back.whole(0, 0, half),
            "the rectangle holding the search is inside the picture, so there is nothing to show"
        );
        assert!(
            back.whole(0, 0, patch),
            "the patch itself is off the picture"
        );
        assert!(
            back.whole(0, 2, patch),
            "a candidate clear of the holes is refused"
        );
        assert!(
            !back.whole(0, 4, patch),
            "a candidate over the holes is kept"
        );
    }

    /// And the search finds the shift anyway, on a rectangle the old rule
    /// refused outright.
    #[test]
    fn the_search_reads_a_patch_the_overlap_band_cuts_short() {
        let patch = (3, 3);
        let search = (4, 8);
        let half = (patch.0 + search.0, patch.1 + search.1);
        let shift = (2, -3);
        let front = grid(patch, texture, |_, _| false);
        let back = grid(half, |i, j| texture(i - shift.0, j - shift.1), |_, j| j > 5);
        let peak = best_shift(&front, &back, search, patch, 0).expect("nothing was tried at all");
        assert!(!peak.pinned, "the peak came back pinned");
        assert!(
            (peak.along - shift.0 as f64).abs() < 0.5 && (peak.across - shift.1 as f64).abs() < 0.5,
            "read {:.2}, {:.2} of an injected {}, {}",
            peak.along,
            peak.across,
            shift.0,
            shift.1,
        );
    }

    /// A shift outside the window is refused rather than reported wrong, and
    /// the window moved onto it reads it. This is the ONE X2's other half: 2.1
    /// to 2.9 degrees along a window of 2.0, so every azimuth peaked against
    /// the limit.
    #[test]
    fn a_shift_outside_the_window_is_refused_until_the_window_moves_onto_it() {
        let patch = (3, 3);
        let search = (4, 4);
        let centre = 9;
        let half = (patch.0 + search.0 + centre, patch.1 + search.1);
        let front = grid(patch, texture, |_, _| false);
        let back = grid(half, |i, j| texture(i - centre, j), |_, _| false);
        let missed = best_shift(&front, &back, search, patch, 0).expect("nothing was tried");
        assert!(
            missed.pinned,
            "a shift {centre} steps out was reported as {:.2} from a window of {}",
            missed.along, search.0,
        );
        let found = best_shift(&front, &back, search, patch, centre).expect("nothing was tried");
        assert!(
            !found.pinned && (found.along - centre as f64).abs() < 0.5,
            "the window moved onto the shift read {:.2}, pinned {}",
            found.along,
            found.pinned,
        );
    }

    /// The window moves only when it cannot reach where the ring is, and one
    /// azimuth's false peak cannot move it: the whole point of the median.
    #[test]
    fn the_window_moves_only_for_a_ring_it_cannot_reach() {
        let probe = Probe::default();
        let steps = |deg: f64| (deg / probe.step) as isize;
        assert_eq!(
            centred_on(vec![0.4; 5], &probe),
            Some(0),
            "a ring inside the window moved"
        );
        assert_eq!(centred_on(vec![2.6; 5], &probe), Some(steps(3.0)));
        assert_eq!(
            centred_on(vec![2.6, 2.5, -5.9, 2.7, 2.4], &probe),
            Some(steps(3.0)),
            "one wild azimuth moved the window",
        );
        assert_eq!(
            centred_on(vec![2.6; ACQUIRE_NEEDED - 1], &probe),
            None,
            "a frame with too little on its seam answered anyway",
        );
    }

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
    /// decode, no thread, nothing to land later, and no walk either. That is
    /// the whole reason the correction moved off the file and onto the camera,
    /// and it is what the step two seconds into a first play used to be.
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
        let correction = Correction::none(&lenses);
        correction.land(fit);
        let corrected = correction.lenses();
        assert_eq!(corrected[1].pose.yaw_deg, lenses[1].pose.yaw_deg - 2.352);
        assert_eq!(corrected[1].intrinsics.cy, lenses[1].intrinsics.cy - 13.91);
        assert_eq!(corrected[0].pose.yaw_deg, lenses[0].pose.yaw_deg);
        assert_eq!(correction.state(), (fit, fit));
    }

    /// A one-lens file has no seam, so whatever is asked of it the picture is
    /// the calibration the camera wrote. That is what makes the legacy
    /// one-lens cameras cost nothing.
    #[test]
    fn a_one_lens_file_is_never_corrected() {
        let lenses: Arc<[Lens]> = fixture_lenses()[..1].into();
        let correction = Correction::none(&lenses);
        correction.land(SeamFit {
            yaw_deg: 1.0,
            ..SeamFit::default()
        });
        assert_eq!(correction.lenses()[0].pose.yaw_deg, lenses[0].pose.yaw_deg);
    }

    /// The walk is paced by the clock and not by the redraw count, and it
    /// takes the same time whatever its size. Both halves matter: the first is
    /// what stops a 144 Hz window correcting five times faster than a 30 Hz
    /// one, and the second is what stops a cold-start correction taking ten
    /// seconds, which is the defect this replaced.
    ///
    /// Wall-clock rather than an injected instant, because the walk reads the
    /// clock itself. The assertion is therefore one-sided: after a known sleep
    /// the picture is at most that far along, however many redraws ran.
    #[test]
    fn the_walk_is_paced_by_the_clock_and_not_by_the_redraw_count() {
        let lenses: Arc<[Lens]> = fixture_lenses().into();
        let asked = SeamFit {
            yaw_deg: -2.4,
            ..SeamFit::default()
        };
        let correction = Correction::none(&lenses);
        correction.ask(asked);

        // The first redraw has no interval behind it and must move nothing.
        correction.lenses();
        assert_eq!(correction.state().0, SeamFit::default());

        let slept = Duration::from_millis(100);
        std::thread::sleep(slept);
        for _ in 0..50 {
            correction.lenses();
        }
        let (shown, _) = correction.state();
        let along = shown.yaw_deg / asked.yaw_deg;
        assert!(along > 0.0, "the walk did not start");
        // 50 redraws over 0.1 s of a 1 s walk. Paced by redraws it would be
        // finished many times over; paced by the clock it is a tenth of the
        // way, and the slack is this box's scheduler under load.
        assert!(
            along < 0.5,
            "{along:.3} of the way after {:.0} ms of a {WALK_SECONDS:.0} s walk",
            slept.as_secs_f64() * 1000.0
        );
    }

    /// A big correction and a small one take the same time, which is the whole
    /// difference between this and the rate it replaced. The cold-start fit of
    /// the owner's own camera is the big one: 26.8 probe steps, which at the
    /// old 2.5 steps a second was 10.7 seconds of half the sphere sliding.
    #[test]
    fn a_big_correction_takes_no_longer_than_a_small_one() {
        let lenses: Arc<[Lens]> = fixture_lenses().into();
        let big = SeamFit {
            roll_deg: 0.789,
            yaw_deg: -2.450,
            pitch_deg: -0.668,
            cx_px: -2.55,
            cy_px: -13.84,
        };
        let small = SeamFit {
            yaw_deg: -0.02,
            ..SeamFit::default()
        };
        assert!(
            distance(SeamFit::default(), big) > 20.0,
            "the big correction is not big: {:.1} steps",
            distance(SeamFit::default(), big)
        );
        for asked in [big, small] {
            let correction = Correction::none(&lenses);
            correction.ask(asked);
            let started = Instant::now();
            let deadline = started + Duration::from_secs(10);
            while correction.state().0 != asked && Instant::now() < deadline {
                correction.lenses();
                std::thread::sleep(Duration::from_millis(5));
            }
            let took = started.elapsed().as_secs_f64();
            assert_eq!(correction.state().0, asked, "the walk did not arrive");
            assert!(
                took < WALK_SECONDS * 2.0,
                "a correction of {:.1} steps took {took:.2} s",
                distance(SeamFit::default(), asked)
            );
        }
    }

    /// The walk arrives, and stops. A correction that crept towards its answer
    /// forever would leave the seam permanently a little wrong and permanently
    /// moving, which is both failures at once.
    ///
    /// The 0.94 probe steps between these two take 0.38 s at [`WALK_STEPS_S`],
    /// so the loop is given comfortably more than that and the assertion is
    /// that it finished, not when.
    #[test]
    fn the_walk_arrives_and_then_costs_nothing() {
        let lenses: Arc<[Lens]> = fixture_lenses().into();
        let asked = SeamFit {
            roll_deg: 0.05,
            yaw_deg: -0.08,
            ..SeamFit::default()
        };
        let correction = Correction::none(&lenses);
        correction.ask(asked);
        let deadline = Instant::now() + Duration::from_secs(5);
        while correction.state().0 != asked && Instant::now() < deadline {
            correction.lenses();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(correction.state(), (asked, asked));
    }

    /// The walk is a straight line in the five knobs, and at the sizes it
    /// walks over that is very nearly the picture a slerp of the rotation part
    /// would draw. The comparison is what licenses the simpler arithmetic, and
    /// what it licenses is stated as the number it measured rather than a
    /// bound chosen to pass.
    ///
    /// The two fits here are 0.48 degrees apart, which is larger than any
    /// correction playback asks for once a calibration has landed. Measured
    /// worst disagreement anywhere along the walk: **0.0120 degrees**. That is
    /// 0.20 of a view pixel at 16.8 px per degree, and it is under a sixth of
    /// [`Probe::step`], which is the finest shift the correlation that produced
    /// either fit can resolve. The bound asserted is a quarter of that step: a
    /// disagreement the instrument which produced the endpoints could not see.
    #[test]
    fn the_walk_is_a_straight_line_between_two_fits() {
        let from = SeamFit {
            roll_deg: 0.80,
            yaw_deg: -2.35,
            pitch_deg: -0.68,
            ..SeamFit::default()
        };
        let to = SeamFit {
            roll_deg: 0.94,
            yaw_deg: -2.60,
            pitch_deg: -0.30,
            ..SeamFit::default()
        };
        let bound = Probe::default().step / 4.0;
        let mut worst: f64 = 0.0;
        for tenth in 0..=10 {
            let fraction = f64::from(tenth) / 10.0;
            let straight = from.towards(to, fraction);
            let slerped = slerp(from, to, fraction);
            worst = worst.max(distance(straight, slerped) * Knob::Roll.probe());
        }
        assert!(worst < bound, "{worst:.5} deg apart, bound {bound:.5}");
        // A bound nothing can fail is not a bound: half a turn apart, the two
        // interpolations are nowhere near each other, and this is the same
        // comparison saying so.
        let far = SeamFit {
            roll_deg: -0.94,
            yaw_deg: 2.60,
            pitch_deg: 0.30,
            ..SeamFit::default()
        };
        let apart = distance(from.towards(far, 0.5), slerp(from, far, 0.5)) * Knob::Roll.probe();
        assert!(apart > bound, "the comparison cannot fail: {apart:.5} deg");
    }

    /// The rotation part of a fit, interpolated the way a rotation should be,
    /// so the straight line can be scored against something rather than
    /// asserted. Axis-angle through the small-angle composition the fit itself
    /// uses: the knobs are added to the calibration's own fields, so the
    /// rotation a fit names is the one those three fields name.
    fn slerp(from: SeamFit, to: SeamFit, fraction: f64) -> SeamFit {
        let axis = |fit: SeamFit| [fit.roll_deg, fit.yaw_deg, fit.pitch_deg];
        let (a, b) = (axis(from), axis(to));
        let (na, nb) = (norm(a), norm(b));
        if na < 1e-12 || nb < 1e-12 {
            return from.towards(to, fraction);
        }
        let (ua, ub) = (unit(a.map(f64::from)), unit(b.map(f64::from)));
        let cos = (ua[0] * ub[0] + ua[1] * ub[1] + ua[2] * ub[2]).clamp(-1.0, 1.0);
        let angle = cos.acos();
        let turned = match angle.abs() < 1e-9 {
            true => ua,
            false => {
                let (sa, sb) = (
                    ((1.0 - fraction) * angle).sin() / angle.sin(),
                    (fraction * angle).sin() / angle.sin(),
                );
                unit(std::array::from_fn(|i| sa * ua[i] + sb * ub[i]))
            }
        };
        let length = na + (nb - na) * fraction;
        SeamFit {
            roll_deg: turned[0] * length,
            yaw_deg: turned[1] * length,
            pitch_deg: turned[2] * length,
            cx_px: from.cx_px + (to.cx_px - from.cx_px) * fraction,
            cy_px: from.cy_px + (to.cy_px - from.cy_px) * fraction,
        }
    }
}
