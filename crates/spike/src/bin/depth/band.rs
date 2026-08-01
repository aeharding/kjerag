//! The overlap band read as a stereo pair: the geometry, and the correlation
//! that turns it into a distance.
//!
//! The two lenses of a back-to-back pair image the band from two centres
//! `t` apart, so inside the band the camera is a **stereo rig** and the
//! disagreement the seam instruments have been reporting as a residual is a
//! disparity with a distance behind it. This module owns the geometry that
//! says which way that disparity runs, and the block matching that reads it.
//!
//! The direction it runs is not assumed. For a ray `d` and a baseline `t`
//! between the two centres, moving the centre by `t` displaces the picture of
//! a point at distance `Z` by `-(t - (t.d) d) / Z`, which is along the part of
//! the baseline the ray can see and along nothing else. On the seam circle
//! that part is the whole of it, which is why the disparity is largest there;
//! and the direction it points is the across-seam tangent **only if the
//! baseline is exactly the body's z**. On the X4 Air fixture it is
//! `[-0.002063, +0.000334, -0.033284]` m, which is 3.6 degrees off that axis,
//! so [`Node`] carries the epipolar direction the calibration actually says
//! and reports how far it is from the naive one.

use kjerag_meta::CalibrationSet;
use kjerag_render::Reframe;
use kjerag_spike::Plane;

/// The vector from lens 0's centre to lens 1's, in the body's frame, metres.
///
/// The file writes it: `offset_v3`'s translation triple, lens 0 at the origin
/// (docs/research/insv-format.md 4.1). Nothing here estimates a baseline.
pub fn baseline(calibration: &CalibrationSet) -> [f64; 3] {
    calibration
        .lenses
        .get(1)
        .map_or([0.0, 0.0, 0.0], |lens| lens.pose.translation_m)
}

/// One direction inside the overlap band, and the two axes of the sphere
/// there that the stereo geometry names.
#[derive(Clone, Copy)]
pub struct Node {
    /// Round the seam great circle, radians from the body's +x.
    pub phi: f64,
    /// How far past the seam, radians: 0 on the seam circle, positive towards
    /// lens 1's hemisphere.
    pub psi: f64,
    pub centre: [f64; 3],
    /// The epipolar direction: the way lens 1's picture of a **near** point
    /// sits from lens 0's picture of the same point. Disparity runs along this
    /// and along nothing else.
    pub epi: [f64; 3],
    /// The other tangent, which disparity cannot reach at any distance.
    ///
    /// Depth is never here, so this axis is where everything that is not depth
    /// shows up: the calibration the per-file fit did not take out, and the
    /// instrument's own noise. Measured on real footage it reads 0.4 to 0.7
    /// degrees, which is calibration and not noise, and a one-dimensional
    /// search held at zero on this axis is therefore correlating patches that
    /// do not hold the same content. [`crate::measure::Prealign`] is what
    /// takes it out first.
    pub perp: [f64; 3],
    /// How much of the baseline this ray can see, metres. `|t|` on the seam
    /// circle, falling off as `cos` towards either lens.
    pub reach_m: f64,
    /// How far the epipolar direction is from the across-seam tangent, in
    /// degrees. The geometry check: it is the baseline's own tilt, and it says
    /// how much of a disparity a naive across-seam search would misplace.
    pub skew_deg: f64,
}

/// The band's own frame at one direction, built from the baseline the file
/// carries.
pub fn node(baseline: [f64; 3], phi: f64, psi: f64) -> Node {
    let (sin_phi, cos_phi) = phi.sin_cos();
    let (sin_psi, cos_psi) = psi.sin_cos();
    let round = [cos_phi, sin_phi, 0.0];
    let centre = unit([cos_psi * round[0], cos_psi * round[1], sin_psi]);
    // The across-seam tangent, which is what every earlier instrument searched
    // along, kept here only to be compared against the real epipolar axis.
    let across = unit(cross(cross(centre, [0.0, 0.0, 1.0]), centre));
    let seen = std::array::from_fn(|axis| baseline[axis] - dot(baseline, centre) * centre[axis]);
    let reach_m = norm(seen);
    // Negated: lens 1 sits behind lens 0, so `-t` points the way its picture
    // of a near point is displaced, which is towards the front lens at every
    // azimuth. That one-signedness is what tells parallax from a residual
    // rotation, and it is a prediction of this line rather than an assumption
    // in the reading.
    let epi = match reach_m > 0.0 {
        true => unit(seen.map(|c| -c)),
        false => across,
    };
    Node {
        phi,
        psi,
        centre,
        epi,
        // `epi x centre` and not the other way up. That is the one convention
        // the tree has for this axis since issue #103 stage 6: the seam
        // circle's own tangent towards increasing azimuth, the sign
        // `seam::ring` publishes and `band::Ring::perp` is now built to.
        perp: unit(cross(epi, centre)),
        reach_m,
        skew_deg: dot(epi, across).clamp(-1.0, 1.0).acos().to_degrees(),
    }
}

