//! The ways to carry the measured disparity into the render path, scored on
//! the same readings.
//!
//! The brief's three are one thing at three resolutions, which is the finding
//! that collapses them: a per-azimuth shift table, a coarse band mesh and a
//! dense per-pixel flow differ in how finely `(phi, psi)` is sampled and in
//! nothing else, because the geometry has already reduced the flow to one
//! number per direction. So there is one [`Warp`] here and several [`Plan`]s
//! over it, not several algorithms.
//!
//! The fourth is the one the measurements argued for and the brief did not
//! ask about: the same table, **pooled over the clip instead of read per
//! frame**. See [`Plan::pooled`].
//!
//! Scored by holding half the band out. Each plan is built from the even
//! positions round the seam circle at its own stride, and every plan is scored
//! at the odd ones, which no plan ever saw. Without that a dense plan scores
//! zero residual against its own training data and the comparison says
//! nothing.

use crate::band::{Accumulator, Node};
use crate::measure::Field;

/// A disparity field over the band: `phis` positions round the seam circle by
/// `rows` distances past it, in radians, with the gaps filled.
pub struct Warp {
    phis: usize,
    rows: Vec<f64>,
    /// Radians of disparity, `phis * rows.len()`, in row-major order.
    cells: Vec<f64>,
    /// How many cells were filled from a neighbour rather than measured.
    pub filled: usize,
    pub cells_total: usize,
}

impl Warp {
    /// The disparity this field carries at one direction, radians. Bilinear in
    /// azimuth, wrapping, and in distance past the seam, clamped.
    pub fn at(&self, phi: f64, psi_deg: f64) -> f64 {
        if self.cells.is_empty() {
            return 0.0;
        }
        let turn = phi.rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU;
        let x = turn * self.phis as f64;
        let (left, fx) = (x.floor() as usize % self.phis, x - x.floor());
        let right = (left + 1) % self.phis;
        match self.rows.len() {
            1 => self.blend(left, 0, right, 0, fx, 0.0),
            _ => {
                let mut low = 0;
                while low + 2 < self.rows.len() && self.rows[low + 1] < psi_deg {
                    low += 1;
                }
                let span = self.rows[low + 1] - self.rows[low];
                let fy = match span.abs() > 0.0 {
                    true => ((psi_deg - self.rows[low]) / span).clamp(0.0, 1.0),
                    false => 0.0,
                };
                self.blend(left, low, right, low + 1, fx, fy)
            }
        }
    }

    fn blend(&self, left: usize, low: usize, right: usize, high: usize, fx: f64, fy: f64) -> f64 {
        let cell = |phi: usize, row: usize| self.cells[row * self.phis + phi];
        let lower = cell(left, low) * (1.0 - fx) + cell(right, low) * fx;
        let upper = cell(left, high) * (1.0 - fx) + cell(right, high) * fx;
        lower * (1.0 - fy) + upper * fy
    }

    /// How many correlations a prepass building this field runs per frame.
    pub fn correlations(&self) -> usize {
        self.cells_total
    }
}

/// One candidate for how finely the band is sampled.
pub struct Plan {
    pub name: &'static str,
    /// Every `stride`-th position round the circle, of the even ones.
    pub stride: usize,
    /// Which of the measured rows this plan keeps, as indices into the
    /// field's own `psis`.
    pub rows: Vec<usize>,
    /// One field for the whole clip, pooled over every frame, instead of one
    /// per frame.
    ///
    /// Its flicker is zero because there is nothing to flicker: the warp does
    /// not change. That is not a cheat, it is the point. What ghosts across
    /// the seam of a paramotor clip is the harness, the cage and the lines,
    /// and those are **bolted to the camera**: their distance is a property of
    /// the aircraft and not of the frame, so a per-frame measurement of it is
    /// measuring noise on top of a constant.
    pub pooled: bool,
}

impl Plan {
    /// The candidates, cheapest first, over a field measured on `psis`. Cost
    /// is `azimuths * rows` correlations per frame, and the first rung is
    /// per clip rather than per frame:
    ///
    /// - the **per-clip table** is one number per direction round the seam
    ///   circle, pooled over every frame read;
    /// - the **per-frame table** is the same shape, re-read each frame;
    /// - the **mesh** carries the variation across the band as well, at half
    ///   the azimuth resolution for a comparable cost;
    /// - **dense** is both at once, which is what a per-pixel flow in the band
    ///   would give once the geometry has reduced it to one scalar per
    ///   direction. There is nothing further a flow could add: the epipolar
    ///   constraint has already taken the second component away.
    pub fn ladder(psis: &[f64]) -> Vec<Self> {
        let middle = psis
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map_or(0, |(index, _)| index);
        vec![
            Self {
                name: "per-clip table",
                stride: 1,
                rows: vec![middle],
                pooled: true,
            },
            Self {
                name: "per-frame table",
                stride: 1,
                rows: vec![middle],
                pooled: false,
            },
            Self {
                name: "per-frame mesh",
                stride: 2,
                rows: (0..psis.len()).collect(),
                pooled: false,
            },
            Self {
                name: "per-frame dense",
                stride: 1,
                rows: (0..psis.len()).collect(),
                pooled: false,
            },
        ]
    }

