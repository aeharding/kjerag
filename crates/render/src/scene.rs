//! The one shader pass the player draws.
//!
//! With no frame it draws nothing at all: every ray misses every lens, which
//! is the transparent room the ball floats in (issue #100), so what the pilot
//! sees between opening a file and its first frame is the backdrop the shell
//! paints behind this widget. With a frame it reprojects a real VA-API frame
//! imported by [`super::dmabuf`]: for every output pixel, a view ray
//! through the camera's yaw, pitch and field of view, rotated into each
//! lens's frame and pushed through the Mei/UCM model in
//! [`super::projection`], sampled from the NV12 planes of whichever lens
//! wins and converted to RGB. One pass, no intermediate target.
//!
//! The frames move (issue #4): a [`Player`] decodes both lenses on its own
//! thread and this file asks it, on every redraw, which pair belongs on
//! screen; [`ScenePipeline::prepare`] imports that pair and binds it. Both
//! lenses are sampled as well as imported, so the picture is the whole
//! sphere (issue #27), and where the two overlap the pass mixes them by the
//! weight field in [`super::projection`] rather than picking one (issue #7).
//! Outside the overlap that weight is exactly 1 and the fetch is the single
//! fetch it always was.
//!
//! [`Scene::pump`] takes `&self` and keeps the clock behind a [`RefCell`],
//! which is not how a player would be written on its own. It is how iced's
//! `shader::Program` is shaped: `update` and `draw` both borrow the program
//! immutably, and the pump has to happen inside the redraw pass, before the
//! draw, or the picture is always one refresh behind the clock. The cell is
//! touched from the UI thread only; the decode thread never sees it.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kjerag_media::{Accuracy, Cue, Frames, Player, Reader, Stats};
use kjerag_meta::{
    CalibrationSet, ExposureTrack, Filter, Format, Lens, OrientationTrack, Quat, Readout,
};

use super::band::{self, Table};
use super::capture::{self, Order, Pending, Request, Shutter, Stamp};
use super::projection::{self, Held, MAX_LENSES, Reframe, Rolling};
use super::sampling::{self, Sampling};
use super::seam::{self, Correction, Harvest, SeamFit};
use super::stall::{Stall, Stalled};
use super::{Camera, Extent, Fallible, Nudge, Planes, Size, Viewpoint, dmabuf};

/// The sampler binding, which sits after every lens's two planes.
const SAMPLER_BINDING: u32 = 1 + 2 * MAX_LENSES as u32;

/// Frames kept alive behind the one being drawn.
///
/// An imported texture aliases the decoder's surface: dropping the
/// [`Frames`] hands that surface back to the decoder, which will write the
/// next picture into it. The GPU may still be reading it, because iced
/// submits after `prepare` returns and presents later still, so a frame is
/// released only once this many newer ones have been bound.
const RETAINED: usize = 3;

/// When the widget should come back, which is the whole of frame pacing:
/// the shell sleeps until the instant the next frame is due rather than
/// polling, so 29.97 fps content costs 29.97 redraws a second.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// Whenever the compositor will take a frame: playback that is still
    /// waiting for its first decoded frame, and a seek that has not landed.
    Refresh,
    /// At this instant, when the frame after the one just taken is due.
    At(Instant),
    /// Nothing changes by itself: paused, ended, or a still frame.
    Never,
    /// The picture is gone and the file has been stopped, sound and all
    /// (issue #124). Nothing changes by itself here either, and the
    /// difference is that somebody has to be told: this arm is how a failure
    /// in the pass reaches the shell's alert, and there is no other way out
    /// of this crate for one.
    Stopped(Stall),
}

/// Which clock a decoded frame's instant is read on, before the camera's
/// orientation is looked up at that instant (issue #8).
///
/// The player uses [`Self::Exposure`] and this exists so the losing
/// hypothesis stays measurable: `kjerag-spike --bin horizon` renders the same
/// frames both ways and reports what the difference is worth in degrees of
/// horizon tilt. Nothing in the shell offers the choice.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FrameClock {
    /// The exposure record's own timestamp for this frame, which is the
    /// camera's clock and the one `pts_type = 2` names
    /// ([`ExposureTrack::frame_time_us`]).
    #[default]
    Exposure,
    /// The container's PTS, which is a nominal 30000/1001 grid.
    Container,
}

/// Whether the picture is held against the world or against the camera body.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Horizon {
    /// The world stays put: the body's roll, pitch and heading are all taken
    /// out, so the view is pointed at a direction in the world and the
    /// aircraft turns underneath it (owner ruling, 2026-08-06). Until then
    /// the heading was high passed and a deliberate turn carried the picture
    /// round with it; what is left of the lock's own motion is the
    /// gyroscope's yaw drift, which nothing in these files can bound.
    #[default]
    Locked,
    /// The view rides the camera, as it did before issue #8.
    Free,
}

/// The widget's state, owned by the shell.
pub struct Scene {
    show: Option<Show>,
    /// Where a capture waits for the redraw that takes it (issue #15).
    shutter: Shutter,
    /// Set while the shell has hidden the controls, which is when the pointer
    /// goes with them: `mouse_interaction` answers `Hidden` instead of `Grab`
    /// (docs/UI.md, "The cursor"). One bit of shell state, and the only one
    /// this crate carries.
    cursor_hidden: bool,
    /// Where the view points, and any drag that has hold of it.
    ///
    /// Here rather than in the shader widget's own `State`, which is where
    /// iced would keep it and where issue #77 found it. Widget state lives in
    /// the widget tree, and the tree is rebuilt from the shell's `view`
    /// whenever the window changes shape. The header bar coming and going is
    /// one of those changes -- libcosmic pushes it into the same column as
    /// the content (`src/app/mod.rs:775`), so hiding it moves the content up
    /// a place -- and it goes on entering fullscreen, on leaving it, and two
    /// seconds after the pointer stops while a file plays. Measured
    /// 2026-07-31 through the headless harness: with the bar pinned up every
    /// one of those transitions held the view, and with it free each one put
    /// the camera back to [`Camera::default`].
    ///
    /// The [`Scene`] is the shell's own, and the shell's own state outlives
    /// its view. A [`Cell`] for the same reason the clock is one:
    /// `shader::Program` hands out `&self` and nothing else.
    viewpoint: Cell<Viewpoint>,
    /// A [`Nudge`] the `View` menu left for the widget. Read once, by the
    /// next redraw, which is where the output's shape is known.
    nudge: Cell<Option<Nudge>>,
    /// `View > Lock horizon`. Read on every redraw rather than taken, because
    /// it is a state and not an event.
    horizon: Cell<Horizon>,
    /// Which clock the orientation is looked up on. The instruments move it;
    /// the shell does not.
    clock: Cell<FrameClock>,
    /// An orientation the harness has forced in place of the file's own.
    forced: Cell<Option<Quat>>,
    /// A sensor readout the harness has forced in place of the file's own,
    /// including a zero one, which is the correction switched off.
    readout: Cell<Option<Readout>>,
    /// How the pass samples where the view magnifies the source (issue #11).
    /// The instruments move it; the shell leaves it alone.
    sampling: Cell<Sampling>,
    /// Where the pass leaves word that it cannot draw this file any more
    /// (issue #124). It belongs to the open capture rather than to the
    /// pipeline, which outlives every file it draws.
    stalled: Stalled,
    /// And what it last managed to draw of this file, for the same reason.
    shown: Shown,
}

/// How the picture is to be held for one redraw: the shell's own toggle, and
/// the three overrides the headless instruments reach for.
#[derive(Clone, Copy, Debug)]
struct Holding {
    horizon: Horizon,
    clock: FrameClock,
    forced: Option<Quat>,
    readout: Option<Readout>,
}

/// A file on screen: its calibration, and where its frames come from.
struct Show {
    /// The capture itself, kept because a seam fit reads its own frames off
    /// it, minutes into the file, long after it was opened (issue #48).
    ///
    /// Every file of it, in lens order, and not the one path the pilot named:
    /// a capture written one lens per file has its second lens beside the
    /// first by every route but the file chooser, which hands both halves
    /// over as documents in a directory each (issue #123). The reader has
    /// already answered where they are, and this is that answer kept rather
    /// than asked again.
    files: Arc<[PathBuf]>,
    /// The size of one lens's decoded frame, which the seam fit reads
    /// through the same map the pass draws with.
    frame: Size,
    /// One per decoded stream, in stream order, as the camera calibrated
    /// them.
    lenses: Arc<[Lens]>,
    /// What names the camera these came off, serial-free
    /// ([`CalibrationSet::camera_key`]). The seam calibration is stored under
    /// it.
    camera: u64,
    /// The same lenses with the seam correction in them (issue #48): what the
    /// pool knows about this camera, landed at open, or a fit off this file's
    /// own frames where it knows nothing. The factory calibration until one of
    /// those arrives, and for good on a file with no seam.
    ///
    /// Shared rather than owned because the fallback fit runs on a thread of
    /// its own and hands its answer back through this.
    corrected: Arc<Correction>,
    /// The along-seam table this camera has been read at, landed at open
    /// (issue #103, stage 9). A cell because it is set once from outside and
    /// read on every redraw, exactly like the toggles above.
    table: Cell<Table>,
    /// What a fallback fit off this file came to, for the pool to keep if it
    /// is good enough. The shell reads it when the file is closed or another
    /// is opened; nothing in the render path touches it.
    harvested: Harvested,
    /// Where the camera body was, over the whole file, and the camera's own
    /// timestamp for each frame. Both come out of the trailer at open
    /// (issue #8); both are empty for a file with no IMU record, and then
    /// horizon lock is a no-op rather than an error.
    held: Arc<Motion>,
    /// The clock and the frame it is showing. See the module docs for why
    /// this is a cell.
    playing: RefCell<Playing>,
}

