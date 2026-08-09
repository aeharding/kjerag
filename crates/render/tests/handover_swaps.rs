//! The handover width can be changed while the process runs, which is what
//! makes a mid-playback A/B arm possible at all (`crates/app/src/ab.rs`).
//!
//! **Why this is out here rather than in `projection.rs`'s own tests.** The
//! width is process-wide, exactly as the environment variable it stands in
//! for is, and the unit tests run in one process on many threads with several
//! of them asserting on the shipped 8 degrees. An integration test is its own
//! binary, so this one moves the width without anything else watching.
//!
//! One test function and not three, for the same reason: two functions in
//! this file would be two threads of one process, and the second would be
//! asserting about a width the first had moved.

use kjerag_render::{Reframe, ask_handover, takes_handover};

/// The shipped width, in degrees, which is `projection::CROSSOVER_DEG` and is
/// not public. A blank pane has one lens and no overlap, so what it carries
/// is the ask itself with nothing clamping it.
const SHIPPED: f32 = 8.0;

/// A width asked for now is the width the next block carries, and a width
/// outside the two lenses' overlap is refused and changes nothing.
///
/// `Reframe::blank` is the block with no file open, and its crossover is the
/// ask. That is the same read `Reframe::new` does for a real file
/// (`afforded`), one clamp later, and both are rebuilt from scratch every
/// frame by `ScenePipeline::prepare`, so what this measures is what the pass
/// draws with on the next redraw.
#[test]
fn a_width_asked_for_now_is_the_width_the_next_block_carries() {
    // `crossover_at` went with the adaptive width on 2026-08-08: the handover
    // is one number for the whole picture now and does not breathe with a
    // measured disparity, so `handover_width` is the same read with nothing to
    // pass it (docs/research/studio-parity.md 3.2).
    let width = || Reframe::blank(1.0, false).handover_width().to_degrees();

    // Nothing asked for yet: the environment's answer, and this test binary
    // has no KJERAG_HANDOVER_DEG in it.
    assert!((width() - SHIPPED).abs() < 0.001, "{}", width());

    // Two arms of one session, in one process, with no reopen and no rebuild.
    for asked in [2.0, 12.0, 0.5, SHIPPED] {
        ask_handover(asked).expect("a width inside the overlap");
        assert!((width() - asked).abs() < 0.001, "{asked}: {}", width());
    }

    // Refused, and the refusal does not move the picture: the ask before it
    // still stands. Every one of these is a session file the app would have
    // turned away before a window opened, and this is the bound it turns them
    // away against.
    for refused in [0.0, -1.0, 40.0, f32::NAN, f32::INFINITY] {
        assert!(takes_handover(refused).is_err(), "{refused}");
        assert!(ask_handover(refused).is_err(), "{refused}");
        assert!((width() - SHIPPED).abs() < 0.001, "{refused}: {}", width());
    }

    // Asking does not have to be told what was asked before: the last ask
    // wins, which is what lets an owner flip 1, 2, 1, 2 as often as he likes.
    ask_handover(3.0).expect("a width");
    ask_handover(9.0).expect("a width");
    assert!((width() - 9.0).abs() < 0.001, "{}", width());
}
