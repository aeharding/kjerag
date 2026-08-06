//! What the IMU track of a real capture actually says.
//!
//! The instrument the filter's constants were chosen with, and the one that
//! answers the two questions no synthetic test can: which sensor axis is up,
//! and which clock the frames are on.
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin gyro -- <file.insv>
//! cargo run --release -p kjerag-spike --bin gyro -- <file.insv> rest=0,20
//! ```
//!
//! `rest=<from>,<to>` names a stretch in seconds the camera was not moving
//! for, which is what the bias and the gravity check are measured over. It
//! defaults to the first twenty seconds, which on a paramotor capture is
//! usually the camera sitting on the ground or being carried.
//!
//! Nothing it prints is a frame of anybody's video, so its output is safe to
//! quote in a doc. It does print the file's own record sizes, which are not
//! personal.

use std::path::PathBuf;

use kjerag_media::{Fallible, Reader};
use kjerag_meta::{
    CalibrationSet, Filter, GyroSample, OrientationTrack, axis_map, body_from_imu, record_index,
};

/// Axis conventions worth printing side by side: the one Kjerag's table
/// picks, and three that published tables name. Only one of them can put
/// gravity on the body's own vertical, and `horizon --sweep` is what tries
/// all 24 rather than these four.
const CANDIDATES: [&str; 4] = ["xZY", "yzX", "Xyz", "xZy"];

fn main() -> Fallible<()> {
    let (path, rest) = parse(std::env::args().skip(1))?;
    let calibration = CalibrationSet::from_insv(&path)?;

    println!("file:   {}", path.display());
    println!(
        "camera: {} {}, key {:016x}",
        calibration.camera_model,
        calibration.firmware,
        calibration.camera_key(),
    );
    print!("records:");
    for (id, format, size) in record_index(&path)? {
        print!(" {id}/{format}={size}");
    }
    println!();

    let imu = &calibration.imu;
    println!(
        "imu:    {} samples, {:.1} Hz, {:?}, {:.1} s to {:.1} s",
        imu.samples().len(),
        imu.rate_hz(),
        calibration.gyro.encoding,
        imu.samples().first().map_or(0.0, seconds),
        imu.samples().last().map_or(0.0, seconds),
    );
    if imu.is_empty() {
        return Err("this file carries no IMU record".into());
    }

    let still = match rest {
        Some(rest) => {
            println!("rest:   {:.0} to {:.0} s, as asked for", rest.0, rest.1);
            window(imu.samples(), rest)
        }
        None => quietest(imu.samples()),
    };
    gravity(still, &calibration);
    bias(still, &calibration);
    tracking(&calibration);
    motion(imu.samples(), &calibration);
    clocks(&path, &calibration)?;
    Ok(())
}

/// Is the filter actually level? The one check that separates a wrong axis
/// convention from a filter that is not converging: where the solved
/// orientation says up is, against where the accelerometer says it is, in the
/// body's own frame.
fn tracking(calibration: &CalibrationSet) {
    let to_body = calibration.body_from_imu();
    let samples = calibration.imu.samples();
    let smoothed = smooth(samples, 1.0);
    for filter in [
        Filter::default(),
        Filter {
            tilt_seconds: 0.05,
            ..Filter::default()
        },
    ] {
        let solved = calibration.orientation(filter);
        let mut worst: f64 = 0.0;
        let mut total = 0.0;
        let mut count = 0.0;
        for (step, accel) in smoothed.iter().enumerate() {
            // `smooth` keeps one in a hundred samples.
            let at = samples[step * 100].offset_us;
            let body_up = solved.at(at).conjugate().rotate([0.0, -1.0, 0.0]);
            let off = dot(unit(body_up), unit(to_body.mul_vec(*accel)))
                .clamp(-1.0, 1.0)
                .acos()
                .to_degrees();
            worst = worst.max(off);
            total += off;
            count += 1.0;
        }
        println!(
            "level:  tilt {:>5} s: the filter's up is {:5.2} deg from the accelerometer's on \
             average, {:6.2} worst",
            filter.tilt_seconds,
            total / count,
            worst,
        );
    }
}

