//! Diagnostic-only field-interior coherence reading.
//!
//! This measures two already-rendered pictures. It does not estimate or apply
//! a correction and has no route into playback. The arithmetic is the field
//! interior gate introduced by the stage-8 `colour` instrument after the
//! owner's dark-soil rejection; its evidence contract is recorded in
//! `docs/research/reference-views.md` and `docs/research/seam-blending.md`.
//! The caller must supply an exact ON/neutral source pair and the [`Reframe`]
//! which rendered both. Shape checks are enforced here, but source identity
//! and shared geometry cannot be recovered from bare RGBA bytes.

use crate::{Reframe, Size};

const LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722];
const SWEEP: usize = 256;
const DARK: f64 = 64.0;

/// How far off the seam the diagnostic samples, in degrees.
pub const INTERIOR: (f64, f64) = (7.0, 60.0);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Interior {
    /// Mean absolute applied correction, in gamma-coded RGB luma codes.
    pub applied: f64,
    /// RMS five-term smooth component divided by content level, as a Weber fraction.
    pub smooth: f64,
    /// RMS residual divided by content level, as a Weber fraction.
    pub rough: f64,
    /// Largest non-wrapping neighboring-bin change, as a Weber fraction.
    pub step: f64,
    /// Number of admitted azimuth bins.
    pub bins: usize,
}

