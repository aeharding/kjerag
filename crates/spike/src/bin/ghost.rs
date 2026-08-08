//! What the live across-seam field does with the loop **closed**: what it
//! learns, how fast, what it leaves the corridor to ramp, and whether it
//! settles or rings (issue #103, the epi fork; docs/research/seam-ghost.md).
//!
//! ```sh
//! # the owner's May-01 downward view: arrival, coverage and the corridor's load
//! cargo run --release -p kjerag-spike --bin ghost -- <file.insv> \
//!   from=65.666 count=900 yaw=179.00 pitch=-36.97 fov=20.00 lock=1 seam=pool
//! # the same arm with the field switched off, which is what main draws
//! cargo run --release -p kjerag-spike --bin ghost -- <file.insv> field=0 ...
//! # the control: a field that is wrong by half a degree before it reads anything
//! cargo run --release -p kjerag-spike --bin ghost -- <file.insv> plant=0.5 seen=8
//! # the poison: a field learned CORRECTLY off content that has since left
//! cargo run --release -p kjerag-spike --bin ghost -- <file.insv> plant=0.24 seen=655
//! ```
//!
//! **This is a measurement and not a simulation, and that is what changed.**
//! The research probe on `research/seam-ghost` ran a private copy of the servo
//! beside a pass that applied nothing, and modelled one line: that the band
//! would read `disparity - applied` once the field were applied. Here the field
//! **is** applied. `ScenePipeline` draws through it and the band's compute pass
//! searches lens 1 through it, so every number below is taken off the loop the
//! player runs, closed, and the memo's open question 2 is what this answers.
//!
//! Two columns are the contract and they are both read off the same cells. What
//! the band still reports is `left`, which is what the corridor is still ramping
//! and is the defect's own size. What it would have reported with nothing
//! applied is `today`, which is that plus what the field is drawing. A run with
//! `field=0` prints the same two columns equal to each other, which is the check
//! that the reconstruction is not inventing the win.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use kjerag_media::Fallible;
use kjerag_render::Size;
use kjerag_render::band::KEEP;
use kjerag_render::{AZIMUTHS, Camera, Cell, Cue, Ghost, Horizon, Sampling, Scene, ScenePipeline};
use kjerag_spike::{FORMAT, Gpu, Render, Seam};

/// How many view pixels one degree is at the field of view the owner's
/// downward complaint is banked at, `fov=20` on a 1024 render, which is where
/// the -46.4 view px in docs/research/seam-temporal.md 2.1 is quoted.
const DOWN_PX_PER_DEG: f64 = 51.2;

/// The shipped pass's own cost per frame in milliseconds, from stage9 13.9 on
/// this box class, which the servo's cost is quoted against.
const PASS_MS: f64 = 8.44;

/// How long the field is given to arrive before the contract is read, in
/// seconds of media. Its own 99 percent at the owner's own view, rounded up.
const SETTLED_S: f32 = 12.0;

