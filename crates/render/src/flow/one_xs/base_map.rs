//! Studio's selected ONE X2 pre-filter line-map composition.
//!
//! The two selected `getFisheye2LineMap` workers call
//! `ins::Transform::mapMerge` with one static coordinate Mat, one 100-row by
//! 200-column `CV_32FC2` flowstate ROI and `MAPTYPE = 0`. The selected static
//! and destination Mats are allocated as 1,080 rows by 60 columns. This
//! specialization nevertheless keeps all runtime ABI dimensions explicit.
//! The pinned Studio
//! 6.0.2 arm64 body at worker `+0x340df7c` ignores `MAPTYPE` and evaluates
//! `out = periodic_bilinear(flowstate_roi, static_coordinates)`. Both sampled
//! axes are periodic. Its scalar weight construction and one-multiply,
//! three-FMA accumulation order are retained below for the ordinary finite,
//! safe-index domain.
//!
//! Studio immediately mutates each result with `filterFisheye2LineMap`, also
//! implemented here as a distinct typed step. The selected static-coordinate
//! producer body and scalar instance are READ, and the target payload is
//! captured. [`one_xs_static_coordinates`] transcribes that producer below,
//! but deliberately does not claim that this host's transcendental
//! functions are bit-identical to macOS `libm` and the selected FPCR.
//! [`FilteredLineMap`] is still not a
//! [`RetainedBaseMaps`](crate::flow::one_xs_belt::RetainedBaseMaps):
//! the selected flowstate arithmetic is transcribed in `metal_calc_map`, but
//! Studio's upstream pose-cache producer remains unread and its general
//! per-frame owner remains unwired. This module is inactive and is not
//! connected to playback.

use std::error::Error;
use std::fmt;

use super::LensPair;

/// The selected flowstate parent is split into two 100-row by 200-column ROIs.
pub const SELECTED_FLOWSTATE_ROWS: usize = 100;
pub const SELECTED_FLOWSTATE_COLS: usize = 200;

/// The selected static and destination line Mats are 1,080 rows by 60 columns.
pub const SELECTED_LINE_ROWS: usize = 1080;
pub const SELECTED_LINE_COLS: usize = 60;

const MAX_EXACT_F32_DIMENSION: usize = 1 << f32::MANTISSA_DIGITS;
const MAX_NATIVE_NODES: usize = i32::MAX as usize / 2;
const FILTER_THRESHOLD: f32 = f32::from_bits(0x3ba3_d70a);
const FILTER_SENTINEL: f32 = f32::from_bits(0xc1a0_0000);
const STATIC_LOWER_DEGREES: f32 = -200.0;
const STATIC_UPPER_DEGREES: f32 = 200.0;
const STATIC_LATERAL_SCALE: f32 = 1.0;
const STATIC_CIRCLE_COUNT: f32 = 2.0;
const STATIC_PI: f32 = f32::from_bits(0x40c9_0fdb);
const STATIC_RADIANS_PER_DEGREE: f32 = f32::from_bits(0x3c8e_fa35);

/// The static per-lens coordinates consumed as map A by `mapMerge`.
#[derive(Clone, Debug, PartialEq)]
pub struct StaticLineCoordinates {
    rows: usize,
    cols: usize,
    values: Box<[[f32; 2]]>,
}

