//! Compressed input is independent of decoded surfaces and GPU backpressure.
//!
//! One worker owns each demuxer. It fills a bounded packet queue even when
//! decode delivery is full, without holding VA-API surfaces or decoded PCM.
//! Seeks clear that queue and invalidate an in-flight read before repositioning
//! the same demuxer. No input operation or worker join runs on the UI thread.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use ffmpeg_next as ff;

use crate::Fallible;
use ff::format::context::Input;

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    bytes: usize,
    packets: usize,
}

impl Limits {
    pub(crate) const VIDEO: Self = Self {
        // The measured camera interleave includes 67 MiB of pictures before
        // the next audio chunk. Read past it without retaining VA surfaces.
        bytes: 128 * 1024 * 1024,
        packets: 512,
    };
    pub(crate) const AUDIO: Self = Self {
        bytes: 256 * 1024,
        packets: 128,
    };
}

#[derive(Clone, Copy)]
pub(crate) struct AudioTimeline {
    pub(crate) stream: usize,
    pub(crate) time_base: ff::Rational,
    pub(crate) start: i64,
}

impl AudioTimeline {
    fn needed(&self, packet: &ff::Packet, cutoff: Option<Duration>) -> bool {
        match (packet.pts().or(packet.dts()), cutoff) {
            (Some(at), Some(cutoff)) => {
                let end = at.saturating_add(packet.duration().max(0));
                crate::media_time(end, self.start, self.time_base) >= cutoff
            }
            _ => true,
        }
    }
}

#[derive(Default)]
struct AudioQueue {
    packets: VecDeque<ff::Packet>,
    bytes: usize,
    active: bool,
    cutoff: Option<Duration>,
}

trait Demux: Send + 'static {
    fn read(&mut self) -> Result<Option<ff::Packet>, String>;
    fn seek(&mut self, to: i64) -> Result<(), String>;
}