/// One tick's worth of what happened, kept so the report can read the run
/// rather than the state it ended in.
struct Step {
    at: Duration,
    /// Directions of the arc the field holds anything at.
    covered: usize,
    /// Directions of the arc reading above the band's gate on this very tick.
    /// It goes to zero when the arc goes dark, and a run where it does says so
    /// rather than printing a zero that looks like a fixed seam.
    read: usize,
    /// The mean of what the picture IS drawn with over the arc, radians.
    applied: f32,
    /// The mean of what the corridor is still left to ramp, radians: the
    /// band's own reading, measured through the field.
    left: f32,
    /// The same with the field added back, which is what the corridor ramps
    /// with no field at all.
    today: f32,
}

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    println!(
        "aim:    {} from={:.3} count={} yaw={} pitch={} fov={} seam={}",
        options.input.display(),
        options.from,
        options.count,
        options.yaw,
        options.pitch,
        options.fov,
        options.pose,
    );
    println!(
        "field:  {}, planted {:.3} deg at seen={:.0}, read back every frame",
        match options.field {
            true => "ON, applied to the picture and read through by the band",
            false => "OFF, which is main's picture",
        },
        options.plant,
        options.plant_seen,
    );

    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    pipeline.use_ghost(options.field);
    if options.field && options.plant != 0.0 {
        pipeline.plant_ghost(Ghost::planted(
            options.plant.to_radians(),
            options.plant_seen,
        ));
    }
    let mut scene = Scene::still(&options.input, options.at())?;
    scene.set_horizon(match options.lock {
        true => Horizon::Locked,
        false => Horizon::Free,
    });
    options.seam.hold(&scene);

    let arc = arc_cells(options.arc);
    let mut steps: Vec<Step> = Vec::new();
    let mut cost = Vec::new();
    let mut frames = 0usize;
    let mut settled: Vec<Cell> = Vec::new();
    let mut shadow = Ghost::rest();
    let mut last: Option<Duration> = None;
    // What the far gate was offered and what it refused, per direction, over
    // the WHOLE run. The end-of-run snapshot cannot answer this: a direction
    // reads a handful of frames and the question is what it was reading while
    // the field was being learned.
    let mut offered = [0usize; AZIMUTHS];
    let mut refused = [0usize; AZIMUTHS];
    // And the sum of the WHOLE readings, which is what a servo with no gate at
    // all would settle on. The counterfactual for nothing, and it needs no
    // second copy of the servo to compute: an ungated `1/n` servo IS a running
    // mean of its input.
    let mut ungated = [0.0f64; AZIMUTHS];

    while let Some((_, at)) = scene.frame() {
        Render {
            gpu: &gpu,
            scene: &scene,
            pipeline: &mut pipeline,
        }
        .frame(options.camera(), Sampling::default(), options.size())?;
        let (_, cells) = pipeline.band_state(&gpu.device, &gpu.queue)?;
        frames += 1;

        // The servo's own cost, timed on the same arithmetic over the same
        // cells. It is a shadow of the shipped tick and not the shipped tick,
        // because the shipped one runs inside `prepare` where a clock cannot
        // reach it without being in the frame path; what the readback costs is
        // in the frame rate and is measured by `--bin playback`.
        let seconds = last
            .replace(at)
            .map_or(1.0 / 30.0, |then| at.as_secs_f32() - then.as_secs_f32());
        let started = Instant::now();
        shadow.tick(&cells, seconds);
        cost.push(started.elapsed());

        let applied = pipeline
            .ghost()
            .map_or([0.0; AZIMUTHS], |ghost| ghost.table().entries());
        for (index, cell) in cells.iter().enumerate().take(AZIMUTHS) {
            if cell.confidence < KEEP || !cell.disparity.is_finite() {
                continue;
            }
            offered[index] += 1;
            // The shipped gate's own arithmetic, on the shipped gate's own
            // quantity: the band's reading plus what the field draws.
            let whole = cell.disparity + applied[index];
            ungated[index] += f64::from(whole.to_degrees());
            if whole > 0.0 {
                refused[index] += 1;
            }
        }
        steps.push(measure(&applied, &cells, &arc, at));
        settled = cells;
        if frames >= options.count || !scene.advance()? {
            break;
        }
    }
    if steps.is_empty() {
        return Err("no frame decoded at that instant".into());
    }

    report(&options, &steps, &pipeline, &settled, &arc, &cost, frames);
    gate(&pipeline, &offered, &refused, &ungated);
    Ok(())
}

/// What one frame left, over the arc the question is about.
fn measure(applied: &[f32; AZIMUTHS], cells: &[Cell], arc: &[usize], at: Duration) -> Step {
    let mut covered = 0usize;
    let mut mean = 0.0f32;
    let mut left = 0.0f32;
    let mut today = 0.0f32;
    let mut read = 0usize;
    for &index in arc {
        // What the picture is drawn with is a mean over the directions that
        // HAVE a field, not over the ones reading this instant: the field is
        // what a direction holds, and the arc going dark for a while does not
        // take it away.
        if applied[index] != 0.0 {
            covered += 1;
            mean += applied[index];
        }
        let cell = &cells[index];
        if cell.confidence < KEEP {
            continue;
        }
        read += 1;
        left += cell.disparity.abs();
        today += (cell.disparity + applied[index]).abs();
    }
    Step {
        at,
        covered,
        read,
        applied: mean / covered.max(1) as f32,
        left: left / read.max(1) as f32,
        today: today / read.max(1) as f32,
    }
}

