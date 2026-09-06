//! Inputs immediately above Studio's selected ONE X2 Metal parent mapper.
//!
//! Static model packing and the 51-time construction are READ.  Studio's
//! upstream `PrecomputeStabilization` pose-cache producer and the general
//! retained mapping operand are not, so this module deliberately cannot make
//! a complete [`super::metal_calc_map::MetalCalcMapParams`] or enter playback.

use std::error::Error;
use std::fmt;

use kjerag_meta::{CalibrationSet, Model, Size, Sweep};

use super::{LENS_TYPE, LensPair};
use crate::projection::{calibrated_parent_lens_quaternion, one_xs_parent_lens_quaternion};

/// The selected mapper requests this many poses across one sensor readout.
pub const POSE_COUNT: usize = 51;

/// The selected scan coordinate used to interpolate the pose batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanAxis {
    Horizontal,
    Vertical,
}

/// Calibration-derived binary32 fields for one selected model-3 Metal call.
///
/// Runtime scale, offset, mapping-base, initial-composed and pose values are
/// intentionally absent.  Those are not calibration and must remain visible
/// at the later completion boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectedModel3Static {
    pub center: [f32; 2],
    pub focal: [f32; 2],
    pub source_size: [f32; 2],
    pub lens_quaternion_xyzw: [f32; 4],
    pub xi: f32,
    pub distortion: [f32; 5],
    pub scan_axis: ScanAxis,
}

/// Why a calibration cannot take the bounded selected ONE X2 adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticInputError(String);

impl fmt::Display for StaticInputError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(&self.0)
    }
}

impl Error for StaticInputError {}

fn fail(message: impl Into<String>) -> StaticInputError {
    StaticInputError(message.into())
}

/// Pack the calibration-derived part of the selected ONE X2 Metal calls.
///
/// The 2880-square boundary is deliberate: it is the delivery mode whose
/// Studio producer and host uploads are read.  A different ONE X2 mode needs
/// its own native setup evidence before this adapter may generalize it.
pub fn diagnostic_selected_static(
    calibration: &CalibrationSet,
) -> Result<LensPair<SelectedModel3Static>, StaticInputError> {
    if !calibration.camera_model.starts_with("Insta360 ONE X2") {
        return Err(fail(format!(
            "camera model {} is not the selected Insta360 ONE X2 route",
            calibration.camera_model
        )));
    }
    if calibration.dimension
        != (Size {
            width: 2_880,
            height: 2_880,
        })
    {
        return Err(fail(format!(
            "selected ONE X2 parent input is 2880 by 2880, not {} by {}",
            calibration.dimension.width, calibration.dimension.height
        )));
    }
    let [left, right]: [&kjerag_meta::Lens; 2] = calibration
        .lenses
        .iter()
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|lenses: Vec<_>| {
            fail(format!(
                "selected ONE X2 parent input has two lenses, not {}",
                lenses.len()
            ))
        })?;
    for (index, lens) in [left, right].into_iter().enumerate() {
        if lens.lens_type != LENS_TYPE || lens.model != Model::Mei || lens.mounting.is_some() {
            return Err(fail(format!(
                "selected ONE X2 lens {index} is not the read type-41 residual Mei model"
            )));
        }
    }
    let scan_axis = match calibration.readout().sweep {
        Sweep::Down => ScanAxis::Vertical,
        sweep => {
            return Err(fail(format!(
                "selected ONE X2 parent input reads down, not {sweep:?}"
            )));
        }
    };

    let pack = |lens: &kjerag_meta::Lens, index| {
        let quaternion = one_xs_parent_lens_quaternion(lens, index);
        SelectedModel3Static {
            center: lens.crop_centre.map(|value| value as f32),
            focal: [lens.intrinsics.fx as f32, lens.intrinsics.fy as f32],
            source_size: [
                calibration.dimension.width as f32,
                calibration.dimension.height as f32,
            ],
            lens_quaternion_xyzw: [
                quaternion.v[0] as f32,
                quaternion.v[1] as f32,
                quaternion.v[2] as f32,
                quaternion.w as f32,
            ],
            xi: lens.intrinsics.xi as f32,
            distortion: [
                lens.distortion.k1 as f32,
                lens.distortion.k2 as f32,
                lens.distortion.k3 as f32,
                lens.distortion.p1 as f32,
                lens.distortion.p2 as f32,
            ],
            scan_axis,
        }
    };
    Ok(LensPair {
        a: pack(left, 0),
        b: pack(right, 1),
    })
}

