//! What a live across-seam field would learn from the band's own readings, how
//! fast, and what it costs (issue #103, the epi fork; docs/research/seam-ghost.md).
//!
//! ```sh
//! # the owner's May-01 downward view: coverage, convergence and cost
//! cargo run --release -p kjerag-spike --bin ghost -- <file.insv> \
//!   from=45.5 count=900 yaw=-146.13 pitch=-37.35 fov=20 lock=1 seam=pool
//! # the control: hand the servo a field that is wrong by a known amount
//! cargo run --release -p kjerag-spike --bin ghost -- <file.insv> plant=0.5
//! ```
//!
//! **This is a simulation on real evidence and it is not a build.** The field
//! is never applied to the picture: what the pass draws on every frame here is
//! `main`'s picture. What is real is the evidence stream - the very `Cell`
//! values `ScenePipeline` dispatches into while it draws, read back frame by
//! frame - and what is modelled is one line: the band would read `disparity -
//! applied` once the field were applied, instead of the `disparity` it reads
//! here. That model is the linearised closed loop and it is the probe's one
//! assumption. Its warrant is measured elsewhere and is not re-derived here:
//! with a per-session across-seam term applied, the band kept every direction
//! it had and its epipolar mean fell 0.554 to 0.190 degrees (stage9 11.4).
//!
//! So the numbers below are honest about coverage, about the value the field
//! settles on, about the wall clock, and about the cost. They are a model, and
//! not a measurement, of the loop's own stability.
//!
//! **The cost column is a measurement.** The servo, the azimuth smoothing and
//! the staging filter are timed on the box that runs them, over the whole run,
//! and reported against the 8.44 ms the shipped pass costs a frame.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use kjerag_media::Fallible;
use kjerag_render::band::{KEEP, ease};
use kjerag_render::{AZIMUTHS, Camera, Cell, Cue, Horizon, Sampling, Scene, ScenePipeline, Size};
use kjerag_spike::{FORMAT, Gpu, Render, Seam};

/// How many view pixels one degree is at the width the seam residuals are
/// quoted in: 1920 across 90 degrees ([`--bin band`]'s own constant), so the
/// two instruments' view-pixel columns are the same statistic.
const VIEW_PX_PER_DEG: f64 = 16.8;

/// The same at the field of view the owner's downward complaint is banked at,
/// `fov=20` on a 1024 render, which is where the -46.4 view px in
/// docs/research/seam-temporal.md 2.1 is quoted.
const DOWN_PX_PER_DEG: f64 = 51.2;

// ------------------------------------------------------- the proposed servo

/// The largest single reading the servo will step by, in degrees. A rail on
/// the STEP and not on the value: one correlation that found the wrong feature
/// may not move the field further than the band's own filter would have.
const STEP_MAX_DEG: f32 = 0.25;

/// What a residual that points the near-field way is worth against one that
/// points the other way.
///
/// This is the far gate in its live form. Parallax on this axis is one-signed
/// (`Cell::metres` is `reach_m / disparity` and exists only where the
/// disparity is positive), so near content can only push a reading one way,
/// and a servo that leans against that way settles on a low quantile of what
/// it sees rather than on the mean. `--bin epifield`'s offline gate dropped a
/// moment whose excursion implied nearer than 60 m; this refuses to be moved
/// far by one, on evidence it never has to store.
const LEAN: f32 = 0.33;

/// The gain a direction's step is taken at once it has plenty of evidence.
/// Before that the gain is `1/n`, which makes the first reading the estimate
/// and the hundredth a hundredth of a correction.
const FLOOR_GAIN: f32 = 1.0 / 256.0;

/// The class bound on the composed term, in degrees, from stage9 12.3: the
/// largest term the gain sweep measured to leave the band's evidence intact.
/// It is a rail against a field that is not a calibration at all and it is not
/// the guard.
const RAIL_DEG: f32 = 2.8;

/// The staging filter's time constant, in seconds: `band::TAU_TRUST_S`'s own
/// value, because this is the same job on a different quantity.
const TAU_S: f32 = 2.0;

