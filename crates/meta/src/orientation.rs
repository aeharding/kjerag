//! Where the camera was pointing, frame by frame: the gyroscope integrated,
//! held level by the accelerometer, and with the fast half of its heading
//! taken out.
//!
//! The output is one quaternion per IMU sample, `world_from_body`, taking a
//! direction in the camera body's frame to the stabilized world frame. Both
//! frames are the one the rest of Kyerag uses: **x right, y down, z forward**,
//! and in the world frame y is gravity. `kyerag-render` composes the inverse
//! into its view rotation, so a body that rolls leaves the world where it was.
//!
//! ## Why a complementary filter and not something cleverer
//!
//! Two sensors, and each is trustworthy over the half of the frequency range
//! the other is not. The gyroscope is exact over a second and drifts over a
//! minute; the accelerometer knows which way is down over a minute and reads
//! every bump over a second. Adding the first and the low-passed disagreement
//! of the second is the whole filter, and it is four lines of
//! [`Filter::solve`]. A Kalman filter estimates the same two states with a
//! covariance nobody here can populate from a file that records no noise
//! figures. Nothing measured on this footage asks for one.
//!
//! ## What it cannot do
//!
//! An accelerometer cannot tell gravity from a turn. In a coordinated turn
//! the specific force points along the aircraft's own vertical rather than the
//! world's, and a filter that believed it would lean the horizon into every
//! turn. Two things keep that out: the correction is trusted only while the
//! magnitude is near 1 g, which a banked turn is not, and its time constant is
//! long enough that a turn is over before it moves the estimate far. What is
//! left is a slow lean during a long banked turn, bounded by the trust window.
//!
//! ## Where it starts
//!
//! The same rule has to cover the first reading as covers all the others, and
//! issue #45 is what happens when it does not. The estimate used to start from
//! whichever tilt put the first tenth of a second of accelerometer on the world
//! vertical, **whatever that tenth of a second read**: on the April 10 X4 Air
//! capture it reads 1.281 g, which is a reading the running filter refuses
//! outright, and the horizon came out 49 degrees off level and took tens of
//! seconds to walk back. [`Filter::seed`] puts the starting reading through the
//! running filter's own trust window, and past the whole of it: a seed is
//! applied at full weight, so it has to be a reading the filter would apply at
//! full weight.

use super::calibration::Pose;
use super::gyro::GyroTrack;
use super::rotation::{Mat3, Quat, cross, dot, norm};

/// Which way gravity points in the world frame: down, and y is down.
///
/// An accelerometer at rest measures the force holding it up, so this is the
/// direction its reading has to end up pointing after the body rotation, and
/// it is the negative of the gravity vector rather than the gravity vector.
const UP_IN_WORLD: [f64; 3] = [0.0, -1.0, 0.0];

/// How far apart the solved orientations are kept, in microseconds.
///
/// The integration runs at the IMU's own rate, which is 997 Hz on the X4 Air
/// and 500 on a ONE X2; what is stored is decimated to 200 a second, because
/// a 30-minute capture is 1.8 million samples and 72 MB of quaternions for a
/// signal nothing reads faster than this. 5 ms is still three times finer
/// than the 15.9 ms rolling-shutter readout it exists to serve (issue #9),
/// and 5 ms of the worst rate measured on this footage, 523 deg/s, is 2.6
/// degrees, which [`OrientationTrack::at`] interpolates across.
const STORE_US: i64 = 5_000;

/// How the gyroscope and the accelerometer are mixed, and how much of the
/// heading reaches the picture.
///
/// Both are time constants in seconds, and both limits are the useful ones:
/// an infinite `tilt_seconds` is the gyroscope alone, a zero `yaw_seconds` is
/// no heading stabilization at all, and an infinite `yaw_seconds` locks the
/// view to the heading the file starts on. The harness in `kyerag-spike` uses
/// exactly those limits to bracket what the shipped numbers buy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Filter {
    /// How long the accelerometer is smoothed over before it is believed.
    ///
    /// The IMU runs at 997 Hz and a paramotor engine runs at about 80, so
    /// the raw signal is mostly vibration: measured over a 30-minute X4 Air
    /// capture, the raw magnitude runs 0.69 to 1.63 g between the 10th and
    /// 90th percentile and the same signal smoothed over a second runs 0.95
    /// to 1.05. Both the direction and the trust window below are read off
    /// the smoothed signal, because the raw one would have the window
    /// throwing away three samples in five for a reason that is not motion.
    pub accel_seconds: f64,
    /// How long the accelerometer takes to pull the estimated vertical back
    /// onto gravity.
    ///
    /// It has one job, which is to cancel gyroscope bias, and bias is
    /// constant where everything else in the signal is not. Long is therefore
    /// safe and short is not: what a short constant buys is faster recovery
    /// from a disturbance the gyroscope did not have, and what it costs is
    /// leaning the horizon into every turn.
    pub tilt_seconds: f64,
    /// Heading changes slower than this reach the picture; faster ones are
    /// cancelled.
    ///
    /// This is the one number that is a judgement about flying rather than
    /// about sensors. A deliberate turn has to read as a turn, and the swing
    /// of a camera under a wing has to not.
    pub yaw_seconds: f64,
    /// How far from 1 g the accelerometer may read and still be believed
    /// completely, and how far before it is not believed at all. Between them
    /// the correction fades linearly.
    pub trust_g: (f64, f64),
}

impl Default for Filter {
    /// The shipped numbers. Every one of them is measured on real X4 Air
    /// paramotor footage; the method and the tables are in
    /// docs/research/insv-format.md 8.5.
    fn default() -> Self {
        Self {
            accel_seconds: 1.0,
            tilt_seconds: 20.0,
            yaw_seconds: 3.0,
            trust_g: (0.05, 0.20),
        }
    }
}

