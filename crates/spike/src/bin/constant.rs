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
//!
//! # The second question: the cycles, not the constant
//!
//! The constant answered its question and the answer was per session. The
//! sinusoid sitting on top of it was printed as an amplitude from the first
//! run and never as a **phase**, so nothing yet says whether the six flights'
//! one-cycle terms point the same way. That is a different design question
//! from the constant's, and the two answers do not follow from each other:
//!
//! - **Same vector on every flight** - a one-cycle term is what a *pose* puts
//!   on this axis (a principal-point shift is one cycle round the azimuth;
//!   seam-two-axis 1), so a term that is one vector on all six flights is
//!   pose shaped, and a *pooled* correction can carry it at no per-session
//!   cost. The pool is the only shape that fits the owner's clock
//!   (stage9 13.11).
//! - **A different vector each flight** - then the sinusoid is per session
//!   like the DC is, and nothing pooled can carry it either.
//!
//! An amplitude alone cannot tell those apart: six flights all reading 0.3 deg
//! at six different phases average to nothing, and reading only the column of
//! amplitudes they would look identical to six flights that agree.
//!
//! **A phase is not a reading until the amplitude is resolved.** Where the
//! amplitude is at or under its own error bar the phase it came from is a
//! random angle, and this instrument says so rather than printing it as a
//! number. The verdict refuses outright where too few flights resolve.
//!
//! **The control has to recover a phase and not only a size.** `plant=` puts
//! a constant on the across column and the first run showed it read back to
//! four digits, which says nothing about the angular half of a vector.
//! `wave=<deg>@<phase deg>` plants a **sinusoid** on the same column and the
//! run prints what it read back in both, taken as the vector difference
//! against the same flight fitted without it, so the recovery is measured on
//! the real footage rather than on a synthetic ring:
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin constant -- <file.insv> seam=pool wave=0.40@110
//! ```
//!
//! **Two models are printed and they are different models.** The three-term
//! fit (`DC + one cycle`) is the one the published table was read off and its
//! numbers are unchanged here. The five-term fit adds two cycles, and adding a
//! term moves every other term, so its DC and its one cycle are **not** the
//! published numbers and are labelled as its own.
//!
//! **What a wandering one-cycle term does NOT distinguish**, exactly as with
//! the constant: a flight's own near content and a per-session geometry error
//! both move it, and nothing here separates them.

use std::path::{Path, PathBuf};

use kjerag_media::Fallible;
use kjerag_meta::CalibrationSet;
use kjerag_render::{SeamFit, Size, seam};
use kjerag_spike::fit_arg;

const USAGE: &str = "usage: constant <file.insv> [<file.insv> ...] [seam=pool|<five knobs>] \
                     [places=] [frames=] [patches=] [plant=<deg>] \
                     [wave=<deg>@<phase deg>] [spin=<deg per capture>]";

/// How many source pixels one degree is at the seam, for the second column of
/// every reading: the same conversion `--bin ceiling` quotes its across-seam
/// numbers in, so the two instruments' numbers are the same statistic.
const SRC_PX_PER_DEG: f64 = 16.33;

/// How far above its own error bar an amplitude has to sit before the phase it
/// came from is printed as a number rather than as a refusal.
///
/// Two sigma, and the reason it is not one: the phase of a vector whose length
/// is one sigma is uniform enough over a quadrant that a table of them would
/// read as a finding.
const RESOLVED: f64 = 2.0;

