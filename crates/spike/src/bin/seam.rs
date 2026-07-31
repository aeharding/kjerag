//! What the seam is misaligned by on a camera that is not moving, what shape
//! of calibration error would explain it, and what correcting it is worth
//! (issue #48).
//!
//! ```sh
//! # the residual round the seam circle, its structure, and the fit
//! cargo run --release -p kyerag-spike --bin seam -- <static.insv> both=1 \
//!   knobs=roll,yaw,pitch,cx,cy control=1 also=<other-static.insv>
//! # a fitted correction, applied and looked at with no blend to hide it
//! cargo run --release -p kyerag-spike --bin seam -- <file.insv> mode=render \
//!   yaw=90 bands=0 fix=roll:0.80,yaw:-2.29,pitch:-0.82,cx:-4.59,cy:-14.73
//! # what a narrower blend buys, and what it costs
//! cargo run --release -p kyerag-spike --bin seam -- <file.insv> mode=blend \
//!   yaw=90 pitch=-60 bands=14,8,4,2,1,0.5,0
//! # our stitch against the camera maker's own, on the same capture
//! cargo run --release -p kyerag-spike --bin seam -- <file.insv> mode=parity \
//!   against=<their-export.mp4>
//! ```
//!
//! Every earlier reading of this residual was taken in flight, where three
//! things move the two lenses' pictures apart and only one of them is
//! calibration: near-field parallax, the readout (issue #9), and the
//! calibration itself. A capture from a camera sitting still removes two of
//! them by physics rather than by argument, which is the retest
//! docs/research/insv-format.md 4.9 and 6.7 both asked for.
//!
//! The measurement is 4.9's, sharpened. Both lenses are sampled on the **same
//! angular grid** around directions on the seam great circle, so the shift
//! that best correlates between them is in degrees of world angle with no
//! rotation to undo, and it splits by construction into
//!
//! - **along** the seam circle, which parallax cannot reach: the baseline
//!   between the two lenses is perpendicular to every direction on that
//!   circle, so a subject's distance displaces it across the seam and never
//!   along it, whatever the distance;
//! - **across** the seam, which parallax owns, and which turned out to carry
//!   the bigger part of the answer: 2.4 to 2.7 degrees of it, which is not
//!   parallax and does not change with the scene.
//!
//! What is new here is the **structure**: the residual is measured round the
//! whole circle and decomposed into harmonics of the azimuth, and each
//! harmonic names a different calibration error. A relative rotation `w`
//! between the two lenses displaces a direction `d` on the circle by `w x d`,
//! whose along-seam component is exactly `w.z` for every direction on it, so
//!
//! - **constant along** is relative **roll**, and nothing else reaches that
//!   term: a tilt has no along-seam component at all;
//! - **one cycle along** is the **principal point**, whose shift is a fixed
//!   direction in the image plane and therefore a tangential displacement
//!   that turns once round the rim;
//! - **two cycles along** is the **focal aspect**, `fx` against `fy`, which
//!   maps the rim circle to an ellipse.
//!
//! The instrument does not assume any of that. It fits the correction through
//! the shipped map itself (`kyerag_render::Reframe`, the shader's own Rust
//! twin) by perturbing one calibration field at a time and reading what that
//! does to the same patches, so the answer comes out in the units `offset_v3`
//! writes and the analytic reading above is only the check on it.
//!
//! `control=1` is the answer to issue #45's lesson: known errors of the size
//! being measured are injected into the calibration and read back off the same
//! pixels. An instrument that cannot see a half degree of roll it put there
//! itself has not measured the half degree it is reporting. `also=` is the
//! other control and the stronger one: a second capture of a different scene,
//! measured on the same ring, because a calibration residual is fixed in the
//! camera's frame and everything else that could produce one is not.
//!
//! `mode=render` and `mode=blend` write PNGs, into gitignored `scratch/seam/`,
//! because a seam is a thing to look at as well as a number and a fitted
//! correction has to be looked at before it is believed. They are luma only:
//! a double image is geometry, and geometry is in the luma plane. Everything
//! else here prints numbers, and the footage stays on the box.

use std::path::{Path, PathBuf};

use kyerag_media::Fallible;
use kyerag_meta::{CalibrationSet, Lens, Mat3, Quat, Size as MetaSize};
use kyerag_render::{Camera, Held, Landing, Reframe, Sampling, Size};
use kyerag_spike::{Pair, Plane, Walk};

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    match options.mode {
        Mode::Residual => residual(&options),
        Mode::Render => render(&options),
        Mode::Blend => blend(&options),
        Mode::Parity => parity(&options),
    }
}

/// What this run is for.
enum Mode {
    /// The seam's own misalignment, round the circle, on a still camera.
    Residual,
    /// One view of one frame, written out, so a correction can be looked at.
    Render,
    /// What a narrower blend buys and costs, on content that crosses the seam.
    Blend,
    /// Our stitch against the camera maker's own, on the same capture.
    Parity,
}

/// The inter-lens baseline in millimetres, which is what sets parallax and is
/// in the file rather than estimated (docs/research/insv-format.md 6.1).
fn baseline_mm(calibration: &CalibrationSet) -> f64 {
    calibration
        .lenses
        .get(1)
        .map_or(0.0, |lens| norm(lens.pose.translation_m) * 1e3)
}

// ------------------------------------------------------------ the ring

/// One direction on the seam great circle, and the two axes of the sphere
/// there: along the circle towards increasing azimuth, and across it towards
/// the front lens.
#[derive(Clone, Copy)]
struct Where {
    phi: f64,
    centre: [f64; 3],
    along: [f64; 3],
    across: [f64; 3],
}