/// What the trailer says about how the camera moved.
struct Motion {
    orientation: OrientationTrack,
    /// Lens 0's shutter track, read for its timestamps rather than its
    /// shutters: `pts_type = 2` makes it the camera's own frame clock.
    exposure: ExposureTrack,
    /// How long one frame takes to come off the sensor and which way it
    /// comes, which is what issue #9's correction is measured against.
    readout: Readout,
}

impl Motion {
    /// The camera's own instant for this frame, in media time.
    fn instant(&self, frames: &Frames, clock: FrameClock) -> i64 {
        let container = || i64::try_from(frames.timestamp.as_micros()).unwrap_or(i64::MAX);
        match clock {
            // The camera's own timestamp, or the container's where the
            // exposure record does not reach: a file whose record is short is
            // a file that still plays.
            FrameClock::Exposure => self
                .exposure
                .frame_time_us(frames.index)
                .unwrap_or_else(container),
            FrameClock::Container => container(),
        }
    }

    /// How the body moved while this frame came off the sensor (issue #9).
    ///
    /// `None` where there is nothing to correct with, or nothing known to
    /// correct: a file with no IMU record, a trailer with no readout time,
    /// and any camera whose readout direction has not been measured
    /// (`kjerag_meta::Sweep::Unknown`, which today is everything but an X4).
    /// The pass is then what it was before issue #9, and the picture with it.
    fn rolling(&self, at: i64, readout: Readout) -> Option<Rolling> {
        let span = (readout.seconds * 1e6) as i64;
        let axis = readout.sweep.axis();
        if self.orientation.is_empty() || span <= 0 || axis == [0.0; 2] {
            return None;
        }
        Some(Rolling {
            // Centred on the frame's own instant, so the middle row is the
            // instant the rest of the pipeline already believes in and the
            // two ends of the window are where the turn is exact.
            turn: self.orientation.turn(at - span / 2, at + span / 2),
            axis,
        })
    }
}

struct Playing {
    frames: Option<Arc<Frames>>,
    source: Source,
}

enum Source {
    /// Playing, or paused mid-play: a decode thread and a clock. Boxed
    /// because the other arm carries nothing.
    Live(Box<Player>),
    /// Frames pulled on this thread, no clock and no thread. What the
    /// headless instruments use; the reader is kept so an instrument that
    /// walks a run of frames pays the container open and the trailer parse
    /// once rather than once per frame.
    Stepped(Box<Reader>),
}

impl Scene {
    /// No file: nothing on the pane but the backdrop behind it.
    pub fn blank() -> Self {
        Self {
            show: None,
            shutter: Shutter::default(),
            cursor_hidden: false,
            viewpoint: Cell::new(Viewpoint::default()),
            nudge: Cell::new(None),
            horizon: Cell::new(Horizon::default()),
            clock: Cell::new(FrameClock::default()),
            forced: Cell::new(None),
            readout: Cell::new(None),
            sampling: Cell::new(Sampling::default()),
            stalled: Stalled::default(),
            shown: Shown::default(),
        }
    }

    /// Opens a file and starts playing it. Returns as soon as the container
    /// is parsed; the first frames arrive on the decode thread.
    pub fn open(path: &Path) -> Fallible<Self> {
        Self::open_with(path, &[])
    }

    /// The same, told about the other files the pilot picked alongside this
    /// one. A capture written one lens per file finds its other half beside
    /// itself, except when a sandbox's file chooser hands over a document
    /// with nothing beside it, and then the pilot's own second pick is the
    /// only place it can come from (issue #123).
    pub fn open_with(path: &Path, alongside: &[PathBuf]) -> Fallible<Self> {
        ours(path)?;
        let mut player = Player::open_with(path, alongside)?;
        let files: Arc<[PathBuf]> = player.paths().into();
        // The trailer is the capture's rather than the picked file's, and on a
        // camera that writes one lens per file only lens 0 carries one
        // (`kjerag_meta::pair`). The pilot picks whichever half his file
        // manager listed first, and a `_10_` document has no trailer and
        // nothing beside it to borrow one from, so reading it from the file
        // the reader put first is the difference between a capture that opens
        // either way round and one that opens only if it was picked in the
        // camera's own order (issue #123).
        let calibrated = calibrated(&files[0], player.size(), player.lenses())?;
        println!(
            "media:  {}{}, {}x{}, {:.3} fps, {} frames, {:.1} s",
            match player.lenses() {
                1 => "1 lens stream".to_owned(),
                n => format!("{n} lens streams"),
            },
            // Two files is a capture the camera wrote one lens per file and
            // the player paired at open (issue #79). Printed because it is
            // the one thing about an open file the pilot cannot otherwise
            // see: half a sphere and a whole one look the same until the
            // view is turned round.
            match player.files() {
                1 => String::new(),
                n => format!(" from {n} files"),
            },
            player.size().width,
            player.size().height,
            player.timing().fps(),
            player.timing().frames,
            player.timing().duration().as_secs_f64(),
        );
        // Opening a file plays it, which is what every player does. Space
        // and the control row's button pause it (issue #16).
        let frame = player.size();
        player.play();
        Ok(Self {
            show: Some(Show::new(
                files,
                frame,
                calibrated,
                None,
                Source::Live(Box::new(player)),
            )),
            ..Self::blank()
        })
    }

    /// One frame of a file, decoded on this thread. The headless
    /// instruments render with this, and it takes a [`Cue`] rather than
    /// always giving frame 0 because #8's Studio-diff harness needs to name
    /// the frame it is checking.
    pub fn still(path: &Path, at: Cue) -> Fallible<Self> {
        ours(path)?;
        let mut reader = Reader::open(path)?;
        let files: Arc<[PathBuf]> = reader.paths().into();
        let calibrated = calibrated(&files[0], reader.size(), reader.lenses())?;
        let frame = reader.size();
        let frames = reader.frame(at)?;
        println!(
            "frame:  {} at {:.3} s",
            frames.index,
            frames.timestamp.as_secs_f64()
        );
        for frame in &frames.lenses {
            println!("drm:    {}", frame.describe());
        }
        Ok(Self {
            show: Some(Show::new(
                files,
                frame,
                calibrated,
                Some(Arc::new(frames)),
                Source::Stepped(Box::new(reader)),
            )),
            ..Self::blank()
        })
    }

    /// What names the camera this file came off, which is what its seam
    /// calibration is stored under (issue #48). `None` with nothing open.
    pub fn camera_key(&self) -> Option<u64> {
        Some(self.show.as_ref()?.camera)
    }

    /// Whether this file has a seam at all: two lens streams, sampled from
    /// one body. A one-stream capture has nothing to hand over and nothing to
    /// calibrate.
    pub fn has_seam(&self) -> bool {
        self.show
            .as_ref()
            .is_some_and(|show| show.lenses.len() >= 2)
    }

    /// How wide this file hands the picture over, in degrees, or `None` for a
    /// capture with one lens stream, which has no seam to hand over at.
    ///
    /// **The width is the camera's and not the build's** since 2026-08-05: the
    /// projection asks for one number and this file's own overlap clamps it
    /// ([`Reframe::crossover_at`], `band::affordable`). An X4 Air takes the 8
    /// asked for; the owner's ONE X2 draws 3.99, and nothing else the app says
    /// would ever mention it.
    ///
    /// Read off the lenses the pass will draw with **now**, correction and all,
    /// because a seam fit moves the principal point, which moves each lens's
    /// coverage boundary, which moves the overlap: on that X2 the factory
    /// calibration affords 4.91 and its own pooled fit affords 3.99. So this is
    /// a reading and not a property of the file, and a fit landing later moves
    /// it - which is why [`fit_into`] says it again when one does.
    pub fn handover_deg(&self) -> Option<f32> {
        let show = self.show.as_ref()?;
        handover_deg(&show.lenses(), show.frame)
    }

    /// Draw this file with what the pool knows about its camera. Applied here
    /// and now, with no walk, so it is in the first frame.
    pub fn use_seam(&self, fit: SeamFit) {
        let Some(show) = &self.show else {
            return;
        };
        show.corrected.land(fit);
    }

    /// Draw this file with the along-seam table its camera has been read at
    /// (issue #103, stage 9).
    ///
    /// Landed rather than walked. A table is a calibration and the caller is
    /// expected to set it before the first frame, where there is no picture
    /// for it to jump.
    ///
    /// **That is a discipline and not a property of this function.** Called
    /// mid-play it lands whatever it is given in the next frame, and the
    /// picture steps by the whole of it. If a later stage ever re-answers a
    /// pool while a file is up, this needs [`Correction`]'s walk rather than
    /// this cell.
    pub fn use_table(&self, table: Table) {
        if let Some(show) = &self.show {
            show.table.set(table);
        }
    }

    /// Ask for this correction, walking to it rather than landing it. What a
    /// freshly pooled fit does to the file it was measured on: the picture is
    /// already up, so it must not jump.
    pub fn aim_seam(&self, fit: SeamFit) {
        if let Some(show) = &self.show {
            show.corrected.ask(fit);
        }
    }