/// Where [`Filter::solve`] starts the estimate, and what it read to get there.
///
/// Handed out rather than kept private because the whole of issue #45 was a
/// seed nothing could see: the instrument that measures the horizon reports
/// this line, so a file whose estimate starts from something the filter does
/// not believe says so before any pixel is measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seed {
    /// The attitude to start integrating from, at the track's first sample.
    pub world_from_body: Quat,
    /// The middle of the window it was read from, on the track's own clock.
    /// Later than the start of the track by however far the search had to go
    /// to find a reading worth having.
    pub at_us: i64,
    /// What that window's mean reading weighed, in g.
    pub magnitude_g: f64,
    /// Whether the running filter would have believed that reading completely.
    ///
    /// False is the documented fallback: no window inside the search read
    /// gravity, so the closest one to it was taken. A file with the motor
    /// running from the first frame is what reaches it.
    pub trusted: bool,
}

/// Where the camera body was, at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrientationSample {
    /// Media time, relative to the file's first frame, on the same clock the
    /// gyro track is read onto.
    pub offset_us: i64,
    /// Takes a direction in the camera body's frame to the stabilized world
    /// frame.
    pub world_from_body: Quat,
}

/// The camera body's orientation over the whole file, at the IMU's own rate.
///
/// Kept at the sample rate rather than reduced to one per frame on purpose:
/// rolling-shutter correction (issue #9) needs an orientation part way
/// through a frame, and the readout it corrects for is 15.9 ms on the X4 Air
/// against a sample every 2 ms.
#[derive(Clone, Default, PartialEq)]
pub struct OrientationTrack {
    samples: Vec<OrientationSample>,
}

impl std::fmt::Debug for OrientationTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrientationTrack")
            .field("samples", &self.samples.len())
            .field("first", &self.samples.first())
            .field("last", &self.samples.last())
            .finish()
    }
}

