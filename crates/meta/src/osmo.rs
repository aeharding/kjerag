//! The DJI Osmo 360 `.OSV` lens calibration, read out of the file's own
//! telemetry track.
//!
//! An `.OSV` is a plain ISO-BMFF MP4: two HEVC Main 10 fisheye video tracks,
//! one AAC track, and four DJI tracks whose sample entries are `djmd`
//! (telemetry) and `dbgi` (debug). Sample 0 of the **first** `djmd` track is a
//! protobuf message that opens with the camera's own name for its schema,
//! `dvtm_oq101.proto`, and carries the factory lens calibration in it. No
//! `.proto` file exists anywhere, so this reads the wire format by field
//! number: [`Fields`] is the whole of the decoder, and every number it looks
//! for is named in [`field`] below.
//!
//! **What is in a lens entry, and what is not.** Sixteen of them are written,
//! two of which carry the numbers this reads (below), and each holds a focal
//! pair, a principal point, **five** `k` coefficients, a reference frame size,
//! a yaw/pitch/roll triple, a unit quaternion, and a fourteen-point polyline.
//! There is no inter-lens translation in it at all, which is why
//! [`Pose::translation_m`] comes out zero and the seam band that measures
//! parallax switches itself off (`kjerag_render::band::baseline`).
//!
//! **The lens model is the Kannala-Brandt fisheye and all five coefficients
//! are applied** ([`Model::Theta`]):
//!
//! ```text
//! r = fx * theta * (1 + k1 t^2 + k2 t^4 + k3 t^6 + k4 t^8 + k5 t^10)
//! ```
//!
//! The fifth is **field 15**, seven fields past the four that sit together, on
//! the far side of the yaw/pitch/roll triple. Until 2026-08-08 this file read
//! the four, measured that they fold the radius over before 90 degrees, and
//! shipped the plain equidistant map with the coefficients called decoration.
//! The four do fold - between 88.4 and 89.8 degrees on the four lenses of the
//! two units - and the fifth is what stops them: with it the radius is
//! monotone in `theta` over the whole sphere on all four, and the model
//! reaches half the delivered frame at 98.9 to 99.4 degrees off axis, so
//! **197.9 to 198.8 degrees of coverage**, which is the Osmo 360's published
//! 199. Equidistant puts that same radius at 209.0 to 210.6 degrees, which is
//! a lens nobody makes, and that surplus is the seam tear: far content that
//! must satisfy `theta0 + theta1 = 180` came out at 194.8 under the shipped
//! map and comes out at 179.4 +-0.6 under this one, which is 272 px of
//! doubled picture across the seam turned into about one
//! (docs/ROADMAP.md 2026-08-08).
//!
//! **What was wrong was the check, not just the arithmetic.** The refusal
//! above rested on fields 22 and 23, the fourteen-point polyline, read as the
//! rim of *this lens's* coverage. It is not per lens and it is not per unit:
//! those 112 bytes are **byte-identical across all four lenses of both units**
//! (verified 2026-08-08), so they are a constant of the model - the nominal
//! arc where the camera's own body cuts the bottom of the picture, which two
//! lenses on one body share - and a constant cannot measure a lens. Under the
//! five-term model that arc sits at 91.7 to 92.6 degrees off axis, which is a
//! body edge and not a coverage rim. A per-unit fact was wanted and a model
//! constant was used, so the number it produced agreed with the published
//! figure under the wrong model and refused the right one.
//!
//! The check that replaced it is per unit and needs no polyline: the delivered
//! frame is 3840 px across and the picture fills it to its edges (measured on
//! frames of both units), so `theta` at 1920 px is half the coverage, and the
//! model has to put it at the published figure. Repeatable from any `.OSV`
//! with fields 1, 5 to 8 and 15 of a lens entry and nothing else.
//!
//! **Which entry pair.** The table holds 24 slots. Slots 1 and 2 are the two
//! lenses and are what this reads; 3 to 10 are empty on every file in the
//! corpus; 11 to 24 are seven further pairs whose focal lengths walk
//! downwards by about 0.03 percent a rung and whose principal points barely
//! move. What that ladder selects on is **not known** - a focus distance, a
//! temperature, a stabilization crop are all consistent with it - so this
//! takes the first pair, which is the one the two cameras' factory numbers
//! agree in shape with. The risk it carries is the width of that ladder: on
//! both cameras, slot 1's focal sits about 0.16 percent above its top rung and
//! 0.35 percent above its bottom, so a file whose true rung is elsewhere is
//! reprojected up to about a third of a percent off in scale.
//!
//! **Which entry is which stream** is taken as file order: entry 1 is stream
//! 0 and entry 2 is stream 1. Entry 1's recorded yaw is about 180 degrees and
//! entry 2's about 0, so the pair is the back-to-back arrangement either way
//! round, and both orders render a level, closed sphere because the two
//! calibrations differ by about a tenth of a percent in focal length and ten
//! px in principal point. An attempt to settle it by scoring overlap
//! agreement was **thrown away rather than believed**: on the far-field
//! capture the score improved when a known 20 px principal-point error was
//! injected into it, so it could not tell a right answer from a wrong one and
//! its preference is not evidence. If the order is backwards, what it costs
//! is that ten px of principal point and a tenth of a percent of scale, at
//! the seam.
//!
//! **The orientation, and which way round it is written.** Every `djmd`
//! sample, not just the first, carries the camera's own answer for its frame:
//! field 3.2.9, a unit quaternion in the same four-`f32` shape the lens
//! entries use, `w` first. Field 3.3 beside it holds the same numbers at about
//! 1 kHz, 33 or 40 a frame depending on the rate, and **the frame's own one is
//! a member of that run** (index 8 of 33 on unit B, 20 of 40 on unit A), so
//! the fast stream is read past: it says nothing the per-frame one does not,
//! it needs an anchor the two units do not agree on, and nothing in the pass
//! asks for an orientation between frames because this camera records no
//! rolling-shutter readout to correct for. Reading one a frame costs a seek
//! and about a kilobyte each: 4384 samples and 4.1 MB in 26 ms on the owner's
//! 146 s capture, about a third of a second for a half-hour one.
//!
//! Until 2026-08-08 none of it was read, because the frame was not pinned and
//! "applying the quaternions made the stitch worse" - which was measured under
//! the four-coefficient lens model above, with 15 degrees of seam tear in the
//! picture, so it settled nothing. Re-derived under the five-term model, the
//! convention is:
//!
//! ```text
//! world_from_body = BODY^-1 . conjugate(w, x, y, z) . BODY
//! ```
//!
//! **The component order is (w, x, y, z)** and it is not a guess: carried
//! through the file's own quaternion the camera's vertical holds still while
//! the wearer turns 179 degrees, and every other order swings with him. **The
//! world is `z` up**, solved for rather than assumed: the direction that best
//! predicts the file's own accelerometer over 4326 steady frames of unit B is
//! `(-0.002, -0.001, -1.000)`.
//!
//! **The conjugate is the measured half.** With the lock off, the view rides
//! the body, so the camera yaw that makes a later frame show what an earlier
//! one showed IS the body's turn, in the renderer's own sign. Searched on the
//! picture over three half-second pairs of the owner's capture it comes to
//! -16, -22 and -10 degrees where the file's own yaw changed by +15.7, +22.0
//! and +8.6: the same turn to about a degree, the opposite way round. So what
//! the file writes is `body_from_world` where Kjerag wants its inverse, and
//! reading it as written turns the picture twice as far as the wearer.
//! `docs/ROADMAP.md` (2026-08-08) has the whole candidate table and what each
//! one scored.
//!
//! **The heading is what this holds, and the tilt is what it gets wrong.**
//! The paragraphs above are about the heading and they stand: the wearer
//! turns and the world stays. What they do not cover is the camera leaning,
//! and the owner's next report was exactly that - "when the camera dips the
//! horizon is no longer locked". It does not. Measured on the picture through
//! the delivered path, on the dip his own view line sits in (138.40 to 140.51
//! s of the 8k30p capture, the camera 8 to 15.9 degrees off vertical), **the
//! lock leaves 21.9 degrees of tilt where switching it off leaves 11.9**. It
//! is 1.84 times the camera's lean at the median, correlated 0.99 with that
//! lean and 0.10 with the rate the body is turning, so it is a standing error
//! in the frame and not a lag in the lookup.
//!
//! **The left-handed reading is not what is wrong.** It was the open question
//! here and it is answered: a mirror in x leaves 16.8 degrees on that dip and
//! a mirror in y 16.4, both of them also worse than no lock at all. All eight
//! sign readings of the four components were scored and the best of them
//! leaves 8.5, which is not a held horizon either. What the file itself says
//! is consistent: low pass field 3.2.10 over 2 s, which takes the wearer's
//! stride out of it, and the quaternion read as written predicts that
//! accelerometer to about 2 degrees on all three unit B files. The picture
//! disagrees with both by 21 degrees at the dip, and the lean MAGNITUDE it
//! measures matches the file's own to 0.18 degrees - so what is misplaced is
//! the direction that lean points round the camera's own vertical. That is a
//! composition between the file's inertial frame and the optical frame
//! [`BODY`] lands in, not a handedness in the quaternion.
//!
//! **That rotation is measured, and it is [`MOUNTING`]: a mirror in `y` and a
//! quarter turn.** The same instrument, run over the whole corpus instead of
//! one dip, states one turn per instant - it is the azimuth between two
//! vectors, not a fit - and the SCATTER of those over instants that lean in
//! different directions is what separates the families. Over 23 instants of
//! the three unit B files and **177 degrees of lean azimuth**, only one of the
//! four families leaves a constant behind: a mirror in `y` turned `+86.8`
//! degrees, scatter 3.3 rms, against 61 to 66 rms for each of the other three.
//! It is the same constant on each file alone (`+88.3`, `+84.5`, `+85.7`) and
//! dropping any whole file moves it by at most 1.8 degrees. Through the
//! owner's own dip that leaves **0.4 degrees of residual tilt where the
//! shipped reading leaves 20.9 and no lock at all leaves 11.4**, and the app's
//! own locked picture, read at the peak of the dip, comes out level to 0 to 2
//! degrees. The owner tested that build and passed it: *"OSV video output
//! looks good, approved"* (2026-08-08).
//!
//! **What it means is that the file's inertial frame is left handed against
//! the optical one**, which is also why the heading looked settled while the
//! tilt was not: a mirror reverses the heading exactly as a conjugate does, so
//! the turn measurement that pinned the conjugate could not tell the two
//! apart, and it picked the one that gets the tilt wrong. The file's own two
//! streams agree with each other under the mirror as they did before, because
//! the accelerometer is written in that same left-handed frame - and what
//! [`Plumb`] can see of that is the conjugation half, which is why the shipped
//! reading misses the file's own gravity by 2 degrees on leaned frames where
//! the conjugate reading of the same quaternion misses it by 9.3 against a
//! null of 9.4. The mirror half it cannot see at all: [`Plumb`] has the proof
//! and the limitation written out.
//!
//! **It is a constant of the model and not a knob.** The turn was staged
//! behind a `KJERAG_MOUNT` environment variable while the eye was deciding; it
//! is baked now, because a setting that moves the horizon is the calibration
//! ritual zero-config playback forbids, and because there is something better
//! than an escape hatch to put in its place: the file's own accelerometer is a
//! second, independent statement of where down is, and [`Plumb`] holds it
//! against the file's own quaternion **per file** and refuses the lock when
//! the two contradict each other. That is a gate on the FILE, not on this
//! constant, and the distinction is not pedantry: it is what tells a reader
//! which instrument to re-run for a camera this corpus has never seen. The
//! other three candidates are not in the code any more; the table above and
//! `docs/ROADMAP.md` (2026-08-08) are their record, and re-staging them is a
//! patch, not a feature that has to ship for ever in case.
//!
//! **Why the oracles that pinned this preferred the shipped reading**, so the
//! next pass does not reuse them: neither measured a distance from level. The
//! tilt head to head scored the angle BETWEEN two locked renders, which two
//! readings that are wrong the same way both do well on. The accelerometer's
//! 1.8 degrees was a median over frames whose lean is 4 degrees, and every
//! sign reading predicts the same lean magnitude - `1 - 2(x^2 + y^2)` carries
//! no sign - so they differ only in proportion to the lean and at 4 degrees
//! they all score within a degree of each other and of the null. The
//! instrument that does separate them is the vertical VANISHING POINT of a
//! lock-off render, which measures the world's vertical in the camera body
//! from the picture alone: eight view directions agree to 0.37 degrees, and
//! with the lock off it reads 1.02 times the lean the file states.
//! `docs/ROADMAP.md` (2026-08-08) has the whole table and the controls.
//!
//! **Where the world frame's zero heading is:** the first frame's, the same
//! convention `super::orientation` uses on an `.insv`. A gyroscope's absolute
//! heading names nothing, so opening a file at `yaw 0` has to mean looking
//! where the camera looked.
//!
//! No filter runs over any of it. There is no gyroscope and no accelerometer
//! to mix, so [`CalibrationSet::imu`] stays empty and the answer lands in
//! [`CalibrationSet::fused`], which [`CalibrationSet::orientation`] hands back
//! whatever filter it is asked for. A capture whose telemetry carries no
//! orientation at all still comes out empty, and horizon lock is still the
//! no-op an empty track makes it.
//!
//! **What is still unread, and named so it can be asked about later.** An
//! entry writes nothing at fields 9, 16 to 19 or 26, and these at the rest:
//!
//! - **20 and 27**, a pair of `f32`s each, and the two fields hold the *same*
//!   two numbers as each other in every entry of both units. They are per unit
//!   and small, -0.00055 to +0.00064, which is the size a normalized-plane
//!   decentring term or a radian-scale mounting residual would be. Nothing
//!   here can tell those apart, so neither is applied.
//! - **24**, `8.0` on all four entries of both units, and **25**, `-1000.0` on
//!   both of unit B's entries and absent from unit A's. A constant and a
//!   sentinel; neither moves with anything measured.
//! - **22 and 23**, the fourteen-point polyline, which is the model constant
//!   the paragraphs above are about. It is read by nothing and would be worth
//!   applying as a mask: the picture inside it, at the bottom of each lens, is
//!   the camera's own body, which the pass currently draws into the sphere.
//! - **The ladder**, still. What selects a rung is not known, and the risk is
//!   still a third of a percent of scale.
//!
//! Each of those is a field this reader walks past on purpose, not one it
//! failed to see. Field 15 was in that list until 2026-08-08 and it was worth
//! 15 degrees of seam, so the list is written down rather than left implicit.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::calibration::{
    CalibrationSet, Distortion, GyroConfig, GyroEncoding, Intrinsics, Lens, Model, Pose, Size,
};
use super::format::{Boxes, moov};
use super::orientation::{OrientationSample, OrientationTrack};
use super::rotation::Mat3;
use super::rotation::Quat;
use super::rotation::{dot, norm};
use super::{Error, GyroTrack};

