//! Whether any rigid five-knob pose flattens one arc of the seam, and what
//! that costs at every other azimuth.
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin downweight -- <file.insv> \
//!   basis=v3 arc=93,125 weight=8 places=6 frames=4 patches=128
//! ```
//!
//! The app fits five knobs to whatever the seam offers and weighs every
//! azimuth alike ([`fit_capture`]). The owner's complaint is at one arc of
//! that circle, the downward one, and the question this answers is whether
//! his arc is *reachable*: is what is left there a pose error some other pose
//! removes, or a shape no rigid pose has?
//!
//! Four fits over one set of readings, so the difference between them is the
//! weighting and nothing else:
//!
//! - `shipped`, the app's own: every reading once, [`KNOBS`] and [`RIDGE`].
//! - `trimmed`, the same with the across-seam outliers dropped, on the ring
//!   gate's own rule ([`seam::GATE_MADS`] and a floor), so a fit dragged by
//!   one bad correlation can be told from one that is not.
//! - `weighted`, his arc counted `weight` times and the rest once.
//! - `arconly`, his arc and nothing else. **This is the ceiling**: no
//!   weighting reaches further than fitting the arc alone, so what it leaves
//!   there is what a rigid pose cannot take off it, and what it does
//!   elsewhere is the whole bill.
//!
//! Weighting by repeating rows is weighted least squares with whole-number
//! weights, and it is the shipped fitter reading a re-weighted population
//! rather than a second fitter. It has one side effect and the run prints it:
//! [`RIDGE`] is a fixed number of extra rows, so more reading rows make the
//! prior on the principal point relatively weaker.
//!
//! `basis=v6` fits on the calibration the camera declares instead of the one
//! kjerag reads (`src/offset.rs`), which is the fair best case for v6: a pose
//! learned on top of v3 and moved onto v6 double counts whatever the two
//! bases already disagree about.
//!
//! **What this prints is the projection's own prediction, not the picture.**
//! A fit's residual here is what the map says the correlation would read with
//! the correction in place. The picture-domain answer is `--bin ceiling` at
//! the knobs this prints, and the two are different domains: nothing here is
//! a promise about what an eye sees.

use std::path::PathBuf;

