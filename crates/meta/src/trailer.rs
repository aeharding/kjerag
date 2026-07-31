//! Reading the `.insv` trailer: find the records Kyerag wants, and touch
//! nothing else.
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
//! **The chain is not walkable on every camera.** On the X4 Air it runs
//! out after three records: the trailer leaves slack between records (163
//! to 250 KB on the captures measured), so walking back off the third one
//! lands in the gap and reads a length out of nothing. Record 0 is an
//! index of the whole trailer and sits last in the file, so the walk
//! always reaches it before it can break, and everything past the third
//! record is found through it. The ONE X2 writes no index and packs its
//! records tight, where the walk alone gets all of them. Measured
//! 2026-07-31 on five captures from the two cameras.
//!
//! Four records are read: 1, the metadata protobuf that carries the
//! calibration, 3, the IMU track, and 4 and 12, the two lenses' shutter
//! tracks. The thumbnails are seeked over, never read.
//!
//! Record 3 is the big one, 35 MB on a 30-minute X4 Air capture, and it is
//! read whole at open. Reading it lazily would buy back a tenth of a second
//! of a file open that already costs 70 ms for the mp4 index, and would cost
//! a second file handle held for the life of the file (issue #8).
//!
//! Record 15, `SecGyro`, is **not** read. No published table says what it
//! holds, and no X4 Air or ONE X2 capture measured here carries one at all;
//! `kyerag-spike --bin gyro` prints the record ids of a file, which is how
//! that was checked rather than assumed.
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
/// The index of the whole trailer: `id u8 | format u8 | size u32 |
/// offset u32` per entry, the offset counted from the start of the
/// trailer. Empty slots are written with a zero size.
const INDEX_RECORD: u8 = 0;
const INDEX_ENTRY_LEN: usize = 1 + 1 + 4 + 4;
const METADATA_RECORD: u8 = 1;
/// The IMU: accelerometer and gyroscope, two encodings (`super::gyro`).
const GYRO_RECORD: u8 = 3;
/// Lens 0's shutter track and lens 1's, in lens order.
///
/// They are two records and they stay two here. telemetry-parser reads
/// both into one `GroupId::Exposure`, where the second silently replaces
/// the first, which is why nothing downstream of it can tell the two
/// lenses apart (docs/research/insv-format.md 6.3).
const EXPOSURE_RECORDS: [u8; 2] = [4, 12];
const BINARY: u8 = 0;
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
    /// Read the calibration and the shutter tracks out of an `.insv` file.
    ///
    /// Only the trailer at the end of the file is read, so a 37 GB
    /// capture costs the same as a small one.
    ///
    /// This is the **literal** file. A ONE X2's `_10_` file carries no
    /// trailer at all and answers [`Error::NoTrailer`] here;
    /// [`Self::from_capture`] is what the player opens files with.
    pub fn from_insv(path: impl AsRef<Path>) -> Result<Self, Error> {
        let mut file = std::fs::File::open(path)?;
        Self::from_trailer(&read_trailer(&mut file)?)
    }

    /// The calibration for the **capture** this file belongs to, which is
    /// not always the calibration in this file.
    ///
    /// The cameras that write one lens per file write one trailer for the
    /// pair, and it lives with lens 0: measured 2026-07-31 on all three ONE
    /// X2 captures on this box, where every `VID_..._00_....insv` ends in
    /// the Insta360 magic and no `VID_..._10_....insv` has a footer at all.
    /// So opening the second file of a pair has to reach across to the
    /// first, or there is no calibration, no IMU and no frame clock; issue
    /// #79 asks for either file of a pair to open the whole sphere.
    ///
    /// The reach is only made when this file has no trailer of its own, so
    /// a file that carries one is read exactly as [`Self::from_insv`] reads
    /// it, down to the bytes, and no X4-class capture takes this path.
    pub fn from_capture(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        match (Self::from_insv(path), super::pair::sibling(path)) {
            (Err(Error::NoTrailer), Some(beside)) => Self::from_insv(beside),
            (read, _) => read,
        }
    }
}

