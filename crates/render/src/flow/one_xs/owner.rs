//! Pair-atomic numeric lineage for the selected ONE X2 estimator.
//!
//! This module owns numeric continuity only. The production [`super::player::FrameOwner`]
//! mints one [`Continuity`] per uninterrupted decode epoch, submits every
//! aligned source pair in order, and separately retains the delivery's
//! `FrameStamp` for type-2 map binding. This owner rejects a declared numeric
//! discontinuity but does not choose Studio's still-open seek/gap reset
//! semantics. Its public tokens alone do not authenticate decoded input bytes
//! or prevent another caller from starting a lineage. Low-level oracle APIs
//! also remain public. This type makes one chosen owner path linear and
//! pair-atomic; source authority remains a separate production boundary.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use super::scalar::{
    ColdInputs, ColdNextCandidate, ColdPair, CpuPairedPisSolver, PairSolveError, PairedPisSolver,
    WorkRowCounts,
};
use super::warm::{KnownWarmNext, WarmPair, WarmTransitionError};
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
    /// The production frame owner binds `at` to `input` and admits this cold
    /// entry only for frame zero. Lower-level callers retain responsibility for
    /// providing an equivalent source-authority boundary.
    pub fn start(at: PairPosition, input: ColdInputs) -> PairStep {
        match Self::try_start_with_solver(at, input, &mut CpuPairedPisSolver) {
            Ok(step) => step,
            Err(error) => match *error {
                FailedStart {
                    source: PairSolveError::Solver { source, .. },
                    ..
                } => match source {},
                FailedStart {
                    source: PairSolveError::Stamp { source, .. },
                    ..
                } => panic!("CPU paired solver returned its own invalid stamp: {source}"),
            },
        }
    }

    pub(crate) fn try_start_with_solver<S: PairedPisSolver>(
        at: PairPosition,
        input: ColdInputs,
        solver: &mut S,
    ) -> Result<PairStep, Box<FailedStart<S::Error>>> {
        let transition = match ColdPair::new().try_transition_with_solver(&input, solver) {
            Ok(transition) => transition,
            Err(source) => return Err(Box::new(FailedStart { at, input, source })),
        };
        Ok(PairStep {
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
        })
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
        match self.try_advance_with_solver(offered, input, &mut CpuPairedPisSolver) {
            Ok(step) => Ok(step),
            Err(error) => match *error {
                FailedAdvance {
                    owner,
                    offered,
                    input,
                    reason: AdvanceFailure::Continuity(reason),
                } => Err(Box::new(RejectedAdvance {
                    owner,
                    offered,
                    input,
                    reason,
                })),
                FailedAdvance {
                    reason: AdvanceFailure::Solver(PairSolveError::Solver { source, .. }),
                    ..
                } => match source {},
                FailedAdvance {
                    reason: AdvanceFailure::Solver(PairSolveError::Stamp { source, .. }),
                    ..
                } => panic!("CPU paired solver returned its own invalid stamp: {source}"),
            },
        }
    }

    pub(crate) fn try_advance_with_solver<S: PairedPisSolver>(
        self,
        offered: PairPosition,
        input: ColdInputs,
        solver: &mut S,
    ) -> Result<PairStep, Box<FailedAdvance<S::Error>>> {
        if let Err(reason) = self.require_adjacent(&offered) {
            return Err(Box::new(FailedAdvance {
                owner: self,
                offered,
                input,
                reason: AdvanceFailure::Continuity(reason),
            }));
        }

        let checkpoint = match &self.next {
            NextState::AfterCold(next) => next.checkpoint_from_borrowed(input.clone()),
            NextState::AfterWarm(next) => next.checkpoint_from_borrowed(input.clone()),
        };
        let transition = match WarmPair::new().try_transition_with_solver(checkpoint, solver) {
            Ok(transition) => transition,
            Err(error) => {
                let WarmTransitionError {
                    inputs: failed_checkpoint,
                    source,
                } = *error;
                drop(failed_checkpoint);
                return Err(Box::new(FailedAdvance {
                    owner: self,
                    offered,
                    input,
                    reason: AdvanceFailure::Solver(source),
                }));
            }
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

/// A failed cold start with both consumed arguments returned intact.
#[allow(dead_code)]
pub(crate) struct FailedStart<E> {
    pub(crate) at: PairPosition,
    pub(crate) input: ColdInputs,
    pub(crate) source: PairSolveError<E>,
}

pub(crate) enum AdvanceFailure<E> {
    Continuity(ContinuityError),
    Solver(PairSolveError<E>),
}

/// A refused or failed warm advance with the exact prior owner returned.
pub(crate) struct FailedAdvance<E> {
    pub(crate) owner: PairOwner,
    pub(crate) offered: PairPosition,
    pub(crate) input: ColdInputs,
    pub(crate) reason: AdvanceFailure<E>,
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
    use crate::flow::one_xs::dense::PublicDenseField;
    use crate::flow::one_xs::pis::{AtoB, BtoA, DescentAdmission, Level, PisDirection};
    use crate::flow::one_xs::scalar::{
        PairSolveStage, PairedPatchGrids, PairedSolveRequest, SolveStampError,
    };
    use crate::flow::one_xs::temporal::BlurredBelts;
    use crate::flow::one_xs::temporal_median::MedianState;
    use crate::flow::one_xs::warm::{EmptyOverrideCadence, HintPyramid, RetainedWorkRows};
    use crate::flow::one_xs::{COLS, Direction, Lens, LensPair, ROWS};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct InjectedFailure;

    impl fmt::Display for InjectedFailure {
        fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
            out.write_str("injected paired PIS failure")
        }
    }

    impl Error for InjectedFailure {}

    struct FailingSolver {
        fail_at: PairSolveStage,
        cpu: CpuPairedPisSolver,
    }

    impl PairedPisSolver for FailingSolver {
        type Error = InjectedFailure;

        fn solve(&mut self, request: PairedSolveRequest) -> Result<PairedPatchGrids, Self::Error> {
            if request.stage == self.fail_at {
                return Err(InjectedFailure);
            }
            Ok(self.cpu.solve(request).unwrap())
        }
    }

    #[derive(Clone, Copy)]
    enum Corruption {
        Direction,
        Level,
        Stage,
    }

    struct CorruptingSolver {
        corrupt_at: PairSolveStage,
        corruption: Corruption,
        cpu: CpuPairedPisSolver,
    }

    struct RecordingSolver {
        calls: Vec<(PairSolveStage, DescentAdmission, DescentAdmission)>,
        cpu: CpuPairedPisSolver,
    }

    impl PairedPisSolver for RecordingSolver {
        type Error = InjectedFailure;

        fn solve(&mut self, request: PairedSolveRequest) -> Result<PairedPatchGrids, Self::Error> {
            self.calls.push((
                request.stage,
                request.a_to_b.admission,
                request.b_to_a.admission,
            ));
            Ok(self.cpu.solve(request).unwrap())
        }
    }

    impl PairedPisSolver for CorruptingSolver {
        type Error = InjectedFailure;

        fn solve(&mut self, request: PairedSolveRequest) -> Result<PairedPatchGrids, Self::Error> {
            let stage = request.stage;
            let mut solved = self.cpu.solve(request).unwrap();
            if stage == self.corrupt_at {
                match self.corruption {
                    Corruption::Direction => {
                        solved.a_to_b.stamp.direction = Direction::BtoA;
                    }
                    Corruption::Level => {
                        solved.a_to_b.stamp.stage = match stage {
                            PairSolveStage::Cold { calculation, level } => PairSolveStage::Cold {
                                calculation,
                                level: other_level(level),
                            },
                            PairSolveStage::Warm { level } => PairSolveStage::Warm {
                                level: other_level(level),
                            },
                        };
                    }
                    Corruption::Stage => {
                        let PairSolveStage::Cold { calculation, level } = stage else {
                            panic!("stage corruption is a cold-only fixture");
                        };
                        solved.a_to_b.stamp.stage = PairSolveStage::Cold {
                            calculation: (calculation + 1) % 3,
                            level,
                        };
                    }
                }
            }
            Ok(solved)
        }
    }

    fn other_level(level: Level) -> Level {
        match level {
            Level::One => Level::Two,
            Level::Two => Level::One,
        }
    }

    fn append_u32(bytes: &mut Vec<u8>, values: impl IntoIterator<Item = u32>) {
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }

    fn append_direction<D: PisDirection>(
        bytes: &mut Vec<u8>,
        public: &PublicDenseField<D>,
        median: &MedianState<D>,
        rows: &RetainedWorkRows<D>,
        hints: &HintPyramid<D>,
        cadence: EmptyOverrideCadence,
    ) {
        append_u32(bytes, public.dcol().iter().map(|value| value.to_bits()));
        append_u32(bytes, public.drow().iter().map(|value| value.to_bits()));
        bytes.extend_from_slice(median.histogram());
        append_u32(bytes, median.offsets().iter().copied());
        append_u32(bytes, median.values().iter().map(|value| value.to_bits()));
        match rows.small_disparity_rows() {
            Some(values) => {
                bytes.push(1);
                bytes.extend(values.iter().map(|value| u8::from(*value)));
            }
            None => bytes.push(0),
        }
        bytes.extend(
            rows.lack_of_texture_rows()
                .iter()
                .map(|value| u8::from(*value)),
        );
        for level in [Level::Two, Level::One] {
            append_u32(
                bytes,
                hints
                    .level(level)
                    .dcol()
                    .iter()
                    .map(|value| value.to_bits()),
            );
            append_u32(
                bytes,
                hints
                    .level(level)
                    .drow()
                    .iter()
                    .map(|value| value.to_bits()),
            );
        }
        bytes.extend_from_slice(&cadence.calc_count().to_le_bytes());
        bytes.extend_from_slice(&cadence.cadence().to_le_bytes());
    }

    #[allow(clippy::too_many_arguments)]
    fn append_next(
        bytes: &mut Vec<u8>,
        references: &BlurredBelts,
        a_public: &PublicDenseField<AtoB>,
        b_public: &PublicDenseField<BtoA>,
        a_median: &MedianState<AtoB>,
        b_median: &MedianState<BtoA>,
        a_rows: &RetainedWorkRows<AtoB>,
        b_rows: &RetainedWorkRows<BtoA>,
        a_hints: &HintPyramid<AtoB>,
        b_hints: &HintPyramid<BtoA>,
        a_cadence: EmptyOverrideCadence,
        b_cadence: EmptyOverrideCadence,
    ) {
        bytes.extend_from_slice(references.lens(Lens::A));
        bytes.extend_from_slice(references.lens(Lens::B));
        append_direction(bytes, a_public, a_median, a_rows, a_hints, a_cadence);
        append_direction(bytes, b_public, b_median, b_rows, b_hints, b_cadence);
    }

    fn owner_bits(owner: &PairOwner) -> Vec<u8> {
        let mut bytes = owner.position().index().to_le_bytes().to_vec();
        match &owner.next {
            NextState::AfterCold(next) => {
                bytes.push(0);
                append_next(
                    &mut bytes,
                    &next.references,
                    &next.a_to_b_public,
                    &next.b_to_a_public,
                    &next.a_to_b_median,
                    &next.b_to_a_median,
                    &next.a_to_b_work_rows,
                    &next.b_to_a_work_rows,
                    &next.a_to_b_hints,
                    &next.b_to_a_hints,
                    next.a_to_b_cadence,
                    next.b_to_a_cadence,
                );
            }
            NextState::AfterWarm(next) => {
                bytes.push(1);
                append_next(
                    &mut bytes,
                    &next.references,
                    &next.a_to_b_public,
                    &next.b_to_a_public,
                    &next.a_to_b_median,
                    &next.b_to_a_median,
                    &next.a_to_b_work_rows,
                    &next.b_to_a_work_rows,
                    &next.a_to_b_hints,
                    &next.b_to_a_hints,
                    next.a_to_b_cadence,
                    next.b_to_a_cadence,
                );
            }
        }
        bytes
    }

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

    #[test]
    fn every_cold_stage_failure_returns_exact_state_for_retry_and_successor() {
        let stages = [
            PairSolveStage::Cold {
                calculation: 0,
                level: Level::Two,
            },
            PairSolveStage::Cold {
                calculation: 0,
                level: Level::One,
            },
            PairSolveStage::Cold {
                calculation: 1,
                level: Level::Two,
            },
            PairSolveStage::Cold {
                calculation: 1,
                level: Level::One,
            },
            PairSolveStage::Cold {
                calculation: 2,
                level: Level::Two,
            },
            PairSolveStage::Cold {
                calculation: 2,
                level: Level::One,
            },
        ];
        for fail_at in stages {
            let continuity = Continuity::new();
            let at = PairPosition::new(&continuity, 20);
            let control = PairOwner::start(at.clone(), input(64, 96));
            let mut failing = FailingSolver {
                fail_at,
                cpu: CpuPairedPisSolver,
            };
            let failed =
                match PairOwner::try_start_with_solver(at.clone(), input(64, 96), &mut failing) {
                    Err(failed) => failed,
                    Ok(_) => panic!("injected {fail_at} unexpectedly succeeded"),
                };
            assert_eq!(failed.at, at);
            assert_input(&failed.input, 64, 96);
            assert!(matches!(
                failed.source,
                PairSolveError::Solver {
                    stage,
                    source: InjectedFailure,
                } if stage == fail_at
            ));

            let retried =
                PairOwner::try_start_with_solver(failed.at, failed.input, &mut CpuPairedPisSolver)
                    .unwrap_or_else(|_| panic!("CPU retry after {fail_at} failed"));
            assert_eq!(retried.output, control.output);
            assert_eq!(owner_bits(&retried.owner), owner_bits(&control.owner));

            let expected = control
                .owner
                .advance(PairPosition::new(&continuity, 21), input(72, 104))
                .unwrap();
            let actual = retried
                .owner
                .advance(PairPosition::new(&continuity, 21), input(72, 104))
                .unwrap();
            assert_eq!(actual.output, expected.output);
            assert_eq!(owner_bits(&actual.owner), owner_bits(&expected.owner));
        }
    }

    #[test]
    fn every_warm_stage_failure_returns_exact_owner_for_retry_and_successor() {
        for fail_at in [
            PairSolveStage::Warm { level: Level::Two },
            PairSolveStage::Warm { level: Level::One },
        ] {
            let continuity = Continuity::new();
            let control = PairOwner::start(PairPosition::new(&continuity, 20), input(64, 96));
            let trial = PairOwner::start(PairPosition::new(&continuity, 20), input(64, 96));
            let before = owner_bits(&trial.owner);
            let mut failing = FailingSolver {
                fail_at,
                cpu: CpuPairedPisSolver,
            };
            let failed = match trial.owner.try_advance_with_solver(
                PairPosition::new(&continuity, 21),
                input(72, 104),
                &mut failing,
            ) {
                Err(failed) => failed,
                Ok(_) => panic!("injected {fail_at} unexpectedly succeeded"),
            };
            assert_eq!(owner_bits(&failed.owner), before);
            assert_input(&failed.input, 72, 104);
            assert!(matches!(
                failed.reason,
                AdvanceFailure::Solver(PairSolveError::Solver {
                    stage,
                    source: InjectedFailure,
                }) if stage == fail_at
            ));

            let expected = control
                .owner
                .advance(PairPosition::new(&continuity, 21), input(72, 104))
                .unwrap();
            let actual = failed
                .owner
                .advance(PairPosition::new(&continuity, 21), failed.input)
                .unwrap();
            assert_eq!(actual.output, expected.output);
            assert_eq!(owner_bits(&actual.owner), owner_bits(&expected.owner));

            let expected = expected
                .owner
                .advance(PairPosition::new(&continuity, 22), input(80, 112))
                .unwrap();
            let actual = actual
                .owner
                .advance(PairPosition::new(&continuity, 22), input(80, 112))
                .unwrap();
            assert_eq!(actual.output, expected.output);
            assert_eq!(owner_bits(&actual.owner), owner_bits(&expected.owner));
        }
    }

    #[test]
    fn paired_receipt_rejects_direction_level_and_stale_calculation_stamps() {
        let expected = PairSolveStage::Cold {
            calculation: 0,
            level: Level::Two,
        };
        for (corruption, expected_source) in [
            (
                Corruption::Direction,
                SolveStampError::Direction {
                    expected: Direction::AtoB,
                    actual: Direction::BtoA,
                },
            ),
            (
                Corruption::Level,
                SolveStampError::Level {
                    expected: Level::Two,
                    actual: Level::One,
                },
            ),
            (
                Corruption::Stage,
                SolveStampError::Stage {
                    expected,
                    actual: PairSolveStage::Cold {
                        calculation: 1,
                        level: Level::Two,
                    },
                },
            ),
        ] {
            let continuity = Continuity::new();
            let at = PairPosition::new(&continuity, 20);
            let mut solver = CorruptingSolver {
                corrupt_at: expected,
                corruption,
                cpu: CpuPairedPisSolver,
            };
            let failed =
                match PairOwner::try_start_with_solver(at.clone(), input(64, 96), &mut solver) {
                    Err(failed) => failed,
                    Ok(_) => panic!("corrupt receipt unexpectedly succeeded"),
                };
            assert!(matches!(
                failed.source,
                PairSolveError::Stamp { stage, source }
                    if stage == expected && source == expected_source
            ));
            let recovered = PairOwner::start(failed.at, failed.input);
            assert_eq!(recovered.owner.position(), &at);
        }
    }

    #[test]
    fn paired_stages_share_one_pre_increment_admission_and_advance_cadence_once() {
        let continuity = Continuity::new();
        let mut cold_solver = RecordingSolver {
            calls: Vec::new(),
            cpu: CpuPairedPisSolver,
        };
        let cold = PairOwner::try_start_with_solver(
            PairPosition::new(&continuity, 20),
            input(64, 96),
            &mut cold_solver,
        )
        .unwrap_or_else(|_| panic!("recorded cold transaction failed"));
        assert_eq!(
            cold_solver.calls,
            vec![
                (
                    PairSolveStage::Cold {
                        calculation: 0,
                        level: Level::Two,
                    },
                    DescentAdmission::EveryPatch,
                    DescentAdmission::EveryPatch,
                ),
                (
                    PairSolveStage::Cold {
                        calculation: 0,
                        level: Level::One,
                    },
                    DescentAdmission::EveryPatch,
                    DescentAdmission::EveryPatch,
                ),
                (
                    PairSolveStage::Cold {
                        calculation: 1,
                        level: Level::Two,
                    },
                    DescentAdmission::NoPatches,
                    DescentAdmission::NoPatches,
                ),
                (
                    PairSolveStage::Cold {
                        calculation: 1,
                        level: Level::One,
                    },
                    DescentAdmission::NoPatches,
                    DescentAdmission::NoPatches,
                ),
                (
                    PairSolveStage::Cold {
                        calculation: 2,
                        level: Level::Two,
                    },
                    DescentAdmission::NoPatches,
                    DescentAdmission::NoPatches,
                ),
                (
                    PairSolveStage::Cold {
                        calculation: 2,
                        level: Level::One,
                    },
                    DescentAdmission::NoPatches,
                    DescentAdmission::NoPatches,
                ),
            ]
        );

        let mut warm_solver = RecordingSolver {
            calls: Vec::new(),
            cpu: CpuPairedPisSolver,
        };
        let warm = cold
            .owner
            .try_advance_with_solver(
                PairPosition::new(&continuity, 21),
                input(72, 104),
                &mut warm_solver,
            )
            .unwrap_or_else(|_| panic!("recorded warm transaction failed"));
        assert_eq!(
            warm_solver.calls,
            vec![
                (
                    PairSolveStage::Warm { level: Level::Two },
                    DescentAdmission::NoPatches,
                    DescentAdmission::NoPatches,
                ),
                (
                    PairSolveStage::Warm { level: Level::One },
                    DescentAdmission::NoPatches,
                    DescentAdmission::NoPatches,
                ),
            ]
        );
        let NextState::AfterWarm(next) = &warm.owner.next else {
            panic!("recorded warm transaction did not publish warm state");
        };
        assert_eq!(next.a_to_b_cadence.calc_count(), 4);
        assert_eq!(next.b_to_a_cadence.calc_count(), 4);
    }
}
