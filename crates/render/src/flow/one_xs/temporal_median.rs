//! Studio's selected ONE X2 finest-level temporal median.
//!
//! This is a patch-grid operation, not a dense-field post-process. Native FDS
//! applies it to component zero after the finest-level patch solve and before
//! densification; component one passes through untouched. The direction types
//! here keep A-to-B's negative quantizer scale separate from B-to-A's positive
//! scale, and each direction owns independent history just as the two native
//! FDS objects do.
//!
//! [`FrontEnd`](crate::flow::one_xs::temporal::FrontEnd) owns the earlier
//! blurred-image references and motion mask. Its reset is deliberately
//! independent of this history. Native `resetOptFlowForStitchImage` disables
//! the FDS temporal gate but does not destroy its temporal object;
//! [`AtoBMedian`](crate::flow::one_xs::temporal_median::AtoBMedian),
//! [`BtoAMedian`](crate::flow::one_xs::temporal_median::BtoAMedian) and
//! [`TemporalMedians`](crate::flow::one_xs::temporal_median::TemporalMedians)
//! expose clears that model object reconstruction, not that gate.
//!
//! The owned state below retains the native histogram and FIFO semantics but
//! deliberately omits its incremental-search caches. Scanning the histogram
//! produces the same selected result and reproduces both directions of the
//! authenticated target transition bit for bit. This state is owned by the
//! selected cold/warm production lineage; the authenticated target transition
//! remains its bit-exact oracle.

use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

use super::pis::{AtoB, BtoA, Level, PatchGrid as UnfilteredPatchGrid, PisDirection};
use super::{COLS, Direction, ROWS};

pub use super::{PATCH_SIZE, PATCH_STRIDE};

/// Native solves level 1 at half the retained dimensions.
pub const FINEST_ROWS: usize = ROWS / 2;
pub const FINEST_COLS: usize = COLS / 2;

/// FDS's selected finest-level patch geometry.
pub const PATCH_ROWS: usize = (FINEST_ROWS - PATCH_SIZE) / PATCH_STRIDE + 1;
pub const PATCH_COLS: usize = (FINEST_COLS - PATCH_SIZE) / PATCH_STRIDE + 1;
pub const PATCHES: usize = PATCH_ROWS * PATCH_COLS;

/// Temporal level 1 selects three accepted samples.
pub const HISTORY_CAPACITY: usize = 3;

/// Captured selected histogram width for every finest-level patch.
pub const HISTOGRAM_BINS: usize = 159;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Quantization {
    base: f32,
    scale: f32,
    bins: usize,
}

impl Quantization {
    /// Reproduce the captured selected finest temporal quantizer.
    fn selected(direction: Direction) -> Self {
        let (base, scale) = match direction {
            Direction::AtoB => (f32::from_bits(0x3f7f_fc66), f32::from_bits(0xc120_0000)),
            Direction::BtoA => (f32::from_bits(0xbf7f_fc66), f32::from_bits(0x4120_0000)),
        };

        Self {
            base,
            scale,
            bins: HISTOGRAM_BINS,
        }
    }

    /// Rust's saturating float-to-int cast has the selected AArch64
    /// `fcvtzs` behavior: round toward zero, saturate overflow, and map NaN to
    /// zero. Keep the subtract and multiply separate so their f32 rounding is
    /// not algebraically rearranged.
    fn bin(self, value: f32) -> i32 {
        let shifted = value - self.base;
        let scaled = shifted * self.scale;
        scaled as i32
    }

    fn value(self, bin: usize) -> f32 {
        let offset = bin as f32 / self.scale;
        self.base + offset
    }
}

/// Owned semantic state for one direction's selected temporal median.
///
/// Histograms are patch-major and contain exactly [`HISTOGRAM_BINS`] bytes
/// per patch. `offsets` indexes each patch's FIFO-ordered values.
#[derive(Clone, Debug, PartialEq)]
pub struct MedianState<D: PisDirection> {
    histogram: Vec<u8>,
    offsets: Vec<u32>,
    values: Vec<f32>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> MedianState<D> {
    pub fn from_parts(
        histogram: Vec<u8>,
        offsets: Vec<u32>,
        values: Vec<f32>,
    ) -> Result<Self, InvalidMedianState> {
        let state = Self {
            histogram,
            offsets,
            values,
            direction: PhantomData,
        };
        state.validate()?;
        Ok(state)
    }

    pub fn histogram(&self) -> &[u8] {
        &self.histogram
    }

    pub fn offsets(&self) -> &[u32] {
        &self.offsets
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    pub fn into_parts(self) -> (Vec<u8>, Vec<u32>, Vec<f32>) {
        (self.histogram, self.offsets, self.values)
    }

    fn validate(&self) -> Result<(), InvalidMedianState> {
        let expected_histogram = PATCHES * HISTOGRAM_BINS;
        if self.histogram.len() != expected_histogram {
            return Err(InvalidMedianState::new(format!(
                "ONE X2 {} temporal state has {} histogram bytes, expected {expected_histogram}",
                D::DIRECTION,
                self.histogram.len(),
            )));
        }
        if self.offsets.len() != PATCHES + 1 {
            return Err(InvalidMedianState::new(format!(
                "ONE X2 {} temporal state has {} offsets, expected {}",
                D::DIRECTION,
                self.offsets.len(),
                PATCHES + 1,
            )));
        }
        if self.offsets[0] != 0 {
            return Err(InvalidMedianState::new(format!(
                "ONE X2 {} temporal state offsets must start at zero",
                D::DIRECTION,
            )));
        }

        let values_len = self.values.len();
        let mut recomputed = vec![0u8; expected_histogram];
        for patch in 0..PATCHES {
            let start = self.offsets[patch] as usize;
            let end = self.offsets[patch + 1] as usize;
            if start > values_len || end > values_len {
                return Err(InvalidMedianState::new(format!(
                    "ONE X2 {} temporal state patch {patch} offset is out of bounds",
                    D::DIRECTION,
                )));
            }
            if end < start {
                return Err(InvalidMedianState::new(format!(
                    "ONE X2 {} temporal state offsets are not monotonic at patch {patch}",
                    D::DIRECTION,
                )));
            }
            if end - start > HISTORY_CAPACITY {
                return Err(InvalidMedianState::new(format!(
                    "ONE X2 {} temporal state patch {patch} has {} values, expected at most {HISTORY_CAPACITY}",
                    D::DIRECTION,
                    end - start,
                )));
            }

            for (fifo_index, value) in self.values[start..end].iter().enumerate() {
                let bin = Quantization::selected(D::DIRECTION).bin(*value);
                let Ok(bin) = usize::try_from(bin) else {
                    return Err(InvalidMedianState::new(format!(
                        "ONE X2 {} temporal state patch {patch} value {fifo_index} is outside the histogram",
                        D::DIRECTION,
                    )));
                };
                if bin >= HISTOGRAM_BINS {
                    return Err(InvalidMedianState::new(format!(
                        "ONE X2 {} temporal state patch {patch} value {fifo_index} is outside the histogram",
                        D::DIRECTION,
                    )));
                }
                recomputed[patch * HISTOGRAM_BINS + bin] += 1;
            }
        }

        if self.offsets[PATCHES] as usize != values_len {
            return Err(InvalidMedianState::new(format!(
                "ONE X2 {} temporal state ends at offset {}, expected {values_len}",
                D::DIRECTION,
                self.offsets[PATCHES],
            )));
        }
        if recomputed != self.histogram {
            return Err(InvalidMedianState::new(format!(
                "ONE X2 {} temporal state histogram does not match its values",
                D::DIRECTION,
            )));
        }
        Ok(())
    }
}

/// A serialized temporal median state is not internally consistent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidMedianState {
    message: String,
}

impl InvalidMedianState {
    fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for InvalidMedianState {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.message)
    }
}

