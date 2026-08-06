//! How long one session's per-direction along-seam field takes to stop
//! moving, and whether the half of a session it was fitted on predicts the
//! half it was not (issue #103, stage 9 layer 2).
//!
//! ```sh
//! # what a span of one flight buys, and what a table off half of it is worth
//! cargo run --release -p kjerag-spike --bin converge -- scratch/layer2/may01.csv
//! # the same, with the table off one window written down for `--bin crossing`
//! cargo run --release -p kjerag-spike --bin converge -- scratch/layer2/may01.csv \
//!   window=0:600 out=scratch/layer2/may01-first10.txt
//! ```
//!
//! It reads what `--bin settle` dumped and decodes nothing. Every number it
//! prints is built out of the repo's own arithmetic: the accumulated field is
//! the per-azimuth mean `kjerag_render::seam::measure` would have produced
//! over that span, gated by `kjerag_render::seam::gated`, and the table is
//! `kjerag_render::Table::of` with its own kernel, ridge and limit.
//!
//! **Stage 9 asked whether the field is a static function of azimuth ACROSS
//! flights and answered no** (docs/research/stage9.md). This asks the question
//! one flight at a time, which is the only form left: a leftover that
//! reproduces within a session and not between them is a per-session
//! correction or it is nothing.
//!
//! **Two spans that overlap are not two measurements.** The convergence
//! column that decides is `windows:`, where the spans compared are disjoint
//! stretches of the same flight; the growing-prefix column underneath it is
//! the same data seen the way a player would accumulate it and cannot fall
//! below its own final answer by construction.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use kjerag_media::Fallible;
use kjerag_render::{Leftover, Table, band, seam};

/// The spans a convergence is reported at, in seconds.
const SPANS: [f64; 6] = [30.0, 60.0, 120.0, 300.0, 600.0, 1200.0];

/// The kernel widths a table is tried at, in degrees of azimuth. `--bin
/// table`'s own sweep, so the two instruments' columns compare.
const WIDTHS: [f32; 8] = [4.0, 6.0, 8.0, 10.0, 12.0, 16.0, 24.0, 36.0];

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let mut rows = load(&options.input)?;
    if let Some((size, cycles)) = options.plant {
        // The positive control: a field of a known size and a known number of
        // cycles, put into every reading of every moment, so a run that finds
        // nothing has been shown able to find something. Six cycles is an
        // order above anything the five terms can describe, which is `--bin
        // table`'s own plant and for its reason.
        for row in &mut rows {
            row.left += size * (cycles * row.phi.to_radians()).cos();
        }
        println!("plant:  {size:.3} deg, {cycles:.0} cycles round the ring, in every reading");
    }
    if rows.is_empty() {
        return Err(format!("{} holds no readings", options.input.display()).into());
    }
    println!(
        "read:   {} readings from {}, {:.0} to {:.0} s",
        rows.len(),
        options.input.display(),
        rows.first().map_or(0.0, |row| row.seconds),
        rows.last().map_or(0.0, |row| row.seconds),
    );
    repeatability(&rows);
    coverage(&rows);
    windows(&rows, &options);
    prefixes(&rows, &options);
    holdout(&rows, &options);
    against(&rows, &options)?;
    write_table(&rows, &options)
}

// ------------------------------------------------------------ the readings

/// One azimuth of one moment, as `--bin settle` wrote it.
#[derive(Clone, Copy)]
struct Row {
    seconds: f64,
    index: usize,
    phi: f64,
    /// What the pose leaves there, in degrees along the seam.
    left: f64,
}

fn load(path: &Path) -> Fallible<Vec<Row>> {
    let text = std::fs::read_to_string(path)?;
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.starts_with("seconds") {
            continue;
        }
        let cells: Vec<&str> = line.split(',').collect();
        let [seconds, index, phi, _along, _across, left, ..] = cells[..] else {
            return Err(format!("{}: {line} is not a settle row", path.display()).into());
        };
        rows.push(Row {
            seconds: seconds.parse()?,
            index: index.parse()?,
            phi: phi.parse()?,
            left: left.parse()?,
        });
    }
    Ok(rows)
}

