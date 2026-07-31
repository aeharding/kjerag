//! The IMU track: trailer record 3, in the sensor's own axes.
//!
//! Two encodings, selected by `is_raw_gyro`, and the same three-and-three
//! layout in both: **accelerometer first, gyroscope second**
//! (docs/research/insv-format.md 8.2). The raw form is 20 bytes per sample,
//! six `u16` biased by 32768, and the scale is the full-scale range the file
//! records rather than any default: the X4 Air runs the accelerometer at
//! +/-32 g, which is not the +/-16 g telemetry-parser falls back to. The
//! scaled form is 56 bytes, six `f64`, accelerometer already in g and
//! gyroscope in rad/s.
//!
//! Nothing here rotates anything. What comes out is what the sensor wrote,
//! and `super::orientation` is where it is turned into the camera body's
//! frame, because that step is a choice with evidence behind it and this one
//! is not.

use super::calibration::GyroEncoding;
use super::exposure::Clock;

/// `u64` timestamp, then six `u16` biased by 32768.
const RAW_SAMPLE_LEN: usize = 8 + 6 * 2;
/// `u64` timestamp, then six `f64`.
const SCALED_SAMPLE_LEN: usize = 8 + 6 * 8;
/// Half of a `u16`'s range, which is what a full-scale range is written
/// across.
const RAW_FULL_SCALE: f64 = 32768.0;

/// One instant of the IMU, in the sensor's own axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GyroSample {
    /// Media time of the sample, relative to the file's first frame. Signed
    /// for the same reason the exposure track's is: the camera writes samples
    /// before it commits the first frame.
    pub offset_us: i64,
    /// Angular rate in degrees per second.
    pub rate_dps: [f64; 3],
    /// Specific force in g. At rest this is 1 g pointing **up**, because an
    /// accelerometer measures the force holding it up rather than the gravity
    /// pulling it down.
    pub accel_g: [f64; 3],
}

/// The whole IMU track of one file, in time order.
#[derive(Clone, Default, PartialEq)]
pub struct GyroTrack {
    samples: Vec<GyroSample>,
}

/// Summarised rather than dumped: a 30-minute capture holds hundreds of
/// thousands of samples and a reader wants the shape of them.
impl std::fmt::Debug for GyroTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GyroTrack")
            .field("samples", &self.samples.len())
            .field("rate_hz", &self.rate_hz())
            .field("first", &self.samples.first())
            .field("last", &self.samples.last())
            .finish()
    }
}

