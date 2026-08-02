//! Offline, calibrated input for the research residual texture.
//!
//! This deliberately rectifies each raw fisheye into body-sphere coordinates
//! before NCC.  Raw lens pixels are not corresponding coordinates.

use std::io::{self, Write};

use kjerag_media::Plane;
use kjerag_render::Reframe;

use crate::dense_residual::{self, Luma, Map};

pub const MAGIC: [u8; 8] = *b"KJRMAP01";
const PATCH: usize = 4;
const SEARCH: usize = 3;
const SPACING: usize = 2 * (PATCH + SEARCH) + 3;

/// A map is meaningful only for this exact frame and calibration.
///
/// `camera` records the generator viewport for provenance. The map itself is
/// indexed on the capture body sphere, so it remains valid after the viewer
/// pans or changes field of view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Header {
    pub camera_key: u64,
    pub calibration: [f64; 5],
    pub pts_ns: u64,
    pub camera: [f32; 3],
}

/// Rectify, correlate, and convert local rectified movement into lens-1 UV.
pub fn generate(map: &Reframe, planes: &[Plane], width: usize, height: usize) -> Map {
    let mut out = Map::identity(width, height);
    let Some([left, right]) = planes.get(0..2) else {
        return out;
    };
    let rw = width.saturating_mul(SPACING).max(1);
    let rh = height.saturating_mul(SPACING).max(1);
    let (a, valid_a) = rectify(map, left, rw, rh, 0);
    let (b, valid_b) = rectify(map, right, rw, rh, 1);
    for gy in 0..height {
        for gx in 0..width {
            let cx = (gx * SPACING + SPACING / 2) as isize;
            let cy = (gy * SPACING + SPACING / 2) as isize;
            let index = gy * width + gx;
            if !patch_valid(&valid_a, rw, cx, cy) || !patch_valid(&valid_b, rw, cx, cy) {
                continue;
            }
            let Ok(estimate) = dense_residual::estimate(
                Luma {
                    width: rw,
                    height: rh,
                    samples: &a,
                },
                Luma {
                    width: rw,
                    height: rh,
                    samples: &b,
                },
                [cx, cy],
                PATCH,
                SEARCH,
            ) else {
                continue;
            };
            let Some(delta) = source_delta(
                map,
                right.size.width,
                right.size.height,
                gx,
                gy,
                width,
                height,
                estimate.shift_px,
            ) else {
                continue;
            };
            out.texels[index] = [delta[0], delta[1], estimate.confidence, 0.0];
        }
    }
    out
}

fn patch_valid(valid: &[bool], width: usize, cx: isize, cy: isize) -> bool {
    let r = (PATCH + SEARCH) as isize;
    (-r..=r).all(|y| (-r..=r).all(|x| valid[(cy + y) as usize * width + (cx + x) as usize]))
}

fn rectify(
    map: &Reframe,
    plane: &Plane,
    width: usize,
    height: usize,
    lens: usize,
) -> (Vec<f32>, Vec<bool>) {
    let mut samples = vec![0.0; width * height];
    let mut valid = vec![false; width * height];
    for y in 0..height {
        for x in 0..width {
            let body = body_ray(
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            );
            let landing = map.project(lens, map.view_ray_from_body(body));
            if landing.inside {
                if let Some(value) = plane.at(landing.pixel[0] as f64, landing.pixel[1] as f64) {
                    samples[y * width + x] = (value / 255.0) as f32;
                    valid[y * width + x] = true;
                }
            }
        }
    }
    (samples, valid)
}

fn source_delta(
    map: &Reframe,
    frame_width: u32,
    frame_height: u32,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    shift: [i32; 2],
) -> Option<[f32; 2]> {
    let u = (x as f32 + 0.5) / width as f32;
    let v = (y as f32 + 0.5) / height as f32;
    let centre = map.project(1, map.view_ray_from_body(body_ray(u, v)));
    let dx = map.project(
        1,
        map.view_ray_from_body(body_ray(u + 1.0 / (width * SPACING) as f32, v)),
    );
    let dy = map.project(
        1,
        map.view_ray_from_body(body_ray(u, v + 1.0 / (height * SPACING) as f32)),
    );
    if !(centre.inside && dx.inside && dy.inside) {
        return None;
    }
    let jx = [
        (dx.pixel[0] - centre.pixel[0]) / SPACING as f32,
        (dx.pixel[1] - centre.pixel[1]) / SPACING as f32,
    ];
    let jy = [
        (dy.pixel[0] - centre.pixel[0]) / SPACING as f32,
        (dy.pixel[1] - centre.pixel[1]) / SPACING as f32,
    ];
    let du = (shift[0] as f32 * jx[0] + shift[1] as f32 * jy[0]) / frame_width as f32;
    let dv = (shift[0] as f32 * jx[1] + shift[1] as f32 * jy[1]) / frame_height as f32;
    (du.is_finite() && dv.is_finite()).then_some([du, dv])
}

fn body_ray(u: f32, v: f32) -> [f32; 3] {
    let azimuth = (u - 0.5) * std::f32::consts::TAU;
    let elevation = (0.5 - v) * std::f32::consts::PI;
    [
        elevation.cos() * azimuth.sin(),
        elevation.sin(),
        elevation.cos() * azimuth.cos(),
    ]
}

/// Stable little-endian sidecar encoding; texture data is RGBA f32.
pub fn write(mut to: impl Write, header: Header, map: &Map) -> io::Result<()> {
    to.write_all(&MAGIC)?;
    to.write_all(&header.camera_key.to_le_bytes())?;
    for value in header.calibration {
        to.write_all(&value.to_le_bytes())?;
    }
    to.write_all(&header.pts_ns.to_le_bytes())?;
    for value in header.camera {
        to.write_all(&value.to_le_bytes())?;
    }
    to.write_all(&(map.width as u32).to_le_bytes())?;
    to.write_all(&(map.height as u32).to_le_bytes())?;
    for texel in &map.texels {
        for value in texel {
            to.write_all(&value.to_le_bytes())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Header, MAGIC, write};
    use crate::dense_residual::Map;
    #[test]
    fn format_is_little_endian_and_fixed() {
        let mut bytes = Vec::new();
        write(
            &mut bytes,
            Header {
                camera_key: 7,
                calibration: [0.0; 5],
                pts_ns: 9,
                camera: [0.0; 3],
            },
            &Map::identity(1, 1),
        )
        .unwrap();
        assert_eq!(&bytes[..8], &MAGIC);
        assert_eq!(&bytes[8..16], &7u64.to_le_bytes());
        assert_eq!(bytes.len(), 8 + 8 + 40 + 8 + 12 + 8 + 16);
    }
}
