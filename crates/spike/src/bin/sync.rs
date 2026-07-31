//! Sound against picture, measured over a long run (issue #13).
//!
//! The claim this exists to check is that the sound follows the presentation
//! clock and keeps following it: a sound card's crystal and `CLOCK_MONOTONIC`
//! are not the same clock, so a player that gets the sound right at the start
//! and never looks again is tens of milliseconds out half an hour later, which
//! is a paramotor flight.
//!
//! ```sh
//! cargo run --release -p kyerag-spike --bin sync -- <file.insv> [seconds]
//! ```
//!
//! It runs the real [`Player`]: the same demuxer, the same decode thread, the
//! same device, and the same clock. There is no GPU in it, because the sound
//! path does not touch one; frames are pumped and dropped at the rate the
//! shell would pump them, so the decode side costs what it costs in the app.
//!
//! It also **exercises the joins**, on a schedule it prints, because the
//! places a player pops are the places it stops and starts: a pause, a resume,
//! a scrub back and a scrub forward. Recording the output device's monitor
//! while this runs and looking for a step at those instants is what says
//! whether the fade is doing its job; the ring's own arithmetic cannot see a
//! click, only where the sound is.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use kyerag_media::{Accuracy, Cue, Fallible, Player, Stats};

/// How often a line is printed, which is the app's own report cadence.
const REPORT: Duration = Duration::from_secs(5);

/// One refresh of a 60 Hz display. The shell cannot redraw faster than the
/// compositor takes frames, so neither does this: without the floor a run that
/// falls behind asks for a redraw that was already due and spins, which pegs a
/// core and makes the thing it is measuring worse.
const REFRESH: Duration = Duration::from_micros(16_666);

/// What the run does to the transport, and when. Every one of these is a
/// join in the sound.
const SCRIPT: [(u64, Act); 6] = [
    (20, Act::Pause),
    (23, Act::Play),
    (30, Act::Seek(600.0)),
    (40, Act::Seek(120.0)),
    (50, Act::Step),
    (55, Act::Play),
];

#[derive(Clone, Copy, Debug)]
enum Act {
    Pause,
    Play,
    /// To this many seconds into the file.
    Seek(f64),
    /// One frame on, which pauses.
    Step,
}

fn main() -> Fallible<()> {
    let args: Vec<String> = std::env::args().collect();
    let input = PathBuf::from(args.get(1).ok_or("usage: sync <file.insv> [seconds]")?);
    let run = Duration::from_secs(match args.get(2) {
        Some(raw) => raw.parse()?,
        None => 300,
    });

    let mut player = Player::open(&input)?;
    println!(
        "media:  {} lens stream(s), {:.3} fps, {:.1} s, sound: {}",
        player.lenses(),
        player.timing().fps(),
        player.timing().duration().as_secs_f64(),
        match player.has_sound() {
            true => "yes",
            false => "none",
        },
    );
    player.play();

    let start = Instant::now();
    let (mut reported, mut counted) = (start, Stats::default());
    let mut script = SCRIPT.iter().peekable();

    while start.elapsed() < run {
        let now = Instant::now();
        // The frames are dropped as soon as they are taken: this instrument
        // measures the sound, and holding them would only hold surfaces out of
        // the decoder's pool.
        player.pump(now)?;

        if let Some((at, act)) = script.peek()
            && start.elapsed() >= Duration::from_secs(*at)
        {
            println!("act:    {:>8.2} s, {act:?}", start.elapsed().as_secs_f64());
            match act {
                Act::Pause => player.pause(now),
                Act::Play => player.play(),
                Act::Seek(to) => {
                    player.seek(Cue::Time(Duration::from_secs_f64(*to)), Accuracy::Exact)
                }
                Act::Step => player.step(now, 1),
            }
            script.next();
        }

        if now.duration_since(reported) >= REPORT {
            let stats = player.stats();
            println!(
                "play:   {:>8.2} s, {}",
                player.position(now).as_secs_f64(),
                stats.since(counted).report(now.duration_since(reported)),
            );
            (reported, counted) = (now, stats);
        }

        let earliest = now + REFRESH;
        let next = player.next_due().unwrap_or(earliest).max(earliest);
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    Ok(())
}
