// SPDX-License-Identifier: GPL-3.0-or-later
// Author: Manao
// Copyright(c)2006 A.G.Balakhnin aka Fizick - global motion, overlap,  mode, refineMVs
// Author: Manao
// Copyright(c)2006 A.G.Balakhnin aka Fizick - overlap, global MV, divide
//! Readable CPU reference for the selected temporal motion search.
//!
//! The search order and arithmetic derive from MVTools, commit
//! `17250aa979616ac48dfb0e18abfdcf2bd4e3afc0`, by Manao and A.G. Balakhnin
//! (Fizick). The source permits GPL-2.0-or-later; this adaptation elects
//! GPL-3.0-or-later for compatibility with Kjerag. It also includes three
//! independently authenticated native-path differences: an immutable per-plane
//! global predictor, inclusive clipping of predictor seeds, and no bottom-right
//! predictor on the smallest plane.
//! The complete elected license is in `temporal_fusion/LICENSE-MVTOOLS`.
//!
//! This is deliberately only the selected pel-1, gray, 16x16, no-overlap,
//! seven-level path. It owns neither frame history nor scheduling and is not
//! used by playback.

use super::pyramid::Level;
use std::fmt;

const LEVELS: usize = 7;
const BLOCK: usize = 16;
const PENALTY_NEW: i64 = 50;
const PENALTY_ZERO: i64 = 50;
const PENALTY_GLOBAL: i64 = 0;
const BAD_SAD: i64 = 40_000;
const BAD_RANGE: i32 = 24;
const FREQUENCY_SIZE: usize = 16_384;

/// The single configuration implemented by [`selected`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectedConfiguration {
    pub levels: usize,
    pub block: [usize; 2],
    pub finest_search: &'static str,
    pub finest_radius: i32,
    pub coarse_search: &'static str,
    pub coarse_radius: i32,
    pub lsad: i64,
    pub penalty_new: i64,
    pub penalty_zero: i64,
    pub penalty_global: i64,
    pub bad_sad: i64,
    pub bad_range: i32,
}

pub const SELECTED_CONFIGURATION: SelectedConfiguration = SelectedConfiguration {
    levels: LEVELS,
    block: [BLOCK, BLOCK],
    finest_search: "Hex2",
    finest_radius: 1,
    coarse_search: "Exhaustive",
    coarse_radius: 2,
    lsad: 1_600,
    penalty_new: PENALTY_NEW,
    penalty_zero: PENALTY_ZERO,
    penalty_global: PENALTY_GLOBAL,
    bad_sad: BAD_SAD,
    bad_range: BAD_RANGE,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Vector {
    x: i32,
    y: i32,
    sad: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    LevelCount {
        current: usize,
        reference: usize,
    },
    Geometry {
        level: usize,
        message: &'static str,
    },
    Length {
        level: usize,
        expected: usize,
        actual: usize,
    },
    ArithmeticOverflow,
    OutputCostOverflow(i64),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LevelCount { current, reference } => write!(
                formatter,
                "selected motion search requires seven levels, got {current} current and {reference} reference"
            ),
            Self::Geometry { level, message } => {
                write!(formatter, "motion pyramid level {level} {message}")
            }
            Self::Length {
                level,
                expected,
                actual,
            } => write!(
                formatter,
                "motion pyramid level {level} has {actual} pixels but requires {expected}"
            ),
            Self::ArithmeticOverflow => formatter.write_str("motion-search geometry overflows"),
            Self::OutputCostOverflow(value) => {
                write!(formatter, "motion-search SAD {value} does not fit i32")
            }
        }
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy)]
enum Refine {
    ExhaustiveTwo,
    HexTwoOne,
}

struct Plane<'a> {
    current: &'a Level,
    reference: &'a Level,
    blocks_x: usize,
    blocks_y: usize,
    vectors: Vec<Vector>,
    smallest: bool,
    global: Vector,
    bad_count: i64,
}

