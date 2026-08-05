//! Where the locked horizon leans, second by second, and what is leaning it.
//!
//! `dip` measures one instant a whole circle round and fits the sinusoid a
//! constant tilt in the estimated vertical draws. That is the shape of the
//! defect; this is its history. It walks the opening of a file, measures the
//! same circle every so often, and writes both the fit and what the IMU was
//! doing at that instant into CSV, so the tilt can be plotted against time,
//! against the turn rate, and against the accelerometer's own disagreement
//! with gravity.
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin lean -- <file.insv> to=40
//! cargo run --release -p kjerag-spike --bin lean -- <file.insv> clock=1
//! cargo run --release -p kjerag-spike --bin lean -- <file.insv> from=12 \
//!     shifts=-600_600_50
//! ```
//!
//! Arguments after the path are `key=value`. `from` and `to` bound the walk
//! in seconds and `every` is how many frames apart the circles are; `steps`
//! is how many yaws a circle is cut into, and `pitch`, `fov`, `width` and
//! `height` shape the view. `tag` names the CSV files, which land in
//! gitignored `scratch/`: they are measurements of somebody's real flight.
//!
//! ## The three injections, and why an instrument needs them
//!
//! A probe that has never been shown to fire proves nothing when it stays
//! quiet (the 2026-07 flake campaign, twice). Each hypothesis this instrument
//! is pointed at therefore has a switch that manufactures it:
//!
//! - `inject=deg about=deg` tilts the held orientation about a horizontal
//!   world axis. This is a wrong vertical by construction, so the `tilt`
//!   column has to read the injected angle back. It is `dip`'s control and it
//!   is carried here so that this instrument's own measurement is checked
//!   against the same known answer.
//! - `centripetal=speed` adds the specific force an aircraft flying that many
//!   metres a second would feel in the turn the gyroscope actually recorded,
//!   `speed * rate` outward, before the filter ever sees the track. If a
//!   coordinated turn is what leans the horizon, this is the knob that leans
//!   it further, and a run where it changes nothing is a run where the
//!   accelerometer is not the path.
//! - `skew=ms` moves the whole IMU track against the video before it is
//!   solved, which is what a wrong `gyro_timestamp` or a wrong tick does. The
//!   `shifts=` sweep then looks the orientation up at a range of offsets and
//!   reports where the tilt is smallest; on a file with an injected skew that
//!   minimum has to move by the skew, and that is what says the sweep can
//!   find one at all.
//!
//! `clock=1` prints the timing chain with no GPU and no pixels: the constants
//! the trailer hands over, and whether the IMU's own timeline steps, gaps or
//! changes rate over the opening. It cannot see a constant offset and does
//! not pretend to. Nothing in the file is a second clock to compare against,
//! so a constant offset is only visible in the picture, which is what
//! `shifts=` is for.

use std::f64::consts::TAU;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::ops::Neg;
use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_meta::{CalibrationSet, Filter, GyroSample, GyroTrack, Mat3, OrientationTrack, Quat};
use kjerag_render::{Camera, Cue, Horizon, Scene, ScenePipeline, Size};
use kjerag_spike::{Gpu, Offscreen, skyline};

/// Not sRGB, so the shader writes the video's own numbers straight out and
/// the measurement reads what the window shows.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Where the estimate says up is, in the frame the camera reads its rays in.
/// y is down, so up is its negative.
const UP: [f64; 3] = [0.0, -1.0, 0.0];

/// One g, in metres a second squared, which is what turns a centripetal
/// acceleration into the units an accelerometer records.
const G: f64 = 9.80665;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let calibration = CalibrationSet::from_insv(&options.input)?;
    if calibration.imu.is_empty() {
        return Err("this file carries no IMU record, so there is nothing to lock to".into());
    }
    fs::create_dir_all("scratch")?;
    if options.clock {
        return clock_chain(&calibration, &options);
    }
    match options.shifts {
        Some(shifts) => shift_sweep(&calibration, &options, shifts),
        None => walk(&calibration, &options),
    }
}