/// The quietest ten seconds in the file, which is where the gyroscope's zero
/// and the accelerometer's agreement with gravity can be read.
///
/// Found rather than asked for, because "the camera was not moving" is not a
/// time anybody knows off a paramotor capture: the first twenty seconds of
/// this footage is the camera being carried to the launch, and it reads
/// 2 deg/s of rate that is not bias at all.
fn quietest(samples: &[GyroSample]) -> &[GyroSample] {
    const WINDOW_S: f64 = 10.0;
    let rate = samples.len() as f64 * 1e6
        / (samples.last().unwrap().offset_us - samples[0].offset_us).max(1) as f64;
    let width = (WINDOW_S * rate) as usize;
    if samples.len() < width * 2 {
        return samples;
    }
    let stir = |window: &[GyroSample]| {
        window
            .iter()
            .step_by(11)
            .map(|sample| length(sample.rate_dps))
            .sum::<f64>()
    };
    let (at, _) = samples
        .chunks_exact(width)
        .enumerate()
        .min_by(|a, b| stir(a.1).total_cmp(&stir(b.1)))
        .expect("at least one window");
    let found = &samples[at * width..(at + 1) * width];
    println!(
        "rest:   quietest {WINDOW_S:.0} s is {:.1} to {:.1} s, {} samples",
        seconds(&found[0]),
        seconds(found.last().unwrap()),
        found.len(),
    );
    found
}

fn seconds(sample: &GyroSample) -> f64 {
    sample.offset_us as f64 * 1e-6
}

type Seconds = (f64, f64);

fn parse(mut args: impl Iterator<Item = String>) -> Fallible<(PathBuf, Option<Seconds>)> {
    const USAGE: &str = "usage: gyro <file.insv> [rest=<from>,<to>]";
    let path = PathBuf::from(args.next().ok_or(USAGE)?);
    let mut rest = None;
    for arg in args {
        let (key, value) = arg.split_once('=').ok_or(USAGE)?;
        let (from, to) = value.split_once(',').ok_or(USAGE)?;
        match key {
            "rest" => rest = Some((from.parse()?, to.parse()?)),
            _ => return Err(format!("unknown argument {key}. {USAGE}").into()),
        }
    }
    Ok((path, rest))
}

fn window(samples: &[GyroSample], (from, to): (f64, f64)) -> &[GyroSample] {
    let at =
        |seconds: f64| samples.partition_point(|sample| sample.offset_us < (seconds * 1e6) as i64);
    let (start, end) = (at(from), at(to).min(samples.len()));
    match start < end {
        true => &samples[start..end],
        false => &samples[..0],
    }
}

fn mean(samples: &[GyroSample], of: impl Fn(&GyroSample) -> [f64; 3]) -> [f64; 3] {
    let count = samples.len().max(1) as f64;
    samples.iter().fold([0.0; 3], |held, sample| {
        let value = of(sample);
        std::array::from_fn(|axis| held[axis] + value[axis] / count)
    })
}

fn length(v: [f64; 3]) -> f64 {
    v.iter().map(|c| c * c).sum::<f64>().sqrt()
}