/// The directions of the ring an azimuth arc in degrees covers.
fn arc_cells((low, high): (f64, f64)) -> Vec<usize> {
    let per = 360.0 / AZIMUTHS as f64;
    (0..AZIMUTHS)
        .filter(|index| {
            let phi = *index as f64 * per;
            phi >= low && phi <= high
        })
        .collect()
}

fn report(
    options: &Options,
    steps: &[Step],
    pipeline: &ScenePipeline,
    settled: &[Cell],
    arc: &[usize],
    cost: &[Duration],
    frames: usize,
) {
    let last = steps.last().expect("at least one frame");
    println!(
        "\nrun:    {frames} frames, {:.2} s to {:.2} s of media",
        steps[0].at.as_secs_f64(),
        last.at.as_secs_f64(),
    );
    println!(
        "\ncoverage of the arc {:.0} to {:.0} deg, {} cells. `applied` is the mean of what the \n\
         picture IS drawn with there; `left` is what the corridor is STILL ramping and `today` \n\
         is what it would ramp with no field. Both are the same cells, read through the field.\n",
        options.arc.0,
        options.arc.1,
        arc.len(),
    );
    println!(
        "  media s    covered   reading      applied deg   applied view px       left deg      today deg"
    );
    for step in landmarks(steps) {
        let (left, today) = match step.read {
            0 => ("             -".to_owned(), "             -".to_owned()),
            _ => (
                format!("{:>14.4}", f64::from(step.left.to_degrees())),
                format!("{:>14.4}", f64::from(step.today.to_degrees())),
            ),
        };
        println!(
            "{:>9.2} {:>6} of {:<4} {:>7} {:>16.4} {:>17.1} {left} {today}",
            step.at.as_secs_f64(),
            step.covered,
            arc.len(),
            step.read,
            f64::from(step.applied.to_degrees()),
            f64::from(step.applied.to_degrees()) * DOWN_PX_PER_DEG,
        );
    }

    arrival(steps);
    trajectory(steps);

    println!("\nthe ring as a whole, at the end of the run:");
    let field = pipeline.ghost().map(Ghost::table).unwrap_or_default();
    let entries = field.entries();
    let drawn = entries.iter().filter(|entry| **entry != 0.0).count();
    println!("  directions the field draws at      {drawn} of {AZIMUTHS}");
    let worst = entries.iter().fold(0.0f32, |worst, at| worst.max(at.abs()));
    println!(
        "  largest applied value              {:.4} deg, rail is 2.8",
        f64::from(worst.to_degrees())
    );
    // The comb question, answered on the field itself: neighbouring entries of
    // a kernel-smoothed field cannot differ by much, and this is the number.
    let teeth = (0..AZIMUTHS)
        .map(|index| (entries[(index + 1) % AZIMUTHS] - entries[index]).abs())
        .fold(0.0f32, f32::max);
    println!(
        "  largest step between neighbours    {:.4} deg ({:.2} view px at fov 20)",
        f64::from(teeth.to_degrees()),
        f64::from(teeth.to_degrees()) * DOWN_PX_PER_DEG,
    );

    contract(steps, arc.len());

    evidence(settled, &entries, arc);

    if options.plant != 0.0 {
        println!(
            "\nthe control: the field was handed {:.3} deg at seen={:.0} before it read anything. \n\
             It is now at {:.4} deg over the arc, having walked out {:.1} percent of the plant.",
            f64::from(options.plant),
            f64::from(options.plant_seen),
            f64::from(last.applied.to_degrees()),
            100.0 * (1.0 - f64::from(last.applied.to_degrees()) / f64::from(options.plant)),
        );
    }

    cost_report(cost, frames);
}

/// When the field arrived, against what it ended the run at.
fn arrival(steps: &[Step]) {
    let target = steps.last().expect("a run").applied;
    println!(
        "\nwhen the field arrives, against what it ends the run at ({:.4} deg):",
        f64::from(target.to_degrees())
    );
    for share in [0.5f32, 0.9, 0.99] {
        let reached = steps
            .iter()
            .find(|step| (step.applied / target).is_finite() && step.applied / target >= share);
        match reached {
            Some(step) => println!(
                "  {:>3.0}% of it at {:>6.2} s of media",
                f64::from(share) * 100.0,
                step.at.as_secs_f64() - steps[0].at.as_secs_f64(),
            ),
            None => println!(
                "  {:>3.0}% never reached in this run",
                f64::from(share) * 100.0
            ),
        }
    }
}

