//! The ffmpeg plumbing: a VA-API device, decoders that refuse to fall back
//! to software, and the two ways a decoded frame leaves the GPU.

use std::ffi::{CStr, c_int};

use ff::ffi::{
    AV_HWFRAME_MAP_DIRECT, AV_HWFRAME_MAP_READ, AVBufferRef, AVCodecContext, AVDRMFrameDescriptor,
    AVFrame, AVHWDeviceType, AVPixelFormat, av_buffer_ref, av_buffer_unref, av_frame_alloc,
    av_frame_free, av_hwdevice_ctx_create, av_hwframe_map, av_hwframe_transfer_data,
};
use ffmpeg_next as ff;

use super::Fallible;

const VAAPI_DEVICE: &CStr = c"/dev/dri/renderD128";

/// An `AVBufferRef` holding a VA-API device context.
pub struct HwDevice(*mut AVBufferRef);

// `AVBufferRef` is ffmpeg's own reference count and a device context is
// shared by every decoder opened against it. The engine moves one to the
// decode thread and nothing else touches it from anywhere else.
unsafe impl Send for HwDevice {}

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

/// Surfaces this decoder's frame pool holds, straight from the
/// `AVHWFramesContext` ffmpeg built for it. A VA-API pool is fixed at
/// `avcodec_open2` time, so this is the hard ceiling on how many decoded
/// frames the engine may hold at once; the playback instrument prints it.
pub fn pool_size(decoder: &ff::decoder::Video) -> Option<i32> {
    unsafe {
        let frames = (*decoder.as_ptr()).hw_frames_ctx;
        if frames.is_null() {
            return None;
        }
        let ctx = (*frames).data.cast::<ff::ffi::AVHWFramesContext>();
        Some((*ctx).initial_pool_size)
    }
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
    /// The engine maps a surface only after later frames have been submitted
    /// to the decoder, which is what makes the wait cheap; see
    /// [`Reader::lookahead`](super::Reader::lookahead).
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
