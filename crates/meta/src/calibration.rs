//! The `offset_v3` lens calibration, turned into numbers a shader can
//! use directly.
//!
//! Grammar, worked example and provenance for every constant here:
//! `docs/research/insv-format.md` sections 3 and 4.

use super::exposure::Clock;
use super::orientation::{Filter, OrientationTrack, body_from_imu};
use super::rotation::Mat3;
use super::trailer::{ExtraMetadata, Trailer};
use super::{Error, ExposureTrack, GyroTrack};

/// `offset_v3` is `lens_count`, then this many fields per lens, then a
/// version word. The field order inside a block is fixed:
/// `xi, fx, fy, cx, cy, yaw, pitch, roll, tx, ty, tz, k1, k2, k3, p1,
/// p2, calib_w, calib_h, lensType`.
const FIELDS_PER_LENS: usize = 19;

/// FNV-1a, which is what [`CalibrationSet::camera_key`] is taken with.
///
/// Written out rather than taken from `std::hash`, whose `DefaultHasher` is
/// explicitly allowed to answer differently between releases: this number is
/// written into the pilot's config and read back by a later build. A hash
/// rather than a cryptographic digest because nothing here defends against a
/// chosen collision; the worst one could do is hand one camera another's seam
/// correction.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// A width and height in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// What the `.insv` trailer says about the camera and the capture: the
/// geometry the shader reprojects with, and the clocks and shutters the
/// rest of it is timed by.
///
/// Every pixel quantity in here is in the coordinate system of the
/// **delivered per-lens frame** ([`Self::dimension`]; 3840x3840 on the
/// X4 Air, one video stream each), origin at its top left. The file does
/// not store them that way. It stores them on a side-by-side calibration
/// canvas ([`Self::calibration_canvas`]; 15360x7680, i.e. two 7680-wide
/// lens images) where lens 1's principal point carries a +7680 offset.
/// [`Intrinsics`] documents the conversion that happens on the way in.
#[derive(Debug, Clone)]
pub struct CalibrationSet {
    /// `camera_type`, e.g. `Insta360 X4 Air`.
    pub camera_model: String,
    /// `fw_version`. The calibration grammar has changed across firmware
    /// generations, so a bug report needs this.
    pub firmware: String,
    /// The delivered frame size of one lens.
    pub dimension: Size,
    /// In file order. Lens 0 is the extrinsic reference.
    pub lenses: Vec<Lens>,
    /// Row readout time in milliseconds (`rolling_shutter_time`; 15.883
    /// on the fixture). At 3840 rows this displaces 12 to 18 px under
    /// typical handheld motion, the same magnitude as seam parallax, so
    /// it is not optional.
    pub rolling_shutter_ms: f64,
    pub gyro: GyroConfig,
    /// One shutter track per lens, in lens order, from trailer records 4
    /// and 12. Empty for a lens the file carries no record for, which is
    /// lens 1 on every camera that writes one lens per file.
    ///
    /// Read [`ExposureTrack`]'s own note before reaching for these to
    /// match brightness across the seam: measured on real captures, they
    /// do not say what they look like they say.
    pub exposure: [ExposureTrack; 2],
    /// The IMU track from trailer record 3, **in the sensor's own axes**.
    /// Empty for a file that carries no gyro record.
    ///
    /// Raw on purpose: turning it into the camera body's frame is a
    /// choice with evidence behind it, and [`Self::orientation`] is where
    /// that choice is made and where it can be overridden by a harness
    /// that wants to watch a wrong one fail.
    pub imu: GyroTrack,
    /// The canvas the file's own numbers were expressed on, kept so the
    /// conversion in [`Intrinsics`] stays auditable. Nothing downstream
    /// needs it.
    pub calibration_canvas: Size,
}

/// One lens: a Mei/UCM camera model plus where the lens sits.
#[derive(Debug, Clone)]
pub struct Lens {
    pub intrinsics: Intrinsics,
    pub distortion: Distortion,
    pub pose: Pose,
    /// `lensType`, 131 on the X4 Air and 71 on the non-Air X4. No
    /// decoder table for this value exists anywhere; it is carried
    /// through so a future reader can match on it.
    pub lens_type: u32,
}

/// Mei/UCM intrinsics in delivered-frame pixels.
///
/// Two different scales get us here from the canvas-space numbers in the
/// file, which is what telemetry-parser's `insert_lens_profile` does as
/// well:
///
/// - the **principal point** by the canvas ratio,
///   `dimension / (calib_w / lens_count)`, after subtracting the lens's
///   own slot offset from `cx`. The x and y ratios are computed
///   separately because they are not always equal: both are 0.5 on the
///   X4 Air's 15360x7680 canvas, but a non-Air X4 records 16000x6000,
///   where they differ.
/// - the **focal length** by the crop ratio,
///   `dimension / window_crop_info.dst`, because the camera delivers a
///   cropped sensor window: 7424 of the 7680-wide canvas on the fixture,
///   so `fx = 7087.49 * 3840 / 7424 = 3665.9`, not `* 0.5`.
///
/// Scaling the principal point the crop way instead (subtracting the
/// 128 px crop origin first) moves it by at most 0.53 px on the fixture,
/// because it sits near the canvas centre either way.
#[derive(Debug, Clone, Copy)]
pub struct Intrinsics {
    /// The unified-camera-model mirror parameter, 2.31494 on the
    /// fixture. It is why rays past 90 degrees off-axis still project to
    /// finite coordinates, and therefore why the overlap region is
    /// representable at all.
    pub xi: f64,
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
}

