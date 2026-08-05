//! How far apart the two lenses draw the same thing, measured along the seam
//! crossing a named view actually shows.
//!
//! The gap this closes: `step` measures where a **horizon** lands either side
//! of the seam, and needs a horizon to measure. On the owner's 2026-05-01
//! views the scenery it fits is a ridge line rather than a great circle and it
//! fits at 51 to 86 px rms, so no seam-fix candidate could be screened there
//! before it reached his eyes. This does not trace scenery. It takes the
//! contour where the pass hands the picture over, and at fixed points along it
//! asks the two raw lens pictures how far apart they draw the same content.
//!
//! **What it reads.** Only [`Plane`], the decoded raw lens frames, projected
//! through the app's own [`Reframe`]. No composited, blended, tone-mapped or
//! band-bent output pixel is ever matched. The projection is the unbent one,
//! which is the calibration's own geometry: the per-frame band bend is a
//! second layer applied on top of it and is deliberately not in this number.
//! That also makes a reading independent of how many frames of film ran into
//! the one being measured, which the rendered picture is not.
//!
//! **The axes.** At a direction on the seam, `epi` is the epipolar axis, the
//! only one a subject's distance can displace content along, and `perp` is the
//! seam circle's own tangent, which parallax cannot reach at any distance
//! (docs/research/seam-two-axis.md 1). So a perp disagreement is the camera
//! and an epi disagreement is the camera plus whatever the scene's depth adds.
//!
//! Ported from the `feat/warp` branch and audited on 2026-08-05: the actual
//! 50/50 contour tracer, the strict no-zero-fill patch sampler, the numeric
//! site response, and (in [`super::registration`]) the solve. What is rewritten
//! is said at [`peak`], [`correlation`] and [`measure`].

use kjerag_media::Plane;
use kjerag_render::{Reframe, Size};

use crate::registration;

/// A measured quantity on the seam's own two axes, in whatever unit the
/// producer names. One type for all of them so that no conversion in this
/// module can silently swap the pair: the ported tree carried `[perp, epi]`
/// arrays and `{epi, perp}` structs side by side.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Axes {
    /// Along the seam circle's own tangent. Calibration only.
    pub perp: f64,
    /// Along the lens-to-lens baseline projected into the tangent plane.
    /// Calibration plus depth.
    pub epi: f64,
}

/// One point of the rendered crossover contour, with the axes the recorded
/// baseline gives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub centre: [f64; 3],
    pub perp: [f64; 3],
    pub epi: [f64; 3],
    /// Azimuth about the body's +x, in radians: the arc position along the
    /// crossing, and the one label a site keeps under any calibration.
    pub phi: f64,
}

/// A place on the visible crossing that gets measured.
///
/// The view ray and pixel are where the contour was traced, kept so a reading
/// can be pointed at in a render. They are a location and nothing more: the
/// evidence is the raw planes below.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Site {
    pub node: Node,
    pub view_ray: [f32; 3],
    pub view_pixel: [f32; 2],
}

/// The patch and search the whole run uses.
///
/// Angular rather than pixel sized so it means the same physical
/// neighbourhood at every azimuth and in every view, and global rather than
/// per site so no reading can be the one that got a bigger patch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Support {
    /// The patch's whole width.
    pub span_deg: f64,
    /// How far the coarse search reaches on each axis.
    pub search_deg: f64,
    /// One grid cell, which is also the coarse search's stride.
    pub step_deg: f64,
}

impl Support {
    pub fn valid(self) -> bool {
        [self.span_deg, self.search_deg, self.step_deg]
            .into_iter()
            .all(|value| value.is_finite() && value > 0.0)
            && self.half() >= 1
            && self.reach() >= 1
    }

    /// Half the patch, in grid cells.
    fn half(self) -> isize {
        (self.span_deg / (2.0 * self.step_deg)).round() as isize
    }

    /// How far the coarse search reaches, in grid cells.
    fn reach(self) -> isize {
        (self.search_deg / self.step_deg).floor() as isize
    }

    fn step_rad(self) -> f64 {
        self.step_deg.to_radians()
    }
}

/// What a reading has to clear to be a reading.
///
/// Both are properties of the evidence rather than of the answer: a patch with
/// no contrast in it and a correlation peak no better than the noise are the
/// two ways glare and blank sky produce a number that means nothing. Refusing
/// is the honest reading there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Floor {
    /// The reference patch's luma standard deviation, in 8-bit codes.
    pub contrast: f64,
    /// The winning normalized correlation.
    pub agreement: f64,
}

/// How far apart the two lenses draw the content at one site.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub site: Site,
    /// Where the target lens draws what the reference lens draws at the site,
    /// as a body-frame angle from it. Radians.
    pub shift_rad: Axes,
    /// One standard deviation of that, from the solve's own residual. Radians.
    pub sigma_rad: Axes,
    pub correlation: f64,
    pub condition: f64,
}

/// Why a site has no reading. Every one of them is a refusal to state a
/// number, and none of them is a small number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Refused {
    /// The reference patch left one lens's picture or the delivered frame.
    NoPatch,
    /// The reference patch has no contrast to match: blank sky, or glare.
    Flat,
    /// No search offset produced a target patch that was both complete and
    /// not perfectly uniform. Perfectly uniform is all this tests: a patch
    /// with a hundredth of a code in it correlates and is judged on the
    /// correlation, not here.
    NoTarget,
    /// The best offset sits on the search boundary, so the declared search did
    /// not contain the answer. Widening the search is not the fix: it lets
    /// content elsewhere win, and the spread of the run says so.
    Railed,
    /// Two offsets matched equally well: repeating content did not identify
    /// one shift.
    Tied,
    /// The peak correlation is below the declared floor. Carries the peak, so
    /// that where the floor was drawn stays visible in the table.
    Weak(f64),
    /// The patch's usable gradients lie on one line, so only one axis of the
    /// two is observed. A straight edge and nothing else reaches this.
    Aperture,
    /// The solve had too few or malformed samples.
    NoSolve,
    /// The along-seam reading is not something a camera can do: it sits
    /// further from this crossing's own along-seam value than [`Plausible`]
    /// allows. Carries the departure in radians. See [`Plausible`] for why
    /// this outranks the correlation.
    PerpImplausible(f64),
}

