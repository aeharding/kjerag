//! Stills of the reframed view: the same pass, into a texture of our own.
//!
//! A capture is not a copy of the window. It is the pipeline and the bind
//! group that draw the window, run a second time into an offscreen texture
//! of the caller's size, so a 3840 px wide still comes off a 1000 px wide
//! window at the sharpness of the source rather than the sharpness of the
//! screen (issue #15). The camera, the frame and the field of view need no
//! plumbing at all: the capture reads the uniform block [`ScenePipeline`]
//! has just written for the redraw it is part of.
//!
//! The target's format is the surface's own, and that is what settles the
//! gamma question. The shader linearizes for an sRGB target and does not
//! for a linear one ([`ScenePipeline`] decides that once, from the format),
//! so a texture in the surface's format holds the numbers the compositor
//! would have been handed, and a PNG of those numbers neither applies a
//! transfer function again nor takes one away. The only thing a surface
//! format changes here is channel order, which [`Order`] puts back the way
//! PNG wants on the CPU.
//!
//! What a still does not hold is the shell: the control overlay and the
//! header bar are iced widgets composited into the surface after this pass,
//! and this pass is all a capture runs. A picture of the view, not a
//! picture of the window.
//!
//! Only the submit happens on the render thread. Waiting for the GPU,
//! mapping, unpadding and whatever the shell then does with the pixels all
//! run on a worker thread, because a capture that stalled the redraw would
//! cost the pilot frames of the flight they are reviewing.
//!
//! [`ScenePipeline`]: super::ScenePipeline

use std::fmt;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kjerag_media::{Fallible, Size};

/// What the shell asks for.
pub struct Request {
    /// Output width in pixels. The height follows the aspect ratio of the
    /// view on screen, so a still frames exactly what the window frames.
    pub width: u32,
    /// What to do with the pixels. It runs on the worker thread that reads
    /// them back, and it must touch neither the GPU nor the shell: encoding
    /// a PNG and writing it is exactly what belongs here.
    pub then: Then,
}

/// The tail of a capture, run once, off the render thread.
pub type Then = Box<dyn FnOnce(Fallible<Shot>) + Send + 'static>;

/// A finished still.
pub struct Shot {
    pub width: u32,
    pub height: u32,
    /// Rows top to bottom, four bytes per pixel, opaque, in the same
    /// encoding the pass writes to the screen.
    pub rgba: Vec<u8>,
    /// The frame that was on screen, counting from the first in the file.
    pub index: u64,
    /// That frame's media time, which is what names the file.
    pub time: Duration,
}

/// Where a capture waits for the next redraw.
///
/// The shell and the render pass cannot reach each other any other way: the
/// pipeline is owned by iced and is touched only through the primitive it
/// prepares, and the shell only ever holds a [`Scene`]. Both hold one of
/// these instead.
///
/// [`Scene`]: super::Scene
#[derive(Clone, Default)]
pub(crate) struct Shutter(Arc<Mutex<Option<Request>>>);

impl Shutter {
    /// Arms a capture for the next redraw. A second one armed before the
    /// first has been taken replaces it: two shutters open on one picture
    /// is one picture.
    pub(crate) fn arm(&self, request: Request) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(request);
        }
    }

    /// Takes the armed request, if there is one. The request stays armed
    /// until a redraw actually takes it, so a capture asked for while the
    /// window is doing something else is late, never lost.
    pub(crate) fn take(&self) -> Option<Request> {
        self.0.lock().ok()?.take()
    }
}

impl fmt::Debug for Shutter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let armed = matches!(self.0.lock().as_deref(), Ok(Some(_)));
        f.debug_tuple("Shutter").field(&armed).finish()
    }
}

/// A capture the GPU has been asked for. Everything left is waiting and
/// copying, which is why this crosses to a worker thread.
pub(crate) struct Pending {
    pub device: wgpu::Device,
    /// Held only so it outlives the copy that reads it.
    pub _texture: wgpu::Texture,
    pub readback: wgpu::Buffer,
    pub submission: wgpu::SubmissionIndex,
    pub size: Size,
    /// Bytes per row in `readback`: `copy_texture_to_buffer` rounds the row
    /// up to [`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`], and at some widths that
    /// is wider than the picture.
    pub stride: u32,
    pub order: Order,
    pub at: Stamp,
}

/// Which frame a still caught.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Stamp {
    pub index: u64,
    pub time: Duration,
}

/// Waits for the pass, reads the pixels back, and hands them to `then`.
/// Spawns a thread even when the capture failed to start, so that a caller
/// has one place its answer arrives and one thread it arrives on.
pub(crate) fn deliver(pending: Fallible<Pending>, then: Then) {
    std::thread::spawn(move || then(pending.and_then(Pending::read)));
}

impl Pending {
    /// Worker thread only: the first line of this blocks until the GPU has
    /// drawn the capture.
    fn read(self) -> Fallible<Shot> {
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(self.submission),
            timeout: None,
        })?;

        // The map is asked for and then driven: `map_async` only queues the
        // request, and it is a poll that runs the callback.
        let (mapped, is_mapped) = mpsc::channel();
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = mapped.send(result);
            });
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        is_mapped.recv()??;

        let view = self.readback.slice(..).get_mapped_range();
        let rgba = tighten(&view, self.size, self.stride, self.order);
        drop(view);
        self.readback.unmap();

        Ok(Shot {
            width: self.size.width,
            height: self.size.height,
            rgba,
            index: self.at.index,
            time: self.at.time,
        })
    }
}

