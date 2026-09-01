//! Scalar reference boundary for the selected ONE X2 patch inverse search.
//!
//! This is deliberately separate from the legacy [`crate::flow::dis`] search.
//! The selected native route solves only pyramid levels one and two of the
//! 1,080-row by 60-column retained belt. Both levels use 8 by 8 patches on a
//! stride-three grid, one global stripe, and two in-place passes: forward with
//! left/top propagation, then backward with right/bottom propagation.
//!
//! The recovered weighted and unweighted candidate-score routes are
//! implemented here, including the native physical-mask ordering. A-to-B swaps
//! neither images nor masks. B-to-A swaps the images but deliberately continues
//! to pass masks A then B. Every dynamic residual tap requires both the
//! source-slot mask and the integer, floor-warped target-slot mask. Prepared
//! gradients and weights are already zero beneath physical mask A; the
//! Hessian, gradient sums, and weight denominator traverse that complete
//! prepared patch without target-mask pruning. Masked native routes use the
//! surviving dual-mask tap count for candidate and descent mean correction;
//! candidates with at most eight taps receive the finite sentinel score.
//!
//! Weighted and unweighted candidate scores reproduce their selected Apple
//! ARMv8.2 helpers' four-lane arithmetic. Descent remains the selected scalar
//! helper's readable arithmetic. Dense voting, variational refinement,
//! temporal filtering, GPU dispatch, and Scene activation remain outside this
//! module.

use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use super::{COLS, Direction, LensPair, PATCH_SIZE, PATCH_STRIDE, ROWS};

/// Correctness-qualified GPU execution of the paired sparse search boundary.
///
/// This remains render-internal until the production Scene transaction is
/// ready to qualify the complete estimator handoff.
#[allow(dead_code)]
pub(crate) mod gpu;

/// The finite score native PIS uses when a candidate has at most eight taps.
pub const SENTINEL_SCORE: f32 = 1.0e10;

/// Nine taps are enough to score; eight retain the finite sentinel.
pub const MIN_SCORE_SURVIVORS: usize = 9;

/// Each of the two spatial passes receives half of the selected 12 descents.
pub const DESCENTS_PER_PASS: usize = 6;

/// The selected retained pyramid contains level one and level two only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    One,
    Two,
}

impl Level {
    pub const fn index(self) -> usize {
        match self {
            Self::One => 1,
            Self::Two => 2,
        }
    }

    pub const fn rows(self) -> usize {
        ROWS >> self.index()
    }

    pub const fn cols(self) -> usize {
        COLS >> self.index()
    }

    pub const fn pixels(self) -> usize {
        self.rows() * self.cols()
    }

    pub const fn patch_rows(self) -> usize {
        (self.rows() - PATCH_SIZE) / PATCH_STRIDE + 1
    }

    pub const fn patch_cols(self) -> usize {
        (self.cols() - PATCH_SIZE) / PATCH_STRIDE + 1
    }

    pub const fn patches(self) -> usize {
        self.patch_rows() * self.patch_cols()
    }
}

impl fmt::Display for Level {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "level {}", self.index())
    }
}

mod direction_sealed {
    pub trait Sealed {}
}

/// Compile-time identity of one selected ONE X2 solver direction.
///
/// This trait is sealed so only the two native directions can label sparse
/// inputs and outputs. Consumers such as the temporal median may be generic
/// over it without permitting a third, unproved direction.
pub trait PisDirection:
    direction_sealed::Sealed + Clone + Copy + fmt::Debug + PartialEq + Eq + 'static
{
    const DIRECTION: Direction;
}

/// A-to-B sparse solver identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AtoB;

impl direction_sealed::Sealed for AtoB {}

impl PisDirection for AtoB {
    const DIRECTION: Direction = Direction::AtoB;
}

/// B-to-A sparse solver identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BtoA;

impl direction_sealed::Sealed for BtoA {}

impl PisDirection for BtoA {
    const DIRECTION: Direction = Direction::BtoA;
}

/// One sparse retained-grid displacement.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flow {
    dcol: f32,
    drow: f32,
}

impl Flow {
    pub const ZERO: Self = Self {
        dcol: 0.0,
        drow: 0.0,
    };

    /// Admit a finite sparse displacement in this pyramid level's pixels.
    pub fn new(dcol: f32, drow: f32) -> Option<Self> {
        (dcol.is_finite() && drow.is_finite()).then_some(Self { dcol, drow })
    }

    pub const fn dcol(self) -> f32 {
        self.dcol
    }

    pub const fn drow(self) -> f32 {
        self.drow
    }

    fn stepped(self, delta: Self) -> Self {
        Self {
            dcol: self.dcol - delta.dcol,
            drow: self.drow - delta.drow,
        }
    }

    fn distance(self, other: Self) -> f64 {
        f64::from(self.dcol - other.dcol).hypot(f64::from(self.drow - other.drow))
    }
}

/// SSD route selected for every patch in one sparse grid row.
///
/// Native carries this choice in its per-level work-row table. That table is
/// still an input to this oracle: callers must not infer weighted rows here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CostMode {
    /// Each surviving tap contributes the raw target-minus-source difference.
    Unweighted,
    /// Each surviving tap uses its prepared weight divided by the full patch
    /// weight sum.
    Weighted,
}

/// The selected per-component disparity interval a refined patch must fall inside.
///
/// Section 120 reads this from the worker. `OpticalFlow::setMaxDisparity` writes a pair of
/// `cv::Vec2f` into the two FDS instances **with the pair exchanged**, and the search gates every
/// refined result on it at `0x2d3fafc..0x2d3fb44`:
///
/// ```text
/// fcmp  c, a ; fccmp c, b, #0, gt ; b.mi accept     ; c > a AND c < b
/// fcmp  c, a ; fccmp c, b, #4, mi ; b.le reject     ; the reversed-interval case
/// ```
///
/// The test is a **strict** interval applied to each component separately, and it is
/// **order-agnostic**: whichever endpoint is larger acts as the upper bound. That is exactly what
/// lets one pair produce opposite-signed ranges in the two directions.
///
/// The selected owner supplies the exact direction- and level-specific endpoints recovered from
/// the accepted row-6 table and the pinned worker's fill, scaling, and direction-exchange code.
/// Generic inputs may leave this gate unset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisparityInterval {
    first: [f32; 2],
    second: [f32; 2],
}

impl DisparityInterval {
    /// Admit the two endpoints in whichever order native holds them.
    pub const fn new(first: [f32; 2], second: [f32; 2]) -> Self {
        Self { first, second }
    }

    /// Whether both components lie strictly between their endpoints.
    const fn admits(self, flow: Flow) -> bool {
        const fn between(value: f32, a: f32, b: f32) -> bool {
            (value > a && value < b) || (value > b && value < a)
        }
        between(flow.dcol(), self.first[0], self.second[0])
            && between(flow.drow(), self.first[1], self.second[1])
    }
}

/// Prepared scalar inputs for one native direction at one selected level.
///
/// `images.a/images.b` and `masks.a/masks.b` are always supplied in physical
/// lens order. The constructor swaps only the images for B-to-A. The resulting
/// first and second mask slots remain A and B in both directions, matching the
/// selected Mac callbacks rather than following semantic source/target names.
#[derive(Clone, Debug)]
pub struct Input<D: PisDirection> {
    level: Level,
    source: Arc<Vec<u8>>,
    target: Arc<Vec<u8>>,
    source_slot_mask: Arc<Vec<u8>>,
    target_slot_mask: Arc<Vec<u8>>,
    gradient_col: Arc<Vec<f32>>,
    gradient_row: Arc<Vec<f32>>,
    raw_weight: Arc<Vec<f32>>,
    patch_weight_sums: Box<[f32]>,
    cost_modes: Box<[CostMode]>,
    disparity: Option<DisparityInterval>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> Input<D> {
    /// Arm the selected post-descent disparity gate (section 120J).
    ///
    /// Unset by default so generic inputs can exercise the ungated core. The
    /// selected scalar owner arms the recovered direction- and level-specific interval.
    #[must_use]
    pub fn with_disparity_interval(mut self, interval: DisparityInterval) -> Self {
        self.disparity = Some(interval);
        self
    }

    /// The armed interval, if any.
    pub const fn disparity_interval(&self) -> Option<DisparityInterval> {
        self.disparity
    }

    /// Admit one prepared native level.
    ///
    /// Gradients and weights belong to the direction's source image and are
    /// already prepared before this boundary. The constructor rejects
    /// non-finite values, finite negative raw weights, and any nonzero prepared
    /// value wherever physical mask slot A is zero, matching native
    /// preparation. Raw weights are prepared as `abs(dx) + abs(dy)`, blurred
    /// by the native 3 by 3 Gaussian at sigma 1, then zeroed by physical mask
    /// A, so negative values cannot be native-prepared inputs. The caller
    /// supplies one [`CostMode`] per sparse patch-grid row; this oracle does not
    /// infer that still-capture-gated native work-row state. It deliberately
    /// does not derive or mask either numerical plane again. In particular,
    /// structure terms consume every admitted gradient tap; target-mask
    /// survivors never prune those precomputed sums.
    #[allow(clippy::too_many_arguments)]
    pub fn from_native_order(
        level: Level,
        images: LensPair<Vec<u8>>,
        masks: LensPair<Vec<u8>>,
        gradient_col: Vec<f32>,
        gradient_row: Vec<f32>,
        raw_weight: Vec<f32>,
        cost_modes: Vec<CostMode>,
    ) -> Result<Self, InputError> {
        Self::from_shared_native_order(
            level,
            LensPair {
                a: Arc::new(images.a),
                b: Arc::new(images.b),
            },
            LensPair {
                a: Arc::new(masks.a),
                b: Arc::new(masks.b),
            },
            Arc::new(gradient_col),
            Arc::new(gradient_row),
            Arc::new(raw_weight),
            cost_modes,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_shared_native_order(
        level: Level,
        images: LensPair<Arc<Vec<u8>>>,
        masks: LensPair<Arc<Vec<u8>>>,
        gradient_col: Arc<Vec<f32>>,
        gradient_row: Arc<Vec<f32>>,
        raw_weight: Arc<Vec<f32>>,
        cost_modes: Vec<CostMode>,
    ) -> Result<Self, InputError> {
        let expected = level.pixels();
        for (part, actual) in [
            ("lens A image", images.a.len()),
            ("lens B image", images.b.len()),
            ("lens A mask", masks.a.len()),
            ("lens B mask", masks.b.len()),
            ("column gradients", gradient_col.len()),
            ("row gradients", gradient_row.len()),
            ("raw SSD weights", raw_weight.len()),
        ] {
            if actual != expected {
                return Err(ShapeError {
                    level,
                    part,
                    expected,
                    actual,
                }
                .into());
            }
        }
        let expected_cost_rows = level.patch_rows();
        if cost_modes.len() != expected_cost_rows {
            return Err(ShapeError {
                level,
                part: "patch-row cost mode",
                expected: expected_cost_rows,
                actual: cost_modes.len(),
            }
            .into());
        }

        for (part, values, require_nonnegative) in [
            ("column gradient", &gradient_col[..], false),
            ("row gradient", &gradient_row[..], false),
            ("raw SSD weight", &raw_weight[..], true),
        ] {
            for (index, value) in values.iter().copied().enumerate() {
                if !value.is_finite() {
                    return Err(InputError::NonFinitePreparedValue { level, part, index });
                }
                if require_nonnegative && value < 0.0 {
                    return Err(InputError::NegativeRawWeight { level, index });
                }
                if masks.a[index] == 0 && value != 0.0 {
                    return Err(InputError::NonZeroBelowPhysicalMaskA { level, part, index });
                }
            }
        }

        let LensPair {
            a: image_a,
            b: image_b,
        } = images;
        let (source, target) = match D::DIRECTION {
            Direction::AtoB => (image_a, image_b),
            Direction::BtoA => (image_b, image_a),
        };
        // This deliberately does not branch on direction. Mac swaps the two
        // image arguments for B-to-A but reuses the same A/B mask vector.
        let LensPair {
            a: source_slot_mask,
            b: target_slot_mask,
        } = masks;
        let patch_weight_sums = rolling_patch_weight_sums(level, &raw_weight);

        Ok(Self {
            level,
            source,
            target,
            source_slot_mask,
            target_slot_mask,
            gradient_col,
            gradient_row,
            raw_weight,
            patch_weight_sums: patch_weight_sums.into_boxed_slice(),
            cost_modes: cost_modes.into_boxed_slice(),
            disparity: None,
            direction: PhantomData,
        })
    }

    pub const fn level(&self) -> Level {
        self.level
    }

    pub const fn direction(&self) -> Direction {
        D::DIRECTION
    }

    /// Evaluate one candidate with its selected row's dual-mask cost mode.
    pub fn score(&self, patch_row: usize, patch_col: usize, flow: Flow) -> Score {
        assert!(
            patch_row < self.level.patch_rows(),
            "ONE X2 PIS patch row is outside the selected level",
        );
        assert!(
            patch_col < self.level.patch_cols(),
            "ONE X2 PIS patch column is outside the selected level",
        );
        self.candidate_score(patch_row, patch_col, flow)
    }

    fn candidate_score(&self, patch_row: usize, patch_col: usize, flow: Flow) -> Score {
        match self.cost_modes[patch_row] {
            CostMode::Weighted => self.weighted_candidate_score(patch_row, patch_col, flow),
            CostMode::Unweighted => self.unweighted_candidate_score(patch_row, patch_col, flow),
        }
    }

    /// Reproduce `computeSSDMeanNormWithoutWeight_asm` at worker
    /// `+0x2d482fc`. Its four persistent lanes each combine columns `i` and
    /// `i+4` across all eight rows. Unlike scalar descent, the helper forms
    /// four coefficient products, pairs them by source row, and multiplies a
    /// rejected tap by zero before reduction.
    fn unweighted_candidate_score(&self, patch_row: usize, patch_col: usize, flow: Flow) -> Score {
        let rows = self.level.rows();
        let cols = self.level.cols();
        let source_row = patch_row * PATCH_STRIDE;
        let source_col = patch_col * PATCH_STRIDE;
        let origin_row = padded_origin(source_row, flow.drow, rows);
        let origin_col = padded_origin(source_col, flow.dcol, cols);
        let sampling = PatchSamplingPlan::new(origin_row, origin_col, rows, cols);
        let mut reduction = CandidateReduction::default();

        for row in 0..PATCH_SIZE {
            let mut residuals = [0.0f32; PATCH_SIZE];
            let mut survivors = [false; PATCH_SIZE];
            for col in 0..PATCH_SIZE {
                let source_at = (source_row + row) * cols + source_col + col;
                let survives = self.source_slot_mask[source_at] != 0
                    && self.target_slot_mask[sampling.target_index(row, col)] != 0;
                residuals[col] = sampling.candidate_residual(
                    &self.target,
                    row,
                    col,
                    self.source[source_at],
                    survives,
                );
                survivors[col] = survives;
            }
            reduction.accumulate_row(residuals, survivors);
        }
        reduction.score()
    }

    /// Reproduce `computeSSDMeanNormWithWeight_asm` at worker
    /// `+0x2d48ab0`. The selected Mac dispatches here after its ARMv8.2
    /// capability check. Its four lanes each combine columns `i` and `i+4`,
    /// accumulate across the eight rows, then reduce pairwise.
    fn weighted_candidate_score(&self, patch_row: usize, patch_col: usize, flow: Flow) -> Score {
        let rows = self.level.rows();
        let cols = self.level.cols();
        let source_row = patch_row * PATCH_STRIDE;
        let source_col = patch_col * PATCH_STRIDE;
        let origin_row = padded_origin(source_row, flow.drow, rows);
        let origin_col = padded_origin(source_col, flow.dcol, cols);
        let sampling = PatchSamplingPlan::new(origin_row, origin_col, rows, cols);
        let patch_sum = self.patch_weight_sums[patch_row * self.level.patch_cols() + patch_col];
        let reciprocal_sum = if patch_sum > 0.0 {
            1.0 / patch_sum
        } else {
            0.0
        };
        let mut reduction = CandidateReduction::default();

        for row in 0..PATCH_SIZE {
            let mut residuals = [0.0f32; PATCH_SIZE];
            let mut survivors = [false; PATCH_SIZE];
            for col in 0..PATCH_SIZE {
                let source_at = (source_row + row) * cols + source_col + col;
                let survives = self.source_slot_mask[source_at] != 0
                    && self.target_slot_mask[sampling.target_index(row, col)] != 0;
                residuals[col] = sampling.weighted_candidate_residual(
                    &self.target,
                    row,
                    col,
                    self.source[source_at],
                    survives,
                    self.raw_weight[source_at],
                    reciprocal_sum,
                );
                survivors[col] = survives;
            }
            reduction.accumulate_row(residuals, survivors);
        }
        reduction.score()
    }

    fn descent_statistics(&self, patch_row: usize, patch_col: usize, flow: Flow) -> Statistics {
        let rows = self.level.rows();
        let cols = self.level.cols();
        let source_row = patch_row * PATCH_STRIDE;
        let source_col = patch_col * PATCH_STRIDE;
        let origin_row = padded_origin(source_row, flow.drow, rows);
        let origin_col = padded_origin(source_col, flow.dcol, cols);
        let sampling = PatchSamplingPlan::new(origin_row, origin_col, rows, cols);

        let weighted_scale = match self.cost_modes[patch_row] {
            CostMode::Unweighted => None,
            CostMode::Weighted => {
                let patch_sum =
                    self.patch_weight_sums[patch_row * self.level.patch_cols() + patch_col];
                Some(if patch_sum > 0.0 {
                    1.0 / patch_sum
                } else {
                    0.0
                })
            }
        };

        let mut statistics = Statistics::default();
        for row in 0..PATCH_SIZE {
            for col in 0..PATCH_SIZE {
                let source_at = (source_row + row) * cols + source_col + col;
                if self.source_slot_mask[source_at] == 0 {
                    continue;
                }
                if self.target_slot_mask[sampling.target_index(row, col)] == 0 {
                    continue;
                }

                let target = sampling.sample_for_descent(&self.target, row, col);
                let difference = target - f32::from(self.source[source_at]);
                let residual = if let Some(scale) = weighted_scale {
                    weighted_residual(difference, self.raw_weight[source_at], scale)
                } else {
                    difference
                };
                statistics.accumulate_descent(
                    residual,
                    self.gradient_col[source_at],
                    self.gradient_row[source_at],
                );
            }
        }
        statistics
    }

    fn prepared_source_terms(&self, patch_row: usize, patch_col: usize) -> PreparedSourceTerms {
        let cols = self.level.cols();
        let source_row = patch_row * PATCH_STRIDE;
        let source_col = patch_col * PATCH_STRIDE;
        let mut terms = PreparedSourceTerms::default();
        for row in 0..PATCH_SIZE {
            for col in 0..PATCH_SIZE {
                let at = (source_row + row) * cols + source_col + col;
                let gradient_col = self.gradient_col[at];
                let gradient_row = self.gradient_row[at];
                terms.gradient_col_sum += gradient_col;
                terms.gradient_row_sum += gradient_row;
                terms.h_col_col += gradient_col * gradient_col;
                terms.h_col_row += gradient_col * gradient_row;
                terms.h_row_row += gradient_row * gradient_row;
            }
        }
        terms
    }
}

/// A fixed-grid input or prepared plane had the wrong number of values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapeError {
    level: Level,
    part: &'static str,
    expected: usize,
    actual: usize,
}

impl fmt::Display for ShapeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "ONE X2 PIS {} has {} {} values, expected {}",
            self.level, self.actual, self.part, self.expected,
        )
    }
}

