//! The per-source representation of a seam fusion decision.
//!
//! The renderer carries [`Self::DISABLED_MAP_MODE`] in its existing uniform
//! block, but does not read a fusion map and continues to consume
//! [`Blend`](super::Blend) directly. Thus merely constructing
//! [`Fusion::disabled`] cannot change a sampled coordinate, a blend weight,
//! or the output pixels. Keeping the identity case explicit gives a later,
//! evidence-gated fusion map one typed place to add a source coordinate
//! residual without turning a screen-space adjustment into part of the
//! calibrated projection.

#![allow(
    dead_code,
    reason = "only the disabled mode is wired; the per-pixel identity representation awaits evidence-gated map use"
)]

use super::{Blend, MAX_LENSES, Size};

/// One lens's contribution to a fused output pixel.
///
/// `uv` follows the shader's existing [`frame_uv`](super::projection::wgsl)
/// convention: camera-model pixel centres become texture coordinates by
/// adding half a texel before dividing by the delivered frame dimensions.
/// `alpha` is normalized across valid sources.  `gain` is kept per source so
/// a future measured colour adjustment cannot become an output-space grade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Source {
    pub(crate) uv: [f32; 2],
    pub(crate) valid: bool,
    pub(crate) alpha: f32,
    pub(crate) gain: [f32; 3],
}

impl Source {
    const REFUSED: Self = Self {
        uv: [0.0; 2],
        valid: false,
        alpha: 0.0,
        gain: [1.0; 3],
    };
}

/// The two source decisions for one output pixel.
///
/// This is not a renderer input yet.  In particular, it does not allocate a
/// map, bind a texture, alter a shader, or introduce another sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Fusion {
    pub(crate) sources: [Source; MAX_LENSES],
}

impl Fusion {
    /// Uniform value which prohibits a fusion-map lookup.
    ///
    /// This is deliberately the only mode the renderer can construct. It
    /// occupies pre-existing ABI padding in [`super::Reframe`], so carrying it
    /// adds no binding and cannot make a map available by accident.
    pub(crate) const DISABLED_MAP_MODE: f32 = 0.0;

    /// Copy the calibrated [`Blend`] as an identity source-fusion decision.
    ///
    /// A source is active only if the existing blend claims it, its landing is
    /// valid, and the delivered frame can form a finite texture coordinate.
    /// This is stricter only for malformed hand-built `Blend` values; a
    /// `Reframe::blend` result already has precisely those invariants.  The
    /// active claims are normalized here so a caller cannot accidentally make
    /// a future fusion path brighten or darken a pixel by passing unnormalized
    /// confidence values.
    pub(crate) fn disabled(blend: Blend, frame: Size) -> Self {
        let usable_frame = frame.width > 0 && frame.height > 0;
        let claims: [bool; MAX_LENSES] = std::array::from_fn(|lens| {
            let landing = blend.landings[lens];
            let weight = blend.weights[lens];
            usable_frame
                && landing.inside
                && landing
                    .pixel
                    .iter()
                    .all(|coordinate| coordinate.is_finite())
                && weight.is_finite()
                && weight > 0.0
        });
        let total: f32 = (0..MAX_LENSES)
            .filter(|&lens| claims[lens])
            .map(|lens| blend.weights[lens])
            .sum();

        let sources = std::array::from_fn(|lens| {
            if !claims[lens] || !total.is_finite() || total <= 0.0 {
                return Source::REFUSED;
            }
            let landing = blend.landings[lens];
            Source {
                uv: [
                    (landing.pixel[0] + 0.5) / frame.width as f32,
                    (landing.pixel[1] + 0.5) / frame.height as f32,
                ],
                valid: true,
                alpha: blend.weights[lens] / total,
                gain: [1.0; 3],
            }
        });
        Self { sources }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Landing;

    const FRAME: Size = Size {
        width: 4000,
        height: 3000,
    };

    fn landing(pixel: [f32; 2]) -> Landing {
        Landing {
            pixel,
            inside: true,
            axis: 1.0,
            depth: 1.0,
        }
    }

    #[test]
    fn disabled_copies_identity_uv_gain_and_normalized_claims() {
        let fusion = Fusion::disabled(
            Blend {
                landings: [landing([999.5, 1499.5]), landing([2999.5, 749.5])],
                weights: [0.25, 0.75],
            },
            FRAME,
        );

        assert_eq!(fusion.sources[0].uv, [0.25, 0.5]);
        assert_eq!(fusion.sources[1].uv, [0.75, 0.25]);
        assert_eq!(fusion.sources[0].gain, [1.0; 3]);
        assert_eq!(fusion.sources[1].gain, [1.0; 3]);
        assert_eq!(fusion.sources[0].alpha, 0.25);
        assert_eq!(fusion.sources[1].alpha, 0.75);
        assert!(fusion.sources.iter().all(|source| source.valid));
    }

    #[test]
    fn disabled_normalizes_only_active_claims() {
        let fusion = Fusion::disabled(
            Blend {
                landings: [landing([0.0, 0.0]), landing([1.0, 1.0])],
                weights: [2.0, 6.0],
            },
            FRAME,
        );

        assert_eq!(fusion.sources[0].alpha, 0.25);
        assert_eq!(fusion.sources[1].alpha, 0.75);
        assert_eq!(
            fusion
                .sources
                .iter()
                .map(|source| source.alpha)
                .sum::<f32>(),
            1.0
        );
    }

    #[test]
    fn disabled_refuses_invalid_or_nonfinite_sources_without_nan() {
        let fusion = Fusion::disabled(
            Blend {
                landings: [
                    Landing::MISSED,
                    Landing {
                        pixel: [f32::NAN, 1.0],
                        inside: true,
                        axis: 0.0,
                        depth: 0.0,
                    },
                ],
                weights: [1.0, 1.0],
            },
            FRAME,
        );

        assert_eq!(fusion.sources, [Source::REFUSED; MAX_LENSES]);
        assert!(
            fusion
                .sources
                .iter()
                .flat_map(|source| [source.uv[0], source.uv[1], source.alpha])
                .all(f32::is_finite)
        );
    }

    #[test]
    fn disabled_refuses_zero_sized_frames() {
        let fusion = Fusion::disabled(
            Blend {
                landings: [landing([0.0, 0.0]), Landing::MISSED],
                weights: [1.0, 0.0],
            },
            Size::new(0, 3000),
        );

        assert_eq!(fusion.sources, [Source::REFUSED; MAX_LENSES]);
    }
}
