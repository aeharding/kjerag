//! The blind A/B session: what a session file says, and what the app is
//! allowed to swap while a clip plays.
//!
//! ```sh
//! kjerag --ab-session=/home/aeharding/kjerag-ab/sessions/handover.ab
//! ```
//!
//! **What this replaces.** Until now every blind A/B was a shell script that
//! launched the player once per arm and waited for the window to be closed
//! (`~/kjerag-ab/temporal-ab.sh`, `comb-ab.sh`, `ghost-ab.sh`). Two arms of
//! one view were two decodes, two seam warm-ups and two walks to the same
//! second of film, minutes apart, and the answer had to be carried across the
//! gap in somebody's memory. A session file plays the segment once and hands
//! the arms over inside it: same decode, same second, same picture, and the
//! flip is a keypress.
//!
//! **The app stays dumb, and that is where the blinding lives.** Nothing here
//! draws an arm's name, prints one, or knows which arm is "the new one". A
//! trial names its arms in the order they are to be shown and the window
//! calls them 1, 2, 3; which is which is a property of the file, and the file
//! is the coordinator's. That is the same contract `comb-ab-key.txt` has
//! today, with one fewer thing to keep in step.
//!
//! **Fail closed.** An arm may only ask for a knob the running pass can take
//! mid-flight ([`Swap::Live`]). A session that asks for a width baked into
//! the shader, or for one read at file open, is refused by name before a
//! window opens, because an arm that half applies is an A/B that measures
//! nothing and says so to nobody. [`KNOBS`] is the table and it is the
//! documentation.
//!
//! # The file
//!
//! One directive a line, first word names it, `#` is a comment. Nothing is
//! indentation sensitive; the indentation below is for the reader.
//!
//! ```text
//! session  handover-2026-08-08
//! results  /home/aeharding/kjerag-ab/results/handover.tsv
//!
//! arm ship  handover=8.0
//! arm wide  handover=12.0
//!
//! trial down1
//!   view /home/aeharding/Videos/Insta/VID_0001.insv time=65.666 yaw=179.00 pitch=-36.97 fov=20.00 lock=1
//!   loop 64.5 70.5
//!   arms wide ship
//! ```
//!
//! The `view` line is the line the `i` key copies, with the whole path on it,
//! so staging a trial is selecting a view out of a terminal (`Framing`).
//!
//! `pool`, `key`, `logs`, `seed` and `scheme` are the runner's own directives
//! (`~/kjerag-ab/ab.sh`). They are read here only far enough to know they
//! were written down, because a session file is one record and a second file
//! beside it would be a second thing to keep in step.

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kjerag_render::{Framing, Sampling};