    /// Fit this capture's seam from its own frames, best effort
    /// (`kjerag_render::seam`).
    ///
    /// The whole capture, which is every file the reader opened it from: a
    /// fit reading one file of a two-file capture finds one lens, and a
    /// capture with one lens has no seam, so it would refuse the very
    /// captures this exists for (issue #123).
    ///
    /// On its own thread for a file that is playing, because a fit is a
    /// second or two of decode and the picture is not waiting for it; on this
    /// one for a still, which has no later to correct itself in.
    ///
    /// A fit that lands while the file plays is **asked for** rather than
    /// landed: the picture walks to it over the next few seconds, because by
    /// then there is a picture to jump.
    /// `drive` puts the answer into the picture as well as into the pool,
    /// which is what a camera with nothing pooled needs. A camera that already
    /// has a pooled answer is drawing with it, and this file's own fit is a
    /// candidate for the next pooled answer rather than a picture of its own:
    /// the shell folds it in and asks for whatever the pool then answers.
    pub fn fit_seam(&self, drive: bool) {
        let Some(show) = &self.show else {
            return;
        };
        if show.lenses.len() < 2 {
            return;
        }
        if drive {
            println!(
                "seam:   nothing pooled for this camera yet, so it is fitted from this file, \
                 best effort, while it plays"
            );
        }
        let stepped = matches!(show.playing.borrow().source, Source::Stepped(_));
        let (files, lenses, frame) = (show.files.clone(), show.lenses.clone(), show.frame);
        let (corrected, kept) = (show.corrected.clone(), show.harvested.clone());
        let into = drive.then(|| corrected.clone());
        if stepped {
            fit_into(&files, &lenses, frame, into.as_ref(), &kept, true);
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("seam fit".to_owned())
            .spawn(move || fit_into(&files, &lenses, frame, into.as_ref(), &kept, false));
        if let Err(e) = spawned {
            eprintln!("kjerag: the seam fit did not start: {e}");
        }
    }

    /// What this file's own frames came to, for the pool to keep if it is good
    /// enough. `None` until a fallback fit has landed, and on a file whose
    /// camera the pool already knew, which fits nothing.
    ///
    /// **Taken, not read.** The shell asks on a timer as well as on the way
    /// out, because the way out is not always taken: `Ctrl+Q` is
    /// `std::process::exit(0)` and runs no shutdown. Taking it means the
    /// answer is folded into the pool exactly once however many times it is
    /// asked for, so the timer costs a lock and nothing else.
    pub fn seam_harvest(&self) -> Option<Harvest> {
        self.show.as_ref()?.harvested.lock().ok()?.take()
    }

    /// Take the next frame of a stepped scene, on this thread. `false` at the
    /// end of the file.
    ///
    /// The instrument that measures the horizon over a run of frames
    /// (issue #8) reads consecutive frames, and a seek per frame would cost
    /// it a keyframe walk each time.
    pub fn advance(&mut self) -> Fallible<bool> {
        let Some(show) = self.show.as_mut() else {
            return Ok(false);
        };
        let Playing { frames, source } = show.playing.get_mut();
        let Source::Stepped(reader) = source else {
            return Ok(false);
        };
        match reader.next_frames()? {
            Some(taken) => {
                *frames = Some(Arc::new(taken));
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// The frame on screen, for an instrument that needs to say which one it
    /// measured.
    pub fn frame(&self) -> Option<(u64, Duration)> {
        let show = self.show.as_ref()?;
        let playing = show.playing.borrow();
        let frames = playing.frames.as_ref()?;
        Some((frames.index, frames.timestamp))
    }

    /// Takes whichever frame belongs on screen at `now`, and says when to
    /// come back. Call it on every redraw: this is the presentation clock's
    /// only tick.
    pub fn pump(&self, now: Instant) -> Next {
        // Nothing open is nothing that changes by itself: no clock, no decode
        // thread, and a pane the shell paints. This asked for a redraw on
        // every compositor refresh while the pass carried an animation, which
        // was a window kept awake to draw a test pattern.
        let Some(show) = &self.show else {
            return Next::Never;
        };
        // Out of the cell in one step: the borrow checker splits the fields
        // of a `&mut Playing`, but not those of a `RefMut`.
        let Playing { frames, source } = &mut *show.playing.borrow_mut();
        let Source::Live(player) = source else {
            return Next::Never;
        };
        // The pass has been unable to put a frame on screen for long enough
        // that it has given up (issue #124). Pausing is what stops the sound
        // as well as the clock, because the sound follows the clock
        // (`kjerag_media`'s `Beat`), and a picture that died while the audio
        // played on is the whole of what that issue was.
        if let Some(stall) = self.stalled.take() {
            player.pause(now);
            return Next::Stopped(stall);
        }
        match player.pump(now) {
            Ok(None) => {}
            Ok(Some(taken)) => *frames = Some(taken),
            // The decode thread has stopped and will deliver nothing more, so
            // the answer is the same one: stop cleanly, and say so. This
            // printed a line and paused in silence until issue #124.
            Err(e) => {
                player.pause(now);
                return Next::Stopped(Stall::new(format_args!("playback stopped: {e}")));
            }
        }
        // The end of the file stops the clock rather than leaving it running
        // against frames that will never arrive.
        if player.is_ended() {
            player.pause(now);
            return Next::Never;
        }
        match (player.is_playing(), player.next_due()) {
            // A seek is outstanding: the picture is about to change even
            // though no clock is running towards it, so keep asking until it
            // does. This is what makes a scrub visible while paused, and
            // dragging the scrubber pauses.
            (false, _) if player.is_seeking() => Next::Refresh,
            (false, _) => Next::Never,
            (true, Some(due)) => Next::At(due),
            // Playing, but the clock has nothing to measure from yet: the
            // first frame is still being decoded.
            (true, None) => Next::Refresh,
        }
    }

    pub fn toggle_play(&mut self, now: Instant) {
        if let Some(player) = self.player_mut() {
            player.toggle(now);
        }
    }

    pub fn play(&mut self) {
        if let Some(player) = self.player_mut() {
            player.play();
        }
    }

    pub fn pause(&mut self, now: Instant) {
        if let Some(player) = self.player_mut() {
            player.pause(now);
        }
    }

    /// Move the picture, to a keyframe while a drag is still going and to the
    /// frame itself when it ends (issue #5).
    pub fn seek(&mut self, to: Duration, accuracy: Accuracy) {
        if let Some(player) = self.player_mut() {
            player.seek(Cue::Time(to), accuracy);
        }
    }

    /// One frame forward or back.
    pub fn step(&mut self, now: Instant, frames: i64) {
        if let Some(player) = self.player_mut() {
            player.step(now, frames);
        }
    }

    /// A seek has been asked for and has not landed. The shell keeps the
    /// picture redrawing while this is true.
    pub fn is_seeking(&self) -> bool {
        self.player(Player::is_seeking).unwrap_or(false)
    }

    pub fn is_playing(&self) -> bool {
        self.player(Player::is_playing).unwrap_or(false)
    }

    pub fn position(&self, now: Instant) -> Duration {
        self.player(|player| player.position(now))
            .unwrap_or_default()
    }

    /// How long the file runs, from the container: the frame count and the
    /// rational frame rate, divided.
    pub fn duration(&self) -> Duration {
        self.player(|player| player.timing().duration())
            .unwrap_or_default()
    }

    /// How many lenses the open capture is read as: two for a whole sphere,
    /// whether they came out of one file or two, one for half of one.
    ///
    /// The shell asks because half a sphere is the one thing about an open
    /// file the pilot cannot see. It looks like a whole one until the view is
    /// turned round (issue #123).
    pub fn lenses(&self) -> usize {
        self.player(Player::lenses).unwrap_or_default()
    }

    /// Whether this file has a sound track that a device took (issue #13).
    /// `false` is a file with no sound in it, or a box with no working output;
    /// the control row draws its volume button disabled for both.
    pub fn has_sound(&self) -> bool {
        self.player(Player::has_sound).unwrap_or(false)
    }

    /// Loudness, 0 to 1. A file with no sound takes it and does nothing.
    pub fn set_volume(&self, volume: f32) {
        self.player(|player| player.set_volume(volume));
    }

    /// Silence without stopping: the sound keeps running under a mute, so
    /// unmuting lands where the picture is rather than where it was.
    pub fn set_muted(&self, muted: bool) {
        self.player(|player| player.set_muted(muted));
    }

    /// Hide the pointer along with the controls, or bring both back.
    pub fn hide_cursor(&mut self, hidden: bool) {
        self.cursor_hidden = hidden;
    }

    pub fn is_cursor_hidden(&self) -> bool {
        self.cursor_hidden
    }

    /// Where the view points, and whether a drag has hold of it.
    pub fn viewpoint(&self) -> Viewpoint {
        self.viewpoint.get()
    }

    /// Move the view, and hand back whatever the move answered. The widget's
    /// mouse handling is the only caller: everything else asks through a
    /// [`Nudge`].
    pub(crate) fn steer<T>(&self, steer: impl FnOnce(&mut Viewpoint) -> T) -> T {
        let mut viewpoint = self.viewpoint.get();
        let answer = steer(&mut viewpoint);
        self.viewpoint.set(viewpoint);
        answer
    }

    /// Leave a view change for the widget to apply on its next redraw.
    pub fn nudge(&self, nudge: Nudge) {
        self.nudge.set(Some(nudge));
    }

    pub(crate) fn take_nudge(&self) -> Option<Nudge> {
        self.nudge.take()
    }

    /// Hold the picture against the world, or let it ride the camera
    /// (issue #8). Takes effect on the next redraw.
    pub fn set_horizon(&self, horizon: Horizon) {
        self.horizon.set(horizon);
    }

    pub fn horizon(&self) -> Horizon {
        self.horizon.get()
    }

    /// Whether this file carries the IMU record horizon lock needs. A file
    /// without one plays with the toggle on and the picture unheld.
    pub fn has_orientation(&self) -> bool {
        self.show
            .as_ref()
            .is_some_and(|show| !show.held.orientation.is_empty())
    }

    /// Which clock a frame's orientation is looked up on. The instrument that
    /// measured the choice moves this; the shell leaves it alone
    /// ([`FrameClock`]).
    pub fn set_frame_clock(&self, clock: FrameClock) {
        self.clock.set(clock);
    }

    /// Hold the picture at this orientation rather than the one the file's
    /// own IMU solves to, until it is set back to `None`.
    ///
    /// The harness's hook, and the reason it is here rather than in the
    /// harness: a deliberately wrong answer has to travel the **same** path
    /// to the shader as the right one, or what fails is the harness's own
    /// copy of the composition and not the thing under test (issue #8's
    /// negative control). Nothing in the shell calls this.
    pub fn hold_at(&self, world_from_body: Option<Quat>) {
        self.forced.set(world_from_body);
    }

    /// Read the sensor this way rather than the way the file describes, until
    /// it is set back to `None`. A [`Readout`] with a zero span is the
    /// rolling-shutter correction switched off.
    ///
    /// The same hook as [`Self::hold_at`] and for the same reason: issue #9's
    /// answer is which way the sensor reads, and the three wrong answers have
    /// to reach the shader by the path the right one takes, or what is
    /// measured is the harness. Nothing in the shell calls this.
    pub fn set_readout(&self, readout: Option<Readout>) {
        self.readout.set(readout);
    }

    /// How one frame comes off this file's sensor, for an instrument that has
    /// to say what it corrected for. `None` before a file is open.
    pub fn readout(&self) -> Option<Readout> {
        self.show.as_ref().map(|show| show.held.readout)
    }

    /// Sample the magnified picture this way rather than the way the player
    /// ships (issue #11). The same hook as [`Self::hold_at`], and the same
    /// reason: what a quality change is worth is the difference between two
    /// pictures, and the losing one has to come out of the same pass.
    /// Nothing in the shell calls this.
    pub fn set_sampling(&self, sampling: Sampling) {
        self.sampling.set(sampling);
    }

    pub fn stats(&self) -> Option<Stats> {
        self.player(Player::stats)
    }

    /// Reading the player needs the cell, so this hands it to a closure
    /// rather than handing out a reference into it.
    fn player<T>(&self, read: impl FnOnce(&Player) -> T) -> Option<T> {
        let show = self.show.as_ref()?;
        let playing = show.playing.borrow();
        match &playing.source {
            Source::Live(player) => Some(read(player)),
            Source::Stepped(_) => None,
        }
    }

    /// The player, for the calls that drive it: play, pause, seek, step.
    ///
    /// A capture the pass has given up on has none to hand out (issue #124).
    /// The sound follows the clock, so a play press that got through here
    /// would be sound over a picture that is not coming back, which is the
    /// symptom this whole issue is about. The transport goes quiet with the
    /// file it belongs to, and opening a file is the way on.
    fn player_mut(&mut self) -> Option<&mut Player> {
        if self.stalled.stopped() {
            return None;
        }
        match &mut self.show.as_mut()?.playing.get_mut().source {
            Source::Live(player) => Some(player),
            Source::Stepped(_) => None,
        }
    }

    /// Asks for a still of whatever the next redraw draws, at the size the
    /// request names. The pixels come back on a worker thread, through the
    /// request's own `then`; nothing here waits.
    pub fn capture(&self, request: Request) {
        self.shutter.arm(request);
    }

    /// The map this scene would draw one view through, for an instrument that
    /// wants to ask where a pixel is looking without opening a window.
    ///
    /// The same `Reframe` `prepare` builds, minus the frame it would be bound
    /// to: what it answers about is geometry, which the pictures do not
    /// change.
    pub fn mapped(&self, camera: Camera, aspect: f32) -> Option<Reframe> {
        let view = self.primitive(camera).view?;
        Some(
            Reframe::new(
                &view.lenses,
                view.frames.size,
                camera,
                view.held,
                aspect,
                false,
                self.sampling.get(),
            )
            .with_table(view.table),
        )
    }

    pub fn primitive(&self, camera: Camera) -> ScenePrimitive {
        let held = Holding {
            horizon: self.horizon.get(),
            clock: self.clock.get(),
            forced: self.forced.get(),
            readout: self.readout.get(),
        };
        ScenePrimitive {
            camera,
            view: self.show.as_ref().and_then(|show| show.view(held)),
            sampling: self.sampling.get(),
            shutter: self.shutter.clone(),
            stalled: self.stalled.clone(),
            shown: self.shown.clone(),
        }
    }
}

impl Show {
    fn new(
        files: Arc<[PathBuf]>,
        frame: Size,
        calibrated: Calibrated,
        frames: Option<Arc<Frames>>,
        source: Source,
    ) -> Self {
        Self {
            files,
            frame,
            corrected: Arc::new(Correction::none(&calibrated.lenses)),
            table: Cell::new(Table::REST),
            harvested: Harvested::default(),
            lenses: calibrated.lenses,
            camera: calibrated.camera,
            held: calibrated.held,
            playing: RefCell::new(Playing { frames, source }),
        }
    }

    /// What the pass runs on this redraw: the correction as it stands, which
    /// is one step further along its walk than it was on the last one.
    fn lenses(&self) -> Arc<[Lens]> {
        self.corrected.lenses()
    }

    fn view(&self, held: Holding) -> Option<View> {
        let frames = self.playing.borrow().frames.clone()?;
        let at = self.held.instant(&frames, held.clock);
        let world_from_body = held.forced.unwrap_or_else(|| self.held.orientation.at(at));
        Some(View {
            held: Held {
                body_from_world: match held.horizon {
                    Horizon::Locked => world_from_body.conjugate(),
                    Horizon::Free => Quat::IDENTITY,
                },
                // Not under the horizon toggle: the readout is the camera's
                // own motion during the frame, and a view that rides the body
                // has the same skew in it as one that does not.
                rolling: self
                    .held
                    .rolling(at, held.readout.unwrap_or(self.held.readout)),
            },
            lenses: self.lenses(),
            table: self.table.get(),
            frames,
        })
    }
}

/// A fallback fit off one capture's own frames, into the correction it will be
/// drawn with and the slot the pool reads it out of.
///
/// `land` for a still, which has no later moment to correct itself in, and
/// `ask` for a file that is playing, which does: by the time this returns
/// there is a picture on screen, and a picture that jumps is worse than a
/// picture that is briefly a degree out.
fn fit_into(
    files: &[PathBuf],
    lenses: &Arc<[Lens]>,
    frame: Size,
    into: Option<&Arc<Correction>>,
    kept: &Harvested,
    now: bool,
) -> Option<Harvest> {
    let started = Instant::now();
    let fitted = seam::fit_reported(files, lenses, frame, &seam::Plan::default())?;
    println!(
        "seam:   lens 1 roll {:+.3}, yaw {:+.3}, pitch {:+.3} deg, cx {:+.2}, cy {:+.2} px ({})",
        fitted.fit.roll_deg,
        fitted.fit.yaw_deg,
        fitted.fit.pitch_deg,
        fitted.fit.cx_px,
        fitted.fit.cy_px,
        fitted.describe(started.elapsed().as_secs_f64()),
    );
    if let Some(into) = into {
        match now {
            true => into.land(fitted.fit),
            false => into.ask(fitted.fit),
        }
        // A fit moves the principal point, which moves each lens's coverage
        // boundary, which moves how much the two of them overlap - and the
        // handover is clamped by that overlap (`band::affordable`). So a
        // fallback fit can change how wide this file hands over, seconds after
        // the shell already said how wide it was: on the owner's ONE X2 the
        // factory calibration affords 4.91 and this fit affords 3.99. Said only
        // when it moves, because it usually does not, and a line that repeats
        // itself is a line nobody reads.
        //
        // Off the fit APPLIED and not off the correction's own lenses: a fit
        // that is asked rather than landed walks in over a second, so the
        // correction is still showing the old calibration at this instant and
        // the width the picture is heading for is the one worth saying.
        let was = handover_deg(lenses, frame);
        let goes = handover_deg(&fitted.fit.applied(lenses), frame);
        if let (Some(was), Some(goes)) = (was, goes)
            && (goes - was).abs() >= 0.01
        {
            println!("blend:  that fit moves the handover: {was:.2} -> {goes:.2} deg");
        }
    }
    let harvest = Harvest {
        fit: fitted.fit,
        patches: fitted.patches,
        residual_deg: fitted.after[0].hypot(fitted.after[1]),
    };
    if let Ok(mut slot) = kept.lock() {
        *slot = Some(harvest);
    }
    Some(harvest)
}

/// How wide a camera with these lenses hands the picture over, in degrees, or
/// `None` where there is no seam to hand over at.
///
/// One place, because two callers need it at two moments: the shell at open,
/// and [`fit_into`] when a fit moves it. It reads the same
/// [`Reframe::crossover_at`] the pass reads, off the lenses it is handed, and
/// the aspect and the camera it builds the map with do not reach the answer.
fn handover_deg(lenses: &[Lens], frame: Size) -> Option<f32> {
    if lenses.len() < 2 {
        return None;
    }
    let mapped = Reframe::new(
        lenses,
        frame,
        Camera::default(),
        Held::default(),
        1.0,
        false,
        Sampling::default(),
    );
    Some(mapped.crossover_at(0.0).to_degrees())
}

/// Where a fallback fit leaves its answer for the shell to pool. Shared,
/// because the fit that fills it runs on a thread of its own.
type Harvested = Arc<Mutex<Option<Harvest>>>;

/// Everything the trailer contributes to one open capture: the calibration
/// for the lenses the shader samples, checked against the streams they will
/// be sampled from, and where the camera body went while it recorded.
///
/// One lens entry per decoded stream, and in the same order: the trailer
/// writes its lens blocks in the order the container carries the streams,
/// and a paired per-lens capture opens lens 0's file first, so the two
/// orders are the same one (`kjerag_meta::lens_index`).
///
/// **The calibration belongs to the capture, not to the file** (issue #79).
/// A camera that writes one lens per file writes one trailer for the pair
/// and keeps it with lens 0, so opening the second file reads the first
/// file's trailer. Before that, opening it failed outright with "file has no
/// Insta360 trailer". A per-lens file whose sibling is not on the card still
/// calibrates two lenses and decodes one, and then this is lens 0 alone and
/// the picture is one hemisphere, exactly as it was.
///
/// The orientation is integrated here, once, at open: a 30-minute X4 Air
/// capture is 1.8 million IMU samples and costs about a fifth of a second to
/// read and integrate, against 70 ms to open the container. Doing it per
/// frame would be 30 times a second for a track that does not change.
/// Another camera's 360 format is refused here, before the decoder is asked
/// for anything (issue #107).
///
/// The file it means opens perfectly well: a GoPro `.360` is a valid MP4 with
/// two HEVC tracks in it, so nothing downstream fails until the trailer read
/// finds no trailer, and "file has no Insta360 trailer" is what a corrupt
/// file says too. The shell turns this error into a line that names the
/// format instead.
///
/// A file nothing recognizes is not refused: the second file of an X2-class
/// pair carries no trailer and no maker's mark, and it is a file Kjerag plays
/// (`kjerag_meta::sibling`).
fn ours(path: &Path) -> Fallible<()> {
    match Format::sniff(path) {
        Format::Foreign(foreign) => Err(Box::new(foreign)),
        Format::Insta360 | Format::Unknown => Ok(()),
    }
}

fn calibrated(path: &Path, size: Size, streams: usize) -> Fallible<Calibrated> {
    let calibration = CalibrationSet::from_capture(path)?;
    // The calibration's pixel numbers are already in delivered-frame
    // coordinates, so they describe this texture only if the stream is the
    // size the trailer says it is. A mismatch reprojects at the wrong scale,
    // which reads as a mild lens error rather than a bug.
    if (calibration.dimension.width, calibration.dimension.height) != (size.width, size.height) {
        return Err(format!(
            "trailer says lens frames are {}x{} but the stream decodes {}x{}",
            calibration.dimension.width, calibration.dimension.height, size.width, size.height
        )
        .into());
    }
    let sampled = streams.min(MAX_LENSES);
    let lenses = calibration
        .lenses
        .get(..sampled)
        .ok_or_else(|| {
            format!(
                "file decodes {streams} lens streams but the trailer calibrates {}",
                calibration.lenses.len()
            )
        })?
        .to_vec();
    // The camera key is printed because it is what a seam calibration is
    // filed under, and a pilot with two cameras or a bug report to write has
    // no other way to see which one this file came off. It names the unit
    // without naming it: model and factory calibration, hashed, no serial.
    println!(
        "lens:   {} {}, sampling {sampled} of {} calibrated, camera {:016x}",
        calibration.camera_model,
        calibration.firmware,
        calibration.lenses.len(),
        calibration.camera_key(),
    );

    let orientation = calibration.orientation(Filter::default());
    println!(
        "imu:    {} samples at {:.0} Hz, {} orientations, axes {}",
        calibration.imu.samples().len(),
        calibration.imu.rate_hz(),
        orientation.samples().len(),
        calibration.gyro.imu_orientation,
    );
    let held = Motion {
        orientation,
        exposure: calibration.exposure[0].clone(),
        readout: calibration.readout(),
    };
    Ok(Calibrated {
        lenses: lenses.into(),
        camera: calibration.camera_key(),
        held: Arc::new(held),
    })
}

/// Everything one open capture's trailer contributes, in one piece so that
/// opening a file hands it over in one piece.
struct Calibrated {
    lenses: Arc<[Lens]>,
    camera: u64,
    held: Arc<Motion>,
}

/// What the shell hands the renderer for one frame.
#[derive(Debug)]
pub struct ScenePrimitive {
    camera: Camera,
    view: Option<View>,
    /// How the pass samples a magnified picture, which is a property of the
    /// redraw rather than of the frame in it.
    sampling: Sampling,
    /// A handle on the [`Scene`]'s shutter, not a copy of it: the request
    /// is taken by whichever redraw reaches [`ScenePipeline::prepare`]
    /// first, and one that never does is still armed for the next.
    shutter: Shutter,
    /// A handle on the [`Scene`]'s stall slot, the same way and for the same
    /// reason, in the other direction: the pass writes and the shell reads
    /// (issue #124).
    stalled: Stalled,
    /// And on the slot the pass keeps the last frame it drew of this capture
    /// in, which it both writes and reads.
    shown: Shown,
}

/// A pair of decoded lenses and the calibration that reprojects them. Both
/// halves are shared, so a redraw that changes nothing but the camera costs
/// two atomic increments.
#[derive(Clone, Debug)]
struct View {
    lenses: Arc<[Lens]>,
    /// What the along-seam axis still disagrees by after the pose, direction
    /// by direction (issue #103, stage 9). Part of this camera's calibration
    /// and carried with it, like the lenses above.
    table: Table,
    frames: Arc<Frames>,
    /// Where the body was when these frames were taken, already inverted for
    /// the pass. Identity with the lock off.
    held: Held,
}

/// The last view the pass actually presented of one capture, which is what the
/// pane holds while a newer one cannot be imported (issue #124).
///
/// It belongs to the capture rather than to the pipeline, for the reason the
/// whole issue is about: iced keeps one pipeline for the life of the window,
/// so whatever it remembers about this file it remembers about the next one.
/// Kept on the pipeline for one commit, this opened a new file onto the last
/// frame of the file before it, until the first frame of the new one arrived.
/// Issue #125's check is what caught it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Shown(Arc<Mutex<Option<View>>>);

impl Shown {
    fn keep(&self, view: &View) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(view.clone());
        }
    }

    fn get(&self) -> Option<View> {
        self.0.lock().ok()?.clone()
    }
}

/// The GPU state behind the widget. iced builds one of these per primitive
/// type and keeps it for the life of the renderer.
pub struct ScenePipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    /// The seam band, measured on the frame the draw is about to sample and
    /// dispatched from `prepare` (issue #103).
    band: Band,
    /// One black pixel, bound wherever a lens has no stream: before a file is
    /// open, and in the second slot of a file that carries one lens.
    blank: Planes,
    bind_group: wgpu::BindGroup,
    /// The frame the bind group points at, and the ones still in flight
    /// behind it. Newest first.
    live: VecDeque<Live>,
    /// The target this pass was built for, which is iced's surface format.
    /// A capture renders into a texture of the same format, so that what it
    /// reads back is what the compositor would have been handed.
    format: wgpu::TextureFormat,
    reported: bool,
}

/// One frame on the GPU. The mapped frames must outlive the textures
/// imported from them, and both must outlive the passes that read them,
/// which is what [`RETAINED`] is about.
struct Live {
    frames: Arc<Frames>,
    _planes: Vec<Planes>,
}

/// The compute half of the seam: the pipeline that measures the overlap band,
/// the state it accumulates into, and where in the film the state is.
///
/// It is dispatched from [`ScenePipeline::prepare`] and never from `draw`,
/// which is handed a render pass it cannot leave. The submit order is what
/// makes that correct and it is the same rule the capture already relies on:
/// a submit from `prepare` lands before iced's own submit of the pass that
/// reads its result.
struct Band {
    pipeline: wgpu::ComputePipeline,
    /// The second dispatch of the same pass: what the ring just read, pooled
    /// into one exposure for the picture (issue #103, stage 3).
    pool: wgpu::ComputePipeline,
    /// The along-seam field fitted over the whole ring, dispatched beside the
    /// exposure pooling and over the same cells (issue #103, stage 5).
    pool_along: wgpu::ComputePipeline,
    /// One [`band::Cell`] per direction, read by the draw and written here.
    state: wgpu::Buffer,
    watch: wgpu::Buffer,
    /// The same buffer twice: writable for the dispatch, read-only for the
    /// draw. Two groups over one buffer, and never both in one pass.
    group: wgpu::BindGroup,
    read: wgpu::BindGroup,
    /// Set by an instrument to stop measuring (`ScenePipeline::hold_band`).
    held: bool,
    /// Set by an instrument to leave the exposure alone
    /// (`ScenePipeline::hold_tone`). The ring is still measured and the bend
    /// is still applied; only the pooling is not dispatched, so the header
    /// stays at the zero it was created in and `tone_split` returns exactly
    /// one on both sides. That is the picture stage 2 drew, and it is how a
    /// before and after differ by this stage and by nothing else.
    tone_held: bool,
    /// Which round of the circle the next frame reads.
    /// How many times the measurement is dispatched per redraw. One, except
    /// under [`ScenePipeline::band_repeats`].
    repeats: u32,
    slice: u32,
    /// Where the last measured frame sat in the film, so the next one knows
    /// how much media time the state has aged by, and whether what happened
    /// in between was play or a seek.
    at: Option<Duration>,
}

impl ScenePipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            // In dependency order, so nothing is used before it is declared:
            // the map and its uniform block, then the band's lookup into it,
            // then the sampling, then this file's own entry points.
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}\n{}\n{SHADER}",
                    projection::wgsl(),
                    band::lookup_wgsl(),
                    sampling::wgsl(),
                )
                .into(),
            ),
        });
        let layout = bind_group_layout(device);
        // Two groups: the pictures and the map, then the band's state. iced's
        // device is asked for a limit of exactly two (`iced_wgpu`), so this is
        // all of them, and there is nowhere for a third to go.
        let reading = read_layout(device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[&layout, &reading],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                // Blending, which the pass did without until issue #100: the
                // picture writes alpha 1 and replaces exactly what a
                // replacing pipeline replaced, and the room around the ball
                // writes alpha 0 and leaves whatever the shell drew behind
                // the widget exactly as it found it. Premultiplied, which is
                // what the room's own colour already is: black at alpha 0.
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            // iced's own pass has one sample and no depth attachment
            // (`iced_wgpu/src/lib.rs`, "iced_wgpu render pass"); a pipeline
            // that disagrees fails to draw rather than looking wrong.
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene"),
            size: std::mem::size_of::<Reframe>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let blank = blank_planes(device);
        let band = Band::new(device, &layout);
        let bind_group = bind(device, &layout, &uniforms, [&blank; MAX_LENSES], &sampler);

        Self {
            pipeline,
            layout,
            sampler,
            uniforms,
            band,
            blank,
            bind_group,
            live: VecDeque::new(),
            format,
            reported: false,
        }
    }

    /// The band, measured on the pair the bind group points at, before the
    /// draw that will read what it wrote.
    ///
    /// One dispatch, one submit, no readback: the state lives on the GPU for
    /// its whole life, so nothing here waits on a fence and nothing stalls the
    /// pipeline. The reason it is here and not in `draw` is that `draw` is
    /// handed a render pass and cannot open a compute one; the reason that is
    /// **correct** is the same rule the capture already relies on, written at
    /// the uniform write above: a submit from `prepare` lands before iced's
    /// own submit of the pass that reads its result.
    fn measure(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, view: Option<&View>) {
        // A file with one lens stream has no seam, and a redraw with no new
        // frame has nothing new to read. The state stays where it is, which
        // for a one-lens file is the zero it was created in.
        let Some(view) =
            view.filter(|view| !self.band.held && view.lenses.len() > 1 && self.is_bound(view))
        else {
            return;
        };
        let Some(watch) = self.band.aged(view.frames.timestamp) else {
            return;
        };
        queue.write_buffer(&self.band.watch, 0, watch.bytes());
        let mut encoder = device.create_command_encoder(&Default::default());
        // Only ever more than one under `band_repeats`, and then each in a pass
        // of its own: dispatches inside one pass have no barrier between them
        // and a device is free to overlap them, which measures throughput where
        // what is wanted is one redraw's worth of latency.
        for _ in 1..self.band.repeats {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("band repeat"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.band.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(1, &self.band.group, &[]);
            pass.dispatch_workgroups(watch.groups(), 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("band"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.band.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(1, &self.band.group, &[]);
            pass.dispatch_workgroups(watch.groups(), 1, 1);
            // One workgroup over the state the dispatch above just wrote.
            // Two dispatches of one pass are ordered against each other by
            // WebGPU itself, so what this pools is this frame's readings and
            // no barrier has to be asked for.
            if !self.band.tone_held {
                pass.set_pipeline(&self.band.pool);
                pass.dispatch_workgroups(1, 1, 1);
            }
            pass.set_pipeline(&self.band.pool_along);
            pass.dispatch_workgroups(1, 1, 1);
        }
        queue.submit([encoder.finish()]);
    }

    /// iced picks an sRGB surface when it gamma-corrects, and the GPU then
    /// encodes whatever the shader writes. Video is already gamma-encoded,
    /// so it has to be decoded back to linear first or the picture washes
    /// out.
    fn linearize(&self) -> bool {
        self.format.is_srgb()
    }

    /// `aspect` is the output's width over its height, which is what decides
    /// the vertical field of view. The widget reads it from its bounds.
    pub fn prepare(
        &mut self,
        primitive: &ScenePrimitive,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        aspect: f32,
    ) {
        if !self.reported {
            self.reported = true;
            println!("device: {}", dmabuf::device_report(device));
        }
        if let Some(view) = &primitive.view {
            self.show(device, view, primitive);
        }

        // The pane holds the last frame this pipeline actually presented
        // whenever the one it is offered is not on the GPU (issue #124).
        // Frames keep arriving while imports fail, so drawing strictly by
        // what the shell offers takes the picture away for as long as the
        // failures last and leaves it away once the capture is stopped, which
        // is a second failure on top of the first from where the pilot sits.
        // Owner: "Why does the video disappear instead of just freezing on
        // current frame though? Its jarring".
        //
        // Every gap rather than only the stopped one, because a hiccup that
        // costs frames should cost frames: a squeeze under the bound took the
        // pane to the backdrop and back before this.
        let showing = match &primitive.view {
            Some(view) if self.is_bound(view) => primitive.view.clone(),
            // Nothing ever presented is the one case with nothing to hold,
            // and the pane is the backdrop. `Stalled` says so in the terminal
            // line when it gives up, because on screen it looks like the
            // other kind of failure.
            _ => primitive.shown.get(),
        };

        let reframe = match &showing {
            Some(view) if self.is_bound(view) => Reframe::new(
                &view.lenses,
                view.frames.size,
                primitive.camera,
                view.held,
                aspect,
                self.linearize(),
                primitive.sampling,
            )
            .with_table(view.table),
            // No frame yet, or none this pipeline has managed to bind: the
            // pane is all room, which the shell's backdrop shows through.
            _ => Reframe::blank(aspect, self.linearize()),
        };
        queue.write_buffer(&self.uniforms, 0, reframe.bytes());
        // After the uniform write, because the band reads the same block: the
        // calibration it measures against has to be the one the draw will use,
        // or the two disagree by whatever the correction walked this redraw.
        self.measure(device, queue, showing.as_ref());

        // After the uniform write, and only after it: the write lands at the
        // next submit on this queue, and the capture's own submit is that
        // one. Taken here rather than in `draw` because this is the call
        // that has a device to render with.
        if let Some(request) = primitive.shutter.take() {
            self.shoot(device, queue, request, aspect, showing.as_ref());
        }
    }

    /// Draws the view a second time, offscreen, at the size the capture
    /// asked for. Everything after the submit is the worker thread's
    /// (`super::capture`).
    fn shoot(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        request: Request,
        aspect: f32,
        view: Option<&View>,
    ) {
        let at = view.map_or_else(Stamp::default, |view| Stamp {
            index: view.frames.index,
            time: view.frames.timestamp,
        });
        capture::deliver(
            self.expose(device, queue, request.width, aspect, at),
            request.then,
        );
    }

    /// The render-thread half: a target, one pass into it, and the copy that
    /// will be read back. The frame it samples is the one the bind group
    /// already points at, and `RETAINED` is what keeps that frame's decoder
    /// surfaces alive long enough for this pass to finish: three frames of
    /// slack against a pass that costs a few milliseconds.
    fn expose(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        aspect: f32,
        at: Stamp,
    ) -> Fallible<Pending> {
        let order = Order::of(self.format)?;
        let size = capture::fitted(width, aspect)?;
        let stride = capture::stride(size.width);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture"),
            size: size.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture"),
            size: u64::from(stride) * u64::from(size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let view = texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("capture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.draw(&mut pass);
        }
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(size.height),
                },
            },
            size.extent(),
        );

        Ok(Pending {
            device: device.clone(),
            _texture: texture,
            readback,
            submission: queue.submit([encoder.finish()]),
            size,
            stride,
            order,
            at,
        })
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(1, &self.band.read, &[]);
        pass.draw(0..3, 0..1);
    }

    fn is_bound(&self, view: &View) -> bool {
        self.live
            .front()
            .is_some_and(|live| Arc::ptr_eq(&live.frames, &view.frames))
    }

    /// Stop measuring the band, which leaves its state where it is and, on a
    /// pipeline that has never measured, leaves it at the zero that bends
    /// nothing: exactly the picture stage 1 drew.
    ///
    /// **Nothing in the player calls this** and no key reaches it (AGENTS.md,
    /// zero-config playback). It exists so `kjerag-spike --bin band` can draw
    /// the same frame both ways through the same pipeline, which is the only
    /// way a before and after differ by the band and by nothing else.
    pub fn hold_band(&mut self, held: bool) {
        self.band.held = held;
    }

    /// Stop pooling the exposure, which leaves the gain at exactly one and
    /// draws the picture stage 2 drew (issue #103, stage 3).
    ///
    /// The same instrument-only switch as [`Self::hold_band`] and for the same
    /// reason: a before and after have to differ by one thing. The band is
    /// still measured and the bend still applied, so what moves between the
    /// two renders is the exposure alone.
    pub fn hold_tone(&mut self, held: bool) {
        self.band.tone_held = held;
    }

    /// Dispatch the measurement this many times per redraw instead of once,
    /// so its cost can be read as a SLOPE (issue #103, stage 6).
    ///
    /// The same instrument-only switch as [`Self::hold_band`] and for a reason
    /// of the same kind. A redraw's wall time on a box with other work on it is
    /// the pass plus whatever else ran, and on this box that second term is
    /// wider than the first: six alternating runs of two builds under a load
    /// average of 21 came back 5.1 to 20.3 ms with the builds interleaved. The
    /// noise is ADDITIVE and the pass is not, so `n` dispatches of it minus one
    /// dispatch of it, over `n - 1`, is the pass with the box divided out.
    ///
    /// Nothing in the player calls this and no key reaches it.
    pub fn band_repeats(&mut self, times: u32) {
        self.band.repeats = times.max(1);
    }

    /// The band as it stands, for an instrument. `None` where the device
    /// cannot map a buffer back, which is not a case the player has.
    ///
    /// Nothing in the player calls this: the state's whole life is on the GPU
    /// and a readback would be a stall. It exists so `kjerag-spike --bin band`
    /// can print what the pass is drawing with.
    pub fn band_state(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Fallible<(band::Along, Vec<band::Cell>)> {
        let (_, along, cells) = self.band.read(device, queue)?;
        Ok((along, cells))
    }

    /// The pooled exposure the pass is drawing with, for an instrument
    /// (issue #103, stage 3). Same readback, same caveat: a stall, and no
    /// shipped path takes it.
    pub fn band_tone(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Fallible<band::Tone> {
        Ok(self.band.read(device, queue)?.0)
    }

    /// Imports a newly delivered pair and points the bind group at it. A
    /// redraw that shows the same pair again does nothing here.
    ///
    /// A failed import costs this frame and no more (issue #124). The next
    /// redraw tries again; what gives up is [`Stalled`], on a run of failures
    /// that lasts, and the pilot hears about it from the shell rather than
    /// from a terminal.
    ///
    /// Once it has given up, this stops trying, and that is not an
    /// optimisation. The view that failed is never bound, so every redraw
    /// after it would try the same import again, and each two seconds of that
    /// raised another alert: the owner met five of them in one sitting.
    fn show(&mut self, device: &wgpu::Device, view: &View, primitive: &ScenePrimitive) {
        if primitive.stalled.stopped() || self.is_bound(view) {
            return;
        }
        match self.import(device, view) {
            Ok(planes) => {
                primitive.stalled.landed();
                self.bind_group = bind(
                    device,
                    &self.layout,
                    &self.uniforms,
                    // A file with one lens stream leaves the second slot on
                    // the blank pixel. Nothing samples it: `Reframe`'s lens
                    // count says one, and a bind group still needs an entry
                    // for every binding the layout declares.
                    std::array::from_fn(|lens| planes.get(lens).unwrap_or(&self.blank)),
                    &self.sampler,
                );
                self.live.push_front(Live {
                    frames: view.frames.clone(),
                    _planes: planes,
                });
                self.live.truncate(RETAINED);
                primitive.shown.keep(view);
            }
            Err(e) => {
                primitive
                    .stalled
                    .failed(Instant::now(), e, primitive.shown.get().is_some());
            }
        }
    }

    /// Every lens of the pair: both are sampled, one per output pixel.
    fn import(&self, device: &wgpu::Device, view: &View) -> Fallible<Vec<Planes>> {
        view.frames
            .lenses
            .iter()
            .map(|frame| dmabuf::import(device, frame.descriptor(), view.frames.size))
            .collect()
    }
}

impl Band {
    fn new(device: &wgpu::Device, scene: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("band"),
            // The same map the draw runs, so the band correlates directions
            // through the calibration the picture is drawn with rather than
            // through a second copy of it.
            source: wgpu::ShaderSource::Wgsl(
                format!("{}\n{}", projection::wgsl(), band::wgsl()).into(),
            ),
        });
        let layout = band_layout(device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("band"),
            bind_group_layouts: &[scene, &layout],
            immediate_size: 0,
        });
        let reading = read_layout(device);
        let compute = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("band"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let pipeline = compute("measure");
        let pool = compute("pool");
        let pool_along = compute("pool_along");
        let state = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("band"),
            size: band::BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            // Zeroed, and zero is the state that bends nothing: a file's first
            // frame is drawn exactly as stage 1 drew it.
            mapped_at_creation: false,
        });
        let watch = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("band"),
            size: std::mem::size_of::<band::Watch>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("band"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: band::STATE_BINDING,
                    resource: state.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: band::WATCH_BINDING,
                    resource: watch.as_entire_binding(),
                },
            ],
        });
        let read = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("band read"),
            layout: &reading,
            entries: &[wgpu::BindGroupEntry {
                binding: band::STATE_BINDING,
                resource: state.as_entire_binding(),
            }],
        });
        Self {
            pipeline,
            pool,
            pool_along,
            state,
            watch,
            group,
            read,
            held: false,
            tone_held: false,
            repeats: 1,
            slice: 0,
            at: None,
        }
    }

    /// How much media time the state has aged by, or `None` for a frame it has
    /// already read.
    ///
    /// Media time and not wall clock, so a paused window does not age the
    /// state, a slow box does not smooth harder than a fast one, and the same
    /// second of film settles the same way at 24 fps and at 60. A gap that is
    /// not a play forward is a reset: what the state holds is an average over
    /// what the seam has been showing, and after a seek that is somewhere
    /// else.
    fn aged(&mut self, now: Duration) -> Option<band::Watch> {
        let before = self.at.replace(now);
        let seconds = before.map(|then| now.as_secs_f32() - then.as_secs_f32());
        let slice = self.slice;
        self.slice = (slice + 1) % band::ROUNDS;
        match seconds {
            Some(0.0) => None,
            Some(seconds) if (0.0..band::Watch::GAP_S).contains(&seconds) => {
                Some(band::Watch::track(seconds, slice))
            }
            // The first frame of a file, and every landing after a seek. The
            // step it is given is one frame's worth, so a direction with
            // content in it starts moving immediately rather than waiting a
            // frame for a gap to exist, and it sweeps the WHOLE ring rather
            // than the slice it happened to land on, because what it is
            // throwing away is per direction (`band::Watch::stride`).
            _ => Some(band::Watch::start(1.0 / 30.0)),
        }
    }

    /// The state copied back to the CPU. For instruments only: see
    /// [`ScenePipeline::band_state`].
    fn read(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Fallible<(band::Tone, band::Along, Vec<band::Cell>)> {
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("band"),
            size: band::BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&self.state, 0, &readback, 0, band::BYTES);
        let submission = queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        let mapped = slice.get_mapped_range();
        let float = |at: usize| {
            f32::from_ne_bytes([mapped[at], mapped[at + 1], mapped[at + 2], mapped[at + 3]])
        };
        let tone = band::Tone::read(float(0), float(4));
        let along = band::Along::read(
            std::array::from_fn(|term| float(band::ALONG_AT + 4 * term)),
            float(band::ALONG_AT + 20),
        );
        let cells = (0..band::AZIMUTHS)
            .map(|index| {
                let at = band::CELLS_AT + index * std::mem::size_of::<band::Cell>();
                band::Cell {
                    disparity: float(at),
                    confidence: float(at + 4),
                    reach_m: float(at + 8),
                    off_epi: float(at + 12),
                    off_conf: float(at + 16),
                    tone: float(at + 20),
                    lit: float(at + 24),
                }
            })
            .collect();
        drop(mapped);
        readback.unmap();
        Ok((tone, along, cells))
    }
}

