//! The selected ONE X2 input reduction and motion-reference front end.
//!
//! This stops at the boundary passed to the two FDS engines. It deliberately
//! does not estimate flow, combine retained hints, run the temporal median,
//! build camera-validity masks or decide when a frame is scheduled. The
//! existing [`DisFlow`](crate::flow::dis::DisFlow) entry blurs its input itself
//! and therefore must not consume this module's already-blurred output; the
//! dedicated ONE X2 FDS entry will accept
//! [`BlurredBelts`](crate::flow::one_xs::temporal::BlurredBelts) directly.

use super::{COLS, Lens, ROWS};
use crate::flow::one_xs_belt::{ShapeError, SolverBelts, SourceBelts};
use std::sync::atomic::{AtomicU64, Ordering};

const PIXELS: usize = ROWS * COLS;

/// `GaussianBlur(Size(5,5), sigma=0.8, BORDER_REFLECT_101)`'s bit-exact U8
/// kernel in OpenCV 4.7's fixed-point path. The coefficients sum to 128.
const GAUSSIAN_Q7: [u32; 5] = [3, 29, 64, 29, 3];
const GAUSSIAN_SHIFT: u32 = 14;
const GAUSSIAN_ROUND: u32 = 1 << (GAUSSIAN_SHIFT - 1);

const CHANGE_THRESHOLD: u8 = 10;
const MOTION_ROWS_BEFORE: usize = 9;
const MOTION_ROWS_AFTER: usize = 10;
const MOTION_COLS_BEFORE: usize = 5;
const MOTION_COLS_AFTER: usize = 6;

static NEXT_FRONT_END_ID: AtomicU64 = AtomicU64::new(1);

fn next_front_end_id() -> u64 {
    NEXT_FRONT_END_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect("ONE X2 temporal front-end identity space exhausted")
}

/// The retained-resolution pair after Studio's selected input Gaussian.
///
/// This wrapper intentionally has no conversion back to [`SolverBelts`]. It
/// keeps an already-blurred pair out of the existing generic DIS entry, which
/// would otherwise apply its own Gaussian and blur the input a second time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlurredBelts {
    inner: SolverBelts,
}

impl BlurredBelts {
    /// Restore two already-blurred retained-resolution lens planes.
    ///
    /// This checks through [`SolverBelts`] but keeps that raw type private, so
    /// restored bytes cannot enter [`gaussian_blur`] for a second blur.
    pub fn from_lenses(lenses: super::LensPair<Vec<u8>>) -> Result<Self, ShapeError> {
        Ok(Self {
            inner: SolverBelts::from_lenses(lenses)?,
        })
    }

    pub fn lens(&self, lens: Lens) -> &[u8] {
        self.inner.lens(lens)
    }

    pub fn pixel(&self, lens: Lens, row: usize, col: usize) -> u8 {
        self.inner.pixel(lens, row, col)
    }

    pub fn bytes(&self) -> &[u8] {
        self.inner.bytes()
    }

    /// Consume the post-Gaussian pair into the estimator's physical lens slots.
    ///
    /// This is deliberately visible only inside the selected ONE X2 module.
    /// Keeping the raw [`SolverBelts`] member private means a caller cannot feed
    /// these bytes back through [`gaussian_blur`] for a second pass.
    pub(super) fn into_lenses(self) -> super::LensPair<Vec<u8>> {
        super::LensPair {
            a: self.inner.lens(Lens::A).to_vec(),
            b: self.inner.lens(Lens::B).to_vec(),
        }
    }
}