impl Refused {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoPatch => "no-patch",
            Self::Flat => "flat",
            Self::NoTarget => "no-target",
            Self::Railed => "railed",
            Self::Tied => "tied",
            Self::Weak(_) => "weak",
            Self::Aperture => "aperture",
            Self::NoSolve => "no-solve",
            Self::PerpImplausible(_) => "perp-implausible",
        }
    }

    /// The peak correlation behind this refusal, where there was one.
    pub fn correlation(self) -> Option<f64> {
        match self {
            Self::Weak(peak) => Some(peak),
            _ => None,
        }
    }
}

/// What a reading's along-seam term is judged against, and how far from it a
/// reading may sit.
///
/// The along-seam axis is the one **no depth can reach** at any distance
/// (docs/research/seam-two-axis.md 1), and a file's calibration does not
/// change while it plays. So one crossing's along-seam term is one number
/// plus a slow trend along its arc, and a reading far from it is a
/// correlation that locked onto the wrong feature rather than a camera doing
/// something. That makes this a stronger test than the correlation, and it
/// **outranks** it: measured on the owner's 2026-05-01 flight, sites passing
/// at up to 0.92 agreement read along-seam values 25 to 46 source px from
/// their own crossing's.
///
/// **What it is not.** It is a tolerance filter on a physical argument, not a
/// validated classifier. There is no measured population of known-wrong
/// readings to draw a cut against: what exists is the physical argument
/// above, a mechanism test ([`gate`]'s planted site), and one consequence the
/// gate cannot have engineered, which is that removing sites on the
/// **along-seam** axis improves the **epipolar** axis's reproducibility
/// across time, a channel this never inspects. An earlier version of this
/// comment claimed a two-view control as validation. It is not one:
/// [`measure`] builds its patches on body-fixed axes, so the view rotation
/// cancels exactly and two views of the same body direction agree to
/// 0.0005 px whether or not the reading is any good.
///
/// It cannot help a run that is more than half wrong: the reference is a
/// median, so it survives contamination up to half and no further. Nor can it
/// help a run whose own scatter is the size of the tolerance, and there
/// [`Self::measured`] withholds a reference rather than judging against a
/// number it does not have.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plausible {
    /// The along-seam value of this crossing, in radians.
    pub reference_rad: f64,
    /// The median absolute deviation behind it, or zero for a declared one.
    pub spread_rad: f64,
    /// How far a reading may sit from the reference, in radians.
    pub tolerance_rad: f64,
    /// How many readings the reference was taken over. Zero when declared.
    pub from: usize,
}

impl Plausible {
    /// Below this many readings a crossing cannot say what its own along-seam
    /// value is, and the honest answer is to gate nothing rather than to gate
    /// against two numbers.
    pub const ENOUGH: usize = 5;

    /// How much of the tolerance a crossing's own scatter may occupy before
    /// its middle stops meaning anything.
    ///
    /// A reference is only a reference if the readings behind it agree with
    /// each other more closely than the tolerance it is about to judge them
    /// by. Without this a run whose own scatter was the size of the tolerance
    /// still produced a number, and gating against it kept the junk and put
    /// the honest core near refusal; one recorded run did exactly that.
    ///
    /// Two fifths, from 33 recorded runs. Sorted, their scatters run 0.03,
    /// 0.05 ... 0.31, 0.32, 0.34, 0.35, then **0.53, 0.57, 1.03** of the
    /// tolerance. **0.35 to 0.53 is the longest stretch of that list with
    /// nothing in it below 1.0**, and the three runs above it are the ones
    /// whose middle meant nothing. So unlike the per-reading cut in
    /// [`Self::tolerance_rad`], which sits in a populated continuum, this one
    /// does sit in a gap in the evidence. (The one wider stretch, 0.57 to
    /// 1.03, is bounded above by a run whose scatter exceeds the whole
    /// tolerance, which is not a place to put a cut.)
    pub const STEADY: f64 = 0.40;

    /// This crossing's own middle, which is where a reference comes from when
    /// the caller does not bring one. `None` where the crossing cannot state
    /// one: too few readings, or a scatter too wide to have a middle.
    pub fn measured(readings: &[Reading], tolerance_rad: f64) -> Option<Self> {
        let perp: Vec<f64> = readings.iter().map(|r| r.shift_rad.perp).collect();
        if perp.len() < Self::ENOUGH {
            return None;
        }
        let reference_rad = middle(&perp);
        let spread_rad = middle(
            &perp
                .iter()
                .map(|v| (v - reference_rad).abs())
                .collect::<Vec<_>>(),
        );
        (spread_rad <= Self::STEADY * tolerance_rad).then_some(Self {
            reference_rad,
            spread_rad,
            tolerance_rad,
            from: perp.len(),
        })
    }

    /// The scatter a crossing reported, whether or not it was steady enough
    /// to become a reference. For saying why one was withheld.
    pub fn scatter(readings: &[Reading]) -> Option<f64> {
        let perp: Vec<f64> = readings.iter().map(|r| r.shift_rad.perp).collect();
        (perp.len() >= Self::ENOUGH).then(|| {
            let middle_of = middle(&perp);
            middle(
                &perp
                    .iter()
                    .map(|v| (v - middle_of).abs())
                    .collect::<Vec<_>>(),
            )
        })
    }

    /// A reference the caller brings, from another run or another file. It is
    /// per crossing and not per camera: a principal-point error is one cycle
    /// round the azimuth, so the far side of the seam circle is a different
    /// number and not this one.
    pub fn declared(reference_rad: f64, tolerance_rad: f64) -> Self {
        Self {
            reference_rad,
            spread_rad: 0.0,
            tolerance_rad,
            from: 0,
        }
    }

    pub fn departure_rad(self, reading: &Reading) -> f64 {
        (reading.shift_rad.perp - self.reference_rad).abs()
    }
}

/// Refuse the readings of one crossing that its own along-seam value says are
/// not readings. Answers how many it took.
pub fn gate(results: &mut [Result<Reading, Refused>], plausible: Plausible) -> usize {
    let mut taken = 0;
    for result in results {
        let Ok(reading) = result else { continue };
        let departure = plausible.departure_rad(reading);
        if departure > plausible.tolerance_rad {
            *result = Err(Refused::PerpImplausible(departure));
            taken += 1;
        }
    }
    taken
}

fn middle(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    match sorted.len() % 2 {
        0 => (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0,
        _ => sorted[sorted.len() / 2],
    }
}

/// Which decoded picture a patch is read from, and which lens's projection
/// puts it there. They are separate because the null control reads one lens
/// twice, and a pair that disagreed would be measuring nothing.
#[derive(Clone, Copy)]
pub struct Source<'a> {
    pub plane: &'a Plane,
    pub lens: usize,
}