/// Run the selected deterministic serial motion search.
///
/// The result is the finest grid in row-major order. Each record contains
/// `(dx, dy, unpenalized 16x16 luma SAD)`. Unsupported level layouts are
/// rejected rather than assigned semantics from another search mode.
pub fn selected(current: &[Level], reference: &[Level]) -> Result<Vec<[i32; 3]>, Error> {
    validate(current, reference)?;
    let mut previous: Option<Vec<Vector>> = None;
    let mut previous_shape = [0, 0];
    let mut global = Vector {
        x: 0,
        y: 0,
        sad: -1,
    };

    for level_index in (0..LEVELS).rev() {
        let current_level = &current[level_index];
        let reference_level = &reference[level_index];
        let blocks_x = current_level.width / BLOCK;
        let blocks_y = current_level.height / BLOCK;
        let smallest = level_index == LEVELS - 1;
        let vectors = if let Some(coarse) = previous.as_deref() {
            global = estimate_global_doubled(coarse)?;
            interpolate(coarse, previous_shape, [blocks_x, blocks_y])?
        } else {
            vec![Vector::default(); blocks_x * blocks_y]
        };
        let mut plane = Plane {
            current: current_level,
            reference: reference_level,
            blocks_x,
            blocks_y,
            vectors,
            smallest,
            global,
            bad_count: 0,
        };
        plane.search(if level_index == 0 {
            Refine::HexTwoOne
        } else {
            Refine::ExhaustiveTwo
        })?;
        previous_shape = [blocks_x, blocks_y];
        previous = Some(plane.vectors);
    }

    previous
        .expect("seven validated levels were searched")
        .into_iter()
        .map(|vector| {
            Ok([
                vector.x,
                vector.y,
                i32::try_from(vector.sad).map_err(|_| Error::OutputCostOverflow(vector.sad))?,
            ])
        })
        .collect()
}

fn validate(current: &[Level], reference: &[Level]) -> Result<(), Error> {
    if current.len() != LEVELS || reference.len() != LEVELS {
        return Err(Error::LevelCount {
            current: current.len(),
            reference: reference.len(),
        });
    }
    let base = [current[0].width, current[0].height];
    if base
        .into_iter()
        .any(|dimension| dimension >= FREQUENCY_SIZE / 2)
    {
        return Err(Error::Geometry {
            level: 0,
            message: "exceeds the selected predictor histogram range",
        });
    }
    if base[0] < BLOCK << (LEVELS - 1) || base[1] < BLOCK << (LEVELS - 1) {
        return Err(Error::Geometry {
            level: 0,
            message: "is too small for seven 16x16 search grids",
        });
    }
    for level in 0..LEVELS {
        let expected_dimensions = [base[0] >> level, base[1] >> level];
        for candidate in [&current[level], &reference[level]] {
            if [candidate.width, candidate.height] != expected_dimensions {
                return Err(Error::Geometry {
                    level,
                    message: "does not have the selected halved dimensions",
                });
            }
            let expected = candidate
                .width
                .checked_mul(candidate.height)
                .ok_or(Error::ArithmeticOverflow)?;
            if candidate.pixels.len() != expected {
                return Err(Error::Length {
                    level,
                    expected,
                    actual: candidate.pixels.len(),
                });
            }
            if candidate.width > i32::MAX as usize || candidate.height > i32::MAX as usize {
                return Err(Error::Geometry {
                    level,
                    message: "dimensions do not fit selected signed coordinates",
                });
            }
        }
    }
    Ok(())
}