/// The opening of the file, measured every `every` frames.
fn walk(calibration: &CalibrationSet, options: &Options) -> Fallible<()> {
    let to_body = calibration.body_from_imu();
    let imu = options.contaminated(&calibration.imu, to_body);
    let track = Filter::default().solve(&imu, to_body);
    let mut scene = options.scene()?;
    let mut rig = Rig::open(options.size)?;
    let mut instants = Csv::open(options, "instants", INSTANT_HEADER)?;
    let mut views = Csv::open(options, "views", VIEW_HEADER)?;
    let mut measured = 0usize;
    report_seed(&imu, to_body);

    println!(
        "\n{:>8} {:>6} {:>9} {:>9} {:>8} {:>9} {:>8} {:>8} {:>9} {:>9}",
        "seconds",
        "views",
        "tilt deg",
        "toward",
        "amp deg",
        "phase",
        "rms",
        "|a| g",
        "a lean",
        "turn dps"
    );
    while let Some((index, stamp)) = scene.frame() {
        let seconds = stamp.as_secs_f64();
        if seconds > options.to {
            break;
        }
        if index.is_multiple_of(options.every) {
            let at = calibration.exposure[0]
                .frame_time_us(index)
                .unwrap_or((seconds * 1e6) as i64);
            let held = options.injected(track.at(at + options.shift_us));
            let points = rig.circle(&scene, held, options)?;
            let sensed = Sensed::at(&imu, &track, to_body, at, Filter::default());
            let summary = Summary::of(&points, &sensed, seconds, held.heading().to_degrees());
            println!("{summary}");
            instants.row(&summary.row())?;
            for point in &points {
                views.row(&[
                    format!("{seconds:.3}"),
                    format!("{:.1}", point.yaw.to_degrees()),
                    format!("{:.3}", point.angle),
                    format!("{:.3}", point.height),
                ])?;
            }
            measured += 1;
        }
        if !scene.advance()? {
            break;
        }
    }

    println!("\n{measured} circles measured, {}", options.describe());
    println!("csv:    {}", instants.path.display());
    println!("csv:    {}", views.path.display());
    println!(
        "\ntilt is the angle between where the picture says up is and where the estimate does. \n\
         amp and phase are the sinusoid fitted to the horizon's angle against view yaw, which is \n\
         the same defect the way a pilot meets it. a lean is how far the smoothed accelerometer \n\
         is from the estimate's own vertical, and turn dps is the body's rate about that vertical."
    );
    Ok(())
}

/// The same circle at one instant, looked up at a range of offsets against
/// the video clock. Where the tilt bottoms out is where the two clocks agree,
/// if disagreeing is what is wrong.
fn shift_sweep(calibration: &CalibrationSet, options: &Options, shifts: [f64; 3]) -> Fallible<()> {
    let to_body = calibration.body_from_imu();
    let imu = options.contaminated(&calibration.imu, to_body);
    let track = Filter::default().solve(&imu, to_body);
    let scene = options.scene()?;
    let mut rig = Rig::open(options.size)?;
    let mut sweep = Csv::open(options, "shifts", SHIFT_HEADER)?;
    let Some((index, stamp)) = scene.frame() else {
        return Err("no frame at that time".into());
    };
    let at = calibration.exposure[0]
        .frame_time_us(index)
        .unwrap_or((stamp.as_secs_f64() * 1e6) as i64);

    println!(
        "\nframe at {:.3} s, {}\n",
        stamp.as_secs_f64(),
        options.describe()
    );
    println!(
        "{:>10} {:>6} {:>9} {:>9} {:>9}",
        "shift ms", "views", "tilt deg", "amp deg", "rms"
    );
    let [first, last, step] = shifts;
    let count = ((last - first) / step).round().max(0.0) as i64;
    let mut best: Option<(f64, f64)> = None;
    for at_step in 0..=count {
        let shift_ms = first + at_step as f64 * step;
        let held = options.injected(track.at(at + (shift_ms * 1_000.0) as i64));
        let points = rig.circle(&scene, held, options)?;
        let tilt = tilt_of(&points);
        let wave = Wave::of(points.iter().map(|point| (point.yaw, point.angle)));
        println!(
            "{shift_ms:>10.0} {:>6} {:>9.3} {:>9.3} {:>9.3}",
            points.len(),
            tilt,
            wave.as_ref().map_or(f64::NAN, |w| w.amplitude),
            wave.as_ref().map_or(f64::NAN, |w| w.residual),
        );
        sweep.row(&[
            format!("{shift_ms:.0}"),
            points.len().to_string(),
            format!("{tilt:.4}"),
            format!("{:.4}", wave.as_ref().map_or(f64::NAN, |w| w.amplitude)),
        ])?;
        if points.len() >= 4 && best.is_none_or(|(held, _)| tilt < held) {
            best = Some((tilt, shift_ms));
        }
    }
    match best {
        Some((tilt, shift_ms)) => println!(
            "\nthe tilt is smallest at a shift of {shift_ms:.0} ms, where it reads {tilt:.2} deg"
        ),
        None => println!("\nno shift found a horizon in enough views to compare"),
    }
    println!("csv:    {}", sweep.path.display());
    Ok(())
}

