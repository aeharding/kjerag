//! What the pass says when it cannot draw, and the only way it says it.
//!
//! A frame import fails for reasons that pass. The box runs out of file
//! descriptors and `dup` answers `EMFILE`; the driver runs out of device
//! memory; a surface is handed over while something else is being torn down.
//! Before issue #124 the first of those was the last frame the player ever
//! drew: one failed import latched a flag on the pipeline, the picture was
//! gone until the app was restarted, the sound played on over it, and the
//! whole of what was said about it was one line on a terminal that a
//! launcher-started Flatpak sends nowhere.
//!
//! So a failure costs a frame, and only a run of them costs the file. The run
//! is measured in time rather than in frames: what the pilot is looking at is
//! a picture that has been frozen for so long, and how many redraws went by
//! inside that is a property of his display and his footage rather than of
//! the failure.
//!
//! When the run does outlast the bound, that capture is over and stays over.
//! The owner's ruling (2026-08-01, issue #124) came from testing the first
//! shape of this, which re-armed as soon as the alert was closed and had
//! another go on its own: "I keep pressing OK and it keeps coming back. Very
//! buggy!" Five alerts in one sitting, from an app whose alert was telling him
//! to open the file again while quietly retrying behind it. So there is one
//! alert per open, and the way back is the pilot opening the file, which is a
//! new capture with a new one of these.
//!
//! That the stop is remembered here rather than on the pipeline is the whole
//! difference from what issue #124 started as. iced builds one pipeline per
//! renderer and keeps it for the life of the window, so a flag there is one
//! every later file inherits. The old bug was not that a stop was remembered;
//! it was remembered by the wrong thing, and after one frame rather than after
//! two seconds.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a run of failed imports has to last before the pass gives up on
/// the file.
///
/// Two seconds, which is the same "long enough that a person has noticed" the
/// shell already measures its controls out in (`CONTROLS_TIMEOUT`). Under it,
/// a hiccup costs the frames it covers and the picture carries on: about 60
/// frames of 30 fps content, which is a stutter and not a session. Over it,
/// the picture is not stuttering, it is dead, and another 30 attempts a
/// second are worth less to the pilot than being told.
pub const STUCK_FOR: Duration = Duration::from_secs(2);

/// The picture stopped and will not come back by itself (issue #124).
///
/// One line, and the pilot reads it. The shell puts it in the alert with
/// "Open the file again." on the end and echoes it to the terminal unchanged
/// (`fail::Failure::Stopped`). It was the developer's alone until 2026-08-01,
/// when the owner ruled that a failure says why in its own words: the
/// sentence that used to stand here in the window knew less than this line
/// does. So this is UI copy now, and the rules for it bind: plain words, no
/// em dashes, specific enough to be worth reading (AGENTS.md).
///
/// The shell cannot be handed one of these and quietly do nothing with it. It
/// arrives as a [`Next::Stopped`] arm, which every caller of
/// [`Scene::pump`] must match, and the widget turns that arm into a message
/// the application's own `Message` has to be able to carry.
///
/// [`Next::Stopped`]: super::Next::Stopped
/// [`Scene::pump`]: super::Scene::pump
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stall(String);

impl Stall {
    /// Public so the shell can build one in its own tests: what a stall is
    /// worth is what the window does with it, and that is testable with no
    /// GPU. Nothing about the funnel rests on this being hard to make. What
    /// it rests on is that the shell has no second way to report one.
    pub fn new(why: impl fmt::Display) -> Self {
        Self(why.to_string())
    }
}

impl fmt::Display for Stall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where the pass leaves a [`Stall`] for the shell, and the run of failures
/// that decides whether there is one to leave.
///
/// The same one-slot handoff [`Shutter`] makes in the other direction, and
/// for the same reason: the shell only ever holds a [`Scene`], the pipeline
/// is iced's, and neither can reach the other any other way.
///
/// [`Shutter`]: super::capture::Shutter
/// [`Scene`]: super::Scene
#[derive(Clone, Default)]
pub(crate) struct Stalled(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    /// The run of consecutive failures in flight, and `None` whenever the
    /// last import landed.
    run: Option<Run>,
    /// Raised once a run outlasts [`STUCK_FOR`], and taken by the shell.
    raised: Option<Stall>,
    /// Set when the bound trips and never cleared: this capture is over.
    stopped: bool,
}

