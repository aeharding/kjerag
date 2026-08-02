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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    pub centre: [f64; 3],
    pub perp: [f64; 3],
    pub epi: [f64; 3],
    pub phi: f64,
}

/// A point selected from the seam contour of the rendered view.  The point is
/// only a location; it contains no composited colour or blend reading.
#[derive(Clone, Copy, Debug, PartialEq)]
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

/// Fixed camera-frame locations at which every traced 50/50 root is probed.
///
/// These are deliberately declared once, in angular `[perp, epi]` offsets
/// from the actual root.  They describe a small overlap strip, not pixels in
/// a named view and not locations chosen because their content looks useful.
/// Changing this list changes every view together.
pub const OVERLAP_STRIP_OFFSETS_DEG: [[f64; 2]; 9] = [
    [-0.40, -0.40],
    [-0.40, 0.00],
    [-0.40, 0.40],
    [0.00, -0.40],
    [0.00, 0.00],
    [0.00, 0.40],
    [0.40, -0.40],
    [0.40, 0.00],
    [0.40, 0.40],
];

/// A fixed physical location around one actual 50/50 root.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StripSite {
    pub root: Candidate,
    /// Camera-frame `[perp, epi]`, in radians, from `root`.
    pub offset_rad: [f64; 2],
}

/// Why a numerical calibration response cannot be stated for a fixed site.
///
/// This is separate from a raw-registration refusal: it reads no planes and
/// has no texture or peak selection.  It only asks whether the three warmed
/// projection maps locally describe the same physical site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseRefused {
    /// The declared central difference was not a finite, positive half-step.
    InvalidStep,
    /// The frozen site, or a local probe used to express it in camera axes,
    /// did not land in lens 1 in one of the warmed maps.
    ProjectedOut,
    /// Lens 1's projection is locally singular at the site, so image motion
    /// cannot be converted to a two-axis camera displacement.
    Singular,
}

/// The central finite-difference response of a frozen site to one seam knob.
///
/// `minus` and `plus` must be independently warmed maps made from the same
/// scene instant, camera, horizon state, and sampling as `base`; their only
/// difference is the selected [`kjerag_render::SeamFit`] knob at `-half_step`
/// and `+half_step`.  The function deliberately receives maps rather than a
/// `Scene`: it neither mutates calibration nor replays media, and therefore
/// cannot turn a diagnostic probe into a visible correction.
///
/// The result is radians of camera-frame `[epi, perp]` displacement per unit
/// of that knob.  It is the shift which, on the unchanged `base` map, follows
/// the moving lens-1 picture.  The sign is consequently the same convention
/// as raw registration: a positive result moves the *target direction*, not
/// the projected pixel.
pub fn central_site_response(
    base: &Reframe,
    minus: &Reframe,
    plus: &Reframe,
    site: StripSite,
    half_step: f64,
) -> Result<CameraDisplacement, ResponseRefused> {
    if !half_step.is_finite() || half_step <= 0.0 {
        return Err(ResponseRefused::InvalidStep);
    }
    let at = site_ray(site);
    let landing = |map: &Reframe, ray: [f64; 3]| {
        map.project(1, map.view_ray_from_body(ray.map(|axis| axis as f32)))
    };
    let (here, before, after) = (landing(base, at), landing(minus, at), landing(plus, at));
    if !here.inside || !before.inside || !after.inside {
        return Err(ResponseRefused::ProjectedOut);
    }

    // The local parameterization is taken only from the unchanged map.  The
    // two perturbed maps contribute the central image derivative, so a
    // calibration-induced change is not confused with a changing crossover
    // or a re-traced, content-selected site.
    let angular_probe = 0.01_f64.to_radians();
    let column = |axis: [f64; 3]| {
        let at_offset = |sign: f64| {
            let ray = unit(std::array::from_fn(|index| {
                at[index] + sign * angular_probe * axis[index]
            }));
            landing(base, ray)
        };
        let (low, high) = (at_offset(-1.0), at_offset(1.0));
        if !low.inside || !high.inside {
            return None;
        }
        Some([
            f64::from(high.pixel[0] - low.pixel[0]) / (2.0 * angular_probe),
            f64::from(high.pixel[1] - low.pixel[1]) / (2.0 * angular_probe),
        ])
    };
    let (perp, epi) = (
        column(site.root.node.perp).ok_or(ResponseRefused::ProjectedOut)?,
        column(site.root.node.epi).ok_or(ResponseRefused::ProjectedOut)?,
    );
    let determinant = perp[0] * epi[1] - perp[1] * epi[0];
    if !determinant.is_finite() || determinant.abs() < 1e-9 {
        return Err(ResponseRefused::Singular);
    }
    let pixels_per_unit = [
        f64::from(after.pixel[0] - before.pixel[0]) / (2.0 * half_step),
        f64::from(after.pixel[1] - before.pixel[1]) / (2.0 * half_step),
    ];
    if pixels_per_unit.iter().any(|value| !value.is_finite()) {
        return Err(ResponseRefused::ProjectedOut);
    }
    // `J [perp, epi] = image motion`; following the content is `-J^-1 d`.
    let perp_response = -(epi[1] * pixels_per_unit[0] - epi[0] * pixels_per_unit[1]) / determinant;
    let epi_response = -(perp[0] * pixels_per_unit[1] - perp[1] * pixels_per_unit[0]) / determinant;
    Ok(CameraDisplacement {
        epi: epi_response,
        perp: perp_response,
    })
}

/// A fixed site's physical body ray.  The site is deliberately not retraced
/// on a perturbed map: a numerical response is about one declared location.
fn site_ray(site: StripSite) -> [f64; 3] {
    unit(std::array::from_fn(|axis| {
        site.root.node.centre[axis]
            + site.root.node.perp[axis] * site.offset_rad[0]
            + site.root.node.epi[axis] * site.offset_rad[1]
    }))
}

/// Make the same declared sites from every root.  This is intentionally pure
/// so the declaration can be checked without decoded pixels or a renderer.
pub fn overlap_strip_sites(candidates: &[Candidate]) -> Vec<StripSite> {
    candidates
        .iter()
        .copied()
        .flat_map(|root| {
            OVERLAP_STRIP_OFFSETS_DEG
                .into_iter()
                .map(move |offset| StripSite {
                    root,
                    offset_rad: offset.map(f64::to_radians),
                })
        })
        .collect()
}

/// Whether one complete raw patch exists at a declared site or shift.  It is
/// coverage evidence only; neither luma nor a correlation score is retained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchCoverage {
    Complete,
    ProjectedOut,
    SourceBoundary,
}

/// Target availability at one globally declared registration shift.
#[derive(Clone, Copy, Debug)]
pub struct ShiftCoverage {
    /// Integer `[perp, epi]` shift in this support's global step.
    pub steps: [isize; 2],
    pub offset_rad: [f64; 2],
    pub coverage: PatchCoverage,
}

