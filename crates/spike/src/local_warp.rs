//! The pure pose-versus-local fit behind Stage 9's instrument.
//!
//! A rendered crossing supplies a two-axis displacement and the numerical
//! response of that crossing to each of the five calibration knobs.  This
//! module asks the narrow first question: whether one pose explains every
//! supplied crossing.  It deliberately knows nothing about pixels, files, or
//! the renderer, so the real instrument and its planted controls share the
//! exact same fit.

/// The number of existing calibration knobs in a global seam pose.
pub const KNOBS: usize = 5;

/// A displacement in the seam's camera-frame axes, in degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Displacement {
    pub epi: f64,
    pub perp: f64,
}

impl Displacement {
    fn squared(self) -> f64 {
        self.epi.powi(2) + self.perp.powi(2)
    }
}

/// The response, in degrees per knob unit, of one crossing to a global pose.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Jacobian {
    pub epi: [f64; KNOBS],
    pub perp: [f64; KNOBS],
}

/// One independently traced seam crossing.
///
/// `covariance` is the trace's full two-axis covariance in degrees squared.
/// The fit whitens by it: a noisy trace remains evidence, but cannot outvote
/// a sharp one merely because its numbers are larger.  The off-diagonal term
/// matters when a registration's axes are correlated.
#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    pub name: String,
    pub displacement: Displacement,
    pub covariance: Covariance,
    pub jacobian: Jacobian,
}

impl Observation {
    /// A control reading of a crossing against itself.
    ///
    /// The instrument supplies its ordinary trace uncertainty and numerical
    /// pose response, but the measured difference is exactly zero.
    pub fn self_pair(name: impl Into<String>, covariance: Covariance, jacobian: Jacobian) -> Self {
        Self {
            name: name.into(),
            displacement: Displacement::default(),
            covariance,
            jacobian,
        }
    }
}

/// Why the global-pose question did not produce a numerical answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Refused {
    /// Fewer than four two-axis crossings leave fewer than three residual
    /// degrees of freedom after the five-knob pose fit.
    TooFewObservations { have: usize },
    /// Five pose knobs need more than five scalar measurements.
    TooFewAxes { have: usize },
    /// A trace did not provide a finite positive-definite covariance.
    InvalidCovariance { observation: usize },
    /// A numerical response was not finite.
    InvalidJacobian { observation: usize },
    /// The references do not constrain one independent global-pose direction.
    Singular,
}

/// The diagnostic result of fitting a shared global pose.
#[derive(Clone, Debug, PartialEq)]
pub struct SharedPose {
    /// One correction for all observations, in the existing five knob units.
    pub knobs: [f64; KNOBS],
    /// What that one pose predicts at each observation, in input order.
    pub predicted: Vec<Displacement>,
    /// Measured minus predicted displacement at each observation.
    pub residuals: Vec<Displacement>,
    /// RMS residual in physical degrees across both axes.
    pub rms: f64,
    /// Square root of chi-squared per residual degree of freedom after the
    /// full covariance whitening.
    pub normalized_rms: f64,
    /// Sum of squared, error-normalized residuals.
    pub chi_squared: f64,
    /// Independent scalar residuals after the five fitted pose knobs.
    pub degrees_of_freedom: usize,
    /// Infinity-norm condition estimate for the whitened normal matrix.
    ///
    /// It is a diagnostic, not a gate: the instrument reports it beside a
    /// conclusion so an apparently good pose is not trusted when the paired
    /// views scarcely constrain one of its knobs.
    pub condition: f64,
}

