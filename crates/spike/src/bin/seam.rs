//! What the seam is misaligned by on a camera that is not moving, what shape
//! of calibration error would explain it, and what correcting it is worth
//! (issue #48).
//!
//! ```sh
//! # the residual round the seam circle, its structure, and the fit
//! cargo run --release -p kjerag-spike --bin seam -- <static.insv> both=1 \
//!   knobs=roll,yaw,pitch,cx,cy control=1 also=<other-static.insv>
//! # a fitted correction, applied and looked at with no blend to hide it
//! cargo run --release -p kjerag-spike --bin seam -- <file.insv> mode=render \
//!   yaw=90 bands=0 fix=roll:0.80,yaw:-2.29,pitch:-0.82,cx:-4.59,cy:-14.73
//! # what a narrower blend buys, and what it costs
//! cargo run --release -p kjerag-spike --bin seam -- <file.insv> mode=blend \
//!   yaw=90 pitch=-60 bands=14,8,4,2,1,0.5,0
//! # our stitch against the camera maker's own, on the same capture
//! cargo run --release -p kjerag-spike --bin seam -- <file.insv> mode=parity \
//!   against=<their-export.mp4>
//! # the same against a reframe whose projection they told us: Studio's
//! # "Distortion" slider is Panini's d, and their d=0 is exactly rectilinear
//! cargo run --release -p kjerag-spike --bin seam -- <file.insv> mode=parity \
//!   against=<their-export.mp4> panini=0.9 fov=150
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
//! the shipped map itself (`kjerag_render::Reframe`, the shader's own Rust
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

