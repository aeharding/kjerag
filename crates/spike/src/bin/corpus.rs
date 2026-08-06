//! Stage 9's own corpus report, over per-reading dumps, with the reduction as
//! an argument (issue #103, stage 9).
//!
//! ```sh
//! # the six X4 Air flights, reduced the way the shipped path reduces them
//! cargo run --release -p kjerag-spike --bin corpus -- scratch/layer2/corpus/x4-*.csv \
//!   middle=mean seam=roll:0.795,yaw:-2.310,pitch:-0.936,cx:-3.28,cy:-11.91
//! # the same readings with a reading that cannot be a camera refused first
//! cargo run --release -p kjerag-spike --bin corpus -- scratch/layer2/corpus/x4-*.csv \
//!   middle=trimmed seam=<stored>
//! # and with the pose refit off the reduced readings rather than stored
//! cargo run --release -p kjerag-spike --bin corpus -- scratch/layer2/corpus/x4-*.csv \
//!   middle=trimmed seam=corpus
//! ```
//!
//! **This is `--bin table`'s report, arm for arm**, so the two compare
//! directly: the same per-capture rms, the same harmonic-order ladder, the
//! same cross-capture agreement, the same leave-one-capture-out kernel sweep,
//! and the same refusal to fit a pose per capture. What it adds is that the
//! readings arrive one moment at a time rather than already averaged, so the
//! step from readings to a ring is visible and can be changed.
//!
//! **Why it exists.** `--bin table`'s ring comes out of `seam::measure`, which
//! reduces every frame an azimuth correlated on with a MEAN. Those readings
//! are heavy tailed, and stage 9's headline finding - that what a pose leaves
//! does not reproduce between flights - is a statement about that mean. Under
//! a reduction that refuses a reading no calibration could have produced, the
//! same readings from the same frames say something different, and this
//! instrument is how the two are put side by side.
//!
//! One pose for every capture, never `seam=file`, for `--bin table`'s reason:
//! a fit off each capture's own frames absorbs that scene into the pose and
//! the leftovers of two such fits are not the same quantity measured twice.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kjerag_media::Fallible;
use kjerag_meta::CalibrationSet;
use kjerag_render::{Leftover, SeamFit, Size, Table, seam};
use kjerag_spike::seam_fit;
use kjerag_spike::settled::{self, At, Middle};

/// The kernel widths the sweep tries, in degrees of azimuth. `--bin table`'s
/// own list, so the two instruments' columns are the same statistic.
const WIDTHS: [f32; 8] = [4.0, 6.0, 8.0, 10.0, 12.0, 16.0, 24.0, 36.0];

/// How many harmonic orders the structure ladder reports.
const ORDERS: usize = 8;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let mut captures = Vec::new();
    for path in &options.inputs {
        captures.push(Capture::read(path, options.middle, options.places)?);
    }
    if captures.is_empty() {
        return Err(USAGE.into());
    }
    let pose = match options.seam {
        Some(seam) => seam,
        None => corpus_pose(&captures)?,
    };
    report(&mut captures, pose, &options)
}

// ------------------------------------------------------------ one capture

/// One capture's ring, reduced, and what a pose leaves on it.
struct Capture {
    name: String,
    lenses: Vec<kjerag_meta::Lens>,
    frame: Size,
    ring: Vec<seam::Where>,
    /// Every azimuth this capture answered, reduced but with no pose off it.
    read: Vec<At>,
    /// The same with the pose taken off and the plausibility gate applied.
    left: Vec<At>,
    before: f64,
    after: f64,
    refused: usize,
    tolerance: f64,
    moments: usize,
}

