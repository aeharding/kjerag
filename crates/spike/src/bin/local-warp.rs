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
use kjerag_render::{Camera, Cue, Horizon, Sampling, Scene, ScenePipeline, Size};
use kjerag_spike::{FORMAT, Gpu, Render, Walk, raw_register, seam_fit};

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let baseline = calibration
        .lenses
        .get(1)
        .map_or([0.0; 3], |lens| lens.pose.translation_m);
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let gpu = Gpu::open()?;
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let mut scene = Scene::still(&options.input, options.start())?;
    // The map drives both the warm render and the raw-lens projections below.
    // Hold one explicitly selected calibration before either so a comparison
    // is not silently between a file fit in one path and another baseline in
    // the other.
    options.seam.hold(&scene);
    scene.set_horizon(if options.lock {
        Horizon::Locked
    } else {
        Horizon::Free
    });

    // Keep this rendered traversal even though its pixels are discarded: the
    // pipeline's media-time band state and held camera pose are exactly what
    // makes `warm` mean the same thing here as it does in `step`.
    let mut rendered = 0usize;
    while let Some((_, at)) = scene.frame() {
        let _ = Render {
            gpu: &gpu,
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
    let candidates = raw_register::visible_candidates(&map, options.size, options.size, baseline);
    println!(
        "played: {rendered} frame(s), ending at {:.3} s; {} visible seam candidates",
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
    let coverage = raw_register::coverage_census(&map, &pair.lenses, options.size, options.size);
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
    for row in supports.into_iter().map(|support| {
        raw_register::overlap_strip_lattice(&map, &pair.lenses, &candidates, support)
    }) {
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
    }
    Ok(())
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
}

/// The same three seam paths that `step` and `reframe` expose.  Stage 9's
/// raw pixels remain raw; this choice only fixes the camera-frame map through
/// which both lenses are sampled.
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
                Some((key, _)) => return Err(format!("no argument called {key}. {USAGE}").into()),
            }
        }
        if out.input.as_os_str().is_empty() {
            return Err(USAGE.into());
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
     [size=px] [lock=0] [span=deg[,deg...]] [search=deg[,deg...]] [trace=1] \\
     [seam=factory|file|roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9]";

#[cfg(test)]
mod tests {
    use std::time::Duration;

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