/// The azimuth smoothing kernel's half-width in degrees. `band::SMOOTH_DEG`'s
/// value, which is the along-seam table's own and is swept below.
const SMOOTH_DEG: f32 = 12.0;

/// How much a direction with no reading inside the kernel is shrunk by, in
/// readings' worth. `band::TABLE_RIDGE`'s value: an entry the ridge is taking
/// more than half of is the taper rather than a measurement.
const RIDGE: f32 = 1.0;

/// The live across-seam field, as this memo proposes to accumulate it.
///
/// Three arrays of [`AZIMUTHS`] and nothing else: 1.5 kB of state for the
/// whole ring, on the CPU, riding into the picture in a uniform block that is
/// written every redraw anyway.
struct Servo {
    /// The per-direction estimate, in radians. Integrated from what the band
    /// still reads, so its fixed point is the band reading nothing.
    field: [f32; AZIMUTHS],
    /// Readings accepted at each direction, which is both the gain schedule
    /// and the support count the taper is read off.
    seen: [f32; AZIMUTHS],
    /// What the picture would be drawn with: `field` smoothed along azimuth,
    /// tapered to nothing where there is no support, and walked in through the
    /// staging filter.
    applied: [f32; AZIMUTHS],
}

impl Servo {
    fn rest() -> Self {
        Self {
            field: [0.0; AZIMUTHS],
            seen: [0.0; AZIMUTHS],
            applied: [0.0; AZIMUTHS],
        }
    }

    /// A field that is wrong by a known constant before a single reading, which
    /// is the control: a servo that cannot walk one out is not a servo.
    fn planted(degrees: f32) -> Self {
        let mut servo = Self::rest();
        servo.field = [degrees.to_radians(); AZIMUTHS];
        servo.applied = servo.field;
        servo
    }

    /// One readback tick: what the band is still reading, integrated.
    ///
    /// `seconds` is the media time since the last tick, so the filter settles
    /// in the same wall time whatever the readback rate is.
    fn tick(&mut self, cells: &[Cell], seconds: f32, smooth_deg: f32, ridge: f32) {
        let step = STEP_MAX_DEG.to_radians();
        let rail = RAIL_DEG.to_radians();
        for (index, cell) in cells.iter().enumerate().take(AZIMUTHS) {
            // The band's own gate on whether a reading may enter a state, read
            // one level out. A direction below it contributes nothing at all,
            // which is what keeps an unread arc at identity.
            if cell.confidence < KEEP || !cell.disparity.is_finite() {
                continue;
            }
            self.seen[index] += 1.0;
            // THE CLOSED-LOOP MODEL, and the probe's one assumption: with the
            // field applied, this is what the band would be left to find.
            let residual = cell.disparity - self.applied[index];
            let gain = (1.0 / self.seen[index]).max(FLOOR_GAIN);
            let lean = match residual > 0.0 {
                true => LEAN,
                false => 1.0,
            };
            let moved = self.field[index] + gain * lean * residual.clamp(-step, step);
            self.field[index] = moved.clamp(-rail, rail);
        }
        let want = self.smoothed(smooth_deg, ridge);
        let learn = ease(seconds, TAU_S);
        for (applied, want) in self.applied.iter_mut().zip(want) {
            *applied += (want - *applied) * learn;
        }
    }

    /// The field smoothed along azimuth and tapered by its own support.
    ///
    /// The raised cosine and the ridge are `band::Table`'s, because this is the
    /// same job: a kernel that is zero at and past its own edge so a reading
    /// walking into a window does not put a corner in the picture, and a ridge
    /// that takes an entry with less than one reading's worth to nothing.
    ///
    /// **This is what the comb lesson requires.** No cell walks on its own:
    /// the value at a direction is a weighted mean over every direction inside
    /// the kernel, so the field has no teeth to comb.
    fn smoothed(&self, smooth_deg: f32, ridge: f32) -> [f32; AZIMUTHS] {
        // Only the directions inside the kernel, and the kernel's own weights
        // worked out once for the whole ring rather than per direction. The
        // first cut of this probe swept all 128 against all 128 and cost 98.7
        // us a tick; the arithmetic is the same and the cost is not.
        let width = smooth_deg.to_radians();
        let step = std::f32::consts::TAU / AZIMUTHS as f32;
        let reach = (width / step).ceil() as i32;
        let weights: Vec<f32> = (-reach..=reach)
            .map(|apart| kernel(apart as f32 * step, width))
            .collect();
        std::array::from_fn(|index| {
            let mut total = 0.0;
            let mut weight = 0.0;
            for (slot, kernel) in weights.iter().enumerate() {
                if *kernel <= 0.0 {
                    continue;
                }
                let other =
                    (index as i32 + slot as i32 - reach).rem_euclid(AZIMUTHS as i32) as usize;
                let support = self.seen[other].min(8.0) / 8.0;
                total += kernel * support * self.field[other];
                weight += kernel * support;
            }
            total / (weight + ridge)
        })
    }
}

