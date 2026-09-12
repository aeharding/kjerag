//! Inactive scalar derivative preparation for selected ONE X2 variational flow.
//!
//! Native converts the target image to binary32, warps it once with OpenCV's
//! 32-phase linear remap and repeated borders, then prepares the eight logical
//! derivative planes consumed by [`super::variational`]. This module preserves
//! that operation graph and the selected U8 intensity scale. It deliberately
//! does not claim OpenCV arm64 arithmetic or Mac bit identity.
//!
//! Selected densification extends the final sparse patch across the residual
//! bottom and right fringe, so it does not normally produce a qNaN tail. The
//! narrowly scoped canonical-tail handling below remains as a defensive test
//! of the generic no-cover sentinel: the converter turns `0x7fc00000` into base
//! 0, phase 0 on both axes, so the warped sample is exactly the target's
//! top-left pixel and the dense field keeps its qNaNs. Every other non-finite or
//! out-of-range map coordinate still returns [`PrepareError`].

use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

use super::dense::DenseField;
use super::pis::{Level, PisDirection};
use super::variational::PreparedDerivatives;
use super::{Direction, Lens, LensPair};

const INTER_BITS: u32 = 5;
const INTER_TAB_SIZE: i32 = 1 << INTER_BITS;
const INTER_TAB_SCALE: f32 = 1.0 / INTER_TAB_SIZE as f32;

/// One U8 image view with an explicit byte stride between rows.
///
/// Construction does not yet know a selected pyramid level. The typed
/// [`PreparationImages`] boundary validates both the stride and reachable
/// storage against its level before admitting the view.
#[derive(Clone, Copy, Debug)]
pub struct StridedImage<'a> {
    values: &'a [u8],
    row_stride: usize,
}

impl<'a> StridedImage<'a> {
    pub const fn new(values: &'a [u8], row_stride: usize) -> Self {
        Self { values, row_stride }
    }

    pub const fn row_stride(self) -> usize {
        self.row_stride
    }

    pub const fn storage_len(self) -> usize {
        self.values.len()
    }

    fn get(self, row: usize, col: usize) -> u8 {
        self.values[row * self.row_stride + col]
    }
}

/// Direction-labelled current and target images for one derivative build.
///
/// Callers provide physical A/B order. A-to-B uses A as `I0` and B as `I1`;
/// B-to-A uses B as `I0` and A as `I1`. Both image views must cover the exact
/// selected level, though trailing allocation bytes are permitted.
#[derive(Debug)]
pub struct PreparationImages<'a, D: PisDirection> {
    level: Level,
    current: StridedImage<'a>,
    target: StridedImage<'a>,
    direction: PhantomData<D>,
}

impl<'a, D: PisDirection> PreparationImages<'a, D> {
    pub fn from_native_order(
        level: Level,
        images: LensPair<StridedImage<'a>>,
    ) -> Result<Self, ImageShapeError> {
        let LensPair { a, b } = images;
        validate_image::<D>(level, Lens::A, a)?;
        validate_image::<D>(level, Lens::B, b)?;
        let (current, target) = match D::DIRECTION {
            Direction::AtoB => (a, b),
            Direction::BtoA => (b, a),
        };
        Ok(Self {
            level,
            current,
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

/// Why one strided U8 image cannot cover its selected logical plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageShapeError {
    RowStride {
        direction: Direction,
        level: Level,
        lens: Lens,
        minimum: usize,
        actual: usize,
    },
    Storage {
        direction: Direction,
        level: Level,
        lens: Lens,
        minimum: usize,
        actual: usize,
    },
    StorageSizeOverflow {
        direction: Direction,
        level: Level,
        lens: Lens,
        row_stride: usize,
    },
}

impl fmt::Display for ImageShapeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::RowStride {
                direction,
                level,
                lens,
                minimum,
                actual,
            } => write!(
                out,
                "ONE X2 {direction} {level} physical lens {lens} image has row stride {actual}, expected at least {minimum}",
            ),
            Self::Storage {
                direction,
                level,
                lens,
                minimum,
                actual,
            } => write!(
                out,
                "ONE X2 {direction} {level} physical lens {lens} image exposes {actual} bytes, expected at least {minimum}",
            ),
            Self::StorageSizeOverflow {
                direction,
                level,
                lens,
                row_stride,
            } => write!(
                out,
                "ONE X2 {direction} {level} physical lens {lens} image row stride {row_stride} cannot address the selected plane",
            ),
        }
    }
}

