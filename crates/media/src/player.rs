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
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

use super::{Fallible, Frames, Reader, Size, Timing};

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

/// A file, decoding on its own thread, and the clock that decides which of
/// its frames belongs on screen.
pub struct Player {
    frames: Receiver<Fallible<Frames>>,
    presenter: Presenter,
    timing: Timing,
    size: Size,
    lenses: usize,
    failure: Option<Box<dyn std::error::Error + Send + Sync>>,
    ended: bool,
}

impl Player {
    /// Opens the file and starts decoding. Returns as soon as the container
    /// is parsed: the first frame arrives on the thread, so a big file does
    /// not hold the window shut.
    pub fn open(path: &Path) -> Fallible<Self> {
        let reader = Reader::open(path)?.lookahead(LOOKAHEAD);
        let (timing, size, lenses) = (reader.timing(), reader.size(), reader.lenses());
        let (sender, frames) = sync_channel(QUEUED);
        thread::Builder::new()
            .name("kyerag-decode".to_owned())
            .spawn(move || decode_ahead(reader, sender))?;

        Ok(Self {
            frames,
            presenter: Presenter::new(timing.interval()),
            timing,
            size,
            lenses,
            failure: None,
            ended: false,
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

    /// The frame that belongs on screen at `now`, or `None` when the picture
    /// must not change. Call it on every redraw; it is the whole clock.
    pub fn pump(&mut self, now: Instant) -> Fallible<Option<Arc<Frames>>> {
        let (frames, failure, ended) = (&self.frames, &mut self.failure, &mut self.ended);
        let shown = self.presenter.advance(now, || match frames.try_recv() {
            Ok(Ok(frames)) => Some(frames),
            Ok(Err(e)) => {
                *failure = Some(e);
                None
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                *ended = true;
                None
            }
        });
        match self.failure.take() {
            Some(e) => Err(e),
            None => Ok(shown),
        }
    }
}

/// Decodes ahead of the picture until the player goes away. The bounded
/// channel is the throttle: a full one blocks here, so a paused player
/// stops decoding after `QUEUED` pairs instead of eating the file.
fn decode_ahead(mut reader: Reader, sender: SyncSender<Fallible<Frames>>) {
    loop {
        let message = match reader.next_frames() {
            Ok(Some(frames)) => Ok(frames),
            // End of file. Dropping the sender closes the channel, which is
            // how the player learns there is no more.
            Ok(None) => return,
            Err(e) => Err(e),
        };
        let failed = message.is_err();
        if sender.send(message).is_err() || failed {
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
    /// picture yet is the file that was just opened: it gets one frame.
    fn is_frozen(&self) -> bool {
        !self.clock.playing && self.current.is_some()
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
}

#[cfg(test)]
mod tests {
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
}