impl GyroTrack {
    pub fn samples(&self) -> &[GyroSample] {
        &self.samples
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// How fast the IMU ran, from the samples rather than from any field: the
    /// trailer does not record it.
    pub fn rate_hz(&self) -> f64 {
        let (first, last) = (self.samples.first(), self.samples.last());
        match (first, last) {
            (Some(first), Some(last)) if last.offset_us > first.offset_us => {
                (self.samples.len() - 1) as f64 * 1e6 / (last.offset_us - first.offset_us) as f64
            }
            _ => 0.0,
        }
    }

    /// A track built from samples rather than from bytes, which is how a test
    /// or an instrument states a motion instead of encoding one. What it
    /// builds is integrated by exactly the path a recorded track takes, which
    /// is the point of having it: a synthetic roll reaches the pass the way a
    /// flown one does.
    pub fn from_samples(samples: Vec<GyroSample>) -> Self {
        Self { samples }
    }

    /// Read record 3's payload.
    ///
    /// `offset_us` is the timestamp put through the trailer's own clock and
    /// then shifted by `gyro_timestamp`, which is the last line of the chain
    /// in docs/research/insv-format.md 8.3. A trailing part-sample is dropped;
    /// no capture has ever had one.
    pub(crate) fn parse(
        payload: &[u8],
        encoding: GyroEncoding,
        clock: Clock,
        offset_us: i64,
    ) -> Self {
        let read = |sample: &[u8]| GyroSample {
            offset_us: clock.offset_us(u64::from_le_bytes(
                sample[..8].try_into().expect("eight bytes"),
            )) - offset_us,
            ..triples(&sample[8..], encoding)
        };
        Self {
            samples: match encoding {
                GyroEncoding::Raw { .. } => {
                    payload.chunks_exact(RAW_SAMPLE_LEN).map(read).collect()
                }
                GyroEncoding::Scaled => payload.chunks_exact(SCALED_SAMPLE_LEN).map(read).collect(),
            },
        }
    }
}

/// The six numbers after the timestamp, accelerometer first.
fn triples(body: &[u8], encoding: GyroEncoding) -> GyroSample {
    let (accel_g, rate_dps) = match encoding {
        GyroEncoding::Raw {
            accel_range_g,
            gyro_range_dps,
        } => {
            let biased = |index: usize, range: f64| {
                let at = index * 2;
                let raw = u16::from_le_bytes(body[at..at + 2].try_into().expect("two bytes"));
                (f64::from(raw) - RAW_FULL_SCALE) * range / RAW_FULL_SCALE
            };
            (
                std::array::from_fn(|axis| biased(axis, accel_range_g)),
                std::array::from_fn(|axis| biased(axis + 3, gyro_range_dps)),
            )
        }
        GyroEncoding::Scaled => {
            let double = |index: usize| {
                let at = index * 8;
                f64::from_le_bytes(body[at..at + 8].try_into().expect("eight bytes"))
            };
            (
                std::array::from_fn(&double),
                std::array::from_fn(|axis| double(axis + 3).to_degrees()),
            )
        }
    };
    GyroSample {
        offset_us: 0,
        rate_dps,
        accel_g,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RANGES: GyroEncoding = GyroEncoding::Raw {
        accel_range_g: 32.0,
        gyro_range_dps: 2000.0,
    };
    /// The X4 Air's: microseconds, and a first frame 3.85 s in.
    const MICROSECONDS: Clock = Clock {
        ticks_per_second: 1_000_000,
        first_frame: 3_848_400,
    };

    /// One raw sample as the camera writes it: the counts that mean these
    /// physical values, so the test states the physics and the code does the
    /// arithmetic.
    fn raw(timestamp: u64, accel_g: [f64; 3], rate_dps: [f64; 3]) -> Vec<u8> {
        let count = |value: f64, range: f64| {
            ((value * RAW_FULL_SCALE / range) + RAW_FULL_SCALE).round() as u16
        };
        let mut out = timestamp.to_le_bytes().to_vec();
        for value in accel_g {
            out.extend(count(value, 32.0).to_le_bytes());
        }
        for value in rate_dps {
            out.extend(count(value, 2000.0).to_le_bytes());
        }
        out
    }

    fn scaled(timestamp: u64, accel_g: [f64; 3], rate_dps: [f64; 3]) -> Vec<u8> {
        let mut out = timestamp.to_le_bytes().to_vec();
        for value in accel_g {
            out.extend(value.to_le_bytes());
        }
        for value in rate_dps {
            out.extend(value.to_radians().to_le_bytes());
        }
        out
    }

    #[track_caller]
    fn near(actual: [f64; 3], expected: [f64; 3], tolerance: f64) {
        for axis in 0..3 {
            assert!(
                (actual[axis] - expected[axis]).abs() <= tolerance,
                "{actual:?} is not within {tolerance} of {expected:?}"
            );
        }
    }

    /// The headline round trip: a sample written at the fixture's ranges reads
    /// back as the physical values it was written from. The tolerance is one
    /// count, which is 1 mg and 0.061 deg/s at +/-32 g and +/-2000 dps.
    #[test]
    fn a_raw_sample_reads_back_the_values_it_was_written_from() {
        let payload = raw(3_848_400, [0.02, -1.0, 0.03], [12.5, -240.0, 0.5]);
        let track = GyroTrack::parse(&payload, RANGES, MICROSECONDS, 0);

        let sample = track.samples()[0];
        assert_eq!(sample.offset_us, 0);
        near(sample.accel_g, [0.02, -1.0, 0.03], 0.001);
        near(sample.rate_dps, [12.5, -240.0, 0.5], 0.07);
    }

    /// The order is accelerometer first. Reading the two triples the other way
    /// round is a gyro that reads 1 g of rate and an accelerometer that reads
    /// hundreds of g, and both look plausible enough in isolation to ship.
    #[test]
    fn the_accelerometer_triple_comes_first() {
        let payload = raw(3_848_400, [0.0, -1.0, 0.0], [0.0, 0.0, 300.0]);
        let sample = GyroTrack::parse(&payload, RANGES, MICROSECONDS, 0).samples()[0];

        assert!(sample.accel_g[1] < -0.9, "{sample:?}");
        assert!(sample.rate_dps[2] > 290.0, "{sample:?}");
    }

    /// The ranges are read, not assumed. The same bytes at
    /// telemetry-parser's +/-16 g fallback read half the acceleration, which
    /// is a level camera that thinks it is falling.
    #[test]
    fn the_scale_is_the_range_the_file_records() {
        let payload = raw(3_848_400, [0.0, -1.0, 0.0], [0.0; 3]);
        let halved = GyroEncoding::Raw {
            accel_range_g: 16.0,
            gyro_range_dps: 2000.0,
        };

        let sample = GyroTrack::parse(&payload, halved, MICROSECONDS, 0).samples()[0];
        near(sample.accel_g, [0.0, -0.5, 0.0], 0.001);
    }

    /// The other encoding, which the ONE X2 writes: 56 bytes, no bias, no
    /// scale, and a gyroscope in rad/s.
    #[test]
    fn a_scaled_sample_reads_the_same_physical_values() {
        let clock = Clock {
            ticks_per_second: 1_000,
            first_frame: 4_254,
        };
        let payload = scaled(4_254, [0.02, -1.0, 0.03], [12.5, -240.0, 0.5]);

        let sample = GyroTrack::parse(&payload, GyroEncoding::Scaled, clock, 0).samples()[0];
        assert_eq!(sample.offset_us, 0);
        near(sample.accel_g, [0.02, -1.0, 0.03], 1e-12);
        near(sample.rate_dps, [12.5, -240.0, 0.5], 1e-9);
    }

    /// The two encodings are two sample lengths, and reading one as the other
    /// is not a parse error, it is a track of noise at the wrong rate. The
    /// same 400 bytes read as 20 samples of 20 bytes and 7 of 56.
    #[test]
    fn the_encoding_decides_how_long_a_sample_is() {
        let payload = vec![0u8; 400];

        assert_eq!(
            GyroTrack::parse(&payload, RANGES, MICROSECONDS, 0)
                .samples()
                .len(),
            20
        );
        assert_eq!(
            GyroTrack::parse(&payload, GyroEncoding::Scaled, MICROSECONDS, 0)
                .samples()
                .len(),
            7
        );
    }

    /// The clock, and the last line of it: `gyro_timestamp` shifts the whole
    /// track against the video.
    #[test]
    fn the_track_is_in_media_time_and_the_gyro_offset_shifts_it() {
        let payload: Vec<u8> = (0..4)
            .flat_map(|index| raw(3_848_400 + 1_000 * index, [0.0; 3], [0.0; 3]))
            .collect();

        let track = GyroTrack::parse(&payload, RANGES, MICROSECONDS, 0);
        assert_eq!(track.samples()[0].offset_us, 0);
        assert_eq!(track.samples()[3].offset_us, 3_000);
        assert!((track.rate_hz() - 1000.0).abs() < 1e-9);

        // `gyro_timestamp` of 1.6, read as milliseconds.
        let shifted = GyroTrack::parse(&payload, RANGES, MICROSECONDS, 1_600);
        assert_eq!(shifted.samples()[0].offset_us, -1_600);
    }

    #[test]
    fn a_short_last_sample_is_dropped_rather_than_read() {
        let mut payload = raw(3_848_400, [0.0; 3], [0.0; 3]);
        payload.extend(&raw(3_849_400, [0.0; 3], [0.0; 3])[..9]);

        assert_eq!(
            GyroTrack::parse(&payload, RANGES, MICROSECONDS, 0)
                .samples()
                .len(),
            1
        );
    }

    #[test]
    fn a_file_with_no_gyro_record_answers_nothing() {
        let track = GyroTrack::parse(&[], RANGES, MICROSECONDS, 0);

        assert!(track.is_empty());
        assert_eq!(track.rate_hz(), 0.0);
    }
}
