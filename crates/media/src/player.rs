//! The presentation clock: what turns decoded frames into moving pictures.
//!
//! A [`Reader`] runs on its own thread, staying a few frames ahead of the
//! picture, and the caller asks [`Player::pump`] what should be on screen
//! *now*. "Now" is a monotonic [`Instant`] the caller supplies, not a count
//! of ticks: 29.97 fps content divides evenly into no refresh rate anyone
//! ships, so a frame is due at a time rather than after N refreshes.
//! [`Player::next_due`] is the other half of that: it says when the next
//! frame is due, so the caller can sleep until then instead of polling, and
//! the picture lands on the refresh nearest its due time with no error
//! carried forward. Counting refreshes per frame is what makes 29.97
//! judder.
//!
//! Which clock is authoritative is not settled. The trailer says
//! `pts_type = 2` (`VideoPtsEexposureFile`), which suggests the per-frame
//! exposure records, not container PTS, are the camera's real frame clock
//! (issue #8, whose Studio-diff harness is what could tell). This is pacing,
//! not gyro alignment, so container PTS is what runs here; if #8 finds
//! otherwise, [`Frames::timestamp`] is the value that changes and nothing
//! above this module needs to know.

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, channel, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

use super::audio::{Audio, Beat, Reading};
use super::sound::Sound;
use super::{Accuracy, Cue, Fallible, Frames, Read, Reader, Size, Timing};

/// Pairs the decode thread may have ready and waiting.
const QUEUED: usize = 2;

/// Frames each lane decodes past a surface before it is mapped
/// ([`Reader::lookahead`]). Measured: 2.19x realtime at 0, 2.46x at 2, and
/// 2.47x at 4, so 2 takes the whole win. With the two queued pairs, the
/// pair on screen, the one peeked and the three the renderer retains, the
/// engine holds 9 of the 20 surfaces in a decoder's pool.
const LOOKAHEAD: usize = 2;

/// What playback did, for the report the app prints and the instrument
/// measures. Every count here is a defect except `presented`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Calls to [`Player::pump`], which is one per redraw. Reported
    /// because the presented rate alone cannot tell a player that wakes
    /// once per frame from one that wakes twice and shows the same picture
    /// again, and the two cost very different amounts of battery.
    pub redraws: u64,
    /// Frames that reached the screen.
    pub presented: u64,
    /// Frames decoded but never shown, because their moment had passed
    /// before the picture next changed. Stutter, in other words.
    pub dropped: u64,
    /// Redraws where the next frame was due and the decoder had not
    /// produced it yet. The picture froze for a beat.
    pub starved: u64,
    /// Worst delay between a frame's due time and the pump that showed it.
    /// A caller redrawing at vsync cannot beat one refresh here.
    pub worst_late: Duration,
    /// What the sound did, and `None` for a file with no sound in it. The
    /// report says nothing about sound rather than reporting a clean zero,
    /// because a silent file and a working one are not the same news.
    pub audio: Option<Audio>,
}

impl Stats {
    /// What happened between an earlier reading and this one, for a report
    /// covering a window rather than the whole run. `worst_late` stays the
    /// running worst: a maximum cannot be subtracted.
    pub fn since(self, earlier: Self) -> Self {
        Self {
            redraws: self.redraws.saturating_sub(earlier.redraws),
            presented: self.presented.saturating_sub(earlier.presented),
            dropped: self.dropped.saturating_sub(earlier.dropped),
            starved: self.starved.saturating_sub(earlier.starved),
            worst_late: self.worst_late,
            audio: self
                .audio
                .map(|audio| audio.since(earlier.audio.unwrap_or_default())),
        }
    }

    /// One line, for a run of `over`.
    pub fn report(&self, over: Duration) -> String {
        let per_second = |count: u64| count as f64 / over.as_secs_f64().max(f64::EPSILON);
        let line = format!(
            "{:.2} fps presented in {:.1} redraws/s, {} dropped, {} starved, \
             worst {:.1} ms late",
            per_second(self.presented),
            per_second(self.redraws),
            self.dropped,
            self.starved,
            self.worst_late.as_secs_f64() * 1000.0,
        );
        match &self.audio {
            Some(audio) => format!("{line}, {}", audio.report()),
            None => line,
        }
    }
}

/// What the decode thread sends back.
enum Note {
    /// A pair, and which seek it belongs to. A seek leaves frames from before
    /// it in the channel, and the epoch is how they are told apart.
    Frames(u64, Frames),
    /// The file has been read to the end. The thread stays alive, because a
    /// seek can send it back into the file.
    Ended(u64),
    Failed(Box<dyn std::error::Error + Send + Sync>),
}

/// What the decode thread is told to do. There is one thing: everything else
/// it does, it does by reading forward.
enum Command {
    Seek {
        epoch: u64,
        to: Cue,
        accuracy: Accuracy,
    },
}

/// A file, decoding on its own thread, and the clock that decides which of
/// its frames belongs on screen.
pub struct Player {
    notes: Receiver<Note>,
    commands: Sender<Command>,
    presenter: Presenter,
    /// The open output device, and `None` for a file with no sound or a box
    /// with no working one. A player that will not show a video because it
    /// could not open a speaker is worse than one that plays it silently.
    sound: Option<Sound>,
    timing: Timing,
    size: Size,
    lenses: usize,
    failure: Option<Box<dyn std::error::Error + Send + Sync>>,
    ended: bool,
    /// Which frames may go on screen, and which seek is still owed one.
    epochs: Epochs,
}

/// What the picture is waiting for, which is what decides which frames may
/// take the screen while it waits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Wait {
    /// Nothing. The clock moves the picture, one frame at its due time.
    #[default]
    Nothing,
    /// A seek's own frame. Only a position newer than the one on screen may
    /// take it: the pilot has asked to leave the one they are looking at, and
    /// the reader is still handing over frames of it.
    Position,
    /// The next frame of the position on screen. Stepping one frame forward
    /// asks for exactly that and sends no seek at all, because the reader is
    /// already sitting on it ([`Player::step`]).
    Frame,
}