/// Whether the running pass can be made to take a knob mid-playback.
///
/// The classification is a property of where the value is read, and the three
/// answers are the three places a number can be in this renderer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Swap {
    /// Rebuilt into the uniform block every frame, so a store now is a
    /// different picture on the next redraw. The only class an arm may use.
    Live,
    /// Read once while a file is being opened, so changing it needs the file
    /// opened again. A session could be written to reopen per arm, and then
    /// it would not be a hot swap and the whole point of this mode would be
    /// gone; refused rather than pretended.
    AtOpen(&'static str),
    /// Substituted into the WGSL source, which is compiled once before any
    /// file is open, or sizing a workgroup array. Nothing short of a second
    /// pipeline can vary it, and there is no machinery to build one.
    Baked(&'static str),
}

/// Every knob a session file may name, and whether an arm may move it.
///
/// **This table is the answer to "what can an A/B swap".** It is written out
/// rather than derived because the refusal has to name the reason: an agent
/// staging a session gets told why its arm cannot be an arm, in the same
/// breath as being told no.
///
/// The three live entries are the whole of what one binary can vary today.
/// Everything else in `crates/render` that looks like a research knob reaches
/// the GPU as a WGSL `const` (`projection::wgsl`, `band::wgsl`,
/// `band::lookup_wgsl`, `sampling::wgsl`) or is consumed by the seam fit at
/// open (`seam.rs`), and the entries below are the ones a session is most
/// likely to reach for.
const KNOBS: &[(&str, Swap)] = &[
    // Live: the whole uniform block is rebuilt and written every frame
    // (`ScenePipeline::prepare`), so these three are a store and a redraw.
    ("handover", Swap::Live),
    ("sampling", Swap::Live),
    // Baked into the shader source.
    (
        "blend_power",
        Swap::Baked(
            "the blend exponent is a WGSL const, and it also sets the band's SPEND and WIDEST, \
             which are two more. The shader is compiled once before any file is open",
        ),
    ),
    (
        "azimuths",
        Swap::Baked(
            "the number of directions round the seam circle is a WGSL const, the length of the \
             along-seam table in the uniform block, and the size of a workgroup array",
        ),
    ),
    (
        "spend",
        Swap::Baked("the fold budget is a WGSL const, derived from the blend exponent"),
    ),
    (
        "contrast",
        Swap::Baked("the band search's contrast weighting is a WGSL const"),
    ),
    (
        "tau_near",
        Swap::Baked("the near-field time constant is a WGSL const"),
    ),
    (
        "tau_far",
        Swap::Baked("the far-field time constant is a WGSL const"),
    ),
    // Read while a file is being opened.
    (
        "seam_rounds",
        Swap::AtOpen(
            "the seam fit runs once per file open, on the shell's own thread or beside it",
        ),
    ),
    (
        "seam_gate",
        Swap::AtOpen("the fit's outlier gate is read while the fit runs, which is at file open"),
    ),
    (
        "smooth_deg",
        Swap::AtOpen("the along-seam table's kernel width is read where the table is built"),
    ),
];

/// One named set of research-config values, which is what an arm is.
#[derive(Clone, Debug, PartialEq)]
pub struct Arm {
    /// Never printed and never drawn. The window says 1, 2, 3.
    name: String,
    /// In file order, and every arm carries the same names ([`Session::read`]).
    knobs: Vec<Knob>,
}

/// A knob set to a value, already parsed into what the renderer takes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Knob {
    /// Degrees of world angle the two lenses hand the picture over across.
    Handover(f32),
    /// Which planes the pass upgrades where the view magnifies the source.
    Sampling(Sampling),
}

/// One segment of one clip, and the arms to show at it, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Trial {
    /// What the results file calls it. The owner sees it too: it is a place,
    /// not an arm, so it gives nothing away.
    pub id: String,
    pub clip: PathBuf,
    pub at: Framing,
    /// Where the loop turns round. `at.time` is inside it ([`Session::read`]).
    pub from: Duration,
    pub to: Duration,
    /// Indices into [`Session::arms`], in the order they are shown. Position
    /// 1 in the window is `order[0]`.
    order: Vec<usize>,
}

/// A whole staged session: what to play, what to swap, where to write.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id: String,
    pub results: PathBuf,
    arms: Vec<Arm>,
    pub trials: Vec<Trial>,
}

/// The shortest a loop may be. Below this the seek is most of what is on
/// screen: an exact seek shows the asked-for frame about 240 ms after the
/// request (`Player::seek`), and the seam band starts its whole ring again on
/// the landing frame, so a one second loop would be mostly warm-up.
const LOOP_LEAST: Duration = Duration::from_secs(2);