impl Error for ShapeError {}

/// A prepared PIS input violated its shape, finite, nonnegative, or source-mask
/// contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputError {
    Shape(ShapeError),
    NonFinitePreparedValue {
        level: Level,
        part: &'static str,
        index: usize,
    },
    NegativeRawWeight {
        level: Level,
        index: usize,
    },
    NonZeroBelowPhysicalMaskA {
        level: Level,
        part: &'static str,
        index: usize,
    },
}

impl From<ShapeError> for InputError {
    fn from(error: ShapeError) -> Self {
        Self::Shape(error)
    }
}

impl fmt::Display for InputError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shape(error) => error.fmt(out),
            Self::NonFinitePreparedValue { level, part, index } => write!(
                out,
                "ONE X2 PIS {level} {part} at pixel {index} is not finite",
            ),
            Self::NegativeRawWeight { level, index } => write!(
                out,
                "ONE X2 PIS {level} raw SSD weight at pixel {index} is negative",
            ),
            Self::NonZeroBelowPhysicalMaskA { level, part, index } => write!(
                out,
                "ONE X2 PIS {level} {part} at pixel {index} is nonzero beneath physical mask A",
            ),
        }
    }
}

impl Error for InputError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Shape(error) => Some(error),
            Self::NonFinitePreparedValue { .. }
            | Self::NegativeRawWeight { .. }
            | Self::NonZeroBelowPhysicalMaskA { .. } => None,
        }
    }
}

/// One candidate's mean-normalized residual and mask support.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Score {
    value: f32,
    survivors: usize,
}

impl Score {
    pub const fn value(self) -> f32 {
        self.value
    }

    pub const fn survivors(self) -> usize {
        self.survivors
    }

    pub fn is_sentinel(self) -> bool {
        self.value == SENTINEL_SCORE
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Statistics {
    sum: f32,
    sum_sq: f32,
    rhs_col: f32,
    rhs_row: f32,
    survivors: usize,
}

fn weighted_residual(difference: f32, raw_weight: f32, reciprocal_sum: f32) -> f32 {
    difference * (raw_weight * reciprocal_sum)
}

/// The selected candidate helpers' four persistent SIMD lanes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CandidateReduction {
    sums: [f32; 4],
    square_sums: [f32; 4],
    survivors: [u32; 4],
}

impl CandidateReduction {
    fn accumulate_row(&mut self, residuals: [f32; PATCH_SIZE], survives: [bool; PATCH_SIZE]) {
        for lane in 0..4 {
            let low = residuals[lane];
            let high = residuals[lane + 4];
            self.sums[lane] += low + high;
            self.square_sums[lane] += low * low + high * high;
            self.survivors[lane] += u32::from(survives[lane]) + u32::from(survives[lane + 4]);
        }
    }

