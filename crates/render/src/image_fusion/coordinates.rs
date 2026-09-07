//! Selected X4 ratio-map coordinates recovered from the static producer.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::studio_type2::{MAP_HEIGHT, MAP_NODES, MAP_WIDTH};

const EPSILON: f32 = f32::from_bits(0x2edb_e6ff);

/// Produce the selected `Ry(-pi/2)` remap in final 200 by 100 chart order.
///
/// The scalar statement order follows the verified producer. In particular,
/// trigonometric results are evaluated rather than replaced with ideal-axis
/// constants, and the upper clamps leave an unordered value unchanged.
pub(super) fn selected_x4() -> Vec<[f32; 2]> {
    let width = MAP_WIDTH as f32;
    let height = MAP_HEIGHT as f32;
    let upper_x = (MAP_WIDTH - 1) as f32;
    let upper_y = (MAP_HEIGHT - 1) as f32;

    let angle = -FRAC_PI_2;
    let cb = angle.cos();
    let sb = angle.sin();
    let neg_sb = -sb;

    let mut coordinates = Vec::with_capacity(MAP_NODES);
    for y in 0..MAP_HEIGHT {
        let theta = (y as f32 * PI) / height;
        let st = theta.sin();
        let ct = theta.cos();
        for x in 0..MAP_WIDTH {
            let phi = (x as f32 * TAU) / width;
            let qx = st * phi.cos();
            let qy = st * phi.sin();
            let qz = ct;

            let vx = cb * qx + sb * qz;
            let vy = qy;
            let vz = neg_sb * qx + cb * qz;

            let radius = (vx * vx + vy * vy + EPSILON).sqrt();
            let mut azimuth = (vx / radius).acos();
            if vy < 0.0 {
                azimuth = TAU - azimuth;
            }

            let raw_x = ((width * azimuth) / PI) * 0.5;
            let raw_y = (upper_y * vz.acos()) / PI;
            coordinates.push([
                ordered_upper(raw_x, upper_x) + 1.0,
                ordered_upper(raw_y, upper_y),
            ]);
        }
    }
    coordinates
}

fn ordered_upper(value: f32, upper: f32) -> f32 {
    if value > upper { upper } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(coordinates: &[[f32; 2]], row: usize, column: usize) -> [f32; 2] {
        coordinates[row * MAP_WIDTH + column]
    }

    fn near(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "coordinate {actual} differs from {expected} by more than {tolerance}"
        );
    }

    #[test]
    fn selected_coordinates_fill_the_final_chart_without_half_texels() {
        let coordinates = selected_x4();
        assert_eq!(coordinates.len(), MAP_NODES);
        for (index, [x, y]) in coordinates.iter().copied().enumerate() {
            assert!(
                x.is_finite() && y.is_finite(),
                "coordinate {index} is {:?}",
                [x, y]
            );
            assert!(
                (1.0..=MAP_WIDTH as f32).contains(&x),
                "coordinate {index} x={x}"
            );
            assert!(
                (0.0..=(MAP_HEIGHT - 1) as f32).contains(&y),
                "coordinate {index} y={y}"
            );
        }

        // The equator at phi=pi/2 maps to the +Y azimuth: 50 plus the
        // producer's periodic-scratch +1, not a texel-centred 51.5.
        let quarter = at(&coordinates, MAP_HEIGHT / 2, MAP_WIDTH / 4);
        near(quarter[0], 51.0, 0.05);
        near(quarter[1], 49.5, 0.05);
    }

    #[test]
    fn landmarks_authenticate_full_azimuth_and_negative_y_rotation() {
        let coordinates = selected_x4();
        let equator_front = at(&coordinates, MAP_HEIGHT / 2, 0);
        let equator_quarter = at(&coordinates, MAP_HEIGHT / 2, MAP_WIDTH / 4);
        let equator_three_quarters = at(&coordinates, MAP_HEIGHT / 2, 3 * MAP_WIDTH / 4);
        let north = at(&coordinates, 0, 0);

        // Ry(-pi/2) moves the phi=0 equator to +Z. The opposite sign would
        // put it at the lower pole.
        near(equator_front[1], 0.0, 0.05);
        // TAU makes both quarter turns land on the chart's mid-latitude.
        // Using PI for phi instead would put them near one and three quarters.
        near(equator_quarter[1], 49.5, 0.05);
        near(equator_three_quarters[1], 49.5, 0.05);
        near(equator_quarter[0], 51.0, 0.05);
        near(equator_three_quarters[0], 151.0, 0.05);
        // The input north pole rotates to azimuth pi, independent of x.
        near(north[0], 101.0, 0.05);
        near(north[1], 49.5, 0.05);
    }

    #[test]
    fn ordered_upper_clamp_does_not_replace_nan() {
        assert!(ordered_upper(f32::NAN, 7.0).is_nan());
        assert_eq!(ordered_upper(8.0, 7.0), 7.0);
        assert_eq!(ordered_upper(-1.0, 7.0), -1.0);
    }
}
