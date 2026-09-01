//! Readable cold scalar producer for the selected ONE X2 flow pair.
//!
//! The selected playback owner uses this producer for its cold transaction,
//! and the detached corpus also exercises it as a readable correctness oracle.
//! It owns both directions and runs each recovered level-two then level-one
//! chain without arithmetic optimization. The independent directions run
//! concurrently; the selected configuration is explicit:
//! twelve descents split six per spatial pass, active levels two and one, and
//! zero variational iterations. Derivative preparation and variational
//! refinement therefore do not appear in this module.
//!
//! Cold means estimator state, not capture time. The V6 staging belts are from
//! a warm frame, while this owner begins without blurred-image history,
//! retained public fields, motion map, work rows, or temporal-median history.
//! Native `prepareBuffers` does materialize present zero hint planes before the
//! first of three inner calculations; each inner result then feeds those hints
//! and the temporal median into its successor. Its output can diagnose the
//! estimator through the captured-field render oracle; it cannot be called a
//! replay of Studio's warm result.

use std::convert::Infallible;
use std::error::Error;
use std::fmt;
#[cfg(test)]
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::thread;

use super::dense::{self, DirectedImages, PublicDenseField};
use super::l2_seed::into_l1_initial_grid;
use super::pis::{
    self, AtoB, BtoA, CostMode, DescentAdmission, InitialGrid, Input, Level, PatchGrid,
    PisDirection,
};
use super::post_update::preserve_without_variational_or_retained;
use super::public_blend::blend_periodic_boundary;
use super::temporal;
use super::temporal_median::{
    AtoBMedian, BtoAMedian, FilteredPatchGrid, MedianState, TemporalMedians,
};
#[cfg(test)]
use super::warm::EffectiveWorkRows;
use super::warm::{
    EmptyOverrideCadence, HintPyramid, RetainedWorkRows, WarmCheckpointInputs, WarmDirection,
};
use super::{
    COLS, DirectedFields, Displacement, InvalidNodeCounts, Lens, LensPair, PATCH_SIZE,
    PATCH_STRIDE, ROWS, selected_pis_interval,
};
use crate::flow::one_xs_belt::{
    CameraMaskError, CameraMaskReport, RetainedBaseMaps, SolverBelts, SourceBelts,
    base_support_masks, reconstructed_camera_masks,
};
use crate::projection::Reframe;

const COLD_INNER_CALCULATIONS: usize = 3;
const COLD_EMPTY_OVERRIDE_CADENCE: i32 = 10;

/// Prepared retained-resolution images and physical-mask slots.
///
/// Images and masks stay in physical A/B order. The B-to-A PIS constructor
/// swaps only the images, matching Studio's selected callbacks.
#[derive(Clone, Debug)]
pub struct ColdInputs {
    image: LensPair<Vec<u8>>,
    mask: LensPair<Vec<u8>>,
}

impl ColdInputs {
    /// Prepare captured staging belts using the base-map support mask.
    ///
    /// The image path is exact at the V6 boundary: 3-by-3 `INTER_AREA`, then
    /// the selected 5-by-5 sigma-0.8 U8 Gaussian. The mask path seeds
    /// `0 < u <= 1 && 0 < v <= 1`, applies the selected 9-by-9 rectangular
    /// erosion, then intersects the physical masks and copies the intersection
    /// to both slots. It deliberately omits the earlier camera clipping so it
    /// remains the control arm for the recovered static-mask experiment.
    pub fn from_staging_and_base_support(
        staging: &SourceBelts,
        base_maps: &RetainedBaseMaps,
    ) -> Self {
        Self::from_staging_and_masks(staging, base_support_masks(base_maps))
    }

    /// Prepare the same images with the §140-derived camera-mask reconstruction.
    ///
    /// This remains a diagnostic reconstruction. Its geometry is byte-exact
    /// against the authenticated matching V9 oracle, but the native V6 mask
    /// bytes were not captured and V6's base maps differ. The returned report
    /// makes the step's actual footprint observable at the call site.
    pub fn from_staging_and_reconstructed_camera_mask(
        staging: &SourceBelts,
        base_maps: &RetainedBaseMaps,
        reframe: &Reframe,
    ) -> Result<(Self, CameraMaskReport), CameraMaskError> {
        let (masks, report) = reconstructed_camera_masks(reframe, base_maps)?;
        Ok((Self::from_staging_and_masks(staging, masks), report))
    }

    fn from_staging_and_masks(staging: &SourceBelts, masks: LensPair<Vec<u8>>) -> Self {
        let retained = staging.reduce_area_3x3();
        let blurred = temporal::gaussian_blur(&retained);
        Self {
            image: LensPair {
                a: blurred.lens(Lens::A).to_vec(),
                b: blurred.lens(Lens::B).to_vec(),
            },
            mask: masks,
        }
    }

    /// Admit 1080-by-60 image planes after Studio's selected input blur.
    pub fn from_prepared(
        image: LensPair<Vec<u8>>,
        mask: LensPair<Vec<u8>>,
    ) -> Result<Self, InputShapeError> {
        Self::validate_shape(&image, &mask)?;
        Ok(Self { image, mask })
    }

    /// Admit retained images captured before Studio's selected input blur.
    ///
    /// `OpticalFlow::calc` applies the 5-by-5 sigma-0.8 U8 Gaussian in place
    /// before either directional FDS task sees the images. This constructor is
    /// for a boundary immediately before that call.
    pub fn from_before_input_blur(
        image: LensPair<Vec<u8>>,
        mask: LensPair<Vec<u8>>,
    ) -> Result<Self, InputShapeError> {
        Self::validate_shape(&image, &mask)?;
        let retained = SolverBelts::from_lenses(image)
            .expect("validated retained image pair has the solver-belt shape");
        Ok(Self::from_solver_belts_and_masks(retained, mask))
    }

    /// Apply the selected input blur to an already shape-typed solver pair.
    pub(crate) fn from_solver_belts_and_masks(
        retained: SolverBelts,
        mask: LensPair<Vec<u8>>,
    ) -> Self {
        debug_assert_eq!(mask.a.len(), ROWS * COLS);
        debug_assert_eq!(mask.b.len(), ROWS * COLS);
        Self::from_blurred_belts_and_masks(temporal::gaussian_blur(&retained), mask)
    }

    /// Admit an exact solver pair that has already crossed the selected input
    /// Gaussian.
    ///
    /// The [`temporal::BlurredBelts`] typestate has no conversion back to
    /// [`SolverBelts`], so this production handoff cannot apply the blur twice.
    pub(crate) fn from_blurred_belts_and_masks(
        blurred: temporal::BlurredBelts,
        mask: LensPair<Vec<u8>>,
    ) -> Self {
        debug_assert_eq!(mask.a.len(), ROWS * COLS);
        debug_assert_eq!(mask.b.len(), ROWS * COLS);
        Self {
            image: blurred.into_lenses(),
            mask,
        }
    }

    fn validate_shape(
        image: &LensPair<Vec<u8>>,
        mask: &LensPair<Vec<u8>>,
    ) -> Result<(), InputShapeError> {
        let expected = ROWS * COLS;
        for (part, lens, actual) in [
            ("image", Lens::A, image.a.len()),
            ("image", Lens::B, image.b.len()),
            ("mask", Lens::A, mask.a.len()),
            ("mask", Lens::B, mask.b.len()),
        ] {
            if actual != expected {
                return Err(InputShapeError {
                    part,
                    lens,
                    expected,
                    actual,
                });
            }
        }
        Ok(())
    }

    pub fn image(&self, lens: Lens) -> &[u8] {
        self.image.get(lens)
    }

    pub fn mask(&self, lens: Lens) -> &[u8] {
        self.mask.get(lens)
    }

    /// Rewrap these already-post-blur images for the temporal motion boundary.
    ///
    /// This deliberately does not call [`temporal::gaussian_blur`]. Warm
    /// checkpoint inputs have already crossed that native operation.
    pub(super) fn blurred_belts(&self) -> temporal::BlurredBelts {
        temporal::BlurredBelts::from_lenses(LensPair {
            a: self.image.a.clone(),
            b: self.image.b.clone(),
        })
        .expect("validated scalar images have the retained blurred-belt shape")
    }
}

/// A prepared retained plane had the wrong fixed-grid shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputShapeError {
    part: &'static str,
    lens: Lens,
    expected: usize,
    actual: usize,
}

impl fmt::Display for InputShapeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "ONE X2 scalar lens {} {} has {} samples, expected {}",
            self.lens, self.part, self.actual, self.expected,
        )
    }
}

impl Error for InputShapeError {}

/// Direction-named counts useful when comparing this cold reconstruction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkRowCounts {
    pub a_to_b_l2: usize,
    pub b_to_a_l2: usize,
    pub a_to_b_l1: usize,
    pub b_to_a_l1: usize,
}

/// One complete cold pair result, ready for the typed renderer oracle.
#[derive(Clone, Debug, PartialEq)]
pub struct ColdEstimate {
    pub displacement: Displacement,
    pub invalid_nodes: InvalidNodeCounts,
    pub weighted_rows: WorkRowCounts,
}

/// Candidate next-frame state composed by the cold transaction.
///
/// V3 authenticates the complete native cold transaction's three calls and
/// terminal direction-owned state, and the detached corpus oracle compares
/// this complete Rust output at that boundary. [`super::player::FrameOwner`]
/// supplies decoded-source authority around the numeric owner. Keeping this
/// type distinct from [`super::warm::KnownWarmNext`] records the cold-to-warm
/// ownership boundary explicitly.
#[must_use = "the candidate ONE X2 cold next state has not been retained or checked"]
#[derive(Debug)]
pub struct ColdNextCandidate {
    pub references: temporal::BlurredBelts,
    pub a_to_b_public: PublicDenseField<AtoB>,
    pub b_to_a_public: PublicDenseField<BtoA>,
    pub a_to_b_median: MedianState<AtoB>,
    pub b_to_a_median: MedianState<BtoA>,
    pub a_to_b_work_rows: RetainedWorkRows<AtoB>,
    pub b_to_a_work_rows: RetainedWorkRows<BtoA>,
    pub a_to_b_hints: HintPyramid<AtoB>,
    pub b_to_a_hints: HintPyramid<BtoA>,
    pub a_to_b_cadence: EmptyOverrideCadence,
    pub b_to_a_cadence: EmptyOverrideCadence,
}

impl ColdNextCandidate {
    pub(super) fn retained_checkpoint_from_borrowed(&self) -> super::warm::WarmRetainedInputs {
        super::warm::WarmRetainedInputs::from_borrowed_state(
            &self.references,
            &self.a_to_b_public,
            &self.b_to_a_public,
            &self.a_to_b_work_rows,
            &self.b_to_a_work_rows,
            &self.a_to_b_hints,
            &self.b_to_a_hints,
            self.a_to_b_cadence,
            self.b_to_a_cadence,
            &self.a_to_b_median,
            &self.b_to_a_median,
        )
    }

    /// Consume this candidate as the pre-state of the first warm calculation.
    ///
    /// The pair owner validates caller-declared numeric adjacency before
    /// calling this sibling-only bridge. A production source authority must
    /// separately prove decoded-frame adjacency.
    #[allow(dead_code)]
    pub(super) fn into_checkpoint(self, current_post_blur: ColdInputs) -> WarmCheckpointInputs {
        let Self {
            references,
            a_to_b_public,
            b_to_a_public,
            a_to_b_median,
            b_to_a_median,
            a_to_b_work_rows,
            b_to_a_work_rows,
            a_to_b_hints,
            b_to_a_hints,
            a_to_b_cadence,
            b_to_a_cadence,
        } = self;
        let temporal_medians = TemporalMedians::from_states(a_to_b_median, b_to_a_median)
            .expect("a produced ONE X2 cold median candidate must restore");
        WarmCheckpointInputs::new(
            current_post_blur,
            references,
            WarmDirection::new(
                a_to_b_public,
                a_to_b_work_rows,
                a_to_b_hints,
                a_to_b_cadence,
            ),
            WarmDirection::new(
                b_to_a_public,
                b_to_a_work_rows,
                b_to_a_hints,
                b_to_a_cadence,
            ),
            temporal_medians,
        )
    }
}

/// One cold estimate paired with every READ next-state component it computes.
///
/// The native terminal state is authenticated and the detached corpus oracle
/// compares this composition exactly. The pair owner consumes it only after
/// the production frame owner has validated the decoded delivery.
#[must_use = "the ONE X2 cold transition has not been consumed"]
#[derive(Debug)]
pub struct ColdTransition {
    pub estimate: ColdEstimate,
    pub candidate_next: ColdNextCandidate,
}

/// One paired sparse-solver position inside a cold or warm transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PairSolveStage {
    Cold { calculation: usize, level: Level },
    Warm { level: Level },
}

impl PairSolveStage {
    pub(crate) const fn level(self) -> Level {
        match self {
            Self::Cold { level, .. } | Self::Warm { level } => level,
        }
    }
}

impl fmt::Display for PairSolveStage {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cold { calculation, level } => {
                write!(out, "cold calculation {} {level}", calculation + 1)
            }
            Self::Warm { level } => write!(out, "warm {level}"),
        }
    }
}

/// One direction's complete scalar PIS call, owned by a paired request.
pub(crate) struct DirectionSolveRequest<D: PisDirection> {
    pub(super) cost_modes: Vec<CostMode>,
    pub(super) initial: InitialGrid<D>,
    pub(super) hint: super::pis::HintGrid<D>,
    pub(super) admission: DescentAdmission,
}

/// Both direction-specific calls for one transaction level.
pub(crate) struct PairedSolveRequest {
    pub(crate) stage: PairSolveStage,
    pub(crate) a_to_b: DirectionSolveRequest<AtoB>,
    pub(crate) b_to_a: DirectionSolveRequest<BtoA>,
}

/// The four frame-static PIS preparations owned by one paired solver session.
///
/// The scalar scheduler retains shared [`LevelInputs`] views for dense seeds,
/// The CPU oracle builds immutable image, mask, gradient and raw-weight planes
/// once and shares them through `Arc`. Per-stage [`Input`] construction still
/// validates them and rebuilds rolling patch sums and source models. That
/// CPU-only cost remains outside [`PairedControlInputs`] and is not part of a
/// future resident GPU session.
#[derive(Clone)]
pub(crate) struct CpuPisOracleInputs {
    a_to_b_l1: LevelInputs,
    a_to_b_l2: LevelInputs,
    b_to_a_l1: LevelInputs,
    b_to_a_l2: LevelInputs,
}

/// Backend-neutral frame data consumed by the scalar schedule after PIS.
///
/// This deliberately excludes gradients, raw weights and PIS source models.
/// A GPU-resident estimator can therefore pair these downstream CPU controls
/// with its own device frame without constructing [`CpuPisOracleInputs`].
pub(crate) struct PairedControlInputs {
    pub(super) current_post_blur: temporal::BlurredBelts,
    pub(super) l1: PreparedLevelImages,
    pub(super) l2: PreparedLevelImages,
    pub(super) a_to_b_lack: dense::LackRows<AtoB>,
    pub(super) b_to_a_lack: dense::LackRows<BtoA>,
    pub(super) l1_block_mask_a: Arc<[u8]>,
}

/// The CPU-oracle compatibility package for one complete cold schedule.
pub(crate) struct ColdPreparedSchedule {
    pub(crate) controls: PairedControlInputs,
    pub(crate) solver: CpuPairedPisSolver,
}

impl CpuPisOracleInputs {
    pub(super) fn new(
        a_to_b_l1: &LevelInputs,
        a_to_b_l2: &LevelInputs,
        b_to_a_l1: &LevelInputs,
        b_to_a_l2: &LevelInputs,
    ) -> Self {
        Self {
            a_to_b_l1: a_to_b_l1.clone(),
            a_to_b_l2: a_to_b_l2.clone(),
            b_to_a_l1: b_to_a_l1.clone(),
            b_to_a_l2: b_to_a_l2.clone(),
        }
    }

    pub(crate) fn a_to_b(&self, level: Level, cost_modes: Vec<CostMode>) -> Input<AtoB> {
        match level {
            Level::One => &self.a_to_b_l1,
            Level::Two => &self.a_to_b_l2,
        }
        .input::<AtoB>(level, cost_modes)
        .0
    }

    pub(crate) fn b_to_a(&self, level: Level, cost_modes: Vec<CostMode>) -> Input<BtoA> {
        match level {
            Level::One => &self.b_to_a_l1,
            Level::Two => &self.b_to_a_l2,
        }
        .input::<BtoA>(level, cost_modes)
        .0
    }
}

/// Runtime identity returned beside one compile-time direction-labelled grid.
///
/// The redundant stamp is intentional: an asynchronous/GPU adapter can bind
/// a returned allocation to the submitted direction and complete
/// within-transaction stage before the scalar transaction consumes it. The
/// outer adapter must additionally bind its frame/flight identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SolveStamp {
    pub(crate) direction: super::Direction,
    pub(crate) stage: PairSolveStage,
}

pub(crate) struct StampedPatchGrid<D: PisDirection> {
    pub(super) stamp: SolveStamp,
    pub(super) grid: PatchGrid<D>,
}

impl<D: PisDirection> StampedPatchGrid<D> {
    pub(crate) fn new(stamp: SolveStamp, grid: PatchGrid<D>) -> Self {
        Self { stamp, grid }
    }
}

/// A fallible paired solver's typed A-to-B and B-to-A result.
pub(crate) struct PairedPatchGrids {
    pub(crate) a_to_b: StampedPatchGrid<AtoB>,
    pub(crate) b_to_a: StampedPatchGrid<BtoA>,
}

/// Injected boundary for the only fallible work in a scalar transaction.
pub(crate) trait PairedPisSolver {
    type Error;
    const BACKEND: crate::studio_type2::PisBackend;

    fn solve(&mut self, request: PairedSolveRequest) -> Result<PairedPatchGrids, Self::Error>;
}

/// Temporary adapter for the current CPU-prepared producer boundary.
///
/// This is deliberately separate from [`PairedPisSolver`]. The eventual GPU
/// estimator frame implements the dynamic solver contract directly and owns
/// its device preparation from construction; it must not accept, ignore or
/// redundantly rebuild these CPU `LevelInputs`.
/// A returned sparse grid did not belong to its submitted direction/level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SolveStampError {
    Direction {
        expected: super::Direction,
        actual: super::Direction,
    },
    Stage {
        expected: PairSolveStage,
        actual: PairSolveStage,
    },
    Level {
        expected: Level,
        actual: Level,
    },
}

impl fmt::Display for SolveStampError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direction { expected, actual } => write!(
                out,
                "ONE X2 paired PIS returned {actual}, expected {expected}",
            ),
            Self::Stage { expected, actual } => write!(
                out,
                "ONE X2 paired PIS returned {actual}, expected {expected}",
            ),
            Self::Level { expected, actual } => write!(
                out,
                "ONE X2 paired PIS returned {actual}, expected {expected}",
            ),
        }
    }
}

impl Error for SolveStampError {}

/// Failure from one injected paired sparse stage.
#[derive(Debug)]
pub(crate) enum PairSolveError<E> {
    Solver {
        stage: PairSolveStage,
        source: E,
    },
    Stamp {
        stage: PairSolveStage,
        source: SolveStampError,
    },
}

impl<E: fmt::Display> fmt::Display for PairSolveError<E> {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Solver { stage, source } => {
                write!(out, "ONE X2 paired PIS failed at {stage}: {source}")
            }
            Self::Stamp { stage, source } => {
                write!(out, "ONE X2 paired PIS stamp failed at {stage}: {source}")
            }
        }
    }
}

impl<E: Error + 'static> Error for PairSolveError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Solver { source, .. } => Some(source),
            Self::Stamp { source, .. } => Some(source),
        }
    }
}

pub(crate) struct CpuPairedPisSolver {
    prepared: CpuPisOracleInputs,
}

impl CpuPairedPisSolver {
    pub(crate) fn new(prepared: CpuPisOracleInputs) -> Self {
        Self { prepared }
    }

    pub(super) fn into_preparation(self) -> CpuPisOracleInputs {
        self.prepared
    }
}

impl PairedPisSolver for CpuPairedPisSolver {
    type Error = Infallible;
    const BACKEND: crate::studio_type2::PisBackend = crate::studio_type2::PisBackend::Cpu;

    fn solve(&mut self, request: PairedSolveRequest) -> Result<PairedPatchGrids, Self::Error> {
        self.solve_prepared(request)
    }
}

impl CpuPairedPisSolver {
    fn solve_prepared(
        &mut self,
        request: PairedSolveRequest,
    ) -> Result<PairedPatchGrids, Infallible> {
        let PairedSolveRequest {
            stage,
            a_to_b,
            b_to_a,
        } = request;
        let prepared = &self.prepared;
        let level = stage.level();
        let DirectionSolveRequest {
            cost_modes,
            initial: a_initial,
            hint: a_hint,
            admission: a_admission,
        } = a_to_b;
        let a_to_b_input = prepared.a_to_b(level, cost_modes);
        let DirectionSolveRequest {
            cost_modes,
            initial: b_initial,
            hint: b_hint,
            admission: b_admission,
        } = b_to_a;
        let b_to_a_input = prepared.b_to_a(level, cost_modes);
        let (a_to_b, b_to_a) = thread::scope(|scope| {
            let b_to_a = scope.spawn(|| solve_one(b_to_a_input, b_initial, b_hint, b_admission));
            let a_to_b = catch_unwind(AssertUnwindSafe(|| {
                solve_one(a_to_b_input, a_initial, a_hint, a_admission)
            }));
            let b_to_a = b_to_a.join();
            match (a_to_b, b_to_a) {
                (Ok(a_to_b), Ok(b_to_a)) => (a_to_b, b_to_a),
                (Err(panic), _) | (Ok(_), Err(panic)) => resume_unwind(panic),
            }
        });
        Ok(PairedPatchGrids {
            a_to_b: StampedPatchGrid::new(
                SolveStamp {
                    direction: super::Direction::AtoB,
                    stage,
                },
                a_to_b,
            ),
            b_to_a: StampedPatchGrid::new(
                SolveStamp {
                    direction: super::Direction::BtoA,
                    stage,
                },
                b_to_a,
            ),
        })
    }
}

fn solve_one<D: PisDirection>(
    input: Input<D>,
    initial: InitialGrid<D>,
    hint: super::pis::HintGrid<D>,
    admission: DescentAdmission,
) -> PatchGrid<D> {
    pis::solve_with_descent_admission(&input, initial, Some(&hint), admission)
        .expect("paired scalar PIS request has one typed level")
}

pub(super) fn weighted_rows(cost_modes: &[CostMode]) -> usize {
    cost_modes
        .iter()
        .filter(|mode| matches!(mode, CostMode::Weighted))
        .count()
}

type PairedGridResult<E> = Result<(PatchGrid<AtoB>, PatchGrid<BtoA>), PairSolveError<E>>;

pub(crate) fn solve_pair<S: PairedPisSolver>(
    solver: &mut S,
    request: PairedSolveRequest,
) -> PairedGridResult<S::Error> {
    let stage = request.stage;
    let expected_level = stage.level();
    let solved = solver
        .solve(request)
        .map_err(|source| PairSolveError::Solver { stage, source })?;
    Ok((
        validate_grid(stage, expected_level, solved.a_to_b)?,
        validate_grid(stage, expected_level, solved.b_to_a)?,
    ))
}

fn validate_grid<D: PisDirection, E>(
    stage: PairSolveStage,
    expected_level: Level,
    solved: StampedPatchGrid<D>,
) -> Result<PatchGrid<D>, PairSolveError<E>> {
    if solved.stamp.direction != D::DIRECTION {
        return Err(PairSolveError::Stamp {
            stage,
            source: SolveStampError::Direction {
                expected: D::DIRECTION,
                actual: solved.stamp.direction,
            },
        });
    }
    if solved.stamp.stage.level() != expected_level {
        return Err(PairSolveError::Stamp {
            stage,
            source: SolveStampError::Level {
                expected: expected_level,
                actual: solved.stamp.stage.level(),
            },
        });
    }
    if solved.stamp.stage != stage {
        return Err(PairSolveError::Stamp {
            stage,
            source: SolveStampError::Stage {
                expected: stage,
                actual: solved.stamp.stage,
            },
        });
    }
    let actual = solved.grid.level();
    if actual != expected_level {
        return Err(PairSolveError::Stamp {
            stage,
            source: SolveStampError::Level {
                expected: expected_level,
                actual,
            },
        });
    }
    Ok(solved.grid)
}

/// Stateless owner for one cold pair estimate.
#[derive(Clone, Copy, Debug, Default)]
pub struct ColdPair;

impl ColdPair {
    pub const fn new() -> Self {
        Self
    }

    /// Run both selected directions and compose one upload-ready displacement.
    pub fn estimate(self, retained: &ColdInputs) -> ColdEstimate {
        self.transition(retained).estimate
    }

    /// Run one cold pair and retain the complete candidate next state.
    ///
    /// The native transaction is READ and its detached terminal corpus exactly
    /// checks this result. The returned cold state remains distinct until the
    /// numeric owner validates adjacency and converts it for the first warm
    /// calculation.
    pub fn transition(self, retained: &ColdInputs) -> ColdTransition {
        let ColdPreparedSchedule {
            controls,
            mut solver,
        } = ColdPreparedSchedule::from_cpu(retained);
        match self.try_transition_prepared_with_solver(&controls, &mut solver) {
            Ok(transition) => transition,
            Err(PairSolveError::Solver { source, .. }) => match source {},
            Err(PairSolveError::Stamp { source, .. }) => {
                panic!("CPU paired solver returned its own invalid stamp: {source}")
            }
        }
    }

