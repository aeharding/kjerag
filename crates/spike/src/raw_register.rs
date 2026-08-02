//! A complete raw-lens patch, registered in the camera's epipolar frame.
//!
//! The rendered path may say which seam segment a named view exposes.  It is
//! never a source of matching pixels: these routines read only `Plane` and
//! project the body's rays through each raw lens.

use kjerag_media::Plane;
use kjerag_render::Reframe;

use crate::local_warp::{self, RegistrationSample};

/// One point on the rendered crossover contour, with the axes the recorded
/// baseline gives it. `perp` is the axis no physical disparity can reach;
/// `epi` is the epipolar axis, in the sign used by the old depth instrument.
#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub centre: [f64; 3],
    pub perp: [f64; 3],
    pub epi: [f64; 3],
    pub phi: f64,
}

/// A point selected from the seam contour of the rendered view.  The point is
/// only a location; it contains no composited colour or blend reading.
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub node: Node,
    /// The view-space root that produced `node`. Kept for locator controls;
    /// registration itself starts afresh from `node` and raw planes.
    pub view_ray: [f32; 3],
    /// The output-raster location of the traced contour root. It is a
    /// location only: raw planes below remain the registration evidence.
    pub view_pixel: [f32; 2],
}

/// A raw measurement in radians on `[perp, epi]` axes.
#[derive(Clone, Copy, Debug)]
pub struct Reading {
    pub candidate: Candidate,
    pub shift_rad: [f64; 2],
    pub covariance_rad2: [[f64; 2]; 2],
    pub condition: f64,
    pub correlation: f64,
    pub score: f64,
}

/// Why the automatic selector could not make a two-axis claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    NoVisibleSeam,
    NoCompletePatch,
    NoPeak,
    Aperture,
}

/// One globally declared raw-lens support.  It is angular rather than pixel
/// sized so it means the same physical neighbourhood in every named view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Support {
    pub span_deg: f64,
    pub search_deg: f64,
    pub step_deg: f64,
}

impl Support {
    fn valid(self) -> bool {
        self.span_deg.is_finite()
            && self.search_deg.is_finite()
            && self.step_deg.is_finite()
            && self.span_deg > 0.0
            && self.search_deg > 0.0
            && self.step_deg > 0.0
    }

    fn half(self) -> isize {
        (self.span_deg.to_radians() / (2.0 * self.step_deg.to_radians())).round() as isize
    }

    fn search_steps(self) -> isize {
        (self.search_deg.to_radians() / self.step_deg.to_radians()).floor() as isize
    }
}

/// The support sweep shipped with the instrument.  It is deliberately global:
/// it may be narrowed or widened on the command line, but never per view,
/// frame, candidate, or result.
pub const SUPPORT_LADDER: [Support; 4] = [
    Support {
        span_deg: 1.20,
        search_deg: 1.00,
        step_deg: 0.08,
    },
    Support {
        span_deg: 2.00,
        search_deg: 1.60,
        step_deg: 0.08,
    },
    Support {
        span_deg: 2.80,
        search_deg: 2.40,
        step_deg: 0.08,
    },
    Support {
        span_deg: 3.68,
        search_deg: 3.00,
        step_deg: 0.08,
    },
];

/// Accounting behind one support result.  In particular, `reference_complete`
/// and `complete_target_patches` separate loss of geometric support from a
/// complete but rank-deficient (aperture) patch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegistrationHealth {
    pub candidates: usize,
    pub reference_complete: usize,
    pub searched_offsets: usize,
    pub complete_target_patches: usize,
    pub readings: usize,
    pub no_complete_patch: usize,
    pub aperture: usize,
    pub no_peak: usize,
    /// Reference patches which left lens 0's calibrated projection.
    pub reference_projected_out: usize,
    /// Reference patches whose projection was legal but the delivered raw
    /// plane had no bilinear sample (normally its source boundary).
    pub reference_source_boundary: usize,
    /// Candidate/search patches which left lens 1's calibrated projection.
    pub target_projected_out: usize,
    /// Candidate/search patches whose projection was legal but the delivered
    /// raw plane had no bilinear sample.
    pub target_source_boundary: usize,
}

/// CPU census of the named view's raw-lens validity mask.  `projected` is
/// solely `Reframe::project(...).inside`; `readable` additionally proves a
/// direct [`Plane::at`] read.  No blend weight, renderer pixel, or coverage
/// pre-test contributes to either number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LensCoverage {
    pub projected: usize,
    pub readable: usize,
    pub source_boundary: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoverageCensus {
    pub view_rays: usize,
    pub outside_view: usize,
    pub lenses: Vec<LensCoverage>,
}