/// Pack the calibration-derived parent inputs for a generic calibrated Mei pair.
///
/// This is deliberately not another camera selector and does not relax the
/// native ONE X2 diagnostic above. The caller owns camera admission. This
/// adapter validates only the geometry it consumes, uses the ordinary
/// renderer mounting, and takes its projection centre from the calibrated Mei
/// principal point rather than ONE X2's captured native crop centre.
pub(crate) fn calibrated_mei_static(
    calibration: &CalibrationSet,
) -> Result<LensPair<SelectedModel3Static>, StaticInputError> {
    let dimension = calibration.dimension;
    if dimension.width == 0 || dimension.height == 0 || dimension.width != dimension.height {
        return Err(fail(format!(
            "calibrated Mei parent input must be a nonzero square, not {} by {}",
            dimension.width, dimension.height
        )));
    }
    let [left, right]: [&kjerag_meta::Lens; 2] = calibration
        .lenses
        .iter()
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|lenses: Vec<_>| {
            fail(format!(
                "calibrated Mei parent input has {} lenses, expected 2",
                lenses.len()
            ))
        })?;
    let scan_axis = match calibration.readout().sweep {
        Sweep::Down => ScanAxis::Vertical,
        sweep => {
            return Err(fail(format!(
                "calibrated Mei parent input has unsupported readout {sweep:?}"
            )));
        }
    };

    let pack = |lens: &kjerag_meta::Lens, index| {
        if lens.model != Model::Mei {
            return Err(fail(format!(
                "calibrated Mei parent lens {index} does not use a Mei model"
            )));
        }
        let numbers = [
            lens.intrinsics.xi,
            lens.intrinsics.fx,
            lens.intrinsics.fy,
            lens.intrinsics.cx,
            lens.intrinsics.cy,
            lens.distortion.k1,
            lens.distortion.k2,
            lens.distortion.k3,
            lens.distortion.p1,
            lens.distortion.p2,
            lens.pose.yaw_deg,
            lens.pose.pitch_deg,
            lens.pose.roll_deg,
            lens.pose.translation_m[0],
            lens.pose.translation_m[1],
            lens.pose.translation_m[2],
        ];
        if numbers.iter().any(|value| !value.is_finite())
            || lens.intrinsics.fx <= 0.0
            || lens.intrinsics.fy <= 0.0
        {
            return Err(fail(format!(
                "calibrated Mei parent lens {index} has invalid calibration values"
            )));
        }
        if let Some(mounting) = lens.mounting
            && mounting
                .rows()
                .iter()
                .flatten()
                .any(|value| !value.is_finite())
        {
            return Err(fail(format!(
                "calibrated Mei parent lens {index} has an invalid mounting"
            )));
        }

        let quaternion = calibrated_parent_lens_quaternion(lens, index);
        Ok(SelectedModel3Static {
            center: [lens.intrinsics.cx as f32, lens.intrinsics.cy as f32],
            focal: [lens.intrinsics.fx as f32, lens.intrinsics.fy as f32],
            source_size: [dimension.width as f32, dimension.height as f32],
            lens_quaternion_xyzw: [
                quaternion.v[0] as f32,
                quaternion.v[1] as f32,
                quaternion.v[2] as f32,
                quaternion.w as f32,
            ],
            xi: lens.intrinsics.xi as f32,
            distortion: [
                lens.distortion.k1 as f32,
                lens.distortion.k2 as f32,
                lens.distortion.k3 as f32,
                lens.distortion.p1 as f32,
                lens.distortion.p2 as f32,
            ],
            scan_axis,
        })
    };

    Ok(LensPair {
        a: pack(left, 0)?,
        b: pack(right, 1)?,
    })
}