/// Which frames may go on screen, and which seek is still owed one.
///
/// Those are two questions, and a hand faster than a landing is where they
/// come apart (issue #55). Every seek bumps `asked` and every frame carries
/// the epoch it was decoded under, so a drag that outruns the decoder has
/// several seeks in flight and the pictures coming out of them are tagged
/// with epochs the pilot has already dragged past. Showing only the newest
/// seek's frames therefore shows nothing at all: at 60 positions a second
/// not one picture reached the screen for the length of the drag.
///
/// Frames arrive in the order they were asked for, so a picture from a seek
/// the pilot has dragged past is still a picture of somewhere they have been
/// since the one on screen, and putting it up can only move the picture
/// forwards.
///
/// The wait is the other question and it is answered separately, because it
/// is what keeps a paused window redrawing: it ends on the newest seek's own
/// frame rather than on whatever picture went up last. Ending it on an
/// intermediate picture stops the redraw loop while the release's exact frame
/// is still being decoded, and leaves the pilot looking at the keyframe their
/// finger passed over instead of the frame they let go on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Epochs {
    /// Bumped by every seek.
    asked: u64,
    /// What the picture on screen was decoded under.
    shown: u64,
    wait: Wait,
}

impl Epochs {
    /// A seek, and the epoch the frames it produces will carry.
    fn ask(&mut self) -> u64 {
        self.asked += 1;
        self.wait = Wait::Position;
        self.asked
    }

    /// A frame is owed under the epoch already in force.
    fn owe(&mut self) {
        self.wait = Wait::Frame;
    }

    /// Nothing is coming: the decode thread has gone away.
    fn give_up(&mut self) {
        self.wait = Wait::Nothing;
    }

    /// Whether a frame decoded under `tag` may take the screen.
    fn accepts(&self, tag: u64) -> bool {
        match self.wait {
            Wait::Position => tag > self.shown,
            Wait::Nothing | Wait::Frame => tag >= self.shown,
        }
    }

    /// Whether `tag` is the newest seek: the only one whose frame ends the
    /// wait, and the only one whose end of file is this player's end of file.
    fn is_newest(&self, tag: u64) -> bool {
        tag == self.asked
    }

    /// A frame decoded under `tag` is going on screen.
    fn showed(&mut self, tag: u64) {
        self.shown = tag;
        if self.is_newest(tag) {
            self.wait = Wait::Nothing;
        }
    }

    fn is_seeking(&self) -> bool {
        self.wait != Wait::Nothing
    }
}

impl Player {
    /// Opens the file and starts decoding. Returns as soon as the container
    /// is parsed: the first frame arrives on the thread, so a big file does
    /// not hold the window shut.
    pub fn open(path: &Path) -> Fallible<Self> {
        let mut reader = Reader::open(path)?.lookahead(LOOKAHEAD);
        let (timing, size, lenses) = (reader.timing(), reader.size(), reader.lenses());
        let beat = Arc::new(Beat::default());
        let sound = reader
            .sound_rate()
            .and_then(|rate| match Sound::open(&beat, rate) {
                Ok(sound) => Some(sound),
                Err(e) => {
                    eprintln!("kyerag: playing silently: {e}");
                    None
                }
            });
        if let Some(sound) = &sound {
            reader = reader.listen(sound)?;
        }
        let (sender, notes) = sync_channel(QUEUED);
        // Unbounded, because a drag asks for a position per pointer move and
        // the player must never block on handing one over. The thread throws
        // away everything but the newest before each read.
        let (commands, orders) = channel();
        thread::Builder::new()
            .name("kyerag-decode".to_owned())
            .spawn(move || decode_ahead(reader, &sender, &orders))?;

        Ok(Self {
            notes,
            commands,
            presenter: Presenter::new(timing.interval(), beat),
            sound,
            timing,
            size,
            lenses,
            failure: None,
            ended: false,
            epochs: Epochs::default(),
        })
    }