impl StaticLineCoordinates {
    /// Admit a tightly packed `[row][column]` float2 payload.
    pub fn from_row_major_values(
        rows: usize,
        cols: usize,
        values: Vec<[f32; 2]>,
    ) -> Result<Self, ShapeError> {
        validate_shape(MapKind::StaticCoordinates, rows, cols, values.len())?;
        Ok(Self {
            rows,
            cols,
            values: values.into_boxed_slice(),
        })
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn row_major_values(&self) -> &[[f32; 2]] {
        &self.values
    }
}

/// Generate the selected ONE X2 static sphere-to-line coordinates.
///
/// The dimensions, constants, selector values, rotation construction and
/// arithmetic schedule are READ from Studio 6.0.2 arm64. This reference uses
/// the current host's `sin`, `cos`, `atan2`, `acos` and remainder operations.
/// Those implement the recovered real-number law, but are not presumed to be
/// raw-bit equivalents of the selected macOS `libm` calls. The generated
/// values implement the READ producer semantics; captured bytes are not a
/// runtime dependency.
///
/// Lens A is native selector zero and lens B is native selector one.
pub fn one_xs_static_coordinates() -> LensPair<StaticLineCoordinates> {
    LensPair {
        a: one_xs_static_lens(StaticLens::A),
        b: one_xs_static_lens(StaticLens::B),
    }
}

#[derive(Clone, Copy)]
enum StaticLens {
    A,
    B,
}

fn one_xs_static_lens(lens: StaticLens) -> StaticLineCoordinates {
    let columns_minus_one = SELECTED_LINE_COLS - 1;
    let half_columns = columns_minus_one as f32 * 0.5;
    let row_step = (STATIC_UPPER_DEGREES - STATIC_LOWER_DEGREES) / (SELECTED_LINE_ROWS - 1) as f32;
    let scale = STATIC_LATERAL_SCALE * ((360.0 / row_step) / STATIC_PI);
    let sector = 360.0 / STATIC_CIRCLE_COUNT;
    let selector = match lens {
        StaticLens::A => 0.0,
        StaticLens::B => 1.0,
    };
    let center = sector * selector;
    let half = (180.0 - sector) * 0.5;
    let yaw_degrees = match lens {
        StaticLens::A => -(half + center),
        StaticLens::B => half - center,
    };
    let yaw_radians = (yaw_degrees % 360.0) * STATIC_RADIANS_PER_DEGREE;
    let (yaw_sine, yaw_cosine) = yaw_radians.sin_cos();
    let rotation = [
        [yaw_cosine, -yaw_sine, 0.0],
        [yaw_sine, yaw_cosine, 0.0],
        [0.0, 0.0, 1.0],
    ];

    let mut values = Vec::with_capacity(SELECTED_LINE_ROWS * SELECTED_LINE_COLS);
    for row in 0..SELECTED_LINE_ROWS {
        let theta_degrees = (row as f32).mul_add(row_step, STATIC_LOWER_DEGREES);
        let theta_radians = (f64::from(theta_degrees) * std::f64::consts::PI / 180.0) as f32;
        let (theta_sine, theta_cosine) = theta_radians.sin_cos();
        for column in 0..SELECTED_LINE_COLS {
            let across = match lens {
                StaticLens::A => (half_columns - column as f32) / scale,
                StaticLens::B => (column as f32 - half_columns) / scale,
            };
            let along = match lens {
                StaticLens::A => -theta_sine,
                StaticLens::B => theta_sine,
            };
            let input = [across, along, theta_cosine];
            let rotated = rotation.map(|matrix_row| {
                let value = matrix_row[0] * input[0];
                let value = matrix_row[1].mul_add(input[1], value);
                matrix_row[2].mul_add(input[2], value)
            });
            let norm_squared = rotated[1] * rotated[1];
            let norm_squared = rotated[0].mul_add(rotated[0], norm_squared);
            let norm_squared = rotated[2].mul_add(rotated[2], norm_squared);
            let norm = norm_squared.sqrt();
            let unit = rotated.map(|component| component / norm);
            let longitude = unit[1].atan2(unit[0]);
            let polar = unit[2].acos();
            let longitude = STATIC_PI.mul_add(2.0, STATIC_PI - longitude) % STATIC_PI;
            let x = longitude * 200.0 / STATIC_PI;
            let y = (f64::from(polar * 99.0) / std::f64::consts::PI) as f32;
            values.push([x, y]);
        }
    }

    StaticLineCoordinates {
        rows: SELECTED_LINE_ROWS,
        cols: SELECTED_LINE_COLS,
        values: values.into_boxed_slice(),
    }
}

/// One selected 100-by-200 half of the dynamic 200-by-200 flowstate map.
#[derive(Clone, Debug, PartialEq)]
pub struct FlowstateRoi {
    rows: usize,
    cols: usize,
    values: Box<[[f32; 2]]>,
}

impl FlowstateRoi {
    /// Admit a tightly packed `[row][column]` float2 payload.
    pub fn from_row_major_values(
        rows: usize,
        cols: usize,
        values: Vec<[f32; 2]>,
    ) -> Result<Self, ShapeError> {
        validate_shape(MapKind::FlowstateRoi, rows, cols, values.len())?;
        Ok(Self {
            rows,
            cols,
            values: values.into_boxed_slice(),
        })
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn row_major_values(&self) -> &[[f32; 2]] {
        &self.values
    }
}

/// One coordinate-shaped line map before Studio's separate native post-filter.
#[derive(Debug, PartialEq)]
pub struct PrefilterLineMap {
    rows: usize,
    cols: usize,
    values: Box<[[f32; 2]]>,
}

impl PrefilterLineMap {
    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    /// The tightly packed `[row][column]` float2 result.
    pub fn row_major_values(&self) -> &[[f32; 2]] {
        &self.values
    }
}

/// One line map after Studio's selected directional continuity filter.
///
/// This is still not a complete retained-base producer: the static reference
/// retains a host-versus-macOS floating-point boundary, while the per-frame
/// flowstate input still lacks its general production owner.
#[derive(Debug, PartialEq)]
pub struct FilteredLineMap {
    rows: usize,
    cols: usize,
    values: Box<[[f32; 2]]>,
}

impl FilteredLineMap {
    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    /// The tightly packed `[row][column]` float2 result.
    pub fn row_major_values(&self) -> &[[f32; 2]] {
        &self.values
    }
}

/// Which native `mapMerge` input owns a shape failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapKind {
    StaticCoordinates,
    FlowstateRoi,
}

impl fmt::Display for MapKind {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaticCoordinates => out.write_str("static line coordinates"),
            Self::FlowstateRoi => out.write_str("flowstate ROI"),
        }
    }
}

