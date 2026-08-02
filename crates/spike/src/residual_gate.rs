//! Evidence gate for a future, camera-agnostic source-coordinate residual map.
//!
//! This module deliberately does *not* estimate a residual, choose a numeric
//! acceptance bound, or emit a map.  It only records whether one pre-declared
//! site is eligible as an independent held-out observation.  Any later map
//! fitter must remain downstream of this gate and calibrate its own numeric
//! thresholds from a declared corpus.

use crate::{
    far_field::Classification,
    raw_register::{BidirectionalOutcome, StripSite, TrackClosure, TrackClosureRefused},
};

/// Assignment made before evaluating a residual-map candidate.
///
/// Only held-out evidence can pass this gate.  Training assignments are kept
/// explicit so an observation cannot silently become its own validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assignment {
    Training,
    HeldOut,
}

/// All non-image evidence required for one future residual-map observation.
///
/// The `site` is repeated deliberately: both controls must refer to exactly
/// that declared physical location, rather than a nearby textured substitute.
#[derive(Clone, Copy, Debug)]
pub struct Evidence {
    pub site: StripSite,
    pub far_field: Classification,
    pub reciprocal: BidirectionalOutcome,
    pub temporal_closure: Result<TrackClosure, TrackClosureRefused>,
    pub assignment: Assignment,
}

/// Why an observation cannot be used as an independent residual-map check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    NotProvenFar300,
    NotHeldOut,
    ReciprocalUnavailable,
    ForwardReverseClosureUnavailable,
    EvidenceSiteMismatch,
}

/// A site eligible for a future independent validation pass.
///
/// This is intentionally only a site declaration.  It has no pixel offset,
/// source coordinate, gain, interpolation, or renderer-facing map value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeldOutCandidate {
    pub site: StripSite,
}

/// Conservative result of the non-rendering evidence gate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decision {
    HeldOutEligible(HeldOutCandidate),
    Refused(Refusal),
}

/// Admit only a proven-far, held-out site with both successful controls.
///
/// Success here means the reciprocal and temporal operations completed at the
/// same declared site.  It deliberately does not interpret the magnitude of
/// either closure: numerical acceptance must be calibrated externally, not
/// smuggled in as an arbitrary threshold in this gate.
pub fn gate(evidence: Evidence) -> Decision {
    if !matches!(evidence.far_field, Classification::ProvenFar300 { .. }) {
        return Decision::Refused(Refusal::NotProvenFar300);
    }
    if evidence.assignment != Assignment::HeldOut {
        return Decision::Refused(Refusal::NotHeldOut);
    }
    if evidence.reciprocal.site != evidence.site {
        return Decision::Refused(Refusal::EvidenceSiteMismatch);
    }
    let Ok(reciprocal) = evidence.reciprocal.result else {
        return Decision::Refused(Refusal::ReciprocalUnavailable);
    };
    let Ok(temporal_closure) = evidence.temporal_closure else {
        return Decision::Refused(Refusal::ForwardReverseClosureUnavailable);
    };
    if reciprocal.site != evidence.site || temporal_closure.site != evidence.site {
        return Decision::Refused(Refusal::EvidenceSiteMismatch);
    }
    Decision::HeldOutEligible(HeldOutCandidate {
        site: evidence.site,
    })
}

#[cfg(test)]
mod tests {
    use super::{Assignment, Decision, Evidence, Refusal, gate};
    use crate::{
        far_field::Classification,
        raw_register::{
            BidirectionalOutcome, BidirectionalReading, CameraCovariance, CameraDisplacement,
            Candidate, Node, StripSite, StripSiteReading, TrackClosure,
        },
    };

    fn site(index: f64) -> StripSite {
        StripSite {
            root: Candidate {
                node: Node {
                    centre: [index, 0.0, 1.0],
                    perp: [0.0, 1.0, 0.0],
                    epi: [1.0, 0.0, 0.0],
                    phi: index,
                },
                view_ray: [0.0, 0.0, 1.0],
                view_pixel: [0.0, 0.0],
            },
            offset_rad: [0.0, 0.0],
        }
    }

    fn reading(site: StripSite) -> StripSiteReading {
        StripSiteReading {
            site,
            epi_axis: [1.0, 0.0, 0.0],
            perp_axis: [0.0, 1.0, 0.0],
            displacement_rad: CameraDisplacement {
                epi: 0.0,
                perp: 0.0,
            },
            covariance_rad2: CameraCovariance {
                epi_epi: 0.0,
                epi_perp: 0.0,
                perp_perp: 0.0,
            },
            condition: 1.0,
            correlation: 1.0,
        }
    }

    fn evidence() -> Evidence {
        let declared = site(0.0);
        let reading = reading(declared);
        Evidence {
            site: declared,
            far_field: Classification::ProvenFar300 {
                distance_lower_bound_metres: 300.0,
                maximum_plausible_epi_rad: 0.0001,
            },
            reciprocal: BidirectionalOutcome {
                site: declared,
                result: Ok(BidirectionalReading {
                    site: declared,
                    forward: reading,
                    reverse: reading,
                    closure: CameraDisplacement {
                        epi: 99.0,
                        perp: -99.0,
                    },
                    closure_covariance_rad2: reading.covariance_rad2,
                }),
            },
            temporal_closure: Ok(TrackClosure {
                site: declared,
                closure_rad: CameraDisplacement {
                    epi: -88.0,
                    perp: 88.0,
                },
                covariance_rad2: reading.covariance_rad2,
            }),
            assignment: Assignment::HeldOut,
        }
    }

    #[test]
    fn admits_only_complete_held_out_proven_far_evidence() {
        let input = evidence();
        assert_eq!(
            gate(input),
            Decision::HeldOutEligible(super::HeldOutCandidate { site: input.site })
        );
    }

    #[test]
    fn does_not_hide_an_arbitrary_closure_magnitude_cutoff() {
        // Deliberately enormous finite closures above still pass.  Their
        // numerical interpretation belongs to a separately calibrated fit.
        assert!(matches!(gate(evidence()), Decision::HeldOutEligible(_)));
    }

    #[test]
    fn refuses_non_far_or_non_held_out_inputs_before_controls() {
        let mut input = evidence();
        input.far_field = Classification::UncertainOrUnplaceable;
        assert_eq!(gate(input), Decision::Refused(Refusal::NotProvenFar300));

        let mut input = evidence();
        input.assignment = Assignment::Training;
        assert_eq!(gate(input), Decision::Refused(Refusal::NotHeldOut));
    }

    #[test]
    fn refuses_missing_or_retargeted_controls() {
        let mut input = evidence();
        input.reciprocal.result = Err(crate::raw_register::BidirectionalRefused {
            forward: None,
            reverse: None,
        });
        assert_eq!(
            gate(input),
            Decision::Refused(Refusal::ReciprocalUnavailable)
        );

        let mut input = evidence();
        input.temporal_closure = Err(crate::raw_register::TrackClosureRefused::MismatchedSite);
        assert_eq!(
            gate(input),
            Decision::Refused(Refusal::ForwardReverseClosureUnavailable)
        );

        let mut input = evidence();
        input.reciprocal.site = site(1.0);
        assert_eq!(
            gate(input),
            Decision::Refused(Refusal::EvidenceSiteMismatch)
        );
    }
}
