//! What `--bin settle` wrote, and the one decision every instrument that
//! reads it has to make: how a direction's readings become one number
//! (issue #103, stage 9 layer 2).
//!
//! Here rather than in one binary because two of them ask it - `--bin
//! converge` of one session and `--bin corpus` of nine captures - and the
//! whole finding is that the answer changes the verdict. Two copies of this
//! would be two chances for the two instruments to disagree about what a
//! reduction is.
//!
//! **The readings are the shipped path's own.** Nothing here correlates
//! anything: `kjerag_render::seam::read_ring_centred` did that, one moment at
//! a time, and this is the arithmetic over what it answered.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kjerag_media::Fallible;
use kjerag_render::{Leftover, seam};

/// One azimuth of one moment, as `--bin settle` wrote it.
#[derive(Clone, Copy)]
pub struct Row {
    pub seconds: f64,
    pub index: usize,
    /// Where on the ring it is, in degrees.
    pub phi: f64,
    /// What the two lenses disagreed by there, in degrees along the seam,
    /// under the calibration the camera wrote. No pose is in this column.
    pub along: f64,
    pub across: f64,
    /// The same with the pose `--bin settle` was given already taken off, in
    /// degrees. An instrument that wants a different pose recomputes it from
    /// `along` and ignores this; one that wants the pose the dump was taken
    /// under reads it.
    pub left: f64,
}

/// One dump: the capture it came off, and every reading in it.
pub struct Dump {
    /// The `.insv` the readings were taken from, off the file's own stamp, so
    /// an instrument can open its calibration without being told twice.
    pub source: PathBuf,
    /// How many azimuths the ring was read at, off the dump's own stamp. An
    /// index is only a direction against this number, and guessing it from the
    /// largest index that answered would guess low on a starved capture.
    pub patches: usize,
    pub rows: Vec<Row>,
}

/// Read one dump, header and all.
pub fn load(path: &Path) -> Fallible<Dump> {
    let text = std::fs::read_to_string(path)?;
    let mut source = None;
    let mut patches = None;
    let mut rows = Vec::new();
    for line in text.lines() {
        if let Some(named) = line.strip_prefix("# file: ") {
            source = Some(PathBuf::from(named.trim()));
            continue;
        }
        if let Some(probe) = line.strip_prefix("# probe: patches=") {
            patches = probe.split_whitespace().next().and_then(|n| n.parse().ok());
            continue;
        }
        if line.starts_with('#') || line.starts_with("seconds") {
            continue;
        }
        let cells: Vec<&str> = line.split(',').collect();
        let [seconds, index, phi, along, across, left, ..] = cells[..] else {
            return Err(format!("{}: {line} is not a settle row", path.display()).into());
        };
        rows.push(Row {
            seconds: seconds.parse()?,
            index: index.parse()?,
            phi: phi.parse()?,
            along: along.parse()?,
            across: across.parse()?,
            left: left.parse()?,
        });
    }
    Ok(Dump {
        source: source.ok_or_else(|| format!("{} has no `# file:` stamp", path.display()))?,
        patches: patches.ok_or_else(|| format!("{} has no `# probe:` stamp", path.display()))?,
        rows,
    })
}

// ------------------------------------------------------------ the reduction

