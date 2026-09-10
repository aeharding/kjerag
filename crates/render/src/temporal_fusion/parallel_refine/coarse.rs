// SPDX-License-Identifier: GPL-3.0-or-later
// Author: Manao
// Copyright(c)2006 A.G.Balakhnin aka Fizick - global motion, overlap, mode, refineMVs
//! Readable CPU oracle for independently parallel coarse motion search.
//!
//! Each block reads only one immutable same-plane seed grid. The recovered
//! inclusive starts, strict tie order, penalties, smallest-plane predictor
//! rule, fixed-center ExhaustiveTwo rings, level order, global estimator and
//! interpolation remain unchanged. The serial controller's searched-neighbor
//! mutation, row-major bad-block count and adaptive UMH recovery are
//! deliberately omitted. This is a disclosed Kjerag candidate, not Studio
//! parity and not a new frame-history or cadence policy.

pub mod gpu;
pub mod prepare;

use super::{BLOCK, Error as RefineError, refine_plane, validate_coarse_plane};
use crate::temporal_fusion::{
    pyramid::Level,
    search::{self, FinestInput},
};
use std::fmt;

const LEVELS: usize = 7;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    Search(search::Error),
    Refine(RefineError),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Search(error) => fmt::Display::fmt(error, formatter),
            Self::Refine(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for Error {}

impl From<search::Error> for Error {
    fn from(error: search::Error) -> Self {
        Self::Search(error)
    }
}

impl From<RefineError> for Error {
    fn from(error: RefineError) -> Self {
        Self::Refine(error)
    }
}

/// Search levels six through one and return immutable seeds plus the exact
/// doubled global predictor for the existing finest-level candidate.
pub fn prepare_finest(current: &[Level], reference: &[Level]) -> Result<FinestInput, Error> {
    search::validate_selected_pyramids(current, reference)?;

    let mut previous: Option<Vec<[i32; 3]>> = None;
    let mut previous_blocks = [0, 0];
    for level in (1..LEVELS).rev() {
        let blocks = [current[level].width / BLOCK, current[level].height / BLOCK];
        let input = if let Some(records) = previous.as_deref() {
            prepare_next(records, previous_blocks, blocks)?
        } else {
            FinestInput {
                seeds: vec![[0, 0, 0]; blocks[0] * blocks[1]],
                global: [0, 0],
            }
        };
        previous = Some(refine_level(
            &current[level],
            &reference[level],
            &input,
            level == LEVELS - 1,
        )?);
        previous_blocks = blocks;
    }

    let finest_blocks = [current[0].width / BLOCK, current[0].height / BLOCK];
    prepare_next(
        previous
            .as_deref()
            .expect("validated seven-level pyramid searches six coarse levels"),
        previous_blocks,
        finest_blocks,
    )
}

/// Refine one coarse plane with immutable predictors and fixed-center
/// ExhaustiveTwo. Public within the crate for exact GPU per-level comparisons.
pub(crate) fn refine_level(
    current: &Level,
    reference: &Level,
    input: &FinestInput,
    smallest: bool,
) -> Result<Vec<[i32; 3]>, Error> {
    let blocks = validate_coarse_plane(current, reference, input)?;
    Ok(refine_plane(
        current, reference, input, blocks, smallest, 2,
    )?)
}

/// Derive the next plane's immutable seeds and exact doubled global predictor
/// from one complete raw coarse field. This is the CPU comparison boundary for
/// the GPU preparation passes.
pub(crate) fn prepare_next(
    records: &[[i32; 3]],
    coarse_blocks: [usize; 2],
    next_blocks: [usize; 2],
) -> Result<FinestInput, Error> {
    Ok(FinestInput {
        seeds: search::interpolate_raw_records(records, coarse_blocks, next_blocks)?,
        global: search::estimate_global_doubled_raw(records)?,
    })
}

#[cfg(test)]
mod tests;
