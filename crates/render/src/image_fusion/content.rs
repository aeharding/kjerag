//! Selected Windows outer admission and 800x16-to-200x4 area reduction.
//! Source geometry is deliberately outside this byte-image boundary.

use crate::studio_type2::{MAP_HEIGHT, MAP_NODES, MAP_WIDTH};

pub(super) const BAND_WIDTH: usize = 800;
pub(super) const BAND_HEIGHT: usize = 16;
const BAND_BYTES: usize = BAND_WIDTH * BAND_HEIGHT * 3;
const STRIPS: usize = 3;
// The native gate divides the *destination* width, even though it reads
// the 800-wide sources. Columns 198..799 do not participate in admission.
const STRIP_WIDTH: usize = MAP_WIDTH / STRIPS;
type Means = [[[f64; 3]; STRIPS]; 2];

#[derive(Default)]
pub(super) struct Gate {
    baseline: Option<Means>,
}

impl Gate {
    pub(super) fn admit(&mut self, bands: [&[u8]; 2]) -> bool {
        let means = bands.map(means);
        let changed = self.baseline.as_ref().is_none_or(|baseline| {
            means
                .iter()
                .flatten()
                .flatten()
                .zip(baseline.iter().flatten().flatten())
                .any(|(current, old)| (*current - *old).abs() > 3.0)
        });
        if changed {
            // The native predicate commits all six mean vectors before MGP,
            // not only the component which exceeded the threshold.
            self.baseline = Some(means);
        }
        changed
    }
}

pub(super) fn validate(bands: [&[u8]; 2]) -> Result<(), String> {
    for (name, band) in ["left", "right"].into_iter().zip(bands) {
        if band.len() != BAND_BYTES {
            return Err(format!(
                "selected X4 fusion {name} source band has {} bytes, expected {BAND_BYTES}",
                band.len()
            ));
        }
    }
    Ok(())
}

fn means(band: &[u8]) -> [[f64; 3]; STRIPS] {
    debug_assert_eq!(band.len(), BAND_BYTES);
    std::array::from_fn(|strip| {
        let mut sum = [0u32; 3];
        for y in 0..BAND_HEIGHT {
            for x in strip * STRIP_WIDTH..(strip + 1) * STRIP_WIDTH {
                let at = (y * BAND_WIDTH + x) * 3;
                for (channel, sum) in sum.iter_mut().enumerate() {
                    *sum += band[at + channel] as u32;
                }
            }
        }
        sum.map(|sum| sum as f64 * (1.0 / (STRIP_WIDTH * BAND_HEIGHT) as f64))
    })
}

