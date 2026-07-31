//! What the per-file seam fit leaves, and what each candidate fix would take
//! out of it (issue #48, the owner's retest of PR #87).
//!
//! ```sh
//! # what is left on his flight footage, and what every candidate fix leaves
//! cargo run --release -p kyerag-spike --bin leftover -- <a.insv> <b.insv> ...
//! # the same seam drawn as one lens against the other, before and after
//! cargo run --release -p kyerag-spike --bin leftover -- <file.insv> mode=crop \
//!   yaw=90 pitch=0 fov=40 from=900
//! ```
//!
//! The owner tested PR #87 and reported that the two streams are still not
//! perfectly aligned: an offset when he looks at the seam, and content that
//! jumps as it sweeps across the seam while the view turns. Those are one
//! finding. A blend hands a feature over from one lens to the other across
//! the crossover, so a feature the two lenses disagree about by `d` degrees
//! slides by `d` while it crosses; standing still that reads as a doubled or
//! offset edge, and turning it reads as a jump. **The number to drive down is
//! `d`, and this instrument is what measures what is left of it.**
//!
//! Everything is read through the shipped code (`kyerag_render::seam`), on the
//! frames the app itself reads at open, so what is scored here is what the
//! player draws.
//!
//! **How geometry is told from parallax, without a distance model.** The seam
//! is read at three places spread through the file, minutes apart. A
//! calibration residual is fixed in the camera and reads the same at every
//! place; parallax is a property of what the camera was looking at and does
//! not. So a patch's mean over places is the calibration part and its spread
//! over places is the scene part, and the **along-seam axis is the control
//! for that**: parallax is displacement towards the front lens along a
//! baseline perpendicular to every direction on the seam circle, so it cannot
//! reach the along column at all (docs/research/insv-format.md 4.9). Whatever
//! spread the along column shows is the instrument's own repeatability, and
//! the across column's spread over and above it is parallax.
//!
//! Numbers are printed in degrees and in **view pixels**, because degrees are
//! not what the owner is looking at. The conversion is the density of the view
//! named on the command line, measured off the map rather than assumed.

use std::path::{Path, PathBuf};

use kyerag_media::Fallible;
use kyerag_meta::{CalibrationSet, Lens};
use kyerag_render::seam::{
    self, Knob, Probe, Reading, Refused, SeamFit, Where, least_squares, mapped, moved, read_ring,
    ring, rms, turned, unit,
};
use kyerag_render::{Camera, Held, Reframe, Sampling, Size};
use kyerag_spike::{Pair, Walk};

const USAGE: &str = "usage: leftover <file.insv> [<file.insv> ...] \
                     [mode=table|crop] [yaw=] [pitch=] [fov=] [from=] [size=] [ridge=]";

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    match options.mode {
        Mode::Table => table(&options),
        Mode::Crop => crop(&options),
    }
}

enum Mode {
    /// What is left, per azimuth and per file, and what each candidate fix
    /// would leave instead.
    Table,
    /// The same seam drawn as lens 0 against lens 1, so the offset can be
    /// looked at rather than read.
    Crop,
}

// ------------------------------------------------------------ reading

/// One azimuth's readings, kept per place rather than pooled.
///
/// The app pools them into one number; this keeps them apart because the
/// spread between places is the measurement that separates the camera from
/// the scene.
struct Azimuth {
    at: Where,
    along: Vec<f64>,
    across: Vec<f64>,
    r: Vec<f64>,
    /// Mean luma of each lens's own picture of the same directions, per
    /// place: what an exposure step would show up in.
    luma: Vec<[f64; 2]>,
}

impl Azimuth {
    fn places(&self) -> usize {
        self.along.len()
    }

    fn mean_along(&self) -> f64 {
        mean(self.along.iter().copied())
    }

    fn mean_across(&self) -> f64 {
        mean(self.across.iter().copied())
    }

    fn reading(&self) -> Reading {
        Reading {
            at: self.at,
            along: self.mean_along(),
            across: self.mean_across(),
        }
    }

    /// How much this patch moved between the places, which for the across
    /// column is parallax and for the along column is the noise floor.
    fn spread(&self) -> [f64; 2] {
        [spread(&self.along), spread(&self.across)]
    }

    /// What the two lenses' pictures of the same directions differ in
    /// brightness by, in 8-bit codes, averaged over the places.
    fn exposure_step(&self) -> f64 {
        mean(self.luma.iter().map(|pair| pair[1] - pair[0]))
    }

    fn exposure_ratio(&self) -> f64 {
        let front = mean(self.luma.iter().map(|pair| pair[0]));
        match front > 1.0 {
            true => mean(self.luma.iter().map(|pair| pair[1])) / front,
            false => 1.0,
        }
    }
}

/// One file: what the seam reads on it, azimuth by azimuth and place by place.
struct Capture {
    path: PathBuf,
    lenses: Vec<Lens>,
    frame: Size,
    azimuths: Vec<Azimuth>,
}

