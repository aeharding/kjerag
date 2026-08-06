//! Where the camera was pointing, frame by frame: the gyroscope integrated
//! and held level by the accelerometer, heading and all.
//!
//! The output is one quaternion per IMU sample, `world_from_body`, taking a
//! direction in the camera body's frame to the stabilized world frame. Both
//! frames are the one the rest of Kjerag uses: **x right, y down, z forward**,
//! and in the world frame y is gravity. `kjerag-render` composes the inverse
//! into its view rotation, so a body that turns leaves the world where it was.
//!
//! ## What the lock holds still
//!
//! The world, all three axes of it (owner ruling, 2026-08-06). The view is
//! pointed at a direction in the world frame and stays pointed there while the
//! aircraft rolls, pitches and turns underneath it, which is what Insta360
//! Studio does and what the owner asked for by name. It replaces a design that
//! took roll and pitch out completely but let heading through a 3 s high pass,
//! so a deliberate turn carried the picture round with it: measured on the July
//! 14 capture, the locked view followed the aircraft at about 450 deg/min where
//! Studio's held to about 2.
//!
//! Nothing bounds the heading, and that is the price. The accelerometer
//! observes the vertical and says nothing about which way round the vertical
//! the body is pointing, so the heading is the gyroscope's alone and it carries
//! the gyroscope's bias: about 0.05 deg/s on this footage
//! (docs/research/insv-format.md 8.5), or 3 degrees of slow yaw a minute, which
//! accumulates over a flight and is never corrected. A magnetometer is what
//! would bound it; the trailer has a record type for one (13, `Magnetic`,
//! contents unknown) and none of the X4 Air or ONE X2 captures here carries a
//! single byte of it. Studio's own drift is the same order, so this is the
//! floor of the technique rather than a shortfall against it.
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
//! seconds to walk back.
//!
//! Putting the seed through the running filter's trust window fixed the
//! magnitudes and left the rest, because **a magnitude is not a direction**.
//! An aircraft accelerating along the ground tilts the specific force it feels
//! by `e` and weighs it `1 / cos e`, so the whole 0.05 g of the full-trust
//! window is spent by 18 degrees of tilt: the test cannot see the one error it
//! is standing in front of. The August 2 capture starts from a second of
//! accelerometer 1.9 s in that weighs 1.039 g, is therefore believed
//! completely, and points 21 degrees off vertical.
//!
//! So [`Filter::seed`] reads a mean rather than a reading. A minute of
//! accelerometer carried into one frame averages to gravity plus the
//! aircraft's own change in speed over that minute, and that is bounded by
//! flying rather than by luck: the manoeuvre that tilts one second of it is
//! pointing somewhere else in the next.

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
/// view to the heading the file starts on. The shipped filter is that last
/// limit, and the harness in `kjerag-spike` sweeps the rest of the range to
/// say what the heading follow it replaced was worth.
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
    /// cancelled. Infinite in the shipped filter, which is none of them.
    ///
    /// Kept as a number because the instruments sweep it and the number is
    /// how they say what a heading follow costs, not because anything in the
    /// app moves it. There was a finite value here until 2026-08-06, chosen
    /// so that a deliberate turn read as a turn and the swing of a camera
    /// under a wing did not, and the owner's ruling is that a deliberate turn
    /// must not read as a turn either: the world is what the lock holds.
    pub yaw_seconds: f64,
    /// How far from 1 g the accelerometer may read and still be believed
    /// completely, and how far before it is not believed at all. Between them
    /// the correction fades linearly.
    pub trust_g: (f64, f64),
}