/// Brown-Conrady distortion **on the Mei normalized plane**, not
/// OpenCV-fisheye theta-polynomial coefficients: feeding these to
/// `cv2.fisheye` maps the image edge to about 35 degrees instead of 90.
///
/// They are order 1 to 4 on real cameras (0.958, -1.801, 3.576 on the
/// fixture), so the polynomial dominates near the frame edge, which is
/// exactly where the seam is.
#[derive(Debug, Clone, Copy)]
pub struct Distortion {
    pub k1: f64,
    pub k2: f64,
    pub k3: f64,
    pub p1: f64,
    pub p2: f64,
}

/// How one frame is read off the sensor: how long the whole readout takes,
/// and which way across the **delivered** picture it runs.
///
/// A rolling shutter does not expose a frame at an instant. Row by row, each
/// one `seconds / rows` after the last, so a camera that moves during the
/// readout writes each row from a different orientation (issue #9). Both
/// halves of that are needed to undo it: the span, which the file records,
/// and the direction, which it does not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Readout {
    /// `rolling_shutter_time`, in seconds: 0.015883 on the X4 Air.
    ///
    /// Zero is a camera whose readout is not known, and it switches the
    /// correction off rather than guessing.
    pub seconds: f64,
    pub sweep: Sweep,
}

/// Which way across the delivered frame the sensor's rows advance in time,
/// as a direction in its own pixel coordinates (x right, y down).
///
/// The direction is **not in the file**, and it is what decides whether a
/// correction removes the skew or doubles it, so it is measured per camera in
/// [`readout_sweep`] and is [`Self::Unknown`] on any camera nobody has
/// measured. Unknown is a zero axis, which switches the correction off
/// entirely rather than guessing at it.
///
/// docs/research/insv-format.md 6.7 has the method and the numbers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Sweep {
    /// Not measured on this camera, and therefore not corrected for. The
    /// pass is then what it was before issue #9, down to the bits.
    #[default]
    Unknown,
    /// Rows advance towards the right of the delivered frame: the leftmost
    /// column is read first.
    Right,
    Left,
    /// Rows advance down the delivered frame, which is what a sensor
    /// delivered unturned would do.
    Down,
    Up,
}

impl Sweep {
    /// The direction as a unit vector in delivered-frame pixels, which is
    /// what turns a landing into its share of the readout. Zero where the
    /// direction is not known, which switches the correction off.
    pub fn axis(self) -> [f64; 2] {
        match self {
            Self::Unknown => [0.0, 0.0],
            Self::Right => [1.0, 0.0],
            Self::Left => [-1.0, 0.0],
            Self::Down => [0.0, 1.0],
            Self::Up => [0.0, -1.0],
        }
    }
}

/// Where the lens sits, as a **residual against the nominal back-to-back
/// arrangement** rather than an absolute orientation.
///
/// Lens 1's yaw is 0.039 degrees on the fixture, not 180: the flip is
/// implied by the arrangement and is not in these numbers. Applying them
/// as absolute poses points both lenses the same way.
#[derive(Debug, Clone, Copy)]
pub struct Pose {
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    /// A deliberate sensor rotation plus tolerance, and not a constant:
    /// near 90 on both X4 variants, near 180 and 0 on a ONE X2. That
    /// negative X2 value is what rules out the `half_fov` reading of
    /// this slot (docs/research/insv-format.md 4.7).
    pub roll_deg: f64,
    /// Metres, lens 0 at the origin. Lens 1 is dominated by z, and that
    /// is the inter-lens baseline that sets parallax: -0.033284 m on the
    /// fixture, -0.03132 m on a non-Air X4.
    pub translation_m: [f64; 3],
}

/// The quarter turn between `offset_v3`'s roll and the delivered frame's
/// own vertical. Applying roll as the file writes it renders an X4 Air a
/// quarter turn on its side; subtracting this datum first renders it
/// upright, and renders a ONE X2 upright as well.
///
/// Measured 2026-07-31 by rendering all four candidate rotations of lens 0
/// against plumb references in real footage from both cameras. The method,
/// the references and the result table are in
/// docs/research/insv-format.md 4.8.
const ROLL_DATUM_DEG: f64 = -90.0;

impl Pose {
    /// The lens's own mounting: roll about the optical axis, then the
    /// sub-degree yaw and pitch. `ray_lens = lens_from_body * ray_body`,
    /// in a right-handed frame whose axes are the delivered frame's own,
    /// x right, y down, z out along the optical axis.
    ///
    /// This is lens 0's whole story. Lens 1 additionally sits in a nominal
    /// arrangement the file does not record, a half turn about the body's
    /// vertical, which `kyerag-render` multiplies on the right of this
    /// (docs/research/insv-format.md 4.9). The **order** of the three
    /// angles is not settled, and neither camera can settle it: yaw and
    /// pitch are 0.103 and 0.07 degrees on the X4 Air, so every ordering
    /// agrees to about 2 px.
    ///
    /// It lives here rather than in the shader layer because the same three
    /// angles describe where the IMU is bolted, one quarter turn away
    /// ([`Self::sensor_from_body`]).
    pub fn lens_from_body(&self) -> Mat3 {
        // The datum is on `roll`, inside the composition, not a quarter turn
        // bolted onto the outside of it: `Rz(roll - 90) Ry Rx` and
        // `Rz(roll) Ry Rx Rz(-90)` are two different rotations wherever yaw
        // and pitch are not zero, and on this fixture the difference moves
        // the seam crossover by 3 percent of the blend band.
        Mat3::rot_z((self.roll_deg + ROLL_DATUM_DEG).to_radians())
            .times(Mat3::rot_y(self.yaw_deg.to_radians()))
            .times(Mat3::rot_x(self.pitch_deg.to_radians()))
    }

