//! The libcosmic shell: one window whose body is the video, with a menu bar
//! in the header and a control overlay at the bottom.
//!
//! The shell owns almost nothing of the playback. It opens the file, it turns
//! keys and buttons into transport calls, and it asks the [`Scene`] for a
//! frame once per window redraw. The clock, the decode thread and the camera
//! all live below it in `kjerag-render` and `kjerag-media`.
//!
//! docs/UI.md is the specification for everything in this file, and it cites
//! a first-party COSMIC app for every call it makes. The two places where a
//! first-party app is not followed are marked in place.
//!
//! ## Frame pacing
//!
//! 29.97 fps content divides evenly into no display refresh rate anyone
//! ships, so there is no "hold each frame for N refreshes" rule that does
//! not drift. Every redraw instead asks the presentation clock which frame
//! is due at that instant, and the widget then asks iced to come back at the
//! instant the next one is due (`kjerag_render`'s `tick`, which is where the
//! clock is pumped). A frame is held for 2 refreshes at 60 Hz and for 4 or 5
//! at 144 Hz, in whatever pattern the arithmetic gives, with no error
//! carried forward.
//!
//! The pacing is deliberately not driven from here. The obvious shell-side
//! version, a `window::frames()` subscription that pumps the clock on each
//! redraw message, was written first and measured: it holds only 33 to 46
//! redraws a second on this box against a 60 Hz display, and drops 1 to 18
//! frames every 5 seconds, because the redraw event has to travel out to a
//! subscription and back through `update` before the next redraw can even be
//! asked for. Pumping inside the redraw pass, where iced already is, drops
//! nothing.
//!
//! It is also why the position on the scrubber is refreshed by a timer while
//! the controls are up: no message is sent per frame, so nothing else would
//! rebuild the view.

use std::any::TypeId;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cosmic::app::{Core, Settings, Task, context_drawer};
use cosmic::cosmic_config;
use cosmic::dialog::file_chooser::{self, FileFilter};
use cosmic::iced::event::{self, Event};
use cosmic::iced::futures::channel::oneshot;
use cosmic::iced::keyboard::key::{Key, Physical};
use cosmic::iced::keyboard::{Event as KeyEvent, Modifiers};
use cosmic::iced::mouse::Event as MouseEvent;
use cosmic::iced::runtime::clipboard;
use cosmic::iced::widget::{Stack, shader};
use cosmic::iced::window::{self, Mode};
use cosmic::iced::{Alignment, Length, Limits, Subscription, time};
use cosmic::widget::about::About;
use cosmic::widget::dnd_destination::dnd_destination_for_data;
use cosmic::widget::menu::Action as _;
use cosmic::widget::menu::key_bind::KeyBind;
use cosmic::widget::{self, Slider, icon};
use cosmic::{Application, ApplicationExt, Element, action, cosmic_theme, executor, font, theme};
use kjerag_render::{
    Accuracy, Foreign, Framing, Horizon, MissingDecoder, Nudge, Request, Scene, Stats,
};

use crate::config::{self, AppTheme, CONFIG_VERSION, Config, ConfigState, Stored};
use crate::dnd::Dropped;
use crate::key_bind::{Action, JUMP, key_binds};
use crate::shot::{Destination, Done};
use crate::{menu, shot, strings};

/// Icons for the two jump buttons, which are not in the icon theme.
/// cosmic-player ships them in its own `res/` and so do we (`res/icons/`,
/// GPL-3.0, attributed in the files themselves).
const JUMP_BACKWARD_ICON: &[u8] = include_bytes!("../res/icons/jump-backward-10-symbolic.svg");
const JUMP_FORWARD_ICON: &[u8] = include_bytes!("../res/icons/jump-forward-10-symbolic.svg");

/// The app icon, for the About page and the welcome view. The drawing itself,
/// rather than the `icon::from_name(Self::APP_ID)` cosmic-edit passes to the
/// same setter (`src/main.rs:1454-1468`), because a name only resolves for a
/// build whose icons are installed into an icon theme.
///
/// Measured three ways through `scripts/uitest.sh`'s welcome-mark check at
/// the rename (issue #75), which reads the colour of the patch this handle
/// draws into: the bytes read 114 181 163 with nothing installed;
/// `from_name` reads 27 27 27, the window background, with nothing installed;
/// `from_name` reads 99 186 173 with the tree on `XDG_DATA_DIRS`. So the ID
/// and the icon names now agree and the lookup does work once installed, and
/// a `cargo run` out of this tree would still draw a 128 px hole. libcosmic
/// hands back an empty SVG on a miss rather than a placeholder
/// (`src/widget/icon/named.rs:136-152`), so nothing on screen would say why.
const APP_ICON: &[u8] =
    include_bytes!("../../../resources/icons/hicolor/scalable/apps/dev.harding.Kjerag.svg");

/// How long the pointer has to sit still before the controls, the header bar
/// and the cursor go away (cosmic-player `src/main.rs:45`).
const CONTROLS_TIMEOUT: Duration = Duration::from_secs(2);

/// How often the position is re-read while the controls are up and the file
/// is playing. It refreshes the clock labels and it is what notices the
/// timeout above, so it bounds how late the controls can hide.
///
/// cosmic-player checks the timeout on its per-frame `NewFrame` message. We
/// have no per-frame message by design, so this timer stands in for one, and
/// it runs *only while playing*, which is the property that matters: the
/// controls must never hide out from under someone who paused to look
/// around, and for a reframing player that is the normal way to use it.
const CONTROLS_POLL: Duration = Duration::from_millis(250);

/// How often the playback report is printed while playing. It is the only
/// way to see dropped frames without a profiler.
const REPORT_EVERY: Duration = Duration::from_secs(5);

/// How long a toast stays up, and how many are kept: libcosmic's own numbers
/// (`src/widget/toaster/mod.rs:79-85`, `162-181`), which cosmic-files takes
/// unchanged.
const TOAST_FOR: Duration = Duration::from_secs(5);
const TOASTS: usize = 5;

/// Width of the volume popup (cosmic-player `src/main.rs:1924`).
const VOLUME_POPUP: f32 = 240.0;

/// Runs the shell.
pub fn run(input: Option<PathBuf>, at: Option<Framing>) -> Result<(), Box<dyn std::error::Error>> {
    let stored = Stored::load(App::APP_ID);
    let settings = Settings::default()
        .size_limits(Limits::NONE.min_width(360.0).min_height(240.0))
        // The window opens in the configured theme rather than flashing the
        // default one first (cosmic-player `src/main.rs:154-155`).
        .theme(stored.config.app_theme.theme());
    cosmic::app::run::<App>(settings, Flags { stored, input, at })?;
    Ok(())
}

pub struct Flags {
    stored: Stored,
    input: Option<PathBuf>,
    /// Where to land, when the command line named a view as well as a file.
    at: Option<Framing>,
}