impl Session {
    /// A session file, read and checked, or the first thing wrong with it.
    ///
    /// Everything is checked here rather than at the trial that trips over
    /// it: a session is a thing the owner is going to sit down in front of,
    /// and finding out at trial four that trial five names a knob nothing can
    /// swap costs the whole sitting.
    pub fn read(text: &str) -> Result<Self, String> {
        let mut id = None;
        let mut results = None;
        let mut arms: Vec<Arm> = Vec::new();
        let mut trials: Vec<Partial> = Vec::new();

        for (number, line) in text.lines().enumerate() {
            let at = |what: String| format!("line {}: {what}", number + 1);
            let line = line.split('#').next().unwrap_or(line).trim();
            let Some((word, rest)) = split(line) else {
                continue;
            };
            match word {
                "session" => id = Some(one(rest).map_err(at)?.to_owned()),
                "results" => results = Some(PathBuf::from(one(rest).map_err(at)?)),
                // The runner's, recorded here so the session is one record.
                // Checked for having a value and nothing else.
                "pool" | "key" | "logs" | "seed" | "scheme" => {
                    if rest.is_empty() {
                        return Err(at(format!("{word} says nothing")));
                    }
                }
                "arm" => arms.push(Arm::read(rest).map_err(at)?),
                "trial" => trials.push(Partial::new(one(rest).map_err(at)?)),
                "view" | "loop" | "arms" => trials
                    .last_mut()
                    .ok_or_else(|| at(format!("{word} before any trial")))?
                    .read(word, rest)
                    .map_err(at)?,
                _ => return Err(at(format!("no session has a {word} line in it"))),
            }
        }

        let id = id.ok_or("a session with no `session` line has nothing to call its results")?;
        let results = results.ok_or("a session with no `results` line has nowhere to answer")?;
        if arms.len() < 2 {
            return Err(format!("{} arm is not a comparison", arms.len()));
        }
        // Every arm sets every knob, or an arm inherits whatever the arm
        // before it left in the renderer and the comparison is between one
        // arm and a mixture.
        let named: Vec<&str> = arms[0].knobs.iter().map(Knob::name).collect();
        for arm in &arms {
            let mine: Vec<&str> = arm.knobs.iter().map(Knob::name).collect();
            if mine != named {
                return Err(format!(
                    "arm {} sets {} and arm {} sets {}. every arm sets every knob, or an arm \
                     inherits what the one before it left behind",
                    arms[0].name,
                    named.join(" "),
                    arm.name,
                    mine.join(" "),
                ));
            }
        }
        if named.is_empty() {
            return Err("arms that set no knob are the same arm twice".to_owned());
        }

        let names: Vec<&str> = arms.iter().map(|arm| arm.name.as_str()).collect();
        let trials = trials
            .into_iter()
            .map(|partial| partial.finish(&names))
            .collect::<Result<Vec<_>, _>>()?;
        if trials.is_empty() {
            return Err("a session with no trials has nothing to ask".to_owned());
        }
        Ok(Self {
            id,
            results,
            arms,
            trials,
        })
    }

    /// How many arms this trial shows, which is what the window's keys count
    /// up to.
    pub fn arms_at(&self, trial: usize) -> usize {
        self.trials[trial].order.len()
    }

    /// The knobs the arm at this position of this trial asks for. Position is
    /// what the owner presses and what the results file records; the name is
    /// never handed out.
    fn knobs(&self, trial: usize, position: usize) -> &[Knob] {
        &self.arms[self.trials[trial].order[position]].knobs
    }

    /// The key: which arm each position of each trial is, for the
    /// coordinator, after the answers are in. Nothing calls this while the
    /// window is open.
    pub fn key(&self) -> String {
        let mut said = String::new();
        for trial in &self.trials {
            for (position, arm) in trial.order.iter().enumerate() {
                let _ = writeln!(
                    said,
                    "{} arm {} = {}",
                    trial.id,
                    position + 1,
                    self.arms[*arm].name
                );
            }
        }
        said
    }
}

/// Set the running pass to one arm of one trial.
///
/// Returns what it cost, which is the whole cost of a swap: every knob here
/// is a store that the next redraw reads ([`Swap::Live`]), so what is
/// measured is the store and not a rebuild. Errors cannot happen at this
/// point, because [`Session::read`] refused any width the renderer would
/// refuse; a refusal here would mean the two bounds had drifted apart, and it
/// is louder than an arm that quietly did not apply.
pub fn wear(session: &Session, trial: usize, position: usize, scene: &kjerag_render::Scene) {
    for knob in session.knobs(trial, position) {
        match knob {
            Knob::Handover(width) => {
                if let Err(said) = kjerag_render::ask_handover(*width) {
                    eprintln!("kjerag: {said}");
                }
            }
            Knob::Sampling(sampling) => scene.set_sampling(*sampling),
        }
    }
}

/// One answered trial, appended to the results file.
///
/// Tab separated and one line per trial, so a coordinator unblinds by joining
/// it against [`Session::key`] on the trial and the position, with no prose
/// to read. The vote is a **position** and never a name: the app does not
/// know which arm won and is not the place that finds out.
pub struct Answer {
    pub trial: String,
    /// The position voted for, from 1, or `None` for cannot tell.
    pub vote: Option<usize>,
    pub loops: u32,
    pub swaps: u32,
    pub watched: Duration,
}

