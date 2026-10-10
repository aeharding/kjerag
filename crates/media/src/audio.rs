//! The sound between the decode thread and the audio device, and the
//! presentation clock in a form the device's callback can read.
//!
//! **The picture is the clock.** Issue #4 anchors playback on video frames and
//! nothing here re-anchors it: every device callback asks where the picture is
//! and makes the sound follow. A sound-mastered clock would move the picture
//! instead, and a reframing player whose frames are paced by a sound card is a
//! player that judders.
//!
//! Following it takes two corrections, and they are different in kind:
//!
//! - **A splice**, in [`Buffer::measure`]: the head of the ring is a long way
//!   from the picture, so sound whose moment has passed is dropped and sound
//!   whose moment has not come waits under silence. A start, a seek landing
//!   and a recovery from a stall all arrive here. The ramp comes down before
//!   the join and up after it, because cutting a live waveform is a click.
//! - **A drift**, in [`compensation`]: the sound card's crystal and
//!   `CLOCK_MONOTONIC` are not the same clock, so a ring that is exactly right
//!   now is tens of milliseconds out half an hour later. The decode thread
//!   resamples by a few parts per million to hold it, which is what
//!   `swr_set_compensation` exists for.
//!
//! Everything in this file is arithmetic over a ring of samples: no ffmpeg and
//! no device, so `cargo test` covers it on a box with no sound card.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// How far the sound may be from the picture before it is spliced rather than
/// resampled back. Below this the drift correction walks it in with no join in
/// the sound; above it, no rate a listener would not hear could close the gap
/// in reasonable time.
const SPLICE: Duration = Duration::from_millis(30);

/// How long a gain change takes. Long enough that the step at a pause is below
/// hearing, short enough that the sound it costs at a splice is not.
const FADE: Duration = Duration::from_millis(5);

/// The share of the error corrected per second of playback, which makes the
/// drift correction a first-order lag with a 2 s time constant: slower than
/// anything a listener tracks and faster than a crystal wanders.
const PULL: f64 = 0.5;

/// The largest resampling ratio the drift correction will ask for: 0.5%, or
/// 8.6 cents of pitch. Two crystals differ by tens of parts per million, so the
/// ratio in steady state is a hundredth of this and the cap is reached only
/// while a stall is being walked off.
const SLEW: f64 = 0.005;

/// What the sound did, for the report the app prints every five seconds. Every
/// count here is a defect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Audio {
    /// Device callbacks with no sound to fill them while the picture was
    /// moving. A hole in the sound, in other words.
    pub underruns: u64,
    /// Chunks the decode thread had decoded and could not hand over, because
    /// the ring was full. The gap they leave is spliced out.
    pub dropped: u64,
    /// Where the sound was against the picture when it was last measured, in
    /// microseconds. Positive is sound ahead of picture.
    pub offset: i64,
    /// The furthest from the picture it has been, either way, since the file
    /// was opened. A maximum cannot be subtracted, so unlike the counts this
    /// one is not windowed.
    pub worst: i64,
    /// Sound waiting in the ring when the device last looked at it, in
    /// microseconds. A level rather than a count, and the one number that
    /// tells a hole in the sound caused by decode falling behind from a hole
    /// caused by sound arriving too late to play: the first empties the ring,
    /// the second fills it with sound whose moment has passed (issue #97).
    pub queued: i64,
}

impl Audio {
    /// What happened between an earlier reading and this one.
    pub fn since(self, earlier: Self) -> Self {
        Self {
            underruns: self.underruns.saturating_sub(earlier.underruns),
            dropped: self.dropped.saturating_sub(earlier.dropped),
            offset: self.offset,
            worst: self.worst,
            queued: self.queued,
        }
    }

    /// The sound's share of the playback report line.
    pub fn report(&self) -> String {
        format!(
            "sound {:+.1} ms (worst {:.1}), {} underruns, {} dropped",
            self.offset as f64 / 1000.0,
            self.worst as f64 / 1000.0,
            self.underruns,
            self.dropped,
        )
    }
}

/// Where the presentation clock is, copied out of it whenever it moves.
///
/// Deliberately the same type the presenter runs on: the sound has to answer
/// "where is the picture" with the arithmetic the picture answers it with, or
/// the two agree on a number that neither of them shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    pub playing: bool,
    /// Media time at `origin`, and the whole answer while paused.
    pub position: Duration,
    /// When the clock was last anchored to a frame. `None` before the first
    /// frame of a start or a seek has arrived.
    pub origin: Option<Instant>,
}

impl Reading {
    /// Media time at `at`, running or not.
    pub fn position(self, at: Instant) -> Duration {
        self.running_at(at).unwrap_or(self.position)
    }

    /// Media time at `at`, or `None` when the clock is not running towards
    /// anything: paused, or playing and not yet anchored to a first frame. The
    /// sound is silent for exactly that answer.
    pub fn running_at(self, at: Instant) -> Option<Duration> {
        let origin = self.origin.filter(|_| self.playing)?;
        Some(self.position + at.saturating_duration_since(origin))
    }
}

/// The presentation clock, readable from the device's callback thread.
///
/// Written on play, pause, seek and the frame that anchors the clock, which is
/// a handful of times a minute; read once per callback. The callback takes it
/// with `try_lock` and keeps the last reading it got while the writer holds
/// it: the sound it emits does not depend on this value, only the correction
/// applied to the sound after it.
#[derive(Debug, Default)]
pub struct Beat(Mutex<Reading>);

impl Beat {
    pub fn publish(&self, reading: Reading) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = reading;
    }

    /// The latest reading, or `was` while the writer holds the lock.
    pub fn read(&self, was: Reading) -> Reading {
        self.0.try_lock().map(|held| *held).unwrap_or(was)
    }
}