/// The moments `--bin table`'s own plan would have read, and no others.
///
/// A dump holds a reading every few seconds of a whole flight; `--bin table`
/// reads `places` moments spread over the middle of the file. Thinning to that
/// answers a question the corpus cannot otherwise: whether a reduction that
/// refuses outliers is reachable from what that instrument sampled, or whether
/// it needs the readings a playing file has and a twelve-place plan does not.
fn thinned(rows: Vec<settled::Row>, places: usize) -> Vec<settled::Row> {
    if places == 0 {
        return rows;
    }
    let mut moments: Vec<f64> = rows.iter().map(|row| row.seconds).collect();
    moments.sort_by(f64::total_cmp);
    moments.dedup();
    let (Some(first), Some(last)) = (moments.first(), moments.last()) else {
        return rows;
    };
    let wanted: Vec<f64> = (0..places)
        .map(|place| {
            let at = first + (last - first) * (place as f64 + 0.5) / places as f64;
            *moments
                .iter()
                .min_by(|one, other| (*one - at).abs().total_cmp(&(*other - at).abs()))
                .unwrap_or(&at)
        })
        .collect();
    rows.into_iter()
        .filter(|row| wanted.iter().any(|at| (row.seconds - at).abs() < 1e-6))
        .collect()
}

impl Capture {
    fn read(path: &Path, middle: Middle, places: usize) -> Fallible<Self> {
        let mut dump = settled::load(path)?;
        dump.rows = thinned(dump.rows, places);
        let calibration = CalibrationSet::from_insv(&dump.source)?;
        let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
        let read = settled::field(&dump.rows, 0.0, f64::INFINITY, middle, false);
        if read.is_empty() {
            return Err(format!("{} holds no readings", path.display()).into());
        }
        let moments = dump
            .rows
            .iter()
            .map(|row| row.seconds.to_bits())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        Ok(Self {
            name: short(path),
            before: seam::rms(read.iter().map(|at| at.along)),
            ring: seam::ring(dump.patches),
            lenses: calibration.lenses.clone(),
            frame,
            read,
            left: Vec::new(),
            after: f64::NAN,
            refused: 0,
            tolerance: f64::NAN,
            moments,
        })
    }

    /// This capture's ring as the fit takes it: one reading per azimuth, on
    /// the sphere direction that azimuth names.
    fn readings(&self) -> Vec<seam::Reading> {
        self.read
            .iter()
            .filter_map(|at| {
                Some(seam::Reading {
                    at: *self.ring.get(at.index)?,
                    along: at.along,
                    across: at.across,
                })
            })
            .collect()
    }

    /// The pose taken off this capture's ring, and the gate applied to what is
    /// left.
    ///
    /// `seam::left`'s own two steps, kept apart only so that the reading count
    /// behind each azimuth survives them: the leftover it returns has no
    /// memory of how many moments it came from and every column here does.
    fn subtract(&mut self, fit: &SeamFit, gate: bool) {
        let base = seam::mapped(&self.lenses, self.frame);
        let corrected = seam::mapped(&fit.applied(&self.lenses), self.frame);
        let mut moved: Vec<At> = self
            .read
            .iter()
            .filter_map(|at| {
                let shift = seam::moved(&base, &corrected, 1, self.ring.get(at.index)?)?;
                Some(At {
                    left: at.along + shift[0],
                    ..*at
                })
            })
            .collect();
        self.after = seam::rms(moved.iter().map(|at| at.left));
        if gate {
            let kept = seam::gated(moved.iter().map(At::leftover).collect());
            let survived: Vec<f32> = kept.readings.iter().map(|l| l.phi).collect();
            self.tolerance = f64::from(kept.tolerance.to_degrees());
            let before = moved.len();
            moved.retain(|at| survived.contains(&at.leftover().phi));
            self.refused = before - moved.len();
            self.after = seam::rms(moved.iter().map(|at| at.left));
        }
        self.left = moved;
    }
}

/// One pose for the whole corpus, fitted on every capture's reduced readings
/// at once. `--bin table`'s `corpus_pose`, over this reduction's rings.
fn corpus_pose(captures: &[Capture]) -> Fallible<SeamFit> {
    let first = captures.first().ok_or("no capture")?;
    let readings: Vec<seam::Reading> = captures.iter().flat_map(Capture::readings).collect();
    Ok(seam::fit_held(
        &readings,
        &first.lenses,
        first.frame,
        &seam::KNOBS,
        seam::RIDGE,
    )
    .ok_or("the pooled readings do not pin a pose")?
    .fit)
}

// ------------------------------------------------------------ the report

