//! Studio's source-time ISO lookup over [`kjerag_meta::DenoiseIsoTrack`].
//!
//! This is only the metadata consumer. It does not select temporal filtering,
//! provide an ISO when the track is empty, or decide how a player resets on a
//! seek. Callers must explicitly [`Lookup::reset`] or start a
//! [`Lookup::restarted`] run before looking up an earlier source time.

use std::sync::Arc;

use kjerag_meta::{DenoiseIsoObservation, DenoiseIsoTrack};

const EPSILON: f64 = 1.0e-6;
const MINIMUM_VALID_ISO: f64 = 100.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Empty,
    NonIncreasingTime { index: usize },
    NonFiniteQuery,
    TimeReversed,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "ISO observations are empty"),
            Self::NonIncreasingTime { index } => {
                write!(
                    f,
                    "ISO observation {index} does not follow the previous time"
                )
            }
            Self::NonFiniteQuery => write!(f, "ISO lookup time is not finite"),
            Self::TimeReversed => write!(f, "ISO lookup time moved backwards without a reset"),
        }
    }
}

impl std::error::Error for Error {}

/// Stateful source-time lookup preserving Studio's forward-zero cache.
///
/// Construction takes one owned snapshot of the track's observations, so a
/// session can retain this state without borrowing its calibration owner.
/// Lookups do not copy that snapshot.
///
/// The cache belongs to one monotonically advancing source-time run. A seek,
/// loop, epoch replacement, or other backwards discontinuity must call
/// [`Self::reset`]; an accidental backwards lookup is rejected rather than
/// silently carrying a result across that boundary.
pub struct Lookup {
    /// One open-time snapshot keeps session lookup state independent of the
    /// profile or calibration owner. No observation is copied per frame.
    observations: Arc<[DenoiseIsoObservation]>,
    last_query_ms: Option<f64>,
    next_nonzero_iso: f64,
}

impl Lookup {
    pub fn new(track: &DenoiseIsoTrack) -> Result<Self, Error> {
        Self::from_observations(track.observations())
    }

    pub(super) fn from_observations(observations: &[DenoiseIsoObservation]) -> Result<Self, Error> {
        if observations.is_empty() {
            return Err(Error::Empty);
        }
        for (index, pair) in observations.windows(2).enumerate() {
            // Native pairs are binary64. Reject a source whose integer times
            // cease to be strictly ordered after that same conversion, which
            // would leave interpolation with a zero or negative denominator.
            if pair[1].offset_ms as f64 <= pair[0].offset_ms as f64 {
                return Err(Error::NonIncreasingTime { index: index + 1 });
            }
        }
        Ok(Self {
            observations: Arc::from(observations),
            last_query_ms: None,
            next_nonzero_iso: -1.0,
        })
    }

    /// Start an independent forward run over the same immutable observations.
    /// This shares the open-time snapshot but not chronology or zero-cache
    /// state, so a replaced decode epoch cannot affect its predecessor.
    pub fn restarted(&self) -> Self {
        Self {
            observations: Arc::clone(&self.observations),
            last_query_ms: None,
            next_nonzero_iso: -1.0,
        }
    }

    /// Begin a new forward source-time run. This resets both chronology and
    /// the last successful forward nonzero result, matching construction of a
    /// fresh native filter lookup state.
    pub fn reset(&mut self) {
        self.last_query_ms = None;
        self.next_nonzero_iso = -1.0;
    }

    /// Return the ISO passed to the denoise backend, including binary64
    /// interpolation and truncation toward zero to signed integer.
    pub fn iso_at(&mut self, source_time_ms: f64) -> Result<i32, Error> {
        if !source_time_ms.is_finite() {
            return Err(Error::NonFiniteQuery);
        }
        if self
            .last_query_ms
            .is_some_and(|previous| source_time_ms < previous)
        {
            return Err(Error::TimeReversed);
        }

        let (lower, upper) = self.bracket(source_time_ms);
        let zero = (lower..=upper).find(|&index| is_zero(self.iso(index)));
        let mut value = match zero {
            Some(index) => self.find_next_nonzero(index + 1),
            None if lower < upper => {
                let time0 = self.time(lower);
                let fraction = (source_time_ms - time0) / (self.time(upper) - time0);
                (self.iso(upper) - self.iso(lower)).mul_add(fraction, self.iso(lower))
            }
            None => self.iso(upper),
        };

        if value < MINIMUM_VALID_ISO {
            value = self.correct_invalid(source_time_ms);
        }
        self.last_query_ms = Some(source_time_ms);
        Ok(value as i32)
    }