impl Error for ImageShapeError {}

fn validate_image<D: PisDirection>(
    level: Level,
    lens: Lens,
    image: StridedImage<'_>,
) -> Result<(), ImageShapeError> {
    let cols = level.cols();
    if image.row_stride < cols {
        return Err(ImageShapeError::RowStride {
            direction: D::DIRECTION,
            level,
            lens,
            minimum: cols,
            actual: image.row_stride,
        });
    }
    let Some(required) = (level.rows() - 1)
        .checked_mul(image.row_stride)
        .and_then(|offset| offset.checked_add(cols))
    else {
        return Err(ImageShapeError::StorageSizeOverflow {
            direction: D::DIRECTION,
            level,
            lens,
            row_stride: image.row_stride,
        });
    };
    if image.values.len() < required {
        return Err(ImageShapeError::Storage {
            direction: D::DIRECTION,
            level,
            lens,
            minimum: required,
            actual: image.values.len(),
        });
    }
    Ok(())
}

/// The map component whose finite-domain contract was violated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapAxis {
    Column,
    Row,
}

impl fmt::Display for MapAxis {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Column => out.write_str("column"),
            Self::Row => out.write_str("row"),
        }
    }
}

/// A derivative preparation cannot be formed within the closed scalar domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrepareError {
    WrongLevel {
        direction: Direction,
        images: Level,
        field: Level,
    },
    NonFiniteCoordinate {
        direction: Direction,
        level: Level,
        row: usize,
        col: usize,
        axis: MapAxis,
        value_bits: u32,
    },
    CoordinateOutOfRange {
        direction: Direction,
        level: Level,
        row: usize,
        col: usize,
        axis: MapAxis,
        value_bits: u32,
    },
}

impl fmt::Display for PrepareError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::WrongLevel {
                direction,
                images,
                field,
            } => write!(
                out,
                "ONE X2 {direction} derivative images are {images}, but the dense field is {field}",
            ),
            Self::NonFiniteCoordinate {
                direction,
                level,
                row,
                col,
                axis,
                value_bits,
            } => write!(
                out,
                "ONE X2 {direction} {level} remap {axis} coordinate at row {row}, column {col} is non-finite ({:?})",
                f32::from_bits(value_bits),
            ),
            Self::CoordinateOutOfRange {
                direction,
                level,
                row,
                col,
                axis,
                value_bits,
            } => write!(
                out,
                "ONE X2 {direction} {level} remap {axis} coordinate at row {row}, column {col} is outside the closed 32-bit map domain ({:?})",
                f32::from_bits(value_bits),
            ),
        }
    }
}

impl Error for PrepareError {}

/// One derivative set bundled with the exact dense field used for its warp.
///
/// Only [`prepare`] can construct this value. Consuming the dense token here
/// prevents the normal preparation path from refining a different same-level
/// field or reusing this preparation for a later epoch.
#[must_use = "the prepared field still needs variational refinement"]
#[derive(Debug)]
pub struct PreparedRefinement<D: PisDirection> {
    derivatives: PreparedDerivatives<D>,
    field: DenseField<D>,
}

impl<D: PisDirection> PreparedRefinement<D> {
    pub const fn level(&self) -> Level {
        self.field.level()
    }

    pub const fn direction(&self) -> Direction {
        D::DIRECTION
    }

    pub const fn field(&self) -> &DenseField<D> {
        &self.field
    }

    pub(super) fn into_parts(self) -> (PreparedDerivatives<D>, DenseField<D>) {
        (self.derivatives, self.field)
    }
}

