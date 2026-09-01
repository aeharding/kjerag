//! Selected ONE X2 retained-public-flow update after the optional variational stage.
//!
//! Native runs this stage after variational refinement and its whole-level
//! rollback, but before each linear resize. Selected `FDS+0xa5=1` reads the
//! per-direction retained public-flow pyramids at `+0x2a8/+0x2c0`. Those
//! pyramids are built directly from the incoming 1080-by-60 public field at
//! every level, with linear resize followed by division by `2^level`.
//!
//! This is deliberately not the auxiliary `calcHintFlow` family at
//! `+0x278/+0x290`. That calculation is invoked and seeds at finest L1, with
//! an internal coarser-index tail; it has no type or entry point in this
//! module, so it cannot be submitted to the selected blend by mistake.
//!
//! The motion boundary recursively area-reduces native's level-zero
//! `calcMotion` result. That base can contain both 1 and 255, and each exact
//! U8 `INTER_AREA` reduction can produce further byte values. The update tests
//! only exact zero versus any nonzero byte.
//!
//! The cold/warm production lineage uses this readable stage. The recursive U8
//! reductions, selected retained-public float resize and retained-blend
//! arithmetic reproduce their authenticated target boundaries bit for bit.
//! The float claim is deliberately limited to the selected 1080-by-60 direct
//! L1/L2 topology and its observed OpenCV arm64 operation order.
//!
//! Cold/warm epoch selection remains private to the ONE X2 module family. The
//! production pair-level estimator owns reference-history provenance; external
//! callers cannot invoke its private entries:
//!
//! ```compile_fail
//! use kjerag_render::flow::one_xs::post_update::preserve_without_retained;
//! ```
//!
//! ```compile_fail
//! use kjerag_render::flow::one_xs::post_update::update_with_retained;
//! ```

#![allow(
    dead_code,
    reason = "the module retains sealed oracle entry points beside its production path"
)]

use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

use super::dense::{DenseField, PublicDenseField};
use super::pis::{Level, PisDirection};
use super::temporal::MotionMask;
use super::variational::Refinement;
use super::{COLS, Direction, ROWS};

/// Exact binary32 weight selected for a fresh field at a zero motion byte.
pub const NON_MOTION_FRESH_WEIGHT: f32 = f32::from_bits(0x3ca3_d70a);

/// Incoming retained public flow prepared for both selected solver levels.
///
/// Construction consumes the prior public field. This models native's
/// per-direction `InputOutputArray`: the old public value is first split and
/// resized into `+0x2a8/+0x2c0`, then the caller-owned public Mat is replaced
/// by the current result.
#[derive(Debug)]
pub struct RetainedPublicPyramids<D: PisDirection> {
    level_one: RetainedLevel<D>,
    level_two: RetainedLevel<D>,
}

impl<D: PisDirection> RetainedPublicPyramids<D> {
    /// Build L1 and L2 directly from one incoming 1080-by-60 public field.
    ///
    /// L1 is a direct linear resize to 540 by 30 followed by multiplication
    /// by one half. L2 is independently resized from the same public source
    /// to 270 by 15 and multiplied by one quarter; it is not recursively
    /// derived from L1.
    pub fn from_public(previous: PublicDenseField<D>) -> Self {
        Self::from_public_ref(&previous)
    }

    /// Build both retained levels without consuming the prior public field.
    ///
    /// A fallible staged solver uses this form so the exact incoming owner can
    /// be returned if either sparse level fails. The completed transaction
    /// still replaces that owner atomically with its newly computed field.
    pub(super) fn from_public_ref(previous: &PublicDenseField<D>) -> Self {
        Self {
            level_one: RetainedLevel::from_public(Level::One, previous.dcol(), previous.drow()),
            level_two: RetainedLevel::from_public(Level::Two, previous.dcol(), previous.drow()),
        }
    }

    pub const fn direction(&self) -> Direction {
        D::DIRECTION
    }

    fn level(&self, level: Level) -> &RetainedLevel<D> {
        match level {
            Level::One => &self.level_one,
            Level::Two => &self.level_two,
        }
    }
}

/// Owned selected-level reductions of one 1080-by-60 motion mask.
///
/// Native constructs L1 by halving the base in each dimension, then constructs
/// L2 by halving L1. Each reduction is exact integer-ratio U8 `INTER_AREA`:
/// the four source bytes are accumulated in `u16` and rounded as `(sum + 2) / 4`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MotionPyramid {
    level_one: Box<[u8]>,
    level_two: Box<[u8]>,
}

impl MotionPyramid {
    pub fn from_base(base: &MotionMask) -> Self {
        Self::from_base_bytes(base.bytes())
    }