/// The payloads Kyerag reads out of one trailer.
#[derive(Debug)]
pub(crate) struct Trailer {
    pub metadata: ExtraMetadata,
    /// Record 3, the IMU. Empty where the file has no such record.
    pub gyro: Vec<u8>,
    /// Records 4 and 12 as they came, one per lens and never merged.
    /// Empty where the file has no such record, which is every camera
    /// that writes one lens per file.
    pub exposure: [Vec<u8>; EXPOSURE_RECORDS.len()],
}

/// Where one record sits in the file.
#[derive(Clone, Copy)]
struct Located {
    id: u8,
    format: u8,
    at: i64,
    size: i64,
}

fn read_trailer<S: Read + Seek>(source: &mut S) -> Result<Trailer, Error> {
    let file_len = source.seek(SeekFrom::End(0))? as i64;
    let trailer_len = trailer_len(source, file_len)?;
    let records = locate(source, file_len, file_len - trailer_len)?;

    let metadata = find(&records, METADATA_RECORD, PROTOBUF).ok_or(Error::NoMetadata)?;
    let metadata = ExtraMetadata::decode(&*read(source, metadata)?)?;

    let mut exposure = [const { Vec::new() }; EXPOSURE_RECORDS.len()];
    for (track, id) in exposure.iter_mut().zip(EXPOSURE_RECORDS) {
        if let Some(record) = find(&records, id, BINARY) {
            *track = read(source, record)?;
        }
    }
    let gyro = match find(&records, GYRO_RECORD, BINARY) {
        Some(record) => read(source, record)?,
        None => Vec::new(),
    };
    Ok(Trailer {
        metadata,
        gyro,
        exposure,
    })
}

/// Every record the trailer carries, as `(id, format, size)`, for the
/// instruments that ask what is in a file rather than reading one thing out
/// of it. Nothing in the player calls this.
pub fn record_index(path: impl AsRef<Path>) -> Result<Vec<(u8, u8, i64)>, Error> {
    let mut file = std::fs::File::open(path)?;
    let file_len = file.seek(SeekFrom::End(0))? as i64;
    let trailer_len = trailer_len(&mut file, file_len)?;
    let mut found: Vec<(u8, u8, i64)> = locate(&mut file, file_len, file_len - trailer_len)?
        .iter()
        .map(|record| (record.id, record.format, record.size))
        .collect();
    found.sort_unstable();
    found.dedup();
    Ok(found)
}

/// How many bytes back from EOF the trailer starts, read from the footer.
fn trailer_len<S: Read + Seek>(source: &mut S, file_len: i64) -> Result<i64, Error> {
    if file_len < FOOTER_LEN {
        return Err(Error::NoTrailer);
    }
    let mut footer = [0u8; FOOTER_LEN as usize];
    source.seek(SeekFrom::End(-FOOTER_LEN))?;
    source.read_exact(&mut footer)?;
    if &footer[FOOTER_LEN as usize - MAGIC.len()..] != MAGIC {
        return Err(Error::NoTrailer);
    }
    let len = u32::from_le_bytes([footer[32], footer[33], footer[34], footer[35]]) as i64;
    match len > file_len {
        true => Err(Error::NoTrailer),
        false => Ok(len),
    }
}

/// Every record the trailer offers, index first and then whatever the
/// backwards walk reached. The two agree wherever both have a record; see
/// the module docs for why only one of them ever has all of them.
fn locate<S: Read + Seek>(
    source: &mut S,
    file_len: i64,
    trailer_start: i64,
) -> Result<Vec<Located>, Error> {
    let walked = walk(file_len, trailer_start, source)?;
    let indexed = match find(&walked, INDEX_RECORD, BINARY) {
        Some(record) => index(&read(source, record)?, file_len, trailer_start),
        None => Vec::new(),
    };
    Ok([indexed, walked].concat())
}