use kjerag_media::{Fallible, Size};
use kjerag_meta::{CalibrationSet, Lens};
use kjerag_render::seam::{self, KNOBS, Plan, Probe, RIDGE, Reading, SeamFit};
use kjerag_spike::offset::{self, Carry};

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = basis(&options, &calibration)?;
    let files = kjerag_render::capture_set::resolve(&options.input).files;
    let plan = Plan {
        places: options.places,
        frames: options.frames,
        probe: Probe {
            patches: options.patches,
            ..Probe::default()
        },
        table: kjerag_render::Table::REST,
    };
    println!(
        "file:   {}",
        options
            .input
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
    );
    println!(
        "basis:  {} ({} lenses, lens 1 cx {:.2} cy {:.2} yaw {:.3} pitch {:.3} roll {:.3})",
        options.basis,
        lenses.len(),
        lenses[1].intrinsics.cx,
        lenses[1].intrinsics.cy,
        lenses[1].pose.yaw_deg,
        lenses[1].pose.pitch_deg,
        lenses[1].pose.roll_deg,
    );
    println!(
        "plan:   {} places x {} frames, {} azimuths, probe span {:.2} step {:.2} keep {:.2}",
        plan.places,
        plan.frames,
        plan.probe.patches,
        plan.probe.span,
        plan.probe.step,
        plan.probe.keep,
    );
    println!(
        "arc:    {:.1} to {:.1} deg of ring azimuth, weighted {} times in the weighted fit",
        options.arc.0, options.arc.1, options.weight,
    );

    reach(&lenses, frame);

    let readings = seam::measure(&files, &lenses, frame, &plan)?;
    let inside: Vec<Reading> = readings
        .iter()
        .filter(|reading| options.holds(reading))
        .copied()
        .collect();
    println!(
        "read:   {} of {} azimuths correlated, {} of them inside the arc",
        readings.len(),
        plan.probe.patches,
        inside.len(),
    );
    if readings.len() < 2 * KNOBS.len() {
        return Err(format!(
            "only {} azimuths correlated, under the {} a five knob fit is believed on",
            readings.len(),
            2 * KNOBS.len(),
        )
        .into());
    }

    let mut weighted = readings.clone();
    for _ in 1..options.weight {
        weighted.extend(inside.iter().copied());
    }
    let arms: Vec<(&str, Vec<Reading>)> = vec![
        ("shipped", readings.clone()),
        ("trimmed", trimmed(&readings)),
        ("weighted", weighted),
        ("arconly", inside.clone()),
    ];

    println!("\nfits, all through {KNOBS:?} with ridge {RIDGE}:");
    let mut fitted = Vec::new();
    for (name, population) in &arms {
        let Some(fit) = seam::fit_held(population, &lenses, frame, &KNOBS, RIDGE) else {
            println!("  {name:<9} refused: these readings do not pin a correction");
            continue;
        };
        println!(
            "  {name:<9} {:>4} rows  roll:{:+.3},yaw:{:+.3},pitch:{:+.3},cx:{:+.2},cy:{:+.2}  \
             across {:.3} -> {:.3} deg, along {:.3} -> {:.3}",
            population.len(),
            fit.fit.roll_deg,
            fit.fit.yaw_deg,
            fit.fit.pitch_deg,
            fit.fit.cx_px,
            fit.fit.cy_px,
            fit.before[1],
            fit.after[1],
            fit.before[0],
            fit.after[0],
        );
        fitted.push((*name, fit.fit));
    }

    // The shipped pose as well, so the profile below has the app's own answer
    // in it beside every fit made here.
    if let Ok(pool) = kjerag_spike::pooled(&options.input) {
        println!(
            "  {:<9} {:>4}       roll:{:+.3},yaw:{:+.3},pitch:{:+.3},cx:{:+.2},cy:{:+.2}  \
             the pose the app draws this camera with",
            "pool", "-", pool.roll_deg, pool.yaw_deg, pool.pitch_deg, pool.cx_px, pool.cy_px,
        );
        fitted.push(("pool", pool));
    }
    fitted.push(("factory", SeamFit::default()));

    profile(&options, &readings, &fitted, &lenses, frame);
    summary(&options, &readings, &fitted, &lenses, frame);
    if let Some(name) = &options.out {
        write(&options, &readings, &fitted, &lenses, frame, name)?;
    }
    Ok(())
}

/// What each knob can and cannot reach on the across-seam axis, as a shape
/// round the ring.
///
/// **This is what decides whether a leftover is a pose error at all**, and it
/// is geometry with no reading in it: each knob is turned by its own probe
/// step and the map is asked what that does to lens 1's picture at 128
/// azimuths, which is the same question [`seam::fit_held`]'s design matrix
/// asks. Each column is then decomposed into a constant, one cycle round the
/// ring, and whatever is left.
///
/// A constant is the term that matters. A leftover with a constant in it can
/// only be corrected by a knob that has one, and if no knob does then no
/// combination of them does either, whatever it is weighted by: that is a
/// property of the model and not of the evidence, and no amount of downward
/// weighting reaches it.
fn reach(lenses: &[Lens], frame: Size) {
    const AZIMUTHS: usize = 128;
    let base = seam::mapped(lenses, frame);
    println!("\nwhat one unit of each knob does across the seam, over {AZIMUTHS} azimuths:");
    println!(
        "  {:<6} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "knob", "unit", "constant", "cos", "sin", "rest rms",
    );
    for knob in seam::KNOBS {
        let turned = seam::mapped(&seam::turned(lenses, knob, knob.probe()), frame);
        let mut phi = Vec::new();
        let mut across = Vec::new();
        for at in seam::ring(AZIMUTHS) {
            if let Some(shift) = seam::moved(&base, &turned, 1, &at) {
                phi.push(at.phi);
                across.push(shift[1] / knob.probe());
            }
        }
        let (constant, cosine, sine, rest) = harmonic(&phi, &across);
        println!(
            "  {:<6} {:>10} {constant:>10.4} {cosine:>10.4} {sine:>10.4} {rest:>10.4}",
            knob.name(),
            knob.unit(),
        );
    }
    println!(
        "  Read the constant column. A relative rotation is one cycle and no constant, which \
         is geometry: the across-seam axis is the ring's own normal, so a turn about any body \
         axis reaches it as sin and cos of the azimuth and nowhere else."
    );
}