/// Full source band area-resized into the selected working ROI (0,48,200,4).
/// Every output byte is the average of its aligned 4x4 source block, rounded
/// ties-to-even. Unconsumed surrounding rows are zero here; current preparation
/// fills the rows subsequently used by ratio construction from rows 48/51.
pub(super) fn working_chart(band: &[u8]) -> Vec<u8> {
    debug_assert_eq!(band.len(), BAND_BYTES);
    let mut output = vec![0; MAP_NODES * 3];
    for row in 0..4 {
        for col in 0..MAP_WIDTH {
            for channel in 0..3 {
                let mut sum = 0u32;
                for dy in 0..4 {
                    for dx in 0..4 {
                        sum +=
                            band[((row * 4 + dy) * BAND_WIDTH + col * 4 + dx) * 3 + channel] as u32;
                    }
                }
                output[((MAP_HEIGHT / 2 - 2 + row) * MAP_WIDTH + col) * 3 + channel] =
                    (sum as f32 * (1.0 / 16.0)).round_ties_even() as u8;
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(value: u8) -> Vec<u8> {
        vec![value; BAND_BYTES]
    }

    #[test]
    fn gate_compares_to_last_trigger_not_previous_observation() {
        let mut gate = Gate::default();
        assert!(gate.admit([&flat(100), &flat(100)]));
        assert!(!gate.admit([&flat(103), &flat(100)]));
        assert!(gate.admit([&flat(104), &flat(100)]));
        assert!(!gate.admit([&flat(101), &flat(100)]));
        assert!(gate.admit([&flat(100), &flat(100)]));
    }

    #[test]
    fn threshold_is_strict_at_one_code_of_aggregate_evidence() {
        for direction in [-1i16, 1] {
            let mut gate = Gate::default();
            let base = flat(100);
            assert!(gate.admit([&base, &base]));
            let mut changed = base.clone();
            for y in 0..BAND_HEIGHT {
                for x in 0..STRIP_WIDTH {
                    changed[(y * BAND_WIDTH + x) * 3] = (100 + 3 * direction) as u8;
                }
            }
            assert!(!gate.admit([&changed, &base]));
            changed[0] = (i16::from(changed[0]) + direction) as u8;
            assert!(gate.admit([&changed, &base]));
        }
    }

    #[test]
    fn f64_mean_rounding_is_not_an_integer_sum_threshold() {
        let mut gate = Gate::default();
        let other = flat(0);
        let mut first = flat(0);
        first[0] = 9;
        assert!(gate.admit([&first, &other]));
        let mut next = first.clone();
        for y in 0..BAND_HEIGHT {
            for x in 0..STRIP_WIDTH {
                next[(y * BAND_WIDTH + x) * 3] += 3;
            }
        }
        // The integer sum increased by exactly 3*1056. Rounding each binary64
        // mean before subtraction nevertheless puts this delta just above 3.
        assert!(gate.admit([&next, &other]));
    }

    #[test]
    fn a_trigger_updates_the_other_lens_baseline_too() {
        let mut gate = Gate::default();
        assert!(gate.admit([&flat(100), &flat(100)]));
        assert!(gate.admit([&flat(104), &flat(102)]));
        assert!(!gate.admit([&flat(104), &flat(105)]));
    }

    #[test]
    fn each_lens_strip_and_channel_can_trigger_and_commits_all_means() {
        for lens in 0..2 {
            for strip in 0..STRIPS {
                for channel in 0..3 {
                    let mut gate = Gate::default();
                    let mut bands = [flat(100), flat(100)];
                    assert!(gate.admit([&bands[0], &bands[1]]));
                    for y in 0..BAND_HEIGHT {
                        for x in strip * STRIP_WIDTH..(strip + 1) * STRIP_WIDTH {
                            bands[lens][(y * BAND_WIDTH + x) * 3 + channel] = 104;
                        }
                    }
                    assert!(gate.admit([&bands[0], &bands[1]]));
                    assert!(!gate.admit([&bands[0], &bands[1]]));
                }
            }
        }
    }

    #[test]
    fn gate_reads_all_sixteen_rows_but_ignores_columns_after_197() {
        let mut gate = Gate::default();
        let mut bands = [flat(100), flat(100)];
        assert!(gate.admit([&bands[0], &bands[1]]));
        for y in 0..BAND_HEIGHT {
            bands[0][(y * BAND_WIDTH + 198) * 3..(y + 1) * BAND_WIDTH * 3].fill(255);
        }
        assert!(!gate.admit([&bands[0], &bands[1]]));
        bands[1][15 * BAND_WIDTH * 3..(15 * BAND_WIDTH + STRIP_WIDTH) * 3].fill(164);
        assert!(gate.admit([&bands[0], &bands[1]]));
    }

    #[test]
    fn area_reduction_uses_all_sixteen_pixels_and_only_writes_selected_roi() {
        let mut band = flat(0);
        for row in 0..4 {
            for col in 0..MAP_WIDTH {
                for dy in 0..4 {
                    for dx in 0..4 {
                        let at = ((row * 4 + dy) * BAND_WIDTH + col * 4 + dx) * 3;
                        band[at..at + 3].copy_from_slice(&[
                            row as u8 * 10 + dy as u8,
                            col as u8,
                            dx as u8 * 2,
                        ]);
                    }
                }
            }
        }
        let working = working_chart(&band);
        assert!(working[..48 * MAP_WIDTH * 3].iter().all(|&v| v == 0));
        assert!(working[52 * MAP_WIDTH * 3..].iter().all(|&v| v == 0));
        for row in 0..4 {
            for col in 0..MAP_WIDTH {
                let at = ((48 + row) * MAP_WIDTH + col) * 3;
                assert_eq!(&working[at..at + 3], &[row as u8 * 10 + 2, col as u8, 3]);
            }
        }
    }

    #[test]
    fn band_shape_is_exact_for_each_ordinal() {
        let good = flat(0);
        assert!(validate([&good, &good]).is_ok());
        assert!(validate([&good[..good.len() - 1], &good]).is_err());
        assert!(validate([&good, &good[..good.len() - 1]]).is_err());
    }

    #[test]
    fn area_rounding_preserves_the_even_byte_on_both_sides_of_a_tie() {
        let mut band = flat(2);
        for y in 0..2 {
            band[y * BAND_WIDTH * 3..(y * BAND_WIDTH + 4) * 3].fill(3);
        }
        let working = working_chart(&band);
        assert_eq!(
            &working[48 * MAP_WIDTH * 3..48 * MAP_WIDTH * 3 + 3],
            &[2, 2, 2]
        );
    }
}