/// One azimuth of one accumulated field: its mean, and how firmly that mean
/// is held.
#[derive(Clone, Copy)]
struct At {
    index: usize,
    /// Where on the ring it is, in degrees.
    phi: f64,
    /// The mean reading there over the stretch, in degrees.
    mean: f64,
    readings: usize,
    /// The standard error of that mean, in degrees, or `NAN` from one reading.
    /// A field is converged when the fields off two stretches sit no further
    /// apart than this says two means of that many readings must.
    error: f64,
}

/// The accumulated field over one stretch: every azimuth's mean reading over
/// the moments inside it, gated the way one ring's readings are gated.
///
/// This is what `seam::measure` returns for a file, restricted to a span of
/// it: `measure` averages each azimuth over every frame it read, and this
/// averages each azimuth over every frame inside the span.
fn field(rows: &[Row], from: f64, to: f64, middle: Middle) -> Vec<At> {
    field_gated(rows, from, to, middle, true)
}

/// The same, with `seam::left`'s plausibility gate on the accumulated field
/// switched off, which is the control that says whether a result belongs to
/// the gate or to the way the readings were reduced.
fn field_gated(rows: &[Row], from: f64, to: f64, middle: Middle, gate: bool) -> Vec<At> {
    let mut all: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
    let mut phi: BTreeMap<usize, f64> = BTreeMap::new();
    for row in rows.iter().filter(|r| r.seconds >= from && r.seconds < to) {
        all.entry(row.index).or_default().push(row.left);
        phi.insert(row.index, row.phi);
    }
    let means: Vec<At> = all
        .iter()
        .map(|(index, readings)| middle_of(*index, phi[index], readings, middle))
        .collect();
    if !gate {
        return means;
    }
    let kept = seam::gated(means.iter().map(leftover).collect());
    let survived: Vec<f32> = kept.readings.iter().map(|l| l.phi).collect();
    means
        .into_iter()
        .filter(|at| survived.contains(&leftover(at).phi))
        .collect()
}

/// One azimuth of a field as the leftover the repo's own arithmetic takes.
fn leftover(at: &At) -> Leftover {
    Leftover {
        phi: at.phi.to_radians() as f32,
        perp: at.mean.to_radians() as f32,
        weight: 1.0,
    }
}

/// How one azimuth's readings are reduced to one number.
///
/// A mean is what `seam::measure` does over the frames it reads, and it is the
/// arm layer 2 would inherit. A median is here because these readings are
/// heavy tailed by the same measurement that made `seam::left` need a gate:
/// one correlation on the wrong feature moves a mean by its whole size and a
/// median not at all, so which of the two is accumulated is a design question
/// and not a detail.
#[derive(Clone, Copy, PartialEq)]
enum Middle {
    Mean,
    Median,
    /// The mean of the readings within four scaled median deviations of the
    /// median, which is `seam::left`'s own gate rule applied to one azimuth's
    /// readings instead of to a ring's.
    ///
    /// It is here because a median is not what a GPU accumulates: the band's
    /// per-direction state is one exponential average and a stream has no
    /// median. Refusing a reading that sits far from what the direction
    /// already holds is a comparison and an early return, and this is the
    /// offline shape of that rule.
    Trimmed,
}