    /// Produce one linear, direction-labelled token for a selected update.
    pub fn level<D: PisDirection>(&self, level: Level) -> MotionLevel<D> {
        let bytes = match level {
            Level::One => self.level_one.clone(),
            Level::Two => self.level_two.clone(),
        };
        MotionLevel {
            level,
            bytes,
            direction: PhantomData,
        }
    }

    pub(super) fn from_base_bytes(base: &[u8]) -> Self {
        assert_eq!(
            base.len(),
            ROWS * COLS,
            "ONE X2 base motion mask must have the retained-grid shape",
        );
        let level_one = area_half_u8(base, ROWS, COLS);
        let level_two = area_half_u8(&level_one, Level::One.rows(), Level::One.cols());
        Self {
            level_one: level_one.into_boxed_slice(),
            level_two: level_two.into_boxed_slice(),
        }
    }
}

fn area_half_u8(source: &[u8], source_rows: usize, source_cols: usize) -> Vec<u8> {
    debug_assert_eq!(source.len(), source_rows * source_cols);
    debug_assert_eq!(source_rows % 2, 0);
    debug_assert_eq!(source_cols % 2, 0);

    let target_rows = source_rows / 2;
    let target_cols = source_cols / 2;
    let mut target = vec![0; target_rows * target_cols];
    for row in 0..target_rows {
        let top = 2 * row * source_cols;
        let bottom = top + source_cols;
        for col in 0..target_cols {
            let left = 2 * col;
            let sum = u16::from(source[top + left])
                + u16::from(source[top + left + 1])
                + u16::from(source[bottom + left])
                + u16::from(source[bottom + left + 1]);
            target[row * target_cols + col] = ((sum + 2) / 4) as u8;
        }
    }
    target
}