    /// The same mounting measured against the **sensor** rather than the
    /// delivered frame: `roll` exactly as `offset_v3` writes it, with no
    /// quarter-turn datum in it.
    ///
    /// **Measured 2026-07-31 (issue #8), and it settles an open question
    /// from 4.8.** That entry recorded two readings of where the 90 degrees
    /// comes from, and said nothing downstream could tell them apart: either
    /// `roll` is measured from the delivered frame's horizontal axis, or the
    /// camera delivers the sensor image already turned a quarter turn. The
    /// IMU tells them apart, because it is bolted to the sensor and not to
    /// the picture. Held level by its accelerometer alone, an X4 Air's
    /// horizon comes out exactly a quarter turn on its side through
    /// `lens_from_body` and level through this. So the picture is rotated
    /// and the sensor is not, and the datum belongs to the delivered frame.
    /// docs/research/insv-format.md 8.5 has the frames.
    pub fn sensor_from_body(&self) -> Mat3 {
        Mat3::rot_z(self.roll_deg.to_radians())
            .times(Mat3::rot_y(self.yaw_deg.to_radians()))
            .times(Mat3::rot_x(self.pitch_deg.to_radians()))
    }
}

/// What it takes to read the gyro record and place its samples in time.
#[derive(Debug, Clone)]
pub struct GyroConfig {
    pub encoding: GyroEncoding,
    /// telemetry-parser's axis convention: three letters naming which
    /// sensor axis feeds x, y and z, uppercase meaning negated. See
    /// [`imu_orientation`].
    pub imu_orientation: &'static str,
    /// `first_frame_timestamp`, the clock origin, in the trailer's own
    /// ticks and **not** always microseconds: `is_raw_gyro` selects the
    /// tick, as it does for the exposure records ([`super::ExposureTrack`],
    /// where the two readings are measured against real captures). The
    /// X4 Air writes 3812440 here and the ONE X2 writes 4254, for files
    /// whose first frames are 3.8 s and 4.3 s in.
    pub first_frame_timestamp: i64,
    /// `gyro_timestamp`, the gyro-to-video offset, applied as
    /// `t -= gyro_timestamp / 1000` at the end of the clock chain in
    /// docs/research/insv-format.md 8.3. `None` when the file says not
    /// to apply it (`is_has_gyro_timestamp` is false).
    pub gyro_timestamp: Option<f64>,
}

/// How the gyro record is encoded. This also decides the timebase: the
/// raw form takes an extra division by 1000 in the clock chain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GyroEncoding {
    /// 20 bytes per sample: `u64` timestamp, then six `u16` biased by
    /// 32768, accelerometer triple first. Scale by `32768 / range`.
    Raw {
        accel_range_g: f64,
        gyro_range_dps: f64,
    },
    /// 56 bytes per sample: `u64` timestamp, then six `f64`,
    /// accelerometer in g and gyroscope in rad/s. Nothing to scale.
    Scaled,
}

impl CalibrationSet {
    /// Interpret the records the trailer handed over.
    pub(crate) fn from_trailer(trailer: &Trailer) -> Result<Self, Error> {
        let clock = Clock {
            ticks_per_second: match trailer.metadata.is_raw_gyro {
                true => 1_000_000,
                false => 1_000,
            },
            first_frame: trailer.metadata.first_frame_timestamp,
        };
        let set = Self::from_metadata(&trailer.metadata)?;
        Ok(Self {
            exposure: std::array::from_fn(|lens| {
                ExposureTrack::parse(&trailer.exposure[lens], clock)
            }),
            imu: GyroTrack::parse(
                &trailer.gyro,
                set.gyro.encoding,
                clock,
                set.gyro.video_offset_us(),
            ),
            ..set
        })
    }

    /// The rotation that takes an IMU reading into the camera body's frame,
    /// from this camera's axis convention and lens 0's own mounting.
    pub fn body_from_imu(&self) -> Mat3 {
        match self.lenses.first() {
            Some(lens) => body_from_imu(self.gyro.imu_orientation, &lens.pose),
            None => Mat3::IDENTITY,
        }
    }

    /// What names the camera this capture came off, without naming the unit:
    /// the model, and the calibration the factory wrote into it.
    ///
    /// The seam correction is a property of one camera rather than of one
    /// file (issue #48), so it is stored under this. Two things make these
    /// bytes the right ones to take it over:
    ///
    /// - **They are per unit.** `offset_v3` is a factory measurement of two
    ///   lenses that were glued into one body, so two cameras of the same
    ///   model do not share it, and the same camera writes the same string
    ///   into every file: byte-identical over the owner's captures from
    ///   April to July.
    /// - **They are not the serial.** The serial and the GPS track are in the
    ///   same metadata record and neither is in here; nothing in this hash
    ///   comes from anywhere but the model name and the lens numbers, and
    ///   those numbers are already public in
    ///   `docs/research/x4air-calibration.json`.
    ///
    /// It covers the **delivered** geometry rather than the canvas one: the
    /// principal point half of a seam correction is in delivered-frame
    /// pixels, so a capture mode that delivers a different frame size is a
    /// different key and gets its own calibration rather than one scaled
    /// wrong.
    ///
    /// 0 for a calibration with no lenses in it, which is not a camera.
    pub fn camera_key(&self) -> u64 {
        if self.lenses.is_empty() {
            return 0;
        }
        let mut hash = FNV_OFFSET;
        let mut eat = |bytes: &[u8]| hash = fnv1a(hash, bytes);
        eat(self.camera_model.as_bytes());
        eat(&self.dimension.width.to_le_bytes());
        eat(&self.dimension.height.to_le_bytes());
        for lens in &self.lenses {
            let Intrinsics { xi, fx, fy, cx, cy } = lens.intrinsics;
            let Distortion { k1, k2, k3, p1, p2 } = lens.distortion;
            let Pose {
                yaw_deg,
                pitch_deg,
                roll_deg,
                translation_m: [tx, ty, tz],
            } = lens.pose;
            for number in [
                xi, fx, fy, cx, cy, k1, k2, k3, p1, p2, yaw_deg, pitch_deg, roll_deg, tx, ty, tz,
            ] {
                eat(&number.to_le_bytes());
            }
            eat(&lens.lens_type.to_le_bytes());
        }
        hash
    }