/// How one direction's readings are reduced to one number.
///
/// A mean is what `kjerag_render::seam::measure` does over the frames it
/// reads, and what the band's own exponential average does over the frames it
/// sees, so it is the arm everything shipped inherits. The other two are here
/// because these readings are heavy tailed by the same measurement that made
/// `seam::left` need a gate at all: one azimuth's reading moves by a
/// hundredth of a degree between frames by median absolute deviation and by
/// two tenths by root mean square, and a mean over that population is a
/// statistic about its outliers.
#[derive(Clone, Copy, PartialEq)]
pub enum Middle {
    Mean,
    Median,
    /// The mean of the readings within [`GATE_MADS`] scaled median deviations
    /// of the median, never nearer than [`GATE_FLOOR_DEG`]: `seam::left`'s own
    /// gate rule applied to one direction's readings instead of to a ring's.
    ///
    /// It is the one of the three a GPU can run. The band's per-direction
    /// state is one exponential average and a stream has no median, but
    /// refusing a reading that sits far from what the direction already holds
    /// is a comparison and an early return.
    Trimmed,
    /// The quantile at one end of the population, in the range 0 to 1.
    ///
    /// **For the across-seam axis only, and it is not an outlier rule.** A
    /// middle is the right answer for a population whose spread is noise, and
    /// the along-seam population's is. Across the seam the spread is partly
    /// *signal that is not the camera*: parallax displaces content one way
    /// only (`band::Cell::disparity`, "positive is lens 1's picture displaced
    /// towards the front lens, which is what a near subject does"), so one
    /// direction's readings over a session are a static camera term plus a
    /// one-signed pile of near field on top of it. The static term is the
    /// **far-field limit** of that population, not its middle, and no
    /// symmetric estimator can reach a limit that sits at one end.
    ///
    /// Which end is far is a measurement and not a convention here:
    /// [`At::skew`] reads it off the readings themselves.
    Far(f64),
}

impl Middle {
    pub fn parse(name: &str) -> Option<Self> {
        match name.split_once(':') {
            Some(("far", quantile)) => Some(Self::Far(quantile.parse().ok()?)),
            _ => match name {
                "mean" => Some(Self::Mean),
                "median" => Some(Self::Median),
                "trimmed" => Some(Self::Trimmed),
                _ => None,
            },
        }
    }

    pub fn name(self) -> String {
        match self {
            Self::Mean => "mean".into(),
            Self::Median => "median".into(),
            Self::Trimmed => "trimmed".into(),
            Self::Far(quantile) => format!("the {:.0}th percentile", quantile * 100.0),
        }
    }
}

/// How many scaled median deviations from the middle a reading may sit before
/// it is a correlation on the wrong feature, and the narrowest that tolerance
/// may become, in degrees.
///
/// `seam`'s own `GATE_MADS` and `GATE_FLOOR_DEG`, one axis over: those are
/// private to it and are about a ring's azimuths, these are about one
/// azimuth's moments. Same two numbers because it is the same argument - a
/// capture's calibration does not change while it plays.
pub const GATE_MADS: f64 = 4.0;
pub const GATE_FLOOR_DEG: f64 = 0.10;

/// What a normal population's standard deviation is, in median absolute
/// deviations of it.
pub const MAD_TO_SIGMA: f64 = 1.4826;

/// One direction's answer over some stretch of a session.
#[derive(Clone, Copy)]
pub struct At {
    pub index: usize,
    /// Where on the ring it is, in degrees.
    pub phi: f64,
    /// The reduced reading there, in degrees along the seam, with no pose
    /// taken off it.
    pub along: f64,
    pub across: f64,
    /// What a pose leaves there, in degrees: the dump's own pose, until an
    /// instrument that fits its own overwrites it.
    pub left: f64,
    pub readings: usize,
    /// The standard error of `left`, in degrees, or `NAN` from one
    /// reading. A field is settled when two stretches of it sit no further
    /// apart than this says two answers off that many readings must.
    pub error: f64,
    /// The same for the across-seam column, which is the axis `--bin epi`
    /// works on: `error` is the along column's and the two axes are reduced
    /// from different populations, so one number cannot stand for both.
    pub across_error: f64,
    /// How many of this direction's readings were negative across the seam.
    ///
    /// The depth control. Parallax reaches this axis and reaches it one-signed
    /// at every azimuth (docs/research/seam-two-axis.md 1), so a direction
    /// whose readings are two-signed, or a ring whose sign flips round it, is
    /// not being read by a distance.
    pub across_negative: usize,
    /// How lopsided this direction's across-seam readings are, in degrees:
    /// [`skew`] over the raw population, before any pose.
    pub across_skew: f64,
}

impl At {
    /// This direction as the leftover the repo's own arithmetic takes.
    pub fn leftover(&self) -> Leftover {
        Leftover {
            phi: self.phi.to_radians() as f32,
            perp: self.left.to_radians() as f32,
            weight: 1.0,
        }
    }
}