/// Candidate locations on the visible crossover, one per azimuth bin.
///
/// The body's `z = 0` circle is a useful nominal seam, but it is not the line
/// the pass hands over on after a selected lens calibration: the pass centres
/// the crossover on the two optical axes, then coverage depth moves its final
/// 50/50 point again. Trace the latter, from the same `Blend` the fragment
/// shader uses. A view with no two-lens 50/50 contour in it returns no sites
/// rather than borrowing a nominal body-circle location.
///
/// `bins` is the arc resolution: the ported tracer had 72 of them fixed, which
/// is 5 degrees of azimuth and about a dozen sites in a 55 degree view. The
/// structure this instrument has to resolve is a few pixels wide, so the count
/// is the caller's. It is part of a reading: two runs at different `bins` are
/// two different sets of sites and their tables do not compare.
///
/// **A bin keeps the root nearest its own centre**, which is the fix for the
/// worst defect this instrument has had. The ported tracer kept the root with
/// the largest `min(blend.weights)`, on the argument that it was the one
/// furthest inside both lenses. It is not: `Reframe::blend` normalizes the
/// pair to sum 1 and a 50/50 root is where the two are equal, so that score is
/// **exactly 0.5 at every candidate** and the twenty-odd candidates a bin
/// holds were separated by the last bit of the bisection's `f32`. A
/// calibration change of a ten-thousandth of a degree then moved the reported
/// medians by about a view pixel, and a rerun of the same command did not
/// reproduce its own table. Azimuth is what every comparison this instrument
/// makes is keyed on, so azimuth is what a site is placed by.
pub fn trace(map: &Reframe, size: Size, baseline: [f64; 3], bins: usize) -> Vec<Site> {
    let (width, height) = (size.width, size.height);
    // Smaller is better: how far this root sits from its bin's centre.
    let mut picked: Vec<Option<(f64, Site)>> = vec![None; bins];
    let mut samples = vec![None; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let uv = [
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            ];
            samples[(y * width + x) as usize] = crossover(map, uv);
        }
    }
    for y in 0..height {
        for x in 0..width {
            let Some(here) = samples[(y * width + x) as usize] else {
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
                let Some(view) = root(map, here, next) else {
                    continue;
                };
                let body = unit(map.body_ray(view).map(f64::from));
                if norm(body) == 0.0 {
                    continue;
                }
                let turn = body[1].atan2(body[0]).rem_euclid(std::f64::consts::TAU)
                    / std::f64::consts::TAU
                    * bins as f64;
                let bin = (turn.floor() as usize) % bins;
                let off_centre = (turn - turn.floor() - 0.5).abs();
                let site = Site {
                    node: node(baseline, body),
                    view_ray: view,
                    view_pixel: [
                        x as f32 + 0.5 + dx as f32 * 0.5,
                        y as f32 + 0.5 + dy as f32 * 0.5,
                    ],
                };
                if picked[bin].is_none_or(|(held, _)| off_centre < held) {
                    picked[bin] = Some((off_centre, site));
                }
            }
        }
    }
    let mut sites: Vec<Site> = picked.into_iter().flatten().map(|(_, site)| site).collect();
    sites.sort_by(|one, other| one.node.phi.total_cmp(&other.node.phi));
    sites
}

/// One raster sample that can take part in a genuine two-lens crossover. The
/// score is the signed final rendered weight difference, not the nominal
/// lens-axis difference; coverage-depth claims are deliberately included.
#[derive(Clone, Copy)]
struct Crossover {
    view: [f32; 3],
    difference: f32,
}

fn crossover(map: &Reframe, uv: [f32; 2]) -> Option<Crossover> {
    let view = map.view_ray(uv)?;
    Some(Crossover {
        view,
        difference: difference(map, view)?,
    })
}

fn difference(map: &Reframe, view: [f32; 3]) -> Option<f32> {
    let blend = map.blend(view);
    two_lens(map, view).then_some(blend.weights[0] - blend.weights[1])
}

fn two_lens(map: &Reframe, view: [f32; 3]) -> bool {
    let blend = map.blend(view);
    blend.weights[0] > 0.0
        && blend.weights[1] > 0.0
        && blend.landings[0].inside
        && blend.landings[1].inside
}

/// A zero of the final rendered weights along one raster edge. The bisection
/// is in view-ray space, which is enough for a subpixel contour root and lets
/// the runtime `Blend` remain the sole definition of the crossover.
fn root(map: &Reframe, left: Crossover, right: Crossover) -> Option<[f32; 3]> {
    if left.difference == 0.0 {
        return two_lens(map, left.view).then_some(left.view);
    }
    if right.difference == 0.0 {
        return two_lens(map, right.view).then_some(right.view);
    }
    if left.difference.signum() == right.difference.signum() {
        return None;
    }
    let (mut low, mut high) = (left.view, right.view);
    let low_sign = left.difference.signum();
    for _ in 0..24 {
        let middle = unit_f32(std::array::from_fn(|axis| 0.5 * (low[axis] + high[axis])));
        if difference(map, middle)?.signum() == low_sign {
            low = middle;
        } else {
            high = middle;
        }
    }
    let found = unit_f32(std::array::from_fn(|axis| 0.5 * (low[axis] + high[axis])));
    two_lens(map, found).then_some(found)
}

/// The seam's axes at one direction, from the recorded baseline.
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

