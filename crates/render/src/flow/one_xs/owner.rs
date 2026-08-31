//! Pair-atomic numeric lineage for the selected ONE X2 estimator.
//!
//! This module owns numeric continuity only. A future decoded-source caller
//! must mint one [`Continuity`] per uninterrupted decode epoch, submit every
//! aligned source pair in order, and separately retain the delivery's
//! `FrameStamp` for type-2 map binding. This owner rejects a declared numeric
//! discontinuity but does not choose Studio's still-open seek/gap reset
//! semantics. The public tokens do not authenticate decoded input bytes or
//! prevent a caller from starting another lineage. Low-level oracle APIs also
//! remain public. This type makes one chosen owner path linear and pair-atomic;
//! source authority remains a separate production boundary.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use super::scalar::{ColdInputs, ColdNextCandidate, ColdPair, WorkRowCounts};
use super::warm::{KnownWarmNext, WarmPair};
use super::{Displacement, InvalidNodeCounts};

/// Caller-supplied identity for one numeric lineage.
///
/// Clones name the same sequence. [`Self::new`] always creates a distinct
/// identity, even if a previous sequence used the same frame indices. This
/// token carries no decoded-source authority by itself.
#[derive(Clone)]
pub struct Continuity(Arc<()>);

impl Continuity {
    pub fn new() -> Self {
        Self(Arc::new(()))
    }
}

impl Default for Continuity {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for Continuity {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Continuity {}

impl fmt::Debug for Continuity {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("Continuity").finish_non_exhaustive()
    }
}

/// One caller-declared paired position inside a numeric lineage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairPosition {
    continuity: Continuity,
    index: u64,
}

impl PairPosition {
    /// Record the index that a caller asserts belongs to this numeric lineage.
    ///
    /// This constructor does not authenticate a decoded frame or its input.
    pub fn new(continuity: &Continuity, index: u64) -> Self {
        Self {
            continuity: continuity.clone(),
            index,
        }
    }

    pub fn index(&self) -> u64 {
        self.index
    }
}

/// Which exact estimator transaction produced a paired result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Cold,
    Warm,
}

/// One computed pair output, independent of its retained next state.
#[derive(Clone, Debug, PartialEq)]
pub struct PairOutput {
    pub phase: Phase,
    pub displacement: Displacement,
    pub invalid_nodes: InvalidNodeCounts,
    pub weighted_rows: WorkRowCounts,
}

enum NextState {
    AfterCold(ColdNextCandidate),
    AfterWarm(KnownWarmNext),
}

/// The indivisible A-to-B and B-to-A state on one chosen numeric lineage.
///
/// `advance` consumes the owner. It cannot be reused for a second successor:
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::owner::{PairOwner, PairPosition};
/// use kjerag_render::flow::one_xs::scalar::ColdInputs;
///
/// fn reuse(
///     owner: PairOwner,
///     first: PairPosition,
///     first_input: ColdInputs,
///     second: PairPosition,
///     second_input: ColdInputs,
/// ) {
///     let _ = owner.advance(first, first_input);
///     let _ = owner.advance(second, second_input);
/// }
/// ```
#[must_use = "the ONE X2 numeric lineage state has not been consumed"]
pub struct PairOwner {
    at: PairPosition,
    next: NextState,
}

impl PairOwner {
    /// Compute the cold entry for this linear owner path.
    ///
    /// A future source authority remains responsible for binding `at` to
    /// `input` and selecting this cold entry only where separately recovered
    /// reset semantics require it.
    pub fn start(at: PairPosition, input: ColdInputs) -> PairStep {
        let transition = ColdPair::new().transition(&input);
        PairStep {
            output: PairOutput {
                phase: Phase::Cold,
                displacement: transition.estimate.displacement,
                invalid_nodes: transition.estimate.invalid_nodes,
                weighted_rows: transition.estimate.weighted_rows,
            },
            owner: Self {
                at,
                next: NextState::AfterCold(transition.candidate_next),
            },
        }
    }