/// One unbroken run of failed imports: when it started, and how many frames
/// it has cost.
struct Run {
    since: Instant,
    failures: u32,
}

impl Stalled {
    /// One import failed.
    ///
    /// A capture that has already been given up on counts nothing and says
    /// nothing: it is stopped, and the pass has no reason to be importing into
    /// it at all (`ScenePipeline::show` asks before it tries).
    ///
    /// `has_frame` is whether the pass has a frame to hold on screen once it
    /// gives up. It usually does, and the pilot gets a frozen picture under
    /// the alert. When it does not, because nothing was ever presented, the
    /// pane falls to the backdrop, and the line says so: an empty pane and a
    /// frozen one look like different bugs from the outside, and the one
    /// sentence the pilot is given is where the difference is written down.
    pub(crate) fn failed(&self, now: Instant, why: impl fmt::Display, has_frame: bool) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        if state.stopped {
            return;
        }
        let run = state.run.get_or_insert(Run {
            since: now,
            failures: 0,
        });
        run.failures += 1;
        let lasted = now.saturating_duration_since(run.since);
        if lasted < STUCK_FOR {
            return;
        }
        let failures = run.failures;
        state.run = None;
        state.stopped = true;
        state.raised = Some(Stall::new(format!(
            "{failures} frames could not be imported over {:.1} s, last: {why}{}",
            lasted.as_secs_f64(),
            match has_frame {
                true => "",
                false => ", and no frame was ever shown, so the pane is empty",
            },
        )));
    }

    /// An import landed, so whatever run was in flight is over. Called on
    /// every frame that reaches the screen, which is one uncontended lock
    /// against a dmabuf import.
    pub(crate) fn landed(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.run = None;
        }
    }

    /// The stall the shell has to say out loud, once.
    pub(crate) fn take(&self) -> Option<Stall> {
        self.0.lock().ok()?.raised.take()
    }

    /// Whether this capture has been given up on, which nothing sets back.
    ///
    /// Two things read it and they are the two halves of what the owner asked
    /// for: the pass stops importing, so there is no second run to raise a
    /// second alert, and the scene stops handing out its player, so nothing
    /// the pilot presses can start the clock over a picture that is not
    /// coming back. His way out is to open the file, which builds another
    /// `Scene` and another one of these.
    pub(crate) fn stopped(&self) -> bool {
        self.0.lock().is_ok_and(|state| state.stopped)
    }
}

impl fmt::Debug for Stalled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let raised = matches!(
            self.0.lock().as_deref(),
            Ok(State {
                raised: Some(_),
                ..
            })
        );
        f.debug_tuple("Stalled").field(&raised).finish()
    }
}

/// The bound, which is the whole of what issue #124 turns on: a transient
/// failure costs frames and a stuck one costs the file. No GPU and no window;
/// the clock is handed in.
#[cfg(test)]
mod tests {
    use super::*;

    /// A hiccup: half a second of failures, then the imports land again.
    /// Nothing is said, because nothing is wrong any more.
    #[test]
    fn a_short_run_costs_frames_and_nothing_else() {
        let stalled = Stalled::default();
        let start = Instant::now();
        for frame in 0..15 {
            stalled.failed(start + Duration::from_millis(frame * 33), "EMFILE", true);
        }
        assert_eq!(stalled.take(), None);

        stalled.landed();
        assert_eq!(stalled.take(), None);
    }

