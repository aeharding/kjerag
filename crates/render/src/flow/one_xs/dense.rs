//! Scalar sparse-to-dense reference for the selected ONE X2 FDS route.
//!
//! Native runs this main densification once per selected pyramid level after
//! both PIS passes. Level two consumes the coarse [`PatchGrid`] directly;
//! level one can enter only through [`FilteredPatchGrid`], after the selected
//! temporal median. Both entry points consume their sparse token, so one PIS
//! result cannot be submitted twice.
//!
//! The selected main call passes an empty OpenCV mask. Consequently every
//! covering sparse patch votes, regardless of camera-validity masks or the
//! weighted-SSD work-row table used earlier by PIS. Native's separate
//! `calcHintFlow` call reuses the same per-pixel vote but writes only the exact
//! fixed-geometry private mask. That auxiliary result has its own typed,
//! crate-private entry below so its zero initialization and write mask cannot
//! leak into the main densifier.
//!
//! The two downstream resampling helpers consume
//! [`super::post_update::PostUpdateDense`], so neither a raw dense field nor a
//! variational result can bypass the mandatory selected post-VR
//! retained-public-flow update. They reproduce only Studio's selected Mac
//! arm64 OpenCV 4.7 exact-two shapes and operation order: separate
//! `CV_32FC1` L2-to-L1 resizes, and the merged `CV_32FC2` L1-to-public resize.
//! They are not a general OpenCV resize implementation. The separate
//! level-two seed adapter owns the subsequent direct patch-centre sampling.
//!
//! Only those two adapters claim corpus bit identity. Main densification
//! remains a readable semantic reference, and the module is disconnected from
//! production.

use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

use super::pis::{Level, PatchGrid, PisDirection};
use super::post_update::PostUpdateDense;
use super::temporal_median::FilteredPatchGrid;
use super::{COLS, Direction, LensPair, PATCH_SIZE, PATCH_STRIDE, ROWS};

/// Only structure values strictly greater than this enter a row mean.
pub const TEXTURE_VALUE_MIN: f32 = 1.0;

/// A patch-grid row lacks texture when its admitted mean is strictly below
/// this value. The selected call passes exact binary32 `2000.0`.
pub const TEXTURE_MEAN_THRESHOLD: f32 = 2000.0;

/// Native's explicit value for an enabled pixel with no covering patch.
pub const NO_COVER_NAN: f32 = f32::from_bits(0x7fc0_0000);

// Native forms `(dimension - 1) + -0.001f` in binary32 before clamping a
// displaced target coordinate. Keep the recovered constant's exact bits.
const UPPER_CLAMP_EPSILON: f32 = f32::from_bits(0xba83_126f);
const VECTOR_SCALE_PER_LEVEL: f32 = 2.0;

/// Direction-labelled source and target images for one prepared FDS level.
///
/// Callers provide the images in physical A/B order. B-to-A swaps only the
/// images here, matching the selected native directional call. Camera masks
/// are intentionally absent because the main densifier does not consume them.
#[derive(Debug)]
pub struct DirectedImages<'a, D: PisDirection> {
    level: Level,
    source: &'a [u8],
    target: &'a [u8],
    direction: PhantomData<D>,
}

impl<'a, D: PisDirection> DirectedImages<'a, D> {
    /// Admit one contiguous pair with the selected level's exact shape.
    pub fn from_native_order(level: Level, images: LensPair<&'a [u8]>) -> Result<Self, ShapeError> {
        let expected = level.pixels();
        for (part, actual) in [
            ("physical lens A image", images.a.len()),
            ("physical lens B image", images.b.len()),
        ] {
            if actual != expected {
                return Err(ShapeError {
                    direction: D::DIRECTION,
                    level: Some(level),
                    part,
                    expected,
                    actual,
                });
            }
        }

        let (source, target) = match D::DIRECTION {
            Direction::AtoB => (images.a, images.b),
            Direction::BtoA => (images.b, images.a),
        };
        Ok(Self {
            level,
            source,
            target,
            direction: PhantomData,
        })
    }

    pub const fn level(&self) -> Level {
        self.level
    }

    pub const fn direction(&self) -> Direction {
        D::DIRECTION
    }
}

/// A direction-labelled dense field in one internal pyramid level's pixels.
///
/// This type intentionally has no `Clone`: variational refinement and the
/// retained-public update should consume and return one linear field token. It
/// may contain non-finite values because the generic no-cover fallback writes
/// [`NO_COVER_NAN`]. The selected level geometries do not reach that fallback:
/// native clamps their first covering origin to the final sparse patch, which
/// extends that patch across the residual bottom and right fringe.
#[must_use = "the dense field still needs variational and retained-public processing"]
#[derive(Debug)]
pub struct DenseField<D: PisDirection> {
    level: Level,
    dcol: Box<[f32]>,
    drow: Box<[f32]>,
    direction: PhantomData<D>,
}

/// Direction-labelled finest-level output of the masked auxiliary hint vote.
///
/// This is not a main dense field: every pixel starts at positive zero and
/// only the private `calcHintFlow` mask is overwritten. Its sole consumer
/// builds the next direction-owned [`super::warm::HintPyramid`].
#[derive(Debug)]
pub(super) struct HintDenseField<D: PisDirection> {
    dcol: Box<[f32]>,
    drow: Box<[f32]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> HintDenseField<D> {
    pub(super) fn into_components(self) -> (Box<[f32]>, Box<[f32]>) {
        (self.dcol, self.drow)
    }
}

impl<D: PisDirection> DenseField<D> {
    pub const fn level(&self) -> Level {
        self.level
    }

    pub const fn rows(&self) -> usize {
        self.level.rows()
    }

    pub const fn cols(&self) -> usize {
        self.level.cols()
    }

    pub const fn direction(&self) -> Direction {
        D::DIRECTION
    }

    pub fn dcol(&self) -> &[f32] {
        &self.dcol
    }

    pub fn drow(&self) -> &[f32] {
        &self.drow
    }

    /// Build a shape-checked dense fixture for this crate's stage tests.
    ///
    /// Shape, not finiteness, is the invariant at this boundary: stage tests
    /// may deliberately exercise non-finite downstream inputs.
    #[cfg(test)]
    pub(crate) fn from_test_components(
        level: Level,
        dcol: Vec<f32>,
        drow: Vec<f32>,
    ) -> Result<Self, ShapeError> {
        let expected = level.pixels();
        for (part, actual) in [
            ("dense dcol plane", dcol.len()),
            ("dense drow plane", drow.len()),
        ] {
            if actual != expected {
                return Err(ShapeError {
                    direction: D::DIRECTION,
                    level: Some(level),
                    part,
                    expected,
                    actual,
                });
            }
        }
        Ok(Self {
            level,
            dcol: dcol.into_boxed_slice(),
            drow: drow.into_boxed_slice(),
            direction: PhantomData,
        })
    }

