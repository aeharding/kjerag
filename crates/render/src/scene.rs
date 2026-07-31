//! The one shader pass the player draws.
//!
//! With no file it is an animated gradient, which proves only that a custom
//! wgpu pass runs inside libcosmic. With a file it reprojects a real VA-API
//! frame imported by [`super::dmabuf`]: for every output pixel, a view ray
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
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kyerag_media::{Accuracy, Cue, Frames, Player, Reader, Stats};
use kyerag_meta::{CalibrationSet, ExposureTrack, Filter, Lens, OrientationTrack, Quat, Readout};

use super::capture::{self, Order, Pending, Request, Shutter, Stamp};
use super::projection::{self, Held, MAX_LENSES, Reframe, Rolling};
use super::{Camera, Extent, Fallible, Nudge, Planes, Size, dmabuf};

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Whenever the compositor will take a frame. The gradient animates on
    /// every refresh, and so does playback that is still waiting for its
    /// first decoded frame.
    Refresh,
    /// At this instant, when the frame after the one just taken is due.
    At(Instant),
    /// Nothing changes by itself: paused, ended, or a still frame.
    Never,
}

/// Which clock a decoded frame's instant is read on, before the camera's
/// orientation is looked up at that instant (issue #8).
///
/// The player uses [`Self::Exposure`] and this exists so the losing
/// hypothesis stays measurable: `kyerag-spike --bin horizon` renders the same
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
    /// The world stays put: the body's roll and pitch are taken out
    /// completely, and the fast half of its heading with them.
    #[default]
    Locked,
    /// The view rides the camera, as it did before issue #8.
    Free,
}

/// The widget's state, owned by the shell.
pub struct Scene {
    show: Option<Show>,
    /// Wall-clock origin for the no-file gradient.
    started: Instant,
    /// Where a capture waits for the redraw that takes it (issue #15).
    shutter: Shutter,
    /// Set while the shell has hidden the controls, which is when the pointer
    /// goes with them: `mouse_interaction` answers `Hidden` instead of `Grab`
    /// (docs/UI.md, "The cursor"). One bit of shell state, and the only one
    /// this crate carries.
    cursor_hidden: bool,
    /// A [`Nudge`] the `View` menu left for the widget, which is where iced
    /// keeps the camera. Read once, by the next redraw.
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
    /// The heading the filter's yaw follow had reached when a drag took hold
    /// of the view, and `None` while the follow still has it (issue #44).
    pinned: Cell<Option<f64>>,
}

/// How the picture is to be held for one redraw: the shell's own toggle, and
/// the three overrides the headless instruments reach for.
#[derive(Clone, Copy, Debug)]
struct Holding {
    horizon: Horizon,
    clock: FrameClock,
    forced: Option<Quat>,
    readout: Option<Readout>,
    pinned: Option<f64>,
}

/// A file on screen: its calibration, and where its frames come from.
struct Show {
    /// One per decoded stream, in stream order.
    lenses: Arc<[Lens]>,
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
    /// and every camera whose readout direction has not been measured, which
    /// today is all of them (`kyerag_meta::Sweep::Unknown`). The pass is then
    /// what it was before issue #9, and the picture with it.
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
    /// No file: the animated gradient.
    pub fn blank() -> Self {
        Self {
            show: None,
            started: Instant::now(),
            shutter: Shutter::default(),
            cursor_hidden: false,
            nudge: Cell::new(None),
            horizon: Cell::new(Horizon::default()),
            clock: Cell::new(FrameClock::default()),
            forced: Cell::new(None),
            readout: Cell::new(None),
            pinned: Cell::new(None),
        }
    }

