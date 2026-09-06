//! Static selected ONE X2 resources generated from READ Studio semantics.
//!
//! These maps contain no frame or captured payload. The two retained base
//! coordinate maps, the common displacement gates, the shared `+0x810`
//! sphere-to-line coordinates and the final Template alpha depend only on the
//! selected camera calibration and fixed resource geometry.

use std::error::Error;
use std::fmt;

use kjerag_meta::Lens;

use crate::projection;
use crate::stitch_camera::StitchCamera;
use crate::studio_type2::{AlphaMap, MAP_HEIGHT, MAP_NODES, MAP_WIDTH};

use super::base_map::{StaticLineCoordinates, one_xs_static_coordinates};
use super::map_patch::{CoordinateMap, GateMap};
use super::{BodyRay, Layout, LensPair};

const GATE_WIDTH: f32 = 16.0_f32.to_radians();

/// The static resources consumed by the selected ONE X2 map transaction.
#[derive(Clone, Debug, PartialEq)]
pub struct OneXsResources {
    static_coordinates: LensPair<StaticLineCoordinates>,
    gates: LensPair<GateMap>,
    coordinates: LensPair<CoordinateMap>,
    alpha: AlphaMap,
}

impl OneXsResources {
    /// Generate every frame-invariant selected ONE X2 map from its calibration.
    pub fn new(lenses: &[Lens]) -> Result<Self, ResourceError> {
        Self::build(lenses, None)
    }

    /// Shared solver resources with camera-owned final blend weights.
    /// The native ONE X2 resource construction remains bit-for-bit unchanged.
    pub(crate) fn for_camera(
        camera: StitchCamera,
        lenses: &[Lens],
        size: crate::Size,
    ) -> Result<Self, ResourceError> {
        if StitchCamera::from_lenses(lenses) != Some(camera) {
            return Err(ResourceError::NotOneXsPair);
        }
        match camera {
            StitchCamera::OneX2 => Self::new(lenses),
            StitchCamera::CalibratedMei => {
                let reframe = crate::Reframe::new(
                    lenses,
                    size,
                    crate::Camera::default(),
                    crate::Held::default(),
                    1.0,
                    false,
                    crate::Sampling::default(),
                );
                Self::build(lenses, Some(&reframe))
            }
        }
    }

    fn build(lenses: &[Lens], calibrated: Option<&crate::Reframe>) -> Result<Self, ResourceError> {
        let mut left_gate = Vec::with_capacity(MAP_NODES);
        let mut right_gate = Vec::with_capacity(MAP_NODES);
        let mut line_coordinates = Vec::with_capacity(MAP_NODES);
        let mut alpha = Vec::with_capacity(MAP_NODES);

        for row in 0..MAP_HEIGHT {
            for column in 0..MAP_WIDTH {
                let body = sphere_node(row, column);
                let left = (0.5 + body[2].clamp(-1.0, 1.0).asin() / GATE_WIDTH).clamp(0.0, 1.0);
                left_gate.push(left);
                right_gate.push(1.0 - left);

                let sample = Layout
                    .sample(BodyRay::new(body).expect("sphere nodes are finite unit directions"))
                    .expect("sphere nodes have a defined ONE X2 line coordinate");
                line_coordinates.push([sample.col(), sample.row()]);

                alpha.push(match calibrated {
                    Some(reframe) => reframe.blend(body).weights[0],
                    None => projection::one_xs_alpha_at_body(lenses, body)
                        .ok_or(ResourceError::NotOneXsPair)?,
                });
            }
        }

        // Studio overwrites the singular pole rows after evaluating the regular
        // sphere nodes. Preserve the exact row ownership in the uploaded raster.
        alpha.copy_within(MAP_WIDTH..2 * MAP_WIDTH, 0);
        alpha.copy_within(
            (MAP_HEIGHT - 2) * MAP_WIDTH..(MAP_HEIGHT - 1) * MAP_WIDTH,
            (MAP_HEIGHT - 1) * MAP_WIDTH,
        );

        let shared = CoordinateMap::new(line_coordinates)
            .expect("type-2 dimensions establish coordinate shape");
        Ok(Self {
            static_coordinates: one_xs_static_coordinates(),
            gates: LensPair {
                a: GateMap::new(left_gate).expect("type-2 dimensions establish gate shape"),
                b: GateMap::new(right_gate).expect("type-2 dimensions establish gate shape"),
            },
            coordinates: LensPair {
                a: shared.clone(),
                b: shared,
            },
            alpha: AlphaMap::new(alpha).expect("type-2 dimensions establish alpha shape"),
        })
    }

