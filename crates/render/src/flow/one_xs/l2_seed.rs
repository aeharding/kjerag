//! Typed level-two post-update to level-one PIS seed adapter.
//!
//! Native first resizes each post-update component with `INTER_LINEAR`, then
//! doubles it into level-one pixel units. The first PIS pass samples those
//! planes directly at each 8-by-8 patch centre. There is no second
//! interpolation, clamp, or integer conversion at this boundary.

use std::error::Error;
use std::fmt;

use super::dense::{DenseField, WrongLevel, upsample_linear_x2};
use super::pis::{Flow, InitialGrid, Level, PisDirection};
use super::post_update::PostUpdateDense;
use super::{Direction, PATCH_SIZE, PATCH_STRIDE};

/// A post-update field cannot form the selected level-one PIS seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedError {
    /// The consumed post-update token did not belong to level two.
    WrongLevel(WrongLevel),
    /// A directly sampled patch centre contained a non-finite component.
    NonFiniteCenter {
        direction: Direction,
        component: &'static str,
        patch_row: usize,
        patch_col: usize,
        dense_row: usize,
        dense_col: usize,
    },
}

impl From<WrongLevel> for SeedError {
    fn from(error: WrongLevel) -> Self {
        Self::WrongLevel(error)
    }
}

impl fmt::Display for SeedError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLevel(error) => error.fmt(out),
            Self::NonFiniteCenter {
                direction,
                component,
                patch_row,
                patch_col,
                dense_row,
                dense_col,
            } => write!(
                out,
                "ONE X2 {direction} level-one initial {component} at patch row {patch_row} column {patch_col}, dense row {dense_row} column {dense_col}, is not finite",
            ),
        }
    }
}

impl Error for SeedError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::WrongLevel(error) => Some(error),
            Self::NonFiniteCenter { .. } => None,
        }
    }
}

/// Consume a selected level-two post-update field and form the level-one PIS
/// initial grid.
///
/// Flows are returned in patch-row-major order. Patch `(r, c)` reads the
/// upsampled components at dense pixel `(3*r + 4, 3*c + 4)` without further
/// interpolation or rounding.
///
/// The raw L2-to-L1 resize is not a public bypass around this adapter:
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::dense::upsample_linear_x2;
/// ```
pub fn into_l1_initial_grid<D: PisDirection>(
    post_update: PostUpdateDense<D>,
) -> Result<InitialGrid<D>, SeedError> {
    let finest = upsample_linear_x2(post_update)?;
    initial_grid_from_upsampled(finest)
}