impl Node {
    /// The distance to whatever produced a disparity of `shift` radians here,
    /// in metres. Positive shifts are near content; a zero or negative shift
    /// is content the instrument cannot place, which it reports as infinity.
    pub fn metres(&self, shift: f64) -> f64 {
        match shift > 0.0 {
            true => self.reach_m / shift,
            false => f64::INFINITY,
        }
    }
}

/// The band, sampled as a grid: `phis` positions round the seam circle by
/// each of `psis` distances past it.
pub fn grid(baseline: [f64; 3], phis: usize, psis: &[f64]) -> Vec<Node> {
    (0..phis)
        .flat_map(|index| {
            let phi = index as f64 / phis as f64 * std::f64::consts::TAU;
            psis.iter()
                .map(move |psi| node(baseline, phi, psi.to_radians()))
        })
        .collect()
}

/// One lens's picture of a rectangle of the sphere, sampled on a grid of
/// **directions**: `2 * perp + 1` across the epipolar axis by `2 * epi + 1`
/// along it, `step` radians apart.
///
/// Sampled at directions rather than at pixels so that the shift the
/// correlation finds is already in degrees of world angle, with no lens model
/// left in it to undo.
pub struct Grid {
    perp: isize,
    epi: isize,
    luma: Vec<f64>,
}

impl Grid {
    fn at(&self, i: isize, j: isize) -> f64 {
        self.luma[((i + self.perp) * (2 * self.epi + 1) + (j + self.epi)) as usize]
    }

    /// How much picture there is to correlate, in 8-bit codes. Flat sky
    /// correlates with anything, and a band that crosses sky is most of a
    /// paramotor's sphere.
    pub fn contrast(&self) -> f64 {
        let count = self.luma.len() as f64;
        let mean = self.luma.iter().sum::<f64>() / count;
        (self.luma.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count).sqrt()
    }