/// The columns, written once when the file is made.
const COLUMNS: &str = "# when\tsession\ttrial\tvote\tloops\tswaps\tseconds";

/// Append one answer, making the file and its directory if they are new.
///
/// Append and not rewrite, because a results file is evidence: a session run
/// twice leaves both runs, and a crash leaves everything answered before it.
pub fn append(session: &Session, answer: &Answer) -> Result<(), String> {
    let path = &session.results;
    let fresh = !path.exists();
    if let Some(folder) = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        std::fs::create_dir_all(folder)
            .map_err(|e| format!("{} not made: {e}", folder.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("{} not opened to append: {e}", path.display()))?;
    let mut said = String::new();
    if fresh {
        said.push_str(COLUMNS);
        said.push('\n');
    }
    let _ = writeln!(
        said,
        "{}\t{}\t{}\t{}\t{}\t{}\t{:.1}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        session.id,
        answer.trial,
        match answer.vote {
            Some(position) => position.to_string(),
            None => "-".to_owned(),
        },
        answer.loops,
        answer.swaps,
        answer.watched.as_secs_f64(),
    );
    file.write_all(said.as_bytes())
        .map_err(|e| format!("{} not written: {e}", path.display()))
}

/// A session file read off disk, or why it is not one.
pub fn open(path: &Path) -> Result<Session, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("{} not read: {e}", path.display()))?;
    Session::read(&text).map_err(|said| format!("{}: {said}", path.display()))
}

impl Arm {
    fn read(rest: &str) -> Result<Self, String> {
        let mut words = rest.split_whitespace();
        let name = words
            .next()
            .ok_or_else(|| "an arm with no name cannot be ordered".to_owned())?;
        let knobs = words.map(Knob::read).collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            name: name.to_owned(),
            knobs,
        })
    }
}

impl Knob {
    /// One `knob=value`, refused by name and with the reason if the running
    /// pass cannot take it mid-flight.
    fn read(term: &str) -> Result<Self, String> {
        let (name, value) = term
            .split_once('=')
            .ok_or_else(|| format!("{term} is not a knob=value"))?;
        match KNOBS.iter().find(|(known, _)| *known == name) {
            None => Err(format!(
                "no knob called {name}. the ones an arm may set are {}",
                live().join(", ")
            )),
            Some((_, Swap::Baked(why))) => Err(format!(
                "{name} cannot be swapped while a clip plays: {why}. an A/B over it is two \
                 builds, the way it was done before this mode existed"
            )),
            Some((_, Swap::AtOpen(why))) => Err(format!(
                "{name} cannot be swapped while a clip plays: {why}. an arm that reopened the \
                 file would not be a hot swap"
            )),
            Some((_, Swap::Live)) => match name {
                "handover" => value
                    .parse::<f32>()
                    .map_err(|e| format!("handover={value}: {e}"))
                    // The renderer's own bound, tested before a window opens
                    // rather than shrugged off at the swap. Tested and not
                    // asked for: reading a file may not move the picture.
                    .and_then(kjerag_render::takes_handover)
                    .map(Self::Handover),
                "sampling" => match value {
                    "bilinear" => Ok(Self::Sampling(Sampling::Bilinear)),
                    "luma" => Ok(Self::Sampling(Sampling::Luma)),
                    "sharp" => Ok(Self::Sampling(Sampling::Sharp)),
                    _ => Err(format!("sampling={value} is not bilinear, luma or sharp")),
                },
                _ => unreachable!("{name} is live in the table and unread here"),
            },
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Handover(_) => "handover",
            Self::Sampling(_) => "sampling",
        }
    }
}

/// The knobs an arm may set, for the message that says one was not.
fn live() -> Vec<&'static str> {
    KNOBS
        .iter()
        .filter(|(_, swap)| *swap == Swap::Live)
        .map(|(name, _)| *name)
        .collect()
}