/// Coverage record for one fixed site.  `target` is empty only when its own
/// reference patch is incomplete: no target result is allowed to stand in for
/// a missing reference at that site.
#[derive(Clone, Debug)]
pub struct StripSiteCoverage {
    pub site: StripSite,
    pub reference: PatchCoverage,
    pub target: Vec<ShiftCoverage>,
}

/// Geometry-only accounting for a declared overlap strip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StripLatticeHealth {
    pub roots: usize,
    pub sites: usize,
    pub reference_complete: usize,
    pub reference_projected_out: usize,
    pub reference_source_boundary: usize,
    pub searched_offsets: usize,
    pub target_complete: usize,
    pub target_projected_out: usize,
    pub target_source_boundary: usize,
}

#[derive(Clone, Debug)]
pub struct StripLatticeResult {
    pub support: Support,
    pub sites: Vec<StripSiteCoverage>,
    pub health: StripLatticeHealth,
}

/// A two-axis translation expressed in the camera's physical seam axes.
///
/// The registration grid is stored as `[perp, epi]`, because its rows and
/// columns follow those offsets.  Consumers fitting a pose, however, should
/// never have to guess that convention: this type is explicitly `[epi,
/// perp]` and carries the corresponding camera-frame axes beside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraDisplacement {
    pub epi: f64,
    pub perp: f64,
}

/// Full covariance in the same `[epi, perp]` order as [`CameraDisplacement`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraCovariance {
    pub epi_epi: f64,
    pub epi_perp: f64,
    pub perp_perp: f64,
}

/// One immutable, body-fixed temporal-tracking declaration.
///
/// `site` is selected before tracking begins and is never re-traced, ranked,
/// or replaced.  Each successful transition returns a new value with the
/// same site and axes, an accumulated camera-frame offset, and the sum of
/// independent transition covariances.  This makes a later temporal fit able
/// to distinguish an unavailable declared site from a convenient neighbour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackState {
    pub site: StripSite,
    /// Accumulated `[epi, perp]` angular offset from the original declared
    /// body ray to the current picture, in radians.
    pub accumulated_rad: CameraDisplacement,
    /// Full covariance of `accumulated_rad`, in radians squared.
    pub covariance_rad2: CameraCovariance,
    /// A caller-declared radial limit on `accumulated_rad`, in radians.
    /// The tracker refuses, rather than silently reselecting a location, when
    /// the next transition would leave this neighbourhood.
    pub excursion_cap_rad: f64,
}

impl TrackState {
    /// Begin tracking one already-declared physical site.
    pub const fn new(site: StripSite, excursion_cap_rad: f64) -> Self {
        Self {
            site,
            accumulated_rad: CameraDisplacement {
                epi: 0.0,
                perp: 0.0,
            },
            covariance_rad2: CameraCovariance {
                epi_epi: 0.0,
                epi_perp: 0.0,
                perp_perp: 0.0,
            },
            excursion_cap_rad,
        }
    }
}

/// A successful one-lens previous-to-next tracking transition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackReading {
    /// The unchanged declaration and updated accumulated state.
    pub state: TrackState,
    /// This previous-to-next increment, in the same `[epi, perp]` axes as
    /// `state.accumulated_rad`.
    pub increment_rad: CameraDisplacement,
    /// Full covariance of `increment_rad`, in radians squared.
    pub covariance_rad2: CameraCovariance,
    pub condition: f64,
    pub samples: usize,
}

/// Why one fixed-site temporal transition cannot be claimed.
#[derive(Clone, Debug, PartialEq)]
pub enum TrackRefused {
    InvalidStep,
    InvalidExcursionCap,
    NoCompletePatch,
    NoPeak,
    Aperture,
    /// The attempted accumulated offset and the predeclared cap, both in
    /// radians.  This is a refusal, not permission to relocate the site.
    Excursion {
        attempted_rad: CameraDisplacement,
        cap_rad: f64,
    },
}

/// A forward/reverse closure at the same unchanged declared site.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackClosure {
    pub site: StripSite,
    /// Forward plus reverse increment.  An unbiased reciprocal pair closes
    /// at zero in the shared body-fixed axes.
    pub closure_rad: CameraDisplacement,
    pub covariance_rad2: CameraCovariance,
}

/// Why two temporal transitions cannot form a reciprocal control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackClosureRefused {
    MismatchedSite,
}

/// One two-dimensional raw registration at a pre-declared lattice site.
///
/// This is deliberately not a candidate winner.  A caller receives one of
/// these (or a concrete refusal) for *every* declared site, preserving the
/// evidence needed to decide whether a global pose explains the crossings.
#[derive(Clone, Copy, Debug)]
pub struct StripSiteReading {
    pub site: StripSite,
    /// Unit body/camera axes for the reported components.
    pub epi_axis: [f64; 3],
    pub perp_axis: [f64; 3],
    pub displacement_rad: CameraDisplacement,
    pub covariance_rad2: CameraCovariance,
    pub condition: f64,
    pub correlation: f64,
}

/// Outcome at exactly one declared site.  A refusal is retained rather than
/// silently letting another, more textured site stand in for it.
#[derive(Clone, Copy, Debug)]
pub struct StripSiteOutcome {
    pub site: StripSite,
    pub result: Result<StripSiteReading, Refused>,
}

/// A reciprocal raw-lens registration at one unchanged physical site.
///
/// `forward` samples lens 0 as reference and lens 1 as target; `reverse`
/// does the converse.  Both displacements use the *same* body-fixed
/// `[epi, perp]` axes from [`StripSite`], so an unbiased reciprocal pair has
/// a zero [`closure`], rather than requiring a lens-local sign convention.
#[derive(Clone, Copy, Debug)]
pub struct BidirectionalReading {
    pub site: StripSite,
    pub forward: StripSiteReading,
    pub reverse: StripSiteReading,
    pub closure: CameraDisplacement,
    /// Sum of the two independent registration covariances, in the same
    /// `[epi, perp]` order as `closure`.
    pub closure_covariance_rad2: CameraCovariance,
}

/// A reciprocal measurement failure.  The failed direction is retained so a
/// successful direction cannot be mistaken for a closure control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BidirectionalRefused {
    pub forward: Option<Refused>,
    pub reverse: Option<Refused>,
}

/// Outcome of one pre-declared reciprocal measurement.
#[derive(Clone, Copy, Debug)]
pub struct BidirectionalOutcome {
    pub site: StripSite,
    pub result: Result<BidirectionalReading, BidirectionalRefused>,
}