/// How many flights have to resolve a one-cycle amplitude before the
/// cross-flight verdict is allowed to be anything but undecidable.
///
/// Three, because two agreeing vectors is a line through two points.
const RESOLVED_NEEDED: usize = 3;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let fit = options.fit()?;
    println!(
        "pose:   roll:{:.3},yaw:{:.3},pitch:{:.3},cx:{:.2},cy:{:.2}",
        fit.roll_deg, fit.yaw_deg, fit.pitch_deg, fit.cx_px, fit.cy_px,
    );
    println!(
        "plan:   {} places x {} frames, {} azimuths{}{}",
        options.places,
        options.frames,
        options.patches,
        match options.plant {
            0.0 => String::new(),
            deg => format!(", ACROSS COLUMN PLANTED with {deg:+.3} deg"),
        },
        match options.wave.planted() {
            false => String::new(),
            true => format!(
                ", ACROSS COLUMN PLANTED with a {:.3} deg sinusoid at {:.1} deg",
                options.wave.amplitude_deg, options.wave.phase_deg,
            ),
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
    for (capture, path) in options.inputs.iter().enumerate() {
        let row = measure(path, capture, &fit, &options)?;
        say(&row);
        rows.push(row);
    }
    verdict(&rows);
    phases(&rows, 1, 3);
    recovery(&rows, options.wave);
    cycle_verdict(&rows, 1);
    two_cycle(&rows);
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
    /// standard error. Three terms: the model the published table was read off.
    across: Harmonic,
    along: Harmonic,
    /// The same two axes under the five-term model, which is a different model
    /// and whose constant and one cycle are therefore different numbers.
    /// `None` where the arc will not pin five terms.
    across_two: Option<Harmonic>,
    along_two: Option<Harmonic>,
    /// What a planted sinusoid read back as: this flight's one-cycle vector
    /// with the plant, minus the same flight's without it. The control, and it
    /// is on the real footage rather than on a synthetic ring.
    recovered: Option<Cycle>,
}

/// A constant, the cycles above it, and how well each is pinned.
#[derive(Clone)]
struct Harmonic {
    constant: f64,
    error: f64,
    /// One entry per order fitted, one cycle first.
    cycles: Vec<Cycle>,
    rms: f64,
}

impl Harmonic {
    fn cycle(&self, order: usize) -> &Cycle {
        &self.cycles[order - 1]
    }
}

/// One harmonic order, kept as the vector it was fitted as.
///
/// The amplitude, the phase and their error bars are all views of these four
/// numbers, and the cross-flight test needs the vector and its covariance
/// rather than any of the views, so the vector is what is stored.
#[derive(Clone, Copy)]
struct Cycle {
    order: usize,
    /// The cosine and sine coefficients, in degrees.
    vector: [f64; 2],
    /// Their covariance, in degrees squared.
    covariance: [[f64; 2]; 2],
}

impl Cycle {
    fn amplitude(&self) -> f64 {
        self.vector[0].hypot(self.vector[1])
    }

    /// The amplitude's own error bar, by propagating the vector's covariance
    /// through `hypot`.
    fn amplitude_error(&self) -> f64 {
        let [a, b] = self.vector;
        let size = self.amplitude();
        if size == 0.0 {
            // At the origin the amplitude has no gradient and the worst
            // direction is the honest answer.
            return self.covariance[0][0].max(self.covariance[1][1]).sqrt();
        }
        let variance = a * a * self.covariance[0][0]
            + b * b * self.covariance[1][1]
            + 2.0 * a * b * self.covariance[0][1];
        (variance.max(0.0) / (size * size)).sqrt()
    }

    /// Where this order's first maximum sits, in degrees of seam azimuth: the
    /// term is `amplitude * cos(order * (phi - phase))`, so the phase of a
    /// two-cycle term lives in the half circle and repeats.
    fn phase_deg(&self) -> f64 {
        let angle = self.vector[1].atan2(self.vector[0]).to_degrees() / self.order as f64;
        let period = 360.0 / self.order as f64;
        angle.rem_euclid(period)
    }

    /// The phase's own error bar, in degrees of seam azimuth.
    ///
    /// Meaningless where the amplitude is not resolved, which is what
    /// [`Cycle::pinned`] is for: a vector at the origin points nowhere and no
    /// error bar on the angle says that as plainly as refusing to print one.
    fn phase_error_deg(&self) -> f64 {
        let [a, b] = self.vector;
        let size = self.amplitude();
        if size == 0.0 {
            return f64::INFINITY;
        }
        let variance = b * b * self.covariance[0][0] + a * a * self.covariance[1][1]
            - 2.0 * a * b * self.covariance[0][1];
        (variance.max(0.0) / size.powi(4)).sqrt().to_degrees() / self.order as f64
    }

    /// Whether the amplitude stands far enough above its own bar for the phase
    /// beside it to be a reading.
    fn pinned(&self) -> bool {
        self.amplitude() > RESOLVED * self.amplitude_error()
    }

    /// This cycle minus another, vector by vector: what a planted sinusoid read
    /// back as, and how far one flight sits from the pool.
    ///
    /// The covariances add, which is right for the plant (the two fits differ
    /// only by the plant, so the difference carries both fits' uncertainty) and
    /// conservative for the pool comparison.
    fn minus(&self, other: &Cycle) -> Cycle {
        Cycle {
            order: self.order,
            vector: [
                self.vector[0] - other.vector[0],
                self.vector[1] - other.vector[1],
            ],
            covariance: std::array::from_fn(|i| {
                std::array::from_fn(|j| self.covariance[i][j] + other.covariance[i][j])
            }),
        }
    }
}

fn measure(path: &Path, capture: usize, fit: &SeamFit, options: &Options) -> Fallible<Row> {
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
    // Both plants go on the across column only, so the along column stays a
    // control while a plant is being read back.
    let bare: Vec<f64> = left.iter().map(|(_, axes)| axes[1]).collect();
    let across: Vec<f64> = phis
        .iter()
        .zip(&bare)
        .map(|(phi, value)| value + options.plant + options.wave.at(capture, *phi))
        .collect();
    let along: Vec<f64> = left.iter().map(|(_, axes)| axes[0]).collect();
    let fitted =
        harmonic::<3>(&phis, &across).ok_or("the across readings do not pin a constant")?;
    let recovered = options.wave.planted().then(|| {
        let bare = harmonic::<3>(&phis, &bare).map(|read| *read.cycle(1));
        bare.map(|bare| fitted.cycle(1).minus(&bare))
    });
    Ok(Row {
        name: name(path),
        sites: left.len(),
        arc_deg,
        gap_deg,
        across: fitted,
        along: harmonic::<3>(&phis, &along).ok_or("the along readings do not pin a constant")?,
        across_two: harmonic::<5>(&phis, &across),
        along_two: harmonic::<5>(&phis, &along),
        recovered: recovered.flatten(),
    })
}

fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

// ------------------------------------------------------------ the fit

/// `1, cos phi, sin phi, cos 2phi, sin 2phi, ...` to `TERMS` of them.
///
/// `TERMS` is odd by construction: a constant and then a cosine and a sine per
/// order.
fn basis<const TERMS: usize>(phi: f64) -> [f64; TERMS] {
    std::array::from_fn(|index| match index {
        0 => 1.0,
        _ => {
            let angle = index.div_ceil(2) as f64 * phi;
            match index % 2 {
                1 => angle.cos(),
                _ => angle.sin(),
            }
        }
    })
}

/// `a0 + sum over orders of (a cos k.phi + b sin k.phi)`, and how well each of
/// them is pinned by the azimuths that carried a reading.
///
/// **`TERMS = 3` is the model the published table was read off** and adding
/// orders changes every coefficient, not only the new ones, so the two fits
/// are reported as two models rather than as one table with extra columns.
///
/// Why the one cycle is in the model at all rather than left in the residual:
/// two named calibration errors put a one-cycle term on this axis and nothing
/// a pose can do puts a constant there (`--bin ceiling`), so the one cycle is
/// here to be taken *out* - a constant read without it would collect whatever
/// a tilted pose left round the ring and call it DC.
fn harmonic<const TERMS: usize>(phis: &[f64], values: &[f64]) -> Option<Harmonic> {
    let mut normal = [[0.0f64; TERMS]; TERMS];
    let mut right = [0.0f64; TERMS];
    for (phi, value) in phis.iter().zip(values) {
        let row = basis::<TERMS>(*phi);
        for i in 0..TERMS {
            right[i] += row[i] * value;
            for j in 0..TERMS {
                normal[i][j] += row[i] * row[j];
            }
        }
    }
    let inverse = invert(normal)?;
    let params: [f64; TERMS] =
        std::array::from_fn(|i| (0..TERMS).map(|j| inverse[i][j] * right[j]).sum::<f64>());
    let residual = |index: usize| {
        let row = basis::<TERMS>(phis[index]);
        values[index] - (0..TERMS).map(|i| row[i] * params[i]).sum::<f64>()
    };
    let count = phis.len();
    if count <= TERMS {
        return None;
    }
    let variance = (0..count).map(|i| residual(i).powi(2)).sum::<f64>() / (count - TERMS) as f64;
    let scatter = variance.sqrt();
    Some(Harmonic {
        constant: params[0],
        // The real one: the residual scatter through the fit's own geometry,
        // so an arc with a hole in it reports a wider bar than a whole ring
        // of the same scatter would.
        error: scatter * inverse[0][0].max(0.0).sqrt(),
        cycles: (1..=TERMS / 2)
            .map(|order| {
                let (cos, sin) = (2 * order - 1, 2 * order);
                Cycle {
                    order,
                    vector: [params[cos], params[sin]],
                    // The same geometry the constant's bar comes through, kept
                    // as the 2x2 block so a phase and a pooling test can be
                    // taken off it rather than off the diagonal alone.
                    covariance: [
                        [variance * inverse[cos][cos], variance * inverse[cos][sin]],
                        [variance * inverse[sin][cos], variance * inverse[sin][sin]],
                    ],
                }
            })
            .collect(),
        rms: scatter,
    })
}

/// How near singular a normal matrix may be before the fit is refused, as a
/// fraction of its own largest entry.
///
/// Scale free, because the normal matrix grows with the site count and an
/// absolute floor would refuse a big ring and accept a small one. The value is
/// far above rounding and far below any real arc: the thinnest arc in the
/// corpus is 195 degrees, which pivots at about `1e-3` of the scale on the
/// five-term fit, and sites all at one azimuth pivot at rounding.
const RANK_FLOOR: f64 = 1e-10;

/// A square inverse by Gauss-Jordan with partial pivoting. `None` where the
/// azimuths do not span enough of the circle to tell the terms apart, which is
/// a refusal and not a number with a big error bar.
fn invert<const N: usize>(m: [[f64; N]; N]) -> Option<[[f64; N]; N]> {
    let scale = m
        .iter()
        .flatten()
        .fold(0.0f64, |worst, entry| worst.max(entry.abs()));
    let mut left = m;
    let mut right: [[f64; N]; N] =
        std::array::from_fn(|i| std::array::from_fn(|j| f64::from(u8::from(i == j))));
    for column in 0..N {
        let pivot =
            (column..N).max_by(|a, b| left[*a][column].abs().total_cmp(&left[*b][column].abs()))?;
        if left[pivot][column].abs() <= RANK_FLOOR * scale {
            return None;
        }
        left.swap(column, pivot);
        right.swap(column, pivot);
        let divisor = left[column][column];
        for j in 0..N {
            left[column][j] /= divisor;
            right[column][j] /= divisor;
        }
        for row in 0..N {
            if row == column {
                continue;
            }
            let factor = left[row][column];
            for j in 0..N {
                left[row][j] -= factor * left[column][j];
                right[row][j] -= factor * right[column][j];
            }
        }
    }
    Some(right)
}

// ------------------------------------------------------ across the flights

/// How far the same term wanders from flight to flight, and how much of it a
/// single pooled vector would still carry.
struct Wander {
    /// The inverse-variance-weighted mean vector and its own error bars.
    pooled: Cycle,
    /// The chi-square of the flights against that one vector, over its own
    /// degrees of freedom: about one where the flights agree inside their bars.
    chi_per_dof: f64,
    /// The largest distance from a flight's vector to the pooled one, in
    /// degrees. This is the design number: what a pooled correction would
    /// leave on its worst flight.
    worst_leftover: f64,
    /// The name of that flight.
    worst_at: String,
    /// Per flight: the name, the term it carries, and what a pooled correction
    /// would leave of it. A verdict is a word and this is the ledger under it.
    leftovers: Vec<(String, f64, f64)>,
    /// How many flights resolved an amplitude above [`RESOLVED`] sigma.
    resolved: usize,
}

/// The pooled vector, the chi-square against it, and the worst flight's
/// distance from it. `None` where a flight's covariance will not invert, which
/// on this corpus means its arc did not pin the term at all.
fn wander(rows: &[Row], pick: fn(&Row) -> &Harmonic, order: usize) -> Option<Wander> {
    let cycles: Vec<(&str, Cycle)> = rows
        .iter()
        .map(|row| (row.name.as_str(), *pick(row).cycle(order)))
        .collect();
    let mut weight = [[0.0f64; 2]; 2];
    let mut weighted = [0.0f64; 2];
    for (_, cycle) in &cycles {
        let inverse = invert(cycle.covariance)?;
        for i in 0..2 {
            for j in 0..2 {
                weight[i][j] += inverse[i][j];
                weighted[i] += inverse[i][j] * cycle.vector[j];
            }
        }
    }
    let spread = invert(weight)?;
    let mean: [f64; 2] =
        std::array::from_fn(|i| (0..2).map(|j| spread[i][j] * weighted[j]).sum::<f64>());
    let pooled = Cycle {
        order,
        vector: mean,
        covariance: spread,
    };
    let mut chi = 0.0;
    let mut worst_leftover = 0.0;
    let mut worst_at = String::new();
    let mut leftovers = Vec::new();
    for &(name, cycle) in &cycles {
        let offset = [cycle.vector[0] - mean[0], cycle.vector[1] - mean[1]];
        let inverse = invert(cycle.covariance)?;
        chi += (0..2)
            .map(|i| {
                (0..2)
                    .map(|j| offset[i] * inverse[i][j] * offset[j])
                    .sum::<f64>()
            })
            .sum::<f64>();
        let leftover = offset[0].hypot(offset[1]);
        leftovers.push((name.to_owned(), cycle.amplitude(), leftover));
        if leftover > worst_leftover {
            worst_leftover = leftover;
            name.clone_into(&mut worst_at);
        }
    }
    Some(Wander {
        pooled,
        // Two per flight, less the two the pooled vector itself spent.
        chi_per_dof: chi / (2 * (cycles.len() - 1)) as f64,
        worst_leftover,
        worst_at,
        leftovers,
        resolved: cycles.iter().filter(|(_, c)| c.pinned()).count(),
    })
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
        row.across.cycle(1).amplitude(),
        row.across.rms,
        row.along.constant,
        row.along.error,
        row.along.cycle(1).amplitude(),
        row.along.rms,
    );
}