/// Prepare the eight logical f32 planes and bind them to their dense epoch.
///
/// `mapX=x+dcol` and `mapY=y+drow` use OpenCV's five-bit interpolation phase,
/// nearest-even coordinate quantization, and `BORDER_REPLICATE`. The target is
/// converted from U8 before interpolation. The current image remains U8 until
/// the f32 average and subtraction, preserving native's 0-to-255 scale.
///
/// The consumed dense token cannot be reused after preparation:
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::dense::DenseField;
/// use kjerag_render::flow::one_xs::derivative_prep::{PreparationImages, prepare};
/// use kjerag_render::flow::one_xs::pis::AtoB;
/// use kjerag_render::flow::one_xs::variational::refine_prepared;
///
/// fn reuse(images: &PreparationImages<'_, AtoB>, field: DenseField<AtoB>) {
///     let prepared = prepare(images, field).unwrap();
///     let _ = field.level();
///     let _ = refine_prepared(prepared);
/// }
/// ```
pub fn prepare<D: PisDirection>(
    images: &PreparationImages<'_, D>,
    field: DenseField<D>,
) -> Result<PreparedRefinement<D>, PrepareError> {
    if images.level != field.level() {
        return Err(PrepareError::WrongLevel {
            direction: D::DIRECTION,
            images: images.level,
            field: field.level(),
        });
    }

    let shape = Shape {
        rows: images.level.rows(),
        cols: images.level.cols(),
    };
    let context = Context {
        direction: D::DIRECTION,
        level: images.level,
    };
    let planes = prepare_planes(
        images.current,
        images.target,
        shape,
        field.dcol(),
        field.drow(),
        context,
    )?;
    let derivatives = PreparedDerivatives::from_components(
        images.level,
        planes.ix,
        planes.iy,
        planes.iz,
        planes.ixx,
        planes.ixy,
        planes.iyy,
        planes.ixz,
        planes.iyz,
    )
    .expect("the derivative graph preserves the selected logical shape");
    Ok(PreparedRefinement { derivatives, field })
}

#[derive(Clone, Copy)]
struct Shape {
    rows: usize,
    cols: usize,
}

impl Shape {
    const fn pixels(self) -> usize {
        self.rows * self.cols
    }
}

#[derive(Clone, Copy)]
struct Context {
    direction: Direction,
    level: Level,
}

struct Planes {
    ix: Vec<f32>,
    iy: Vec<f32>,
    iz: Vec<f32>,
    ixx: Vec<f32>,
    ixy: Vec<f32>,
    iyy: Vec<f32>,
    ixz: Vec<f32>,
    iyz: Vec<f32>,
}

fn prepare_planes(
    current: StridedImage<'_>,
    target: StridedImage<'_>,
    shape: Shape,
    dcol: &[f32],
    drow: &[f32],
    context: Context,
) -> Result<Planes, PrepareError> {
    debug_assert_eq!(dcol.len(), shape.pixels());
    debug_assert_eq!(drow.len(), shape.pixels());

    // Native explicitly converts only I1 to one continuous CV32F Mat.
    let mut target_f32 = Vec::with_capacity(shape.pixels());
    for row in 0..shape.rows {
        for col in 0..shape.cols {
            target_f32.push(f32::from(target.get(row, col)));
        }
    }

    let mut mean = Vec::with_capacity(shape.pixels());
    let mut iz = Vec::with_capacity(shape.pixels());
    for row in 0..shape.rows {
        for col in 0..shape.cols {
            let pixel = row * shape.cols + col;
            let warped = if canonical_tail_sentinel(shape, row, col, dcol[pixel], drow[pixel]) {
                // Section 114N: both axes quantize to base 0, phase 0, and the
                // replicate border leaves the bilinear sample on the top-left pixel.
                target_f32[0]
            } else {
                let map_col = col as f32 + dcol[pixel];
                let map_row = row as f32 + drow[pixel];
                sample_linear_replicate(&target_f32, shape, map_row, map_col, context, row, col)?
            };
            let reference = f32::from(current.get(row, col));
            mean.push(reference * 0.5 + warped * 0.5);
            iz.push(warped - reference);
        }
    }

    // Native's first independent Sobel batch.
    let ix = sobel_x(&mean, shape);
    let iy = sobel_y(&mean, shape);
    let ixz = sobel_x(&iz, shape);
    let iyz = sobel_y(&iz, shape);

    // Native waits for that batch, then launches these three operations.
    let ixx = sobel_x(&ix, shape);
    let ixy = sobel_y(&ix, shape);
    let iyy = sobel_y(&iy, shape);

    Ok(Planes {
        ix,
        iy,
        iz,
        ixx,
        ixy,
        iyy,
        ixz,
        iyz,
    })
}