/// The uniform block, then each lens's luma and chroma planes in lens order,
/// then the sampler they share. The shader names those textures rather than
/// indexing them, so the count is [`MAX_LENSES`] on both sides.
fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    lenses: [&Planes; MAX_LENSES],
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    // The views have to outlive the descriptor that borrows them, so they are
    // built before it rather than inside it.
    let views: Vec<wgpu::TextureView> = lenses
        .iter()
        .flat_map(|planes| [&planes.luma, &planes.chroma])
        .map(|texture| texture.create_view(&Default::default()))
        .collect();
    let mut entries = vec![wgpu::BindGroupEntry {
        binding: 0,
        resource: uniforms.as_entire_binding(),
    }];
    entries.extend(
        views
            .iter()
            .enumerate()
            .map(|(plane, view)| wgpu::BindGroupEntry {
                binding: 1 + plane as u32,
                resource: wgpu::BindingResource::TextureView(view),
            }),
    );
    entries.push(wgpu::BindGroupEntry {
        binding: SAMPLER_BINDING,
        resource: wgpu::BindingResource::Sampler(sampler),
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene"),
        layout,
        entries: &entries,
    })
}

fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    // The uniform block and the pictures are read by both passes: the draw
    // samples them and the band correlates them (issue #103). The state buffer
    // at the end is the draw's alone here, read-only; the compute pass reaches
    // the same buffer through a group of its own, where it is writable.
    let both = wgpu::ShaderStages::FRAGMENT.union(wgpu::ShaderStages::COMPUTE);
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: both,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: both,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            // The one place the Rust and WGSL definitions of the uniform
            // block are checked against each other: pipeline creation fails
            // if the shader's struct wants more bytes than `Reframe` has.
            min_binding_size: NonZeroU64::new(std::mem::size_of::<Reframe>() as u64),
        },
        count: None,
    }];
    entries.extend((1..SAMPLER_BINDING).map(texture));
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: SAMPLER_BINDING,
        visibility: both,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    });
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("scene"),
        entries: &entries,
    })
}

