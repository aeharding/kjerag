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
        }
    }

    /// One line, for a run of `over`.
    pub fn report(&self, over: Duration) -> String {
        let per_second = |count: u64| count as f64 / over.as_secs_f64().max(f64::EPSILON);
        format!(
            "{:.2} fps presented in {:.1} redraws/s, {} dropped, {} starved, \
             worst {:.1} ms late",
            per_second(self.presented),
            per_second(self.redraws),
            self.dropped,
            self.starved,
            self.worst_late.as_secs_f64() * 1000.0,
        )
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
    timing: Timing,
    size: Size,
    lenses: usize,
    failure: Option<Box<dyn std::error::Error + Send + Sync>>,
    ended: bool,
    /// Bumped by every seek. Frames tagged with an older one were decoded
    /// before it and are no longer what the pilot asked to see.
    epoch: u64,
    /// Set from a seek until the frame it asked for is on screen. The picture
    /// is about to change even when the clock is not running, which is what
    /// tells a paused caller to keep redrawing.
    seeking: bool,
}

impl Player {
    /// Opens the file and starts decoding. Returns as soon as the container
    /// is parsed: the first frame arrives on the thread, so a big file does
    /// not hold the window shut.
    pub fn open(path: &Path) -> Fallible<Self> {
        let reader = Reader::open(path)?.lookahead(LOOKAHEAD);
        let (timing, size, lenses) = (reader.timing(), reader.size(), reader.lenses());
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
            presenter: Presenter::new(timing.interval()),
            timing,
            size,
            lenses,
            failure: None,
            ended: false,
            epoch: 0,
            seeking: false,
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
        self.presenter.clock.playing
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
    pub fn is_seeking(&self) -> bool {
        self.seeking
    }

    pub fn stats(&self) -> Stats {
        self.presenter.stats
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
        self.epoch += 1;
        self.ended = false;
        self.seeking = true;
        self.presenter.reseek(to.time(self.timing));
        let command = Command::Seek {
            epoch: self.epoch,
            to,
            accuracy,
        };
        if self.commands.send(command).is_err() {
            self.seeking = false;
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
                self.presenter.reseek(self.timing.time_of(target));
                self.seeking = true;
            }
            false => self.seek(Cue::Index(target), Accuracy::Exact),
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
        let epoch = self.epoch;
        let (notes, failure, ended) = (&self.notes, &mut self.failure, &mut self.ended);
        let shown = self.presenter.advance(now, || {
            loop {
                match notes.try_recv() {
                    Ok(Note::Frames(tag, frames)) if tag == epoch => return Some(frames),
                    // Decoded before the seek that is now in force.
                    Ok(Note::Frames(..)) => continue,
                    Ok(Note::Ended(tag)) => {
                        *ended = tag == epoch;
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
        if shown.is_some() {
            self.seeking = false;
        }
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
    fn new(interval: Duration) -> Self {
        Self {
            clock: Clock::default(),
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

    fn advance(
        &mut self,
        now: Instant,
        mut next: impl FnMut() -> Option<Frames>,
    ) -> Option<Arc<Frames>> {
        self.stats.redraws += 1;
        if self.is_frozen() {
            return None;
        }
        let mut shown = None;

        while let Some(frames) = self.peeked.take().or_else(&mut next) {
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
            if !self.clock.playing {
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
    fn is_frozen(&self) -> bool {
        !self.clock.playing && self.current.is_some()
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
        if !self.clock.playing || !self.clock.is_anchored() {
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
#[derive(Debug, Default)]
struct Clock {
    playing: bool,
    position: Duration,
    origin: Option<Instant>,
}

impl Clock {
    fn position(&self, now: Instant) -> Duration {
        match self.origin {
            Some(origin) if self.playing => self.position + now.saturating_duration_since(origin),
            _ => self.position,
        }
    }

    fn is_anchored(&self) -> bool {
        self.origin.is_some()
    }

    /// The instant this clock will read `target`, if it is running towards
    /// it. A due time, in other words, and the reason the caller can sleep
    /// instead of polling.
    fn reaches(&self, target: Duration) -> Option<Instant> {
        let origin = self.origin?;
        match self.playing {
            true => Some(origin + target.saturating_sub(self.position)),
            false => None,
        }
    }

    fn play(&mut self) {
        self.playing = true;
        self.origin = None;
    }

    fn pause(&mut self, now: Instant) {
        self.position = self.position(now);
        self.playing = false;
        self.origin = None;
    }

    fn anchor(&mut self, now: Instant, at: Duration) {
        self.position = at;
        self.origin = Some(now);
    }

    /// Where the clock reads until the frame that was seeked to arrives.
    /// Unanchoring is what makes that frame anchor it, whenever it comes.
    fn seek(&mut self, to: Duration) {
        self.position = to;
        self.origin = None;
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
        let mut presenter = Presenter::new(NTSC);
        presenter.clock.play();
        let start = Instant::now();
        let mut next = 0;

        let mut shown = Vec::new();
        for tick in 0..600u32 {
            let at = start + HZ_60 * tick;
            if let Some(frames) = presenter.advance(at, feed(&mut next)) {
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
        let mut presenter = Presenter::new(NTSC);
        presenter.clock.play();
        let start = Instant::now();
        let mut next = 0;

        for tick in 0..100u32 {
            presenter.advance(start + NTSC * 3 * tick, feed(&mut next));
        }

        // One frame shown per pump, the other two of each three skipped.
        assert_eq!(presenter.stats.presented, 100);
        assert!(presenter.stats.dropped >= 197);
        assert_eq!(presenter.stats.starved, 0);
    }

    #[test]
    fn a_stalled_decoder_starves_and_never_drops() {
        let mut presenter = Presenter::new(NTSC);
        presenter.clock.play();
        let start = Instant::now();

        // One frame decoded, and then the decoder produces nothing.
        let mut first = Some(frame(0));
        presenter.advance(start, || first.take());
        for tick in 1..10u32 {
            presenter.advance(start + HZ_60 * tick, || None);
        }

        assert_eq!(presenter.stats.presented, 1);
        assert_eq!(presenter.stats.dropped, 0);
        // Every refresh from the second frame's due time on is a frozen
        // picture, and each one is counted.
        assert_eq!(presenter.stats.starved, 7);
    }

    #[test]
    fn pause_freezes_the_position_and_resume_carries_on_from_it() {
        let mut presenter = Presenter::new(NTSC);
        presenter.clock.play();
        let start = Instant::now();
        let mut next = 0;

        for tick in 0..60u32 {
            presenter.advance(start + HZ_60 * tick, feed(&mut next));
        }
        let paused_at = start + HZ_60 * 60;
        presenter.clock.pause(paused_at);
        let position = presenter.clock.position(paused_at);

        // An hour later the position has not moved and no frame is shown.
        let later = paused_at + Duration::from_secs(3600);
        assert_eq!(presenter.clock.position(later), position);
        assert!(presenter.advance(later, feed(&mut next)).is_none());

        // Resuming picks up where it stopped rather than jumping an hour.
        presenter.clock.play();
        presenter.advance(later, feed(&mut next));
        assert!(presenter.clock.position(later) < position + NTSC * 2);
        assert_eq!(presenter.stats.dropped, 0);
    }

    #[test]
    fn the_first_frame_shows_however_long_the_decoder_took_to_produce_it() {
        let mut presenter = Presenter::new(NTSC);
        presenter.clock.play();
        let start = Instant::now();
        let mut next = 0;

        // Half a second of opening the file and warming the decoder.
        for tick in 0..30u32 {
            presenter.advance(start + HZ_60 * tick, || None);
        }
        let shown = presenter.advance(start + Duration::from_millis(500), feed(&mut next));

        assert_eq!(shown.map(|f| f.index), Some(0));
        assert_eq!(presenter.stats.dropped, 0);
        assert_eq!(presenter.stats.starved, 0);
    }

    /// A seek while paused has to produce exactly one picture, or the pilot
    /// is left looking at the frame they seeked away from. It is the case
    /// that matters most, because dragging the scrubber pauses.
    #[test]
    fn a_seek_while_paused_shows_one_frame_and_then_stands_still() {
        let mut presenter = Presenter::new(NTSC);
        let start = Instant::now();
        let mut next = 0;
        presenter.advance(start, feed(&mut next));
        assert!(presenter.advance(start, feed(&mut next)).is_none());

        presenter.reseek(NTSC * 900);
        assert_eq!(presenter.clock.position(start), NTSC * 900);

        let mut landed = Some(frame(900));
        assert_eq!(
            presenter.advance(start, || landed.take()).map(|f| f.index),
            Some(900)
        );
        assert!(presenter.advance(start, feed(&mut next)).is_none());
    }

    /// A keyframe seek lands at or before what it was asked for, and the
    /// clock has to read where the picture really is: a clock left on the
    /// request would make the next frame look late and drop it.
    #[test]
    fn the_clock_takes_the_landing_time_and_not_the_request() {
        let mut presenter = Presenter::new(NTSC);
        presenter.clock.play();
        let start = Instant::now();

        presenter.reseek(NTSC * 950);
        let mut landed = Some(frame(900));
        presenter.advance(start, || landed.take());

        assert_eq!(presenter.clock.position(start), NTSC * 900);
        assert_eq!(presenter.stats.dropped, 0);
    }

    #[test]
    fn a_paused_player_still_takes_one_frame_so_there_is_a_picture() {
        let mut presenter = Presenter::new(NTSC);
        let start = Instant::now();
        let mut next = 0;

        assert_eq!(
            presenter.advance(start, feed(&mut next)).map(|f| f.index),
            Some(0)
        );
        assert!(presenter.advance(start, feed(&mut next)).is_none());
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
}