/// A row-major `mapMerge` input cannot enter native 32-bit indexing safely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeError {
    Empty {
        kind: MapKind,
        rows: usize,
        cols: usize,
    },
    DimensionTooLarge {
        kind: MapKind,
        rows: usize,
        cols: usize,
        maximum: usize,
    },
    IndexOverflow {
        kind: MapKind,
        rows: usize,
        cols: usize,
        maximum_nodes: usize,
    },
    NodeCount {
        kind: MapKind,
        rows: usize,
        cols: usize,
        expected: usize,
        actual: usize,
    },
}

impl fmt::Display for ShapeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Empty { kind, rows, cols } => write!(
                out,
                "ONE X2 {kind} are {rows} by {cols}; both dimensions must be nonzero",
            ),
            Self::DimensionTooLarge {
                kind,
                rows,
                cols,
                maximum,
            } => write!(
                out,
                "ONE X2 {kind} are {rows} by {cols}; each dimension must be at most {maximum} for exact f32 addressing",
            ),
            Self::IndexOverflow {
                kind,
                rows,
                cols,
                maximum_nodes,
            } => write!(
                out,
                "ONE X2 {kind} shape {rows} by {cols} exceeds the native safe limit of {maximum_nodes} float2 nodes",
            ),
            Self::NodeCount {
                kind,
                rows,
                cols,
                expected,
                actual,
            } => write!(
                out,
                "ONE X2 {kind} are {rows} by {cols} but have {actual} float2 nodes, expected {expected}",
            ),
        }
    }
}

impl Error for ShapeError {}