    /// How one frame is read off this camera's sensor (issue #9): the span
    /// from the file, and the direction from [`readout_sweep`].
    pub fn readout(&self) -> Readout {
        Readout {
            seconds: self.rolling_shutter_ms / 1_000.0,
            sweep: readout_sweep(&self.camera_model),
        }
    }

    /// Where the camera body was pointing, over the whole file.
    ///
    /// Empty for a file with no IMU record, which is what makes horizon
    /// lock a no-op on such a file rather than an error.
    pub fn orientation(&self, filter: Filter) -> OrientationTrack {
        filter.solve(&self.imu, self.body_from_imu())
    }

    /// Interpret the trailer's metadata record.
    fn from_metadata(metadata: &ExtraMetadata) -> Result<Self, Error> {
        let dimension = {
            let d = metadata
                .dimension
                .as_ref()
                .ok_or(Error::MissingField("dimension"))?;
            Size {
                width: d.x.max(0) as u32,
                height: d.y.max(0) as u32,
            }
        };
        let crop = {
            let c = metadata
                .window_crop_info
                .as_ref()
                .ok_or(Error::MissingField("window_crop_info"))?;
            Size {
                width: c.dst_width,
                height: c.dst_height,
            }
        };

        let tokens: Vec<f64> = metadata
            .offset_v3
            .split('_')
            .map(|token| token.parse::<f64>().map_err(|_| Error::OffsetNotNumeric))
            .collect::<Result<_, _>>()?;

        let lens_count = *tokens.first().ok_or(Error::MissingField("offset_v3"))? as usize;
        let grammar_error = Error::OffsetGrammar {
            lens_count,
            tokens: tokens.len(),
        };
        if lens_count == 0 || tokens.len() != 2 + FIELDS_PER_LENS * lens_count {
            return Err(grammar_error);
        }

        let blocks: Vec<LensBlock> = (0..lens_count)
            .map(|index| LensBlock::read(&tokens, index))
            .collect::<Option<_>>()
            .ok_or(grammar_error)?;

        let canvas = blocks[0].canvas;
        if blocks.iter().any(|block| block.canvas != canvas) {
            return Err(Error::CanvasMismatch);
        }
        // One lens's slot on the shared canvas. That slot, the canvas
        // height and the delivered crop window are all divisors below.
        let slot_width = canvas.width as f64 / lens_count as f64;
        if slot_width == 0.0 || canvas.height == 0 || crop.width == 0 || crop.height == 0 {
            return Err(Error::DegenerateCanvas);
        }

        let lenses = blocks
            .iter()
            .enumerate()
            .map(|(index, block)| block.to_lens(index, dimension, slot_width, canvas.height, crop))
            .collect();

        Ok(Self {
            camera_model: metadata.camera_type.clone(),
            firmware: metadata.fw_version.clone(),
            dimension,
            lenses,
            rolling_shutter_ms: metadata.rolling_shutter_time,
            gyro: GyroConfig::from_metadata(metadata),
            exposure: Default::default(),
            imu: GyroTrack::default(),
            calibration_canvas: canvas,
        })
    }
}

impl GyroConfig {
    fn from_metadata(metadata: &ExtraMetadata) -> Self {
        // Ranges only mean anything to the raw encoding, and only the
        // raw encoding needs them, so a file that lacks them is only a
        // problem if it also claims raw samples. The fallback there is
        // telemetry-parser's, and it is wrong for this camera.
        let encoding = match (metadata.is_raw_gyro, &metadata.gyro_cfg_info) {
            (true, Some(ranges)) => GyroEncoding::Raw {
                accel_range_g: ranges.acc_range as f64,
                gyro_range_dps: ranges.gyro_range as f64,
            },
            (true, None) => GyroEncoding::Raw {
                accel_range_g: 16.0,
                gyro_range_dps: 2000.0,
            },
            (false, _) => GyroEncoding::Scaled,
        };
        Self {
            encoding,
            imu_orientation: imu_orientation(&metadata.camera_type),
            first_frame_timestamp: metadata.first_frame_timestamp,
            gyro_timestamp: match metadata.is_has_gyro_timestamp {
                true => Some(metadata.gyro_timestamp),
                false => None,
            },
        }
    }

