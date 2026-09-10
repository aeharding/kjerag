//! Source-derived selected ONE X2 parent-map construction.
//!
//! The fixed sphere basis, calibration packing, 51-pose schedule and Metal
//! law are READ from Studio. Kjerag's orientation track remains Kjerag's pose
//! provider; this module does not claim that provider is Studio's unresolved
//! `PrecomputeStabilization` producer. The caller supplies the exact delivered
//! frame through [`FrameStamp`], so a numeric frame index or timestamp cannot
//! authenticate stale work after a seek.

use std::error::Error;
use std::fmt;
use std::time::Duration;

use kjerag_media::FrameStamp;
use kjerag_meta::{CalibrationSet, OrientationTrack, Quat, Readout};

use super::LensPair;
use super::base_map::FlowstateRoi;
use super::metal_calc_map::{MetalCalcMapParams, diagnostic_metal_calc_map};
use super::parent_inputs::{
    POSE_COUNT, ScanAxis, StaticLensInputs, calibrated_mei_static, diagnostic_pose_times,
    diagnostic_selected_static, x4_model6_static,
};
use crate::stitch_camera::StitchCamera;

/// Why the selected ONE X2 parent map could not be built for one delivery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParentMapError(String);

impl fmt::Display for ParentMapError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(&self.0)
    }
}

impl Error for ParentMapError {}

fn fail(message: impl Into<String>) -> ParentMapError {
    ParentMapError(message.into())
}

/// Calibration-owned reusable inputs for the selected ONE X2 parent mapper.
///
/// Construction performs the bounded camera/model packing once. A frame
/// owner can then build each map pair from its orientation track, readout and
/// exact authenticated container center without repeating static work.
#[derive(Clone, Debug)]
pub struct ParentMapBuilder {
    selected: LensPair<StaticLensInputs>,
    readout: Readout,
}

/// Exact binary32 inputs shared by the scalar oracle and resident GPU stage.
pub(in crate::flow) struct PreparedParentMap {
    pub(super) parameters: LensPair<MetalCalcMapParams>,
    pub(super) poses: [[f32; 4]; POSE_COUNT],
}

impl ParentMapBuilder {
    /// Pack the static selected ONE X2 inputs from one capture calibration.
    pub fn new(calibration: &CalibrationSet) -> Result<Self, ParentMapError> {
        let readout = calibration.readout();
        if !readout.seconds.is_finite() || readout.seconds <= 0.0 {
            return Err(fail(
                "selected ONE X2 parent readout is not finite and positive",
            ));
        }
        let selected = match StitchCamera::from_lenses(&calibration.lenses) {
            Some(StitchCamera::OneX2) => diagnostic_selected_static(calibration),
            Some(StitchCamera::CalibratedMei) if calibration.model6.is_some() => {
                x4_model6_static(calibration)
            }
            Some(StitchCamera::CalibratedMei) => calibrated_mei_static(calibration),
            None => {
                return Err(fail(
                    "resident stitching requires two compatible calibrated Mei lenses",
                ));
            }
        }
        .map_err(|error| fail(format!("selected ONE X2 parent input: {error}")))?;
        Ok(Self { selected, readout })
    }

    /// The calibration-owned readout used by resident parent preparation.
    /// Keeping it behind the builder prevents a capture session from storing
    /// or accepting an independently chosen duplicate.
    pub(super) fn readout(&self) -> Readout {
        self.readout
    }

    /// Build the map pair for one exact delivered frame identity.
    pub fn build_for_frame(
        &self,
        orientation: &OrientationTrack,
        frame: &FrameStamp,
        readout: Readout,
    ) -> Result<LensPair<FlowstateRoi>, ParentMapError> {
        self.build(orientation, frame.timestamp(), readout)
    }

    /// Build from a container center already authenticated by a frame owner.
    ///
    /// This narrow form lets a forensic instrument exercise the production
    /// arithmetic without decoding. Runtime callers retain and bind the
    /// `FrameStamp` which authenticated `center` to the resulting map.
    pub fn build(
        &self,
        orientation: &OrientationTrack,
        center: Duration,
        readout: Readout,
    ) -> Result<LensPair<FlowstateRoi>, ParentMapError> {
        let prepared = self.prepare(orientation, center, readout)?;
        Ok(prepared.scalar_maps())
    }