/// The state buffer alone, as the draw sees it: read-only, on a group of its
/// own (see [`band::STATE_BINDING`]).
fn read_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("band read"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: band::STATE_BINDING,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(band::BYTES),
            },
            count: None,
        }],
    })
}

/// What the band writes and what it is told about this frame. A group of its
/// own so the same buffer can be read-only on the draw's side.
fn band_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("band"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: band::STATE_BINDING,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(band::BYTES),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: band::WATCH_BINDING,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<band::Watch>() as u64),
                },
                count: None,
            },
        ],
    })
}

/// The shader samples its planes unconditionally, so something has to be bound
/// before a file is. One black pixel each.
fn blank_planes(device: &wgpu::Device) -> Planes {
    let make = |format| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blank"),
            size: Size::new(1, 1).extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };
    Planes {
        luma: make(wgpu::TextureFormat::R8Unorm),
        chroma: make(wgpu::TextureFormat::Rg8Unorm),
    }
}

/// The half of the shader that belongs to this file. `projection::WGSL`
/// declares the uniform block, the view ray and the forward map, and is
/// concatenated ahead of this.
const SHADER: &str = r#"
struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  let x = f32((i << 1u) & 2u);
  let y = f32(i & 2u);
  var out: VsOut;
  out.uv = vec2<f32>(x, y);
  out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  return out;
}