fn validate_shape(
    kind: MapKind,
    rows: usize,
    cols: usize,
    actual: usize,
) -> Result<usize, ShapeError> {
    if rows == 0 || cols == 0 {
        return Err(ShapeError::Empty { kind, rows, cols });
    }
    if rows > MAX_EXACT_F32_DIMENSION || cols > MAX_EXACT_F32_DIMENSION {
        return Err(ShapeError::DimensionTooLarge {
            kind,
            rows,
            cols,
            maximum: MAX_EXACT_F32_DIMENSION,
        });
    }
    let expected = rows
        .checked_mul(cols)
        .filter(|&nodes| nodes <= MAX_NATIVE_NODES)
        .ok_or(ShapeError::IndexOverflow {
            kind,
            rows,
            cols,
            maximum_nodes: MAX_NATIVE_NODES,
        })?;
    if actual != expected {
        return Err(ShapeError::NodeCount {
            kind,
            rows,
            cols,
            expected,
            actual,
        });
    }
    Ok(expected)
}

/// The coordinate component whose native indexing domain was not safe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoordinateAxis {
    Column,
    Row,
}

impl fmt::Display for CoordinateAxis {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Column => out.write_str("column"),
            Self::Row => out.write_str("row"),
        }
    }
}

/// A coordinate cannot enter Studio's unchecked index conversion safely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeError {
    NonFiniteCoordinate {
        node: usize,
        axis: CoordinateAxis,
        bits: u32,
    },
    UnsafeWrappedCoordinate {
        node: usize,
        axis: CoordinateAxis,
        bits: u32,
        extent: usize,
    },
}

impl fmt::Display for MergeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NonFiniteCoordinate { node, axis, bits } => write!(
                out,
                "ONE X2 static line coordinate {node} {axis} is not finite (bits 0x{bits:08x})",
            ),
            Self::UnsafeWrappedCoordinate {
                node,
                axis,
                bits,
                extent,
            } => write!(
                out,
                "ONE X2 static line coordinate {node} {axis} wraps to bits 0x{bits:08x}, outside [0, {extent})",
            ),
        }
    }
}

impl Error for MergeError {}

/// The selected pair cannot enter Studio's unchecked filter indexing safely.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterError {
    ShapeMismatch {
        a_rows: usize,
        a_cols: usize,
        b_rows: usize,
        b_cols: usize,
    },
    TooFewColumns {
        cols: usize,
    },
}

impl fmt::Display for FilterError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ShapeMismatch {
                a_rows,
                a_cols,
                b_rows,
                b_cols,
            } => write!(
                out,
                "ONE X2 prefilter line maps differ in shape: lens A is {a_rows} by {a_cols}, lens B is {b_rows} by {b_cols}",
            ),
            Self::TooFewColumns { cols } => write!(
                out,
                "ONE X2 prefilter line maps have {cols} columns; the native continuity filter requires at least 3",
            ),
        }
    }
}

impl Error for FilterError {}