impl Capture {
    fn name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }

    /// Only the azimuths that read at every place. A patch one place has and
    /// another does not cannot have its spread taken, and the whole
    /// separation below is the spread.
    fn steady(&self) -> Vec<&Azimuth> {
        let places = self.azimuths.iter().map(Azimuth::places).max().unwrap_or(0);
        self.azimuths
            .iter()
            .filter(|azimuth| azimuth.places() == places && places > 1)
            .collect()
    }

    /// Every azimuth whose two pictures actually matched, which is what an
    /// exposure question needs: a patch the two lenses correlate at 0.8 is a
    /// patch they are half looking at different things in, and its brightness
    /// difference is content rather than exposure.
    fn matched(&self) -> Vec<&Azimuth> {
        self.azimuths
            .iter()
            .filter(|azimuth| azimuth.places() > 0 && mean(azimuth.r.iter().copied()) >= 0.95)
            .collect()
    }

    fn readings(&self) -> Vec<Reading> {
        self.azimuths
            .iter()
            .filter(|azimuth| azimuth.places() > 0)
            .map(Azimuth::reading)
            .collect()
    }
}

/// The seam read the way the app reads it, with the places kept apart.
///
/// The plan is the shipped one ([`seam::Plan::default`]): the same places in
/// the file, the same 72 azimuths, the same probe. What differs is only that
/// nothing is averaged away.
fn read(path: &Path, options: &Options) -> Fallible<Capture> {
    let calibration = CalibrationSet::from_insv(path)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = calibration.lenses.clone();
    let plan = seam::Plan::default();
    let base = mapped(&lenses, frame);
    let circle = ring(plan.probe.patches);
    let mut walk = Walk::open(path, 0.0, frame)?;
    if walk.streams() < 2 {
        return Err("this file carries one lens stream, so it has no seam".into());
    }
    let duration = walk.duration().as_secs_f64();
    let mut azimuths: Vec<Azimuth> = circle
        .iter()
        .map(|at| Azimuth {
            at: *at,
            along: Vec::new(),
            across: Vec::new(),
            r: Vec::new(),
            luma: Vec::new(),
        })
        .collect();
    let mut refused = Refused::default();
    for place in 0..plan.places {
        let at = duration * (place as f64 + 0.5) / plan.places as f64;
        walk.jump(at)?;
        // One reading per place rather than per frame: the places are what
        // the separation below is taken over, and two frames a thirtieth of
        // a second apart are the same scene.
        let Some(pair) = walk.next_pair()? else {
            break;
        };
        let found = read_ring(&base, &pair.lenses, &circle, &plan.probe, &mut refused);
        for (azimuth, found) in azimuths.iter_mut().zip(&found) {
            let Some(found) = found.filter(|found| found.r >= plan.probe.keep) else {
                continue;
            };
            azimuth.along.push(found.along);
            azimuth.across.push(found.across);
            azimuth.r.push(found.r);
            azimuth
                .luma
                .push(brightness(&base, &pair, &azimuth.at, &plan.probe));
        }
    }
    if options.verbose {
        println!("read:   {refused:?}");
    }
    Ok(Capture {
        path: path.to_path_buf(),
        lenses,
        frame,
        azimuths,
    })
}

/// Each lens's mean brightness over the same directions, in 8-bit codes.
///
/// Sampled at the same world angles in both lenses rather than at the
/// correlation's own peak: an exposure step is a question about the same
/// content, and a fraction of a degree of geometric residual moves the patch
/// far less than the patch is wide.
fn brightness(reframe: &Reframe, pair: &Pair, at: &Where, probe: &Probe) -> [f64; 2] {
    let step = probe.step.to_radians();
    let half = (probe.span.to_radians() / 2.0 / step) as isize;
    let mut sums = [0.0; 2];
    let mut counts = [0.0; 2];
    for i in -half..=half {
        for j in -half..=half {
            let ray = unit(std::array::from_fn(|axis| {
                at.centre[axis]
                    + at.along[axis] * (i as f64 * step)
                    + at.across[axis] * (j as f64 * step)
            }));
            for lens in 0..2 {
                let landing = reframe.project(lens, ray.map(|c| c as f32));
                if !landing.inside {
                    continue;
                }
                let Some(plane) = pair.lenses.get(lens) else {
                    continue;
                };
                let Some(code) = plane.at(f64::from(landing.pixel[0]), f64::from(landing.pixel[1]))
                else {
                    continue;
                };
                sums[lens] += code;
                counts[lens] += 1.0;
            }
        }
    }
    std::array::from_fn(|lens| match counts[lens] > 0.0 {
        true => sums[lens] / counts[lens],
        false => 0.0,
    })
}

// ------------------------------------------------------------ the candidates

/// One candidate correction: which knobs it turns, which axes it is fitted
/// to, and how hard the principal point is held towards zero.
///
/// No knobs at all is the factory calibration, which is the row every other
/// row is read against.
struct Recipe {
    name: String,
    knobs: Vec<Knob>,
    /// Degrees of penalty per pixel of principal-point shift. Zero is the
    /// unregularized fit, which is the one 6.8 watched run away to 55 px on
    /// the file with seven patches.
    ///
    /// The data's own weight on the principal point is its leverage squared
    /// times the patch count, which at 0.032 degrees per pixel over fifty
    /// patches is 0.05, so a ridge above about 0.2 has already won and a
    /// ridge below about 0.02 does nothing.
    ridge: f64,
    /// A correction to score rather than fit, which is how one camera's
    /// answer is tested against another capture's pixels: the transfer
    /// control of 6.8, applied to the residual rather than to the raw seam.
    preset: Option<SeamFit>,
    /// How many times the across-seam residual a patch may read before it is
    /// dropped and the fit taken again, or zero to keep every patch.
    ///
    /// The across column carries parallax as well as calibration, and on this
    /// footage that is not a rounding term: at a 33 mm baseline, the wing
    /// lines and the harness at 0.5 to 4 m disagree by 0.5 to 4 degrees from
    /// parallax alone, against the 2.4 degrees of calibration being fitted.
    /// A patch reading far more than the rest is the near field, and a fit
    /// that keeps it is fitting the harness.
    reject: f64,
}