    fn bracket(&self, source_time_ms: f64) -> (usize, usize) {
        let upper = self
            .observations
            .partition_point(|sample| greater(source_time_ms, sample.offset_ms as f64));
        match upper {
            0 => (0, 0),
            index if index == self.observations.len() => (index - 1, index - 1),
            index => (index - 1, index),
        }
    }

    fn find_next_nonzero(&mut self, from: usize) -> f64 {
        if let Some(value) = (from..self.observations.len())
            .map(|index| self.iso(index))
            .find(|&iso| !is_zero(iso))
        {
            self.next_nonzero_iso = value;
        }
        self.next_nonzero_iso
    }

    fn correct_invalid(&self, source_time_ms: f64) -> f64 {
        if self.iso(0) < MINIMUM_VALID_ISO {
            return MINIMUM_VALID_ISO;
        }

        let mut selected = None;
        let mut best_distance = f64::MAX;
        for (index, _) in self.observations.iter().enumerate() {
            let iso = self.iso(index);
            if iso < MINIMUM_VALID_ISO {
                continue;
            }
            let time = self.time(index);
            let distance = (time - source_time_ms).abs();
            if distance < best_distance || (distance == best_distance && time < source_time_ms) {
                selected = Some(iso);
                best_distance = distance;
            }
        }
        selected.unwrap_or(MINIMUM_VALID_ISO)
    }

    fn time(&self, index: usize) -> f64 {
        self.observations[index].offset_ms as f64
    }

    fn iso(&self, index: usize) -> f64 {
        self.observations[index].iso as f64
    }
}

fn greater(left: f64, right: f64) -> bool {
    left > right && (left - right).abs() >= EPSILON
}

