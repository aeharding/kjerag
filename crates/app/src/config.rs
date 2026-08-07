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
use kjerag_render::seam::distance;
use kjerag_render::{Harvest, SeamFit};
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

/// How many fits a camera's pool wants before it stops asking for more.
///
/// Small enough to be reached in a few sittings, and enough of a majority to
/// choose between: five tolerates two contaminated fits, which is one more
/// than the record's seven captures contain (04-10, and the deck capture that
/// never gets pooled at all). Past this a file is drawn with the pooled answer
/// and costs no fit.
pub const POOL_ENOUGH: usize = 5;

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
    /// What the same ring read along the seam above the calibration the camera
    /// wrote, as the five terms `band::Along` is written in, in degrees
    /// ([`kjerag_render::seam::along_kept`]).
    ///
    /// **NOTHING READS THIS AND NOTHING DRAWS WITH IT** (issue #103, stage 9
    /// layer 2, docs/research/stage9.md 9). It was composed into the picture
    /// and withdrawn: it improves the **unbent** projection every seam
    /// instrument in this repository measures, and in the **delivered** picture
    /// the per-frame band has already taken the same leftover out, so applying
    /// it bought nothing at two reference views and cost about two view pixels
    /// at a third. The owner, blind, said "same, both bad".
    ///
    /// It accumulates anyway, because a pool fills over months and the one
    /// regime that finding does not cover is the first frames of a session,
    /// before the band has any evidence. **Anything that reads this has to
    /// clear a delivered-app-path comparison against `main` before it draws
    /// with it**, and it has `T - fit(T)` to answer for at the directions a
    /// session never reads (stage9.md 9.2). No number measured on the
    /// projection is that comparison.
    ///
    /// **Not a leftover**, which is what makes it worth storing at all: no pose
    /// has been taken off it, so it is the same quantity on every capture of
    /// this camera whatever the pool's answer happens to be that day.
    ///
    /// `None` on a ring that could not pin five terms, or on one the harvest
    /// guard refused, which costs the pool this sample's field and keeps its
    /// pose.
    pub along_deg: Option<[f64; 5]>,
}

impl SeamSample {
    /// The five knobs alone, which is what a pooled answer is made of.
    fn fit(&self) -> SeamFit {
        SeamFit {
            roll_deg: self.roll_deg,
            yaw_deg: self.yaw_deg,
            pitch_deg: self.pitch_deg,
            cx_px: self.cx_px,
            cy_px: self.cy_px,
        }
    }
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
            along_deg: harvest.along,
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
    /// The pooled answer: the one fit the rest of the pool agrees with most.
    ///
    /// A whole fit and not a median of each knob separately, which is what
    /// shipped and what was measured wrong
    /// (docs/research/seam-two-axis.md 4). The five knobs trade against each
    /// other inside one fit, a relative roll and a principal-point shift
    /// leaving overlapping signatures round the seam, so a knob-by-knob middle
    /// ships a combination no capture ever asked for: roll from one fit, yaw
    /// from a second, pitch and cy from a third. On the owner's camera that
    /// combination was beaten by a member of its own pool on all six flights
    /// it was re-read against.
    ///
    /// Choosing a member keeps the median's own argument, which is that the
    /// failure to survive is one bad fit rather than noise on every fit.
    /// 04-10 is that file in the record: its content is 2.6 m away and it asks
    /// for a yaw 0.9 degrees from what the same camera asks for elsewhere. The
    /// score is a sum of distances rather than of squares, so such a fit sits
    /// far from every other and cannot win. It can still tip the choice from
    /// one clean fit to its neighbour, and that is a step the width of the
    /// pool's own scatter rather than of the contamination: 0.04 degrees of
    /// yaw against the bad fit's 0.9
    /// (`one_contaminated_fit_does_not_move_the_pooled_answer`).
    ///
    /// **A pool that is split evenly answers with the middle of what it is
    /// split between**, which is a fit nobody took and is this rule's own
    /// point given up. There is nothing else to answer with: two fits are the
    /// same distance from each other, so a two-entry pool has no member the
    /// rest of it agrees with more, and choosing one would be choosing by
    /// which file was watched first. That is the old rule's answer for two and
    /// it is a live path rather than a corner, because `App::hold_seam` draws
    /// with the answer from the first capture on, before `POOL_ENOUGH` is
    /// anywhere near.
    ///
    /// The equality is exact and can be: a split of this kind is one distance
    /// added to zeros in a different order, and `f64` addition of a zero and
    /// of a number to itself are both exact.
    pub fn answer(&self) -> Option<SeamFit> {
        let fits: Vec<SeamFit> = self.samples.iter().map(SeamSample::fit).collect();
        let apart: Vec<f64> = fits.iter().map(|fit| apart_from_all(*fit, &fits)).collect();
        let least = apart.iter().copied().reduce(f64::min)?;
        let agreed: Vec<SeamFit> = fits
            .into_iter()
            .zip(apart)
            .filter(|(_, apart)| *apart == least)
            .map(|(fit, _)| fit)
            .collect();
        middle_of(&agreed)
    }