/// The response of one named calibration knob at exactly one declared site.
///
/// Kept beside the site rather than as an anonymous position in a vector so
/// the pose assembly can refuse a reordered or re-traced response.  A site
/// that has no response remains present as a concrete refusal; it is never
/// replaced by a more convenient site.
#[derive(Clone, Copy, Debug)]
pub struct SiteResponse {
    pub site: StripSite,
    pub result: Result<CameraDisplacement, ResponseRefused>,
}

/// Build one site response record without changing the frozen site identity.
pub fn site_response(
    base: &Reframe,
    minus: &Reframe,
    plus: &Reframe,
    site: StripSite,
    half_step: f64,
) -> SiteResponse {
    SiteResponse {
        site,
        result: central_site_response(base, minus, plus, site, half_step),
    }
}

/// Why fixed-site evidence could not be assembled into a shared-pose input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssemblyRefused {
    /// A response column was made from a different site declaration or was
    /// reordered.  Pairing by a score or proximity would make a different
    /// physical observation, so the instrument refuses instead.
    MismatchedSite { knob: usize, site: usize },
    /// A response column omitted a declared site.
    MismatchedLength {
        knob: usize,
        expected: usize,
        got: usize,
    },
    /// Fewer than four sites supplied both a two-axis reading and all five
    /// finite central responses.  This leaves fewer than three residual
    /// degrees of freedom after the five-knob fit.
    TooFewCompleteSites { have: usize },
}

/// Fixed-site raw evidence ready for the one-view shared-pose test.
#[derive(Clone, Debug)]
pub struct PoseAssembly {
    /// Only readings whose *same declared site* also has every response.
    /// Their order follows `outcomes`; no texture or residual winner is
    /// selected.
    pub observations: Vec<local_warp::Observation>,
    /// Every site which supplied a raw two-axis reading before response gates.
    pub raw_readings: usize,
    /// Readings excluded because at least one response was refused or not
    /// finite.  This is evidence health, not a reason to substitute a site.
    pub incomplete_responses: usize,
}

/// Assemble raw registrations and central responses for one warmed view.
///
/// Both source units are radians: output observations are physical degrees,
/// so each covariance entry, including the correlation term, is multiplied
/// by `(180 / pi)^2`.  The five response columns must be the exact fixed-site
/// order used for `outcomes`; this routine performs no cross-view pairing and
/// no best-site selection.
pub fn assemble_pose_observations(
    outcomes: &[StripSiteOutcome],
    responses: &[Vec<SiteResponse>; local_warp::KNOBS],
) -> Result<PoseAssembly, AssemblyRefused> {
    for (knob, column) in responses.iter().enumerate() {
        if column.len() != outcomes.len() {
            return Err(AssemblyRefused::MismatchedLength {
                knob,
                expected: outcomes.len(),
                got: column.len(),
            });
        }
        for (site, (outcome, response)) in outcomes.iter().zip(column).enumerate() {
            if outcome.site != response.site {
                return Err(AssemblyRefused::MismatchedSite { knob, site });
            }
        }
    }

    let radians_to_degrees = 180.0 / std::f64::consts::PI;
    let covariance_scale = radians_to_degrees.powi(2);
    let mut raw_readings = 0;
    let mut incomplete_responses = 0;
    let mut observations = Vec::new();
    for (site_index, outcome) in outcomes.iter().enumerate() {
        let Ok(reading) = outcome.result else {
            continue;
        };
        raw_readings += 1;
        let mut epi = [0.0; local_warp::KNOBS];
        let mut perp = [0.0; local_warp::KNOBS];
        let complete = responses.iter().enumerate().all(|(knob, column)| {
            let Ok(response) = column[site_index].result else {
                return false;
            };
            if !response.epi.is_finite() || !response.perp.is_finite() {
                return false;
            }
            epi[knob] = response.epi * radians_to_degrees;
            perp[knob] = response.perp * radians_to_degrees;
            true
        });
        if !complete {
            incomplete_responses += 1;
            continue;
        }
        observations.push(local_warp::Observation {
            name: format!(
                "root-phi-{:.3}-perp-{:.3}-epi-{:.3}",
                reading.site.root.node.phi.to_degrees(),
                reading.site.offset_rad[0].to_degrees(),
                reading.site.offset_rad[1].to_degrees(),
            ),
            displacement: local_warp::Displacement {
                epi: reading.displacement_rad.epi * radians_to_degrees,
                perp: reading.displacement_rad.perp * radians_to_degrees,
            },
            covariance: local_warp::Covariance {
                xx: reading.covariance_rad2.epi_epi * covariance_scale,
                xy: reading.covariance_rad2.epi_perp * covariance_scale,
                yy: reading.covariance_rad2.perp_perp * covariance_scale,
            },
            jacobian: local_warp::Jacobian { epi, perp },
        });
    }
    if observations.len() < 4 {
        return Err(AssemblyRefused::TooFewCompleteSites {
            have: observations.len(),
        });
    }
    Ok(PoseAssembly {
        observations,
        raw_readings,
        incomplete_responses,
    })
}

/// Register every fixed lattice site independently, with no score-based
/// selection between sites.
///
/// Reference support and every target shift are checked at the site itself.
/// A unique peak must lie inside the declared search, then the local two-axis
/// solve either refines it or returns the aperture refusal.  This retains the
/// exact `StripSite` in every outcome so later cross-capture pairing cannot
/// substitute a convenient neighbour.
pub fn register_overlap_strip(
    map: &Reframe,
    planes: &[Plane],
    candidates: &[Candidate],
    support: Support,
) -> Vec<StripSiteOutcome> {
    register_strip_sites(map, planes, &overlap_strip_sites(candidates), support)
}

/// Register a caller's declared fixed sites exactly as supplied.
///
/// This lower-level entry point lets a cross-capture instrument pair a stable
/// declaration without regenerating or ranking locations from its pixels.
pub fn register_strip_sites(
    map: &Reframe,
    planes: &[Plane],
    sites: &[StripSite],
    support: Support,
) -> Vec<StripSiteOutcome> {
    sites
        .iter()
        .copied()
        .map(|site| StripSiteOutcome {
            result: register_site(map, planes, site, support),
            site,
        })
        .collect()
}

/// Register every supplied site in both raw-lens directions.
///
/// This is a control, not an alternative estimator: it does not rank sites,
/// combine captures, fit a pose, or alter a map.  Each direction is sampled
/// against the identical declared body axes, allowing the reported sum to
/// reveal an orientation or registration inconsistency directly.
pub fn register_strip_sites_bidirectional(
    map: &Reframe,
    planes: &[Plane],
    sites: &[StripSite],
    support: Support,
) -> Vec<BidirectionalOutcome> {
    sites
        .iter()
        .copied()
        .map(|site| {
            let result = bidirectional_result(
                site,
                register_site_direction(map, planes, site, support, 0, 1),
                register_site_direction(map, planes, site, support, 1, 0),
            );
            BidirectionalOutcome { site, result }
        })
        .collect()
}