impl Error for InvalidMedianState {}

#[derive(Clone)]
struct PixelState {
    histogram: [u8; HISTOGRAM_BINS],
    // A fixed-capacity ring has the same pop-front/push-back order as the
    // native deque while making the selected capacity part of the type.
    history: [f32; HISTORY_CAPACITY],
    head: usize,
    len: usize,
}

impl Default for PixelState {
    fn default() -> Self {
        Self {
            histogram: [0; HISTOGRAM_BINS],
            history: [0.0; HISTORY_CAPACITY],
            head: 0,
            len: 0,
        }
    }
}

impl PixelState {
    fn update(&mut self, current: f32, quantization: Quantization) -> Option<f32> {
        let current_bin = quantization.bin(current);
        let current_bin = usize::try_from(current_bin).ok()?;
        if current_bin >= quantization.bins {
            return None;
        }

        if self.len == HISTORY_CAPACITY {
            let oldest = self.history[self.head];
            let oldest_bin = usize::try_from(quantization.bin(oldest))
                .expect("accepted ONE X2 temporal sample changed to a negative bin");
            assert!(
                oldest_bin < quantization.bins,
                "accepted ONE X2 temporal sample changed beyond the histogram",
            );
            self.histogram[oldest_bin] = self.histogram[oldest_bin].wrapping_sub(1);

            // Replacing the old head and then advancing it is pop-front
            // followed by push-back for a full three-element ring.
            self.history[self.head] = current;
            self.head = (self.head + 1) % HISTORY_CAPACITY;
        } else {
            let tail = (self.head + self.len) % HISTORY_CAPACITY;
            self.history[tail] = current;
            self.len += 1;
        }
        self.histogram[current_bin] = self.histogram[current_bin].wrapping_add(1);

        // Native uses the one-based rank `(available + 1) / 2`. This is 1,
        // 1, 2 for one, two and three available samples respectively.
        let rank = self.len.div_ceil(2);
        let mut cumulative = 0usize;
        let median_bin = self
            .histogram
            .iter()
            .take(quantization.bins)
            .position(|count| {
                cumulative += usize::from(*count);
                cumulative >= rank
            })
            .expect("ONE X2 temporal histogram lost an accepted sample");
        Some(quantization.value(median_bin))
    }
}

struct DirectionalMedian {
    quantization: Quantization,
    pixels: Vec<PixelState>,
}

impl DirectionalMedian {
    fn selected(direction: Direction) -> Self {
        Self {
            quantization: Quantization::selected(direction),
            pixels: vec![PixelState::default(); PATCHES],
        }
    }

    fn run(&mut self, dcol: &mut [f32]) {
        assert_eq!(
            dcol.len(),
            PATCHES,
            "checked ONE X2 patch grid changed shape",
        );
        for (state, current) in self.pixels.iter_mut().zip(dcol) {
            if let Some(filtered) = state.update(*current, self.quantization) {
                *current = filtered;
            }
        }
    }

    fn run_finest<D: PisDirection>(
        &mut self,
        patches: UnfilteredPatchGrid<D>,
    ) -> FilteredPatchGrid<D> {
        let (level, mut dcol, drow) = patches.into_row_major_components();
        debug_assert_eq!(level, Level::One);
        self.run(&mut dcol);
        FilteredPatchGrid::from_components(dcol, drow)
    }

    fn clear(&mut self) {
        for state in &mut self.pixels {
            *state = PixelState::default();
        }
    }

    fn from_state<D: PisDirection>(state: MedianState<D>) -> Result<Self, InvalidMedianState> {
        state.validate()?;
        let quantization = Quantization::selected(D::DIRECTION);
        let mut pixels = Vec::with_capacity(PATCHES);
        for patch in 0..PATCHES {
            let histogram_start = patch * HISTOGRAM_BINS;
            let values_start = state.offsets[patch] as usize;
            let values_end = state.offsets[patch + 1] as usize;
            let mut pixel = PixelState::default();
            pixel.histogram.copy_from_slice(
                &state.histogram[histogram_start..histogram_start + HISTOGRAM_BINS],
            );
            for value in &state.values[values_start..values_end] {
                pixel.history[pixel.len] = *value;
                pixel.len += 1;
            }
            pixels.push(pixel);
        }
        Ok(Self {
            quantization,
            pixels,
        })
    }

    fn state<D: PisDirection>(&self) -> MedianState<D> {
        debug_assert_eq!(self.quantization, Quantization::selected(D::DIRECTION));
        let mut histogram = Vec::with_capacity(PATCHES * HISTOGRAM_BINS);
        let mut offsets = Vec::with_capacity(PATCHES + 1);
        let mut values = Vec::with_capacity(PATCHES * HISTORY_CAPACITY);
        offsets.push(0);
        for pixel in &self.pixels {
            histogram.extend_from_slice(&pixel.histogram);
            for index in 0..pixel.len {
                values.push(pixel.history[(pixel.head + index) % HISTORY_CAPACITY]);
            }
            offsets.push(values.len() as u32);
        }
        MedianState {
            histogram,
            offsets,
            values,
            direction: PhantomData,
        }
    }
}