#[derive(Clone, Debug)]
pub enum Message {
    /// The alert's own button, and Escape, which is the whole of what a
    /// dialog with nothing to decide can be answered with.
    AlertClose,
    /// The speaker button in the control row: show the volume slider, or take
    /// it away (cosmic-player `src/main.rs:1049-1053`, one dropdown of its
    /// four).
    AudioDropdown,
    /// Mute, or unmute (cosmic-player `src/main.rs:1223-1227`).
    AudioToggle,
    /// The volume slider was dragged, 0 to 1.
    AudioVolume(f64),
    /// It was let go, which is when the setting is written. A cosmic-config
    /// entry per pointer move would be a file write per pointer move.
    AudioVolumeRelease,
    Config(Config),
    ConfigState(ConfigState),
    /// A drag and drop landed. `None` is a payload that could not be read.
    Dropped(Option<Dropped>),
    /// A drop that arrived as a document portal transfer key, which is the
    /// only shape of drop a sandboxed app can open (issue #118, `dnd.rs`).
    /// The paths it stands for come back as a [`Message::Dropped`].
    DroppedTransfer(String),
    FileClearRecents,
    FileClose,
    FileLoad(PathBuf),
    FileOpen,
    FileOpenRecent(usize),
    Fullscreen,
    Key(Modifiers, Physical, Key),
    LaunchUrl(String),
    /// Hold the picture against the world, or let it ride the camera
    /// (issue #8).
    LockHorizon,
    /// A view change from the `View` menu or its keys.
    Look(Nudge),
    PlayPause,
    Quit,
    /// Five seconds have passed and playback has a line to print.
    Report,
    /// Take a still of the view as it stands: `s`, `Ctrl+C`, the camera
    /// button, or either File menu item (issue #15).
    Capture(Destination),
    /// One came back, some milliseconds later, off the render thread. It
    /// carries what was asked for as well as what happened, because a failure
    /// has to say which of the two it was.
    Captured(Destination, Result<Done, String>),
    /// A toast's close button, or its own five seconds running out: both
    /// arrive here (cosmic-files `src/app.rs:3008-3010`).
    CloseToast(u64),
    /// Put the view on the clipboard as one line of text: `i`, or
    /// `File > Copy current view reference`.
    CopyView,
    /// Go where a copied line says: `Ctrl+V`, or
    /// `File > Go to copied view reference`. The clipboard is read by a task,
    /// so the text arrives in the message below.
    PasteView,
    /// What the clipboard turned out to hold. `None` is an empty clipboard,
    /// and most of what is not `None` is not a view either.
    PastedView(Option<String>),
    /// The scrubber was dragged to this position, in seconds.
    Seek(f64),
    /// The scrubber was let go.
    SeekRelease,
    /// Seconds to jump, forward or back.
    SeekRelative(f64),
    /// Frames to step, forward or back.
    StepFrame(i64),
    /// Pointer input: the controls, the header bar and the cursor come back.
    ShowControls,
    /// A menu popup, which libcosmic runs as its own surface.
    Surface(cosmic::surface::Action),
    SystemThemeModeChange,
    Theme(AppTheme),
    /// The controls are up and the file is playing: re-read the clock, and
    /// hide everything if the pointer has sat still long enough.
    Tick,
    ToggleContextPage(ContextPage),
    /// The left button went down over the video. cosmic-player takes this as
    /// the way out of an open dropdown, and as play/pause when none is open
    /// (`src/main.rs:1507-1513`). Ours does the first half only: the same
    /// press is the look-around grab, so it cannot also be a transport
    /// control (docs/UI.md, conflict 1).
    VideoAreaClick,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextPage {
    About,
    Settings,
}

pub struct App {
    core: Core,
    /// The file on screen, if there is one.
    open: Option<Open>,
    /// What a file that would not open said, while the alert saying it is on
    /// screen. It holds the line rather than a flag because which line it is
    /// depends on why the open failed (issues #69 and #107).
    alert: Option<String>,
    stored: Stored,
    key_binds: HashMap<KeyBind, Action>,
    about: About,
    context_page: ContextPage,
    /// The theme names the settings dropdown shows, in its own order.
    themes: Vec<String>,
    /// What a capture says when it lands.
    toasts: Toasts,
    controls: Controls,
    /// Set while the scrubber is being dragged, to whether the file was
    /// playing when the drag started. cosmic-player pauses for the drag and
    /// restores the previous state on release (`src/main.rs:1325-1357`), and
    /// so do we.
    dragging: Option<bool>,
    fullscreen: bool,
    reported: Instant,
    /// The counters as of the last report, so each line covers its own five
    /// seconds instead of the whole run.
    counted: Stats,
}

/// A file on screen.
struct Open {
    path: PathBuf,
    scene: Scene,
    /// From the container: the frame count and the rational frame rate,
    /// divided.
    duration: Duration,
    /// Where the clock was when the view was last rebuilt. `view` has no
    /// instant to ask with, so this is refreshed by whichever message caused
    /// the rebuild.
    position: Duration,
}

/// The lines a capture leaves on screen, newest last.
///
/// libcosmic's `toaster::Toasts` is the model and the numbers are its own
/// ([`TOASTS`], [`TOAST_FOR`]). It is not that type because that type is only
/// readable by the widget that draws it, and that widget nails its stack to
/// the bottom of the window, which is where the control row lives
/// (docs/UI.md, "The capture toast").
#[derive(Default)]
struct Toasts {
    lines: Vec<Toast>,
    /// Never reused, so the line a dismissal names is the line it was pushed
    /// with: the five second task of a line already dropped for being the
    /// sixth closes nothing.
    next: u64,
}

struct Toast {
    id: u64,
    message: String,
}

impl Toasts {
    fn push(&mut self, message: String) -> u64 {
        let id = self.next;
        self.lines.push(Toast { id, message });
        self.next += 1;
        if self.lines.len() > TOASTS {
            self.lines.remove(0);
        }
        id
    }

    fn close(&mut self, id: u64) {
        self.lines.retain(|toast| toast.id != id);
    }
}

/// What a pasted line asks the window to do, given what is already on screen.
///
/// A paste is the one input that arrives with no idea what it is. Deciding
/// here rather than inside the shell is what lets all four answers be read in
/// one place, and tested with no window and no file.
#[derive(Clone, Debug, PartialEq)]
enum Goto {
    /// Nothing this app can read. No toast, no line, nothing at all:
    /// `Ctrl+V` over a video means nothing in any other player either, and a
    /// player that argues with every paste is a player nobody pastes into.
    Nothing,
    /// The file already on screen: seek and turn, and that is the whole of
    /// it.
    Here(Framing),
    /// A view of some other file, named with the directories to find it in.
    /// That is the terminal line, which is the one a pilot has in a report.
    Open(PathBuf, Framing),
    /// A view of some other file, named by that file alone. There is nothing
    /// to open and nowhere to go; all the window can do is say which video
    /// the line belongs to.
    Elsewhere(PathBuf),
}

impl Goto {
    fn read(text: Option<&str>, open: Option<&Path>) -> Self {
        let Some((file, framing)) = text.and_then(Framing::read_line) else {
            return Self::Nothing;
        };
        if names(open, &file) {
            return Self::Here(framing);
        }
        match file.parent().is_some_and(|up| !up.as_os_str().is_empty()) {
            true => Self::Open(file, framing),
            false => Self::Elsewhere(file),
        }
    }
}

/// Whether a line naming `file` is naming the file on screen.
///
/// A copied line carries the name alone and a printed one carries the path,
/// so both have to answer yes for the file being watched: the name is what a
/// copy has, and the whole path is what tells two videos of the same name in
/// two folders apart.
fn names(open: Option<&Path>, file: &Path) -> bool {
    let Some(open) = open else {
        return false;
    };
    match file.parent().is_some_and(|up| !up.as_os_str().is_empty()) {
        true => open == file,
        false => open.file_name() == Some(file.as_os_str()),
    }
}

/// The overlay's visibility, and when the pointer last asked for it.
struct Controls {
    shown: bool,
    since: Instant,
    /// The volume slider, which sits in a popup above the row rather than in
    /// it (cosmic-player `src/main.rs:1777-1807`). Everything that is not the
    /// slider itself takes it away again: a press in the video, the transport,
    /// fullscreen, and the row going ([`App::hide_volume`]).
    ///
    /// It goes when the row goes, which cosmic-player does the other way
    /// round: it holds the controls up for as long as a dropdown is open
    /// (`src/main.rs:1627`). Ours cannot, because a drag to look around is
    /// pointer input, so the row would never time out.
    volume: bool,
}

impl cosmic::Application for App {
    type Executor = executor::Default;
    type Flags = Flags;
    type Message = Message;

