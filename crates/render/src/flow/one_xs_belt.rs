//! The selected ONE X2 line-image law and its scalar oracle.
//!
//! Native V6 captures two `CV_8UC1` staging images with 3,240 rows along the
//! seam and 180 columns across it. It then reduces every aligned 3 by 3 cell
//! with OpenCV `INTER_AREA` into the 1,080 by 60 images the flow solver reads.
//! For this integer ratio the byte result is exactly `(sum + 4) / 9`. Applying
//! that expression to both captured staging images reproduces the two recorded
//! pre-blur SHA-256 values byte for byte (§114D).
//!
//! Panotype 5 supplies two distinct source-image inputs in physical A/B order.
//! The selected callable presents one U8 value per sampled source pixel, so
//! this oracle accepts each through a compact U8C1-equivalent semantic
//! interface. That does not infer the native Mat's storage type, channel
//! layout or step. Each source is remapped independently through its matching
//! retained `+0x8d0/+0x930` `CV_32FC2` base map. A staging pixel `(row,col)`
//! samples that map at `(col/3,row/3)`, with no half-pixel offset and
//! clamp-to-edge bilinear interpolation. An ordered `u > 0 && v > 0` UV then
//! samples the corresponding source image at `(u*cols,v*rows)` through the
//! selected clamp/bilinear callable and truncates the nonnegative result to
//! U8; other UV becomes black. The target-frame source dimensions and bytes
//! remain runtime inputs rather than constants guessed by this module.
//! The pinned Mac anchors are the call setups `0x2ab4bcc..0x2ab4bf0` and
//! `0x2ab4bf4..0x2ab4c20`, wrapper `0x2ab5ea8`, row body `0x2ae10a0`, map
//! sampler `0x2a369ec`, and source sampler `0x2a9924c` (§114K).
//!
//! The CPU sampler below preserves the selected scalar instruction order: the
//! retained-map and U8 source helpers start with top-right multiplication,
//! then accumulate top-left, bottom-left and bottom-right with fused
//! multiply-add. The inactive ordinary-WGSL instrument mirrors the coordinate
//! and topology law with a one-code contraction bound and performs the exact
//! integer area reduction; this CPU path is the accessible bit-exact oracle.
//! This stays separate from [`crate::band`], whose legacy rotated-equirect
//! strip has different dimensions and axes.

use std::error::Error;
use std::fmt;

use super::one_xs::{self, Lens, LensPair};
use crate::projection::Reframe;

/// The selected route has exactly two physical lens images.
pub const LENSES: usize = 2;

/// The native staging-to-solver scale on both axes.
pub const AREA_SCALE: usize = 3;

/// Largest dynamic source dimension whose final zero-based index is exactly
/// representable in the f32 coordinate domain used by the native sampler.
const MAX_EXACT_F32_DIMENSION: usize = (1 << 24) + 1;

/// One compact row-major U8C1-equivalent semantic source interface.
///
/// This is the scalar callable's value interface, not a claim about the native
/// Mat's storage type, channels or step. The target dimensions are
/// intentionally dynamic. Requiring nonzero dimensions and exactly
/// `rows * cols` semantic values makes clamp-to-edge sampling total without
/// inventing a fixed source shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceImage {
    rows: usize,
    cols: usize,
    pixels: Box<[u8]>,
}

impl SourceImage {
    /// Admit a compact one-channel image with checked dynamic dimensions.
    pub fn from_compact(
        rows: usize,
        cols: usize,
        pixels: Vec<u8>,
    ) -> Result<Self, SourceShapeError> {
        if rows == 0 || cols == 0 {
            return Err(SourceShapeError::Empty { rows, cols });
        }
        if rows > MAX_EXACT_F32_DIMENSION || cols > MAX_EXACT_F32_DIMENSION {
            return Err(SourceShapeError::DimensionTooLarge {
                rows,
                cols,
                maximum: MAX_EXACT_F32_DIMENSION,
            });
        }
        let expected = rows
            .checked_mul(cols)
            .ok_or(SourceShapeError::Overflow { rows, cols })?;
        if pixels.len() != expected {
            return Err(SourceShapeError::PixelCount {
                rows,
                cols,
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            rows,
            cols,
            pixels: pixels.into_boxed_slice(),
        })
    }

    pub const fn rows(&self) -> usize {
        self.rows
    }

    pub const fn cols(&self) -> usize {
        self.cols
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    fn sample_clamped(&self, row: f32, col: f32) -> u8 {
        let row = ClampedAxis::new(row, self.rows);
        let col = ClampedAxis::new(col, self.cols);
        let at = |row: usize, col: usize| f32::from(self.pixels[row * self.cols + col]);
        let weights = BilinearWeights::new(row, col);
        // `slerpNorm00<Vec<U8,1>> @ +0x2a9932c..+0x2a99348` forms the
        // top-right product first, then accumulates top-left, bottom-left and
        // bottom-right with three FMA instructions.
        let value = weights.top_right * at(row.lo, col.hi);
        let value = weights.top_left.mul_add(at(row.lo, col.lo), value);
        let value = weights.bottom_left.mul_add(at(row.hi, col.lo), value);
        let value = weights.bottom_right.mul_add(at(row.hi, col.hi), value);
        debug_assert!(value.is_finite() && (0.0..=255.0).contains(&value));
        // Native `fcvtzs` truncates this nonnegative result toward zero.
        value as u8
    }
}

/// A dynamic compact source image had no sampleable shape or the wrong byte
/// count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceShapeError {
    Empty {
        rows: usize,
        cols: usize,
    },
    Overflow {
        rows: usize,
        cols: usize,
    },
    DimensionTooLarge {
        rows: usize,
        cols: usize,
        maximum: usize,
    },
    PixelCount {
        rows: usize,
        cols: usize,
        expected: usize,
        actual: usize,
    },
}

impl fmt::Display for SourceShapeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Empty { rows, cols } => write!(
                out,
                "ONE X2 compact source is {rows} by {cols}; both dimensions must be nonzero"
            ),
            Self::Overflow { rows, cols } => {
                write!(
                    out,
                    "ONE X2 compact source shape {rows} by {cols} is too large"
                )
            }
            Self::DimensionTooLarge {
                rows,
                cols,
                maximum,
            } => write!(
                out,
                "ONE X2 compact source is {rows} by {cols}; each dimension must be at most \
                 {maximum} for exact f32 addressing"
            ),
            Self::PixelCount {
                rows,
                cols,
                expected,
                actual,
            } => write!(
                out,
                "ONE X2 compact source is {rows} by {cols} but has {actual} bytes, expected {expected}"
            ),
        }
    }
}

impl Error for SourceShapeError {}

/// The two retained 1,080-row by 60-column `CV_32FC2` base maps.
///
/// `a` is native `+0x8d0` and `b` is native `+0x930`. Values are not required
/// to be finite: an unordered sampled UV deliberately fails the later strict
/// positive gate and writes black.
#[derive(Clone, Debug, PartialEq)]
pub struct RetainedBaseMaps {
    uv: Box<[[f32; 2]]>,
}

impl RetainedBaseMaps {
    pub const NODES_PER_LENS: usize = one_xs::ROWS * one_xs::COLS;
    pub const NODES: usize = LENSES * Self::NODES_PER_LENS;

    /// Admit the named native-order compact float2 maps.
    pub fn from_lenses(maps: LensPair<Vec<[f32; 2]>>) -> Result<Self, BaseMapShapeError> {
        for lens in Lens::ALL {
            let map = maps.get(lens);
            if map.len() != Self::NODES_PER_LENS {
                return Err(BaseMapShapeError {
                    lens,
                    expected: Self::NODES_PER_LENS,
                    actual: map.len(),
                });
            }
        }

        let mut uv = Vec::with_capacity(Self::NODES);
        uv.extend(maps.a);
        uv.extend(maps.b);
        Ok(Self {
            uv: uv.into_boxed_slice(),
        })
    }

    /// Both admitted maps as tightly packed float bytes, lens A then lens B.
    ///
    /// This is the explicit upload boundary for the inactive GPU instrument;
    /// it does not generate or approximate a retained map.
    #[cfg(test)]
    fn bytes(&self) -> &[u8] {
        // `[f32; 2]` has no padding, and every f32 bit pattern is valid. This
        // is the same upload boundary used by `one_xs::Displacement::bytes`.
        unsafe {
            std::slice::from_raw_parts(
                self.uv.as_ptr().cast::<u8>(),
                std::mem::size_of_val(&*self.uv),
            )
        }
    }

    /// One lens's compact native row-major float2 payload.
    pub fn lens(&self, lens: Lens) -> &[[f32; 2]] {
        let start = lens.index() * Self::NODES_PER_LENS;
        &self.uv[start..start + Self::NODES_PER_LENS]
    }

    fn sample_clamped(&self, lens: Lens, row: f32, col: f32) -> [f32; 2] {
        let row = ClampedAxis::new(row, one_xs::ROWS);
        let col = ClampedAxis::new(col, one_xs::COLS);
        let map = self.lens(lens);
        let at = |row: usize, col: usize| map[row * one_xs::COLS + col];
        let top_left = at(row.lo, col.lo);
        let top_right = at(row.lo, col.hi);
        let bottom_left = at(row.hi, col.lo);
        let bottom_right = at(row.hi, col.hi);
        let weights = BilinearWeights::new(row, col);
        std::array::from_fn(|component| {
            // `slerp00<Vec<float,2>> @ +0x2a36bcc..+0x2a36be8` forms the
            // top-right product first, then accumulates top-left,
            // bottom-left and bottom-right with three FMA instructions.
            let value = weights.top_right * top_right[component];
            let value = weights.top_left.mul_add(top_left[component], value);
            let value = weights.bottom_left.mul_add(bottom_left[component], value);
            weights.bottom_right.mul_add(bottom_right[component], value)
        })
    }
}

/// A retained float2 map did not have the selected fixed grid shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaseMapShapeError {
    lens: Lens,
    expected: usize,
    actual: usize,
}

impl fmt::Display for BaseMapShapeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "ONE X2 lens {} base map has {} float2 nodes, expected {}",
            self.lens, self.actual, self.expected,
        )
    }
}

impl Error for BaseMapShapeError {}

// -------------------------------- static ONE X2 camera mask (§140)

/// Studio's selected ONE X2 camera-mask raster is 400 by 400 per lens.
const CAMERA_MASK_SIZE: usize = 400;

const HALF_FIELD_DEG: f32 = 100.0;
const EXPAND_DEG: f32 = 98.0;
const WEDGE_COS_20_DEG: f32 = 0.9396926;
/// OpenCV's fixed-point shift for the selected native `DIST_L2`, mask-size 5
/// distance transform.
const DISTANCE_SHIFT: u32 = 16;
const DISTANCE_HV: u32 = 65_536;
const DISTANCE_DIAGONAL: u32 = 91_750;
const DISTANCE_LONG: u32 = 143_976;
const DISTANCE_MAX: u32 = (i32::MAX as u32) >> 2;
const DISTANCE_CONDITION_SCALE: f32 = 0.243_902_44;
const BELT_ROW_FRACTIONS: [f32; 2] = [0.2, 0.8];

/// `ins::Lens::getCoeff` for lens type `0x29`, read in §140F.
const RADIUS_COEFFICIENTS: [f64; 5] = [
    0.0,
    0.024_132_66,
    -7.451_592e-5,
    2.083_951_1e-6,
    -1.457_321_1e-8,
];

/// `(azimuth, colatitude limit)` in degrees, read in §140E.
const AZIMUTH_LIMITS: [[f32; 2]; 11] = [
    [0.0, 90.5],
    [13.0, 90.5],
    [18.0, 91.5],
    [24.0, 92.5],
    [28.0, 93.5],
    [37.0, 94.5],
    [45.0, 95.5],
    [53.5, 96.5],
    [57.0, 97.5],
    [61.0, 98.5],
    [70.0, 99.5],
];

/// The mixed-width polynomial schedule selected by both native mask helpers.
fn radius_law(theta_deg: f32) -> f32 {
    let c = RADIUS_COEFFICIENTS;
    let theta = f64::from(theta_deg);
    let square = theta * theta;
    let cube = square * theta;
    let fourth = cube * theta;
    let value = c[1].mul_add(theta, c[0]);
    let value = c[2].mul_add(square, value);
    let value = c[3].mul_add(cube, value);
    c[4].mul_add(fourth, value) as f32
}