    /// Run a backend-neutral cold schedule through an already-prepared solver.
    pub(crate) fn try_transition_prepared_with_solver<S: PairedPisSolver>(
        self,
        controls: &PairedControlInputs,
        solver: &mut S,
    ) -> Result<ColdTransition, PairSolveError<S::Error>> {
        let a_to_b_finest_inputs = &controls.l1;
        let b_to_a_finest_inputs = &controls.l1;
        let a_to_b_coarse_inputs = &controls.l2;
        let b_to_a_coarse_inputs = &controls.l2;
        let a_to_b_work_rows = RetainedWorkRows::after_cold_lack(&controls.a_to_b_lack);
        let b_to_a_work_rows = RetainedWorkRows::after_cold_lack(&controls.b_to_a_lack);
        let a_to_b_effective = a_to_b_work_rows.effective();
        let b_to_a_effective = b_to_a_work_rows.effective();
        let mut a_to_b_hints = HintPyramid::cold_zeros();
        let mut b_to_a_hints = HintPyramid::cold_zeros();
        let mut a_to_b_cadence = EmptyOverrideCadence::new(0, COLD_EMPTY_OVERRIDE_CADENCE).unwrap();
        let mut b_to_a_cadence = EmptyOverrideCadence::new(0, COLD_EMPTY_OVERRIDE_CADENCE).unwrap();
        let mut a_to_b_median = AtoBMedian::new();
        let mut b_to_a_median = BtoAMedian::new();
        let mut a_to_b_raw = None;
        let mut b_to_a_raw = None;
        let mut a_to_b_l2 = 0;
        let mut b_to_a_l2 = 0;
        let mut a_to_b_l1 = 0;
        let mut b_to_a_l1 = 0;

        for calculation in 0..COLD_INNER_CALCULATIONS {
            let a_l2_modes = a_to_b_effective.modes(Level::Two).to_vec();
            let b_l2_modes = b_to_a_effective.modes(Level::Two).to_vec();
            let a_weighted = weighted_rows(&a_l2_modes);
            let b_weighted = weighted_rows(&b_l2_modes);
            let (a_l2, b_l2) = solve_pair(
                solver,
                PairedSolveRequest {
                    stage: PairSolveStage::Cold {
                        calculation,
                        level: Level::Two,
                    },
                    a_to_b: DirectionSolveRequest {
                        cost_modes: a_l2_modes,
                        initial: InitialGrid::coarse_zeros(),
                        hint: a_to_b_hints.grid(Level::Two),
                        admission: a_to_b_cadence.admission(),
                    },
                    b_to_a: DirectionSolveRequest {
                        cost_modes: b_l2_modes,
                        initial: InitialGrid::coarse_zeros(),
                        hint: b_to_a_hints.grid(Level::Two),
                        admission: b_to_a_cadence.admission(),
                    },
                },
            )?;
            let a_seed = cold_seed(a_to_b_coarse_inputs, a_l2);
            let b_seed = cold_seed(b_to_a_coarse_inputs, b_l2);

            let a_l1_modes = a_to_b_effective.modes(Level::One).to_vec();
            let b_l1_modes = b_to_a_effective.modes(Level::One).to_vec();
            let a_finest_weighted = weighted_rows(&a_l1_modes);
            let b_finest_weighted = weighted_rows(&b_l1_modes);
            let (a_grid, b_grid) = solve_pair(
                solver,
                PairedSolveRequest {
                    stage: PairSolveStage::Cold {
                        calculation,
                        level: Level::One,
                    },
                    a_to_b: DirectionSolveRequest {
                        cost_modes: a_l1_modes,
                        initial: a_seed,
                        hint: a_to_b_hints.grid(Level::One),
                        admission: a_to_b_cadence.admission(),
                    },
                    b_to_a: DirectionSolveRequest {
                        cost_modes: b_l1_modes,
                        initial: b_seed,
                        hint: b_to_a_hints.grid(Level::One),
                        admission: b_to_a_cadence.admission(),
                    },
                },
            )?;

            let a_hint_images = a_to_b_finest_inputs.directed::<AtoB>();
            let b_hint_images = b_to_a_finest_inputs.directed::<BtoA>();
            a_to_b_hints = HintPyramid::from_current_finest(&a_hint_images, &a_grid);
            b_to_a_hints = HintPyramid::from_current_finest(&b_hint_images, &b_grid);
            let a_filtered = a_to_b_median
                .run(a_grid)
                .expect("selected finest grid has the temporal median's level");
            let b_filtered = b_to_a_median
                .run(b_grid)
                .expect("selected finest grid has the temporal median's level");
            a_to_b_raw = Some(finish_direction(a_to_b_finest_inputs, a_filtered));
            b_to_a_raw = Some(finish_direction(b_to_a_finest_inputs, b_filtered));
            a_to_b_l2 = a_weighted;
            b_to_a_l2 = b_weighted;
            a_to_b_l1 = a_finest_weighted;
            b_to_a_l1 = b_finest_weighted;
            a_to_b_cadence = a_to_b_cadence.after_calc();
            b_to_a_cadence = b_to_a_cadence.after_calc();
        }

        let a_to_b = ColdDirectionResult {
            public: finish_cold_public(a_to_b_raw.unwrap()),
            median: a_to_b_median.state(),
            work_rows: a_to_b_work_rows,
            hints: a_to_b_hints,
            cadence: a_to_b_cadence,
            weighted_l2: a_to_b_l2,
            weighted_l1: a_to_b_l1,
        };
        let b_to_a = ColdDirectionResult {
            public: finish_cold_public(b_to_a_raw.unwrap()),
            median: b_to_a_median.state(),
            work_rows: b_to_a_work_rows,
            hints: b_to_a_hints,
            cadence: b_to_a_cadence,
            weighted_l2: b_to_a_l2,
            weighted_l1: b_to_a_l1,
        };
        let (fields, invalid_nodes) =
            DirectedFields::from_public_dense_ref(&a_to_b.public, &b_to_a.public);

        Ok(ColdTransition {
            estimate: ColdEstimate {
                displacement: Displacement::compose(&fields),
                invalid_nodes,
                weighted_rows: WorkRowCounts {
                    a_to_b_l2: a_to_b.weighted_l2,
                    b_to_a_l2: b_to_a.weighted_l2,
                    a_to_b_l1: a_to_b.weighted_l1,
                    b_to_a_l1: b_to_a.weighted_l1,
                },
            },
            candidate_next: ColdNextCandidate {
                references: controls.current_post_blur.clone(),
                a_to_b_public: a_to_b.public,
                b_to_a_public: b_to_a.public,
                a_to_b_median: a_to_b.median,
                b_to_a_median: b_to_a.median,
                a_to_b_work_rows: a_to_b.work_rows,
                b_to_a_work_rows: b_to_a.work_rows,
                a_to_b_hints: a_to_b.hints,
                b_to_a_hints: b_to_a.hints,
                a_to_b_cadence: a_to_b.cadence,
                b_to_a_cadence: b_to_a.cadence,
            },
        })
    }
}

fn cold_seed<D: PisDirection>(
    prepared: &PreparedLevelImages,
    patches: PatchGrid<D>,
) -> InitialGrid<D> {
    let images = prepared.directed::<D>();
    let dense = dense::densify_coarse(&images, patches).expect("densify scalar level two");
    let post = preserve_without_variational_or_retained(dense);
    into_l1_initial_grid(post).expect("scalar level-two field forms a finest seed")
}

struct ColdDirectionResult<D: PisDirection> {
    public: PublicDenseField<D>,
    median: MedianState<D>,
    work_rows: RetainedWorkRows<D>,
    hints: HintPyramid<D>,
    cadence: EmptyOverrideCadence,
    weighted_l2: usize,
    weighted_l1: usize,
}

#[cfg(test)]
trait ColdMedian<D: PisDirection>: Sized {
    fn new() -> Self;
    fn run(&mut self, grid: PatchGrid<D>) -> FilteredPatchGrid<D>;
    fn state(&self) -> MedianState<D>;
}

#[cfg(test)]
impl ColdMedian<AtoB> for AtoBMedian {
    fn new() -> Self {
        Self::new()
    }

    fn run(&mut self, grid: PatchGrid<AtoB>) -> FilteredPatchGrid<AtoB> {
        self.run(grid)
            .expect("selected finest grid has the temporal median's level")
    }

    fn state(&self) -> MedianState<AtoB> {
        self.state()
    }
}

#[cfg(test)]
impl ColdMedian<BtoA> for BtoAMedian {
    fn new() -> Self {
        Self::new()
    }

    fn run(&mut self, grid: PatchGrid<BtoA>) -> FilteredPatchGrid<BtoA> {
        self.run(grid)
            .expect("selected finest grid has the temporal median's level")
    }

    fn state(&self) -> MedianState<BtoA> {
        self.state()
    }
}

#[cfg(test)]
fn run_cold_direction<D, M>(
    finest_inputs: &LevelInputs,
    coarse_inputs: &LevelInputs,
) -> ColdDirectionResult<D>
where
    D: PisDirection,
    M: ColdMedian<D>,
{
    let work_rows = RetainedWorkRows::after_cold_calc(finest_inputs);
    let effective_rows = work_rows.effective();
    let mut hints = HintPyramid::cold_zeros();
    let mut cadence = EmptyOverrideCadence::new(0, COLD_EMPTY_OVERRIDE_CADENCE)
        .expect("selected cold cadence is nonzero");
    let mut median = M::new();
    let mut raw = None;
    let mut weighted_l2 = 0;
    let mut weighted_l1 = 0;

    for _ in 0..COLD_INNER_CALCULATIONS {
        // One pre-increment decision is shared by both spatial levels.
        let admission = cadence.admission();
        let (seed, current_weighted_l2) =
            coarse_seed::<D>(coarse_inputs, &effective_rows, &hints, admission);
        let (grid, current_weighted_l1) =
            finest_solve::<D>(finest_inputs, seed, &effective_rows, &hints, admission);

        // `calcHintFlow` consumes the raw L1 sparse result before the
        // persistent temporal median filters that result for densifying.
        let hint_images = finest_inputs.directed_images::<D>(Level::One);
        hints = HintPyramid::from_current_finest(&hint_images, &grid);
        let filtered = median.run(grid);

        // Native recognizes and repackages the previous public destination
        // before calls two and three. Cold motion is `noArray`, though, so its
        // empty-pyramid guard returns before reading those retained numerics.
        raw = Some(finish_direction(
            &PreparedLevelImages::from_pis(finest_inputs, Level::One),
            filtered,
        ));
        weighted_l2 = current_weighted_l2;
        weighted_l1 = current_weighted_l1;
        cadence = cadence.after_calc();
    }

    ColdDirectionResult {
        // Native publishes only the third raw destination. Periodic repair is
        // an outer transaction step, after all three FDS calls in this
        // direction and, at the pair boundary, after both tasks have joined.
        public: finish_cold_public(raw.expect("cold fold runs three calls")),
        median: median.state(),
        work_rows,
        hints,
        cadence,
        weighted_l2,
        weighted_l1,
    }
}

/// The selected recursive `INTER_NEAREST` mask pyramid.
///
/// Native derives level one from the full retained mask and level two from
/// level one. Both selected resizes are exact 2-to-1 reductions whose source
/// sample is the zero-based top-left node `(2r, 2c)`.
pub(super) struct MaskPyramid {
    level_one: LensPair<Vec<u8>>,
    level_two: LensPair<Vec<u8>>,
}

impl MaskPyramid {
    pub(super) fn build(retained: &ColdInputs) -> Self {
        let level_one = LensPair {
            a: inter_nearest_half(&retained.mask.a, COLS, ROWS).0,
            b: inter_nearest_half(&retained.mask.b, COLS, ROWS).0,
        };
        let level_two = LensPair {
            a: inter_nearest_half(&level_one.a, Level::One.cols(), Level::One.rows()).0,
            b: inter_nearest_half(&level_one.b, Level::One.cols(), Level::One.rows()).0,
        };
        Self {
            level_one,
            level_two,
        }
    }

    fn level(&self, level: Level) -> LensPair<Vec<u8>> {
        match level {
            Level::One => self.level_one.clone(),
            Level::Two => self.level_two.clone(),
        }
    }
}

#[derive(Clone)]
pub(super) struct LevelInputs {
    image: LensPair<Arc<Vec<u8>>>,
    mask: LensPair<Arc<Vec<u8>>>,
    gradient_col: Arc<Vec<f32>>,
    gradient_row: Arc<Vec<f32>>,
    raw_weight: Arc<Vec<f32>>,
}

/// Downstream image views for densification and hint construction.
///
/// PIS-only gradients, weights, masks and source models never enter this
/// type. The images remain in physical lens order and are shared by `Arc`.
pub(crate) struct PreparedLevelImages {
    level: Level,
    image: LensPair<Arc<Vec<u8>>>,
}

impl PreparedLevelImages {
    pub(super) fn from_pis(inputs: &LevelInputs, level: Level) -> Self {
        Self {
            level,
            image: inputs.image.clone(),
        }
    }

    pub(super) fn directed<D: PisDirection>(&self) -> DirectedImages<'_, D> {
        DirectedImages::<D>::from_native_order(
            self.level,
            LensPair {
                a: &self.image.a,
                b: &self.image.b,
            },
        )
        .expect("scalar control images have the selected directed shape")
    }
}

/// Direction-labelled weighted-SSD rows derived from the finest classifier.
///
/// Native classifies level one once, then propagates each contiguous pixel
/// interval geometrically to level two. It does not classify level two again.
#[cfg(test)]
struct WorkRows<D: PisDirection> {
    level_one: Vec<CostMode>,
    level_two: Vec<CostMode>,
    direction: PhantomData<D>,
}

#[cfg(test)]
impl<D: PisDirection> WorkRows<D> {
    fn from_finest(finest: &LevelInputs) -> Self {
        let level_one = finest.classify_modes::<D>(Level::One);
        let level_two = propagate_work_modes(&level_one);
        Self {
            level_one,
            level_two,
            direction: PhantomData,
        }
    }
}

impl LevelInputs {
    pub(super) fn build<D: PisDirection>(
        retained: &ColdInputs,
        masks: &MaskPyramid,
        level: Level,
    ) -> Self {
        let (image_a, cols, rows) = image_level(&retained.image.a, level);
        let (image_b, _, _) = image_level(&retained.image.b, level);
        let LensPair {
            a: mask_a,
            b: mask_b,
        } = masks.level(level);
        debug_assert_eq!((rows, cols), (level.rows(), level.cols()));

        let source = match D::DIRECTION {
            super::Direction::AtoB => &image_a,
            super::Direction::BtoA => &image_b,
        };
        let (mut gradient_col, mut gradient_row) = sobel3(source, cols, rows);
        for index in 0..gradient_col.len() {
            if mask_a[index] == 0 {
                gradient_col[index] = 0.0;
                gradient_row[index] = 0.0;
            }
        }
        let magnitude = gradient_col
            .iter()
            .zip(&gradient_row)
            .map(|(col, row)| col.abs() + row.abs())
            .collect::<Vec<_>>();
        let mut raw_weight = studio_weight_gaussian_3x3(&magnitude, cols, rows);
        for index in 0..raw_weight.len() {
            if mask_a[index] == 0 || raw_weight[index] < 0.0 {
                raw_weight[index] = 0.0;
            }
        }

        Self {
            image: LensPair {
                a: Arc::new(image_a),
                b: Arc::new(image_b),
            },
            mask: LensPair {
                a: Arc::new(mask_a),
                b: Arc::new(mask_b),
            },
            gradient_col: Arc::new(gradient_col),
            gradient_row: Arc::new(gradient_row),
            raw_weight: Arc::new(raw_weight),
        }
    }

    pub(super) fn input<D: PisDirection>(
        &self,
        level: Level,
        cost_modes: Vec<CostMode>,
    ) -> (Input<D>, usize) {
        let weighted = cost_modes
            .iter()
            .filter(|mode| matches!(mode, CostMode::Weighted))
            .count();
        let input = Input::<D>::from_shared_native_order(
            level,
            LensPair {
                a: Arc::clone(&self.image.a),
                b: Arc::clone(&self.image.b),
            },
            LensPair {
                a: Arc::clone(&self.mask.a),
                b: Arc::clone(&self.mask.b),
            },
            Arc::clone(&self.gradient_col),
            Arc::clone(&self.gradient_row),
            Arc::clone(&self.raw_weight),
            cost_modes,
        )
        .expect("scalar preparation satisfies the selected PIS input contract")
        .with_disparity_interval(selected_pis_interval(D::DIRECTION, level));
        (input, weighted)
    }

    #[cfg(test)]
    pub(super) fn directed_images<D: PisDirection>(&self, level: Level) -> DirectedImages<'_, D> {
        DirectedImages::<D>::from_native_order(
            level,
            LensPair {
                a: &self.image.a,
                b: &self.image.b,
            },
        )
        .expect("scalar level images have the selected directed shape")
    }

    /// Reproduce the direction-owned finest lack-of-texture rows that native
    /// stores at `FDS+0x120` before deriving first-calculation work rows, then
    /// retains across nonzero warm calculations.
    pub(super) fn lack_rows<D: PisDirection>(&self, level: Level) -> dense::LackRows<D> {
        let mut patch_values = Vec::with_capacity(level.patches());
        for patch_row in 0..level.patch_rows() {
            for patch_col in 0..level.patch_cols() {
                let mut sum = 0.0f32;
                for row in 0..PATCH_SIZE {
                    for col in 0..PATCH_SIZE {
                        let image_row = patch_row * PATCH_STRIDE + row;
                        let image_col = patch_col * PATCH_STRIDE + col;
                        sum += self.raw_weight[image_row * level.cols() + image_col];
                    }
                }
                patch_values.push(sum);
            }
        }
        dense::calculate_lack_rows::<D>(level, &patch_values)
            .expect("texture grid has the selected patch shape")
    }

    /// Reproduce the selected CPU `GenerateBlockMask` over source mask slot A.
    ///
    /// Native keeps this mask behind a same-shape cache. The selected ONE X2
    /// physical mask is static, so rebuilding it has identical values while
    /// avoiding a second retained cache object in this readable oracle.
    pub(super) fn small_disparity_block_mask(&self, level: Level) -> Box<[u8]> {
        assert_eq!(self.mask.a.len(), level.pixels());
        const VALID_FRACTION: f32 = f32::from_bits(0x3dcc_cccd);
        let mut blocks = vec![u8::MAX; level.patches()];
        for patch_row in 0..level.patch_rows() {
            for patch_col in 0..level.patch_cols() {
                let mut nonzero = 0i32;
                for row in 0..PATCH_SIZE {
                    for col in 0..PATCH_SIZE {
                        let image_row = patch_row * PATCH_STRIDE + row;
                        let image_col = patch_col * PATCH_STRIDE + col;
                        if self.mask.a[image_row * level.cols() + image_col] != 0 {
                            nonzero += 1;
                        }
                    }
                }
                let fraction = nonzero as f32 / (PATCH_SIZE * PATCH_SIZE) as f32;
                if fraction < VALID_FRACTION {
                    blocks[patch_row * level.patch_cols() + patch_col] = 0;
                }
            }
        }
        blocks.into_boxed_slice()
    }

    #[cfg(test)]
    fn classify_modes<D: PisDirection>(&self, level: Level) -> Vec<CostMode> {
        self.lack_rows::<D>(level)
            .rows()
            .iter()
            .map(|low_texture| {
                if *low_texture {
                    CostMode::Weighted
                } else {
                    CostMode::Unweighted
                }
            })
            .collect()
    }
}

impl ColdPreparedSchedule {
    pub(crate) fn from_cpu(retained: &ColdInputs) -> Self {
        let masks = MaskPyramid::build(retained);
        let a_to_b_l1 = LevelInputs::build::<AtoB>(retained, &masks, Level::One);
        let b_to_a_l1 = LevelInputs::build::<BtoA>(retained, &masks, Level::One);
        let a_to_b_l2 = LevelInputs::build::<AtoB>(retained, &masks, Level::Two);
        let b_to_a_l2 = LevelInputs::build::<BtoA>(retained, &masks, Level::Two);
        let controls = PairedControlInputs {
            current_post_blur: retained.blurred_belts(),
            l1: PreparedLevelImages::from_pis(&a_to_b_l1, Level::One),
            l2: PreparedLevelImages::from_pis(&a_to_b_l2, Level::Two),
            a_to_b_lack: a_to_b_l1.lack_rows::<AtoB>(Level::One),
            b_to_a_lack: b_to_a_l1.lack_rows::<BtoA>(Level::One),
            l1_block_mask_a: Arc::from(a_to_b_l1.small_disparity_block_mask(Level::One)),
        };
        let solver = CpuPairedPisSolver::new(CpuPisOracleInputs::new(
            &a_to_b_l1, &a_to_b_l2, &b_to_a_l1, &b_to_a_l2,
        ));
        Self { controls, solver }
    }

    pub(crate) fn into_parts(self) -> (PairedControlInputs, CpuPisOracleInputs) {
        (self.controls, self.solver.into_preparation())
    }
}

pub(super) fn propagate_work_modes(finest: &[CostMode]) -> Vec<CostMode> {
    assert_eq!(finest.len(), Level::One.patch_rows());
    let mut coarse = vec![CostMode::Unweighted; Level::Two.patch_rows()];
    let mut row = 0;
    while row < finest.len() {
        if finest[row] != CostMode::Weighted {
            row += 1;
            continue;
        }

        let first = row;
        while row + 1 < finest.len() && finest[row + 1] == CostMode::Weighted {
            row += 1;
        }
        let last = row;

        let source_top = (PATCH_STRIDE * first) as f32;
        let source_height = (PATCH_STRIDE * (last - first) + PATCH_SIZE) as f32;
        let scale = 2.0f32;
        let patch = PATCH_SIZE as f32;
        let half_patch = patch / 2.0;
        let target_top = source_top / scale;
        let target_bottom = target_top + source_height / scale;
        let top_pixel =
            ((target_top - patch) + half_patch).clamp(0.0, (Level::Two.rows() - PATCH_SIZE) as f32);
        let bottom_pixel = (target_bottom - half_patch).clamp(0.0, (Level::Two.rows() - 1) as f32);
        let last_patch_row = Level::Two.patch_rows() - 1;
        let first_coarse = ((top_pixel / PATCH_STRIDE as f32).ceil() as usize).min(last_patch_row);
        let last_coarse =
            ((bottom_pixel / PATCH_STRIDE as f32).floor() as usize).min(last_patch_row);
        coarse[first_coarse..=last_coarse].fill(CostMode::Weighted);
        row += 1;
    }
    coarse
}

fn image_level(src: &[u8], level: Level) -> (Vec<u8>, usize, usize) {
    let (level_one, cols, rows) = inter_area(src, COLS, ROWS, 2);
    match level {
        Level::One => (level_one, cols, rows),
        Level::Two => inter_area(&level_one, cols, rows, 2),
    }
}

#[cfg(test)]
fn coarse_seed<D: PisDirection>(
    prepared: &LevelInputs,
    rows: &EffectiveWorkRows<D>,
    hints: &HintPyramid<D>,
    admission: DescentAdmission,
) -> (InitialGrid<D>, usize) {
    let level = Level::Two;
    let (input, weighted) = prepared.input::<D>(level, rows.modes(level).to_vec());
    let hint = hints.grid(level);
    let patches = pis::solve_with_descent_admission(
        &input,
        InitialGrid::coarse_zeros(),
        Some(&hint),
        admission,
    )
    .expect("cold level-two scalar solve");
    let images = prepared.directed_images::<D>(level);
    let dense = dense::densify_coarse(&images, patches).expect("densify scalar level two");
    let post = preserve_without_variational_or_retained(dense);
    let seed = into_l1_initial_grid(post).expect("scalar level-two field forms a finest seed");
    (seed, weighted)
}

#[cfg(test)]
fn finest_solve<D: PisDirection>(
    prepared: &LevelInputs,
    seed: InitialGrid<D>,
    rows: &EffectiveWorkRows<D>,
    hints: &HintPyramid<D>,
    admission: DescentAdmission,
) -> (PatchGrid<D>, usize) {
    let level = Level::One;
    let (input, weighted) = prepared.input::<D>(level, rows.modes(level).to_vec());
    let hint = hints.grid(level);
    let patches = pis::solve_with_descent_admission(&input, seed, Some(&hint), admission)
        .expect("cold level-one scalar solve");
    (patches, weighted)
}

fn finish_direction<D: PisDirection>(
    prepared: &PreparedLevelImages,
    filtered: FilteredPatchGrid<D>,
) -> PublicDenseField<D> {
    let images = prepared.directed::<D>();
    let dense = dense::densify_finest(&images, filtered).expect("densify scalar level one");
    dense::finish_linear_x2(preserve_without_variational_or_retained(dense))
        .expect("scalar finest field resizes to the public grid")
}

/// Consume the third raw inner destination at the outer cold boundary.
///
/// Keeping the periodic operation in this single finalizer makes it
/// structurally impossible for the inner fold to blend calls one or two.
fn finish_cold_public<D: PisDirection>(raw: PublicDenseField<D>) -> PublicDenseField<D> {
    blend_periodic_boundary(raw)
}

fn reflect_101(index: isize, extent: usize) -> usize {
    let last = extent as isize - 1;
    let mut index = index;
    if last == 0 {
        return 0;
    }
    while index < 0 || index > last {
        if index < 0 {
            index = -index;
        }
        if index > last {
            index = 2 * last - index;
        }
    }
    index as usize
}

/// Studio's selected CV_32F 3-by-3 Gaussian weight preparation.
///
/// The READ call is `GaussianBlur` over CV_32F with a 3-by-3 kernel, sigmaX 1,
/// sigmaY 0, and `BORDER_REFLECT_101`. A direct 1-by-5 response MEASURES the
/// coefficient bits from Studio 6.0.2's bundled OpenCV 4.7 arm64 library
/// (SHA-256 `cc86bd6db0a7331728bd9e9cb9caa7641503c3ce5354ae15a0366bea6964d9c7`).
/// Explicit CV_32F storage between passes and the compact fused operation order
/// are DERIVED by differential complete-output tests against that exact library
/// at both selected sizes. The direct-dylib oracle source is tracked at
/// `ec97b9d1b03eecd0453533d33ca4b950c90a496f`; accepted receipt SHA-256 is
/// `1dc578692e21eb075da93beded41f487fc7fb3482b522b54369865d25e0c8a05`.
/// It launches neither Studio nor its UI. At the selected odd width its
/// horizontal tail evaluates the last cell in the opposite fused order from
/// the normal SIMD-width cells.
fn studio_weight_gaussian_3x3(src: &[f32], cols: usize, rows: usize) -> Vec<f32> {
    assert_eq!(src.len(), cols * rows);
    assert!(
        (cols, rows) == (15, 270) || (cols, rows) == (30, 540),
        "Studio weight Gaussian only has exact evidence at the selected ONE X2 pyramid sizes"
    );
    const SIDE: f32 = f32::from_bits(0x3e8c_52b9);
    const CENTRE: f32 = f32::from_bits(0x3ee7_5a8e);

    let mut horizontal = vec![0.0f32; src.len()];
    for row in 0..rows {
        for col in 0..cols {
            let left = src[row * cols + reflect_101(col as isize - 1, cols)];
            let middle = src[row * cols + col];
            let right = src[row * cols + reflect_101(col as isize + 1, cols)];
            horizontal[row * cols + col] = if cols % 2 == 1 && col + 1 == cols {
                (left + right).mul_add(SIDE, middle * CENTRE)
            } else {
                middle.mul_add(CENTRE, (left + right) * SIDE)
            };
        }
    }

    let mut output = vec![0.0f32; src.len()];
    for row in 0..rows {
        for col in 0..cols {
            let top = horizontal[reflect_101(row as isize - 1, rows) * cols + col];
            let middle = horizontal[row * cols + col];
            let bottom = horizontal[reflect_101(row as isize + 1, rows) * cols + col];
            output[row * cols + col] = (top + bottom).mul_add(SIDE, middle * CENTRE);
        }
    }
    output
}

fn inter_area(src: &[u8], cols: usize, rows: usize, factor: usize) -> (Vec<u8>, usize, usize) {
    let (output_cols, output_rows) = (cols / factor, rows / factor);
    let area = (factor * factor) as u32;
    let mut output = vec![0u8; output_cols * output_rows];
    for row in 0..output_rows {
        for col in 0..output_cols {
            let mut sum = 0u32;
            for source_row in 0..factor {
                for source_col in 0..factor {
                    sum += u32::from(
                        src[(row * factor + source_row) * cols + col * factor + source_col],
                    );
                }
            }
            output[row * output_cols + col] = ((sum * 2 + area) / (area * 2)) as u8;
        }
    }
    (output, output_cols, output_rows)
}

fn inter_nearest_half(src: &[u8], cols: usize, rows: usize) -> (Vec<u8>, usize, usize) {
    let (output_cols, output_rows) = (cols / 2, rows / 2);
    let mut output = vec![0u8; output_cols * output_rows];
    for row in 0..output_rows {
        for col in 0..output_cols {
            output[row * output_cols + col] = src[(row * 2) * cols + col * 2];
        }
    }
    (output, output_cols, output_rows)
}

