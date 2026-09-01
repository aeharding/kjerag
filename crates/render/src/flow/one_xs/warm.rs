//! Readable composition of one ONE X2 warm transition.
//!
//! This module starts after Studio's selected input Gaussian. It computes the
//! current motion mask from the restored previous blurred belts, restores the
//! two direction-owned retained inputs, and then reuses the accepted scalar
//! stages in their native L2-to-L1 order. Captured motion, sparse results,
//! dense results, public outputs, and displacement are deliberately absent
//! from [`WarmCheckpointInputs`], so captured outputs cannot be fed back as
//! current inputs.
//!
//! The selected playback owner consumes this transition sequentially after the
//! cold frame. Detached checkpoint fixtures exercise the same code as an
//! oracle. It runs the separate masked `calcHintFlow` and retained-row updates,
//! then exports their next direction-owned state together with blurred
//! references, public fields, temporal medians, and cadence counters.

use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
#[cfg(test)]
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use super::dense::{self, PublicDenseField};
use super::l2_seed::into_l1_initial_grid;
#[cfg(test)]
use super::pis;
use super::pis::{
    AtoB, BtoA, CostMode, DescentAdmission, Flow, HintGrid, InitialGrid, Level, PatchGrid,
    PisDirection,
};
use super::post_update::{
    MotionPyramid, RetainedPublicPyramids, update_without_variational_with_retained,
};
use super::public_blend::blend_periodic_boundary;
use super::scalar::{
    ColdInputs, ColdPreparedSchedule, DirectionSolveRequest, PairSolveError, PairSolveStage,
    PairedControlInputs, PairedPisSolver, PairedSolveRequest, PreparedLevelImages, WorkRowCounts,
    propagate_work_modes, solve_pair, weighted_rows,
};
#[cfg(test)]
use super::scalar::{LevelInputs, MaskPyramid};
use super::temporal::{BlurredBelts, MotionMask, next_warm_references};
use super::temporal_median::{DirectedPatchGrids, FilteredPatchGrid, MedianState, TemporalMedians};
use super::{
    COLS, DirectedFields, Direction, Displacement, InvalidNodeCounts, PATCH_SIZE, PATCH_STRIDE,
};

const VECTOR_BOOL_WORD_BITS: usize = 64;
const VECTOR_BOOL_WORD_BYTES: usize = VECTOR_BOOL_WORD_BITS / 8;

/// A captured warm-checkpoint input failed its external restore contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarmRestoreError {
    WorkRowByteCount {
        direction: Direction,
        level: Level,
        actual: usize,
        expected: usize,
    },
    NonCanonicalWorkRowPadding {
        direction: Direction,
        level: Level,
        bit: usize,
        logical_bits: usize,
    },
    HintPlaneShape {
        direction: Direction,
        level: Level,
        component: &'static str,
        actual: usize,
        expected: usize,
    },
    NonFiniteHint {
        direction: Direction,
        level: Level,
        component: &'static str,
        index: usize,
    },
    ZeroCadence,
}

impl fmt::Display for WarmRestoreError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkRowByteCount {
                direction,
                level,
                actual,
                expected,
            } => write!(
                out,
                "ONE X2 warm {direction} {level} work rows have {actual} bytes, expected {expected}",
            ),
            Self::NonCanonicalWorkRowPadding {
                direction,
                level,
                bit,
                logical_bits,
            } => write!(
                out,
                "ONE X2 warm {direction} {level} work-row padding bit {bit} is set; bits from {logical_bits} onward must be zero",
            ),
            Self::HintPlaneShape {
                direction,
                level,
                component,
                actual,
                expected,
            } => write!(
                out,
                "ONE X2 warm {direction} {level} hint {component} plane has {actual} values, expected {expected}",
            ),
            Self::NonFiniteHint {
                direction,
                level,
                component,
                index,
            } => write!(
                out,
                "ONE X2 warm {direction} {level} hint {component} value {index} is not finite",
            ),
            Self::ZeroCadence => {
                out.write_str("ONE X2 warm empty-override cadence must not be zero")
            }
        }
    }
}

impl Error for WarmRestoreError {}

/// Direction-labelled final `d8` work rows consumed by PIS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveWorkRows<D: PisDirection> {
    level_one: Box<[CostMode]>,
    level_two: Box<[CostMode]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> EffectiveWorkRows<D> {
    /// Decode word-backed `vector<bool>` storage in exact LSB0 order.
    ///
    /// L1 has 178 logical rows in 24 bytes; L2 has 88 logical rows in 16
    /// bytes. Every bit in each rounded 64-bit tail must be zero.
    pub fn from_lsb0_bytes(level_one: &[u8], level_two: &[u8]) -> Result<Self, WarmRestoreError> {
        Ok(Self {
            level_one: decode_effective_work_rows::<D>(Level::One, level_one)?.into_boxed_slice(),
            level_two: decode_effective_work_rows::<D>(Level::Two, level_two)?.into_boxed_slice(),
            direction: PhantomData,
        })
    }

    pub fn modes(&self, level: Level) -> &[CostMode] {
        match level {
            Level::One => &self.level_one,
            Level::Two => &self.level_two,
        }
    }

    fn from_raw(small: Option<&[bool]>, lack: &[bool]) -> Self {
        if let Some(small) = small {
            assert_eq!(small.len(), Level::One.patch_rows());
        }
        assert_eq!(lack.len(), Level::One.patch_rows());
        let level_one = lack
            .iter()
            .enumerate()
            .map(|(row, lack)| {
                if small.is_some_and(|small| small[row]) || *lack {
                    CostMode::Weighted
                } else {
                    CostMode::Unweighted
                }
            })
            .collect::<Vec<_>>();
        let level_two = propagate_work_modes(&level_one);
        Self {
            level_one: level_one.into_boxed_slice(),
            level_two: level_two.into_boxed_slice(),
            direction: PhantomData,
        }
    }
}

/// Raw direction-owned row state retained between native calculations.
///
/// `small_disparity` is `FDS+0x108`; `None` preserves native's absent
/// `vector<bool>` state and is distinct from a present all-zero vector.
/// `lack_of_texture` is the finest `FDS+0x120` vector. Native derives the
/// effective `d8` rows consumed by PIS from these two values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainedWorkRows<D: PisDirection> {
    small_disparity: Option<Box<[bool]>>,
    lack_of_texture: Box<[bool]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> RetainedWorkRows<D> {
    /// Restore exact canonical LSB0 row vectors from a checkpoint.
    ///
    /// `None` means native's small-disparity vector is absent. A present
    /// vector, including an all-zero one, must contain the canonical 24 bytes.
    pub fn from_lsb0_bytes(
        small_disparity: Option<&[u8]>,
        lack_of_texture: &[u8],
    ) -> Result<Self, WarmRestoreError> {
        Ok(Self {
            small_disparity: small_disparity
                .map(decode_bool_rows::<D>)
                .transpose()?
                .map(Vec::into_boxed_slice),
            lack_of_texture: decode_bool_rows::<D>(lack_of_texture)?.into_boxed_slice(),
            direction: PhantomData,
        })
    }

    /// Derive the exact current L1/L2 `d8` inputs from retained raw state.
    pub fn effective(&self) -> EffectiveWorkRows<D> {
        EffectiveWorkRows::from_raw(self.small_disparity.as_deref(), &self.lack_of_texture)
    }

    pub fn small_disparity_rows(&self) -> Option<&[bool]> {
        self.small_disparity.as_deref()
    }

    pub fn lack_of_texture_rows(&self) -> &[bool] {
        &self.lack_of_texture
    }

    /// Construct the raw row state prepared by the first inner cold
    /// calculation and retained by its remaining inner calculations.
    ///
    /// The first calculation derives `FDS+0x120` from its finest prepared
    /// texture before PIS. The optional `FDS+0x108` owner is still absent: its
    /// writer does not become eligible until pre-increment count three.
    #[cfg(test)]
    pub(super) fn after_cold_calc(inputs: &LevelInputs) -> Self {
        let lack = inputs.lack_rows::<D>(Level::One);
        Self::after_cold_lack(&lack)
    }

    pub(super) fn after_cold_lack(lack: &super::dense::LackRows<D>) -> Self {
        Self {
            small_disparity: None,
            lack_of_texture: lack.rows().to_vec().into_boxed_slice(),
            direction: PhantomData,
        }
    }

    fn after_warm_calc(&self, small: &SharedSmallDisparityRows) -> Self {
        Self {
            small_disparity: small.rows.clone(),
            lack_of_texture: self.lack_of_texture.clone(),
            direction: PhantomData,
        }
    }

    #[cfg(test)]
    fn prepare_lack_on_first_calc(&self, inputs: &LevelInputs, calc_count: i32) -> Self {
        let lack = inputs.lack_rows::<D>(Level::One);
        self.prepare_lack_rows_on_first_calc(&lack, calc_count)
    }

    fn prepare_lack_rows_on_first_calc(
        &self,
        lack: &super::dense::LackRows<D>,
        calc_count: i32,
    ) -> Self {
        if calc_count != 0 {
            return self.clone();
        }
        Self {
            small_disparity: self.small_disparity.clone(),
            lack_of_texture: lack.rows().to_vec().into_boxed_slice(),
            direction: PhantomData,
        }
    }
}

/// Direction-owned `FDS+0x108` rows produced from post-temporal sparse U.
#[derive(Debug, PartialEq, Eq)]
struct SmallDisparityRows<D: PisDirection> {
    rows: Option<Box<[bool]>>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> SmallDisparityRows<D> {
    fn refresh(
        retained: &RetainedWorkRows<D>,
        filtered: &FilteredPatchGrid<D>,
        block_mask: &[u8],
        calc_count: i32,
    ) -> Self {
        let fresh = (calc_count >= SMALL_DISPARITY_FIRST_CALC_COUNT)
            .then(|| classify_small_disparity_rows(filtered.dcol(), block_mask));
        Self::retain_or_replace(retained, fresh, calc_count)
    }

    fn retain_or_replace(
        retained: &RetainedWorkRows<D>,
        fresh: Option<Box<[bool]>>,
        calc_count: i32,
    ) -> Self {
        Self {
            rows: if calc_count >= SMALL_DISPARITY_FIRST_CALC_COUNT {
                Some(fresh.expect("mature small-disparity updates have a fresh result"))
            } else {
                retained.small_disparity.clone()
            },
            direction: PhantomData,
        }
    }
}

/// Bilateral union written back to both native `FDS+0x108` row owners.
#[derive(Debug, PartialEq, Eq)]
struct SharedSmallDisparityRows {
    rows: Option<Box<[bool]>>,
}

impl SharedSmallDisparityRows {
    fn merge(a_to_b: &SmallDisparityRows<AtoB>, b_to_a: &SmallDisparityRows<BtoA>) -> Self {
        for rows in [a_to_b.rows.as_deref(), b_to_a.rows.as_deref()]
            .into_iter()
            .flatten()
        {
            assert_eq!(rows.len(), Level::One.patch_rows());
        }
        Self {
            rows: if a_to_b.rows.is_none() && b_to_a.rows.is_none() {
                None
            } else {
                Some(
                    (0..Level::One.patch_rows())
                        .map(|row| {
                            a_to_b.rows.as_ref().is_some_and(|rows| rows[row])
                                || b_to_a.rows.as_ref().is_some_and(|rows| rows[row])
                        })
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                )
            },
        }
    }
}

/// Selected normal-arm threshold: signed original solver-input columns / 12,
/// converted to binary32 after the integer division. The ONE X2 belt has 60
/// columns; native reads that input Mat header rather than the L1 pyramid.
const SMALL_DISPARITY_MEAN_THRESHOLD: f32 = (COLS as i32 / 12) as f32;
const SMALL_DISPARITY_FIRST_CALC_COUNT: i32 = 3;

fn classify_small_disparity_rows(dcol: &[f32], block_mask: &[u8]) -> Box<[bool]> {
    assert_eq!(dcol.len(), Level::One.patches());
    assert_eq!(block_mask.len(), Level::One.patches());
    let mut rows = Vec::with_capacity(Level::One.patch_rows());
    for patch_row in 0..Level::One.patch_rows() {
        let first = patch_row * Level::One.patch_cols();
        let mut sum = 0.0f32;
        let mut count = 0i32;
        for patch_col in 0..Level::One.patch_cols() {
            let patch = first + patch_col;
            if block_mask[patch] != 0 {
                sum += dcol[patch].abs();
                count += 1;
            }
        }
        let mean = if count == 0 {
            0.0f32
        } else {
            sum / count as f32
        };
        rows.push(mean < SMALL_DISPARITY_MEAN_THRESHOLD);
    }
    rows.into_boxed_slice()
}

fn decode_effective_work_rows<D: PisDirection>(
    level: Level,
    bytes: &[u8],
) -> Result<Vec<CostMode>, WarmRestoreError> {
    let logical_bits = level.patch_rows();
    let expected = logical_bits.div_ceil(VECTOR_BOOL_WORD_BITS) * VECTOR_BOOL_WORD_BYTES;
    if bytes.len() != expected {
        return Err(WarmRestoreError::WorkRowByteCount {
            direction: D::DIRECTION,
            level,
            actual: bytes.len(),
            expected,
        });
    }

    for bit in logical_bits..bytes.len() * 8 {
        if bytes[bit / 8] & (1u8 << (bit % 8)) != 0 {
            return Err(WarmRestoreError::NonCanonicalWorkRowPadding {
                direction: D::DIRECTION,
                level,
                bit,
                logical_bits,
            });
        }
    }

    Ok((0..logical_bits)
        .map(|bit| {
            if bytes[bit / 8] & (1u8 << (bit % 8)) == 0 {
                CostMode::Unweighted
            } else {
                CostMode::Weighted
            }
        })
        .collect())
}

fn decode_bool_rows<D: PisDirection>(bytes: &[u8]) -> Result<Vec<bool>, WarmRestoreError> {
    decode_effective_work_rows::<D>(Level::One, bytes).map(|rows| {
        rows.into_iter()
            .map(|mode| mode == CostMode::Weighted)
            .collect()
    })
}

/// One named row-major dense hint level before shape and finite validation.
#[derive(Clone, Debug, PartialEq)]
pub struct DenseHintLevel {
    dcol: Vec<f32>,
    drow: Vec<f32>,
}

impl DenseHintLevel {
    pub fn from_row_major_components(dcol: Vec<f32>, drow: Vec<f32>) -> Self {
        Self { dcol, drow }
    }

    pub fn dcol(&self) -> &[f32] {
        &self.dcol
    }