fn bidirectional_result(
    site: StripSite,
    forward: Result<StripSiteReading, Refused>,
    reverse: Result<StripSiteReading, Refused>,
) -> Result<BidirectionalReading, BidirectionalRefused> {
    match (forward, reverse) {
        (Ok(forward), Ok(reverse)) => Ok(bidirectional_reading(site, forward, reverse)),
        (forward, reverse) => Err(BidirectionalRefused {
            forward: forward.err(),
            reverse: reverse.err(),
        }),
    }
}

fn register_site(
    map: &Reframe,
    planes: &[Plane],
    site: StripSite,
    support: Support,
) -> Result<StripSiteReading, Refused> {
    register_site_direction(map, planes, site, support, 0, 1)
}

fn register_site_direction(
    map: &Reframe,
    planes: &[Plane],
    site: StripSite,
    support: Support,
    reference_lens: usize,
    target_lens: usize,
) -> Result<StripSiteReading, Refused> {
    let (Some(front), Some(back)) = (planes.first(), planes.get(1)) else {
        return Err(Refused::NoCompletePatch);
    };
    if !support.valid() || support.half() < 1 || support.search_steps() < 1 {
        return Err(Refused::NoCompletePatch);
    }
    let step = support.step_deg.to_radians();
    let half = support.half();
    let reference_plane = if reference_lens == 0 { front } else { back };
    let target_plane = if target_lens == 0 { front } else { back };
    let reference = sample(
        map,
        reference_plane,
        reference_lens,
        site.root.node,
        half,
        step,
        site.offset_rad,
    )
    .map_err(|_| Refused::NoCompletePatch)?;
    let coarse = support.search_steps();
    let mut legal = Vec::new();
    for perp in -coarse..=coarse {
        for epi in -coarse..=coarse {
            let offset = [
                site.offset_rad[0] + perp as f64 * step,
                site.offset_rad[1] + epi as f64 * step,
            ];
            if let Ok(target) = sample(
                map,
                target_plane,
                target_lens,
                site.root.node,
                half,
                step,
                offset,
            ) {
                legal.push(([perp, epi], correlation(&reference, &target)));
            }
        }
    }
    let (grid_shift, correlation) = peak(&legal, coarse)?;
    let target_offset = [
        site.offset_rad[0] + grid_shift[0] as f64 * step,
        site.offset_rad[1] + grid_shift[1] as f64 * step,
    ];
    let target = sample(
        map,
        target_plane,
        target_lens,
        site.root.node,
        half,
        step,
        target_offset,
    )
    .map_err(|_| Refused::NoCompletePatch)?;
    let fitted =
        local_warp::register(&samples(&reference, &target, step)).map_err(|why| match why {
            local_warp::RegistrationRefused::Aperture => Refused::Aperture,
            _ => Refused::NoPeak,
        })?;
    Ok(camera_reading(site, grid_shift, step, correlation, fitted))
}

/// Register one raw lens from a previous frame to a next frame at one frozen
/// site, then return the next immutable tracking state.
///
/// Unlike seam registration this does not cross lenses: `lens` is used for
/// both patches.  The reference patch is centred at the state's accumulated
/// body-fixed offset; the target search is centred at that same offset.  No
/// candidate contour is traced, and no site can be replaced when this one is
/// unavailable or exceeds its declared excursion.
pub fn track_one_lens(
    previous_map: &Reframe,
    previous_plane: &Plane,
    next_map: &Reframe,
    next_plane: &Plane,
    lens: usize,
    state: TrackState,
    support: Support,
) -> Result<TrackReading, TrackRefused> {
    if !support.valid() || support.half() < 1 || support.search_steps() < 1 {
        return Err(TrackRefused::NoCompletePatch);
    }
    let step = support.step_deg.to_radians();
    let half = support.half();
    let centre_offset = [
        state.site.offset_rad[0] + state.accumulated_rad.perp,
        state.site.offset_rad[1] + state.accumulated_rad.epi,
    ];
    let reference = sample(
        previous_map,
        previous_plane,
        lens,
        state.site.root.node,
        half,
        step,
        centre_offset,
    )
    .map_err(|_| TrackRefused::NoCompletePatch)?;
    let coarse = support.search_steps();
    let mut legal = Vec::new();
    for perp in -coarse..=coarse {
        for epi in -coarse..=coarse {
            let offset = [
                centre_offset[0] + perp as f64 * step,
                centre_offset[1] + epi as f64 * step,
            ];
            if let Ok(target) = sample(
                next_map,
                next_plane,
                lens,
                state.site.root.node,
                half,
                step,
                offset,
            ) {
                legal.push(([perp, epi], correlation(&reference, &target)));
            }
        }
    }
    let (grid, _) = peak(&legal, coarse).map_err(|why| match why {
        Refused::NoCompletePatch => TrackRefused::NoCompletePatch,
        Refused::NoPeak | Refused::Aperture | Refused::NoVisibleSeam => TrackRefused::NoPeak,
    })?;
    let target_offset = [
        centre_offset[0] + grid[0] as f64 * step,
        centre_offset[1] + grid[1] as f64 * step,
    ];
    let target = sample(
        next_map,
        next_plane,
        lens,
        state.site.root.node,
        half,
        step,
        target_offset,
    )
    .map_err(|_| TrackRefused::NoCompletePatch)?;
    // The local solver sees the patch at the coarse winner.  Add that known
    // grid translation into its linear residual so `advance_track` receives
    // the full previous-to-next displacement in one explicit basis.
    let coarse_rad = [grid[0] as f64 * step, grid[1] as f64 * step];
    let samples = samples(&reference, &target, step)
        .into_iter()
        .map(|mut sample| {
            sample.residual += sample.gradient[0] * coarse_rad[0] / step
                + sample.gradient[1] * coarse_rad[1] / step;
            sample
        })
        .collect::<Vec<_>>();
    advance_track(state, &samples, step)
}

fn bidirectional_reading(
    site: StripSite,
    forward: StripSiteReading,
    reverse: StripSiteReading,
) -> BidirectionalReading {
    // Both searches offset `site_ray` in the unmodified, body-fixed axes.
    // Therefore the inverse relationship is addition, with no hidden lens
    // basis rotation or sign flip.
    BidirectionalReading {
        site,
        forward,
        reverse,
        closure: CameraDisplacement {
            epi: forward.displacement_rad.epi + reverse.displacement_rad.epi,
            perp: forward.displacement_rad.perp + reverse.displacement_rad.perp,
        },
        closure_covariance_rad2: CameraCovariance {
            epi_epi: forward.covariance_rad2.epi_epi + reverse.covariance_rad2.epi_epi,
            epi_perp: forward.covariance_rad2.epi_perp + reverse.covariance_rad2.epi_perp,
            perp_perp: forward.covariance_rad2.perp_perp + reverse.covariance_rad2.perp_perp,
        },
    }
}

