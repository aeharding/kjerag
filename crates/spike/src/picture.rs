//! One rendered view, and the questions two of them are asked.
//!
//! The instruments that measure pictures rather than numbers all want the
//! same three things: draw the app's own pass into a target of any size, read
//! it back, and say what separates two of them. `zoom` (issue #11) asked them
//! of a sampling kernel and `ball` (issue #47) asks them of a projection, so
//! they live here rather than in either.

use std::path::{Path, PathBuf};

use kjerag_media::Fallible;
use kjerag_render::{Camera, Sampling, Scene, ScenePipeline, Size};

use super::{Gpu, Offscreen};

/// Not sRGB, so the pass writes the video's own numbers: what keeps a
/// difference between two renders a difference in the pass rather than in a
/// transfer function.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub fn aspect(size: Size) -> f32 {
    size.width as f32 / size.height as f32
}

/// What the pass draws, both ways, into a target of any size.
pub struct Render<'a> {
    pub gpu: &'a Gpu,
    pub scene: &'a Scene,
    pub pipeline: &'a mut ScenePipeline,
}

impl Render<'_> {
    /// One view, one setting, one target's worth of pixels.
    pub fn frame(&mut self, camera: Camera, sampling: Sampling, size: Size) -> Fallible<Picture> {
        self.scene.set_sampling(sampling);
        let primitive = self.scene.primitive(camera);
        self.pipeline
            .prepare(&primitive, &self.gpu.device, &self.gpu.queue, aspect(size));
        let target = Offscreen::new(&self.gpu.device, size, FORMAT);
        target.render(&self.gpu.device, &self.gpu.queue, self.pipeline)?;
        Ok(Picture {
            rgba: target.read(&self.gpu.device, &self.gpu.queue)?,
            size,
        })
    }
}

/// One rendered view, and the questions asked of a pair of them.
pub struct Picture {
    pub rgba: Vec<u8>,
    pub size: Size,
}

impl Picture {
    pub fn write(&self, gpu: &Gpu, name: &str) -> Fallible<PathBuf> {
        let out = PathBuf::from("scratch").join(name);
        let target = Offscreen::new(&gpu.device, self.size, FORMAT);
        target.write_png(&self.rgba, &out)?;
        Ok(out)
    }

    /// The same, into a path of the caller's own choosing.
    pub fn save(&self, gpu: &Gpu, path: &Path) -> Fallible<()> {
        Offscreen::new(&gpu.device, self.size, FORMAT).write_png(&self.rgba, path)
    }

    /// What moved between two pictures, drawn: the difference at 8x about mid
    /// grey, so a change of one code is visible and a change of none is
    /// unambiguously flat.
    ///
    /// The first picture to look at in a proof package, and the one with an
    /// acceptance sentence attached to it: it has to be flat grey everywhere
    /// except where the change was supposed to be.
    pub fn amplified(&self, other: &Self) -> Self {
        Self {
            rgba: self
                .rgba
                .chunks_exact(4)
                .zip(other.rgba.chunks_exact(4))
                .flat_map(|(a, b)| {
                    let lift = |c: usize| {
                        (128 + 8 * (i32::from(a[c]) - i32::from(b[c]))).clamp(0, 255) as u8
                    };
                    [lift(0), lift(1), lift(2), 255]
                })
                .collect(),
            size: self.size,
        }
    }

    /// How much detail the picture holds: the mean absolute Laplacian of its
    /// luma, in codes. A resampling that resolves what bilinear smeared has
    /// to raise this, and one that only rings raises it too, which is why the
    /// pictures are looked at as well as measured.
    pub fn detail(&self) -> f64 {
        let luma = self.luma();
        let (w, h) = (self.size.width as usize, self.size.height as usize);
        let mut total = 0.0;
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let at = |dx: usize, dy: usize| f64::from(luma[(y + dy - 1) * w + x + dx - 1]);
                total += (4.0 * at(1, 1) - at(0, 1) - at(2, 1) - at(1, 0) - at(1, 2)).abs();
            }
        }
        total / ((w - 2) * (h - 2)) as f64
    }

    pub fn luma(&self) -> Vec<f32> {
        self.rgba
            .chunks_exact(4)
            .map(|p| 0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2]))
            .collect()
    }

    /// What separates this picture from another one of the same size.
    pub fn against(&self, other: &Self) -> Difference {
        let mut moved = 0u64;
        let mut total = 0u64;
        let mut worst = 0u8;
        for (a, b) in self.rgba.chunks_exact(4).zip(other.rgba.chunks_exact(4)) {
            let step = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0);
            moved += u64::from(step > 0);
            total += u64::from(step);
            worst = worst.max(step);
        }
        Difference {
            pixels: self.rgba.len() as u64 / 4,
            moved,
            mean: total as f64 / (self.rgba.len() as f64 / 4.0),
            worst,
        }
    }
}

/// Two pictures, compared. `worst` and `mean` are in 8-bit codes of the
/// channel that moved furthest.
pub struct Difference {
    pub pixels: u64,
    pub moved: u64,
    pub mean: f64,
    pub worst: u8,
}

impl Difference {
    pub fn is_identical(&self) -> bool {
        self.moved == 0
    }

    pub fn report(&self) -> String {
        format!(
            "{:.2}% of pixels moved, {:.3} codes mean, {} worst",
            100.0 * self.moved as f64 / self.pixels as f64,
            self.mean,
            self.worst,
        )
    }
}