/// One azimuth's answer and the standard error of it.
fn middle_of(index: usize, phi: f64, readings: &[f64], middle: Middle) -> At {
    let count = readings.len();
    let kept: Vec<f64> = match middle {
        Middle::Trimmed => {
            let (centre, spread) = (median(readings), MAD_TO_SIGMA * mad(readings));
            let tolerance = (GATE_MADS * spread).max(GATE_FLOOR_DEG);
            readings
                .iter()
                .copied()
                .filter(|value| (value - centre).abs() <= tolerance)
                .collect()
        }
        _ => readings.to_vec(),
    };
    let mean = match middle {
        Middle::Mean => readings.iter().sum::<f64>() / count as f64,
        Middle::Median => median(readings),
        Middle::Trimmed => kept.iter().sum::<f64>() / kept.len().max(1) as f64,
    };
    // The spread the error is taken from is the one that matches: a mean's is
    // the sample's own, and a median's is the scaled median absolute
    // deviation, which is what a median's standard error is written in.
    let spread = match middle {
        Middle::Mean => seam::rms(readings.iter().map(|value| value - mean)),
        Middle::Median => MAD_TO_SIGMA * mad(readings),
        Middle::Trimmed => seam::rms(kept.iter().map(|value| value - mean)),
    };
    At {
        index,
        phi,
        mean,
        readings: count,
        // `n - 1` under the root because `spread` is about the sample's own
        // mean, and `NAN` from one reading rather than zero: one reading says
        // nothing about how firmly it is held.
        error: match count > 1 {
            true => spread * (count as f64 / (count - 1) as f64).sqrt() / (count as f64).sqrt(),
            false => f64::NAN,
        },
    }
}

/// The same azimuth in two fields.
fn shared(one: &[At], other: &[At]) -> Vec<(At, At)> {
    let theirs: BTreeMap<usize, At> = other.iter().map(|at| (at.index, *at)).collect();
    one.iter()
        .filter_map(|at| Some((*at, *theirs.get(&at.index)?)))
        .collect()
}

/// A whole field as the leftovers the repo's own arithmetic takes.
fn leftovers(field: &[At]) -> Vec<Leftover> {
    field.iter().map(leftover).collect()
}

// ------------------------------------------------------------ the floors

/// What one reading of one azimuth repeats to, which is the floor every
/// column below is quoted against.
///
/// Two numbers, and the gap between them is the whole convergence question.
/// The first is two readings of one azimuth a frame apart: the correlator's
/// own noise on content that has not moved. The second is two readings of one
/// azimuth at consecutive moments, which is the same camera on different
/// content, and it is what an accumulation actually has to average down.
fn repeatability(rows: &[Row]) {
    let mut by_azimuth: BTreeMap<usize, Vec<(f64, f64)>> = BTreeMap::new();
    for row in rows {
        by_azimuth
            .entry(row.index)
            .or_default()
            .push((row.seconds, row.left));
    }
    let mut frame = Vec::new();
    let mut moment = Vec::new();
    for readings in by_azimuth.values() {
        for pair in readings.windows(2) {
            let apart = pair[1].0 - pair[0].0;
            let half = (pair[1].1 - pair[0].1) / std::f64::consts::SQRT_2;
            match apart < 0.5 {
                true => frame.push(half),
                false => moment.push(half),
            }
        }
    }
    let say = |name: &str, all: &[f64]| {
        println!(
            "{name:<10} {:>6} {:>10.4} {:>10.4}",
            all.len(),
            seam::rms(all.iter().copied()),
            mad(all),
        );
    };
    println!(
        "\nfloors: how far one azimuth's reading moves between two frames of one moment (33 ms) \n\
         \x20       and between two moments, as half a pair's difference in degrees. the rms and \n\
         \x20       the median absolute deviation are both here because these are heavy tailed \n\
         \x20       and an rms over that population is a statistic about the outliers."
    );
    println!("{:<10} {:>6} {:>10} {:>10}", "apart", "pairs", "rms", "mad");
    say("a frame", &frame);
    say("a moment", &moment);
}

/// How many scaled median deviations from the middle a reading may sit before
/// it is a correlation on the wrong feature, and the narrowest that tolerance
/// may become, in degrees. `seam`'s own `GATE_MADS` and `GATE_FLOOR_DEG`,
/// which are private to it; the same two numbers, one axis over.
const GATE_MADS: f64 = 4.0;
const GATE_FLOOR_DEG: f64 = 0.10;

/// What a normal population's standard deviation is, in median absolute
/// deviations of it. The constant every robust spread is quoted in.
const MAD_TO_SIGMA: f64 = 1.4826;

/// What a written table carries: the part of the field the five terms cannot
/// describe, or the five terms themselves.
#[derive(Clone, Copy, PartialEq)]
enum Form {
    Residual,
    Terms,
}