    /// How far ahead of the video the IMU's own timestamps run, in
    /// microseconds, which is what comes off every gyro sample's media
    /// time.
    ///
    /// `gyro_timestamp` is **milliseconds** (MED, and the reading is
    /// arithmetic rather than a source: the field reads 1.6 on the X4 Air,
    /// which is 1.6 ms of a 33.4 ms frame. The two other readings that fit
    /// the same number are 1.6 microseconds, which is 20 times finer than
    /// one IMU sample and could not have been worth a field, and 1.6
    /// seconds, which is 48 frames and would be visible as a horizon that
    /// leads the picture). At the roll rates on this footage it is worth
    /// about a fifth of a degree, so it is small either way and it is
    /// applied because the file asks for it.
    fn video_offset_us(&self) -> i64 {
        (self.gyro_timestamp.unwrap_or(0.0) * 1_000.0) as i64
    }
}

/// Which sensor axis feeds which body axis, as the three-letter convention
/// [`super::axis_map`] reads, where a lower case letter is negated.
///
/// **Measured, not transcribed (issue #8).** The string is only half of a
/// convention; the other half is the frame it lands in. Kyerag takes it
/// through [`Pose::sensor_from_body`] into the body frame the reprojection
/// pass uses, which is not the frame any other project's table is written
/// against, so this table is derived from footage rather than copied and
/// copying one would have meant nothing.
///
/// How: for each of the 24 conventions that are rotations, compare the
/// accelerometer's idea of up against the horizon in an **unlocked** rendered
/// frame, which is the true vertical in body coordinates. Over five stretches
/// of two X4 Air captures, 37 to 50 frames each, `xZY` reads 2.3 to 12.2
/// degrees off and the runner-up of the 24 reads 15.1 to 36.6. What is left
/// is the accelerometer's own disagreement with vertical in flight, which is
/// real acceleration and not a convention. The command is
/// `cargo run --release -p kyerag-spike --bin horizon -- <file.insv>
/// from=<seconds> sweep=1`, and docs/research/insv-format.md 8.5 has the
/// tables.
///
/// **The ONE X2 is `Zxy`, and it is not a near miss of the X4's** (issue
/// #79, measured 2026-07-31 on three X2 captures). Held by the X4's `xZY`,
/// an X2's accelerometer points **121 degrees** from where the picture says
/// up is, which is the owner's "horizon is way wrong" and most of his
/// "upside down" as well: the app locks the horizon by default, so a wrong
/// vertical arrives as a picture turned over.
///
/// The 24-way sweep narrows an X2 to two candidates and stops there. `zYX`
/// and `Zxy` are a half turn apart about `(1, -1, 0)`, and on this camera's
/// resting attitudes that half turn moves the accelerometer's up by only
/// 13 degrees, so each wins some stretches: 8.99 against 19.66 at one, 5.86
/// against 25.92 the other way at another. What separates them is that this
/// footage has no true horizon in it - it is a mountain launch, and the
/// sky-to-ground line the finder locks onto is a ridge, which is not level.
///
/// So the last step is not a horizon at all. **Aim the view along what the
/// accelerometer calls up, on a frame where the camera is not moving, and
/// look at what is there.** At rest the accelerometer is gravity and
/// nothing else, so the right convention points at the sky by physics.
/// Three instants across two captures, each with the camera under 5 deg/s
/// and inside 0.015 g of 1 g, and at each of them the two candidates point
/// a half turn apart:
///
/// | capture, instant | `zYX` points at | `Zxy` points at |
/// | --- | --- | --- |
/// | 184419, 1.0 s | bare dirt | sky, and a helmet from below |
/// | 184419, 5.0 s | dirt, and a pair of boots | sky, a helmet and the lines |
/// | 191318, 1.0 s | dirt, and a pair of boots | sky, and a helmet from below |
///
/// A pair of boots seen from above is the nadir. The renders are
/// `kyerag-spike --bin reframe -- <file.insv> time=5 yaw=15.1 pitch=39.4`
/// and its half turn; they are frames of somebody's flying day, so they
/// stay on the box.
///
/// telemetry-parser's no-`offset_v3` table names `xZy` for this model, and
/// that is **not** this answer and could not have been: in Kyerag's frame
/// `xZy` has determinant -1, so it is a reflection rather than a mounting,
/// and the sweep does not even enumerate it. It is carried in
/// `kyerag-spike --bin horizon` as a standing wrong answer instead, where
/// it puts the line 22 degrees from where `Zxy` puts it. That is 8.4's
/// point with a number on it: a string is only half of a convention, and
/// the other half is the frame it lands in.
///
/// Held by `Zxy`, an X2's locked horizon holds to **0.11 degrees of
/// standard deviation and 0.33 peak to peak** over the frames a horizon is
/// findable in; held by `xZY` the finder reads **0 of 120 frames**, which
/// is what a horizon 121 degrees off level looks like from a camera aimed
/// at where it ought to be.
///
/// The default stays the X4's. It is now known to be wrong on one camera
/// family rather than merely unverified, so a model that is not in this
/// table gets a horizon nobody has checked; there is no better guess to
/// make, and the check is one run of the sweep plus the zenith render
/// above.
fn imu_orientation(camera_model: &str) -> &'static str {
    match camera_model {
        m if m.starts_with("Insta360 X4") => "xZY",
        m if m.starts_with("Insta360 X5") => "xZY",
        m if m.starts_with("Insta360 ONE X2") => "Zxy",
        _ => "xZY",
    }
}