    pub fn drow(&self) -> &[f32] {
        &self.drow
    }
}

/// Direction-labelled dense auxiliary hints at both selected solver levels.
#[derive(Clone, Debug, PartialEq)]
pub struct HintPyramid<D: PisDirection> {
    level_one: DenseHintLevel,
    level_two: DenseHintLevel,
    direction: PhantomData<D>,
}

impl<D: PisDirection> HintPyramid<D> {
    /// Restore complete dense L1 and L2 hint planes.
    ///
    /// Every value is validated, including pixels that are not sparse patch
    /// centres, so corrupt captured state cannot hide outside the samples.
    pub fn from_levels(
        level_one: DenseHintLevel,
        level_two: DenseHintLevel,
    ) -> Result<Self, WarmRestoreError> {
        validate_hint_level::<D>(Level::One, &level_one)?;
        validate_hint_level::<D>(Level::Two, &level_two)?;
        Ok(Self {
            level_one,
            level_two,
            direction: PhantomData,
        })
    }

    pub(super) fn grid(&self, level: Level) -> HintGrid<D> {
        let dense = self.level(level);
        let mut flows = Vec::with_capacity(level.patches());
        for patch_row in 0..level.patch_rows() {
            let dense_row = patch_row * PATCH_STRIDE + PATCH_SIZE / 2;
            for patch_col in 0..level.patch_cols() {
                let dense_col = patch_col * PATCH_STRIDE + PATCH_SIZE / 2;
                let index = dense_row * level.cols() + dense_col;
                flows.push(
                    Flow::new(dense.dcol[index], dense.drow[index])
                        .expect("validated warm dense hint centre became non-finite"),
                );
            }
        }
        HintGrid::from_row_major(level, flows)
            .expect("warm dense hints produce the selected sparse patch count")
    }

    pub fn level(&self, level: Level) -> &DenseHintLevel {
        match level {
            Level::One => &self.level_one,
            Level::Two => &self.level_two,
        }
    }

    /// Materialize the present zero-valued hints prepared for the first cold
    /// calculation.
    ///
    /// Native distinguishes these allocated zero planes from an absent hint;
    /// both selected levels therefore enter PIS through `Some(zero_hint)`.
    pub(super) fn cold_zeros() -> Self {
        let zeros = |level: Level| {
            DenseHintLevel::from_row_major_components(
                vec![0.0f32; level.pixels()],
                vec![0.0f32; level.pixels()],
            )
        };
        Self::from_levels(zeros(Level::One), zeros(Level::Two))
            .expect("selected cold zero hints have finite exact-shape planes")
    }

    /// Consume the incoming auxiliary hints after their current PIS use and
    /// construct the hints for the next call from the raw finest sparse grid.
    ///
    /// Studio retains the Mat allocations but zeroes every payload before the
    /// masked writes. Rust may replace those allocations, which is an internal
    /// ownership difference with identical values.
    fn replace_from_current_finest(
        &self,
        images: &dense::DirectedImages<'_, D>,
        patches: &PatchGrid<D>,
    ) -> Self {
        Self::from_current_finest(images, patches)
    }

    /// Construct the next auxiliary hints after a calculation.
    ///
    /// Cold and warm calls share the same post-PIS `calcHintFlow` producer.
    /// The first cold calculation consumes present zero-valued hints; its
    /// result becomes the incoming state for the next inner calculation.
    pub(super) fn from_current_finest(
        images: &dense::DirectedImages<'_, D>,
        patches: &PatchGrid<D>,
    ) -> Self {
        let finest = dense::densify_hint(images, patches)
            .expect("typed warm finest images and raw patches share the hint level");
        let (dcol, drow) = finest.into_components();
        let level_one = DenseHintLevel::from_row_major_components(dcol.into_vec(), drow.into_vec());
        let level_two = propagate_hint_level(&level_one);
        Self::from_levels(level_one, level_two)
            .expect("selected masked hint construction produces finite exact-shape planes")
    }
}

fn propagate_hint_level(level_one: &DenseHintLevel) -> DenseHintLevel {
    let mut dcol = vec![0.0f32; Level::Two.pixels()];
    let mut drow = vec![0.0f32; Level::Two.pixels()];
    for patch_row in 0..Level::Two.patch_rows() {
        let row = patch_row * PATCH_STRIDE + PATCH_SIZE / 2;
        for patch_col in 0..Level::Two.patch_cols() {
            let col = patch_col * PATCH_STRIDE + PATCH_SIZE / 2;
            let at = row * Level::Two.cols() + col;
            dcol[at] = propagated_hint_value(&level_one.dcol, row, col);
            drow[at] = propagated_hint_value(&level_one.drow, row, col);
        }
    }
    DenseHintLevel::from_row_major_components(dcol, drow)
}

fn propagated_hint_value(source: &[f32], row: usize, col: usize) -> f32 {
    debug_assert_eq!(source.len(), Level::One.pixels());
    let row0 = (2 * row).min(Level::One.rows() - 1);
    let row1 = (2 * row + 1).min(Level::One.rows() - 1);
    let col0 = (2 * col).min(Level::One.cols() - 1);
    let col1 = (2 * col + 1).min(Level::One.cols() - 1);
    let x00 = source[row0 * Level::One.cols() + col0];
    let x01 = source[row0 * Level::One.cols() + col1];
    let x10 = source[row1 * Level::One.cols() + col0];
    let x11 = source[row1 * Level::One.cols() + col1];
    let sum0 = x00 + 0.0f32;
    let sum1 = sum0 + x01;
    let sum2 = sum1 + x10;
    let sum3 = sum2 + x11;
    let quarter = sum3 * 0.25f32;
    quarter * 0.5f32
}

fn validate_hint_level<D: PisDirection>(
    level: Level,
    dense: &DenseHintLevel,
) -> Result<(), WarmRestoreError> {
    let expected = level.pixels();
    for (component, values) in [("dcol", &dense.dcol), ("drow", &dense.drow)] {
        if values.len() != expected {
            return Err(WarmRestoreError::HintPlaneShape {
                direction: D::DIRECTION,
                level,
                component,
                actual: values.len(),
                expected,
            });
        }
        if let Some(index) = values.iter().position(|value| !value.is_finite()) {
            return Err(WarmRestoreError::NonFiniteHint {
                direction: D::DIRECTION,
                level,
                component,
                index,
            });
        }
    }
    Ok(())
}

/// Authenticated empty-override cadence inputs shared by one direction's levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmptyOverrideCadence {
    calc_count: i32,
    cadence: i32,
}

impl EmptyOverrideCadence {
    pub fn new(calc_count: i32, cadence: i32) -> Result<Self, WarmRestoreError> {
        if cadence == 0 {
            return Err(WarmRestoreError::ZeroCadence);
        }
        Ok(Self {
            calc_count,
            cadence,
        })
    }

    pub const fn calc_count(self) -> i32 {
        self.calc_count
    }

    pub const fn cadence(self) -> i32 {
        self.cadence
    }

    pub(super) fn admission(self) -> DescentAdmission {
        DescentAdmission::from_empty_override_cadence(self.calc_count, self.cadence)
    }

    /// Advance the native 32-bit calculation counter once after both levels.
    ///
    /// ARM's `add w8, w8, #1` wraps modulo 2^32. The selected target advances
    /// from 6372 to 6373 after L2 and L1 have both consumed 6372.
    pub(super) fn after_calc(self) -> Self {
        Self {
            calc_count: self.calc_count.wrapping_add(1),
            cadence: self.cadence,
        }
    }
}

/// All restored warm state owned by one semantic solver direction.
///
/// Direction labels make a prior public field, retained work rows, and dense
/// auxiliary hints nonexchangeable at compile time.
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::dense::PublicDenseField;
/// use kjerag_render::flow::one_xs::pis::{AtoB, BtoA};
/// use kjerag_render::flow::one_xs::warm::{
///     EmptyOverrideCadence, HintPyramid, RetainedWorkRows, WarmDirection,
/// };
///
/// fn swapped(
///     public: PublicDenseField<AtoB>,
///     rows: RetainedWorkRows<BtoA>,
///     hints: HintPyramid<AtoB>,
///     cadence: EmptyOverrideCadence,
/// ) {
///     let _ = WarmDirection::<AtoB>::new(public, rows, hints, cadence);
/// }
/// ```
#[derive(Debug)]
pub struct WarmDirection<D: PisDirection> {
    prior_public: PublicDenseField<D>,
    work_rows: RetainedWorkRows<D>,
    hints: HintPyramid<D>,
    cadence: EmptyOverrideCadence,
}

impl<D: PisDirection> WarmDirection<D> {
    pub fn new(
        prior_public: PublicDenseField<D>,
        work_rows: RetainedWorkRows<D>,
        hints: HintPyramid<D>,
        cadence: EmptyOverrideCadence,
    ) -> Self {
        Self {
            prior_public,
            work_rows,
            hints,
            cadence,
        }
    }
}

/// Complete semantic pre-state for one target warm composition.
///
/// `current_post_blur` owns [`ColdInputs`] whose images have already crossed
/// the selected Gaussian. `prior_references` is the preceding blurred A/B
/// pair used only to derive current motion. The two FDS directions and paired
/// temporal medians are consumed together.
pub struct WarmCheckpointInputs {
    current_post_blur: ColdInputs,
    retained: WarmRetainedInputs,
}

pub struct WarmRetainedInputs {
    prior_references: BlurredBelts,
    a_to_b: WarmDirection<AtoB>,
    b_to_a: WarmDirection<BtoA>,
    a_to_b_median: MedianState<AtoB>,
    b_to_a_median: MedianState<BtoA>,
}

impl WarmCheckpointInputs {
    pub fn new(
        current_post_blur: ColdInputs,
        prior_references: BlurredBelts,
        a_to_b: WarmDirection<AtoB>,
        b_to_a: WarmDirection<BtoA>,
        temporal_medians: TemporalMedians,
    ) -> Self {
        let (a_to_b_median, b_to_a_median) = temporal_medians.into_states();
        Self {
            current_post_blur,
            retained: WarmRetainedInputs {
                prior_references,
                a_to_b,
                b_to_a,
                a_to_b_median,
                b_to_a_median,
            },
        }
    }
}

impl WarmRetainedInputs {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_borrowed_state(
        prior_references: &BlurredBelts,
        a_to_b_public: &PublicDenseField<AtoB>,
        b_to_a_public: &PublicDenseField<BtoA>,
        a_to_b_rows: &RetainedWorkRows<AtoB>,
        b_to_a_rows: &RetainedWorkRows<BtoA>,
        a_to_b_hints: &HintPyramid<AtoB>,
        b_to_a_hints: &HintPyramid<BtoA>,
        a_to_b_cadence: EmptyOverrideCadence,
        b_to_a_cadence: EmptyOverrideCadence,
        a_to_b_median: &MedianState<AtoB>,
        b_to_a_median: &MedianState<BtoA>,
    ) -> Self {
        Self {
            prior_references: prior_references.clone(),
            a_to_b: WarmDirection::new(
                copy_public(a_to_b_public),
                a_to_b_rows.clone(),
                a_to_b_hints.clone(),
                a_to_b_cadence,
            ),
            b_to_a: WarmDirection::new(
                copy_public(b_to_a_public),
                b_to_a_rows.clone(),
                b_to_a_hints.clone(),
                b_to_a_cadence,
            ),
            a_to_b_median: a_to_b_median.clone(),
            b_to_a_median: b_to_a_median.clone(),
        }
    }
}

impl std::ops::Deref for WarmCheckpointInputs {
    type Target = WarmRetainedInputs;

    fn deref(&self) -> &Self::Target {
        &self.retained
    }
}

fn copy_public<D: PisDirection>(field: &PublicDenseField<D>) -> PublicDenseField<D> {
    PublicDenseField::from_row_major_components(field.dcol().to_vec(), field.drow().to_vec())
        .expect("borrowed public field retains the selected shape")
}

/// One complete warm pair result.
#[derive(Clone, Debug, PartialEq)]
pub struct WarmEstimate {
    pub displacement: Displacement,
    pub invalid_nodes: InvalidNodeCounts,
    pub weighted_rows: WorkRowCounts,
}