    pub(super) fn prepare(
        &self,
        orientation: &OrientationTrack,
        center: Duration,
        readout: Readout,
    ) -> Result<PreparedParentMap, ParentMapError> {
        if readout.seconds.to_bits() != self.readout.seconds.to_bits()
            || readout.sweep != self.readout.sweep
        {
            return Err(fail(
                "selected ONE X2 parent readout does not match its calibration",
            ));
        }
        prepare_at_center(&self.selected, orientation, center, readout)
    }
}

impl PreparedParentMap {
    pub(super) fn scalar_maps(&self) -> LensPair<FlowstateRoi> {
        LensPair {
            a: diagnostic_metal_calc_map(&self.parameters.a, &self.poses),
            b: diagnostic_metal_calc_map(&self.parameters.b, &self.poses),
        }
    }

    /// Native `sin`/`atan` are not a bit-portable GPU contract. Selected GPU
    /// playback therefore admits only the same near-quaternion linear branch
    /// that has been qualified bit-for-bit on the active adapter.
    pub(super) fn require_linear_gpu_slerp(&self) -> Result<(), ParentMapError> {
        if self.poses.windows(2).any(|pair| {
            let dot = pair[0][0] * pair[1][0]
                + pair[0][1] * pair[1][1]
                + pair[0][2] * pair[1][2]
                + pair[0][3] * pair[1][3];
            dot.abs() <= 0.9995
        }) {
            return Err(fail(
                "selected ONE X2 GPU parent poses require unqualified nonlinear interpolation",
            ));
        }
        Ok(())
    }
}

fn prepare_at_center(
    selected: &LensPair<StaticLensInputs>,
    orientation: &OrientationTrack,
    center: Duration,
    readout: Readout,
) -> Result<PreparedParentMap, ParentMapError> {
    if orientation.is_empty() {
        return Err(fail("selected ONE X2 parent orientation provider is empty"));
    }

    let center_seconds = center.as_secs_f64();
    let pose_times = diagnostic_pose_times(center_seconds, readout.seconds);
    let lookup_us = pose_times.map(|seconds| seconds * 1_000_000.0);
    require_lookup_range(orientation, &lookup_us)?;

    // READ ONE X2 parent sphere mapping:
    // (body.x, body.y, body.z) -> (body.z, body.x, body.y).
    let sphere_from_body = Quat {
        w: 0.5,
        v: [0.5, 0.5, 0.5],
    };
    let center_world = orientation_at_fractional_us(orientation, center_seconds * 1_000_000.0)?;
    let provider_center = sphere_from_body
        .times(center_world.conjugate())
        .normalized();
    let mapping_base = provider_center.conjugate();
    let poses = lookup_us.map(|time| {
        quat_f32_xyzw(
            sphere_from_body
                .times(
                    orientation_at_fractional_us(orientation, time)
                        .expect("the complete fractional lookup range was checked")
                        .conjugate(),
                )
                .normalized(),
        )
    });
    let mapping_base = quat_f32_xyzw(mapping_base);

    Ok(PreparedParentMap {
        parameters: LensPair {
            a: parameters(&selected.a, mapping_base),
            b: parameters(&selected.b, mapping_base),
        },
        poses,
    })
}

fn parameters(selected: &StaticLensInputs, mapping_base: [f32; 4]) -> MetalCalcMapParams {
    MetalCalcMapParams {
        center: selected.center,
        focal: selected.focal,
        src_size: selected.source_size,
        inv_src_size: [1.0, 1.0],
        qci: selected.lens_quaternion_xyzw,
        qwm: mapping_base,
        // Both READ initial provider calls request the center instant from the
        // same provider, so their frame-local mean removes that pose. Keeping
        // the calibration sign avoids a target-specific quaternion sign.
        q_c0_f0: selected.lens_quaternion_xyzw,
        shift: [0.0, 0.0],
        xi: selected.xi,
        is_horizon_sweep: selected.scan_axis == ScanAxis::Horizontal,
        flip: 1.0,
        max_fov: f32::from_bits(0x4006_0a92),
        distort_coeffs: selected.distortion,
        model6_distortion: selected.model6_distortion,
        pos_scale: selected.source_size,
    }
}