fn ring(patches: usize) -> Vec<Where> {
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

/// Which way is up in the camera body's frame, from the accelerometer.
///
/// Legitimate on this capture and on no other: a camera at rest measures
/// gravity and nothing else, and `gyro` reports 100 percent of this file's
/// samples inside the filter's own 0.20 g trust window. It is what turns the
/// across-seam readings into a parallax control, because it says which patches
/// are looking at the deck the camera is standing on and how far away that
/// deck is along each of them.
fn body_up(calibration: &CalibrationSet) -> Option<[f64; 3]> {
    let samples = calibration.imu.samples();
    if samples.is_empty() {
        return None;
    }
    let body_from_imu = calibration.body_from_imu();
    let mut sum = [0.0; 3];
    for sample in samples {
        let g = body_from_imu.mul_vec(sample.accel_g);
        for axis in 0..3 {
            sum[axis] += g[axis];
        }
    }
    Some(unit(sum))
}

/// Which way one lens is pointing, in body coordinates, read out of the map
/// rather than out of the calibration: the direction whose projection is
/// stationary under a small turn is the axis, and a bisection on the model's
/// own `axis` field is the cheapest way to it.
fn axis_of(reframe: &Reframe, lens: usize) -> [f64; 3] {
    let cosine = |v: [f64; 3]| f64::from(reframe.project(lens, unit(v).map(|c| c as f32)).axis);
    let mut best = [0.0, 0.0, 1.0];
    let mut step = 1.0;
    for _ in 0..40 {
        let mut improved = false;
        for axis in 0..3 {
            for sign in [1.0, -1.0] {
                let mut candidate = best;
                candidate[axis] += sign * step;
                if cosine(candidate) > cosine(best) {
                    best = unit(candidate);
                    improved = true;
                }
            }
        }
        if !improved {
            step *= 0.5;
        }
    }
    best
}

// ------------------------------------------------------------ the patches

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

    /// How much picture there is to correlate, in 8-bit codes. Flat sky
    /// correlates with anything.
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
/// is exactly the size this instrument is trying to resolve.
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
struct Found {
    along: f64,
    across: f64,
    r: f64,
    contrast: f64,
}

/// Every patch round the seam of one frame, under one calibration, in patch
/// order. `None` where a lens has no usable picture of that patch, or where
/// there is nothing in it to correlate.
/// Why a patch was not a patch, which on a ring that crosses a deck, a
/// treeline and a blank sky is most of them and is worth saying out loud.
#[derive(Default)]
struct Refused {
    /// One of the two lenses has no picture of the whole rectangle, which
    /// past about 6 degrees off the seam is every patch: the overlap band is
    /// only so wide, so near-field content that parallax has moved further
    /// than that is not in both pictures at all and no instrument can pair it.
    outside: usize,
    flat: usize,
    unlike: usize,
    pinned: usize,
}

fn measure(
    reframe: &Reframe,
    pair: &Pair,
    ring: &[Where],
    options: &Options,
    refused: &mut Refused,
) -> Vec<Option<Found>> {
    let step = options.step.to_radians();
    let half = (options.span.to_radians() / 2.0 / step) as isize;
    let search = (
        (options.along / options.step) as isize,
        (options.across / options.step) as isize,
    );
    ring.iter()
        .map(|at| {
            let Some(front) = sample(reframe, &pair.lenses[0], 0, at, (half, half), step) else {
                refused.outside += 1;
                return None;
            };
            if front.contrast() < options.contrast {
                refused.flat += 1;
                return None;
            }
            let Some(back) = sample(
                reframe,
                &pair.lenses[1],
                1,
                at,
                (half + search.0, half + search.1),
                step,
            ) else {
                refused.outside += 1;
                return None;
            };
            let (along, across, r) = best_shift(&front, &back, search)?;
            if r < options.keep {
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
/// two lenses' disagreement and cannot say which of them is wrong: a correction
/// of `+x` on lens 1 and one of `-x` on lens 0 are the same picture at the
/// seam. Reported that way round, a fitted number is a patch to lens 1's block
/// of the string the camera wrote.
#[derive(Clone, Copy, PartialEq)]
enum Knob {
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
    const ALL: [Self; 8] = [
        Self::Roll,
        Self::Yaw,
        Self::Pitch,
        Self::Cx,
        Self::Cy,
        Self::Fx,
        Self::Fy,
        Self::Xi,
    ];

    fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|knob| knob.name() == name)
    }

    fn name(self) -> &'static str {
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
    fn unit(self) -> &'static str {
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
    fn probe(self) -> f64 {
        match self {
            Self::Roll | Self::Yaw | Self::Pitch => 0.10,
            Self::Cx | Self::Cy => 4.0,
            Self::Fx | Self::Fy => 0.001,
            Self::Xi => 0.005,
        }
    }

    fn apply(self, lens: &mut Lens, amount: f64) {
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
fn turned(lenses: &[Lens], knob: Knob, amount: f64) -> Vec<Lens> {
    let mut lenses = lenses.to_vec();
    if let Some(lens) = lenses.get_mut(1) {
        knob.apply(lens, amount);
    }
    lenses
}

/// The map for one calibration: the camera left alone and the horizon
/// unlocked, so a view ray is a direction in the camera body's own frame and a
/// patch of the sphere is addressed by its angles.
fn mapped(lenses: &[Lens], frame: Size) -> Reframe {
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
/// Everything else in this file is one measurement compared against this.
fn moved(base: &Reframe, tweaked: &Reframe, lens: usize, at: &Where) -> Option<[f64; 2]> {
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

// ------------------------------------------------------------ the run

/// What one patch came to over the run, and what the geometry says about it.
struct Patch {
    at: Where,
    along: Vec<f64>,
    across: Vec<f64>,
    r: Vec<f64>,
    contrast: f64,
    /// `-dot(centre, up)`: 1 straight down, 0 at the horizontal, negative
    /// above it. The deck this camera stands on is a plane below it, so a
    /// patch's distance to that deck is the camera's height over this number,
    /// which makes the whole across-seam column a one-parameter prediction.
    below: f64,
}

impl Patch {
    fn mean_along(&self) -> f64 {
        mean(self.along.iter().copied())
    }

    fn mean_across(&self) -> f64 {
        mean(self.across.iter().copied())
    }

    fn frames(&self) -> usize {
        self.along.len()
    }
}

/// The first capture's calibration, the correction applied to it, its map, and
/// the injected candidates measured beside it. Every later section is fitted
/// against these, because the camera is the same in every capture and the
/// scene is not.
struct Fitted {
    calibration: CalibrationSet,
    lenses: Vec<Lens>,
    base: Reframe,
    candidates: Vec<(String, Reframe)>,
}

fn residual(options: &Options) -> Fallible<()> {
    let mut sweeps: Vec<(PathBuf, Vec<Vec<Patch>>)> = Vec::new();
    let mut first: Option<Fitted> = None;
    for path in options.inputs() {
        let calibration = CalibrationSet::from_insv(&path)?;
        let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
        let ring = ring(options.patches);
        let lenses = fixed(&calibration.lenses, &options.fix);
        let base = mapped(&lenses, frame);
        // Every candidate is measured on the same frames, and each is compared
        // against the measurement on the patches the two of them share: an
        // injected error moves the picture far enough to lose patches at the
        // edge of the overlap, and holding every candidate to one global
        // intersection empties it.
        let mut candidates: Vec<(String, Reframe)> = vec![("measured".to_owned(), base)];
        for (knob, amount) in options.injections() {
            candidates.push((
                format!("{}{:+.2}", knob.name(), amount),
                mapped(&turned(&lenses, knob, amount), frame),
            ));
        }
        announce(&calibration, &base, options, &path)?;
        let taken = sweep(&calibration, &ring, &candidates, options, &path)?;
        report(
            &taken[0]
                .iter()
                .filter(|p| p.frames() > 0)
                .collect::<Vec<_>>(),
            options,
        );
        sweeps.push((path, taken));
        if first.is_none() {
            first = Some(Fitted {
                calibration,
                lenses,
                base,
                candidates,
            });
        }
    }
    let Fitted {
        calibration,
        lenses,
        base,
        candidates,
    } = first.ok_or("no input file")?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let ring = ring(options.patches);

    if sweeps.len() > 1 {
        agreement(&sweeps);
    }
    // The two captures are pooled for the structure and the fit. They were
    // taken minutes apart with the camera picked up and put down between them,
    // so an azimuth one of them has no content at is often one the other does,
    // and a calibration residual is the same in both by definition.
    let pooled: Vec<&Patch> = sweeps
        .iter()
        .flat_map(|(_, taken)| taken[0].iter())
        .filter(|p| p.frames() > 0)
        .collect();
    if pooled.len() < 6 {
        return Err("too few patches correlated to say anything about structure".into());
    }
    println!(
        "\npooled: {} patch readings over {} capture(s)",
        pooled.len(),
        sweeps.len()
    );
    structure(&pooled);
    parallax(&pooled, baseline_mm(&calibration) / 1e3);
    signatures(&base, &lenses, frame, &ring, &Knob::ALL);
    correction(&pooled, &base, &lenses, frame, options);
    if candidates.len() > 1 {
        println!(
            "\nthe controls: a known error injected into lens 1's calibration and read back off \n\
             the same pixels, against what the map says it should read"
        );
        let (measured, injected) = sweeps[0].1.split_at(1);
        for ((name, reframe), with) in candidates.iter().skip(1).zip(injected) {
            control(name, &measured[0], with, &base, reframe);
        }
    }
    Ok(())
}

/// What the file is, where its lenses are pointing, and what the ring is.
fn announce(
    calibration: &CalibrationSet,
    base: &Reframe,
    options: &Options,
    path: &Path,
) -> Fallible<()> {
    let up = body_up(calibration).ok_or("this file carries no IMU record, so up is unknown")?;
    println!(
        "\n{}: {} {}, baseline {:.2} mm",
        path.file_name().unwrap_or_default().to_string_lossy(),
        calibration.camera_model,
        calibration.firmware,
        baseline_mm(calibration),
    );
    println!(
        "seam:   {} patches, {:.1} deg across, correlated over +/-{:.1} along and +/-{:.1} \
         across in {:.3} deg steps, kept above r={:.2}",
        options.patches, options.span, options.along, options.across, options.step, options.keep,
    );
    println!(
        "up:     body [{:+.4}, {:+.4}, {:+.4}], the lens axis {:.2} deg off the horizontal",
        up[0],
        up[1],
        up[2],
        up[2].asin().to_degrees().abs(),
    );
    // Where the composition of docs/research/insv-format.md 4.8 and 4.9 puts
    // the two lenses. The ring is the circle perpendicular to the body's z, so
    // if the two axes are not a half turn apart the ring is not the seam and
    // the difference shows up as one cycle of across-seam offset.
    let axes: Vec<[f64; 3]> = (0..2).map(|lens| axis_of(base, lens)).collect();
    println!(
        "axes:   lens 0 [{:+.4}, {:+.4}, {:+.4}], lens 1 [{:+.4}, {:+.4}, {:+.4}], \
         {:.3} deg from opposed",
        axes[0][0],
        axes[0][1],
        axes[0][2],
        axes[1][0],
        axes[1][1],
        axes[1][2],
        180.0 - dot(axes[0], axes[1]).clamp(-1.0, 1.0).acos().to_degrees(),
    );
    Ok(())
}

/// One file's ring, under every candidate calibration.
fn sweep(
    calibration: &CalibrationSet,
    ring: &[Where],
    candidates: &[(String, Reframe)],
    options: &Options,
    path: &Path,
) -> Fallible<Vec<Vec<Patch>>> {
    let up = body_up(calibration).ok_or("this file carries no IMU record, so up is unknown")?;
    let mut walk = Walk::open(path, options.from, calibration.dimension)?;
    if walk.streams() < 2 {
        return Err("this file carries one lens stream, so it has no seam".into());
    }
    let mut patches: Vec<Vec<Patch>> = candidates
        .iter()
        .map(|_| {
            ring.iter()
                .map(|at| Patch {
                    at: *at,
                    along: Vec::new(),
                    across: Vec::new(),
                    r: Vec::new(),
                    contrast: 0.0,
                    below: -dot(at.centre, up),
                })
                .collect()
        })
        .collect();
    let mut measured = 0usize;
    let mut refused = Refused::default();
    for _ in 0..options.count {
        let Some(pair) = walk.next_pair()? else {
            break;
        };
        let found: Vec<Vec<Option<Found>>> = candidates
            .iter()
            .map(|(_, reframe)| measure(reframe, &pair, ring, options, &mut refused))
            .collect();
        for (candidate, patches) in found.iter().zip(&mut patches) {
            for (index, found) in candidate.iter().enumerate() {
                let Some(found) = found.filter(|f| f.r >= options.keep) else {
                    continue;
                };
                patches[index].along.push(found.along);
                patches[index].across.push(found.across);
                patches[index].r.push(found.r);
                patches[index].contrast = found.contrast;
            }
        }
        measured += 1;
    }
    if measured == 0 {
        return Err("no frames were decoded at all".into());
    }
    println!(
        "frames: {measured}, {} patch tries: {} not in both pictures, {} too flat to \
         correlate, {} correlated under r={:.2}, {} peaked against the search limit\n",
        measured * ring.len() * candidates.len(),
        refused.outside,
        refused.flat,
        refused.unlike,
        options.keep,
        refused.pinned,
    );
    Ok(patches)
}

/// The same azimuths, two captures, two scenes: the control that says which of
/// these numbers belong to the camera.
///
/// A calibration residual is fixed in the camera's own frame and does not know
/// what the camera is looking at. Everything else that could produce a
/// disagreement at the seam does: parallax is the scene's distances, a false
/// correlation peak is the scene's texture. The camera was picked up and put
/// down between these two captures and they share no content at all, so an
/// azimuth where both of them read the same number is an azimuth where the
/// number is the camera's.
fn agreement(sweeps: &[(PathBuf, Vec<Vec<Patch>>)]) {
    let (first, second) = (&sweeps[0].1[0], &sweeps[1].1[0]);
    let mut rows: Vec<(f64, f64, f64, f64, f64)> = Vec::new();
    for (a, b) in first.iter().zip(second) {
        if a.frames() == 0 || b.frames() == 0 {
            continue;
        }
        rows.push((
            a.at.phi.to_degrees(),
            a.mean_along(),
            b.mean_along(),
            a.mean_across(),
            b.mean_across(),
        ));
    }
    println!("\nthe two captures at the azimuths both of them found content at:");
    if rows.is_empty() {
        println!("  they share no azimuth, so this control says nothing");
        return;
    }
    println!(
        "{:>6} {:>10} {:>10} {:>8} {:>10} {:>10} {:>8}",
        "phi", "along A", "along B", "apart", "across A", "across B", "apart"
    );
    for (phi, along_a, along_b, across_a, across_b) in &rows {
        println!(
            "{phi:>6.0} {along_a:>10.3} {along_b:>10.3} {:>8.3} {across_a:>10.3} \
             {across_b:>10.3} {:>8.3}",
            along_a - along_b,
            across_a - across_b,
        );
    }
    println!(
        "\ntwo scenes with nothing in common read the same seam: {:.3} deg apart along and \n\
         {:.3} deg across, root mean square, against residuals of {:.3} and {:.3} deg. what \n\
         parallax there is differs between them by the difference of their distances, so the \n\
         part that repeats is the camera's own.",
        rms(rows.iter().map(|r| r.1 - r.2)),
        rms(rows.iter().map(|r| r.3 - r.4)),
        rms(rows.iter().map(|r| r.1)),
        rms(rows.iter().map(|r| r.3)),
    );
}

/// Every patch, in azimuth order: what it read and how repeatable it was.
fn report(kept: &[&Patch], options: &Options) {
    println!(
        "{:>6} {:>7} {:>8} {:>8} {:>8} {:>8} {:>7} {:>6} {:>7}",
        "phi", "below", "along", "sd", "across", "sd", "r", "codes", "frames"
    );
    for patch in kept {
        println!(
            "{:>6.0} {:>7.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>7.3} {:>6.1} {:>7}",
            patch.at.phi.to_degrees(),
            patch.below,
            patch.mean_along(),
            spread(patch.along.iter().copied()),
            patch.mean_across(),
            spread(patch.across.iter().copied()),
            mean(patch.r.iter().copied()),
            patch.contrast,
            patch.frames(),
        );
    }
    let scatter = mean(kept.iter().map(|p| spread(p.along.iter().copied())));
    println!(
        "\nphi runs round the seam circle from the body's +x; below is -dot(centre, up), so 1 is \n\
         straight down at the deck and 0 is the horizontal. along and across are how far lens 1's \n\
         picture of the same directions sits from lens 0's, in degrees of world angle.\n\
         \n\
         the camera did not move, so the sd columns are the instrument's own repeatability and \n\
         nothing else: {scatter:.4} deg along over {} readings, at a {:.3} deg correlation step.",
        kept.iter().map(|p| p.frames()).sum::<usize>(),
        options.step,
    );
}

// ------------------------------------------------------------ the structure

/// A constant plus one and two cycles round the seam circle, least squares,
/// with what is left over after each order.
struct Harmonics {
    terms: [f64; 5],
    residual: [f64; 3],
}

fn harmonics(points: &[(f64, f64)]) -> Harmonics {
    let basis = |phi: f64| {
        [
            1.0,
            phi.cos(),
            phi.sin(),
            (2.0 * phi).cos(),
            (2.0 * phi).sin(),
        ]
    };
    let mut terms = [0.0; 5];
    let mut residual = [0.0; 3];
    for (order, residual) in residual.iter_mut().enumerate() {
        let width = 1 + 2 * order;
        let rows: Vec<(Vec<f64>, f64)> = points
            .iter()
            .map(|(phi, value)| (basis(*phi)[..width].to_vec(), *value))
            .collect();
        let Some(fit) = least_squares(&rows) else {
            continue;
        };
        terms[..width].copy_from_slice(&fit.params);
        *residual = fit.residual;
    }
    Harmonics { terms, residual }
}

impl Harmonics {
    /// The amplitude and phase of one cycle count, the phase being the azimuth
    /// the term is largest at.
    fn cycle(&self, order: usize) -> (f64, f64) {
        let (cos, sin) = (self.terms[order * 2 - 1], self.terms[order * 2]);
        (cos.hypot(sin), sin.atan2(cos).to_degrees() / order as f64)
    }
}

/// What each knob would look like if it were the whole answer: the same
/// harmonic decomposition, run over the model's own prediction round the
/// **whole** ring rather than over the patches that happened to correlate.
///
/// This is what turns the measured structure into an attribution. Two knobs
/// that move the same axis by the same amount are told apart by how much of
/// the *other* axis they move with it, and by which cycle count they land in;
/// reading those ratios off the shipped map beats deriving them, because the
/// map is what the picture is made with.
fn signatures(base: &Reframe, lenses: &[Lens], frame: Size, ring: &[Where], knobs: &[Knob]) {
    println!("\nwhat each knob would look like, per unit, through the map:");
    println!(
        "{:<8} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "knob", "unit", "along 0", "along 1", "along 2", "across 0", "across 1", "across 2"
    );
    for knob in knobs {
        let probe = mapped(&turned(lenses, *knob, knob.probe()), frame);
        let shifts: Vec<(f64, [f64; 2])> = ring
            .iter()
            .filter_map(|at| Some((at.phi, moved(base, &probe, 1, at)?)))
            .map(|(phi, shift)| (phi, shift.map(|c| c / knob.probe())))
            .collect();
        let along = harmonics(
            &shifts
                .iter()
                .map(|(phi, shift)| (*phi, shift[0]))
                .collect::<Vec<_>>(),
        );
        let across = harmonics(
            &shifts
                .iter()
                .map(|(phi, shift)| (*phi, shift[1]))
                .collect::<Vec<_>>(),
        );
        println!(
            "{:<8} {:>9} {:>9.4} {:>9.4} {:>9.4} {:>9.4} {:>9.4} {:>9.4}",
            knob.name(),
            knob.unit(),
            along.terms[0],
            along.cycle(1).0,
            along.cycle(2).0,
            across.terms[0],
            across.cycle(1).0,
            across.cycle(2).0,
        );
    }
    println!(
        "columns are the constant, one cycle and two cycles round the seam circle, in degrees of \n\
         displacement per unit of the knob. a knob is identified by its whole row, not by one \n\
         cell: the ratio between the two axes at the same cycle count is what tells a lens tilt \n\
         from a principal point, since only one of them reaches along the seam."
    );
}

fn structure(kept: &[&Patch]) {
    let along = harmonics(
        &kept
            .iter()
            .map(|p| (p.at.phi, p.mean_along()))
            .collect::<Vec<_>>(),
    );
    let across = harmonics(
        &kept
            .iter()
            .map(|p| (p.at.phi, p.mean_across()))
            .collect::<Vec<_>>(),
    );
    println!("\nthe structure round the circle, least squares on the patch means:");
    println!(
        "{:<26} {:>10} {:>10} {:>10} {:>10}",
        "term", "along", "phase", "across", "phase"
    );
    println!(
        "{:<26} {:>10.3} {:>10} {:>10.3} {:>10}",
        "constant (relative roll)", along.terms[0], "", across.terms[0], "",
    );
    for (order, what) in [
        (1, "one cycle (principal pt)"),
        (2, "two cycles (focal aspect)"),
    ] {
        let (amplitude_along, phase_along) = along.cycle(order);
        let (amplitude_across, phase_across) = across.cycle(order);
        println!(
            "{what:<26} {amplitude_along:>10.3} {phase_along:>10.0} \
             {amplitude_across:>10.3} {phase_across:>10.0}"
        );
    }
    println!(
        "{:<26} {:>10.3} {:>10} {:>10.3} {:>10}",
        "left after constant", along.residual[0], "", across.residual[0], "",
    );
    println!(
        "{:<26} {:>10.3} {:>10} {:>10.3} {:>10}",
        "left after one cycle", along.residual[1], "", across.residual[1], "",
    );
    println!(
        "{:<26} {:>10.3} {:>10} {:>10.3} {:>10}",
        "left after two cycles", along.residual[2], "", across.residual[2], "",
    );
    println!(
        "\nalong the seam parallax cannot reach, so every term in that column is calibration. a \n\
         relative rotation w displaces a seam direction by w x d, whose along-seam component is \n\
         w.z whatever the direction: constant along IS relative roll, and a lens tilt cannot \n\
         reach that column at all. across carries parallax as well, which is what the next \n\
         section separates."
    );
}

// ------------------------------------------------------------ the controls

/// Parallax, predicted from one number and checked against the across-seam
/// column.
///
/// The camera stands on a deck, which is a plane a fixed height under it, so
/// the distance along a patch's own direction is the height over `below` and
/// the disparity is the baseline over that distance. One free parameter, the
/// height, against a column that runs from nothing at the horizontal to
/// degrees at the deck: if the fitted height is a camera's height and the fit
/// is tight, the across-seam axis is reading real parallax at the size real
/// parallax has, which is the control that makes the along-seam column's
/// silence worth something.
fn parallax(kept: &[&Patch], baseline_m: f64) {
    let rows: Vec<(Vec<f64>, f64)> = kept
        .iter()
        .filter(|p| p.below > 0.15)
        .map(|p| (vec![p.below], p.mean_across().to_radians()))
        .collect();
    println!("\nthe parallax control: the deck as one plane under the camera");
    if rows.len() < 3 {
        println!("  too few patches are looking at the deck to fit it");
        return;
    }
    let Some(fit) = least_squares(&rows) else {
        return;
    };
    let height = baseline_m / fit.params[0].abs();
    let above = kept.iter().filter(|p| p.below < -0.15).collect::<Vec<_>>();
    println!(
        "  {} patches below the horizontal fit disparity = {:+.4} rad per unit of `below`, \n\
         which is a baseline of {:.1} mm at a camera height of {:.0} mm, leaving {:.3} deg",
        rows.len(),
        fit.params[0],
        baseline_m * 1e3,
        height * 1e3,
        fit.residual.to_degrees(),
    );
    println!(
        "  the {} patches ABOVE the horizontal, where there is no plane and the content is far, \n\
         read {:+.3} deg across on average: that is the same column with the parallax taken away",
        above.len(),
        mean(above.iter().map(|p| p.mean_across())),
    );
}

/// An injected calibration error, read back off the same pixels.
///
/// The point of issue #45: a control has to be able to catch the failure it is
/// clearing. What is injected here is the size of the thing being reported, so
/// a slope of one says this instrument can see an error of that size and shape
/// on these pixels, and the measurement's own numbers mean what they say.
fn control(name: &str, base: &[Patch], with: &[Patch], reframe: &Reframe, tweaked: &Reframe) {
    let mut rows: Vec<(Vec<f64>, f64)> = Vec::new();
    let mut across: Vec<(Vec<f64>, f64)> = Vec::new();
    for (before, after) in base.iter().zip(with) {
        if before.frames() == 0 || after.frames() == 0 {
            continue;
        }
        let Some(predicted) = moved(reframe, tweaked, 1, &before.at) else {
            continue;
        };
        rows.push((vec![predicted[0]], after.mean_along() - before.mean_along()));
        across.push((
            vec![predicted[1]],
            after.mean_across() - before.mean_across(),
        ));
    }
    let (Some(along_fit), Some(across_fit)) = (least_squares(&rows), least_squares(&across)) else {
        return;
    };
    println!(
        "  {name:<12} along {:>6.3} of predicted (r {:>6.3}, spread {:.3} deg), \
         across {:>6.3} (r {:>6.3}, spread {:.3} deg)",
        along_fit.params[0],
        correlation(&rows),
        spread(rows.iter().map(|(x, _)| x[0])),
        across_fit.params[0],
        correlation(&across),
        spread(across.iter().map(|(x, _)| x[0])),
    );
}

// ------------------------------------------------------------ the correction

/// The calibration correction that would flatten what was measured, fitted
/// through the shipped map.
///
/// Each knob is turned by its own probe amount and the map is asked what that
/// does to every patch, which is a column of the design matrix in the units
/// `offset_v3` writes. The fit is on the **along-seam** column alone, because
/// that is the one parallax cannot reach; the across-seam column is then
/// predicted from the fitted parameters and compared against what was
/// measured, and the difference is what parallax and the scene owe.
fn correction(kept: &[&Patch], base: &Reframe, lenses: &[Lens], frame: Size, options: &Options) {
    let knobs = &options.knobs;
    let probes: Vec<Reframe> = knobs
        .iter()
        .map(|knob| mapped(&turned(lenses, *knob, knob.probe()), frame))
        .collect();
    let mut rows: Vec<(Vec<f64>, f64)> = Vec::new();
    let mut leverage: Vec<Vec<f64>> = vec![Vec::new(); knobs.len()];
    // The same knobs' effect on the other axis, kept so the fitted correction
    // can be asked what it predicts there and the rest handed to parallax.
    let mut sideways: Vec<(Vec<f64>, f64)> = Vec::new();
    for patch in kept {
        let mut row = Vec::with_capacity(knobs.len());
        let mut across = Vec::with_capacity(knobs.len());
        for (index, probe) in probes.iter().enumerate() {
            let Some(shift) = moved(base, probe, 1, &patch.at) else {
                row.clear();
                break;
            };
            row.push(shift[0] / knobs[index].probe());
            across.push(shift[1] / knobs[index].probe());
        }
        if row.len() != knobs.len() {
            continue;
        }
        for (index, value) in row.iter().enumerate() {
            leverage[index].push(*value);
        }
        // The correction is what has to be ADDED to the calibration to bring
        // the disagreement to zero, so the target is the negative of it.
        if options.both {
            // The across axis carries parallax as well as calibration, so it
            // is in the fit only when asked for and only on far content: on
            // this ring the far-field disparity is a tenth of a degree
            // against a residual of two, which the section above measures
            // rather than assumes.
            rows.push((across.clone(), -patch.mean_across()));
        }
        rows.push((row, -patch.mean_along()));
        sideways.push((across, patch.mean_across()));
    }
    println!(
        "\nthe correction that would flatten the {} column(s), fitted through the map:",
        match options.both {
            true => "along-seam and across-seam",
            false => "along-seam",
        },
    );
    let Some(fit) = least_squares(&rows) else {
        println!("  the fit is singular: these knobs are not separable on this ring");
        return;
    };
    println!(
        "{:<8} {:>12} {:>12} {:>14} {:>10}",
        "knob", "correction", "+/-", "unit", "leverage"
    );
    for (index, knob) in knobs.iter().enumerate() {
        println!(
            "{:<8} {:>12.4} {:>12.4} {:>14} {:>10.3}",
            knob.name(),
            fit.params[index],
            fit.errors[index],
            knob.unit(),
            rms(leverage[index].iter().copied()),
        );
    }
    println!(
        "\nalong-seam residual: {:.3} deg before, {:.3} deg predicted after",
        rms(kept.iter().map(|p| p.mean_along())),
        fit.residual,
    );
    let left = rms(sideways.iter().map(|(basis, measured)| {
        measured
            + basis
                .iter()
                .zip(&fit.params)
                .map(|(b, p)| b * p)
                .sum::<f64>()
    }));
    println!(
        "across-seam:         {:.3} deg before, {:.3} deg left once the same correction is \
         applied to it",
        rms(kept.iter().map(|p| p.mean_across())),
        left,
    );
    println!(
        "leverage is how many degrees of along-seam shift one unit of that knob is worth on this \n\
         ring, root mean square: a knob with none of it cannot be fitted from this axis whatever \n\
         the number beside it says, and the +/- column is where that shows up. what the \n\
         correction leaves across the seam is not an error: parallax lives there, and the \n\
         section above says how much of it this scene has."
    );
}

// ------------------------------------------------------------ least squares

struct Fit {
    params: Vec<f64>,
    errors: Vec<f64>,
    residual: f64,
}

/// Ordinary least squares through the normal equations, with the standard
/// error of each parameter. Small systems only, which every fit here is.
fn least_squares(rows: &[(Vec<f64>, f64)]) -> Option<Fit> {
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

/// Pearson's r between the one basis column and the value, which is the
/// statistic a control is read by.
fn correlation(rows: &[(Vec<f64>, f64)]) -> f64 {
    let count = rows.len() as f64;
    if count < 3.0 {
        return 0.0;
    }
    let mean_x = rows.iter().map(|(x, _)| x[0]).sum::<f64>() / count;
    let mean_y = rows.iter().map(|(_, y)| y).sum::<f64>() / count;
    let (mut covariance, mut var_x, mut var_y) = (0.0, 0.0, 0.0);
    for (x, y) in rows {
        let (x, y) = (x[0] - mean_x, y - mean_y);
        covariance += x * y;
        var_x += x * x;
        var_y += y * y;
    }
    match var_x > 0.0 && var_y > 0.0 {
        true => covariance / (var_x * var_y).sqrt(),
        false => 0.0,
    }
}

// ------------------------------------------------------------ plumbing

struct Options {
    mode: Mode,
    input: PathBuf,
    /// A second capture of a different scene, measured on the same ring. It is
    /// the control that separates the camera from what it is looking at.
    also: Option<PathBuf>,
    /// The camera maker's own export of the same capture, for `mode=parity`.
    against: Option<PathBuf>,
    from: f64,
    count: usize,
    patches: usize,
    /// How wide a patch is, in degrees of world angle.
    span: f64,
    /// How finely the correlation is stepped, in degrees.
    step: f64,
    /// How far the correlation looks along the seam, and across it. They
    /// differ because parallax only reaches one of them, and reaches it by
    /// degrees in the near field.
    along: f64,
    across: f64,
    keep: f64,
    contrast: f64,
    knobs: Vec<Knob>,
    /// Whether the across-seam readings are in the fit as well as the
    /// along-seam ones. Off by default because parallax lives on that axis.
    both: bool,
    control: bool,
    /// A correction applied to lens 1 before anything is measured or drawn,
    /// which is how a fitted answer is checked: apply it, measure again, and
    /// the residual it was fitted to should be gone.
    fix: Vec<(Knob, f64)>,
    yaw: f64,
    pitch: f64,
    fov: f64,
    size: u32,
    /// The blend widths compared, in degrees, plus the shipped weights.
    bands: Vec<f64>,
    out: Option<PathBuf>,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Fallible<Self> {
        let input = PathBuf::from(args.next().ok_or(USAGE)?);
        let mut options = Self {
            mode: Mode::Residual,
            input,
            also: None,
            against: None,
            from: 0.0,
            count: 6,
            patches: 72,
            span: 3.7,
            step: 0.08,
            along: 2.0,
            across: 4.0,
            keep: 0.80,
            contrast: 6.0,
            knobs: vec![Knob::Roll, Knob::Cx, Knob::Cy],
            both: false,
            control: false,
            fix: Vec::new(),
            yaw: 90.0,
            pitch: 0.0,
            fov: 50.0,
            size: 1024,
            bands: vec![14.0, 8.0, 4.0, 2.0, 1.0, 0.0],
            out: None,
        };
        for arg in args {
            let (key, value) = arg.split_once('=').ok_or(USAGE)?;
            match key {
                "mode" => {
                    options.mode = match value {
                        "residual" => Mode::Residual,
                        "render" => Mode::Render,
                        "blend" => Mode::Blend,
                        "parity" => Mode::Parity,
                        _ => return Err(format!("no mode called {value}. {USAGE}").into()),
                    };
                }
                "also" => options.also = Some(PathBuf::from(value)),
                "against" => options.against = Some(PathBuf::from(value)),
                "yaw" => options.yaw = value.parse()?,
                "pitch" => options.pitch = value.parse()?,
                "fov" => options.fov = value.parse()?,
                "size" => options.size = value.parse()?,
                "out" => options.out = Some(PathBuf::from(value)),
                "bands" => {
                    options.bands = value
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<Vec<f64>, _>>()?;
                }
                "fix" => options.fix = turns(value)?,
                "from" => options.from = value.parse()?,
                "count" => options.count = value.parse()?,
                "patches" => options.patches = value.parse()?,
                "span" => options.span = value.parse()?,
                "step" => options.step = value.parse()?,
                "along" => options.along = value.parse()?,
                "across" => options.across = value.parse()?,
                "keep" => options.keep = value.parse()?,
                "contrast" => options.contrast = value.parse()?,
                "both" => options.both = value.parse::<u32>()? != 0,
                "control" => options.control = value.parse::<u32>()? != 0,
                "knobs" => {
                    options.knobs = value
                        .split(',')
                        .map(|name| Knob::parse(name).ok_or(format!("no knob called {name}")))
                        .collect::<Result<Vec<_>, _>>()?;
                }
                _ => return Err(format!("unknown argument {key}. {USAGE}").into()),
            }
        }
        Ok(options)
    }

    fn camera(&self) -> Camera {
        Camera {
            yaw: (self.yaw as f32).to_radians(),
            pitch: (self.pitch as f32).to_radians(),
            fov: (self.fov as f32).to_radians(),
        }
    }

    /// The shipped weights first, then every band width asked for.
    fn weightings(&self) -> Vec<Weighting> {
        let mut all = vec![Weighting::Shipped];
        all.extend(self.bands.iter().map(|width| Weighting::Band(*width)));
        all
    }

    fn weighting(&self) -> Weighting {
        match self.bands.len() {
            1 => Weighting::Band(self.bands[0]),
            _ => Weighting::Shipped,
        }
    }

    fn out_dir(&self) -> PathBuf {
        PathBuf::from("scratch/seam")
    }

    fn out(&self) -> PathBuf {
        self.out_dir().join(self.out.clone().unwrap_or_else(|| {
            PathBuf::from(format!(
                "seam-yaw{:.0}-pitch{:.0}-fov{:.0}.png",
                self.yaw, self.pitch, self.fov
            ))
        }))
    }

    fn inputs(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.input.clone()];
        paths.extend(self.also.clone());
        paths
    }

    /// The known errors injected for `control=1`, sized to the regime being
    /// measured: the residual this instrument is reporting is a fraction of a
    /// degree, so the controls are fractions of a degree.
    fn injections(&self) -> Vec<(Knob, f64)> {
        match self.control {
            true => vec![
                (Knob::Roll, 0.50),
                (Knob::Roll, -0.25),
                (Knob::Yaw, 0.50),
                (Knob::Cx, 20.0),
            ],
            false => Vec::new(),
        }
    }
}

/// `roll:0.79,yaw:-2.1` and the like: a list of knobs and how far to turn
/// each, in that knob's own units.
fn turns(value: &str) -> Fallible<Vec<(Knob, f64)>> {
    value
        .split(',')
        .map(|term| {
            let (name, amount) = term.split_once(':').ok_or("a fix is knob:amount")?;
            Ok((
                Knob::parse(name).ok_or(format!("no knob called {name}"))?,
                amount.parse()?,
            ))
        })
        .collect()
}

const USAGE: &str = "usage: seam <file.insv> [mode=residual|render|blend] [also=<other.insv>] \
     [fix=roll:0.8,yaw:-2] [yaw=deg] [pitch=deg] [fov=deg] [size=px] [bands=14,8,4] [out=x.png] \
     [from=seconds] [count=frames] [patches=n] \
     [span=deg] [step=deg] [along=deg] [across=deg] [keep=r] [contrast=codes] \
     [knobs=roll,cx,cy,...] [control=1]";

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let values: Vec<f64> = values.collect();
    match values.is_empty() {
        true => 0.0,
        false => values.iter().sum::<f64>() / values.len() as f64,
    }
}

fn spread(values: impl Iterator<Item = f64>) -> f64 {
    let values: Vec<f64> = values.collect();
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt()
}

fn rms(values: impl Iterator<Item = f64>) -> f64 {
    let values: Vec<f64> = values.collect();
    match values.is_empty() {
        true => 0.0,
        false => (values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64).sqrt(),
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|axis| a[axis] * b[axis]).sum()
}

fn norm(v: [f64; 3]) -> f64 {
    dot(v, v).sqrt()
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let length = norm(v).max(f64::MIN_POSITIVE);
    v.map(|c| c / length)
}

// ------------------------------------------------------------ the blend

/// How the two lenses' claims are weighed against each other across the
/// overlap.
///
/// The shipped one carries no width: the band comes out as the overlap itself,
/// 14 degrees wide on this camera, because the coverage depth only reaches
/// zero where a lens runs out of picture (docs/research/insv-format.md 6.6).
/// [`Band`] is the experiment the owner's complaint asks for, a crossover of a
/// stated width centred on the seam, and it lives here rather than in the
/// shader because phase 1 is measurement.
#[derive(Clone, Copy)]
enum Weighting {
    Shipped,
    /// A linear ramp `width` degrees wide, centred on the seam great circle.
    /// Zero is a hard cut.
    Band(f64),
    /// One lens alone wherever it has any picture: the sharp reference every
    /// blended band is measured against, since a single lens is never doubled.
    Single(usize),
}

impl Weighting {
    /// The two lenses' shares of one ray, and where each of them reads it.
    fn at(self, reframe: &Reframe, ray: [f64; 3]) -> ([f64; 2], [Landing; 2]) {
        let shipped = reframe.blend(ray.map(|c| c as f32));
        let landings = shipped.landings;
        let covered = |lens: usize| landings[lens].inside;
        let weights = match self {
            Self::Shipped => shipped.weights.map(f64::from),
            Self::Single(lens) => {
                let mut weights = [0.0; 2];
                weights[lens] = f64::from(u8::from(covered(lens)));
                weights
            }
            Self::Band(width) => {
                // How far the ray is past the seam, towards the back lens, so
                // the front lens's share falls as it grows.
                let past = past_seam(reframe, ray);
                let front = match width > 0.0 {
                    true => (0.5 - past / width).clamp(0.0, 1.0),
                    false => f64::from(u8::from(past < 0.0)),
                };
                let mut weights = [front, 1.0 - front];
                for (lens, weight) in weights.iter_mut().enumerate() {
                    if !covered(lens) {
                        *weight = 0.0;
                    }
                }
                let total: f64 = weights.iter().sum();
                match total > 0.0 {
                    true => weights.map(|w| w / total),
                    false => [0.0; 2],
                }
            }
        };
        (weights, landings)
    }
}

/// How far past the seam a ray looks, in degrees: negative in the front
/// lens's hemisphere, zero where the two lenses are equally far off axis, and
/// positive behind it.
///
/// Written as the difference of the two lenses' own angles rather than as
/// "ninety degrees off the front one", so that it still names the crossover
/// when the two axes are not exactly opposed. That is not a hypothetical: the
/// correction this instrument fits moves one axis by a couple of degrees, and
/// a band centred on the front lens alone would then sit off the overlap.
fn past_seam(reframe: &Reframe, ray: [f64; 3]) -> f64 {
    let off = |lens: usize| {
        f64::from(reframe.project(lens, ray.map(|c| c as f32)).axis)
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
    };
    0.5 * (off(0) - off(1))
}

/// One rendered view, in luma alone.
///
/// Colour would need the chroma plane and a colour transform; a double image
/// is a geometry question and shows in luma, which is also what every measure
/// below is computed on.
struct View {
    size: u32,
    luma: Vec<f64>,
    /// Per pixel, how far past the seam it looks, in degrees: negative in the
    /// front lens's hemisphere. What the band profiles are binned by.
    past: Vec<f64>,
    /// Per pixel, the smaller of the two weights, which is how doubled that
    /// pixel is: 0 is one lens alone and 0.5 is an even mix of two.
    mixed: Vec<f64>,
    /// Whether both lenses have this ray at all.
    ///
    /// Every measure below is taken inside this mask and no measure is taken
    /// across its edge. Where a lens's picture stops, an unmasked gradient
    /// reads the edge of the picture rather than the picture: one column of
    /// black against ordinary daylight is a squared step of fourteen thousand
    /// against a texture's few hundred, which is enough to treble a mean over
    /// sixty thousand pixels and did.
    both: Vec<bool>,
}

impl View {
    fn paint(reframe: &Reframe, pair: &Pair, weighting: Weighting, size: u32) -> Self {
        let mut view = Self {
            size,
            luma: Vec::with_capacity((size * size) as usize),
            past: Vec::with_capacity((size * size) as usize),
            mixed: Vec::with_capacity((size * size) as usize),
            both: Vec::with_capacity((size * size) as usize),
        };
        for y in 0..size {
            for x in 0..size {
                let uv = [
                    (x as f32 + 0.5) / size as f32,
                    (y as f32 + 0.5) / size as f32,
                ];
                // The camera is baked into the map, so a view ray is all the
                // pass itself is ever handed. At this instrument's fields of
                // view every pixel has a ray; the void case exists only past
                // the tiny planet, so an empty ray just keeps the grids
                // aligned.
                let Some(ray) = reframe.view_ray(uv) else {
                    view.luma.push(0.0);
                    view.past.push(0.0);
                    view.mixed.push(0.0);
                    view.both.push(false);
                    continue;
                };
                let ray = ray.map(f64::from);
                let (weights, landings) = weighting.at(reframe, unit(ray));
                let mut luma = 0.0;
                let mut total = 0.0;
                for lens in 0..2 {
                    if weights[lens] <= 0.0 {
                        continue;
                    }
                    let landing = landings[lens];
                    let Some(code) = pair.lenses[lens]
                        .at(f64::from(landing.pixel[0]), f64::from(landing.pixel[1]))
                    else {
                        continue;
                    };
                    luma += weights[lens] * code;
                    total += weights[lens];
                }
                view.luma.push(match total > 0.0 {
                    true => luma / total,
                    false => 0.0,
                });
                view.past.push(past_seam(reframe, unit(ray)));
                view.mixed.push(weights[0].min(weights[1]));
                view.both.push(landings[0].inside && landings[1].inside);
            }
        }
        view
    }

    fn write(&self, path: &Path) -> Fallible<()> {
        let pixels: Vec<u8> = self
            .luma
            .iter()
            .map(|code| code.clamp(0.0, 255.0) as u8)
            .collect();
        let mut png = png::Encoder::new(
            std::io::BufWriter::new(std::fs::File::create(path)?),
            self.size,
            self.size,
        );
        png.set_color(png::ColorType::Grayscale);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()?.write_image_data(&pixels)?;
        Ok(())
    }

    /// Mean squared gradient across the picture, over the pixels whose
    /// distance past the seam falls in `band`.
    ///
    /// A doubled edge is a blurred edge, and blur is exactly what a gradient
    /// measures. Taken across the picture rather than down it because a
    /// seam-centred view runs the seam down the frame and the doubling is
    /// across it, which is the axis parallax and the calibration residual both
    /// displace along.
    fn sharpness(&self, band: (f64, f64)) -> f64 {
        let mut total = 0.0;
        let mut count = 0.0;
        for y in 0..self.size as usize {
            for x in 1..self.size as usize - 1 {
                let index = y * self.size as usize + x;
                if self.past[index] < band.0 || self.past[index] > band.1 {
                    continue;
                }
                if !(self.both[index - 1] && self.both[index] && self.both[index + 1]) {
                    continue;
                }
                let step = self.luma[index + 1] - self.luma[index - 1];
                total += step * step;
                count += 1.0;
            }
        }
        match count > 0.0 {
            true => total / count,
            false => 0.0,
        }
    }

    /// How many pixels the band above is measured over, which is the check
    /// that two runs are being compared on the same picture.
    fn counted(&self, band: (f64, f64)) -> usize {
        self.past
            .iter()
            .zip(&self.both)
            .filter(|(past, both)| **both && **past >= band.0 && **past <= band.1)
            .count()
    }

    /// How wide the doubled band is, in degrees: the span of `past` over which
    /// both lenses are contributing more than `floor` of the picture.
    fn doubled(&self, floor: f64) -> f64 {
        let past: Vec<f64> = self
            .past
            .iter()
            .zip(&self.mixed)
            .filter(|(_, mixed)| **mixed > floor)
            .map(|(past, _)| *past)
            .collect();
        match past.is_empty() {
            true => 0.0,
            false => {
                let low = past.iter().copied().fold(f64::MAX, f64::min);
                let high = past.iter().copied().fold(f64::MIN, f64::max);
                high - low
            }
        }
    }
}

/// One frame of one file, decoded, plus the map that reads it.
fn frame_at(options: &Options, path: &Path) -> Fallible<(CalibrationSet, Pair)> {
    let calibration = CalibrationSet::from_insv(path)?;
    let mut walk = Walk::open(path, options.from, calibration.dimension)?;
    if walk.streams() < 2 {
        return Err("this file carries one lens stream, so it has no seam".into());
    }
    let pair = walk.next_pair()?.ok_or("no frame decoded")?;
    Ok((calibration, pair))
}

/// The map for one view of one calibration, the camera baked in.
fn viewed(lenses: &[Lens], frame: Size, camera: Camera) -> Reframe {
    Reframe::new(
        lenses,
        frame,
        camera,
        Held::default(),
        1.0,
        false,
        Sampling::default(),
    )
}

fn render(options: &Options) -> Fallible<()> {
    let (calibration, pair) = frame_at(options, &options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = fixed(&calibration.lenses, &options.fix);
    let reframe = viewed(&lenses, frame, options.camera());
    let view = View::paint(&reframe, &pair, options.weighting(), options.size);
    let out = options.out();
    view.write(&out)?;
    println!(
        "wrote {} at yaw {:.1}, pitch {:.1}, fov {:.1}, {}",
        out.display(),
        options.yaw,
        options.pitch,
        options.fov,
        options.weighting().name(),
    );
    Ok(())
}

/// What a narrower blend buys and what it costs, on real content that crosses
/// the seam.
///
/// Two numbers per width, measured on the same pixels. **Doubled** is how many
/// degrees of the picture have both lenses in it, which is the extent of the
/// ghost; **sharpness** is the picture's own gradient energy over that band
/// against the same band rendered from one lens alone, which is 1.0 when
/// nothing is doubled and falls as the two copies pull apart. The third
/// number is not measured but arithmetic: a disparity of `d` degrees crossed
/// in a band `w` degrees wide shears the picture by `d / w`, and at `d / w`
/// above 1 the band is folded rather than blended.
fn blend(options: &Options) -> Fallible<()> {
    let (calibration, pair) = frame_at(options, &options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = fixed(&calibration.lenses, &options.fix);
    let reframe = viewed(&lenses, frame, options.camera());
    let ring = ring(options.patches);

    // The band the sharpness is measured over: the whole overlap, so every
    // width is scored on the same pixels rather than on its own band.
    let band = (-7.0, 7.0);
    let single = View::paint(&reframe, &pair, Weighting::Single(0), options.size);
    let reference = single.sharpness(band);

    // What the two lenses actually disagree by where this view crosses the
    // seam, which is what the widths below are trading against. Measured in
    // the body's own frame, like every other reading in this file, and then
    // restricted to the azimuths this view can see.
    let body = mapped(&lenses, frame);
    let seen: Vec<Where> = ring
        .iter()
        .filter(|at| in_view(options, at.centre))
        .copied()
        .collect();
    let mut refused = Refused::default();
    let found = measure(&body, &pair, &seen, options, &mut refused);
    let disparities: Vec<f64> = found
        .iter()
        .flatten()
        .filter(|f| f.r >= options.keep)
        .map(|f| f.along.hypot(f.across))
        .collect();
    let disparity = mean(disparities.iter().copied());
    println!(
        "view:   yaw {:.1}, pitch {:.1}, fov {:.1}, {} azimuths on the seam in it, {} of them \
         correlated",
        options.yaw,
        options.pitch,
        options.fov,
        seen.len(),
        disparities.len(),
    );
    println!(
        "seam:   the two lenses disagree by {disparity:.2} deg here on average, {:.2} at worst",
        disparities.iter().copied().fold(0.0, f64::max),
    );
    println!(
        "single: the front lens alone over the same band carries {reference:.1} of gradient \
         energy over {} pixels, which is what the sharpness column is a share of",
        single.counted(band),
    );
    println!(
        "\n{:>10} {:>10} {:>12} {:>10} {:>10}",
        "band", "doubled", "sharpness", "shear", "png"
    );
    for weighting in options.weightings() {
        let view = View::paint(&reframe, &pair, weighting, options.size);
        let doubled = view.doubled(0.1);
        let name = format!("seam-{}.png", weighting.name().replace(' ', "-"));
        view.write(&options.out_dir().join(&name))?;
        println!(
            "{:>10} {doubled:>10.2} {:>12.3} {:>10} {name:>10}",
            weighting.name(),
            match reference > 0.0 {
                true => view.sharpness(band) / reference,
                false => 0.0,
            },
            match doubled > 0.0 {
                true => format!("{:.2}", disparity / doubled),
                false => "cut".to_owned(),
            },
        );
    }
    println!(
        "\ndoubled is the width of the band where both lenses are over a tenth of the picture, \n\
         in degrees. sharpness is that band's gradient energy against the same band rendered \n\
         from the front lens alone, so 1.000 is a picture no wider than one lens's own. shear \n\
         is the disparity above divided by the band: over 1 the crossover is a fold rather \n\
         than a blend, and that is the number a narrower band buys the ghost's width with."
    );
    Ok(())
}

/// Whether a direction in the body's frame is inside the view the options
/// describe.
///
/// The camera's own rotation is `Ry(yaw) Rx(pitch)` and it takes a view ray to
/// the body (`kyerag_render::projection`), so its transpose is what brings a
/// body direction back into the view to be tested against the frustum.
fn in_view(options: &Options, ray: [f64; 3]) -> bool {
    let camera = Mat3::rot_y(options.yaw.to_radians())
        .times(Mat3::rot_x(options.pitch.to_radians()))
        .transpose();
    let v = camera.mul_vec(ray);
    let edge = (options.fov.to_radians() / 2.0).tan();
    v[2] > 0.0 && (v[0] / v[2]).abs() <= edge && (v[1] / v[2]).abs() <= edge
}

impl Weighting {
    fn name(self) -> String {
        match self {
            Self::Shipped => "shipped".to_owned(),
            Self::Single(lens) => format!("lens {lens}"),
            Self::Band(width) => match width > 0.0 {
                true => format!("{width:.1} deg"),
                false => "hard cut".to_owned(),
            },
        }
    }
}

/// A calibration with a whole correction applied to lens 1.
fn fixed(lenses: &[Lens], fix: &[(Knob, f64)]) -> Vec<Lens> {
    let mut lenses = lenses.to_vec();
    if let Some(lens) = lenses.get_mut(1) {
        for (knob, amount) in fix {
            knob.apply(lens, *amount);
        }
    }
    lenses
}

// ------------------------------------------------------------ parity

/// Where Insta360's own stitcher puts the content our stitch puts somewhere
/// else.
///
/// Their export of the same capture is the parity benchmark the issue asks
/// for, and the first thing it needs is what projection it is in. Nothing in
/// the file says, so it is fitted: our own pass is rendered under a candidate
/// rotation and field of view, and the candidate that correlates best with
/// their frame is the answer. A rectilinear view is the hypothesis being
/// fitted, and the residual is what says whether it was the right one.
///
/// The control is built into the same measurement. Away from the seam, in the
/// middle of one lens's picture, both stitchers are drawing the same lens
/// through the same model and any disagreement there is the fit's own error;
/// at the seam, the disagreement is the two stitchers disagreeing. The first
/// is what makes the second a number.
fn parity(options: &Options) -> Fallible<()> {
    let theirs = options
        .against
        .clone()
        .ok_or("parity wants against=<export.mp4>")?;
    let (calibration, ours) = frame_at(options, &options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = fixed(&calibration.lenses, &options.fix);
    let export = export_frame(&theirs, options)?;
    println!(
        "theirs: {} at {:.2} s, {}x{}",
        theirs.file_name().unwrap_or_default().to_string_lossy(),
        options.from,
        export.size,
        export.size,
    );

    let mut best = (
        f64::MIN,
        Look {
            angles: [0.0; 3],
            fov: 90.0,
            compression: 1.0,
        },
    );
    let coarse = 48;
    let small = export.resampled(coarse);
    for yaw in (0..24).map(|step| f64::from(step) * 15.0) {
        for pitch in (-5..=5).map(|step| f64::from(step) * 15.0) {
            for roll in [-15.0, 0.0, 15.0] {
                for fov in [60.0, 80.0, 100.0, 120.0] {
                    let look = Look {
                        angles: [yaw, pitch, roll],
                        fov,
                        compression: 1.0,
                    };
                    let score = agree(&looked(&lenses, frame, look, &ours, coarse), &small);
                    if score > best.0 {
                        best = (score, look);
                    }
                }
            }
        }
    }
    // Pattern search from the coarse winner, at the resolution the answer is
    // wanted in.
    let fine = 200;
    let small = export.resampled(fine);
    let scored = |look: Look, pair: &Pair| agree(&looked(&lenses, frame, look, pair, fine), &small);
    let mut step = 8.0;
    let mut score = scored(best.1, &ours);
    while step > 0.005 {
        let mut improved = false;
        for axis in 0..5 {
            for sign in [1.0, -1.0] {
                let look = best.1.nudge(axis, sign * step);
                let candidate = scored(look, &ours);
                if candidate > score {
                    (score, best.1) = (candidate, look);
                    improved = true;
                }
            }
        }
        if !improved {
            step *= 0.5;
        }
    }
    let look = best.1;
    println!(
        "fitted: yaw {:.2}, pitch {:.2}, roll {:.2}, fov {:.2} deg, compression {:.3} \
         (1.000 is rectilinear), correlating {score:.4}",
        look.angles[0], look.angles[1], look.angles[2], look.fov, look.compression,
    );

    // The comparison, and it is each stitch against itself rather than against
    // the other.
    //
    // A global fit good to a degree cannot measure a disagreement of a degree,
    // and this one is good to about that. What does not need the fit at all is
    // whether a stitch's own overlap band is as sharp as the rest of its own
    // picture: a doubled image is a blurred image, both pictures are scored by
    // the same statistic on their own pixels, and each is its own control, so
    // their tone curve and our lack of one cancel. The fit is then wanted only
    // to say which pixels are the band, and a degree of slack in a band 14
    // degrees wide is slack it can afford.
    let ours = looked(&lenses, frame, look, &ours, export.size);
    let seam = seam_map(&lenses, frame, look, export.size);
    println!(
        "\n{:<12} {:>14} {:>14} {:>12}",
        "picture", "in the band", "either side", "share"
    );
    for (name, luma) in [("ours", &ours), ("Insta360", &export.luma)] {
        let inside = banded(luma, &seam, export.size, (0.0, 5.0));
        let outside = banded(luma, &seam, export.size, (9.0, 25.0));
        println!(
            "{name:<12} {inside:>14.1} {outside:>14.1} {:>12.3}",
            match outside > 0.0 {
                true => inside / outside,
                false => 0.0,
            },
        );
    }
    println!(
        "\nthe numbers are mean squared gradient, which a doubled edge lowers and a single one \n\
         does not, taken over the pixels within 5 degrees of the seam and over the pixels 9 to \n\
         25 degrees off it in the same picture. the share is the one to read: each stitch is \n\
         measured against its own picture, so a tone curve, a sharpening pass and a lens are \n\
         all in both terms and divide out. a stitch that doubles nothing has the same share as \n\
         a picture with no seam in it."
    );
    Ok(())
}

/// Mean squared gradient over the pixels whose distance from the seam falls in
/// `band`.
fn banded(luma: &[f64], seam: &[f64], size: u32, band: (f64, f64)) -> f64 {
    let size = size as usize;
    let mut total = 0.0;
    let mut count = 0.0;
    for y in 1..size - 1 {
        for x in 1..size - 1 {
            let index = y * size + x;
            let past = seam[index].abs();
            if past < band.0 || past > band.1 {
                continue;
            }
            // A pixel no lens reached is not a pixel, and its edge against the
            // picture is not an edge in the picture.
            if [index - 1, index, index + 1]
                .iter()
                .any(|at| luma[*at] <= 0.0)
            {
                continue;
            }
            let step = luma[index + 1] - luma[index - 1];
            total += step * step;
            count += 1.0;
        }
    }
    match count > 0.0 {
        true => total / count,
        false => 0.0,
    }
}

/// One frame of a stitched export, luma only.
struct Export {
    size: u32,
    luma: Vec<f64>,
}

impl Export {
    fn resampled(&self, to: u32) -> Vec<f64> {
        let step = f64::from(self.size) / f64::from(to);
        (0..to * to)
            .map(|index| {
                let (x, y) = (index % to, index / to);
                let (sx, sy) = (
                    (f64::from(x) * step) as usize,
                    (f64::from(y) * step) as usize,
                );
                self.luma[sy * self.size as usize + sx]
            })
            .collect()
    }
}

fn export_frame(path: &Path, options: &Options) -> Fallible<Export> {
    let probe = ffprobe_size(path)?;
    let mut walk = Walk::open(path, options.from, probe)?;
    let pair = walk
        .next_pair()?
        .ok_or("no frame decoded from the export")?;
    let plane = &pair.lenses[0];
    let luma = (0..probe.height as usize)
        .flat_map(|y| (0..probe.width as usize).map(move |x| (x, y)))
        .map(|(x, y)| f64::from(plane.luma[y * plane.stride + x]))
        .collect();
    Ok(Export {
        size: probe.width,
        luma,
    })
}

/// The export's frame size, read off the stream rather than assumed.
fn ffprobe_size(path: &Path) -> Fallible<MetaSize> {
    ffmpeg_next::init()?;
    let input = ffmpeg_next::format::input(&path)?;
    let stream = input
        .streams()
        .find(|s| s.parameters().medium() == ffmpeg_next::media::Type::Video)
        .ok_or("the export carries no video stream")?;
    let decoder =
        ffmpeg_next::codec::context::Context::from_parameters(stream.parameters())?.decoder();
    let video = decoder.video()?;
    Ok(MetaSize {
        width: video.width(),
        height: video.height(),
    })
}

/// What an unknown stitcher's output might be a picture in.
///
/// One family with one number in it, because guessing between named
/// projections and fitting a parameter are the same amount of code and only
/// one of them answers when the guess is wrong: `theta = atan(rho tan(c phi))
/// / c` is rectilinear at `c = 1`, equidistant in the limit at `c = 0`, and
/// everything a consumer 360 app calls "natural" in between. A fit that lands
/// on 1 has found a rectilinear reframe and said so.
fn ray_of(uv: [f64; 2], half_fov: f64, compression: f64) -> [f64; 3] {
    let (u, v) = (uv[0] * 2.0 - 1.0, uv[1] * 2.0 - 1.0);
    let rho = u.hypot(v);
    let c = compression.max(1e-3);
    let theta = (rho * (c * half_fov).tan()).atan() / c;
    let (sin, cos) = theta.sin_cos();
    match rho > 0.0 {
        true => [sin * u / rho, sin * v / rho, cos],
        false => [0.0, 0.0, 1.0],
    }
}

/// Our own pass, rendered into a candidate projection under a candidate
/// rotation, which is what a fitted export has to be compared against.
fn looked(lenses: &[Lens], frame: Size, view: Look, pair: &Pair, size: u32) -> Vec<f64> {
    let reframe = Reframe::new(
        lenses,
        frame,
        Camera::default(),
        Held {
            body_from_world: orientation(view.angles),
            rolling: None,
        },
        1.0,
        false,
        Sampling::default(),
    );
    (0..size * size)
        .map(|index| {
            let uv = [
                (f64::from(index % size) + 0.5) / f64::from(size),
                (f64::from(index / size) + 0.5) / f64::from(size),
            ];
            let ray = ray_of(uv, view.fov.to_radians() / 2.0, view.compression);
            let (weights, landings) = Weighting::Shipped.at(&reframe, ray);
            let mut luma = 0.0;
            let mut total = 0.0;
            for lens in 0..2 {
                if weights[lens] <= 0.0 {
                    continue;
                }
                let Some(code) = pair.lenses[lens].at(
                    f64::from(landings[lens].pixel[0]),
                    f64::from(landings[lens].pixel[1]),
                ) else {
                    continue;
                };
                luma += weights[lens] * code;
                total += weights[lens];
            }
            match total > 0.0 {
                true => luma / total,
                false => 0.0,
            }
        })
        .collect()
}

/// A candidate for what the export is a picture of.
#[derive(Clone, Copy)]
struct Look {
    angles: [f64; 3],
    fov: f64,
    compression: f64,
}

impl Look {
    /// The five numbers as one vector, so the pattern search can step any of
    /// them without knowing which is which.
    fn nudge(mut self, axis: usize, step: f64) -> Self {
        match axis {
            0..=2 => self.angles[axis] += step,
            3 => self.fov += step,
            _ => self.compression = (self.compression + step * 0.005).clamp(0.05, 1.0),
        }
        self
    }
}

/// How far every pixel of that view is from the seam, in degrees.
fn seam_map(lenses: &[Lens], frame: Size, view: Look, size: u32) -> Vec<f64> {
    let reframe = Reframe::new(
        lenses,
        frame,
        Camera::default(),
        Held {
            body_from_world: orientation(view.angles),
            rolling: None,
        },
        1.0,
        false,
        Sampling::default(),
    );
    (0..size * size)
        .map(|index| {
            let uv = [
                (f64::from(index % size) + 0.5) / f64::from(size),
                (f64::from(index / size) + 0.5) / f64::from(size),
            ];
            past_seam(
                &reframe,
                ray_of(uv, view.fov.to_radians() / 2.0, view.compression),
            )
        })
        .collect()
}

/// Yaw, pitch and roll in degrees, as the rotation the view is held at.
fn orientation(angles: [f64; 3]) -> Quat {
    let about = |axis: usize, degrees: f64| {
        let mut v = [0.0; 3];
        v[axis] = degrees.to_radians();
        Quat::from_rotation_vector(v)
    };
    about(1, angles[0])
        .times(about(0, angles[1]))
        .times(about(2, angles[2]))
}

/// Zero-mean normalized cross-correlation between two pictures of one size.
fn agree(ours: &[f64], theirs: &[f64]) -> f64 {
    let count = ours.len().min(theirs.len()) as f64;
    if count < 4.0 {
        return 0.0;
    }
    let mean = |v: &[f64]| v.iter().take(count as usize).sum::<f64>() / count;
    let (mean_a, mean_b) = (mean(ours), mean(theirs));
    let (mut covariance, mut var_a, mut var_b) = (0.0, 0.0, 0.0);
    for (a, b) in ours.iter().zip(theirs) {
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
