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
use kyerag_render::{Harvest, SeamFit};
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

/// How many fits one camera's pool keeps. Past this the worst is dropped,
/// worst being the fewest azimuths, because the azimuth count is what caught
/// both of 6.8's bad captures where the residual did not.
const POOLED: usize = 16;

/// The widest residual a fit may leave and still be pooled, in degrees.
///
/// Read off the applied-and-re-read table (6.8, and
/// `scratch/seam2-investigation/11-applied-and-reread.txt`), which is the only
/// place in the record a correction was measured on the pixels **after** it was
/// applied rather than predicted: the five flights and the static capture come
/// out between 0.15 and 0.87 degrees, and the deck capture, whose seam is
/// 5 to 30 cm of decking and which the fit cannot help, stays at 1.65. One
/// degree is the gap between those two populations.
const POOL_RESIDUAL_DEG: f64 = 1.0;

/// One fit the app made by watching, as it is written to disk.
///
/// Its own type rather than `Harvest` because that one belongs to the render
/// layer, which has no serde and no business having one: a number the app
/// stores is the app's own shape, and this is where the two are converted.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct SeamSample {
    pub roll_deg: f64,
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    pub cx_px: f64,
    pub cy_px: f64,
    /// How many azimuths round the seam correlated, and what the fit left.
    /// Kept beside the answer because the pool has to be able to tell a fit
    /// off fifty far-field patches from one off seven near-field ones.
    pub patches: usize,
    pub residual_deg: f64,
}

impl From<Harvest> for SeamSample {
    fn from(harvest: Harvest) -> Self {
        Self {
            roll_deg: harvest.fit.roll_deg,
            yaw_deg: harvest.fit.yaw_deg,
            pitch_deg: harvest.fit.pitch_deg,
            cx_px: harvest.fit.cx_px,
            cy_px: harvest.fit.cy_px,
            patches: harvest.patches,
            residual_deg: harvest.residual_deg,
        }
    }
}

/// What this box has learned about one camera's seam, by watching.
///
/// A pool rather than one answer, and this is the whole of what changed
/// (owner ruling, 2026-07-31, zero-config playback). The single-entry store
/// this replaces was filled by a menu action, and the action stored a fit off
/// **whichever file happened to be open**: on this box it stored the May 1
/// flight's fit, then the April 10 flight's, and never the static capture it
/// was meant for. A fit taken through a seam full of near content absorbs that
/// flight's parallax and then applies it to the whole sphere (6.8), so both
/// answers were wrong in a way nothing on screen could show.
///
/// The pool's premise is that the contamination is per file and the
/// calibration is not: what a flight's own parallax adds points wherever that
/// flight's near content happened to be, while the factory extrinsic error is
/// the same every time, so across files the first should scatter and the
/// second should not. **The premise is stated, not yet established**; the
/// experiment that would settle it is in the PR.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct SeamPool {
    pub samples: Vec<SeamSample>,
}

impl SeamPool {
    /// The pooled answer: the median of each knob over the samples.
    ///
    /// A median rather than a mean, weighted or not, because the failure this
    /// has to survive is one bad fit rather than noise on every fit. 04-10 is
    /// that file in the record: its content is 2.6 m away and it asks for a
    /// yaw 0.9 degrees from what the same camera asks for elsewhere. A mean
    /// carries a sixteenth of that into the answer; a median carries none of
    /// it until such files are half the pool.
    pub fn answer(&self) -> Option<SeamFit> {
        if self.samples.is_empty() {
            return None;
        }
        let median = |of: fn(&SeamSample) -> f64| {
            let mut values: Vec<f64> = self.samples.iter().map(of).collect();
            values.sort_by(f64::total_cmp);
            let middle = values.len() / 2;
            match values.len() % 2 {
                0 => (values[middle - 1] + values[middle]) / 2.0,
                _ => values[middle],
            }
        };
        Some(SeamFit {
            roll_deg: median(|s| s.roll_deg),
            yaw_deg: median(|s| s.yaw_deg),
            pitch_deg: median(|s| s.pitch_deg),
            cx_px: median(|s| s.cx_px),
            cy_px: median(|s| s.cy_px),
        })
    }

    /// Take one fit if it is worth keeping. `false` for one that is not, which
    /// is not an error: most captures have something near the seam.
    fn keep(&mut self, sample: SeamSample) -> bool {
        if !sample.residual_deg.is_finite() || sample.residual_deg > POOL_RESIDUAL_DEG {
            return false;
        }
        self.samples.push(sample);
        if self.samples.len() > POOLED {
            let worst = self
                .samples
                .iter()
                .enumerate()
                .min_by_key(|(_, sample)| sample.patches)
                .map(|(index, _)| index);
            if let Some(worst) = worst {
                self.samples.remove(worst);
            }
        }
        true
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
    /// What each camera's seam has been measured to be off by, under
    /// [`CalibrationSet::camera_key`](kyerag_meta::CalibrationSet::camera_key)
    /// in hex.
    ///
    /// **State rather than config**, which is the same call cosmic-player
    /// makes for its recent files (docs/UI.md, "Persistence"): this is a
    /// measurement the app made, not a preference the pilot expressed, and it
    /// has no row in the Settings page.
    ///
    /// It is now a **cache**, which is what changed: nothing here costs the
    /// pilot an action to remake, so deleting it costs a few seconds of
    /// watching and nothing else. That is why the single-entry
    /// `seam_calibration` this replaces is discarded rather than migrated. The
    /// old entries were made by a menu action off whichever file was open, so
    /// migrating them would carry exactly the contamination the pool exists to
    /// average out; the old key is left on disk unread and the pool refills
    /// itself from the next few files played.
    pub seam_pool: BTreeMap<String, SeamPool>,
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
        self.seam_pool.get(&camera_name(camera))?.answer()
    }