fn require_lookup_range(
    track: &OrientationTrack,
    lookup_us: &[f64; POSE_COUNT],
) -> Result<(), ParentMapError> {
    if lookup_us
        .windows(2)
        .any(|pair| !pair[0].is_finite() || pair[0] >= pair[1])
        || !lookup_us[POSE_COUNT - 1].is_finite()
    {
        return Err(fail(
            "selected ONE X2 parent pose instants are not finite and strictly increasing",
        ));
    }
    let first = track
        .samples()
        .first()
        .ok_or_else(|| fail("selected ONE X2 parent orientation provider is empty"))?
        .offset_us as f64;
    let last = track.samples().last().unwrap().offset_us as f64;
    if lookup_us[0] < first || lookup_us[POSE_COUNT - 1] > last {
        return Err(fail(format!(
            "selected ONE X2 parent pose lookup [{}, {}] is outside orientation track [{first}, {last}]",
            lookup_us[0],
            lookup_us[POSE_COUNT - 1]
        )));
    }
    Ok(())
}

fn orientation_at_fractional_us(
    track: &OrientationTrack,
    offset_us: f64,
) -> Result<Quat, ParentMapError> {
    if !offset_us.is_finite() {
        return Err(fail(
            "selected ONE X2 parent fractional pose lookup is not finite",
        ));
    }
    let samples = track.samples();
    let first = samples
        .first()
        .ok_or_else(|| fail("selected ONE X2 parent orientation provider is empty"))?;
    let last = samples.last().unwrap();
    if offset_us < first.offset_us as f64 || offset_us > last.offset_us as f64 {
        return Err(fail(format!(
            "selected ONE X2 parent fractional pose lookup {offset_us} is outside orientation track [{}, {}]",
            first.offset_us, last.offset_us
        )));
    }
    let after = samples.partition_point(|sample| (sample.offset_us as f64) < offset_us);
    let Some(next) = samples.get(after) else {
        return Ok(last.world_from_body);
    };
    let Some(previous) = after.checked_sub(1).and_then(|index| samples.get(index)) else {
        return Ok(next.world_from_body);
    };
    let span = (next.offset_us - previous.offset_us) as f64;
    if span <= 0.0 {
        return Err(fail(
            "selected ONE X2 parent orientation sample times are not increasing",
        ));
    }
    let fraction = (offset_us - previous.offset_us as f64) / span;
    Ok(previous
        .world_from_body
        .nlerp(next.world_from_body, fraction))
}

fn quat_f32_xyzw(quaternion: Quat) -> [f32; 4] {
    [
        quaternion.v[0] as f32,
        quaternion.v[1] as f32,
        quaternion.v[2] as f32,
        quaternion.w as f32,
    ]
}

#[cfg(test)]
mod tests {
    use kjerag_meta::{
        CalibrationSet, ExposureTrack, GyroConfig, GyroEncoding, GyroTrack, OrientationSample,
        Size, Sweep,
    };
    use sha2::{Digest as _, Sha256};

    use super::*;
    use crate::projection::tests::{FRAME, ONE_XS_FRAME, fixture_lenses, one_xs_lenses};

    const CENTER: Duration = Duration::from_micros(2_000_000);

    fn calibration() -> CalibrationSet {
        CalibrationSet {
            camera_model: "Insta360 ONE X2".to_owned(),
            firmware: "synthetic".to_owned(),
            source_group_type: None,
            dimension: Size {
                width: ONE_XS_FRAME.width,
                height: ONE_XS_FRAME.height,
            },
            lenses: one_xs_lenses(),
            model6: None,
            rolling_shutter_ms: 20.0,
            gyro: GyroConfig {
                encoding: GyroEncoding::Scaled,
                imu_orientation: "Zxy",
                first_frame_timestamp: 0,
                gyro_timestamp: None,
            },
            exposure: [ExposureTrack::default(), ExposureTrack::default()],
            denoise_iso: Default::default(),
            imu: GyroTrack::default(),
            fused: OrientationTrack::default(),
            calibration_canvas: Size {
                width: 6_080,
                height: 3_040,
            },
        }
    }

    fn orientation() -> OrientationTrack {
        OrientationTrack::from_samples(
            (1_980_000..=2_020_000)
                .step_by(2_000)
                .map(|offset_us| OrientationSample {
                    offset_us,
                    world_from_body: Quat::from_rotation_vector([
                        (offset_us - 2_000_000) as f64 * 1.0e-7,
                        (offset_us - 2_000_000) as f64 * -0.5e-7,
                        (offset_us - 2_000_000) as f64 * 0.25e-7,
                    ]),
                })
                .collect(),
        )
    }