/// Which way is up, in the sensor's axes and then in the body's.
///
/// This is the check that settles the axis convention, and it is physics
/// rather than a reading of somebody's table: a camera that is not moving
/// measures 1 g, pointing up, and up in the body frame is -y.
fn gravity(still: &[GyroSample], calibration: &CalibrationSet) {
    let quiet = mean(still, |sample| sample.accel_g);
    let whole = mean(calibration.imu.samples(), |sample| sample.accel_g);
    println!(
        "gravity: sensor axes, at rest {quiet:.4?} = {:.4} g, whole file {whole:.4?} = {:.4} g",
        length(quiet),
        length(whole),
    );
    for orientation in CANDIDATES {
        let map = body_from_imu(orientation, &calibration.lenses[0].pose);
        let off = |v: [f64; 3]| {
            let body = map.mul_vec(v);
            (-body[1] / length(body))
                .clamp(-1.0, 1.0)
                .acos()
                .to_degrees()
        };
        println!(
            "         {orientation}: body {:.4?}, {:6.2} deg off the body vertical at rest, \
             {:6.2} deg over the file{}",
            map.mul_vec(quiet),
            off(quiet),
            off(whole),
            match orientation == calibration.gyro.imu_orientation {
                true => "   <- this camera's",
                false => "",
            }
        );
    }
    // The axis map on its own, so the lens extrinsics can be seen to be the
    // small correction they are.
    let bare = axis_map(calibration.gyro.imu_orientation).mul_vec(quiet);
    let full = calibration.body_from_imu().mul_vec(quiet);
    println!(
        "         lens extrinsics move it {:.2} deg",
        dot(unit(bare), unit(full))
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
    );

    // Per axis, once the engine's vibration is smoothed out of it. An axis
    // whose slow part never moves while the other two swing through tens of
    // degrees is not measuring gravity, it is reading an offset, and that is
    // the difference between a mounting and a broken scale.
    let smoothed = smooth(calibration.imu.samples(), 1.0);
    let count = smoothed.len().max(1) as f64;
    let mean: [f64; 3] =
        std::array::from_fn(|axis| smoothed.iter().map(|a| a[axis]).sum::<f64>() / count);
    let sd: [f64; 3] = std::array::from_fn(|axis| {
        (smoothed
            .iter()
            .map(|a| (a[axis] - mean[axis]).powi(2))
            .sum::<f64>()
            / count)
            .sqrt()
    });
    println!("         smoothed sensor axes: mean {mean:.4?}, sd {sd:.4?}");
    let mut lengths: Vec<f64> = smoothed.iter().map(|a| length(*a)).collect();
    lengths.sort_by(f64::total_cmp);
    println!(
        "         smoothed |a|: median {:.4} g, 10th {:.4}, 90th {:.4}",
        lengths[lengths.len() / 2],
        lengths[lengths.len() / 10],
        lengths[lengths.len() * 9 / 10],
    );
}

