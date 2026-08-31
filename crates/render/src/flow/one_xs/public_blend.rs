//! Selected CPU periodic-boundary blend for one public ONE X2 flow field.
//!
//! Studio calls `SeamlessBlenderImpl::blendFlow` after the warm estimator
//! returns.  For the selected 1080-row, 60-column field and width `2.0`, the
//! routine blends rows 51 through 55 with their partners 1022 through 1026,
//! then stores the same result into both rows of each pair.  The arithmetic
//! order below follows the pinned arm64 body: the partner term is rounded
//! first and the row term is fused with [`f32::mul_add`].
//!
//! The selected cold/warm production lineage uses this stage. Bit identity is
//! established for the authenticated finite target fields, not for NaN
//! payload or flush-to-zero behavior under another floating-point control
//! mode.

#![allow(
    dead_code,
    reason = "the module retains sealed oracle entry points beside its production path"
)]

use super::dense::PublicDenseField;
use super::pis::PisDirection;

const ANGULAR_ORIGIN: f32 = f32::from_bits(0xc348_0000); // -200.0
const ANGULAR_END: f32 = f32::from_bits(0x4348_0000); // 200.0
const PERIOD: f32 = 360.0;
const BOUNDARY: f32 = -180.0;
const SELECTED_WIDTH: f32 = f32::from_bits(0x4000_0000); // 2.0

#[derive(Clone, Copy, Debug, PartialEq)]
struct BoundaryPair {
    row: usize,
    partner: usize,
    weight: f32,
}

/// Apply Studio's selected in-place CPU boundary conditioner.
///
/// The field is consumed so later stages cannot accidentally retain an
/// unconditioned alias.  Both vector components use the same row pairing and
/// weight, but are evaluated independently.
pub(super) fn blend_periodic_boundary<D: PisDirection>(
    mut field: PublicDenseField<D>,
) -> PublicDenseField<D> {
    let pairs = selected_pairs(field.rows());
    let cols = field.cols();
    let (dcol, drow) = field.components_mut();

    for pair in pairs {
        let row_start = pair.row * cols;
        let partner_start = pair.partner * cols;
        for col in 0..cols {
            blend_component(dcol, row_start + col, partner_start + col, pair.weight);
            blend_component(drow, row_start + col, partner_start + col, pair.weight);
        }
    }

    field
}

fn blend_component(values: &mut [f32], row: usize, partner: usize, weight: f32) {
    let inverse = 1.0 - weight;
    let partner_term = values[partner] * inverse;
    let result = values[row].mul_add(weight, partner_term);
    values[row] = result;
    values[partner] = result;
}

