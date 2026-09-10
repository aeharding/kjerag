// SPDX-License-Identifier: GPL-3.0-or-later
// Author: Manao
// Copyright(c)2006 A.G.Balakhnin aka Fizick - global motion, overlap, mode, refineMVs
//! Readable CPU oracle for Kjerag's independently parallel finest refinement.
//!
//! The candidate order and arithmetic derive from MVTools, commit
//! `17250aa979616ac48dfb0e18abfdcf2bd4e3afc0`, by Manao and A.G. Balakhnin
//! (Fizick). That source permits GPL-2.0-or-later; this adaptation elects
//! GPL-3.0-or-later for compatibility with Kjerag.
//!
//! Unlike Studio's selected serial controller, every block reads left, up and
//! diagonal predictors from one immutable interpolated seed grid. It performs
//! no bad-block count, uneven multi-hexagon search or cross-block mutation.
//! This is a disclosed Kjerag execution candidate, not a Studio-parity claim.

use super::{pyramid::Level, search::FinestInput};
use std::fmt;

pub mod gpu;

/// Readable CPU oracle for the independently parallel coarse-level candidate.
pub mod coarse;

const BLOCK: usize = 16;
const MIN_DIMENSION: usize = 1_024;
const MAX_DIMENSION_EXCLUSIVE: usize = 8_192;
const MIN_DISPLACEMENT: i32 = -8_192;
const MAX_DISPLACEMENT_EXCLUSIVE: i32 = 8_192;
const MAX_SAD: i32 = 65_280;
const PENALTY_NEW: i64 = 50;
const PENALTY_ZERO: i64 = 50;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    Geometry,
    Length {
        which: &'static str,
        expected: usize,
        actual: usize,
    },
    SeedCount {
        expected: usize,
        actual: usize,
    },
    SeedValue {
        index: usize,
    },
    GlobalValue,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Geometry => formatter.write_str(
                "parallel finest refinement needs matching dimensions from 1024 through 8191",
            ),
            Self::Length {
                which,
                expected,
                actual,
            } => write!(
                formatter,
                "parallel finest {which} has {actual} pixels but needs {expected}"
            ),
            Self::SeedCount { expected, actual } => write!(
                formatter,
                "parallel finest refinement has {actual} seeds but needs {expected}"
            ),
            Self::SeedValue { index } => write!(
                formatter,
                "parallel finest seed {index} is outside the selected safe range"
            ),
            Self::GlobalValue => formatter
                .write_str("parallel finest global predictor is outside the selected safe range"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Vector {
    x: i32,
    y: i32,
    sad: i32,
}

/// Refine one finest motion plane with independent immutable predictors.
///
/// Output records are row-major `(dx, dy, unpenalized 16x16 SAD)`. Blocks
/// cover only complete 16x16 cells when a permitted dimension is not a
/// multiple of sixteen, matching the supplied finest seed grid.
pub fn finest(
    current: &Level,
    reference: &Level,
    input: &FinestInput,
) -> Result<Vec<[i32; 3]>, Error> {
    let [blocks_x, blocks_y] = validate(current, reference, input)?;
    refine_plane(current, reference, input, [blocks_x, blocks_y], false, 1)
}

/// Shared independent-block controller. `smallest` selects the recovered
/// smallest-plane predictor rule; `radius` is one for finest Hex2 and two for
/// coarse ExhaustiveTwo. Callers validate their own level contract first.
pub(super) fn refine_plane(
    current: &Level,
    reference: &Level,
    input: &FinestInput,
    [blocks_x, blocks_y]: [usize; 2],
    smallest: bool,
    radius: i32,
) -> Result<Vec<[i32; 3]>, Error> {
    let seeds: Vec<Vector> = input
        .seeds
        .iter()
        .map(|record| Vector {
            x: record[0],
            y: record[1],
            sad: record[2],
        })
        .collect();
    let global = Vector {
        x: input.global[0],
        y: input.global[1],
        sad: -1,
    };
    let mut output = Vec::with_capacity(seeds.len());

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let index = block_y * blocks_x + block_x;
            let bounds = Bounds::new(current, block_x, block_y);
            let zero = bounds.clip(Vector::default());
            let coarse = bounds.clip(seeds[index]);
            let left = if block_x > 0 {
                bounds.clip(seeds[index - 1])
            } else {
                zero
            };
            let up = if block_y > 0 {
                bounds.clip(seeds[index - blocks_x])
            } else {
                zero
            };
            // The selected smallest plane never reads the future bottom-right
            // predictor. Other planes prefer it, then the top-right edge seed.
            let diagonal = if !smallest && block_y + 1 < blocks_y && block_x + 1 < blocks_x {
                bounds.clip(seeds[index + blocks_x + 1])
            } else if block_y > 0 && block_x + 1 < blocks_x {
                bounds.clip(seeds[index - blocks_x + 1])
            } else {
                zero
            };
            let median = if block_y > 0 {
                Vector {
                    x: median3(left.x, up.x, diagonal.x),
                    y: median3(left.y, up.y, diagonal.y),
                    sad: left.sad.max(up.sad).max(diagonal.sad),
                }
            } else {
                left
            };

            let mut search = Search::new(current, reference, block_x, block_y, bounds);
            // These first three are inclusive clipped starts. Strictly equal
            // costs retain the earlier candidate.
            search.start(zero, PENALTY_ZERO);
            search.start(bounds.clip(global), 0);
            search.start(if smallest { median } else { coarse }, 0);
            for predictor in [median, left, up, diagonal] {
                search.checked(predictor, 0);
            }

            let center = search.best;
            for ring in 1..=radius {
                search.expanding(ring, center);
            }
            output.push([search.best.x, search.best.y, search.best.sad]);
        }
    }
    Ok(output)
}