/// Finest-level patch components after the temporal median and before native
/// densification.
///
/// `D` retains the PIS solver direction. There is deliberately no conversion
/// back to [`UnfilteredPatchGrid`], so downstream code can require this type
/// without permitting a second temporal submission.
#[derive(Debug, PartialEq)]
pub struct FilteredPatchGrid<D: PisDirection> {
    dcol: Vec<f32>,
    drow: Vec<f32>,
    direction: PhantomData<D>,
}

impl<D: PisDirection> FilteredPatchGrid<D> {
    fn from_components(dcol: Vec<f32>, drow: Vec<f32>) -> Self {
        debug_assert_eq!(dcol.len(), PATCHES);
        debug_assert_eq!(drow.len(), PATCHES);
        Self {
            dcol,
            drow,
            direction: PhantomData,
        }
    }

    pub fn dcol(&self) -> &[f32] {
        &self.dcol
    }

    pub fn drow(&self) -> &[f32] {
        &self.drow
    }

    pub const fn direction(&self) -> Direction {
        D::DIRECTION
    }

    /// Consume the filtered grid into row-major component planes.
    pub fn into_row_major_components(self) -> (Vec<f32>, Vec<f32>) {
        (self.dcol, self.drow)
    }
}

/// An A-to-B grid that has passed through [`AtoBMedian`].
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::temporal_median::{
///     AtoBMedian, FilteredAtoBPatchGrid,
/// };
///
/// fn resubmit(filter: &mut AtoBMedian, filtered: FilteredAtoBPatchGrid) {
///     let _ = filter.run(filtered); // `run` requires an unfiltered PIS grid.
/// }
/// ```
pub type FilteredAtoBPatchGrid = FilteredPatchGrid<AtoB>;

/// A B-to-A grid that has passed through [`BtoAMedian`].
pub type FilteredBtoAPatchGrid = FilteredPatchGrid<BtoA>;

/// Both direction-specific PIS results, still unfiltered and sparse.
#[derive(Debug, PartialEq)]
pub struct DirectedPatchGrids {
    a_to_b: UnfilteredPatchGrid<AtoB>,
    b_to_a: UnfilteredPatchGrid<BtoA>,
}

impl DirectedPatchGrids {
    pub const fn new(a_to_b: UnfilteredPatchGrid<AtoB>, b_to_a: UnfilteredPatchGrid<BtoA>) -> Self {
        Self { a_to_b, b_to_a }
    }

    pub const fn a_to_b(&self) -> &UnfilteredPatchGrid<AtoB> {
        &self.a_to_b
    }

    pub const fn b_to_a(&self) -> &UnfilteredPatchGrid<BtoA> {
        &self.b_to_a
    }
}

/// Both finest-level direction-specific results after temporal filtering.
#[derive(Debug, PartialEq)]
pub struct FilteredDirectedPatchGrids {
    a_to_b: FilteredAtoBPatchGrid,
    b_to_a: FilteredBtoAPatchGrid,
}

impl FilteredDirectedPatchGrids {
    pub const fn a_to_b(&self) -> &FilteredAtoBPatchGrid {
        &self.a_to_b
    }

    pub const fn b_to_a(&self) -> &FilteredBtoAPatchGrid {
        &self.b_to_a
    }

    pub fn into_parts(self) -> (FilteredAtoBPatchGrid, FilteredBtoAPatchGrid) {
        (self.a_to_b, self.b_to_a)
    }
}

/// A solver result was not the finest level selected for temporal filtering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WrongLevel {
    direction: Direction,
    actual: Level,
}

impl fmt::Display for WrongLevel {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "ONE X2 {} temporal median requires level 1, got {}",
            self.direction, self.actual,
        )
    }
}

impl Error for WrongLevel {}

fn require_finest<D: PisDirection>(patches: &UnfilteredPatchGrid<D>) -> Result<(), WrongLevel> {
    if patches.level() == Level::One {
        Ok(())
    } else {
        Err(WrongLevel {
            direction: D::DIRECTION,
            actual: patches.level(),
        })
    }
}

/// The selected A-to-B filter. Its negative scale makes the two-sample lower
/// bin the numerically higher displacement.
pub struct AtoBMedian(DirectionalMedian);

impl Default for AtoBMedian {
    fn default() -> Self {
        Self::new()
    }
}

impl AtoBMedian {
    pub fn new() -> Self {
        Self(DirectionalMedian::selected(Direction::AtoB))
    }

    /// Restore a validated A-to-B FIFO and histogram state.
    pub fn from_state(state: MedianState<AtoB>) -> Result<Self, InvalidMedianState> {
        Ok(Self(DirectionalMedian::from_state(state)?))
    }

    /// Export this direction in canonical patch-major and FIFO order.
    pub fn state(&self) -> MedianState<AtoB> {
        self.0.state()
    }

    /// Consume and filter one fresh native solve result.
    ///
    /// The input and output types make a second submission ill-typed. Level
    /// two is rejected before any per-pixel history changes.
    ///
    /// ```compile_fail
    /// use kjerag_render::flow::one_xs::pis::{AtoB, PatchGrid};
    /// use kjerag_render::flow::one_xs::temporal_median::AtoBMedian;
    ///
    /// fn submit_twice(filter: &mut AtoBMedian, patches: PatchGrid<AtoB>) {
    ///     let _filtered = filter.run(patches).unwrap();
    ///     let _ = filter.run(patches); // the unfiltered grid was consumed.
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// use kjerag_render::flow::one_xs::pis::{AtoB, PatchGrid};
    ///
    /// fn duplicate(patches: PatchGrid<AtoB>) {
    ///     let _copy: PatchGrid<AtoB> = patches.clone();
    /// }
    /// ```
    pub fn run(
        &mut self,
        patches: UnfilteredPatchGrid<AtoB>,
    ) -> Result<FilteredAtoBPatchGrid, WrongLevel> {
        require_finest(&patches)?;
        Ok(self.0.run_finest(patches))
    }