impl Default for Filter {
    /// The shipped numbers. The three finite ones are measured on real X4 Air
    /// paramotor footage; the method and the tables are in
    /// docs/research/insv-format.md 8.5. The infinite one is a design ruling
    /// and no measurement chooses it: the heading is locked because holding
    /// the world still is what the lock is for.
    fn default() -> Self {
        Self {
            accel_seconds: 1.0,
            tilt_seconds: 20.0,
            yaw_seconds: f64::INFINITY,
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
    /// The middle of what was averaged, on the track's own clock. Half a
    /// minute in on any file long enough to fill [`SEED_MINUTE_US`], and
    /// earlier only on one that ends before it does, so what this reports is
    /// how much of the minute the file had.
    pub at_us: i64,
    /// What that mean reading weighed, in g.
    pub magnitude_g: f64,
    /// Whether the running filter would believe that mean completely.
    ///
    /// Not a selector: the seed is this mean whatever this says. It is the
    /// warning. A minute of accelerometer that averages to something which
    /// does not weigh what gravity weighs is a minute that never settled on
    /// gravity, and the horizon it starts on is the least bad answer rather
    /// than a good one.
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

/// How much of the opening of the track the seed is averaged over, in
/// microseconds.
///
/// The mean specific force over a stretch is gravity plus the aircraft's own
/// mean acceleration, and that is however much its speed changed divided by the
/// stretch: a minute of a paramotor's whole speed range, 15 m/s, is 0.025 g, or
/// one and a half degrees, and a manoeuvre inside the minute cancels itself
/// because it points somewhere different in the next one. One second of the
/// same signal is bounded by nothing of the kind. Against that, the gyroscope
/// has to carry the answer back to the start of the track, and at the 0.05
/// deg/s an X4 Air measures at rest (docs/research/insv-format.md 8.5) a minute
/// of carrying is 3 degrees.
///
/// A minute is where those two meet, and the six flights say the same: against
/// a backward pass over the same files, 40 seconds is worse than 60 on four of
/// them and 90 is worse on five. A finer sweep in review, over more spans than
/// `kjerag-spike --bin hindsight` prints, put the minimum at 60 on both its
/// worst case and its mean.
const SEED_MINUTE_US: i64 = 60_000_000;

impl Filter {
    /// The attitude to start the estimate from: the whole opening minute of
    /// accelerometer, carried back to the start of the track by the gyroscope
    /// and averaged.
    ///
    /// **Issue #45 is what the first version of this cost, and the August 2
    /// capture is what the second one cost.** The first took the opening tenth
    /// of a second whatever it read, which on the April 10 capture weighs
    /// 1.281 g. The second took the first second the running filter believed
    /// completely, which on the August 2 capture is a second of the launch
    /// weighing 1.039 g and pointing 21 degrees off vertical: it tested the
    /// magnitude of a reading and called that testing the reading. Both left a
    /// tilt every frame after them inherited, because the correction that has
    /// to undo a bad seed is the same slow one that exists to ignore turns.
    ///
    /// Three choices in here, and each is measured rather than assumed.
    ///
    /// - **A mean over [`SEED_MINUTE_US`], not a reading inside it.** A
    ///   magnitude test cannot see a horizontal acceleration, which is the one
    ///   that tilts the answer, so no test of one second can tell a launch
    ///   from gravity. What bounds a mean is flying: see [`SEED_MINUTE_US`].
    /// - **Every sample counted once.** Selecting by [`Filter::trust`] sounds
    ///   like the rule that covers every other sample. On the file the defect
    ///   was reported on it is worse, and worse the harder it selects: over
    ///   the August 2 capture's first forty seconds through the render path,
    ///   **3.75** degrees for counting every sample, 6.73 for weighting each
    ///   second by trust, 9.32 for keeping only the seconds inside the trust
    ///   window, 18.86 for the one window the old rule chose. The reading of
    ///   that which would explain it is that the moments of a manoeuvring
    ///   flight which weigh 1 g are the unloaded and transitional ones, and a
    ///   sample of manoeuvres leans. It is one file on one instrument, and the
    ///   backward pass of `kjerag-spike --bin hindsight` orders the first two
    ///   the other way round on the same file: what is settled across both
    ///   instruments and all six flights is this rule against the one it
    ///   replaced, not counting against weighting
    ///   (docs/research/insv-format.md 8.8).
    /// - **Every reading carried back before it is averaged.** The body turns
    ///   during the minute, so each sample is rotated into the frame of the
    ///   track's first sample by the gyroscope before it goes into the mean.
    ///   That makes the answer an attitude at the start of the track directly,
    ///   with no separate back-rotation and no assumption that the body held
    ///   still, and it is why a manoeuvre cancels: the force that leans one
    ///   second of it points somewhere else in this frame a second later.
    ///
    /// What it cannot do: a minute that holds one steady thing which is not
    /// gravity reads exactly like a minute of gravity, and only the weight of
    /// the answer gives it away. Sustained acceleration is what makes one, a
    /// minute is how much of it a flight can hold, and [`Seed::trusted`] is
    /// the warning. Nor can any one reading be refused any more: a reading the
    /// running filter would throw out reaches this mean, worth its own share
    /// of the minute and nothing more.
    ///
    /// `None` only for a track with no samples in it.
    pub fn seed(&self, track: &GyroTrack, body_from_imu: Mat3) -> Option<Seed> {
        let samples = track.samples();
        let first = samples.first()?;
        let mut turned = Quat::IDENTITY;
        let mut previous = first.offset_us;
        let mut opening = Mean::default();

        for sample in samples {
            if sample.offset_us - first.offset_us > SEED_MINUTE_US {
                break;
            }
            let dt = (sample.offset_us - previous).max(0) as f64 * 1e-6;
            previous = sample.offset_us;
            let rate = body_from_imu.mul_vec(sample.rate_dps);
            turned = turned
                .times(Quat::from_rotation_vector(
                    rate.map(|axis| axis.to_radians() * dt),
                ))
                .normalized();
            opening.add(turned.rotate(body_from_imu.mul_vec(sample.accel_g)));
        }
        // The middle of what was averaged, which for a sample rate that does
        // not change is where the weight of the answer sits.
        self.reading(opening.mean()?, (first.offset_us + previous) / 2)
    }

    /// One mean reading as something to start from, or `None` where the
    /// readings cancelled and there is no direction in them at all.
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

            // How far the frame the view is pointed in has been carried round
            // by the body: the low-passed heading, which is what gets taken
            // back off below. At the shipped `yaw_seconds` this stays at zero
            // and the frame is the world's, which is the whole of the lock.
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
#[derive(Clone, Copy, Default)]
struct Mean {
    sum: [f64; 3],
    count: f64,
}

impl Mean {
    fn add(&mut self, reading: [f64; 3]) {
        self.sum = std::array::from_fn(|axis| self.sum[axis] + reading[axis]);
        self.count += 1.0;
    }

    /// The mean of what went in. `None` where nothing did, which is a track
    /// with no samples in it.
    fn mean(self) -> Option<[f64; 3]> {
        match self.count > 0.0 {
            true => Some(self.sum.map(|axis| axis / self.count)),
            false => None,
        }
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
        // Ten minutes at 0.05 deg/s is 30 degrees, and the seed answers for
        // the middle of the minute it averaged rather than for the first
        // sample of it, so half a minute of the same drift, 1.5 degrees, is
        // already inside the answer it starts from.
        assert!(
            (last(&adrift) - 28.5).abs() < 0.5,
            "{} degrees adrift, against 28.5 predicted",
            last(&adrift)
        );
        let settled = Filter::default().tilt_seconds * 0.05;
        assert!(
            (last(&held) - settled).abs() < 0.1,
            "{} degrees held, against {settled} predicted",
            last(&held)
        );
    }

    /// And it settles rather than oscillating: a camera that really is rolled
    /// at the first frame is drawn rolled, and the estimate follows it level
    /// and stays there.
    ///
    /// Everything the accelerometer says here is something the gyroscope also
    /// says, which is what makes the answer checkable: the seed is read from
    /// the still stretch and carried back over the recorded roll, so the first
    /// frame has to come out at the 30 degrees the camera was really at.
    #[test]
    fn a_camera_that_starts_rolled_is_drawn_rolled_and_settles_level() {
        let filter = Filter {
            tilt_seconds: 1.0,
            ..Filter::default()
        };
        // Rolled 30 degrees and still, then rolled level at 60 deg/s over half
        // a second, then still. A positive rate about the forward axis takes
        // the roll off, which is why the accelerometer reads it counting down.
        let rolled = |degrees: f64| {
            let angle = degrees.to_radians();
            ([0.0; 3], [angle.sin(), -angle.cos(), 0.0])
        };
        let solved = filter.solve(
            &track(30.0, |t| match t {
                t if t < 2.0 => rolled(30.0),
                t if t < 2.5 => ([0.0, 0.0, 60.0], rolled(30.0 - 60.0 * (t - 2.0)).1),
                _ => resting(t),
            }),
            Mat3::IDENTITY,
        );

        let at = |seconds: f64| tilt_deg(solved.at((seconds * 1e6) as i64));
        assert!((at(0.0) - 30.0).abs() < 1.0, "{} degrees", at(0.0));
        assert!(at(10.0) < 0.5, "{} degrees", at(10.0));
        assert!(at(29.0) < 0.5, "{} degrees", at(29.0));
    }

    /// **Issue #45, and what a mean can and cannot promise about it.** No one
    /// reading can set the estimate any more, because the estimate starts from
    /// a mean of a minute; what a reading the running filter would refuse can
    /// still do is add its own share of that minute.
    ///
    /// The opening two seconds here weigh 1.34 g, which is the shape of the
    /// April 10 capture's first tenth of a second at 1.281 g, and they point
    /// 63 degrees off the body's own vertical. Two seconds of thirty is a
    /// fifteenth, and a fifteenth of 63 degrees is the 4.7 this comes out at,
    /// against the 48.9 the seed of issue #45 gave that file at 6 seconds. A
    /// launch of two seconds inside a whole minute, which is what a file has,
    /// is worth a degree and a half.
    #[test]
    fn a_reading_the_filter_refuses_is_worth_its_share_and_no_more() {
        let refused = [1.2, -0.6, 0.0];
        let filter = Filter::default();
        assert_eq!(
            filter.trust(norm(refused)),
            0.0,
            "{refused:?} has to be a reading the running filter refuses outright"
        );
        // What starting from that reading alone would have been worth, so the
        // test says what it is defending against rather than only that it
        // holds.
        let refuses = upright(refused.map(|axis| axis / norm(refused)))
            .angle_to(Quat::IDENTITY)
            .to_degrees();
        assert!(
            (refuses - 63.0).abs() < 0.5,
            "{refuses} degrees off vertical"
        );

        let track = track(30.0, |t| match t < 2.0 {
            true => ([0.0; 3], refused),
            false => resting(t),
        });
        let seed = filter.seed(&track, Mat3::IDENTITY).unwrap();
        assert!(seed.trusted, "{seed:?}");

        let solved = filter.solve(&track, Mat3::IDENTITY);
        let at = |seconds: f64| tilt_deg(solved.at((seconds * 1e6) as i64));
        assert!(
            (at(0.0) - 4.7).abs() < 0.5,
            "{} degrees at the first frame",
            at(0.0)
        );
        // It grows to 6.3 degrees by four seconds before it decays, because
        // the smoothed accelerometer crosses the trust window on its way from
        // that opening reading to gravity and is believed as it passes
        // through. That is the running filter and not the seed. What is left
        // then decays at `tilt_seconds` like anything else the correction has
        // to undo, where 49 degrees took a minute and a half of the April 10
        // capture.
        assert!(at(6.0) < 7.0, "{} degrees at 6 s", at(6.0));
        assert!(at(30.0) < 2.0 && at(30.0) < at(6.0), "{} at 30 s", at(30.0));
    }

    /// And every reading is carried back over whatever the body did before it.
    ///
    /// The body rolls a quarter turn over the first two seconds of this track
    /// and is still for the rest, and the accelerometer reads gravity
    /// throughout, so everything that goes into the mean has to be rotated
    /// into the frame the track started in before it can be averaged with
    /// anything else. Averaging the readings where they were taken would put
    /// the seed somewhere between the two attitudes and the first frame a long
    /// way from either.
    #[test]
    fn the_seed_is_carried_back_over_whatever_the_body_did_first() {
        // A positive rate about the forward axis takes the reading's roll
        // angle down, so two seconds at 45 deg/s from level ends at -90, which
        // is gravity along the body's own x axis.
        let rolled = |degrees: f64| {
            let angle = degrees.to_radians();
            [angle.sin(), -angle.cos(), 0.0]
        };
        let solved = Filter::default().solve(
            &track(30.0, |t| match t < 2.0 {
                true => ([0.0, 0.0, 45.0], rolled(-45.0 * t)),
                false => ([0.0; 3], [-1.0, 0.0, 0.0]),
            }),
            Mat3::IDENTITY,
        );

        // The body was level when the track started, whatever it did next, and
        // a quarter turn from level by the time it stopped rolling. Read as
        // tilt rather than as a whole rotation because the heading half of the
        // filter has its own opinion about a body this far over and it is not
        // what this test is about.
        let at = |seconds: f64| tilt_deg(solved.at((seconds * 1e6) as i64));
        assert!(at(0.0) < 1.0, "{} degrees at the first frame", at(0.0));
        // Two degrees of the quarter turn go missing because the smoothed
        // accelerometer cannot follow a body rolling at 45 deg/s: it lags, the
        // lagged reading still weighs 1 g, and the correction leans on it.
        // That is the running filter meeting a manoeuvre no paraglider makes,
        // and it decays afterwards like everything else.
        assert!((at(3.0) - 90.0).abs() < 3.0, "{} degrees at 3 s", at(3.0));
        assert!(
            (at(29.0) - 90.0).abs() < 1.0,
            "{} degrees at 29 s",
            at(29.0)
        );
    }

    /// **The August 2 capture.** A launch weighs what gravity weighs and does
    /// not point where gravity points, and the seed may not start from it.
    ///
    /// The opening here is that file's own numbers: an aircraft accelerating
    /// along the ground reads 1.038 g, which the running filter believes
    /// completely, 21 degrees off its own vertical. The rule this replaced
    /// took the first second it believed, and that is the second, so the whole
    /// 21 degrees reached the estimate. Averaging cannot do that: four seconds
    /// of launch in forty is a tenth of the mean, and a tenth of 21 degrees is
    /// the 2.1 this comes out at.
    #[test]
    fn a_launch_that_weighs_a_gravity_it_is_not_does_not_start_the_estimate() {
        let filter = Filter::default();
        // Accelerating forward at 0.37 g, and unloaded enough to weigh 1.038.
        let launching = [0.0, -0.97, 0.37];
        assert_eq!(
            filter.trust(norm(launching)),
            1.0,
            "{} g has to be a magnitude the filter believes completely, or this \
             test is not about the defect",
            norm(launching)
        );
        let leans = upright(launching.map(|axis| axis / norm(launching)))
            .angle_to(Quat::IDENTITY)
            .to_degrees();
        assert!((leans - 21.0).abs() < 0.5, "{leans} degrees off vertical");

        let track = track(40.0, |t| match t < 4.0 {
            true => ([0.0; 3], launching),
            false => resting(t),
        });
        let seed = filter.seed(&track, Mat3::IDENTITY).unwrap();

        assert!(seed.trusted, "{seed:?}");
        assert!(
            (seed.world_from_body.angle_to(Quat::IDENTITY).to_degrees() - 2.1).abs() < 0.5,
            "{seed:?}"
        );
        let solved = filter.solve(&track, Mat3::IDENTITY);
        let at = |seconds: f64| tilt_deg(solved.at((seconds * 1e6) as i64));
        assert!(at(0.0) < 3.0, "{} degrees at the first frame", at(0.0));
        // And it is inside a degree and a half by the end, where the 21 the old
        // rule started from outlived the whole opening of the file. It is not
        // zero because the running filter believes 1.038 g as well and leans
        // towards the launch for as long as the launch lasts, which is the
        // correction working as designed and bounded by `tilt_seconds`.
        assert!(at(40.0) < 1.5, "{} degrees at 40 s", at(40.0));
    }

    /// A file that never reads gravity gets the mean of what there is, said
    /// out loud, and not a panic.
    ///
    /// The mean is always taken, so what makes this case a case is that the
    /// answer does not weigh what gravity weighs, and [`Seed::trusted`] is the
    /// only thing that can say so. A sensor reading a scale it was not
    /// calibrated for is what gets here. A flight is not, because the mean of
    /// a minute of flying is gravity plus however much the aircraft's speed
    /// changed.
    #[test]
    fn a_file_that_never_reads_gravity_takes_the_mean_of_what_there_is() {
        let filter = Filter::default();
        // Everything weighs too much: 25 seconds at 1.6 g along the body's own
        // vertical, then 35 at 1.26 g pointing 19 degrees off it.
        let track = track(60.0, |t| match t < 25.0 {
            true => ([0.0; 3], [0.0, -1.6, 0.0]),
            false => ([0.0; 3], [0.42, -1.19, 0.0]),
        });

        let seed = filter.seed(&track, Mat3::IDENTITY).unwrap();
        assert!(!seed.trusted, "{seed:?}");
        // The mean of the two stretches by their lengths, which is what taking
        // everything equally means, rather than either of them.
        assert!((seed.magnitude_g - 1.383).abs() < 0.01, "{seed:?}");
        assert!((25_000_000..35_000_000).contains(&seed.at_us), "{seed:?}");
        assert!(
            (seed.world_from_body.angle_to(Quat::IDENTITY).to_degrees() - 10.2).abs() < 0.5,
            "{seed:?}"
        );
        assert!(!filter.solve(&track, Mat3::IDENTITY).is_empty());
    }

    /// Nothing past the first minute reaches the seed.
    ///
    /// The gyroscope carries the answer back to the start of the track, and a
    /// reading fetched from further than [`SEED_MINUTE_US`] costs more of its
    /// drift than a nearer one costs in accuracy. This track only reads
    /// gravity at 90 seconds and the average stops long before it, so what the
    /// seed comes out at is what the first minute weighed and not what the
    /// file has later.
    #[test]
    fn nothing_past_the_first_minute_reaches_the_seed() {
        let seed = Filter::default()
            .seed(
                &track(120.0, |t| match t < 90.0 {
                    true => ([0.0; 3], [0.0, -1.4, 0.0]),
                    false => resting(t),
                }),
                Mat3::IDENTITY,
            )
            .unwrap();

        assert!(!seed.trusted, "{seed:?}");
        assert!((seed.magnitude_g - 1.4).abs() < 0.01, "{seed:?}");
        assert!(seed.at_us < SEED_MINUTE_US, "{seed:?}");
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

    /// Yaw is locked and not high passed: a slow turn does not reach the
    /// picture and neither does an oscillation. Same amplitude in both, an
    /// order of magnitude apart in period, which is what says the answer is
    /// no longer a question about how fast the aircraft turned.
    ///
    /// What reaches the picture is how far the frame the view is pointed in
    /// was carried round by the body, which is the body's own heading less
    /// the heading the stored orientation kept. This track's heading is known
    /// in closed form, so that is what it is measured against, rather than
    /// against a second run of the same filter that could be wrong the same
    /// way. The third case is the design this replaced: **the measurement can
    /// see a heading follow**, because at the 3 s constant that shipped until
    /// 2026-08-06 the slow turn carried the view almost the whole way round
    /// with the aircraft.
    #[test]
    fn neither_a_slow_turn_nor_an_oscillation_reaches_the_view() {
        // 15 degrees either side of where it started, whatever the period,
        // so the amplitude is not what tells the cases apart.
        const SWING: f64 = 15.0;
        let yawing = |period: f64| {
            move |t: f64| {
                let rate = SWING * (t * 2.0 * PI / period).cos() * (2.0 * PI / period);
                ([0.0, rate, 0.0], [0.0, -1.0, 0.0])
            }
        };
        let reaches_the_view = |filter: Filter, period: f64| {
            let solved = filter.solve(&track(period * 2.0, yawing(period)), Mat3::IDENTITY);
            let carried: Vec<f64> = solved
                .samples()
                .iter()
                .map(|sample| {
                    let seconds = sample.offset_us as f64 * 1e-6;
                    let body = SWING * (seconds * 2.0 * PI / period).sin();
                    wrap(body.to_radians() - sample.world_from_body.heading()).to_degrees()
                })
                .collect();
            carried.iter().fold(f64::MIN, |a, b| a.max(*b))
                - carried.iter().fold(f64::MAX, |a, b| a.min(*b))
        };
        let followed = Filter {
            yaw_seconds: 3.0,
            ..Filter::default()
        };

        let oscillation = reaches_the_view(Filter::default(), 1.0);
        let turn = reaches_the_view(Filter::default(), 40.0);
        let control = reaches_the_view(followed, 40.0);
        // Half a sample interval of the integrator's own lag is what is left
        // of either: 0.47 degrees at the oscillation's 94 deg/s and 0.01 at
        // the slow turn's 2.4, against 27 for the follow.
        assert!(
            oscillation < 1.0,
            "an oscillation swept the view {oscillation} degrees"
        );
        assert!(turn < 1.0, "a slow turn swept the view {turn} degrees");
        assert!(
            control > 20.0,
            "the follow this replaced only swept the view {control} degrees, \
             so this test cannot see one"
        );
    }

    /// The two limits of the yaw constant, which is what says the constant is
    /// the only thing choosing between them: no stabilization at all, and a
    /// view welded to the heading the file started on. The second is what
    /// [`Filter::default`] ships, stated here as an angle rather than as an
    /// infinity, so a build that quietly stopped locking fails here too.
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