/// A constant, one cycle, and the root mean square of what neither explains.
fn harmonic(phi: &[f64], values: &[f64]) -> (f64, f64, f64, f64) {
    let count = phi.len() as f64;
    if count == 0.0 {
        return (f64::NAN, f64::NAN, f64::NAN, f64::NAN);
    }
    let constant = values.iter().sum::<f64>() / count;
    let project = |basis: fn(f64) -> f64| {
        2.0 * phi
            .iter()
            .zip(values)
            .map(|(phi, value)| value * basis(*phi))
            .sum::<f64>()
            / count
    };
    let (cosine, sine) = (project(f64::cos), project(f64::sin));
    let rest: Vec<f64> = phi
        .iter()
        .zip(values)
        .map(|(phi, value)| value - constant - cosine * phi.cos() - sine * phi.sin())
        .collect();
    (constant, cosine, sine, rms(&rest))
}

/// The lens set a fit is taken against.
fn basis(options: &Options, calibration: &CalibrationSet) -> Fallible<Vec<Lens>> {
    match options.basis.as_str() {
        "v3" => Ok(calibration.lenses.clone()),
        "v6" => {
            let written = offset::written(&options.input)?;
            let v6 = written
                .v6
                .as_ref()
                .ok_or("this file writes no offset_v6, so there is no v6 basis")?;
            offset::parse(v6)?.lenses(
                written.dimension,
                written.crop,
                Carry::From(&calibration.lenses),
            )
        }
        other => Err(format!("no calibration basis called {other}").into()),
    }
}

/// The readings the ring gate keeps, on the across-seam axis.
///
/// [`seam::left`] applies this rule along the seam, where a leftover is the
/// camera. Across the seam a capture's readings carry its scene's distances
/// as well, so this is not the same physical argument and is not offered as
/// one: it is here to say whether a fit is being dragged by a handful of
/// correlations, and its answer is only worth reading beside `shipped`.
fn trimmed(readings: &[Reading]) -> Vec<Reading> {
    let across: Vec<f64> = readings.iter().map(|reading| reading.across).collect();
    let middle = median(&across);
    let scatter = median(
        &across
            .iter()
            .map(|value| (value - middle).abs())
            .collect::<Vec<_>>(),
    );
    let tolerance = (4.0 * 1.4826 * scatter).max(0.10);
    readings
        .iter()
        .filter(|reading| (reading.across - middle).abs() <= tolerance)
        .copied()
        .collect()
}