    /// Mutate both component planes without dropping their direction label.
    pub(super) fn components_mut(&mut self) -> (&mut [f32], &mut [f32]) {
        (&mut self.dcol, &mut self.drow)
    }
}

/// A direction-labelled field after the final level-one-to-public resize.
///
/// Components are already in 1080-row by 60-column public-grid pixels. The
/// later retained apply consumes these values unchanged and must not apply a
/// second factor of two.
#[must_use = "the public dense field has not yet been handed to the retained apply"]
#[derive(Debug)]
pub struct PublicDenseField<D: PisDirection> {
    dcol: Box<[f32]>,
    drow: Box<[f32]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> PublicDenseField<D> {
    /// Admit separate row-major public-grid component planes.
    ///
    /// Shape, not finiteness, is the invariant at this restore boundary:
    /// native public fields may retain non-finite values from densification.
    pub fn from_row_major_components(dcol: Vec<f32>, drow: Vec<f32>) -> Result<Self, ShapeError> {
        let expected = ROWS * COLS;
        for (part, actual) in [
            ("public dcol plane", dcol.len()),
            ("public drow plane", drow.len()),
        ] {
            if actual != expected {
                return Err(ShapeError {
                    direction: D::DIRECTION,
                    level: None,
                    part,
                    expected,
                    actual,
                });
            }
        }
        Ok(Self {
            dcol: dcol.into_boxed_slice(),
            drow: drow.into_boxed_slice(),
            direction: PhantomData,
        })
    }

    /// Admit one row-major public-grid vector per pixel as `[dcol, drow]`.
    pub fn from_row_major_interleaved(values: Vec<[f32; 2]>) -> Result<Self, ShapeError> {
        let expected = ROWS * COLS;
        if values.len() != expected {
            return Err(ShapeError {
                direction: D::DIRECTION,
                level: None,
                part: "public interleaved vectors",
                expected,
                actual: values.len(),
            });
        }

        let mut dcol = Vec::with_capacity(expected);
        let mut drow = Vec::with_capacity(expected);
        for [col, row] in values {
            dcol.push(col);
            drow.push(row);
        }
        Ok(Self {
            dcol: dcol.into_boxed_slice(),
            drow: drow.into_boxed_slice(),
            direction: PhantomData,
        })
    }

    pub const fn rows(&self) -> usize {
        ROWS
    }

    pub const fn cols(&self) -> usize {
        COLS
    }

    pub const fn direction(&self) -> Direction {
        D::DIRECTION
    }

    pub fn dcol(&self) -> &[f32] {
        &self.dcol
    }

    pub fn drow(&self) -> &[f32] {
        &self.drow
    }

    pub(super) fn components_mut(&mut self) -> (&mut [f32], &mut [f32]) {
        (&mut self.dcol, &mut self.drow)
    }

    #[cfg(test)]
    pub(super) fn into_components(self) -> (Box<[f32]>, Box<[f32]>) {
        (self.dcol, self.drow)
    }

    #[cfg(test)]
    pub(crate) fn from_test_components(dcol: Vec<f32>, drow: Vec<f32>) -> Self {
        Self::from_row_major_components(dcol, drow)
            .expect("public dense test fixture must have the selected shape")
    }
}

/// A direction-labelled row decision from native's low-texture calculation.
#[derive(Debug, PartialEq, Eq)]
pub struct LackRows<D: PisDirection> {
    level: Level,
    rows: Box<[bool]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> LackRows<D> {
    pub const fn level(&self) -> Level {
        self.level
    }

    pub const fn direction(&self) -> Direction {
        D::DIRECTION
    }

    pub fn rows(&self) -> &[bool] {
        &self.rows
    }
}

/// A direction-labelled plane or patch grid has the wrong number of values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapeError {
    direction: Direction,
    level: Option<Level>,
    part: &'static str,
    expected: usize,
    actual: usize,
}

impl fmt::Display for ShapeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(level) = self.level {
            write!(
                out,
                "ONE X2 {} {} has {} {}, expected {}",
                self.direction, level, self.actual, self.part, self.expected,
            )
        } else {
            write!(
                out,
                "ONE X2 {} public field has {} {}, expected {}",
                self.direction, self.actual, self.part, self.expected,
            )
        }
    }
}

impl Error for ShapeError {}

/// A typed dense-stage value belongs to another selected pyramid level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WrongLevel {
    direction: Direction,
    part: &'static str,
    expected: Level,
    actual: Level,
}

impl fmt::Display for WrongLevel {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "ONE X2 {} {} is {}, expected {}",
            self.direction, self.part, self.actual, self.expected,
        )
    }
}

impl Error for WrongLevel {}

/// Densify a coarse, level-two PIS result.
///
/// The sparse token is consumed. Passing a level-one raw PIS result returns a
/// level error rather than bypassing the temporal median.
pub fn densify_coarse<D: PisDirection>(
    images: &DirectedImages<'_, D>,
    patches: PatchGrid<D>,
) -> Result<DenseField<D>, WrongLevel> {
    require_level(
        D::DIRECTION,
        "coarse patch grid",
        patches.level(),
        Level::Two,
    )?;
    require_level(D::DIRECTION, "coarse image pair", images.level, Level::Two)?;

    let mut dcol = Vec::with_capacity(Level::Two.patches());
    let mut drow = Vec::with_capacity(Level::Two.patches());
    for patch in patches.patches() {
        dcol.push(patch.flow().dcol());
        drow.push(patch.flow().drow());
    }
    Ok(densify_components(images, &dcol, &drow))
}

/// Densify a finest-level PIS result after the selected temporal median.
///
/// A raw [`PatchGrid`] is not accepted, making a successful finest-level
/// median bypass ill-typed. The filtered token is consumed, preventing a
/// second submission.
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::dense::{DirectedImages, densify_finest};
/// use kjerag_render::flow::one_xs::pis::{AtoB, PatchGrid};
///
/// fn bypass(images: &DirectedImages<'_, AtoB>, raw: PatchGrid<AtoB>) {
///     let _ = densify_finest(images, raw);
/// }
/// ```
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::dense::{DirectedImages, densify_finest};
/// use kjerag_render::flow::one_xs::pis::AtoB;
/// use kjerag_render::flow::one_xs::temporal_median::FilteredPatchGrid;
///
/// fn twice(
///     images: &DirectedImages<'_, AtoB>,
///     filtered: FilteredPatchGrid<AtoB>,
/// ) {
///     let _first = densify_finest(images, filtered);
///     let _second = densify_finest(images, filtered);
/// }
/// ```
pub fn densify_finest<D: PisDirection>(
    images: &DirectedImages<'_, D>,
    patches: FilteredPatchGrid<D>,
) -> Result<DenseField<D>, WrongLevel> {
    require_level(D::DIRECTION, "finest image pair", images.level, Level::One)?;
    let (dcol, drow) = patches.into_row_major_components();
    Ok(densify_components(images, &dcol, &drow))
}

/// Reproduce the selected finest-level `calcHintFlow` densification.
///
/// Both inputs are borrowed because the raw PIS result must subsequently enter
/// the temporal median. Native zeroes the complete auxiliary destination and
/// invokes the ordinary photometric vote only at the fixed private-mask
/// coordinates.
pub(super) fn densify_hint<D: PisDirection>(
    images: &DirectedImages<'_, D>,
    patches: &PatchGrid<D>,
) -> Result<HintDenseField<D>, WrongLevel> {
    require_level(D::DIRECTION, "hint patch grid", patches.level(), Level::One)?;
    require_level(D::DIRECTION, "hint image pair", images.level, Level::One)?;

    let mask = private_hint_mask();
    let mut dcol = vec![0.0f32; Level::One.pixels()];
    let mut drow = vec![0.0f32; Level::One.pixels()];
    for (pixel, enabled) in mask.into_iter().enumerate() {
        if enabled == 0 {
            continue;
        }
        let row = pixel / Level::One.cols();
        let col = pixel % Level::One.cols();
        let (vote_col, vote_row) = photometric_vote_at(images, row, col, |patch| {
            let flow = patches.patches()[patch].flow();
            (flow.dcol(), flow.drow())
        })
        .expect("selected private hint-mask coordinate has a covering patch");
        dcol[pixel] = vote_col;
        drow[pixel] = vote_row;
    }

    Ok(HintDenseField {
        dcol: dcol.into_boxed_slice(),
        drow: drow.into_boxed_slice(),
        direction: PhantomData,
    })
}