/// The middle value.
fn median(all: &[f64]) -> f64 {
    let mut all = all.to_vec();
    all.sort_by(f64::total_cmp);
    all.get(all.len() / 2).copied().unwrap_or(f64::NAN)
}

/// The median absolute deviation from the median: the spread of a heavy-tailed
/// population, which is what `seam::left`'s own gate is built on.
fn mad(all: &[f64]) -> f64 {
    let middle = |mut all: Vec<f64>| {
        all.sort_by(f64::total_cmp);
        all.get(all.len() / 2).copied().unwrap_or(f64::NAN)
    };
    let centre = middle(all.to_vec());
    middle(all.iter().map(|value| (value - centre).abs()).collect())
}

/// How much of the ring answers at all, and how often.
fn coverage(rows: &[Row]) {
    let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
    for row in rows {
        *counts.entry(row.index).or_default() += 1;
    }
    let mut all: Vec<usize> = counts.values().copied().collect();
    all.sort_unstable();
    let starved = all.iter().filter(|count| **count < 10).count();
    println!(
        "cover:  {} azimuths ever answered, {} of them fewer than 10 times. readings per azimuth: \n\
         \x20       least {}, quartile {}, median {}, most {}.",
        all.len(),
        starved,
        all.first().copied().unwrap_or(0),
        all.get(all.len() / 4).copied().unwrap_or(0),
        all.get(all.len() / 2).copied().unwrap_or(0),
        all.last().copied().unwrap_or(0),
    );
}

// ------------------------------------------------------------ probe 1

/// What a span of this length is worth, measured between spans that share no
/// frame: the field off one stretch against the field off the next.
///
/// **The column that decides.** Two accumulations of disjoint stretches are
/// two measurements of the same camera, so what separates them is the
/// sampling noise a span of that length leaves, and where it stops falling is
/// where more session buys nothing.
fn windows(rows: &[Row], options: &Options) {
    println!(
        "\nwindows: fields off DISJOINT stretches of this session, in degrees along the seam. \n\
         \x20       `apart` is the rms difference between consecutive stretches' fields at the \n\
         \x20       azimuths both reached. `noise` is how far apart two means of that many \n\
         \x20       readings HAVE to sit from the readings' own scatter, which is the floor a \n\
         \x20       longer span buys down. apart at the noise line is a field that is all \n\
         \x20       correlator; apart above it and flat is a field that is changing."
    );
    println!(
        "{:>8} {:>8} {:>8} {:>8} {:>10} {:>10} {:>10} {:>10}",
        "span s", "windows", "shared", "per azi", "apart rms", "noise", "own rms", "applied"
    );
    let last = rows.last().map_or(0.0, |row| row.seconds);
    for span in options.spans.iter().copied() {
        let fields: Vec<Vec<At>> = (0..)
            .map(|step| span * step as f64)
            .take_while(|from| from + span <= last + 1.0)
            .map(|from| field(rows, from, from + span, options.middle))
            .collect();
        let both: Vec<(At, At)> = fields
            .windows(2)
            .flat_map(|pair| shared(&pair[0], &pair[1]))
            .collect();
        if both.is_empty() {
            println!(
                "{span:>8.0} {:>8} {:>8} {:>8} {:>10} {:>10} {:>10} {:>10}",
                fields.len(),
                0,
                "-",
                "-",
                "-",
                "-",
                "-"
            );
            continue;
        }
        println!(
            "{span:>8.0} {:>8} {:>8} {:>8.1} {:>10.4} {:>10.4} {:>10.4} {:>10.4}",
            fields.len(),
            both.len(),
            both.iter().map(|(a, _)| a.readings as f64).sum::<f64>() / both.len() as f64,
            seam::rms(both.iter().map(|(a, b)| a.mean - b.mean)),
            seam::rms(
                both.iter()
                    .filter(|(a, b)| a.error.is_finite() && b.error.is_finite())
                    .map(|(a, b)| a.error.hypot(b.error))
            ),
            seam::rms(both.iter().map(|(a, _)| a.mean)),
            applied_apart(&fields),
        );
    }
    starved(rows, options);
}