/// How many times a fit is re-linearized, which is what the shipped one does
/// ([`seam::fit`]) and what any candidate has to do to be compared with it.
const ROUNDS: usize = 3;

/// One candidate's answer on one file's readings.
fn fitted(readings: &[Reading], lenses: &[Lens], frame: Size, recipe: &Recipe) -> Option<SeamFit> {
    if let Some(preset) = recipe.preset {
        return Some(preset);
    }
    if recipe.knobs.is_empty() {
        return Some(SeamFit::default());
    }
    match recipe.reject > 0.0 {
        false => converge(readings, lenses, frame, recipe),
        true => {
            let first = converge(readings, lenses, frame, recipe)?;
            let left = leftover(readings, &first, lenses, frame);
            let floor = rms(left.iter().map(|axes| axes[1]));
            let kept: Vec<Reading> = readings
                .iter()
                .zip(&left)
                .filter(|(_, axes)| axes[1].abs() <= recipe.reject * floor)
                .map(|(reading, _)| *reading)
                .collect();
            converge(&kept, lenses, frame, recipe)
        }
    }
}

/// The Gauss-Newton rounds themselves, on whatever readings they are handed.
fn converge(
    readings: &[Reading],
    lenses: &[Lens],
    frame: Size,
    recipe: &Recipe,
) -> Option<SeamFit> {
    if recipe.knobs.is_empty() {
        return Some(SeamFit::default());
    }
    let base = mapped(lenses, frame);
    let mut fit = SeamFit::default();
    for _ in 0..ROUNDS {
        let so_far = fit.applied(lenses);
        let here = mapped(&so_far, frame);
        let left: Vec<Reading> = readings
            .iter()
            .filter_map(|reading| {
                let shift = moved(&base, &here, 1, &reading.at)?;
                Some(Reading {
                    at: reading.at,
                    along: reading.along + shift[0],
                    across: reading.across + shift[1],
                })
            })
            .collect();
        fit = plus(fit, round(&left, &so_far, frame, recipe)?);
    }
    Some(fit)
}

/// One linear round, about the calibration the readings were re-expressed
/// against.
fn round(readings: &[Reading], lenses: &[Lens], frame: Size, recipe: &Recipe) -> Option<SeamFit> {
    let mut rows = design(readings, lenses, frame, &recipe.knobs);
    for (index, knob) in recipe.knobs.iter().enumerate() {
        if matches!(knob, Knob::Roll | Knob::Yaw | Knob::Pitch) || recipe.ridge <= 0.0 {
            continue;
        }
        let mut basis = vec![0.0; recipe.knobs.len()];
        // In units of the knob's own probe step, so one ridge number means the
        // same thing to a principal point in pixels and to a focal length as a
        // ratio. The scale is the principal point's own probe, which is what
        // the ridge scan below was measured in.
        basis[index] = recipe.ridge * Knob::Cx.probe() / knob.probe();
        rows.push((basis, 0.0));
    }
    Some(assemble(&recipe.knobs, &least_squares(&rows)?.params))
}

/// The design matrix: one row per patch per axis, each column what one unit of
/// one knob does to that patch on that axis, through the shipped map.
fn design(
    readings: &[Reading],
    lenses: &[Lens],
    frame: Size,
    knobs: &[Knob],
) -> Vec<(Vec<f64>, f64)> {
    let base = mapped(lenses, frame);
    let probes: Vec<Reframe> = knobs
        .iter()
        .map(|knob| mapped(&turned(lenses, *knob, knob.probe()), frame))
        .collect();
    let mut rows = Vec::new();
    for reading in readings {
        let mut along = Vec::with_capacity(knobs.len());
        let mut across = Vec::with_capacity(knobs.len());
        for (index, probe) in probes.iter().enumerate() {
            let Some(shift) = moved(&base, probe, 1, &reading.at) else {
                along.clear();
                break;
            };
            along.push(shift[0] / knobs[index].probe());
            across.push(shift[1] / knobs[index].probe());
        }
        if along.len() != knobs.len() {
            continue;
        }
        // The correction is what has to be ADDED to the calibration to bring
        // the disagreement to zero, so the target is the negative of it.
        rows.push((across, -reading.across));
        rows.push((along, -reading.along));
    }
    rows
}

fn assemble(knobs: &[Knob], params: &[f64]) -> SeamFit {
    let mut fit = SeamFit::default();
    for (knob, amount) in knobs.iter().zip(params) {
        match knob {
            Knob::Roll => fit.roll_deg = *amount,
            Knob::Yaw => fit.yaw_deg = *amount,
            Knob::Pitch => fit.pitch_deg = *amount,
            Knob::Cx => fit.cx_px = *amount,
            Knob::Cy => fit.cy_px = *amount,
            Knob::Fx | Knob::Fy | Knob::Xi => {}
        }
    }
    fit
}