/// The reduced field over one stretch, gated the way one ring's readings are
/// gated.
///
/// `to` is exclusive and `gate` is the plausibility test [`gated`] applies;
/// the control that turns it off says whether a result belongs to the gate or
/// to the reduction.
pub fn field(rows: &[Row], from: f64, to: f64, middle: Middle, gate: bool) -> Vec<At> {
    let mut all: BTreeMap<usize, Vec<Row>> = BTreeMap::new();
    for row in rows.iter().filter(|r| r.seconds >= from && r.seconds < to) {
        all.entry(row.index).or_default().push(*row);
    }
    let reduced: Vec<At> = all.values().map(|rows| reduce(rows, middle)).collect();
    if !gate {
        return reduced;
    }
    let (kept, _) = gated(&leftovers(&reduced));
    let survived: Vec<f32> = kept.iter().map(|l| l.phi).collect();
    reduced
        .into_iter()
        .filter(|at| survived.contains(&at.leftover().phi))
        .collect()
}

/// One ring's plausibility gate: [`kjerag_render::seam::left`]'s own rule, in
/// a form that hands back what survived rather than folding the pose
/// subtraction into the same call.
///
/// It is a copy because the shipped one is written for the along-seam axis and
/// takes readings rather than leftovers; the rule itself - a median, four
/// scaled median deviations, a floor - is the shipped rule and the constants
/// below are read off it. Returns the survivors and the tolerance, in radians.
pub fn gated(all: &[Leftover]) -> (Vec<Leftover>, f32) {
    let values: Vec<f64> = all.iter().map(|l| f64::from(l.perp)).collect();
    if values.is_empty() {
        return (Vec::new(), f32::INFINITY);
    }
    let centre = median(&values);
    let scatter = median(
        &values
            .iter()
            .map(|value| (value - centre).abs())
            .collect::<Vec<_>>(),
    );
    let tolerance = (GATE_MADS * scatter).max(GATE_FLOOR_DEG.to_radians());
    (
        all.iter()
            .copied()
            .filter(|l| (f64::from(l.perp) - centre).abs() <= tolerance)
            .collect(),
        tolerance as f32,
    )
}

/// One direction's readings, reduced.
pub fn reduce(rows: &[Row], middle: Middle) -> At {
    let along: Vec<f64> = rows.iter().map(|row| row.along).collect();
    let across: Vec<f64> = rows.iter().map(|row| row.across).collect();
    let left: Vec<f64> = rows.iter().map(|row| row.left).collect();
    At {
        index: rows[0].index,
        phi: rows[0].phi,
        along: reduced(&along, middle),
        across: reduced(&across, middle),
        left: reduced(&left, middle),
        readings: rows.len(),
        error: error_of(&left, middle),
        across_error: error_of(&across, middle),
        across_negative: across.iter().filter(|value| **value < 0.0).count(),
        across_skew: skew(&across),
    }
}

/// The reduced value of one axis.
fn reduced(all: &[f64], middle: Middle) -> f64 {
    match middle {
        Middle::Mean => all.iter().sum::<f64>() / all.len() as f64,
        Middle::Median => median(all),
        Middle::Trimmed => {
            let kept = kept_of(all);
            kept.iter().sum::<f64>() / kept.len().max(1) as f64
        }
        Middle::Far(at) => quantile(all, at),
    }
}

/// The `at` quantile of a population, by nearest rank on the sorted readings.
pub fn quantile(all: &[f64], at: f64) -> f64 {
    if all.is_empty() {
        return f64::NAN;
    }
    let mut all = all.to_vec();
    all.sort_by(f64::total_cmp);
    let rank = (at * (all.len() - 1) as f64).round() as usize;
    all[rank.min(all.len() - 1)]
}

/// How lopsided one direction's readings are, in degrees: the far tail's reach
/// past the median less the near tail's.
///
/// The depth control. Noise is symmetric and parallax is not, so a population
/// that is one camera plus a scene's distances leans, and it leans the same
/// way at every azimuth of the ring, because a near subject is displaced
/// one-signed at all of them (docs/research/seam-two-axis.md 1). A ring whose
/// azimuths lean in both directions is not being read by a distance.
pub fn skew(all: &[f64]) -> f64 {
    let middle = median(all);
    (quantile(all, 0.90) - middle) - (middle - quantile(all, 0.10))
}

