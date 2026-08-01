//! One rendered view, with the depth warp switched on or off, and the two
//! measures a stitch is scored by.
//!
//! The warp itself is four lines of [`paint`] and they are the whole proposal:
//! each lens's ray is bent along the epipolar axis by the **other** lens's
//! blend weight times the measured disparity. The two bends differ by exactly
//! the disparity wherever the weights sum to one, so the two lenses agree
//! everywhere in the band; and each lens's own bend is zero wherever its
//! weight is one, so nothing moves outside the band and there is no edge to
//! feather. Neither property is arranged: both fall out of the weights the
//! pass already computes.
//!
//! Luma only. Colour would need the chroma plane and a colour transform, and a
//! double image is geometry, which is in the luma.

use std::path::Path;

use kyerag_media::Fallible;
use kyerag_render::Reframe;
use kyerag_spike::{Pair, Plane};

use crate::band::{node, unit};
use crate::strategy::Warp;

/// What an unknown stitcher's output might be a picture in, and what our own
/// pass is rendered into to compare with it.
///
/// One family with one number in it: `theta = atan(rho tan(c phi)) / c` is
/// rectilinear at `c = 1` and equidistant in the limit at `c = 0`. The
/// `mode=parity` fit in `crates/spike/src/bin/seam.rs` is what produces the
/// numbers for an Insta360 export; this instrument takes them as an argument
/// rather than fitting them a second time.
#[derive(Clone, Copy)]
pub struct Look {
    pub yaw: f64,
    pub pitch: f64,
    pub roll: f64,
    pub fov: f64,
    pub compression: f64,
}

impl Look {
    /// The ray one point of the output looks along, in the body's own frame.
    pub fn ray(&self, uv: [f64; 2]) -> [f64; 3] {
        let (u, v) = (uv[0] * 2.0 - 1.0, uv[1] * 2.0 - 1.0);
        let rho = u.hypot(v);
        let c = self.compression.max(1e-3);
        let half = (self.fov.to_radians() / 2.0).max(1e-3);
        let theta = (rho * (c * half).tan()).atan() / c;
        let (sin, cos) = theta.sin_cos();
        let view = match rho > 0.0 {
            true => [sin * u / rho, sin * v / rho, cos],
            false => [0.0, 0.0, 1.0],
        };
        let turn = kyerag_meta::Mat3::rot_y(self.yaw.to_radians())
            .times(kyerag_meta::Mat3::rot_x(self.pitch.to_radians()))
            .times(kyerag_meta::Mat3::rot_z(self.roll.to_radians()));
        unit(turn.mul_vec(view))
    }
}

/// One rendered picture and what each pixel is.
pub struct Painted {
    pub size: u32,
    pub luma: Vec<f64>,
    /// How far past the seam plane each pixel looks, degrees.
    pub psi: Vec<f64>,
    /// Whether the picture is defined here, which every measure is taken
    /// inside. The edge of a lens's picture is a step of a hundred codes and
    /// would treble a gradient mean that crossed it.
    ///
    /// It is "one lens has the ray" and not "both do", because both is empty
    /// where this statistic needs pixels: `Landing::inside` carries
    /// `depth > 0`, so it goes off at the image circle about 7 degrees past
    /// the seam, and the surrounding term of [`Self::parity`] is taken 9 to 25
    /// degrees out. Measured with the stricter mask, that term is 0 pixels of
    /// 1048576 and the ratio reads 0.000, which looks like a picture with no
    /// sharpness rather than a mask with no pixels. Every number here is
    /// therefore on this mask, ours and theirs alike, and the counts are
    /// printed beside the ratio.
    pub defined: Vec<bool>,
    /// The disparity the warp applied there, degrees.
    pub applied: Vec<f64>,
}