    /// This plan's field for one frame of a sweep, built from the even
    /// positions round the circle alone.
    pub fn build(&self, field: &Field, frame: usize, keep: f64) -> Warp {
        let rows: Vec<f64> = self.rows.iter().map(|row| field.psis[*row]).collect();
        let taken: Vec<usize> = (0..field.phis)
            .filter(|phi| phi % (2 * self.stride) == 0)
            .collect();
        let mut cells = Vec::with_capacity(taken.len() * rows.len());
        let mut measured = Vec::with_capacity(taken.len() * rows.len());
        for row in &self.rows {
            for phi in &taken {
                let node = phi * field.psis.len() + row;
                let read = match self.pooled {
                    true => field.held(node, keep).map(|(mean, _)| mean),
                    false => field.frames[frame].peaks[node]
                        .filter(|peak| peak.r >= keep)
                        .map(|peak| peak.epi),
                };
                cells.push(read.unwrap_or(0.0));
                measured.push(read.is_some());
            }
        }
        // A position with nothing to correlate is filled from the nearest one
        // that had something, round the circle. Sky is the common case and
        // zero would be right there; an occlusion at a depth edge is the other
        // one and zero would be wrong, so the neighbour wins on the case that
        // matters and the fill rate is reported either way.
        let filled = measured.iter().filter(|seen| !**seen).count();
        for row in 0..rows.len() {
            let base = row * taken.len();
            for index in 0..taken.len() {
                if measured[base + index] {
                    continue;
                }
                let mut reach = 1;
                while reach <= taken.len() / 2 {
                    let left = (index + taken.len() - reach % taken.len()) % taken.len();
                    let right = (index + reach) % taken.len();
                    if measured[base + left] {
                        cells[base + index] = cells[base + left];
                        break;
                    }
                    if measured[base + right] {
                        cells[base + index] = cells[base + right];
                        break;
                    }
                    reach += 1;
                }
            }
        }
        Warp {
            phis: taken.len(),
            rows,
            cells,
            filled,
            cells_total: taken.len() * self.rows.len(),
        }
    }
}

/// What one plan left on the half of the band it never saw.
pub struct Scored {
    pub name: &'static str,
    pub correlations: usize,
    /// Root mean square of what is left at the held-out nodes, degrees.
    pub residual_deg: f64,
    /// The worst node, degrees. A feature crossing the seam slides by the
    /// disagreement at its own direction, not by the average of the circle.
    pub worst_deg: f64,
    /// The same, over the held-out nodes whose content is nearer than
    /// [`NEAR_M`]: the harness and the lines, which is what this campaign is
    /// for.
    pub near_deg: f64,
    pub near_nodes: usize,
    /// How many held-out readings the row above is over.
    pub scored_nodes: usize,
    /// What the same nodes read with no correction at all, degrees.
    pub uncorrected_deg: f64,
    /// Frame to frame, how much the field moves at a fixed direction: the
    /// flicker, in degrees, and the same after the smoothing below.
    pub flicker_deg: f64,
    pub smoothed_flicker_deg: f64,
    pub smoothed_residual_deg: f64,
    pub filled_share: f64,
}

/// What counts as near-field for the score above. Past this a disparity is
/// under a fifth of a degree, which is 3 px of a 1920-wide 90 degree view and
/// under what the blend already hides (docs/research/insv-format.md 6.1).
pub const NEAR_M: f64 = 10.0;

/// How much of the last frame's field is kept. One over the number of frames
/// the field takes to answer a step, so 0.5 is a two-frame settle.
pub const SMOOTHING: f64 = 0.5;

/// How many directions the flicker is watched at, round the seam circle.
///
/// Flicker is measured where the warp is **applied**, not where it was
/// measured: what the eye sees is the whole band moving, and most of the band
/// is filled rather than read. Watching only the nodes that correlated would
/// report the flicker of the readings and call it the flicker of the picture.
const WATCHED: usize = 360;

