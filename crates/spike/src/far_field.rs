//! Conservative, camera-agnostic far-field classification for Stage 9 raw
//! stereo observations.
//!
//! This module deliberately knows only the physical baseline and the reported
//! epipolar disparity.  It does not select texture, fit a pose, or generate
//! pixels.  In particular, a small *measured* disparity is not enough to call
//! a point far away: the largest one-sided plausible positive disparity must
//! still triangulate beyond the declared distance.

/// The only distance at which Stage 9 may call an observation proven far.
pub const PROVEN_FAR_METRES: f64 = 300.0;

/// Fixed, predeclared one-sided normal multiplier.  This is not tuned per
/// capture, site, or result.
pub const ONE_SIDED_Z: f64 = 3.0;

/// Far-field status for one raw stereo observation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Classification {
    /// Even the maximum plausible positive disparity triangulates at least
    /// [`PROVEN_FAR_METRES`] away.
    ProvenFar300 {
        /// `baseline / tan(epi + 3 sigma)`: a lower, not upper, bound.
        distance_lower_bound_metres: f64,
        maximum_plausible_epi_rad: f64,
    },
    /// The point disparity describes a finite, physically valid distance,
    /// but the one-sided bound cannot prove it is at least 300 m away.
    FiniteButNotFar {
        point_distance_metres: f64,
        /// Present only when the full one-sided bound remains in the physical
        /// angular domain.  It is a lower bound on distance.
        distance_lower_bound_metres: Option<f64>,
    },
    /// Zero/sign-uncertain disparity is compatible with infinity, but cannot
    /// prove it.  It must never be promoted to a far observation.
    UncertainOrUnplaceable,
    /// The supplied baseline, disparity, or variance is not meaningful.
    Invalid,
}

/// Classify one epipolar stereo result using a one-sided 3-sigma bound.
///
/// For a positive angular disparity `d`, the small-angle stereo relation is
/// represented geometrically as `distance = baseline / tan(d)`.  Since this
/// distance decreases as a positive disparity grows, `d + 3 sigma` gives the
/// conservative *lower* distance bound.  The old Stage 9 proxy used
/// `d - sigma`, which instead gives an upper distance bound and cannot prove
/// that a feature is far away.
pub fn classify(
    baseline_m: [f64; 3],
    epi_disparity_rad: f64,
    epi_variance_rad2: f64,
) -> Classification {
    let baseline = baseline_m
        .iter()
        .map(|axis| axis.powi(2))
        .sum::<f64>()
        .sqrt();
    if !baseline.is_finite()
        || baseline <= 0.0
        || !epi_disparity_rad.is_finite()
        || !epi_variance_rad2.is_finite()
        || epi_variance_rad2 < 0.0
    {
        return Classification::Invalid;
    }
    let sigma = epi_variance_rad2.sqrt();
    let maximum = epi_disparity_rad + ONE_SIDED_Z * sigma;

    // A non-positive point disparity has no finite stereo triangulation in
    // this sign convention.  A positive upper tail merely makes it possible,
    // not proven, so retain it as uncertainty.
    if !(epi_disparity_rad > 0.0 && epi_disparity_rad < std::f64::consts::FRAC_PI_2) {
        return Classification::UncertainOrUnplaceable;
    }
    let point_distance = baseline / epi_disparity_rad.tan();
    if !(point_distance.is_finite() && point_distance > 0.0) {
        return Classification::Invalid;
    }
    if !(maximum > 0.0 && maximum < std::f64::consts::FRAC_PI_2) {
        return Classification::FiniteButNotFar {
            point_distance_metres: point_distance,
            distance_lower_bound_metres: None,
        };
    }
    let lower_bound = baseline / maximum.tan();
    if !(lower_bound.is_finite() && lower_bound > 0.0) {
        return Classification::Invalid;
    }
    if lower_bound >= PROVEN_FAR_METRES {
        Classification::ProvenFar300 {
            distance_lower_bound_metres: lower_bound,
            maximum_plausible_epi_rad: maximum,
        }
    } else {
        Classification::FiniteButNotFar {
            point_distance_metres: point_distance,
            distance_lower_bound_metres: Some(lower_bound),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Classification, ONE_SIDED_Z, PROVEN_FAR_METRES, classify};

    const BASELINE: [f64; 3] = [0.033, 0.0, 0.0];

    #[test]
    fn classifies_the_exact_300_m_lower_bound_as_proven_far() {
        let maximum = (BASELINE[0] / PROVEN_FAR_METRES).atan();
        let got = classify(BASELINE, maximum, 0.0);
        let Classification::ProvenFar300 {
            distance_lower_bound_metres,
            maximum_plausible_epi_rad,
        } = got
        else {
            panic!("the declared threshold is inclusive")
        };
        assert!((distance_lower_bound_metres - PROVEN_FAR_METRES).abs() < 1e-9);
        assert_eq!(maximum_plausible_epi_rad, maximum);
    }

    #[test]
    fn uses_point_plus_three_sigma_not_the_old_subtracted_sigma_proxy() {
        let threshold = (BASELINE[0] / PROVEN_FAR_METRES).atan();
        let sigma = threshold * 0.2;
        let point = threshold - ONE_SIDED_Z * sigma * 0.5;
        let got = classify(BASELINE, point, sigma * sigma);
        let Classification::FiniteButNotFar {
            point_distance_metres,
            distance_lower_bound_metres: Some(lower_bound),
        } = got
        else {
            panic!("the +3 sigma maximum must prevent a far claim")
        };
        assert!(point_distance_metres > PROVEN_FAR_METRES);
        assert!(lower_bound < PROVEN_FAR_METRES);
    }

    #[test]
    fn distinguishes_finite_near_from_sign_uncertain() {
        assert!(matches!(
            classify(BASELINE, 0.02, 1e-8),
            Classification::FiniteButNotFar { .. }
        ));
        assert_eq!(
            classify(BASELINE, 0.0, 1e-8),
            Classification::UncertainOrUnplaceable
        );
        assert_eq!(
            classify(BASELINE, -0.001, 1e-8),
            Classification::UncertainOrUnplaceable
        );
    }

    #[test]
    fn rejects_invalid_measurement_inputs_without_turning_them_into_depth() {
        assert_eq!(classify([0.0; 3], 0.02, 1e-8), Classification::Invalid);
        assert_eq!(classify(BASELINE, f64::NAN, 1e-8), Classification::Invalid);
        assert_eq!(classify(BASELINE, 0.02, -1e-8), Classification::Invalid);
    }

    #[test]
    fn a_bound_outside_the_physical_angle_does_not_become_proven_far() {
        let got = classify(BASELINE, 0.02, 1.0);
        assert!(matches!(
            got,
            Classification::FiniteButNotFar {
                distance_lower_bound_metres: None,
                ..
            }
        ));
    }
}