/// Render one view of one decoded frame.
///
/// `warp` is the depth field; `None` is the shipped path, which bends nothing.
pub fn paint(
    reframe: &Reframe,
    pair: &Pair,
    look: Look,
    size: u32,
    baseline: [f64; 3],
    warp: Option<&Warp>,
) -> Painted {
    let mut painted = Painted {
        size,
        luma: Vec::with_capacity((size * size) as usize),
        psi: Vec::with_capacity((size * size) as usize),
        defined: Vec::with_capacity((size * size) as usize),
        applied: Vec::with_capacity((size * size) as usize),
    };
    for index in 0..size * size {
        let uv = [
            (f64::from(index % size) + 0.5) / f64::from(size),
            (f64::from(index / size) + 0.5) / f64::from(size),
        ];
        let ray = look.ray(uv);
        let psi = ray[2].clamp(-1.0, 1.0).asin();
        let phi = ray[1].atan2(ray[0]);
        let blend = reframe.blend(ray.map(|c| c as f32));
        let weights = blend.weights.map(f64::from);
        let disparity = warp.map_or(0.0, |warp| warp.at(phi, psi.to_degrees()));
        // The epipolar axis at this ray, from the file's own baseline. The
        // node constructor is the one place that geometry is written down.
        let epi = node(baseline, phi, psi).epi;
        let mut luma = 0.0;
        let mut total = 0.0;
        let mut inside = [false; 2];
        for lens in 0..2 {
            // The other lens's weight is this lens's share of the bend, so the
            // two of them are always exactly one disparity apart and each is
            // still at zero where it owns the picture alone.
            let share = weights[1 - lens] * disparity * if lens == 0 { -1.0 } else { 1.0 };
            let bent = unit(std::array::from_fn(|axis| ray[axis] + share * epi[axis]));
            let landing = reframe.project(lens, bent.map(|c| c as f32));
            // Asked of both lenses whatever the weights are, because this is
            // the mask the sharpness statistic is taken inside and that mask
            // has to reach past the band: gating it on a nonzero weight makes
            // the "either side" term empty, and a ratio with an empty
            // denominator reads 0.000 and looks like a measurement.
            inside[lens] = landing.inside;
            if weights[lens] <= 0.0 {
                continue;
            }
            let Some(code) =
                pair.lenses[lens].at(f64::from(landing.pixel[0]), f64::from(landing.pixel[1]))
            else {
                continue;
            };
            luma += weights[lens] * code;
            total += weights[lens];
        }
        painted.luma.push(match total > 0.0 {
            true => luma / total,
            false => 0.0,
        });
        painted.psi.push(psi.to_degrees());
        painted.defined.push(inside[0] || inside[1]);
        painted.applied.push(disparity.to_degrees());
    }
    painted
}

/// One frame of a stitched export, in the same shape a [`Painted`] is scored
/// in, so the two go through the same statistic.
pub fn imported(plane: &Plane, size: u32, look: Look, up_to: u32) -> Painted {
    let step = f64::from(size) / f64::from(up_to);
    let mut painted = Painted {
        size: up_to,
        luma: Vec::with_capacity((up_to * up_to) as usize),
        psi: Vec::with_capacity((up_to * up_to) as usize),
        defined: Vec::with_capacity((up_to * up_to) as usize),
        applied: vec![0.0; (up_to * up_to) as usize],
    };
    for index in 0..up_to * up_to {
        let (x, y) = (index % up_to, index / up_to);
        let uv = [
            (f64::from(x) + 0.5) / f64::from(up_to),
            (f64::from(y) + 0.5) / f64::from(up_to),
        ];
        let ray = look.ray(uv);
        let (sx, sy) = (
            ((f64::from(x) + 0.5) * step) as usize,
            ((f64::from(y) + 0.5) * step) as usize,
        );
        painted
            .luma
            .push(f64::from(plane.luma[sy * plane.stride + sx]));
        painted
            .psi
            .push(ray[2].clamp(-1.0, 1.0).asin().to_degrees());
        painted.defined.push(true);
    }
    painted
}