/// The raised cosine `band::Table` smooths with: zero at and past its own edge.
fn kernel(apart: f32, width: f32) -> f32 {
    match apart.abs() < width {
        true => 0.5 * (1.0 + (std::f32::consts::PI * apart / width).cos()),
        false => 0.0,
    }
}

// ------------------------------------------------------------------- the run

/// One tick's worth of what happened, kept so the report can read the run
/// rather than the state it ended in.
struct Step {
    at: Duration,
    /// Directions of the owner's arc with any accepted reading yet.
    covered: usize,
    /// Directions of the arc reading ABOVE the band's gate on this very tick,
    /// which is what `left` and `today` are means over. It goes to zero when
    /// the arc goes dark, and a run where it does is saying so rather than
    /// printing a zero that looks like a fixed seam.
    read: usize,
    /// The mean of what the picture would be drawn with over the arc, radians.
    applied: f32,
    /// The mean of what the corridor would still be left to ramp, radians.
    residual: f32,
    /// The same with no field at all, which is what the corridor ramps today.
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
        "servo:  rate={:.1} Hz smooth={:.1} deg ridge={:.2} lean={LEAN} step={STEP_MAX_DEG} deg \
         tau={TAU_S} s plant={:.3} deg",
        options.rate, options.smooth, options.ridge, options.plant,
    );

    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let mut scene = Scene::still(&options.input, options.at())?;
    scene.set_horizon(match options.lock {
        true => Horizon::Locked,
        false => Horizon::Free,
    });
    options.seam.hold(&scene);

    let mut servo = match options.plant {
        0.0 => Servo::rest(),
        degrees => Servo::planted(degrees),
    };
    let arc = arc_cells(options.arc);
    let mut steps: Vec<Step> = Vec::new();
    let mut last: Option<Duration> = None;
    let mut cost = Vec::new();
    let mut frames = 0usize;
    let mut settled: Vec<Cell> = Vec::new();

    while let Some((_, at)) = scene.frame() {
        Render {
            gpu: &gpu,
            scene: &scene,
            pipeline: &mut pipeline,
        }
        .frame(options.camera(), Sampling::default(), options.size())?;
        let (_, cells) = pipeline.band_state(&gpu.device, &gpu.queue)?;
        frames += 1;

        // The readback rate is the design's, not the frame rate's: the field
        // moves on a filter measured in seconds and a tick per frame buys it
        // nothing.
        let due =
            last.is_none_or(|then| (at.saturating_sub(then)).as_secs_f64() >= 1.0 / options.rate);
        if due {
            let seconds = last.map_or(0.0, |then| (at.saturating_sub(then)).as_secs_f32());
            let started = Instant::now();
            servo.tick(&cells, seconds, options.smooth, options.ridge);
            cost.push(started.elapsed());
            last = Some(at);
            steps.push(measure(&servo, &cells, &arc, at));
        }
        settled = cells;
        if frames >= options.count || !scene.advance()? {
            break;
        }
    }
    if steps.is_empty() {
        return Err("no frame decoded at that instant".into());
    }

    report(&options, &steps, &servo, &settled, &arc, &cost, frames);
    Ok(())
}

