//! ffmpeg: demux, VA-API decode, and delivery as DRM_PRIME. No shell types,
//! no wgpu.
//!
//! Five layers, smallest first:
//!
//! - [`decode`] is the ffmpeg plumbing: the VA-API device, one decoder per
//!   stream, and the map to DRM_PRIME that [`kyerag_render`] imports.
//! - [`audio`] is the ring of samples between the decode thread and the sound
//!   card, and the arithmetic that keeps it on the picture's clock. No ffmpeg
//!   and no device in it, so `cargo test` covers all of it.
//! - [`track`] is the file's own sound: one AAC stream decoded and resampled
//!   into that ring, off the same demuxer as the pictures. [`sound`] is the
//!   device it goes out of.
//! - [`reader`] is one demuxer driving every video stream of a file in
//!   lockstep and handing out [`Frames`]: the same PTS from both lenses,
//!   always as a pair. It reads forward and it reads by [`Cue`].
//! - [`player`] is the presentation clock around a [`Reader`] on its own
//!   thread: play, pause, and "which frame is due now". The sound follows that
//!   clock; it never sets it.
//!
//! [`kyerag_render`]: <https://docs.rs/kyerag-render>

mod audio;
mod decode;
mod player;
mod reader;
mod sound;
mod track;

use std::time::Duration;

use ffmpeg_next as ff;

pub use audio::Audio;
pub use decode::{DrmFrame, HwDevice, SwFrame, open_decoder};
pub use player::{Player, Stats};
pub use reader::{Accuracy, Cue, Frames, Read, Reader, Timing};

const NANOS: u64 = 1_000_000_000;

/// A stream's timestamp as media time from the start of the file.
///
/// `start` is where the container chose to begin, which it is free to put
/// anywhere; every clock above this crate measures from it.
pub(crate) fn media_time(pts: i64, start: i64, time_base: ff::Rational) -> Duration {
    let ticks = pts.saturating_sub(start).max(0) as u128;
    let nanos =
        ticks * time_base.numerator() as u128 * u128::from(NANOS) / time_base.denominator() as u128;
    Duration::from_nanos(nanos as u64)
}

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