    pub fn timing(&self) -> Timing {
        self.timing
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn lenses(&self) -> usize {
        self.lenses
    }

    pub fn is_playing(&self) -> bool {
        self.presenter.clock.is_playing()
    }

    /// Whether this file has a sound track that a device took. `false` is
    /// either a file with no sound or a box with no working output, and the
    /// pictures play the same either way.
    pub fn has_sound(&self) -> bool {
        self.sound.is_some()
    }

    /// Loudness, 0 to 1. Applied per sample in the device callback, so it
    /// takes effect within one buffer and ramps rather than steps.
    pub fn set_volume(&self, volume: f32) {
        if let Some(sound) = &self.sound {
            sound.pipe().set_volume(volume);
        }
    }

    /// Silence without stopping. The sound keeps running under a mute, so
    /// unmuting lands where the picture is rather than where it was.
    pub fn set_muted(&self, muted: bool) {
        if let Some(sound) = &self.sound {
            sound.pipe().set_muted(muted);
        }
    }

    /// True once the file has been read to the end and every frame of it
    /// has been shown. The last frame stays on screen; there is nothing
    /// left to pace.
    pub fn is_ended(&self) -> bool {
        self.ended && self.presenter.peeked.is_none()
    }

    pub fn position(&self, now: Instant) -> Duration {
        self.presenter.clock.position(now)
    }

    /// The frame on screen, counting from the first of the file.
    pub fn index(&self) -> Option<u64> {
        Some(self.presenter.current.as_ref()?.index)
    }

    /// A seek has been asked for and the frame it asked for is not on screen
    /// yet. A paused caller has to keep redrawing while this is true, because
    /// there is no clock running to tell it the picture will change.
    ///
    /// A drag can put pictures on screen while this stays true: they are
    /// landings of seeks the pilot has already dragged past, and the seek
    /// under the finger is still owed its own.
    pub fn is_seeking(&self) -> bool {
        self.epochs.is_seeking()
    }

    pub fn stats(&self) -> Stats {
        Stats {
            audio: self.sound.as_ref().map(|sound| sound.pipe().health()),
            ..self.presenter.stats
        }
    }

    /// When the next frame is due, for a caller that would rather sleep than
    /// poll. `None` while paused, and before the clock has been anchored to
    /// a first frame: neither has a due time yet.
    pub fn next_due(&self) -> Option<Instant> {
        self.presenter.next_due()
    }

    pub fn play(&mut self) {
        self.presenter.clock.play();
    }

    pub fn pause(&mut self, now: Instant) {
        self.presenter.clock.pause(now);
    }

    pub fn toggle(&mut self, now: Instant) {
        match self.is_playing() {
            true => self.pause(now),
            false => self.play(),
        }
    }

    /// Move the picture to `to`.
    ///
    /// [`Accuracy::Keyframe`] is what a slider being dragged asks for and
    /// [`Accuracy::Exact`] is what letting go of it does (issue #5). Both
    /// return immediately: the seek happens on the decode thread, and
    /// [`Player::is_seeking`] is true until its first frame arrives.
    pub fn seek(&mut self, to: Cue, accuracy: Accuracy) {
        let epoch = self.epochs.ask();
        self.ended = false;
        self.hush();
        self.presenter.reseek(to.time(self.timing));
        let command = Command::Seek {
            epoch,
            to,
            accuracy,
        };
        if self.commands.send(command).is_err() {
            self.epochs.give_up();
            return;
        }
        // Frames decoded before the seek are still in the channel, and the
        // thread may be blocked handing over one more; the epoch is what
        // drops them, and emptying the channel here is what lets the thread
        // reach the command.
        while self.notes.try_recv().is_ok() {}
    }

    /// One frame forward or back, which is what `.` and `,` do.
    ///
    /// Forward by one needs no seek at all: the reader is already sitting on
    /// the next pair, so this only lets the clock take it. Every other step
    /// is a seek, because the frame wanted is behind the decoder and HEVC
    /// cannot be read backwards.
    pub fn step(&mut self, now: Instant, frames: i64) {
        self.pause(now);
        let Some(index) = self.index() else {
            return;
        };
        let target = index.saturating_add_signed(frames).min(self.last_index());
        match frames == 1 && target == index + 1 {
            true => {
                self.hush();
                self.presenter.reseek(self.timing.time_of(target));
                self.epochs.owe();
            }
            false => self.seek(Cue::Index(target), Accuracy::Exact),
        }
    }

    /// Throw away sound decoded before a jump, here on the shell's thread
    /// rather than waiting for the decode thread to reach the seek: that
    /// thread can be blocked handing over a frame, and every millisecond it
    /// waits is a millisecond of the old position still playing.
    fn hush(&self) {
        if let Some(sound) = &self.sound {
            sound.pipe().flush();
        }
    }

    /// The last frame of the file, or 0 for a container that does not say how
    /// many it has.
    fn last_index(&self) -> u64 {
        self.timing.frames.saturating_sub(1)
    }

    /// The frame that belongs on screen at `now`, or `None` when the picture
    /// must not change. Call it on every redraw; it is the whole clock.
    pub fn pump(&mut self, now: Instant) -> Fallible<Option<Arc<Frames>>> {
        let (notes, failure, ended) = (&self.notes, &mut self.failure, &mut self.ended);
        let epochs = &mut self.epochs;
        let owed = epochs.is_seeking();
        let shown = self.presenter.advance(now, owed, || {
            loop {
                match notes.try_recv() {
                    Ok(Note::Frames(tag, frames)) if epochs.accepts(tag) => {
                        epochs.showed(tag);
                        return Some(frames);
                    }
                    // Older than the picture on screen: decoded before it, so
                    // showing it would run the picture backwards.
                    Ok(Note::Frames(..)) => continue,
                    Ok(Note::Ended(tag)) => {
                        *ended = epochs.is_newest(tag);
                        return None;
                    }
                    Ok(Note::Failed(e)) => {
                        *failure = Some(e);
                        return None;
                    }
                    Err(TryRecvError::Empty) => return None,
                    Err(TryRecvError::Disconnected) => {
                        *ended = true;
                        return None;
                    }
                }
            }
        });
        match self.failure.take() {
            Some(e) => Err(e),
            None => Ok(shown),
        }
    }
}

/// What [`decode_ahead`] needs of a [`Reader`]. A trait because the loop
/// below is the whole of newest-command-wins, preemption and epoch tagging,
/// and testing those against a real reader would need a VA-API device and
/// 38 GB of footage.
trait Source {
    fn seek(&mut self, to: Cue, accuracy: Accuracy) -> Fallible<()>;
    fn read_until(&mut self, interrupted: &mut dyn FnMut() -> bool) -> Fallible<Read>;
}

impl Source for Reader {
    fn seek(&mut self, to: Cue, accuracy: Accuracy) -> Fallible<()> {
        Reader::seek(self, to, accuracy)
    }

