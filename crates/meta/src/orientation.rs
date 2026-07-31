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
    ///
    /// What it follows is the whole stabilized frame, and the view's own yaw
    /// is read in that frame, so this constant does not only decide how much
    /// of a turn reaches the picture: it decides that the body's heading owns
    /// the view. That is issue #44, and the answer is not here. A view a drag
    /// has taken hold of pins the follow where it found it
    /// ([`OrientationTrack::follow`]), so the two requirements stop competing
    /// for one number.
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

/// Where the camera body was, at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrientationSample {
    /// Media time, relative to the file's first frame, on the same clock the
    /// gyro track is read onto.
    pub offset_us: i64,
    /// Takes a direction in the camera body's frame to the stabilized world
    /// frame.
    pub world_from_body: Quat,
    /// How far round the world vertical the heading follow has carried that
    /// stabilized frame by this instant, in radians.
    ///
    /// Kept rather than discarded because the frame it turns is the one the
    /// **view's** own yaw is read in, so a view that is to stay where a drag
    /// left it has to take this back off (issue #44). It accumulates rather
    /// than wrapping, so a file with three turns in it reads three turns and
    /// interpolating across the back of the compass is a small step.
    pub heading_held: f64,
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
        let Some((from, to, t)) = self.between(offset_us) else {
            return Quat::IDENTITY;
        };
        let (previous, next) = (
            self.samples[from].world_from_body,
            self.samples[to].world_from_body,
        );
        // An instant on or outside a sample is that sample rather than a mix
        // of it with itself: an nlerp renormalizes, and the ends of a track
        // are read for equality.
        match from == to {
            true => previous,
            false => previous.nlerp(next, t),
        }
    }

    /// How far round the world vertical the heading follow has carried the
    /// stabilized frame at `offset_us` ([`OrientationSample::heading_held`]).
    ///
    /// The view's own yaw turns about that same vertical and is read in that
    /// same frame, so this is exactly what a view that has to stay put in the
    /// world takes back off (issue #44). Zero for a file with no IMU record,
    /// which leaves such a file's view where it always was.
    pub fn follow(&self, offset_us: i64) -> f64 {
        let Some((from, to, t)) = self.between(offset_us) else {
            return 0.0;
        };
        let (previous, next) = (
            self.samples[from].heading_held,
            self.samples[to].heading_held,
        );
        previous + (next - previous) * t
    }

    /// The samples `offset_us` falls between and how far between them it is,
    /// clamped to one sample twice over at both ends of the track. `None`
    /// where there are no samples at all.
    fn between(&self, offset_us: i64) -> Option<(usize, usize, f64)> {
        let after = self
            .samples
            .partition_point(|sample| sample.offset_us < offset_us);
        let Some(next) = self.samples.get(after) else {
            let last = self.samples.len().checked_sub(1)?;
            return Some((last, last, 0.0));
        };
        let Some(previous) = after.checked_sub(1) else {
            return Some((after, after, 0.0));
        };
        let span = (next.offset_us - self.samples[previous].offset_us) as f64;
        let t = match span > 0.0 {
            true => (offset_us - self.samples[previous].offset_us) as f64 / span,
            false => 0.0,
        };
        Some((previous, after, t))
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

impl Filter {
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

        let mut world_from_body = level(samples, body_from_imu);
        let mut gravity = world_from_body.conjugate().rotate(UP_IN_WORLD);
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
                    heading_held,
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

/// The orientation to start from: whichever tilt puts the first tenth of a
/// second of accelerometer readings on the world vertical, and heading zero.
///
/// Averaged rather than taken from one sample because one sample of an
/// airframe's accelerometer is mostly vibration, and because the estimate
/// starts here and the time constant is long.
fn level(samples: &[super::gyro::GyroSample], body_from_imu: Mat3) -> Quat {
    const SETTLE_US: i64 = 100_000;

    let until = samples[0].offset_us + SETTLE_US;
    let mut mean = [0.0; 3];
    let mut count = 0.0;
    for sample in samples.iter().take_while(|s| s.offset_us <= until) {
        let accel = body_from_imu.mul_vec(sample.accel_g);
        mean = std::array::from_fn(|axis| mean[axis] + accel[axis]);
        count += 1.0;
    }
    let length = norm(mean);
    if count == 0.0 || length == 0.0 {
        return Quat::IDENTITY;
    }

    // The shortest rotation from where the reading says up is to where the
    // world says it is. A camera exactly upside down has a whole circle of
    // shortest rotations and no reason to prefer one; nothing else does.
    let up = mean.map(|axis| axis / length);
    let axis = cross(up, UP_IN_WORLD);
    let angle = dot(up, UP_IN_WORLD).clamp(-1.0, 1.0).acos();
    match norm(axis) > 1e-9 {
        true => Quat::from_rotation_vector(axis.map(|c| c * angle / norm(axis))),
        false => Quat::from_rotation_vector([angle, 0.0, 0.0]),
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
    #[test]
    fn a_constant_rate_integrates_to_the_angle_it_should() {
        let solved = gyro_only().solve(
            &track(2.0, |_| ([0.0, 0.0, 90.0], [0.0, -1.0, 0.0])),
            Mat3::IDENTITY,
        );

        let end = solved.samples().last().unwrap().world_from_body;
        // Two seconds at 90 deg/s, less the one sample interval the track
        // stops short of it.
        let expected = Quat::from_rotation_vector([0.0, 0.0, (2.0 - 1.0 / HZ as f64) * PI / 2.0]);
        assert!(
            end.angle_to(expected).to_degrees() < 0.01,
            "{:?}",
            end.angle_to(expected).to_degrees()
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
    #[test]
    fn a_level_camera_that_starts_wrong_is_pulled_level() {
        let filter = Filter {
            tilt_seconds: 1.0,
            ..Filter::default()
        };
        // The accelerometer disagrees with the attitude the track starts on by
        // 30 degrees, because the first tenth of a second reads tilted.
        let solved = filter.solve(
            &track(20.0, |t| match t < 0.1 {
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

    /// What a held view takes back off (issue #44): the heading the follow
    /// has caught up with, which is the body's own heading low passed and
    /// **not** the body's own heading.
    ///
    /// A first-order lag on a ramp settles one time constant behind it, so a
    /// steady 20 deg/s and a 3 second constant put the follow 60 degrees back.
    /// That gap is the part of the turn that has reached the picture.
    #[test]
    fn the_follow_is_the_heading_the_filter_has_caught_up_with() {
        let turning = track(30.0, |_| ([0.0, 20.0, 0.0], [0.0, -1.0, 0.0]));
        let solved = Filter::default().solve(&turning, Mat3::IDENTITY);

        let at = |seconds: f64| solved.follow((seconds * 1e6) as i64).to_degrees();
        assert!((at(30.0) - (600.0 - 60.0)).abs() < 5.0, "{}", at(30.0));
        // It accumulates rather than wrapping, or a body that turns twice
        // would read as one that turned back.
        assert!(at(30.0) > 360.0, "{}", at(30.0));
        assert!(at(10.0) < at(20.0) && at(20.0) < at(30.0));
        // And it is read between the stored samples, like the orientation it
        // belongs to: a fifth of the way in is a fifth of the way along.
        let step = STORE_US as f64 * 1e-6;
        let between = solved.follow((STORE_US as f64 * 1.2) as i64).to_degrees();
        assert!(
            (between - (at(step) + 0.2 * (at(2.0 * step) - at(step)))).abs() < 1e-9,
            "{between}"
        );
    }

    #[test]
    fn a_file_with_no_imu_follows_nothing() {
        let solved = Filter::default().solve(&GyroTrack::default(), Mat3::IDENTITY);

        assert_eq!(solved.follow(0), 0.0);
        assert_eq!(solved.follow(8_000_000), 0.0);
    }

    /// The interpolation rolling shutter will call: between two samples, and
    /// clamped rather than empty outside the track.
    #[test]
    fn an_orientation_is_available_between_samples_and_outside_the_track() {
        let solved = gyro_only().solve(
            &track(1.0, |_| ([0.0, 0.0, 90.0], [0.0, -1.0, 0.0])),
            Mat3::IDENTITY,
        );

        // Half a sample interval in, half a sample interval of rotation.
        let half = solved.at(STEP_US / 2);
        assert!((half.angle_to(Quat::IDENTITY).to_degrees() - 90.0 / HZ as f64 / 2.0).abs() < 0.01);
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
