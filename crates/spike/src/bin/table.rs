//! The static per-azimuth along-seam table: what a fitted pose leaves round
//! the seam circle, pooled per camera, and whether it predicts a capture it
//! was not fitted on (issue #103, stage 9).
//!
//! ```sh
//! # what one flight's pose leaves, azimuth by azimuth, under the app's own
//! cargo run --release -p kjerag-spike --bin table -- <a.insv> seam=pool
//! # a table off several flights, written down
//! cargo run --release -p kjerag-spike --bin table -- <a.insv> <b.insv> <c.insv> \
//!   seam=<stored> out=scratch/stage9/table.txt
//! # the same, with one flight held out of the fit and predicted from it
//! cargo run --release -p kjerag-spike --bin table -- <a.insv> <b.insv> <c.insv> \
//!   seam=<stored> hold=<c.insv>
//! # a planted table, for reading back through the picture
//! cargo run --release -p kjerag-spike --bin table -- plant=0.10:6 out=scratch/stage9/plant.txt
//! ```
//!
//! **One pose for every capture.** `seam=file` is prohibited: a fit off each
//! capture's own frames absorbs that scene's own content into the pose, and the
//! leftovers of two such fits are not the same quantity measured twice.
//! `seam=pool` is the pose the app itself draws these captures with and is what
//! a reading meant to be quoted later should use; five knobs written out are
//! the same thing pinned to a date. Name neither and one pose is fitted on
//! every capture's readings at once, which is what a per-camera pool is when
//! there is no pool yet. The app has exactly one fit per camera and so does
//! this.
//!
//! **The readings are the shipped fit's own.** Nothing here re-derives a seam
//! measurement: it calls `kjerag_render::seam::measure`, which is the function
//! the app runs on a background thread while a file plays, and
//! `kjerag_render::seam::left`, which is the same subtraction the fit's own
//! residual is reduced from. What this binary adds is the pooling, the
//! smoothing sweep and the hold-out.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kjerag_media::Fallible;
use kjerag_meta::CalibrationSet;
use kjerag_render::{Leftover, SeamFit, Size, Table, seam};
use kjerag_spike::fit_arg;

/// How wide a kernel the sweep tries, in degrees of azimuth, as half-widths:
/// a kernel of `w` reaches `w` either side, so its window is `2w`.
///
/// It runs past any width a per-azimuth field could be interesting at, because
/// the best number in the held-out column is the bound on what **any** static
/// table could buy on this corpus, and a bound read off the edge of a sweep is
/// a bound on the sweep.
const WIDTHS: [f32; 11] = [
    4.0, 6.0, 8.0, 10.0, 12.0, 16.0, 24.0, 36.0, 48.0, 60.0, 90.0,
];

/// How many harmonic orders the structure table reports. Two is what the pass
/// already applies, so anything this table is for lives above it.
const ORDERS: usize = 8;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    match &options.mode {
        Mode::Read(path) => show(path),
        Mode::Plant { size_deg, cycles } => plant(*size_deg, *cycles, &options),
        Mode::Fit => fit(&options),
    }
}

// ------------------------------------------------------------ the captures

/// One capture's ring, and the leftovers a pose leaves on it.
struct Capture {
    path: PathBuf,
    lenses: Vec<kjerag_meta::Lens>,
    frame: Size,
    readings: Vec<seam::Reading>,
    left: Vec<Leftover>,
    /// What the ring read before the pose was taken off it, and after, in
    /// degrees root mean square along the seam.
    before: f64,
    after: f64,
    /// How many readings the along-seam plausibility gate refused, and the
    /// tolerance it used, in degrees.
    refused: usize,
    tolerance: f64,
}

/// Every azimuth this capture's seam offers.
///
/// Both halves of the capture, not the one file it is named by: a camera that
/// writes one lens per file has its seam between two paths, and a ring read
/// off one of them is a ring with one lens on it (issue #123).
fn observe(path: &Path, options: &Options) -> Fallible<Capture> {
    let calibration = CalibrationSet::from_insv(path)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = calibration.lenses.clone();
    let files = kjerag_render::capture_set::resolve(path).files;
    let readings = seam::measure(&files, &lenses, frame, &options.plan())?;
    if readings.is_empty() {
        return Err(format!("{}: no azimuth on the seam correlated", name(path)).into());
    }
    Ok(Capture {
        path: path.to_path_buf(),
        before: seam::rms(readings.iter().map(|reading| reading.along)),
        after: f64::NAN,
        refused: 0,
        tolerance: f64::NAN,
        left: Vec::new(),
        readings,
        lenses,
        frame,
    })
}

/// The pose taken off every capture's ring, and what it leaves.
fn subtract(captures: &mut [Capture], fit: &SeamFit, gate: Option<f64>) {
    for capture in captures {
        let left = seam::left(&capture.readings, fit, &capture.lenses, capture.frame, gate);
        capture.after = seam::rms(left.readings.iter().map(|l| f64::from(l.perp.to_degrees())));
        capture.refused = left.refused;
        capture.tolerance = f64::from(left.tolerance.to_degrees());
        capture.left = left.readings;
    }
}

