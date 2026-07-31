//! ffmpeg: demux, VA-API decode, and delivery as DRM_PRIME. No shell types,
//! no wgpu.

use std::ffi::{CStr, c_int};
use std::path::Path;

use ff::ffi::{
    AV_HWFRAME_MAP_DIRECT, AV_HWFRAME_MAP_READ, AVBufferRef, AVCodecContext, AVDRMFrameDescriptor,
    AVFrame, AVHWDeviceType, AVPixelFormat, av_buffer_ref, av_buffer_unref, av_frame_alloc,
    av_frame_free, av_hwdevice_ctx_create, av_hwframe_map, av_hwframe_transfer_data,
};
use ffmpeg_next as ff;

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

const VAAPI_DEVICE: &CStr = c"/dev/dri/renderD128";

/// An `AVBufferRef` holding a VA-API device context.
pub struct HwDevice(*mut AVBufferRef);

impl HwDevice {
    pub fn vaapi() -> Fallible<Self> {
        let mut raw = std::ptr::null_mut();
        let rc = unsafe {
            av_hwdevice_ctx_create(
                &mut raw,
                AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                VAAPI_DEVICE.as_ptr(),
                std::ptr::null_mut(),
                0,
            )
        };
        if rc < 0 {
            return Err(format!("av_hwdevice_ctx_create: {}", ff::Error::from(rc)).into());
        }
        Ok(Self(raw))
    }
}

impl Drop for HwDevice {
    fn drop(&mut self) {
        unsafe { av_buffer_unref(&mut self.0) };
    }
}

/// Force the hardware pixel format. The default would fall back to software
/// decoding without complaint, which would make every timing a lie.
unsafe extern "C" fn pick_vaapi(
    _ctx: *mut AVCodecContext,
    mut formats: *const AVPixelFormat,
) -> AVPixelFormat {
    unsafe {
        while *formats != AVPixelFormat::AV_PIX_FMT_NONE {
            if *formats == AVPixelFormat::AV_PIX_FMT_VAAPI {
                return AVPixelFormat::AV_PIX_FMT_VAAPI;
            }
            formats = formats.add(1);
        }
        AVPixelFormat::AV_PIX_FMT_NONE
    }
}

pub fn open_decoder(
    ictx: &ff::format::context::Input,
    stream: usize,
    hw: &HwDevice,
) -> Fallible<ff::decoder::Video> {
    let params = ictx.stream(stream).ok_or("no such stream")?.parameters();
    let mut ctx = ff::codec::context::Context::from_parameters(params)?;
    unsafe {
        let raw = ctx.as_mut_ptr();
        (*raw).hw_device_ctx = av_buffer_ref(hw.0);
        (*raw).get_format = Some(pick_vaapi);
    }
    Ok(ctx.decoder().video()?)
}

/// A frame mapped to DRM_PRIME. Dropping it unmaps and closes the exported
/// fds, and returns the surface to the decoder's pool, so it must outlive
/// every texture imported from it.
pub struct DrmFrame(*mut AVFrame);

// An `AVFrame` is a reference-counted handle ffmpeg itself passes between
// threads; only its owner touches it, and we only ever read the descriptor.
unsafe impl Send for DrmFrame {}
unsafe impl Sync for DrmFrame {}

impl DrmFrame {
    /// `MAP_READ` makes ffmpeg call `vaSyncSurface` first (ffmpeg 6.1
    /// `hwcontext_vaapi.c:1337`), so this waits for the decode to finish.
    pub fn map(src: &ff::frame::Video) -> Fallible<Self> {
        let raw = unsafe { av_frame_alloc() };
        if raw.is_null() {
            return Err("av_frame_alloc".into());
        }
        let frame = Self(raw);
        let rc = unsafe {
            (*raw).format = AVPixelFormat::AV_PIX_FMT_DRM_PRIME as c_int;
            av_hwframe_map(
                raw,
                src.as_ptr(),
                AV_HWFRAME_MAP_READ as c_int | AV_HWFRAME_MAP_DIRECT as c_int,
            )
        };
        if rc < 0 {
            return Err(format!("av_hwframe_map: {}", ff::Error::from(rc)).into());
        }
        Ok(frame)
    }

    pub fn descriptor(&self) -> &AVDRMFrameDescriptor {
        unsafe { &*((*self.0).data[0].cast::<AVDRMFrameDescriptor>()) }
    }

    /// What the driver actually exported, so pitches in a report are quoted
    /// rather than computed.
    pub fn describe(&self) -> String {
        let desc = self.descriptor();
        let layers: Vec<String> = (0..desc.nb_layers as usize)
            .map(|i| {
                let layer = &desc.layers[i];
                let plane = &layer.planes[0];
                format!(
                    "layer {i}: fourcc {:#x}, object {}, pitch {}, offset {}",
                    layer.format, plane.object_index, plane.pitch, plane.offset
                )
            })
            .collect();
        format!(
            "{} object(s), modifier {:#x}\n        {}",
            desc.nb_objects,
            desc.objects[0].format_modifier,
            layers.join("\n        ")
        )
    }
}

impl Drop for DrmFrame {
    fn drop(&mut self) {
        unsafe { av_frame_free(&mut self.0) };
    }
}

/// A frame the driver copied into system memory (the measured-against path).
pub struct SwFrame(*mut AVFrame);

impl SwFrame {
    pub fn transfer(src: &ff::frame::Video) -> Fallible<Self> {
        let raw = unsafe { av_frame_alloc() };
        if raw.is_null() {
            return Err("av_frame_alloc".into());
        }
        let frame = Self(raw);
        let rc = unsafe { av_hwframe_transfer_data(raw, src.as_ptr(), 0) };
        if rc < 0 {
            return Err(format!("av_hwframe_transfer_data: {}", ff::Error::from(rc)).into());
        }
        Ok(frame)
    }

    /// Plane bytes and their stride, both straight from the frame.
    pub fn plane(&self, index: usize, rows: u32) -> (&[u8], u32) {
        unsafe {
            let stride = (*self.0).linesize[index] as u32;
            let len = stride as usize * rows as usize;
            (
                std::slice::from_raw_parts((*self.0).data[index], len),
                stride,
            )
        }
    }
}

impl Drop for SwFrame {
    fn drop(&mut self) {
        unsafe { av_frame_free(&mut self.0) };
    }
}

/// The first decoded frame of a stream, mapped to DRM_PRIME. The decoder and
/// its hardware context are dropped on the way out, which is safe because the
/// map holds its own reference to the surface.
pub fn first_frame(path: &Path, stream: usize) -> Fallible<(DrmFrame, Size)> {
    ff::init()?;
    let hw = HwDevice::vaapi()?;
    let mut ictx = ff::format::input(&path)?;
    let mut decoder = open_decoder(&ictx, stream, &hw)?;
    let size = Size::new(decoder.width(), decoder.height());
    let mut frame = ff::frame::Video::empty();

    for (from, packet) in ictx.packets() {
        if from.index() != stream {
            continue;
        }
        decoder.send_packet(&packet)?;
        if decoder.receive_frame(&mut frame).is_ok() {
            return Ok((DrmFrame::map(&frame)?, size));
        }
    }
    Err("stream ended before the first frame decoded".into())
}
