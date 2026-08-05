//! Where the estimate should have started, judged with the benefit of the
//! whole file: how far a seed is from an answer that used minutes of the
//! flight instead of the opening of it (issue #152).
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin hindsight -- <file.insv>
//! cargo run --release -p kjerag-spike --bin hindsight -- <file.insv> spans=60_120
//! ```
//!
//! The arbiter is a **backward pass**: the same complementary filter run from
//! a couple of minutes in back to the first sample, with the gyroscope's rates
//! negated. It meets a launch last instead of first, it is corrected by every
//! trusted reading in between, and whatever it started from has decayed at
//! `tilt_seconds` long before it arrives, so its answer at the first sample
//! owes nothing to any seed rule. **Read the `back moved` column first**: an
//! answer that moves when its span is doubled is not an answer. On the six
//! flights this was written for it moves 0.02 to 0.83 degrees between 120 and
//! 240 seconds, and it puts the seed of issue #45's fix 24.18 degrees off on
//! the file the owner reported, against the 20.944 `lean` measured in the
//! picture. A probe that has never been shown to fire proves nothing when it
//! stays quiet, and that is this one firing.
//!
//! No GPU and no pixels, which is what makes it the cheap search that says
//! which rule is worth rendering. It carries the rule this branch replaced and
//! the ones that were considered instead, each measured against the same
//! arbiter, because the argument for a seed is a table and not a sentence.

use kjerag_media::Fallible;
use kjerag_meta::{CalibrationSet, Filter, GyroSample, GyroTrack, Mat3, Quat};
use std::path::PathBuf;

/// Where the estimate says up is, in the world frame: y is down.
const UP: [f64; 3] = [0.0, -1.0, 0.0];