fn plus(fit: SeamFit, step: SeamFit) -> SeamFit {
    SeamFit {
        roll_deg: fit.roll_deg + step.roll_deg,
        yaw_deg: fit.yaw_deg + step.yaw_deg,
        pitch_deg: fit.pitch_deg + step.pitch_deg,
        cx_px: fit.cx_px + step.cx_px,
        cy_px: fit.cy_px + step.cy_px,
    }
}

/// What each patch would still read with a correction in place, predicted
/// through the map.
fn leftover(readings: &[Reading], fit: &SeamFit, lenses: &[Lens], frame: Size) -> Vec<[f64; 2]> {
    let base = mapped(lenses, frame);
    let corrected = mapped(&fit.applied(lenses), frame);
    readings
        .iter()
        .filter_map(|reading| {
            let shift = moved(&base, &corrected, 1, &reading.at)?;
            Some([reading.along + shift[0], reading.across + shift[1]])
        })
        .collect()
}

/// The worst disparity any one azimuth is left with, which is what sets the
/// jump: a feature handed over across the crossover slides by the whole of it.
fn worst(left: &[[f64; 2]]) -> f64 {
    left.iter()
        .map(|axes| axes[0].hypot(axes[1]))
        .fold(0.0, f64::max)
}

fn typical(left: &[[f64; 2]]) -> f64 {
    rms(left.iter().map(|axes| axes[0].hypot(axes[1])))
}

// ------------------------------------------------------------ the harmonics

/// A per-azimuth correction table: what is left, modelled as a constant and
/// the first two cycles round the seam circle, which is every term 6.8's own
/// structure section names.
///
/// This is not a calibration. It is what a table of residuals indexed by
/// azimuth could take out if one were applied pointwise at the seam, and the
/// number worth having is what it **cannot**: the part of the leftover that
/// has no structure round the circle is the part no table can hold.
fn harmonics(patches: &[(f64, f64)]) -> Option<(f64, f64)> {
    let rows: Vec<(Vec<f64>, f64)> = patches
        .iter()
        .map(|(phi, value)| {
            (
                vec![
                    1.0,
                    phi.cos(),
                    phi.sin(),
                    (2.0 * phi).cos(),
                    (2.0 * phi).sin(),
                ],
                *value,
            )
        })
        .collect();
    let fit = least_squares(&rows)?;
    let before = rms(patches.iter().map(|(_, value)| *value));
    let left = rms(rows.iter().map(|(basis, value)| {
        value
            - basis
                .iter()
                .zip(&fit.params)
                .map(|(b, p)| b * p)
                .sum::<f64>()
    }));
    Some((before, left))
}

// ------------------------------------------------------------ the table

fn table(options: &Options) -> Fallible<()> {
    let mut captures = Vec::new();
    for path in &options.inputs {
        captures.push(read(path, options)?);
    }
    let density = density(&captures[0], options);
    println!(
        "\nview:   {} px across {:.0} degrees, so the seam runs {:.1} view px per degree. every\n\
         \x20       px number below is that density; a degree of disagreement is {:.1} px of\n\
         \x20       picture where it crosses the seam.",
        options.size, options.fov, density, density,
    );
    for capture in &captures {
        per_file(capture, density, options);
    }
    if captures.len() > 1 {
        pooled(&captures, density);
    }
    Ok(())
}

fn per_file(capture: &Capture, density: f64, options: &Options) {
    let steady = capture.steady();
    println!(
        "\n=== {} ===\n{} azimuths correlated at all {} places, of {} that correlated anywhere",
        capture.name(),
        steady.len(),
        seam::Plan::default().places,
        capture.readings().len(),
    );
    separation(&steady, density);
    exposure(&capture.matched());
    let readings = capture.readings();
    candidates(&readings, capture, density, options);
    structure(&readings, capture, density);
    per_azimuth(capture, density);
}

/// What is left at each azimuth after the shipped fit, and what could have
/// put it there.
///
/// The sign column is the test that tells parallax from geometry, and it is
/// 6.8's own: the baseline runs along the lens axis, so a subject's distance
/// displaces it towards the front lens at **every** azimuth and parallax is
/// therefore one-signed round the whole circle. A residual rotation or a
/// principal-point shift is a one-cycle term, positive at one azimuth and
/// negative at the one opposite. Count the signs and the question is answered
/// without a distance model.
///
/// `metres` is the distance the across reading would be parallax from, at this
/// camera's baseline. It is a reading of the column and not a claim about the
/// scene: where the column is geometry the number is meaningless, which is
/// exactly what the sign count decides.
fn per_azimuth(capture: &Capture, density: f64) {
    let readings = capture.readings();
    let shipped = Recipe {
        name: "shipped".to_owned(),
        knobs: seam::KNOBS.to_vec(),
        preset: None,
        ridge: 0.0,
        reject: 0.0,
    };
    let Some(fit) = fitted(&readings, &capture.lenses, capture.frame, &shipped) else {
        return;
    };
    let left = leftover(&readings, &fit, &capture.lenses, capture.frame);
    let positive = left.iter().filter(|axes| axes[1] > 0.0).count();
    println!(
        "\nwhat is left at each azimuth after the shipped fit ({} px per degree):\n\
         {:>6} {:>9} {:>9} {:>9} {:>9}",
        density.round(),
        "phi",
        "along",
        "across",
        "px",
        "metres",
    );
    for (reading, axes) in readings.iter().zip(&left) {
        println!(
            "{:>6.0} {:>9.3} {:>9.3} {:>9.1} {:>9.1}",
            reading.at.phi.to_degrees(),
            axes[0],
            axes[1],
            axes[0].hypot(axes[1]) * density,
            match axes[1].abs() > 1e-6 {
                true => BASELINE_MM / 1e3 / axes[1].abs().to_radians(),
                false => f64::INFINITY,
            },
        );
    }
    println!(
        "\x20 across the seam, {positive} of {} azimuths read one way and {} the other. parallax \
         is one-signed\n\x20 round the whole circle by construction and a residual rotation or \
         principal point is not,\n\x20 so a lopsided count is the scene and an even one is the \
         camera.",
        left.len(),
        left.len() - positive,
    );
}

