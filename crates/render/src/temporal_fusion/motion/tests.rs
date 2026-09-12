use super::*;
use sha2::{Digest, Sha256};
use std::path::Path;

const SMALL: Geometry = Geometry {
    full: [32, 32],
    raw_grid: [1, 1],
    output_grid: [2, 2],
    block: [16, 16],
};

fn parameters<'a>(
    geometry: Geometry,
    luma: &'a [u8],
    confidence_y: &'a [f32; 256],
    confidence_uv: &'a [f32; 256],
) -> Parameters<'a> {
    Parameters {
        geometry,
        luma,
        confidence_y,
        confidence_uv,
        scale_base: 4,
        scale_extra: 700,
        temporal: 1.25,
        phase: 0.0,
    }
}

#[test]
fn clamps_expanded_motion_at_each_image_border() {
    let tables = [1.0; 256];
    let actual = pack_motion(
        &[[-20, 20, 0]],
        &parameters(SMALL, &[0; 4], &tables, &tables),
    )
    .unwrap();
    assert_eq!(actual[0][..2], [0, 16]);
    assert_eq!(actual[1][..2], [-16, 16]);
    assert_eq!(actual[2][..2], [0, 0]);
    assert_eq!(actual[3][..2], [-16, 0]);
}

#[test]
fn interpolates_then_expands_displacement_and_truncates_cost() {
    let geometry = Geometry {
        full: [64, 64],
        raw_grid: [2, 2],
        output_grid: [4, 4],
        block: [16, 16],
    };
    let raw = [[0, 0, 0], [2, 4, 3], [4, 8, 5], [6, 12, 7]];
    let tables = [1.0; 256];
    let mut parameters = parameters(geometry, &[0; 16], &tables, &tables);
    parameters.scale_base = 1;
    parameters.scale_extra = 4;
    parameters.temporal = 1.0;
    let actual = pack_motion(&raw, &parameters).unwrap();
    // At expanded coordinate (1,1), all four raw records have weight 1/4.
    assert_eq!(actual[5][..2], [6, 12]);
    assert_eq!(interpolated_channel(&raw, 2, 1, 1, 2), 3.75);
    // Cost truncates to 3, rather than rounding to the rejecting threshold 4.
    assert_eq!(actual[5][2..], [71, 71]);
    // The final odd output coordinate clamps the bilinear neighbor to the
    // last raw sample instead of reading beyond the source grid.
    assert_eq!(interpolated_channel(&raw, 2, 3, 3, 0), 6.0);
}

#[test]
fn confidence_threshold_and_y_uv_tables_are_independent() {
    let mut y = [0.0; 256];
    let mut uv = [0.0; 256];
    y[9] = 1.0;
    uv[9] = 5.0 / 3500.0;
    let actual = pack_motion(&[[0, 0, 5]], &parameters(SMALL, &[9; 4], &y, &uv)).unwrap();
    assert!(actual.iter().all(|lanes| lanes[2] > 0));
    // T_uv truncates to exactly the cost, and the native T <= C gate wins.
    assert!(actual.iter().all(|lanes| lanes[3] == 0));
    assert_eq!(confidence(5, 5).unwrap(), 0);
    assert_eq!(confidence(6, 5).unwrap(), 46);
}

#[test]
fn expands_signed_fractional_motion_before_truncation() {
    let geometry = Geometry {
        full: [64, 64],
        raw_grid: [2, 2],
        output_grid: [4, 4],
        block: [16, 16],
    };
    let tables = [1.0; 256];
    let raw = [[-1, 0, 0], [-1, 0, 0], [-1, 0, 0], [0, 0, 0]];
    let actual = pack_motion(&raw, &parameters(geometry, &[0; 16], &tables, &tables)).unwrap();
    assert_eq!(interpolated_channel(&raw, 2, 1, 1, 0), -0.75);
    assert_eq!(actual[5][0], -1);
}

