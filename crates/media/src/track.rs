//! The file's sound: one AAC stream, off the same demuxer as the pictures.
//!
//! There is no second reader and no second thread. An `.insv` writes the sound
//! interleaved with the two lens streams in one MP4, so its packets arrive at
//! the demuxer that is already running and this is the lane that takes them
//! ([`Reader::pump`](super::Reader)). Decoding it anywhere else would mean a
//! second file handle seeking against the first.
//!
//! What leaves the decoder is planar `fltp` at the file's own rate; what the
//! device wants is interleaved at the device's rate and channel count. So
//! `swresample` sits between them, and it is there for a second reason as
//! well: `swr_set_compensation` is the drift correction ([`super::audio`]),
//! and it exists precisely to slave a sound to a clock that is not the sound
//! card's.

use std::ffi::c_int;
use std::time::Duration;

use ffmpeg_next as ff;

use super::audio::{Pipe, compensation};
use super::{Fallible, media_time};

/// Output frames the drift correction is spread over: one second. Long enough
/// that the ratio is a rounding error, short enough that it is re-aimed before
/// the file has moved far.
const DISTANCE: u32 = 1;

type Resampler = ff::software::resampling::Context;

/// One audio stream, decoded and resampled into the device's own format.
pub struct Track {
    pub stream: usize,
    decoder: ff::decoder::Audio,
    /// Built from the first decoded frame rather than at open: a decoder does
    /// not know its own sample format until it has decoded something, and
    /// asking earlier gets `AV_SAMPLE_FMT_NONE`.
    resampler: Option<Resampler>,
    format: ff::format::Sample,
    layout: ff::ChannelLayout,
    rate: u32,
    channels: usize,
    /// Stream time base, and the PTS the container starts from.
    time_base: ff::Rational,
    start: i64,
    pipe: Pipe,
    /// Interleaved scratch the planar output is woven into, kept between
    /// chunks so a packet costs no allocation.
    woven: Vec<f32>,
}

impl Track {
    /// Opens the first audio stream of `input`, if it has one, to be resampled
    /// into `rate` and `channels`.
    ///
    /// `Ok(None)` is a file with no sound in it, which the older cameras'
    /// per-lens files are. Those play their pictures exactly as before, and
    /// silently rather than by refusing to open.
    pub fn open(
        input: &ff::format::context::Input,
        pipe: Pipe,
        rate: u32,
        channels: usize,
    ) -> Fallible<Option<Self>> {
        let Some(stream) = input
            .streams()
            .find(|s| s.parameters().medium() == ff::media::Type::Audio)
        else {
            return Ok(None);
        };
        let (index, time_base, start) = (stream.index(), stream.time_base(), stream.start_time());
        let context = ff::codec::context::Context::from_parameters(stream.parameters())?;

        Ok(Some(Self {
            stream: index,
            decoder: context.decoder().audio()?,
            resampler: None,
            format: ff::format::Sample::F32(ff::format::sample::Type::Planar),
            layout: ff::ChannelLayout::default(channels as i32),
            rate,
            channels,
            time_base,
            start: match start == ff::ffi::AV_NOPTS_VALUE {
                true => 0,
                false => start,
            },
            pipe,
            woven: Vec::new(),
        }))
    }

    /// One packet in, and everything it completes out to the device.
    pub fn take(&mut self, packet: &ff::Packet) -> Fallible<()> {
        self.decoder.send_packet(packet)?;
        self.drain()
    }

    /// The end of the file: whatever the decoder is still holding, which is
    /// the last few tens of milliseconds of the track.
    pub fn end(&mut self) -> Fallible<()> {
        self.decoder.send_eof()?;
        self.drain()
    }

    /// Throw away what is decoded but not yet heard. Paired with the video
    /// decoders' flush in [`Reader::seek`](super::Reader::seek), so the sound
    /// and the pictures start again from the same instant.
    pub fn flush(&mut self) {
        self.decoder.flush();
        // The resampler holds a few samples of its own. They are from before
        // the seek too, and prepending them to what lands after it would put
        // the ring's media time out by however many they are.
        if let Some(resampler) = &mut self.resampler {
            let mut spill = ff::frame::Audio::new(self.format, self.rate as usize, self.layout);
            while matches!(resampler.flush(&mut spill), Ok(Some(_))) {}
        }
        self.pipe.flush();
    }

    fn drain(&mut self) -> Fallible<()> {
        // A handle rather than a borrow of `self`, so the conversion below can
        // hold the scratch buffer while it writes.
        let pipe = self.pipe.clone();
        let mut frame = ff::frame::Audio::empty();
        while self.decoder.receive_frame(&mut frame).is_ok() {
            let through = self.through(&frame);
            self.aim(pipe.offset(), &frame)?;
            if self.weave(&frame)? {
                pipe.write(&self.woven, through);
            }
        }
        Ok(())
    }

    /// Point the resampler at the picture: a ratio a few parts per million off
    /// 1, which over a minute is the difference between the sound card's
    /// crystal and `CLOCK_MONOTONIC`.
    fn aim(&mut self, offset: i64, first: &ff::frame::Audio) -> Fallible<()> {
        let distance = self.rate * DISTANCE;
        let delta = compensation(offset, distance);
        let resampler = self.resampler(first)?;
        // ffmpeg turns its resampler on for this when the two rates are equal,
        // which they usually are: 48 kHz sound into a 48 kHz device. That
        // happens on the first call, before any samples have gone through.
        unsafe {
            ff::ffi::swr_set_compensation(resampler.as_mut_ptr(), delta, distance as c_int);
        }
        Ok(())
    }

    fn resampler(&mut self, first: &ff::frame::Audio) -> Fallible<&mut Resampler> {
        if self.resampler.is_none() {
            self.resampler = Some(first.resampler(self.format, self.layout, self.rate)?);
        }
        self.resampler
            .as_mut()
            .ok_or_else(|| "the resampler went away".into())
    }

    /// Resample one decoded frame into [`Self::woven`], interleaved. `false`
    /// when the resampler kept everything it was given, which it does at the
    /// start of a stream.
    fn weave(&mut self, frame: &ff::frame::Audio) -> Fallible<bool> {
        let (format, layout, rate, channels) = (self.format, self.layout, self.rate, self.channels);
        // The resampler can hand out more than it took, so the room is
        // computed from the rate change rather than assumed to be one for one.
        let room = (frame.samples() as u64 * u64::from(rate)) / u64::from(frame.rate().max(1));
        let mut out = ff::frame::Audio::new(format, room as usize + 1024, layout);
        out.set_rate(rate);
        self.resampler(frame)?.run(frame, &mut out)?;
        if out.samples() == 0 {
            return Ok(false);
        }

        self.woven.clear();
        self.woven.resize(out.samples() * channels, 0.0);
        for channel in 0..channels.min(out.planes()) {
            for (index, sample) in out.plane::<f32>(channel).iter().enumerate() {
                self.woven[index * channels + channel] = *sample;
            }
        }
        Ok(true)
    }

    /// Media time just past this frame's last sample, which is what the ring's
    /// head-time arithmetic is measured from. Taken from the container's own
    /// timestamp rather than counted, so a gap in the file is a gap in the
    /// sound rather than a slide in everything after it.
    fn through(&self, frame: &ff::frame::Audio) -> Duration {
        let at = media_time(frame.timestamp().unwrap_or(0), self.start, self.time_base);
        let held = frame.samples() as f64 / f64::from(frame.rate().max(1));
        at + Duration::from_secs_f64(held)
    }
}