    /// Take one fit if it is worth keeping. `false` for one that is not, which
    /// is not an error: most captures have something near the seam.
    fn keep(&mut self, sample: SeamSample) -> bool {
        // A knob that came out NaN is a broken fit rather than a bad one, and
        // one of them in a pool makes every distance in it NaN: `answer` finds
        // no least sum, and the camera it belongs to is drawn uncalibrated
        // from then on, because the sample is on disk.
        let numbers = [
            sample.roll_deg,
            sample.yaw_deg,
            sample.pitch_deg,
            sample.cx_px,
            sample.cy_px,
            sample.residual_deg,
        ];
        if !numbers.iter().all(|number| number.is_finite()) {
            return false;
        }
        if sample.residual_deg > POOL_RESIDUAL_DEG {
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

/// How far one fit sits from a whole pool, summed, in probe steps.
///
/// The distance is the render layer's own ([`kjerag_render::seam::distance`]),
/// because the five knobs are not commensurable and the probe is the scale the
/// fit already compares them on. A fit's distance to itself is in the sum and
/// is zero, so counting it changes no ordering and leaving it out would only
/// be a line of arithmetic to get wrong.
fn apart_from_all(fit: SeamFit, fits: &[SeamFit]) -> f64 {
    fits.iter().map(|other| distance(fit, *other)).sum()
}

/// The middle of some fits, knob by knob, or `None` for none of them.
///
/// The knob-by-knob middle is what [`SeamPool::answer`] exists to stop
/// shipping, and this is the one place it is still the answer: fits that are
/// tied for the pool's agreement are fits nothing in the pool can choose
/// between, and their middle is at least not chosen by storage order.
fn middle_of(fits: &[SeamFit]) -> Option<SeamFit> {
    let count = fits.len();
    if count == 0 {
        return None;
    }
    let middle = |of: fn(&SeamFit) -> f64| fits.iter().map(of).sum::<f64>() / count as f64;
    Some(SeamFit {
        roll_deg: middle(|fit| fit.roll_deg),
        yaw_deg: middle(|fit| fit.yaw_deg),
        pitch_deg: middle(|fit| fit.pitch_deg),
        cx_px: middle(|fit| fit.cx_px),
        cy_px: middle(|fit| fit.cy_px),
    })
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
    /// `CalibrationSet::camera_key` (`crates/meta/src/calibration.rs`) in hex.
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
    ///
    /// **It survives the per-frame trim rather than being discarded for it**
    /// (issue #103, stage 9 layer 2, docs/research/stage9.md 9.3). Samples
    /// stored before it were fitted through rings reduced by a mean and are a
    /// worse estimate of the same quantity - `cy` -11.91 against -13.18
    /// refitted - and an earlier form of that change discarded the pool for it.
    /// What paid for the discard was an applied field the same PR withdrew, and
    /// without that the ledger is a certain cost against a benefit the band
    /// already covers wherever it has evidence. A pool is a mixture of better
    /// and worse estimates of one number by construction, so a trimmed fit
    /// joins it like any other and the answer migrates as they accumulate. A
    /// later stage that shows the trimmed pose is worth forcing can discard on
    /// its own evidence.
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
        let (state_handler, state) = saved_state(app_id);
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

/// What the app noticed, on its own: the saved state and the handler that
/// writes it back.
fn saved_state(app_id: &str) -> (Option<cosmic_config::Config>, ConfigState) {
    read(
        "saved state",
        cosmic_config::Config::new_state(app_id, CONFIG_VERSION),
    )
}

/// The saved state read-only, which is what a headless instrument drawing with
/// `seam=pool` reads and all of what it reads (`crates/spike/src/seam.rs`).
///
/// The config entry beside it is the pilot's own preferences and none of an
/// instrument's business, and this returns no handler, so nothing that calls
/// it can write the pool back.
pub fn state(app_id: &str) -> ConfigState {
    saved_state(app_id).1
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
                eprintln!("kjerag: {what} partly unreadable: {errors:?}");
                entry
            });
            (Some(handler), entry)
        }
        Err(e) => {
            eprintln!("kjerag: {what} will not be saved: {e}");
            (None, T::default())
        }
    }
}