/// How many lens entries this reads, which is one per video stream.
const LENSES: usize = 2;

/// A direction in Kjerag's camera body, expressed in the one the file's
/// quaternions are written against.
///
/// **Measured off the file rather than assumed** (`scratch/osmo/`). Carried
/// through each lens's own quaternion, the file's `+z` lands at `(0, -1, 0)`
/// in both lenses' image frames, which is straight up the picture, so `+z` is
/// the camera's vertical. The two optical axes land on `+y` and `-y` to
/// within a hundredth, and the entry whose recorded yaw is about zero is the
/// one on `+y`, so `+y` is where the camera calls forward. `+x` is what a
/// right-handed frame has left over.
///
/// Kjerag's is `x` right, `y` down, `z` forward (`kjerag_render::projection`),
/// so this sends `z` to the file's `y`, `y` to its `-z`, and `x` to its `x`.
/// It is a rotation, which `the_body_frames_differ_by_a_rotation` checks: a
/// reflection here would render the sphere inside out.
const BODY: Mat3 = Mat3::new([[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]]);

/// The same change of basis as a quaternion, which is what the per-frame
/// orientation is composed with. `the_body_quaternion_is_the_body_matrix`
/// holds the two together.
const BODY_QUAT: Quat = Quat {
    w: std::f64::consts::FRAC_1_SQRT_2,
    v: [-std::f64::consts::FRAC_1_SQRT_2, 0.0, 0.0],
};

/// The calibration of the capture at `path`, and the orientation it recorded.
pub(crate) fn read(path: &Path) -> Result<CalibrationSet, Error> {
    let mut file = File::open(path)?;
    let len = file.seek(SeekFrom::End(0))?;
    let moov = moov(&mut file, len)?.ok_or(Error::NoTelemetry)?;
    let track = telemetry_track(&moov).ok_or(Error::NoTelemetry)?;
    let zero = *track.at.first().ok_or(Error::NoTelemetry)?;
    let mut set = from_record(&sample(&mut file, zero)?)?;
    set.fused = orientation(&mut file, &track);
    Ok(set)
}

/// Where one sample sits in the file.
#[derive(Clone, Copy)]
struct At {
    offset: u64,
    size: usize,
}

/// The whole sample table of the first `djmd` track.
///
/// The first, not any: an `.OSV` writes two, and the second one's samples are
/// a sixth the size and carry neither the calibration nor an orientation.
struct Track {
    at: Vec<At>,
    /// When each sample plays, in microseconds from the file's first frame,
    /// which is the clock `kjerag_render` looks an orientation up on.
    offset_us: Vec<i64>,
}

fn telemetry_track(moov: &[u8]) -> Option<Track> {
    Boxes::new(moov)
        .filter(|(kind, _)| *kind == b"trak")
        .filter_map(|(_, trak)| track(trak))
        .next()
}

/// One track's samples, if that track is a `djmd` one.
fn track(trak: &[u8]) -> Option<Track> {
    let mdia = child(trak, b"mdia")?;
    let stbl = child(child(mdia, b"minf")?, b"stbl")?;
    // `stsd` is a full box: four bytes of version and flags, four of entry
    // count, then the entries, each of which opens with its own size and 4cc.
    if child(stbl, b"stsd")?.get(12..16)? != b"djmd" {
        return None;
    }
    let at = places(stbl)?;
    let offset_us = times(stbl, timescale(child(mdia, b"mdhd")?)?, at.len());
    Some(Track { at, offset_us })
}

/// The media timescale, in ticks a second. `mdhd`'s two versions differ by
/// the width of the two times before it.
fn timescale(mdhd: &[u8]) -> Option<u32> {
    match mdhd.first()? {
        0 => be32(mdhd, 12),
        _ => be32(mdhd, 20),
    }
}

/// Where every sample of one track sits, from the three tables that say so.
fn places(stbl: &[u8]) -> Option<Vec<At>> {
    // `stsz` is version and flags, one size for every sample or zero, the
    // sample count, and then the table.
    let stsz = child(stbl, b"stsz")?;
    let uniform = be32(stsz, 4)?;
    let count = be32(stsz, 8)? as usize;
    // `stco` is version and flags, the chunk count, then the offsets. `co64`
    // is the same with 64-bit ones, which a file over 4 GB needs.
    let chunks: Vec<u64> = match child(stbl, b"stco") {
        Some(stco) => (0..be32(stco, 4)? as usize)
            .map(|index| be32(stco, 8 + 4 * index).map(u64::from))
            .collect::<Option<_>>()?,
        None => {
            let co64 = child(stbl, b"co64")?;
            (0..be32(co64, 4)? as usize)
                .map(|index| be64(co64, 8 + 8 * index))
                .collect::<Option<_>>()?
        }
    };
    // `stsc` says how many samples a chunk holds, as runs: an entry opens a
    // run at its own chunk, numbered from one, and the run lasts until the
    // next entry opens one. The entries are in chunk order, so writing each
    // run over the tail of the table leaves every chunk on its own run.
    let stsc = child(stbl, b"stsc")?;
    let mut held = vec![0usize; chunks.len()];
    for run in 0..be32(stsc, 4)? as usize {
        let first = be32(stsc, 8 + 12 * run)? as usize;
        let each = be32(stsc, 12 + 12 * run)? as usize;
        for slot in held.iter_mut().skip(first.saturating_sub(1)) {
            *slot = each;
        }
    }
    let mut out = Vec::with_capacity(count);
    for (chunk, each) in chunks.iter().zip(&held) {
        let mut offset = *chunk;
        for _ in 0..*each {
            if out.len() >= count {
                return Some(out);
            }
            let size = match uniform {
                0 => be32(stsz, 12 + 4 * out.len())?,
                every => every,
            } as usize;
            out.push(At { offset, size });
            offset += size as u64;
        }
    }
    Some(out)
}

/// When every sample plays, in microseconds from the first frame.
///
/// `stts` is a run-length table of per-sample durations. A `djmd` track
/// writes one run of one sample per video frame, and the edit list on every
/// file in the corpus shifts neither track, so a sample's media time is the
/// matching video frame's.
fn times(stbl: &[u8], timescale: u32, count: usize) -> Vec<i64> {
    let mut out = Vec::with_capacity(count);
    let (Some(stts), true) = (child(stbl, b"stts"), timescale > 0) else {
        return out;
    };
    let mut ticks = 0u64;
    for run in 0..be32(stts, 4).unwrap_or(0) as usize {
        let (Some(each), Some(delta)) = (be32(stts, 8 + 8 * run), be32(stts, 12 + 8 * run)) else {
            break;
        };
        for _ in 0..each {
            if out.len() >= count {
                return out;
            }
            out.push((ticks * 1_000_000 / u64::from(timescale)) as i64);
            ticks += u64::from(delta);
        }
    }
    out
}

fn sample(file: &mut File, at: At) -> Result<Vec<u8>, Error> {
    let mut record = vec![0u8; at.size];
    file.seek(SeekFrom::Start(at.offset))?;
    file.read_exact(&mut record)?;
    Ok(record)
}

/// Where the camera says it was pointing, one orientation a frame.
///
/// Reading all of them costs one seek and about a kilobyte a frame: measured
/// on the owner's 146 s capture, 4384 samples and 4.1 MB, 26 ms, which is
/// about a third of a second for a half-hour one.
///
/// A sample that cannot be read or that carries no orientation is skipped
/// rather than fatal, and a file where that is every sample comes out empty,
/// which is the refusal an `.OSV` had until 2026-08-08 and still has if the
/// stream is not there.
fn orientation(file: &mut File, track: &Track) -> OrientationTrack {
    let mut samples = Vec::with_capacity(track.at.len());
    let mut plumb = Vec::new();
    for (at, offset_us) in track.at.iter().zip(&track.offset_us) {
        let Ok(record) = sample(file, *at) else {
            continue;
        };
        let Some(world_from_body) = pointing(&record) else {
            continue;
        };
        if let (Some(raw), Some(measured)) = (raw_pointing(&record), accelerometer(&record)) {
            plumb.push((MOUNTING.read(raw).normalized(), measured));
        }
        samples.push(OrientationSample {
            offset_us: *offset_us,
            world_from_body,
        });
    }
    // The gate. Every file is asked whether its own accelerometer agrees with
    // its own quaternion, and a file that says no comes out with no
    // orientation at all - which is the same empty track a file with no
    // inertial record at all comes out with, and reaches the pilot as the same
    // disabled menu item and the same `level:` line
    // (`kjerag_render::scene`). One refusal, one shape, whatever the reason.
    // It is a check on the file and not on `MOUNTING`; `Plumb` says why, and
    // what would have to be re-run to check a mounting.
    let verdict = Plumb::read(&MOUNTING, &leaned(&plumb, &track.offset_us));
    verdict.say();
    if !verdict.held() {
        return OrientationTrack::default();
    }
    from_first_heading(samples)
}

/// Each frame's lean, its low-passed accelerometer and its reading, which is
/// what [`Plumb::read`] scores.
fn leaned(plumb: &[(Quat, [f64; 3])], offset_us: &[i64]) -> Vec<(f64, [f64; 3], Quat)> {
    let span = offset_us.last().unwrap_or(&0) - offset_us.first().unwrap_or(&0);
    let rate_hz = match span > 0 {
        true => plumb.len() as f64 / (span as f64 / 1e6),
        false => 30.0,
    };
    let raw: Vec<[f64; 3]> = plumb.iter().map(|(_, a)| *a).collect();
    low_passed(&raw, rate_hz, PLUMB_SECS)
        .into_iter()
        .zip(plumb)
        .map(|(measured, (written, _))| {
            // Every reading puts the same number in the third component of the
            // body's own up, so the lean magnitude is one fact about the file.
            let up = written.conjugate().rotate([0.0, 0.0, 1.0]);
            (
                up[2].clamp(-1.0, 1.0).acos().to_degrees(),
                measured,
                *written,
            )
        })
        .collect()
}