use kjerag_media::Fallible;
use kjerag_meta::{CalibrationSet, Filter, Lens, Mat3, Quat, Size as MetaSize};
use kjerag_render::seam::{
    self, Found, Knob, Probe, Reading, Refused, Where, least_squares, mapped, moved, read_ring,
    read_ring_centred, ring, rms, turned, unit,
};
use kjerag_render::{Camera, Held, Landing, Reframe, Sampling, Size};
use kjerag_spike::{Pair, Walk};

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    match options.mode {
        Mode::Residual => residual(&options),
        Mode::Render => render(&options),
        Mode::Blend => blend(&options),
        Mode::Parity => parity(&options),
        Mode::Fit => fit(&options),
        Mode::Solve => solve(&options),
        Mode::Register => register(&options),
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
    /// What the shipped per-file fit reads on this file, and what it costs:
    /// the app's own path (issue #48 phase 2), timed and printed.
    Fit,
    /// The inverse solve: which calibration would make our picture land where
    /// a given reframed picture's does. Run it against a PLANT before ever
    /// running it against an export (docs/research/parity-protocol.md).
    Solve,
    /// Where an export is pointed and how wide it is, found without believing
    /// a single one of the stitcher's own labels. What `mode=solve` has to be
    /// started from, and the thing section 8 of the protocol called the
    /// blocker.
    Register,
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
        let ring = options.ring();
        let lenses = options.corrected(std::slice::from_ref(&path), &calibration.lenses, frame);
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
    let ring = options.ring();

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
    knob_counts(&pooled, &lenses, frame);
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
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let mut walk = Walk::open(path, options.from, frame)?;
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
    let mut centre = None;
    for _ in 0..options.count {
        let Some(pair) = walk.next_pair()? else {
            break;
        };
        // Where along the seam this ring's own picture sits, which is where
        // the search is centred (issue #130). The camera's answer and not the
        // frame's, so it is asked for once and held and every frame of the run
        // is read the same way; nought is the search where the calibration
        // puts it.
        if centre.is_none()
            && let Some((_, reframe)) = candidates.first()
        {
            centre = seam::acquired(reframe, &pair.lenses, ring, &options.probe());
            if let Some(centre) = centre {
                println!(
                    "gross:  the along-seam search is centred {:+.2} deg from the calibration's \
                     own answer",
                    centre as f64 * options.step,
                );
            }
        }
        let found: Vec<Vec<Option<Found>>> = candidates
            .iter()
            .map(|(_, reframe)| {
                read_ring_centred(
                    reframe,
                    &pair.lenses,
                    ring,
                    &options.probe(),
                    centre.unwrap_or(0),
                    &mut refused,
                )
            })
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

/// The shipped fit, on the frames the app itself would read.
///
/// Everything the app does at open: the same places in the file, the same
/// patches, the same iterated fit. What it adds is the clock and the second
/// knob set, because what the app cannot report is how long it took and what
/// the fit it did not run would have said.
fn fit(options: &Options) -> Fallible<()> {
    for path in options.inputs() {
        let calibration = CalibrationSet::from_insv(&path)?;
        let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
        let lenses = fixed(&calibration.lenses, &options.fix);
        let plan = seam::Plan {
            probe: options.probe(),
            ..seam::Plan::default()
        };
        let started = std::time::Instant::now();
        let readings = seam::measure(std::slice::from_ref(&path), &lenses, frame, &plan)?;
        let read = started.elapsed().as_secs_f64();
        println!(
            "\n{}: {} of {} azimuths correlated over {} places x {} frames, read in {:.2} s",
            path.file_name().unwrap_or_default().to_string_lossy(),
            readings.len(),
            plan.probe.patches,
            plan.places,
            plan.frames,
            read,
        );
        let readings: Vec<Reading> = readings;
        let solved = std::time::Instant::now();
        let fitted = seam::fit_held(&readings, &lenses, frame, &seam::KNOBS, seam::RIDGE);
        println!(
            "the fit itself took {:.3} s, so the whole of it is {:.2} s",
            solved.elapsed().as_secs_f64(),
            read + solved.elapsed().as_secs_f64(),
        );
        if fitted.is_none() {
            println!("no fit: these patches do not separate the knobs");
            continue;
        }
        let held: Vec<Patch> = readings
            .iter()
            .map(|reading| Patch {
                at: reading.at,
                along: vec![reading.along],
                across: vec![reading.across],
                r: Vec::new(),
                contrast: 0.0,
                below: 0.0,
            })
            .collect();
        knob_counts(&held.iter().collect::<Vec<_>>(), &lenses, frame);
    }
    Ok(())
}

/// The shipped fitter's own answer on these patches, a rotation against the
/// shipped five knobs: the decision 6.8 took, re-taken here through the code
/// that ships rather than beside it.
///
/// Every row is the same iterated fit on the same readings, so the only
/// difference between them is which knobs turn and whether the principal
/// point is held ([`seam::RIDGE`]). `along` and `across` are the root mean
/// square of the patch readings before the correction and after it, predicted
/// through the map; the honest after is a second decode with the correction in
/// place, which is what `fit=1` draws and what a residual run of this
/// instrument re-measures.
fn knob_counts(kept: &[&Patch], lenses: &[Lens], frame: Size) {
    let readings: Vec<Reading> = kept
        .iter()
        .map(|patch| Reading {
            at: patch.at,
            along: patch.mean_along(),
            across: patch.mean_across(),
        })
        .collect();
    println!("\nthe shipped fit on these patches, and a rotation beside it:");
    println!(
        "{:<10} {:>8} {:>8} {:>8} {:>8} {:>8} {:>16} {:>16}",
        "knobs", "roll", "yaw", "pitch", "cx", "cy", "along", "across"
    );
    let rotation = [Knob::Roll, Knob::Yaw, Knob::Pitch];
    for (name, knobs, ridge) in [
        ("rotation", &rotation[..], 0.0),
        ("five", &seam::KNOBS[..], 0.0),
        ("shipped", &seam::KNOBS[..], seam::RIDGE),
    ] {
        let Some(fitted) = seam::fit_held(&readings, lenses, frame, knobs, ridge) else {
            println!("{name:<10} singular on these patches");
            continue;
        };
        println!(
            "{name:<10} {:>8.3} {:>8.3} {:>8.3} {:>8.2} {:>8.2} {:>7.3} -> {:>5.3} \
             {:>7.3} -> {:>5.3}",
            fitted.fit.roll_deg,
            fitted.fit.yaw_deg,
            fitted.fit.pitch_deg,
            fitted.fit.cx_px,
            fitted.fit.cy_px,
            fitted.before[0],
            fitted.after[0],
            fitted.before[1],
            fitted.after[1],
        );
    }
}

// ------------------------------------------------------------ plumbing

struct Options {
    mode: Mode,
    input: PathBuf,
    /// A second capture of a different scene, measured on the same ring. It is
    /// the control that separates the camera from what it is looking at.
    also: Option<PathBuf>,
    /// Insta360 Studio's "Distortion" slider for `mode=parity`, which is
    /// Panini's `d`, or negative for an export whose projection is unknown and
    /// has to be fitted. Given one, `fov=` is taken as given too.
    panini: f64,
    /// The camera maker's own export of the same capture, for `mode=parity`.
    against: Option<PathBuf>,
    /// What the shipped per-frame band settled on, written out by
    /// `kjerag-spike --bin band save=` (issue #103). Empty is the picture
    /// before stage 2, and `mode=parity` draws both.
    band: Vec<kjerag_render::Cell>,
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
    /// How far off the seam circle the whole ring is tilted before anything is
    /// read, in degrees towards the front lens (issue #103, stage 6).
    ///
    /// Zero is the seam itself, which is where every earlier reading of this
    /// residual was taken and where the band pass measures. It is an argument
    /// because the along-seam residual turned out **not to be constant across
    /// the seam**: a horizon fitted from content a few degrees either side
    /// shows more misregistration than the correlation at the seam reports,
    /// and this is what says whether that is the trace or the optics.
    off: f64,
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
    /// Run the shipped per-file fit and draw with it, which is what the app
    /// does at open (issue #48 phase 2).
    fit: bool,
    yaw: f64,
    pitch: f64,
    fov: f64,
    size: u32,
    /// The blend widths compared, in degrees, plus the shipped weights.
    bands: Vec<f64>,
    out: Option<PathBuf>,

    // ---- mode=solve. Its own block because every one of them is about a
    // picture drawn in somebody else's projection rather than about the ring.
    /// The view's three angles in OUR frame, which is the frame the solve
    /// answers in. Studio's pan/tilt is not this and is not converted:
    /// `viewoff=` starts the solve away from here and the solve walks back.
    view: [f64; 3],
    /// The export's shape, so a plant with no export to read still draws the
    /// picture the export would have been.
    aspect: f64,
    /// A known perturbation of lens 1, injected into OUR OWN render to make
    /// the target. The only honest way to know what the solver can recover is
    /// to give it an answer nobody can argue with.
    plant: Option<kjerag_render::SeamFit>,
    /// Where the solve starts from, which is not where the plant is.
    start: kjerag_render::SeamFit,
    /// How far the solve's starting view is from the true one, in degrees.
    viewoff: [f64; 3],
    /// The same for the view's field of view, which is the knob Studio's own
    /// number does not give us.
    fovoff: f64,
    /// How many sites across the picture, how big a patch is, and how far the
    /// correlation looks, all in pixels of the working shape.
    ///
    /// `search` is not a taste: the first round has to reach the whole of the
    /// starting error, and a site whose true shift is past the search is
    /// dropped rather than found. Measured 2026-08-08 at 60 degrees on a 640
    /// wide picture, where one degree is 10.7 px: at `search=12` a start 1.2
    /// deg and 3 deg of scale off kept 50 of 500 sites and the run refused,
    /// and at `search=30` the same start solved to 0.013 px. If the aim is
    /// uncertain by D degrees, `search` must exceed D times the picture's
    /// pixels per degree.
    sites: usize,
    patch: usize,
    search: usize,
    rounds: usize,
    /// The correlation a site has to reach to be kept.
    floor: f64,
    /// `source_time = export_time + lag`, from scripts/research/pair.py.
    lag: f64,
    /// Report the registration gradient instead of solving: how the agreement
    /// falls away as the view is deliberately mis-aimed. The control that says
    /// a registration score is a score and not a constant.
    aim: bool,
    // ---- mode=register. The search that has to run BEFORE a solve, because
    // the solve is local and the export arrives with nothing but a maker's
    // label on it.
    /// Every source instant the registration is run at, in seconds of the
    /// source file's own clock. Three or more is the point: one instant's
    /// answer is a coincidence until another instant says the same thing.
    instants: Vec<f64>,
    /// The Panini `d` values swept, or empty for "whatever `panini=` said".
    ///
    /// Their "Distortion" slider is a label like every other label here, so a
    /// told `d` is a place to start a sweep and not a number to believe.
    dset: Vec<f64>,
    /// The scale window, as OUR family's full field of view in degrees. It is
    /// absolute and not a factor on their label on purpose: a window centred
    /// on a number nobody trusts is a window that trusts it.
    fovlo: f64,
    /// The top of that window, or zero for "as far as the projection itself
    /// goes" -- Panini's own pole is at `lambda = acos(-d)`, so `d = 0.9`
    /// cannot draw more than 308 degrees whatever a search asks for.
    fovhi: f64,
    /// The three working widths of the cascade: the sphere sweep, the middle
    /// refinement, and the final polish (`size=`).
    coarse: u32,
    mid: u32,
    /// How many coarse candidates are carried into the refinement, and how far
    /// apart they are made to sit first.
    keepn: usize,
    /// The high pass, as a fraction of the working picture's width.
    ///
    /// This is the whole difference between a peak and the flat 0.7 the
    /// protocol's section 8 recorded. A raw correlation between our picture
    /// and theirs is dominated by the biggest thing in both of them, which is
    /// the sky/ground brightness ramp, and that ramp is there whatever the aim
    /// is: it scores 0.73 at the pose and 0.74 half a degree off it. Taking a
    /// local mean out leaves the structure, which is the only part of a
    /// picture that knows where it is.
    hp: f64,
    /// How many of the sweep's aims are carried into the densification.
    ///
    /// Deep, and the depth is measured rather than chosen. The sweep's own
    /// ranking is a filter and not a judgement: at 16 pixels across it scores
    /// the export's own answer at about 0.45 when the nearest grid point sits
    /// a degree off it, against a sweep maximum of 0.633 and a 99.9th
    /// percentile of 0.485 -- so the right answer sits around the 99.8th
    /// percentile of two million aims, which is rank four thousand. A pool cut
    /// at a hundred, or at a thousand, does not contain it, and every run that
    /// cut it there reported the export's second basin instead. The pool is
    /// cut where the answer is, and the 48 pixel densification below is what
    /// turns the pool back into a ranking.
    pool: usize,
    /// How wide the picture is at the SPHERE SWEEP, which is not the same as
    /// `coarse` and is the number that makes the sweep possible at all.
    ///
    /// A correlation between two band-passed pictures falls off over about ONE
    /// PIXEL OF THE WORKING PICTURE, whatever that pixel is worth in degrees.
    /// So a sweep at `W` pixels across a view `F` degrees wide can only find
    /// what it steps within about `F/W` of, and a grid stepping `F/8` needs
    /// `W` no larger than 16. At 48 it needs `F/24`, which is fourteen times
    /// the grid. Measured at 360 s on the creek: at 48 pixels the export's own
    /// answer scores 0.92 where it stands and under 0.65 at the nearest point
    /// a fifth-of-a-view grid visited, which is not enough to outrank a wrong
    /// basin that landed on a grid point -- and every run built on that sweep
    /// reported the wrong basin.
    sweepw: u32,
    /// The high pass used by the SWEEP, which is a different number from the
    /// one the refinement uses and for a reason that is the whole of
    /// coarse-to-fine.
    ///
    /// A sphere grid stepped by a fifth of the view can only be as good as the
    /// objective's own capture radius, and that radius is set by the width of
    /// the structure left after the high pass: measured at 360 s on the creek,
    /// with the pass at an eighth of the width, the export's own answer scores
    /// 0.9312 where it stands and 0.469 two degrees away, so the grid point
    /// nearest the answer does not outrank a wrong basin that happened to land
    /// on a grid point. A wider pass at the sweep leaves broader structure,
    /// which is blunter and reaches further -- which is what a coarse stage is
    /// for.
    hpc: f64,
    /// How coarsely the scale window is stepped, as a ratio between one swept
    /// field of view and the next. The refinement is continuous, so this only
    /// has to be fine enough that some sampled scale lands inside the right
    /// basin.
    ratio: f64,
    /// How much local contrast a pixel of THEIR picture has to carry before
    /// the correlation is allowed to score it, in luma codes.
    ///
    /// Not a taste. Outdoors, most of a frame is sky; sky with its local mean
    /// taken out is sensor noise; and a correlation that scores noise against
    /// noise gives every aim the same nothing over half the picture, which is
    /// exactly the dilution that let a rival 64 degrees away come within 0.005
    /// of the answer.
    texture: f64,
    /// Studio's own pan, tilt and roll for this export, in Studio's numbers.
    ///
    /// Given these, the search stops being four dimensional. Their reframe is
    /// direction locked, so their view is fixed in the WORLD; the file's own
    /// orientation track says where the body was at each instant; and the only
    /// thing left unknown between their frame and ours is **one angle**, the
    /// heading datum the IMU integration started from. That is a 1-D sweep of
    /// a few hundred scores instead of a 4-D one of two million, and it is the
    /// difference between a search that is information-limited on a 20 degree
    /// view and one that is not.
    ///
    /// Their tilt and roll are used as given because they were MEASURED to be
    /// ours: on the July-14 exports a told tilt of -90 came back as a world
    /// pitch of -88.4 and a told 3.5 as +5.4, and a told roll of 0 as -0.6.
    /// The scale is not taken on trust at all -- it is swept over the same
    /// generous window the label-free search uses, and the refinement is free
    /// to move all five numbers afterwards.
    told: Vec<f64>,
    /// Start the cascade from a known aim instead of sweeping the sphere:
    /// `yaw,pitch,roll,fov,d`.
    ///
    /// Not a shortcut for the answer -- a seeded run reports no prominence,
    /// because prominence is a statement about a search and a seeded run has
    /// not made one. It is here so the objective's own settings can be argued
    /// about in seconds rather than in minutes, and so a pose found at one
    /// instant can be scored at another.
    seed: Vec<f64>,
    /// Whether the found aim is turned back into the world frame with the
    /// file's own IMU before it is compared across instants.
    ///
    /// It has to be. Studio's reframe is gyro-stabilized, so their view is
    /// fixed in the WORLD and ours is solved in the camera BODY's frame: the
    /// same export registers at a different body aim on every instant, by
    /// exactly the camera's own motion, and comparing the body angles across
    /// instants would report the flight rather than the export.
    world: bool,
    /// Which frame a told pan/tilt/roll is told IN, which is exactly what
    /// Studio's Direction Lock checkbox decides.
    ///
    /// **This is not a preference and it is not read off the checkbox.** It
    /// picks the model the `told=` sweep builds its aims with, and the two
    /// models are different geometry:
    ///
    /// - `lock=world` — DIRECTION LOCK ON. Their view is fixed in the world,
    ///   so the body aim is `body(t)^-1 * world(pan + datum, tilt, roll)` and
    ///   is different at every instant by exactly the flight. This is what the
    ///   July-14 exports were and what section 2 of the protocol describes.
    /// - `lock=body` — DIRECTION LOCK OFF. Their view is fixed in the camera
    ///   body, so the aim is `orientation(pan + datum, tilt, roll)` with no
    ///   IMU in it at all, and it is the SAME three numbers at every instant.
    ///   Section 5 item 5 is the request that produced this, and the thing
    ///   that confirms it is that the body aims agree across instants.
    ///
    /// Run BOTH on a new export and let the score say which the export is.
    /// A checkbox is a label like every other label here.
    lock: Lock,
    /// Tikhonov damping, in output pixels of cost per [`KNOB_STEPS`] of step.
    ///
    /// It exists for one reason and it is not conditioning-in-general: a view
    /// pointing straight down is a gimbal lock, where the view's own yaw and
    /// its roll are the SAME rotation, and the normal equations there are
    /// exactly singular. Damping lets the well-determined directions answer
    /// and leaves the null one where it started, instead of refusing the run.
    /// It is small against the noise floor on purpose, so a direction the data
    /// does constrain is not pulled by it.
    damp: f64,
    /// Which of the nine numbers `mode=solve` is allowed to move.
    ///
    /// Default is all of them, which is every run this file has ever made.
    /// It exists for **the protocol's match criterion (b)**, which asks for
    /// the same measurement made with the SHIPPED FACTORY calibration in place
    /// of the solved one and wants the solved one to beat it by three times.
    /// That comparison is only fair if the view is still fitted in both — a
    /// factory run whose view is also wrong would be beaten by its own aim
    /// error rather than by its calibration — so the factory arm is
    /// `free=view`: the four view numbers move and lens 1's five stay exactly
    /// where the file's own `offset_v3` put them.
    free: Vec<String>,
    /// The geometries a joint solve fits at once, each
    /// `label@<export.mp4|plant>@<from>@<yaw>,<pitch>,<roll>@<fov>`.
    ///
    /// Empty is the single-geometry solve every earlier run made, built out of
    /// `against=`, `from=`, `view=` and `fov=`.
    arms: Vec<ArmSpec>,
    /// A known radial perturbation injected into OUR OWN render to make the
    /// target, as ten mode amplitudes: five for lens 0, a colon, five for
    /// lens 1.
    plantradial: Option<[[f64; RADIAL_ORDERS]; 2]>,
    /// Where the radial delta starts, and where it STAYS on a run that does
    /// not free it. This is what a cross-validated prediction is made with.
    startradial: Option<[[f64; RADIAL_ORDERS]; 2]>,
    /// Whether our render corrects the ROLLING SHUTTER with the file's own
    /// IMU, the way the shipped pass does (issue #9).
    ///
    /// Off by default, which is what every solve before it was written did, so
    /// a run with it off is the run that was there before. It exists because
    /// the residual against a real export turned out to be dominated by
    /// site-to-site SCATTER rather than by anything smooth, and a readout is
    /// the one thing in this pipeline that displaces content by tens of pixels
    /// in a pattern that is smooth in the SENSOR's rows and scattered in
    /// everything this instrument bins by. The turn across one readout is
    /// printed for every arm whether or not this is on, because it is a
    /// property of the flight and not of the correction.
    rolling: bool,
    /// How much the difference picture is amplified about mid grey.
    ///
    /// Section 6 of the protocol says 4x and the owner asked for 8x on the
    /// real run, so it is an argument and the number is written into the file
    /// name. A difference picture with no amplification stated is a picture
    /// nobody can read a size off.
    amp: f64,
}

/// Which frame Studio's told pan/tilt/roll is told in. See [`Options::lock`].
#[derive(Clone, Copy, PartialEq)]
enum Lock {
    /// Direction Lock ON: the view is fixed in the world.
    World,
    /// Direction Lock OFF: the view is fixed in the camera body.
    Body,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Fallible<Self> {
        let input = PathBuf::from(args.next().ok_or(USAGE)?);
        let mut options = Self {
            mode: Mode::Residual,
            input,
            also: None,
            panini: -1.0,
            against: None,
            band: Vec::new(),
            from: 0.0,
            count: 6,
            patches: 72,
            span: 3.7,
            step: 0.08,
            along: 2.0,
            across: 4.0,
            off: 0.0,
            keep: 0.80,
            contrast: 6.0,
            knobs: vec![Knob::Roll, Knob::Cx, Knob::Cy],
            both: false,
            control: false,
            fix: Vec::new(),
            fit: false,
            yaw: 90.0,
            pitch: 0.0,
            fov: 50.0,
            size: 1024,
            bands: vec![14.0, 8.0, 4.0, 2.0, 1.0, 0.0],
            out: None,
            view: [0.0; 3],
            aspect: 16.0 / 9.0,
            plant: None,
            start: kjerag_render::SeamFit::default(),
            viewoff: [0.0; 3],
            fovoff: 0.0,
            sites: 40,
            patch: 15,
            search: 30,
            rounds: 5,
            floor: 0.5,
            lag: 0.0,
            damp: 0.002,
            aim: false,
            instants: Vec::new(),
            dset: Vec::new(),
            fovlo: 12.0,
            fovhi: 0.0,
            coarse: 48,
            mid: 192,
            keepn: 24,
            hp: 0.125,
            hpc: 0.3,
            sweepw: 16,
            pool: 6000,
            ratio: 1.15,
            texture: 8.0,
            seed: Vec::new(),
            told: Vec::new(),
            world: true,
            lock: Lock::World,
            rolling: false,
            free: vec!["all".to_owned()],
            arms: Vec::new(),
            plantradial: None,
            startradial: None,
            amp: 4.0,
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
                        "fit" => Mode::Fit,
                        "solve" => Mode::Solve,
                        "register" => Mode::Register,
                        _ => return Err(format!("no mode called {value}. {USAGE}").into()),
                    };
                }
                "also" => options.also = Some(PathBuf::from(value)),
                "against" => options.against = Some(PathBuf::from(value)),
                "band" => {
                    options.band = kjerag_render::Cell::read(&std::fs::read_to_string(value)?)
                        .ok_or("that is not a band state written by --bin band")?;
                }
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
                "view" => options.view = triple(value)?,
                "viewoff" => options.viewoff = triple(value)?,
                "fovoff" => options.fovoff = value.parse()?,
                "aspect" => options.aspect = value.parse()?,
                "plant" => options.plant = Some(kjerag_spike::seam_fit(value)?),
                "start" => options.start = kjerag_spike::seam_fit(value)?,
                "sites" => options.sites = value.parse()?,
                "patch" => options.patch = value.parse()?,
                "search" => options.search = value.parse()?,
                "rounds" => options.rounds = value.parse()?,
                "floor" => options.floor = value.parse()?,
                "lag" => options.lag = value.parse()?,
                "damp" => options.damp = value.parse()?,
                "aim" => options.aim = value.parse::<u32>()? != 0,
                "rolling" => options.rolling = value.parse::<u32>()? != 0,
                "instants" => {
                    options.instants = value
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<Vec<f64>, _>>()?;
                }
                "dset" => {
                    options.dset = value
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<Vec<f64>, _>>()?;
                }
                "fovlo" => options.fovlo = value.parse()?,
                "fovhi" => options.fovhi = value.parse()?,
                "coarse" => options.coarse = value.parse()?,
                "mid" => options.mid = value.parse()?,
                "keepn" => options.keepn = value.parse()?,
                "hp" => options.hp = value.parse()?,
                "hpc" => options.hpc = value.parse()?,
                "sweepw" => options.sweepw = value.parse()?,
                "pool" => options.pool = value.parse()?,
                "ratio" => options.ratio = value.parse()?,
                "told" => {
                    options.told = value
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<Vec<f64>, _>>()?;
                }
                "seed" => {
                    options.seed = value
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<Vec<f64>, _>>()?;
                }
                "texture" => options.texture = value.parse()?,
                "world" => options.world = value.parse::<u32>()? != 0,
                "amp" => options.amp = value.parse()?,
                "lock" => {
                    options.lock = match value {
                        "world" | "on" => Lock::World,
                        "body" | "off" => Lock::Body,
                        _ => return Err(format!("lock is world or body, not {value}").into()),
                    };
                }
                "free" => {
                    options.free = value.split(',').map(str::to_owned).collect();
                }
                "arm" => options.arms.push(arm_spec(value)?),
                "plantradial" => options.plantradial = Some(radial_pair(value)?),
                "startradial" => options.startradial = Some(radial_pair(value)?),
                "fit" => options.fit = value.parse::<u32>()? != 0,
                "panini" => options.panini = value.parse()?,
                "from" => options.from = value.parse()?,
                "count" => options.count = value.parse()?,
                "patches" => options.patches = value.parse()?,
                "span" => options.span = value.parse()?,
                "step" => options.step = value.parse()?,
                "along" => options.along = value.parse()?,
                "across" => options.across = value.parse()?,
                "off" => options.off = value.parse()?,
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

    /// The directions this run reads, which is the seam circle itself unless
    /// [`Self::off`] tilts it (issue #103, stage 6).
    ///
    /// A tilt turns the whole local frame about the along-seam axis, so the
    /// triad stays orthonormal and the `along` column keeps meaning the same
    /// thing: degrees of world angle along the seam's own tangent. What moves
    /// is only which content is being asked about.
    fn ring(&self) -> Vec<Where> {
        let (sin, cos) = self.off.to_radians().sin_cos();
        ring(self.patches)
            .into_iter()
            .map(|at| Where {
                phi: at.phi,
                centre: std::array::from_fn(|c| at.centre[c] * cos + at.across[c] * sin),
                along: at.along,
                across: std::array::from_fn(|c| at.across[c] * cos - at.centre[c] * sin),
            })
            .collect()
    }

    /// How the seam is read, which is the shipped fitter's own knobs with
    /// this run's numbers in them.
    fn probe(&self) -> Probe {
        Probe {
            patches: self.patches,
            span: self.span,
            step: self.step,
            along: self.along,
            across: self.across,
            keep: self.keep,
            contrast: self.contrast,
        }
    }

    /// The correction to draw with: whatever `fix=` says, and with `fit=1`
    /// **the fit off this file's own frames**, run here exactly as the app
    /// runs it for a camera it has no stored calibration for. What the eye is
    /// checking is then what the player draws on that path; `fix=` is how a
    /// stored calibration is checked, because the store belongs to the app.
    fn corrected(&self, files: &[PathBuf], lenses: &[Lens], frame: Size) -> Vec<Lens> {
        let lenses = fixed(lenses, &self.fix);
        if !self.fit {
            return lenses;
        }
        let started = std::time::Instant::now();
        let Some(fitted) = seam::fit_reported(files, &lenses, frame, &seam::Plan::default()) else {
            println!("fit:    no fit; the factory calibration stands");
            return lenses;
        };
        println!(
            "fit:    roll {:+.3}, yaw {:+.3}, pitch {:+.3} deg, cx {:+.2}, cy {:+.2} px \
             over {} patches in {:.1} s\n\
             fit:    along {:.3} -> {:.3}, across {:.3} -> {:.3} deg (predicted)",
            fitted.fit.roll_deg,
            fitted.fit.yaw_deg,
            fitted.fit.pitch_deg,
            fitted.fit.cx_px,
            fitted.fit.cy_px,
            fitted.patches,
            started.elapsed().as_secs_f64(),
            fitted.before[0],
            fitted.after[0],
            fitted.before[1],
            fitted.after[1],
        );
        fitted.fit.applied(&lenses)
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

/// `yaw,pitch,roll` in degrees, which is the order [`Look`] applies them in.
/// One geometry of a joint solve, as one argument:
/// `label@<export.mp4|plant>@<from>@<yaw>,<pitch>,<roll>@<fov>`.
///
/// `plant` in the second slot is an arm whose target is one of our own renders
/// rather than an export, which is what the plant and the null gates are.
fn arm_spec(value: &str) -> Fallible<ArmSpec> {
    let parts: Vec<&str> = value.split('@').collect();
    let [label, against, from, view, fov] = parts.as_slice() else {
        return Err(format!(
            "an arm is label@<export.mp4|plant>@<from>@<yaw>,<pitch>,<roll>@<fov>, not {value}"
        )
        .into());
    };
    Ok(ArmSpec {
        label: (*label).to_owned(),
        against: match *against {
            "plant" | "-" => None,
            path => Some(PathBuf::from(path)),
        },
        from: from.parse()?,
        view: triple(view)?,
        fov: fov.parse()?,
    })
}

/// Ten radial mode amplitudes: five for lens 0, a colon, five for lens 1.
fn radial_pair(value: &str) -> Fallible<[[f64; RADIAL_ORDERS]; 2]> {
    let mut out = [[0.0; RADIAL_ORDERS]; 2];
    let halves: Vec<&str> = value.split(':').collect();
    if halves.len() != 2 {
        return Err(format!("a radial pair is <five>:<five>, not {value}").into());
    }
    for (lens, half) in halves.iter().enumerate() {
        let numbers: Vec<f64> = half
            .split(',')
            .map(str::parse)
            .collect::<Result<_, _>>()
            .map_err(|error| format!("{error} in {half}"))?;
        if numbers.len() != RADIAL_ORDERS {
            return Err(format!("a lens wants {RADIAL_ORDERS} amplitudes, got {half}").into());
        }
        out[lens].copy_from_slice(&numbers);
    }
    Ok(out)
}

fn triple(value: &str) -> Fallible<[f64; 3]> {
    let parts: Vec<f64> = value.split(',').map(str::parse).collect::<Result<_, _>>()?;
    match parts.len() {
        3 => Ok([parts[0], parts[1], parts[2]]),
        _ => Err("a view is yaw,pitch,roll in degrees".into()),
    }
}

const USAGE: &str = "usage: seam <file.insv> [mode=residual|render|blend|parity|fit|solve|register] [also=<other.insv>] \
     [fix=roll:0.8,yaw:-2] [fit=1] [yaw=deg] [pitch=deg] [fov=deg] [size=px] [bands=14,8,4] [out=x.png] \
     [from=seconds] [count=frames] [patches=n] [panini=d] \
     [span=deg] [step=deg] [along=deg] [across=deg] [off=deg] [keep=r] [contrast=codes] \
     [knobs=roll,cx,cy,...] [control=1] \
     mode=solve: [view=yaw,pitch,roll] [viewoff=yaw,pitch,roll] [aspect=1.7778] \
     [plant=cx:8,pitch:0.2 | against=<export.mp4> lag=seconds] [start=roll:0,...] \
     [sites=n] [patch=px] [search=px] [rounds=n] [floor=r] \
     [arm=label@<export.mp4|plant>@from@yaw,pitch,roll@fov ...] [free=all|calib|view|lens1|radial|radial0|radial1] \
     [plantradial=a,b,c,d,e:a,b,c,d,e] [startradial=a,b,c,d,e:a,b,c,d,e] \
     mode=register: against=<export.mp4> [instants=30,60,90] [lag=seconds] [panini=d] [dset=0,0.45,0.9] \
     [fovlo=12] [fovhi=0] [coarse=48] [mid=192] [size=480] [keepn=24] [hp=0.125] [hpc=0.3] [sweepw=16] [pool=6000] [ratio=1.15] [texture=8] [seed=yaw,pitch,roll,fov,d] [told=pan,tilt,roll] [world=1]";

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

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|axis| a[axis] * b[axis]).sum()
}

fn norm(v: [f64; 3]) -> f64 {
    dot(v, v).sqrt()
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
    /// The two lenses' shares of one ray, and where each of them reads it,
    /// with **no per-frame bend at all**.
    ///
    /// Empty cells, and that is the whole of what `mode=blend` measures - for
    /// [`Self::Shipped`] as much as for the synthetic [`Self::Band`]. It is the
    /// right picture for what that mode compares, which is one weighting
    /// against another over the same content, and it is the wrong instrument
    /// for anything near field: past 10 m the bend is a fraction of a degree,
    /// but the pilot's own gear on the seam reads 1.9 to 2.3 degrees of
    /// disparity and the bend is the mechanism that carries it. Score a
    /// near-field view with `--bin band mode=render`, which draws the real
    /// pass with the band pass live (2026-08-06, after a near-field
    /// measurement taken here understated the width's cost).
    fn at(self, reframe: &Reframe, ray: [f64; 3]) -> ([f64; 2], [Landing; 2]) {
        self.bent(reframe, ray, &[])
    }

    /// The same with the per-frame band's own correction in it (issue #103).
    ///
    /// `cells` is the state the shipped compute pass settled on, written out
    /// by `kjerag-spike --bin band save=`. Empty is the picture before stage 2,
    /// which is what a before-and-after needs one of.
    ///
    /// It has to come in as a table rather than being measured here because
    /// the camera maker's own export is in a projection family the app's pass
    /// does not draw, so the comparison is a CPU render through
    /// `Reframe::blend_bent` and never goes near a window.
    fn bent(
        self,
        reframe: &Reframe,
        ray: [f64; 3],
        cells: &[kjerag_render::Cell],
    ) -> ([f64; 2], [Landing; 2]) {
        let ray32 = ray.map(|c| c as f32);
        let shipped = reframe.blend_bent(
            ray32,
            reframe.reading_at(ray32, cells, kjerag_render::Along::fit(cells)),
        );
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
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let mut walk = Walk::open(path, options.from, frame)?;
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
    let lenses = options.corrected(
        std::slice::from_ref(&options.input),
        &calibration.lenses,
        frame,
    );
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
///
/// **Every row here is drawn with the per-frame bend off** ([`Weighting::at`]),
/// the `shipped` row included, so this mode prices a weighting and not the
/// picture the pass draws near field. Far field that is the same thing to
/// within the bend's own size; near field it is not, and `--bin band
/// mode=render` is the instrument there.
fn blend(options: &Options) -> Fallible<()> {
    let (calibration, pair) = frame_at(options, &options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = options.corrected(
        std::slice::from_ref(&options.input),
        &calibration.lenses,
        frame,
    );
    let reframe = viewed(&lenses, frame, options.camera());
    let ring = options.ring();

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
    let found = read_ring(&body, &pair.lenses, &seen, &options.probe(), &mut refused);
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
    match disparities.is_empty() {
        // Which is not a disparity of zero, and the shear column below says
        // so rather than printing one: a correction that turns lens 1 by a
        // couple of degrees moves some of these patch rectangles out of one
        // lens's picture, and a rectangle that is not in both pictures cannot
        // be correlated at all.
        true => println!("seam:   no patch in this view correlated, so the disparity is unknown"),
        false => println!(
            "seam:   the two lenses disagree by {disparity:.2} deg here on average, {:.2} at worst",
            disparities.iter().copied().fold(0.0, f64::max),
        ),
    }
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
            match (doubled > 0.0, disparities.is_empty()) {
                (true, false) => format!("{:.2}", disparity / doubled),
                (true, true) => "-".to_owned(),
                (false, _) => "cut".to_owned(),
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
/// the body (`kjerag_render::projection`), so its transpose is what brings a
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
    let lenses = options.corrected(
        std::slice::from_ref(&options.input),
        &calibration.lenses,
        frame,
    );
    let export = export_frame(&theirs, options.from)?;
    println!(
        "theirs: {} at {:.2} s, {}x{}",
        theirs.file_name().unwrap_or_default().to_string_lossy(),
        options.from,
        export.shape.width,
        export.shape.height,
    );

    let mut best = (
        f64::MIN,
        Look {
            angles: [0.0; 3],
            fov: 90.0,
            compression: 1.0,
            panini: options.panini,
        },
    );
    // A view whose projection is known is searched on its angles alone, and the
    // grid is stepped by a fraction of what it can see: a 15 degree step finds
    // nothing in a 20 degree view.
    let told = options.panini >= 0.0;
    // A told projection still has its scale searched: Studio's own FOV number
    // is not the half angle this family measures in, and assuming it was put
    // the fit in the wrong basin (measured 2026-08-06: correlating 0.39).
    let fovs: Vec<f64> = match told {
        true => vec![
            options.fov,
            options.fov * 1.4,
            options.fov * 1.8,
            options.fov * 2.2,
        ],
        false => vec![60.0, 80.0, 100.0, 120.0],
    };
    let step = match told {
        true => (options.fov / 6.0).clamp(2.0, 15.0),
        false => 15.0,
    };
    let coarse = export.shape.scaled(48);
    let small = export.resampled(coarse);
    for yaw in (0..(360.0 / step) as i32).map(|n| f64::from(n) * step) {
        for pitch in (-(90.0 / step) as i32..=(90.0 / step) as i32).map(|n| f64::from(n) * step) {
            for roll in [-15.0, 0.0, 15.0] {
                for fov in fovs.iter().copied() {
                    let look = Look {
                        angles: [yaw, pitch, roll],
                        fov,
                        compression: 1.0,
                        panini: options.panini,
                    };
                    let score = agree(&looked(&lenses, frame, look, &ours, coarse, &[], None), &small);
                    if score > best.0 {
                        best = (score, look);
                    }
                }
            }
        }
    }
    // Pattern search from the coarse winner, at the resolution the answer is
    // wanted in.
    let fine = export.shape.scaled(200);
    let small = export.resampled(fine);
    let scored =
        |look: Look, pair: &Pair| agree(&looked(&lenses, frame, look, pair, fine, &[], None), &small);
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
        "fitted: yaw {:.2}, pitch {:.2}, roll {:.2}, fov {:.2} deg, {}, correlating {score:.4}",
        look.angles[0],
        look.angles[1],
        look.angles[2],
        look.fov,
        match look.panini >= 0.0 {
            true => format!("panini d {:.2} as told", look.panini),
            false => format!("compression {:.3} (1.000 is rectilinear)", look.compression),
        },
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
    // The view is fitted on the picture the band does NOT touch, and then both
    // of our pictures are drawn at it: the band moves a strip a couple of
    // degrees wide, and a fit free to follow it would score the strip it
    // chose.
    //
    // The two windows below, (0, 5) and (9, 25) degrees off the seam, were
    // drawn around a 2 degree handover. At the 8 the pass draws now, the band
    // plus its bend reaches 6.60, so the inner window stops short of the
    // corridor's outer 1.6 degrees while the outer window still starts clear
    // of it. The bias is one way: this understates a wide handover's cost
    // rather than inventing one.
    let stage1 = looked(&lenses, frame, look, &ours, export.shape, &[], None);
    let banded_picture = looked(&lenses, frame, look, &ours, export.shape, &options.band, None);
    let seam = seam_map(&lenses, frame, look, export.shape);
    println!(
        "\n{:<14} {:>13} {:>13} {:>9} {:>9} {:>9}",
        "picture", "in the band", "either side", "share", "band px", "side px"
    );
    let ours_rows: [(&str, &Vec<f64>); 3] = [
        ("ours, stage 1", &stage1),
        ("ours, band", &banded_picture),
        ("Insta360", &export.luma),
    ];
    for (name, luma) in ours_rows {
        let inside = banded(luma, &seam, export.shape, (0.0, 5.0));
        let outside = banded(luma, &seam, export.shape, (9.0, 25.0));
        println!(
            "{name:<14} {inside:>13.1} {outside:>13.1} {:>9.3} {:>9} {:>9}",
            match outside > 0.0 {
                true => inside / outside,
                false => 0.0,
            },
            counted(luma, &seam, export.shape, (0.0, 5.0)),
            counted(luma, &seam, export.shape, (9.0, 25.0)),
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

// ------------------------------------------------ the inverse solve

/// How many numbers a solve turns: the view's own three angles, then lens 1's
/// five.
///
/// The view's angles are in because Studio's pan/tilt frame is not ours and
/// nobody has derived the map between them. Solving them beside the
/// calibration is the honest alternative to assuming one, and it costs
/// nothing that matters: a view rotation moves BOTH lenses' content and a
/// lens 1 correction moves ONE lens's, so the two are separable by any set of
/// sites that covers both hemispheres. `mode=solve` prints the conditioning
/// that says whether the sites it actually kept did (`spread`, below), so a
/// run that lands in the degenerate corner says so instead of answering.
/// The radial delta's orders, which are `offset_v6`'s own: five.
///
/// `offset_v3` writes three (`k1 k2 k3`) and `offset_v6` writes five
/// (`k1..k5`), on the same normalized plane radius --
/// `docs/research/offset-v6.md` refuted the angle-polynomial reading before a
/// pixel was read. The form is borrowed from the file; the COEFFICIENTS here
/// are fitted from pixels and are not read off any token.
const RADIAL_ORDERS: usize = 5;

/// How many numbers belong to the CAMERA rather than to one view: lens 1's
/// five pose-and-principal-point knobs, then five radial modes on each lens.
///
/// A calibration is a property of the camera and a view is a property of a
/// frame, and this split is the whole of the protocol's section 11.2 B2: the
/// camera's numbers are fitted once over several geometries and then HELD
/// while another geometry is predicted with only its own view free.
const SHARED: usize = 5 + 2 * RADIAL_ORDERS;

/// How many belong to one view: its three angles and its field of view.
const PER_ARM: usize = 4;

const SHARED_NAMES: [&str; SHARED] = [
    "lens1 roll",
    "lens1 yaw",
    "lens1 pitch",
    "lens1 cx",
    "lens1 cy",
    "l0 rad1",
    "l0 rad2",
    "l0 rad3",
    "l0 rad4",
    "l0 rad5",
    "l1 rad1",
    "l1 rad2",
    "l1 rad3",
    "l1 rad4",
    "l1 rad5",
];
const SHARED_UNITS: [&str; SHARED] = [
    "deg", "deg", "deg", "px", "px", "rms", "rms", "rms", "rms", "rms", "rms", "rms", "rms", "rms",
    "rms",
];
/// The step each knob's Jacobian column is taken with. Big enough to clear the
/// map's own f32 arithmetic and small enough that the map is linear over it.
///
/// The radial modes are in units of the rms of the radial MULTIPLIER over the
/// declared window, so `2e-4` moves the landing about 0.3 lens px at the rim,
/// which is three thousand times the f32 resolution of a two-thousand-pixel
/// coordinate and a fortieth of the structure being chased.
const SHARED_STEPS: [f64; SHARED] = [
    0.05, 0.05, 0.05, 1.0, 1.0, 2e-4, 2e-4, 2e-4, 2e-4, 2e-4, 2e-4, 2e-4, 2e-4, 2e-4, 2e-4,
];
/// The most any one round may move each knob: a trust region, in each knob's
/// own units. See the note on the old `KNOB_CAPS`; a radial mode of 0.02 is a
/// two percent radial multiplier, which is ten times anything a real lens
/// calibration disagrees by.
const SHARED_CAPS: [f64; SHARED] = [
    1.0, 1.0, 1.0, 20.0, 20.0, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02, 0.02,
];

const ARM_NAMES: [&str; PER_ARM] = ["view yaw", "view pitch", "view roll", "view fov"];
const ARM_STEPS: [f64; PER_ARM] = [0.05, 0.05, 0.05, 0.5];
const ARM_CAPS: [f64; PER_ARM] = [3.0, 3.0, 3.0, 5.0];

/// How many numbers a run of `arms` geometries turns.
fn solved(arms: usize) -> usize {
    SHARED + PER_ARM * arms
}

/// Which knob a global column is, named for a person.
fn knob_name(labels: &[String], k: usize) -> String {
    match k < SHARED {
        true => SHARED_NAMES[k].to_owned(),
        false => format!(
            "{} {}",
            labels[(k - SHARED) / PER_ARM],
            ARM_NAMES[(k - SHARED) % PER_ARM]
        ),
    }
}

fn knob_unit(k: usize) -> &'static str {
    match k < SHARED {
        true => SHARED_UNITS[k],
        false => "deg",
    }
}

fn knob_step(k: usize) -> f64 {
    match k < SHARED {
        true => SHARED_STEPS[k],
        false => ARM_STEPS[(k - SHARED) % PER_ARM],
    }
}

fn knob_cap(k: usize) -> f64 {
    match k < SHARED {
        true => SHARED_CAPS[k],
        false => ARM_CAPS[(k - SHARED) % PER_ARM],
    }
}

/// The orthonormal radial modes one lens's delta is fitted in.
///
/// **Never five raw coefficients.** `r^2` to `r^10` over the narrow band of
/// field angle a seam view actually sees are so nearly parallel that the five
/// raw numbers are one number wearing five names, and a run that reported them
/// would be reporting arithmetic noise with confident-looking signs on it. The
/// fit turns instead the amplitudes of five modes that are ORTHONORMAL over a
/// declared window, so the printed correlation matrix is a statement about the
/// data rather than about the parameterisation.
///
/// The window is **fixed** -- 40 to 95 degrees of field angle, under a measure
/// uniform in that angle -- and depends only on the file's own mirror
/// parameter. That is what makes a coefficient fitted at one geometry mean the
/// same thing at another, which is what the protocol's cross-validation needs
/// in order to be a test at all. A basis orthogonalised over whatever sites a
/// run happened to keep would be a different basis every run.
#[derive(Clone, Copy, Debug)]
struct RadialBasis {
    /// `to_k[mode][order]`: how much of `r^(2(order+1))` one unit of `mode`
    /// carries. Lower triangular, by construction.
    to_k: [[f64; RADIAL_ORDERS]; RADIAL_ORDERS],
}

/// The window the modes are orthonormal over, in degrees of field angle.
const RADIAL_WINDOW: (f64, f64) = (40.0, 95.0);

impl RadialBasis {
    /// Gram-Schmidt of `r^2 ... r^10` over [`RADIAL_WINDOW`], each mode scaled
    /// to unit rms of the radial multiplier it adds.
    fn build(lens: &Lens) -> Self {
        let xi = lens.intrinsics.xi;
        let samples: Vec<f64> = (0..=550)
            .map(|i| {
                let theta = (RADIAL_WINDOW.0
                    + (RADIAL_WINDOW.1 - RADIAL_WINDOW.0) * f64::from(i) / 550.0)
                    .to_radians();
                theta.sin() / (theta.cos() + xi)
            })
            .collect();
        let value = |coefficients: &[f64; RADIAL_ORDERS], r: f64| {
            let r2 = r * r;
            (0..RADIAL_ORDERS)
                .map(|order| coefficients[order] * r2.powi(order as i32 + 1))
                .sum::<f64>()
        };
        let inner = |a: &[f64; RADIAL_ORDERS], b: &[f64; RADIAL_ORDERS]| {
            samples
                .iter()
                .map(|r| value(a, *r) * value(b, *r))
                .sum::<f64>()
                / samples.len() as f64
        };
        let mut to_k = [[0.0; RADIAL_ORDERS]; RADIAL_ORDERS];
        for mode in 0..RADIAL_ORDERS {
            let mut row = [0.0; RADIAL_ORDERS];
            row[mode] = 1.0;
            for done in 0..mode {
                let overlap = inner(&row, &to_k[done]);
                for order in 0..RADIAL_ORDERS {
                    row[order] -= overlap * to_k[done][order];
                }
            }
            let size = inner(&row, &row).sqrt();
            if size > 0.0 {
                for value in &mut row {
                    *value /= size;
                }
            }
            to_k[mode] = row;
        }
        Self { to_k }
    }

    /// The raw `k1..k5` deltas a vector of mode amplitudes is.
    fn deltas(&self, amplitudes: &[f64; RADIAL_ORDERS]) -> [f64; RADIAL_ORDERS] {
        std::array::from_fn(|order| {
            (0..RADIAL_ORDERS)
                .map(|mode| amplitudes[mode] * self.to_k[mode][order])
                .sum()
        })
    }
}

/// One lens's radial map, as the two things a profile needs: where the plane
/// radius comes from and what the polynomial does to it.
#[derive(Clone, Copy, Debug)]
struct RadialModel {
    xi: f64,
    focal: f64,
    k: [f64; RADIAL_ORDERS],
    /// Which way the polynomial runs. `false` is forward, ideal plane to
    /// distorted plane, which is where `offset_v3` is evaluated and where this
    /// run's own fit lives. `true` reads the same coefficients as a map from
    /// the distorted plane back to the ideal one, which a projection has to
    /// SOLVE rather than evaluate; it is one of `offset_v6`'s four open
    /// choices and B7 scores it like any other.
    inverse: bool,
}

impl RadialModel {
    fn of(lens: &Lens) -> Self {
        let d = lens.distortion;
        Self {
            xi: lens.intrinsics.xi,
            focal: lens.intrinsics.fx,
            k: [d.k1, d.k2, d.k3, d.k4, d.k5],
            inverse: false,
        }
    }

    fn with(mut self, deltas: [f64; RADIAL_ORDERS]) -> Self {
        for (k, delta) in self.k.iter_mut().zip(deltas) {
            *k += delta;
        }
        self
    }

    /// The image radius, in delivered-frame pixels, a ray this many degrees
    /// off the axis lands at.
    fn radius(&self, field_deg: f64) -> f64 {
        let theta = field_deg.to_radians();
        let r = theta.sin() / (theta.cos() + self.xi);
        match self.inverse {
            false => self.focal * r * self.multiplied(r),
            // The polynomial runs the other way, so the distorted radius is
            // whatever the polynomial maps BACK to this ideal one. Bisection:
            // `rd * radial(rd)` is monotone over the field a real lens has.
            true => {
                let (mut low, mut high) = (0.0_f64, 3.0_f64);
                for _ in 0..60 {
                    let middle = 0.5 * (low + high);
                    match middle * self.multiplied(middle) < r {
                        true => low = middle,
                        false => high = middle,
                    }
                }
                self.focal * 0.5 * (low + high)
            }
        }
    }

    /// The radial multiplier at one plane radius.
    fn multiplied(&self, r: f64) -> f64 {
        let r2 = r * r;
        1.0 + r2
            * (self.k[0]
                + r2 * (self.k[1] + r2 * (self.k[2] + r2 * (self.k[3] + r2 * self.k[4]))))
    }
}

/// How far a picture drawn through `with` moves its content OUTWARD, in
/// degrees of field angle, against the same picture drawn through `base`.
///
/// This is the quantity `Site::radial` reads off the pixels, so a fitted model
/// and the residual it was fitted to are in the same units and the same sign,
/// and so are the file's own `offset_v6` candidates in the cross-check the
/// protocol's section 11.2 B7 asks for.
///
/// The content drawn at field angle `theta` under `with` is whatever sits at
/// image radius `with.radius(theta)`, and under `base` that content sat at
/// `theta_shown` where `base.radius(theta_shown)` is the same radius. So the
/// content has moved from `theta_shown` to `theta`, which is outward by
/// `theta - theta_shown`. Solved by bisection: both maps are monotone over the
/// field a real lens has.
fn radial_shift(base: &RadialModel, with: &RadialModel, field_deg: f64) -> f64 {
    let want = with.radius(field_deg);
    let (mut low, mut high) = (0.0_f64, 110.0_f64);
    if base.radius(high) < want {
        return f64::NAN;
    }
    for _ in 0..60 {
        let middle = 0.5 * (low + high);
        match base.radius(middle) < want {
            true => low = middle,
            false => high = middle,
        }
    }
    field_deg - 0.5 * (low + high)
}

/// The camera's own numbers, which are shared across every geometry a run
/// fits: lens 1's five, and one radial delta per lens.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Calib {
    fit: kjerag_render::SeamFit,
    /// Amplitudes of [`RadialBasis`]'s orthonormal modes, per lens.
    radial: [[f64; RADIAL_ORDERS]; 2],
}

impl Calib {
    fn get(&self, k: usize) -> f64 {
        match k {
            0 => self.fit.roll_deg,
            1 => self.fit.yaw_deg,
            2 => self.fit.pitch_deg,
            3 => self.fit.cx_px,
            4 => self.fit.cy_px,
            _ => self.radial[(k - 5) / RADIAL_ORDERS][(k - 5) % RADIAL_ORDERS],
        }
    }

    fn nudged(mut self, k: usize, step: f64) -> Self {
        match k {
            0 => self.fit.roll_deg += step,
            1 => self.fit.yaw_deg += step,
            2 => self.fit.pitch_deg += step,
            3 => self.fit.cx_px += step,
            4 => self.fit.cy_px += step,
            _ => self.radial[(k - 5) / RADIAL_ORDERS][(k - 5) % RADIAL_ORDERS] += step,
        }
        self
    }

    /// The calibration with this correction on it: lens 1's five as
    /// [`kjerag_render::SeamFit`] applies them, and the radial delta on BOTH
    /// lenses, expressed in the raw `k1..k5` the shipped map already reads.
    fn applied(&self, lenses: &[Lens], basis: &[RadialBasis; 2]) -> Vec<Lens> {
        let mut lenses = self.fit.applied(lenses);
        for (index, lens) in lenses.iter_mut().enumerate().take(2) {
            let deltas = basis[index].deltas(&self.radial[index]);
            lens.distortion.k1 += deltas[0];
            lens.distortion.k2 += deltas[1];
            lens.distortion.k3 += deltas[2];
            lens.distortion.k4 += deltas[3];
            lens.distortion.k5 += deltas[4];
        }
        lenses
    }

    fn radial_line(&self) -> String {
        let one = |lens: usize| {
            self.radial[lens]
                .iter()
                .map(|value| format!("{value:.3e}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        format!("{}:{}", one(0), one(1))
    }
}

/// One view of one target: the geometry a solve reads its sites at.
#[derive(Clone, Debug)]
struct ArmSpec {
    label: String,
    /// The export this arm is measured against, or `None` for an arm whose
    /// target is one of our own renders (the plant and the null).
    against: Option<PathBuf>,
    from: f64,
    view: [f64; 3],
    fov: f64,
}

/// One arm's own four numbers, which are the only ones a prediction is allowed
/// to move.
#[derive(Clone, Copy, Debug, Default)]
struct ViewAim {
    view: [f64; 3],
    fov: f64,
}

impl ViewAim {
    fn get(&self, j: usize) -> f64 {
        match j {
            0..=2 => self.view[j],
            _ => self.fov,
        }
    }

    fn nudged(mut self, j: usize, step: f64) -> Self {
        match j {
            0..=2 => self.view[j] += step,
            _ => self.fov += step,
        }
        self
    }

    fn look(&self, base: Look) -> Look {
        let mut look = base;
        look.angles = self.view;
        look.fov = self.fov;
        look
    }
}

/// Our own pass at one calibration and one aim, drawn into the export's
/// projection.
fn drawn(
    lenses: &[Lens],
    frame: Size,
    look: Look,
    pair: &Pair,
    shape: Shape,
    cells: &[kjerag_render::Cell],
    rolling: Option<kjerag_render::Rolling>,
) -> Vec<f64> {
    looked(lenses, frame, look, pair, shape, cells, rolling)
}

/// Zero-mean normalized cross-correlation of a patch of `target` into `ours`.
///
/// Returns the offset `u` with `target(p) ~ ours(p + u)`, the peak, and how
/// far the peak stands above the best rival more than two pixels away. The
/// rival is the ambiguity check: a patch of sky or of one straight edge
/// correlates as well in a line of places, and a shift read off one of them is
/// a number with no direction in it.
fn correlate_patch(
    target: &[f64],
    ours: &[f64],
    shape: Shape,
    at: [usize; 2],
    half: usize,
    search: usize,
) -> Option<([f64; 2], f64, f64)> {
    let (width, height) = (shape.width as usize, shape.height as usize);
    let margin = half + search + 1;
    if at[0] < margin || at[1] < margin || at[0] + margin >= width || at[1] + margin >= height {
        return None;
    }
    let span = 2 * half + 1;
    let mut patch = Vec::with_capacity(span * span);
    for dy in 0..span {
        for dx in 0..span {
            let index = (at[1] + dy - half) * width + at[0] + dx - half;
            let code = target[index];
            // A pixel no lens reached is not content, and a patch that
            // straddles the edge of the picture correlates on that edge.
            if code <= 0.0 {
                return None;
            }
            patch.push(code);
        }
    }
    let mean = patch.iter().sum::<f64>() / patch.len() as f64;
    let patch: Vec<f64> = patch.iter().map(|c| c - mean).collect();
    let energy: f64 = patch.iter().map(|c| c * c).sum();
    // Texture floor: a flat patch has no shift in it and correlating one
    // reports the noise. 4 codes rms over the patch, squared, times its area.
    if energy < 16.0 * patch.len() as f64 {
        return None;
    }

    let reach = search as isize;
    let mut scores = vec![f64::NAN; (2 * search + 1) * (2 * search + 1)];
    for oy in -reach..=reach {
        for ox in -reach..=reach {
            let mut other = Vec::with_capacity(span * span);
            let mut clean = true;
            for dy in 0..span {
                for dx in 0..span {
                    let x = (at[0] + dx - half) as isize + ox;
                    let y = (at[1] + dy - half) as isize + oy;
                    let code = ours[y as usize * width + x as usize];
                    if code <= 0.0 {
                        clean = false;
                    }
                    other.push(code);
                }
            }
            if !clean {
                continue;
            }
            let mean = other.iter().sum::<f64>() / other.len() as f64;
            let mut covariance = 0.0;
            let mut variance = 0.0;
            for (a, b) in patch.iter().zip(&other) {
                let b = b - mean;
                covariance += a * b;
                variance += b * b;
            }
            if variance <= 0.0 {
                continue;
            }
            let index = (oy + reach) as usize * (2 * search + 1) + (ox + reach) as usize;
            scores[index] = covariance / (energy * variance).sqrt();
        }
    }

    let (mut best, mut peak) = (None, f64::MIN);
    for (index, score) in scores.iter().enumerate() {
        if score.is_nan() {
            continue;
        }
        if *score > peak {
            peak = *score;
            best = Some(index);
        }
    }
    let index = best?;
    let (kx, ky) = (
        (index % (2 * search + 1)) as isize - reach,
        (index / (2 * search + 1)) as isize - reach,
    );
    // A peak on the edge of the search is a peak that was cut off, not found.
    if kx.abs() == reach || ky.abs() == reach {
        return None;
    }
    let rival = scores
        .iter()
        .enumerate()
        .filter(|(other, score)| {
            if score.is_nan() {
                return false;
            }
            let (ox, oy) = (
                (*other % (2 * search + 1)) as isize - reach,
                (*other / (2 * search + 1)) as isize - reach,
            );
            (ox - kx).abs() > 2 || (oy - ky).abs() > 2
        })
        .map(|(_, score)| *score)
        .fold(f64::MIN, f64::max);

    let width_scores = 2 * search + 1;
    let sub = |low: f64, mid: f64, high: f64| {
        let curve = low - 2.0 * mid + high;
        match curve == 0.0 || !curve.is_finite() {
            true => 0.0,
            false => (-0.5 * (high - low) / curve).clamp(-1.0, 1.0),
        }
    };
    let get = |dx: isize, dy: isize| {
        scores[((ky + dy + reach) as usize) * width_scores + (kx + dx + reach) as usize]
    };
    let (left, right) = (get(-1, 0), get(1, 0));
    let (up, down) = (get(0, -1), get(0, 1));
    if left.is_nan() || right.is_nan() || up.is_nan() || down.is_nan() {
        return None;
    }
    Some((
        [
            kx as f64 + sub(left, peak, right),
            ky as f64 + sub(up, peak, down),
        ],
        peak,
        peak - rival,
    ))
}

/// The five knobs on one line, in the form every other instrument takes them.
fn knobs_line(fit: &kjerag_render::SeamFit) -> String {
    format!(
        "roll:{:.4},yaw:{:.4},pitch:{:.4},cx:{:.3},cy:{:.3}",
        fit.roll_deg, fit.yaw_deg, fit.pitch_deg, fit.cx_px, fit.cy_px
    )
}

/// What one round of the solve read, kept so the report can say where the
/// residual is rather than only how big it is.
struct Displacements {
    residuals: Vec<Site>,
}

/// One kept site's reading, in the two coordinates the answer is read in.
///
/// The magnitude is what the match criterion is in. The rest is what a FAILURE
/// is reported with: the protocol's section 3 asks a run that does not match to
/// print the residual's structure round the seam, because that structure names
/// the error -- constant along the seam is a relative roll, one cycle round it
/// is the principal point, two cycles is the focal aspect -- and a number with
/// no structure beside it is the next diagnosis's dead end.
#[derive(Clone, Copy)]
struct Site {
    /// Displacement magnitude in output pixels.
    size: f64,
    lens: usize,
    /// Degrees past the seam: negative in the front lens's hemisphere.
    past: f64,
    /// Where round the seam circle this site sits, in degrees. The seam circle
    /// is the body frame's own `z = 0` plane (`kjerag_render::seam::ring`), so
    /// this is `atan2(y, x)` of the ray and nothing more.
    azimuth: f64,
    /// The displacement resolved ALONG the seam circle's tangent and ACROSS
    /// it, in degrees of world angle. Parallax cannot reach the along column
    /// and owns much of the across one, which is why they are kept apart.
    along: f64,
    across: f64,
    /// How far off ITS OWN LENS'S AXIS this site looks, in degrees. The field
    /// angle, which is the variable a distortion polynomial is a function of.
    field: f64,
    /// The same displacement resolved RADIALLY away from that lens's axis and
    /// TANGENTIALLY about it, in degrees.
    ///
    /// This is the second coordinate the protocol's section 3 asks a failure
    /// to be reported in, and it separates two things nothing else here can.
    /// A **pose** error between two calibrations is a rotation, and a rotation
    /// of the lens shows up as a dipole -- one cycle of azimuth, changing sign
    /// across the frame -- with no net radial term. A **distortion** error is
    /// a different radial law, and it shows up as a radial displacement that
    /// is a smooth function of the field angle and the SAME sign all the way
    /// round. The five knobs this solve carries can chase the first and cannot
    /// touch the second, so which of the two the residual is decides whether
    /// the next attempt moves the pose or the polynomial.
    radial: f64,
    tangential: f64,
}

impl Displacements {
    /// Root mean square displacement over the sites a predicate keeps, and how
    /// many that was.
    fn rms(&self, keep: impl Fn(usize, f64) -> bool) -> (f64, usize) {
        let picked: Vec<f64> = self
            .residuals
            .iter()
            .filter(|site| keep(site.lens, site.past))
            .map(|site| site.size * site.size)
            .collect();
        match picked.is_empty() {
            true => (f64::NAN, 0),
            false => (
                (picked.iter().sum::<f64>() / picked.len() as f64).sqrt(),
                picked.len(),
            ),
        }
    }

    /// How much of the seam circle the near-seam sites actually cover, in
    /// degrees.
    ///
    /// Azimuth is circular, so this is 360 less the widest gap between
    /// neighbouring sites and not `max - min`: a handful of sites either side
    /// of zero spans a few degrees and would read 359 the naive way. The
    /// protocol's match criterion asks for 60 degrees of it and this is the
    /// number that answers.
    fn azimuth_spread(&self) -> f64 {
        let mut all: Vec<f64> = self
            .residuals
            .iter()
            .filter(|site| site.past.abs() < 8.0)
            .map(|site| site.azimuth)
            .collect();
        if all.len() < 2 {
            return 0.0;
        }
        all.sort_by(f64::total_cmp);
        let mut widest = all[0] + 360.0 - all[all.len() - 1];
        for pair in all.windows(2) {
            widest = widest.max(pair[1] - pair[0]);
        }
        (360.0 - widest).max(0.0)
    }

    /// The along-seam and across-seam residual binned by azimuth, and the
    /// first three harmonics of the along-seam column.
    ///
    /// Returned as the bins and as `[(cos, sin); 4]` for orders 0 to 3, in
    /// degrees. Order 0 is a relative roll, order 1 is the principal point and
    /// order 2 is the focal aspect: the derivation is this file's own header
    /// and the fit here is the reading of it.
    fn profile(&self, bins: usize) -> (Vec<(f64, f64, f64, usize)>, [(f64, f64); 4]) {
        let near: Vec<&Site> = self
            .residuals
            .iter()
            .filter(|site| site.past.abs() < 8.0)
            .collect();
        let mut binned = vec![(0.0, 0.0, 0usize); bins];
        for site in &near {
            let at = ((site.azimuth + 360.0) / 360.0 * bins as f64) as usize % bins;
            binned[at].0 += site.along;
            binned[at].1 += site.across;
            binned[at].2 += 1;
        }
        let table = binned
            .iter()
            .enumerate()
            .filter(|(_, (_, _, count))| *count > 0)
            .map(|(at, (along, across, count))| {
                (
                    (at as f64 + 0.5) / bins as f64 * 360.0 - 180.0,
                    along / *count as f64,
                    across / *count as f64,
                    *count,
                )
            })
            .collect();
        // A plain least-squares harmonic fit on the sites themselves, not on
        // the bins: the bins are for looking at and the fit is for reporting.
        let mut harmonics = [(0.0, 0.0); 4];
        if !near.is_empty() {
            let n = near.len() as f64;
            harmonics[0].0 = near.iter().map(|site| site.along).sum::<f64>() / n;
            for order in 1..4 {
                let (mut c, mut s) = (0.0, 0.0);
                for site in &near {
                    let angle = f64::from(order as u32) * site.azimuth.to_radians();
                    c += site.along * angle.cos();
                    s += site.along * angle.sin();
                }
                harmonics[order] = (2.0 * c / n, 2.0 * s / n);
            }
        }
        (table, harmonics)
    }

    /// How much of one lens's residual is a SMOOTH function of WHERE the site
    /// is, and how much is scatter that no smooth calibration can reach.
    ///
    /// **This is the question a residual size cannot answer on its own.** A
    /// model is worth adding if the thing it would remove is there to remove:
    /// a residual whose binned means are small and whose site-to-site spread
    /// is large is not a calibration error at all, and a bigger polynomial
    /// fitted to it would be fitting the scatter.
    ///
    /// The sites are binned in TWO coordinates, the field angle and the
    /// azimuth round the seam, because a calibration error is smooth in both
    /// and averaging one of them away hides exactly the errors a rotation
    /// makes -- a dipole in azimuth has zero mean at every field angle. Cells
    /// with fewer than three sites are dropped: a one-site cell has no spread
    /// in it and would be counted as pure structure.
    ///
    /// Returns `(structured, scatter, cells, sites)`, the first two in degrees
    /// of world angle, so that `structured^2 + scatter^2` is the whole of the
    /// residual in the lens's own polar frame.
    fn split(&self, lens: usize, field_step: f64, azimuth_step: f64) -> (f64, f64, usize, usize) {
        let mut cells: std::collections::BTreeMap<(i64, i64), Vec<(f64, f64)>> =
            std::collections::BTreeMap::new();
        for site in self.residuals.iter().filter(|site| site.lens == lens) {
            let key = (
                (site.field / field_step).floor() as i64,
                ((site.azimuth + 360.0) / azimuth_step).floor() as i64,
            );
            cells
                .entry(key)
                .or_default()
                .push((site.radial, site.tangential));
        }
        let (mut structured, mut scatter, mut kept, mut used) = (0.0, 0.0, 0usize, 0usize);
        for values in cells.values().filter(|values| values.len() >= 3) {
            let n = values.len() as f64;
            let mean = (
                values.iter().map(|v| v.0).sum::<f64>() / n,
                values.iter().map(|v| v.1).sum::<f64>() / n,
            );
            structured += n * (mean.0 * mean.0 + mean.1 * mean.1);
            for value in values {
                scatter += (value.0 - mean.0).powi(2) + (value.1 - mean.1).powi(2);
            }
            kept += 1;
            used += values.len();
        }
        match used {
            0 => (f64::NAN, f64::NAN, 0, 0),
            _ => (
                (structured / used as f64).sqrt(),
                (scatter / used as f64).sqrt(),
                kept,
                used,
            ),
        }
    }

    /// The residual against the FIELD ANGLE, per lens: how far off its own
    /// lens's axis a site looks against how far its content moved radially and
    /// tangentially.
    ///
    /// The other half of what section 3 asks a failure to be reported with,
    /// and the half that names the next diagnosis. A calibration whose POSE
    /// differs displaces content by a rotation, which has no net radial term
    /// at any field angle and averages to nothing round the axis. A
    /// calibration whose DISTORTION differs displaces it radially by a smooth
    /// function of the field angle with one sign, which is the shape a
    /// polynomial difference has and the shape none of this solve's five knobs
    /// can produce.
    fn radially(&self, lens: usize, step: f64) -> Vec<(f64, f64, f64, f64, usize)> {
        let mut bins: std::collections::BTreeMap<i64, (f64, f64, f64, usize)> =
            std::collections::BTreeMap::new();
        for site in self.residuals.iter().filter(|site| site.lens == lens) {
            let at = (site.field / step).floor() as i64;
            let entry = bins.entry(at).or_insert((0.0, 0.0, 0.0, 0));
            entry.0 += site.radial;
            entry.1 += site.tangential;
            entry.2 += site.size * site.size;
            entry.3 += 1;
        }
        bins.into_iter()
            .map(|(at, (radial, tangential, squared, count))| {
                let n = count as f64;
                (
                    (at as f64 + 0.5) * step,
                    radial / n,
                    tangential / n,
                    (squared / n).sqrt(),
                    count,
                )
            })
            .collect()
    }
}

/// How much each pair of knobs is the same knob, as far as these rows can tell.
///
/// The correlation matrix of the fitted parameters: the inverse of the normal
/// matrix, scaled to unit diagonal. A pair at 0.999 is a pair the data cannot
/// separate, and reporting each of them with its own small 1 sigma would be a
/// lie of exactly the kind this whole exercise exists to prevent.
fn correlated(rows: &[(Vec<f64>, f64)]) -> Option<Vec<Vec<f64>>> {
    let width = rows.first()?.0.len();
    let mut normal = vec![vec![0.0; 2 * width]; width];
    for (basis, _) in rows {
        for i in 0..width {
            for j in 0..width {
                normal[i][j] += basis[i] * basis[j];
            }
        }
    }
    for (i, row) in normal.iter_mut().enumerate() {
        row[width + i] = 1.0;
    }
    // Gauss-Jordan with partial pivoting.
    for column in 0..width {
        let pivot = (column..width)
            .max_by(|a, b| {
                normal[*a][column]
                    .abs()
                    .partial_cmp(&normal[*b][column].abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(column);
        normal.swap(column, pivot);
        let head = normal[column][column];
        if head.abs() < 1e-18 {
            return None;
        }
        for value in normal[column].iter_mut() {
            *value /= head;
        }
        for row in 0..width {
            if row == column {
                continue;
            }
            let factor = normal[row][column];
            if factor == 0.0 {
                continue;
            }
            let pivot_row = normal[column].clone();
            for (value, above) in normal[row].iter_mut().zip(&pivot_row) {
                *value -= factor * above;
            }
        }
    }
    let inverse: Vec<Vec<f64>> = normal.iter().map(|row| row[width..].to_vec()).collect();
    Some(
        (0..width)
            .map(|i| {
                (0..width)
                    .map(|j| {
                        let scale = (inverse[i][i] * inverse[j][j]).sqrt();
                        match scale > 0.0 {
                            true => inverse[i][j] / scale,
                            false => f64::NAN,
                        }
                    })
                    .collect()
            })
            .collect(),
    )
}

/// A vector over the free knobs, spread back over all of them, with `fill`
/// wherever a knob was pinned.
fn spread_over(values: &[f64], free: &[usize], total: usize, fill: f64) -> Vec<f64> {
    let mut whole = vec![fill; total];
    for (at, k) in free.iter().copied().enumerate() {
        whole[k] = values[at];
    }
    whole
}

/// Fit lens 1's calibration by making our picture land where theirs does.
///
/// **The estimator is the same one on a plant and on a real export**, which is
/// the whole point of the plant: what it recovers is what this can recover.
/// A patch of the target picture is located in ours by normalized cross
/// correlation, which is a GEOMETRIC reading and not a photometric one, so
/// Studio's tone curve, their sharpening and our lack of either divide out of
/// it. Every kept site's displacement is then written as a linear function of
/// the eight numbers through the shipped map itself
/// (`kjerag_render::Reframe`, the shader's own Rust twin) and the stack is
/// solved by least squares, exactly as `mode=residual` fits the five.
/// One geometry's live state during a joint solve.
struct Arm {
    spec: ArmSpec,
    pair: Pair,
    target: Vec<f64>,
    aim: ViewAim,
    /// This instant's own readout turn, or `None` where it is not corrected.
    rolling: Option<kjerag_render::Rolling>,
    coverage: Displacements,
    last: Option<Reached>,
}

/// What one arm's last round read.
#[derive(Clone, Copy)]
struct Reached {
    whole: f64,
    seam: f64,
    at_zero: usize,
    at_one: usize,
    at_seam: usize,
    sites: usize,
}

/// The two lens frames of one instant of the source.
fn pair_at(path: &Path, from: f64, frame: Size) -> Fallible<Pair> {
    let mut walk = Walk::open(path, from, frame)?;
    if walk.streams() < 2 {
        return Err("this file carries one lens stream, so it has no seam".into());
    }
    walk.next_pair()?.ok_or_else(|| "no frame decoded".into())
}

/// Which global columns a run is allowed to move, out of the names it was
/// given.
fn resolve_free(tokens: &[String], labels: &[String]) -> Fallible<Vec<usize>> {
    let total = solved(labels.len());
    let mut out: Vec<usize> = Vec::new();
    for token in tokens {
        let name = token.replace(' ', "");
        let picked: Vec<usize> = match name.as_str() {
            "all" => (0..total).collect(),
            "view" => (SHARED..total).collect(),
            "lens1" => (0..5).collect(),
            "radial" => (5..SHARED).collect(),
            "radial0" => (5..5 + RADIAL_ORDERS).collect(),
            "radial1" => (5 + RADIAL_ORDERS..SHARED).collect(),
            "calib" => (0..SHARED).collect(),
            other => match SHARED_NAMES
                .iter()
                .position(|known| known.replace(' ', "") == other)
            {
                Some(at) => vec![at],
                None => return Err(format!("no knob or group called {other}").into()),
            },
        };
        out.extend(picked);
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// Fit the camera's calibration by making our picture land where theirs does,
/// over one geometry or several at once.
///
/// **The estimator is the same one on a plant and on a real export**, which is
/// the whole point of the plant: what it recovers is what this can recover.
/// A patch of the target picture is located in ours by normalized cross
/// correlation, which is a GEOMETRIC reading and not a photometric one, so
/// Studio's tone curve, their sharpening and our lack of either divide out of
/// it. Every kept site's displacement is then written as a linear function of
/// the numbers through the shipped map itself (`kjerag_render::Reframe`, the
/// shader's own Rust twin) and the stack is solved by least squares.
///
/// **What is new at section 11 of the protocol is the split between the
/// camera's numbers and a view's.** Lens 1's five and the two radial deltas
/// are one camera's and are shared across every geometry in the run; each
/// geometry's own three angles and field of view are its own. That is what
/// makes a cross-validation possible: fit the camera on one aim's geometries,
/// hold it, and let another aim's geometries move nothing but their own view.
fn solve(options: &Options) -> Fallible<()> {
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = options.corrected(
        std::slice::from_ref(&options.input),
        &calibration.lenses,
        frame,
    );
    if lenses.len() < 2 {
        return Err("this file carries one lens, so it has no seam to solve".into());
    }
    let basis = [RadialBasis::build(&lenses[0]), RadialBasis::build(&lenses[1])];
    // The readout, which is the camera's own motion inside one frame and not
    // the display's (issue #9). Read whether or not it is corrected for: the
    // turn across one readout is what says how much of the residual could
    // possibly be the readout, and it is a property of the flight at that
    // instant rather than of any calibration.
    let track = calibration.orientation(Filter::default());
    let readout = calibration.readout();
    let span = (readout.seconds * 1e6) as i64;
    let shape = Shape {
        width: options.size,
        height: ((f64::from(options.size) / options.aspect).round() as u32).max(1),
    };
    let base = Look {
        angles: [0.0; 3],
        fov: 0.0,
        compression: 1.0,
        panini: options.panini,
    };

    let specs: Vec<ArmSpec> = match options.arms.is_empty() {
        true => vec![ArmSpec {
            label: "one".to_owned(),
            against: options.against.clone(),
            from: options.from,
            view: options.view,
            fov: options.fov,
        }],
        false => options.arms.clone(),
    };
    let labels: Vec<String> = specs.iter().map(|spec| spec.label.clone()).collect();
    let free = resolve_free(&options.free, &labels)?;
    let total = solved(specs.len());

    // ---------------------------------------------------------- the target
    let truth = match (options.plant, options.plantradial) {
        (None, None) => None,
        (fit, radial) => Some(Calib {
            fit: fit.unwrap_or_default(),
            radial: radial.unwrap_or_default(),
        }),
    };
    println!(
        "view:   {} {}x{}, {} arm(s), {} free of {total} numbers",
        match options.panini >= 0.0 {
            true => format!("panini d {:.2}", options.panini),
            false => "rectilinear".to_owned(),
        },
        shape.width,
        shape.height,
        specs.len(),
        free.len(),
    );
    println!(
        "radial: {} orders per lens, orthonormal over {:.0} to {:.0} deg of field angle; \n\
         radial: one unit of a mode is one rms of radial multiplier over that window",
        RADIAL_ORDERS, RADIAL_WINDOW.0, RADIAL_WINDOW.1,
    );
    if let Some(truth) = &truth {
        println!(
            "target: A PLANT, not Studio: our own render with {} and radial {} on it",
            knobs_line(&truth.fit),
            truth.radial_line(),
        );
    }

    let mut arms: Vec<Arm> = Vec::new();
    for spec in &specs {
        let pair = pair_at(&options.input, spec.from, frame)?;
        let at = pair.at.as_micros() as i64;
        let turn = match track.is_empty() || span <= 0 {
            true => [0.0; 3],
            false => track.turn(at - span / 2, at + span / 2),
        };
        let sweep = readout.sweep.axis();
        let rolling = match options.rolling && turn != [0.0; 3] && sweep != [0.0; 2] {
            true => Some(kjerag_render::Rolling { turn, axis: sweep }),
            false => None,
        };
        println!(
            "readout: {} turns {:.4} deg across one {:.2} ms readout at {:.1} s, which is \
             {:.1} px of \n         this picture edge to edge; the correction is {}",
            spec.label,
            norm(turn).to_degrees(),
            readout.seconds * 1e3,
            spec.from,
            norm(turn).to_degrees() * f64::from(shape.width) / spec.fov,
            match rolling.is_some() {
                true => "ON",
                false => "off",
            },
        );
        let aim = ViewAim {
            view: spec.view,
            fov: spec.fov,
        };
        let target = match (&truth, &spec.against) {
            (Some(truth), _) => drawn(
                &truth.applied(&lenses, &basis),
                frame,
                aim.look(base),
                &pair,
                shape,
                &options.band,
                rolling,
            ),
            (None, Some(path)) => {
                let at = spec.from - options.lag;
                println!(
                    "target: {} `{}` at {:.4} s, which is source {:.4} s at lag {:+.5} s",
                    spec.label,
                    path.display(),
                    at,
                    spec.from,
                    options.lag,
                );
                export_frame(path, at)?.resampled(shape)
            }
            (None, None) => {
                return Err("solve wants plant=/plantradial= or against=/arm=".into());
            }
        };
        arms.push(Arm {
            spec: spec.clone(),
            pair,
            target,
            aim,
            rolling,
            coverage: Displacements {
                residuals: Vec::new(),
            },
            last: None,
        });
    }

    let mut calib = Calib {
        fit: options.start,
        radial: options.startradial.unwrap_or_default(),
    };

    // ------------------------------------------------- the aim, and its gradient
    // A registration number nobody has watched fall is not a measurement. This
    // walks the first arm's view off the answer in each axis and prints what
    // that costs, so the reader can see the peak rather than be told there is
    // one.
    if options.aim {
        let arm = &arms[0];
        let at = |view: [f64; 3]| {
            let picture = drawn(
                &calib.applied(&lenses, &basis),
                frame,
                ViewAim {
                    view,
                    fov: arm.aim.fov,
                }
                .look(base),
                &arm.pair,
                shape,
                &options.band,
                arm.rolling,
            );
            agree(&picture, &arm.target)
        };
        println!(
            "\n{:>9} {:>11} {:>11} {:>11}",
            "off deg", "yaw", "pitch", "roll"
        );
        for off in [0.0, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0] {
            let mut scores = [0.0; 3];
            for (axis, score) in scores.iter_mut().enumerate() {
                let mut view = arm.aim.view;
                view[axis] += off;
                *score = at(view);
            }
            println!(
                "{off:>9.2} {:>11.5} {:>11.5} {:>11.5}",
                scores[0], scores[1], scores[2]
            );
        }
        println!(
            "\nzero-mean normalized cross correlation of our whole picture against the target. \n\
             the row at 0.00 is the same number three times by construction; every row below it \n\
             is the cost of aiming wrong by that many degrees about that axis."
        );
        return Ok(());
    }

    // ----------------------------------------------------------- the solve
    for (t, arm) in arms.iter_mut().enumerate() {
        arm.aim.view = std::array::from_fn(|c| arm.spec.view[c] + options.viewoff[c]);
        arm.aim.fov = arm.spec.fov + options.fovoff;
        println!(
            "start:  {} view yaw {:+.3}, pitch {:+.3}, roll {:+.3}, fov {:.3}",
            labels[t], arm.aim.view[0], arm.aim.view[1], arm.aim.view[2], arm.aim.fov,
        );
    }
    println!(
        "start:  lens 1 {}; radial {}",
        knobs_line(&calib.fit),
        calib.radial_line(),
    );
    println!(
        "\n{:>5} {:>10} {:>7} {:>9} {:>9} {:>9} {:>9}",
        "round", "arm", "sites", "rms px", "lens0 px", "lens1 px", "seam px"
    );

    let du = 1.0 / f64::from(shape.width);
    let dv = 1.0 / f64::from(shape.height);
    let mut errors = vec![f64::NAN; total];
    let mut correlations: Option<Vec<Vec<f64>>> = None;
    for round in 0..=options.rounds {
        // The search shrinks once the first round has taken out the bulk: a
        // wide search is what finds a two degree error and a narrow one is
        // what stops a repeating texture from being found in the wrong place.
        let search = match round {
            0 => options.search,
            _ => (options.search / 3).max(3),
        };
        let mut rows: Vec<(Vec<f64>, f64)> = Vec::new();
        let mut refused = None;
        for (t, arm) in arms.iter_mut().enumerate() {
            // Which global columns this arm can say anything about: every
            // shared one, plus its own four.
            let mine: Vec<usize> = (0..SHARED)
                .chain(SHARED + PER_ARM * t..SHARED + PER_ARM * (t + 1))
                .collect();
            let mut column_of = vec![None; total];
            for (at, k) in mine.iter().copied().enumerate() {
                column_of[k] = Some(at);
            }
            let arm_rolling = arm.rolling;
            let build = |calib: Calib, aim: ViewAim| {
                let applied = calib.applied(&lenses, &basis);
                (
                    aim.look(base),
                    Reframe::new(
                        &applied,
                        frame,
                        Camera::default(),
                        Held {
                            body_from_world: orientation(aim.view),
                            rolling: arm_rolling,
                        },
                        1.0,
                        false,
                        Sampling::default(),
                    ),
                )
            };
            let applied = calib.applied(&lenses, &basis);
            let picture = drawn(
                &applied,
                frame,
                arm.aim.look(base),
                &arm.pair,
                shape,
                &options.band,
                arm_rolling,
            );
            let here = build(calib, arm.aim);
            // Where the two lenses point in the body frame, read out of the
            // map this round is drawing through rather than out of the
            // calibration, so a solve that has moved lens 1 measures its field
            // angles about where lens 1 now is.
            let lens_axes = [axis_of(&here.1, 0), axis_of(&here.1, 1)];
            let steps: Vec<((Look, Reframe), (Look, Reframe))> = mine
                .iter()
                .copied()
                .map(|k| {
                    let step = knob_step(k);
                    match k < SHARED {
                        true => (
                            build(calib.nudged(k, -step), arm.aim),
                            build(calib.nudged(k, step), arm.aim),
                        ),
                        false => {
                            let j = (k - SHARED) % PER_ARM;
                            (
                                build(calib, arm.aim.nudged(j, -step)),
                                build(calib, arm.aim.nudged(j, step)),
                            )
                        }
                    }
                })
                .collect();
            // The ray comes out of THIS aim's own projection, so a step in the
            // view's field of view moves the landing exactly as a step in a
            // lens pose does, and the two are solved side by side.
            let land = |at: &(Look, Reframe), uv: [f64; 2], lens: usize| -> Option<[f64; 2]> {
                let ray = at.0.ray(uv, shape.aspect());
                let (_, landings) = Weighting::Shipped.bent(&at.1, ray, &options.band);
                landings[lens].inside.then(|| {
                    [
                        f64::from(landings[lens].pixel[0]),
                        f64::from(landings[lens].pixel[1]),
                    ]
                })
            };

            let stride = (shape.width as usize / options.sites).max(1);
            let mut reading = Displacements {
                residuals: Vec::new(),
            };
            let mut sites = 0usize;
            let margin = options.patch + search + 2;
            let mut y = margin;
            while y + margin < shape.height as usize {
                let mut x = margin;
                while x + margin < shape.width as usize {
                    let at = [x, y];
                    x += stride;
                    let uv = [(at[0] as f64 + 0.5) * du, (at[1] as f64 + 0.5) * dv];
                    let ray = here.0.ray(uv, shape.aspect());
                    let (weights, _) = Weighting::Shipped.bent(&here.1, ray, &options.band);
                    let lens = usize::from(weights[1] > weights[0]);
                    // A site the two lenses share is not a site: the content
                    // there is a blend of two displacements and a correlation
                    // finds neither of them.
                    if weights[lens] < 0.999 {
                        continue;
                    }
                    let past = past_seam(&here.1, ray);

                    let Some(right) = land(&here, [uv[0] + du, uv[1]], lens) else {
                        continue;
                    };
                    let Some(left) = land(&here, [uv[0] - du, uv[1]], lens) else {
                        continue;
                    };
                    let Some(down) = land(&here, [uv[0], uv[1] + dv], lens) else {
                        continue;
                    };
                    let Some(up) = land(&here, [uv[0], uv[1] - dv], lens) else {
                        continue;
                    };
                    let a = [
                        [(right[0] - left[0]) / 2.0, (down[0] - up[0]) / 2.0],
                        [(right[1] - left[1]) / 2.0, (down[1] - up[1]) / 2.0],
                    ];
                    let determinant = a[0][0] * a[1][1] - a[0][1] * a[1][0];
                    if determinant.abs() < 1e-9 {
                        continue;
                    }
                    let inverse = [
                        [a[1][1] / determinant, -a[0][1] / determinant],
                        [-a[1][0] / determinant, a[0][0] / determinant],
                    ];
                    let mut jacobian = vec![[0.0; 2]; mine.len()];
                    let mut whole = true;
                    for (column, k) in mine.iter().copied().enumerate() {
                        let (Some(minus), Some(plus)) = (
                            land(&steps[column].0, uv, lens),
                            land(&steps[column].1, uv, lens),
                        ) else {
                            whole = false;
                            break;
                        };
                        let step = knob_step(k);
                        let b = [
                            (plus[0] - minus[0]) / (2.0 * step),
                            (plus[1] - minus[1]) / (2.0 * step),
                        ];
                        // A landing moved by b makes the content at this pixel
                        // the content that was b away, so the picture moves by
                        // -A^-1 b.
                        jacobian[column][0] = -(inverse[0][0] * b[0] + inverse[0][1] * b[1]);
                        jacobian[column][1] = -(inverse[1][0] * b[0] + inverse[1][1] * b[1]);
                    }
                    if !whole {
                        continue;
                    }

                    let Some((shift, peak, margin_over_rival)) =
                        correlate_patch(&arm.target, &picture, shape, at, options.patch, search)
                    else {
                        continue;
                    };
                    if peak < options.floor || margin_over_rival < 0.03 {
                        continue;
                    }
                    sites += 1;
                    // Where round the seam this site is, and what its
                    // displacement is when it is resolved onto the seam's own
                    // tangent rather than onto the picture's axes.
                    let azimuth = ray[1].atan2(ray[0]).to_degrees();
                    let (sin, cos) = azimuth.to_radians().sin_cos();
                    let (along_axis, across_axis) = ([-sin, cos, 0.0], [0.0, 0.0, 1.0]);
                    let step_u = here.0.ray([uv[0] + du, uv[1]], shape.aspect());
                    let back_u = here.0.ray([uv[0] - du, uv[1]], shape.aspect());
                    let step_v = here.0.ray([uv[0], uv[1] + dv], shape.aspect());
                    let back_v = here.0.ray([uv[0], uv[1] - dv], shape.aspect());
                    let moved: [f64; 3] = std::array::from_fn(|c| {
                        shift[0] * (step_u[c] - back_u[c]) / 2.0
                            + shift[1] * (step_v[c] - back_v[c]) / 2.0
                    });
                    let onto = |axis: [f64; 3]| {
                        (0..3).map(|c| moved[c] * axis[c]).sum::<f64>().to_degrees()
                    };
                    let axis = lens_axes[lens];
                    let along_ray = (0..3).map(|c| axis[c] * ray[c]).sum::<f64>();
                    let perpendicular: [f64; 3] =
                        std::array::from_fn(|c| axis[c] - along_ray * ray[c]);
                    let field = along_ray.clamp(-1.0, 1.0).acos().to_degrees();
                    let outward = unit(perpendicular).map(|c| -c);
                    let round_axis = [
                        ray[1] * outward[2] - ray[2] * outward[1],
                        ray[2] * outward[0] - ray[0] * outward[2],
                        ray[0] * outward[1] - ray[1] * outward[0],
                    ];
                    reading.residuals.push(Site {
                        size: shift[0].hypot(shift[1]),
                        lens,
                        past,
                        azimuth,
                        along: onto(along_axis),
                        across: onto(across_axis),
                        field,
                        radial: onto(outward),
                        tangential: onto(round_axis),
                    });
                    // The picture has to move by -shift to land on the target.
                    for axis in 0..2 {
                        let row: Vec<f64> = free
                            .iter()
                            .map(|k| match column_of[*k] {
                                Some(column) => jacobian[column][axis],
                                None => 0.0,
                            })
                            .collect();
                        rows.push((row, -shift[axis]));
                    }
                }
                y += stride;
            }

            let (whole, _) = reading.rms(|_, _| true);
            let (zero, at_zero) = reading.rms(|lens, _| lens == 0);
            let (one, at_one) = reading.rms(|lens, _| lens == 1);
            let (seam, at_seam) = reading.rms(|_, past| past.abs() < 8.0);
            println!(
                "{round:>5} {:>10} {sites:>7} {whole:>9.4} {zero:>9.4} {one:>9.4} {seam:>9.4}",
                labels[t],
            );
            if at_zero == 0 || at_one == 0 {
                refused = Some((labels[t].clone(), at_zero, at_one, at_seam));
            }
            arm.last = Some(Reached {
                whole,
                seam,
                at_zero,
                at_one,
                at_seam,
                sites,
            });
            arm.coverage = reading;
        }
        if let Some((label, at_zero, at_one, at_seam)) = refused {
            println!(
                "\nREFUSED: arm {label} kept {at_zero} sites on lens 0 and {at_one} on lens 1. \n\
                 A view that does not carry both lenses cannot separate the view's own pose \n\
                 from lens 1's correction, and a number fitted here would be the two of them \n\
                 added together. ({at_seam} sites within 8 degrees of the seam.)"
            );
            return Ok(());
        }

        if round == options.rounds {
            break;
        }
        // See Options::damp: this is what stops a straight-down view from
        // being a singular matrix rather than an answer.
        for (column, k) in free.iter().copied().enumerate() {
            let mut basis_row = vec![0.0; free.len()];
            basis_row[column] = options.damp / knob_step(k);
            rows.push((basis_row, 0.0));
        }
        let Some(fit) = least_squares(&rows) else {
            println!(
                "\nREFUSED: the normal equations are singular on {} rows",
                rows.len()
            );
            return Ok(());
        };
        errors = spread_over(&fit.errors, &free, total, f64::NAN);
        correlations = correlated(&rows).map(|reduced| {
            let mut whole = vec![vec![f64::NAN; total]; total];
            for (i, row) in free.iter().copied().enumerate() {
                for (j, column) in free.iter().copied().enumerate() {
                    whole[row][column] = reduced[i][j];
                }
            }
            whole
        });
        let step = spread_over(
            &(0..free.len())
                .map(|column| {
                    let k = free[column];
                    fit.params[column].clamp(-knob_cap(k), knob_cap(k))
                })
                .collect::<Vec<_>>(),
            &free,
            total,
            0.0,
        );
        for k in 0..SHARED {
            calib = calib.nudged(k, step[k]);
        }
        for (t, arm) in arms.iter_mut().enumerate() {
            for j in 0..PER_ARM {
                arm.aim = arm.aim.nudged(j, step[SHARED + PER_ARM * t + j]);
            }
        }
    }

    // ---------------------------------------------------------- the answer
    let at = |k: usize| match k < SHARED {
        true => calib.get(k),
        false => arms[(k - SHARED) / PER_ARM].aim.get((k - SHARED) % PER_ARM),
    };
    println!(
        "\n{:<20} {:>11} {:>11} {:>11} {:>10}",
        "knob", "solved", "planted", "error", "1 sigma"
    );
    for k in 0..total {
        let got = at(k);
        let held = match free.contains(&k) {
            true => "",
            false => "  HELD",
        };
        match truth.filter(|_| k < SHARED) {
            Some(truth) => {
                let want = truth.get(k);
                println!(
                    "{:<20} {got:>11.5} {want:>11.5} {:>+11.5} {:>10.5}  {}{held}",
                    knob_name(&labels, k),
                    got - want,
                    errors[k],
                    knob_unit(k),
                );
            }
            None => println!(
                "{:<20} {got:>11.5} {:>11} {:>11} {:>10.5}  {}{held}",
                knob_name(&labels, k),
                "-",
                "-",
                errors[k],
                knob_unit(k),
            ),
        }
    }

    // ------------------------------------- what the match criterion asks of it
    //
    // Printed for every arm, pass or fail, because it is the criterion's own
    // arithmetic and not a summary of it: section 3(a) of
    // docs/research/parity-protocol.md wants 100 sites, 40 of them within 8
    // degrees of the seam, over 60 degrees of seam azimuth, at 1.0 px rms of
    // the export's own pixel scale.
    for (t, arm) in arms.iter().enumerate() {
        let Some(reached) = arm.last else { continue };
        let Reached {
            whole,
            seam,
            at_zero,
            at_one,
            at_seam,
            sites,
        } = reached;
        println!(
            "\n=== arm {} ===\nresidual {whole:.4} px rms over {sites} sites ({at_zero} on \
             lens 0, {at_one} on lens 1, {at_seam} within 8 deg of the seam at {seam:.4} px rms)",
            labels[t],
        );
        let spread = arm.coverage.azimuth_spread();
        let per_degree = f64::from(shape.width) / arm.aim.fov;
        let gate = |ok: bool| match ok {
            true => "PASS",
            false => "FAIL",
        };
        println!(
            "criterion 3(a), at {} px across the picture ({per_degree:.2} px per degree of \
             world angle, so 1.0 px is {:.4} deg):\n  \
             {:<5} {sites} sites kept, wanted 100\n  \
             {:<5} {at_seam} of them within 8 deg of the seam, wanted 40\n  \
             {:<5} {spread:.1} deg of seam azimuth covered, wanted 60\n  \
             {:<5} {whole:.4} px rms over all of them, wanted 1.0 ({seam:.4} px near the seam)",
            shape.width,
            1.0 / per_degree,
            gate(sites >= 100),
            gate(at_seam >= 40),
            gate(spread >= 60.0),
            gate(whole <= 1.0),
        );
        // ---- the structure, which is what a failure is reported WITH
        let (table, harmonics) = arm.coverage.profile(24);
        println!(
            "\nthe residual round the seam, over the sites within 8 deg of it:\n\
             \n{:>10} {:>12} {:>12} {:>8}",
            "azimuth", "along deg", "across deg", "sites",
        );
        for (azimuth, along, across, count) in &table {
            println!("{azimuth:>10.1} {along:>12.5} {across:>12.5} {count:>8}");
        }
        println!(
            "\n{:>8} {:>12} {:>12} {:>12}   what each order of the along-seam column names",
            "order", "cos deg", "sin deg", "amplitude",
        );
        for (order, (cosine, sine)) in harmonics.iter().enumerate() {
            println!(
                "{order:>8} {cosine:>12.5} {sine:>12.5} {:>12.5}   {}",
                cosine.hypot(*sine),
                match order {
                    0 => "a relative ROLL between the lenses",
                    1 => "the PRINCIPAL POINT",
                    2 => "the FOCAL ASPECT, fx against fy",
                    _ => "no calibration term of this map reaches here",
                },
            );
        }
        // ---- and the same residual against the field angle, per lens
        for lens in 0..2 {
            let table = arm.coverage.radially(lens, 5.0);
            if table.is_empty() {
                continue;
            }
            println!(
                "\nlens {lens}, the residual against ITS OWN field angle:\n\
                 \n{:>10} {:>12} {:>12} {:>10} {:>8}",
                "field deg", "radial deg", "tangent deg", "rms px", "sites",
            );
            let mut heaviest: (f64, f64) = (0.0, 0.0);
            let mut swing: (f64, f64) = (f64::MAX, f64::MIN);
            for (field, radial, tangential, rms, count) in &table {
                println!(
                    "{field:>10.1} {radial:>12.5} {tangential:>12.5} {rms:>10.4} {count:>8}"
                );
                if radial.abs() > heaviest.0.abs() {
                    heaviest = (*radial, *field);
                }
                swing = (swing.0.min(*tangential), swing.1.max(*tangential));
            }
            println!(
                "heaviest radial bin {:+.5} deg at {:.1} deg of field ({:+.2} px here); \
                 tangential swing {:.5} deg over the field, which is B6's own number.",
                heaviest.0,
                heaviest.1,
                heaviest.0 * per_degree,
                swing.1 - swing.0,
            );
            let (structured, scatter, cells, used) = arm.coverage.split(lens, 5.0, 30.0);
            println!(
                "of that residual, {:.3} px is a SMOOTH function of (field, azimuth) and \
                 {:.3} px is \nsite-to-site SCATTER, over {used} sites in {cells} cells of 5 \
                 deg by 30. No calibration \nof any shape can reach the second number: it is \
                 what the two pictures disagree by \nat sites that sit in the same place.",
                structured * per_degree,
                scatter * per_degree,
            );
        }
    }

    // ------------------------------------------- the radial answer, in the file's units
    println!("\n=== the fitted radial law ===");
    for lens in 0..2 {
        let deltas = basis[lens].deltas(&calib.radial[lens]);
        println!(
            "lens {lens}: modes {} \n  raw deltas k1 {:+.6e}  k2 {:+.6e}  k3 {:+.6e}  \
             k4 {:+.6e}  k5 {:+.6e}",
            calib.radial[lens]
                .iter()
                .map(|value| format!("{value:+.4e}"))
                .collect::<Vec<_>>()
                .join(" "),
            deltas[0],
            deltas[1],
            deltas[2],
            deltas[3],
            deltas[4],
        );
    }
    let fields: Vec<f64> = (0..=9).map(|i| 45.0 + 5.0 * f64::from(i)).collect();
    println!(
        "\nthe fitted delta as a DISPLACEMENT, in degrees of field angle, outward positive \n\
         (the same quantity and the same sign as the `radial deg` column above):\n\
         \n{:>10} {:>14} {:>14}",
        "field deg", "lens 0 deg", "lens 1 deg",
    );
    let fitted_models: Vec<(RadialModel, RadialModel)> = (0..2)
        .map(|lens| {
            let base = RadialModel::of(&lenses[lens]);
            (base, base.with(basis[lens].deltas(&calib.radial[lens])))
        })
        .collect();
    for field in &fields {
        println!(
            "{field:>10.1} {:>14.5} {:>14.5}",
            radial_shift(&fitted_models[0].0, &fitted_models[0].1, *field),
            radial_shift(&fitted_models[1].0, &fitted_models[1].1, *field),
        );
    }
    if let Some(truth) = &truth {
        println!(
            "\nthe PLANTED delta, the same way, so the recovery can be read as a picture \n\
             rather than as five coefficients:\n\
             \n{:>10} {:>14} {:>14}",
            "field deg", "lens 0 deg", "lens 1 deg",
        );
        let planted: Vec<(RadialModel, RadialModel)> = (0..2)
            .map(|lens| {
                let base = RadialModel::of(&lenses[lens]);
                (base, base.with(basis[lens].deltas(&truth.radial[lens])))
            })
            .collect();
        for field in &fields {
            println!(
                "{field:>10.1} {:>14.5} {:>14.5}",
                radial_shift(&planted[0].0, &planted[0].1, *field),
                radial_shift(&planted[1].0, &planted[1].1, *field),
            );
        }
        let mut worst_angle = 0.0_f64;
        let mut worst_point = 0.0_f64;
        let mut worst_radial = 0.0_f64;
        for k in 0..SHARED {
            let off = (calib.get(k) - truth.get(k)).abs();
            match k {
                0..=2 => worst_angle = worst_angle.max(off),
                3 | 4 => worst_point = worst_point.max(off),
                _ => worst_radial = worst_radial.max(off),
            }
        }
        println!(
            "PLANT: lens 1's three angles recovered to {worst_angle:.4} deg, its principal \n\
             point to {worst_point:.3} px, and the worst radial mode to {worst_radial:.3e} \n\
             of what was planted. The views' own numbers are nuisance and are not scored."
        );
    }

    // --------------------------------- the cross-check against the file's own v6
    v6_cross_check(&calibration, &lenses, &fitted_models, &fields);

    // What the numbers above are allowed to be read as, one knob at a time.
    if let Some(correlations) = &correlations {
        println!("\ncorrelation between the fitted numbers, as these sites constrain them:");
        print!("{:<20}", "");
        for k in &free {
            print!(" {:>10.10}", knob_name(&labels, *k));
        }
        println!();
        for i in &free {
            print!("{:<20}", knob_name(&labels, *i));
            for j in &free {
                print!(" {:>10.4}", correlations[*i][*j]);
            }
            println!();
        }
        let mut worst: Option<(f64, usize, usize)> = None;
        for (at, i) in free.iter().copied().enumerate() {
            for j in free.iter().copied().skip(at + 1) {
                let value = correlations[i][j];
                if worst.is_none_or(|(seen, _, _)| value.abs() > seen.abs()) {
                    worst = Some((value, i, j));
                }
            }
        }
        if let Some((value, i, j)) = worst {
            println!(
                "\nconditioning (G6, and B5 of section 11): this run separates {} and {} \n\
                 least, at {value:+.4}. A pair past 0.99 is one number wearing two names: \n\
                 their combination is measured and each of them on its own is not, whatever \n\
                 the 1 sigma column says.",
                knob_name(&labels, i),
                knob_name(&labels, j),
            );
        }
    }

    // ------------------------------------------------------- what to look at
    if let Some(out) = &options.out {
        for (t, arm) in arms.iter().enumerate() {
            let out = match arms.len() {
                1 => out.clone(),
                _ => out.join(&labels[t]),
            };
            std::fs::create_dir_all(&out)?;
            let picture = drawn(
                &calib.applied(&lenses, &basis),
                frame,
                arm.aim.look(base),
                &arm.pair,
                shape,
                &options.band,
                arm.rolling,
            );
            let difference: Vec<f64> = picture
                .iter()
                .zip(&arm.target)
                .map(|(a, b)| match *a > 0.0 && *b > 0.0 {
                    true => 128.0 + (a - b) * options.amp,
                    false => 0.0,
                })
                .collect();
            let amp = format!("{:.0}", options.amp);
            write_gray(&arm.target, shape, &out.join("theirs.png"))?;
            write_gray(&picture, shape, &out.join("ours.png"))?;
            write_gray(&difference, shape, &out.join(format!("difference-{amp}x.png")))?;
            let (beside, wide) = side_by_side(&arm.target, &picture, shape);
            write_gray(&beside, wide, &out.join("side-by-side.png"))?;
            println!(
                "wrote {}/theirs.png, ours.png, difference-{amp}x.png (amplified {amp}x about \
                 mid grey) and side-by-side.png (theirs left, ours right, half scale)",
                out.display(),
            );
        }
    }
    Ok(())
}

/// The protocol's section 11.2 B7: does the radial law this run FITTED from
/// pixels match any reading of the thirteen the file itself writes?
///
/// The comparison is made in the observable and not in the coefficients -- the
/// displacement in degrees of field angle, which is what the residual is
/// measured in -- because two different `(xi, f, k)` triples can draw the same
/// picture and a coefficient-by-coefficient comparison would call that a
/// mismatch.
///
/// The candidate space is small and is not a matter of opinion: `offset_v6`'s
/// radial head is tokens 1 to 5 under **every one** of the sixteen readings
/// `kjerag_meta::Reading` enumerates (only the tangential pair, the growing
/// pair, the prism order and the DIRECTION differ between them), so what can
/// vary here is which intrinsics the radial head is composed with, and whether
/// the polynomial runs forward or inverse.
fn v6_cross_check(
    calibration: &CalibrationSet,
    lenses: &[Lens],
    fitted: &[(RadialModel, RadialModel)],
    fields: &[f64],
) {
    println!("\n=== the cross-check against the file's own offset_v6 (B7) ===");
    let Some(v6) = v6_lenses(calibration) else {
        println!("this file carries no readable offset_v6, so there is nothing to check against.");
        return;
    };
    let candidates: Vec<(String, [RadialModel; 2])> = vec![
        (
            "v6 radial over v6 intrinsics, forward".to_owned(),
            [v6[0], v6[1]],
        ),
        (
            "v6 radial over v3 intrinsics, forward".to_owned(),
            std::array::from_fn(|lens| RadialModel {
                xi: fitted[lens].0.xi,
                focal: fitted[lens].0.focal,
                k: v6[lens].k,
                inverse: false,
            }),
        ),
        (
            "v6 intrinsics over v3 radial (the v6pose arm)".to_owned(),
            std::array::from_fn(|lens| RadialModel {
                xi: v6[lens].xi,
                focal: v6[lens].focal,
                k: fitted[lens].0.k,
                inverse: false,
            }),
        ),
        (
            "v6 radial over v6 intrinsics, INVERSE direction".to_owned(),
            std::array::from_fn(|lens| RadialModel {
                inverse: true,
                ..v6[lens]
            }),
        ),
    ];
    println!(
        "each row is that reading's own displacement profile against the SAME v3 base the \n\
         fit was measured against, and the two rms columns are how far it sits from what \n\
         this run fitted. B7 calls a reading a match at 0.01 deg rms or better.\n\
         \n{:<46} {:>12} {:>12} {:>12} {:>12}",
        "reading", "l0 own rms", "l0 vs fit", "l1 own rms", "l1 vs fit",
    );
    let mut best: Option<(f64, String)> = None;
    for (name, models) in &candidates {
        let mut own = [0.0; 2];
        let mut against = [0.0; 2];
        for lens in 0..2 {
            let base = fitted[lens].0;
            let (mut sum_own, mut sum_against, mut count) = (0.0, 0.0, 0.0);
            for field in fields {
                let theirs = radial_shift(&base, &models[lens], *field);
                let ours = radial_shift(&base, &fitted[lens].1, *field);
                if !theirs.is_finite() || !ours.is_finite() {
                    continue;
                }
                sum_own += theirs * theirs;
                sum_against += (theirs - ours) * (theirs - ours);
                count += 1.0;
            }
            if count > 0.0 {
                own[lens] = (sum_own / count).sqrt();
                against[lens] = (sum_against / count).sqrt();
            }
        }
        let worst = against[0].max(against[1]);
        if best.as_ref().is_none_or(|(seen, _)| worst < *seen) {
            best = Some((worst, name.clone()));
        }
        println!(
            "{name:<46} {:>12.5} {:>12.5} {:>12.5} {:>12.5}",
            own[0], against[0], own[1], against[1],
        );
    }
    println!(
        "\nfor reference, the fitted law's own size: lens 0 {:.5} deg rms, lens 1 {:.5} deg rms.",
        rms_over(fields, &fitted[0].0, &fitted[0].1),
        rms_over(fields, &fitted[1].0, &fitted[1].1),
    );
    if let Some((worst, name)) = best {
        println!(
            "closest reading: {name}, at {worst:.5} deg rms on its worse lens. B7 wants \n\
             0.01000 or better on BOTH lenses to call it a match.",
        );
    }
    let _ = lenses;
}

fn rms_over(fields: &[f64], base: &RadialModel, with: &RadialModel) -> f64 {
    let (mut sum, mut count) = (0.0, 0.0);
    for field in fields {
        let value = radial_shift(base, with, *field);
        if value.is_finite() {
            sum += value * value;
            count += 1.0;
        }
    }
    match count > 0.0 {
        true => (sum / count).sqrt(),
        false => f64::NAN,
    }
}

/// The two lenses' radial models as `offset_v6` writes them, scaled into the
/// delivered frame the same way `offset_v3` is.
///
/// `offset_v6` is `lens_count`, then 27 fields a lens -- the same eleven pose
/// and intrinsic fields `offset_v3` opens with, THIRTEEN distortion
/// coefficients where v3 writes five, then the canvas and the lens type -- and
/// a version word. The crop ratio the focal length needs is not in the string;
/// it is recovered from what the v3 read already did to v3's own focal length,
/// so the two are scaled identically by construction.
fn v6_lenses(calibration: &CalibrationSet) -> Option<[RadialModel; 2]> {
    let tokens: Vec<f64> = calibration
        .offset_v6
        .split('_')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let count = *tokens.first()? as usize;
    if count < 2 || tokens.len() < 1 + 27 * count {
        return None;
    }
    let v3: Vec<f64> = calibration
        .offset_v3
        .split('_')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    Some(std::array::from_fn(|lens| {
        let block = &tokens[1 + 27 * lens..1 + 27 * (lens + 1)];
        // The same crop ratio the v3 read applied, taken from what it did.
        let ratio = calibration.lenses[lens].intrinsics.fx / v3[1 + 19 * lens + 1];
        RadialModel {
            xi: block[0],
            focal: block[1] * ratio,
            k: [block[11], block[12], block[13], block[14], block[15]],
            inverse: false,
        }
    }))
}

/// Two pictures of the same shape, halved and set beside each other.
///
/// Half scale on purpose: the thing this is for is a person looking at two
/// pictures at once and saying whether they are the same picture, and a pair of
/// 4K frames laid side by side is 7680 pixels of something nobody can see at
/// once. The full-size pair is written beside it for anyone who wants to
/// pixel-peep, and the difference picture is where the size of the
/// disagreement is read off anyway.
fn side_by_side(left: &[f64], right: &[f64], shape: Shape) -> (Vec<f64>, Shape) {
    let half = Shape {
        width: (shape.width / 2).max(1),
        height: (shape.height / 2).max(1),
    };
    let shrink = |luma: &[f64]| -> Vec<f64> {
        (0..half.pixels() as usize)
            .map(|index| {
                let (x, y) = (
                    index % half.width as usize * 2,
                    index / half.width as usize * 2,
                );
                let at = |dx: usize, dy: usize| {
                    luma[(y + dy).min(shape.height as usize - 1) * shape.width as usize
                        + (x + dx).min(shape.width as usize - 1)]
                };
                (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / 4.0
            })
            .collect()
    };
    let (left, right) = (shrink(left), shrink(right));
    let wide = Shape {
        width: half.width * 2 + 8,
        height: half.height,
    };
    let mut both = vec![255.0; wide.pixels() as usize];
    for y in 0..half.height as usize {
        for x in 0..half.width as usize {
            both[y * wide.width as usize + x] = left[y * half.width as usize + x];
            both[y * wide.width as usize + x + half.width as usize + 8] =
                right[y * half.width as usize + x];
        }
    }
    (both, wide)
}

/// A rectangular luma picture, written where it can be looked at.
fn write_gray(luma: &[f64], shape: Shape, path: &Path) -> Fallible<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let pixels: Vec<u8> = luma.iter().map(|c| c.clamp(0.0, 255.0) as u8).collect();
    let mut png = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(path)?),
        shape.width,
        shape.height,
    );
    png.set_color(png::ColorType::Grayscale);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()?.write_image_data(&pixels)?;
    Ok(())
}

/// Mean squared gradient over the pixels whose distance from the seam falls in
/// `band`.
/// How many pixels the statistic above was taken over.
///
/// Printed beside every ratio, and the reason is a failure already in the
/// record: a band that lands on nothing at all reads 0.000, which looks like a
/// picture with no sharpness rather than a mask with no pixels.
fn counted(luma: &[f64], seam: &[f64], shape: Shape, band: (f64, f64)) -> usize {
    let (width, height) = (shape.width as usize, shape.height as usize);
    let mut count = 0;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let index = y * width + x;
            let past = seam[index].abs();
            if past < band.0 || past > band.1 {
                continue;
            }
            if [index - 1, index, index + 1]
                .iter()
                .any(|at| luma[*at] <= 0.0)
            {
                continue;
            }
            count += 1;
        }
    }
    count
}

fn banded(luma: &[f64], seam: &[f64], shape: Shape, band: (f64, f64)) -> f64 {
    let (width, height) = (shape.width as usize, shape.height as usize);
    let mut total = 0.0;
    let mut count = 0.0;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let index = y * width + x;
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
///
/// Rectangular, because a stitcher's output is: the owner's own export is
/// 16:9 and a square reader indexed off the end of it.
struct Export {
    shape: Shape,
    luma: Vec<f64>,
}

/// A picture's size in pixels, and the aspect every ray of it is built with.
#[derive(Clone, Copy)]
struct Shape {
    width: u32,
    height: u32,
}

impl Shape {
    /// The same picture `width` pixels across, keeping its shape.
    fn scaled(self, width: u32) -> Self {
        Self {
            width,
            height: (u64::from(width) * u64::from(self.height) / u64::from(self.width)).max(1)
                as u32,
        }
    }

    fn pixels(self) -> u32 {
        self.width * self.height
    }

    fn aspect(self) -> f64 {
        f64::from(self.width) / f64::from(self.height)
    }

    /// Where pixel `index` sits in the picture, 0 to 1 across each axis.
    fn uv(self, index: u32) -> [f64; 2] {
        [
            (f64::from(index % self.width) + 0.5) / f64::from(self.width),
            (f64::from(index / self.width) + 0.5) / f64::from(self.height),
        ]
    }
}

impl Export {
    fn resampled(&self, to: Shape) -> Vec<f64> {
        let step = (
            f64::from(self.shape.width) / f64::from(to.width),
            f64::from(self.shape.height) / f64::from(to.height),
        );
        (0..to.pixels())
            .map(|index| {
                let (x, y) = (index % to.width, index / to.width);
                let (sx, sy) = (
                    (f64::from(x) * step.0) as usize,
                    (f64::from(y) * step.1) as usize,
                );
                self.luma[sy * self.shape.width as usize + sx]
            })
            .collect()
    }

    /// The same picture at `to`, each output pixel the MEAN of the input
    /// pixels it covers.
    ///
    /// [`Self::resampled`] point-samples, which is right for a target the
    /// solve reads at nearly its own resolution and wrong for a search that
    /// looks at a 1920-wide export 48 pixels across: point-sampling a factor
    /// of forty is aliasing, and two differently aliased pictures do not
    /// correlate on the content they share. This is the coarse search's own
    /// reader for that reason.
    fn averaged(&self, to: Shape) -> Vec<f64> {
        let (width, height) = (self.shape.width as usize, self.shape.height as usize);
        let step = (
            f64::from(self.shape.width) / f64::from(to.width),
            f64::from(self.shape.height) / f64::from(to.height),
        );
        (0..to.pixels())
            .map(|index| {
                let (x, y) = (index % to.width, index / to.width);
                let x0 = (f64::from(x) * step.0) as usize;
                let y0 = (f64::from(y) * step.1) as usize;
                let x1 = ((f64::from(x + 1) * step.0) as usize).clamp(x0 + 1, width);
                let y1 = ((f64::from(y + 1) * step.1) as usize).clamp(y0 + 1, height);
                let mut total = 0.0;
                for row in y0..y1 {
                    for column in x0..x1 {
                        total += self.luma[row * width + column];
                    }
                }
                total / ((x1 - x0) * (y1 - y0)) as f64
            })
            .collect()
    }
}

fn export_frame(path: &Path, from: f64) -> Fallible<Export> {
    let probe = ffprobe_size(path)?;
    let mut walk = Walk::open(path, from, Size::new(probe.width, probe.height))?;
    let pair = walk
        .next_pair()?
        .ok_or("no frame decoded from the export")?;
    let plane = &pair.lenses[0];
    let luma = (0..probe.height as usize)
        .flat_map(|y| (0..probe.width as usize).map(move |x| (x, y)))
        .map(|(x, y)| f64::from(plane.luma[y * plane.stride + x]))
        .collect();
    Ok(Export {
        shape: Shape {
            width: probe.width,
            height: probe.height,
        },
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
fn panini_ray(uv: [f64; 2], half_fov: f64, d: f64, aspect: f64) -> [f64; 3] {
    // Their own shader: s = (d+1)/(d+cos lambda), x = s sin lambda, y = s tan phi,
    // scaled so that the picture's half width is the half field of view asked for.
    let edge = (d + 1.0) / (d + half_fov.cos()) * half_fov.sin();
    let (x, y) = (
        (uv[0] * 2.0 - 1.0) * edge,
        (uv[1] * 2.0 - 1.0) * edge / aspect,
    );
    // x (d + cos l) = (d+1) sin l, which is one sinusoid in l and so has a closed inverse.
    let radius = ((d + 1.0).powi(2) + x * x).sqrt();
    let lambda = (x * d / radius).clamp(-1.0, 1.0).asin() + x.atan2(d + 1.0);
    let s = (d + 1.0) / (d + lambda.cos());
    let phi = (y / s).atan();
    [
        lambda.sin() * phi.cos(),
        phi.sin(),
        lambda.cos() * phi.cos(),
    ]
}

fn ray_of(uv: [f64; 2], half_fov: f64, compression: f64, aspect: f64) -> [f64; 3] {
    let (u, v) = (uv[0] * 2.0 - 1.0, (uv[1] * 2.0 - 1.0) / aspect);
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
fn looked(
    lenses: &[Lens],
    frame: Size,
    view: Look,
    pair: &Pair,
    shape: Shape,
    cells: &[kjerag_render::Cell],
    rolling: Option<kjerag_render::Rolling>,
) -> Vec<f64> {
    let reframe = Reframe::new(
        lenses,
        frame,
        Camera::default(),
        Held {
            body_from_world: orientation(view.angles),
            rolling,
        },
        1.0,
        false,
        Sampling::default(),
    );
    (0..shape.pixels())
        .map(|index| {
            let ray = view.ray(shape.uv(index), shape.aspect());
            let (weights, landings) = Weighting::Shipped.bent(&reframe, ray, cells);
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
    /// Insta360 Studio's "Distortion" slider, which is Panini's `d`, or a
    /// negative number for a projection nobody has told us. Zero is exactly
    /// rectilinear, because their d=0 collapses to gnomonic. A view given a
    /// `d` is given its field of view with it, so neither is fitted and only
    /// the three angles are searched.
    panini: f64,
}

impl Look {
    /// The five numbers as one vector, so the pattern search can step any of
    /// them without knowing which is which.
    fn ray(&self, uv: [f64; 2], aspect: f64) -> [f64; 3] {
        let half = self.fov.to_radians() / 2.0;
        match self.panini >= 0.0 {
            true => panini_ray(uv, half, self.panini, aspect),
            false => ray_of(uv, half, self.compression, aspect),
        }
    }

    fn nudge(mut self, axis: usize, step: f64) -> Self {
        match axis {
            0..=2 => self.angles[axis] += step,
            3 => self.fov += step,
            _ if self.panini >= 0.0 => {}
            _ => self.compression = (self.compression + step * 0.005).clamp(0.05, 1.0),
        }
        self
    }
}

/// How far every pixel of that view is from the seam, in degrees.
fn seam_map(lenses: &[Lens], frame: Size, view: Look, shape: Shape) -> Vec<f64> {
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
    (0..shape.pixels())
        .map(|index| past_seam(&reframe, view.ray(shape.uv(index), shape.aspect())))
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

// ------------------------------------- the label-free aim and scale search
//
// docs/research/parity-protocol.md section 8 called this the blocker: the
// solve is local, it needs to start within about fifteen degrees of the true
// scale and a search radius of the true aim, and on a real Studio export
// nothing got it there. Two things were wrong, and both of them are here.
//
// **The objective was the picture and not its structure.** A zero-mean
// correlation between two whole pictures is carried by the biggest thing in
// both, which outdoors is the sky-to-ground ramp, and that ramp is in the
// frame whatever the view is pointed at. It scored 0.7366 at the pose and
// 0.7380 half a degree off it -- a ladder with the wrong sign, which is what
// "flat" meant. [`Detail`] takes a local mean out of both pictures before
// they are compared, and what is left is the only part of a picture that
// knows where it is.
//
// **The search believed the label.** Their FOV number was used to centre the
// scale ladder, so a label that means something else put the search in the
// wrong basin and, on the tiny planet, walked it into the top of its own
// window. The sweep here is over an absolute range of OUR family's own field
// of view, from `fovlo` to the projection's own pole, and their number is
// read at the end to say what it turned out to mean.

/// A picture with its low frequencies taken out, and a record of where it has
/// any content at all.
///
/// The mask is not a detail. Our render leaves a pixel at zero where no lens
/// reached it, and a Panini view wide enough to be a tiny planet has corners
/// like that; correlating those against Studio's real corners would score the
/// shape of our own coverage.
struct Detail {
    value: Vec<f64>,
    valid: Vec<bool>,
    /// Where the picture has enough local contrast to say anything at all.
    ///
    /// Only the TARGET's copy of this is read, and it is the second half of
    /// the discrimination problem. Half of an outdoor frame is sky, sky is
    /// sensor noise once the local mean is out of it, and a correlation that
    /// includes it is being asked to match noise to noise: every aim scores
    /// the same nothing there, which dilutes the part of the score that
    /// separates aims. Scoring only where their picture has content is what
    /// makes a wrong aim pay for putting our sky over their hillside.
    textured: Vec<bool>,
}

/// How much local contrast a neighbourhood has to have before it is believed,
/// in luma codes.
///
/// The normalization below divides by the local spread, which is what stops
/// one strong edge from being the whole correlation. Divided by nothing it
/// would also amplify a flat patch of sky into pure sensor noise and put that
/// in the sum with the same weight as a hillside. Four codes is a little above
/// this camera's noise at these exposures, so flat stays near zero and texture
/// comes up to one.
const CONTRAST_FLOOR: f64 = 4.0;

impl Detail {
    /// A picture reduced to local contrast: the neighbourhood mean taken out,
    /// and what is left divided by the neighbourhood's own spread.
    ///
    /// Both halves are load bearing, and each was measured against the export
    /// it was written for.
    ///
    /// - **The mean out** is the difference between a peak and the flat 0.7
    ///   the protocol's section 8 recorded: a raw correlation is carried by
    ///   the sky-to-ground ramp, which is in the frame at every aim.
    /// - **The spread out** is the difference between a peak that stands
    ///   0.03 above its best rival and one that stands 0.2 above it. What is
    ///   left after the ramp goes is the horizon EDGE, which is one line, and
    ///   any aim that puts a horizon at the same height scores nearly as well
    ///   as the right one. Dividing by the local spread caps that edge at the
    ///   same weight as a square of hillside, and hillside is the part of a
    ///   picture that knows which hillside it is.
    fn of(luma: &[f64], shape: Shape, fraction: f64, texture: f64) -> Self {
        let radius = ((f64::from(shape.width) * fraction).round() as usize).max(1);
        let valid: Vec<bool> = luma.iter().map(|code| *code > 0.0).collect();
        let (mean, spread) = local_moments(luma, &valid, shape, radius);
        let value = luma
            .iter()
            .zip(mean.iter().zip(&spread))
            .map(|(code, (mean, spread))| {
                (code - mean) / (spread * spread + CONTRAST_FLOOR * CONTRAST_FLOOR).sqrt()
            })
            .collect();
        let textured = valid
            .iter()
            .zip(&spread)
            .map(|(valid, spread)| *valid && *spread >= texture)
            .collect();
        Self {
            value,
            valid,
            textured,
        }
    }

    fn textured_pixels(&self) -> usize {
        self.textured.iter().filter(|on| **on).count()
    }
}

/// The mean and the standard deviation of each pixel's own neighbourhood,
/// over the pixels that have content in them, by summed-area table so the
/// radius costs nothing.
fn local_moments(
    luma: &[f64],
    valid: &[bool],
    shape: Shape,
    radius: usize,
) -> (Vec<f64>, Vec<f64>) {
    let (width, height) = (shape.width as usize, shape.height as usize);
    let stride = width + 1;
    let mut sum = vec![0.0; stride * (height + 1)];
    let mut squares = vec![0.0; stride * (height + 1)];
    let mut count = vec![0.0; stride * (height + 1)];
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let (code, one) = match valid[index] {
                true => (luma[index], 1.0),
                false => (0.0, 0.0),
            };
            for (table, value) in [
                (&mut sum, code),
                (&mut squares, code * code),
                (&mut count, one),
            ] {
                table[(y + 1) * stride + x + 1] =
                    value + table[y * stride + x + 1] + table[(y + 1) * stride + x]
                        - table[y * stride + x];
            }
        }
    }
    let area = |table: &[f64], x0: usize, y0: usize, x1: usize, y1: usize| {
        table[y1 * stride + x1] + table[y0 * stride + x0]
            - table[y0 * stride + x1]
            - table[y1 * stride + x0]
    };
    let mut means = vec![0.0; width * height];
    let mut spreads = vec![0.0; width * height];
    for index in 0..width * height {
        let (x, y) = (index % width, index / width);
        let x0 = x.saturating_sub(radius);
        let y0 = y.saturating_sub(radius);
        let x1 = (x + radius + 1).min(width);
        let y1 = (y + radius + 1).min(height);
        let n = area(&count, x0, y0, x1, y1);
        if n <= 0.0 {
            continue;
        }
        let mean = area(&sum, x0, y0, x1, y1) / n;
        means[index] = mean;
        spreads[index] = (area(&squares, x0, y0, x1, y1) / n - mean * mean)
            .max(0.0)
            .sqrt();
    }
    (means, spreads)
}

/// Zero-mean normalized cross-correlation of two [`Detail`]s, over the pixels
/// both of them have content in.
fn agree_detail(ours: &Detail, theirs: &Detail, want: usize) -> f64 {
    let mut n = 0.0;
    let (mut sum_a, mut sum_b) = (0.0, 0.0);
    for index in 0..ours.value.len().min(theirs.value.len()) {
        if !(ours.valid[index] && theirs.textured[index]) {
            continue;
        }
        n += 1.0;
        sum_a += ours.value[index];
        sum_b += theirs.value[index];
    }
    // Half of their content is the floor. A pose whose coverage barely
    // overlaps theirs can correlate perfectly on the sliver that does, and a
    // sliver is not a registration.
    if n < 0.5 * want as f64 {
        return -1.0;
    }
    let (mean_a, mean_b) = (sum_a / n, sum_b / n);
    let (mut covariance, mut var_a, mut var_b) = (0.0, 0.0, 0.0);
    for index in 0..ours.value.len().min(theirs.value.len()) {
        if !(ours.valid[index] && theirs.textured[index]) {
            continue;
        }
        let (a, b) = (ours.value[index] - mean_a, theirs.value[index] - mean_b);
        covariance += a * b;
        var_a += a * a;
        var_b += b * b;
    }
    match var_a > 0.0 && var_b > 0.0 {
        true => covariance / (var_a * var_b).sqrt(),
        false => -1.0,
    }
}

/// One candidate for what an export is a picture of: where it points, how
/// wide it is in OUR family's terms, and which Panini it is drawn in.
#[derive(Clone, Copy, Debug)]
struct Aim {
    angles: [f64; 3],
    fov: f64,
    d: f64,
}

impl Aim {
    /// The projection, as one number covering both families.
    ///
    /// Zero and up is Panini's `d`, which is what Studio's "Distortion"
    /// slider is on a flat reframe. **Below zero is the compression family**,
    /// `c = -d`: `theta = atan(rho tan(c H))/c`, rectilinear at `c = 1`,
    /// equidistant in the limit at `c = 0`, and **stereographic at exactly
    /// `c = 0.5`**.
    ///
    /// The second family is not decoration. Their tiny planet's horizon is a
    /// CIRCLE in the delivered picture -- measured on the July-14 export at
    /// 360 s, 721.5 pixels of radius across against 717.5 down, which is
    /// round to within 0.6 percent -- and Panini cannot draw a circular
    /// horizon at a nadir aim whatever `d` is: with the axis down, `y = s tan
    /// phi` sends the horizon to infinity up and down the picture and leaves
    /// it as two straight verticals at `lambda = +/-90`. A search that only
    /// had Panini in it could not draw their picture at any scale, which is
    /// why the field of view walked into the top of its own window instead of
    /// finding a peak. This family is radially symmetric by construction and
    /// draws a circular horizon at every `c`.
    fn look(self) -> Look {
        Look {
            angles: self.angles,
            fov: self.fov,
            compression: match self.d < 0.0 {
                true => -self.d,
                false => 1.0,
            },
            panini: match self.d < 0.0 {
                true => -1.0,
                false => self.d,
            },
        }
    }

    fn projection(self) -> String {
        match self.d < 0.0 {
            true => format!(
                "compression {:.3}{}",
                -self.d,
                match (-self.d - 0.5).abs() < 0.02 {
                    true => " (stereographic)",
                    false => "",
                }
            ),
            false => format!("panini d {:.3}", self.d),
        }
    }

    /// The widest this family can draw at this projection, less a margin.
    ///
    /// Panini's own pole: `s = (d+1)/(d+cos lambda)` runs to infinity at
    /// `lambda = acos(-d)`, so `d = 0` cannot draw 180 degrees and `d = 0.9`
    /// cannot draw 309. The compression family's pole is `c H = 90 deg`, so
    /// `c = 0.5` cannot draw past 360. A search allowed past either is a
    /// search that will report a number from beyond the edge of its own
    /// projection, which is what the saturated 328 in section 8 of the
    /// protocol was.
    fn ceiling(d: f64) -> f64 {
        match d < 0.0 {
            true => 0.96 * 180.0 / -d,
            false => 0.96 * 2.0 * (-d).clamp(-1.0, 1.0).acos().to_degrees(),
        }
    }

    fn nudged(mut self, axis: usize, step: f64) -> Option<Self> {
        match axis {
            0..=2 => self.angles[axis] += step,
            3 => self.fov *= (step / 50.0).exp(),
            // The two families do not meet except at Panini 0 = compression
            // 1, so a step never crosses between them: a search that changed
            // family mid-polish would be comparing two answers and reporting
            // one.
            // Bounded, because below about sixty degrees of view the
            // projection is not identifiable at all: every smooth radial map
            // is linear over a small enough picture, so the refinement will
            // trade `d` against the field of view without the score moving.
            // Unbounded it reported Panini d = 9.4 on a twenty degree view,
            // which is not a projection anybody ships -- it is the search
            // telling us, in the only way it can, that this view does not
            // constrain the number.
            _ if self.d < 0.0 => self.d = (self.d + step / 40.0).clamp(-1.0, -0.3),
            _ => self.d = (self.d + step / 40.0).clamp(0.0, 2.5),
        }
        (self.fov > 4.0 && self.fov < Self::ceiling(self.d)).then_some(self)
    }
}

/// How far apart two aims point, as one angle: the rotation that takes one to
/// the other.
fn apart(a: &Aim, b: &Aim) -> f64 {
    norm(
        orientation(a.angles)
            .conjugate()
            .times(orientation(b.angles))
            .rotation_vector(),
    )
    .to_degrees()
}

/// What a whole sweep's scores looked like, so "there is a peak" is a claim
/// about a distribution rather than about one number.
#[derive(Clone)]
struct Spread {
    bins: Vec<u64>,
    total: u64,
}

impl Spread {
    fn new() -> Self {
        Self {
            bins: vec![0; 400],
            total: 0,
        }
    }

    fn add(&mut self, score: f64) {
        let at = (((score + 1.0) * 200.0) as isize).clamp(0, 399) as usize;
        self.bins[at] += 1;
        self.total += 1;
    }

    fn merge(&mut self, other: &Self) {
        for (mine, theirs) in self.bins.iter_mut().zip(&other.bins) {
            *mine += theirs;
        }
        self.total += other.total;
    }

    /// The score `share` of the sweep came in under, to the bin's own width.
    fn quantile(&self, share: f64) -> f64 {
        let want = (share * self.total as f64) as u64;
        let mut seen = 0;
        for (at, count) in self.bins.iter().enumerate() {
            seen += count;
            if seen >= want {
                return f64::from(at as u32) / 200.0 - 1.0;
            }
        }
        1.0
    }
}

/// How finely the sphere is stepped at a given scale: an eighth of what the
/// view can see, which is what a 16 pixel sweep picture can capture.
fn sweep_step(fov: f64) -> f64 {
    (fov / 8.0).clamp(1.5, 20.0)
}

/// The sphere sweep: every aim on a grid whose step is a fraction of what the
/// candidate view can see, at every scale in the window, at every `d` asked
/// for.
///
/// The step is not a constant. A fifteen degree grid finds nothing in a
/// twenty degree view and is a waste of a day in a three hundred degree one,
/// so it is the field of view over five, and the roll step is the aim step
/// divided by the sine of the view's own angular radius -- which is what a
/// roll displaces the rim by.
///
/// **The shortlist is kept per scale, not per sweep.** One scale scoring a
/// little better at 48 pixels across would otherwise fill the whole list and
/// the refinement would never visit the others, which is a search that has
/// decided the answer at the resolution least able to decide it. Every scale
/// in the window comes out of here with its own best aims, and the scale
/// profile printed later is what that buys: a curve with a peak in it, rather
/// than a number.
#[allow(clippy::too_many_arguments)]
fn sphere_sweep(
    lenses: &[Lens],
    frame: Size,
    ours: &Pair,
    cells: &[kjerag_render::Cell],
    theirs: &Detail,
    shape: Shape,
    hp: f64,
    texture: f64,
    scales: &[(f64, f64)],
    keep: usize,
) -> (Vec<Vec<(f64, Aim)>>, Spread) {
    let mut work: Vec<(usize, f64, f64)> = Vec::new();
    for (which, (fov, _)) in scales.iter().enumerate() {
        let step = sweep_step(*fov);
        let turns = (360.0 / step).round().max(1.0) as i32;
        for turn in 0..turns {
            work.push((which, f64::from(turn) * 360.0 / f64::from(turns), 0.0));
        }
    }
    let want = theirs.textured_pixels();
    let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let mut found: Vec<Vec<(f64, Aim)>> = vec![Vec::new(); scales.len()];
    let mut spread = Spread::new();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for lane in 0..threads {
            let (work, scales) = (&work, &scales);
            handles.push(scope.spawn(move || {
                let mut best: Vec<Vec<(f64, Aim)>> = vec![Vec::new(); scales.len()];
                let mut seen = Spread::new();
                for (which, yaw, _) in work.iter().copied().skip(lane).step_by(threads) {
                    let (fov, d) = scales[which];
                    let step = sweep_step(fov);
                    let rolls =
                        (step / (fov.to_radians() / 2.0).sin().abs().max(0.05)).clamp(6.0, 60.0);
                    let tilts = (180.0 / step).round().max(1.0) as i32;
                    let turns = (360.0 / rolls).round().max(1.0) as i32;
                    for tilt in 0..=tilts {
                        let pitch = -90.0 + f64::from(tilt) * 180.0 / f64::from(tilts);
                        for turn in 0..turns {
                            let roll = f64::from(turn) * 360.0 / f64::from(turns) - 180.0;
                            let aim = Aim {
                                angles: [yaw, pitch, roll],
                                fov,
                                d,
                            };
                            let picture = looked(lenses, frame, aim.look(), ours, shape, cells, None);
                            let score = agree_detail(
                                &Detail::of(&picture, shape, hp, texture),
                                theirs,
                                want,
                            );
                            seen.add(score);
                            best[which].push((score, aim));
                            if best[which].len() > 8 * keep {
                                best[which].sort_by(|a, b| b.0.total_cmp(&a.0));
                                best[which].truncate(keep);
                            }
                        }
                    }
                }
                (best, seen)
            }));
        }
        for handle in handles {
            let (best, seen) = handle.join().expect("a sweep lane panicked");
            for (mine, theirs) in found.iter_mut().zip(best) {
                mine.extend(theirs);
            }
            spread.merge(&seen);
        }
    });
    for list in &mut found {
        list.sort_by(|a, b| b.0.total_cmp(&a.0));
        list.truncate(4 * keep);
    }
    (found, spread)
}
/// The best candidates that are not each other: a peak covered by a hundred
/// of its own neighbours has no rival in the list to be measured against.
fn spread_out(candidates: &[(f64, Aim)], apart_deg: f64, want: usize) -> Vec<(f64, Aim)> {
    let mut kept: Vec<(f64, Aim)> = Vec::new();
    for (score, aim) in candidates.iter().copied() {
        if kept
            .iter()
            .any(|(_, held)| apart(held, &aim) < apart_deg && (held.fov / aim.fov).ln().abs() < 0.2)
        {
            continue;
        }
        kept.push((score, aim));
        if kept.len() >= want {
            break;
        }
    }
    kept
}

/// Pattern search on the five numbers, halving its step until it is finer
/// than `floor`.
///
/// `wide` scores the ten candidate steps of a round in parallel and takes the
/// best of them rather than the first that improves. Both are the same search
/// in the limit; the parallel one is what finishes at a picture wide enough to
/// resolve a tiny planet, where one score is a million rays. It is off when
/// the CALLER is already the parallel one -- a pattern search inside a lane
/// that is itself a lane spawns twelve threads inside twelve and spends the
/// run in the scheduler rather than in the picture.
fn polish(
    start: Aim,
    step: f64,
    floor: f64,
    axes: usize,
    wide: bool,
    score: &(impl Fn(Aim) -> f64 + Sync),
) -> (f64, Aim) {
    let mut best = (score(start), start);
    let mut step = step;
    while step > floor {
        let tries: Vec<Aim> = (0..axes)
            .flat_map(|axis| [1.0, -1.0].map(|sign| best.1.nudged(axis, sign * step)))
            .flatten()
            .collect();
        let scored: Vec<(f64, Aim)> = match wide {
            false => tries.iter().map(|aim| (score(*aim), *aim)).collect(),
            true => {
                let mut all = Vec::new();
                std::thread::scope(|scope| {
                    let mut handles = Vec::new();
                    for chunk in tries.chunks(2) {
                        handles.push(scope.spawn(move || {
                            chunk
                                .iter()
                                .map(|aim| (score(*aim), *aim))
                                .collect::<Vec<_>>()
                        }));
                    }
                    for handle in handles {
                        all.extend(handle.join().expect("a polish lane panicked"));
                    }
                });
                all
            }
        };
        match scored
            .into_iter()
            .fold(None::<(f64, Aim)>, |best, one| match best {
                Some(held) if held.0 >= one.0 => Some(held),
                _ => Some(one),
            }) {
            Some(found) if found.0 > best.0 => best = found,
            _ => step *= 0.5,
        }
    }
    best
}

/// What one instant's registration came to.
struct Registered {
    at: f64,
    aim: Aim,
    peak: f64,
    rival: f64,
    /// The rotation the export's view is at in the WORLD, which is the frame
    /// their stabilizer works in and the only frame two instants can be
    /// compared in.
    world: Option<Quat>,
    /// The file's own orientation at this instant, in the same three angles.
    /// What says whether their heading is fixed in the world or follows the
    /// camera, which is what Direction Lock decides.
    track: [f64; 3],
    ladder: Vec<(f64, [f64; 3])>,
    scale_ladder: Vec<(f64, f64)>,
    saturated: bool,
    /// What share of THEIR frame carried enough local contrast to be scored.
    /// The protocol's section 7 says a frame can fail on content; this is the
    /// number that says so before the gates do.
    textured: f64,
}

impl Registered {
    fn at_off(&self, axis: usize, off: f64) -> f64 {
        self.ladder
            .iter()
            .find(|(at, _)| (at - off).abs() < 1e-9)
            .map_or(f64::MIN, |(_, scores)| scores[axis])
    }

    /// The worse of the two sides at `off`, which is the number a gate about
    /// "beats plus or minus" is asking for.
    fn beside(&self, axis: usize, off: f64) -> f64 {
        self.at_off(axis, off).max(self.at_off(axis, -off))
    }
}

fn frame_of(path: &Path, at: f64) -> Fallible<(CalibrationSet, Pair)> {
    let calibration = CalibrationSet::from_insv(path)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let mut walk = Walk::open(path, at, frame)?;
    if walk.streams() < 2 {
        return Err("this file carries one lens stream, so it has no seam".into());
    }
    let pair = walk.next_pair()?.ok_or("no frame decoded")?;
    Ok((calibration, pair))
}

/// Yaw, pitch and roll back out of the rotation [`orientation`] builds, so a
/// world-frame aim can be printed in the same three numbers a body-frame one
/// is.
fn angles_of(q: Quat) -> [f64; 3] {
    let rows = q.matrix().rows();
    // orientation() composes rot_y(yaw) rot_x(pitch) rot_z(roll), whose
    // middle row is [cos(pitch) sin(roll), cos(pitch) cos(roll), -sin(pitch)]
    // and whose last column is [sin(yaw) cos(pitch), -sin(pitch),
    // cos(yaw) cos(pitch)]. `the_world_frame_survives_a_round_trip` is the
    // check on that reading rather than this comment.
    let pitch = (-rows[1][2]).clamp(-1.0, 1.0).asin();
    let (yaw, roll) = match rows[1][2].abs() < 0.9999 {
        true => (rows[0][2].atan2(rows[2][2]), rows[1][0].atan2(rows[1][1])),
        // Straight up or straight down is a gimbal lock: yaw and roll are one
        // rotation there and only their sum is a number.
        false => (rows[2][0].atan2(rows[0][0]), 0.0),
    };
    [yaw.to_degrees(), pitch.to_degrees(), roll.to_degrees()]
}

fn register(options: &Options) -> Fallible<()> {
    let theirs_path = options
        .against
        .clone()
        .ok_or("register wants against=<export.mp4>")?;
    let instants = match options.instants.is_empty() {
        true => vec![options.from],
        false => options.instants.clone(),
    };
    let scales_of = |d: f64| -> Vec<f64> {
        let top = match options.fovhi > 0.0 {
            true => options.fovhi.min(Aim::ceiling(d)),
            false => Aim::ceiling(d),
        };
        let mut all = Vec::new();
        let mut fov = options.fovlo;
        while fov < top {
            all.push(fov);
            fov *= options.ratio;
        }
        all
    };
    // Both families, generously, whatever the export was labelled: Panini for
    // a flat reframe and the compression family for anything that draws a
    // round horizon. Panini 0 and compression 1 are the same projection, so
    // only one of them is in the list.
    let ds: Vec<f64> = match options.dset.is_empty() {
        false => options.dset.clone(),
        true => vec![0.0, 0.45, 0.9, 1.35, -0.85, -0.7, -0.6, -0.5, -0.42, -0.35],
    };
    let scales: Vec<(f64, f64)> = ds
        .iter()
        .flat_map(|d| scales_of(*d).into_iter().map(move |fov| (fov, *d)))
        .collect();

    println!(
        "register: {} against {}",
        options
            .input
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
        theirs_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
    );
    println!(
        "search:   fov {:.1} to {:.1} deg stepped by {:.2}, {} scales in all\n\
         search:   projections {}\n\
         search:   the aim grid steps a view over eight, high pass {:.0} percent of the \n\
         search:   width at the sweep\n\
         labels:   Studio says fov {:.1}, distortion {:.2} -- READ AT THE END, \
         BELIEVED NOWHERE",
        options.fovlo,
        scales.iter().map(|(fov, _)| *fov).fold(0.0, f64::max),
        options.ratio,
        scales.len(),
        ds.iter()
            .map(|d| Aim {
                angles: [0.0; 3],
                fov: 0.0,
                d: *d,
            }
            .projection())
            .collect::<Vec<_>>()
            .join(", "),
        options.hpc * 100.0,
        options.fov,
        options.panini,
    );

    let mut runs: Vec<Registered> = Vec::new();
    for at in instants.iter().copied() {
        let started = std::time::Instant::now();
        let (calibration, ours) = frame_of(&options.input, at)?;
        let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
        let lenses = options.corrected(
            std::slice::from_ref(&options.input),
            &calibration.lenses,
            frame,
        );
        let track = calibration.orientation(Filter::default());
        let export = export_frame(&theirs_path, at - options.lag)?;

        let swept = export.shape.scaled(options.sweepw);
        let coarse = export.shape.scaled(options.coarse);
        let middle = export.shape.scaled(options.mid);
        let fine = export.shape.scaled(options.size);
        let theirs_swept = Detail::of(&export.averaged(swept), swept, options.hpc, options.texture);
        let theirs_coarse = Detail::of(
            &export.averaged(coarse),
            coarse,
            options.hpc,
            options.texture,
        );
        let theirs_middle = Detail::of(
            &export.averaged(middle),
            middle,
            options.hp,
            options.texture,
        );
        let theirs_fine = Detail::of(&export.averaged(fine), fine, options.hp, options.texture);
        let textured = theirs_fine.textured_pixels() as f64 / fine.pixels() as f64;
        let at_shape = |aim: Aim, shape: Shape, theirs: &Detail| {
            let picture = looked(&lenses, frame, aim.look(), &ours, shape, &options.band, None);
            let pass = match shape.width <= coarse.width {
                true => options.hpc,
                false => options.hp,
            };
            agree_detail(
                &Detail::of(&picture, shape, pass, options.texture),
                theirs,
                theirs.textured_pixels(),
            )
        };

        let axes = match ds.len() > 1 {
            true => 5,
            false => 4,
        };
        let seeded = options.seed.len() >= 5;
        let heading = options.told.len() >= 3;
        let (candidates, spread) = match (seeded, heading) {
            // ---- their own pan and tilt, and the one angle between us
            //
            // The sweep is over the heading datum alone: the angle the file's
            // orientation track started counting from, which is arbitrary,
            // constant for the file, and the only thing standing between a
            // told pan and a body aim. Every scale in the window is swept with
            // it, because their field of view number is still a label.
            (false, true) => {
                let at = (ours.at.as_secs_f64() * 1e6).round() as i64;
                // With the lock ON their view is fixed in the world and this
                // is what carries it into the body frame the search works in.
                // With it OFF the view is fixed in the body already and the
                // IMU has no business in the aim at all, so the carrier is
                // the identity and the sweep is over the body's own heading.
                let body = match options.lock {
                    Lock::World => track.at(at).conjugate(),
                    Lock::Body => Quat::default(),
                };
                // The datum steps a degree because the sweep picture is 16
                // across and a correlation reaches about one of its pixels,
                // which on a 20 degree view is 1.25 degrees. Their tilt and
                // roll were measured to be ours to within two degrees, so
                // those are walked over a four degree box at the same step
                // rather than taken exactly: told is not the same as true.
                let mut per_scale: Vec<Vec<(f64, Aim)>> = vec![Vec::new(); scales.len()];
                let mut seen = Spread::new();
                for (which, (fov, d)) in scales.iter().enumerate() {
                    for step in 0..360 {
                        for tilt in -4..=4 {
                            for roll in -4..=4 {
                                let world = orientation([
                                    options.told[0] + f64::from(step),
                                    options.told[1] + f64::from(tilt),
                                    options.told[2] + f64::from(roll),
                                ]);
                                let aim = Aim {
                                    angles: angles_of(body.times(world)),
                                    fov: *fov,
                                    d: *d,
                                };
                                let score = at_shape(aim, swept, &theirs_swept);
                                seen.add(score);
                                per_scale[which].push((score, aim));
                            }
                        }
                    }
                    per_scale[which].sort_by(|a, b| b.0.total_cmp(&a.0));
                    per_scale[which].truncate(options.pool);
                }
                (per_scale, seen)
            }
            (true, _) => (
                vec![vec![(
                    0.0,
                    Aim {
                        angles: [options.seed[0], options.seed[1], options.seed[2]],
                        fov: options.seed[3],
                        d: options.seed[4],
                    },
                )]],
                Spread::new(),
            ),
            _ => sphere_sweep(
                &lenses,
                frame,
                &ours,
                &options.band,
                &theirs_coarse,
                coarse,
                options.hp,
                options.texture,
                &scales,
                options.keepn,
            ),
        };
        let coarse_peak = candidates
            .iter()
            .filter_map(|list| list.first().map(|(score, _)| *score))
            .fold(-1.0, f64::max);
        let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
        let lanes = |seeds: Vec<(usize, Aim)>,
                     run: &(dyn Fn(usize, Aim) -> (f64, Aim, usize) + Sync)| {
            let mut done: Vec<(f64, Aim, usize)> = Vec::new();
            std::thread::scope(|scope| {
                let mut handles = Vec::new();
                for lane in 0..threads {
                    let seeds = &seeds;
                    handles.push(scope.spawn(move || {
                        seeds
                            .iter()
                            .copied()
                            .skip(lane)
                            .step_by(threads)
                            .map(|(which, aim)| run(which, aim))
                            .collect::<Vec<_>>()
                    }));
                }
                for handle in handles {
                    done.extend(handle.join().expect("a refinement lane panicked"));
                }
            });
            done
        };

        // ---- the grid's own phase, taken out at the resolution it was swept
        //
        // The sphere grid steps by a fifth of what a view can see, so the best
        // grid point can sit half a step -- two degrees on a twenty degree
        // view -- off the answer. At 48 pixels across, with the high pass
        // holding structure down to six of them, two degrees is most of the
        // agreement: measured at 360 s on the creek, the export's own answer
        // scores 0.9312 when it is looked at and 0.469 at the nearest point
        // the grid actually visited, which is not enough to outrank a second
        // basin that happened to land on a grid point. Every shortlisted aim
        // therefore gets a local grid of its own, five deep in each angle at
        // two fifths of the sweep's step and three in scale. It is 375 scores
        // of 1300 rays each, which is nothing beside the sweep that produced
        // the seed, and it is a GRID rather than a hill climb on purpose: a
        // pattern search started off a peak walks down whichever side it
        // tried first, and one of those walks is what reported fov 16.9 at
        // 0.745 for an export whose answer is 19.6 at 0.95.
        let mut caught: Vec<(f64, Aim, usize)> = candidates
            .iter()
            .enumerate()
            .flat_map(|(which, list)| list.iter().map(move |(score, aim)| (*score, *aim, which)))
            .collect();
        caught.sort_by(|a, b| b.0.total_cmp(&a.0));
        let densified = lanes(
            {
                let ranked: Vec<(f64, Aim)> = caught
                    .iter()
                    .map(|(score, aim, _)| (*score, *aim))
                    .collect();
                let mut chosen: Vec<(usize, Aim)> = spread_out(&ranked, 2.0, options.pool)
                    .into_iter()
                    .map(|(_, aim)| {
                        let which = caught
                            .iter()
                            .find(|(_, other, _)| {
                                other.fov == aim.fov && other.angles == aim.angles
                            })
                            .map_or(0, |(_, _, at)| *at);
                        (which, aim)
                    })
                    .collect();
                // Two per scale on top of the pool, so no scale in the window
                // goes into the profile unrepresented however the pool fell.
                for (which, list) in candidates.iter().enumerate() {
                    chosen.extend(
                        spread_out(list, 4.0, 2)
                            .into_iter()
                            .map(|(_, aim)| (which, aim)),
                    );
                }
                chosen
            },
            &|which, aim| {
                let step = sweep_step(aim.fov) * 0.5;
                let roll = (step / (aim.fov.to_radians() / 2.0).sin().abs().max(0.05)).min(20.0);
                let mut best = (at_shape(aim, coarse, &theirs_coarse), aim);
                for turn in -1..=1 {
                    for tilt in -1..=1 {
                        for spin in -1..=1 {
                            for scale in [-1.0, 0.0, 1.0] {
                                let mut here = aim;
                                here.angles[0] += f64::from(turn) * step;
                                here.angles[1] += f64::from(tilt) * step;
                                here.angles[2] += f64::from(spin) * roll;
                                here.fov *= (scale * 0.05f64).exp();
                                if here.fov >= Aim::ceiling(here.d) {
                                    continue;
                                }
                                let score = at_shape(here, coarse, &theirs_coarse);
                                if score > best.0 {
                                    best = (score, here);
                                }
                            }
                        }
                    }
                }
                (best.0, best.1, which)
            },
        );

        // ---- what goes on to the middle width, and why it is two lists
        //
        // ONE PER SCALE, so the profile below is a fair comparison: every
        // scale in the window is judged on what it can be made to do, not on
        // where a grid point happened to fall in it.
        //
        // AND THE BEST OF ALL OF THEM TOGETHER, which is what actually finds
        // the answer. A correlation between two band-passed pictures falls to
        // half at a shift of about a sixth of the finest structure in them, so
        // at 48 pixels across a twenty degree view the sweep's capture radius
        // is under a degree while its grid steps four: measured at 360 s on
        // the creek, the export's own answer scores 0.946 where it stands and
        // 0.604 at the best point the grid visited anywhere. What saves it is
        // that the grid's PHASE differs from scale to scale -- each scale
        // steps by its own fifth -- so across a window of twenty scales one of
        // them lands near the answer even though its neighbours do not, and
        // the one that did is only visible in a ranking that puts all the
        // scales side by side. A per-scale ranking hides it behind its own
        // scale's second basin, which is exactly the run that reported fov
        // 11.2 at 0.489 for an export whose answer is 19.6 at 0.95.
        let mut everything: Vec<(f64, Aim, usize)> = densified.clone();
        everything.sort_by(|a, b| b.0.total_cmp(&a.0));
        let global: Vec<(f64, Aim)> = everything
            .iter()
            .map(|(score, aim, _)| (*score, *aim))
            .collect();
        let mut seeds: Vec<(usize, Aim)> = spread_out(&global, 4.0, options.keepn * 2)
            .into_iter()
            .map(|(_, aim)| {
                let which = everything
                    .iter()
                    .find(|(_, other, _)| other.fov == aim.fov && other.angles == aim.angles)
                    .map_or(0, |(_, _, at)| *at);
                (which, aim)
            })
            .collect();
        for which in 0..scales.len() {
            let mut mine: Vec<(f64, Aim)> = densified
                .iter()
                .filter(|(_, _, at)| *at == which)
                .map(|(score, aim, _)| (*score, *aim))
                .collect();
            mine.sort_by(|a, b| b.0.total_cmp(&a.0));
            seeds.extend(
                spread_out(&mine, 4.0, 2)
                    .into_iter()
                    .map(|(_, aim)| (which, aim)),
            );
        }
        let refined = lanes(seeds, &|which, aim| {
            let (score, landed) = polish(
                aim,
                (aim.fov / 8.0).clamp(2.0, 10.0),
                0.8,
                axes,
                false,
                &|a| at_shape(a, middle, &theirs_middle),
            );
            (score, landed, which)
        });
        let mut profile: Vec<(f64, f64, f64)> = scales
            .iter()
            .enumerate()
            .map(|(which, (fov, d))| {
                (
                    *fov,
                    *d,
                    refined
                        .iter()
                        .filter(|(_, _, at)| *at == which)
                        .map(|(score, _, _)| *score)
                        .fold(-1.0, f64::max),
                )
            })
            .collect();
        let mut middled: Vec<(f64, Aim)> = refined
            .iter()
            .map(|(score, aim, _)| (*score, *aim))
            .collect();
        middled.sort_by(|a, b| b.0.total_cmp(&a.0));

        let (peak, aim) = polish(middled[0].1, 0.4, 0.004, axes, true, &|a| {
            at_shape(a, fine, &theirs_fine)
        });
        // The rival: the best pose in the list that is NOT this one, run
        // through the same refinement so the two numbers are comparable, and
        // still not this one when the refinement has finished. A rival that
        // walked home during its own polish is the answer wearing a hat, and
        // scoring it as a rival would report a prominence of nothing.
        let mut rival = -1.0_f64;
        let mut rival_aim = None;
        let mut tried = 0;
        for (_, other) in &middled {
            if apart(other, &aim) <= 12.0 || tried >= 8 {
                continue;
            }
            tried += 1;
            let (score, landed) = polish(*other, 0.4, 0.02, axes, true, &|a| {
                at_shape(a, fine, &theirs_fine)
            });
            if apart(&landed, &aim) > 12.0 && score > rival {
                rival = score;
                rival_aim = Some(landed);
            }
        }
        // What each PROJECTION could be made to agree at, over the whole scale
        // window. On the tiny planet this is the finding: Panini's best, at
        // any d and any scale, is well below the compression family's, because
        // Panini cannot draw a round horizon at a nadir aim.
        let families: Vec<(f64, f64, f64)> = ds
            .iter()
            .map(|d| {
                let best = profile
                    .iter()
                    .filter(|(_, at, _)| (at - d).abs() < 1e-9)
                    .fold((0.0, f64::MIN), |held, (fov, _, score)| {
                        match *score > held.1 {
                            true => (*fov, *score),
                            false => held,
                        }
                    });
                (*d, best.0, best.1)
            })
            .collect();
        let home = middled[0].1.d;
        profile.retain(|(_, d, _)| (d - home).abs() < 1e-9);

        let ladder: Vec<(f64, [f64; 3])> = [
            -8.0, -4.0, -3.0, -2.0, -1.5, -1.0, -0.5, -0.25, -0.1, 0.1, 0.25, 0.5, 1.0, 1.5, 2.0,
            3.0, 4.0, 8.0,
        ]
        .into_iter()
        .map(|off| {
            let scores = std::array::from_fn(|axis| {
                let mut moved = aim;
                moved.angles[axis] += off;
                at_shape(moved, fine, &theirs_fine)
            });
            (off, scores)
        })
        .collect();
        // The scale's own ladder, which is the axis mode=solve cannot start
        // more than about fifteen degrees away from.
        let scale_ladder: Vec<(f64, f64)> =
            [0.75, 0.85, 0.92, 0.96, 0.98, 1.02, 1.04, 1.08, 1.18, 1.33]
                .into_iter()
                .map(|factor| {
                    let mut moved = aim;
                    moved.fov *= factor;
                    (factor, at_shape(moved, fine, &theirs_fine))
                })
                .collect();

        // The capture gate, per projection: a scale is against the edge of
        // the search if it is against the top of what was SWEPT for its own
        // projection, or against the pole of that projection itself, or back
        // against the bottom of the window. Read against the whole run's
        // widest scale it would let a narrow projection's answer hide behind
        // a wide one's window.
        // The widest scale the sweep actually visited, and the pole of the
        // projection the answer ended up in. Read over the WHOLE window and
        // not over the answer's own `d`: the refinement is free to move the
        // projection, and a window looked up by a `d` that is no longer in the
        // swept set comes back empty and refuses a run that was never near an
        // edge.
        let ceiling = Aim::ceiling(aim.d);
        let top = scales
            .iter()
            .map(|(fov, _)| *fov)
            .fold(options.fovlo, f64::max);
        let saturated =
            aim.fov > 0.985 * ceiling || aim.fov > 0.99 * top || aim.fov < 1.02 * options.fovlo;

        let world = options.world.then(|| {
            let us = (ours.at.as_secs_f64() * 1e6).round() as i64;
            track.at(us).times(orientation(aim.angles))
        });

        println!(
            "\n---- source {at:.3} s (frame at {:.3} s), export {:.3} s, {:.0} s of search",
            ours.at.as_secs_f64(),
            at - options.lag,
            started.elapsed().as_secs_f64(),
        );
        println!(
            "peak:     yaw {:+8.3}, pitch {:+8.3}, roll {:+8.3} deg (our body frame), \
             fov {:7.3} deg, {}",
            aim.angles[0],
            aim.angles[1],
            aim.angles[2],
            aim.fov,
            aim.projection(),
        );
        println!(
            "score:    {peak:.4} at the peak, {rival:.4} at the best rival more than 12 deg \
             away, prominence {:+.4}",
            peak - rival,
        );
        if let Some(other) = rival_aim {
            println!(
                "rival:    yaw {:+8.3}, pitch {:+8.3}, roll {:+8.3} deg, fov {:7.3}, {} \
                 -- {:.2} deg of rotation from the peak",
                other.angles[0],
                other.angles[1],
                other.angles[2],
                other.fov,
                other.projection(),
                apart(&other, &aim),
            );
        }
        if seeded {
            println!("SEEDED:   the sphere sweep was skipped, so no prominence is claimed");
        }
        println!(
            "content:  {:.1} percent of their frame carries enough local contrast to score",
            100.0 * textured,
        );
        println!(
            "sweep:    {} aims scored, median {:.3}, 99th {:.3}, 99.9th {:.3}, best {:.3}",
            spread.total,
            spread.quantile(0.5),
            spread.quantile(0.99),
            spread.quantile(0.999),
            coarse_peak,
        );
        if let Some(world) = world {
            let angles = angles_of(world);
            println!(
                "world:    yaw {:+8.3}, pitch {:+8.3}, roll {:+8.3} deg, once the file's own \
                 IMU is taken out",
                angles[0], angles[1], angles[2],
            );
            // The file's own orientation at this instant, printed beside the
            // two aims because it is the only thing that can tell them apart.
            //
            // A view whose HEADING is fixed in the world keeps `world` still
            // while this moves. A view whose heading follows the camera --
            // which is what Direction Lock OFF does, with the horizon still
            // levelled -- keeps `world minus track` still instead. Neither is
            // visible in one instant and both are visible in three, so the
            // number is printed rather than the conclusion.
            let track_angles = angles_of(track.at((ours.at.as_secs_f64() * 1e6).round() as i64));
            println!(
                "track:    yaw {:+8.3}, pitch {:+8.3}, roll {:+8.3} deg, the file's own \
                 orientation here; world less track heading {:+8.3}",
                track_angles[0],
                track_angles[1],
                track_angles[2],
                angles[0] - track_angles[0],
            );
        }
        println!(
            "\n{:>9} {:>11} {:>11} {:>11}",
            "off deg", "yaw", "pitch", "roll"
        );
        for (off, scores) in &ladder {
            if *off == 0.1 {
                println!("{:>9.2} {peak:>11.5} {peak:>11.5} {peak:>11.5}", 0.0);
            }
            println!(
                "{off:>9.2} {:>11.5} {:>11.5} {:>11.5}",
                scores[0], scores[1], scores[2]
            );
        }
        println!(
            "\n{:>28} {:>9} {:>11}   the best each PROJECTION could be\n\
             {:>28} {:>9} {:>11}   made to agree at, over the whole\n\
             {:>28} {:>9} {:>11}   scale window",
            "projection", "at fov", "best", "", "", "", "", "", "",
        );
        for (d, fov, score) in &families {
            println!(
                "{:>28} {fov:>9.2} {score:>11.5}",
                Aim {
                    angles: [0.0; 3],
                    fov: 0.0,
                    d: *d,
                }
                .projection(),
            );
        }
        let mut ranked: Vec<f64> = families.iter().map(|(_, _, score)| *score).collect();
        ranked.sort_by(|a, b| b.total_cmp(a));
        if ranked.len() > 1 {
            println!(
                "\nprojection: the best beats the next by {:.5}. Below about sixty degrees of \n\
                 view this number is near zero and it should be: every smooth radial map is \n\
                 linear over a small enough picture, so a narrow reframe cannot tell one \n\
                 projection from another and the aim and the scale are what it reports.",
                ranked[0] - ranked[1],
            );
        }
        println!(
            "\n{:>9} {:>11}   the whole scale window at the winning projection, each\n\
             {:>9} {:>11}   swept scale refined from its own best aims at {} px",
            "fov deg", "best", "", "", middle.width,
        );
        for (fov, _, score) in &profile {
            println!("{fov:>9.2} {score:>11.5}");
        }
        println!("\n{:>9} {:>11}", "fov x", "score");
        for (factor, score) in &scale_ladder {
            if *factor > 1.0 && *factor == 1.02 {
                println!("{:>9.2} {peak:>11.5}", 1.0);
            }
            println!("{factor:>9.2} {score:>11.5}");
        }

        // A registration nobody has looked at is a correlation coefficient.
        if let Some(out) = &options.out {
            let theirs = export.averaged(fine);
            let picture = looked(&lenses, frame, aim.look(), &ours, fine, &options.band, None);
            let difference: Vec<f64> = picture
                .iter()
                .zip(&theirs)
                .map(|(a, b)| match *a > 0.0 && *b > 0.0 {
                    true => 128.0 + (a - b) * 4.0,
                    false => 0.0,
                })
                .collect();
            let stamp = format!("{at:.3}");
            write_gray(&theirs, fine, &out.join(format!("theirs-{stamp}.png")))?;
            write_gray(&picture, fine, &out.join(format!("ours-{stamp}.png")))?;
            write_gray(
                &difference,
                fine,
                &out.join(format!("difference-4x-{stamp}.png")),
            )?;
            println!(
                "wrote:    {}/{{theirs,ours,difference-4x}}-{stamp}.png",
                out.display()
            );
        }

        runs.push(Registered {
            at,
            aim,
            peak,
            rival,
            world,
            track: angles_of(track.at((ours.at.as_secs_f64() * 1e6).round() as i64)),
            ladder,
            scale_ladder,
            saturated,
            textured,
        });
    }

    report_registration(options, &runs, &ds)
}

/// Whether one instant registered, and by what margins.
///
/// Read per instant and not per run, because the protocol already knows that
/// single frames fail for reasons of content rather than geometry (section 7)
/// and that the analysis pools the ones that pass. A run's answer is the
/// instants that passed, and a run whose failures are silent would be the
/// dangerous kind: every one of these prints its own number beside its own
/// verdict.
struct Verdict {
    ok: bool,
    lines: Vec<(String, bool, String)>,
}

fn judge(run: &Registered, fovlo: f64, top: f64) -> Verdict {
    let mut lines: Vec<(String, bool, String)> = Vec::new();
    let mut add = |name: &str, ok: bool, detail: String| lines.push((name.to_owned(), ok, detail));

    add(
        "R1 agreement",
        run.peak >= 0.30,
        format!("peak correlation {:.4}, wanted 0.30", run.peak),
    );
    add(
        "R2 prominence",
        run.peak - run.rival >= 0.05,
        format!(
            "stands {:+.4} above the best rival past 12 deg ({:.4}), wanted 0.05",
            run.peak - run.rival,
            run.rival,
        ),
    );

    // The protocol's G5, in G5's own words: monotone away from the fitted
    // pose, and the pose beats +/-2 deg in every axis by more than the
    // ladder's own step at 0.25 deg. Read over +/-2 deg because that is the
    // range a solve is started inside; what happens eight degrees out is
    // printed above and is R2's business, and R2 asks it of the whole sphere
    // rather than of three Euler axes.
    let mut monotone = true;
    let mut sharpest = f64::MAX;
    let mut margin = f64::MAX;
    for axis in 0..3 {
        let mut previous = run.peak;
        for off in [0.1, 0.25, 0.5, 1.0, 1.5, 2.0] {
            let here = run.beside(axis, off);
            if here > previous + 1e-6 {
                monotone = false;
            }
            previous = here;
        }
        let quarter = run.peak - run.beside(axis, 0.25);
        sharpest = sharpest.min(quarter);
        margin = margin.min(run.peak - run.beside(axis, 2.0) - quarter);
    }
    add(
        "R3 peak",
        monotone && sharpest > 0.0 && margin > 0.0,
        format!(
            "over +/-2 deg the ladder falls away in all three axes ({}); a quarter degree \
             costs {sharpest:.5} and two degrees cost {margin:.5} more",
            match monotone {
                true => "monotone",
                false => "NOT monotone",
            },
        ),
    );

    let mut peaked = true;
    let mut cost = f64::MAX;
    let mut previous = (run.peak, run.peak);
    for factor in [1.02, 1.04, 1.08, 1.18, 1.33] {
        let at = |want: f64| {
            run.scale_ladder
                .iter()
                .find(|(at, _)| (at - want).abs() < 0.006)
                .map_or(f64::MIN, |(_, score)| *score)
        };
        let (up, down) = (at(factor), at(1.0 / factor));
        if up > previous.0 + 1e-6 || down > previous.1 + 1e-6 {
            peaked = false;
        }
        previous = (up, down);
        if (factor - 1.04).abs() < 1e-9 {
            cost = run.peak - up.max(down);
        }
    }
    add(
        "R7 scale peak",
        peaked && cost > 0.0,
        format!("the field of view's ladder falls both ways; 4 percent of scale costs {cost:.5}"),
    );
    add(
        "R4 capture",
        !run.saturated,
        format!(
            "fov {:.3} is inside the window swept for its own projection and inside \
             that projection's own pole; the window ran {fovlo:.1} to {top:.1} deg",
            run.aim.fov,
        ),
    );

    let ok = lines.iter().all(|(_, ok, _)| *ok);
    Verdict { ok, lines }
}

/// What the whole run is allowed to be read as: the per-instant verdicts, the
/// stability across the instants that passed, and the mapping their labels
/// turned out to be.
fn report_registration(options: &Options, runs: &[Registered], ds: &[f64]) -> Fallible<()> {
    let widest = ds.iter().copied().fold(0.0, f64::max);
    let top = match options.fovhi > 0.0 {
        true => options.fovhi.min(Aim::ceiling(widest)),
        false => Aim::ceiling(widest),
    };
    println!("\n================ the run, as a whole\n");
    println!(
        "{:>9} {:>9} {:>9} {:>11} {:>8} {:>6} {:>9} {:>8} {:>6}",
        "source s", "peak", "rival", "prominence", "fov", "d", "1 deg off", "textured", "verdict"
    );
    let verdicts: Vec<Verdict> = runs
        .iter()
        .map(|run| judge(run, options.fovlo, top))
        .collect();
    for (run, verdict) in runs.iter().zip(&verdicts) {
        let one = (0..3)
            .map(|axis| run.peak - run.beside(axis, 1.0))
            .fold(f64::MAX, f64::min);
        println!(
            "{:>9.3} {:>9.4} {:>9.4} {:>+11.4} {:>8.3} {:>6.3} {:>9.4} {:>7.1}% {:>6}",
            run.at,
            run.peak,
            run.rival,
            run.peak - run.rival,
            run.aim.fov,
            run.aim.d,
            one,
            100.0 * run.textured,
            match verdict.ok {
                true => "PASS",
                false => "FAIL",
            },
        );
    }
    for (run, verdict) in runs.iter().zip(&verdicts) {
        println!("\n{:.3} s:", run.at);
        for (name, ok, detail) in &verdict.lines {
            println!(
                "  {name:<14} {:<5} {detail}",
                match ok {
                    true => "PASS",
                    false => "FAIL",
                }
            );
        }
    }

    let kept: Vec<&Registered> = runs
        .iter()
        .zip(&verdicts)
        .filter(|(_, verdict)| verdict.ok)
        .map(|(run, _)| run)
        .collect();
    println!("\n---- across instants");
    let mut passed = true;
    let mut say = |name: &str, ok: bool, detail: String| {
        passed &= ok;
        println!(
            "{name:<14} {:<5} {detail}",
            match ok {
                true => "PASS",
                false => "FAIL",
            }
        );
    };
    say(
        "R0 repeats",
        kept.len() >= 3,
        format!(
            "{} of {} instants registered. The protocol's section 7 already says single \
             frames fail on content; what it asks is that they fail loudly, and each \
             failure above names the gate it failed.",
            kept.len(),
            runs.len(),
        ),
    );
    if kept.len() >= 2 {
        let fovs: Vec<f64> = kept.iter().map(|run| run.aim.fov).collect();
        let mean_fov = mean(fovs.iter().copied());
        let sd_fov = spread(fovs.iter().copied());
        say(
            "R6 scale",
            sd_fov / mean_fov < 0.02,
            format!(
                "fov {mean_fov:.3} deg, sd {sd_fov:.3} ({:.2} percent) over the {} that \
                 passed, wanted under 2 percent",
                100.0 * sd_fov / mean_fov,
                kept.len(),
            ),
        );
        // ---- DIRECTION LOCK OFF: the body aim itself is the answer
        //
        // With the lock off Studio's view is fixed in the camera body, so
        // there is ONE body aim for the whole export and the file's own IMU is
        // not in it. That makes the across-instant test both simpler and
        // STRICTER than the locked one: all three angles are gated, including
        // the heading, which the locked path cannot gate because the locked
        // path's heading is the integration's arbitrary datum and this camera
        // has no magnetometer to hold it. Agreement here IS the confirmation
        // that the lock was off, read off the pictures rather than off a
        // checkbox.
        if options.lock == Lock::Body {
            let angles: Vec<[f64; 3]> = kept.iter().map(|run| run.aim.angles).collect();
            let apart = |axis: usize| {
                let all: Vec<f64> = angles.iter().map(|a| a[axis]).collect();
                all.iter().copied().fold(f64::MIN, f64::max)
                    - all.iter().copied().fold(f64::MAX, f64::min)
            };
            println!(
                "\nbody aim:  yaw {:.3} sd {:.3}, pitch {:.3} sd {:.3}, roll {:.3} sd {:.3} deg",
                mean(angles.iter().map(|a| a[0])),
                spread(angles.iter().map(|a| a[0])),
                mean(angles.iter().map(|a| a[1])),
                spread(angles.iter().map(|a| a[1])),
                mean(angles.iter().map(|a| a[2])),
                spread(angles.iter().map(|a| a[2])),
            );
            println!(
                "this is the aim in the camera BODY's frame, with no IMU in it. Direction Lock \n\
                 is OFF on this export, so the same three numbers have to come back at every \n\
                 instant however the camera moved between them -- and unlike the locked case \n\
                 the HEADING is gated too, because with no orientation track in the answer \n\
                 there is no arbitrary datum for it to wander on."
            );
            say(
                "R5 stability",
                apart(0) < 3.0 && apart(1) < 3.0 && apart(2) < 3.5,
                format!(
                    "the body heading agrees to {:.3} deg, the tilt to {:.3} and the roll to \
                     {:.3} across the instants that passed, wanted 3.0, 3.0 and 3.5",
                    apart(0),
                    apart(1),
                    apart(2),
                ),
            );
        } else if kept.iter().all(|run| run.world.is_some()) {
            let worlds: Vec<Quat> = kept.iter().filter_map(|run| run.world).collect();
            let angles: Vec<[f64; 3]> = worlds.iter().map(|q| angles_of(*q)).collect();
            let apart = |axis: usize| {
                let all: Vec<f64> = angles.iter().map(|a| a[axis]).collect();
                all.iter().copied().fold(f64::MIN, f64::max)
                    - all.iter().copied().fold(f64::MAX, f64::min)
            };
            println!(
                "\nworld aim: yaw {:.3} sd {:.3}, pitch {:.3} sd {:.3}, roll {:.3} sd {:.3} deg",
                mean(angles.iter().map(|a| a[0])),
                spread(angles.iter().map(|a| a[0])),
                mean(angles.iter().map(|a| a[1])),
                spread(angles.iter().map(|a| a[1])),
                mean(angles.iter().map(|a| a[2])),
                spread(angles.iter().map(|a| a[2])),
            );
            println!(
                "the world aim is the body aim with the file's own IMU taken out. Studio's \n\
                 reframe is DIRECTION LOCKED, so their view is fixed in the world and ours is \n\
                 solved in the camera body's frame: the body aim above is different at every \n\
                 instant by exactly the flight, and only this is comparable.\n\
                 \n\
                 TILT AND ROLL ARE GATED AND HEADING IS NOT, and that is a property of the \n\
                 file rather than of the search. Tilt and roll are referenced to gravity, \n\
                 which the accelerometer measures directly and the integration holds; heading \n\
                 is referenced to nothing. This camera carries no magnetometer, so the yaw of \n\
                 the integrated track is an arbitrary datum that wanders, and any disagreement \n\
                 in the column above is the track's and not the registration's. It is the \n\
                 reason section 5 now asks for direction lock OFF: with the lock off there is \n\
                 one body aim for the whole export and the IMU is not in the answer at all."
            );
            say(
                "R5 stability",
                apart(1) < 3.0 && apart(2) < 3.5,
                format!(
                    "the world tilt agrees to {:.3} deg and the world roll to {:.3} across \
                     the instants that passed, wanted 3.0 and 3.5; the heading spans \
                     {:.3} deg and is not gated",
                    apart(1),
                    apart(2),
                    apart(0),
                ),
            );
            // ---- and which of the two headings is the steady one
            //
            // Not a gate. It is the reading that says what Direction Lock did,
            // out of the pictures rather than out of a checkbox: a heading
            // fixed in the WORLD holds the first column still while the file's
            // own track moves under it, and a heading that FOLLOWS THE CAMERA
            // holds the second still instead. Both are printed because on a
            // camera with no magnetometer the first column also carries the
            // integration's drift, and a reader who is shown only the winner
            // has been shown a conclusion.
            let held: Vec<f64> = kept
                .iter()
                .zip(&angles)
                .map(|(run, aim)| aim[0] - run.track[0])
                .collect();
            let span = |all: &[f64]| {
                all.iter().copied().fold(f64::MIN, f64::max)
                    - all.iter().copied().fold(f64::MAX, f64::min)
            };
            println!(
                "\n{:>9} {:>12} {:>12} {:>12}   the two headings a reframe can hold",
                "source s", "world yaw", "track yaw", "world - track",
            );
            for (run, aim) in kept.iter().zip(&angles) {
                println!(
                    "{:>9.3} {:>12.3} {:>12.3} {:>12.3}",
                    run.at,
                    aim[0],
                    run.track[0],
                    aim[0] - run.track[0],
                );
            }
            println!(
                "the world heading spans {:.3} deg and the heading held in the camera's own \n\
                 heading spans {:.3}. WHICHEVER IS SMALLER IS THE ONE THEIR REFRAME HOLDS: \n\
                 a direction-locked view is fixed in the world, and an unlocked one follows \n\
                 the camera round while the horizon stays levelled. This is reported and not \n\
                 gated, because this camera has no magnetometer and the integrated yaw drifts \n\
                 under both of them.",
                span(&angles.iter().map(|a| a[0]).collect::<Vec<_>>()),
                span(&held),
            );
        }
    } else {
        passed = false;
    }

    // ----------------------------------------------------- what they meant
    println!("\n---- Studio's labels, in our terms");
    if kept.is_empty() {
        println!("nothing registered, so there is nothing to read their labels against.");
    } else {
        let mean_fov = mean(kept.iter().map(|run| run.aim.fov));
        let mean_d = mean(kept.iter().map(|run| run.aim.d));
        println!(
            "their FOV {:.2} is our {mean_fov:.3} deg of full field of view: a factor of \
             {:.4} on their number, and our own HALF angle is {:.3} deg (factor {:.4}).",
            options.fov,
            mean_fov / options.fov,
            mean_fov / 2.0,
            mean_fov / 2.0 / options.fov,
        );
        println!(
            "their Distortion {:.2} came back as {}.",
            options.panini,
            Aim {
                angles: [0.0; 3],
                fov: 0.0,
                d: mean_d,
            }
            .projection(),
        );
        if options.told.len() >= 3 {
            // Their pan against our heading, in whichever frame the lock says
            // the aim lives in. It is a per-file constant and not a conversion:
            // section 4b of the protocol measured it as one, and the only thing
            // it can be checked against is the OTHER export of the same file.
            let held: Vec<[f64; 3]> = match options.lock {
                Lock::Body => kept.iter().map(|run| run.aim.angles).collect(),
                Lock::World => kept.iter().filter_map(|run| run.world).map(angles_of).collect(),
            };
            if !held.is_empty() {
                let frame = match options.lock {
                    Lock::Body => "body",
                    Lock::World => "world",
                };
                let heading = mean(held.iter().map(|a| a[0]));
                let mut constant = heading - options.told[0];
                while constant > 180.0 {
                    constant -= 360.0;
                }
                while constant < -180.0 {
                    constant += 360.0;
                }
                println!(
                    "their pan {:+.2} is our {frame} heading {:+.3} deg: a per-file constant of \
                     {constant:+.3} deg.",
                    options.told[0], heading,
                );
                println!(
                    "their tilt {:+.2} is our {frame} pitch {:+.3} (off by {:+.3}) and their \
                     roll {:+.2} is our {frame} roll {:+.3} (off by {:+.3}).",
                    options.told[1],
                    mean(held.iter().map(|a| a[1])),
                    mean(held.iter().map(|a| a[1])) - options.told[1],
                    options.told[2],
                    mean(held.iter().map(|a| a[2])),
                    mean(held.iter().map(|a| a[2])) - options.told[2],
                );
            }
        }
    }
    println!(
        "\n{}",
        match passed {
            true => "REGISTERED: every gate above passed. mode=solve can be started here.",
            false =>
                "NOT REGISTERED: a gate above failed, so this is not a starting point for \
                 a solve.",
        }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Their `d = 0` is gnomonic, which is the identity `mode=parity`'s own
    /// rectilinear family already had, so the two have to draw one picture.
    #[test]
    fn panini_at_zero_is_the_rectilinear_family() {
        for u in [0.0, 0.25, 0.5, 0.75, 1.0] {
            for v in [0.0, 0.3, 1.0] {
                let half = 60f64.to_radians();
                let told = panini_ray([u, v], half, 0.0, 16.0 / 9.0);
                let fitted = ray_of([u, v], half, 1.0, 16.0 / 9.0);
                for axis in 0..3 {
                    assert!(
                        (told[axis] - fitted[axis]).abs() < 1e-9,
                        "at ({u}, {v}) axis {axis}: {} against {}",
                        told[axis],
                        fitted[axis],
                    );
                }
            }
        }
    }

    /// The inverse is closed form, so the forward map is the check on it:
    /// `s = (d+1)/(d+cos lambda)`, `x = s sin lambda`, `y = s tan phi`.
    #[test]
    fn the_panini_inverse_undoes_their_forward_map() {
        let (d, half, aspect) = (0.9, 75f64.to_radians(), 16.0 / 9.0);
        let edge = (d + 1.0) / (d + half.cos()) * half.sin();
        for u in [0.0, 0.2, 0.5, 0.9, 1.0] {
            for v in [0.0, 0.4, 1.0] {
                let ray = panini_ray([(u + 1.0) / 2.0, (v + 1.0) / 2.0], half, d, aspect);
                let lambda = ray[0].atan2(ray[2]);
                let phi = ray[1].asin();
                let s = (d + 1.0) / (d + lambda.cos());
                assert!(
                    (s * lambda.sin() / edge - u).abs() < 1e-9
                        && (s * phi.tan() * aspect / edge - v).abs() < 1e-9,
                    "({u}, {v}) came back as ({}, {})",
                    s * lambda.sin() / edge,
                    s * phi.tan() * aspect / edge,
                );
            }
        }
    }

    /// The world aim is only comparable across instants if the three angles
    /// printed for it are the three angles that build it back.
    #[test]
    fn the_world_frame_survives_a_round_trip() {
        for angles in [
            [0.0, 0.0, 0.0],
            [47.5, 52.0, 23.7],
            [-108.3, -6.2, -4.8],
            [239.4 - 360.0, -38.4, 179.6 - 360.0],
            [170.0, 89.0, -12.0],
        ] {
            let back = angles_of(orientation(angles));
            let same = orientation(back);
            let apart = norm(
                orientation(angles)
                    .conjugate()
                    .times(same)
                    .rotation_vector(),
            )
            .to_degrees();
            assert!(
                apart < 1e-6,
                "{angles:?} came back as {back:?}, which is {apart} deg away",
            );
        }
    }

    /// The compression family is what draws a round horizon, and `c = 0.5` is
    /// exactly the stereographic projection every "little planet" is.
    ///
    /// `rho = tan(c theta) / tan(c H)`, so at `c = 0.5` the picture radius
    /// goes as `tan(theta/2)`, which is stereographic's own law. This is the
    /// projection the tiny planet export registered in, and the reason a
    /// Panini-only search could not draw it at any scale.
    #[test]
    fn the_compression_family_is_stereographic_at_a_half() {
        let (half, aspect) = (146.0f64.to_radians(), 16.0 / 9.0);
        for theta in [10.0, 45.0, 90.0, 140.0f64] {
            // Where a ray `theta` off the axis lands, along the picture's own
            // horizontal, under stereographic's law.
            let rho = (theta.to_radians() / 2.0).tan() / (half / 2.0).tan();
            let ray = ray_of([(rho + 1.0) / 2.0, 0.5], half, 0.5, aspect);
            let got = ray[0].hypot(ray[1]).atan2(ray[2]).to_degrees();
            assert!(
                (got - theta).abs() < 1e-9,
                "stereographic {theta} deg landed at {got}",
            );
        }
    }

    /// A view pointed at the ground sees the horizon as a CIRCLE in the
    /// compression family and as two straight lines in Panini, which is the
    /// whole of why the tiny planet had to be given a second family.
    #[test]
    fn panini_cannot_draw_a_round_horizon_at_a_nadir_aim() {
        // "The horizon" is every ray perpendicular to the view axis. Walk the
        // picture out from the middle along two directions and record where
        // it is crossed.
        let (half, aspect) = (146.0f64.to_radians(), 16.0 / 9.0);
        let crossing = |ray: &dyn Fn(f64, f64) -> [f64; 3], dx: f64, dy: f64| {
            (1..2000)
                .map(|step| f64::from(step) / 2000.0)
                .find(|t| ray(dx * t, dy * t)[2] <= 0.0)
                .unwrap_or(f64::INFINITY)
        };
        let panini = |x: f64, y: f64| panini_ray([0.5 + x / 2.0, 0.5 + y / 2.0], half, 0.9, aspect);
        let round = |x: f64, y: f64| ray_of([0.5 + x / 2.0, 0.5 + y / 2.0], half, 0.5, aspect);
        // Across, both families cross it somewhere in the picture.
        assert!(crossing(&panini, 1.0, 0.0).is_finite());
        assert!(crossing(&round, 1.0, 0.0).is_finite());
        // Down the middle, only the round one does. Panini's `y = s tan phi`
        // never reaches the pole, so its horizon runs off the top and bottom.
        assert!(
            crossing(&panini, 0.0, 1.0).is_infinite(),
            "panini crossed the horizon vertically at {}",
            crossing(&panini, 0.0, 1.0),
        );
        assert!(crossing(&round, 0.0, 1.0).is_finite());
        // And the round one crosses it at the same radius both ways, which is
        // the circle the export was measured to have.
        let (across, down) = (crossing(&round, 1.0, 0.0), crossing(&round, 0.0, 1.0));
        assert!(
            (across - down * aspect.recip()).abs() < 2e-3,
            "round horizon at {across} across and {down} down",
        );
    }

    /// The half width is the half field of view asked for, whatever `d` does
    /// in between: a scale that drifts with the slider would move the seam map
    /// under every reading taken through it.
    #[test]
    fn the_picture_edge_is_the_field_of_view_asked_for() {
        for d in [0.0, 0.3, 0.9, 2.0] {
            for fov in [20.0, 90.0, 150.0f64] {
                let half = (fov / 2.0).to_radians();
                let ray = panini_ray([1.0, 0.5], half, d, 16.0 / 9.0);
                assert!(
                    (ray[0].atan2(ray[2]) - half).abs() < 1e-9,
                    "d {d} fov {fov}: edge at {} deg",
                    ray[0].atan2(ray[2]).to_degrees(),
                );
            }
        }
    }
}