/// The timing chain, with no pixels: what the trailer says, and whether the
/// IMU's own timeline is regular over the opening.
fn clock_chain(calibration: &CalibrationSet, options: &Options) -> Fallible<()> {
    let imu = options.skewed(&calibration.imu);
    let samples = imu.samples();
    let exposure = &calibration.exposure[0];
    let first = samples.first().ok_or("no IMU samples")?;
    let last = samples.last().ok_or("no IMU samples")?;

    println!(
        "camera: {} {}",
        calibration.camera_model, calibration.firmware
    );
    println!("encode: {:?}", calibration.gyro.encoding);
    println!(
        "clock:  first_frame_timestamp {} ticks, gyro_timestamp {:?} ms, so the track is shifted \
         {} us against the video",
        calibration.gyro.first_frame_timestamp,
        calibration.gyro.gyro_timestamp,
        (calibration.gyro.gyro_timestamp.unwrap_or(0.0) * 1_000.0) as i64,
    );
    if options.skew_us != 0 {
        println!(
            "skew:   {} us injected into every IMU timestamp",
            options.skew_us
        );
    }
    println!(
        "imu:    {} samples at {:.2} Hz, {:.3} s to {:.3} s",
        samples.len(),
        imu.rate_hz(),
        first.offset_us as f64 * 1e-6,
        last.offset_us as f64 * 1e-6,
    );
    println!(
        "frames: exposure record has {} samples, frame 0 at {:?} us, frame 1 at {:?} us",
        exposure.samples().len(),
        exposure.frame_time_us(0),
        exposure.frame_time_us(1),
    );
    println!(
        "cover:  the IMU starts {:.3} s before frame 0 and the video's first frame is inside it: {}",
        -(first.offset_us as f64) * 1e-6,
        first.offset_us <= exposure.frame_time_us(0).unwrap_or(0),
    );

    println!(
        "\n{:>8} {:>9} {:>11} {:>11} {:>11} {:>10}",
        "second", "samples", "rate hz", "mean us", "worst gap", "frame us"
    );
    let mut table = Csv::open(options, "clock", CLOCK_HEADER)?;
    for second in 0..options.to as i64 {
        let from = samples.partition_point(|s| s.offset_us < second * 1_000_000);
        let to = samples.partition_point(|s| s.offset_us < (second + 1) * 1_000_000);
        let window = &samples[from..to];
        if window.len() < 2 {
            continue;
        }
        let gaps: Vec<i64> = window
            .windows(2)
            .map(|pair| pair[1].offset_us - pair[0].offset_us)
            .collect();
        let mean = gaps.iter().sum::<i64>() as f64 / gaps.len() as f64;
        let worst = gaps.iter().copied().max().unwrap_or(0);
        // The frame nearest this second, and how far the nearest IMU sample
        // sits from it. Bounded by half a sample interval on a healthy file,
        // whatever the true offset between the two clocks is: a constant skew
        // is not observable from inside the file.
        let frame = (second as f64 * 30_000.0 / 1_001.0).round() as u64;
        let frame_us = exposure.frame_time_us(frame).unwrap_or(0);
        let near = samples.partition_point(|s| s.offset_us < frame_us);
        let residual = samples
            .get(near.saturating_sub(1)..(near + 1).min(samples.len()))
            .unwrap_or_default()
            .iter()
            .map(|s| s.offset_us - frame_us)
            .min_by_key(|off| off.abs())
            .unwrap_or(0);
        println!(
            "{second:>8} {:>9} {:>11.2} {:>11.1} {worst:>11} {residual:>10}",
            window.len(),
            1e6 / mean,
            mean,
        );
        table.row(&[
            second.to_string(),
            window.len().to_string(),
            format!("{:.3}", 1e6 / mean),
            format!("{mean:.2}"),
            worst.to_string(),
            residual.to_string(),
        ])?;
    }
    println!("csv:    {}", table.path.display());
    println!(
        "\nrate hz is this second's own sample rate, so a clock that changed pace shows as a \n\
         column that is not flat. worst gap is the largest step between two samples in the \n\
         second, which is what a dropped block looks like. frame us is how far the nearest \n\
         IMU sample sits from the camera's own time for the frame at that second, and it can \n\
         only ever be half a sample interval: a constant offset between the two clocks leaves \n\
         no trace in the file and is measured in the picture, with shifts=."
    );
    Ok(())
}

