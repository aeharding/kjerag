//! Selected X4 ratio-map coordinates recovered from the static producer.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use crate::stitch_camera::StitchCamera;
use crate::studio_type2::{MAP_HEIGHT, MAP_WIDTH};

const EPSILON: f32 = f32::from_bits(0x2edb_e6ff);

/// Produce the selected `Ry(-pi/2)` remap in final 200 by 100 chart order.
///
/// The scalar statement order follows the verified producer. In particular,
/// trigonometric results are evaluated rather than replaced with ideal-axis
/// constants, and the upper clamps leave an unordered value unchanged.
pub(super) fn selected_x4() -> Vec<[f32; 2]> {
    selected(-FRAC_PI_2, 0..MAP_HEIGHT, false)
}

/// Selected source-map lookup, before endpoint band expansion. These are the
/// actual working rows 48..51 of the positive-rotation chart, not the inverse
/// of the later ratio lookup sampled on a 99-step sphere.
pub(super) fn selected_x4_band() -> Vec<[f32; 2]> {
    selected(FRAC_PI_2, 48..52, true)
}

/// Ratio-map publication coordinates in the selected camera's chart.
///
/// ONE X2 keeps the recovered native inverse. X4's fixed `Ry(pi)` body datum
/// reflects the consumed native ratio texture. That reflection is an exact
/// permutation of bilinear texel centers, not a second evaluation of the
/// producer's endpoint lattice: row/column `(r,c)` reads native
/// `(99-r,(99-c) mod 200)`.
pub(super) fn for_camera_output(camera: StitchCamera) -> Vec<[f32; 2]> {
    match camera {
        StitchCamera::OneX2 => selected_x4(),
        StitchCamera::CalibratedMei => reindex_x4_output(&selected_x4()),
    }
}

fn reindex_x4_output<T: Copy>(native: &[T]) -> Vec<T> {
    assert_eq!(native.len(), MAP_WIDTH * MAP_HEIGHT);
    let mut output = Vec::with_capacity(native.len());
    for row in 0..MAP_HEIGHT {
        for column in 0..MAP_WIDTH {
            let native_row = MAP_HEIGHT - 1 - row;
            let native_column = (MAP_WIDTH / 2 - 1 + MAP_WIDTH - column) % MAP_WIDTH;
            output.push(native[native_row * MAP_WIDTH + native_column]);
        }
    }
    output
}

/// Source-band lookup in the selected camera's packed-map chart.
///
/// X4's packed map has already crossed the fixed `Ry(pi)` datum at the camera
/// boundary, so native's positive quarter turn becomes a negative one.
pub(super) fn for_camera_band(camera: StitchCamera) -> Vec<[f32; 2]> {
    match camera {
        StitchCamera::OneX2 => selected_x4_band(),
        StitchCamera::CalibratedMei => selected(-FRAC_PI_2, 48..52, true),
    }
}