/// Compose the selected static coordinates with one dynamic flowstate ROI.
///
/// Studio's native routine performs unchecked float-to-index conversion.
/// Non-finite coordinates and negative exact multiples of an extent do not
/// have a safe native result. Until the selected static coordinate payload is
/// authenticated, this oracle refuses them instead of inventing a clamp or a
/// second wrap.
pub fn map_merge(
    coordinates: &StaticLineCoordinates,
    flowstate: &FlowstateRoi,
) -> Result<PrefilterLineMap, MergeError> {
    let mut output = Vec::with_capacity(coordinates.row_major_values().len());
    for (node, &[column, row]) in coordinates.row_major_values().iter().enumerate() {
        let column = wrap_coordinate(column, flowstate.cols(), node, CoordinateAxis::Column)?;
        let row = wrap_coordinate(row, flowstate.rows(), node, CoordinateAxis::Row)?;

        // Native FCVTZS converts toward zero. The wrapped selected domain is
        // non-negative and small enough that Rust's conversion is identical.
        let column0 = column as i32 as usize;
        let row0 = row as i32 as usize;
        let column1 = if column0 + 1 < flowstate.cols() {
            column0 + 1
        } else {
            column0 + 1 - flowstate.cols()
        };
        let row1 = if row0 + 1 < flowstate.rows() {
            row0 + 1
        } else {
            row0 + 1 - flowstate.rows()
        };

        // Preserve the instruction order at worker +0x340e040..+0x340e074.
        let column_inverse = (1.0_f32 - column) + column0 as f32;
        let row_inverse = (1.0_f32 - row) + row0 as f32;
        let top_left_weight = column_inverse * row_inverse;
        let top_right_weight = row_inverse - top_left_weight;
        let bottom_left_weight = column_inverse - top_left_weight;
        let bottom_right_weight = ((1.0_f32 - column_inverse) - row_inverse) + top_left_weight;

        let values = flowstate.row_major_values();
        let top_left = values[row0 * flowstate.cols() + column0];
        let top_right = values[row0 * flowstate.cols() + column1];
        let bottom_left = values[row1 * flowstate.cols() + column0];
        let bottom_right = values[row1 * flowstate.cols() + column1];

        output.push(std::array::from_fn(|component| {
            // Native +0x340e0d8 is one rounded FMUL. The following three
            // instructions are vector FMLA in exactly this order.
            let sum = top_right[component] * top_right_weight;
            let sum = top_left[component].mul_add(top_left_weight, sum);
            let sum = bottom_left[component].mul_add(bottom_left_weight, sum);
            bottom_right[component].mul_add(bottom_right_weight, sum)
        }));
    }

    Ok(PrefilterLineMap {
        rows: coordinates.rows(),
        cols: coordinates.cols(),
        values: output.into_boxed_slice(),
    })
}

/// Apply Studio's selected in-place continuity filter to the two line maps.
///
/// Lens A is native `+0x8d0`: it keeps columns through `cols / 2`, then
/// invalidates the suffix beginning with the first component-zero jump
/// strictly greater than the READ threshold. Lens B is native `+0x930`: it
/// compares from `cols / 2` toward zero and invalidates the corresponding
/// prefix. A trip writes `[-20,-20]` through that row's remaining edge.
/// Component one never participates in the decision. Equality and unordered
/// comparisons preserve the current pixel.
pub fn filter_fisheye_line_pair(
    maps: LensPair<PrefilterLineMap>,
) -> Result<LensPair<FilteredLineMap>, FilterError> {
    let LensPair { mut a, mut b } = maps;
    if (a.rows, a.cols) != (b.rows, b.cols) {
        return Err(FilterError::ShapeMismatch {
            a_rows: a.rows,
            a_cols: a.cols,
            b_rows: b.rows,
            b_cols: b.cols,
        });
    }
    if a.cols < 3 {
        return Err(FilterError::TooFewColumns { cols: a.cols });
    }

    let pivot = a.cols / 2;
    for row in 0..a.rows {
        let start = row * a.cols;
        let mut tripped = false;
        for col in pivot + 1..a.cols {
            if !tripped {
                let current = a.values[start + col][0];
                let previous = a.values[start + col - 1][0];
                tripped = (current - previous).abs() > FILTER_THRESHOLD;
            }
            if tripped {
                a.values[start + col] = [FILTER_SENTINEL; 2];
            }
        }
    }

    for row in 0..b.rows {
        let start = row * b.cols;
        let mut tripped = false;
        for col in (0..=pivot).rev() {
            if !tripped {
                let next = b.values[start + col + 1][0];
                let current = b.values[start + col][0];
                tripped = (next - current).abs() > FILTER_THRESHOLD;
            }
            if tripped {
                b.values[start + col] = [FILTER_SENTINEL; 2];
            }
        }
    }

    Ok(LensPair {
        a: FilteredLineMap {
            rows: a.rows,
            cols: a.cols,
            values: a.values,
        },
        b: FilteredLineMap {
            rows: b.rows,
            cols: b.cols,
            values: b.values,
        },
    })
}

