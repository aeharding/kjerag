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
    fn finite_and_positive(self) -> bool {
        self.epi.is_finite() && self.perp.is_finite() && self.epi > 0.0 && self.perp > 0.0
    }

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
/// `error` is the trace's one-standard-deviation error on each axis.  The fit
/// whitens by it: a noisy trace remains evidence, but cannot outvote a sharp
/// one merely because its numbers are larger.
#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    pub name: String,
    pub displacement: Displacement,
    pub error: Displacement,
    pub jacobian: Jacobian,
}

impl Observation {
    /// A control reading of a crossing against itself.
    ///
    /// The instrument supplies its ordinary trace uncertainty and numerical
    /// pose response, but the measured difference is exactly zero.
    pub fn self_pair(name: impl Into<String>, error: Displacement, jacobian: Jacobian) -> Self {
        Self {
            name: name.into(),
            displacement: Displacement::default(),
            error,
            jacobian,
        }
    }
}

/// Why the global-pose question did not produce a numerical answer.
#[derive(Clone, Debug, PartialEq)]
pub enum Refused {
    /// Five pose knobs need more than five scalar measurements.
    TooFewAxes { have: usize },
    /// A trace did not provide a finite positive uncertainty.
    InvalidError { observation: usize },
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
    /// RMS residual after each axis is divided by its trace error.
    pub normalized_rms: f64,
    /// Sum of squared, error-normalized residuals.
    pub chi_squared: f64,
    /// Independent scalar residuals behind the RMS diagnostic.
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
    let axes = observations.len() * 2;
    if axes <= KNOBS {
        return Err(Refused::TooFewAxes { have: axes });
    }

    let mut rows = Vec::with_capacity(axes);
    for (index, observation) in observations.iter().enumerate() {
        if !observation.error.finite_and_positive() {
            return Err(Refused::InvalidError { observation: index });
        }
        for (basis, value, error) in [
            (
                observation.jacobian.epi,
                observation.displacement.epi,
                observation.error.epi,
            ),
            (
                observation.jacobian.perp,
                observation.displacement.perp,
                observation.error.perp,
            ),
        ] {
            if !value.is_finite() || basis.iter().any(|term| !term.is_finite()) {
                return Err(Refused::InvalidJacobian { observation: index });
            }
            rows.push((basis.map(|term| term / error), value / error));
        }
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
        .map(|(residual, observation)| {
            (residual.epi / observation.error.epi).powi(2)
                + (residual.perp / observation.error.perp).powi(2)
        })
        .sum();
    let degrees_of_freedom = axes - KNOBS;
    Ok(SharedPose {
        knobs,
        predicted,
        residuals,
        rms: (squared / axes as f64).sqrt(),
        normalized_rms: (chi_squared / axes as f64).sqrt(),
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

fn norm_inf(matrix: [[f64; KNOBS]; KNOBS]) -> f64 {
    matrix
        .iter()
        .map(|row| row.iter().map(|value| value.abs()).sum::<f64>())
        .fold(0.0, f64::max)
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
        if work[pivot][column].abs() < 1e-12 {
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
    Some(std::array::from_fn(|row| {
        std::array::from_fn(|column| work[row][KNOBS + column])
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ERROR: Displacement = Displacement {
        epi: 0.01,
        perp: 0.01,
    };

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
                    error: ERROR,
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
        assert!(fit.condition.is_finite() && fit.condition > 1.0);
    }

    #[test]
    fn a_self_pair_is_exactly_zero() {
        let observations: Vec<Observation> = (0..6)
            .map(|index| Observation::self_pair(format!("self-{index}"), ERROR, jacobian(index)))
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
}