/// A trial being read, before it is known that its arms exist.
struct Partial {
    id: String,
    view: Option<(PathBuf, Framing)>,
    span: Option<(Duration, Duration)>,
    arms: Vec<String>,
}

impl Partial {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            view: None,
            span: None,
            arms: Vec::new(),
        }
    }

    fn read(&mut self, word: &str, rest: &str) -> Result<(), String> {
        match word {
            "view" => {
                self.view = Some(Framing::read_line(rest).ok_or_else(|| {
                    format!(
                        "{rest} is not a view line. it is what the `i` key copies, with the whole \
                         path on it"
                    )
                })?);
            }
            "loop" => {
                let mut ends = rest.split_whitespace();
                let mut second = |which| {
                    ends.next()
                        .ok_or_else(|| format!("a loop needs a {which} second"))?
                        .parse::<f64>()
                        .map_err(|e| format!("{which}: {e}"))
                };
                let (from, to) = (second("from")?, second("to")?);
                if !(from >= 0.0 && to > from) {
                    return Err(format!("{from} to {to} is not a stretch of film"));
                }
                self.span = Some((
                    Duration::try_from_secs_f64(from).unwrap_or_default(),
                    Duration::try_from_secs_f64(to).unwrap_or_default(),
                ));
            }
            _ => self.arms = rest.split_whitespace().map(str::to_owned).collect(),
        }
        Ok(())
    }

    /// The trial, with its arms resolved to the session's own, or the first
    /// thing missing from it.
    fn finish(self, names: &[&str]) -> Result<Trial, String> {
        let what = |said: String| format!("trial {}: {said}", self.id);
        let (clip, at) = self.view.ok_or_else(|| what("no view line".to_owned()))?;
        let (from, to) = self.span.ok_or_else(|| what("no loop line".to_owned()))?;
        if to - from < LOOP_LEAST {
            return Err(what(format!(
                "a {:.1} second loop is mostly the seek and the seam's warm-up. {} is the least",
                (to - from).as_secs_f64(),
                LOOP_LEAST.as_secs_f64()
            )));
        }
        if at.at < from || at.at > to {
            return Err(what(format!(
                "the view is at {:.3} s and the loop runs {:.3} to {:.3}",
                at.at.as_secs_f64(),
                from.as_secs_f64(),
                to.as_secs_f64()
            )));
        }
        if self.arms.len() < 2 {
            return Err(what(
                "an arms line with fewer than two arms on it".to_owned(),
            ));
        }
        let order = self
            .arms
            .iter()
            .map(|arm| {
                names
                    .iter()
                    .position(|known| known == arm)
                    .ok_or_else(|| what(format!("no arm called {arm}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Trial {
            id: self.id,
            clip,
            at,
            from,
            to,
            order,
        })
    }
}

/// A line's first word and the rest of it, or nothing for a blank line.
fn split(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    match line.split_once(char::is_whitespace) {
        Some((word, rest)) => Some((word, rest.trim())),
        None if line.is_empty() => None,
        None => Some((line, "")),
    }
}

/// A directive that takes exactly one word.
fn one(rest: &str) -> Result<&str, String> {
    let mut words = rest.split_whitespace();
    match (words.next(), words.next()) {
        (Some(word), None) => Ok(word),
        (Some(_), Some(_)) => Err(format!("{rest} is more than one word")),
        (None, _) => Err("nothing said".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIP: &str = "/home/pilot/a.insv";

    fn session(more: &str) -> String {
        format!(
            "session s\nresults /tmp/r.tsv\narm ship handover=8.0\narm wide handover=12.0\n{more}"
        )
    }

    fn trial(arms: &str) -> String {
        session(&format!(
            "trial down1\n  view {CLIP} time=65.666 yaw=179.00 pitch=-36.97 fov=20.00 lock=1\n  \
             loop 64.5 70.5\n  arms {arms}\n"
        ))
    }

    #[test]
    fn a_session_is_its_arms_and_its_trials() {
        let read = Session::read(&trial("wide ship")).expect("a session");
        assert_eq!(read.id, "s");
        assert_eq!(read.results, PathBuf::from("/tmp/r.tsv"));
        assert_eq!(read.trials.len(), 1);
        assert_eq!(read.arms_at(0), 2);
        let trial = &read.trials[0];
        assert_eq!(trial.id, "down1");
        assert_eq!(trial.clip, PathBuf::from(CLIP));
        assert_eq!(trial.from, Duration::from_millis(64_500));
        assert_eq!(trial.to, Duration::from_millis(70_500));
        // Position 1 is what the `arms` line put first, which is the whole of
        // how an order is staged.
        assert_eq!(read.knobs(0, 0), &[Knob::Handover(12.0)]);
        assert_eq!(read.knobs(0, 1), &[Knob::Handover(8.0)]);
    }

    /// The key is the only place a name is said, and it is not said while the
    /// window is open.
    #[test]
    fn the_key_maps_a_position_to_an_arm() {
        let read = Session::read(&trial("wide ship")).expect("a session");
        assert_eq!(read.key(), "down1 arm 1 = wide\ndown1 arm 2 = ship\n");
    }

    /// The point of the table: a knob the pass cannot take mid-flight is
    /// refused with the reason, before a window opens.
    #[test]
    fn a_knob_baked_into_the_shader_is_refused_by_name() {
        let said = Session::read(&session("arm steep blend_power=2.5\n")).expect_err("a refusal");
        assert!(said.contains("blend_power"), "{said}");
        assert!(said.contains("WGSL const"), "{said}");
    }

    #[test]
    fn a_knob_read_at_file_open_is_refused_too() {
        let said = Session::read(&session("arm slow seam_rounds=5\n")).expect_err("a refusal");
        assert!(said.contains("seam_rounds"), "{said}");
        assert!(said.contains("would not be a hot swap"), "{said}");
    }

    #[test]
    fn a_knob_nothing_has_ever_heard_of_is_refused_with_the_list() {
        let said = Session::read(&session("arm odd wobble=3\n")).expect_err("a refusal");
        assert!(said.contains("no knob called wobble"), "{said}");
        assert!(said.contains("handover"), "{said}");
        assert!(said.contains("sampling"), "{said}");
    }

    /// The renderer's own bound, asked at read time rather than shrugged off
    /// at the swap.
    #[test]
    fn a_width_the_renderer_would_refuse_is_refused_here() {
        assert!(Session::read(&session("arm huge handover=40\n")).is_err());
        assert!(Session::read(&session("arm none handover=0\n")).is_err());
        assert!(Session::read(&session("arm what handover=x\n")).is_err());
    }

    /// An arm that leaves a knob unset inherits whatever the arm before it
    /// left in the renderer, so what is compared is one arm and a mixture.
    #[test]
    fn every_arm_sets_every_knob() {
        let said = Session::read(
            "session s\nresults /tmp/r.tsv\narm a handover=8.0 sampling=luma\narm b handover=12.0\n",
        )
        .expect_err("a refusal");
        assert!(said.contains("every arm sets every knob"), "{said}");
    }

    /// The other live knob, in all three of the values the pass has.
    #[test]
    fn the_sampling_arm_reads_all_three_of_its_values() {
        for (value, sampling) in [
            ("bilinear", Sampling::Bilinear),
            ("luma", Sampling::Luma),
            ("sharp", Sampling::Sharp),
        ] {
            let text = trial("a b")
                .replace("arm ship handover=8.0", &format!("arm a sampling={value}"))
                .replace("arm wide handover=12.0", "arm b sampling=luma");
            let read = Session::read(&text).unwrap_or_else(|said| panic!("{value}: {said}"));
            assert_eq!(read.knobs(0, 0), &[Knob::Sampling(sampling)]);
        }
        let said = Session::read(&session("arm odd sampling=cubic\n")).expect_err("a refusal");
        assert!(said.contains("bilinear, luma or sharp"), "{said}");
    }

    #[test]
    fn a_trial_names_arms_the_session_has() {
        let said = Session::read(&trial("wide nobody")).expect_err("a refusal");
        assert!(said.contains("no arm called nobody"), "{said}");
        assert!(Session::read(&trial("wide")).is_err());
    }

    /// Half a trial is not a place to look, exactly as half a view is not.
    #[test]
    fn a_trial_missing_a_line_is_refused() {
        let no_view = session("trial t\n  loop 1 9\n  arms ship wide\n");
        assert!(
            Session::read(&no_view)
                .expect_err("a refusal")
                .contains("no view line")
        );
        let no_loop = session(&format!(
            "trial t\n  view {CLIP} time=2.0 yaw=0 pitch=0 fov=60 lock=1\n  arms ship wide\n"
        ));
        assert!(
            Session::read(&no_loop)
                .expect_err("a refusal")
                .contains("no loop line")
        );
    }

    /// A loop that does not contain the view would open somewhere and jump
    /// somewhere else on the first turn, and the owner would answer about a
    /// place nobody chose.
    #[test]
    fn the_view_sits_inside_the_loop() {
        let outside = session(&format!(
            "trial t\n  view {CLIP} time=2.0 yaw=0 pitch=0 fov=60 lock=1\n  loop 10 20\n  arms \
             ship wide\n"
        ));
        let said = Session::read(&outside).expect_err("a refusal");
        assert!(said.contains("the view is at 2.000"), "{said}");
    }

    #[test]
    fn a_loop_shorter_than_the_warm_up_is_refused() {
        let brief = session(&format!(
            "trial t\n  view {CLIP} time=2.0 yaw=0 pitch=0 fov=60 lock=1\n  loop 1.5 2.5\n  arms \
             ship wide\n"
        ));
        assert!(
            Session::read(&brief)
                .expect_err("a refusal")
                .contains("mostly the seek")
        );
    }

    #[test]
    fn a_session_with_one_arm_or_no_trial_is_not_a_comparison() {
        let lonely = "session s\nresults /tmp/r.tsv\narm ship handover=8.0\n";
        assert!(
            Session::read(lonely)
                .expect_err("a refusal")
                .contains("not a comparison")
        );
        assert!(
            Session::read(&session(""))
                .expect_err("a refusal")
                .contains("no trials")
        );
    }

    /// Comments, blank lines and indentation are the file's own, and a word
    /// no directive is named by is loud rather than skipped.
    #[test]
    fn the_grammar_is_the_grammar() {
        let commented = format!("# a session\n\n{}", trial("ship wide"));
        assert!(Session::read(&commented).is_ok());
        let runner = format!(
            "pool /tmp/pool\nkey /tmp/k.txt\nlogs /tmp/l\nseed 1234\nscheme balanced\n{}",
            trial("ship wide")
        );
        assert!(Session::read(&runner).is_ok());
        let typo = trial("ship wide").replace("trial down1", "trail down1");
        assert!(
            Session::read(&typo)
                .expect_err("a refusal")
                .contains("trail")
        );
        let empty = session("pool\n");
        assert!(
            Session::read(&empty)
                .expect_err("a refusal")
                .contains("says nothing")
        );
    }

    /// Appending is what makes a results file evidence: a session run twice
    /// leaves both runs, and the columns are written once.
    #[test]
    fn results_are_appended_under_one_header() {
        let folder = std::env::temp_dir().join(format!("kjerag-ab-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let mut read = Session::read(&trial("wide ship")).expect("a session");
        read.results = folder.join("deep").join("r.tsv");
        let answer = |trial: &str, vote| Answer {
            trial: trial.to_owned(),
            vote,
            loops: 3,
            swaps: 5,
            watched: Duration::from_millis(41_250),
        };
        append(&read, &answer("down1", Some(2))).expect("appended");
        append(&read, &answer("down2", None)).expect("appended");
        let text = std::fs::read_to_string(&read.results).expect("written");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], COLUMNS);
        let columns: Vec<&str> = lines[1].split('\t').collect();
        assert_eq!(columns.len(), 7);
        assert_eq!(&columns[1..], ["s", "down1", "2", "3", "5", "41.2"]);
        assert!(columns[0].parse::<u64>().expect("a unix second") > 1_700_000_000);
        // Cannot tell is a dash and not an empty column, so a split on tabs
        // counts the same seven either way.
        assert_eq!(lines[2].split('\t').nth(3), Some("-"));
        let _ = std::fs::remove_dir_all(&folder);
    }
}