/// Fit one global five-knob pose to two-axis crossing observations.
pub fn fit(observations: &[Observation]) -> Result<SharedPose, Refused> {
    if observations.len() < 4 {
        return Err(Refused::TooFewObservations {
            have: observations.len(),
        });
    }
    let axes = observations.len() * 2;
    if axes <= KNOBS {
        return Err(Refused::TooFewAxes { have: axes });
    }

    let mut rows = Vec::with_capacity(axes);
    for (index, observation) in observations.iter().enumerate() {
        if !observation.displacement.epi.is_finite()
            || !observation.displacement.perp.is_finite()
            || observation
                .jacobian
                .epi
                .iter()
                .chain(&observation.jacobian.perp)
                .any(|term| !term.is_finite())
        {
            return Err(Refused::InvalidJacobian { observation: index });
        }
        let whitened =
            whiten(observation).ok_or(Refused::InvalidCovariance { observation: index })?;
        rows.extend(whitened);
    }

    let mut normal = [[0.0; KNOBS]; KNOBS];
    let mut right = [0.0; KNOBS];
    for (basis, value) in &rows {
        for row in 0..KNOBS {
            right[row] += basis[row] * value;
            for column in 0..KNOBS {
                normal[row][column] += basis[row] * basis[column];
            }
        }
    }
    let inverse = invert(normal).ok_or(Refused::Singular)?;
    let knobs = std::array::from_fn(|row| {
        (0..KNOBS)
            .map(|column| inverse[row][column] * right[column])
            .sum()
    });

    let predicted: Vec<Displacement> = observations
        .iter()
        .map(|observation| predict(observation.jacobian, knobs))
        .collect();
    let residuals: Vec<Displacement> = observations
        .iter()
        .zip(&predicted)
        .map(|(observation, predicted)| Displacement {
            epi: observation.displacement.epi - predicted.epi,
            perp: observation.displacement.perp - predicted.perp,
        })
        .collect();
    let squared: f64 = residuals.iter().map(|residual| residual.squared()).sum();
    let chi_squared: f64 = residuals
        .iter()
        .zip(observations)
        .map(|(residual, observation)| whiten_displacement(*residual, observation.covariance))
        .map(|whitened| whitened.map_or(f64::NAN, |value| value.squared()))
        .sum();
    let degrees_of_freedom = axes - KNOBS;
    Ok(SharedPose {
        knobs,
        predicted,
        residuals,
        rms: (squared / axes as f64).sqrt(),
        normalized_rms: (chi_squared / degrees_of_freedom as f64).sqrt(),
        chi_squared,
        degrees_of_freedom,
        condition: norm_inf(normal) * norm_inf(inverse),
    })
}

/// What a pose predicts at one crossing.  Public for planted controls.
pub fn predict(jacobian: Jacobian, knobs: [f64; KNOBS]) -> Displacement {
    let apply = |basis: [f64; KNOBS]| basis.iter().zip(knobs).map(|(a, b)| a * b).sum();
    Displacement {
        epi: apply(jacobian.epi),
        perp: apply(jacobian.perp),
    }
}

/// One pixel in a close two-dimensional crossing patch, linearized about the
/// reference picture.
///
/// `gradient` is the reference luma gradient in screen x/y, `residual` is
/// the target-minus-reference luma at that pixel, and `weight` is its inverse
/// variance.  Thus a small translation `d` obeys
/// `residual = gradient dot d`.  The caller owns patch selection and any
/// robust reweighting: this deliberately does not pretend a scalar edge is a
/// two-dimensional crossing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegistrationSample {
    pub gradient: [f64; 2],
    pub residual: f64,
    pub weight: f64,
}

/// A sub-pixel screen displacement registered from a textured crossing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PixelDisplacement {
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

impl Covariance {
    /// A covariance with independent one-standard-deviation errors on each
    /// axis.  This is a convenience for callers whose trace has measured no
    /// correlation; real registrations should retain their full covariance.
    pub const fn diagonal(x: f64, y: f64) -> Self {
        Self {
            xx: x * x,
            xy: 0.0,
            yy: y * y,
        }
    }
}

/// A two-dimensional registration reading, before the renderer maps its
/// screen axes into the seam's camera-frame epi/perp axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Registration {
    pub displacement: PixelDisplacement,
    /// Estimated from the residual variance.  A caller must add its own
    /// rendering and trace uncertainty before constructing an [`Observation`].
    pub covariance: Covariance,
    /// Infinity-norm condition estimate of the weighted structure tensor.
    pub condition: f64,
    pub samples: usize,
}

