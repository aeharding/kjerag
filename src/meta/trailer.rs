//! Reading the `.insv` trailer: find the metadata record, decode it,
//! and touch nothing else.
//!
//! Format, from docs/research/insv-format.md section 2, where three
//! independent implementations are recorded as agreeing byte for byte.
//! The last 72 bytes of the file are
//! `padding[32] | extra_size u32 | version u32 | magic[32]`, and
//! `extra_size` covers the whole trailer including that footer. Records
//! sit backwards from there, each one
//! `payload[size] | format u8 | id u8 | size u32`, so the 6-byte header
//! trails its own payload and a reader walks headers from the end.
//!
//! Kyerag walks to record 1 and stops, so it never reads the GPS track
//! (record 7) or the thumbnails at all.
//!
//! The `ExtraMetadata` field tags are transcribed from telemetry-parser's
//! `src/insta360/extra_info.rs` (MIT OR Apache-2.0). Only the eleven
//! fields Kyerag needs are declared; protobuf skips the other ~54.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use prost::Message;

use super::{CalibrationSet, Error};

const MAGIC: &[u8] = b"8db42d694ccc418790edff439fe026bf";
/// `padding[32] | extra_size u32 | version u32 | magic[32]`.
const FOOTER_LEN: i64 = 32 + 4 + 4 + 32;
/// `format u8 | id u8 | size u32`, trailing its own payload.
const RECORD_HEADER_LEN: i64 = 1 + 1 + 4;
const METADATA_RECORD: u8 = 1;
const PROTOBUF: u8 = 1;

/// The trailer fields Kyerag reads, named as the file names them.
#[derive(Clone, PartialEq, Message)]
#[cfg_attr(test, derive(serde::Deserialize))]
#[cfg_attr(test, serde(default))]
pub(crate) struct ExtraMetadata {
    /// Selects the IMU orientation table, and nothing else.
    #[prost(string, tag = "2")]
    pub camera_type: String,
    #[prost(string, tag = "3")]
    pub fw_version: String,
    /// The delivered frame size of one lens.
    #[prost(message, optional, tag = "19")]
    pub dimension: Option<Vector2>,
    #[prost(int64, tag = "24")]
    pub first_frame_timestamp: i64,
    /// Milliseconds.
    #[prost(double, tag = "25")]
    pub rolling_shutter_time: f64,
    #[prost(message, optional, tag = "27")]
    pub window_crop_info: Option<WindowCropInfo>,
    #[prost(double, tag = "28")]
    pub gyro_timestamp: f64,
    #[prost(bool, tag = "29")]
    pub is_has_gyro_timestamp: bool,
    /// The calibration, as underscore-separated decimals. Kyerag reads
    /// this rather than `original_offset_v3` (tag 56): where the two
    /// differ, this is the one that describes the glass that was
    /// actually in front of the sensor.
    #[prost(string, tag = "54")]
    #[cfg_attr(test, serde(deserialize_with = "offset_v3_from_fixture"))]
    pub offset_v3: String,
    #[prost(bool, tag = "62")]
    pub is_raw_gyro: bool,
    #[prost(message, optional, tag = "65")]
    pub gyro_cfg_info: Option<GyroConfigInfo>,
}

#[derive(Clone, PartialEq, Message)]
#[cfg_attr(test, derive(serde::Deserialize))]
#[cfg_attr(test, serde(default))]
pub(crate) struct Vector2 {
    #[prost(int32, tag = "1")]
    pub x: i32,
    #[prost(int32, tag = "2")]
    pub y: i32,
}

/// The sensor window the camera crops out of the calibration canvas
/// before delivering it, which is what the focal length scales by.
#[derive(Clone, PartialEq, Message)]
#[cfg_attr(test, derive(serde::Deserialize))]
#[cfg_attr(test, serde(default))]
pub(crate) struct WindowCropInfo {
    #[prost(uint32, tag = "1")]
    pub src_width: u32,
    #[prost(uint32, tag = "2")]
    pub src_height: u32,
    #[prost(uint32, tag = "3")]
    pub dst_width: u32,
    #[prost(uint32, tag = "4")]
    pub dst_height: u32,
}