// Two planes per lens, in lens order, then the sampler they share. WGSL has
// no texture array to index here, so `picture` branches on the lens the ray
// picked; `SAMPLER_BINDING` is the Rust half of these numbers.
@group(0) @binding(1) var luma0: texture_2d<f32>;
@group(0) @binding(2) var chroma0: texture_2d<f32>;
@group(0) @binding(3) var luma1: texture_2d<f32>;
@group(0) @binding(4) var chroma1: texture_2d<f32>;
@group(0) @binding(5) var samp: sampler;

// Each lens's picture at that ray, mixed by its weight, or the room where no
// lens has the ray. A lens weighted zero is not sampled at all, so outside the
// overlap this is the single fetch the hard pick took before issue #7, and
// the second fetch is what the blend band costs.
//
// The alpha rides back with the colour: 1 for a picture, and the room's own
// for the room, which is what the target then blends by.
//
// `ratio` is each lens's magnification at its own landing, in delivered-frame
// texels per output pixel (issue #11). It arrives as an argument because the
// derivatives it comes from have to be read where the control flow is
// uniform, which is the entry point and not here.
//
// WGSL has no texture array to index here, so the lenses are named rather
// than looped. The explicit mip level is what makes that legal: a
// `textureSample` computes its own level from derivatives and needs uniform
// control flow to do it, and every one of these textures has a single level
// anyway.
fn picture(mix: Blend, ratio: vec2<f32>) -> vec4<f32> {
  var rgb = vec3<f32>(0.0);
  var total = 0.0;
  // What the two lenses' exposures have to be brought together by, split
  // between them (issue #103, stage 3). One uniform read for the whole draw,
  // and exactly 1.0 on both sides until something has been measured, so the
  // weights below are the weights this pass has always used and a picture
  // with no reading behind it is the picture stage 2 drew.
  let tone = tone_split();
  if mix.weights[0] > 0.0 {
    rgb += (mix.weights[0] * tone.x) * nv12(luma0, chroma0, frame_uv(mix.landings[0].pixel), ratio.x);
    total += mix.weights[0];
  }
  if mix.weights[1] > 0.0 {
    rgb += (mix.weights[1] * tone.y) * nv12(luma1, chroma1, frame_uv(mix.landings[1].pixel), ratio.y);
    total += mix.weights[1];
  }
  // The room around the ball, written rather than painted: transparent black,
  // which through the pass's premultiplied blend leaves what is under the
  // widget alone (issue #100). Black is what makes it premultiplied; what
  // fills the room is the shell's business and not this pass's.
  return select(vec4<f32>(0.0), vec4<f32>(rgb, 1.0), total > 0.0);
}