/// The GPU, the target and the pass, which every circle needs and none of the
/// sweeps want to thread by hand.
struct Rig {
    gpu: Gpu,
    pipeline: ScenePipeline,
    target: Offscreen,
    size: Size,
    aspect: f32,
}

impl Rig {
    fn open(size: Size) -> Fallible<Self> {
        let gpu = Gpu::open()?;
        println!("gpu:    {}", gpu.name);
        let pipeline = ScenePipeline::new(&gpu.device, FORMAT);
        let target = Offscreen::new(&gpu.device, size, FORMAT);
        Ok(Self {
            gpu,
            pipeline,
            target,
            size,
            aspect: size.width as f32 / size.height as f32,
        })
    }

    /// One circle of views at one held orientation, and what the horizon in
    /// each of them says. Views with no horizon in them contribute nothing.
    fn circle(&mut self, scene: &Scene, held: Quat, options: &Options) -> Fallible<Vec<Point>> {
        scene.hold_at(Some(held));
        let mut out = Vec::new();
        for turn in 0..options.steps {
            let yaw = turn as f64 * TAU / options.steps as f64;
            let camera = Camera {
                yaw: yaw as f32,
                pitch: options.pitch,
                fov: options.fov,
            };
            self.pipeline.prepare(
                &scene.primitive(camera),
                &self.gpu.device,
                &self.gpu.queue,
                self.aspect,
            );
            self.target
                .render(&self.gpu.device, &self.gpu.queue, &self.pipeline)?;
            let pixels = self.target.read(&self.gpu.device, &self.gpu.queue)?;
            let Some(line) = skyline(&pixels, self.size) else {
                continue;
            };
            // The sky has to be above the line, or what was found is not a
            // horizon this way up.
            if line.sky[1] >= 0.0 {
                continue;
            }
            let look = |uv: [f64; 2]| {
                camera
                    .look(uv.map(|c| c as f32), self.aspect)
                    .expect("the circles are measured in flat views")
                    .map(f64::from)
            };
            let normal = unit(cross(look(line.through[0]), look(line.through[1])));
            let up = match dot(normal, UP) > 0.0 {
                true => normal,
                false => normal.map(Neg::neg),
            };
            out.push(Point {
                yaw,
                angle: line.degrees,
                height: -dot(up, look([0.5, 0.5]))
                    .clamp(-1.0, 1.0)
                    .asin()
                    .to_degrees(),
                up,
            });
        }
        Ok(out)
    }
}

/// One rendered view and what the horizon in it says.
struct Point {
    yaw: f64,
    /// The horizon's angle in the picture, degrees.
    angle: f64,
    /// How far below the middle of the view it sits, in degrees.
    height: f64,
    /// Where the picture says up is, in the frame the camera reads its rays
    /// in, which with the lock on is the stabilized world.
    up: [f64; 3],
}

/// What the IMU was doing at one instant, read the way the filter reads it.
struct Sensed {
    /// The smoothed specific force, in g.
    magnitude_g: f64,
    /// How much of that reading the running filter believes.
    trust: f64,
    /// How far the smoothed accelerometer is from the estimate's own
    /// vertical, in degrees, and which way it leans. This is the correction
    /// the filter is applying, and its bearing is where a leaning estimate
    /// would end up.
    lean_deg: f64,
    lean_toward_deg: f64,
    /// The body's rate about the estimate's vertical, in degrees a second,
    /// which is the rate a coordinated turn's centripetal force comes from.
    turn_dps: f64,
    /// How fast the body is turning about any axis.
    rate_dps: f64,
}