    fn x4_air_calibration() -> CalibrationSet {
        CalibrationSet {
            camera_model: "Insta360 X4 Air".to_owned(),
            firmware: "fixture".to_owned(),
            source_group_type: None,
            dimension: Size {
                width: FRAME.width,
                height: FRAME.height,
            },
            lenses: fixture_lenses(),
            model6: None,
            rolling_shutter_ms: 15.882_978_439_331_055,
            gyro: GyroConfig {
                encoding: GyroEncoding::Scaled,
                imu_orientation: "xZY",
                first_frame_timestamp: 0,
                gyro_timestamp: None,
            },
            exposure: [ExposureTrack::default(), ExposureTrack::default()],
            denoise_iso: Default::default(),
            imu: GyroTrack::default(),
            fused: OrientationTrack::default(),
            calibration_canvas: Size {
                width: 15_360,
                height: 7_680,
            },
        }
    }

    fn neutral_orientation() -> OrientationTrack {
        OrientationTrack::from_samples(
            [1_980_000, 2_020_000]
                .into_iter()
                .map(|offset_us| OrientationSample {
                    offset_us,
                    world_from_body: Quat::IDENTITY,
                })
                .collect(),
        )
    }

    fn parent_node_body_ray(row: usize, column: usize) -> [f32; 3] {
        let dst = [
            ((column as f32 * 2.0) * std::f32::consts::PI) / 200.0,
            (row as f32 * std::f32::consts::PI) / 99.0,
        ];
        let theta = 0.5 * std::f32::consts::PI - dst[1];
        let phi = 2.0 * std::f32::consts::PI - dst[0];
        let z = theta.sin();
        let radius = theta.cos();
        let sphere = [radius * phi.cos(), radius * phi.sin(), z];
        // Parent's fixed basis is `sphere = [body.z, body.x, body.y]`.
        [sphere[1], sphere[2], sphere[0]]
    }