/// Why a patch cannot provide a two-axis crossing measurement.
#[derive(Clone, Debug, PartialEq)]
pub enum RegistrationRefused {
    TooFewSamples {
        have: usize,
    },
    InvalidSample {
        sample: usize,
    },
    /// All usable gradients lie on one line: the aperture problem.  A scalar
    /// horizon trace reaches this refusal by design, rather than fabricating
    /// its unobserved tangent displacement.
    Aperture,
}

/// Register a close patch's two-dimensional translation by weighted least
/// squares.
///
/// This is intentionally only the local, linearized solve.  It does not pick
/// a patch, derive gradients, warp a renderer, or turn screen pixels into
/// camera-frame degrees; those pieces need to be recorded beside real pixels
/// before Stage 9 may make a physical claim.
pub fn register(samples: &[RegistrationSample]) -> Result<Registration, RegistrationRefused> {
    if samples.len() <= 2 {
        return Err(RegistrationRefused::TooFewSamples {
            have: samples.len(),
        });
    }
    let mut normal = [[0.0; 2]; 2];
    let mut right = [0.0; 2];
    for (index, sample) in samples.iter().enumerate() {
        if !sample.weight.is_finite()
            || sample.weight <= 0.0
            || !sample.residual.is_finite()
            || sample.gradient.iter().any(|value| !value.is_finite())
        {
            return Err(RegistrationRefused::InvalidSample { sample: index });
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
        return Err(RegistrationRefused::Aperture);
    }
    let inverse = [
        [normal[1][1] / determinant, -normal[0][1] / determinant],
        [-normal[1][0] / determinant, normal[0][0] / determinant],
    ];
    let displacement = PixelDisplacement {
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

fn norm_inf(matrix: [[f64; KNOBS]; KNOBS]) -> f64 {
    matrix
        .iter()
        .map(|row| row.iter().map(|value| value.abs()).sum::<f64>())
        .fold(0.0, f64::max)
}

/// Premultiply one two-axis observation by the inverse Cholesky factor of
/// its covariance.  The two output rows have unit covariance, including when
/// the registered epi/perp axes are correlated.
fn whiten(observation: &Observation) -> Option<[([f64; KNOBS], f64); 2]> {
    let (first, second) = cholesky_inverse_rows(observation.covariance)?;
    Some([
        (
            std::array::from_fn(|index| first[0] * observation.jacobian.epi[index]),
            first[0] * observation.displacement.epi,
        ),
        (
            std::array::from_fn(|index| {
                second[0] * observation.jacobian.epi[index]
                    + second[1] * observation.jacobian.perp[index]
            }),
            second[0] * observation.displacement.epi + second[1] * observation.displacement.perp,
        ),
    ])
}

fn whiten_displacement(value: Displacement, covariance: Covariance) -> Option<Displacement> {
    let (first, second) = cholesky_inverse_rows(covariance)?;
    Some(Displacement {
        epi: first[0] * value.epi,
        perp: second[0] * value.epi + second[1] * value.perp,
    })
}

/// Rows of the inverse of the lower Cholesky factor `L` where
/// `covariance = L * L^T`.  Strict positive definiteness is the only
/// covariance gate; no empirical condition cut-off is invented here.
fn cholesky_inverse_rows(covariance: Covariance) -> Option<([f64; 2], [f64; 2])> {
    if !covariance.xx.is_finite() || !covariance.xy.is_finite() || !covariance.yy.is_finite() {
        return None;
    }
    let l00 = covariance.xx.sqrt();
    if !l00.is_finite() || l00 <= 0.0 {
        return None;
    }
    let l10 = covariance.xy / l00;
    let l11_squared = covariance.yy - l10 * l10;
    let l11 = l11_squared.sqrt();
    if !l10.is_finite() || !l11.is_finite() || l11 <= 0.0 {
        return None;
    }
    Some(([1.0 / l00, 0.0], [-l10 / (l00 * l11), 1.0 / l11]))
}

/// Gauss-Jordan with partial pivoting.  The fit is only five by five, and
/// keeping it here makes its condition diagnostic describe the exact solve.
fn invert(matrix: [[f64; KNOBS]; KNOBS]) -> Option<[[f64; KNOBS]; KNOBS]> {
    let mut work = [[0.0; KNOBS * 2]; KNOBS];
    for row in 0..KNOBS {
        work[row][..KNOBS].copy_from_slice(&matrix[row]);
        work[row][KNOBS + row] = 1.0;
    }
    for column in 0..KNOBS {
        let pivot = (column..KNOBS).max_by(|left, right| {
            work[*left][column]
                .abs()
                .total_cmp(&work[*right][column].abs())
        })?;
        if !work[pivot][column].is_finite() || work[pivot][column] == 0.0 {
            return None;
        }
        work.swap(column, pivot);
        let divisor = work[column][column];
        for value in &mut work[column] {
            *value /= divisor;
        }
        for row in 0..KNOBS {
            if row == column {
                continue;
            }
            let factor = work[row][column];
            let pivot = work[column];
            for (value, above) in work[row].iter_mut().zip(pivot) {
                *value -= factor * above;
            }
        }
    }
    if work.iter().flatten().any(|value| !value.is_finite()) {
        return None;
    }
    Some(std::array::from_fn(|row| {
        std::array::from_fn(|column| work[row][KNOBS + column])
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const COVARIANCE: Covariance = Covariance::diagonal(0.01, 0.01);

    fn jacobian(index: usize) -> Jacobian {
        let x = index as f64 + 1.0;
        Jacobian {
            epi: [1.0, x, x * x, (0.7 * x).sin(), (0.3 * x).cos()],
            perp: [x * x, 1.0, (0.5 * x).cos(), x, (0.9 * x).sin()],
        }
    }

    fn planted(knobs: [f64; KNOBS]) -> Vec<Observation> {
        (0..6)
            .map(|index| {
                let jacobian = jacobian(index);
                Observation {
                    name: format!("crossing-{index}"),
                    displacement: predict(jacobian, knobs),
                    covariance: COVARIANCE,
                    jacobian,
                }
            })
            .collect()
    }

    #[test]
    fn an_exact_global_plant_is_recovered_at_every_crossing() {
        let wanted = [0.31, -0.17, 0.08, 0.43, -0.29];
        let fit = fit(&planted(wanted)).expect("the plant constrains every knob");
        for (got, wanted) in fit.knobs.iter().zip(wanted) {
            assert!((got - wanted).abs() < 1e-10, "{got} instead of {wanted}");
        }
        assert!(fit.rms < 1e-11, "global plant left {:.3e} degrees", fit.rms);
        assert_eq!(fit.degrees_of_freedom, 7);
        assert!(fit.condition.is_finite() && fit.condition > 1.0);
    }

    #[test]
    fn a_self_pair_is_exactly_zero() {
        let observations: Vec<Observation> = (0..6)
            .map(|index| {
                Observation::self_pair(format!("self-{index}"), COVARIANCE, jacobian(index))
            })
            .collect();
        let fit = fit(&observations).expect("zero readings still constrain the model");
        assert_eq!(fit.knobs, [0.0; KNOBS]);
        assert!(
            fit.predicted
                .iter()
                .all(|reading| *reading == Displacement::default())
        );
        assert!(
            fit.residuals
                .iter()
                .all(|reading| *reading == Displacement::default())
        );
        assert_eq!(fit.chi_squared, 0.0);
    }

    #[test]
    fn a_localized_plant_leaves_a_rejection_residual() {
        let mut observations = planted([0.0; KNOBS]);
        // A local two-axis shift at one crossing, with every other crossing
        // explicitly outside its support and therefore exactly zero.
        observations[0].displacement = Displacement {
            epi: 0.80,
            perp: -0.55,
        };
        let fit = fit(&observations).expect("the references constrain a pose");
        assert!(
            fit.rms > 0.10,
            "a local plant was absorbed as pose: {:.3e} degrees",
            fit.rms
        );
        assert!(
            fit.normalized_rms > 10.0,
            "a 0.01 degree trace would not reject the local residue: {:.2} sigma rms",
            fit.normalized_rms
        );
    }

    #[test]
    fn fewer_than_four_two_axis_crossings_refuse_before_fitting() {
        let observations = planted([0.0; KNOBS]);
        assert_eq!(
            fit(&observations[..3]),
            Err(Refused::TooFewObservations { have: 3 })
        );
        let fit = fit(&observations[..4]).expect("four crossings leave three residual dof");
        assert_eq!(fit.degrees_of_freedom, 3);
    }

    #[test]
    fn correlated_covariance_is_whitened_as_a_full_two_axis_measurement() {
        let covariance = Covariance {
            xx: 4.0,
            xy: 3.0,
            yy: 9.0,
        };
        let whitened = whiten_displacement(
            Displacement {
                epi: 2.0,
                perp: 3.0,
            },
            covariance,
        )
        .expect("positive-definite covariance");
        // [2, 3] C^-1 [2, 3]^T = 4/3.  Dividing axes independently would
        // incorrectly report 2 instead and discard their correlation.
        assert!((whitened.squared() - 4.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn non_positive_definite_covariance_refuses() {
        let mut observations = planted([0.0; KNOBS]);
        observations[0].covariance = Covariance {
            xx: 1.0,
            xy: 2.0,
            yy: 1.0,
        };
        assert_eq!(
            fit(&observations),
            Err(Refused::InvalidCovariance { observation: 0 })
        );
    }

    fn registration_samples(shift: PixelDisplacement) -> Vec<RegistrationSample> {
        // Several non-collinear gradients stand in for a textured crossing.
        // A straight horizon would instead repeat one direction and must not
        // be promoted to a two-axis measurement.
        [[1.0, 0.0], [0.0, 1.0], [1.0, 1.0], [2.0, -1.0]]
            .into_iter()
            .map(|gradient| RegistrationSample {
                residual: gradient[0] * shift.x + gradient[1] * shift.y,
                gradient,
                weight: 1.0,
            })
            .collect()
    }

    #[test]
    fn a_textured_crossing_recovers_a_planted_two_axis_translation() {
        let wanted = PixelDisplacement { x: 0.37, y: -0.22 };
        let reading = register(&registration_samples(wanted)).expect("a crossing has two axes");
        assert!((reading.displacement.x - wanted.x).abs() < 1e-12);
        assert!((reading.displacement.y - wanted.y).abs() < 1e-12);
        assert!(reading.covariance.xx < 1e-30);
        assert!(reading.covariance.xy.abs() < 1e-30);
        assert!(reading.covariance.yy < 1e-30);
        assert!(reading.condition.is_finite() && reading.condition > 1.0);
    }

    #[test]
    fn a_scalar_edge_refuses_the_aperture_problem() {
        let samples: Vec<RegistrationSample> = (0..4)
            .map(|index| RegistrationSample {
                gradient: [1.0, 0.0],
                residual: index as f64,
                weight: 1.0,
            })
            .collect();
        assert_eq!(register(&samples), Err(RegistrationRefused::Aperture));
    }

    #[test]
    fn registration_refuses_invalid_or_insufficient_evidence() {
        assert_eq!(
            register(&registration_samples(PixelDisplacement::default())[..2]),
            Err(RegistrationRefused::TooFewSamples { have: 2 })
        );
        let mut samples = registration_samples(PixelDisplacement::default());
        samples[2].weight = 0.0;
        assert_eq!(
            register(&samples),
            Err(RegistrationRefused::InvalidSample { sample: 2 })
        );
    }
}