    /// Clear this direction as native object destruction/reconstruction does.
    pub fn clear_for_reconstruction(&mut self) {
        self.0.clear();
    }
}

/// The selected B-to-A filter. Its positive scale preserves numeric order.
pub struct BtoAMedian(DirectionalMedian);

impl Default for BtoAMedian {
    fn default() -> Self {
        Self::new()
    }
}

impl BtoAMedian {
    pub fn new() -> Self {
        Self(DirectionalMedian::selected(Direction::BtoA))
    }

    /// Restore a validated B-to-A FIFO and histogram state.
    pub fn from_state(state: MedianState<BtoA>) -> Result<Self, InvalidMedianState> {
        Ok(Self(DirectionalMedian::from_state(state)?))
    }

    /// Export this direction in canonical patch-major and FIFO order.
    pub fn state(&self) -> MedianState<BtoA> {
        self.0.state()
    }

    /// Consume and filter one fresh native solve result.
    ///
    /// The input and output types make a second submission ill-typed. Level
    /// two is rejected before any per-pixel history changes.
    pub fn run(
        &mut self,
        patches: UnfilteredPatchGrid<BtoA>,
    ) -> Result<FilteredBtoAPatchGrid, WrongLevel> {
        require_finest(&patches)?;
        Ok(self.0.run_finest(patches))
    }

    /// Clear this direction as native object destruction/reconstruction does.
    pub fn clear_for_reconstruction(&mut self) {
        self.0.clear();
    }
}

/// The two independently owned selected directional filters.
#[derive(Default)]
pub struct TemporalMedians {
    a_to_b: AtoBMedian,
    b_to_a: BtoAMedian,
}

impl TemporalMedians {
    pub fn new() -> Self {
        Self::default()
    }

    /// Restore both validated directional histories as one owned pair.
    ///
    /// ```compile_fail
    /// use kjerag_render::flow::one_xs::pis::{AtoB, BtoA};
    /// use kjerag_render::flow::one_xs::temporal_median::{MedianState, TemporalMedians};
    ///
    /// fn swapped(a_to_b: MedianState<AtoB>, b_to_a: MedianState<BtoA>) {
    ///     let _ = TemporalMedians::from_states(b_to_a, a_to_b);
    /// }
    /// ```
    pub fn from_states(
        a_to_b: MedianState<AtoB>,
        b_to_a: MedianState<BtoA>,
    ) -> Result<Self, InvalidMedianState> {
        let a_to_b = AtoBMedian::from_state(a_to_b)?;
        let b_to_a = BtoAMedian::from_state(b_to_a)?;
        Ok(Self { a_to_b, b_to_a })
    }

    /// Consume the paired filters into their exact serializable state.
    pub(super) fn into_states(self) -> (MedianState<AtoB>, MedianState<BtoA>) {
        (self.a_to_b.state(), self.b_to_a.state())
    }

    /// Consume and filter both fresh directional solve results exactly once.
    ///
    /// Both levels are checked before either direction mutates history.
    pub fn run(
        &mut self,
        patches: DirectedPatchGrids,
    ) -> Result<FilteredDirectedPatchGrids, WrongLevel> {
        require_finest(&patches.a_to_b)?;
        require_finest(&patches.b_to_a)?;
        Ok(FilteredDirectedPatchGrids {
            a_to_b: self.a_to_b.0.run_finest(patches.a_to_b),
            b_to_a: self.b_to_a.0.run_finest(patches.b_to_a),
        })
    }

    /// Split ownership so callers may run the two native directions in
    /// parallel without sharing temporal state.
    pub fn split_mut(&mut self) -> (&mut AtoBMedian, &mut BtoAMedian) {
        (&mut self.a_to_b, &mut self.b_to_a)
    }

