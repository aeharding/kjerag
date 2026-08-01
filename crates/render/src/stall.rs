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
//! The run lives in the [`Stalled`] the open capture owns, not on the
//! pipeline, and that is what keeps the latch from coming back. iced builds
//! one pipeline per renderer and keeps it for the life of the window, so
//! anything the pipeline remembers about a failure it remembers about every
//! later file as well.

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
/// One line, and it is the developer's. What the pilot is told is the shell's
/// alert, and that says the same thing however the picture died, because the
/// answer is the same one: open the file again.
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
    /// Giving up ends the run as well as raising the stall, so the count
    /// starts again from nothing: a pilot who presses play after being told
    /// gets the same two seconds of patience the first attempt had, and is
    /// told again if it is still stuck.
    pub(crate) fn failed(&self, now: Instant, why: impl fmt::Display) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
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
        state.raised = Some(Stall::new(format!(
            "{failures} frames could not be imported over {:.1} s, last: {why}",
            lasted.as_secs_f64(),
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
            stalled.failed(start + Duration::from_millis(frame * 33), "EMFILE");
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
            stalled.failed(start + Duration::from_secs(minute * 60), "EMFILE");
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
            stalled.failed(start + at, "EMFILE");
            assert_eq!(stalled.take(), None, "gave up after {at:?}");
            at += Duration::from_millis(33);
        }

        stalled.failed(start + STUCK_FOR, "EMFILE");
        let raised = stalled.take().expect("nothing raised after STUCK_FOR");
        assert!(raised.to_string().contains("EMFILE"), "{raised}");
        assert!(raised.to_string().contains("2.0 s"), "{raised}");

        // Taken, so it is said once: a shell that redraws again does not put
        // the alert up a second time.
        assert_eq!(stalled.take(), None);
    }

    /// Giving up starts the clock again rather than latching, so a pilot who
    /// presses play into a still-stuck pipeline is told again, and one who
    /// presses play into a healthy one is not told at all.
    #[test]
    fn giving_up_is_not_a_latch() {
        let stalled = Stalled::default();
        let start = Instant::now();
        stalled.failed(start, "EMFILE");
        stalled.failed(start + STUCK_FOR, "EMFILE");
        assert!(stalled.take().is_some());

        // The very next failure is the start of a new run, not the end of the
        // old one.
        stalled.failed(start + STUCK_FOR + Duration::from_millis(33), "EMFILE");
        assert_eq!(stalled.take(), None);

        stalled.failed(start + STUCK_FOR * 2 + Duration::from_millis(33), "EMFILE");
        assert!(stalled.take().is_some());
    }
}