impl Demux for Input {
    fn read(&mut self) -> Result<Option<ff::Packet>, String> {
        let mut packet = ff::Packet::empty();
        let result = packet.read(self);
        match result {
            Ok(()) => Ok(Some(packet)),
            Err(ff::Error::Eof) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn seek(&mut self, to: i64) -> Result<(), String> {
        ff::format::context::Input::seek(self, to, ..to).map_err(|error| error.to_string())
    }
}

#[derive(Default)]
struct State {
    packets: VecDeque<ff::Packet>,
    bytes: usize,
    generation: u64,
    started: bool,
    stopped: bool,
    terminal: Option<Result<(), String>>,
    failure: Option<String>,
    seek: Option<i64>,
    seek_result: Option<Result<(), String>>,
    audio: AudioQueue,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    audio: Option<AudioTimeline>,
}

// The primary reader owns container seeking and shutdown. An audio reader is
// a second bounded packet consumer, never another demuxer or file cursor.
pub(crate) struct PacketInput(Arc<Shared>, bool);

impl PacketInput {
    #[cfg(test)]
    pub(crate) fn new(input: Input, limits: Limits) -> Fallible<Self> {
        Self::spawn(input, limits)
    }

    pub(crate) fn with_audio(
        input: Input,
        limits: Limits,
        audio: Option<AudioTimeline>,
    ) -> Fallible<Self> {
        Self::spawn_routed(input, limits, audio)
    }

    pub(crate) fn audio_reader(&self) -> Fallible<Self> {
        if self.0.audio.is_none() {
            return Err("container input has no audio stream".into());
        }
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.audio.active {
            return Err("container audio packets already have a consumer".into());
        }
        if state.started {
            return Err("audio must be attached before reading the capture".into());
        }
        state.audio.active = true;
        Ok(Self(self.0.clone(), true))
    }

    #[cfg(test)]
    fn spawn(input: impl Demux, limits: Limits) -> Fallible<Self> {
        Self::spawn_routed(input, limits, None)
    }

    fn spawn_routed(
        input: impl Demux,
        limits: Limits,
        audio: Option<AudioTimeline>,
    ) -> Fallible<Self> {
        assert!(limits.bytes > 0 && limits.packets > 0);
        let shared = Arc::new(Shared {
            audio,
            ..Shared::default()
        });
        let running = shared.clone();
        std::thread::Builder::new()
            .name("kjerag-demux".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(input, &running, limits);
                }));
                if let Err(payload) = outcome {
                    let error = payload
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_owned())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "input reader panicked".into());
                    let mut state = running.state.lock().unwrap_or_else(|e| e.into_inner());
                    state.terminal = Some(Err(error.clone()));
                    state.failure = Some(error.clone());
                    state.seek_result = Some(Err(error));
                    running.changed.notify_all();
                }
            })?;
        Ok(Self(shared, false))
    }

    pub(crate) fn read(&mut self) -> Fallible<Option<ff::Packet>> {
        self.read_while(|| true)
    }

    pub(crate) fn read_while(
        &mut self,
        current: impl Fn() -> bool,
    ) -> Fallible<Option<ff::Packet>> {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        let generation = state.generation;
        state.started = true;
        self.0.changed.notify_all();
        loop {
            // A blocked old audio turn must not consume a new seek's packet.
            // Returning no packet lets its pending seek flush the decoder.
            if state.generation != generation || !current() {
                return Ok(None);
            }
            let packet = if self.1 {
                let packet = state.audio.packets.pop_front();
                if let Some(packet) = &packet {
                    state.audio.bytes -= packet.size();
                }
                packet
            } else {
                let packet = state.packets.pop_front();
                if let Some(packet) = &packet {
                    state.bytes -= packet.size();
                }
                packet
            };
            if let Some(packet) = packet {
                self.0.changed.notify_all();
                return Ok(Some(packet));
            }
            if state.stopped {
                return Ok(None);
            }
            if let Some(result) = &state.terminal {
                return result.clone().map(|()| None).map_err(Into::into);
            }
            state = self
                .0
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// Only decoder/audio producers call this. The sole mutable consumer
    /// cannot read packets until the seek is acknowledged, and the generation
    /// check prevents a read already outside the lock from refilling old data.
    pub(crate) fn seek(&mut self, to: i64) -> Fallible<()> {
        self.seek_with_audio(to, u64::try_from(to).ok().map(Duration::from_micros))
    }

    /// Set the audio landing gate in the same transaction as the physical
    /// seek. Video can replay earlier history without filling the audio queue
    /// with pre-target packets or racing a previous seek's later gate.
    pub(crate) fn seek_with_audio(&mut self, to: i64, audio_at: Option<Duration>) -> Fallible<()> {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = &state.failure {
            return Err(error.clone().into());
        }
        if self.1 {
            let cutoff = Duration::from_micros(u64::try_from(to)?);
            let timeline = self
                .0
                .audio
                .ok_or("audio reader has no container timeline")?;
            state.audio.cutoff = Some(cutoff);
            state
                .audio
                .packets
                .retain(|packet| timeline.needed(packet, Some(cutoff)));
            state.audio.bytes = state.audio.packets.iter().map(ff::Packet::size).sum();
            self.0.changed.notify_all();
            return Ok(());
        }
        state.generation = state.generation.wrapping_add(1);
        state.packets.clear();
        state.bytes = 0;
        state.audio.packets.clear();
        state.audio.bytes = 0;
        state.audio.cutoff = audio_at;
        state.terminal = None;
        state.seek_result = None;
        state.seek = Some(to);
        self.0.changed.notify_all();
        loop {
            if let Some(result) = state.seek_result.take() {
                return result.map_err(Into::into);
            }
            state = self
                .0
                .changed
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}

impl Drop for PacketInput {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if self.1 {
            state.audio.active = false;
            state.audio.packets.clear();
            state.audio.bytes = 0;
            self.0.changed.notify_all();
            return;
        }
        state.stopped = true;
        state.packets.clear();
        state.bytes = 0;
        state.audio.packets.clear();
        state.audio.bytes = 0;
        self.0.changed.notify_all();
        // A filesystem read can still be in flight. Its worker owns the input
        // until that operation returns; destruction never waits for the disk.
    }
}

fn run(mut input: impl Demux, shared: &Shared, limits: Limits) {
    loop {
        let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
        while !state.stopped
            && state.seek.is_none()
            && (!state.started
                || state.terminal.is_some()
                || state.bytes >= limits.bytes
                || state.packets.len() >= limits.packets
                || (state.audio.active
                    && (state.audio.bytes >= Limits::AUDIO.bytes
                        || state.audio.packets.len() >= Limits::AUDIO.packets)))
        {
            state = shared
                .changed
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
        if state.stopped {
            return;
        }
        let generation = state.generation;
        if let Some(to) = state.seek.take() {
            drop(state);
            let result = input.seek(to);
            state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.started = true;
            state.terminal = result.as_ref().err().map(|error| Err(error.clone()));
            state.seek_result = Some(result);
        } else {
            drop(state);
            let result = input.read();
            state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.generation != generation || state.stopped {
                continue;
            }
            match result {
                Ok(Some(packet)) => {
                    // The size is known only after reading. The byte limit
                    // therefore permits at most one packet of overshoot; the
                    // independent count limit also bounds tiny/empty packets.
                    if let Some(audio) = shared.audio
                        && packet.stream() == audio.stream
                    {
                        if state.audio.active && audio.needed(&packet, state.audio.cutoff) {
                            state.audio.bytes += packet.size();
                            state.audio.packets.push_back(packet);
                        }
                    } else {
                        state.bytes += packet.size();
                        state.packets.push_back(packet);
                    }
                }
                Ok(None) => state.terminal = Some(Ok(())),
                Err(error) => state.terminal = Some(Err(error)),
            }
        }
        shared.changed.notify_all();
    }
}

#[cfg(test)]
mod tests;
