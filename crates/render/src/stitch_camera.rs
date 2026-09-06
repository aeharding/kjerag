//! Camera interpretation at the boundary of the shared resident stitcher.
//!
//! Solver grids, temporal processing, frame ownership and GPU drawing are
//! shared. The captured ONE X2 geometry stays a distinct compatibility law;
//! X4 Air residual-pose Mei lenses use Kjerag's calibrated projection law.
//! Selecting that law does not claim Studio parity on an unreviewed camera.

use kjerag_meta::{Lens, Model};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StitchCamera {
    OneX2,
    CalibratedMei,
}

impl StitchCamera {
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