    /// Consume one token-and-index-adjacent pair and run the warm transaction.
    ///
    /// A rejected position performs no estimator work and returns the owner
    /// and prepared input unchanged. There is deliberately no reset branch:
    /// the source authority must not silently continue warm across a
    /// discontinuity, and Studio's restart semantics remain a separate
    /// decision. At `u64::MAX`, `Exhausted` takes precedence because no
    /// representable successor can ever be accepted.
    pub fn advance(
        self,
        offered: PairPosition,
        input: ColdInputs,
    ) -> Result<PairStep, Box<RejectedAdvance>> {
        if let Err(reason) = self.require_adjacent(&offered) {
            return Err(Box::new(RejectedAdvance {
                owner: self,
                offered,
                input,
                reason,
            }));
        }

        let Self { next, .. } = self;
        let transition = match next {
            NextState::AfterCold(next) => WarmPair::new().transition(next.into_checkpoint(input)),
            NextState::AfterWarm(next) => WarmPair::new().transition(next.into_checkpoint(input)),
        };
        Ok(PairStep {
            output: PairOutput {
                phase: Phase::Warm,
                displacement: transition.estimate.displacement,
                invalid_nodes: transition.estimate.invalid_nodes,
                weighted_rows: transition.estimate.weighted_rows,
            },
            owner: Self {
                at: offered,
                next: NextState::AfterWarm(transition.known_next),
            },
        })
    }

    pub fn position(&self) -> &PairPosition {
        &self.at
    }

    fn require_adjacent(&self, offered: &PairPosition) -> Result<(), ContinuityError> {
        let Some(expected) = self.at.index.checked_add(1) else {
            return Err(ContinuityError::Exhausted {
                previous: self.at.index,
            });
        };
        if self.at.continuity != offered.continuity {
            return Err(ContinuityError::DifferentLineage);
        }
        if offered.index == expected {
            return Ok(());
        }
        if offered.index == self.at.index {
            return Err(ContinuityError::Duplicate {
                index: offered.index,
            });
        }
        if offered.index < expected {
            return Err(ContinuityError::Backward {
                previous: self.at.index,
                offered: offered.index,
            });
        }
        Err(ContinuityError::Gap {
            expected,
            offered: offered.index,
        })
    }
}

impl fmt::Debug for PairOwner {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.debug_struct("PairOwner")
            .field("at", &self.at)
            .field(
                "next",
                &match self.next {
                    NextState::AfterCold(_) => Phase::Cold,
                    NextState::AfterWarm(_) => Phase::Warm,
                },
            )
            .finish()
    }
}

/// One result and its linear owner for the successor on this chosen path.
#[must_use = "the ONE X2 pair result and retained owner have not been consumed"]
#[derive(Debug)]
pub struct PairStep {
    pub output: PairOutput,
    pub owner: PairOwner,
}

/// Why an offered pair cannot consume the retained warm state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContinuityError {
    DifferentLineage,
    Duplicate { index: u64 },
    Backward { previous: u64, offered: u64 },
    Gap { expected: u64, offered: u64 },
    Exhausted { previous: u64 },
}

impl fmt::Display for ContinuityError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DifferentLineage => {
                out.write_str("ONE X2 flow position uses another numeric lineage")
            }
            Self::Duplicate { index } => {
                write!(out, "ONE X2 flow frame repeats frame {index}")
            }
            Self::Backward { previous, offered } => write!(
                out,
                "ONE X2 flow moved backward from frame {previous} to frame {offered}"
            ),
            Self::Gap { expected, offered } => write!(
                out,
                "ONE X2 flow skipped frame {expected}; next offered frame is {offered}"
            ),
            Self::Exhausted { previous } => {
                write!(out, "ONE X2 flow cannot advance past frame {previous}")
            }
        }
    }
}

