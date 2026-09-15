use std::path::PathBuf;
use std::sync::Arc;
use std::task::Context;
use std::time::{Duration, Instant};

use ffmpeg_next::Rational;
use kjerag_media::{FrameStamp, Frames, Player, PresentationPolicy, Timing};
use kjerag_meta::{ExposureTrack, Lens, OrientationTrack, Readout, Sweep};

use super::{Calibrated, Motion, Next, Scene, Show, Source};
use crate::Size;

const FRAME: Size = Size {
    width: 2,
    height: 2,
};

fn timing() -> Timing {
    Timing::new(Rational::new(30, 1), 300).unwrap()
}

fn frame(index: u64, timing: Timing, previous: Option<&FrameStamp>) -> (FrameStamp, Frames) {
    let stamp = FrameStamp::for_test(index, timing.time_of(index), previous);
    let frames = Frames::empty_for_test(stamp.clone(), FRAME);
    (stamp, frames)
}

fn generic_scene(player: Player) -> Scene {
    let calibrated = Calibrated {
        lenses: Arc::<[Lens]>::from([]),
        camera: 0,
        held: Arc::new(Motion {
            orientation: OrientationTrack::default(),
            exposure: ExposureTrack::default(),
            readout: Readout {
                seconds: 0.0,
                sweep: Sweep::Unknown,
            },
        }),
        one_xs: None,
        filtered: None,
    };
    let show = Show::new(
        Arc::<[PathBuf]>::from([]),
        FRAME,
        calibrated,
        None,
        Source::Live(Box::new(player)),
    );
    Scene {
        show: Some(show),
        ..Scene::blank()
    }
}

#[test]
fn overdue_empty_generic_sleeps_until_the_exact_decoded_source_arrives() {
    let timing = timing();
    let interval = timing.interval();
    let (player, decoder) = Player::controlled_for_test(timing, FRAME);
    let mut scene = generic_scene(player);
    scene.play();
    let mut listener = scene.ready_wake.listen();
    let mut context = Context::from_waker(std::task::Waker::noop());

    let (first_stamp, first) = frame(0, timing, None);
    decoder.deliver(first);
    let start = Instant::now();
    assert_eq!(
        scene.pump(start),
        Next::At(start + interval),
        "an empty queue must not replace a future source deadline"
    );
    assert_eq!(scene.frame_stamp().as_ref(), Some(&first_stamp));

    let overdue = start + interval * 2;
    assert_eq!(
        scene.pump(overdue),
        Next::Never,
        "an overdue empty generic queue with a listener kept polling"
    );
    assert!(listener.poll_ready(&mut context).is_pending());

    let (second_stamp, second) = frame(1, timing, Some(&first_stamp));
    decoder.deliver(second);
    assert!(
        listener.poll_ready(&mut context).is_ready(),
        "decoded source delivery did not wake the sleeping Scene"
    );

    // This source is already late. Present it, draw it once, and arm the same
    // arrival path for its absent successor without an extra polling redraw.
    assert_eq!(scene.pump(overdue), Next::Never);
    assert_eq!(scene.frame_stamp().as_ref(), Some(&second_stamp));
}

#[test]
fn overdue_empty_generic_without_a_listener_keeps_the_instrument_fallback() {
    let timing = timing();
    let interval = timing.interval();
    let (player, decoder) = Player::controlled_for_test(timing, FRAME);
    let mut scene = generic_scene(player);
    scene.play();

    let (_, first) = frame(0, timing, None);
    decoder.deliver(first);
    let start = Instant::now();
    assert_eq!(scene.pump(start), Next::At(start + interval));

    let overdue = start + interval * 2;
    assert_eq!(
        scene.pump(overdue),
        Next::At(start + interval),
        "an instrument without a listener lost its established polling fallback"
    );
}

#[test]
fn sequential_playback_does_not_use_the_generic_decoder_sleep() {
    let timing = timing();
    let interval = timing.interval();
    let (mut player, decoder) = Player::controlled_for_test(timing, FRAME);
    assert!(player.set_presentation_policy(PresentationPolicy::SequentialRealtime));
    let mut scene = generic_scene(player);
    scene.play();
    let _listener = scene.ready_wake.listen();

    let (_, first) = frame(0, timing, None);
    decoder.deliver(first);
    let start = Instant::now();
    assert_eq!(scene.pump(start), Next::At(start + interval));

    assert_eq!(
        scene.pump(start + Duration::from_secs(1)),
        Next::At(start + interval),
        "selected sequential cadence was replaced by the generic arrival sleep"
    );
}