fn main() -> Fallible<()> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(
        args.next()
            .ok_or("usage: hindsight <file.insv> [spans=first_second_third]")?,
    );
    let mut spans = vec![60.0, 120.0, 240.0];
    for arg in args {
        let (key, value) = arg.split_once('=').ok_or("key=value")?;
        match key {
            "spans" => {
                spans = value
                    .split('_')
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|_| "spans wants numbers separated by underscores")?
            }
            _ => return Err(format!("unknown {key}").into()),
        }
    }
    let calibration = CalibrationSet::from_insv(&input)?;
    let to_body = calibration.body_from_imu();
    let filter = Filter::default();
    let imu = &calibration.imu;
    println!(
        "file:   {}\nimu:    {} samples at {:.0} Hz",
        input.display(),
        imu.samples().len(),
        imu.rate_hz(),
    );

    // The rule issue #152 replaced, kept here so both can be measured against
    // the same arbiter in one run: the first second of accelerometer the
    // filter believes completely, else the closest to 1 g inside 20 s.
    let old = first_trusted(&filter, imu, to_body).ok_or("no IMU samples")?;
    let new = filter.seed(imu, to_body).ok_or("no IMU samples")?;
    let up_of = |q: Quat| q.conjugate().rotate(UP);
    println!(
        "before: the replaced rule seeds at {:>7.2} s, {:.3} g, trusted {}",
        old.1 as f64 * 1e-6,
        norm(old.2),
        filter_trusts(&filter, norm(old.2)),
    );
    println!(
        "seed:   the shipped rule seeds at {:>7.2} s, {:.3} g, trusted {}",
        new.at_us as f64 * 1e-6,
        new.magnitude_g,
        new.trusted,
    );
    println!(
        "apart:  the two seeds disagree by {:.2} degrees",
        angle(up_of(old.0), up_of(new.world_from_body))
    );

    println!(
        "\n{:>8} {:>12} {:>12} {:>12}",
        "span s", "old vs back", "new vs back", "back moved"
    );
    let mut held: Option<[f64; 3]> = None;
    for span in &spans {
        let Some(back) = backward(&filter, imu, to_body, *span) else {
            continue;
        };
        println!(
            "{span:>8.0} {:>12.2} {:>12.2} {:>12.2}",
            angle(up_of(old.0), back),
            angle(up_of(new.world_from_body), back),
            held.map_or(f64::NAN, |held| angle(held, back)),
        );
        held = Some(back);
    }
    println!(
        "\nvs back is how far that seed's idea of up is from the backward pass's, in degrees, \n\
         at the first sample of the track. back moved is how far the arbiter itself moved when \n\
         the span grew, which is the only measure of how much it is worth."
    );

    // How much of what is left at the first frame is the seed, and how much
    // is the running correction leaning into whatever happened before that
    // frame: the same solve with the correction effectively off differs from
    // the shipped one by exactly what the correction did.
    let seeded_only = Filter {
        tilt_seconds: 1e5,
        ..filter
    };
    let shipped = filter.solve(imu, to_body);
    let frozen = seeded_only.solve(imu, to_body);
    let up_in_body =
        |track: &kjerag_meta::OrientationTrack, at: i64| track.at(at).conjugate().rotate(UP);
    println!(
        "\npre-roll: the IMU starts {:.2} s before the first frame, and the correction moves \n\
         \tthe estimate {:.2} degrees over that stretch, {:.2} by 10 s and {:.2} by 40 s",
        -(imu.samples().first().map_or(0, |s| s.offset_us) as f64) * 1e-6,
        angle(up_in_body(&shipped, 0), up_in_body(&frozen, 0)),
        angle(
            up_in_body(&shipped, 10_000_000),
            up_in_body(&frozen, 10_000_000)
        ),
        angle(
            up_in_body(&shipped, 40_000_000),
            up_in_body(&frozen, 40_000_000)
        ),
    );

    // The rules that were considered, each against the same arbiter and on
    // one walk of the samples. Every one of them tests or selects readings in
    // some way, and the table is what says that testing them is the trap: the
    // plain mean, which tests nothing, is the one that ships.
    let Some(back) = backward(&filter, imu, to_body, 120.0) else {
        return Ok(());
    };
    let buckets = seconds(imu, to_body, 95.0);
    println!("\ncandidate rule                   vs back");
    println!(
        "{:<32} {:>7.2}",
        "old (first trusted, 1 s)",
        angle(up_of(old.0), back)
    );
    for span in [40, 60, 90] {
        for window in [10, 20, 30] {
            if window > span {
                continue;
            }
            for pick in [Pick::Closest, Pick::Steady] {
                let Some(up) = windowed(&buckets, window, span, pick) else {
                    continue;
                };
                println!(
                    "{:<32} {:>7.2}",
                    format!("{pick:?} {window} s window in {span} s"),
                    angle(up, back)
                );
            }
        }
        let Some(up) = windowed(&buckets, span, span, Pick::Closest) else {
            continue;
        };
        println!(
            "{:<32} {:>7.2}",
            format!("plain mean over {span} s"),
            angle(up, back)
        );
        for (name, graded) in [("graded", true), ("gated", false)] {
            let Some(up) = weighted(&filter, &buckets, span, graded) else {
                continue;
            };
            println!(
                "{:<32} {:>7.2}",
                format!("{name} mean over {span} s"),
                angle(up, back)
            );
        }
    }
    Ok(())
}

/// The same mean, with each second counting for as much as the running filter
/// would believe it: `graded` weighs by the trust itself, otherwise a second
/// is in or out.
fn weighted(filter: &Filter, buckets: &[Bucket], span: usize, graded: bool) -> Option<[f64; 3]> {
    let mut sum = [0.0; 3];
    let mut weight = 0.0;
    for bucket in buckets.iter().take(span) {
        let Some(mean) = bucket.mean() else {
            continue;
        };
        let trust = {
            let (full, none) = filter.trust_g;
            let off = (norm(mean) - 1.0).abs();
            match off < full {
                true => 1.0,
                false => ((none - off) / (none - full)).clamp(0.0, 1.0),
            }
        };
        let w = match graded {
            true => trust,
            false => (trust > 0.0) as u8 as f64,
        };
        if w <= 0.0 {
            continue;
        }
        sum = std::array::from_fn(|axis| sum[axis] + mean[axis] * w);
        weight += w;
    }
    match weight > 0.0 {
        true => Some(unit(sum)),
        false => None,
    }
}