    /// The latch, as issue #124 met it: one failure, and then a picture that
    /// never comes back. A single failure a long time ago cannot give up on
    /// a file that has been drawing ever since.
    #[test]
    fn one_failure_long_ago_does_not_give_up_now() {
        let stalled = Stalled::default();
        let start = Instant::now();
        for minute in 0..10 {
            stalled.failed(start + Duration::from_secs(minute * 60), "EMFILE", true);
            stalled.landed();
        }
        assert_eq!(stalled.take(), None);
    }

    /// And a run that does not end: the picture has been gone for two
    /// seconds, so the pass gives up and the shell is handed the line.
    #[test]
    fn a_run_that_outlasts_the_bound_gives_up_once() {
        let stalled = Stalled::default();
        let start = Instant::now();
        let mut at = Duration::ZERO;
        while at < STUCK_FOR {
            stalled.failed(start + at, "EMFILE", true);
            assert_eq!(stalled.take(), None, "gave up after {at:?}");
            at += Duration::from_millis(33);
        }

        stalled.failed(start + STUCK_FOR, "EMFILE", true);
        let raised = stalled.take().expect("nothing raised after STUCK_FOR");
        assert!(raised.to_string().contains("EMFILE"), "{raised}");
        assert!(raised.to_string().contains("2.0 s"), "{raised}");
        // There was a frame to hold, so the line says nothing about the pane.
        assert!(!raised.to_string().contains("pane is empty"), "{raised}");

        // Taken, so it is said once: a shell that redraws again does not put
        // the alert up a second time.
        assert_eq!(stalled.take(), None);
    }

    /// And giving up is the end of this capture, which is the owner's ruling
    /// after testing the shape that re-armed: closing the alert let the pass
    /// try again, and two seconds later it said the same thing, five times
    /// over. However long the failures go on, one open is one alert.
    #[test]
    fn giving_up_is_the_end_of_this_capture() {
        let stalled = Stalled::default();
        let start = Instant::now();
        assert!(!stalled.stopped());

        stalled.failed(start, "EMFILE", true);
        stalled.failed(start + STUCK_FOR, "EMFILE", true);
        assert!(stalled.take().is_some());
        assert!(stalled.stopped());

        // A minute of redraws still failing, which is what the gate left on
        // looks like. Nothing counts and nothing is raised again.
        let mut at = STUCK_FOR;
        while at < Duration::from_secs(60) {
            stalled.failed(start + at, "EMFILE", true);
            at += Duration::from_millis(33);
        }
        assert_eq!(stalled.take(), None);
        assert!(stalled.stopped());
    }

    /// The one case with nothing to freeze on: every import of this file
    /// failed, the first included, so there is no picture to hold under the
    /// alert and the pane is the backdrop. The pilot is told the same thing
    /// either way, because there is the same one thing to do about it; the
    /// terminal is where the difference is written down.
    #[test]
    fn a_stop_with_no_frame_to_hold_says_so() {
        let stalled = Stalled::default();
        let start = Instant::now();
        stalled.failed(start, "EMFILE", false);
        stalled.failed(start + STUCK_FOR, "EMFILE", false);
        let raised = stalled.take().expect("nothing raised after STUCK_FOR");
        assert!(raised.to_string().contains("pane is empty"), "{raised}");
    }

    /// The way back is a new capture, so what says the latch is not the old
    /// one is that nothing here carries over: opening the file again builds
    /// another `Scene`, and its `Stalled` has never heard of this one.
    #[test]
    fn opening_the_file_again_is_a_fresh_detector() {
        let stopped = Stalled::default();
        let start = Instant::now();
        stopped.failed(start, "EMFILE", true);
        stopped.failed(start + STUCK_FOR, "EMFILE", true);
        assert!(stopped.stopped());

        let reopened = Stalled::default();
        assert!(!reopened.stopped());
        assert_eq!(reopened.take(), None);

        // And it has the same patience the first one had.
        reopened.failed(start, "EMFILE", true);
        assert_eq!(reopened.take(), None);
        assert!(!reopened.stopped());
        reopened.landed();
        assert!(!reopened.stopped());
    }
}