impl Interior {
    pub fn report(&self) -> String {
        format!(
            "applied {:.2} codes; smooth {:.2}%, ROUGH {:.2}%, worst neighbour step {:.2}%              ({} bins)",
            self.applied,
            100.0 * self.smooth,
            100.0 * self.rough,
            100.0 * self.step,
            self.bins,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bin {
    /// Zero-based index in the 256-bin azimuth sweep.
    pub index: usize,
    /// Number of eligible dark pixels accumulated into this bin.
    pub pixels: usize,
    /// Bin azimuth, in radians.
    pub phi: f64,
    /// Mean applied correction, in gamma-coded RGB luma codes.
    pub codes: f64,
    /// Mean neutral-picture level, in gamma-coded RGB luma codes.
    pub level: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub summary: Option<Interior>,
    pub bins: Vec<Bin>,
}

/// Measure the non-smooth part of an applied field over dark interior pixels.
///
/// `ripple` is the diagnostic's existing eight-cycle positive control, in
/// codes. Both inputs must be tightly packed RGBA8 pictures of exactly `size`.
pub fn measure(before: &[u8], after: &[u8], reframe: &Reframe, size: Size, ripple: f64) -> Reading {
    measure_with_selection(before, after, reframe, size, ripple, |_, _| {})
}

/// Measure while observing each eligible pixel's row-major index and azimuth bin.
///
/// Selection is reported before the per-bin minimum population is applied.
/// The observer does not participate in the measurement or change its order.
pub fn measure_with_selection(
    before: &[u8],
    after: &[u8],
    reframe: &Reframe,
    size: Size,
    ripple: f64,
    mut selected: impl FnMut(usize, usize),
) -> Reading {
    let width = size.width as usize;
    let pixels = width
        .checked_mul(size.height as usize)
        .expect("field-interior picture dimensions overflow");
    let bytes = pixels
        .checked_mul(4)
        .expect("field-interior RGBA dimensions overflow");
    assert_eq!(
        before.len(),
        bytes,
        "field-interior before picture is not exact RGBA8 shape"
    );
    assert_eq!(
        after.len(),
        bytes,
        "field-interior after picture is not exact RGBA8 shape"
    );

    // Sums per azimuth bin: the applied correction, the level it sits on, and
    // how many pixels answered. Keep the f64 accumulation and traversal order
    // of the original colour diagnostic.
    let mut held = vec![(0.0f64, 0.0f64, 0.0f64); SWEEP];
    for index in 0..pixels {
        let uv = [
            (index % width) as f32 / size.width as f32,
            (index / width) as f32 / size.height as f32,
        ];
        let Some(ray) = reframe.view_ray(uv) else {
            continue;
        };
        let body = reframe.body_ray(ray);
        let length = (body[0] * body[0] + body[1] * body[1] + body[2] * body[2]).sqrt();
        if length <= 0.0 {
            continue;
        }
        let off = f64::from((body[2] / length).asin().to_degrees()).abs();
        if !(INTERIOR.0..=INTERIOR.1).contains(&off) {
            continue;
        }
        let phi = f64::from(body[1].atan2(body[0]));
        let bin = ((phi / std::f64::consts::TAU + 1.0) * SWEEP as f64) as usize % SWEEP;
        let (mut lift, mut level) = (0.0, 0.0);
        for (channel, weight) in LUMA.iter().enumerate() {
            let a = f64::from(before[4 * index + channel]);
            let b = f64::from(after[4 * index + channel]);
            lift += weight * (b - a);
            level += weight * a;
        }
        if level <= 0.0 || level > DARK {
            continue;
        }
        selected(index, bin);
        let planted = ripple * (8.0 * phi).cos();
        held[bin].0 += lift + planted;
        held[bin].1 += level;
        held[bin].2 += 1.0;
    }
    let seen: Vec<(f64, f64, f64)> = held
        .iter()
        .enumerate()
        .filter(|(_, bin)| bin.2 > 16.0)
        .map(|(index, bin)| {
            (
                index as f64 / SWEEP as f64 * std::f64::consts::TAU,
                bin.0 / bin.2,
                bin.1 / bin.2,
            )
        })
        .collect();
    let bins = held
        .iter()
        .enumerate()
        .filter(|(_, bin)| bin.2 > 16.0)
        .map(|(index, bin)| Bin {
            index,
            pixels: bin.2 as usize,
            phi: index as f64 / SWEEP as f64 * std::f64::consts::TAU,
            codes: bin.0 / bin.2,
            level: bin.1 / bin.2,
        })
        .collect();
    if seen.len() < 16 {
        return Reading {
            summary: None,
            bins,
        };
    }

    let mut normal = [[0.0f64; 5]; 5];
    let mut right = [0.0f64; 5];
    for (phi, codes, _) in &seen {
        let basis = [
            1.0,
            phi.cos(),
            phi.sin(),
            (2.0 * phi).cos(),
            (2.0 * phi).sin(),
        ];
        for row in 0..5 {
            for column in 0..5 {
                normal[row][column] += basis[row] * basis[column];
            }
            right[row] += basis[row] * codes;
        }
    }
    let fitted = solve5(normal, right);
    let smooth_at = |phi: f64| -> f64 {
        let basis = [
            1.0,
            phi.cos(),
            phi.sin(),
            (2.0 * phi).cos(),
            (2.0 * phi).sin(),
        ];
        (0..5).map(|term| fitted[term] * basis[term]).sum()
    };
    let count = seen.len() as f64;
    let mut applied = 0.0;
    let mut smooth = 0.0;
    let mut rough = 0.0;
    for (phi, codes, level) in &seen {
        applied += codes.abs();
        smooth += (smooth_at(*phi) / level).powi(2);
        rough += ((codes - smooth_at(*phi)) / level).powi(2);
    }
    let mut step: f64 = 0.0;
    for pair in seen.windows(2) {
        let level = 0.5 * (pair[0].2 + pair[1].2);
        if level > 0.0 {
            step = step.max((pair[1].1 - pair[0].1).abs() / level);
        }
    }
    Reading {
        summary: Some(Interior {
            applied: applied / count,
            smooth: (smooth / count).sqrt(),
            rough: (rough / count).sqrt(),
            step,
            bins: seen.len(),
        }),
        bins,
    }
}

fn solve5(mut normal: [[f64; 5]; 5], mut right: [f64; 5]) -> [f64; 5] {
    for pivot in 0..5 {
        let leading = normal[pivot];
        for row in (pivot + 1)..5 {
            let factor = normal[row][pivot] / leading[pivot];
            for (column, above) in leading.iter().enumerate().skip(pivot) {
                normal[row][column] -= factor * above;
            }
            right[row] -= factor * right[pivot];
        }
    }
    let mut out = [0.0f64; 5];
    for row in (0..5).rev() {
        let mut total = right[row];
        for column in (row + 1)..5 {
            total -= normal[row][column] * out[column];
        }
        out[row] = total / normal[row][row];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Size = Size {
        width: 512,
        height: 512,
    };

    fn picture(level: u8) -> Vec<u8> {
        [level, level, level, 255].repeat((SIZE.width * SIZE.height) as usize)
    }

    #[test]
    fn null_and_planted_ripples_are_relative_controls() {
        let picture = picture(32);
        let reframe = Reframe::blank(1.0, false);
        let null = measure(&picture, &picture, &reframe, SIZE, 0.0)
            .summary
            .expect("wide synthetic view did not cover the interior");
        assert_eq!(null.applied, 0.0);
        assert_eq!(null.smooth, 0.0);
        assert_eq!(null.rough, 0.0);
        assert_eq!(null.step, 0.0);

        let small = measure(&picture, &picture, &reframe, SIZE, 0.5)
            .summary
            .unwrap();
        let large = measure(&picture, &picture, &reframe, SIZE, 2.0)
            .summary
            .unwrap();
        assert!((large.applied / small.applied - 4.0).abs() < 1e-12);
        assert!((large.rough / small.rough - 4.0).abs() < 1e-12);
        assert!((large.step / small.step - 4.0).abs() < 1e-12);
    }

    #[test]
    fn bins_are_dark_eligible_and_small_coverage_keeps_diagnostics() {
        let reframe = Reframe::blank(1.0, false);
        let dark = picture(64);
        let reading = measure(&dark, &dark, &reframe, SIZE, 0.0);
        assert!(reading.summary.is_some());
        assert!(reading.bins.iter().all(|bin| bin.pixels > 16));
        assert!(reading.bins.iter().all(|bin| bin.level == 64.0));

        let bright = picture(65);
        let reading = measure(&bright, &bright, &reframe, SIZE, 0.0);
        assert!(reading.summary.is_none());
        assert!(reading.bins.is_empty());

        let tiny = Size {
            width: 8,
            height: 8,
        };
        let tiny_picture = [32, 32, 32, 255].repeat(64);
        let reading = measure(&tiny_picture, &tiny_picture, &reframe, tiny, 0.0);
        assert!(reading.summary.is_none());
        assert!(reading.bins.len() < 16);
    }

    #[test]
    fn selection_observer_preserves_reading_and_exact_bin_populations() {
        let reframe = Reframe::blank(1.0, false);
        let mut before = picture(32);
        for (index, pixel) in before.chunks_exact_mut(4).enumerate() {
            if index % 3 == 0 {
                pixel[..3].fill(65);
            } else if index % 7 == 0 {
                pixel[..3].fill(0);
            }
        }
        let after = picture(36);
        let plain = measure(&before, &after, &reframe, SIZE, 0.5);
        let mut population = [0usize; SWEEP];
        let mut previous = None;
        let observed =
            measure_with_selection(&before, &after, &reframe, SIZE, 0.5, |index, bin| {
                assert!(previous.is_none_or(|last| index > last));
                assert_eq!(before[4 * index], 32);
                previous = Some(index);
                population[bin] += 1;
            });
        assert!(observed.summary.is_some());
        assert_eq!(plain, observed);
        assert_eq!(
            observed.bins.len(),
            population.iter().filter(|&&count| count > 16).count()
        );
        for bin in observed.bins {
            assert_eq!(bin.pixels, population[bin.index]);
            assert_eq!(bin.level, 32.0);
        }
    }

    #[test]
    #[should_panic(expected = "before picture is not exact RGBA8 shape")]
    fn invalid_picture_shape_is_rejected() {
        measure(&[], &[], &Reframe::blank(1.0, false), SIZE, 0.0);
    }
}
