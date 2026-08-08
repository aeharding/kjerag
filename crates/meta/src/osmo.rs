//! The DJI Osmo 360 `.OSV` lens calibration, read out of the file's own
//! telemetry track.
//!
//! An `.OSV` is a plain ISO-BMFF MP4: two HEVC Main 10 fisheye video tracks,
//! one AAC track, and four DJI tracks whose sample entries are `djmd`
//! (telemetry) and `dbgi` (debug). Sample 0 of the **first** `djmd` track is a
//! protobuf message that opens with the camera's own name for its schema,
//! `dvtm_oq101.proto`, and carries the factory lens calibration in it. No
//! `.proto` file exists anywhere, so this reads the wire format by field
//! number: [`Fields`] is the whole of the decoder, and every number it looks
//! for is named in [`field`] below.
//!
//! **What is in a lens entry, and what is not.** Sixteen of them are written,
//! two of which carry the numbers this reads (below), and each holds a focal
//! pair, a principal point, four `k` coefficients, a reference frame size,
//! a yaw/pitch/roll triple, a unit quaternion, and a fourteen-point polyline
//! that traces where the camera's own body cuts into the picture. There is no
//! inter-lens translation in it at all, which is why [`Pose::translation_m`]
//! comes out zero and the seam band that measures parallax switches itself
//! off (`kjerag_render::band::baseline`).
//!
//! **The lens model is equidistant, `r = fx * theta`, and the four `k`
//! coefficients are not applied.** They read like Kannala-Brandt
//! theta-polynomial terms and they are not: the file says so itself. Each
//! entry's fields 22 and 23 are the fourteen points of that lens's body mask,
//! which lies along the edge of the lens's stated coverage. Over the four
//! lenses of the two cameras measured, those points sit 1804 to 1860 px from
//! the principal point. Equidistant puts that ring at 98.6 to 101.7 degrees
//! off axis, so 197 to 203 degrees of coverage, which brackets the Osmo 360's
//! own published 199. Read as
//! `r = fx * theta * (1 + k1 t^2 + k2 t^4 + k3 t^6 + k4 t^8)` the same
//! coefficients turn the radius over between **88.4 and 89.8 degrees**, at
//! 1615 to 1640 px, and bring it back down: on all four lenses the mask ring
//! is then unreachable at any angle and the lens could not see past a
//! hemisphere. The inverse reading folds as well, at about 1.55 radians.
//! Neither is a 199 degree lens, so neither ships, and the coefficients are
//! read past rather than stored: the form is unresolved and a field nothing
//! can use is not evidence, it is decoration.
//!
//! Those are the file's own numbers and the arithmetic above them, so the
//! check is repeatable from any `.OSV`: read fields 1, 22 and 23 of a lens
//! entry and compare `hypot(x - cx, y - cy)` against each candidate model's
//! radius. Re-measured on both units 2026-08-07.
//!
//! **Which entry pair.** The table holds 24 slots. Slots 1 and 2 are the two
//! lenses and are what this reads; 3 to 10 are empty on every file in the
//! corpus; 11 to 24 are seven further pairs whose focal lengths walk
//! downwards by about 0.03 percent a rung and whose principal points barely
//! move. What that ladder selects on is **not known** - a focus distance, a
//! temperature, a stabilization crop are all consistent with it - so this
//! takes the first pair, which is the one the two cameras' factory numbers
//! agree in shape with. The risk it carries is the width of that ladder: on
//! both cameras, slot 1's focal sits about 0.16 percent above its top rung and
//! 0.35 percent above its bottom, so a file whose true rung is elsewhere is
//! reprojected up to about a third of a percent off in scale.
//!
//! **Which entry is which stream** is taken as file order: entry 1 is stream
//! 0 and entry 2 is stream 1. Entry 1's recorded yaw is about 180 degrees and
//! entry 2's about 0, so the pair is the back-to-back arrangement either way
//! round, and both orders render a level, closed sphere because the two
//! calibrations differ by about a tenth of a percent in focal length and ten
//! px in principal point. An attempt to settle it by scoring overlap
//! agreement was **thrown away rather than believed**: on the far-field
//! capture the score improved when a known 20 px principal-point error was
//! injected into it, so it could not tell a right answer from a wrong one and
//! its preference is not evidence. If the order is backwards, what it costs
//! is that ten px of principal point and a tenth of a percent of scale, at
//! the seam.
//!
//! **No IMU.** The file carries a fused orientation at about 1 kHz and the
//! frame it is written in is not pinned; applying it naively made a stitch
//! worse rather than better. So none is read, [`CalibrationSet::imu`] is
//! empty, and horizon lock is the no-op an empty track already makes it
//! (`CalibrationSet::orientation`). Manual pan is the whole of the view.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::calibration::{
    CalibrationSet, Distortion, GyroConfig, GyroEncoding, Intrinsics, Lens, Model, Pose, Size,
};
use super::format::{Boxes, moov};
use super::rotation::Mat3;
use super::rotation::Quat;
use super::{Error, GyroTrack};