/// One pose for the whole corpus, fitted on every capture's readings at once.
///
/// This is what a per-camera pool is when there is no pool yet: the camera is
/// the same in every capture and the scene is not, so a pose fitted on all of
/// them together cannot absorb any one scene the way `seam=file` can.
fn corpus_pose(captures: &[Capture]) -> Fallible<SeamFit> {
    let first = captures.first().ok_or("no capture")?;
    let readings: Vec<seam::Reading> = captures
        .iter()
        .flat_map(|c| c.readings.iter().copied())
        .collect();
    let fitted = seam::fit_held(
        &readings,
        &first.lenses,
        first.frame,
        &seam::KNOBS,
        seam::RIDGE,
    )
    .ok_or("the pooled readings do not pin a pose")?;
    Ok(fitted.fit)
}

// ------------------------------------------------------------ the report

fn fit(options: &Options) -> Fallible<()> {
    println!(
        "plan:   {} places x {} frames, {} azimuths round the ring, gate {}{}",
        options.places,
        options.frames,
        options.patches,
        match options.gate {
            Some(mads) => format!("{mads:.1} scatters"),
            None => "OFF".to_owned(),
        },
        match options.through.is_rest() {
            true => String::new(),
            false => format!(
                ", read THROUGH a table of {:.4} deg rms",
                seam::rms(
                    options
                        .through
                        .entries()
                        .iter()
                        .map(|e| f64::from(e.to_degrees()))
                ),
            ),
        },
    );
    let mut captures = Vec::new();
    for path in &options.inputs {
        captures.push(observe(path, options)?);
    }
    let pose = match options.seam {
        Some(seam) => seam,
        None => corpus_pose(&captures)?,
    };
    println!(
        "seam:   one pose for every capture{}, roll {:+.3} yaw {:+.3} pitch {:+.3} cx {:+.2} \
         cy {:+.2}",
        match options.seam {
            Some(_) => " (stored)",
            None => " (fitted on the whole corpus)",
        },
        pose.roll_deg,
        pose.yaw_deg,
        pose.pitch_deg,
        pose.cx_px,
        pose.cy_px,
    );
    subtract(&mut captures, &pose, options.gate);
    for capture in &captures {
        println!(
            "read:   {:<40} {:>3} azimuths, along the seam {:.3} -> {:.3} deg rms under the \
             pose; {} refused past {:.3} deg",
            name(&capture.path),
            capture.left.len(),
            capture.before,
            capture.after,
            capture.refused,
            capture.tolerance,
        );
    }
    coverage_of(&captures, &pose);
    let groups: Vec<Vec<Leftover>> = captures.iter().map(|c| c.left.clone()).collect();
    let pooled: Vec<Leftover> = groups.iter().flatten().copied().collect();
    if pooled.is_empty() {
        return Err("no capture had a reading on its seam".into());
    }
    structure(&pooled);
    reproduces(&captures, options.patches);
    if captures.len() > 1 {
        ladder(&captures, &pose, options);
    }
    let table = Table::of(&pooled, options.smooth);
    coverage(&table, &pooled, options.smooth);
    sweep(&captures);
    power(&captures.iter().map(|c| c.left.clone()).collect::<Vec<_>>());
    if let Some(held) = &options.hold {
        held_out(&captures, held, options)?;
    }
    if let Some(out) = &options.out {
        write(&table, out)?;
    }
    if let Some(out) = &options.field {
        let first = captures.first().ok_or("no capture")?;
        // `along_kept` and not `along_terms`: what this writes is what the app
        // would draw with, so a capture the app would refuse to pool has to be
        // refused here too or the file is a table nothing ships.
        let fields: Vec<[f64; 5]> = captures
            .iter()
            .filter_map(|c| seam::along_kept(&c.readings, &pose, &c.lenses, c.frame))
            .collect();
        let pooled = middle(&fields).ok_or("no capture pinned five terms")?;
        let table = seam::along_table(pooled, pose, &first.lenses, first.frame)
            .ok_or("that field and that pose do not compose into a calibration")?;
        println!(
            "field:  five terms off {} captures, {:.4} deg rms composed with this pose",
            fields.len(),
            seam::rms(table.entries().iter().map(|e| f64::from(e.to_degrees()))),
        );
        write(&table, out)?;
    }
    if let Some(dump) = &options.dump {
        spill(&captures, dump, options, &pose)?;
    }
    Ok(())
}

/// Every reading behind every number above, so a claim about this corpus can
/// be re-checked without a second decode.
fn spill(captures: &[Capture], dump: &Path, options: &Options, pose: &SeamFit) -> Fallible<()> {
    if let Some(parent) = dump.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = format!(
        "# kjerag-spike --bin table, {} places x {} frames, {} azimuths, gate {}\n\
         # pose: roll:{} yaw:{} pitch:{} cx:{} cy:{}\n\
         # REDUCTION: trimmed. `seam::measure` reduces each azimuth's frames by\n\
         # `seam::left`'s own rule applied per frame, so these rows are what the\n\
         # shipped path reads. Rows written before 2026-08-06 were MEAN reduced over a\n\
         # heavy-tailed population and carry that estimator's scatter\n\
         # (docs/research/stage9.md 4.5).\n\
         capture,phi_deg,left_deg\n",
        options.places,
        options.frames,
        options.patches,
        match options.gate {
            Some(mads) => format!("{mads:.1} scatters"),
            None => "off".to_owned(),
        },
        pose.roll_deg,
        pose.yaw_deg,
        pose.pitch_deg,
        pose.cx_px,
        pose.cy_px,
    );
    for capture in captures {
        for reading in &capture.left {
            text.push_str(&format!(
                "{},{:.4},{:.6}\n",
                short(&capture.path),
                f64::from(reading.phi.to_degrees()),
                f64::from(reading.perp.to_degrees()),
            ));
        }
    }
    std::fs::write(dump, text)?;
    println!("wrote:  {}", dump.display());
    Ok(())
}