/// What each pose leaves across the seam, azimuth by azimuth.
///
/// The shape is the question. A relative rotation reaches the across-seam
/// axis as one cycle round the ring and nothing else, so a leftover that is
/// one cycle is a pose error some pose removes and a leftover that is not is
/// a shape no rigid pose has. Printing the whole ring is what lets that be
/// read rather than asserted.
fn profile(
    options: &Options,
    readings: &[Reading],
    fitted: &[(&str, SeamFit)],
    lenses: &[Lens],
    frame: Size,
) {
    println!("\nacross-seam leftover by azimuth, in degrees ('*' marks the arc):");
    print!("  {:>7} {:>4}", "azimuth", "");
    for (name, _) in fitted {
        print!(" {name:>9}");
    }
    println!();
    let columns: Vec<Vec<Option<f64>>> = fitted
        .iter()
        .map(|(_, fit)| left(readings, *fit, lenses, frame))
        .collect();
    for (index, reading) in readings.iter().enumerate() {
        let phi = reading.at.phi.to_degrees();
        print!(
            "  {phi:>7.1} {:>4}",
            match options.holds(reading) {
                true => "*",
                false => "",
            }
        );
        for column in &columns {
            match column[index] {
                Some(value) => print!(" {value:>9.3}"),
                None => print!(" {:>9}", "-"),
            }
        }
        println!();
    }
}

/// One line per pose: what it leaves on the arc and what it leaves off it.
fn summary(
    options: &Options,
    readings: &[Reading],
    fitted: &[(&str, SeamFit)],
    lenses: &[Lens],
    frame: Size,
) {
    println!("\nwhat each pose leaves across the seam, in degrees:");
    println!(
        "  {:<9} {:>10} {:>10} {:>10} {:>10}",
        "pose", "arc rms", "arc median", "rest rms", "rest median",
    );
    for (name, fit) in fitted {
        let left = left(readings, *fit, lenses, frame);
        let split = |inside: bool| -> Vec<f64> {
            readings
                .iter()
                .zip(&left)
                .filter(|(reading, _)| options.holds(reading) == inside)
                .filter_map(|(_, value)| *value)
                .collect()
        };
        let (arc, rest) = (split(true), split(false));
        println!(
            "  {name:<9} {:>10.3} {:>10.3} {:>10.3} {:>10.3}",
            rms(&arc),
            median(&arc),
            rms(&rest),
            median(&rest),
        );
    }
}

/// What one pose leaves at every reading, across the seam, or `None` where
/// the projection cannot reach that direction under it.
fn left(readings: &[Reading], fit: SeamFit, lenses: &[Lens], frame: Size) -> Vec<Option<f64>> {
    let base = seam::mapped(lenses, frame);
    let corrected = seam::mapped(&fit.applied(lenses), frame);
    readings
        .iter()
        .map(|reading| {
            seam::moved(&base, &corrected, 1, &reading.at).map(|shift| reading.across + shift[1])
        })
        .collect()
}