/// The accelerometer with a first-order low pass on it, decimated to ten a
/// second. At 1 kHz the raw signal is mostly engine vibration.
fn smooth(samples: &[GyroSample], seconds: f64) -> Vec<[f64; 3]> {
    let mut held = samples[0].accel_g;
    let mut previous = samples[0].offset_us;
    let mut out = Vec::new();
    for (index, sample) in samples.iter().enumerate() {
        let dt = (sample.offset_us - previous).max(0) as f64 * 1e-6;
        previous = sample.offset_us;
        let follow = dt / (seconds + dt);
        held =
            std::array::from_fn(|axis| held[axis] + (sample.accel_g[axis] - held[axis]) * follow);
        if index % 100 == 0 {
            out.push(held);
        }
    }
    out
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let length = length(v);
    v.map(|c| c / length.max(f64::MIN_POSITIVE))
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

/// The gyroscope's zero, which is what decides how long `tilt_seconds` may
/// be: a complementary filter settles at `tilt_seconds * bias`.
fn bias(still: &[GyroSample], calibration: &CalibrationSet) {
    let raw = mean(still, |sample| sample.rate_dps);
    let body = calibration.body_from_imu().mul_vec(raw);
    println!(
        "bias:   body {body:.4?} deg/s, magnitude {:.4} deg/s",
        length(body)
    );
    for seconds in [10.0, 20.0, 40.0] {
        println!(
            "         a {seconds:.0} s tilt constant settles at {:.2} deg of tilt",
            seconds * length(body)
        );
    }
}

/// What the flight looks like to the filter: how far the accelerometer
/// wanders from 1 g, how fast the airframe turns, and how often it changes
/// its mind about which way.
fn motion(samples: &[GyroSample], calibration: &CalibrationSet) {
    let to_body = calibration.body_from_imu();
    let mut magnitudes: Vec<f64> = samples
        .iter()
        .map(|sample| length(sample.accel_g))
        .collect();
    magnitudes.sort_by(f64::total_cmp);
    let at = |fraction: f64| magnitudes[((magnitudes.len() - 1) as f64 * fraction) as usize];
    println!(
        "accel:  |a| median {:.3} g, 10th {:.3}, 90th {:.3}, 99th {:.3} g",
        at(0.5),
        at(0.1),
        at(0.9),
        at(0.99)
    );
    let believed = magnitudes
        .iter()
        .filter(|m| (*m - 1.0).abs() < Filter::default().trust_g.1)
        .count();
    println!(
        "         {:.1}% of samples fall inside the default trust window",
        100.0 * believed as f64 / magnitudes.len() as f64
    );

    let mut worst = [0.0f64; 3];
    let mut squared = [0.0f64; 3];
    let mut crossings = [0usize; 3];
    let mut held = [0.0f64; 3];
    for sample in samples {
        let rate = to_body.mul_vec(sample.rate_dps);
        for axis in 0..3 {
            worst[axis] = worst[axis].max(rate[axis].abs());
            squared[axis] += rate[axis] * rate[axis] / samples.len() as f64;
            if held[axis] * rate[axis] < 0.0 {
                crossings[axis] += 1;
            }
            held[axis] = rate[axis];
        }
    }
    let span = (samples.last().unwrap().offset_us - samples[0].offset_us) as f64 * 1e-6;
    let name = ["pitch (x)", "yaw   (y)", "roll  (z)"];
    for axis in 0..3 {
        println!(
            "rate:   {} rms {:6.1} deg/s, peak {:6.1} deg/s, mean period {:.2} s",
            name[axis],
            squared[axis].sqrt(),
            worst[axis],
            2.0 * span / crossings[axis].max(1) as f64,
        );
    }

    // What a heading follow would do to this flight, priced against the
    // shipped lock. The reference is the fully locked solve, which is what
    // ships: what reaches the picture at a finite constant is the heading the
    // lock holds and that solve does not. Every row here is a design that was
    // rejected on 2026-08-06, and the table is why.
    let locked = calibration.orientation(Filter {
        yaw_seconds: f64::INFINITY,
        ..Filter::default()
    });
    for yaw_seconds in [0.0, 1.0, 2.0, 3.0, 5.0, 10.0] {
        let solved = calibration.orientation(Filter {
            yaw_seconds,
            ..Filter::default()
        });
        let (fast, slow) = view_heading(&locked, &solved);
        println!(
            "yaw:    {yaw_seconds:>4} s constant: the view swings {fast:5.1} deg in a second, \
             and follows {slow:6.1} deg of turn in a minute"
        );
    }
}

/// How the view's own heading moves at one filter setting: the worst it
/// swings inside a second, and the most real turning it follows in a minute.
///
/// The same measurement over two window lengths. The first was read as the
/// artifact to remove and the second as the motion to keep, which is the
/// trade the shipped lock stopped making: both are now motion the picture
/// does not take.
fn view_heading(locked: &OrientationTrack, solved: &OrientationTrack) -> (f64, f64) {
    let turned: Vec<f64> = locked
        .samples()
        .iter()
        .zip(solved.samples())
        .step_by(100)
        .map(|(all, left)| {
            wrap(all.world_from_body.heading() - left.world_from_body.heading()).to_degrees()
        })
        .collect();
    // The samples above are a tenth of a second apart, so a second is ten of
    // them and a minute is six hundred.
    let unwrapped = unwrap(&turned);
    (worst_over(&unwrapped, 10), worst_over(&unwrapped, 600))
}

/// A run of angles with the jumps taken out, so that a difference across a
/// window is a turn and not a wrap.
fn unwrap(angles: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(angles.len());
    let mut carried = 0.0;
    for (index, angle) in angles.iter().enumerate() {
        if index > 0 {
            carried += wrap((angle - angles[index - 1]).to_radians()).to_degrees();
        }
        out.push(carried);
    }
    out
}

fn worst_over(angles: &[f64], window: usize) -> f64 {
    angles
        .windows(window.max(2))
        .map(|run| {
            run.iter().fold(f64::MIN, |a, b| a.max(*b))
                - run.iter().fold(f64::MAX, |a, b| a.min(*b))
        })
        .fold(0.0f64, f64::max)
}

/// An angle wrapped into (-pi, pi].
fn wrap(angle: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    (angle + PI).rem_euclid(TAU) - PI
}

/// The two candidate frame clocks, side by side.
///
/// `pts_type = 2` says the exposure records are the authoritative frame
/// timestamps, and the container's own PTS is what the engine paces on
/// today. If they disagree, the gyro is aligned to the wrong one and the
/// horizon swims; if they agree, the hint costs nothing and this is what
/// says so.
fn clocks(path: &std::path::Path, calibration: &CalibrationSet) -> Fallible<()> {
    let reader = Reader::open(path)?;
    let timing = reader.timing();
    let track = &calibration.exposure[0];
    if track.is_empty() {
        println!("clocks: no exposure record, so container PTS is the only clock");
        return Ok(());
    }

    let samples = track.samples();
    // Frame 0 is the sample nearest media time zero: the camera writes a few
    // before it commits the first frame.
    let zero = samples
        .iter()
        .enumerate()
        .min_by_key(|(_, sample)| sample.offset_us.abs())
        .map(|(index, _)| index)
        .unwrap_or(0);
    println!(
        "clocks: {} exposure samples for {} frames, frame 0 is sample {zero}",
        samples.len(),
        timing.frames
    );

    let mut worst = 0.0f64;
    for fraction in [0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 0.999] {
        let frame = (timing.frames as f64 * fraction) as u64;
        let Some(sample) = samples.get(zero + frame as usize) else {
            continue;
        };
        let pts_us = timing.time_of(frame).as_micros() as i64;
        let apart = sample.offset_us - pts_us;
        worst = worst.max(apart.abs() as f64);
        println!(
            "         frame {frame:>6}: pts {:>10.3} s, exposure {:>10.3} s, apart {:>8.3} ms \
             ({:.4} frames)",
            pts_us as f64 * 1e-6,
            sample.offset_us as f64 * 1e-6,
            apart as f64 * 1e-3,
            apart as f64 / timing.interval().as_micros() as f64,
        );
    }
    println!(
        "         worst disagreement {:.3} ms, {:.4} of a frame",
        worst * 1e-3,
        worst / timing.interval().as_micros() as f64
    );

    // What the disagreement is worth where it lands, which is the only form
    // of it that matters: two clocks a few milliseconds apart cost the
    // horizon the camera's own rotation over those milliseconds, so the same
    // gap is nothing while the camera is still and degrees while it rolls.
    let solved = calibration.orientation(Filter::default());
    let mut worst = (0.0f64, 0u64);
    let mut total = 0.0f64;
    let mut count = 0.0f64;
    for frame in (0..timing.frames).step_by(97) {
        let Some(exposure) = track.frame_time_us(frame) else {
            continue;
        };
        let container = timing.time_of(frame).as_micros() as i64;
        let apart = solved
            .at(exposure)
            .angle_to(solved.at(container))
            .to_degrees();
        if apart > worst.0 {
            worst = (apart, frame);
        }
        total += apart;
        count += 1.0;
    }
    println!(
        "         the two clocks put the camera {:.3} deg apart on average, {:.2} deg worst \
         (frame {}, {:.1} s)",
        total / count.max(1.0),
        worst.0,
        worst.1,
        timing.time_of(worst.1).as_secs_f64(),
    );
    Ok(())
}