impl Plane<'_> {
    fn search(&mut self, refine: Refine) -> Result<(), Error> {
        let supplied_global = self.global;
        for block_y in 0..self.blocks_y {
            for block_x in 0..self.blocks_x {
                let index = block_y * self.blocks_x + block_x;
                let bounds = Bounds::new(self.current, block_x, block_y)?;
                let coarse_seed = bounds.clip(self.vectors[index]);
                let (predictor, predictors) =
                    self.predictors(block_x, block_y, bounds, coarse_seed);
                let source = [block_x * BLOCK, block_y * BLOCK];
                let mut search = BlockSearch {
                    source,
                    current: self.current,
                    reference: self.reference,
                    bounds,
                    best: Vector::default(),
                    minimum_cost: i64::MAX,
                };

                search.start_seed(Vector::default(), PENALTY_ZERO)?;
                search.start_seed(bounds.clip(supplied_global), PENALTY_GLOBAL)?;
                search.start_seed(predictor, 0)?;
                for predictor in predictors {
                    search.check(predictor.x, predictor.y, false, true)?;
                }

                match refine {
                    Refine::ExhaustiveTwo => search.exhaustive_two()?,
                    Refine::HexTwoOne => search.expanding(1, 1, search.best.x, search.best.y)?,
                }
                let found_sad = search.best.sad;
                if index > 1 && found_sad > BAD_SAD + BAD_SAD * self.bad_count / 16 {
                    self.bad_count += 1;
                    search.uneven_multi_hexagon(BAD_RANGE)?;
                }
                self.vectors[index] = search.best;
            }
        }
        Ok(())
    }

    fn predictors(
        &self,
        x: usize,
        y: usize,
        bounds: Bounds,
        coarse_seed: Vector,
    ) -> (Vector, [Vector; 4]) {
        let zero = Vector::default();
        let left = if x > 0 {
            bounds.clip(self.vectors[y * self.blocks_x + x - 1])
        } else {
            bounds.clip(zero)
        };
        let up = if y > 0 {
            bounds.clip(self.vectors[(y - 1) * self.blocks_x + x])
        } else {
            bounds.clip(zero)
        };
        let diagonal = if !self.smallest && y + 1 < self.blocks_y && x + 1 < self.blocks_x {
            bounds.clip(self.vectors[(y + 1) * self.blocks_x + x + 1])
        } else if y > 0 && x + 1 < self.blocks_x {
            bounds.clip(self.vectors[(y - 1) * self.blocks_x + x + 1])
        } else {
            bounds.clip(zero)
        };
        let median = if y > 0 {
            Vector {
                x: median(left.x, up.x, diagonal.x),
                y: median(left.y, up.y, diagonal.y),
                sad: left.sad.max(up.sad).max(diagonal.sad),
            }
        } else {
            left
        };
        let selected_seed = if self.smallest { median } else { coarse_seed };
        (selected_seed, [median, left, up, diagonal])
    }
}

#[derive(Clone, Copy)]
struct Bounds {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

impl Bounds {
    fn new(level: &Level, block_x: usize, block_y: usize) -> Result<Self, Error> {
        let x = i32::try_from(
            block_x
                .checked_mul(BLOCK)
                .ok_or(Error::ArithmeticOverflow)?,
        )
        .map_err(|_| Error::ArithmeticOverflow)?;
        let y = i32::try_from(
            block_y
                .checked_mul(BLOCK)
                .ok_or(Error::ArithmeticOverflow)?,
        )
        .map_err(|_| Error::ArithmeticOverflow)?;
        let width = i32::try_from(level.width).map_err(|_| Error::ArithmeticOverflow)?;
        let height = i32::try_from(level.height).map_err(|_| Error::ArithmeticOverflow)?;
        Ok(Self {
            min_x: -x,
            max_x: width - x - BLOCK as i32,
            min_y: -y,
            max_y: height - y - BLOCK as i32,
        })
    }

    fn clip(self, vector: Vector) -> Vector {
        Vector {
            x: vector.x.clamp(self.min_x, self.max_x),
            y: vector.y.clamp(self.min_y, self.max_y),
            sad: vector.sad,
        }
    }

    fn candidate(self, x: i32, y: i32) -> bool {
        x >= self.min_x && x < self.max_x && y >= self.min_y && y < self.max_y
    }
}

struct BlockSearch<'a> {
    source: [usize; 2],
    current: &'a Level,
    reference: &'a Level,
    bounds: Bounds,
    best: Vector,
    minimum_cost: i64,
}