    const APP_ID: &'static str = "dev.harding.Kjerag";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    /// No `nav_model`, which is the default: libcosmic adds the nav-bar
    /// toggle to the header only when there is a model to toggle
    /// (`src/app/mod.rs:786`), and we have no playlist.
    fn init(mut core: Core, flags: Flags) -> (Self, Task<Self::Message>) {
        // Video wants both window edges, and this is cosmic-player's way of
        // getting them (`src/main.rs:895`): zero the border padding and keep
        // libcosmic's content container. `main_content_padding` is then
        // `[0, 0, 0, 0]` (`app/mod.rs:632-639`), which is the same view as
        // turning the container off, with the window background that turning
        // it off takes away: libcosmic paints
        // `background(theme.transparent).base` only on the container branch
        // (`app/mod.rs:856-874`), and that colour is the whole of what makes
        // a COSMIC window a darkened pane over the compositor's blur rather
        // than bare blur. cosmic-files leaves the container on for the same
        // reason and never paints a background of its own
        // (`src/app.rs:2352-2367`, container off in desktop mode only).
        //
        // Since issue #100 the room around the ball is that pane as well:
        // the pass writes it transparent, so turning the container off would
        // take the picture's surroundings with the window's background.
        core.window.border_padding = Some(0);

        let mut app = App {
            core,
            open: None,
            alert: None,
            stored: flags.stored,
            key_binds: key_binds(),
            about: about(),
            context_page: ContextPage::Settings,
            themes: vec![
                strings::THEME_SYSTEM.to_owned(),
                strings::THEME_DARK.to_owned(),
                strings::THEME_LIGHT.to_owned(),
            ],
            toasts: Toasts::default(),
            controls: Controls {
                shown: true,
                since: Instant::now(),
                volume: false,
            },
            dragging: None,
            fullscreen: false,
            reported: Instant::now(),
            counted: Stats::default(),
        };

        let task = match flags.input {
            Some(path) => app.update(Message::FileLoad(path)),
            None => app.retitle(),
        };
        // A view named on the command line lands with no toast. Nothing was
        // pasted and nobody needs telling what they just typed; the window
        // opening at that view is the whole of the answer.
        if let Some(at) = flags.at {
            app.place(at);
        }
        (app, task)
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        let now = Instant::now();
        match message {
            Message::AlertClose => self.alert = None,
            Message::AudioDropdown => {
                self.controls.volume = !self.controls.volume;
                self.show_controls(now);
            }
            Message::AudioToggle => {
                // A file with no sound in it draws the speaker button
                // disabled, and a key has no disabled state: the guard the
                // two ways in share lives here, so `m` on a silent file
                // leaves the setting and the icon alone.
                if !self.has_sound() {
                    return Task::none();
                }
                self.stored.config.muted = !self.stored.config.muted;
                self.stored.write_config();
                self.hold_sound();
                self.show_controls(now);
            }
            Message::AudioVolume(volume) => {
                // Moving the slider unmutes, which is what cosmic-player does
                // (`src/main.rs:1229-1235`): the pilot is asking to hear
                // something.
                self.stored.config.volume = volume.clamp(0.0, 1.0);
                self.stored.config.muted = false;
                self.hold_sound();
                self.show_controls(now);
            }
            Message::AudioVolumeRelease => {
                self.stored.write_config();
                self.show_controls(now);
            }
            Message::Config(config) => {
                self.stored.config = config;
                // The settings can change from outside this window, so the
                // scene is told again rather than only on the toggle.
                self.hold_horizon();
                self.hold_sound();
                return cosmic::command::set_theme(self.stored.config.app_theme.theme());
            }
            Message::ConfigState(state) => self.stored.state = state,
            Message::Dropped(dropped) => {
                // First file wins, others are ignored.
                let Some(path) = dropped.and_then(|files| files.0.into_iter().next()) else {
                    eprintln!("kjerag: that drop carried no local file");
                    self.alert = Some(strings::open_failed(None));
                    return Task::none();
                };
                return self.update(Message::FileLoad(path));
            }
            Message::DroppedTransfer(key) => {
                // The key is the drop; the files are the portal's to hand
                // over, which is a call to another process rather than bytes
                // that came with the drag (`dnd.rs`).
                return cosmic::command::file_transfer_receive(key).map(|answer| {
                    action::app(Message::Dropped(match answer {
                        Ok(paths) => Some(Dropped::transferred(paths)),
                        Err(e) => {
                            eprintln!("kjerag: that drop's files stayed with the portal: {e}");
                            None
                        }
                    }))
                });
            }
            Message::FileClearRecents => {
                self.stored.state.recent_files.clear();
                self.stored.write_state();
            }
            Message::FileClose => {
                self.pool_seam();
                self.open = None;
                self.show_controls(now);
                return self.retitle();
            }
            Message::FileLoad(path) => {
                self.load(&path);
                self.show_controls(now);
                return self.retitle();
            }
            Message::FileOpen => return chooser(),
            Message::FileOpenRecent(index) => {
                let Some(path) = self.stored.state.recent_files.get(index).cloned() else {
                    return Task::none();
                };
                return self.update(Message::FileLoad(path));
            }
            Message::Fullscreen => {
                let Some(id) = self.core.main_window_id() else {
                    return Task::none();
                };
                self.fullscreen = !self.fullscreen;
                // The popup is laid out against the window, so it does not
                // survive the window changing shape under it (cosmic-player
                // `src/main.rs:1190-1193`).
                self.hide_volume();
                self.show_controls(now);
                let mode = match self.fullscreen {
                    true => Mode::Fullscreen,
                    false => Mode::Windowed,
                };
                return window::set_mode(id, mode);
            }
            Message::Key(modifiers, physical, key) => {
                for (bind, action) in &self.key_binds {
                    if bind.matches(modifiers, &key, Some(&physical)) {
                        return self.update(action.message());
                    }
                }
            }
            Message::LaunchUrl(url) => {
                if let Err(e) = open::that_detached(&url) {
                    eprintln!("kjerag: {url} not opened: {e}");
                }
            }
            Message::LockHorizon => {
                self.stored.config.horizon_lock = !self.stored.config.horizon_lock;
                self.stored.write_config();
                self.hold_horizon();
                self.show_controls(now);
            }
            Message::Look(nudge) => {
                if let Some(open) = &self.open {
                    open.scene.nudge(nudge);
                }
            }
            Message::PlayPause => {
                if let Some(open) = &mut self.open {
                    open.scene.toggle_play(now);
                }
                self.hide_volume();
                // A transport control is pointer or key input: the controls
                // stay up long enough to see what it did.
                self.show_controls(now);
            }
            Message::Capture(to) => {
                self.show_controls(now);
                return self.capture(to);
            }
            Message::Captured(to, still) => return self.captured(to, still),
            Message::CloseToast(id) => self.toasts.close(id),
            Message::CopyView => {
                self.show_controls(now);
                return self.copy_view();
            }
            Message::PasteView => {
                self.show_controls(now);
                // The clipboard is somebody else's process on Wayland, so
                // reading it is a task and not a call.
                return clipboard::read().map(|text| action::app(Message::PastedView(text)));
            }
            Message::PastedView(text) => return self.go_to_view(text.as_deref()),
            Message::Seek(seconds) => {
                let position = Duration::from_secs_f64(seconds.max(0.0));
                self.hide_volume();
                let Some(open) = &mut self.open else {
                    return Task::none();
                };
                if self.dragging.is_none() {
                    self.dragging = Some(open.scene.is_playing());
                    open.scene.pause(now);
                }
                // Where the drag is, not where the picture landed: a
                // keyframe seek comes down up to a second early, and the
                // label has to say what the pilot is pointing at.
                open.position = position;
                open.scene.seek(position, Accuracy::Keyframe);
            }
            Message::SeekRelease => {
                self.hide_volume();
                let was_playing = self.dragging.take().unwrap_or(false);
                if let Some(open) = &mut self.open {
                    open.scene.seek(open.position, Accuracy::Exact);
                    if was_playing {
                        open.scene.play();
                    }
                }
                self.show_controls(now);
            }
            Message::SeekRelative(seconds) => {
                if let Some(open) = &mut self.open {
                    // From the clock rather than from the label: with the
                    // controls hidden nothing has refreshed the label since
                    // they went.
                    let to = shift(open.scene.position(now), seconds).min(open.duration);
                    open.scene.seek(to, Accuracy::Exact);
                }
                self.show_controls(now);
            }
            Message::StepFrame(frames) => {
                if let Some(open) = &mut self.open {
                    open.scene.step(now, frames);
                }
                self.show_controls(now);
            }
            Message::Quit => {
                // Before the exit, because the exit is a real one: nothing
                // below this runs any shutdown, so a fit that landed during
                // this file would be thrown away with the process.
                self.pool_seam();
                std::process::exit(0)
            }
            Message::Report => {
                self.report(now);
                // The fit lands on a thread of its own with no message to
                // announce it, and the pilot may never close the file: five
                // seconds is soon enough and `seam_harvest` takes rather than
                // reads, so this is a lock on every report and nothing more.
                self.pool_seam();
            }
            Message::ShowControls => self.show_controls(now),
            Message::Surface(action) => {
                return cosmic::task::message(cosmic::Action::Cosmic(
                    cosmic::app::Action::Surface(action),
                ));
            }
            Message::SystemThemeModeChange => {
                return cosmic::command::set_theme(self.stored.config.app_theme.theme());
            }
            Message::Theme(app_theme) => {
                self.stored.config.app_theme = app_theme;
                self.stored.write_config();
                return cosmic::command::set_theme(app_theme.theme());
            }
            Message::Tick => {
                self.read_clock(now);
                self.hide_idle_controls(now);
            }
            Message::ToggleContextPage(page) => {
                if self.context_page == page {
                    self.core.window.show_context = !self.core.window.show_context;
                } else {
                    self.context_page = page;
                    self.core.window.show_context = true;
                }
            }
            Message::VideoAreaClick => self.hide_volume(),
        }
        Task::none()
    }

    /// Escape takes the alert away first, then closes the context drawer,
    /// then leaves fullscreen (cosmic-edit `src/main.rs:1583-1592`, and the
    /// dialog goes first the way cosmic-files does it, `src/app.rs:2769-2776`
    /// on master). One press, one thing, outermost first. libcosmic gives
    /// Escape to the app through this hook, which is why the key map does not
    /// bind it.
    fn on_escape(&mut self) -> Task<Self::Message> {
        if self.alert.is_some() {
            self.alert = None;
            return Task::none();
        }
        if self.core.window.show_context {
            self.core.window.show_context = false;
            return Task::none();
        }
        if self.fullscreen {
            return self.update(Message::Fullscreen);
        }
        Task::none()
    }

    /// A file that would not open, as an alert in the middle of the window
    /// (owner's call, 2026-08-01: the line it used to leave on the welcome
    /// view was the wrong surface for it).
    ///
    /// The stock dialog, shaped the way cosmic-files shapes the one it puts
    /// up for an operation that failed: a title, the error as the body, the
    /// `dialog-error` icon at 64, and one button (`src/app.rs:5665-5678` on
    /// master). The button is `suggested` rather than that dialog's
    /// `standard`, because there is nothing here to cancel: it is the primary
    /// action and the only one, which is what cosmic-edit's primary action is
    /// (`src/main.rs:1657-1671`). libcosmic centres whatever this returns
    /// over the view in a modal popover (`src/app/mod.rs:877-884`).
    fn dialog(&self) -> Option<Element<'_, Self::Message>> {
        let said = self.alert.as_deref()?;
        Some(
            widget::dialog()
                .title(strings::CANNOT_OPEN)
                .body(said)
                .icon(icon::from_name("dialog-error").size(64))
                .primary_action(
                    widget::button::suggested(strings::CLOSE).on_press(Message::AlertClose),
                )
                .into(),
        )
    }