/// How many lens entries this reads, which is one per video stream.
const LENSES: usize = 2;

/// A direction in Kjerag's camera body, expressed in the one the file's
/// quaternions are written against.
///
/// **Measured off the file rather than assumed** (`scratch/osmo/`). Carried
/// through each lens's own quaternion, the file's `+z` lands at `(0, -1, 0)`
/// in both lenses' image frames, which is straight up the picture, so `+z` is
/// the camera's vertical. The two optical axes land on `+y` and `-y` to
/// within a hundredth, and the entry whose recorded yaw is about zero is the
/// one on `+y`, so `+y` is where the camera calls forward. `+x` is what a
/// right-handed frame has left over.
///
/// Kjerag's is `x` right, `y` down, `z` forward (`kjerag_render::projection`),
/// so this sends `z` to the file's `y`, `y` to its `-z`, and `x` to its `x`.
/// It is a rotation, which `the_body_frames_differ_by_a_rotation` checks: a
/// reflection here would render the sphere inside out.
const BODY: Mat3 = Mat3::new([[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]]);

/// The calibration of the capture at `path`.
pub(crate) fn read(path: &Path) -> Result<CalibrationSet, Error> {
    let mut file = File::open(path)?;
    let len = file.seek(SeekFrom::End(0))?;
    let moov = moov(&mut file, len)?.ok_or(Error::NoTelemetry)?;
    let at = telemetry_sample(&moov).ok_or(Error::NoTelemetry)?;
    let mut record = vec![0u8; at.size];
    file.seek(SeekFrom::Start(at.offset))?;
    file.read_exact(&mut record)?;
    from_record(&record)
}

/// Where one sample sits in the file.
struct At {
    offset: u64,
    size: usize,
}

/// Sample 0 of the first `djmd` track.
///
/// The first, not any: an `.OSV` writes two, and the second one's samples are
/// a sixth the size and carry no header. Sample 0 is the first sample of the
/// first chunk, so `stsc` is not needed to place it, only `stco` and `stsz`.
fn telemetry_sample(moov: &[u8]) -> Option<At> {
    Boxes::new(moov)
        .filter(|(kind, _)| *kind == b"trak")
        .filter_map(|(_, trak)| sample_zero(trak))
        .next()
}

/// The first sample of one track, if that track is a `djmd` one.
fn sample_zero(trak: &[u8]) -> Option<At> {
    let stbl = child(child(child(trak, b"mdia")?, b"minf")?, b"stbl")?;
    // `stsd` is a full box: four bytes of version and flags, four of entry
    // count, then the entries, each of which opens with its own size and 4cc.
    let stsd = child(stbl, b"stsd")?;
    if stsd.get(12..16)? != b"djmd" {
        return None;
    }
    // `stsz` is version and flags, one size for every sample or zero, the
    // sample count, and then the table.
    let stsz = child(stbl, b"stsz")?;
    let size = match be32(stsz, 4)? {
        0 => be32(stsz, 12)?,
        every => every,
    };
    // `stco` is version and flags, the chunk count, then the offsets. `co64`
    // is the same with 64-bit ones, which a file over 4 GB needs.
    let offset = match child(stbl, b"stco") {
        Some(stco) => u64::from(be32(stco, 8)?),
        None => be64(child(stbl, b"co64")?, 8)?,
    };
    Some(At {
        offset,
        size: size as usize,
    })
}

fn child<'a>(body: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    Boxes::new(body)
        .find(|(found, _)| *found == kind)
        .map(|(_, payload)| payload)
}

fn be32(body: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(body.get(at..at + 4)?.try_into().ok()?))
}

fn be64(body: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(body.get(at..at + 8)?.try_into().ok()?))
}

/// The field numbers this reader looks for, which are the whole of the schema
/// it knows. Everything else in the record is walked past.
mod field {
    /// Top level.
    pub const HEADER: u32 = 1;
    pub const VIDEO: u32 = 2;

