//! The audio device, and the only file in this crate that names cpal.
//!
//! **Why cpal and not what cosmic-player uses.** cosmic-player has no audio
//! output code to copy: its player is `iced_video_player`, which is GStreamer
//! `playbin` with the *video* sink replaced by an appsink
//! (cosmic-player `src/video.rs:20-26`), so the sound leaves through playbin's
//! default `autoaudiosink` and its volume and mute are playbin properties
//! (`src/main.rs:1225-1235`). No COSMIC first-party binary on this box links
//! an audio library for playback at all; `cosmic-settings-daemon` links
//! libpipewire, and that is for routing rather than for playing anything.
//! GStreamer is already rejected here for the frame path
//! (docs/ARCHITECTURE.md, decisions log 2026-07-30: no wgpu or dmabuf sink),
//! and pulling it in for the sound alone would put a second media framework
//! in the tree for one stereo track. Issue #13 names the alternative: cpal, or
//! PipeWire directly. cpal is the smaller of the two by a wide margin, and
//! PipeWire is still what plays this, through `pipewire-alsa`.
//!
//! The cost is one apt package, `libasound2-dev`: cpal's Linux target depends
//! on `alsa` unconditionally, whatever host it ends up using.

use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, StreamConfig, SupportedStreamConfig};

use super::Fallible;
use super::audio::{Beat, Pipe, Reading};

/// How much sound the ring holds. Two orders of magnitude more than a device
/// callback needs, because what it is really covering is the decode thread
/// blocking on the picture queue, which it does for a frame at a time.
const DEPTH: Duration = Duration::from_millis(500);

/// An open output device with the file's sound going to it.
///
/// Dropping this stops the stream and closes the device, which is what closing
/// a file does.
pub struct Sound {
    pipe: Pipe,
    rate: u32,
    channels: usize,
    _stream: cpal::Stream,
}

impl Sound {
    /// Opens the default output device and starts it running.
    ///
    /// The stream runs for as long as the file is open, playing or not: a
    /// paused player writes silence rather than stopping the device, because
    /// starting and stopping a device is the one thing on this path that is
    /// both slow and audible.
    pub fn open(beat: &Arc<Beat>, wanted: u32) -> Fallible<Self> {
        let beat = beat.clone();
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no audio output device")?;
        let chosen = choose(&device, wanted)?;
        let (rate, channels) = (chosen.sample_rate(), chosen.channels() as usize);
        let config = StreamConfig::from(chosen);
        let pipe = Pipe::new(rate, channels, DEPTH);

        let filling = pipe.clone();
        let mut reading = Reading::default();
        let stream = device.build_output_stream(
            config,
            move |out: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let now = Instant::now();
                reading = beat.read(reading);
                // What is written here is heard when the device gets to it,
                // and where the picture will be *then* is what the sound has
                // to match.
                let stamp = info.timestamp();
                let latency = stamp.playback.duration_since(stamp.callback);
                filling.fill(out, reading.running_at(now + latency));
            },
            |e| eprintln!("kyerag: sound stopped: {e}"),
            None,
        )?;
        stream.play()?;

        println!("sound:  {rate} Hz, {channels} channel(s), f32");
        Ok(Self {
            pipe,
            rate,
            channels,
            _stream: stream,
        })
    }

    /// A handle on the ring, for the decode thread and the shell.
    pub fn pipe(&self) -> Pipe {
        self.pipe.clone()
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> usize {
        self.channels
    }
}

/// A configuration that takes `f32` at the file's own rate if the device will,
/// and the device's default otherwise.
///
/// Only `f32` is asked for. Every ALSA device PipeWire presents accepts it, and
/// the alternative is a conversion per sample format for a case nothing on this
/// desktop reaches.
fn choose(device: &Device, wanted: u32) -> Fallible<SupportedStreamConfig> {
    let float = |config: &SupportedStreamConfig| config.sample_format() == SampleFormat::F32;

    let ranges: Vec<_> = device
        .supported_output_configs()
        .map(Iterator::collect)
        .unwrap_or_default();
    let exact = ranges
        .iter()
        .filter(|range| range.sample_format() == SampleFormat::F32)
        .filter(|range| range.contains_rate(wanted))
        // Two channels for a stereo track. A device that will not take two
        // takes whatever it offers, and the resampler lays the track out for
        // it.
        .min_by_key(|range| range.channels().abs_diff(2))
        .map(|range| (*range).with_sample_rate(wanted));
    if let Some(config) = exact {
        return Ok(config);
    }

    let default = device.default_output_config()?;
    match float(&default) {
        true => Ok(default),
        false => Err(format!(
            "the audio device takes {:?}, not f32",
            default.sample_format()
        )
        .into()),
    }
}