    pub fn static_coordinates(&self) -> &LensPair<StaticLineCoordinates> {
        &self.static_coordinates
    }

    /// The common SphereAlpha gates: A is `+0x1ab0`, B is exact `1-A`.
    pub fn gates(&self) -> &LensPair<GateMap> {
        &self.gates
    }

    /// The shared `+0x810` map duplicated for the direction-named consumers.
    /// Both maps store each node as `[column, row]`.
    pub fn coordinates(&self) -> &LensPair<CoordinateMap> {
        &self.coordinates
    }

    /// The final selected left Template-alpha raster.
    pub fn alpha(&self) -> &AlphaMap {
        &self.alpha
    }
}

fn sphere_node(row: usize, column: usize) -> [f32; 3] {
    debug_assert!(row < MAP_HEIGHT && column < MAP_WIDTH);
    let theta = row as f32 * std::f32::consts::PI / (MAP_HEIGHT - 1) as f32;
    let phi = std::f32::consts::TAU - column as f32 * std::f32::consts::TAU / MAP_WIDTH as f32;
    let (sin_theta, cos_theta) = theta.sin_cos();
    let (sin_phi, cos_phi) = phi.sin_cos();
    [sin_theta * sin_phi, cos_theta, sin_theta * cos_phi]
}

/// Why a static resource request cannot be fulfilled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceError {
    NotOneXsPair,
}

impl fmt::Display for ResourceError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotOneXsPair => {
                out.write_str("ONE X2 static resources require a selected two-lens ONE X2 camera")
            }
        }
    }
}

impl Error for ResourceError {}

#[cfg(test)]
mod tests {
    use crate::projection::tests::{FRAME, fixture_lenses, one_xs_lenses};
    use crate::stitch_camera::StitchCamera;
    use crate::{Camera, Held, Sampling, Size};

    use super::*;
    use crate::flow::one_xs::base_map::{SELECTED_LINE_COLS, SELECTED_LINE_ROWS};

    fn resources() -> OneXsResources {
        OneXsResources::new(&one_xs_lenses()).unwrap()
    }

    #[test]
    fn resources_have_the_read_shapes() {
        let resources = resources();
        for coordinates in [
            &resources.static_coordinates().a,
            &resources.static_coordinates().b,
        ] {
            assert_eq!(coordinates.rows(), SELECTED_LINE_ROWS);
            assert_eq!(coordinates.cols(), SELECTED_LINE_COLS);
            assert_eq!(
                coordinates.row_major_values().len(),
                SELECTED_LINE_ROWS * SELECTED_LINE_COLS
            );
        }
        assert_eq!(resources.gates().a.values().len(), MAP_NODES);
        assert_eq!(resources.gates().b.values().len(), MAP_NODES);
        assert_eq!(resources.coordinates().a.values().len(), MAP_NODES);
        assert_eq!(resources.coordinates().b.values().len(), MAP_NODES);
        assert_eq!(resources.alpha().nodes().len(), MAP_NODES);
    }

    #[test]
    fn gates_are_complements_and_saturate_on_opposite_sides() {
        let resources = resources();
        for (&left, &right) in resources
            .gates()
            .a
            .values()
            .iter()
            .zip(resources.gates().b.values())
        {
            assert_eq!(right.to_bits(), (1.0_f32 - left).to_bits());
        }

        let equator = MAP_HEIGHT / 2;
        let positive = equator * MAP_WIDTH;
        let negative = positive + MAP_WIDTH / 2;
        assert_eq!(resources.gates().a.values()[positive], 1.0);
        assert_eq!(resources.gates().b.values()[positive], 0.0);
        assert_eq!(resources.gates().a.values()[negative], 0.0);
        assert_eq!(resources.gates().b.values()[negative], 1.0);
    }