    fn map_digest(maps: &LensPair<FlowstateRoi>) -> String {
        let bytes = maps
            .a
            .row_major_values()
            .iter()
            .chain(maps.b.row_major_values())
            .flat_map(|node| node.iter())
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn synthetic_parent_pair_is_deterministic() {
        let calibration = calibration();
        let builder = ParentMapBuilder::new(&calibration).unwrap();
        let first = builder
            .build(&orientation(), CENTER, calibration.readout())
            .unwrap();
        let second = builder
            .build(&orientation(), CENTER, calibration.readout())
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(first.a.rows(), 100);
        assert_eq!(first.a.cols(), 200);
        assert_eq!(first.b.rows(), 100);
        assert_eq!(first.b.cols(), 200);
        assert_eq!(
            map_digest(&first),
            "b03ed396eef59e54aaac55db16817bd8f55030d781cc08ccb109b3224de05481"
        );
    }

    #[test]
    fn calibrated_x4_neutral_parent_matches_generic_projection_before_uv_normalization() {
        let calibration = x4_air_calibration();
        let maps = ParentMapBuilder::new(&calibration)
            .unwrap()
            .build(&neutral_orientation(), CENTER, calibration.readout())
            .unwrap();
        let reframe = crate::Reframe::new(
            &calibration.lenses,
            FRAME,
            crate::Camera::default(),
            crate::Held::default(),
            1.0,
            false,
            crate::Sampling::default(),
        );
        let extent = FRAME.width as f32;
        let mut compared = [0_usize; 2];
        let mut generic_inside = [0_usize; 2];
        let mut parent_valid_generic_outside = [0_usize; 2];
        let mut squared_model_error = [0.0_f64; 2];
        let mut max_model_error = [0.0_f32; 2];
        let mut max_sample_displacement = [0.0_f32; 2];
        let mut max_normalization_residual = [0.0_f32; 2];

        for (lens, map) in [&maps.a, &maps.b].into_iter().enumerate() {
            for row in 0..map.rows() {
                for column in 0..map.cols() {
                    let index = row * map.cols() + column;
                    let uv = map.row_major_values()[index];
                    let parent_valid = uv != [-1.0, -1.0];
                    let generic = reframe.project(lens, parent_node_body_ray(row, column));
                    if generic.inside {
                        generic_inside[lens] += 1;
                        assert!(
                            parent_valid,
                            "generic-valid lens {lens} node ({row},{column})"
                        );
                    } else {
                        parent_valid_generic_outside[lens] += usize::from(parent_valid);
                    }
                    if !parent_valid {
                        continue;
                    }
                    compared[lens] += 1;
                    let parent_pixel = uv.map(|value| value * (extent - 1.0));
                    let model_error = [
                        parent_pixel[0] - generic.pixel[0],
                        parent_pixel[1] - generic.pixel[1],
                    ];
                    let model_distance = model_error[0].hypot(model_error[1]);
                    max_model_error[lens] = max_model_error[lens].max(model_distance);
                    squared_model_error[lens] += f64::from(model_distance * model_distance);

                    // The resident source sampler multiplies parent UV by the
                    // full extent. Separate that known convention from model
                    // disagreement with the ordinary projection itself.
                    let sampled_pixel = uv.map(|value| value * extent);
                    let sample_displacement = [
                        sampled_pixel[0] - generic.pixel[0],
                        sampled_pixel[1] - generic.pixel[1],
                    ];
                    max_sample_displacement[lens] = max_sample_displacement[lens]
                        .max(sample_displacement[0].hypot(sample_displacement[1]));
                    let expected_normalization = generic.pixel.map(|value| value / (extent - 1.0));
                    let normalization_residual = [
                        sample_displacement[0] - expected_normalization[0],
                        sample_displacement[1] - expected_normalization[1],
                    ];
                    max_normalization_residual[lens] = max_normalization_residual[lens]
                        .max(normalization_residual[0].hypot(normalization_residual[1]));
                }
            }
        }

        let rms_model_error: [f32; 2] = std::array::from_fn(|lens| {
            (squared_model_error[lens] / compared[lens] as f64).sqrt() as f32
        });
        eprintln!(
            "X4 neutral parent: compared {compared:?}, generic-inside {generic_inside:?}, \
             parent-valid/generic-outside {parent_valid_generic_outside:?}, model max px \
             {max_model_error:?}, model RMS px {rms_model_error:?}, downstream normalization max \
             px {max_sample_displacement:?}, residual after exact normalization \
             {max_normalization_residual:?}"
        );

        assert!(compared.into_iter().all(|count| count > 10_000));
        assert!(generic_inside.into_iter().all(|count| count > 10_000));
        assert!(max_model_error.into_iter().all(|error| error < 0.01));
        assert!(rms_model_error.into_iter().all(|error| error < 0.001));
        assert!(
            max_normalization_residual
                .into_iter()
                .all(|error| error < 0.011)
        );
        assert!(
            max_sample_displacement
                .into_iter()
                .all(|error| error > 1.0 && error < 1.5)
        );
    }

    #[test]
    fn center_and_fractional_motion_both_affect_the_pair() {
        let calibration = calibration();
        let builder = ParentMapBuilder::new(&calibration).unwrap();
        let base = builder
            .build(&orientation(), CENTER, calibration.readout())
            .unwrap();
        let moved = builder
            .build(
                &orientation(),
                CENTER + Duration::from_micros(1),
                calibration.readout(),
            )
            .unwrap();

        assert_ne!(map_digest(&base), map_digest(&moved));
    }

    #[test]
    fn readout_must_be_the_calibrated_selected_readout() {
        let calibration = calibration();
        let builder = ParentMapBuilder::new(&calibration).unwrap();
        let error = builder
            .build(
                &orientation(),
                CENTER,
                Readout {
                    seconds: calibration.readout().seconds,
                    sweep: Sweep::Up,
                },
            )
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "selected ONE X2 parent readout does not match its calibration"
        );
    }

    #[test]
    fn lookup_refuses_a_track_that_does_not_cover_the_readout() {
        let calibration = calibration();
        let short = OrientationTrack::from_samples(vec![OrientationSample {
            offset_us: 2_000_000,
            world_from_body: Quat::IDENTITY,
        }]);
        let error = ParentMapBuilder::new(&calibration)
            .unwrap()
            .build(&short, CENTER, calibration.readout())
            .unwrap_err();

        assert!(error.to_string().contains("is outside orientation track"));
    }
}
