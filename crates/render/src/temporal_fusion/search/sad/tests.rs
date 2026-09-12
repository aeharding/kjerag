use super::*;

#[test]
fn equal_blocks_are_zero() {
    let pixels: Vec<u8> = (0..256).map(|value| value as u8).collect();
    compare(&pixels, 16, 0, &pixels, 16, 0, 0);
}

#[test]
fn zero_against_255_reaches_the_exact_maximum() {
    compare(&[0; 256], 16, 0, &[255; 256], 16, 0, 65_280);
}

#[test]
fn patterned_blocks_allow_independent_misalignment_and_strides() {
    let current = patterned(701, 17);
    let reference = patterned(937, 83);
    compare(&current, 37, 3, &reference, 53, 11, 22_192);
}

#[test]
fn final_legal_rows_and_columns_end_at_each_plane_boundary() {
    let current_stride = 41;
    let reference_stride = 59;
    let current_span = 15 * current_stride + BLOCK;
    let reference_span = 15 * reference_stride + BLOCK;
    let current = patterned(current_span + 23, 29);
    let reference = patterned(reference_span + 7, 131);
    compare(
        &current,
        current_stride,
        current.len() - current_span,
        &reference,
        reference_stride,
        reference.len() - reference_span,
        22_034,
    );
}

#[test]
fn every_input_alignment_matches_the_portable_oracle() {
    let current = patterned(16 * 47 + 31, 17);
    let reference = patterned(16 * 53 + 31, 83);
    for current_start in 0..16 {
        for reference_start in 0..16 {
            let expected = portable(&current, 47, current_start, &reference, 53, reference_start);
            assert_eq!(
                exact(&current, 47, current_start, &reference, 53, reference_start,),
                expected,
                "starts {current_start} and {reference_start}"
            );
        }
    }
}

#[test]
fn stride_zero_and_overlapping_rows_are_preserved() {
    let current = patterned(192, 7);
    let reference = patterned(192, 211);
    for (current_stride, reference_stride) in [(0, 0), (0, 1), (1, 0), (7, 9)] {
        let expected = portable(&current, current_stride, 3, &reference, reference_stride, 5);
        assert_eq!(
            exact(&current, current_stride, 3, &reference, reference_stride, 5,),
            expected
        );
    }
}

#[test]
fn invalid_or_overflowing_footprints_are_rejected_before_the_leaf() {
    let pixels = [0_u8; 256];
    assert_panics(|| exact(&pixels, usize::MAX, 0, &pixels, 16, 0));
    assert_panics(|| exact(&pixels, 16, usize::MAX, &pixels, 16, 0));
    assert_panics(|| exact(&pixels[..255], 16, 0, &pixels, 16, 0));
    assert_panics(|| exact(&pixels, 16, 0, &pixels[..255], 16, 0));
}

fn assert_panics(call: impl FnOnce() -> u32 + std::panic::UnwindSafe) {
    assert!(std::panic::catch_unwind(call).is_err());
}

fn patterned(length: usize, salt: usize) -> Vec<u8> {
    (0..length)
        .map(|index| ((index * 73 + index / 7 * 19 + salt) & 255) as u8)
        .collect()
}

fn compare(
    current: &[u8],
    current_stride: usize,
    current_start: usize,
    reference: &[u8],
    reference_stride: usize,
    reference_start: usize,
    expected: u32,
) {
    let scalar = scalar(
        current,
        current_stride,
        current_start,
        reference,
        reference_stride,
        reference_start,
    );
    assert_eq!(scalar, expected);
    assert_eq!(
        portable(
            current,
            current_stride,
            current_start,
            reference,
            reference_stride,
            reference_start,
        ),
        scalar,
    );
    assert_eq!(
        exact(
            current,
            current_stride,
            current_start,
            reference,
            reference_stride,
            reference_start,
        ),
        scalar,
    );
}