    /// Opens a file and starts playing it. Returns as soon as the container
    /// is parsed; the first frames arrive on the decode thread.
    pub fn open(path: &Path) -> Fallible<Self> {
        let mut player = Player::open(path)?;
        let (lenses, held) = calibrated(path, player.size(), player.lenses())?;
        println!(
            "media:  {}, {}x{}, {:.3} fps, {} frames, {:.1} s",
            // The older cameras write one lens per file, so this is 1 as
            // often as it is 2.
            match player.lenses() {
                1 => "1 lens stream".to_owned(),
                n => format!("{n} lens streams"),
            },
            player.size().width,
            player.size().height,
            player.timing().fps(),
            player.timing().frames,
            player.timing().duration().as_secs_f64(),
        );
        // Opening a file plays it, which is what every player does. Space
        // and the control row's button pause it (issue #16).
        player.play();
        Ok(Self {
            show: Some(Show::new(
                lenses,
                held,
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
        let mut reader = Reader::open(path)?;
        let (lenses, held) = calibrated(path, reader.size(), reader.lenses())?;
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
                lenses,
                held,
                Some(Arc::new(frames)),
                Source::Stepped(Box::new(reader)),
            )),
            ..Self::blank()
        })
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
        let Some(show) = &self.show else {
            return Next::Refresh;
        };
        // Out of the cell in one step: the borrow checker splits the fields
        // of a `&mut Playing`, but not those of a `RefMut`.
        let Playing { frames, source } = &mut *show.playing.borrow_mut();
        let Source::Live(player) = source else {
            return Next::Never;
        };
        match player.pump(now) {
            Ok(None) => {}
            Ok(Some(taken)) => *frames = Some(taken),
            Err(e) => {
                eprintln!("kyerag: playback stopped: {e}");
                player.pause(now);
                return Next::Never;
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

    /// Hide the pointer along with the controls, or bring both back.
    pub fn hide_cursor(&mut self, hidden: bool) {
        self.cursor_hidden = hidden;
    }

    pub fn is_cursor_hidden(&self) -> bool {
        self.cursor_hidden
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

    /// Hold the view where it is now, against the world, so that the filter's
    /// heading follow cannot carry it any further (issue #44). What a drag
    /// that moves the camera calls.
    ///
    /// The first drag takes hold and every later one inherits it: pinning
    /// again part way through a file would jump the picture by however far
    /// the follow had travelled since, and the view is already the pilot's by
    /// then anyway. [`Self::follow_view`] is the way back.
    pub fn pin_view(&self) {
        if self.pinned.get().is_some() {
            return;
        }
        // Nothing to pin to before the first frame arrives, and then the next
        // move of the same drag pins instead.
        self.pinned.set(
            self.show
                .as_ref()
                .and_then(|show| show.follow(self.clock.get())),
        );
    }

    /// Hand the view back to the camera's heading, which `View > Reset view`
    /// does along with putting the view straight.
    pub fn follow_view(&self) {
        self.pinned.set(None);
    }

    /// Whether a drag has taken the view off the heading follow (issue #44).
    pub fn is_view_pinned(&self) -> bool {
        self.pinned.get().is_some()
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

    fn player_mut(&mut self) -> Option<&mut Player> {
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

    pub fn primitive(&self, camera: Camera) -> ScenePrimitive {
        let held = Holding {
            horizon: self.horizon.get(),
            clock: self.clock.get(),
            forced: self.forced.get(),
            readout: self.readout.get(),
            pinned: self.pinned.get(),
        };
        ScenePrimitive {
            elapsed: self.started.elapsed().as_secs_f32(),
            camera,
            view: self.show.as_ref().and_then(|show| show.view(held)),
            shutter: self.shutter.clone(),
        }
    }
}

impl Show {
    fn new(
        lenses: Arc<[Lens]>,
        held: Arc<Motion>,
        frames: Option<Arc<Frames>>,
        source: Source,
    ) -> Self {
        Self {
            lenses,
            held,
            playing: RefCell::new(Playing { frames, source }),
        }
    }

    fn view(&self, held: Holding) -> Option<View> {
        let frames = self.playing.borrow().frames.clone()?;
        let at = self.held.instant(&frames, held.clock);
        let world_from_body = held.forced.unwrap_or_else(|| self.held.orientation.at(at));
        // Not under the horizon toggle: the readout is the camera's own
        // motion during the frame, and a view that rides the body has the
        // same skew in it as one that does not.
        let rolling = self
            .held
            .rolling(at, held.readout.unwrap_or(self.held.readout));
        Some(View {
            held: match held.horizon {
                Horizon::Locked => Held::locked(
                    world_from_body,
                    self.held.orientation.follow(at),
                    held.pinned,
                    rolling,
                ),
                Horizon::Free => Held::free(rolling),
            },
            lenses: self.lenses.clone(),
            frames,
        })
    }

    /// How far the heading follow has carried the stabilized frame at the
    /// frame on screen, which is what a drag pins the view at (issue #44).
    fn follow(&self, clock: FrameClock) -> Option<f64> {
        let frames = self.playing.borrow().frames.clone()?;
        Some(
            self.held
                .orientation
                .follow(self.held.instant(&frames, clock)),
        )
    }
}

/// Everything the trailer contributes to one open file: the calibration for
/// the lenses the shader samples, checked against the streams they will be
/// sampled from, and where the camera body went while it recorded.
///
/// One lens entry per decoded stream, and in the same order: the trailer
/// writes its lens blocks in the order the container carries the streams. A
/// camera that writes one lens per file (the ONE X2 and older) calibrates two
/// lenses in a file that decodes one, and then this is lens 0 alone and the
/// picture is one hemisphere.
///
/// The orientation is integrated here, once, at open: a 30-minute X4 Air
/// capture is 1.8 million IMU samples and costs about a fifth of a second to
/// read and integrate, against 70 ms to open the container. Doing it per
/// frame would be 30 times a second for a track that does not change.
fn calibrated(path: &Path, size: Size, streams: usize) -> Fallible<(Arc<[Lens]>, Arc<Motion>)> {
    let calibration = CalibrationSet::from_insv(path)?;
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
    println!(
        "lens:   {} {}, sampling {sampled} of {} calibrated",
        calibration.camera_model,
        calibration.firmware,
        calibration.lenses.len(),
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
    Ok((lenses.into(), Arc::new(held)))
}

/// What the shell hands the renderer for one frame.
#[derive(Debug)]
pub struct ScenePrimitive {
    elapsed: f32,
    camera: Camera,
    view: Option<View>,
    /// A handle on the [`Scene`]'s shutter, not a copy of it: the request
    /// is taken by whichever redraw reaches [`ScenePipeline::prepare`]
    /// first, and one that never does is still armed for the next.
    shutter: Shutter,
}

/// A pair of decoded lenses and the calibration that reprojects them. Both
/// halves are shared, so a redraw that changes nothing but the camera costs
/// two atomic increments.
#[derive(Clone, Debug)]
struct View {
    lenses: Arc<[Lens]>,
    frames: Arc<Frames>,
    /// Where the body was when these frames were taken, already inverted for
    /// the pass. Identity with the lock off.
    held: Held,
}

/// The GPU state behind the widget. iced builds one of these per primitive
/// type and keeps it for the life of the renderer.
pub struct ScenePipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    /// One black pixel, bound wherever a lens has no stream: before a file is
    /// open, and in the second slot of a file that carries one lens.
    blank: Planes,
    bind_group: wgpu::BindGroup,
    /// The frame the bind group points at, and the ones still in flight
    /// behind it. Newest first.
    live: VecDeque<Live>,
    /// Set when an import fails, so the message is printed once rather than
    /// on every redraw.
    failed: bool,
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

impl ScenePipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(format!("{}\n{SHADER}", projection::wgsl()).into()),
        });
        let layout = bind_group_layout(device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[&layout],
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
                targets: &[Some(format.into())],
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
        let bind_group = bind(device, &layout, &uniforms, [&blank; MAX_LENSES], &sampler);

        Self {
            pipeline,
            layout,
            sampler,
            uniforms,
            blank,
            bind_group,
            live: VecDeque::new(),
            failed: false,
            format,
            reported: false,
        }
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
            self.show(device, view);
        }

        let reframe = match &primitive.view {
            Some(view) if self.is_bound(view) => Reframe::new(
                &view.lenses,
                view.frames.size,
                primitive.camera,
                view.held,
                aspect,
                self.linearize(),
            ),
            _ => Reframe::gradient(primitive.elapsed, aspect, self.linearize()),
        };
        queue.write_buffer(&self.uniforms, 0, reframe.bytes());

        // After the uniform write, and only after it: the write lands at the
        // next submit on this queue, and the capture's own submit is that
        // one. Taken here rather than in `draw` because this is the call
        // that has a device to render with.
        if let Some(request) = primitive.shutter.take() {
            self.shoot(device, queue, request, aspect, primitive.view.as_ref());
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
        pass.draw(0..3, 0..1);
    }

    fn is_bound(&self, view: &View) -> bool {
        self.live
            .front()
            .is_some_and(|live| Arc::ptr_eq(&live.frames, &view.frames))
    }

    /// Imports a newly delivered pair and points the bind group at it. A
    /// redraw that shows the same pair again does nothing here.
    fn show(&mut self, device: &wgpu::Device, view: &View) {
        if self.failed || self.is_bound(view) {
            return;
        }
        match self.import(device, view) {
            Ok(planes) => {
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
            }
            Err(e) => {
                eprintln!("kyerag: frame not shown: {e}");
                self.failed = true;
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
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
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
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    });
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("scene"),
        entries: &entries,
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

fn gradient(uv: vec2<f32>, t: f32) -> vec3<f32> {
  let d = length(uv - vec2<f32>(0.5, 0.5));
  let wave = 0.5 + 0.5 * sin(d * 24.0 - t * 3.0);
  return vec3<f32>(wave * uv.x, wave * uv.y, wave);
}

// Each lens's picture at that ray, mixed by its weight, or grey where no lens
// has the ray. A lens weighted zero is not sampled at all, so outside the
// overlap this is the single fetch the hard pick took before issue #7, and
// the second fetch is what the blend band costs.
//
// WGSL has no texture array to index here, so the lenses are named rather
// than looped. The explicit mip level is what makes that legal: a
// `textureSample` computes its own level from derivatives and needs uniform
// control flow to do it, and every one of these textures has a single level
// anyway.
fn picture(mix: Blend) -> vec3<f32> {
  var rgb = vec3<f32>(0.0);
  var total = 0.0;
  if mix.weights[0] > 0.0 {
    rgb += mix.weights[0] * nv12(luma0, chroma0, frame_uv(mix.landings[0].pixel));
    total += mix.weights[0];
  }
  if mix.weights[1] > 0.0 {
    rgb += mix.weights[1] * nv12(luma1, chroma1, frame_uv(mix.landings[1].pixel));
    total += mix.weights[1];
  }
  return select(OUTSIDE_GRAY, rgb, total > 0.0);
}

// BT.709 full range: ffprobe reports bt709 and the camera writes yuvj420p.
// DRM_FORMAT_GR88 is little endian G:R, so .r is Cb and .g is Cr.
fn nv12(luma: texture_2d<f32>, chroma: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> {
  let y = textureSampleLevel(luma, samp, uv, 0.0).r;
  let c = textureSampleLevel(chroma, samp, uv, 0.0).rg - vec2<f32>(0.5, 0.5);
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
  let lens = picture(blend(view_ray(in.uv)));
  let rgb = select(gradient(in.uv, reframe.elapsed), lens, reframe.has_frame > 0.5);
  return vec4<f32>(select(rgb, linearize(rgb), reframe.linearize > 0.5), 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use kyerag_meta::{Filter, GyroSample, GyroTrack, Sweep};

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
            kyerag_meta::Mat3::IDENTITY,
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
    /// case, which today is every camera: `Sweep::Unknown` is a zero axis and
    /// there is nothing to apply it along.
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
