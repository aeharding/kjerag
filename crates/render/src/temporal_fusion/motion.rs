//! Readable CPU reference for Studio's selected motion-grid expansion and
//! confidence packing. This is an oracle, not the playback implementation.

/// Render-pass implementation of the captured nonnegative-cost subset.
pub mod gpu;

/// Geometry of the selected two-times motion-grid expansion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub full: [u32; 2],
    pub raw_grid: [u32; 2],
    pub output_grid: [u32; 2],
    pub block: [u32; 2],
}

/// Explicit captured/calibrated inputs for one reference frame.
pub struct Parameters<'a> {
    pub geometry: Geometry,
    pub luma: &'a [u8],
    pub confidence_y: &'a [f32; 256],
    pub confidence_uv: &'a [f32; 256],
    pub scale_base: i32,
    pub scale_extra: i32,
    pub temporal: f32,
    pub phase: f64,
}

fn checked_area(size: [u32; 2], what: &str) -> Result<usize, String> {
    let area = size[0]
        .checked_mul(size[1])
        .ok_or_else(|| format!("{what} dimensions overflow"))?;
    usize::try_from(area).map_err(|_| format!("{what} dimensions do not fit usize"))
}

fn validate(raw: &[[i32; 3]], parameters: &Parameters<'_>) -> Result<(), String> {
    let geometry = parameters.geometry;
    if geometry
        .full
        .into_iter()
        .chain(geometry.raw_grid)
        .chain(geometry.output_grid)
        .chain(geometry.block)
        .any(|value| value == 0 || value > i32::MAX as u32)
    {
        return Err("motion geometry must be nonzero and fit signed i32".into());
    }
    if geometry.block != [16, 16] {
        return Err("motion reference supports only the selected 16x16 blocks".into());
    }
    if geometry
        .output_grid
        .into_iter()
        .any(|value| value > 1 << 24)
    {
        return Err("motion grid coordinates must be exactly representable as f32".into());
    }
    let expanded = geometry.raw_grid.map(|value| value.checked_mul(2));
    if expanded != geometry.output_grid.map(Some) {
        return Err("motion reference supports only the selected 2x expansion".into());
    }
    let covered = [
        geometry.output_grid[0].checked_mul(geometry.block[0]),
        geometry.output_grid[1].checked_mul(geometry.block[1]),
    ];
    if covered != geometry.full.map(Some) {
        return Err("full image is not exactly covered by the selected block grid".into());
    }
    if raw.len() != checked_area(geometry.raw_grid, "raw grid")? {
        return Err("raw motion length does not match its grid".into());
    }
    if parameters.luma.len() != checked_area(geometry.output_grid, "luma grid")? {
        return Err("luma length does not match the output grid".into());
    }
    if !parameters.temporal.is_finite() || !parameters.phase.is_finite() {
        return Err("temporal scale and phase must be finite".into());
    }
    if parameters
        .confidence_y
        .iter()
        .chain(parameters.confidence_uv)
        .any(|value| !value.is_finite())
    {
        return Err("confidence tables must be finite".into());
    }
    Ok(())
}

fn fcvtzs_i32(value: f32, what: &str) -> Result<i32, String> {
    if !value.is_finite() || value < i32::MIN as f32 || value >= 2_147_483_648.0 {
        return Err(format!(
            "{what} is outside the supported i32 conversion range"
        ));
    }
    Ok(value.trunc() as i32)
}

fn fcvtzs_i32_f64(value: f64, what: &str) -> Result<i32, String> {
    if !value.is_finite() || value < i32::MIN as f64 || value >= 2_147_483_648.0 {
        return Err(format!(
            "{what} is outside the supported i32 conversion range"
        ));
    }
    Ok(value.trunc() as i32)
}

