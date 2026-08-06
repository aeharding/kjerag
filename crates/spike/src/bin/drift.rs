//! What the world-fixed lock's own heading does over a whole flight, and where
//! the view's zero sits when the first frame arrives.
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin drift -- <file.insv>
//! cargo run --release -p kjerag-spike --bin drift -- <file.insv> rest=1766,1776
//! ```
//!
//! Written for PR #165's review, because the number the change was going to be
//! accepted on was wrong in shape. **The locked frame turns about the world
//! vertical at `bias . up_in_body`, and not at the body's own yaw-axis bias.**
//! A camera hanging under a wing spends the flight tens of degrees off
//! vertical, so its horizontal bias components project onto the world vertical
//! and dominate the term everybody quotes. This walks the solved track and
//! integrates that projection, minute by minute.
//!
//! On VID_20260714_193252_00_006, 1805 s of X4 Air paramotor flight, from the
//! zero over this file's own quietest ten seconds: the running error is -36
//! degrees by minute 3, +87 by minute 8 and +149 by minute 19, about 185
//! degrees peak to peak, while its signed mean is 2.08 deg/min. **A steady
//! creep of two degrees a minute and a swing of that size are the same average
//! and not the same experience**, which is why the average on its own is not
//! the thing to quote. Nor is the body-y term: on the same run it accounts for
//! 7.90 deg/min, in one direction, of an answer that is nothing like one
//! direction.
//!
//! **The shape is robust and the size is not, because this file has no still
//! moment good enough to read a zero from.** `rest=` the ten seconds
//! `--bin gyro` picks instead, 2.7 s later than the ten this one picks, and
//! the same walk swings +64 degrees by minute 3 and -313 by minute 18 and ends
//! at -43: hundreds of degrees either way, and 1.40 deg/min of signed mean
//! against 2.08. Windows a few seconds apart read biases from 0.5 to 4.8 deg/s
//! here, which is a camera being carried and not a gyroscope being read. So
//! **take the wandering as the finding and the degrees as an order of
//! magnitude**, and read a real number off a capture that is actually still.
//!
//! `rest=from,to` names the seconds to read the zero over; the default is the
//! quietest ten seconds, the same rule `--bin gyro` uses. What this cannot do
//! is measure the drift: it integrates one constant zero through the file's own
//! attitude history, so it is a model of the drift and it is only ever as good
//! as that zero. The measurement of the real thing is a picture against a
//! reference, which is `wide.py` on `research/oracle-probe`.

use std::path::PathBuf;

use kjerag_media::Fallible;
use kjerag_meta::{CalibrationSet, Filter, GyroSample};

/// Which way the world's own up points in the world frame: y is down.
const UP_IN_WORLD: [f64; 3] = [0.0, -1.0, 0.0];

/// How long a window the zero is read over, in seconds.
const REST_S: f64 = 10.0;

fn main() -> Fallible<()> {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().ok_or(USAGE)?);
    let rest = match args.next() {
        Some(argument) => Some(window(&argument)?),
        None => None,
    };

    let calibration = CalibrationSet::from_insv(&path)?;
    let samples = calibration.imu.samples().to_vec();
    let to_body = calibration.body_from_imu();
    let still = match rest {
        Some((from, to)) => held(&samples, from, to),
        None => quietest(&samples),
    };
    let bias = to_body.mul_vec(mean(still));
    println!(
        "still:  {:.1} to {:.1} s, {} samples",
        seconds(still.first()),
        seconds(still.last()),
        still.len(),
    );
    println!(
        "bias:   body [{:+.4}, {:+.4}, {:+.4}] deg/s: {:.4} about the body's own \
         vertical and {:.4} across it, and it is the second that reaches the view \
         whenever the camera is not level",
        bias[0],
        bias[1],
        bias[2],
        bias[1].abs(),
        bias[0].hypot(bias[2]),
    );

    let solved = calibration.orientation(Filter::default());
    let Some(first) = solved.samples().first() else {
        return Err("no IMU record in this file".into());
    };
    if let Some(frame_us) = calibration.exposure[0].frame_time_us(0) {
        println!(
            "zero:   the view's yaw zero is the heading at the first IMU sample, {:.2} s \
             before the first video frame, and the body turned {:.2} deg in between: \
             that is how far off the nose the default view opens",
            (frame_us - first.offset_us) as f64 * 1e-6,
            wrap(solved.at(frame_us).heading() - first.world_from_body.heading()).to_degrees(),
        );
    }

    println!(
        "\n{:>8}{:>12}{:>14}{:>13}{:>11}",
        "minute", "drift deg", "if level deg", "unsigned", "tilt deg"
    );
    let mut drift = Turned::default();
    let mut previous = first.offset_us;
    let mut due = 60.0;
    for sample in solved.samples() {
        let dt = (sample.offset_us - previous).max(0) as f64 * 1e-6;
        previous = sample.offset_us;
        drift.add(
            bias,
            sample.world_from_body.conjugate().rotate(UP_IN_WORLD),
            dt,
        );
        if sample.offset_us as f64 * 1e-6 >= due {
            println!(
                "{due:8.0}{:12.1}{:14.1}{:13.1}{:11.2}",
                drift.signed, drift.level, drift.unsigned, drift.tilt
            );
            due += 60.0;
        }
    }

    let span = (previous - first.offset_us) as f64 * 1e-6;
    let rate = |degrees: f64| 60.0 * degrees / span.max(1e-9);
    println!(
        "\ntotal:  {span:.1} s. The locked frame's heading ends {:.1} deg from where it \
         started ({:.2} deg/min), having gone {:.1} deg to get there ({:.2} deg/min \
         unsigned); the body-y term on its own would be {:.1} deg ({:.2} deg/min). \
         Worst tilt {:.1} deg.\n\nThe signed column is what the pilot sees: read its \
         swing and not its mean, because a frame that wanders out and back has the \
         average of one that never moved.",
        drift.signed,
        rate(drift.signed),
        drift.unsigned,
        rate(drift.unsigned),
        drift.level,
        rate(drift.level),
        drift.worst_tilt,
    );
    Ok(())
}

