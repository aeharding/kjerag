//! Stage 9's observation-only raw-lens registration.
//!
//! `warm` follows the same rendered traversal as `step`, but only to decide
//! which physical seam contour the named view exposes.  Registration then
//! starts again from the synchronized decoded lens planes; no composited,
//! blended, colour-corrected, or warped output is matched.

use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_meta::CalibrationSet;
use kjerag_render::{Camera, Cue, Horizon, Reframe, Sampling, Scene, ScenePipeline, SeamFit, Size};
use kjerag_spike::{FORMAT, Gpu, Render, Walk, local_warp, raw_register, seam_fit};

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let baseline = calibration
        .lenses
        .get(1)
        .map_or([0.0; 3], |lens| lens.pose.translation_m);
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let gpu = Gpu::open()?;
    // Every response map is made by the same fresh traversal.  A perturbed
    // fit is never landed into an already-warmed scene, where a hidden
    // correction walk or different band history could mimic a derivative.
    let warmed = warm_map(&gpu, &options, options.seam)?;
    let at = warmed.at;
    let candidates =
        raw_register::visible_candidates(&warmed.map, options.size, options.size, baseline);
    println!(
        "played: {} frame(s), ending at {:.3} s; {} visible seam candidates",
        warmed.rendered,
        at.as_secs_f64(),
        candidates.len()
    );

    // `Walk` is deliberately independent of `Scene`: it returns delivered
    // raw planes and cannot accidentally hand this instrument an output pixel.
    let mut walk = Walk::open(&options.input, at.as_secs_f64(), frame)?;
    let pair = walk
        .next_pair()?
        .ok_or("no synchronized raw lens pair at that instant")?;
    require_same_pts(at, pair.at)?;
    println!(
        "pts:    raw pair and warmed Scene both at {:.9} s",
        at.as_secs_f64()
    );
    let coverage =
        raw_register::coverage_census(&warmed.map, &pair.lenses, options.size, options.size);
    println!(
        "coverage: view rays {}; outside view {}",
        coverage.view_rays, coverage.outside_view
    );
    for (lens, coverage) in coverage.lenses.iter().enumerate() {
        println!(
            "          lens {lens}: projected {}; readable {}; source-boundary {}",
            coverage.projected, coverage.readable, coverage.source_boundary
        );
    }
    let supports = options.supports()?;
    if let Some(frames) = options.temporal_frames {
        let [support] = supports.as_slice() else {
            return Err(
                "temporal=<frames> requires exactly one declared support: give one span= and one search= value"
                    .into(),
            );
        };
        let sites = raw_register::overlap_strip_sites(&candidates);
        report_temporal(&gpu, &options, &warmed, frame, &sites, *support, frames)?;
        return Ok(());
    }
    if options.fit && supports.len() != 1 {
        return Err(
            "fit=1 requires exactly one declared support: give one span= and one search= value"
                .into(),
        );
    }
    if options.reciprocal && supports.len() != 1 {
        return Err(
            "reciprocal=1 requires exactly one declared support: give one span= and one search= value"
                .into(),
        );
    }
    for support in supports {
        let row =
            raw_register::overlap_strip_lattice(&warmed.map, &pair.lenses, &candidates, support);
        let health = row.health;
        println!(
            "support: span {:.2} deg, search {:.2} deg, step {:.2} deg\nlattice: roots {}; sites {}; reference-complete {}; target shifts {}; target-complete {}\ncoverage: reference [projected-out {}, source-boundary {}]; target [projected-out {}, source-boundary {}]",
            row.support.span_deg,
            row.support.search_deg,
            row.support.step_deg,
            health.roots,
            health.sites,
            health.reference_complete,
            health.searched_offsets,
            health.target_complete,
            health.reference_projected_out,
            health.reference_source_boundary,
            health.target_projected_out,
            health.target_source_boundary,
        );
        if options.trace {
            for site in row.sites {
                println!(
                    "site: root view ({:.2}, {:.2}), body phi {:.2} deg; offset [perp {:.2}, epi {:.2}] deg; reference {:?}",
                    site.site.root.view_pixel[0],
                    site.site.root.view_pixel[1],
                    site.site.root.node.phi.to_degrees(),
                    site.site.offset_rad[0].to_degrees(),
                    site.site.offset_rad[1].to_degrees(),
                    site.reference,
                );
                for target in site.target {
                    println!(
                        "  shift: steps [perp {}, epi {}]; target offset [perp {:.2}, epi {:.2}] deg; {:?}",
                        target.steps[0],
                        target.steps[1],
                        target.offset_rad[0].to_degrees(),
                        target.offset_rad[1].to_degrees(),
                        target.coverage,
                    );
                }
            }
        }
        println!(
            "meaning: fixed raw-lens coverage only; no texture score selected a view or a warp."
        );
        if options.observations || options.fit {
            let outcomes = raw_register::register_overlap_strip(
                &warmed.map,
                &pair.lenses,
                &candidates,
                support,
            );
            if options.observations {
                report_observations(support, &outcomes, options.trace);
            }
            if options.fit {
                report_fit(&gpu, &options, &warmed, support, &outcomes)?;
            }
        }
        if options.reciprocal {
            // This deliberately reuses the declared lattice rather than any
            // textured subset selected by the forward registration.  It is a
            // reciprocal control, not a replacement observation estimator.
            let sites = raw_register::overlap_strip_sites(&candidates);
            let outcomes = raw_register::register_strip_sites_bidirectional(
                &warmed.map,
                &pair.lenses,
                &sites,
                support,
            );
            report_reciprocal(support, &outcomes, options.trace);
        }
    }
    if options.responses {
        let sites = raw_register::overlap_strip_sites(&candidates);
        report_responses(&gpu, &options, &warmed, &sites)?;
    }
    Ok(())
}

