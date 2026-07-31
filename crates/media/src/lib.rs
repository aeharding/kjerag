//! ffmpeg: demux, VA-API decode, and delivery as DRM_PRIME. No shell types,
//! no wgpu.
//!
//! Three layers, smallest first:
//!
//! - [`decode`] is the ffmpeg plumbing: the VA-API device, one decoder per
//!   stream, and the map to DRM_PRIME that [`kyerag_render`] imports.
//! - [`reader`] is one demuxer driving every video stream of a file in
//!   lockstep and handing out [`Frames`]: the same PTS from both lenses,
//!   always as a pair. It reads forward and it reads by [`Cue`].
//! - [`player`] is the presentation clock around a [`Reader`] on its own
//!   thread: play, pause, and "which frame is due now".
//!
//! [`kyerag_render`]: <https://docs.rs/kyerag-render>

mod decode;
mod player;
mod reader;

pub use decode::{DrmFrame, HwDevice, SwFrame, open_decoder};
pub use player::{Player, Stats};
pub use reader::{Accuracy, Cue, Frames, Read, Reader, Timing};

/// Errors cross thread boundaries here because iced's shader primitives are
/// `Send + Sync`, so the plain `Box<dyn Error>` a binary would use will not do.
pub type Fallible<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// A frame size in pixels. NV12 chroma is half of luma in both axes, and
/// getting that halving wrong is a silent half-image, so it has a name.
/// `kyerag-render` turns one of these into a `wgpu::Extent3d`; that half
/// cannot live here, because this crate has no wgpu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn halved(self) -> Self {
        Self::new(self.width / 2, self.height / 2)
    }
}
