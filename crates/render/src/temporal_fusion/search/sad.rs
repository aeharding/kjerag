//! Exact 16x16 byte SAD for the selected CPU search.
//!
//! The selected wrapper validates both complete footprints once, then uses a
//! small SSE2 leaf on x86-64. Readable scalar and portable implementations stay
//! as independent test oracles and the non-x86 fallback. The complete sum fits
//! u32 (at most 65,280); no cost-based early termination changes search ties.

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
#[cfg(any(test, not(target_arch = "x86_64")))]
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

/// Exact selected SAD with one bounds proof for each complete footprint.
pub(super) fn exact(
    current: &[u8],
    current_stride: usize,
    current_start: usize,
    reference: &[u8],
    reference_stride: usize,
    reference_start: usize,
) -> u32 {
    footprint(current, current_stride, current_start);
    footprint(reference, reference_stride, reference_start);

    #[cfg(target_arch = "x86_64")]
    // SAFETY: `footprint` proved every 16-byte row is in its allocation.
    unsafe {
        sse2(
            current.as_ptr().add(current_start),
            current_stride,
            reference.as_ptr().add(reference_start),
            reference_stride,
        )
    }

    #[cfg(not(target_arch = "x86_64"))]
    portable(
        current,
        current_stride,
        current_start,
        reference,
        reference_stride,
        reference_start,
    )
}

fn footprint(pixels: &[u8], stride: usize, start: usize) {
    let end = (BLOCK - 1)
        .checked_mul(stride)
        .and_then(|offset| start.checked_add(offset))
        .and_then(|last| last.checked_add(BLOCK))
        .expect("16x16 SAD footprint overflows usize");
    assert!(
        end <= pixels.len(),
        "16x16 SAD footprint extends past its pixel plane"
    );
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn sse2(
    mut current: *const u8,
    current_stride: usize,
    mut reference: *const u8,
    reference_stride: usize,
) -> u32 {
    use std::arch::x86_64::{
        __m128i, _mm_add_epi64, _mm_loadu_si128, _mm_sad_epu8, _mm_setzero_si128, _mm_storeu_si128,
    };

    let mut lanes = _mm_setzero_si128();
    for row in 0..BLOCK {
        // SAFETY: the wrapper proved both complete footprints in bounds.
        // Unaligned loads are intentional: candidate X positions have no
        // alignment contract. Stride zero and overlapping rows remain valid.
        let (a, b) = unsafe {
            (
                _mm_loadu_si128(current.cast::<__m128i>()),
                _mm_loadu_si128(reference.cast::<__m128i>()),
            )
        };
        lanes = _mm_add_epi64(lanes, _mm_sad_epu8(a, b));
        if row + 1 != BLOCK {
            // SAFETY: `footprint` proved the next row start and its load.
            unsafe {
                current = current.add(current_stride);
                reference = reference.add(reference_stride);
            }
        }
    }
    let mut sums = [0_u64; 2];
    // SAFETY: `sums` is exactly one unaligned 128-bit destination.
    unsafe { _mm_storeu_si128(sums.as_mut_ptr().cast::<__m128i>(), lanes) };
    u32::try_from(sums[0] + sums[1]).expect("a 16x16 byte SAD fits u32")
}

#[cfg(any(test, not(target_arch = "x86_64")))]
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