/// Interpolate the native f32 azimuth table. Its intervals are lower-inclusive
/// and upper-exclusive; outside all ten intervals the helper retains the
/// already-computed 98-degree circle radius.
fn theta_limit(phi_deg: f32) -> Option<f32> {
    for bracket in 0..AZIMUTH_LIMITS.len() - 1 {
        let [azimuth, limit] = AZIMUTH_LIMITS[bracket];
        let [next_azimuth, next_limit] = AZIMUTH_LIMITS[bracket + 1];
        if azimuth <= phi_deg && phi_deg < next_azimuth {
            let slope = (next_limit - limit) / (next_azimuth - azimuth);
            return Some(slope.mul_add(phi_deg - azimuth, limit));
        }
    }
    None
}

fn camera_mask_ranges() -> [(usize, usize); 2] {
    [
        (0, (BELT_ROW_FRACTIONS[0] * one_xs::ROWS as f32) as usize),
        (
            (BELT_ROW_FRACTIONS[1] * one_xs::ROWS as f32) as usize,
            one_xs::ROWS,
        ),
    ]
}

/// Geometry used for one reconstructed 400-by-400 camera mask.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraMaskGeometry {
    pub centre: [f32; 2],
    /// Studio's `Offset::setOffset` image-circle radius, retained as the
    /// binary32 value its `Offset` object stores.
    pub image_circle_radius: f32,
    pub mask_radius: f32,
    pub scale: f32,
    pub support_pixels: usize,
}

/// Measured outcome of the reconstructed static camera-mask step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraMaskReport {
    pub geometry: [CameraMaskGeometry; LENSES],
    pub ranges: [(usize, usize); 2],
    pub rows_walked: usize,
    pub cleared: [usize; LENSES],
    pub unified: usize,
    pub final_valid: usize,
}

impl fmt::Display for CameraMaskReport {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "rows [{},{}) and [{},{}) ({} of {}), cleared A {} B {}, unify moved {}, final valid {}",
            self.ranges[0].0,
            self.ranges[0].1,
            self.ranges[1].0,
            self.ranges[1].1,
            self.rows_walked,
            one_xs::ROWS,
            self.cleared[0],
            self.cleared[1],
            self.unified,
            self.final_valid,
        )
    }
}

/// Why the static camera-mask reconstruction could not produce a truthful
/// physical mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraMaskError {
    NotOneXsPair,
}

impl fmt::Display for CameraMaskError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NotOneXsPair => write!(
                out,
                "static ONE X2 camera mask needs a two-lens ONE X2 projection"
            ),
        }
    }
}

impl Error for CameraMaskError {}

#[derive(Debug)]
pub(crate) struct CameraMaskSupport {
    pixels: [Box<[f32]>; LENSES],
    geometry: [CameraMaskGeometry; LENSES],
}

impl CameraMaskSupport {
    pub(crate) fn from_reframe(reframe: &Reframe) -> Result<Self, CameraMaskError> {
        if !reframe.is_one_xs_pair() {
            return Err(CameraMaskError::NotOneXsPair);
        }
        let [width, _] = reframe.frame_size();
        let scale = CAMERA_MASK_SIZE as f32 / width;
        let half_field_law = radius_law(HALF_FIELD_DEG);
        let expand_law = radius_law(EXPAND_DEG);
        let mut supports = Vec::with_capacity(LENSES);
        let mut geometry = Vec::with_capacity(LENSES);
        for lens in Lens::ALL {
            let [cx, cy] = reframe.lens_image_circle_centre(lens.index());
            let image_circle_radius = reframe.one_xs_v3_image_circle_radius(lens.index());
            let centre = [scale * cx, scale * cy];
            let mask_radius = scale * (image_circle_radius * expand_law / half_field_law);
            let binary = draw_camera_support(centre, mask_radius, image_circle_radius, scale, lens);
            let support_pixels = binary.iter().filter(|value| **value > 0.0).count();
            let support = condition_camera_support(&binary, CAMERA_MASK_SIZE, CAMERA_MASK_SIZE);
            supports.push(support);
            geometry.push(CameraMaskGeometry {
                centre,
                image_circle_radius,
                mask_radius,
                scale,
                support_pixels,
            });
        }
        Ok(Self {
            pixels: [supports[0].clone(), supports[1].clone()],
            geometry: [geometry[0], geometry[1]],
        })
    }

    fn sample(&self, lens: Lens, [u, v]: [f32; 2]) -> f32 {
        let support = &self.pixels[lens.index()];
        let col = ClampedAxis::new(u * CAMERA_MASK_SIZE as f32, CAMERA_MASK_SIZE);
        let row = ClampedAxis::new(v * CAMERA_MASK_SIZE as f32, CAMERA_MASK_SIZE);
        let at = |row: usize, col: usize| support[row * CAMERA_MASK_SIZE + col];
        let top = lerp(at(row.lo, col.lo), at(row.lo, col.hi), col.frac);
        let bottom = lerp(at(row.hi, col.lo), at(row.hi, col.hi), col.frac);
        lerp(top, bottom, row.frac)
    }

    pub(crate) fn apply(&self, base: &RetainedBaseMaps) -> (LensPair<Vec<u8>>, CameraMaskReport) {
        apply_camera_support(base, self)
    }
}

fn conditioned_sample_clears(sample: f32) -> bool {
    // The native body promotes the sampled float to binary64 before comparing
    // it with the binary64 literal. Keeping the promotion is observable at the
    // nearest f32 representation of 1e-8.
    f64::from(sample) < 1e-8_f64
}

/// Seed, erode and unify the physical base-map validity masks.
pub(crate) fn base_support_masks(base: &RetainedBaseMaps) -> LensPair<Vec<u8>> {
    let eroded = |lens| erode_rect_9x9(&mask_seed(base.lens(lens)), one_xs::COLS, one_xs::ROWS);
    let a = eroded(Lens::A);
    let b = eroded(Lens::B);
    let unified = a
        .into_iter()
        .zip(b)
        .map(|(left, right)| left & right)
        .collect::<Vec<_>>();
    LensPair {
        a: unified.clone(),
        b: unified,
    }
}

/// Apply the §140-derived static camera-mask reconstruction to already
/// authenticated retained base maps.
///
/// The control flow, constants, crop-translated centres, scalar image-circle
/// producer, arrangement and binary32 geometry schedule reproduce the
/// authenticated matching V9 native mask byte for byte. That oracle does not
/// make this an exact V6 replay: V6's base maps differ and its native final
/// mask bytes were not captured.
///
/// Native conditions the binary support with its fixed-point 5-by-5 L2
/// distance transform before the bilinear sample and `< 1e-8` test. This path
/// materialises that field rather than replacing it with a binary bound.
pub fn reconstructed_camera_masks(
    reframe: &Reframe,
    base: &RetainedBaseMaps,
) -> Result<(LensPair<Vec<u8>>, CameraMaskReport), CameraMaskError> {
    let support = CameraMaskSupport::from_reframe(reframe)?;
    Ok(support.apply(base))
}

fn apply_camera_support(
    base: &RetainedBaseMaps,
    support: &CameraMaskSupport,
) -> (LensPair<Vec<u8>>, CameraMaskReport) {
    let mut masks = base_support_masks(base);
    let ranges = camera_mask_ranges();
    let mut cleared = [0; LENSES];
    for lens in Lens::ALL {
        let map = base.lens(lens);
        let mask = match lens {
            Lens::A => &mut masks.a,
            Lens::B => &mut masks.b,
        };
        for (start, end) in ranges {
            for row in start..end {
                for col in 0..one_xs::COLS {
                    let index = row * one_xs::COLS + col;
                    if mask[index] == 0 {
                        continue;
                    }
                    let [u, v] = map[index];
                    if !(u > 0.0 && u <= 1.0 && v > 0.0 && v <= 1.0) {
                        continue;
                    }
                    if conditioned_sample_clears(support.sample(lens, [u, v])) {
                        mask[index] = 0;
                        cleared[lens.index()] += 1;
                    }
                }
            }
        }
    }
    let mut unified = 0;
    for index in 0..one_xs::ROWS * one_xs::COLS {
        let both = masks.a[index] & masks.b[index];
        unified += usize::from(masks.a[index] != both || masks.b[index] != both);
        masks.a[index] = both;
        masks.b[index] = both;
    }
    let final_valid = masks.a.iter().filter(|value| **value != 0).count();
    (
        masks,
        CameraMaskReport {
            geometry: support.geometry,
            ranges,
            rows_walked: ranges.iter().map(|(start, end)| end - start).sum(),
            cleared,
            unified,
            final_valid,
        },
    )
}

fn mask_seed(base: &[[f32; 2]]) -> Vec<u8> {
    base.iter()
        .map(|[u, v]| u8::from(*u > 0.0 && *u <= 1.0 && *v > 0.0 && *v <= 1.0) * 255)
        .collect()
}

/// Exact 9-by-9 rectangular minimum with OpenCV's selected `+inf` border.
///
/// A rectangular minimum is separable. Ignoring samples outside each axis in
/// the two one-dimensional passes is exactly the same as ignoring every
/// outside sample in the original two-dimensional window.
fn erode_rect_9x9(mask: &[u8], cols: usize, rows: usize) -> Vec<u8> {
    debug_assert_eq!(mask.len(), rows * cols);
    const RADIUS: usize = 4;

    let mut horizontal = vec![u8::MAX; mask.len()];
    for row in 0..rows {
        let row_start = row * cols;
        let source = &mask[row_start..row_start + cols];
        for col in 0..cols {
            let first = col.saturating_sub(RADIUS);
            let last = col.saturating_add(RADIUS).min(cols - 1);
            horizontal[row_start + col] = source[first..=last]
                .iter()
                .copied()
                .min()
                .expect("a 9-by-9 erosion window contains its centre");
        }
    }

    let mut output = vec![0u8; mask.len()];
    for row in 0..rows {
        for col in 0..cols {
            let first = row.saturating_sub(RADIUS);
            let last = row.saturating_add(RADIUS).min(rows - 1);
            output[row * cols + col] = (first..=last)
                .map(|source_row| horizontal[source_row * cols + col])
                .min()
                .expect("a 9-by-9 erosion window contains its centre");
        }
    }
    output
}

/// Filled OpenCV-style midpoint circle followed by §140E's inward wedge.
fn draw_camera_support(
    centre: [f32; 2],
    radius: f32,
    image_circle_radius: f32,
    scale: f32,
    lens: Lens,
) -> Box<[f32]> {
    let size = CAMERA_MASK_SIZE;
    let mut mask = vec![0.0f32; size * size];
    let cx = native_circle_integer(centre[0]);
    let cy = native_circle_integer(centre[1]);
    let radius_integer = native_circle_integer(radius);

    draw_filled_circle(&mut mask, size, cx, cy, radius_integer);

    let row_lo = radius.mul_add(-WEDGE_COS_20_DEG, centre[1]).max(0.0) as i64;
    let row_hi = radius.mul_add(WEDGE_COS_20_DEG, centre[1]).min(size as f32) as i64;
    let half_field_law = radius_law(HALF_FIELD_DEG);
    let columns = match lens {
        Lens::A => (size / 2 + 1..size).rev().collect::<Vec<_>>(),
        Lens::B => (0..size / 2).collect::<Vec<_>>(),
    };
    for row in row_lo..row_hi {
        let dy = row as f32 - centre[1];
        let dy_squared = dy * dy;
        for &col in &columns {
            let dx = col as f32 - centre[0];
            let phi_radians = match lens {
                Lens::A => dy.abs().atan2(dx.abs()),
                Lens::B => (dy / dx).abs().atan(),
            };
            let phi_degrees = (f64::from(phi_radians * 180.0) / std::f64::consts::PI) as f32;
            let limit_radius = theta_limit(phi_degrees).map_or(radius, |theta| {
                let law = radius_law(theta);
                scale * (image_circle_radius * law / half_field_law)
            });
            let reach = dx.mul_add(dx, dy_squared).sqrt();
            if reach > limit_radius {
                mask[row as usize * size + col] = 0.0;
            } else {
                break;
            }
        }
    }
    mask.into_boxed_slice()
}

