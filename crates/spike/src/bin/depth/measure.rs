//! The disparity field: what the band reads, frame by frame, and the controls
//! that say the readings mean what they claim.

use std::path::Path;

use kyerag_media::Fallible;
use kyerag_meta::{CalibrationSet, Lens};
use kyerag_render::{Camera, Held, Reframe, Sampling, Size};
use kyerag_spike::{Pair, Walk};

use crate::Options;
use crate::band::{Accumulator, Node, Peak, dot, epipolar_shift, free_shift, grid, sample, unit};

/// One frame's reading of the whole band, in node order.
pub struct Frame {
    pub at: f64,
    pub peaks: Vec<Option<Peak>>,
}

/// A run of frames read on one node grid, plus what it took to get them.
pub struct Field {
    pub nodes: Vec<Node>,
    pub phis: usize,
    pub psis: Vec<f64>,
    pub frames: Vec<Frame>,
    /// Why a node was not read, summed over the run.
    pub outside: usize,
    pub flat: usize,
    pub pinned: usize,
    pub seconds: f64,
}

impl Field {
    /// Every reading of one node over the run, in frame order.
    pub fn track(&self, node: usize) -> Vec<Option<Peak>> {
        self.frames.iter().map(|frame| frame.peaks[node]).collect()
    }

    /// The mean disparity at one node over the run, radians, and how many
    /// frames it was seen in.
    pub fn held(&self, node: usize, keep: f64) -> Option<(f64, usize)> {
        self.mean(node, keep, |peak| peak.epi)
    }

    /// The same on the axis depth cannot reach, which is what
    /// [`Prealign`] is fitted to.
    pub fn held_perp(&self, node: usize, keep: f64) -> Option<(f64, usize)> {
        self.mean(node, keep, |peak| peak.perp)
    }

    fn mean(&self, node: usize, keep: f64, of: impl Fn(Peak) -> f64) -> Option<(f64, usize)> {
        let taken: Vec<f64> = self
            .track(node)
            .into_iter()
            .flatten()
            .filter(|peak| peak.r >= keep)
            .map(of)
            .collect();
        match taken.is_empty() {
            true => None,
            false => Some((taken.iter().sum::<f64>() / taken.len() as f64, taken.len())),
        }
    }

    /// The same readings with a known step put into every frame, alternating
    /// sign. What the flicker instrument is checked against: a field that
    /// steps by `radians` each frame has to come back at twice that, and an
    /// instrument that cannot see a step it was handed cannot be believed when
    /// it reports none.
    pub fn shaken(&self, radians: f64) -> Self {
        Self {
            nodes: self.nodes.clone(),
            phis: self.phis,
            psis: self.psis.clone(),
            frames: self
                .frames
                .iter()
                .enumerate()
                .map(|(index, frame)| Frame {
                    at: frame.at,
                    peaks: frame
                        .peaks
                        .iter()
                        .map(|peak| {
                            peak.map(|peak| Peak {
                                epi: peak.epi
                                    + match index % 2 {
                                        0 => radians,
                                        _ => -radians,
                                    },
                                ..peak
                            })
                        })
                        .collect(),
                })
                .collect(),
            outside: self.outside,
            flat: self.flat,
            pinned: self.pinned,
            seconds: self.seconds,
        }
    }

    /// What the run refused to read, and why. Repetitive texture and flat sky
    /// are the two failure modes a band-wide search has, and they do not look
    /// the same: sky has no contrast to correlate, and a repetitive texture
    /// has plenty and a flat correlation peak.
    pub fn trust(&self, keep: f64) -> Trust {
        let mut curvature = Vec::new();
        let mut contrast = Accumulator::default();
        let (mut kept, mut weak) = (0, 0);
        for frame in &self.frames {
            for peak in frame.peaks.iter().flatten() {
                match peak.r >= keep {
                    true => kept += 1,
                    false => weak += 1,
                }
                curvature.push(peak.curvature);
                contrast.add(peak.contrast);
            }
        }
        curvature.sort_by(f64::total_cmp);
        Trust {
            kept,
            weak,
            // A peak this flat moves by a whole step for a hundredth of a
            // correlation, which is the noise, so its shift is a guess.
            flat_peaks: curvature.iter().filter(|curve| **curve < 0.02).count(),
            median_curvature: curvature
                .get(curvature.len() / 2)
                .copied()
                .unwrap_or_default(),
            contrast: contrast.rms(),
        }
    }
}