fn initial_grid_from_upsampled<D: PisDirection>(
    finest: DenseField<D>,
) -> Result<InitialGrid<D>, SeedError> {
    debug_assert_eq!(finest.level(), Level::One);
    let level = Level::One;
    let mut flows = Vec::with_capacity(level.patches());

    for patch_row in 0..level.patch_rows() {
        let dense_row = patch_row * PATCH_STRIDE + PATCH_SIZE / 2;
        for patch_col in 0..level.patch_cols() {
            let dense_col = patch_col * PATCH_STRIDE + PATCH_SIZE / 2;
            let dense_index = dense_row * level.cols() + dense_col;
            let dcol = finest.dcol()[dense_index];
            let drow = finest.drow()[dense_index];

            for (component, value) in [("dcol", dcol), ("drow", drow)] {
                if !value.is_finite() {
                    return Err(SeedError::NonFiniteCenter {
                        direction: D::DIRECTION,
                        component,
                        patch_row,
                        patch_col,
                        dense_row,
                        dense_col,
                    });
                }
            }
            flows.push(
                Flow::new(dcol, drow)
                    .expect("validated ONE X2 level-one PIS seed became non-finite"),
            );
        }
    }

    Ok(InitialGrid::from_l1_row_major(flows)
        .expect("ONE X2 level-one PIS seed has the native patch count"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::one_xs::dense::NO_COVER_NAN;
    use crate::flow::one_xs::pis::{AtoB, BtoA};

    fn affine_components(level: Level) -> (Vec<f32>, Vec<f32>) {
        let mut dcol = Vec::with_capacity(level.pixels());
        let mut drow = Vec::with_capacity(level.pixels());
        for row in 0..level.rows() {
            for col in 0..level.cols() {
                dcol.push(row as f32 * 100.0 + col as f32);
                drow.push(row as f32 - col as f32 * 10.0);
            }
        }
        (dcol, drow)
    }

    fn post_update<D: PisDirection>(
        level: Level,
        dcol: Vec<f32>,
        drow: Vec<f32>,
    ) -> PostUpdateDense<D> {
        PostUpdateDense::from_test_field(
            DenseField::from_test_components(level, dcol, drow).unwrap(),
            false,
        )
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 1.0e-4,
            "actual {actual:?}, expected {expected:?}",
        );
    }

    #[test]
    fn l2_resize_scale_and_l1_center_fetch_keep_native_order_and_units() {
        let level = Level::Two;
        let (mut dcol, mut drow) = affine_components(level);

        // A synthetic non-finite tail proves that no L1 patch centre's resize
        // footprint reaches the last L2 row or column. Selected densification
        // itself supplies finite values there by extending its last patch.
        for row in 0..level.rows() {
            let index = row * level.cols() + level.cols() - 1;
            dcol[index] = NO_COVER_NAN;
            drow[index] = NO_COVER_NAN;
        }
        for col in 0..level.cols() {
            let index = (level.rows() - 1) * level.cols() + col;
            dcol[index] = NO_COVER_NAN;
            drow[index] = NO_COVER_NAN;
        }

        let initial = into_l1_initial_grid(post_update::<AtoB>(level, dcol, drow)).unwrap();
        assert_eq!(initial.level(), Level::One);
        assert_eq!(initial.flows().len(), Level::One.patches());

        for patch_row in 0..Level::One.patch_rows() {
            for patch_col in 0..Level::One.patch_cols() {
                let at = patch_row * Level::One.patch_cols() + patch_col;
                let flow = initial.flows()[at];
                let source_row = patch_row as f32 * 1.5 + 1.75;
                let source_col = patch_col as f32 * 1.5 + 1.75;
                assert_close(flow.dcol(), 2.0 * (source_row * 100.0 + source_col));
                assert_close(flow.drow(), 2.0 * (source_row - source_col * 10.0));
            }
        }

        let last = initial.flows()[Level::One.patches() - 1];
        assert_close(last.dcol(), 2.0 * (267.25 * 100.0 + 12.25));
        assert_close(last.drow(), 2.0 * (267.25 - 12.25 * 10.0));
    }

    #[test]
    fn direct_l1_fetch_uses_integer_centres_without_touching_other_pixels() {
        let level = Level::One;
        let mut dcol = vec![NO_COVER_NAN; level.pixels()];
        let mut drow = vec![NO_COVER_NAN; level.pixels()];
        for patch_row in 0..level.patch_rows() {
            let row = patch_row * PATCH_STRIDE + PATCH_SIZE / 2;
            for patch_col in 0..level.patch_cols() {
                let col = patch_col * PATCH_STRIDE + PATCH_SIZE / 2;
                let pixel = row * level.cols() + col;
                let patch = patch_row * level.patch_cols() + patch_col;
                dcol[pixel] = patch as f32 + 0.25;
                drow[pixel] = -(patch as f32) - 0.5;
            }
        }

        let finest = DenseField::<BtoA>::from_test_components(level, dcol, drow).unwrap();
        let initial = initial_grid_from_upsampled(finest).unwrap();
        for (patch, flow) in initial.flows().iter().copied().enumerate() {
            assert_eq!(flow.dcol(), patch as f32 + 0.25);
            assert_eq!(flow.drow(), -(patch as f32) - 0.5);
        }
    }

    #[test]
    fn rejects_nonfinite_sampled_centres_and_non_l2_tokens() {
        let level = Level::One;
        let wrong_level =
            post_update::<AtoB>(level, vec![0.0; level.pixels()], vec![0.0; level.pixels()]);
        assert!(matches!(
            into_l1_initial_grid(wrong_level),
            Err(SeedError::WrongLevel(_))
        ));

        let mut dcol = vec![0.0; level.pixels()];
        let drow = vec![0.0; level.pixels()];
        dcol[4 * level.cols() + 4] = f32::INFINITY;
        let error = initial_grid_from_upsampled(
            DenseField::<BtoA>::from_test_components(level, dcol, drow).unwrap(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            SeedError::NonFiniteCenter {
                direction: Direction::BtoA,
                component: "dcol",
                patch_row: 0,
                patch_col: 0,
                dense_row: 4,
                dense_col: 4,
            }
        );
    }
}