impl BlockSearch<'_> {
    fn start_seed(&mut self, vector: Vector, penalty: i64) -> Result<(), Error> {
        let sad = self.block_sad(vector.x, vector.y)?;
        let cost = sad + ((penalty * sad) >> 8);
        if cost < self.minimum_cost {
            self.best = Vector { sad, ..vector };
            self.minimum_cost = cost;
        }
        Ok(())
    }

    fn check(&mut self, x: i32, y: i32, penalize: bool, update_best: bool) -> Result<bool, Error> {
        if !self.bounds.candidate(x, y) {
            return Ok(false);
        }
        let sad = self.block_sad(x, y)?;
        let cost = sad
            + if penalize {
                (PENALTY_NEW * sad) >> 8
            } else {
                0
            };
        if cost >= self.minimum_cost {
            return Ok(false);
        }
        if update_best {
            self.best.x = x;
            self.best.y = y;
        }
        self.best.sad = sad;
        self.minimum_cost = cost;
        Ok(true)
    }

    fn block_sad(&self, x: i32, y: i32) -> Result<i64, Error> {
        let reference_x = usize::try_from(self.source[0] as i64 + i64::from(x))
            .map_err(|_| Error::ArithmeticOverflow)?;
        let reference_y = usize::try_from(self.source[1] as i64 + i64::from(y))
            .map_err(|_| Error::ArithmeticOverflow)?;
        let mut sad = 0_i64;
        for row in 0..BLOCK {
            let current_start = (self.source[1] + row) * self.current.width + self.source[0];
            let reference_start = (reference_y + row) * self.reference.width + reference_x;
            for column in 0..BLOCK {
                sad += (i64::from(self.current.pixels[current_start + column])
                    - i64::from(self.reference.pixels[reference_start + column]))
                .abs();
            }
        }
        Ok(sad)
    }

    fn expanding(
        &mut self,
        radius: i32,
        step: i32,
        center_x: i32,
        center_y: i32,
    ) -> Result<(), Error> {
        let mut value = -radius + step;
        while value < radius {
            self.check(center_x + value, center_y - radius, true, true)?;
            self.check(center_x + value, center_y + radius, true, true)?;
            value += step;
        }
        value = -radius + step;
        while value < radius {
            self.check(center_x - radius, center_y + value, true, true)?;
            self.check(center_x + radius, center_y + value, true, true)?;
            value += step;
        }
        for (x, y) in [
            (-radius, -radius),
            (-radius, radius),
            (radius, -radius),
            (radius, radius),
        ] {
            self.check(center_x + x, center_y + y, true, true)?;
        }
        Ok(())
    }

    fn exhaustive_two(&mut self) -> Result<(), Error> {
        let center = [self.best.x, self.best.y];
        self.expanding(1, 1, center[0], center[1])?;
        self.expanding(2, 1, center[0], center[1])
    }

    fn uneven_multi_hexagon(&mut self, range: i32) -> Result<(), Error> {
        for value in (1..range).step_by(2) {
            self.check(-value, 0, true, true)?;
            self.check(value, 0, true, true)?;
        }
        for value in (1..range).step_by(2) {
            self.check(0, -value, true, true)?;
            self.check(0, value, true, true)?;
        }
        const HEX4: [[i32; 2]; 16] = [
            [-4, 2],
            [-4, 1],
            [-4, 0],
            [-4, -1],
            [-4, -2],
            [4, -2],
            [4, -1],
            [4, 0],
            [4, 1],
            [4, 2],
            [2, 3],
            [0, 4],
            [-2, 3],
            [-2, -3],
            [0, -4],
            [2, -3],
        ];
        for scale in 1..=range / 4 {
            for offset in HEX4 {
                self.check(offset[0] * scale, offset[1] * scale, true, true)?;
            }
        }
        self.hex_two(range)
    }

    fn hex_two(&mut self, range: i32) -> Result<(), Error> {
        const HEX: [[i32; 2]; 8] = [
            [-1, -2],
            [-2, 0],
            [-1, 2],
            [1, 2],
            [2, 0],
            [1, -2],
            [-1, -2],
            [-2, 0],
        ];
        const MOD_MINUS_ONE: [i32; 8] = [5, 0, 1, 2, 3, 4, 5, 0];
        let mut direction = -2;
        let mut center = [self.best.x, self.best.y];
        if range > 1 {
            for (index, offset) in HEX[1..7].iter().enumerate() {
                if self.check(center[0] + offset[0], center[1] + offset[1], true, false)? {
                    direction = index as i32;
                }
            }
            if direction != -2 {
                center[0] += HEX[(direction + 1) as usize][0];
                center[1] += HEX[(direction + 1) as usize][1];
                for _ in 1..range / 2 {
                    if !self.bounds.candidate(center[0], center[1]) {
                        break;
                    }
                    let old_direction = MOD_MINUS_ONE[(direction + 1) as usize];
                    direction = -2;
                    for offset_index in 0..3 {
                        let candidate_direction = old_direction + offset_index as i32 - 1;
                        let offset = HEX[(old_direction as usize) + offset_index];
                        if self.check(center[0] + offset[0], center[1] + offset[1], true, false)? {
                            direction = candidate_direction;
                        }
                    }
                    if direction == -2 {
                        break;
                    }
                    center[0] += HEX[(direction + 1) as usize][0];
                    center[1] += HEX[(direction + 1) as usize][1];
                }
            }
            self.best.x = center[0];
            self.best.y = center[1];
        }
        self.expanding(1, 1, center[0], center[1])
    }
}