fn validate(current: &Level, reference: &Level, input: &FinestInput) -> Result<[usize; 2], Error> {
    validate_with_minimum(current, reference, input, MIN_DIMENSION)
}

pub(super) fn validate_coarse_plane(
    current: &Level,
    reference: &Level,
    input: &FinestInput,
) -> Result<[usize; 2], Error> {
    validate_with_minimum(current, reference, input, BLOCK)
}

fn validate_with_minimum(
    current: &Level,
    reference: &Level,
    input: &FinestInput,
    minimum: usize,
) -> Result<[usize; 2], Error> {
    if current.width != reference.width
        || current.height != reference.height
        || !(minimum..MAX_DIMENSION_EXCLUSIVE).contains(&current.width)
        || !(minimum..MAX_DIMENSION_EXCLUSIVE).contains(&current.height)
    {
        return Err(Error::Geometry);
    }
    let expected = current
        .width
        .checked_mul(current.height)
        .ok_or(Error::Geometry)?;
    for (which, level) in [("current", current), ("reference", reference)] {
        if level.pixels.len() != expected {
            return Err(Error::Length {
                which,
                expected,
                actual: level.pixels.len(),
            });
        }
    }
    let blocks = [current.width / BLOCK, current.height / BLOCK];
    let count = blocks[0].checked_mul(blocks[1]).ok_or(Error::Geometry)?;
    if input.seeds.len() != count {
        return Err(Error::SeedCount {
            expected: count,
            actual: input.seeds.len(),
        });
    }
    for (index, &[dx, dy, sad]) in input.seeds.iter().enumerate() {
        if !safe_displacement(dx) || !safe_displacement(dy) || !(0..=MAX_SAD).contains(&sad) {
            return Err(Error::SeedValue { index });
        }
    }
    if input
        .global
        .into_iter()
        .any(|value| !safe_displacement(value))
    {
        return Err(Error::GlobalValue);
    }
    Ok(blocks)
}

fn safe_displacement(value: i32) -> bool {
    (MIN_DISPLACEMENT..MAX_DISPLACEMENT_EXCLUSIVE).contains(&value)
}