/// Apply Studio's in-place OpenCV 5-by-5, sigma-0.8 input blur to both
/// retained lens images.
///
/// The implementation retains each horizontal Q7 result and rounds only after
/// the vertical pass, exactly as the bit-exact U8 operation does.
/// `BORDER_REFLECT_101` mirrors without repeating the edge pixel.
pub fn gaussian_blur(input: &SolverBelts) -> BlurredBelts {
    let mut horizontal = vec![0u32; SolverBelts::BYTES];
    for lens in Lens::ALL {
        let base = lens.index() * PIXELS;
        for row in 0..ROWS {
            for col in 0..COLS {
                let mut sum = 0u32;
                for (tap, weight) in GAUSSIAN_Q7.into_iter().enumerate() {
                    let source_col = reflect_101(col as isize + tap as isize - 2, COLS);
                    sum += weight * u32::from(input.pixel(lens, row, source_col));
                }
                horizontal[base + row * COLS + col] = sum;
            }
        }
    }

    BlurredBelts {
        inner: SolverBelts::from_fn(|lens, row, col| {
            let base = lens.index() * PIXELS;
            let mut sum = 0u32;
            for (tap, weight) in GAUSSIAN_Q7.into_iter().enumerate() {
                let source_row = reflect_101(row as isize + tap as isize - 2, ROWS);
                sum += weight * horizontal[base + source_row * COLS + col];
            }
            ((sum + GAUSSIAN_ROUND) >> GAUSSIAN_SHIFT) as u8
        }),
    }
}

/// Studio's one-channel motion byte field.
///
/// A byte begins as the threshold comparison, 0 or 1. A qualifying local
/// population promotes it to 255; bytes outside qualifying windows retain the
/// original 0/1 value rather than being normalised back to zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MotionMask {
    bytes: Vec<u8>,
}

