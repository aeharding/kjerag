//! Conservative full-panorama scissors for a rectilinear projected view.
//!
//! These rectangles reduce fragment work without changing texture coordinates
//! or rebasing an intermediate. Unsupported geometry deliberately becomes one
//! full-frame rectangle.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use crate::Reframe;

const RGB_TEXEL_HALO: i64 = 1;
const FUSION_Y_TEXEL_HALO: u32 = 2;
const GRAM_LIMIT: f64 = 1.0e-4;
// Covers ordinary f32 matrix/trigonometric drift. This is an execution bound,
// not a visible-quality or filtering parameter. A small non-orthogonality is
// padded separately below; larger departures select the full-frame fallback.
const ANGULAR_PAD: f64 = 32.0 * f32::EPSILON as f64;

/// Full-coordinate scissors which cover one exact projected view.
///
/// This is execution geometry only. It does not change the view, filtering,
/// temporal parameters, or source association.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewRegions {
    rgb: Vec<[u32; 4]>,
    fusion: Vec<[u32; 4]>,
}

impl ViewRegions {
    /// Bound the exact `Reframe`, falling back to the full panorama whenever
    /// its projection or body transform cannot be bounded conservatively.
    pub fn for_view(full: [u32; 2], reframe: &Reframe) -> Result<Self, String> {
        valid_full(full)?;
        let fallback = || Self::full(full);
        if !reframe.is_rectilinear() {
            return Ok(fallback());
        }

        let axes = [
            reframe.body_ray([1.0, 0.0, 0.0]),
            reframe.body_ray([0.0, 1.0, 0.0]),
            reframe.body_ray([0.0, 0.0, 1.0]),
        ];
        let Some(gram_error) = gram_error(axes) else {
            return Ok(fallback());
        };
        if gram_error > GRAM_LIMIT {
            return Ok(fallback());
        }

        let Some(center_view) = reframe.view_ray([0.5, 0.5]) else {
            return Ok(fallback());
        };
        let Some(center) = unit(reframe.body_ray(center_view)) else {
            return Ok(fallback());
        };
        let mut radius = 0.0_f64;
        for uv in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]] {
            let Some(view) = reframe.view_ray(uv) else {
                return Ok(fallback());
            };
            let Some(corner) = unit(reframe.body_ray(view)) else {
                return Ok(fallback());
            };
            radius = radius.max(dot(center, corner).clamp(-1.0, 1.0).acos());
        }
        // For a nearly orthogonal matrix, each Gram-entry error is bounded by
        // `gram_error`; this deliberately loose multiplier covers its angular
        // distortion rather than treating the f32 columns as an exact rotation.
        radius += ANGULAR_PAD + 16.0 * gram_error;
        if !radius.is_finite() || radius >= FRAC_PI_2 {
            return Ok(fallback());
        }

        let latitude = center[1].clamp(-1.0, 1.0).asin();
        let low_latitude = (latitude - radius).max(-FRAC_PI_2);
        let high_latitude = (latitude + radius).min(FRAC_PI_2);
        let v_low = (FRAC_PI_2 - high_latitude) / PI;
        let v_high = (FRAC_PI_2 - low_latitude) / PI;
        let longitude = center[0].atan2(-center[2]).rem_euclid(TAU);

        let longitude_spans = if latitude.abs() + radius >= FRAC_PI_2 {
            vec![(0.0, 1.0)]
        } else {
            let ratio = radius.sin() / latitude.cos();
            if !ratio.is_finite() || ratio.abs() > 1.0 {
                return Ok(fallback());
            }
            padded_longitude_spans(longitude, ratio.asin(), full[0])
        };
        let mut rgb: Vec<_> = longitude_spans
            .into_iter()
            .map(|(u0, u1)| raster_rectangle(full, u0, u1, v_low, v_high, RGB_TEXEL_HALO))
            .collect();
        merge_linear(&mut rgb);
        validate(full, &rgb)?;

        let mut fusion: Vec<_> = rgb
            .iter()
            .map(|&rectangle| expand_clamped(full, rectangle, FUSION_Y_TEXEL_HALO))
            .collect();
        merge_linear(&mut fusion);
        validate(full, &fusion)?;
        Ok(Self { rgb, fusion })
    }

    fn full(full: [u32; 2]) -> Self {
        let rectangle = [0, 0, full[0], full[1]];
        Self {
            rgb: vec![rectangle],
            fusion: vec![rectangle],
        }
    }

    /// Full-coordinate RGB scissors including the projector's linear-filter halo.
    pub fn rgb(&self) -> &[[u32; 4]] {
        &self.rgb
    }

    /// Full-coordinate luma rectangles. The GPU consumer halves them for UV.
    pub fn fusion(&self) -> &[[u32; 4]] {
        &self.fusion
    }
}