fn selected_pairs(rows: usize) -> Vec<BoundaryPair> {
    assert!(rows > 1, "ONE X2 periodic blend needs at least two rows");

    let step = (ANGULAR_END - ANGULAR_ORIGIN) / (rows - 1) as f32;
    let half_width = SELECTED_WIDTH * 0.5;
    let band_start = BOUNDARY - half_width;
    let mut band_end = BOUNDARY + half_width;

    let mut start = ((band_start - ANGULAR_ORIGIN) / step) as i32;
    if start < 0 {
        start = ((band_start + PERIOD - ANGULAR_ORIGIN) / step) as i32;
        band_end += PERIOD;
    }
    start = start.max(0);
    let stop = (((band_end - ANGULAR_ORIGIN) / step) as i32).min(rows as i32);

    let mut pairs = Vec::with_capacity((stop - start).max(0) as usize);
    for row in start..stop {
        let angle = (row as f32).mul_add(step, ANGULAR_ORIGIN);
        let shifted = angle + PERIOD;
        let wrapped = if shifted > ANGULAR_END {
            shifted + -PERIOD
        } else {
            shifted
        };
        let partner = ((wrapped - ANGULAR_ORIGIN) / step) as i32;
        if !(0..rows as i32).contains(&partner) {
            continue;
        }

        // This is the selected path through arm64 FMINNM followed by FMAXNM.
        // Keep the instruction order instead of rewriting it as `clamp`.
        #[allow(
            clippy::manual_clamp,
            reason = "the pinned native order is FMINNM followed by FMAXNM"
        )]
        let weight = ((angle - band_start).abs() / SELECTED_WIDTH)
            .min(1.0)
            .max(0.0);
        pairs.push(BoundaryPair {
            row: row as usize,
            partner: partner as usize,
            weight,
        });
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::one_xs::pis::{AtoB, BtoA};
    use crate::flow::one_xs::{COLS, ROWS};

    #[test]
    fn selected_pairing_and_weights_have_the_read_bits() {
        let step = (ANGULAR_END - ANGULAR_ORIGIN) / (ROWS - 1) as f32;
        assert_eq!(step.to_bits(), 0x3ebd_ce2d);

        let pairs = selected_pairs(ROWS);
        assert_eq!(
            pairs.iter().map(|pair| pair.row).collect::<Vec<_>>(),
            [51, 52, 53, 54, 55]
        );
        assert_eq!(
            pairs.iter().map(|pair| pair.partner).collect::<Vec<_>>(),
            [1022, 1023, 1024, 1025, 1026]
        );
        assert_eq!(
            pairs
                .iter()
                .map(|pair| pair.weight.to_bits())
                .collect::<Vec<_>>(),
            [
                0x3d3f_b800,
                0x3e0d_e200,
                0x3ea5_d800,
                0x3f02_5f80,
                0x3f31_d300
            ]
        );
        assert_eq!(
            pairs
                .iter()
                .map(|pair| (1.0 - pair.weight).to_bits())
                .collect::<Vec<_>>(),
            [
                0x3f74_0480,
                0x3f5c_8780,
                0x3f2d_1400,
                0x3efb_4100,
                0x3e9c_5a00
            ]
        );
    }

    #[test]
    fn only_selected_pairs_change_and_each_pair_becomes_equal() {
        let dcol = (0..ROWS * COLS)
            .map(|index| index as f32 * 0.25 - 8000.0)
            .collect::<Vec<_>>();
        let drow = (0..ROWS * COLS)
            .map(|index| 4000.0 - index as f32 * 0.125)
            .collect::<Vec<_>>();
        let original_col = dcol.clone();
        let original_row = drow.clone();
        let result =
            blend_periodic_boundary(PublicDenseField::<AtoB>::from_test_components(dcol, drow));

        let pairs = selected_pairs(ROWS);
        let mut selected = vec![false; ROWS];
        for pair in &pairs {
            selected[pair.row] = true;
            selected[pair.partner] = true;
            for col in 0..COLS {
                assert_eq!(
                    result.dcol()[pair.row * COLS + col].to_bits(),
                    result.dcol()[pair.partner * COLS + col].to_bits()
                );
                assert_eq!(
                    result.drow()[pair.row * COLS + col].to_bits(),
                    result.drow()[pair.partner * COLS + col].to_bits()
                );
            }
        }
        for (row, &is_selected) in selected.iter().enumerate() {
            if is_selected {
                continue;
            }
            let range = row * COLS..(row + 1) * COLS;
            assert_eq!(
                result.dcol()[range.clone()]
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>(),
                original_col[range.clone()]
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                result.drow()[range.clone()]
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>(),
                original_row[range]
                    .iter()
                    .map(|value| value.to_bits())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    #[ignore = "requires KJERAG_ONE_XS_WARM_PAYLOAD with the authenticated run-06 payload"]
    fn accepted_run06_pre_maps_reproduce_both_post_maps_bit_exact() {
        use sha2::{Digest as _, Sha256};

        const PAYLOAD_SHA256: &str =
            "a5311370d7946d45094cc85c8ff6631bf2b1195e8c127e3b1f909310168ce8c1";
        const MAP_BYTES: usize = ROWS * COLS * 2 * size_of::<f32>();
        const PRE_C30: usize = 453_889;
        const PRE_C90: usize = 972_335;
        const POST_C30: usize = 4_998_998;
        const POST_C90: usize = 5_517_445;

        let path = std::env::var_os("KJERAG_ONE_XS_WARM_PAYLOAD")
            .expect("set KJERAG_ONE_XS_WARM_PAYLOAD to authenticated run-06 warm-pair-payload.bin");
        let payload = std::fs::read(path).unwrap();
        assert_eq!(sha256_hex(&payload), PAYLOAD_SHA256);
        assert_eq!(&payload[..8], b"KJWP602\x04");

        assert_replay::<AtoB>(
            &payload[PRE_C30..PRE_C30 + MAP_BYTES],
            &payload[POST_C30..POST_C30 + MAP_BYTES],
        );
        assert_replay::<BtoA>(
            &payload[PRE_C90..PRE_C90 + MAP_BYTES],
            &payload[POST_C90..POST_C90 + MAP_BYTES],
        );

        fn assert_replay<D: PisDirection>(pre: &[u8], expected: &[u8]) {
            let mut dcol = Vec::with_capacity(ROWS * COLS);
            let mut drow = Vec::with_capacity(ROWS * COLS);
            for node in pre.chunks_exact(8) {
                dcol.push(f32::from_le_bytes(node[..4].try_into().unwrap()));
                drow.push(f32::from_le_bytes(node[4..].try_into().unwrap()));
            }
            let actual =
                blend_periodic_boundary(PublicDenseField::<D>::from_test_components(dcol, drow));
            for (index, node) in expected.chunks_exact(8).enumerate() {
                assert_eq!(
                    actual.dcol()[index].to_bits(),
                    u32::from_le_bytes(node[..4].try_into().unwrap()),
                    "dcol node {index}"
                );
                assert_eq!(
                    actual.drow()[index].to_bits(),
                    u32::from_le_bytes(node[4..].try_into().unwrap()),
                    "drow node {index}"
                );
            }
        }

        fn sha256_hex(bytes: &[u8]) -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }
    }
}