/// One exact selected-level motion-mask payload.
///
/// Values are intentionally retained as bytes rather than normalized to a
/// boolean. Native accepts every byte value and tests only `byte == 0`.
/// This token is non-`Clone` and is consumed by one level update.
#[derive(Debug)]
pub struct MotionLevel<D: PisDirection> {
    level: Level,
    bytes: Box<[u8]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> MotionLevel<D> {
    /// Admit one post-`INTER_AREA` mask with the exact selected-level shape.
    pub fn from_bytes(level: Level, bytes: Vec<u8>) -> Result<Self, ShapeError> {
        let expected = level.pixels();
        if bytes.len() != expected {
            return Err(ShapeError {
                direction: D::DIRECTION,
                level,
                part: "motion-mask bytes",
                expected,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            level,
            bytes: bytes.into_boxed_slice(),
            direction: PhantomData,
        })
    }

    pub const fn level(&self) -> Level {
        self.level
    }

    pub const fn direction(&self) -> Direction {
        D::DIRECTION
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug)]
struct RetainedLevel<D: PisDirection> {
    dcol: Box<[f32]>,
    drow: Box<[f32]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> RetainedLevel<D> {
    fn from_public(level: Level, dcol: &[f32], drow: &[f32]) -> Self {
        Self {
            dcol: resize_retained_public(dcol, level),
            drow: resize_retained_public(drow, level),
            direction: PhantomData,
        }
    }
}

/// Reproduce the selected OpenCV float resize and following unit conversion.
///
/// This is intentionally only the captured 1080-by-60 retained-public
/// topology. Both levels come directly from that public field. Native resizes
/// an interleaved `CV_32FC2` Mat; at L1 OpenCV's exact-two reduction processes
/// the first 56 of 60 interleaved destination lanes through its SIMD body.
/// Those lanes are the first 28 vector columns in each separated component
/// below. The final two columns use a scalar remainder with a different
/// association. L2 uses the pairwise linear body over its complete width.
/// Every source footprint is interior, so no border rule is exercised here.
fn resize_retained_public(source: &[f32], level: Level) -> Box<[f32]> {
    assert_eq!(
        source.len(),
        ROWS * COLS,
        "ONE X2 retained public plane must have the public-grid shape",
    );

    let mut target = Vec::with_capacity(level.pixels());
    match level {
        Level::One => {
            const SIMD_COLUMNS: usize = 28;
            for row in 0..Level::One.rows() {
                let top = 2 * row * COLS;
                let bottom = top + COLS;
                for col in 0..Level::One.cols() {
                    let left = 2 * col;
                    let top_left = source[top + left];
                    let top_right = source[top + left + 1];
                    let bottom_left = source[bottom + left];
                    let bottom_right = source[bottom + left + 1];

                    let resized = if col < SIMD_COLUMNS {
                        linear_half_pair_association(top_left, top_right, bottom_left, bottom_right)
                    } else {
                        linear_half_scalar_tail(top_left, top_right, bottom_left, bottom_right)
                    };
                    target.push(resized * 0.5);
                }
            }
        }
        Level::Two => {
            for row in 0..Level::Two.rows() {
                let top = (4 * row + 1) * COLS;
                let bottom = top + COLS;
                for col in 0..Level::Two.cols() {
                    let left = 4 * col + 1;
                    let resized = linear_half_pair_association(
                        source[top + left],
                        source[top + left + 1],
                        source[bottom + left],
                        source[bottom + left + 1],
                    );
                    target.push(resized * 0.25);
                }
            }
        }
    }
    target.into_boxed_slice()
}

fn linear_half_pair_association(
    top_left: f32,
    top_right: f32,
    bottom_left: f32,
    bottom_right: f32,
) -> f32 {
    let top_sum = top_left + top_right;
    let bottom_sum = bottom_left + bottom_right;
    let top = top_sum * 0.5;
    let bottom = bottom_sum * 0.5;
    (top + bottom) * 0.5
}

fn linear_half_scalar_tail(
    top_left: f32,
    top_right: f32,
    bottom_left: f32,
    bottom_right: f32,
) -> f32 {
    let sum = top_left + top_right;
    let sum = sum + bottom_left;
    let sum = sum + bottom_right;
    sum * 0.25
}

/// What happened at the optional variational stage before this update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariationalOutcome {
    /// The selected configuration sets the iteration count to zero.
    Disabled,
    /// Refinement ran and its whole-level guard accepted the result.
    Kept,
    /// Refinement ran and its whole-level guard restored the dense input.
    RolledBack,
}

/// One consumed dense field after the selected retained-public update.
///
/// Fields are private and this type has no `Clone`. The dense resize/seed
/// adapters consume this token, making a direct resize of [`DenseField`], or
/// a resize of [`Refinement`], ill-typed.
#[must_use = "the post-update field has not been resized or submitted as a seed"]
#[derive(Debug)]
pub struct PostUpdateDense<D: PisDirection> {
    field: DenseField<D>,
    variational: VariationalOutcome,
    retained_public_applied: bool,
}

impl<D: PisDirection> PostUpdateDense<D> {
    pub const fn level(&self) -> Level {
        self.field.level()
    }

    pub const fn direction(&self) -> Direction {
        D::DIRECTION
    }

    /// Whether native's preceding whole-level variational guard restored both
    /// component planes before this stage ran.
    pub const fn variational_rolled_back(&self) -> bool {
        matches!(self.variational, VariationalOutcome::RolledBack)
    }

    /// Whether the optional variational stage was disabled, kept or rolled back.
    pub const fn variational_outcome(&self) -> VariationalOutcome {
        self.variational
    }

    /// Whether this token passed through the warm retained-public blend.
    pub const fn retained_public_applied(&self) -> bool {
        self.retained_public_applied
    }

    /// Inspect the updated components without recovering a resizable token.
    pub const fn field(&self) -> &DenseField<D> {
        &self.field
    }

    pub(super) fn into_field(self) -> DenseField<D> {
        self.field
    }

    #[cfg(test)]
    pub(crate) fn from_test_field(field: DenseField<D>, variational_rolled_back: bool) -> Self {
        Self {
            field,
            variational: if variational_rolled_back {
                VariationalOutcome::RolledBack
            } else {
                VariationalOutcome::Kept
            },
            retained_public_applied: false,
        }
    }
}

/// A selected-level plane has the wrong number of values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapeError {
    direction: Direction,
    level: Level,
    part: &'static str,
    expected: usize,
    actual: usize,
}

impl fmt::Display for ShapeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "ONE X2 {} {} has {} {}, expected {}",
            self.direction, self.level, self.actual, self.part, self.expected,
        )
    }
}

impl Error for ShapeError {}

/// The refinement and motion token name different selected levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WrongLevel {
    direction: Direction,
    field: Level,
    motion: Level,
}

impl fmt::Display for WrongLevel {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "ONE X2 {} post-update field is {}, but its motion mask is {}",
            self.direction, self.field, self.motion,
        )
    }
}

impl Error for WrongLevel {}

/// Preserve native's cold/no-retained behavior.
///
/// Selected `calcWithMotion` routes an empty reference history through cold
/// `calc`, with no retained-public/motion update. This consumes the refinement
/// and returns the same field bits while retaining the preceding whole-level
/// rollback provenance.
pub(super) fn preserve_without_retained<D: PisDirection>(
    refinement: Refinement<D>,
) -> PostUpdateDense<D> {
    let (field, variational_rolled_back) = refinement.into_parts();
    PostUpdateDense {
        field,
        variational: if variational_rolled_back {
            VariationalOutcome::RolledBack
        } else {
            VariationalOutcome::Kept
        },
        retained_public_applied: false,
    }
}

