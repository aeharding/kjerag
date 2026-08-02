//! A complete raw-lens patch, registered in the camera's epipolar frame.
//!
//! The rendered path may say which seam segment a named view exposes.  It is
//! never a source of matching pixels: these routines read only `Plane` and
//! project the body's rays through each raw lens.

use kjerag_media::Plane;
use kjerag_render::Reframe;

use crate::local_warp::{self, RegistrationSample};

/// One point on the body `z = 0` seam, with the axes the recorded baseline
/// gives it.  `perp` is the axis no physical disparity can reach; `epi` is
/// the epipolar axis, in the sign used by the old depth instrument.
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
    pub view_pixel: [u32; 2],
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

/// The fixed, angular instrument grid.  These are global physical quantities,
/// not image-pixel or per-view tuning knobs.
const STEP_DEG: f64 = 0.08;
const SPAN_DEG: f64 = 3.7;
const SEARCH_DEG: f64 = 3.0;
const BINS: usize = 72;

/// Candidate locations on the visible seam, one closest-to-seam pixel per
/// camera-frame azimuth bin.  A view not containing `body.z = 0` returns no
/// candidates rather than borrowing a location from another view.
pub fn visible_candidates(
    map: &Reframe,
    width: u32,
    height: u32,
    baseline: [f64; 3],
) -> Vec<Candidate> {
    let mut picked: [Option<(f64, Candidate)>; BINS] = [None; BINS];
    for y in 0..height {
        for x in 0..width {
            let uv = [
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            ];
            let Some(view) = map.view_ray(uv) else {
                continue;
            };
            let body = map.body_ray(view).map(f64::from);
            let length = norm(body);
            if length == 0.0 {
                continue;
            }
            let latitude = (body[2] / length).abs();
            // Keep only pixels whose cell can plausibly contain the zero
            // contour.  The later per-bin minimum is the actual contour
            // approximation; this guard keeps lens-axis views from creating
            // a fake seam candidate.
            if latitude > 2.0_f64.to_radians() {
                continue;
            }
            let phi = body[1].atan2(body[0]);
            let bin = ((phi.rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU * BINS as f64)
                .floor() as usize)
                % BINS;
            let candidate = Candidate {
                node: node(baseline, phi),
                view_pixel: [x, y],
            };
            if picked[bin].is_none_or(|(held, _)| latitude < held) {
                picked[bin] = Some((latitude, candidate));
            }
        }
    }
    picked
        .into_iter()
        .flatten()
        .map(|(_, candidate)| candidate)
        .collect()
}

/// Register and select the most informative complete raw patch.  A complete
/// patch is required at the winning offset in both lenses; there is no hole
/// fill or best-effort rectangle.
pub fn select(
    map: &Reframe,
    planes: &[Plane],
    candidates: &[Candidate],
) -> Result<Reading, Refused> {
    if candidates.is_empty() {
        return Err(Refused::NoVisibleSeam);
    }
    if planes.len() < 2 {
        return Err(Refused::NoCompletePatch);
    }
    let mut outside = 0usize;
    let mut aperture = 0usize;
    let mut best: Option<Reading> = None;
    for candidate in candidates {
        match read(map, &planes[0], &planes[1], *candidate) {
            Ok(reading) if best.is_none_or(|held| reading.score > held.score) => {
                best = Some(reading)
            }
            Ok(_) => {}
            Err(Refused::NoCompletePatch) => outside += 1,
            Err(Refused::Aperture) => aperture += 1,
            Err(_) => {}
        }
    }
    best.ok_or_else(|| {
        if outside == candidates.len() {
            Refused::NoCompletePatch
        } else if aperture + outside == candidates.len() {
            Refused::Aperture
        } else {
            Refused::NoPeak
        }
    })
}

fn read(
    map: &Reframe,
    front: &Plane,
    back: &Plane,
    candidate: Candidate,
) -> Result<Reading, Refused> {
    let step = STEP_DEG.to_radians();
    let half = (SPAN_DEG.to_radians() / (2.0 * step)) as isize;
    let a = sample(map, front, 0, candidate.node, half, step, [0.0, 0.0])
        .ok_or(Refused::NoCompletePatch)?;
    let coarse = SEARCH_DEG.to_radians() / step;
    let mut winner: Option<([isize; 2], f64)> = None;
    // This is deliberately an unconstrained 2-D search.  A physical depth
    // hypothesis can later explain the epi term, but must not manufacture a
    // zero perp term before the evidence has been read.
    for i in -(coarse as isize)..=(coarse as isize) {
        for j in -(coarse as isize)..=(coarse as isize) {
            let offset = [i as f64 * step, j as f64 * step];
            let Some(b) = sample(map, back, 1, candidate.node, half, step, offset) else {
                continue;
            };
            let r = correlation(&a, &b);
            if winner.is_none_or(|(_, held)| r > held) {
                winner = Some(([i, j], r));
            }
        }
    }
    let ([i, j], correlation) = winner.ok_or(Refused::NoCompletePatch)?;
    if i.abs() as f64 >= coarse || j.abs() as f64 >= coarse {
        return Err(Refused::NoPeak);
    }
    let offset = [i as f64 * step, j as f64 * step];
    let b =
        sample(map, back, 1, candidate.node, half, step, offset).ok_or(Refused::NoCompletePatch)?;
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

fn node(baseline: [f64; 3], phi: f64) -> Node {
    let (sin, cos) = phi.sin_cos();
    let centre = [cos, sin, 0.0];
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

fn sample(
    map: &Reframe,
    plane: &Plane,
    lens: usize,
    node: Node,
    half: isize,
    step: f64,
    offset: [f64; 2],
) -> Option<Vec<f64>> {
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
                return None;
            }
            out.push(plane.at(f64::from(landing.pixel[0]), f64::from(landing.pixel[1]))?);
        }
    }
    Some(out)
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn baseline_axes_are_orthogonal_to_the_ray() {
        let node = node([0.0, 0.0, -0.033], 0.7);
        assert!(dot(node.centre, node.epi).abs() < 1e-12);
        assert!(dot(node.centre, node.perp).abs() < 1e-12);
        assert!(dot(node.epi, node.perp).abs() < 1e-12);
    }
    #[test]
    fn correlation_refuses_flat_content_by_reporting_no_agreement() {
        assert_eq!(correlation(&[1.0; 4], &[1.0; 4]), 0.0);
    }
}