    /// Inside `HEADER`.
    pub const CAMERA: u32 = 1;
    /// Inside `HEADER.CAMERA`. The serial number is field 5 and is
    /// deliberately not read: it names the pilot's unit and nothing here
    /// needs it (`CalibrationSet::camera_key`).
    pub const FIRMWARE: u32 = 6;
    pub const MODEL: u32 = 10;

    /// Inside `VIDEO`.
    pub const PICTURE: u32 = 3;
    pub const LENSES: u32 = 6;
    /// Inside `VIDEO.PICTURE`.
    pub const WIDTH: u32 = 1;
    pub const HEIGHT: u32 = 2;

    /// Inside one lens entry of `VIDEO.LENSES`. Fields 5 to 8 are the four
    /// `k` coefficients and are read past; the module's own doc says why.
    pub const FX: u32 = 1;
    pub const FY: u32 = 2;
    pub const CX: u32 = 3;
    pub const CY: u32 = 4;
    /// The frame the pixel numbers above are expressed in, which is the
    /// delivered frame: 3840 by 3840 on every file in the corpus.
    pub const CANVAS_WIDTH: u32 = 10;
    pub const CANVAS_HEIGHT: u32 = 11;
    /// Where the lens points, as a unit quaternion in a submessage of four
    /// `f32`s. There is a second copy of the same four numbers packed into
    /// field 21; this one is read because a submessage of named fields cannot
    /// be misread as a different length.
    pub const ORIENTATION: u32 = 28;
}