/// The outcome of one isolated, fully rendered traversal.  The frame count
/// is retained so a central difference cannot quietly compare equal PTSs
/// reached through different warm histories.
struct Warmed {
    map: Reframe,
    at: Duration,
    rendered: usize,
}

/// Rebuild and warm a scene from the same cue and explicit calibration.
///
/// This is intentionally the only route used for the finite-difference maps.
/// `Scene` owns held and correction state privately, so changing a fit after
/// a warm cannot prove the three maps saw the same traversal.
fn warm_map(gpu: &Gpu, options: &Options, seam: Seam) -> Fallible<Warmed> {
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let mut scene = Scene::still(&options.input, options.start())?;
    seam.hold(&scene);
    scene.set_horizon(if options.lock {
        Horizon::Locked
    } else {
        Horizon::Free
    });
    scene.set_sampling(Sampling::default());
    let mut rendered = 0usize;
    while let Some((_, at)) = scene.frame() {
        let _ = Render {
            gpu,
            scene: &scene,
            pipeline: &mut pipeline,
        }
        .frame(options.camera(), Sampling::default(), options.size())?;
        rendered += 1;
        if at.as_secs_f64() >= options.time || !scene.advance()? {
            break;
        }
    }
    let (_, at) = scene.frame().ok_or("no frame decoded at that instant")?;
    let map = scene
        .mapped(options.camera(), 1.0)
        .ok_or("no frame to map")?;
    Ok(Warmed { map, at, rendered })
}

/// Native-unit central-difference steps for the three angular and two pixel
/// calibration knobs.  These are diagnostic probes, not corrections.
const RESPONSE_KNOBS: [(&str, f64); 5] = [
    ("roll", 0.05),
    ("yaw", 0.05),
    ("pitch", 0.05),
    ("cx", 0.25),
    ("cy", 0.25),
];

fn perturb(mut fit: SeamFit, knob: usize, amount: f64) -> SeamFit {
    match knob {
        0 => fit.roll_deg += amount,
        1 => fit.yaw_deg += amount,
        2 => fit.pitch_deg += amount,
        3 => fit.cx_px += amount,
        4 => fit.cy_px += amount,
        _ => unreachable!("the response knob list has exactly five entries"),
    }
    fit
}

fn report_responses(
    gpu: &Gpu,
    options: &Options,
    base: &Warmed,
    sites: &[raw_register::StripSite],
) -> Fallible<()> {
    let Seam::Stored(_) = options.seam else {
        return Err("responses=1 requires seam=<stored fit>".into());
    };
    println!(
        "responses: frozen baseline roots/sites {}; central maps separately warmed",
        sites.len()
    );
    let responses = central_responses(gpu, options, base, sites)?;
    for ((name, half_step), column) in RESPONSE_KNOBS.into_iter().zip(responses) {
        let mut available = 0usize;
        let mut projected_out = 0usize;
        let mut singular = 0usize;
        for response in column {
            match response.result {
                Ok(_) => available += 1,
                Err(raw_register::ResponseRefused::ProjectedOut) => projected_out += 1,
                Err(raw_register::ResponseRefused::Singular) => singular += 1,
                Err(raw_register::ResponseRefused::InvalidStep) => {
                    unreachable!("constant positive step")
                }
            }
        }
        println!(
            "response: {name}; central half-step {half_step:.3}; sites {}; available {}; projected-out {}; singular {}; no pose fit or warp applied",
            sites.len(),
            available,
            projected_out,
            singular,
        );
    }
    Ok(())
}

/// Independently warm all five central response pairs for these frozen sites.
/// The returned columns retain every declared site and its refusal, so callers
/// cannot quietly replace a weak site before assembling a fit.
fn central_responses(
    gpu: &Gpu,
    options: &Options,
    base: &Warmed,
    sites: &[raw_register::StripSite],
) -> Fallible<[Vec<raw_register::SiteResponse>; local_warp::KNOBS]> {
    let Seam::Stored(fit) = options.seam else {
        return Err("central responses require seam=<stored fit>".into());
    };
    let mut columns = Vec::with_capacity(local_warp::KNOBS);
    for (index, (name, half_step)) in RESPONSE_KNOBS.into_iter().enumerate() {
        let minus = warm_map(gpu, options, Seam::Stored(perturb(fit, index, -half_step)))?;
        let plus = warm_map(gpu, options, Seam::Stored(perturb(fit, index, half_step)))?;
        require_same_warm(base, &minus, name, "minus")?;
        require_same_warm(base, &plus, name, "plus")?;
        columns.push(
            sites
                .iter()
                .map(|site| {
                    raw_register::site_response(&base.map, &minus.map, &plus.map, *site, half_step)
                })
                .collect(),
        );
    }
    Ok(columns
        .try_into()
        .expect("the five fixed calibration knobs produced five response columns"))
}

