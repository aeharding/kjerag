//! Whether the across-seam constant is the camera or the session: the same
//! ring residual, decomposed the same way, on every flight the corpus holds.
//!
//! ```sh
//! # every X4 flight, through the pose the app draws them with
//! cargo run --release -p kjerag-spike --bin constant -- \
//!   ~/Videos/Insta/VID_20260501_183417_00_002.insv \
//!   ~/Videos/Insta/VID_20260714_193252_00_006.insv seam=pool
//! # the control that says the reading can find a constant at all
//! cargo run --release -p kjerag-spike --bin constant -- <file.insv> seam=pool plant=0.49
//! ```
//!
//! **The question.** `offset_v6` is byte identical on every X4 Air capture in
//! the corpus - one md5, six files - so a calibration is the same number on
//! all of them and can only carry an error that is the same number on all of
//! them. If the across-seam constant is flight stable, a fixed calibration is
//! a candidate carrier for it. If it moves flight to flight, no fixed
//! calibration can be its carrier and v6 is out, whatever else v6 is worth.
//!
//! **Two axes, and the second one is the control.** Parallax is displacement
//! towards the front lens along a baseline perpendicular to every direction on
//! the seam circle, so it cannot reach the along-seam axis at all
//! (docs/research/insv-format.md 4.9). The along-seam constant is therefore
//! camera and pose and nothing else, and its spread across flights is what
//! this instrument's own constant-detection floor looks like on real footage.
//! The across-seam constant carries that same floor plus whatever the scene
//! put there. **Reading the across column without the along column beside it
//! would call the instrument's own spread a finding**, which is the whole
//! reason both are printed.
//!
//! **What a moving across-seam constant does NOT distinguish.** A flight's own
//! mean parallax and a per-session geometry error both move it, and this does
//! not tell them apart. It does not have to: the verdict it is built for is
//! whether a *fixed* calibration can carry the term, and neither of those is
//! one. Say it that way and do not say more.
//!
//! **The floor is computed and not assumed.** A constant fitted on a partial
//! arc is not pinned as well as one fitted round a whole circle, and how much
//! worse depends on where the gap is. The standard error printed is the real
//! one - the residual scatter times the diagonal of the fit's own inverse
//! normal matrix - so a flight whose sites sit in one place says so in its
//! own error bar rather than in a footnote.

use std::path::{Path, PathBuf};

use kjerag_media::Fallible;
use kjerag_meta::CalibrationSet;
use kjerag_render::{SeamFit, Size, seam};
use kjerag_spike::fit_arg;

const USAGE: &str = "usage: constant <file.insv> [<file.insv> ...] [seam=pool|<five knobs>] \
                     [places=] [frames=] [patches=] [plant=<deg>]";

/// How many source pixels one degree is at the seam, for the second column of
/// every reading: the same conversion `--bin ceiling` quotes its across-seam
/// numbers in, so the two instruments' numbers are the same statistic.
const SRC_PX_PER_DEG: f64 = 16.33;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let fit = options.fit()?;
    println!(
        "pose:   roll:{:.3},yaw:{:.3},pitch:{:.3},cx:{:.2},cy:{:.2}",
        fit.roll_deg, fit.yaw_deg, fit.pitch_deg, fit.cx_px, fit.cy_px,
    );
    println!(
        "plan:   {} places x {} frames, {} azimuths{}",
        options.places,
        options.frames,
        options.patches,
        match options.plant {
            0.0 => String::new(),
            deg => format!(", ACROSS COLUMN PLANTED with {deg:+.3} deg"),
        },
    );
    println!(
        "{:<34} {:>5} {:>7} {:>7}  {:>18} {:>7} {:>7}  {:>18} {:>7} {:>7}",
        "capture",
        "sites",
        "arc",
        "gap",
        "across DC (deg)",
        "1cyc",
        "rms",
        "along DC (deg)",
        "1cyc",
        "rms",
    );
    let mut rows = Vec::new();
    for path in &options.inputs {
        let row = measure(path, &fit, &options)?;
        say(&row);
        rows.push(row);
    }
    verdict(&rows);
    Ok(())
}

// ------------------------------------------------------------ one capture

/// One flight's ring, decomposed.
struct Row {
    name: String,
    sites: usize,
    /// How much of the circle carried a site, and the widest stretch that
    /// carried none, in degrees.
    arc_deg: f64,
    gap_deg: f64,
    /// The constant and the one-cycle amplitude on each axis, across the seam
    /// first and along it second, in degrees. The constant carries its own
    /// standard error.
    across: Harmonic,
    along: Harmonic,
}