fn from_record(record: &[u8]) -> Result<CalibrationSet, Error> {
    let header = message(record, field::HEADER).ok_or(Error::TelemetryField("header"))?;
    let camera = message(header, field::CAMERA).ok_or(Error::TelemetryField("camera"))?;
    let video = message(record, field::VIDEO).ok_or(Error::TelemetryField("video"))?;
    let picture = message(video, field::PICTURE).ok_or(Error::TelemetryField("picture"))?;
    let table = message(video, field::LENSES).ok_or(Error::TelemetryField("lens table"))?;

    let dimension = Size {
        width: varint(picture, field::WIDTH).ok_or(Error::TelemetryField("width"))? as u32,
        height: varint(picture, field::HEIGHT).ok_or(Error::TelemetryField("height"))? as u32,
    };
    if dimension.width == 0 || dimension.height == 0 {
        return Err(Error::DegenerateCanvas);
    }

    let lenses = (0..LENSES)
        .map(|index| {
            // The entries are numbered from 1, which is what makes the first
            // pair fields 1 and 2.
            let entry = message(table, index as u32 + 1).ok_or(Error::TelemetryField("lens"))?;
            lens(entry, dimension)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(CalibrationSet {
        camera_model: text(camera, field::MODEL).unwrap_or_default(),
        firmware: text(camera, field::FIRMWARE).unwrap_or_default(),
        dimension,
        lenses,
        // Not in the record, and a readout nobody has measured is one this
        // does not correct for (`kjerag_meta::Sweep`).
        rolling_shutter_ms: 0.0,
        gyro: GyroConfig {
            encoding: GyroEncoding::Scaled,
            imu_orientation: "xZY",
            first_frame_timestamp: 0,
            gyro_timestamp: None,
        },
        exposure: Default::default(),
        // Deliberately empty: the frame the file's own orientation is written
        // in is not pinned, and an unverified frame is worse than none.
        imu: GyroTrack::default(),
        calibration_canvas: dimension,
    })
}

/// One lens entry, in delivered-frame pixels, which is the frame the file
/// already writes them in.
fn lens(entry: &[u8], dimension: Size) -> Result<Lens, Error> {
    let take = |number, what| f32s(entry, number).ok_or(Error::TelemetryField(what));
    let canvas = Size {
        width: take(field::CANVAS_WIDTH, "lens canvas width")? as u32,
        height: take(field::CANVAS_HEIGHT, "lens canvas height")? as u32,
    };
    // The numbers are already in the frame the streams decode at on every
    // file in the corpus. A file that says otherwise is one this has never
    // seen, and scaling it on a guess would be worse than saying so.
    if canvas != dimension {
        return Err(Error::CanvasMismatch);
    }
    let orientation =
        message(entry, field::ORIENTATION).ok_or(Error::TelemetryField("lens orientation"))?;
    let quaternion = |number, what| f32s(orientation, number).ok_or(Error::TelemetryField(what));
    let pointing = Quat {
        w: quaternion(1, "quaternion w")?,
        v: [
            quaternion(2, "quaternion x")?,
            quaternion(3, "quaternion y")?,
            quaternion(4, "quaternion z")?,
        ],
    }
    .normalized();

    Ok(Lens {
        intrinsics: Intrinsics {
            // No mirror parameter: the model is equidistant and `xi` belongs
            // to the Mei one.
            xi: 0.0,
            fx: take(field::FX, "fx")?,
            fy: take(field::FY, "fy")?,
            cx: take(field::CX, "cx")?,
            cy: take(field::CY, "cy")?,
        },
        // The `k` coefficients are in the file and are not read; the module
        // doc has the measurement that refused them.
        distortion: Distortion {
            k1: 0.0,
            k2: 0.0,
            k3: 0.0,
            p1: 0.0,
            p2: 0.0,
        },
        model: Model::Equidistant,
        // The yaw, pitch and roll in fields 12 to 14 describe the same
        // pointing as the quaternion and in the same absolute terms, so
        // carrying both would be carrying one convention twice. The
        // quaternion is the one taken because it needs no rotation order.
        pose: Pose {
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            roll_deg: 0.0,
            // No inter-lens translation is recorded, so there is no baseline
            // and the parallax band has nothing to measure against.
            translation_m: [0.0; 3],
        },
        // `ray_lens = pointing * ray_file`, and Kjerag asks for
        // `ray_lens = mounting * ray_body`, so the change of basis goes on
        // the right where the body vector arrives.
        mounting: Some(pointing.matrix().times(BODY)),
        // DJI writes no equivalent of Insta360's `lensType`.
        lens_type: 0,
    })
}

/// One field of a protobuf message, as it sits on the wire.
enum Wire<'a> {
    Varint(u64),
    Fixed64,
    Bytes(&'a [u8]),
    Fixed32([u8; 4]),
}

/// The fields of one message, in the order they were written.
///
/// A hand-rolled reader rather than a generated one because there is no
/// `.proto` to generate from: DJI names its schema in the record and ships it
/// nowhere. It stops at the first byte it cannot read rather than erroring,
/// which is what makes a truncated or foreign record answer "field absent"
/// instead of taking the open down.
struct Fields<'a> {
    body: &'a [u8],
    at: usize,
}

impl<'a> Iterator for Fields<'a> {
    type Item = (u32, Wire<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        let (key, at) = varint_at(self.body, self.at)?;
        let (number, wire) = ((key >> 3) as u32, key & 7);
        if number == 0 {
            return None;
        }
        let (value, at) = match wire {
            0 => {
                let (value, at) = varint_at(self.body, at)?;
                (Wire::Varint(value), at)
            }
            1 => (
                Wire::Fixed64,
                at.checked_add(8).filter(|e| *e <= self.body.len())?,
            ),
            2 => {
                let (length, at) = varint_at(self.body, at)?;
                let end = at.checked_add(usize::try_from(length).ok()?)?;
                (Wire::Bytes(self.body.get(at..end)?), end)
            }
            5 => {
                let end = at.checked_add(4)?;
                (Wire::Fixed32(self.body.get(at..end)?.try_into().ok()?), end)
            }
            // Groups, which nothing in this record uses and which cannot be
            // skipped without knowing where they end.
            _ => return None,
        };
        self.at = at;
        Some((number, value))
    }
}

/// One varint at `at`, and where it ended.
fn varint_at(body: &[u8], mut at: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *body.get(at)?;
        at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, at));
        }
    }
    None
}

fn fields(body: &[u8]) -> Fields<'_> {
    Fields { body, at: 0 }
}

/// The last value written for one field, which is what a protobuf reader is
/// required to take when a field is written more than once.
fn last(body: &[u8], number: u32) -> Option<Wire<'_>> {
    fields(body)
        .filter(|(found, _)| *found == number)
        .map(|(_, wire)| wire)
        .last()
}

fn message(body: &[u8], number: u32) -> Option<&[u8]> {
    match last(body, number)? {
        Wire::Bytes(inner) => Some(inner),
        _ => None,
    }
}

fn varint(body: &[u8], number: u32) -> Option<u64> {
    match last(body, number)? {
        Wire::Varint(value) => Some(value),
        _ => None,
    }
}

/// A single-precision float, widened, which is how every number in the
/// calibration is written.
fn f32s(body: &[u8], number: u32) -> Option<f64> {
    match last(body, number)? {
        Wire::Fixed32(raw) => Some(f64::from(f32::from_le_bytes(raw))),
        _ => None,
    }
}