/// Preserve the selected cold route when variational iterations are disabled.
///
/// Sections 150 and 151 close the selected `+0x20` value as zero, so the
/// derivative and variational bodies do not execute at either active level.
/// This consumes the dense field directly and records that absence rather
/// than manufacturing a refinement or calling the stage a rollback.
pub(super) fn preserve_without_variational_or_retained<D: PisDirection>(
    field: DenseField<D>,
) -> PostUpdateDense<D> {
    PostUpdateDense {
        field,
        variational: VariationalOutcome::Disabled,
        retained_public_applied: false,
    }
}

/// Apply the selected warm retained-public-flow blend to one refined level.
///
/// For each component and pixel, native uses exact binary32
/// `w = byte == 0 ? 0.02 : 1.0` and computes
/// `fresh*w + retained_public*(1-w)`. The retained multiplication is evaluated
/// even when `w == 1`, so a retained NaN or infinity can contaminate a motion
/// pixel through multiplication by zero. There is no post-update validity
/// scan.
///
pub(super) fn update_with_retained<D: PisDirection>(
    refinement: Refinement<D>,
    retained: &RetainedPublicPyramids<D>,
    motion: MotionLevel<D>,
) -> Result<PostUpdateDense<D>, WrongLevel> {
    let (field, variational_rolled_back) = refinement.into_parts();
    let outcome = if variational_rolled_back {
        VariationalOutcome::RolledBack
    } else {
        VariationalOutcome::Kept
    };
    update_field_with_retained(field, outcome, retained, motion)
}

/// Apply the selected warm retained-public-flow blend when variational is disabled.
///
/// This is the selected configuration at both active levels. Consuming the
/// dense field directly preserves truthful [`VariationalOutcome::Disabled`]
/// provenance while performing the same retained blend as the optional-VR path.
pub(super) fn update_without_variational_with_retained<D: PisDirection>(
    field: DenseField<D>,
    retained: &RetainedPublicPyramids<D>,
    motion: MotionLevel<D>,
) -> Result<PostUpdateDense<D>, WrongLevel> {
    update_field_with_retained(field, VariationalOutcome::Disabled, retained, motion)
}