/// How far apart the two pictures draw the content at one site.
///
/// Rewritten from the ported `read`, which selected one winning candidate over
/// a whole view. This reports every site separately, because sites a fraction
/// of a degree apart are not independent evidence and pooling them was the
/// defect that made the ported tree's significance claims meaningless.
///
/// The two lens sources are named rather than assumed to be 0 and 1, so the
/// null control (one lens against itself, which must read exactly zero) runs
/// the same code as a measurement.
pub fn measure(
    map: &Reframe,
    reference: Source<'_>,
    target: Source<'_>,
    site: Site,
    support: Support,
    floor: Floor,
) -> Result<Reading, Refused> {
    let step = support.step_rad();
    let half = support.half();
    let held = patch(map, reference, site.node, half, step, [0.0; 2]).ok_or(Refused::NoPatch)?;
    if spread(&held) < floor.contrast {
        return Err(Refused::Flat);
    }
    let reach = support.reach();
    let mut legal = Vec::new();
    for perp in -reach..=reach {
        for epi in -reach..=reach {
            let offset = [perp as f64 * step, epi as f64 * step];
            // A search offset the target cannot supply, or supplies with no
            // contrast in it, is one candidate missing. It is not evidence
            // against the site: the ported estimator returned a flat
            // candidate's refusal out of the whole site.
            let Some(found) = patch(map, target, site.node, half, step, offset) else {
                continue;
            };
            let Some(score) = correlation(&held, &found) else {
                continue;
            };
            legal.push(([perp, epi], score));
        }
    }
    let (steps, correlation) = peak(&legal, reach)?;
    if correlation < floor.agreement {
        return Err(Refused::Weak(correlation));
    }
    let offset = [steps[0] as f64 * step, steps[1] as f64 * step];
    let found = patch(map, target, site.node, half, step, offset).ok_or(Refused::NoPatch)?;
    let fitted = registration::register(&gradients(&held, &found)).map_err(|why| match why {
        registration::Refused::Aperture => Refused::Aperture,
        _ => Refused::NoSolve,
    })?;
    // The solve is in grid steps, because its gradients are differences one
    // grid cell apart. Both the estimate and its covariance become radians
    // here, together.
    Ok(Reading {
        site,
        shift_rad: Axes {
            perp: offset[0] + fitted.displacement.x * step,
            epi: offset[1] + fitted.displacement.y * step,
        },
        sigma_rad: Axes {
            perp: fitted.covariance.xx.max(0.0).sqrt() * step,
            epi: fitted.covariance.yy.max(0.0).sqrt() * step,
        },
        correlation,
        condition: fitted.condition,
    })
}

/// How close two scores have to be to be the same score.
///
/// One rule, used everywhere a correlation is compared. The ported tree had
/// two: a `1e-6` band in one estimator and exact `f64` equality in another, so
/// the same repeating content was ambiguous to one and decided by scan order
/// in the other. Exact equality is the one that had to go: two offsets over
/// near-flat content differ in the last bits and neither answer is real.
const TIE: f64 = 1e-6;

/// Pick one maximum from the offsets that produced a usable target patch.
///
/// A boundary maximum says the declared search did not contain the answer;
/// two maxima within [`TIE`] say the content did not identify one. Both are
/// refusals. Missing offsets are not evidence against the offsets that remain.
fn peak(legal: &[([isize; 2], f64)], reach: isize) -> Result<([isize; 2], f64), Refused> {
    let Some((steps, best)) = legal
        .iter()
        .copied()
        .max_by(|one, other| one.1.total_cmp(&other.1))
    else {
        return Err(Refused::NoTarget);
    };
    if legal
        .iter()
        .filter(|(_, score)| best - score <= TIE)
        .count()
        > 1
    {
        return Err(Refused::Tied);
    }
    if steps[0].abs() >= reach || steps[1].abs() >= reach {
        return Err(Refused::Railed);
    }
    Ok((steps, best))
}

/// One patch of one raw lens picture, on the site's own body-frame grid.
///
/// `None` where any one of its samples left the lens's picture or the
/// delivered frame: a patch is complete or it is not evidence. There is no
/// hole fill and no best-effort rectangle.
fn patch(
    map: &Reframe,
    source: Source<'_>,
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
            // The node is body-fixed so its epipolar axes stand still while
            // the named view turns; `project` takes the renderer's view-space
            // ray. Passing the body ray straight in mixes the two frames and
            // reports a real root as projected out merely because the view
            // was rotated.
            let landing = map.project(source.lens, map.view_ray_from_body(ray.map(|v| v as f32)));
            if !landing.inside {
                return None;
            }
            out.push(
                source
                    .plane
                    .at(f64::from(landing.pixel[0]), f64::from(landing.pixel[1]))?,
            );
        }
    }
    Some(out)
}

/// Zero-mean normalized cross correlation, or `None` where either patch has
/// no variance at all and there is nothing to correlate.
fn correlation(a: &[f64], b: &[f64]) -> Option<f64> {
    let mean = |x: &[f64]| x.iter().sum::<f64>() / x.len() as f64;
    let (ma, mb) = (mean(a), mean(b));
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (x - ma, y - mb);
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    (aa > 0.0 && bb > 0.0).then(|| ab / (aa * bb).sqrt())
}

/// A patch's luma standard deviation, in the codes it was decoded in.
fn spread(patch: &[f64]) -> f64 {
    let mean = patch.iter().sum::<f64>() / patch.len() as f64;
    (patch.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / patch.len() as f64).sqrt()
}

/// The solve's samples: target gradients in grid steps, and the luma the
/// reference has left over at the same cell.
fn gradients(held: &[f64], found: &[f64]) -> Vec<registration::Sample> {
    let side = (held.len() as f64).sqrt() as usize;
    let mut out = Vec::new();
    for row in 1..side - 1 {
        for column in 1..side - 1 {
            let at = row * side + column;
            let perp = (found[(row + 1) * side + column] - found[(row - 1) * side + column]) * 0.5;
            let epi = (found[row * side + column + 1] - found[row * side + column - 1]) * 0.5;
            out.push(registration::Sample {
                gradient: [perp, epi],
                residual: held[at] - found[at],
                weight: 1.0,
            });
        }
    }
    out
}

/// Why a site's pixel scale, or its response to a calibration knob, cannot be
/// stated. This reads no planes and has no texture or peak in it: it only asks
/// whether the projection locally describes the site at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoScale {
    /// The site, or a probe used to express it in camera axes, did not land in
    /// the lens in one of the maps.
    ProjectedOut,
    /// The projection is locally singular at the site, so image motion cannot
    /// be resolved into a two-axis camera displacement.
    Singular,
    /// The declared central difference was not a finite positive half-step.
    InvalidStep,
}

/// How far one degree of body angle moves this site in the lens's own raster,
/// on each of the seam's axes. The probe is the local parameterization the
/// ported site response takes its columns from.
const PROBE_DEG: f64 = 0.01;

/// The columns of `d(lens pixel) / d(body angle)` at a site, `perp` first.
///
/// They are not orthogonal in general, which is why a conversion out of
/// radians uses the column and not one scalar focal length.
fn columns(map: &Reframe, lens: usize, node: Node, at: [f64; 3]) -> Option<[[f64; 2]; 2]> {
    let probe = PROBE_DEG.to_radians();
    let landing = |ray: [f64; 3]| {
        let found = map.project(lens, map.view_ray_from_body(ray.map(|axis| axis as f32)));
        found.inside.then_some(found.pixel)
    };
    let column = |axis: [f64; 3]| {
        let at_offset = |sign: f64| {
            landing(unit(std::array::from_fn(|index| {
                at[index] + sign * probe * axis[index]
            })))
        };
        let (low, high) = (at_offset(-1.0)?, at_offset(1.0)?);
        Some([
            f64::from(high[0] - low[0]) / (2.0 * probe),
            f64::from(high[1] - low[1]) / (2.0 * probe),
        ])
    };
    Some([column(node.perp)?, column(node.epi)?])
}