/// Reproduce the selected host's 51 binary64 pose-request instants.
///
/// `parent_timestamp_seconds` is the already converted result of Studio's
/// `RuntimeStabilization::GetReallyGyroTimestampInMs` multiplied by the
/// binary64 value `0.001`.
/// The converter between `IdxTimed` and that value remains outside this
/// adapter and has not yet been admitted by a completed live capture.  It is
/// not raw PTS or Kjerag's exposure clock.  The
/// fifty coefficients below are the pre-rounded binary64 `j/50` literals for
/// `j = 0..49` embedded by Studio; the first fifty operations are fused and
/// the final endpoint is one add.
pub fn diagnostic_pose_times(
    parent_timestamp_seconds: f64,
    duration_seconds: f64,
) -> [f64; POSE_COUNT] {
    let start = duration_seconds.mul_add(-0.5, parent_timestamp_seconds);
    let mut times = [0.0; POSE_COUNT];
    for (index, fraction) in POSE_FRACTIONS.into_iter().enumerate() {
        times[index] = duration_seconds.mul_add(fraction, start);
    }
    times[POSE_COUNT - 1] = start + duration_seconds;
    times
}

const POSE_FRACTIONS: [f64; 50] = [
    f64::from_bits(0x0000000000000000),
    f64::from_bits(0x3f947ae147ae147b),
    f64::from_bits(0x3fa47ae147ae147b),
    f64::from_bits(0x3faeb851eb851eb8),
    f64::from_bits(0x3fb47ae147ae147b),
    f64::from_bits(0x3fb999999999999a),
    f64::from_bits(0x3fbeb851eb851eb8),
    f64::from_bits(0x3fc1eb851eb851ec),
    f64::from_bits(0x3fc47ae147ae147b),
    f64::from_bits(0x3fc70a3d70a3d70a),
    f64::from_bits(0x3fc999999999999a),
    f64::from_bits(0x3fcc28f5c28f5c29),
    f64::from_bits(0x3fceb851eb851eb8),
    f64::from_bits(0x3fd0a3d70a3d70a4),
    f64::from_bits(0x3fd1eb851eb851ec),
    f64::from_bits(0x3fd3333333333333),
    f64::from_bits(0x3fd47ae147ae147b),
    f64::from_bits(0x3fd5c28f5c28f5c3),
    f64::from_bits(0x3fd70a3d70a3d70a),
    f64::from_bits(0x3fd851eb851eb852),
    f64::from_bits(0x3fd999999999999a),
    f64::from_bits(0x3fdae147ae147ae1),
    f64::from_bits(0x3fdc28f5c28f5c29),
    f64::from_bits(0x3fdd70a3d70a3d71),
    f64::from_bits(0x3fdeb851eb851eb8),
    f64::from_bits(0x3fe0000000000000),
    f64::from_bits(0x3fe0a3d70a3d70a4),
    f64::from_bits(0x3fe147ae147ae148),
    f64::from_bits(0x3fe1eb851eb851ec),
    f64::from_bits(0x3fe28f5c28f5c28f),
    f64::from_bits(0x3fe3333333333333),
    f64::from_bits(0x3fe3d70a3d70a3d7),
    f64::from_bits(0x3fe47ae147ae147b),
    f64::from_bits(0x3fe51eb851eb851f),
    f64::from_bits(0x3fe5c28f5c28f5c3),
    f64::from_bits(0x3fe6666666666666),
    f64::from_bits(0x3fe70a3d70a3d70a),
    f64::from_bits(0x3fe7ae147ae147ae),
    f64::from_bits(0x3fe851eb851eb852),
    f64::from_bits(0x3fe8f5c28f5c28f6),
    f64::from_bits(0x3fe999999999999a),
    f64::from_bits(0x3fea3d70a3d70a3d),
    f64::from_bits(0x3feae147ae147ae1),
    f64::from_bits(0x3feb851eb851eb85),
    f64::from_bits(0x3fec28f5c28f5c29),
    f64::from_bits(0x3feccccccccccccd),
    f64::from_bits(0x3fed70a3d70a3d71),
    f64::from_bits(0x3fee147ae147ae14),
    f64::from_bits(0x3feeb851eb851eb8),
    f64::from_bits(0x3fef5c28f5c28f5c),
];

