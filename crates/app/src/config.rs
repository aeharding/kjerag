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

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

use cosmic::cosmic_config::cosmic_config_derive::CosmicConfigEntry;
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::theme;
use kyerag_render::SeamFit;
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

/// One camera's seam calibration, as it is written to disk.
///
/// Its own type rather than `SeamFit` because that one belongs to the render
/// layer, which has no serde and no business having one: a number the app
/// stores is the app's own shape, and this is where the two are converted.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct SeamCalibration {
    pub roll_deg: f64,
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    pub cx_px: f64,
    pub cy_px: f64,
}

impl From<SeamFit> for SeamCalibration {
    fn from(fit: SeamFit) -> Self {
        Self {
            roll_deg: fit.roll_deg,
            yaw_deg: fit.yaw_deg,
            pitch_deg: fit.pitch_deg,
            cx_px: fit.cx_px,
            cy_px: fit.cy_px,
        }
    }
}

impl From<SeamCalibration> for SeamFit {
    fn from(stored: SeamCalibration) -> Self {
        Self {
            roll_deg: stored.roll_deg,
            yaw_deg: stored.yaw_deg,
            pitch_deg: stored.pitch_deg,
            cx_px: stored.cx_px,
            cy_px: stored.cy_px,
        }
    }
}

/// Things the app remembers.
///
/// Paths rather than cosmic-player's URLs, for the same reason the command
/// line takes a path: we decode local files, we do not stream.
#[derive(Clone, CosmicConfigEntry, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct ConfigState {
    pub recent_files: VecDeque<PathBuf>,
    /// What each camera's seam was measured to be off by (issue #48), under
    /// [`CalibrationSet::camera_key`](kyerag_meta::CalibrationSet::camera_key)
    /// in hex.
    ///
    /// **State rather than config**, which is the same call cosmic-player
    /// makes for its recent files (docs/UI.md, "Persistence"): this is a
    /// measurement the app made, not a preference the pilot expressed, it has
    /// no row in the Settings page, and resetting the settings must not throw
    /// away a calibration that took a capture and two seconds to make. It is
    /// not a cache either, which is what changed on this branch: deleting it
    /// does not cost a recompute, it costs the pilot the capture he pointed
    /// the app at.
    pub seam_calibration: BTreeMap<String, SeamCalibration>,
}

impl ConfigState {
    /// Most recent first, deduplicated, ten at most.
    pub fn remember(&mut self, path: &Path) {
        self.recent_files.retain(|recent| recent != path);
        self.recent_files.push_front(path.to_path_buf());
        self.recent_files.truncate(RECENT);
    }

    /// What this box knows about this camera's seam, if anything.
    pub fn seam(&self, camera: u64) -> Option<SeamFit> {
        self.seam_calibration
            .get(&camera_name(camera))
            .map(|stored| (*stored).into())
    }

    /// Remember one camera's seam, replacing whatever was there: a second
    /// calibration is the pilot saying the first one was not good enough.
    pub fn calibrate(&mut self, camera: u64, fit: SeamFit) {
        self.seam_calibration
            .insert(camera_name(camera), fit.into());
    }
}

/// The camera key as the config file spells it. Hex, because a `u64` key in a
/// RON map reads as a decimal wall of digits and this one is a fingerprint
/// that a bug report may have to be matched against by eye.
fn camera_name(camera: u64) -> String {
    format!("{camera:016x}")
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

    /// One entry per camera, and calibrating again replaces it: the pilot
    /// pointing the action at a better capture has to be able to overrule the
    /// answer he got from a worse one.
    #[test]
    fn a_camera_has_one_calibration_and_the_newest_wins() {
        let mut state = ConfigState::default();
        let first = SeamFit {
            roll_deg: 0.702,
            yaw_deg: -2.605,
            pitch_deg: 0.176,
            ..SeamFit::default()
        };
        let better = SeamFit {
            roll_deg: 0.810,
            yaw_deg: -2.352,
            pitch_deg: -0.678,
            cx_px: -4.18,
            cy_px: -13.91,
        };
        state.calibrate(0x1234_5678_9abc_def0, first);
        state.calibrate(0x1234_5678_9abc_def0, better);
        state.calibrate(0x0fed_cba9_8765_4321, first);

        assert_eq!(state.seam(0x1234_5678_9abc_def0), Some(better));
        assert_eq!(state.seam(0x0fed_cba9_8765_4321), Some(first));
        assert_eq!(state.seam(0), None);
        assert_eq!(state.seam_calibration.len(), 2);
        assert!(state.seam_calibration.contains_key("123456789abcdef0"));
    }
}