/// How far the readings of one run can be trusted, by their own shape.
pub struct Trust {
    pub kept: usize,
    pub weak: usize,
    pub flat_peaks: usize,
    pub median_curvature: f64,
    pub contrast: f64,
}

/// The map for one calibration, the camera left alone: a view ray is then a
/// direction in the body's own frame and a node of the band is addressed by
/// its angles.
pub fn mapped(lenses: &[Lens], frame: Size) -> Reframe {
    Reframe::new(
        lenses,
        frame,
        Camera::default(),
        Held::default(),
        1.0,
        false,
        Sampling::default(),
    )
}

/// A calibration with a fitted correction applied to lens 1.
///
/// The correction is an input here, not a fit: `crates/spike/src/bin/seam.rs`
/// (issue #48) is the one fitter in the tree and this instrument reads its
/// answer rather than growing a second one. What is left after it is what this
/// campaign is about.
pub fn fixed(lenses: &[Lens], fix: &[(String, f64)]) -> Vec<Lens> {
    let mut lenses = lenses.to_vec();
    let Some(lens) = lenses.get_mut(1) else {
        return lenses;
    };
    for (knob, amount) in fix {
        match knob.as_str() {
            "roll" => lens.pose.roll_deg += amount,
            "yaw" => lens.pose.yaw_deg += amount,
            "pitch" => lens.pose.pitch_deg += amount,
            "cx" => lens.intrinsics.cx += amount,
            "cy" => lens.intrinsics.cy += amount,
            _ => {}
        }
    }
    lenses
}

/// Which way is up in the body's frame, from the accelerometer, averaged over
/// the file.
///
/// On a still capture this is gravity and nothing else, which is what makes
/// the deck a plane at a known angle under the camera. In flight it is gravity
/// plus whatever the wing was doing, so it is reported and used for the deck
/// control alone.
pub fn body_up(calibration: &CalibrationSet) -> Option<[f64; 3]> {
    let samples = calibration.imu.samples();
    if samples.is_empty() {
        return None;
    }
    let body_from_imu = calibration.body_from_imu();
    let mut sum = [0.0; 3];
    for sample in samples {
        let g = body_from_imu.mul_vec(sample.accel_g);
        for axis in 0..3 {
            sum[axis] += g[axis];
        }
    }
    Some(unit(sum))
}

/// One capture's calibration and the frames the field is read from.
pub fn open(options: &Options, path: &Path) -> Fallible<(CalibrationSet, Vec<Pair>)> {
    let calibration = CalibrationSet::from_insv(path)?;
    let dim = calibration.dimension;
    let mut walk = Walk::open(
        path,
        options.from,
        kyerag_render::Size {
            width: dim.width,
            height: dim.height,
        },
    )?;
    if walk.streams() < 2 {
        return Err("this file carries one lens stream, so it has no seam".into());
    }
    let mut pairs = Vec::new();
    while pairs.len() < options.count {
        let Some(pair) = walk.next_pair()? else { break };
        pairs.push(pair);
    }
    match pairs.is_empty() {
        true => Err("no frame decoded".into()),
        false => Ok((calibration, pairs)),
    }
}

/// What is left on the axis depth cannot reach, as a function of azimuth.
///
/// Measured, not assumed: a free two-dimensional search reads both axes, and
/// the off-epipolar one comes back at 0.4 to 0.7 degrees on real footage after
/// the per-file calibration fit. That is five to nine correlation steps of
/// along-seam disagreement, so a one-dimensional search held at zero is
/// correlating a patch against content that is not the same content, and its
/// peak wanders by more than the disparity it is looking for (measured: it
/// disagreed with the free search by 0.42 to 1.65 degrees rms before this
/// existed).
///
/// It is a per-**file** table and not a per-frame one, because it is
/// calibration: a residual rotation and principal point are fixed in the
/// camera and turn with the azimuth in a constant and the first two harmonics
/// of it, which is what `--bin seam` decomposes the same column into
/// (docs/research/insv-format.md 6.8). Fitting those five numbers per row
/// rather than keeping a reading per node is what lets a node with nothing to
/// correlate still be pre-aligned, and what keeps the prepass's per-frame cost
/// one-dimensional.
pub struct Prealign {
    /// Five coefficients per row of the band, in `psis` order.
    rows: Vec<[f64; 5]>,
    psis: usize,
    pub read: usize,
    pub residual_deg: f64,
}

