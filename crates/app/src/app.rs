//! The libcosmic shell: one window whose body is the video, with a menu bar
//! in the header and a control overlay at the bottom.
//!
//! The shell owns almost nothing of the playback. It opens the file, it turns
//! keys and buttons into transport calls, and it asks the [`Scene`] for a
//! frame once per window redraw. The clock, the decode thread and the camera
//! all live below it in `kyerag-render` and `kyerag-media`.
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
//! instant the next one is due (`kyerag_render`'s `tick`, which is where the
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
use kyerag_render::{Accuracy, Horizon, MissingDecoder, Nudge, Request, Scene, SeamFit, Stats};

use crate::config::{AppTheme, CONFIG_VERSION, Config, ConfigState, Stored};
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
/// same setter (`src/main.rs:1454-1468`): a name resolves through the icon
/// theme, and these icons are installed as `dev.harding.Kjerag` while the
/// binary still calls itself `app.kyerag.Kyerag`, so the lookup finds nothing.
///
/// TODO(#75): revisit at the rename, remembering that a name still resolves
/// to nothing for a build run out of the source tree.
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
pub fn run(input: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let stored = Stored::load(App::APP_ID);
    let settings = Settings::default()
        .size_limits(Limits::NONE.min_width(360.0).min_height(240.0))
        // The window opens in the configured theme rather than flashing the
        // default one first (cosmic-player `src/main.rs:154-155`).
        .theme(stored.config.app_theme.theme());
    cosmic::app::run::<App>(settings, Flags { stored, input })?;
    Ok(())
}