#[cfg(test)]
mod tests {
    use kjerag_meta::{
        CalibrationSet, ExposureTrack, GyroConfig, GyroEncoding, GyroTrack, OrientationTrack, Size,
    };
    use sha2::{Digest as _, Sha256};

    use super::*;
    use crate::projection::tests::{ONE_XS_FRAME, one_xs_lenses};

    fn calibration() -> CalibrationSet {
        CalibrationSet {
            camera_model: "Insta360 ONE X2".to_owned(),
            firmware: "v1.0.62_build2".to_owned(),
            dimension: Size {
                width: ONE_XS_FRAME.width,
                height: ONE_XS_FRAME.height,
            },
            lenses: one_xs_lenses(),
            rolling_shutter_ms: 23.516_071_319_580_078,
            gyro: GyroConfig {
                encoding: GyroEncoding::Scaled,
                imu_orientation: "Zxy",
                first_frame_timestamp: 4_254,
                gyro_timestamp: None,
            },
            exposure: [ExposureTrack::default(), ExposureTrack::default()],
            imu: GyroTrack::default(),
            fused: OrientationTrack::default(),
            calibration_canvas: Size {
                width: 6_080,
                height: 3_040,
            },
        }
    }

    fn x4_air_calibration() -> CalibrationSet {
        CalibrationSet {
            camera_model: "Insta360 X4 Air".to_owned(),
            firmware: "v1.2.7_build1".to_owned(),
            dimension: Size {
                width: 3_840,
                height: 3_840,
            },
            lenses: crate::projection::tests::fixture_lenses(),
            rolling_shutter_ms: 15.882_978_439_331_055,
            gyro: GyroConfig {
                encoding: GyroEncoding::Raw {
                    accel_range_g: 32.0,
                    gyro_range_dps: 2_000.0,
                },
                imu_orientation: "xZY",
                first_frame_timestamp: 3_848_400,
                gyro_timestamp: Some(1.6),
            },
            exposure: [ExposureTrack::default(), ExposureTrack::default()],
            imu: GyroTrack::default(),
            fused: OrientationTrack::default(),
            calibration_canvas: Size {
                width: 15_360,
                height: 7_680,
            },
        }
    }

    #[test]
    fn selected_static_inputs_match_the_captured_binary32_uploads() {
        let calibration = calibration();
        let packed = diagnostic_selected_static(&calibration).unwrap();
        let bits = |values: &[f32]| {
            values
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>()
        };

        assert_eq!(bits(&packed.a.center), [0x44b5_3d71, 0x44b5_423d]);
        assert_eq!(bits(&packed.b.center), [0x44b3_e7ae, 0x44b2_ed71]);
        assert_ne!(
            packed.b.center[0].to_bits(),
            calibration.lenses[1].image_circle_centre[0].to_bits()
        );
        assert_eq!(bits(&packed.a.focal), [0x4511_6400, 0x4511_5f33]);
        assert_eq!(bits(&packed.b.focal), [0x4511_175c, 0x4511_1b33]);
        assert_eq!(bits(&packed.a.source_size), [0x4534_0000, 0x4534_0000]);
        assert_eq!(packed.a.xi.to_bits(), 0x3fdd_4270);
        assert_eq!(packed.b.xi.to_bits(), 0x3fdd_4270);
        assert_eq!(
            bits(&packed.a.lens_quaternion_xyzw),
            [0x3b8a_3665, 0xbf36_6657, 0xbbf9_aa3d, 0x3f33_9d4e]
        );
        assert_eq!(
            bits(&packed.b.lens_quaternion_xyzw),
            [0x3a18_01c1, 0x3f33_085f, 0xbc3b_2352, 0x3f36_f602]
        );
        assert_eq!(packed.a.scan_axis, ScanAxis::Vertical);
        assert_eq!(packed.b.scan_axis, ScanAxis::Vertical);
    }