/// Which way round a surface stores its colour channels. Compositors hand
/// out both, and PNG takes only one of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Order {
    Rgba,
    Bgra,
}

impl Order {
    pub fn of(format: wgpu::TextureFormat) -> Fallible<Self> {
        match format {
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => Ok(Self::Rgba),
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => Ok(Self::Bgra),
            other => Err(format!("no capture from a {other:?} surface").into()),
        }
    }
}

/// The output size a capture of `width` gets, at the aspect ratio the view
/// is on screen. Rounding the height costs at most half a pixel of field of
/// view, which is the whole of the difference between a still and the
/// window it was taken from.
pub(crate) fn fitted(width: u32, aspect: f32) -> Fallible<Size> {
    let height = (width as f32 / aspect.max(f32::MIN_POSITIVE)).round();
    if !(1.0..=f32::from(u16::MAX)).contains(&height) || width == 0 {
        return Err(format!("{width} px at aspect {aspect} is not a picture").into());
    }
    Ok(Size::new(width, height as u32))
}

/// Bytes per row of a readback of this width.
pub(crate) fn stride(width: u32) -> u32 {
    let row = width * 4;
    row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

/// The mapped rows without the copy's row padding, in the order PNG wants.
fn tighten(mapped: &[u8], size: Size, stride: u32, order: Order) -> Vec<u8> {
    let row = size.width as usize * 4;
    let mut rgba = Vec::with_capacity(row * size.height as usize);
    for padded in mapped
        .chunks_exact(stride as usize)
        .take(size.height as usize)
    {
        rgba.extend_from_slice(&padded[..row]);
    }
    if order == Order::Bgra {
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
    rgba
}

/// The two halves of the readback that need no GPU: the row arithmetic, and
/// the transfer function the sRGB half of the pass turns on.
#[cfg(test)]
mod tests {
    use super::*;

    /// What `crates/render/src/scene.rs`'s `linearize` does in WGSL, in
    /// Rust, so the round trip below can be checked without a GPU. The
    /// same two-place trick `projection.rs` uses for the forward map.
    fn linearize(c: f32) -> f32 {
        match c > 0.04045 {
            true => ((c + 0.055) / 1.055).powf(2.4),
            false => c / 12.92,
        }
    }

    /// And what an sRGB render target does on store, which is that
    /// function's inverse.
    fn encode(c: f32) -> f32 {
        match c > 0.0031308 {
            true => 1.055 * c.powf(1.0 / 2.4) - 0.055,
            false => c * 12.92,
        }
    }

    /// The window's own round trip, which is what a capture reads back:
    /// the shader linearizes the video's gamma-encoded numbers, the sRGB
    /// target re-encodes them on store, and the bytes in the texture are
    /// the bytes that went in. Eight bits out for eight bits in, every
    /// code, which is the claim "the capture neither doubles the transfer
    /// nor drops it" reduced to arithmetic.
    #[test]
    fn the_srgb_round_trip_is_the_identity() {
        for code in 0..=255u8 {
            let stored = encode(linearize(f32::from(code) / 255.0));
            assert_eq!((stored * 255.0).round() as u8, code, "code {code}");
        }
    }

    /// And the two ways to get it wrong are nowhere near it, so the test
    /// above is worth running: half grey lands 55 codes light with one
    /// transfer left unapplied and 68 dark with one applied twice.
    #[test]
    fn a_missing_or_doubled_transfer_is_not_subtle() {
        let grey = 128.0 / 255.0;
        for wrong in [encode(grey), linearize(grey)] {
            assert!(
                (wrong - grey).abs() > 0.2,
                "{wrong} is within 0.2 of {grey}"
            );
        }
    }

    #[test]
    fn the_readback_drops_the_row_padding() {
        // Three rows of two pixels, in a buffer whose rows are padded out
        // to the copy alignment.
        let size = Size::new(2, 3);
        let stride = stride(size.width);
        assert_eq!(stride, 256);

        let mut mapped = vec![0u8; stride as usize * 3];
        for (row, chunk) in mapped.chunks_exact_mut(stride as usize).enumerate() {
            chunk[..8].copy_from_slice(&[row as u8; 8]);
        }

        let rgba = tighten(&mapped, size, stride, Order::Rgba);
        assert_eq!(rgba, [[0; 8], [1; 8], [2; 8]].concat());
    }

    #[test]
    fn a_bgra_surface_comes_back_rgba() {
        let size = Size::new(1, 1);
        let stride = stride(size.width);
        let mut mapped = vec![0u8; stride as usize];
        mapped[..4].copy_from_slice(&[10, 20, 30, 255]);

        assert_eq!(
            tighten(&mapped, size, stride, Order::Bgra),
            [30, 20, 10, 255]
        );
        assert_eq!(
            tighten(&mapped, size, stride, Order::Rgba),
            [10, 20, 30, 255]
        );
    }

    /// A capture is the window's own view at a bigger size, so its height
    /// comes from the window's aspect ratio and not from a setting.
    #[test]
    fn the_height_follows_the_view() {
        assert_eq!(fitted(3840, 16.0 / 9.0).unwrap(), Size::new(3840, 2160));
        assert_eq!(fitted(1024, 1.0).unwrap(), Size::new(1024, 1024));
        assert!(fitted(3840, 0.0).is_err());
        assert!(fitted(0, 1.0).is_err());
    }

    #[test]
    fn the_stride_is_the_row_rounded_up() {
        assert_eq!(stride(3840), 3840 * 4);
        assert_eq!(stride(100), 512);
        assert_eq!(stride(64), 256);
    }
}