/// What each capture's OWN field composes to against the leftover it was fitted
/// to, and how much of the circle it was fitted over.
///
/// The harvest guard's own numbers (`seam`'s `FIELD_LIMIT`): a field fitted over
/// one arc says whatever it likes over the rest of the circle, and this is where
/// that shows. `covered` is the widest run of azimuths with no reading in it,
/// subtracted from the whole circle.
fn coverage_of(captures: &[Capture], pose: &SeamFit) {
    println!(
        "\ncoverage: each capture's own five terms composed with this pose, against the leftover \n\
        they were fitted to. a ring with a hole in it fits terms that speak where it never read."
    );
    println!(
        "{:<17} {:>9} {:>9} {:>10} {:>10} {:>7}",
        "capture", "azimuths", "covered", "leftover", "composed", "ratio"
    );
    for capture in captures {
        let Some(terms) =
            seam::along_terms(&capture.readings, pose, &capture.lenses, capture.frame)
        else {
            println!(
                "{:<17} {:>9} no five terms",
                short(&capture.path),
                capture.left.len()
            );
            continue;
        };
        let composed = seam::along_table(terms, *pose, &capture.lenses, capture.frame)
            .map(|table| seam::rms(table.entries().iter().map(|e| f64::from(e.to_degrees()))));
        let leftover = seam::rms(capture.left.iter().map(|l| f64::from(l.perp.to_degrees())));
        println!(
            "{:<17} {:>9} {:>8.0} {:>10.4} {:>10} {:>7}",
            short(&capture.path),
            capture.left.len(),
            covered_deg(&capture.left),
            leftover,
            composed.map_or_else(|| "refused".to_owned(), |rms| format!("{rms:.4}")),
            composed.map_or_else(|| "-".to_owned(), |rms| format!("{:.2}", rms / leftover)),
        );
    }
}

/// How much of the circle a capture read, in degrees: the whole of it less the
/// widest gap between two readings it has.
fn covered_deg(left: &[Leftover]) -> f64 {
    let mut phi: Vec<f64> = left.iter().map(|l| f64::from(l.phi.to_degrees())).collect();
    if phi.len() < 2 {
        return 0.0;
    }
    phi.sort_by(f64::total_cmp);
    let widest = phi
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .chain(std::iter::once(360.0 - (phi[phi.len() - 1] - phi[0])))
        .fold(0.0f64, f64::max);
    360.0 - widest
}

/// How much of the pooled leftover each harmonic order can describe.
///
/// The pass already applies the first three of these (a constant and two
/// cycles), so a table is only worth having if the columns keep falling past
/// `two`. Fitted on the readings themselves rather than on the smoothed ring,
/// so the smoothing cannot be what makes a term look real.
fn structure(pooled: &[Leftover]) {
    println!(
        "\nstructure: what each harmonic order leaves on the {} pooled readings, in degrees rms \n\
         along the seam. the pass applies orders 0 to 2 already.",
        pooled.len(),
    );
    print!("order  ");
    for order in 0..ORDERS {
        print!("{order:>8}");
    }
    println!();
    print!("left   ");
    for order in 0..ORDERS {
        print!("{:>8.4}", harmonic_left(pooled, order));
    }
    println!();
}

/// What is left of these readings once every cycle up to `order` has been
/// fitted and taken off, in degrees.
fn harmonic_left(pooled: &[Leftover], order: usize) -> f64 {
    let terms = 1 + 2 * order;
    let rows: Vec<(Vec<f64>, f64)> = pooled
        .iter()
        .map(|l| {
            (
                basis(f64::from(l.phi), order),
                f64::from(l.perp.to_degrees()),
            )
        })
        .collect();
    let Some(fitted) = seam::least_squares(&rows) else {
        return f64::NAN;
    };
    seam::rms(
        rows.iter()
            .map(|(row, value)| value - (0..terms).map(|t| row[t] * fitted.params[t]).sum::<f64>()),
    )
}

/// The real Fourier basis up to `order` cycles round the circle.
fn basis(phi: f64, order: usize) -> Vec<f64> {
    let mut row = vec![1.0];
    for cycle in 1..=order {
        row.push((cycle as f64 * phi).cos());
        row.push((cycle as f64 * phi).sin());
    }
    row
}