/// A constant, a one-cycle amplitude, and how well the constant is pinned.
struct Harmonic {
    constant: f64,
    error: f64,
    cycle: f64,
    rms: f64,
}

fn measure(path: &Path, fit: &SeamFit, options: &Options) -> Fallible<Row> {
    let calibration = CalibrationSet::from_insv(path)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = calibration.lenses.clone();
    let files = kjerag_render::capture_set::resolve(path).files;
    let readings = seam::measure(&files, &lenses, frame, &options.plan())?;
    let left = seam::predicted(&readings, fit, &lenses, frame);
    if left.is_empty() {
        return Err(format!("{}: no azimuth on the seam correlated", name(path)).into());
    }
    let phis: Vec<f64> = left.iter().map(|(reading, _)| reading.at.phi).collect();
    let (arc_deg, gap_deg) = coverage(&phis);
    // The plant goes on the across column only, so the along column stays a
    // control while the plant is being read back.
    let across: Vec<f64> = left
        .iter()
        .map(|(_, axes)| axes[1] + options.plant)
        .collect();
    let along: Vec<f64> = left.iter().map(|(_, axes)| axes[0]).collect();
    Ok(Row {
        name: name(path),
        sites: left.len(),
        arc_deg,
        gap_deg,
        across: harmonic(&phis, &across).ok_or("the across readings do not pin a constant")?,
        along: harmonic(&phis, &along).ok_or("the along readings do not pin a constant")?,
    })
}

fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

// ------------------------------------------------------------ the fit

/// `a0 + a1 cos(phi) + b1 sin(phi)`, and how well `a0` is pinned by the
/// azimuths that carried a reading.
///
/// One cycle and no more. Two named calibration errors put a one-cycle term on
/// this axis and nothing a pose can do puts a constant there
/// (`--bin ceiling`), so the one cycle is here to be taken *out* - a constant
/// read without it would collect whatever a tilted pose left round the ring
/// and call it DC.
fn harmonic(phis: &[f64], values: &[f64]) -> Option<Harmonic> {
    let basis = |phi: f64| [1.0, phi.cos(), phi.sin()];
    let mut normal = [[0.0f64; 3]; 3];
    let mut right = [0.0f64; 3];
    for (phi, value) in phis.iter().zip(values) {
        let row = basis(*phi);
        for i in 0..3 {
            right[i] += row[i] * value;
            for j in 0..3 {
                normal[i][j] += row[i] * row[j];
            }
        }
    }
    let inverse = invert(normal)?;
    let params = [0, 1, 2].map(|i| (0..3).map(|j| inverse[i][j] * right[j]).sum::<f64>());
    let residual = |index: usize| {
        let row = basis(phis[index]);
        values[index] - (0..3).map(|i| row[i] * params[i]).sum::<f64>()
    };
    let count = phis.len();
    if count <= 3 {
        return None;
    }
    let scatter =
        ((0..count).map(|i| residual(i).powi(2)).sum::<f64>() / (count - 3) as f64).sqrt();
    Some(Harmonic {
        constant: params[0],
        // The real one: the residual scatter through the fit's own geometry,
        // so an arc with a hole in it reports a wider bar than a whole ring
        // of the same scatter would.
        error: scatter * inverse[0][0].max(0.0).sqrt(),
        cycle: params[1].hypot(params[2]),
        rms: scatter,
    })
}

/// A 3x3 inverse by cofactors. `None` where the azimuths do not span enough of
/// the circle to tell a constant from a cycle, which is a refusal and not a
/// number with a big error bar.
fn invert(m: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let cofactor = |i: usize, j: usize| {
        let rows: Vec<usize> = (0..3).filter(|r| *r != i).collect();
        let cols: Vec<usize> = (0..3).filter(|c| *c != j).collect();
        let minor =
            m[rows[0]][cols[0]] * m[rows[1]][cols[1]] - m[rows[0]][cols[1]] * m[rows[1]][cols[0]];
        match (i + j) % 2 {
            0 => minor,
            _ => -minor,
        }
    };
    let determinant = (0..3).map(|j| m[0][j] * cofactor(0, j)).sum::<f64>();
    // Scale-free: the normal matrix grows with the site count, so an absolute
    // floor here would refuse a big ring and accept a small one.
    if determinant.abs() <= f64::EPSILON * m[0][0].abs().powi(3) {
        return None;
    }
    Some(std::array::from_fn(|i| {
        std::array::from_fn(|j| cofactor(j, i) / determinant)
    }))
}