/// The readings a trimmed reduction keeps.
fn kept_of(all: &[f64]) -> Vec<f64> {
    let (centre, spread) = (median(all), MAD_TO_SIGMA * mad(all));
    let tolerance = (GATE_MADS * spread).max(GATE_FLOOR_DEG);
    all.iter()
        .copied()
        .filter(|value| (value - centre).abs() <= tolerance)
        .collect()
}

/// How firmly the reduced value is held, in degrees.
///
/// Each reduction's own spread over the root of its own count, and `NAN` from
/// one reading rather than zero: one reading says nothing about how firmly it
/// is held.
fn error_of(all: &[f64], middle: Middle) -> f64 {
    let (spread, count) = match middle {
        Middle::Mean => {
            let mean = all.iter().sum::<f64>() / all.len() as f64;
            (seam::rms(all.iter().map(|value| value - mean)), all.len())
        }
        Middle::Median | Middle::Far(_) => (MAD_TO_SIGMA * mad(all), all.len()),
        Middle::Trimmed => {
            let kept = kept_of(all);
            let mean = kept.iter().sum::<f64>() / kept.len().max(1) as f64;
            (seam::rms(kept.iter().map(|value| value - mean)), kept.len())
        }
    };
    match count > 1 {
        true => spread * (count as f64 / (count - 1) as f64).sqrt() / (count as f64).sqrt(),
        false => f64::NAN,
    }
}

/// The middle value.
pub fn median(all: &[f64]) -> f64 {
    let mut all = all.to_vec();
    all.sort_by(f64::total_cmp);
    all.get(all.len() / 2).copied().unwrap_or(f64::NAN)
}

/// The median absolute deviation from the median: the spread of a heavy-tailed
/// population, which is what `seam::left`'s own gate is built on.
pub fn mad(all: &[f64]) -> f64 {
    let centre = median(all);
    median(
        &all.iter()
            .map(|value| (value - centre).abs())
            .collect::<Vec<_>>(),
    )
}

// ------------------------------------------------------------ the arithmetic

/// The same azimuth in two fields.
pub fn shared(one: &[At], other: &[At]) -> Vec<(At, At)> {
    let theirs: BTreeMap<usize, At> = other.iter().map(|at| (at.index, *at)).collect();
    one.iter()
        .filter_map(|at| Some((*at, *theirs.get(&at.index)?)))
        .collect()
}

/// A whole field as the leftovers the repo's own arithmetic takes.
pub fn leftovers(field: &[At]) -> Vec<Leftover> {
    field.iter().map(At::leftover).collect()
}

/// The five terms the band fits per session, in degrees, off these leftovers.
///
/// Ordinary least squares over the same basis `band::Along` is written in,
/// which is `--bin table`'s own order-2 arm. `Along::fit` solves the same
/// system with a ridge of one direction's evidence on it; on a ring of fifty
/// readings the two answers differ by that ridge and by nothing else.
pub fn five(left: &[Leftover]) -> Vec<f64> {
    let rows: Vec<(Vec<f64>, f64)> = left
        .iter()
        .map(|l| (basis(f64::from(l.phi)), f64::from(l.perp.to_degrees())))
        .collect();
    seam::least_squares(&rows).map_or_else(|| vec![0.0; 5], |fit| fit.params)
}

/// The real Fourier basis up to `order` cycles round the circle.
pub fn harmonics(phi: f64, order: usize) -> Vec<f64> {
    let mut row = vec![1.0];
    for cycle in 1..=order {
        row.push((cycle as f64 * phi).cos());
        row.push((cycle as f64 * phi).sin());
    }
    row
}

/// The five basis functions, which is [`harmonics`] at order two.
pub fn basis(phi: f64) -> Vec<f64> {
    harmonics(phi, 2)
}

/// What a set of coefficients says at one azimuth, in degrees.
pub fn at_phi(terms: &[f64], phi: f64) -> f64 {
    harmonics(phi, terms.len() / 2)
        .iter()
        .zip(terms)
        .map(|(term, coefficient)| term * coefficient)
        .sum()
}