    /// Clear both histories as replacing both native FDS objects does.
    pub fn clear_for_reconstruction(&mut self) {
        self.a_to_b.clear_for_reconstruction();
        self.b_to_a.clear_for_reconstruction();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_to_b(value: f32, drow: f32) -> UnfilteredPatchGrid<AtoB> {
        UnfilteredPatchGrid::from_row_major_components(
            Level::One,
            vec![value; PATCHES],
            vec![drow; PATCHES],
        )
        .unwrap()
    }

    fn b_to_a(value: f32, drow: f32) -> UnfilteredPatchGrid<BtoA> {
        UnfilteredPatchGrid::from_row_major_components(
            Level::One,
            vec![value; PATCHES],
            vec![drow; PATCHES],
        )
        .unwrap()
    }

    fn in_bin(quantization: Quantization, bin: usize) -> f32 {
        let interior = (bin as f32 + 0.5) / quantization.scale;
        let value = quantization.base + interior;
        assert_eq!(quantization.bin(value), bin as i32);
        value
    }

    fn patch_index(row: usize, col: usize) -> usize {
        assert!(row < PATCH_ROWS);
        assert!(col < PATCH_COLS);
        row * PATCH_COLS + col
    }

    #[test]
    fn selected_constructor_math_has_native_bits_and_patch_shape() {
        assert_eq!((FINEST_ROWS, FINEST_COLS), (540, 30));
        assert_eq!((PATCH_ROWS, PATCH_COLS, PATCHES), (178, 8, 1424));
        assert_eq!(HISTORY_CAPACITY, 3);

        let a_to_b = Quantization::selected(Direction::AtoB);
        assert_eq!(a_to_b.base.to_bits(), 0x3f7f_fc66);
        assert_eq!(a_to_b.scale.to_bits(), 0xc120_0000);
        assert_eq!(a_to_b.bins, HISTOGRAM_BINS);

        let b_to_a = Quantization::selected(Direction::BtoA);
        assert_eq!(b_to_a.base.to_bits(), 0xbf7f_fc66);
        assert_eq!(b_to_a.scale.to_bits(), 0x4120_0000);
        assert_eq!(b_to_a.bins, HISTOGRAM_BINS);
        assert_eq!(HISTOGRAM_BINS, 159);
    }

    #[test]
    fn one_two_three_samples_filter_immediately_with_directional_even_rule() {
        let mut a_filter = AtoBMedian::default();
        let mut b_filter = BtoAMedian::default();

        let a_first = a_filter.run(a_to_b(-1.0, 101.0)).unwrap();
        let b_first = b_filter.run(b_to_a(-1.0, 201.0)).unwrap();
        assert_eq!(a_first.dcol()[0].to_bits(), 0xbf66_6a00); // bin 19
        assert_eq!(b_first.dcol()[0].to_bits(), 0xbf7f_fc66); // bin 0
        assert!(a_first.drow().iter().all(|value| *value == 101.0));
        assert!(b_first.drow().iter().all(|value| *value == 201.0));

        let a_second = a_filter.run(a_to_b(1.0, 102.0)).unwrap();
        let b_second = b_filter.run(b_to_a(1.0, 202.0)).unwrap();
        // Rank one selects the lowest bin. Negative A-to-B scale reverses
        // numeric order; positive B-to-A scale does not. Neither averages.
        assert_eq!(a_second.dcol()[0].to_bits(), 0x3f7f_fc66); // bin 0
        assert_eq!(b_second.dcol()[0].to_bits(), 0xbf7f_fc66); // bin 0

        let a_third = a_filter.run(a_to_b(0.0, 103.0)).unwrap();
        let b_third = b_filter.run(b_to_a(0.0, 203.0)).unwrap();
        assert_eq!(a_third.dcol()[0].to_bits(), 0x3dcc_b000); // bin 9
        assert_eq!(b_third.dcol()[0].to_bits(), 0xbdcc_b000); // bin 9
        assert!(a_third.drow().iter().all(|value| *value == 103.0));
        assert!(b_third.drow().iter().all(|value| *value == 203.0));
    }

    #[test]
    fn every_row_major_patch_keeps_independent_history() {
        let quantization = Quantization::selected(Direction::BtoA);
        let first_bins = (0..PATCH_ROWS)
            .flat_map(|row| (0..PATCH_COLS).map(move |col| (row * 17 + col * 23) % HISTOGRAM_BINS))
            .collect::<Vec<_>>();
        let drow = (0..PATCHES).map(|index| index as f32).collect::<Vec<_>>();
        let first = UnfilteredPatchGrid::<BtoA>::from_row_major_components(
            Level::One,
            first_bins
                .iter()
                .map(|bin| in_bin(quantization, *bin))
                .collect(),
            drow.clone(),
        )
        .unwrap();
        let mut filter = BtoAMedian::default();
        let first = filter.run(first).unwrap();

        for row in 0..PATCH_ROWS {
            for col in 0..PATCH_COLS {
                let index = patch_index(row, col);
                assert_eq!(
                    first.dcol()[index].to_bits(),
                    quantization.value(first_bins[index]).to_bits(),
                    "first sample changed patch ({row},{col})",
                );
            }
        }
        assert_eq!(first.drow(), drow);

        let second_bins = first_bins
            .iter()
            .enumerate()
            .map(|(index, first)| (HISTOGRAM_BINS - 1 - first + index % 7) % HISTOGRAM_BINS)
            .collect::<Vec<_>>();
        let second = UnfilteredPatchGrid::<BtoA>::from_row_major_components(
            Level::One,
            second_bins
                .iter()
                .map(|bin| in_bin(quantization, *bin))
                .collect(),
            drow.clone(),
        )
        .unwrap();
        let second = filter.run(second).unwrap();

        for row in 0..PATCH_ROWS {
            for col in 0..PATCH_COLS {
                let index = patch_index(row, col);
                let lower_bin = first_bins[index].min(second_bins[index]);
                assert_eq!(
                    second.dcol()[index].to_bits(),
                    quantization.value(lower_bin).to_bits(),
                    "history crossed patch ({row},{col})",
                );
            }
        }
        assert_eq!(second.drow(), drow);
    }

    #[test]
    fn values_tied_in_one_bin_reconstruct_the_anchor_not_their_average() {
        let quantization = Quantization::selected(Direction::BtoA);
        let mut filter = BtoAMedian::default();
        let (low, high) = (quantization.base + 1.91, quantization.base + 1.99);
        assert_eq!(quantization.bin(low), 19);
        assert_eq!(quantization.bin(high), 19);
        let _first = filter.run(b_to_a(low, 7.0)).unwrap();
        let second = filter.run(b_to_a(high, 8.0)).unwrap();

        assert_eq!(second.dcol()[0].to_bits(), quantization.value(19).to_bits(),);
        assert_ne!(second.dcol()[0], (low + high) / 2.0);
        assert!(second.drow().iter().all(|value| *value == 8.0));
    }

    #[test]
    fn out_of_range_values_are_unchanged_and_do_not_advance_history() {
        let quantization = Quantization::selected(Direction::BtoA);
        let mut filter = BtoAMedian::default();
        for bin in [10, 20, 30] {
            let _ = filter
                .run(b_to_a(in_bin(quantization, bin), bin as f32))
                .unwrap();
        }

        let low = quantization.base - 1.0;
        assert!(quantization.bin(low) < 0);
        let below = filter.run(b_to_a(low, 401.0)).unwrap();
        assert_eq!(below.dcol()[0].to_bits(), low.to_bits());
        assert!(below.drow().iter().all(|value| *value == 401.0));

        let high = in_bin(quantization, HISTOGRAM_BINS);
        assert_eq!(quantization.bin(high), HISTOGRAM_BINS as i32);
        let above = filter.run(b_to_a(high, 402.0)).unwrap();
        assert_eq!(above.dcol()[0].to_bits(), high.to_bits());

        // Both rejected calls left [10,20,30] intact. Accepting 40 evicts 10,
        // so the three-sample median is bin 30.
        let accepted = filter.run(b_to_a(in_bin(quantization, 40), 403.0)).unwrap();
        assert_eq!(
            accepted.dcol()[0].to_bits(),
            quantization.value(30).to_bits(),
        );
        assert!(accepted.drow().iter().all(|value| *value == 403.0));
    }

    #[test]
    fn fcvtzs_nonfinite_and_fractional_boundaries_match_native() {
        let b_quantization = Quantization::selected(Direction::BtoA);
        let mut b_filter = BtoAMedian::default();

        // AArch64 FCVTZS maps NaN to zero. Native therefore admits the raw
        // NaN into history as bin zero and reconstructs the finite base.
        let nan = b_filter.run(b_to_a(f32::NAN, 1.0)).unwrap();
        assert!(
            nan.dcol()
                .iter()
                .all(|value| value.to_bits() == b_quantization.base.to_bits())
        );

        // Both infinities saturate outside the selected range and are total no-ops.
        let positive_infinity = b_filter.run(b_to_a(f32::INFINITY, 2.0)).unwrap();
        assert!(
            positive_infinity
                .dcol()
                .iter()
                .all(|value| *value == f32::INFINITY)
        );
        let negative_infinity = b_filter.run(b_to_a(f32::NEG_INFINITY, 3.0)).unwrap();
        assert!(
            negative_infinity
                .dcol()
                .iter()
                .all(|value| *value == f32::NEG_INFINITY)
        );

        // The rejected calls did not advance history: [NaN/bin0, bin10]
        // still has rank one, and adding bin20 then selects bin10.
        let second = b_filter
            .run(b_to_a(in_bin(b_quantization, 10), 4.0))
            .unwrap();
        assert!(
            second
                .dcol()
                .iter()
                .all(|value| value.to_bits() == b_quantization.value(0).to_bits())
        );
        let third = b_filter
            .run(b_to_a(in_bin(b_quantization, 20), 5.0))
            .unwrap();
        assert!(
            third
                .dcol()
                .iter()
                .all(|value| value.to_bits() == b_quantization.value(10).to_bits())
        );

        // Truncation toward zero admits a negative fractional q as bin zero;
        // the last positive bin remains 158.
        let mut fractional_filter = BtoAMedian::default();
        let fractional_input = b_to_a(b_quantization.base - 0.05, 6.0);
        assert_eq!(
            b_quantization.bin(fractional_input.patch(0, 0).flow().dcol()),
            0
        );
        let fractional = fractional_filter.run(fractional_input).unwrap();
        assert_eq!(
            fractional.dcol()[0].to_bits(),
            b_quantization.value(0).to_bits()
        );
        let mut last_bin_filter = BtoAMedian::default();
        let last_bin = last_bin_filter
            .run(b_to_a(in_bin(b_quantization, HISTOGRAM_BINS - 1), 7.0))
            .unwrap();
        assert_eq!(
            last_bin.dcol()[0].to_bits(),
            b_quantization.value(HISTOGRAM_BINS - 1).to_bits()
        );

        // Negative A-to-B scale swaps infinity signs before FCVTZS, but both
        // still reject; NaN still enters bin zero.
        let a_quantization = Quantization::selected(Direction::AtoB);
        let mut a_filter = AtoBMedian::default();
        let a_nan = a_filter.run(a_to_b(f32::NAN, 8.0)).unwrap();
        assert_eq!(a_nan.dcol()[0].to_bits(), a_quantization.value(0).to_bits());
        let a_positive_infinity = a_filter.run(a_to_b(f32::INFINITY, 9.0)).unwrap();
        assert_eq!(a_positive_infinity.dcol()[0], f32::INFINITY);
        let a_negative_infinity = a_filter.run(a_to_b(f32::NEG_INFINITY, 10.0)).unwrap();
        assert_eq!(a_negative_infinity.dcol()[0], f32::NEG_INFINITY);
    }

    #[test]
    fn full_ring_wrap_and_reconstruction_clear_cover_a_nonzero_patch() {
        let quantization = Quantization::selected(Direction::BtoA);
        let target = patch_index(PATCH_ROWS / 2, PATCH_COLS - 2);
        let mut filter = BtoAMedian::default();

        // Three fills followed by four replacements visit heads 0, 1, 2,
        // wrap to 0, and also exercise decrementing a duplicate-bin count.
        for (bin, expected_bin) in [
            (10, 10),
            (10, 10),
            (30, 10),
            (40, 30),
            (5, 30),
            (50, 40),
            (1, 5),
        ] {
            let mut dcol = vec![in_bin(quantization, 100); PATCHES];
            dcol[target] = in_bin(quantization, bin);
            let patches = UnfilteredPatchGrid::<BtoA>::from_row_major_components(
                Level::One,
                dcol,
                vec![77.0; PATCHES],
            )
            .unwrap();
            let patches = filter.run(patches).unwrap();
            assert_eq!(
                patches.dcol()[target].to_bits(),
                quantization.value(expected_bin).to_bits(),
            );
            assert_eq!(patches.drow()[target], 77.0);
        }

        filter.clear_for_reconstruction();
        let mut dcol = vec![in_bin(quantization, 100); PATCHES];
        dcol[target] = in_bin(quantization, 70);
        let after_clear = UnfilteredPatchGrid::<BtoA>::from_row_major_components(
            Level::One,
            dcol,
            vec![88.0; PATCHES],
        )
        .unwrap();
        let after_clear = filter.run(after_clear).unwrap();
        assert_eq!(
            after_clear.dcol()[target].to_bits(),
            quantization.value(70).to_bits(),
        );
        assert_eq!(after_clear.drow()[target], 88.0);
    }

    #[test]
    fn direction_histories_and_reconstruction_clears_are_independent() {
        let aq = Quantization::selected(Direction::AtoB);
        let bq = Quantization::selected(Direction::BtoA);
        let mut filters = TemporalMedians::default();

        for (a_bin, b_bin) in [(10, 20), (30, 40)] {
            let patches = DirectedPatchGrids::new(
                a_to_b(in_bin(aq, a_bin), 1.0),
                b_to_a(in_bin(bq, b_bin), 2.0),
            );
            let _ = filters.run(patches).unwrap();
        }

        // Destroy/reconstruct only A-to-B. B-to-A must retain [20,40].
        filters.split_mut().0.clear_for_reconstruction();
        let after_one_reset =
            DirectedPatchGrids::new(a_to_b(in_bin(aq, 50), 3.0), b_to_a(in_bin(bq, 0), 4.0));
        let after_one_reset = filters.run(after_one_reset).unwrap();
        assert_eq!(
            after_one_reset.a_to_b().dcol()[0].to_bits(),
            aq.value(50).to_bits(),
        );
        assert_eq!(
            after_one_reset.b_to_a().dcol()[0].to_bits(),
            bq.value(20).to_bits(),
        );

        filters.clear_for_reconstruction();
        let after_both_reset =
            DirectedPatchGrids::new(a_to_b(in_bin(aq, 60), 5.0), b_to_a(in_bin(bq, 60), 6.0));
        let after_both_reset = filters.run(after_both_reset).unwrap();
        assert_eq!(after_both_reset.a_to_b().dcol()[0], aq.value(60),);
        assert_eq!(after_both_reset.b_to_a().dcol()[0], bq.value(60),);
    }

    #[test]
    fn paired_level_check_is_atomic_before_either_history_changes() {
        let aq = Quantization::selected(Direction::AtoB);
        let bq = Quantization::selected(Direction::BtoA);
        let mut filters = TemporalMedians::default();
        let coarse_b = UnfilteredPatchGrid::<BtoA>::from_row_major_components(
            Level::Two,
            vec![in_bin(bq, 20); Level::Two.patches()],
            vec![0.0; Level::Two.patches()],
        )
        .unwrap();
        let error = filters
            .run(DirectedPatchGrids::new(
                a_to_b(in_bin(aq, 10), 1.0),
                coarse_b,
            ))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ONE X2 B-to-A temporal median requires level 1, got level 2",
        );

        let filtered = filters
            .run(DirectedPatchGrids::new(
                a_to_b(in_bin(aq, 30), 2.0),
                b_to_a(in_bin(bq, 40), 3.0),
            ))
            .unwrap();
        assert_eq!(filtered.a_to_b().dcol()[0], aq.value(30));
        assert_eq!(filtered.b_to_a().dcol()[0], bq.value(40));
    }