/// Which way across the delivered frame this camera's sensor reads out
/// ([`Sweep`]), measured on real footage because the file does not say.
///
/// **The X4 Air reads down the delivered picture** (issue #9, settled
/// 2026-07-31). One lens against itself a few frames apart, with the horizon
/// lock's rigid rotation and the camera's own translation fitted out
/// alongside, over five stretches of a 30-minute capture rolling 68 to 145
/// deg/s: the readout runs **down** the delivered frame at 1.00 +-0.12 of a
/// whole frame in the trailer's own 15.883 ms, and across it at 0.02 +-0.07,
/// which is nothing. Both lenses read down their own pictures, measured
/// separately. Nothing in that fit knows how long a readout takes, so the
/// size landing on the span the trailer records is the check on the shape.
///
/// What took two rounds to get here is that a control has to be injected on
/// **each axis the fit answers on**. Issue #42 injected a readout across the
/// frame only, read it back at 0.79 to 0.84, and reported the down-frame term
/// of the same fits as not repeating. It repeats wherever an injected control
/// works; the stretch that disagreed is the one where injecting a known
/// displacement reads back at -0.10. `kyerag-spike --bin rolling pair=1` now
/// injects all four directions and prints what each reads back.
///
/// The seam says nothing about this and cannot: two sensors reading down
/// their own delivered pictures sweep the **same** world direction, which
/// cancels between them, and the relative displacement a down sweep puts into
/// the seam band measures 0.000 degrees. That is also why switching this on
/// cannot disturb issue #7's blend, where a sweep across the frame would have
/// put 1.9 degrees of misalignment into it.
///
/// X5 and everything else stay [`Sweep::Unknown`], which is a zero axis and
/// therefore no correction: the direction is not in the file and no X5 has
/// been measured. docs/research/insv-format.md 6.7 has the tables.
fn readout_sweep(camera_model: &str) -> Sweep {
    match camera_model {
        m if m.starts_with("Insta360 X4") => Sweep::Down,
        m if m.starts_with("Insta360 X5") => Sweep::Unknown,
        _ => Sweep::Unknown,
    }
}

/// One 19-field lens block, still in canvas coordinates.
struct LensBlock {
    fields: [f64; FIELDS_PER_LENS],
    canvas: Size,
}

impl LensBlock {
    fn read(tokens: &[f64], index: usize) -> Option<Self> {
        let start = 1 + index * FIELDS_PER_LENS;
        let fields: [f64; FIELDS_PER_LENS] = tokens
            .get(start..start + FIELDS_PER_LENS)?
            .try_into()
            .ok()?;
        Some(Self {
            fields,
            canvas: Size {
                width: fields[16] as u32,
                height: fields[17] as u32,
            },
        })
    }

    fn to_lens(&self, index: usize, dimension: Size, slot: f64, canvas_h: u32, crop: Size) -> Lens {
        let [
            xi,
            fx,
            fy,
            cx,
            cy,
            yaw,
            pitch,
            roll,
            tx,
            ty,
            tz,
            k1,
            k2,
            k3,
            p1,
            p2,
            _,
            _,
            lens_type,
        ] = self.fields;
        Lens {
            intrinsics: Intrinsics {
                xi,
                fx: fx * dimension.width as f64 / crop.width as f64,
                fy: fy * dimension.height as f64 / crop.height as f64,
                // Lens i occupies x in [i * slot, (i + 1) * slot) on the
                // shared canvas, so its slot offset comes off first:
                // lens 1's cx of 11550.7 is 7680 + 3870.7.
                cx: (cx - index as f64 * slot) * (dimension.width as f64 / slot),
                cy: cy * (dimension.height as f64 / canvas_h as f64),
            },
            distortion: Distortion { k1, k2, k3, p1, p2 },
            pose: Pose {
                yaw_deg: yaw,
                pitch_deg: pitch,
                roll_deg: roll,
                translation_m: [tx, ty, tz],
            },
            lens_type: lens_type as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use crate::orientation::axis_map;
    use crate::rotation::dot;

    fn calibration() -> CalibrationSet {
        CalibrationSet::from_metadata(&fixture::metadata()).unwrap()
    }

    /// The fixture with its `offset_v3` tokens edited, re-joined the way
    /// the wire carries them.
    fn with_offset_tokens(edit: impl FnOnce(&mut Vec<f64>)) -> ExtraMetadata {
        let mut metadata = fixture::metadata();
        let mut tokens: Vec<f64> = metadata
            .offset_v3
            .split('_')
            .map(|t| t.parse().unwrap())
            .collect();
        edit(&mut tokens);
        metadata.offset_v3 = tokens
            .iter()
            .map(f64::to_string)
            .collect::<Vec<_>>()
            .join("_");
        metadata
    }

    #[track_caller]
    fn near(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} is not within {tolerance} of {expected}"
        );
    }

    #[test]
    fn fixture_describes_a_two_lens_x4_air() {
        let calibration = calibration();
        assert_eq!(calibration.camera_model, "Insta360 X4 Air");
        assert_eq!(calibration.firmware, "v1.2.7_build1");
        assert_eq!(calibration.lenses.len(), 2);
        assert_eq!(
            calibration.dimension,
            Size {
                width: 3840,
                height: 3840
            }
        );
        assert_eq!(
            calibration.calibration_canvas,
            Size {
                width: 15360,
                height: 7680
            }
        );
        assert_eq!(calibration.lenses[0].lens_type, 131);
        near(calibration.rolling_shutter_ms, 15.883, 0.001);
    }