impl OrientationTrack {
    pub fn samples(&self) -> &[OrientationSample] {
        &self.samples
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Where the body was at `offset_us`, interpolated between the two
    /// samples either side and clamped at both ends of the track.
    ///
    /// Identity for a file with no IMU record, which is what makes horizon
    /// lock a no-op on such a file rather than an error.
    ///
    /// [`Self::turn`] is what rolling-shutter correction reads (issue #9),
    /// and it is this call at the two ends of one frame's readout.
    pub fn at(&self, offset_us: i64) -> Quat {
        let after = self
            .samples
            .partition_point(|sample| sample.offset_us < offset_us);
        let Some(next) = self.samples.get(after) else {
            return self.samples.last().map_or(Quat::IDENTITY, at_sample);
        };
        let Some(previous) = after.checked_sub(1).and_then(|at| self.samples.get(at)) else {
            return at_sample(next);
        };
        let span = (next.offset_us - previous.offset_us) as f64;
        let t = match span > 0.0 {
            true => (offset_us - previous.offset_us) as f64 / span,
            false => 0.0,
        };
        at_sample(previous).nlerp(at_sample(next), t)
    }

    /// How the body turned between two instants, as a rotation vector in the
    /// body's own frame: what a direction fixed in the world does when seen
    /// from a body that moved.
    ///
    /// **This is the hook rolling-shutter correction hangs on (issue #9).** A
    /// sensor row is exposed `rolling_shutter_time * (row / rows - 0.5)` away
    /// from the frame's own instant, so the turn across one whole readout is
    /// this call at the two ends of it, and a row's share of that turn is the
    /// vector scaled. Reading it as a vector rather than as one orientation
    /// per row is what lets the shader scale it per pixel: 3840 rows would
    /// otherwise be 3840 lookups, and the samples are 5 ms apart against
    /// 15.9 ms of readout, so there is nothing in between them to resolve.
    ///
    /// Zero for a file with no IMU record, which is what silently switches
    /// the correction off rather than erroring.
    pub fn turn(&self, from_us: i64, to_us: i64) -> [f64; 3] {
        self.at(to_us)
            .conjugate()
            .times(self.at(from_us))
            .rotation_vector()
    }
}

fn at_sample(sample: &OrientationSample) -> Quat {
    sample.world_from_body
}

/// The rotation that takes an IMU reading into the camera body's frame.
///
/// Two steps, and only the first of them is large:
///
/// - the **axis convention**, a three-letter string per camera model
///   ([`axis_map`]), which says which sensor axis is which and which way
///   round. A wrong one tilts or mirrors the horizon, and it is the thing
///   this file's negative control checks can be caught.
/// - the **sensor extrinsics**, `offset_v3`'s own yaw, pitch and roll for
///   lens 0, inverted, because the axis convention lands in the front
///   sensor's frame and the body is what everything downstream is measured
///   in (docs/research/insv-format.md 8.4).
///
/// Note which of the two mountings that second step is:
/// [`Pose::sensor_from_body`], with `roll` as the file writes it, and **not**
/// [`Pose::lens_from_body`], which carries the quarter-turn datum the
/// delivered picture needs. They differ by 90 degrees and the difference is
/// visible in one rendered frame: held level by its accelerometer alone, an
/// X4 Air comes out a quarter turn on its side through the second and level
/// through the first. The IMU is bolted to the sensor, not to the picture.
pub fn body_from_imu(orientation: &str, lens0: &Pose) -> Mat3 {
    lens0
        .sensor_from_body()
        .transpose()
        .times(axis_map(orientation))
}

/// A three-letter axis convention as a matrix: letter `i` names the sensor
/// axis that feeds output axis `i`, and **a lower-case letter is negated**.
///
/// The case rule is the one the format study records
/// (docs/research/insv-format.md 8.4), and it is checked against physics
/// rather than against upstream's source: at rest an accelerometer reads 1 g
/// straight up, so the axis map that puts a resting X4 Air's reading on the
/// body frame's own vertical is the right one, and
/// `an_x4_air_at_rest_reads_gravity_up` is that check on real samples.
///
/// An unknown letter contributes a zero row, which is a reading that goes
/// nowhere rather than one that goes somewhere wrong.
pub fn axis_map(orientation: &str) -> Mat3 {
    let mut rows = [[0.0; 3]; 3];
    for (row, letter) in orientation.chars().take(3).enumerate() {
        let axis = match letter.to_ascii_lowercase() {
            'x' => 0,
            'y' => 1,
            'z' => 2,
            _ => continue,
        };
        rows[row][axis] = match letter.is_lowercase() {
            true => -1.0,
            false => 1.0,
        };
    }
    Mat3::new(rows)
}

/// How far into the track a seed may be looked for, in microseconds.
///
/// The default `tilt_seconds`, and that is the argument for it: a reading
/// fetched from further away than the filter's own settling time is worth less
/// than letting the filter settle, and the gyroscope has to carry it back over
/// more of its own drift. At the 0.05 deg/s an X4 Air measures at rest
/// (docs/research/insv-format.md 8.5), 20 seconds of carrying is 1 degree.
const SEED_SEARCH_US: i64 = 20_000_000;

impl Filter {
    /// The attitude to start the estimate from: the first stretch of
    /// accelerometer the running filter would believe completely, carried back
    /// to the start of the track by the gyroscope.
    ///
    /// **Issue #45 is what the previous seed cost.** It took the first tenth
    /// of a second whatever it read, and on the April 10 capture that tenth of
    /// a second weighs 1.281 g, which [`Filter::trust`] refuses outright. The
    /// horizon started 49 degrees off level and walked back over tens of
    /// seconds, because the correction that has to undo a bad seed is the same
    /// slow one that exists to ignore turns.
    ///
    /// Four choices in here, and each has a cost.
    ///
    /// - **Search forward rather than burn in.** A burn-in pass would use
    ///   every trusted sample in the opening stretch instead of the first
    ///   window's worth, but it converges at `tilt_seconds` from whatever it
    ///   started at, so it needs a second time constant of its own to be worth
    ///   anything. This is one extra walk over at most [`SEED_SEARCH_US`] of
    ///   samples and no new number to justify.
    /// - **A window as long as `accel_seconds`.** The running filter reads its
    ///   trust off the accelerometer smoothed over that constant, so this is
    ///   the same test on the same kind of signal. A shorter window would let
    ///   a magnitude that is only passing through 1 g on its way somewhere
    ///   else be taken for stillness: the raw signal on this footage runs 0.69
    ///   to 1.63 g between the 10th and 90th percentile and crosses 1 g
    ///   constantly.
    /// - **Believed completely, not believed at all.** The running filter
    ///   applies a fraction of a correction to a reading it half believes; a
    ///   seed is applied whole, so what it asks for is the whole of
    ///   [`Filter::trust`] rather than any of it. That is worth the difference
    ///   between a horizon 13.8 degrees off level at 6 seconds and one 1.8
    ///   degrees off, measured on the April 10 capture through the render path
    ///   (`kyerag-spike --bin dip`), because the window it settles for
    ///   otherwise is one taken during the launch.
    /// - **Every reading carried back before it is averaged.** The window may
    ///   sit seconds after the start of the track, and the body will have
    ///   turned in between, so each sample is rotated into the frame of the
    ///   track's first sample by the gyroscope before it goes into the mean.
    ///   That makes the answer an attitude at the start of the track directly,
    ///   with no separate back-rotation and no assumption that the body held
    ///   still inside the window.
    ///
    /// `None` only for a track with no samples in it.
    pub fn seed(&self, track: &GyroTrack, body_from_imu: Mat3) -> Option<Seed> {
        let samples = track.samples();
        let first = samples.first()?;
        let window_us = (self.accel_seconds * 1e6).max(0.0) as i64;
        let mut turned = Quat::IDENTITY;
        let mut previous = first.offset_us;
        let mut opened = first.offset_us;
        let mut mean = Mean::default();
        let mut best = None;

        for sample in samples {
            let dt = (sample.offset_us - previous).max(0) as f64 * 1e-6;
            previous = sample.offset_us;
            let rate = body_from_imu.mul_vec(sample.rate_dps);
            turned = turned
                .times(Quat::from_rotation_vector(
                    rate.map(|axis| axis.to_radians() * dt),
                ))
                .normalized();
            mean.add(turned.rotate(body_from_imu.mul_vec(sample.accel_g)));

            if sample.offset_us - opened < window_us {
                continue;
            }
            let at_us = (opened + sample.offset_us) / 2;
            opened = sample.offset_us;
            match mean.take().and_then(|mean| self.reading(mean, at_us)) {
                Some(candidate) if candidate.trusted => return Some(candidate),
                Some(candidate) => best = closer_to_gravity(best, candidate),
                None => (),
            }
            if sample.offset_us - first.offset_us >= SEED_SEARCH_US {
                break;
            }
        }
        // Whatever the last window did not fill. A track shorter than one
        // window is still a track, and it is the only way this arm is reached.
        let at_us = (opened + previous) / 2;
        match mean.take().and_then(|mean| self.reading(mean, at_us)) {
            Some(candidate) => closer_to_gravity(best, candidate),
            None => best,
        }
    }