/// How far apart the CORRECTION two consecutive windows would apply sits,
/// over the whole ring, in degrees.
///
/// The column a warm-up is read off. What reaches the picture is not the
/// per-azimuth field but the five terms fitted to it (`band::Along`), which is
/// a much smaller thing to settle, and it is defined at every direction
/// including the ones neither window reached.
fn applied_apart(fields: &[Vec<At>]) -> f64 {
    let ring: Vec<f64> = (0..kjerag_render::AZIMUTHS)
        .map(|index| index as f64 / kjerag_render::AZIMUTHS as f64 * std::f64::consts::TAU)
        .collect();
    let mut apart = Vec::new();
    for pair in fields.windows(2) {
        let (one, other) = (five(&leftovers(&pair[0])), five(&leftovers(&pair[1])));
        apart.extend(ring.iter().map(|phi| {
            let at = |terms: &[f64]| {
                basis(*phi)
                    .iter()
                    .zip(terms)
                    .map(|(term, coefficient)| term * coefficient)
                    .sum::<f64>()
            };
            at(&one) - at(&other)
        }));
    }
    seam::rms(apart.into_iter())
}

/// The same, cut by how much evidence an azimuth had.
///
/// A convergence that is really a coverage problem shows up here: if the
/// content-starved azimuths are the ones that disagree, more session at those
/// directions is the answer, and if they disagree no more than the rich ones,
/// it is not.
fn starved(rows: &[Row], options: &Options) {
    let span = options.spans.last().copied().unwrap_or(600.0).min(600.0);
    let last = rows.last().map_or(0.0, |row| row.seconds);
    let fields: Vec<Vec<At>> = (0..)
        .map(|step| span * step as f64)
        .take_while(|from| from + span <= last + 1.0)
        .map(|from| field(rows, from, from + span, options.middle))
        .collect();
    let both: Vec<(At, At)> = fields
        .windows(2)
        .flat_map(|pair| shared(&pair[0], &pair[1]))
        .collect();
    println!(
        "\nstarved: the same {span:.0} s windows, cut by how many readings the azimuth had in \n\
         \x20       the first of the pair."
    );
    println!(
        "{:>12} {:>8} {:>10} {:>10}",
        "readings", "shared", "apart rms", "noise"
    );
    for (name, low, high) in [
        ("1 to 4", 1, 4),
        ("5 to 15", 5, 15),
        ("16 to 50", 16, 50),
        ("over 50", 51, usize::MAX),
    ] {
        let cut: Vec<(At, At)> = both
            .iter()
            .copied()
            .filter(|(a, _)| a.readings >= low && a.readings <= high)
            .collect();
        if cut.is_empty() {
            continue;
        }
        println!(
            "{name:>12} {:>8} {:>10.4} {:>10.4}",
            cut.len(),
            seam::rms(cut.iter().map(|(a, b)| a.mean - b.mean)),
            seam::rms(
                cut.iter()
                    .filter(|(a, b)| a.error.is_finite() && b.error.is_finite())
                    .map(|(a, b)| a.error.hypot(b.error))
            ),
        );
    }
}

/// The same session accumulated the way a player would accumulate it: from
/// the first frame, growing.
///
/// Reported against the whole session's own field, which the growing prefix
/// converges on by construction. It is here because a warm-up is a prefix and
/// not a window, and the number a warm-up needs is how far a prefix still
/// sits from where it is going.
fn prefixes(rows: &[Row], options: &Options) {
    let last = rows.last().map_or(0.0, |row| row.seconds);
    let whole = field(rows, 0.0, last + 1.0, options.middle);
    println!(
        "\nprefix: the field off the first `span` seconds against the field off the whole \n\
         \x20       session ({} azimuths, {:.4} deg rms), in degrees.",
        whole.len(),
        seam::rms(whole.iter().map(|at| at.mean)),
    );
    println!(
        "{:>8} {:>10} {:>10} {:>12} {:>12}",
        "span s", "azimuths", "shared", "from final", "from before"
    );
    let mut before: Option<Vec<At>> = None;
    for span in options.spans.iter().copied() {
        let grown = field(rows, 0.0, span, options.middle);
        let both = shared(&grown, &whole);
        let step = before
            .as_ref()
            .map(|before| seam::rms(shared(&grown, before).iter().map(|(a, b)| a.mean - b.mean)));
        println!(
            "{span:>8.0} {:>10} {:>10} {:>12.4} {:>12}",
            grown.len(),
            both.len(),
            seam::rms(both.iter().map(|(a, b)| a.mean - b.mean)),
            step.map_or_else(|| "-".to_owned(), |step| format!("{step:.4}")),
        );
        before = Some(grown);
    }
}