/// Report one capture and one support only.  This is an instrument readout:
/// it prints no decision threshold and never applies its fitted knobs.
fn report_fit(
    gpu: &Gpu,
    options: &Options,
    base: &Warmed,
    support: raw_register::Support,
    outcomes: &[raw_register::StripSiteOutcome],
) -> Fallible<()> {
    let sites: Vec<_> = outcomes.iter().map(|outcome| outcome.site).collect();
    let responses = central_responses(gpu, options, base, &sites)?;
    let assembled = raw_register::assemble_pose_observations(outcomes, &responses)
        .map_err(|refusal| format!("fit refused while assembling fixed sites: {refusal:?}"))?;
    println!(
        "fit: span {:.2} deg; raw readings {}; incomplete responses {}; complete observations {}",
        support.span_deg,
        assembled.raw_readings,
        assembled.incomplete_responses,
        assembled.observations.len(),
    );
    // A raw-lens pair cannot be physically perturbed by changing only its
    // projection map: its delivered pixels were captured with one fixed lens
    // pose.  `plant=` consequently does not pretend to be a second raw
    // capture.  It replaces only the assembled displacement by this same
    // independently-warmed map Jacobian's prediction, retaining the actual
    // fixed-site identity and full measured covariance.  It is an end-to-end
    // assembly/linear-solve control, not a registration or linearity claim.
    let observations = options.plant.map_or_else(
        || assembled.observations.clone(),
        |knobs| planted_observations(&assembled.observations, knobs),
    );
    if let Some(knobs) = options.plant {
        println!(
            "fit plant: synthetic map-Jacobian displacement; knobs roll {:+.6}; yaw {:+.6}; pitch {:+.6}; cx {:+.6}; cy {:+.6}; raw pixels were not re-captured or re-registered",
            knobs[0], knobs[1], knobs[2], knobs[3], knobs[4],
        );
    }
    if options.trace {
        for observation in &observations {
            println!(
                "fit observation: {}; measured [epi {:+.5}, perp {:+.5}] deg; covariance [[{:.3e}, {:.3e}], [{:.3e}, {:.3e}]] deg^2",
                observation.name,
                observation.displacement.epi,
                observation.displacement.perp,
                observation.covariance.xx,
                observation.covariance.xy,
                observation.covariance.xy,
                observation.covariance.yy,
            );
        }
    }
    let shared = local_warp::fit(&observations)
        .map_err(|refusal| format!("fit refused for this one capture/support: {refusal:?}"))?;
    println!(
        "fit pose: roll {:+.6}; yaw {:+.6}; pitch {:+.6}; cx {:+.6}; cy {:+.6}; diagnostic only",
        shared.knobs[0], shared.knobs[1], shared.knobs[2], shared.knobs[3], shared.knobs[4],
    );
    if options.trace {
        for ((observation, predicted), residual) in assembled
            .observations
            .iter()
            .zip(&shared.predicted)
            .zip(&shared.residuals)
        {
            println!(
                "fit residual: {}; predicted [epi {:+.5}, perp {:+.5}] deg; residual [epi {:+.5}, perp {:+.5}] deg",
                observation.name, predicted.epi, predicted.perp, residual.epi, residual.perp,
            );
        }
    }
    println!(
        "fit summary: chi2 {:.5}; dof {}; chi2/dof {:.5}; normalized-rms {:.5}; residual-rms {:.5} deg; condition {:.3e}; no threshold or warp applied",
        shared.chi_squared,
        shared.degrees_of_freedom,
        shared.chi_squared / shared.degrees_of_freedom as f64,
        shared.normalized_rms,
        shared.rms,
        shared.condition,
    );
    Ok(())
}

/// Make a known shared-pose reading through the exact observations which the
/// real `fit=1` path assembled.  This is kept here rather than in the pure
/// solver because its value is precisely that it exercises the raw-register
/// covariance/unit conversion and fixed-site response assembly first.
fn planted_observations(
    observations: &[local_warp::Observation],
    knobs: [f64; local_warp::KNOBS],
) -> Vec<local_warp::Observation> {
    observations
        .iter()
        .cloned()
        .map(|mut observation| {
            observation.displacement = local_warp::predict(observation.jacobian, knobs);
            observation
        })
        .collect()
}

fn fit_knobs(fit: kjerag_render::SeamFit) -> [f64; local_warp::KNOBS] {
    [
        fit.roll_deg,
        fit.yaw_deg,
        fit.pitch_deg,
        fit.cx_px,
        fit.cy_px,
    ]
}

fn require_same_warm(base: &Warmed, other: &Warmed, knob: &str, side: &str) -> Fallible<()> {
    if base.at == other.at && base.rendered == other.rendered {
        return Ok(());
    }
    Err(format!(
        "refused: {knob} {side} map ended at {:.9} s after {} frames, baseline was {:.9} s after {} frames",
        other.at.as_secs_f64(), other.rendered, base.at.as_secs_f64(), base.rendered,
    )
    .into())
}

/// `Scene` and `Walk` both report the container's media time as an exact
/// nanosecond `Duration`; they use the same floor conversion from PTS.  A
/// tolerance would therefore turn a different decoded frame into a seeming
/// raw-lens observation.  Refuse instead of registering it.
fn require_same_pts(scene: Duration, raw: Duration) -> Fallible<()> {
    if scene == raw {
        return Ok(());
    }
    Err(format!(
        "refused: warmed Scene PTS {:.9} s differs from raw-pair PTS {:.9} s; no registration was inferred",
        scene.as_secs_f64(),
        raw.as_secs_f64()
    )
    .into())
}