fn text(body: &[u8], number: u32) -> Option<String> {
    match last(body, number)? {
        Wire::Bytes(raw) => Some(String::from_utf8_lossy(raw).into_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `fixed32` field, as the calibration writes every one of its numbers.
    fn f32field(number: u32, value: f32) -> Vec<u8> {
        let mut out = key(number, 5);
        out.extend_from_slice(&value.to_le_bytes());
        out
    }

    fn key(number: u32, wire: u64) -> Vec<u8> {
        let mut value = u64::from(number) << 3 | wire;
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            match value {
                0 => {
                    out.push(byte);
                    return out;
                }
                _ => out.push(byte | 0x80),
            }
        }
    }

    fn submessage(number: u32, body: &[u8]) -> Vec<u8> {
        let mut out = key(number, 2);
        let mut length = body.len() as u64;
        loop {
            let byte = (length & 0x7f) as u8;
            length >>= 7;
            match length {
                0 => {
                    out.push(byte);
                    break;
                }
                _ => out.push(byte | 0x80),
            }
        }
        out.extend_from_slice(body);
        out
    }

    fn varint_field(number: u32, value: u64) -> Vec<u8> {
        let mut out = key(number, 0);
        let mut value = value;
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            match value {
                0 => {
                    out.push(byte);
                    return out;
                }
                _ => out.push(byte | 0x80),
            }
        }
    }

    /// One lens entry with the numbers this reader takes, plus the fields it
    /// walks past, laid out the way the camera lays them out.
    fn entry(fx: f32, fy: f32, cx: f32, cy: f32, q: [f32; 4]) -> Vec<u8> {
        let mut out = f32field(field::FX, fx);
        out.extend(f32field(field::FY, fy));
        out.extend(f32field(field::CX, cx));
        out.extend(f32field(field::CY, cy));
        // The four `k` coefficients, which are read past.
        for (number, value) in [(5, 0.068134), (6, -0.013797), (7, 0.011794), (8, -0.007332)] {
            out.extend(f32field(number, value));
        }
        out.extend(f32field(field::CANVAS_WIDTH, 3840.0));
        out.extend(f32field(field::CANVAS_HEIGHT, 3840.0));
        let mut orientation = Vec::new();
        for (number, value) in (1..).zip(q) {
            orientation.extend(f32field(number, value));
        }
        out.extend(submessage(field::ORIENTATION, &orientation));
        out
    }

    /// The real record of `CAM_20250715191201_0003_D.OSV`, cut down to the
    /// fields this reads and with the serial left out.
    fn record() -> Vec<u8> {
        let mut camera = submessage(field::FIRMWARE, b"10.00.05.06");
        camera.extend(submessage(field::MODEL, b"Osmo 360"));
        let header = submessage(field::CAMERA, &camera);

        let mut picture = varint_field(field::WIDTH, 3840);
        picture.extend(varint_field(field::HEIGHT, 3840));

        let mut table = submessage(
            1,
            &entry(
                1046.3793,
                1046.168,
                1920.8534,
                1916.7302,
                [-0.011197756, 0.005501542, -0.7032019, 0.71088076],
            ),
        );
        table.extend(submessage(
            2,
            &entry(
                1048.025,
                1047.8745,
                1910.7661,
                1916.2424,
                [0.70341, 0.710761, -0.000703606, 0.005721177],
            ),
        ));

        let mut video = submessage(field::PICTURE, &picture);
        video.extend(submessage(field::LENSES, &table));

        let mut out = submessage(field::HEADER, &header);
        out.extend(submessage(field::VIDEO, &video));
        out
    }

    #[track_caller]
    fn near(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} is not within {tolerance} of {expected}"
        );
    }

    /// The headline: the two lenses come out with the numbers the file wrote,
    /// in the frame the streams decode at, needing no scaling on the way.
    #[test]
    fn the_record_describes_a_two_lens_osmo() {
        let calibration = from_record(&record()).unwrap();
        assert_eq!(calibration.camera_model, "Osmo 360");
        assert_eq!(calibration.firmware, "10.00.05.06");
        assert_eq!(
            calibration.dimension,
            Size {
                width: 3840,
                height: 3840
            }
        );
        assert_eq!(calibration.lenses.len(), 2);
        near(calibration.lenses[0].intrinsics.fx, 1046.379, 0.001);
        near(calibration.lenses[0].intrinsics.cx, 1920.853, 0.001);
        near(calibration.lenses[1].intrinsics.fx, 1048.025, 0.001);
        near(calibration.lenses[1].intrinsics.cx, 1910.766, 0.001);
        for lens in &calibration.lenses {
            assert_eq!(lens.model, Model::Equidistant);
            near(lens.intrinsics.cy, 1916.5, 0.5);
        }
    }

    /// The `k` coefficients are in the record and are not carried: the module
    /// doc has the measurement that refused them, and a zero here is what
    /// makes the pass run the plain equidistant map.
    #[test]
    fn the_distortion_coefficients_are_read_past() {
        let calibration = from_record(&record()).unwrap();
        for lens in &calibration.lenses {
            let Distortion { k1, k2, k3, p1, p2 } = lens.distortion;
            assert_eq!([k1, k2, k3, p1, p2], [0.0; 5]);
            assert_eq!(lens.intrinsics.xi, 0.0);
        }
    }

    /// The two lenses point opposite ways, which is what a back-to-back pair
    /// is, and they point along Kjerag's own axes rather than the file's: the
    /// entry whose recorded yaw is about zero looks **forward**, which is
    /// `+z`, and the other looks back. That is [`BODY`] doing its job, and
    /// without it both lenses point at the sky.
    #[test]
    fn the_two_mountings_are_opposed_along_kjerags_own_axes() {
        let calibration = from_record(&record()).unwrap();
        let axis = |index: usize| {
            calibration.lenses[index]
                .mounting
                .expect("an Osmo lens carries its own mounting")
                .transpose()
                .mul_vec([0.0, 0.0, 1.0])
        };
        // Entry 1 is the one whose yaw is about 180 and entry 2 the one whose
        // yaw is about 0, in that order, so lens 0 looks back.
        let (back, front) = (axis(0), axis(1));
        near(front[2], 1.0, 0.01);
        near(back[2], -1.0, 0.01);
        for pointing in [front, back] {
            near(pointing[1], 0.0, 0.05);
        }
    }

    /// [`BODY`] is a change of basis and not a reshuffle: a reflection here
    /// would render the sphere inside out, and every mounting built through it
    /// would carry the reflection too.
    #[test]
    fn the_body_frames_differ_by_a_rotation() {
        near(BODY.determinant(), 1.0, 1e-12);
        // The file's own vertical, `+z`, is up the picture, which in Kjerag's
        // frame is `-y`.
        let up = BODY.transpose().mul_vec([0.0, 0.0, 1.0]);
        assert_eq!(up, [0.0, -1.0, 0.0]);
        for lens in from_record(&record()).unwrap().lenses {
            near(lens.mounting.unwrap().determinant(), 1.0, 1e-9);
        }
    }

    /// A file with no calibration in it says so rather than producing one, and
    /// a record cut off in the middle of a field is that same answer: nothing
    /// here panics on bytes it did not write.
    #[test]
    fn a_record_that_is_not_one_names_what_is_missing() {
        assert!(matches!(
            from_record(&[]),
            Err(Error::TelemetryField("header"))
        ));
        let whole = record();
        for cut in [1, 5, 20, 60, whole.len() / 2, whole.len() - 1] {
            match from_record(&whole[..cut]) {
                Ok(_) | Err(Error::TelemetryField(_)) | Err(Error::CanvasMismatch) => {}
                other => panic!("cut at {cut} answered {other:?}"),
            }
        }
    }

    /// A lens whose numbers are written against a frame this file does not
    /// deliver is refused rather than scaled on a guess.
    #[test]
    fn a_lens_calibrated_against_another_frame_is_refused() {
        let mut camera = submessage(field::FIRMWARE, b"1");
        camera.extend(submessage(field::MODEL, b"Osmo 360"));
        let mut picture = varint_field(field::WIDTH, 2880);
        picture.extend(varint_field(field::HEIGHT, 2880));
        let mut table = submessage(
            1,
            &entry(1046.0, 1046.0, 1440.0, 1440.0, [1.0, 0.0, 0.0, 0.0]),
        );
        table.extend(submessage(
            2,
            &entry(1046.0, 1046.0, 1440.0, 1440.0, [0.0, 0.0, 1.0, 0.0]),
        ));
        let mut video = submessage(field::PICTURE, &picture);
        video.extend(submessage(field::LENSES, &table));
        let mut record = submessage(field::HEADER, &submessage(field::CAMERA, &camera));
        record.extend(submessage(field::VIDEO, &video));

        assert!(matches!(from_record(&record), Err(Error::CanvasMismatch)));
    }
}
