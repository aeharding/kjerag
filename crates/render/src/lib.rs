//! wgpu: frame import and the shader pass. No demuxing and no decoding; it
//! takes frames from `kyerag-media` and hands the pass to iced (src/widget.rs).

mod camera;
mod capture;
pub mod dmabuf;
mod projection;
mod scene;
mod widget;

pub use camera::{Camera, Viewpoint};
pub use capture::{Request, Shot, Shutter, Then};
pub use kyerag_media::{Cue, Fallible, Size, Stats};
pub use projection::{Landing, MAX_LENSES, OUTSIDE_GRAY, Pick, Reframe};
pub use scene::{Next, Scene, ScenePipeline, ScenePrimitive};

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