/// How long the accelerometer is averaged over, in seconds: long enough that a
/// stride cancels, short enough that a real lean survives. Measured: the
/// instrument sharpens as the window grows and settles by 1 s.
const PLUMB_SECS: f64 = 2.0;

/// The same orientations with the first one's heading taken out, which is
/// where the world frame's zero goes.
///
/// The same convention the `.insv` path's own zero is (`super::orientation`):
/// a gyroscope's absolute heading names nothing, so opening a file at `yaw 0`
/// has to mean looking where the camera looked. Left multiplication is the
/// world side, so this moves the datum and leaves every tilt where it was.
fn from_first_heading(mut samples: Vec<OrientationSample>) -> OrientationTrack {
    let Some(first) = samples.first() else {
        return OrientationTrack::default();
    };
    let zero = Quat::about_down(first.world_from_body.heading()).conjugate();
    for sample in &mut samples {
        sample.world_from_body = zero.times(sample.world_from_body);
    }
    OrientationTrack::from_samples(samples)
}

/// One sample's orientation, in Kjerag's frames.
///
/// Two steps, and the module doc has the evidence for both.
///
/// - **The quaternion is conjugated**, because what the file writes takes a
///   direction from the world to the body and Kjerag's `world_from_body`
///   takes one the other way. Read without it, the picture turns twice as far
///   as the wearer instead of standing still.
/// - **Then the change of basis, on both sides.** The file's world is `z` up
///   and Kjerag's is `y` down, and [`BODY`] is the rotation between the two
///   bodies, so it goes on the right where a Kjerag body vector arrives and
///   its inverse on the left where the answer comes back.
fn pointing(record: &[u8]) -> Option<Quat> {
    let written = MOUNTING.read(raw_pointing(record)?).normalized();
    Some(
        BODY_QUAT
            .conjugate()
            .times(written)
            .times(BODY_QUAT)
            .times(MOUNTING.turn()),
    )
}

/// The four `f32`s of one frame's orientation, in the order they are written.
fn raw_pointing(record: &[u8]) -> Option<Quat> {
    let state = message(message(record, field::FRAME)?, field::STATE)?;
    let quaternion = message(state, field::POINTING)?;
    Some(Quat {
        w: f32s(quaternion, 1)?,
        v: [
            f32s(quaternion, 2)?,
            f32s(quaternion, 3)?,
            f32s(quaternion, 4)?,
        ],
    })
}

/// One frame's accelerometer, in g, in the file's own inertial axes.
///
/// The plumb line the mounting is checked against, and read by nothing else.
fn accelerometer(record: &[u8]) -> Option<[f64; 3]> {
    let state = message(message(record, field::FRAME)?, field::STATE)?;
    let block = message(state, field::ACCELEROMETER)?;
    Some([f32s(block, 2)?, f32s(block, 3)?, f32s(block, 4)?])
}

/// The mounting: which reading of the four components the file's inertial
/// frame is, and how far round the camera's own vertical the optical frame
/// sits from it.
///
/// **The two halves are one model and not two knobs.** A reading and a turn
/// compose as `BODY^-1 . reading(q) . BODY . Rot(up, turn)`, and negating `x`
/// and `y` together is conjugation by a half turn about the file's `z`, which
/// splits into a world-side yaw - removed by [`from_first_heading`] - and a
/// body-side half turn, which IS a mounting of 180 degrees. So the eight sign
/// readings of the last session are four families sampled at 0 and 180 only,
/// which is why none of its eight rows held the horizon: the answer is at 87.
struct Mounting {
    /// The signs the reading puts on `(x, y, z)`. `(-1, -1, -1)` is the
    /// conjugate this file shipped with until 2026-08-08.
    reading: [f64; 3],
    /// The turn about Kjerag's own up axis, in degrees, applied on the body
    /// side after the change of basis.
    turn_deg: f64,
    /// The signs the same reading puts on the accelerometer, which is a vector
    /// in the same inertial frame and has to be carried through the same
    /// reflection for [`Plumb`] to be comparing two of one thing.
    plumb: [f64; 3],
}

impl Mounting {
    fn read(&self, raw: Quat) -> Quat {
        Quat {
            w: raw.w,
            v: std::array::from_fn(|i| raw.v[i] * self.reading[i]),
        }
    }

    /// The turn as a quaternion. Kjerag's world vertical is `-y`, and
    /// [`Quat::about_down`] turns about `+y`, so the angle goes in negated.
    fn turn(&self) -> Quat {
        Quat::about_down(-self.turn_deg.to_radians())
    }
}

/// **The mounting an Osmo 360 is read with**, derived from the picture over 23
/// instants of the three unit B files.
///
/// **The picture is the only instrument that has ever verified this**, and it
/// is an offline one: the derivation below is where the mounting was settled
/// and it is where a second one would have to be settled too. [`Plumb`], which
/// runs at open on every file, is NOT that instrument and cannot be: the
/// reflection and the turn cancel out of it, and its own doc says so with the
/// proof.
///
/// The instrument is the vertical vanishing point of a **lock off** render,
/// which measures where the world's up sits in Kjerag's camera body from the
/// picture alone and is identical whatever candidate is under test. Each
/// instant states one turn on its own - it is the azimuth between two vectors,
/// not a fit - and the SCATTER of those over instants that lean in different
/// directions is what tells the families apart, because only the right family
/// leaves a constant of the hardware behind. Pooled over unit B, weighted by
/// each instant's own lean because an azimuth of a nearly upright vector is
/// nearly undefined, over 177 degrees of lean azimuth:
///
/// ```text
/// family                  turn      rms scatter    worst    walks with
/// as written             -84.6           65.9      148.8    heading 0.69
/// conjugate (pre-08-08) -107.1           62.1      133.2    azimuth 0.73
/// mirror in y            +86.8            3.3       10.1    nothing 0.15
/// mirror x, conjugated   -90.9           61.2      170.8    azimuth 0.89
/// ```
///
/// **One of the four is a constant and the other three are not.** It is a
/// constant on each file separately as well as pooled - `+88.3` on B003 over
/// 11 instants, `+84.5` on B002 over 7, `+85.7` on B001 over 5 - and dropping
/// any whole file moves it by at most 1.8 degrees (`+85.0`, `+87.6`, `+87.1`).
/// What the others walk with says why they are wrong: the conjugate families
/// track the body's own heading and the as-written one tracks the direction it
/// leans, which is the signature of a reflection read as a rotation.
///
/// **So the file's inertial frame is left handed against the optical one**, and
/// that is why the heading looked settled while the tilt was not: a mirror
/// reverses the heading exactly as a conjugate does, so the turn measurement of
/// 88d9f3c could not tell them apart and picked the one that got the tilt
/// wrong. What the accelerometer adds from the other side is the half of that
/// pair it can see - the conjugation and not the mirror - and that is
/// [`Plumb`].
///
/// **Why the quarter turn and not the `+86.8` that was measured.** The two are
/// 3.2 degrees apart, which is inside the measurement's own 3.3 rms scatter, so
/// the corpus cannot separate them; a right angle is what a screw can hold and
/// `+86.8` is what an estimator returns, and it puts the mirror plane on the 45
/// degree diagonal between the two lenses. The eye ruled on this arm and not on
/// the other: `+90.0` is the build the owner tested and approved.
///
/// **Everything about it is one hardware constant**, so it is `const` and not
/// configuration. The three refuted families are gone from the code; the table
/// above is their record.
///
/// **What is derived from unit B is a guess about a third unit, and nothing at
/// open would catch it.** The evidence above is three files of one camera. If
/// some other Osmo 360 writes its inertial frame through a different
/// reflection, that capture renders a visibly tilted horizon and [`Plumb`]
/// prints the line it prints for a file that agrees, because a reflection
/// moves the quaternion and the accelerometer together. Re-deriving this
/// constant means re-running the vanishing-point instrument on that unit's own
/// film; there is no shortcut through the file's gravity. Recorded here rather
/// than left for the next reader to find out from a picture.
const MOUNTING: Mounting = Mounting {
    reading: [-1.0, 1.0, -1.0],
    turn_deg: 90.0,
    plumb: [1.0, -1.0, 1.0],
};

/// **Whether one file's own two records of where down is agree with each
/// other**, which is what decides whether this capture's horizon is held.
///
/// **The gate, and what it is a gate on.** The file writes a fused quaternion
/// and, beside it, an accelerometer. Those are two independent statements of
/// the same thing, so they can be held against each other per file, and a
/// capture whose own two records contradict each other does not get its
/// horizon held. That is the house pattern: verify what can be verified, or
/// refuse gracefully and say why.
///
/// **It is NOT a check on [`MOUNTING`], and this is the correction of
/// 2026-08-09.** The line it printed used to say the mounting was "confirmed",
/// and it cannot say that. Both streams are read out of the file's own
/// inertial frame and both are carried through the mounting on the way here -
/// the quaternion through [`Mounting::read`], the accelerometer through
/// [`Mounting::plumb`] - so anything the mounting does to one it does to the
/// other, and the ANGLE BETWEEN THEM does not move. Concretely, for a mounting
/// whose reading puts signs `s` on the quaternion's vector part, the
/// accelerometer's signs are `plumb = s * s2` and NOT `s` itself - the
/// shipped constant says so, `reading = [-1, 1, -1]` against
/// `plumb = [1, -1, 1]` - because the `S z = s2 z` factor rides on both
/// sides:
///
/// ```text
/// S = diag(s)      S z = s2 z      predicted = R_s^T z = s2 . S R^T z
/// up = -diag(s * s2) a / |a|  =  -s2 . S a / |a|
/// angle(predicted, up) = angle(S R^T z, -S a/|a|) = angle(R^T z, -a/|a|)
///                                        // s2 cancels, then S drops out
/// ```
///
/// So of the eight sign families, all four whose `S` is a rotation score
/// **identically**, and the four whose `S` is a reflection score identically
/// to each other; the turn cancels the same way, because it rotates both. The
/// corpus table in `docs/research/osv-format.md` has two columns and not four
/// for exactly this reason. `the_check_scores_a_mirrored_mounting_identically`
/// is the proof in code, and it is a proof about arithmetic rather than a
/// measurement about this corpus.
///
/// **So what does it see?** Two real things:
///
/// - **A file whose own two records disagree**, which is what unit A is:
///   23 degrees between its quaternion and its accelerometer against a 15
///   degree null. That is a fault in the file by the file's own evidence and
///   needs no reference to a mounting at all.
/// - **The conjugation**, which is the one bit of the mounting that does NOT
///   cancel: reading the quaternion forwards rather than inverted transposes
///   `R_s` and moves the angle. That is [`Reading::flipped`], measured beside
///   the miss on every file and gated on, so the printed claim is one the
///   numbers support.
///
/// **And what it is blind to**: the mirror and the turn, which is most of what
/// [`MOUNTING`] is. A capture off a unit whose inertial frame is reflected the
/// other way renders a visibly tilted horizon and passes this gate with the
/// numbers of a file that agrees. Mounting verification lives offline, in the
/// vanishing-point solve over 177 degrees of lean azimuth that derived the
/// constant ([`MOUNTING`], commits 32a37ec and 46a5a99, docs/ROADMAP.md
/// 2026-08-08), and there is no way to move it to open time from gravity
/// alone.
///
/// Read on frames leaning more than [`LEANED_DEG`] because every reading
/// predicts the same lean MAGNITUDE - `1 - 2(x^2 + y^2)` carries no sign - so
/// they differ only in azimuth and only in proportion to the lean. **On an
/// upright camera this instrument says nothing, and it does not need to**: the
/// error it exists to catch is a lean pointed the wrong way, and a file with no
/// lean in it has no such error to show. So [`Plumb::Blind`] holds the lock
/// rather than refusing it.
enum Plumb {
    /// The file's two records agree, and better than either bar, so the lock
    /// is held.
    Agrees(Reading),
    /// Nothing in this file leans far enough for its accelerometer to say
    /// anything about its quaternion. The lock is held: see above.
    Blind,
    /// The file's two records contradict each other, so this capture's
    /// orientation is not read and the lock refuses.
    Disagrees(Reading),
}