    /// The menu bar and nothing else, which is unanimous across the three
    /// first-party apps. `header_end` stays empty: every action we have is
    /// either a transport control, which belongs in the overlay, or a menu
    /// item. No header title either, so the picture has the window to itself.
    fn header_start(&self) -> Vec<Element<'_, Self::Message>> {
        vec![menu::menu_bar(
            &self.core,
            &self.stored.state,
            &self.key_binds,
            self.open.is_some(),
            self.stored.config.horizon_lock,
        )]
    }

    fn context_drawer(&self) -> Option<context_drawer::ContextDrawer<'_, Self::Message>> {
        if !self.core.window.show_context {
            return None;
        }
        Some(match self.context_page {
            ContextPage::About => context_drawer::about(
                &self.about,
                |url| Message::LaunchUrl(url.to_owned()),
                Message::ToggleContextPage(ContextPage::About),
            ),
            ContextPage::Settings => context_drawer::context_drawer(
                self.settings(),
                Message::ToggleContextPage(ContextPage::Settings),
            )
            .title(strings::SETTINGS_TITLE),
        })
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let shown = match &self.open {
            Some(open) => self.playing(open),
            None => self.welcome(),
        };
        // A layer over the picture rather than a row beside it, so the toast
        // hangs under the header and the picture keeps the whole window.
        // `Stack` only takes the cursor away from the layer beneath where the
        // layer above reports an interaction for it
        // (`iced/widget/src/stack.rs`, `update`), so the drag that looks
        // around still starts anywhere except on a toast's close button, and
        // `overlay::from_children` keeps the control row's overlay working
        // under it.
        //
        // The layer is mounted whether or not there is a toast in it, so the
        // shape of the tree around the picture never changes. Building the
        // stack only when a toast arrives was measured under the harness and
        // is not a free rearrangement: the toast reached the screen on the
        // first capture after it landed with a fixed tree, and on the sixth,
        // two seconds later, with a tree that grew a layer.
        let content = Stack::with_children(vec![shown, self.toast_stack()]);
        // cosmic-player implements no drag and drop, so this follows
        // cosmic-files (`src/app.rs:6491-6496`). The destination is the whole
        // window rather than only the video: a file dropped on "No video
        // open" is the drop most worth catching, and it is the one a video
        // widget would not be there to catch.
        dnd_destination_for_data(content, |dropped: Option<Dropped>, _action| {
            Message::Dropped(dropped)
        })
        // The other shape a drop comes in, and the only one that survives a
        // sandbox (issue #118). This is what adds the mime type to the
        // offer as well as handling it, and it goes ahead of `text/uri-list`
        // in what the window will accept, which is the order GTK uses for
        // the same choice: a source that offers both is a source that
        // registered the files with the portal, and the portal's answer is
        // openable from either side of a sandbox where a bare path is not.
        .on_file_transfer(Message::DroppedTransfer)
        .into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        struct ConfigSubscription;
        struct ConfigStateSubscription;
        struct ThemeSubscription;

        let mut sources = vec![
            event::listen_with(|event, status, _window| match (event, status) {
                // `Ignored` is what stops a key firing twice when a widget
                // already took it.
                (
                    Event::Keyboard(KeyEvent::KeyPressed {
                        modifiers,
                        physical_key,
                        key,
                        ..
                    }),
                    event::Status::Ignored,
                ) => Some(Message::Key(modifiers, physical_key, key)),
                (Event::Mouse(MouseEvent::CursorMoved { .. }), _) => Some(Message::ShowControls),
                _ => None,
            }),
            cosmic_config::config_subscription(
                TypeId::of::<ConfigSubscription>(),
                Self::APP_ID.into(),
                CONFIG_VERSION,
            )
            .map(|update| Message::Config(update.config)),
            cosmic_config::config_state_subscription(
                TypeId::of::<ConfigStateSubscription>(),
                Self::APP_ID.into(),
                CONFIG_VERSION,
            )
            .map(|update| Message::ConfigState(update.config)),
            // "Match desktop" has to follow the desktop while the app is
            // running, not only at startup.
            cosmic_config::config_subscription::<_, cosmic_theme::ThemeMode>(
                TypeId::of::<ThemeSubscription>(),
                cosmic_theme::THEME_MODE_ID.into(),
                cosmic_theme::ThemeMode::version(),
            )
            .map(|_| Message::SystemThemeModeChange),
        ];
        if self.is_playing() {
            sources.push(time::every(REPORT_EVERY).map(|_| Message::Report));
            if self.controls.shown {
                sources.push(time::every(CONTROLS_POLL).map(|_| Message::Tick));
            }
        }
        Subscription::batch(sources)
    }
}

impl App {
    fn is_playing(&self) -> bool {
        self.open
            .as_ref()
            .is_some_and(|open| open.scene.is_playing())
    }

    /// Opens a file, or says why it did not in an alert over whatever the
    /// window was already showing. cosmic-player only logs
    /// (`src/video.rs:63`), which leaves the pilot staring at an unchanged
    /// window; a player with exactly one job should say when it cannot do it.
    ///
    /// A failed open takes nothing away (owner's call, 2026-08-01): the video
    /// that was playing carries on playing behind the alert, because a file
    /// that would not open is not a reason to stop the one that did.
    fn load(&mut self, path: &Path) {
        self.pool_seam();
        match Scene::open(path) {
            Ok(scene) => {
                self.alert = None;
                self.hold_seam(&scene);
                self.open = Some(Open {
                    path: path.to_path_buf(),
                    duration: scene.duration(),
                    position: Duration::ZERO,
                    scene,
                });
                self.stored.state.remember(path);
                self.stored.write_state();
                self.hold_horizon();
                self.hold_sound();
            }
            Err(e) => {
                eprintln!("kjerag: {} not shown: {e}", path.display());
                self.alert = Some(refusal(&*e, path));
            }
        }
    }

    /// Hand the horizon setting to the scene, which is where the picture is
    /// held. A file with no IMU record takes it and does nothing with it.
    fn hold_horizon(&self) {
        let Some(open) = &self.open else {
            return;
        };
        open.scene
            .set_horizon(match self.stored.config.horizon_lock {
                true => Horizon::Locked,
                false => Horizon::Free,
            });
    }

    /// Hand this camera's pooled seam calibration to the scene, before its
    /// first frame is drawn (issue #48).
    ///
    /// A camera the pool knows nothing about falls back to a fit off this
    /// file's own frames, which is the weaker answer for the reason 6.8
    /// measures: a flight's own seam carries that flight's parallax, and a fit
    /// taken through it absorbs some. That is the whole of the difference
    /// between the two paths here, and it is why the fallback's answer is
    /// pooled rather than believed.
    ///
    /// Nothing is asked of the pilot either way (AGENTS.md, zero-config
    /// playback). The terminal line is the whole of what is said about it.
    fn hold_seam(&self, scene: &Scene) {
        let Some(camera) = scene.camera_key() else {
            return;
        };
        let pooled = self.stored.state.seam_pooled(camera);
        if let Some(fit) = self.stored.state.seam(camera) {
            println!(
                "seam:   lens 1 roll {:+.3}, yaw {:+.3}, pitch {:+.3} deg, cx {:+.2}, \
                 cy {:+.2} px (pooled over {pooled} fits of this camera)",
                fit.roll_deg, fit.yaw_deg, fit.pitch_deg, fit.cx_px, fit.cy_px,
            );
            scene.use_seam(fit);
        }
        // The pool keeps growing until it has enough to median over, and this
        // is the whole of "calibrate by watching": a camera with one fit in it
        // is drawn with that fit and still learns from the next file, because
        // one fit is one flight's parallax and the median over several is not.
        if pooled < config::POOL_ENOUGH {
            scene.fit_seam(pooled == 0);
        }
    }

    /// Fold whatever the open file taught us about its camera's seam into that
    /// camera's pool, on the way out.
    ///
    /// Called when a file is closed or replaced rather than when the fit
    /// lands, because a fit that landed one second into a file the pilot then
    /// scrubbed through is the same evidence as one that landed and was
    /// watched: what makes it worth keeping is its own quality, which travels
    /// with it, and waiting until the file is done costs nothing.
    fn pool_seam(&mut self) {
        let Some(open) = &self.open else {
            return;
        };
        let (Some(camera), Some(harvest)) = (open.scene.camera_key(), open.scene.seam_harvest())
        else {
            return;
        };
        if !self.stored.state.harvest(camera, harvest) {
            return;
        }
        let pooled = self.stored.state.seam_pooled(camera);
        println!(
            "seam:   kept that fit, {} azimuths leaving {:.3} deg; this camera's pool is {pooled}",
            harvest.patches, harvest.residual_deg,
        );
        self.stored.write_state();
        // The median moved, so the picture follows it. Walked, not landed:
        // there has been a picture on screen for seconds by now.
        if let Some(fit) = self.stored.state.seam(camera) {
            open.scene.aim_seam(fit);
        }
    }