// ------------------------------------------------------------ probe 2

/// One half of the session predicted by the other half, four ways.
///
/// The arms are the layers, in the order the pass applies them: the pose
/// alone, the pose and the five terms the band already fits per session, and
/// those plus a per-direction table. The fourth is stage 9's own arm, the
/// table with no five terms under it, so this instrument's numbers and
/// `--bin table`'s are the same statistic.
fn holdout(rows: &[Row], options: &Options) {
    let last = rows.last().map_or(0.0, |row| row.seconds);
    let split = options.split;
    println!(
        "\nheld out: one half of THIS session fitted, the other half predicted, in degrees rms \n\
         \x20       along the seam at a {:.0} deg kernel. a table earns its place only where the \n\
         \x20       `5+table` column beats the `5 terms` column.",
        options.smooth,
    );
    println!(
        "{:<22} {:>8} {:>10} {:>10} {:>10} {:>10}",
        "fitted on", "azimuths", "pose only", "5 terms", "5+table", "table only"
    );
    for (name, train, test) in [
        (
            format!("0-{:.0} s -> rest", split),
            (0.0, split),
            (split, last + 1.0),
        ),
        (
            format!("{:.0}-end s -> first", split),
            (split, last + 1.0),
            (0.0, split),
        ),
    ] {
        let train = field(rows, train.0, train.1, options.middle);
        let test = field(rows, test.0, test.1, options.middle);
        let arms = predict(&train, &test, options.smooth);
        println!(
            "{name:<22} {:>8} {:>10.4} {:>10.4} {:>10.4} {:>10.4}",
            test.len(),
            arms[0],
            arms[1],
            arms[2],
            arms[3],
        );
    }
    sweep(rows, options, split, last);
}