/// Whether two captures read the same thing at the same azimuth, which is the
/// premise the whole table rests on.
///
/// Two columns per pair, and the second is the one that matters. `apart` and
/// `spread` are taken on the leftovers as they stand; `apart'` and `spread'`
/// are taken with **each capture's own five terms removed first**, which is
/// what `band::Along` already applies per session. A leftover that is a static
/// per-azimuth property of the camera has to survive that removal; one that is
/// only the low orders does not.
fn reproduces(captures: &[Capture], patches: usize) {
    if captures.len() < 2 {
        return;
    }
    println!(
        "\nagreement: the same azimuth read on two captures, in degrees along the seam. `apart` \n\
         is the standard deviation of the difference and `spread` the pooled standard deviation \n\
         of the two captures' own readings there; primed columns have each capture's own five \n\
         terms taken off first, which is what the pass already applies per session."
    );
    println!(
        "{:<17} {:<17} {:>7} {:>8} {:>8} {:>8} {:>8} {:>7}",
        "capture", "against", "shared", "apart", "spread", "apart'", "spread'", "r'"
    );
    let levelled: Vec<Vec<Leftover>> = captures.iter().map(without_pose).collect();
    let (mut raw, mut clean) = (Vec::new(), Vec::new());
    for (index, one) in captures.iter().enumerate() {
        for (other, them) in captures.iter().enumerate().skip(index + 1) {
            let plain = paired(&one.left, &captures[other].left, patches);
            let cut = paired(&levelled[index], &levelled[other], patches);
            println!(
                "{:<17} {:<17} {:>7} {:>8.4} {:>8.4} {:>8.4} {:>8.4} {:>7.3}",
                short(&one.path),
                short(&them.path),
                plain.len(),
                deviation(plain.iter().map(|(a, b)| a - b)),
                pooled_deviation(&plain),
                deviation(cut.iter().map(|(a, b)| a - b)),
                pooled_deviation(&cut),
                correlation(&cut),
            );
            raw.extend(plain);
            clean.extend(cut);
        }
    }
    println!(
        "\nover all {} pairs at once, the two captures' readings correlate at {:+.3} as they \n\
         stand and {:+.3} with each capture's own five terms taken off. **All of the agreement \n\
         between flights is in the orders the pass already applies**, and what is left above \n\
         them is uncorrelated between flights, which is what a static per-azimuth field cannot \n\
         be.",
        captures.len() * (captures.len() - 1) / 2,
        correlation(&raw),
        correlation(&clean),
    );
}

/// One capture's readings with its own five terms taken off it.
fn without_pose(capture: &Capture) -> Vec<Leftover> {
    let rows: Vec<(Vec<f64>, f64)> = capture
        .left
        .iter()
        .map(|l| (basis(f64::from(l.phi), 2), f64::from(l.perp.to_degrees())))
        .collect();
    let Some(fitted) = seam::least_squares(&rows) else {
        return capture.left.clone();
    };
    capture
        .left
        .iter()
        .zip(&rows)
        .map(|(l, (row, value))| Leftover {
            perp: (value - dot(row, &fitted.params)).to_radians() as f32,
            ..*l
        })
        .collect()
}

fn dot(row: &[f64], params: &[f64]) -> f64 {
    row.iter().zip(params).map(|(a, b)| a * b).sum()
}

/// The two sets of readings at the azimuths both of them reached, in degrees.
///
/// Binned on the **patch index** and not on a truncated division of the
/// azimuth: the ring's azimuths are exact multiples of its own spacing only up
/// to the float that carried them, and truncating put nine of seventy-two
/// readings in their neighbour's bin, which compared readings five degrees
/// apart in an eighth of the rows.
fn paired(one: &[Leftover], other: &[Leftover], patches: usize) -> Vec<(f64, f64)> {
    let index = |l: &Leftover| {
        let turn = f64::from(l.phi) / std::f64::consts::TAU * patches as f64;
        (turn.round() as i64).rem_euclid(patches as i64)
    };
    let theirs: BTreeMap<i64, f64> = other
        .iter()
        .map(|l| (index(l), f64::from(l.perp.to_degrees())))
        .collect();
    one.iter()
        .filter_map(|l| Some((f64::from(l.perp.to_degrees()), *theirs.get(&index(l))?)))
        .collect()
}

/// The standard deviation about the sample's own mean, which is what a
/// "variation" is. Root mean square about zero is a magnitude and was printed
/// under this heading until 2026-08-06.
fn deviation(values: impl Iterator<Item = f64>) -> f64 {
    let all: Vec<f64> = values.collect();
    if all.len() < 2 {
        return f64::NAN;
    }
    let mean = all.iter().sum::<f64>() / all.len() as f64;
    (all.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (all.len() - 1) as f64).sqrt()
}

/// How much the two captures vary at these azimuths, both of them together:
/// the number `apart` has to be small against if the leftover is the camera.
fn pooled_deviation(pairs: &[(f64, f64)]) -> f64 {
    let one = deviation(pairs.iter().map(|(a, _)| *a));
    let other = deviation(pairs.iter().map(|(_, b)| *b));
    (0.5 * (one * one + other * other)).sqrt()
}

/// Pearson's, between the two captures' readings at the azimuths they share.
fn correlation(pairs: &[(f64, f64)]) -> f64 {
    if pairs.len() < 3 {
        return f64::NAN;
    }
    let mean = |f: fn(&(f64, f64)) -> f64| pairs.iter().map(f).sum::<f64>() / pairs.len() as f64;
    let (ma, mb) = (mean(|p| p.0), mean(|p| p.1));
    let together: f64 = pairs.iter().map(|(a, b)| (a - ma) * (b - mb)).sum();
    let spread = |f: fn(&(f64, f64)) -> f64, m: f64| {
        pairs.iter().map(|p| (f(p) - m).powi(2)).sum::<f64>().sqrt()
    };
    together / (spread(|p| p.0, ma) * spread(|p| p.1, mb))
}