impl MotionMask {
    /// Compare a current blurred pair with the preceding blurred references.
    pub fn between(current: &BlurredBelts, references: &BlurredBelts) -> Self {
        let mut base = vec![0; PIXELS];
        for (index, value) in base.iter_mut().enumerate() {
            let a = current.lens(Lens::A)[index].abs_diff(references.lens(Lens::A)[index]);
            let b = current.lens(Lens::B)[index].abs_diff(references.lens(Lens::B)[index]);
            *value = u8::from(a.max(b) >= CHANGE_THRESHOLD);
        }
        Self {
            bytes: promote_motion(&base),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Whether the selected call uses the cold or motion-aware native entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum History {
    /// No preceding reference exists. Native code calls the cold estimator and
    /// clones this pair into its two reference members.
    Cold,
    /// A preceding reference exists. Native code supplies a motion mask and
    /// updates both references after comparing them.
    Warm,
}

/// Inputs and motion state produced for one estimator call.
///
/// This is a linear handoff token: [`FrontEnd::finish`] consumes it, so one
/// prepared image pair cannot be committed twice. While a token is
/// outstanding the front end refuses another preparation. If the downstream
/// solve aborts, [`FrontEnd::reset`] abandons the token and makes the front end
/// ready again.
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::temporal::{FrontEnd, Prepared};
///
/// fn finish_twice(front_end: &mut FrontEnd, prepared: Prepared) {
///     front_end.finish(prepared);
///     front_end.finish(prepared); // `prepared` was moved by the first call.
/// }
/// ```
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::temporal::Prepared;
///
/// fn duplicate(prepared: Prepared) {
///     let _copy: Prepared = prepared.clone();
/// }
/// ```
#[must_use = "finish this prepared input after the solver returns, or reset the front end"]
#[derive(Debug, PartialEq, Eq)]
pub struct Prepared {
    current: BlurredBelts,
    motion: Option<MotionMask>,
    owner_id: u64,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Ready,
    Prepared,
}

impl Prepared {
    pub fn current(&self) -> &BlurredBelts {
        &self.current
    }

    pub fn motion(&self) -> Option<&MotionMask> {
        self.motion.as_ref()
    }

    pub fn history(&self) -> History {
        if self.motion.is_some() {
            History::Warm
        } else {
            History::Cold
        }
    }
}

/// Stateful prior-image references owned by `OpticalFlow::calcWithMotion`.
///
/// This is intentionally only the input/reference front end. A future caller
/// remains responsible for FDS, retained flow hints and scheduling.
#[derive(Debug, PartialEq, Eq)]
pub struct FrontEnd {
    references: Option<BlurredBelts>,
    owner_id: u64,
    generation: u64,
    phase: Phase,
}

impl Default for FrontEnd {
    fn default() -> Self {
        Self {
            references: None,
            owner_id: next_front_end_id(),
            generation: 0,
            phase: Phase::Ready,
        }
    }
}

impl FrontEnd {
    /// Reduce and blur a high-resolution pair without updating its references.
    ///
    /// Native runs the downstream directional FDS calls after this work. Call
    /// [`Self::finish`] only after those calls return normally.
    pub fn prepare_sources(&mut self, source: &SourceBelts) -> Prepared {
        self.assert_ready();
        self.prepare_blurred(gaussian_blur(&source.reduce_area_3x3()))
    }

    /// Blur a retained-resolution pair without updating its references.
    ///
    /// Native runs the downstream directional FDS calls after this work. Call
    /// [`Self::finish`] only after those calls return normally.
    pub fn prepare_retained(&mut self, input: &SolverBelts) -> Prepared {
        self.assert_ready();
        self.prepare_blurred(gaussian_blur(input))
    }

    /// Complete one normally returned native call.
    ///
    /// The first call clones the blurred pair. A warm call applies the 0.7/0.3
    /// update. Studio performs each lens update immediately after that lens's
    /// void FDS task returns, before joining the task group; this pair-level
    /// commit has the same observable state at the group boundary. A caller
    /// that unwinds or otherwise does not reach normal return must not call
    /// this method.
    pub fn finish(&mut self, prepared: Prepared) {
        assert_eq!(
            self.owner_id, prepared.owner_id,
            "ONE X2 temporal input belongs to a different front end"
        );
        assert_eq!(
            self.generation, prepared.generation,
            "ONE X2 temporal input was reset, superseded or already finished"
        );
        assert_eq!(
            self.phase,
            Phase::Prepared,
            "ONE X2 temporal front end has no outstanding prepared input"
        );
        let next_generation = self
            .generation
            .checked_add(1)
            .expect("ONE X2 temporal generation space exhausted");
        let history = prepared.history();
        let Prepared { current, .. } = prepared;
        match (&mut self.references, history) {
            (references @ None, History::Cold) => {
                *references = Some(current);
            }
            (Some(references), History::Warm) => {
                *references = next_warm_references(references, &current);
            }
            _ => panic!("ONE X2 temporal history changed between prepare and finish"),
        }
        self.generation = next_generation;
        self.phase = Phase::Ready;
    }

    /// Drop both native prior-image references. The next pair is cold again.
    ///
    /// This also abandons an outstanding [`Prepared`] token. Its generation no
    /// longer matches, so attempting to finish that stale token cannot commit.
    pub fn reset(&mut self) {
        let next_generation = self
            .generation
            .checked_add(1)
            .expect("ONE X2 temporal generation space exhausted");
        self.references = None;
        self.generation = next_generation;
        self.phase = Phase::Ready;
    }

    pub fn references(&self) -> Option<&BlurredBelts> {
        self.references.as_ref()
    }

    fn prepare_blurred(&mut self, current: BlurredBelts) -> Prepared {
        self.assert_ready();
        let motion = self
            .references
            .as_ref()
            .map(|references| MotionMask::between(&current, references));
        let prepared = Prepared {
            current,
            motion,
            owner_id: self.owner_id,
            generation: self.generation,
        };
        self.phase = Phase::Prepared;
        prepared
    }

    fn assert_ready(&self) {
        assert_eq!(
            self.phase,
            Phase::Ready,
            "ONE X2 temporal front end already has an outstanding prepared input"
        );
    }
}

fn reflect_101(index: isize, len: usize) -> usize {
    debug_assert!(len > 2);
    if index < 0 {
        (-index) as usize
    } else if index >= len as isize {
        (2 * len as isize - index - 2) as usize
    } else {
        index as usize
    }
}

/// Studio constructs `old*0.7 + current*0.3` as a `MatExpr`, which OpenCV 4.7
/// lowers to `cv::addWeighted`. The optimized arm64 U8 path selected in the
/// authenticated run converts the weights to f32, starts its accumulator at
/// `gamma + 0.5`, fused-adds the old term and then the current term, truncates
/// toward zero and saturating-narrows to U8. The contiguous 64,800-byte plane
/// is an exact multiple of its 32-byte SIMD loop, so no scalar tail runs.
///
/// This deliberately describes the selected optimized path. Other OpenCV
/// `addWeighted` implementations need not use this conversion sequence.
fn ema_byte(reference: u8, current: u8) -> u8 {
    let biased_reference = (reference as f32).mul_add(0.7, 0.5);
    let blended = (current as f32).mul_add(0.3, biased_reference);
    blended.trunc().clamp(0.0, 255.0) as u8
}

/// Compute the references committed after one normally returned warm pair.
///
/// This is visible to the sibling warm-checkpoint composer so it can export
/// the same already-READ transition without duplicating the selected OpenCV
/// arithmetic. It does not mutate either input.
pub(super) fn next_warm_references(
    reference: &BlurredBelts,
    current: &BlurredBelts,
) -> BlurredBelts {
    BlurredBelts {
        inner: SolverBelts::from_fn(|lens, row, col| {
            ema_byte(
                reference.pixel(lens, row, col),
                current.pixel(lens, row, col),
            )
        }),
    }
}

fn motion_bounds(row: usize, col: usize) -> (usize, usize, usize, usize) {
    // These are native integral indices, not symmetric saturating
    // subtractions. The low edge deliberately begins at one.
    let y0 = row.max(MOTION_ROWS_AFTER) - MOTION_ROWS_BEFORE;
    let y1 = (row + MOTION_ROWS_AFTER).min(ROWS - 1) + 1;
    let x0 = col.max(MOTION_COLS_AFTER) - MOTION_COLS_BEFORE;
    let x1 = (col + MOTION_COLS_AFTER).min(COLS - 1) + 1;
    (y0, y1, x0, x1)
}

fn promote_motion(base: &[u8]) -> Vec<u8> {
    debug_assert_eq!(base.len(), PIXELS);
    let integral_cols = COLS + 1;
    let mut integral = vec![0u32; (ROWS + 1) * integral_cols];
    for row in 0..ROWS {
        let mut row_sum = 0u32;
        for col in 0..COLS {
            row_sum += u32::from(base[row * COLS + col]);
            integral[(row + 1) * integral_cols + col + 1] =
                integral[row * integral_cols + col + 1] + row_sum;
        }
    }

    let mut motion = base.to_vec();
    for row in 0..ROWS {
        for col in 0..COLS {
            let (y0, y1, x0, x1) = motion_bounds(row, col);
            // Sum the strip between y0/y1 first on each x boundary, then
            // subtract the narrower prefix. Every subtraction is monotonic,
            // so the u32 intermediates cannot underflow in a debug build.
            let through_x1 = integral[y1 * integral_cols + x1] - integral[y0 * integral_cols + x1];
            let through_x0 = integral[y1 * integral_cols + x0] - integral[y0 * integral_cols + x0];
            let count = through_x1 - through_x0;
            let area = (y1 - y0) * (x1 - x0);
            // arm64 `fcvtzs` truncates this positive f32 value toward zero.
            let threshold = (0.2f32 * area as f32).trunc() as u32;
            if count >= threshold {
                motion[row * COLS + col] = 255;
            }
        }
    }
    motion
}

#[cfg(test)]
mod tests {
    use super::super::LensPair;
    use super::*;

    fn pixel(row: usize, col: usize) -> usize {
        row * COLS + col
    }

    fn constant(value: u8) -> Vec<u8> {
        vec![value; PIXELS]
    }

    fn constant_belts(value: u8) -> SolverBelts {
        SolverBelts::from_fn(|_, _, _| value)
    }

    fn constant_blurred(value: u8) -> BlurredBelts {
        BlurredBelts {
            inner: constant_belts(value),
        }
    }

    #[test]
    fn blurred_restore_checks_each_lens_and_preserves_bytes() {
        let a = SolverBelts::from_fn(|_, row, col| ((row + col) % 251) as u8)
            .lens(Lens::A)
            .to_vec();
        let b = vec![197; PIXELS];
        let restored = BlurredBelts::from_lenses(LensPair {
            a: a.clone(),
            b: b.clone(),
        })
        .unwrap();
        assert_eq!(restored.lens(Lens::A), a);
        assert_eq!(restored.lens(Lens::B), b);

        let error = BlurredBelts::from_lenses(LensPair {
            a: vec![0; PIXELS],
            b: vec![0; PIXELS - 1],
        })
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "ONE X2 lens B belt has {} bytes, expected {PIXELS}",
                PIXELS - 1
            ),
        );
    }

    #[test]
    fn gaussian_impulse_is_the_opencv_u8_kernel_and_rounding() {
        let centre = (500, 30);
        let input = SolverBelts::from_fn(|lens, row, col| {
            u8::from(lens == Lens::A && (row, col) == centre) * 255
        });
        let blurred = gaussian_blur(&input);
        let expected = [
            [0, 1, 3, 1, 0],
            [1, 13, 29, 13, 1],
            [3, 29, 64, 29, 3],
            [1, 13, 29, 13, 1],
            [0, 1, 3, 1, 0],
        ];
        for (dy, row) in expected.into_iter().enumerate() {
            for (dx, value) in row.into_iter().enumerate() {
                assert_eq!(
                    blurred.pixel(Lens::A, centre.0 + dy - 2, centre.1 + dx - 2),
                    value
                );
            }
        }
        // Per-pixel U8 rounding makes the displayed impulse sum 252 even
        // though the fixed-point kernel itself has unit gain.
        assert_eq!(
            blurred
                .lens(Lens::A)
                .iter()
                .map(|value| u32::from(*value))
                .sum::<u32>(),
            252
        );
        assert!(blurred.lens(Lens::B).iter().all(|value| *value == 0));
    }

    #[test]
    fn gaussian_reflects_without_repeating_the_edge() {
        assert_eq!(reflect_101(-2, COLS), 2);
        assert_eq!(reflect_101(-1, COLS), 1);
        assert_eq!(reflect_101(0, COLS), 0);
        assert_eq!(reflect_101(COLS as isize - 1, COLS), COLS - 1);
        assert_eq!(reflect_101(COLS as isize, COLS), COLS - 2);
        assert_eq!(reflect_101(COLS as isize + 1, COLS), COLS - 3);

        let flat = gaussian_blur(&constant_belts(137));
        assert!(flat.bytes().iter().all(|value| *value == 137));
    }

    #[test]
    fn ema_matches_the_selected_opencv_simd_bias_fma_order_and_saturation() {
        // 58/73 distinguishes bias-and-truncate from nearest-even. 52/17 and
        // 67/12 distinguish putting the bias before the old term and adding
        // old before current from the reversed or post-sum alternatives.
        // The asymmetric endpoints bind which operand receives each weight.
        for (reference, current, expected) in [
            (58, 73, 63),
            (52, 17, 41),
            (67, 12, 50),
            (0, 5, 2),
            (0, 15, 5),
            (1, 6, 3),
            (3, 28, 11),
            (255, 0, 179),
            (0, 255, 77),
            (255, 255, 255),
        ] {
            assert_eq!(ema_byte(reference, current), expected);
        }
    }

    #[test]
    fn motion_begins_as_the_maximum_pair_difference_at_inclusive_ten() {
        // Construct already-blurred inputs directly: this test isolates the
        // threshold operation from the separately tested Gaussian front end.
        let references = constant_blurred(100);
        let mut a = constant(100);
        let mut b = constant(100);
        a[pixel(500, 30)] = 109;
        b[pixel(500, 30)] = 110;
        let current = BlurredBelts {
            inner: SolverBelts::from_lenses(LensPair { a, b }).unwrap(),
        };
        let motion = MotionMask::between(&current, &references);
        assert_eq!(motion.bytes[pixel(500, 30)], 1);
        assert_eq!(motion.bytes.iter().filter(|value| **value != 0).count(), 1);
    }

    #[test]
    fn population_promotes_at_exactly_twenty_percent() {
        let target = (100, 30);
        let mut base = constant(0);
        // Target window is rows 91..111 and columns 25..37: area 240,
        // threshold 48. Keep the target itself zero so 0 -> 255 is explicit.
        for row in 91..95 {
            for col in 25..37 {
                base[pixel(row, col)] = 1;
            }
        }
        let promoted = promote_motion(&base);
        assert_eq!(promoted[pixel(target.0, target.1)], 255);

        base[pixel(94, 36)] = 0;
        let below = promote_motion(&base);
        assert_eq!(below[pixel(target.0, target.1)], 0);
    }

    #[test]
    fn low_edge_population_excludes_integral_row_and_column_zero() {
        let mut base = constant(0);
        // Neither set contributes to (0,0)'s rectangle [1,11)x[1,7).
        for col in 1..COLS {
            base[pixel(0, col)] = 1;
        }
        for row in 1..ROWS {
            base[pixel(row, 0)] = 1;
        }
        assert_eq!(promote_motion(&base)[pixel(0, 0)], 0);

        // The clipped area is 10*6=60, so twelve included changes promote.
        for row in 1..3 {
            for col in 1..7 {
                base[pixel(row, col)] = 1;
            }
        }
        assert_eq!(promote_motion(&base)[pixel(0, 0)], 255);
    }

    #[test]
    fn clipped_population_truncates_a_fractional_twenty_percent() {
        let target = (1, 1);
        assert_eq!(motion_bounds(target.0, target.1), (1, 12, 1, 8));
        // Area 11*7=77 gives fcvtzs(15.4)=15. Keep the target byte zero so
        // promotion itself is visible.
        let mut base = constant(0);
        for row in 2..4 {
            for col in 1..8 {
                base[pixel(row, col)] = 1;
            }
        }
        assert_eq!(promote_motion(&base)[pixel(target.0, target.1)], 0);
        base[pixel(4, 1)] = 1;
        assert_eq!(promote_motion(&base)[pixel(target.0, target.1)], 255);
    }

    #[test]
    fn motion_bounds_guard_low_transitions_and_high_edges() {
        assert_eq!(motion_bounds(0, 0), (1, 11, 1, 7));
        assert_eq!(motion_bounds(9, 5), (1, 20, 1, 12));
        assert_eq!(motion_bounds(10, 6), (1, 21, 1, 13));
        assert_eq!(
            motion_bounds(ROWS - 1, COLS - 1),
            (ROWS - 10, ROWS, COLS - 6, COLS)
        );
    }

    #[test]
    fn first_reference_clones_the_blurred_pair_not_the_unblurred_input() {
        let centre = (500, 30);
        let input = SolverBelts::from_fn(|lens, row, col| {
            u8::from(lens == Lens::A && (row, col) == centre) * 255
        });
        let expected = gaussian_blur(&input);
        let mut front_end = FrontEnd::default();
        let prepared = front_end.prepare_retained(&input);
        assert_eq!(prepared.current(), &expected);
        assert!(front_end.references().is_none());
        front_end.finish(prepared);
        assert_eq!(front_end.references(), Some(&expected));
        assert_eq!(expected.pixel(Lens::A, centre.0, centre.1), 64);
        assert_eq!(expected.pixel(Lens::A, centre.0, centre.1 - 1), 29);
    }

    #[test]
    fn first_frame_clones_warm_frames_update_and_reset_is_cold() {
        let mut front_end = FrontEnd::default();
        let first = front_end.prepare_retained(&constant_belts(100));
        assert_eq!(first.history(), History::Cold);
        assert!(first.motion().is_none());
        assert!(front_end.references().is_none());
        front_end.finish(first);
        assert!(
            front_end
                .references()
                .unwrap()
                .lens(Lens::A)
                .iter()
                .all(|value| *value == 100)
        );

        let second = front_end.prepare_retained(&constant_belts(120));
        assert_eq!(second.history(), History::Warm);
        assert!(
            second
                .motion()
                .unwrap()
                .bytes()
                .iter()
                .all(|value| *value == 255)
        );
        // The old reference remains visible until both downstream directional
        // calls have returned and the pair-level group boundary is finished.
        assert!(
            front_end
                .references()
                .unwrap()
                .lens(Lens::A)
                .iter()
                .all(|value| *value == 100)
        );
        front_end.finish(second);
        assert!(
            front_end
                .references()
                .unwrap()
                .lens(Lens::A)
                .iter()
                .all(|value| *value == 106)
        );

        front_end.reset();
        assert!(front_end.references().is_none());
        let after_reset = front_end.prepare_retained(&constant_belts(200));
        assert_eq!(after_reset.history(), History::Cold);
        assert!(after_reset.motion().is_none());
        front_end.finish(after_reset);
        assert!(
            front_end
                .references()
                .unwrap()
                .lens(Lens::B)
                .iter()
                .all(|value| *value == 200)
        );
    }

    #[test]
    fn reset_invalidates_an_outstanding_prepared_pair() {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        let mut front_end = FrontEnd::default();
        let prepared = front_end.prepare_blurred(constant_blurred(1));
        front_end.reset();
        assert!(catch_unwind(AssertUnwindSafe(|| front_end.finish(prepared))).is_err());

        let fresh = front_end.prepare_blurred(constant_blurred(2));
        front_end.finish(fresh);
        assert_eq!(front_end.references(), Some(&constant_blurred(2)));
    }

    #[test]
    fn a_second_prepare_is_rejected_while_the_first_is_outstanding() {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        let mut front_end = FrontEnd::default();
        let first = front_end.prepare_blurred(constant_blurred(1));
        let sibling = catch_unwind(AssertUnwindSafe(|| {
            front_end.prepare_blurred(constant_blurred(2))
        }));
        let panic = sibling.unwrap_err();
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap();
        assert!(
            message.contains("already has an outstanding prepared input"),
            "unexpected panic: {message}",
        );

        front_end.finish(first);
        let next = front_end.prepare_blurred(constant_blurred(3));
        assert_eq!(next.history(), History::Warm);
        front_end.finish(next);
    }

    #[test]
    #[should_panic(expected = "belongs to a different front end")]
    fn a_prepared_pair_is_bound_to_the_front_end_that_created_it() {
        let mut source = FrontEnd::default();
        let prepared = source.prepare_blurred(constant_blurred(1));
        let mut unrelated = FrontEnd::default();
        unrelated.finish(prepared);
    }

    #[test]
    fn generation_exhaustion_cannot_partially_finish_or_reset() {
        use std::panic::{AssertUnwindSafe, catch_unwind};

        let mut cold = FrontEnd {
            generation: u64::MAX,
            ..FrontEnd::default()
        };
        let prepared = cold.prepare_blurred(constant_blurred(1));
        assert!(catch_unwind(AssertUnwindSafe(|| cold.finish(prepared))).is_err());
        assert!(cold.references.is_none());
        assert_eq!(cold.generation, u64::MAX);
        assert_eq!(cold.phase, Phase::Prepared);

        let mut warm = FrontEnd::default();
        let first = warm.prepare_blurred(constant_blurred(2));
        warm.finish(first);
        let references = warm.references.clone();
        warm.generation = u64::MAX;
        assert!(catch_unwind(AssertUnwindSafe(|| warm.reset())).is_err());
        assert_eq!(warm.references, references);
        assert_eq!(warm.generation, u64::MAX);
    }

    #[test]
    #[ignore = "requires KJERAG_ONE_XS_WARM_PAYLOAD with the authenticated run-06 payload"]
    fn accepted_run06_motion_and_reference_updates_are_bit_exact() {
        use sha2::{Digest as _, Sha256};

        const PAYLOAD_BYTES: usize = 7_238_679;
        const PAYLOAD_SHA256: &str =
            "a5311370d7946d45094cc85c8ff6631bf2b1195e8c127e3b1f909310168ce8c1";
        const CURRENT_A: usize = 37;
        const CURRENT_B: usize = 64_866;
        const PRIOR_A: usize = 129_706;
        const PRIOR_B: usize = 194_546;
        const MOTION: usize = 389_043;
        const MOTION_SHA256: &str =
            "e80db2c4a997464700cad6f0e4d6fd35104935bfe76ddd97e8bfac5a8cf00554";
        const POST_A: usize = 4_869_320;
        const POST_B: usize = 4_934_151;

        let path = std::env::var_os("KJERAG_ONE_XS_WARM_PAYLOAD")
            .expect("set KJERAG_ONE_XS_WARM_PAYLOAD to authenticated run-06 warm-pair-payload.bin");
        let payload = std::fs::read(path).unwrap();
        assert_eq!(payload.len(), PAYLOAD_BYTES);
        assert_eq!(sha256_hex(&payload), PAYLOAD_SHA256);
        assert_eq!(&payload[..8], b"KJWP602\x04");

        let current = BlurredBelts {
            inner: SolverBelts::from_lenses(LensPair {
                a: record(&payload, CURRENT_A).to_vec(),
                b: record(&payload, CURRENT_B).to_vec(),
            })
            .unwrap(),
        };
        let reference = BlurredBelts {
            inner: SolverBelts::from_lenses(LensPair {
                a: record(&payload, PRIOR_A).to_vec(),
                b: record(&payload, PRIOR_B).to_vec(),
            })
            .unwrap(),
        };
        let expected_motion = record(&payload, MOTION);
        assert_eq!(sha256_hex(expected_motion), MOTION_SHA256);
        let motion = MotionMask::between(&current, &reference);
        assert_eq!(motion.bytes(), expected_motion);
        assert_eq!(sha256_hex(motion.bytes()), MOTION_SHA256);

        let actual = next_warm_references(&reference, &current);
        assert_eq!(actual.lens(Lens::A), record(&payload, POST_A));
        assert_eq!(actual.lens(Lens::B), record(&payload, POST_B));

        fn record(payload: &[u8], offset: usize) -> &[u8] {
            let end = offset.checked_add(PIXELS).expect("record end overflowed");
            payload
                .get(offset..end)
                .expect("authenticated run-06 record is outside the payload")
        }

        fn sha256_hex(bytes: &[u8]) -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }
    }