    /// Zero-mean normalized cross-correlation against `other` shifted by
    /// `(di, dj)`, over every sample of this grid.
    fn correlation(&self, other: &Grid, di: isize, dj: isize) -> f64 {
        let (mut sum_a, mut sum_b, mut count) = (0.0, 0.0, 0.0);
        let mut pairs: Vec<(f64, f64)> = Vec::with_capacity(self.luma.len());
        for i in -self.perp..=self.perp {
            for j in -self.epi..=self.epi {
                let (a, b) = (self.at(i, j), other.at(i + di, j + dj));
                sum_a += a;
                sum_b += b;
                count += 1.0;
                pairs.push((a, b));
            }
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

/// One lens's picture of the sphere around `at`, on the band's own axes.
/// `None` where any corner falls outside this lens's picture: two lenses have
/// to be answering about the same directions or a correlation between them
/// means nothing.
/// `offset` slides the whole grid, in radians, `[perp, epi]`. Along the
/// epipolar axis that is how a synthetic disparity is injected: content
/// sampled `x` further along it is content that would be `x` nearer, and the
/// reading has to come back `x` larger. Across it, it is where the residual
/// calibration is taken out so that a one-dimensional search is looking at the
/// same content it is measuring.
pub fn sample(
    reframe: &Reframe,
    plane: &Plane,
    lens: usize,
    at: &Node,
    half: (isize, isize),
    step: f64,
    offset: [f64; 2],
) -> Option<Grid> {
    let mut luma = Vec::with_capacity(((2 * half.0 + 1) * (2 * half.1 + 1)) as usize);
    for i in -half.0..=half.0 {
        for j in -half.1..=half.1 {
            let (a, b) = (i as f64 * step + offset[0], j as f64 * step + offset[1]);
            let ray = unit(std::array::from_fn(|axis| {
                at.centre[axis] + at.perp[axis] * a + at.epi[axis] * b
            }));
            let landing = reframe.project(lens, ray.map(|c| c as f32));
            if !landing.inside {
                return None;
            }
            luma.push(plane.at(f64::from(landing.pixel[0]), f64::from(landing.pixel[1]))?);
        }
    }
    Some(Grid {
        perp: half.0,
        epi: half.1,
        luma,
    })
}

/// What one node's correlation found.
#[derive(Clone, Copy)]
pub struct Peak {
    /// Along the epipolar axis, radians. This is the disparity.
    pub epi: f64,
    /// Across it, radians. Nothing at any distance can put a signal here, so
    /// it is the instrument reading its own noise floor on the same pixels.
    pub perp: f64,
    pub r: f64,
    /// How sharply the correlation falls away from the peak along the
    /// epipolar axis, per step squared. A flat peak is a repetitive texture
    /// or a smooth gradient: the shift it reports is the least trustworthy
    /// number this file produces, and this is what separates it out.
    pub curvature: f64,
    pub contrast: f64,
}

/// The disparity at one node: a search along the epipolar axis alone, at
/// whole steps, then a parabola through the winner.
///
/// One dimension, because the geometry says the answer is in one dimension
/// and searching the other costs a square. [`free_shift`] is the check on
/// that claim, run once per campaign rather than once per node.
pub fn epipolar_shift(front: &Grid, back: &Grid, search: isize, step: f64) -> Option<Peak> {
    let score = |dj: isize| front.correlation(back, 0, dj);
    let mut best: Option<(isize, f64)> = None;
    for dj in -search..=search {
        let r = score(dj);
        if best.is_none_or(|(_, held)| r > held) {
            best = Some((dj, r));
        }
    }
    let (j, r) = best?;
    // A peak against the edge of the search is not a peak, it is the search
    // running out. Content nearer than the search is wide reads as the limit
    // and would be believed.
    if j.abs() >= search {
        return None;
    }
    let (minus, plus) = (score(j - 1), score(j + 1));
    let curve = minus - 2.0 * r + plus;
    let refined = match curve < 0.0 {
        true => (0.5 * (minus - plus) / curve).clamp(-1.0, 1.0),
        false => 0.0,
    };
    Some(Peak {
        epi: (j as f64 + refined) * step,
        perp: 0.0,
        r,
        curvature: -curve,
        contrast: front.contrast(),
    })
}

/// The same disparity found without being told which way to look: a full
/// two-dimensional search on the same patches.
///
/// This is the geometry check, and it is a check because it can fail. If the
/// band really is a stereo pair with the baseline the file records, the
/// off-epipolar component of this shift is zero to the instrument's own
/// repeatability at every node, whatever the scene. If the baseline is wrong,
/// or the seam is somewhere else, or the search is finding texture rather than
/// content, it is not.
pub fn free_shift(front: &Grid, back: &Grid, search: (isize, isize), step: f64) -> Option<Peak> {
    let score = |di: isize, dj: isize| front.correlation(back, di, dj);
    let mut best: Option<(isize, isize, f64)> = None;
    for di in -search.0..=search.0 {
        for dj in -search.1..=search.1 {
            let r = score(di, dj);
            if best.is_none_or(|(_, _, held)| r > held) {
                best = Some((di, dj, r));
            }
        }
    }
    let (i, j, r) = best?;
    if i.abs() >= search.0 || j.abs() >= search.1 {
        return None;
    }
    let peak = |minus: f64, here: f64, plus: f64| {
        let curve = minus - 2.0 * here + plus;
        match curve < 0.0 {
            true => (0.5 * (minus - plus) / curve, -curve),
            false => (0.0, 0.0),
        }
    };
    let (across_epi, curvature) = peak(score(i, j - 1), r, score(i, j + 1));
    let (across_perp, _) = peak(score(i - 1, j), r, score(i + 1, j));
    Some(Peak {
        epi: (j as f64 + across_epi) * step,
        perp: (i as f64 + across_perp) * step,
        r,
        curvature,
        contrast: front.contrast(),
    })
}

/// Root mean square of whatever is put in it. Every number this instrument
/// reports about a spread is one of these.
#[derive(Default)]
pub struct Accumulator {
    total: f64,
    pub count: usize,
}

impl Accumulator {
    pub fn add(&mut self, value: f64) {
        self.total += value * value;
        self.count += 1;
    }

    pub fn rms(&self) -> f64 {
        match self.count {
            0 => 0.0,
            count => (self.total / count as f64).sqrt(),
        }
    }
}

// ------------------------------------------------------------ vectors

pub fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn norm(v: [f64; 3]) -> f64 {
    dot(v, v).sqrt()
}

pub fn unit(v: [f64; 3]) -> [f64; 3] {
    let length = norm(v);
    match length > 0.0 {
        true => v.map(|c| c / length),
        false => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The X4 Air fixture's baseline, which every number in this instrument's
    /// report is scaled by (docs/research/x4air-calibration.json).
    const T: [f64; 3] = [-0.002063, 0.000334, -0.033284];

    /// The geometry, checked against the thing it is a model of: a point at a
    /// known distance, imaged from two centres `T` apart, with no lens and no
    /// correlation in the way.
    ///
    /// This is the claim the whole instrument rests on, so it is checked by
    /// construction rather than by reading a number back off a picture: the
    /// two directions to the point differ by `reach / Z` along `epi` and by
    /// nothing at all along `perp`.
    #[test]
    fn a_point_at_a_known_distance_moves_by_reach_over_distance() {
        for phi in [0.0, 0.7, 2.5, 4.9] {
            for psi in [-0.05, 0.0, 0.05] {
                let at = node(T, phi, psi);
                for metres in [0.5, 1.0, 3.0, 10.0] {
                    let point = at.centre.map(|c| c * metres);
                    // Lens 1 sits at T, so the point is at `point - T` from it.
                    let from_one = unit(std::array::from_fn(|axis| point[axis] - T[axis]));
                    let moved: [f64; 3] =
                        std::array::from_fn(|axis| from_one[axis] - at.centre[axis]);
                    let along = dot(moved, at.epi);
                    let across = dot(moved, at.perp);
                    let predicted = at.reach_m / metres;
                    assert!(
                        (along - predicted).abs() < 0.02 * predicted,
                        "at phi {phi}, {metres} m: {along} along, predicted {predicted}"
                    );
                    assert!(
                        across.abs() < 0.02 * predicted,
                        "at phi {phi}, {metres} m: {across} across, which depth cannot reach"
                    );
                    assert!((at.metres(along) - metres).abs() < 0.05 * metres);
                }
            }
        }
    }

    /// The epipolar axis is the baseline's, not the seam's. A search along the
    /// across-seam tangent would be 3.6 degrees off at worst on this camera,
    /// and `skew_deg` is what reports that rather than assuming it away.
    #[test]
    fn the_epipolar_axis_is_tilted_off_the_across_seam_tangent() {
        let skews: Vec<f64> = grid(T, 72, &[0.0])
            .iter()
            .map(|node| node.skew_deg)
            .collect();
        let worst = skews.iter().copied().fold(f64::MIN, f64::max);
        assert!((3.0..4.5).contains(&worst), "worst skew {worst} deg");
        // A baseline exactly down the lens axis has none of it, which is the
        // case the naive across-seam search would have been right for.
        let straight = grid([0.0, 0.0, -0.0333], 72, &[0.0]);
        assert!(straight.iter().all(|node| node.skew_deg < 1e-6));
    }

    /// The reach falls off towards either lens, so the same subject reads a
    /// smaller disparity off the seam circle and the **distance** stays put.
    #[test]
    fn the_reach_is_the_whole_baseline_on_the_seam_circle() {
        let on = node(T, 1.0, 0.0);
        let off = node(T, 1.0, 0.5);
        assert!((on.reach_m - norm(T)).abs() < 2e-4);
        assert!(off.reach_m < on.reach_m);
    }
}