/// How much of the ring the table speaks for, and how large it is where it
/// does.
fn coverage(table: &Table, pooled: &[Leftover], smooth: f32) {
    let entries = table.entries();
    let spoken = entries.iter().filter(|e| **e != 0.0).count();
    let worst = entries
        .iter()
        .fold(0.0f32, |worst, e| worst.max(e.abs()))
        .to_degrees();
    println!(
        "\ntable:  {spoken} of {} directions are moved at all at a {smooth:.0} deg kernel, off {} \n\
         readings; {} of them have a whole reading's worth of evidence or more, which is where \n\
         the ridge stops halving the answer. worst entry {worst:.4} deg, rms {:.4} deg.",
        entries.len(),
        pooled.len(),
        believed(pooled, smooth),
        seam::rms(entries.iter().map(|e| f64::from(e.to_degrees()))),
    );
}

/// How many directions have at least one reading's worth of kernel weight
/// behind them: below that the ridge is taking more than half the answer away
/// and the entry is a taper rather than a measurement.
fn believed(pooled: &[Leftover], smooth: f32) -> usize {
    Table::evidence(pooled, smooth)
        .iter()
        .filter(|weight| **weight >= 1.0)
        .count()
}

/// What each kernel width leaves on the captures it was fitted to, and on one
/// it was not.
///
/// The second column is the one that decides the width. A narrow kernel always
/// wins the first: it is free to follow its own readings' noise, and that is
/// the stage-7 striping lesson written as a number.
fn sweep(captures: &[Capture]) {
    println!(
        "\nkernel: what each width leaves, in degrees rms along the seam. `fitted` is measured on \n\
         the captures the table was built from, `held out` on the one capture it was not, taken \n\
         in turn. a width is chosen by the second column."
    );
    println!("{:>8} {:>10} {:>10}", "deg", "fitted", "held out");
    let groups: Vec<Vec<Leftover>> = captures.iter().map(|c| c.left.clone()).collect();
    let pooled: Vec<Leftover> = groups.iter().flatten().copied().collect();
    // The row a table has to beat: every reading as it stands, with nothing
    // applied. A width whose held-out column sits above this one has cost the
    // capture it was not fitted on.
    let none = seam::rms(pooled.iter().map(|l| f64::from(l.perp.to_degrees())));
    println!("{:>8} {none:>10.4} {none:>10.4}", "none");
    let mut best = (f64::INFINITY, 0.0f32);
    for width in WIDTHS {
        let table = Table::of(&pooled, width);
        let fitted = seam::rms(pooled.iter().map(|l| left_of(&table, l)));
        let held = rotated(&groups, width);
        println!("{width:>8.0} {fitted:>10.4} {held:>10.4}");
        if held < best.0 {
            best = (held, width);
        }
    }
    println!(
        "\nthe bound: the best any static table reaches on a capture it was not fitted on is \n\
         {:.4} deg at a {:.0} deg kernel, which is {:+.2} percent of the {:.4} deg it would \n\
         have read with no table at all. That is not a width to ship, it is the ceiling on \n\
         what this corpus could pay for a per-azimuth field at any setting.",
        best.0,
        best.1,
        100.0 * (none - best.0) / none,
        none,
    );
}

/// The leave-one-capture-out mean: each capture predicted by a table built
/// from every other one.
fn rotated(groups: &[Vec<Leftover>], width: f32) -> f64 {
    if groups.len() < 2 {
        return f64::NAN;
    }
    let mut left = Vec::new();
    for (index, held) in groups.iter().enumerate() {
        let pooled: Vec<Leftover> = groups
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .flat_map(|(_, group)| group.iter().copied())
            .collect();
        let table = Table::of(&pooled, width);
        left.extend(held.iter().map(|l| left_of(&table, l)));
    }
    seam::rms(left.into_iter())
}

/// What size of static per-azimuth field this corpus could have found, order
/// by order.
///
/// **The number a refusal is worth nothing without.** A field of a known order
/// and a known size is added to every capture's readings - the same field in
/// all of them, which is what "static" means - and the whole leave-one-out test
/// is run again. What is asked of the result is not "did it help", because a
/// noiseless plant helps a little at any size; it is **how much of the planted
/// field's own power came back** on the captures the table was not fitted on,
/// over and above what the same test recovers with nothing planted.
///
/// The bound grows with order because the kernel that keeps the table honest
/// is a low-pass filter. Orders 1 and 2 never come back at all, and that is
/// correct rather than a failure: they are a pose, [`Table`] has them taken
/// out of it by construction, and `band::Along` already applies them.
fn power(groups: &[Vec<Leftover>]) {
    let floor = recovered(groups, 0, 0.0).0;
    println!(
        "\npower: the smallest static field of each order this corpus can recover half of, in \n\
         degrees of amplitude. planted into every capture's readings and put through the same \n\
         leave-one-out test at every kernel width. orders 1 and 2 are a pose: the table has \n\
         them taken out of it and the pass applies them itself, so they never come back here. \n\
         a size under {:.4} deg is not tried: that is the field whose power equals the \n\
         improvement this test makes out of a corpus with nothing planted in it at all.",
        (2.0f64 * floor).sqrt(),
    );
    println!(
        "{:>6} {:>14} {:>10} {:>12}",
        "order", "half back at", "at kernel", "recovered"
    );
    for order in 1..=8 {
        match limit(groups, order, floor) {
            Some((size, width, share)) => {
                println!(
                    "{order:>6} {size:>14.4} {width:>10.0} {:>11.0}%",
                    100.0 * share
                )
            }
            None => println!("{order:>6} {:>14} {:>10} {:>12}", "not by 0.24", "-", "-"),
        }
    }
}

