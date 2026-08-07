//! The along-seam ring read over and over through one session, so the
//! question "how long does an accumulated per-direction field take to stop
//! moving" has readings behind it (issue #103, stage 9 layer 2).
//!
//! ```sh
//! # one flight, a ring every 5 s over its first 20 minutes, one frame each
//! cargo run --release -p kjerag-spike --bin settle -- <a.insv> \
//!   seam=roll:0.795,yaw:-2.310,pitch:-0.936,cx:-3.28,cy:-11.91 \
//!   every=5 span=1200 dump=scratch/layer2/may01.csv
//! ```
//!
//! **The readings are `--bin table`'s own, kept per moment instead of
//! pooled.** `kjerag_render::seam::measure` averages every frame it reads into
//! one number per azimuth and hands back the ring; what a convergence question
//! needs is the frames themselves, so this walks the same file with the same
//! [`seam::Probe`], calls the same [`seam::read_ring_centred`] on each frame,
//! and writes one row per moment per azimuth. Nothing here re-derives a seam
//! measurement and nothing here gates one: the pose is subtracted exactly the
//! way [`seam::left`] subtracts it, and the gate the readings then pass
//! through is applied by whatever reads this file, on the whole session or on
//! a span of it, which is the thing being measured.
//!
//! **One search centre for the whole session**, acquired from the first frame
//! that can answer and held ([`seam::acquired`]), because what it measures is
//! fixed in the camera for the life of the file and a per-moment centre would
//! put the instrument's own drift into the column being watched.
//!
//! **One stored pose, never a per-file fit**, for `--bin table`'s reason: a
//! fit off this capture's own frames absorbs this scene into the pose.
//!
//! The CSV lands where it is asked to; `scratch/` is gitignored, and these are
//! moments of somebody's real flights.

use std::fmt::Write as _;
use std::path::PathBuf;

use kjerag_media::{Fallible, Walk};
use kjerag_meta::CalibrationSet;
use kjerag_render::{SeamFit, Size, seam};
use kjerag_spike::seam_fit;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    walk(&options)
}

/// One azimuth of one moment: what the two lenses disagreed by there, and
/// what the pose leaves of it.
struct Row {
    seconds: f64,
    index: usize,
    phi_deg: f64,
    along_deg: f64,
    across_deg: f64,
    left_deg: f64,
    r: f64,
    contrast: f64,
}