/// What one file's plumb check measured, in degrees, at the median over the
/// frames that leaned.
#[derive(Debug)]
struct Reading {
    /// How far the orientation's predicted up sits from the measured one.
    miss: f64,
    /// The same for a camera assumed never to lean, which is what the reading
    /// has to beat to have said anything.
    null: f64,
    /// The same for the file's quaternion read the other way round, which is
    /// the one part of the mounting this can see (see [`Plumb`]). It is the
    /// difference between `R_s` and `R_s^T`, so it costs one more rotate of
    /// one vector and it is the only mounting-sensitive number here.
    flipped: f64,
    /// The measured `|a|`, in g. 1.00 is gravity alone.
    magnitude: f64,
    frames: usize,
}

/// How far the orientation may miss the file's own gravity and still be
/// believed, in degrees.
///
/// **Measured, 2026-08-09, over the whole seven-file corpus** (the sweep is in
/// the PR body and in `docs/research/osv-format.md` 6.1). The six unit B files
/// come out at 1.6, 2.0, 2.7, 2.9, 2.9 and 3.8 degrees; the one unit A file
/// comes out at 23.0. So the bar is twice the worst file that agrees and a
/// third of the one that does not, and no file in the corpus is anywhere near
/// it.
const PLUMB_CEILING_DEG: f64 = 8.0;

/// Where the plumb check stops being blind, in degrees of lean.
///
/// **It decides how much of the corpus is measured at all**, so it is not a
/// free parameter. Measured 2026-08-09: at 8 the seven files offer 39, 53, 72,
/// 244, 539, 680 and 893 leaned frames; at 10 the thinnest of them offers none
/// and goes [`Plumb::Blind`], and `2 Lens Sharpness Test` falls from 72 frames
/// to 6. Lower would let a nearly upright frame vote, and an azimuth of a
/// nearly upright vector is nearly undefined.
const LEANED_DEG: f64 = 8.0;

impl Plumb {
    /// The check itself: the up the file's own quaternion predicts against the
    /// up its own accelerometer measured, in the file's own inertial axes.
    ///
    /// **Three bars, and each has a job.** The miss must be under
    /// [`PLUMB_CEILING_DEG`], which bounds what a held horizon can be wrong
    /// by. It must beat the NULL of a camera assumed never to lean, which is
    /// what makes this a verification rather than a tolerance: a reading no
    /// better than assuming the camera is upright has told us nothing,
    /// whatever its absolute number. And it must beat the FLIPPED reading of
    /// the same quaternion, which is the only bar here that a mounting can
    /// fail - the other two would pass a mirrored mounting unchanged
    /// ([`Plumb`]).
    ///
    /// On this corpus all three agree on every file, which is why each can be
    /// this simple: the six that pass beat their own nulls by 1.51 to 4.92
    /// times and their own flipped readings by 1.87 to 10.00, and the one that
    /// fails loses to its own null by 1.56 and to its own flipped reading by
    /// 2.03. **The thinnest of those margins is real and worth naming**: `1-
    /// 8k30p stable` passes at 3.75 against a null of 5.68, over 39 leaned
    /// frames of an 83 second capture, and at [`LEANED_DEG`] of 10 rather than
    /// 8 it would have no leaned frames at all and go [`Plumb::Blind`]
    /// (measured). A file barely leans or leans a lot; this corpus has one of
    /// the former in it and the bar sits where it does partly because of that.
    ///
    /// `mount` is a parameter rather than [`MOUNTING`] read directly so that
    /// two mountings can be scored side by side in a test, which is how the
    /// blindness above is proved rather than asserted.
    fn read(mount: &Mounting, plumb: &[(f64, [f64; 3], Quat)]) -> Self {
        let mut errors: Vec<f64> = Vec::new();
        let mut nulls: Vec<f64> = Vec::new();
        let mut flips: Vec<f64> = Vec::new();
        let mut magnitudes: Vec<f64> = Vec::new();
        for (lean, measured, written) in plumb {
            magnitudes.push(norm(*measured));
            if *lean <= LEANED_DEG {
                continue;
            }
            let predicted = written.conjugate().rotate([0.0, 0.0, 1.0]);
            // The same quaternion read the other way round. Conjugating the
            // reading is what tells the two mounting families apart, and it
            // reaches this arithmetic as the transpose of one rotation, so the
            // whole of the other family's answer is this one line.
            let other = written.rotate([0.0, 0.0, 1.0]);
            let mut up = [0.0; 3];
            let length = norm(*measured);
            if length <= 0.0 {
                continue;
            }
            for axis in 0..3 {
                up[axis] = -measured[axis] / length * mount.plumb[axis];
            }
            errors.push(between(predicted, up));
            flips.push(between(other, up));
            nulls.push(between([0.0, 0.0, 1.0], up));
        }
        if errors.is_empty() {
            return Self::Blind;
        }
        let reading = Reading {
            frames: errors.len(),
            miss: median(&mut errors),
            null: median(&mut nulls),
            flipped: median(&mut flips),
            magnitude: median(&mut magnitudes),
        };
        let held = reading.miss < PLUMB_CEILING_DEG
            && reading.miss < reading.null
            && reading.miss < reading.flipped;
        match held {
            true => Self::Agrees(reading),
            false => Self::Disagrees(reading),
        }
    }

    /// The one line this prints per file, at open, whichever way it went.
    ///
    /// It is said out loud in every case and not only the refusal, because the
    /// number is the evidence that the horizon being held IS held on
    /// something, and a pilot comparing two captures wants to see the same
    /// line twice.
    ///
    /// **It says what it measured and not what it would like to have
    /// measured.** It used to end "so it is confirmed", of the mounting, and
    /// the mounting is the thing it cannot see ([`Plumb`]). What is claimed
    /// here is agreement between the file's own two records, with all three
    /// numbers the verdict rests on beside it, so a reader can check the
    /// verdict rather than take it.
    fn say(&self) {
        match self {
            Self::Blind => eprintln!(
                "osmo:   plumb check: no frame leans more than {LEANED_DEG:.0} degrees, so this \
                 file's own gravity cannot say anything about its own quaternion here and does \
                 not need to; the horizon is held"
            ),
            Self::Agrees(reading) => eprintln!(
                "osmo:   plumb check: this file's orientation misses its own gravity by {:.1} \
                 degrees at the median over {} leaned frames, against {:.1} for a camera assumed \
                 upright and {:.1} for the quaternion read the other way round, so the file agrees \
                 with itself and the horizon is held. |a| is {:.2} g at the median (1.00 is \
                 gravity alone). This is a check on the file, not on the mounting, which no \
                 reading of gravity can see",
                reading.miss, reading.frames, reading.null, reading.flipped, reading.magnitude,
            ),
            Self::Disagrees(reading) => eprintln!(
                "osmo:   plumb check: this file's orientation misses its own gravity by {:.1} \
                 degrees at the median over {} leaned frames, against {:.1} for a camera assumed \
                 upright and {:.1} for the quaternion read the other way round, so this capture's \
                 own two records contradict each other and Kjerag will not hold a horizon on a \
                 record it cannot believe. |a| is {:.2} g at the median (1.00 is gravity alone). \
                 The picture is unaffected and the view is yours to pan",
                reading.miss, reading.frames, reading.null, reading.flipped, reading.magnitude,
            ),
        }
    }

    /// Whether the orientation this file recorded may be used.
    fn held(&self) -> bool {
        !matches!(self, Self::Disagrees(_))
    }
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values[values.len() / 2]
}

fn between(a: [f64; 3], b: [f64; 3]) -> f64 {
    let cosine = dot(a, b) / (norm(a) * norm(b));
    cosine.clamp(-1.0, 1.0).acos().to_degrees()
}

/// A centred box filter over `secs` of samples, which is what takes the
/// wearer's stride out of the accelerometer.
///
/// A worn camera's accelerometer is gravity plus the wearer's stride, and on
/// this corpus the stride is the bigger of the two at frame rate. A stride
/// averages to zero over a step and gravity does not.
fn low_passed(raw: &[[f64; 3]], rate_hz: f64, secs: f64) -> Vec<[f64; 3]> {
    let half = ((rate_hz * secs * 0.5).round() as usize).max(1);
    (0..raw.len())
        .map(|at| {
            let from = at.saturating_sub(half);
            let upto = (at + half + 1).min(raw.len());
            let mut sum = [0.0; 3];
            for one in &raw[from..upto] {
                for axis in 0..3 {
                    sum[axis] += one[axis];
                }
            }
            sum.map(|c| c / (upto - from) as f64)
        })
        .collect()
}

fn child<'a>(body: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    Boxes::new(body)
        .find(|(found, _)| *found == kind)
        .map(|(_, payload)| payload)
}

fn be32(body: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(body.get(at..at + 4)?.try_into().ok()?))
}

fn be64(body: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(body.get(at..at + 8)?.try_into().ok()?))
}

/// The field numbers this reader looks for, which are the whole of the schema
/// it knows. Everything else in the record is walked past.
mod field {
    /// Top level.
    pub const HEADER: u32 = 1;
    pub const VIDEO: u32 = 2;

    /// Inside `HEADER`.
    pub const CAMERA: u32 = 1;
    /// Inside `HEADER.CAMERA`. The serial number is field 5 and is
    /// deliberately not read: it names the pilot's unit and nothing here
    /// needs it (`CalibrationSet::camera_key`).
    pub const FIRMWARE: u32 = 6;
    pub const MODEL: u32 = 10;

    /// Inside `VIDEO`.
    pub const PICTURE: u32 = 3;
    pub const LENSES: u32 = 6;
    /// Inside `VIDEO.PICTURE`.
    pub const WIDTH: u32 = 1;
    pub const HEIGHT: u32 = 2;

    /// Inside one lens entry of `VIDEO.LENSES`.
    pub const FX: u32 = 1;
    pub const FY: u32 = 2;
    pub const CX: u32 = 3;
    pub const CY: u32 = 4;
    /// The five coefficients of the lens's theta polynomial, in order. **Four
    /// of them are adjacent and the fifth is not**: 5 to 8 sit together where
    /// a reader expects them, and the one that makes the model a 199 degree
    /// lens instead of a hemisphere is field 15, seven fields later, past the
    /// yaw, pitch and roll triple. The module's own doc has what reading them
    /// four-at-a-time cost.
    pub const K: [u32; 5] = [5, 6, 7, 8, 15];
    /// The frame the pixel numbers above are expressed in, which is the
    /// delivered frame: 3840 by 3840 on every file in the corpus.
    pub const CANVAS_WIDTH: u32 = 10;
    pub const CANVAS_HEIGHT: u32 = 11;
    /// Where the lens points, as a unit quaternion in a submessage of four
    /// `f32`s. There is a second copy of the same four numbers packed into
    /// field 21; this one is read because a submessage of named fields cannot
    /// be misread as a different length.
    pub const ORIENTATION: u32 = 28;

    /// Top level, and in **every** sample rather than only the first: what
    /// the camera was doing while this frame was taken.
    pub const FRAME: u32 = 3;
    /// Inside `FRAME`. Its own field 1 is a frame counter and a device clock,
    /// and its field 3 is the kilohertz stream the module doc is about.
    pub const STATE: u32 = 2;
    /// Inside `FRAME.STATE`: this frame's orientation, `w` first, in the same
    /// four-`f32` submessage shape as [`ORIENTATION`] above. Its neighbour at
    /// field 10 is the accelerometer in g, three `f32`s at fields 2 to 4,
    /// which is the plumb line the frame convention was pinned against.
    pub const POINTING: u32 = 9;
    /// The accelerometer beside it, in g, three `f32`s at fields 2 to 4. Read
    /// only by the mounting's own self-check ([`super::Plumb`]), which is the
    /// one thing in the file that can say a reading is the wrong family
    /// without a picture to look at, and which the horizon lock is gated on.
    pub const ACCELEROMETER: u32 = 10;
}