/// Count every camera-frame pixel in a named view against each raw lens.
/// This is deliberately a separate diagnostic from registration: it says
/// whether a missing patch is already explained by the source plane boundary
/// before texture or peak selection gets to make a claim.
pub fn coverage_census(map: &Reframe, planes: &[Plane], width: u32, height: u32) -> CoverageCensus {
    let mut census = CoverageCensus {
        lenses: vec![LensCoverage::default(); planes.len()],
        ..CoverageCensus::default()
    };
    for y in 0..height {
        for x in 0..width {
            let uv = [
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            ];
            let Some(ray) = map.view_ray(uv) else {
                census.outside_view += 1;
                continue;
            };
            census.view_rays += 1;
            for (lens, plane) in planes.iter().enumerate() {
                let landing = map.project(lens, ray);
                if !landing.inside {
                    continue;
                }
                let coverage = &mut census.lenses[lens];
                coverage.projected += 1;
                if plane
                    .at(f64::from(landing.pixel[0]), f64::from(landing.pixel[1]))
                    .is_some()
                {
                    coverage.readable += 1;
                } else {
                    coverage.source_boundary += 1;
                }
            }
        }
    }
    census
}

/// One row of a support ladder.  A refusal is evidence, not an omitted row.
#[derive(Clone, Copy, Debug)]
pub struct SupportResult {
    pub support: Support,
    pub result: Result<Reading, Refused>,
    pub health: RegistrationHealth,
}

const BINS: usize = 72;

/// Candidate locations on the visible rendered crossover, one per
/// camera-frame azimuth bin.
///
/// The body's `z = 0` circle is a useful nominal seam, but it is not the
/// line the pass hands over on after a selected lens calibration: the pass
/// centres the crossover on the two optical axes, then coverage depth moves
/// its final 50/50 point again. Trace the latter, from the same `Blend` the
/// fragment shader uses. A view which has no two-lens 50/50 contour returns
/// no candidates rather than borrowing a nominal body-circle location.
pub fn visible_candidates(
    map: &Reframe,
    width: u32,
    height: u32,
    baseline: [f64; 3],
) -> Vec<Candidate> {
    let mut picked: [Option<(f32, Candidate)>; BINS] = [None; BINS];
    let mut samples = vec![None; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let uv = [
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            ];
            samples[(y * width + x) as usize] = sample_crossover(map, uv);
        }
    }
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            let Some(here) = samples[index] else {
                continue;
            };
            for (dx, dy) in [(1, 0), (0, 1)] {
                let (next_x, next_y) = (x + dx, y + dy);
                if next_x >= width || next_y >= height {
                    continue;
                }
                let Some(next) = samples[(next_y * width + next_x) as usize] else {
                    continue;
                };
                let Some((view, weight)) = crossover_root(map, here, next) else {
                    continue;
                };
                let body = unit(map.body_ray(view).map(f64::from));
                if norm(body) == 0.0 {
                    continue;
                }
                let phi = body[1].atan2(body[0]);
                let bin = ((phi.rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU
                    * BINS as f64)
                    .floor() as usize)
                    % BINS;
                let candidate = Candidate {
                    node: node(baseline, body),
                    view_ray: view,
                    view_pixel: [
                        x as f32 + 0.5 + dx as f32 * 0.5,
                        y as f32 + 0.5 + dy as f32 * 0.5,
                    ],
                };
                // Prefer the root whose two lens claims are furthest from an
                // edge. This only chooses among already-valid 50/50 roots;
                // it never promotes a rendered pixel to measurement data.
                if picked[bin].is_none_or(|(held, _)| weight > held) {
                    picked[bin] = Some((weight, candidate));
                }
            }
        }
    }
    picked
        .into_iter()
        .flatten()
        .map(|(_, candidate)| candidate)
        .collect()
}

/// One raster sample that can participate in a genuine two-lens crossover.
/// The score is the signed final rendered-weight difference, not the nominal
/// lens-axis difference; coverage-depth claims are deliberately included.
#[derive(Clone, Copy)]
struct CrossoverSample {
    view: [f32; 3],
    difference: f32,
}