fn selected(angle: f32, rows: std::ops::Range<usize>, band: bool) -> Vec<[f32; 2]> {
    let width = MAP_WIDTH as f32;
    let height = MAP_HEIGHT as f32;
    let upper_x = (MAP_WIDTH - 1) as f32;
    let upper_y = (MAP_HEIGHT - 1) as f32;

    let cb = angle.cos();
    let sb = angle.sin();
    let neg_sb = -sb;

    let mut coordinates = Vec::with_capacity(rows.len() * MAP_WIDTH);
    for y in rows {
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

            let raw_x = if band {
                width * azimuth / TAU
            } else {
                ((width * azimuth) / PI) * 0.5
            };
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
    use crate::image_fusion::RatioMap;
    use crate::studio_type2::MAP_NODES;

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

    #[test]
    fn source_band_has_four_rows_and_positive_rotation_landmarks() {
        let coords = selected_x4_band();
        assert_eq!(coords.len(), MAP_WIDTH * 4);
        assert!(coords.iter().flatten().all(|v| v.is_finite()));
        near(coords[0][0], 1.0, 0.05);
        near(coords[0][1], 97.02, 0.01);
        near(coords[MAP_WIDTH][1], 98.01, 0.01);
        // The equatorial quarter turn has a stable nonsingular azimuth.
        near(coords[2 * MAP_WIDTH + 50][0], 51.0, 0.05);
        near(coords[2 * MAP_WIDTH + 50][1], 49.5, 0.05);
        // Its three-quarter counterpart is 151, not a half-circle chart.
        near(coords[2 * MAP_WIDTH + 150][0], 151.0, 0.05);
        near(coords[2 * MAP_WIDTH + 150][1], 49.5, 0.05);
    }

    #[test]
    fn one_x2_camera_helpers_preserve_the_recovered_tables_bit_for_bit() {
        assert_eq!(for_camera_band(StitchCamera::OneX2), selected_x4_band());
        assert_eq!(for_camera_output(StitchCamera::OneX2), selected_x4());
    }

    #[test]
    fn x4_body_datum_composes_continuous_quarter_turns_on_the_same_ray() {
        let rotate_y = |angle: f32, [x, y, z]: [f32; 3]| {
            let (s, c) = angle.sin_cos();
            [c * x + s * z, y, -s * x + c * z]
        };
        let native = [0.25_f32, -0.5, 0.75];
        let bridged_band = rotate_y(PI, rotate_y(FRAC_PI_2, native));
        let direct_band = rotate_y(-FRAC_PI_2, native);
        let bridged_output = rotate_y(PI, rotate_y(-FRAC_PI_2, native));
        let direct_output = rotate_y(FRAC_PI_2, native);
        for (actual, expected) in bridged_band.into_iter().zip(direct_band) {
            near(actual, expected, 2.0e-7);
        }
        for (actual, expected) in bridged_output.into_iter().zip(direct_output) {
            near(actual, expected, 2.0e-7);
        }
    }

    #[test]
    fn x4_source_band_lookup_remains_the_negative_quarter_turn() {
        assert_eq!(
            for_camera_band(StitchCamera::CalibratedMei),
            selected(-FRAC_PI_2, 48..52, true)
        );
    }

    #[test]
    fn x4_production_output_table_is_the_exact_native_texture_permutation() {
        let native = selected_x4();
        let actual = for_camera_output(StitchCamera::CalibratedMei);
        assert_eq!(actual.len(), MAP_NODES);
        for row in 0..MAP_HEIGHT {
            for column in 0..MAP_WIDTH {
                let native_row = MAP_HEIGHT - 1 - row;
                let native_column = (MAP_WIDTH / 2 - 1 + MAP_WIDTH - column) % MAP_WIDTH;
                assert_eq!(
                    actual[row * MAP_WIDTH + column],
                    native[native_row * MAP_WIDTH + native_column],
                    "X4 output coordinate differs at row {row}, column {column}"
                );
            }
        }
        assert_ne!(
            actual,
            selected(FRAC_PI_2, 0..MAP_HEIGHT, false),
            "endpoint-lattice recomputation must remain distinct from texture rebasing"
        );
    }

    #[test]
    fn x4_output_permutation_commutes_with_ratio_texture_sampling() {
        let values = (0..MAP_NODES)
            .map(|node| {
                let column = (node % MAP_WIDTH) as f32;
                let row = (node / MAP_WIDTH) as f32;
                [
                    0.25 + column * 0.003 + row * 0.0007,
                    0.5 + column * 0.0002 + row * 0.004,
                    0.75 + column * 0.001 + row * 0.002,
                    column - row * 0.5,
                ]
            })
            .collect();
        let native = RatioMap::new(values).unwrap();
        let rebased = RatioMap::new(reindex_x4_output(native.values())).unwrap();
        let points: [[f32; 2]; 7] = [
            [0.0013, 0.0017],
            [0.9981, 0.9989],
            [0.2473, 0.3821],
            [0.5037, 0.6159],
            [0.7511, 0.9437],
            [0.0, 0.0],
            [1.0, 1.0],
        ];
        for uv in points {
            let native_uv = [(0.5 - uv[0]).rem_euclid(1.0), 1.0 - uv[1]];
            let expected = native.sample(native_uv);
            let actual = rebased.sample(uv);
            for channel in 0..3 {
                let difference = (actual[channel] - expected[channel]).abs();
                assert!(
                    difference <= 2.0e-5,
                    "sample {uv:?} channel {channel} differs by {difference}, above binary32 tolerance"
                );
            }
        }
    }
}