fn report(captures: &mut [Capture], pose: SeamFit, options: &Options) -> Fallible<()> {
    println!(
        "reduce: one direction's readings become one number by {}{}{}",
        options.middle.name(),
        match options.places {
            0 => String::new(),
            places => format!(", off {places} moments of each dump and no more"),
        },
        match options.gate {
            true => ", then the ring passes seam::left's plausibility gate",
            false => ", and NO gate is applied (control)",
        },
    );
    println!(
        "seam:   one pose for every capture{}, roll {:+.3} yaw {:+.3} pitch {:+.3} cx {:+.2} \
         cy {:+.2}",
        match options.seam {
            Some(_) => " (stored)",
            None => " (fitted on this corpus, this reduction)",
        },
        pose.roll_deg,
        pose.yaw_deg,
        pose.pitch_deg,
        pose.cx_px,
        pose.cy_px,
    );
    for capture in captures.iter_mut() {
        capture.subtract(&pose, options.gate);
    }
    println!(
        "\n{:<12} {:>8} {:>9} {:>10} {:>10} {:>9} {:>10}",
        "capture", "moments", "azimuths", "factory", "under pose", "refused", "tolerance"
    );
    for capture in captures.iter() {
        println!(
            "{:<12} {:>8} {:>9} {:>10.4} {:>10.4} {:>9} {:>10.3}",
            capture.name,
            capture.moments,
            capture.left.len(),
            capture.before,
            capture.after,
            capture.refused,
            capture.tolerance,
        );
    }
    let pooled: Vec<Leftover> = captures
        .iter()
        .flat_map(|capture| settled::leftovers(&capture.left))
        .collect();
    if pooled.is_empty() {
        return Err("no capture had a reading on its seam".into());
    }
    structure(&pooled);
    reproduces(captures);
    sweep(captures);
    per_capture(captures, options);
    spill(captures, options)
}