/// Samples on their way to the device, shared by three threads: the decode
/// thread writes, the device callback reads, and the shell sets the volume and
/// throws the ring away on a seek.
///
/// One lock covers the samples and the media time they carry, because the two
/// are only meaningful together: a ring depth with no media time at its head
/// says nothing about whether the sound is late.
#[derive(Clone)]
pub struct Pipe(Arc<Mutex<Buffer>>);

/// A producer's authorization to write one audio lineage. Flush replaces it
/// under the ring lock, so an old packet cannot refill a newly cleared ring.
#[derive(Clone, Debug)]
pub(crate) struct AudioEpoch(Arc<()>);

impl AudioEpoch {
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Pipe {
    /// A ring `depth` long, in the device's own format.
    pub fn new(rate: u32, channels: usize, depth: Duration) -> Self {
        Self(Arc::new(Mutex::new(Buffer::new(rate, channels, depth))))
    }

    fn locked(&self) -> MutexGuard<'_, Buffer> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Take one decoded and resampled chunk. `through` is the media time just
    /// past its last frame, and it is what the whole clock slave measures
    /// from.
    #[cfg(test)]
    pub fn write(&self, samples: &[f32], through: Duration) {
        self.locked().write(samples, through);
    }

    pub(crate) fn epoch(&self) -> AudioEpoch {
        self.locked().epoch.clone()
    }

    pub(crate) fn is_current(&self, epoch: &AudioEpoch) -> bool {
        self.locked().epoch.same(epoch)
    }

    pub(crate) fn write_in(&self, epoch: &AudioEpoch, samples: &[f32], through: Duration) {
        let mut buffer = self.locked();
        if buffer.epoch.same(epoch) {
            buffer.write(samples, through);
        }
    }

    /// Throw the sound away, and fade what is already on its way out.
    ///
    /// A seek calls this, on the shell's thread. Everything in the ring was
    /// decoded before it, and playing that after a scrub is exactly the stale
    /// tail the epoch discipline exists to prevent (issue #5).
    pub(crate) fn invalidate(&self) -> AudioEpoch {
        let mut buffer = self.locked();
        buffer.epoch = AudioEpoch(Arc::new(()));
        buffer.flush();
        buffer.epoch.clone()
    }

    pub fn set_volume(&self, volume: f32) {
        // RON settings can contain NaN, which clamp preserves. It is not a
        // gain target: approach would walk below zero instead of converging.
        // Fail silent through the existing fade, retaining all other clamps.
        self.locked().volume = if volume.is_nan() {
            0.0
        } else {
            volume.clamp(0.0, 1.0)
        };
    }

    pub fn set_muted(&self, muted: bool) {
        self.locked().muted = muted;
    }

    pub fn health(&self) -> Audio {
        self.locked().health
    }

    /// Read-only overlay snapshot. Never wait behind an audio callback.
    pub(crate) fn buffer_snapshot(&self) -> Option<(Duration, Audio)> {
        let buffer = self.0.try_lock().ok()?;
        let queued = if buffer.stale {
            Duration::ZERO
        } else {
            buffer.seconds(buffer.frames)
        };
        Some((queued, buffer.health))
    }

    /// How much more sound the ring would take. This is the pacing for the
    /// sound's own producer: it reads until
    /// the ring is nearly full and stops, so nothing it reads is ever
    /// dropped for want of room.
    ///
    /// Zero while a seek's flush is still on its way through the device
    /// callback. Everything written then is thrown away, so reading it would
    /// be reading the file for nothing.
    pub fn room(&self) -> Duration {
        let buffer = self.locked();
        match buffer.stale {
            true => Duration::ZERO,
            false => buffer.seconds(buffer.room()),
        }
    }

    /// Whether this lineage has decoded sound through the requested lead.
    /// The existing callback owns timestamp gaps and device latency. Holding
    /// playback must not flush its ring or discard the fade's remaining sound.
    pub(crate) fn buffered_through(&self, at: Duration, lead: Duration) -> bool {
        let buffer = self.locked();
        buffer.muted
            || buffer.volume == 0.0
            || (!buffer.stale && buffer.frames > 0 && buffer.through >= at.saturating_add(lead))
    }

    /// Once the callback has faded to silence, remove only sound whose PTS
    /// precedes the held clock. Otherwise a full old ring could prevent refill
    /// forever. This is the existing late-audio splice, without a seek/flush.
    pub(crate) fn discard_before_if_silent(&self, at: Duration) {
        let mut buffer = self.locked();
        if !buffer.stale && buffer.gain == 0.0 && at > buffer.head_time() {
            let behind = i64::try_from((at - buffer.head_time()).as_micros()).unwrap_or(i64::MAX);
            let frames = buffer.frames_in(behind);
            buffer.drop_front(frames);
        }
    }

    /// How the sound stood against the picture when it was last measured, in
    /// microseconds, positive when the sound is ahead. The decode thread turns
    /// this into a resampling ratio.
    pub fn offset(&self) -> i64 {
        self.locked().health.offset
    }

    /// One device callback's worth of sound, interleaved at the device's
    /// channel count. `due` is where the picture will be when the first frame
    /// written here is heard, and `None` is a clock that is not running.
    pub fn fill(&self, out: &mut [f32], due: Option<Duration>) {
        self.locked().fill(out, due);
    }
}

/// What the head of the ring needs before it can be played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Splice {
    /// It is where the picture is, to within [`SPLICE`].
    None,
    /// It is behind: this many frames of it have already had their moment.
    Skip(usize),
    /// It is ahead: the picture has not reached it, and silence is what
    /// belongs in the gap until it does.
    Wait,
}

