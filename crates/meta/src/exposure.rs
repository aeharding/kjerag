//! The per-lens shutter track: trailer records 4 and 12, kept apart.
//!
//! Each record is `{u64 timestamp, f64 shutter_seconds}` per sample, one
//! sample per frame, and there is one record per lens: 4 is lens 0 and 12
//! is lens 1. They carry the same shape and they are read separately,
//! because `telemetry-parser` reads both under one key and lets the second
//! overwrite the first, which is the bug that makes every consumer of that
//! crate blind to the difference between the two lenses
//! (docs/research/insv-format.md 6.3).
//!
//! **What this is not.** It is not a brightness signal. The two lenses run
//! independent auto-exposure loops that trade shutter against sensor gain
//! to reach the same picture brightness, so the ratio of the two shutters
//! measures how differently the two hemispheres are lit and not how
//! differently they came out. Measured over two 30-minute X4 Air captures:
//! the shutters differ by 34 to 59 percent while the two pictures of the
//! overlap band differ by 0.9 to 3.5 percent, with no correlation between
//! them (docs/research/insv-format.md 6.3). Nothing corrects exposure from
//! these numbers; issue #8 wants them for the clock.

use std::time::Duration;

/// `u64` timestamp then `f64` shutter, little endian like the rest of the
/// trailer.
const SAMPLE_LEN: usize = 8 + 8;
const MICROS_PER_SECOND: i64 = 1_000_000;

/// What one lens's shutter was at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExposureSample {
    /// Media time of the sample, relative to the file's first frame.
    /// Negative on the samples the camera writes before it commits that
    /// frame, which is 267 ms of them on the X4 Air fixture, which is why
    /// this is signed.
    pub offset_us: i64,
    /// Shutter time in seconds: 1/2717 s on a sunlit X4 Air frame, 1/154 s
    /// on an evening ONE X2 one.
    pub shutter_s: f64,
}

/// One lens's shutter over the whole file, in time order.
#[derive(Clone, Default, PartialEq)]
pub struct ExposureTrack {
    samples: Vec<ExposureSample>,
    /// Which sample is the file's first frame. The camera writes a handful
    /// before it commits one, 8 of them on the X4 Air captures measured, so
    /// this is not zero and the count is not the frame count.
    first_frame: usize,
}

/// Summarised rather than dumped: a 30-minute capture holds 54017 samples
/// and a reader wants the shape of them.
impl std::fmt::Debug for ExposureTrack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let shutters = self.samples.iter().map(|sample| sample.shutter_s);
        f.debug_struct("ExposureTrack")
            .field("samples", &self.samples.len())
            .field("first", &self.samples.first())
            .field("last", &self.samples.last())
            .field(
                "shortest_s",
                &shutters.clone().fold(f64::INFINITY, f64::min),
            )
            .field("longest_s", &shutters.fold(0.0f64, f64::max))
            .finish()
    }
}