    #[test]
    fn pis_shape_is_checked_before_temporal_handoff() {
        let error = UnfilteredPatchGrid::<AtoB>::from_row_major_components(
            Level::One,
            vec![0.0; PATCHES - 1],
            vec![0.0; PATCHES],
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ONE X2 PIS level 1 has 1423 dcol patch values, expected 1424",
        );
    }

    #[test]
    fn semantic_state_roundtrips_wrapped_fifo_and_continues_identically() {
        let quantization = Quantization::selected(Direction::BtoA);
        let mut original = BtoAMedian::new();
        for bin in [10, 20, 30, 40] {
            let _ = original
                .run(b_to_a(in_bin(quantization, bin), 0.0))
                .unwrap();
        }

        let state = original.state();
        assert_eq!(state.histogram().len(), PATCHES * HISTOGRAM_BINS);
        assert_eq!(state.offsets().len(), PATCHES + 1);
        assert_eq!(state.values().len(), PATCHES * HISTORY_CAPACITY);
        assert_eq!(
            state.values()[..HISTORY_CAPACITY]
                .iter()
                .map(|value| quantization.bin(*value))
                .collect::<Vec<_>>(),
            vec![20, 30, 40],
        );

        let (histogram, offsets, values) = state.into_parts();
        let imported = MedianState::<BtoA>::from_parts(histogram, offsets, values).unwrap();
        let mut restored = BtoAMedian::from_state(imported).unwrap();
        assert_eq!(original.state(), restored.state());

        let original_next = original.run(b_to_a(in_bin(quantization, 5), 11.0)).unwrap();
        let restored_next = restored.run(b_to_a(in_bin(quantization, 5), 11.0)).unwrap();
        assert_eq!(original_next, restored_next);
        assert_eq!(original.state(), restored.state());
    }