/// The body-fixed radius allowed for the entire opt-in temporal sequence.
/// This is deliberately distinct from one frame's local `search=` window:
/// exceeding it ends that declared track rather than moving it to a new root.
const TEMPORAL_EXCURSION_CAP_DEG: f64 = 5.0;

#[derive(Default)]
struct TemporalHealth {
    transitions: usize,
    tracked: usize,
    no_complete: usize,
    no_peak: usize,
    aperture: usize,
    excursion: usize,
    ended: usize,
}

/// Sequential, one-lens raw tracking after the same rendered warm-up used to
/// declare the anchor sites.  It intentionally owns neither a depth model nor
/// a pose fit: this is only temporal observability evidence.
fn report_temporal(
    gpu: &Gpu,
    options: &Options,
    anchor: &Warmed,
    frame: Size,
    sites: &[raw_register::StripSite],
    support: raw_register::Support,
    frames: usize,
) -> Fallible<()> {
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let mut scene = Scene::still(&options.input, options.start())?;
    options.seam.hold(&scene);
    scene.set_horizon(if options.lock {
        Horizon::Locked
    } else {
        Horizon::Free
    });
    scene.set_sampling(Sampling::default());
    let mut rendered = 0usize;
    while let Some((_, at)) = scene.frame() {
        let _ = Render {
            gpu,
            scene: &scene,
            pipeline: &mut pipeline,
        }
        .frame(options.camera(), Sampling::default(), options.size())?;
        rendered += 1;
        if at.as_secs_f64() >= options.time || !scene.advance()? {
            break;
        }
    }
    let (_, at) = scene.frame().ok_or("no frame decoded at temporal anchor")?;
    if at != anchor.at || rendered != anchor.rendered {
        return Err(
            "refused: temporal anchor did not reproduce the declared warm traversal".into(),
        );
    }
    let mut previous_map = scene
        .mapped(options.camera(), 1.0)
        .ok_or("no map at temporal anchor")?;
    let mut walk = Walk::open(&options.input, at.as_secs_f64(), frame)?;
    let mut previous_pair = walk
        .next_pair()?
        .ok_or("no synchronized raw lens pair at temporal anchor")?;
    require_same_pts(at, previous_pair.at)?;
    let cap = TEMPORAL_EXCURSION_CAP_DEG.to_radians();
    let mut states: Vec<Option<raw_register::TrackState>> = sites
        .iter()
        .copied()
        .map(|site| Some(raw_register::TrackState::new(site, cap)))
        .collect();
    let mut health = TemporalHealth::default();
    for _ in 0..frames {
        if !scene.advance()? {
            break;
        }
        let (_, next_at) = scene
            .frame()
            .ok_or("no frame decoded during temporal track")?;
        let _ = Render {
            gpu,
            scene: &scene,
            pipeline: &mut pipeline,
        }
        .frame(options.camera(), Sampling::default(), options.size())?;
        let next_map = scene
            .mapped(options.camera(), 1.0)
            .ok_or("no map during temporal track")?;
        let next_pair = walk
            .next_pair()?
            .ok_or("raw lens pair ended during temporal track")?;
        require_same_pts(next_at, next_pair.at)?;
        health.transitions += 1;
        for state in &mut states {
            let Some(current) = *state else {
                continue;
            };
            match raw_register::track_one_lens(
                &previous_map,
                &previous_pair.lenses[0],
                &next_map,
                &next_pair.lenses[0],
                0,
                current,
                support,
            ) {
                Ok(reading) => {
                    health.tracked += 1;
                    *state = Some(reading.state);
                }
                Err(refusal) => {
                    match refusal {
                        raw_register::TrackRefused::NoCompletePatch => health.no_complete += 1,
                        raw_register::TrackRefused::NoPeak
                        | raw_register::TrackRefused::InvalidStep
                        | raw_register::TrackRefused::InvalidExcursionCap => health.no_peak += 1,
                        raw_register::TrackRefused::Aperture => health.aperture += 1,
                        raw_register::TrackRefused::Excursion { .. } => health.excursion += 1,
                    }
                    health.ended += 1;
                    *state = None;
                }
            }
        }
        previous_map = next_map;
        previous_pair = next_pair;
    }
    let active = states.iter().filter(|state| state.is_some()).count();
    println!(
        "temporal: lens 0; anchor {:.9} s; requested frames {}; transitions {}; declared sites {}; active {}; ended {}; successful steps {}; no-complete {}; no-peak {}; aperture {}; excursion {}; cap {:.2} deg",
        anchor.at.as_secs_f64(),
        frames,
        health.transitions,
        sites.len(),
        active,
        health.ended,
        health.tracked,
        health.no_complete,
        health.no_peak,
        health.aperture,
        health.excursion,
        TEMPORAL_EXCURSION_CAP_DEG,
    );
    println!(
        "temporal closure: unavailable (this opt-in is one forward lens-0 traversal; no reverse traversal was inferred); no depth, pose fit, or warp applied"
    );
    Ok(())
}

struct Options {
    input: PathBuf,
    time: f64,
    warm: f64,
    yaw: f64,
    pitch: f64,
    fov: f64,
    size: u32,
    lock: bool,
    seam: Seam,
    spans: Option<Vec<f64>>,
    searches: Option<Vec<f64>>,
    trace: bool,
    observations: bool,
    responses: bool,
    fit: bool,
    reciprocal: bool,
    temporal_frames: Option<usize>,
    plant: Option<[f64; local_warp::KNOBS]>,
}

