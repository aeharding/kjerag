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
}