#[derive(Clone, Copy)]
struct Bounds {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

impl Bounds {
    fn new(level: &Level, block_x: usize, block_y: usize) -> Self {
        let x = (block_x * BLOCK) as i32;
        let y = (block_y * BLOCK) as i32;
        Self {
            min_x: -x,
            max_x: level.width as i32 - x - BLOCK as i32,
            min_y: -y,
            max_y: level.height as i32 - y - BLOCK as i32,
        }
    }

    fn clip(self, vector: Vector) -> Vector {
        Vector {
            x: vector.x.clamp(self.min_x, self.max_x),
            y: vector.y.clamp(self.min_y, self.max_y),
            sad: vector.sad,
        }
    }

    fn candidate(self, vector: Vector) -> bool {
        vector.x >= self.min_x
            && vector.x < self.max_x
            && vector.y >= self.min_y
            && vector.y < self.max_y
    }
}

struct Search<'a> {
    current: &'a Level,
    reference: &'a Level,
    source: [usize; 2],
    bounds: Bounds,
    best: Vector,
    minimum_cost: i64,
}

impl<'a> Search<'a> {
    fn new(
        current: &'a Level,
        reference: &'a Level,
        block_x: usize,
        block_y: usize,
        bounds: Bounds,
    ) -> Self {
        Self {
            current,
            reference,
            source: [block_x * BLOCK, block_y * BLOCK],
            bounds,
            best: Vector::default(),
            minimum_cost: i64::MAX,
        }
    }

    fn start(&mut self, vector: Vector, penalty: i64) {
        self.consider(vector, penalty);
    }

    fn checked(&mut self, vector: Vector, penalty: i64) {
        if self.bounds.candidate(vector) {
            self.consider(vector, penalty);
        }
    }

    fn expanding(&mut self, radius: i32, center: Vector) {
        for value in (-radius + 1)..radius {
            self.checked(
                Vector {
                    x: center.x + value,
                    y: center.y - radius,
                    sad: -1,
                },
                PENALTY_NEW,
            );
            self.checked(
                Vector {
                    x: center.x + value,
                    y: center.y + radius,
                    sad: -1,
                },
                PENALTY_NEW,
            );
        }
        for value in (-radius + 1)..radius {
            self.checked(
                Vector {
                    x: center.x - radius,
                    y: center.y + value,
                    sad: -1,
                },
                PENALTY_NEW,
            );
            self.checked(
                Vector {
                    x: center.x + radius,
                    y: center.y + value,
                    sad: -1,
                },
                PENALTY_NEW,
            );
        }
        for [x, y] in [
            [-radius, -radius],
            [-radius, radius],
            [radius, -radius],
            [radius, radius],
        ] {
            self.checked(
                Vector {
                    x: center.x + x,
                    y: center.y + y,
                    sad: -1,
                },
                PENALTY_NEW,
            );
        }
    }

    fn consider(&mut self, vector: Vector, penalty: i64) {
        let sad = self.sad(vector.x, vector.y);
        let cost = sad + ((penalty * sad) >> 8);
        if cost < self.minimum_cost {
            self.best = Vector {
                x: vector.x,
                y: vector.y,
                sad: sad as i32,
            };
            self.minimum_cost = cost;
        }
    }

    fn sad(&self, dx: i32, dy: i32) -> i64 {
        let reference_x = (self.source[0] as i32 + dx) as usize;
        let reference_y = (self.source[1] as i32 + dy) as usize;
        let mut sad = 0_i64;
        for row in 0..BLOCK {
            let current_at = (self.source[1] + row) * self.current.width + self.source[0];
            let reference_at = (reference_y + row) * self.reference.width + reference_x;
            for column in 0..BLOCK {
                sad += (i64::from(self.current.pixels[current_at + column])
                    - i64::from(self.reference.pixels[reference_at + column]))
                .abs();
            }
        }
        sad
    }
}

fn median3(a: i32, b: i32, c: i32) -> i32 {
    a.min(b).max(a.max(b).min(c))
}

#[cfg(test)]
mod tests;
