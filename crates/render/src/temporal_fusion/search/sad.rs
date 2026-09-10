//! Exact 16x16 byte SAD for the selected CPU search.
//!
//! Bounded row slices expose the byte absolute-difference reduction to the
//! compiler without architecture-specific intrinsics or unsafe loads. The
//! complete sum fits u32 (at most 65,280); the caller keeps its existing i64
//! penalty arithmetic. No cost-based early termination changes search ties.

const BLOCK: usize = 16;

/// Readable form of the original scalar SAD loop.
#[cfg(test)]
pub(super) fn scalar(
    current: &[u8],
    current_stride: usize,
    current_start: usize,
    reference: &[u8],
    reference_stride: usize,
    reference_start: usize,
) -> u32 {
    let mut sad = 0_i64;
    for row_index in 0..BLOCK {
        let current_row = row(current, current_stride, current_start, row_index);
        let reference_row = row(reference, reference_stride, reference_start, row_index);
        for column in 0..BLOCK {
            sad += (i64::from(current_row[column]) - i64::from(reference_row[column])).abs();
        }
    }
    u32::try_from(sad).expect("a 16x16 byte SAD fits u32")
}

/// Safe portable expression using bounded row slices and byte absolute difference.
pub(super) fn portable(
    current: &[u8],
    current_stride: usize,
    current_start: usize,
    reference: &[u8],
    reference_stride: usize,
    reference_start: usize,
) -> u32 {
    let mut sad = 0_u32;
    for row_index in 0..BLOCK {
        let current_row = row(current, current_stride, current_start, row_index);
        let reference_row = row(reference, reference_stride, reference_start, row_index);
        sad += current_row
            .iter()
            .zip(reference_row)
            .map(|(&current, &reference)| u32::from(current.abs_diff(reference)))
            .sum::<u32>();
    }
    sad
}

fn row(pixels: &[u8], stride: usize, start: usize, row_index: usize) -> &[u8] {
    let at = row_index
        .checked_mul(stride)
        .and_then(|offset| start.checked_add(offset))
        .expect("16x16 SAD row offset overflows usize");
    let end = at
        .checked_add(BLOCK)
        .expect("16x16 SAD row end overflows usize");
    pixels
        .get(at..end)
        .expect("16x16 SAD row extends past its pixel plane")
}

#[cfg(test)]
mod tests;