/// What the held-out half reads with each arm taken off it, in degrees rms.
fn predict(train: &[At], test: &[At], smooth: f32) -> [f64; 4] {
    let train = leftovers(train);
    let test = leftovers(test);
    let low = five(&train);
    let table = Table::of(&train, smooth);
    let at = |l: &Leftover| {
        let (sin, cos) = l.phi.sin_cos();
        (
            basis(f64::from(l.phi))
                .iter()
                .zip(&low)
                .map(|(term, coefficient)| term * coefficient)
                .sum::<f64>(),
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

/// The five terms the band fits per session, in degrees, off these leftovers.
///
/// Ordinary least squares over the same basis, which is `--bin table`'s own
/// order-2 arm: `band::Along::fit` solves the same system with a ridge of one
/// direction's evidence on it, and on a ring of fifty readings the two answers
/// differ by that ridge and by nothing else.
fn five(left: &[Leftover]) -> Vec<f64> {
    let rows: Vec<(Vec<f64>, f64)> = left
        .iter()
        .map(|l| (basis(f64::from(l.phi)), f64::from(l.perp.to_degrees())))
        .collect();
    seam::least_squares(&rows).map_or_else(|| vec![0.0; 5], |fit| fit.params)
}

/// The real Fourier basis up to two cycles: a constant, one cycle, two.
/// `band`'s own `terms`, written in the form `seam::least_squares` takes.
fn basis(phi: f64) -> Vec<f64> {
    vec![
        1.0,
        phi.cos(),
        phi.sin(),
        (2.0 * phi).cos(),
        (2.0 * phi).sin(),
    ]
}

/// The held-out arms at every kernel width, which is where a width would be
/// chosen if one were ever worth choosing.
fn sweep(rows: &[Row], options: &Options, split: f64, last: f64) {
    let first = field(rows, 0.0, split, options.middle);
    let second = field(rows, split, last + 1.0, options.middle);
    println!(
        "\nkernel: both directions of the same split, averaged, at every width. the row a table \n\
         \x20       has to beat is `5 terms`, which is what the pass already applies."
    );
    println!(
        "{:>8} {:>12} {:>12} {:>12}",
        "deg", "5 terms", "5+table", "table only"
    );
    for width in WIDTHS {
        let there = predict(&first, &second, width);
        let back = predict(&second, &first, width);
        let mean = |index: usize| (there[index] + back[index]) / 2.0;
        println!(
            "{width:>8.0} {:>12.4} {:>12.4} {:>12.4}",
            mean(1),
            mean(2),
            mean(3),
        );
    }
}

/// The other session, and whether either predicts the other.
///
/// Stage 9's own test, with one session's whole accumulated field on each
/// side. It is here because this instrument changes how a field is
/// accumulated, and a change that makes a field more reproducible WITHIN a
/// session has to be asked whether it made it reproducible BETWEEN them: that
/// is the question `--bin table` answered no to, on fields accumulated the
/// other way.
fn against(rows: &[Row], options: &Options) -> Fallible<()> {
    let Some(other) = &options.against else {
        return Ok(());
    };
    let mine = field_gated(rows, 0.0, f64::INFINITY, options.middle, options.gate);
    let theirs = field_gated(
        &load(other)?,
        0.0,
        f64::INFINITY,
        options.middle,
        options.gate,
    );
    let both = shared(&mine, &theirs);
    println!(
        "\nbetween: this session against {}, at the {} azimuths both reached. `apart` is how far \n\
         \x20       the two fields sit from each other and `own` how large each is; a field that \n\
         \x20       is a camera has apart under own.",
        other.display(),
        both.len(),
    );
    println!("{:>12} {:>10} {:>10}", "apart rms", "own rms", "theirs rms");
    println!(
        "{:>12.4} {:>10.4} {:>10.4}",
        seam::rms(both.iter().map(|(a, b)| a.mean - b.mean)),
        seam::rms(both.iter().map(|(a, _)| a.mean)),
        seam::rms(both.iter().map(|(_, b)| b.mean)),
    );
    let there = predict(&theirs, &mine, options.smooth);
    let back = predict(&mine, &theirs, options.smooth);
    println!(
        "{:<22} {:>10} {:>10} {:>10} {:>10}",
        "fitted on", "pose only", "5 terms", "5+table", "table only"
    );
    for (name, arms) in [("the other -> this", there), ("this -> the other", back)] {
        println!(
            "{name:<22} {:>10.4} {:>10.4} {:>10.4} {:>10.4}",
            arms[0], arms[1], arms[2], arms[3],
        );
    }
    Ok(())
}

// ------------------------------------------------------------ the table out

/// One window's table written down, so `--bin crossing table=` can read the
/// picture through it.
fn write_table(rows: &[Row], options: &Options) -> Fallible<()> {
    let (Some(out), Some(window)) = (&options.out, options.window) else {
        return Ok(());
    };
    let left = leftovers(&field(rows, window.0, window.1, options.middle));
    let table = match options.form {
        Form::Residual => Table::of(&left, options.smooth),
        Form::Terms => terms_table(&left),
    };
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, table.write())?;
    println!(
        "\nwrote:  {} off {} azimuths accumulated over {:.0} to {:.0} s, {:.4} deg rms, worst \n\
         \x20       entry {:.4} deg. source: kjerag-spike --bin converge, args: {}",
        out.display(),
        left.len(),
        window.0,
        window.1,
        seam::rms(
            table
                .entries()
                .iter()
                .map(|entry| f64::from(entry.to_degrees()))
        ),
        table
            .entries()
            .iter()
            .fold(0.0f32, |worst, entry| worst.max(entry.abs()))
            .to_degrees(),
        std::env::args().skip(1).collect::<Vec<_>>().join(" "),
    );
    Ok(())
}

/// The five terms this field asks for, written into all 128 directions.
///
/// Not a per-direction table at all: it is `band::Along`'s own field, carried
/// in `Table`'s vehicle so that `--bin crossing table=` can read a picture
/// through it. What it answers is what a STATIC five-term along-seam
/// correction does to a view, which is the layer this instrument found
/// reproduces and the table above is the layer that does not.
fn terms_table(left: &[Leftover]) -> Table {
    let low = five(left);
    Table::of_entries(std::array::from_fn(|index| {
        let phi = index as f64 / kjerag_render::AZIMUTHS as f64 * std::f64::consts::TAU;
        basis(phi)
            .iter()
            .zip(&low)
            .map(|(term, coefficient)| term * coefficient)
            .sum::<f64>()
            .to_radians() as f32
    }))
}

// ------------------------------------------------------------ the options

struct Options {
    input: PathBuf,
    /// Another session's dump, to ask whether either predicts the other.
    against: Option<PathBuf>,
    /// What a written table carries.
    form: Form,
    /// Whether the accumulated field passes the plausibility gate. On, except
    /// in the control that asks what the gate is doing.
    gate: bool,
    middle: Middle,
    /// A per-azimuth field of a known size and a known number of cycles, added
    /// to every reading before anything else touches it.
    plant: Option<(f64, f64)>,
    spans: Vec<f64>,
    split: f64,
    smooth: f32,
    window: Option<(f64, f64)>,
    out: Option<PathBuf>,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut input = None;
        let mut against = None;
        let mut gate = true;
        let mut form = Form::Residual;
        let mut middle = Middle::Mean;
        let mut plant = None;
        let mut spans = SPANS.to_vec();
        let mut split = 600.0;
        let mut smooth = band::SMOOTH_DEG;
        let mut window = None;
        let mut out = None;
        for arg in args {
            match arg.split_once('=') {
                Some(("spans", value)) => {
                    spans = value
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<_, _>>()
                        .map_err(|e| format!("spans are seconds: {e}"))?;
                }
                Some(("against", value)) => against = Some(PathBuf::from(value)),
                Some(("gate", value)) => gate = value != "0",
                Some(("form", "residual")) => form = Form::Residual,
                Some(("form", "terms")) => form = Form::Terms,
                Some(("form", _)) => return Err("form is residual or terms".into()),
                Some(("middle", "mean")) => middle = Middle::Mean,
                Some(("middle", "median")) => middle = Middle::Median,
                Some(("middle", "trimmed")) => middle = Middle::Trimmed,
                Some(("middle", _)) => return Err("middle is mean, median or trimmed".into()),
                Some(("plant", value)) => plant = Some(pair_of(value, ':')?),
                Some(("split", value)) => split = value.parse()?,
                Some(("smooth", value)) => smooth = value.parse()?,
                Some(("window", value)) => window = Some(pair_of(value, ':')?),
                Some(("out", value)) => out = Some(PathBuf::from(value)),
                Some(_) => return Err(format!("{USAGE}\n\nunknown: {arg}").into()),
                None => input = Some(PathBuf::from(arg)),
            }
        }
        Ok(Self {
            input: input.ok_or(USAGE)?,
            against,
            form,
            gate,
            middle,
            plant,
            spans,
            split,
            smooth,
            window,
            out,
        })
    }
}

fn pair_of(value: &str, between: char) -> Fallible<(f64, f64)> {
    let (one, other) = value
        .split_once(between)
        .ok_or("two numbers, separated by a colon")?;
    Ok((one.parse()?, other.parse()?))
}

const USAGE: &str = "usage: converge <settle-dump.csv> [spans=30,60,...] [split=seconds] \
[against=other.csv] [gate=0] [smooth=deg] [middle=mean|median|trimmed] [plant=deg:cycles] [window=from:to form=residual|terms out=table.txt]";