fn from_record(record: &[u8]) -> Result<CalibrationSet, Error> {
    let header = message(record, field::HEADER).ok_or(Error::TelemetryField("header"))?;
    let camera = message(header, field::CAMERA).ok_or(Error::TelemetryField("camera"))?;
    let video = message(record, field::VIDEO).ok_or(Error::TelemetryField("video"))?;
    let picture = message(video, field::PICTURE).ok_or(Error::TelemetryField("picture"))?;
    let table = message(video, field::LENSES).ok_or(Error::TelemetryField("lens table"))?;

    let dimension = Size {
        width: varint(picture, field::WIDTH).ok_or(Error::TelemetryField("width"))? as u32,
        height: varint(picture, field::HEIGHT).ok_or(Error::TelemetryField("height"))? as u32,
    };
    if dimension.width == 0 || dimension.height == 0 {
        return Err(Error::DegenerateCanvas);
    }

    let lenses = (0..LENSES)
        .map(|index| {
            // The entries are numbered from 1, which is what makes the first
            // pair fields 1 and 2.
            let entry = message(table, index as u32 + 1).ok_or(Error::TelemetryField("lens"))?;
            lens(entry, dimension)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(CalibrationSet {
        camera_model: text(camera, field::MODEL).unwrap_or_default(),
        firmware: text(camera, field::FIRMWARE).unwrap_or_default(),
        dimension,
        lenses,
        // Not in the record, and a readout nobody has measured is one this
        // does not correct for (`kjerag_meta::Sweep`).
        rolling_shutter_ms: 0.0,
        gyro: GyroConfig {
            encoding: GyroEncoding::Scaled,
            imu_orientation: "xZY",
            first_frame_timestamp: 0,
            gyro_timestamp: None,
        },
        exposure: Default::default(),
        // Empty because there is no raw IMU to read: this camera writes the
        // fused answer, which lands in `fused` instead and needs no filter.
        imu: GyroTrack::default(),
        fused: OrientationTrack::default(),
        calibration_canvas: dimension,
    })
}

/// One lens entry, in delivered-frame pixels, which is the frame the file
/// already writes them in.
fn lens(entry: &[u8], dimension: Size) -> Result<Lens, Error> {
    let take = |number, what| f32s(entry, number).ok_or(Error::TelemetryField(what));
    let canvas = Size {
        width: take(field::CANVAS_WIDTH, "lens canvas width")? as u32,
        height: take(field::CANVAS_HEIGHT, "lens canvas height")? as u32,
    };
    // The numbers are already in the frame the streams decode at on every
    // file in the corpus. A file that says otherwise is one this has never
    // seen, and scaling it on a guess would be worse than saying so.
    if canvas != dimension {
        return Err(Error::CanvasMismatch);
    }
    let orientation =
        message(entry, field::ORIENTATION).ok_or(Error::TelemetryField("lens orientation"))?;
    let quaternion = |number, what| f32s(orientation, number).ok_or(Error::TelemetryField(what));
    let pointing = Quat {
        w: quaternion(1, "quaternion w")?,
        v: [
            quaternion(2, "quaternion x")?,
            quaternion(3, "quaternion y")?,
            quaternion(4, "quaternion z")?,
        ],
    }
    .normalized();

    let mut k = [0.0; 5];
    for (index, number) in field::K.into_iter().enumerate() {
        k[index] = take(number, "lens coefficient")?;
    }

    Ok(Lens {
        intrinsics: Intrinsics {
            // No mirror parameter: the model is a theta polynomial and `xi`
            // belongs to the Mei one.
            xi: 0.0,
            fx: take(field::FX, "fx")?,
            fy: take(field::FY, "fy")?,
            cx: take(field::CX, "cx")?,
            cy: take(field::CY, "cy")?,
        },
        // Brown-Conrady on a normalized plane, which is Insta360's model and
        // not this one. This lens's five coefficients are a polynomial in the
        // angle off the axis and travel on [`Model::Theta`] with it.
        distortion: Distortion {
            k1: 0.0,
            k2: 0.0,
            k3: 0.0,
            p1: 0.0,
            p2: 0.0,
        },
        model: Model::Theta { k },
        // The yaw, pitch and roll in fields 12 to 14 describe the same
        // pointing as the quaternion and in the same absolute terms, so
        // carrying both would be carrying one convention twice. The
        // quaternion is the one taken because it needs no rotation order.
        pose: Pose {
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            roll_deg: 0.0,
            // No inter-lens translation is recorded, so there is no baseline
            // and the parallax band has nothing to measure against.
            translation_m: [0.0; 3],
        },
        // `ray_lens = pointing * ray_file`, and Kjerag asks for
        // `ray_lens = mounting * ray_body`, so the change of basis goes on
        // the right where the body vector arrives.
        mounting: Some(pointing.matrix().times(BODY)),
        // DJI writes no equivalent of Insta360's `lensType`.
        lens_type: 0,
    })
}

/// One field of a protobuf message, as it sits on the wire.
enum Wire<'a> {
    Varint(u64),
    Fixed64,
    Bytes(&'a [u8]),
    Fixed32([u8; 4]),
}

/// The fields of one message, in the order they were written.
///
/// A hand-rolled reader rather than a generated one because there is no
/// `.proto` to generate from: DJI names its schema in the record and ships it
/// nowhere. It stops at the first byte it cannot read rather than erroring,
/// which is what makes a truncated or foreign record answer "field absent"
/// instead of taking the open down.
struct Fields<'a> {
    body: &'a [u8],
    at: usize,
}

impl<'a> Iterator for Fields<'a> {
    type Item = (u32, Wire<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        let (key, at) = varint_at(self.body, self.at)?;
        let (number, wire) = ((key >> 3) as u32, key & 7);
        if number == 0 {
            return None;
        }
        let (value, at) = match wire {
            0 => {
                let (value, at) = varint_at(self.body, at)?;
                (Wire::Varint(value), at)
            }
            1 => (
                Wire::Fixed64,
                at.checked_add(8).filter(|e| *e <= self.body.len())?,
            ),
            2 => {
                let (length, at) = varint_at(self.body, at)?;
                let end = at.checked_add(usize::try_from(length).ok()?)?;
                (Wire::Bytes(self.body.get(at..end)?), end)
            }
            5 => {
                let end = at.checked_add(4)?;
                (Wire::Fixed32(self.body.get(at..end)?.try_into().ok()?), end)
            }
            // Groups, which nothing in this record uses and which cannot be
            // skipped without knowing where they end.
            _ => return None,
        };
        self.at = at;
        Some((number, value))
    }
}

/// One varint at `at`, and where it ended.
fn varint_at(body: &[u8], mut at: usize) -> Option<(u64, usize)> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *body.get(at)?;
        at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, at));
        }
    }
    None
}

fn fields(body: &[u8]) -> Fields<'_> {
    Fields { body, at: 0 }
}

/// The last value written for one field, which is what a protobuf reader is
/// required to take when a field is written more than once.
fn last(body: &[u8], number: u32) -> Option<Wire<'_>> {
    fields(body)
        .filter(|(found, _)| *found == number)
        .map(|(_, wire)| wire)
        .last()
}

fn message(body: &[u8], number: u32) -> Option<&[u8]> {
    match last(body, number)? {
        Wire::Bytes(inner) => Some(inner),
        _ => None,
    }
}

fn varint(body: &[u8], number: u32) -> Option<u64> {
    match last(body, number)? {
        Wire::Varint(value) => Some(value),
        _ => None,
    }
}

/// A single-precision float, widened, which is how every number in the
/// calibration is written.
fn f32s(body: &[u8], number: u32) -> Option<f64> {
    match last(body, number)? {
        Wire::Fixed32(raw) => Some(f64::from(f32::from_le_bytes(raw))),
        _ => None,
    }
}