    /// One window's mean reading as a candidate to start from, or `None` where
    /// the readings cancelled and there is no direction in them at all.
    fn reading(&self, mean: [f64; 3], at_us: i64) -> Option<Seed> {
        let magnitude_g = norm(mean);
        if magnitude_g <= 0.0 {
            return None;
        }
        Some(Seed {
            world_from_body: upright(mean.map(|axis| axis / magnitude_g)),
            at_us,
            magnitude_g,
            trusted: self.trust(magnitude_g) >= 1.0,
        })
    }

    /// Integrate one IMU track into an orientation track.
    ///
    /// `body_from_imu` is the rotation from [`body_from_imu`], handed in
    /// rather than derived here so that the harness can hand in a deliberately
    /// wrong one and watch the horizon fall over.
    pub fn solve(&self, track: &GyroTrack, body_from_imu: Mat3) -> OrientationTrack {
        let samples = track.samples();
        let Some(first) = samples.first() else {
            return OrientationTrack::default();
        };

        let mut world_from_body = self
            .seed(track, body_from_imu)
            .map_or(Quat::IDENTITY, |seed| seed.world_from_body);
        // The smoothed accelerometer starts on the accelerometer, and not on
        // the reading the seed wishes it had. Starting it at 1 g along the
        // estimated vertical made the smoother walk from that fiction out to
        // whatever the sensor really said, and everything it passed through on
        // the way was inside the trust window and believed: a second helping of
        // the same defect as issue #45's seed, worth a few degrees on a file
        // that opens far from gravity.
        let mut gravity = body_from_imu.mul_vec(first.accel_g);
        let mut heading_held = 0.0;
        let mut previous = first.offset_us;
        let mut out = Vec::with_capacity(samples.len() / 4);

        for sample in samples {
            let dt = (sample.offset_us - previous).max(0) as f64 * 1e-6;
            previous = sample.offset_us;

            let rate = body_from_imu.mul_vec(sample.rate_dps);
            world_from_body = world_from_body
                .times(Quat::from_rotation_vector(
                    rate.map(|axis| axis.to_radians() * dt),
                ))
                .normalized();

            let accel = body_from_imu.mul_vec(sample.accel_g);
            let follow = |seconds: f64| match seconds + dt > 0.0 {
                true => dt / (seconds + dt),
                false => 0.0,
            };
            let smoothing = follow(self.accel_seconds);
            gravity = std::array::from_fn(|axis| {
                gravity[axis] + (accel[axis] - gravity[axis]) * smoothing
            });
            world_from_body = self.levelled(world_from_body, gravity, dt);

            // The heading the view is allowed to keep is the part of it the
            // low pass has not caught up with yet, so the filtered heading is
            // simply taken back off. Everything below the corner frequency
            // reaches the picture and everything above it is cancelled.
            let heading = world_from_body.heading();
            heading_held += wrap(heading - heading_held) * follow(self.yaw_seconds);

            let due = out.last().is_none_or(|held: &OrientationSample| {
                sample.offset_us - held.offset_us >= STORE_US
            });
            if due {
                out.push(OrientationSample {
                    offset_us: sample.offset_us,
                    world_from_body: Quat::about_down(-heading_held).times(world_from_body),
                });
            }
        }
        OrientationTrack { samples: out }
    }

    /// One step of the accelerometer half: turn the estimate towards the
    /// reading, by as much of the disagreement as the time constant and the
    /// trust in this reading allow.
    fn levelled(&self, world_from_body: Quat, accel_g: [f64; 3], dt: f64) -> Quat {
        let magnitude = norm(accel_g);
        let gain = self.trust(magnitude) * (dt / self.tilt_seconds).min(1.0);
        if gain <= 0.0 {
            return world_from_body;
        }
        // The disagreement between where the reading says up is and where the
        // estimate says up is, as a rotation vector. Its cross product with
        // the world vertical has no vertical component of its own, so this can
        // only ever correct tilt: heading is not observable from gravity and
        // is not touched here.
        let up = world_from_body.rotate(accel_g.map(|axis| axis / magnitude));
        let error = cross(up, UP_IN_WORLD);
        Quat::from_rotation_vector(error.map(|axis| axis * gain))
            .times(world_from_body)
            .normalized()
    }

    /// How much of a reading to believe, from how far its magnitude is from
    /// 1 g. Everything inside the window is gravity plus noise; everything
    /// outside it is the aircraft accelerating, and a turn is the case that
    /// matters.
    fn trust(&self, magnitude_g: f64) -> f64 {
        let (full, none) = self.trust_g;
        let off = (magnitude_g - 1.0).abs();
        match off < full {
            true => 1.0,
            false => ((none - off) / (none - full)).clamp(0.0, 1.0),
        }
    }
}

/// The shortest rotation from where a reading says up is to where the world
/// says it is.
///
/// A camera exactly upside down has a whole circle of shortest rotations and
/// no reason to prefer one; nothing else does.
fn upright(up: [f64; 3]) -> Quat {
    let axis = cross(up, UP_IN_WORLD);
    let angle = dot(up, UP_IN_WORLD).clamp(-1.0, 1.0).acos();
    match norm(axis) > 1e-9 {
        true => Quat::from_rotation_vector(axis.map(|c| c * angle / norm(axis))),
        false => Quat::from_rotation_vector([angle, 0.0, 0.0]),
    }
}

/// A running mean of readings.
#[derive(Default)]
struct Mean {
    sum: [f64; 3],
    count: f64,
}

impl Mean {
    fn add(&mut self, reading: [f64; 3]) {
        self.sum = std::array::from_fn(|axis| self.sum[axis] + reading[axis]);
        self.count += 1.0;
    }