fn sample_crossover(map: &Reframe, uv: [f32; 2]) -> Option<CrossoverSample> {
    let view = map.view_ray(uv)?;
    let blend = map.blend(view);
    (blend.weights[0] > 0.0
        && blend.weights[1] > 0.0
        && blend.landings[0].inside
        && blend.landings[1].inside)
        .then_some(CrossoverSample {
            view,
            difference: blend.weights[0] - blend.weights[1],
        })
}

/// A zero of the final rendered weights along one raster edge. The bisection
/// is in view-ray space, which is sufficient for a subpixel contour root and
/// lets the runtime `Blend` remain the sole definition of the crossover.
fn crossover_root(
    map: &Reframe,
    left: CrossoverSample,
    right: CrossoverSample,
) -> Option<([f32; 3], f32)> {
    if left.difference == 0.0 {
        return Some((left.view, crossover_weight(map, left.view)?));
    }
    if right.difference == 0.0 {
        return Some((right.view, crossover_weight(map, right.view)?));
    }
    if left.difference.signum() == right.difference.signum() {
        return None;
    }
    let (mut low, mut high) = (left.view, right.view);
    let low_sign = left.difference.signum();
    for _ in 0..24 {
        let middle = unit_f32(std::array::from_fn(|axis| 0.5 * (low[axis] + high[axis])));
        let middle_difference = sample_crossover_ray(map, middle)?;
        if middle_difference.signum() == low_sign {
            low = middle;
        } else {
            high = middle;
        }
    }
    let root = unit_f32(std::array::from_fn(|axis| 0.5 * (low[axis] + high[axis])));
    Some((root, crossover_weight(map, root)?))
}

fn sample_crossover_ray(map: &Reframe, view: [f32; 3]) -> Option<f32> {
    let blend = map.blend(view);
    (blend.weights[0] > 0.0
        && blend.weights[1] > 0.0
        && blend.landings[0].inside
        && blend.landings[1].inside)
        .then_some(blend.weights[0] - blend.weights[1])
}

fn crossover_weight(map: &Reframe, view: [f32; 3]) -> Option<f32> {
    let blend = map.blend(view);
    (blend.weights[0] > 0.0
        && blend.weights[1] > 0.0
        && blend.landings[0].inside
        && blend.landings[1].inside)
        .then_some(blend.weights[0].min(blend.weights[1]))
}

/// Register and select the most informative complete raw patch.  A complete
/// patch is required at the winning offset in both lenses; there is no hole
/// fill or best-effort rectangle.
pub fn select(
    map: &Reframe,
    planes: &[Plane],
    candidates: &[Candidate],
) -> Result<Reading, Refused> {
    select_with_support(map, planes, candidates, SUPPORT_LADDER[3]).result
}

/// Run the declared global support ladder.  Every candidate is attempted at
/// every rung; a successful small patch is not permission to hide a larger
/// patch's support or aperture refusal.
pub fn select_ladder(
    map: &Reframe,
    planes: &[Plane],
    candidates: &[Candidate],
    supports: &[Support],
) -> Vec<SupportResult> {
    supports
        .iter()
        .copied()
        .map(|support| select_with_support(map, planes, candidates, support))
        .collect()
}

/// Register and select one declared support, retaining all refusal counts.
pub fn select_with_support(
    map: &Reframe,
    planes: &[Plane],
    candidates: &[Candidate],
    support: Support,
) -> SupportResult {
    let mut health = RegistrationHealth {
        candidates: candidates.len(),
        ..RegistrationHealth::default()
    };
    if candidates.is_empty() {
        return SupportResult {
            support,
            result: Err(Refused::NoVisibleSeam),
            health,
        };
    }
    if planes.len() < 2 || !support.valid() || support.half() < 1 || support.search_steps() < 1 {
        health.no_complete_patch = candidates.len();
        return SupportResult {
            support,
            result: Err(Refused::NoCompletePatch),
            health,
        };
    }
    let mut best: Option<Reading> = None;
    for candidate in candidates {
        match read(
            map,
            &planes[0],
            &planes[1],
            *candidate,
            support,
            &mut health,
        ) {
            Ok(reading) if best.is_none_or(|held| reading.score > held.score) => {
                health.readings += 1;
                best = Some(reading)
            }
            Ok(_) => health.readings += 1,
            Err(Refused::NoCompletePatch) => health.no_complete_patch += 1,
            Err(Refused::Aperture) => health.aperture += 1,
            Err(Refused::NoPeak) => health.no_peak += 1,
            Err(Refused::NoVisibleSeam) => {}
        }
    }
    let result = best.ok_or_else(|| {
        if health.no_complete_patch == candidates.len() {
            Refused::NoCompletePatch
        } else if health.aperture + health.no_complete_patch == candidates.len() {
            Refused::Aperture
        } else {
            Refused::NoPeak
        }
    });
    SupportResult {
        support,
        result,
        health,
    }
}