impl Sensed {
    /// Smoothed over the filter's own `accel_seconds`, because that is the
    /// signal the filter forms its opinion from: one sample at 997 Hz is
    /// mostly engine vibration.
    fn at(
        imu: &GyroTrack,
        track: &OrientationTrack,
        to_body: Mat3,
        at: i64,
        filter: Filter,
    ) -> Self {
        let half = (filter.accel_seconds * 5e5) as i64;
        let samples = imu.samples();
        let from = samples.partition_point(|s| s.offset_us < at - half);
        let to = samples.partition_point(|s| s.offset_us < at + half);
        let window = &samples[from..to.max(from + 1).min(samples.len())];
        let count = window.len().max(1) as f64;
        let mean = |of: fn(&GyroSample) -> [f64; 3]| {
            let sum = window.iter().fold([0.0; 3], |held, sample| {
                let value = of(sample);
                std::array::from_fn(|axis| held[axis] + value[axis])
            });
            to_body.mul_vec(sum.map(|axis| axis / count))
        };
        let accel = mean(|sample| sample.accel_g);
        let rate = mean(|sample| sample.rate_dps);
        let magnitude_g = norm(accel);
        // In the world frame the estimate believes in: up is UP by
        // construction there, so the reading's own lean is read straight off.
        let world = track.at(at).rotate(unit(accel));
        let turning = track.at(at).rotate(rate);
        Self {
            magnitude_g,
            // The running filter's own window, recomputed here rather than
            // reached for: `Filter::trust` is private and this is the whole
            // of it.
            trust: trust(filter, magnitude_g),
            lean_deg: dot(world, UP).clamp(-1.0, 1.0).acos().to_degrees(),
            lean_toward_deg: world[0].atan2(world[2]).to_degrees(),
            turn_dps: -turning[1],
            rate_dps: norm(rate),
        }
    }
}

/// How much of a reading the shipped filter believes, from how far its
/// magnitude is from 1 g. `Filter::trust`, which is private to the crate.
fn trust(filter: Filter, magnitude_g: f64) -> f64 {
    let (full, none) = filter.trust_g;
    let off = (magnitude_g - 1.0).abs();
    match off < full {
        true => 1.0,
        false => ((none - off) / (none - full)).clamp(0.0, 1.0),
    }
}

/// What one circle came to.
struct Summary {
    seconds: f64,
    views: usize,
    tilt: f64,
    toward: f64,
    /// Where the camera body is pointing inside the stabilized frame the
    /// circle was measured in.
    ///
    /// What tells a lean bolted to the **world** from one bolted to the
    /// **aircraft**: a centripetal force is always lateral to the airframe,
    /// so its bearing sits at a fixed offset from this, and a wrong vertical
    /// stands still while this turns underneath it.
    heading: f64,
    angle: Option<Wave>,
    height: Option<Wave>,
    sensed: Sensed,
}

impl Summary {
    fn of(points: &[Point], sensed: &Sensed, seconds: f64, heading: f64) -> Self {
        let mean = points.iter().fold([0.0; 3], |held, point| {
            std::array::from_fn(|axis| held[axis] + point.up[axis])
        });
        let up = unit(mean);
        Self {
            seconds,
            heading,
            views: points.len(),
            tilt: tilt_of(points),
            toward: match points.is_empty() {
                true => f64::NAN,
                false => up[0].atan2(up[2]).to_degrees(),
            },
            angle: Wave::of(points.iter().map(|point| (point.yaw, point.angle))),
            height: Wave::of(points.iter().map(|point| (point.yaw, point.height))),
            sensed: Sensed { ..*sensed },
        }
    }

    fn row(&self) -> Vec<String> {
        let wave = |wave: &Option<Wave>, of: fn(&Wave) -> f64| wave.as_ref().map_or(f64::NAN, of);
        vec![
            format!("{:.3}", self.seconds),
            self.views.to_string(),
            format!("{:.4}", self.tilt),
            format!("{:.2}", self.toward),
            format!("{:.2}", self.heading),
            format!("{:.2}", wrap(self.toward - self.heading)),
            format!("{:.4}", wave(&self.angle, |w| w.amplitude)),
            format!("{:.2}", wave(&self.angle, |w| w.phase.to_degrees())),
            format!("{:.4}", wave(&self.angle, |w| w.residual)),
            format!("{:.4}", wave(&self.height, |w| w.amplitude)),
            format!("{:.4}", self.sensed.magnitude_g),
            format!("{:.4}", self.sensed.trust),
            format!("{:.3}", self.sensed.lean_deg),
            format!("{:.2}", self.sensed.lean_toward_deg),
            format!("{:.3}", self.sensed.turn_dps),
            format!("{:.3}", self.sensed.rate_dps),
        ]
    }
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let wave = |of: fn(&Wave) -> f64| self.angle.as_ref().map_or(f64::NAN, of);
        write!(
            f,
            "{:>8.2} {:>6} {:>9.3} {:>9.0} {:>8.3} {:>9.0} {:>8.3} {:>8.3} {:>9.1} {:>9.1}",
            self.seconds,
            self.views,
            self.tilt,
            self.toward,
            wave(|w| w.amplitude),
            wave(|w| w.phase.to_degrees()),
            wave(|w| w.residual),
            self.sensed.magnitude_g,
            self.sensed.lean_deg,
            self.sensed.turn_dps,
        )
    }
}