    /// How many fits that answer rests on, for the report line.
    pub fn seam_pooled(&self, camera: u64) -> usize {
        self.seam_pool
            .get(&camera_name(camera))
            .map_or(0, |pool| pool.samples.len())
    }

    /// Fold one watched fit into this camera's pool. `false` where the fit was
    /// not good enough to keep, which is ordinary and is not shown anywhere.
    pub fn harvest(&mut self, camera: u64, harvest: Harvest) -> bool {
        self.seam_pool
            .entry(camera_name(camera))
            .or_default()
            .keep(harvest.into())
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

    fn harvest(yaw_deg: f64, patches: usize, residual_deg: f64) -> Harvest {
        Harvest {
            fit: SeamFit {
                roll_deg: 0.8,
                yaw_deg,
                pitch_deg: -0.7,
                cx_px: -2.5,
                cy_px: -13.8,
            },
            patches,
            residual_deg,
        }
    }

    /// The pool is per camera and it accumulates rather than replacing: no
    /// action fills it, so there is no pilot to say which of two answers he
    /// meant.
    #[test]
    fn each_camera_pools_its_own_fits() {
        let mut state = ConfigState::default();
        for yaw in [-2.35, -2.45] {
            assert!(state.harvest(0x1234_5678_9abc_def0, harvest(yaw, 30, 0.6)));
        }
        assert!(state.harvest(0x0fed_cba9_8765_4321, harvest(-1.10, 30, 0.6)));

        assert_eq!(state.seam_pooled(0x1234_5678_9abc_def0), 2);
        assert_eq!(state.seam(0x0fed_cba9_8765_4321).unwrap().yaw_deg, -1.10);
        assert_eq!(state.seam(0), None);
        assert_eq!(state.seam_pool.len(), 2);
        assert!(state.seam_pool.contains_key("123456789abcdef0"));
    }

    /// The point of a median: one file whose seam is full of near content asks
    /// for an answer of its own, and the pool must not follow it. 04-10 is
    /// that file in the record, 0.9 degrees of yaw away from what the same
    /// camera asks for on every other capture (6.8).
    #[test]
    fn one_contaminated_fit_does_not_move_the_pooled_answer() {
        let mut state = ConfigState::default();
        let camera = 0xd8a3_9338_9b7b_8639;
        for yaw in [-2.45, -2.35, -2.44, -2.40] {
            state.harvest(camera, harvest(yaw, 30, 0.6));
        }
        let clean = state.seam(camera).unwrap().yaw_deg;
        state.harvest(camera, harvest(-1.69, 23, 0.9));
        let polluted = state.seam(camera).unwrap().yaw_deg;
        assert!(
            (polluted - clean).abs() < 0.06,
            "one bad fit moved the answer from {clean:.3} to {polluted:.3}"
        );
    }

    /// A capture the fit cannot help is not pooled at all. The deck capture is
    /// the one in the record: 5 to 30 cm of decking across the seam, which no
    /// rotation reaches, and it comes out of the fit still 1.65 degrees wrong.
    #[test]
    fn a_fit_that_did_not_flatten_the_seam_is_not_kept() {
        let mut state = ConfigState::default();
        assert!(!state.harvest(1, harvest(-2.4, 5, 1.65)));
        assert!(!state.harvest(1, harvest(-2.4, 5, f64::NAN)));
        assert_eq!(state.seam(1), None);
        assert_eq!(state.seam_pooled(1), 0);
    }

    /// The pool is bounded, and what it drops is the fit with the fewest
    /// azimuths behind it rather than the oldest: a capture with content round
    /// the whole circle is better evidence than a newer one without.
    #[test]
    fn the_pool_is_bounded_and_drops_its_thinnest_fit() {
        let mut state = ConfigState::default();
        for patches in 0..POOLED {
            state.harvest(1, harvest(-2.4, 20 + patches, 0.5));
        }
        state.harvest(1, harvest(-2.4, 99, 0.5));
        let pool = &state.seam_pool["0000000000000001"];
        assert_eq!(pool.samples.len(), POOLED);
        assert_eq!(pool.samples.iter().map(|s| s.patches).min(), Some(21));
        assert!(pool.samples.iter().any(|s| s.patches == 99));
    }
}