fn walk(options: &Options) -> Fallible<()> {
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = calibration.lenses.clone();
    let files = kjerag_render::capture_set::resolve(&options.input).files;
    // The map the ring is READ through. Factory unless a plant is asked for,
    // which is the positive control: a known turn on lens 1 must move every
    // reading by exactly what `seam::moved` says it will, on both axes, and a
    // run with `plant=` beside one without it is the only way to see that
    // through the real correlator rather than through the arithmetic alone.
    let read_through = match options.plant {
        Some(plant) => seam::mapped(&plant.applied(&lenses), frame),
        None => seam::mapped(&lenses, frame),
    };
    let base = seam::mapped(&lenses, frame);
    // What the pose would move each azimuth's reading by, once, because it is
    // a function of the azimuth and the calibration and of nothing that
    // changes while the file plays. `seam::left`'s own arithmetic.
    let corrected = seam::mapped(&options.seam.applied(&lenses), frame);
    let ring = seam::ring(options.probe.patches);
    let shift: Vec<Option<f64>> = ring
        .iter()
        .map(|at| seam::moved(&base, &corrected, 1, at).map(|axes| axes[0]))
        .collect();
    if let Some(plant) = options.plant {
        let planted = seam::mapped(&plant.applied(&lenses), frame);
        println!(
            "plant:  lens 1 turned roll {:+.3} yaw {:+.3} pitch {:+.3} cx {:+.2} cy {:+.2}. every \n\
             \x20       reading below is read THROUGH that turn, so it must sit `predicted` from \n\
             \x20       where a run with no plant reads it.",
            plant.roll_deg, plant.yaw_deg, plant.pitch_deg, plant.cx_px, plant.cy_px
        );
        println!("index,phi_deg,predicted_along_deg,predicted_across_deg");
        for (index, at) in ring.iter().enumerate() {
            if let Some(axes) = seam::moved(&base, &planted, 1, at) {
                println!(
                    "plant,{index},{:.4},{:.6},{:.6}",
                    at.phi.to_degrees(),
                    axes[0],
                    axes[1]
                );
            }
        }
    }

    println!(
        "scale:  {:.2} source px per degree along the seam, median round the ring, in the {}x{} \n\
         \x20       delivered frame. every degree below is this many pixels of what the lens \n\
         \x20       actually recorded.",
        scale(&base, &ring),
        frame.width,
        frame.height,
    );
    let mut walk = Walk::over(&files, 0.0, frame)?;
    if walk.streams() < 2 {
        return Err("this capture carries one lens stream, so it has no seam".into());
    }
    let duration = walk.duration().as_secs_f64();
    let span = options.span.min(duration - options.from);
    let mut centre = None;
    let mut rows = Vec::new();
    let mut moments = 0;
    let mut refused = seam::Refused::default();
    let mut at = options.from;
    while at < options.from + span {
        walk.jump(at)?;
        for _ in 0..options.frames.max(1) {
            let Some(pair) = walk.next_pair()? else {
                break;
            };
            if centre.is_none() {
                centre = seam::acquired(&read_through, &pair.lenses, &ring, &options.probe);
            }
            moments += 1;
            let found = seam::read_ring_centred(
                &read_through,
                &pair.lenses,
                &ring,
                &options.probe,
                centre.unwrap_or(0),
                &mut refused,
            );
            for (index, (found, at)) in found.iter().zip(&ring).enumerate() {
                let Some(found) = found.filter(|f| f.r >= options.probe.keep) else {
                    continue;
                };
                let Some(shift) = shift[index] else {
                    continue;
                };
                rows.push(Row {
                    seconds: pair.at.as_secs_f64(),
                    index,
                    phi_deg: at.phi.to_degrees(),
                    along_deg: found.along,
                    across_deg: found.across,
                    left_deg: found.along + shift,
                    r: found.r,
                    contrast: found.contrast,
                });
            }
        }
        at += options.every;
    }
    println!(
        "read:   {} moments over {span:.0} s from {:.0} s, {} readings at {} azimuths, search \
         centred {} steps out; refused {} outside {} flat {} unlike {} pinned",
        moments,
        options.from,
        rows.len(),
        options.probe.patches,
        centre.unwrap_or(0),
        refused.outside,
        refused.flat,
        refused.unlike,
        refused.pinned,
    );
    write(&rows, options)
}

/// How many pixels of the delivered frame one degree along the seam is, at
/// the front lens, as a median round the ring.
///
/// `seam::moved`'s own Jacobian column, which is where every degree in this
/// project is turned into a pixel: the map is probed a hundredth of a degree
/// each way along the seam tangent and asked how far the projection travelled.
fn scale(base: &kjerag_render::Reframe, ring: &[seam::Where]) -> f64 {
    let probe = 0.01f64.to_radians();
    let mut all: Vec<f64> = ring
        .iter()
        .filter_map(|at| {
            let step = |sign: f64| {
                let ray: [f64; 3] =
                    std::array::from_fn(|c| at.centre[c] + sign * probe * at.along[c]);
                let length = ray.iter().map(|c| c * c).sum::<f64>().sqrt();
                let landing = base.project(0, ray.map(|c| (c / length) as f32));
                landing.inside.then_some(landing.pixel)
            };
            let (plus, minus) = (step(1.0)?, step(-1.0)?);
            Some(
                f64::from(plus[0] - minus[0]).hypot(f64::from(plus[1] - minus[1]))
                    / (2.0 * probe).to_degrees(),
            )
        })
        .collect();
    all.sort_by(f64::total_cmp);
    all.get(all.len() / 2).copied().unwrap_or(f64::NAN)
}