/// The angle between where the picture says up is and where the estimate
/// does, averaged over the views that found a horizon, as a mean of
/// directions rather than of angles.
fn tilt_of(points: &[Point]) -> f64 {
    if points.is_empty() {
        return f64::NAN;
    }
    let mean = points.iter().fold([0.0; 3], |held, point| {
        std::array::from_fn(|axis| held[axis] + point.up[axis])
    });
    dot(unit(mean), UP).clamp(-1.0, 1.0).acos().to_degrees()
}

/// A least-squares `offset + amplitude * cos(yaw - phase)` through the
/// points, which is the one shape a constant tilt in the vertical can draw.
///
/// The same fit `dip` runs, and deliberately the same: two instruments that
/// disagree about the amplitude of the same circle would be an argument about
/// the fit rather than about the footage.
struct Wave {
    amplitude: f64,
    phase: f64,
    residual: f64,
}

impl Wave {
    fn of(points: impl Iterator<Item = (f64, f64)> + Clone) -> Option<Self> {
        let count = points.clone().count();
        if count < 8 {
            return None;
        }
        let basis = |yaw: f64| [1.0, yaw.cos(), yaw.sin()];
        let mut normal = [[0.0f64; 4]; 3];
        for (yaw, value) in points.clone() {
            let row = basis(yaw);
            for (index, cell) in normal.iter_mut().enumerate() {
                for (column, term) in row.iter().enumerate() {
                    cell[column] += row[index] * term;
                }
                cell[3] += row[index] * value;
            }
        }
        let solved = solve(normal)?;
        let residual = (points
            .map(|(yaw, value)| {
                let fitted: f64 = basis(yaw)
                    .iter()
                    .zip(&solved)
                    .map(|(term, coefficient)| term * coefficient)
                    .sum();
                (value - fitted).powi(2)
            })
            .sum::<f64>()
            / count as f64)
            .sqrt();
        Some(Self {
            amplitude: solved[1].hypot(solved[2]),
            phase: solved[2].atan2(solved[1]),
            residual,
        })
    }
}

/// Gauss-Jordan on a 3x4 augmented matrix. `None` where the columns are not
/// independent, which here means the circle did not cover enough of itself.
fn solve(mut rows: [[f64; 4]; 3]) -> Option<[f64; 3]> {
    for step in 0..3 {
        let pivot =
            (step..3).max_by(|a, b| rows[*a][step].abs().total_cmp(&rows[*b][step].abs()))?;
        rows.swap(step, pivot);
        if rows[step][step].abs() < 1e-9 {
            return None;
        }
        let scale = rows[step][step];
        for cell in rows[step].iter_mut().skip(step) {
            *cell /= scale;
        }
        let pivoted = rows[step];
        for (row, cells) in rows.iter_mut().enumerate() {
            if row == step {
                continue;
            }
            let factor = cells[step];
            for (cell, above) in cells.iter_mut().zip(&pivoted).skip(step) {
                *cell -= factor * above;
            }
        }
    }
    Some([rows[0][3], rows[1][3], rows[2][3]])
}

const INSTANT_HEADER: &str = "seconds,views,tilt_deg,toward_deg,body_heading_deg,\
                              toward_in_body_deg,angle_amp_deg,angle_phase_deg,angle_rms_deg,\
                              height_amp_deg,accel_g,trust,accel_lean_deg,accel_toward_deg,\
                              turn_dps,rate_dps";
const VIEW_HEADER: &str = "seconds,view_yaw_deg,horizon_angle_deg,horizon_height_deg";
const SHIFT_HEADER: &str = "shift_ms,views,tilt_deg,angle_amp_deg";
const CLOCK_HEADER: &str = "second,samples,rate_hz,mean_interval_us,worst_gap_us,frame_residual_us";