/// An amplitude and a phase with their bars, or the refusal that stands where
/// the amplitude did not clear [`RESOLVED`] sigma and the phase is therefore a
/// random angle.
fn vector(cycle: &Cycle) -> String {
    let size = format!(
        "{:>7.4} +-{:.4}",
        cycle.amplitude(),
        cycle.amplitude_error()
    );
    match cycle.pinned() {
        true => format!(
            "{size} {:>7.1} +-{:>5.1}",
            cycle.phase_deg(),
            cycle.phase_error_deg(),
        ),
        // A phase belongs to a vector with a length. Printing one here would
        // put a number where the measurement has none.
        false => format!("{size} {:>15}", "not pinned"),
    }
}

/// The second table: where each flight's one-cycle term points, beside how big
/// it is, on both axes.
fn phases(rows: &[Row], order: usize, terms: usize) {
    println!(
        "\nthe {} term under the {terms}-term model: amplitude and where its maximum sits",
        match order {
            1 => "one-cycle",
            _ => "two-cycle",
        },
    );
    println!(
        "{:<34}  {:>28}  {:>28}",
        "capture", "across: deg, phase deg", "along: deg, phase deg",
    );
    for row in rows {
        println!(
            "{:<34}  {}  {}",
            row.name,
            vector(row.across.cycle(order)),
            vector(row.along.cycle(order)),
        );
    }
}