/// The next-frame state produced by one warm transaction.
#[must_use = "the ONE X2 warm next state has not been retained or checked"]
#[derive(Debug)]
pub struct KnownWarmNext {
    pub references: BlurredBelts,
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

impl KnownWarmNext {
    pub(super) fn retained_checkpoint_from_borrowed(&self) -> WarmRetainedInputs {
        WarmRetainedInputs::from_borrowed_state(
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

    /// Consume this paired state with one already aligned next blurred input.
    ///
    /// This preserves semantic ownership only. The caller remains responsible
    /// for proving that `current_post_blur` is the adjacent decoded delivery.
    ///
    /// ```compile_fail
    /// use kjerag_render::flow::one_xs::scalar::ColdInputs;
    /// use kjerag_render::flow::one_xs::warm::KnownWarmNext;
    ///
    /// fn reuse(next: KnownWarmNext, first: ColdInputs, second: ColdInputs) {
    ///     let _ = next.into_checkpoint(first);
    ///     let _ = next.into_checkpoint(second);
    /// }
    /// ```
    pub fn into_checkpoint(self, current_post_blur: ColdInputs) -> WarmCheckpointInputs {
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
        let a_to_b = WarmDirection::new(
            a_to_b_public,
            a_to_b_work_rows,
            a_to_b_hints,
            a_to_b_cadence,
        );
        let b_to_a = WarmDirection::new(
            b_to_a_public,
            b_to_a_work_rows,
            b_to_a_hints,
            b_to_a_cadence,
        );
        let temporal_medians = TemporalMedians::from_states(a_to_b_median, b_to_a_median)
            .expect("a produced ONE X2 warm median state must restore");
        WarmCheckpointInputs::new(
            current_post_blur,
            references,
            a_to_b,
            b_to_a,
            temporal_medians,
        )
    }
}

/// One checkpoint result paired with every exact next-state component it
/// computes today.
#[must_use = "the ONE X2 warm transition has not been consumed"]
#[derive(Debug)]
pub struct WarmTransition {
    pub estimate: WarmEstimate,
    pub known_next: KnownWarmNext,
}

/// Stateless owner for one warm checkpoint composition.
#[derive(Clone, Copy, Debug, Default)]
pub struct WarmPair;

impl WarmPair {
    pub const fn new() -> Self {
        Self
    }

    /// Consume one complete warm checkpoint and compose its displacement.
    ///
    /// This compatibility entry discards the produced next-frame state.
    /// Sequential ownership should use [`Self::transition`] instead.
    pub fn estimate(self, inputs: WarmCheckpointInputs) -> WarmEstimate {
        self.transition(inputs).estimate
    }

    /// Compose one checkpoint and export the exact known next-state subset.
    pub fn transition(self, inputs: WarmCheckpointInputs) -> WarmTransition {
        let ColdPreparedSchedule {
            controls,
            mut solver,
        } = ColdPreparedSchedule::from_cpu(&inputs.current_post_blur);
        match staged_warm_prepared_transition(&inputs.retained, &controls, &mut solver) {
            Ok(transition) => transition,
            Err(PairSolveError::Solver { source, .. }) => match source {},
            Err(PairSolveError::Stamp { source, .. }) => {
                panic!("CPU paired solver returned its own invalid stamp: {source}")
            }
        }
    }

    pub(crate) fn try_transition_prepared_with_solver<S: PairedPisSolver>(
        self,
        retained: &WarmRetainedInputs,
        controls: &PairedControlInputs,
        solver: &mut S,
    ) -> Result<WarmTransition, PairSolveError<S::Error>> {
        staged_warm_prepared_transition(retained, controls, solver)
    }

    #[cfg(test)]
    fn transition_serial(self, inputs: WarmCheckpointInputs) -> WarmTransition {
        self.transition_with_schedule(inputs, DirectionSchedule::Serial)
    }

    #[cfg(test)]
    fn transition_with_schedule(
        self,
        inputs: WarmCheckpointInputs,
        schedule: DirectionSchedule,
    ) -> WarmTransition {
        let WarmCheckpointInputs {
            current_post_blur,
            retained:
                WarmRetainedInputs {
                    prior_references,
                    a_to_b,
                    b_to_a,
                    a_to_b_median,
                    b_to_a_median,
                },
        } = inputs;
        let mut temporal_medians = TemporalMedians::from_states(a_to_b_median, b_to_a_median)
            .expect("validated warm median checkpoint must restore");
        let WarmDirection {
            prior_public: a_to_b_prior,
            work_rows: a_to_b_rows,
            hints: a_to_b_incoming_hints,
            cadence: a_to_b_cadence,
        } = a_to_b;
        let WarmDirection {
            prior_public: b_to_a_prior,
            work_rows: b_to_a_rows,
            hints: b_to_a_incoming_hints,
            cadence: b_to_a_cadence,
        } = b_to_a;

        let current_belts = current_post_blur.blurred_belts();
        let motion = MotionMask::between(&current_belts, &prior_references);
        let motion = MotionPyramid::from_base(&motion);
        let masks = MaskPyramid::build(&current_post_blur);
        let a_to_b_retained = RetainedPublicPyramids::from_public(a_to_b_prior);
        let b_to_a_retained = RetainedPublicPyramids::from_public(b_to_a_prior);

        // Native resolves the empty-override cadence once per FDS call. The
        // counter is not incremented between its L2 and L1 solves.
        let a_to_b_admission = a_to_b_cadence.admission();
        let b_to_a_admission = b_to_a_cadence.admission();
        let a_to_b_finest_inputs =
            LevelInputs::build::<AtoB>(&current_post_blur, &masks, Level::One);
        let b_to_a_finest_inputs =
            LevelInputs::build::<BtoA>(&current_post_blur, &masks, Level::One);
        // Native prepares +0x120 before deriving d8 only when the calculation
        // counter is exactly zero. Every nonzero warm call retains it.
        let a_to_b_rows = a_to_b_rows
            .prepare_lack_on_first_calc(&a_to_b_finest_inputs, a_to_b_cadence.calc_count());
        let b_to_a_rows = b_to_a_rows
            .prepare_lack_on_first_calc(&b_to_a_finest_inputs, b_to_a_cadence.calc_count());
        let (a_to_b_pre, b_to_a_pre) = match schedule {
            DirectionSchedule::Parallel => std::thread::scope(|scope| {
                let b_to_a = scope.spawn(|| {
                    solve_pre_median::<BtoA>(
                        &current_post_blur,
                        &masks,
                        &b_to_a_retained,
                        &motion,
                        b_to_a_finest_inputs,
                        b_to_a_rows,
                        b_to_a_incoming_hints,
                        b_to_a_admission,
                    )
                });
                // Always join the sibling before crossing into paired median
                // state. Preserve the caller lane's original panic, with
                // A-to-B priority if both independent solves fail.
                let a_to_b = catch_unwind(AssertUnwindSafe(|| {
                    solve_pre_median::<AtoB>(
                        &current_post_blur,
                        &masks,
                        &a_to_b_retained,
                        &motion,
                        a_to_b_finest_inputs,
                        a_to_b_rows,
                        a_to_b_incoming_hints,
                        a_to_b_admission,
                    )
                }));
                let b_to_a = b_to_a.join();
                match (a_to_b, b_to_a) {
                    (Ok(a_to_b), Ok(b_to_a)) => (a_to_b, b_to_a),
                    (Err(a_to_b), _) => resume_unwind(a_to_b),
                    (Ok(_), Err(b_to_a)) => resume_unwind(b_to_a),
                }
            }),
            #[cfg(test)]
            DirectionSchedule::Serial => (
                solve_pre_median::<AtoB>(
                    &current_post_blur,
                    &masks,
                    &a_to_b_retained,
                    &motion,
                    a_to_b_finest_inputs,
                    a_to_b_rows,
                    a_to_b_incoming_hints,
                    a_to_b_admission,
                ),
                solve_pre_median::<BtoA>(
                    &current_post_blur,
                    &masks,
                    &b_to_a_retained,
                    &motion,
                    b_to_a_finest_inputs,
                    b_to_a_rows,
                    b_to_a_incoming_hints,
                    b_to_a_admission,
                ),
            ),
        };
        let DirectionPreMedian {
            finest_inputs: a_to_b_finest_inputs,
            rows: a_to_b_rows,
            grid: a_to_b_grid,
            block_mask: a_to_b_block_mask,
            next_hints: a_to_b_next_hints,
            l2_weighted: a_to_b_l2,
            l1_weighted: a_to_b_l1,
        } = a_to_b_pre;
        let DirectionPreMedian {
            finest_inputs: b_to_a_finest_inputs,
            rows: b_to_a_rows,
            grid: b_to_a_grid,
            block_mask: b_to_a_block_mask,
            next_hints: b_to_a_next_hints,
            l2_weighted: b_to_a_l2,
            l1_weighted: b_to_a_l1,
        } = b_to_a_pre;

        let filtered = temporal_medians
            .run(DirectedPatchGrids::new(a_to_b_grid, b_to_a_grid))
            .expect("warm paired finest grids have the temporal median level");
        let (a_to_b_filtered, b_to_a_filtered) = filtered.into_parts();
        let a_to_b_small = SmallDisparityRows::refresh(
            &a_to_b_rows,
            &a_to_b_filtered,
            &a_to_b_block_mask,
            a_to_b_cadence.calc_count(),
        );
        let b_to_a_small = SmallDisparityRows::refresh(
            &b_to_a_rows,
            &b_to_a_filtered,
            &b_to_a_block_mask,
            b_to_a_cadence.calc_count(),
        );
        let shared_small = SharedSmallDisparityRows::merge(&a_to_b_small, &b_to_a_small);
        let a_to_b_next_rows = a_to_b_rows.after_warm_calc(&shared_small);
        let b_to_a_next_rows = b_to_a_rows.after_warm_calc(&shared_small);

        let a_to_b = finish_direction::<AtoB>(
            &PreparedLevelImages::from_pis(&a_to_b_finest_inputs, Level::One),
            a_to_b_filtered,
            &a_to_b_retained,
            &motion,
        );
        let b_to_a = finish_direction::<BtoA>(
            &PreparedLevelImages::from_pis(&b_to_a_finest_inputs, Level::One),
            b_to_a_filtered,
            &b_to_a_retained,
            &motion,
        );
        let a_to_b = blend_periodic_boundary(a_to_b);
        let b_to_a = blend_periodic_boundary(b_to_a);
        let next_references = next_warm_references(&prior_references, &current_belts);
        let (a_to_b_median, b_to_a_median) = {
            let (a_to_b, b_to_a) = temporal_medians.split_mut();
            (a_to_b.state(), b_to_a.state())
        };

        // Retain the exact linear public-field tokens for the next call. The
        // checkpoint-only displacement adapter copies their components behind
        // a crate-private borrowed bridge rather than making them cloneable.
        let (fields, invalid_nodes) = DirectedFields::from_public_dense_ref(&a_to_b, &b_to_a);

        WarmTransition {
            estimate: WarmEstimate {
                displacement: Displacement::compose(&fields),
                invalid_nodes,
                weighted_rows: WorkRowCounts {
                    a_to_b_l2,
                    b_to_a_l2,
                    a_to_b_l1,
                    b_to_a_l1,
                },
            },
            known_next: KnownWarmNext {
                references: next_references,
                a_to_b_public: a_to_b,
                b_to_a_public: b_to_a,
                a_to_b_median,
                b_to_a_median,
                a_to_b_work_rows: a_to_b_next_rows,
                b_to_a_work_rows: b_to_a_next_rows,
                a_to_b_hints: a_to_b_next_hints,
                b_to_a_hints: b_to_a_next_hints,
                a_to_b_cadence: a_to_b_cadence.after_calc(),
                b_to_a_cadence: b_to_a_cadence.after_calc(),
            },
        }
    }
}

pub(crate) fn staged_warm_prepared_transition<S: PairedPisSolver>(
    inputs: &WarmRetainedInputs,
    controls: &PairedControlInputs,
    solver: &mut S,
) -> Result<WarmTransition, PairSolveError<S::Error>> {
    let current_belts = &controls.current_post_blur;
    let motion = MotionPyramid::from_base(&MotionMask::between(
        current_belts,
        &inputs.prior_references,
    ));
    let a_retained = RetainedPublicPyramids::from_public_ref(&inputs.a_to_b.prior_public);
    let b_retained = RetainedPublicPyramids::from_public_ref(&inputs.b_to_a.prior_public);
    let a_rows = inputs
        .a_to_b
        .work_rows
        .prepare_lack_rows_on_first_calc(&controls.a_to_b_lack, inputs.a_to_b.cadence.calc_count());
    let b_rows = inputs
        .b_to_a
        .work_rows
        .prepare_lack_rows_on_first_calc(&controls.b_to_a_lack, inputs.b_to_a.cadence.calc_count());
    let a_effective = a_rows.effective();
    let b_effective = b_rows.effective();

    let a_l2_modes = a_effective.modes(Level::Two).to_vec();
    let b_l2_modes = b_effective.modes(Level::Two).to_vec();
    let a_l2_weighted = weighted_rows(&a_l2_modes);
    let b_l2_weighted = weighted_rows(&b_l2_modes);
    let (a_l2, b_l2) = solve_pair(
        solver,
        PairedSolveRequest {
            stage: PairSolveStage::Warm { level: Level::Two },
            a_to_b: DirectionSolveRequest {
                cost_modes: a_l2_modes,
                initial: InitialGrid::coarse_zeros(),
                hint: inputs.a_to_b.hints.grid(Level::Two),
                admission: inputs.a_to_b.cadence.admission(),
            },
            b_to_a: DirectionSolveRequest {
                cost_modes: b_l2_modes,
                initial: InitialGrid::coarse_zeros(),
                hint: inputs.b_to_a.hints.grid(Level::Two),
                admission: inputs.b_to_a.cadence.admission(),
            },
        },
    )?;
    let a_seed = warm_seed(&controls.l2, a_l2, &a_retained, &motion);
    let b_seed = warm_seed(&controls.l2, b_l2, &b_retained, &motion);

    let a_l1_modes = a_effective.modes(Level::One).to_vec();
    let b_l1_modes = b_effective.modes(Level::One).to_vec();
    let a_l1_weighted = weighted_rows(&a_l1_modes);
    let b_l1_weighted = weighted_rows(&b_l1_modes);
    let (a_grid, b_grid) = solve_pair(
        solver,
        PairedSolveRequest {
            stage: PairSolveStage::Warm { level: Level::One },
            a_to_b: DirectionSolveRequest {
                cost_modes: a_l1_modes,
                initial: a_seed,
                hint: inputs.a_to_b.hints.grid(Level::One),
                admission: inputs.a_to_b.cadence.admission(),
            },
            b_to_a: DirectionSolveRequest {
                cost_modes: b_l1_modes,
                initial: b_seed,
                hint: inputs.b_to_a.hints.grid(Level::One),
                admission: inputs.b_to_a.cadence.admission(),
            },
        },
    )?;

    let a_hint_images = controls.l1.directed::<AtoB>();
    let b_hint_images = controls.l1.directed::<BtoA>();
    let a_next_hints = inputs
        .a_to_b
        .hints
        .replace_from_current_finest(&a_hint_images, &a_grid);
    let b_next_hints = inputs
        .b_to_a
        .hints
        .replace_from_current_finest(&b_hint_images, &b_grid);
    let mut medians =
        TemporalMedians::from_states(inputs.a_to_b_median.clone(), inputs.b_to_a_median.clone())
            .expect("validated warm median checkpoint must restore");
    let filtered = medians
        .run(DirectedPatchGrids::new(a_grid, b_grid))
        .expect("warm paired finest grids have the temporal median level");
    let (a_filtered, b_filtered) = filtered.into_parts();
    let a_small = SmallDisparityRows::refresh(
        &a_rows,
        &a_filtered,
        &controls.l1_block_mask_a,
        inputs.a_to_b.cadence.calc_count(),
    );
    let b_small = SmallDisparityRows::refresh(
        &b_rows,
        &b_filtered,
        &controls.l1_block_mask_a,
        inputs.b_to_a.cadence.calc_count(),
    );
    let shared_small = SharedSmallDisparityRows::merge(&a_small, &b_small);
    let a_next_rows = a_rows.after_warm_calc(&shared_small);
    let b_next_rows = b_rows.after_warm_calc(&shared_small);
    let a_public = blend_periodic_boundary(finish_direction::<AtoB>(
        &controls.l1,
        a_filtered,
        &a_retained,
        &motion,
    ));
    let b_public = blend_periodic_boundary(finish_direction::<BtoA>(
        &controls.l1,
        b_filtered,
        &b_retained,
        &motion,
    ));
    let next_references = next_warm_references(&inputs.prior_references, current_belts);
    let (a_median, b_median) = medians.into_states();
    let (fields, invalid_nodes) = DirectedFields::from_public_dense_ref(&a_public, &b_public);

    Ok(WarmTransition {
        estimate: WarmEstimate {
            displacement: Displacement::compose(&fields),
            invalid_nodes,
            weighted_rows: WorkRowCounts {
                a_to_b_l2: a_l2_weighted,
                b_to_a_l2: b_l2_weighted,
                a_to_b_l1: a_l1_weighted,
                b_to_a_l1: b_l1_weighted,
            },
        },
        known_next: KnownWarmNext {
            references: next_references,
            a_to_b_public: a_public,
            b_to_a_public: b_public,
            a_to_b_median: a_median,
            b_to_a_median: b_median,
            a_to_b_work_rows: a_next_rows,
            b_to_a_work_rows: b_next_rows,
            a_to_b_hints: a_next_hints,
            b_to_a_hints: b_next_hints,
            a_to_b_cadence: inputs.a_to_b.cadence.after_calc(),
            b_to_a_cadence: inputs.b_to_a.cadence.after_calc(),
        },
    })
}

fn warm_seed<D: PisDirection>(
    prepared: &PreparedLevelImages,
    patches: PatchGrid<D>,
    retained: &RetainedPublicPyramids<D>,
    motion: &MotionPyramid,
) -> InitialGrid<D> {
    let images = prepared.directed::<D>();
    let dense = dense::densify_coarse(&images, patches)
        .expect("typed warm level-two images and patches share one level");
    let post =
        update_without_variational_with_retained(dense, retained, motion.level::<D>(Level::Two))
            .expect("typed warm level-two field and motion share one level");
    into_l1_initial_grid(post)
        .expect("typed warm level-two post-update field forms the finest seed")
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Clone, Copy)]
enum DirectionSchedule {
    Parallel,
    #[cfg(test)]
    Serial,
}

#[cfg(test)]
struct DirectionPreMedian<D: PisDirection> {
    finest_inputs: LevelInputs,
    rows: RetainedWorkRows<D>,
    grid: PatchGrid<D>,
    block_mask: Box<[u8]>,
    next_hints: HintPyramid<D>,
    l2_weighted: usize,
    l1_weighted: usize,
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn solve_pre_median<D: PisDirection>(
    current: &ColdInputs,
    masks: &MaskPyramid,
    retained: &RetainedPublicPyramids<D>,
    motion: &MotionPyramid,
    finest_inputs: LevelInputs,
    rows: RetainedWorkRows<D>,
    hints: HintPyramid<D>,
    admission: DescentAdmission,
) -> DirectionPreMedian<D> {
    let effective_rows = rows.effective();
    let (seed, l2_weighted) = solve_coarse::<D>(
        current,
        masks,
        retained,
        motion,
        &effective_rows,
        &hints,
        admission,
    );
    let block_mask = finest_inputs.small_disparity_block_mask(Level::One);
    let (grid, l1_weighted) =
        solve_finest::<D>(&finest_inputs, seed, &effective_rows, &hints, admission);
    let hint_images = finest_inputs.directed_images::<D>(Level::One);
    let next_hints = hints.replace_from_current_finest(&hint_images, &grid);
    DirectionPreMedian {
        finest_inputs,
        rows,
        grid,
        block_mask,
        next_hints,
        l2_weighted,
        l1_weighted,
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn solve_coarse<D: PisDirection>(
    current: &ColdInputs,
    masks: &MaskPyramid,
    retained: &RetainedPublicPyramids<D>,
    motion: &MotionPyramid,
    rows: &EffectiveWorkRows<D>,
    hints: &HintPyramid<D>,
    admission: DescentAdmission,
) -> (InitialGrid<D>, usize) {
    let level = Level::Two;
    let prepared = LevelInputs::build::<D>(current, masks, level);
    let (input, weighted) = prepared.input::<D>(level, rows.modes(level).to_vec());
    let hint = hints.grid(level);
    let patches = pis::solve_with_descent_admission(
        &input,
        InitialGrid::coarse_zeros(),
        Some(&hint),
        admission,
    )
    .expect("typed warm level-two PIS inputs share one level");
    let images = prepared.directed_images::<D>(level);
    let dense = dense::densify_coarse(&images, patches)
        .expect("typed warm level-two images and patches share one level");
    let post = update_without_variational_with_retained(dense, retained, motion.level::<D>(level))
        .expect("typed warm level-two field and motion share one level");
    let seed = into_l1_initial_grid(post)
        .expect("typed warm level-two post-update field forms the finest seed");
    (seed, weighted)
}

#[cfg(test)]
fn solve_finest<D: PisDirection>(
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
        .expect("typed warm level-one PIS inputs share one level");
    (patches, weighted)
}

fn finish_direction<D: PisDirection>(
    prepared: &PreparedLevelImages,
    filtered: FilteredPatchGrid<D>,
    retained: &RetainedPublicPyramids<D>,
    motion: &MotionPyramid,
) -> PublicDenseField<D> {
    let level = Level::One;
    let images = prepared.directed::<D>();
    let dense = dense::densify_finest(&images, filtered)
        .expect("typed warm finest images and filtered patches share one level");
    let post = update_without_variational_with_retained(dense, retained, motion.level::<D>(level))
        .expect("typed warm level-one field and motion share one level");
    dense::finish_linear_x2(post)
        .expect("typed warm finest post-update field resizes to the public grid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::one_xs::{COLS, LensPair, ROWS};

    #[derive(Clone, Copy)]
    struct CorpusRecord {
        label: &'static str,
        offset: usize,
        bytes: usize,
        sha256: &'static str,
    }

    impl CorpusRecord {
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

    fn encoded_rows(level: Level, weighted_bits: &[usize]) -> Vec<u8> {
        let mut bytes =
            vec![0u8; level.patch_rows().div_ceil(VECTOR_BOOL_WORD_BITS) * VECTOR_BOOL_WORD_BYTES];
        for &bit in weighted_bits {
            assert!(bit < level.patch_rows());
            bytes[bit / 8] |= 1u8 << (bit % 8);
        }
        bytes
    }

    fn encoded_bool_rows(level: Level, rows: &[bool]) -> Vec<u8> {
        assert_eq!(rows.len(), level.patch_rows());
        encoded_rows(
            level,
            &rows
                .iter()
                .enumerate()
                .filter_map(|(row, enabled)| enabled.then_some(row))
                .collect::<Vec<_>>(),
        )
    }

    fn dense_level(level: Level, value: f32) -> DenseHintLevel {
        DenseHintLevel::from_row_major_components(
            vec![value; level.pixels()],
            vec![value; level.pixels()],
        )
    }

    fn zero_hints<D: PisDirection>() -> HintPyramid<D> {
        HintPyramid::cold_zeros()
    }

    #[test]
    fn work_rows_decode_exact_lsb0_bits_and_shapes() {
        let l1_bits = [0, 7, 8, 63, 64, Level::One.patch_rows() - 1];
        let l2_bits = [1, 63, 64, Level::Two.patch_rows() - 1];
        let rows = EffectiveWorkRows::<AtoB>::from_lsb0_bytes(
            &encoded_rows(Level::One, &l1_bits),
            &encoded_rows(Level::Two, &l2_bits),
        )
        .unwrap();

        assert_eq!(rows.modes(Level::One).len(), 178);
        assert_eq!(rows.modes(Level::Two).len(), 88);
        for level in [Level::One, Level::Two] {
            let expected = if level == Level::One {
                &l1_bits[..]
            } else {
                &l2_bits[..]
            };
            for (row, mode) in rows.modes(level).iter().enumerate() {
                assert_eq!(
                    *mode,
                    if expected.contains(&row) {
                        CostMode::Weighted
                    } else {
                        CostMode::Unweighted
                    },
                    "wrong {level} mode at row {row}",
                );
            }
        }

        assert!(matches!(
            EffectiveWorkRows::<AtoB>::from_lsb0_bytes(&[0; 23], &[0; 16]),
            Err(WarmRestoreError::WorkRowByteCount {
                direction: Direction::AtoB,
                level: Level::One,
                actual: 23,
                expected: 24,
            })
        ));
        assert!(matches!(
            EffectiveWorkRows::<BtoA>::from_lsb0_bytes(&[0; 24], &[0; 15]),
            Err(WarmRestoreError::WorkRowByteCount {
                direction: Direction::BtoA,
                level: Level::Two,
                actual: 15,
                expected: 16,
            })
        ));
    }

    #[test]
    fn work_rows_reject_every_noncanonical_padding_bit() {
        for level in [Level::One, Level::Two] {
            let logical = level.patch_rows();
            let byte_count = logical.div_ceil(VECTOR_BOOL_WORD_BITS) * VECTOR_BOOL_WORD_BYTES;
            for bit in logical..byte_count * 8 {
                let mut bad = vec![0u8; byte_count];
                bad[bit / 8] |= 1u8 << (bit % 8);
                let result = match level {
                    Level::One => EffectiveWorkRows::<AtoB>::from_lsb0_bytes(&bad, &[0; 16]),
                    Level::Two => EffectiveWorkRows::<AtoB>::from_lsb0_bytes(&[0; 24], &bad),
                };
                assert!(matches!(
                    result,
                    Err(WarmRestoreError::NonCanonicalWorkRowPadding {
                        direction: Direction::AtoB,
                        level: error_level,
                        bit: error_bit,
                        logical_bits,
                    }) if error_level == level && error_bit == bit && logical_bits == logical
                ));
            }
        }
    }

    #[test]
    fn small_disparity_rows_use_the_selected_mask_mean_and_strict_five_pixel_cutoff() {
        assert_eq!(SMALL_DISPARITY_MEAN_THRESHOLD.to_bits(), 5.0f32.to_bits());
        let patches = Level::One.patches();
        let below = f32::from_bits(5.0f32.to_bits() - 1);

        let enabled = vec![u8::MAX; patches];
        assert!(
            classify_small_disparity_rows(&vec![below; patches], &enabled)
                .iter()
                .all(|row| *row)
        );
        assert!(
            classify_small_disparity_rows(&vec![5.0; patches], &enabled)
                .iter()
                .all(|row| !*row)
        );

        let mut dcol = vec![f32::NAN; patches];
        let mut mask = vec![0u8; patches];
        for patch_row in 0..Level::One.patch_rows() {
            let first = patch_row * Level::One.patch_cols();
            dcol[first] = below;
            mask[first] = 1;
        }
        assert!(
            classify_small_disparity_rows(&dcol, &mask)
                .iter()
                .all(|row| *row),
            "masked NaNs must not enter the scalar mean",
        );

        mask[0] = 0;
        assert!(classify_small_disparity_rows(&dcol, &mask)[0]);
        mask[0] = 1;
        dcol[0] = f32::NAN;
        assert!(!classify_small_disparity_rows(&dcol, &mask)[0]);
    }

    #[test]
    fn small_disparity_rows_retain_until_signed_calc_count_three() {
        let retained_bits = [7, 93, 177];
        let retained = RetainedWorkRows::<AtoB>::from_lsb0_bytes(
            Some(&encoded_rows(Level::One, &retained_bits)),
            &encoded_rows(Level::One, &[]),
        )
        .unwrap();
        let fresh = vec![true; Level::One.patch_rows()].into_boxed_slice();

        for calc_count in [i32::MIN, -1, 0, 1, 2] {
            assert_eq!(
                SmallDisparityRows::retain_or_replace(&retained, Some(fresh.clone()), calc_count,)
                    .rows
                    .as_deref(),
                retained.small_disparity_rows(),
                "native preserves +0x108 when it skips the writer at count {calc_count}",
            );
        }
        for calc_count in [3, 4, i32::MAX] {
            assert!(
                SmallDisparityRows::retain_or_replace(&retained, Some(fresh.clone()), calc_count,)
                    .rows
                    .unwrap()
                    .iter()
                    .all(|row| *row),
                "native replaces +0x108 when it runs the writer at count {calc_count}",
            );
        }
    }

    #[test]
    fn retained_rows_distinguish_absent_from_present_zero_small_disparity() {
        let lack = encoded_rows(Level::One, &[11, 87]);
        let absent = RetainedWorkRows::<AtoB>::from_lsb0_bytes(None, &lack).unwrap();
        let present_zero =
            RetainedWorkRows::<AtoB>::from_lsb0_bytes(Some(&encoded_rows(Level::One, &[])), &lack)
                .unwrap();

        assert_ne!(absent, present_zero);
        assert!(absent.small_disparity_rows().is_none());
        assert_eq!(
            present_zero.small_disparity_rows(),
            Some(vec![false; Level::One.patch_rows()].as_slice()),
        );
        assert_eq!(absent.effective(), present_zero.effective());
    }

    #[test]
    fn skipped_small_disparity_writer_preserves_absence_and_mature_zero_materializes() {
        let retained =
            RetainedWorkRows::<AtoB>::from_lsb0_bytes(None, &encoded_rows(Level::One, &[]))
                .unwrap();
        let fresh_zero = vec![false; Level::One.patch_rows()].into_boxed_slice();

        for calc_count in [i32::MIN, -1, 0, 1, 2] {
            assert!(
                SmallDisparityRows::retain_or_replace(
                    &retained,
                    Some(fresh_zero.clone()),
                    calc_count,
                )
                .rows
                .is_none(),
                "native preserves absent +0x108 at count {calc_count}",
            );
        }
        assert_eq!(
            SmallDisparityRows::retain_or_replace(&retained, Some(fresh_zero), 3)
                .rows
                .as_deref(),
            Some(vec![false; Level::One.patch_rows()].as_slice()),
            "running the writer materializes even an all-zero +0x108 vector",
        );
    }

    #[test]
    fn bilateral_small_row_merge_preserves_optional_vector_topology() {
        let absent_a = SmallDisparityRows::<AtoB> {
            rows: None,
            direction: PhantomData,
        };
        let absent_b = SmallDisparityRows::<BtoA> {
            rows: None,
            direction: PhantomData,
        };
        assert!(
            SharedSmallDisparityRows::merge(&absent_a, &absent_b)
                .rows
                .is_none()
        );

        let zero_a = SmallDisparityRows::<AtoB> {
            rows: Some(vec![false; Level::One.patch_rows()].into_boxed_slice()),
            direction: PhantomData,
        };
        let zero_b = SmallDisparityRows::<BtoA> {
            rows: Some(vec![false; Level::One.patch_rows()].into_boxed_slice()),
            direction: PhantomData,
        };
        for merged in [
            SharedSmallDisparityRows::merge(&zero_a, &absent_b),
            SharedSmallDisparityRows::merge(&absent_a, &zero_b),
        ] {
            assert_eq!(
                merged.rows.as_deref(),
                Some(vec![false; Level::One.patch_rows()].as_slice()),
            );
        }
    }

    #[test]
    fn lack_rows_refresh_only_when_the_calc_counter_is_exactly_zero() {
        let pixels = ROWS * COLS;
        let current = ColdInputs::from_prepared(
            LensPair {
                a: vec![73; pixels],
                b: vec![73; pixels],
            },
            LensPair {
                a: vec![u8::MAX; pixels],
                b: vec![u8::MAX; pixels],
            },
        )
        .unwrap();
        let masks = MaskPyramid::build(&current);
        let inputs = LevelInputs::build::<AtoB>(&current, &masks, Level::One);
        let retained =
            RetainedWorkRows::<AtoB>::from_lsb0_bytes(None, &encoded_rows(Level::One, &[]))
                .unwrap();

        for calc_count in [i32::MIN, -1, 1, i32::MAX] {
            assert!(
                retained
                    .clone()
                    .prepare_lack_on_first_calc(&inputs, calc_count)
                    .lack_of_texture_rows()
                    .iter()
                    .all(|row| !*row),
                "nonzero warm calculation {calc_count} must retain the existing +0x120 bytes",
            );
        }
        assert!(
            retained
                .prepare_lack_on_first_calc(&inputs, 0)
                .lack_of_texture_rows()
                .iter()
                .all(|row| *row),
            "counter zero prepares +0x120 before current d8",
        );
    }

    #[test]
    fn bilateral_small_rows_union_before_directional_lack_and_run_propagation() {
        let mut a = vec![false; Level::One.patch_rows()];
        let mut b = vec![false; Level::One.patch_rows()];
        a[18..=27].fill(true);
        b[53..=60].fill(true);
        let shared = SharedSmallDisparityRows::merge(
            &SmallDisparityRows::<AtoB> {
                rows: Some(a.into_boxed_slice()),
                direction: PhantomData,
            },
            &SmallDisparityRows::<BtoA> {
                rows: Some(b.into_boxed_slice()),
                direction: PhantomData,
            },
        );

        let mut a_texture = vec![3_000.0f32; Level::One.patches()];
        let mut b_texture = a_texture.clone();
        for patch_col in 0..Level::One.patch_cols() {
            a_texture[79 * Level::One.patch_cols() + patch_col] = 0.0;
            b_texture[84 * Level::One.patch_cols() + patch_col] = 0.0;
        }
        let a_lack = dense::calculate_lack_rows::<AtoB>(Level::One, &a_texture).unwrap();
        let b_lack = dense::calculate_lack_rows::<BtoA>(Level::One, &b_texture).unwrap();
        let a_rows = RetainedWorkRows::<AtoB>::from_lsb0_bytes(
            None,
            &encoded_bool_rows(Level::One, a_lack.rows()),
        )
        .unwrap()
        .after_warm_calc(&shared)
        .effective();
        let b_rows = RetainedWorkRows::<BtoA>::from_lsb0_bytes(
            None,
            &encoded_bool_rows(Level::One, b_lack.rows()),
        )
        .unwrap()
        .after_warm_calc(&shared)
        .effective();

        for row in 0..Level::One.patch_rows() {
            let shared_row = (18..=27).contains(&row) || (53..=60).contains(&row);
            assert_eq!(
                a_rows.modes(Level::One)[row],
                if shared_row || row == 79 {
                    CostMode::Weighted
                } else {
                    CostMode::Unweighted
                },
            );
            assert_eq!(
                b_rows.modes(Level::One)[row],
                if shared_row || row == 84 {
                    CostMode::Weighted
                } else {
                    CostMode::Unweighted
                },
            );
        }
        let a_coarse = a_rows
            .modes(Level::Two)
            .iter()
            .enumerate()
            .filter_map(|(row, mode)| (*mode == CostMode::Weighted).then_some(row))
            .collect::<Vec<_>>();
        let b_coarse = b_rows
            .modes(Level::Two)
            .iter()
            .enumerate()
            .filter_map(|(row, mode)| (*mode == CostMode::Weighted).then_some(row))
            .collect::<Vec<_>>();
        assert_eq!(
            a_coarse,
            (8..=13)
                .chain(26..=30)
                .chain(std::iter::once(39))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            b_coarse,
            (8..=13).chain(26..=30).chain(41..=42).collect::<Vec<_>>()
        );
    }

    #[test]
    fn hint_pyramid_validates_complete_planes_and_samples_patch_centres() {
        let cold = HintPyramid::<AtoB>::cold_zeros();
        for level in [Level::One, Level::Two] {
            let dense = cold.level(level);
            assert!(
                dense
                    .dcol()
                    .iter()
                    .chain(dense.drow())
                    .all(|value| value.to_bits() == 0.0f32.to_bits()),
                "cold prepareBuffers hints must be present positive-zero planes",
            );
            assert!(
                cold.grid(level)
                    .flows()
                    .iter()
                    .all(|flow| flow.dcol().to_bits() == 0 && flow.drow().to_bits() == 0)
            );
        }

        let l1_dcol = (0..Level::One.pixels())
            .map(|index| index as f32)
            .collect::<Vec<_>>();
        let l1_drow = l1_dcol.iter().map(|value| -*value).collect::<Vec<_>>();
        let l2_dcol = (0..Level::Two.pixels())
            .map(|index| index as f32 + 0.25)
            .collect::<Vec<_>>();
        let l2_drow = l2_dcol.iter().map(|value| -*value).collect::<Vec<_>>();
        let hints = HintPyramid::<AtoB>::from_levels(
            DenseHintLevel::from_row_major_components(l1_dcol, l1_drow),
            DenseHintLevel::from_row_major_components(l2_dcol, l2_drow),
        )
        .unwrap();

        for level in [Level::One, Level::Two] {
            let grid = hints.grid(level);
            assert_eq!(grid.direction(), Direction::AtoB);
            assert_eq!(grid.flows().len(), level.patches());
            for (patch_row, patch_col) in [(0, 0), (level.patch_rows() - 1, level.patch_cols() - 1)]
            {
                let dense_row = patch_row * PATCH_STRIDE + PATCH_SIZE / 2;
                let dense_col = patch_col * PATCH_STRIDE + PATCH_SIZE / 2;
                let dense_index = dense_row * level.cols() + dense_col;
                let patch_index = patch_row * level.patch_cols() + patch_col;
                let offset = if level == Level::One { 0.0 } else { 0.25 };
                assert_eq!(
                    grid.flows()[patch_index].dcol(),
                    dense_index as f32 + offset
                );
                assert_eq!(
                    grid.flows()[patch_index].drow(),
                    -(dense_index as f32 + offset)
                );
            }
        }

        let error = HintPyramid::<BtoA>::from_levels(
            DenseHintLevel::from_row_major_components(
                vec![0.0; Level::One.pixels() - 1],
                vec![0.0; Level::One.pixels()],
            ),
            dense_level(Level::Two, 0.0),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            WarmRestoreError::HintPlaneShape {
                direction: Direction::BtoA,
                level: Level::One,
                component: "dcol",
                ..
            }
        ));

        let mut unsampled = vec![0.0; Level::Two.pixels()];
        unsampled[0] = f32::NAN;
        let error = HintPyramid::<AtoB>::from_levels(
            dense_level(Level::One, 0.0),
            DenseHintLevel::from_row_major_components(vec![0.0; Level::Two.pixels()], unsampled),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            WarmRestoreError::NonFiniteHint {
                direction: Direction::AtoB,
                level: Level::Two,
                component: "drow",
                index: 0,
            }
        ));
    }

    #[test]
    fn next_hint_level_uses_native_left_association_scaling_and_sparse_writes() {
        let mut source = vec![0.0f32; Level::One.pixels()];
        source[0] = 1.0e20;
        source[1] = 1.0;
        source[Level::One.cols()] = -1.0e20;
        source[Level::One.cols() + 1] = 3.0;
        assert_eq!(
            propagated_hint_value(&source, 0, 0).to_bits(),
            0.375f32.to_bits()
        );

        source.fill(0.0);
        source[0] = f32::from_bits(5);
        assert_eq!(
            propagated_hint_value(&source, 0, 0).to_bits(),
            0.0f32.to_bits(),
            "separate quarter then half scaling must not collapse to one eighth",
        );

        source.fill(0.0);
        source[538 * Level::One.cols() + 28] = 2.0;
        source[538 * Level::One.cols() + 29] = 4.0;
        source[539 * Level::One.cols() + 28] = 8.0;
        source[539 * Level::One.cols() + 29] = 16.0;
        assert_eq!(
            propagated_hint_value(&source, Level::Two.rows() - 1, Level::Two.cols() - 1,),
            3.75,
            "the four boundary children keep their independent coordinates",
        );
        assert_eq!(
            propagated_hint_value(&source, Level::One.rows(), Level::One.cols()),
            8.0,
            "out-of-range doubled children clamp to the last source pixel",
        );
        source.fill(8.0);
        let level_one = DenseHintLevel::from_row_major_components(source.clone(), source);
        let level_two = propagate_hint_level(&level_one);
        for row in 0..Level::Two.rows() {
            for col in 0..Level::Two.cols() {
                let at = row * Level::Two.cols() + col;
                let is_centre = row >= PATCH_SIZE / 2
                    && col >= PATCH_SIZE / 2
                    && (row - PATCH_SIZE / 2).is_multiple_of(PATCH_STRIDE)
                    && (col - PATCH_SIZE / 2).is_multiple_of(PATCH_STRIDE)
                    && (row - PATCH_SIZE / 2) / PATCH_STRIDE < Level::Two.patch_rows()
                    && (col - PATCH_SIZE / 2) / PATCH_STRIDE < Level::Two.patch_cols();
                assert_eq!(
                    level_two.dcol[at].to_bits(),
                    if is_centre { 4.0f32 } else { 0.0f32 }.to_bits(),
                );
                assert_eq!(level_two.drow[at].to_bits(), level_two.dcol[at].to_bits());
            }
        }
    }

    #[test]
    fn empty_override_admission_is_shared_without_an_interlevel_increment() {
        let no_descents = EmptyOverrideCadence::new(6_372, 10).unwrap();
        let first_level = no_descents.admission();
        let second_level = no_descents.admission();
        assert_eq!(first_level, DescentAdmission::NoPatches);
        assert_eq!(second_level, first_level);

        assert_eq!(
            EmptyOverrideCadence::new(6_370, 10).unwrap().admission(),
            DescentAdmission::EveryPatch,
        );
        assert_eq!(
            EmptyOverrideCadence::new(1, 0),
            Err(WarmRestoreError::ZeroCadence),
        );
        assert_eq!(no_descents.after_calc().calc_count(), 6_373);
        assert_eq!(no_descents.after_calc().cadence(), 10);

        let mut cold = EmptyOverrideCadence::new(0, 10).unwrap();
        let mut cold_admissions = Vec::new();
        for _ in 0..3 {
            cold_admissions.push(cold.admission());
            cold = cold.after_calc();
        }
        assert_eq!(
            cold_admissions,
            [
                DescentAdmission::EveryPatch,
                DescentAdmission::NoPatches,
                DescentAdmission::NoPatches,
            ],
        );
        assert_eq!(cold.calc_count(), 3);
        assert_eq!(cold.cadence(), 10);

        assert_eq!(
            EmptyOverrideCadence::new(i32::MAX, 10)
                .unwrap()
                .after_calc()
                .calc_count(),
            i32::MIN,
        );
    }

    fn synthetic_current(image: Vec<u8>) -> ColdInputs {
        let pixels = ROWS * COLS;
        assert_eq!(image.len(), pixels);
        let images = LensPair {
            a: image.clone(),
            b: image,
        };
        let masks = LensPair {
            a: vec![255; pixels],
            b: vec![255; pixels],
        };
        ColdInputs::from_prepared(images, masks).unwrap()
    }

    fn synthetic_checkpoint(image: Vec<u8>, calc_count: i32) -> WarmCheckpointInputs {
        let pixels = ROWS * COLS;
        let current = synthetic_current(image);
        let references = current.blurred_belts();
        let empty_lack = encoded_rows(Level::One, &[]);
        let rows_a_to_b = RetainedWorkRows::from_lsb0_bytes(None, &empty_lack).unwrap();
        let rows_b_to_a = RetainedWorkRows::from_lsb0_bytes(None, &empty_lack).unwrap();
        let cadence = EmptyOverrideCadence::new(calc_count, 10).unwrap();
        let a_to_b = WarmDirection::new(
            PublicDenseField::<AtoB>::from_row_major_components(
                vec![0.0; pixels],
                vec![0.0; pixels],
            )
            .unwrap(),
            rows_a_to_b,
            zero_hints(),
            cadence,
        );
        let b_to_a = WarmDirection::new(
            PublicDenseField::<BtoA>::from_row_major_components(
                vec![0.0; pixels],
                vec![0.0; pixels],
            )
            .unwrap(),
            rows_b_to_a,
            zero_hints(),
            cadence,
        );
        WarmCheckpointInputs::new(current, references, a_to_b, b_to_a, TemporalMedians::new())
    }

    fn asymmetric_checkpoint(
        a_to_b_calc_count: i32,
        b_to_a_calc_count: i32,
    ) -> WarmCheckpointInputs {
        let pixels = ROWS * COLS;
        let image_a = (0..pixels)
            .map(|index| {
                let row = index / COLS;
                let col = index % COLS;
                ((row * 3 + col * 5) % 251) as u8
            })
            .collect::<Vec<_>>();
        let image_b = (0..pixels)
            .map(|index| {
                let row = index / COLS;
                let col = index % COLS;
                ((row * 11 + col * 7 + 19) % 253) as u8
            })
            .collect::<Vec<_>>();
        let mask_a = (0..pixels)
            .map(|index| if index.is_multiple_of(17) { 0 } else { 255 })
            .collect::<Vec<_>>();
        let mask_b = (0..pixels)
            .map(|index| if index.is_multiple_of(23) { 0 } else { 255 })
            .collect::<Vec<_>>();
        let current = ColdInputs::from_prepared(
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
        let references = synthetic_current(vec![37; pixels]).blurred_belts();

        let a_to_b_rows = RetainedWorkRows::from_lsb0_bytes(
            Some(&encoded_rows(Level::One, &[1, 9, 77, 150])),
            &encoded_rows(Level::One, &[3, 31, 92]),
        )
        .unwrap();
        let b_to_a_rows = RetainedWorkRows::from_lsb0_bytes(
            Some(&encoded_rows(Level::One, &[4, 18, 103, 166])),
            &encoded_rows(Level::One, &[12, 64, 120]),
        )
        .unwrap();
        let a_to_b = WarmDirection::new(
            PublicDenseField::<AtoB>::from_row_major_components(
                (0..pixels)
                    .map(|index| (index % COLS) as f32 * 0.03125)
                    .collect(),
                (0..pixels)
                    .map(|index| -((index / COLS) as f32) * 0.000_976_562_5)
                    .collect(),
            )
            .unwrap(),
            a_to_b_rows,
            zero_hints(),
            EmptyOverrideCadence::new(a_to_b_calc_count, 10).unwrap(),
        );
        let b_to_a = WarmDirection::new(
            PublicDenseField::<BtoA>::from_row_major_components(
                (0..pixels)
                    .map(|index| -((index % COLS) as f32) * 0.015625)
                    .collect(),
                (0..pixels)
                    .map(|index| (index / COLS) as f32 * 0.001_953_125)
                    .collect(),
            )
            .unwrap(),
            b_to_a_rows,
            zero_hints(),
            EmptyOverrideCadence::new(b_to_a_calc_count, 10).unwrap(),
        );
        WarmCheckpointInputs::new(current, references, a_to_b, b_to_a, TemporalMedians::new())
    }

    fn assert_f32_bits_eq(actual: &[f32], expected: &[f32], label: &str) {
        assert_eq!(actual.len(), expected.len(), "{label} length");
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(
                actual.to_bits(),
                expected.to_bits(),
                "{label} bit mismatch at {index}",
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
        assert_f32_bits_eq(actual.values(), expected.values(), label);
    }

    fn assert_transition_bit_exact(actual: WarmTransition, expected: WarmTransition) {
        assert_eq!(actual.estimate, expected.estimate);
        let actual = actual.known_next;
        let expected = expected.known_next;
        assert_eq!(actual.references.bytes(), expected.references.bytes());
        for (actual, expected, label) in [
            (
                actual.a_to_b_public.dcol(),
                expected.a_to_b_public.dcol(),
                "A-to-B public dcol",
            ),
            (
                actual.a_to_b_public.drow(),
                expected.a_to_b_public.drow(),
                "A-to-B public drow",
            ),
            (
                actual.b_to_a_public.dcol(),
                expected.b_to_a_public.dcol(),
                "B-to-A public dcol",
            ),
            (
                actual.b_to_a_public.drow(),
                expected.b_to_a_public.drow(),
                "B-to-A public drow",
            ),
        ] {
            assert_f32_bits_eq(actual, expected, label);
        }
        assert_median_bits_eq(
            &actual.a_to_b_median,
            &expected.a_to_b_median,
            "A-to-B temporal median values",
        );
        assert_median_bits_eq(
            &actual.b_to_a_median,
            &expected.b_to_a_median,
            "B-to-A temporal median values",
        );
        assert_eq!(actual.a_to_b_work_rows, expected.a_to_b_work_rows);
        assert_eq!(actual.b_to_a_work_rows, expected.b_to_a_work_rows);
        for (actual, expected, label) in [
            (
                actual.a_to_b_hints.level(Level::One),
                expected.a_to_b_hints.level(Level::One),
                "A-to-B L1 hints",
            ),
            (
                actual.a_to_b_hints.level(Level::Two),
                expected.a_to_b_hints.level(Level::Two),
                "A-to-B L2 hints",
            ),
            (
                actual.b_to_a_hints.level(Level::One),
                expected.b_to_a_hints.level(Level::One),
                "B-to-A L1 hints",
            ),
            (
                actual.b_to_a_hints.level(Level::Two),
                expected.b_to_a_hints.level(Level::Two),
                "B-to-A L2 hints",
            ),
        ] {
            assert_f32_bits_eq(actual.dcol(), expected.dcol(), label);
            assert_f32_bits_eq(actual.drow(), expected.drow(), label);
        }
        assert_eq!(actual.a_to_b_cadence, expected.a_to_b_cadence);
        assert_eq!(actual.b_to_a_cadence, expected.b_to_a_cadence);
    }

    #[test]
    fn asymmetric_parallel_warm_solve_is_bit_exact_to_serial_across_barriers() {
        for (a_to_b_calc_count, b_to_a_calc_count) in [(0, 3), (6_370, 6_372)] {
            let serial = WarmPair::new()
                .transition_serial(asymmetric_checkpoint(a_to_b_calc_count, b_to_a_calc_count));
            let parallel = WarmPair::new()
                .transition(asymmetric_checkpoint(a_to_b_calc_count, b_to_a_calc_count));
            assert_transition_bit_exact(parallel, serial);
        }
    }

    #[test]
    fn synthetic_warm_checkpoint_runs_the_complete_composition() {
        let WarmTransition {
            estimate: result,
            known_next,
        } = WarmPair::new().transition(synthetic_checkpoint(vec![73; ROWS * COLS], 6_372));

        assert_eq!(result.weighted_rows, WorkRowCounts::default());
        assert_eq!(result.displacement.bytes().len(), Displacement::BYTES);
        assert_eq!(result.invalid_nodes.a_to_b, result.invalid_nodes.b_to_a);
        assert!(
            result
                .displacement
                .bytes()
                .chunks_exact(4)
                .all(|bytes| f32::from_ne_bytes(bytes.try_into().unwrap()).is_finite())
        );
        assert!(
            known_next
                .references
                .bytes()
                .iter()
                .all(|value| *value == 73)
        );
        assert!(
            known_next
                .a_to_b_public
                .dcol()
                .iter()
                .chain(known_next.a_to_b_public.drow())
                .chain(known_next.b_to_a_public.dcol())
                .chain(known_next.b_to_a_public.drow())
                .all(|value| value.is_finite())
        );
        for hints in [
            known_next.a_to_b_hints.level(Level::One),
            known_next.a_to_b_hints.level(Level::Two),
            known_next.b_to_a_hints.level(Level::One),
            known_next.b_to_a_hints.level(Level::Two),
        ] {
            assert!(
                hints
                    .dcol()
                    .iter()
                    .chain(hints.drow())
                    .all(|value| value.to_bits() == 0.0f32.to_bits())
            );
        }
        let a_to_b_effective = known_next.a_to_b_work_rows.effective();
        let b_to_a_effective = known_next.b_to_a_work_rows.effective();
        for level in [Level::One, Level::Two] {
            assert!(
                a_to_b_effective
                    .modes(level)
                    .iter()
                    .all(|mode| *mode == CostMode::Weighted)
            );
            assert!(
                b_to_a_effective
                    .modes(level)
                    .iter()
                    .all(|mode| *mode == CostMode::Weighted)
            );
        }
        assert_eq!(known_next.a_to_b_cadence.calc_count(), 6_373);
        assert_eq!(known_next.b_to_a_cadence.calc_count(), 6_373);
        assert_eq!(known_next.a_to_b_cadence.cadence(), 10);
        assert_eq!(known_next.b_to_a_cadence.cadence(), 10);
    }

    #[test]
    fn warm_transition_uses_the_pre_increment_count_for_small_disparity_rows() {
        let ramp = (0..ROWS * COLS)
            .map(|index| ((index % COLS) * 4) as u8)
            .collect::<Vec<_>>();

        for (calc_count, expected) in [(2, CostMode::Unweighted), (3, CostMode::Weighted)] {
            let next = WarmPair::new()
                .transition(synthetic_checkpoint(ramp.clone(), calc_count))
                .known_next;
            assert_eq!(
                next.a_to_b_work_rows.small_disparity_rows().is_some(),
                calc_count == 3,
                "AB raw +0x108 topology at pre-increment count {calc_count}",
            );
            assert_eq!(
                next.b_to_a_work_rows.small_disparity_rows().is_some(),
                calc_count == 3,
                "BA raw +0x108 topology at pre-increment count {calc_count}",
            );
            let a_to_b_effective = next.a_to_b_work_rows.effective();
            let b_to_a_effective = next.b_to_a_work_rows.effective();
            for level in [Level::One, Level::Two] {
                assert_eq!(
                    a_to_b_effective.modes(level),
                    vec![expected; level.patch_rows()],
                    "AB {level} rows at pre-increment count {calc_count}",
                );
                assert_eq!(
                    b_to_a_effective.modes(level),
                    vec![expected; level.patch_rows()],
                    "BA {level} rows at pre-increment count {calc_count}",
                );
            }
        }
    }

    #[test]
    fn counter_zero_prepares_lack_rows_before_the_same_transition_consumes_d8() {
        let image = vec![73; ROWS * COLS];
        let first = WarmPair::new().transition(synthetic_checkpoint(image.clone(), 0));
        assert_eq!(
            first.estimate.weighted_rows,
            WorkRowCounts {
                a_to_b_l2: Level::Two.patch_rows(),
                b_to_a_l2: Level::Two.patch_rows(),
                a_to_b_l1: Level::One.patch_rows(),
                b_to_a_l1: Level::One.patch_rows(),
            },
            "first-calculation +0x120 must feed current d8 before PIS",
        );
        assert!(
            first
                .known_next
                .a_to_b_work_rows
                .lack_of_texture_rows()
                .iter()
                .all(|row| *row),
        );
        assert!(
            first
                .known_next
                .b_to_a_work_rows
                .lack_of_texture_rows()
                .iter()
                .all(|row| *row),
        );

        let retained = WarmPair::new().transition(synthetic_checkpoint(image, 1));
        assert_eq!(retained.estimate.weighted_rows, WorkRowCounts::default());
        assert!(
            retained
                .known_next
                .a_to_b_work_rows
                .lack_of_texture_rows()
                .iter()
                .all(|row| !*row),
        );
        assert!(
            retained
                .known_next
                .b_to_a_work_rows
                .lack_of_texture_rows()
                .iter()
                .all(|row| !*row),
        );
    }

    #[test]
    fn produced_state_is_consumed_as_one_owned_second_warm_checkpoint() {
        fn assert_saturated_small_and_empty_lack<D: PisDirection>(rows: &RetainedWorkRows<D>) {
            assert!(
                rows.small_disparity_rows().unwrap().iter().all(|row| *row),
                "the calculation must retain saturated raw +0x108",
            );
            assert!(
                rows.lack_of_texture_rows().iter().all(|row| !*row),
                "nonzero cadence must preserve the empty raw +0x120 owner",
            );
        }

        let first = WarmPair::new().transition(synthetic_checkpoint(vec![73; ROWS * COLS], 3));
        assert_eq!(first.known_next.a_to_b_cadence.calc_count(), 4);
        assert_eq!(first.known_next.b_to_a_cadence.calc_count(), 4);
        assert_saturated_small_and_empty_lack(&first.known_next.a_to_b_work_rows);
        assert_saturated_small_and_empty_lack(&first.known_next.b_to_a_work_rows);

        let second_inputs = first
            .known_next
            .into_checkpoint(synthetic_current(vec![80; ROWS * COLS]));
        let second = WarmPair::new().transition(second_inputs);
        assert_eq!(
            second.estimate.weighted_rows,
            WorkRowCounts {
                a_to_b_l2: Level::Two.patch_rows(),
                b_to_a_l2: Level::Two.patch_rows(),
                a_to_b_l1: Level::One.patch_rows(),
                b_to_a_l1: Level::One.patch_rows(),
            },
            "the second calculation must consume the first calculation's raw retained rows",
        );
        assert!(
            second
                .estimate
                .displacement
                .bytes()
                .chunks_exact(4)
                .all(|bytes| f32::from_ne_bytes(bytes.try_into().unwrap()).is_finite()),
        );
        assert!(
            second
                .known_next
                .references
                .bytes()
                .iter()
                .all(|value| *value == 75),
            "the second checkpoint must update the first calculation's retained reference",
        );
        assert_saturated_small_and_empty_lack(&second.known_next.a_to_b_work_rows);
        assert_saturated_small_and_empty_lack(&second.known_next.b_to_a_work_rows);
        assert_eq!(second.known_next.a_to_b_cadence.calc_count(), 5);
        assert_eq!(second.known_next.b_to_a_cadence.calc_count(), 5);
    }

    #[test]
    #[ignore = "requires KJERAG_ONE_XS_WARM_PAYLOAD with the authenticated run-06 payload and sibling evidence JSON"]
    fn accepted_run06_complete_warm_pair_reproduces_post_public_displacement_bit_exact() {
        use crate::flow::one_xs::temporal_median::MedianState;
        use sha2::{Digest as _, Sha256};

        const PAYLOAD_BYTES: usize = 7_238_679;
        const PAYLOAD_SHA256: &str =
            "a5311370d7946d45094cc85c8ff6631bf2b1195e8c127e3b1f909310168ce8c1";
        const EVIDENCE_BYTES: usize = 6_719_162;
        const EVIDENCE_SHA256: &str =
            "9f8f531071a5bfcd03d6ff3eb73fc4f57b2d99b02a5dfecc0bc9497418da6baf";
        const EXPECTED_DISPLACEMENT_SHA256: &str =
            "4d18ab99b89ef69dd319c29f2aebe6b89f25743eac226a647f0b970cdbd50c31";

        const POSTBLUR_A: CorpusRecord = CorpusRecord::new(
            "shared_postblur_A",
            37,
            64_800,
            "c802e909b7970e8e94d77c398fd3faba9040f194c190101b15f4a3e3ac201a71",
        );
        const POSTBLUR_B: CorpusRecord = CorpusRecord::new(
            "shared_postblur_B",
            64_866,
            64_800,
            "0101d1c611bb5c2234019b9c5bdb4f0a34197017b782414cfd6f4b485d5e4b9d",
        );
        const PRIOR_A: CorpusRecord = CorpusRecord::new(
            "shared_prior_A_before_update",
            129_706,
            64_800,
            "3a6f8053d75f83df8e3eb66577b22d79c0642af941b028dead924eac3c01ffca",
        );
        const PRIOR_B: CorpusRecord = CorpusRecord::new(
            "shared_prior_B_before_update",
            194_546,
            64_800,
            "968db1f828fc149816755e79e600dde23f610234181fd4fee12e8337e892cc78",
        );
        const MASK_A: CorpusRecord = CorpusRecord::new(
            "shared_physical_mask_0",
            259_380,
            64_800,
            "20a85a8bf17041939185190513e14057ef31827a3c343e379ee8deea6f1a6d50",
        );
        const MASK_B: CorpusRecord = CorpusRecord::new(
            "shared_physical_mask_1",
            324_214,
            64_800,
            "20a85a8bf17041939185190513e14057ef31827a3c343e379ee8deea6f1a6d50",
        );
        const AB_PRIVATE_HINT_MASK: CorpusRecord = CorpusRecord::new(
            "ab_private_hint_densification_mask",
            1_490_781,
            16_200,
            "d669875402f653cece7ea45a3774a15c8eba0a5d006872d3bb8e02c3a0779ae2",
        );
        const AB_ROWS_L1: CorpusRecord = CorpusRecord::new(
            "ab_retained_rows_d8_1",
            1_507_070,
            24,
            "2c26920da6fe8c3f03a7c57f0d17fd8046113223d66f5fdc419e7381bf95e70d",
        );
        const AB_ROWS_L2: CorpusRecord = CorpusRecord::new(
            "ab_retained_rows_d8_2",
            1_507_127,
            16,
            "de3ae02d748c43ef1b0ac89c7c15214456e2d756fa76b33008ba01dc23b76aee",
        );
        const AB_PRE_SMALL_ROWS: CorpusRecord = CorpusRecord::new(
            "ab_pre_rows_108",
            6_120_994,
            24,
            "2c26920da6fe8c3f03a7c57f0d17fd8046113223d66f5fdc419e7381bf95e70d",
        );
        const AB_PRE_LACK_ROWS: CorpusRecord = CorpusRecord::new(
            "ab_pre_rows_120",
            6_121_045,
            24,
            "b2a3b6c6b66c60b2e64bf113dbbc996112d75c53a050f54288ff11bf304126de",
        );
        const AB_HINT_L2_U: CorpusRecord = CorpusRecord::new(
            "ab_l2_pre_pis_aux_hint_u",
            1_539_643,
            16_200,
            "7a2813955bf6fb5dc6c7a18aac769739efb2f8b110d948e9a6b4f35213b5faa1",
        );
        const AB_HINT_L2_V: CorpusRecord = CorpusRecord::new(
            "ab_l2_pre_pis_aux_hint_v",
            1_555_879,
            16_200,
            "d80c98b0df1d8ba16cb67fb5bbe6e6400e9d7ceb8c2ef6f974bff2fed3503f17",
        );
        const AB_HINT_L1_U: CorpusRecord = CorpusRecord::new(
            "ab_l1_pre_pis_aux_hint_u",
            1_963_563,
            64_800,
            "939a4e623432161a1f392eac4b39cfb82109aa61935c99a8e01826ada5073f6f",
        );
        const AB_HINT_L1_V: CorpusRecord = CorpusRecord::new(
            "ab_l1_pre_pis_aux_hint_v",
            2_028_399,
            64_800,
            "4ebf756ce28a09a3af31ed44b5d271891def4311825ca120a8597f58a2e688c4",
        );
        const AB_NEXT_HINT_L2_U: CorpusRecord = CorpusRecord::new(
            "ab_l2_post_calc_hint_aux_u",
            2_104_699,
            16_200,
            "9d38760cd22b0af9ff6fd06d6f498abcb19b84e5da151eb6a1e18e05ff5bd623",
        );
        const AB_NEXT_HINT_L2_V: CorpusRecord = CorpusRecord::new(
            "ab_l2_post_calc_hint_aux_v",
            2_120_937,
            16_200,
            "7c00ada1a5ec57313bf7376609ad56febc15452ad64ad760cf213a94a7219793",
        );
        const AB_NEXT_HINT_L1_U: CorpusRecord = CorpusRecord::new(
            "ab_l1_post_calc_hint_aux_u",
            2_137_175,
            64_800,
            "9cfb90ffaf883907a6853d5c50b5d2dc40c89b913681eeea9824f765e973f81e",
        );
        const AB_NEXT_HINT_L1_V: CorpusRecord = CorpusRecord::new(
            "ab_l1_post_calc_hint_aux_v",
            2_202_013,
            64_800,
            "28f56c21dbe7806e1c37124ffb2f23a5c023a4e214fdc765556974b9ace44f7f",
        );
        const AB_POST_TEMPORAL_U: CorpusRecord = CorpusRecord::new(
            "ab_post_temporal_median_sparse_u",
            2_266_857,
            5_696,
            "3e3a6d2c0d29b7a52180a623ad16bf357edc92fd63bf4d77447f4ebf8695d2e4",
        );
        const BA_PRIVATE_HINT_MASK: CorpusRecord = CorpusRecord::new(
            "ba_private_hint_densification_mask",
            2_661_625,
            16_200,
            "d669875402f653cece7ea45a3774a15c8eba0a5d006872d3bb8e02c3a0779ae2",
        );
        const BA_ROWS_L1: CorpusRecord = CorpusRecord::new(
            "ba_retained_rows_d8_1",
            2_677_914,
            24,
            "2c26920da6fe8c3f03a7c57f0d17fd8046113223d66f5fdc419e7381bf95e70d",
        );
        const BA_ROWS_L2: CorpusRecord = CorpusRecord::new(
            "ba_retained_rows_d8_2",
            2_677_971,
            16,
            "de3ae02d748c43ef1b0ac89c7c15214456e2d756fa76b33008ba01dc23b76aee",
        );
        const BA_PRE_SMALL_ROWS: CorpusRecord = CorpusRecord::new(
            "ba_pre_rows_108",
            6_722_411,
            24,
            "2c26920da6fe8c3f03a7c57f0d17fd8046113223d66f5fdc419e7381bf95e70d",
        );
        const BA_PRE_LACK_ROWS: CorpusRecord = CorpusRecord::new(
            "ba_pre_rows_120",
            6_722_462,
            24,
            "7addf7c58f8048efe4e7c5680a8bbe97f2328b50ba8755c0b6a9fb8e9d1d33b9",
        );
        const BA_HINT_L2_U: CorpusRecord = CorpusRecord::new(
            "ba_l2_pre_pis_aux_hint_u",
            2_710_487,
            16_200,
            "42a7cc46e69db0c983b147a77df353c8c00df46751ee40f583a7390599e24de1",
        );
        const BA_HINT_L2_V: CorpusRecord = CorpusRecord::new(
            "ba_l2_pre_pis_aux_hint_v",
            2_726_723,
            16_200,
            "fea54e47213c53ffcfaf558d811db5cbd5bb10d39349d1c1d6b5b05935b0b909",
        );
        const BA_HINT_L1_U: CorpusRecord = CorpusRecord::new(
            "ba_l1_pre_pis_aux_hint_u",
            3_134_407,
            64_800,
            "fe4f617b194f322035872ff664089cfee480eb9e77df3dbf1a5a8d9906016408",
        );
        const BA_HINT_L1_V: CorpusRecord = CorpusRecord::new(
            "ba_l1_pre_pis_aux_hint_v",
            3_199_243,
            64_800,
            "620f632a9e30408561b2339473fe9d7df736b470d3b34008d3bfdb1a19bd7fd2",
        );
        const BA_NEXT_HINT_L2_U: CorpusRecord = CorpusRecord::new(
            "ba_l2_post_calc_hint_aux_u",
            3_275_543,
            16_200,
            "e396d5dc0d3396eb9acf8084b1a71fd6e95c7b5c62c966b33eb25429330991d6",
        );
        const BA_NEXT_HINT_L2_V: CorpusRecord = CorpusRecord::new(
            "ba_l2_post_calc_hint_aux_v",
            3_291_781,
            16_200,
            "865a0a60e1db828c648b46bbe2d2a9f1c6938efa881ed29e9a228f70788b594f",
        );
        const BA_NEXT_HINT_L1_U: CorpusRecord = CorpusRecord::new(
            "ba_l1_post_calc_hint_aux_u",
            3_308_019,
            64_800,
            "7ab5f44b462df0ef1d6e707f19adda5bb148f3e3efad12c05b92ec8ade1187f4",
        );
        const BA_NEXT_HINT_L1_V: CorpusRecord = CorpusRecord::new(
            "ba_l1_post_calc_hint_aux_v",
            3_372_857,
            64_800,
            "8e98d659483d883f3054291dfe44a16f9e36220025da87bb479272e803a0242a",
        );
        const BA_POST_TEMPORAL_U: CorpusRecord = CorpusRecord::new(
            "ba_post_temporal_median_sparse_u",
            3_437_701,
            5_696,
            "f81744fbe6f7e7a76b42c9daeaf04c0f40b5c75f1abaac46a175255d3d47aa93",
        );
        const PRIOR_PUBLIC_AB: CorpusRecord = CorpusRecord::new(
            "shared_pre_public_c30",
            3_832_456,
            518_400,
            "d7cd591d6e96469ddb5ef91152b9643554cf6b0435831083d51fdbfc055cd319",
        );
        const PRIOR_PUBLIC_BA: CorpusRecord = CorpusRecord::new(
            "shared_pre_public_c90",
            4_350_889,
            518_400,
            "4d670d4a3ed16675b52c96bb4ebd807c196f77754b1c51161513b8ecfdecd26c",
        );
        const EXPECTED_AB: CorpusRecord = CorpusRecord::new(
            "shared_post_cpu_blend_public_c30_ab",
            4_998_998,
            518_400,
            "d1f8c04d625bd7a17951110eac1bdc3148fdb8230cd49c1ce6ec25ed020e6a28",
        );
        const EXPECTED_BA: CorpusRecord = CorpusRecord::new(
            "shared_post_cpu_blend_public_c90_ba",
            5_517_445,
            518_400,
            "ef9d7e22e841c8983d2939d9d8d98c41f072c928fa5f07641e6d74c1bb7de3c8",
        );
        const POST_REFERENCE_A: CorpusRecord = CorpusRecord::new(
            "shared_post_prior_A",
            4_869_320,
            64_800,
            "25db1917049e45f1eb6d6d34ecb7b689e6020a8e11645008fc8558e7212e1efc",
        );
        const POST_REFERENCE_B: CorpusRecord = CorpusRecord::new(
            "shared_post_prior_B",
            4_934_151,
            64_800,
            "5dd6333399b631c17f9cc269e7f627e388460b9a6fe27872467b5b280b007e81",
        );
        const AB_HISTORY: CorpusRecord = CorpusRecord::new(
            "ab_temporal_pre.histograms_u8",
            6_121_162,
            226_416,
            "568a33dc067a1a2528299ac668b193204607b6625e7061c8a6988e4876c0f8c6",
        );
        const AB_OFFSETS: CorpusRecord = CorpusRecord::new(
            "ab_temporal_pre.history_offsets_u32le",
            6_347_627,
            5_700,
            "6766b11f7b2e6307feded8bc831e23795470b25fa20f260d5c888fdc03913604",
        );
        const AB_VALUES: CorpusRecord = CorpusRecord::new(
            "ab_temporal_pre.history_values_f32le",
            6_353_375,
            17_088,
            "c40f1c63ff9e6a55e2094ccbccb3edf638a1ad620ac9f81c918ef9d7b5646c2d",
        );
        const AB_POST_HISTORY: CorpusRecord = CorpusRecord::new(
            "ab_temporal_post.histograms_u8",
            6_379_230,
            226_416,
            "c1f39197531efd5790132caeba36a0c4231cc357477c5a88b0f669bdaadcb5f0",
        );
        const AB_POST_OFFSETS: CorpusRecord = CorpusRecord::new(
            "ab_temporal_post.history_offsets_u32le",
            6_605_696,
            5_700,
            "6766b11f7b2e6307feded8bc831e23795470b25fa20f260d5c888fdc03913604",
        );
        const AB_POST_VALUES: CorpusRecord = CorpusRecord::new(
            "ab_temporal_post.history_values_f32le",
            6_611_445,
            17_088,
            "b7e6a9186753673765795a2d671897d83621c1e93675ced5435e85834b587309",
        );
        const BA_HISTORY: CorpusRecord = CorpusRecord::new(
            "ba_temporal_pre.histograms_u8",
            6_722_579,
            226_416,
            "b6acfbfb87bb69e5bb1682b0bb306a23ddec460b47bebbc5ca3bc96c60696f15",
        );
        const BA_OFFSETS: CorpusRecord = CorpusRecord::new(
            "ba_temporal_pre.history_offsets_u32le",
            6_949_044,
            5_700,
            "6766b11f7b2e6307feded8bc831e23795470b25fa20f260d5c888fdc03913604",
        );
        const BA_VALUES: CorpusRecord = CorpusRecord::new(
            "ba_temporal_pre.history_values_f32le",
            6_954_792,
            17_088,
            "1910ba64d9945423460dd88d845dc91689c687186474796b29722b834632b6d2",
        );
        const BA_POST_HISTORY: CorpusRecord = CorpusRecord::new(
            "ba_temporal_post.histograms_u8",
            6_980_647,
            226_416,
            "9d87b7ed7e00d7c4e9d55093cc8f080a895fc4e00a844683caea36820dbc8dd7",
        );
        const BA_POST_OFFSETS: CorpusRecord = CorpusRecord::new(
            "ba_temporal_post.history_offsets_u32le",
            7_207_113,
            5_700,
            "6766b11f7b2e6307feded8bc831e23795470b25fa20f260d5c888fdc03913604",
        );
        const BA_POST_VALUES: CorpusRecord = CorpusRecord::new(
            "ba_temporal_post.history_values_f32le",
            7_212_862,
            17_088,
            "96bac1fad8c1fa82fa485d68db4b0caadb6add51aa9cdbf0f5f0a45727212fb7",
        );
        const AB_POST_SMALL_ROWS: CorpusRecord = CorpusRecord::new(
            "ab_post_rows_108",
            6_121_097,
            24,
            "2c26920da6fe8c3f03a7c57f0d17fd8046113223d66f5fdc419e7381bf95e70d",
        );
        const BA_POST_SMALL_ROWS: CorpusRecord = CorpusRecord::new(
            "ba_post_rows_108",
            6_722_514,
            24,
            "2c26920da6fe8c3f03a7c57f0d17fd8046113223d66f5fdc419e7381bf95e70d",
        );

        let path = std::path::PathBuf::from(std::env::var_os("KJERAG_ONE_XS_WARM_PAYLOAD").expect(
            "set KJERAG_ONE_XS_WARM_PAYLOAD to authenticated run-06 warm-pair-payload.bin",
        ));
        let payload = std::fs::read(&path).expect("could not read authenticated run-06 payload");
        assert_eq!(payload.len(), PAYLOAD_BYTES);
        assert_eq!(&payload[..8], b"KJWP602\x04");
        assert_eq!(sha256_hex(&payload), PAYLOAD_SHA256);
        let evidence = std::fs::read(path.with_file_name("warm-pair-evidence.json"))
            .expect("could not read authenticated run-06 evidence JSON beside the payload");
        assert_eq!(evidence.len(), EVIDENCE_BYTES);
        assert_eq!(sha256_hex(&evidence), EVIDENCE_SHA256);

        let current = ColdInputs::from_prepared(
            LensPair {
                a: record_bytes(&payload, POSTBLUR_A).to_vec(),
                b: record_bytes(&payload, POSTBLUR_B).to_vec(),
            },
            LensPair {
                a: record_bytes(&payload, MASK_A).to_vec(),
                b: record_bytes(&payload, MASK_B).to_vec(),
            },
        )
        .expect("accepted run-06 post-blur inputs have retained-grid shape");
        let computed_shared_small_rows = {
            let masks = MaskPyramid::build(&current);
            let a_to_b = LevelInputs::build::<AtoB>(&current, &masks, Level::One);
            let b_to_a = LevelInputs::build::<BtoA>(&current, &masks, Level::One);
            let a_to_b = classify_small_disparity_rows(
                &record_f32(&payload, AB_POST_TEMPORAL_U),
                &a_to_b.small_disparity_block_mask(Level::One),
            );
            let b_to_a = classify_small_disparity_rows(
                &record_f32(&payload, BA_POST_TEMPORAL_U),
                &b_to_a.small_disparity_block_mask(Level::One),
            );
            let encode = |rows: &[bool]| {
                encoded_rows(
                    Level::One,
                    &rows
                        .iter()
                        .enumerate()
                        .filter_map(|(row, enabled)| enabled.then_some(row))
                        .collect::<Vec<_>>(),
                )
            };
            let a_to_b = encode(&a_to_b);
            let b_to_a = encode(&b_to_a);
            a_to_b
                .iter()
                .zip(&b_to_a)
                .map(|(a, b)| a | b)
                .collect::<Vec<_>>()
        };
        let references = BlurredBelts::from_lenses(LensPair {
            a: record_bytes(&payload, PRIOR_A).to_vec(),
            b: record_bytes(&payload, PRIOR_B).to_vec(),
        })
        .expect("accepted run-06 prior references have retained-grid shape");

        let cadence = EmptyOverrideCadence::new(6_372, 10).unwrap();
        let a_to_b_rows = RetainedWorkRows::<AtoB>::from_lsb0_bytes(
            Some(record_bytes(&payload, AB_PRE_SMALL_ROWS)),
            record_bytes(&payload, AB_PRE_LACK_ROWS),
        )
        .unwrap();
        assert_eq!(
            a_to_b_rows.effective(),
            EffectiveWorkRows::from_lsb0_bytes(
                record_bytes(&payload, AB_ROWS_L1),
                record_bytes(&payload, AB_ROWS_L2),
            )
            .unwrap(),
            "accepted Run06 AB d8 must be derived from retained +0x108/+0x120",
        );
        let a_to_b = WarmDirection::new(
            public_field::<AtoB>(&payload, PRIOR_PUBLIC_AB),
            a_to_b_rows,
            HintPyramid::from_levels(
                DenseHintLevel::from_row_major_components(
                    record_f32(&payload, AB_HINT_L1_U),
                    record_f32(&payload, AB_HINT_L1_V),
                ),
                DenseHintLevel::from_row_major_components(
                    record_f32(&payload, AB_HINT_L2_U),
                    record_f32(&payload, AB_HINT_L2_V),
                ),
            )
            .unwrap(),
            cadence,
        );
        let b_to_a_rows = RetainedWorkRows::<BtoA>::from_lsb0_bytes(
            Some(record_bytes(&payload, BA_PRE_SMALL_ROWS)),
            record_bytes(&payload, BA_PRE_LACK_ROWS),
        )
        .unwrap();
        assert_eq!(
            b_to_a_rows.effective(),
            EffectiveWorkRows::from_lsb0_bytes(
                record_bytes(&payload, BA_ROWS_L1),
                record_bytes(&payload, BA_ROWS_L2),
            )
            .unwrap(),
            "accepted Run06 BA d8 must be derived from retained +0x108/+0x120",
        );
        let b_to_a = WarmDirection::new(
            public_field::<BtoA>(&payload, PRIOR_PUBLIC_BA),
            b_to_a_rows,
            HintPyramid::from_levels(
                DenseHintLevel::from_row_major_components(
                    record_f32(&payload, BA_HINT_L1_U),
                    record_f32(&payload, BA_HINT_L1_V),
                ),
                DenseHintLevel::from_row_major_components(
                    record_f32(&payload, BA_HINT_L2_U),
                    record_f32(&payload, BA_HINT_L2_V),
                ),
            )
            .unwrap(),
            cadence,
        );
        let medians = TemporalMedians::from_states(
            MedianState::<AtoB>::from_parts(
                record_bytes(&payload, AB_HISTORY).to_vec(),
                record_u32(&payload, AB_OFFSETS),
                record_f32(&payload, AB_VALUES),
            )
            .unwrap(),
            MedianState::<BtoA>::from_parts(
                record_bytes(&payload, BA_HISTORY).to_vec(),
                record_u32(&payload, BA_OFFSETS),
                record_f32(&payload, BA_VALUES),
            )
            .unwrap(),
        )
        .unwrap();

        let WarmTransition {
            estimate: result,
            known_next,
        } = WarmPair::new().transition(WarmCheckpointInputs::new(
            current, references, a_to_b, b_to_a, medians,
        ));
        assert_eq!(
            result.weighted_rows,
            WorkRowCounts {
                a_to_b_l2: 88,
                b_to_a_l2: 88,
                a_to_b_l1: 178,
                b_to_a_l1: 178,
            }
        );
        assert_eq!(
            result.invalid_nodes,
            InvalidNodeCounts {
                a_to_b: 0,
                b_to_a: 0,
            }
        );

        let expected_ba = record_interleaved_planes(&payload, EXPECTED_BA);
        let expected_ab = record_interleaved_planes(&payload, EXPECTED_AB);
        assert_plane_bits(
            "next AB.dcol",
            known_next.a_to_b_public.dcol(),
            &expected_ab.0,
        );
        assert_plane_bits(
            "next AB.drow",
            known_next.a_to_b_public.drow(),
            &expected_ab.1,
        );
        assert_plane_bits(
            "next BA.dcol",
            known_next.b_to_a_public.dcol(),
            &expected_ba.0,
        );
        assert_plane_bits(
            "next BA.drow",
            known_next.b_to_a_public.drow(),
            &expected_ba.1,
        );
        let private_hint_mask = dense::private_hint_mask();
        assert_eq!(
            private_hint_mask.as_ref(),
            record_bytes(&payload, AB_PRIVATE_HINT_MASK),
        );
        assert_eq!(
            private_hint_mask.as_ref(),
            record_bytes(&payload, BA_PRIVATE_HINT_MASK),
        );
        assert_plane_bits(
            "next AB L1 hint dcol",
            known_next.a_to_b_hints.level(Level::One).dcol(),
            &record_f32(&payload, AB_NEXT_HINT_L1_U),
        );
        assert_plane_bits(
            "next AB L1 hint drow",
            known_next.a_to_b_hints.level(Level::One).drow(),
            &record_f32(&payload, AB_NEXT_HINT_L1_V),
        );
        assert_plane_bits(
            "next AB L2 hint dcol",
            known_next.a_to_b_hints.level(Level::Two).dcol(),
            &record_f32(&payload, AB_NEXT_HINT_L2_U),
        );
        assert_plane_bits(
            "next AB L2 hint drow",
            known_next.a_to_b_hints.level(Level::Two).drow(),
            &record_f32(&payload, AB_NEXT_HINT_L2_V),
        );
        assert_plane_bits(
            "next BA L1 hint dcol",
            known_next.b_to_a_hints.level(Level::One).dcol(),
            &record_f32(&payload, BA_NEXT_HINT_L1_U),
        );
        assert_plane_bits(
            "next BA L1 hint drow",
            known_next.b_to_a_hints.level(Level::One).drow(),
            &record_f32(&payload, BA_NEXT_HINT_L1_V),
        );
        assert_plane_bits(
            "next BA L2 hint dcol",
            known_next.b_to_a_hints.level(Level::Two).dcol(),
            &record_f32(&payload, BA_NEXT_HINT_L2_U),
        );
        assert_plane_bits(
            "next BA L2 hint drow",
            known_next.b_to_a_hints.level(Level::Two).drow(),
            &record_f32(&payload, BA_NEXT_HINT_L2_V),
        );
        assert_eq!(
            computed_shared_small_rows,
            record_bytes(&payload, AB_POST_SMALL_ROWS),
        );
        assert_eq!(
            computed_shared_small_rows,
            record_bytes(&payload, BA_POST_SMALL_ROWS),
        );
        assert_eq!(
            encoded_bool_rows(
                Level::One,
                known_next
                    .a_to_b_work_rows
                    .small_disparity_rows()
                    .expect("accepted mature Run06 AB +0x108 is present"),
            ),
            record_bytes(&payload, AB_POST_SMALL_ROWS),
        );
        assert_eq!(
            encoded_bool_rows(
                Level::One,
                known_next
                    .b_to_a_work_rows
                    .small_disparity_rows()
                    .expect("accepted mature Run06 BA +0x108 is present"),
            ),
            record_bytes(&payload, BA_POST_SMALL_ROWS),
        );
        assert_eq!(
            encoded_bool_rows(
                Level::One,
                known_next.a_to_b_work_rows.lack_of_texture_rows(),
            ),
            record_bytes(&payload, AB_PRE_LACK_ROWS),
            "accepted nonzero Run06 AB must retain first-calc +0x120",
        );
        assert_eq!(
            encoded_bool_rows(
                Level::One,
                known_next.b_to_a_work_rows.lack_of_texture_rows(),
            ),
            record_bytes(&payload, BA_PRE_LACK_ROWS),
            "accepted nonzero Run06 BA must retain first-calc +0x120",
        );
        let a_to_b_next_effective = known_next.a_to_b_work_rows.effective();
        let b_to_a_next_effective = known_next.b_to_a_work_rows.effective();
        for level in [Level::One, Level::Two] {
            let expected = vec![CostMode::Weighted; level.patch_rows()];
            assert_eq!(
                a_to_b_next_effective.modes(level),
                expected.as_slice(),
                "accepted Run06 next AB {level} work rows",
            );
            assert_eq!(
                b_to_a_next_effective.modes(level),
                expected.as_slice(),
                "accepted Run06 next BA {level} work rows",
            );
        }
        assert_eq!(
            known_next.references.lens(crate::flow::one_xs::Lens::A),
            record_bytes(&payload, POST_REFERENCE_A),
        );
        assert_eq!(
            known_next.references.lens(crate::flow::one_xs::Lens::B),
            record_bytes(&payload, POST_REFERENCE_B),
        );
        assert_eq!(
            known_next.a_to_b_median.histogram(),
            record_bytes(&payload, AB_POST_HISTORY),
        );
        assert_eq!(
            known_next.a_to_b_median.offsets(),
            record_u32(&payload, AB_POST_OFFSETS),
        );
        assert_plane_bits(
            "next AB median values",
            known_next.a_to_b_median.values(),
            &record_f32(&payload, AB_POST_VALUES),
        );
        assert_eq!(
            known_next.b_to_a_median.histogram(),
            record_bytes(&payload, BA_POST_HISTORY),
        );
        assert_eq!(
            known_next.b_to_a_median.offsets(),
            record_u32(&payload, BA_POST_OFFSETS),
        );
        assert_plane_bits(
            "next BA median values",
            known_next.b_to_a_median.values(),
            &record_f32(&payload, BA_POST_VALUES),
        );
        assert_eq!(known_next.a_to_b_cadence.calc_count(), 6_373);
        assert_eq!(known_next.b_to_a_cadence.calc_count(), 6_373);
        assert_eq!(known_next.a_to_b_cadence.cadence(), 10);
        assert_eq!(known_next.b_to_a_cadence.cadence(), 10);
        let plane_pixels = ROWS * COLS;
        let planes = result.displacement.planes();
        assert_plane_bits("BA.dcol", &planes[..plane_pixels], &expected_ba.0);
        assert_plane_bits(
            "BA.drow",
            &planes[plane_pixels..2 * plane_pixels],
            &expected_ba.1,
        );
        assert_plane_bits(
            "AB.dcol",
            &planes[2 * plane_pixels..3 * plane_pixels],
            &expected_ab.0,
        );
        assert_plane_bits("AB.drow", &planes[3 * plane_pixels..], &expected_ab.1);
        let displacement_le = planes
            .iter()
            .flat_map(|value| value.to_bits().to_le_bytes())
            .collect::<Vec<_>>();
        assert_eq!(sha256_hex(&displacement_le), EXPECTED_DISPLACEMENT_SHA256);

        fn sha256_hex(bytes: &[u8]) -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }
    }

    fn record_bytes(payload: &[u8], record: CorpusRecord) -> &[u8] {
        use sha2::{Digest as _, Sha256};

        let label_start = record
            .offset
            .checked_sub(record.label.len())
            .expect("run-06 record label offset underflowed");
        let header_start = label_start
            .checked_sub(12)
            .expect("run-06 record header offset underflowed");
        let end = record
            .offset
            .checked_add(record.bytes)
            .expect("run-06 record end overflowed");
        assert!(
            end <= payload.len(),
            "{} lies outside payload",
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
            "{} label differs",
            record.label,
        );
        let bytes = &payload[record.offset..end];
        let hash = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(hash, record.sha256, "{} SHA-256 differs", record.label);
        bytes
    }

    fn record_f32(payload: &[u8], record: CorpusRecord) -> Vec<f32> {
        record_bytes(payload, record)
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect()
    }

    fn record_u32(payload: &[u8], record: CorpusRecord) -> Vec<u32> {
        record_bytes(payload, record)
            .chunks_exact(4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
            .collect()
    }

    fn record_interleaved_planes(payload: &[u8], record: CorpusRecord) -> (Vec<f32>, Vec<f32>) {
        let values = record_bytes(payload, record);
        let mut dcol = Vec::with_capacity(ROWS * COLS);
        let mut drow = Vec::with_capacity(ROWS * COLS);
        for node in values.chunks_exact(8) {
            dcol.push(f32::from_le_bytes(node[..4].try_into().unwrap()));
            drow.push(f32::from_le_bytes(node[4..].try_into().unwrap()));
        }
        assert_eq!(dcol.len(), ROWS * COLS, "{} node count", record.label);
        (dcol, drow)
    }

    fn public_field<D: PisDirection>(payload: &[u8], record: CorpusRecord) -> PublicDenseField<D> {
        let (dcol, drow) = record_interleaved_planes(payload, record);
        PublicDenseField::from_row_major_components(dcol, drow).unwrap()
    }

    fn assert_plane_bits(label: &str, actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len(), "{label} length differs");
        for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(
                actual.to_bits(),
                expected.to_bits(),
                "{label} first mismatch at node {index}: actual=0x{:08x}, expected=0x{:08x}",
                actual.to_bits(),
                expected.to_bits(),
            );
        }
    }
}