// BT.709 full range: ffprobe reports bt709 and the camera writes yuvj420p.
// DRM_FORMAT_GR88 is little endian G:R, so .r is Cb and .g is Cr.
//
// The two planes are handed the same magnification and reach their own
// conclusions from it, because they are not the same size: `plane` scales the
// ratio by the grid it is sampling (`sampling::plane_ratio`), so the chroma
// plane upgrades an octave of zoom before the luma plane does.
fn nv12(luma: texture_2d<f32>, chroma: texture_2d<f32>, uv: vec2<f32>, ratio: f32) -> vec3<f32> {
  let y = plane(luma, samp, uv, ratio, reframe.sharpen_luma).r;
  let c = plane(chroma, samp, uv, ratio, reframe.sharpen_chroma).rg - vec2<f32>(0.5, 0.5);
  return vec3<f32>(
    y + 1.5748 * c.g,
    y - 0.1873 * c.r - 0.4681 * c.g,
    y + 1.8556 * c.r,
  );
}

fn linearize(c: vec3<f32>) -> vec3<f32> {
  let lo = c / 12.92;
  let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
  return select(lo, hi, c > vec3<f32>(0.04045));
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let look = view_ray(in.uv);
  // Zero, which is every weight zero: the room around the ball at the far end
  // of the zoom (issue #47) is a fragment no lens has, and `picture` already
  // paints that. Nothing is sampled for it and no model is run.
  var mix: Blend;
  if look.w > 0.0 {
    mix = blend(look.xyz, band_bend(look.xyz));
  }
  // Here rather than inside the blend: a derivative has to be taken where
  // every lane of the quad is running, and the blend is all branches. What
  // the neighbouring lanes landed on is exactly what this asks about, so the
  // landings are read after the blend has answered for all of them.
  let ratio = vec2<f32>(
    texel_ratio(mix.landings[0].pixel),
    texel_ratio(mix.landings[1].pixel),
  );
  let lens = picture(mix, ratio);
  return vec4<f32>(
    select(lens.rgb, linearize(lens.rgb), reframe.linearize > 0.5),
    lens.a,
  );
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use kjerag_meta::{Filter, GyroSample, GyroTrack, Sweep};

    /// A camera rolling at a constant rate, as an orientation track: enough
    /// for the one question this module owns, which is whether a frame's
    /// readout reaches the pass.
    fn turning(rate_dps: f64) -> OrientationTrack {
        let samples = (0..2_000)
            .map(|index| GyroSample {
                offset_us: index * 1_000,
                rate_dps: [0.0, 0.0, rate_dps],
                accel_g: [0.0, -1.0, 0.0],
            })
            .collect();
        Filter::default().solve(
            &GyroTrack::from_samples(samples),
            kjerag_meta::Mat3::IDENTITY,
        )
    }

    fn motion(orientation: OrientationTrack) -> Motion {
        Motion {
            orientation,
            exposure: ExposureTrack::default(),
            readout: Readout {
                seconds: 0.015_883,
                sweep: Sweep::Right,
            },
        }
    }

    /// The whole of issue #9's "and if there is no gyro": a file with no IMU
    /// record has nothing to correct with, so the pass is handed no readout at
    /// all rather than a zero one, and it runs as it did before.
    #[test]
    fn a_file_with_no_gyro_track_gets_no_readout() {
        let held = motion(OrientationTrack::default());

        assert_eq!(held.rolling(1_000_000, held.readout), None);
    }

    /// And a camera whose readout direction has not been measured is the same
    /// case, which today is everything that is not an X4: `Sweep::Unknown` is
    /// a zero axis and there is nothing to apply it along.
    #[test]
    fn an_unknown_sweep_gets_no_readout() {
        let held = motion(turning(90.0));
        let unknown = Readout {
            sweep: Sweep::Unknown,
            ..held.readout
        };

        assert_eq!(held.rolling(1_000_000, unknown), None);
        assert!(held.rolling(1_000_000, held.readout).is_some());
    }

    /// With both, the turn handed to the pass is the one the body made across
    /// that frame's readout, centred on the frame's own instant: 90 deg/s
    /// through 15.883 ms is 1.43 degrees, about the body's forward axis.
    #[test]
    fn a_readout_carries_the_turn_the_body_made_during_it() {
        let held = motion(turning(90.0));
        let rolling = held.rolling(1_000_000, held.readout).expect("no readout");

        let turn = rolling.turn[2].to_degrees();
        assert!(
            (turn + 1.43).abs() < 0.05,
            "{turn} degrees across the readout"
        );
        assert_eq!(rolling.axis, [1.0, 0.0]);
    }
}