fn write<T: CosmicConfigEntry>(what: &str, entry: &T, handler: Option<&cosmic_config::Config>) {
    let Some(handler) = handler else {
        return;
    };
    if let Err(e) = entry.write_entry(handler) {
        eprintln!("kjerag: {what} not saved: {e}");
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
            along: None,
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
        let two = state.seam(0x1234_5678_9abc_def0).unwrap().yaw_deg;
        assert!((two - -2.40).abs() < 1e-9, "{two} is not the middle of two");
        assert_eq!(state.seam(0x0fed_cba9_8765_4321).unwrap().yaw_deg, -1.10);
        assert_eq!(state.seam(0), None);
        assert_eq!(state.seam_pool.len(), 2);
        assert!(state.seam_pool.contains_key("123456789abcdef0"));
    }

    /// The field rides in on the same sample as the fit, and it is **stored
    /// and not applied** (docs/research/stage9.md 9.4). What is under test is
    /// that a capture that read one keeps it and a capture that did not is
    /// still pooled for its pose.
    #[test]
    fn a_sample_carries_the_field_its_ring_read_and_is_pooled_either_way() {
        let mut state = ConfigState::default();
        assert!(state.harvest(1, harvest(-2.4, 30, 0.6)));
        assert!(state.harvest(
            1,
            Harvest {
                along: Some([-0.77, -0.40, -0.10, -0.02, 0.02]),
                ..harvest(-2.4, 30, 0.6)
            },
        ));
        let pool = &state.seam_pool["0000000000000001"];
        assert_eq!(pool.samples.len(), 2);
        assert_eq!(pool.samples[0].along_deg, None, "a ring that read none");
        assert_eq!(
            pool.samples[1].along_deg.map(|terms| terms[0]),
            Some(-0.77),
            "a ring that read one",
        );
        assert!(state.seam(1).is_some(), "the pose is pooled either way");
    }

    /// The point of pooling: one file whose seam is full of near content asks
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

    /// A pool of two is split evenly whichever order its files were watched
    /// in, so the answer may not depend on that order. It is the middle of the
    /// two, which is what the knobwise median answered here as well.
    ///
    /// Half of a bad fit is in that answer and nothing in a pool of two can
    /// say which half: this pins the order, not the quality. The pool being
    /// asked at all before it is deep is `App::hold_seam`'s doing and is the
    /// point of it (zero-config playback).
    #[test]
    fn a_pool_of_two_answers_the_same_whichever_file_was_watched_first() {
        let contaminated = -1.69;
        let clean = -2.45;
        let answer = |first: f64, then: f64| {
            let mut state = ConfigState::default();
            state.harvest(1, harvest(first, 30, 0.6));
            state.harvest(1, harvest(then, 30, 0.6));
            state.seam(1).unwrap().yaw_deg
        };
        let bad_first = answer(contaminated, clean);
        assert!(
            (bad_first - answer(clean, contaminated)).abs() < 1e-12,
            "{bad_first} is the fit that was stored first, not the pool's answer"
        );
        assert!((bad_first - (contaminated + clean) / 2.0).abs() < 1e-12);
    }

    /// An evenly split pool is not only the pool of two: the owner's own has
    /// two of its captures stored twice (issue #156), and a pool of two such
    /// pairs is split as squarely as a pool of two.
    #[test]
    fn an_evenly_split_pool_answers_between_the_fits_it_is_split_between() {
        let mut state = ConfigState::default();
        for yaw in [-2.45, -2.45, -2.25, -2.25] {
            state.harvest(1, harvest(yaw, 30, 0.6));
        }
        let answer = state.seam(1).unwrap().yaw_deg;
        assert!(
            (answer - -2.35).abs() < 1e-9,
            "{answer} is one of the tied pairs rather than the middle of them"
        );
    }

    /// The score sums distances and not squares, which is what makes it
    /// survive a bad fit rather than average one in. Four fits a tenth of a
    /// degree apart and one two degrees out: summing squares moves the answer
    /// one fit towards the far one, because squaring is what makes a distant
    /// point worth pulling towards.
    #[test]
    fn the_pooled_answer_sums_distances_rather_than_their_squares() {
        let mut state = ConfigState::default();
        for yaw in [-2.40, -2.30, -2.20, -2.10, -0.40] {
            state.harvest(1, harvest(yaw, 30, 0.6));
        }
        let answer = state.seam(1).unwrap().yaw_deg;
        assert!(
            (answer - -2.20).abs() < 1e-9,
            "{answer} is what a sum of squares answers, not a sum of distances"
        );
    }

    /// A fit that came out of the arithmetic as NaN is refused at the door.
    /// One in a pool makes every distance in that pool NaN, and a pool with no
    /// least sum has no answer at all: the camera would be drawn uncalibrated
    /// for as long as the sample sat in the file.
    #[test]
    fn a_fit_with_a_knob_that_is_not_a_number_is_not_kept() {
        let mut state = ConfigState::default();
        let mut broken = harvest(-2.4, 30, 0.5);
        broken.fit.roll_deg = f64::NAN;
        assert!(!state.harvest(1, broken));
        let mut runaway = harvest(-2.4, 30, 0.5);
        runaway.fit.cx_px = f64::INFINITY;
        assert!(!state.harvest(1, runaway));

        assert!(state.harvest(1, harvest(-2.4, 30, 0.5)));
        assert_eq!(state.seam_pooled(1), 1);
        assert_eq!(state.seam(1).unwrap().yaw_deg, -2.4);
    }

    /// What the shipped answer used to be: the middle of each knob taken on
    /// its own. Here so that the fixture below shows the combination that rule
    /// produces rather than asserting one written out by hand.
    fn knobwise_median(pool: &SeamPool) -> SeamFit {
        let median = |of: fn(&SeamSample) -> f64| {
            let mut values: Vec<f64> = pool.samples.iter().map(of).collect();
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };
        SeamFit {
            roll_deg: median(|s| s.roll_deg),
            yaw_deg: median(|s| s.yaw_deg),
            pitch_deg: median(|s| s.pitch_deg),
            cx_px: median(|s| s.cx_px),
            cy_px: median(|s| s.cy_px),
        }
    }

    /// The five knobs are correlated inside one fit, so a middle taken knob by
    /// knob is a fit nobody measured: these are the three distinct fits in the
    /// owner's own camera's pool, and the median of them takes roll and cx
    /// from the first, yaw from the second, and pitch and cy from the third.
    ///
    /// His pool holds five samples of these three, because two captures are
    /// each stored twice (issue #156), so the fixture is grown to that shape
    /// at the end: the duplicates weight the sums and the answer is the same
    /// fit either way.
    ///
    /// **The numbers are his `seam_pool` file to every digit it stores**, and
    /// `crates/spike/src/seam.rs` holds the same five for the same reason at
    /// the other end of `seam=pool`. Two copies because the crates cannot
    /// share a `#[cfg(test)]` fixture and the app is not going to ship one;
    /// they are kept identical, and the string asserted below is what pins
    /// them together, because it is the string the registry quoted.
    ///
    /// Re-read off the pixels of six of his flights, at the three places in
    /// each file the app's own fit reads, that combination leaves 0.28 to 0.49
    /// deg along the seam where the third fit leaves 0.20 to 0.41, and it is
    /// beaten by a member of its own pool on every flight (the PR's table).
    #[test]
    fn the_pooled_answer_is_a_fit_some_capture_actually_took() {
        let mut state = ConfigState::default();
        let camera = 0xd8a3_9338_9b7b_8639;
        let fits = [
            SeamFit {
                roll_deg: 0.5770177572311984,
                yaw_deg: -1.693547826643539,
                pitch_deg: -0.796449725529272,
                cx_px: -9.531358691231077,
                cy_px: -5.414553495776632,
            },
            SeamFit {
                roll_deg: 0.4592518809185011,
                yaw_deg: -2.0772194092771397,
                pitch_deg: -2.219459668631724,
                cx_px: -14.786100683560385,
                cy_px: -20.659845193073906,
            },
            SeamFit {
                roll_deg: 0.7954311295817457,
                yaw_deg: -2.309572216062777,
                pitch_deg: -0.9358779752048013,
                cx_px: -3.2814366126974686,
                cy_px: -11.91227998928906,
            },
        ];
        let reading = [
            (27, 0.7979799684676536),
            (12, 0.760502617023373),
            (41, 0.49833332566304156),
        ];
        for (fit, (patches, residual_deg)) in fits.into_iter().zip(reading) {
            assert!(state.harvest(
                camera,
                Harvest {
                    fit,
                    patches,
                    residual_deg,
                    along: None,
                },
            ));
        }
        let pool = &state.seam_pool[&camera_name(camera)];

        let median = knobwise_median(pool);
        assert!(
            !fits.contains(&median),
            "the fixture is not correlated: {median:?} is a fit somebody took"
        );
        let answer = pool.answer().unwrap();
        assert!(fits.contains(&answer), "{answer:?} is nobody's fit");
        assert_eq!(answer, fits[2], "the pool agrees with the third fit most");

        for (index, (patches, residual_deg)) in [(0, reading[0]), (2, reading[2])] {
            assert!(state.harvest(
                camera,
                Harvest {
                    fit: fits[index],
                    patches,
                    residual_deg,
                    along: None,
                },
            ));
        }
        assert_eq!(state.seam_pooled(camera), 5);
        assert_eq!(state.seam(camera), Some(fits[2]), "the owner's own pool");

        // The string two acceptance lines and four copies of them quoted
        // between 2026-08-05 and 2026-08-07, written out of this pool by the
        // rule that stopped shipping. It is here so that the claim the
        // registry now makes about it is checked and not asserted.
        let five = knobwise_median(&state.seam_pool[&camera_name(camera)]);
        assert_eq!(
            knobs(five),
            "roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91",
        );
        assert_eq!(
            knobs(state.seam(camera).unwrap()),
            "roll:0.795,yaw:-2.310,pitch:-0.936,cx:-3.28,cy:-11.91",
        );
    }

    /// A fit as a `seam=` argument spells one, which is the form the registry
    /// and the instruments both carry it in.
    fn knobs(fit: SeamFit) -> String {
        format!(
            "roll:{:.3},yaw:{:.3},pitch:{:.3},cx:{:.2},cy:{:.2}",
            fit.roll_deg, fit.yaw_deg, fit.pitch_deg, fit.cx_px, fit.cy_px,
        )
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