    #[test]
    fn shared_coordinates_are_row_major_column_then_row() {
        let resources = resources();
        let row = 37;
        let column = 83;
        let index = row * MAP_WIDTH + column;
        let expected = Layout
            .sample(BodyRay::new(sphere_node(row, column)).unwrap())
            .unwrap();
        let actual = resources.coordinates().a.values()[index];
        assert_eq!(actual, [expected.col(), expected.row()]);
        assert_ne!(actual, [expected.row(), expected.col()]);

        let next = resources.coordinates().a.values()[index + 1];
        let next_expected = Layout
            .sample(BodyRay::new(sphere_node(row, column + 1)).unwrap())
            .unwrap();
        assert_eq!(next, [next_expected.col(), next_expected.row()]);
        assert_eq!(resources.coordinates().a, resources.coordinates().b);
    }

    #[test]
    fn alpha_copies_the_adjacent_rows_into_both_poles() {
        let resources = resources();
        let alpha = resources.alpha().nodes();
        assert_eq!(&alpha[..MAP_WIDTH], &alpha[MAP_WIDTH..2 * MAP_WIDTH]);
        assert_eq!(
            &alpha[(MAP_HEIGHT - 1) * MAP_WIDTH..],
            &alpha[(MAP_HEIGHT - 2) * MAP_WIDTH..(MAP_HEIGHT - 1) * MAP_WIDTH]
        );
        assert_ne!(
            &alpha[MAP_WIDTH..2 * MAP_WIDTH],
            &alpha[2 * MAP_WIDTH..3 * MAP_WIDTH]
        );
    }

    #[test]
    fn calibrated_x4_resources_reuse_geometry_with_calibrated_alpha() {
        let lenses = fixture_lenses();
        let calibrated =
            OneXsResources::for_camera(StitchCamera::CalibratedMei, &lenses, FRAME).unwrap();
        let native = resources();

        assert_eq!(calibrated.static_coordinates(), native.static_coordinates());
        assert_eq!(calibrated.gates(), native.gates());
        assert_eq!(calibrated.coordinates(), native.coordinates());
        assert_ne!(calibrated.alpha().bytes(), native.alpha().bytes());
        assert!(
            calibrated
                .alpha()
                .nodes()
                .iter()
                .all(|weight| weight.is_finite() && (0.0..=1.0).contains(weight))
        );

        let reframe = crate::Reframe::new(
            &lenses,
            FRAME,
            Camera::default(),
            Held::default(),
            1.0,
            false,
            Sampling::default(),
        );
        for (row, column) in [(1, 0), (MAP_HEIGHT / 2, 17), (MAP_HEIGHT - 2, 59)] {
            let index = row * MAP_WIDTH + column;
            assert_eq!(
                calibrated.alpha().nodes()[index].to_bits(),
                reframe.blend(sphere_node(row, column)).weights[0].to_bits(),
                "calibrated alpha at ({row},{column})",
            );
        }
    }

    #[test]
    fn native_camera_selection_preserves_exact_one_x2_resources() {
        let lenses = one_xs_lenses();
        let selected = OneXsResources::for_camera(
            StitchCamera::OneX2,
            &lenses,
            Size {
                width: 2880,
                height: 2880,
            },
        )
        .unwrap();
        let native = OneXsResources::new(&lenses).unwrap();

        assert_eq!(selected, native);
        assert_eq!(selected.alpha().bytes(), native.alpha().bytes());
    }

    #[test]
    fn resources_are_capture_static_across_user_view_and_horizon_state() {
        let lenses = one_xs_lenses();
        let before = OneXsResources::new(&lenses).unwrap();

        // These distinct draw blocks demonstrate the state deliberately absent
        // from `OneXsResources::new`: camera view and held horizon cannot enter
        // the resource hash because calibration is its only input.
        let frame = Size {
            width: 2880,
            height: 2880,
        };
        let _first_view = crate::Reframe::new(
            &lenses,
            frame,
            Camera::default(),
            Held::default(),
            1.0,
            false,
            Sampling::default(),
        );
        let _other_view = crate::Reframe::new(
            &lenses,
            frame,
            Camera {
                yaw: 1.1,
                pitch: -0.4,
                fov: 2.0,
            },
            Held {
                body_from_world: kjerag_meta::Quat {
                    w: 0.923_879_5,
                    v: [0.0, 0.382_683_43, 0.0],
                },
                rolling: None,
            },
            1.0,
            false,
            Sampling::default(),
        );

        let after = OneXsResources::new(&lenses).unwrap();
        assert_eq!(before, after);
        assert_eq!(before.alpha().bytes(), after.alpha().bytes());
    }
}