    /// The seam correction is stored under this key, so what it answers has
    /// to be the same for two files off one camera and different for two
    /// cameras: a firmware string that changed and a capture that is a
    /// different length are the same camera, and a lens that sits a
    /// hundredth of a degree elsewhere is not.
    #[test]
    fn the_camera_key_names_the_camera_and_not_the_capture() {
        let key = calibration().camera_key();
        assert_ne!(key, 0);

        let mut later_firmware = fixture::metadata();
        later_firmware.fw_version = "v1.3.0_build4".to_owned();
        let later_firmware = CalibrationSet::from_metadata(&later_firmware).unwrap();
        assert_eq!(later_firmware.camera_key(), key);

        // Lens 1's yaw, a hundredth of a degree off: another unit off the
        // same line, with its own factory measurement.
        let another_unit = with_offset_tokens(|tokens| tokens[1 + FIELDS_PER_LENS + 5] += 0.01);
        let another_unit = CalibrationSet::from_metadata(&another_unit).unwrap();
        assert_ne!(another_unit.camera_key(), key);
    }

    /// A correction fitted in delivered-frame pixels does not survive a
    /// change of delivered frame size, so a capture mode that delivers a
    /// different one is a different camera as far as the store is concerned.
    #[test]
    fn a_different_delivered_frame_is_a_different_key() {
        let mut smaller = fixture::metadata();
        if let Some(dimension) = smaller.dimension.as_mut() {
            dimension.x = 2880;
            dimension.y = 2880;
        }
        let smaller = CalibrationSet::from_metadata(&smaller).unwrap();
        assert_ne!(smaller.camera_key(), calibration().camera_key());
    }

    /// The headline check: both principal points land near the centre of
    /// their own 3840x3840 frame. Lens 1's only does if its slot offset
    /// comes off the canvas coordinate first; without that it lands at
    /// 5775.
    #[test]
    fn principal_points_land_near_frame_centre() {
        let calibration = calibration();

        near(calibration.lenses[0].intrinsics.cx, 1918.94, 0.01);
        near(calibration.lenses[0].intrinsics.cy, 1927.21, 0.01);
        near(calibration.lenses[1].intrinsics.cx, 1935.35, 0.01);
        near(calibration.lenses[1].intrinsics.cy, 1935.09, 0.01);

        for lens in &calibration.lenses {
            near(lens.intrinsics.cx, 1920.0, 20.0);
            near(lens.intrinsics.cy, 1920.0, 20.0);
        }
    }

    #[test]
    fn focal_lengths_scale_by_the_delivered_crop_window() {
        let calibration = calibration();
        // 7087.49 * 3840 / 7424, not * 0.5.
        near(calibration.lenses[0].intrinsics.fx, 3665.94, 0.01);
        near(calibration.lenses[0].intrinsics.fy, 3667.42, 0.01);
        near(calibration.lenses[1].intrinsics.fx, 3671.91, 0.01);
        near(calibration.lenses[1].intrinsics.fy, 3671.08, 0.01);
    }

    #[test]
    fn mei_and_distortion_terms_come_through_unscaled() {
        let calibration = calibration();
        for lens in &calibration.lenses {
            near(lens.intrinsics.xi, 2.31494, 1e-9);
        }

        let distortion = calibration.lenses[0].distortion;
        near(distortion.k1, 0.95820886, 1e-9);
        near(distortion.k2, -1.80141151, 1e-9);
        near(distortion.k3, 3.57555127, 1e-9);
        near(distortion.p1, -0.0007338, 1e-9);
        near(distortion.p2, -0.00115458, 1e-9);
    }

    #[test]
    fn lens_one_carries_the_baseline_and_lens_zero_is_the_origin() {
        let calibration = calibration();
        assert_eq!(calibration.lenses[0].pose.translation_m, [0.0, 0.0, 0.0]);

        let pose = calibration.lenses[1].pose;
        near(pose.translation_m[2], -0.0333, 0.0001);
        // Sub-degree mounting tolerances, not the 180 degree flip.
        near(pose.yaw_deg, 0.039, 1e-9);
        near(pose.pitch_deg, -0.193, 1e-9);
        near(pose.roll_deg, 89.076, 1e-9);
        near(calibration.lenses[0].pose.roll_deg, 90.534, 1e-9);
    }

    #[test]
    fn gyro_config_reads_the_recorded_ranges() {
        let gyro = calibration().gyro;
        assert_eq!(
            gyro.encoding,
            GyroEncoding::Raw {
                accel_range_g: 32.0,
                gyro_range_dps: 2000.0
            }
        );
        assert_eq!(gyro.first_frame_timestamp, 3_848_400);
        assert_eq!(gyro.gyro_timestamp, Some(1.6));
    }

    /// The readout the trailer describes, and the direction it does not: the
    /// span is the file's, and the sweep is the one measured for issue #9,
    /// down the delivered frame on an X4.
    ///
    /// A camera nobody has measured keeps `Unknown`, which is a zero axis and
    /// therefore no correction at all rather than a guess.
    /// docs/research/insv-format.md 6.7 is what both rest on.
    #[test]
    fn the_readout_span_is_the_files_and_the_direction_is_measured() {
        let readout = calibration().readout();

        near(readout.seconds, 0.015_883, 1e-6);
        assert_eq!(readout.sweep, Sweep::Down);
        assert_eq!(readout.sweep.axis(), [0.0, 1.0]);
        assert_eq!(readout_sweep("Insta360 X4 Air"), Sweep::Down);
        assert_eq!(readout_sweep("Insta360 X5"), Sweep::Unknown);
        assert_eq!(readout_sweep("GoPro Max"), Sweep::Unknown);
        assert_eq!(Sweep::Unknown.axis(), [0.0, 0.0]);
    }