/// How a candidate stretch is judged.
#[derive(Clone, Copy, Debug)]
enum Pick {
    /// The mean closest to 1 g.
    Closest,
    /// The pieces closest to the gravity their own mean claims.
    Steady,
}

/// One second of carried readings, summed.
#[derive(Clone, Copy, Default)]
struct Bucket {
    sum: [f64; 3],
    count: f64,
}

impl Bucket {
    fn and(self, other: Self) -> Self {
        Self {
            sum: std::array::from_fn(|axis| self.sum[axis] + other.sum[axis]),
            count: self.count + other.count,
        }
    }

    fn mean(self) -> Option<[f64; 3]> {
        match self.count > 0.0 {
            true => Some(self.sum.map(|axis| axis / self.count)),
            false => None,
        }
    }
}

/// The opening `span` seconds of the track, one bucket a second, every reading
/// rotated into the frame of the first sample.
fn seconds(track: &GyroTrack, to_body: Mat3, span: f64) -> Vec<Bucket> {
    let samples = track.samples();
    let Some(first) = samples.first() else {
        return Vec::new();
    };
    let mut turned = Quat::IDENTITY;
    let mut previous = first.offset_us;
    let mut out: Vec<Bucket> = Vec::new();
    for sample in samples {
        let at = sample.offset_us - first.offset_us;
        if at > (span * 1e6) as i64 {
            break;
        }
        let dt = (sample.offset_us - previous).max(0) as f64 * 1e-6;
        previous = sample.offset_us;
        let rate = to_body.mul_vec(sample.rate_dps);
        turned = turned
            .times(Quat::from_rotation_vector(
                rate.map(|axis| axis.to_radians() * dt),
            ))
            .normalized();
        let accel = turned.rotate(to_body.mul_vec(sample.accel_g));
        let index = (at / 1_000_000).max(0) as usize;
        if out.len() <= index {
            out.resize(index + 1, Bucket::default());
        }
        out[index].sum = std::array::from_fn(|axis| out[index].sum[axis] + accel[axis]);
        out[index].count += 1.0;
    }
    out
}

/// The best `window` second stretch inside the first `span` seconds, judged
/// the way `pick` says, as the direction it calls up.
fn windowed(buckets: &[Bucket], window: usize, span: usize, pick: Pick) -> Option<[f64; 3]> {
    let reach = span.min(buckets.len());
    let mut best: Option<(f64, [f64; 3])> = None;
    for start in (0..reach.saturating_sub(window) + 1).step_by((window / 4).max(1)) {
        let inside = &buckets[start..(start + window).min(reach)];
        let whole = inside
            .iter()
            .fold(Bucket::default(), |held, b| held.and(*b));
        let mean = whole.mean()?;
        let magnitude = norm(mean);
        if magnitude <= 0.0 {
            continue;
        }
        let score = match pick {
            Pick::Closest => (magnitude - 1.0).abs(),
            Pick::Steady => {
                let gravity = mean.map(|axis| axis / magnitude);
                let pieces: Vec<[f64; 3]> = inside
                    .chunks((window / 4).max(1))
                    .filter_map(|chunk| {
                        chunk
                            .iter()
                            .fold(Bucket::default(), |held, b| held.and(*b))
                            .mean()
                    })
                    .collect();
                let spread: f64 = pieces
                    .iter()
                    .map(|piece| {
                        let off: [f64; 3] = std::array::from_fn(|axis| piece[axis] - gravity[axis]);
                        dot(off, off)
                    })
                    .sum();
                (spread / pieces.len().max(1) as f64).sqrt()
            }
        };
        if best.is_none_or(|(held, _)| score < held) {
            best = Some((score, mean));
        }
    }
    best.map(|(_, mean)| unit(mean))
}

