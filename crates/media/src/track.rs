//! The file's sound: one AAC stream, on a demuxer of its own.
//!
//! **Why its own** (issue #97). The sound used to come off the pictures'
//! demuxer, because an `.insv` writes all three streams into one MP4 and one
//! file handle is simpler than two. The owner's April capture says that
//! cannot hold: it has one place, 4.885 s in, where the camera left 67 MB of
//! picture between two audio samples, and libavformat reads a file whose
//! streams are interleaved like that by letting one of them fall up to a
//! second behind (`mov_find_next_sample` reads in file order until the
//! timestamps differ by more than `AV_TIME_BASE`, and only then seeks). The
//! sound for those three and a half seconds therefore arrived after its
//! moment had passed, was dropped by the splice, and the owner heard silence
//! from 4.9 s to 8.2 s. No ring depth can fix that: the samples had not been
//! read yet, and the pictures cannot be read further ahead than the decoder's
//! surface pool allows.
//!
//! A demuxer of its own has no other stream to fall behind. It carries the
//! same file, with the pictures discarded, so libavformat seeks straight to
//! each audio chunk: 190 kbps of a 180 Mbps file, measured at 40x realtime
//! for the whole 30 minute capture. The cost is one more open of the
//! container (measured at 0.2 s on the 36 GB file) and one more file handle.
//!
//! What leaves the decoder is planar `fltp` at the file's own rate; what the
//! device wants is interleaved at the device's rate and channel count. So
//! `swresample` sits between them, and it is there for a second reason as
//! well: `swr_set_compensation` is the drift correction ([`super::audio`]),
//! and it exists precisely to slave a sound to a clock that is not the sound
//! card's.

use std::ffi::c_int;
use std::path::Path;
use std::time::Duration;

use ffmpeg_next as ff;

use super::audio::{Pipe, compensation};
use super::{Fallible, media_time, read_only};

/// Output frames the drift correction is spread over: one second. Long enough
/// that the ratio is a rounding error, short enough that it is re-aimed before
/// the file has moved far.
const DISTANCE: u32 = 1;

/// How much room the ring must have before another packet is read. One AAC
/// packet is 21 ms of sound and a decoder can hand over more than one at a
/// time, so the margin is a few of them: read past it and [`Pipe::write`]
/// would drop what it had just read.
const HEADROOM: Duration = Duration::from_millis(100);

type Resampler = ff::software::resampling::Context;

/// One audio stream, on its own demuxer, decoded and resampled into the
/// device's own format.
pub struct Track {
    /// The same file the pictures are read from, opened again with every
    /// other stream discarded. Two file handles rather than one, which is
    /// what the interleave costs (issue #97).
    input: ff::format::context::Input,
    stream: usize,
    /// The file has been read to its end. Cleared by a seek, which is the
    /// only way back into it.
    drained: bool,
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
    /// Opens `path` again for its first audio stream, if it has one, to be
    /// resampled into `rate` and `channels`.
    ///
    /// `Ok(None)` is a file with no sound in it, which the older cameras'
    /// per-lens files are. Those play their pictures exactly as before, and
    /// silently rather than by refusing to open.
    pub fn open(path: &Path, pipe: Pipe, rate: u32, channels: usize) -> Fallible<Option<Self>> {
        let mut input = ff::format::input(&path)?;
        let Some(stream) = input
            .streams()
            .find(|s| s.parameters().medium() == ff::media::Type::Audio)
        else {
            return Ok(None);
        };
        let (index, time_base, start) = (stream.index(), stream.time_base(), stream.start_time());
        let context = ff::codec::context::Context::from_parameters(stream.parameters())?;
        read_only(&mut input, &[index]);

        Ok(Some(Self {
            input,
            stream: index,
            drained: false,
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

    /// Read sound until the ring is nearly full, and no further.
    ///
    /// The ring is the pacing: the reader calls this once per turn of its own
    /// loop ([`Reader::read_until`](super::Reader::read_until)), which is at
    /// least once per pair of pictures, and it returns at once with nothing
    /// read when there is no room. Sound the ring cannot take yet is sound
    /// that would be dropped, and it is still in the file next time.
    pub fn pump(&mut self) -> Fallible<()> {
        while !self.drained && self.pipe.room() > HEADROOM {
            let mut packet = ff::Packet::empty();
            match packet.read(&mut self.input) {
                // Every other stream is discarded, so this is the sound's own
                // packet; the guard is for a container that puts something
                // else through anyway.
                Ok(()) if packet.stream() == self.stream => self.take(&packet)?,
                Ok(()) => {}
                Err(ff::Error::Eof) => {
                    self.drained = true;
                    self.end()?;
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// Put the sound where a seek has put the pictures, and throw away
    /// everything decoded before it.
    ///
    /// `to` is media time in microseconds, the same number
    /// [`Reader::seek`](super::Reader::seek) gives its own demuxers, so both
    /// land on the same instant. AAC frames are all keyframes, so this lands
    /// within one packet of it.
    pub fn seek(&mut self, to: i64) -> Fallible<()> {
        self.flush();
        self.input.seek(to, ..to)?;
        self.drained = false;
        Ok(())
    }

    /// One packet in, and everything it completes out to the device.
    fn take(&mut self, packet: &ff::Packet) -> Fallible<()> {
        self.decoder.send_packet(packet)?;
        self.drain()
    }

    /// The end of the file: whatever the decoder is still holding, which is
    /// the last few tens of milliseconds of the track.
    fn end(&mut self) -> Fallible<()> {
        self.decoder.send_eof()?;
        self.drain()
    }

    /// Throw away what is decoded but not yet heard. Paired with the video
    /// decoders' flush in [`Reader::seek`](super::Reader::seek), so the sound
    /// and the pictures start again from the same instant.
    fn flush(&mut self) {
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