/// **Does it ring.** The memo's open question 2, answered as a number: a stable
/// loop approaches its answer from one side and stops there.
fn trajectory(steps: &[Step]) {
    let settled = steps.last().expect("a run").applied;
    let worst = steps
        .iter()
        .fold(0.0f32, |worst, step| worst.max(step.applied.abs()));
    let overshoot = match settled.abs() > 0.0 {
        true => (worst.abs() / settled.abs() - 1.0) * 100.0,
        false => 0.0,
    };
    // How many times the walk turned round after it first got within a tenth
    // of where it ended, which is a ring counted rather than argued.
    let near = steps
        .iter()
        .position(|step| (step.applied - settled).abs() <= settled.abs() * 0.1)
        .unwrap_or(steps.len());
    let turns = steps[near..]
        .windows(3)
        .filter(|w| {
            let (a, b, c) = (w[0].applied, w[1].applied, w[2].applied);
            (b - a).signum() != (c - b).signum() && (c - b).abs() > settled.abs() * 0.002
        })
        .count();
    println!(
        "\nstability: the walk reached {:.4} deg at its furthest against {:.4} at the end, which \n\
         is {overshoot:+.2} percent of overshoot, and it turned round {turns} time(s) after first \n\
         coming within a tenth of where it ended.",
        f64::from(worst.to_degrees()),
        f64::from(settled.to_degrees()),
    );
}

/// THE CONTRACT, pooled: what the corridor is left to ramp over the arc once
/// the field has arrived.
///
/// Over every frame past [`SETTLED_S`] and weighted by how many directions were
/// above the band's gate on it, because one frame with one direction reading is
/// not a measurement of an arc and reading it off the last such frame was this
/// instrument's own first bug. A frame the whole arc is dark on contributes
/// nothing rather than a zero.
fn contract(steps: &[Step], cells: usize) {
    let from = steps[0].at + Duration::from_secs_f32(SETTLED_S);
    let (mut left, mut today, mut weight, mut frames) = (0.0f64, 0.0f64, 0.0f64, 0usize);
    let mut most = 0usize;
    for step in steps.iter().filter(|step| step.at >= from && step.read > 0) {
        let w = f64::from(step.read as u32);
        left += w * f64::from(step.left.to_degrees());
        today += w * f64::from(step.today.to_degrees());
        weight += w;
        frames += 1;
        most = most.max(step.read);
    }
    println!(
        "\nTHE CONTRACT: what the corridor is left to ramp over the arc, pooled over the {frames} \n\
         frame(s) past {SETTLED_S:.0} s that read anything, weighted by how much of the arc each \n\
         one read (up to {most} of {cells} directions):"
    );
    if weight <= 0.0 {
        println!("  the arc never read above the gate after it settled");
        return;
    }
    for (label, value) in [("today", today / weight), ("with the field", left / weight)] {
        println!(
            "  {label:<33} {value:.4} deg  ({:.1} view px at fov 20)",
            value * DOWN_PX_PER_DEG,
        );
    }
    println!(
        "  the corridor's load falls by          {:.1} percent",
        100.0 * (1.0 - (left / weight) / (today / weight).max(1e-9)),
    );
}