pub(crate) fn validate(full: [u32; 2], rectangles: &[[u32; 4]]) -> Result<(), String> {
    valid_full(full)?;
    if !(1..=2).contains(&rectangles.len()) {
        return Err("panorama scissors need one or two rectangles".into());
    }
    for &[x, y, width, height] in rectangles {
        if [x, y, width, height]
            .into_iter()
            .any(|value| !value.is_multiple_of(2))
            || width == 0
            || height == 0
            || x.checked_add(width).is_none_or(|end| end > full[0])
            || y.checked_add(height).is_none_or(|end| end > full[1])
        {
            return Err(
                "panorama scissors must be positive even rectangles inside the frame".into(),
            );
        }
    }
    if rectangles.len() == 2 && overlaps(rectangles[0], rectangles[1]) {
        return Err("panorama scissors must not overlap".into());
    }
    Ok(())
}

fn valid_full(full: [u32; 2]) -> Result<(), String> {
    if full
        .into_iter()
        .any(|value| value == 0 || !value.is_multiple_of(2))
    {
        return Err("panorama scissor frame must have positive even dimensions".into());
    }
    Ok(())
}

fn unit(value: [f32; 3]) -> Option<[f64; 3]> {
    let value = value.map(f64::from);
    let length = dot(value, value).sqrt();
    (length.is_finite() && length > 1.0e-12).then(|| value.map(|lane| lane / length))
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn gram_error(axes: [[f32; 3]; 3]) -> Option<f64> {
    let axes: Vec<_> = axes.into_iter().map(|axis| axis.map(f64::from)).collect();
    if axes.iter().flatten().any(|value| !value.is_finite()) {
        return None;
    }
    let mut error = 0.0_f64;
    for row in 0..3 {
        for column in 0..3 {
            let wanted = if row == column { 1.0 } else { 0.0 };
            error = error.max((dot(axes[row], axes[column]) - wanted).abs());
        }
    }
    Some(error)
}

fn split_longitude(low: f64, high: f64) -> Vec<(f64, f64)> {
    if low < 0.0 {
        vec![(0.0, high / TAU), ((low + TAU) / TAU, 1.0)]
    } else if high > TAU {
        vec![(0.0, (high - TAU) / TAU), (low / TAU, 1.0)]
    } else {
        vec![(low / TAU, high / TAU)]
    }
}

fn padded_longitude_spans(longitude: f64, half_width: f64, full_width: u32) -> Vec<(f64, f64)> {
    // A normalized periodic linear sample can cross the physical texture edge
    // even when the geometric cap itself stops just short of it. Expand before
    // splitting so both edge texels are drawn; raster padding below is extra.
    let padded = half_width + TAU / f64::from(full_width);
    if padded * 2.0 >= TAU {
        vec![(0.0, 1.0)]
    } else {
        split_longitude(longitude - padded, longitude + padded)
    }
}

fn raster_rectangle(full: [u32; 2], u0: f64, u1: f64, v0: f64, v1: f64, halo: i64) -> [u32; 4] {
    let (x0, x1) = raster_axis(full[0], u0, u1, halo);
    let (y0, y1) = raster_axis(full[1], v0, v1, halo);
    [x0, y0, x1 - x0, y1 - y0]
}

fn raster_axis(size: u32, low: f64, high: f64, halo: i64) -> (u32, u32) {
    let size_i = i64::from(size);
    let mut start = (low * f64::from(size)).floor() as i64 - halo;
    let mut end = (high * f64::from(size)).ceil() as i64 + halo;
    start = start.clamp(0, size_i) & !1;
    end = end.clamp(0, size_i);
    end = (end + 1).min(size_i) & !1;
    if start == end {
        if end < size_i {
            end += 2;
        } else {
            start -= 2;
        }
    }
    (start as u32, end as u32)
}

fn expand_clamped(full: [u32; 2], rectangle: [u32; 4], halo: u32) -> [u32; 4] {
    let x0 = rectangle[0].saturating_sub(halo) & !1;
    let y0 = rectangle[1].saturating_sub(halo) & !1;
    let x1 = rectangle[0]
        .saturating_add(rectangle[2])
        .saturating_add(halo)
        .min(full[0]);
    let y1 = rectangle[1]
        .saturating_add(rectangle[3])
        .saturating_add(halo)
        .min(full[1]);
    let x1 = (x1 + 1).min(full[0]) & !1;
    let y1 = (y1 + 1).min(full[1]) & !1;
    [x0, y0, x1 - x0, y1 - y0]
}

fn merge_linear(rectangles: &mut Vec<[u32; 4]>) {
    rectangles.sort_unstable_by_key(|rectangle| rectangle[0]);
    if rectangles.len() != 2
        || rectangles[0][1] != rectangles[1][1]
        || rectangles[0][3] != rectangles[1][3]
    {
        return;
    }
    let first_end = rectangles[0][0] + rectangles[0][2];
    if first_end < rectangles[1][0] {
        return;
    }
    let end = (rectangles[1][0] + rectangles[1][2]).max(first_end);
    rectangles[0][2] = end - rectangles[0][0];
    rectangles.pop();
}

fn overlaps(a: [u32; 4], b: [u32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod gpu_tests;