impl Prealign {
    /// No pre-alignment: what the instrument did before this existed, kept so
    /// the difference can be measured rather than asserted.
    pub fn none(psis: usize) -> Self {
        Self {
            rows: vec![[0.0; 5]; psis],
            psis,
            read: 0,
            residual_deg: 0.0,
        }
    }

    /// The off-epipolar offset at one node, radians.
    pub fn at(&self, node: usize, phi: f64) -> f64 {
        let row = self.rows[node % self.psis];
        row[0]
            + row[1] * phi.cos()
            + row[2] * phi.sin()
            + row[3] * (2.0 * phi).cos()
            + row[4] * (2.0 * phi).sin()
    }

    /// Fitted from a free two-dimensional search over `pairs`.
    pub fn fit(reframe: &Reframe, nodes: &[Node], pairs: &[Pair], options: &Options) -> Self {
        let psis = options.psis.len();
        let free = sweep(
            reframe,
            nodes,
            pairs,
            &Options {
                free: true,
                ..options.clone()
            },
            0.0,
            &Self::none(psis),
        );
        let mut rows = Vec::with_capacity(psis);
        let mut left = Accumulator::default();
        let mut read = 0;
        for row in 0..psis {
            let seen: Vec<(f64, f64)> = (row..nodes.len())
                .step_by(psis)
                .filter_map(|node| {
                    let (perp, _) = free.held_perp(node, options.keep)?;
                    Some((nodes[node].phi, perp))
                })
                .collect();
            read += seen.len();
            let terms = |phi: f64| {
                [
                    1.0,
                    phi.cos(),
                    phi.sin(),
                    (2.0 * phi).cos(),
                    (2.0 * phi).sin(),
                ]
            };
            // Under six readings the five harmonics are not determined, so the
            // row falls back to the mean, which is the term that carries most
            // of this column anyway (6.8: the constant is relative roll).
            let fitted = match seen.len() >= 6 {
                true => solve(&seen, terms),
                false => {
                    let mean = match seen.is_empty() {
                        true => 0.0,
                        false => seen.iter().map(|(_, v)| v).sum::<f64>() / seen.len() as f64,
                    };
                    [mean, 0.0, 0.0, 0.0, 0.0]
                }
            };
            for (phi, perp) in &seen {
                let predicted: f64 = terms(*phi)
                    .iter()
                    .zip(&fitted)
                    .map(|(term, coefficient)| term * coefficient)
                    .sum();
                left.add((perp - predicted).to_degrees());
            }
            rows.push(fitted);
        }
        Self {
            rows,
            psis,
            read,
            residual_deg: left.rms(),
        }
    }
}

/// Least squares for five coefficients, by Gauss-Jordan on the normal
/// equations. Five unknowns over tens of readings, so the conditioning that
/// would argue for anything better is not in play.
fn solve(rows: &[(f64, f64)], terms: impl Fn(f64) -> [f64; 5]) -> [f64; 5] {
    let mut matrix = [[0.0f64; 6]; 5];
    for (phi, value) in rows {
        let basis = terms(*phi);
        for (r, row) in matrix.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().take(5).enumerate() {
                *cell += basis[r] * basis[c];
            }
            row[5] += basis[r] * value;
        }
    }
    for pivot in 0..5 {
        let best = (pivot..5)
            .max_by(|a, b| matrix[*a][pivot].abs().total_cmp(&matrix[*b][pivot].abs()))
            .unwrap_or(pivot);
        matrix.swap(pivot, best);
        if matrix[pivot][pivot].abs() < 1e-12 {
            return [0.0; 5];
        }
        let scale = matrix[pivot][pivot];
        for cell in &mut matrix[pivot] {
            *cell /= scale;
        }
        let above = matrix[pivot];
        for (index, row) in matrix.iter_mut().enumerate() {
            if index == pivot {
                continue;
            }
            let factor = row[pivot];
            for (cell, leading) in row.iter_mut().zip(&above) {
                *cell -= factor * leading;
            }
        }
    }
    std::array::from_fn(|index| matrix[index][5])
}

