//! Inactive scalar variational-refinement reference for selected ONE X2 FDS.
//!
//! Native prepares the eight derivative planes once from the freshly
//! densified field, then runs five fixed-point rounds. Each round rebuilds the
//! robust data term, adds horizontal and vertical smoothness, and performs five
//! red-then-black SOR sweeps. The increment survives every round; there is no
//! rewarp and no increment reset.
//!
//! This module starts after OpenCV's remap/Sobel preparation and ends before
//! the mandatory selected `updateFlowUsingMotionPyramid` stage in
//! [`super::post_update`]. That stage consumes incoming retained-public-flow
//! pyramids and itself precedes OpenCV's pyramid/public resize; it is distinct
//! from the finest-only auxiliary hint calculation. This is a readable
//! binary32 topology oracle, not a claim of arm64 SIMD, fused-operation,
//! OpenCV, or Mac bit identity. It remains disconnected from Scene and the
//! production renderer.

use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

use super::Direction;
use super::dense::DenseField;
use super::derivative_prep::PreparedRefinement;
use super::pis::{Level, PisDirection};

/// Native's selected number of fixed-point rounds at each level.
pub const FIXED_POINT_ROUNDS: usize = 5;

/// Native's selected number of red-black SOR sweeps in each round.
pub const SOR_SWEEPS_PER_ROUND: usize = 5;

/// Smoothness coefficient selected by the owning FDS instance.
pub const ALPHA: f32 = 20.0;

/// Brightness-constancy coefficient selected by the owning FDS instance.
pub const DELTA: f32 = 5.0;

/// Gradient-constancy coefficient selected by the owning FDS instance.
pub const GAMMA: f32 = 10.0;

/// Isotropic conditioning term in the two data normalizers.
pub const ZETA: f32 = 0.1;

/// Charbonnier epsilon selected by the owning FDS instance.
pub const EPSILON: f32 = 0.001;

/// Native's selected successive-over-relaxation factor.
pub const SOR_OMEGA: f32 = 1.6;

const ZETA_SQUARED: f32 = ZETA * ZETA;
const EPSILON_SQUARED: f32 = EPSILON * EPSILON;
const HALF_DELTA: f32 = DELTA * 0.5;
const HALF_GAMMA: f32 = GAMMA * 0.5;
const HALF_ALPHA: f32 = ALPHA * 0.5;

/// The eight derivative planes prepared once before native's fixed-point loop.
///
/// All values are contiguous row-major binary32 at one selected level. This
/// boundary deliberately validates shape only. Non-finite values are allowed
/// to flow into native's final whole-level rollback predicate.
#[derive(Debug)]
pub(super) struct PreparedDerivatives<D: PisDirection> {
    level: Level,
    ix: Box<[f32]>,
    iy: Box<[f32]>,
    iz: Box<[f32]>,
    ixx: Box<[f32]>,
    ixy: Box<[f32]>,
    iyy: Box<[f32]>,
    ixz: Box<[f32]>,
    iyz: Box<[f32]>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> PreparedDerivatives<D> {
    /// Admit exactly one selected level's already-prepared derivative planes.
    ///
    /// Preparation itself is intentionally outside this oracle. Native remaps
    /// the second image with repeated borders once, averages the two images,
    /// and uses OpenCV Sobel operators to produce these eight planes.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_components(
        level: Level,
        ix: Vec<f32>,
        iy: Vec<f32>,
        iz: Vec<f32>,
        ixx: Vec<f32>,
        ixy: Vec<f32>,
        iyy: Vec<f32>,
        ixz: Vec<f32>,
        iyz: Vec<f32>,
    ) -> Result<Self, ShapeError> {
        let expected = level.pixels();
        for (part, actual) in [
            ("Ix plane", ix.len()),
            ("Iy plane", iy.len()),
            ("Iz plane", iz.len()),
            ("Ixx plane", ixx.len()),
            ("Ixy plane", ixy.len()),
            ("Iyy plane", iyy.len()),
            ("Ixz plane", ixz.len()),
            ("Iyz plane", iyz.len()),
        ] {
            if actual != expected {
                return Err(ShapeError {
                    direction: D::DIRECTION,
                    level,
                    part,
                    expected,
                    actual,
                });
            }
        }

        Ok(Self {
            level,
            ix: ix.into_boxed_slice(),
            iy: iy.into_boxed_slice(),
            iz: iz.into_boxed_slice(),
            ixx: ixx.into_boxed_slice(),
            ixy: ixy.into_boxed_slice(),
            iyy: iyy.into_boxed_slice(),
            ixz: ixz.into_boxed_slice(),
            iyz: iyz.into_boxed_slice(),
            direction: PhantomData,
        })
    }

    pub const fn rows(&self) -> usize {
        self.level.rows()
    }

    pub const fn cols(&self) -> usize {
        self.level.cols()
    }

    #[cfg(test)]
    const fn direction(&self) -> Direction {
        D::DIRECTION
    }
}

/// A derivative plane has the wrong selected-level shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ShapeError {
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
            "ONE X2 {} {} has {} values in its {}, expected {}",
            self.direction, self.level, self.actual, self.part, self.expected,
        )
    }
}

impl Error for ShapeError {}

/// The dense field and derivative preparation name different pyramid levels.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WrongLevel {
    direction: Direction,
    derivatives: Level,
    field: Level,
}

#[cfg(test)]
impl fmt::Display for WrongLevel {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "ONE X2 {} variational derivatives are {}, but the dense field is {}",
            self.direction, self.derivatives, self.field,
        )
    }
}