    fn score(self) -> Score {
        let survivors = self.survivors.into_iter().sum::<u32>() as usize;
        let value = if survivors >= MIN_SCORE_SURVIVORS {
            let sum = (self.sums[0] + self.sums[1]) + (self.sums[2] + self.sums[3]);
            let sum_sq = (self.square_sums[0] + self.square_sums[1])
                + (self.square_sums[2] + self.square_sums[3]);
            let count = survivors as f32;
            sum_sq - (sum * sum) / count
        } else {
            SENTINEL_SCORE
        };
        Score { value, survivors }
    }
}

/// Reconstruct Studio's cached 8 by 8 weight denominator table.
///
/// Static arm64 proves that the structure-tensor producer does not resummate
/// each patch. It forms the first horizontal window with scalar additions,
/// advances it with `previous + (entering - leaving)`, then applies the same
/// initial-window and sliding recurrence vertically. Only stride-selected
/// origins are retained. No live cached denominator bits were captured, so
/// these values are an instruction reconstruction rather than an authenticated
/// native payload.
fn rolling_patch_weight_sums(level: Level, raw_weight: &[f32]) -> Vec<f32> {
    let rows = level.rows();
    let cols = level.cols();
    let patch_rows = level.patch_rows();
    let patch_cols = level.patch_cols();
    let mut horizontal = vec![0.0f32; rows * patch_cols];

    for row in 0..rows {
        let row_start = row * cols;
        let mut sum = 0.0f32;
        for col in 0..PATCH_SIZE {
            sum += raw_weight[row_start + col];
        }
        horizontal[row * patch_cols] = sum;

        for source_col in 1..=(cols - PATCH_SIZE) {
            let entering = raw_weight[row_start + source_col + PATCH_SIZE - 1];
            let leaving = raw_weight[row_start + source_col - 1];
            let delta = entering - leaving;
            sum += delta;
            if source_col % PATCH_STRIDE == 0 {
                horizontal[row * patch_cols + source_col / PATCH_STRIDE] = sum;
            }
        }
    }

    let mut patches = vec![0.0f32; level.patches()];
    for patch_col in 0..patch_cols {
        let mut sum = 0.0f32;
        for row in 0..PATCH_SIZE {
            sum += horizontal[row * patch_cols + patch_col];
        }
        patches[patch_col] = sum;

        for source_row in 1..=(rows - PATCH_SIZE) {
            let entering = horizontal[(source_row + PATCH_SIZE - 1) * patch_cols + patch_col];
            let leaving = horizontal[(source_row - 1) * patch_cols + patch_col];
            let delta = entering - leaving;
            sum += delta;
            if source_row % PATCH_STRIDE == 0 {
                patches[(source_row / PATCH_STRIDE) * patch_cols + patch_col] = sum;
            }
        }
    }
    debug_assert_eq!(patches.len(), patch_rows * patch_cols);
    patches
}

impl Statistics {
    fn accumulate_descent(&mut self, residual: f32, gradient_col: f32, gradient_row: f32) {
        self.rhs_col = residual.mul_add(gradient_col, self.rhs_col);
        self.sum += residual;
        self.rhs_row = residual.mul_add(gradient_row, self.rhs_row);
        self.sum_sq = residual.mul_add(residual, self.sum_sq);
        self.survivors += 1;
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PreparedSourceTerms {
    gradient_col_sum: f32,
    gradient_row_sum: f32,
    h_col_col: f32,
    h_col_row: f32,
    h_row_row: f32,
}

impl PreparedSourceTerms {
    /// Form the inverse from the raw prepared-source Hessian before any
    /// dual-mask residual or mean-correction work. Native replaces a small
    /// determinant with positive 0.001 rather than preserving its sign.
    fn inverted(self) -> PreparedSourceModel {
        // Native rounds the negated cross-square first (`fnmul`), then fuses
        // Hxx * Hyy plus that value (`fmadd`). Keeping those as two explicit
        // operations reproduces its product-difference contraction boundary.
        let negative_cross_square = -(self.h_col_row * self.h_col_row);
        let mut determinant = self
            .h_col_col
            .mul_add(self.h_row_row, negative_cross_square);
        if determinant.abs() < 0.001 {
            determinant = 0.001;
        }
        PreparedSourceModel {
            gradient_col_sum: self.gradient_col_sum,
            gradient_row_sum: self.gradient_row_sum,
            inverse_col_col: self.h_row_row / determinant,
            inverse_col_row: -self.h_col_row / determinant,
            inverse_row_row: self.h_col_col / determinant,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PreparedSourceModel {
    gradient_col_sum: f32,
    gradient_row_sum: f32,
    inverse_col_col: f32,
    inverse_col_row: f32,
    inverse_row_row: f32,
}

impl PreparedSourceModel {
    fn delta(self, rhs_col: f32, rhs_row: f32) -> Flow {
        // Each native chain rounds its first product (`fmul`) and fuses the
        // other product with that addend (`fmadd`).
        let cross_row = self.inverse_col_row * rhs_row;
        let delta_col = self.inverse_col_col.mul_add(rhs_col, cross_row);
        let row_row = self.inverse_row_row * rhs_row;
        let delta_row = self.inverse_col_row.mul_add(rhs_col, row_row);
        Flow {
            dcol: delta_col,
            drow: delta_row,
        }
    }
}

/// Initial per-patch flows, before the first spatial pass.
///
/// Public construction is restricted to the coarsest selected level. A
/// level-one seed can be produced only by the post-update L2 adapter inside
/// the ONE X2 module family.
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::pis::{AtoB, Flow, InitialGrid, Level};
///
/// let flows = vec![Flow::ZERO; Level::One.patches()];
/// let _ = InitialGrid::<AtoB>::from_l1_row_major(flows);
/// ```
///
/// Stage-derived seeds are linear tokens and cannot be duplicated:
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::pis::{AtoB, InitialGrid};
///
/// fn duplicate(seed: InitialGrid<AtoB>) {
///     let _copy = seed.clone();
/// }
/// ```
#[derive(Debug, PartialEq)]
pub struct InitialGrid<D: PisDirection> {
    level: Level,
    flows: Box<[Flow]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> InitialGrid<D> {
    /// Build native's zero initial grid for the coarsest selected solve.
    pub fn coarse_zeros() -> Self {
        Self::zeros_at_level(Level::Two)
    }

    /// Admit an explicit row-major initial grid for the coarsest selected
    /// solve.
    pub fn coarse_from_row_major(flows: Vec<Flow>) -> Result<Self, ShapeError> {
        Self::from_row_major_at_level(Level::Two, flows)
    }

    pub(super) fn from_l1_row_major(flows: Vec<Flow>) -> Result<Self, ShapeError> {
        Self::from_row_major_at_level(Level::One, flows)
    }

    fn zeros_at_level(level: Level) -> Self {
        Self {
            level,
            flows: vec![Flow::ZERO; level.patches()].into_boxed_slice(),
            direction: PhantomData,
        }
    }

    fn from_row_major_at_level(level: Level, flows: Vec<Flow>) -> Result<Self, ShapeError> {
        checked_patch_flows(level, "initial patch", flows).map(|flows| Self {
            level,
            flows,
            direction: PhantomData,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_test_row_major(level: Level, flows: Vec<Flow>) -> Result<Self, ShapeError> {
        Self::from_row_major_at_level(level, flows)
    }

    pub const fn level(&self) -> Level {
        self.level
    }

    pub fn flows(&self) -> &[Flow] {
        &self.flows
    }

    pub const fn direction(&self) -> Direction {
        D::DIRECTION
    }
}

/// Optional retained/coarser hint candidates, one per sparse patch.
///
/// This is intentionally not interchangeable with [`InitialGrid`]. Native
/// evaluates the current candidate first and this hint second; equal scores
/// therefore keep current.
#[derive(Clone, Debug, PartialEq)]
pub struct HintGrid<D: PisDirection> {
    level: Level,
    flows: Box<[Flow]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> HintGrid<D> {
    pub fn from_row_major(level: Level, flows: Vec<Flow>) -> Result<Self, ShapeError> {
        checked_patch_flows(level, "hint patch", flows).map(|flows| Self {
            level,
            flows,
            direction: PhantomData,
        })
    }

    pub const fn level(&self) -> Level {
        self.level
    }

    pub fn flows(&self) -> &[Flow] {
        &self.flows
    }

    pub const fn direction(&self) -> Direction {
        D::DIRECTION
    }
}

/// The sparse patch grid after forward and backward passes.
#[derive(Debug, PartialEq)]
pub struct PatchGrid<D: PisDirection> {
    level: Level,
    patches: Box<[Patch]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> PatchGrid<D> {
    pub const fn level(&self) -> Level {
        self.level
    }

    pub fn patches(&self) -> &[Patch] {
        &self.patches
    }

    pub fn patch(&self, row: usize, col: usize) -> Patch {
        assert!(row < self.level.patch_rows());
        assert!(col < self.level.patch_cols());
        self.patches[row * self.level.patch_cols() + col]
    }

    pub const fn direction(&self) -> Direction {
        D::DIRECTION
    }

    /// Consume the sparse oracle result into row-major component planes.
    ///
    /// The returned level lets the finest-only temporal stage reject level two
    /// before it mutates history. Direction remains in this method's input
    /// type, so callers cannot exchange the two native solver outputs first.
    pub(super) fn into_row_major_components(self) -> (Level, Vec<f32>, Vec<f32>) {
        let mut dcol = Vec::with_capacity(self.patches.len());
        let mut drow = Vec::with_capacity(self.patches.len());
        for patch in self.patches {
            dcol.push(patch.flow.dcol);
            drow.push(patch.flow.drow);
        }
        (self.level, dcol, drow)
    }

    /// Build a solver-output token without running PIS for downstream unit
    /// tests. Production obtains this type only through [`solve`].
    #[cfg(test)]
    pub(crate) fn from_row_major_components(
        level: Level,
        dcol: Vec<f32>,
        drow: Vec<f32>,
    ) -> Result<Self, ShapeError> {
        let expected = level.patches();
        for (part, actual) in [("dcol patch", dcol.len()), ("drow patch", drow.len())] {
            if actual != expected {
                return Err(ShapeError {
                    level,
                    part,
                    expected,
                    actual,
                });
            }
        }
        let patches = dcol
            .into_iter()
            .zip(drow)
            .map(|(dcol, drow)| Patch::seeded(Flow { dcol, drow }))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self {
            level,
            patches,
            direction: PhantomData,
        })
    }
}

/// A typed seed or hint grid belonged to another selected pyramid level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelError {
    part: &'static str,
    expected: Level,
    actual: Level,
}

impl fmt::Display for LevelError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "ONE X2 PIS {} is {}, expected {}",
            self.part, self.actual, self.expected,
        )
    }
}

impl Error for LevelError {}

/// Run the selected patch inverse search for one direction and level.
///
/// This is a correctness oracle, not the playback implementation. It performs
/// both native in-place passes with a six-descent budget per pass. Weighted
/// candidates reproduce the selected ARMv8.2 SIMD reduction; the remaining
/// routes keep their recovered scalar operation order explicit. This does not
/// claim whole-solver NEON bit identity.
///
/// Seeds, hints, and results cannot cross native directions:
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::pis::{AtoB, BtoA, HintGrid, InitialGrid, Input, solve};
///
/// fn cannot_mix(
///     input: &Input<AtoB>,
///     initial: InitialGrid<BtoA>,
///     hint: &HintGrid<BtoA>,
/// ) {
///     let _ = solve(input, initial, Some(hint));
/// }
/// ```
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::pis::{AtoB, BtoA, PatchGrid};
///
/// fn cannot_relabel(reverse: PatchGrid<BtoA>) -> PatchGrid<AtoB> {
///     reverse
/// }
/// ```
///
/// An initial seed is consumed by its one solve:
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::pis::{AtoB, InitialGrid, Input, solve};
///
/// fn cannot_reuse(input: &Input<AtoB>, initial: InitialGrid<AtoB>) {
///     let _first = solve(input, initial, None);
///     let _second = solve(input, initial, None);
/// }
/// ```
pub fn solve<D: PisDirection>(
    input: &Input<D>,
    initial: InitialGrid<D>,
    hint: Option<&HintGrid<D>>,
) -> Result<PatchGrid<D>, LevelError> {
    solve_with_descent_admission(input, initial, hint, DescentAdmission::EveryPatch)
}

/// Studio's outer admission decision for the Gauss-Newton descent body.
///
/// The selected `FDS+0x9c == 0` route seeds this from
/// `FDS+0xc8 % FDS+0xa8 == 0`. A nonempty `FDS+0xb0` table can override that
/// decision for individual spatial regions. The selected ONE X2 capture has
/// an authenticated empty table, so its decision is uniform for one solve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DescentAdmission {
    EveryPatch,
    NoPatches,
}

impl DescentAdmission {
    /// Resolve the selected plain-cadence route when `FDS+0xb0` is empty.
    pub(super) fn from_empty_override_cadence(calc_count: i32, cadence: i32) -> Self {
        assert_ne!(cadence, 0, "ONE X2 PIS cadence must not be zero");
        if calc_count % cadence == 0 {
            Self::EveryPatch
        } else {
            Self::NoPatches
        }
    }

    const fn admits(self) -> bool {
        matches!(self, Self::EveryPatch)
    }
}

pub(super) fn solve_with_descent_admission<D: PisDirection>(
    input: &Input<D>,
    initial: InitialGrid<D>,
    hint: Option<&HintGrid<D>>,
    admission: DescentAdmission,
) -> Result<PatchGrid<D>, LevelError> {
    solve_with_descents(input, initial, hint, DESCENTS_PER_PASS, admission)
}

#[cfg(test)]
pub(crate) fn solve_with_test_descents<D: PisDirection>(
    input: &Input<D>,
    initial: InitialGrid<D>,
    hint: Option<&HintGrid<D>>,
    descents_per_pass: usize,
) -> Result<PatchGrid<D>, LevelError> {
    solve_with_descents(
        input,
        initial,
        hint,
        descents_per_pass,
        DescentAdmission::EveryPatch,
    )
}

/// Exact f32 boundaries of one pass-zero descent for a selected patch.
///
/// This exists only for authenticated solver-oracle tests. The original
/// consumer records only the first descent; a bounded trajectory diagnostic
/// can retain each ordinary kernel call. Values are stored as bits so a native
/// debugger capture can be compared without decimal formatting or parsing
/// changing the evidence.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FirstDescentTrace {
    pub patch_row: usize,
    pub patch_col: usize,
    pub seed: [u32; 2],
    pub hessian: [u32; 3],
    pub inverse: [u32; 3],
    pub gradient_sums: [u32; 2],
    pub raw_sum: u32,
    pub raw_sum_sq: u32,
    pub raw_rhs: [u32; 2],
    pub mean_correction: [u32; 2],
    pub corrected_rhs: [u32; 2],
    pub corrected_residual: u32,
    pub delta: [u32; 2],
    pub updated_flow: [u32; 2],
    pub survivors: usize,
}

/// One dual-mask survivor in the exact row-major descent accumulation order.
///
/// This is test-only evidence plumbing. It lets an authenticated native tap
/// receipt identify the first differing accumulation without changing the
/// solver or rounding decimal diagnostics back into floats.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DescentSurvivorTrace {
    pub ordinal: usize,
    pub patch_row: usize,
    pub patch_col: usize,
    pub raw_weight: u32,
    pub reciprocal_weight_sum: u32,
    pub difference: u32,
    pub normalized_residual: u32,
    pub gradient_col: u32,
    pub gradient_row: u32,
    pub pre: [u32; 4],
    pub post: [u32; 4],
}

/// The complete active sparse grid immediately after one pass-zero current
/// score returns and before any competing candidate is evaluated.
///
/// The component bits are separate row-major planes so a debugger capture of
/// native's U and V backing Mats can be compared without changing either
/// payload's ordering. The selected row-54 oracle asserts the level-two 88 by
/// 3 shape rather than relying on this generic trace helper to imply it.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PassZeroPostCurrentScoreGridTrace {
    pub level: Level,
    pub patch_rows: usize,
    pub patch_cols: usize,
    pub patch_row: usize,
    pub patch_col: usize,
    pub current_score: u32,
    pub current_survivors: usize,
    pub dcol_bits: Box<[u32]>,
    pub drow_bits: Box<[u32]>,
}

#[cfg(test)]
type TestPatchTraces<D> = (
    PatchGrid<D>,
    Box<[FirstDescentTrace]>,
    PassZeroPostCurrentScoreGridTrace,
);

/// Evaluate the immediate descent at one exact seed without running candidate
/// selection or the in-place two-pass schedule.
///
/// This is test-only so authenticated native captures can distinguish a seed
/// difference from descent arithmetic on otherwise identical inputs.
#[cfg(test)]
pub(crate) fn test_first_descent_at<D: PisDirection>(
    input: &Input<D>,
    patch_row: usize,
    patch_col: usize,
    seed: Flow,
) -> FirstDescentTrace {
    assert!(patch_row < input.level.patch_rows());
    assert!(patch_col < input.level.patch_cols());
    let patch = patch_row * input.level.patch_cols() + patch_col;
    let mut kernel = ImageKernel::new_traced(input, patch);
    kernel
        .descent(0, patch, seed)
        .expect("selected patch did not produce a first descent");
    kernel
        .pass_zero_traces
        .first()
        .copied()
        .expect("selected patch did not record its first descent")
}

/// Trace every admitted tap for one exact descent seed.
///
/// The loop intentionally calls the same sampling, weighting, and
/// [`Statistics::accumulate`] operations as the selected descent route. The
/// final assertion binds the diagnostic replay back to the ordinary helper so
/// this cannot silently become an independent arithmetic model.
#[cfg(test)]
pub(crate) fn test_descent_survivors_at<D: PisDirection>(
    input: &Input<D>,
    patch_row: usize,
    patch_col: usize,
    seed: Flow,
) -> Vec<DescentSurvivorTrace> {
    assert!(patch_row < input.level.patch_rows());
    assert!(patch_col < input.level.patch_cols());
    let rows = input.level.rows();
    let cols = input.level.cols();
    let source_row = patch_row * PATCH_STRIDE;
    let source_col = patch_col * PATCH_STRIDE;
    let origin_row = padded_origin(source_row, seed.drow, rows);
    let origin_col = padded_origin(source_col, seed.dcol, cols);
    let sampling = PatchSamplingPlan::new(origin_row, origin_col, rows, cols);
    assert_eq!(input.cost_modes[patch_row], CostMode::Weighted);
    let patch_sum = input.patch_weight_sums[patch_row * input.level.patch_cols() + patch_col];
    let reciprocal = if patch_sum > 0.0 {
        1.0 / patch_sum
    } else {
        0.0
    };
    let mut statistics = Statistics::default();
    let mut trace = Vec::new();

    for row in 0..PATCH_SIZE {
        for col in 0..PATCH_SIZE {
            let source_at = (source_row + row) * cols + source_col + col;
            if input.source_slot_mask[source_at] == 0 {
                continue;
            }
            if input.target_slot_mask[sampling.target_index(row, col)] == 0 {
                continue;
            }

            let target = sampling.sample_for_descent(&input.target, row, col);
            let difference = target - f32::from(input.source[source_at]);
            let residual = weighted_residual(difference, input.raw_weight[source_at], reciprocal);
            let gradient_col = input.gradient_col[source_at];
            let gradient_row = input.gradient_row[source_at];
            let pre = statistics;
            statistics.accumulate_descent(residual, gradient_col, gradient_row);
            trace.push(DescentSurvivorTrace {
                ordinal: trace.len() + 1,
                patch_row: row,
                patch_col: col,
                raw_weight: input.raw_weight[source_at].to_bits(),
                reciprocal_weight_sum: reciprocal.to_bits(),
                difference: difference.to_bits(),
                normalized_residual: residual.to_bits(),
                gradient_col: gradient_col.to_bits(),
                gradient_row: gradient_row.to_bits(),
                pre: [
                    pre.sum.to_bits(),
                    pre.sum_sq.to_bits(),
                    pre.rhs_col.to_bits(),
                    pre.rhs_row.to_bits(),
                ],
                post: [
                    statistics.sum.to_bits(),
                    statistics.sum_sq.to_bits(),
                    statistics.rhs_col.to_bits(),
                    statistics.rhs_row.to_bits(),
                ],
            });
        }
    }

    let ordinary = input.descent_statistics(patch_row, patch_col, seed);
    assert_eq!(statistics.sum.to_bits(), ordinary.sum.to_bits());
    assert_eq!(statistics.sum_sq.to_bits(), ordinary.sum_sq.to_bits());
    assert_eq!(statistics.rhs_col.to_bits(), ordinary.rhs_col.to_bits());
    assert_eq!(statistics.rhs_row.to_bits(), ordinary.rhs_row.to_bits());
    assert_eq!(statistics.survivors, ordinary.survivors);
    trace
}

/// Exact translated origin and four-corner weights used by descent's first
/// target sample at one supplied seed.
#[cfg(test)]
pub(crate) fn test_descent_sample_boundary_at<D: PisDirection>(
    input: &Input<D>,
    patch_row: usize,
    patch_col: usize,
    seed: Flow,
) -> ([u32; 2], [u32; 4]) {
    assert!(patch_row < input.level.patch_rows());
    assert!(patch_col < input.level.patch_cols());
    const TARGET_PADDING: f32 = (2 * PATCH_SIZE) as f32;
    let row = padded_origin(patch_row * PATCH_STRIDE, seed.drow, input.level.rows());
    let col = padded_origin(patch_col * PATCH_STRIDE, seed.dcol, input.level.cols());
    let sampling = PatchSamplingPlan::new(row, col, input.level.rows(), input.level.cols());
    (
        [
            (row + TARGET_PADDING).to_bits(),
            (col + TARGET_PADDING).to_bits(),
        ],
        sampling.coefficients().map(f32::to_bits),
    )
}

/// Run the ordinary solver and return the immediate first pass-zero descent
/// at one patch, rather than the two-pass output produced by a one-descent
/// budget.
#[cfg(test)]
pub(crate) fn solve_with_test_first_descent<D: PisDirection>(
    input: &Input<D>,
    initial: InitialGrid<D>,
    hint: Option<&HintGrid<D>>,
    descents_per_pass: usize,
    patch_row: usize,
    patch_col: usize,
) -> Result<(PatchGrid<D>, FirstDescentTrace), LevelError> {
    let (grid, descents, _) = solve_with_test_traces(
        input,
        initial,
        hint,
        descents_per_pass,
        patch_row,
        patch_col,
    )?;
    Ok((grid, descents[0]))
}

/// Run the ordinary solver and return every pass-zero descent at one patch.
///
/// This is test-only evidence plumbing. The traces come from the same kernel
/// calls that mutate the ordinary solve, rather than from a separate replay.
#[cfg(test)]
pub(crate) fn solve_with_test_descent_sequence<D: PisDirection>(
    input: &Input<D>,
    initial: InitialGrid<D>,
    hint: Option<&HintGrid<D>>,
    descents_per_pass: usize,
    patch_row: usize,
    patch_col: usize,
) -> Result<(PatchGrid<D>, Box<[FirstDescentTrace]>), LevelError> {
    let (grid, descents, _) = solve_with_test_traces(
        input,
        initial,
        hint,
        descents_per_pass,
        patch_row,
        patch_col,
    )?;
    Ok((grid, descents))
}

/// Run the ordinary solver and snapshot both test boundaries for one patch:
/// the complete active grid after pass zero's current score and the immediate
/// first descent after candidate selection.
#[cfg(test)]
pub(crate) fn solve_with_test_patch_boundary<D: PisDirection>(
    input: &Input<D>,
    initial: InitialGrid<D>,
    hint: Option<&HintGrid<D>>,
    descents_per_pass: usize,
    patch_row: usize,
    patch_col: usize,
) -> Result<
    (
        PatchGrid<D>,
        FirstDescentTrace,
        PassZeroPostCurrentScoreGridTrace,
    ),
    LevelError,
> {
    let (grid, descents, grid_trace) = solve_with_test_traces(
        input,
        initial,
        hint,
        descents_per_pass,
        patch_row,
        patch_col,
    )?;
    Ok((grid, descents[0], grid_trace))
}

#[cfg(test)]
fn solve_with_test_traces<D: PisDirection>(
    input: &Input<D>,
    initial: InitialGrid<D>,
    hint: Option<&HintGrid<D>>,
    descents_per_pass: usize,
    patch_row: usize,
    patch_col: usize,
) -> Result<TestPatchTraces<D>, LevelError> {
    assert!(
        descents_per_pass > 0,
        "a first descent requires a nonzero budget"
    );
    assert!(patch_row < input.level.patch_rows());
    assert!(patch_col < input.level.patch_cols());
    if initial.level != input.level {
        return Err(LevelError {
            part: "initial grid",
            expected: input.level,
            actual: initial.level,
        });
    }
    if let Some(hint) = hint
        && hint.level != input.level
    {
        return Err(LevelError {
            part: "hint grid",
            expected: input.level,
            actual: hint.level,
        });
    }

    let trace_patch = patch_row * input.level.patch_cols() + patch_col;
    let mut kernel = ImageKernel::new_traced(input, trace_patch);
    let patches = schedule(
        input.level.patch_rows(),
        input.level.patch_cols(),
        &initial.flows,
        hint.map(|hint| hint.flows.as_ref()),
        input.disparity,
        descents_per_pass,
        &mut kernel,
    );
    let traces = kernel.pass_zero_traces.into_boxed_slice();
    assert!(
        !traces.is_empty(),
        "selected patch did not execute a pass-zero descent"
    );
    let grid_trace = kernel
        .pass_zero_post_current_score_grid_trace
        .expect("selected patch did not record its pass-zero current-score grid");
    Ok((
        PatchGrid {
            level: input.level,
            patches: patches.into_boxed_slice(),
            direction: PhantomData,
        },
        traces,
        grid_trace,
    ))
}

fn solve_with_descents<D: PisDirection>(
    input: &Input<D>,
    initial: InitialGrid<D>,
    hint: Option<&HintGrid<D>>,
    descents_per_pass: usize,
    descent_admission: DescentAdmission,
) -> Result<PatchGrid<D>, LevelError> {
    assert!(
        u8::try_from(descents_per_pass).is_ok(),
        "ONE X2 PIS descent report cannot represent this budget"
    );
    if initial.level != input.level {
        return Err(LevelError {
            part: "initial grid",
            expected: input.level,
            actual: initial.level,
        });
    }
    if let Some(hint) = hint
        && hint.level != input.level
    {
        return Err(LevelError {
            part: "hint grid",
            expected: input.level,
            actual: hint.level,
        });
    }

    let mut kernel = ImageKernel::with_descent_admission(input, descent_admission);
    let disparity = input.disparity;
    let patches = schedule(
        input.level.patch_rows(),
        input.level.patch_cols(),
        &initial.flows,
        hint.map(|hint| hint.flows.as_ref()),
        disparity,
        descents_per_pass,
        &mut kernel,
    );
    Ok(PatchGrid {
        level: input.level,
        patches: patches.into_boxed_slice(),
        direction: PhantomData,
    })
}

fn checked_patch_flows(
    level: Level,
    part: &'static str,
    flows: Vec<Flow>,
) -> Result<Box<[Flow]>, ShapeError> {
    let expected = level.patches();
    if flows.len() != expected {
        return Err(ShapeError {
            level,
            part,
            expected,
            actual: flows.len(),
        });
    }
    Ok(flows.into_boxed_slice())
}

/// Candidate order within each patch visit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Candidate {
    Current,
    Hint,
    Horizontal,
    Vertical,
}

impl Candidate {
    const ORDER: [Self; 4] = [Self::Current, Self::Hint, Self::Horizontal, Self::Vertical];