    /// Hand the volume and the mute to the scene, which is where the sound is
    /// (issue #13). A file with no sound takes them and does nothing.
    fn hold_sound(&self) {
        let Some(open) = &self.open else {
            return;
        };
        open.scene.set_volume(self.stored.config.volume as f32);
        open.scene.set_muted(self.stored.config.muted);
    }

    /// Whether there is anything to mute: `false` for no file, for a file
    /// with no sound in it, and for a box with no working output.
    fn has_sound(&self) -> bool {
        self.open
            .as_ref()
            .is_some_and(|open| open.scene.has_sound())
    }

    fn retitle(&mut self) -> Task<Message> {
        let Some(id) = self.core.main_window_id() else {
            return Task::none();
        };
        let title = strings::window_title(self.open.as_ref().map(|open| open.path.as_path()));
        self.set_window_title(title, id)
    }

    /// Pointer input, or anything else worth looking at: the control row, the
    /// header bar and the cursor all come back together.
    fn show_controls(&mut self, now: Instant) {
        self.controls = Controls {
            shown: true,
            since: now,
            ..self.controls
        };
        self.core.window.show_headerbar = !self.fullscreen;
        self.hide_cursor(false);
        self.read_clock(now);
    }

    /// Two seconds of no pointer input takes all three away again. Only ever
    /// called from [`Message::Tick`], whose subscription runs while playing:
    /// a naive timer would hide the controls out from under someone who
    /// paused to look around.
    fn hide_idle_controls(&mut self, now: Instant) {
        if !self.controls.shown || now.duration_since(self.controls.since) < CONTROLS_TIMEOUT {
            return;
        }
        self.controls.shown = false;
        self.hide_volume();
        self.core.window.show_headerbar = false;
        self.hide_cursor(true);
    }

    /// Take the volume popup away, which everything that is not the slider
    /// itself does. cosmic-player closes its dropdowns from each of these in
    /// turn: fullscreen (`src/main.rs:1191-1192`), play and pause (`1253`),
    /// the scrubber and its release (`1326`, `1348`), and a press in the
    /// video (`1508-1509`).
    fn hide_volume(&mut self) {
        self.controls.volume = false;
    }

    fn hide_cursor(&mut self, hidden: bool) {
        if let Some(open) = &mut self.open {
            open.scene.hide_cursor(hidden);
        }
    }

    fn read_clock(&mut self, now: Instant) {
        // A drag owns the position while it lasts. The clock is showing the
        // keyframe the scrub landed on, which is behind the drag by up to a
        // GOP, and the label must follow the pilot's hand.
        if self.dragging.is_some() {
            return;
        }
        if let Some(open) = &mut self.open {
            open.position = open.scene.position(now).min(open.duration);
        }
    }

    /// Arms a still of the next frame drawn, and waits for it (issue #15).
    ///
    /// Nothing here touches the picture. The render pass takes the request on
    /// its next redraw, a worker thread reads the pixels back and either
    /// writes a JPEG or encodes a PNG for the paste, and this task is woken
    /// when that is done. The clipboard is the one step that has to come back
    /// to the shell, because on Wayland it is the window that offers the data.
    fn capture(&self, to: Destination) -> Task<Message> {
        let Some(open) = &self.open else {
            return Task::none();
        };
        let video = open.path.clone();
        // Read as the capture is armed, and printed against the frame the
        // redraw after this one actually caught: a still taken with a drag in
        // flight can be one redraw of turning ahead of the line.
        let camera = open.scene.viewpoint().camera();
        let horizon = open.scene.horizon();
        let (finished, waiting) = oneshot::channel();
        open.scene.capture(Request {
            width: shot::WIDTH,
            then: Box::new(move |taken| {
                let done = taken
                    .and_then(|still| {
                        // A JPEG carries the video and the timecode in its
                        // name and no direction anywhere, so where the still
                        // was looking is only ever recoverable from here.
                        // Printed before the answer is sent, which is what
                        // puts it above the `shot:` line the shell writes
                        // when it arrives.
                        let framing = Framing {
                            at: still.time,
                            camera,
                            horizon,
                        };
                        println!("view:   {}", framing.printed(&video));
                        shot::finish(&still, &video, to)
                    })
                    .map_err(|e| e.to_string());
                let _ = finished.send(done);
            }),
        });
        Task::perform(waiting, move |done| {
            action::app(Message::Captured(
                to,
                done.unwrap_or_else(|_| {
                    Err("the capture was replaced before a redraw took it".to_owned())
                }),
            ))
        })
    }

    /// Says where the still went, and puts it on the clipboard when that is
    /// what was asked for.
    ///
    /// The terminal line stays: it is what the headless harness reads, and it
    /// carries the whole path, which the toast deliberately does not.
    fn captured(&mut self, to: Destination, still: Result<Done, String>) -> Task<Message> {
        match still {
            Ok(Done::Saved(path)) => {
                println!("shot:   {}", path.display());
                self.toast(strings::frame_saved(&path))
            }
            Ok(Done::Copied(png)) => {
                println!("shot:   copied");
                Task::batch([
                    self.toast(strings::FRAME_COPIED.to_owned()),
                    clipboard::write_data(png),
                ])
            }
            Err(e) => {
                eprintln!("kjerag: no still: {e}");
                self.toast(strings::capture_failed(to, &e))
            }
        }
    }

    /// The view as one line of `reframe`'s own arguments, on the clipboard
    /// and on the terminal.
    ///
    /// What a report about a 360 video is missing is the direction it was
    /// pointing, and this is the whole of the fix: which video, which frame,
    /// and the three angles, in a line that can be pasted into an issue and
    /// run as a command. The clipboard copy carries the file's name and the
    /// terminal one carries its path ([`Framing`]).
    ///
    /// The frame is the one on screen rather than the clock's position: a
    /// paused scrub shows the keyframe it landed on, and what is asked for
    /// here is the picture the pilot is looking at.
    fn copy_view(&mut self) -> Task<Message> {
        let Some(open) = &self.open else {
            return Task::none();
        };
        let framing = Framing {
            at: open.scene.frame().map_or(open.position, |(_, time)| time),
            camera: open.scene.viewpoint().camera(),
            horizon: open.scene.horizon(),
        };
        println!("view:   {}", framing.printed(&open.path));
        let line = framing.copied(&open.path);
        Task::batch([
            self.toast(strings::VIEW_COPIED.to_owned()),
            clipboard::write(line),
        ])
    }

    /// Go where a copied line says: the frame it names, the direction it was
    /// pointing, and the horizon it was held with.
    ///
    /// This is the other half of [`Self::copy_view`], and the pair is what
    /// makes the line a place rather than a label. The four things a paste
    /// can be are [`Goto`]'s four answers, decided there so they can be read
    /// and tested without a window.
    fn go_to_view(&mut self, text: Option<&str>) -> Task<Message> {
        let open = self.open.as_ref().map(|open| open.path.as_path());
        match Goto::read(text, open) {
            Goto::Nothing => Task::none(),
            Goto::Elsewhere(file) => self.toast(strings::view_is_from(&file)),
            Goto::Here(framing) => {
                self.place(framing);
                self.toast(strings::WENT_TO_VIEW.to_owned())
            }
            Goto::Open(file, framing) => {
                self.load(&file);
                let titled = self.retitle();
                if self.open.is_none() {
                    return titled;
                }
                self.place(framing);
                Task::batch([titled, self.toast(strings::WENT_TO_VIEW.to_owned())])
            }
        }
    }

    /// Put the window at a framing: seek to the frame, point the view, hold
    /// the horizon the way it was held.
    ///
    /// A jump and not an animation. The seek is the exact one rather than the
    /// keyframe a scrub settles for, because the line names one frame; the
    /// camera lands on the next redraw, which is the only place the far end
    /// of the zoom can be clamped to this window's shape ([`Nudge::Point`]).
    ///
    /// The lock is written to the config, which is what pressing `h` does:
    /// there is one horizon setting and a view that was copied held is not
    /// the same view unheld.
    fn place(&mut self, framing: Framing) {
        let locked = matches!(framing.horizon, Horizon::Locked);
        if self.stored.config.horizon_lock != locked {
            self.stored.config.horizon_lock = locked;
            self.stored.write_config();
        }
        self.hold_horizon();
        let Some(open) = &mut self.open else {
            return;
        };
        println!("goto:   {}", framing.printed(&open.path));
        open.position = framing.at.min(open.duration);
        open.scene.seek(open.position, Accuracy::Exact);
        open.scene.nudge(Nudge::Point(framing.camera));
    }

    /// One toast, and the task that takes it away again five seconds later.
    /// That is libcosmic's own dismissal, moved out of `Toasts::push` with
    /// nothing else changed: a sleep on the async runtime rather than a timer
    /// the shell has to keep, so a toast that is up costs no redraws
    /// (`src/widget/toaster/mod.rs:183-196`).
    fn toast(&mut self, message: String) -> Task<Message> {
        let id = self.toasts.push(message);
        cosmic::task::future(async move {
            tokio::time::sleep(TOAST_FOR).await;
            Message::CloseToast(id)
        })
        .map(cosmic::Action::App)
    }

