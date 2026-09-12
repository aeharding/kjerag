//! Readable CPU materializer for Studio's bilateral ordinary map patch.
//!
//! Lens A consumes the B-to-A field and lens B consumes the A-to-B field.
//! Both native wrappers use `1-g`; the
//! captured right gate is already `1-a`, so that wrapper's effective factor is
//! `a`. A rejected node is not filled: its captured pre-call local UV is
//! retained bit for bit.
//!
//! The scalar weight construction, TR-first multiply/FMA accumulation, and
//! flow FMA are READ from the target. The capture-owned frame transaction uses
//! this materializer; its output is authoritative only while bound to that
//! transaction's exact source/map identity. The tap chain and final scalar
//! guards have different unordered behavior; authenticated target inputs are
//! finite, but both native branches are represented here.

use std::error::Error;
use std::fmt;

use super::{COLS, ROWS};

pub const OUTPUT_ROWS: usize = 100;
pub const OUTPUT_COLS: usize = 200;
pub const OUTPUT_NODES: usize = OUTPUT_ROWS * OUTPUT_COLS;
pub const RETAINED_NODES: usize = ROWS * COLS;

/// Effective f32 cutoff of the READ binary64 `1e-8` coordinate guard.
pub const POSITIVE_EPSILON: f32 = f32::from_bits(0x322b_cc77);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapeError {
    pub map: &'static str,
    pub expected: usize,
    pub actual: usize,
}

impl fmt::Display for ShapeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "{} has {} nodes, expected {}",
            self.map, self.actual, self.expected
        )
    }
}

impl Error for ShapeError {}

macro_rules! map_type {
    ($name:ident, $value:ty, $nodes:expr, $label:literal) => {
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name(Vec<$value>);

        impl $name {
            pub fn new(values: Vec<$value>) -> Result<Self, ShapeError> {
                if values.len() != $nodes {
                    return Err(ShapeError {
                        map: $label,
                        expected: $nodes,
                        actual: values.len(),
                    });
                }
                Ok(Self(values))
            }

            pub fn values(&self) -> &[$value] {
                &self.0
            }
        }
    };
}

// Retained-grid source `[u, v]` lookup map.
map_type!(BaseMap, [f32; 2], RETAINED_NODES, "base map");
// Retained-grid ordinary field stored as `[dcol, drow]`.
map_type!(FlowMap, [f32; 2], RETAINED_NODES, "flow map");
map_type!(GateMap, f32, OUTPUT_NODES, "gate map");
// Captured line coordinate stored as `[col, row]`.
map_type!(CoordinateMap, [f32; 2], OUTPUT_NODES, "coordinate map");
// Captured pre-call local source `[u, v]` map.
map_type!(PreimageMap, [f32; 2], OUTPUT_NODES, "preimage map");
// Materialized post-call local source `[u, v]` map.
map_type!(LocalMap, [f32; 2], OUTPUT_NODES, "local map");

/// Inputs for one direction's captured same-call map transaction.
#[derive(Clone, Copy, Debug)]
pub struct SideInputs<'a> {
    pub preimage: &'a PreimageMap,
    pub base: &'a BaseMap,
    pub flow: &'a FlowMap,
    pub gate: &'a GateMap,
    pub coordinate: &'a CoordinateMap,
}

/// Direction-named inputs. B-to-A is owned by lens A; A-to-B is owned by B.
#[derive(Clone, Copy, Debug)]
pub struct BilateralInputs<'a> {
    pub b_to_a: SideInputs<'a>,
    pub a_to_b: SideInputs<'a>,
}

/// Complete diagnostic result in local and READ packed coordinate systems.
#[derive(Clone, Debug, PartialEq)]
pub struct BilateralMaps {
    pub lens_a: LocalMap,
    pub lens_b: LocalMap,
    pub lens_a_census: Census,
    pub lens_b_census: Census,
    pub packed: Vec<[f32; 4]>,
}

/// What the native call does to one destination node.
///
/// Both keep variants mean "return without writing". They remain distinct so
/// the corpus audit can report which READ guard fired. Neither carries a
/// fabricated fallback UV because V3 did not capture the pre-call ROI bytes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    KeepGate,
    KeepLookup,
    Store([f32; 2]),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub keep_gate: usize,
    pub keep_lookup: usize,
    pub store: usize,
}

impl Census {
    pub fn from_actions(actions: &[Action]) -> Self {
        let mut census = Self::default();
        for action in actions {
            match action {
                Action::KeepGate => census.keep_gate += 1,
                Action::KeepLookup => census.keep_lookup += 1,
                Action::Store(_) => census.store += 1,
            }
        }
        census
    }
}