/// Geometry against scene, taken over the places rather than modelled.
fn separation(steady: &[&Azimuth], density: f64) {
    let along_spread = rms(steady.iter().map(|a| a.spread()[0]));
    let across_spread = rms(steady.iter().map(|a| a.spread()[1]));
    // The across column's spread over and above the along column's, which is
    // the only part of it the scene can own: the along column cannot carry
    // parallax at all, so whatever it spreads by is this instrument reading
    // the same fixed thing three times.
    let scene = (across_spread * across_spread - along_spread * along_spread)
        .max(0.0)
        .sqrt();
    println!(
        "\nthe scene's own share, taken over the {} places rather than modelled:\n\
         \x20 along  spread over places {along_spread:.3} deg  ({:.1} px) - parallax cannot reach \
         this axis, so this is the instrument's own repeatability\n\
         \x20 across spread over places {across_spread:.3} deg  ({:.1} px)\n\
         \x20 scene  {scene:.3} deg ({:.1} px) of the across column moves with what the camera was \
         looking at, which at a {:.1} mm baseline is content at {:.1} m",
        seam::Plan::default().places,
        along_spread * density,
        across_spread * density,
        scene * density,
        BASELINE_MM,
        match scene > 1e-6 {
            true => BASELINE_MM / 1e3 / scene.to_radians(),
            false => f64::INFINITY,
        },
    );
}

/// This camera's inter-lens baseline, in millimetres, which is what turns a
/// disparity into a distance. Read off the owner's own files by
/// `kyerag-spike --bin seam`, which prints it per capture; it is the same
/// number on every X4 Air file here.
const BASELINE_MM: f64 = 33.35;

fn exposure(matched: &[&Azimuth]) {
    let steps: Vec<f64> = matched.iter().map(|a| a.exposure_step()).collect();
    let ratios: Vec<f64> = matched.iter().map(|a| a.exposure_ratio()).collect();
    println!(
        "\nexposure across the seam over the {} azimuths the two lenses matched above r=0.95, \
         both\nsampled at the same directions:\n\
         \x20 step  {:+.1} codes on average, {:+.1} at worst, {:.1} codes of spread round the circle\n\
         \x20 gain  lens 1 reads {:.4} of lens 0, so one gain would take out {:.1} codes of the \
         average and leave the spread",
        matched.len(),
        mean(steps.iter().copied()),
        steps
            .iter()
            .copied()
            .fold(0.0f64, |held, step| match step.abs() > held.abs() {
                true => step,
                false => held,
            }),
        spread(&steps),
        mean(ratios.iter().copied()),
        mean(steps.iter().copied()).abs(),
    );
}

fn candidates(readings: &[Reading], capture: &Capture, density: f64, options: &Options) {
    println!(
        "\nwhat each candidate correction leaves, all fitted to the same {} readings:",
        readings.len(),
    );
    println!(
        "{:<24} {:>7} {:>7} {:>7} {:>7} {:>7} {:>8} {:>8} {:>9} {:>8}",
        "candidate", "roll", "yaw", "pitch", "cx", "cy", "along", "across", "typical", "worst",
    );
    for recipe in recipes(options) {
        let Some(fit) = fitted(readings, &capture.lenses, capture.frame, &recipe) else {
            println!("{:<24} singular on these patches", recipe.name);
            continue;
        };
        let left = leftover(readings, &fit, &capture.lenses, capture.frame);
        println!(
            "{:<24} {:>7.3} {:>7.3} {:>7.3} {:>7.2} {:>7.2} {:>8.3} {:>8.3} {:>8.1}p {:>7.1}p",
            recipe.name,
            fit.roll_deg,
            fit.yaw_deg,
            fit.pitch_deg,
            fit.cx_px,
            fit.cy_px,
            rms(left.iter().map(|axes| axes[0])),
            rms(left.iter().map(|axes| axes[1])),
            typical(&left) * density,
            worst(&left) * density,
        );
    }
    println!(
        "\nalong and across are the root mean square of what is left on each axis, in degrees.\n\
         typical and worst are the two axes together, in view px: worst is the number the eye\n\
         gets, because a feature crossing the crossover slides by the whole disagreement at its\n\
         own azimuth rather than by the average of the circle.",
    );
}

