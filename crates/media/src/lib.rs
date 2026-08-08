//! ffmpeg: demux, VA-API decode, and delivery as DRM_PRIME. No shell types,
//! no wgpu.
//!
//! Five layers, smallest first:
//!
//! - [`decode`] is the ffmpeg plumbing: the VA-API device, one decoder per
//!   stream, and the map to DRM_PRIME that [`kjerag_render`] imports.
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
//! - [`walk`] is the other delivery: the same frames in **system memory**,
//!   which is what anything reading the delivered pixels at angles needs
//!   (the seam fit of issue #48, and `kjerag-spike --bin rolling`).
//!
//! [`kjerag_render`]: <https://docs.rs/kjerag-render>

mod audio;
mod decode;
mod player;
mod reader;
mod sound;
mod track;
mod walk;

use std::time::Duration;

use ffmpeg_next as ff;

pub use audio::Audio;
pub use decode::{DrmFrame, HwDevice, MissingDecoder, SwFrame, open_decoder};
pub use player::{Player, Stats};
pub use reader::{Accuracy, Cue, Frames, Read, Reader, Timing};
pub use walk::{Chroma, Pair, Plane, Walk};

const NANOS: u64 = 1_000_000_000;

/// Tell a container which of its streams are wanted, and discard the rest.
///
/// A discarded stream is not read at all: libavformat's MP4 demuxer skips its
/// samples and seeks past them. That is what lets the sound have a demuxer of
/// its own for the price of its own bitrate rather than the file's, and what
/// keeps the pictures' demuxer from crossing the file to fetch sound nobody
/// takes from it any more (issue #97).
pub(crate) fn read_only(input: &mut ff::format::context::Input, wanted: &[usize]) {
    for index in 0..input.nb_streams() as usize {
        let discard = match wanted.contains(&index) {
            true => ff::Discard::Default,
            false => ff::Discard::All,
        };
        if let Some(mut stream) = input.stream_mut(index) {
            // `StreamMut` has no setter for this one field, and reaching for
            // the pointer is what the rest of this crate does when a
            // container's own struct is the only place an answer lives
            // (`Reader::sound_rate`).
            unsafe { (*stream.as_mut_ptr()).discard = discard.into() };
        }
    }
}

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

/// How one decoded frame's samples are written, which the shader needs and
/// the pixels do not say.
///
/// Two facts, both read off the container rather than guessed from each
/// other: an 8-bit stream decodes to NV12 and a 10-bit one to P010, and
/// either can be written full range or studio swing. Insta360 writes 8-bit
/// full range and DJI writes 10-bit studio swing, which is a coincidence of
/// the corpus and not a rule, so neither flag is derived from the other.
///
/// [`Self::default`] is 8-bit full range, which is what every `.insv` in the
/// corpus is and what the pass drew before this existed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Samples {
    /// The two planes hold 16-bit little-endian words rather than bytes:
    /// P010, whose ten bits sit at the top of each word.
    pub wide: bool,
    /// Studio swing: luma runs 64 to 940 of 1023 and chroma is centred on 512
    /// with 448 either side, rather than either using the whole range.
    pub limited: bool,
}

/// A frame size in pixels. NV12 chroma is half of luma in both axes, and
/// getting that halving wrong is a silent half-image, so it has a name.
/// `kjerag-render` turns one of these into a `wgpu::Extent3d`; that half
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