impl ExposureTrack {
    /// One sample per frame of the file, or nothing at all when the file
    /// carries no record for this lens.
    pub fn samples(&self) -> &[ExposureSample] {
        &self.samples
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The shutter the camera was using at `time`, media time from the
    /// first frame.
    ///
    /// The nearest sample rather than an interpolation: the track carries
    /// one sample per frame and the question is always asked about a
    /// frame, so the nearest sample is that frame's own.
    pub fn shutter_at(&self, time: Duration) -> Option<f64> {
        let wanted = i64::try_from(time.as_micros()).unwrap_or(i64::MAX);
        let after = self
            .samples
            .partition_point(|sample| sample.offset_us < wanted);
        let window = self
            .samples
            .get(after.saturating_sub(1)..(after + 1).min(self.samples.len()))?;
        window
            .iter()
            .min_by_key(|sample| (sample.offset_us - wanted).abs())
            .map(|sample| sample.shutter_s)
    }

    /// When the camera itself says frame `index` was exposed, in media time
    /// from the first frame.
    ///
    /// **This is the file's authoritative frame clock, and the container's
    /// PTS is not** (`pts_type = 2`, `VideoPtsEexposureFile`). The two agree
    /// at the first frame and drift apart at 6.4 parts per million, which is
    /// the camera's real sensor clock against the container's nominal
    /// 30000/1001: measured over a 30-minute X4 Air capture, 11.5 ms by the
    /// end of it, a third of a frame. Aligning the gyro to this one is what
    /// keeps a fast roll at the end of a long file from tilting the horizon
    /// by the rate times that gap. docs/research/insv-format.md 8.6 has the
    /// table and what it costs to get it wrong.
    ///
    /// `None` for a file whose exposure record does not reach that frame,
    /// where the container's own PTS is the only clock there is.
    pub fn frame_time_us(&self, index: u64) -> Option<i64> {
        let at = self.first_frame.checked_add(usize::try_from(index).ok()?)?;
        Some(self.samples.get(at)?.offset_us)
    }

    /// Read one record's payload. A trailing part-sample is dropped: no
    /// capture has ever had one, and half a timestamp is not a sample.
    pub(crate) fn parse(payload: &[u8], clock: Clock) -> Self {
        let samples: Vec<ExposureSample> = payload
            .chunks_exact(SAMPLE_LEN)
            .map(|sample| ExposureSample {
                offset_us: clock.offset_us(u64::from_le_bytes(
                    sample[..8].try_into().expect("eight bytes"),
                )),
                shutter_s: f64::from_le_bytes(sample[8..].try_into().expect("eight bytes")),
            })
            .collect();
        Self {
            first_frame: samples
                .iter()
                .enumerate()
                .min_by_key(|(_, sample)| sample.offset_us.abs())
                .map_or(0, |(index, _)| index),
            samples,
        }
    }
}

/// The timebase the trailer's timestamps are in, which is not the same on
/// every camera.
///
/// `is_raw_gyro` selects it, which is the extra division by 1000 in the
/// clock chain of docs/research/insv-format.md 8.3 read as what it is: the
/// X4 Air sets that flag and writes microseconds, the ONE X2 does not and
/// writes milliseconds. `first_frame_timestamp` is in whichever of the two
/// the file uses, so it is subtracted before the scale rather than after.
///
/// Measured 2026-07-31, because the research note calls
/// `first_frame_timestamp` microseconds without qualifying it: on the X4
/// Air the field reads 3812440 against an exposure track running
/// 3545503 to 1805890895 for 1798 s of video, and on the ONE X2 it reads
/// 4254 against a track running 2930 to 265326 for 261 s. Only one reading
/// of each fits its own file.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Clock {
    pub ticks_per_second: i64,
    pub first_frame: i64,
}