/// The same filter run from `span` seconds in back to the first sample, and
/// where it says up is when it gets there, in the body's frame.
///
/// Reversing a track is negating its rates: the body that turned one way as
/// time ran forward turns the other way as it runs back. The accelerometer is
/// not a rate and is left alone.
fn backward(filter: &Filter, imu: &GyroTrack, to_body: Mat3, span: f64) -> Option<[f64; 3]> {
    let span_us = (span * 1e6) as i64;
    let first = imu.samples().first()?.offset_us;
    let inside: Vec<&GyroSample> = imu
        .samples()
        .iter()
        .take_while(|sample| sample.offset_us - first <= span_us)
        .collect();
    let last = inside.last()?.offset_us;
    let reversed: Vec<GyroSample> = inside
        .iter()
        .rev()
        .map(|sample| GyroSample {
            offset_us: last - sample.offset_us,
            rate_dps: sample.rate_dps.map(|rate| -rate),
            accel_g: sample.accel_g,
        })
        .collect();
    let solved = filter.solve(&GyroTrack::from_samples(reversed), to_body);
    // The reversed track's last sample is the forward track's first one. A
    // yaw about the world's own vertical cannot change which body direction
    // the estimate calls up, so the heading stabilization in here is harmless.
    let held = solved.samples().last()?.world_from_body;
    Some(held.conjugate().rotate(UP))
}

/// The seed rule this branch replaces: the first `accel_seconds` of
/// accelerometer the filter believes completely, carried back by the
/// gyroscope, else the window closest to 1 g inside 20 seconds.
fn first_trusted(
    filter: &Filter,
    track: &GyroTrack,
    to_body: Mat3,
) -> Option<(Quat, i64, [f64; 3])> {
    const SEARCH_US: i64 = 20_000_000;
    let samples = track.samples();
    let first = samples.first()?;
    let window_us = (filter.accel_seconds * 1e6) as i64;
    let mut turned = Quat::IDENTITY;
    let mut previous = first.offset_us;
    let mut opened = first.offset_us;
    let mut sum = [0.0; 3];
    let mut count = 0.0;
    let mut best: Option<(Quat, i64, [f64; 3])> = None;

    for sample in samples {
        let dt = (sample.offset_us - previous).max(0) as f64 * 1e-6;
        previous = sample.offset_us;
        let rate = to_body.mul_vec(sample.rate_dps);
        turned = turned
            .times(Quat::from_rotation_vector(
                rate.map(|axis| axis.to_radians() * dt),
            ))
            .normalized();
        let accel = turned.rotate(to_body.mul_vec(sample.accel_g));
        sum = std::array::from_fn(|axis| sum[axis] + accel[axis]);
        count += 1.0;

        if sample.offset_us - opened < window_us {
            continue;
        }
        let at_us = (opened + sample.offset_us) / 2;
        opened = sample.offset_us;
        let mean = sum.map(|axis| axis / count);
        sum = [0.0; 3];
        count = 0.0;
        let candidate = (upright(unit(mean)), at_us, mean);
        if filter_trusts(filter, norm(mean)) {
            return Some(candidate);
        }
        let off = |mean: [f64; 3]| (norm(mean) - 1.0).abs();
        best = match best {
            Some(held) if off(held.2) <= off(mean) => Some(held),
            _ => Some(candidate),
        };
        if sample.offset_us - first.offset_us >= SEARCH_US {
            break;
        }
    }
    best
}

fn filter_trusts(filter: &Filter, magnitude_g: f64) -> bool {
    (magnitude_g - 1.0).abs() < filter.trust_g.0
}

/// The shortest rotation from where a reading says up is to where the world
/// says it is.
fn upright(up: [f64; 3]) -> Quat {
    let axis = cross(up, UP);
    let angle = dot(up, UP).clamp(-1.0, 1.0).acos();
    match norm(axis) > 1e-9 {
        true => Quat::from_rotation_vector(axis.map(|c| c * angle / norm(axis))),
        false => Quat::from_rotation_vector([angle, 0.0, 0.0]),
    }
}

fn angle(a: [f64; 3], b: [f64; 3]) -> f64 {
    dot(unit(a), unit(b)).clamp(-1.0, 1.0).acos().to_degrees()
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