fn read(
    map: &Reframe,
    front: &Plane,
    back: &Plane,
    candidate: Candidate,
    support: Support,
    health: &mut RegistrationHealth,
) -> Result<Reading, Refused> {
    let step = support.step_deg.to_radians();
    let half = support.half();
    let a = sample(map, front, 0, candidate.node, half, step, [0.0, 0.0]).map_err(|why| {
        match why {
            PatchRefusal::ProjectedOut => health.reference_projected_out += 1,
            PatchRefusal::SourceBoundary => health.reference_source_boundary += 1,
        }
        Refused::NoCompletePatch
    })?;
    health.reference_complete += 1;
    let coarse = support.search_steps();
    let mut legal = Vec::new();
    // This is deliberately an unconstrained 2-D search.  A physical depth
    // hypothesis can later explain the epi term, but must not manufacture a
    // zero perp term before the evidence has been read.  Coverage applies to
    // this *one* shift's patch: an unavailable neighbouring shift is omitted,
    // never allowed to turn the legal shifts into a rectangle-sized refusal.
    for i in -coarse..=coarse {
        for j in -coarse..=coarse {
            health.searched_offsets += 1;
            let offset = [i as f64 * step, j as f64 * step];
            let b = match sample(map, back, 1, candidate.node, half, step, offset) {
                Ok(b) => b,
                Err(PatchRefusal::ProjectedOut) => {
                    health.target_projected_out += 1;
                    continue;
                }
                Err(PatchRefusal::SourceBoundary) => {
                    health.target_source_boundary += 1;
                    continue;
                }
            };
            health.complete_target_patches += 1;
            legal.push(([i, j], correlation(&a, &b)));
        }
    }
    let ([i, j], correlation) = peak(&legal, coarse)?;
    let offset = [i as f64 * step, j as f64 * step];
    let b = sample(map, back, 1, candidate.node, half, step, offset)
        .map_err(|_| Refused::NoCompletePatch)?;
    let samples = samples(&a, &b, step);
    let fitted = local_warp::register(&samples).map_err(|why| match why {
        local_warp::RegistrationRefused::Aperture => Refused::Aperture,
        _ => Refused::NoPeak,
    })?;
    // `register` is in grid steps because its gradients are differences one
    // grid cell apart.  Convert both estimate and covariance to radians.
    let shift = [
        offset[0] + fitted.displacement.x * step,
        offset[1] + fitted.displacement.y * step,
    ];
    let covariance = [
        [
            fitted.covariance.xx * step * step,
            fitted.covariance.xy * step * step,
        ],
        [
            fitted.covariance.xy * step * step,
            fitted.covariance.yy * step * step,
        ],
    ];
    // Information in both axes rewards texture and penalizes an enormous
    // uncertainty; it is a selector score, not an acceptance claim.
    let uncertainty = (covariance[0][0] * covariance[1][1] - covariance[0][1].powi(2))
        .max(0.0)
        .sqrt();
    Ok(Reading {
        candidate,
        shift_rad: shift,
        covariance_rad2: covariance,
        condition: fitted.condition,
        correlation,
        score: correlation / (1.0 + uncertainty),
    })
}

/// Pick one maximum from the patches which were complete at their own shift.
///
/// A boundary maximum says the declared search did not contain the answer;
/// tied maxima say the content did not identify one.  Both are `NoPeak`.
/// Missing shifts are not evidence against the complete shifts that remain.
fn peak(legal: &[([isize; 2], f64)], coarse: isize) -> Result<([isize; 2], f64), Refused> {
    let Some((offset, correlation)) = legal
        .iter()
        .copied()
        .max_by(|left, right| left.1.total_cmp(&right.1))
    else {
        return Err(Refused::NoCompletePatch);
    };
    if offset[0].abs() >= coarse
        || offset[1].abs() >= coarse
        || legal
            .iter()
            .filter(|(_, score)| *score == correlation)
            .nth(1)
            .is_some()
    {
        return Err(Refused::NoPeak);
    }
    Ok((offset, correlation))
}