/// How much of the pooled leftover each harmonic order can describe.
fn structure(pooled: &[Leftover]) {
    println!(
        "\nstructure: what each harmonic order leaves on the {} pooled readings, degrees rms \n\
         \x20       along the seam. the pass applies orders 0 to 2 already.",
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
                settled::harmonics(f64::from(l.phi), order),
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

/// Whether two captures read the same thing at the same azimuth, which is the
/// premise a static field rests on.
fn reproduces(captures: &[Capture]) {
    if captures.len() < 2 {
        return;
    }
    println!(
        "\nagreement: the same azimuth read on two captures, degrees along the seam. a leftover \n\
         \x20       that does not reproduce is a scene and not a camera: `apart` under `spread` \n\
         \x20       is a camera, `apart` over it is not."
    );
    println!(
        "{:<12} {:<12} {:>7} {:>10} {:>10} {:>10}",
        "capture", "against", "shared", "apart rms", "spread", "5 terms off"
    );
    let mut apart_all = Vec::new();
    let mut spread_all = Vec::new();
    for (index, one) in captures.iter().enumerate() {
        for other in captures.iter().skip(index + 1) {
            let shared = settled::shared(&one.left, &other.left);
            let apart = seam::rms(shared.iter().map(|(a, b)| a.left - b.left));
            let spread = seam::rms(shared.iter().map(|(a, _)| a.left));
            apart_all.push(apart);
            spread_all.push(spread);
            println!(
                "{:<12} {:<12} {:>7} {:>10.4} {:>10.4} {:>10.4}",
                one.name,
                other.name,
                shared.len(),
                apart,
                spread,
                levelled_apart(one, other),
            );
        }
    }
    println!(
        "{:<12} {:<12} {:>7} {:>10.4} {:>10.4}",
        "all pairs",
        "",
        apart_all.len(),
        seam::rms(apart_all.into_iter()),
        seam::rms(spread_all.into_iter()),
    );
}

/// The same two captures with each one's own five terms taken off first: what
/// is left is the part a static table would have to carry.
fn levelled_apart(one: &Capture, other: &Capture) -> f64 {
    let mine = settled::five(&settled::leftovers(&one.left));
    let theirs = settled::five(&settled::leftovers(&other.left));
    let shared = settled::shared(&one.left, &other.left);
    seam::rms(shared.iter().map(|(a, b)| {
        let phi = a.phi.to_radians();
        (a.left - settled::at_phi(&mine, phi)) - (b.left - settled::at_phi(&theirs, phi))
    }))
}

/// What each kernel width leaves on the captures it was fitted to, and on one
/// it was not. `--bin table`'s own sweep, table-only arm.
fn sweep(captures: &[Capture]) {
    println!(
        "\nkernel: what each width leaves, degrees rms along the seam, with a TABLE and no five \n\
         \x20       terms under it - `--bin table`'s own arm, so the two reports compare. `fitted` \n\
         \x20       is measured on the captures the table was built from, `held out` on the one \n\
         \x20       it was not, taken in turn."
    );
    println!("{:>8} {:>10} {:>10}", "deg", "fitted", "held out");
    let pooled: Vec<Leftover> = captures
        .iter()
        .flat_map(|capture| settled::leftovers(&capture.left))
        .collect();
    println!(
        "{:>8} {:>10.4} {:>10.4}",
        "none",
        seam::rms(pooled.iter().map(|l| f64::from(l.perp.to_degrees()))),
        seam::rms(pooled.iter().map(|l| f64::from(l.perp.to_degrees()))),
    );
    for width in WIDTHS {
        let table = Table::of(&pooled, width);
        println!(
            "{width:>8.0} {:>10.4} {:>10.4}",
            seam::rms(pooled.iter().map(|l| left_of(&table, l))),
            rotated(captures, width),
        );
    }
}

/// The leave-one-capture-out mean at one width.
fn rotated(captures: &[Capture], width: f32) -> f64 {
    if captures.len() < 2 {
        return f64::NAN;
    }
    let mut left = Vec::new();
    for (index, held) in captures.iter().enumerate() {
        let table = Table::of(&others(captures, index), width);
        left.extend(
            settled::leftovers(&held.left)
                .iter()
                .map(|l| left_of(&table, l)),
        );
    }
    seam::rms(left.into_iter())
}

/// Every capture's leftovers but one's.
fn others(captures: &[Capture], index: usize) -> Vec<Leftover> {
    captures
        .iter()
        .enumerate()
        .filter(|(other, _)| *other != index)
        .flat_map(|(_, capture)| settled::leftovers(&capture.left))
        .collect()
}

/// One reading with the table's answer at its own azimuth taken off it.
fn left_of(table: &Table, reading: &Leftover) -> f64 {
    let (sin, cos) = reading.phi.sin_cos();
    f64::from((reading.perp - table.at(cos, sin)).to_degrees())
}

/// Each capture predicted by the others, layer by layer.
///
/// The column that decides which layer is owed. `5 terms` is a static
/// per-camera field of the shape `band::Along` already fits live; `5+table`
/// adds the per-direction entries stage 9 asked about; `table only` is stage
/// 9's own arm again, per capture this time.
fn per_capture(captures: &[Capture], options: &Options) {
    println!(
        "\nheld out: each capture predicted by a field fitted on the OTHER captures, degrees rms \n\
         \x20       along the seam at a {:.0} deg kernel. every arm is held out; nothing below is \n\
         \x20       measured on its own data.",
        options.smooth,
    );
    println!(
        "{:<12} {:>9} {:>10} {:>10} {:>10} {:>10}",
        "capture", "azimuths", "pose only", "5 terms", "5+table", "table only"
    );
    let mut totals = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    for (index, held) in captures.iter().enumerate() {
        let train = others(captures, index);
        let test = settled::leftovers(&held.left);
        let each = predict(&train, &test, options.smooth);
        for (all, arm) in totals.iter_mut().zip(&each) {
            all.push((test.len(), *arm));
        }
        println!(
            "{:<12} {:>9} {:>10.4} {:>10.4} {:>10.4} {:>10.4}",
            held.name,
            test.len(),
            each[0],
            each[1],
            each[2],
            each[3],
        );
    }
    let pooled = |all: &[(usize, f64)]| {
        let count: usize = all.iter().map(|(n, _)| n).sum();
        (all.iter().map(|(n, v)| *n as f64 * v * v).sum::<f64>() / count as f64).sqrt()
    };
    println!(
        "{:<12} {:>9} {:>10.4} {:>10.4} {:>10.4} {:>10.4}",
        "all",
        totals[0].iter().map(|(n, _)| n).sum::<usize>(),
        pooled(&totals[0]),
        pooled(&totals[1]),
        pooled(&totals[2]),
        pooled(&totals[3]),
    );
}

/// What the held-out capture reads with each arm taken off it, in degrees rms.
fn predict(train: &[Leftover], test: &[Leftover], smooth: f32) -> [f64; 4] {
    let low = settled::five(train);
    let table = Table::of(train, smooth);
    let at = |l: &Leftover| {
        let (sin, cos) = l.phi.sin_cos();
        (
            settled::at_phi(&low, f64::from(l.phi)),
            f64::from(table.at(cos, sin).to_degrees()),
        )
    };
    let value = |l: &Leftover| f64::from(l.perp.to_degrees());
    [
        seam::rms(test.iter().map(value)),
        seam::rms(test.iter().map(|l| value(l) - at(l).0)),
        seam::rms(test.iter().map(|l| value(l) - at(l).0 - at(l).1)),
        seam::rms(test.iter().map(|l| value(l) - at(l).1)),
    ]
}

/// Every leftover behind every number above, so a claim about this corpus can
/// be re-checked without a second decode.
fn spill(captures: &[Capture], options: &Options) -> Fallible<()> {
    let Some(dump) = &options.dump else {
        return Ok(());
    };
    if let Some(parent) = dump.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = format!(
        "# source: kjerag-spike --bin corpus\n# args: {}\n# reduction: {}\n",
        std::env::args().skip(1).collect::<Vec<_>>().join(" "),
        options.middle.name(),
    );
    text.push_str("capture,phi_deg,left_deg,readings\n");
    for capture in captures {
        for at in &capture.left {
            let _ = writeln!(
                text,
                "{},{:.4},{:.6},{}",
                capture.name, at.phi, at.left, at.readings
            );
        }
    }
    std::fs::write(dump, text)?;
    println!("\nwrote:  {}", dump.display());
    Ok(())
}

fn short(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into()
}

// ------------------------------------------------------------ the options

struct Options {
    inputs: Vec<PathBuf>,
    seam: Option<SeamFit>,
    middle: Middle,
    gate: bool,
    /// How many moments of each dump are read, spread over it, or 0 for all.
    places: usize,
    smooth: f32,
    dump: Option<PathBuf>,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut out = Self {
            inputs: Vec::new(),
            seam: None,
            middle: Middle::Mean,
            gate: true,
            places: 0,
            smooth: kjerag_render::band::SMOOTH_DEG,
            dump: None,
        };
        for arg in args {
            match arg.split_once('=') {
                Some(("seam", "corpus")) => out.seam = None,
                Some(("seam", value)) => {
                    if value == "file" || value == "factory" {
                        return Err(USAGE_SEAM.into());
                    }
                    out.seam = Some(seam_fit(value)?);
                }
                Some(("middle", value)) => {
                    out.middle = Middle::parse(value).ok_or("middle is mean, median or trimmed")?;
                }
                Some(("gate", value)) => out.gate = value != "0",
                Some(("places", value)) => out.places = value.parse()?,
                Some(("smooth", value)) => out.smooth = value.parse()?,
                Some(("dump", value)) => out.dump = Some(PathBuf::from(value)),
                Some(_) => return Err(format!("{USAGE}\n\nunknown: {arg}").into()),
                None => out.inputs.push(PathBuf::from(arg)),
            }
        }
        Ok(out)
    }
}

const USAGE: &str = "usage: corpus <settle-dump.csv> [<settle-dump.csv> ...] \
[seam=corpus|roll:0.8,yaw:-2.3,pitch:-0.9,cx:-3.3,cy:-11.9] [middle=mean|median|trimmed] \
[gate=0] [places=n] [smooth=deg] [dump=path.csv]";

const USAGE_SEAM: &str = "this instrument needs one pose for every capture: a fit off each \
capture's own frames absorbs that scene into the pose, and two such fits do not leave the same \
quantity behind. seam=corpus, or seam=roll:..,yaw:..,pitch:..,cx:..,cy:..";