fn update_field_with_retained<D: PisDirection>(
    mut field: DenseField<D>,
    variational: VariationalOutcome,
    retained: &RetainedPublicPyramids<D>,
    motion: MotionLevel<D>,
) -> Result<PostUpdateDense<D>, WrongLevel> {
    let field_level = field.level();
    if field_level != motion.level {
        return Err(WrongLevel {
            direction: D::DIRECTION,
            field: field_level,
            motion: motion.level,
        });
    }

    let retained = retained.level(field_level);
    debug_assert_eq!(retained.dcol.len(), field_level.pixels());
    debug_assert_eq!(retained.drow.len(), field_level.pixels());
    debug_assert_eq!(motion.bytes.len(), field_level.pixels());

    let (fresh_dcol, fresh_drow) = field.components_mut();
    for pixel in 0..field_level.pixels() {
        let weight = if motion.bytes[pixel] == 0 {
            NON_MOTION_FRESH_WEIGHT
        } else {
            1.0
        };
        let retained_weight = 1.0 - weight;

        let retained_dcol = retained.dcol[pixel] * retained_weight;
        fresh_dcol[pixel] = fresh_dcol[pixel].mul_add(weight, retained_dcol);

        let retained_drow = retained.drow[pixel] * retained_weight;
        fresh_drow[pixel] = fresh_drow[pixel].mul_add(weight, retained_drow);
    }

    Ok(PostUpdateDense {
        field,
        variational,
        retained_public_applied: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::one_xs::dense::DenseField;
    use crate::flow::one_xs::pis::{AtoB, BtoA};

    fn refinement<D: PisDirection>(
        level: Level,
        dcol: Vec<f32>,
        drow: Vec<f32>,
        rolled_back: bool,
    ) -> Refinement<D> {
        let field = DenseField::from_test_components(level, dcol, drow).unwrap();
        Refinement::from_test_field(field, rolled_back)
    }

    fn constant_public<D: PisDirection>(dcol: f32, drow: f32) -> PublicDenseField<D> {
        PublicDenseField::from_test_components(vec![dcol; ROWS * COLS], vec![drow; ROWS * COLS])
    }

    fn assert_close(actual: f32, expected: f32) {
        let tolerance = 2.0e-6 * expected.abs().max(1.0);
        assert!(
            (actual - expected).abs() <= tolerance,
            "actual {actual:?}, expected {expected:?}, tolerance {tolerance:?}",
        );
    }

    #[test]
    fn retained_pyramids_are_direct_public_resizes_in_internal_units() {
        let dcol = (0..ROWS)
            .flat_map(|row| (0..COLS).map(move |col| (row * 100 + col) as f32))
            .collect::<Vec<_>>();
        let drow = dcol.iter().map(|value| -*value).collect::<Vec<_>>();
        let previous = PublicDenseField::<AtoB>::from_test_components(dcol, drow);
        let pyramid = RetainedPublicPyramids::from_public(previous);

        assert_eq!(pyramid.direction(), Direction::AtoB);
        assert_eq!(pyramid.level_one.dcol.len(), Level::One.pixels());
        assert_eq!(pyramid.level_two.dcol.len(), Level::Two.pixels());

        // L1 target (0,0) samples the centre of public pixels (0,0)..(1,1),
        // then converts public units to L1 units with a factor of one half.
        assert_close(pyramid.level_one.dcol[0], 50.5 * 0.5);
        assert_close(pyramid.level_one.drow[0], -50.5 * 0.5);

        // L2 is resized directly from public. Its first centre is (1.5,1.5),
        // so only public pixels (1,1)..(2,2) contribute before the /4.
        assert_close(pyramid.level_two.dcol[0], 151.5 * 0.25);
        assert_close(pyramid.level_two.drow[0], -151.5 * 0.25);
    }

    #[test]
    fn nonlinear_public_field_proves_l2_is_not_recursively_resized() {
        let mut dcol = vec![0.0; ROWS * COLS];
        dcol[0] = 64.0;
        let recursive_l1 = super::super::dense::resize_linear_scaled(
            &dcol,
            ROWS,
            COLS,
            Level::One.rows(),
            Level::One.cols(),
            0.5,
        );
        let recursive_l2 = super::super::dense::resize_linear_scaled(
            &recursive_l1,
            Level::One.rows(),
            Level::One.cols(),
            Level::Two.rows(),
            Level::Two.cols(),
            0.5,
        );

        let previous = PublicDenseField::<AtoB>::from_test_components(dcol, vec![0.0; ROWS * COLS]);
        let pyramid = RetainedPublicPyramids::from_public(previous);

        // The direct 4x resize samples public coordinate (1.5, 1.5), so the
        // corner impulse is outside its footprint. Two recursive 2x resizes
        // would spread that impulse into the first L2 sample instead.
        assert_eq!(pyramid.level_two.dcol[0], 0.0);
        assert_eq!(recursive_l2[0], 1.0);
        assert_ne!(pyramid.level_two.dcol[0], recursive_l2[0]);
    }

    #[test]
    fn retained_resize_preserves_opencv_association_and_l1_tail() {
        let operands = [0x3eff_4786, 0x3eff_6895, 0x3f00_3890, 0x3f00_48ff].map(f32::from_bits);
        let mut public = vec![0.0; ROWS * COLS];
        let l1_row = 10;

        for col in [0, 28] {
            let left = 2 * col;
            let top = 2 * l1_row * COLS;
            public[top + left] = operands[0];
            public[top + left + 1] = operands[1];
            public[top + COLS + left] = operands[2];
            public[top + COLS + left + 1] = operands[3];
        }
        // L2 output (0,0) samples public rows/columns 1 and 2 directly.
        public[COLS + 1] = operands[0];
        public[COLS + 2] = operands[1];
        public[2 * COLS + 1] = operands[2];
        public[2 * COLS + 2] = operands[3];

        let l1 = resize_retained_public(&public, Level::One);
        let l2 = resize_retained_public(&public, Level::Two);

        assert_eq!(l1[l1_row * Level::One.cols()].to_bits(), 0x3e7f_eccf);
        assert_eq!(l1[l1_row * Level::One.cols() + 28].to_bits(), 0x3e7f_ecce);
        assert_eq!(l2[0].to_bits(), 0x3dff_eccf);
    }

    #[test]
    fn motion_level_accepts_every_byte_but_requires_exact_shape() {
        let mut bytes = vec![0; Level::Two.pixels()];
        bytes[0] = 1;
        bytes[1] = 63;
        bytes[2] = 255;
        let motion = MotionLevel::<BtoA>::from_bytes(Level::Two, bytes).unwrap();
        assert_eq!(motion.level(), Level::Two);
        assert_eq!(motion.direction(), Direction::BtoA);

        let error = MotionLevel::<AtoB>::from_bytes(Level::One, vec![0; Level::One.pixels() - 1])
            .unwrap_err();
        assert_eq!(error.expected, Level::One.pixels());
        assert_eq!(error.actual, Level::One.pixels() - 1);
    }

    #[test]
    fn area_half_uses_exact_u8_rounding_with_u16_accumulation() {
        let source = [
            0, 0, 0, 1, 0, 2, 1, 2, 255, 255, // first source row
            0, 1, 0, 1, 1, 2, 1, 2, 255, 255, // second source row
        ];

        assert_eq!(area_half_u8(&source, 2, 10), [0, 1, 1, 2, 255]);
    }

    #[test]
    fn motion_pyramid_is_recursive_with_typed_directional_levels() {
        let mut base = vec![0; ROWS * COLS];
        // Three of the first four 2x2 cells have sum two. Each rounds to one
        // at L1, whose 2x2 sum three then rounds to one at L2. A direct 4x4
        // average would round the original sum six to zero.
        for (row, col) in [(0, 0), (0, 1), (0, 2), (0, 3), (2, 0), (2, 1)] {
            base[row * COLS + col] = 1;
        }

        let pyramid = MotionPyramid::from_base_bytes(&base);
        let level_one = pyramid.level::<AtoB>(Level::One);
        let level_two = pyramid.level::<BtoA>(Level::Two);

        assert_eq!(level_one.bytes().len(), 540 * 30);
        assert_eq!(level_two.bytes().len(), 270 * 15);
        assert_eq!(level_one.direction(), Direction::AtoB);
        assert_eq!(level_two.direction(), Direction::BtoA);
        assert_eq!(&level_one.bytes()[..2], &[1, 1]);
        assert_eq!(level_one.bytes()[Level::One.cols()], 1);
        assert_eq!(level_one.bytes()[Level::One.cols() + 1], 0);
        assert_eq!(level_two.bytes()[0], 1);
    }

    #[test]
    fn zero_blends_retained_and_every_nonzero_byte_selects_fresh() {
        assert_eq!(NON_MOTION_FRESH_WEIGHT.to_bits(), 0x3ca3_d70a);
        assert_eq!((1.0 - NON_MOTION_FRESH_WEIGHT).to_bits(), 0x3f7a_e148);

        let level = Level::Two;
        let retained = RetainedPublicPyramids::from_public(constant_public::<AtoB>(100.0, 200.0));
        let mut bytes = vec![0; level.pixels()];
        bytes[1] = 1;
        bytes[2] = 63;
        bytes[3] = 255;
        let motion = MotionLevel::from_bytes(level, bytes).unwrap();
        let result = update_with_retained(
            refinement::<AtoB>(
                level,
                vec![10.0; level.pixels()],
                vec![20.0; level.pixels()],
                true,
            ),
            &retained,
            motion,
        )
        .unwrap();

        // The constant retained public field becomes (25,50) at L2.
        assert_close(result.field().dcol()[0], 10.0 * 0.02 + 25.0 * 0.98);
        assert_close(result.field().drow()[0], 20.0 * 0.02 + 50.0 * 0.98);
        for pixel in 1..=3 {
            assert_eq!(result.field().dcol()[pixel], 10.0);
            assert_eq!(result.field().drow()[pixel], 20.0);
        }
        assert!(result.variational_rolled_back());
        assert!(result.retained_public_applied());
    }

    #[test]
    fn motion_weight_still_evaluates_retained_times_zero() {
        let level = Level::One;
        let retained =
            RetainedPublicPyramids::from_public(constant_public::<AtoB>(f32::NAN, f32::INFINITY));
        let motion = MotionLevel::from_bytes(level, vec![255; level.pixels()]).unwrap();
        let result = update_with_retained(
            refinement::<AtoB>(
                level,
                vec![3.0; level.pixels()],
                vec![4.0; level.pixels()],
                false,
            ),
            &retained,
            motion,
        )
        .unwrap();

        assert!(result.field().dcol()[0].is_nan());
        assert!(result.field().drow()[0].is_nan());
    }

    #[test]
    fn cold_path_preserves_component_bits_and_rollback_provenance() {
        let level = Level::Two;
        let mut dcol = vec![2.0; level.pixels()];
        let mut drow = vec![-3.0; level.pixels()];
        dcol[0] = f32::from_bits(0x7fc0_0042);
        drow[0] = f32::NEG_INFINITY;
        let expected_dcol = dcol.iter().map(|value| value.to_bits()).collect::<Vec<_>>();
        let expected_drow = drow.iter().map(|value| value.to_bits()).collect::<Vec<_>>();

        let result = preserve_without_retained(refinement::<AtoB>(level, dcol, drow, true));
        assert_eq!(
            result
                .field()
                .dcol()
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            expected_dcol,
        );
        assert_eq!(
            result
                .field()
                .drow()
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            expected_drow,
        );
        assert!(result.variational_rolled_back());
        assert!(!result.retained_public_applied());
    }

    #[test]
    fn disabled_variational_path_preserves_bits_without_claiming_rollback() {
        let level = Level::Two;
        let mut dcol = vec![2.0; level.pixels()];
        let mut drow = vec![-3.0; level.pixels()];
        dcol[0] = f32::from_bits(0x7fc0_0042);
        drow[0] = f32::NEG_INFINITY;
        let expected_dcol = dcol.iter().map(|value| value.to_bits()).collect::<Vec<_>>();
        let expected_drow = drow.iter().map(|value| value.to_bits()).collect::<Vec<_>>();
        let field = DenseField::<AtoB>::from_test_components(level, dcol, drow).unwrap();

        let result = preserve_without_variational_or_retained(field);

        assert_eq!(
            result
                .field()
                .dcol()
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            expected_dcol,
        );
        assert_eq!(
            result
                .field()
                .drow()
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            expected_drow,
        );
        assert_eq!(result.variational_outcome(), VariationalOutcome::Disabled);
        assert!(!result.variational_rolled_back());
        assert!(!result.retained_public_applied());
    }

    #[test]
    fn disabled_variational_warm_path_blends_with_truthful_provenance() {
        let level = Level::Two;
        let retained = RetainedPublicPyramids::from_public(constant_public::<BtoA>(100.0, 200.0));
        let mut bytes = vec![255; level.pixels()];
        bytes[0] = 0;
        let motion = MotionLevel::from_bytes(level, bytes).unwrap();
        let field = DenseField::<BtoA>::from_test_components(
            level,
            vec![10.0; level.pixels()],
            vec![20.0; level.pixels()],
        )
        .unwrap();

        let result = update_without_variational_with_retained(field, &retained, motion).unwrap();

        assert_close(result.field().dcol()[0], 10.0 * 0.02 + 25.0 * 0.98);
        assert_close(result.field().drow()[0], 20.0 * 0.02 + 50.0 * 0.98);
        assert_eq!(result.field().dcol()[1], 10.0);
        assert_eq!(result.field().drow()[1], 20.0);
        assert_eq!(result.variational_outcome(), VariationalOutcome::Disabled);
        assert!(!result.variational_rolled_back());
        assert!(result.retained_public_applied());
        assert_eq!(result.direction(), Direction::BtoA);
    }

    #[test]
    #[ignore = "requires KJERAG_ONE_XS_WARM_PAYLOAD with the authenticated run-06 payload"]
    fn accepted_run06_retained_resize_motion_and_blends_are_bit_exact() {
        use sha2::{Digest as _, Sha256};

        const PAYLOAD_SHA256: &str =
            "a5311370d7946d45094cc85c8ff6631bf2b1195e8c127e3b1f909310168ce8c1";
        const SHARED_MOTION: usize = 389_043;
        const SHARED_MOTION_BYTES: usize = ROWS * COLS;
        const DERIVED_L1_SHA256: &str =
            "83dcb1542384804bc0248365324f52916790374ddafb89b1c9f10ded970e94a1";
        const DERIVED_L2_SHA256: &str =
            "8399b87666109592cc7350a87fa9635b08f4f8c7be2631fbb1af474820f70dca";

        let path = std::env::var_os("KJERAG_ONE_XS_WARM_PAYLOAD")
            .expect("set KJERAG_ONE_XS_WARM_PAYLOAD to authenticated run-06 warm-pair-payload.bin");
        let payload = std::fs::read(path).unwrap();
        assert_eq!(sha256_hex(&payload), PAYLOAD_SHA256);
        assert_eq!(&payload[..8], b"KJWP602\x04");

        let pyramid = MotionPyramid::from_base_bytes(record_bytes(
            &payload,
            SHARED_MOTION,
            SHARED_MOTION_BYTES,
        ));
        assert_eq!(sha256_hex(&pyramid.level_one), DERIVED_L1_SHA256);
        assert_eq!(sha256_hex(&pyramid.level_two), DERIVED_L2_SHA256);

        assert_direction::<AtoB>(
            &payload,
            &pyramid,
            DirectionRecords {
                public: 3_832_456,
                public_sha256: "d7cd591d6e96469ddb5ef91152b9643554cf6b0435831083d51fdbfc055cd319",
                retained_l2_u: 1_671_731,
                retained_l2_v: 1_687_975,
                retained_l1_u: 1_704_219,
                retained_l1_v: 1_769_063,
                fresh_l2_u: 1_606_776,
                fresh_l2_v: 1_623_017,
                expected_l2_u: 1_639_252,
                expected_l2_v: 1_655_487,
                fresh_l1_u: 2_402_268,
                fresh_l1_v: 2_467_109,
                expected_l1_u: 2_531_944,
                expected_l1_v: 2_596_779,
            },
        );
        assert_direction::<BtoA>(
            &payload,
            &pyramid,
            DirectionRecords {
                public: 4_350_889,
                public_sha256: "4d670d4a3ed16675b52c96bb4ebd807c196f77754b1c51161513b8ecfdecd26c",
                retained_l2_u: 2_842_575,
                retained_l2_v: 2_858_819,
                retained_l1_u: 2_875_063,
                retained_l1_v: 2_939_907,
                fresh_l2_u: 2_777_620,
                fresh_l2_v: 2_793_861,
                expected_l2_u: 2_810_096,
                expected_l2_v: 2_826_331,
                fresh_l1_u: 3_573_112,
                fresh_l1_v: 3_637_953,
                expected_l1_u: 3_702_788,
                expected_l1_v: 3_767_623,
            },
        );

        #[derive(Clone, Copy)]
        struct DirectionRecords {
            public: usize,
            public_sha256: &'static str,
            retained_l2_u: usize,
            retained_l2_v: usize,
            retained_l1_u: usize,
            retained_l1_v: usize,
            fresh_l2_u: usize,
            fresh_l2_v: usize,
            expected_l2_u: usize,
            expected_l2_v: usize,
            fresh_l1_u: usize,
            fresh_l1_v: usize,
            expected_l1_u: usize,
            expected_l1_v: usize,
        }

        fn assert_direction<D: PisDirection>(
            payload: &[u8],
            pyramid: &MotionPyramid,
            records: DirectionRecords,
        ) {
            const PUBLIC_BYTES: usize = ROWS * COLS * 2 * size_of::<f32>();
            let public_bytes = record_bytes(payload, records.public, PUBLIC_BYTES);
            assert_eq!(sha256_hex(public_bytes), records.public_sha256);
            let retained = RetainedPublicPyramids::from_public(read_public_field(public_bytes));
            assert_plane(
                &retained.level_one.dcol,
                &read_plane(payload, records.retained_l1_u, Level::One),
            );
            assert_plane(
                &retained.level_one.drow,
                &read_plane(payload, records.retained_l1_v, Level::One),
            );
            assert_plane(
                &retained.level_two.dcol,
                &read_plane(payload, records.retained_l2_u, Level::Two),
            );
            assert_plane(
                &retained.level_two.drow,
                &read_plane(payload, records.retained_l2_v, Level::Two),
            );
            for (level, fresh_u, fresh_v, expected_u, expected_v) in [
                (
                    Level::Two,
                    records.fresh_l2_u,
                    records.fresh_l2_v,
                    records.expected_l2_u,
                    records.expected_l2_v,
                ),
                (
                    Level::One,
                    records.fresh_l1_u,
                    records.fresh_l1_v,
                    records.expected_l1_u,
                    records.expected_l1_v,
                ),
            ] {
                let field = DenseField::<D>::from_test_components(
                    level,
                    read_plane(payload, fresh_u, level).into_vec(),
                    read_plane(payload, fresh_v, level).into_vec(),
                )
                .unwrap();
                let actual = update_without_variational_with_retained(
                    field,
                    &retained,
                    pyramid.level(level),
                )
                .unwrap();
                assert_plane(
                    actual.field().dcol(),
                    &read_plane(payload, expected_u, level),
                );
                assert_plane(
                    actual.field().drow(),
                    &read_plane(payload, expected_v, level),
                );
                assert_eq!(actual.variational_outcome(), VariationalOutcome::Disabled);
            }
        }

        fn read_public_field<D: PisDirection>(bytes: &[u8]) -> PublicDenseField<D> {
            assert_eq!(bytes.len(), ROWS * COLS * 2 * size_of::<f32>());
            let mut dcol = Vec::with_capacity(ROWS * COLS);
            let mut drow = Vec::with_capacity(ROWS * COLS);
            for vector in bytes.chunks_exact(2 * size_of::<f32>()) {
                dcol.push(f32::from_le_bytes(vector[..4].try_into().unwrap()));
                drow.push(f32::from_le_bytes(vector[4..].try_into().unwrap()));
            }
            PublicDenseField::from_test_components(dcol, drow)
        }

        fn read_plane(payload: &[u8], offset: usize, level: Level) -> Box<[f32]> {
            record_bytes(payload, offset, level.pixels() * size_of::<f32>())
                .chunks_exact(4)
                .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
                .collect()
        }

        fn record_bytes(payload: &[u8], offset: usize, bytes: usize) -> &[u8] {
            let end = offset
                .checked_add(bytes)
                .expect("payload record end overflowed");
            payload
                .get(offset..end)
                .expect("payload record lies outside authenticated payload")
        }

        fn assert_plane(actual: &[f32], expected: &[f32]) {
            assert_eq!(actual.len(), expected.len());
            for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
                assert_eq!(actual.to_bits(), expected.to_bits(), "plane value {index}");
            }
        }

        fn sha256_hex(bytes: &[u8]) -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }
    }

    #[test]
    fn a_motion_token_cannot_be_applied_to_another_level() {
        let retained = RetainedPublicPyramids::from_public(constant_public::<AtoB>(0.0, 0.0));
        let motion = MotionLevel::from_bytes(Level::One, vec![0; Level::One.pixels()]).unwrap();
        let error = update_with_retained(
            refinement::<AtoB>(
                Level::Two,
                vec![0.0; Level::Two.pixels()],
                vec![0.0; Level::Two.pixels()],
                false,
            ),
            &retained,
            motion,
        )
        .unwrap_err();
        assert_eq!(error.field, Level::Two);
        assert_eq!(error.motion, Level::One);
    }
}