fn median(a: i32, b: i32, c: i32) -> i32 {
    a.min(b).max(a.max(b).min(c))
}

fn interpolate(
    coarse: &[Vector],
    coarse_shape: [usize; 2],
    shape: [usize; 2],
) -> Result<Vec<Vector>, Error> {
    let [coarse_width, coarse_height] = coarse_shape;
    if coarse.len()
        != coarse_width
            .checked_mul(coarse_height)
            .ok_or(Error::ArithmeticOverflow)?
    {
        return Err(Error::ArithmeticOverflow);
    }
    let mut output = Vec::with_capacity(
        shape[0]
            .checked_mul(shape[1])
            .ok_or(Error::ArithmeticOverflow)?,
    );
    for y in 0..shape[1] {
        for x in 0..shape[0] {
            let i = x.min(2 * coarse_width - 1);
            let j = y.min(2 * coarse_height - 1);
            let offset_x = if i % 2 == 0 { -1 } else { 1 };
            let offset_y = if j % 2 == 0 { -1 } else { 1 };
            let at = |cx: isize, cy: isize| coarse[cy as usize * coarse_width + cx as usize];
            let base_x = (i / 2) as isize;
            let base_y = (j / 2) as isize;
            let (a, b, c, d) = if i == 0 || i >= 2 * coarse_width - 1 {
                if j == 0 || j >= 2 * coarse_height - 1 {
                    let value = at(base_x, base_y);
                    (value, value, value, value)
                } else {
                    let top = at(base_x, base_y);
                    let bottom = at(base_x, base_y + offset_y);
                    (top, top, bottom, bottom)
                }
            } else if j == 0 || j >= 2 * coarse_height - 1 {
                let left = at(base_x, base_y);
                let right = at(base_x + offset_x, base_y);
                (left, left, right, right)
            } else {
                (
                    at(base_x, base_y),
                    at(base_x + offset_x, base_y),
                    at(base_x, base_y + offset_y),
                    at(base_x + offset_x, base_y + offset_y),
                )
            };
            output.push(Vector {
                x: (9 * a.x + 3 * b.x + 3 * c.x + d.x) >> 3,
                y: (9 * a.y + 3 * b.y + 3 * c.y + d.y) >> 3,
                sad: (9 * a.sad + 3 * b.sad + 3 * c.sad + d.sad + 8) >> 4,
            });
        }
    }
    Ok(output)
}

fn estimate_global_doubled(vectors: &[Vector]) -> Result<Vector, Error> {
    let mode = |pick: fn(Vector) -> i32| -> Result<i32, Error> {
        let mut frequency = vec![0_u32; FREQUENCY_SIZE];
        let midpoint = (FREQUENCY_SIZE / 2) as i32;
        for &vector in vectors {
            let index = midpoint + pick(vector);
            if let Ok(index) = usize::try_from(index)
                && index < FREQUENCY_SIZE
            {
                frequency[index] += 1;
            }
        }
        let (mode_index, _) = frequency
            .iter()
            .enumerate()
            .filter(|(_, count)| **count != 0)
            .max_by_key(|(index, count)| (**count, std::cmp::Reverse(*index)))
            .ok_or(Error::ArithmeticOverflow)?;
        Ok(mode_index as i32 - midpoint)
    };
    let mode_x = mode(|vector| vector.x)?;
    let mode_y = mode(|vector| vector.y)?;
    let mut sum_x = 0_i64;
    let mut sum_y = 0_i64;
    let mut count = 0_i64;
    for &vector in vectors {
        if (vector.x - mode_x).abs() < 6 && (vector.y - mode_y).abs() < 6 {
            sum_x += i64::from(vector.x);
            sum_y += i64::from(vector.y);
            count += 1;
        }
    }
    let doubled = |sum: i64, mode: i32| {
        let value = if count > 0 {
            2 * sum / count
        } else {
            2 * i64::from(mode)
        };
        i32::try_from(value).map_err(|_| Error::ArithmeticOverflow)
    };
    Ok(Vector {
        x: doubled(sum_x, mode_x)?,
        y: doubled(sum_y, mode_y)?,
        sad: -1,
    })
}

#[cfg(test)]
mod tests;
