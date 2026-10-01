//! The file's sound: AAC decoding and resampling, independent of video delivery.
//!
//! Production consumes its own bounded packet queue from the capture demuxer.
//! That reader can pass the measured 67 MB picture interleave gap (issue #97)
//! without holding decoder surfaces. It replaces competing audio/video file
//! cursors, not the independent audio producer. Tests retain an audio-only
//! demuxer as a reference.
//!
//! What leaves the decoder is planar `fltp` at the file's own rate; what the
//! device wants is interleaved at the device's rate and channel count. So
//! `swresample` sits between them, and it is there for a second reason as
//! well: `swr_set_compensation` is the drift correction ([`super::audio`]),
//! and it exists precisely to slave a sound to a clock that is not the sound
//! card's.

use std::ffi::c_int;
#[cfg(test)]
use std::path::Path;
use std::time::Duration;

use ffmpeg_next as ff;

use super::audio::{AudioEpoch, Pipe, compensation};
use super::packet_input::{AudioTimeline, PacketInput};
use super::{Fallible, media_time};
#[cfg(test)]
use super::{packet_input::Limits, read_only};
use ff::format::context::Input;

/// Output frames the drift correction is spread over: one second. Long enough
/// that the ratio is a rounding error, short enough that it is re-aimed before
/// the file has moved far.
const DISTANCE: u32 = 1;

/// How much room the ring must have before another packet is read. One AAC
/// packet is 21 ms of sound and a decoder can hand over more than one at a
/// time, so the margin is a few of them: read past it and [`Pipe::write`]
/// would drop what it had just read.
pub(crate) const HEADROOM: Duration = Duration::from_millis(100);

type Resampler = ff::software::resampling::Context;

/// Owned stream metadata retained before the capture input moves to its reader.
pub(crate) struct AudioSpec {
    timeline: AudioTimeline,
    parameters: ff::codec::Parameters,
}

impl AudioSpec {
    pub(crate) fn inspect(input: &Input) -> Option<Self> {
        let stream = input
            .streams()
            .find(|s| s.parameters().medium() == ff::media::Type::Audio)?;
        Some(Self {
            timeline: AudioTimeline {
                stream: stream.index(),
                time_base: stream.time_base(),
                start: match stream.start_time() {
                    ff::ffi::AV_NOPTS_VALUE => 0,
                    start => start,
                },
            },
            parameters: stream.parameters().clone(),
        })
    }

    pub(crate) fn timeline(&self) -> AudioTimeline {
        self.timeline
    }

    pub(crate) fn rate(&self) -> Option<u32> {
        let rate = unsafe { (*self.parameters.as_ptr()).sample_rate };
        u32::try_from(rate).ok().filter(|rate| *rate > 0)
    }
}

/// One audio stream decoded and resampled into the device's own format.
pub struct Track {
    /// Audio packets from the capture's reader, or the test-only reference.
    input: PacketInput,
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
    rate: u32,
    channels: usize,
    /// Stream time base, and the PTS the container starts from.
    time_base: ff::Rational,
    start: i64,
    pipe: Pipe,
    epoch: AudioEpoch,
    /// Interleaved scratch the planar output is woven into, kept between
    /// chunks so a packet costs no allocation.
    woven: Vec<f32>,
}

impl Track {
    pub(crate) fn epoch(&self) -> AudioEpoch {
        self.epoch.clone()
    }

    /// Opens `path` again for its first audio stream, if it has one, to be
    /// resampled into `rate` and `channels`.
    ///
    /// `Ok(None)` is a file with no sound in it, which the older cameras'
    /// per-lens files are. Those play their pictures exactly as before, and
    /// silently rather than by refusing to open.
    #[cfg(test)]
    pub fn open(path: &Path, pipe: Pipe, rate: u32, channels: usize) -> Fallible<Option<Self>> {
        let input = super::capture::open_input(path)?;
        Self::from_input(input, pipe, rate, channels)
    }

    #[cfg(test)]
    fn from_input(
        mut input: Input,
        pipe: Pipe,
        rate: u32,
        channels: usize,
    ) -> Fallible<Option<Self>> {
        let Some(spec) = AudioSpec::inspect(&input) else {
            return Ok(None);
        };
        read_only(&mut input, &[spec.timeline.stream]);
        Self::from_packets(
            spec,
            PacketInput::new(input, Limits::AUDIO)?,
            pipe,
            rate,
            channels,
        )
        .map(Some)
    }

    pub(crate) fn from_packets(
        spec: AudioSpec,
        input: PacketInput,
        pipe: Pipe,
        rate: u32,
        channels: usize,
    ) -> Fallible<Self> {
        let context = ff::codec::context::Context::from_parameters(spec.parameters)?;
        Ok(Self {
            input,
            stream: spec.timeline.stream,
            drained: false,
            decoder: context.decoder().audio()?,
            resampler: None,
            format: ff::format::Sample::F32(ff::format::sample::Type::Planar),
            rate,
            channels,
            time_base: spec.timeline.time_base,
            start: spec.timeline.start,
            epoch: pipe.epoch(),
            pipe,
            woven: Vec::new(),
        })
    }