pub fn score(plan: &Plan, field: &Field, keep: f64) -> Scored {
    let held: Vec<usize> = (0..field.nodes.len())
        .filter(|node| (node / field.psis.len()) % 2 == 1)
        .collect();
    let warps: Vec<Warp> = (0..field.frames.len())
        .map(|frame| plan.build(field, frame, keep))
        .collect();
    let watched: Vec<(f64, f64)> = (0..WATCHED)
        .flat_map(|index| {
            let phi = index as f64 / WATCHED as f64 * std::f64::consts::TAU;
            field.psis.clone().into_iter().map(move |psi| (phi, psi))
        })
        .collect();

    let mut residual = Accumulator::default();
    let mut near = Accumulator::default();
    let mut uncorrected = Accumulator::default();
    let mut flicker = Accumulator::default();
    let mut smoothed_flicker = Accumulator::default();
    let mut smoothed_residual = Accumulator::default();
    let mut worst: f64 = 0.0;
    let mut smoothed: Vec<Option<f64>> = vec![None; held.len()];

    for (frame, warp) in warps.iter().enumerate() {
        for (slot, node) in held.iter().enumerate() {
            let at: &Node = &field.nodes[*node];
            let predicted = warp.at(at.phi, at.psi.to_degrees());
            let smooth = match smoothed[slot] {
                Some(previous) => previous + SMOOTHING * (predicted - previous),
                None => predicted,
            };
            smoothed[slot] = Some(smooth);
            let Some(peak) = field.frames[frame].peaks[*node] else {
                continue;
            };
            if peak.r < keep {
                continue;
            }
            let left = (peak.epi - predicted).to_degrees();
            residual.add(left);
            smoothed_residual.add((peak.epi - smooth).to_degrees());
            uncorrected.add(peak.epi.to_degrees());
            worst = worst.max(left.abs());
            if at.metres(peak.epi) < NEAR_M {
                near.add(left);
            }
        }
    }
    // The flicker, measured over the whole band rather than over the nodes:
    // both the raw field and the same field through the filter, on one pass,
    // so the two numbers are of the same run.
    let mut raw_before: Vec<Option<f64>> = vec![None; watched.len()];
    let mut smooth_before: Vec<Option<f64>> = vec![None; watched.len()];
    let mut running: Vec<Option<f64>> = vec![None; watched.len()];
    for warp in &warps {
        for (slot, (phi, psi)) in watched.iter().enumerate() {
            let predicted = warp.at(*phi, *psi);
            let smooth = match running[slot] {
                Some(previous) => previous + SMOOTHING * (predicted - previous),
                None => predicted,
            };
            running[slot] = Some(smooth);
            if let Some(before) = raw_before[slot] {
                flicker.add((predicted - before).to_degrees());
            }
            if let Some(before) = smooth_before[slot] {
                smoothed_flicker.add((smooth - before).to_degrees());
            }
            raw_before[slot] = Some(predicted);
            smooth_before[slot] = Some(smooth);
        }
    }

    let sample = warps.first();
    Scored {
        name: plan.name,
        correlations: sample.map_or(0, Warp::correlations),
        residual_deg: residual.rms(),
        worst_deg: worst,
        near_deg: near.rms(),
        near_nodes: near.count,
        scored_nodes: residual.count,
        uncorrected_deg: uncorrected.rms(),
        flicker_deg: flicker.rms(),
        smoothed_flicker_deg: smoothed_flicker.rms(),
        smoothed_residual_deg: smoothed_residual.rms(),
        filled_share: match sample {
            Some(warp) if warp.cells_total > 0 => warp.filled as f64 / warp.cells_total as f64,
            _ => 0.0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warp(cells: Vec<f64>, phis: usize, rows: Vec<f64>) -> Warp {
        let cells_total = cells.len();
        Warp {
            phis,
            rows,
            cells,
            filled: 0,
            cells_total,
        }
    }

    /// The field wraps round the circle. An azimuth just under a full turn
    /// blends the last cell with the first, and not with nothing.
    #[test]
    fn the_field_wraps_round_the_seam_circle() {
        let field = warp(vec![0.0, 1.0, 2.0, 3.0], 4, vec![0.0]);
        let turn = std::f64::consts::TAU;
        assert!((field.at(0.0, 0.0) - 0.0).abs() < 1e-12);
        assert!((field.at(turn * 0.25, 0.0) - 1.0).abs() < 1e-12);
        // Half way between the last cell and the first, the short way round.
        assert!((field.at(turn * 0.875, 0.0) - 1.5).abs() < 1e-12);
        assert!((field.at(turn, 0.0) - 0.0).abs() < 1e-12);
    }

    /// Across the band it interpolates between rows and clamps outside them,
    /// so a ray past the measured rows keeps the nearest row's disparity
    /// rather than running off to an extrapolated one.
    #[test]
    fn the_field_clamps_past_the_rows_it_was_measured_on() {
        let field = warp(vec![0.0, 0.0, 1.0, 1.0], 2, vec![-2.0, 2.0]);
        assert!((field.at(0.0, 0.0) - 0.5).abs() < 1e-12);
        assert!((field.at(0.0, -2.0) - 0.0).abs() < 1e-12);
        assert!((field.at(0.0, 2.0) - 1.0).abs() < 1e-12);
        assert!((field.at(0.0, -40.0) - 0.0).abs() < 1e-12);
        assert!((field.at(0.0, 40.0) - 1.0).abs() < 1e-12);
    }

    /// The ladder's first rung is the pooled one, and it is the only one that
    /// cannot flicker.
    #[test]
    fn the_cheapest_plan_is_the_one_that_cannot_flicker() {
        let ladder = Plan::ladder(&[-2.0, 0.0, 2.0]);
        assert!(ladder[0].pooled);
        assert!(ladder[1..].iter().all(|plan| !plan.pooled));
        assert_eq!(ladder[0].rows, vec![1]);
    }
}