#[cfg(test)]
impl Error for WrongLevel {}

/// One consumed dense token after variational refinement or native rollback.
#[must_use = "the refined dense field has not been handed to the next FDS stage"]
#[derive(Debug)]
pub struct Refinement<D: PisDirection> {
    field: DenseField<D>,
    rolled_back: bool,
}

impl<D: PisDirection> Refinement<D> {
    pub const fn rolled_back(&self) -> bool {
        self.rolled_back
    }

    pub const fn field(&self) -> &DenseField<D> {
        &self.field
    }

    #[allow(
        dead_code,
        reason = "consumed only by the sealed inactive post-update stage"
    )]
    pub(super) fn into_parts(self) -> (DenseField<D>, bool) {
        (self.field, self.rolled_back)
    }

    #[cfg(test)]
    pub(crate) fn from_test_field(field: DenseField<D>, rolled_back: bool) -> Self {
        Self { field, rolled_back }
    }
}

/// Refine one freshly densified selected level.
///
/// The caller must keep this preparation paired with the exact dense field
/// from which native's one-time warp was made. The type system seals direction
/// and level, but cannot distinguish two fields from the same direction and
/// level or prevent reusing a preparation with a later field.
///
/// The input token is consumed exactly once. If either output component
/// contains a NaN or a value strictly greater than the level's column count,
/// both complete component planes are restored to their pre-refinement values.
/// Equality, negative finite values, and negative infinity do not trigger the
/// native predicate; positive infinity does.
#[cfg(test)]
fn refine<D: PisDirection>(
    derivatives: &PreparedDerivatives<D>,
    field: DenseField<D>,
) -> Result<Refinement<D>, WrongLevel> {
    if derivatives.level != field.level() {
        return Err(WrongLevel {
            direction: D::DIRECTION,
            derivatives: derivatives.level,
            field: field.level(),
        });
    }

    Ok(refine_matching(derivatives, field))
}

/// Refine the exact dense token consumed by scalar derivative preparation.
///
/// Unlike the test-only unbound helper, this entry point cannot pair a
/// derivative set with a different same-direction, same-level field or reuse
/// it for a later epoch.
/// Raw derivative construction and unbound refinement are not public:
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::variational::{PreparedDerivatives, refine};
/// ```
pub fn refine_prepared<D: PisDirection>(prepared: PreparedRefinement<D>) -> Refinement<D> {
    let (derivatives, field) = prepared.into_parts();
    refine_matching(&derivatives, field)
}

fn refine_matching<D: PisDirection>(
    derivatives: &PreparedDerivatives<D>,
    mut field: DenseField<D>,
) -> Refinement<D> {
    let original_dcol = field.dcol().to_vec();
    let original_drow = field.drow().to_vec();
    let mut solver = Solver::new(
        derivatives.rows(),
        derivatives.cols(),
        &original_dcol,
        &original_drow,
    );
    solver.run(derivatives);

    let candidate_dcol = solver.total_dcol.row_major();
    let candidate_drow = solver.total_drow.row_major();
    let rolled_back =
        requires_whole_level_rollback(derivatives.cols(), &candidate_dcol, &candidate_drow);

    let (dcol, drow) = field.components_mut();
    if rolled_back {
        dcol.copy_from_slice(&original_dcol);
        drow.copy_from_slice(&original_drow);
    } else {
        dcol.copy_from_slice(&candidate_dcol);
        drow.copy_from_slice(&candidate_drow);
    }

    Refinement { field, rolled_back }
}