    const fn index(self) -> usize {
        match self {
            Self::Current => 0,
            Self::Hint => 1,
            Self::Horizontal => 2,
            Self::Vertical => 3,
        }
    }
}

/// One candidate as observed at a particular pass visit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CandidateEvaluation {
    flow: Flow,
    score: Score,
}

impl CandidateEvaluation {
    pub const fn flow(self) -> Flow {
        self.flow
    }

    pub const fn score(self) -> Score {
        self.score
    }
}

/// Trace of one native directional pass through one patch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassReport {
    winner: Candidate,
    candidates: [Option<CandidateEvaluation>; 4],
    descent_admitted: bool,
    descent_iterations: u8,
    stopped_on_no_improvement: bool,
    guarded: bool,
    stored_flow: Flow,
}

impl PassReport {
    pub const fn winner(self) -> Candidate {
        self.winner
    }

    /// Candidate slots in current, hint, horizontal, vertical order.
    pub const fn candidates(self) -> [Option<CandidateEvaluation>; 4] {
        self.candidates
    }

    pub const fn descent_admitted(self) -> bool {
        self.descent_admitted
    }

    pub const fn descent_iterations(self) -> u8 {
        self.descent_iterations
    }

    pub const fn stopped_on_no_improvement(self) -> bool {
        self.stopped_on_no_improvement
    }

    pub const fn guarded(self) -> bool {
        self.guarded
    }

    /// Flow present in the sparse cell at this pass's terminal join.
    pub const fn stored_flow(self) -> Flow {
        self.stored_flow
    }
}

impl Default for PassReport {
    fn default() -> Self {
        Self {
            winner: Candidate::Current,
            candidates: [None; 4],
            descent_admitted: false,
            descent_iterations: 0,
            stopped_on_no_improvement: false,
            guarded: false,
            stored_flow: Flow::ZERO,
        }
    }
}

/// One sparse patch after both in-place passes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Patch {
    flow: Flow,
    residual: f32,
    passes: [PassReport; 2],
}

impl Patch {
    fn seeded(flow: Flow) -> Self {
        Self {
            flow,
            residual: SENTINEL_SCORE,
            passes: [PassReport::default(); 2],
        }
    }

    pub const fn flow(self) -> Flow {
        self.flow
    }

    pub const fn residual(self) -> f32 {
        self.residual
    }

    pub const fn passes(self) -> [PassReport; 2] {
        self.passes
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DescentStep {
    delta: Flow,
    residual: f32,
    survivors: usize,
}

trait Kernel {
    fn score(&mut self, pass: usize, patch: usize, flow: Flow) -> Score;

    fn descent(&mut self, pass: usize, patch: usize, flow: Flow) -> Option<DescentStep>;

    fn admits_descent(&self, _pass: usize, _patch: usize) -> bool {
        true
    }

    #[cfg(test)]
    fn record_post_current_score_grid(
        &mut self,
        _pass: usize,
        _patch: usize,
        _score: Score,
        _patches: &[Patch],
    ) {
    }
}

struct ImageKernel<'a, D: PisDirection> {
    input: &'a Input<D>,
    prepared_source_models: Vec<PreparedSourceModel>,
    descent_admission: DescentAdmission,
    #[cfg(test)]
    trace_patch: Option<usize>,
    #[cfg(test)]
    pass_zero_traces: Vec<FirstDescentTrace>,
    #[cfg(test)]
    pass_zero_post_current_score_grid_trace: Option<PassZeroPostCurrentScoreGridTrace>,
}

impl<'a, D: PisDirection> ImageKernel<'a, D> {
    #[cfg(test)]
    fn new(input: &'a Input<D>) -> Self {
        Self::with_descent_admission(input, DescentAdmission::EveryPatch)
    }

    fn with_descent_admission(input: &'a Input<D>, descent_admission: DescentAdmission) -> Self {
        let prepared_source_models = (0..input.level.patches())
            .map(|patch| {
                let row = patch / input.level.patch_cols();
                let col = patch % input.level.patch_cols();
                input.prepared_source_terms(row, col).inverted()
            })
            .collect();
        Self {
            input,
            prepared_source_models,
            descent_admission,
            #[cfg(test)]
            trace_patch: None,
            #[cfg(test)]
            pass_zero_traces: Vec::new(),
            #[cfg(test)]
            pass_zero_post_current_score_grid_trace: None,
        }
    }

    #[cfg(test)]
    fn new_traced(input: &'a Input<D>, trace_patch: usize) -> Self {
        let mut kernel = Self::new(input);
        kernel.trace_patch = Some(trace_patch);
        kernel
    }

    fn coordinate(&self, patch: usize) -> (usize, usize) {
        (
            patch / self.input.level.patch_cols(),
            patch % self.input.level.patch_cols(),
        )
    }
}

impl<D: PisDirection> Kernel for ImageKernel<'_, D> {
    fn score(&mut self, _pass: usize, patch: usize, flow: Flow) -> Score {
        let (row, col) = self.coordinate(patch);
        self.input.candidate_score(row, col, flow)
    }

    fn descent(&mut self, _pass: usize, patch: usize, flow: Flow) -> Option<DescentStep> {
        let (row, col) = self.coordinate(patch);
        let statistics = self.input.descent_statistics(row, col, flow);

        // Dynamic dual-mask survivors and SSD weights affect S/Q/Rx/Ry/N only.
        // The gradient sums and Hessian use the full prepared source patch,
        // whose source-slot rejects were already zeroed upstream; target-slot
        // rejects and weights never prune these precomputed terms.
        let source = self.prepared_source_models[patch];
        // Native tests `w9 < 1` at `0x2d40f4c..0x2d40f50` before the
        // survivor divides. Its zero path stores zero deltas and returns the
        // exact finite sentinel at `0x2d40f88..0x2d40f98`.
        if statistics.survivors == 0 {
            return Some(DescentStep {
                delta: Flow::ZERO,
                residual: SENTINEL_SCORE,
                survivors: 0,
            });
        }
        // Both selected masked helper routes preserve their dynamic dual-mask
        // survivor counter and divide all three mean corrections by it.
        let count = statistics.survivors as f32;
        let correction_col = statistics.sum * source.gradient_col_sum / count;
        let correction_row = statistics.sum * source.gradient_row_sum / count;
        let rhs_col = statistics.rhs_col - correction_col;
        let rhs_row = statistics.rhs_row - correction_row;

        let delta = source.delta(rhs_col, rhs_row);
        let residual = statistics.sum_sq - statistics.sum * statistics.sum / count;

        #[cfg(test)]
        if _pass == 0 && self.trace_patch == Some(patch) {
            let terms = self.input.prepared_source_terms(row, col);
            let updated = flow.stepped(delta);
            self.pass_zero_traces.push(FirstDescentTrace {
                patch_row: row,
                patch_col: col,
                seed: [flow.dcol.to_bits(), flow.drow.to_bits()],
                hessian: [
                    terms.h_col_col.to_bits(),
                    terms.h_col_row.to_bits(),
                    terms.h_row_row.to_bits(),
                ],
                inverse: [
                    source.inverse_col_col.to_bits(),
                    source.inverse_col_row.to_bits(),
                    source.inverse_row_row.to_bits(),
                ],
                gradient_sums: [
                    source.gradient_col_sum.to_bits(),
                    source.gradient_row_sum.to_bits(),
                ],
                raw_sum: statistics.sum.to_bits(),
                raw_sum_sq: statistics.sum_sq.to_bits(),
                raw_rhs: [statistics.rhs_col.to_bits(), statistics.rhs_row.to_bits()],
                mean_correction: [correction_col.to_bits(), correction_row.to_bits()],
                corrected_rhs: [rhs_col.to_bits(), rhs_row.to_bits()],
                corrected_residual: residual.to_bits(),
                delta: [delta.dcol.to_bits(), delta.drow.to_bits()],
                updated_flow: [updated.dcol.to_bits(), updated.drow.to_bits()],
                survivors: statistics.survivors,
            });
        }

        Some(DescentStep {
            delta,
            residual,
            survivors: statistics.survivors,
        })
    }

    fn admits_descent(&self, _pass: usize, _patch: usize) -> bool {
        self.descent_admission.admits()
    }

    #[cfg(test)]
    fn record_post_current_score_grid(
        &mut self,
        pass: usize,
        patch: usize,
        score: Score,
        patches: &[Patch],
    ) {
        if pass != 0
            || self.trace_patch != Some(patch)
            || self.pass_zero_post_current_score_grid_trace.is_some()
        {
            return;
        }
        let (patch_row, patch_col) = self.coordinate(patch);
        self.pass_zero_post_current_score_grid_trace = Some(PassZeroPostCurrentScoreGridTrace {
            level: self.input.level,
            patch_rows: self.input.level.patch_rows(),
            patch_cols: self.input.level.patch_cols(),
            patch_row,
            patch_col,
            current_score: score.value.to_bits(),
            current_survivors: score.survivors,
            dcol_bits: patches
                .iter()
                .map(|active| active.flow.dcol.to_bits())
                .collect(),
            drow_bits: patches
                .iter()
                .map(|active| active.flow.drow.to_bits())
                .collect(),
        });
    }
}

/// The exact in-place scheduling core, parameterized only so focused tests can
/// distinguish dependency order from image arithmetic.
fn schedule<K: Kernel>(
    patch_rows: usize,
    patch_cols: usize,
    initial: &[Flow],
    hint: Option<&[Flow]>,
    disparity: Option<DisparityInterval>,
    descents_per_pass: usize,
    kernel: &mut K,
) -> Vec<Patch> {
    let patch_count = patch_rows * patch_cols;
    assert_eq!(initial.len(), patch_count);
    if let Some(hint) = hint {
        assert_eq!(hint.len(), patch_count);
    }
    let mut patches: Vec<_> = initial.iter().copied().map(Patch::seeded).collect();

    for pass in 0..2 {
        for ordinal in 0..patch_count {
            let patch = if pass == 0 {
                ordinal
            } else {
                patch_count - 1 - ordinal
            };
            let row = patch / patch_cols;
            let col = patch % patch_cols;
            let horizontal = if pass == 0 {
                col.checked_sub(1).map(|_| patch - 1)
            } else {
                (col + 1 < patch_cols).then_some(patch + 1)
            };
            let vertical = if pass == 0 {
                row.checked_sub(1).map(|_| patch - patch_cols)
            } else {
                (row + 1 < patch_rows).then_some(patch + patch_cols)
            };
            let flows = [
                Some(patches[patch].flow),
                hint.map(|hint| hint[patch]),
                horizontal.map(|neighbour| patches[neighbour].flow),
                vertical.map(|neighbour| patches[neighbour].flow),
            ];

            let mut report = PassReport::default();
            let mut winner = Candidate::Current;
            let mut best_flow = flows[Candidate::Current.index()]
                .expect("current ONE X2 PIS candidate disappeared");
            let mut best_score = SENTINEL_SCORE;
            for candidate in Candidate::ORDER {
                let Some(flow) = flows[candidate.index()] else {
                    continue;
                };
                let score = kernel.score(pass, patch, flow);
                report.candidates[candidate.index()] = Some(CandidateEvaluation { flow, score });
                #[cfg(test)]
                if candidate == Candidate::Current {
                    // `score` receives the current flow by value and has no
                    // access to `patches`, so this post-helper snapshot is
                    // byte-equivalent to the grid at helper entry. It is also
                    // before hint/horizontal/vertical candidate competition.
                    kernel.record_post_current_score_grid(pass, patch, score, &patches);
                }
                if candidate == Candidate::Current || score.value < best_score {
                    winner = candidate;
                    best_flow = flow;
                    best_score = score.value;
                }
            }
            report.winner = winner;

            let seed = best_flow;
            let mut current = seed;
            let mut previous = SENTINEL_SCORE;
            let mut residual = (best_score < SENTINEL_SCORE).then_some(best_score);
            report.descent_admitted = kernel.admits_descent(pass, patch);
            if report.descent_admitted {
                for _ in 0..descents_per_pass {
                    let Some(step) = kernel.descent(pass, patch, current) else {
                        break;
                    };
                    // Native writes the step before deciding that this residual
                    // failed to improve, including the step that ends the loop.
                    current = current.stepped(step.delta);
                    report.descent_iterations += 1;
                    if step.residual >= previous {
                        report.stopped_on_no_improvement = true;
                        break;
                    }
                    previous = step.residual;
                    if step.residual < SENTINEL_SCORE {
                        residual = Some(step.residual);
                    }
                }
            }

            // Native tests the descent result twice before accepting it, at
            // `0x2d3fa5c..0x2d3fb50`: first its Euclidean distance from the
            // selected propagation/hint seed, then the disparity interval.
            // Candidate selection has already stored that seed in the sparse
            // U/V slots, so either rejection retains it by skipping the only
            // post-descent stores.
            report.guarded = current.distance(seed) > PATCH_SIZE as f64
                || disparity.is_some_and(|interval| !interval.admits(current));
            report.stored_flow = if report.guarded { seed } else { current };
            patches[patch].flow = report.stored_flow;
            if let Some(residual) = residual {
                patches[patch].residual = residual;
            }
            patches[patch].passes[pass] = report;
        }
    }
    patches
}

fn padded_origin(source: usize, displacement: f32, extent: usize) -> f32 {
    // Native addresses a target plane with a 16-pixel replicated border. It
    // adds that integer border in f32 before clamping and extracting the
    // bilinear fraction, which can round low displacement bits away. Keep the
    // translated-domain rounding even though this readable oracle stores the
    // target without its physical border.
    const TARGET_PADDING: f32 = (2 * PATCH_SIZE) as f32;
    let translated = (source as f32 + displacement) + TARGET_PADDING;
    translated.clamp(
        TARGET_PADDING + 1.0 - PATCH_SIZE as f32,
        TARGET_PADDING + extent as f32 - 1.0,
    ) - TARGET_PADDING
}

fn clamp_index(index: isize, extent: usize) -> usize {
    index.clamp(0, extent as isize - 1) as usize
}

/// One target patch's translated base cell and bilinear coefficients.
///
/// Studio forms these values once before traversing the 8 by 8 patch. Its
/// replicated physical padding changes only which integer source cell each
/// corner reads; it does not erase the translated-domain fraction at an image
/// edge. The readable unpadded representation therefore clamps the four
/// integer neighbor indices independently while reusing this one coefficient
/// set for every tap.
#[derive(Clone, Debug)]
struct PatchSamplingPlan {
    row_indices: [usize; PATCH_SIZE + 1],
    col_indices: [usize; PATCH_SIZE + 1],
    cols: usize,
    coefficients: [f32; 4],
}

impl PatchSamplingPlan {
    fn new(origin_row: f32, origin_col: f32, rows: usize, cols: usize) -> Self {
        let floor_row = origin_row.floor();
        let floor_col = origin_col.floor();
        let row_fraction = origin_row - floor_row;
        let col_fraction = origin_col - floor_col;
        let inverse_row = 1.0 - row_fraction;
        let inverse_col = 1.0 - col_fraction;
        let base_row = floor_row as isize;
        let base_col = floor_col as isize;
        Self {
            row_indices: std::array::from_fn(|offset| {
                clamp_index(base_row + offset as isize, rows)
            }),
            col_indices: std::array::from_fn(|offset| {
                clamp_index(base_col + offset as isize, cols)
            }),
            cols,
            coefficients: [
                inverse_row * inverse_col,
                inverse_row * col_fraction,
                row_fraction * inverse_col,
                row_fraction * col_fraction,
            ],
        }
    }