/// How much of the circle carried a site and the widest stretch that carried
/// none, both in degrees.
fn coverage(phis: &[f64]) -> (f64, f64) {
    let mut sorted: Vec<f64> = phis.to_vec();
    sorted.sort_by(f64::total_cmp);
    let Some((first, last)) = sorted.first().zip(sorted.last()) else {
        return (0.0, 360.0);
    };
    let wrap = std::f64::consts::TAU - (last - first);
    let widest = sorted
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .fold(wrap, f64::max);
    (360.0 - widest.to_degrees(), widest.to_degrees())
}

// ------------------------------------------------------------ the report

fn say(row: &Row) {
    println!(
        "{:<34} {:>5} {:>6.0}d {:>6.0}d  {:>8.4} +-{:.4} {:>7.4} {:>7.4}  \
         {:>8.4} +-{:.4} {:>7.4} {:>7.4}",
        row.name,
        row.sites,
        row.arc_deg,
        row.gap_deg,
        row.across.constant,
        row.across.error,
        row.across.cycle,
        row.across.rms,
        row.along.constant,
        row.along.error,
        row.along.cycle,
        row.along.rms,
    );
}

/// The two spreads side by side, which is the whole verdict: how far the
/// across-seam constant moves between flights, against how far the axis no
/// scene can reach moves between the same flights.
fn verdict(rows: &[Row]) {
    if rows.len() < 2 {
        println!(
            "\nonly {} capture: nothing to compare it against",
            rows.len()
        );
        return;
    }
    let spread = |pick: fn(&Row) -> &Harmonic| {
        let values: Vec<f64> = rows.iter().map(|row| pick(row).constant).collect();
        let low = values.iter().copied().fold(f64::INFINITY, f64::min);
        let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let bars = rows
            .iter()
            .map(|row| pick(row).error)
            .fold(0.0f64, f64::max);
        (high - low, bars)
    };
    let (across, across_bar) = spread(|row| &row.across);
    let (along, along_bar) = spread(|row| &row.along);
    println!("\nspread of the constant across {} captures:", rows.len());
    println!(
        "  across the seam  {across:.4} deg ({:.2} src px), widest error bar {across_bar:.4}",
        across * SRC_PX_PER_DEG,
    );
    println!(
        "  along the seam   {along:.4} deg ({:.2} src px), widest error bar {along_bar:.4}   \
         <- no scene can reach this axis",
        along * SRC_PX_PER_DEG,
    );
    println!("\n{}", reading(across, across_bar, along));
}

/// What those two numbers mean for a fixed calibration, in the only three
/// answers the measurement can give.
fn reading(across: f64, across_bar: f64, along: f64) -> &'static str {
    // A spread inside the error bars is not a spread. Below that there is
    // nothing to explain and the constant is the same number every flight.
    if across <= across_bar {
        return "FLIGHT STABLE: the across-seam constant moves less than one flight's own error \
                bar, so a fixed calibration can carry it and v6 stays a candidate.";
    }
    // The along axis carries the instrument's own flight-to-flight scatter
    // and nothing a scene put there, so a smaller across spread than that is
    // not a reading of the scene either.
    if across <= along {
        return "FLIGHT STABLE: the across-seam constant moves no further between flights than \
                the axis no scene can reach does, so what moves is the instrument and a fixed \
                calibration can still carry the term.";
    }
    "FLIGHT VARYING: the across-seam constant moves further between flights than the axis no \
     scene can reach, so something per session is on it. No fixed calibration can carry that, \
     v6 included. Which per-session thing it is - the flights' own parallax or their own \
     geometry - this does not say and must not be read as saying."
}

// ------------------------------------------------------------ the line