impl Error for ContinuityError {}

/// A refused advance with every consumed argument returned intact.
#[derive(Debug)]
pub struct RejectedAdvance {
    pub owner: PairOwner,
    pub offered: PairPosition,
    pub input: ColdInputs,
    pub reason: ContinuityError,
}

impl fmt::Display for RejectedAdvance {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reason.fmt(out)
    }
}

impl Error for RejectedAdvance {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::one_xs::{COLS, Lens, LensPair, ROWS};

    fn input(a: u8, b: u8) -> ColdInputs {
        let pixels = ROWS * COLS;
        ColdInputs::from_prepared(
            LensPair {
                a: vec![a; pixels],
                b: vec![b; pixels],
            },
            LensPair {
                a: vec![255; pixels],
                b: vec![255; pixels],
            },
        )
        .unwrap()
    }

    fn assert_output_matches_cold(
        actual: &PairOutput,
        expected: &super::super::scalar::ColdEstimate,
    ) {
        assert_eq!(actual.phase, Phase::Cold);
        assert_eq!(actual.displacement, expected.displacement);
        assert_eq!(actual.invalid_nodes, expected.invalid_nodes);
        assert_eq!(actual.weighted_rows, expected.weighted_rows);
    }

    fn assert_output_matches_warm(
        actual: &PairOutput,
        expected: &super::super::warm::WarmEstimate,
    ) {
        assert_eq!(actual.phase, Phase::Warm);
        assert_eq!(actual.displacement, expected.displacement);
        assert_eq!(actual.invalid_nodes, expected.invalid_nodes);
        assert_eq!(actual.weighted_rows, expected.weighted_rows);
    }

    fn assert_input(actual: &ColdInputs, a: u8, b: u8) {
        assert!(actual.image(Lens::A).iter().all(|value| *value == a));
        assert!(actual.image(Lens::B).iter().all(|value| *value == b));
        assert!(actual.mask(Lens::A).iter().all(|value| *value == 255));
        assert!(actual.mask(Lens::B).iter().all(|value| *value == 255));
    }

    #[test]
    fn first_pair_is_the_exact_cold_transition() {
        let current = input(64, 96);
        let expected = ColdPair::new().transition(&current);
        let continuity = Continuity::new();
        let step = PairOwner::start(PairPosition::new(&continuity, 20), current);

        assert_output_matches_cold(&step.output, &expected.estimate);
        assert_eq!(step.owner.position().index(), 20);
        let NextState::AfterCold(next) = &step.owner.next else {
            panic!("a first pair must retain the cold destination");
        };
        assert_eq!(next.a_to_b_cadence.calc_count(), 3);
        assert_eq!(next.b_to_a_cadence.calc_count(), 3);
        assert!(
            next.references
                .lens(Lens::A)
                .iter()
                .all(|value| *value == 64)
        );
        assert!(
            next.references
                .lens(Lens::B)
                .iter()
                .all(|value| *value == 96)
        );
    }

    #[test]
    fn adjacent_second_pair_is_the_exact_warm_transition_from_cold() {
        let first = input(64, 96);
        let second = input(72, 104);
        let direct_cold = ColdPair::new().transition(&first);
        let expected =
            WarmPair::new().transition(direct_cold.candidate_next.into_checkpoint(second.clone()));
        let continuity = Continuity::new();
        let first = PairOwner::start(PairPosition::new(&continuity, 20), first);
        let actual = first
            .owner
            .advance(PairPosition::new(&continuity, 21), second)
            .unwrap();

        assert_output_matches_warm(&actual.output, &expected.estimate);
        let NextState::AfterWarm(next) = &actual.owner.next else {
            panic!("an adjacent second pair must retain a warm destination");
        };
        assert_eq!(next.a_to_b_cadence.calc_count(), 4);
        assert_eq!(next.b_to_a_cadence.calc_count(), 4);
    }

