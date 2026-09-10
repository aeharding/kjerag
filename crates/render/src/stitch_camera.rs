//! Camera interpretation at the boundary of the shared resident stitcher.
//!
//! Solver grids, temporal processing, frame ownership and GPU drawing are
//! shared. The captured ONE X2 geometry stays a distinct compatibility law;
//! X4 Air uses the captured model-6 Template geometry, associated once with
//! delivered source lanes. Its v3-only classifier remains a reference boundary;
//! live X4 admission requires the extended calibration.

use kjerag_meta::{CalibrationSet, Lens, Model};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StitchCamera {
    OneX2,
    CalibratedMei,
}

impl StitchCamera {
    /// Native fusion ordinal to delivered source stream. X4's geometry
    /// adapter resolves this same association in `x4_model6_static`; color
    /// measurement and ratio publication must cross that boundary as well.
    pub(crate) fn fusion_streams(self) -> [usize; 2] {
        match self {
            Self::OneX2 => [0, 1],
            Self::CalibratedMei => [1, 0],
        }
    }

    /// Camera admission for live resident playback.
    ///
    /// The generic lens-only classifier remains useful to resource and
    /// reference-map code, but the selected X4 path additionally requires the
    /// extended calibration its parent projector consumes.
    pub(crate) fn from_calibration(calibration: &CalibrationSet) -> Option<Self> {
        match Self::from_lenses(&calibration.lenses) {
            Some(Self::CalibratedMei) if calibration.model6.is_none() => None,
            camera => camera,
        }
    }

    pub(crate) fn from_lenses(lenses: &[Lens]) -> Option<Self> {
        let [a, b] = lenses else { return None };
        if [a, b]
            .iter()
            .any(|lens| lens.model != Model::Mei || lens.mounting.is_some())
        {
            return None;
        }
        // These are the two camera families exercised through real playback.
        // The calibrated law is reusable, but accepting arbitrary residual
        // poses would also require aligning the solver chart with their seam.
        match [a.lens_type, b.lens_type] {
            [41, 41] => Some(Self::OneX2),
            [131, 131] => Some(Self::CalibratedMei),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::tests::{fixture_lenses, one_xs_lenses};
    use kjerag_meta::{ExposureTrack, GyroConfig, GyroEncoding, GyroTrack, OrientationTrack, Size};

    fn calibration(lenses: Vec<Lens>) -> CalibrationSet {
        CalibrationSet {
            camera_model: "admission fixture".to_owned(),
            firmware: String::new(),
            dimension: Size {
                width: 3_840,
                height: 3_840,
            },
            lenses,
            model6: None,
            rolling_shutter_ms: 1.0,
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
                width: 7_680,
                height: 3_840,
            },
        }
    }

    #[test]
    fn camera_laws_share_admission_without_changing_lens_identity() {
        assert_eq!(
            StitchCamera::from_lenses(&one_xs_lenses()),
            Some(StitchCamera::OneX2)
        );
        let x4 = fixture_lenses();
        assert_eq!(
            StitchCamera::from_lenses(&x4),
            Some(StitchCamera::CalibratedMei)
        );
        assert_eq!(x4[0].lens_type, 131);
        assert_eq!(x4[1].lens_type, 131);
    }

    #[test]
    fn live_x4_admission_requires_model6_without_changing_one_x2() {
        let one_x2 = calibration(one_xs_lenses());
        assert_eq!(
            StitchCamera::from_calibration(&one_x2),
            Some(StitchCamera::OneX2)
        );

        let mut x4 = calibration(fixture_lenses());
        assert_eq!(StitchCamera::from_calibration(&x4), None);
        x4.model6 = Some(Vec::new());
        assert_eq!(
            StitchCamera::from_calibration(&x4),
            Some(StitchCamera::CalibratedMei)
        );
    }

    #[test]
    fn incomplete_extra_and_mixed_native_pairs_are_not_admitted() {
        let mut lenses = one_xs_lenses();
        assert_eq!(StitchCamera::from_lenses(&[]), None);
        assert_eq!(StitchCamera::from_lenses(&lenses[..1]), None);
        lenses.push(lenses[0].clone());
        assert_eq!(StitchCamera::from_lenses(&lenses), None);
        lenses.pop();
        lenses[1] = fixture_lenses()[1].clone();
        assert_eq!(StitchCamera::from_lenses(&lenses), None);
        for lens in &mut lenses {
            lens.lens_type = 0;
        }
        assert_eq!(StitchCamera::from_lenses(&lenses), None);
    }
}