    #[cfg(test)]
    const fn coefficients(&self) -> [f32; 4] {
        self.coefficients
    }

    fn target_index(&self, patch_row: usize, patch_col: usize) -> usize {
        self.row_indices[patch_row] * self.cols + self.col_indices[patch_col]
    }

    fn sample_for_descent(&self, image: &[u8], patch_row: usize, patch_col: usize) -> f32 {
        let row0 = self.row_indices[patch_row];
        let col0 = self.col_indices[patch_col];
        let row1 = self.row_indices[patch_row + 1];
        let col1 = self.col_indices[patch_col + 1];
        let top_left = f32::from(image[row0 * self.cols + col0]);
        let top_right = f32::from(image[row0 * self.cols + col1]);
        let bottom_left = f32::from(image[row1 * self.cols + col0]);
        let bottom_right = f32::from(image[row1 * self.cols + col1]);

        let [
            top_left_weight,
            top_right_weight,
            bottom_left_weight,
            bottom_right_weight,
        ] = self.coefficients;
        let top_right = top_right * top_right_weight;
        let top = top_left.mul_add(top_left_weight, top_right);
        let bottom_left = bottom_left.mul_add(bottom_left_weight, top);
        bottom_right.mul_add(bottom_right_weight, bottom_left)
    }

    /// The shared interpolation and mask prefix of the selected ARMv8.2
    /// candidate helpers. Keep every association in instruction order: two
    /// separately rounded products per source row, bottom minus source before
    /// top is added, then the combined physical-mask multiplication.
    fn candidate_residual(
        &self,
        image: &[u8],
        patch_row: usize,
        patch_col: usize,
        source: u8,
        survives: bool,
    ) -> f32 {
        let row0 = self.row_indices[patch_row];
        let col0 = self.col_indices[patch_col];
        let row1 = self.row_indices[patch_row + 1];
        let col1 = self.col_indices[patch_col + 1];
        let [
            top_left_weight,
            top_right_weight,
            bottom_left_weight,
            bottom_right_weight,
        ] = self.coefficients;
        let top_left = f32::from(image[row0 * self.cols + col0]) * top_left_weight;
        let top_right = f32::from(image[row0 * self.cols + col1]) * top_right_weight;
        let top = top_left + top_right;
        let bottom_left = f32::from(image[row1 * self.cols + col0]) * bottom_left_weight;
        let bottom_right = f32::from(image[row1 * self.cols + col1]) * bottom_right_weight;
        let bottom = bottom_left + bottom_right;
        let difference = (bottom - f32::from(source)) + top;
        difference * f32::from(u8::from(survives))
    }

    /// One selected ARMv8.2 weighted-candidate tap. Keep every association in
    /// instruction order: two separately rounded products per row, bottom
    /// minus source before top is added, then mask, weight and reciprocal.
    #[allow(clippy::too_many_arguments)]
    fn weighted_candidate_residual(
        &self,
        image: &[u8],
        patch_row: usize,
        patch_col: usize,
        source: u8,
        survives: bool,
        raw_weight: f32,
        reciprocal_sum: f32,
    ) -> f32 {
        let masked = self.candidate_residual(image, patch_row, patch_col, source, survives);
        let weighted = masked * raw_weight;
        weighted * reciprocal_sum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(dcol: f32, drow: f32) -> Flow {
        Flow::new(dcol, drow).unwrap()
    }

    #[test]
    fn origin_rounds_in_the_native_sixteen_pixel_target_domain() {
        let row = padded_origin(18, f32::from_bits(0x3dcf_7d3c), Level::Two.rows());
        let col = padded_origin(6, f32::from_bits(0x3dcd_8267), Level::Two.cols());

        assert_eq!(row.to_bits(), 0x4190_cf7c);
        assert_eq!(col.to_bits(), 0x40c3_3608);
        assert_eq!((row - row.floor()).to_bits(), 0x3dcf_7c00);
        assert_eq!((col - col.floor()).to_bits(), 0x3dcd_8200);
    }

    #[test]
    fn weight_denominators_follow_recovered_two_axis_rolling_addition_order() {
        let level = Level::Two;
        let raw_weight: Vec<f32> = (0..level.pixels())
            .map(|index| ((index % 11) + 1) as f32 / 3.0)
            .collect();
        let rolling = rolling_patch_weight_sums(level, &raw_weight);

        let patch_row = 6;
        let patch_col = 2;
        let source_row = patch_row * PATCH_STRIDE;
        let source_col = patch_col * PATCH_STRIDE;
        let mut fresh = 0.0f32;
        for row in 0..PATCH_SIZE {
            for col in 0..PATCH_SIZE {
                fresh += raw_weight[(source_row + row) * level.cols() + source_col + col];
            }
        }

        assert_eq!(rolling.len(), level.patches());
        assert_eq!(
            rolling[patch_row * level.patch_cols() + patch_col].to_bits(),
            0x4302_0002
        );
        assert_eq!(fresh.to_bits(), 0x4301_ffff);
    }

    fn constant_input<D: PisDirection>(
        source_value: u8,
        target_value: u8,
        source_mask: Vec<u8>,
        target_mask: Vec<u8>,
        weights: Vec<f32>,
    ) -> Input<D> {
        let level = Level::Two;
        let pixels = level.pixels();
        let (a, b) = match D::DIRECTION {
            Direction::AtoB => (vec![source_value; pixels], vec![target_value; pixels]),
            Direction::BtoA => (vec![target_value; pixels], vec![source_value; pixels]),
        };
        let mut gradient_col = vec![1.0; pixels];
        let mut gradient_row = vec![2.0; pixels];
        let mut weights = weights;
        for at in 0..pixels {
            if source_mask[at] == 0 {
                gradient_col[at] = 0.0;
                gradient_row[at] = 0.0;
                weights[at] = 0.0;
            }
        }
        Input::<D>::from_native_order(
            level,
            LensPair { a, b },
            LensPair {
                a: source_mask,
                b: target_mask,
            },
            gradient_col,
            gradient_row,
            weights,
            vec![CostMode::Weighted; level.patch_rows()],
        )
        .unwrap()
    }

    #[test]
    fn selected_candidates_preserve_armv8_2_interpolation_and_mask_order() {
        let image = [179, 201, 239, 150];
        let row = f32::from_bits(0x3f2e_ef32);
        let col = f32::from_bits(0x3f51_2f3e);
        let sampling = PatchSamplingPlan::new(row, col, 2, 2);
        let unweighted = sampling.candidate_residual(&image, 0, 0, 0, true);
        let unweighted_difference = sampling.candidate_residual(&image, 0, 0, 167, true);
        let masked_negative = sampling.candidate_residual(&image, 0, 0, 255, false);
        let selected = sampling.weighted_candidate_residual(&image, 0, 0, 0, true, 1.0, 1.0);
        let selected_difference =
            sampling.weighted_candidate_residual(&image, 0, 0, 167, true, 1.0, 1.0);
        let descent = sampling.sample_for_descent(&image, 0, 0);

        assert_eq!(unweighted.to_bits(), 0x432f_ff62);
        assert_eq!(unweighted_difference.to_bits(), 0x410f_f618);
        assert_eq!(masked_negative.to_bits(), (-0.0f32).to_bits());
        assert_eq!(selected.to_bits(), 0x432f_ff62);
        assert_eq!(selected_difference.to_bits(), 0x410f_f618);
        assert_eq!(descent.to_bits(), 0x432f_ff61);
        assert_ne!(selected.to_bits(), descent.to_bits());

        let [top_left, top_right, bottom_left, bottom_right] = sampling.coefficients;
        let top = f32::from(image[0]) * top_left + f32::from(image[1]) * top_right;
        let bottom = f32::from(image[2]) * bottom_left + f32::from(image[3]) * bottom_right;
        let reassociated = (top + bottom) - 167.0;
        assert_eq!(reassociated.to_bits(), 0x410f_f620);
        assert_ne!(selected_difference.to_bits(), reassociated.to_bits());
    }

    #[test]
    fn selected_candidates_reduce_four_lanes_in_native_order() {
        let mut reduction = CandidateReduction::default();
        reduction.accumulate_row(
            [1.25, 0.0, 3.75, 4.125, 5.5, -6.25, 0.0, -8.5],
            [true, false, true, true, true, true, false, true],
        );
        reduction.accumulate_row(
            [0.75, 1.5, -2.25, 0.0, 0.0, 5.25, 6.5, -7.0],
            [true, true, true, false, false, true, true, true],
        );

        assert_eq!(
            reduction.sums.map(f32::to_bits),
            [0x40f0_0000, 0x3f00_0000, 0x4100_0000, 0xc136_0000]
        );
        assert_eq!(
            reduction.square_sums.map(f32::to_bits),
            [0x4201_8000, 0x4289_c000, 0x4275_8000, 0x430a_4400]
        );
        assert_eq!(reduction.survivors, [3, 3, 3, 3]);
        let score = reduction.score();
        assert_eq!(score.survivors(), 12);
        assert_eq!(score.value().to_bits(), 0x4395_8dd5);
    }

    #[test]
    fn sampling_plan_replicates_every_edge_and_corner_without_erasing_fraction() {
        let image = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];
        let sample = |row: f32, col: f32| {
            PatchSamplingPlan::new(row, col, 3, 4).candidate_residual(&image, 0, 0, 0, true)
        };

        assert_eq!(sample(0.25, -0.25), 20.0); // left
        assert_eq!(sample(0.25, 3.75), 50.0); // right
        assert_eq!(sample(-0.25, 0.5), 15.0); // top
        assert_eq!(sample(2.75, 0.5), 95.0); // bottom
        assert_eq!(sample(-0.25, -0.25), 10.0); // top left
        assert_eq!(sample(-0.25, 3.75), 40.0); // top right
        assert_eq!(sample(2.75, -0.25), 90.0); // bottom left
        assert_eq!(sample(2.75, 3.75), 120.0); // bottom right
        assert_eq!(sample(0.0, 0.0), 10.0); // integer first pixel
        assert_eq!(sample(2.0, 3.0), 120.0); // integer last pixel
        assert_eq!(sample(0.25, 0.5), 25.0); // interior
    }