/// Every reading, stamped with what produced it: this repo's rule is that a
/// CSV says its own source and arguments or it is not evidence.
fn write(rows: &[Row], options: &Options) -> Fallible<()> {
    let Some(dump) = &options.dump else {
        return Ok(());
    };
    if let Some(parent) = dump.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = format!(
        "# source: kjerag-spike --bin settle ({})\n# args: {}\n# file: {}\n\
         # seam: roll:{} yaw:{} pitch:{} cx:{} cy:{}\n\
         # probe: patches={} span={} step={} along={} across={} keep={} contrast={}\n\
         # scale: see the run's own `scale:` line for source px per degree\n\
         # left_deg is along_deg with the pose's own shift added, which is seam::left's \
         arithmetic before its gate.\n",
        env!("CARGO_PKG_VERSION"),
        std::env::args().skip(1).collect::<Vec<_>>().join(" "),
        options.input.display(),
        options.seam.roll_deg,
        options.seam.yaw_deg,
        options.seam.pitch_deg,
        options.seam.cx_px,
        options.seam.cy_px,
        options.probe.patches,
        options.probe.span,
        options.probe.step,
        options.probe.along,
        options.probe.across,
        options.probe.keep,
        options.probe.contrast,
    );
    text.push_str("seconds,index,phi_deg,along_deg,across_deg,left_deg,r,contrast\n");
    for row in rows {
        let _ = writeln!(
            text,
            "{:.3},{},{:.4},{:.6},{:.6},{:.6},{:.4},{:.2}",
            row.seconds,
            row.index,
            row.phi_deg,
            row.along_deg,
            row.across_deg,
            row.left_deg,
            row.r,
            row.contrast,
        );
    }
    std::fs::write(dump, text)?;
    println!("wrote:  {}", dump.display());
    Ok(())
}

struct Options {
    input: PathBuf,
    seam: SeamFit,
    probe: seam::Probe,
    /// How far apart the moments are, in seconds of media time.
    every: f64,
    /// How many frames are read at each moment.
    frames: usize,
    /// A known turn on lens 1 the ring is read through: the positive control.
    plant: Option<SeamFit>,
    from: f64,
    span: f64,
    dump: Option<PathBuf>,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut input = None;
        let mut seam = None;
        let mut probe = seam::Probe::default();
        let mut every = 5.0;
        let mut frames = 1;
        let mut from = 0.0;
        let mut span = f64::INFINITY;
        let mut plant = None;
        let mut dump = None;
        for arg in args {
            match arg.split_once('=') {
                Some(("seam", value)) => {
                    if value == "file" || value == "factory" {
                        return Err(USAGE_SEAM.into());
                    }
                    seam = Some(seam_fit(value)?);
                }
                Some(("every", value)) => every = value.parse()?,
                Some(("frames", value)) => frames = value.parse()?,
                Some(("from", value)) => from = value.parse()?,
                Some(("span", value)) => span = value.parse()?,
                Some(("plant", value)) => plant = Some(seam_fit(value)?),
                Some(("patches", value)) => probe.patches = value.parse()?,
                Some(("keep", value)) => probe.keep = value.parse()?,
                Some(("dump", value)) => dump = Some(PathBuf::from(value)),
                Some(_) => return Err(format!("{USAGE}\n\nunknown: {arg}").into()),
                None => input = Some(PathBuf::from(arg)),
            }
        }
        Ok(Self {
            input: input.ok_or(USAGE)?,
            seam: seam.ok_or(USAGE_SEAM)?,
            probe,
            every,
            frames,
            plant,
            from,
            span,
            dump,
        })
    }
}

const USAGE: &str = "usage: settle <file.insv> seam=roll:0.8,yaw:-2.3,pitch:-0.9,cx:-3.3,cy:-11.9 \
[every=seconds] [frames=n] [from=seconds] [span=seconds] [patches=n] [keep=r] \
[plant=roll:0.5,...] [dump=path.csv]";

const USAGE_SEAM: &str = "this instrument needs one stored pose: a fit off this capture's own \
frames absorbs this scene into the pose, and a convergence measured against it would be measured \
against the scene. seam=roll:..,yaw:..,pitch:..,cx:..,cy:..";