/// The smallest planted size of this order whose power the held-out test gets
/// half of back, the kernel that got it, and what fraction that was.
fn limit(groups: &[Vec<Leftover>], order: usize, floor: f64) -> Option<(f64, f32, f64)> {
    for size in SIZES {
        // A field smaller than the improvement this test manufactures from
        // nothing cannot be claimed to have been recovered from it: the share
        // below would be a ratio of two numbers the same size, and it reads
        // eight hundred percent as readily as fifty.
        let planted = 0.5 * size * size;
        if planted <= floor {
            continue;
        }
        let (power, width) = recovered(groups, order, size);
        let share = (power - floor) / planted;
        if share >= 0.5 {
            return Some((size, width, share));
        }
    }
    None
}

/// How much mean square the held-out test takes off a corpus with this field
/// planted in it, and the kernel that took the most.
///
/// Mean square rather than rms because power is what adds: a planted field of
/// amplitude `a` carries `a * a / 2` of it, and that is what the share above
/// is a share of.
fn recovered(groups: &[Vec<Leftover>], order: usize, size: f64) -> (f64, f32) {
    let planted: Vec<Vec<Leftover>> = groups
        .iter()
        .map(|group| {
            group
                .iter()
                .copied()
                .map(|l| ripple(l, order, size))
                .collect()
        })
        .collect();
    let flat: Vec<Leftover> = planted.iter().flatten().copied().collect();
    let none = seam::rms(flat.iter().map(|l| f64::from(l.perp.to_degrees())));
    let best = WIDTHS
        .iter()
        .map(|width| (rotated(&planted, *width), *width))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap_or((none, 0.0));
    (none * none - best.0 * best.0, best.1)
}

fn ripple(reading: Leftover, order: usize, size: f64) -> Leftover {
    let added = size * (order as f64 * f64::from(reading.phi)).cos();
    Leftover {
        perp: reading.perp + added.to_radians() as f32,
        ..reading
    }
}

/// The sizes the power scan tries, in degrees, smallest first.
const SIZES: [f64; 13] = [
    0.0025, 0.005, 0.0075, 0.01, 0.015, 0.02, 0.03, 0.04, 0.06, 0.08, 0.12, 0.16, 0.24,
];

/// One reading with the table's answer at its own azimuth taken off it, in
/// degrees.
fn left_of(table: &Table, reading: &Leftover) -> f64 {
    let (sin, cos) = reading.phi.sin_cos();
    f64::from((reading.perp - table.at(cos, sin)).to_degrees())
}

/// The ladder the shipped answer is measured on: each capture predicted by a
/// **five-term field** pooled off the others, through the very functions the
/// app runs (issue #103, stage 9 layer 2).
///
/// Every arm is held out. `pose only` is what the pose alone leaves, `5 terms`
/// is what it leaves with `seam::along_kept` pooled off the OTHER captures and
/// composed with this pose by `seam::along_table`, and `5 + table` puts the
/// per-azimuth table of the same others on top of that, which is stage 9's own
/// refusal re-asked with the field underneath it.
///
/// The fields are `seam::along_kept`'s, which is the function the app harvests
/// through, so a capture whose ring did not pin a field it can stand behind is
/// out of every other capture's pool here exactly as it would be out of the
/// app's.
///
/// The pooling rule is the app's own, `SeamPool::field`, restated here because
/// the app crate is a binary and this one cannot link it: a middle taken
/// coefficient by coefficient. `mean` is printed beside it as the control,
/// because the corpus arm that measured this pooled the readings and fitted
/// once, which is an average of coefficients and not a middle of them.
fn ladder(captures: &[Capture], pose: &SeamFit, options: &Options) {
    let fields: Vec<Option<[f64; 5]>> = captures
        .iter()
        .map(|c| seam::along_kept(&c.readings, pose, &c.lenses, c.frame))
        .collect();
    println!(
        "\nfield: each capture predicted by a five-term along-seam field fitted on the OTHERS and \n\
        composed with this pose, in degrees rms along the seam. every column is held out. \n\
        `middle` is the app's pooling rule and `mean` is the control."
    );
    println!(
        "{:<17} {:>9} {:>10} {:>10} {:>10} {:>10}",
        "capture", "azimuths", "pose only", "middle", "mean", "5 + table"
    );
    let mut totals: [Vec<f64>; 4] = std::array::from_fn(|_| Vec::new());
    for (index, capture) in captures.iter().enumerate() {
        let others: Vec<[f64; 5]> = fields
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .filter_map(|(_, terms)| *terms)
            .collect();
        let elsewhere: Vec<Leftover> = captures
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .flat_map(|(_, c)| c.left.iter().copied())
            .collect();
        let arms = [None, middle(&others), mean(&others), middle(&others)];
        let mut read = [0.0f64; 4];
        for (column, terms) in arms.into_iter().enumerate() {
            let field = terms
                .and_then(|terms| seam::along_table(terms, *pose, &capture.lenses, capture.frame));
            let above = (column == 3).then(|| Table::of(&elsewhere, options.smooth));
            read[column] = seam::rms(capture.left.iter().map(|l| {
                let (sin, cos) = l.phi.sin_cos();
                let corrected =
                    field.map_or(0.0, |t| t.at(cos, sin)) + above.map_or(0.0, |t| t.at(cos, sin));
                f64::from((l.perp - corrected).to_degrees())
            }));
            totals[column].push(read[column]);
        }
        println!(
            "{:<17} {:>9} {:>10.4} {:>10.4} {:>10.4} {:>10.4}",
            short(&capture.path),
            capture.left.len(),
            read[0],
            read[1],
            read[2],
            read[3],
        );
    }
    let pooled = |column: usize| {
        seam::rms(
            captures
                .iter()
                .zip(&totals[column])
                .flat_map(|(c, value)| std::iter::repeat_n(*value, c.left.len())),
        )
    };
    println!(
        "{:<17} {:>9} {:>10.4} {:>10.4} {:>10.4} {:>10.4}",
        "all",
        captures.iter().map(|c| c.left.len()).sum::<usize>(),
        pooled(0),
        pooled(1),
        pooled(2),
        pooled(3),
    );
    let improved = captures
        .iter()
        .zip(totals[1].iter().zip(&totals[0]))
        .filter(|(_, (five, pose))| five < pose)
        .count();
    println!(
        "\n{improved} of {} captures improved by a field they were not in.",
        captures.len(),
    );
}

