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
use cosmic::iced::keyboard::key::{Key, Physical};
use cosmic::iced::keyboard::{Event as KeyEvent, Modifiers};
use cosmic::iced::mouse::Event as MouseEvent;
use cosmic::iced::widget::shader;
use cosmic::iced::window::{self, Mode};
use cosmic::iced::{Alignment, Length, Limits, Subscription, time};
use cosmic::widget::about::About;
use cosmic::widget::dnd_destination::dnd_destination_for_data;
use cosmic::widget::menu::Action as _;
use cosmic::widget::menu::key_bind::KeyBind;
use cosmic::widget::{self, icon};
use cosmic::{Application, ApplicationExt, Element, action, cosmic_theme, executor, font, theme};
use kyerag_render::{Nudge, Scene, Stats};

use crate::config::{AppTheme, CONFIG_VERSION, Config, ConfigState, Stored};
use crate::dnd::Dropped;
use crate::key_bind::{Action, key_binds};
use crate::{menu, strings};

/// Icons for the two jump buttons, which are not in the icon theme.
/// cosmic-player ships them in its own `res/` and so do we (`res/icons/`,
/// GPL-3.0, attributed in the files themselves).
const JUMP_BACKWARD_ICON: &[u8] = include_bytes!("../res/icons/jump-backward-10-symbolic.svg");
const JUMP_FORWARD_ICON: &[u8] = include_bytes!("../res/icons/jump-forward-10-symbolic.svg");

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
    /// A view change from the `View` menu or its keys.
    Look(Nudge),
    PlayPause,
    Quit,
    /// Five seconds have passed and playback has a line to print.
    Report,
    /// An action whose capability has not landed yet: the four seek and
    /// frame-step items (issue #5) and the two frame ones (issue #15). They
    /// are in the key map and in the menu so that the menu is complete and
    /// draws their accelerators; this is what they do until then.
    NotYet,
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
    /// Set when the last attempt to open a file did not work, which puts a
    /// second line under the welcome view's first.
    failed: bool,
    stored: Stored,
    key_binds: HashMap<KeyBind, Action>,
    about: About,
    context_page: ContextPage,
    /// The theme names the settings dropdown shows, in its own order.
    themes: Vec<String>,
    controls: Controls,
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

/// The overlay's visibility, and when the pointer last asked for it.
struct Controls {
    shown: bool,
    since: Instant,
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
        // libcosmic's content container insets the app's view by
        // `border_padding` on the right and, because `nav_bar.active`
        // defaults to true even for an app with no nav model, by nothing on
        // the left (`app/mod.rs`, `main_content_padding`). Measured at scale
        // 1.25: 1 physical px of window border on the left against 10 on the
        // right. Video wants both edges, so the container comes off.
        core.window.content_container = false;

        let mut app = App {
            core,
            open: None,
            failed: false,
            stored: flags.stored,
            key_binds: key_binds(),
            about: about(),
            context_page: ContextPage::Settings,
            themes: vec![
                strings::THEME_SYSTEM.to_owned(),
                strings::THEME_DARK.to_owned(),
                strings::THEME_LIGHT.to_owned(),
            ],
            controls: Controls {
                shown: true,
                since: Instant::now(),
            },
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
            Message::Config(config) => {
                self.stored.config = config;
                return cosmic::command::set_theme(self.stored.config.app_theme.theme());
            }
            Message::ConfigState(state) => self.stored.state = state,
            Message::Dropped(dropped) => {
                // First file wins, others are ignored.
                let Some(path) = dropped.and_then(|files| files.0.into_iter().next()) else {
                    eprintln!("kyerag: that drop carried no local file");
                    self.failed = true;
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
                self.failed = false;
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
            // Issues #5 and #15 are what make these do something.
            Message::NotYet => {}
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
        let content = match &self.open {
            Some(open) => self.playing(open),
            None => self.welcome(),
        };
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
                self.failed = false;
                self.open = Some(Open {
                    path: path.to_path_buf(),
                    duration: scene.duration(),
                    position: Duration::ZERO,
                    scene,
                });
                self.stored.state.remember(path);
                self.stored.write_state();
            }
            Err(e) => {
                eprintln!("kyerag: {} not shown: {e}", path.display());
                self.failed = true;
                self.open = None;
            }
        }
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
        self.core.window.show_headerbar = false;
        self.hide_cursor(true);
    }

    fn hide_cursor(&mut self, hidden: bool) {
        if let Some(open) = &mut self.open {
            open.scene.hide_cursor(hidden);
        }
    }

    fn read_clock(&mut self, now: Instant) {
        if let Some(open) = &mut self.open {
            open.position = open.scene.position(now).min(open.duration);
        }
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
    fn welcome(&self) -> Element<'_, Message> {
        let mut said = widget::column::with_capacity(3)
            .align_x(Alignment::Center)
            .spacing(8)
            .push(widget::icon::from_name("video-x-generic-symbolic").size(64))
            .push(widget::text::body(strings::NOTHING_OPEN));
        if self.failed {
            said = said.push(widget::text::body(strings::OPEN_FAILED));
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
            // Seeking is issue #5: no `on_press` renders a button disabled,
            // which is how the row can exist before the capability does.
            .push(widget::button::icon(
                icon::from_svg_bytes(JUMP_BACKWARD_ICON).symbolic(true),
            ))
            .push(play_pause(open))
            .push(widget::button::icon(
                icon::from_svg_bytes(JUMP_FORWARD_ICON).symbolic(true),
            ));

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
                .push(widget::button::icon(
                    icon::from_name("camera-photo-symbolic").size(16),
                ))
                .push(
                    widget::button::icon(icon::from_name("view-fullscreen-symbolic").size(16))
                        .on_press(Message::Fullscreen),
                ),
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
/// docs/UI.md's scrubber is a `Slider`, and this one is not yet: it shows the
/// position and takes no input, because a slider that moves and seeks nothing
/// is a broken control rather than an unfinished one. Issue #5 is where the
/// drag becomes a seek to the nearest keyframe and the release an accurate
/// one, and where this becomes the slider.
fn scrubber(open: &Open) -> [Element<'static, Message>; 3] {
    let through = match open.duration.is_zero() {
        true => 0.0,
        false => (open.position.as_secs_f64() / open.duration.as_secs_f64()) as f32,
    };
    [
        widget::text(strings::clock(open.position))
            .font(font::mono())
            .into(),
        widget::determinate_linear(through)
            .width(Length::Fill)
            .into(),
        widget::text(strings::clock(open.duration - open.position))
            .font(font::mono())
            .into(),
    ]
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
fn about() -> About {
    About::default()
        .name(strings::APP_NAME)
        .icon(icon::from_name(App::APP_ID))
        .version(env!("CARGO_PKG_VERSION"))
        .author(strings::AUTHOR)
        .comments(strings::COMMENTS)
        .license(strings::LICENSE)
        .links([
            (strings::REPOSITORY, strings::REPOSITORY_URL),
            (strings::SUPPORT, strings::SUPPORT_URL),
        ])
}
