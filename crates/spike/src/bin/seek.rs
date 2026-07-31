//! How long a seek takes, and whether that depends on where in the file it
//! lands.
//!
//! Issue #5 asks for a keyframe index built from `moov` at open. There is
//! already one: libavformat parses `stss`/`stco` when the file is opened, so
//! `av_seek_frame` is a lookup in a table that is already in memory. This
//! instrument is the evidence for that, and the numbers behind the scrub
//! design: a drag seeks to keyframes (one decode per lens) and the release
//! seeks exactly (that keyframe, then every frame between).
//!
//! ```sh
//! cargo run --release -p kyerag-spike --bin seek -- <file.insv>
//! ```
//!
//! It measures two shapes of scrub. One seek at a time is what a hand that
//! stops between positions asks for; a drag fires a position per pointer
//! move whether or not the picture has caught up, and only the second one
//! says how often the picture actually changes under a finger.
//!
//! Nothing is written to disk: this instrument reports, it does not render
//! pictures of real footage.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kyerag_media::{Accuracy, Cue, Fallible, Player, Reader};

/// Where in the file each seek asks for, as a fraction of its length. The
/// order jumps about on purpose: that is what a drag does, and a walk in
/// order would be flattered by anything the kernel read ahead.
const PLACES: [f64; 12] = [
    0.01, 0.97, 0.33, 0.66, 0.5, 0.9, 0.05, 0.75, 0.2, 0.42, 0.85, 0.13,
];

fn main() -> Fallible<()> {
    let args: Vec<String> = std::env::args().collect();
    let input = PathBuf::from(args.get(1).ok_or("usage: seek <file.insv>")?);

    let opened = Instant::now();
    let reader = Reader::open(&input)?;
    let timing = reader.timing();
    println!(
        "open:   {:.1} ms for a {} MB file, {} frames, {:.1} s",
        opened.elapsed().as_secs_f64() * 1000.0,
        std::fs::metadata(&input)?.len() / 1_000_000,
        timing.frames,
        timing.duration().as_secs_f64(),
    );
    println!(
        "        the whole keyframe table comes out of moov here, which is\n\
         \x20       why the numbers below do not care where in the file they land\n"
    );
    drop(reader);

    for lookahead in [0, 2] {
        for accuracy in [Accuracy::Keyframe, Accuracy::Exact] {
            println!("{}", measure(&input, lookahead, accuracy)?);
        }
    }
    println!();
    for accuracy in [Accuracy::Keyframe, Accuracy::Exact] {
        println!("{}", scrub(&input, accuracy)?);
    }
    println!();
    // 60 is what iced coalesces pointer moves to, and so the fastest a
    // scrubber can be asked for a position; the slower rates are a hand that
    // is not throwing the pointer across the window.
    for rate in [10, 20, 30, 45, 60] {
        println!("{}", drag(&input, rate, Duration::from_secs(2))?);
    }
    Ok(())
}