/// The middle of some fields, coefficient by coefficient: `SeamPool::field`.
fn middle(fields: &[[f64; 5]]) -> Option<[f64; 5]> {
    if fields.is_empty() {
        return None;
    }
    Some(std::array::from_fn(|term| {
        let mut all: Vec<f64> = fields.iter().map(|terms| terms[term]).collect();
        all.sort_by(f64::total_cmp);
        all[all.len() / 2]
    }))
}

/// The control: their mean, which is what pooling the readings and fitting once
/// comes to on a basis this nearly orthogonal.
fn mean(fields: &[[f64; 5]]) -> Option<[f64; 5]> {
    if fields.is_empty() {
        return None;
    }
    Some(std::array::from_fn(|term| {
        fields.iter().map(|terms| terms[term]).sum::<f64>() / fields.len() as f64
    }))
}

/// One named capture predicted by a table fitted on the others, reported on
/// its own.
fn held_out(captures: &[Capture], held: &Path, options: &Options) -> Fallible<()> {
    let Some(subject) = captures.iter().find(|c| c.path == held) else {
        return Err(format!("{} is not one of the captures read", name(held)).into());
    };
    let pooled: Vec<Leftover> = captures
        .iter()
        .filter(|c| c.path != held)
        .flat_map(|c| c.left.iter().copied())
        .collect();
    let table = Table::of(&pooled, options.smooth);
    println!(
        "\nheld out: {} was not in the fit. under the pose alone it reads {:.4} deg rms along the \n\
         seam; with a table off the other {} captures it reads {:.4}.",
        name(held),
        subject.after,
        captures.len() - 1,
        seam::rms(subject.left.iter().map(|l| left_of(&table, l))),
    );
    Ok(())
}

// ------------------------------------------------------------ the controls

/// A table of a known size and a known number of cycles, for reading back
/// through the picture.
///
/// Six cycles and up is deliberately above the two the pass already applies:
/// a plant the five terms could describe would be corrected by the band as
/// well and the two arms would not be comparable.
fn plant(size_deg: f32, cycles: f32, options: &Options) -> Fallible<()> {
    let entries = std::array::from_fn(|index| {
        let phi = index as f32 / kjerag_render::AZIMUTHS as f32 * std::f32::consts::TAU;
        size_deg.to_radians() * (cycles * phi).cos()
    });
    let table = Table::of_entries(entries);
    println!(
        "plant:  {size_deg:.3} deg, {cycles:.0} cycles round the ring, {} directions.",
        kjerag_render::AZIMUTHS,
    );
    match &options.out {
        Some(out) => write(&table, out),
        None => {
            print!("{}", table.write());
            Ok(())
        }
    }
}

fn load(path: &Path) -> Fallible<Table> {
    Table::read(&std::fs::read_to_string(path)?)
        .ok_or_else(|| format!("{} is not {} numbers", name(path), kjerag_render::AZIMUTHS).into())
}

fn show(path: &Path) -> Fallible<()> {
    let table = load(path)?;
    println!("{:>7} {:>12}", "phi", "deg");
    for (index, entry) in table.entries().iter().enumerate() {
        let phi = index as f64 / kjerag_render::AZIMUTHS as f64 * 360.0;
        println!("{phi:>7.1} {:>12.5}", f64::from(entry.to_degrees()));
    }
    Ok(())
}

fn write(table: &Table, out: &Path) -> Fallible<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, table.write())?;
    println!("wrote:  {}", out.display());
    Ok(())
}

fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into()
}

/// A capture named short enough for a column: the date and the clip, which is
/// what tells the owner's flights apart.
fn short(path: &Path) -> String {
    let name = name(path);
    name.strip_prefix("VID_")
        .map_or(name.clone(), |rest| rest.chars().take(15).collect())
}

// ------------------------------------------------------------ the options

enum Mode {
    Fit,
    Read(PathBuf),
    Plant { size_deg: f32, cycles: f32 },
}

struct Options {
    mode: Mode,
    inputs: Vec<PathBuf>,
    seam: Option<SeamFit>,
    /// A table the ring is read through, which is how a plant is read back:
    /// the leftovers must come back moved by exactly its negative.
    through: Table,
    hold: Option<PathBuf>,
    out: Option<PathBuf>,
    dump: Option<PathBuf>,
    smooth: f32,
    /// How many times its own scatter a reading may sit from its capture's
    /// middle. `None` under `gate=0`, which is how the report says what the
    /// gate is worth rather than assuming it.
    gate: Option<f64>,
    places: usize,
    frames: usize,
    /// Where to write the pooled FIVE-TERM field composed with the pose, which
    /// is what the app applies, as against `out`, which writes the per-azimuth
    /// table stage 9 refused.
    field: Option<PathBuf>,
    patches: usize,
}

impl Options {
    fn plan(&self) -> seam::Plan {
        let mut plan = seam::Plan {
            places: self.places,
            frames: self.frames,
            table: self.through,
            ..seam::Plan::default()
        };
        plan.probe.patches = self.patches;
        plan
    }

    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            mode: Mode::Fit,
            inputs: Vec::new(),
            seam: None,
            through: Table::REST,
            hold: None,
            out: None,
            dump: None,
            smooth: kjerag_render::band::SMOOTH_DEG,
            gate: Some(seam::GATE_MADS),
            places: 3,
            frames: 2,
            field: None,
            patches: 72,
        };
        let mut seam = None;
        for arg in args {
            match arg.split_once('=') {
                Some(("seam", value)) => {
                    if value == "file" || value == "factory" {
                        return Err(USAGE_SEAM.into());
                    }
                    seam = Some(value.to_string());
                }
                Some(("through", value)) => options.through = load(Path::new(value))?,
                Some(("hold", value)) => options.hold = Some(PathBuf::from(value)),
                Some(("out", value)) => options.out = Some(PathBuf::from(value)),
                Some(("dump", value)) => options.dump = Some(PathBuf::from(value)),
                Some(("read", value)) => options.mode = Mode::Read(PathBuf::from(value)),
                Some(("plant", value)) => options.mode = planted(value)?,
                Some(("smooth", value)) => options.smooth = value.parse()?,
                Some(("gate", value)) => options.gate = gate_of(value.parse()?),
                Some(("field", value)) => options.field = Some(PathBuf::from(value)),
                Some(("places", value)) => options.places = value.parse()?,
                Some(("frames", value)) => options.frames = value.parse()?,
                Some(("patches", value)) => options.patches = value.parse()?,
                Some(_) => return Err(format!("{USAGE}\n\nunknown: {arg}").into()),
                None => options.inputs.push(PathBuf::from(arg)),
            }
        }
        if matches!(options.mode, Mode::Fit) && options.inputs.is_empty() {
            return Err(USAGE.into());
        }
        // After the loop, because `seam=pool` is resolved against a capture
        // and the captures may be named anywhere on the line. The first is
        // enough: the pool is keyed by camera and this instrument is a
        // per-camera reading already.
        if let Some(value) = seam {
            let input = options.inputs.first().ok_or(USAGE_SEAM_POOL)?;
            options.seam = Some(fit_arg(&value, input)?);
        }
        Ok(options)
    }
}

/// `gate=0` turns the along-seam plausibility filter off, and anything else
/// is how many of a capture's own scatters a reading may sit from its middle.
fn gate_of(mads: f64) -> Option<f64> {
    (mads > 0.0).then_some(mads)
}

fn planted(value: &str) -> Fallible<Mode> {
    let (size, cycles) = value
        .split_once(':')
        .ok_or("a plant is size_deg:cycles, and the cycles are whole")?;
    Ok(Mode::Plant {
        size_deg: size.parse()?,
        cycles: cycles.parse()?,
    })
}

const USAGE: &str = "usage: table <file.insv> [<file.insv> ...] [seam=pool|roll:0.8,yaw:-2.3,\
pitch:-0.9,cx:-3.3,cy:-11.9] [through=table.txt] [hold=<file.insv>] [smooth=deg] [gate=mads|0] [places=n] [frames=n] [patches=n] [out=path] [field=path] [dump=path.csv] \
| read=path | plant=size_deg:cycles [out=path]";

const USAGE_SEAM: &str = "this instrument needs one stored fit for every capture: a fit off each \
capture's own frames absorbs that scene into the pose, and two such fits do not leave the same \
quantity behind. seam=pool, which is the one the app draws with, or \
seam=roll:..,yaw:..,pitch:..,cx:..,cy:..";

const USAGE_SEAM_POOL: &str = "seam=pool is read out of the saved state under the camera a capture \
names, so it needs a capture on the line to name one";