/// The record chain, from the footer back to wherever it stops making
/// sense.
fn walk<S: Read + Seek>(
    file_len: i64,
    trailer_start: i64,
    source: &mut S,
) -> Result<Vec<Located>, Error> {
    let mut found = Vec::new();
    // Distance back from EOF to the record header being read.
    let mut back = FOOTER_LEN + RECORD_HEADER_LEN;
    while file_len - back > trailer_start {
        source.seek(SeekFrom::End(-back))?;
        let mut header = [0u8; RECORD_HEADER_LEN as usize];
        source.read_exact(&mut header)?;
        let size = u32::from_le_bytes([header[2], header[3], header[4], header[5]]) as i64;
        let at = file_len - back - size;
        if at < trailer_start {
            break;
        }
        found.push(Located {
            id: header[1],
            format: header[0],
            at,
            size,
        });
        back += size + RECORD_HEADER_LEN;
    }
    Ok(found)
}

/// Record 0's payload, as records.
fn index(payload: &[u8], file_len: i64, trailer_start: i64) -> Vec<Located> {
    payload
        .chunks_exact(INDEX_ENTRY_LEN)
        .map(|entry| Located {
            id: entry[0],
            format: entry[1],
            at: trailer_start + u32::from_le_bytes([entry[6], entry[7], entry[8], entry[9]]) as i64,
            size: u32::from_le_bytes([entry[2], entry[3], entry[4], entry[5]]) as i64,
        })
        .filter(|record| record.at + record.size <= file_len)
        .collect()
}

/// The first record of this id and format that has anything in it.
fn find(records: &[Located], id: u8, format: u8) -> Option<Located> {
    records
        .iter()
        .copied()
        .find(|record| (record.id, record.format) == (id, format) && record.size > 0)
}