fn interpolated_channel(
    raw: &[[i32; 3]],
    raw_width: usize,
    x: usize,
    y: usize,
    channel: usize,
) -> f32 {
    let sx = x as f32 * 0.5;
    let sy = y as f32 * 0.5;
    let x0 = sx.trunc() as usize;
    let y0 = sy.trunc() as usize;
    let x1 = (x0 + 1).min(raw_width - 1);
    let raw_height = raw.len() / raw_width;
    let y1 = (y0 + 1).min(raw_height - 1);
    let fx = sx - x0 as f32;
    let fy = sy - y0 as f32;
    let weights = [
        (1.0 - fy) * (1.0 - fx),
        (1.0 - fy) * fx,
        fy * (1.0 - fx),
        fy * fx,
    ];
    let samples = [
        raw[y0 * raw_width + x0][channel],
        raw[y0 * raw_width + x1][channel],
        raw[y1 * raw_width + x0][channel],
        raw[y1 * raw_width + x1][channel],
    ];

    // Native order: top-right multiply, then top-left, bottom-left and
    // bottom-right fused multiply-adds.
    let mut value = samples[1] as f32 * weights[1];
    value = (samples[0] as f32).mul_add(weights[0], value);
    value = (samples[2] as f32).mul_add(weights[2], value);
    (samples[3] as f32).mul_add(weights[3], value)
}

fn confidence(threshold: i32, cost: i32) -> Result<i16, String> {
    if threshold <= cost {
        return Ok(0);
    }
    let square = threshold.wrapping_mul(threshold) as f64;
    let cost = cost as f64;
    let cost_square = cost * cost;
    let value = 256.0 * (square - cost_square) / (square + cost_square);
    // Native stores the low halfword after FCVTZS.
    Ok(fcvtzs_i32_f64(value, "confidence")? as i16)
}

/// Expand raw `(dx, dy, cost)` records and pack `(dx, dy, Y, UV)` i16 lanes.
///
/// Unsupported geometry and numeric values are rejected rather than assigned
/// semantics that were not established by the selected native path.
pub fn pack_motion(raw: &[[i32; 3]], parameters: &Parameters<'_>) -> Result<Vec<[i16; 4]>, String> {
    validate(raw, parameters)?;
    let geometry = parameters.geometry;
    let output_width = geometry.output_grid[0] as usize;
    let output_height = geometry.output_grid[1] as usize;
    let raw_width = geometry.raw_grid[0] as usize;

    let blend64 = (parameters.temporal as f64).mul_add(1.0 - parameters.phase, parameters.phase);
    let blend32 = blend64 as f32;
    let base = parameters.scale_base.wrapping_mul(parameters.scale_extra);
    let scale = fcvtzs_i32((base as f32).mul_add(blend32, 0.5), "confidence scale")?;

    let mut packed = Vec::with_capacity(output_width * output_height);
    for y in 0..output_height {
        for x in 0..output_width {
            let dx = fcvtzs_i32(
                interpolated_channel(raw, raw_width, x, y, 0) / 0.5,
                "horizontal displacement",
            )?;
            let dy = fcvtzs_i32(
                interpolated_channel(raw, raw_width, x, y, 1) / 0.5,
                "vertical displacement",
            )?;
            let cost = fcvtzs_i32(interpolated_channel(raw, raw_width, x, y, 2), "match cost")?;

            let origin_x = (x as i32)
                .checked_mul(geometry.block[0] as i32)
                .ok_or_else(|| "horizontal block origin overflows i32".to_string())?;
            let origin_y = (y as i32)
                .checked_mul(geometry.block[1] as i32)
                .ok_or_else(|| "vertical block origin overflows i32".to_string())?;
            let maximum_x = geometry.full[0] as i32 - geometry.block[0] as i32;
            let maximum_y = geometry.full[1] as i32 - geometry.block[1] as i32;
            let dx = origin_x.wrapping_add(dx).clamp(0, maximum_x) - origin_x;
            let dy = origin_y.wrapping_add(dy).clamp(0, maximum_y) - origin_y;

            let luma = parameters.luma[y * output_width + x] as usize;
            let threshold_y =
                fcvtzs_i32(parameters.confidence_y[luma] * scale as f32, "Y threshold")?;
            let threshold_uv = fcvtzs_i32(
                parameters.confidence_uv[luma] * scale as f32,
                "UV threshold",
            )?;
            packed.push([
                dx as i16,
                dy as i16,
                confidence(threshold_y, cost)?,
                confidence(threshold_uv, cost)?,
            ]);
        }
    }
    Ok(packed)
}

#[cfg(test)]
mod tests;