fn text(body: &[u8], number: u32) -> Option<String> {
    match last(body, number)? {
        Wire::Bytes(raw) => Some(String::from_utf8_lossy(raw).into_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `fixed32` field, as the calibration writes every one of its numbers.
    fn f32field(number: u32, value: f32) -> Vec<u8> {
        let mut out = key(number, 5);
        out.extend_from_slice(&value.to_le_bytes());
        out
    }

    fn key(number: u32, wire: u64) -> Vec<u8> {
        let mut value = u64::from(number) << 3 | wire;
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            match value {
                0 => {
                    out.push(byte);
                    return out;
                }
                _ => out.push(byte | 0x80),
            }
        }
    }

    fn submessage(number: u32, body: &[u8]) -> Vec<u8> {
        let mut out = key(number, 2);
        let mut length = body.len() as u64;
        loop {
            let byte = (length & 0x7f) as u8;
            length >>= 7;
            match length {
                0 => {
                    out.push(byte);
                    break;
                }
                _ => out.push(byte | 0x80),
            }
        }
        out.extend_from_slice(body);
        out
    }

    fn varint_field(number: u32, value: u64) -> Vec<u8> {
        let mut out = key(number, 0);
        let mut value = value;
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            match value {
                0 => {
                    out.push(byte);
                    return out;
                }
                _ => out.push(byte | 0x80),
            }
        }
    }

    /// One lens entry with the numbers this reader takes, plus the fields it
    /// walks past, laid out **in the camera's own order**, which is what makes
    /// this fixture worth anything: the fifth coefficient is written where the
    /// camera writes it, at field 15, on the far side of the yaw/pitch/roll
    /// triple and of the four that sit together.
    fn entry(fx: f32, fy: f32, cx: f32, cy: f32, k: [f32; 5], q: [f32; 4]) -> Vec<u8> {
        let mut out = f32field(field::FX, fx);
        out.extend(f32field(field::FY, fy));
        out.extend(f32field(field::CX, cx));
        out.extend(f32field(field::CY, cy));
        for (number, value) in field::K.into_iter().take(4).zip(k) {
            out.extend(f32field(number, value));
        }
        out.extend(f32field(field::CANVAS_WIDTH, 3840.0));
        out.extend(f32field(field::CANVAS_HEIGHT, 3840.0));
        // Yaw, pitch and roll, which say the same thing the quaternion does
        // and are walked past - and which is what field 15 sits behind.
        for (number, value) in [(12, 179.5457), (13, 90.61688), (14, -1.3556259)] {
            out.extend(f32field(number, value));
        }
        out.extend(f32field(field::K[4], k[4]));
        let mut orientation = Vec::new();
        for (number, value) in (1..).zip(q) {
            orientation.extend(f32field(number, value));
        }
        out.extend(submessage(field::ORIENTATION, &orientation));
        out
    }

    /// The real record of `CAM_20250715191201_0003_D.OSV` (unit A), cut down
    /// to the fields this reads and with the serial left out.
    fn record() -> Vec<u8> {
        record_of(
            "10.00.05.06",
            entry(
                1046.3793,
                1046.168,
                1920.8534,
                1916.7302,
                [0.068134, -0.013797, 0.0117944, -0.00733225, 0.00104408],
                [-0.011197756, 0.005501542, -0.7032019, 0.71088076],
            ),
            entry(
                1048.025,
                1047.8745,
                1910.7661,
                1916.2424,
                [0.0644356, -0.00886799, 0.00849704, -0.00639127, 0.000955506],
                [0.70341, 0.710761, -0.000703606, 0.005721177],
            ),
        )
    }

    /// The same for the owner's own camera (unit B, `1 8k30p standard 10bit
    /// iso max 800-003.OSV`), which is a different factory measurement of the
    /// same model: every one of the ten coefficients differs from unit A's.
    fn record_b() -> Vec<u8> {
        record_of(
            "10.00.05.06",
            entry(
                1052.6311,
                1052.5306,
                1914.0237,
                1918.4277,
                [0.0671674, -0.0126124, 0.0101191, -0.00672833, 0.000983484],
                [0.00174622, -0.00134898, -0.707093, 0.707117],
            ),
            entry(
                1044.5287,
                1044.2777,
                1916.2665,
                1929.8512,
                [0.0700723, -0.016647, 0.0146503, -0.00833712, 0.00115667],
                [0.70661, 0.707568, 0.00588298, 0.00387544],
            ),
        )
    }

    fn record_of(firmware: &str, first: Vec<u8>, second: Vec<u8>) -> Vec<u8> {
        let mut camera = submessage(field::FIRMWARE, firmware.as_bytes());
        camera.extend(submessage(field::MODEL, b"Osmo 360"));
        let header = submessage(field::CAMERA, &camera);

        let mut picture = varint_field(field::WIDTH, 3840);
        picture.extend(varint_field(field::HEIGHT, 3840));

        let mut table = submessage(1, &first);
        table.extend(submessage(2, &second));

        let mut video = submessage(field::PICTURE, &picture);
        video.extend(submessage(field::LENSES, &table));

        let mut out = submessage(field::HEADER, &header);
        out.extend(submessage(field::VIDEO, &video));
        out
    }

    /// One per-frame telemetry sample, carrying the orientation this frame was
    /// taken at, in the camera's own nesting: field 3, then 2, then 9.
    fn frame_record(q: [f32; 4]) -> Vec<u8> {
        let mut written = Vec::new();
        for (number, value) in (1..).zip(q) {
            written.extend(f32field(number, value));
        }
        let mut state = f32field(3, 800.0);
        state.extend(submessage(field::POINTING, &written));
        // The accelerometer beside it, which this reads past.
        let mut accel = f32field(2, 0.01);
        accel.extend(f32field(3, -0.02));
        accel.extend(f32field(4, -0.99));
        state.extend(submessage(10, &accel));
        let mut frame = submessage(1, &varint_field(1, 20));
        frame.extend(submessage(field::STATE, &state));
        submessage(field::FRAME, &frame)
    }

    /// The five coefficients of one lens of one record.
    fn coefficients(record: &[u8], lens: usize) -> [f64; 5] {
        match from_record(record).unwrap().lenses[lens].model {
            Model::Theta { k } => k,
            other => panic!("an Osmo lens is a theta polynomial, not {other:?}"),
        }
    }

    /// The same orientation with its heading taken off, which is the frame
    /// every assertion about a mounting is made in.
    ///
    /// [`from_first_heading`] removes the first sample's heading from the whole
    /// track, so a constant yaw - which is exactly what [`MOUNTING`]'s quarter
    /// turn is on an upright camera - is not a thing the composition can be
    /// wrong about. Comparing with it left on would be asserting a datum.
    fn bare(q: Quat) -> Quat {
        Quat::about_down(q.heading()).conjugate().times(q)
    }

    #[track_caller]
    fn near(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} is not within {tolerance} of {expected}"
        );
    }

    /// The headline: the two lenses come out with the numbers the file wrote,
    /// in the frame the streams decode at, needing no scaling on the way.
    #[test]
    fn the_record_describes_a_two_lens_osmo() {
        let calibration = from_record(&record()).unwrap();
        assert_eq!(calibration.camera_model, "Osmo 360");
        assert_eq!(calibration.firmware, "10.00.05.06");
        assert_eq!(
            calibration.dimension,
            Size {
                width: 3840,
                height: 3840
            }
        );
        assert_eq!(calibration.lenses.len(), 2);
        near(calibration.lenses[0].intrinsics.fx, 1046.379, 0.001);
        near(calibration.lenses[0].intrinsics.cx, 1920.853, 0.001);
        near(calibration.lenses[1].intrinsics.fx, 1048.025, 0.001);
        near(calibration.lenses[1].intrinsics.cx, 1910.766, 0.001);
        for lens in &calibration.lenses {
            assert!(matches!(lens.model, Model::Theta { .. }), "{lens:?}");
            near(lens.intrinsics.cy, 1916.5, 0.5);
        }
    }

    /// **The fifth coefficient is field 15, and it is read.** Four of them sit
    /// together at 5 to 8 and the fifth is seven fields later, behind the
    /// yaw/pitch/roll triple; a reader that takes the run of four gets a lens
    /// that folds at 89 degrees, and the whole of the difference is this one
    /// `f32`. Both units, because it is a per-unit measurement and not a
    /// constant: the two cameras write 0.00104408 and 0.00098348 for their
    /// first lens.
    #[test]
    fn the_fifth_coefficient_is_read_out_of_field_15() {
        near(coefficients(&record(), 0)[4], 0.001_044_08, 1e-9);
        near(coefficients(&record(), 1)[4], 0.000_955_506, 1e-9);
        near(coefficients(&record_b(), 0)[4], 0.000_983_484, 1e-9);
        near(coefficients(&record_b(), 1)[4], 0.001_156_67, 1e-9);
        // And it is not zero, which is what a reader that stopped at field 8
        // would leave behind, and what turns this lens back into the 210
        // degree one the pass drew until 2026-08-08.
        for record in [record(), record_b()] {
            for lens in 0..2 {
                assert!(coefficients(&record, lens)[4] > 0.0);
            }
        }
    }

    /// All five come through in the file's own order and unscaled, and the
    /// four that sit together are not silently shifted along by the fifth
    /// being read: `k1` is the big one, near 0.068, and the run alternates in
    /// sign down to `k4`.
    #[test]
    fn the_four_adjacent_coefficients_keep_their_own_order() {
        let k = coefficients(&record(), 0);
        // A tolerance of a ten-millionth and not a billionth: the file writes
        // `f32`, so every number here is the widening of one.
        near(k[0], 0.068_134, 1e-7);
        near(k[1], -0.013_797, 1e-7);
        near(k[2], 0.011_794_4, 1e-7);
        near(k[3], -0.007_332_25, 1e-7);
        for (index, coefficient) in coefficients(&record_b(), 1).into_iter().enumerate() {
            let expected = [0.0700723, -0.016647, 0.0146503, -0.00833712, 0.00115667][index];
            near(coefficient, expected, 1e-7);
        }
    }

    /// The Brown-Conrady block stays empty on this camera, and so does the
    /// mirror parameter: those are Insta360's model and this lens's five
    /// numbers are a polynomial in an angle, which travels on the model
    /// itself. A non-zero here would be the `mei` arm of the pass reading a
    /// DJI lens.
    #[test]
    fn the_mei_terms_are_left_empty_on_a_dji_lens() {
        let calibration = from_record(&record()).unwrap();
        for lens in &calibration.lenses {
            let Distortion { k1, k2, k3, p1, p2 } = lens.distortion;
            assert_eq!([k1, k2, k3, p1, p2], [0.0; 5]);
            assert_eq!(lens.intrinsics.xi, 0.0);
        }
    }

    /// A calibration is per unit, so two cameras of one model do not share
    /// one: every coefficient of the owner's camera differs from the sample
    /// unit's, and the key the seam correction is filed under differs with
    /// them.
    #[test]
    fn two_units_are_two_calibrations() {
        let (a, b) = (coefficients(&record(), 0), coefficients(&record_b(), 0));
        for (index, (left, right)) in a.into_iter().zip(b).enumerate() {
            assert_ne!(left, right, "coefficient {index}");
        }
        assert_ne!(
            from_record(&record()).unwrap().camera_key(),
            from_record(&record_b()).unwrap().camera_key()
        );
    }

    /// The two lenses point opposite ways, which is what a back-to-back pair
    /// is, and they point along Kjerag's own axes rather than the file's: the
    /// entry whose recorded yaw is about zero looks **forward**, which is
    /// `+z`, and the other looks back. That is [`BODY`] doing its job, and
    /// without it both lenses point at the sky.
    #[test]
    fn the_two_mountings_are_opposed_along_kjerags_own_axes() {
        let calibration = from_record(&record()).unwrap();
        let axis = |index: usize| {
            calibration.lenses[index]
                .mounting
                .expect("an Osmo lens carries its own mounting")
                .transpose()
                .mul_vec([0.0, 0.0, 1.0])
        };
        // Entry 1 is the one whose yaw is about 180 and entry 2 the one whose
        // yaw is about 0, in that order, so lens 0 looks back.
        let (back, front) = (axis(0), axis(1));
        near(front[2], 1.0, 0.01);
        near(back[2], -1.0, 0.01);
        for pointing in [front, back] {
            near(pointing[1], 0.0, 0.05);
        }
    }

    /// [`BODY`] is a change of basis and not a reshuffle: a reflection here
    /// would render the sphere inside out, and every mounting built through it
    /// would carry the reflection too.
    #[test]
    fn the_body_frames_differ_by_a_rotation() {
        near(BODY.determinant(), 1.0, 1e-12);
        // The file's own vertical, `+z`, is up the picture, which in Kjerag's
        // frame is `-y`.
        let up = BODY.transpose().mul_vec([0.0, 0.0, 1.0]);
        assert_eq!(up, [0.0, -1.0, 0.0]);
        for lens in from_record(&record()).unwrap().lenses {
            near(lens.mounting.unwrap().determinant(), 1.0, 1e-9);
        }
    }

    /// [`BODY_QUAT`] is [`BODY`], which is what lets the orientation be
    /// composed as a quaternion and the mountings as a matrix without the two
    /// drifting apart.
    #[test]
    fn the_body_quaternion_is_the_body_matrix() {
        let (got, want) = (BODY_QUAT.matrix().rows(), BODY.rows());
        for (row, (a, b)) in got.into_iter().zip(want).enumerate() {
            for axis in 0..3 {
                near(a[axis], b[axis], 1e-12);
            }
            let _ = row;
        }
        near(BODY_QUAT.matrix().determinant(), 1.0, 1e-12);
    }

    /// **The headline, and the owner's defect.** The wearer turns; the world
    /// does not.
    ///
    /// The file states the turn the way this camera states one: a
    /// `body_from_world` quaternion about a `z` up world, so a body that has
    /// turned `psi` to the left writes `-psi`. Run through [`pointing`] and
    /// then through the composition `kjerag_render` does with it, a direction
    /// fixed in the world has to come back into the body turned by exactly
    /// that much, which is what leaves it on the same pixel.
    ///
    /// **Measured against the capture's first frame and not against an
    /// absolute datum**, which is both what the shipped path does and the only
    /// claim that means anything. [`from_first_heading`] takes the first
    /// sample's heading off the whole track, so what a pilot ever sees is the
    /// turn RELATIVE to where the camera was looking when the file opened;
    /// [`MOUNTING`]'s quarter turn is a constant yaw on an upright camera
    /// (`a_mounting_turn_is_a_pure_heading_on_an_upright_camera`) and comes off
    /// with it. So the fixture is two frames - level, then turned - run through
    /// the real datum step, and the assertion is on the second.
    #[test]
    fn a_wearer_who_turns_leaves_the_world_where_it_was() {
        for turn_deg in [10.0f64, -35.0, 90.0, 133.0, 179.0] {
            let turn = turn_deg.to_radians();
            // What the camera writes: the world seen from a body that turned.
            let (sin, cos) = (-turn * 0.5).sin_cos();
            let frames = [[1.0f32, 0.0, 0.0, 0.0], [cos as f32, 0.0, 0.0, sin as f32]];
            let track = from_first_heading(
                frames
                    .into_iter()
                    .enumerate()
                    .map(|(index, written)| OrientationSample {
                        offset_us: index as i64 * 33_000,
                        world_from_body: pointing(&frame_record(written))
                            .expect("no orientation in the record"),
                    })
                    .collect(),
            );
            let world_from_body = track.samples()[1].world_from_body;
            // Kjerag's world vertical is `-y`, and a turn about it is what
            // `about_down` names, the other way round because down is `+y`.
            let expected = Quat::about_down(-turn);
            assert!(
                world_from_body.angle_to(expected).to_degrees() < 1e-4,
                "{turn_deg} degrees came out {world_from_body:?}, wanted {expected:?}"
            );
            // And what the pass then draws with: the picture's forward, held
            // still, is the body direction the turn moved it to.
            let body_from_world = world_from_body.conjugate();
            // A tolerance of a millionth and not a billionth: the camera
            // writes `f32`, so the fixture's turn is one too.
            let ahead = body_from_world.rotate([0.0, 0.0, 1.0]);
            near(ahead[0], turn.sin(), 1e-6);
            near(ahead[1], 0.0, 1e-6);
            near(ahead[2], turn.cos(), 1e-6);
            // The first frame is the datum, so it comes out looking ahead
            // whatever the mounting is.
            near(track.samples()[0].world_from_body.heading(), 0.0, 1e-9);
        }
    }

    /// The negative control for the same fact, which is the reading this file
    /// shipped with until 2026-08-08: take the quaternion as written and the
    /// picture turns **twice** as far as the wearer rather than standing
    /// still. Measured on the owner's capture as a locked view that tracked
    /// the turn worse than no lock at all.
    #[test]
    fn reading_the_quaternion_unconjugated_turns_the_picture_twice() {
        let turn = 40.0f64.to_radians();
        let (sin, cos) = (-turn * 0.5).sin_cos();
        let written = Quat {
            w: cos,
            v: [0.0, 0.0, sin],
        };
        let wrong = BODY_QUAT.conjugate().times(written).times(BODY_QUAT);
        let right = BODY_QUAT
            .conjugate()
            .times(written.conjugate())
            .times(BODY_QUAT);
        near(right.angle_to(wrong).to_degrees(), 2.0 * 40.0, 1e-9);
    }

    /// A camera held upright is upright, and one that leans leans the way it
    /// leaned: the file's own vertical, carried through, is the world's.
    ///
    /// With the heading off, for the reason above: the mounting's quarter turn
    /// leaves an upright camera upright and pointed a constant 90 degrees
    /// round, and the constant is removed a step later.
    #[test]
    fn an_upright_camera_comes_out_upright() {
        let level = bare(pointing(&frame_record([1.0, 0.0, 0.0, 0.0])).expect("no orientation"));
        assert!(
            level.angle_to(Quat::IDENTITY).to_degrees() < 1e-4,
            "{level:?}"
        );
        // Leaned 20 degrees about the file's own `x`, which is the body
        // rotation `body_from_world` writes negated.
        let lean = 20.0f64.to_radians();
        let (sin, cos) = (-lean * 0.5).sin_cos();
        let leaned = pointing(&frame_record([cos as f32, sin as f32, 0.0, 0.0])).expect("none");
        // The camera's own up is `-y` in Kjerag's body frame, and it has to
        // land `lean` away from the world's own up, not twice that and not
        // nothing.
        let up = leaned.rotate([0.0, -1.0, 0.0]);
        near(up[1].acos().to_degrees(), 180.0 - lean.to_degrees(), 1e-4);
    }

    /// **A mounting turn cannot move the heading**, which is what lets the
    /// proven half of this file stand while the tilt is worked on.
    ///
    /// The turn is about the camera's own up axis, so on an upright camera it
    /// IS a turn about the world's vertical - a pure yaw, and
    /// [`from_first_heading`] takes the first frame's yaw off every sample. So
    /// the mounting changes an upright frame by a constant heading and by
    /// nothing else, and a whole capture of upright frames comes out where it
    /// came out before. What it does move, and is meant to, is the tilt, in
    /// proportion to the lean.
    #[test]
    fn a_mounting_turn_is_a_pure_heading_on_an_upright_camera() {
        {
            let turn = MOUNTING.turn();
            // The camera's own up in Kjerag's frame, which the turn is about.
            let up = turn.rotate([0.0, -1.0, 0.0]);
            near(up[0], 0.0, 1e-12);
            near(up[1], -1.0, 1e-12);
            near(up[2], 0.0, 1e-12);
            // And it is exactly the yaw `from_first_heading` removes.
            near(
                turn.heading().to_degrees(),
                -MOUNTING.turn_deg,
                1e-9 * MOUNTING.turn_deg.abs().max(1.0),
            );
        }
    }

    /// The four sign readings of the last session are two of these families
    /// sampled half a turn apart, which is why none of its eight rows held the
    /// horizon: negating `x` and `y` together is conjugation by a half turn
    /// about the file's `z`, and in this composition that is a mounting of 180
    /// degrees plus a world yaw nobody can see.
    #[test]
    fn a_half_turn_of_mounting_is_the_other_sign_reading() {
        let raw = Quat {
            w: 0.83,
            v: [0.21, -0.37, 0.35],
        }
        .normalized();
        for (reading, other) in [
            ([-1.0, -1.0, -1.0], [1.0, 1.0, -1.0]), // wXYZ / wxyZ
            ([-1.0, 1.0, -1.0], [1.0, -1.0, -1.0]),
        ]
        // wXyZ / wxYZ
        {
            let sign = |signs: [f64; 3]| Quat {
                w: raw.w,
                v: std::array::from_fn(|i| raw.v[i] * signs[i]),
            };
            let compose = |written: Quat, turn_deg: f64| {
                BODY_QUAT
                    .conjugate()
                    .times(written)
                    .times(BODY_QUAT)
                    .times(Quat::about_down(-turn_deg.to_radians()))
            };
            let half = compose(sign(reading), 180.0);
            let named = compose(sign(other), 0.0);
            // Equal up to a world yaw, which `from_first_heading` removes, so
            // the two are compared with their headings taken off.
            assert!(
                bare(half).angle_to(bare(named)).to_degrees() < 1e-9,
                "{reading:?} at 180 is not {other:?}: {half:?} against {named:?}"
            );
        }
    }

    /// **The composition that ships, written out in full and by hand**, so that
    /// changing [`MOUNTING`] fails a test rather than moving a horizon quietly.
    ///
    /// The mirror is not the conjugate: a conjugate negates all three of `x`,
    /// `y` and `z`, and this negates `x` and `z` only, which is what makes the
    /// file's inertial frame left handed against the optical one. The quarter
    /// turn goes on the body side, after the change of basis. The composition
    /// this pins is the one the owner tested and approved on 2026-08-08.
    #[test]
    fn the_shipped_composition_is_the_mirror_and_the_quarter_turn() {
        let raw = Quat {
            w: 0.83,
            v: [0.21, -0.37, 0.35],
        };
        let record = frame_record([
            raw.w as f32,
            raw.v[0] as f32,
            raw.v[1] as f32,
            raw.v[2] as f32,
        ]);
        // Through an `f32` and back, because that is what the file holds.
        let raw = Quat {
            w: f64::from(raw.w as f32),
            v: raw.v.map(|c| f64::from(c as f32)),
        };
        let mirrored = Quat {
            w: raw.w,
            v: [-raw.v[0], raw.v[1], -raw.v[2]],
        }
        .normalized();
        let want = BODY_QUAT
            .conjugate()
            .times(mirrored)
            .times(BODY_QUAT)
            .times(Quat::about_down(-90f64.to_radians()));
        let got = pointing(&record).expect("no orientation in the record");
        assert!(got.angle_to(want).to_degrees() < 1e-12, "{got:?}");

        // And it is NOT what shipped before the mounting was measured, which is
        // the whole of what changed for the pilot: the conjugate with no turn.
        let was = BODY_QUAT
            .conjugate()
            .times(raw.normalized().conjugate())
            .times(BODY_QUAT);
        assert!(
            got.angle_to(was).to_degrees() > 1.0,
            "the mounting composes to what shipped before it, so nothing was baked"
        );
    }

    /// The accelerometer beside the quaternion is read, and it is the one
    /// stream in the file that can refuse a reading without a picture.
    #[test]
    fn the_plumb_line_is_read_where_the_quaternion_is() {
        let got = accelerometer(&frame_record([1.0, 0.0, 0.0, 0.0])).expect("no accelerometer");
        near(got[0], 0.01, 1e-7);
        near(got[1], -0.02, 1e-7);
        near(got[2], -0.99, 1e-7);
        // Gravity points down the camera's own `z`, so world up is the other
        // way: a camera this nearly upright leans about a degree.
        let lean = between([0.0, 0.0, 1.0], got.map(|c| -c));
        assert!(lean < 2.0, "{lean}");
    }

    /// One frame's worth of what [`Plumb::read`] scores: a lean in degrees, a
    /// measured accelerometer, and the reading whose predicted up is compared
    /// against it.
    ///
    /// The reading is built from the up direction it is meant to predict, so a
    /// test says what it wants in world terms and the fixture composes the
    /// quaternion that says it.
    fn plumb_frame(
        lean_deg: f64,
        predicted: [f64; 3],
        measured: [f64; 3],
    ) -> (f64, [f64; 3], Quat) {
        // `Plumb::read` takes the predicted up as `written.conjugate()` turning
        // the file's `+z`, so the quaternion wanted here is the one whose
        // conjugate does that.
        let axis = [-predicted[1], predicted[0], 0.0];
        let sine = norm(axis);
        let written = match sine > 0.0 {
            false => Quat {
                w: 1.0,
                v: [0.0; 3],
            },
            true => {
                let angle = sine.atan2(predicted[2]);
                let half = 0.5 * angle;
                Quat {
                    w: half.cos(),
                    v: axis.map(|c| c / sine * half.sin()),
                }
            }
        };
        (lean_deg, measured, written.conjugate())
    }

    /// **The gate holds when the file's own two records agree.**
    ///
    /// Down measured where the file's own quaternion predicts it, on frames
    /// that lean: the file agrees with itself, and its orientations are used.
    #[test]
    fn a_file_whose_gravity_confirms_the_mounting_holds_its_horizon() {
        // The mounting's `plumb` signs are carried onto the measurement, so a
        // fixture that wants "measured down agrees with predicted up" writes
        // the measurement through them.
        let up = [0.3, 0.0, (1.0f64 - 0.09).sqrt()];
        let measured = std::array::from_fn(|i| -up[i] * MOUNTING.plumb[i]);
        let frames: Vec<_> = (0..40).map(|_| plumb_frame(20.0, up, measured)).collect();
        let verdict = Plumb::read(&MOUNTING, &frames);
        assert!(
            matches!(verdict, Plumb::Agrees(_)),
            "a file whose gravity is where the mounting says was not believed"
        );
        assert!(verdict.held());
    }

    /// **The gate refuses when the file's own two records contradict each
    /// other**, which is unit A and is the whole reason there is a gate.
    ///
    /// The fixture is the failure the corpus actually shows: the lean is the
    /// right size and points somewhere else entirely. The refusal is an EMPTY
    /// orientation track, the same shape a capture with no inertial record at
    /// all comes out with, so it reaches the pilot as the disabled menu item
    /// and the `level:` line and never as an error.
    #[test]
    fn a_file_whose_gravity_contradicts_the_mounting_holds_no_horizon() {
        let predicted = [0.3, 0.0, (1.0f64 - 0.09).sqrt()];
        // The same lean, turned most of the way round the vertical.
        let elsewhere = [-0.3, 0.0, (1.0f64 - 0.09).sqrt()];
        let measured = std::array::from_fn(|i| -elsewhere[i] * MOUNTING.plumb[i]);
        let frames: Vec<_> = (0..40)
            .map(|_| plumb_frame(20.0, predicted, measured))
            .collect();
        let verdict = Plumb::read(&MOUNTING, &frames);
        let Plumb::Disagrees(reading) = &verdict else {
            panic!("a file whose gravity says otherwise was believed anyway");
        };
        assert!(
            reading.miss > reading.null,
            "the fixture does not actually lose to its own null: {} against {}",
            reading.miss,
            reading.null
        );
        assert!(!verdict.held());
    }

    /// **A reading inside the ceiling but no better than assuming the camera
    /// never leans is still refused**, which is what makes this a verification
    /// and not a tolerance.
    ///
    /// Without the null bar a file could pass on a number that says nothing:
    /// this fixture misses by less than [`PLUMB_CEILING_DEG`] and the null
    /// misses by less still. The other two bars are checked to be slack here,
    /// so what refuses it is the null and nothing else.
    #[test]
    fn a_mounting_no_better_than_no_mounting_is_refused() {
        let predicted = [0.10, 0.0, (1.0f64 - 0.01).sqrt()];
        // Barely leaned, in the same direction the reading says: the miss is
        // the difference of the two and the null is the smaller of them.
        let measured_up = [0.02, 0.0, (1.0f64 - 0.0004).sqrt()];
        let measured = std::array::from_fn(|i| -measured_up[i] * MOUNTING.plumb[i]);
        let frames: Vec<_> = (0..40)
            .map(|_| plumb_frame(20.0, predicted, measured))
            .collect();
        let Plumb::Disagrees(reading) = Plumb::read(&MOUNTING, &frames) else {
            panic!("a reading that beat nothing was believed");
        };
        assert!(
            reading.miss < PLUMB_CEILING_DEG,
            "the fixture is refused by the ceiling, so it does not test the null bar: {}",
            reading.miss
        );
        assert!(
            reading.miss < reading.flipped,
            "the fixture is refused by the flipped bar too, so it does not test the null bar \
             alone: {} against {}",
            reading.miss,
            reading.flipped
        );
        assert!(reading.miss > reading.null, "{reading:?}");
    }

    /// **The one bar the mounting itself can fail**: a quaternion the file
    /// wrote the other way round is caught even where it is inside the ceiling
    /// and beats the null.
    ///
    /// Without this bar the check has nothing in it that any mounting can
    /// fail, because the mirror and the turn cancel out of the other two
    /// (`the_check_scores_a_mirrored_mounting_identically`).
    #[test]
    fn a_quaternion_better_read_the_other_way_round_is_refused() {
        // A reading and its flip sit at the same lean and differ in azimuth
        // (`1 - 2(x^2 + y^2)` is the same for a rotation and its transpose),
        // so all three vectors here are put on one 9 degree circle about
        // vertical and named by azimuth: the reading at 0, the file's own
        // gravity at 45, the flipped reading at 70.
        let on_circle = |azimuth: f64| {
            let (lean, azimuth) = (9.0f64.to_radians(), azimuth.to_radians());
            [
                lean.sin() * azimuth.cos(),
                lean.sin() * azimuth.sin(),
                lean.cos(),
            ]
        };
        let measured_up = on_circle(45.0);
        let measured: [f64; 3] = std::array::from_fn(|i| -measured_up[i] * MOUNTING.plumb[i]);
        let (lean, _, upright) = plumb_frame(20.0, on_circle(0.0), measured);
        // The spin is what makes this fixture possible at all. A reading with
        // no spin about the file's own vertical in it puts its flip
        // diametrically opposite, and then the flipped bar can never be the
        // one that binds: the arithmetic has the flip nearer only when the
        // measurement is on the far side of vertical, which is where the null
        // bar has already refused. A real capture's quaternion is not so
        // obliging, so 110 degrees of it go in here.
        let written =
            Quat::from_rotation_vector([0.0, 0.0, (-110.0f64).to_radians()]).times(upright);
        let frames: Vec<_> = (0..40).map(|_| (lean, measured, written)).collect();
        let Plumb::Disagrees(reading) = Plumb::read(&MOUNTING, &frames) else {
            panic!("a quaternion the file reads better inverted was believed anyway");
        };
        assert!(
            reading.miss < PLUMB_CEILING_DEG && reading.miss < reading.null,
            "the fixture is refused by the ceiling or the null, so it does not test the flipped \
             bar: {reading:?}"
        );
        assert!(reading.flipped < reading.miss, "{reading:?}");
    }

    /// **The check cannot see a mirror in the mounting.** Not "does not
    /// happen to on this corpus": cannot, as arithmetic, and this is the
    /// proof.
    ///
    /// Both of the file's records are carried through the mounting on the way
    /// into [`Plumb::read`] - the quaternion through [`Mounting::read`] and
    /// the accelerometer through [`Mounting::plumb`] - so a change of basis
    /// applied to both leaves the angle between them alone. `as written` below
    /// is not a straw man: it is the family the picture refuted at 65.9
    /// degrees rms of scatter (docs/research/osv-format.md 6), and it scores
    /// this check to the last bit the same as the shipped one while composing
    /// an orientation tens of degrees away from it.
    ///
    /// What DOES move is the flipped reading's own family, so the third bar is
    /// not vacuous: scoring the conjugate family directly returns the number
    /// the shipped family reports as [`Reading::flipped`].
    #[test]
    fn the_check_scores_a_mirrored_mounting_identically() {
        // The same conjugation family as MOUNTING, reached through no
        // reflection at all: `S` is the identity and so are the accelerometer
        // signs it implies.
        let as_written = Mounting {
            reading: [1.0, 1.0, 1.0],
            turn_deg: -84.6,
            plumb: [1.0, 1.0, 1.0],
        };
        // The other conjugation family, which is the one the flipped number
        // is about: `S = -I`, whose accelerometer signs are the identity.
        let conjugate = Mounting {
            reading: [-1.0, -1.0, -1.0],
            turn_deg: -107.1,
            plumb: [1.0, 1.0, 1.0],
        };

        // A dozen leaned instants of one imaginary flight, and for each the
        // accelerometer the file would have written if it agreed with its own
        // quaternion to about a degree. The measurement is ONE array of three
        // numbers per frame, shared by every candidate below, exactly as a
        // real file's is.
        let raws: Vec<Quat> = (0..12)
            .map(|k| {
                let lean = (12.0 + 1.5 * k as f64).to_radians();
                let azimuth = (37.0 * k as f64).to_radians();
                let tilt =
                    Quat::from_rotation_vector([lean * azimuth.cos(), lean * azimuth.sin(), 0.0]);
                let spin = Quat::from_rotation_vector([0.0, 0.0, (23.0 * k as f64).to_radians()]);
                spin.times(tilt).normalized()
            })
            .collect();
        let measured: Vec<[f64; 3]> = raws
            .iter()
            .enumerate()
            .map(|(k, raw)| {
                let up = MOUNTING
                    .read(*raw)
                    .normalized()
                    .conjugate()
                    .rotate([0.0, 0.0, 1.0]);
                // A degree of honest disagreement, so the equality below is
                // about two numbers that are not both zero.
                let off = Quat::from_rotation_vector([
                    0.0,
                    1.0f64.to_radians(),
                    (0.5 * k as f64).to_radians(),
                ]);
                let up = off.rotate(up);
                std::array::from_fn(|i| -up[i] * MOUNTING.plumb[i])
            })
            .collect();

        // What `leaned` hands the check, per candidate: the lean is one fact
        // about the file (`1 - 2(x^2 + y^2)` is the same for a rotation and
        // its transpose and carries no sign), and the reading is the
        // candidate's own.
        let frames = |mount: &Mounting| -> Vec<(f64, [f64; 3], Quat)> {
            raws.iter()
                .zip(&measured)
                .map(|(raw, a)| {
                    let written = mount.read(*raw).normalized();
                    let up = written.conjugate().rotate([0.0, 0.0, 1.0]);
                    (up[2].clamp(-1.0, 1.0).acos().to_degrees(), *a, written)
                })
                .collect()
        };
        let score = |mount: &Mounting| match Plumb::read(mount, &frames(mount)) {
            Plumb::Agrees(reading) | Plumb::Disagrees(reading) => reading,
            Plumb::Blind => panic!("the fixture does not lean"),
        };

        let shipped = score(&MOUNTING);
        let mirrored = score(&as_written);
        assert!(shipped.frames > 0 && shipped.miss > 0.1, "{shipped:?}");
        assert_eq!(mirrored.frames, shipped.frames, "{mirrored:?}");
        near(mirrored.miss, shipped.miss, 1e-9);
        near(mirrored.null, shipped.null, 1e-9);
        near(mirrored.flipped, shipped.flipped, 1e-9);

        // And it is not that the two mountings are the same mounting: the
        // orientation they compose is tens of degrees apart, which is the
        // tilted horizon the check would let through.
        let compose = |mount: &Mounting, raw: Quat| {
            BODY_QUAT
                .conjugate()
                .times(mount.read(raw).normalized())
                .times(BODY_QUAT)
                .times(mount.turn())
        };
        let apart = raws
            .iter()
            .map(|raw| {
                compose(&MOUNTING, *raw)
                    .angle_to(compose(&as_written, *raw))
                    .to_degrees()
            })
            .fold(0.0f64, f64::max);
        assert!(
            apart > 20.0,
            "the two mountings draw the same picture: {apart}"
        );

        // The third bar is the one that does move, and the number it moves to
        // is the one the shipped reading already prints.
        near(score(&conjugate).miss, shipped.flipped, 1e-9);
        near(score(&conjugate).flipped, shipped.miss, 1e-9);
    }

    /// **A capture that never leans is held, not refused.**
    ///
    /// The check is blind on an upright camera by construction - every reading
    /// predicts the same lean magnitude, so they differ only in proportion to
    /// the lean - and being blind is not evidence against. It costs nothing to
    /// be wrong about, either: the error this check exists to catch is a lean
    /// pointed the wrong way, and a file with no lean in it cannot show one.
    #[test]
    fn a_capture_that_never_leans_is_held_rather_than_refused() {
        let up = [0.0, 0.0, 1.0];
        let measured = std::array::from_fn(|i| -up[i] * MOUNTING.plumb[i]);
        let frames: Vec<_> = (0..40).map(|_| plumb_frame(0.5, up, measured)).collect();
        let verdict = Plumb::read(&MOUNTING, &frames);
        assert!(matches!(verdict, Plumb::Blind), "a still camera was scored");
        assert!(verdict.held());
        // And a file with no inertial record at all reaches the same place.
        assert!(Plumb::read(&MOUNTING, &[]).held());
    }

    /// A box filter over a stride leaves gravity behind, which is the whole
    /// reason the plumb check low passes at all.
    #[test]
    fn the_plumb_low_pass_takes_a_stride_out() {
        let rate = 30.0;
        let raw: Vec<[f64; 3]> = (0..300)
            .map(|n| {
                // Gravity, plus a 2 Hz stride half a g across.
                let phase = std::f64::consts::TAU * 2.0 * n as f64 / rate;
                [0.5 * phase.sin(), 0.5 * phase.cos(), -1.0]
            })
            .collect();
        let smooth = low_passed(&raw, rate, PLUMB_SECS);
        let worst = smooth[100..200]
            .iter()
            .map(|v| between(*v, [0.0, 0.0, -1.0]))
            .fold(0.0, f64::max);
        let before = between(raw[150], [0.0, 0.0, -1.0]);
        assert!(worst < 1.0, "{worst} left of a stride that was {before}");
        assert!(before > 20.0, "the fixture has no stride in it: {before}");
    }

    /// The world frame's zero heading is the first frame's, so a file opens
    /// looking where the camera looked rather than at whatever azimuth the
    /// camera's gyroscope happened to have started counting from.
    #[test]
    fn the_world_frames_zero_heading_is_the_first_frame() {
        let track = from_first_heading(
            [0.0f64, 30.0, -95.0]
                .into_iter()
                .enumerate()
                .map(|(index, heading)| OrientationSample {
                    offset_us: index as i64 * 33_000,
                    world_from_body: Quat::about_down(140.0f64.to_radians())
                        .times(Quat::about_down(heading.to_radians())),
                })
                .collect(),
        );
        let headings: Vec<f64> = track
            .samples()
            .iter()
            .map(|sample| sample.world_from_body.heading().to_degrees())
            .collect();
        near(headings[0], 0.0, 1e-9);
        near(headings[1], 30.0, 1e-9);
        near(headings[2], -95.0, 1e-9);
        assert!(from_first_heading(Vec::new()).is_empty());
    }

    /// A sample the camera wrote no orientation into is one this walks past,
    /// and a capture where that is every sample is the refusal an `.OSV` had
    /// before this file read any: an empty track, which horizon lock is a
    /// no-op on rather than an error.
    #[test]
    fn a_sample_with_no_orientation_in_it_is_read_past() {
        assert!(pointing(&[]).is_none());
        assert!(pointing(&record()).is_none());
        let whole = frame_record([1.0, 0.0, 0.0, 0.0]);
        for cut in [1, 4, 9, whole.len() / 2, whole.len() - 1] {
            assert!(pointing(&whole[..cut]).is_none(), "cut at {cut}");
        }
    }

    /// A file with no calibration in it says so rather than producing one, and
    /// a record cut off in the middle of a field is that same answer: nothing
    /// here panics on bytes it did not write.
    #[test]
    fn a_record_that_is_not_one_names_what_is_missing() {
        assert!(matches!(
            from_record(&[]),
            Err(Error::TelemetryField("header"))
        ));
        let whole = record();
        for cut in [1, 5, 20, 60, whole.len() / 2, whole.len() - 1] {
            match from_record(&whole[..cut]) {
                Ok(_) | Err(Error::TelemetryField(_)) | Err(Error::CanvasMismatch) => {}
                other => panic!("cut at {cut} answered {other:?}"),
            }
        }
    }

    /// A lens whose numbers are written against a frame this file does not
    /// deliver is refused rather than scaled on a guess.
    #[test]
    fn a_lens_calibrated_against_another_frame_is_refused() {
        let mut camera = submessage(field::FIRMWARE, b"1");
        camera.extend(submessage(field::MODEL, b"Osmo 360"));
        let mut picture = varint_field(field::WIDTH, 2880);
        picture.extend(varint_field(field::HEIGHT, 2880));
        let mut table = submessage(
            1,
            &entry(
                1046.0,
                1046.0,
                1440.0,
                1440.0,
                [0.0; 5],
                [1.0, 0.0, 0.0, 0.0],
            ),
        );
        table.extend(submessage(
            2,
            &entry(
                1046.0,
                1046.0,
                1440.0,
                1440.0,
                [0.0; 5],
                [0.0, 0.0, 1.0, 0.0],
            ),
        ));
        let mut video = submessage(field::PICTURE, &picture);
        video.extend(submessage(field::LENSES, &table));
        let mut record = submessage(field::HEADER, &submessage(field::CAMERA, &camera));
        record.extend(submessage(field::VIDEO, &video));

        assert!(matches!(from_record(&record), Err(Error::CanvasMismatch)));
    }
}
