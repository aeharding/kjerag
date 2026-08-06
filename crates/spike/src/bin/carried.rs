//! What a `lock=1` view line meant before 2026-08-06, written in the frame it
//! means now.
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin carried -- <file.insv> \
//!   time=36.303 yaw=3.78
//! cargo run --release -p kjerag-spike --bin carried -- <file.insv> \
//!   time=50.117 yaw=-74.43 time=50.117 yaw=106.98
//! ```
//!
//! The lock became world-fixed that day (#165): [`Filter::yaw_seconds`] went
//! from 3 s to infinite, so the stabilized frame's zero stopped following the
//! aircraft's slow heading and became the heading the file opened on. A yaw
//! written down before then is measured in a frame that had been carried round
//! by however far that follow had got, and the rule is
//! `new_yaw = old_yaw + carried(t)`.
//!
//! **`carried(t)` is neither a constant nor a rate.** It is the old filter's
//! own low-passed heading at one instant, so it is worth degrees a second
//! while the aircraft turns and nothing at all while it flies straight: on
//! VID_20260714_193252_00_006 it is 6.8 degrees at the first frame, 44 by
//! 6.5 s and 157 by 36 s. Two lines a second apart in the same file get
//! different corrections, which is why this reads it per line rather than per
//! file.
//!
//! This solves the same IMU track twice, once with each filter, and
//! differences the two headings at the frame the line names. The two differ by
//! a rotation about the world vertical and by nothing else, so roll and pitch
//! carry over untouched and are not printed.
//!
//! **The instant is the frame's, on the camera's own clock.** A view line's
//! `time=` is media time; the frame at that time is the one a seek lands on
//! ([`Cue::Time`]), and the orientation is looked up at that frame's exposure
//! timestamp, which is what [`Scene`](kjerag_render::Scene) does for the
//! picture. Nothing is decoded here: the index is arithmetic on the frame rate
//! and the timestamp is read out of the trailer.
//!
//! **What this cannot say is whether the line was true when it was written.**
//! It re-derives across one change, the lock going world-fixed, on the track
//! the file carries today. A line older than #158 (2026-08-05) was also
//! written against a different horizon seed, and this instrument cannot see
//! that: it solves both filters with the seed the code has now. Where a line's
//! re-derived aim is checked in the picture, it is checked against a build of
//! the commit before #165 and not against the build the line was born on.

use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::{Cue, Fallible, Reader};
use kjerag_meta::{CalibrationSet, Filter};

/// The heading follow that shipped until 2026-08-06, in seconds. The frame
/// every stale `lock=1` yaw is written in is this filter's.
const FOLLOWED_S: f64 = 3.0;

fn main() -> Fallible<()> {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().ok_or(USAGE)?);
    let views = parse(args)?;

    let calibration = CalibrationSet::from_insv(&path)?;
    if calibration.imu.is_empty() {
        return Err(
            "this file carries no IMU record, so it has no lock and no frame to re-derive in"
                .into(),
        );
    }
    let timing = Reader::open(&path)?.timing();
    let locked = calibration.orientation(Filter::default());
    let followed = calibration.orientation(Filter {
        yaw_seconds: FOLLOWED_S,
        ..Filter::default()
    });

    println!(
        "file:   {} at {:.3} fps",
        path.file_name().unwrap_or_default().to_string_lossy(),
        timing.fps(),
    );
    println!(
        "\n{:>9}{:>8}{:>11}{:>13}{:>11}{:>11}",
        "time s", "frame", "read at s", "carried deg", "old yaw", "new yaw"
    );
    for view in &views {
        let index = Cue::Time(Duration::from_secs_f64(view.seconds)).index(timing);
        let at_us = calibration.exposure[0]
            .frame_time_us(index)
            .ok_or("that time is past the end of this file's exposure record")?;
        let carried = wrap(locked.at(at_us).heading() - followed.at(at_us).heading()).to_degrees();
        println!(
            "{:9.3}{index:8}{:11.3}{carried:13.2}{:11.2}{:11.2}",
            view.seconds,
            at_us as f64 * 1e-6,
            view.yaw,
            wrap((view.yaw + carried).to_radians()).to_degrees(),
        );
    }
    println!(
        "\nnew_yaw = old_yaw + carried, and pitch, roll and fov carry over unchanged.\n\
         `carried` is the heading the {FOLLOWED_S:.0} s follow had been taken by that instant, \
         which is what the world-fixed lock stopped taking."
    );
    Ok(())
}

const USAGE: &str = "usage: carried <file.insv> time=seconds yaw=degrees [time=... yaw=...]";

/// One line to re-derive: where it is in the file, and what it aimed at.
struct View {
    seconds: f64,
    yaw: f64,
}

/// `time=` opens a view and `yaw=` fills the one it opened, so a run can carry
/// as many lines as the file has and they read in the order they were written.
fn parse(args: impl Iterator<Item = String>) -> Fallible<Vec<View>> {
    let mut views: Vec<View> = Vec::new();
    for arg in args {
        let (key, value) = arg.split_once('=').ok_or(USAGE)?;
        match key {
            "time" => views.push(View {
                seconds: value.parse()?,
                yaw: 0.0,
            }),
            "yaw" => {
                let view = views.last_mut().ok_or("a yaw= needs a time= before it")?;
                view.yaw = value.parse()?;
            }
            _ => return Err(USAGE.into()),
        }
    }
    match views.is_empty() {
        true => Err(USAGE.into()),
        false => Ok(views),
    }
}

/// An angle wrapped into (-pi, pi], so that a heading crossing the back of the
/// compass is a small change and not a whole turn.
fn wrap(angle: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    (angle + PI).rem_euclid(TAU) - PI
}