/// A CSV in gitignored `scratch/`, because every number in one of these came
/// off a frame of somebody's real flight.
struct Csv {
    path: PathBuf,
    file: BufWriter<File>,
}

impl Csv {
    fn open(options: &Options, name: &str, header: &str) -> Fallible<Self> {
        let path = PathBuf::from("scratch").join(format!("{}-{name}.csv", options.tag));
        let mut file = BufWriter::new(File::create(&path)?);
        writeln!(file, "{header}")?;
        Ok(Self { path, file })
    }

    fn row(&mut self, cells: &[String]) -> Fallible<()> {
        writeln!(self.file, "{}", cells.join(","))?;
        Ok(())
    }
}

struct Options {
    input: PathBuf,
    from: f64,
    to: f64,
    every: u64,
    steps: usize,
    pitch: f32,
    fov: f32,
    size: Size,
    tag: String,
    /// A known tilt added to every held orientation, about the horizontal
    /// world axis at bearing `about`. The control for a defect in the world.
    inject: f64,
    about: f64,
    /// Where to look the orientation up, against the frame's own time.
    shift_us: i64,
    /// The sweep of those offsets: first, last and step, in milliseconds.
    shifts: Option<[f64; 3]>,
    /// A known offset applied to the IMU track before it is solved, which is
    /// what a wrong clock does. The control for the sweep.
    skew_us: i64,
    /// Metres a second of airspeed to manufacture a coordinated turn's
    /// centripetal force from. The control for the accelerometer path.
    centripetal: f64,
    clock: bool,
}

impl Options {
    fn scene(&self) -> Fallible<Scene> {
        let scene = Scene::still(&self.input, Cue::Time(Duration::from_secs_f64(self.from)))?;
        // An instrument has no stored calibration to read: the app keeps that
        // in its own config, and this is not the app. So the seam is fitted
        // off this file, which is what every instrument did before the
        // calibration moved to the camera (issue #48).
        scene.fit_seam(true);
        scene.set_horizon(Horizon::Locked);
        Ok(scene)
    }

    /// The held orientation with the control's tilt in it, on the left, which
    /// is a rotation of the estimated world rather than of the body.
    fn injected(&self, held: Quat) -> Quat {
        if self.inject == 0.0 {
            return held;
        }
        let (sin, cos) = self.about.sin_cos();
        Quat::from_rotation_vector([cos * self.inject, 0.0, sin * self.inject]).times(held)
    }

    /// The IMU track as the filter will see it: moved against the video by
    /// `skew`, and with `centripetal`'s manufactured turn force added.
    fn contaminated(&self, track: &GyroTrack, to_body: Mat3) -> GyroTrack {
        let skewed = self.skewed(track);
        if self.centripetal == 0.0 {
            return skewed;
        }
        // An aircraft flying at `speed` and turning at `rate` about its own
        // vertical feels `speed * rate` of centripetal acceleration towards
        // the centre of the turn, and an accelerometer measures the specific
        // force holding it there, which points outward. In the body frame
        // that is the lateral axis, and the sensor frame is a rotation away.
        let from_body = to_body.transpose();
        GyroTrack::from_samples(
            skewed
                .samples()
                .iter()
                .map(|sample| {
                    let rate = to_body.mul_vec(sample.rate_dps);
                    let outward = [rate[1].to_radians() * self.centripetal / G, 0.0, 0.0];
                    let sensed = from_body.mul_vec(outward);
                    GyroSample {
                        accel_g: std::array::from_fn(|axis| sample.accel_g[axis] + sensed[axis]),
                        ..*sample
                    }
                })
                .collect(),
        )
    }

    /// The IMU track moved bodily against the video, which is what a wrong
    /// `gyro_timestamp` or a wrong tick does to it.
    fn skewed(&self, track: &GyroTrack) -> GyroTrack {
        if self.skew_us == 0 {
            return track.clone();
        }
        GyroTrack::from_samples(
            track
                .samples()
                .iter()
                .map(|sample| GyroSample {
                    offset_us: sample.offset_us + self.skew_us,
                    ..*sample
                })
                .collect(),
        )
    }