/// IMU full-scale ranges. Read, never assumed: the X4 Air's +/-32 g is
/// not the +/-16 g that telemetry-parser falls back to.
#[derive(Clone, PartialEq, Message)]
#[cfg_attr(test, derive(serde::Deserialize))]
#[cfg_attr(test, serde(default))]
pub(crate) struct GyroConfigInfo {
    #[prost(uint32, tag = "1")]
    pub acc_range: u32,
    #[prost(uint32, tag = "2")]
    pub gyro_range: u32,
}

/// The checked-in fixture was produced by telemetry-parser, which splits
/// this field into an array of floats on the way out. The wire carries
/// the string.
#[cfg(test)]
fn offset_v3_from_fixture<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    use serde::Deserialize;
    let tokens = Vec::<f64>::deserialize(deserializer)?;
    Ok(tokens
        .iter()
        .map(f64::to_string)
        .collect::<Vec<_>>()
        .join("_"))
}

impl CalibrationSet {
    /// Read the calibration out of an `.insv` file.
    ///
    /// Only the trailer at the end of the file is read, so a 37 GB
    /// capture costs the same as a small one.
    pub fn from_insv(path: impl AsRef<Path>) -> Result<Self, Error> {
        let mut file = std::fs::File::open(path)?;
        let record = metadata_record(&mut file)?;
        Self::from_metadata(&ExtraMetadata::decode(&*record)?)
    }
}