    #[test]
    fn selected_pose_times_reproduce_the_complete_captured_schedule() {
        let parent_timestamp = f64::from_bits(0x406b_1862_bf6b_712d);
        let duration = f64::from_bits(0x3f98_1498_d4fd_f3b6);
        let times = diagnostic_pose_times(parent_timestamp, duration);

        assert_eq!(times[0].to_bits(), 0x406b_1802_6d08_1d35);
        assert_eq!(times[25].to_bits(), parent_timestamp.to_bits());
        assert_eq!(times[50].to_bits(), 0x406b_18c3_11ce_c525);
        let bytes = times
            .into_iter()
            .flat_map(f64::to_le_bytes)
            .collect::<Vec<_>>();
        assert_eq!(
            Sha256::digest(bytes).as_slice(),
            &[
                0xcd, 0x1d, 0x97, 0xbf, 0xdf, 0x7c, 0xe5, 0xd0, 0xda, 0x35, 0xb3, 0xc6, 0x78, 0xf7,
                0x56, 0x66, 0x88, 0x50, 0x18, 0x01, 0x50, 0xa6, 0x35, 0xdc, 0x4a, 0xc6, 0xe0, 0x43,
                0xc6, 0x77, 0x72, 0xb7,
            ]
        );
    }

    #[test]
    fn calibrated_x4_air_inputs_use_delivered_intrinsics_and_dynamic_size() {
        let calibration = x4_air_calibration();
        let packed = calibrated_mei_static(&calibration).unwrap();

        assert_eq!(packed.a.center, [1918.94_f32, 1927.21_f32]);
        assert_eq!(packed.b.center, [1935.35_f32, 1935.09_f32]);
        assert_ne!(
            packed.a.center,
            calibration.lenses[0].crop_centre.map(|value| value as f32)
        );
        assert_ne!(
            packed.b.center,
            calibration.lenses[1].crop_centre.map(|value| value as f32)
        );
        assert_eq!(packed.a.focal, [3665.9397_f32, 3667.4194_f32]);
        assert_eq!(packed.b.focal, [3671.9126_f32, 3671.0823_f32]);
        assert_eq!(packed.a.source_size, [3840.0, 3840.0]);
        assert_eq!(packed.b.source_size, [3840.0, 3840.0]);
        assert_eq!(packed.a.xi, 2.31494_f32);
        assert_eq!(packed.a.scan_axis, ScanAxis::Vertical);
        assert_eq!(packed.b.scan_axis, ScanAxis::Vertical);
    }

    #[test]
    fn calibrated_mei_inputs_validate_the_geometry_they_consume() {
        let mut calibration = x4_air_calibration();
        calibration.dimension.width = 0;
        assert!(calibrated_mei_static(&calibration).is_err());

        let mut calibration = x4_air_calibration();
        calibration.dimension.height -= 1;
        assert!(calibrated_mei_static(&calibration).is_err());

        let mut calibration = x4_air_calibration();
        calibration.lenses[1].model = Model::Theta { k: [0.0; 5] };
        assert!(calibrated_mei_static(&calibration).is_err());

        let mut calibration = x4_air_calibration();
        calibration.lenses[1].intrinsics.fx = f64::NAN;
        assert!(calibrated_mei_static(&calibration).is_err());
    }
}
