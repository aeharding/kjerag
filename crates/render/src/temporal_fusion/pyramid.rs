//! Readable CPU reference for Studio's selected temporal luma pyramid.
//!
//! This owns only pyramid arithmetic. Callers explicitly supply the base
//! dimensions and level count; frame history and playback policy live outside
//! this module.

use std::fmt;

/// Render-pass preparation of the same explicit pyramid arithmetic.
pub mod gpu;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Level {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    EmptyDimensions,
    EmptyPyramid,
    DimensionOverflow,
    Length {
        expected: usize,
        actual: usize,
    },
    EmptyReduction {
        level: usize,
        width: usize,
        height: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::EmptyDimensions => formatter.write_str("pyramid dimensions must be nonzero"),
            Self::EmptyPyramid => formatter.write_str("pyramid level count must be nonzero"),
            Self::DimensionOverflow => formatter.write_str("pyramid dimensions overflow usize"),
            Self::Length { expected, actual } => write!(
                formatter,
                "base plane has {actual} bytes but its dimensions require {expected}"
            ),
            Self::EmptyReduction {
                level,
                width,
                height,
            } => write!(
                formatter,
                "pyramid level {level} is {width}x{height}; reduction would produce an empty level"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Construct logical levels with Studio's selected separable reduction.
///
/// Each axis uses `[1, 3, 3, 1] / 8` in the interior and a two-sample average
/// at its first and last output. An unmatched final sample on an odd axis is
/// discarded. The vertical result is rounded to `u8` before the horizontal
/// pass, matching the native in-place implementation.
pub fn build(
    base: &[u8],
    base_width: usize,
    base_height: usize,
    level_count: usize,
) -> Result<Vec<Level>, Error> {
    if base_width == 0 || base_height == 0 {
        return Err(Error::EmptyDimensions);
    }
    if level_count == 0 {
        return Err(Error::EmptyPyramid);
    }
    let expected = base_width
        .checked_mul(base_height)
        .ok_or(Error::DimensionOverflow)?;
    if base.len() != expected {
        return Err(Error::Length {
            expected,
            actual: base.len(),
        });
    }

    // Geometry bounds the possible levels. Do not reserve an unchecked count
    // before rejecting an impossible reduction request.
    let mut levels = Vec::new();
    levels.push(Level {
        width: base_width,
        height: base_height,
        pixels: base.to_vec(),
    });
    while levels.len() < level_count {
        let source = levels.last().expect("the base level was inserted");
        if source.width / 2 == 0 || source.height / 2 == 0 {
            return Err(Error::EmptyReduction {
                level: levels.len() - 1,
                width: source.width,
                height: source.height,
            });
        }
        levels.push(reduce(source));
    }
    Ok(levels)
}

fn reduce(source: &Level) -> Level {
    let destination_width = source.width / 2;
    let destination_height = source.height / 2;
    let mut vertical = vec![0_u8; source.width * destination_height];

    for x in 0..source.width {
        vertical[x] = rounded_half(source.pixels[x], source.pixels[source.width + x]);
        for y in 1..destination_height.saturating_sub(1) {
            vertical[y * source.width + x] = filtered(
                source.pixels[(2 * y - 1) * source.width + x],
                source.pixels[2 * y * source.width + x],
                source.pixels[(2 * y + 1) * source.width + x],
                source.pixels[(2 * y + 2) * source.width + x],
            );
        }
        if destination_height > 1 {
            let source_y = 2 * (destination_height - 1);
            vertical[(destination_height - 1) * source.width + x] = rounded_half(
                source.pixels[source_y * source.width + x],
                source.pixels[(source_y + 1) * source.width + x],
            );
        }
    }

    let mut pixels = vec![0_u8; destination_width * destination_height];
    for y in 0..destination_height {
        let source_row = &vertical[y * source.width..(y + 1) * source.width];
        let destination_row = &mut pixels[y * destination_width..(y + 1) * destination_width];
        destination_row[0] = rounded_half(source_row[0], source_row[1]);
        for x in 1..destination_width.saturating_sub(1) {
            destination_row[x] = filtered(
                source_row[2 * x - 1],
                source_row[2 * x],
                source_row[2 * x + 1],
                source_row[2 * x + 2],
            );
        }
        if destination_width > 1 {
            let source_x = 2 * (destination_width - 1);
            destination_row[destination_width - 1] =
                rounded_half(source_row[source_x], source_row[source_x + 1]);
        }
    }

    Level {
        width: destination_width,
        height: destination_height,
        pixels,
    }
}

fn rounded_half(a: u8, b: u8) -> u8 {
    ((u16::from(a) + u16::from(b) + 1) >> 1) as u8
}

fn filtered(a: u8, b: u8, c: u8, d: u8) -> u8 {
    ((u16::from(a) + 3 * u16::from(b) + 3 * u16::from(c) + u16::from(d) + 4) >> 3) as u8
}

#[cfg(test)]
mod tests;