fn requires_whole_level_rollback(cols: usize, dcol: &[f32], drow: &[f32]) -> bool {
    let limit = cols as f32;
    dcol.iter()
        .chain(drow)
        .any(|value| value.is_nan() || *value > limit)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Color {
    Red,
    Black,
}

impl Color {
    const ALL: [Self; 2] = [Self::Red, Self::Black];

    const fn other(self) -> Self {
        match self {
            Self::Red => Self::Black,
            Self::Black => Self::Red,
        }
    }

    const fn parity(self) -> usize {
        match self {
            Self::Red => 0,
            Self::Black => 1,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Side {
    Left,
    Right,
    Up,
    Down,
}

/// Two padded checkerboard planes with native red/black ownership.
///
/// The stride is `ceil(cols/2)+2`; physical values begin at packed row/column
/// one. Cardinal neighbours always belong to the opposite plane. Repeated
/// padding is refreshed only for total flow. Zero-initialized increment and
/// weight padding is deliberately never refreshed.
#[derive(Clone, Debug)]
struct PackedPlane {
    rows: usize,
    cols: usize,
    stride: usize,
    red: Vec<f32>,
    black: Vec<f32>,
}

impl PackedPlane {
    fn zero(rows: usize, cols: usize) -> Self {
        let stride = cols.div_ceil(2) + 2;
        let len = (rows + 2) * stride;
        Self {
            rows,
            cols,
            stride,
            red: vec![0.0; len],
            black: vec![0.0; len],
        }
    }

    fn from_row_major(rows: usize, cols: usize, values: &[f32]) -> Self {
        debug_assert_eq!(values.len(), rows * cols);
        let mut packed = Self::zero(rows, cols);
        for y in 0..rows {
            for x in 0..cols {
                packed.set(y, x, values[y * cols + x]);
            }
        }
        packed.update_repeated_borders();
        packed
    }

    const fn color(y: usize, x: usize) -> Color {
        if (x + y) & 1 == 0 {
            Color::Red
        } else {
            Color::Black
        }
    }

    fn first_x(y: usize, color: Color) -> usize {
        color.parity() ^ (y & 1)
    }

    fn interior_index(&self, y: usize, x: usize) -> usize {
        (y + 1) * self.stride + x / 2 + 1
    }

    fn plane(&self, color: Color) -> &[f32] {
        match color {
            Color::Red => &self.red,
            Color::Black => &self.black,
        }
    }

    fn plane_mut(&mut self, color: Color) -> &mut [f32] {
        match color {
            Color::Red => &mut self.red,
            Color::Black => &mut self.black,
        }
    }

    fn get(&self, y: usize, x: usize) -> f32 {
        let color = Self::color(y, x);
        self.plane(color)[self.interior_index(y, x)]
    }

    fn set(&mut self, y: usize, x: usize, value: f32) {
        let color = Self::color(y, x);
        let index = self.interior_index(y, x);
        self.plane_mut(color)[index] = value;
    }

    fn add(&mut self, y: usize, x: usize, value: f32) {
        let color = Self::color(y, x);
        let index = self.interior_index(y, x);
        self.plane_mut(color)[index] += value;
    }

    fn neighbour_location(&self, color: Color, y: usize, x: usize, side: Side) -> (Color, usize) {
        let packed_x = x / 2 + 1;
        let (packed_y, packed_x) = match side {
            Side::Left => (y + 1, if x & 1 == 0 { packed_x - 1 } else { packed_x }),
            Side::Right => (y + 1, if x & 1 == 0 { packed_x } else { packed_x + 1 }),
            Side::Up => (y, packed_x),
            Side::Down => (y + 2, packed_x),
        };
        (color.other(), packed_y * self.stride + packed_x)
    }

    fn neighbour(&self, color: Color, y: usize, x: usize, side: Side) -> f32 {
        let (other, index) = self.neighbour_location(color, y, x, side);
        self.plane(other)[index]
    }

    fn set_neighbour(&mut self, color: Color, y: usize, x: usize, side: Side, value: f32) {
        let (other, index) = self.neighbour_location(color, y, x, side);
        self.plane_mut(other)[index] = value;
    }

    fn update_repeated_borders(&mut self) {
        for y in 0..self.rows {
            let left = self.get(y, 0);
            let left_color = Self::color(y, 0);
            self.set_neighbour(left_color, y, 0, Side::Left, left);

            let right_x = self.cols - 1;
            let right = self.get(y, right_x);
            let right_color = Self::color(y, right_x);
            self.set_neighbour(right_color, y, right_x, Side::Right, right);
        }
        for x in 0..self.cols {
            let top = self.get(0, x);
            let top_color = Self::color(0, x);
            self.set_neighbour(top_color, 0, x, Side::Up, top);

            let bottom_y = self.rows - 1;
            let bottom = self.get(bottom_y, x);
            let bottom_color = Self::color(bottom_y, x);
            self.set_neighbour(bottom_color, bottom_y, x, Side::Down, bottom);
        }
    }

    fn row_major(&self) -> Vec<f32> {
        let mut values = vec![0.0; self.rows * self.cols];
        for y in 0..self.rows {
            for x in 0..self.cols {
                values[y * self.cols + x] = self.get(y, x);
            }
        }
        values
    }
}

#[derive(Debug)]
struct LinearSystem {
    a11: PackedPlane,
    a12: PackedPlane,
    a22: PackedPlane,
    b1: PackedPlane,
    b2: PackedPlane,
    weight: PackedPlane,
}

impl LinearSystem {
    fn new(rows: usize, cols: usize) -> Self {
        Self {
            a11: PackedPlane::zero(rows, cols),
            a12: PackedPlane::zero(rows, cols),
            a22: PackedPlane::zero(rows, cols),
            b1: PackedPlane::zero(rows, cols),
            b2: PackedPlane::zero(rows, cols),
            weight: PackedPlane::zero(rows, cols),
        }
    }
}

#[derive(Debug)]
struct Solver {
    rows: usize,
    cols: usize,
    base_dcol: PackedPlane,
    base_drow: PackedPlane,
    increment_dcol: PackedPlane,
    increment_drow: PackedPlane,
    total_dcol: PackedPlane,
    total_drow: PackedPlane,
    system: LinearSystem,
}

impl Solver {
    fn new(rows: usize, cols: usize, dcol: &[f32], drow: &[f32]) -> Self {
        let base_dcol = PackedPlane::from_row_major(rows, cols, dcol);
        let base_drow = PackedPlane::from_row_major(rows, cols, drow);
        Self {
            rows,
            cols,
            total_dcol: base_dcol.clone(),
            total_drow: base_drow.clone(),
            base_dcol,
            base_drow,
            increment_dcol: PackedPlane::zero(rows, cols),
            increment_drow: PackedPlane::zero(rows, cols),
            system: LinearSystem::new(rows, cols),
        }
    }

    fn run<D: PisDirection>(&mut self, derivatives: &PreparedDerivatives<D>) {
        for _ in 0..FIXED_POINT_ROUNDS {
            self.fixed_point_round(derivatives);
        }
    }

    fn fixed_point_round<D: PisDirection>(&mut self, derivatives: &PreparedDerivatives<D>) {
        self.compute_data_term(derivatives);
        self.compute_horizontal_smoothness();
        self.compute_vertical_smoothness();
        for _ in 0..SOR_SWEEPS_PER_ROUND {
            self.sor_color(Color::Red, SOR_OMEGA);
            self.sor_color(Color::Black, SOR_OMEGA);
        }
        self.update_total();
    }

    fn compute_data_term<D: PisDirection>(&mut self, d: &PreparedDerivatives<D>) {
        for color in Color::ALL {
            for y in 0..self.rows {
                for x in (PackedPlane::first_x(y, color)..self.cols).step_by(2) {
                    let p = y * self.cols + x;
                    let du = self.increment_dcol.get(y, x);
                    let dv = self.increment_drow.get(y, x);
                    let ix = d.ix[p];
                    let iy = d.iy[p];
                    let iz = d.iz[p];

                    let brightness_norm = ix * ix + iy * iy + ZETA_SQUARED;
                    let brightness_residual = iz + ix * du + iy * dv;
                    let brightness_weight = HALF_DELTA
                        / (brightness_residual * brightness_residual / brightness_norm
                            + EPSILON_SQUARED)
                            .sqrt()
                        / brightness_norm;

                    let mut a11 = brightness_weight * ix * ix + ZETA_SQUARED;
                    let mut a12 = brightness_weight * ix * iy;
                    let mut a22 = brightness_weight * iy * iy + ZETA_SQUARED;
                    let mut b1 = -brightness_weight * iz * ix;
                    let mut b2 = -brightness_weight * iz * iy;

                    let ixx = d.ixx[p];
                    let ixy = d.ixy[p];
                    let iyy = d.iyy[p];
                    let ixz = d.ixz[p];
                    let iyz = d.iyz[p];
                    let gradient_x_norm = ixx * ixx + ixy * ixy + ZETA_SQUARED;
                    let gradient_y_norm = iyy * iyy + ixy * ixy + ZETA_SQUARED;
                    let gradient_x_residual = ixz + ixx * du + ixy * dv;
                    let gradient_y_residual = iyz + ixy * du + iyy * dv;
                    let gradient_weight = HALF_GAMMA
                        / (gradient_x_residual * gradient_x_residual / gradient_x_norm
                            + gradient_y_residual * gradient_y_residual / gradient_y_norm
                            + EPSILON_SQUARED)
                            .sqrt();

                    a11 += gradient_weight
                        * (ixx * ixx / gradient_x_norm + ixy * ixy / gradient_y_norm);
                    a12 += gradient_weight
                        * (ixx * ixy / gradient_x_norm + ixy * iyy / gradient_y_norm);
                    a22 += gradient_weight
                        * (ixy * ixy / gradient_x_norm + iyy * iyy / gradient_y_norm);
                    b1 -= gradient_weight
                        * (ixx * ixz / gradient_x_norm + ixy * iyz / gradient_y_norm);
                    b2 -= gradient_weight
                        * (ixy * ixz / gradient_x_norm + iyy * iyz / gradient_y_norm);

                    self.system.a11.set(y, x, a11);
                    self.system.a12.set(y, x, a12);
                    self.system.a22.set(y, x, a22);
                    self.system.b1.set(y, x, b1);
                    self.system.b2.set(y, x, b2);
                }
            }
        }
    }

    fn smoothness_weight(&self, color: Color, y: usize, x: usize) -> f32 {
        let dcol = self.total_dcol.get(y, x);
        let drow = self.total_drow.get(y, x);
        let right_dcol = self.total_dcol.neighbour(color, y, x, Side::Right);
        let right_drow = self.total_drow.neighbour(color, y, x, Side::Right);
        let down_dcol = self.total_dcol.neighbour(color, y, x, Side::Down);
        let down_drow = self.total_drow.neighbour(color, y, x, Side::Down);
        let right_col = right_dcol - dcol;
        let right_row = right_drow - drow;
        let down_col = down_dcol - dcol;
        let down_row = down_drow - drow;
        HALF_ALPHA
            / (right_col * right_col
                + right_row * right_row
                + down_col * down_col
                + down_row * down_row
                + EPSILON_SQUARED)
                .sqrt()
    }

    fn compute_horizontal_smoothness(&mut self) {
        for color in Color::ALL {
            for y in 0..self.rows {
                for x in (PackedPlane::first_x(y, color)..self.cols).step_by(2) {
                    let weight = self.smoothness_weight(color, y, x);
                    self.system.weight.set(y, x, weight);
                    if x + 1 == self.cols {
                        continue;
                    }

                    self.system.a11.add(y, x, weight);
                    self.system.a11.add(y, x + 1, weight);
                    self.system.a22.add(y, x, weight);
                    self.system.a22.add(y, x + 1, weight);

                    let base_col_difference = self.base_dcol.neighbour(color, y, x, Side::Right)
                        - self.base_dcol.get(y, x);
                    let base_row_difference = self.base_drow.neighbour(color, y, x, Side::Right)
                        - self.base_drow.get(y, x);
                    let rhs_col = weight * base_col_difference;
                    let rhs_row = weight * base_row_difference;
                    self.system.b1.add(y, x, rhs_col);
                    self.system.b1.add(y, x + 1, -rhs_col);
                    self.system.b2.add(y, x, rhs_row);
                    self.system.b2.add(y, x + 1, -rhs_row);
                }
            }
        }
    }

    fn compute_vertical_smoothness(&mut self) {
        for color in Color::ALL {
            for y in 0..self.rows {
                for x in (PackedPlane::first_x(y, color)..self.cols).step_by(2) {
                    if y + 1 == self.rows {
                        continue;
                    }
                    let weight = self.system.weight.get(y, x);
                    self.system.a11.add(y, x, weight);
                    self.system.a11.add(y + 1, x, weight);
                    self.system.a22.add(y, x, weight);
                    self.system.a22.add(y + 1, x, weight);

                    let base_col_difference = self.base_dcol.neighbour(color, y, x, Side::Down)
                        - self.base_dcol.get(y, x);
                    let base_row_difference = self.base_drow.neighbour(color, y, x, Side::Down)
                        - self.base_drow.get(y, x);
                    let rhs_col = weight * base_col_difference;
                    let rhs_row = weight * base_row_difference;
                    self.system.b1.add(y, x, rhs_col);
                    self.system.b1.add(y + 1, x, -rhs_col);
                    self.system.b2.add(y, x, rhs_row);
                    self.system.b2.add(y + 1, x, -rhs_row);
                }
            }
        }
    }

    fn sor_color(&mut self, color: Color, omega: f32) {
        for y in 0..self.rows {
            for x in (PackedPlane::first_x(y, color)..self.cols).step_by(2) {
                let weight_left = self.system.weight.neighbour(color, y, x, Side::Left);
                let weight_right = self.system.weight.get(y, x);
                let weight_up = self.system.weight.neighbour(color, y, x, Side::Up);
                let weight_down = self.system.weight.get(y, x);

                let sigma_col = weight_left
                    * self.increment_dcol.neighbour(color, y, x, Side::Left)
                    + weight_right * self.increment_dcol.neighbour(color, y, x, Side::Right)
                    + weight_up * self.increment_dcol.neighbour(color, y, x, Side::Up)
                    + weight_down * self.increment_dcol.neighbour(color, y, x, Side::Down);
                let sigma_row = weight_left
                    * self.increment_drow.neighbour(color, y, x, Side::Left)
                    + weight_right * self.increment_drow.neighbour(color, y, x, Side::Right)
                    + weight_up * self.increment_drow.neighbour(color, y, x, Side::Up)
                    + weight_down * self.increment_drow.neighbour(color, y, x, Side::Down);

                let old_col = self.increment_dcol.get(y, x);
                let old_row = self.increment_drow.get(y, x);
                let a11 = self.system.a11.get(y, x);
                let a12 = self.system.a12.get(y, x);
                let a22 = self.system.a22.get(y, x);
                let new_col = old_col
                    + omega
                        * ((sigma_col + self.system.b1.get(y, x) - a12 * old_row) / a11 - old_col);
                self.increment_dcol.set(y, x, new_col);
                let new_row = old_row
                    + omega
                        * ((sigma_row + self.system.b2.get(y, x) - a12 * new_col) / a22 - old_row);
                self.increment_drow.set(y, x, new_row);
            }
        }
    }

    fn update_total(&mut self) {
        for color in Color::ALL {
            for y in 0..self.rows {
                for x in (PackedPlane::first_x(y, color)..self.cols).step_by(2) {
                    self.total_dcol.set(
                        y,
                        x,
                        self.base_dcol.get(y, x) + self.increment_dcol.get(y, x),
                    );
                    self.total_drow.set(
                        y,
                        x,
                        self.base_drow.get(y, x) + self.increment_drow.get(y, x),
                    );
                }
            }
        }
        self.total_dcol.update_repeated_borders();
        self.total_drow.update_repeated_borders();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::one_xs::pis::{AtoB, BtoA};

    fn constant_derivatives<D: PisDirection>(
        level: Level,
        ix: f32,
        iy: f32,
        iz: f32,
    ) -> PreparedDerivatives<D> {
        let pixels = level.pixels();
        PreparedDerivatives::from_components(
            level,
            vec![ix; pixels],
            vec![iy; pixels],
            vec![iz; pixels],
            vec![0.0; pixels],
            vec![0.0; pixels],
            vec![0.0; pixels],
            vec![0.0; pixels],
            vec![0.0; pixels],
        )
        .unwrap()
    }

    fn assert_close(actual: f32, expected: f32) {
        let tolerance = 2.0e-6 * expected.abs().max(1.0);
        assert!(
            (actual - expected).abs() <= tolerance,
            "actual {actual:?}, expected {expected:?}, tolerance {tolerance:?}",
        );
    }

    fn assert_cardinal_neighbours(packed: &PackedPlane, y: usize, x: usize) {
        let color = PackedPlane::color(y, x);
        let expected = |y: usize, x: usize| (y * packed.cols + x) as f32 + 0.25;
        assert_eq!(
            packed.neighbour(color, y, x, Side::Left),
            expected(y, x - 1),
        );
        assert_eq!(
            packed.neighbour(color, y, x, Side::Right),
            expected(y, x + 1),
        );
        assert_eq!(packed.neighbour(color, y, x, Side::Up), expected(y - 1, x),);
        assert_eq!(
            packed.neighbour(color, y, x, Side::Down),
            expected(y + 1, x),
        );
    }

    #[test]
    fn selected_shapes_and_direction_labels_are_closed() {
        assert_eq!(FIXED_POINT_ROUNDS, 5);
        assert_eq!(SOR_SWEEPS_PER_ROUND, 5);
        assert_eq!((Level::Two.rows(), Level::Two.cols()), (270, 15));
        assert_eq!((Level::One.rows(), Level::One.cols()), (540, 30));

        let l2 = constant_derivatives::<AtoB>(Level::Two, 0.0, 0.0, 0.0);
        let l1 = constant_derivatives::<BtoA>(Level::One, 0.0, 0.0, 0.0);
        assert_eq!(l2.direction(), Direction::AtoB);
        assert_eq!(l1.direction(), Direction::BtoA);
        assert_eq!((l2.rows(), l2.cols()), (270, 15));
        assert_eq!((l1.rows(), l1.cols()), (540, 30));

        let pixels = Level::Two.pixels();
        let error = PreparedDerivatives::<AtoB>::from_components(
            Level::Two,
            vec![0.0; pixels - 1],
            vec![0.0; pixels],
            vec![0.0; pixels],
            vec![0.0; pixels],
            vec![0.0; pixels],
            vec![0.0; pixels],
            vec![0.0; pixels],
            vec![0.0; pixels],
        )
        .unwrap_err();
        assert_eq!(error.part, "Ix plane");
        assert_eq!(error.expected, pixels);
        assert_eq!(error.actual, pixels - 1);
    }

    #[test]
    fn every_cardinal_mapping_crosses_color_on_odd_and_even_selected_widths() {
        for cols in [Level::Two.cols(), Level::One.cols()] {
            let rows = 4;
            let values: Vec<f32> = (0..rows * cols).map(|p| p as f32 + 0.25).collect();
            let packed = PackedPlane::from_row_major(rows, cols, &values);

            // Odd x on an odd row and even x on an even row exercise both
            // horizontal packed-slot offsets as well as both vertical rows.
            assert_cardinal_neighbours(&packed, 1, 7);
            assert_cardinal_neighbours(&packed, 2, 8);
        }
    }

    #[test]
    fn checkerboard_owns_even_parity_and_l2_odd_tail_alternates() {
        assert_eq!(PackedPlane::color(0, 0), Color::Red);
        assert_eq!(PackedPlane::color(0, 1), Color::Black);
        assert_eq!(PackedPlane::color(1, 0), Color::Black);
        assert_eq!(PackedPlane::color(1, 1), Color::Red);

        let l2_cols = Level::Two.cols();
        let row0_red = (PackedPlane::first_x(0, Color::Red)..l2_cols)
            .step_by(2)
            .count();
        let row0_black = (PackedPlane::first_x(0, Color::Black)..l2_cols)
            .step_by(2)
            .count();
        let row1_red = (PackedPlane::first_x(1, Color::Red)..l2_cols)
            .step_by(2)
            .count();
        let row1_black = (PackedPlane::first_x(1, Color::Black)..l2_cols)
            .step_by(2)
            .count();
        assert_eq!((row0_red, row0_black), (8, 7));
        assert_eq!((row1_red, row1_black), (7, 8));

        let l1_cols = Level::One.cols();
        for y in 0..2 {
            for color in Color::ALL {
                assert_eq!(
                    (PackedPlane::first_x(y, color)..l1_cols).step_by(2).count(),
                    15,
                );
            }
        }
    }

    #[test]
    fn total_borders_repeat_but_increment_and_weight_borders_stay_zero() {
        let rows = 3;
        let cols = Level::Two.cols();
        let values: Vec<f32> = (0..rows * cols).map(|p| p as f32 + 1.0).collect();
        let total = PackedPlane::from_row_major(rows, cols, &values);

        for y in 0..rows {
            let left_color = PackedPlane::color(y, 0);
            assert_eq!(
                total.neighbour(left_color, y, 0, Side::Left),
                total.get(y, 0),
            );
            let right_x = cols - 1;
            let right_color = PackedPlane::color(y, right_x);
            assert_eq!(
                total.neighbour(right_color, y, right_x, Side::Right),
                total.get(y, right_x),
            );
        }
        for x in 0..cols {
            let top_color = PackedPlane::color(0, x);
            assert_eq!(total.neighbour(top_color, 0, x, Side::Up), total.get(0, x),);
            let bottom_y = rows - 1;
            let bottom_color = PackedPlane::color(bottom_y, x);
            assert_eq!(
                total.neighbour(bottom_color, bottom_y, x, Side::Down),
                total.get(bottom_y, x),
            );
        }

        for mut zero_padded in [PackedPlane::zero(rows, cols), PackedPlane::zero(rows, cols)] {
            for y in 0..rows {
                for x in 0..cols {
                    zero_padded.set(y, x, 7.0);
                }
            }
            let left_color = PackedPlane::color(1, 0);
            let right_x = cols - 1;
            let right_color = PackedPlane::color(1, right_x);
            let top_color = PackedPlane::color(0, 4);
            let bottom_color = PackedPlane::color(rows - 1, 4);
            assert_eq!(zero_padded.neighbour(left_color, 1, 0, Side::Left), 0.0);
            assert_eq!(
                zero_padded.neighbour(right_color, 1, right_x, Side::Right),
                0.0,
            );
            assert_eq!(zero_padded.neighbour(top_color, 0, 4, Side::Up), 0.0);
            assert_eq!(
                zero_padded.neighbour(bottom_color, rows - 1, 4, Side::Down),
                0.0,
            );
            assert_eq!(zero_padded.get(1, right_x), 7.0);
            assert_eq!(zero_padded.get(rows - 1, 4), 7.0);
        }
    }

    #[test]
    fn one_weight_combines_right_and_down_total_flow_differences() {
        let mut solver = Solver::new(2, 2, &[0.0; 4], &[0.0; 4]);
        solver.total_dcol = PackedPlane::from_row_major(2, 2, &[0.0, 3.0, 0.0, 0.0]);
        solver.total_drow = PackedPlane::from_row_major(2, 2, &[0.0, 0.0, 4.0, 0.0]);

        let weight = solver.smoothness_weight(Color::Red, 0, 0);
        let expected = HALF_ALPHA / (3.0_f32 * 3.0 + 4.0 * 4.0 + EPSILON_SQUARED).sqrt();
        assert_close(weight, expected);
    }

    #[test]
    fn asymmetric_data_term_matches_the_eight_plane_hand_oracle() {
        let level = Level::Two;
        let pixels = level.pixels();
        let (y, x) = (3, 4);
        let p = y * level.cols() + x;
        let mut ix = vec![0.0; pixels];
        let mut iy = vec![0.0; pixels];
        let mut iz = vec![0.0; pixels];
        let mut ixx = vec![0.0; pixels];
        let mut ixy = vec![0.0; pixels];
        let mut iyy = vec![0.0; pixels];
        let mut ixz = vec![0.0; pixels];
        let mut iyz = vec![0.0; pixels];
        ix[p] = 3.0;
        iy[p] = 4.0;
        iz[p] = 5.0;
        ixx[p] = 1.0;
        ixy[p] = 2.0;
        iyy[p] = -3.0;
        ixz[p] = 4.0;
        iyz[p] = -5.0;
        let derivatives = PreparedDerivatives::<AtoB>::from_components(
            level, ix, iy, iz, ixx, ixy, iyy, ixz, iyz,
        )
        .unwrap();
        let zeros = vec![0.0; pixels];
        let mut solver = Solver::new(level.rows(), level.cols(), &zeros, &zeros);
        solver.increment_dcol.set(y, x, 0.25);
        solver.increment_drow.set(y, x, -0.5);

        solver.compute_data_term(&derivatives);

        // Independent binary32 hand fixture for Ix/Iy/Iz=(3,4,5),
        // Ixx/Ixy/Iyy/Ixz/Iyz=(1,2,-3,4,-5), and d=(0.25,-0.5).
        // Its unequal normalizers are Dx=5.01 and Dy=13.01, so swapping or
        // merging them, dropping a cross term, or changing an RHS sign fails.
        assert_close(solver.system.a11.get(y, x), 2.724_864);
        assert_close(solver.system.a12.get(y, x), 1.414_473_5);
        assert_close(solver.system.a22.get(y, x), 6.595_618);
        assert_close(solver.system.b1.get(y, x), -2.088_533_2);
        assert_close(solver.system.b2.get(y, x), -10.882_539);
    }

    #[test]
    fn edge_assembly_uses_real_edges_and_reuses_the_terminal_weight_downward() {
        let rows = 2;
        let cols = 3;
        let base_dcol = [0.0, 1.0, 3.0, 4.0, 9.0, 15.0];
        let base_drow = [0.0, -1.0, -3.0, -4.0, -9.0, -15.0];
        let mut solver = Solver::new(rows, cols, &base_dcol, &base_drow);
        solver.total_dcol = PackedPlane::from_row_major(rows, cols, &[0.0; 6]);
        solver.total_drow = PackedPlane::from_row_major(rows, cols, &[0.0; 6]);
        for y in 0..rows {
            for x in 0..cols {
                solver.system.a11.set(y, x, 1.0);
                solver.system.a12.set(y, x, 0.5);
                solver.system.a22.set(y, x, 2.0);
                solver.system.b1.set(y, x, 3.0);
                solver.system.b2.set(y, x, 4.0);
            }
        }

        solver.compute_horizontal_smoothness();
        solver.compute_vertical_smoothness();

        let weight = 10_000.0;
        assert_eq!(solver.system.weight.get(0, 2), weight);
        assert_eq!(solver.system.weight.get(1, 2), weight);
        assert_eq!(solver.system.a12.get(0, 2), 0.5);

        // Top-right has a real left and down edge, but no right or up edge.
        // Its own terminal-column weight supplies the down edge.
        assert_eq!(solver.system.a11.get(0, 2), 1.0 + 2.0 * weight);
        assert_eq!(solver.system.a22.get(0, 2), 2.0 + 2.0 * weight);
        assert_eq!(solver.system.b1.get(0, 2), 3.0 + 10.0 * weight);
        assert_eq!(solver.system.b2.get(0, 2), 4.0 - 10.0 * weight);

        // Bottom-right has only its real left and up edges. The stored
        // bottom/right weight creates neither an exterior diagonal nor RHS.
        assert_eq!(solver.system.a11.get(1, 2), 1.0 + 2.0 * weight);
        assert_eq!(solver.system.a22.get(1, 2), 2.0 + 2.0 * weight);
        assert_eq!(solver.system.b1.get(1, 2), 3.0 - 18.0 * weight);
        assert_eq!(solver.system.b2.get(1, 2), 4.0 + 18.0 * weight);

        let top_right_color = PackedPlane::color(0, 2);
        assert_eq!(
            solver
                .system
                .weight
                .neighbour(top_right_color, 0, 2, Side::Right),
            0.0,
        );
        assert_eq!(
            solver
                .system
                .weight
                .neighbour(top_right_color, 0, 2, Side::Up),
            0.0,
        );
    }

    #[test]
    fn data_overwrites_prior_round_smoothness_instead_of_accumulating_it() {
        let level = Level::Two;
        let derivatives = constant_derivatives::<AtoB>(level, 1.0, 0.25, 0.5);
        let mut solver = Solver::new(
            level.rows(),
            level.cols(),
            &vec![0.0; level.pixels()],
            &vec![0.0; level.pixels()],
        );
        let (y, x) = (4, 4);

        solver.compute_data_term(&derivatives);
        let data_a11 = solver.system.a11.get(y, x);
        let data_b1 = solver.system.b1.get(y, x);
        solver.compute_horizontal_smoothness();
        solver.compute_vertical_smoothness();
        assert!(solver.system.a11.get(y, x) > data_a11);

        solver.compute_data_term(&derivatives);
        assert_eq!(solver.system.a11.get(y, x).to_bits(), data_a11.to_bits());
        assert_eq!(solver.system.b1.get(y, x).to_bits(), data_b1.to_bits());
    }

    #[test]
    fn red_then_black_reads_the_fresh_opposite_color() {
        let mut solver = Solver::new(1, 2, &[0.0; 2], &[0.0; 2]);
        for x in 0..2 {
            solver.system.a11.set(0, x, 10.0);
            solver.system.a12.set(0, x, 0.0);
            solver.system.a22.set(0, x, 10.0);
            solver.system.b1.set(0, x, 0.0);
            solver.system.b2.set(0, x, 0.0);
            solver.system.weight.set(0, x, 1.0);
        }
        solver.increment_dcol.set(0, 1, 2.0);

        solver.sor_color(Color::Red, 1.0);
        assert_close(solver.increment_dcol.get(0, 0), 0.2);
        solver.sor_color(Color::Black, 1.0);
        assert_close(solver.increment_dcol.get(0, 1), 0.02);
    }

    #[test]
    fn componentwise_gauss_seidel_uses_new_column_increment_for_row() {
        let mut solver = Solver::new(1, 1, &[0.0], &[0.0]);
        solver.system.a11.set(0, 0, 2.0);
        solver.system.a12.set(0, 0, 1.0);
        solver.system.a22.set(0, 0, 4.0);
        solver.system.b1.set(0, 0, 2.0);
        solver.system.b2.set(0, 0, 4.0);

        solver.sor_color(Color::Red, 1.0);
        assert_close(solver.increment_dcol.get(0, 0), 1.0);
        assert_close(solver.increment_drow.get(0, 0), 0.75);
    }

    #[test]
    fn all_five_rounds_reuse_one_preparation_and_persist_increments() {
        let level = Level::Two;
        let derivatives = constant_derivatives::<AtoB>(level, 1.0, 0.0, -0.25);
        let original_iz = derivatives.iz.to_vec();
        let zeros = vec![0.0; level.pixels()];
        let mut five_rounds = Solver::new(level.rows(), level.cols(), &zeros, &zeros);
        let mut manual_five = Solver::new(level.rows(), level.cols(), &zeros, &zeros);
        let mut one_round = Solver::new(level.rows(), level.cols(), &zeros, &zeros);

        five_rounds.run(&derivatives);
        for _ in 0..FIXED_POINT_ROUNDS {
            manual_five.fixed_point_round(&derivatives);
        }
        one_round.fixed_point_round(&derivatives);

        let centre = (level.rows() / 2, level.cols() / 2);
        let five = five_rounds.increment_dcol.get(centre.0, centre.1);
        let manual = manual_five.increment_dcol.get(centre.0, centre.1);
        let one = one_round.increment_dcol.get(centre.0, centre.1);
        assert_eq!(five.to_bits(), manual.to_bits());
        assert_ne!(five.to_bits(), one.to_bits());
        assert_ne!(five.to_bits(), 0.0_f32.to_bits());
        assert_eq!(&*derivatives.iz, &original_iz);
    }

    #[test]
    fn rollback_threshold_is_strict_level_cols_with_native_nonfinite_rules() {
        for cols in [Level::Two.cols(), Level::One.cols()] {
            let limit = cols as f32;
            let just_above = f32::from_bits(limit.to_bits() + 1);
            assert!(!requires_whole_level_rollback(cols, &[limit], &[0.0]));
            assert!(!requires_whole_level_rollback(
                cols,
                &[-limit],
                &[f32::NEG_INFINITY]
            ));
            assert!(requires_whole_level_rollback(cols, &[just_above], &[0.0]));
            assert!(requires_whole_level_rollback(
                cols,
                &[f32::INFINITY],
                &[0.0]
            ));
            assert!(requires_whole_level_rollback(cols, &[0.0], &[f32::NAN]));
        }
    }

    #[test]
    fn one_invalid_component_restores_both_whole_planes() {
        let level = Level::Two;
        let derivatives = constant_derivatives::<AtoB>(level, 0.0, 0.0, 0.0);
        let mut original_dcol = vec![0.0; level.pixels()];
        original_dcol[17] = f32::from_bits(0x7fc0_0000);
        let original_drow: Vec<f32> = (0..level.pixels())
            .map(|p| (p % 9) as f32 * 0.125)
            .collect();
        let field = DenseField::<AtoB>::from_test_components(
            level,
            original_dcol.clone(),
            original_drow.clone(),
        )
        .unwrap();

        let result = refine(&derivatives, field).unwrap();
        assert!(result.rolled_back());
        assert_eq!(
            result
                .field()
                .dcol()
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
            original_dcol
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>(),
        );
        assert_eq!(result.field().drow(), &original_drow);
    }

    #[test]
    fn refinement_rejects_a_same_direction_field_from_another_level() {
        let derivatives = constant_derivatives::<AtoB>(Level::One, 0.0, 0.0, 0.0);
        let field = DenseField::<AtoB>::from_test_components(
            Level::Two,
            vec![0.0; Level::Two.pixels()],
            vec![0.0; Level::Two.pixels()],
        )
        .unwrap();
        let error = refine(&derivatives, field).unwrap_err();
        assert_eq!(error.derivatives, Level::One);
        assert_eq!(error.field, Level::Two);
        assert_eq!(error.direction, Direction::AtoB);
    }
}