/// The ring, the media time it carries, and the gain being applied to it.
struct Buffer {
    epoch: AudioEpoch,
    /// Interleaved frames, `channels` values each, laid out as a ring. Half
    /// holds decoded successors; half retains recently consumed samples so a
    /// stopped media clock can resume behind the device's queued-ahead sound.
    samples: Box<[f32]>,
    channels: usize,
    rate: u32,
    /// Where the next frame out starts, as an index into `samples`.
    head: usize,
    /// Frames held.
    frames: usize,
    /// Contiguous samples before `head`, retained for a stopped-clock resume.
    past: usize,
    /// A stopped callback was observed. Rewind is not a steady-state drift
    /// correction: uninterrupted playback retains the existing splice law.
    resume_pending: bool,
    /// Media time just past the last frame written, which together with
    /// `frames` is what says where the head of the ring belongs.
    through: Duration,
    /// A seek has thrown this ring away. The callback fades out over what is
    /// left, drops it, and clears the flag; nothing is written while it
    /// stands.
    stale: bool,
    volume: f32,
    muted: bool,
    /// Applied gain. Ramped rather than stepped, because a step from a
    /// half-scale sample to zero is a click and pausing does exactly that.
    gain: f32,
    /// The most the gain may change in one frame.
    step: f32,
    health: Audio,
}

impl Buffer {
    fn new(rate: u32, channels: usize, depth: Duration) -> Self {
        let frames = (depth.as_secs_f64() * f64::from(rate)).ceil() as usize;
        Self {
            epoch: AudioEpoch(Arc::new(())),
            samples: vec![0.0; frames.max(1) * 2 * channels].into_boxed_slice(),
            channels,
            rate,
            head: 0,
            frames: 0,
            past: 0,
            resume_pending: false,
            through: Duration::ZERO,
            stale: false,
            volume: 1.0,
            muted: false,
            gain: 0.0,
            step: 1.0 / (FADE.as_secs_f32() * rate as f32),
            health: Audio::default(),
        }
    }

    fn capacity(&self) -> usize {
        self.samples.len() / self.channels / 2
    }

    fn room(&self) -> usize {
        // A resume can temporarily restore more than the normal future lead.
        // The producer waits for that restored sound rather than overwriting it.
        self.capacity().saturating_sub(self.frames)
    }

    fn seconds(&self, frames: usize) -> Duration {
        Duration::from_secs_f64(frames as f64 / f64::from(self.rate))
    }

    fn frames_in(&self, micros: i64) -> usize {
        (micros as f64 * 1e-6 * f64::from(self.rate)) as usize
    }

    /// Media time of the frame at the head of the ring, which is the next one
    /// the device will hear.
    fn head_time(&self) -> Duration {
        self.through.saturating_sub(self.seconds(self.frames))
    }

    fn write(&mut self, samples: &[f32], through: Duration) {
        // A ring that is about to be thrown away takes nothing: the fresh
        // sound would land behind samples from before the seek.
        if self.stale {
            return;
        }
        let frames = samples.len() / self.channels;
        // `through` moves whether the chunk fits or not, so the hole a
        // dropped one leaves is a hole the head-time arithmetic can see and
        // the next splice takes out. Writing part of a chunk instead would
        // leave the media time lying.
        let fits = frames <= self.room();
        self.through = through;
        if !fits {
            self.past = 0;
            self.health.dropped += 1;
            return;
        }
        for (index, value) in samples.iter().enumerate() {
            let at = (self.head + self.frames * self.channels + index) % self.samples.len();
            self.samples[at] = *value;
        }
        self.frames += frames;
    }

    fn flush(&mut self) {
        self.stale = true;
    }

    fn clear(&mut self) {
        self.head = 0;
        self.frames = 0;
        self.past = 0;
        self.resume_pending = false;
        self.stale = false;
    }

    /// Drop `frames` from the head: sound whose moment has passed.
    fn drop_front(&mut self, frames: usize) {
        let frames = frames.min(self.frames);
        self.head = (self.head + frames * self.channels) % self.samples.len();
        self.frames -= frames;
        self.past = (self.past + frames).min(self.capacity());
    }

    /// Device callbacks consume future audio before it is heard. A pause
    /// freezes the media clock earlier than that read head, then consumes its
    /// fade. Restore the actual retained samples at the resumed device PTS,
    /// without moving the media clock, manufacturing sound or changing the
    /// ordinary splice threshold. Missing history still produces a real gap.
    fn rewind_to(&mut self, due: Duration) {
        let ahead = self.head_time().saturating_sub(due);
        let micros = i64::try_from(ahead.as_micros()).unwrap_or(i64::MAX);
        let frames = self.frames_in(micros).min(self.past);
        self.head = (self.head + self.samples.len() - frames * self.channels) % self.samples.len();
        self.frames += frames;
        self.past -= frames;
    }

    /// The head of the ring against the picture, in microseconds, positive
    /// when the sound is ahead of it.
    fn error(&self, due: Duration) -> i64 {
        self.head_time().as_micros() as i64 - due.as_micros() as i64
    }

    /// Where the head of the ring sits against the picture, recorded for the
    /// report on the way past.
    fn measure(&mut self, due: Duration) -> Splice {
        let error = self.error(due);
        self.health.offset = error;
        self.health.worst = self.health.worst.max(error.abs());
        let splice = SPLICE.as_micros() as i64;
        if error <= -splice {
            return Splice::Skip(self.frames_in(-error));
        }
        if error >= splice {
            return Splice::Wait;
        }
        Splice::None
    }

    /// The gain this callback is heading for: the pilot's volume while the
    /// picture is moving, and zero for everything else.
    fn target(&self, running: bool) -> f32 {
        match running && !self.stale && !self.muted {
            true => self.volume,
            false => 0.0,
        }
    }