/// What one tick left, over the arc the question is about.
fn measure(servo: &Servo, cells: &[Cell], arc: &[usize], at: Duration) -> Step {
    let mut covered = 0usize;
    let mut applied = 0.0f32;
    let mut residual = 0.0f32;
    let mut today = 0.0f32;
    let mut read = 0usize;
    for &index in arc {
        // What the picture is drawn with is a mean over the directions that
        // HAVE a field, not over the ones reading this instant: the field is
        // what a direction holds, and the arc going dark for a while does not
        // take it away. Reading it over the live set instead was this probe's
        // own first bug and it printed a corrected seam as a zero.
        if servo.seen[index] > 0.0 {
            covered += 1;
            applied += servo.applied[index];
        }
        let cell = &cells[index];
        if cell.confidence < KEEP {
            continue;
        }
        read += 1;
        residual += (cell.disparity - servo.applied[index]).abs();
        today += cell.disparity.abs();
    }
    Step {
        at,
        covered,
        read,
        applied: applied / covered.max(1) as f32,
        residual: residual / read.max(1) as f32,
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

// ---------------------------------------------------------------- the report

fn report(
    options: &Options,
    steps: &[Step],
    servo: &Servo,
    settled: &[Cell],
    arc: &[usize],
    cost: &[Duration],
    frames: usize,
) {
    let last = steps.last().expect("at least one tick");
    println!(
        "\nrun:    {frames} frames, {} ticks at {:.1} Hz, {:.2} s to {:.2} s of media",
        steps.len(),
        options.rate,
        steps[0].at.as_secs_f64(),
        last.at.as_secs_f64(),
    );

    println!(
        "\ncoverage of the arc {:.0} to {:.0} deg, {} cells. `covered` is directions with any \n\
         accepted reading; `applied` is the mean of what the picture would be drawn with over \n\
         the arc; `left` is what the corridor would still ramp and `today` is what it ramps now.\n",
        options.arc.0,
        options.arc.1,
        arc.len(),
    );
    println!(
        "  media s    covered   reading      applied deg   applied view px       left deg      today deg"
    );
    for step in landmarks(steps) {
        let (left, today) = match step.read {
            0 => ("             -".to_string(), "             -".to_string()),
            _ => (
                format!("{:>14.4}", f64::from(step.residual.to_degrees())),
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

    let target = last.applied;
    println!(
        "\nwhen the field arrives, against what it settles on ({:.4} deg):",
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

    println!("\nthe ring as a whole, at the end of the run:");
    let read: usize = (0..AZIMUTHS).filter(|&i| servo.seen[i] > 0.0).count();
    let plenty: usize = (0..AZIMUTHS).filter(|&i| servo.seen[i] >= 8.0).count();
    println!("  directions with any evidence      {read} of {AZIMUTHS}");
    println!("  directions past the taper's floor  {plenty} of {AZIMUTHS}");
    let worst = servo
        .applied
        .iter()
        .fold(0.0f32, |worst, value| worst.max(value.abs()));
    println!(
        "  largest applied value             {:.4} deg, rail is {RAIL_DEG}",
        f64::from(worst.to_degrees())
    );

    println!(
        "\nwhat the corridor is left to ramp over the arc, which is the contract. Read at the \n\
         last tick the arc was above the band's gate on, because a dark arc has nothing to say:"
    );
    match steps.iter().rev().find(|step| step.read > 0) {
        None => println!("  the arc never read above the gate in this run"),
        Some(step) => {
            println!(
                "  at {:.2} s of media, over {} directions",
                step.at.as_secs_f64(),
                step.read
            );
            println!(
                "  today                             {:.4} deg  ({:.1} view px at fov 20)",
                f64::from(step.today.to_degrees()),
                f64::from(step.today.to_degrees()) * DOWN_PX_PER_DEG,
            );
            println!(
                "  with the field                    {:.4} deg  ({:.1} view px at fov 20)",
                f64::from(step.residual.to_degrees()),
                f64::from(step.residual.to_degrees()) * DOWN_PX_PER_DEG,
            );
        }
    }

    // The band's own settled reading over the arc, for the value's sake: the
    // field has no business landing anywhere else.
    let mut total = 0.0f64;
    let mut count = 0.0f64;
    for &index in arc {
        if settled[index].confidence >= KEEP {
            total += f64::from(settled[index].disparity.to_degrees());
            count += 1.0;
        }
    }
    if count > 0.0 {
        println!(
            "\nthe band's own settled reading over the arc: {:.4} deg over {count:.0} directions \n\
             ({:.1} view px at fov 20, {:.1} at 90 deg across 1920)",
            total / count,
            total / count * DOWN_PX_PER_DEG,
            total / count * VIEW_PX_PER_DEG,
        );
    }

    if options.plant != 0.0 {
        println!(
            "\nthe control: the servo was handed {:.3} deg of field before it read anything. \n\
             It is now at {:.4} deg over the arc, having walked out {:.1}% of the plant.",
            f64::from(options.plant),
            f64::from(last.applied.to_degrees()),
            100.0 * (1.0 - f64::from(last.applied.to_degrees()) / f64::from(options.plant)),
        );
    }

    cost_report(cost, steps.len(), frames, options.rate);
}

/// What the servo costs, measured on the box that ran it.
fn cost_report(cost: &[Duration], ticks: usize, frames: usize, rate: f64) {
    let mut sorted: Vec<u128> = cost.iter().map(Duration::as_nanos).collect();
    sorted.sort_unstable();
    if sorted.is_empty() {
        return;
    }
    let mean = sorted.iter().sum::<u128>() as f64 / sorted.len() as f64;
    let p99 = sorted[(sorted.len() * 99 / 100).min(sorted.len() - 1)] as f64;
    let worst = *sorted.last().expect("sorted is not empty") as f64;
    // The shipped pass's own cost, from stage9 13.9 on this box class.
    let pass_ms = 8.44;
    let per_frame = mean * rate / 30.0;
    println!("\nwhat the servo costs, over {ticks} ticks and {frames} frames:");
    println!(
        "  mean per tick                     {:.1} us",
        mean / 1000.0
    );
    println!("  p99 per tick                      {:.1} us", p99 / 1000.0);
    println!(
        "  worst tick                        {:.1} us",
        worst / 1000.0
    );
    println!(
        "  amortised per frame at 30 fps     {:.1} us, which is {:.3}% of the {pass_ms} ms pass",
        per_frame / 1000.0,
        per_frame / 1_000_000.0 / pass_ms * 100.0,
    );
}

/// The ticks worth printing: the first, then a landmark every few seconds, then
/// the last, because a table of every tick is not a table anybody reads.
fn landmarks(steps: &[Step]) -> Vec<&Step> {
    let start = steps[0].at.as_secs_f64();
    let wanted = [
        0.0f64, 0.5, 1.0, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0, 30.0, 45.0, 60.0,
    ];
    let mut out: Vec<&Step> = Vec::new();
    for want in wanted {
        if let Some(step) = steps
            .iter()
            .find(|step| step.at.as_secs_f64() - start >= want)
        {
            if !out.iter().any(|kept| kept.at == step.at) {
                out.push(step);
            }
        }
    }
    if let Some(last) = steps.last() {
        if !out.iter().any(|kept| kept.at == last.at) {
            out.push(last);
        }
    }
    out
}

// ------------------------------------------------------------------ options

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
    /// How often the field would be read back and stepped, in Hz.
    rate: f64,
    /// The azimuth smoothing kernel's half-width, in degrees.
    smooth: f32,
    /// How much a direction with little support is shrunk by, in readings'
    /// worth. An argument because the along-seam table's own 1.0 is a
    /// measurable haircut on this axis and the memo has to price it.
    ridge: f32,
    /// A constant field, in degrees, handed to the servo before it reads
    /// anything. The control.
    plant: f32,
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
            rate: 10.0,
            smooth: SMOOTH_DEG,
            ridge: RIDGE,
            plant: 0.0,
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
                Some(("rate", value)) => options.rate = value.parse()?,
                Some(("smooth", value)) => options.smooth = value.parse()?,
                Some(("ridge", value)) => options.ridge = value.parse()?,
                Some(("plant", value)) => options.plant = value.parse()?,
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
     [fov=deg] [size=px] [lock=0] [arc=low:high] [rate=hz] [smooth=deg] [ridge=n] [plant=deg] \
     [seam=factory|file|pool]";
