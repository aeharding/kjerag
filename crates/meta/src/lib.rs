//! Everything Kyerag reads out of an `.insv` file that is not pixels.
//!
//! The lens calibration, the two lenses' shutter tracks, and the IMU: its
//! samples as the sensor wrote them, and the orientation those samples
//! integrate to, which is what holds the horizon still (issue #8). No UI and
//! no ffmpeg: this layer needs a file handle and the last few megabytes of
//! the file.
//!
//! ```no_run
//! let calibration = kyerag_meta::CalibrationSet::from_insv("VID.insv")?;
//! let horizon = calibration.orientation(kyerag_meta::Filter::default());
//! # Ok::<(), kyerag_meta::Error>(())
//! ```
//!
//! The trailer format, the `offset_v3` grammar, the clock chain and the
//! provenance of every number quoted in these doc comments are in
//! `docs/research/insv-format.md`.

mod calibration;
mod exposure;
mod gyro;
mod orientation;
mod rotation;
mod trailer;

pub use calibration::{
    CalibrationSet, Distortion, GyroConfig, GyroEncoding, Intrinsics, Lens, Pose, Readout, Size,
    Sweep,
};
pub use exposure::{ExposureSample, ExposureTrack};
pub use gyro::{GyroSample, GyroTrack};
pub use orientation::{Filter, OrientationSample, OrientationTrack, Seed, axis_map, body_from_imu};
pub use rotation::{Mat3, Quat};
pub use trailer::record_index;

/// Everything that can go wrong between an `.insv` path and a
/// [`CalibrationSet`].
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// The last 72 bytes are not an Insta360 trailer footer.
    NoTrailer,
    /// The trailer carries no metadata record, so there is no
    /// calibration in it.
    NoMetadata,
    /// The metadata record did not decode as `ExtraMetadata`.
    Protobuf(prost::DecodeError),
    /// A metadata field Kyerag needs was absent.
    MissingField(&'static str),
    /// A token in `offset_v3` was not a number.
    OffsetNotNumeric,
    /// `offset_v3` did not parse as `1 + 19 * lens_count + 1` tokens.
    OffsetGrammar {
        lens_count: usize,
        tokens: usize,
    },
    /// The lens blocks disagreed about the size of the calibration
    /// canvas, which means the string was misread.
    CanvasMismatch,
    /// A canvas or crop dimension was zero, so no pixel scale exists.
    DegenerateCanvas,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::NoTrailer => write!(f, "file has no Insta360 trailer"),
            Self::NoMetadata => write!(f, "trailer carries no metadata record"),
            Self::Protobuf(e) => write!(f, "metadata record did not decode: {e}"),
            Self::MissingField(name) => write!(f, "metadata field {name} is missing"),
            Self::OffsetNotNumeric => write!(f, "offset_v3 holds a token that is not a number"),
            Self::OffsetGrammar { lens_count, tokens } => write!(
                f,
                "offset_v3 has {tokens} tokens, which is not 1 + 19 * {lens_count} + 1"
            ),
            Self::CanvasMismatch => write!(f, "lens blocks disagree about the calibration canvas"),
            Self::DegenerateCanvas => write!(f, "a canvas or crop dimension is zero"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<prost::DecodeError> for Error {
    fn from(e: prost::DecodeError) -> Self {
        Self::Protobuf(e)
    }
}

/// The real X4 Air trailer with serial number, GPS and capture times
/// stripped, as checked in by the format study. Both halves of this
/// module test against it: the calibration maths directly, and the
/// trailer walk by re-encoding it into a synthetic `.insv`.
#[cfg(test)]
mod fixture {
    pub const JSON: &str = include_str!("../../../docs/research/x4air-calibration.json");

    pub fn metadata() -> super::trailer::ExtraMetadata {
        serde_json::from_str(JSON).expect("fixture matches the metadata shape")
    }
}