/// Every reading the arc offered the servo, by **sign**, which is the one
/// column that tells a camera term from near ground.
///
/// Parallax on this axis is one-signed and positive, so a reading short of zero
/// is a distance no content can stand at and is the camera. What is printed is
/// the whole disagreement - the band's reading plus what the field is drawing -
/// because that is the quantity a distance is read off and the quantity the
/// far gate is asked about.
fn evidence(settled: &[Cell], applied: &[f32; AZIMUTHS], arc: &[usize]) {
    println!(
        "\nwhat the arc reads, by sign, at the end of the run. `whole` is the band's reading plus \n\
         what the field draws, which is what the band would report with nothing applied and what \n\
         the far gate refuses when it is past zero. `nearest` is what that whole would be if it \n\
         were all parallax."
    );
    println!("\n   phi      whole deg       left deg      field deg      nearest       gate");
    let mut near = 0usize;
    let mut seen = 0usize;
    for &index in arc {
        let cell = &settled[index];
        if cell.confidence < KEEP {
            println!(
                "{:>6}              -",
                index as f64 / AZIMUTHS as f64 * 360.0
            );
            continue;
        }
        seen += 1;
        let whole = cell.disparity + applied[index];
        let nearest = match whole > 0.0 && cell.reach_m > 0.0 {
            true => format!("{:.1} m", f64::from(cell.reach_m / whole)),
            false => "-".to_owned(),
        };
        if whole > 0.0 {
            near += 1;
        }
        println!(
            "{:>6} {:>14.4} {:>14.4} {:>14.4} {nearest:>12} {:>10}",
            index as f64 / AZIMUTHS as f64 * 360.0,
            f64::from(whole.to_degrees()),
            f64::from(cell.disparity.to_degrees()),
            f64::from(applied[index].to_degrees()),
            match whole > 0.0 {
                true => "REFUSED",
                false => "kept",
            },
        );
    }
    println!(
        "\n  over the arc: {near} of {seen} directions read past zero at the end, so that many \n\
         could have had any content in them at all and the field refuses to be moved by them."
    );
}

/// THE FAR GATE, over the whole run rather than at one frame.
///
/// Parallax on this axis is one-signed and positive, so a reading past zero is
/// near content by construction and the field refuses to be moved by one. The
/// question this answers is the one the landing chapter asks: at the directions
/// that were looking at NEAR content while the field was being learned, did the
/// field learn anything?
///
/// A direction is called near-fed where the gate refused most of what it was
/// offered. The acceptance is that those directions are at identity, and the
/// number is the mean of what they actually hold.
fn gate(
    pipeline: &ScenePipeline,
    offered: &[usize; AZIMUTHS],
    refused: &[usize; AZIMUTHS],
    ungated: &[f64; AZIMUTHS],
) {
    let Some(field) = pipeline.ghost().map(Ghost::table) else {
        return;
    };
    let entries = field.entries();
    let (mut near, mut near_field, mut near_worst) = (0usize, 0.0f64, 0.0f64);
    let (mut near_ungated, mut near_ungated_worst) = (0.0f64, 0.0f64);
    let (mut far, mut far_field) = (0usize, 0.0f64);
    let (mut all, mut cut) = (0usize, 0usize);
    for index in 0..AZIMUTHS {
        all += offered[index];
        cut += refused[index];
        if offered[index] < 8 {
            continue;
        }
        let held = f64::from(entries[index].to_degrees()).abs();
        // Most of what it was offered was past zero: this direction was
        // measured on content a distance can reach, for most of the run.
        if refused[index] * 2 > offered[index] {
            near += 1;
            near_field += held;
            near_worst = near_worst.max(held);
            let would = (ungated[index] / offered[index] as f64).abs();
            near_ungated += would;
            near_ungated_worst = near_ungated_worst.max(would);
        } else {
            far += 1;
            far_field += held;
        }
    }
    println!(
        "\nTHE FAR GATE over the whole run: {cut} of {all} readings were past zero and refused, \n\
         which is {:.1} percent of everything the field was offered.",
        100.0 * cut as f64 / all.max(1) as f64,
    );
    println!(
        "  directions fed mostly NEAR content  {near}, holding {:.4} deg on average, worst {near_worst:.4}",
        near_field / near.max(1) as f64,
    );
    println!(
        "  the same with NO gate at all        would hold {:.4} deg on average, worst {near_ungated_worst:.4}",
        near_ungated / near.max(1) as f64,
    );
    println!(
        "  directions fed mostly far content   {far}, holding {:.4} deg on average",
        far_field / far.max(1) as f64,
    );
}

/// What the servo costs, measured on the box that ran it.
fn cost_report(cost: &[Duration], frames: usize) {
    let mut sorted: Vec<u128> = cost.iter().map(Duration::as_nanos).collect();
    sorted.sort_unstable();
    if sorted.is_empty() {
        return;
    }
    let mean = sorted.iter().sum::<u128>() as f64 / sorted.len() as f64;
    let p99 = sorted[(sorted.len() * 99 / 100).min(sorted.len() - 1)] as f64;
    println!("\nwhat the servo costs, over {frames} frames:");
    println!(
        "  mean per frame                     {:.1} us",
        mean / 1000.0
    );
    println!(
        "  p99 per frame                      {:.1} us",
        p99 / 1000.0
    );
    println!(
        "  which is                           {:.3}% of the {PASS_MS} ms pass",
        mean / 1_000_000.0 / PASS_MS * 100.0,
    );
}