    /// The mean so far, and the accumulator emptied. `None` where nothing was
    /// added, which is the window a track shorter than one leaves behind.
    fn take(&mut self) -> Option<[f64; 3]> {
        let mean = match self.count > 0.0 {
            true => Some(self.sum.map(|axis| axis / self.count)),
            false => None,
        };
        *self = Self::default();
        mean
    }
}

/// Of two readings neither of which is gravity, the one closer to it.
fn closer_to_gravity(held: Option<Seed>, candidate: Seed) -> Option<Seed> {
    let off = |seed: &Seed| (seed.magnitude_g - 1.0).abs();
    match held {
        Some(held) if off(&held) <= off(&candidate) => Some(held),
        _ => Some(candidate),
    }
}

/// An angle wrapped into (-pi, pi], so that a heading crossing the back of
/// the compass is a small change and not a whole turn.
fn wrap(angle: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    let wrapped = (angle + PI).rem_euclid(TAU) - PI;
    match wrapped <= -PI {
        true => PI,
        false => wrapped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gyro::GyroSample;
    use std::f64::consts::PI;

    const HZ: i64 = 200;
    const STEP_US: i64 = 1_000_000 / HZ;

    /// A synthetic IMU: level, still, and reading gravity, unless a step says
    /// otherwise.
    fn track(seconds: f64, mut step: impl FnMut(f64) -> ([f64; 3], [f64; 3])) -> GyroTrack {
        let count = (seconds * HZ as f64) as i64;
        GyroTrack::from_samples(
            (0..count)
                .map(|index| {
                    let (rate_dps, accel_g) = step(index as f64 / HZ as f64);
                    GyroSample {
                        offset_us: index * STEP_US,
                        rate_dps,
                        accel_g,
                    }
                })
                .collect(),
        )
    }

    /// Still and level: gravity up the body's own vertical, nothing turning.
    fn resting(_t: f64) -> ([f64; 3], [f64; 3]) {
        ([0.0; 3], [0.0, -1.0, 0.0])
    }

    /// How far a stabilized orientation leaves the world's vertical from the
    /// body's, in degrees. Zero is a level horizon.
    fn tilt_deg(q: Quat) -> f64 {
        dot(q.rotate([0.0, 1.0, 0.0]), [0.0, 1.0, 0.0])
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees()
    }

    fn gyro_only() -> Filter {
        Filter {
            tilt_seconds: f64::INFINITY,
            yaw_seconds: f64::INFINITY,
            ..Filter::default()
        }
    }

    /// Constant rate in, expected angle out: 90 degrees per second about the
    /// body's forward axis for two seconds is half a turn, and no filter
    /// setting may change that, because gravity says nothing about roll rate
    /// over two seconds.
    ///
    /// The **turn** is what that claim is about, so the turn is what is
    /// measured. This track rolls the body while holding the accelerometer
    /// fixed in it, which is not something gravity can do, so where the
    /// estimate starts is a question the track has no answer to; where it ends
    /// up relative to where it started is the gyroscope's alone.
    #[test]
    fn a_constant_rate_integrates_to_the_angle_it_should() {
        let solved = gyro_only().solve(
            &track(2.0, |_| ([0.0, 0.0, 90.0], [0.0, -1.0, 0.0])),
            Mat3::IDENTITY,
        );

        let turned = solved
            .at(0)
            .conjugate()
            .times(solved.samples().last().unwrap().world_from_body);
        // Two seconds at 90 deg/s, less the one sample interval the track
        // stops short of it.
        let expected = Quat::from_rotation_vector([0.0, 0.0, (2.0 - 1.0 / HZ as f64) * PI / 2.0]);
        assert!(
            turned.angle_to(expected).to_degrees() < 0.01,
            "{:?}",
            turned.angle_to(expected).to_degrees()
        );
    }

    /// The whole point of the accelerometer half: a gyroscope with a bias in
    /// it drifts away from level without bound, and the correction holds it.
    ///
    /// **What it holds it to is `tilt_seconds * bias`**, and that is the
    /// design constraint rather than an implementation detail: the correction
    /// only pushes back in proportion to the error it can see, so it settles
    /// where the push matches the bias. 0.05 deg/s is what an X4 Air measures
    /// at rest (docs/research/insv-format.md 8.5) and 20 seconds is the
    /// shipped constant, so 1 degree is the tilt this arrangement is worth,
    /// and it is why `tilt_seconds` cannot simply be made enormous.
    #[test]
    fn a_biased_gyroscope_drifts_and_the_accelerometer_bounds_it() {
        let biased = || track(600.0, |_| ([0.05, 0.0, 0.0], [0.0, -1.0, 0.0]));

        let adrift = gyro_only().solve(&biased(), Mat3::IDENTITY);
        let held = Filter::default().solve(&biased(), Mat3::IDENTITY);

        let last =
            |solved: &OrientationTrack| tilt_deg(solved.samples().last().unwrap().world_from_body);
        assert!(last(&adrift) > 29.0, "{} degrees adrift", last(&adrift));
        let settled = Filter::default().tilt_seconds * 0.05;
        assert!(
            (last(&held) - settled).abs() < 0.1,
            "{} degrees held, against {settled} predicted",
            last(&held)
        );
    }

    /// And it settles rather than oscillating: an estimate started a long way
    /// off level comes back and stays back.
    ///
    /// The camera really is tilted for the first two seconds here, at a
    /// magnitude the filter believes. A start that is wrong because the
    /// *reading* was wrong is issue #45 and no longer reaches the estimate at
    /// all, so it cannot be what this test starts from.
    #[test]
    fn a_level_camera_that_starts_wrong_is_pulled_level() {
        let filter = Filter {
            tilt_seconds: 1.0,
            ..Filter::default()
        };
        // A 30 degree tilt at 1 g, held long enough to fill the seed's window,
        // and then the camera is level and still.
        let solved = filter.solve(
            &track(20.0, |t| match t < 2.0 {
                true => ([0.0; 3], [0.5, -0.866, 0.0]),
                false => resting(t),
            }),
            Mat3::IDENTITY,
        );

        let at = |seconds: f64| tilt_deg(solved.at((seconds * 1e6) as i64));
        assert!(at(0.0) > 25.0, "{} degrees", at(0.0));
        assert!(at(10.0) < 0.5, "{} degrees", at(10.0));
        assert!(at(19.0) < 0.5, "{} degrees", at(19.0));
    }

    /// **Issue #45.** The estimate may not start from a reading the running
    /// filter would refuse, because the correction that has to undo a bad
    /// start is the same slow one that exists to ignore turns: on the April 10
    /// capture that was 49 degrees of horizon at 6 seconds and tens of seconds
    /// of walking back.
    ///
    /// The opening two seconds here weigh 1.34 g, which is the shape of that
    /// file's first tenth of a second at 1.281 g, and they point 63 degrees
    /// off the body's own vertical. Believing them is the defect; the horizon
    /// has to be level from the first frame.
    #[test]
    fn a_reading_the_filter_refuses_does_not_start_the_estimate() {
        let refused = [1.2, -0.6, 0.0];
        let track = track(30.0, |t| match t < 2.0 {
            true => ([0.0; 3], refused),
            false => resting(t),
        });

        let seed = Filter::default().seed(&track, Mat3::IDENTITY).unwrap();
        assert!(seed.trusted, "{seed:?}");
        assert!(seed.at_us > 2_000_000, "{seed:?}");
        // What believing that opening would have been worth, so the test says
        // what it is defending against rather than only that it holds.
        assert!(
            (upright(refused.map(|axis| axis / norm(refused))).angle_to(Quat::IDENTITY)
                - 63f64.to_radians())
            .abs()
                < 0.02
        );

        let solved = Filter::default().solve(&track, Mat3::IDENTITY);
        let at = |seconds: f64| tilt_deg(solved.at((seconds * 1e6) as i64));
        assert!(at(0.0) < 0.5, "{} degrees at the first frame", at(0.0));
        // Two degrees survive at 6 s: the smoothed accelerometer crosses the
        // trust window as it converges on the step this track makes at 2 s,
        // and it is believed on the way through. Against that, the seed this
        // replaced put the April 10 capture 48.9 degrees off at 6 s, measured
        // through the render path by `kyerag-spike --bin dip`.
        assert!(at(6.0) < 3.0, "{} degrees at 6 s", at(6.0));
        // And that much decays at `tilt_seconds` like anything else the
        // correction has to undo.
        assert!(at(30.0) < 1.0 && at(30.0) < at(6.0), "{} at 30 s", at(30.0));
    }

    /// And the reading is carried back over whatever the body did before it.
    ///
    /// The window this seed comes from sits two seconds into the track, and
    /// the body rolls a quarter turn to reach it. Reading the attitude there
    /// and starting the track on it would have every frame of those two
    /// seconds a quarter turn out.
    #[test]
    fn the_seed_is_carried_back_over_whatever_the_body_did_first() {
        // Rolling at 45 deg/s for two seconds, then still, so the accelerometer
        // reads gravity in the frame the quarter turn left the body in.
        let solved = Filter::default().solve(
            &track(30.0, |t| match t < 2.0 {
                true => ([0.0, 0.0, 45.0], [1.2, -0.6, 0.0]),
                false => ([0.0; 3], [-1.0, 0.0, 0.0]),
            }),
            Mat3::IDENTITY,
        );

        // The body was level when the track started, whatever it did next.
        let start = solved.at(0).angle_to(Quat::IDENTITY).to_degrees();
        assert!(start < 1.0, "{start} degrees at the first frame");
        // And a quarter turn on by the time the reading was taken.
        let rolled = solved
            .at(3_000_000)
            .angle_to(Quat::from_rotation_vector([0.0, 0.0, PI / 2.0]))
            .to_degrees();
        assert!(rolled < 1.0, "{rolled} degrees off the quarter turn");
    }

    /// Half believed is not good enough to start from.
    ///
    /// The running filter would apply part of a correction to a 1.1 g reading.
    /// A seed is applied whole, so it waits for a reading worth applying
    /// whole: on the April 10 capture the difference is a window taken during
    /// the launch against one taken after it, and 13.8 degrees of horizon at
    /// 6 seconds against 1.8.
    #[test]
    fn a_half_believed_reading_is_not_good_enough_to_start_from() {
        let filter = Filter::default();
        let leaning = [0.71, -0.84, 0.0];
        assert!(
            filter.trust(norm(leaning)) > 0.0 && filter.trust(norm(leaning)) < 1.0,
            "{} g has to be the half believed case",
            norm(leaning)
        );

        let seed = filter
            .seed(
                &track(30.0, |t| match t < 5.0 {
                    true => ([0.0; 3], leaning),
                    false => resting(t),
                }),
                Mat3::IDENTITY,
            )
            .unwrap();

        assert!(seed.trusted, "{seed:?}");
        assert!(seed.at_us > 5_000_000, "{seed:?}");
        assert!(
            seed.world_from_body.angle_to(Quat::IDENTITY).to_degrees() < 0.5,
            "{seed:?}"
        );
    }

    /// A file that never reads gravity gets the closest thing to it, not the
    /// first thing, and not a panic.
    ///
    /// A motor running from the first frame is the case: nothing in the search
    /// is inside the trust window, so there is no right answer, only a least
    /// bad one. Taking the closest reading to 1 g makes the fallback at worst
    /// equal to the opening window this used to take unconditionally, because
    /// that window is one of the candidates.
    #[test]
    fn a_file_that_never_reads_gravity_takes_the_closest_thing_to_it() {
        let filter = Filter::default();
        // The opening reading is the furthest from gravity and points 40
        // degrees off; the one at 5 seconds is the closest and points along
        // the body's own vertical.
        let track = track(30.0, |t| {
            let accel = match t {
                t if t < 5.0 => [1.29, -1.53, 0.0],
                t if t < 7.0 => [0.0, -1.25, 0.0],
                _ => [0.0, -1.4, 0.0],
            };
            ([0.0; 3], accel)
        });

        let seed = filter.seed(&track, Mat3::IDENTITY).unwrap();
        assert!(!seed.trusted, "{seed:?}");
        assert!((seed.magnitude_g - 1.25).abs() < 0.01, "{seed:?}");
        assert!((5_000_000..7_000_000).contains(&seed.at_us), "{seed:?}");
        assert!(
            seed.world_from_body.angle_to(Quat::IDENTITY).to_degrees() < 0.5,
            "{seed:?}"
        );
        assert!(!filter.solve(&track, Mat3::IDENTITY).is_empty());
    }

    /// The search gives up rather than reaching across the file for a reading.
    ///
    /// A seed fetched from further away than the filter's own settling time is
    /// worth less than letting the filter settle, and the gyroscope has to
    /// carry it back over more of its own drift. This track only reads gravity
    /// at 25 seconds and the search stops before it.
    #[test]
    fn the_search_for_a_seed_stops_at_the_filters_own_settling_time() {
        let seed = Filter::default()
            .seed(
                &track(40.0, |t| match t < 25.0 {
                    true => ([0.0; 3], [0.0, -1.4, 0.0]),
                    false => resting(t),
                }),
                Mat3::IDENTITY,
            )
            .unwrap();

        assert!(!seed.trusted, "{seed:?}");
        assert!(seed.at_us < SEED_SEARCH_US + 1_000_000, "{seed:?}");
    }

    /// A turn is not gravity. In a 45 degree banked turn the specific force is
    /// 1.41 g and points along the aircraft's own vertical, and a filter that
    /// believed it would lean the horizon into the turn.
    #[test]
    fn a_banked_turn_is_not_believed_to_be_gravity() {
        let filter = Filter::default();
        assert_eq!(filter.trust(1.0), 1.0);
        assert_eq!(filter.trust(1.02), 1.0);
        assert_eq!(filter.trust(2.0f64.sqrt()), 0.0, "a 45 degree bank");
        assert!(
            filter.trust(1.1) > 0.0 && filter.trust(1.1) < 1.0,
            "a shallow one"
        );
        // Falling, which is the other end of the same window.
        assert_eq!(filter.trust(0.5), 0.0);
    }

    /// Roll and pitch are locked completely: a body swinging like a pendulum
    /// leaves the world's vertical where it was, whatever the swing.
    #[test]
    fn a_swinging_body_leaves_the_horizon_where_it_was() {
        // 20 degrees of roll at a 3 second period, which is a paraglider's
        // pendulum, with the accelerometer reading the swing as well.
        let swing = |t: f64| {
            let phase = t * 2.0 * PI / 3.0;
            let angle = 20f64.to_radians() * phase.sin();
            let rate = 20f64.to_radians() * (2.0 * PI / 3.0) * phase.cos();
            (
                [0.0, 0.0, rate.to_degrees()],
                [-angle.sin(), -angle.cos(), 0.0],
            )
        };
        let solved = Filter::default().solve(&track(30.0, swing), Mat3::IDENTITY);

        let worst = solved
            .samples()
            .iter()
            .skip(HZ as usize)
            .map(|sample| {
                let world = sample.world_from_body.rotate([0.0, 0.0, -1.0]);
                // The body's forward axis rolls with the swing; in the world
                // frame it must stay level, which is what a locked horizon is.
                world[1].abs()
            })
            .fold(0.0f64, f64::max);
        assert!(
            worst.asin().to_degrees() < 0.5,
            "{} degrees",
            worst.asin().to_degrees()
        );
    }

    /// Yaw is not locked, it is high passed: a slow turn reaches the picture
    /// and an oscillation does not. Same amplitude in both, an order of
    /// magnitude apart in period.
    ///
    /// What reaches the picture is the heading the filter has **not**
    /// cancelled, which is the difference between a fully locked solve and a
    /// stabilized one. Reading the stabilized heading on its own says the
    /// opposite of what it looks like it says: a large one is an oscillation
    /// being taken out of the view, not one arriving in it.
    #[test]
    fn a_slow_turn_reaches_the_view_and_an_oscillation_does_not() {
        let yawing = |period: f64| {
            move |t: f64| {
                let rate = 30f64 * (t * 2.0 * PI / period).cos() * (2.0 * PI / period) / 2.0;
                ([0.0, rate, 0.0], [0.0, -1.0, 0.0])
            }
        };
        let reaches_the_view = |period: f64| {
            let track = track(period * 2.0, yawing(period));
            let locked = Filter {
                yaw_seconds: f64::INFINITY,
                ..Filter::default()
            }
            .solve(&track, Mat3::IDENTITY);
            let stabilized = Filter::default().solve(&track, Mat3::IDENTITY);

            let turned: Vec<f64> = locked
                .samples()
                .iter()
                .zip(stabilized.samples())
                .map(|(all, left)| {
                    wrap(all.world_from_body.heading() - left.world_from_body.heading())
                        .to_degrees()
                })
                .collect();
            turned.iter().fold(f64::MIN, |a, b| a.max(*b))
                - turned.iter().fold(f64::MAX, |a, b| a.min(*b))
        };

        // 15 degrees either side in both cases, so the amplitude is not what
        // tells them apart.
        let oscillation = reaches_the_view(1.0);
        let turn = reaches_the_view(40.0);
        assert!(
            oscillation < 3.0,
            "an oscillation swept the view {oscillation} degrees"
        );
        assert!(
            turn > 20.0,
            "a slow turn only swept the view {turn} degrees"
        );
    }

    /// The two limits of the yaw constant, which is what says the constant is
    /// the only thing choosing between them: no stabilization at all, and a
    /// view welded to the heading the file started on.
    #[test]
    fn the_yaw_constant_runs_from_following_to_locked() {
        let turning = || track(10.0, |_| ([0.0, 20.0, 0.0], [0.0, -1.0, 0.0]));
        let swept = |yaw_seconds| {
            let filter = Filter {
                yaw_seconds,
                ..Filter::default()
            };
            filter
                .solve(&turning(), Mat3::IDENTITY)
                .samples()
                .last()
                .unwrap()
                .world_from_body
                .heading()
                .to_degrees()
        };

        assert!(swept(0.0).abs() < 0.5, "{}", swept(0.0));
        assert!(
            (swept(f64::INFINITY) - 200.0).abs() < 2.0,
            "{}",
            swept(f64::INFINITY)
        );
    }

    /// The interpolation rolling shutter will call: between two samples, and
    /// clamped rather than empty outside the track.
    #[test]
    fn an_orientation_is_available_between_samples_and_outside_the_track() {
        let solved = gyro_only().solve(
            &track(1.0, |_| ([0.0, 0.0, 90.0], [0.0, -1.0, 0.0])),
            Mat3::IDENTITY,
        );

        // Half a sample interval in, half a sample interval of rotation. From
        // where the track starts, which is not the identity: this track's
        // accelerometer is pinned to a rolling body and the seed reads that.
        let half = solved.at(STEP_US / 2).angle_to(solved.at(0));
        assert!((half.to_degrees() - 90.0 / HZ as f64 / 2.0).abs() < 0.01);
        assert_eq!(solved.at(-1_000_000), solved.samples()[0].world_from_body);
        assert_eq!(
            solved.at(10_000_000),
            solved.samples().last().unwrap().world_from_body
        );
    }

    /// What rolling-shutter correction reads (issue #9): a body turning at a
    /// constant rate turns by rate times time, and the vector it comes back
    /// as points the way a world-fixed direction goes when seen from the body,
    /// which is the **opposite** way round from the body's own rotation.
    ///
    /// That sign is the one thing in the correction that cannot be checked by
    /// looking at a picture, and getting it backwards would double every
    /// displacement instead of removing it.
    #[test]
    fn a_turn_across_a_window_is_the_rotation_the_world_makes_in_it() {
        let solved = gyro_only().solve(
            &track(2.0, |_| ([0.0, 0.0, 90.0], [0.0, -1.0, 0.0])),
            Mat3::IDENTITY,
        );

        // A tenth of a second of 90 deg/s, about the body's forward axis.
        let turn = solved.turn(500_000, 600_000);
        assert!((turn[2] + 9f64.to_radians()).abs() < 1e-3, "{turn:?}");
        assert!(turn[0].abs() < 1e-6 && turn[1].abs() < 1e-6, "{turn:?}");
        // And it scales with the window rather than with anything else.
        let half = solved.turn(525_000, 575_000);
        assert!((half[2] - turn[2] / 2.0).abs() < 1e-4, "{half:?}");
        // Backwards through the same window is the same turn the other way.
        let back = solved.turn(600_000, 500_000);
        assert!((back[2] + turn[2]).abs() < 1e-6, "{back:?}");
    }

    /// And with no IMU record there is no turn, which is what switches the
    /// correction off on a file that carries none rather than erroring.
    #[test]
    fn a_file_with_no_imu_turns_by_nothing() {
        let solved = Filter::default().solve(&GyroTrack::default(), Mat3::IDENTITY);

        assert_eq!(solved.turn(-8_000, 8_000), [0.0; 3]);
    }

    #[test]
    fn a_file_with_no_imu_leaves_the_view_alone() {
        let solved = Filter::default().solve(&GyroTrack::default(), Mat3::IDENTITY);

        assert!(solved.is_empty());
        assert_eq!(solved.at(0), Quat::IDENTITY);
    }

    /// The axis convention, as a matrix. Read as "the letter in slot `i` is
    /// the sensor axis that feeds output `i`, negated if it is lower case".
    #[test]
    fn an_axis_map_permutes_and_negates_the_axes_it_names() {
        let x4 = axis_map("yzX");
        assert_eq!(x4.mul_vec([1.0, 2.0, 3.0]), [-2.0, -3.0, 1.0]);

        assert_eq!(axis_map("XYZ"), Mat3::IDENTITY);
        assert_eq!(axis_map("xyz").mul_vec([1.0, 2.0, 3.0]), [-1.0, -2.0, -3.0]);
        // An unknown letter drops its axis rather than aliasing another one.
        assert_eq!(axis_map("qYZ").mul_vec([1.0, 2.0, 3.0]), [0.0, 2.0, 3.0]);
    }

    /// A wrong convention is not a subtle error, and this is the arithmetic
    /// behind the negative control the harness runs on pixels: the string
    /// telemetry-parser falls through to for an X4 Air puts the same resting
    /// camera 90 degrees off level.
    #[test]
    fn the_wrong_axis_convention_puts_gravity_somewhere_else() {
        let resting = [0.0, -1.0, 0.0];
        let right = axis_map("yzX").mul_vec(resting);
        let wrong = axis_map("Xyz").mul_vec(resting);

        assert_eq!(right, [1.0, 0.0, 0.0]);
        assert_eq!(wrong, [0.0, 1.0, 0.0]);
        assert!(
            dot(right, wrong).abs() < 1e-12,
            "the two conventions have to disagree, or the control proves nothing"
        );
    }
}
