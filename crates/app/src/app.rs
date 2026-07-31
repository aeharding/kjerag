//! The libcosmic shell: one window whose whole body is an iced shader widget.
//!
//! The shell owns almost nothing. It opens the file, it turns space into
//! play/pause, and it asks the [`Scene`] for a frame once per window redraw.
//! Everything else, the clock and the decode thread and the camera, lives
//! below it in `kyerag-render` and `kyerag-media`.
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

use std::path::PathBuf;
use std::time::{Duration, Instant};

use cosmic::app::{Core, Settings, Task};
use cosmic::iced::futures::channel::oneshot;
use cosmic::iced::runtime::clipboard;
use cosmic::iced::widget::shader;
use cosmic::iced::{Length, Subscription, event, keyboard, time, window};
use cosmic::{Element, action, executor};
use kyerag_render::{Request, Scene, Stats};

use crate::shot::{self, Destination, Done};

/// How often the playback report is printed while playing. It is the only
/// way to see dropped frames without a profiler; issue #16's chrome is where
/// that belongs on screen.
const REPORT_EVERY: Duration = Duration::from_secs(5);

/// Runs the shell. With no path the widget draws an animated WGSL gradient
/// and nothing is decoded; with one it plays the file.
pub fn run(input: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let limits = cosmic::iced::Limits::NONE
        .min_width(360.0)
        .min_height(240.0);
    cosmic::app::run::<App>(Settings::default().size_limits(limits), input)?;
    Ok(())
}

#[derive(Clone, Debug)]
pub enum Message {
    /// Five seconds have passed and playback has a line to print.
    Report,
    TogglePlay,
    /// Take a still of the view as it stands (issue #15).
    Capture(Destination),
    /// One came back, some milliseconds later, off the render thread.
    Captured(Result<Done, String>),
}

pub struct App {
    core: Core,
    scene: Scene,
    /// The file being played, which is half of a still's name.
    video: Option<PathBuf>,
    reported: Instant,
    /// The counters as of the last report, so each line covers its own five
    /// seconds instead of the whole run.
    counted: Stats,
}

impl cosmic::Application for App {
    type Executor = executor::Default;
    type Flags = Option<PathBuf>;
    type Message = Message;

    const APP_ID: &'static str = "app.kyerag.Kyerag";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(mut core: Core, input: Self::Flags) -> (Self, Task<Self::Message>) {
        // libcosmic's content container insets the app's view by
        // `border_padding` on the right and, because `nav_bar.active`
        // defaults to true even for an app with no nav model, by nothing on
        // the left (`app/mod.rs`, `main_content_padding`). Measured at scale
        // 1.25: 1 physical px of window border on the left against 10 on the
        // right. Video wants both edges, so the container comes off.
        core.window.content_container = false;
        let (scene, video) = open(input);
        (
            App {
                core,
                scene,
                video,
                reported: Instant::now(),
                counted: Stats::default(),
            },
            Task::none(),
        )
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        let now = Instant::now();
        match message {
            Message::Report => self.report(now),
            Message::TogglePlay => self.scene.toggle_play(now),
            Message::Capture(to) => return self.capture(to),
            Message::Captured(shot) => return report_shot(shot),
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let mut sources = vec![event::listen_with(key_pressed)];
        if self.scene.is_playing() {
            sources.push(time::every(REPORT_EVERY).map(|_| Message::Report));
        }
        Subscription::batch(sources)
    }

    fn view(&self) -> Element<'_, Self::Message> {
        Element::from(
            shader::Shader::new(&self.scene)
                .width(Length::Fill)
                .height(Length::Fill),
        )
    }
}

impl App {
    /// Arms a capture of the next frame drawn, and waits for it.
    ///
    /// Nothing here touches the picture: the render pass takes the request
    /// on its next redraw, a worker thread reads the pixels back and either
    /// writes the PNG or encodes it for the clipboard, and this task is
    /// woken when that is done. The clipboard is the one step that has to
    /// come back to the shell, because on Wayland it is the window that
    /// offers the data.
    fn capture(&self, to: Destination) -> Task<Message> {
        let Some(video) = self.video.clone() else {
            return Task::none();
        };
        let (finished, waiting) = oneshot::channel();
        self.scene.capture(Request {
            width: shot::WIDTH,
            then: Box::new(move |taken| {
                let done = taken
                    .and_then(|shot| shot::finish(&shot, &video, to))
                    .map_err(|e| e.to_string());
                let _ = finished.send(done);
            }),
        });
        Task::perform(waiting, |done| {
            action::app(Message::Captured(done.unwrap_or_else(|_| {
                Err("the capture was replaced before it was taken".to_owned())
            })))
        })
    }

    fn report(&mut self, now: Instant) {
        let Some(stats) = self.scene.stats() else {
            return;
        };
        println!(
            "play:   {:>8.2} s, {}",
            self.scene.position(now).as_secs_f64(),
            stats
                .since(self.counted)
                .report(now.duration_since(self.reported)),
        );
        self.counted = stats;
        self.reported = now;
    }
}

/// Says where the still went, and puts it on the clipboard when that is
/// what was asked for.
///
/// A line on the terminal is all the feedback there is today. docs/UI.md
/// asks for a toast here and leaves its wording, and whether it carries an
/// action, as an open question for the owner; the control row and the File
/// menu items that fire this are the app shell's (issue #16). Neither is
/// this PR's to settle.
fn report_shot(shot: Result<Done, String>) -> Task<Message> {
    match shot {
        Ok(Done::Saved(path)) => {
            println!("shot:   {}", path.display());
            Task::none()
        }
        Ok(Done::Copied(png)) => {
            println!("shot:   copied");
            clipboard::write_data(png)
        }
        Err(e) => {
            eprintln!("kyerag: no still: {e}");
            Task::none()
        }
    }
}

/// A file that will not open leaves the window up with the gradient in it: a
/// player that vanishes on a bad path is harder to report than one that says
/// why on the terminal.
fn open(input: Option<PathBuf>) -> (Scene, Option<PathBuf>) {
    let Some(path) = input else {
        return (Scene::blank(), None);
    };
    match Scene::open(&path) {
        Ok(scene) => (scene, Some(path)),
        Err(e) => {
            eprintln!("kyerag: {} not shown: {e}", path.display());
            (Scene::blank(), None)
        }
    }
}

/// Space toggles play, `s` saves a still, and `Ctrl+C` copies one
/// (docs/UI.md's keyboard table).
///
/// It is the physical key that is matched, not the character it types: a
/// transport control should not move with the keyboard layout. iced's
/// `keyboard::key::Named` has no `Space` in it at all, so the layout-shaped
/// match is not even available here.
///
/// `Ignored` keeps this from firing while a widget that wants the key has
/// it; nothing in this window does today.
///
/// The app shell (issue #16) replaces this with libcosmic's `KeyBind` map,
/// which is what draws the same accelerators in the menu.
fn key_pressed(event: event::Event, status: event::Status, _: window::Id) -> Option<Message> {
    use keyboard::key::{Code, Physical};

    let event::Event::Keyboard(keyboard::Event::KeyPressed {
        physical_key,
        modifiers,
        ..
    }) = event
    else {
        return None;
    };
    if status != event::Status::Ignored {
        return None;
    }
    match physical_key {
        Physical::Code(Code::Space) => Some(Message::TogglePlay),
        Physical::Code(Code::KeyS) if modifiers.is_empty() => {
            Some(Message::Capture(Destination::Save))
        }
        Physical::Code(Code::KeyC) if modifiers.control() => {
            Some(Message::Capture(Destination::Copy))
        }
        _ => None,
    }
}