/// Read the band on every frame of `pairs`.
///
/// `inject` adds a synthetic disparity to every node, in radians, which is the
/// positive control: a reading that does not move with it is not a reading of
/// disparity.
pub fn sweep(
    reframe: &Reframe,
    nodes: &[Node],
    pairs: &[Pair],
    options: &Options,
    inject: f64,
    prealign: &Prealign,
) -> Field {
    let started = std::time::Instant::now();
    let step = options.step.to_radians();
    let half = (options.span.to_radians() / 2.0 / step) as isize;
    // The search window is one-sided, because the answer is. A subject's
    // distance displaces its picture towards the front lens at every azimuth
    // and never the other way, so a window from `far` to `near` covers every
    // distance the band can hold at half the width a symmetric one would need,
    // and the width is what decides whether lens 1 still has a picture of the
    // content at all.
    let centre = (options.near + options.far).to_radians() / 2.0;
    let search = ((options.near - options.far) / 2.0 / options.step) as isize;
    let perp = match options.free {
        true => search.min(half),
        false => 0,
    };
    let (mut outside, mut flat, mut pinned) = (0, 0, 0);
    let frames = pairs
        .iter()
        .map(|pair| {
            let peaks = nodes
                .iter()
                .enumerate()
                .map(|(index, at)| {
                    let front = sample(
                        reframe,
                        &pair.lenses[0],
                        0,
                        at,
                        (half, half),
                        step,
                        [0.0; 2],
                    );
                    let Some(front) = front else {
                        outside += 1;
                        return None;
                    };
                    if front.contrast() < options.contrast {
                        flat += 1;
                        return None;
                    }
                    let back = sample(
                        reframe,
                        &pair.lenses[1],
                        1,
                        at,
                        (half + perp, half + search),
                        step,
                        [prealign.at(index, at.phi), centre - inject],
                    );
                    let Some(back) = back else {
                        outside += 1;
                        return None;
                    };
                    let found = match options.free {
                        true => free_shift(&front, &back, (perp, search), step),
                        false => epipolar_shift(&front, &back, search, step),
                    };
                    if found.is_none() {
                        pinned += 1;
                    }
                    // The window's own centre is part of the answer: a shift of
                    // zero steps means a disparity of `centre`, not of nothing.
                    found.map(|peak| Peak {
                        epi: peak.epi + centre,
                        ..peak
                    })
                })
                .collect();
            Frame {
                at: pair.at.as_secs_f64(),
                peaks,
            }
        })
        .collect();
    Field {
        nodes: nodes.to_vec(),
        phis: options.phis,
        psis: options.psis.clone(),
        frames,
        outside,
        flat,
        pinned,
        seconds: started.elapsed().as_secs_f64(),
    }
}

/// The node grid this campaign reads: `phis` positions round the seam circle
/// by the stated distances past it.
pub fn nodes(baseline: [f64; 3], options: &Options) -> Vec<Node> {
    grid(baseline, options.phis, &options.psis)
}

// ------------------------------------------------------------ the controls

/// What an injected disparity read back as.
///
/// The lesson of issue #45 at the size being reported: a known shift of the
/// magnitude near-field content produces is put into lens 1's sampling and
/// read off the same pixels. A slope of one says this instrument can see a
/// disparity of that size on this scene; anything else says the numbers beside
/// it are not measurements.
pub struct Recovered {
    pub injected_deg: f64,
    pub metres: f64,
    pub read_deg: f64,
    pub spread_deg: f64,
    pub nodes: usize,
}

pub fn recover(
    reframe: &Reframe,
    nodes: &[Node],
    pairs: &[Pair],
    options: &Options,
    base: &Field,
    injected_deg: f64,
    prealign: &Prealign,
) -> Recovered {
    let with = sweep(
        reframe,
        nodes,
        pairs,
        options,
        injected_deg.to_radians(),
        prealign,
    );
    let moved: Vec<f64> = (0..nodes.len())
        .filter_map(|node| {
            let (before, _) = base.held(node, options.keep)?;
            let (after, _) = with.held(node, options.keep)?;
            Some((after - before).to_degrees())
        })
        .collect();
    let count = moved.len();
    let mean = match count {
        0 => 0.0,
        _ => moved.iter().sum::<f64>() / count as f64,
    };
    let spread = match count {
        0 => 0.0,
        _ => (moved.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count as f64).sqrt(),
    };
    // What distance that injection stands for, quoted at the seam circle where
    // the whole baseline is visible.
    Recovered {
        injected_deg,
        metres: nodes
            .first()
            .map_or(f64::INFINITY, |node| node.metres(injected_deg.to_radians())),
        read_deg: mean,
        spread_deg: spread,
        nodes: count,
    }
}

