//! The static per-azimuth along-seam table: what a fitted pose leaves round
//! the seam circle, pooled per camera, and whether it predicts a capture it
//! was not fitted on (issue #103, stage 9).
//!
//! ```sh
//! # what one flight's pose leaves, azimuth by azimuth, under a stored fit
//! cargo run --release -p kjerag-spike --bin table -- <a.insv> \
//!   seam=roll:0.795,yaw:-2.310,pitch:-0.936,cx:-3.28,cy:-11.91
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
//! leftovers of two such fits are not the same quantity measured twice. Name a
//! stored fit with `seam=`, or name none and get one pose fitted on every
//! capture's readings at once, which is what a per-camera pool is when there is
//! no pool yet. The app has exactly one fit per camera and so does this.
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
use kjerag_spike::seam_fit;

/// How wide a kernel the sweep tries, in degrees of azimuth. The shipped
/// width has to be one of them or the report cannot say where it sits.
const WIDTHS: [f32; 8] = [4.0, 6.0, 8.0, 10.0, 12.0, 16.0, 24.0, 36.0];

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
fn subtract(captures: &mut [Capture], fit: &SeamFit) {
    for capture in captures {
        let left = seam::left(&capture.readings, fit, &capture.lenses, capture.frame);
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
        "plan:   {} places x {} frames, {} azimuths round the ring{}",
        options.places,
        options.frames,
        options.patches,
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
    subtract(&mut captures, &pose);
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
    let pooled: Vec<Leftover> = captures
        .iter()
        .flat_map(|c| c.left.iter().copied())
        .collect();
    if pooled.is_empty() {
        return Err("no capture had a reading on its seam".into());
    }
    structure(&pooled);
    reproduces(&captures);
    let table = Table::of(&pooled, options.smooth);
    coverage(&table, &pooled, options.smooth);
    sweep(&captures);
    if let Some(held) = &options.hold {
        held_out(&captures, held, options)?;
    }
    if let Some(out) = &options.out {
        write(&table, out)?;
    }
    if let Some(dump) = &options.dump {
        spill(&captures, dump)?;
    }
    Ok(())
}

/// Every reading behind every number above, so a claim about this corpus can
/// be re-checked without a second decode.
fn spill(captures: &[Capture], dump: &Path) -> Fallible<()> {
    if let Some(parent) = dump.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = String::from("capture,phi_deg,left_deg\n");
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
/// Binned at the ring's own spacing and compared only where both captures have
/// a reading: an azimuth one of them never correlated at says nothing about
/// agreement.
fn reproduces(captures: &[Capture]) {
    if captures.len() < 2 {
        return;
    }
    println!(
        "\nagreement: the same azimuth read on two captures, in degrees along the seam. this is \n\
         the premise: a leftover that does not reproduce is a scene and not a camera."
    );
    println!(
        "{:<20} {:<20} {:>7} {:>10} {:>10}",
        "capture", "against", "shared", "apart rms", "spread"
    );
    for (index, one) in captures.iter().enumerate() {
        for other in captures.iter().skip(index + 1) {
            let shared = paired(one, other);
            let apart = seam::rms(shared.iter().map(|(a, b)| a - b));
            let spread = seam::rms(shared.iter().map(|(a, _)| *a));
            println!(
                "{:<20} {:<20} {:>7} {:>10.4} {:>10.4}",
                short(&one.path),
                short(&other.path),
                shared.len(),
                apart,
                spread,
            );
        }
    }
}

/// The two captures' readings at the azimuths both of them reached, in
/// degrees.
fn paired(one: &Capture, other: &Capture) -> Vec<(f64, f64)> {
    let index = |l: &Leftover| (f64::from(l.phi).to_degrees().rem_euclid(360.0) / 5.0) as i32;
    let theirs: BTreeMap<i32, f64> = other
        .left
        .iter()
        .map(|l| (index(l), f64::from(l.perp.to_degrees())))
        .collect();
    one.left
        .iter()
        .filter_map(|l| Some((f64::from(l.perp.to_degrees()), *theirs.get(&index(l))?)))
        .collect()
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
    let pooled: Vec<Leftover> = captures
        .iter()
        .flat_map(|c| c.left.iter().copied())
        .collect();
    // The row a table has to beat: every reading as it stands, with nothing
    // applied. A width whose held-out column sits above this one has cost the
    // capture it was not fitted on.
    println!(
        "{:>8} {:>10.4} {:>10.4}",
        "none",
        seam::rms(pooled.iter().map(|l| f64::from(l.perp.to_degrees()))),
        seam::rms(pooled.iter().map(|l| f64::from(l.perp.to_degrees()))),
    );
    for width in WIDTHS {
        let table = Table::of(&pooled, width);
        let fitted = seam::rms(pooled.iter().map(|l| left_of(&table, l)));
        println!(
            "{width:>8.0} {fitted:>10.4} {:>10.4}",
            rotated(captures, width),
        );
    }
}

/// The leave-one-capture-out mean: each capture predicted by a table built
/// from every other one.
fn rotated(captures: &[Capture], width: f32) -> f64 {
    if captures.len() < 2 {
        return f64::NAN;
    }
    let mut left = Vec::new();
    for (index, held) in captures.iter().enumerate() {
        let pooled: Vec<Leftover> = captures
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .flat_map(|(_, c)| c.left.iter().copied())
            .collect();
        let table = Table::of(&pooled, width);
        left.extend(held.left.iter().map(|l| left_of(&table, l)));
    }
    seam::rms(left.into_iter())
}

/// One reading with the table's answer at its own azimuth taken off it, in
/// degrees.
fn left_of(table: &Table, reading: &Leftover) -> f64 {
    let (sin, cos) = reading.phi.sin_cos();
    f64::from((reading.perp - table.at(cos, sin)).to_degrees())
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
    places: usize,
    frames: usize,
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
            places: 3,
            frames: 2,
            patches: 72,
        };
        for arg in args {
            match arg.split_once('=') {
                Some(("seam", value)) => {
                    if value == "file" || value == "factory" {
                        return Err(USAGE_SEAM.into());
                    }
                    options.seam = Some(seam_fit(value)?);
                }
                Some(("through", value)) => options.through = load(Path::new(value))?,
                Some(("hold", value)) => options.hold = Some(PathBuf::from(value)),
                Some(("out", value)) => options.out = Some(PathBuf::from(value)),
                Some(("dump", value)) => options.dump = Some(PathBuf::from(value)),
                Some(("read", value)) => options.mode = Mode::Read(PathBuf::from(value)),
                Some(("plant", value)) => options.mode = planted(value)?,
                Some(("smooth", value)) => options.smooth = value.parse()?,
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
        Ok(options)
    }
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

const USAGE: &str = "usage: table <file.insv> [<file.insv> ...] seam=roll:0.8,yaw:-2.3,pitch:-0.9,\
cx:-3.3,cy:-11.9] [through=table.txt] [hold=<file.insv>] [smooth=deg] [places=n] [frames=n] [patches=n] [out=path] [dump=path.csv] \
| read=path | plant=size_deg:cycles [out=path]";

const USAGE_SEAM: &str = "this instrument needs one stored fit for every capture: a fit off each \
capture's own frames absorbs that scene into the pose, and two such fits do not leave the same \
quantity behind. seam=roll:..,yaw:..,pitch:..,cx:..,cy:..";