/// Reproduce native's low-texture row classification.
///
/// Each input row has one scalar per sparse patch column. Only ordered values
/// strictly above 1 enter its binary32 sum and count. No admitted value leaves
/// mean zero. A row is flagged only when the resulting mean is strictly below
/// 2000. NaNs therefore do not enter the mean; positive infinity does.
pub fn calculate_lack_rows<D: PisDirection>(
    level: Level,
    patch_values: &[f32],
) -> Result<LackRows<D>, ShapeError> {
    let expected = level.patches();
    if patch_values.len() != expected {
        return Err(ShapeError {
            direction: D::DIRECTION,
            level: Some(level),
            part: "texture-grid values",
            expected,
            actual: patch_values.len(),
        });
    }

    let rows = patch_values
        .chunks_exact(level.patch_cols())
        .map(|values| {
            let mut sum = 0.0f32;
            let mut count = 0usize;
            for value in values.iter().copied() {
                if value > TEXTURE_VALUE_MIN {
                    sum += value;
                    count += 1;
                }
            }
            let mean = if count == 0 { 0.0 } else { sum / count as f32 };
            mean < TEXTURE_MEAN_THRESHOLD
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    Ok(LackRows {
        level,
        rows,
        direction: PhantomData,
    })
}

/// Linearly resize a post-update level-two field to level one, then multiply
/// both vector components by two so they remain displacements in the
/// destination grid.
///
pub(super) fn upsample_linear_x2<D: PisDirection>(
    coarse: PostUpdateDense<D>,
) -> Result<DenseField<D>, WrongLevel> {
    let coarse = coarse.into_field();
    require_level(D::DIRECTION, "coarse dense field", coarse.level, Level::Two)?;
    let dcol = resize_selected_x2::<1>(&coarse.dcol, Level::Two.rows(), Level::Two.cols());
    let drow = resize_selected_x2::<1>(&coarse.drow, Level::Two.rows(), Level::Two.cols());
    Ok(DenseField {
        level: Level::One,
        dcol: dcol.into_boxed_slice(),
        drow: drow.into_boxed_slice(),
        direction: PhantomData,
    })
}

/// Linearly resize a post-update level-one field to the public 1080-by-60
/// grid, then multiply both components by two exactly once.
pub fn finish_linear_x2<D: PisDirection>(
    finest: PostUpdateDense<D>,
) -> Result<PublicDenseField<D>, WrongLevel> {
    let finest = finest.into_field();
    require_level(D::DIRECTION, "finest dense field", finest.level, Level::One)?;
    let mut merged = Vec::with_capacity(Level::One.pixels() * 2);
    for pixel in 0..Level::One.pixels() {
        merged.push(finest.dcol[pixel]);
        merged.push(finest.drow[pixel]);
    }
    let resized = resize_selected_x2::<2>(&merged, Level::One.rows(), Level::One.cols());
    let mut dcol = Vec::with_capacity(ROWS * COLS);
    let mut drow = Vec::with_capacity(ROWS * COLS);
    for pair in resized.chunks_exact(2) {
        dcol.push(pair[0]);
        drow.push(pair[1]);
    }
    Ok(PublicDenseField {
        dcol: dcol.into_boxed_slice(),
        drow: drow.into_boxed_slice(),
        direction: PhantomData,
    })
}

fn require_level(
    direction: Direction,
    part: &'static str,
    actual: Level,
    expected: Level,
) -> Result<(), WrongLevel> {
    if actual == expected {
        Ok(())
    } else {
        Err(WrongLevel {
            direction,
            part,
            expected,
            actual,
        })
    }
}

fn densify_components<D: PisDirection>(
    images: &DirectedImages<'_, D>,
    sparse_dcol: &[f32],
    sparse_drow: &[f32],
) -> DenseField<D> {
    let level = images.level;
    debug_assert_eq!(sparse_dcol.len(), level.patches());
    debug_assert_eq!(sparse_drow.len(), level.patches());
    let mut dcol = vec![NO_COVER_NAN; level.pixels()];
    let mut drow = vec![NO_COVER_NAN; level.pixels()];

    for row in 0..level.rows() {
        for col in 0..level.cols() {
            let Some((vote_col, vote_row)) = photometric_vote_at(images, row, col, |patch| {
                (sparse_dcol[patch], sparse_drow[patch])
            }) else {
                continue;
            };
            let pixel = row * level.cols() + col;
            dcol[pixel] = vote_col;
            drow[pixel] = vote_row;
        }
    }

    DenseField {
        level,
        dcol: dcol.into_boxed_slice(),
        drow: drow.into_boxed_slice(),
        direction: PhantomData,
    }
}

fn photometric_vote_at<D, F>(
    images: &DirectedImages<'_, D>,
    row: usize,
    col: usize,
    mut sparse_flow: F,
) -> Option<(f32, f32)>
where
    D: PisDirection,
    F: FnMut(usize) -> (f32, f32),
{
    let level = images.level;
    let patch_row_range = covering_patch_range(row, level.patch_rows())?;
    let patch_col_range = covering_patch_range(col, level.patch_cols())?;
    let source = f32::from(images.source[row * level.cols() + col]);
    let mut dcol_sum = 0.0f32;
    let mut drow_sum = 0.0f32;
    let mut weight_sum = 0.0f32;

    // Native enumerates covering origins in ascending patch row, then
    // ascending patch column, and accumulates each component with FMA.
    for patch_row in patch_row_range {
        for patch_col in patch_col_range.clone() {
            let patch = patch_row * level.patch_cols() + patch_col;
            let (patch_dcol, patch_drow) = sparse_flow(patch);
            let target = sample_clamped_bilinear(
                images.target,
                level.rows(),
                level.cols(),
                row as f32 + patch_drow,
                col as f32 + patch_dcol,
            );
            let difference = (source - target).abs();
            let reciprocal = 1.0f32 / difference;
            let weight = if difference > 1.0 { reciprocal } else { 1.0 };
            dcol_sum = weight.mul_add(patch_dcol, dcol_sum);
            drow_sum = weight.mul_add(patch_drow, drow_sum);
            weight_sum += weight;
        }
    }

    Some((dcol_sum / weight_sum, drow_sum / weight_sum))
}

/// Reconstruct the selected 540-by-30 `FDS+0x488` mask.
///
/// It is the union of all L1 patch centres and all four L1 children of every
/// L2 patch centre. Native stores `0xff` at every member and zero elsewhere.
pub(super) fn private_hint_mask() -> Box<[u8]> {
    let mut mask = vec![0u8; Level::One.pixels()];
    let mut enable = |row: usize, col: usize| {
        mask[row * Level::One.cols() + col] = u8::MAX;
    };

    for patch_row in 0..Level::One.patch_rows() {
        let row = patch_row * PATCH_STRIDE + PATCH_SIZE / 2;
        for patch_col in 0..Level::One.patch_cols() {
            let col = patch_col * PATCH_STRIDE + PATCH_SIZE / 2;
            enable(row, col);
        }
    }
    for patch_row in 0..Level::Two.patch_rows() {
        let coarse_row = patch_row * PATCH_STRIDE + PATCH_SIZE / 2;
        for patch_col in 0..Level::Two.patch_cols() {
            let coarse_col = patch_col * PATCH_STRIDE + PATCH_SIZE / 2;
            for child_row in [2 * coarse_row, 2 * coarse_row + 1] {
                for child_col in [2 * coarse_col, 2 * coarse_col + 1] {
                    enable(
                        child_row.min(Level::One.rows() - 1),
                        child_col.min(Level::One.cols() - 1),
                    );
                }
            }
        }
    }

    mask.into_boxed_slice()
}

fn covering_patch_range(
    pixel: usize,
    patch_count: usize,
) -> Option<std::ops::RangeInclusive<usize>> {
    let last_patch = patch_count.checked_sub(1)?;
    let first = (pixel + 1)
        .saturating_sub(PATCH_SIZE)
        .div_ceil(PATCH_STRIDE)
        .min(last_patch);
    let last = (pixel / PATCH_STRIDE).min(last_patch);
    (first <= last).then_some(first..=last)
}

fn sample_clamped_bilinear(image: &[u8], rows: usize, cols: usize, row: f32, col: f32) -> f32 {
    let upper_row = (rows as f32 - 1.0) + UPPER_CLAMP_EPSILON;
    let upper_col = (cols as f32 - 1.0) + UPPER_CLAMP_EPSILON;
    let row = ordered_clamp(row, upper_row);
    let col = ordered_clamp(col, upper_col);
    // Like AArch64 FCVTZS, Rust's float-to-usize cast maps NaN to zero. The
    // fractional terms remain NaN and propagate through the interpolation.
    let row0 = row as usize;
    let col0 = col as usize;
    let row1 = row0 + 1;
    let col1 = col0 + 1;
    let row_fraction = row - row0 as f32;
    let col_fraction = col - col0 as f32;
    let row_inverse = row1 as f32 - row;
    let col_inverse = col1 as f32 - col;

    let bottom_right = f32::from(image[row1 * cols + col1]);
    let bottom_left = f32::from(image[row1 * cols + col0]);
    let top_right = f32::from(image[row0 * cols + col1]);
    let top_left = f32::from(image[row0 * cols + col0]);

    let bottom_left_term = (col_inverse * row_fraction) * bottom_left;
    let bottom = (col_fraction * row_fraction).mul_add(bottom_right, bottom_left_term);
    let right = (col_fraction * row_inverse).mul_add(top_right, bottom);
    (col_inverse * row_inverse).mul_add(top_left, right)
}

fn ordered_clamp(value: f32, upper: f32) -> f32 {
    let value = if value < 0.0 { 0.0 } else { value };
    if upper < value { upper } else { value }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HorizontalLane {
    Simd,
    Scalar,
    Copy,
}

/// Reproduce the two exact selected Mac arm64 OpenCV 4.7 x2 resize shapes.
///
/// `CHANNELS=1` is one component plane at L2. `CHANNELS=2` is the merged U/V
/// L1 field. OpenCV dispatches over flattened channel lanes, which is why the
/// final merged resize cannot be replaced by two independent plane resizes.
fn resize_selected_x2<const CHANNELS: usize>(
    source: &[f32],
    source_rows: usize,
    source_cols: usize,
) -> Vec<f32> {
    assert!(
        matches!(
            (CHANNELS, source_rows, source_cols),
            (1, 270, 15) | (2, 540, 30)
        ),
        "ONE X2 exact downstream resize has an unsupported shape",
    );
    assert_eq!(source.len(), source_rows * source_cols * CHANNELS);

    let target_rows = source_rows * 2;
    let target_cols = source_cols * 2;
    let target_lanes = target_cols * CHANNELS;
    let mut horizontal = vec![0.0; source_rows * target_lanes];
    for row in 0..source_rows {
        let source_row = &source[row * source_cols * CHANNELS..][..source_cols * CHANNELS];
        let target_row = &mut horizontal[row * target_lanes..][..target_lanes];
        for col in 0..target_cols {
            let (left, right, left_weight, right_weight) = horizontal_x2_sample(col, source_cols);
            for channel in 0..CHANNELS {
                let lane = col * CHANNELS + channel;
                target_row[lane] = match horizontal_lane(col, channel, target_cols, CHANNELS) {
                    HorizontalLane::Simd => horizontal_simd(
                        source_row[left * CHANNELS + channel],
                        source_row[right * CHANNELS + channel],
                        left_weight,
                        right_weight,
                    ),
                    HorizontalLane::Scalar => horizontal_scalar(
                        source_row[left * CHANNELS + channel],
                        source_row[right * CHANNELS + channel],
                        left_weight,
                        right_weight,
                    ),
                    HorizontalLane::Copy => source_row[left * CHANNELS + channel],
                };
            }
        }
    }

    let mut target = vec![0.0; target_rows * target_lanes];
    for row in 0..target_rows {
        let (top, bottom, top_weight, bottom_weight) = vertical_x2_sample(row, source_rows);
        let top_row = &horizontal[top * target_lanes..][..target_lanes];
        let bottom_row = &horizontal[bottom * target_lanes..][..target_lanes];
        let target_row = &mut target[row * target_lanes..][..target_lanes];
        for lane in 0..target_lanes {
            // The four-lane SIMD body and its scalar remainder have the same
            // selected fmul-then-fmadd association.
            target_row[lane] =
                vertical_linear(top_row[lane], bottom_row[lane], top_weight, bottom_weight)
                    * VECTOR_SCALE_PER_LEVEL;
        }
    }
    target
}

fn horizontal_lane(
    target_col: usize,
    channel: usize,
    target_cols: usize,
    channels: usize,
) -> HorizontalLane {
    if target_col == target_cols - 1 {
        return HorizontalLane::Copy;
    }
    let xmax = (target_cols - 1) * channels;
    let simd_end = xmax & !3;
    if target_col * channels + channel < simd_end {
        HorizontalLane::Simd
    } else {
        HorizontalLane::Scalar
    }
}

#[cfg(test)]
fn vertical_lane_is_simd(lane: usize, target_lanes: usize) -> bool {
    lane < (target_lanes & !3)
}

fn horizontal_x2_sample(target: usize, source_len: usize) -> (usize, usize, f32, f32) {
    if target == 0 {
        (0, 1, 1.0, 0.0)
    } else if target == source_len * 2 - 1 {
        (source_len - 1, source_len - 1, 1.0, 0.0)
    } else if target.is_multiple_of(2) {
        (target / 2 - 1, target / 2, 0.25, 0.75)
    } else {
        (target / 2, target / 2 + 1, 0.75, 0.25)
    }
}

fn vertical_x2_sample(target: usize, source_len: usize) -> (usize, usize, f32, f32) {
    if target == 0 {
        (0, 0, 0.25, 0.75)
    } else if target == source_len * 2 - 1 {
        (source_len - 1, source_len - 1, 0.75, 0.25)
    } else if target.is_multiple_of(2) {
        (target / 2 - 1, target / 2, 0.25, 0.75)
    } else {
        (target / 2, target / 2 + 1, 0.75, 0.25)
    }
}

fn horizontal_simd(left: f32, right: f32, left_weight: f32, right_weight: f32) -> f32 {
    let left = left * left_weight;
    let right = right * right_weight;
    left + right
}

fn horizontal_scalar(left: f32, right: f32, left_weight: f32, right_weight: f32) -> f32 {
    let right = right * right_weight;
    left.mul_add(left_weight, right)
}

fn vertical_linear(top: f32, bottom: f32, top_weight: f32, bottom_weight: f32) -> f32 {
    let bottom = bottom * bottom_weight;
    top.mul_add(top_weight, bottom)
}

/// Generic semantic resize retained only for synthetic negative comparisons.
///
/// This difference-FMA helper does not reproduce the selected Mac arm64
/// OpenCV operation order and must not be used by either downstream adapter.
#[cfg(test)]
pub(super) fn resize_linear_scaled(
    source: &[f32],
    source_rows: usize,
    source_cols: usize,
    target_rows: usize,
    target_cols: usize,
    scale: f32,
) -> Vec<f32> {
    debug_assert_eq!(source.len(), source_rows * source_cols);
    let mut target = Vec::with_capacity(target_rows * target_cols);
    for row in 0..target_rows {
        let (row0, row1, row_fraction) = resize_axis(row, source_rows, target_rows);
        for col in 0..target_cols {
            let (col0, col1, col_fraction) = resize_axis(col, source_cols, target_cols);
            let top_left = source[row0 * source_cols + col0];
            let top_right = source[row0 * source_cols + col1];
            let bottom_left = source[row1 * source_cols + col0];
            let bottom_right = source[row1 * source_cols + col1];
            let top = (top_right - top_left).mul_add(col_fraction, top_left);
            let bottom = (bottom_right - bottom_left).mul_add(col_fraction, bottom_left);
            let value = (bottom - top).mul_add(row_fraction, top);
            target.push(value * scale);
        }
    }
    target
}

#[cfg(test)]
fn resize_axis(target: usize, source_len: usize, target_len: usize) -> (usize, usize, f32) {
    let coordinate = (target as f32 + 0.5) * source_len as f32 / target_len as f32 - 0.5;
    let lower = coordinate.floor() as isize;
    if lower < 0 {
        return (0, 0, 0.0);
    }
    let lower = lower as usize;
    if lower >= source_len - 1 {
        return (source_len - 1, source_len - 1, 0.0);
    }
    (lower, lower + 1, coordinate - lower as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::one_xs::pis::{AtoB, BtoA};
    use crate::flow::one_xs::post_update::PostUpdateDense;
    use crate::flow::one_xs::temporal_median::AtoBMedian;

    fn images<'a, D: PisDirection>(
        level: Level,
        a: &'a [u8],
        b: &'a [u8],
    ) -> DirectedImages<'a, D> {
        DirectedImages::from_native_order(level, LensPair { a, b }).unwrap()
    }

    fn grid<D: PisDirection>(level: Level, dcol: Vec<f32>, drow: Vec<f32>) -> PatchGrid<D> {
        PatchGrid::from_row_major_components(level, dcol, drow).unwrap()
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 1.0e-5,
            "actual {actual:?}, expected {expected:?}",
        );
    }

    #[test]
    fn selected_geometry_extends_the_last_patch_across_the_residual_fringe() {
        assert_eq!((Level::One.rows(), Level::One.cols()), (540, 30));
        assert_eq!((Level::One.patch_rows(), Level::One.patch_cols()), (178, 8));
        assert_eq!((Level::Two.rows(), Level::Two.cols()), (270, 15));
        assert_eq!((Level::Two.patch_rows(), Level::Two.patch_cols()), (88, 3));

        let level = Level::Two;
        let a = vec![17; level.pixels()];
        let b = a.clone();
        let patches = grid::<AtoB>(
            level,
            vec![2.0; level.patches()],
            vec![-3.0; level.patches()],
        );
        let field = densify_coarse(&images(level, &a, &b), patches).unwrap();

        for row in 0..level.rows() {
            let fringe = row * level.cols() + level.cols() - 1;
            assert_eq!(field.dcol()[fringe], 2.0);
            assert_eq!(field.drow()[fringe], -3.0);
        }
        for col in 0..level.cols() {
            let fringe = (level.rows() - 1) * level.cols() + col;
            assert_eq!(field.dcol()[fringe], 2.0);
            assert_eq!(field.drow()[fringe], -3.0);
        }
    }

    #[test]
    fn selected_coverage_clamps_only_the_first_origin_at_each_far_boundary() {
        let level = Level::Two;

        // The final row origin is 87 * 3 = 261. Its ordinary eight-pixel
        // footprint ends at row 268, then native extends it through row 269.
        assert_eq!(87 * PATCH_STRIDE + PATCH_SIZE - 1, 268);
        assert_eq!(covering_patch_range(268, level.patch_rows()), Some(87..=87));
        assert_eq!(covering_patch_range(269, level.patch_rows()), Some(87..=87));

        // The final column origin is 2 * 3 = 6. Its ordinary footprint ends
        // at column 13, then native extends it through column 14.
        assert_eq!(2 * PATCH_STRIDE + PATCH_SIZE - 1, 13);
        assert_eq!(covering_patch_range(13, level.patch_cols()), Some(2..=2));
        assert_eq!(covering_patch_range(14, level.patch_cols()), Some(2..=2));

        // Keep the generic truly-empty fallback explicit even though neither
        // selected level has an empty sparse dimension.
        assert_eq!(covering_patch_range(0, 0), None);
    }

    #[test]
    fn private_hint_mask_matches_the_complete_captured_geometry() {
        use sha2::{Digest as _, Sha256};

        let mask = private_hint_mask();
        assert_eq!(mask.len(), 16_200);
        assert_eq!(
            mask.iter().filter(|value| **value == u8::MAX).count(),
            2_480
        );
        assert_eq!(mask.iter().filter(|value| **value == 0).count(), 13_720);
        assert!(mask.iter().all(|value| matches!(*value, 0 | u8::MAX)));
        let hash = Sha256::digest(&mask)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            hash,
            "d669875402f653cece7ea45a3774a15c8eba0a5d006872d3bb8e02c3a0779ae2"
        );

        for (row, col) in [(4, 4), (535, 25), (8, 8), (8, 9), (9, 8), (9, 9)] {
            assert_eq!(mask[row * Level::One.cols() + col], u8::MAX);
        }
        assert_eq!(mask[0], 0);
        assert_eq!(mask[Level::One.pixels() - 1], 0);
    }

    #[test]
    fn hint_densification_writes_only_the_private_mask_with_the_main_vote_bits() {
        let level = Level::One;
        let a = vec![73; level.pixels()];
        let b = a.clone();
        let dcol = (0..level.patches())
            .map(|index| (index % 17) as f32 - 8.0)
            .collect::<Vec<_>>();
        let drow = (0..level.patches())
            .map(|index| (index % 11) as f32 - 5.0)
            .collect::<Vec<_>>();
        let directed = images::<AtoB>(level, &a, &b);
        let main = densify_components(&directed, &dcol, &drow);
        let hint =
            densify_hint(&directed, &grid::<AtoB>(level, dcol.clone(), drow.clone())).unwrap();
        let (hint_col, hint_row) = hint.into_components();
        let mask = private_hint_mask();

        for pixel in 0..level.pixels() {
            if mask[pixel] == 0 {
                assert_eq!(hint_col[pixel].to_bits(), 0.0f32.to_bits());
                assert_eq!(hint_row[pixel].to_bits(), 0.0f32.to_bits());
            } else {
                assert_eq!(hint_col[pixel].to_bits(), main.dcol()[pixel].to_bits());
                assert_eq!(hint_row[pixel].to_bits(), main.drow()[pixel].to_bits());
            }
        }

        let wrong_level = densify_hint(
            &images::<BtoA>(
                Level::Two,
                &a[..Level::Two.pixels()],
                &b[..Level::Two.pixels()],
            ),
            &grid::<BtoA>(
                Level::Two,
                vec![0.0; Level::Two.patches()],
                vec![0.0; Level::Two.patches()],
            ),
        )
        .unwrap_err();
        assert_eq!(wrong_level.direction, Direction::BtoA);
        assert_eq!(wrong_level.part, "hint patch grid");
        assert_eq!(wrong_level.actual, Level::Two);
        assert_eq!(wrong_level.expected, Level::One);
    }

    #[test]
    fn photometric_vote_uses_cutoff_one_and_keeps_component_order() {
        let level = Level::Two;
        let a = vec![10; level.pixels()];
        let b = (0..level.rows())
            .flat_map(|_| (0..level.cols()).map(|col| 10 + col as u8))
            .collect::<Vec<_>>();
        let mut dcol = vec![0.0; level.patches()];
        let mut drow = vec![0.0; level.patches()];
        for (patch_row, patch_col, col_flow, row_flow) in [
            (0, 0, -3.0, 0.0),
            (0, 1, -2.0, 10.0),
            (1, 0, -1.0, 20.0),
            (1, 1, 0.0, 30.0),
        ] {
            let at = patch_row * level.patch_cols() + patch_col;
            dcol[at] = col_flow;
            drow[at] = row_flow;
        }
        let field =
            densify_coarse(&images::<AtoB>(level, &a, &b), grid(level, dcol, drow)).unwrap();

        let weights = [1.0f32, 1.0, 0.5, 1.0 / 3.0];
        let weight_sum = weights.into_iter().sum::<f32>();
        let expected_col = (-3.0 - 2.0 - 0.5) / weight_sum;
        let expected_row = (10.0 + 10.0 + 10.0) / weight_sum;
        let at = 3 * level.cols() + 3;
        assert_close(field.dcol()[at], expected_col);
        assert_close(field.drow()[at], expected_row);
    }

    #[test]
    fn vote_accumulation_is_patch_row_then_patch_column() {
        let level = Level::Two;
        let a = vec![23; level.pixels()];
        let b = a.clone();
        let mut dcol = vec![0.0; level.patches()];
        let drow = vec![0.0; level.patches()];
        let large = 16_777_216.0f32;
        let expected_patch_indices = [0, 1, 3, 4];
        for (patch, value) in expected_patch_indices
            .into_iter()
            .zip([large, 1.0, -large, 1.0])
        {
            dcol[patch] = value;
        }

        assert_eq!(covering_patch_range(3, level.patch_rows()).unwrap(), 0..=1);
        assert_eq!(covering_patch_range(3, level.patch_cols()).unwrap(), 0..=1);
        let field =
            densify_coarse(&images::<AtoB>(level, &a, &b), grid(level, dcol, drow)).unwrap();
        let at = 3 * level.cols() + 3;

        // gy-major/gx-minor is [large, 1, -large, 1]: its binary32 sum is 1.
        // Swapping the loops gives [large, -large, 1, 1], whose sum is 2.
        assert_eq!(field.dcol()[at], 0.25);
        let swapped_sum = [large, -large, 1.0, 1.0]
            .into_iter()
            .fold(0.0f32, |sum, value| 1.0f32.mul_add(value, sum));
        assert_eq!(swapped_sum / 4.0, 0.5);
    }

    #[test]
    fn physical_images_swap_for_reverse_direction_only() {
        let level = Level::Two;
        let a = vec![11; level.pixels()];
        let b = vec![29; level.pixels()];
        let forward = images::<AtoB>(level, &a, &b);
        let reverse = images::<BtoA>(level, &a, &b);
        assert_eq!(forward.source[0], 11);
        assert_eq!(forward.target[0], 29);
        assert_eq!(reverse.source[0], 29);
        assert_eq!(reverse.target[0], 11);
    }

    #[test]
    fn clamp_bilinear_covers_fractional_edges_and_nan() {
        let image = [0, 10, 20, 30, 40, 50];
        assert_close(sample_clamped_bilinear(&image, 2, 3, 0.5, 0.5), 20.0);
        assert_eq!(sample_clamped_bilinear(&image, 2, 3, -4.0, -2.0), 0.0);

        let upper = sample_clamped_bilinear(&image, 2, 3, 99.0, 99.0);
        assert!(upper > 49.9 && upper < 50.0, "upper edge was {upper}");
        assert!(sample_clamped_bilinear(&image, 2, 3, f32::NAN, 0.0).is_nan());
    }

    #[test]
    fn texture_lack_boundaries_are_strict_and_ordered() {
        let level = Level::Two;
        let mut values = vec![2000.0; level.patches()];
        let next_after_one = f32::from_bits(1.0f32.to_bits() + 1);
        values[0..3].copy_from_slice(&[f32::NAN, 1.0, -4.0]);
        values[3..6].copy_from_slice(&[2000.0, 1.0, f32::NAN]);
        values[6..9].copy_from_slice(&[f32::from_bits(2000.0f32.to_bits() - 1), 1.0, f32::NAN]);
        values[9..12].copy_from_slice(&[1.0, 3998.0, f32::NAN]);
        values[12..15].copy_from_slice(&[next_after_one, 3998.0, 1.0]);
        values[15..18].copy_from_slice(&[f32::INFINITY, 1.0, f32::NAN]);

        let rows = calculate_lack_rows::<AtoB>(level, &values).unwrap();
        assert_eq!(&rows.rows()[..6], &[true, false, true, false, true, false]);
    }

    #[test]
    fn finest_requires_the_filtered_token_and_consumes_it() {
        let level = Level::One;
        let a = vec![3; level.pixels()];
        let b = a.clone();
        let raw = grid::<AtoB>(
            level,
            vec![0.0; level.patches()],
            vec![4.0; level.patches()],
        );
        let filtered = AtoBMedian::new().run(raw).unwrap();
        let field = densify_finest(&images(level, &a, &b), filtered).unwrap();
        assert_eq!(field.level(), Level::One);
        assert_eq!(field.drow()[0], 4.0);
    }

    #[test]
    fn coarse_rejects_a_raw_finest_grid() {
        let level = Level::One;
        let a = vec![0; level.pixels()];
        let b = a.clone();
        let raw = grid::<AtoB>(
            level,
            vec![0.0; level.patches()],
            vec![0.0; level.patches()],
        );
        let error = densify_coarse(&images(level, &a, &b), raw).unwrap_err();
        assert_eq!(error.actual, Level::One);
        assert_eq!(error.expected, Level::Two);
    }

    #[test]
    fn linear_resize_then_scale_preserves_axes_and_public_units() {
        let level = Level::Two;
        let dcol = (0..level.rows())
            .flat_map(|row| (0..level.cols()).map(move |col| (row * 100 + col) as f32))
            .collect::<Vec<_>>();
        let drow = (0..level.rows())
            .flat_map(|row| (0..level.cols()).map(move |col| -(row as f32 * 10.0 + col as f32)))
            .collect::<Vec<_>>();
        let finest = upsample_linear_x2(PostUpdateDense::from_test_field(
            DenseField::<AtoB>::from_test_components(level, dcol, drow).unwrap(),
            false,
        ))
        .unwrap();
        assert_eq!((finest.rows(), finest.cols()), (540, 30));
        assert_eq!(finest.dcol()[0], 0.0);
        assert_close(finest.dcol()[finest.cols() + 1], 50.5);
        assert_close(finest.drow()[finest.cols() + 1], -5.5);
        let last = finest.rows() * finest.cols() - 1;
        assert_eq!(finest.dcol()[last], ((269 * 100 + 14) as f32) * 2.0);

        let constant = DenseField::<AtoB>::from_test_components(
            Level::One,
            vec![1.25; Level::One.pixels()],
            vec![-2.5; Level::One.pixels()],
        )
        .unwrap();
        let public = finish_linear_x2(PostUpdateDense::from_test_field(constant, false)).unwrap();
        assert_eq!((public.rows(), public.cols()), (1080, 60));
        assert!(public.dcol().iter().all(|value| *value == 2.5));
        assert!(public.drow().iter().all(|value| *value == -5.0));
    }

    #[test]
    fn selected_resize_binds_arm64_simd_tail_and_copy_lanes() {
        assert_eq!(horizontal_lane(27, 0, 30, 1), HorizontalLane::Simd);
        assert_eq!(horizontal_lane(28, 0, 30, 1), HorizontalLane::Scalar);
        assert_eq!(horizontal_lane(29, 0, 30, 1), HorizontalLane::Copy);
        assert!(vertical_lane_is_simd(27, 30));
        assert!(!vertical_lane_is_simd(28, 30));
        assert!(!vertical_lane_is_simd(29, 30));

        // The merged CV_32FC2 path dispatches over 120 flattened lanes. Both
        // components of column 57 are SIMD, both of column 58 are scalar, and
        // both of column 59 take OpenCV's right-edge copy loop.
        for channel in 0..2 {
            assert_eq!(horizontal_lane(57, channel, 60, 2), HorizontalLane::Simd);
            assert_eq!(horizontal_lane(58, channel, 60, 2), HorizontalLane::Scalar);
            assert_eq!(horizontal_lane(59, channel, 60, 2), HorizontalLane::Copy);
        }
        assert!(vertical_lane_is_simd(119, 120));
    }

    #[test]
    fn selected_resize_binds_separate_and_fused_horizontal_schedules() {
        let left = f32::from_bits(0x4fc7_f911);
        let right = f32::from_bits(0x4f8c_a62a);
        let simd = horizontal_simd(left, right, 0.75, 0.25);
        let scalar = horizontal_scalar(left, right, 0.75, 0.25);

        assert_eq!(simd.to_bits(), 0x4fb9_2458);
        assert_eq!(scalar.to_bits(), 0x4fb9_2457);
        assert_ne!(simd.to_bits(), scalar.to_bits());
    }

    #[test]
    fn selected_resize_binds_vertical_borders_and_horizontal_copy_edge() {
        assert_eq!(horizontal_x2_sample(0, 15), (0, 1, 1.0, 0.0));
        assert_eq!(horizontal_x2_sample(29, 15), (14, 14, 1.0, 0.0));
        assert_eq!(vertical_x2_sample(0, 270), (0, 0, 0.25, 0.75));
        assert_eq!(vertical_x2_sample(539, 270), (269, 269, 0.75, 0.25));

        let mut source = vec![0.0; Level::Two.pixels()];
        for row in 0..Level::Two.rows() {
            source[row * Level::Two.cols() + 1] = f32::NAN;
            source[row * Level::Two.cols() + Level::Two.cols() - 1] = 3.25;
        }
        let resized = resize_selected_x2::<1>(&source, Level::Two.rows(), Level::Two.cols());
        for row in 0..Level::One.rows() {
            // Column zero executes 1*source[0] + 0*source[1], including the
            // zero-times-NaN operation. The right edge is a true copy before
            // the following exact unit conversion.
            assert!(resized[row * Level::One.cols()].is_nan());
            assert_eq!(resized[row * Level::One.cols() + 29], 6.5);
        }
    }

    #[test]
    #[ignore = "requires KJERAG_ONE_XS_WARM_PAYLOAD with authenticated run-06 warm-pair-payload.bin"]
    fn accepted_run06_downstream_resizes_are_bit_exact() {
        use sha2::{Digest as _, Sha256};

        const PAYLOAD_BYTES: usize = 7_238_679;
        const PAYLOAD_SHA256: &str =
            "a5311370d7946d45094cc85c8ff6631bf2b1195e8c127e3b1f909310168ce8c1";
        const AB_L2_U: Record = Record::new(
            "ab_l2_post_blend_main_u",
            1_639_252,
            16_200,
            "8a1302896b8ef716015e4d33b002a88b8ba58afd44f6aecf413e3b98ce424634",
        );
        const AB_L2_V: Record = Record::new(
            "ab_l2_post_blend_main_v",
            1_655_487,
            16_200,
            "589d6d3ddbe3959b167db1eb83a5d8d2301df67764f3ba405dedcf4b92ce8714",
        );
        const AB_L1_SEED_U: Record = Record::new(
            "ab_l1_pre_pis_main_u",
            1_833_895,
            64_800,
            "0babe3dbe33360091772a1669c6c14b7670242500597a77b8fe2444edfd66f05",
        );
        const AB_L1_SEED_V: Record = Record::new(
            "ab_l1_pre_pis_main_v",
            1_898_727,
            64_800,
            "194bbea4e59230942844bd94da87b1d9dbb11079dc102ae0d2af6cfb94930965",
        );
        const BA_L2_U: Record = Record::new(
            "ba_l2_post_blend_main_u",
            2_810_096,
            16_200,
            "399da80a0f4cd232c88261f0029d37fc5ed3a7b78d268fa782a1cceefc803df8",
        );
        const BA_L2_V: Record = Record::new(
            "ba_l2_post_blend_main_v",
            2_826_331,
            16_200,
            "46d9a6c3b6742990ca051b6b3d7aa20f367e746fb03f919770c41f2a9166919e",
        );
        const BA_L1_SEED_U: Record = Record::new(
            "ba_l1_pre_pis_main_u",
            3_004_739,
            64_800,
            "86fbec53fc9b595eb778f4af0ea0f202d2a8180a3f9d3328abb96871b4bfc51c",
        );
        const BA_L1_SEED_V: Record = Record::new(
            "ba_l1_pre_pis_main_v",
            3_069_571,
            64_800,
            "50259cd5a3670225692f009c7e149a9777fcbbd2c560fb25e5720b29096ad454",
        );
        const AB_L1_U: Record = Record::new(
            "ab_l1_post_blend_main_u",
            2_531_944,
            64_800,
            "16c94ff6fa7ed56650948be2adae7de62dd3e361f44253de0935ed9888aa23d0",
        );
        const AB_L1_V: Record = Record::new(
            "ab_l1_post_blend_main_v",
            2_596_779,
            64_800,
            "4c45b8796ed084376cc6ab0a96d68ec1f20b5e3440497a0557c25a2707fd412d",
        );
        const AB_PUBLIC: Record = Record::new(
            "shared_pre_cpu_blend_public_c30_ab",
            453_889,
            518_400,
            "fb738200390cbe0f044057e318e06a93227424b2ec40ce359e40c93241d6b315",
        );
        const BA_L1_U: Record = Record::new(
            "ba_l1_post_blend_main_u",
            3_702_788,
            64_800,
            "2a2c9deb373ae1e902fa55a1214214139dfb52ce7e76bb62d140f308a2f680a5",
        );
        const BA_L1_V: Record = Record::new(
            "ba_l1_post_blend_main_v",
            3_767_623,
            64_800,
            "dfa51967565857b29dab2d7358fb8fe22364679d929abafc6fe9fb3fe0dae6bb",
        );
        const BA_PUBLIC: Record = Record::new(
            "shared_pre_cpu_blend_public_c90_ba",
            972_335,
            518_400,
            "518c6cb262053bb0e5e1191ae62814c632181fc1c9ab34e7948f78bb6641e16b",
        );

        let path = std::env::var_os("KJERAG_ONE_XS_WARM_PAYLOAD")
            .expect("set KJERAG_ONE_XS_WARM_PAYLOAD to authenticated run-06 payload");
        let payload = std::fs::read(path).unwrap();
        assert_eq!(payload.len(), PAYLOAD_BYTES);
        assert_eq!(&payload[..8], b"KJWP602\x04");
        assert_eq!(sha256_hex(&payload), PAYLOAD_SHA256);

        assert_l2_resize::<AtoB>(&payload, [AB_L2_U, AB_L2_V], [AB_L1_SEED_U, AB_L1_SEED_V]);
        assert_l2_resize::<BtoA>(&payload, [BA_L2_U, BA_L2_V], [BA_L1_SEED_U, BA_L1_SEED_V]);
        assert_public_resize::<AtoB>(&payload, [AB_L1_U, AB_L1_V], AB_PUBLIC);
        assert_public_resize::<BtoA>(&payload, [BA_L1_U, BA_L1_V], BA_PUBLIC);

        fn assert_l2_resize<D: PisDirection>(
            payload: &[u8],
            source: [Record; 2],
            expected: [Record; 2],
        ) {
            let source_u = record_f32(payload, source[0]);
            let source_v = record_f32(payload, source[1]);
            let field =
                DenseField::<D>::from_test_components(Level::Two, source_u, source_v).unwrap();
            let resized =
                upsample_linear_x2(PostUpdateDense::from_test_field(field, false)).unwrap();
            assert_f32_record(payload, expected[0], resized.dcol());
            assert_f32_record(payload, expected[1], resized.drow());
        }

        fn assert_public_resize<D: PisDirection>(
            payload: &[u8],
            source: [Record; 2],
            expected: Record,
        ) {
            let source_u = record_f32(payload, source[0]);
            let source_v = record_f32(payload, source[1]);
            let field =
                DenseField::<D>::from_test_components(Level::One, source_u, source_v).unwrap();
            let resized = finish_linear_x2(PostUpdateDense::from_test_field(field, false)).unwrap();
            let mut interleaved = Vec::with_capacity(ROWS * COLS * 8);
            for pixel in 0..ROWS * COLS {
                interleaved.extend_from_slice(&resized.dcol()[pixel].to_le_bytes());
                interleaved.extend_from_slice(&resized.drow()[pixel].to_le_bytes());
            }
            assert_eq!(interleaved, record_bytes(payload, expected));
        }

        fn assert_f32_record(payload: &[u8], expected: Record, actual: &[f32]) {
            let actual = actual
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>();
            assert_eq!(actual, record_bytes(payload, expected));
        }

        fn record_f32(payload: &[u8], record: Record) -> Vec<f32> {
            record_bytes(payload, record)
                .chunks_exact(4)
                .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
                .collect()
        }

        fn record_bytes(payload: &[u8], record: Record) -> &[u8] {
            let label_start = record.offset.checked_sub(record.label.len()).unwrap();
            let header_start = label_start.checked_sub(12).unwrap();
            let end = record.offset.checked_add(record.bytes).unwrap();
            assert!(end <= payload.len());
            assert_eq!(
                u32::from_le_bytes(payload[header_start..header_start + 4].try_into().unwrap()),
                record.label.len() as u32,
            );
            assert_eq!(
                u64::from_le_bytes(payload[header_start + 4..label_start].try_into().unwrap()),
                record.bytes as u64,
            );
            assert_eq!(
                &payload[label_start..record.offset],
                record.label.as_bytes()
            );
            let bytes = &payload[record.offset..end];
            assert_eq!(sha256_hex(bytes), record.sha256);
            bytes
        }

        fn sha256_hex(bytes: &[u8]) -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }
    }

    #[derive(Clone, Copy)]
    struct Record {
        label: &'static str,
        offset: usize,
        bytes: usize,
        sha256: &'static str,
    }

    impl Record {
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

    #[test]
    fn checked_component_adapter_accepts_native_nan_but_rejects_bad_shape() {
        let level = Level::Two;
        let mut dcol = vec![0.0; level.pixels()];
        dcol[0] = NO_COVER_NAN;
        let field =
            DenseField::<AtoB>::from_test_components(level, dcol, vec![0.0; level.pixels()])
                .unwrap();
        assert_eq!(field.dcol()[0].to_bits(), NO_COVER_NAN.to_bits());

        let error = DenseField::<AtoB>::from_test_components(
            level,
            vec![0.0; level.pixels() - 1],
            vec![0.0; level.pixels()],
        )
        .unwrap_err();
        assert_eq!(error.actual, level.pixels() - 1);
    }

    #[test]
    fn public_restore_adapters_check_shape_axes_and_direction() {
        let pixels = ROWS * COLS;
        let mut interleaved = vec![[0.0, 0.0]; pixels];
        interleaved[0] = [NO_COVER_NAN, -7.5];
        interleaved[pixels - 1] = [3.25, 11.0];
        let field = PublicDenseField::<BtoA>::from_row_major_interleaved(interleaved).unwrap();
        assert_eq!(field.direction(), Direction::BtoA);
        assert_eq!(field.dcol()[0].to_bits(), NO_COVER_NAN.to_bits());
        assert_eq!(field.drow()[0], -7.5);
        assert_eq!(field.dcol()[pixels - 1], 3.25);
        assert_eq!(field.drow()[pixels - 1], 11.0);

        let error = PublicDenseField::<AtoB>::from_row_major_components(
            vec![0.0; pixels],
            vec![0.0; pixels - 1],
        )
        .unwrap_err();
        assert_eq!(error.direction, Direction::AtoB);
        assert_eq!(error.level, None);
        assert_eq!(error.expected, pixels);
        assert_eq!(error.actual, pixels - 1);

        let error =
            PublicDenseField::<BtoA>::from_row_major_interleaved(vec![[0.0; 2]; pixels + 1])
                .unwrap_err();
        assert_eq!(error.direction, Direction::BtoA);
        assert_eq!(error.expected, pixels);
        assert_eq!(error.actual, pixels + 1);
    }
}