    /// The four directions a sensor could be read in, as the map reads them:
    /// a unit step across the delivered frame, or down it, either way round.
    #[test]
    fn a_known_sweep_is_a_unit_direction_in_delivered_pixels() {
        assert_eq!(Sweep::Right.axis(), [1.0, 0.0]);
        assert_eq!(Sweep::Left.axis(), [-1.0, 0.0]);
        assert_eq!(Sweep::Down.axis(), [0.0, 1.0]);
        assert_eq!(Sweep::Up.axis(), [0.0, -1.0]);
    }

    /// The convention the sweep in `kyerag-spike --bin horizon` settled, and
    /// the fixture is the camera it was settled on.
    #[test]
    fn the_x4_air_gets_the_measured_imu_orientation() {
        assert_eq!(calibration().gyro.imu_orientation, "xZY");
        assert_eq!(imu_orientation("Insta360 X4"), "xZY");
        assert_eq!(imu_orientation("Insta360 X5"), "xZY");
    }

    /// **Issue #79.** The ONE X2's IMU is not mounted the way an X4's is, and
    /// the difference is not small: held by the X4's string, an X2's
    /// accelerometer points 121 degrees from the picture's own up.
    ///
    /// The two strings have to be genuinely different rotations, or the
    /// entry above is decoration. The arithmetic for that is here, where a
    /// change to either string breaks it, rather than in a comment.
    #[test]
    fn the_one_x2_gets_its_own_measured_imu_orientation() {
        assert_eq!(imu_orientation("Insta360 ONE X2"), "Zxy");
        assert_ne!(
            imu_orientation("Insta360 ONE X2"),
            imu_orientation("Insta360 X4 Air")
        );

        // Both are rotations, as an IMU's three right-handed axes must be.
        for axes in ["Zxy", "xZY"] {
            assert!((axis_map(axes).determinant() - 1.0).abs() < 1e-12, "{axes}");
        }
        // And they are a long way apart: a resting reading sent through one
        // and then back through the other lands 121 degrees from where it
        // started, which is the size of the defect the owner reported.
        let resting = [0.0, -1.0, 0.0];
        let x2 = axis_map("Zxy").mul_vec(resting);
        let x4 = axis_map("xZY").mul_vec(resting);
        let apart = dot(x2, x4).clamp(-1.0, 1.0).acos().to_degrees();
        assert!(apart > 89.0, "the two conventions are only {apart} apart");
    }

    /// The whole chain from the file to the sensor's axes, checked at the
    /// one instant physics knows the answer for: an accelerometer reading
    /// 1 g up the camera's own vertical is a camera sitting level, and it
    /// has to come out as the body frame's up, which is -y.
    ///
    /// The X4 Air's sensor is rolled 90.534 degrees, so "up the camera's own
    /// vertical" is not up the sensor's: it is the sensor axis the
    /// convention names, and this test is that composition end to end.
    #[test]
    fn a_level_camera_reads_gravity_up_the_body_vertical() {
        let calibration = calibration();
        let to_body = calibration.body_from_imu();
        // What the sensor reads with the camera level, from the same two
        // steps read backwards.
        let level = to_body.transpose().mul_vec([0.0, -1.0, 0.0]);

        let body = to_body.mul_vec(level);
        assert!((body[1] + 1.0).abs() < 1e-9, "{body:?}");
        // And it is a rotation, so the reading keeps its length: an IMU's
        // three axes are right handed and a reflection is not a mounting.
        assert!((to_body.determinant() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_gyro_record_that_is_not_raw_needs_no_ranges() {
        let mut metadata = fixture::metadata();
        metadata.is_raw_gyro = false;
        metadata.gyro_cfg_info = None;

        let calibration = CalibrationSet::from_metadata(&metadata).unwrap();
        assert_eq!(calibration.gyro.encoding, GyroEncoding::Scaled);
    }

    #[test]
    fn a_token_count_that_does_not_fit_the_grammar_is_rejected() {
        let metadata = with_offset_tokens(|tokens| {
            tokens.pop();
        });

        let error = CalibrationSet::from_metadata(&metadata).unwrap_err();
        assert!(
            matches!(
                error,
                Error::OffsetGrammar {
                    lens_count: 2,
                    tokens: 39
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn lens_blocks_that_disagree_about_the_canvas_are_rejected() {
        // Lens 1's calib_w.
        let metadata = with_offset_tokens(|tokens| tokens[1 + FIELDS_PER_LENS + 16] = 16000.0);

        let error = CalibrationSet::from_metadata(&metadata).unwrap_err();
        assert!(matches!(error, Error::CanvasMismatch), "{error:?}");
    }

    #[test]
    fn a_canvas_that_cannot_be_scaled_from_is_rejected() {
        // Both lenses' calib_w, so the blocks still agree.
        let metadata = with_offset_tokens(|tokens| {
            tokens[1 + 16] = 0.0;
            tokens[1 + FIELDS_PER_LENS + 16] = 0.0;
        });

        let error = CalibrationSet::from_metadata(&metadata).unwrap_err();
        assert!(matches!(error, Error::DegenerateCanvas), "{error:?}");
    }

    #[test]
    fn a_missing_field_names_itself() {
        let mut metadata = fixture::metadata();
        metadata.window_crop_info = None;

        let error = CalibrationSet::from_metadata(&metadata).unwrap_err();
        assert!(
            matches!(error, Error::MissingField("window_crop_info")),
            "{error:?}"
        );
    }
}