impl Clock {
    pub(crate) fn offset_us(self, timestamp: u64) -> i64 {
        let ticks = i64::try_from(timestamp).unwrap_or(i64::MAX) - self.first_frame;
        match self.ticks_per_second {
            0 => 0,
            per_second => ticks.saturating_mul(MICROS_PER_SECOND) / per_second,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MICROSECONDS: Clock = Clock {
        ticks_per_second: MICROS_PER_SECOND,
        first_frame: 3_812_440,
    };

    /// The X4 Air's own numbers: a track that starts 267 ms before the
    /// first frame and steps one frame at a time.
    fn payload(start: u64, step: u64, shutters: &[f64]) -> Vec<u8> {
        shutters
            .iter()
            .enumerate()
            .flat_map(|(index, shutter)| {
                let timestamp = start + step * index as u64;
                [timestamp.to_le_bytes(), shutter.to_le_bytes()].concat()
            })
            .collect()
    }

    #[test]
    fn a_track_reads_its_samples_in_media_time() {
        let track = ExposureTrack::parse(
            &payload(3_545_503, 33_366, &[0.000_428, 0.000_431, 0.000_44]),
            MICROSECONDS,
        );

        assert_eq!(track.samples().len(), 3);
        // 267 ms before the first frame, then two frame intervals on.
        assert_eq!(track.samples()[0].offset_us, -266_937);
        assert_eq!(track.samples()[1].offset_us, -233_571);
        assert_eq!(track.samples()[0].shutter_s, 0.000_428);
    }

    /// The ONE X2 writes the same record in milliseconds, and its
    /// `first_frame_timestamp` is in milliseconds with it. Reading either
    /// as microseconds puts a four-minute capture in the first quarter
    /// second of itself.
    #[test]
    fn a_millisecond_camera_lands_on_the_same_media_time() {
        let clock = Clock {
            ticks_per_second: 1_000,
            first_frame: 4_254,
        };
        let track = ExposureTrack::parse(&payload(2_930, 33, &[0.006_5, 0.006_6]), clock);

        assert_eq!(track.samples()[0].offset_us, -1_324_000);
        assert_eq!(track.samples()[1].offset_us, -1_291_000);
    }

    #[test]
    fn the_shutter_at_a_frame_is_that_frames_own_sample() {
        let track = ExposureTrack::parse(
            &payload(3_812_440, 33_366, &[0.001, 0.002, 0.003, 0.004]),
            MICROSECONDS,
        );

        // Dead on the third sample, and a third of a frame either side of
        // it, which is as far as a frame's own time can be from it.
        for micros in [66_732 - 11_000, 66_732, 66_732 + 11_000] {
            assert_eq!(
                track.shutter_at(Duration::from_micros(micros)),
                Some(0.003),
                "{micros} us"
            );
        }
    }

    /// Before the first sample and past the last one the answer is the
    /// nearest sample, not nothing: a track that starts 267 ms late would
    /// otherwise leave the first eight frames unanswered.
    #[test]
    fn the_ends_of_a_track_clamp() {
        let track = ExposureTrack::parse(
            &payload(3_812_440, 33_366, &[0.001, 0.002, 0.003]),
            MICROSECONDS,
        );

        assert_eq!(track.shutter_at(Duration::ZERO), Some(0.001));
        assert_eq!(track.shutter_at(Duration::from_secs(600)), Some(0.003));
    }

    /// The frame clock: the track starts before the first frame, so frame 0
    /// is not sample 0, and reading it as sample 0 puts every frame's gyro
    /// lookup 267 ms early on this camera.
    #[test]
    fn frame_zero_is_the_sample_the_camera_committed_it_on() {
        // Eight samples of pre-roll, as the X4 Air writes, then the frames.
        let track = ExposureTrack::parse(&payload(3_545_503, 33_366, &[0.001; 20]), MICROSECONDS);

        assert_eq!(track.frame_time_us(0), Some(-9));
        assert_eq!(track.frame_time_us(1), Some(33_357));
        assert_eq!(track.frame_time_us(11), Some(367_017));
        // Past the end of the record there is no camera timestamp at all.
        assert_eq!(track.frame_time_us(12), None);
    }

    /// And the two clocks are compared frame by frame, which is what says
    /// the camera's is worth having: 30 frames of the container's nominal
    /// 1001/30000 against 30 of the camera's own slightly different rate.
    #[test]
    fn the_camera_clock_and_the_container_grid_drift_apart() {
        // 33 366.67 us is the container's frame; the camera writes 33 366.
        let track = ExposureTrack::parse(&payload(3_812_440, 33_366, &[0.001; 1000]), MICROSECONDS);
        let container = |frame: u64| (frame as f64 * 1_001.0 / 30_000.0 * 1e6) as i64;

        assert_eq!(track.frame_time_us(0), Some(0));
        let apart = track.frame_time_us(999).unwrap() - container(999);
        assert!(
            (-700..-600).contains(&apart),
            "{apart} us apart at frame 999"
        );
    }

    #[test]
    fn a_lens_with_no_record_answers_nothing() {
        let track = ExposureTrack::parse(&[], MICROSECONDS);

        assert!(track.is_empty());
        assert_eq!(track.shutter_at(Duration::ZERO), None);
    }

    /// A payload that is not a whole number of samples keeps the samples
    /// it does hold. Nothing has ever written one; the alternative is a
    /// panic on a file we have not seen.
    #[test]
    fn a_short_last_sample_is_dropped_rather_than_read() {
        let mut payload = payload(3_812_440, 33_366, &[0.001, 0.002]);
        payload.truncate(SAMPLE_LEN + 5);

        assert_eq!(
            ExposureTrack::parse(&payload, MICROSECONDS).samples().len(),
            1
        );
    }
}
