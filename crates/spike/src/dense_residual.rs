//! Deterministic, CPU-only source-coordinate residual measurements.
//!
//! This is deliberately an estimator, not an enable policy.  It compares
//! already-calibrated raw-luma patches, returns a bounded integer-pixel shift
//! and normalized-correlation confidence, and refuses flat, malformed or
//! edge patches.  [`crate::residual_gate`] remains responsible for deciding
//! whether an observation may ever become a renderer map.

/// One immutable single-channel luma image in row-major order.
#[derive(Clone, Copy, Debug)]
pub struct Luma<'a> {
    pub width: usize,
    pub height: usize,
    pub samples: &'a [f32],
}

impl Luma<'_> {
    fn valid(self) -> bool {
        self.width != 0
            && self.height != 0
            && self.samples.len() == self.width.saturating_mul(self.height)
            && self.samples.iter().all(|sample| sample.is_finite())
    }

    fn at(self, x: isize, y: isize) -> f32 {
        self.samples[y as usize * self.width + x as usize]
    }
}

/// Why a patch has no safe residual estimate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    InvalidImage,
    OutsideImage,
    FlatReference,
    NoFiniteCandidate,
    AmbiguousPeak,
}

/// A bounded translation from reference coordinates to moving coordinates.
///
/// `shift_px = [dx, dy]` means `moving(x + dx, y + dy)` best matches
/// `reference(x, y)`.  This is the direction a later lens-1 UV map needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    pub shift_px: [i32; 2],
    /// Zero to one normalized cross-correlation, never negative.
    pub confidence: f32,
}

/// A regular set of measured residual texels suitable for an `Rgba16Float`
/// upload later: `(du, dv, confidence, reserved)`. `du`/`dv` are normalized
/// lens-1 texture-coordinate deltas, not pixels or output coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Map {
    pub width: usize,
    pub height: usize,
    pub texels: Vec<[f32; 4]>,
}

impl Map {
    pub fn identity(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            texels: vec![[0.0; 4]; width.saturating_mul(height)],
        }
    }
}

/// Search a square patch deterministically using zero-mean normalized cross
/// correlation. `radius` and `search` are inclusive integer pixel radii.
///
/// A tie for the best NCC is refused rather than resolved by scan order.
/// This avoids turning repetitive horizon texture into an arbitrary map.
pub fn estimate(
    reference: Luma<'_>,
    moving: Luma<'_>,
    centre: [isize; 2],
    radius: usize,
    search: usize,
) -> Result<Estimate, Refusal> {
    if !reference.valid() || !moving.valid() {
        return Err(Refusal::InvalidImage);
    }
    let radius = radius as isize;
    let search = search as isize;
    let fits = |image: Luma<'_>, x: isize, y: isize| {
        x - radius >= 0
            && y - radius >= 0
            && x + radius < image.width as isize
            && y + radius < image.height as isize
    };
    if !fits(reference, centre[0], centre[1]) {
        return Err(Refusal::OutsideImage);
    }

    let mut best: Option<(f32, [i32; 2])> = None;
    let mut tied = false;
    for dy in -search..=search {
        for dx in -search..=search {
            let moved = [centre[0] + dx, centre[1] + dy];
            if !fits(moving, moved[0], moved[1]) {
                continue;
            }
            let score = ncc(reference, moving, centre, moved, radius)?;
            match best {
                None => best = Some((score, [dx as i32, dy as i32])),
                Some((previous, _)) if score > previous + 1e-6 => {
                    best = Some((score, [dx as i32, dy as i32]));
                    tied = false;
                }
                Some((previous, _)) if (score - previous).abs() <= 1e-6 => tied = true,
                _ => {}
            }
        }
    }
    let Some((score, shift_px)) = best else {
        return Err(Refusal::NoFiniteCandidate);
    };
    if tied {
        return Err(Refusal::AmbiguousPeak);
    }
    Ok(Estimate {
        shift_px,
        confidence: score.max(0.0),
    })
}

fn ncc(
    reference: Luma<'_>,
    moving: Luma<'_>,
    reference_centre: [isize; 2],
    moving_centre: [isize; 2],
    radius: isize,
) -> Result<f32, Refusal> {
    let side = 2 * radius + 1;
    let count = (side * side) as f32;
    let mut a_sum = 0.0;
    let mut b_sum = 0.0;
    for y in -radius..=radius {
        for x in -radius..=radius {
            a_sum += reference.at(reference_centre[0] + x, reference_centre[1] + y);
            b_sum += moving.at(moving_centre[0] + x, moving_centre[1] + y);
        }
    }
    let (a_mean, b_mean) = (a_sum / count, b_sum / count);
    let (mut dot, mut a_energy, mut b_energy) = (0.0, 0.0, 0.0);
    for y in -radius..=radius {
        for x in -radius..=radius {
            let a = reference.at(reference_centre[0] + x, reference_centre[1] + y) - a_mean;
            let b = moving.at(moving_centre[0] + x, moving_centre[1] + y) - b_mean;
            dot += a * b;
            a_energy += a * a;
            b_energy += b * b;
        }
    }
    if a_energy <= f32::EPSILON {
        return Err(Refusal::FlatReference);
    }
    if b_energy <= f32::EPSILON {
        return Err(Refusal::NoFiniteCandidate);
    }
    Ok(dot / (a_energy * b_energy).sqrt())
}

#[cfg(test)]
mod tests {
    use super::{Estimate, Luma, Map, Refusal, estimate};

    fn patterned(width: usize, height: usize) -> Vec<f32> {
        (0..height)
            .flat_map(|y| (0..width).map(move |x| ((x * 17 + y * 29 + x * y * 3) % 251) as f32))
            .collect()
    }

    #[test]
    fn recovers_a_known_two_axis_translation() {
        let source = patterned(25, 25);
        let mut moved = vec![0.0; source.len()];
        // moving(x + 2, y - 1) = reference(x, y)
        for y in 1..25 {
            for x in 0..23 {
                moved[(y - 1) * 25 + x + 2] = source[y * 25 + x];
            }
        }
        let result = estimate(
            Luma {
                width: 25,
                height: 25,
                samples: &source,
            },
            Luma {
                width: 25,
                height: 25,
                samples: &moved,
            },
            [12, 12],
            4,
            3,
        )
        .expect("textured planted patch");
        assert_eq!(result.shift_px, [2, -1]);
        assert!(result.confidence > 0.999);
    }

    #[test]
    fn refuses_a_flat_patch_instead_of_inventing_a_shift() {
        let flat = vec![0.5; 15 * 15];
        assert_eq!(
            estimate(
                Luma {
                    width: 15,
                    height: 15,
                    samples: &flat
                },
                Luma {
                    width: 15,
                    height: 15,
                    samples: &flat
                },
                [7, 7],
                3,
                2,
            ),
            Err(Refusal::FlatReference)
        );
    }

    #[test]
    fn map_identity_is_an_exact_zero_confidence_upload_shape() {
        assert_eq!(Map::identity(2, 3).texels, vec![[0.0; 4]; 6]);
    }

    #[test]
    fn estimate_value_is_a_small_plain_data_contract() {
        assert_eq!(
            Estimate {
                shift_px: [1, -2],
                confidence: 0.5
            }
            .shift_px,
            [1, -2]
        );
    }
}