#[test]
fn rejects_geometry_outside_the_verified_selected_path() {
    let tables = [1.0; 256];
    let mut geometry = SMALL;
    geometry.output_grid = [3, 2];
    let error = pack_motion(
        &[[0, 0, 0]],
        &parameters(geometry, &[0; 6], &tables, &tables),
    )
    .unwrap_err();
    assert!(error.contains("2x expansion"));
}

#[test]
fn confidence_keeps_native_wrapping_threshold_square() {
    assert_eq!(confidence(65_536, 1).unwrap(), -256);
}

#[test]
fn scale_multiplies_in_wrapping_i32_before_float_conversion() {
    let tables = [1.0; 256];
    let mut parameters = parameters(SMALL, &[0; 4], &tables, &tables);
    parameters.scale_base = i32::MAX;
    parameters.scale_extra = 2;
    parameters.temporal = 1.0;
    let actual = pack_motion(&[[0, 0, 0]], &parameters).unwrap();
    assert!(actual.iter().all(|lanes| lanes[2..] == [0, 0]));
}

#[test]
fn rejects_nonfinite_inputs_and_inexact_grid_coordinates() {
    let tables = [1.0; 256];
    let mut parameters = parameters(SMALL, &[0; 4], &tables, &tables);
    parameters.phase = f64::NAN;
    assert!(
        pack_motion(&[[0, 0, 0]], &parameters)
            .unwrap_err()
            .contains("finite")
    );
    parameters.phase = 0.0;
    parameters.geometry.output_grid[0] = (1 << 24) + 2;
    assert!(
        pack_motion(&[[0, 0, 0]], &parameters)
            .unwrap_err()
            .contains("exactly representable")
    );
}

fn read_hashed(directory: &Path, name: &str, expected: &str) -> Vec<u8> {
    let bytes = std::fs::read(directory.join(name)).unwrap();
    let digest = Sha256::digest(&bytes);
    let actual = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(actual, expected, "fixture hash changed for {name}");
    bytes
}

fn f32_table(bytes: &[u8]) -> [f32; 256] {
    assert_eq!(bytes.len(), 1024);
    std::array::from_fn(|index| {
        f32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap())
    })
}

fn raw_records(bytes: &[u8]) -> Vec<[i32; 3]> {
    assert_eq!(bytes.len(), 240 * 120 * 12);
    bytes
        .chunks_exact(12)
        .map(|record| {
            std::array::from_fn(|lane| {
                i32::from_le_bytes(record[lane * 4..lane * 4 + 4].try_into().unwrap())
            })
        })
        .collect()
}

fn packed_bytes(records: &[[i16; 4]]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|record| record.iter().flat_map(|lane| lane.to_le_bytes()))
        .collect()
}

#[test]
#[ignore = "requires private Studio motion capture named by KJERAG_MOTION_FIXTURE_DIR"]
fn captured_six_reference_motion_matrices_are_exact() {
    let fixture = native_fixture();
    for (ordinal, reference) in fixture.references.iter().enumerate() {
        let actual = packed_bytes(
            &pack_motion(&reference.raw, &fixture.parameters(reference.phase)).unwrap(),
        );
        assert!(
            actual == reference.expected,
            "reference {ordinal}: lengths {} and {}, first unequal byte {:?}",
            actual.len(),
            reference.expected.len(),
            actual
                .iter()
                .zip(&reference.expected)
                .position(|(a, b)| a != b)
        );
    }
}

pub(super) struct NativeFixture {
    geometry: Geometry,
    luma: Vec<u8>,
    y: [f32; 256],
    uv: [f32; 256],
    pub references: Vec<NativeReference>,
}

pub(super) struct NativeReference {
    pub raw: Vec<[i32; 3]>,
    pub expected: Vec<u8>,
    pub phase: f64,
}

