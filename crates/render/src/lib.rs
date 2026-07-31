//! wgpu: frame import and the shader pass. No demuxing and no decoding; it
//! takes frames from `kyerag-media` and hands the pass to iced (src/widget.rs).

mod camera;
mod capture;
pub mod dmabuf;
mod projection;
/// How a magnified picture is sampled, and where the upgrade engages
/// (issue #11). Public for the instrument that measures it, like
/// [`Reframe`]'s own mirror of the map.
pub mod sampling;
mod scene;
/// The per-camera seam calibration, and the fit behind it (issue #48).
/// Public for `kyerag-spike --bin seam`, which is the same core with the
/// attribution and the controls printed round it.
pub mod seam;
mod widget;

pub use camera::{Camera, Nudge, Viewpoint};
pub use capture::{Request, Shot, Then};
pub use kyerag_media::{Accuracy, Cue, Fallible, Size, Stats};
pub use kyerag_meta::{Quat, Readout, Sweep};
pub use projection::{Blend, Held, Landing, MAX_LENSES, OUTSIDE_GRAY, Reframe, Rolling};
pub use sampling::Sampling;
pub use scene::{FrameClock, Horizon, Next, Scene, ScenePipeline, ScenePrimitive};
pub use seam::{Corrected, SeamFit};

/// A frame [`Size`] as wgpu wants it. This is a trait rather than a method on
/// `Size` because `Size` belongs to `kyerag-media`, which has no wgpu.
pub trait Extent {
    fn extent(self) -> wgpu::Extent3d;
}

impl Extent for Size {
    fn extent(self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        }
    }
}

/// One decoded NV12 frame as wgpu sees it: two single-plane textures, not one
/// two-plane texture. VA-API exports separate layers and wgpu cannot sample a
/// multi-planar format anyway (`wgpu-core/src/validation.rs` panics on it).
pub struct Planes {
    pub luma: wgpu::Texture,
    pub chroma: wgpu::Texture,
}