/// Classify every captured ordinary-right node under the READ apply law.
///
/// This compatibility view has no preimage, so keep actions carry no value.
/// Its `gate` is the captured right `1-a` resource; the shared native body's
/// `1-g` operation therefore produces the effective lens-B factor `a`.
pub fn classify(
    base: &BaseMap,
    flow: &FlowMap,
    gate: &GateMap,
    coordinate: &CoordinateMap,
) -> Vec<Action> {
    gate.values()
        .iter()
        .zip(coordinate.values())
        .map(|(&gate, &coordinate)| classify_node(base, flow, gate, coordinate))
        .collect()
}

/// Materialize both local maps and their packed float4 representation.
///
/// Only [`Action::Store`] nodes overwrite their corresponding preimage.
pub fn materialize(inputs: BilateralInputs<'_>) -> BilateralMaps {
    let (lens_a, lens_a_census) = materialize_side(inputs.b_to_a);
    let (lens_b, lens_b_census) = materialize_side(inputs.a_to_b);
    let packed = lens_a
        .values()
        .iter()
        .zip(lens_b.values())
        .map(|(&left, &right)| {
            let left_u = left[0] * 0.5_f32;
            let right_plus_one = right[0] + 1.0_f32;
            let right_u = right_plus_one * 0.5_f32;
            [left_u, left[1], right_u, right[1]]
        })
        .collect();
    BilateralMaps {
        lens_a,
        lens_b,
        lens_a_census,
        lens_b_census,
        packed,
    }
}

fn materialize_side(inputs: SideInputs<'_>) -> (LocalMap, Census) {
    let actions = inputs
        .gate
        .values()
        .iter()
        .zip(inputs.coordinate.values())
        .map(|(&gate, &coordinate)| classify_node(inputs.base, inputs.flow, gate, coordinate))
        .collect::<Vec<_>>();
    let census = Census::from_actions(&actions);
    let mut values = inputs.preimage.values().to_vec();
    for (value, action) in values.iter_mut().zip(actions) {
        if let Action::Store(uv) = action {
            *value = uv;
        }
    }
    (
        LocalMap::new(values).expect("preimage shape establishes local map shape"),
        census,
    )
}

fn classify_node(base: &BaseMap, flow: &FlowMap, gate: f32, c: [f32; 2]) -> Action {
    // The native ordered comparisons keep finite values outside `(0, 1)` but
    // let an unordered gate continue.
    if gate <= 0.0 || gate >= 1.0 {
        return Action::KeepGate;
    }

    let sampled_flow = bilinear(flow.values(), c);
    let weight = 1.0_f32 - gate;
    let q = displace(c, sampled_flow, weight);
    let taps = base_taps(q);

    // The READ guard tests only component u of each base tap. Component v is
    // deliberately not tested here. Native FCMP/FCCMP rejects `<=`, so an
    // unordered tap passes; the authenticated target domain is finite.
    if taps
        .indices
        .iter()
        .any(|&index| base.values()[index][0] <= POSITIVE_EPSILON)
    {
        return Action::KeepLookup;
    }

    let uv = interpolate(base.values(), taps);
    if !(uv[0] > POSITIVE_EPSILON && uv[1] > POSITIVE_EPSILON) {
        return Action::KeepLookup;
    }
    Action::Store(uv)
}

fn displace(c: [f32; 2], sampled_flow: [f32; 2], weight: f32) -> [f32; 2] {
    // Captured coordinates and public flow are [col, row] and [dcol, drow].
    [
        sampled_flow[0].mul_add(weight, c[0]),
        sampled_flow[1].mul_add(weight, c[1]),
    ]
}

#[derive(Clone, Copy)]
struct Taps {
    indices: [usize; 4],
    weights: [f32; 4],
}

fn flow_taps(c: [f32; 2]) -> Taps {
    let col = c[0].min(COLS as f32 - 1.0).max(0.0);
    let row = c[1].min(ROWS as f32 - 1.0).max(0.0);
    make_taps(row, col)
}

fn base_taps(c: [f32; 2]) -> Taps {
    // The native body clamps first to the flow extent, then to the base extent.
    let col = c[0]
        .max(0.0)
        .min(COLS as f32 - 1.0)
        .min(COLS as f32 - 1.0)
        .max(0.0);
    let row = c[1]
        .max(0.0)
        .min(ROWS as f32 - 1.0)
        .min(ROWS as f32 - 1.0)
        .max(0.0);
    make_taps(row, col)
}