fn node(baseline: [f64; 3], centre: [f64; 3]) -> Node {
    let centre = unit(centre);
    let phi = centre[1].atan2(centre[0]);
    let seen = std::array::from_fn(|k| baseline[k] - dot(baseline, centre) * centre[k]);
    let epi = match norm(seen) > 0.0 {
        true => unit(seen.map(|v| -v)),
        false => [0.0, 0.0, 1.0],
    };
    Node {
        centre,
        epi,
        perp: unit(cross(epi, centre)),
        phi,
    }
}

#[derive(Clone, Copy, Debug)]
enum PatchRefusal {
    ProjectedOut,
    SourceBoundary,
}

fn sample(
    map: &Reframe,
    plane: &Plane,
    lens: usize,
    node: Node,
    half: isize,
    step: f64,
    offset: [f64; 2],
) -> Result<Vec<f64>, PatchRefusal> {
    let mut out = Vec::with_capacity(((2 * half + 1).pow(2)) as usize);
    for i in -half..=half {
        for j in -half..=half {
            let ray = unit(std::array::from_fn(|k| {
                node.centre[k]
                    + node.perp[k] * (i as f64 * step + offset[0])
                    + node.epi[k] * (j as f64 * step + offset[1])
            }));
            let landing = map.project(lens, ray.map(|v| v as f32));
            if !landing.inside {
                return Err(PatchRefusal::ProjectedOut);
            }
            let luma = plane
                .at(f64::from(landing.pixel[0]), f64::from(landing.pixel[1]))
                .ok_or(PatchRefusal::SourceBoundary)?;
            out.push(luma);
        }
    }
    Ok(out)
}

fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let mean = |x: &[f64]| x.iter().sum::<f64>() / x.len() as f64;
    let (ma, mb) = (mean(a), mean(b));
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (x - ma, y - mb);
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    if aa > 0.0 && bb > 0.0 {
        ab / (aa * bb).sqrt()
    } else {
        0.0
    }
}