    fn read_until(&mut self, interrupted: &mut dyn FnMut() -> bool) -> Fallible<Read> {
        Reader::read_until(self, interrupted)
    }
}

/// Decodes ahead of the picture until the player goes away. The bounded
/// channel is the throttle: a full one blocks here, so a paused player
/// stops decoding after `QUEUED` pairs instead of eating the file.
fn decode_ahead(mut reader: impl Source, notes: &SyncSender<Note>, commands: &Receiver<Command>) {
    let mut epoch = 0;
    let mut ended = false;
    // Taken off the channel by the interrupt below and not acted on yet.
    let mut held: Option<Command> = None;
    loop {
        let mut order = held.take();
        // At the end of the file there is nothing to read and nothing to do
        // but wait for a seek, so the thread blocks rather than spinning.
        if order.is_none() && ended {
            match commands.recv() {
                Ok(first) => order = Some(first),
                Err(_) => return,
            }
        }
        // Only the newest command is still wanted. A drag asks for a position
        // per pointer move, and serving the ones behind the last would spend
        // a keyframe decode each on pictures nobody waited to see.
        loop {
            match commands.try_recv() {
                Ok(newer) => order = Some(newer),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        // Set while this read is the landing the newest command asked for.
        let landing = order.is_some();
        if let Some(Command::Seek {
            epoch: to,
            to: cue,
            accuracy,
        }) = order
        {
            epoch = to;
            ended = false;
            if let Err(e) = reader.seek(cue, accuracy) {
                let _ = notes.send(Note::Failed(e));
                return;
            }
        }

        // A newer command takes the thread off the lookahead refill, which is
        // three pair decodes for a position the pilot may already have
        // dragged past: 38 ms of the 59 ms a scrub update used to cost.
        //
        // A landing is exempt and always finishes. Positions arrive faster
        // than pictures come out of them (10 to 12 a second against 20 to 60
        // asked for), so a rule that gave up whatever was newest would give
        // up every landing, and a fast drag would show nothing at all.
        let read = reader.read_until(&mut || match landing {
            true => false,
            false => match commands.try_recv() {
                Ok(newer) => {
                    held = Some(newer);
                    true
                }
                Err(TryRecvError::Empty) => false,
                // Nobody is waiting for this any more. Stopping here reaches
                // the drain above, which returns.
                Err(TryRecvError::Disconnected) => true,
            },
        });
        let note = match read {
            Ok(Read::Frames(frames)) => Note::Frames(epoch, frames),
            // Overtaken. The lanes keep what they decoded; the seek that
            // comes of `held` is what clears it.
            Ok(Read::Interrupted) => continue,
            Ok(Read::Ended) => {
                ended = true;
                Note::Ended(epoch)
            }
            Err(e) => Note::Failed(e),
        };
        let failed = matches!(note, Note::Failed(_));
        if notes.send(note).is_err() || failed {
            return;
        }
    }
}

/// The clock and the frame it is showing. Split out from [`Player`] because
/// this half has no decoder in it and can therefore be tested: every pacing
/// rule the engine has is here.
struct Presenter {
    clock: Clock,
    interval: Duration,
    current: Option<Arc<Frames>>,
    /// Pulled from the queue, not due yet.
    peeked: Option<Frames>,
    stats: Stats,
}

impl Presenter {
    fn new(interval: Duration, beat: Arc<Beat>) -> Self {
        Self {
            clock: Clock::new(beat),
            interval,
            current: None,
            peeked: None,
            stats: Stats::default(),
        }
    }

    /// The timestamp of the frame after the one on screen: the queued frame
    /// if there is one, and otherwise where the container's rate says it
    /// will be.
    fn next_due(&self) -> Option<Instant> {
        let current = self.current.as_ref()?;
        let next = self
            .peeked
            .as_ref()
            .map_or(current.timestamp + self.interval, |frames| frames.timestamp);
        self.clock.reaches(next)
    }

    /// `owed` is a seek waiting for its picture: the first frame this takes
    /// is that seek's landing, and a landing goes up wherever the clock is
    /// rather than when the clock reaches it.
    fn advance(
        &mut self,
        now: Instant,
        mut owed: bool,
        mut next: impl FnMut() -> Option<Frames>,
    ) -> Option<Arc<Frames>> {
        self.stats.redraws += 1;
        if self.is_frozen(owed) {
            return None;
        }
        let mut shown = None;

        while let Some(frames) = self.peeked.take().or_else(&mut next) {
            // The landing is the new position, so the clock moves to it. What
            // follows it in the same pump is ordinary playback and is paced.
            if owed {
                owed = false;
                self.reseek(frames.timestamp);
            }
            if !self.claim(now, &frames) {
                self.peeked = Some(frames);
                break;
            }
            // Replacing a frame this pump already took means its whole
            // moment passed between two redraws: it is a dropped frame, not
            // a fast one.
            if shown.replace(frames).is_some() {
                self.stats.dropped += 1;
            }
            // Paused, one frame is a picture rather than playback.
            if !self.clock.is_playing() {
                break;
            }
        }

        match shown {
            Some(frames) => Some(self.present(now, frames)),
            None => {
                self.note_starvation(now);
                None
            }
        }
    }

    /// Paused with a picture on screen, nothing may change. Paused with no
    /// picture yet is the file that was just opened, or one that has just
    /// been seeked: it gets one frame.
    ///
    /// A seek still owed a picture gets one however many pictures have
    /// already gone up, which is what lets a drag keep moving: it shows a
    /// landing per redraw, and the release's exact frame is the last of them.
    fn is_frozen(&self, owed: bool) -> bool {
        !owed && !self.clock.is_playing() && self.current.is_some()
    }

    /// Throw away the picture and put the clock where the seek is going. The
    /// frame that arrives next anchors it again, at its own timestamp: a
    /// keyframe seek lands before what it was asked for, and the clock has to
    /// say where the picture really is rather than where it was aimed.
    fn reseek(&mut self, to: Duration) {
        self.current = None;
        self.peeked = None;
        self.clock.seek(to);
    }

    /// Whether this frame's moment has come. Also anchors the clock when
    /// nothing has anchored it yet: the first frame of a start or a resume
    /// is due on arrival, because charging the wait for it against a clock
    /// that started earlier would drop every frame the decoder spent
    /// warming up.
    fn claim(&mut self, now: Instant, frames: &Frames) -> bool {
        if self.clock.is_anchored() {
            return frames.timestamp <= self.clock.position(now);
        }
        self.clock.anchor(now, frames.timestamp);
        true
    }

    fn present(&mut self, now: Instant, frames: Frames) -> Arc<Frames> {
        let late = self.clock.position(now).saturating_sub(frames.timestamp);
        self.stats.worst_late = self.stats.worst_late.max(late);
        self.stats.presented += 1;
        let frames = Arc::new(frames);
        self.current = Some(frames.clone());
        frames
    }

    /// Nothing was shown. That is only a fault if a frame was owed: between
    /// two frames the picture is meant to stand still.
    fn note_starvation(&mut self, now: Instant) {
        if !self.clock.is_playing() || !self.clock.is_anchored() {
            return;
        }
        let Some(current) = &self.current else {
            return;
        };
        if current.timestamp + self.interval <= self.clock.position(now) {
            self.stats.starved += 1;
        }
    }
}

/// Media position against the monotonic clock.
///
/// `position` is where the last anchor put us and `origin` is when that
/// happened, so playing position is `position + (now - origin)` and paused
/// position is `position`. Anchoring happens on the frame that starts or
/// resumes playback, never on every frame: a clock re-anchored per frame
/// cannot measure its own drift, and drift is the thing worth measuring.
///
/// Every move is published to a [`Beat`], because the sound follows this
/// clock from the audio device's own thread (issue #13). Publishing rather
/// than sharing keeps the arithmetic here, where the pictures read it.
#[derive(Debug)]
struct Clock {
    reading: Reading,
    beat: Arc<Beat>,
}

impl Clock {
    fn new(beat: Arc<Beat>) -> Self {
        Self {
            reading: Reading::default(),
            beat,
        }
    }

    fn position(&self, now: Instant) -> Duration {
        self.reading.position(now)
    }

    fn is_playing(&self) -> bool {
        self.reading.playing
    }

    fn is_anchored(&self) -> bool {
        self.reading.origin.is_some()
    }

    /// The instant this clock will read `target`, if it is running towards
    /// it. A due time, in other words, and the reason the caller can sleep
    /// instead of polling.
    fn reaches(&self, target: Duration) -> Option<Instant> {
        let origin = self.reading.origin?;
        match self.reading.playing {
            true => Some(origin + target.saturating_sub(self.reading.position)),
            false => None,
        }
    }

    fn play(&mut self) {
        self.moved(Reading {
            playing: true,
            origin: None,
            ..self.reading
        });
    }

    fn pause(&mut self, now: Instant) {
        self.moved(Reading {
            playing: false,
            position: self.position(now),
            origin: None,
        });
    }

    fn anchor(&mut self, now: Instant, at: Duration) {
        self.moved(Reading {
            position: at,
            origin: Some(now),
            ..self.reading
        });
    }

    /// Where the clock reads until the frame that was seeked to arrives.
    /// Unanchoring is what makes that frame anchor it, whenever it comes.
    fn seek(&mut self, to: Duration) {
        self.moved(Reading {
            position: to,
            origin: None,
            ..self.reading
        });
    }

    fn moved(&mut self, reading: Reading) {
        self.reading = reading;
        self.beat.publish(reading);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::Size;

    const NTSC: Duration = Duration::from_nanos(33_366_666);
    const HZ_60: Duration = Duration::from_nanos(16_666_666);

    /// A frame with no lenses in it: everything the clock decides is decided
    /// from the timestamp, so the pixels are not needed to test the pacing.
    fn frame(index: u64) -> Frames {
        Frames {
            index,
            timestamp: NTSC * index as u32,
            lenses: Vec::new(),
            size: Size::new(3840, 3840),
        }
    }

    /// A decoder that always has the next frame ready.
    fn feed(next: &mut u64) -> impl FnMut() -> Option<Frames> + '_ {
        move || {
            let frames = frame(*next);
            *next += 1;
            Some(frames)
        }
    }

    #[test]
    fn a_60_hz_redraw_shows_29_97_content_without_dropping_a_frame() {
        let mut presenter = Presenter::new(NTSC, Arc::new(Beat::default()));
        presenter.clock.play();
        let start = Instant::now();
        let mut next = 0;

        let mut shown = Vec::new();
        for tick in 0..600u32 {
            let at = start + HZ_60 * tick;
            if let Some(frames) = presenter.advance(at, false, feed(&mut next)) {
                shown.push(frames.index);
            }
        }

        assert_eq!(presenter.stats.dropped, 0);
        assert_eq!(presenter.stats.starved, 0);
        // 600 refreshes is 10 s of 60 Hz, which is 299 or 300 frames of
        // 29.97, and every one of them arrives in order exactly once.
        assert!(shown.len() >= 299, "only {} frames shown", shown.len());
        assert!(shown.windows(2).all(|w| w[1] == w[0] + 1));
    }

    #[test]
    fn a_display_slower_than_the_content_drops_rather_than_falls_behind() {
        let mut presenter = Presenter::new(NTSC, Arc::new(Beat::default()));
        presenter.clock.play();
        let start = Instant::now();
        let mut next = 0;

        for tick in 0..100u32 {
            presenter.advance(start + NTSC * 3 * tick, false, feed(&mut next));
        }

        // One frame shown per pump, the other two of each three skipped.
        assert_eq!(presenter.stats.presented, 100);
        assert!(presenter.stats.dropped >= 197);
        assert_eq!(presenter.stats.starved, 0);
    }

    #[test]
    fn a_stalled_decoder_starves_and_never_drops() {
        let mut presenter = Presenter::new(NTSC, Arc::new(Beat::default()));
        presenter.clock.play();
        let start = Instant::now();

        // One frame decoded, and then the decoder produces nothing.
        let mut first = Some(frame(0));
        presenter.advance(start, false, || first.take());
        for tick in 1..10u32 {
            presenter.advance(start + HZ_60 * tick, false, || None);
        }

        assert_eq!(presenter.stats.presented, 1);
        assert_eq!(presenter.stats.dropped, 0);
        // Every refresh from the second frame's due time on is a frozen
        // picture, and each one is counted.
        assert_eq!(presenter.stats.starved, 7);
    }

    #[test]
    fn pause_freezes_the_position_and_resume_carries_on_from_it() {
        let mut presenter = Presenter::new(NTSC, Arc::new(Beat::default()));
        presenter.clock.play();
        let start = Instant::now();
        let mut next = 0;

        for tick in 0..60u32 {
            presenter.advance(start + HZ_60 * tick, false, feed(&mut next));
        }
        let paused_at = start + HZ_60 * 60;
        presenter.clock.pause(paused_at);
        let position = presenter.clock.position(paused_at);

        // An hour later the position has not moved and no frame is shown.
        let later = paused_at + Duration::from_secs(3600);
        assert_eq!(presenter.clock.position(later), position);
        assert!(presenter.advance(later, false, feed(&mut next)).is_none());

        // Resuming picks up where it stopped rather than jumping an hour.
        presenter.clock.play();
        presenter.advance(later, false, feed(&mut next));
        assert!(presenter.clock.position(later) < position + NTSC * 2);
        assert_eq!(presenter.stats.dropped, 0);
    }

    #[test]
    fn the_first_frame_shows_however_long_the_decoder_took_to_produce_it() {
        let mut presenter = Presenter::new(NTSC, Arc::new(Beat::default()));
        presenter.clock.play();
        let start = Instant::now();
        let mut next = 0;

        // Half a second of opening the file and warming the decoder.
        for tick in 0..30u32 {
            presenter.advance(start + HZ_60 * tick, false, || None);
        }
        let shown = presenter.advance(start + Duration::from_millis(500), false, feed(&mut next));

        assert_eq!(shown.map(|f| f.index), Some(0));
        assert_eq!(presenter.stats.dropped, 0);
        assert_eq!(presenter.stats.starved, 0);
    }

    /// A seek while paused has to produce exactly one picture, or the pilot
    /// is left looking at the frame they seeked away from. It is the case
    /// that matters most, because dragging the scrubber pauses.
    #[test]
    fn a_seek_while_paused_shows_one_frame_and_then_stands_still() {
        let mut presenter = Presenter::new(NTSC, Arc::new(Beat::default()));
        let start = Instant::now();
        let mut next = 0;
        presenter.advance(start, false, feed(&mut next));
        assert!(presenter.advance(start, false, feed(&mut next)).is_none());

        presenter.reseek(NTSC * 900);
        assert_eq!(presenter.clock.position(start), NTSC * 900);

        // The seek is owed a picture, which is what the player says while one
        // is outstanding, and nothing is owed once it has landed.
        let mut landed = Some(frame(900));
        assert_eq!(
            presenter
                .advance(start, true, || landed.take())
                .map(|f| f.index),
            Some(900)
        );
        assert!(presenter.advance(start, false, feed(&mut next)).is_none());
    }

    /// A keyframe seek lands at or before what it was asked for, and the
    /// clock has to read where the picture really is: a clock left on the
    /// request would make the next frame look late and drop it.
    #[test]
    fn the_clock_takes_the_landing_time_and_not_the_request() {
        let mut presenter = Presenter::new(NTSC, Arc::new(Beat::default()));
        presenter.clock.play();
        let start = Instant::now();

        presenter.reseek(NTSC * 950);
        let mut landed = Some(frame(900));
        presenter.advance(start, true, || landed.take());

        assert_eq!(presenter.clock.position(start), NTSC * 900);
        assert_eq!(presenter.stats.dropped, 0);
    }

    #[test]
    fn a_paused_player_still_takes_one_frame_so_there_is_a_picture() {
        let mut presenter = Presenter::new(NTSC, Arc::new(Beat::default()));
        let start = Instant::now();
        let mut next = 0;

        assert_eq!(
            presenter
                .advance(start, false, feed(&mut next))
                .map(|f| f.index),
            Some(0)
        );
        assert!(presenter.advance(start, false, feed(&mut next)).is_none());
        assert_eq!(presenter.stats.presented, 1);
        assert_eq!(presenter.stats.dropped, 0);
    }

    /// Packet reads in one read of the fake source below. A real read asks
    /// the interrupt between packets, so preemption can only ever happen at
    /// that grain and the fake has to have the same grain to be worth
    /// anything.
    const PACKETS: usize = 8;

    /// What the fake source below was asked to do, in order. `Gave` is the
    /// whole point: an abandoned read sends no note, so the trace is the
    /// only place the work it did not finish shows up.
    #[derive(Debug, PartialEq, Eq)]
    enum Did {
        Seek(u64, Accuracy),
        Read(u64),
        Gave(u64),
    }

    /// A source with no file behind it, so that the decode thread's loop
    /// (newest command wins, preempt the lookahead, tag by epoch) can be run
    /// on the test's own thread.
    ///
    /// `script` has one entry per read the loop attempts, interrupted ones
    /// included: the command, if any, that lands in the middle of it. Mid-read is the case that matters, because a
    /// command arriving between two reads was never in danger of being
    /// missed. When the script runs out the source lets go of the command
    /// channel and says the file ended, which is what stops the loop.
    struct Fake {
        at: u64,
        read: usize,
        script: Vec<Option<Command>>,
        commands: Option<Sender<Command>>,
        did: Arc<Mutex<Vec<Did>>>,
    }

    impl Fake {
        fn note(&self, did: Did) {
            self.did.lock().unwrap().push(did);
        }
    }

    impl Source for Fake {
        fn seek(&mut self, to: Cue, accuracy: Accuracy) -> Fallible<()> {
            // The fake counts frames, so an index cue is all it can answer.
            let Cue::Index(index) = to else {
                return Err("the fake source seeks by index".into());
            };
            self.at = index;
            self.note(Did::Seek(index, accuracy));
            Ok(())
        }

        fn read_until(&mut self, interrupted: &mut dyn FnMut() -> bool) -> Fallible<Read> {
            let Some(mut arriving) = self.script.get_mut(self.read).map(Option::take) else {
                self.commands = None;
                return Ok(Read::Ended);
            };
            self.read += 1;
            for packet in 0..PACKETS {
                if packet == PACKETS / 2
                    && let Some(order) = arriving.take()
                    && let Some(commands) = &self.commands
                {
                    commands.send(order).unwrap();
                }
                if interrupted() {
                    self.note(Did::Gave(self.at));
                    return Ok(Read::Interrupted);
                }
            }
            self.note(Did::Read(self.at));
            self.at += 1;
            Ok(Read::Frames(frame(self.at - 1)))
        }
    }

    /// Runs the decode thread's loop against [`Fake`] until its script runs
    /// out, and reports what came back and what the source was asked for.
    fn decode(first: Vec<Command>, script: Vec<Option<Command>>) -> (Vec<(u64, u64)>, Vec<Did>) {
        let (commands, orders) = channel();
        let (sender, notes) = sync_channel(64);
        let did = Arc::new(Mutex::new(Vec::new()));
        for order in first {
            commands.send(order).unwrap();
        }
        let source = Fake {
            at: 0,
            read: 0,
            script,
            commands: Some(commands.clone()),
            did: did.clone(),
        };
        // The source's clone is the only one left, so the loop ends when it
        // lets go rather than when this function does.
        drop(commands);
        decode_ahead(source, &sender, &orders);
        drop(sender);

        let shown = notes
            .try_iter()
            .filter_map(|note| match note {
                Note::Frames(epoch, frames) => Some((epoch, frames.index)),
                _ => None,
            })
            .collect();
        let did = did.lock().unwrap().drain(..).collect();
        (shown, did)
    }

    fn seek_to(epoch: u64, index: u64, accuracy: Accuracy) -> Command {
        Command::Seek {
            epoch,
            to: Cue::Index(index),
            accuracy,
        }
    }

    /// The refill after a landing is three pair decodes for a position the
    /// pilot may already have dragged away from (38 ms of the 59 ms a scrub
    /// update cost, issue #46). A command landing in the middle of it takes
    /// the thread off it, and the frame it was decoding is never shown.
    #[test]
    fn a_command_arriving_mid_refill_preempts_it() {
        let (shown, did) = decode(
            vec![seek_to(1, 100, Accuracy::Keyframe)],
            vec![None, Some(seek_to(2, 500, Accuracy::Keyframe)), None],
        );

        // Frame 101, the one the refill was reading, never reaches the
        // player, and the picture that follows carries the newer epoch.
        assert_eq!(shown, [(1, 100), (2, 500)]);
        assert_eq!(
            did,
            [
                Did::Seek(100, Accuracy::Keyframe),
                Did::Read(100),
                Did::Gave(101),
                Did::Seek(500, Accuracy::Keyframe),
                Did::Read(500),
            ]
        );
    }

    /// Playback is the case where nothing is pending, and it must not pay
    /// for the interrupt path at all: every read runs to a frame, in order,
    /// under one epoch.
    #[test]
    fn nothing_is_preempted_while_the_channel_is_empty() {
        let (shown, did) = decode(
            vec![seek_to(1, 100, Accuracy::Keyframe)],
            vec![None, None, None, None],
        );

        assert_eq!(shown, [(1, 100), (1, 101), (1, 102), (1, 103)]);
        assert!(!did.contains(&Did::Gave(101)));
        assert_eq!(did.iter().filter(|d| matches!(d, Did::Gave(_))).count(), 0);
    }

    /// Letting go of the scrubber asks for the frame itself, and it has to
    /// win however many drag positions are still in flight: one exact seek,
    /// one picture, tagged with the release's own epoch.
    #[test]
    fn a_release_overtakes_the_drag_positions_behind_it() {
        let (shown, did) = decode(
            vec![
                seek_to(1, 100, Accuracy::Keyframe),
                seek_to(2, 200, Accuracy::Keyframe),
                seek_to(3, 300, Accuracy::Keyframe),
                seek_to(4, 317, Accuracy::Exact),
            ],
            vec![None],
        );

        assert_eq!(shown, [(4, 317)]);
        assert_eq!(did, [Did::Seek(317, Accuracy::Exact), Did::Read(317)]);
    }

    /// The same release, arriving while the thread is refilling behind a
    /// drag landing. The interruption must not cost it its accuracy or its
    /// epoch: this is the one path the pilot's finger leaving the scrubber
    /// goes down.
    #[test]
    fn a_release_mid_refill_still_lands_exactly() {
        let (shown, did) = decode(
            vec![seek_to(1, 100, Accuracy::Keyframe)],
            vec![None, Some(seek_to(2, 117, Accuracy::Exact)), None],
        );

        assert_eq!(shown, [(1, 100), (2, 117)]);
        assert_eq!(
            did,
            [
                Did::Seek(100, Accuracy::Keyframe),
                Did::Read(100),
                Did::Gave(101),
                Did::Seek(117, Accuracy::Exact),
                Did::Read(117),
            ]
        );
    }

    /// A landing is what the newest command asked for, so it finishes even
    /// with a newer one waiting. Interrupting it would mean a drag whose
    /// positions arrive faster than a keyframe decode takes (16.7 ms against
    /// 21 ms on this camera) never produces a picture at all.
    #[test]
    fn a_landing_finishes_even_with_a_command_waiting() {
        let (shown, did) = decode(
            vec![seek_to(1, 100, Accuracy::Keyframe)],
            vec![Some(seek_to(2, 500, Accuracy::Keyframe)), None],
        );

        assert_eq!(shown, [(1, 100), (2, 500)]);
        assert_eq!(
            did,
            [
                Did::Seek(100, Accuracy::Keyframe),
                Did::Read(100),
                Did::Seek(500, Accuracy::Keyframe),
                Did::Read(500),
            ]
        );
    }

    /// Which frames may take the screen is a different question with a seek
    /// outstanding and without one, and the whole of issue #55 is that the
    /// answer to the first is not "only the newest seek's".
    #[test]
    fn what_may_take_the_screen_depends_on_what_the_picture_is_waiting_for() {
        let mut epochs = Epochs::default();
        assert!(epochs.accepts(0), "the first frame of a file carries 0");

        for _ in 0..4 {
            epochs.ask();
        }
        epochs.showed(2);

        // Waiting on a seek: a newer position, and nothing else.
        assert!(
            epochs.accepts(3),
            "a position asked for after the one shown"
        );
        assert!(!epochs.accepts(2), "more of the position being left");
        assert!(!epochs.accepts(1), "a position left two seeks ago");

        // Waiting on the frame after the one on screen, which is what
        // stepping forward asks for and which carries the epoch in force.
        epochs.owe();
        assert!(epochs.accepts(2));
        assert!(!epochs.accepts(1));

        // Playing: every frame carries the epoch on screen.
        epochs.showed(4);
        assert!(!epochs.is_seeking());
        assert!(epochs.accepts(4));
        assert!(!epochs.accepts(3));
    }

    /// The lifecycle, which is the half of this the epochs do not answer: a
    /// drag leaves several seeks in flight, pictures go up from the ones the
    /// pilot has dragged past, and the wait ends on the newest seek's own
    /// frame and no other. `seeking` is what keeps a paused window redrawing,
    /// so a wait that ends early is a picture that never arrives.
    #[test]
    fn overlapping_seeks_keep_seeking_until_the_newest_lands() {
        let mut epochs = Epochs::default();
        assert!(!epochs.is_seeking(), "an open file is not seeking");

        for epoch in 1..=3 {
            assert_eq!(epochs.ask(), epoch);
        }
        epochs.showed(1);
        assert!(epochs.is_seeking(), "two seeks are still in flight");
        epochs.showed(2);
        assert!(epochs.is_seeking(), "one seek is still in flight");
        epochs.showed(3);
        assert!(!epochs.is_seeking());

        // Stepping one frame on sends no seek: the reader is already sitting
        // on the frame, and the epoch in force is the one it will carry.
        epochs.owe();
        assert!(epochs.is_seeking());
        epochs.showed(3);
        assert!(!epochs.is_seeking());
    }

    /// A [`Player`] with no file behind it. The test is the decode thread: it
    /// sends the notes and reads the commands, so the whole of `pump` runs on
    /// the test's own thread with no decoder and no footage.
    struct Bench {
        player: Player,
        notes: SyncSender<Note>,
        commands: Receiver<Command>,
    }

    impl Bench {
        fn new() -> Self {
            let (sender, notes) = sync_channel(64);
            let (commands, orders) = channel();
            let timing = Timing::new(crate::ff::Rational::new(30_000, 1001), 100_000).unwrap();
            Self {
                player: Player {
                    notes,
                    commands,
                    presenter: Presenter::new(timing.interval(), Arc::new(Beat::default())),
                    sound: None,
                    timing,
                    size: Size::new(3840, 3840),
                    lenses: 2,
                    failure: None,
                    ended: false,
                    epochs: Epochs::default(),
                },
                notes: sender,
                commands: orders,
            }
        }

        /// A pair decoded under `epoch`, as the decode thread would hand it
        /// over.
        fn decoded(&self, epoch: u64, index: u64) {
            self.notes.send(Note::Frames(epoch, frame(index))).unwrap();
        }

        /// One redraw, and the frame it put on screen.
        fn redraw(&mut self, now: Instant) -> Option<u64> {
            self.player.pump(now).unwrap().map(|frames| frames.index)
        }

        /// What the decode thread was asked for, in order.
        fn asked(&self) -> Vec<(u64, Accuracy)> {
            self.commands
                .try_iter()
                .map(|Command::Seek { to, accuracy, .. }| (to.index(self.player.timing), accuracy))
                .collect()
        }
    }

    /// The whole of issue #55. Every position is asked for before the last
    /// one has produced a picture, so each picture that does arrive carries
    /// an epoch the pilot dragged past two seeks ago. Showing only the newest
    /// seek's frames shows none of them, and the picture is frozen for the
    /// length of the drag.
    #[test]
    fn a_drag_faster_than_the_decoder_still_moves_the_picture() {
        let mut bench = Bench::new();
        let now = Instant::now();
        let mut shown = Vec::new();

        // Five positions, and the decoder two landings behind the hand.
        for position in 0..5u64 {
            bench
                .player
                .seek(Cue::Index(position * 1000), Accuracy::Keyframe);
            if let Some(landed) = position.checked_sub(2) {
                bench.decoded(landed + 1, landed * 1000);
            }
            shown.extend(bench.redraw(now));
        }

        assert_eq!(shown, [0, 1000, 2000]);
        assert!(
            bench.player.is_seeking(),
            "the position under the finger has not landed"
        );
    }

    /// Letting go asks for the frame under the handle, and the drag's
    /// leftovers are still coming out of the decoder behind it. They may have
    /// the screen while the release is still being decoded, and they may not
    /// end the wait for it: `is_seeking` false is what stops a paused window
    /// redrawing, and it would stop it one picture short of the release.
    #[test]
    fn the_release_lands_the_exact_frame_behind_the_drag_s_leftovers() {
        let mut bench = Bench::new();
        let now = Instant::now();

        bench.player.seek(Cue::Index(1000), Accuracy::Keyframe);
        bench.player.seek(Cue::Index(2000), Accuracy::Keyframe);
        bench.player.seek(Cue::Index(3000), Accuracy::Exact);

        bench.decoded(2, 2000);
        assert_eq!(bench.redraw(now), Some(2000), "a keyframe from the drag");
        assert!(
            bench.player.is_seeking(),
            "the release is still being decoded"
        );

        bench.decoded(3, 3000);
        assert_eq!(bench.redraw(now), Some(3000));
        assert!(!bench.player.is_seeking());
        assert_eq!(bench.player.index(), Some(3000));

        // And it stands still on the frame it landed on.
        assert_eq!(bench.redraw(now), None);
        assert_eq!(bench.player.index(), Some(3000));
        assert_eq!(
            bench.asked(),
            [
                (1000, Accuracy::Keyframe),
                (2000, Accuracy::Keyframe),
                (3000, Accuracy::Exact),
            ]
        );
    }

    /// A seek is a request to leave the position on screen, and the read that
    /// was in flight when it was sent lands after it: the reader hands over
    /// one more frame of the position being left. That frame is a picture of
    /// nowhere the pilot asked to be. Measured through the real player, an
    /// exact seek shows it 79 ms after the request and the frame that was
    /// asked for 159 ms after that.
    #[test]
    fn frames_of_the_position_being_left_do_not_take_the_screen() {
        let mut bench = Bench::new();
        let now = Instant::now();

        bench.player.seek(Cue::Index(1000), Accuracy::Keyframe);
        bench.decoded(1, 1000);
        assert_eq!(bench.redraw(now), Some(1000));

        bench.player.seek(Cue::Index(9000), Accuracy::Exact);
        bench.decoded(1, 1001);
        assert_eq!(bench.redraw(now), None, "the position being left");
        assert!(bench.player.is_seeking());

        bench.decoded(2, 9000);
        assert_eq!(bench.redraw(now), Some(9000));
        assert!(!bench.player.is_seeking());
    }

    /// The floor under all of it: a frame older than the picture on screen
    /// would run the picture backwards under the finger. One decode thread
    /// hands frames over in the order they were asked for, so this delivery
    /// is one it does not make; the floor is what makes "never backwards"
    /// something `pump` decides rather than something the thread happens to
    /// do.
    #[test]
    fn a_frame_older_than_the_picture_on_screen_is_refused() {
        let mut bench = Bench::new();
        let now = Instant::now();

        bench.player.seek(Cue::Index(1000), Accuracy::Keyframe);
        bench.player.seek(Cue::Index(2000), Accuracy::Keyframe);
        bench.player.seek(Cue::Index(3000), Accuracy::Keyframe);

        bench.decoded(2, 2000);
        assert_eq!(bench.redraw(now), Some(2000));

        bench.decoded(1, 1000);
        assert_eq!(bench.redraw(now), None, "older than what is on screen");
        assert_eq!(bench.player.index(), Some(2000));
        assert!(bench.player.is_seeking());
    }

    /// A drag that runs off the end of the file reads one position to the end
    /// while the hand is already somewhere else. That end belongs to the seek
    /// that hit it and to no other: taken for the player's own, it stops the
    /// clock and the redraws with a seek still owed a picture.
    #[test]
    fn the_end_of_a_superseded_seek_is_not_the_end_of_the_file() {
        let mut bench = Bench::new();
        let now = Instant::now();

        bench.player.seek(Cue::Index(99_999), Accuracy::Keyframe);
        bench.player.seek(Cue::Index(1000), Accuracy::Keyframe);

        bench.notes.send(Note::Ended(1)).unwrap();
        assert_eq!(bench.redraw(now), None);
        assert!(
            !bench.player.is_ended(),
            "the newest seek is not at the end"
        );

        bench.decoded(2, 1000);
        assert_eq!(bench.redraw(now), Some(1000));
        assert!(!bench.player.is_seeking());
    }

    /// Playback is the case with nothing outstanding, and the epochs must not
    /// reach it: frames are paced by their own timestamps, one to a due time,
    /// however many redraws that takes.
    #[test]
    fn a_landing_while_playing_is_followed_by_ordinary_pacing() {
        let mut bench = Bench::new();
        let start = Instant::now();

        bench.player.play();
        bench.player.seek(Cue::Index(900), Accuracy::Exact);
        for index in 900..904 {
            bench.decoded(1, index);
        }

        // The landing goes up on arrival, wherever the clock is.
        assert_eq!(bench.redraw(start), Some(900));
        assert!(!bench.player.is_seeking());
        // The three behind it wait for their own due times.
        assert_eq!(bench.redraw(start + HZ_60), None);
        assert_eq!(bench.redraw(start + NTSC), Some(901));
        assert_eq!(bench.redraw(start + NTSC * 2), Some(902));
        assert_eq!(bench.player.stats().dropped, 0);
        assert_eq!(bench.player.stats().starved, 0);
    }
}