/// A drag, which is not the same shape as the jumps above: the shell fires a
/// position per pointer move whether or not the picture has caught up, so
/// the commands arrive faster than a landing takes and most are overtaken
/// before they are read. What the pilot judges is how often the picture
/// changes, and that depends on how fast the hand is going, so `moves` is
/// swept rather than picked.
///
/// A picture only reaches the screen while its own seek is still the newest
/// (the epoch discipline in `Player::pump`), so a hand moving faster than a
/// landing takes shows nothing at all until it slows down.
///
/// The release is measured with it, because the whole point of dragging to
/// keyframes is that letting go still lands on the frame under the finger.
fn drag(input: &Path, moves: u32, over: Duration) -> Fallible<String> {
    let mut player = Player::open(input)?;
    let timing = player.timing();
    while player.pump(Instant::now())?.is_none() {
        std::thread::sleep(Duration::from_millis(1));
    }

    let period = Duration::from_secs(1) / moves;
    let start = Instant::now();
    let mut updates = 0u32;
    let mut moved = start;
    let mut at = Duration::ZERO;
    loop {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(start);
        if elapsed >= over {
            break;
        }
        if now >= moved {
            // One end of the file to the other, so that no two positions
            // are near each other.
            at = timing
                .duration()
                .mul_f64(0.05 + 0.9 * elapsed.div_duration_f64(over));
            player.seek(Cue::Time(at), Accuracy::Keyframe);
            moved += period;
        }
        if player.pump(now)?.is_some() {
            updates += 1;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    let released = Instant::now();
    player.seek(Cue::Time(at), Accuracy::Exact);
    while player.pump(Instant::now())?.is_none() {
        std::thread::sleep(Duration::from_millis(1));
    }
    let landed = player.index().ok_or("the release produced no picture")?;

    Ok(format!(
        "drag at {moves:3} positions/s: {:5.1} picture updates/s, release \
         landed on {landed} wanting {} in {:5.1} ms",
        f64::from(updates) / over.as_secs_f64(),
        timing.index_at(at),
        released.elapsed().as_secs_f64() * 1000.0,
    ))
}

/// One seek at a time through the whole path: the shell asks the [`Player`],
/// the decode thread seeks, and a redraw finds the picture changed. The
/// reader numbers above are the decode half of this; the rest is the thread
/// handover. [`drag`] is the same path with the next position asked for
/// before the last picture arrives, which is what a hand actually does.
///
/// The poll here is 1 ms because the point is the engine's own cost. A window
/// asks again at its refresh rate, so add up to one refresh (16.7 ms at
/// 60 Hz) for what the pilot sees.
fn scrub(input: &Path, accuracy: Accuracy) -> Fallible<String> {
    let mut player = Player::open(input)?;
    let timing = player.timing();
    while player.pump(Instant::now())?.is_none() {
        std::thread::sleep(Duration::from_millis(1));
    }

    let mut took = Vec::with_capacity(PLACES.len());
    for place in PLACES {
        let at = timing.duration().mul_f64(place);
        let start = Instant::now();
        player.seek(Cue::Time(at), accuracy);
        while player.pump(Instant::now())?.is_none() {
            std::thread::sleep(Duration::from_millis(1));
        }
        took.push((start.elapsed(), at));
    }
    took.sort_unstable();

    Ok(format!(
        "{:9} through the player: median {:6.1} ms, worst {:6.1} ms at {:6.1} s",
        format!("{accuracy:?}"),
        took[took.len() / 2].0.as_secs_f64() * 1000.0,
        took[took.len() - 1].0.as_secs_f64() * 1000.0,
        took[took.len() - 1].1.as_secs_f64(),
    ))
}

/// One run of [`PLACES`] against a warm reader.
fn measure(input: &Path, lookahead: usize, accuracy: Accuracy) -> Fallible<String> {
    let mut reader = Reader::open(input)?.lookahead(lookahead);
    let timing = reader.timing();
    // The first frame of the file pays for opening the decoders and building
    // the VA-API surface pool. A scrub in a running player never does.
    reader.next_frames()?;

    let mut took = Vec::with_capacity(PLACES.len());
    let mut behind = Duration::ZERO;
    for place in PLACES {
        let at = timing.duration().mul_f64(place);
        let start = Instant::now();
        reader.seek(Cue::Time(at), accuracy)?;
        let frames = reader
            .next_frames()?
            .ok_or("the file ended where a seek was aiming")?;
        took.push((start.elapsed(), at));
        behind += at.saturating_sub(frames.timestamp);
    }
    took.sort_unstable();
    let (worst, where_worst) = took[took.len() - 1];

    Ok(format!(
        "{:9} lookahead {lookahead}: median {:6.1} ms, worst {:6.1} ms at \
         {:6.1} s, landed {:5.0} ms early on average",
        format!("{accuracy:?}"),
        took[took.len() / 2].0.as_secs_f64() * 1000.0,
        worst.as_secs_f64() * 1000.0,
        where_worst.as_secs_f64(),
        behind.as_secs_f64() * 1000.0 / took.len() as f64,
    ))
}