/// The structure of what the shipped fit leaves, and what a per-azimuth table
/// could take out of it.
fn structure(readings: &[Reading], capture: &Capture, density: f64) {
    let shipped = Recipe {
        name: "shipped".to_owned(),
        knobs: seam::KNOBS.to_vec(),
        preset: None,
        ridge: 0.0,
        reject: 0.0,
    };
    let Some(fit) = fitted(readings, &capture.lenses, capture.frame, &shipped) else {
        return;
    };
    let left = leftover(readings, &fit, &capture.lenses, capture.frame);
    let phis: Vec<f64> = readings.iter().map(|reading| reading.at.phi).collect();
    println!("\nwhat the shipped fit leaves, and how much of it turns with the azimuth:");
    for (axis, index) in [("along", 0), ("across", 1)] {
        let patches: Vec<(f64, f64)> = phis
            .iter()
            .zip(&left)
            .map(|(phi, axes)| (*phi, axes[index]))
            .collect();
        let Some((before, after)) = harmonics(&patches) else {
            continue;
        };
        println!(
            "\x20 {axis:<7} {before:.3} deg ({:.1} px) left, of which a constant and two cycles \
             round the circle hold all but {after:.3} deg ({:.1} px)",
            before * density,
            after * density,
        );
    }
    println!(
        "\x20 a per-azimuth table is worth the difference between those two numbers, and no more:\n\
         \x20 what has no structure round the circle is what the scene put there, and a table\n\
         \x20 indexed by azimuth cannot hold it.",
    );
}

/// The principal point fitted once across every file, with each file keeping
/// its own rotation.
///
/// It is an intrinsic. A lens's principal point is a property of the camera
/// and cannot differ between two captures from it, so the seven separate
/// answers 6.8 measured (-1 to -55 px) are seven fits of one number, each on
/// the azimuths its own capture happened to have content at. Fitted jointly it
/// is one number over every patch of every file, which is the regularization
/// the scatter itself asks for.
fn pooled(captures: &[Capture], density: f64) {
    println!("\n=== the principal point fitted once, across every file ===");
    let mut fits: Vec<SeamFit> = vec![SeamFit::default(); captures.len()];
    let mut shared = [0.0, 0.0];
    for _ in 0..ROUNDS {
        let width = 2 + 3 * captures.len();
        let mut rows: Vec<(Vec<f64>, f64)> = Vec::new();
        for (index, capture) in captures.iter().enumerate() {
            let base = mapped(&capture.lenses, capture.frame);
            let so_far = fits[index].applied(&capture.lenses);
            let here = mapped(&so_far, capture.frame);
            let left: Vec<Reading> = capture
                .readings()
                .iter()
                .filter_map(|reading| {
                    let shift = moved(&base, &here, 1, &reading.at)?;
                    Some(Reading {
                        at: reading.at,
                        along: reading.along + shift[0],
                        across: reading.across + shift[1],
                    })
                })
                .collect();
            let knobs = [Knob::Cx, Knob::Cy, Knob::Roll, Knob::Yaw, Knob::Pitch];
            for (basis, value) in design(&left, &so_far, capture.frame, &knobs) {
                let mut row = vec![0.0; width];
                row[0] = basis[0];
                row[1] = basis[1];
                for knob in 0..3 {
                    row[2 + 3 * index + knob] = basis[2 + knob];
                }
                rows.push((row, value));
            }
        }
        let Some(fit) = least_squares(&rows) else {
            println!("singular");
            return;
        };
        shared[0] += fit.params[0];
        shared[1] += fit.params[1];
        for (index, held) in fits.iter_mut().enumerate() {
            *held = plus(
                *held,
                SeamFit {
                    roll_deg: fit.params[2 + 3 * index],
                    yaw_deg: fit.params[3 + 3 * index],
                    pitch_deg: fit.params[4 + 3 * index],
                    cx_px: fit.params[0],
                    cy_px: fit.params[1],
                },
            );
        }
    }
    println!(
        "one principal point over every file: cx {:+.2}, cy {:+.2} px",
        shared[0], shared[1],
    );
    println!(
        "\n{:<44} {:>7} {:>7} {:>7} {:>8} {:>8} {:>9} {:>8}",
        "file", "roll", "yaw", "pitch", "along", "across", "typical", "worst",
    );
    for (capture, fit) in captures.iter().zip(&fits) {
        let readings = capture.readings();
        let left = leftover(&readings, fit, &capture.lenses, capture.frame);
        println!(
            "{:<44} {:>7.3} {:>7.3} {:>7.3} {:>8.3} {:>8.3} {:>8.1}p {:>7.1}p",
            capture.name(),
            fit.roll_deg,
            fit.yaw_deg,
            fit.pitch_deg,
            rms(left.iter().map(|axes| axes[0])),
            rms(left.iter().map(|axes| axes[1])),
            typical(&left) * density,
            worst(&left) * density,
        );
    }
}