fn wrap_coordinate(
    value: f32,
    extent: usize,
    node: usize,
    axis: CoordinateAxis,
) -> Result<f32, MergeError> {
    if !value.is_finite() {
        return Err(MergeError::NonFiniteCoordinate {
            node,
            axis,
            bits: value.to_bits(),
        });
    }

    let extent_float = extent as f32;
    let wrapped = if value < 0.0 {
        (value % extent_float) + extent_float
    } else if value >= extent_float {
        value % extent_float
    } else {
        value
    };

    // The native negative branch adds the extent after fmodf. A negative
    // exact multiple, or a sufficiently small negative value lost to f32
    // addition, can therefore become exactly `extent` and index outside B.
    if !(0.0..extent_float).contains(&wrapped) {
        return Err(MergeError::UnsafeWrappedCoordinate {
            node,
            axis,
            bits: wrapped.to_bits(),
            extent,
        });
    }
    Ok(wrapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coordinates(rows: usize, cols: usize, fill: [f32; 2]) -> StaticLineCoordinates {
        StaticLineCoordinates::from_row_major_values(rows, cols, vec![fill; rows * cols]).unwrap()
    }

    fn coordinate_cases(cases: &[[f32; 2]]) -> StaticLineCoordinates {
        StaticLineCoordinates::from_row_major_values(1, cases.len(), cases.to_vec()).unwrap()
    }

    fn flowstate_coordinates() -> FlowstateRoi {
        let values = (0..2)
            .flat_map(|row| (0..3).map(move |column| [column as f32, row as f32]))
            .collect();
        FlowstateRoi::from_row_major_values(2, 3, values).unwrap()
    }

    fn prefilter(rows: usize, cols: usize, values: Vec<[f32; 2]>) -> PrefilterLineMap {
        assert_eq!(values.len(), rows * cols);
        PrefilterLineMap {
            rows,
            cols,
            values: values.into_boxed_slice(),
        }
    }

    fn value_bits(values: &[[f32; 2]]) -> Vec<[u32; 2]> {
        values
            .iter()
            .map(|value| [value[0].to_bits(), value[1].to_bits()])
            .collect()
    }

    #[test]
    fn row_major_shapes_are_enforced() {
        assert_eq!(SELECTED_FLOWSTATE_ROWS, 100);
        assert_eq!(SELECTED_FLOWSTATE_COLS, 200);
        assert_eq!(SELECTED_LINE_ROWS, 1080);
        assert_eq!(SELECTED_LINE_COLS, 60);
        assert_eq!(
            StaticLineCoordinates::from_row_major_values(0, 1, Vec::new()).unwrap_err(),
            ShapeError::Empty {
                kind: MapKind::StaticCoordinates,
                rows: 0,
                cols: 1,
            }
        );
        assert_eq!(
            FlowstateRoi::from_row_major_values(2, 3, vec![[0.0; 2]; 5]).unwrap_err(),
            ShapeError::NodeCount {
                kind: MapKind::FlowstateRoi,
                rows: 2,
                cols: 3,
                expected: 6,
                actual: 5,
            }
        );
    }

    #[test]
    fn selected_static_reference_keeps_the_read_setup_and_domain() {
        let row_step =
            (STATIC_UPPER_DEGREES - STATIC_LOWER_DEGREES) / (SELECTED_LINE_ROWS - 1) as f32;
        let turns_per_row = 360.0 / row_step;
        let scale = STATIC_LATERAL_SCALE * (turns_per_row / STATIC_PI);
        assert_eq!(row_step.to_bits(), 0x3ebd_ce2d);
        assert_eq!(turns_per_row.to_bits(), 0x4472_c667);
        assert_eq!(scale.to_bits(), 0x431a_8e2d);
        assert_eq!(
            ((SELECTED_LINE_COLS - 1) as f32 * 0.5).to_bits(),
            0x41ec_0000
        );

        let maps = one_xs_static_coordinates();
        for map in [&maps.a, &maps.b] {
            assert_eq!(
                (map.rows(), map.cols()),
                (SELECTED_LINE_ROWS, SELECTED_LINE_COLS)
            );
            assert!(map.row_major_values().iter().all(|&[x, y]| {
                x.is_finite()
                    && y.is_finite()
                    && (0.0..200.0).contains(&x)
                    && (0.0..=99.0).contains(&y)
            }));
        }
        assert_ne!(maps.a, maps.b);
    }

    #[test]
    fn wraps_both_axes_at_both_sides() {
        let coordinates = coordinate_cases(&[[2.25, 1.5], [-0.25, -0.5], [3.25, 2.5], [0.25, 0.5]]);
        let output = map_merge(&coordinates, &flowstate_coordinates()).unwrap();

        assert_eq!((output.rows(), output.cols()), (1, 4));
        assert_eq!(output.row_major_values()[0], [1.5, 0.5]);
        assert_eq!(output.row_major_values()[1], [0.5, 0.5]);
        assert_eq!(output.row_major_values()[2], [0.25, 0.5]);
        assert_eq!(output.row_major_values()[3], [0.25, 0.5]);
    }

    #[test]
    fn retains_the_arm_fma_accumulation_order() {
        // These literals were produced by an independent IEEE-f32
        // transcription of worker +0x340e040..+0x340e0ec. They are not
        // derived through `map_merge`; replacing the three FMLAs with rounded
        // multiply-then-add operations changes lane zero to 0x4797_4864.
        let column = f32::from_bits(0x3ecc_d879);
        let row = f32::from_bits(0x3d7a_d915);
        let coordinates = coordinates(1, 1, [column, row]);
        let mut values = vec![[0.0; 2]; 4];
        values[0] = [f32::from_bits(0x4915_2da2), f32::from_bits(0xc85f_532f)];
        values[1] = [f32::from_bits(0xc918_5989), f32::from_bits(0xc931_4d0f)];
        values[2] = [f32::from_bits(0xc931_4d0f), f32::from_bits(0xc918_5989)];
        values[3] = [f32::from_bits(0xc85f_532f), f32::from_bits(0x4915_2da2)];
        let flowstate = FlowstateRoi::from_row_major_values(2, 2, values).unwrap();

        let output = map_merge(&coordinates, &flowstate).unwrap();
        assert_eq!(output.row_major_values()[0][0].to_bits(), 0x4797_4865);
        assert_eq!(output.row_major_values()[0][1].to_bits(), 0xc8c7_f3fc);
    }

    #[test]
    fn refuses_coordinates_with_no_safe_native_index() {
        let flowstate = flowstate_coordinates();
        let error = map_merge(&coordinates(1, 1, [f32::NAN, 0.0]), &flowstate).unwrap_err();
        assert_eq!(
            error,
            MergeError::NonFiniteCoordinate {
                node: 0,
                axis: CoordinateAxis::Column,
                bits: f32::NAN.to_bits(),
            }
        );

        let error = map_merge(&coordinates(1, 1, [-3.0, 0.0]), &flowstate).unwrap_err();
        assert_eq!(
            error,
            MergeError::UnsafeWrappedCoordinate {
                node: 0,
                axis: CoordinateAxis::Column,
                bits: 3.0_f32.to_bits(),
                extent: 3,
            }
        );

        let error = map_merge(&coordinates(1, 1, [0.0, -2.0]), &flowstate).unwrap_err();
        assert_eq!(
            error,
            MergeError::UnsafeWrappedCoordinate {
                node: 0,
                axis: CoordinateAxis::Row,
                bits: 2.0_f32.to_bits(),
                extent: 2,
            }
        );
    }

    #[test]
    fn directional_filter_preserves_ordered_and_unordered_nontrips() {
        const ROWS: usize = 5;
        const COLS: usize = 6;
        const PIVOT: usize = COLS / 2;
        let threshold = f32::from_bits(0x3ba3_d70a);
        let greater = f32::from_bits(0x3ba3_d70b);
        let component_one_nan = f32::from_bits(0x7fc0_1234);
        let component_zero_nan = f32::from_bits(0x7fc0_5678);

        let mut a = vec![[0.0, 1.0]; ROWS * COLS];
        let mut b = vec![[0.0, 1.0]; ROWS * COLS];

        // Equality to the threshold is preserved in both directions.
        a[PIVOT] = [0.0, 2.0];
        a[PIVOT + 1] = [threshold, 3.0];
        a[PIVOT + 2] = [threshold, 4.0];
        b[PIVOT] = [threshold, 5.0];
        b[PIVOT + 1] = [0.0, 6.0];
        b[PIVOT - 1] = [threshold, 7.0];

        // The next representable delta trips, including the current pixel.
        let row = 1;
        a[row * COLS + PIVOT] = [0.0, 8.0];
        a[row * COLS + PIVOT + 1] = [greater, 9.0];
        a[row * COLS + PIVOT + 2] = [component_zero_nan, 10.0];
        b[row * COLS + PIVOT] = [greater, 11.0];
        b[row * COLS + PIVOT + 1] = [0.0, 12.0];

        // Component one is never consulted, even when unordered.
        let row = 2;
        a[row * COLS + PIVOT + 1][1] = component_one_nan;
        b[row * COLS + PIVOT][1] = component_one_nan;

        // An unordered component-zero difference does not trip B.GT.
        let row = 3;
        a[row * COLS + PIVOT + 1][0] = component_zero_nan;
        b[row * COLS + PIVOT][0] = component_zero_nan;

        // Signed zeros remain byte-exact when no row trips.
        let row = 4;
        for col in 0..COLS {
            let zero = if col % 2 == 0 { 0.0 } else { -0.0 };
            a[row * COLS + col] = [zero, f32::from_bits(0x8000_0000 | col as u32)];
            b[row * COLS + col] = [zero, f32::from_bits(col as u32)];
        }

        let mut expected_a = value_bits(&a);
        let mut expected_b = value_bits(&b);
        for col in PIVOT + 1..COLS {
            expected_a[COLS + col] = [FILTER_SENTINEL.to_bits(); 2];
        }
        for col in 0..=PIVOT {
            expected_b[COLS + col] = [FILTER_SENTINEL.to_bits(); 2];
        }

        let filtered = filter_fisheye_line_pair(LensPair {
            a: prefilter(ROWS, COLS, a),
            b: prefilter(ROWS, COLS, b),
        })
        .unwrap();

        assert_eq!((filtered.a.rows(), filtered.a.cols()), (ROWS, COLS));
        assert_eq!((filtered.b.rows(), filtered.b.cols()), (ROWS, COLS));
        assert_eq!(value_bits(filtered.a.row_major_values()), expected_a);
        assert_eq!(value_bits(filtered.b.row_major_values()), expected_b);
    }

    #[test]
    fn directional_filter_refuses_native_unsafe_shapes() {
        let error = filter_fisheye_line_pair(LensPair {
            a: prefilter(1, 3, vec![[0.0; 2]; 3]),
            b: prefilter(2, 3, vec![[0.0; 2]; 6]),
        })
        .unwrap_err();
        assert_eq!(
            error,
            FilterError::ShapeMismatch {
                a_rows: 1,
                a_cols: 3,
                b_rows: 2,
                b_cols: 3,
            }
        );

        let error = filter_fisheye_line_pair(LensPair {
            a: prefilter(1, 2, vec![[0.0; 2]; 2]),
            b: prefilter(1, 2, vec![[0.0; 2]; 2]),
        })
        .unwrap_err();
        assert_eq!(error, FilterError::TooFewColumns { cols: 2 });
    }
}