fn is_zero(value: f64) -> bool {
    value.abs() <= EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observations(values: &[(i64, u32)]) -> Vec<DenoiseIsoObservation> {
        values
            .iter()
            .map(|&(offset_ms, iso)| DenoiseIsoObservation { offset_ms, iso })
            .collect()
    }

    #[test]
    fn rejects_empty_or_unordered_input_and_nonfinite_queries() {
        assert!(matches!(
            Lookup::new(&DenoiseIsoTrack::default()),
            Err(Error::Empty)
        ));
        let unordered = observations(&[(1, 100), (1, 200)]);
        assert!(matches!(
            Lookup::from_observations(&unordered),
            Err(Error::NonIncreasingTime { index: 1 })
        ));
        let reversed = observations(&[(2, 100), (1, 200)]);
        assert!(matches!(
            Lookup::from_observations(&reversed),
            Err(Error::NonIncreasingTime { index: 1 })
        ));
        let source = observations(&[(0, 100)]);
        let mut lookup = Lookup::from_observations(&source).unwrap();
        assert_eq!(lookup.iso_at(f64::NAN), Err(Error::NonFiniteQuery));
        assert_eq!(lookup.iso_at(f64::INFINITY), Err(Error::NonFiniteQuery));
        assert_eq!(lookup.iso_at(f64::NEG_INFINITY), Err(Error::NonFiniteQuery));
    }

    #[test]
    fn bracket_uses_the_native_greater_tolerance() {
        let source = observations(&[(0, 100), (10, 200)]);
        let lookup = Lookup::from_observations(&source).unwrap();
        assert_eq!(lookup.bracket(EPSILON / 2.0), (0, 0));
        assert_eq!(lookup.bracket(EPSILON), (0, 1));
    }

    #[test]
    fn lookup_observes_the_exact_greater_tolerance_boundary() {
        let source = observations(&[(0, 100), (10, 2_000_000_100)]);
        let mut lookup = Lookup::from_observations(&source).unwrap();
        assert_eq!(lookup.iso_at(EPSILON / 2.0).unwrap(), 100);
        assert_eq!(lookup.iso_at(EPSILON).unwrap(), 300);
        assert_eq!(lookup.iso_at(EPSILON * 1.5).unwrap(), 400);
    }

    #[test]
    fn endpoints_interpolation_fma_and_integer_truncation_are_explicit() {
        let source = observations(&[(0, 100), (3, 107)]);
        let mut lookup = Lookup::from_observations(&source).unwrap();
        assert_eq!(lookup.iso_at(-1.0).unwrap(), 100);
        assert_eq!(lookup.iso_at(1.0).unwrap(), 102);
        assert_eq!(lookup.iso_at(3.0).unwrap(), 107);
        assert_eq!(lookup.iso_at(30.0).unwrap(), 107);
    }

    #[test]
    fn backwards_time_requires_an_explicit_full_state_reset() {
        let source = observations(&[(0, 100), (10, 200)]);
        let mut lookup = Lookup::from_observations(&source).unwrap();
        assert_eq!(lookup.iso_at(8.0).unwrap(), 180);
        assert_eq!(lookup.iso_at(7.0), Err(Error::TimeReversed));
        lookup.reset();
        assert_eq!(lookup.iso_at(7.0).unwrap(), 170);
    }

    #[test]
    fn zero_uses_forward_search_then_the_last_successful_cache() {
        let source = observations(&[(0, 0), (10, 0), (20, 240), (30, 0)]);
        let mut lookup = Lookup::from_observations(&source).unwrap();
        assert_eq!(lookup.iso_at(5.0).unwrap(), 240);
        assert_eq!(lookup.iso_at(30.0).unwrap(), 240);

        lookup.reset();
        // No successful forward search exists in this new run. Its -1 result
        // reaches invalid correction, whose first-item rule returns 100.
        assert_eq!(lookup.iso_at(30.0).unwrap(), 100);
    }

    #[test]
    fn invalid_values_choose_the_nearest_valid_and_ties_choose_earlier() {
        let source = observations(&[(0, 100), (10, 50), (20, 120)]);
        let mut lookup = Lookup::from_observations(&source).unwrap();
        assert_eq!(lookup.iso_at(10.0).unwrap(), 100);
        assert_eq!(lookup.iso_at(12.0).unwrap(), 120);
    }

    #[test]
    fn an_invalid_first_observation_uses_the_native_minimum_shortcut() {
        let source = observations(&[(0, 50), (10, 150)]);
        let mut lookup = Lookup::from_observations(&source).unwrap();
        assert_eq!(lookup.iso_at(1.0).unwrap(), 100);
    }

    #[test]
    fn restarted_lookup_shares_observations_but_not_run_state() {
        let source = observations(&[(0, 0), (10, 0), (20, 240), (30, 0)]);
        let mut original = Lookup::from_observations(&source).unwrap();
        assert_eq!(original.iso_at(5.0).unwrap(), 240);
        assert_eq!(original.iso_at(30.0).unwrap(), 240);

        let mut restarted = original.restarted();
        assert!(Arc::ptr_eq(&original.observations, &restarted.observations));
        // The restarted run has neither the original's chronology nor its
        // successful forward-nonzero cache.
        assert_eq!(restarted.iso_at(30.0).unwrap(), 100);
        assert_eq!(original.iso_at(5.0), Err(Error::TimeReversed));
        restarted.reset();
        assert_eq!(restarted.iso_at(5.0).unwrap(), 240);
    }

    #[test]
    #[ignore = "reads the owner's private capture files"]
    fn owner_capture_times_match_the_authenticated_iso_values() {
        let cases = [
            (
                "KJERAG_DENOISE_ISO_X4",
                &[(607_574.0, 100), (612_078.0, 100)][..],
            ),
            (
                "KJERAG_DENOISE_ISO_X2",
                &[(212_478.0, 365), (212_512.0, 365)][..],
            ),
        ];
        for (variable, expected) in cases {
            let path = std::env::var(variable).unwrap_or_else(|_| panic!("{variable} is required"));
            let calibration = kjerag_meta::CalibrationSet::from_capture(path).unwrap();
            let mut lookup = Lookup::new(&calibration.denoise_iso).unwrap();
            for &(time, iso) in expected {
                assert_eq!(lookup.iso_at(time).unwrap(), iso, "{variable} at {time}");
            }
        }
    }
}