    /// The toasts, under the header and centered, newest first: cosmic-files'
    /// stack order (libcosmic `src/widget/toaster/mod.rs:56-63`, which reads
    /// its queue back to front) against the top edge instead of the bottom
    /// one.
    fn toast_stack(&self) -> Element<'_, Message> {
        let spacing = theme::active().cosmic().spacing;
        let lines = self.toasts.lines.iter().rev().fold(
            widget::column::with_capacity(self.toasts.lines.len()).spacing(spacing.space_xxxs),
            |column, toast| column.push(toast_line(toast, spacing)),
        );
        widget::column::with_children(vec![lines.into(), widget::space::vertical().into()])
            .align_x(Alignment::Center)
            // Clear of the menu bar rather than tucked under it: the header
            // is the one part of the window a pointer goes to while a capture
            // is landing.
            .padding([spacing.space_m, spacing.space_none])
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn report(&mut self, now: Instant) {
        let Some(stats) = self.open.as_ref().and_then(|open| open.scene.stats()) else {
            return;
        };
        println!(
            "play:   {:>8.2} s, {}",
            self.open
                .as_ref()
                .map_or(0.0, |open| open.scene.position(now).as_secs_f64()),
            stats
                .since(self.counted)
                .report(now.duration_since(self.reported)),
        );
        self.counted = stats;
        self.reported = now;
    }

    /// Nothing open: an icon, a line saying so, and the button that fixes it
    /// (cosmic-player `src/main.rs:1676-1695`).
    ///
    /// The mark is the app icon at the size a first-party empty state draws
    /// one: cosmic-files' empty folder is `.size(64)` over a `text::body`,
    /// and ours is twice that at the owner's direction (2026-07-31)
    /// line (`src/tab.rs:5627-5655`), and cosmic-player's welcome view is the
    /// same shape. It used to be `video-x-generic-symbolic`, which said
    /// "video" where the window can already say which video player this is.
    fn welcome(&self) -> Element<'_, Message> {
        let said = widget::column::with_capacity(2)
            .align_x(Alignment::Center)
            .spacing(8)
            .push(icon::from_svg_bytes(APP_ICON).icon().size(128))
            .push(widget::text::body(strings::NOTHING_OPEN));
        widget::column::with_capacity(4)
            .align_x(Alignment::Center)
            .spacing(24)
            .width(Length::Fill)
            .height(Length::Fill)
            .push(widget::space::vertical())
            .push(said)
            .push(widget::button::suggested(strings::OPEN_BUTTON).on_press(Message::FileOpen))
            .push(widget::space::vertical())
            .into()
    }

    /// The video, on whatever is behind it, with the controls over the bottom
    /// of it.
    ///
    /// A single click on the video does not toggle playback, which is the one
    /// place cosmic-player's pointer map cannot be copied: the same press
    /// starts the drag that looks around, and a control that fires on press
    /// cannot coexist with a grab that starts on press. Space does it, and so
    /// does the button in the row.
    ///
    /// The press still arrives here, and closing the volume popup is what it
    /// is for (issue #126). The pass does not capture it, deliberately, which
    /// is what leaves it for this `mouse_area`
    /// (`kjerag_render`'s widget, `ButtonPressed`).
    fn playing<'a>(&'a self, open: &'a Open) -> Element<'a, Message> {
        let video = widget::mouse_area(
            shader::Shader::new(&open.scene)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_press(Message::VideoAreaClick)
        .on_double_press(Message::Fullscreen);
        let stage = widget::container(video)
            .width(Length::Fill)
            .height(Length::Fill)
            .class(backdrop(self.fullscreen));

        let mut popover = widget::popover(stage).position(widget::popover::Position::Bottom);
        if self.controls.shown {
            popover = popover.popup(self.control_rows(open));
        }
        popover.into()
    }

    /// The control overlay: one row, or two when the window is too narrow to
    /// hold the buttons and the scrubber side by side
    /// (cosmic-player `src/main.rs:1999-2012`).
    fn control_rows(&self, open: &Open) -> Element<'_, Message> {
        let spacing = theme::active().cosmic().spacing;
        let condensed = self.core.is_condensed();

        let mut buttons = widget::row::with_capacity(8)
            .align_y(Alignment::Center)
            .spacing(spacing.space_xxs)
            .push(
                widget::button::icon(icon::from_svg_bytes(JUMP_BACKWARD_ICON).symbolic(true))
                    .on_press(Message::SeekRelative(-JUMP)),
            )
            .push(play_pause(open))
            .push(
                widget::button::icon(icon::from_svg_bytes(JUMP_FORWARD_ICON).symbolic(true))
                    .on_press(Message::SeekRelative(JUMP)),
            );

        if condensed {
            buttons = buttons.push(widget::space::horizontal());
        } else {
            for element in scrubber(open) {
                buttons = buttons.push(element);
            }
        }

        let mut rows = vec![bar(
            buttons
                // Issue #15. A frame capture is about the view rather than
                // the transport, so it joins fullscreen in the right hand
                // group (cosmic-player `src/main.rs:2013-2051`).
                .push(
                    widget::button::icon(icon::from_name("camera-photo-symbolic").size(16))
                        .on_press(Message::Capture(Destination::Save)),
                )
                .push(
                    widget::button::icon(icon::from_name("view-fullscreen-symbolic").size(16))
                        .on_press(Message::Fullscreen),
                )
                // Last, after fullscreen, which is cosmic-player's own order
                // (`src/main.rs:2013-2051`). A file with no sound draws it
                // with no `on_press`, which renders it disabled.
                .push(speaker(open, &self.stored.config, Message::AudioDropdown)),
            spacing,
        )];
        if condensed {
            let mut times = widget::row::with_capacity(3)
                .align_y(Alignment::Center)
                .spacing(spacing.space_xxs);
            for element in scrubber(open) {
                times = times.push(element);
            }
            rows.push(bar(times, spacing));
        }
        if self.controls.volume {
            // Above the row, right aligned under the button that opened it
            // (cosmic-player `src/main.rs:1899-1926`).
            rows.insert(0, volume_popup(open, &self.stored.config, spacing));
        }
        widget::column::with_children(rows).into()
    }

    /// The Settings page. Issue #15 adds the two screenshot rows.
    fn settings(&self) -> Element<'_, Message> {
        let selected = match self.stored.config.app_theme {
            AppTheme::System => 0,
            AppTheme::Dark => 1,
            AppTheme::Light => 2,
        };
        widget::settings::view_column(vec![
            widget::settings::section()
                .title(strings::APPEARANCE)
                .add(
                    widget::settings::item::builder(strings::THEME).control(widget::dropdown(
                        &self.themes,
                        Some(selected),
                        |index| {
                            Message::Theme(match index {
                                1 => AppTheme::Dark,
                                2 => AppTheme::Light,
                                _ => AppTheme::System,
                            })
                        },
                    )),
                )
                .into(),
        ])
        .into()
    }
}

/// One toast, built out of the same pieces libcosmic's own toaster builds one
/// out of (`src/widget/toaster/mod.rs:33-54`): the line, a close button, and
/// a tooltip-class container around both, with its paddings and spacings.
/// libcosmic's version has a second, optional action button in there; ours
/// never carries an action (docs/UI.md), so it is one button.
fn toast_line(toast: &Toast, spacing: cosmic_theme::Spacing) -> Element<'_, Message> {
    let inside = widget::row::with_capacity(2)
        .align_y(Alignment::Center)
        .spacing(spacing.space_s)
        .push(widget::text(&toast.message))
        .push(
            widget::button::icon(icon::from_name("window-close-symbolic"))
                .on_press(Message::CloseToast(toast.id)),
        );
    widget::container(inside)
        .padding([
            spacing.space_xxs,
            spacing.space_s,
            spacing.space_xxs,
            spacing.space_m,
        ])
        .class(theme::Container::Tooltip)
        .into()
}

/// What sits behind the video, which is what the room around the ball is made
/// of (issue #100).
///
/// The pass writes that room transparent rather than painting it, so this one
/// layer decides what shows there and the shader decides nothing:
///
/// - **In a window, nothing.** What comes through is libcosmic's own pane,
///   `background(theme.transparent).base` (`src/app/mod.rs:856-874`): a
///   translucent copy of the background colour while the theme is frosted, so
///   the ball floats on the compositor's blurred desktop, and the same colour
///   opaque when it is not. The welcome view sits on that same fill, which is
///   the look this matches, and the no-blur case needs no fallback of its own
///   because it is the same line of libcosmic either way.
/// - **In fullscreen, black.** There is no desktop behind a fullscreen window
///   to show through, and black is what a player puts around a picture.
///
/// cosmic-player paints exactly this black under its video and nothing under
/// its welcome view (`src/main.rs:1711-1714`).
fn backdrop(fullscreen: bool) -> theme::Container<'static> {
    theme::Container::custom(move |_| widget::container::Style {
        background: fullscreen
            .then_some(cosmic::iced::Background::Color(cosmic::iced::Color::BLACK)),
        ..Default::default()
    })
}