    /// One device callback's worth of sound.
    fn fill(&mut self, out: &mut [f32], due: Option<Duration>) {
        out.fill(0.0);
        if due.is_none() && !self.stale {
            self.resume_pending = true;
        }
        if let Some(due) =
            due.filter(|_| self.resume_pending && self.gain == 0.0 && self.target(true) > 0.0)
        {
            self.rewind_to(due);
            self.resume_pending = false;
        }
        self.health.queued = self.seconds(self.frames).as_micros() as i64;
        // What the pilot asked to hear, kept from before the splice logic
        // lowers it: a hole is a hole whether or not the ring was also too
        // far out to play, and `target` is zero in both cases.
        let asked = self.target(due.is_some());
        let mut target = asked;
        let mut waiting = false;
        if let Some(due) = due.filter(|_| target > 0.0) {
            match self.measure(due) {
                Splice::None => {}
                // Cutting a live waveform is a click, so the ramp comes down
                // first and the join happens on the next callback, in
                // silence.
                _ if self.gain > 0.0 => target = 0.0,
                Splice::Skip(frames) => self.drop_front(frames),
                Splice::Wait => waiting = true,
            }
        }
        if waiting {
            self.health.underruns += 1;
            return;
        }
        // Silent and staying silent: nothing is consumed, so a pause holds the
        // sound where it stopped rather than running it out under a still
        // picture, and a flushed ring is dropped here once the ramp that
        // covered it has finished.
        if target == 0.0 && self.gain == 0.0 {
            if self.stale {
                self.clear();
            }
            return;
        }

        let wanted = out.len() / self.channels;
        let mut written = 0;
        while written < wanted && self.frames > 0 {
            if self.gain == 0.0 && target == 0.0 {
                break;
            }
            self.gain = approach(self.gain, target, self.step);
            let (from, to) = (self.head, written * self.channels);
            for channel in 0..self.channels {
                out[to + channel] = self.samples[from + channel] * self.gain;
            }
            self.drop_front(1);
            written += 1;
        }
        // A stopped/reset empty ring is already silent. No old sample remains
        // to fade, so do not leave gain waiting for a sample that a stale ring
        // refuses to accept. Zero gain also lets pause/resume rewind retained
        // PCM. Keep the ordinary running gain when refill can still arrive.
        if self.frames == 0 && target == 0.0 {
            self.gain = 0.0;
            if self.stale {
                self.clear();
            }
        }
        // A callback the ring could not fill while the picture was moving is a
        // hole in the sound, and an empty ring is one however far out its head
        // was: a ring that has run dry behind a splice never reaches the ramp
        // above, so it used to leave `target` at zero and count nothing. That
        // is why the 3.3 s hole of issue #97 measured as 2.4 s, and why its
        // first 0.9 s were invisible to the report altogether.
        //
        // Fading down over sound the ring still holds is not a hole: that is
        // the join before a splice, and the rest of the callback is silence
        // by design.
        if asked > 0.0 && written < wanted && self.frames == 0 {
            self.health.underruns += 1;
        }
    }
}

/// Step `from` towards `to` by at most `step`.
fn approach(from: f32, to: f32, step: f32) -> f32 {
    match to > from {
        true => (from + step).min(to),
        false => (from - step).max(to),
    }
}