/// Every candidate correction, all fitted to the same readings.
///
/// **Nothing here fits the along-seam axis alone**, tempting as that is: the
/// along column is the one parallax cannot reach, so a fit taken on it would
/// be free of the scene. It is also the column a lens tilt does not reach, at
/// 0.0000 degrees per degree against the across column's 1.0000 (6.8's knob
/// table), so yaw and pitch are not identifiable from it at all and the fit
/// runs to thousands of degrees. Measured: both along-only rows came back at
/// 10^3 to 10^5 degrees of rotation. The tilt can only be read off the axis
/// parallax lives on, which is the shape of this whole problem, so the rows
/// below attack the scene rather than the axis.
fn recipes(options: &Options) -> Vec<Recipe> {
    let rotation = seam::KNOBS.to_vec();
    let five = vec![Knob::Roll, Knob::Yaw, Knob::Pitch, Knob::Cx, Knob::Cy];
    let mut all = vec![
        Recipe {
            name: "none (factory)".to_owned(),
            knobs: Vec::new(),
            preset: None,
            reject: 0.0,
            ridge: 0.0,
        },
        Recipe {
            name: "shipped: rotation".to_owned(),
            knobs: rotation.clone(),
            preset: None,
            reject: 0.0,
            ridge: 0.0,
        },
        Recipe {
            name: "rotation, reject 2.0".to_owned(),
            knobs: rotation,
            preset: None,
            reject: 2.0,
            ridge: 0.0,
        },
        Recipe {
            name: "five, no ridge".to_owned(),
            knobs: five.clone(),
            preset: None,
            reject: 0.0,
            ridge: 0.0,
        },
        Recipe {
            name: "five, reject 2.0".to_owned(),
            knobs: five.clone(),
            preset: None,
            reject: 2.0,
            ridge: 0.0,
        },
    ];
    let eight = vec![
        Knob::Roll,
        Knob::Yaw,
        Knob::Pitch,
        Knob::Cx,
        Knob::Cy,
        Knob::Fx,
        Knob::Fy,
        Knob::Xi,
    ];
    for ridge in &options.ridges {
        all.push(Recipe {
            name: format!("eight, ridge {ridge:.2}"),
            knobs: eight.clone(),
            preset: None,
            reject: 0.0,
            ridge: *ridge,
        });
    }
    for preset in &options.presets {
        all.push(Recipe {
            name: format!(
                "transfer {:+.2}/{:+.2}/{:+.2}, {:+.1}/{:+.1}",
                preset.roll_deg, preset.yaw_deg, preset.pitch_deg, preset.cx_px, preset.cy_px,
            ),
            knobs: Vec::new(),
            preset: Some(*preset),
            reject: 0.0,
            ridge: 0.0,
        });
    }
    for ridge in &options.ridges {
        all.push(Recipe {
            name: format!("five, ridge {ridge:.2}"),
            knobs: five.clone(),
            preset: None,
            reject: 0.0,
            ridge: *ridge,
        });
    }
    all
}

// ------------------------------------------------------------ the picture

/// How many view pixels one degree of world angle is where the seam crosses
/// the view, measured off the map rather than assumed.
///
/// A rectilinear frame's density is not uniform, and the seam does not have to
/// cross the middle of it. This steps one output pixel at the point the seam
/// crosses the horizontal centre line and reads how far the ray turned.
fn density(capture: &Capture, options: &Options) -> f64 {
    let reframe = viewed(&capture.lenses, capture.frame, options.camera());
    let uv = |x: f64| [(x + 0.5) / f64::from(options.size), 0.5].map(|c| c as f32);
    let middle = f64::from(options.size) / 2.0;
    let Some(here) = reframe.view_ray(uv(middle)) else {
        return f64::from(options.size) / options.fov;
    };
    let Some(there) = reframe.view_ray(uv(middle + 1.0)) else {
        return f64::from(options.size) / options.fov;
    };
    let angle = angle_between(here.map(f64::from), there.map(f64::from));
    match angle > 0.0 {
        true => 1.0 / angle,
        false => f64::from(options.size) / options.fov,
    }
}

fn angle_between(a: [f64; 3], b: [f64; 3]) -> f64 {
    let a = unit(a);
    let b = unit(b);
    (0..3)
        .map(|axis| a[axis] * b[axis])
        .sum::<f64>()
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees()
}

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

/// The seam drawn as one lens against the other: lens 0 in red, lens 1 in
/// cyan.
///
/// Nothing is blended and nothing is annotated, because nothing needs to be.
/// Where the two lenses agree the picture is grey; where they disagree every
/// edge splits into a red copy and a cyan one, and how far apart those copies
/// sit **is** the disagreement, at the density the header line states. It is
/// the same picture the blend hands over across, with the handover taken out
/// so that what the handover is hiding can be seen.
fn crop(options: &Options) -> Fallible<()> {
    let out = PathBuf::from("scratch/seam2-investigation");
    std::fs::create_dir_all(&out)?;
    for path in &options.inputs {
        let calibration = CalibrationSet::from_insv(path)?;
        let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
        let mut walk = Walk::open(path, options.from, frame)?;
        let pair = walk.next_pair()?.ok_or("no frame decoded")?;
        let factory = calibration.lenses.clone();
        let fit = seam::fit_file(path, &factory, frame, &seam::Plan::default())
            .ok_or("this file gets no fit")?;
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        for (name, lenses) in [
            ("factory", factory.clone()),
            ("fitted", fit.fit.applied(&factory)),
        ] {
            let reframe = viewed(&lenses, frame, options.camera());
            let png = out.join(format!(
                "{stem}-{name}-yaw{:.0}-pitch{:.0}-fov{:.0}.png",
                options.yaw, options.pitch, options.fov,
            ));
            split(&reframe, &pair, options.size).write(&png)?;
            println!("wrote {}", png.display());
        }
        println!(
            "fit:    roll {:+.3}, yaw {:+.3}, pitch {:+.3} deg over {} patches",
            fit.fit.roll_deg, fit.fit.yaw_deg, fit.fit.pitch_deg, fit.patches,
        );
    }
    Ok(())
}