/// Convert a grid solve to its explicit camera-axis representation.
///
/// `local_warp::Registration` is ordered `[perp, epi]`; the permutation here
/// applies to both components and all covariance terms, not just its diagonal.
fn camera_reading(
    site: StripSite,
    coarse: [isize; 2],
    step: f64,
    correlation: f64,
    fitted: local_warp::Registration,
) -> StripSiteReading {
    let grid_perp = coarse[0] as f64 * step + fitted.displacement.x * step;
    let grid_epi = coarse[1] as f64 * step + fitted.displacement.y * step;
    StripSiteReading {
        site,
        epi_axis: site.root.node.epi,
        perp_axis: site.root.node.perp,
        displacement_rad: CameraDisplacement {
            epi: grid_epi,
            perp: grid_perp,
        },
        covariance_rad2: CameraCovariance {
            epi_epi: fitted.covariance.yy * step * step,
            epi_perp: fitted.covariance.xy * step * step,
            perp_perp: fitted.covariance.xx * step * step,
        },
        condition: fitted.condition,
        correlation,
    }
}

/// Advance one declared site from a previous single-lens patch to its next
/// single-lens patch.
///
/// `samples` use [`RegistrationSample`]'s ordinary convention: a target
/// minus reference residual equals its target-picture gradient dotted with a
/// `[perp, epi]` grid displacement.  `step_rad` converts that grid result to
/// the body-fixed angular axes stored by [`TrackState`].  The function is
/// pure: callers which obtain the samples from raw planes cannot use it to
/// alter a map, retrace a seam, or choose another site.
pub fn advance_track(
    state: TrackState,
    samples: &[RegistrationSample],
    step_rad: f64,
) -> Result<TrackReading, TrackRefused> {
    if !step_rad.is_finite() || step_rad <= 0.0 {
        return Err(TrackRefused::InvalidStep);
    }
    if !state.excursion_cap_rad.is_finite() || state.excursion_cap_rad < 0.0 {
        return Err(TrackRefused::InvalidExcursionCap);
    }
    let fitted = local_warp::register(samples).map_err(|why| match why {
        local_warp::RegistrationRefused::Aperture => TrackRefused::Aperture,
        local_warp::RegistrationRefused::TooFewSamples { .. }
        | local_warp::RegistrationRefused::InvalidSample { .. } => TrackRefused::NoPeak,
    })?;
    let increment_rad = CameraDisplacement {
        epi: fitted.displacement.y * step_rad,
        perp: fitted.displacement.x * step_rad,
    };
    let attempted_rad = CameraDisplacement {
        epi: state.accumulated_rad.epi + increment_rad.epi,
        perp: state.accumulated_rad.perp + increment_rad.perp,
    };
    if !attempted_rad.epi.is_finite()
        || !attempted_rad.perp.is_finite()
        || displacement_norm(attempted_rad) > state.excursion_cap_rad
    {
        return Err(TrackRefused::Excursion {
            attempted_rad,
            cap_rad: state.excursion_cap_rad,
        });
    }
    let covariance_rad2 = CameraCovariance {
        epi_epi: fitted.covariance.yy * step_rad * step_rad,
        epi_perp: fitted.covariance.xy * step_rad * step_rad,
        perp_perp: fitted.covariance.xx * step_rad * step_rad,
    };
    let state = TrackState {
        accumulated_rad: attempted_rad,
        covariance_rad2: CameraCovariance {
            epi_epi: state.covariance_rad2.epi_epi + covariance_rad2.epi_epi,
            epi_perp: state.covariance_rad2.epi_perp + covariance_rad2.epi_perp,
            perp_perp: state.covariance_rad2.perp_perp + covariance_rad2.perp_perp,
        },
        ..state
    };
    Ok(TrackReading {
        state,
        increment_rad,
        covariance_rad2,
        condition: fitted.condition,
        samples: fitted.samples,
    })
}

/// Compare opposite temporal transitions without converting into lens-local
/// axes.  The exact same [`StripSite`] is required; proximity and texture are
/// intentionally not substitutes for identity.
pub fn track_closure(
    forward: TrackReading,
    reverse: TrackReading,
) -> Result<TrackClosure, TrackClosureRefused> {
    if forward.state.site != reverse.state.site {
        return Err(TrackClosureRefused::MismatchedSite);
    }
    Ok(TrackClosure {
        site: forward.state.site,
        closure_rad: CameraDisplacement {
            epi: forward.increment_rad.epi + reverse.increment_rad.epi,
            perp: forward.increment_rad.perp + reverse.increment_rad.perp,
        },
        covariance_rad2: CameraCovariance {
            epi_epi: forward.covariance_rad2.epi_epi + reverse.covariance_rad2.epi_epi,
            epi_perp: forward.covariance_rad2.epi_perp + reverse.covariance_rad2.epi_perp,
            perp_perp: forward.covariance_rad2.perp_perp + reverse.covariance_rad2.perp_perp,
        },
    })
}

fn displacement_norm(displacement: CameraDisplacement) -> f64 {
    displacement.epi.hypot(displacement.perp)
}

/// Census fixed overlap-strip sites and every target shift independently.
///
/// This is the Stage 9 location/coverage instrument.  A complete reference
/// patch is required at every site.  Once it exists, every target shift is
/// retained with its own coverage outcome: there is no enclosing search
/// rectangle and no texture, correlation, or per-view winner selection.
pub fn overlap_strip_lattice(
    map: &Reframe,
    planes: &[Plane],
    candidates: &[Candidate],
    support: Support,
) -> StripLatticeResult {
    let mut health = StripLatticeHealth {
        roots: candidates.len(),
        ..StripLatticeHealth::default()
    };
    let sites = overlap_strip_sites(candidates);
    health.sites = sites.len();
    let (Some(front), Some(back)) = (planes.first(), planes.get(1)) else {
        return StripLatticeResult {
            support,
            sites: sites
                .into_iter()
                .map(|site| StripSiteCoverage {
                    site,
                    reference: PatchCoverage::ProjectedOut,
                    target: Vec::new(),
                })
                .collect(),
            health,
        };
    };
    if !support.valid() || support.half() < 1 || support.search_steps() < 1 {
        return StripLatticeResult {
            support,
            sites: sites
                .into_iter()
                .map(|site| StripSiteCoverage {
                    site,
                    reference: PatchCoverage::ProjectedOut,
                    target: Vec::new(),
                })
                .collect(),
            health,
        };
    }
    let step = support.step_deg.to_radians();
    let half = support.half();
    let coarse = support.search_steps();
    let sites = sites
        .into_iter()
        .map(|site| {
            let reference = coverage(map, front, 0, site.root.node, half, step, site.offset_rad);
            match reference {
                PatchCoverage::Complete => health.reference_complete += 1,
                PatchCoverage::ProjectedOut => health.reference_projected_out += 1,
                PatchCoverage::SourceBoundary => health.reference_source_boundary += 1,
            }
            let mut target = Vec::new();
            if reference == PatchCoverage::Complete {
                for perp in -coarse..=coarse {
                    for epi in -coarse..=coarse {
                        health.searched_offsets += 1;
                        let offset_rad = [
                            site.offset_rad[0] + perp as f64 * step,
                            site.offset_rad[1] + epi as f64 * step,
                        ];
                        let coverage =
                            coverage(map, back, 1, site.root.node, half, step, offset_rad);
                        match coverage {
                            PatchCoverage::Complete => health.target_complete += 1,
                            PatchCoverage::ProjectedOut => health.target_projected_out += 1,
                            PatchCoverage::SourceBoundary => health.target_source_boundary += 1,
                        }
                        target.push(ShiftCoverage {
                            steps: [perp, epi],
                            offset_rad,
                            coverage,
                        });
                    }
                }
            }
            StripSiteCoverage {
                site,
                reference,
                target,
            }
        })
        .collect();
    StripLatticeResult {
        support,
        sites,
        health,
    }
}

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

