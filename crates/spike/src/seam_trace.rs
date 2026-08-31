//! Shared output-view seam trace law used by the authenticated type-2 tools.

use kjerag_media::Fallible;
use kjerag_render::map_oracle::DenseMap;

/// Paint the actual selected-alpha 0.5 crossings over an existing RGBA view.
///
/// Each covered pixel is compared only with its right and lower covered
/// neighbour. A crossing endpoint is expanded to a clipped 3 by 3 red mark.
/// The input pixels are otherwise preserved byte for byte.
pub fn trace_alpha(rgba: &[u8], dense: &DenseMap) -> Fallible<Vec<u8>> {
    let width = dense.size.width as usize;
    let height = dense.size.height as usize;
    if rgba.len() != width * height * 4 {
        return Err("trace input extent differs from dense map".into());
    }
    let signed = dense
        .pixels
        .iter()
        .map(|pixel| (pixel.alpha - 0.5, pixel.covered > 0.5))
        .collect::<Vec<_>>();
    Ok(trace_signed(rgba, &signed, width, height))
}

fn trace_signed(rgba: &[u8], signed: &[(f32, bool)], width: usize, height: usize) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let crosses = |other: usize| {
                signed[index].1
                    && signed[other].1
                    && (signed[index].0 == 0.0
                        || signed[other].0 == 0.0
                        || signed[index].0.is_sign_positive() != signed[other].0.is_sign_positive())
            };
            if (x + 1 < width && crosses(index + 1)) || (y + 1 < height && crosses(index + width)) {
                for mark_y in y.saturating_sub(1)..=(y + 1).min(height - 1) {
                    for mark_x in x.saturating_sub(1)..=(x + 1).min(width - 1) {
                        out[4 * (mark_y * width + mark_x)..4 * (mark_y * width + mark_x) + 4]
                            .copy_from_slice(&[255, 40, 40, 255]);
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossing_law_requires_covered_four_neighbours_and_preserves_the_base() {
        let base = (0..4 * 4 * 4).map(|value| value as u8).collect::<Vec<_>>();
        let signed = [
            (-0.1, true),
            (0.1, true),
            (-0.1, false),
            (0.1, true),
            (-0.2, true),
            (-0.2, true),
            (0.0, true),
            (-0.2, true),
            (-0.3, true),
            (-0.3, true),
            (-0.3, true),
            (-0.3, true),
            (-0.4, true),
            (-0.4, true),
            (-0.4, true),
            (-0.4, true),
        ];
        let traced = trace_signed(&base, &signed, 4, 4);
        let red = [255, 40, 40, 255];
        // (0,0) crosses right and marks its clipped 3 by 3 neighbourhood.
        assert_eq!(&traced[0..4], red);
        assert_eq!(&traced[4 * 5..4 * 6], red);
        // Exact zero at (2,1) crosses its lower neighbour and marks around it.
        assert_eq!(&traced[4 * 10..4 * 11], red);
        // The far lower-left pixel is outside every mark and stays exact.
        assert_eq!(&traced[4 * 12..4 * 13], &base[4 * 12..4 * 13]);
    }

    #[test]
    fn uncovered_sign_change_is_not_a_crossing() {
        let base = [1_u8; 8];
        let traced = trace_signed(&base, &[(-1.0, false), (1.0, true)], 2, 1);
        assert_eq!(traced, base);
    }
}