/// The frames worth printing: the first, then a landmark every few seconds,
/// then the last, because a table of every frame is not a table anybody reads.
fn landmarks(steps: &[Step]) -> Vec<&Step> {
    let start = steps[0].at.as_secs_f64();
    let mut out: Vec<&Step> = Vec::new();
    for want in [
        0.0f64, 0.5, 1.0, 2.0, 3.0, 5.0, 8.0, 12.0, 16.0, 20.0, 30.0, 45.0, 60.0,
    ] {
        if let Some(step) = steps
            .iter()
            .find(|step| step.at.as_secs_f64() - start >= want)
            && !out.iter().any(|kept| kept.at == step.at)
        {
            out.push(step);
        }
    }
    if let Some(last) = steps.last()
        && !out.iter().any(|kept| kept.at == last.at)
    {
        out.push(last);
    }
    out
}

struct Options {
    input: PathBuf,
    from: f64,
    count: usize,
    yaw: f64,
    pitch: f64,
    fov: f64,
    size: u32,
    lock: bool,
    /// The azimuths the report calls the arc, in degrees. 93 to 125 is where
    /// the owner's three banked downward views sit.
    arc: (f64, f64),
    /// Whether the field is drawn at all. `field=0` is main's picture and is
    /// the arm every number here is read against.
    field: bool,
    /// A constant field, in degrees, handed to the servo before it reads
    /// anything. The control.
    plant: f32,
    /// How much evidence the planted field arrives with, which is the
    /// difference between the easy control and the honest one.
    plant_seen: f32,
    seam: Seam,
    /// The `seam=` line as it was written, so a run can say what pose it drew
    /// its numbers at.
    pose: String,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            from: 0.0,
            count: 900,
            yaw: 90.0,
            pitch: 0.0,
            fov: 60.0,
            size: 1024,
            lock: true,
            arc: (93.0, 125.0),
            field: true,
            plant: 0.0,
            plant_seen: 0.0,
            seam: Seam::File,
            pose: String::new(),
        };
        let mut seam = String::from("file");
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("from", value)) => options.from = value.parse()?,
                Some(("count", value)) => options.count = value.parse()?,
                Some(("yaw", value)) => options.yaw = value.parse()?,
                Some(("pitch", value)) => options.pitch = value.parse()?,
                Some(("fov", value)) => options.fov = value.parse()?,
                Some(("size", value)) => options.size = value.parse()?,
                Some(("lock", value)) => options.lock = value.parse::<u32>()? != 0,
                Some(("field", value)) => options.field = value.parse::<u32>()? != 0,
                Some(("plant", value)) => options.plant = value.parse()?,
                Some(("seen", value)) => options.plant_seen = value.parse()?,
                Some(("seam", value)) => seam = value.to_string(),
                Some(("arc", value)) => {
                    let (low, high) = value.split_once(':').ok_or("arc=<low deg>:<high deg>")?;
                    options.arc = (low.parse()?, high.parse()?);
                }
                Some((key, _)) => return Err(format!("no argument called {key}").into()),
            }
        }
        if options.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        options.seam = Seam::parse(&seam, &options.input)?;
        options.pose = seam;
        Ok(options)
    }

    fn at(&self) -> Cue {
        Cue::Time(Duration::from_secs_f64(self.from.max(0.0)))
    }

    fn size(&self) -> Size {
        Size::new(self.size, self.size)
    }

    fn camera(&self) -> Camera {
        Camera {
            yaw: self.yaw.to_radians() as f32,
            pitch: self.pitch.to_radians() as f32,
            fov: self.fov.to_radians() as f32,
        }
    }
}

const USAGE: &str = "usage: ghost <file.insv> [from=seconds] [count=frames] [yaw=deg] [pitch=deg] \
     [fov=deg] [size=px] [lock=0] [arc=low:high] [field=0] [plant=deg] [seen=n] \
     [seam=factory|file|pool]";