fn coverage(
    map: &Reframe,
    plane: &Plane,
    lens: usize,
    node: Node,
    half: isize,
    step: f64,
    offset: [f64; 2],
) -> PatchCoverage {
    match sample(map, plane, lens, node, half, step, offset) {
        Ok(_) => PatchCoverage::Complete,
        Err(PatchRefusal::ProjectedOut) => PatchCoverage::ProjectedOut,
        Err(PatchRefusal::SourceBoundary) => PatchCoverage::SourceBoundary,
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
) -> Result<Vec<f64>, PatchRefusal> {
    let mut out = Vec::with_capacity(((2 * half + 1).pow(2)) as usize);
    for i in -half..=half {
        for j in -half..=half {
            let ray = unit(std::array::from_fn(|k| {
                node.centre[k]
                    + node.perp[k] * (i as f64 * step + offset[0])
                    + node.epi[k] * (j as f64 * step + offset[1])
            }));
            // `Node` is deliberately camera-body based so its epipolar axes
            // remain fixed while the named view turns. `project`, however,
            // takes the renderer's view-space ray. Passing `ray` directly
            // mixed those frames and could report an actual crossover root
            // as projected out merely because the view was rotated.
            let view = map.view_ray_from_body(ray.map(|v| v as f32));
            let landing = map.project(lens, view);
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

    fn central_site(map: &Reframe) -> StripSite {
        StripSite {
            root: visible_candidates(map, 320, 320, [0.0, 0.0, -0.033])
                .into_iter()
                .next()
                .expect("the crossover fixture exposes a root"),
            offset_rad: [0.0, 0.0],
        }
    }

    #[test]
    fn central_response_is_zero_when_both_perturbations_are_the_base_map() {
        let base = crossover_map(0.0);
        let response = central_site_response(&base, &base, &base, central_site(&base), 0.25)
            .expect("the fixture has a regular central lens-1 projection");
        assert_eq!(
            response,
            CameraDisplacement {
                epi: 0.0,
                perp: 0.0
            }
        );
    }

    #[test]
    fn central_response_is_finite_and_reverses_with_the_known_map_perturbation() {
        let base = crossover_map(0.0);
        let minus = crossover_map(-0.5);
        let plus = crossover_map(0.5);
        let site = central_site(&base);
        let forward = central_site_response(&base, &minus, &plus, site, 0.5)
            .expect("the pitched maps retain this fixed physical site");
        let reverse = central_site_response(&base, &plus, &minus, site, 0.5)
            .expect("reversing the maps retains the same fixed physical site");
        assert!(forward.epi.is_finite() && forward.perp.is_finite());
        assert!(forward.epi.abs() > 1e-8 || forward.perp.abs() > 1e-8);
        assert!((forward.epi + reverse.epi).abs() < 1e-9);
        assert!((forward.perp + reverse.perp).abs() < 1e-9);
    }

    #[test]
    fn central_response_refuses_invalid_step_singular_axes_and_projected_out_sites() {
        let base = crossover_map(0.0);
        let site = central_site(&base);
        assert_eq!(
            central_site_response(&base, &base, &base, site, 0.0),
            Err(ResponseRefused::InvalidStep)
        );

        let singular = StripSite {
            root: Candidate {
                node: Node {
                    perp: [0.0; 3],
                    epi: [0.0; 3],
                    ..site.root.node
                },
                ..site.root
            },
            ..site
        };
        assert_eq!(
            central_site_response(&base, &base, &base, singular, 0.25),
            Err(ResponseRefused::Singular)
        );

        let body = [0.0, 0.0, 1.0];
        let out = StripSite {
            root: Candidate {
                node: node([0.0, 0.0, -0.033], body),
                view_ray: base.view_ray_from_body(body.map(|axis| axis as f32)),
                view_pixel: [0.0, 0.0],
            },
            offset_rad: [0.0, 0.0],
        };
        assert!(
            !base
                .project(1, base.view_ray_from_body(body.map(|axis| axis as f32)))
                .inside,
            "the fixture's north-pole body ray must be outside lens 1"
        );
        assert_eq!(
            central_site_response(&base, &base, &base, out, 0.25),
            Err(ResponseRefused::ProjectedOut)
        );
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
            let recovered = map.view_ray_from_body(candidate.node.centre.map(|v| v as f32));
            for axis in 0..3 {
                assert!(
                    (recovered[axis] - candidate.view_ray[axis]).abs() < 1e-5,
                    "body-to-view inverse changed axis {axis}: {recovered:?} vs {:?}",
                    candidate.view_ray,
                );
            }
            for lens in 0..2 {
                let from_node = map.project(lens, recovered);
                assert!(from_node.inside);
                for axis in 0..2 {
                    assert!(
                        (from_node.pixel[axis] - blend.landings[lens].pixel[axis]).abs() < 1e-3,
                        "lens {lens} landing differs after body-to-view round-trip"
                    );
                }
            }
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
    fn overlap_strip_sites_are_fixed_for_every_actual_root() {
        let first = Candidate {
            node: node([0.0, 0.0, -0.033], [1.0, 0.0, 0.0]),
            view_ray: [1.0, 0.0, 0.0],
            view_pixel: [10.0, 20.0],
        };
        let second = Candidate {
            node: node([0.0, 0.0, -0.033], [0.0, 1.0, 0.0]),
            view_ray: [0.0, 1.0, 0.0],
            view_pixel: [30.0, 40.0],
        };
        let sites = overlap_strip_sites(&[first, second]);
        assert_eq!(sites.len(), 2 * OVERLAP_STRIP_OFFSETS_DEG.len());
        for (root, group) in sites
            .chunks_exact(OVERLAP_STRIP_OFFSETS_DEG.len())
            .enumerate()
        {
            assert_eq!(
                group[0].root.view_pixel,
                if root == 0 {
                    [10.0, 20.0]
                } else {
                    [30.0, 40.0]
                }
            );
            for (site, declared) in group.iter().zip(OVERLAP_STRIP_OFFSETS_DEG) {
                assert_eq!(site.offset_rad, declared.map(f64::to_radians));
            }
        }
    }

    #[test]
    fn all_sites_are_reported_without_a_winner_selection() {
        let root = Candidate {
            node: node([0.0, 0.0, -0.033], [1.0, 0.0, 0.0]),
            view_ray: [1.0, 0.0, 0.0],
            view_pixel: [10.0, 20.0],
        };
        // Missing planes refuse each declared site separately.  In
        // particular, this result cannot collapse to one nominal "best"
        // candidate as `select_with_support` historically did.
        let outcomes = register_overlap_strip(&crossover_map(0.0), &[], &[root], SUPPORT_LADDER[0]);
        assert_eq!(outcomes.len(), OVERLAP_STRIP_OFFSETS_DEG.len());
        assert!(
            outcomes
                .iter()
                .all(|outcome| matches!(outcome.result, Err(Refused::NoCompletePatch)))
        );
        for (outcome, declared) in outcomes.iter().zip(OVERLAP_STRIP_OFFSETS_DEG) {
            assert_eq!(outcome.site.offset_rad, declared.map(f64::to_radians));
        }
    }

    #[test]
    fn camera_reading_permutes_the_full_grid_covariance() {
        let site = StripSite {
            root: Candidate {
                node: node([0.0, 0.0, -0.033], [1.0, 0.0, 0.0]),
                view_ray: [1.0, 0.0, 0.0],
                view_pixel: [10.0, 20.0],
            },
            offset_rad: [0.0, 0.0],
        };
        let step = 0.25;
        let reading = camera_reading(
            site,
            [2, -3],
            step,
            0.9,
            local_warp::Registration {
                displacement: local_warp::PixelDisplacement { x: 0.5, y: -0.25 },
                covariance: local_warp::Covariance {
                    xx: 4.0,
                    xy: 1.5,
                    yy: 9.0,
                },
                condition: 7.0,
                samples: 12,
            },
        );
        assert_eq!(reading.displacement_rad.perp, 0.625);
        assert_eq!(reading.displacement_rad.epi, -0.8125);
        assert_eq!(reading.covariance_rad2.epi_epi, 9.0 * step * step);
        assert_eq!(reading.covariance_rad2.epi_perp, 1.5 * step * step);
        assert_eq!(reading.covariance_rad2.perp_perp, 4.0 * step * step);
        assert_eq!(reading.epi_axis, site.root.node.epi);
        assert_eq!(reading.perp_axis, site.root.node.perp);
    }

    #[test]
    fn reciprocal_body_axis_readings_close_and_sum_full_covariance() {
        let site = assembled_site(3);
        // These are planted physical camera-axis readings, not a second
        // content search.  The reverse direction is the exact inverse in the
        // same declared body axes, which verifies that closure is a sum (and
        // not a lens-local sign conversion).
        let forward = StripSiteReading {
            site,
            epi_axis: site.root.node.epi,
            perp_axis: site.root.node.perp,
            displacement_rad: CameraDisplacement {
                epi: 0.018,
                perp: -0.027,
            },
            covariance_rad2: CameraCovariance {
                epi_epi: 2.0,
                epi_perp: -0.3,
                perp_perp: 4.0,
            },
            condition: 2.0,
            correlation: 0.9,
        };
        let reverse = StripSiteReading {
            displacement_rad: CameraDisplacement {
                epi: -forward.displacement_rad.epi,
                perp: -forward.displacement_rad.perp,
            },
            covariance_rad2: CameraCovariance {
                epi_epi: 3.0,
                epi_perp: 0.7,
                perp_perp: 5.0,
            },
            ..forward
        };
        let paired = bidirectional_result(site, Ok(forward), Ok(reverse))
            .expect("opposite planted directions form a reciprocal pair");
        assert_eq!(
            paired.closure,
            CameraDisplacement {
                epi: 0.0,
                perp: 0.0,
            }
        );
        assert_eq!(paired.closure_covariance_rad2.epi_epi, 5.0);
        assert!((paired.closure_covariance_rad2.epi_perp - 0.4).abs() < 1e-12);
        assert_eq!(paired.closure_covariance_rad2.perp_perp, 9.0);
    }

    #[test]
    fn reciprocal_control_keeps_the_failed_direction() {
        let site = assembled_site(0);
        assert!(matches!(
            bidirectional_result(site, Err(Refused::NoPeak), Err(Refused::Aperture),),
            Err(BidirectionalRefused {
                forward: Some(Refused::NoPeak),
                reverse: Some(Refused::Aperture),
            })
        ));
        let reading = StripSiteReading {
            site,
            epi_axis: site.root.node.epi,
            perp_axis: site.root.node.perp,
            displacement_rad: CameraDisplacement {
                epi: 0.0,
                perp: 0.0,
            },
            covariance_rad2: CameraCovariance {
                epi_epi: 0.0,
                epi_perp: 0.0,
                perp_perp: 0.0,
            },
            condition: 1.0,
            correlation: 1.0,
        };
        assert!(matches!(
            bidirectional_result(site, Ok(reading), Err(Refused::Aperture)),
            Err(BidirectionalRefused {
                forward: None,
                reverse: Some(Refused::Aperture),
            })
        ));
    }
    #[test]
    fn correlation_refuses_flat_content_by_reporting_no_agreement() {
        assert_eq!(correlation(&[1.0; 4], &[1.0; 4]), 0.0);
    }

    #[test]
    fn legal_interior_peak_survives_unavailable_neighbouring_shifts() {
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

    fn assembled_site(index: usize) -> StripSite {
        StripSite {
            root: Candidate {
                node: Node {
                    centre: [1.0, 0.0, 0.0],
                    perp: [0.0, 1.0, 0.0],
                    epi: [0.0, 0.0, 1.0],
                    phi: index as f64 * 0.1,
                },
                view_ray: [1.0, 0.0, 0.0],
                view_pixel: [index as f32, 0.0],
            },
            offset_rad: [index as f64 * 0.01, -(index as f64) * 0.02],
        }
    }

    fn assembled_outcome(index: usize) -> StripSiteOutcome {
        let site = assembled_site(index);
        StripSiteOutcome {
            site,
            result: Ok(StripSiteReading {
                site,
                epi_axis: site.root.node.epi,
                perp_axis: site.root.node.perp,
                displacement_rad: CameraDisplacement {
                    epi: 0.1 + index as f64 * 0.01,
                    perp: -0.2,
                },
                covariance_rad2: CameraCovariance {
                    epi_epi: 4.0e-6,
                    epi_perp: -1.5e-6,
                    perp_perp: 9.0e-6,
                },
                condition: 2.0,
                correlation: 0.9,
            }),
        }
    }

    #[test]
    fn pose_assembly_preserves_full_covariance_when_converting_radians_to_degrees() {
        let outcomes: Vec<_> = (0..4).map(assembled_outcome).collect();
        let responses = std::array::from_fn(|knob| {
            outcomes
                .iter()
                .map(|outcome| SiteResponse {
                    site: outcome.site,
                    result: Ok(CameraDisplacement {
                        epi: (knob + 1) as f64 * 0.01,
                        perp: -((knob + 1) as f64) * 0.02,
                    }),
                })
                .collect()
        });
        let assembled = assemble_pose_observations(&outcomes, &responses)
            .expect("four complete fixed sites are enough to test a pose");
        assert_eq!(assembled.raw_readings, 4);
        assert_eq!(assembled.incomplete_responses, 0);
        let first = &assembled.observations[0];
        let scale = 180.0 / std::f64::consts::PI;
        assert!((first.displacement.epi - 0.1 * scale).abs() < 1e-12);
        assert!((first.displacement.perp + 0.2 * scale).abs() < 1e-12);
        assert!((first.covariance.xx - 4.0e-6 * scale.powi(2)).abs() < 1e-15);
        assert!((first.covariance.xy + 1.5e-6 * scale.powi(2)).abs() < 1e-15);
        assert!((first.covariance.yy - 9.0e-6 * scale.powi(2)).abs() < 1e-15);
        assert!((first.jacobian.epi[4] - 0.05 * scale).abs() < 1e-12);
        assert!((first.jacobian.perp[4] + 0.10 * scale).abs() < 1e-12);
    }

    #[test]
    fn pose_assembly_refuses_fewer_than_four_sites_with_every_response() {
        let outcomes: Vec<_> = (0..4).map(assembled_outcome).collect();
        let responses = std::array::from_fn(|knob| {
            outcomes
                .iter()
                .enumerate()
                .map(|(index, outcome)| SiteResponse {
                    site: outcome.site,
                    result: if index == 3 && knob == 2 {
                        Err(ResponseRefused::ProjectedOut)
                    } else {
                        Ok(CameraDisplacement {
                            epi: 0.01,
                            perp: -0.01,
                        })
                    },
                })
                .collect()
        });
        assert!(matches!(
            assemble_pose_observations(&outcomes, &responses),
            Err(AssemblyRefused::TooFewCompleteSites { have: 3 })
        ));
    }

    fn temporal_samples(shift: [f64; 2]) -> Vec<RegistrationSample> {
        // A planted textured one-lens previous/next patch.  `shift` is in
        // the raw solver's `[perp, epi]` grid convention.
        (0..12)
            .map(|index| {
                let gradient = [1.0 + index as f64 * 0.11, 0.4 + (index % 3) as f64 * 0.17];
                RegistrationSample {
                    residual: gradient[0] * shift[0] + gradient[1] * shift[1],
                    gradient,
                    weight: 1.0,
                }
            })
            .collect()
    }

    #[test]
    fn temporal_tracker_recovers_a_known_displacement_without_changing_site_identity() {
        let site = assembled_site(4);
        let initial = TrackState::new(site, 0.1);
        let step = 0.01;
        let reading = advance_track(initial, &temporal_samples([1.5, -0.75]), step)
            .expect("the planted patch has two textured axes");
        assert_eq!(reading.state.site, site);
        assert_eq!(reading.state.site.root.view_pixel, [4.0, 0.0]);
        assert!((reading.increment_rad.perp - 0.015).abs() < 1e-12);
        assert!((reading.increment_rad.epi + 0.0075).abs() < 1e-12);
        assert_eq!(reading.state.accumulated_rad, reading.increment_rad);
        assert!(reading.condition.is_finite());
        assert_eq!(reading.samples, 12);
    }

    #[test]
    fn temporal_tracker_refuses_an_excursion_and_an_aperture_without_reselecting() {
        let site = assembled_site(2);
        let initial = TrackState::new(site, 0.01);
        assert!(matches!(
            advance_track(initial, &temporal_samples([1.5, 0.0]), 0.01),
            Err(TrackRefused::Excursion {
                attempted_rad: CameraDisplacement { perp, .. },
                cap_rad: 0.01,
            }) if (perp - 0.015).abs() < 1e-12
        ));
        let aperture = vec![
            RegistrationSample {
                gradient: [1.0, 0.0],
                residual: 0.2,
                weight: 1.0,
            };
            4
        ];
        assert_eq!(
            advance_track(initial, &aperture, 0.01),
            Err(TrackRefused::Aperture)
        );
        assert_eq!(
            initial.site, site,
            "a refusal must not relocate the declaration"
        );
    }

    #[test]
    fn temporal_closure_requires_the_same_site_and_sums_full_covariance() {
        let site = assembled_site(1);
        let forward = TrackReading {
            state: TrackState::new(site, 1.0),
            increment_rad: CameraDisplacement {
                epi: 0.02,
                perp: -0.03,
            },
            covariance_rad2: CameraCovariance {
                epi_epi: 2.0,
                epi_perp: -0.4,
                perp_perp: 4.0,
            },
            condition: 2.0,
            samples: 9,
        };
        let reverse = TrackReading {
            state: TrackState::new(site, 1.0),
            increment_rad: CameraDisplacement {
                epi: -0.02,
                perp: 0.03,
            },
            covariance_rad2: CameraCovariance {
                epi_epi: 3.0,
                epi_perp: 0.7,
                perp_perp: 5.0,
            },
            ..forward
        };
        let closure = track_closure(forward, reverse).expect("same fixed site closes");
        assert_eq!(
            closure.closure_rad,
            CameraDisplacement {
                epi: 0.0,
                perp: 0.0
            }
        );
        assert_eq!(closure.covariance_rad2.epi_epi, 5.0);
        assert!((closure.covariance_rad2.epi_perp - 0.3).abs() < 1e-12);
        assert_eq!(closure.covariance_rad2.perp_perp, 9.0);
        let other = TrackReading {
            state: TrackState::new(assembled_site(3), 1.0),
            ..reverse
        };
        assert_eq!(
            track_closure(forward, other),
            Err(TrackClosureRefused::MismatchedSite)
        );
    }
}
