//! Everything Kjerag reads out of a capture that is not pixels.
//!
//! The lens calibration, the two lenses' shutter tracks, and the IMU: its
//! samples as the sensor wrote them, and the orientation those samples
//! integrate to, which is what holds the horizon still (issue #8). No UI and
//! no ffmpeg: this layer needs a file handle and the last few megabytes of
//! the file.
//!
//! ```no_run
//! let calibration = kjerag_meta::CalibrationSet::from_insv("VID.insv")?;
//! let horizon = calibration.orientation(kjerag_meta::Filter::default());
//! # Ok::<(), kjerag_meta::Error>(())
//! ```
//!
//! The trailer format, the `offset_v3` grammar, the clock chain and the
//! provenance of every number quoted in these doc comments are in
//! `docs/research/insv-format.md`.

mod calibration;
/// Public where the rest of these are private, because its one function only
/// reads right qualified: `capture::resolve(path)`, rather than a bare
/// `resolve` at the root of a crate about trailers.
pub mod capture;
mod exposure;
mod format;
mod gyro;
mod orientation;
/// The DJI Osmo 360 `.OSV` calibration, which is in the file's own telemetry
/// track rather than in a trailer. Private like the rest; `CalibrationSet`
/// routes to it.
mod osmo;
mod pair;
mod rotation;
mod trailer;

pub use calibration::{
    CalibrationSet, Distortion, GyroConfig, GyroEncoding, Intrinsics, Lens, Model, Pose, Readout,
    Size, Sweep,
};
pub use exposure::{ExposureSample, ExposureTrack};
pub use format::{Foreign, Format};
pub use gyro::{GyroSample, GyroTrack};
pub use orientation::{Filter, OrientationSample, OrientationTrack, Seed, axis_map, body_from_imu};
pub use pair::{lens_index, sibling};
pub use rotation::{Mat3, Quat};
pub use trailer::record_index;

/// Everything that can go wrong between an `.insv` path and a
/// [`CalibrationSet`].
///
/// **The pilot reads these.** A failed open carries whatever it failed on up
/// to the shell, and the alert shows that message word for word (AGENTS.md,
/// "Errors are the error"), so these sentences are UI copy: plain words, no
/// em dashes, and specific enough to be worth reading. The test at the foot
/// of this file holds the em dash half of that.
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
    /// A metadata field Kjerag needs was absent.
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
    /// A DJI capture with no `djmd` telemetry track in it, which is where an
    /// Osmo 360 keeps its calibration.
    NoTelemetry,
    /// A field the telemetry record has to carry was not in it.
    TelemetryField(&'static str),
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
            Self::NoTelemetry => write!(f, "file has no DJI telemetry track"),
            Self::TelemetryField(name) => {
                write!(f, "the DJI telemetry record carries no {name}")
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// These are what the pilot is shown when an open fails, so the UI copy
    /// rule binds them (AGENTS.md).
    ///
    /// The `match` is the ratchet: it has no wildcard arm, so a variant added
    /// later does not compile until it is named here, and naming it is where
    /// its wording gets looked at.
    #[test]
    fn no_error_the_alert_can_show_carries_an_em_dash() {
        let every = [
            Error::Io(std::io::Error::other("that file is not readable")),
            Error::NoTrailer,
            Error::NoMetadata,
            // A real decode of real rubbish, because prost's own constructor
            // for one of these is deprecated and the message is what is under
            // test.
            Error::Protobuf(
                prost::Message::decode(&[0xff, 0xff][..])
                    .map(|_: trailer::ExtraMetadata| ())
                    .expect_err("two 0xff bytes are not a metadata record"),
            ),
            Error::MissingField("dimension"),
            Error::OffsetNotNumeric,
            Error::OffsetGrammar {
                lens_count: 2,
                tokens: 3,
            },
            Error::CanvasMismatch,
            Error::DegenerateCanvas,
            Error::NoTelemetry,
            Error::TelemetryField("lens"),
        ];
        for e in &every {
            let named = match e {
                Error::Io(_) => "Io",
                Error::NoTrailer => "NoTrailer",
                Error::NoMetadata => "NoMetadata",
                Error::Protobuf(_) => "Protobuf",
                Error::MissingField(_) => "MissingField",
                Error::OffsetNotNumeric => "OffsetNotNumeric",
                Error::OffsetGrammar { .. } => "OffsetGrammar",
                Error::CanvasMismatch => "CanvasMismatch",
                Error::DegenerateCanvas => "DegenerateCanvas",
                Error::NoTelemetry => "NoTelemetry",
                Error::TelemetryField(_) => "TelemetryField",
            };
            let said = e.to_string();
            assert!(!said.contains('\u{2014}'), "em dash in {named}: {said}");
            assert!(!said.is_empty(), "{named} says nothing");
        }
    }
}
