//! Compressed input is independent of decoded surfaces and GPU backpressure.
//!
//! One worker owns each demuxer. It fills a bounded packet queue even when
//! decode delivery is full, without holding VA-API surfaces or decoded PCM.
//! Seeks clear that queue and invalidate an in-flight read before repositioning
//! the same demuxer. No input operation or worker join runs on the UI thread.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};

use ffmpeg_next as ff;

use crate::Fallible;
use crate::file_input::Input;

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    bytes: usize,
    packets: usize,
}

impl Limits {
    pub(crate) const VIDEO: Self = Self {
        bytes: 64 * 1024 * 1024,
        packets: 512,
    };
    pub(crate) const AUDIO: Self = Self {
        bytes: 256 * 1024,
        packets: 128,
    };
}

trait Demux: Send + 'static {
    fn read(&mut self) -> Result<Option<ff::Packet>, String>;
    fn seek(&mut self, to: i64) -> Result<(), String>;
}

impl Demux for Input {
    fn read(&mut self) -> Result<Option<ff::Packet>, String> {
        let mut packet = ff::Packet::empty();
        let result = packet.read(self);
        if let Some(error) = self.failure() {
            return Err(error);
        }
        match result {
            Ok(()) => Ok(Some(packet)),
            Err(ff::Error::Eof) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn seek(&mut self, to: i64) -> Result<(), String> {
        ff::format::context::Input::seek(self, to, ..to)
            .map_err(|error| self.failure().unwrap_or_else(|| error.to_string()))
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
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

pub(crate) struct PacketInput(Arc<Shared>);

impl PacketInput {
    pub(crate) fn new(input: Input, limits: Limits) -> Fallible<Self> {
        Self::spawn(input, limits)
    }

    fn spawn(input: impl Demux, limits: Limits) -> Fallible<Self> {
        assert!(limits.bytes > 0 && limits.packets > 0);
        let shared = Arc::new(Shared::default());
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
        Ok(Self(shared))
    }

    pub(crate) fn read(&mut self) -> Fallible<Option<ff::Packet>> {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.started = true;
        self.0.changed.notify_all();
        loop {
            if let Some(packet) = state.packets.pop_front() {
                state.bytes -= packet.size();
                self.0.changed.notify_all();
                return Ok(Some(packet));
            }
            if let Some(result) = &state.terminal {
                return result.clone().map(|()| None).map_err(Into::into);
            }
            state = self
                .0
                .changed
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Only decoder/audio producers call this. The sole mutable consumer
    /// cannot read packets until the seek is acknowledged, and the generation
    /// check prevents a read already outside the lock from refilling old data.
    pub(crate) fn seek(&mut self, to: i64) -> Fallible<()> {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = &state.failure {
            return Err(error.clone().into());
        }
        state.generation = state.generation.wrapping_add(1);
        state.packets.clear();
        state.bytes = 0;
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
        state.stopped = true;
        state.packets.clear();
        state.bytes = 0;
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
                || state.packets.len() >= limits.packets)
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
                    state.bytes += packet.size();
                    state.packets.push_back(packet);
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