/// Walk the record chain backwards and return the payload of the
/// metadata record.
fn metadata_record<S: Read + Seek>(source: &mut S) -> Result<Vec<u8>, Error> {
    let file_len = source.seek(SeekFrom::End(0))? as i64;
    if file_len < FOOTER_LEN {
        return Err(Error::NoTrailer);
    }

    let mut footer = [0u8; FOOTER_LEN as usize];
    source.seek(SeekFrom::End(-FOOTER_LEN))?;
    source.read_exact(&mut footer)?;
    if &footer[FOOTER_LEN as usize - MAGIC.len()..] != MAGIC {
        return Err(Error::NoTrailer);
    }
    let trailer_len = u32::from_le_bytes([footer[32], footer[33], footer[34], footer[35]]) as i64;
    if trailer_len > file_len {
        return Err(Error::NoTrailer);
    }

    // Distance back from EOF to the record header being read.
    let mut back = FOOTER_LEN + RECORD_HEADER_LEN;
    while back < trailer_len {
        source.seek(SeekFrom::End(-back))?;
        let mut header = [0u8; RECORD_HEADER_LEN as usize];
        source.read_exact(&mut header)?;
        let (format, id) = (header[0], header[1]);
        let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as i64;
        if back + size > trailer_len {
            break;
        }
        if id == METADATA_RECORD && format == PROTOBUF {
            let mut payload = vec![0u8; size as usize];
            source.seek(SeekFrom::End(-back - size))?;
            source.read_exact(&mut payload)?;
            return Ok(payload);
        }
        back += size + RECORD_HEADER_LEN;
    }
    Err(Error::NoMetadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::fixture;
    use std::io::Cursor;

    /// A minimal `.insv`: some payload bytes standing in for the mp4,
    /// then a decoy record, then the metadata record, then the footer.
    fn synthetic_insv(metadata: &ExtraMetadata) -> Vec<u8> {
        fn record(format: u8, id: u8, payload: &[u8]) -> Vec<u8> {
            let mut out = payload.to_vec();
            out.extend([format, id]);
            out.extend((payload.len() as u32).to_le_bytes());
            out
        }

        let mut trailer = record(0, 3, &[0xAB; 64]); // gyro, walked over
        trailer.extend(record(PROTOBUF, METADATA_RECORD, &metadata.encode_to_vec()));

        let mut footer = vec![0u8; 32];
        let extra_size = trailer.len() as u32 + FOOTER_LEN as u32;
        footer.extend(extra_size.to_le_bytes());
        footer.extend(3u32.to_le_bytes()); // version
        footer.extend(MAGIC);

        let mut file = b"not really an mp4".to_vec();
        file.extend(trailer);
        file.extend(footer);
        file
    }

    #[test]
    fn the_walk_skips_other_records_and_finds_the_metadata() {
        let expected = fixture::metadata();
        let file = synthetic_insv(&expected);

        let record = metadata_record(&mut Cursor::new(file)).unwrap();
        let decoded = ExtraMetadata::decode(&*record).unwrap();

        assert_eq!(decoded, expected);
        assert_eq!(decoded.camera_type, "Insta360 X4 Air");
        assert_eq!(decoded.offset_v3.split('_').count(), 40);
    }

    #[test]
    fn a_file_without_the_magic_is_not_a_trailer() {
        let mut file = synthetic_insv(&fixture::metadata());
        let last = file.len() - 1;
        file[last] = b'0';

        let error = metadata_record(&mut Cursor::new(file)).unwrap_err();
        assert!(matches!(error, Error::NoTrailer), "{error:?}");
    }

    #[test]
    fn a_trailer_without_a_metadata_record_says_so() {
        let mut metadata = fixture::metadata();
        metadata.camera_type = "decoy".into();
        let mut file = synthetic_insv(&metadata);
        // Demote the metadata record to an unknown id.
        let id = file.len() - FOOTER_LEN as usize - 5;
        file[id] = 99;

        let error = metadata_record(&mut Cursor::new(file)).unwrap_err();
        assert!(matches!(error, Error::NoMetadata), "{error:?}");
    }

    /// The whole path a real file takes, minus the file: fixture ->
    /// protobuf -> trailer -> walk -> decode -> calibration.
    #[test]
    fn a_synthetic_capture_yields_the_fixture_calibration() {
        let file = synthetic_insv(&fixture::metadata());
        let record = metadata_record(&mut Cursor::new(file)).unwrap();
        let calibration =
            CalibrationSet::from_metadata(&ExtraMetadata::decode(&*record).unwrap()).unwrap();

        assert_eq!(calibration.camera_model, "Insta360 X4 Air");
        assert_eq!(calibration.lenses.len(), 2);
        assert!((calibration.lenses[1].intrinsics.cx - 1935.35).abs() < 0.01);
    }

    /// The first `.insv` under `~/Videos`, or whatever
    /// `KYERAG_TEST_INSV` points at.
    fn test_capture() -> Option<std::path::PathBuf> {
        if let Ok(path) = std::env::var("KYERAG_TEST_INSV") {
            return Some(path.into());
        }
        let videos = std::path::PathBuf::from(std::env::var("HOME").ok()?).join("Videos");
        let mut captures: Vec<_> = std::fs::read_dir(videos)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("insv"))
            })
            .collect();
        captures.sort();
        captures.into_iter().next()
    }

    /// Ignored because the footage is 36 GB and lives on one box. Run it
    /// with `cargo test -- --ignored --nocapture` after touching
    /// anything in this module.
    #[test]
    #[ignore = "needs real footage at ~/Videos/*.insv"]
    fn a_real_capture_parses() {
        let Some(path) = test_capture() else {
            eprintln!("no .insv found, skipping");
            return;
        };
        let calibration = CalibrationSet::from_insv(&path).unwrap();
        println!("{}: {calibration:#?}", path.display());

        // Only what holds for any dual-fisheye capture, since
        // KYERAG_TEST_INSV can point anywhere. The X4 Air's exact
        // numbers are the fixture's job.
        assert_eq!(calibration.lenses.len(), 2);
        assert!(calibration.rolling_shutter_ms > 0.0);

        let centre = (
            calibration.dimension.width as f64 / 2.0,
            calibration.dimension.height as f64 / 2.0,
        );
        for lens in &calibration.lenses {
            assert!(
                (lens.intrinsics.cx - centre.0).abs() < 40.0
                    && (lens.intrinsics.cy - centre.1).abs() < 40.0,
                "principal point {:?} is not near the frame centre {centre:?}",
                (lens.intrinsics.cx, lens.intrinsics.cy)
            );
            assert!(lens.intrinsics.xi > 1.0);
            assert!(lens.intrinsics.fx > 0.0 && lens.intrinsics.fy > 0.0);
        }

        // Lens 0 is the reference; lens 1 sits one camera body away.
        assert_eq!(calibration.lenses[0].pose.translation_m, [0.0, 0.0, 0.0]);
        let baseline = calibration.lenses[1].pose.translation_m[2].abs();
        assert!((0.02..0.05).contains(&baseline), "baseline {baseline} m");
    }
}