/// The same three seam paths that `step` and `reframe` expose.  Stage 9's
/// raw pixels remain raw; this choice only fixes the camera-frame map through
/// which both lenses are sampled.
#[derive(Clone, Copy)]
enum Seam {
    Factory,
    File,
    Stored(kjerag_render::SeamFit),
}

impl Seam {
    fn hold(&self, scene: &Scene) {
        match self {
            Self::Factory => println!("seam:   factory calibration, no correction"),
            Self::File => scene.fit_seam(true),
            Self::Stored(fit) => scene.use_seam(*fit),
        }
    }
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut out = Self {
            input: PathBuf::new(),
            time: 0.0,
            warm: 0.0,
            yaw: 90.0,
            pitch: 0.0,
            fov: 20.0,
            size: 1024,
            lock: true,
            // The shipped/configured baseline is this file's fitted
            // calibration, as it is in `step`; `factory` is an explicit
            // control rather than an accidental alternate baseline.
            seam: Seam::File,
            spans: None,
            searches: None,
            trace: false,
            observations: false,
            responses: false,
            fit: false,
            reciprocal: false,
            temporal_frames: None,
            plant: None,
        };
        for arg in args {
            match arg.split_once('=') {
                None => out.input = PathBuf::from(arg),
                Some(("time", v)) => out.time = v.parse()?,
                Some(("warm", v)) => out.warm = v.parse()?,
                Some(("yaw", v)) => out.yaw = v.parse()?,
                Some(("pitch", v)) => out.pitch = v.parse()?,
                Some(("fov", v)) => out.fov = v.parse()?,
                Some(("size", v)) => out.size = v.parse()?,
                Some(("lock", v)) => out.lock = v.parse::<u32>()? != 0,
                Some(("seam", value)) => {
                    out.seam = match value {
                        "factory" => Seam::Factory,
                        "file" => Seam::File,
                        _ => Seam::Stored(seam_fit(value)?),
                    }
                }
                Some(("span", value)) => out.spans = Some(degrees(value)?),
                Some(("search", value)) => out.searches = Some(degrees(value)?),
                Some(("trace", value)) => out.trace = value.parse::<u32>()? != 0,
                Some(("observations", value)) => out.observations = value.parse::<u32>()? != 0,
                Some(("responses", value)) => out.responses = value.parse::<u32>()? != 0,
                Some(("fit", value)) => out.fit = value.parse::<u32>()? != 0,
                Some(("reciprocal", value)) => out.reciprocal = value.parse::<u32>()? != 0,
                Some(("temporal", value)) => {
                    let frames: usize = value.parse()?;
                    if frames == 0 {
                        return Err("temporal must be a positive frame count".into());
                    }
                    out.temporal_frames = Some(frames);
                }
                Some(("plant", value)) => out.plant = Some(fit_knobs(seam_fit(value)?)),
                Some((key, _)) => return Err(format!("no argument called {key}. {USAGE}").into()),
            }
        }
        if out.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        if (out.observations
            || out.responses
            || out.fit
            || out.reciprocal
            || out.temporal_frames.is_some())
            && !matches!(&out.seam, Seam::Stored(_))
        {
            return Err(
                "observations=1/responses=1/fit=1/reciprocal=1 require seam=<stored fit>; factory/file are coverage-only controls"
                    .into(),
            );
        }
        if out.plant.is_some() && !out.fit {
            return Err(
                "plant=<knobs> requires fit=1; it is a pose-fit control, not a renderer option"
                    .into(),
            );
        }
        if out.temporal_frames.is_some()
            && (out.observations || out.responses || out.fit || out.reciprocal)
        {
            return Err(
                "temporal=<frames> is observation-only and cannot combine with observations/responses/fit/reciprocal"
                    .into(),
            );
        }
        // Reject before opening media or warming a scene.  A reciprocal
        // closure is only interpretable at one pre-declared support scale.
        if out.reciprocal && out.supports()?.len() != 1 {
            return Err(
                "reciprocal=1 requires exactly one declared support: give one span= and one search= value"
                    .into(),
            );
        }
        Ok(out)
    }
    fn start(&self) -> Cue {
        Cue::Time(Duration::from_secs_f64((self.time - self.warm).max(0.0)))
    }
    fn camera(&self) -> Camera {
        Camera {
            yaw: self.yaw.to_radians() as f32,
            pitch: self.pitch.to_radians() as f32,
            fov: self.fov.to_radians() as f32,
        }
    }
    fn size(&self) -> Size {
        Size::new(self.size, self.size)
    }
    fn supports(&self) -> Fallible<Vec<raw_register::Support>> {
        let default = raw_register::SUPPORT_LADDER;
        match (&self.spans, &self.searches) {
            (None, None) => Ok(default.to_vec()),
            (spans, searches) => {
                let spans = spans.as_deref().unwrap_or(&[]);
                let searches = searches.as_deref().unwrap_or(&[]);
                let count = spans.len().max(searches.len());
                if count == 0
                    || (spans.len() != 1 && spans.len() != count)
                    || (searches.len() != 1 && searches.len() != count)
                {
                    return Err(
                        "span/search must each provide one value or equally many values".into(),
                    );
                }
                Ok((0..count)
                    .map(|index| raw_register::Support {
                        span_deg: spans.get(index).copied().unwrap_or(spans[0]),
                        search_deg: searches.get(index).copied().unwrap_or(searches[0]),
                        step_deg: default[0].step_deg,
                    })
                    .collect())
            }
        }
    }
}
fn degrees(value: &str) -> Fallible<Vec<f64>> {
    let values: Result<Vec<f64>, _> = value.split(',').map(str::parse).collect();
    let values = values?;
    if values.is_empty()
        || values
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err("angular support values must be positive finite degrees".into());
    }
    Ok(values)
}

