//! The `offset_v3` lens calibration, turned into numbers a shader can
//! use directly.
//!
//! Grammar, worked example and provenance for every constant here:
//! `docs/research/insv-format.md` sections 3 and 4.

use super::Error;
use super::trailer::ExtraMetadata;

/// `offset_v3` is `lens_count`, then this many fields per lens, then a
/// version word. The field order inside a block is fixed:
/// `xi, fx, fy, cx, cy, yaw, pitch, roll, tx, ty, tz, k1, k2, k3, p1,
/// p2, calib_w, calib_h, lensType`.
const FIELDS_PER_LENS: usize = 19;

/// A width and height in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// The camera's geometric self-description, read from the `.insv`
/// trailer.
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
    /// this slot (docs/research/insv-format.md 4.7). How to compose the
    /// three angles into a rotation is still open.
    pub roll_deg: f64,
    /// Metres, lens 0 at the origin. Lens 1 is dominated by z, and that
    /// is the inter-lens baseline that sets parallax: -0.033284 m on the
    /// fixture, -0.03132 m on a non-Air X4.
    pub translation_m: [f64; 3],
}

/// What it takes to read the gyro record and place its samples in time.
#[derive(Debug, Clone)]
pub struct GyroConfig {
    pub encoding: GyroEncoding,
    /// telemetry-parser's axis convention: three letters naming which
    /// sensor axis feeds x, y and z, uppercase meaning negated. See
    /// [`imu_orientation`].
    pub imu_orientation: &'static str,
    /// `first_frame_timestamp`, the clock origin in microseconds.
    pub first_frame_timestamp_us: i64,
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
    /// Interpret the trailer's metadata record.
    pub(crate) fn from_metadata(metadata: &ExtraMetadata) -> Result<Self, Error> {
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
            first_frame_timestamp_us: metadata.first_frame_timestamp,
            gyro_timestamp: match metadata.is_has_gyro_timestamp {
                true => Some(metadata.gyro_timestamp),
                false => None,
            },
        }
    }
}

/// The IMU axis convention for a camera that has an `offset_v3`
/// calibration, which is the only kind Kyerag reads. Upstream's table is
/// two-dimensional (model crossed with whether `offset_v3` is present);
/// this is that half of it.
///
/// telemetry-parser matches `Some("Insta360 X4")` exactly, so an
/// `Insta360 X4 Air` falls through to the default `Xyz` and its horizon
/// tilts. Matching the family fixes that. The Air is assumed to share
/// the X4's IMU mounting; unverified, because no horizon has been
/// rendered yet (issue #8).
fn imu_orientation(camera_model: &str) -> &'static str {
    match camera_model {
        m if m.starts_with("Insta360 X4") => "yzX",
        m if m.starts_with("Insta360 X5") => "yzX",
        _ => "Xyz",
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
        assert_eq!(gyro.first_frame_timestamp_us, 3_848_400);
        assert_eq!(gyro.gyro_timestamp, Some(1.6));
    }

    /// telemetry-parser matches "Insta360 X4" exactly and drops the Air
    /// on the default, which tilts the horizon.
    #[test]
    fn the_x4_air_gets_the_x4_imu_orientation() {
        assert_eq!(calibration().gyro.imu_orientation, "yzX");
        assert_eq!(imu_orientation("Insta360 X4"), "yzX");
        assert_eq!(imu_orientation("Insta360 X5"), "yzX");
        assert_eq!(imu_orientation("Insta360 ONE X2"), "Xyz");
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