    #[test]
    fn adjacent_third_pair_uses_the_prior_warm_state() {
        let first = input(64, 96);
        let second = input(72, 104);
        let third = input(80, 112);
        let direct_cold = ColdPair::new().transition(&first);
        let direct_second =
            WarmPair::new().transition(direct_cold.candidate_next.into_checkpoint(second.clone()));
        let expected =
            WarmPair::new().transition(direct_second.known_next.into_checkpoint(third.clone()));

        let continuity = Continuity::new();
        let first = PairOwner::start(PairPosition::new(&continuity, 20), first);
        let second = first
            .owner
            .advance(PairPosition::new(&continuity, 21), second)
            .unwrap();
        let actual = second
            .owner
            .advance(PairPosition::new(&continuity, 22), third)
            .unwrap();

        assert_output_matches_warm(&actual.output, &expected.estimate);
        let NextState::AfterWarm(next) = &actual.owner.next else {
            panic!("an adjacent third pair must retain a warm destination");
        };
        assert_eq!(next.a_to_b_cadence.calc_count(), 5);
        assert_eq!(next.b_to_a_cadence.calc_count(), 5);
    }

    #[test]
    fn duplicate_redraw_is_rejected_without_consuming_state() {
        let continuity = Continuity::new();
        let control = PairOwner::start(PairPosition::new(&continuity, 20), input(64, 96));
        let trial = PairOwner::start(PairPosition::new(&continuity, 20), input(64, 96));
        let rejected = trial
            .owner
            .advance(PairPosition::new(&continuity, 20), input(72, 104))
            .unwrap_err();
        assert_eq!(rejected.reason, ContinuityError::Duplicate { index: 20 });
        assert_eq!(rejected.offered, PairPosition::new(&continuity, 20));
        assert_input(&rejected.input, 72, 104);

        let expected = control
            .owner
            .advance(PairPosition::new(&continuity, 21), input(72, 104))
            .unwrap();
        let recovered = rejected
            .owner
            .advance(PairPosition::new(&continuity, 21), rejected.input)
            .unwrap();
        assert_eq!(recovered.output, expected.output);
    }

    #[test]
    fn rejected_warm_advance_preserves_the_complete_warm_state() {
        let continuity = Continuity::new();
        let control = PairOwner::start(PairPosition::new(&continuity, 20), input(64, 96))
            .owner
            .advance(PairPosition::new(&continuity, 21), input(72, 104))
            .unwrap();
        let trial = PairOwner::start(PairPosition::new(&continuity, 20), input(64, 96))
            .owner
            .advance(PairPosition::new(&continuity, 21), input(72, 104))
            .unwrap();

        let rejected = trial
            .owner
            .advance(PairPosition::new(&continuity, 23), input(80, 112))
            .unwrap_err();
        assert_eq!(
            rejected.reason,
            ContinuityError::Gap {
                expected: 22,
                offered: 23,
            }
        );
        assert_eq!(rejected.offered, PairPosition::new(&continuity, 23));
        assert_input(&rejected.input, 80, 112);

        let expected = control
            .owner
            .advance(PairPosition::new(&continuity, 22), input(80, 112))
            .unwrap();
        let recovered = rejected
            .owner
            .advance(PairPosition::new(&continuity, 22), rejected.input)
            .unwrap();
        assert_eq!(recovered.output, expected.output);
        let NextState::AfterWarm(recovered_next) = &recovered.owner.next else {
            panic!("a recovered warm advance must retain a warm destination");
        };
        let NextState::AfterWarm(expected_next) = &expected.owner.next else {
            panic!("the untouched control must retain a warm destination");
        };
        assert_eq!(recovered_next.a_to_b_cadence.calc_count(), 5);
        assert_eq!(recovered_next.b_to_a_cadence.calc_count(), 5);
        assert_eq!(recovered_next.a_to_b_cadence, expected_next.a_to_b_cadence);
        assert_eq!(recovered_next.b_to_a_cadence, expected_next.b_to_a_cadence);
        assert_eq!(recovered_next.references, expected_next.references);
    }