fn make_taps(row: f32, col: f32) -> Taps {
    let col0 = col.floor() as usize;
    let row0 = row.floor() as usize;
    let col1 = (col0 + 1).min(COLS - 1);
    let row1 = (row0 + 1).min(ROWS - 1);
    let row_inverse = (1.0_f32 - row) + row0 as f32;
    let col_inverse = (1.0_f32 - col) + col0 as f32;
    let tl = row_inverse * col_inverse;
    let tr = row_inverse - tl;
    let bl = col_inverse - tl;
    let tmp = (1.0_f32 - col_inverse) - row_inverse;
    let br = tmp + tl;
    Taps {
        indices: [
            row0 * COLS + col0,
            row0 * COLS + col1,
            row1 * COLS + col0,
            row1 * COLS + col1,
        ],
        weights: [tl, tr, bl, br],
    }
}

fn bilinear(values: &[[f32; 2]], c: [f32; 2]) -> [f32; 2] {
    interpolate(values, flow_taps(c))
}

fn interpolate(values: &[[f32; 2]], taps: Taps) -> [f32; 2] {
    let [i00, i01, i10, i11] = taps.indices;
    let [tl, tr, bl, br] = taps.weights;
    std::array::from_fn(|component| {
        let sum = values[i01][component] * tr;
        let sum = values[i00][component].mul_add(tl, sum);
        let sum = values[i10][component].mul_add(bl, sum);
        values[i11][component].mul_add(br, sum)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_float(path: &std::path::Path, nodes: usize) -> Vec<f32> {
        let bytes = std::fs::read(path).unwrap_or_else(|error| {
            panic!("could not read corpus leaf {}: {error}", path.display())
        });
        assert_eq!(bytes.len(), nodes * size_of::<f32>());
        bytes
            .chunks_exact(size_of::<f32>())
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect()
    }

    fn read_float2(path: &std::path::Path, nodes: usize) -> Vec<[f32; 2]> {
        read_float(path, nodes * 2)
            .chunks_exact(2)
            .map(|values| values.try_into().unwrap())
            .collect()
    }

    fn base(fill: [f32; 2]) -> BaseMap {
        BaseMap::new(vec![fill; RETAINED_NODES]).unwrap()
    }

    fn flow(fill: [f32; 2]) -> FlowMap {
        FlowMap::new(vec![fill; RETAINED_NODES]).unwrap()
    }

    fn gates(fill: f32) -> GateMap {
        GateMap::new(vec![fill; OUTPUT_NODES]).unwrap()
    }

    fn coordinates(fill: [f32; 2]) -> CoordinateMap {
        CoordinateMap::new(vec![fill; OUTPUT_NODES]).unwrap()
    }

    fn preimage(fill: [f32; 2]) -> PreimageMap {
        PreimageMap::new(vec![fill; OUTPUT_NODES]).unwrap()
    }

    #[test]
    fn every_map_shape_is_checked() {
        assert_eq!(BaseMap::new(vec![]).unwrap_err().expected, RETAINED_NODES);
        assert_eq!(FlowMap::new(vec![]).unwrap_err().expected, RETAINED_NODES);
        assert_eq!(GateMap::new(vec![]).unwrap_err().expected, OUTPUT_NODES);
        assert_eq!(
            CoordinateMap::new(vec![]).unwrap_err().expected,
            OUTPUT_NODES
        );
        assert_eq!(PreimageMap::new(vec![]).unwrap_err().expected, OUTPUT_NODES);
        assert_eq!(LocalMap::new(vec![]).unwrap_err().expected, OUTPUT_NODES);
    }

    #[test]
    fn ordered_gate_rejections_are_actions_without_fallback_values() {
        let base = base([0.25, 0.75]);
        let flow = flow([0.0, 0.0]);
        for gate in [0.0, 1.0, -0.1, 1.1] {
            assert_eq!(
                classify_node(&base, &flow, gate, [3.0, 4.0]),
                Action::KeepGate
            );
        }
        assert_eq!(
            classify_node(&base, &flow, f32::NAN, [3.0, 4.0]),
            Action::Store([0.25, 0.75])
        );
    }

    #[test]
    fn coordinate_is_col_row_and_flow_is_dcol_drow() {
        let mut values = vec![[0.25, 0.75]; RETAINED_NODES];
        values[7 * COLS + 5] = [0.4, 0.6];
        let base = BaseMap::new(values).unwrap();
        let action = classify_node(&base, &flow([2.0, 4.0]), 0.5, [4.0, 5.0]);
        assert_eq!(action, Action::Store([0.4, 0.6]));
    }

    #[test]
    fn lookup_taps_test_u_but_not_v() {
        let mut values = vec![[0.5, 0.5]; RETAINED_NODES];
        values[0][1] = -0.5;
        let base = BaseMap::new(values).unwrap();
        assert!(matches!(
            classify_node(&base, &flow([0.0, 0.0]), 0.5, [0.9, 0.9]),
            Action::Store(_)
        ));

        let mut values = vec![[0.5, 0.5]; RETAINED_NODES];
        values[0][0] = POSITIVE_EPSILON.next_down();
        let base = BaseMap::new(values).unwrap();
        assert_eq!(
            classify_node(&base, &flow([0.0, 0.0]), 0.5, [0.25, 0.25]),
            Action::KeepLookup
        );
    }

    #[test]
    fn tap_and_final_guards_are_strictly_above_epsilon() {
        let at_epsilon = base([POSITIVE_EPSILON, 0.5]);
        assert_eq!(
            classify_node(&at_epsilon, &flow([0.0, 0.0]), 0.5, [2.0, 2.0]),
            Action::KeepLookup
        );
        let above_epsilon = base([POSITIVE_EPSILON.next_up(), 0.5]);
        assert!(matches!(
            classify_node(&above_epsilon, &flow([0.0, 0.0]), 0.5, [2.0, 2.0]),
            Action::Store(_)
        ));
    }

    #[test]
    fn both_sampling_coordinates_clamp_to_the_retained_grid() {
        let mut values = vec![[0.25, 0.75]; RETAINED_NODES];
        values[(ROWS - 1) * COLS + (COLS - 1)] = [0.8, 0.9];
        let base = BaseMap::new(values).unwrap();
        assert_eq!(
            classify_node(&base, &flow([500.0, 500.0]), 0.5, [5_000.0, 5_000.0]),
            Action::Store([0.8, 0.9])
        );
    }

    #[test]
    fn materializer_preserves_keep_bits_including_signed_zero_and_sentinel() {
        let sentinel = f32::from_bits(0xffc0_1234);
        let mut pre = vec![[0.25, 0.75]; OUTPUT_NODES];
        pre[0] = [-0.0, sentinel];
        pre[1] = [sentinel, -0.0];
        let pre = PreimageMap::new(pre).unwrap();
        let base = base([0.5, 0.5]);
        let flow = flow([0.0, 0.0]);
        let gate = gates(0.0);
        let coordinate = coordinates([0.0, 0.0]);
        let side = SideInputs {
            preimage: &pre,
            base: &base,
            flow: &flow,
            gate: &gate,
            coordinate: &coordinate,
        };
        let maps = materialize(BilateralInputs {
            b_to_a: side,
            a_to_b: side,
        });
        for local in [&maps.lens_a, &maps.lens_b] {
            assert_eq!(
                local.values()[0].map(f32::to_bits),
                [0x8000_0000, 0xffc0_1234]
            );
            assert_eq!(
                local.values()[1].map(f32::to_bits),
                [0xffc0_1234, 0x8000_0000]
            );
        }
    }

    #[test]
    fn directions_own_their_lenses_gates_and_separate_coordinates() {
        let mut left_base = vec![[0.2, 0.2]; RETAINED_NODES];
        left_base[4 * COLS + 5] = [0.31, 0.41];
        let left_base = BaseMap::new(left_base).unwrap();
        let mut right_base = vec![[0.7, 0.7]; RETAINED_NODES];
        right_base[8 * COLS + 11] = [0.61, 0.71];
        let right_base = BaseMap::new(right_base).unwrap();
        let flow = flow([4.0, 0.0]);
        let mut gate_values = vec![0.0; OUTPUT_NODES];
        gate_values[0] = 0.25;
        let gate = GateMap::new(gate_values).unwrap();
        let mut left_coordinates = vec![[0.0, 0.0]; OUTPUT_NODES];
        left_coordinates[0] = [2.0, 4.0];
        let left_coordinates = CoordinateMap::new(left_coordinates).unwrap();
        let mut right_coordinates = vec![[0.0, 0.0]; OUTPUT_NODES];
        right_coordinates[0] = [8.0, 8.0];
        let right_coordinates = CoordinateMap::new(right_coordinates).unwrap();
        let pre = preimage([0.9, 0.9]);

        let maps = materialize(BilateralInputs {
            b_to_a: SideInputs {
                preimage: &pre,
                base: &left_base,
                flow: &flow,
                gate: &gate,
                coordinate: &left_coordinates,
            },
            a_to_b: SideInputs {
                preimage: &pre,
                base: &right_base,
                flow: &flow,
                gate: &gate,
                coordinate: &right_coordinates,
            },
        });
        assert_eq!(maps.lens_a.values()[0], [0.31, 0.41]);
        assert_eq!(maps.lens_b.values()[0], [0.61, 0.71]);
    }

    #[test]
    fn tap_nan_continues_but_final_nan_rejects() {
        let valid_flow = flow([0.0, 0.0]);
        let mut tap_nan = vec![[0.5, 0.5]; RETAINED_NODES];
        tap_nan[0][0] = f32::NAN;
        let tap_nan = BaseMap::new(tap_nan).unwrap();
        let taps = base_taps([0.25, 0.25]);
        assert!(
            !taps
                .indices
                .iter()
                .any(|&index| tap_nan.values()[index][0] <= POSITIVE_EPSILON)
        );
        assert_eq!(
            classify_node(&tap_nan, &valid_flow, 0.5, [0.25, 0.25]),
            Action::KeepLookup
        );
        assert_eq!(
            classify_node(&base([0.5, f32::NAN]), &valid_flow, 0.5, [2.0, 2.0]),
            Action::KeepLookup
        );
    }

    #[test]
    fn read_weight_and_fma_order_is_bit_exact() {
        let row = f32::from_bits(0x3f5a_a5df);
        let col = f32::from_bits(0x3eea_9463);
        let taps = make_taps(row, col);
        assert_eq!(
            taps.weights.map(f32::to_bits),
            [0x3da1_e8e5, 0x3d88_e823, 0x3eec_f163, 0x3ec8_5a5b]
        );

        let mut values = vec![[0.5, 0.5]; RETAINED_NODES];
        for (index, bits) in
            taps.indices
                .into_iter()
                .zip([0x40f3_deb0_u32, 0x402a_2ce2, 0xbf9e_cb7e, 0xc086_5b06])
        {
            values[index][0] = f32::from_bits(bits);
        }
        assert_eq!(interpolate(&values, taps)[0].to_bits(), 0xbfb7_eabc);

        let q = displace(
            [f32::from_bits(0x3f4c_6453), 0.0],
            [f32::from_bits(0xc026_fedf), 0.0],
            f32::from_bits(0x3f32_7885),
        );
        assert_eq!(q[0].to_bits(), 0xbf82_a581);
    }

    #[test]
    fn exact_censuses_cover_every_node() {
        let mut base_values = vec![[0.5, 0.5]; RETAINED_NODES];
        base_values[10 * COLS + 10][0] = POSITIVE_EPSILON.next_down();
        let base = BaseMap::new(base_values).unwrap();
        let flow = flow([0.0, 0.0]);
        let mut gate_values = vec![0.0; OUTPUT_NODES];
        gate_values[0] = 0.5;
        gate_values[1] = 0.5;
        let gate = GateMap::new(gate_values).unwrap();
        let mut coordinate_values = vec![[0.0, 0.0]; OUTPUT_NODES];
        coordinate_values[1] = [10.0, 10.0];
        let coordinate = CoordinateMap::new(coordinate_values).unwrap();
        let pre = preimage([0.9, 0.9]);
        let side = SideInputs {
            preimage: &pre,
            base: &base,
            flow: &flow,
            gate: &gate,
            coordinate: &coordinate,
        };
        let maps = materialize(BilateralInputs {
            b_to_a: side,
            a_to_b: side,
        });
        let expected = Census {
            keep_gate: OUTPUT_NODES - 2,
            keep_lookup: 1,
            store: 1,
        };
        assert_eq!(maps.lens_a_census, expected);
        assert_eq!(maps.lens_b_census, expected);
    }

    #[test]
    fn packing_rounds_right_add_before_half() {
        let right_u = f32::from_bits(0xbf7f_ffff);
        let left_pre = preimage([0.75, -0.0]);
        let right_pre = preimage([right_u, 0.25]);
        let base = base([0.5, 0.5]);
        let flow = flow([0.0, 0.0]);
        let gate = gates(0.0);
        let coordinate = coordinates([0.0, 0.0]);
        let maps = materialize(BilateralInputs {
            b_to_a: SideInputs {
                preimage: &left_pre,
                base: &base,
                flow: &flow,
                gate: &gate,
                coordinate: &coordinate,
            },
            a_to_b: SideInputs {
                preimage: &right_pre,
                base: &base,
                flow: &flow,
                gate: &gate,
                coordinate: &coordinate,
            },
        });
        let right_plus_one = right_u + 1.0_f32;
        assert_eq!(
            maps.packed[0].map(f32::to_bits),
            [
                (0.75_f32 * 0.5).to_bits(),
                0x8000_0000,
                (right_plus_one * 0.5_f32).to_bits(),
                0.25_f32.to_bits(),
            ]
        );
    }

    #[test]
    #[ignore = "requires KJERAG_TARGET_V6_LINE_MAP_DIR with the detached target corpus"]
    fn target_v6_bilateral_replay_is_bit_exact() {
        use sha2::{Digest as _, Sha256};

        let root = std::path::PathBuf::from(
            std::env::var_os("KJERAG_TARGET_V6_LINE_MAP_DIR")
                .expect("set KJERAG_TARGET_V6_LINE_MAP_DIR to the detached target corpus"),
        );
        let load_side = |base_name: &str,
                         flow_name: &str,
                         gate_name: &str,
                         coordinate_name: &str,
                         preimage_name: &str|
         -> (BaseMap, FlowMap, GateMap, CoordinateMap, PreimageMap) {
            (
                BaseMap::new(read_float2(&root.join(base_name), RETAINED_NODES)).unwrap(),
                FlowMap::new(read_float2(&root.join(flow_name), RETAINED_NODES)).unwrap(),
                GateMap::new(read_float(&root.join(gate_name), OUTPUT_NODES)).unwrap(),
                CoordinateMap::new(read_float2(&root.join(coordinate_name), OUTPUT_NODES)).unwrap(),
                PreimageMap::new(read_float2(&root.join(preimage_name), OUTPUT_NODES)).unwrap(),
            )
        };
        let (left_base, left_flow, left_gate, left_coordinate, left_preimage) = load_side(
            "line-map-base-left-8d0.bin",
            "line-map-ordinary-left-c90.bin",
            "line-map-left-gate-1ab0.bin",
            "line-map-left-810.bin",
            "line-map-ordinary-left-output-pre.bin",
        );
        let (right_base, right_flow, right_gate, right_coordinate, right_preimage) = load_side(
            "line-map-base-right-930.bin",
            "line-map-ordinary-right-c30.bin",
            "line-map-right-gate-1b70.bin",
            "line-map-right-810.bin",
            "line-map-ordinary-right-output-pre.bin",
        );
        let maps = materialize(BilateralInputs {
            b_to_a: SideInputs {
                preimage: &left_preimage,
                base: &left_base,
                flow: &left_flow,
                gate: &left_gate,
                coordinate: &left_coordinate,
            },
            a_to_b: SideInputs {
                preimage: &right_preimage,
                base: &right_base,
                flow: &right_flow,
                gate: &right_gate,
                coordinate: &right_coordinate,
            },
        });
        assert_eq!(
            maps.lens_a_census,
            Census {
                keep_gate: 14_940,
                keep_lookup: 56,
                store: 5_004,
            }
        );
        assert_eq!(
            maps.lens_b_census,
            Census {
                keep_gate: 14_940,
                keep_lookup: 155,
                store: 4_905,
            }
        );

        let expected_left = read_float2(
            &root.join("line-map-ordinary-left-output-post.bin"),
            OUTPUT_NODES,
        );
        let expected_right = read_float2(
            &root.join("line-map-ordinary-right-output-post.bin"),
            OUTPUT_NODES,
        );
        assert!(
            maps.lens_a
                .values()
                .iter()
                .zip(expected_left)
                .all(|(&actual, expected)| actual.map(f32::to_bits) == expected.map(f32::to_bits))
        );
        assert!(
            maps.lens_b
                .values()
                .iter()
                .zip(expected_right)
                .all(|(&actual, expected)| actual.map(f32::to_bits) == expected.map(f32::to_bits))
        );

        let packed_bytes = maps
            .packed
            .iter()
            .flat_map(|node| node.iter().flat_map(|value| value.to_le_bytes()))
            .collect::<Vec<_>>();
        let packed_sha256 = Sha256::digest(&packed_bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            packed_sha256,
            "ec0fa73abd3fd169d2ec80d26bb8068f9693acc753ad881269717af0e7e97c6a"
        );
    }
}