const USAGE: &str = "usage: local-warp <file.insv> time=seconds warm=seconds yaw=deg pitch=deg fov=deg \\
     [size=px] [lock=0] [span=deg[,deg...]] [search=deg[,deg...]] [trace=1] [observations=1] [responses=1] [fit=1] [reciprocal=1] [temporal=frames] [plant=roll:0.1,yaw:0,pitch:0,cx:0,cy:0] \\
     [seam=factory|file|roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9]";

#[derive(Default)]
struct ObservationHealth {
    readings: usize,
    no_peak: usize,
    aperture: usize,
    no_complete: usize,
}

fn report_observations(
    support: raw_register::Support,
    outcomes: &[raw_register::StripSiteOutcome],
    trace: bool,
) {
    let mut health = ObservationHealth::default();
    for outcome in outcomes {
        match outcome.result {
            Ok(reading) => {
                health.readings += 1;
                if trace {
                    println!(
                        "observation: root body phi {:.2} deg; offset [perp {:.2}, epi {:.2}] deg; shift [epi {:.4}, perp {:.4}] deg; correlation {:.4}; condition {:.2}",
                        reading.site.root.node.phi.to_degrees(),
                        reading.site.offset_rad[0].to_degrees(),
                        reading.site.offset_rad[1].to_degrees(),
                        reading.displacement_rad.epi.to_degrees(),
                        reading.displacement_rad.perp.to_degrees(),
                        reading.correlation,
                        reading.condition,
                    );
                }
            }
            Err(refusal) => {
                match refusal {
                    raw_register::Refused::NoPeak => health.no_peak += 1,
                    raw_register::Refused::Aperture => health.aperture += 1,
                    raw_register::Refused::NoCompletePatch => health.no_complete += 1,
                    raw_register::Refused::NoVisibleSeam => health.no_complete += 1,
                }
                if trace {
                    println!(
                        "observation: root body phi {:.2} deg; offset [perp {:.2}, epi {:.2}] deg; refused {:?}",
                        outcome.site.root.node.phi.to_degrees(),
                        outcome.site.offset_rad[0].to_degrees(),
                        outcome.site.offset_rad[1].to_degrees(),
                        refusal,
                    );
                }
            }
        }
    }
    println!(
        "observations: span {:.2} deg; sites {}; readings {}; no-peak {}; aperture {}; no-complete {}; no pose fit or warp applied",
        support.span_deg,
        outcomes.len(),
        health.readings,
        health.no_peak,
        health.aperture,
        health.no_complete,
    );
}

/// Aggregate the reciprocal control without treating an unavailable direction
/// as a zero displacement.  The covariance is the explicitly propagated sum
/// of the two directional registrations; the covariance of the reported mean
/// is that sum divided by the square of the number of complete pairs.
#[derive(Default)]
struct ReciprocalHealth {
    both: usize,
    forward_refused: usize,
    reverse_refused: usize,
    closure_epi_sum: f64,
    closure_perp_sum: f64,
    closure_norm_squared_sum: f64,
    covariance_epi_epi_sum: f64,
    covariance_epi_perp_sum: f64,
    covariance_perp_perp_sum: f64,
}