    #[test]
    fn paired_states_restore_directional_histories_and_continue_independently() {
        let aq = Quantization::selected(Direction::AtoB);
        let bq = Quantization::selected(Direction::BtoA);
        let mut original = TemporalMedians::new();
        for (ab_bin, ba_bin) in [(12, 82), (24, 70), (36, 58), (48, 46)] {
            original
                .run(DirectedPatchGrids::new(
                    a_to_b(in_bin(aq, ab_bin), 1.0),
                    b_to_a(in_bin(bq, ba_bin), -1.0),
                ))
                .unwrap();
        }

        let (ab_state, ba_state) = {
            let (ab, ba) = original.split_mut();
            (ab.state(), ba.state())
        };
        let mut restored =
            TemporalMedians::from_states(ab_state.clone(), ba_state.clone()).unwrap();
        let (restored_ab, restored_ba) = restored.split_mut();
        assert_eq!(restored_ab.state(), ab_state);
        assert_eq!(restored_ba.state(), ba_state);

        let next =
            DirectedPatchGrids::new(a_to_b(in_bin(aq, 5), 2.0), b_to_a(in_bin(bq, 99), -2.0));
        let restored_next = restored.run(next).unwrap();
        let original_next = original
            .run(DirectedPatchGrids::new(
                a_to_b(in_bin(aq, 5), 2.0),
                b_to_a(in_bin(bq, 99), -2.0),
            ))
            .unwrap();
        assert_eq!(restored_next, original_next);
    }