    #[test]
    #[ignore = "requires detached private V6 and derived-oracle directories"]
    fn detached_v6_inputs_match_the_byte_oracles() {
        use std::path::PathBuf;

        let corpus = PathBuf::from(
            std::env::var_os("KJERAG_ONE_XS_V6_DIR")
                .expect("set KJERAG_ONE_XS_V6_DIR to the detached V6 corpus"),
        );
        let oracle = PathBuf::from(
            std::env::var_os("KJERAG_ONE_XS_V6_ORACLE_DIR")
                .expect("set KJERAG_ONE_XS_V6_ORACLE_DIR to the detached derived payloads"),
        );
        let source = SourceBelts::from_lenses(LensPair {
            a: std::fs::read(corpus.join("00038_target_input_belt_b70.bin")).unwrap(),
            b: std::fs::read(corpus.join("00039_target_input_belt_bd0.bin")).unwrap(),
        })
        .unwrap();
        let expected_area = LensPair {
            a: std::fs::read(oracle.join("retained-b70-area.u8")).unwrap(),
            b: std::fs::read(oracle.join("retained-bd0-area.u8")).unwrap(),
        };
        let expected_blurred = LensPair {
            a: std::fs::read(oracle.join("retained-b70-area-gauss5-sigma0p8-reflect101.u8"))
                .unwrap(),
            b: std::fs::read(oracle.join("retained-bd0-area-gauss5-sigma0p8-reflect101.u8"))
                .unwrap(),
        };

        let area = source.reduce_area_3x3();
        let prepared = FrontEnd::default().prepare_sources(&source);
        assert_eq!(prepared.history(), History::Cold);
        for lens in Lens::ALL {
            assert_eq!(area.lens(lens), expected_area.get(lens));
            assert_eq!(prepared.current().lens(lens), expected_blurred.get(lens));
        }
    }
}