fn report_reciprocal(
    support: raw_register::Support,
    outcomes: &[raw_register::BidirectionalOutcome],
    trace: bool,
) {
    let mut health = ReciprocalHealth::default();
    let radians_to_degrees = 180.0 / std::f64::consts::PI;
    let covariance_scale = radians_to_degrees.powi(2);
    for outcome in outcomes {
        match outcome.result {
            Ok(reading) => {
                health.both += 1;
                health.closure_epi_sum += reading.closure.epi;
                health.closure_perp_sum += reading.closure.perp;
                health.closure_norm_squared_sum +=
                    reading.closure.epi.powi(2) + reading.closure.perp.powi(2);
                health.covariance_epi_epi_sum += reading.closure_covariance_rad2.epi_epi;
                health.covariance_epi_perp_sum += reading.closure_covariance_rad2.epi_perp;
                health.covariance_perp_perp_sum += reading.closure_covariance_rad2.perp_perp;
                if trace {
                    println!(
                        "reciprocal: root body phi {:.2} deg; offset [perp {:.2}, epi {:.2}] deg; closure [epi {:.4}, perp {:.4}] deg; summed covariance [epi² {:.6}, epi-perp {:.6}, perp² {:.6}] deg²",
                        reading.site.root.node.phi.to_degrees(),
                        reading.site.offset_rad[0].to_degrees(),
                        reading.site.offset_rad[1].to_degrees(),
                        reading.closure.epi.to_degrees(),
                        reading.closure.perp.to_degrees(),
                        reading.closure_covariance_rad2.epi_epi * covariance_scale,
                        reading.closure_covariance_rad2.epi_perp * covariance_scale,
                        reading.closure_covariance_rad2.perp_perp * covariance_scale,
                    );
                }
            }
            Err(refusal) => {
                health.forward_refused += refusal.forward.is_some() as usize;
                health.reverse_refused += refusal.reverse.is_some() as usize;
                if trace {
                    println!(
                        "reciprocal: root body phi {:.2} deg; offset [perp {:.2}, epi {:.2}] deg; forward refused {:?}; reverse refused {:?}",
                        outcome.site.root.node.phi.to_degrees(),
                        outcome.site.offset_rad[0].to_degrees(),
                        outcome.site.offset_rad[1].to_degrees(),
                        refusal.forward,
                        refusal.reverse,
                    );
                }
            }
        }
    }
    if health.both == 0 {
        println!(
            "reciprocal: span {:.2} deg; sites {}; both 0; forward-refused {}; reverse-refused {}; closure unavailable; no pose fit or warp applied",
            support.span_deg,
            outcomes.len(),
            health.forward_refused,
            health.reverse_refused,
        );
        return;
    }
    let count = health.both as f64;
    let mean_epi = health.closure_epi_sum / count;
    let mean_perp = health.closure_perp_sum / count;
    let rms = (health.closure_norm_squared_sum / count).sqrt();
    // Independent pair covariances add.  Dividing their total by n² is the
    // covariance of the displayed mean, rather than an optimistic sample
    // variance inferred from the closures themselves.
    let mean_scale = covariance_scale / count.powi(2);
    println!(
        "reciprocal: span {:.2} deg; sites {}; both {}; forward-refused {}; reverse-refused {}; closure mean [epi {:.4}, perp {:.4}] deg; closure RMS {:.4} deg; summed covariance of mean [epi² {:.6}, epi-perp {:.6}, perp² {:.6}] deg²; no pose fit or warp applied",
        support.span_deg,
        outcomes.len(),
        health.both,
        health.forward_refused,
        health.reverse_refused,
        mean_epi.to_degrees(),
        mean_perp.to_degrees(),
        rms.to_degrees(),
        health.covariance_epi_epi_sum * mean_scale,
        health.covariance_epi_perp_sum * mean_scale,
        health.covariance_perp_perp_sum * mean_scale,
    );
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use kjerag_spike::local_warp::{Covariance, Displacement, Jacobian, Observation};

    use super::{Options, Seam};

    fn options(args: &[&str]) -> Options {
        Options::parse(args.iter().map(|arg| arg.to_string())).expect("valid local-warp options")
    }

    #[test]
    fn seam_defaults_to_the_file_calibration() {
        assert!(matches!(options(&["flight.insv"]).seam, Seam::File));
    }

    #[test]
    fn seam_accepts_each_explicit_calibration_path() {
        assert!(matches!(
            options(&["flight.insv", "seam=factory"]).seam,
            Seam::Factory
        ));
        assert!(matches!(
            options(&["flight.insv", "seam=file"]).seam,
            Seam::File
        ));
        let Seam::Stored(fit) = options(&[
            "flight.insv",
            "seam=roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9",
        ])
        .seam
        else {
            panic!("stored seam fit was not parsed")
        };
        assert_eq!(fit.roll_deg, 0.6);
        assert_eq!(fit.yaw_deg, -2.1);
        assert_eq!(fit.pitch_deg, -0.9);
        assert_eq!(fit.cx_px, -9.5);
        assert_eq!(fit.cy_px, -11.9);
    }

    #[test]
    fn angular_supports_are_global_and_pair_or_broadcast() {
        let paired = options(&["flight.insv", "span=1.2,2.8", "search=1.0,2.4"])
            .supports()
            .expect("paired angular ladder");
        assert_eq!(
            paired
                .iter()
                .map(|support| support.span_deg)
                .collect::<Vec<_>>(),
            vec![1.2, 2.8]
        );
        assert_eq!(
            paired
                .iter()
                .map(|support| support.search_deg)
                .collect::<Vec<_>>(),
            vec![1.0, 2.4]
        );
        let broadcast = options(&["flight.insv", "span=2.0", "search=1.0,1.6"])
            .supports()
            .expect("one span broadcasts");
        assert!(broadcast.iter().all(|support| support.span_deg == 2.0));
    }

    #[test]
    fn trace_is_opt_in() {
        assert!(!options(&["flight.insv"]).trace);
        assert!(options(&["flight.insv", "trace=1"]).trace);
        assert!(!options(&["flight.insv", "trace=0"]).trace);
    }

    #[test]
    fn temporal_frames_are_opt_in_and_observation_only() {
        assert_eq!(options(&["flight.insv"]).temporal_frames, None);
        let temporal = options(&[
            "flight.insv",
            "temporal=12",
            "span=1.2",
            "search=1.0",
            "seam=roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9",
        ]);
        assert_eq!(temporal.temporal_frames, Some(12));
        assert!(
            Options::parse(
                ["flight.insv", "temporal=0"]
                    .into_iter()
                    .map(str::to_string)
            )
            .is_err()
        );
        assert!(
            Options::parse(
                ["flight.insv", "temporal=3", "seam=file"]
                    .into_iter()
                    .map(str::to_string)
            )
            .is_err()
        );
        assert!(
            Options::parse(
                [
                    "flight.insv",
                    "temporal=3",
                    "observations=1",
                    "seam=roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9",
                ]
                .into_iter()
                .map(str::to_string)
            )
            .is_err()
        );
    }

    #[test]
    fn observations_are_opt_in_and_require_a_stored_fit() {
        assert!(!options(&["flight.insv"]).observations);
        assert!(
            Options::parse(
                ["flight.insv", "observations=1"]
                    .into_iter()
                    .map(str::to_string)
            )
            .is_err()
        );
        assert!(
            Options::parse(
                ["flight.insv", "observations=1", "seam=factory"]
                    .into_iter()
                    .map(str::to_string)
            )
            .is_err()
        );
        assert!(
            options(&[
                "flight.insv",
                "observations=1",
                "seam=roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9",
            ])
            .observations
        );
        assert!(
            Options::parse(
                ["flight.insv", "responses=1", "seam=file"]
                    .into_iter()
                    .map(str::to_string)
            )
            .is_err()
        );
        assert!(
            options(&[
                "flight.insv",
                "responses=1",
                "seam=roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9",
            ])
            .responses
        );
        assert!(
            Options::parse(
                ["flight.insv", "fit=1", "seam=file"]
                    .into_iter()
                    .map(str::to_string)
            )
            .is_err()
        );
        assert!(
            options(&[
                "flight.insv",
                "fit=1",
                "seam=roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9",
            ])
            .fit
        );
        assert!(
            Options::parse(
                ["flight.insv", "reciprocal=1", "seam=file"]
                    .into_iter()
                    .map(str::to_string)
            )
            .is_err()
        );
        assert!(
            Options::parse(
                [
                    "flight.insv",
                    "reciprocal=1",
                    "seam=roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9",
                ]
                .into_iter()
                .map(str::to_string)
            )
            .is_err()
        );
        let reciprocal = options(&[
            "flight.insv",
            "reciprocal=1",
            "span=1.2",
            "search=1.0",
            "seam=roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9",
        ]);
        assert!(reciprocal.reciprocal);
        assert_eq!(
            reciprocal.supports().expect("one reciprocal support").len(),
            1
        );
    }

    #[test]
    fn fit_requires_one_declared_support() {
        let default = options(&[
            "flight.insv",
            "fit=1",
            "seam=roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9",
        ]);
        assert!(default.supports().expect("the default ladder").len() > 1);
        let one = options(&[
            "flight.insv",
            "fit=1",
            "span=1.2",
            "search=1.0",
            "seam=roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9",
        ]);
        assert_eq!(one.supports().expect("one declared support").len(), 1);
    }

    #[test]
    fn plant_requires_fit_and_preserves_the_declared_knob_units() {
        assert!(
            Options::parse(
                [
                    "flight.insv",
                    "plant=roll:0.1,yaw:-0.2,pitch:0.3,cx:4,cy:-5"
                ]
                .into_iter()
                .map(str::to_string)
            )
            .is_err()
        );
        let planted = options(&[
            "flight.insv",
            "fit=1",
            "span=1.2",
            "search=1.0",
            "seam=roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9",
            "plant=roll:0.1,yaw:-0.2,pitch:0.3,cx:4,cy:-5",
        ]);
        assert_eq!(planted.plant, Some([0.1, -0.2, 0.3, 4.0, -5.0]));
    }

    #[test]
    fn plant_recovers_through_the_same_observation_shape_as_fit() {
        let wanted = [0.31, -0.17, 0.08, 0.43, -0.29];
        let observations: Vec<_> = (0..6)
            .map(|index| {
                let x = index as f64 + 1.0;
                Observation {
                    name: format!("fixed-site-{index}"),
                    displacement: Displacement::default(),
                    covariance: Covariance::diagonal(0.01, 0.01),
                    jacobian: Jacobian {
                        epi: [1.0, x, x * x, (0.7 * x).sin(), (0.3 * x).cos()],
                        perp: [x * x, 1.0, (0.5 * x).cos(), x, (0.9 * x).sin()],
                    },
                }
            })
            .collect();
        let planted = super::planted_observations(&observations, wanted);
        let solved = kjerag_spike::local_warp::fit(&planted)
            .expect("well-spread sites constrain five knobs");
        for (got, expected) in solved.knobs.into_iter().zip(wanted) {
            assert!(
                (got - expected).abs() < 1e-10,
                "{got} instead of {expected}"
            );
        }
        assert!(solved.rms < 1e-11);
    }

    #[test]
    fn central_difference_changes_only_the_named_knob() {
        let original = kjerag_render::SeamFit {
            roll_deg: 1.0,
            yaw_deg: 2.0,
            pitch_deg: 3.0,
            cx_px: 4.0,
            cy_px: 5.0,
        };
        for knob in 0..5 {
            let changed = super::perturb(original, knob, 0.25);
            let before = [
                original.roll_deg,
                original.yaw_deg,
                original.pitch_deg,
                original.cx_px,
                original.cy_px,
            ];
            let after = [
                changed.roll_deg,
                changed.yaw_deg,
                changed.pitch_deg,
                changed.cx_px,
                changed.cy_px,
            ];
            for axis in 0..5 {
                let expected = if axis == knob { 0.25 } else { 0.0 };
                assert_eq!(after[axis] - before[axis], expected);
            }
        }
    }

    #[test]
    fn raw_pair_must_have_the_warmed_scenes_exact_pts() {
        assert!(
            super::require_same_pts(Duration::from_nanos(1001), Duration::from_nanos(1001)).is_ok()
        );
        assert!(
            super::require_same_pts(Duration::from_nanos(1001), Duration::from_nanos(1002))
                .is_err()
        );
    }
}