fn write(
    options: &Options,
    readings: &[Reading],
    fitted: &[(&str, SeamFit)],
    lenses: &[Lens],
    frame: Size,
    name: &str,
) -> Fallible<()> {
    let out = PathBuf::from("scratch").join(name);
    std::fs::create_dir_all(out.parent().unwrap_or(std::path::Path::new(".")))?;
    let mut csv = format!(
        "# instrument: kjerag-spike --bin downweight\n# source: {}\n# args: {}\n\
         # reduction: one ring read once through the {} basis; every column is that same \
         ring re-predicted through one pose, across the seam, in degrees\n\
         # domain: projection, not picture. --bin ceiling is the picture.\n\
         azimuth_deg,in_arc,read_across_deg,read_along_deg",
        options.input.display(),
        std::env::args().skip(1).collect::<Vec<_>>().join(" "),
        options.basis,
    );
    for (name, _) in fitted {
        csv.push(',');
        csv.push_str(name);
    }
    csv.push('\n');
    let columns: Vec<Vec<Option<f64>>> = fitted
        .iter()
        .map(|(_, fit)| left(readings, *fit, lenses, frame))
        .collect();
    for (index, reading) in readings.iter().enumerate() {
        csv.push_str(&format!(
            "{:.4},{},{:.6},{:.6}",
            reading.at.phi.to_degrees(),
            u8::from(options.holds(reading)),
            reading.across,
            reading.along,
        ));
        for column in &columns {
            match column[index] {
                Some(value) => csv.push_str(&format!(",{value:.6}")),
                None => csv.push(','),
            }
        }
        csv.push('\n');
    }
    std::fs::write(&out, csv)?;
    println!("\nwrote {}", out.display());
    Ok(())
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    match sorted.len() % 2 {
        0 => (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0,
        _ => sorted[sorted.len() / 2],
    }
}

fn rms(values: &[f64]) -> f64 {
    match values.is_empty() {
        true => f64::NAN,
        false => (values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64).sqrt(),
    }
}

struct Options {
    input: PathBuf,
    basis: String,
    /// The arc of ring azimuths the weighting is about, in degrees.
    arc: (f64, f64),
    weight: usize,
    places: usize,
    frames: usize,
    patches: usize,
    out: Option<String>,
}

impl Options {
    /// Whether a reading is on the arc. Azimuth is a circle, so an arc that
    /// wraps past 360 is an arc and not an empty set.
    fn holds(&self, reading: &Reading) -> bool {
        let phi = reading.at.phi.to_degrees().rem_euclid(360.0);
        let (from, to) = (self.arc.0.rem_euclid(360.0), self.arc.1.rem_euclid(360.0));
        match from <= to {
            true => phi >= from && phi <= to,
            false => phi >= from || phi <= to,
        }
    }

    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut out = Self {
            input: PathBuf::new(),
            basis: "v3".to_owned(),
            arc: (93.0, 125.0),
            weight: 8,
            places: Plan::default().places,
            frames: Plan::default().frames,
            patches: Probe::default().patches,
            out: None,
        };
        for arg in args {
            match arg.split_once('=') {
                None => out.input = PathBuf::from(arg),
                Some(("basis", value)) => out.basis = value.to_owned(),
                Some(("arc", value)) => {
                    let (from, to) = value
                        .split_once(',')
                        .ok_or("arc= is two degrees, e.g. arc=93,125")?;
                    out.arc = (from.parse()?, to.parse()?);
                }
                Some(("weight", value)) => out.weight = value.parse()?,
                Some(("places", value)) => out.places = value.parse()?,
                Some(("frames", value)) => out.frames = value.parse()?,
                Some(("patches", value)) => out.patches = value.parse()?,
                Some(("out", value)) => out.out = Some(value.to_owned()),
                Some((key, _)) => return Err(format!("no argument called {key}. {USAGE}").into()),
            }
        }
        if out.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        if out.weight == 0 {
            return Err(
                "weight= is how many times an arc reading counts, and 0 is not a fit".into(),
            );
        }
        Ok(out)
    }
}

const USAGE: &str = "usage: downweight <file.insv> [basis=v3|v6] [arc=from,to] [weight=n] \
     [places=n] [frames=n] [patches=n] [out=name.csv]";

#[cfg(test)]
mod tests {
    use super::*;

    fn options(arc: (f64, f64)) -> Options {
        Options {
            input: PathBuf::new(),
            basis: "v3".to_owned(),
            arc,
            weight: 8,
            places: 3,
            frames: 2,
            patches: 72,
            out: None,
        }
    }

    fn at(phi_deg: f64) -> Reading {
        Reading {
            at: seam::ring(360)[phi_deg as usize],
            along: 0.0,
            across: 0.0,
        }
    }

    #[test]
    fn an_arc_holds_the_azimuths_between_its_ends() {
        let options = options((93.0, 125.0));
        assert!(options.holds(&at(100.0)));
        assert!(!options.holds(&at(50.0)));
        assert!(!options.holds(&at(200.0)));
    }

    /// Azimuth is a circle and an arc across its zero is an arc, not the
    /// empty set a naive range test makes of it.
    #[test]
    fn an_arc_may_wrap_past_the_zero() {
        let options = options((350.0, 10.0));
        assert!(options.holds(&at(355.0)));
        assert!(options.holds(&at(5.0)));
        assert!(!options.holds(&at(180.0)));
    }
}