/// One row of the overlay: cosmic-player's padding and background
/// (`src/main.rs:2052-2060`), in a mouse area that re-arms the auto-hide
/// timer when the row itself is used.
fn bar<'a>(
    row: impl Into<Element<'a, Message>>,
    spacing: cosmic_theme::Spacing,
) -> Element<'a, Message> {
    widget::mouse_area(
        widget::container(row)
            .padding([spacing.space_xxs, spacing.space_xs])
            .class(theme::Container::WindowBackground),
    )
    .on_press(Message::ShowControls)
    .into()
}

/// The speaker button, which says what the sound is doing and is the way to
/// the slider. Its four icons and the two thirds they switch at are
/// cosmic-player's (`src/main.rs:2033-2051`).
fn speaker(open: &Open, config: &Config, press: Message) -> Element<'static, Message> {
    let name = match (config.muted, config.volume) {
        (true, _) => "audio-volume-muted-symbolic",
        (false, volume) if volume >= 2.0 / 3.0 => "audio-volume-high-symbolic",
        (false, volume) if volume >= 1.0 / 3.0 => "audio-volume-medium-symbolic",
        (false, _) => "audio-volume-low-symbolic",
    };
    let button = widget::button::icon(icon::from_name(name).size(16));
    match open.scene.has_sound() {
        true => button.on_press(press).into(),
        // No `on_press` is how libcosmic draws a button disabled, which is
        // what a file with no sound in it should show.
        false => button.into(),
    }
}

/// The volume slider, in a popup above the control row rather than in it
/// (cosmic-player `src/main.rs:1780-1807`, `1899-1926`).
///
/// cosmic-player styles the card with a hand-rolled container closure carrying
/// a `//TODO: move style to libcosmic` next to it (`src/main.rs:1905-1922`).
/// libcosmic has since moved it: `theme::Container::Dropdown` is the same
/// component base, divider border and small radius
/// (`src/theme/style/iced.rs:608-619`), so the stock one is what this uses.
fn volume_popup(
    open: &Open,
    config: &Config,
    spacing: cosmic_theme::Spacing,
) -> Element<'static, Message> {
    let inside = widget::row::with_capacity(2)
        .align_y(Alignment::Center)
        .spacing(spacing.space_xxs)
        .push(speaker(open, config, Message::AudioToggle))
        .push(
            Slider::new(0.0..=1.0, config.volume, Message::AudioVolume)
                .step(0.01)
                .on_release(Message::AudioVolumeRelease),
        );
    widget::row::with_capacity(2)
        .push(widget::space::horizontal())
        .push(
            widget::mouse_area(
                widget::container(
                    widget::container(inside).padding([spacing.space_xxs, spacing.space_m]),
                )
                .padding(1)
                .class(theme::Container::Dropdown)
                .width(Length::Fixed(VOLUME_POPUP)),
            )
            .on_press(Message::ShowControls),
        )
        .into()
}

fn play_pause(open: &Open) -> Element<'static, Message> {
    let name = match open.scene.is_playing() {
        true => "media-playback-pause-symbolic",
        false => "media-playback-start-symbolic",
    };
    widget::button::icon(icon::from_name(name).size(16))
        .on_press(Message::PlayPause)
        .into()
}

/// Elapsed, the scrubber, and the time left. Both labels are monospace, so
/// the row does not shuffle as the digits change, and the right hand one is
/// what is left rather than the total: for a 30-minute file that reads
/// `00:12:34` and `00:17:26`.
///
/// Dragging it seeks to keyframes and letting go seeks to the frame, which
/// is docs/UI.md's one deliberate deviation from cosmic-player: an accurate
/// seek per slider tick, on a dual 3840x3840 HEVC file, is a decode of every
/// frame since the last keyframe, twice, per pixel of drag.
fn scrubber(open: &Open) -> [Element<'static, Message>; 3] {
    let seconds = |time: Duration| time.as_secs_f64();
    [
        widget::text(strings::clock(open.position))
            .font(font::mono())
            .into(),
        Slider::new(
            0.0..=seconds(open.duration),
            seconds(open.position),
            Message::Seek,
        )
        .step(0.1)
        .on_release(Message::SeekRelease)
        .into(),
        widget::text(strings::clock(open.duration.saturating_sub(open.position)))
            .font(font::mono())
            .into(),
    ]
}

/// A position `seconds` away, which is a signed jump on an unsigned clock.
fn shift(from: Duration, seconds: f64) -> Duration {
    let by = Duration::from_secs_f64(seconds.abs());
    match seconds < 0.0 {
        true => from.saturating_sub(by),
        false => from.saturating_add(by),
    }
}