/// One view with each lens on its own colour channel.
struct Split {
    size: u32,
    pixels: Vec<u8>,
}

impl Split {
    fn write(&self, path: &Path) -> Fallible<()> {
        let mut png = png::Encoder::new(
            std::io::BufWriter::new(std::fs::File::create(path)?),
            self.size,
            self.size,
        );
        png.set_color(png::ColorType::Rgb);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()?.write_image_data(&self.pixels)?;
        Ok(())
    }
}

fn split(reframe: &Reframe, pair: &Pair, size: u32) -> Split {
    let mut pixels = Vec::with_capacity((size * size * 3) as usize);
    for y in 0..size {
        for x in 0..size {
            let uv = [
                (x as f32 + 0.5) / size as f32,
                (y as f32 + 0.5) / size as f32,
            ];
            let mut codes = [0.0f64; 2];
            if let Some(ray) = reframe.view_ray(uv) {
                for (lens, code) in codes.iter_mut().enumerate() {
                    let landing = reframe.project(lens, ray);
                    if !landing.inside {
                        continue;
                    }
                    let Some(plane) = pair.lenses.get(lens) else {
                        continue;
                    };
                    *code = plane
                        .at(f64::from(landing.pixel[0]), f64::from(landing.pixel[1]))
                        .unwrap_or(0.0);
                }
            }
            let front = codes[0].clamp(0.0, 255.0) as u8;
            let back = codes[1].clamp(0.0, 255.0) as u8;
            pixels.extend_from_slice(&[front, back, back]);
        }
    }
    Split { size, pixels }
}

// ------------------------------------------------------------ plumbing

struct Options {
    mode: Mode,
    inputs: Vec<PathBuf>,
    from: f64,
    yaw: f64,
    pitch: f64,
    fov: f64,
    size: u32,
    ridges: Vec<f64>,
    /// Corrections fitted somewhere else and scored here, `fixed=` on the
    /// command line, as many as are given.
    presets: Vec<SeamFit>,
    verbose: bool,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            mode: Mode::Table,
            inputs: Vec::new(),
            from: 0.0,
            yaw: 90.0,
            pitch: 0.0,
            // The player's own default field of view, so a px number here is
            // a px number the owner is looking at.
            fov: 90.0,
            size: 1920,
            ridges: vec![0.05, 0.10, 0.20],
            presets: Vec::new(),
            verbose: false,
        };
        for arg in args {
            let Some((key, value)) = arg.split_once('=') else {
                options.inputs.push(PathBuf::from(arg));
                continue;
            };
            match key {
                "mode" => {
                    options.mode = match value {
                        "table" => Mode::Table,
                        "crop" => Mode::Crop,
                        _ => return Err(format!("no mode called {value}. {USAGE}").into()),
                    };
                }
                "from" => options.from = value.parse()?,
                "yaw" => options.yaw = value.parse()?,
                "pitch" => options.pitch = value.parse()?,
                "fov" => options.fov = value.parse()?,
                "size" => options.size = value.parse()?,
                "verbose" => options.verbose = value.parse::<u32>()? != 0,
                "fixed" => options.presets.push(preset(value)?),
                "ridge" => {
                    options.ridges = value
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<Vec<f64>, _>>()?;
                }
                _ => return Err(format!("unknown argument {key}. {USAGE}").into()),
            }
        }
        if options.inputs.is_empty() {
            return Err(USAGE.into());
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
}

/// `roll:0.81,yaw:-2.35,pitch:-0.68,cx:-4.2,cy:-13.9`, in each knob's own
/// units, as a correction to score rather than fit.
fn preset(value: &str) -> Fallible<SeamFit> {
    let mut fit = SeamFit::default();
    for term in value.split(',') {
        let (name, amount) = term
            .split_once(':')
            .ok_or("a fixed correction is knob:amount")?;
        let amount: f64 = amount.parse()?;
        match Knob::parse(name).ok_or(format!("no knob called {name}"))? {
            Knob::Roll => fit.roll_deg = amount,
            Knob::Yaw => fit.yaw_deg = amount,
            Knob::Pitch => fit.pitch_deg = amount,
            Knob::Cx => fit.cx_px = amount,
            Knob::Cy => fit.cy_px = amount,
            Knob::Fx | Knob::Fy | Knob::Xi => {
                return Err("only a rotation and a principal point".into());
            }
        }
    }
    Ok(fit)
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let values: Vec<f64> = values.collect();
    match values.is_empty() {
        true => 0.0,
        false => values.iter().sum::<f64>() / values.len() as f64,
    }
}

fn spread(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64).sqrt()
}