pub struct Flags {
    stored: Stored,
    input: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum Message {
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
    /// `View > Calibrate seam from this video`: measure this camera's seam off
    /// the open file and keep the answer (issue #48).
    CalibrateSeam,
    /// It came back, a second or two later, off a worker thread. The camera
    /// travels with it, because the answer is stored under the camera and the
    /// pilot may have opened something else while it ran.
    Calibrated(u64, Result<SeamFit, String>),
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
    /// The line under the welcome view's first, set when the last attempt to
    /// open a file did not work. It holds the line rather than a flag because
    /// which line it is depends on why the open failed (issue #69).
    failed: Option<String>,
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

/// The overlay's visibility, and when the pointer last asked for it.
struct Controls {
    shown: bool,
    since: Instant,
    /// The volume slider, which sits in a popup above the row rather than in
    /// it (cosmic-player `src/main.rs:1777-1807`). It goes when the row goes,
    /// which cosmic-player does the other way round: it holds the controls up
    /// for as long as a dropdown is open (`src/main.rs:1627`). Ours cannot,
    /// because a drag to look around is pointer input, so the row would never
    /// time out and the dropdown would never close.
    volume: bool,
}

impl cosmic::Application for App {
    type Executor = executor::Default;
    type Flags = Flags;
    type Message = Message;

    const APP_ID: &'static str = "app.kyerag.Kyerag";

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
        core.window.border_padding = Some(0);

        let mut app = App {
            core,
            open: None,
            failed: None,
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
        (app, task)
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        let now = Instant::now();
        match message {
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
                    eprintln!("kyerag: that drop carried no local file");
                    self.failed = Some(strings::open_failed(None));
                    return Task::none();
                };
                return self.update(Message::FileLoad(path));
            }
            Message::FileClearRecents => {
                self.stored.state.recent_files.clear();
                self.stored.write_state();
            }
            Message::FileClose => {
                self.open = None;
                self.failed = None;
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
                self.controls.volume = false;
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
                    eprintln!("kyerag: {url} not opened: {e}");
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
            Message::CalibrateSeam => {
                self.show_controls(now);
                return self.calibrate();
            }
            Message::Calibrated(camera, fit) => return self.calibrated(camera, fit),
            Message::Seek(seconds) => {
                let position = Duration::from_secs_f64(seconds.max(0.0));
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
            Message::Quit => std::process::exit(0),
            Message::Report => self.report(now),
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
        }
        Task::none()
    }

    /// Escape closes the context drawer first and leaves fullscreen second
    /// (cosmic-edit `src/main.rs:1583-1592`). libcosmic gives Escape to the
    /// app through this hook, which is why the key map does not bind it.
    fn on_escape(&mut self) -> Task<Self::Message> {
        if self.core.window.show_context {
            self.core.window.show_context = false;
            return Task::none();
        }
        if self.fullscreen {
            return self.update(Message::Fullscreen);
        }
        Task::none()
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
            self.has_seam(),
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

    /// Opens a file, or leaves the welcome view up with a line saying it did
    /// not work. cosmic-player only logs (`src/video.rs:63`), which leaves the
    /// pilot staring at an unchanged window; a player with exactly one job
    /// should say when it cannot do it.
    fn load(&mut self, path: &Path) {
        match Scene::open(path) {
            Ok(scene) => {
                self.failed = None;
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
                eprintln!("kyerag: {} not shown: {e}", path.display());
                self.failed = Some(strings::open_failed(missing_decoder(&*e)));
                self.open = None;
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

    /// Hand this camera's seam calibration to the scene, before its first
    /// frame is drawn (issue #48).
    ///
    /// A camera this box has never been asked to calibrate falls back to a fit
    /// off this file's own frames, which is the weaker answer for the reason
    /// 6.8 measures: a flight's own seam carries that flight's parallax, and a
    /// fit taken through it absorbs some. That is the whole of the difference
    /// between the two paths here.
    fn hold_seam(&self, scene: &Scene) {
        match scene.camera_key().and_then(|c| self.stored.state.seam(c)) {
            Some(fit) => scene.use_seam(fit),
            None => scene.fit_seam(),
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

    /// Whether there is a seam to calibrate: `false` for no file and for a
    /// capture that carries one lens stream (issue #79's camera, a file at a
    /// time).
    fn has_seam(&self) -> bool {
        self.open.as_ref().is_some_and(|open| open.scene.has_seam())
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
        self.controls.volume = false;
        self.core.window.show_headerbar = false;
        self.hide_cursor(true);
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
        let (finished, waiting) = oneshot::channel();
        open.scene.capture(Request {
            width: shot::WIDTH,
            then: Box::new(move |taken| {
                let done = taken
                    .and_then(|still| shot::finish(&still, &video, to))
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
                eprintln!("kyerag: no still: {e}");
                self.toast(strings::capture_failed(to, &e))
            }
        }
    }

    /// Measures this camera's seam off the open file and keeps the answer
    /// (issue #48).
    ///
    /// The run reads real frames from three places in the file, so it is a
    /// second or two and it happens on a thread of its own; the window keeps
    /// playing while it does. A capture from a camera standing still is what
    /// this wants pointed at it, and where the app says so is docs/UI.md's
    /// line in the menu plus the report line a file with no calibration
    /// prints.
    ///
    /// A thread and a channel rather than the async runtime's blocking pool,
    /// which is what [`Self::capture`] does two functions up and for the same
    /// reason: the work is a decoder, it belongs on a thread that is allowed
    /// to sit in `read`, and this way the shell asks the runtime for nothing
    /// but the wakeup.
    fn calibrate(&self) -> Task<Message> {
        let Some(open) = &self.open else {
            return Task::none();
        };
        let (Some(camera), Some(job)) = (open.scene.camera_key(), open.scene.seam_job()) else {
            return Task::none();
        };
        let path = open.path.clone();
        let (finished, waiting) = oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name("seam calibration".to_owned())
            .spawn(move || {
                let _ = finished.send(job.run(&path));
            });
        if let Err(e) = spawned {
            return cosmic::task::message(cosmic::Action::App(Message::Calibrated(
                camera,
                Err(e.to_string()),
            )));
        }
        Task::perform(waiting, move |done| {
            action::app(Message::Calibrated(
                camera,
                done.unwrap_or_else(|_| Err("the calibration did not finish".to_owned())),
            ))
        })
    }

    /// Says what the calibration came to, keeps it, and puts it into the
    /// picture that is already on screen if that picture came off the same
    /// camera.
    fn calibrated(&mut self, camera: u64, fit: Result<SeamFit, String>) -> Task<Message> {
        let fit = match fit {
            Ok(fit) => fit,
            Err(e) => {
                eprintln!("kyerag: the seam was not calibrated: {e}");
                return self.toast(strings::calibration_failed(&e));
            }
        };
        self.stored.state.calibrate(camera, fit);
        self.stored.write_state();
        if let Some(open) = &self.open {
            // Not only the file it was measured on: any file from that camera
            // that happens to be open is drawn with it from here.
            if open.scene.camera_key() == Some(camera) {
                open.scene.use_seam(fit);
            }
        }
        self.toast(strings::SEAM_CALIBRATED.to_owned())
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
        let mut said = widget::column::with_capacity(3)
            .align_x(Alignment::Center)
            .spacing(8)
            .push(icon::from_svg_bytes(APP_ICON).icon().size(128))
            .push(widget::text::body(strings::NOTHING_OPEN));
        if let Some(line) = &self.failed {
            said = said.push(widget::text::body(line.as_str()));
        }
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

    /// The video, with the controls over the bottom of it.
    ///
    /// A single click on the video does not toggle playback, which is the one
    /// place cosmic-player's pointer map cannot be copied: the same press
    /// starts the drag that looks around, and a control that fires on press
    /// cannot coexist with a grab that starts on press. Space does it, and so
    /// does the button in the row.
    fn playing<'a>(&'a self, open: &'a Open) -> Element<'a, Message> {
        let video = widget::mouse_area(
            shader::Shader::new(&open.scene)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_double_press(Message::Fullscreen);

        let mut popover = widget::popover(video).position(widget::popover::Position::Bottom);
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
/// arrives with whatever was put in it: `kyerag-media` refuses a stream whose
/// codec has no decoder with a [`MissingDecoder`], and nothing between here
/// and there re-wraps it. So this is a downcast rather than a string match.
///
/// The `'static` on the trait object is what makes the downcast legal: without
/// it the reference's own lifetime becomes the object's, and `downcast_ref` is
/// only implemented for `dyn Error + Send + Sync + 'static`.
fn missing_decoder(e: &(dyn std::error::Error + Send + Sync + 'static)) -> Option<&'static str> {
    Some(e.downcast_ref::<MissingDecoder>()?.codec)
}

/// The XDG portal file chooser (cosmic-player `src/main.rs:1066-1085`).
fn chooser() -> Task<Message> {
    Task::perform(
        async {
            let dialog = file_chooser::open::Dialog::new()
                .title(strings::OPEN_TITLE)
                .filter(FileFilter::new(strings::INSV_FILTER).glob("*.insv"));
            match dialog.open_file().await {
                Ok(response) => match response.url().to_file_path() {
                    Ok(path) => action::app(Message::FileLoad(path)),
                    Err(()) => {
                        eprintln!("kyerag: {} is not a local file", response.url());
                        action::none()
                    }
                },
                Err(file_chooser::Error::Cancelled) => action::none(),
                Err(e) => {
                    eprintln!("kyerag: no file chosen: {e}");
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

/// What the shell decides on its own: which line a failed open leaves on the
/// welcome view, and the three rules of the toast queue, which is ours now
/// rather than libcosmic's and so is tested rather than taken on trust.
#[cfg(test)]
mod tests {
    use super::*;

    /// The shell has to tell a build with no decoder apart from a file it
    /// cannot read, because they get different lines and only one of them is
    /// the pilot's to fix (issue #69). This is that test with the probe stood
    /// in for: the error is built by hand, the way `kyerag-media` builds it on
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