/// Reproduce OpenCV 4.7's native `distanceTransform(DIST_L2, 5, CV_32F)`,
/// followed by the selected `convertTo` scale and `THRESH_TRUNC` at one.
///
/// OpenCV represents the chamfer costs as unsigned 16.16 fixed point. The
/// neighbour order below is the order in the two scalar passes; retaining it
/// also keeps this readable against `distransform.cpp`.
fn condition_camera_support(binary: &[f32], rows: usize, cols: usize) -> Box<[f32]> {
    debug_assert_eq!(binary.len(), rows * cols);
    const INITIAL: u32 = i32::MAX as u32;
    const FORWARD: [(isize, isize, u32); 8] = [
        (-2, -1, DISTANCE_LONG),
        (-2, 1, DISTANCE_LONG),
        (-1, -2, DISTANCE_LONG),
        (-1, -1, DISTANCE_DIAGONAL),
        (-1, 0, DISTANCE_HV),
        (-1, 1, DISTANCE_DIAGONAL),
        (-1, 2, DISTANCE_LONG),
        (0, -1, DISTANCE_HV),
    ];
    const BACKWARD: [(isize, isize, u32); 8] = [
        (2, 1, DISTANCE_LONG),
        (2, -1, DISTANCE_LONG),
        (1, 2, DISTANCE_LONG),
        (1, 1, DISTANCE_DIAGONAL),
        (1, 0, DISTANCE_HV),
        (1, -1, DISTANCE_DIAGONAL),
        (1, -2, DISTANCE_LONG),
        (0, 1, DISTANCE_HV),
    ];

    let mut distance = binary
        .iter()
        .map(|value| if *value == 0.0 { 0 } else { INITIAL })
        .collect::<Vec<_>>();
    let relax = |distance: &[u32], row: usize, col: usize, neighbours: &[(isize, isize, u32)]| {
        let mut best = distance[row * cols + col];
        for &(dr, dc, cost) in neighbours {
            let source_row = row as isize + dr;
            let source_col = col as isize + dc;
            if source_row < 0
                || source_col < 0
                || source_row >= rows as isize
                || source_col >= cols as isize
            {
                continue;
            }
            let source = distance[source_row as usize * cols + source_col as usize];
            best = best.min(source.saturating_add(cost));
        }
        best
    };

    for row in 0..rows {
        for col in 0..cols {
            let index = row * cols + col;
            if binary[index] != 0.0 {
                distance[index] = relax(&distance, row, col, &FORWARD);
            }
        }
    }
    for row in (0..rows).rev() {
        for col in (0..cols).rev() {
            let index = row * cols + col;
            if binary[index] != 0.0 {
                distance[index] = relax(&distance, row, col, &BACKWARD);
            }
        }
    }

    let fixed_scale = 1.0_f32 / (1_u32 << DISTANCE_SHIFT) as f32;
    distance
        .into_iter()
        .map(|value| {
            let transformed = value.min(DISTANCE_MAX) as f32 * fixed_scale;
            (transformed * DISTANCE_CONDITION_SCALE).min(1.0)
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

/// Studio narrows each scalar to binary32 and ARM64 `fcvtzs` converts it to
/// the signed integer passed to `cv::circle`. The mask geometry is positive,
/// so Rust's saturating float cast has the same truncation-toward-zero result.
fn native_circle_integer(value: f32) -> i64 {
    value as i64
}

fn draw_filled_circle(mask: &mut [f32], size: usize, cx: i64, cy: i64, radius: i64) {
    debug_assert_eq!(mask.len(), size * size);

    fn span(mask: &mut [f32], size: usize, y: i64, x0: i64, x1: i64) {
        let last = size as i64 - 1;
        if y < 0 || y > last || x1 < 0 || x0 > last || x0 > x1 {
            return;
        }
        let lo = x0.max(0) as usize;
        let hi = x1.min(last) as usize;
        let row = y as usize * size;
        for value in &mut mask[row + lo..=row + hi] {
            *value = 1.0;
        }
    }

    let (mut error, mut dx, mut dy, mut plus, mut minus) =
        (0i64, radius, 0i64, 1i64, (radius << 1) - 1);
    while dx >= dy {
        span(mask, size, cy - dy, cx - dx, cx + dx);
        span(mask, size, cy + dy, cx - dx, cx + dx);
        span(mask, size, cy - dx, cx - dy, cx + dy);
        span(mask, size, cy + dx, cx - dy, cx + dy);
        dy += 1;
        error += plus;
        plus += 2;
        let carry = i64::from(error <= 0) - 1;
        error -= minus & carry;
        dx += carry;
        minus -= carry & 2;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ClampedAxis {
    lo: usize,
    hi: usize,
    frac: f32,
    value: f32,
}

impl ClampedAxis {
    fn new(value: f32, length: usize) -> Self {
        debug_assert!(length > 0);
        let value = value.clamp(0.0, (length - 1) as f32);
        // A dynamic semantic source is normally only a few thousand pixels
        // wide. Keep the helper total even beyond f32's exact-integer range:
        // `(length - 1) as f32` can round up to `length`, so clamp the integer
        // conversion once more before indexing.
        let lo = (value.trunc() as usize).min(length - 1);
        Self {
            lo,
            hi: (lo + 1).min(length - 1),
            frac: value - lo as f32,
            value,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct BilinearWeights {
    top_left: f32,
    top_right: f32,
    bottom_left: f32,
    bottom_right: f32,
}

impl BilinearWeights {
    fn new(row: ClampedAxis, col: ClampedAxis) -> Self {
        // Both selected ARM64 helpers construct the two low-side weights as
        // `(1 - coordinate) + trunc(coordinate)`, not `1 - fraction`.
        let left = (1.0_f32 - col.value) + col.lo as f32;
        let top = (1.0_f32 - row.value) + row.lo as f32;
        let top_left = left * top;
        let top_right = top - top_left;
        let bottom_left = left - top_left;
        let bottom_right = ((1.0_f32 - left) - top) + top_left;
        Self {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        }
    }
}

fn lerp(a: f32, b: f32, fraction: f32) -> f32 {
    a + (b - a) * fraction
}

fn sample_source_uv(source: &SourceImage, [u, v]: [f32; 2]) -> u8 {
    if !(u > 0.0 && v > 0.0) {
        return 0;
    }
    source.sample_clamped(v * source.rows as f32, u * source.cols as f32)
}

/// Reconstruct selected panotype 5's source-belt coordinate, gate and
/// conversion semantics.
///
/// The pair remains named at both inputs and the return boundary. Lens A reads
/// only source A through base map A; lens B reads only source B through base
/// map B. Strict ordered positivity is tested before source-coordinate
/// multiplication, so zero, negative and NaN UV components write black while
/// positive values above one clamp to the final source pixel. Selected
/// intensity adjustment is disabled, so this output passes directly to the
/// separately exact 3-by-3 area reduction.
pub fn sample_source_belts(
    sources: &LensPair<SourceImage>,
    base_maps: &RetainedBaseMaps,
) -> SourceBelts {
    const RETAINED_SCALE: f32 = 1.0 / AREA_SCALE as f32;
    SourceBelts::from_fn(|lens, row, col| {
        let [u, v] = base_maps.sample_clamped(
            lens,
            row as f32 * RETAINED_SCALE,
            col as f32 * RETAINED_SCALE,
        );
        sample_source_uv(sources.get(lens), [u, v])
    })
}

/// A pair of fixed-shape single-channel images, lens A followed by lens B in
/// the private flat storage.
///
/// Rows are always the along-seam axis and columns the across-seam axis. The
/// element type is a byte because both captured native payloads are
/// `CV_8UC1`; no normalised-float or no-picture sentinel is admitted here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Belts<const ROWS: usize, const COLS: usize> {
    pixels: Box<[u8]>,
}

impl<const ROWS: usize, const COLS: usize> Belts<ROWS, COLS> {
    /// The number of bytes in the two compact row-major images.
    pub const BYTES: usize = LENSES * ROWS * COLS;

    /// Admit two named compact row-major lens images.
    pub fn from_lenses(lenses: LensPair<Vec<u8>>) -> Result<Self, ShapeError> {
        let expected = ROWS * COLS;
        for lens in Lens::ALL {
            let pixels = lenses.get(lens);
            if pixels.len() != expected {
                return Err(ShapeError {
                    lens,
                    expected,
                    actual: pixels.len(),
                });
            }
        }

        let mut pixels = Vec::with_capacity(Self::BYTES);
        pixels.extend(lenses.a);
        pixels.extend(lenses.b);
        Ok(Self {
            pixels: pixels.into_boxed_slice(),
        })
    }

    /// Build a synthetic or decoded pair without exposing its flat indexing.
    pub fn from_fn(mut pixel: impl FnMut(Lens, usize, usize) -> u8) -> Self {
        let mut pixels = Vec::with_capacity(Self::BYTES);
        for lens in Lens::ALL {
            for row in 0..ROWS {
                for col in 0..COLS {
                    pixels.push(pixel(lens, row, col));
                }
            }
        }
        Self {
            pixels: pixels.into_boxed_slice(),
        }
    }

    /// One lens's compact row-major payload.
    pub fn lens(&self, lens: Lens) -> &[u8] {
        let per_lens = ROWS * COLS;
        let start = lens.index() * per_lens;
        &self.pixels[start..start + per_lens]
    }

    /// One byte at `(row along seam, column across seam)`.
    pub fn pixel(&self, lens: Lens, row: usize, col: usize) -> u8 {
        assert!(row < ROWS, "ONE X2 belt row is outside its grid");
        assert!(col < COLS, "ONE X2 belt column is outside its grid");
        self.lens(lens)[row * COLS + col]
    }

    /// Both compact native-order payloads as one byte slice.
    pub fn bytes(&self) -> &[u8] {
        &self.pixels
    }
}

/// The captured 3,240-row by 180-column U8 staging pair.
pub type SourceBelts = Belts<{ one_xs::SOURCE_ROWS }, { one_xs::SOURCE_COLS }>;

/// The selected 1,080-row by 60-column U8 flow-solver input pair.
pub type SolverBelts = Belts<{ one_xs::ROWS }, { one_xs::COLS }>;

impl SourceBelts {
    /// OpenCV's exact integer-ratio `INTER_AREA` result for the selected route.
    ///
    /// Each destination byte owns one non-overlapping 3 by 3 source cell. A
    /// nine-byte sum cannot tie at half an integer, so OpenCV's nearest-even
    /// conversion and `(sum + 4) / 9` agree for every possible cell.
    pub fn reduce_area_3x3(&self) -> SolverBelts {
        debug_assert_eq!(one_xs::SOURCE_ROWS, AREA_SCALE * one_xs::ROWS);
        debug_assert_eq!(one_xs::SOURCE_COLS, AREA_SCALE * one_xs::COLS);

        SolverBelts::from_fn(|lens, row, col| {
            let source_row = AREA_SCALE * row;
            let source_col = AREA_SCALE * col;
            let mut sum = 0u16;
            for dr in 0..AREA_SCALE {
                for dc in 0..AREA_SCALE {
                    sum += u16::from(self.pixel(lens, source_row + dr, source_col + dc));
                }
            }
            ((sum + 4) / 9) as u8
        })
    }
}

/// A fixed-grid payload had the wrong number of bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShapeError {
    lens: Lens,
    expected: usize,
    actual: usize,
}

impl fmt::Display for ShapeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "ONE X2 lens {} belt has {} bytes, expected {}",
            self.lens, self.actual, self.expected
        )
    }
}

impl Error for ShapeError {}

/// WGSL helpers for the independently closed retained-line coordinate test.
///
/// `one_xs_retained_line_ray` is the closed native `+0x810` inverse, and
/// `one_xs_retained_body_ray` applies the independently closed registration
/// `body=[-line.x,line.z,line.y]`. These helpers do not supply the dynamic
/// target source bytes or the history/calibration-dependent target state.
fn math_wgsl() -> String {
    format!(
        "const ONE_XS_ROWS = {rows}u;\n\
         const ONE_XS_COLS = {cols}u;\n\
         const ONE_XS_ROW_PITCH_DEG = {pitch:?};\n\
         const ONE_XS_GNOMONIC_PIXELS = {gnomonic:?};\n\
         const ONE_XS_CENTRE_COL = {centre:?};\n\
         fn one_xs_line_to_body_ray(line: vec3<f32>) -> vec3<f32> {{\n\
         \x20 return vec3<f32>(-line.x, line.z, line.y);\n\
         }}\n\
         fn one_xs_body_to_line_ray(body: vec3<f32>) -> vec3<f32> {{\n\
         \x20 return vec3<f32>(-body.x, body.z, body.y);\n\
         }}\n\
         fn one_xs_retained_line_ray(sample: vec2<f32>) -> vec3<f32> {{\n\
         \x20 // sample.x is the across-seam column; sample.y is the along-seam row.\n\
         \x20 let row = clamp(sample.y, 0.0, f32(ONE_XS_ROWS - 1u));\n\
         \x20 let col = clamp(sample.x, 0.0, f32(ONE_XS_COLS - 1u));\n\
         \x20 let phi = radians(-200.0 + row * ONE_XS_ROW_PITCH_DEG);\n\
         \x20 let t = (ONE_XS_CENTRE_COL - col) / ONE_XS_GNOMONIC_PIXELS;\n\
         \x20 let rho = inverseSqrt(1.0 + t * t);\n\
         \x20 return vec3<f32>(sin(phi) * rho, t * rho, cos(phi) * rho);\n\
         }}\n\
         fn one_xs_retained_body_ray(sample: vec2<f32>) -> vec3<f32> {{\n\
         \x20 return one_xs_line_to_body_ray(one_xs_retained_line_ray(sample));\n\
         }}\n",
        rows = one_xs::ROWS,
        cols = one_xs::COLS,
        pitch = one_xs::DEGREES_PER_ROW,
        gnomonic = one_xs::GNOMONIC_PIXELS,
        centre = one_xs::CENTRE_COL,
    )
}

/// The exact 3 by 3 U8-equivalent reducer.
///
/// WGSL storage buffers have no portable byte scalar, so one `u32` lane holds
/// one code in `[0,255]`. The values and arithmetic are nevertheless exactly
/// U8: the future source pass writes one code per lane, and this pass writes
/// one code per lane for the solver. No conversion through normalised float is
/// involved.
pub fn reduction_wgsl() -> String {
    format!(
        "{}\n\
         const ONE_XS_SOURCE_ROWS = {source_rows}u;\n\
         const ONE_XS_SOURCE_COLS = {source_cols}u;\n\
         const ONE_XS_AREA_SCALE = {scale}u;\n\
         const ONE_XS_SOURCE_PIXELS = ONE_XS_SOURCE_ROWS * ONE_XS_SOURCE_COLS;\n\
         const ONE_XS_SOLVER_PIXELS = ONE_XS_ROWS * ONE_XS_COLS;\n\
         @group(0) @binding(0) var<storage, read> source_belts: array<u32>;\n\
         @group(0) @binding(1) var<storage, read_write> solver_belts: array<u32>;\n\
         @compute @workgroup_size(64)\n\
         fn one_xs_reduce_area_3x3(@builtin(global_invocation_id) id: vec3<u32>) {{\n\
         \x20 let index = id.x;\n\
         \x20 if index >= {lenses}u * ONE_XS_SOLVER_PIXELS {{ return; }}\n\
         \x20 let lens = index / ONE_XS_SOLVER_PIXELS;\n\
         \x20 let local = index - lens * ONE_XS_SOLVER_PIXELS;\n\
         \x20 let row = local / ONE_XS_COLS;\n\
         \x20 let col = local - row * ONE_XS_COLS;\n\
         \x20 let source_row = ONE_XS_AREA_SCALE * row;\n\
         \x20 let source_col = ONE_XS_AREA_SCALE * col;\n\
         \x20 var sum = 0u;\n\
         \x20 for (var dr = 0u; dr < ONE_XS_AREA_SCALE; dr += 1u) {{\n\
         \x20   for (var dc = 0u; dc < ONE_XS_AREA_SCALE; dc += 1u) {{\n\
         \x20     let at = lens * ONE_XS_SOURCE_PIXELS\n\
         \x20       + (source_row + dr) * ONE_XS_SOURCE_COLS + source_col + dc;\n\
         \x20     sum += source_belts[at];\n\
         \x20   }}\n\
         \x20 }}\n\
         \x20 solver_belts[index] = (sum + 4u) / 9u;\n\
         }}\n",
        math_wgsl(),
        source_rows = one_xs::SOURCE_ROWS,
        source_cols = one_xs::SOURCE_COLS,
        scale = AREA_SCALE,
        lenses = LENSES,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAY_PROBE: &str = r#"
@group(0) @binding(0) var<storage, read> samples: array<vec2<f32>>;
@group(0) @binding(1) var<storage, read_write> rays: array<vec4<f32>>;

@compute @workgroup_size(64)
fn one_xs_ray_probe(@builtin(global_invocation_id) id: vec3<u32>) {
  let index = id.x;
  if index >= arrayLength(&samples) { return; }
  let body = one_xs_retained_body_ray(samples[index]);
  rays[2u * index] = vec4<f32>(one_xs_body_to_line_ray(body), 0.0);
  rays[2u * index + 1u] = vec4<f32>(body, 0.0);
}
"#;

    fn patterned_source() -> SourceBelts {
        SourceBelts::from_fn(|lens, row, col| {
            // Constant within each 3x3 destination footprint except for the
            // two local coordinates. Their means are both one, so the exact
            // expected output is easy to state independently of the reducer.
            let base = lens.index() * 40 + (row / 3 % 5) * 20 + (col / 3 % 7) * 2;
            (base + row % 3 + col % 3) as u8
        })
    }

    fn compact_source(
        rows: usize,
        cols: usize,
        mut pixel: impl FnMut(usize, usize) -> u8,
    ) -> SourceImage {
        let pixels = (0..rows)
            .flat_map(|row| (0..cols).map(move |col| (row, col)))
            .map(|(row, col)| pixel(row, col))
            .collect();
        SourceImage::from_compact(rows, cols, pixels).unwrap()
    }

    fn set_map_quad(map: &mut [[f32; 2]], row: usize, col: usize, values: [[[f32; 2]; 2]; 2]) {
        for (dr, values) in values.into_iter().enumerate() {
            for (dc, uv) in values.into_iter().enumerate() {
                map[(row + dr) * one_xs::COLS + col + dc] = uv;
            }
        }
    }

    fn synthetic_camera_support(a: f32, b: f32) -> CameraMaskSupport {
        let geometry = CameraMaskGeometry {
            centre: [200.0, 200.0],
            image_circle_radius: 0.0,
            mask_radius: 0.0,
            scale: 1.0,
            support_pixels: 0,
        };
        CameraMaskSupport {
            pixels: [
                vec![a; CAMERA_MASK_SIZE * CAMERA_MASK_SIZE].into_boxed_slice(),
                vec![b; CAMERA_MASK_SIZE * CAMERA_MASK_SIZE].into_boxed_slice(),
            ],
            geometry: [geometry; LENSES],
        }
    }

    /// Deliberately direct test oracle for the selected rectangular erosion.
    /// Keep this independent of the production separable schedule.
    fn naive_erode_rect_9x9(mask: &[u8], cols: usize, rows: usize) -> Vec<u8> {
        const RADIUS: isize = 4;
        let mut output = vec![0; mask.len()];
        for row in 0..rows {
            for col in 0..cols {
                let mut minimum = u8::MAX;
                for row_delta in -RADIUS..=RADIUS {
                    for col_delta in -RADIUS..=RADIUS {
                        let source_row = row as isize + row_delta;
                        let source_col = col as isize + col_delta;
                        if source_row < 0
                            || source_col < 0
                            || source_row >= rows as isize
                            || source_col >= cols as isize
                        {
                            continue;
                        }
                        minimum =
                            minimum.min(mask[source_row as usize * cols + source_col as usize]);
                    }
                }
                output[row * cols + col] = minimum;
            }
        }
        output
    }

    #[test]
    fn separable_erosion_matches_naive_oracle_at_every_boundary_shape() {
        for (rows, cols) in [
            (1, 1),
            (1, 12),
            (12, 1),
            (3, 4),
            (8, 8),
            (9, 9),
            (10, 11),
            (19, 23),
        ] {
            let mut mask = vec![255; rows * cols];
            let probes = [
                (0, 0),
                (0, cols - 1),
                (rows - 1, 0),
                (rows - 1, cols - 1),
                (rows / 2, cols / 2),
            ];
            for (probe, (row, col)) in probes.into_iter().enumerate() {
                mask[row * cols + col] = probe as u8;
            }
            assert_eq!(
                erode_rect_9x9(&mask, cols, rows),
                naive_erode_rect_9x9(&mask, cols, rows),
                "{rows}-by-{cols} boundary mask",
            );
        }
    }

    #[test]
    fn separable_erosion_matches_naive_oracle_for_random_masks() {
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next_byte = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        };
        for (rows, cols) in [(2, 17), (7, 13), (9, 31), (18, 5), (37, 41)] {
            for case in 0..16 {
                let mask = (0..rows * cols).map(|_| next_byte()).collect::<Vec<_>>();
                assert_eq!(
                    erode_rect_9x9(&mask, cols, rows),
                    naive_erode_rect_9x9(&mask, cols, rows),
                    "random case {case} on {rows}-by-{cols}",
                );
            }
        }
    }

    #[test]
    fn camera_mask_constants_and_interpolation_match_the_read_literals() {
        assert_eq!(CAMERA_MASK_SIZE, 400);
        assert_eq!(camera_mask_ranges(), [(0, 216), (864, 1080)]);
        assert_eq!(RADIUS_COEFFICIENTS[0].to_bits(), 0x0000_0000_0000_0000);
        assert_eq!(RADIUS_COEFFICIENTS[1].to_bits(), 0x3f98_b63b_65dc_a8b8);
        assert_eq!(RADIUS_COEFFICIENTS[2].to_bits(), 0xbf13_88ad_c1fa_4adc);
        assert_eq!(RADIUS_COEFFICIENTS[3].to_bits(), 0x3ec1_7b40_3bab_58c0);
        assert_eq!(RADIUS_COEFFICIENTS[4].to_bits(), 0xbe4f_4bb5_1d08_959d);
        assert_eq!(radius_law(98.0).to_bits(), 0x4011_0f57);
        assert_eq!(radius_law(100.0).to_bits(), 0x4012_dcf8);

        for [azimuth, limit] in AZIMUTH_LIMITS[..AZIMUTH_LIMITS.len() - 1].iter().copied() {
            assert_eq!(theta_limit(azimuth), Some(limit));
        }
        assert_eq!(theta_limit(15.5), Some(91.0));
        assert_eq!(theta_limit(70.0), None);
    }

    #[test]
    fn camera_mask_uses_the_crop_translated_one_x2_centres() {
        let reframe = Reframe::new(
            &crate::projection::tests::one_xs_lenses(),
            crate::projection::tests::ONE_XS_FRAME,
            crate::Camera::default(),
            crate::Held::default(),
            1.0,
            false,
            crate::Sampling::default(),
        );
        let support = CameraMaskSupport::from_reframe(&reframe).unwrap();

        assert_eq!(
            support
                .geometry
                .map(|geometry| geometry.centre.map(f32::to_bits)),
            [[0x4349_60b7, 0x4349_660c], [0x4347_e4fd, 0x4346_cef0],]
        );
    }

    #[test]
    fn midpoint_filled_circle_has_the_opencv_line_eight_footprint() {
        let mut mask = vec![0.0; 9 * 9];
        draw_filled_circle(&mut mask, 9, 4, 4, 3);
        let rows = mask
            .chunks_exact(9)
            .map(|row| {
                row.iter()
                    .map(|value| if *value == 0.0 { '.' } else { '#' })
                    .collect::<String>()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                ".........",
                "....#....",
                "..#####..",
                "..#####..",
                ".#######.",
                "..#####..",
                "..#####..",
                "....#....",
                ".........",
            ]
        );
    }

    #[test]
    fn native_circle_integerization_truncates_positive_binary32_values() {
        assert_eq!(native_circle_integer(199.900), 199);
        assert_eq!(native_circle_integer(198.871), 198);
        assert_eq!(native_circle_integer(203.999), 203);

        // These values all select the next integer under the old nearest
        // conversion, so the regression cannot pass with `.round()` restored.
        assert_eq!(199.900_f64.round() as i64, 200);
        assert_eq!(198.871_f64.round() as i64, 199);
        assert_eq!(203.999_f64.round() as i64, 204);
    }

    #[test]
    fn camera_wedge_rasters_are_mirrored_and_stable() {
        let a = draw_camera_support([200.0, 200.0], 190.0, 195.0, 1.0, Lens::A);
        let b = draw_camera_support([200.0, 200.0], 190.0, 195.0, 1.0, Lens::B);
        let count = |mask: &[f32]| mask.iter().filter(|value| **value != 0.0).count();

        assert_eq!(count(&a), 111_872);
        assert_eq!(count(&a), count(&b));
        for row in 0..CAMERA_MASK_SIZE {
            for col in 1..CAMERA_MASK_SIZE {
                assert_eq!(
                    a[row * CAMERA_MASK_SIZE + col],
                    b[row * CAMERA_MASK_SIZE + CAMERA_MASK_SIZE - col],
                    "mirror at row {row}, col {col}",
                );
            }
        }
    }

    #[test]
    fn native_distance_transform_matches_opencv_five_by_five_bits() {
        let mut binary = vec![1.0; 25];
        binary[2 * 5 + 2] = 0.0;
        let conditioned = condition_camera_support(&binary, 5, 5);
        let bits = conditioned
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>();
        assert_eq!(
            bits,
            [
                0x3f2e_d419,
                0x3f09_2c19,
                0x3ef9_c190,
                0x3f09_2c19,
                0x3f2e_d419,
                0x3f09_2c19,
                0x3eae_d419,
                0x3e79_c190,
                0x3eae_d419,
                0x3f09_2c19,
                0x3ef9_c190,
                0x3e79_c190,
                0x0000_0000,
                0x3e79_c190,
                0x3ef9_c190,
                0x3f09_2c19,
                0x3eae_d419,
                0x3e79_c190,
                0x3eae_d419,
                0x3f09_2c19,
                0x3f2e_d419,
                0x3f09_2c19,
                0x3ef9_c190,
                0x3f09_2c19,
                0x3f2e_d419,
            ]
        );
    }

    #[test]
    fn native_distance_transform_matches_full_opencv_mask_hash() {
        use sha2::{Digest as _, Sha256};

        let binary = (0..400)
            .flat_map(|row| (0..400).map(move |col| (row, col)))
            .map(|(row, col)| {
                let dr = row - 197;
                let dc = col - 203;
                if dr * dr + dc * dc <= 185 * 185 && !(row < 37 && col > 210) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect::<Vec<_>>();
        let conditioned = condition_camera_support(&binary, 400, 400);
        let mut digest = Sha256::new();
        for value in conditioned.iter() {
            digest.update(value.to_bits().to_le_bytes());
        }
        let digest = digest.finalize();
        assert_eq!(
            digest.as_slice(),
            [
                0x80, 0x3a, 0x12, 0x5b, 0xf3, 0x55, 0x7d, 0x4f, 0xb0, 0xe0, 0xb7, 0x48, 0x6f, 0x21,
                0xb5, 0x2f, 0xb2, 0xd7, 0x2d, 0x22, 0xa6, 0xed, 0x60, 0x81, 0x63, 0x10, 0xce, 0x35,
                0x7e, 0x8f, 0x45, 0x10,
            ]
        );
        assert_eq!(
            conditioned.iter().filter(|value| **value != 0.0).count(),
            106_160
        );
        assert_eq!(
            conditioned.iter().filter(|value| **value == 1.0).count(),
            101_761
        );
    }

    #[test]
    fn camera_sample_cut_uses_the_promoted_float_against_binary64() {
        assert!(conditioned_sample_clears(0.0));
        assert!(conditioned_sample_clears(1e-8_f32));
        assert!(!conditioned_sample_clears(f32::from_bits(
            1e-8_f32.to_bits() + 1
        )));
    }

    #[test]
    fn frame_4565_ambiguous_binary_sample_is_cleared_after_conditioning() {
        let mut support = synthetic_camera_support(0.0, 0.0);
        support.pixels[Lens::B.index()][101 * CAMERA_MASK_SIZE + 29] = DISTANCE_CONDITION_SCALE;
        let sample = support.sample(Lens::B, [0.070_000_015, 0.250_008_05]);

        // Owner clip frame 4565, belt row 942, col 19. The old binary bound
        // saw 1.8422725e-8 and refused the frame. Studio samples the conditioned
        // distance field, whose sole positive tap is one axial unit scaled by
        // 0.24390244, so the resulting value is below its binary64 cutoff.
        assert_eq!(sample.to_bits(), 0x319a_63e7);
        assert!(conditioned_sample_clears(sample));
    }

    #[test]
    fn camera_mask_sampling_uses_full_size_bilinear_coordinates_and_clamp() {
        let mut support = synthetic_camera_support(0.0, 0.0);
        let pixels = &mut support.pixels[Lens::A.index()];
        let at = |row, col| row * CAMERA_MASK_SIZE + col;
        pixels[at(20, 10)] = 0.0;
        pixels[at(20, 11)] = 4.0;
        pixels[at(21, 10)] = 8.0;
        pixels[at(21, 11)] = 12.0;
        pixels[at(399, 399)] = 17.0;

        assert_eq!(support.sample(Lens::A, [10.25 / 400.0, 20.5 / 400.0]), 5.0);
        assert_eq!(support.sample(Lens::A, [1.0, 1.0]), 17.0);
        assert_eq!(support.sample(Lens::A, [2.0, 2.0]), 17.0);
    }

    #[test]
    fn camera_clipping_walks_only_outer_rows_then_unifies_lenses() {
        let maps = RetainedBaseMaps::from_lenses(LensPair {
            a: vec![[0.5, 0.5]; RetainedBaseMaps::NODES_PER_LENS],
            b: vec![[0.5, 0.5]; RetainedBaseMaps::NODES_PER_LENS],
        })
        .unwrap();
        let support = synthetic_camera_support(0.0, 1.0);
        let (masks, report) = apply_camera_support(&maps, &support);

        assert_eq!(masks.a, masks.b);
        assert_eq!(report.ranges, [(0, 216), (864, 1080)]);
        assert_eq!(report.rows_walked, 432);
        assert_eq!(report.cleared, [25_920, 0]);
        assert_eq!(report.unified, 25_920);
        assert_eq!(report.final_valid, 38_880);
        assert_eq!(masks.a[100 * one_xs::COLS + 30], 0);
        assert_eq!(masks.a[216 * one_xs::COLS + 30], 255);
        assert_eq!(masks.a[863 * one_xs::COLS + 30], 255);
        assert_eq!(masks.a[864 * one_xs::COLS + 30], 0);
    }

    #[test]
    fn compact_sources_check_dynamic_dimensions_without_guessing_a_frame_shape() {
        let image = SourceImage::from_compact(2, 3, vec![1, 2, 3, 4, 5, 6]).unwrap();
        assert_eq!((image.rows(), image.cols()), (2, 3));
        assert_eq!(image.pixels(), &[1, 2, 3, 4, 5, 6]);

        assert_eq!(
            SourceImage::from_compact(0, 3, vec![])
                .unwrap_err()
                .to_string(),
            "ONE X2 compact source is 0 by 3; both dimensions must be nonzero"
        );
        assert_eq!(
            SourceImage::from_compact(2, 3, vec![0; 5])
                .unwrap_err()
                .to_string(),
            "ONE X2 compact source is 2 by 3 but has 5 bytes, expected 6"
        );
        assert_eq!(
            SourceImage::from_compact(MAX_EXACT_F32_DIMENSION + 1, 2, vec![])
                .unwrap_err()
                .to_string(),
            format!(
                "ONE X2 compact source is {} by 2; each dimension must be at most {} for exact \
                 f32 addressing",
                MAX_EXACT_F32_DIMENSION + 1,
                MAX_EXACT_F32_DIMENSION,
            )
        );
    }

    #[test]
    fn retained_maps_check_each_named_physical_lens() {
        assert_eq!(RetainedBaseMaps::NODES_PER_LENS, 1080 * 60);
        let error = RetainedBaseMaps::from_lenses(LensPair {
            a: vec![[0.0; 2]; RetainedBaseMaps::NODES_PER_LENS],
            b: vec![[0.0; 2]; RetainedBaseMaps::NODES_PER_LENS - 1],
        })
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ONE X2 lens B base map has 64799 float2 nodes, expected 64800"
        );
    }

    #[test]
    fn retained_map_bytes_are_packed_a_then_b_float_pairs() {
        let mut a = vec![[0.0; 2]; RetainedBaseMaps::NODES_PER_LENS];
        let mut b = a.clone();
        a[0] = [1.25, -2.5];
        a[1] = [3.75, 4.5];
        b[0] = [-5.25, 6.5];
        let maps = RetainedBaseMaps::from_lenses(LensPair { a, b }).unwrap();
        let bytes = maps.bytes();
        let pair_bytes = std::mem::size_of::<[f32; 2]>();
        let b_start = RetainedBaseMaps::NODES_PER_LENS * pair_bytes;

        assert_eq!(bytes.len(), RetainedBaseMaps::NODES * pair_bytes);
        assert_eq!(&bytes[0..4], &1.25f32.to_ne_bytes());
        assert_eq!(&bytes[4..8], &(-2.5f32).to_ne_bytes());
        assert_eq!(&bytes[8..12], &3.75f32.to_ne_bytes());
        assert_eq!(&bytes[12..16], &4.5f32.to_ne_bytes());
        assert_eq!(&bytes[b_start..b_start + 4], &(-5.25f32).to_ne_bytes());
        assert_eq!(&bytes[b_start + 4..b_start + 8], &6.5f32.to_ne_bytes());
    }

    #[test]
    fn retained_map_preserves_studio_top_right_first_fma_order() {
        let quad = [
            [
                [f32::from_bits(1_064_954_653), f32::from_bits(1_051_416_063)],
                [f32::from_bits(1_064_974_894), f32::from_bits(1_051_403_380)],
            ],
            [
                [f32::from_bits(1_064_972_601), f32::from_bits(1_051_518_451)],
                [f32::from_bits(1_064_992_780), f32::from_bits(1_051_505_921)],
            ],
        ];
        let mut a = vec![[0.0; 2]; RetainedBaseMaps::NODES_PER_LENS];
        set_map_quad(&mut a, 1, 47, quad);
        let maps = RetainedBaseMaps::from_lenses(LensPair { b: a.clone(), a }).unwrap();
        let scale = 1.0_f32 / 3.0;
        let row = 4.0_f32 * scale;
        let col = 142.0_f32 * scale;
        let sampled = maps.sample_clamped(Lens::A, row, col);

        assert_eq!(sampled.map(f32::to_bits), [1_064_967_376, 1_051_445_982]);

        // The prior top-left-first transcription differs by one binary32 bit
        // here. This negative control makes the fixture specifically guard
        // the selected ARM64 accumulation schedule, not generic bilinear math.
        let weights = BilinearWeights::new(
            ClampedAxis::new(row, one_xs::ROWS),
            ClampedAxis::new(col, one_xs::COLS),
        );
        let top_left_first = std::array::from_fn(|component| {
            let value = weights.top_left * quad[0][0][component];
            let value = weights.top_right.mul_add(quad[0][1][component], value);
            let value = weights.bottom_left.mul_add(quad[1][0][component], value);
            weights.bottom_right.mul_add(quad[1][1][component], value)
        });
        assert_ne!(sampled.map(f32::to_bits), top_left_first.map(f32::to_bits));
    }

    #[test]
    fn source_uv_uses_strict_ordered_positive_full_dimensions_and_u8_truncation() {
        let source = compact_source(2, 2, |row, col| (20 * row + 10 * col) as u8);

        // u*cols=v*rows=0.25, so the four taps yield 7.5 and fcvtzs yields 7.
        assert_eq!(sample_source_uv(&source, [0.125, 0.125]), 7);
        // Scaling by the full dimensions puts this at (row=0.5,col=1), not
        // the coordinate that multiplying by (n-1) would produce.
        assert_eq!(sample_source_uv(&source, [0.5, 0.25]), 20);
        assert_eq!(sample_source_uv(&source, [2.0, 2.0]), 30);
        assert_eq!(
            sample_source_uv(&source, [f32::INFINITY, f32::INFINITY]),
            30
        );

        for uv in [
            [0.0, 0.5],
            [-0.0, 0.5],
            [-0.25, 0.5],
            [0.5, 0.0],
            [f32::NAN, 0.5],
            [0.5, f32::NAN],
        ] {
            assert_eq!(sample_source_uv(&source, uv), 0, "gate at {uv:?}");
        }
    }

    #[test]
    fn source_sampler_preserves_studio_top_right_first_fma_order() {
        let pixels = [17, 201, 93, 248];
        let source = SourceImage::from_compact(2, 2, pixels.to_vec()).unwrap();
        let row = 251.0_f32 / 1000.0;
        let col = 871.0_f32 / 1000.0;

        assert_eq!(source.sample_clamped(row, col), 190);

        let weights = BilinearWeights::new(ClampedAxis::new(row, 2), ClampedAxis::new(col, 2));
        let value = weights.top_left * pixels[0] as f32;
        let value = weights.top_right.mul_add(pixels[1] as f32, value);
        let value = weights.bottom_left.mul_add(pixels[2] as f32, value);
        let top_left_first = weights.bottom_right.mul_add(pixels[3] as f32, value) as u8;
        assert_eq!(top_left_first, 189);
    }

    #[test]
    fn panotype_five_sampler_keeps_lenses_and_asymmetric_axes_in_native_order() {
        let sources = LensPair {
            a: compact_source(4, 6, |row, col| (60 * row + 10 * col) as u8),
            b: compact_source(3, 5, |row, col| (100 + 40 * row + 7 * col) as u8),
        };
        let mut a = vec![[0.0; 2]; RetainedBaseMaps::NODES_PER_LENS];
        let mut b = a.clone();
        set_map_quad(
            &mut a,
            1,
            1,
            [
                [[0.125, 0.125], [0.375, 0.125]],
                [[0.125, 0.375], [0.375, 0.375]],
            ],
        );
        set_map_quad(
            &mut b,
            1,
            1,
            [[[0.2, 0.3], [0.6, 0.3]], [[0.2, 0.7], [0.6, 0.7]]],
        );
        // The final staging coordinate samples beyond the retained grid and
        // must clamp to the final base node, then to each source's final pixel.
        *a.last_mut().unwrap() = [2.0, 2.0];
        *b.last_mut().unwrap() = [2.0, 2.0];
        let maps = RetainedBaseMaps::from_lenses(LensPair { a, b }).unwrap();

        let a_uv = maps.sample_clamped(Lens::A, 4.0 / 3.0, 5.0 / 3.0);
        let b_uv = maps.sample_clamped(Lens::B, 4.0 / 3.0, 5.0 / 3.0);
        assert!((a_uv[0] - 7.0 / 24.0).abs() < 1e-6);
        assert!((a_uv[1] - 5.0 / 24.0).abs() < 1e-6);
        assert!((b_uv[0] - 7.0 / 15.0).abs() < 1e-6);
        assert!((b_uv[1] - 13.0 / 30.0).abs() < 1e-6);

        let belts = sample_source_belts(&sources, &maps);
        // (row=4,col=5) samples the base at (4/3,5/3), with no half pixel.
        // The two unequal source shapes and patterns make lens exchange and
        // row/column transposition independently visible.
        assert_eq!(belts.pixel(Lens::A, 4, 5), 67);
        assert_eq!(belts.pixel(Lens::B, 4, 5), 168);
        assert_eq!(belts.pixel(Lens::A, 0, 0), 0);
        assert_eq!(belts.pixel(Lens::B, 0, 0), 0);
        assert_eq!(
            belts.pixel(Lens::A, one_xs::SOURCE_ROWS - 1, one_xs::SOURCE_COLS - 1),
            230
        );
        assert_eq!(
            belts.pixel(Lens::B, one_xs::SOURCE_ROWS - 1, one_xs::SOURCE_COLS - 1),
            208
        );
    }

    #[test]
    #[ignore = "requires the accepted adjacent-source -05 directory and Run06 warm payload"]
    fn accepted_p0_sources_match_delayed_native_staging_and_run06_postblur() {
        use crate::flow::one_xs::temporal::gaussian_blur;
        use sha2::{Digest as _, Sha256};
        use std::io::{BufRead as _, Read as _};
        use std::path::Path;

        const ADJACENT_EVIDENCE_BYTES: usize = 18_213;
        const ADJACENT_EVIDENCE_SHA256: &str =
            "6af52b3dd5e34423a1346c718a9deb1c4afb234318234b69876f01f6db43867d";
        const ADJACENT_LAUNCH_BYTES: usize = 8_350;
        const ADJACENT_LAUNCH_SHA256: &str =
            "4fb31ea6e1724e59b734717847e2a2f4930c770d37ca9e200d95b2731f97cd55";
        const ADJACENT_PACKET_LOG_BYTES: u64 = 2_157_053_438;
        const ADJACENT_PACKET_LOG_SHA256: &str =
            "0b7c0670aff54c0cd80844c619cfec56142345783aefe2d37e54aa1e97c8c146";
        const TARGET_IDXTIMED_SHA256: &str =
            "b2671702d16959f8c428352cbe9e38bef3b800a8408c39e1c8fbf42ab9b48bb7";
        const SOURCE_BYTES: usize = 2_880 * 2_880;
        const SOURCE_A_SHA256: &str =
            "6ff087ae2908858eb0e3a27f382b6ef0c8d2e39da50e37f442fc88524a435eec";
        const SOURCE_B_SHA256: &str =
            "f38fd360172d42388a7cb87119844ea32b38059b959b463d883b9ce76d534bb6";
        const BASE_BYTES: usize = RetainedBaseMaps::NODES_PER_LENS * 2 * size_of::<f32>();
        const BASE_A_SHA256: &str =
            "cb6f344d8c571d1c5c4f6275500b834a734004f75774cc89fdc79cbfc18f1596";
        const BASE_B_SHA256: &str =
            "323f7cf184c4594a823db8293cf791d4cda35bbb63eba20b5aa9acf17c27849b";
        const NATIVE_STAGING_A_SHA256: &str =
            "f718c029d7e2b5670b7b9b61c154231427bfe5091dba11debfb5022ff3a350f5";
        const NATIVE_STAGING_B_SHA256: &str =
            "7310e471b6fa21128493e455907584a148efff9099a9edac56701bf5a148bf3a";
        const RUN06_PAYLOAD_BYTES: usize = 7_238_679;
        const RUN06_PAYLOAD_SHA256: &str =
            "a5311370d7946d45094cc85c8ff6631bf2b1195e8c127e3b1f909310168ce8c1";
        const RUN06_EVIDENCE_BYTES: usize = 6_719_162;
        const RUN06_EVIDENCE_SHA256: &str =
            "9f8f531071a5bfcd03d6ff3eb73fc4f57b2d99b02a5dfecc0bc9497418da6baf";
        const POSTBLUR_A_OFFSET: usize = 37;
        const POSTBLUR_B_OFFSET: usize = 64_866;
        const POSTBLUR_A_SHA256: &str =
            "c802e909b7970e8e94d77c398fd3faba9040f194c190101b15f4a3e3ac201a71";
        const POSTBLUR_B_SHA256: &str =
            "0101d1c611bb5c2234019b9c5bdb4f0a34197017b782414cfd6f4b485d5e4b9d";

        fn sha256_hex(bytes: &[u8]) -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }

        fn read_authenticated(path: &Path, bytes: usize, sha256: &str) -> Vec<u8> {
            let payload = std::fs::read(path)
                .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
            assert_eq!(
                payload.len(),
                bytes,
                "{} byte count differs",
                path.display()
            );
            assert_eq!(
                sha256_hex(&payload),
                sha256,
                "{} SHA-256 differs",
                path.display()
            );
            payload
        }

        fn sha256_file(path: &Path) -> String {
            let mut file = std::fs::File::open(path)
                .unwrap_or_else(|error| panic!("could not open {}: {error}", path.display()));
            let mut digest = Sha256::new();
            let mut buffer = vec![0_u8; 1024 * 1024];
            loop {
                let read = file
                    .read(&mut buffer)
                    .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
            }
            digest
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }

        fn assert_exact(label: &str, actual: &[u8], expected: &[u8]) {
            assert_eq!(actual.len(), expected.len(), "{label} byte count differs");
            if actual == expected {
                return;
            }
            let mismatches = actual
                .iter()
                .zip(expected)
                .filter(|(actual, expected)| actual != expected)
                .count();
            let first = actual
                .iter()
                .zip(expected)
                .position(|(actual, expected)| actual != expected);
            panic!(
                "{label} differs at {mismatches} of {} bytes; first mismatch at {first:?}",
                actual.len().min(expected.len())
            );
        }

        fn float2(bytes: &[u8]) -> Vec<[f32; 2]> {
            assert_eq!(bytes.len() % 8, 0);
            bytes
                .chunks_exact(8)
                .map(|pair| {
                    [
                        f32::from_le_bytes(pair[..4].try_into().unwrap()),
                        f32::from_le_bytes(pair[4..].try_into().unwrap()),
                    ]
                })
                .collect()
        }

        fn captured_binary_read(
            log: &Path,
            start: u64,
            bytes: usize,
            occurrence: usize,
        ) -> Vec<u8> {
            let file = std::fs::File::open(log)
                .unwrap_or_else(|error| panic!("could not open {}: {error}", log.display()));
            let mut lines = std::io::BufReader::new(file).lines();
            let mut result = Vec::with_capacity(bytes);
            let mut next_address = start;
            let mut starts_seen = 0;
            while result.len() < bytes {
                let request = loop {
                    let line = lines
                        .next()
                        .unwrap_or_else(|| {
                            panic!("{} ended before read {next_address:#x}", log.display())
                        })
                        .unwrap_or_else(|error| {
                            panic!("could not read {}: {error}", log.display())
                        });
                    let Some(packet) = line.split("send packet: $x").nth(1) else {
                        continue;
                    };
                    let packet = packet
                        .split('#')
                        .next()
                        .expect("binary-read packet has a checksum delimiter");
                    let (address, count) = packet
                        .split_once(',')
                        .expect("binary-read packet has address and count");
                    let address = u64::from_str_radix(address, 16)
                        .expect("binary-read address is lowercase hexadecimal");
                    if result.is_empty() && address == start {
                        starts_seen += 1;
                    }
                    if address != next_address || (result.is_empty() && starts_seen != occurrence) {
                        continue;
                    }
                    break usize::from_str_radix(count, 16)
                        .expect("binary-read count is lowercase hexadecimal");
                };
                let line = loop {
                    let line = lines
                        .next()
                        .expect("binary-read request has no response")
                        .expect("binary-read response could not be read");
                    if line.contains("read packet: $") {
                        break line;
                    }
                };
                let encoded = line
                    .split("read packet: $")
                    .nth(1)
                    .expect("binary-read response marker disappeared")
                    .split('#')
                    .next()
                    .expect("binary-read response has a checksum delimiter");
                assert_eq!(encoded.len(), 2 * request);
                for pair in encoded.as_bytes().chunks_exact(2) {
                    result.push(
                        u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16)
                            .expect("binary-read response is hexadecimal"),
                    );
                }
                next_address += request as u64;
            }
            assert_eq!(result.len(), bytes);
            result
        }

        let adjacent = std::path::PathBuf::from(
            std::env::var_os("KJERAG_ONE_XS_ADJACENT_SOURCE_DIR").expect(
                "set KJERAG_ONE_XS_ADJACENT_SOURCE_DIR to the accepted adjacent-source -05 directory",
            ),
        );
        let adjacent_evidence = read_authenticated(
            &adjacent.join("adjacent-source-evidence.json"),
            ADJACENT_EVIDENCE_BYTES,
            ADJACENT_EVIDENCE_SHA256,
        );
        assert!(
            std::str::from_utf8(&adjacent_evidence)
                .unwrap()
                .contains(TARGET_IDXTIMED_SHA256),
            "adjacent source evidence does not name the selected target"
        );
        read_authenticated(
            &adjacent.join("adjacent-source-launch-receipt.json"),
            ADJACENT_LAUNCH_BYTES,
            ADJACENT_LAUNCH_SHA256,
        );
        let source_a = read_authenticated(
            &adjacent.join("adjacent-source-p0-left.bin"),
            SOURCE_BYTES,
            SOURCE_A_SHA256,
        );
        let source_b = read_authenticated(
            &adjacent.join("adjacent-source-p0-right.bin"),
            SOURCE_BYTES,
            SOURCE_B_SHA256,
        );

        let warm_payload_path = std::path::PathBuf::from(
            std::env::var_os("KJERAG_ONE_XS_WARM_PAYLOAD")
                .expect("set KJERAG_ONE_XS_WARM_PAYLOAD to the accepted Run06 payload"),
        );
        let warm_root = warm_payload_path
            .parent()
            .expect("the accepted Run06 payload must have a parent directory");
        let base_a = read_authenticated(
            &warm_root.join("line-map-base-left-8d0.bin"),
            BASE_BYTES,
            BASE_A_SHA256,
        );
        let base_b = read_authenticated(
            &warm_root.join("line-map-base-right-930.bin"),
            BASE_BYTES,
            BASE_B_SHA256,
        );
        let sources = LensPair {
            a: SourceImage::from_compact(2_880, 2_880, source_a).unwrap(),
            b: SourceImage::from_compact(2_880, 2_880, source_b).unwrap(),
        };
        let bases = RetainedBaseMaps::from_lenses(LensPair {
            a: float2(&base_a),
            b: float2(&base_b),
        })
        .unwrap();

        let staging = sample_source_belts(&sources, &bases);

        let packet_log = adjacent.join("lldb-packets.log");
        let packet_metadata = std::fs::symlink_metadata(&packet_log)
            .unwrap_or_else(|error| panic!("could not inspect {}: {error}", packet_log.display()));
        assert!(
            packet_metadata.file_type().is_file(),
            "{} is not a regular file",
            packet_log.display()
        );
        assert_eq!(
            packet_metadata.len(),
            ADJACENT_PACKET_LOG_BYTES,
            "{} byte count differs",
            packet_log.display()
        );
        assert_eq!(
            sha256_file(&packet_log),
            ADJACENT_PACKET_LOG_SHA256,
            "{} SHA-256 differs",
            packet_log.display()
        );
        // Each dispatch receipt samples the destination before its BL executes.
        // The second read of these same two buffers is therefore the P1
        // pre-call value. Its exact join to both this P0 reconstruction and
        // Run06 postblur is corpus-scoped; it is not a general lifetime claim.
        let native_staging_a = captured_binary_read(&packet_log, 0x82c6_ac000, 583_200, 2);
        let native_staging_b = captured_binary_read(&packet_log, 0x82c7_3c000, 583_200, 2);
        assert_eq!(sha256_hex(&native_staging_a), NATIVE_STAGING_A_SHA256);
        assert_eq!(sha256_hex(&native_staging_b), NATIVE_STAGING_B_SHA256);
        assert_exact("lens A staging", staging.lens(Lens::A), &native_staging_a);
        assert_exact("lens B staging", staging.lens(Lens::B), &native_staging_b);

        let warm_payload = read_authenticated(
            &warm_payload_path,
            RUN06_PAYLOAD_BYTES,
            RUN06_PAYLOAD_SHA256,
        );
        let warm_evidence = read_authenticated(
            &warm_root.join("warm-pair-evidence.json"),
            RUN06_EVIDENCE_BYTES,
            RUN06_EVIDENCE_SHA256,
        );
        assert!(
            std::str::from_utf8(&warm_evidence)
                .unwrap()
                .contains(TARGET_IDXTIMED_SHA256),
            "Run06 evidence does not name the selected target"
        );
        assert_eq!(&warm_payload[..8], b"KJWP602\x04");
        let postblur = gaussian_blur(&staging.reduce_area_3x3());
        let expected_a =
            &warm_payload[POSTBLUR_A_OFFSET..POSTBLUR_A_OFFSET + one_xs::ROWS * one_xs::COLS];
        let expected_b =
            &warm_payload[POSTBLUR_B_OFFSET..POSTBLUR_B_OFFSET + one_xs::ROWS * one_xs::COLS];
        assert_eq!(sha256_hex(expected_a), POSTBLUR_A_SHA256);
        assert_eq!(sha256_hex(expected_b), POSTBLUR_B_SHA256);
        let native_postblur = gaussian_blur(
            &SourceBelts::from_lenses(LensPair {
                a: native_staging_a,
                b: native_staging_b,
            })
            .unwrap()
            .reduce_area_3x3(),
        );
        assert_exact(
            "delayed native lens A postblur",
            native_postblur.lens(Lens::A),
            expected_a,
        );
        assert_exact(
            "delayed native lens B postblur",
            native_postblur.lens(Lens::B),
            expected_b,
        );
        assert_exact(
            "computed lens A postblur",
            postblur.lens(Lens::A),
            expected_a,
        );
        assert_exact(
            "computed lens B postblur",
            postblur.lens(Lens::B),
            expected_b,
        );
    }

    #[test]
    fn fixed_shapes_refuse_wrong_lens_byte_counts() {
        assert_eq!(SourceBelts::BYTES, 2 * 3240 * 180);
        assert_eq!(SolverBelts::BYTES, 2 * 1080 * 60);
        assert_eq!(one_xs::SOURCE_ROWS, AREA_SCALE * one_xs::ROWS);
        assert_eq!(one_xs::SOURCE_COLS, AREA_SCALE * one_xs::COLS);

        let error = SourceBelts::from_lenses(LensPair {
            a: vec![0; one_xs::SOURCE_ROWS * one_xs::SOURCE_COLS],
            b: vec![0; one_xs::SOURCE_ROWS * one_xs::SOURCE_COLS - 1],
        })
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ONE X2 lens B belt has 583199 bytes, expected 583200"
        );
    }

    #[test]
    fn named_lens_pair_preserves_physical_identity() {
        let belts = SolverBelts::from_lenses(LensPair {
            a: vec![7; one_xs::ROWS * one_xs::COLS],
            b: vec![19; one_xs::ROWS * one_xs::COLS],
        })
        .unwrap();
        assert!(belts.lens(Lens::A).iter().all(|value| *value == 7));
        assert!(belts.lens(Lens::B).iter().all(|value| *value == 19));
    }

    #[test]
    fn cpu_area_reduction_keeps_rows_along_and_columns_across() {
        let reduced = patterned_source().reduce_area_3x3();
        for lens in Lens::ALL {
            for row in 0..one_xs::ROWS {
                for col in 0..one_xs::COLS {
                    let expected = lens.index() * 40 + (row % 5) * 20 + (col % 7) * 2 + 2;
                    assert_eq!(reduced.pixel(lens, row, col), expected as u8);
                }
            }
        }

        // These asymmetric coordinates make a row/column transpose visible
        // in the failure rather than merely relying on the exhaustive loop.
        assert_eq!(reduced.pixel(Lens::A, 731, 23), 26);
        assert_eq!(reduced.pixel(Lens::A, 23, 59), 68);
        assert_eq!(reduced.pixel(Lens::B, 731, 23), 66);
    }

    #[test]
    fn integer_rounding_is_the_exact_u8_area_rule() {
        let mut lens0 = vec![0u8; one_xs::SOURCE_ROWS * one_xs::SOURCE_COLS];
        let lens1 = lens0.clone();
        // Sums 4, 5, 13 and 14 become 0, 1, 1 and 2. Since the denominator is
        // odd, no possible sum is an exact .5 tie.
        for (block, sum) in [4u8, 5, 13, 14].into_iter().enumerate() {
            let col = AREA_SCALE * block;
            lens0[col] = sum.min(9);
            lens0[col + 1] = sum.saturating_sub(9);
        }
        let reduced = SourceBelts::from_lenses(LensPair { a: lens0, b: lens1 })
            .unwrap()
            .reduce_area_3x3();
        assert_eq!(&reduced.lens(Lens::A)[..4], &[0, 1, 1, 2]);
    }

    #[test]
    fn gpu_reduction_and_registered_line_body_axes_match_static_and_cpu_oracles() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}",
                );
                eprintln!("skipping ONE X2 belt GPU twin: {why}");
                return;
            }
        };

        let source = patterned_source();
        let expected = source.reduce_area_3x3();
        let source_words: Vec<u32> = source
            .bytes()
            .iter()
            .map(|value| u32::from(*value))
            .collect();
        let source_buffer = storage_buffer(
            &device,
            &queue,
            "ONE X2 source belts",
            &source_words,
            wgpu::BufferUsages::STORAGE,
        );
        let solver_bytes = (SolverBelts::BYTES * std::mem::size_of::<u32>()) as u64;
        let solver_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 solver belts"),
            size: solver_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let reducer_layout = storage_layout(&device, "ONE X2 belt reducer");
        let reducer_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 belt reducer"),
            source: wgpu::ShaderSource::Wgsl(reduction_wgsl().into()),
        });
        let reducer = compute_pipeline(
            &device,
            "ONE X2 belt reducer",
            &reducer_layout,
            &reducer_module,
            "one_xs_reduce_area_3x3",
        );
        let reducer_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 belt reducer"),
            layout: &reducer_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: source_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: solver_buffer.as_entire_binding(),
                },
            ],
        });

        let at_phi = |phi: f32, t: f32| {
            one_xs::Sample::new(
                (phi + 200.0) / one_xs::DEGREES_PER_ROW,
                one_xs::CENTRE_COL - one_xs::GNOMONIC_PIXELS * t,
            )
            .unwrap()
        };
        let t = 0.125;
        let rho = (1.0_f32 + t * t).sqrt().recip();
        let samples = [
            at_phi(0.0, 0.0),
            at_phi(90.0, 0.0),
            at_phi(-30.0, t),
            one_xs::Sample::new(0.0, 0.0).unwrap(),
            one_xs::Sample::new(123.25, 7.5).unwrap(),
            one_xs::Sample::new(1016.134, 27.458).unwrap(),
            one_xs::Sample::new((one_xs::ROWS - 1) as f32, (one_xs::COLS - 1) as f32).unwrap(),
        ];
        let static_line_oracles = [
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [-0.5 * rho, t * rho, 30.0_f32.to_radians().cos() * rho],
        ];
        let static_body_oracles = [
            [0.0, 1.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.5 * rho, 30.0_f32.to_radians().cos() * rho, t * rho],
        ];
        let sample_words: Vec<u32> = samples
            .iter()
            .flat_map(|sample| [sample.col().to_bits(), sample.row().to_bits()])
            .collect();
        let sample_buffer = storage_buffer(
            &device,
            &queue,
            "ONE X2 registered samples",
            &sample_words,
            wgpu::BufferUsages::STORAGE,
        );
        let ray_bytes = (2 * samples.len() * 4 * std::mem::size_of::<f32>()) as u64;
        let ray_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 registered rays"),
            size: ray_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let ray_layout = storage_layout(&device, "ONE X2 registered axes");
        let ray_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 registered axes"),
            source: wgpu::ShaderSource::Wgsl(format!("{}\n{RAY_PROBE}", math_wgsl()).into()),
        });
        let ray_pipeline = compute_pipeline(
            &device,
            "ONE X2 registered axes",
            &ray_layout,
            &ray_module,
            "one_xs_ray_probe",
        );
        let ray_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 registered axes"),
            layout: &ray_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: sample_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: ray_buffer.as_entire_binding(),
                },
            ],
        });

        let solver_readback = readback_buffer(&device, "ONE X2 solver belts", solver_bytes);
        let ray_readback = readback_buffer(&device, "ONE X2 registered rays", ray_bytes);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&reducer);
            pass.set_bind_group(0, &reducer_group, &[]);
            pass.dispatch_workgroups((SolverBelts::BYTES as u32).div_ceil(64), 1, 1);
            pass.set_pipeline(&ray_pipeline);
            pass.set_bind_group(0, &ray_group, &[]);
            pass.dispatch_workgroups((samples.len() as u32).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&solver_buffer, 0, &solver_readback, 0, solver_bytes);
        encoder.copy_buffer_to_buffer(&ray_buffer, 0, &ray_readback, 0, ray_bytes);
        let submission = queue.submit([encoder.finish()]);

        let solver_bytes = mapped(&device, &solver_readback, submission.clone());
        let actual: Vec<u8> = solver_bytes
            .chunks_exact(4)
            .map(|bytes| u32::from_ne_bytes(bytes.try_into().unwrap()) as u8)
            .collect();
        assert_eq!(actual, expected.bytes(), "GPU area reduction on {adapter}");
        drop(solver_bytes);
        solver_readback.unmap();

        let ray_bytes = mapped(&device, &ray_readback, submission);
        let actual_rays: Vec<[f32; 3]> = ray_bytes
            .chunks_exact(16)
            .map(|lane| {
                std::array::from_fn(|component| {
                    let at = 4 * component;
                    f32::from_ne_bytes(lane[at..at + 4].try_into().unwrap())
                })
            })
            .collect();
        drop(ray_bytes);
        ray_readback.unmap();

        for probe in 0..static_line_oracles.len() {
            let actual_line = actual_rays[2 * probe];
            let actual_body = actual_rays[2 * probe + 1];
            for (frame, actual, expected) in [
                ("line", actual_line, static_line_oracles[probe]),
                ("body", actual_body, static_body_oracles[probe]),
            ] {
                for component in 0..3 {
                    assert!(
                        (actual[component] - expected[component]).abs() <= 2e-6,
                        "static {frame}-axis mismatch on {adapter} at probe {probe}, component \
                         {component}: GPU {} oracle {}",
                        actual[component],
                        expected[component],
                    );
                }
            }
        }

        for (sample, actual) in samples.into_iter().zip(actual_rays.chunks_exact(2)) {
            let expected_line = one_xs::Layout.line_ray(sample).components();
            let expected_body = one_xs::Layout.body_ray(sample).components();
            for (frame, actual, expected) in [
                ("line", actual[0], expected_line),
                ("body", actual[1], expected_body),
            ] {
                for component in 0..3 {
                    assert!(
                        (actual[component] - expected[component]).abs() <= 2e-6,
                        "registered {frame} axis mismatch on {adapter} at {sample:?}, component \
                         {component}: GPU {} CPU {}",
                        actual[component],
                        expected[component],
                    );
                }
            }
        }
    }

    #[test]
    fn gpu_belt_instrument_stays_within_the_scalar_contraction_bound() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}",
                );
                eprintln!("skipping ONE X2 belt GPU instrument: {why}");
                return;
            }
        };

        // A 256-byte row needs no upload padding. Unequal, non-constant lens
        // patterns make a lens swap, row/column transpose, wrong source extent
        // or lost bilinear fraction visible across the complete output.
        const SOURCE_ROWS: usize = 128;
        const SOURCE_COLS: usize = 256;
        let sources = LensPair {
            a: compact_source(SOURCE_ROWS, SOURCE_COLS, |row, col| {
                ((5 * row + 3 * col + 17) % 251) as u8
            }),
            b: compact_source(SOURCE_ROWS, SOURCE_COLS, |row, col| {
                ((11 * row + 7 * col + 83) % 251) as u8
            }),
        };

        let map_for = |lens: Lens| {
            (0..RetainedBaseMaps::NODES_PER_LENS)
                .map(|index| {
                    let row = index / one_xs::COLS;
                    let col = index % one_xs::COLS;
                    // Most nodes are interior fractional source coordinates.
                    // Sparse zero and over-one nodes exercise the ordered gate
                    // and clamp without reducing the fixture to edge cases.
                    if (row * 17 + col * 29 + lens.index()).is_multiple_of(521) {
                        [0.0, 0.5]
                    } else if (row * 31 + col * 7 + lens.index()).is_multiple_of(887) {
                        [1.125, 1.25]
                    } else {
                        let x = 12 + (3 * col + row % 19 + 23 * lens.index()) % 220;
                        let y = 8 + (2 * row + col % 11 + 17 * lens.index()) % 108;
                        [x as f32 / SOURCE_COLS as f32, y as f32 / SOURCE_ROWS as f32]
                    }
                })
                .collect::<Vec<_>>()
        };
        let mut a = map_for(Lens::A);
        let mut b = map_for(Lens::B);
        let constant_quad = |uv| [[uv; 2]; 2];

        // Exact, threshold-safe answers pin the semantics that a broad
        // one-code contraction bound alone cannot distinguish. Keeping each
        // UV constant across a base-map quad makes all nine staging samples
        // equal, except for the last deliberately patterned reducer anchor.
        set_map_quad(&mut a, 100, 10, constant_quad([0.0, 0.5]));
        set_map_quad(&mut a, 200, 20, constant_quad([2.0, 2.0]));
        set_map_quad(
            &mut a,
            300,
            30,
            constant_quad([20.25 / SOURCE_COLS as f32, 40.5 / SOURCE_ROWS as f32]),
        );
        set_map_quad(
            &mut a,
            400,
            40,
            constant_quad([30.0 / SOURCE_COLS as f32, 60.0 / SOURCE_ROWS as f32]),
        );
        set_map_quad(
            &mut b,
            400,
            40,
            constant_quad([25.0 / SOURCE_COLS as f32, 50.0 / SOURCE_ROWS as f32]),
        );
        set_map_quad(
            &mut a,
            500,
            50,
            [
                [
                    [61.3 / SOURCE_COLS as f32, 10.2 / SOURCE_ROWS as f32],
                    [64.3 / SOURCE_COLS as f32, 10.2 / SOURCE_ROWS as f32],
                ],
                [
                    [61.3 / SOURCE_COLS as f32, 13.2 / SOURCE_ROWS as f32],
                    [64.3 / SOURCE_COLS as f32, 13.2 / SOURCE_ROWS as f32],
                ],
            ],
        );

        let maps = RetainedBaseMaps::from_lenses(LensPair { a, b }).unwrap();
        let expected = sample_source_belts(&sources, &maps).reduce_area_3x3();

        // The reducer anchor crosses the A pattern's modulo boundary at
        // threshold-safe fractional texels. Its staging bytes are
        // 141/3/6, 5/8/11, 10/13/16: sum 213, so exact `(sum + 4) / 9` is 24
        // rather than truncating to 23 or selecting the centre byte 8.
        let exact_anchors = [
            ("ordered-positive gate", Lens::A, 100, 10, 0),
            ("source clamp", Lens::A, 200, 20, 162),
            ("U8 truncation", Lens::A, 300, 30, 29),
            ("lens A source", Lens::A, 400, 40, 156),
            ("lens B source", Lens::B, 400, 40, 55),
            ("exact 3-by-3 reducer", Lens::A, 500, 50, 24),
        ];
        let output_index = |lens: Lens, row: usize, col: usize| {
            lens.index() * RetainedBaseMaps::NODES_PER_LENS + row * one_xs::COLS + col
        };
        for &(law, lens, row, col, wanted) in &exact_anchors {
            let index = output_index(lens, row, col);
            assert_eq!(
                expected.bytes()[index],
                wanted,
                "scalar fixture no longer pins the {law} anchor",
            );
        }

        // Negative control for the coordinate bug this twin first exposed.
        // Applying a texture-centre subtraction after the positive-UV gate is
        // the former GPU law. It is a coordinate/topology error, not ordinary
        // WGSL contraction noise, and this fixture must keep separating it
        // from the scalar oracle by more than one final byte code.
        let half_pixel_source = SourceBelts::from_fn(|lens, row, col| {
            let [u, v] = maps.sample_clamped(
                lens,
                row as f32 / AREA_SCALE as f32,
                col as f32 / AREA_SCALE as f32,
            );
            if !(u > 0.0 && v > 0.0) {
                return 0;
            }
            let source = sources.get(lens);
            source.sample_clamped(
                v * source.rows() as f32 - 0.5,
                u * source.cols() as f32 - 0.5,
            )
        })
        .reduce_area_3x3();
        let half_pixel_max = half_pixel_source
            .bytes()
            .iter()
            .zip(expected.bytes())
            .map(|(actual, expected)| actual.abs_diff(*expected))
            .max()
            .unwrap();
        println!(
            "ONE X2 half-pixel negative control: max absolute {half_pixel_max} (allowed \
             contraction bound 1)"
        );
        assert!(
            half_pixel_max > 1,
            "the fixture no longer distinguishes the half-pixel topology error from the \
             one-code contraction bound"
        );

        let texture = |label: &str, source: &SourceImage| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: SOURCE_COLS as u32,
                    height: SOURCE_ROWS as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                source.pixels(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SOURCE_COLS as u32),
                    rows_per_image: Some(SOURCE_ROWS as u32),
                },
                texture.size(),
            );
            texture
        };
        let texture_a = texture("ONE X2 instrument lens A", &sources.a);
        let texture_b = texture("ONE X2 instrument lens B", &sources.b);
        let view_a = texture_a.create_view(&Default::default());
        let view_b = texture_b.create_view(&Default::default());

        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ONE X2 instrument sources"),
            entries: &[1, 3].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }),
        });
        let scene_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 instrument sources"),
            layout: &scene_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view_a),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&view_b),
                },
            ],
        });

        let flow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ONE X2 instrument maps and belts"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: crate::band::STRIP_BINDING,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: crate::band::ONE_XS_BASE_BINDING,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 instrument output"),
            size: crate::band::ONE_XS_BELT_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let map_words: Vec<u32> = maps
            .bytes()
            .chunks_exact(4)
            .map(|bytes| u32::from_ne_bytes(bytes.try_into().unwrap()))
            .collect();
        let map_buffer = storage_buffer(
            &device,
            &queue,
            "ONE X2 instrument retained maps",
            &map_words,
            wgpu::BufferUsages::STORAGE,
        );
        let flow_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 instrument maps and belts"),
            layout: &flow_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: crate::band::STRIP_BINDING,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: crate::band::ONE_XS_BASE_BINDING,
                    resource: map_buffer.as_entire_binding(),
                },
            ],
        });

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 belt instrument"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    crate::projection::wgsl(),
                    crate::band::one_xs_belt_wgsl()
                )
                .into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 belt instrument"),
            bind_group_layouts: &[&scene_layout, &flow_layout],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ONE X2 belt instrument"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("one_xs_belt"),
            compilation_options: Default::default(),
            cache: None,
        });

        let readback = readback_buffer(
            &device,
            "ONE X2 instrument readback",
            crate::band::ONE_XS_BELT_BYTES,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &scene_group, &[]);
            pass.set_bind_group(1, &flow_group, &[]);
            pass.dispatch_workgroups(crate::band::ONE_XS_BELT_GROUPS, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, crate::band::ONE_XS_BELT_BYTES);
        let submission = queue.submit([encoder.finish()]);
        let bytes = mapped(&device, &readback, submission);
        for &(law, lens, row, col, wanted) in &exact_anchors {
            let index = output_index(lens, row, col);
            let at = 4 * index;
            let actual = f32::from_ne_bytes(bytes[at..at + 4].try_into().unwrap());
            assert_eq!(
                actual,
                f32::from(wanted),
                "ONE X2 belt instrument changed the exact {law} anchor at lens {lens}, row \
                 {row}, column {col} on {adapter}",
            );
        }
        let mut mismatches = 0usize;
        let mut absolute_sum = 0.0f64;
        let mut absolute_max = 0.0f32;
        let mut first = None;
        for (index, (word, expected)) in bytes.chunks_exact(4).zip(expected.bytes()).enumerate() {
            let actual = f32::from_ne_bytes(word.try_into().unwrap());
            assert!(
                actual.is_finite() && actual.fract() == 0.0 && (0.0..=255.0).contains(&actual),
                "ONE X2 belt instrument wrote a non-byte value {actual} at {index} on {adapter}",
            );
            let absolute = (actual - f32::from(*expected)).abs();
            absolute_sum += f64::from(absolute);
            absolute_max = absolute_max.max(absolute);
            if absolute != 0.0 {
                mismatches += 1;
                first.get_or_insert((index, actual, *expected));
            }
        }
        let mean_absolute = absolute_sum / expected.bytes().len() as f64;
        drop(bytes);
        readback.unmap();
        println!(
            "ONE X2 belt instrument on {adapter}: {mismatches}/{} scalar differences, mean \
             absolute {mean_absolute:.8}, max absolute {absolute_max}, first {first:?}",
            expected.bytes().len(),
        );

        // The CPU oracle preserves the selected f32 instruction order while
        // ordinary WGSL may contract or reassociate the equivalent bilinear
        // expression. That can move a value across the following U8
        // truncation threshold, but only into the adjacent code.
        // This is a per-output arithmetic bound; the mismatch count is always
        // reported above and is deliberately not accepted by a rate threshold.
        assert!(
            absolute_max <= 1.0,
            "ONE X2 belt instrument exceeded its one-code contraction bound on {adapter}; max \
             {absolute_max}, first mismatch {first:?}",
        );
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            match future.as_mut().poll(&mut context) {
                std::task::Poll::Ready(answer) => return answer,
                std::task::Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn gpu() -> Result<(wgpu::Device, wgpu::Queue, String), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
            .ok_or("no Vulkan adapter")?;
        let name = adapter.get_info().name;
        // This guard exercises only ordinary compute buffers. Unlike the
        // frame-path twins it imports no dmabuf, so asking the driver for the
        // modifier extensions here would make an unrelated extension decide
        // whether the arithmetic can be tested.
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ONE X2 belt twin"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        Ok((device, queue, name))
    }

    fn storage_layout(device: &wgpu::Device, label: &str) -> wgpu::BindGroupLayout {
        let entry = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &[entry(0, true), entry(1, false)],
        })
    }

    fn compute_pipeline(
        device: &wgpu::Device,
        label: &str,
        layout: &wgpu::BindGroupLayout,
        module: &wgpu::ShaderModule,
        entry_point: &str,
    ) -> wgpu::ComputePipeline {
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[layout],
            immediate_size: 0,
        });
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            module,
            entry_point: Some(entry_point),
            compilation_options: Default::default(),
            cache: None,
        })
    }

    fn storage_buffer(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        label: &str,
        words: &[u32],
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: std::mem::size_of_val(words) as u64,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bytes = unsafe {
            std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), std::mem::size_of_val(words))
        };
        queue.write_buffer(&buffer, 0, bytes);
        buffer
    }

    fn readback_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    }

    fn mapped(
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
        submission: wgpu::SubmissionIndex,
    ) -> wgpu::BufferView {
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        slice.get_mapped_range()
    }
}