    /// What was injected, said out loud, so a CSV cannot be mistaken for a
    /// measurement of the file as it is.
    fn describe(&self) -> String {
        let mut said = vec![format!(
            "{}x{} at pitch {:.0}, fov {:.0}, {} yaws",
            self.size.width,
            self.size.height,
            self.pitch.to_degrees(),
            self.fov.to_degrees(),
            self.steps,
        )];
        if self.inject != 0.0 {
            said.push(format!(
                "inject {:.1} deg about {:.0}",
                self.inject.to_degrees(),
                self.about.to_degrees()
            ));
        }
        if self.skew_us != 0 {
            said.push(format!("skew {} us", self.skew_us));
        }
        if self.centripetal != 0.0 {
            said.push(format!("centripetal at {:.0} m/s", self.centripetal));
        }
        if self.shift_us != 0 {
            said.push(format!("looked up {} us late", self.shift_us));
        }
        said.join(", ")
    }

    fn parse(mut args: impl Iterator<Item = String>) -> Fallible<Self> {
        let input = PathBuf::from(args.next().ok_or(USAGE)?);
        let mut options = Self {
            input,
            from: 0.0,
            to: 40.0,
            every: 15,
            steps: 24,
            pitch: 0.0,
            fov: 100f32.to_radians(),
            size: Size::new(960, 540),
            tag: "lean".to_owned(),
            inject: 0.0,
            about: 0.0,
            shift_us: 0,
            shifts: None,
            skew_us: 0,
            centripetal: 0.0,
            clock: false,
        };
        for arg in args {
            let (key, value) = arg.split_once('=').ok_or(USAGE)?;
            match key {
                "from" => options.from = value.parse()?,
                "to" => options.to = value.parse()?,
                "every" => options.every = value.parse()?,
                "steps" => options.steps = value.parse()?,
                "pitch" => options.pitch = value.parse::<f32>()?.to_radians(),
                "fov" => options.fov = value.parse::<f32>()?.to_radians(),
                "width" => options.size.width = value.parse()?,
                "height" => options.size.height = value.parse()?,
                "tag" => options.tag = value.to_owned(),
                "inject" => options.inject = value.parse::<f64>()?.to_radians(),
                "about" => options.about = value.parse::<f64>()?.to_radians(),
                "shift" => options.shift_us = (value.parse::<f64>()? * 1_000.0) as i64,
                "shifts" => options.shifts = Some(numbers(value)?),
                "skew" => options.skew_us = (value.parse::<f64>()? * 1_000.0) as i64,
                "centripetal" => options.centripetal = value.parse()?,
                "clock" => options.clock = value.parse::<u32>()? != 0,
                _ => return Err(format!("unknown argument {key}. {USAGE}").into()),
            }
        }
        Ok(options)
    }
}

const USAGE: &str = "usage: lean <file.insv> [from=seconds] [to=seconds] [every=frames] \
     [steps=yaws] [pitch=deg] [fov=deg] [width=px] [height=px] [tag=name] [inject=deg] \
     [about=deg] [shift=ms] [shifts=first_last_step] [skew=ms] [centripetal=m/s] [clock=1]";

/// The underscore-separated numbers of one argument, as the fixed-size array
/// its reader wants.
fn numbers<const N: usize>(value: &str) -> Fallible<[f64; N]> {
    let read: Vec<f64> = value
        .split('_')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| format!("{value} is not {N} numbers separated by underscores"))?;
    read.try_into()
        .map_err(|_| format!("{value} is not {N} numbers").into())
}

/// What the filter started the estimate from, read out of the filter itself.
///
/// The first line of any horizon question about the opening of a file: a seed
/// is applied whole, so a window that weighed near 1 g without pointing at
/// gravity is a tilt every frame after it inherits (issue #45).
fn report_seed(imu: &GyroTrack, to_body: Mat3) {
    let filter = Filter::default();
    let Some(seed) = filter.seed(imu, to_body) else {
        println!("seed:   no IMU samples, so there is nothing to start from");
        return;
    };
    println!(
        "seed:   from {:.2} s in, where {:.1} s of accelerometer weighs {:.3} g, which the \
         filter {}",
        seed.at_us as f64 * 1e-6,
        filter.accel_seconds,
        seed.magnitude_g,
        match seed.trusted {
            true => "believes completely",
            false => "would refuse, and nothing in the search read closer to gravity",
        },
    );
}

/// An angle in degrees wrapped into (-180, 180].
fn wrap(degrees: f64) -> f64 {
    (degrees + 180.0).rem_euclid(360.0) - 180.0
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

fn norm(v: [f64; 3]) -> f64 {
    dot(v, v).sqrt()
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let length = norm(v);
    v.map(|c| c / length.max(f64::MIN_POSITIVE))
}