/// The five-term model's own table, kept apart from the three-term one because
/// it is a different model and its constant is a different number.
fn two_cycle(rows: &[Row]) {
    let complete: Vec<Row> = rows
        .iter()
        .filter_map(|row| {
            Some(Row {
                name: row.name.clone(),
                sites: row.sites,
                arc_deg: row.arc_deg,
                gap_deg: row.gap_deg,
                across: row.across_two.clone()?,
                along: row.along_two.clone()?,
                across_two: None,
                along_two: None,
                recovered: None,
            })
        })
        .collect();
    if complete.len() < rows.len() {
        println!(
            "\n{} of {} captures do not pin five terms and are left out of the two-cycle model",
            rows.len() - complete.len(),
            rows.len(),
        );
    }
    if complete.is_empty() {
        return;
    }
    println!(
        "\nTHE FIVE-TERM MODEL. Adding a term moves every other term, so this model's constant \
         and\nits one cycle are NOT the three-term numbers above and must not be quoted as them."
    );
    for order in [1, 2] {
        phases(&complete, order, 5);
    }
    for order in [1, 2] {
        cycle_verdict(&complete, order);
    }
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

/// Whether one vector fits all the flights' cycles at this order, on the axis
/// that matters and on the control axis beside it.
fn cycle_verdict(rows: &[Row], order: usize) {
    if rows.len() < 2 {
        return;
    }
    let (Some(across), Some(along)) = (
        wander(rows, |row| &row.across, order),
        wander(rows, |row| &row.along, order),
    ) else {
        println!("\nthe {order}-cycle vectors do not pin a pool: a flight's own bars are singular");
        return;
    };
    println!("\nis the {order}-cycle term one vector on every flight?");
    for (axis, read) in [("across the seam", &across), ("along the seam  ", &along)] {
        println!(
            "  {axis}  pooled {:.4} deg at {:>5.1} deg, chi2/dof {:>7.2}, worst flight {:.4} deg \
             ({:.2} src px) away: {}",
            read.pooled.amplitude(),
            read.pooled.phase_deg(),
            read.chi_per_dof,
            read.worst_leftover,
            read.worst_leftover * SRC_PX_PER_DEG,
            read.worst_at,
        );
    }
    println!(
        "  {} of {} flights resolve an across-seam amplitude above {RESOLVED} of its own bar",
        across.resolved,
        rows.len(),
    );
    println!(
        "\n  what a pooled correction would actually do, across the seam, flight by flight:\n  \
         {:<32} {:>10} {:>12} {:>10}",
        "capture", "carries", "pool leaves", "bought",
    );
    for (name, carries, leftover) in &across.leftovers {
        println!(
            "  {name:<32} {carries:>10.4} {leftover:>12.4} {:>+10.4}",
            leftover - carries,
        );
    }
    println!("\n{}", pooling(&across, &along, rows.len()));
}

/// The three answers a cross-flight cycle reading can give, and the gate that
/// stands in front of all of them.
fn pooling(across: &Wander, along: &Wander, flights: usize) -> String {
    if across.resolved < RESOLVED_NEEDED {
        return format!(
            "UNDECIDABLE: only {} of {flights} flights resolve this amplitude above {RESOLVED} of \
             its own error bar, so most of the phases above are random angles and nothing about \
             pooling can be read off them. What is needed is more sites per flight, not a \
             different reduction.",
            across.resolved,
        );
    }
    if across.chi_per_dof <= 1.0 {
        return format!(
            "POOLABLE: the flights' vectors agree with one pooled vector to chi2/dof {:.2}, which \
             is what agreement inside their own error bars looks like. The term is the same on \
             every flight, so it is pose shaped and a pooled correction can carry it. A pool would \
             leave {:.4} deg ({:.2} src px) on its worst flight.",
            across.chi_per_dof,
            across.worst_leftover,
            across.worst_leftover * SRC_PX_PER_DEG,
        );
    }
    if across.chi_per_dof <= along.chi_per_dof {
        return format!(
            "POOLABLE: the across-seam vectors disagree (chi2/dof {:.2}), but no more than the \
             axis no scene can reach does ({:.2}), so what is wandering is this instrument and \
             not the seam. A pooled correction can still carry the term, and it would leave \
             {:.4} deg ({:.2} src px) on its worst flight.",
            across.chi_per_dof,
            along.chi_per_dof,
            across.worst_leftover,
            across.worst_leftover * SRC_PX_PER_DEG,
        );
    }
    format!(
        "PER SESSION: the across-seam vectors disagree with any one pooled vector by chi2/dof \
         {:.2}, further than the axis no scene can reach does ({:.2}), so the disagreement is not \
         this instrument's. A pooled correction would leave {:.4} deg ({:.2} src px) on its worst \
         flight, against a pooled amplitude of {:.4}. Which per-session thing it is - the \
         flights' own near content or their own geometry - this does not say and must not be read \
         as saying.",
        across.chi_per_dof,
        along.chi_per_dof,
        across.worst_leftover,
        across.worst_leftover * SRC_PX_PER_DEG,
        across.pooled.amplitude(),
    )
}

/// What the planted sinusoid read back as, per flight: the control that has to
/// pass in BOTH amplitude and phase before any sentence above counts.
fn recovery(rows: &[Row], wave: Wave) {
    if !wave.planted() {
        return;
    }
    println!(
        "\nTHE CONTROL. A {:.4} deg sinusoid was planted on the across column of every capture, \
         its\nmaximum at {:.1} deg{}. What came back, as the vector difference \
         against the same\ncapture fitted without it:",
        wave.amplitude_deg,
        wave.phase_deg,
        match wave.spin_deg {
            0.0 => " on all of them".to_owned(),
            spin => format!(" on the first and turning {spin:+.1} deg per capture"),
        },
    );
    println!(
        "{:<34}  {:>12}  {:>12}  {:>10}  {:>10}",
        "capture", "read deg", "read phase", "size err", "phase err",
    );
    for (capture, row) in rows.iter().enumerate() {
        let Some(read) = &row.recovered else {
            println!("{:<34}  the unplanted fit did not pin a cycle", row.name);
            continue;
        };
        let asked = wave.cycle(capture).expect("a plant was asked for");
        println!(
            "{:<34}  {:>12.4}  {:>12.1}  {:>+10.4}  {:>+10.1}",
            row.name,
            read.amplitude(),
            read.phase_deg(),
            read.amplitude() - asked.amplitude(),
            (read.phase_deg() - asked.phase_deg() + 180.0).rem_euclid(360.0) - 180.0,
        );
    }
}

// ------------------------------------------------------------ the line

struct Options {
    inputs: Vec<PathBuf>,
    seam: String,
    places: usize,
    frames: usize,
    patches: usize,
    plant: f64,
    wave: Wave,
}

/// A known sinusoid to add to the across column, so the run can be asked to
/// find one that is there.
///
/// With `spin` at zero the same wave goes on every capture, which is the
/// recovery control: it must come back in amplitude and in phase. With `spin`
/// set, each successive capture's wave is turned by that much, which plants a
/// **per-session wander of a known size** on a corpus - and that is the
/// control the cross-flight verdict actually needs, because the verdict's
/// risk is a false negative. Sweeping the amplitude down until the verdict
/// stops flipping to PER SESSION measures how big a wander this instrument
/// can find on this footage, and a wander this instrument cannot find is one
/// the verdict was never entitled to rule out.
#[derive(Clone, Copy, Default)]
struct Wave {
    amplitude_deg: f64,
    phase_deg: f64,
    spin_deg: f64,
}

impl Wave {
    /// `<deg>@<phase deg>`, the phase being where the maximum sits.
    fn parse(text: &str) -> Fallible<Self> {
        let (amplitude, phase) = text
            .split_once('@')
            .ok_or("wave takes <deg>@<phase deg>, as in wave=0.40@110")?;
        Ok(Self {
            amplitude_deg: amplitude.parse()?,
            phase_deg: phase.parse()?,
            spin_deg: 0.0,
        })
    }

    fn planted(&self) -> bool {
        self.amplitude_deg != 0.0
    }

    /// Where this capture's wave points, the corpus's order being the order
    /// the captures were named on the command line.
    fn phase_at(&self, capture: usize) -> f64 {
        self.phase_deg + self.spin_deg * capture as f64
    }

    fn at(&self, capture: usize, phi: f64) -> f64 {
        self.amplitude_deg * (phi - self.phase_at(capture).to_radians()).cos()
    }

    /// The plant as the vector the fit is asked to return, so what was asked
    /// and what came back are the same kind of thing.
    fn cycle(&self, capture: usize) -> Option<Cycle> {
        let phase = self.phase_at(capture).to_radians();
        self.planted().then(|| Cycle {
            order: 1,
            vector: [
                self.amplitude_deg * phase.cos(),
                self.amplitude_deg * phase.sin(),
            ],
            covariance: [[0.0; 2]; 2],
        })
    }
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
            wave: Wave::default(),
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
                "wave" => options.wave = Wave::parse(value)?,
                "spin" => options.wave.spin_deg = value.parse()?,
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

    /// The same, over whatever stretch of the circle is asked for, with a
    /// sinusoid of a known amplitude AND a known phase on it.
    fn arc(count: usize, span_deg: f64, amplitude: f64, phase_deg: f64) -> (Vec<f64>, Vec<f64>) {
        let phis: Vec<f64> = (0..count)
            .map(|i| (i as f64 / count as f64 * span_deg).to_radians())
            .collect();
        let values = phis
            .iter()
            .map(|phi| amplitude * (phi - phase_deg.to_radians()).cos())
            .collect();
        (phis, values)
    }

    /// The plant: a constant that is there is read back at its own size, and
    /// the one-cycle term beside it does not leak into it.
    #[test]
    fn a_planted_constant_reads_back_at_its_own_size() {
        let (phis, values) = ring(72, -0.49, 0.30);
        let read = harmonic::<3>(&phis, &values).expect("a whole ring pins a constant");
        assert!((read.constant + 0.49).abs() < 1e-9, "{}", read.constant);
        let size = read.cycle(1).amplitude();
        assert!((size - 0.30).abs() < 1e-9, "{size}");
        assert!(read.rms < 1e-9, "{}", read.rms);
    }

    /// The null: a one-cycle term with no constant under it reads no
    /// constant, so the column cannot manufacture one out of a tilted pose.
    #[test]
    fn a_pure_cycle_reads_no_constant() {
        let (phis, values) = ring(72, 0.0, 0.42);
        let read = harmonic::<3>(&phis, &values).expect("a whole ring pins a constant");
        assert!(read.constant.abs() < 1e-9, "{}", read.constant);
    }

    /// THE CONTROL THIS EXTENSION EXISTS FOR: a planted sinusoid comes back in
    /// amplitude AND in phase. A fit that recovered the size and not the angle
    /// would pass every test above this one and answer the cross-flight
    /// question wrongly, because six flights at six phases have the same
    /// column of amplitudes as six flights that agree.
    #[test]
    fn a_planted_sinusoid_reads_back_in_amplitude_and_in_phase() {
        for phase_deg in [0.0, 37.5, 110.0, 184.0, 300.0] {
            let (phis, values) = arc(72, 360.0, 0.40, phase_deg);
            let read = harmonic::<3>(&phis, &values).expect("a whole ring pins a cycle");
            let cycle = read.cycle(1);
            assert!(
                (cycle.amplitude() - 0.40).abs() < 1e-9,
                "{}",
                cycle.amplitude()
            );
            assert!(
                (cycle.phase_deg() - phase_deg).abs() < 1e-6,
                "asked {phase_deg}, read {}",
                cycle.phase_deg(),
            );
        }
    }

    /// The same on an arc rather than a ring, because no flight in the corpus
    /// carries a whole circle: the thinnest is 195 degrees.
    #[test]
    fn a_planted_sinusoid_reads_back_on_the_thinnest_arc_in_the_corpus() {
        let (phis, values) = arc(20, 195.0, 0.40, 110.0);
        let read = harmonic::<3>(&phis, &values).expect("195 degrees pins a cycle");
        let cycle = read.cycle(1);
        assert!(
            (cycle.amplitude() - 0.40).abs() < 1e-6,
            "{}",
            cycle.amplitude()
        );
        assert!(
            (cycle.phase_deg() - 110.0).abs() < 1e-6,
            "{}",
            cycle.phase_deg()
        );
    }

    /// A two-cycle term does not leak into the one-cycle phase under the model
    /// that has a place for it, so the five-term table reads its own orders.
    #[test]
    fn the_two_cycle_term_stays_out_of_the_one_cycle_vector() {
        let phis: Vec<f64> = (0..72)
            .map(|i| i as f64 / 72.0 * std::f64::consts::TAU)
            .collect();
        let values: Vec<f64> = phis
            .iter()
            .map(|phi| 0.40 * (phi - 110f64.to_radians()).cos() + 0.25 * (2.0 * (phi - 0.7)).cos())
            .collect();
        let read = harmonic::<5>(&phis, &values).expect("a whole ring pins five terms");
        assert!((read.cycle(1).amplitude() - 0.40).abs() < 1e-9);
        assert!((read.cycle(1).phase_deg() - 110.0).abs() < 1e-6);
        assert!((read.cycle(2).amplitude() - 0.25).abs() < 1e-9);
        assert!(
            (read.cycle(2).phase_deg() - 0.7f64.to_degrees()).abs() < 1e-6,
            "{}",
            read.cycle(2).phase_deg(),
        );
    }

    /// A phase is refused where the amplitude did not clear its own bar. The
    /// arm that says the gate can also pass is the line under it: the same
    /// scatter with a real term on top is pinned.
    #[test]
    fn a_phase_is_not_pinned_where_the_amplitude_is_noise() {
        let phis: Vec<f64> = (0..72)
            .map(|i| i as f64 / 72.0 * std::f64::consts::TAU)
            .collect();
        let dither: Vec<f64> = (0..72).map(|i| f64::from(i % 2) * 0.2 - 0.1).collect();
        let noise = harmonic::<3>(&phis, &dither).expect("a whole ring pins a fit");
        assert!(!noise.cycle(1).pinned(), "{}", noise.cycle(1).amplitude());
        let signal: Vec<f64> = phis
            .iter()
            .zip(&dither)
            .map(|(phi, d)| d + 0.40 * (phi - 110f64.to_radians()).cos())
            .collect();
        let read = harmonic::<3>(&phis, &signal).expect("a whole ring pins a fit");
        assert!(read.cycle(1).pinned(), "{}", read.cycle(1).amplitude());
        assert!((read.cycle(1).phase_deg() - 110.0).abs() < 2.0);
    }

    /// A thin arc pins a phase worse than a whole ring does, at the same
    /// scatter: the bar carries the geometry and not only the noise.
    #[test]
    fn a_thin_arc_reports_a_wider_phase_bar() {
        let bar = |count: usize, span: f64| {
            let (phis, mut values) = arc(count, span, 0.40, 110.0);
            for (index, value) in values.iter_mut().enumerate() {
                *value += f64::from(index as u8 % 2) * 0.02 - 0.01;
            }
            harmonic::<3>(&phis, &values)
                .expect("enough sites")
                .cycle(1)
                .phase_error_deg()
        };
        assert!(
            bar(24, 120.0) > 2.0 * bar(72, 360.0),
            "{} against {}",
            bar(24, 120.0),
            bar(72, 360.0)
        );
    }

    /// The five-term fit is not refused on the corpus's own thinnest arc: a
    /// rank floor that quietly rejected real data would turn every two-cycle
    /// row into a refusal and the refusal would look like a finding.
    #[test]
    fn five_terms_still_invert_on_a_195_degree_arc() {
        let (phis, values) = arc(20, 195.0, 0.40, 110.0);
        assert!(harmonic::<5>(&phis, &values).is_some());
    }

    /// The pooling test says "one vector" when there is one, and the arm that
    /// makes that count is the second half: vectors at scattered phases are
    /// not called poolable.
    #[test]
    fn the_pooling_test_separates_one_vector_from_many() {
        let agreeing: Vec<Row> = (0..6).map(|i| planted_row(i, 110.0)).collect();
        let read = wander(&agreeing, |row| &row.across, 1).expect("bars invert");
        assert!(read.chi_per_dof < 1.0, "{}", read.chi_per_dof);
        assert!(read.worst_leftover < 0.02, "{}", read.worst_leftover);
        let scattered: Vec<Row> = (0..6).map(|i| planted_row(i, 60.0 * i as f64)).collect();
        let read = wander(&scattered, |row| &row.across, 1).expect("bars invert");
        assert!(read.chi_per_dof > 100.0, "{}", read.chi_per_dof);
        assert!(read.worst_leftover > 0.3, "{}", read.worst_leftover);
    }

    /// A whole ring of one flight, at a given phase, with a little scatter so
    /// the bars are not zero.
    fn planted_row(seed: usize, phase_deg: f64) -> Row {
        let (phis, mut values) = arc(72, 360.0, 0.40, phase_deg);
        for (index, value) in values.iter_mut().enumerate() {
            *value += f64::from(((index + seed) % 3) as u8) * 0.01 - 0.01;
        }
        let fitted = harmonic::<3>(&phis, &values).expect("a whole ring pins a fit");
        Row {
            name: format!("flight {seed}"),
            sites: phis.len(),
            arc_deg: 360.0,
            gap_deg: 5.0,
            across: fitted.clone(),
            along: fitted,
            across_two: None,
            along_two: None,
            recovered: None,
        }
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
            harmonic::<3>(phis, &values).expect("enough sites").error
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
        assert!(harmonic::<3>(&phis, &values).is_none());
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