/// The codec a failed open could find no decoder for, and `None` for every
/// other failure (issue #69).
///
/// The engine hands the shell one boxed error from the whole open, and the box
/// arrives with whatever was put in it: `kjerag-media` refuses a stream whose
/// codec has no decoder with a [`MissingDecoder`], and nothing between here
/// and there re-wraps it. So this is a downcast rather than a string match.
///
/// The `'static` on the trait object is what makes the downcast legal: without
/// it the reference's own lifetime becomes the object's, and `downcast_ref` is
/// only implemented for `dyn Error + Send + Sync + 'static`.
fn missing_decoder(e: &(dyn std::error::Error + Send + Sync + 'static)) -> Option<&'static str> {
    Some(e.downcast_ref::<MissingDecoder>()?.codec)
}

/// What the alert says a failed open failed for. Four lines, in the order of
/// how much they can tell the pilot: the file is another camera's format
/// (issue #107), the sandbox was never shown it (issue #118), this build has
/// no decoder for it (issue #69), or nothing more is known than that it did
/// not open.
///
/// The path decides the second one rather than the error, and on purpose. A
/// file the sandbox has no mount for fails somewhere inside libav, which
/// answers "No such file or directory" for a file the pilot is looking at in
/// their file manager; asking the filesystem whether the path is there at all
/// is the same question with none of libav's spelling in it, and it keeps the
/// sandbox out of the layers below the shell (docs/ARCHITECTURE.md).
fn refusal(e: &(dyn std::error::Error + Send + Sync + 'static), path: &Path) -> String {
    if let Some(foreign) = e.downcast_ref::<Foreign>() {
        return strings::foreign(*foreign);
    }
    if sandboxed() && !path.exists() {
        return strings::out_of_reach();
    }
    strings::open_failed(missing_decoder(e))
}

/// Whether this is running inside a Flatpak, which is a fact about the run and
/// not about the platform: `/.flatpak-info` is the file flatpak mounts into
/// every sandbox, and asking for it is how every toolkit asks this.
fn sandboxed() -> bool {
    Path::new("/.flatpak-info").exists()
}

/// The XDG portal file chooser (cosmic-player `src/main.rs:1066-1085`).
///
/// The line it prints is the answer to a question the app cannot ask any
/// other way (issue #123): what the chooser hands back inside a sandbox. A
/// path under `/run/user/<uid>/doc/` is the document portal's translation of
/// the file, and its directory holds that file alone, which is why a capture
/// written as two files plays one lens when it is picked there. The portal's
/// own `Documents.Info` is refused to sandboxed callers ("Not allowed in
/// sandbox", measured), so the terminal is where this can be seen at all.
fn chooser() -> Task<Message> {
    Task::perform(
        async {
            let dialog = file_chooser::open::Dialog::new()
                .title(strings::OPEN_TITLE)
                .filter(FileFilter::new(strings::INSV_FILTER).glob("*.insv"));
            match dialog.open_file().await {
                Ok(response) => match response.url().to_file_path() {
                    Ok(path) => {
                        println!("chose:  {}", response.url());
                        action::app(Message::FileLoad(path))
                    }
                    Err(()) => {
                        eprintln!("kjerag: {} is not a local file", response.url());
                        action::none()
                    }
                },
                Err(file_chooser::Error::Cancelled) => action::none(),
                Err(e) => {
                    eprintln!("kjerag: no file chosen: {e}");
                    action::none()
                }
            }
        },
        |action| action,
    )
}

/// No `developers([...])`: that setter turns name and email pairs into
/// `mailto:` links, and this repository does not publish personal addresses.
/// The name is in `author` and contact goes through the repository link.
///
/// The widget takes an `icon::Handle` and draws it at 128 px
/// (libcosmic `src/widget/about.rs:132-141`), so the drawing goes in
/// directly. See [`APP_ICON`] for why it is not asked for by name.
fn about() -> About {
    About::default()
        .name(strings::APP_NAME)
        .icon(icon::from_svg_bytes(APP_ICON))
        .version(env!("CARGO_PKG_VERSION"))
        .author(strings::AUTHOR)
        .comments(strings::COMMENTS)
        .license(strings::LICENSE)
        .links([
            (strings::REPOSITORY, strings::REPOSITORY_URL),
            (strings::SUPPORT, strings::SUPPORT_URL),
        ])
}

/// What the shell decides on its own: which line a failed open puts in the
/// alert, what a paste turns out to be asking for, and the three rules
/// of the toast queue, which is ours now rather than libcosmic's and so is
/// tested rather than taken on trust.
#[cfg(test)]
mod tests {
    use super::*;

    /// What the room around the ball is made of (issue #100), read out of the
    /// theme the way iced reads it. The pass writes that room transparent, so
    /// this one style is the whole of the decision: nothing behind the video
    /// in a window, which leaves libcosmic's own pane showing through the
    /// room, and black in fullscreen.
    #[test]
    fn a_window_leaves_the_room_to_the_pane_and_fullscreen_fills_it_black() {
        use cosmic::iced::widget::container::Catalog;

        let theme = <theme::Theme as Default>::default();
        let behind = |fullscreen| theme.style(&backdrop(fullscreen)).background;

        assert_eq!(behind(false), None);
        assert_eq!(
            behind(true),
            Some(cosmic::iced::Background::Color(cosmic::iced::Color::BLACK))
        );
    }

    /// A line of the open file's own, as the clipboard would hold it: the
    /// name alone, which is all a copy ever carries.
    const COPIED: &str = "VID_0001.insv time=754.321 yaw=-37.42 pitch=8.06 fov=64.30 lock=1";
    /// And as the terminal prints it, with the path in front.
    const PRINTED: &str =
        "/home/pilot/Videos/VID_0001.insv time=754.321 yaw=-37.42 pitch=8.06 fov=64.30 lock=1";

    fn watching() -> PathBuf {
        PathBuf::from("/home/pilot/Videos/VID_0001.insv")
    }

    fn read(text: &str, open: Option<&Path>) -> Goto {
        Goto::read(Some(text), open)
    }

    /// (a) The file on screen, named either way. A copy carries the name and
    /// a printed line carries the path, and both are this video.
    #[test]
    fn a_reference_to_the_open_file_goes_there() {
        let open = watching();
        assert!(matches!(read(COPIED, Some(&open)), Goto::Here(_)));
        assert!(matches!(read(PRINTED, Some(&open)), Goto::Here(_)));

        let Goto::Here(framing) = read(COPIED, Some(&open)) else {
            panic!("not here");
        };
        assert!((framing.at.as_secs_f64() - 754.321).abs() < 0.000_5);
        assert!((framing.camera.yaw.to_degrees() + 37.42).abs() < 0.005);
        assert_eq!(framing.horizon, Horizon::Locked);
    }

    /// (c) A name that is not the open file's, with nothing to find it by.
    /// Nowhere to go, so the window says which video it belongs to and stays
    /// where it is.
    #[test]
    fn a_reference_to_another_video_says_which_one() {
        let open = watching();
        let elsewhere = COPIED.replace("VID_0001", "VID_0002");
        assert_eq!(
            read(&elsewhere, Some(&open)),
            Goto::Elsewhere(PathBuf::from("VID_0002.insv"))
        );
        // And with nothing open at all, which is the same answer: a name is
        // not enough to open anything.
        assert_eq!(
            read(COPIED, None),
            Goto::Elsewhere(PathBuf::from("VID_0001.insv"))
        );
    }

    /// (b) A whole path that is not the file on screen: open it, then go.
    /// That is the terminal line, which is the one a pilot has in a report.
    #[test]
    fn a_reference_carrying_a_path_opens_that_video() {
        let other = PathBuf::from("/home/pilot/Videos/VID_0002.insv");
        let printed = PRINTED.replace("VID_0001", "VID_0002");
        assert!(matches!(read(&printed, Some(&watching())), Goto::Open(file, _) if file == other));
        assert!(matches!(read(&printed, None), Goto::Open(file, _) if file == other));
    }

    /// Two videos of the same name in two folders are two videos, and only a
    /// line carrying the path can tell them apart.
    #[test]
    fn the_same_name_in_another_folder_is_another_video() {
        let printed = PRINTED.replace("/Videos/", "/Videos/2026/");
        assert!(matches!(
            read(&printed, Some(&watching())),
            Goto::Open(_, _)
        ));
    }

    /// (d) Everything else a clipboard holds, which is nearly all of it.
    /// Nothing happens and nothing is said: `Ctrl+V` over a video means
    /// nothing in any other player either.
    #[test]
    fn anything_that_is_not_a_reference_does_nothing() {
        let open = watching();
        for text in [
            None,
            Some(""),
            Some("https://example.com"),
            Some("VID.insv"),
        ] {
            assert_eq!(Goto::read(text, Some(&open)), Goto::Nothing, "{text:?}");
        }
    }

    /// The shell has to tell a build with no decoder apart from a file it
    /// cannot read, because they get different lines and only one of them is
    /// the pilot's to fix (issue #69). This is that test with the probe stood
    /// in for: the error is built by hand, the way `kjerag-media` builds it on
    /// a box whose ffmpeg has no HEVC in it.
    #[test]
    fn a_missing_decoder_is_told_apart_from_a_file_that_will_not_open() {
        let missing: Box<dyn std::error::Error + Send + Sync> =
            Box::new(MissingDecoder { codec: "hevc" });
        assert_eq!(missing_decoder(&*missing), Some("hevc"));
        assert!(strings::open_failed(missing_decoder(&*missing)).contains("HEVC"));

        let broken: Box<dyn std::error::Error + Send + Sync> = "file has no video stream".into();
        assert_eq!(missing_decoder(&*broken), None);
        assert_eq!(
            strings::open_failed(missing_decoder(&*broken)),
            strings::OPEN_FAILED
        );
    }

    /// And the lines a failed open can leave, told apart by the type in the
    /// box rather than by anything in the message (issue #107). The foreign
    /// one names the format; the others are what they were.
    ///
    /// The path is one that exists, so nothing here takes the sandbox arm:
    /// that one is a fact about the run and the test below says what it is.
    #[test]
    fn another_cameras_format_gets_a_line_of_its_own() {
        let here = Path::new(file!());
        let gopro: Box<dyn std::error::Error + Send + Sync> = Box::new(Foreign::GoPro);
        assert_eq!(refusal(&*gopro, here), strings::foreign(Foreign::GoPro));
        assert!(
            refusal(&*gopro, here).contains("GoPro"),
            "{}",
            refusal(&*gopro, here)
        );

        let missing: Box<dyn std::error::Error + Send + Sync> =
            Box::new(MissingDecoder { codec: "hevc" });
        assert!(refusal(&*missing, here).contains("HEVC"));

        let broken: Box<dyn std::error::Error + Send + Sync> = "file has no video stream".into();
        assert_eq!(refusal(&*broken, here), strings::OPEN_FAILED);
    }

    /// A path that is not there says so in the sandbox's words and nowhere
    /// else (issue #118). Outside a Flatpak the same open is a file that was
    /// deleted or renamed, and "Kjerag cannot reach that file from inside its
    /// sandbox" would be a sentence about a sandbox that is not there.
    ///
    /// Both arms are exercised wherever this runs, because what decides is
    /// `/.flatpak-info` and not the test: in CI and on a developer box the
    /// first is what a run gets, and inside the Flatpak the second is, which
    /// is the build the line was written for.
    #[test]
    fn a_path_the_sandbox_cannot_see_is_not_a_missing_file() {
        let gone = Path::new("/nowhere/at/all/flight.insv");
        let broken: Box<dyn std::error::Error + Send + Sync> = "No such file or directory".into();
        let expected = match sandboxed() {
            true => strings::out_of_reach(),
            false => strings::OPEN_FAILED.to_owned(),
        };
        assert_eq!(refusal(&*broken, gone), expected);
        assert!(strings::out_of_reach().contains(strings::OPEN_TITLE));
    }

    fn lines(toasts: &Toasts) -> Vec<&str> {
        toasts
            .lines
            .iter()
            .map(|toast| toast.message.as_str())
            .collect()
    }

    #[test]
    fn five_are_kept_and_the_oldest_goes_first() {
        let mut toasts = Toasts::default();
        for i in 0..7 {
            toasts.push(i.to_string());
        }
        assert_eq!(lines(&toasts), ["2", "3", "4", "5", "6"]);
    }

    /// Ids are never reused, so the close button on a line that is still up
    /// cannot take away a later one that landed in its place.
    #[test]
    fn closing_one_leaves_the_rest() {
        let mut toasts = Toasts::default();
        let first = toasts.push("first".to_owned());
        toasts.push("second".to_owned());
        toasts.close(first);
        assert_eq!(lines(&toasts), ["second"]);
        toasts.close(first);
        assert_eq!(lines(&toasts), ["second"]);
    }

    /// A line dropped for being the sixth still has five seconds of its own
    /// left to run, and what it names by then may be a line the pilot has
    /// only just been shown.
    #[test]
    fn a_dropped_line_dismisses_nothing_later() {
        let mut toasts = Toasts::default();
        let mut dropped = 0;
        for i in 0..6 {
            let id = toasts.push(i.to_string());
            if i == 0 {
                dropped = id;
            }
        }
        assert_eq!(lines(&toasts), ["1", "2", "3", "4", "5"]);
        toasts.close(dropped);
        assert_eq!(lines(&toasts), ["1", "2", "3", "4", "5"]);
    }
}