/// Bit pattern used by the generic densifier's no-cover fallback.
const CANONICAL_QNAN: u32 = 0x7fc0_0000;

/// Whether this pixel is the exact defensive tail sentinel accepted here.
///
/// The bundled remap converter quantizes a canonical qNaN to base 0, phase 0 on
/// both axes, so the sample is the target's top-left pixel identically in
/// OpenCV's SIMD and scalar columns. At the selected 15 and 30 wide grids the
/// final column is always scalar, so both lane classes are exercised across the
/// synthetic tail. The admission is deliberately narrow: the cell must lie in
/// the final row or final column, and BOTH map components must carry that exact
/// bit pattern. Interior sentinels, one-axis sentinels, other NaN encodings or
/// signs, and infinities stay rejected because their conversion differs between
/// vector and scalar lanes.
fn canonical_tail_sentinel(shape: Shape, row: usize, col: usize, dcol: f32, drow: f32) -> bool {
    let tail = row + 1 == shape.rows || col + 1 == shape.cols;
    tail && dcol.to_bits() == CANONICAL_QNAN && drow.to_bits() == CANONICAL_QNAN
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QuantizedCoordinate {
    base: i32,
    phase: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QuantizeError {
    NonFinite(u32),
    OutOfRange(u32),
}

fn quantize_coordinate(value: f32) -> Result<QuantizedCoordinate, QuantizeError> {
    if !value.is_finite() {
        return Err(QuantizeError::NonFinite(value.to_bits()));
    }
    let scaled = value * INTER_TAB_SIZE as f32;
    if !scaled.is_finite() {
        return Err(QuantizeError::OutOfRange(value.to_bits()));
    }
    let rounded = scaled.round_ties_even();
    let rounded_f64 = f64::from(rounded);
    if rounded_f64 < f64::from(i32::MIN) || rounded_f64 > f64::from(i32::MAX) {
        return Err(QuantizeError::OutOfRange(value.to_bits()));
    }
    let fixed = rounded as i32;
    Ok(QuantizedCoordinate {
        base: fixed >> INTER_BITS,
        phase: (fixed & (INTER_TAB_SIZE - 1)) as usize,
    })
}

#[allow(clippy::too_many_arguments)]
fn sample_linear_replicate(
    image: &[f32],
    shape: Shape,
    row_coordinate: f32,
    col_coordinate: f32,
    context: Context,
    output_row: usize,
    output_col: usize,
) -> Result<f32, PrepareError> {
    let col = quantize_coordinate(col_coordinate).map_err(|error| {
        coordinate_error(error, context, output_row, output_col, MapAxis::Column)
    })?;
    let row = quantize_coordinate(row_coordinate)
        .map_err(|error| coordinate_error(error, context, output_row, output_col, MapAxis::Row))?;

    let col0 = repeated_index(col.base, shape.cols);
    let col1 = repeated_index(col.base.saturating_add(1), shape.cols);
    let row0 = repeated_index(row.base, shape.rows);
    let row1 = repeated_index(row.base.saturating_add(1), shape.rows);
    let col_fraction = col.phase as f32 * INTER_TAB_SCALE;
    let row_fraction = row.phase as f32 * INTER_TAB_SCALE;
    let col_inverse = 1.0 - col_fraction;
    let row_inverse = 1.0 - row_fraction;

    let top = image[row0 * shape.cols + col0] * col_inverse
        + image[row0 * shape.cols + col1] * col_fraction;
    let bottom = image[row1 * shape.cols + col0] * col_inverse
        + image[row1 * shape.cols + col1] * col_fraction;
    Ok(top * row_inverse + bottom * row_fraction)
}

fn coordinate_error(
    error: QuantizeError,
    context: Context,
    row: usize,
    col: usize,
    axis: MapAxis,
) -> PrepareError {
    match error {
        QuantizeError::NonFinite(value_bits) => PrepareError::NonFiniteCoordinate {
            direction: context.direction,
            level: context.level,
            row,
            col,
            axis,
            value_bits,
        },
        QuantizeError::OutOfRange(value_bits) => PrepareError::CoordinateOutOfRange {
            direction: context.direction,
            level: context.level,
            row,
            col,
            axis,
            value_bits,
        },
    }
}

fn repeated_index(index: i32, len: usize) -> usize {
    index.clamp(0, len as i32 - 1) as usize
}

fn sobel_x(source: &[f32], shape: Shape) -> Vec<f32> {
    let mut output = vec![0.0; shape.pixels()];
    for row in 0..shape.rows {
        for col in 0..shape.cols {
            let left = col.saturating_sub(1);
            let right = (col + 1).min(shape.cols - 1);
            output[row * shape.cols + col] =
                source[row * shape.cols + right] - source[row * shape.cols + left];
        }
    }
    output
}

fn sobel_y(source: &[f32], shape: Shape) -> Vec<f32> {
    let mut output = vec![0.0; shape.pixels()];
    for row in 0..shape.rows {
        let above = row.saturating_sub(1);
        let below = (row + 1).min(shape.rows - 1);
        for col in 0..shape.cols {
            output[row * shape.cols + col] =
                source[below * shape.cols + col] - source[above * shape.cols + col];
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::one_xs::pis::{AtoB, BtoA};
    use crate::flow::one_xs::variational::refine_prepared;

    const TEST_CONTEXT: Context = Context {
        direction: Direction::AtoB,
        level: Level::Two,
    };

    fn packed_u8(rows: &[&[u8]], stride: usize, padding: u8) -> Vec<u8> {
        let mut storage = vec![padding; rows.len() * stride];
        for (row, values) in rows.iter().enumerate() {
            storage[row * stride..row * stride + values.len()].copy_from_slice(values);
        }
        storage
    }

    #[test]
    fn strided_inputs_validate_reachable_storage_and_direction() {
        let level = Level::Two;
        let stride = level.cols() + 3;
        let required = (level.rows() - 1) * stride + level.cols();
        let mut a = vec![0; required];
        let mut b = vec![0; required];
        a[0] = 11;
        b[0] = 29;
        let images = PreparationImages::<BtoA>::from_native_order(
            level,
            LensPair {
                a: StridedImage::new(&a, stride),
                b: StridedImage::new(&b, stride),
            },
        )
        .unwrap();
        assert_eq!(images.direction(), Direction::BtoA);
        assert_eq!(images.current.get(0, 0), 29);
        assert_eq!(images.target.get(0, 0), 11);

        let stride_error = PreparationImages::<AtoB>::from_native_order(
            level,
            LensPair {
                a: StridedImage::new(&a, level.cols() - 1),
                b: StridedImage::new(&b, stride),
            },
        )
        .unwrap_err();
        assert!(matches!(
            stride_error,
            ImageShapeError::RowStride {
                lens: Lens::A,
                minimum,
                actual,
                ..
            } if minimum == level.cols() && actual == level.cols() - 1
        ));

        let short = vec![0; required - 1];
        let storage_error = PreparationImages::<AtoB>::from_native_order(
            level,
            LensPair {
                a: StridedImage::new(&short, stride),
                b: StridedImage::new(&b, stride),
            },
        )
        .unwrap_err();
        assert!(matches!(
            storage_error,
            ImageShapeError::Storage {
                lens: Lens::A,
                minimum,
                actual,
                ..
            } if minimum == required && actual == required - 1
        ));
    }

    #[test]
    fn remap_uses_asymmetric_five_bit_phases_and_nearest_even_ties() {
        assert_eq!(
            quantize_coordinate(2.5 / 32.0).unwrap(),
            QuantizedCoordinate { base: 0, phase: 2 },
        );
        assert_eq!(
            quantize_coordinate(3.5 / 32.0).unwrap(),
            QuantizedCoordinate { base: 0, phase: 4 },
        );
        assert_eq!(
            quantize_coordinate(-2.5 / 32.0).unwrap(),
            QuantizedCoordinate {
                base: -1,
                phase: 30,
            },
        );

        let value = sample_linear_replicate(
            &[0.0, 32.0, 64.0, 96.0],
            Shape { rows: 2, cols: 2 },
            5.5 / 32.0,
            2.5 / 32.0,
            TEST_CONTEXT,
            0,
            0,
        )
        .unwrap();
        assert_eq!(value.to_bits(), 14.0f32.to_bits());
        assert!(matches!(
            quantize_coordinate(f32::MAX),
            Err(QuantizeError::OutOfRange(_))
        ));
    }

    #[test]
    fn remap_replicates_each_bilinear_neighbour_at_every_border() {
        let image = [10.0, 20.0, 30.0, 40.0, 50.0, 60.0];
        let shape = Shape { rows: 2, cols: 3 };
        let left_of_top =
            sample_linear_replicate(&image, shape, -0.25, 1.5, TEST_CONTEXT, 0, 0).unwrap();
        let right_edge =
            sample_linear_replicate(&image, shape, 0.5, 2.25, TEST_CONTEXT, 0, 0).unwrap();
        let far_corner =
            sample_linear_replicate(&image, shape, -100.0, 100.0, TEST_CONTEXT, 0, 0).unwrap();
        assert_eq!(left_of_top.to_bits(), 25.0f32.to_bits());
        assert_eq!(right_edge.to_bits(), 45.0f32.to_bits());
        assert_eq!(far_corner.to_bits(), 30.0f32.to_bits());
    }

    #[test]
    fn average_and_residual_keep_u8_scale_stride_and_warp_minus_current_sign() {
        let shape = Shape { rows: 2, cols: 3 };
        let current_storage = packed_u8(&[&[10, 30, 50], &[70, 90, 110]], 5, 251);
        let target_storage = packed_u8(&[&[14, 34, 54], &[74, 94, 114]], 6, 253);
        let planes = prepare_planes(
            StridedImage::new(&current_storage, 5),
            StridedImage::new(&target_storage, 6),
            shape,
            &[0.0; 6],
            &[0.0; 6],
            TEST_CONTEXT,
        )
        .unwrap();
        assert_eq!(planes.iz, vec![4.0; 6]);

        let expected_mean = [12.0, 32.0, 52.0, 72.0, 92.0, 112.0];
        let expected_ix = sobel_x(&expected_mean, shape);
        let expected_iy = sobel_y(&expected_mean, shape);
        assert_eq!(planes.ix, expected_ix);
        assert_eq!(planes.iy, expected_iy);
    }

    #[test]
    fn ksize_one_sobel_has_no_half_scale_and_replicates_each_pass() {
        let shape = Shape { rows: 3, cols: 4 };
        let values = [
            1.0, 4.0, 9.0, 16.0, 21.0, 24.0, 29.0, 36.0, 61.0, 64.0, 69.0, 76.0,
        ];
        assert_eq!(
            sobel_x(&values, shape),
            vec![
                3.0, 8.0, 12.0, 7.0, 3.0, 8.0, 12.0, 7.0, 3.0, 8.0, 12.0, 7.0
            ]
        );
        assert_eq!(
            sobel_y(&values, shape),
            vec![
                20.0, 20.0, 20.0, 20.0, 60.0, 60.0, 60.0, 60.0, 40.0, 40.0, 40.0, 40.0
            ]
        );
    }

    #[test]
    fn ixy_is_vertical_sobel_of_ix_in_native_f32_grouping() {
        let shape = Shape { rows: 3, cols: 3 };
        let mut mean = vec![0.0; shape.pixels()];
        mean[2] = f32::from_bits(0x2eeb_f2b2);
        mean[0] = f32::from_bits(0x4ed5_4f95);
        mean[8] = f32::from_bits(0xca06_db94);
        mean[6] = f32::from_bits(0xc4b5_9c0c);
        let ix = sobel_x(&mean, shape);
        let ixy = sobel_y(&ix, shape);
        let alternative = sobel_x(&sobel_y(&mean, shape), shape);
        assert_eq!(ixy[4].to_bits(), 0x4ed5_0c33);
        assert_eq!(alternative[4].to_bits(), 0x4ed5_0c32);
        assert_ne!(ixy[4].to_bits(), alternative[4].to_bits());
    }

    #[test]
    fn second_derivatives_are_sequential_first_derivatives() {
        let shape = Shape { rows: 1, cols: 5 };
        let mean = [0.0, 1.0, 4.0, 9.0, 16.0];
        let ix = sobel_x(&mean, shape);
        let ixx = sobel_x(&ix, shape);
        assert_eq!(ix, vec![1.0, 4.0, 8.0, 12.0, 7.0]);
        assert_eq!(ixx[2].to_bits(), 8.0f32.to_bits());
        assert_ne!(ixx[2].to_bits(), 2.0f32.to_bits());
        assert_eq!(sobel_y(&sobel_y(&mean, shape), shape), vec![0.0; 5]);
    }

    #[test]
    fn public_preparation_rejects_nonfinite_maps_and_owns_the_finite_epoch() {
        let level = Level::Two;
        let a = vec![17; level.pixels()];
        let b = vec![17; level.pixels()];
        let images = PreparationImages::<AtoB>::from_native_order(
            level,
            LensPair {
                a: StridedImage::new(&a, level.cols()),
                b: StridedImage::new(&b, level.cols()),
            },
        )
        .unwrap();

        let mut invalid_dcol = vec![0.0; level.pixels()];
        invalid_dcol[7] = f32::from_bits(0x7fc0_0000);
        let invalid = DenseField::<AtoB>::from_test_components(
            level,
            invalid_dcol,
            vec![0.0; level.pixels()],
        )
        .unwrap();
        let error = prepare(&images, invalid).unwrap_err();
        assert!(matches!(
            error,
            PrepareError::NonFiniteCoordinate {
                row: 0,
                col: 7,
                axis: MapAxis::Column,
                value_bits: 0x7fc0_0000,
                ..
            }
        ));

        let finite = DenseField::<AtoB>::from_test_components(
            level,
            vec![0.0; level.pixels()],
            vec![0.0; level.pixels()],
        )
        .unwrap();
        let prepared = prepare(&images, finite).unwrap();
        assert_eq!(prepared.level(), level);
        assert_eq!(prepared.direction(), Direction::AtoB);
        let refinement = refine_prepared(prepared);
        assert!(!refinement.rolled_back());
        assert!(refinement.field().dcol().iter().all(|value| *value == 0.0));
        assert!(refinement.field().drow().iter().all(|value| *value == 0.0));
    }

    fn canonical_tail_planes(level: Level) -> (Vec<f32>, Vec<f32>) {
        let mut dcol = vec![0.0; level.pixels()];
        let mut drow = vec![0.0; level.pixels()];
        for row in 0..level.rows() {
            for col in 0..level.cols() {
                if row + 1 == level.rows() || col + 1 == level.cols() {
                    let pixel = row * level.cols() + col;
                    dcol[pixel] = f32::from_bits(CANONICAL_QNAN);
                    drow[pixel] = f32::from_bits(CANONICAL_QNAN);
                }
            }
        }
        (dcol, drow)
    }

    fn prepare_with(level: Level, dcol: Vec<f32>, drow: Vec<f32>) -> Result<(), PrepareError> {
        let a = vec![17; level.pixels()];
        let b = vec![29; level.pixels()];
        let images = PreparationImages::<AtoB>::from_native_order(
            level,
            LensPair {
                a: StridedImage::new(&a, level.cols()),
                b: StridedImage::new(&b, level.cols()),
            },
        )
        .unwrap();
        let field = DenseField::<AtoB>::from_test_components(level, dcol, drow).unwrap();
        prepare(&images, field).map(|_| ())
    }

    #[test]
    fn canonical_tail_prepares_at_both_selected_levels() {
        for level in [Level::Two, Level::One] {
            let (dcol, drow) = canonical_tail_planes(level);
            prepare_with(level, dcol, drow).expect("canonical fallback fixture must prepare");
        }
    }

    #[test]
    fn canonical_tail_samples_the_target_top_left_across_the_whole_tail() {
        // The canonical fallback quantizes to base 0, phase 0 on both axes in
        // OpenCV's SIMD columns and in the always-scalar final column alike, so
        // every cell of the synthetic tail reads the target's top-left pixel.
        let level = Level::Two;
        let shape = Shape {
            rows: level.rows(),
            cols: level.cols(),
        };
        let mut target = vec![0u8; level.pixels()];
        target[0] = 200;
        let current = vec![0u8; level.pixels()];
        let (dcol, drow) = canonical_tail_planes(level);
        let planes = prepare_planes(
            StridedImage::new(&current, shape.cols),
            StridedImage::new(&target, shape.cols),
            shape,
            &dcol,
            &drow,
            TEST_CONTEXT,
        )
        .unwrap();

        let mut tail = 0;
        for row in 0..shape.rows {
            for col in 0..shape.cols {
                let pixel = row * shape.cols + col;
                if row + 1 == shape.rows || col + 1 == shape.cols {
                    assert_eq!(
                        planes.iz[pixel].to_bits(),
                        200.0f32.to_bits(),
                        "tail ({row},{col}) must sample the target top-left pixel",
                    );
                    tail += 1;
                } else if pixel != 0 {
                    // (0,0) legitimately reads the same pixel through a zero map.
                    assert_eq!(planes.iz[pixel].to_bits(), 0.0f32.to_bits());
                }
            }
        }
        assert_eq!(tail, shape.rows + shape.cols - 1);
    }

    #[test]
    fn canonical_tail_is_preserved_in_the_retained_dense_field() {
        let level = Level::Two;
        let a = vec![17; level.pixels()];
        let b = vec![29; level.pixels()];
        let images = PreparationImages::<AtoB>::from_native_order(
            level,
            LensPair {
                a: StridedImage::new(&a, level.cols()),
                b: StridedImage::new(&b, level.cols()),
            },
        )
        .unwrap();
        let (dcol, drow) = canonical_tail_planes(level);
        let field = DenseField::<AtoB>::from_test_components(level, dcol, drow).unwrap();
        let prepared = prepare(&images, field).unwrap();

        let retained = prepared.field();
        for row in 0..level.rows() {
            for col in 0..level.cols() {
                if row + 1 == level.rows() || col + 1 == level.cols() {
                    let pixel = row * level.cols() + col;
                    assert_eq!(retained.dcol()[pixel].to_bits(), CANONICAL_QNAN);
                    assert_eq!(retained.drow()[pixel].to_bits(), CANONICAL_QNAN);
                }
            }
        }
    }

    #[test]
    fn admission_is_narrow_and_rejects_every_other_exceptional_map() {
        let level = Level::Two;
        let tail = (level.rows() - 1) * level.cols() + (level.cols() - 1);
        let interior = level.cols() + 1;
        let qnan = f32::from_bits(CANONICAL_QNAN);

        // An interior canonical pair is not a producer sentinel.
        let (mut dcol, mut drow) = (vec![0.0; level.pixels()], vec![0.0; level.pixels()]);
        dcol[interior] = qnan;
        drow[interior] = qnan;
        assert!(matches!(
            prepare_with(level, dcol, drow).unwrap_err(),
            PrepareError::NonFiniteCoordinate { .. }
        ));

        // One axis only, even in the tail, is not the pair native emits.
        let (mut dcol, drow) = (vec![0.0; level.pixels()], vec![0.0; level.pixels()]);
        dcol[tail] = qnan;
        assert!(matches!(
            prepare_with(level, dcol, drow).unwrap_err(),
            PrepareError::NonFiniteCoordinate { .. }
        ));

        // Other encodings and signs are lane-dependent in OpenCV and unread here.
        for bits in [0xffc0_0000u32, 0x7fa0_0000, 0x7f80_0000, 0xff80_0000] {
            let (mut dcol, mut drow) = (vec![0.0; level.pixels()], vec![0.0; level.pixels()]);
            dcol[tail] = f32::from_bits(bits);
            drow[tail] = f32::from_bits(bits);
            assert!(
                matches!(
                    prepare_with(level, dcol, drow).unwrap_err(),
                    PrepareError::NonFiniteCoordinate { .. }
                ),
                "bits {bits:#x} must stay rejected",
            );
        }

        // Finite but outside the converter's i32 domain stays rejected too.
        let (mut dcol, mut drow) = (vec![0.0; level.pixels()], vec![0.0; level.pixels()]);
        dcol[tail] = f32::MAX;
        drow[tail] = f32::MAX;
        assert!(matches!(
            prepare_with(level, dcol, drow).unwrap_err(),
            PrepareError::CoordinateOutOfRange { .. }
        ));
    }
}