impl NativeFixture {
    pub fn parameters(&self, phase: f64) -> Parameters<'_> {
        Parameters {
            geometry: self.geometry,
            luma: &self.luma,
            confidence_y: &self.y,
            confidence_uv: &self.uv,
            scale_base: 4,
            scale_extra: 700,
            temporal: 1.25,
            phase,
        }
    }
}

/// CPU and GPU tests consume the same hash-sealed captured bytes, rather
/// than using one implementation to manufacture the other's expectations.
pub(super) fn native_fixture() -> NativeFixture {
    let directory = std::env::var_os("KJERAG_MOTION_FIXTURE_DIR")
        .map(std::path::PathBuf::from)
        .expect("set KJERAG_MOTION_FIXTURE_DIR to temporal-motion-capture-01/run-01");
    read_hashed(
        &directory,
        "events.jsonl",
        "72d7233f89e3ff9749c33eaa405cc408787ae9299260fa15a9f1cf1fba4560c4",
    );
    let luma = read_hashed(
        &directory,
        "luma.bin",
        "43968e5468fd2efde00160dc6b7ec377d1ea91a32009720b00cb34d3abcfa9ab",
    );
    let y = f32_table(&read_hashed(
        &directory,
        "confidence-y.f32",
        "893a106828fbdb9521e1d868c985aab7ad2ae2f606edc55329265a5e7676006c",
    ));
    let uv = f32_table(&read_hashed(
        &directory,
        "confidence-uv.f32",
        "e4b7d0f9baaf61ee92c605361c3e89c72de0d113f5bcb4bcf4678695bf504fdf",
    ));
    let phases = [1.0, 0.49999999999999994, 0.0, 0.0, 0.49999999999999994, 1.0];
    let raw_hashes = [
        "b4506fb6ffc384ad66f1865de7ac3059ab795818e7d8e8d57781a9e84185fbca",
        "e5f04348ac7ac508319c9fe368dc2e8afa9060bbb19765042697efa42891f3f0",
        "54cd08de368b44a74c0d18d1ab5390ba897183934a4519f2e588ea58bc7d9ff9",
        "ef1096dc340d321fa63f1247a362237960b17d73c6e1fa52455619339b0bc270",
        "36a84a670c6c8d7ac6f7a17d469c5b76c64b8969cc6ad290c831a151f2e1910a",
        "89cc607cbad76771bb70e6c9f00c3f753df586d380a7a7bd2a940092dc4d922e",
    ];
    let packed_hashes = [
        "7424aa6f681a94a7febc7b7fab985a3cf73c2362652b814a7fc2024b6503ed09",
        "50325739e0d70c83e5820fa208c069b5c72ab114185344e07349603d5fff8a14",
        "c99b497ef4bba9ad2060418ac9e468e96d4803f03dd508eee8170bcbe2828610",
        "97a4bae01e3a84bdea77bbfd2a0a333facc625aa6c2e2698c674120b0a18a5fd",
        "ebd2b3155c7b39b823f27f133c7d2f792b5644289fc5d0db613ec759707eed9f",
        "67d026dfb656a3ec9e6ebcd67fffe43a63b428304dcc765f661b0b8512e2a164",
    ];
    let geometry = Geometry {
        full: [7680, 3840],
        raw_grid: [240, 120],
        output_grid: [480, 240],
        block: [16, 16],
    };

    let references = (0..6)
        .map(|ordinal| {
            let raw_name = format!("reference-{ordinal}-raw.bin");
            let expected_name = format!("reference-{ordinal}-packed.bin");
            let raw = raw_records(&read_hashed(&directory, &raw_name, raw_hashes[ordinal]));
            let expected = read_hashed(&directory, &expected_name, packed_hashes[ordinal]);
            NativeReference {
                raw,
                expected,
                phase: phases[ordinal],
            }
        })
        .collect();
    NativeFixture {
        geometry,
        luma,
        y,
        uv,
        references,
    }
}
