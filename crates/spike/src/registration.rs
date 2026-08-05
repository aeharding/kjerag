//! The local, linearized two-axis registration solve.
//!
//! Ported unchanged from the `feat/warp` branch's `local_warp.rs`, which a
//! 2026-08-05 audit passed: the solve, its aperture refusal and its
//! covariance. The rest of that file was a five-knob pose fit which pooled
//! sites 0.4 degrees apart as independent chi-square observations, and is
//! deliberately not here. This module makes no pose claim and fits no
//! calibration; it turns one patch of gradients and residuals into one
//! sub-pixel translation and the uncertainty of it.

/// One pixel of a patch, as the solve reads it.
///
/// `gradient` is the target luma gradient in grid x/y, `residual` is the
/// reference-minus-target luma at that pixel, and `weight` is its inverse
/// variance. Thus a small translation `d` obeys `residual = gradient dot d`.
/// The caller owns patch selection and any robust reweighting: this
/// deliberately does not pretend a scalar edge is a two-dimensional crossing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub gradient: [f64; 2],
    pub residual: f64,
    pub weight: f64,
}

/// A sub-pixel grid displacement registered from a textured crossing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Displacement {
    pub x: f64,
    pub y: f64,
}

/// The independent terms of a symmetric two-by-two covariance matrix.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Covariance {
    pub xx: f64,
    pub xy: f64,
    pub yy: f64,
}

/// A two-dimensional registration reading, in the grid the samples came in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Registration {
    pub displacement: Displacement,
    /// Estimated from the residual variance. A caller must add its own
    /// sampling and trace uncertainty before making a physical claim.
    pub covariance: Covariance,
    /// Infinity-norm condition estimate of the weighted structure tensor.
    pub condition: f64,
    pub samples: usize,
}

/// Why a patch cannot provide a two-axis crossing measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    TooFewSamples,
    InvalidSample,
    /// All usable gradients lie on one line: the aperture problem. A patch
    /// holding one straight edge and nothing else reaches this refusal by
    /// design, rather than fabricating its unobserved tangent displacement.
    Aperture,
}

/// Register a close patch's two-dimensional translation by weighted least
/// squares.
///
/// This is intentionally only the local solve. It does not pick a patch,
/// derive gradients, or turn grid steps into camera-frame degrees.
pub fn register(samples: &[Sample]) -> Result<Registration, Refused> {
    if samples.len() <= 2 {
        return Err(Refused::TooFewSamples);
    }
    let mut normal = [[0.0; 2]; 2];
    let mut right = [0.0; 2];
    for sample in samples {
        if !sample.weight.is_finite()
            || sample.weight <= 0.0
            || !sample.residual.is_finite()
            || sample.gradient.iter().any(|value| !value.is_finite())
        {
            return Err(Refused::InvalidSample);
        }
        for (row, gradient) in sample.gradient.iter().enumerate() {
            right[row] += sample.weight * gradient * sample.residual;
            for (column, other) in sample.gradient.iter().enumerate() {
                normal[row][column] += sample.weight * gradient * other;
            }
        }
    }
    let determinant = normal[0][0] * normal[1][1] - normal[0][1] * normal[1][0];
    let scale = normal[0][0].abs().max(normal[1][1].abs()).powi(2);
    if !determinant.is_finite() || determinant <= 1e-12 * scale {
        return Err(Refused::Aperture);
    }
    let inverse = [
        [normal[1][1] / determinant, -normal[0][1] / determinant],
        [-normal[1][0] / determinant, normal[0][0] / determinant],
    ];
    let displacement = Displacement {
        x: inverse[0][0] * right[0] + inverse[0][1] * right[1],
        y: inverse[1][0] * right[0] + inverse[1][1] * right[1],
    };
    let squared_residual: f64 = samples
        .iter()
        .map(|sample| {
            let prediction =
                sample.gradient[0] * displacement.x + sample.gradient[1] * displacement.y;
            sample.weight * (sample.residual - prediction).powi(2)
        })
        .sum();
    let variance = squared_residual / (samples.len() - 2) as f64;
    Ok(Registration {
        displacement,
        covariance: Covariance {
            xx: variance * inverse[0][0],
            xy: variance * inverse[0][1],
            yy: variance * inverse[1][1],
        },
        condition: norm2_inf(normal) * norm2_inf(inverse),
        samples: samples.len(),
    })
}

fn norm2_inf(matrix: [[f64; 2]; 2]) -> f64 {
    matrix
        .iter()
        .map(|row| row.iter().map(|value| value.abs()).sum::<f64>())
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(shift: Displacement) -> Vec<Sample> {
        // Several non-collinear gradients stand in for a textured crossing.
        // A straight horizon would instead repeat one direction and must not
        // be promoted to a two-axis measurement.
        [[1.0, 0.0], [0.0, 1.0], [1.0, 1.0], [2.0, -1.0]]
            .into_iter()
            .map(|gradient| Sample {
                residual: gradient[0] * shift.x + gradient[1] * shift.y,
                gradient,
                weight: 1.0,
            })
            .collect()
    }

    #[test]
    fn a_textured_crossing_recovers_a_planted_two_axis_translation() {
        let wanted = Displacement { x: 0.37, y: -0.22 };
        let reading = register(&samples(wanted)).expect("a crossing has two axes");
        assert!((reading.displacement.x - wanted.x).abs() < 1e-12);
        assert!((reading.displacement.y - wanted.y).abs() < 1e-12);
        assert!(reading.covariance.xx < 1e-30);
        assert!(reading.covariance.xy.abs() < 1e-30);
        assert!(reading.covariance.yy < 1e-30);
        assert!(reading.condition.is_finite() && reading.condition > 1.0);
    }

    #[test]
    fn a_scalar_edge_refuses_the_aperture_problem() {
        let edge: Vec<Sample> = (0..4)
            .map(|index| Sample {
                gradient: [1.0, 0.0],
                residual: f64::from(index),
                weight: 1.0,
            })
            .collect();
        assert_eq!(register(&edge), Err(Refused::Aperture));
    }

    #[test]
    fn registration_refuses_invalid_or_insufficient_evidence() {
        assert_eq!(
            register(&samples(Displacement::default())[..2]),
            Err(Refused::TooFewSamples)
        );
        let mut invalid = samples(Displacement::default());
        invalid[2].weight = 0.0;
        assert_eq!(register(&invalid), Err(Refused::InvalidSample));
    }
}