struct Options {
    inputs: Vec<PathBuf>,
    seam: String,
    places: usize,
    frames: usize,
    patches: usize,
    plant: f64,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            inputs: Vec::new(),
            seam: "pool".to_owned(),
            places: 3,
            frames: 8,
            patches: 72,
            plant: 0.0,
        };
        for arg in args {
            let Some((key, value)) = arg.split_once('=') else {
                options.inputs.push(PathBuf::from(arg));
                continue;
            };
            match key {
                "seam" => options.seam = value.to_owned(),
                "places" => options.places = value.parse()?,
                "frames" => options.frames = value.parse()?,
                "patches" => options.patches = value.parse()?,
                "plant" => options.plant = value.parse()?,
                _ => return Err(format!("{USAGE}\nunknown argument {key}").into()),
            }
        }
        if options.inputs.is_empty() {
            return Err(USAGE.into());
        }
        Ok(options)
    }

    fn fit(&self) -> Fallible<SeamFit> {
        fit_arg(&self.seam, self.inputs.first().map(PathBuf::as_path))
    }

    fn plan(&self) -> seam::Plan {
        seam::Plan {
            places: self.places,
            frames: self.frames,
            probe: seam::Probe {
                patches: self.patches,
                ..seam::Probe::default()
            },
            ..seam::Plan::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Azimuths round a whole circle, and a field with a known constant and a
    /// known one-cycle term on it.
    fn ring(count: usize, constant: f64, cycle: f64) -> (Vec<f64>, Vec<f64>) {
        let phis: Vec<f64> = (0..count)
            .map(|i| i as f64 / count as f64 * std::f64::consts::TAU)
            .collect();
        let values = phis
            .iter()
            .map(|phi| constant + cycle * phi.cos())
            .collect();
        (phis, values)
    }

    /// The plant: a constant that is there is read back at its own size, and
    /// the one-cycle term beside it does not leak into it.
    #[test]
    fn a_planted_constant_reads_back_at_its_own_size() {
        let (phis, values) = ring(72, -0.49, 0.30);
        let read = harmonic(&phis, &values).expect("a whole ring pins a constant");
        assert!((read.constant + 0.49).abs() < 1e-9, "{}", read.constant);
        assert!((read.cycle - 0.30).abs() < 1e-9, "{}", read.cycle);
        assert!(read.rms < 1e-9, "{}", read.rms);
    }

    /// The null: a one-cycle term with no constant under it reads no
    /// constant, so the column cannot manufacture one out of a tilted pose.
    #[test]
    fn a_pure_cycle_reads_no_constant() {
        let (phis, values) = ring(72, 0.0, 0.42);
        let read = harmonic(&phis, &values).expect("a whole ring pins a constant");
        assert!(read.constant.abs() < 1e-9, "{}", read.constant);
    }

    /// A partial arc still pins a constant, and says how much worse: the same
    /// scatter over a third of the circle reports a wider bar than over all
    /// of it, which is what stops a thin flight being quoted like a full one.
    #[test]
    fn a_thin_arc_reports_a_wider_error_bar() {
        let scatter = |phis: &[f64]| {
            let values: Vec<f64> = phis
                .iter()
                .enumerate()
                .map(|(i, _)| match i % 2 {
                    0 => 0.01,
                    _ => -0.01,
                })
                .collect();
            harmonic(phis, &values).expect("enough sites").error
        };
        let whole: Vec<f64> = (0..72)
            .map(|i| i as f64 / 72.0 * std::f64::consts::TAU)
            .collect();
        let third: Vec<f64> = (0..24)
            .map(|i| i as f64 / 72.0 * std::f64::consts::TAU)
            .collect();
        assert!(
            scatter(&third) > 2.0 * scatter(&whole),
            "{} against {}",
            scatter(&third),
            scatter(&whole),
        );
    }

    /// Sites in one place do not pin a constant at all, and that is a refusal
    /// rather than a number with a large bar beside it.
    #[test]
    fn one_azimuth_pins_nothing() {
        let phis = vec![1.0; 8];
        let values = vec![0.2; 8];
        assert!(harmonic(&phis, &values).is_none());
    }

    /// The gap is the widest stretch with no site in it, wrap included: a ring
    /// missing a third reports that third and not the two ends of it.
    #[test]
    fn the_widest_hole_is_found_across_the_wrap() {
        let phis: Vec<f64> = (0..48)
            .map(|i| i as f64 / 72.0 * std::f64::consts::TAU)
            .collect();
        let (arc, gap) = coverage(&phis);
        // 48 sites five degrees apart reach 235 degrees round, so the hole is
        // the 125 the last site does not close.
        assert!((gap - 125.0).abs() < 1e-9, "{gap}");
        assert!((arc - 235.0).abs() < 1e-9, "{arc}");
    }
}