impl Painted {
    /// Mean squared gradient across the picture, over the pixels whose
    /// distance past the seam plane falls in `band`.
    ///
    /// A doubled edge is a blurred edge and blur is what a gradient measures.
    /// Taken across the picture because the seam runs down a seam-centred view
    /// and the doubling is across it, which is the axis the disparity runs
    /// along.
    pub fn sharpness(&self, band: (f64, f64)) -> f64 {
        let size = self.size as usize;
        let (mut total, mut count) = (0.0, 0.0);
        for y in 0..size {
            for x in 1..size - 1 {
                let index = y * size + x;
                let past = self.psi[index].abs();
                if past < band.0 || past > band.1 {
                    continue;
                }
                if [index - 1, index, index + 1]
                    .iter()
                    .any(|at| !self.defined[*at] || self.luma[*at] <= 0.0)
                {
                    continue;
                }
                let step = self.luma[index + 1] - self.luma[index - 1];
                total += step * step;
                count += 1.0;
            }
        }
        match count > 0.0 {
            true => total / count,
            false => 0.0,
        }
    }

    /// The one number the parity ladder is read by: the band's own sharpness
    /// over the same picture's sharpness away from the band. Each stitch is
    /// its own control, so a tone curve and a sharpening pass divide out.
    ///
    /// The same statistic as docs/research/insv-format.md 6.8, so the numbers
    /// here sit on the same scale as the ones already recorded there.
    pub fn parity(&self) -> f64 {
        let outside = self.sharpness((9.0, 25.0));
        match outside > 0.0 {
            true => self.sharpness((0.0, 5.0)) / outside,
            false => 0.0,
        }
    }

    /// How many pixels each term of [`Self::parity`] was taken over. Two runs
    /// are only comparable on the same picture, and a term over no pixels at
    /// all is what an empty mask looks like from the outside.
    pub fn counted(&self, band: (f64, f64)) -> usize {
        self.psi
            .iter()
            .zip(&self.defined)
            .filter(|(psi, defined)| **defined && psi.abs() >= band.0 && psi.abs() <= band.1)
            .count()
    }

    pub fn write(&self, path: &Path) -> Fallible<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let pixels: Vec<u8> = self
            .luma
            .iter()
            .map(|code| code.clamp(0.0, 255.0) as u8)
            .collect();
        write_gray(path, self.size, &pixels)
    }

    /// The disparity the warp applied, as a picture: mid grey is no
    /// correction, brighter is nearer content. What the owner is shown beside
    /// the frame it corrected.
    pub fn write_disparity(&self, path: &Path, full_scale_deg: f64) -> Fallible<()> {
        let pixels: Vec<u8> = self
            .applied
            .iter()
            .map(|deg| (128.0 + 127.0 * deg / full_scale_deg).clamp(0.0, 255.0) as u8)
            .collect();
        write_gray(path, self.size, &pixels)
    }

    /// What moved between two renders of the same frame, at 8x, centred on mid
    /// grey. Where this is flat the warp changed nothing.
    pub fn write_difference(&self, other: &Self, path: &Path) -> Fallible<()> {
        let pixels: Vec<u8> = self
            .luma
            .iter()
            .zip(&other.luma)
            .map(|(a, b)| (128.0 + 8.0 * (a - b)).clamp(0.0, 255.0) as u8)
            .collect();
        write_gray(path, self.size, &pixels)
    }
}

fn write_gray(path: &Path, size: u32, pixels: &[u8]) -> Fallible<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut png = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(path)?),
        size,
        size,
    );
    png.set_color(png::ColorType::Grayscale);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()?.write_image_data(pixels)?;
    Ok(())
}

/// How much of a view is in the overlap band at all, which is what the render
/// path's added cost is charged against.
pub fn band_share(painted: &Painted, width_deg: f64) -> f64 {
    let inside = painted
        .psi
        .iter()
        .filter(|psi| psi.abs() <= width_deg / 2.0)
        .count();
    inside as f64 / painted.psi.len() as f64
}