/// How many pixels of one lens's own raster a radian of body angle is worth at
/// a site, on each axis.
pub fn source_scale(map: &Reframe, lens: usize, site: Site) -> Result<Axes, NoScale> {
    let columns = columns(map, lens, site.node, site.node.centre).ok_or(NoScale::ProjectedOut)?;
    Ok(Axes {
        perp: norm2(columns[0]),
        epi: norm2(columns[1]),
    })
}

/// How many pixels of the **output** a radian of body angle is worth at a
/// site, in the view that traced it.
///
/// Taken from `Reframe::view_ray` itself rather than from the field of view,
/// so a wide view's compression toward its edges is in the number instead of
/// being assumed away.
pub fn view_scale(map: &Reframe, site: Site, size: Size) -> Result<Axes, NoScale> {
    let (width, height) = (f64::from(size.width), f64::from(size.height));
    let uv = |du: f64, dv: f64| {
        let uv = [
            (f64::from(site.view_pixel[0]) + du) / width,
            (f64::from(site.view_pixel[1]) + dv) / height,
        ];
        Some(unit(map.view_ray(uv.map(|v| v as f32))?.map(f64::from)))
    };
    let axes = [site.node.perp, site.node.epi]
        .map(|axis| unit_f32(map.view_ray_from_body(axis.map(|v| v as f32))).map(f64::from));
    // Rows are the seam's axes and columns are the output's, in radians of
    // body angle per output pixel.
    let mut jacobian = [[0.0; 2]; 2];
    for (column, (du, dv)) in [(1.0, 0.0), (0.0, 1.0)].into_iter().enumerate() {
        let (low, high) = (
            uv(-du, -dv).ok_or(NoScale::ProjectedOut)?,
            uv(du, dv).ok_or(NoScale::ProjectedOut)?,
        );
        let along: [f64; 3] = std::array::from_fn(|k| (high[k] - low[k]) / 2.0);
        for (row, axis) in axes.iter().enumerate() {
            jacobian[row][column] = dot(along, *axis);
        }
    }
    let determinant = jacobian[0][0] * jacobian[1][1] - jacobian[0][1] * jacobian[1][0];
    if !determinant.is_finite() || determinant == 0.0 {
        return Err(NoScale::Singular);
    }
    // Inverted, the columns are output pixels per radian on each seam axis.
    Ok(Axes {
        perp: norm2([jacobian[1][1], -jacobian[1][0]]) / determinant.abs(),
        epi: norm2([-jacobian[0][1], jacobian[0][0]]) / determinant.abs(),
    })
}