/// Output frames to add over `distance` frames to walk the sound back onto the
/// picture, which is `swr_set_compensation`'s pair of arguments.
///
/// Positive is slower: more output frames for the same sound means the media
/// time under the head of the ring advances less per second, which is what a
/// sound running ahead of the picture needs.
pub fn compensation(offset: i64, distance: u32) -> i32 {
    let ratio = (PULL * offset as f64 * 1e-6).clamp(-SLEW, SLEW);
    (ratio * f64::from(distance)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_audio_queue_is_live_read_only_and_does_not_wait() {
        let pipe = Pipe::new(1000, 2, Duration::from_secs(1));
        pipe.write(&[0.25; 200], Duration::from_millis(100));
        assert_eq!(
            pipe.buffer_snapshot().unwrap().0,
            Duration::from_millis(100)
        );
        {
            let buffer = pipe.0.lock().unwrap();
            assert_eq!(buffer.frames, 100, "snapshot cannot consume PCM");
            assert_eq!(
                pipe.buffer_snapshot(),
                None,
                "snapshot cannot wait for callback"
            );
        }
        pipe.invalidate();
        assert_eq!(
            pipe.buffer_snapshot().unwrap().0,
            Duration::ZERO,
            "old seek-lineage PCM is not usable lead"
        );
    }

    const RATE: u32 = 48_000;
    const CHANNELS: usize = 2;

    fn buffer() -> Buffer {
        Buffer::new(RATE, CHANNELS, Duration::from_secs(1))
    }

    fn frames(count: usize, value: f32) -> Vec<f32> {
        vec![value; count * CHANNELS]
    }

    fn at(frames: usize) -> Duration {
        Duration::from_secs_f64(frames as f64 / f64::from(RATE))
    }

    #[test]
    fn buffering_readiness_requires_current_samples_through_the_refill_lead() {
        let pipe = Pipe::new(RATE, CHANNELS, Duration::from_secs(1));
        assert!(!pipe.buffered_through(Duration::ZERO, Duration::ZERO));
        pipe.write(&frames(4_800, 0.5), at(4_800));
        assert!(pipe.buffered_through(at(2_400), at(2_400)));
        assert!(!pipe.buffered_through(at(2_400), at(2_401)));
        pipe.invalidate();
        assert!(!pipe.buffered_through(Duration::ZERO, Duration::ZERO));
        pipe.fill(&mut frames(480, 0.0), None);
        assert!(!pipe.buffered_through(Duration::ZERO, Duration::ZERO));
    }

    #[test]
    fn a_seek_after_audio_runs_dry_allows_fresh_samples_to_refill() {
        let pipe = Pipe::new(RATE, CHANNELS, Duration::from_millis(500));
        pipe.set_volume(0.2);
        let old = pipe.epoch();
        let origin = Duration::from_secs(4);
        pipe.write_in(&old, &frames(480, 0.5), origin + at(480));
        let mut out = frames(512, 0.0);
        pipe.fill(&mut out, Some(origin));
        assert!(out.iter().any(|sample| *sample > 0.0));
        assert_eq!(pipe.room(), Duration::from_millis(500));

        let fresh = pipe.invalidate();
        pipe.fill(&mut out, None);
        assert!(out.iter().all(|sample| *sample == 0.0));
        assert_eq!(pipe.room(), Duration::from_millis(500));
        let held = Duration::from_millis(142_442);
        pipe.write_in(&old, &frames(4_800, 0.9), held + at(4_800));
        assert!(!pipe.buffered_through(held, at(3_200)));
        pipe.write_in(&fresh, &frames(4_800, 0.25), held + at(4_800));
        assert!(pipe.buffered_through(held, at(3_200)));
        pipe.fill(&mut out, Some(held));
        assert!(out[0] > 0.0 && out[0] < 0.25 * 0.2);
        assert_eq!(out[out.len() - 1], 0.25 * 0.2);
        assert_eq!(pipe.health().dropped, 0);
    }

    #[test]
    fn a_seek_fade_finishes_when_its_remaining_samples_run_out() {
        let pipe = Pipe::new(RATE, CHANNELS, Duration::from_millis(500));
        pipe.set_volume(0.2);
        let origin = Duration::from_secs(4);
        pipe.write_in(&pipe.epoch(), &frames(544, 0.5), origin + at(544));
        let mut out = frames(512, 0.0);
        pipe.fill(&mut out, Some(origin));
        pipe.invalidate();
        pipe.fill(&mut out, None);
        assert!(out[0] > 0.0, "retain the fade over available old samples");
        assert!(out[32 * CHANNELS..].iter().all(|sample| *sample == 0.0));
        assert_eq!(pipe.room(), Duration::from_millis(500));
        assert_eq!(pipe.locked().gain, 0.0);
        assert_eq!(pipe.locked().past, 0, "a seek cannot retain old history");
    }

    #[test]
    fn a_pause_after_audio_runs_dry_can_rewind_retained_samples_on_resume() {
        let pipe = Pipe::new(RATE, CHANNELS, Duration::from_millis(500));
        pipe.set_volume(0.2);
        pipe.write_in(&pipe.epoch(), &frames(480, 0.5), at(480));
        let mut out = frames(512, 0.0);
        pipe.fill(&mut out, Some(Duration::ZERO));
        pipe.fill(&mut out, None);
        pipe.fill(&mut out, Some(at(240)));
        assert!(out[0] > 0.0, "resume must reuse the actual retained PCM");
        assert!(out[0] < 0.5 * 0.2, "resume starts with the existing fade");
        assert_eq!(pipe.health().offset, 0);
    }

    #[test]
    fn running_refill_keeps_its_gain_when_a_callback_empties_the_ring() {
        let pipe = Pipe::new(RATE, CHANNELS, Duration::from_millis(500));
        pipe.set_volume(0.2);
        let epoch = pipe.epoch();
        pipe.write_in(&epoch, &frames(480, 0.5), at(480));
        let mut out = frames(480, 0.0);
        pipe.fill(&mut out, Some(Duration::ZERO));
        pipe.write_in(&epoch, &frames(480, 0.5), at(960));
        pipe.fill(&mut out, Some(at(480)));
        assert_eq!(
            out[0],
            0.5 * 0.2,
            "ordinary refill must not restart its fade"
        );
        assert_eq!(pipe.health().underruns, 0);
    }

    #[test]
    fn inaudible_sound_does_not_hold_video_for_an_unconsumed_audio_ring() {
        let pipe = Pipe::new(RATE, CHANNELS, Duration::from_secs(1));
        pipe.set_muted(true);
        assert!(pipe.buffered_through(Duration::from_secs(10), Duration::from_secs(1)));
        pipe.set_muted(false);
        assert!(!pipe.buffered_through(Duration::from_secs(10), Duration::ZERO));
        pipe.set_volume(0.0);
        assert!(pipe.buffered_through(Duration::from_secs(10), Duration::from_secs(1)));
    }

    #[test]
    fn a_full_old_audio_ring_is_retired_only_after_the_buffer_hold_fades() {
        let pipe = Pipe::new(RATE, CHANNELS, Duration::from_secs(1));
        pipe.write(&frames(RATE as usize, 0.5), Duration::from_secs(1));
        pipe.fill(&mut frames(480, 0.0), Some(Duration::ZERO));
        let occupied_room = pipe.room();
        pipe.discard_before_if_silent(Duration::from_secs(2));
        assert_eq!(pipe.room(), occupied_room, "do not cut a live fade");
        pipe.fill(&mut frames(480, 0.0), None);
        pipe.discard_before_if_silent(Duration::from_secs(2));
        assert_eq!(pipe.room(), Duration::from_secs(1));
        assert!(!pipe.buffered_through(Duration::from_secs(2), Duration::ZERO));
    }

    #[test]
    fn resuming_a_hold_can_reuse_samples_already_scheduled_ahead_of_the_clock() {
        let pipe = Pipe::new(RATE, CHANNELS, Duration::from_millis(500));
        // 512-frame periods match the slow-input player's PipeWire request.
        // An ordinary 20 ms lead is below the existing splice threshold.
        // Stopping the UI clock just after a callback adds that callback's
        // remainder and the fade, pushing the next head beyond the threshold.
        let period = at(512);
        let latency = at(1_536);
        let offset = at(960);
        let origin = Instant::now();
        let playing = Reading {
            playing: true,
            position: Duration::ZERO,
            origin: Some(origin),
        };
        let samples: Vec<_> = (0..24_000)
            .flat_map(|index| [index as f32 / 48_000.0; CHANNELS])
            .collect();
        pipe.write(&samples, latency + offset + at(24_000));
        let mut out = frames(512, 0.0);
        for callback in 0..10 {
            pipe.fill(
                &mut out,
                playing.running_at(origin + period * callback + latency),
            );
        }
        assert_eq!(pipe.health().underruns, 0);
        let stopped = Reading {
            playing: false,
            position: playing.position(origin + period * 9 + at(48)),
            origin: Some(origin + period * 9 + at(48)),
        };
        pipe.fill(&mut out, stopped.running_at(origin + period * 10 + latency));
        let resumed = Reading {
            playing: true,
            origin: Some(origin + Duration::from_secs(1)),
            ..stopped
        };
        let due = resumed.running_at(origin + Duration::from_secs(1) + latency);
        pipe.fill(&mut out, due);
        assert!(
            out.iter().any(|sample| *sample > 0.0),
            "resume must emit the retained audio, not a blank callback"
        );
        assert_eq!(
            pipe.health().underruns,
            0,
            "a full ring must not restart with a timestamp-induced audio gap"
        );
        assert!(
            pipe.health().offset.abs() <= 21,
            "restored PCM must match the resumed device PTS to one sample"
        );
        let first_full_gain = out[240 * CHANNELS];
        let expected_index =
            (due.unwrap() - latency - offset).as_secs_f64() * f64::from(RATE) + 240.0;
        assert!((first_full_gain - expected_index as f32 / 48_000.0).abs() <= 1.0 / 48_000.0);
    }

    #[test]
    fn consumed_history_survives_refill_wrapping_the_ring() {
        let mut buffer = Buffer::new(1_000, 1, Duration::from_millis(8));
        let samples: Vec<_> = (0..20).map(|index| index as f32 / 20.0).collect();
        buffer.write(&samples[..8], Duration::from_millis(8));
        buffer.fill(&mut [0.0; 4], Some(Duration::ZERO));
        buffer.fill(&mut [0.0; 4], Some(Duration::from_millis(4)));
        buffer.write(&samples[8..16], Duration::from_millis(16));
        buffer.fill(&mut [0.0; 8], None);
        assert!(matches!(buffer.head_time().as_millis(), 13 | 14));
        buffer.write(&samples[16..20], Duration::from_millis(20));
        let mut out = [0.0; 3];
        buffer.fill(&mut out, Some(Duration::from_millis(9)));
        for (index, sample) in out.into_iter().enumerate() {
            let gain = (index + 1) as f32 * buffer.step;
            assert!((sample - samples[index + 9] * gain).abs() < f32::EPSILON);
        }
        assert_eq!(buffer.head_time(), Duration::from_millis(12));
        assert_eq!(buffer.health.underruns, 0);
        assert_eq!(
            buffer.room(),
            0,
            "restored future lead must throttle refill"
        );
        assert!(buffer.past + buffer.frames <= buffer.capacity() * 2);
    }

    #[test]
    fn unavailable_history_still_reports_a_real_resume_gap() {
        let mut buffer = Buffer::new(1_000, 1, Duration::from_millis(20));
        buffer.write(&[0.5; 20], Duration::from_millis(1_010));
        buffer.fill(&mut [0.0; 5], Some(Duration::from_millis(990)));
        buffer.fill(&mut [0.0; 8], None);
        let mut out = [1.0; 5];
        buffer.fill(&mut out, Some(Duration::from_millis(900)));
        assert_eq!(out, [0.0; 5]);
        assert_eq!(buffer.health.underruns, 1);
        assert_eq!(buffer.head_time(), Duration::from_millis(990));
    }

    #[test]
    fn uninterrupted_audio_does_not_replay_consumed_history_to_correct_drift() {
        let mut buffer = buffer();
        buffer.write(&frames(24_000, 0.5), at(24_000));
        for _ in 0..4 {
            buffer.fill(&mut frames(480, 0.0), Some(Duration::ZERO));
        }
        assert_eq!(buffer.gain, 0.0);
        let head = buffer.head_time();
        assert!(buffer.past > 0);
        let mut out = frames(480, 1.0);
        buffer.fill(&mut out, Some(Duration::ZERO));
        assert!(out.iter().all(|sample| *sample == 0.0));
        assert_eq!(buffer.head_time(), head);
        assert_eq!(buffer.health.underruns, 1);
    }

    #[test]
    fn a_seek_discards_consumed_history_as_well_as_queued_audio() {
        let pipe = Pipe::new(RATE, CHANNELS, Duration::from_millis(500));
        pipe.write(&frames(24_000, 0.5), at(24_000));
        pipe.fill(&mut frames(480, 0.0), Some(Duration::ZERO));
        assert!(pipe.locked().past > 0);
        pipe.invalidate();
        pipe.fill(&mut frames(480, 0.0), None);
        pipe.fill(&mut frames(480, 0.0), None);
        assert_eq!(pipe.locked().past, 0);
        pipe.write(&frames(4_800, 0.25), Duration::from_secs(2) + at(4_800));
        let mut out = frames(480, 1.0);
        pipe.fill(&mut out, Some(Duration::from_secs(1)));
        assert!(out.iter().all(|sample| *sample == 0.0));
        assert_eq!(pipe.health().underruns, 1);
    }

    #[test]
    fn nan_volume_cannot_invert_or_amplify_pipe_output() {
        for initial_volume in [0.0, 0.5, 1.0] {
            let pipe = Pipe::new(RATE, CHANNELS, Duration::from_secs(1));
            pipe.set_volume(initial_volume);
            pipe.write(&frames(960, 1.0), at(960));
            pipe.fill(&mut frames(480, 0.0), Some(Duration::ZERO));

            // A live config update may contain RON's NaN literal. Exercise
            // the public callback boundary without opening an audio device.
            pipe.set_volume(f32::NAN);
            let mut out = frames(480, 0.0);
            pipe.fill(&mut out, Some(at(480)));
            assert!(
                out.iter()
                    .all(|sample| sample.is_finite() && (0.0..=initial_volume).contains(sample)),
                "invalid gain after NaN volume: initial={initial_volume}, first={}, last={}",
                out[0],
                out[out.len() - 1]
            );
            assert_eq!(out[out.len() - 1], 0.0, "invalid volume fades to silence");
            assert_eq!(pipe.health().underruns, 0);
        }
    }

    #[test]
    fn pipe_volume_preserves_existing_finite_and_infinite_clamps() {
        for (volume, expected) in [
            (f32::NEG_INFINITY, 0.0),
            (-1.0, 0.0),
            (0.0, 0.0),
            (0.25, 0.25),
            (1.0, 1.0),
            (2.0, 1.0),
            (f32::INFINITY, 1.0),
        ] {
            let pipe = Pipe::new(RATE, CHANNELS, Duration::from_secs(1));
            pipe.set_volume(volume);
            pipe.write(&frames(480, 1.0), at(480));
            let mut out = frames(480, 0.0);
            pipe.fill(&mut out, Some(Duration::ZERO));
            assert!(out.iter().all(|sample| (0.0..=expected).contains(sample)));
            assert_eq!(out[out.len() - 1], expected, "volume={volume}");
        }
    }

    /// The whole clock slave rests on this one line of arithmetic: what the
    /// device is about to play is the write head less everything queued behind
    /// it.
    #[test]
    fn the_head_of_the_ring_is_the_write_head_less_what_is_queued() {
        let mut buffer = buffer();
        buffer.write(&frames(4800, 0.5), at(9600));
        assert_eq!(buffer.head_time(), at(4800));

        buffer.fill(&mut frames(2400, 0.0), Some(at(4800)));
        assert_eq!(buffer.head_time(), at(7200));
    }

    /// The depth the device saw, which is what tells a ring that ran dry from
    /// one holding sound whose moment has passed (issue #97: the second).
    #[test]
    fn the_depth_the_device_saw_is_reported() {
        let mut buffer = buffer();
        buffer.fill(&mut frames(480, 0.0), Some(Duration::ZERO));
        assert_eq!(buffer.health.queued, 0);

        buffer.write(&frames(4800, 0.5), at(4800));
        buffer.fill(&mut frames(480, 0.0), Some(Duration::ZERO));
        assert_eq!(buffer.health.queued, 100_000);
    }

    /// Sound whose moment has passed is dropped, not played late: a scrub
    /// backwards leaves a ring full of it.
    #[test]
    fn sound_left_behind_the_picture_is_dropped_to_it() {
        let mut buffer = buffer();
        buffer.write(&frames(24_000, 0.5), at(24_000));
        assert_eq!(buffer.head_time(), Duration::ZERO);

        buffer.fill(&mut frames(480, 0.0), Some(at(4800)));

        // The 100 ms that had already been missed is gone, and what is left
        // starts where the picture is.
        assert_eq!(buffer.frames, 24_000 - 4800 - 480);
        assert_eq!(buffer.health.offset, -100_000);
    }

    /// Sound whose moment has not come waits under silence, which is the gap
    /// it is waiting for. Playing it early would run ahead of the picture.
    #[test]
    fn sound_ahead_of_the_picture_waits_under_silence() {
        let mut buffer = buffer();
        buffer.write(&frames(4800, 0.5), at(9600));

        let mut out = frames(480, 0.0);
        buffer.fill(&mut out, Some(Duration::ZERO));

        assert!(out.iter().all(|sample| *sample == 0.0));
        assert_eq!(buffer.frames, 4800);
        assert_eq!(buffer.head_time(), at(4800));
        assert_eq!(buffer.health.underruns, 1);
    }

    /// Drift is walked in by the resampler, not spliced: a join inside
    /// [`SPLICE`] would put a click into sound nobody could hear was late.
    #[test]
    fn an_error_smaller_than_a_splice_is_left_to_the_resampler() {
        let mut buffer = buffer();
        buffer.write(&frames(24_000, 0.5), at(48_000));
        let due = at(24_000) - Duration::from_millis(20);

        buffer.fill(&mut frames(480, 0.0), Some(due));

        assert_eq!(buffer.health.offset, 20_000);
        assert_eq!(buffer.frames, 24_000 - 480);
    }

    /// The correction has to pull the right way, settle where two crystals
    /// differ, and never ask for a ratio anyone could hear.
    #[test]
    fn the_drift_correction_slows_a_sound_that_runs_ahead() {
        // 10 ms ahead: half of it over the next second, capped by the slew.
        assert_eq!(compensation(10_000, RATE), 240);
        assert_eq!(compensation(-10_000, RATE), -240);
        // Where crystals 50 ppm apart settle: a tenth of a millisecond.
        assert_eq!(compensation(100, RATE), 2);
        assert_eq!(compensation(0, RATE), 0);
        // And the cap holds however far out the sound is.
        assert_eq!(compensation(10_000_000, RATE), 240);
        assert_eq!(compensation(-10_000_000, RATE), -240);
    }

    /// A pause has to stop the sound without a step in it, and without eating
    /// the sound it stopped on: resuming plays what was next.
    #[test]
    fn a_pause_fades_out_and_holds_what_it_did_not_play() {
        let mut buffer = buffer();
        buffer.write(&frames(24_000, 1.0), at(24_000));
        buffer.fill(&mut frames(480, 0.0), Some(Duration::ZERO));
        assert_eq!(buffer.gain, 1.0);
        let playing = buffer.frames;

        let mut out = frames(4800, 0.0);
        buffer.fill(&mut out, None);

        // The ramp is a ramp: no two neighbouring frames step by more than one
        // frame's worth of gain.
        let mut last = 1.0;
        for frame in out.chunks(CHANNELS) {
            assert!(
                (last - frame[0]).abs() <= buffer.step + f32::EPSILON,
                "{last} to {}",
                frame[0]
            );
            last = frame[0];
        }
        assert_eq!(buffer.gain, 0.0);
        // Only the fade was consumed, not the whole callback.
        let faded = (FADE.as_secs_f64() * f64::from(RATE)).ceil() as usize;
        assert!(playing - buffer.frames <= faded + 1);

        // And a second silent callback consumes nothing at all.
        let held = buffer.frames;
        buffer.fill(&mut frames(4800, 0.0), None);
        assert_eq!(buffer.frames, held);
    }

    /// The epoch discipline the pictures already have (issue #5), for the
    /// sound: nothing decoded before a seek may be heard after it.
    #[test]
    fn a_flush_drops_every_sample_from_before_the_seek() {
        let mut buffer = buffer();
        buffer.write(&frames(24_000, 1.0), at(24_000));
        buffer.fill(&mut frames(480, 0.0), Some(Duration::ZERO));

        buffer.flush();
        // Nothing from before the seek is taken while it is being thrown
        // away, and the fade covers what is already on its way out.
        buffer.write(&frames(480, 1.0), at(96_000));
        assert_eq!(buffer.through, at(24_000));
        let mut out = frames(4800, 0.0);
        buffer.fill(&mut out, Some(Duration::ZERO));
        assert!(out.iter().any(|sample| *sample != 0.0), "cut, not faded");

        buffer.fill(&mut frames(480, 0.0), Some(Duration::ZERO));
        assert_eq!(buffer.frames, 0);
        assert!(!buffer.stale);

        // And what comes after it lands on the picture rather than behind it.
        buffer.write(&frames(4800, 1.0), at(96_000 + 4800));
        buffer.fill(&mut frames(480, 0.0), Some(at(96_000)));
        assert_eq!(buffer.health.offset, 0);
        assert_eq!(buffer.frames, 4800 - 480);
    }

    /// The report's underrun count has to mean "the sound broke", so neither a
    /// paused player nor a fading one may raise it.
    #[test]
    fn only_a_moving_picture_with_no_sound_behind_it_is_an_underrun() {
        let mut buffer = buffer();
        buffer.fill(&mut frames(480, 0.0), None);
        assert_eq!(buffer.health.underruns, 0);

        buffer.fill(&mut frames(480, 0.0), Some(Duration::ZERO));
        assert_eq!(buffer.health.underruns, 1);

        buffer.write(&frames(4800, 0.5), at(4800));
        buffer.fill(&mut frames(480, 0.0), Some(Duration::ZERO));
        assert_eq!(buffer.health.underruns, 1);
    }

    /// The hole of issue #97: a ring that ran dry while its head was a long
    /// way behind the picture. The splice lowers the gain over sound that is
    /// not there, so the ramp is never reached, and every callback of those
    /// three seconds used to count nothing at all.
    #[test]
    fn a_dry_ring_behind_the_picture_is_counted_every_callback() {
        let mut buffer = buffer();
        buffer.write(&frames(4800, 0.5), at(4800));
        buffer.fill(&mut frames(4800, 0.0), Some(Duration::ZERO));
        assert_eq!((buffer.frames, buffer.health.underruns), (0, 0));

        // A second of picture later, with nothing having arrived since.
        for beat in 1..=3 {
            buffer.fill(&mut frames(480, 0.0), Some(at(48_000)));
            assert_eq!(buffer.health.underruns, beat);
        }
    }

    /// Mute is silence, not a stopped clock: the sound keeps running under it,
    /// so unmuting lands where the picture is rather than where it was.
    #[test]
    fn muting_holds_the_sound_against_the_picture() {
        let mut buffer = buffer();
        buffer.write(&frames(24_000, 1.0), at(24_000));
        buffer.muted = true;
        buffer.fill(&mut frames(4800, 0.0), Some(Duration::ZERO));
        assert_eq!(buffer.gain, 0.0);
        assert_eq!(buffer.frames, 24_000);

        // Quarter of a second of picture later, unmuting plays the sound from
        // there and not from where mute started.
        buffer.muted = false;
        buffer.fill(&mut frames(480, 0.0), Some(at(12_000)));
        assert_eq!(buffer.frames, 24_000 - 12_000 - 480);
    }

    /// A ring that fills has to lose the chunk it cannot take, and say so,
    /// rather than write half of it and leave the media time lying.
    #[test]
    fn a_full_ring_drops_a_whole_chunk_and_counts_it() {
        let mut buffer = buffer();
        buffer.write(&frames(RATE as usize, 0.5), at(48_000));
        assert_eq!(buffer.room(), 0);

        buffer.write(&frames(1024, 0.5), at(49_024));
        assert_eq!(buffer.health.dropped, 1);
        assert_eq!(buffer.frames, RATE as usize);
        assert_eq!(buffer.through, at(49_024));
    }

    /// The clock reading is the presenter's arithmetic, and the sound is
    /// silent for exactly the states the picture does not move in.
    #[test]
    fn the_clock_reads_the_same_as_the_presenter() {
        let now = Instant::now();
        let running = Reading {
            playing: true,
            position: Duration::from_secs(10),
            origin: Some(now),
        };
        assert_eq!(
            running.running_at(now + Duration::from_secs(1)),
            Some(Duration::from_secs(11))
        );

        let paused = Reading {
            playing: false,
            ..running
        };
        assert_eq!(paused.running_at(now + Duration::from_secs(1)), None);
        assert_eq!(
            paused.position(now + Duration::from_secs(1)),
            Duration::from_secs(10)
        );

        // Playing, and not yet anchored to a first frame.
        let waiting = Reading {
            origin: None,
            ..running
        };
        assert_eq!(waiting.running_at(now), None);
    }

    /// The callback keeps the last reading rather than reading a busy lock as
    /// a stopped clock, which would fade the sound out on a mutex.
    #[test]
    fn a_busy_clock_reads_as_it_last_did() {
        let beat = Beat::default();
        let was = Reading {
            playing: true,
            position: Duration::from_secs(3),
            origin: Some(Instant::now()),
        };
        beat.publish(was);
        assert_eq!(beat.read(Reading::default()), was);

        let held = beat.0.lock().unwrap();
        assert_eq!(beat.read(was), was);
        drop(held);
    }
}