    #[test]
    fn foreign_backward_and_gap_positions_return_the_owner_and_input() {
        let continuity = Continuity::new();
        let foreign = Continuity::new();
        for (offered, reason) in [
            (
                PairPosition::new(&foreign, 11),
                ContinuityError::DifferentLineage,
            ),
            (
                PairPosition::new(&continuity, 9),
                ContinuityError::Backward {
                    previous: 10,
                    offered: 9,
                },
            ),
            (
                PairPosition::new(&continuity, 12),
                ContinuityError::Gap {
                    expected: 11,
                    offered: 12,
                },
            ),
        ] {
            let step = PairOwner::start(PairPosition::new(&continuity, 10), input(64, 96));
            let expected_offered = offered.clone();
            let rejected = step.owner.advance(offered, input(72, 104)).unwrap_err();
            assert_eq!(rejected.reason, reason);
            assert_eq!(rejected.offered, expected_offered);
            assert_eq!(
                rejected.owner.position(),
                &PairPosition::new(&continuity, 10)
            );
            assert_input(&rejected.input, 72, 104);
            assert_eq!(
                rejected
                    .owner
                    .advance(PairPosition::new(&continuity, 11), rejected.input)
                    .unwrap()
                    .output
                    .phase,
                Phase::Warm
            );
        }
    }

    #[test]
    fn maximum_index_refuses_overflow_without_consuming_state() {
        let continuity = Continuity::new();
        let foreign = Continuity::new();
        let maximum = PairOwner::start(PairPosition::new(&continuity, u64::MAX - 1), input(64, 96))
            .owner
            .advance(PairPosition::new(&continuity, u64::MAX), input(72, 104))
            .unwrap();
        assert_eq!(maximum.output.phase, Phase::Warm);
        let rejected_foreign = maximum
            .owner
            .advance(PairPosition::new(&foreign, 0), input(80, 112))
            .unwrap_err();
        assert_eq!(
            rejected_foreign.reason,
            ContinuityError::Exhausted { previous: u64::MAX }
        );
        assert_eq!(rejected_foreign.offered, PairPosition::new(&foreign, 0));
        assert_eq!(rejected_foreign.owner.position().index(), u64::MAX);
        assert_input(&rejected_foreign.input, 80, 112);

        let rejected_same = rejected_foreign
            .owner
            .advance(PairPosition::new(&continuity, u64::MAX), input(80, 112))
            .unwrap_err();
        assert_eq!(
            rejected_same.reason,
            ContinuityError::Exhausted { previous: u64::MAX }
        );
        assert_eq!(
            rejected_same.offered,
            PairPosition::new(&continuity, u64::MAX)
        );
        assert_eq!(rejected_same.owner.position().index(), u64::MAX);
        assert_input(&rejected_same.input, 80, 112);
    }

    #[test]
    fn fresh_continuity_starts_cold_at_any_index() {
        let old = Continuity::new();
        let _old = PairOwner::start(PairPosition::new(&old, 20), input(64, 96));
        let fresh = Continuity::new();
        let reset = PairOwner::start(PairPosition::new(&fresh, 6_369), input(72, 104));
        assert_eq!(reset.output.phase, Phase::Cold);
        assert_eq!(reset.owner.position().index(), 6_369);
        assert_ne!(old, fresh);
    }

    #[test]
    fn continuity_identity_stays_unique_while_an_owner_holds_it() {
        let continuity = Continuity::new();
        let owner = PairOwner::start(PairPosition::new(&continuity, 20), input(64, 96)).owner;
        let same = continuity.clone();
        assert_eq!(continuity, same);
        for _ in 0..1_000 {
            assert_ne!(continuity, Continuity::new());
        }
        assert_eq!(owner.position().continuity, continuity);
    }
}