/// The central-difference response of a fixed site to one calibration knob.
///
/// `minus` and `plus` must be independently built maps from the same instant,
/// camera and horizon state as `base`, differing only in that knob at
/// `-half_step` and `+half_step`. The site is deliberately not retraced on the
/// perturbed maps: a response is about one declared location.
///
/// The result is radians of body-frame displacement per unit of that knob, in
/// the same sense a [`Reading`] is: it is how far the moving lens's picture of
/// the content at this site travels. So a plant of `amount` should move a
/// reading by `response * amount`, which is what the plant control asserts.
pub fn response(
    base: &Reframe,
    minus: &Reframe,
    plus: &Reframe,
    lens: usize,
    site: Site,
    half_step: f64,
) -> Result<Axes, NoScale> {
    if !half_step.is_finite() || half_step <= 0.0 {
        return Err(NoScale::InvalidStep);
    }
    let at = site.node.centre;
    let landing = |map: &Reframe| {
        let found = map.project(lens, map.view_ray_from_body(at.map(|axis| axis as f32)));
        found.inside.then_some(found.pixel)
    };
    let (before, after) = (
        landing(minus).ok_or(NoScale::ProjectedOut)?,
        landing(plus).ok_or(NoScale::ProjectedOut)?,
    );
    // The local parameterization comes only from the unchanged map, so a
    // calibration-induced change is not confused with a moving crossover or a
    // re-traced, content-selected site.
    let [perp, epi] = columns(base, lens, site.node, at).ok_or(NoScale::ProjectedOut)?;
    let determinant = perp[0] * epi[1] - perp[1] * epi[0];
    if !determinant.is_finite() || determinant.abs() < 1e-9 {
        return Err(NoScale::Singular);
    }
    let moved = [
        f64::from(after[0] - before[0]) / (2.0 * half_step),
        f64::from(after[1] - before[1]) / (2.0 * half_step),
    ];
    if moved.iter().any(|value| !value.is_finite()) {
        return Err(NoScale::ProjectedOut);
    }
    // `J [perp, epi] = image motion`, so the content follows `-J^-1 d`.
    Ok(Axes {
        perp: -(epi[1] * moved[0] - epi[0] * moved[1]) / determinant,
        epi: -(perp[0] * moved[1] - perp[1] * moved[0]) / determinant,
    })
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

fn norm2(v: [f64; 2]) -> f64 {
    v[0].hypot(v[1])
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

    const BASELINE: [f64; 3] = [0.0, 0.0, -0.033];
    const RASTER: Size = Size {
        width: 320,
        height: 320,
    };

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
        let pose = |pitch_deg: f64, translation_m: [f64; 3]| Pose {
            yaw_deg: 0.0,
            pitch_deg,
            roll_deg: 90.0,
            translation_m,
        };
        vec![
            Lens {
                intrinsics,
                distortion,
                pose: pose(0.0, [0.0; 3]),
                lens_type: 131,
            },
            Lens {
                intrinsics,
                distortion,
                pose: pose(back_pitch_deg, BASELINE),
                lens_type: 131,
            },
        ]
    }

    fn map(back_pitch_deg: f64) -> Reframe {
        Reframe::new(
            &lenses(back_pitch_deg),
            Size::new(3840, 3840),
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

    fn sites(map: &Reframe) -> Vec<Site> {
        trace(map, RASTER, BASELINE, 72)
    }

    fn first(map: &Reframe) -> Site {
        sites(map)
            .into_iter()
            .next()
            .expect("the crossover fixture exposes a root")
    }

    /// A flat grey plane, which every patch of is complete and none of is
    /// evidence.
    fn blank(code: u8) -> Plane {
        let size = Size::new(3840, 3840);
        Plane {
            luma: vec![code; (size.width * size.height) as usize],
            stride: size.width as usize,
            size,
            chroma: None,
        }
    }

    /// A deterministic textured plane. Both axes vary and neither is periodic
    /// over a patch, so a two-axis peak exists and is unique.
    fn textured() -> Plane {
        let size = Size::new(3840, 3840);
        let luma = (0..size.height)
            .flat_map(|y| {
                (0..size.width).map(move |x| {
                    let (x, y) = (f64::from(x), f64::from(y));
                    let value = 128.0
                        + 60.0 * (x * 0.017).sin() * (y * 0.011).cos()
                        + 40.0 * ((x + y) * 0.0043).sin();
                    value.clamp(0.0, 255.0) as u8
                })
            })
            .collect();
        Plane {
            luma,
            stride: size.width as usize,
            size,
            chroma: None,
        }
    }

    const SUPPORT: Support = Support {
        span_deg: 1.10,
        search_deg: 0.70,
        step_deg: 0.07,
    };
    const FLOOR: Floor = Floor {
        contrast: 2.0,
        agreement: 0.5,
    };

    #[test]
    fn the_seam_axes_are_orthogonal_to_the_direction_they_are_taken_at() {
        let node = node(BASELINE, [0.7f64.cos(), 0.7f64.sin(), 0.0]);
        assert!(dot(node.centre, node.epi).abs() < 1e-12);
        assert!(dot(node.centre, node.perp).abs() < 1e-12);
        assert!(dot(node.epi, node.perp).abs() < 1e-12);
    }

    #[test]
    fn traced_sites_are_actual_two_lens_weight_roots() {
        let map = map(3.0);
        let sites = sites(&map);
        assert!(!sites.is_empty(), "the crossover should be visible");
        for site in &sites {
            let blend = map.blend(site.view_ray);
            assert!(blend.weights[0] > 0.0 && blend.weights[1] > 0.0);
            assert!(blend.landings[0].inside && blend.landings[1].inside);
            assert!(
                (blend.weights[0] - blend.weights[1]).abs() < 1e-5,
                "root weighs {:?}",
                blend.weights
            );
            let recovered = map.view_ray_from_body(site.node.centre.map(|v| v as f32));
            for (axis, (found, traced)) in recovered.iter().zip(site.view_ray).enumerate() {
                assert!(
                    (found - traced).abs() < 1e-5,
                    "body-to-view inverse changed axis {axis}"
                );
            }
        }
    }

    /// One reading, on the seam's two axes, in degrees. The site is a bare
    /// one: what the gate weighs is the along-seam number, and tracing a real
    /// contour to get one would be measuring the tracer instead.
    fn reading(perp_deg: f64, epi_deg: f64, correlation: f64) -> Reading {
        Reading {
            site: Site {
                node: node(BASELINE, [1.0, 0.0, 0.0]),
                view_ray: [0.0, 0.0, 1.0],
                view_pixel: [0.0, 0.0],
            },
            shift_rad: Axes {
                perp: perp_deg.to_radians(),
                epi: epi_deg.to_radians(),
            },
            sigma_rad: Axes::default(),
            correlation,
            condition: 1.0,
        }
    }

    /// A crossing whose along-seam readings sit round a fifth of a degree,
    /// with one planted site that says something a camera cannot do.
    fn planted() -> Vec<Result<Reading, Refused>> {
        let mut out: Vec<Result<Reading, Refused>> = [-0.22, -0.19, -0.24, -0.20, -0.18, -0.21]
            .into_iter()
            .map(|perp| Ok(reading(perp, -0.40, 0.75)))
            .collect();
        // Along-seam by 1.2 degrees, at a correlation that clears every floor
        // the instrument has. Measured mismatches look exactly like this.
        out.insert(3, Ok(reading(1.20, -0.40, 0.92)));
        out
    }

    /// 0.40 degrees is 12.6 source px at the seam on this camera family.
    const TOLERANCE: f64 = 0.40;

    #[test]
    fn an_implausible_along_seam_reading_refuses_however_well_it_correlated() {
        let mut results = planted();
        let readings: Vec<Reading> = results.iter().copied().filter_map(Result::ok).collect();
        let plausible = Plausible::measured(&readings, TOLERANCE.to_radians())
            .expect("seven readings is enough for a reference");
        // The median is the point of it: one planted site 1.2 degrees out
        // does not drag the reference off the other six.
        assert!(
            (plausible.reference_rad.to_degrees() + 0.205).abs() < 0.02,
            "reference is {} deg",
            plausible.reference_rad.to_degrees()
        );
        assert_eq!(gate(&mut results, plausible), 1);
        match results[3] {
            Err(Refused::PerpImplausible(departure)) => {
                assert!((departure.to_degrees() - 1.405).abs() < 0.02);
            }
            ref other => panic!("the planted site was not refused: {other:?}"),
        }
        assert!(
            results
                .iter()
                .enumerate()
                .all(|(at, r)| at == 3 || r.is_ok()),
            "the gate took a site it had no business taking"
        );
        assert_eq!(
            results[3].as_ref().err().map(|why| why.label()),
            Some("perp-implausible")
        );
    }

    /// A crossing with too few readings cannot say what its own along-seam
    /// value is, and then the honest answer is to gate nothing.
    #[test]
    fn too_few_readings_state_no_reference_at_all() {
        let readings: Vec<Reading> = (0..Plausible::ENOUGH - 1)
            .map(|_| reading(-0.20, -0.40, 0.9))
            .collect();
        assert_eq!(Plausible::measured(&readings, TOLERANCE.to_radians()), None);
        assert!(Plausible::measured(&planted_readings(), TOLERANCE.to_radians()).is_some());
    }

    /// Nor can a crossing whose readings scatter as far as the tolerance is
    /// about to judge them by: its middle is not a value, and gating against
    /// it keeps the junk and refuses the honest core. One recorded run did
    /// exactly that.
    #[test]
    fn a_crossing_that_scatters_as_wide_as_the_tolerance_withholds_its_reference() {
        let tolerance = TOLERANCE.to_radians();
        let scattered: Vec<Reading> = [-0.40, -0.15, 0.05, 0.30, -0.05, 0.55, -0.60]
            .into_iter()
            .map(|perp| reading(perp, -0.40, 0.9))
            .collect();
        let scatter = Plausible::scatter(&scattered).expect("seven readings have a scatter");
        assert!(
            scatter > Plausible::STEADY * tolerance,
            "the fixture is not scattered: {} deg",
            scatter.to_degrees()
        );
        assert_eq!(Plausible::measured(&scattered, tolerance), None);
        // And a steady crossing still states one, with its scatter on it.
        let steady = Plausible::measured(&planted_readings(), tolerance).expect("steady");
        assert!(steady.spread_rad <= Plausible::STEADY * tolerance);
        assert!(
            (steady.spread_rad - Plausible::scatter(&planted_readings()).unwrap()).abs() < 1e-15
        );
    }

    /// A caller may bring the reference instead, from another run or another
    /// file, and then it is the caller's number and not this crossing's.
    #[test]
    fn a_declared_reference_replaces_the_crossings_own() {
        let mut results = planted();
        // Declared a degree and a half away: now it is the six that are
        // implausible and the planted one that is not.
        let declared = Plausible::declared(1.20_f64.to_radians(), TOLERANCE.to_radians());
        assert_eq!(declared.from, 0);
        assert_eq!(gate(&mut results, declared), 6);
        assert!(results[3].is_ok());
    }

    /// The null control reads exactly zero everywhere, so the gate has
    /// nothing to say about it and must not invent something.
    #[test]
    fn a_run_that_reads_zero_everywhere_survives_the_gate() {
        let mut results: Vec<Result<Reading, Refused>> =
            (0..8).map(|_| Ok(reading(0.0, 0.0, 1.0))).collect();
        let readings: Vec<Reading> = results.iter().copied().filter_map(Result::ok).collect();
        let plausible = Plausible::measured(&readings, TOLERANCE.to_radians()).expect("eight");
        assert_eq!(plausible.reference_rad, 0.0);
        assert_eq!(plausible.spread_rad, 0.0);
        assert_eq!(gate(&mut results, plausible), 0);
    }

    fn planted_readings() -> Vec<Reading> {
        planted().into_iter().filter_map(Result::ok).collect()
    }

    /// The traced contour is the pass's own handover, which an asymmetric lens
    /// pose moves off the nominal `body.z = 0` circle. A tracer that took the
    /// nominal circle would read the same everywhere here.
    #[test]
    fn the_traced_contour_follows_the_pose_and_not_nominal_body_z_zero() {
        let displaced = sites(&map(3.0))
            .iter()
            .map(|site| site.node.centre[2].abs())
            .fold(0.0_f64, f64::max);
        assert!(displaced > 0.01, "the crossover did not move: {displaced}");
    }

    /// The arc resolution is the caller's, and each bin holds at most one
    /// site, so asking for more of them cannot ask for fewer.
    #[test]
    fn more_arc_bins_resolve_more_sites_along_the_same_crossing() {
        let map = map(0.0);
        let coarse = trace(&map, RASTER, BASELINE, 72).len();
        let fine = trace(&map, RASTER, BASELINE, 720).len();
        assert!(fine > coarse, "{fine} sites is not finer than {coarse}");
    }

    /// A bin keeps the root nearest its own centre. The score it replaced was
    /// `min(blend.weights)`, which this asserts is the constant it was: the
    /// weights are normalized to sum 1 and a root is where they are equal, so
    /// every candidate scored exactly a half and the winner was whichever the
    /// bisection's last `f32` bit happened to favour.
    #[test]
    fn a_bin_keeps_the_root_nearest_its_centre_and_not_a_tied_one() {
        let map = map(3.0);
        let bins = 180;
        let width = std::f64::consts::TAU / bins as f64;
        for site in trace(&map, RASTER, BASELINE, bins) {
            let turn = site.node.phi.rem_euclid(std::f64::consts::TAU) / width;
            let off_centre = (turn - turn.floor() - 0.5).abs();
            // Half a bin is the whole of a bin, so anything under a quarter of
            // one says a centre was aimed at rather than landed on by chance.
            assert!(off_centre < 0.25, "site sits {off_centre} bins off centre");
            let weights = map.blend(site.view_ray).weights;
            assert!(
                (weights[0].min(weights[1]) - 0.5).abs() < 1e-6,
                "the replaced score was not constant after all: {weights:?}"
            );
        }
    }

    /// And the sites a run reports do not move when the calibration moves by
    /// less than the arithmetic can see. This is what the old selector failed:
    /// a ten-thousandth of a degree re-ordered its ties and moved the reported
    /// medians by about a view pixel.
    #[test]
    fn a_calibration_dither_too_small_to_be_physical_does_not_move_the_sites() {
        let held = trace(&map(0.0), RASTER, BASELINE, 180);
        let dithered = trace(&map(0.0001), RASTER, BASELINE, 180);
        assert_eq!(held.len(), dithered.len());
        let worst = held
            .iter()
            .zip(&dithered)
            .map(|(a, b)| (a.node.phi - b.node.phi).abs().to_degrees())
            .fold(0.0_f64, f64::max);
        assert!(
            worst < 0.01,
            "a site moved {worst} deg on a 0.0001 deg dither"
        );
    }

    /// Sites stay in body coordinates while the named view turns, so sampling
    /// has to undo that rotation before it asks the lens for a pixel. The
    /// fixture yaws 90 degrees: at least one real root is out of a lens if its
    /// body ray is handed straight to `project`.
    #[test]
    fn a_patch_projects_body_sites_through_the_named_view() {
        let map = map(3.0);
        let (site, lens) = sites(&map)
            .into_iter()
            .flat_map(|site| (0..2).map(move |lens| (site, lens)))
            .find(|(site, lens)| {
                !map.project(*lens, site.node.centre.map(|axis| axis as f32))
                    .inside
            })
            .expect("the turned view needs a body ray the raw projection rejects");
        let plane = blank(128);
        let source = Source {
            plane: &plane,
            lens,
        };
        assert!(
            patch(&map, source, site.node, 1, 0.001, [0.0; 2]).is_some(),
            "the body node was not round-tripped through view space"
        );
    }

    /// The null: one lens against its own picture is exactly zero, not nearly
    /// zero. Anything else means the sampler or the solve has a bias in it.
    #[test]
    fn one_lens_against_itself_reads_exactly_zero() {
        let map = map(0.0);
        let plane = textured();
        let source = Source {
            plane: &plane,
            lens: 0,
        };
        let mut read = 0;
        for site in sites(&map) {
            let Ok(reading) = measure(&map, source, source, site, SUPPORT, FLOOR) else {
                continue;
            };
            assert_eq!(
                reading.shift_rad,
                Axes {
                    perp: 0.0,
                    epi: 0.0
                }
            );
            assert!((reading.correlation - 1.0).abs() < 1e-9);
            read += 1;
        }
        assert!(read > 0, "the null control measured nothing at all");
    }

    #[test]
    fn a_flat_reference_patch_refuses_rather_than_matching_noise() {
        let map = map(0.0);
        let plane = blank(200);
        let source = Source {
            plane: &plane,
            lens: 0,
        };
        assert_eq!(
            measure(&map, source, source, first(&map), SUPPORT, FLOOR),
            Err(Refused::Flat)
        );
    }

    /// D8: one search offset with no contrast in it costs that offset, not the
    /// site. The target here is flat over most of the search and textured at
    /// one end of it, and the site still reads.
    #[test]
    fn a_flat_search_offset_costs_the_offset_and_not_the_site() {
        assert_eq!(correlation(&[1.0, 2.0, 3.0], &[5.0; 3]), None);
        assert_eq!(correlation(&[5.0; 3], &[1.0, 2.0, 3.0]), None);
        let scored = correlation(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]).expect("both patches vary");
        assert!((scored - 1.0).abs() < 1e-12);
        // A missing candidate is a gap in `legal`, and a gap does not stop the
        // remaining offsets from having a unique maximum.
        assert_eq!(peak(&[([-1, 0], 0.2), ([1, 1], 0.9)], 2), Ok(([1, 1], 0.9)));
    }

    /// D7: one tie rule. Two offsets a hair apart are the same offset, and the
    /// site refuses rather than being decided by scan order.
    #[test]
    fn scores_within_the_tie_band_refuse_instead_of_picking_by_scan_order() {
        assert_eq!(
            peak(&[([0, 0], 0.80), ([1, 0], 0.80 - TIE / 2.0)], 3),
            Err(Refused::Tied)
        );
        assert_eq!(
            peak(&[([0, 0], 0.80), ([1, 0], 0.80 - TIE * 10.0)], 3),
            Ok(([0, 0], 0.80))
        );
        assert_eq!(peak(&[], 3), Err(Refused::NoTarget));
    }

    #[test]
    fn a_peak_on_the_search_boundary_refuses_the_search_it_was_given() {
        assert_eq!(peak(&[([2, 0], 0.9)], 2), Err(Refused::Railed));
        assert_eq!(peak(&[([0, -2], 0.9)], 2), Err(Refused::Railed));
    }

    /// A response of zero is what an unperturbed pair of maps has to give:
    /// the finite difference is between the two perturbed maps alone.
    #[test]
    fn an_unperturbed_response_is_zero() {
        let base = map(0.0);
        assert_eq!(
            response(&base, &base, &base, 1, first(&base), 0.25),
            Ok(Axes::default())
        );
        assert_eq!(
            response(&base, &base, &base, 1, first(&base), 0.0),
            Err(NoScale::InvalidStep)
        );
    }

    #[test]
    fn a_response_reverses_with_the_maps_it_was_taken_between() {
        let (base, minus, plus) = (map(0.0), map(-0.5), map(0.5));
        let site = first(&base);
        let forward = response(&base, &minus, &plus, 1, site, 0.5).expect("the site survives");
        let reverse = response(&base, &plus, &minus, 1, site, 0.5).expect("the site survives");
        assert!(forward.epi.abs() > 1e-8 || forward.perp.abs() > 1e-8);
        assert!((forward.epi + reverse.epi).abs() < 1e-9);
        assert!((forward.perp + reverse.perp).abs() < 1e-9);
    }

    /// The predicted displacement is what a perturbed map actually does to the
    /// site, which is the whole basis of the plant control. Checked here
    /// against the projection itself, with no pixels involved.
    /// The predicted displacement is what a perturbed map actually does to the
    /// site, which is the whole basis of the plant control: a plant of
    /// `amount` is claimed to move a reading by `response * amount`. Checked
    /// here against the projection itself, with no pixels involved, and taken
    /// between exactly the two maps the plant control uses.
    #[test]
    fn a_response_predicts_where_the_perturbed_map_draws_the_site() {
        let amount = 0.2;
        let (base, planted) = (map(0.0), map(amount));
        let site = first(&base);
        let predicted = response(&base, &base, &planted, 1, site, amount / 2.0)
            .expect("the site survives the perturbation");
        // Follow the prediction on the perturbed map and the two landings
        // should be the same pixel of the same unchanged raw picture.
        let moved = unit(std::array::from_fn(|k| {
            site.node.centre[k]
                + site.node.perp[k] * predicted.perp * amount
                + site.node.epi[k] * predicted.epi * amount
        }));
        let held = base.project(
            1,
            base.view_ray_from_body(site.node.centre.map(|v| v as f32)),
        );
        let found = planted.project(1, planted.view_ray_from_body(moved.map(|v| v as f32)));
        assert!(held.inside && found.inside);
        // The plant is 0.2 degrees and moves this site about 4 px, so half a
        // pixel is what the linearization is allowed to cost.
        for axis in 0..2 {
            let off = (held.pixel[axis] - found.pixel[axis]).abs();
            assert!(off < 0.5, "axis {axis} is {off} px out of its prediction");
        }
    }

    /// The two scales are conversions and nothing else, so what they have to
    /// be is positive, finite, and what this lens model says.
    ///
    /// A unified-model fisheye at `xi` 2.315 and `fx` 3666 puts the seam
    /// circle at `fx / xi` = 1583 px of radius, so a radian along the seam is
    /// about 1583 px and a radian across it is `fx / xi^2` = 684, before the
    /// distortion polynomial. The two are not interchangeable and no single
    /// focal length can stand in for them.
    #[test]
    fn the_pixel_scales_are_finite_and_of_the_right_order() {
        let map = map(0.0);
        let site = first(&map);
        let source = source_scale(&map, 1, site).expect("the site lands in lens 1");
        let view = view_scale(&map, site, RASTER).expect("the site is in the view");
        for scale in [source, view] {
            assert!(scale.perp.is_finite() && scale.perp > 0.0);
            assert!(scale.epi.is_finite() && scale.epi > 0.0);
        }
        assert!(source.perp > 1.5 * source.epi, "{source:?}");
        assert!((1200.0..2400.0).contains(&source.perp), "{source:?}");
        assert!((600.0..1400.0).contains(&source.epi), "{source:?}");
        // 320 px of output over 70 degrees is 262 px per radian at the middle
        // and more toward the edges, on both axes alike.
        assert!((200.0..500.0).contains(&view.perp), "{view:?}");
        assert!((200.0..500.0).contains(&view.epi), "{view:?}");
    }
}