fn read<S: Read + Seek>(source: &mut S, record: Located) -> Result<Vec<u8>, Error> {
    let mut payload = vec![0u8; record.size as usize];
    source.seek(SeekFrom::Start(record.at as u64))?;
    source.read_exact(&mut payload)?;
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use std::io::Cursor;
    use std::time::Duration;

    /// A minimal `.insv`: bytes standing in for the mp4, then a trailer of
    /// records, then the footer.
    #[derive(Default)]
    struct Capture {
        /// `(id, format, payload)` in file order.
        records: Vec<(u8, u8, Vec<u8>)>,
        /// Dead bytes written before each record. The X4 Air leaves 163
        /// to 250 KB of it and the ONE X2 leaves none, and it is the
        /// whole difference between a chain a reader can walk and one
        /// that stops making sense after the records nearest the footer.
        slack: usize,
        /// Whether to write record 0, the index of everything else.
        indexed: bool,
    }

    impl Capture {
        /// What every camera writes: a gyro track that must be stepped
        /// over rather than read, and the metadata record.
        fn of(metadata: &ExtraMetadata) -> Self {
            Self {
                records: vec![
                    (3, BINARY, vec![0xAB; 64]),
                    (METADATA_RECORD, PROTOBUF, metadata.encode_to_vec()),
                ],
                ..Self::default()
            }
        }

        fn with(mut self, id: u8, payload: Vec<u8>) -> Self {
            self.records.push((id, BINARY, payload));
            self
        }

        fn insv(&self) -> Vec<u8> {
            let mut trailer = Vec::new();
            let mut entries = Vec::new();
            for (id, format, payload) in &self.records {
                trailer.extend(std::iter::repeat_n(0xEE, self.slack));
                entries.push((*id, *format, payload.len() as u32, trailer.len() as u32));
                trailer.extend(header(*id, *format, payload));
            }
            if self.indexed {
                let index: Vec<u8> = entries
                    .iter()
                    .flat_map(|(id, format, size, at)| {
                        [&[*id, *format][..], &size.to_le_bytes(), &at.to_le_bytes()].concat()
                    })
                    .collect();
                trailer.extend(header(INDEX_RECORD, BINARY, &index));
            }

            let mut file = b"not really an mp4".to_vec();
            file.extend(&trailer);
            file.extend(vec![0u8; 32]);
            file.extend((trailer.len() as u32 + FOOTER_LEN as u32).to_le_bytes());
            file.extend(3u32.to_le_bytes()); // version
            file.extend(MAGIC);
            file
        }
    }

    /// `payload | format | id | size`, the way the trailer stores one.
    fn header(id: u8, format: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = payload.to_vec();
        out.extend([format, id]);
        out.extend((payload.len() as u32).to_le_bytes());
        out
    }

    /// An exposure record: `{u64 timestamp, f64 shutter}` per sample, at
    /// the X4 Air's microsecond timebase and frame interval.
    fn shutters(shutters: &[f64]) -> Vec<u8> {
        shutters
            .iter()
            .enumerate()
            .flat_map(|(index, shutter)| {
                let timestamp = 3_812_440u64 + 33_366 * index as u64;
                [timestamp.to_le_bytes(), shutter.to_le_bytes()].concat()
            })
            .collect()
    }

    fn trailer_of(file: Vec<u8>) -> Result<Trailer, Error> {
        read_trailer(&mut Cursor::new(file))
    }

    #[test]
    fn the_walk_skips_other_records_and_finds_the_metadata() {
        let expected = fixture::metadata();
        let decoded = trailer_of(Capture::of(&expected).insv()).unwrap().metadata;

        assert_eq!(decoded, expected);
        assert_eq!(decoded.camera_type, "Insta360 X4 Air");
        assert_eq!(decoded.offset_v3.split('_').count(), 40);
    }

    #[test]
    fn a_file_without_the_magic_is_not_a_trailer() {
        let mut file = Capture::of(&fixture::metadata()).insv();
        let last = file.len() - 1;
        file[last] = b'0';

        let error = trailer_of(file).unwrap_err();
        assert!(matches!(error, Error::NoTrailer), "{error:?}");
    }

    /// A capture written one lens per file, as a ONE X2 writes it: a `_00_`
    /// file with the whole trailer in it and a `_10_` file with no trailer
    /// at all. Both are synthetic; the shape is the measured one.
    fn per_lens_capture(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kyerag-pair-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut metadata = fixture::metadata();
        metadata.camera_type = "Insta360 ONE X2".into();
        std::fs::write(
            dir.join("VID_20251018_184419_00_001.insv"),
            Capture::of(&metadata).insv(),
        )
        .unwrap();
        // The second file really is a plain mp4 as far as the trailer reader
        // is concerned: no records, no footer, no magic.
        std::fs::write(
            dir.join("VID_20251018_184419_10_001.insv"),
            b"not really an mp4 either",
        )
        .unwrap();
        dir
    }

    /// **Issue #79.** Opening the second file of a per-lens pair used to fail
    /// outright, because the calibration is not in it. It belongs to the
    /// capture rather than to the file, so the read reaches across.
    #[test]
    fn the_second_file_of_a_pair_reads_the_first_file_s_trailer() {
        let dir = per_lens_capture("second");
        let lens0 = dir.join("VID_20251018_184419_00_001.insv");
        let lens1 = dir.join("VID_20251018_184419_10_001.insv");

        // What the file itself says, which is nothing.
        assert!(
            matches!(CalibrationSet::from_insv(&lens1), Err(Error::NoTrailer)),
            "the second file of a pair carries no trailer"
        );

        // What the capture says, from either end of it.
        let from0 = CalibrationSet::from_capture(&lens0).unwrap();
        let from1 = CalibrationSet::from_capture(&lens1).unwrap();
        assert_eq!(from0.camera_model, "Insta360 ONE X2");
        assert_eq!(from1.camera_model, from0.camera_model);
        assert_eq!(from1.lenses.len(), 2);
        assert_eq!(
            from1.lenses[1].pose.roll_deg, from0.lenses[1].pose.roll_deg,
            "either file of a pair calibrates the same two lenses"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// And the reach is only ever made by a file that has no trailer of its
    /// own, so nothing that used to be read from one file is read from two.
    #[test]
    fn a_file_with_a_trailer_never_reads_its_sibling_s() {
        let dir = per_lens_capture("first");
        let lens0 = dir.join("VID_20251018_184419_00_001.insv");

        // Put a decoy in the sibling's place. If `from_capture` reached for
        // it, the camera model below would be the decoy's.
        let mut decoy = fixture::metadata();
        decoy.camera_type = "decoy".into();
        std::fs::write(
            dir.join("VID_20251018_184419_10_001.insv"),
            Capture::of(&decoy).insv(),
        )
        .unwrap();

        let read = CalibrationSet::from_capture(&lens0).unwrap();
        assert_eq!(read.camera_model, "Insta360 ONE X2");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A per-lens file whose partner is not on the card is what it always
    /// was: no trailer, no calibration, and the same error as before. The
    /// pilot copied one file off the camera, and half a capture cannot be
    /// invented.
    #[test]
    fn a_pair_with_no_sibling_on_disk_fails_the_way_it_did() {
        let dir = per_lens_capture("alone");
        let lens1 = dir.join("VID_20251018_184419_10_001.insv");
        std::fs::remove_file(dir.join("VID_20251018_184419_00_001.insv")).unwrap();

        assert!(matches!(
            CalibrationSet::from_capture(&lens1),
            Err(Error::NoTrailer)
        ));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_trailer_without_a_metadata_record_says_so() {
        let mut metadata = fixture::metadata();
        metadata.camera_type = "decoy".into();
        let mut file = Capture::of(&metadata).insv();
        // Demote the metadata record to an unknown id.
        let id = file.len() - FOOTER_LEN as usize - 5;
        file[id] = 99;

        let error = trailer_of(file).unwrap_err();
        assert!(matches!(error, Error::NoMetadata), "{error:?}");
    }

    /// The whole path a real file takes, minus the file: fixture ->
    /// protobuf -> trailer -> walk -> decode -> calibration.
    #[test]
    fn a_synthetic_capture_yields_the_fixture_calibration() {
        let trailer = trailer_of(Capture::of(&fixture::metadata()).insv()).unwrap();
        let calibration = CalibrationSet::from_trailer(&trailer).unwrap();

        assert_eq!(calibration.camera_model, "Insta360 X4 Air");
        assert_eq!(calibration.lenses.len(), 2);
        assert!((calibration.lenses[1].intrinsics.cx - 1935.35).abs() < 0.01);
    }

    /// Records 4 and 12 are two lenses and stay two. Reading them into one
    /// key, as telemetry-parser does, leaves lens 0's shutters replaced by
    /// lens 1's and no way to tell that has happened: here that would show
    /// as both tracks reading 3 ms.
    #[test]
    fn the_two_lenses_shutter_tracks_do_not_overwrite_each_other() {
        let file = Capture::of(&fixture::metadata())
            .with(EXPOSURE_RECORDS[0], shutters(&[0.001, 0.002]))
            .with(EXPOSURE_RECORDS[1], shutters(&[0.003, 0.004]))
            .insv();

        let calibration = CalibrationSet::from_trailer(&trailer_of(file).unwrap()).unwrap();
        let shutter = |lens: usize| {
            calibration.exposure[lens]
                .samples()
                .iter()
                .map(|sample| sample.shutter_s)
                .collect::<Vec<_>>()
        };

        assert_eq!(shutter(0), [0.001, 0.002]);
        assert_eq!(shutter(1), [0.003, 0.004]);
    }

    /// A camera that writes one lens per file writes record 4 and no
    /// record 12, and that is a file that opens rather than an error.
    #[test]
    fn a_file_with_one_lenss_track_leaves_the_other_empty() {
        let file = Capture::of(&fixture::metadata())
            .with(EXPOSURE_RECORDS[0], shutters(&[0.0065]))
            .insv();

        let calibration = CalibrationSet::from_trailer(&trailer_of(file).unwrap()).unwrap();

        assert_eq!(calibration.exposure[0].samples().len(), 1);
        assert!(calibration.exposure[1].is_empty());
    }

    /// The X4 Air's trailer, in miniature: slack between the records, so
    /// the backwards walk steps off the record nearest the footer into
    /// dead bytes and stops. Everything is still found, through the index.
    /// Without the index this same file yields nothing but the record the
    /// walk started on.
    #[test]
    fn a_chain_that_stops_short_is_read_through_the_index() {
        let spaced = Capture {
            slack: 64,
            indexed: true,
            ..Capture::of(&fixture::metadata())
        }
        .with(EXPOSURE_RECORDS[0], shutters(&[0.001]))
        .with(EXPOSURE_RECORDS[1], shutters(&[0.003]));

        let calibration =
            CalibrationSet::from_trailer(&trailer_of(spaced.insv()).unwrap()).unwrap();
        assert_eq!(calibration.camera_model, "Insta360 X4 Air");
        assert_eq!(
            calibration.exposure[0].shutter_at(Duration::ZERO),
            Some(0.001)
        );
        assert_eq!(
            calibration.exposure[1].shutter_at(Duration::ZERO),
            Some(0.003)
        );

        let unindexed = Capture {
            indexed: false,
            ..spaced
        };
        let error = trailer_of(unindexed.insv()).unwrap_err();
        assert!(matches!(error, Error::NoMetadata), "{error:?}");
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

        // The IMU, and the one thing about it physics knows the answer
        // to: an accelerometer measures 1 g and nothing else, on
        // average, however the camera is pointing. It comes out at 1.00
        // to 1.02 g on every capture measured, and it is the check that
        // says the ranges, the bias and the accelerometer-first ordering
        // were all read right: any one of them wrong and this is 0.5, 2
        // or a number with no units.
        let imu = &calibration.imu;
        assert!(!imu.is_empty(), "no gyro record");
        assert!(
            (100.0..2000.0).contains(&imu.rate_hz()),
            "the IMU runs at {} Hz",
            imu.rate_hz()
        );
        let mean = imu
            .samples()
            .iter()
            .map(|sample| {
                sample
                    .accel_g
                    .iter()
                    .map(|axis| axis * axis)
                    .sum::<f64>()
                    .sqrt()
            })
            .sum::<f64>()
            / imu.samples().len() as f64;
        println!("accelerometer reads {mean:.4} g on average");
        assert!((0.9..1.3).contains(&mean), "accelerometer reads {mean} g");

        // And the orientation those samples integrate to is a rotation at
        // every instant, which is what the shader composes with.
        let held = calibration.orientation(crate::Filter::default());
        assert!(!held.is_empty());
        for sample in held.samples().iter().step_by(1000) {
            let q = sample.world_from_body;
            let length = (q.w * q.w + q.v.iter().map(|c| c * c).sum::<f64>()).sqrt();
            assert!((length - 1.0).abs() < 1e-6, "{q:?} is not a rotation");
        }

        // Lens 0's shutter track is the one every camera writes. Its
        // samples run one per frame, which is what says the timebase was
        // read right: at the wrong one they land 1000x apart.
        let track = &calibration.exposure[0];
        assert!(!track.is_empty(), "no exposure record for lens 0");
        let step = track.samples()[1].offset_us - track.samples()[0].offset_us;
        assert!((20_000..60_000).contains(&step), "samples {step} us apart");

        // And where the file has both, they are two tracks and not one:
        // the shutters differ, which is the whole reason record 12 must
        // not be read over record 4.
        if calibration.exposure[1].is_empty() {
            return;
        }
        let ratios: Vec<f64> = (0..60)
            .filter_map(|second| {
                let at = std::time::Duration::from_secs(second * 30);
                Some(
                    calibration.exposure[0].shutter_at(at)?
                        / calibration.exposure[1].shutter_at(at)?,
                )
            })
            .collect();
        let worst = ratios
            .iter()
            .fold(1.0f64, |held, ratio| held.max(ratio.max(1.0 / ratio)));
        println!(
            "shutter ratio over the file: worst {worst:.3} across {} places",
            ratios.len()
        );
        assert!(
            worst > 1.05,
            "the two lenses' shutters never differ, which is not a track per lens"
        );
    }
}
