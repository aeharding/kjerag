//! What the app keeps between runs.
//!
//! Two cosmic-config entries, which is the split every first-party COSMIC app
//! uses (docs/UI.md, "Persistence"): [`Config`] is what the pilot chose, and
//! [`ConfigState`] is what the app noticed. They live in different
//! directories, so a settings reset does not also forget the recent files.
//!
//! A handler is `None` when the config directory could not be opened. The app
//! then runs on defaults and forgets on exit rather than refusing to start: a
//! player that will not open a video because it cannot write a preferences
//! file is worse than one that forgets.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use cosmic::cosmic_config::cosmic_config_derive::CosmicConfigEntry;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::theme;
use serde::{Deserialize, Serialize};

pub const CONFIG_VERSION: u64 = 1;

/// Recent files remembered. cosmic-player's number (`src/main.rs:397-401`).
const RECENT: usize = 10;

/// Which theme the window opens in. Verbatim cosmic-player's shape
/// (`src/config.rs:13-27`).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AppTheme {
    Dark,
    Light,
    System,
}

impl AppTheme {
    pub fn theme(self) -> theme::Theme {
        match self {
            Self::Dark => theme::Theme::dark(),
            Self::Light => theme::Theme::light(),
            Self::System => theme::system_preference(),
        }
    }
}

/// Things the pilot chose. Issue #15 adds the screenshot folder and scale.
///
/// Not `Eq`: the volume is a fraction, and a fraction has no total equality.
#[derive(Clone, CosmicConfigEntry, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Config {
    pub app_theme: AppTheme,
    /// Hold the picture against the world rather than the camera (issue #8).
    ///
    /// **On by default**, because of what the footage looks like without it:
    /// this camera is clamped rolled about a quarter turn and pitched down,
    /// so an unlocked view of a paramotor flight has its horizon running down
    /// the picture and swinging, and the reframed view inherits every swing
    /// of a camera hanging under a wing. Measured over three seconds of calm
    /// flight, the horizon in a locked view moves 0.24 degrees peak to peak
    /// against 3.19 unlocked, and in a wingover 2.76 against a horizon that
    /// leaves the picture entirely. `View > Lock horizon` and `h` flip it
    /// live for anyone who wants the camera's own view.
    pub horizon_lock: bool,
    /// Loudness, 0 to 1 (issue #13).
    ///
    /// cosmic-player keeps neither this nor [`Config::muted`]: its volume is a
    /// GStreamer playbin property and starts at 1 every run. Remembering them
    /// is the owner's ask, and it suits this player: a paramotor track is
    /// half an hour of wind noise, so whoever turns it down means it.
    pub volume: f64,
    pub muted: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            app_theme: AppTheme::System,
            horizon_lock: true,
            volume: 1.0,
            muted: false,
        }
    }
}

/// Things the app remembers.
///
/// Paths rather than cosmic-player's URLs, for the same reason the command
/// line takes a path: we decode local files, we do not stream.
#[derive(Clone, CosmicConfigEntry, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ConfigState {
    pub recent_files: VecDeque<PathBuf>,
}

impl ConfigState {
    /// Most recent first, deduplicated, ten at most.
    pub fn remember(&mut self, path: &Path) {
        self.recent_files.retain(|recent| recent != path);
        self.recent_files.push_front(path.to_path_buf());
        self.recent_files.truncate(RECENT);
    }
}

/// Both entries, and the handlers that write them back.
pub struct Stored {
    pub config: Config,
    pub state: ConfigState,
    config_handler: Option<cosmic_config::Config>,
    state_handler: Option<cosmic_config::Config>,
}

impl Stored {
    pub fn load(app_id: &str) -> Self {
        let (config_handler, config) =
            read("config", cosmic_config::Config::new(app_id, CONFIG_VERSION));
        let (state_handler, state) = read(
            "saved state",
            cosmic_config::Config::new_state(app_id, CONFIG_VERSION),
        );
        Self {
            config,
            state,
            config_handler,
            state_handler,
        }
    }

    pub fn write_config(&self) {
        write("config", &self.config, self.config_handler.as_ref());
    }

    pub fn write_state(&self) {
        write("saved state", &self.state, self.state_handler.as_ref());
    }
}

/// One entry, or the default if it is unreadable. A half-readable entry keeps
/// the fields that did parse, which is what `get_entry` hands back with its
/// errors.
fn read<T: CosmicConfigEntry + Default>(
    what: &str,
    handler: Result<cosmic_config::Config, cosmic_config::Error>,
) -> (Option<cosmic_config::Config>, T) {
    match handler {
        Ok(handler) => {
            let entry = T::get_entry(&handler).unwrap_or_else(|(errors, entry)| {
                eprintln!("kyerag: {what} partly unreadable: {errors:?}");
                entry
            });
            (Some(handler), entry)
        }
        Err(e) => {
            eprintln!("kyerag: {what} will not be saved: {e}");
            (None, T::default())
        }
    }
}

fn write<T: CosmicConfigEntry>(what: &str, entry: &T, handler: Option<&cosmic_config::Config>) {
    let Some(handler) = handler else {
        return;
    };
    if let Err(e) = entry.write_entry(handler) {
        eprintln!("kyerag: {what} not saved: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(state: &ConfigState) -> Vec<&str> {
        state
            .recent_files
            .iter()
            .map(|path| path.to_str().unwrap())
            .collect()
    }

    #[test]
    fn the_newest_file_is_first_and_never_listed_twice() {
        let mut state = ConfigState::default();
        for path in ["/a.insv", "/b.insv", "/a.insv"] {
            state.remember(Path::new(path));
        }
        assert_eq!(paths(&state), ["/a.insv", "/b.insv"]);
    }

    #[test]
    fn only_ten_are_kept() {
        let mut state = ConfigState::default();
        for i in 0..25 {
            state.remember(Path::new(&format!("/{i}.insv")));
        }
        assert_eq!(state.recent_files.len(), RECENT);
        assert_eq!(paths(&state)[0], "/24.insv");
    }
}
