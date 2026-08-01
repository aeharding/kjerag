//! Sound against picture, measured over a long run (issue #13).
//!
//! The claim this exists to check is that the sound follows the presentation
//! clock and keeps following it: a sound card's crystal and `CLOCK_MONOTONIC`
//! are not the same clock, so a player that gets the sound right at the start
//! and never looks again is tens of milliseconds out half an hour later, which
//! is a paramotor flight.
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin sync -- <file.insv> [seconds] [stall=at:for] [ring]
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
//!
//! **Silences are timestamped, not just counted** (issue #97). The five-second
//! report counts underruns per window, and a count cannot tell a burst at the
//! start of a file from a hole three seconds into it: 227 underruns were read
//! as a startup burst for two issues running, and they were a three and a half
//! second hole in the middle of the owner's file. So every unbroken run of
//! underruns is collected here as one `gap:` line with the media time it
//! started at, the time the sound came back, and what the ring held while it
//! lasted. An empty ring is decode falling behind; a full one whose head is a
//! second behind the picture is sound arriving too late to play.
//!
//! `stall=12:1` holds the shell for a second at twelve seconds in, which
//! starves the ring on purpose. A clean run means nothing from an instrument
//! that has not been shown able to catch a dirty one.
//!
//! `ring` prints the ring's level and the media time the sound has been
//! decoded to once a second for the whole run, rather than only inside a
//! hole. That last number is the producer against the consumer: sound that
//! stops arriving shows up there a second before it is audible.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use kjerag_media::{Accuracy, Audio, Cue, Fallible, Player, Stats};

/// How often a line is printed, which is the app's own report cadence.
const REPORT: Duration = Duration::from_secs(5);

/// One refresh of a 60 Hz display. The shell cannot redraw faster than the
/// compositor takes frames, so neither does this: without the floor a run that
/// falls behind asks for a redraw that was already due and spins, which pegs a
/// core and makes the thing it is measuring worse.
const REFRESH: Duration = Duration::from_micros(16_666);

/// How long the sound has to hold before a silence counts as over. Underruns
/// arrive one device callback at a time and a hole is hundreds of them, so
/// without a gate every callback would be reported as its own gap.
const HEALED: Duration = Duration::from_millis(250);

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

/// A hole in the sound, in the file's own time: what the owner of issue #97
/// heard and where he heard it.
#[derive(Clone, Copy, Debug)]
struct Silence {
    started: Duration,
    ended: Duration,
    underruns: u64,
    /// The furthest the head of the ring was from the picture while it
    /// lasted, in microseconds, with its sign. Near zero is a ring that ran
    /// dry; a large negative number is sound that arrived after its moment
    /// had passed; a large positive one is sound waiting for a picture that
    /// has not got there yet.
    behind: i64,
    /// The most the ring held while it lasted, in microseconds.
    held: i64,
}

impl Silence {
    fn report(&self) -> String {
        format!(
            "gap:    sound stopped at {:.2} s, back at {:.2} s: {:.2} s of silence, \
             {} underruns, ring {:.0} ms at worst {:+.0} ms",
            self.started.as_secs_f64(),
            self.ended.as_secs_f64(),
            (self.ended - self.started).as_secs_f64(),
            self.underruns,
            self.held as f64 / 1000.0,
            self.behind as f64 / 1000.0,
        )
    }
}

/// Underrun counts turned into the silences a listener would hear.
#[derive(Default)]
struct Ear {
    seen: u64,
    open: Option<Silence>,
    heard: Vec<Silence>,
}

impl Ear {
    /// One reading of the counters, at the position the picture is at.
    fn sample(&mut self, at: Duration, audio: Audio) {
        let broke = audio.underruns.saturating_sub(self.seen);
        self.seen = audio.underruns;
        if broke > 0 {
            let open = self.open.get_or_insert(Silence {
                started: at,
                ended: at,
                underruns: 0,
                behind: 0,
                held: 0,
            });
            open.ended = at;
            open.underruns += broke;
            if audio.offset.abs() > open.behind.abs() {
                open.behind = audio.offset;
            }
            open.held = open.held.max(audio.queued);
            return;
        }
        let Some(open) = self.open else {
            return;
        };
        if at < open.ended + HEALED {
            return;
        }
        self.heard.push(open);
        self.open = None;
    }

    /// Whatever was still broken when the run ended.
    fn finish(&mut self) {
        self.heard.extend(self.open.take());
    }

    fn is_broken(&self) -> bool {
        self.open.is_some()
    }
}

/// The stall a run was told to inject: when, and for how long.
fn stall(args: &[String]) -> Option<(Duration, Duration)> {
    let raw = args.iter().find_map(|arg| arg.strip_prefix("stall="))?;
    let (at, held) = raw.split_once(':')?;
    Some((
        Duration::from_secs_f64(at.parse().ok()?),
        Duration::from_secs_f64(held.parse().ok()?),
    ))
}

fn main() -> Fallible<()> {
    let args: Vec<String> = std::env::args().collect();
    let input = PathBuf::from(
        args.get(1)
            .ok_or("usage: sync <file.insv> [seconds] [stall=at:for] [ring]")?,
    );
    let digits = |arg: &&String| arg.chars().all(|c| c.is_ascii_digit());
    let run = Duration::from_secs(match args.get(2).filter(digits) {
        Some(raw) => raw.parse()?,
        None => 300,
    });
    let mut injected = stall(&args);
    let always_ring = args.iter().any(|arg| arg == "ring");

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
    let mut ringed = start;
    let mut script = SCRIPT.iter().peekable();
    let mut ear = Ear::default();

    while start.elapsed() < run {
        let now = Instant::now();
        // The frames are dropped as soon as they are taken: this instrument
        // measures the sound, and holding them would only hold surfaces out of
        // the decoder's pool.
        player.pump(now)?;

        let stats = player.stats();
        let at = player.position(now);
        if let Some(audio) = stats.audio {
            ear.sample(at, audio);
            // The ring's level, by default only while the sound is broken:
            // what a hole is made of is worth a line a second, and a clean
            // half hour is not.
            let wanted = ear.is_broken() || always_ring;
            if wanted && now.duration_since(ringed) >= Duration::from_secs(1) {
                println!(
                    "ring:   {:>8.2} s, {:.0} ms queued, head {:+.0} ms, sound decoded to {:.2} s",
                    at.as_secs_f64(),
                    audio.queued as f64 / 1000.0,
                    audio.offset as f64 / 1000.0,
                    at.as_secs_f64() + (audio.offset + audio.queued) as f64 / 1e6,
                );
                ringed = now;
            }
        }

        if let Some((after, held)) = injected
            && start.elapsed() >= after
        {
            println!("act:    {:>8.2} s, Stall({held:?})", at.as_secs_f64());
            std::thread::sleep(held);
            injected = None;
        }

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
            println!(
                "play:   {:>8.2} s, {}",
                at.as_secs_f64(),
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

    ear.finish();
    println!(
        "sound:  {} hole(s) in {:.0} s of playback",
        ear.heard.len(),
        run.as_secs_f64()
    );
    for silence in &ear.heard {
        println!("{}", silence.report());
    }
    Ok(())
}