    /// Where the samples are going: the plain layout for [`Self::channels`].
    ///
    /// Derived at each use rather than held in a field, because ffmpeg 7's
    /// `AVChannelLayout` carries two raw pointers (`u.map` for a custom
    /// order, and `opaque`) and so is not `Send`. A `Track` rides its
    /// `Reader` onto the decode thread ([`Player::open`](super::Player::open)),
    /// and `channels` is the whole of the input anyway.
    fn layout(&self) -> ff::ChannelLayout {
        ff::ChannelLayout::default(self.channels as i32)
    }

    /// Read sound until the ring is nearly full, and no further.
    ///
    /// The retained interleave regression uses this synchronous reference.
    /// Production performs one packet per independent producer turn.
    #[cfg(test)]
    pub fn pump(&mut self) -> Fallible<()> {
        while !self.drained && self.pipe.room() > HEADROOM {
            self.pump_one()?;
        }
        Ok(())
    }

    /// One packet per producer turn leaves seeks and shutdown interruptible
    /// between packets, without changing decoding or resampling arithmetic.
    pub(crate) fn pump_one(&mut self) -> Fallible<bool> {
        if !self.drained {
            let pipe = self.pipe.clone();
            let epoch = self.epoch.clone();
            match self.input.read_while(|| pipe.is_current(&epoch))? {
                // Every other stream is discarded, so this is the sound's own
                // packet; the guard is for a container that puts something
                // else through anyway.
                Some(packet) if packet.stream() == self.stream => self.take(&packet)?,
                Some(_) => {}
                // Cancellation is not EOF and must not flush the AAC decoder.
                // The producer will handle its pending seek or shutdown next.
                None if !pipe.is_current(&epoch) => return Ok(false),
                None => {
                    self.drained = true;
                    self.end()?;
                }
            }
        }
        Ok(self.drained)
    }

    /// Put the sound where a seek has put the pictures, and throw away
    /// everything decoded before it.
    ///
    /// `to` is media time in microseconds, the same number
    /// [`Reader::seek`](super::Reader::seek) gives its own demuxers, so both
    /// land on the same instant. AAC frames are all keyframes, so this lands
    /// within one packet of it.
    pub(crate) fn seek_in(&mut self, to: i64, epoch: AudioEpoch) -> Fallible<()> {
        self.flush();
        self.input.seek(to)?;
        self.drained = false;
        self.epoch = epoch;
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
        let layout = self.layout();
        if let Some(resampler) = &mut self.resampler {
            let mut spill = ff::frame::Audio::new(self.format, self.rate as usize, layout);
            while matches!(resampler.flush(&mut spill), Ok(Some(_))) {}
        }
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
                pipe.write_in(&self.epoch, &self.woven, through);
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
            self.resampler = Some(first.resampler(self.format, self.layout(), self.rate)?);
        }
        self.resampler
            .as_mut()
            .ok_or_else(|| "the resampler went away".into())
    }

    /// Resample one decoded frame into [`Self::woven`], interleaved. `false`
    /// when the resampler kept everything it was given, which it does at the
    /// start of a stream.
    fn weave(&mut self, frame: &ff::frame::Audio) -> Fallible<bool> {
        let (format, layout, rate, channels) =
            (self.format, self.layout(), self.rate, self.channels);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routed_aac_decoder_matches_audio_only_pcm_without_video_consumption() {
        let fixture = crate::capture_fixture::FixtureDir::new();
        let path = fixture.write_audio("track-routed.mov");
        let input = super::super::capture::open_input(&path).unwrap();
        let spec = AudioSpec::inspect(&input).unwrap();
        let video = PacketInput::with_audio(input, Limits::VIDEO, Some(spec.timeline())).unwrap();
        let routed_pipe = Pipe::new(48_000, 1, Duration::from_millis(500));
        let reference_pipe = Pipe::new(48_000, 1, Duration::from_millis(500));
        let mut routed = Track::from_packets(
            spec,
            video.audio_reader().unwrap(),
            routed_pipe.clone(),
            48_000,
            1,
        )
        .unwrap();
        let mut reference = Track::open(&path, reference_pipe.clone(), 48_000, 1)
            .unwrap()
            .unwrap();
        let mut actual = [0.0; 960];
        let mut expected = [0.0; 960];
        for tick in 0..140 {
            routed.pump().unwrap();
            reference.pump().unwrap();
            let due = Duration::from_millis(tick * 20);
            routed_pipe.fill(&mut actual, Some(due));
            reference_pipe.fill(&mut expected, Some(due));
            assert_eq!(actual, expected, "decoded PCM at {due:?}");
        }
        assert_eq!(routed_pipe.health().underruns, 0);
        assert_eq!(routed_pipe.health().dropped, 0);
    }
}