    #[test]
    #[ignore = "requires KJERAG_ONE_XS_WARM_PAYLOAD with the authenticated run-06 payload"]
    fn accepted_run06_semantic_state_and_outputs_are_bit_exact() {
        use sha2::{Digest as _, Sha256};

        const PAYLOAD_SHA256: &str =
            "a5311370d7946d45094cc85c8ff6631bf2b1195e8c127e3b1f909310168ce8c1";
        const HISTOGRAM_BYTES: usize = PATCHES * HISTOGRAM_BINS;
        const OFFSET_BYTES: usize = (PATCHES + 1) * size_of::<u32>();
        const VALUE_BYTES: usize = PATCHES * HISTORY_CAPACITY * size_of::<f32>();
        const PATCH_BYTES: usize = PATCHES * size_of::<f32>();

        let path = std::env::var_os("KJERAG_ONE_XS_WARM_PAYLOAD")
            .expect("set KJERAG_ONE_XS_WARM_PAYLOAD to authenticated run-06 warm-pair-payload.bin");
        let payload = std::fs::read(path).unwrap();
        assert_eq!(sha256_hex(&payload), PAYLOAD_SHA256);
        assert_eq!(&payload[..8], b"KJWP602\x04");

        let ab_pre = MedianState::<AtoB>::from_parts(
            payload[6_121_162..6_121_162 + HISTOGRAM_BYTES].to_vec(),
            read_u32(&payload, 6_347_627, OFFSET_BYTES),
            read_f32(&payload, 6_353_375, VALUE_BYTES),
        )
        .unwrap();
        let mut ab = AtoBMedian::from_state(ab_pre).unwrap();
        let ab_input_u = read_f32(&payload, 2_093_234, PATCH_BYTES);
        let ab_input_v = read_f32(&payload, 2_098_965, PATCH_BYTES);
        let ab_filtered = ab
            .run(
                UnfilteredPatchGrid::from_row_major_components(
                    Level::One,
                    ab_input_u,
                    ab_input_v.clone(),
                )
                .unwrap(),
            )
            .unwrap();
        assert_f32_bits(
            ab_filtered.dcol(),
            &read_f32(&payload, 2_266_857, PATCH_BYTES),
        );
        assert_f32_bits(ab_filtered.drow(), &ab_input_v);
        assert_state(
            ab.state(),
            &payload[6_379_230..6_379_230 + HISTOGRAM_BYTES],
            &read_u32(&payload, 6_605_696, OFFSET_BYTES),
            &read_f32(&payload, 6_611_445, VALUE_BYTES),
        );

        let ba_pre = MedianState::<BtoA>::from_parts(
            payload[6_722_579..6_722_579 + HISTOGRAM_BYTES].to_vec(),
            read_u32(&payload, 6_949_044, OFFSET_BYTES),
            read_f32(&payload, 6_954_792, VALUE_BYTES),
        )
        .unwrap();
        let mut ba = BtoAMedian::from_state(ba_pre).unwrap();
        let ba_input_u = read_f32(&payload, 3_264_078, PATCH_BYTES);
        let ba_input_v = read_f32(&payload, 3_269_809, PATCH_BYTES);
        let ba_filtered = ba
            .run(
                UnfilteredPatchGrid::from_row_major_components(
                    Level::One,
                    ba_input_u,
                    ba_input_v.clone(),
                )
                .unwrap(),
            )
            .unwrap();
        assert_f32_bits(
            ba_filtered.dcol(),
            &read_f32(&payload, 3_437_701, PATCH_BYTES),
        );
        assert_f32_bits(ba_filtered.drow(), &ba_input_v);
        assert_state(
            ba.state(),
            &payload[6_980_647..6_980_647 + HISTOGRAM_BYTES],
            &read_u32(&payload, 7_207_113, OFFSET_BYTES),
            &read_f32(&payload, 7_212_862, VALUE_BYTES),
        );

        fn assert_state<D: PisDirection>(
            actual: MedianState<D>,
            expected_histogram: &[u8],
            expected_offsets: &[u32],
            expected_values: &[f32],
        ) {
            assert_eq!(actual.histogram(), expected_histogram);
            assert_eq!(actual.offsets(), expected_offsets);
            assert_f32_bits(actual.values(), expected_values);
        }

        fn read_u32(payload: &[u8], offset: usize, bytes: usize) -> Vec<u32> {
            payload[offset..offset + bytes]
                .chunks_exact(4)
                .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
                .collect()
        }

        fn read_f32(payload: &[u8], offset: usize, bytes: usize) -> Vec<f32> {
            payload[offset..offset + bytes]
                .chunks_exact(4)
                .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
                .collect()
        }

        fn assert_f32_bits(actual: &[f32], expected: &[f32]) {
            assert_eq!(actual.len(), expected.len());
            for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
                assert_eq!(actual.to_bits(), expected.to_bits(), "value {index}");
            }
        }

        fn sha256_hex(bytes: &[u8]) -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }
    }

    #[test]
    fn semantic_state_rejects_malformed_shapes_offsets_values_and_histogram() {
        fn rejected(histogram: Vec<u8>, offsets: Vec<u32>, values: Vec<f32>, expected: &str) {
            let error = MedianState::<BtoA>::from_parts(histogram, offsets, values).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "unexpected error: {error}",
            );
        }

        let empty_histogram = vec![0; PATCHES * HISTOGRAM_BINS];
        let empty_offsets = vec![0; PATCHES + 1];
        rejected(
            vec![0; PATCHES * HISTOGRAM_BINS - 1],
            empty_offsets.clone(),
            Vec::new(),
            "histogram bytes",
        );
        rejected(
            empty_histogram.clone(),
            vec![0; PATCHES],
            Vec::new(),
            "offsets, expected",
        );

        let mut nonzero_start = empty_offsets.clone();
        nonzero_start[0] = 1;
        rejected(
            empty_histogram.clone(),
            nonzero_start,
            Vec::new(),
            "start at zero",
        );

        let mut nonmonotonic = empty_offsets.clone();
        nonmonotonic[1] = 1;
        rejected(
            empty_histogram.clone(),
            nonmonotonic,
            vec![in_bin(Quantization::selected(Direction::BtoA), 0)],
            "not monotonic",
        );

        let mut out_of_bounds = empty_offsets.clone();
        out_of_bounds[1] = 1;
        rejected(
            empty_histogram.clone(),
            out_of_bounds,
            Vec::new(),
            "out of bounds",
        );

        let mut too_many = vec![4; PATCHES + 1];
        too_many[0] = 0;
        rejected(
            empty_histogram.clone(),
            too_many,
            vec![in_bin(Quantization::selected(Direction::BtoA), 0); 4],
            "expected at most 3",
        );

        rejected(
            empty_histogram.clone(),
            empty_offsets.clone(),
            vec![in_bin(Quantization::selected(Direction::BtoA), 0)],
            "ends at offset 0, expected 1",
        );

        let mut one_value_offsets = vec![1; PATCHES + 1];
        one_value_offsets[0] = 0;
        rejected(
            empty_histogram.clone(),
            one_value_offsets,
            vec![in_bin(
                Quantization::selected(Direction::BtoA),
                HISTOGRAM_BINS,
            )],
            "outside the histogram",
        );

        let mut wrong_histogram = empty_histogram;
        wrong_histogram[0] = 1;
        rejected(
            wrong_histogram,
            empty_offsets,
            Vec::new(),
            "histogram does not match",
        );

        let mut one_a_value_offsets = vec![1; PATCHES + 1];
        one_a_value_offsets[0] = 0;
        let b_only_value = in_bin(Quantization::selected(Direction::BtoA), HISTOGRAM_BINS - 1);
        let error = MedianState::<AtoB>::from_parts(
            vec![0; PATCHES * HISTOGRAM_BINS],
            one_a_value_offsets,
            vec![b_only_value],
        )
        .unwrap_err();
        assert!(error.to_string().contains("outside the histogram"));
    }
}