const USAGE: &str = "usage: drift <file.insv> [rest=from_seconds,to_seconds]";

/// How far the locked frame has turned about the world vertical, three ways.
#[derive(Default)]
struct Turned {
    /// The drift itself: where the frame is now against where it started.
    signed: f64,
    /// The same integration with the camera held level, which is the term a
    /// yaw-axis bias figure describes and the reason it is not enough.
    level: f64,
    /// Distance travelled rather than displacement, which says whether a small
    /// signed answer is a frame that held or one that came back.
    unsigned: f64,
    /// How far the body is off the world vertical at this sample, and the
    /// worst of that so far: the lever the horizontal bias comes in on.
    tilt: f64,
    worst_tilt: f64,
}

impl Turned {
    fn add(&mut self, bias: [f64; 3], up_in_body: [f64; 3], dt: f64) {
        // `up_in_body` points where the world's up is, which is -y here, so the
        // component of the bias about the world vertical is its negative.
        let rate = -(bias[0] * up_in_body[0] + bias[1] * up_in_body[1] + bias[2] * up_in_body[2]);
        self.signed += rate * dt;
        self.level += bias[1] * dt;
        self.unsigned += rate.abs() * dt;
        let off = up_in_body[1].clamp(-1.0, 1.0).acos().to_degrees();
        self.tilt = off.min(180.0 - off);
        self.worst_tilt = self.worst_tilt.max(self.tilt);
    }
}

fn window(argument: &str) -> Fallible<(f64, f64)> {
    let (from, to) = argument
        .strip_prefix("rest=")
        .and_then(|value| value.split_once(','))
        .ok_or(USAGE)?;
    Ok((from.parse()?, to.parse()?))
}

fn seconds(sample: Option<&GyroSample>) -> f64 {
    sample.map_or(0.0, |sample| sample.offset_us as f64 * 1e-6)
}

fn mean(samples: &[GyroSample]) -> [f64; 3] {
    let mut sum = [0.0; 3];
    for sample in samples {
        for (total, rate) in sum.iter_mut().zip(sample.rate_dps) {
            *total += rate;
        }
    }
    sum.map(|axis| axis / samples.len().max(1) as f64)
}

fn held(samples: &[GyroSample], from: f64, to: f64) -> &[GyroSample] {
    let inside = |sample: &GyroSample| {
        let at = sample.offset_us as f64 * 1e-6;
        at >= from && at <= to
    };
    let first = samples.iter().position(inside).unwrap_or(0);
    let last = samples.iter().rposition(inside).unwrap_or(first);
    &samples[first..=last]
}

/// The quietest [`REST_S`], the same rule `--bin gyro` reads a zero over.
fn quietest(samples: &[GyroSample]) -> &[GyroSample] {
    let span = (samples.last().map_or(0, |s| s.offset_us)
        - samples.first().map_or(0, |s| s.offset_us))
    .max(1);
    let width = (REST_S * samples.len() as f64 * 1e6 / span as f64) as usize;
    if samples.len() < width * 2 || width == 0 {
        return samples;
    }
    let stir = |window: &[GyroSample]| {
        window
            .iter()
            .map(|s| s.rate_dps[0].abs() + s.rate_dps[1].abs() + s.rate_dps[2].abs())
            .sum::<f64>()
    };
    let mut best = 0;
    let mut quiet = f64::MAX;
    let mut at = 0;
    while at + width < samples.len() {
        let now = stir(&samples[at..at + width]);
        if now < quiet {
            quiet = now;
            best = at;
        }
        at += width / 4;
    }
    &samples[best..best + width]
}

/// An angle wrapped into (-pi, pi], so a heading crossing the back of the
/// compass is a small change and not a whole turn.
fn wrap(angle: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    (angle + PI).rem_euclid(TAU) - PI
}