    #[test]
    fn cached_patch_indices_match_the_old_scalar_lookup_at_every_tap() {
        fn old_indices(
            origin_row: f32,
            origin_col: f32,
            rows: usize,
            cols: usize,
            patch_row: usize,
            patch_col: usize,
        ) -> [usize; 4] {
            let base_row = origin_row.floor() as isize;
            let base_col = origin_col.floor() as isize;
            [
                clamp_index(base_row + patch_row as isize, rows),
                clamp_index(base_col + patch_col as isize, cols),
                clamp_index(base_row + patch_row as isize + 1, rows),
                clamp_index(base_col + patch_col as isize + 1, cols),
            ]
        }

        fn old_descent_sample(
            image: &[u8],
            cols: usize,
            indices: [usize; 4],
            coefficients: [f32; 4],
        ) -> f32 {
            let [row0, col0, row1, col1] = indices;
            let top_left = f32::from(image[row0 * cols + col0]);
            let top_right = f32::from(image[row0 * cols + col1]);
            let bottom_left = f32::from(image[row1 * cols + col0]);
            let bottom_right = f32::from(image[row1 * cols + col1]);
            let [
                top_left_weight,
                top_right_weight,
                bottom_left_weight,
                bottom_right_weight,
            ] = coefficients;
            let top_right = top_right * top_right_weight;
            let top = top_left.mul_add(top_left_weight, top_right);
            let bottom_left = bottom_left.mul_add(bottom_left_weight, top);
            bottom_right.mul_add(bottom_right_weight, bottom_left)
        }

        fn old_candidate_residual(
            image: &[u8],
            cols: usize,
            indices: [usize; 4],
            coefficients: [f32; 4],
            source: u8,
            survives: bool,
        ) -> f32 {
            let [row0, col0, row1, col1] = indices;
            let [
                top_left_weight,
                top_right_weight,
                bottom_left_weight,
                bottom_right_weight,
            ] = coefficients;
            let top_left = f32::from(image[row0 * cols + col0]) * top_left_weight;
            let top_right = f32::from(image[row0 * cols + col1]) * top_right_weight;
            let top = top_left + top_right;
            let bottom_left = f32::from(image[row1 * cols + col0]) * bottom_left_weight;
            let bottom_right = f32::from(image[row1 * cols + col1]) * bottom_right_weight;
            let bottom = bottom_left + bottom_right;
            let difference = (bottom - f32::from(source)) + top;
            difference * f32::from(u8::from(survives))
        }

        for level in [Level::One, Level::Two] {
            let rows = level.rows();
            let cols = level.cols();
            let image: Vec<u8> = (0..level.pixels())
                .map(|index| ((index * 37 + 11) & 0xff) as u8)
                .collect();
            let interior = (11.375, 3.625);
            let top = -6.625;
            let bottom = rows as f32 - 0.375;
            let left = -6.625;
            let right = cols as f32 - 0.375;
            let origins = [
                interior,
                (top, interior.1),
                (bottom, interior.1),
                (interior.0, left),
                (interior.0, right),
                (top, left),
                (top, right),
                (bottom, left),
                (bottom, right),
            ];

            for (origin_row, origin_col) in origins {
                let sampling = PatchSamplingPlan::new(origin_row, origin_col, rows, cols);
                let coefficients = sampling.coefficients();
                for patch_row in 0..PATCH_SIZE {
                    for patch_col in 0..PATCH_SIZE {
                        let indices =
                            old_indices(origin_row, origin_col, rows, cols, patch_row, patch_col);
                        let [row0, col0, row1, col1] = indices;
                        assert_eq!(sampling.row_indices[patch_row], row0);
                        assert_eq!(sampling.col_indices[patch_col], col0);
                        assert_eq!(sampling.row_indices[patch_row + 1], row1);
                        assert_eq!(sampling.col_indices[patch_col + 1], col1);
                        assert_eq!(
                            sampling.target_index(patch_row, patch_col),
                            row0 * cols + col0
                        );

                        let old_descent = old_descent_sample(&image, cols, indices, coefficients);
                        let cached_descent =
                            sampling.sample_for_descent(&image, patch_row, patch_col);
                        assert_eq!(cached_descent.to_bits(), old_descent.to_bits());

                        for survives in [false, true] {
                            let source = ((patch_row * 31 + patch_col * 17 + 7) & 0xff) as u8;
                            let old_candidate = old_candidate_residual(
                                &image,
                                cols,
                                indices,
                                coefficients,
                                source,
                                survives,
                            );
                            let cached_candidate = sampling
                                .candidate_residual(&image, patch_row, patch_col, source, survives);
                            assert_eq!(cached_candidate.to_bits(), old_candidate.to_bits());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn sampling_plan_reuses_one_translated_fraction_for_every_patch_tap() {
        let image: Vec<u8> = (0..100).collect();
        let sampling = PatchSamplingPlan::new(
            f32::from_bits(0x42f8_266f) - 16.0,
            f32::from_bits(0x417d_3f55) - 16.0,
            10,
            10,
        );
        let coefficient_bits = sampling.coefficients().map(f32::to_bits);

        let _first = sampling.sample_for_descent(&image, 0, 0);
        let _last = sampling.sample_for_descent(&image, 7, 7);

        assert_eq!(
            coefficient_bits,
            [0x3e22_f162, 0x3f44_0c27, 0x3c53_95d8, 0x3d7e_928a]
        );
        assert_eq!(sampling.coefficients().map(f32::to_bits), coefficient_bits);
    }

    /// The retired implementation clamped a floating coordinate before it
    /// extracted the fraction. Keep it only as the explicit negative control
    /// for the delayed-seal row-36 regression.
    fn sample_replicate_native_fma_clamp_before_fraction(
        image: &[u8],
        rows: usize,
        cols: usize,
        row: f32,
        col: f32,
    ) -> f32 {
        let row = row.clamp(0.0, rows as f32 - 1.0);
        let col = col.clamp(0.0, cols as f32 - 1.0);
        let row0 = row.floor() as usize;
        let col0 = col.floor() as usize;
        let row1 = (row0 + 1).min(rows - 1);
        let col1 = (col0 + 1).min(cols - 1);
        let row_fraction = row - row0 as f32;
        let col_fraction = col - col0 as f32;
        let inverse_row = 1.0 - row_fraction;
        let inverse_col = 1.0 - col_fraction;
        let top_left_weight = inverse_row * inverse_col;
        let top_right_weight = inverse_row * col_fraction;
        let bottom_left_weight = row_fraction * inverse_col;
        let bottom_right_weight = row_fraction * col_fraction;
        let top_right = f32::from(image[row0 * cols + col1]) * top_right_weight;
        let top = f32::from(image[row0 * cols + col0]).mul_add(top_left_weight, top_right);
        let bottom_left = f32::from(image[row1 * cols + col0]).mul_add(bottom_left_weight, top);
        f32::from(image[row1 * cols + col1]).mul_add(bottom_right_weight, bottom_left)
    }

    #[test]
    fn row36_attempt4_left_border_fraction_probe() {
        // Bytes from row36-descent-loop-1c3b4a08-02, adopted through the
        // distinct delayed-seal receipt. That receipt validates the preserved
        // transaction's current bytes and semantics, but is not a
        // contemporaneous outer hash or original launch receipt.
        const SOURCE: [u8; 64] = [
            95, 94, 91, 93, 96, 92, 95, 98, 95, 93, 103, 91, 94, 96, 95, 96, 89, 94, 101, 78, 94,
            96, 97, 92, 95, 97, 92, 87, 96, 96, 96, 96, 96, 96, 92, 93, 96, 97, 96, 93, 95, 91, 96,
            95, 94, 94, 95, 97, 93, 71, 94, 96, 96, 94, 94, 94, 95, 95, 92, 93, 91, 92, 85, 92,
        ];
        // Nine physical columns begin at padded column 15. Columns 15 and 16
        // are the duplicate border pixel and logical column zero.
        const PADDED_TARGET: [u8; 81] = [
            76, 76, 89, 88, 92, 90, 89, 92, 94, 81, 81, 89, 96, 85, 92, 91, 92, 91, 80, 80, 89, 92,
            77, 93, 92, 90, 85, 88, 88, 90, 86, 86, 93, 92, 91, 86, 91, 91, 90, 87, 91, 94, 92, 92,
            89, 90, 90, 90, 92, 90, 90, 89, 91, 90, 79, 79, 77, 93, 93, 92, 92, 91, 91, 87, 87, 88,
            87, 89, 86, 85, 84, 90, 93, 93, 89, 89, 90, 91, 88, 90, 91,
        ];
        const GRADIENT_COL: [i16; 64] = [
            0, -3, 1, 4, 0, -5, 14, 5, 0, 24, -21, -20, 27, 4, 2, -9, 0, 29, -44, -19, 50, 7, -8,
            -29, 0, 2, -39, 5, 40, 3, -8, -27, 0, -10, -12, 10, 16, 1, -5, -13, 0, -1, 30, 2, 0, 0,
            2, -5, 0, 0, 52, 1, -6, -9, 3, 10, 0, -11, 18, 2, -4, -12, 0, 20,
        ];
        const GRADIENT_ROW: [i16; 64] = [
            -2, 9, 13, -6, -10, 3, 6, 3, -12, 4, 5, -22, -15, 8, 2, -21, 8, -3, -22, -17, 0, 3, 2,
            -5, 18, 2, -1, 23, 20, 3, 0, 9, -12, -8, 10, 18, 2, -7, -3, 5, -56, -51, -18, 8, 0, -8,
            -6, 1, 8, 4, -6, -11, -10, -17, -27, -18, 62, 55, 20, -8, -6, 0, 4, 2,
        ];

        let translated_row = f32::from_bits(0x42f8_266f);
        let translated_col = f32::from_bits(0x417d_3f55);
        let row_fraction = translated_row - translated_row.floor();
        let col_fraction = translated_col - translated_col.floor();
        let unpadded_col = translated_col - 16.0;
        let unpadded_target: Vec<u8> = PADDED_TARGET
            .chunks_exact(9)
            .flat_map(|row| row[1..].iter().copied())
            .collect();

        let production = PatchSamplingPlan::new(row_fraction, unpadded_col, 9, 8);
        let physical = PatchSamplingPlan::new(row_fraction, col_fraction, 9, 9);
        let accumulate = |retired_clamp_before_fraction: bool| {
            let mut statistics = Statistics::default();
            for row in 0..PATCH_SIZE {
                for col in 0..PATCH_SIZE {
                    let at = row * PATCH_SIZE + col;
                    let target = if retired_clamp_before_fraction {
                        sample_replicate_native_fma_clamp_before_fraction(
                            &unpadded_target,
                            9,
                            8,
                            row_fraction + row as f32,
                            unpadded_col + col as f32,
                        )
                    } else {
                        let target = production.sample_for_descent(&unpadded_target, row, col);
                        let padded = physical.sample_for_descent(&PADDED_TARGET, row, col);
                        assert_eq!(target.to_bits(), padded.to_bits());
                        target
                    };
                    let residual = target - f32::from(SOURCE[at]);
                    statistics.accumulate_descent(
                        residual,
                        f32::from(GRADIENT_COL[at]),
                        f32::from(GRADIENT_ROW[at]),
                    );
                }
            }
            [
                statistics.sum.to_bits(),
                statistics.sum_sq.to_bits(),
                statistics.rhs_col.to_bits(),
                statistics.rhs_row.to_bits(),
            ]
        };

        let production_replicated_padding = accumulate(false);
        let retired_clamp_before_fraction = accumulate(true);
        assert_eq!(
            production_replicated_padding,
            [0xc39c_be2f, 0x450f_640f, 0x43af_411c, 0xc0c6_cee0]
        );
        assert_eq!(
            retired_clamp_before_fraction,
            [0xc39c_be2f, 0x450f_640f, 0x43af_411c, 0xc0c6_d2e0]
        );
    }

    #[test]
    fn scalar_descent_normalizes_before_scaling_the_difference() {
        let difference = f32::from_bits(0xc327_2b0f);
        let weight = f32::from_bits(0x43ac_52ed);
        let reciprocal_sum = f32::from_bits(0x38a0_d364);
        let residual = weighted_residual(difference, weight, reciprocal_sum);

        assert_eq!(residual.to_bits(), 0xc08d_62b1);
    }

    #[test]
    fn scalar_descent_uses_fused_square_and_rhs_accumulators() {
        let residual = f32::from_bits(0x4097_4c8f);
        let gradient = f32::from_bits(0x42a1_7a6c);
        let initial = f32::from_bits(0xc3a2_b230);
        let mut native = Statistics {
            sum_sq: initial,
            rhs_col: initial,
            rhs_row: initial,
            ..Statistics::default()
        };
        native.accumulate_descent(residual, gradient, gradient);

        assert_eq!(native.rhs_col.to_bits(), 0x4261_6684);
        assert_eq!(native.sum_sq.to_bits(), 0xc397_84c3);
        assert_eq!(native.rhs_col.to_bits(), native.rhs_row.to_bits());
    }

    fn checked_prepared_input<D: PisDirection>(
        mask_a: Vec<u8>,
        gradient_col: Vec<f32>,
        gradient_row: Vec<f32>,
        raw_weight: Vec<f32>,
    ) -> Result<Input<D>, InputError> {
        let level = Level::Two;
        let pixels = level.pixels();
        Input::<D>::from_native_order(
            level,
            LensPair {
                a: vec![0; pixels],
                b: vec![0; pixels],
            },
            LensPair {
                a: mask_a,
                b: vec![255; pixels],
            },
            gradient_col,
            gradient_row,
            raw_weight,
            vec![CostMode::Unweighted; level.patch_rows()],
        )
    }

    #[test]
    fn shared_preparation_reuses_planes_and_keeps_every_solver_result_bit() {
        fn assert_flow_bits(actual: Flow, expected: Flow) {
            assert_eq!(actual.dcol.to_bits(), expected.dcol.to_bits());
            assert_eq!(actual.drow.to_bits(), expected.drow.to_bits());
        }

        fn assert_grid_bits(actual: &PatchGrid<AtoB>, expected: &PatchGrid<AtoB>) {
            assert_eq!(actual.level, expected.level);
            assert_eq!(actual.patches.len(), expected.patches.len());
            for (actual, expected) in actual.patches.iter().zip(&expected.patches) {
                assert_flow_bits(actual.flow, expected.flow);
                assert_eq!(actual.residual.to_bits(), expected.residual.to_bits());
                for (actual, expected) in actual.passes.iter().zip(&expected.passes) {
                    assert_eq!(actual.winner, expected.winner);
                    assert_eq!(actual.descent_admitted, expected.descent_admitted);
                    assert_eq!(actual.descent_iterations, expected.descent_iterations);
                    assert_eq!(
                        actual.stopped_on_no_improvement,
                        expected.stopped_on_no_improvement
                    );
                    assert_eq!(actual.guarded, expected.guarded);
                    assert_flow_bits(actual.stored_flow, expected.stored_flow);
                    for (actual, expected) in actual.candidates.iter().zip(&expected.candidates) {
                        match (actual, expected) {
                            (Some(actual), Some(expected)) => {
                                assert_flow_bits(actual.flow, expected.flow);
                                assert_eq!(
                                    actual.score.value.to_bits(),
                                    expected.score.value.to_bits()
                                );
                                assert_eq!(actual.score.survivors, expected.score.survivors);
                            }
                            (None, None) => {}
                            _ => panic!("shared and owned candidate presence differs"),
                        }
                    }
                }
            }
        }

        let level = Level::Two;
        let pixels = level.pixels();
        let image_a = Arc::new((0..pixels).map(|index| index as u8).collect());
        let image_b = Arc::new(
            (0..pixels)
                .map(|index| (index as u8).wrapping_mul(3))
                .collect(),
        );
        let mask_a = Arc::new(vec![u8::MAX; pixels]);
        let mask_b = Arc::new(vec![u8::MAX; pixels]);
        let gradient_col = Arc::new((0..pixels).map(|index| (index % 13) as f32 - 6.0).collect());
        let gradient_row = Arc::new((0..pixels).map(|index| 4.0 - (index % 9) as f32).collect());
        let raw_weight = Arc::new(
            (0..pixels)
                .map(|index| ((index % 17) + 1) as f32 / 17.0)
                .collect(),
        );
        let modes = (0..level.patch_rows())
            .map(|row| {
                if row % 2 == 0 {
                    CostMode::Weighted
                } else {
                    CostMode::Unweighted
                }
            })
            .collect::<Vec<_>>();

        let shared = Input::<AtoB>::from_shared_native_order(
            level,
            LensPair {
                a: Arc::clone(&image_a),
                b: Arc::clone(&image_b),
            },
            LensPair {
                a: Arc::clone(&mask_a),
                b: Arc::clone(&mask_b),
            },
            Arc::clone(&gradient_col),
            Arc::clone(&gradient_row),
            Arc::clone(&raw_weight),
            modes.clone(),
        )
        .unwrap();
        assert!(Arc::ptr_eq(&shared.source, &image_a));
        assert!(Arc::ptr_eq(&shared.target, &image_b));
        assert!(Arc::ptr_eq(&shared.source_slot_mask, &mask_a));
        assert!(Arc::ptr_eq(&shared.target_slot_mask, &mask_b));
        assert!(Arc::ptr_eq(&shared.gradient_col, &gradient_col));
        assert!(Arc::ptr_eq(&shared.gradient_row, &gradient_row));
        assert!(Arc::ptr_eq(&shared.raw_weight, &raw_weight));

        let owned = Input::<AtoB>::from_native_order(
            level,
            LensPair {
                a: image_a.to_vec(),
                b: image_b.to_vec(),
            },
            LensPair {
                a: mask_a.to_vec(),
                b: mask_b.to_vec(),
            },
            gradient_col.to_vec(),
            gradient_row.to_vec(),
            raw_weight.to_vec(),
            modes,
        )
        .unwrap();
        let shared_result = solve_with_descent_admission(
            &shared,
            InitialGrid::coarse_zeros(),
            None,
            DescentAdmission::EveryPatch,
        )
        .unwrap();
        let owned_result = solve_with_descent_admission(
            &owned,
            InitialGrid::coarse_zeros(),
            None,
            DescentAdmission::EveryPatch,
        )
        .unwrap();

        assert_grid_bits(&shared_result, &owned_result);
    }

    #[test]
    fn selected_levels_and_patch_grids_are_fixed() {
        assert_eq!((PATCH_SIZE, PATCH_STRIDE), (8, 3));
        assert_eq!((Level::One.rows(), Level::One.cols()), (540, 30));
        assert_eq!((Level::One.patch_rows(), Level::One.patch_cols()), (178, 8));
        assert_eq!(Level::One.patches(), 1424);
        assert_eq!((Level::Two.rows(), Level::Two.cols()), (270, 15));
        assert_eq!((Level::Two.patch_rows(), Level::Two.patch_cols()), (88, 3));
        assert_eq!(Level::Two.patches(), 264);
        assert_eq!(DESCENTS_PER_PASS, 6);
        assert_eq!(SENTINEL_SCORE.to_bits(), 0x5015_02f9);
    }

    #[test]
    fn reverse_swaps_images_but_never_the_native_mask_pair() {
        let level = Level::Two;
        let pixels = level.pixels();
        let mut mask_a = vec![0; pixels];
        let mut mask_b = vec![0; pixels];
        mask_a[0] = 17;
        mask_b[0] = 29;
        let mut weights = vec![0.0; pixels];
        weights[0] = 1.0;
        let input = Input::<BtoA>::from_native_order(
            level,
            LensPair {
                a: vec![11; pixels],
                b: vec![23; pixels],
            },
            LensPair {
                a: mask_a,
                b: mask_b,
            },
            vec![0.0; pixels],
            vec![0.0; pixels],
            weights,
            vec![CostMode::Weighted; level.patch_rows()],
        )
        .unwrap();
        assert_eq!(input.source[0], 23);
        assert_eq!(input.target[0], 11);
        assert_eq!(input.source_slot_mask[0], 17);
        assert_eq!(input.target_slot_mask[0], 29);
        assert_eq!(input.direction(), Direction::BtoA);
    }

    #[test]
    fn prepared_planes_reject_nonfinite_and_values_beneath_physical_mask_a() {
        let level = Level::Two;
        let pixels = level.pixels();

        let mut gradient_col = vec![0.0; pixels];
        gradient_col[7] = f32::NAN;
        assert_eq!(
            checked_prepared_input::<AtoB>(
                vec![255; pixels],
                gradient_col,
                vec![0.0; pixels],
                vec![0.0; pixels],
            )
            .unwrap_err(),
            InputError::NonFinitePreparedValue {
                level,
                part: "column gradient",
                index: 7,
            },
        );

        let mut gradient_row = vec![0.0; pixels];
        gradient_row[11] = f32::INFINITY;
        assert_eq!(
            checked_prepared_input::<BtoA>(
                vec![255; pixels],
                vec![0.0; pixels],
                gradient_row,
                vec![0.0; pixels],
            )
            .unwrap_err(),
            InputError::NonFinitePreparedValue {
                level,
                part: "row gradient",
                index: 11,
            },
        );

        let mut raw_weight = vec![0.0; pixels];
        raw_weight[13] = f32::NEG_INFINITY;
        assert_eq!(
            checked_prepared_input::<AtoB>(
                vec![255; pixels],
                vec![0.0; pixels],
                vec![0.0; pixels],
                raw_weight,
            )
            .unwrap_err(),
            InputError::NonFinitePreparedValue {
                level,
                part: "raw SSD weight",
                index: 13,
            },
        );

        let mut mask_a = vec![255; pixels];
        mask_a[17] = 0;
        let mut masked_gradient = vec![0.0; pixels];
        masked_gradient[17] = 1.0;
        assert_eq!(
            checked_prepared_input::<BtoA>(
                mask_a.clone(),
                masked_gradient,
                vec![0.0; pixels],
                vec![0.0; pixels],
            )
            .unwrap_err(),
            InputError::NonZeroBelowPhysicalMaskA {
                level,
                part: "column gradient",
                index: 17,
            },
        );

        let mut masked_weight = vec![0.0; pixels];
        masked_weight[17] = 1.0;
        assert_eq!(
            checked_prepared_input::<AtoB>(
                mask_a,
                vec![0.0; pixels],
                vec![0.0; pixels],
                masked_weight,
            )
            .unwrap_err(),
            InputError::NonZeroBelowPhysicalMaskA {
                level,
                part: "raw SSD weight",
                index: 17,
            },
        );
    }

    #[test]
    fn prepared_raw_weights_reject_finite_negatives_but_accept_signed_zero() {
        let level = Level::Two;
        let pixels = level.pixels();
        let mut negative = vec![0.0; pixels];
        negative[19] = -f32::MIN_POSITIVE;
        let error = checked_prepared_input::<AtoB>(
            vec![255; pixels],
            vec![0.0; pixels],
            vec![0.0; pixels],
            negative,
        )
        .unwrap_err();
        assert_eq!(error, InputError::NegativeRawWeight { level, index: 19 });
        assert_eq!(
            error.to_string(),
            "ONE X2 PIS level 2 raw SSD weight at pixel 19 is negative",
        );

        let mut signed_zero = vec![0.0; pixels];
        signed_zero[0] = -0.0;
        let mut mask_a = vec![255; pixels];
        mask_a[0] = 0;
        let input = checked_prepared_input::<BtoA>(
            mask_a,
            vec![0.0; pixels],
            vec![0.0; pixels],
            signed_zero,
        )
        .unwrap();
        assert_eq!(input.raw_weight[0].to_bits(), (-0.0f32).to_bits());
        assert_eq!(input.raw_weight[1].to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn weighted_score_uses_patch_sum_but_survivor_count_for_mean() {
        let level = Level::Two;
        let pixels = level.pixels();
        let input = constant_input::<AtoB>(
            10,
            30,
            vec![255; pixels],
            vec![255; pixels],
            (0..pixels).map(|index| (index % 7 + 1) as f32).collect(),
        );
        let score = input.score(0, 0, flow(0.25, 0.0));
        assert_eq!(score.survivors(), 64);

        assert_eq!(score.value().to_bits(), 0x3fd0_a874);

        let zero_sum = constant_input::<AtoB>(
            10,
            30,
            vec![255; pixels],
            vec![255; pixels],
            vec![0.0; pixels],
        )
        .score(0, 0, Flow::ZERO);
        assert_eq!(zero_sum.survivors(), 64);
        assert_eq!(zero_sum.value(), 0.0);
    }

    #[test]
    fn caller_supplied_grid_rows_select_weighted_or_unweighted_cost() {
        let level = Level::Two;
        let cols = level.cols();
        let pixels = level.pixels();
        let target: Vec<_> = (0..pixels).map(|at| (at % 251) as u8).collect();
        let weights: Vec<_> = (0..pixels).map(|at| (at % 7 + 1) as f32).collect();

        let unweighted_expected = |patch_row: usize| {
            let source_row = patch_row * PATCH_STRIDE;
            let mut sum = 0.0f32;
            let mut sum_sq = 0.0f32;
            for row in 0..PATCH_SIZE {
                for col in 0..PATCH_SIZE {
                    let at = (source_row + row) * cols + col;
                    let difference = f32::from(target[at]);
                    sum += difference;
                    sum_sq += difference * difference;
                }
            }
            sum_sq - sum * sum / 64.0
        };
        let unweighted_expected = unweighted_expected(0);

        let mut cost_modes = vec![CostMode::Weighted; level.patch_rows()];
        cost_modes[0] = CostMode::Unweighted;
        let input = Input::<AtoB>::from_native_order(
            level,
            LensPair {
                a: vec![0; pixels],
                b: target,
            },
            LensPair {
                a: vec![255; pixels],
                b: vec![255; pixels],
            },
            vec![0.0; pixels],
            vec![0.0; pixels],
            weights,
            cost_modes,
        )
        .unwrap();

        let unweighted = input.score(0, 0, Flow::ZERO);
        let weighted = input.score(1, 0, Flow::ZERO);
        assert_eq!(unweighted.survivors(), 64);
        assert_eq!(weighted.survivors(), 64);
        assert_eq!(unweighted.value().to_bits(), unweighted_expected.to_bits());
        assert_ne!(weighted.value().to_bits(), unweighted.value().to_bits());
    }

    #[test]
    fn target_intensity_is_bilinear_with_replicated_border() {
        let level = Level::Two;
        let cols = level.cols();
        let pixels = level.pixels();
        let mut target = vec![0; pixels];
        for row in 0..=PATCH_SIZE {
            for col in 0..=PATCH_SIZE {
                target[row * cols + col] = (row * 10 + col) as u8;
            }
        }
        let input = Input::<AtoB>::from_native_order(
            level,
            LensPair {
                a: vec![0; pixels],
                b: target,
            },
            LensPair {
                a: vec![255; pixels],
                b: vec![255; pixels],
            },
            vec![0.0; pixels],
            vec![0.0; pixels],
            vec![0.0; pixels],
            vec![CostMode::Unweighted; level.patch_rows()],
        )
        .unwrap();

        let interpolated = input.descent_statistics(0, 0, flow(0.5, 0.25));
        let mut expected_sum = 0.0f32;
        let mut expected_sum_sq = 0.0f32;
        for row in 0..PATCH_SIZE {
            for col in 0..PATCH_SIZE {
                let difference = (row * 10 + col) as f32 + 3.0;
                expected_sum += difference;
                expected_sum_sq += difference * difference;
            }
        }
        assert_eq!(interpolated.sum.to_bits(), expected_sum.to_bits());
        assert_eq!(interpolated.sum_sq.to_bits(), expected_sum_sq.to_bits());

        let replicated = input.descent_statistics(0, 0, flow(-7.0, -7.0));
        assert_eq!((replicated.sum, replicated.sum_sq), (0.0, 0.0));
        assert_eq!(replicated.survivors, 64);
    }

    #[test]
    fn candidate_masks_apply_per_tap_and_target_uses_floor_not_round() {
        let level = Level::Two;
        let rows = level.rows();
        let cols = level.cols();
        let pixels = level.pixels();
        let source_mask = vec![255; pixels];
        let mut target_mask = vec![0; pixels];
        target_mask[..PATCH_SIZE].fill(255);
        target_mask[cols] = 255;
        let weights = vec![1.0; pixels];
        let mut input = constant_input::<AtoB>(
            10,
            20,
            source_mask.clone(),
            target_mask.clone(),
            weights.clone(),
        );
        input.cost_modes[0] = CostMode::Unweighted;
        // floor(0.75) keeps the nine marked cells. Rounding to one would leave
        // only seven, below native's nine-survivor score threshold.
        let nine = input.score(0, 0, flow(0.75, 0.0));
        assert_eq!(nine.survivors(), 9);
        assert!(!nine.is_sentinel());
        assert_eq!(nine.value().to_bits(), 0.0f32.to_bits());

        target_mask[cols] = 0;
        let mut eight_input =
            constant_input::<AtoB>(10, 20, source_mask.clone(), target_mask, weights.clone());
        eight_input.cost_modes[0] = CostMode::Unweighted;
        let eight = eight_input.score(0, 0, flow(0.75, 0.0));
        assert_eq!(eight.survivors(), 8);
        assert!(eight.is_sentinel());

        let mut source_mask = source_mask;
        source_mask[0] = 0;
        let mut source_rejects_one_input = constant_input::<AtoB>(
            10,
            20,
            source_mask,
            {
                let mut mask = vec![0; pixels];
                mask[..PATCH_SIZE].fill(255);
                mask[cols] = 255;
                mask
            },
            weights,
        );
        source_rejects_one_input.cost_modes[0] = CostMode::Unweighted;
        let source_rejects_one = source_rejects_one_input.score(0, 0, flow(0.75, 0.0));
        assert_eq!(source_rejects_one.survivors(), 8);
        assert!(source_rejects_one.is_sentinel());
        assert_eq!(rows, 270); // keep the asymmetric row/column fixture explicit
    }

    #[test]
    fn descent_rhs_uses_dual_survivors_but_full_prepared_source_support() {
        let level = Level::Two;
        let cols = level.cols();
        let pixels = level.pixels();
        let source_mask = vec![255; pixels];
        let mut target_mask = vec![0; pixels];
        target_mask[0] = 255;
        let mut gradient_col = vec![0.0; pixels];
        let mut gradient_row = vec![0.0; pixels];
        for row in 0..PATCH_SIZE {
            for col in 0..PATCH_SIZE {
                let at = row * cols + col;
                if row < PATCH_SIZE / 2 {
                    gradient_col[at] = 1.0;
                } else {
                    gradient_row[at] = 1.0;
                }
            }
        }
        let input = Input::<AtoB>::from_native_order(
            level,
            LensPair {
                a: vec![10; pixels],
                b: vec![30; pixels],
            },
            LensPair {
                a: source_mask,
                b: target_mask,
            },
            gradient_col,
            gradient_row,
            vec![1.0; pixels],
            vec![CostMode::Weighted; level.patch_rows()],
        )
        .unwrap();

        let mut kernel = ImageKernel::new(&input);
        let step = kernel.descent(0, 0, Flow::ZERO).unwrap();
        assert_eq!(step.survivors, 1);

        // One target-surviving tap contributes d=20/64, Rx=d, Ry=0. The full
        // prepared source patch still contributes Gx=Gy=32 and diagonal
        // H=(32,32); target-mask rejection does not prune those terms. The
        // selected masked route divides its mean corrections by the one
        // surviving tap.
        let d = 20.0f32 / 64.0;
        let count = 1.0f32;
        let rhs_col = d - d * 32.0 / count;
        let rhs_row = 0.0 - d * 32.0 / count;
        assert_eq!(step.delta.dcol().to_bits(), (rhs_col / 32.0).to_bits());
        assert_eq!(step.delta.drow().to_bits(), (rhs_row / 32.0).to_bits());
        assert_eq!(step.residual, d * d - d * d / count);
    }

    #[test]
    fn all_masked_descent_executes_the_native_zero_step() {
        let level = Level::Two;
        let pixels = level.pixels();
        let input =
            constant_input::<AtoB>(10, 30, vec![0; pixels], vec![0; pixels], vec![0.0; pixels]);

        let score = input.score(0, 0, Flow::ZERO);
        assert_eq!(score.survivors(), 0);
        assert!(score.is_sentinel());
        let step = ImageKernel::new(&input)
            .descent(0, 0, Flow::ZERO)
            .expect("selected masked helper still runs with zero survivors");
        assert_eq!(step.survivors, 0);
        assert_eq!(step.delta.dcol().to_bits(), 0x0000_0000);
        assert_eq!(step.delta.drow().to_bits(), 0x0000_0000);
        assert_eq!(step.residual.to_bits(), 0x5015_02f9);
    }

    #[derive(Default)]
    struct ZeroSurvivorStepKernel {
        calls: usize,
    }

    impl Kernel for ZeroSurvivorStepKernel {
        fn score(&mut self, _pass: usize, _patch: usize, _flow: Flow) -> Score {
            Score {
                value: 7.0,
                survivors: 9,
            }
        }

        fn descent(&mut self, _pass: usize, _patch: usize, _flow: Flow) -> Option<DescentStep> {
            self.calls += 1;
            Some(DescentStep {
                delta: Flow::ZERO,
                residual: SENTINEL_SCORE,
                survivors: 0,
            })
        }
    }

    #[test]
    fn zero_survivor_step_stops_once_per_pass_and_retains_the_candidate() {
        let mut kernel = ZeroSurvivorStepKernel::default();
        let patches = schedule(
            1,
            1,
            &[Flow::ZERO],
            None,
            None,
            DESCENTS_PER_PASS,
            &mut kernel,
        );

        assert_eq!(kernel.calls, 2);
        for report in patches[0].passes {
            assert_eq!(report.descent_iterations, 1);
            assert!(report.stopped_on_no_improvement);
            assert!(!report.guarded);
        }
        assert_eq!(patches[0].flow, Flow::ZERO);
        assert_eq!(patches[0].residual, 7.0);
    }

    #[test]
    fn descent_uses_raw_hessian_cross_term_and_positive_determinant_clamp() {
        let level = Level::Two;
        let pixels = level.pixels();
        let mut target_mask = vec![0; pixels];
        target_mask[0] = 255;
        let gradient_col = vec![1.0; pixels];
        let mut gradient_row = vec![1.0; pixels];
        gradient_row[0] = 1.001;
        let input = Input::<AtoB>::from_native_order(
            level,
            LensPair {
                a: vec![10; pixels],
                b: vec![30; pixels],
            },
            LensPair {
                a: vec![255; pixels],
                b: target_mask,
            },
            gradient_col,
            gradient_row,
            vec![1.0; pixels],
            vec![CostMode::Weighted; level.patch_rows()],
        )
        .unwrap();

        let source = input.prepared_source_terms(0, 0);
        assert_ne!(source.h_col_row, 0.0);
        let raw_determinant =
            source.h_col_col * source.h_row_row - source.h_col_row * source.h_col_row;
        assert!(raw_determinant.abs() < 0.001);

        let statistics = input.descent_statistics(0, 0, Flow::ZERO);
        let count = statistics.survivors as f32;
        let rhs_col = statistics.rhs_col - statistics.sum * source.gradient_col_sum / count;
        let rhs_row = statistics.rhs_row - statistics.sum * source.gradient_row_sum / count;
        let determinant = 0.001f32;
        let inverse_col_col = source.h_row_row / determinant;
        let inverse_col_row = -source.h_col_row / determinant;
        let inverse_row_row = source.h_col_col / determinant;
        let expected_col = inverse_col_col.mul_add(rhs_col, inverse_col_row * rhs_row);
        let expected_row = inverse_col_row.mul_add(rhs_col, inverse_row_row * rhs_row);

        let step = ImageKernel::new(&input).descent(0, 0, Flow::ZERO).unwrap();
        assert_eq!(step.delta.dcol().to_bits(), expected_col.to_bits());
        assert_eq!(step.delta.drow().to_bits(), expected_row.to_bits());
    }

    #[test]
    fn source_model_uses_native_fused_determinant() {
        let terms = PreparedSourceTerms {
            gradient_col_sum: 0.0,
            gradient_row_sum: 0.0,
            h_col_col: f32::from_bits(0x4547_e588),
            h_col_row: f32::from_bits(0x454c_efee),
            h_row_row: f32::from_bits(0x478f_e475),
        };
        let negative_cross_square = -(terms.h_col_row * terms.h_col_row);
        let fused = terms
            .h_col_col
            .mul_add(terms.h_row_row, negative_cross_square);
        let split = terms.h_col_col * terms.h_row_row + negative_cross_square;
        assert_ne!(fused.to_bits(), split.to_bits());

        let model = terms.inverted();
        assert_eq!(
            model.inverse_col_col.to_bits(),
            (terms.h_row_row / fused).to_bits()
        );
        assert_ne!(
            model.inverse_col_col.to_bits(),
            (terms.h_row_row / split).to_bits()
        );
    }

    #[test]
    fn source_model_uses_native_fused_matrix_vector_products() {
        let model = PreparedSourceModel {
            gradient_col_sum: 0.0,
            gradient_row_sum: 0.0,
            inverse_col_col: f32::from_bits(0x4097_4c8f),
            inverse_col_row: f32::from_bits(0xc0a1_5b23),
            inverse_row_row: f32::from_bits(0x4097_4c8f),
        };
        let rhs_col = f32::from_bits(0x42a1_7a6c);
        let rhs_row = f32::from_bits(0x4281_1022);
        let rounded_cross_row = model.inverse_col_row * rhs_row;
        let fused_col = model.inverse_col_col.mul_add(rhs_col, rounded_cross_row);
        let split_col = model.inverse_col_col * rhs_col + rounded_cross_row;
        assert_ne!(fused_col.to_bits(), split_col.to_bits());

        let delta = model.delta(rhs_col, rhs_row);
        assert_eq!(delta.dcol().to_bits(), fused_col.to_bits());
        let rounded_row_row = model.inverse_row_row * rhs_row;
        assert_eq!(
            delta.drow().to_bits(),
            model
                .inverse_col_row
                .mul_add(rhs_col, rounded_row_row)
                .to_bits()
        );
    }

    fn assert_public_solver_direction<D: PisDirection>() {
        let level = Level::Two;
        let pixels = level.pixels();
        let input = Input::<D>::from_native_order(
            level,
            LensPair {
                a: vec![42; pixels],
                b: vec![42; pixels],
            },
            LensPair {
                a: vec![255; pixels],
                b: vec![255; pixels],
            },
            vec![1.0; pixels],
            vec![2.0; pixels],
            vec![1.0; pixels],
            vec![CostMode::Weighted; level.patch_rows()],
        )
        .unwrap();
        let initial = InitialGrid::<D>::coarse_zeros();
        let hint =
            HintGrid::<D>::from_row_major(level, vec![flow(1.0, 1.0); level.patches()]).unwrap();
        assert_eq!(input.direction(), D::DIRECTION);
        assert_eq!(initial.direction(), D::DIRECTION);
        assert_eq!(hint.direction(), D::DIRECTION);
        let patches = solve(&input, initial, Some(&hint)).unwrap();

        assert_eq!(patches.direction(), D::DIRECTION);
        assert_eq!(patches.patches().len(), 264);
        let first = patches.patch(0, 0);
        assert_eq!(first.flow(), Flow::ZERO);
        for pass in first.passes() {
            assert_eq!(pass.winner(), Candidate::Current);
            assert!(pass.descent_admitted());
            assert_eq!(pass.descent_iterations(), 2);
            assert!(pass.stopped_on_no_improvement());
            assert!(!pass.guarded());
        }
    }

    #[test]
    fn empty_override_cadence_can_skip_descent_without_skipping_candidates() {
        assert_eq!(
            DescentAdmission::from_empty_override_cadence(6_370, 10),
            DescentAdmission::EveryPatch,
        );
        assert_eq!(
            DescentAdmission::from_empty_override_cadence(6_372, 10),
            DescentAdmission::NoPatches,
        );

        let level = Level::Two;
        let pixels = level.pixels();
        let input = constant_input::<AtoB>(
            10,
            30,
            vec![255; pixels],
            vec![255; pixels],
            vec![1.0; pixels],
        );
        let hint =
            HintGrid::<AtoB>::from_row_major(level, vec![flow(1.0, 1.0); level.patches()]).unwrap();
        let patches = solve_with_descent_admission(
            &input,
            InitialGrid::coarse_zeros(),
            Some(&hint),
            DescentAdmission::NoPatches,
        )
        .unwrap();

        for patch in patches.patches() {
            for pass in patch.passes() {
                assert!(pass.candidates()[Candidate::Current.index()].is_some());
                assert!(pass.candidates()[Candidate::Hint.index()].is_some());
                assert!(!pass.descent_admitted());
                assert_eq!(pass.descent_iterations(), 0);
                assert!(!pass.stopped_on_no_improvement());
            }
        }
    }

    #[test]
    fn public_solver_runs_both_directions_and_current_wins_hint_ties() {
        assert_public_solver_direction::<AtoB>();
        assert_public_solver_direction::<BtoA>();
    }

    #[derive(Default)]
    struct FlowScoreKernel;

    impl Kernel for FlowScoreKernel {
        fn score(&mut self, _pass: usize, _patch: usize, flow: Flow) -> Score {
            Score {
                value: flow.dcol(),
                survivors: 64,
            }
        }

        fn descent(&mut self, _pass: usize, _patch: usize, _flow: Flow) -> Option<DescentStep> {
            None
        }
    }

    #[test]
    fn passes_are_in_place_and_candidate_order_keeps_strict_ties() {
        let initial = [
            flow(4.0, 0.0),
            flow(5.0, 0.0),
            flow(6.0, 0.0),
            flow(7.0, 0.0),
        ];
        let hint = [
            flow(3.0, 0.0),
            flow(2.0, 0.0),
            flow(1.0, 0.0),
            flow(0.0, 0.0),
        ];
        let patches = schedule(
            2,
            2,
            &initial,
            Some(&hint),
            None,
            DESCENTS_PER_PASS,
            &mut FlowScoreKernel,
        );

        assert_eq!(patches[0].passes[0].winner, Candidate::Hint);
        assert_eq!(patches[1].passes[0].winner, Candidate::Hint);
        assert_eq!(patches[2].passes[0].winner, Candidate::Hint);
        assert_eq!(patches[3].passes[0].winner, Candidate::Hint);
        assert_eq!(patches[3].passes[1].winner, Candidate::Current);
        assert_eq!(patches[2].passes[1].winner, Candidate::Horizontal);
        assert_eq!(patches[1].passes[1].winner, Candidate::Vertical);
        // At patch zero, right and bottom both carry zero. Horizontal is checked
        // first and strict `<` prevents vertical from replacing its tie.
        assert_eq!(patches[0].passes[1].winner, Candidate::Horizontal);
        assert!(patches.iter().all(|patch| patch.flow == Flow::ZERO));

        let patch_three_forward = patches[3].passes[0].candidates;
        assert_eq!(
            patch_three_forward[Candidate::Horizontal.index()]
                .unwrap()
                .flow,
            flow(1.0, 0.0),
        );
        assert_eq!(
            patch_three_forward[Candidate::Vertical.index()]
                .unwrap()
                .flow,
            flow(2.0, 0.0),
        );
    }

    struct SentinelKernel;

    impl Kernel for SentinelKernel {
        fn score(&mut self, _pass: usize, patch: usize, flow: Flow) -> Score {
            let finite_neighbour = patch == 1 && flow == Flow::new(7.0, 0.0).unwrap();
            Score {
                value: if finite_neighbour {
                    0.0
                } else {
                    SENTINEL_SCORE
                },
                survivors: usize::from(finite_neighbour) * 9,
            }
        }

        fn descent(&mut self, _pass: usize, _patch: usize, _flow: Flow) -> Option<DescentStep> {
            None
        }
    }

    #[test]
    fn sentinel_nodes_stay_in_the_grid_and_propagate() {
        let patches = schedule(
            1,
            2,
            &[flow(7.0, 0.0), flow(8.0, 0.0)],
            None,
            None,
            DESCENTS_PER_PASS,
            &mut SentinelKernel,
        );
        assert_eq!(
            patches[0].passes[0].candidates[0].unwrap().score.value,
            SENTINEL_SCORE
        );
        assert_eq!(patches[0].flow, flow(7.0, 0.0));
        assert_eq!(patches[1].passes[0].winner, Candidate::Horizontal);
        assert_eq!(patches[1].flow, flow(7.0, 0.0));
    }

    struct SecondPassSentinelKernel;

    impl Kernel for SecondPassSentinelKernel {
        fn score(&mut self, pass: usize, _patch: usize, _flow: Flow) -> Score {
            Score {
                value: if pass == 0 { 7.0 } else { SENTINEL_SCORE },
                survivors: usize::from(pass == 0) * 9,
            }
        }

        fn descent(&mut self, _pass: usize, _patch: usize, _flow: Flow) -> Option<DescentStep> {
            None
        }
    }

    #[test]
    fn all_sentinel_visit_preserves_the_prior_finite_residual() {
        let patches = schedule(
            1,
            1,
            &[Flow::ZERO],
            None,
            None,
            DESCENTS_PER_PASS,
            &mut SecondPassSentinelKernel,
        );

        assert_eq!(patches[0].residual, 7.0);
        assert!(
            patches[0].passes[1].candidates[0]
                .unwrap()
                .score
                .is_sentinel()
        );
    }

    #[test]
    fn disparity_interval_is_strict_and_order_agnostic() {
        // Section 120J: `c > a && c < b`, or the same with the endpoints reversed.
        let forward = DisparityInterval::new([-2.0, -1.0], [4.0, 1.0]);
        let reversed = DisparityInterval::new([4.0, 1.0], [-2.0, -1.0]);
        for interval in [forward, reversed] {
            assert!(interval.admits(flow(0.0, 0.0)));
            assert!(interval.admits(flow(3.9, 0.9)));
            assert!(interval.admits(flow(-1.9, -0.9)));
            // Strict: an endpoint is outside.
            assert!(!interval.admits(flow(4.0, 0.0)));
            assert!(!interval.admits(flow(-2.0, 0.0)));
            assert!(!interval.admits(flow(0.0, 1.0)));
            assert!(!interval.admits(flow(0.0, -1.0)));
            // Either component alone is enough to reject.
            assert!(!interval.admits(flow(5.0, 0.0)));
            assert!(!interval.admits(flow(0.0, 2.0)));
        }
        for probe in [
            flow(0.0, 0.0),
            flow(4.0, 0.0),
            flow(-2.0, 0.0),
            flow(3.9, -0.9),
            flow(9.0, 9.0),
        ] {
            assert_eq!(forward.admits(probe), reversed.admits(probe));
        }
    }

    struct OutsideHintKernel;

    impl Kernel for OutsideHintKernel {
        fn score(&mut self, _pass: usize, _patch: usize, flow: Flow) -> Score {
            Score {
                value: -flow.dcol(),
                survivors: 64,
            }
        }

        fn descent(&mut self, _pass: usize, _patch: usize, current: Flow) -> Option<DescentStep> {
            (current == flow(2.0, 0.0)).then_some(DescentStep {
                delta: flow(1.5, 0.0),
                residual: 1.0,
                survivors: 64,
            })
        }
    }

    #[test]
    fn interval_scores_an_outside_hint_then_accepts_its_refined_flow() {
        let patches = schedule(
            1,
            1,
            &[Flow::ZERO],
            Some(&[flow(2.0, 0.0)]),
            Some(DisparityInterval::new([-1.0, -1.0], [1.0, 1.0])),
            DESCENTS_PER_PASS,
            &mut OutsideHintKernel,
        );

        for pass in patches[0].passes {
            assert!(pass.candidates[Candidate::Current.index()].is_some());
            assert!(pass.candidates[Candidate::Hint.index()].is_some());
            assert_eq!(pass.winner, Candidate::Hint);
            assert!(!pass.guarded);
        }
        assert_eq!(patches[0].flow, flow(0.5, 0.0));
    }

    struct PropagatedSeedGuardKernel;

    impl Kernel for PropagatedSeedGuardKernel {
        fn score(&mut self, pass: usize, _patch: usize, flow: Flow) -> Score {
            Score {
                value: if pass == 0 {
                    flow.dcol()
                } else {
                    -flow.dcol().abs()
                },
                survivors: 64,
            }
        }

        fn descent(&mut self, pass: usize, _patch: usize, current: Flow) -> Option<DescentStep> {
            (pass == 0 && current == flow(-7.0, 0.0)).then_some(DescentStep {
                delta: flow(1.5, 0.0),
                residual: 1.0,
                survivors: 64,
            })
        }
    }

    #[test]
    fn distance_guard_is_relative_to_and_retains_the_selected_seed() {
        let patches = schedule(
            1,
            1,
            &[Flow::ZERO],
            Some(&[flow(-7.0, 0.0)]),
            None,
            DESCENTS_PER_PASS,
            &mut PropagatedSeedGuardKernel,
        );

        assert_eq!(patches[0].passes[0].winner, Candidate::Hint);
        assert!(!patches[0].passes[0].guarded);
        assert_eq!(patches[0].flow, flow(-8.5, 0.0));
    }

    #[derive(Default)]
    struct StepBeforeStopKernel {
        calls: usize,
    }

    impl Kernel for StepBeforeStopKernel {
        fn score(&mut self, _pass: usize, _patch: usize, _flow: Flow) -> Score {
            Score {
                value: 100.0,
                survivors: 9,
            }
        }

        fn descent(&mut self, pass: usize, _patch: usize, _flow: Flow) -> Option<DescentStep> {
            if pass != 1 || self.calls >= 2 {
                return None;
            }
            let step = if self.calls == 0 {
                DescentStep {
                    delta: flow(1.0, 0.0),
                    residual: 10.0,
                    survivors: 1,
                }
            } else {
                DescentStep {
                    delta: flow(2.0, 0.0),
                    residual: 11.0,
                    survivors: 1,
                }
            };
            self.calls += 1;
            Some(step)
        }
    }

    #[test]
    fn descent_writes_the_nonimproving_step_but_keeps_the_prior_residual() {
        let patches = schedule(
            1,
            1,
            &[Flow::ZERO],
            None,
            None,
            DESCENTS_PER_PASS,
            &mut StepBeforeStopKernel::default(),
        );
        let report = patches[0].passes[1];
        assert_eq!(report.descent_iterations, 2);
        assert!(report.stopped_on_no_improvement);
        assert!(!report.guarded);
        assert_eq!(patches[0].flow, flow(-3.0, 0.0));
        // The -3 flow proves the failed second step was written. The residual
        // stays at the first step's last-improving value.
        assert_eq!(patches[0].residual, 10.0);
    }

    struct GuardKernel {
        delta: Flow,
        used: bool,
    }

    impl Kernel for GuardKernel {
        fn score(&mut self, _pass: usize, _patch: usize, _flow: Flow) -> Score {
            Score {
                value: 100.0,
                survivors: 9,
            }
        }

        fn descent(&mut self, pass: usize, _patch: usize, _flow: Flow) -> Option<DescentStep> {
            if pass != 1 || self.used {
                return None;
            }
            self.used = true;
            Some(DescentStep {
                delta: self.delta,
                residual: 7.0,
                survivors: 1,
            })
        }
    }

    #[test]
    fn guard_is_strict_reverts_flow_and_keeps_the_descent_residual() {
        let at_limit = schedule(
            1,
            1,
            &[Flow::ZERO],
            None,
            None,
            DESCENTS_PER_PASS,
            &mut GuardKernel {
                delta: flow(8.0, 0.0),
                used: false,
            },
        );
        assert!(!at_limit[0].passes[1].guarded);
        assert_eq!(at_limit[0].flow, flow(-8.0, 0.0));
        assert_eq!(at_limit[0].residual, 7.0);

        let beyond_limit = schedule(
            1,
            1,
            &[Flow::ZERO],
            None,
            None,
            DESCENTS_PER_PASS,
            &mut GuardKernel {
                delta: flow(8.25, 0.0),
                used: false,
            },
        );
        assert!(beyond_limit[0].passes[1].guarded);
        assert_eq!(beyond_limit[0].flow, Flow::ZERO);
        assert_eq!(beyond_limit[0].residual, 7.0);

        let f64_only = schedule(
            1,
            1,
            &[Flow::ZERO],
            None,
            None,
            DESCENTS_PER_PASS,
            &mut GuardKernel {
                delta: flow(8.0, 0.001),
                used: false,
            },
        );
        assert!(f64_only[0].passes[1].guarded);
        assert_eq!(f64_only[0].flow, Flow::ZERO);
    }

    #[test]
    fn typed_patch_grids_reject_other_levels_and_lengths() {
        let error =
            HintGrid::<AtoB>::from_row_major(Level::One, vec![Flow::ZERO; 264]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "ONE X2 PIS level 1 has 264 hint patch values, expected 1424",
        );
        assert_eq!(InitialGrid::<AtoB>::coarse_zeros().flows().len(), 264);
        assert_eq!(
            InitialGrid::<AtoB>::coarse_from_row_major(vec![Flow::ZERO; 264])
                .unwrap()
                .level(),
            Level::Two,
        );
        assert_eq!(
            InitialGrid::<AtoB>::from_test_row_major(
                Level::One,
                vec![Flow::ZERO; Level::One.patches()],
            )
            .unwrap()
            .level(),
            Level::One,
        );

        let level = Level::Two;
        let pixels = level.pixels();
        let error = Input::<AtoB>::from_native_order(
            level,
            LensPair {
                a: vec![0; pixels],
                b: vec![0; pixels],
            },
            LensPair {
                a: vec![255; pixels],
                b: vec![255; pixels],
            },
            vec![0.0; pixels],
            vec![0.0; pixels],
            vec![1.0; pixels],
            vec![CostMode::Unweighted; level.patch_rows() - 1],
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ONE X2 PIS level 2 has 87 patch-row cost mode values, expected 88",
        );

        let dcol = vec![1.0; Level::One.patches()];
        let drow = vec![2.0; Level::One.patches()];
        let token =
            PatchGrid::<AtoB>::from_row_major_components(Level::One, dcol.clone(), drow.clone())
                .unwrap();
        assert_eq!(token.direction(), Direction::AtoB);
        let (token_level, token_dcol, token_drow) = token.into_row_major_components();
        assert_eq!(token_level, Level::One);
        assert_eq!(token_dcol, dcol);
        assert_eq!(token_drow, drow);
    }
}