fn sobel3(src: &[u8], cols: usize, rows: usize) -> (Vec<f32>, Vec<f32>) {
    let derivative = [-1.0f32, 0.0, 1.0];
    let smooth = [1.0f32, 2.0, 1.0];
    let mut col_gradient = vec![0.0; src.len()];
    let mut row_gradient = vec![0.0; src.len()];
    for row in 0..rows {
        for col in 0..cols {
            let (mut col_sum, mut row_sum) = (0.0, 0.0);
            for row_tap in 0..3isize {
                for col_tap in 0..3isize {
                    let source_row = reflect_101(row as isize + row_tap - 1, rows);
                    let source_col = reflect_101(col as isize + col_tap - 1, cols);
                    let value = f32::from(src[source_row * cols + source_col]);
                    col_sum += value * derivative[col_tap as usize] * smooth[row_tap as usize];
                    row_sum += value * smooth[col_tap as usize] * derivative[row_tap as usize];
                }
            }
            col_gradient[row * cols + col] = col_sum;
            row_gradient[row * cols + col] = row_sum;
        }
    }
    (col_gradient, row_gradient)
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};

    use sha2::{Digest, Sha256};

    use super::*;

    fn serial_cold_transition(retained: &ColdInputs) -> ColdTransition {
        let masks = MaskPyramid::build(retained);
        let a_to_b_finest_inputs = LevelInputs::build::<AtoB>(retained, &masks, Level::One);
        let b_to_a_finest_inputs = LevelInputs::build::<BtoA>(retained, &masks, Level::One);
        let a_to_b_coarse_inputs = LevelInputs::build::<AtoB>(retained, &masks, Level::Two);
        let b_to_a_coarse_inputs = LevelInputs::build::<BtoA>(retained, &masks, Level::Two);
        let a_to_b =
            run_cold_direction::<AtoB, AtoBMedian>(&a_to_b_finest_inputs, &a_to_b_coarse_inputs);
        let b_to_a =
            run_cold_direction::<BtoA, BtoAMedian>(&b_to_a_finest_inputs, &b_to_a_coarse_inputs);
        let (fields, invalid_nodes) =
            DirectedFields::from_public_dense_ref(&a_to_b.public, &b_to_a.public);

        ColdTransition {
            estimate: ColdEstimate {
                displacement: Displacement::compose(&fields),
                invalid_nodes,
                weighted_rows: WorkRowCounts {
                    a_to_b_l2: a_to_b.weighted_l2,
                    b_to_a_l2: b_to_a.weighted_l2,
                    a_to_b_l1: a_to_b.weighted_l1,
                    b_to_a_l1: b_to_a.weighted_l1,
                },
            },
            candidate_next: ColdNextCandidate {
                references: retained.blurred_belts(),
                a_to_b_public: a_to_b.public,
                b_to_a_public: b_to_a.public,
                a_to_b_median: a_to_b.median,
                b_to_a_median: b_to_a.median,
                a_to_b_work_rows: a_to_b.work_rows,
                b_to_a_work_rows: b_to_a.work_rows,
                a_to_b_hints: a_to_b.hints,
                b_to_a_hints: b_to_a.hints,
                a_to_b_cadence: a_to_b.cadence,
                b_to_a_cadence: b_to_a.cadence,
            },
        }
    }

    fn assert_f32_bits_eq(actual: &[f32], expected: &[f32], label: &str) {
        assert_eq!(actual.len(), expected.len(), "{label} length differs");
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(
                actual.to_bits(),
                expected.to_bits(),
                "{label} differs at sample {index}",
            );
        }
    }

    fn assert_public_bits_eq<D: PisDirection>(
        actual: &PublicDenseField<D>,
        expected: &PublicDenseField<D>,
        label: &str,
    ) {
        assert_f32_bits_eq(actual.dcol(), expected.dcol(), &format!("{label} dcol"));
        assert_f32_bits_eq(actual.drow(), expected.drow(), &format!("{label} drow"));
    }

    fn assert_hint_bits_eq<D: PisDirection>(
        actual: &HintPyramid<D>,
        expected: &HintPyramid<D>,
        label: &str,
    ) {
        for level in [Level::One, Level::Two] {
            let actual = actual.level(level);
            let expected = expected.level(level);
            assert_f32_bits_eq(
                actual.dcol(),
                expected.dcol(),
                &format!("{label} {level} dcol"),
            );
            assert_f32_bits_eq(
                actual.drow(),
                expected.drow(),
                &format!("{label} {level} drow"),
            );
        }
    }

    fn assert_median_bits_eq<D: PisDirection>(
        actual: &MedianState<D>,
        expected: &MedianState<D>,
        label: &str,
    ) {
        assert_eq!(
            actual.histogram(),
            expected.histogram(),
            "{label} histogram"
        );
        assert_eq!(actual.offsets(), expected.offsets(), "{label} offsets");
        assert_f32_bits_eq(
            actual.values(),
            expected.values(),
            &format!("{label} values"),
        );
    }

    const RUN06_PAYLOAD_BYTES: usize = 7_238_679;
    const RUN06_PAYLOAD_SHA256: &str =
        "a5311370d7946d45094cc85c8ff6631bf2b1195e8c127e3b1f909310168ce8c1";
    const RUN06_EVIDENCE_BYTES: usize = 6_719_162;
    const RUN06_EVIDENCE_SHA256: &str =
        "9f8f531071a5bfcd03d6ff3eb73fc4f57b2d99b02a5dfecc0bc9497418da6baf";
    const RUN06_POSTBLUR_A: WarmRecord = WarmRecord::new(
        "shared_postblur_A",
        37,
        64_800,
        "c802e909b7970e8e94d77c398fd3faba9040f194c190101b15f4a3e3ac201a71",
    );
    const RUN06_POSTBLUR_B: WarmRecord = WarmRecord::new(
        "shared_postblur_B",
        64_866,
        64_800,
        "0101d1c611bb5c2234019b9c5bdb4f0a34197017b782414cfd6f4b485d5e4b9d",
    );
    const RUN06_MASK_0: WarmRecord = WarmRecord::new(
        "shared_physical_mask_0",
        259_380,
        64_800,
        "20a85a8bf17041939185190513e14057ef31827a3c343e379ee8deea6f1a6d50",
    );
    const RUN06_MASK_1: WarmRecord = WarmRecord::new(
        "shared_physical_mask_1",
        324_214,
        64_800,
        "20a85a8bf17041939185190513e14057ef31827a3c343e379ee8deea6f1a6d50",
    );
    const RUN06_MOTION: WarmRecord = WarmRecord::new(
        "shared_motion_map",
        389_043,
        64_800,
        "e80db2c4a997464700cad6f0e4d6fd35104935bfe76ddd97e8bfac5a8cf00554",
    );
    const RUN06_AB_L2: WarmL2Records = WarmL2Records {
        rows: WarmRecord::new(
            "ab_retained_rows_d8_2",
            1_507_127,
            16,
            "de3ae02d748c43ef1b0ac89c7c15214456e2d756fa76b33008ba01dc23b76aee",
        ),
        main_u: WarmRecord::new(
            "ab_l2_pre_pis_main_u",
            1_507_175,
            16_200,
            "9dd76d9311e87123c936ab0d956e9ead34dae3fb68beb075b6ee6fee9b08d9e0",
        ),
        main_v: WarmRecord::new(
            "ab_l2_pre_pis_main_v",
            1_523_407,
            16_200,
            "9dd76d9311e87123c936ab0d956e9ead34dae3fb68beb075b6ee6fee9b08d9e0",
        ),
        hint_u: WarmRecord::new(
            "ab_l2_pre_pis_aux_hint_u",
            1_539_643,
            16_200,
            "7a2813955bf6fb5dc6c7a18aac769739efb2f8b110d948e9a6b4f35213b5faa1",
        ),
        hint_v: WarmRecord::new(
            "ab_l2_pre_pis_aux_hint_v",
            1_555_879,
            16_200,
            "d80c98b0df1d8ba16cb67fb5bbe6e6400e9d7ceb8c2ef6f974bff2fed3503f17",
        ),
        sparse_u: WarmRecord::new(
            "ab_l2_post_pis_sparse_u",
            1_572_114,
            1_056,
            "2b3d93fe9d9c8b2a1e955e0e86ac1d439e79c3c16cd2d02f40177e2103716dac",
        ),
        sparse_v: WarmRecord::new(
            "ab_l2_post_pis_sparse_v",
            1_573_205,
            1_056,
            "459d335bbfc68771ff07a9e373f1d3ebc94222fc66632d2217203b6aae049d60",
        ),
    };
    const RUN06_BA_L2: WarmL2Records = WarmL2Records {
        rows: WarmRecord::new(
            "ba_retained_rows_d8_2",
            2_677_971,
            16,
            "de3ae02d748c43ef1b0ac89c7c15214456e2d756fa76b33008ba01dc23b76aee",
        ),
        main_u: WarmRecord::new(
            "ba_l2_pre_pis_main_u",
            2_678_019,
            16_200,
            "9dd76d9311e87123c936ab0d956e9ead34dae3fb68beb075b6ee6fee9b08d9e0",
        ),
        main_v: WarmRecord::new(
            "ba_l2_pre_pis_main_v",
            2_694_251,
            16_200,
            "9dd76d9311e87123c936ab0d956e9ead34dae3fb68beb075b6ee6fee9b08d9e0",
        ),
        hint_u: WarmRecord::new(
            "ba_l2_pre_pis_aux_hint_u",
            2_710_487,
            16_200,
            "42a7cc46e69db0c983b147a77df353c8c00df46751ee40f583a7390599e24de1",
        ),
        hint_v: WarmRecord::new(
            "ba_l2_pre_pis_aux_hint_v",
            2_726_723,
            16_200,
            "fea54e47213c53ffcfaf558d811db5cbd5bb10d39349d1c1d6b5b05935b0b909",
        ),
        sparse_u: WarmRecord::new(
            "ba_l2_post_pis_sparse_u",
            2_742_958,
            1_056,
            "9d15c7351dad90ee367a38ad8f5fc07d697c44f7a1670e4e71c5c4bfc991725c",
        ),
        sparse_v: WarmRecord::new(
            "ba_l2_post_pis_sparse_v",
            2_744_049,
            1_056,
            "d82d63debe69a5c24dd76858c5a865596e0225eb60b6f57e99a83b958f465728",
        ),
    };

    fn synthetic_weight_plane(cols: usize, rows: usize) -> Vec<f32> {
        let mut state = 0x1357_9bdfu32;
        (0..cols * rows)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) & 0xffff) as f32 / 64.0
            })
            .collect()
    }

    #[test]
    fn studio_weight_gaussian_matches_direct_synthetic_oracles() {
        let finest = synthetic_weight_plane(30, 540);
        assert_eq!(
            f32_sha256(&finest),
            "6f4b53b3f9a6dbceb97dae0fa6b98bab93009f9b6b1e13ea556b62e7241a46ab"
        );
        let finest = studio_weight_gaussian_3x3(&finest, 30, 540);
        assert_eq!(
            f32_sha256(&finest),
            "081606510d3ce4153a7c5f1b1391090f3620402e29273b53407af493cde8b913"
        );
        for (index, bits) in [
            (0, 0x444a_ea37),
            (29, 0x4406_762b),
            (31, 0x4420_4570),
            (8_115, 0x43c8_2b11),
            (16_170, 0x444b_ded3),
            (16_199, 0x440b_ad45),
        ] {
            assert_eq!(finest[index].to_bits(), bits);
        }

        let coarse = synthetic_weight_plane(15, 270);
        assert_eq!(
            f32_sha256(&coarse),
            "a534c81d9bb1eb68c9d2e62014c30c121d6e6e76a15291c476a70e26c03ee031"
        );
        let coarse = studio_weight_gaussian_3x3(&coarse, 15, 270);
        assert_eq!(
            f32_sha256(&coarse),
            "c7b87c0e145b48ceb5a2054fc24414eb2e919e025f4cd972ef8882c701d7d5db"
        );
        for (index, bits) in [
            (0, 0x4437_7e57),
            (14, 0x4401_a1a3),
            (16, 0x4423_7072),
            (2_032, 0x43dc_dc78),
            (4_035, 0x4456_be68),
            (4_049, 0x43f4_cbab),
        ] {
            assert_eq!(coarse[index].to_bits(), bits);
        }
    }

    #[test]
    #[ignore = "requires KJERAG_V9_STUDIO_WEIGHT_DIR with the authenticated accepted planes"]
    fn accepted_studio_weight_plane_matches_exactly() {
        let directory = std::env::var_os("KJERAG_V9_STUDIO_WEIGHT_DIR")
            .map(PathBuf::from)
            .expect("set KJERAG_V9_STUDIO_WEIGHT_DIR to the accepted derived-plane directory");
        // Authenticate every private plane before interpreting any of them.
        let gradient_col = authenticated_v9_file(
            &directory,
            "studio-ab-l2-gradient-col.s16le",
            8_100,
            "58554eab80066c102bed7a7db710078a11f46d09f1e67a3a867ed88722e1e787",
        );
        let gradient_row = authenticated_v9_file(
            &directory,
            "studio-ab-l2-gradient-row.s16le",
            8_100,
            "fd7185976e1e6cb54551b792aea166f3d94e17396e9e372f8a7a8bef1ecb4270",
        );
        let source_mask = authenticated_v9_file(
            &directory,
            "studio-ab-l2-source-mask.u8",
            4_050,
            "e30f604d1d1edb23dab753dfca8724b3caf070aeb4fd3a9a3accb306cd173ce9",
        );
        let studio_weight = authenticated_v9_file(
            &directory,
            "studio-ab-l2-raw-weight.f32le",
            16_200,
            "08fa436db72c82ca823189965c4d8316d32f917abf40b1882005ce77aa4efff5",
        );

        let gradient_col = gradient_col
            .chunks_exact(2)
            .map(|bytes| i16::from_le_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        let gradient_row = gradient_row
            .chunks_exact(2)
            .map(|bytes| i16::from_le_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        let magnitude = gradient_col
            .iter()
            .zip(&gradient_row)
            .map(|(&col, &row)| f32::from(col).abs() + f32::from(row).abs())
            .collect::<Vec<_>>();
        let mut actual = studio_weight_gaussian_3x3(&magnitude, 15, 270);
        for (value, &mask) in actual.iter_mut().zip(&source_mask) {
            if mask == 0 || *value < 0.0 {
                *value = 0.0;
            }
        }
        let expected = f32_plane("accepted Studio AB L2 raw weight", &studio_weight, 4_050);
        assert_eq!(actual, expected);
        assert_eq!(
            f32_sha256(&actual),
            "08fa436db72c82ca823189965c4d8316d32f917abf40b1882005ce77aa4efff5"
        );
    }

    #[test]
    #[ignore = "requires KJERAG_ONE_XS_WARM_PAYLOAD with the authenticated run-06 payload and sibling evidence JSON"]
    fn accepted_run06_both_l2_post_pis_replays_are_bit_exact() {
        let (payload, retained) = authenticated_run06_fixture();
        let masks = MaskPyramid::build(&retained);
        assert_run06_l2_replay::<AtoB>(&payload, &retained, &masks, RUN06_AB_L2);
        assert_run06_l2_replay::<BtoA>(&payload, &retained, &masks, RUN06_BA_L2);
    }

    #[test]
    #[ignore = "requires KJERAG_ONE_XS_WARM_PAYLOAD with the authenticated run-06 payload and sibling evidence JSON"]
    fn accepted_run06_both_l1_post_pis_replays_from_computed_l2_are_bit_exact() {
        let (payload, retained) = authenticated_run06_fixture();
        let masks = MaskPyramid::build(&retained);
        assert_run06_l1_replay::<AtoB>(&payload, &retained, &masks, RUN06_AB_L2, RUN06_AB_L1);
        assert_run06_l1_replay::<BtoA>(&payload, &retained, &masks, RUN06_BA_L2, RUN06_BA_L1);
    }

    // The authenticated event ledger names 22 physical binary payloads. Keep
    // the ledger itself in the contract too, so no event-backed corpus input
    // can be silently omitted before an oracle interprets the stage.
    const V9_ARTIFACTS: [(&str, usize, &str); 23] = [
        (
            "events.jsonl",
            19_913,
            "486f3aa3fd352c73661e678a7183165de137e9ee243698c8a9ad305384e6cbc9",
        ),
        (
            "000_shared_postblur_A.bin",
            64_800,
            "c1e404daa371b37a1285bf397370defa6ac162f342145e4dea86edf6118938d9",
        ),
        (
            "001_shared_postblur_B.bin",
            64_800,
            "80151a171e5aab56d63178d1964440b898fc9186bb1d229cf90c3233bf018f15",
        ),
        (
            "002_shared_physical_mask_0.bin",
            64_800,
            "20a85a8bf17041939185190513e14057ef31827a3c343e379ee8deea6f1a6d50",
        ),
        (
            "003_shared_physical_mask_1.bin",
            64_800,
            "20a85a8bf17041939185190513e14057ef31827a3c343e379ee8deea6f1a6d50",
        ),
        (
            "004_ab_private_hint_densification_mask.bin",
            16_200,
            "d669875402f653cece7ea45a3774a15c8eba0a5d006872d3bb8e02c3a0779ae2",
        ),
        (
            "005_ba_private_hint_densification_mask.bin",
            16_200,
            "d669875402f653cece7ea45a3774a15c8eba0a5d006872d3bb8e02c3a0779ae2",
        ),
        (
            "006_ab_retained_rows_120.bin",
            24,
            "e7ddce7bf0b5e15fdf297b3f361358f489d3126b57f04b6d8b6787a31899a4d9",
        ),
        (
            "007_ab_retained_rows_d8_1.bin",
            24,
            "e7ddce7bf0b5e15fdf297b3f361358f489d3126b57f04b6d8b6787a31899a4d9",
        ),
        (
            "008_ab_retained_rows_d8_2.bin",
            16,
            "23b16ea2aab8780bee8a8b761fc9916b38bca0e23270cfe711e347ecbc0fceb6",
        ),
        (
            "009_ba_retained_rows_120.bin",
            24,
            "48fd3590197582cb83a648977f95a47ff00a74af8f7014de494fcbffc363543f",
        ),
        (
            "010_ba_retained_rows_d8_1.bin",
            24,
            "48fd3590197582cb83a648977f95a47ff00a74af8f7014de494fcbffc363543f",
        ),
        (
            "011_ba_retained_rows_d8_2.bin",
            16,
            "4c3383507e09ed374a164d739ff5421e70bf17dd3ebea5d30439d9677ba398ce",
        ),
        (
            "012_ab_l2_pre_pis_main_u.bin",
            16_200,
            "9dd76d9311e87123c936ab0d956e9ead34dae3fb68beb075b6ee6fee9b08d9e0",
        ),
        (
            "013_ab_l2_pre_pis_main_v.bin",
            16_200,
            "9dd76d9311e87123c936ab0d956e9ead34dae3fb68beb075b6ee6fee9b08d9e0",
        ),
        (
            "014_ab_l2_pre_pis_aux_hint_u.bin",
            16_200,
            "9dd76d9311e87123c936ab0d956e9ead34dae3fb68beb075b6ee6fee9b08d9e0",
        ),
        (
            "015_ab_l2_pre_pis_aux_hint_v.bin",
            16_200,
            "9dd76d9311e87123c936ab0d956e9ead34dae3fb68beb075b6ee6fee9b08d9e0",
        ),
        (
            "016_ba_l2_pre_pis_main_u.bin",
            16_200,
            "9dd76d9311e87123c936ab0d956e9ead34dae3fb68beb075b6ee6fee9b08d9e0",
        ),
        (
            "017_ba_l2_pre_pis_main_v.bin",
            16_200,
            "9dd76d9311e87123c936ab0d956e9ead34dae3fb68beb075b6ee6fee9b08d9e0",
        ),
        (
            "018_ba_l2_pre_pis_aux_hint_u.bin",
            16_200,
            "9dd76d9311e87123c936ab0d956e9ead34dae3fb68beb075b6ee6fee9b08d9e0",
        ),
        (
            "019_ba_l2_pre_pis_aux_hint_v.bin",
            16_200,
            "9dd76d9311e87123c936ab0d956e9ead34dae3fb68beb075b6ee6fee9b08d9e0",
        ),
        (
            "020_ab_l2_post_pis_sparse_u.bin",
            1_056,
            "3742bf1751c2b59b2bd8f5500277694b993b64ff0f03f702f7cec1f3f628be1a",
        ),
        (
            "021_ab_l2_post_pis_sparse_v.bin",
            1_056,
            "87f12ff7010f00df4f0bc657452f6dfec0dbf2be5f498e4cc9a522c3bf181f8b",
        ),
    ];

    fn v9_stage() -> PathBuf {
        std::env::var_os("KJERAG_V9_STAGE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                panic!("set KJERAG_V9_STAGE directly to the partial stage_v9 directory")
            })
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        hex_digest(&Sha256::digest(bytes))
    }

    fn hex_digest(bytes: &[u8]) -> String {
        let mut output = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            write!(&mut output, "{byte:02x}").unwrap();
        }
        output
    }

    fn authenticated_v9_file(stage: &Path, name: &str, size: usize, hash: &str) -> Vec<u8> {
        let path = stage.join(name);
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
        assert_eq!(bytes.len(), size, "{name} byte count differs");
        assert_eq!(sha256_hex(&bytes), hash, "{name} SHA-256 differs");
        bytes
    }

    #[derive(Clone, Copy)]
    struct WarmRecord {
        label: &'static str,
        offset: usize,
        bytes: usize,
        sha256: &'static str,
    }

    impl WarmRecord {
        const fn new(
            label: &'static str,
            offset: usize,
            bytes: usize,
            sha256: &'static str,
        ) -> Self {
            Self {
                label,
                offset,
                bytes,
                sha256,
            }
        }
    }

    #[derive(Clone, Copy)]
    struct WarmL2Records {
        rows: WarmRecord,
        main_u: WarmRecord,
        main_v: WarmRecord,
        hint_u: WarmRecord,
        hint_v: WarmRecord,
        sparse_u: WarmRecord,
        sparse_v: WarmRecord,
    }

    #[derive(Clone, Copy)]
    struct WarmL1Records {
        prior_public: WarmRecord,
        rows: WarmRecord,
        main_u: WarmRecord,
        main_v: WarmRecord,
        hint_u: WarmRecord,
        hint_v: WarmRecord,
        sparse_u: WarmRecord,
        sparse_v: WarmRecord,
    }

    const RUN06_AB_L1: WarmL1Records = WarmL1Records {
        prior_public: WarmRecord::new(
            "shared_pre_public_c30",
            3_832_456,
            518_400,
            "d7cd591d6e96469ddb5ef91152b9643554cf6b0435831083d51fdbfc055cd319",
        ),
        rows: WarmRecord::new(
            "ab_retained_rows_d8_1",
            1_507_070,
            24,
            "2c26920da6fe8c3f03a7c57f0d17fd8046113223d66f5fdc419e7381bf95e70d",
        ),
        main_u: WarmRecord::new(
            "ab_l1_pre_pis_main_u",
            1_833_895,
            64_800,
            "0babe3dbe33360091772a1669c6c14b7670242500597a77b8fe2444edfd66f05",
        ),
        main_v: WarmRecord::new(
            "ab_l1_pre_pis_main_v",
            1_898_727,
            64_800,
            "194bbea4e59230942844bd94da87b1d9dbb11079dc102ae0d2af6cfb94930965",
        ),
        hint_u: WarmRecord::new(
            "ab_l1_pre_pis_aux_hint_u",
            1_963_563,
            64_800,
            "939a4e623432161a1f392eac4b39cfb82109aa61935c99a8e01826ada5073f6f",
        ),
        hint_v: WarmRecord::new(
            "ab_l1_pre_pis_aux_hint_v",
            2_028_399,
            64_800,
            "4ebf756ce28a09a3af31ed44b5d271891def4311825ca120a8597f58a2e688c4",
        ),
        sparse_u: WarmRecord::new(
            "ab_l1_post_pis_sparse_u",
            2_093_234,
            5_696,
            "cbdcfdd4963fc75a979d04a270709ade9fde9d885a282646104197042f6dbfad",
        ),
        sparse_v: WarmRecord::new(
            "ab_l1_post_pis_sparse_v",
            2_098_965,
            5_696,
            "7d5069548ea204ba67f132aeb3fdb08ee4aa72452d124cb7c32e9d5b96537ed6",
        ),
    };

    const RUN06_BA_L1: WarmL1Records = WarmL1Records {
        prior_public: WarmRecord::new(
            "shared_pre_public_c90",
            4_350_889,
            518_400,
            "4d670d4a3ed16675b52c96bb4ebd807c196f77754b1c51161513b8ecfdecd26c",
        ),
        rows: WarmRecord::new(
            "ba_retained_rows_d8_1",
            2_677_914,
            24,
            "2c26920da6fe8c3f03a7c57f0d17fd8046113223d66f5fdc419e7381bf95e70d",
        ),
        main_u: WarmRecord::new(
            "ba_l1_pre_pis_main_u",
            3_004_739,
            64_800,
            "86fbec53fc9b595eb778f4af0ea0f202d2a8180a3f9d3328abb96871b4bfc51c",
        ),
        main_v: WarmRecord::new(
            "ba_l1_pre_pis_main_v",
            3_069_571,
            64_800,
            "50259cd5a3670225692f009c7e149a9777fcbbd2c560fb25e5720b29096ad454",
        ),
        hint_u: WarmRecord::new(
            "ba_l1_pre_pis_aux_hint_u",
            3_134_407,
            64_800,
            "fe4f617b194f322035872ff664089cfee480eb9e77df3dbf1a5a8d9906016408",
        ),
        hint_v: WarmRecord::new(
            "ba_l1_pre_pis_aux_hint_v",
            3_199_243,
            64_800,
            "620f632a9e30408561b2339473fe9d7df736b470d3b34008d3bfdb1a19bd7fd2",
        ),
        sparse_u: WarmRecord::new(
            "ba_l1_post_pis_sparse_u",
            3_264_078,
            5_696,
            "b7f07190bd0759114abc6fda65518d8be2dd2b658d518bbc683240767bcdb6fa",
        ),
        sparse_v: WarmRecord::new(
            "ba_l1_post_pis_sparse_v",
            3_269_809,
            5_696,
            "1bdc0227a55719404eb79785fb398b7c83b39d9383b67bf2a6d815833d5f0f32",
        ),
    };

    fn authenticated_run06_fixture() -> (Vec<u8>, ColdInputs) {
        let path = PathBuf::from(std::env::var_os("KJERAG_ONE_XS_WARM_PAYLOAD").expect(
            "set KJERAG_ONE_XS_WARM_PAYLOAD to authenticated run-06 warm-pair-payload.bin",
        ));
        let payload = std::fs::read(&path).expect("could not read authenticated run-06 payload");
        assert_eq!(payload.len(), RUN06_PAYLOAD_BYTES);
        assert_eq!(&payload[..8], b"KJWP602\x04");
        assert_eq!(sha256_hex(&payload), RUN06_PAYLOAD_SHA256);
        let evidence = std::fs::read(path.with_file_name("warm-pair-evidence.json"))
            .expect("could not read authenticated run-06 evidence JSON beside the payload");
        assert_eq!(evidence.len(), RUN06_EVIDENCE_BYTES);
        assert_eq!(sha256_hex(&evidence), RUN06_EVIDENCE_SHA256);

        // Run06 binds these records to warm `owner_pre_motion` at
        // `+0x2d4c224 bl calcMotion`, with semantic epoch
        // `post_blur_pre_calcMotion`. They are already past the selected input
        // Gaussian. Section 163B's pre-blur boundary belongs to the different
        // cold `+0x2d4c13c` OpticalFlow::calc call site.
        let retained = ColdInputs::from_prepared(
            LensPair {
                a: warm_record_bytes(&payload, RUN06_POSTBLUR_A).to_vec(),
                b: warm_record_bytes(&payload, RUN06_POSTBLUR_B).to_vec(),
            },
            LensPair {
                a: warm_record_bytes(&payload, RUN06_MASK_0).to_vec(),
                b: warm_record_bytes(&payload, RUN06_MASK_1).to_vec(),
            },
        )
        .expect("accepted run-06 post-input-blur state has retained-grid shape");
        (payload, retained)
    }

    fn warm_record_bytes(payload: &[u8], record: WarmRecord) -> &[u8] {
        let label_start = record
            .offset
            .checked_sub(record.label.len())
            .expect("warm record label offset underflowed");
        let header_start = label_start
            .checked_sub(12)
            .expect("warm record header offset underflowed");
        let end = record
            .offset
            .checked_add(record.bytes)
            .expect("warm record end overflowed");
        assert!(
            end <= payload.len(),
            "{} is outside the payload",
            record.label
        );
        assert_eq!(
            u32::from_le_bytes(payload[header_start..header_start + 4].try_into().unwrap()),
            record.label.len() as u32,
            "{} label length differs",
            record.label,
        );
        assert_eq!(
            u64::from_le_bytes(payload[header_start + 4..label_start].try_into().unwrap()),
            record.bytes as u64,
            "{} byte count differs",
            record.label,
        );
        assert_eq!(
            &payload[label_start..record.offset],
            record.label.as_bytes(),
            "{} evidence label differs",
            record.label,
        );
        let bytes = &payload[record.offset..end];
        assert_eq!(
            sha256_hex(bytes),
            record.sha256,
            "{} SHA-256 differs",
            record.label,
        );
        bytes
    }

    fn warm_record_f32(payload: &[u8], record: WarmRecord, values: usize) -> Vec<f32> {
        f32_plane(record.label, warm_record_bytes(payload, record), values)
    }

    fn warm_hint_grid<D: PisDirection>(
        payload: &[u8],
        level: Level,
        u: WarmRecord,
        v: WarmRecord,
    ) -> pis::HintGrid<D> {
        let aux_u = warm_record_f32(payload, u, level.pixels());
        let aux_v = warm_record_f32(payload, v, level.pixels());
        let flows = (0..level.patch_rows())
            .flat_map(|patch_row| {
                let aux_u = &aux_u;
                let aux_v = &aux_v;
                (0..level.patch_cols()).map(move |patch_col| {
                    let row = patch_row * PATCH_STRIDE + PATCH_SIZE / 2;
                    let col = patch_col * PATCH_STRIDE + PATCH_SIZE / 2;
                    let at = row * level.cols() + col;
                    pis::Flow::new(aux_u[at], aux_v[at])
                        .expect("accepted run-06 auxiliary centre is finite")
                })
            })
            .collect::<Vec<_>>();
        pis::HintGrid::<D>::from_row_major(level, flows)
            .expect("accepted run-06 auxiliary centres have the selected patch shape")
    }

    fn warm_public_field<D: PisDirection>(
        payload: &[u8],
        record: WarmRecord,
    ) -> PublicDenseField<D> {
        let bytes = warm_record_bytes(payload, record);
        let mut dcol = Vec::with_capacity(ROWS * COLS);
        let mut drow = Vec::with_capacity(ROWS * COLS);
        for vector in bytes.chunks_exact(8) {
            dcol.push(f32::from_le_bytes(vector[..4].try_into().unwrap()));
            drow.push(f32::from_le_bytes(vector[4..].try_into().unwrap()));
        }
        PublicDenseField::from_row_major_components(dcol, drow)
            .expect("accepted run-06 retained public field has the public-grid shape")
    }

    fn assert_run06_l2_replay<D: PisDirection>(
        payload: &[u8],
        retained: &ColdInputs,
        masks: &MaskPyramid,
        records: WarmL2Records,
    ) -> PatchGrid<D> {
        let prepared = LevelInputs::build::<D>(retained, masks, Level::Two);
        let modes = native_work_modes(warm_record_bytes(payload, records.rows), Level::Two);

        // The evidence labels bind both main dense inputs at the pre-PIS
        // epoch. They are bit-zero, so the typed zero grid is their exact
        // sparse-centre representation rather than an inferred cold default.
        for record in [records.main_u, records.main_v] {
            assert!(
                warm_record_bytes(payload, record)
                    .chunks_exact(4)
                    .all(|word| u32::from_le_bytes(word.try_into().unwrap()) == 0),
                "{} is not the captured zero main plane",
                record.label,
            );
        }

        // Native PIS consumes the dense auxiliary pair at sparse centres
        // `(3*r+4, 3*c+4)`, in row-major order.
        let hint = warm_hint_grid::<D>(payload, Level::Two, records.hint_u, records.hint_v);
        let (input, _) = prepared.input::<D>(Level::Two, modes);

        // Each direction's authenticated scalar capsule has FDS+0x9c=0,
        // FDS+0xa8=10 and FDS+0xc8=6372. Its independently captured
        // inverse-interval table is canonical logical-empty incoming,
        // post-producer and outgoing. Both levels execute before c8 advances.
        let admission = pis::DescentAdmission::from_empty_override_cadence(6_372, 10);
        assert_eq!(admission, pis::DescentAdmission::NoPatches);
        let actual = pis::solve_with_descent_admission(
            &input,
            InitialGrid::coarse_zeros(),
            Some(&hint),
            admission,
        )
        .unwrap_or_else(|error| panic!("accepted run-06 {} L2 PIS replay: {error}", D::DIRECTION));
        assert!(actual.patches().iter().all(|patch| {
            patch
                .passes()
                .iter()
                .all(|pass| !pass.descent_admitted() && pass.descent_iterations() == 0)
        }));

        let actual_u = actual
            .patches()
            .iter()
            .map(|patch| patch.flow().dcol().to_bits())
            .collect::<Vec<_>>();
        let actual_v = actual
            .patches()
            .iter()
            .map(|patch| patch.flow().drow().to_bits())
            .collect::<Vec<_>>();
        let expected_u = warm_record_f32(payload, records.sparse_u, Level::Two.patches())
            .into_iter()
            .map(f32::to_bits)
            .collect::<Vec<_>>();
        let expected_v = warm_record_f32(payload, records.sparse_v, Level::Two.patches())
            .into_iter()
            .map(f32::to_bits)
            .collect::<Vec<_>>();

        let hint_bits = hint
            .flows()
            .iter()
            .map(|flow| [flow.dcol().to_bits(), flow.drow().to_bits()])
            .collect::<Vec<_>>();
        let studio_from_seed_set = expected_u
            .iter()
            .zip(&expected_v)
            .filter(|&(u, v)| {
                let flow = [*u, *v];
                flow == [0, 0] || hint_bits.contains(&flow)
            })
            .count();
        assert_eq!(studio_from_seed_set, Level::Two.patches());

        assert_eq!(actual_u, expected_u, "{} L2 dcolumn", D::DIRECTION);
        assert_eq!(actual_v, expected_v, "{} L2 drow", D::DIRECTION);
        assert_eq!(f32_bits_sha256(&actual_u), records.sparse_u.sha256);
        assert_eq!(f32_bits_sha256(&actual_v), records.sparse_v.sha256);
        actual
    }

    fn assert_run06_l1_replay<D: PisDirection>(
        payload: &[u8],
        retained: &ColdInputs,
        masks: &MaskPyramid,
        l2: WarmL2Records,
        l1: WarmL1Records,
    ) {
        use crate::flow::one_xs::post_update::{
            MotionPyramid, RetainedPublicPyramids, update_without_variational_with_retained,
        };

        // The L1 initial grid is computed from the authenticated L2 semantic
        // inputs and exact L2 PIS result. Captured L1 main planes are checked
        // only after this chain; they never enter the solve as its seed.
        let l2_patches = assert_run06_l2_replay::<D>(payload, retained, masks, l2);
        let l2_prepared = LevelInputs::build::<D>(retained, masks, Level::Two);
        let l2_dense =
            dense::densify_coarse(&l2_prepared.directed_images::<D>(Level::Two), l2_patches)
                .expect("accepted run-06 computed L2 patches densify at L2");
        let retained_public =
            RetainedPublicPyramids::from_public(warm_public_field::<D>(payload, l1.prior_public));
        let motion = MotionPyramid::from_base_bytes(warm_record_bytes(payload, RUN06_MOTION));
        let l2_post = update_without_variational_with_retained(
            l2_dense,
            &retained_public,
            motion.level::<D>(Level::Two),
        )
        .expect("accepted run-06 computed L2 field and motion share L2");
        let seed = into_l1_initial_grid(l2_post)
            .expect("accepted run-06 computed L2 post-update forms the L1 seed");

        let captured_main_u = warm_record_f32(payload, l1.main_u, Level::One.pixels());
        let captured_main_v = warm_record_f32(payload, l1.main_v, Level::One.pixels());
        for (patch, flow) in seed.flows().iter().enumerate() {
            let patch_row = patch / Level::One.patch_cols();
            let patch_col = patch % Level::One.patch_cols();
            let row = patch_row * PATCH_STRIDE + PATCH_SIZE / 2;
            let col = patch_col * PATCH_STRIDE + PATCH_SIZE / 2;
            let pixel = row * Level::One.cols() + col;
            assert_eq!(
                flow.dcol().to_bits(),
                captured_main_u[pixel].to_bits(),
                "{} computed L1 seed dcolumn at patch {patch}",
                D::DIRECTION,
            );
            assert_eq!(
                flow.drow().to_bits(),
                captured_main_v[pixel].to_bits(),
                "{} computed L1 seed drow at patch {patch}",
                D::DIRECTION,
            );
        }

        let prepared = LevelInputs::build::<D>(retained, masks, Level::One);
        let modes = native_work_modes(warm_record_bytes(payload, l1.rows), Level::One);
        let (input, _) = prepared.input::<D>(Level::One, modes);
        let hint = warm_hint_grid::<D>(payload, Level::One, l1.hint_u, l1.hint_v);
        let admission = pis::DescentAdmission::from_empty_override_cadence(6_372, 10);
        assert_eq!(admission, pis::DescentAdmission::NoPatches);
        let actual = pis::solve_with_descent_admission(&input, seed, Some(&hint), admission)
            .unwrap_or_else(|error| {
                panic!("accepted run-06 {} L1 PIS replay: {error}", D::DIRECTION)
            });
        assert!(actual.patches().iter().all(|patch| {
            patch
                .passes()
                .iter()
                .all(|pass| !pass.descent_admitted() && pass.descent_iterations() == 0)
        }));

        let actual_u = actual
            .patches()
            .iter()
            .map(|patch| patch.flow().dcol().to_bits())
            .collect::<Vec<_>>();
        let actual_v = actual
            .patches()
            .iter()
            .map(|patch| patch.flow().drow().to_bits())
            .collect::<Vec<_>>();
        let expected_u = warm_record_f32(payload, l1.sparse_u, Level::One.patches())
            .into_iter()
            .map(f32::to_bits)
            .collect::<Vec<_>>();
        let expected_v = warm_record_f32(payload, l1.sparse_v, Level::One.patches())
            .into_iter()
            .map(f32::to_bits)
            .collect::<Vec<_>>();

        assert_eq!(actual_u, expected_u, "{} L1 dcolumn", D::DIRECTION);
        assert_eq!(actual_v, expected_v, "{} L1 drow", D::DIRECTION);
        assert_eq!(f32_bits_sha256(&actual_u), l1.sparse_u.sha256);
        assert_eq!(f32_bits_sha256(&actual_v), l1.sparse_v.sha256);
    }

    fn unique_json_object<'a>(json: &'a str, key: &str) -> &'a str {
        let marker = format!(r#""{key}": {{"#);
        let mut matches = json.match_indices(&marker);
        let (start, _) = matches
            .next()
            .unwrap_or_else(|| panic!("missing JSON object {key}"));
        assert!(matches.next().is_none(), "duplicate JSON object {key}");
        let object_start = start + marker.len() - 1;
        let mut depth = 0usize;
        let mut quoted = false;
        let mut escaped = false;
        for (offset, byte) in json.as_bytes()[object_start..].iter().copied().enumerate() {
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
                continue;
            }
            match byte {
                b'"' => quoted = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &json[object_start..=object_start + offset];
                    }
                }
                _ => {}
            }
        }
        panic!("unterminated JSON object {key}");
    }

    fn unique_json_array<'a>(json: &'a str, key: &str) -> &'a str {
        let marker = format!(r#""{key}": ["#);
        let mut matches = json.match_indices(&marker);
        let (start, _) = matches
            .next()
            .unwrap_or_else(|| panic!("missing JSON array {key}"));
        assert!(matches.next().is_none(), "duplicate JSON array {key}");
        let array_start = start + marker.len() - 1;
        let mut depth = 0usize;
        let mut quoted = false;
        let mut escaped = false;
        for (offset, byte) in json.as_bytes()[array_start..].iter().copied().enumerate() {
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
                continue;
            }
            match byte {
                b'"' => quoted = true,
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return &json[array_start..=array_start + offset];
                    }
                }
                _ => {}
            }
        }
        panic!("unterminated JSON array {key}");
    }

    fn json_object_elements(array: &str) -> Vec<&str> {
        assert!(array.starts_with('[') && array.ends_with(']'));
        let mut objects = Vec::new();
        let mut start = None;
        let mut depth = 0usize;
        let mut quoted = false;
        let mut escaped = false;
        for (offset, byte) in array.bytes().enumerate() {
            if quoted {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    quoted = false;
                }
                continue;
            }
            match byte {
                b'"' => quoted = true,
                b'{' => {
                    if depth == 0 {
                        start = Some(offset);
                    }
                    depth += 1;
                }
                b'}' => {
                    assert!(depth > 0, "JSON array has an unmatched object close");
                    depth -= 1;
                    if depth == 0 {
                        let start = start.take().expect("JSON object start disappeared");
                        objects.push(&array[start..=offset]);
                    }
                }
                _ => {}
            }
        }
        assert_eq!(depth, 0, "JSON array has an unterminated object");
        objects
    }

    fn unique_json_string<'a>(json: &'a str, key: &str) -> &'a str {
        let marker = format!(r#""{key}": ""#);
        let mut matches = json.match_indices(&marker);
        let (start, _) = matches
            .next()
            .unwrap_or_else(|| panic!("missing JSON string {key}"));
        assert!(matches.next().is_none(), "duplicate JSON string {key}");
        let value_start = start + marker.len();
        let value_end = json[value_start..]
            .find('"')
            .map(|offset| value_start + offset)
            .unwrap_or_else(|| panic!("unterminated JSON string {key}"));
        &json[value_start..value_end]
    }

    fn unique_json_usize(json: &str, key: &str) -> usize {
        let marker = format!(r#""{key}": "#);
        let mut matches = json.match_indices(&marker);
        let (start, _) = matches
            .next()
            .unwrap_or_else(|| panic!("missing JSON integer {key}"));
        assert!(matches.next().is_none(), "duplicate JSON integer {key}");
        let value = &json[start + marker.len()..];
        let digits = value.bytes().take_while(u8::is_ascii_digit).count();
        assert!(digits > 0, "JSON field {key} is not an unsigned integer");
        value[..digits]
            .parse()
            .unwrap_or_else(|error| panic!("invalid JSON integer {key}: {error}"))
    }

    fn unique_json_bool(json: &str, key: &str) -> bool {
        let marker = format!(r#""{key}": "#);
        let mut matches = json.match_indices(&marker);
        let (start, _) = matches
            .next()
            .unwrap_or_else(|| panic!("missing JSON boolean {key}"));
        assert!(matches.next().is_none(), "duplicate JSON boolean {key}");
        let value = &json[start + marker.len()..];
        if value.starts_with("true") {
            true
        } else if value.starts_with("false") {
            false
        } else {
            panic!("JSON field {key} is not a boolean");
        }
    }

    fn unique_json_usize_pair(json: &str, key: &str) -> [usize; 2] {
        let marker = format!(r#""{key}": ["#);
        let mut matches = json.match_indices(&marker);
        let (start, _) = matches
            .next()
            .unwrap_or_else(|| panic!("missing JSON integer pair {key}"));
        assert!(
            matches.next().is_none(),
            "duplicate JSON integer pair {key}"
        );
        let value = &json[start + marker.len()..];
        let end = value
            .find(']')
            .unwrap_or_else(|| panic!("unterminated JSON integer pair {key}"));
        let values = value[..end]
            .split(',')
            .map(|part| {
                part.trim()
                    .parse::<usize>()
                    .unwrap_or_else(|error| panic!("invalid JSON integer pair {key}: {error}"))
            })
            .collect::<Vec<_>>();
        assert_eq!(values.len(), 2, "JSON field {key} is not an integer pair");
        [values[0], values[1]]
    }

    fn parse_json_hex_u32(name: &str, value: &str) -> u32 {
        let digits = value
            .strip_prefix("0x")
            .unwrap_or_else(|| panic!("JSON field {name} lacks a 0x prefix"));
        assert_eq!(digits.len(), 8, "JSON field {name} is not one u32");
        u32::from_str_radix(digits, 16)
            .unwrap_or_else(|error| panic!("invalid JSON hex field {name}: {error}"))
    }

    fn unique_json_hex_u32(json: &str, key: &str) -> u32 {
        parse_json_hex_u32(key, unique_json_string(json, key))
    }

    fn unique_json_hex_u32_array<const N: usize>(json: &str, key: &str) -> [u32; N] {
        let marker = format!(r#""{key}": ["#);
        let mut matches = json.match_indices(&marker);
        let (start, _) = matches
            .next()
            .unwrap_or_else(|| panic!("missing JSON hex array {key}"));
        assert!(matches.next().is_none(), "duplicate JSON hex array {key}");
        let value = &json[start + marker.len()..];
        let end = value
            .find(']')
            .unwrap_or_else(|| panic!("unterminated JSON hex array {key}"));
        let values = value[..end]
            .split(',')
            .map(|part| {
                let part = part.trim();
                let part = part
                    .strip_prefix('"')
                    .and_then(|part| part.strip_suffix('"'))
                    .unwrap_or_else(|| panic!("JSON hex array {key} has an unquoted member"));
                parse_json_hex_u32(key, part)
            })
            .collect::<Vec<_>>();
        assert_eq!(values.len(), N, "JSON field {key} has the wrong length");
        std::array::from_fn(|index| values[index])
    }

    fn unique_json_hex_u32_pair(json: &str, key: &str) -> [u32; 2] {
        unique_json_hex_u32_array(json, key)
    }

    fn lowercase_hex_prefix_bytes(
        name: &str,
        hex: &str,
        expected_chars: usize,
        prefix_chars: usize,
    ) -> Vec<u8> {
        lowercase_hex_prefix_bytes_with(name, hex, expected_chars, prefix_chars, |text| {
            u8::from_str_radix(text, 16)
                .unwrap_or_else(|error| panic!("invalid {name} hex byte: {error}"))
        })
    }

    fn lowercase_hex_prefix_bytes_with(
        name: &str,
        hex: &str,
        expected_chars: usize,
        prefix_chars: usize,
        mut decode: impl FnMut(&str) -> u8,
    ) -> Vec<u8> {
        assert_eq!(
            hex.len(),
            expected_chars,
            "{name} hex character count differs"
        );
        assert_eq!(prefix_chars % 2, 0, "{name} hex prefix is not byte-aligned");
        assert!(
            prefix_chars <= hex.len(),
            "{name} hex prefix exceeds the authenticated lexical payload"
        );
        assert!(
            hex.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "{name} must contain lowercase hexadecimal characters only"
        );
        hex.as_bytes()[..prefix_chars]
            .chunks_exact(2)
            .map(|pair| {
                let text = std::str::from_utf8(pair).unwrap();
                decode(text)
            })
            .collect()
    }

    fn sparse_snapshot_prefix(json: &str, key: &str) -> Vec<u32> {
        let plane = unique_json_object(json, key);
        assert_eq!(unique_json_usize(plane, "active_rows"), 88);
        assert_eq!(unique_json_usize(plane, "active_cols"), 3);
        assert_eq!(
            unique_json_string(plane, "active_layout"),
            "tight_contiguous_prefix"
        );
        assert_eq!(unique_json_usize(plane, "bytes"), 1_056);
        // The authenticated whole-evidence hash covers this declared value as
        // metadata. Do not decode the excluded suffix to recompute it.
        let declared_full_plane_sha = unique_json_string(plane, "sha256");
        let authenticated_metadata_sha = match key {
            "sparse_u" => "f0fccfc031d65004cae10171985b63725496a752d81cd3004ac9ba4628a19796",
            "sparse_v" => "ce38606d5df88780b190a880beff33494e24d2b6c5add938bee835f182e2ae44",
            _ => panic!("unexpected sparse snapshot plane {key}"),
        };
        assert_eq!(
            declared_full_plane_sha, authenticated_metadata_sha,
            "{key} declared full-plane SHA differs from evidence-authenticated metadata"
        );
        let bytes = lowercase_hex_prefix_bytes(key, unique_json_string(plane, "hex"), 2_112, 1_320);
        assert_eq!(bytes.len(), 660);
        bytes
            .chunks_exact(4)
            .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
            .collect()
    }

    #[test]
    fn excluded_sparse_suffix_is_lexically_checked_but_not_decoded() {
        let prefix = "01234567".repeat(165);
        let zero_suffix = "00".repeat(99 * 4);
        let ff_suffix = "ff".repeat(99 * 4);
        let zero_payload = format!("{prefix}{zero_suffix}");
        let ff_payload = format!("{prefix}{ff_suffix}");

        let zero_prefix = lowercase_hex_prefix_bytes("zero suffix", &zero_payload, 2_112, 1_320);
        let mut decoded_pairs = 0usize;
        let ff_prefix =
            lowercase_hex_prefix_bytes_with("ff suffix", &ff_payload, 2_112, 1_320, |pair| {
                decoded_pairs += 1;
                assert!(decoded_pairs <= 660, "excluded suffix reached the decoder");
                u8::from_str_radix(pair, 16).unwrap()
            });
        assert_eq!(zero_prefix.len(), 660);
        assert_eq!(zero_prefix, ff_prefix);
        assert_eq!(decoded_pairs, 660);

        let invalid_suffix = format!("{prefix}{}0G", "00".repeat(99 * 4 - 1));
        assert!(
            std::panic::catch_unwind(|| {
                lowercase_hex_prefix_bytes("invalid suffix", &invalid_suffix, 2_112, 1_320)
            })
            .is_err(),
            "the excluded suffix must still pass the full lexical contract"
        );
    }

    fn native_work_modes(bytes: &[u8], level: Level) -> Vec<CostMode> {
        let logical_bits = level.patch_rows();
        let expected_bytes = logical_bits.div_ceil(64) * 8;
        assert_eq!(
            bytes.len(),
            expected_bytes,
            "native {level} work-row bitset has wrong shape"
        );
        assert!(
            (logical_bits..bytes.len() * 8).all(|bit| bytes[bit / 8] & (1 << (bit % 8)) == 0),
            "native {level} work-row bitset has nonzero padding"
        );
        (0..logical_bits)
            .map(|bit| {
                if bytes[bit / 8] & (1 << (bit % 8)) != 0 {
                    CostMode::Weighted
                } else {
                    CostMode::Unweighted
                }
            })
            .collect()
    }

    fn f32_plane(name: &str, bytes: &[u8], expected_values: usize) -> Vec<f32> {
        assert_eq!(bytes.len(), expected_values * 4, "{name} has wrong shape");
        let values = bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
            .collect::<Vec<_>>();
        assert!(
            values.iter().all(|value| value.is_finite()),
            "{name} is non-finite"
        );
        values
    }

    fn solve_l2_with_modes(
        prepared: &LevelInputs,
        modes: Vec<CostMode>,
        initial: InitialGrid<AtoB>,
        hint: Option<&pis::HintGrid<AtoB>>,
        descents_per_pass: usize,
    ) -> PatchGrid<AtoB> {
        let input = ab_l2_input(prepared, modes);
        pis::solve_with_test_descents(&input, initial, hint, descents_per_pass)
            .expect("cold V9 L2 solve")
    }

    fn ab_l2_input(prepared: &LevelInputs, modes: Vec<CostMode>) -> Input<AtoB> {
        Input::<AtoB>::from_shared_native_order(
            Level::Two,
            LensPair {
                a: Arc::clone(&prepared.image.a),
                b: Arc::clone(&prepared.image.b),
            },
            LensPair {
                a: Arc::clone(&prepared.mask.a),
                b: Arc::clone(&prepared.mask.b),
            },
            Arc::clone(&prepared.gradient_col),
            Arc::clone(&prepared.gradient_row),
            Arc::clone(&prepared.raw_weight),
            modes,
        )
        .expect("authenticated V9 planes must form a valid L2 PIS input")
        .with_disparity_interval(selected_pis_interval(
            super::super::Direction::AtoB,
            Level::Two,
        ))
    }

    fn first_descent_trace_line(boundary: &str, trace: pis::FirstDescentTrace) -> String {
        format!(
            concat!(
                r#"{{"boundary":"{}","implementation":"kjerag_scalar_hypothesis","patch_row":{},"patch_col":{},"seed":["0x{:08x}","0x{:08x}"],"hessian":["0x{:08x}","0x{:08x}","0x{:08x}"],"inverse":["0x{:08x}","0x{:08x}","0x{:08x}"],"gradient_sums":["0x{:08x}","0x{:08x}"],"raw_s":"0x{:08x}","raw_q":"0x{:08x}","raw_rhs":["0x{:08x}","0x{:08x}"],"mean_correction":["0x{:08x}","0x{:08x}"],"corrected_rhs":["0x{:08x}","0x{:08x}"],"corrected_residual":"0x{:08x}","delta":["0x{:08x}","0x{:08x}"],"updated_flow":["0x{:08x}","0x{:08x}"],"survivors":{}}}"#,
            ),
            boundary,
            trace.patch_row,
            trace.patch_col,
            trace.seed[0],
            trace.seed[1],
            trace.hessian[0],
            trace.hessian[1],
            trace.hessian[2],
            trace.inverse[0],
            trace.inverse[1],
            trace.inverse[2],
            trace.gradient_sums[0],
            trace.gradient_sums[1],
            trace.raw_sum,
            trace.raw_sum_sq,
            trace.raw_rhs[0],
            trace.raw_rhs[1],
            trace.mean_correction[0],
            trace.mean_correction[1],
            trace.corrected_rhs[0],
            trace.corrected_rhs[1],
            trace.corrected_residual,
            trace.delta[0],
            trace.delta[1],
            trace.updated_flow[0],
            trace.updated_flow[1],
            trace.survivors,
        )
    }

    fn candidate_bits(
        candidate: Option<pis::CandidateEvaluation>,
    ) -> Option<([u32; 2], u32, usize)> {
        candidate.map(|candidate| {
            let flow = candidate.flow();
            (
                [flow.dcol().to_bits(), flow.drow().to_bits()],
                candidate.score().value().to_bits(),
                candidate.score().survivors(),
            )
        })
    }

    fn candidate_name(candidate: pis::Candidate) -> &'static str {
        match candidate {
            pis::Candidate::Current => "current",
            pis::Candidate::Hint => "hint",
            pis::Candidate::Horizontal => "horizontal",
            pis::Candidate::Vertical => "vertical",
        }
    }

    fn candidate_json(candidate: Option<pis::CandidateEvaluation>) -> String {
        match candidate_bits(candidate) {
            Some((flow, score, survivors)) => format!(
                r#"{{"flow":["0x{:08x}","0x{:08x}"],"score":"0x{score:08x}","survivors":{survivors}}}"#,
                flow[0], flow[1]
            ),
            None => "null".to_owned(),
        }
    }

    fn flow_components(grid: &PatchGrid<AtoB>) -> (Vec<f32>, Vec<f32>) {
        let dcol = grid
            .patches()
            .iter()
            .map(|patch| patch.flow().dcol())
            .collect::<Vec<_>>();
        let drow = grid
            .patches()
            .iter()
            .map(|patch| patch.flow().drow())
            .collect::<Vec<_>>();
        (dcol, drow)
    }

    fn report_native_hint_group(
        arm: &str,
        label: &str,
        solved: &PatchGrid<AtoB>,
        native_u: &[f32],
        native_v: &[f32],
        include: impl Fn(usize, usize) -> bool,
    ) {
        let mut patches = 0usize;
        let mut hint_better = 0usize;
        let mut hint_equal = 0usize;
        let mut hint_worse = 0usize;
        let mut winners = [0usize; 4];
        let mut guarded = [0usize; 2];
        let mut descents = [0usize; 2];
        let mut stops = [0usize; 2];
        let mut final_sse = 0.0f64;

        for (index, patch) in solved.patches().iter().enumerate() {
            let row = index / Level::Two.patch_cols();
            let col = index % Level::Two.patch_cols();
            if !include(row, col) {
                continue;
            }
            patches += 1;
            let reports = patch.passes();
            let pass_zero = reports[0];
            let candidates = pass_zero.candidates();
            let current = candidates[0].expect("native-hint arm must score current");
            let hint = candidates[1].expect("native-hint arm must score its hint");
            match hint.score().value().total_cmp(&current.score().value()) {
                std::cmp::Ordering::Less => hint_better += 1,
                std::cmp::Ordering::Equal => hint_equal += 1,
                std::cmp::Ordering::Greater => hint_worse += 1,
            }
            winners[match pass_zero.winner() {
                pis::Candidate::Current => 0,
                pis::Candidate::Hint => 1,
                pis::Candidate::Horizontal => 2,
                pis::Candidate::Vertical => 3,
            }] += 1;
            for pass in 0..2 {
                guarded[pass] += usize::from(reports[pass].guarded());
                descents[pass] += usize::from(reports[pass].descent_iterations());
                stops[pass] += usize::from(reports[pass].stopped_on_no_improvement());
            }
            let error_u = f64::from(patch.flow().dcol()) - f64::from(native_u[index]);
            let error_v = f64::from(patch.flow().drow()) - f64::from(native_v[index]);
            final_sse += error_u * error_u + error_v * error_v;
        }

        assert!(patches > 0, "native-hint report group {label} is empty");
        println!(
            "{arm} {label}: patches {patches}, pass-0 hint vs current better/equal/worse {hint_better}/{hint_equal}/{hint_worse}, winners current/hint/horizontal/vertical {}/{}/{}/{}, guards {}/{}, descents {}/{}, stops {}/{}, final vector SSE {final_sse:.9}",
            winners[0],
            winners[1],
            winners[2],
            winners[3],
            guarded[0],
            guarded[1],
            descents[0],
            descents[1],
            stops[0],
            stops[1],
        );
    }

    fn report_native_hint(
        arm: &str,
        solved: &PatchGrid<AtoB>,
        native_u: &[f32],
        native_v: &[f32],
        modes: &[CostMode],
    ) {
        report_native_hint_group(arm, "all", solved, native_u, native_v, |_, _| true);
        for (label, mode) in [
            ("unweighted", CostMode::Unweighted),
            ("weighted", CostMode::Weighted),
        ] {
            report_native_hint_group(arm, label, solved, native_u, native_v, |row, _| {
                modes[row] == mode
            });
        }
        for col in 0..Level::Two.patch_cols() {
            report_native_hint_group(
                arm,
                &format!("column {col}"),
                solved,
                native_u,
                native_v,
                |_, candidate_col| candidate_col == col,
            );
        }
    }

    fn report_native_seed(solved: &PatchGrid<AtoB>, native_u: &[f32], native_v: &[f32]) {
        let mut winners = [0usize; 4];
        let mut guarded = [0usize; 2];
        let mut descents = [0usize; 2];
        let mut stops = [0usize; 2];
        let mut unchanged = 0usize;
        for (index, patch) in solved.patches().iter().enumerate() {
            let reports = patch.passes();
            winners[match reports[0].winner() {
                pis::Candidate::Current => 0,
                pis::Candidate::Hint => 1,
                pis::Candidate::Horizontal => 2,
                pis::Candidate::Vertical => 3,
            }] += 1;
            for pass in 0..2 {
                guarded[pass] += usize::from(reports[pass].guarded());
                descents[pass] += usize::from(reports[pass].descent_iterations());
                stops[pass] += usize::from(reports[pass].stopped_on_no_improvement());
            }
            unchanged += usize::from(
                patch.flow().dcol().to_bits() == native_u[index].to_bits()
                    && patch.flow().drow().to_bits() == native_v[index].to_bits(),
            );
        }
        println!(
            "native-seed schedule: unchanged {unchanged}/{}, pass-0 winners current/hint/horizontal/vertical {}/{}/{}/{}, guards {}/{}, descents {}/{}, stops {}/{}",
            solved.patches().len(),
            winners[0],
            winners[1],
            winners[2],
            winners[3],
            guarded[0],
            guarded[1],
            descents[0],
            descents[1],
            stops[0],
            stops[1],
        );
    }

    fn report_component(label: &str, actual: &[f32], native: &[f32]) -> (f64, f64) {
        assert_eq!(
            actual.len(),
            native.len(),
            "{label} comparison shape differs"
        );
        assert!(
            actual.iter().all(|value| value.is_finite()),
            "Kjerag {label} is non-finite"
        );
        let mut absolute_sum = 0.0f64;
        let mut error_sse = 0.0f64;
        let mut baseline_sse = 0.0f64;
        let mut maximum = 0.0f64;
        let mut bit_exact = 0usize;
        for (&actual, &native) in actual.iter().zip(native) {
            bit_exact += usize::from(actual.to_bits() == native.to_bits());
            let error = f64::from(actual) - f64::from(native);
            absolute_sum += error.abs();
            error_sse += error * error;
            baseline_sse += f64::from(native) * f64::from(native);
            maximum = maximum.max(error.abs());
        }
        let count = actual.len() as f64;
        println!(
            "{label}: bit-exact {bit_exact}/{}, MAE {:.9}, RMS {:.9}, max {:.9}, SSE {:.9}, zero-baseline SSE {:.9}",
            actual.len(),
            absolute_sum / count,
            (error_sse / count).sqrt(),
            maximum,
            error_sse,
            baseline_sse,
        );
        (error_sse, baseline_sse)
    }

    fn report_vector(label: &str, u_sse: f64, v_sse: f64, zero_sse: f64) {
        let vector_sse = u_sse + v_sse;
        let vector_rms = (vector_sse / Level::Two.patches() as f64).sqrt();
        let improvement = (zero_sse - vector_sse) / zero_sse * 100.0;
        assert!(
            vector_sse.is_finite()
                && zero_sse.is_finite()
                && zero_sse > 0.0
                && vector_rms.is_finite()
                && improvement.is_finite(),
            "V9 L2 {label} comparison metrics are not finite"
        );
        println!(
            "{label} vector: SSE {vector_sse:.9}, RMS {vector_rms:.9}, zero-baseline SSE {zero_sse:.9}, improvement {improvement:.6}%"
        );
    }

    fn v9_spec(name: &str) -> (usize, &'static str) {
        V9_ARTIFACTS
            .iter()
            .find_map(|&(candidate, size, hash)| (candidate == name).then_some((size, hash)))
            .unwrap_or_else(|| panic!("missing test contract for {name}"))
    }

    fn read_v9(stage: &Path, name: &str) -> Vec<u8> {
        let (size, hash) = v9_spec(name);
        authenticated_v9_file(stage, name, size, hash)
    }

    fn f32_sha256(values: &[f32]) -> String {
        let mut digest = Sha256::new();
        for value in values {
            digest.update(value.to_le_bytes());
        }
        hex_digest(&digest.finalize())
    }

    fn f32_bits_sha256(values: &[u32]) -> String {
        let mut digest = Sha256::new();
        for value in values {
            digest.update(value.to_le_bytes());
        }
        hex_digest(&digest.finalize())
    }

    fn sparse_uv_f32le_sha256(dcol_bits: &[u32], drow_bits: &[u32]) -> String {
        let mut digest = Sha256::new();
        for plane in [dcol_bits, drow_bits] {
            for value in plane {
                digest.update(value.to_le_bytes());
            }
        }
        hex_digest(&digest.finalize())
    }

    fn report_work_modes(label: &str, level: Level, recomputed: &[CostMode], native: &[CostMode]) {
        assert_eq!(
            recomputed.len(),
            native.len(),
            "{label} {level} work-row shape differs"
        );
        let matches = recomputed
            .iter()
            .zip(native)
            .filter(|(left, right)| left == right)
            .count();
        let differences = recomputed
            .iter()
            .zip(native)
            .enumerate()
            .filter_map(|(row, (left, right))| (left != right).then_some(row))
            .collect::<Vec<_>>();
        let recomputed_weighted = recomputed
            .iter()
            .filter(|mode| matches!(mode, CostMode::Weighted))
            .count();
        let native_weighted = native
            .iter()
            .filter(|mode| matches!(mode, CostMode::Weighted))
            .count();
        println!(
            "{label} {level} work rows: {matches}/{} bit matches, weighted {recomputed_weighted} Kjerag/{native_weighted} native; differing rows {differences:?}",
            native.len(),
        );
        assert_eq!(
            recomputed, native,
            "{label} {level} reconstructed work rows differ from native"
        );
    }

    fn block_value_with_source_count(nonzero: usize, value: u8, target_mask_full: bool) -> u8 {
        assert!(nonzero <= PATCH_SIZE * PATCH_SIZE);
        let level = Level::One;
        let mut source = vec![0u8; level.pixels()];
        for index in 0..nonzero {
            let row = index / PATCH_SIZE;
            let col = index % PATCH_SIZE;
            source[row * level.cols() + col] = value;
        }
        let prepared = LevelInputs {
            image: LensPair {
                a: Arc::new(Vec::new()),
                b: Arc::new(Vec::new()),
            },
            mask: LensPair {
                a: Arc::new(source),
                b: Arc::new(vec![
                    if target_mask_full { u8::MAX } else { 0 };
                    level.pixels()
                ]),
            },
            gradient_col: Arc::new(Vec::new()),
            gradient_row: Arc::new(Vec::new()),
            raw_weight: Arc::new(Vec::new()),
        };
        prepared.small_disparity_block_mask(level)[0]
    }

    #[test]
    fn source_block_mask_counts_any_nonzero_and_keeps_exactly_seven_of_sixty_four() {
        assert_eq!(block_value_with_source_count(6, u8::MAX, true), 0);
        assert_eq!(block_value_with_source_count(7, u8::MAX, true), u8::MAX);
        assert_eq!(block_value_with_source_count(7, 1, false), u8::MAX);
        assert_eq!(
            block_value_with_source_count(0, 0, true),
            0,
            "physical target mask B must not enter native source-slot-A block generation",
        );
    }

    #[test]
    fn finest_weighted_runs_propagate_to_the_selected_coarse_rows() {
        let mut finest = vec![CostMode::Unweighted; Level::One.patch_rows()];
        for &(first, last) in &[
            (0, 15),
            (18, 27),
            (53, 60),
            (64, 64),
            (79, 79),
            (84, 95),
            (129, 177),
        ] {
            finest[first..=last].fill(CostMode::Weighted);
        }

        let coarse = propagate_work_modes(&finest);
        let mut expected = vec![CostMode::Unweighted; Level::Two.patch_rows()];
        for &(first, last) in &[(0, 13), (26, 32), (39, 39), (41, 47), (64, 87)] {
            expected[first..=last].fill(CostMode::Weighted);
        }

        assert_eq!(coarse, expected);
    }

    #[test]
    #[ignore = "requires KJERAG_V9_STAGE with the authenticated partial stage_v9 corpus"]
    fn partial_v9_cold_l2_oracle() {
        let stage = v9_stage();
        assert_eq!(
            stage.file_name().and_then(|name| name.to_str()),
            Some("stage_v9"),
            "KJERAG_V9_STAGE must point directly at stage_v9"
        );
        let _events = read_v9(&stage, "events.jsonl");
        let image_a = read_v9(&stage, "000_shared_postblur_A.bin");
        let image_b = read_v9(&stage, "001_shared_postblur_B.bin");
        let mask_a = read_v9(&stage, "002_shared_physical_mask_0.bin");
        let mask_b = read_v9(&stage, "003_shared_physical_mask_1.bin");
        let native_ab_l1_modes = native_work_modes(
            &read_v9(&stage, "007_ab_retained_rows_d8_1.bin"),
            Level::One,
        );
        let native_ab_l2_modes = native_work_modes(
            &read_v9(&stage, "008_ab_retained_rows_d8_2.bin"),
            Level::Two,
        );
        let native_ba_l1_modes = native_work_modes(
            &read_v9(&stage, "010_ba_retained_rows_d8_1.bin"),
            Level::One,
        );
        let native_ba_l2_modes = native_work_modes(
            &read_v9(&stage, "011_ba_retained_rows_d8_2.bin"),
            Level::Two,
        );

        // Despite their historical filenames, these owner-level payloads were
        // captured immediately before OpticalFlow::calc applies its input blur.
        let retained = ColdInputs::from_before_input_blur(
            LensPair {
                a: image_a,
                b: image_b,
            },
            LensPair {
                a: mask_a,
                b: mask_b,
            },
        )
        .expect("authenticated V9 images and masks must have retained-grid shape");
        let masks = MaskPyramid::build(&retained);
        let prepared_ab_l1 = LevelInputs::build::<AtoB>(&retained, &masks, Level::One);
        let prepared_ba_l1 = LevelInputs::build::<BtoA>(&retained, &masks, Level::One);
        let prepared_ab_l2 = LevelInputs::build::<AtoB>(&retained, &masks, Level::Two);
        println!(
            "prepared AB L2 image A SHA-256 {}",
            sha256_hex(&prepared_ab_l2.image.a)
        );
        println!(
            "prepared AB L2 image B SHA-256 {}",
            sha256_hex(&prepared_ab_l2.image.b)
        );
        println!(
            "prepared AB L2 mask A SHA-256 {}",
            sha256_hex(&prepared_ab_l2.mask.a)
        );
        println!(
            "prepared AB L2 mask B SHA-256 {}",
            sha256_hex(&prepared_ab_l2.mask.b)
        );
        println!(
            "prepared AB L2 gradient col f32le SHA-256 {}",
            f32_sha256(&prepared_ab_l2.gradient_col)
        );
        println!(
            "prepared AB L2 gradient row f32le SHA-256 {}",
            f32_sha256(&prepared_ab_l2.gradient_row)
        );
        println!(
            "prepared AB L2 raw weight f32le SHA-256 {}",
            f32_sha256(&prepared_ab_l2.raw_weight)
        );
        let recomputed_ab = WorkRows::<AtoB>::from_finest(&prepared_ab_l1);
        let recomputed_ba = WorkRows::<BtoA>::from_finest(&prepared_ba_l1);
        report_work_modes(
            "A-to-B",
            Level::One,
            &recomputed_ab.level_one,
            &native_ab_l1_modes,
        );
        report_work_modes(
            "B-to-A",
            Level::One,
            &recomputed_ba.level_one,
            &native_ba_l1_modes,
        );
        report_work_modes(
            "A-to-B",
            Level::Two,
            &recomputed_ab.level_two,
            &native_ab_l2_modes,
        );
        report_work_modes(
            "B-to-A",
            Level::Two,
            &recomputed_ba.level_two,
            &native_ba_l2_modes,
        );
        for name in [
            "012_ab_l2_pre_pis_main_u.bin",
            "013_ab_l2_pre_pis_main_v.bin",
            "014_ab_l2_pre_pis_aux_hint_u.bin",
            "015_ab_l2_pre_pis_aux_hint_v.bin",
            "016_ba_l2_pre_pis_main_u.bin",
            "017_ba_l2_pre_pis_main_v.bin",
            "018_ba_l2_pre_pis_aux_hint_u.bin",
            "019_ba_l2_pre_pis_aux_hint_v.bin",
        ] {
            let values = f32_plane(name, &read_v9(&stage, name), Level::Two.pixels());
            assert!(
                values.iter().all(|value| value.to_bits() == 0),
                "{name} is not the exact zero f32 plane"
            );
        }

        let native_u_bytes = read_v9(&stage, "020_ab_l2_post_pis_sparse_u.bin");
        let native_v_bytes = read_v9(&stage, "021_ab_l2_post_pis_sparse_v.bin");
        let native_u = f32_plane("native A-to-B L2 U", &native_u_bytes, Level::Two.patches());
        let native_v = f32_plane("native A-to-B L2 V", &native_v_bytes, Level::Two.patches());

        // The exact captured A-to-B work modes replace Kjerag's recomputation here. This
        // deliberately isolates downstream numerical preprocessing and PIS from row membership.
        let solved = solve_l2_with_modes(
            &prepared_ab_l2,
            native_ab_l2_modes.clone(),
            InitialGrid::coarse_zeros(),
            None,
            pis::DESCENTS_PER_PASS,
        );
        let trace_input = ab_l2_input(&prepared_ab_l2, native_ab_l2_modes.clone());
        let (_trace_solved, first_descent) = pis::solve_with_test_first_descent(
            &trace_input,
            InitialGrid::coarse_zeros(),
            None,
            pis::DESCENTS_PER_PASS,
            14,
            2,
        )
        .expect("authenticated V9 AB L2 first-descent trace");
        assert_eq!((first_descent.patch_row, first_descent.patch_col), (14, 2));
        // Studio 6.0.2 worker SHA-256
        // 0a34f593198a0a7a29239ccded91231af3d7bc94e04213c67a2d44a04076a452
        // captured these exact boundaries on the same authenticated AB L2
        // inputs. The native receipt deliberately leaves the candidate origin
        // open, but closes the selected zero seed and every value below.
        assert_eq!(first_descent.seed, [0x0000_0000, 0x0000_0000]);
        assert_eq!(
            first_descent.inverse,
            [0x373b_3213, 0xb761_795d, 0x378e_63fd]
        );
        assert_eq!(first_descent.gradient_sums, [0xc5c6_3000, 0xc5ad_8000]);
        assert_eq!(first_descent.raw_sum, 0xc375_0000);
        assert_eq!(first_descent.raw_sum_sq, 0x44b6_a000);
        assert_eq!(first_descent.raw_rhs, [0x467e_4c00, 0x4688_5e00]);
        assert_eq!(first_descent.corrected_rhs, [0xc63b_1ee8, 0xc5e3_6518]);
        assert_eq!(first_descent.corrected_residual, 0x43b8_d174);
        assert_eq!(first_descent.delta, [0xbd12_c168, 0x3d19_4fee]);
        assert_eq!(first_descent.updated_flow, [0x3d12_c168, 0xbd19_4fee]);
        assert_eq!(first_descent.survivors, 55);
        println!(
            "{}",
            first_descent_trace_line("ab_l2_pass0_first_descent", first_descent)
        );
        let (kjerag_u, kjerag_v) = flow_components(&solved);
        assert_eq!(kjerag_u.len(), Level::Two.patches());
        assert_eq!(kjerag_v.len(), Level::Two.patches());

        let (u_sse, u_zero_sse) = report_component("baseline U", &kjerag_u, &native_u);
        let (v_sse, v_zero_sse) = report_component("baseline V", &kjerag_v, &native_v);
        let vector_zero_sse = u_zero_sse + v_zero_sse;
        report_vector("baseline", u_sse, v_sse, vector_zero_sse);
        println!("baseline U f32le SHA-256 {}", f32_sha256(&kjerag_u));
        println!("baseline V f32le SHA-256 {}", f32_sha256(&kjerag_v));

        // Change one input only: offer Studio's captured final sparse field as
        // the optional candidate grid while retaining the same zero initial
        // grid, images, masks, work modes, interval, and 6+6 schedule.
        let native_flows = native_u
            .iter()
            .copied()
            .zip(native_v.iter().copied())
            .map(|(dcol, drow)| {
                pis::Flow::new(dcol, drow).expect("authenticated native flow is finite")
            })
            .collect::<Vec<_>>();
        let native_hint = pis::HintGrid::<AtoB>::from_row_major(Level::Two, native_flows.clone())
            .expect("authenticated native hint has the selected L2 patch shape");
        let hinted = solve_l2_with_modes(
            &prepared_ab_l2,
            native_ab_l2_modes.clone(),
            InitialGrid::coarse_zeros(),
            Some(&native_hint),
            pis::DESCENTS_PER_PASS,
        );
        let (hinted_u, hinted_v) = flow_components(&hinted);
        let (hinted_u_sse, _) = report_component("native-hint U", &hinted_u, &native_u);
        let (hinted_v_sse, _) = report_component("native-hint V", &hinted_v, &native_v);
        report_vector("native-hint", hinted_u_sse, hinted_v_sse, vector_zero_sse);
        println!("native-hint U f32le SHA-256 {}", f32_sha256(&hinted_u));
        println!("native-hint V f32le SHA-256 {}", f32_sha256(&hinted_v));
        report_native_hint(
            "native-hint",
            &hinted,
            &native_u,
            &native_v,
            &native_ab_l2_modes,
        );

        // Remove only Gauss-Newton descent. Both candidate passes, in-place
        // propagation, scoring, the zero initial grid, and the native hint stay
        // fixed, separating candidate competition from descent arithmetic.
        let no_descent = solve_l2_with_modes(
            &prepared_ab_l2,
            native_ab_l2_modes.clone(),
            InitialGrid::coarse_zeros(),
            Some(&native_hint),
            0,
        );
        let (no_descent_u, no_descent_v) = flow_components(&no_descent);
        let (no_descent_u_sse, _) =
            report_component("native-hint-no-descent U", &no_descent_u, &native_u);
        let (no_descent_v_sse, _) =
            report_component("native-hint-no-descent V", &no_descent_v, &native_v);
        report_vector(
            "native-hint-no-descent",
            no_descent_u_sse,
            no_descent_v_sse,
            vector_zero_sse,
        );
        println!(
            "native-hint-no-descent U f32le SHA-256 {}",
            f32_sha256(&no_descent_u)
        );
        println!(
            "native-hint-no-descent V f32le SHA-256 {}",
            f32_sha256(&no_descent_v)
        );
        report_native_hint(
            "native-hint-no-descent",
            &no_descent,
            &native_u,
            &native_v,
            &native_ab_l2_modes,
        );

        // Admit exactly the first descent in each pass. This keeps every
        // candidate and propagation decision while locating whether the
        // divergence begins on the first native-shaped Gauss-Newton step or
        // only after the iterative trajectory feeds back into itself.
        let one_descent = solve_l2_with_modes(
            &prepared_ab_l2,
            native_ab_l2_modes.clone(),
            InitialGrid::coarse_zeros(),
            Some(&native_hint),
            1,
        );
        let (one_descent_u, one_descent_v) = flow_components(&one_descent);
        let (one_descent_u_sse, _) =
            report_component("native-hint-one-descent U", &one_descent_u, &native_u);
        let (one_descent_v_sse, _) =
            report_component("native-hint-one-descent V", &one_descent_v, &native_v);
        report_vector(
            "native-hint-one-descent",
            one_descent_u_sse,
            one_descent_v_sse,
            vector_zero_sse,
        );
        println!(
            "native-hint-one-descent U f32le SHA-256 {}",
            f32_sha256(&one_descent_u)
        );
        println!(
            "native-hint-one-descent V f32le SHA-256 {}",
            f32_sha256(&one_descent_v)
        );
        report_native_hint(
            "native-hint-one-descent",
            &one_descent,
            &native_u,
            &native_v,
            &native_ab_l2_modes,
        );

        // The hint arm is ambiguous when Studio's candidate scores well but
        // spatial propagation still wins. Replace only the initial zero grid
        // with Studio's field to test whether that field is stationary under
        // the reconstructed objective and descent.
        let native_seed = InitialGrid::<AtoB>::coarse_from_row_major(native_flows)
            .expect("authenticated native seed has the selected L2 patch shape");
        let seeded = solve_l2_with_modes(
            &prepared_ab_l2,
            native_ab_l2_modes,
            native_seed,
            None,
            pis::DESCENTS_PER_PASS,
        );
        let (seeded_u, seeded_v) = flow_components(&seeded);
        let (seeded_u_sse, _) = report_component("native-seed U", &seeded_u, &native_u);
        let (seeded_v_sse, _) = report_component("native-seed V", &seeded_v, &native_v);
        report_vector("native-seed", seeded_u_sse, seeded_v_sse, vector_zero_sse);
        println!("native-seed U f32le SHA-256 {}", f32_sha256(&seeded_u));
        println!("native-seed V f32le SHA-256 {}", f32_sha256(&seeded_v));
        report_native_seed(&seeded, &native_u, &native_v);
    }

    #[test]
    #[ignore = "requires authenticated V9 stage and row-54 candidate evidence"]
    fn partial_v9_ab_l2_post_current_initialized_prefix_is_bit_exact() {
        const PATCH_ROW: usize = 54;
        const PATCH_COL: usize = 2;
        const VALID_PREFIX_CELLS: usize = 165;
        const VALID_PREFIX_BYTES: usize = VALID_PREFIX_CELLS * 4;

        let stage = v9_stage();
        assert_eq!(
            stage.file_name().and_then(|name| name.to_str()),
            Some("stage_v9"),
            "KJERAG_V9_STAGE must point directly at stage_v9"
        );
        for &(name, size, hash) in &V9_ARTIFACTS {
            let _ = authenticated_v9_file(&stage, name, size, hash);
        }
        let retained = ColdInputs::from_before_input_blur(
            LensPair {
                a: read_v9(&stage, "000_shared_postblur_A.bin"),
                b: read_v9(&stage, "001_shared_postblur_B.bin"),
            },
            LensPair {
                a: read_v9(&stage, "002_shared_physical_mask_0.bin"),
                b: read_v9(&stage, "003_shared_physical_mask_1.bin"),
            },
        )
        .expect("authenticated V9 images and masks must have retained-grid shape");
        let masks = MaskPyramid::build(&retained);
        let prepared = LevelInputs::build::<AtoB>(&retained, &masks, Level::Two);
        let modes = native_work_modes(
            &read_v9(&stage, "008_ab_retained_rows_d8_2.bin"),
            Level::Two,
        );
        let input = ab_l2_input(&prepared, modes);
        let (_, _, post_current_grid) = pis::solve_with_test_patch_boundary(
            &input,
            InitialGrid::coarse_zeros(),
            None,
            pis::DESCENTS_PER_PASS,
            PATCH_ROW,
            PATCH_COL,
        )
        .expect("authenticated V9 AB L2 row-54 post-current-score trace");
        assert_eq!(
            (post_current_grid.patch_row, post_current_grid.patch_col),
            (PATCH_ROW, PATCH_COL)
        );
        assert_eq!(post_current_grid.level, Level::Two);
        assert_eq!(
            (post_current_grid.patch_rows, post_current_grid.patch_cols),
            (88, 3)
        );
        assert_eq!(post_current_grid.dcol_bits.len(), 264);
        assert_eq!(post_current_grid.drow_bits.len(), 264);

        let evidence_path = std::env::var_os("KJERAG_UNWEIGHTED_CANDIDATE_EVIDENCE")
            .map(PathBuf::from)
            .expect(
                "set KJERAG_UNWEIGHTED_CANDIDATE_EVIDENCE to the authenticated candidate-boundary evidence JSON",
            );
        let evidence_bytes = std::fs::read(&evidence_path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", evidence_path.display()));
        assert_eq!(evidence_bytes.len(), 17_608, "evidence byte count differs");
        assert_eq!(
            sha256_hex(&evidence_bytes),
            "69ffba28ea42dd57d5e67a39e29be2c73d34b31adc3268807c8d8287bc5189ce",
            "evidence SHA-256 differs"
        );
        let evidence = std::str::from_utf8(&evidence_bytes)
            .expect("authenticated candidate-boundary evidence must be UTF-8 JSON");
        assert_eq!(
            unique_json_string(evidence, "schema"),
            "kjerag.v9-unweighted-candidate-boundary-capture.v1"
        );
        let sparse = unique_json_object(evidence, "sparse_state_at_current_stop");
        let studio_u = sparse_snapshot_prefix(sparse, "sparse_u");
        let studio_v = sparse_snapshot_prefix(sparse, "sparse_v");
        assert_eq!(studio_u.len(), VALID_PREFIX_CELLS);
        assert_eq!(studio_v.len(), VALID_PREFIX_CELLS);

        let distilled_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/research/studio-candidate-row54-unweighted-602.json");
        let distilled = std::fs::read_to_string(&distilled_path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", distilled_path.display()));
        assert_eq!(
            unique_json_string(&distilled, "schema"),
            "kjerag.studio-unweighted-candidate-boundary-oracle.v1"
        );
        let valid_prefix = unique_json_object(&distilled, "valid_sparse_snapshot_prefix");
        let excluded_marker = r#""excluded_suffix": {"#;
        let excluded_start = valid_prefix
            .find(excluded_marker)
            .expect("valid sparse prefix must name its excluded suffix");
        assert_eq!(
            unique_json_usize_pair(&valid_prefix[..excluded_start], "inclusive_indices"),
            [0, 164]
        );
        assert_eq!(
            unique_json_usize(valid_prefix, "f32_values_per_plane"),
            VALID_PREFIX_CELLS
        );
        assert_eq!(
            unique_json_usize(valid_prefix, "bytes_per_plane"),
            VALID_PREFIX_BYTES
        );
        let studio_hashes = unique_json_object(valid_prefix, "studio_sha256");
        assert_eq!(
            unique_json_string(studio_hashes, "u_f32le"),
            "15398af08dbcdb5889f7455a10558de446b836f19dd8198bacdd3414d2fc8675"
        );
        assert_eq!(
            unique_json_string(studio_hashes, "v_f32le"),
            "56b443a2e97873c3980bcb569eb114ff2966645d2a17c27c163d443664052040"
        );
        assert_eq!(
            unique_json_string(studio_hashes, "u_then_v_f32le"),
            "5b0daba50f9baadb73afbf9a22c6d5a805759b279af07657428785f0c52c23a9"
        );
        let excluded_suffix = unique_json_object(valid_prefix, "excluded_suffix");
        assert_eq!(
            unique_json_usize_pair(excluded_suffix, "inclusive_indices"),
            [165, 263]
        );

        let studio_u_prefix = &studio_u[..VALID_PREFIX_CELLS];
        let studio_v_prefix = &studio_v[..VALID_PREFIX_CELLS];
        assert_eq!(
            f32_bits_sha256(studio_u_prefix),
            unique_json_string(studio_hashes, "u_f32le")
        );
        assert_eq!(
            f32_bits_sha256(studio_v_prefix),
            unique_json_string(studio_hashes, "v_f32le")
        );
        assert_eq!(
            sparse_uv_f32le_sha256(studio_u_prefix, studio_v_prefix),
            unique_json_string(studio_hashes, "u_then_v_f32le")
        );

        const CLOSED_ROW6_INDEX: usize = 20;
        assert_eq!(
            [
                post_current_grid.dcol_bits[CLOSED_ROW6_INDEX],
                post_current_grid.drow_bits[CLOSED_ROW6_INDEX],
            ],
            [studio_u[CLOSED_ROW6_INDEX], studio_v[CLOSED_ROW6_INDEX]],
            "the accepted row-6 disparity SKIP regressed"
        );

        let rust_u_prefix = &post_current_grid.dcol_bits[..VALID_PREFIX_CELLS];
        let rust_v_prefix = &post_current_grid.drow_bits[..VALID_PREFIX_CELLS];
        assert_eq!(rust_u_prefix, studio_u_prefix);
        assert_eq!(rust_v_prefix, studio_v_prefix);
        assert_eq!(
            f32_bits_sha256(rust_u_prefix),
            unique_json_string(studio_hashes, "u_f32le")
        );
        assert_eq!(
            f32_bits_sha256(rust_v_prefix),
            unique_json_string(studio_hashes, "v_f32le")
        );
        assert_eq!(
            sparse_uv_f32le_sha256(rust_u_prefix, rust_v_prefix),
            unique_json_string(studio_hashes, "u_then_v_f32le")
        );
        println!(
            "all {VALID_PREFIX_CELLS} initialized Studio cells match Kjerag: U {}, V {}, U-then-V {}",
            f32_bits_sha256(rust_u_prefix),
            f32_bits_sha256(rust_v_prefix),
            sparse_uv_f32le_sha256(rust_u_prefix, rust_v_prefix),
        );
    }

    #[test]
    #[ignore = "requires authenticated V9 stage plus row-54 and delayed-sealed row-36 evidence"]
    fn partial_v9_ab_l2_row36_zero_hint_descent_diagnostic() {
        const PATCH_ROW: usize = 36;
        const PATCH_COL: usize = 0;
        const PREFIX_TRACE_ROW: usize = 54;
        const PREFIX_TRACE_COL: usize = 2;
        const VALID_PREFIX_CELLS: usize = 165;

        let stage = v9_stage();
        assert_eq!(
            stage.file_name().and_then(|name| name.to_str()),
            Some("stage_v9"),
            "KJERAG_V9_STAGE must point directly at stage_v9"
        );
        for &(name, size, hash) in &V9_ARTIFACTS {
            let _ = authenticated_v9_file(&stage, name, size, hash);
        }

        let evidence_path = std::env::var_os("KJERAG_UNWEIGHTED_CANDIDATE_EVIDENCE")
            .map(PathBuf::from)
            .expect(
                "set KJERAG_UNWEIGHTED_CANDIDATE_EVIDENCE to the authenticated candidate-boundary evidence JSON",
            );
        let evidence_bytes = std::fs::read(&evidence_path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", evidence_path.display()));
        assert_eq!(evidence_bytes.len(), 17_608, "evidence byte count differs");
        assert_eq!(
            sha256_hex(&evidence_bytes),
            "69ffba28ea42dd57d5e67a39e29be2c73d34b31adc3268807c8d8287bc5189ce",
            "evidence SHA-256 differs"
        );
        let evidence = std::str::from_utf8(&evidence_bytes)
            .expect("authenticated candidate-boundary evidence must be UTF-8 JSON");
        assert_eq!(
            unique_json_string(evidence, "schema"),
            "kjerag.v9-unweighted-candidate-boundary-capture.v1"
        );
        let hint_evidence = unique_json_object(evidence, "hint");
        assert!(unique_json_bool(hint_evidence, "candidate_present"));
        assert_eq!(unique_json_usize(hint_evidence, "fds_0x9c"), 0);
        assert_eq!(unique_json_usize(hint_evidence, "stack_skip_u32"), 0);
        let input_planes = unique_json_object(evidence, "input_planes");
        for (key, name) in [
            ("aux_hint_u", "014_ab_l2_pre_pis_aux_hint_u.bin"),
            ("aux_hint_v", "015_ab_l2_pre_pis_aux_hint_v.bin"),
        ] {
            let plane_evidence = unique_json_object(input_planes, key);
            assert_eq!(unique_json_usize(plane_evidence, "rows"), 270);
            assert_eq!(unique_json_usize(plane_evidence, "cols"), 15);
            assert_eq!(unique_json_usize(plane_evidence, "row_bytes"), 60);
            assert_eq!(
                unique_json_string(plane_evidence, "sha256"),
                "9dd76d9311e87123c936ab0d956e9ead34dae3fb68beb075b6ee6fee9b08d9e0"
            );
            let values = f32_plane(name, &read_v9(&stage, name), Level::Two.pixels());
            assert!(
                values.iter().all(|value| value.to_bits() == 0),
                "{name} is not the authenticated positive-zero f32 plane"
            );
        }
        let sparse = unique_json_object(evidence, "sparse_state_at_current_stop");
        let studio_u = sparse_snapshot_prefix(sparse, "sparse_u");
        let studio_v = sparse_snapshot_prefix(sparse, "sparse_v");
        assert_eq!(studio_u.len(), VALID_PREFIX_CELLS);
        assert_eq!(studio_v.len(), VALID_PREFIX_CELLS);

        let row36_evidence_path = std::env::var_os("KJERAG_ROW36_DESCENT_LOOP_EVIDENCE")
            .map(PathBuf::from)
            .expect(
                "set KJERAG_ROW36_DESCENT_LOOP_EVIDENCE to the delayed-sealed row-36 evidence JSON",
            );
        let row36_evidence_bytes = std::fs::read(&row36_evidence_path).unwrap_or_else(|error| {
            panic!("could not read {}: {error}", row36_evidence_path.display())
        });
        assert_eq!(
            row36_evidence_bytes.len(),
            92_848,
            "row-36 evidence byte count differs"
        );
        assert_eq!(
            sha256_hex(&row36_evidence_bytes),
            "32ec691a2d42324bb15b3431a2fb6b3d203fb71a996c79e01df06b6968eb83bd",
            "row-36 evidence SHA-256 differs"
        );
        let row36_evidence = std::str::from_utf8(&row36_evidence_bytes)
            .expect("delayed-sealed row-36 evidence must be UTF-8 JSON");
        assert_eq!(
            unique_json_string(row36_evidence, "schema"),
            "kjerag.v9-row36-descent-loop-capture.v1"
        );

        let row36_oracle_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/research/studio-row36-descent-loop-602.json");
        let row36_oracle_bytes = std::fs::read(&row36_oracle_path).unwrap_or_else(|error| {
            panic!("could not read {}: {error}", row36_oracle_path.display())
        });
        assert_eq!(
            row36_oracle_bytes.len(),
            12_142,
            "row-36 oracle byte count differs"
        );
        assert_eq!(
            sha256_hex(&row36_oracle_bytes),
            "414416035a97e2fdd05ff5ee9b9a72eda0f7106604e46c1e6373594297039169",
            "row-36 oracle SHA-256 differs"
        );
        let row36_oracle = std::str::from_utf8(&row36_oracle_bytes)
            .expect("tracked row-36 oracle must be UTF-8 JSON");
        assert_eq!(
            unique_json_string(row36_oracle, "schema"),
            "kjerag.studio-row36-descent-loop-delayed-oracle.v1"
        );
        let row36_provenance = unique_json_object(row36_oracle, "capture_provenance");
        assert_eq!(
            unique_json_string(row36_provenance, "evidence_sha256"),
            sha256_hex(&row36_evidence_bytes)
        );
        assert_eq!(
            unique_json_string(row36_provenance, "verification_verdict"),
            "accepted_delayed_seal"
        );
        let delayed_adoption = unique_json_object(row36_oracle, "delayed_adoption");
        assert!(unique_json_bool(
            delayed_adoption,
            "not_original_launch_receipt"
        ));
        assert!(!unique_json_bool(
            delayed_adoption,
            "ordinary_postflight_passed"
        ));
        assert!(!unique_json_bool(
            delayed_adoption,
            "fresh_postflight_passed"
        ));
        assert!(!unique_json_bool(
            delayed_adoption,
            "historical_adversarial_immutability_claimed"
        ));

        let retained = ColdInputs::from_before_input_blur(
            LensPair {
                a: read_v9(&stage, "000_shared_postblur_A.bin"),
                b: read_v9(&stage, "001_shared_postblur_B.bin"),
            },
            LensPair {
                a: read_v9(&stage, "002_shared_physical_mask_0.bin"),
                b: read_v9(&stage, "003_shared_physical_mask_1.bin"),
            },
        )
        .expect("authenticated V9 images and masks must have retained-grid shape");
        let masks = MaskPyramid::build(&retained);
        let prepared = LevelInputs::build::<AtoB>(&retained, &masks, Level::Two);
        let modes = native_work_modes(
            &read_v9(&stage, "008_ab_retained_rows_d8_2.bin"),
            Level::Two,
        );
        let input = ab_l2_input(&prepared, modes.clone());

        // Studio's authenticated call carries a present, complete HintGrid
        // whose component planes are both byte-zero. Preserve the presence of
        // that candidate instead of collapsing it to `None`.
        let zero_hint = pis::HintGrid::<AtoB>::from_row_major(
            Level::Two,
            vec![pis::Flow::ZERO; Level::Two.patches()],
        )
        .expect("zero hint has the authenticated L2 patch shape");
        let (_, _, post_current_grid) = pis::solve_with_test_patch_boundary(
            &input,
            InitialGrid::coarse_zeros(),
            Some(&zero_hint),
            pis::DESCENTS_PER_PASS,
            PREFIX_TRACE_ROW,
            PREFIX_TRACE_COL,
        )
        .expect("authenticated V9 AB L2 present-zero-hint prefix trace");
        assert_eq!(
            (post_current_grid.patch_row, post_current_grid.patch_col),
            (PREFIX_TRACE_ROW, PREFIX_TRACE_COL)
        );
        assert_eq!(post_current_grid.level, Level::Two);
        assert_eq!(
            (post_current_grid.patch_rows, post_current_grid.patch_cols),
            (88, 3)
        );
        assert_eq!(post_current_grid.dcol_bits.len(), 264);
        assert_eq!(post_current_grid.drow_bits.len(), 264);

        let distilled_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/research/studio-candidate-row54-unweighted-602.json");
        let distilled = std::fs::read_to_string(&distilled_path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", distilled_path.display()));
        assert_eq!(
            unique_json_string(&distilled, "schema"),
            "kjerag.studio-unweighted-candidate-boundary-oracle.v1"
        );
        let valid_prefix = unique_json_object(&distilled, "valid_sparse_snapshot_prefix");
        let excluded_marker = r#""excluded_suffix": {"#;
        let excluded_start = valid_prefix
            .find(excluded_marker)
            .expect("valid sparse prefix must name its excluded suffix");
        assert_eq!(
            unique_json_usize_pair(&valid_prefix[..excluded_start], "inclusive_indices"),
            [0, VALID_PREFIX_CELLS - 1]
        );
        assert_eq!(
            unique_json_usize(valid_prefix, "f32_values_per_plane"),
            VALID_PREFIX_CELLS
        );
        let studio_hashes = unique_json_object(valid_prefix, "studio_sha256");
        assert_eq!(
            f32_bits_sha256(&studio_u),
            unique_json_string(studio_hashes, "u_f32le")
        );
        assert_eq!(
            f32_bits_sha256(&studio_v),
            unique_json_string(studio_hashes, "v_f32le")
        );
        assert_eq!(
            sparse_uv_f32le_sha256(&studio_u, &studio_v),
            unique_json_string(studio_hashes, "u_then_v_f32le")
        );
        let excluded_suffix = unique_json_object(valid_prefix, "excluded_suffix");
        assert_eq!(
            unique_json_usize_pair(excluded_suffix, "inclusive_indices"),
            [VALID_PREFIX_CELLS, 263]
        );

        let mismatch = (0..VALID_PREFIX_CELLS).find(|&index| {
            post_current_grid.dcol_bits[index] != studio_u[index]
                || post_current_grid.drow_bits[index] != studio_v[index]
        });
        assert_eq!(
            mismatch, None,
            "an initialized Studio prefix cell still differs from Kjerag"
        );
        let patch_index = PATCH_ROW * Level::Two.patch_cols() + PATCH_COL;
        assert_eq!(
            [studio_u[patch_index], studio_v[patch_index]],
            [0xbe55_fe44, 0x3dc1_b763]
        );
        let vertical_index = patch_index - Level::Two.patch_cols();
        assert_eq!(
            [studio_u[vertical_index], studio_v[vertical_index]],
            [0x0000_0000, 0x0000_0000],
            "Studio's row-35 vertical neighbour is not zero"
        );
        assert_eq!(
            [
                post_current_grid.dcol_bits[patch_index],
                post_current_grid.drow_bits[patch_index],
            ],
            [0xbe55_fe44, 0x3dc1_b763]
        );

        let (solved, descents) = pis::solve_with_test_descent_sequence(
            &input,
            InitialGrid::coarse_zeros(),
            Some(&zero_hint),
            pis::DESCENTS_PER_PASS,
            PATCH_ROW,
            PATCH_COL,
        )
        .expect("authenticated V9 AB L2 row-36 descent trace");
        let patch = solved.patches()[patch_index];
        let pass = patch.passes()[0];
        let candidates = pass.candidates();

        let captured_structure = unique_json_object(row36_oracle, "captured_structure");
        let captured_hessian = unique_json_object(captured_structure, "hessian");
        let captured_inverse = unique_json_object(captured_structure, "inverse_labels");
        let captured_gradient_sums = unique_json_object(captured_structure, "gradient_sums");
        let expected_hessian = [
            unique_json_hex_u32(captured_hessian, "col_col"),
            unique_json_hex_u32(captured_hessian, "col_row"),
            unique_json_hex_u32(captured_hessian, "row_row"),
        ];
        let expected_inverse = [
            unique_json_hex_u32(captured_inverse, "inv11"),
            unique_json_hex_u32(captured_inverse, "inv13"),
            unique_json_hex_u32(captured_inverse, "inv10"),
        ];
        let expected_gradient_sums = [
            unique_json_hex_u32(captured_gradient_sums, "col"),
            unique_json_hex_u32(captured_gradient_sums, "row"),
        ];
        let captured_attempts =
            json_object_elements(unique_json_array(row36_oracle, "captured_attempts"));
        assert_eq!(captured_attempts.len(), descents.len());
        let captured_seed = unique_json_object(row36_oracle, "captured_seed");
        let expected_seed = unique_json_hex_u32_pair(captured_seed, "flow");
        let mut previous_residual = unique_json_hex_u32(captured_seed, "initial_previous_residual");
        let mut accepted_count = 0;
        for (index, (captured, descent)) in
            captured_attempts.iter().zip(descents.iter()).enumerate()
        {
            assert_eq!(unique_json_usize(captured, "ordinal"), index + 1);
            assert_eq!(descent.seed, unique_json_hex_u32_pair(captured, "old_flow"));
            assert_eq!(descent.hessian, expected_hessian);
            assert_eq!(descent.inverse, expected_inverse);
            assert_eq!(descent.gradient_sums, expected_gradient_sums);
            assert_eq!(descent.raw_sum, unique_json_hex_u32(captured, "raw_s"));
            assert_eq!(descent.raw_sum_sq, unique_json_hex_u32(captured, "raw_q"));
            let raw_rhs = unique_json_object(captured, "raw_rhs");
            assert_eq!(
                descent.raw_rhs,
                [
                    unique_json_hex_u32(raw_rhs, "col"),
                    unique_json_hex_u32(raw_rhs, "row"),
                ]
            );
            let corrected_rhs = unique_json_object(captured, "corrected_rhs");
            assert_eq!(
                descent.corrected_rhs,
                [
                    unique_json_hex_u32(corrected_rhs, "col"),
                    unique_json_hex_u32(corrected_rhs, "row"),
                ]
            );
            assert_eq!(
                descent.corrected_residual,
                unique_json_hex_u32(captured, "corrected_residual")
            );
            assert_eq!(descent.delta, unique_json_hex_u32_pair(captured, "delta"));
            assert_eq!(
                descent.updated_flow,
                unique_json_hex_u32_pair(captured, "updated_flow")
            );
            assert_eq!(descent.survivors, unique_json_usize(captured, "survivors"));
            assert_eq!(
                descent.survivors,
                unique_json_usize(captured, "mean_divisor")
            );
            assert_eq!(
                previous_residual,
                unique_json_hex_u32(captured, "previous_residual")
            );
            assert_eq!(
                accepted_count,
                unique_json_usize(captured, "accepted_before")
            );
            let accepted =
                f32::from_bits(descent.corrected_residual) < f32::from_bits(previous_residual);
            assert_eq!(unique_json_bool(captured, "accepted"), accepted);
            if accepted {
                previous_residual = descent.corrected_residual;
                accepted_count += 1;
            }
            assert_eq!(
                accepted_count,
                unique_json_usize(captured, "accepted_after")
            );

            let origin = unique_json_object(captured, "translated_target_origin");
            let expected_origin = [
                unique_json_hex_u32(origin, "row"),
                unique_json_hex_u32(origin, "col"),
            ];
            let expected_coefficients =
                unique_json_hex_u32_array::<4>(captured, "fixed_bilinear_coefficients");
            let seed = pis::Flow::new(
                f32::from_bits(descent.seed[0]),
                f32::from_bits(descent.seed[1]),
            )
            .expect("delayed-sealed descent seed must remain finite");
            assert_eq!(
                pis::test_descent_sample_boundary_at(&input, PATCH_ROW, PATCH_COL, seed),
                (expected_origin, expected_coefficients)
            );
        }

        assert_eq!(modes[PATCH_ROW], CostMode::Unweighted);
        assert_eq!(
            candidate_bits(candidates[2]),
            None,
            "column zero has no horizontal neighbour"
        );
        let current = candidate_bits(candidates[0]).expect("current candidate must be present");
        let hint = candidate_bits(candidates[1]).expect("present zero hint must be evaluated");
        let vertical =
            candidate_bits(candidates[3]).expect("row 36 must have a vertical neighbour");
        assert_eq!(
            current,
            (
                expected_seed,
                unique_json_hex_u32(captured_seed, "candidate_score"),
                64,
            )
        );
        assert!(unique_json_bool(captured_seed, "hint_candidate_present"));
        assert_eq!(hint.0, unique_json_hex_u32_pair(captured_seed, "hint_flow"));
        assert!(!unique_json_bool(captured_seed, "horizontal_available"));
        assert_eq!(vertical.0, [0x0000_0000, 0x0000_0000]);
        assert_eq!(
            hint, current,
            "zero hint score/survivor tuple differs from current"
        );
        assert_eq!(
            vertical, current,
            "zero vertical score/survivor tuple differs from current"
        );
        assert_eq!(pass.winner(), pis::Candidate::Current);
        assert_eq!(descents[0].seed, expected_seed);
        let captured_termination = unique_json_object(row36_oracle, "captured_termination");
        let captured_final_guard = unique_json_object(row36_oracle, "captured_final_guard");
        assert_eq!(descents.len(), usize::from(pass.descent_iterations()));
        assert_eq!(
            usize::from(pass.descent_iterations()),
            unique_json_usize(captured_termination, "completed_attempts")
        );
        assert_eq!(unique_json_usize(captured_termination, "accepted_count"), 3);
        assert_eq!(
            unique_json_string(captured_termination, "reason"),
            "non_improvement"
        );
        assert!(pass.stopped_on_no_improvement());
        assert!(!pass.guarded());
        let expected_stored_flow = unique_json_hex_u32_pair(captured_final_guard, "stored_flow");
        assert_eq!(
            unique_json_string(captured_final_guard, "store_action"),
            "STORE"
        );
        assert_eq!(
            [
                pass.stored_flow().dcol().to_bits(),
                pass.stored_flow().drow().to_bits(),
            ],
            expected_stored_flow
        );
        assert_eq!(
            descents
                .last()
                .expect("fourth descent disappeared")
                .updated_flow,
            expected_stored_flow
        );

        println!(
            concat!(
                r#"{{"boundary":"ab_l2_row36_col0_pass0_candidates","implementation":"kjerag_scalar_diagnostic","row_mode":"unweighted","current":{},"hint":{},"horizontal":{},"vertical":{},"winner":"{}"}}"#
            ),
            candidate_json(candidates[0]),
            candidate_json(candidates[1]),
            candidate_json(candidates[2]),
            candidate_json(candidates[3]),
            candidate_name(pass.winner()),
        );
        for (ordinal, descent) in descents.iter().enumerate() {
            let seed = pis::Flow::new(
                f32::from_bits(descent.seed[0]),
                f32::from_bits(descent.seed[1]),
            )
            .expect("authenticated descent seed must remain finite");
            let (translated_origin, coefficients) =
                pis::test_descent_sample_boundary_at(&input, PATCH_ROW, PATCH_COL, seed);
            println!(
                r#"{{"boundary":"ab_l2_row36_col0_pass0_sample","implementation":"kjerag_scalar_diagnostic","ordinal":{},"translated_origin":["0x{:08x}","0x{:08x}"],"coefficients":["0x{:08x}","0x{:08x}","0x{:08x}","0x{:08x}"]}}"#,
                ordinal + 1,
                translated_origin[0],
                translated_origin[1],
                coefficients[0],
                coefficients[1],
                coefficients[2],
                coefficients[3],
            );
            println!(
                "{}",
                first_descent_trace_line("ab_l2_row36_col0_pass0_descent_full", *descent,)
            );
            println!(
                r#"{{"boundary":"ab_l2_row36_col0_pass0_descent","implementation":"kjerag_scalar_diagnostic","ordinal":{},"seed":["0x{:08x}","0x{:08x}"],"residual":"0x{:08x}","delta":["0x{:08x}","0x{:08x}"],"updated_flow":["0x{:08x}","0x{:08x}"],"survivors":{}}}"#,
                ordinal + 1,
                descent.seed[0],
                descent.seed[1],
                descent.corrected_residual,
                descent.delta[0],
                descent.delta[1],
                descent.updated_flow[0],
                descent.updated_flow[1],
                descent.survivors,
            );
        }
        println!(
            r#"{{"boundary":"ab_l2_row36_col0_pass0_terminal","implementation":"kjerag_scalar_diagnostic","descent_iterations":{},"stopped_on_no_improvement":{},"guarded":{},"stored_flow":["0x{:08x}","0x{:08x}"],"studio_terminal":["0x{:08x}","0x{:08x}"]}}"#,
            pass.descent_iterations(),
            pass.stopped_on_no_improvement(),
            pass.guarded(),
            pass.stored_flow().dcol().to_bits(),
            pass.stored_flow().drow().to_bits(),
            studio_u[patch_index],
            studio_v[patch_index],
        );
    }

    #[test]
    #[ignore = "requires KJERAG_V9_STAGE with the authenticated partial stage_v9 corpus"]
    fn partial_v9_ab_l2_unweighted_helper_closes_captured_row54_outputs() {
        const PATCH_ROW: usize = 54;
        const PATCH_COL: usize = 2;

        let stage = v9_stage();
        assert_eq!(
            stage.file_name().and_then(|name| name.to_str()),
            Some("stage_v9"),
            "KJERAG_V9_STAGE must point directly at stage_v9"
        );
        // Authenticate the event ledger and all 22 physical payloads it names
        // before interpreting any input. JSON sidecars and the stale non-final
        // manifest are not evidence inputs.
        for &(name, size, hash) in &V9_ARTIFACTS {
            let _ = authenticated_v9_file(&stage, name, size, hash);
        }
        let retained = ColdInputs::from_before_input_blur(
            LensPair {
                a: read_v9(&stage, "000_shared_postblur_A.bin"),
                b: read_v9(&stage, "001_shared_postblur_B.bin"),
            },
            LensPair {
                a: read_v9(&stage, "002_shared_physical_mask_0.bin"),
                b: read_v9(&stage, "003_shared_physical_mask_1.bin"),
            },
        )
        .expect("authenticated V9 images and masks must have retained-grid shape");
        let masks = MaskPyramid::build(&retained);
        let prepared = LevelInputs::build::<AtoB>(&retained, &masks, Level::Two);
        let modes = native_work_modes(
            &read_v9(&stage, "008_ab_retained_rows_d8_2.bin"),
            Level::Two,
        );
        let input = ab_l2_input(&prepared, modes.clone());

        let distilled_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/research/studio-candidate-row54-unweighted-602.json");
        let distilled_bytes = std::fs::read(&distilled_path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", distilled_path.display()));
        assert_eq!(
            distilled_bytes.len(),
            6_681,
            "distilled oracle byte count differs"
        );
        assert_eq!(
            sha256_hex(&distilled_bytes),
            "8f8df87fda7f50248b4a80bada0267341113050e000d6c65c896d3290f23757b",
            "distilled oracle SHA-256 differs"
        );
        let distilled = std::str::from_utf8(&distilled_bytes)
            .expect("distilled row-54 candidate oracle must be UTF-8 JSON");
        assert_eq!(
            unique_json_string(distilled, "schema"),
            "kjerag.studio-unweighted-candidate-boundary-oracle.v1"
        );
        let captured_candidates = unique_json_object(distilled, "captured_candidates");
        let captured_horizontal = unique_json_object(captured_candidates, "horizontal");
        let studio_seed_bits = unique_json_hex_u32_pair(captured_horizontal, "flow");
        let studio_score_bits = unique_json_hex_u32(captured_horizontal, "score");
        assert_eq!(studio_seed_bits, [0xbec7_b2b5, 0x3d6f_fe86]);
        assert_eq!(studio_score_bits, 0x46f5_4a4a);
        let studio_seed = pis::Flow::new(
            f32::from_bits(studio_seed_bits[0]),
            f32::from_bits(studio_seed_bits[1]),
        )
        .expect("authenticated Studio candidate is finite");
        let studio_seed_candidate = input.score(PATCH_ROW, PATCH_COL, studio_seed);
        assert_eq!(studio_seed_candidate.survivors(), 64);
        assert_eq!(studio_seed_candidate.value().to_bits(), studio_score_bits);

        let (solved, first_descent, post_current_grid) = pis::solve_with_test_patch_boundary(
            &input,
            InitialGrid::coarse_zeros(),
            None,
            pis::DESCENTS_PER_PASS,
            PATCH_ROW,
            PATCH_COL,
        )
        .expect("authenticated V9 AB L2 largest-unweighted trace");

        let patch_index = PATCH_ROW * Level::Two.patch_cols() + PATCH_COL;
        let patch = solved.patches()[patch_index];
        let pass = patch.passes()[0];
        let candidates = pass.candidates();
        assert_eq!(
            (post_current_grid.patch_row, post_current_grid.patch_col),
            (PATCH_ROW, PATCH_COL)
        );
        assert_eq!(post_current_grid.level, Level::Two);
        assert_eq!(
            (post_current_grid.patch_rows, post_current_grid.patch_cols),
            (88, 3)
        );
        assert_eq!(post_current_grid.dcol_bits.len(), Level::Two.patches());
        assert_eq!(post_current_grid.drow_bits.len(), Level::Two.patches());
        assert_eq!(post_current_grid.current_score, 0x46fd_89f8);
        assert_eq!(post_current_grid.current_survivors, 64);
        assert!(
            post_current_grid.dcol_bits[patch_index..]
                .iter()
                .all(|&bits| bits == 0)
        );
        assert!(
            post_current_grid.drow_bits[patch_index..]
                .iter()
                .all(|&bits| bits == 0)
        );
        assert_eq!(
            [
                post_current_grid.dcol_bits[patch_index - 1],
                post_current_grid.drow_bits[patch_index - 1],
            ],
            [0xbec7_b2b5, 0x3d6f_fe86]
        );
        assert_eq!(
            [
                post_current_grid.dcol_bits[patch_index - Level::Two.patch_cols()],
                post_current_grid.drow_bits[patch_index - Level::Two.patch_cols()],
            ],
            [0x0000_0000, 0x0000_0000]
        );
        let post_current_u_hash = f32_bits_sha256(&post_current_grid.dcol_bits);
        let post_current_v_hash = f32_bits_sha256(&post_current_grid.drow_bits);
        let post_current_uv_hash =
            sparse_uv_f32le_sha256(&post_current_grid.dcol_bits, &post_current_grid.drow_bits);
        assert_eq!(
            post_current_u_hash,
            "fd249197cbc7e44577e380f09372e18bd356d09c27cbfdf283f90c99e14b0a2a"
        );
        assert_eq!(
            post_current_v_hash,
            "7724321227af20b53e0fa2dbc0d0e8ba1da7c828765885342708d358ae76ddc8"
        );
        assert_eq!(
            post_current_uv_hash,
            "1bd1dde12d48e38fd50c19e764752084a9fda9b115a9d6a8f210d0ae5aea075f"
        );
        println!(
            "row54 post-current-score U f32le SHA-256 {}",
            post_current_u_hash
        );
        println!(
            "row54 post-current-score V f32le SHA-256 {}",
            post_current_v_hash
        );
        println!(
            "row54 post-current-score U-then-V 2112-byte f32le SHA-256 {}",
            post_current_uv_hash
        );
        let native_u = f32_plane(
            "native A-to-B L2 U",
            &read_v9(&stage, "020_ab_l2_post_pis_sparse_u.bin"),
            Level::Two.patches(),
        );
        let native_v = f32_plane(
            "native A-to-B L2 V",
            &read_v9(&stage, "021_ab_l2_post_pis_sparse_v.bin"),
            Level::Two.patches(),
        );
        let native_target = [
            native_u[patch_index].to_bits(),
            native_v[patch_index].to_bits(),
        ];
        let rust_final = patch.flow();
        let rust_final = [rust_final.dcol().to_bits(), rust_final.drow().to_bits()];

        assert_eq!(
            (PATCH_ROW * PATCH_STRIDE, PATCH_COL * PATCH_STRIDE),
            (162, 6)
        );
        assert_eq!(modes[PATCH_ROW], CostMode::Unweighted);
        assert_eq!(native_target, [0xbecb_c69b, 0x3de8_386a]);
        assert_eq!(rust_final, native_target);
        println!(
            "row54 native final 0x{:08x}/0x{:08x}, Rust final 0x{:08x}/0x{:08x}, residual 0x{:08x}",
            native_target[0],
            native_target[1],
            rust_final[0],
            rust_final[1],
            patch.residual().to_bits(),
        );

        assert_eq!(
            candidate_bits(candidates[0]),
            Some(([0x0000_0000, 0x0000_0000], 0x46fd_89f8, 64))
        );
        assert_eq!(candidate_bits(candidates[1]), None);
        assert_eq!(
            candidate_bits(candidates[2]),
            Some(([0xbec7_b2b5, 0x3d6f_fe86], 0x46f5_4a4a, 64))
        );
        assert_eq!(
            candidate_bits(candidates[3]),
            Some(([0x0000_0000, 0x0000_0000], 0x46fd_89f8, 64))
        );
        assert_eq!(pass.winner(), pis::Candidate::Horizontal);
        assert_eq!(first_descent.seed, [0xbec7_b2b5, 0x3d6f_fe86]);
        assert_eq!(
            first_descent.hessian,
            [0x4954_99e0, 0x467b_7000, 0x46a2_0c00]
        );
        assert_eq!(
            first_descent.inverse,
            [0x359c_5ef1, 0xb572_a162, 0x384d_2780]
        );
        assert_eq!(first_descent.gradient_sums, [0xc594_8000, 0x4208_0000]);
        assert_eq!(first_descent.raw_sum, 0x4421_bb5b);
        assert_eq!(first_descent.raw_sum_sq, 0x4714_3073);
        assert_eq!(first_descent.raw_rhs, [0xc816_8dc2, 0xc54d_f76a]);
        assert_eq!(first_descent.corrected_rhs, [0xc7cf_4a56, 0xc563_724c]);
        assert_eq!(first_descent.corrected_residual, 0x46f5_4a4d);
        assert_eq!(first_descent.delta, [0xbdf6_7fce, 0xbda8_144c]);
        assert_eq!(first_descent.updated_flow, [0xbe8a_12c2, 0x3e10_09c8]);
        assert_eq!(first_descent.survivors, 64);

        // Independently hold the authenticated Studio 6.0.2 DB8 seed fixed
        // while replaying Kjerag's immediate descent. The ordinary propagated
        // seed above now equals this captured value; the duplicate route keeps
        // the candidate helper and scalar descent boundaries independently
        // pinned.
        let (studio_seed_origin, studio_seed_coefficients) =
            pis::test_descent_sample_boundary_at(&input, PATCH_ROW, PATCH_COL, studio_seed);
        let studio_seed_descent =
            pis::test_first_descent_at(&input, PATCH_ROW, PATCH_COL, studio_seed);
        assert_eq!(studio_seed_origin, [0x4332_0f00, 0x41ac_e135]);
        assert_eq!(
            studio_seed_coefficients,
            [0x3ebb_ff47, 0x3f13_005d, 0x3cbb_3794, 0x3d12_6436]
        );
        assert_eq!(studio_seed_descent.seed, [0xbec7_b2b5, 0x3d6f_fe86]);
        assert_eq!(
            studio_seed_descent.hessian,
            [0x4954_99e0, 0x467b_7000, 0x46a2_0c00]
        );
        assert_eq!(
            studio_seed_descent.inverse,
            [0x359c_5ef1, 0xb572_a162, 0x384d_2780]
        );
        assert_eq!(
            studio_seed_descent.gradient_sums,
            [0xc594_8000, 0x4208_0000]
        );
        assert_eq!(studio_seed_descent.raw_sum, 0x4421_bb5b);
        assert_eq!(studio_seed_descent.raw_sum_sq, 0x4714_3073);
        assert_eq!(studio_seed_descent.raw_rhs, [0xc816_8dc2, 0xc54d_f76a]);
        assert_eq!(
            studio_seed_descent.corrected_rhs,
            [0xc7cf_4a56, 0xc563_724c]
        );
        assert_eq!(studio_seed_descent.corrected_residual, 0x46f5_4a4d);
        assert_eq!(
            studio_seed_descent.corrected_residual - studio_seed_candidate.value().to_bits(),
            3,
            "candidate and descent are distinct native helpers"
        );
        assert_eq!(studio_seed_descent.delta, [0xbdf6_7fce, 0xbda8_144c]);
        assert_eq!(studio_seed_descent.updated_flow, [0xbe8a_12c2, 0x3e10_09c8]);
        assert_eq!(studio_seed_descent.survivors, 64);
        println!(
            "{}",
            first_descent_trace_line(
                "ab_l2_row54_col2_unweighted_studio_seed_first_descent",
                studio_seed_descent,
            )
        );

        println!(
            concat!(
                r#"{{"boundary":"ab_l2_pass0_candidate","implementation":"kjerag_scalar_hypothesis","patch_row":{},"patch_col":{},"candidates":{{"current":{},"hint":{},"horizontal":{},"vertical":{}}},"winner":"{}","seed":["0x{:08x}","0x{:08x}"],"captured_native_final_target":["0x{:08x}","0x{:08x}"],"rust_final":["0x{:08x}","0x{:08x}"]}}"#
            ),
            PATCH_ROW,
            PATCH_COL,
            candidate_json(candidates[0]),
            candidate_json(candidates[1]),
            candidate_json(candidates[2]),
            candidate_json(candidates[3]),
            candidate_name(pass.winner()),
            first_descent.seed[0],
            first_descent.seed[1],
            native_target[0],
            native_target[1],
            rust_final[0],
            rust_final[1],
        );
        println!(
            "{}",
            first_descent_trace_line(
                "ab_l2_row54_col2_unweighted_pass0_first_descent",
                first_descent,
            )
        );
    }

    #[test]
    #[ignore = "requires KJERAG_V9_STAGE with the authenticated partial stage_v9 corpus"]
    fn partial_v9_ab_l2_weighted_first_cell_trace() {
        const PATCH_ROW: usize = 0;
        const PATCH_COL: usize = 0;

        let stage = v9_stage();
        for &(name, size, hash) in &V9_ARTIFACTS {
            let _ = authenticated_v9_file(&stage, name, size, hash);
        }
        let retained = ColdInputs::from_before_input_blur(
            LensPair {
                a: read_v9(&stage, "000_shared_postblur_A.bin"),
                b: read_v9(&stage, "001_shared_postblur_B.bin"),
            },
            LensPair {
                a: read_v9(&stage, "002_shared_physical_mask_0.bin"),
                b: read_v9(&stage, "003_shared_physical_mask_1.bin"),
            },
        )
        .expect("authenticated V9 images and masks must have retained-grid shape");
        let masks = MaskPyramid::build(&retained);
        let prepared = LevelInputs::build::<AtoB>(&retained, &masks, Level::Two);
        let raw_weight_hash = f32_sha256(&prepared.raw_weight);
        assert_eq!(
            raw_weight_hash,
            "08fa436db72c82ca823189965c4d8316d32f917abf40b1882005ce77aa4efff5"
        );
        println!(
            "weighted row-zero prepared raw weight f32le SHA-256 {}",
            raw_weight_hash
        );
        if let Some(path) = std::env::var_os("KJERAG_V9_STUDIO_RAW_WEIGHT") {
            let bytes = std::fs::read(PathBuf::from(path))
                .expect("could not read accepted Studio AB L2 raw-weight plane");
            assert_eq!(bytes.len(), Level::Two.pixels() * 4);
            assert_eq!(
                sha256_hex(&bytes),
                "08fa436db72c82ca823189965c4d8316d32f917abf40b1882005ce77aa4efff5"
            );
            let studio = f32_plane(
                "accepted Studio AB L2 raw weight",
                &bytes,
                Level::Two.pixels(),
            );
            assert_eq!(prepared.raw_weight.as_slice(), studio);
            println!(
                r#"{{"boundary":"ab_l2_raw_weight_plane","implementation":"kjerag_scalar_exact_selected_gaussian","studio_sha256":"08fa436db72c82ca823189965c4d8316d32f917abf40b1882005ce77aa4efff5","kjerag_sha256":"{}","values":{},"mismatches":0}}"#,
                raw_weight_hash,
                prepared.raw_weight.len(),
            );
        }
        let modes = native_work_modes(
            &read_v9(&stage, "008_ab_retained_rows_d8_2.bin"),
            Level::Two,
        );
        let input = ab_l2_input(&prepared, modes.clone());
        let (solved, first_descent) = pis::solve_with_test_first_descent(
            &input,
            InitialGrid::coarse_zeros(),
            None,
            pis::DESCENTS_PER_PASS,
            PATCH_ROW,
            PATCH_COL,
        )
        .expect("authenticated V9 AB L2 weighted first-cell trace");
        let patch = solved.patch(PATCH_ROW, PATCH_COL);
        let pass = patch.passes()[0];
        let candidates = pass.candidates();
        let survivor_trace =
            pis::test_descent_survivors_at(&input, PATCH_ROW, PATCH_COL, pis::Flow::ZERO);

        assert_eq!(modes[PATCH_ROW], CostMode::Weighted);
        assert_eq!(
            candidate_bits(candidates[0]),
            Some(([0x0000_0000, 0x0000_0000], 0x5015_02f9, 2))
        );
        assert_eq!(candidate_bits(candidates[1]), None);
        assert_eq!(candidate_bits(candidates[2]), None);
        assert_eq!(candidate_bits(candidates[3]), None);
        assert_eq!(pass.winner(), pis::Candidate::Current);
        assert_eq!(first_descent.seed, [0x0000_0000, 0x0000_0000]);
        assert_eq!(
            first_descent.hessian,
            [0x461a_6400, 0x45f8_e800, 0x468e_6200]
        );
        assert_eq!(
            first_descent.inverse,
            [0x3923_d70a, 0xb88f_3553, 0x38b1_a84b]
        );
        assert_eq!(first_descent.gradient_sums, [0xc30b_0000, 0xc307_0000]);
        assert_eq!(first_descent.raw_sum, 0xc1e2_f3ca);
        assert_eq!(first_descent.raw_sum_sq, 0x43d8_1bca);
        assert_eq!(first_descent.raw_rhs, [0x4500_4bb2, 0x44ae_353a]);
        assert_eq!(first_descent.corrected_rhs, [0x42a2_2a70, 0xc402_4fca]);
        assert_eq!(first_descent.corrected_residual, 0x41ee_8710);
        assert_eq!(first_descent.delta, [0x3d45_b00c, 0xbd4b_8b89]);
        assert_eq!(first_descent.updated_flow, [0xbd45_b00c, 0x3d4b_8b89]);
        assert_eq!(first_descent.survivors, 2);
        assert_eq!(survivor_trace.len(), 2);
        assert_eq!(
            (
                survivor_trace[0].ordinal,
                survivor_trace[0].patch_row,
                survivor_trace[0].patch_col,
                survivor_trace[0].raw_weight,
                survivor_trace[0].reciprocal_weight_sum,
                survivor_trace[0].normalized_residual,
                survivor_trace[0].pre,
                survivor_trace[0].post,
            ),
            (
                1,
                0,
                7,
                0x42db_6d49,
                0x3b9e_84fc,
                0xc190_5d66,
                [0x0000_0000; 4],
                [0xc190_5d66, 0x43a2_d26a, 0x44b4_74c0, 0x0000_0000],
            )
        );
        assert_eq!(
            (
                survivor_trace[1].ordinal,
                survivor_trace[1].patch_row,
                survivor_trace[1].patch_col,
                survivor_trace[1].raw_weight,
                survivor_trace[1].reciprocal_weight_sum,
                survivor_trace[1].normalized_residual,
                survivor_trace[1].pre,
                survivor_trace[1].post,
            ),
            (
                2,
                1,
                7,
                0x42c1_ffa8,
                0x3b9e_84fc,
                0xc125_2cc7,
                [0xc190_5d66, 0x43a2_d26a, 0x44b4_74c0, 0x0000_0000],
                [0xc1e2_f3ca, 0x43d8_1bca, 0x4500_4bb2, 0x44ae_353a],
            )
        );

        // Accepted Studio 6.0.2 V5-06 evidence SHA-256
        // 282d45907b37c00ffd6a33855771566dce85102eb3b150f55db9352fc97fc4fb
        // carries these exact comparable boundaries. This is a bounded first
        // descent result, not a hint-route, full-field, or playback claim.
        const STUDIO_RAW_WEIGHT_2: u32 = 0x42c1_ffa8;
        const STUDIO_RESIDUAL_2: u32 = 0xc125_2cc7;
        const STUDIO_POST_2: [u32; 4] = [0xc1e2_f3ca, 0x43d8_1bca, 0x4500_4bb2, 0x44ae_353a];
        const STUDIO_CORRECTED_RHS: [u32; 2] = [0x42a2_2a70, 0xc402_4fca];
        const STUDIO_CORRECTED_RESIDUAL: u32 = 0x41ee_8710;
        const STUDIO_DELTA: [u32; 2] = [0x3d45_b00c, 0xbd4b_8b89];
        const STUDIO_UPDATED_FLOW: [u32; 2] = [0xbd45_b00c, 0x3d4b_8b89];
        assert_eq!(survivor_trace[1].raw_weight, STUDIO_RAW_WEIGHT_2);
        assert_eq!(survivor_trace[1].normalized_residual, STUDIO_RESIDUAL_2);
        assert_eq!(survivor_trace[1].post, STUDIO_POST_2);
        assert_eq!(first_descent.corrected_rhs, STUDIO_CORRECTED_RHS);
        assert_eq!(first_descent.corrected_residual, STUDIO_CORRECTED_RESIDUAL);
        assert_eq!(first_descent.delta, STUDIO_DELTA);
        assert_eq!(first_descent.updated_flow, STUDIO_UPDATED_FLOW);
        println!(
            concat!(
                r#"{{"boundary":"ab_l2_row0_col0_weighted_pass0_candidate","implementation":"kjerag_scalar_hypothesis","patch_row":{},"patch_col":{},"candidates":{{"current":{},"hint":{},"horizontal":{},"vertical":{}}},"winner":"{}","seed":["0x{:08x}","0x{:08x}"]}}"#
            ),
            PATCH_ROW,
            PATCH_COL,
            candidate_json(candidates[0]),
            candidate_json(candidates[1]),
            candidate_json(candidates[2]),
            candidate_json(candidates[3]),
            candidate_name(pass.winner()),
            first_descent.seed[0],
            first_descent.seed[1],
        );
        println!(
            "{}",
            first_descent_trace_line(
                "ab_l2_row0_col0_weighted_pass0_first_descent",
                first_descent,
            )
        );
        for tap in &survivor_trace {
            println!(
                concat!(
                    r#"{{"boundary":"ab_l2_row0_col0_weighted_descent_survivor","implementation":"kjerag_scalar_hypothesis","ordinal":{},"patch_row":{},"patch_col":{},"raw_weight":"0x{:08x}","reciprocal":"0x{:08x}","difference":"0x{:08x}","normalized_residual":"0x{:08x}","gradient":["0x{:08x}","0x{:08x}"],"pre":{{"s":"0x{:08x}","q":"0x{:08x}","col_rhs":"0x{:08x}","row_rhs":"0x{:08x}"}},"post":{{"s":"0x{:08x}","q":"0x{:08x}","col_rhs":"0x{:08x}","row_rhs":"0x{:08x}"}}}}"#
                ),
                tap.ordinal,
                tap.patch_row,
                tap.patch_col,
                tap.raw_weight,
                tap.reciprocal_weight_sum,
                tap.difference,
                tap.normalized_residual,
                tap.gradient_col,
                tap.gradient_row,
                tap.pre[0],
                tap.pre[1],
                tap.pre[2],
                tap.pre[3],
                tap.post[0],
                tap.post[1],
                tap.post[2],
                tap.post[3],
            );
        }
    }

    #[test]
    #[ignore = "requires authenticated V9 stage and accepted row-6 evidence"]
    fn partial_v9_ab_l2_row6_descent_loop_oracle() {
        const PATCH_ROW: usize = 6;
        const PATCH_COL: usize = 2;

        let stage = v9_stage();
        assert_eq!(
            stage.file_name().and_then(|name| name.to_str()),
            Some("stage_v9"),
            "KJERAG_V9_STAGE must point directly at stage_v9"
        );
        for &(name, size, hash) in &V9_ARTIFACTS {
            let _ = authenticated_v9_file(&stage, name, size, hash);
        }

        let evidence_path = std::env::var_os("KJERAG_ROW6_DESCENT_LOOP_EVIDENCE")
            .map(PathBuf::from)
            .expect("set KJERAG_ROW6_DESCENT_LOOP_EVIDENCE to the accepted row-6 evidence JSON");
        let evidence_bytes = std::fs::read(&evidence_path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", evidence_path.display()));
        assert_eq!(evidence_bytes.len(), 17_578, "evidence byte count differs");
        assert_eq!(
            sha256_hex(&evidence_bytes),
            "3549b8857d37ba99075161813a2586950f629f48f5399532d6ea8477dd749c2f",
            "evidence SHA-256 differs"
        );
        let evidence = std::str::from_utf8(&evidence_bytes)
            .expect("accepted row-6 evidence must be UTF-8 JSON");
        assert_eq!(
            unique_json_string(evidence, "schema"),
            "kjerag.v9-row6-descent-loop-capture.v2"
        );
        let live_interval = unique_json_object(evidence, "selected_disparity");
        assert_eq!(unique_json_string(live_interval, "a_u"), "0xc0700000");
        assert_eq!(unique_json_string(live_interval, "a_v"), "0xbdfffc66");
        assert_eq!(unique_json_string(live_interval, "b_u"), "0x3e7ffc66");
        assert_eq!(unique_json_string(live_interval, "b_v"), "0x3dfffc66");
        assert_eq!(unique_json_string(evidence, "outcome"), "disparity_skip");
        assert_eq!(unique_json_string(evidence, "store_action"), "SKIP");

        let oracle_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/research/studio-row6-descent-loop-602.json");
        let oracle_bytes = std::fs::read(&oracle_path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", oracle_path.display()));
        assert_eq!(oracle_bytes.len(), 5_447, "oracle byte count differs");
        assert_eq!(
            sha256_hex(&oracle_bytes),
            "ceaf62a6a65cd72b20cd0d5b6e987cf8a87e663e797203892428061314c16f70",
            "oracle SHA-256 differs"
        );
        let oracle =
            std::str::from_utf8(&oracle_bytes).expect("tracked row-6 oracle must be UTF-8 JSON");
        assert_eq!(
            unique_json_string(oracle, "schema"),
            "kjerag.studio-row6-descent-loop-oracle.v1"
        );
        let provenance = unique_json_object(oracle, "capture_provenance");
        assert_eq!(
            unique_json_string(provenance, "evidence_sha256"),
            sha256_hex(&evidence_bytes)
        );
        assert_eq!(
            unique_json_string(provenance, "verification_verdict"),
            "accepted"
        );
        let terminal_guard = unique_json_object(oracle, "captured_final_guard");
        assert_eq!(
            unique_json_string(terminal_guard, "outcome"),
            "disparity_skip"
        );
        assert_eq!(unique_json_string(terminal_guard, "store_action"), "SKIP");
        let captured_candidates = unique_json_object(oracle, "captured_candidates");
        let candidate_oracle = [
            Some({
                let candidate = unique_json_object(captured_candidates, "current");
                (
                    unique_json_hex_u32_pair(candidate, "flow"),
                    unique_json_hex_u32(candidate, "score"),
                )
            }),
            None,
            Some({
                let candidate = unique_json_object(captured_candidates, "horizontal");
                (
                    unique_json_hex_u32_pair(candidate, "flow"),
                    unique_json_hex_u32(candidate, "score"),
                )
            }),
            Some({
                let candidate = unique_json_object(captured_candidates, "vertical");
                (
                    unique_json_hex_u32_pair(candidate, "flow"),
                    unique_json_hex_u32(candidate, "score"),
                )
            }),
        ];
        let oracle_winner = unique_json_string(captured_candidates, "winner");
        let oracle_seed = unique_json_hex_u32_pair(captured_candidates, "seed");
        let oracle_updates = json_object_elements(unique_json_array(oracle, "captured_updates"))
            .into_iter()
            .enumerate()
            .map(|(index, update)| {
                assert_eq!(unique_json_usize(update, "ordinal"), index + 1);
                (
                    unique_json_hex_u32(update, "residual"),
                    unique_json_hex_u32_pair(update, "tentative_flow"),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(oracle_updates.len(), pis::DESCENTS_PER_PASS);
        let oracle_first_endpoint = unique_json_hex_u32_pair(terminal_guard, "first_endpoint");
        let oracle_second_endpoint = unique_json_hex_u32_pair(terminal_guard, "second_endpoint");
        let oracle_tentative = unique_json_hex_u32_pair(terminal_guard, "tentative_flow");
        let oracle_retained = unique_json_hex_u32_pair(terminal_guard, "retained_flow");

        let retained = ColdInputs::from_before_input_blur(
            LensPair {
                a: read_v9(&stage, "000_shared_postblur_A.bin"),
                b: read_v9(&stage, "001_shared_postblur_B.bin"),
            },
            LensPair {
                a: read_v9(&stage, "002_shared_physical_mask_0.bin"),
                b: read_v9(&stage, "003_shared_physical_mask_1.bin"),
            },
        )
        .expect("authenticated V9 images and masks must have retained-grid shape");
        let masks = MaskPyramid::build(&retained);
        let prepared = LevelInputs::build::<AtoB>(&retained, &masks, Level::Two);
        let modes = native_work_modes(
            &read_v9(&stage, "008_ab_retained_rows_d8_2.bin"),
            Level::Two,
        );
        let input = ab_l2_input(&prepared, modes.clone());
        let (solved, descent_sequence) = pis::solve_with_test_descent_sequence(
            &input,
            InitialGrid::coarse_zeros(),
            None,
            pis::DESCENTS_PER_PASS,
            PATCH_ROW,
            PATCH_COL,
        )
        .expect("authenticated V9 AB L2 row-6 descent-loop trace");
        let first_descent = descent_sequence[0];
        let index = PATCH_ROW * Level::Two.patch_cols() + PATCH_COL;
        let patch = solved.patches()[index];
        let pass = patch.passes()[0];
        let candidates = pass.candidates();
        let studio_seed = pis::Flow::new(
            f32::from_bits(oracle_seed[0]),
            f32::from_bits(oracle_seed[1]),
        )
        .expect("sealed Studio seed is finite");
        let studio_seed_score = input.score(PATCH_ROW, PATCH_COL, studio_seed);
        let studio_seed_descent =
            pis::test_first_descent_at(&input, PATCH_ROW, PATCH_COL, studio_seed);

        assert_eq!(
            input.disparity_interval(),
            Some(pis::DisparityInterval::new(
                [
                    f32::from_bits(oracle_first_endpoint[0]),
                    f32::from_bits(oracle_first_endpoint[1]),
                ],
                [
                    f32::from_bits(oracle_second_endpoint[0]),
                    f32::from_bits(oracle_second_endpoint[1]),
                ],
            ))
        );
        assert_eq!(oracle_first_endpoint, [0xc070_0000, 0xbdff_fc66]);
        assert_eq!(oracle_second_endpoint, [0x3e7f_fc66, 0x3dff_fc66]);

        assert_eq!(modes[PATCH_ROW], CostMode::Weighted);
        for earlier in 0..index {
            let row = earlier / Level::Two.patch_cols();
            if modes[row] == CostMode::Weighted {
                assert_eq!(
                    solved.patches()[earlier].passes()[0].winner(),
                    pis::Candidate::Current,
                    "an earlier weighted patch selected a propagated candidate"
                );
            }
        }

        assert_eq!(
            candidate_bits(candidates[0]),
            Some(([0x0000_0000, 0x0000_0000], 0x4223_54a2, 9))
        );
        assert_eq!(candidate_bits(candidates[1]), None);
        assert_eq!(
            candidate_bits(candidates[2]),
            Some(([0x3dcd_8267, 0x3dcf_7d3c], 0x41cd_a678, 9))
        );
        assert_eq!(
            candidate_bits(candidates[3]),
            Some(([0xbc9d_74cf, 0xbc87_bb87], 0x5015_02f9, 1))
        );
        let candidate_result = candidates.map(|candidate| {
            candidate.map(|candidate| {
                (
                    [
                        candidate.flow().dcol().to_bits(),
                        candidate.flow().drow().to_bits(),
                    ],
                    candidate.score().value().to_bits(),
                )
            })
        });
        assert_eq!(candidate_result, candidate_oracle);
        assert_eq!(pass.winner(), pis::Candidate::Horizontal);
        assert_eq!(candidate_name(pass.winner()), oracle_winner);
        assert_eq!(pass.descent_iterations(), 6);
        assert!(!pass.stopped_on_no_improvement());
        assert!(pass.guarded());
        assert_eq!(
            [
                pass.stored_flow().dcol().to_bits(),
                pass.stored_flow().drow().to_bits(),
            ],
            oracle_retained
        );
        assert_eq!(oracle_retained, [0x3dcd_8267, 0x3dcf_7d3c]);
        assert_eq!(first_descent.seed, oracle_seed);
        assert_eq!(
            first_descent.hessian,
            [0x4755_5b00, 0xc741_8b00, 0x4815_6bc0]
        );
        assert_eq!(
            first_descent.inverse,
            [0x37d9_76eb, 0x370c_d6ea, 0x371b_41c3]
        );
        assert_eq!(first_descent.gradient_sums, [0xc425_c000, 0x4435_4000]);
        assert_eq!(first_descent.raw_sum, 0xc189_1b0b);
        assert_eq!(first_descent.raw_sum_sq, 0x4269_5dd2);
        assert_eq!(first_descent.raw_rhs, [0x4495_1416, 0xc535_9176]);
        assert_eq!(first_descent.mean_correction, [0x449d_d076, 0xc4ac_927c]);
        assert_eq!(first_descent.corrected_rhs, [0xc28b_c600, 0xc4be_9070]);
        assert_eq!(first_descent.corrected_residual, 0x41cd_a678);
        assert_eq!(first_descent.delta, [0xbc6f_5ce1, 0xbc70_c183]);
        assert_eq!(first_descent.updated_flow, [0x3deb_6e03, 0x3ded_956c]);
        assert_eq!(first_descent.survivors, 9);
        assert_eq!(
            candidates[2]
                .expect("horizontal candidate was evaluated")
                .score()
                .value()
                .to_bits(),
            first_descent.corrected_residual,
        );

        // Hold Studio's sealed winner fixed while evaluating Kjerag's score
        // and immediate descent, independent of Kjerag's propagation seed.
        const STUDIO_HORIZONTAL_SCORE: u32 = 0x41cd_a678;
        const STUDIO_INVERSE: [u32; 3] = [0x37d9_76eb, 0x370c_d6ea, 0x371b_41c3];
        const STUDIO_GRADIENT_SUMS: [u32; 2] = [0xc425_c000, 0x4435_4000];
        const STUDIO_RAW_SUMS: [u32; 2] = [0xc189_1b0b, 0x4269_5dd2];
        const STUDIO_RAW_RHS: [u32; 2] = [0x4495_1416, 0xc535_9176];
        const STUDIO_CORRECTED_RHS: [u32; 2] = [0xc28b_c600, 0xc4be_9070];
        const STUDIO_RESIDUAL: u32 = 0x41cd_a678;
        const STUDIO_DELTA: [u32; 2] = [0xbc6f_5ce1, 0xbc70_c183];
        const STUDIO_UPDATED_FLOW: [u32; 2] = [0x3deb_6e03, 0x3ded_956c];

        assert_eq!(studio_seed_score.value().to_bits(), 0x41cd_a678);
        assert_eq!(studio_seed_score.survivors(), 9);
        assert_eq!(studio_seed_descent.seed, [0x3dcd_8267, 0x3dcf_7d3c]);
        assert_eq!(
            studio_seed_descent.hessian,
            [0x4755_5b00, 0xc741_8b00, 0x4815_6bc0]
        );
        assert_eq!(
            studio_seed_descent.inverse,
            [0x37d9_76eb, 0x370c_d6ea, 0x371b_41c3]
        );
        assert_eq!(
            studio_seed_descent.gradient_sums,
            [0xc425_c000, 0x4435_4000]
        );
        assert_eq!(studio_seed_descent.raw_sum, 0xc189_1b0b);
        assert_eq!(studio_seed_descent.raw_sum_sq, 0x4269_5dd2);
        assert_eq!(studio_seed_descent.raw_rhs, [0x4495_1416, 0xc535_9176]);
        assert_eq!(
            studio_seed_descent.mean_correction,
            [0x449d_d076, 0xc4ac_927c]
        );
        assert_eq!(
            studio_seed_descent.corrected_rhs,
            [0xc28b_c600, 0xc4be_9070]
        );
        assert_eq!(studio_seed_descent.corrected_residual, 0x41cd_a678);
        assert_eq!(studio_seed_descent.delta, [0xbc6f_5ce1, 0xbc70_c183]);
        assert_eq!(studio_seed_descent.updated_flow, [0x3deb_6e03, 0x3ded_956c]);
        assert_eq!(studio_seed_descent.survivors, 9);
        assert_eq!(
            studio_seed_score.value().to_bits(),
            studio_seed_descent.corrected_residual,
        );

        assert_eq!(studio_seed_score.value().to_bits(), STUDIO_HORIZONTAL_SCORE);
        assert_eq!(studio_seed_descent.inverse, STUDIO_INVERSE);
        assert_eq!(studio_seed_descent.gradient_sums, STUDIO_GRADIENT_SUMS);
        assert_eq!(
            [studio_seed_descent.raw_sum, studio_seed_descent.raw_sum_sq],
            STUDIO_RAW_SUMS
        );
        assert_eq!(studio_seed_descent.raw_rhs, STUDIO_RAW_RHS);
        assert_eq!(studio_seed_descent.corrected_rhs, STUDIO_CORRECTED_RHS);
        assert_eq!(studio_seed_descent.corrected_residual, STUDIO_RESIDUAL);
        assert_eq!(studio_seed_descent.delta, STUDIO_DELTA);
        assert_eq!(studio_seed_descent.updated_flow, STUDIO_UPDATED_FLOW);

        // The accepted Studio capture observes all six residuals and tentative
        // flows. These traces come from the ordinary scalar solve, not a
        // separate arithmetic replay; the deltas remain Kjerag-computed.
        let descent_bits = descent_sequence
            .iter()
            .map(|descent| {
                (
                    descent.seed,
                    descent.corrected_residual,
                    descent.delta,
                    descent.updated_flow,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            descent_sequence.len(),
            usize::from(pass.descent_iterations())
        );
        let captured_update_result = descent_sequence
            .iter()
            .map(|descent| (descent.corrected_residual, descent.updated_flow))
            .collect::<Vec<_>>();
        assert_eq!(captured_update_result, oracle_updates);
        assert_eq!(
            descent_bits,
            vec![
                (
                    [0x3dcd_8267, 0x3dcf_7d3c],
                    0x41cd_a678,
                    [0xbc6f_5ce1, 0xbc70_c183],
                    [0x3deb_6e03, 0x3ded_956c],
                ),
                (
                    [0x3deb_6e03, 0x3ded_956c],
                    0x41be_d0c4,
                    [0xbc66_96ff, 0xbc67_aa54],
                    [0x3e04_2071, 0x3e05_455b],
                ),
                (
                    [0x3e04_2071, 0x3e05_455b],
                    0x41b1_292a,
                    [0xbc5e_3012, 0xbc5e_f567],
                    [0x3e12_0372, 0x3e13_34b1],
                ),
                (
                    [0x3e12_0372, 0x3e13_34b1],
                    0x41a4_99f7,
                    [0xbc56_25ef, 0xbc56_a07a],
                    [0x3e1f_65d1, 0x3e20_9eb9],
                ),
                (
                    [0x3e1f_65d1, 0x3e20_9eb9],
                    0x4199_07a8,
                    [0xbc4e_717d, 0xbc4e_a43a],
                    [0x3e2c_4ce9, 0x3e2d_88fd],
                ),
                (
                    [0x3e2c_4ce9, 0x3e2d_88fd],
                    0x418e_5d89,
                    [0xbc47_0ed1, 0xbc46_fc98],
                    [0x3e38_bdd6, 0x3e39_f8c6],
                ),
            ]
        );
        assert_eq!(
            descent_sequence
                .last()
                .expect("six-descent row-6 trace disappeared")
                .updated_flow,
            oracle_tentative
        );
        assert_eq!(oracle_tentative, [0x3e38_bdd6, 0x3e39_f8c6]);

        println!(
            concat!(
                r#"{{"boundary":"ab_l2_row6_col2_pass0_terminal","implementation":"kjerag_scalar_oracle","patch_row":{},"patch_col":{},"source_row":{},"source_col":{},"row_mode":"weighted","estimated_earlier_current_site_hits":{},"candidates":{{"current":{},"hint":{},"horizontal":{},"vertical":{}}},"winner":"{}","seed":["0x{:08x}","0x{:08x}"],"tentative":["0x{:08x}","0x{:08x}"],"guarded":true,"store_action":"SKIP","stored_flow":["0x{:08x}","0x{:08x}"]}}"#
            ),
            PATCH_ROW,
            PATCH_COL,
            PATCH_ROW * PATCH_STRIDE,
            PATCH_COL * PATCH_STRIDE,
            index,
            candidate_json(candidates[0]),
            candidate_json(candidates[1]),
            candidate_json(candidates[2]),
            candidate_json(candidates[3]),
            candidate_name(pass.winner()),
            first_descent.seed[0],
            first_descent.seed[1],
            descent_sequence.last().unwrap().updated_flow[0],
            descent_sequence.last().unwrap().updated_flow[1],
            pass.stored_flow().dcol().to_bits(),
            pass.stored_flow().drow().to_bits(),
        );
        println!(
            "{}",
            first_descent_trace_line("ab_l2_row6_col2_pass0_first_descent", first_descent)
        );
        println!(
            r#"{{"boundary":"ab_l2_row6_col2_studio_seed_score","implementation":"kjerag_scalar_hypothesis","seed":["0x{:08x}","0x{:08x}"],"score":"0x{:08x}","survivors":{}}}"#,
            studio_seed.dcol().to_bits(),
            studio_seed.drow().to_bits(),
            studio_seed_score.value().to_bits(),
            studio_seed_score.survivors(),
        );
        println!(
            "{}",
            first_descent_trace_line(
                "ab_l2_row6_col2_studio_seed_first_descent",
                studio_seed_descent
            )
        );
    }

    #[test]
    fn prepared_inputs_reject_one_wrong_physical_plane() {
        let pixels = ROWS * COLS;
        let error = ColdInputs::from_prepared(
            LensPair {
                a: vec![0; pixels],
                b: vec![0; pixels - 1],
            },
            LensPair {
                a: vec![255; pixels],
                b: vec![255; pixels],
            },
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ONE X2 scalar lens B image has 64799 samples, expected 64800",
        );
    }

    #[test]
    fn preblurred_handoff_matches_existing_path_and_does_not_blur_again() {
        let centre = (ROWS / 2, COLS / 2);
        let retained = SolverBelts::from_fn(|lens, row, col| {
            u8::from(lens == Lens::A && (row, col) == centre) * u8::MAX
        });
        let masks = LensPair {
            a: vec![u8::MAX; ROWS * COLS],
            b: vec![u8::MAX; ROWS * COLS],
        };
        let expected = ColdInputs::from_solver_belts_and_masks(retained.clone(), masks.clone());
        let once = temporal::gaussian_blur(&retained);
        let actual = ColdInputs::from_blurred_belts_and_masks(once.clone(), masks);

        assert_eq!(actual.image(Lens::A), expected.image(Lens::A));
        assert_eq!(actual.image(Lens::B), expected.image(Lens::B));
        assert_eq!(actual.mask(Lens::A), expected.mask(Lens::A));
        assert_eq!(actual.mask(Lens::B), expected.mask(Lens::B));

        let twice_input = SolverBelts::from_lenses(LensPair {
            a: once.lens(Lens::A).to_vec(),
            b: once.lens(Lens::B).to_vec(),
        })
        .unwrap();
        let twice = temporal::gaussian_blur(&twice_input);
        assert_ne!(actual.image(Lens::A), twice.lens(Lens::A));
    }

    #[test]
    fn base_support_masks_are_intersected_and_copied_to_both_slots() {
        let nodes = ROWS * COLS;
        let mut map_a = vec![[0.5, 0.5]; nodes];
        let mut map_b = vec![[0.5, 0.5]; nodes];
        let only_a_invalid = 100 * COLS + 20;
        let only_b_invalid = 200 * COLS + 30;
        map_a[only_a_invalid] = [0.0, 0.5];
        map_b[only_b_invalid] = [0.5, f32::NAN];
        let base_maps = RetainedBaseMaps::from_lenses(LensPair { a: map_a, b: map_b }).unwrap();
        let staging = SourceBelts::from_lenses(LensPair {
            a: vec![0; SourceBelts::BYTES / 2],
            b: vec![0; SourceBelts::BYTES / 2],
        })
        .unwrap();

        let inputs = ColdInputs::from_staging_and_base_support(&staging, &base_maps);

        assert_eq!(inputs.mask(Lens::A), inputs.mask(Lens::B));
        assert_eq!(inputs.mask(Lens::A)[only_a_invalid], 0);
        assert_eq!(inputs.mask(Lens::A)[only_b_invalid], 0);
    }

    #[test]
    fn mask_pyramid_uses_recursive_zero_based_nearest_samples() {
        let nodes = ROWS * COLS;
        let mut mask = vec![0; nodes];
        mask[0] = 11;
        mask[COLS + 1] = 22;
        mask[2 * COLS + 2] = 33;
        mask[4 * COLS + 4] = 44;
        let inputs = ColdInputs::from_prepared(
            LensPair {
                a: vec![0; nodes],
                b: vec![0; nodes],
            },
            LensPair {
                a: mask.clone(),
                b: mask,
            },
        )
        .unwrap();

        let pyramid = MaskPyramid::build(&inputs);

        assert_eq!(pyramid.level_one.a[0], 11);
        assert_eq!(pyramid.level_one.a[Level::One.cols() + 1], 33);
        assert_eq!(pyramid.level_two.a[0], 11);
        assert_eq!(pyramid.level_two.a[Level::Two.cols() + 1], 44);
        assert_eq!(pyramid.level_one.a, pyramid.level_one.b);
        assert_eq!(pyramid.level_two.a, pyramid.level_two.b);
    }

    #[test]
    fn direction_type_alone_selects_the_gradient_source_image() {
        let nodes = ROWS * COLS;
        let image_a = (0..ROWS)
            .flat_map(|_| (0..COLS).map(|col| (col * 3) as u8))
            .collect::<Vec<_>>();
        let image_b = (0..ROWS)
            .flat_map(|row| (0..COLS).map(move |_| (row % 200) as u8))
            .collect::<Vec<_>>();
        let inputs = ColdInputs::from_prepared(
            LensPair {
                a: image_a,
                b: image_b,
            },
            LensPair {
                a: vec![255; nodes],
                b: vec![255; nodes],
            },
        )
        .unwrap();
        let masks = MaskPyramid::build(&inputs);

        let a_to_b = LevelInputs::build::<AtoB>(&inputs, &masks, Level::Two);
        let b_to_a = LevelInputs::build::<BtoA>(&inputs, &masks, Level::Two);
        let interior = 5 * Level::Two.cols() + 5;

        assert!(a_to_b.gradient_col[interior] > 0.0);
        assert_eq!(a_to_b.gradient_row[interior], 0.0);
        assert_eq!(b_to_a.gradient_col[interior], 0.0);
        assert!(b_to_a.gradient_row[interior] > 0.0);
    }

    #[test]
    fn cold_direction_parallelism_is_bit_exact_against_asymmetric_serial_reference() {
        let pixels = ROWS * COLS;
        let image_a = (0..pixels)
            .map(|index| {
                let row = index / COLS;
                let col = index % COLS;
                ((row * 17 + col * 29 + row * col * 3) & 0xff) as u8
            })
            .collect::<Vec<_>>();
        let image_b = (0..pixels)
            .map(|index| {
                let row = index / COLS;
                let col = index % COLS;
                ((row * 43 + col * 7 + (row ^ col) * 11 + 19) & 0xff) as u8
            })
            .collect::<Vec<_>>();
        let mask_a = (0..pixels)
            .map(|index| {
                let row = index / COLS;
                let col = index % COLS;
                if (row + 2 * col) % 37 < 3 { 0 } else { 255 }
            })
            .collect::<Vec<_>>();
        let mask_b = (0..pixels)
            .map(|index| {
                let row = index / COLS;
                let col = index % COLS;
                if (3 * row + col + 5) % 41 < 4 { 0 } else { 255 }
            })
            .collect::<Vec<_>>();
        let inputs = ColdInputs::from_prepared(
            LensPair {
                a: image_a,
                b: image_b,
            },
            LensPair {
                a: mask_a,
                b: mask_b,
            },
        )
        .unwrap();

        let expected = serial_cold_transition(&inputs);
        let actual = ColdPair::new().transition(&inputs);
        assert_f32_bits_eq(
            actual.estimate.displacement.planes(),
            expected.estimate.displacement.planes(),
            "composed displacement",
        );
        assert_eq!(
            actual.estimate.invalid_nodes,
            expected.estimate.invalid_nodes
        );
        assert_eq!(
            actual.estimate.weighted_rows,
            expected.estimate.weighted_rows
        );

        let actual = actual.candidate_next;
        let expected = expected.candidate_next;
        assert_eq!(actual.references, expected.references);
        assert_public_bits_eq(
            &actual.a_to_b_public,
            &expected.a_to_b_public,
            "A-to-B public",
        );
        assert_public_bits_eq(
            &actual.b_to_a_public,
            &expected.b_to_a_public,
            "B-to-A public",
        );
        assert_median_bits_eq(
            &actual.a_to_b_median,
            &expected.a_to_b_median,
            "A-to-B median",
        );
        assert_median_bits_eq(
            &actual.b_to_a_median,
            &expected.b_to_a_median,
            "B-to-A median",
        );
        assert_eq!(actual.a_to_b_work_rows, expected.a_to_b_work_rows);
        assert_eq!(actual.b_to_a_work_rows, expected.b_to_a_work_rows);
        assert_hint_bits_eq(&actual.a_to_b_hints, &expected.a_to_b_hints, "A-to-B hints");
        assert_hint_bits_eq(&actual.b_to_a_hints, &expected.b_to_a_hints, "B-to-A hints");
        assert_eq!(actual.a_to_b_cadence, expected.a_to_b_cadence);
        assert_eq!(actual.b_to_a_cadence, expected.b_to_a_cadence);
    }

    #[test]
    fn cold_transition_folds_three_calls_into_one_candidate_state() {
        use crate::flow::one_xs::warm::WarmPair;

        fn assert_periodic_pairs<D: PisDirection>(fields: &PublicDenseField<D>) {
            for col in 0..COLS {
                for (row, partner) in [(51, 1022), (52, 1023), (53, 1024), (54, 1025), (55, 1026)] {
                    let first = row * COLS + col;
                    let second = partner * COLS + col;
                    assert_eq!(
                        fields.dcol()[first].to_bits(),
                        fields.dcol()[second].to_bits()
                    );
                    assert_eq!(
                        fields.drow()[first].to_bits(),
                        fields.drow()[second].to_bits()
                    );
                }
            }
        }

        fn assert_three_samples_per_patch<D: PisDirection>(median: &MedianState<D>) {
            assert!(
                median
                    .offsets()
                    .windows(2)
                    .all(|offsets| offsets[1] - offsets[0] == 3),
                "each finest patch must retain all three inner cold samples",
            );
            assert_eq!(median.values().len(), Level::One.patches() * 3);
            assert_eq!(
                median.offsets().last().copied(),
                Some(median.values().len() as u32),
            );
        }

        let pixels = ROWS * COLS;
        let inputs = ColdInputs::from_prepared(
            LensPair {
                a: vec![96; pixels],
                b: vec![96; pixels],
            },
            LensPair {
                a: vec![255; pixels],
                b: vec![255; pixels],
            },
        )
        .unwrap();
        let ColdTransition {
            estimate,
            candidate_next,
        } = ColdPair::new().transition(&inputs);

        assert_eq!(
            estimate.invalid_nodes,
            InvalidNodeCounts {
                a_to_b: 0,
                b_to_a: 0,
            },
        );
        assert!(
            estimate
                .displacement
                .planes()
                .iter()
                .all(|value| value.is_finite())
        );
        assert_eq!(
            estimate.weighted_rows,
            WorkRowCounts {
                a_to_b_l2: Level::Two.patch_rows(),
                b_to_a_l2: Level::Two.patch_rows(),
                a_to_b_l1: Level::One.patch_rows(),
                b_to_a_l1: Level::One.patch_rows(),
            },
        );

        assert!(
            candidate_next
                .references
                .bytes()
                .iter()
                .all(|value| *value == 96),
            "a normal cold return must clone the current blurred references",
        );
        for cadence in [candidate_next.a_to_b_cadence, candidate_next.b_to_a_cadence] {
            assert_eq!(cadence.calc_count(), COLD_INNER_CALCULATIONS as i32);
            assert_eq!(cadence.cadence(), COLD_EMPTY_OVERRIDE_CADENCE);
        }
        for rows in [
            candidate_next.a_to_b_work_rows.lack_of_texture_rows(),
            candidate_next.b_to_a_work_rows.lack_of_texture_rows(),
        ] {
            assert!(rows.iter().all(|row| *row));
        }
        assert!(
            candidate_next
                .a_to_b_work_rows
                .small_disparity_rows()
                .is_none()
        );
        assert!(
            candidate_next
                .b_to_a_work_rows
                .small_disparity_rows()
                .is_none()
        );
        for hints in [
            &candidate_next.a_to_b_hints.level(Level::One),
            &candidate_next.b_to_a_hints.level(Level::One),
        ] {
            assert_eq!(hints.dcol().len(), Level::One.pixels());
            assert_eq!(hints.drow().len(), Level::One.pixels());
            assert!(
                hints
                    .dcol()
                    .iter()
                    .chain(hints.drow())
                    .all(|value| value.is_finite())
            );
        }
        assert_three_samples_per_patch(&candidate_next.a_to_b_median);
        assert_three_samples_per_patch(&candidate_next.b_to_a_median);
        assert_periodic_pairs(&candidate_next.a_to_b_public);
        assert_periodic_pairs(&candidate_next.b_to_a_public);

        let second_current = ColdInputs::from_prepared(
            LensPair {
                a: vec![106; pixels],
                b: vec![106; pixels],
            },
            LensPair {
                a: vec![255; pixels],
                b: vec![255; pixels],
            },
        )
        .unwrap();
        let second = WarmPair::new().transition(candidate_next.into_checkpoint(second_current));
        assert_eq!(second.known_next.a_to_b_cadence.calc_count(), 4);
        assert_eq!(second.known_next.b_to_a_cadence.calc_count(), 4);
        assert!(
            second
                .known_next
                .references
                .bytes()
                .iter()
                .all(|value| *value == 99),
            "the validation call must consume the cold reference and commit the READ 0.7/0.3 update",
        );
    }

    #[test]
    fn cold_periodic_blend_is_only_the_outer_public_finalizer() {
        let pixels = ROWS * COLS;
        let mut dcol = vec![0.0f32; pixels];
        let mut drow = vec![0.0f32; pixels];
        for col in 0..COLS {
            dcol[51 * COLS + col] = 2.0;
            dcol[1022 * COLS + col] = 6.0;
            drow[51 * COLS + col] = 10.0;
            drow[1022 * COLS + col] = 14.0;
        }
        let raw = PublicDenseField::<AtoB>::from_row_major_components(dcol, drow).unwrap();
        assert_ne!(raw.dcol()[51 * COLS], raw.dcol()[1022 * COLS]);
        assert_ne!(raw.drow()[51 * COLS], raw.drow()[1022 * COLS]);

        let finished = finish_cold_public(raw);

        for col in 0..COLS {
            assert_eq!(
                finished.dcol()[51 * COLS + col].to_bits(),
                finished.dcol()[1022 * COLS + col].to_bits(),
            );
            assert_eq!(
                finished.drow()[51 * COLS + col].to_bits(),
                finished.drow()[1022 * COLS + col].to_bits(),
            );
        }
    }
}