/// The deck under a still camera, as one plane.
///
/// A camera standing on a deck looks at a plane a fixed distance under it, so
/// the distance along a node's own direction is that height over `-dot(centre,
/// up)` and the disparity is the reach over that distance. One free parameter
/// against a column that runs from nothing at the horizontal to degrees at the
/// boards: if the fitted height is a height and the fit is tight, the
/// instrument is reading depth and not something that correlates with azimuth.
pub struct Deck {
    pub height_m: f64,
    pub r: f64,
    pub nodes: usize,
    pub residual_deg: f64,
}

pub fn deck(field: &Field, up: [f64; 3], keep: f64) -> Option<Deck> {
    let rows: Vec<(f64, f64)> = (0..field.nodes.len())
        .filter_map(|index| {
            let node = &field.nodes[index];
            let below = -dot(node.centre, up);
            if below <= 0.05 {
                return None;
            }
            let (shift, _) = field.held(index, keep)?;
            // Disparity = reach / (height / below), so the basis is
            // reach * below and the one parameter is 1 / height.
            Some((node.reach_m * below, shift))
        })
        .collect();
    if rows.len() < 4 {
        return None;
    }
    let (sum_xy, sum_xx) = rows
        .iter()
        .fold((0.0, 0.0), |(xy, xx), (x, y)| (xy + x * y, xx + x * x));
    let slope = sum_xy / sum_xx;
    let mean_x = rows.iter().map(|(x, _)| x).sum::<f64>() / rows.len() as f64;
    let mean_y = rows.iter().map(|(_, y)| y).sum::<f64>() / rows.len() as f64;
    let (mut cov, mut vx, mut vy) = (0.0, 0.0, 0.0);
    for (x, y) in &rows {
        let (x, y) = (x - mean_x, y - mean_y);
        cov += x * y;
        vx += x * x;
        vy += y * y;
    }
    let residual = (rows
        .iter()
        .map(|(x, y)| (y - slope * x).powi(2))
        .sum::<f64>()
        / rows.len() as f64)
        .sqrt();
    Some(Deck {
        height_m: 1.0 / slope,
        r: match vx > 0.0 && vy > 0.0 {
            true => cov / (vx * vy).sqrt(),
            false => 0.0,
        },
        nodes: rows.len(),
        residual_deg: residual.to_degrees(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(phi: f64) -> [f64; 5] {
        [
            1.0,
            phi.cos(),
            phi.sin(),
            (2.0 * phi).cos(),
            (2.0 * phi).sin(),
        ]
    }

    /// The pre-alignment's own control: known coefficients, put through the
    /// same solver the fit uses, have to come back.
    ///
    /// A constant and two cycles is exactly what `--bin seam` decomposes the
    /// along-seam column into (docs/research/insv-format.md 6.8), so this is
    /// checking that the model the residual is known to have is the model the
    /// solver can recover.
    #[test]
    fn the_harmonic_solver_returns_the_coefficients_it_was_given() {
        let truth = [0.012, -0.004, 0.007, 0.001, -0.002];
        let rows: Vec<(f64, f64)> = (0..24)
            .map(|index| {
                let phi = f64::from(index) / 24.0 * std::f64::consts::TAU;
                let value = terms(phi)
                    .iter()
                    .zip(&truth)
                    .map(|(term, coefficient)| term * coefficient)
                    .sum();
                (phi, value)
            })
            .collect();
        let fitted = solve(&rows, terms);
        for (found, wanted) in fitted.iter().zip(&truth) {
            assert!((found - wanted).abs() < 1e-9, "{found} against {wanted}");
        }
    }

    /// A row with too few readings falls back to the mean rather than to five
    /// harmonics through four points, which would fit the noise exactly and
    /// then swing between the readings.
    #[test]
    fn a_thin_row_is_a_constant_and_not_five_harmonics() {
        let none = Prealign::none(3);
        assert_eq!(none.at(0, 1.2), 0.0);
        assert_eq!(none.at(2, 5.0), 0.0);
    }
}
