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
use cosmic::iced::widget::shader;
use cosmic::iced::{Length, Subscription, event, keyboard, time, window};
use cosmic::{Element, executor};
use kyerag_render::{Scene, Stats};

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
}

pub struct App {
    core: Core,
    scene: Scene,
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
        (
            App {
                core,
                scene: open(input),
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
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let mut sources = vec![event::listen_with(space_bar)];
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

/// A file that will not open leaves the window up with the gradient in it: a
/// player that vanishes on a bad path is harder to report than one that says
/// why on the terminal.
fn open(input: Option<PathBuf>) -> Scene {
    let Some(path) = input else {
        return Scene::blank();
    };
    match Scene::open(&path) {
        Ok(scene) => scene,
        Err(e) => {
            eprintln!("kyerag: {} not shown: {e}", path.display());
            Scene::blank()
        }
    }
}

/// Space toggles play.
///
/// It is the physical key that is matched, not the character it types: a
/// transport control should not move with the keyboard layout. iced's
/// `keyboard::key::Named` has no `Space` in it at all, so the layout-shaped
/// match is not even available here.
///
/// `Ignored` keeps this from firing while a widget that wants the key has
/// it; nothing in this window does today.
fn space_bar(event: event::Event, status: event::Status, _: window::Id) -> Option<Message> {
    let event::Event::Keyboard(keyboard::Event::KeyPressed { physical_key, .. }) = event else {
        return None;
    };
    match (physical_key, status) {
        (keyboard::key::Physical::Code(keyboard::key::Code::Space), event::Status::Ignored) => {
            Some(Message::TogglePlay)
        }
        _ => None,
    }
}
