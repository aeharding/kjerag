//! wgpu: frame import and the shader pass. No demuxing and no decoding; it
//! takes frames from `kjerag-media` and hands the pass to iced (src/widget.rs).

/// The per-frame seam band: what the two lenses still disagree about after
/// the calibration, measured on the GPU and bent out (issue #103). Public for
/// `kjerag-spike --bin band`, which reads the state back and reports it.
pub mod band;
mod camera;
mod capture;
/// Studio's chromatic correction field: the regularized solve that turns a
/// thin band of per-channel evidence into a smooth correction over the seam
/// (docs/research/studio-seam-re.md, docs/research/chromatic.md section 2).
/// Public because the constants in it are the reverse engineering's, and the
/// instruments that judge the field read them rather than restating them.
pub mod chroma;
pub mod chromatic;
mod direct_type2;
pub mod dmabuf;
pub mod flow;
mod framing;
pub mod map_oracle;
mod one_xs_luma;
mod projection;
/// How a magnified picture is sampled, and where the upgrade engages
/// (issue #11). Public for the instrument that measures it, like
/// [`Reframe`]'s own mirror of the map.
pub mod sampling;
mod scene;
/// The per-camera seam calibration, and the fit behind it (issue #48).
/// Public for `kjerag-spike --bin seam`, which is the same core with the
/// attribution and the controls printed round it.
pub mod seam;
mod stall;
pub mod studio_type2;
/// The shader and its Rust mirror, asked the same question on a real GPU
/// (`src/twin.rs`). Tests only: it exists because a change to one half and not
/// the other used to pass the whole suite.
#[cfg(test)]
mod twin;
mod widget;

pub use band::{
    AZIMUTHS, Along, Cell, KEEP, Leftover, PERP_DEG, Reading, Ring, Table, Tone, depth_leak, ease,
    time_constant,
};
pub use camera::{Camera, Nudge, Viewpoint};
pub use capture::{Request, Shot, Then};
pub use framing::Framing;
pub use kjerag_media::{Accuracy, Cue, Fallible, FrameStamp, MissingDecoder, Size, Stats};
/// Which files one capture is made of (issue #123), under a name that does
/// not collide with this crate's own `capture`, which is the screenshot one.
pub use kjerag_meta::capture as capture_set;
pub use kjerag_meta::{Foreign, Quat, Readout, Sweep};
pub use one_xs_luma::{OneXsLumaFrame, PendingOneXsLuma};
pub use projection::{
    Blend, Held, Landing, MAX_LENSES, Reframe, Rolling, SeamAnchor, seam_follows_a_lens,
};
pub use sampling::Sampling;
pub use scene::{FrameClock, Horizon, Next, Scene, ScenePipeline, ScenePrimitive};
pub use seam::{Correction, SeamFit};
pub use stall::{STUCK_FOR, Stall};
pub use studio_type2::{MapBindError, OneXsMapFrame, OneXsMapRaster, PisBackend, PreparedPicture};

/// A frame [`Size`] as wgpu wants it. This is a trait rather than a method on
/// `Size` because `Size` belongs to `kjerag-media`, which has no wgpu.
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