fn samples(a: &[f64], b: &[f64], _step: f64) -> Vec<RegistrationSample> {
    let side = (a.len() as f64).sqrt() as usize;
    let mut out = Vec::new();
    for row in 1..side - 1 {
        for column in 1..side - 1 {
            let at = row * side + column;
            // Derivatives in grid steps. `read` converts the resulting estimate
            // and covariance to physical radians together.
            let perp = (b[(row + 1) * side + column] - b[(row - 1) * side + column]) * 0.5;
            let epi = (b[row * side + column + 1] - b[row * side + column - 1]) * 0.5;
            out.push(RegistrationSample {
                gradient: [perp, epi],
                residual: a[at] - b[at],
                weight: 1.0,
            });
        }
    }
    out
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(v: [f64; 3]) -> f64 {
    dot(v, v).sqrt()
}
fn unit(v: [f64; 3]) -> [f64; 3] {
    let n = norm(v);
    if n > 0.0 { v.map(|x| x / n) } else { [0.0; 3] }
}

fn unit_f32(v: [f32; 3]) -> [f32; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n > 0.0 { v.map(|x| x / n) } else { [0.0; 3] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kjerag_meta::{Distortion, Intrinsics, Lens, Pose};
    #[test]
    fn baseline_axes_are_orthogonal_to_the_ray() {
        let node = node([0.0, 0.0, -0.033], [0.7f64.cos(), 0.7f64.sin(), 0.0]);
        assert!(dot(node.centre, node.epi).abs() < 1e-12);
        assert!(dot(node.centre, node.perp).abs() < 1e-12);
        assert!(dot(node.epi, node.perp).abs() < 1e-12);
    }

    fn lenses(back_pitch_deg: f64) -> Vec<Lens> {
        let intrinsics = Intrinsics {
            xi: 2.31494,
            fx: 3665.9397,
            fy: 3667.4194,
            cx: 1920.0,
            cy: 1920.0,
        };
        let distortion = Distortion {
            k1: 0.95820886,
            k2: -1.80141151,
            k3: 3.57555127,
            p1: 0.0,
            p2: 0.0,
        };
        vec![
            Lens {
                intrinsics,
                distortion,
                pose: Pose {
                    yaw_deg: 0.0,
                    pitch_deg: 0.0,
                    roll_deg: 90.0,
                    translation_m: [0.0; 3],
                },
                lens_type: 131,
            },
            Lens {
                intrinsics,
                distortion,
                pose: Pose {
                    yaw_deg: 0.0,
                    pitch_deg: back_pitch_deg,
                    roll_deg: 90.0,
                    translation_m: [0.0, 0.0, -0.033],
                },
                lens_type: 131,
            },
        ]
    }

    fn crossover_map(back_pitch_deg: f64) -> Reframe {
        Reframe::new(
            &lenses(back_pitch_deg),
            kjerag_render::Size::new(3840, 3840),
            kjerag_render::Camera {
                yaw: 90.0_f32.to_radians(),
                pitch: 0.0,
                fov: 70.0_f32.to_radians(),
            },
            kjerag_render::Held::default(),
            1.0,
            false,
            kjerag_render::Sampling::default(),
        )
    }

    #[test]
    fn traced_candidates_are_actual_two_lens_weight_roots() {
        let map = crossover_map(3.0);
        let candidates = visible_candidates(&map, 320, 320, [0.0, 0.0, -0.033]);
        assert!(!candidates.is_empty(), "the crossover should be visible");
        for candidate in &candidates {
            let blend = map.blend(candidate.view_ray);
            assert!(blend.weights[0] > 0.0 && blend.weights[1] > 0.0);
            assert!(blend.landings[0].inside && blend.landings[1].inside);
            assert!(
                (blend.weights[0] - blend.weights[1]).abs() < 1e-5,
                "root weighs {:?}",
                blend.weights
            );
        }
    }

    #[test]
    fn actual_crossover_follows_an_asymmetric_pose_not_body_z_zero() {
        let map = crossover_map(3.0);
        let candidates = visible_candidates(&map, 320, 320, [0.0, 0.0, -0.033]);
        let displaced = candidates
            .iter()
            .map(|candidate| candidate.node.centre[2].abs())
            .fold(0.0_f64, f64::max);
        assert!(
            displaced > 0.01,
            "the asymmetric lens pose should move its actual crossover off nominal body.z=0; got {displaced}"
        );
    }
    #[test]
    fn correlation_refuses_flat_content_by_reporting_no_agreement() {
        assert_eq!(correlation(&[1.0; 4], &[1.0; 4]), 0.0);
    }

    #[test]
    fn an_interior_peak_survives_missing_neighbouring_search_patches() {
        // The omitted shifts model a patch crossing the lens boundary.  They
        // are not entries with invented luma and must not invalidate the
        // complete shifts left inside the aperture.
        let legal = [([-1, 0], 0.61), ([0, 0], 0.98), ([1, 0], 0.42)];
        assert_eq!(peak(&legal, 3), Ok(([0, 0], 0.98)));
    }

    #[test]
    fn a_railed_or_tied_legal_peak_still_refuses() {
        assert_eq!(peak(&[([3, 0], 0.98)], 3), Err(Refused::NoPeak));
        assert_eq!(
            peak(&[([0, 0], 0.98), ([1, 0], 0.98)], 3),
            Err(Refused::NoPeak)
        );
        assert_eq!(peak(&[], 3), Err(Refused::NoCompletePatch));
    }

    fn planted_crossing(support: Support, shift: [f64; 2]) -> Vec<RegistrationSample> {
        let half = support.half();
        (-half..=half)
            .flat_map(|row| {
                (-half..=half).map(move |column| {
                    // A small non-collinear textured crossing.  This is a
                    // planted local linearization, not an image-size proxy:
                    // each rung gets its own angular support and grid count.
                    let gradient = [1.0 + row as f64 * 0.03, 0.7 + column as f64 * 0.02];
                    RegistrationSample {
                        residual: gradient[0] * shift[0] + gradient[1] * shift[1],
                        gradient,
                        weight: 1.0,
                    }
                })
            })
            .collect()
    }

    #[test]
    fn every_declared_support_recovers_the_same_planted_two_axis_shift() {
        let wanted = [0.37, -0.22];
        for support in SUPPORT_LADDER {
            let reading = local_warp::register(&planted_crossing(support, wanted))
                .expect("the planted crossing has two textured axes");
            assert!(
                (reading.displacement.x - wanted[0]).abs() < 1e-12,
                "span {} recovered {} instead of {}",
                support.span_deg,
                reading.displacement.x,
                wanted[0]
            );
            assert!((reading.displacement.y - wanted[1]).abs() < 1e-12);
            assert!(reading.condition.is_finite());
        }
    }
}
