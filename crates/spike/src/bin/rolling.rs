//! Which way the sensor reads, and what correcting for it is worth.
//!
//! The verification harness for issue #9. The trailer says a frame takes
//! 15.883 ms to come off the sensor and says nothing about which way it comes,
//! and the direction is the whole answer: a correction applied the wrong way
//! round does not fail to remove the skew, it doubles it.
//!
//! ```sh
//! cargo run --release -p kyerag-spike --bin rolling -- <file.insv> model=1
//! cargo run --release -p kyerag-spike --bin rolling -- <file.insv> from=1043 count=12
//! ```
//!
//! Two instruments, and the second is the one that answers the question.
//!
//! - `model=1` reads the gyro track alone, no pixels and no GPU: how fast this
//!   file actually turns, whether the turn across one readout is worth
//!   modelling as a straight line, whether the orientation track's own turn is
//!   the gyroscope's, and how many rounds the row solve takes to settle.
//! - With no `model`, it measures **the seam**, which is the one place a
//!   readout displacement cannot hide. The two lenses are mounted a half turn
//!   apart, so their sensors' rows run in nearly opposite world directions and
//!   the displacement does not cancel between them: it doubles. Both lenses
//!   are sampled on the *same* angular grid around directions on the seam
//!   circle, so the shift that best correlates between them is in degrees of
//!   world angle with no rotation to undo, exactly as issue #7 re-measured it
//!   (docs/research/insv-format.md 4.9). Every candidate direction is run over
//!   the same pixels, so the three wrong answers are the negative control.
//!
//! The frames are decoded to system memory rather than imported, because this
//! instrument samples the delivered picture at angles rather than drawing it:
//! the map it samples through is `kyerag_render::Reframe`, the shader's own
//! Rust twin, so what is measured is the pass that ships.
//!
//! Nothing here writes an image. The numbers are the output, and the footage
//! stays on the box.

use std::collections::VecDeque;
use std::path::PathBuf;

use kyerag_media::Fallible;
use kyerag_meta::{CalibrationSet, Filter, OrientationTrack, Quat, Readout, Sweep};
use kyerag_render::{Camera, Held, Reframe, Rolling, Sampling, Size};
use kyerag_spike::{Pair, Plane, Walk};

/// A unit vector, as the projection's own mirror normalizes one.
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|c| c / length)
}

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let calibration = CalibrationSet::from_insv(&options.input)?;
    let track = calibration.orientation(Filter::default());
    if track.is_empty() {
        return Err("this file carries no IMU record, so there is no readout to correct".into());
    }
    let readout = calibration.readout();
    println!(
        "camera: {} {}, readout {:.3} ms, sweep {}",
        calibration.camera_model,
        calibration.firmware,
        readout.seconds * 1e3,
        name(readout.sweep),
    );
    let rates = rates(&track, readout, options.instants);
    carries(&rates, readout);

    match (options.model, options.find, options.pair) {
        (_, Some(count), _) => fastest(&track, readout, count),
        (true, _, _) => model(&calibration, &track, readout, &options),
        (_, _, true) => pairs(&calibration, &track, readout, &options),
        _ => seam(&calibration, &track, readout, &options),
    }
}

// ------------------------------------------------------------ what it carries

/// How fast the body turns across one readout, at evenly spaced instants of
/// the whole file, sorted: the distribution every measurement below is scaled
/// by.
fn rates(track: &OrientationTrack, readout: Readout, count: usize) -> Vec<f64> {
    let span = (readout.seconds * 1e6) as i64;
    let first = track.samples().first().map_or(0, |s| s.offset_us);
    let last = track.samples().last().map_or(0, |s| s.offset_us);
    let mut rates: Vec<f64> = (0..count.max(1))
        .map(|step| first + (last - first) * step as i64 / count.max(1) as i64)
        .map(|at| norm(track.turn(at - span / 2, at + span / 2)).to_degrees() / readout.seconds)
        .collect();
    rates.sort_by(f64::total_cmp);
    rates
}

/// Whether this file has a readout displacement in it to measure at all,
/// printed before anything is decoded.
///
/// Everything below reads the displacement a readout leaves in the pictures,
/// and that displacement is the rate the camera turned at times the readout's
/// own length. **A file of a camera that did not turn carries none of it**,
/// whichever way its sensor reads, so every instrument then reports its own
/// noise and the control that would catch that has nothing to apply either.
///
/// It prints rather than refuses, because a still capture is worth measuring
/// for other reasons: on 2026-07-31 one gave the seam instrument's own noise
/// floor, 0.018 degrees frame to frame, which is the number the flight
/// footage's 0.100 has to be read against (docs/research/insv-format.md 4.9).
/// What it must not do is answer issue #9, and the line below is what says so:
/// that capture's whole-frame displacement was 0.02 degrees against the 4.8
/// a hand twist gives.
fn carries(rates: &[f64], readout: Readout) {
    let at = |p: f64| rates[((rates.len().max(1) - 1) as f64 * p) as usize];
    println!(
        "carries: {} instants, median {:.1} deg/s, 90th {:.1}, 99th {:.1}, worst {:.1}",
        rates.len(),
        at(0.5),
        at(0.9),
        at(0.99),
        at(1.0),
    );
    println!(
        "         a whole-frame readout displaces the picture by {:.2} degrees at the 99th and \
         {:.2} at\n         the worst. the hand twist 6.7 asks for is {TWIST:.0} deg/s, which is \
         {:.1} degrees",
        at(0.99) * readout.seconds,
        at(1.0) * readout.seconds,
        TWIST * readout.seconds,
    );
}

/// The rate a wrist turns a camera at, which is what the settling capture of
/// docs/research/insv-format.md 6.7 is asking for.
const TWIST: f64 = 300.0;

// ------------------------------------------------------------ frame pairs

/// The readout direction, measured inside one lens rather than across the
/// seam.
///
/// The seam cannot answer it. Both lenses' pictures of a direction on the seam
/// come off their sensors at the same instant unless the two readouts run in
/// opposite world directions, and the [`seam`] measurement says they do not,
/// so whatever displacement the readout leaves there is the same in both and
/// cancels in the comparison. What does not cancel is the **same lens at two
/// instants**: the camera is turning at a different rate in each, so a patch
/// of far-off content that has not moved in the world is displaced by a
/// different amount in each frame, and the difference is what a readout
/// predicts and nothing else does.
///
/// Two things sit on top of that and are fitted out rather than assumed away.
/// The horizon lock leaves a **rigid rotation** between the two frames, three
/// unknowns, which is the same for every patch and is solved for alongside the
/// readout's own scale. The camera's own **translation** displaces near
/// content, which is why the patches are far off the wing and why the residual
/// after the fit is reported next to the scale.
fn pairs(
    calibration: &CalibrationSet,
    track: &OrientationTrack,
    readout: Readout,
    options: &Options,
) -> Fallible<()> {
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let mut walk = Walk::open(&options.input, options.from, calibration.dimension)?;
    // A ring of frames, so that a pair can be taken a chosen distance apart:
    // consecutive frames are turning at nearly the same rate and so carry
    // nearly the same readout displacement, which cancels in the difference.
    // A few frames apart, a roll that is winding up carries a very different
    // one, and the content is still the same content.
    let mut held: VecDeque<(Pair, Instant)> = VecDeque::new();
    // One row per patch per frame pair: the two measured components, and each
    // candidate's prediction of them.
    let mut rows: Vec<Row> = Vec::new();
    let mut controls: Vec<Vec<Row>> = INJECTED.iter().map(|_| Vec::new()).collect();
    let mut steps = 0usize;

    println!(
        "pair:   patches {} degrees off each lens axis, {:.1} degrees across, correlated over \
         +/-{:.1} in {:.2}",
        options.off_axis, options.span, options.search, options.step,
    );
    println!(
        "{:<8} {:>9} {:>9} {:>8}",
        "frame", "roll d/s", "turn gap", "patches"
    );

    while steps < options.count {
        let Some(pair) = walk.next_pair()? else {
            break;
        };
        let at = calibration.exposure[0]
            .frame_time_us(pair.index)
            .unwrap_or((pair.at.as_micros() as i64).max(0));
        let now = Instant {
            world_from_body: track.at(at),
            turn: rolling(track, at, Some(readout)).map_or([0.0; 3], |r| r.turn),
        };
        if held.len() > options.gap {
            held.pop_front();
        }
        if let Some((before, then)) = held.front() {
            let taken = between(calibration, frame, (before, then), (&pair, &now), options);
            // The same pass again with each direction's own readout applied,
            // which displaces the pictures by exactly what that direction
            // predicts: the controls. All four, because the fit answers on two
            // axes and a control on one of them proves nothing about the
            // other. Issue #9's answer sits on the axis #42 never injected.
            for (sweep, control) in INJECTED.iter().zip(&mut controls) {
                control.extend(between_with(
                    calibration,
                    frame,
                    (before, then),
                    (&pair, &now),
                    options,
                    Some(Readout {
                        sweep: *sweep,
                        ..readout
                    }),
                ));
            }
            println!(
                "{:<8} {:>9.1} {:>9.2} {:>8}",
                pair.index,
                now.turn[2].abs().to_degrees() / readout.seconds,
                (norm(now.turn) - norm(then.turn)).to_degrees(),
                taken.len(),
            );
            rows.extend(taken);
            steps += 1;
        }
        held.push_back((pair, now));
    }
    if rows.len() < 8 {
        return Err("too few patches correlated to fit anything".into());
    }

    println!(
        "\n{} patch readings, and one run of every injected readout over the same patches",
        rows.len(),
    );
    println!(
        "{:<12} {:>9} {:>9} {:>9} {:>9} {:>10} {:>12} {:>11}",
        "measured", "across x", "", "down y", "", "residual", "predicts deg", "reads back"
    );
    let uncorrected = fit(&rows);
    println!(
        "{:<12} {uncorrected} {:>12.3}",
        "as it comes",
        spread(rows.iter().map(|row| norm2(row.predicted[0]))),
    );
    // Per lens, which is the question the seam cannot answer: two sensors
    // that sweep the same way in the world sweep opposite ways in their own
    // delivered pictures, because lens 1 is mounted a half turn round.
    for lens in 0..2 {
        let mine: Vec<&Row> = rows.iter().filter(|row| row.lens == lens).collect();
        if mine.len() < 8 {
            continue;
        }
        let mine: Vec<Row> = mine.into_iter().cloned().collect();
        println!("{:<12} {}", format!("lens {lens}"), fit(&mine));
    }
    // The controls. Correcting for a readout subtracts exactly that readout
    // from the pictures, so the fitted sweep has to come down by one along
    // that direction's own axis. Where it does not, this instrument cannot see
    // a displacement of that size on that axis and nothing above it means
    // anything on that axis either.
    for (sweep, control) in INJECTED.iter().zip(&controls) {
        if control.len() < 8 {
            continue;
        }
        let (axis, sign) = reads(*sweep);
        let fitted = fit(control);
        println!(
            "{:<12} {fitted} {:>12.3} {:>11.2}",
            format!("+ {}", name(*sweep)),
            spread(control.iter().map(|row| norm2(row.predicted[0]))),
            sign * (uncorrected.sweep[axis] - fitted.sweep[axis]),
        );
    }
    println!(
        "\nthe sweep is in whole-frame readouts of {:.3} ms: (1, 0) is a sensor read across the \n\
         delivered picture in exactly that time, (-1, 0) the other way, (0, 1) down it, and \n\
         (0, 0) a picture with no readout displacement in it. the four rows below the lenses \n\
         are the controls: each has that direction's own readout taken out of the pictures, so \n\
         it has to read back at 1.00 on its own axis and leave the other axis where it was.",
        readout.seconds * 1e3,
    );
    Ok(())
}

/// Where the body was at one frame's instant, and how it turned across that
/// frame's readout.
struct Instant {
    world_from_body: Quat,
    turn: [f64; 3],
}

/// One patch of one frame pair: what moved, and what each candidate says
/// should have.
#[derive(Clone)]
struct Row {
    /// Which lens saw it. The two are fitted apart as well as together,
    /// because whether their sensors sweep the same way in the world is
    /// exactly what the seam measurement leaves open.
    lens: usize,
    /// The measured displacement, in degrees, along the patch's own two axes.
    measured: [f64; 2],
    /// A rigid rotation's contribution to those two components, per world
    /// axis: the three columns the lock's own residual is fitted through.
    rigid: [[f64; 2]; 3],
    /// A translation's contribution, per world axis, for content all at one
    /// distance: the three columns the camera's own movement between the two
    /// frames is fitted through. Its amplitude carries the distance, which is
    /// why the distance itself does not have to be known.
    flow: [[f64; 2]; 3],
    /// Each candidate's predicted displacement, in the same two components.
    predicted: [[f64; 2]; 4],
}

fn between(
    calibration: &CalibrationSet,
    frame: Size,
    before: (&Pair, &Instant),
    after: (&Pair, &Instant),
    options: &Options,
) -> Vec<Row> {
    between_with(calibration, frame, before, after, options, None)
}

/// Every patch of one frame pair, measured through maps that both hold the
/// world still, so a patch that has not moved in the world reads zero.
fn between_with(
    calibration: &CalibrationSet,
    frame: Size,
    before: (&Pair, &Instant),
    after: (&Pair, &Instant),
    options: &Options,
    correct: Option<Readout>,
) -> Vec<Row> {
    let step = options.step.to_radians();
    let half = (options.span.to_radians() / 2.0 / step) as isize;
    let search = (options.search.to_radians() / step) as isize;
    let held = |when: &Instant| Held {
        body_from_world: when.world_from_body.conjugate(),
        rolling: correct.map(|readout| Rolling {
            turn: when.turn,
            axis: readout.sweep.axis(),
        }),
    };
    let maps = [before.1, after.1].map(|when| {
        Reframe::new(
            &calibration.lenses,
            frame,
            Camera::default(),
            held(when),
            1.0,
            false,
            Sampling::default(),
        )
    });

    let mut out = Vec::new();
    for lens in 0..calibration.lenses.len().min(2) {
        for index in 0..options.patches {
            let phi = index as f64 / options.patches as f64 * std::f64::consts::TAU;
            let (sin, cos) = phi.sin_cos();
            let theta = options.off_axis.to_radians();
            // A direction that far off this lens's own axis, in the body frame
            // of the earlier frame, and then in the world where both maps
            // address it.
            let body = match lens {
                0 => [theta.sin() * cos, theta.sin() * sin, theta.cos()],
                _ => [theta.sin() * cos, theta.sin() * sin, -theta.cos()],
            };
            let world = before.1.world_from_body.rotate(body);
            let (along, across) = tangents(world);

            let patches = [(&maps[0], &before.0), (&maps[1], &after.0)].map(|(map, pair)| {
                patch(
                    map,
                    &pair.lenses[lens],
                    lens,
                    world,
                    along,
                    across,
                    half + match std::ptr::eq(map, &maps[1]) {
                        true => search,
                        false => 0,
                    },
                    step,
                )
            });
            let [Some(first), Some(second)] = patches else {
                continue;
            };
            if first.contrast() < options.contrast {
                continue;
            }
            let Some((di, dj, agreement)) = first.best_shift(&second, search) else {
                continue;
            };
            if agreement < options.correlation {
                continue;
            }
            // A rotation r displaces this direction by r x w, and the two
            // components are that displacement along the patch's own axes.
            let component = |rotation: [f64; 3]| {
                let moved = cross(rotation, world);
                [
                    dot(moved, along).to_degrees(),
                    dot(moved, across).to_degrees(),
                ]
            };
            let predicted = [Sweep::Right, Sweep::Left, Sweep::Down, Sweep::Up].map(|sweep| {
                let axis = sweep.axis();
                let share = |when: &(&Pair, &Instant), map: &Reframe| {
                    let landing = map.project(lens, world.map(|c| c as f32));
                    let pixel = [
                        f64::from(landing.pixel[0]) / f64::from(when.0.lenses[lens].size.width),
                        f64::from(landing.pixel[1]) / f64::from(when.0.lenses[lens].size.height),
                    ];
                    ((pixel[0] - 0.5) * axis[0] + (pixel[1] - 0.5) * axis[1]).clamp(-0.5, 0.5)
                };
                let turn_of = |when: &(&Pair, &Instant), map: &Reframe| {
                    let scaled = when.1.turn.map(|c| c * share(when, map));
                    when.1.world_from_body.rotate(scaled)
                };
                component(std::array::from_fn(|axis| {
                    turn_of(&after, &maps[1])[axis] - turn_of(&before, &maps[0])[axis]
                }))
            });
            // A translation displaces a direction by its own component
            // across that direction, which is the flow field a moving camera
            // draws over far-off content.
            let sideways = |axis: usize| {
                let mut moved = [0.0; 3];
                moved[axis] = 1.0;
                let along_view = dot(moved, world);
                let across_view: [f64; 3] =
                    std::array::from_fn(|at| moved[at] - along_view * world[at]);
                [
                    dot(across_view, along).to_degrees(),
                    dot(across_view, across).to_degrees(),
                ]
            };
            out.push(Row {
                lens,
                measured: [
                    (di as f64 * step).to_degrees(),
                    (dj as f64 * step).to_degrees(),
                ],
                rigid: std::array::from_fn(|axis| {
                    let mut unit = [0.0; 3];
                    unit[axis] = 1.0;
                    component(unit)
                }),
                flow: std::array::from_fn(sideways),
                predicted,
            });
        }
    }
    out
}

/// Two perpendicular directions in the sphere's surface at `w`.
fn tangents(w: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let aside = match w[1].abs() < 0.9 {
        true => [0.0, 1.0, 0.0],
        false => [1.0, 0.0, 0.0],
    };
    let along = unit(cross(aside, w));
    (along, unit(cross(w, along)))
}

/// How much of each axis's predicted displacement is in the measurements,
/// with the lock's own rigid rotation fitted out alongside.
///
/// Eight unknowns, least squares, and six of them are there to be thrown
/// away. Three are the **rotation** between the two frames, which every patch
/// shares: the horizon lock leaves one, and a rotation left in would be read
/// as a readout. Three more are the camera's own **translation**, whose flow
/// field over far-off content is a dipole and looks enough like a readout's to
/// be worth taking out; its amplitude carries the unknown distance with it.
/// The last two are how far the readout sweeps across the delivered frame and
/// how far down it, in units of a sweep that crosses the whole frame in the
/// time the trailer gives. **Those two are fitted together**, because a fit to
/// one axis alone reads the other's displacement as its own wherever the two
/// predictors lean the same way, and on a real frame they do.
///
/// So the answer comes out as a direction and a size: `(1, 0)` is a sensor
/// read left to right across the delivered picture in exactly the trailer's
/// time, `(-1, 0)` right to left, `(0, 1)` top to bottom, and `(0, 0)` is a
/// picture with no readout displacement in it at all.
fn fit(rows: &[Row]) -> Fit {
    const TERMS: usize = 8;
    let mut normal = [[0.0f64; TERMS + 1]; TERMS];
    let design = |row: &Row, component: usize| {
        [
            row.rigid[0][component],
            row.rigid[1][component],
            row.rigid[2][component],
            row.flow[0][component],
            row.flow[1][component],
            row.flow[2][component],
            row.predicted[0][component],
            row.predicted[2][component],
        ]
    };
    for row in rows {
        for component in 0..2 {
            let terms = design(row, component);
            for (i, left) in terms.iter().enumerate() {
                for (j, right) in terms.iter().enumerate() {
                    normal[i][j] += left * right;
                }
                normal[i][TERMS] += left * row.measured[component];
            }
        }
    }
    let Some(solved) = solve(normal) else {
        return Fit::default();
    };
    let mut residual = 0.0;
    let mut count = 0.0f64;
    for row in rows {
        for component in 0..2 {
            let modelled: f64 = design(row, component)
                .iter()
                .zip(solved)
                .map(|(term, weight)| term * weight)
                .sum();
            residual += (row.measured[component] - modelled).powi(2);
            count += 1.0;
        }
    }
    let residual = (residual / count.max(1.0)).sqrt();
    // The standard error of each sweep term, from the residual and the
    // leverage that term keeps after the other four have taken theirs.
    let error = |term: usize| {
        let leverage: f64 = rows
            .iter()
            .flat_map(|row| [design(row, 0)[term], design(row, 1)[term]])
            .map(|value| value * value)
            .sum();
        match leverage > 0.0 {
            true => residual / leverage.sqrt(),
            false => 0.0,
        }
    };
    Fit {
        sweep: [solved[6], solved[7]],
        error: [error(6), error(7)],
        residual,
    }
}

/// What one run of patch readings says the sensor's readout is.
#[derive(Default, Clone, Copy)]
struct Fit {
    /// Across the delivered frame and down it, in whole-frame readouts.
    sweep: [f64; 2],
    error: [f64; 2],
    /// What the fit does not explain, in degrees.
    residual: f64,
}

impl std::fmt::Display for Fit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:>9.2} +-{:<6.2} {:>9.2} +-{:<6.2} {:>10.3}",
            self.sweep[0], self.error[0], self.sweep[1], self.error[1], self.residual,
        )
    }
}

/// Gauss-Jordan on a small normal system, or `None` where it is singular.
fn solve(mut system: [[f64; 9]; 8]) -> Option<[f64; 8]> {
    for column in 0..8 {
        let pivot = (column..8).max_by(|a, b| {
            system[*a][column]
                .abs()
                .total_cmp(&system[*b][column].abs())
        })?;
        if system[pivot][column].abs() < 1e-12 {
            return None;
        }
        system.swap(column, pivot);
        let scale = system[column][column];
        for value in &mut system[column] {
            *value /= scale;
        }
        for row in 0..8 {
            if row == column {
                continue;
            }
            let factor = system[row][column];
            let taken = system[column];
            for (value, above) in system[row].iter_mut().zip(taken) {
                *value -= factor * above;
            }
        }
    }
    Some(std::array::from_fn(|row| system[row][8]))
}

fn norm2(v: [f64; 2]) -> f64 {
    v[0].hypot(v[1])
}

/// Where in the file the camera **rolls** hardest, which is the only motion
/// the seam measurement can see.
///
/// Along the seam is where parallax cannot reach, which is what makes that
/// axis worth measuring at all (docs/research/insv-format.md 4.9), and a turn
/// about the lens axis is the only one that displaces a seam direction along
/// it: yaw and pitch move the seam circle across itself, where parallax
/// already lives. So the ranking is by the roll component of the turn, and the
/// total is printed beside it.
fn fastest(track: &OrientationTrack, readout: Readout, count: usize) -> Fallible<()> {
    let span = (readout.seconds * 1e6) as i64;
    let mut instants: Vec<(f64, f64, f64)> = track
        .samples()
        .iter()
        .step_by(4)
        .map(|sample| {
            let at = sample.offset_us;
            let turn = track.turn(at - span / 2, at + span / 2);
            (
                turn[2].abs().to_degrees() / readout.seconds,
                norm(turn).to_degrees() / readout.seconds,
                at as f64 * 1e-6,
            )
        })
        .collect();
    instants.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!("the {count} hardest rolls, one per second at most:");
    let mut taken: Vec<f64> = Vec::new();
    for (roll, rate, at) in instants {
        if taken.len() >= count {
            break;
        }
        if taken.iter().any(|held| (held - at).abs() < 1.0) {
            continue;
        }
        println!("  from={at:<9.2} roll {roll:6.0} deg/s of {rate:6.0} total");
        taken.push(at);
    }
    Ok(())
}

// ------------------------------------------------------------ the candidates

/// The readouts compared on the same pixels: the correction switched off, and
/// every direction the sensor could be read in.
///
/// Three of the four are wrong by construction, which is what makes this a
/// measurement rather than a demonstration: if the instrument cannot tell them
/// apart it has not measured anything.
fn candidates(readout: Readout) -> Vec<(String, Option<Readout>)> {
    let mut out = vec![("off".to_owned(), None)];
    for sweep in [Sweep::Right, Sweep::Left, Sweep::Down, Sweep::Up] {
        out.push((name(sweep).to_owned(), Some(Readout { sweep, ..readout })));
    }
    out
}

/// The four readouts injected as controls: one per axis and sign, because an
/// instrument that answers on two axes has to be shown to read both.
const INJECTED: [Sweep; 4] = [Sweep::Right, Sweep::Left, Sweep::Down, Sweep::Up];

/// Which fitted axis a direction lives on, and which way round: injecting it
/// takes that coefficient down by one of its own sign, so a control reads back
/// at 1.00 whichever of the four it is.
fn reads(sweep: Sweep) -> (usize, f64) {
    match sweep {
        Sweep::Right => (0, 1.0),
        Sweep::Left => (0, -1.0),
        Sweep::Down => (1, 1.0),
        Sweep::Up => (1, -1.0),
        Sweep::Unknown => (0, 0.0),
    }
}

fn name(sweep: Sweep) -> &'static str {
    match sweep {
        Sweep::Unknown => "unknown",
        Sweep::Right => "right",
        Sweep::Left => "left",
        Sweep::Down => "down",
        Sweep::Up => "up",
    }
}

/// The map for one frame under one candidate: the camera left alone and the
/// horizon unlocked, so a view ray is a direction in the camera body's own
/// frame and a patch of the sphere is addressed by its angles.
fn mapped(calibration: &CalibrationSet, frame: Size, rolling: Option<Rolling>) -> Reframe {
    Reframe::new(
        &calibration.lenses,
        frame,
        Camera::default(),
        Held {
            body_from_world: Quat::IDENTITY,
            rolling,
        },
        1.0,
        false,
        Sampling::default(),
    )
}

/// How the body turned across one frame's readout, as the pass computes it.
fn rolling(track: &OrientationTrack, at: i64, readout: Option<Readout>) -> Option<Rolling> {
    let readout = readout?;
    let span = (readout.seconds * 1e6) as i64;
    Some(Rolling {
        turn: track.turn(at - span / 2, at + span / 2),
        axis: readout.sweep.axis(),
    })
}

// ------------------------------------------------------------ the seam

/// The along-seam residual, under every candidate, on the same frames and the
/// same patches.
///
/// **The same patches is load-bearing.** A candidate that moves the picture
/// also moves which patches correlate at all, and comparing a candidate's good
/// patches against another's would rank the gate rather than the candidate. So
/// a patch counts only where every candidate found it, which is the paired
/// comparison the question needs: one set of directions, five ways of reading
/// the sensor, one frame's pixels.
fn seam(
    calibration: &CalibrationSet,
    track: &OrientationTrack,
    readout: Readout,
    options: &Options,
) -> Fallible<()> {
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let candidates = candidates(readout);
    let mut walk = Walk::open(&options.input, options.from, calibration.dimension)?;
    if walk.streams() < 2 {
        return Err("this file carries one lens stream, so it has no seam".into());
    }
    let mut runs: Vec<Run> = candidates.iter().map(|(name, _)| Run::new(name)).collect();
    let mut uncorrected: Vec<Found> = Vec::new();
    let mut measured = 0usize;

    println!(
        "seam:   {} patches, {:.1} degrees across, correlated over +/-{:.1} degrees in {:.2} \
         degree steps, kept above {:.2}",
        options.patches, options.span, options.search, options.step, options.correlation,
    );
    println!(
        "{:<8} {:>9} {:>7}   {}",
        "frame",
        "roll d/s",
        "shared",
        candidates
            .iter()
            .map(|(name, _)| format!("{name:>7}"))
            .collect::<Vec<_>>()
            .join(" "),
    );

    for _ in 0..options.count {
        let Some(pair) = walk.next_pair()? else {
            break;
        };
        let at = calibration.exposure[0]
            .frame_time_us(pair.index)
            .unwrap_or((pair.at.as_micros() as i64).max(0));
        let turn = rolling(track, at, Some(readout)).map_or([0.0; 3], |r| r.turn);
        // The roll component, which is the only one that displaces a seam
        // direction along the seam.
        let rate = turn[2].abs().to_degrees() / readout.seconds;

        // Every candidate over the same pixels, then the patches all of them
        // agree are patches.
        let found: Vec<Vec<Option<Found>>> = candidates
            .iter()
            .map(|(_, candidate)| {
                let reframe = mapped(calibration, frame, rolling(track, at, *candidate));
                measure(&reframe, &pair, turn, options)
            })
            .collect();
        let shared: Vec<usize> = (0..options.patches)
            .filter(|patch| {
                found.iter().all(|candidate| {
                    candidate[*patch].is_some_and(|f| f.agreement >= options.correlation)
                })
            })
            .collect();

        let mut row = format!("{:<8} {:>9.1} {:>7}  ", pair.index, rate, shared.len());
        for (index, (candidate, run)) in found.iter().zip(&mut runs).enumerate() {
            let kept: Vec<Found> = shared
                .iter()
                .filter_map(|patch| candidate[*patch])
                .collect();
            row.push_str(&format!(" {:>7.2}", spread(kept.iter().map(|f| f.along))));
            if index == 0 {
                uncorrected.extend(kept.iter().copied());
            }
            for (patch, found) in shared.iter().zip(&kept) {
                run.take(*patch, rate, *found);
            }
        }
        println!("{row}");
        measured += 1;
    }
    if measured == 0 {
        return Err("no frames were decoded at all".into());
    }

    println!("\nthe row above is each candidate's along-seam spread on that frame, in degrees\n");
    println!(
        "{:<8} {:>8} {:>10} {:>12} {:>12} {:>12} {:>10}",
        "sweep", "patches", "mean deg", "spread", "within patch", "within fast", "across"
    );
    for run in &runs {
        println!("{run}");
    }
    println!(
        "\nspread is over every reading; within patch is with each patch's own mean taken off, \n\
         which is what leaves the part that moves with the camera rather than with the \n\
         calibration. fast is the half of those readings taken on the hardest-rolling frames."
    );

    // And the direct answer, off the uncorrected pictures alone: how much of
    // what moves frame to frame is the shape a readout would leave.
    println!(
        "\nthe uncorrected disagreement against what each candidate axis predicts, within \n\
         patches, {} readings of {} patches seen at least {APPEARANCES} times:",
        runs[0].within().len(),
        uncorrected.len(),
    );
    println!(
        "{:<10} {:>10} {:>12} {:>12}",
        "axis", "slope", "r", "residual"
    );
    let within = runs[0].within();
    for (axis, name) in [(1, "across x"), (2, "down y")] {
        let (slope, r, residual) = regression(&within, axis);
        println!("{name:<10} {slope:>10.3} {r:>12.3} {residual:>12.3}");
    }
    println!(
        "\na slope of 1 is a readout that sweeps the way the prediction is written and takes the \n\
         whole frame time to do it; a negative one sweeps the other way; zero is an axis the \n\
         readout does not run along. residual is the along-seam spread left after the fit."
    );

    // The positive control, without which the line above is not a result.
    //
    // Applying a candidate displaces the two lenses' pictures by exactly the
    // amount that candidate predicts, so the instrument has to read that
    // displacement back at minus one: a correction moves the disagreement by
    // the same amount it would have removed, the other way. If it does, this
    // instrument can see a displacement of that size and shape on these
    // pixels, and a slope of zero above is the picture having none.
    println!("\nthe control: each candidate's own displacement, read back off the pictures");
    println!(
        "{:<10} {:>10} {:>12} {:>12}",
        "candidate", "slope", "r", "predicted"
    );
    for (candidate, run) in candidates.iter().zip(&runs).skip(1) {
        let (axis, sign) = match candidate.0.as_str() {
            "right" => (1, 1.0),
            "left" => (1, -1.0),
            "down" => (2, 1.0),
            _ => (2, -1.0),
        };
        let moved: Vec<[f64; 4]> = run
            .samples
            .iter()
            .zip(&runs[0].samples)
            .map(|((_, rate, on), (_, _, off))| {
                [
                    on.along - off.along,
                    sign * on.predicted[0],
                    sign * on.predicted[1],
                    *rate,
                ]
            })
            .collect();
        let (slope, r, _) = regression(&moved, axis);
        println!(
            "{:<10} {slope:>10.3} {r:>12.3} {:>12.3}",
            candidate.0,
            spread(moved.iter().map(|row| row[axis])),
        );
    }
    println!(
        "\npredicted is the standard deviation of the displacement that candidate applies, in \n\
         degrees: it is the size of effect the row above proves this instrument can read."
    );
    Ok(())
}

/// One patch's answer: where lens 1's picture of the same directions sits
/// relative to lens 0's.
#[derive(Clone, Copy, Debug)]
struct Found {
    /// Degrees along the seam circle, towards increasing azimuth. Parallax
    /// cannot reach this axis: the baseline between the lenses is
    /// perpendicular to every direction on the seam, so a subject's distance
    /// displaces it across the seam and never along it.
    along: f64,
    /// Degrees across the seam, which parallax owns and which is reported
    /// rather than argued about.
    across: f64,
    agreement: f64,
    /// What a readout running across the delivered frame predicts this
    /// patch's along-seam disagreement to be, in degrees, for a readout that
    /// spans the whole frame in the time the trailer gives. Its sign is the
    /// direction of the sweep and its scale is the span, so the slope of the
    /// measurement against it is both answers at once.
    predicted: [f64; 2],
}

/// Every patch round the seam of one frame, under one map, in patch order:
/// `None` where this map has no usable picture of that patch.
fn measure(
    reframe: &Reframe,
    pair: &Pair,
    turn: [f64; 3],
    options: &Options,
) -> Vec<Option<Found>> {
    let frame_edge = [pair.lenses[0].size.width, pair.lenses[0].size.height];
    let step = options.step.to_radians();
    let half = (options.span.to_radians() / 2.0 / step) as isize;
    let search = (options.search.to_radians() / step) as isize;

    (0..options.patches)
        .map(|index| {
            let phi = index as f64 / options.patches as f64 * std::f64::consts::TAU;
            let (sin, cos) = phi.sin_cos();
            // A direction on the seam great circle, and the two axes of the
            // sphere at it: along the circle, and across it towards the front
            // lens. Both are unit and perpendicular by construction.
            let centre = [cos, sin, 0.0];
            let along = [-sin, cos, 0.0];
            let across = [0.0, 0.0, 1.0];

            // Where each lens reads this direction, and what that is worth:
            // the two lenses' shares of their own readouts differ, so the
            // displacement does not cancel between them, and the angle it
            // comes to is the turn crossed into the direction, along the seam.
            let landings = [0, 1].map(|lens| reframe.project(lens, centre.map(|c| c as f32)));
            let leverage = dot(along, cross(turn, centre));
            let predicted = [0, 1].map(|axis| {
                let share = |lens: usize| {
                    f64::from(landings[lens].pixel[axis]) / f64::from(frame_edge[axis]) - 0.5
                };
                ((share(1) - share(0)) * leverage).to_degrees()
            });

            let front = patch(
                reframe,
                &pair.lenses[0],
                0,
                centre,
                along,
                across,
                half,
                step,
            )?;
            let back = patch(
                reframe,
                &pair.lenses[1],
                1,
                centre,
                along,
                across,
                half + search,
                step,
            )?;
            if front.contrast() < options.contrast {
                return None;
            }
            let (di, dj, agreement) = front.best_shift(&back, search)?;
            Some(Found {
                along: (di as f64 * step).to_degrees(),
                across: (dj as f64 * step).to_degrees(),
                agreement,
                predicted,
            })
        })
        .collect()
}

/// One lens's picture of a square of the sphere, sampled on a grid of
/// directions rather than of pixels: `2 * half + 1` a side, `step` radians
/// apart, laid out along then across.
struct Patch {
    half: isize,
    luma: Vec<f64>,
}

#[allow(clippy::too_many_arguments)]
fn patch(
    reframe: &Reframe,
    plane: &Plane,
    lens: usize,
    centre: [f64; 3],
    along: [f64; 3],
    across: [f64; 3],
    half: isize,
    step: f64,
) -> Option<Patch> {
    let side = (2 * half + 1) as usize;
    let mut luma = Vec::with_capacity(side * side);
    for i in -half..=half {
        for j in -half..=half {
            let (a, b) = (i as f64 * step, j as f64 * step);
            let ray = unit(std::array::from_fn(|axis| {
                centre[axis] + along[axis] * a + across[axis] * b
            }));
            let landing = reframe.project(lens, ray.map(|c| c as f32));
            // A patch with any corner outside this lens's picture is not a
            // patch: the two lenses have to be answering about the same
            // directions or the correlation means nothing.
            if !landing.inside {
                return None;
            }
            luma.push(plane.at(f64::from(landing.pixel[0]), f64::from(landing.pixel[1]))?);
        }
    }
    Some(Patch { half, luma })
}

impl Patch {
    fn side(&self) -> isize {
        2 * self.half + 1
    }

    fn at(&self, i: isize, j: isize) -> f64 {
        self.luma[((i + self.half) * self.side() + (j + self.half)) as usize]
    }

    /// How much picture there is to correlate, in 8-bit codes. Flat sky
    /// correlates with anything.
    fn contrast(&self) -> f64 {
        let count = self.luma.len() as f64;
        let mean = self.luma.iter().sum::<f64>() / count;
        (self.luma.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count).sqrt()
    }

    /// The shift, in grid steps, that lines `other`'s picture up with this
    /// one, and how well it correlates there.
    ///
    /// `other` is sampled `search` steps wider in both axes, so every shift is
    /// a whole-step lookup into it and nothing is interpolated twice.
    fn best_shift(&self, other: &Patch, search: isize) -> Option<(isize, isize, f64)> {
        let mut best: Option<(isize, isize, f64)> = None;
        for di in -search..=search {
            for dj in -search..=search {
                let agreement = self.correlation(other, di, dj);
                if best.is_none_or(|(_, _, held)| agreement > held) {
                    best = Some((di, dj, agreement));
                }
            }
        }
        best
    }

    /// Zero-mean normalized cross-correlation between this patch and `other`
    /// shifted by `(di, dj)`.
    fn correlation(&self, other: &Patch, di: isize, dj: isize) -> f64 {
        let mut pairs = Vec::with_capacity(self.luma.len());
        for i in -self.half..=self.half {
            for j in -self.half..=self.half {
                pairs.push((self.at(i, j), other.at(i + di, j + dj)));
            }
        }
        let count = pairs.len() as f64;
        let mean = |pick: fn(&(f64, f64)) -> f64| pairs.iter().map(pick).sum::<f64>() / count;
        let (mean_a, mean_b) = (mean(|p| p.0), mean(|p| p.1));
        let mut covariance = 0.0;
        let (mut var_a, mut var_b) = (0.0, 0.0);
        for (a, b) in &pairs {
            let (a, b) = (a - mean_a, b - mean_b);
            covariance += a * b;
            var_a += a * a;
            var_b += b * b;
        }
        match var_a > 0.0 && var_b > 0.0 {
            true => covariance / (var_a * var_b).sqrt(),
            false => 0.0,
        }
    }
}

/// What one candidate came to over the whole run.
///
/// Kept per patch rather than pooled, because the two things in an along-seam
/// residual separate that way and no other. A patch's own calibration residual
/// is **the same in every frame**; a readout displacement is not, because it
/// scales with how fast the camera was rolling at that instant and reverses
/// when the roll does. So taking each patch's own mean off its readings leaves
/// exactly the part that moves with the motion, which is the only part a
/// readout correction can touch.
struct Run {
    name: String,
    /// Patch index, the roll rate of the frame it was measured on, and what
    /// the correlation found there.
    samples: Vec<(usize, f64, Found)>,
}

/// How many frames a patch has to appear in before its own mean means
/// anything.
const APPEARANCES: usize = 4;

impl Run {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            samples: Vec::new(),
        }
    }

    fn take(&mut self, patch: usize, rate: f64, found: Found) {
        self.samples.push((patch, rate, found));
    }

    /// Every reading with its own patch's mean taken off it, as
    /// `(measured, predicted across x, predicted down y, rate)`.
    fn within(&self) -> Vec<[f64; 4]> {
        let mut out = Vec::new();
        for patch in 0..=self
            .samples
            .iter()
            .map(|(patch, _, _)| *patch)
            .max()
            .unwrap_or(0)
        {
            let mine: Vec<&(usize, f64, Found)> = self
                .samples
                .iter()
                .filter(|(at, _, _)| *at == patch)
                .collect();
            if mine.len() < APPEARANCES {
                continue;
            }
            let count = mine.len() as f64;
            let mean_of = |pick: &dyn Fn(&Found) -> f64| {
                mine.iter().map(|(_, _, found)| pick(found)).sum::<f64>() / count
            };
            let (mean_along, mean_x, mean_y) = (
                mean_of(&|f: &Found| f.along),
                mean_of(&|f: &Found| f.predicted[0]),
                mean_of(&|f: &Found| f.predicted[1]),
            );
            for (_, rate, found) in mine {
                out.push([
                    found.along - mean_along,
                    found.predicted[0] - mean_x,
                    found.predicted[1] - mean_y,
                    *rate,
                ]);
            }
        }
        out
    }
}

impl std::fmt::Display for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let within = self.within();
        let fast: Vec<f64> = {
            let mut rates: Vec<f64> = within.iter().map(|row| row[3]).collect();
            rates.sort_by(f64::total_cmp);
            let median = rates.get(rates.len() / 2).copied().unwrap_or(0.0);
            within
                .iter()
                .filter(|row| row[3] >= median)
                .map(|row| row[0])
                .collect()
        };
        write!(
            f,
            "{:<8} {:>8} {:>10.3} {:>12.3} {:>12.3} {:>12.3} {:>10.3}",
            self.name,
            self.samples.len(),
            mean(self.samples.iter().map(|(_, _, found)| found.along)),
            spread(self.samples.iter().map(|(_, _, found)| found.along)),
            spread(within.iter().map(|row| row[0])),
            spread(fast.into_iter()),
            spread(self.samples.iter().map(|(_, _, found)| found.across)),
        )
    }
}

// ------------------------------------------------------------ the model

/// The numbers behind the shape of the correction, off the gyro track alone.
fn model(
    calibration: &CalibrationSet,
    track: &OrientationTrack,
    readout: Readout,
    options: &Options,
) -> Fallible<()> {
    let span = (readout.seconds * 1e6) as i64;
    let first = track.samples().first().map_or(0, |s| s.offset_us);
    let last = track.samples().last().map_or(0, |s| s.offset_us);
    let instants: Vec<i64> = (0..options.instants)
        .map(|step| first + (last - first) * step as i64 / options.instants as i64)
        .collect();

    // How fast this file turns is the `carries` line, printed for every mode.

    // A straight line in time across the readout, against the track's own
    // orientation at each row's instant. This is the whole modelling
    // assumption: one rotation vector per frame instead of one lookup per row.
    let mut line = Vec::with_capacity(instants.len());
    let mut gyro = Vec::with_capacity(instants.len());
    let mut flipped = Vec::with_capacity(instants.len());
    let to_body = calibration.body_from_imu();
    for at in &instants {
        let turn = track.turn(at - span / 2, at + span / 2);
        let mut worst = 0.0f64;
        for step in -10..=10 {
            let share = f64::from(step) / 20.0;
            let modelled = Quat::from_rotation_vector(turn.map(|axis| axis * share));
            let truth = track
                .at(at + (span as f64 * share) as i64)
                .conjugate()
                .times(track.at(*at));
            worst = worst.max(modelled.angle_to(truth).to_degrees());
        }
        line.push(worst);
        // And the filtered track's own turn against the gyroscope's, which is
        // the other thing being assumed: that a rotation read off a stabilized
        // orientation is the camera's real motion and not the filter's. A
        // sign error here would read as twice the turn itself, which at these
        // rates is degrees rather than hundredths.
        let raw = raw_turn(calibration, to_body, *at, span);
        gyro.push(
            Quat::from_rotation_vector(turn)
                .angle_to(Quat::from_rotation_vector(raw))
                .to_degrees(),
        );
        // The negative control for the one sign in the correction: read the
        // other way round, the same two turns disagree by twice the turn
        // itself, which is degrees where the agreement above is hundredths.
        flipped.push(
            Quat::from_rotation_vector(turn)
                .angle_to(Quat::from_rotation_vector(raw.map(std::ops::Neg::neg)))
                .to_degrees(),
        );
    }
    report(
        "line",
        "a straight turn across the readout against the track's own orientation at each row",
        &line,
    );
    report(
        "gyro",
        "the stabilized track's turn against the raw gyroscope's over the same window",
        &gyro,
    );
    report(
        "flipped",
        "the same comparison with the turn read the other way round, which is the control",
        &flipped,
    );

    convergence(calibration, track, readout);
    bend(calibration, readout, options);
    Ok(())
}

/// One column of degrees, as the spread of it rather than the worst of it: a
/// worst case on 30 minutes of vibrating airframe is one bump.
fn report(name: &str, what: &str, values: &[f64]) {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let at = |p: f64| sorted[((sorted.len().max(1) - 1) as f64 * p) as usize];
    println!(
        "{name:<7} median {:.4}, 90th {:.4}, 99th {:.4}, worst {:.4} degrees: {what}",
        at(0.5),
        at(0.9),
        at(0.99),
        at(1.0),
    );
}

/// How far an uncorrected readout bends a horizon, in pixels of a rendered
/// view: the sensitivity the horizon harness has to have to say anything.
///
/// A great circle projects to a straight line in a rectilinear view, so a
/// bend in one is the picture's and not the world's. The readout draws it:
/// content is displaced by the turn the camera made between the middle row of
/// the sensor and the row that content came off, which grows along the
/// picture, and a displacement that grows along a line curves it.
///
/// This is the prediction `kyerag-spike --bin horizon readout=off` is measured
/// against. It is computed through the shipped map rather than by a formula,
/// so what it predicts is what the pass would do.
fn bend(calibration: &CalibrationSet, readout: Readout, options: &Options) {
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let (width, height) = (960.0f64, 540.0f64);
    let fov = 100f64.to_radians();
    let scale = width / 2.0 / (fov / 2.0).tan();

    println!(
        "bend:   what an uncorrected readout does to a horizon across a {width:.0}x{height:.0} \
         view at {:.0} degrees",
        fov.to_degrees()
    );
    println!("{:<10} {:>12} {:>12}", "roll d/s", "sweep px", "bend px");
    for rate in [50.0f64, 100.0, 200.0, 400.0] {
        // Rolling about the lens axis, which is the motion a seam-blind
        // readout still shows inside one lens.
        let turn = [0.0, 0.0, (rate * readout.seconds).to_radians()];
        // Explicitly across the frame rather than through the file's own
        // sweep, which is `Unknown` and would predict nothing: this table is
        // what a readout of the trailer's length would do if it ran that way.
        let reframe = mapped(
            calibration,
            frame,
            Some(Rolling {
                turn,
                axis: Sweep::Right.axis(),
            }),
        );
        // Along the middle of the view, which is where a level horizon runs.
        let points: Vec<(f64, f64)> = (0..=64)
            .map(|step| {
                let u = f64::from(step) / 64.0 - 0.5;
                let ray = [(u * width / scale) as f32, 0.0, 1.0];
                let ahead = normalize(ray);
                let landing = reframe.project(0, ahead);
                let share = f64::from(reframe.readout_share(landing.pixel));
                // The displacement this row's own instant leaves, as an angle,
                // and then in pixels of this view: a rotation moves a
                // direction by the rotation crossed into it.
                let moved = cross(turn.map(|axis| axis * share), ahead.map(f64::from));
                let along_view = 1.0 + (u * width / scale).powi(2);
                (u * width, moved[1] * scale * along_view)
            })
            .collect();
        // A straight line through them, and what is left over, which is the
        // bend: a uniform displacement is a shift and a linear one is a tilt,
        // and neither of those is a bend.
        let count = points.len() as f64;
        let mean_x = points.iter().map(|(x, _)| x).sum::<f64>() / count;
        let mean_y = points.iter().map(|(_, y)| y).sum::<f64>() / count;
        let covariance: f64 = points
            .iter()
            .map(|(x, y)| (x - mean_x) * (y - mean_y))
            .sum();
        let variance: f64 = points.iter().map(|(x, _)| (x - mean_x).powi(2)).sum();
        let slope = covariance / variance.max(f64::MIN_POSITIVE);
        let bend = (points
            .iter()
            .map(|(x, y)| (y - mean_y - slope * (x - mean_x)).powi(2))
            .sum::<f64>()
            / count)
            .sqrt();
        let sweep = points.iter().map(|(_, y)| y.abs()).fold(0.0f64, f64::max);
        println!("{rate:<10.0} {sweep:>12.2} {bend:>12.2}");
    }
    let _ = options;
}

/// The turn across one readout straight off the gyroscope, with no filter in
/// it: the rotation a world-fixed direction makes as seen from a body turning
/// at the measured rate, which is the negative of the body's own rotation.
fn raw_turn(
    calibration: &CalibrationSet,
    to_body: kyerag_meta::Mat3,
    at: i64,
    span: i64,
) -> [f64; 3] {
    let samples = calibration.imu.samples();
    let from = samples.partition_point(|s| s.offset_us < at - span / 2);
    let to = samples.partition_point(|s| s.offset_us < at + span / 2);
    let window = &samples[from.min(samples.len())..to.max(from).min(samples.len())];
    if window.is_empty() {
        return [0.0; 3];
    }
    let seconds = span as f64 * 1e-6;
    let mean: [f64; 3] = window.iter().fold([0.0; 3], |held, sample| {
        let rate = to_body.mul_vec(sample.rate_dps);
        std::array::from_fn(|axis| held[axis] + rate[axis] / window.len() as f64)
    });
    mean.map(|dps| -dps.to_radians() * seconds)
}

/// How many rounds the row solve takes to stop moving, in pixels of the
/// delivered frame, at this file's own worst rate.
///
/// The map is its own input: the row a ray lands on decides the instant its
/// orientation is read at, and that instant moves the row. Zero rounds is the
/// pass as it was before issue #9.
fn convergence(calibration: &CalibrationSet, track: &OrientationTrack, readout: Readout) {
    const SETTLED: usize = 8;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let span = (readout.seconds * 1e6) as i64;
    // The instant the file turns hardest, found by the same window the rest
    // of this uses.
    let at = track
        .samples()
        .iter()
        .map(|sample| sample.offset_us)
        .max_by_key(|at| {
            (norm(track.turn(at - span / 2, at + span / 2)).to_degrees() / readout.seconds * 1e3)
                as i64
        })
        .unwrap_or(0);
    // Across the frame explicitly: the file's own direction is not known, and
    // what is being measured here is how many rounds the solve takes when
    // there is a readout to solve for at all.
    let rolling = rolling(
        track,
        at,
        Some(Readout {
            sweep: Sweep::Right,
            ..readout
        }),
    );
    let rate = rolling.map_or(0.0, |r| norm(r.turn).to_degrees() / readout.seconds);
    let reframe = mapped(calibration, frame, rolling);

    println!(
        "solve:  the file's hardest instant is {rate:.0} deg/s, against a solve run {SETTLED} \
         rounds"
    );
    println!("{:<8} {:>12} {:>12}", "rounds", "worst px", "worst deg");
    for rounds in 0..4 {
        let mut worst = 0.0f64;
        for theta in (0..=180).step_by(5) {
            for phi in (0..360).step_by(15) {
                let ray = spherical(f64::from(theta), f64::from(phi));
                for lens in 0..calibration.lenses.len().min(2) {
                    let settled = reframe.solve(lens, ray, SETTLED);
                    if !settled.inside {
                        continue;
                    }
                    let landing = reframe.solve(lens, ray, rounds);
                    worst = worst.max(f64::from(
                        (landing.pixel[0] - settled.pixel[0])
                            .hypot(landing.pixel[1] - settled.pixel[1]),
                    ));
                }
            }
        }
        // The picture near the seam is about 15 px per degree, which is the
        // hardest place to be wrong in and the one issue #7 asks about.
        println!("{rounds:<8} {worst:>12.3} {:>12.4}", worst / 15.0);
    }
}

fn spherical(theta: f64, phi: f64) -> [f32; 3] {
    let (sin_theta, cos_theta) = theta.to_radians().sin_cos();
    let (sin_phi, cos_phi) = phi.to_radians().sin_cos();
    [
        (sin_theta * cos_phi) as f32,
        (sin_theta * sin_phi) as f32,
        cos_theta as f32,
    ]
}

// ------------------------------------------------------------ plumbing

struct Options {
    input: PathBuf,
    model: bool,
    from: f64,
    count: usize,
    patches: usize,
    /// How wide a patch is, in degrees of world angle.
    span: f64,
    /// How far the correlation looks for a shift, and how finely.
    search: f64,
    step: f64,
    correlation: f64,
    contrast: f64,
    instants: usize,
    find: Option<usize>,
    pair: bool,
    /// How far off a lens's own axis the frame-pair patches sit, in degrees.
    off_axis: f64,
    /// How many frames apart the two halves of a pair are taken.
    gap: usize,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Fallible<Self> {
        let input = PathBuf::from(args.next().ok_or(USAGE)?);
        let mut options = Self {
            input,
            model: false,
            from: 0.0,
            count: 12,
            patches: 36,
            span: 3.7,
            search: 2.0,
            step: 0.12,
            correlation: 0.85,
            contrast: 6.0,
            instants: 2000,
            find: None,
            pair: false,
            off_axis: 55.0,
            gap: 6,
        };
        for arg in args {
            let (key, value) = arg.split_once('=').ok_or(USAGE)?;
            match key {
                "model" => options.model = value.parse::<u32>()? != 0,
                "from" => options.from = value.parse()?,
                "count" => options.count = value.parse()?,
                "patches" => options.patches = value.parse()?,
                "span" => options.span = value.parse()?,
                "search" => options.search = value.parse()?,
                "step" => options.step = value.parse()?,
                "correlation" => options.correlation = value.parse()?,
                "contrast" => options.contrast = value.parse()?,
                "instants" => options.instants = value.parse()?,
                "find" => options.find = Some(value.parse()?),
                "pair" => options.pair = value.parse::<u32>()? != 0,
                "off_axis" => options.off_axis = value.parse()?,
                "gap" => options.gap = value.parse()?,
                _ => return Err(format!("unknown argument {key}. {USAGE}").into()),
            }
        }
        Ok(options)
    }
}

const USAGE: &str = "usage: rolling <file.insv> [model=1] [from=seconds] [count=frames] \
     [patches=n] [span=deg] [search=deg] [step=deg] [correlation=r] [contrast=codes] \
     [instants=n] [find=n] [pair=1] [off_axis=deg] [gap=frames]";

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let values: Vec<f64> = values.collect();
    match values.is_empty() {
        true => 0.0,
        false => values.iter().sum::<f64>() / values.len() as f64,
    }
}

/// A least-squares line through the measured disagreement against what one
/// candidate axis predicts, as the slope, the correlation coefficient, and
/// what is left over.
///
/// This is the measurement the whole instrument is for. The prediction is
/// written for a readout that sweeps one way and takes the whole frame time,
/// so the slope carries both answers: its **sign** is which way the sensor
/// reads, and its **size** is how much of the delivered frame the readout time
/// actually spans.
fn regression(within: &[[f64; 4]], axis: usize) -> (f64, f64, f64) {
    let count = within.len() as f64;
    if count < 3.0 {
        return (0.0, 0.0, 0.0);
    }
    let (mut covariance, mut var_x, mut var_y) = (0.0, 0.0, 0.0);
    for row in within {
        let (x, y) = (row[axis], row[0]);
        covariance += x * y;
        var_x += x * x;
        var_y += y * y;
    }
    if var_x <= 0.0 || var_y <= 0.0 {
        return (0.0, 0.0, spread(within.iter().map(|row| row[0])));
    }
    let slope = covariance / var_x;
    let residual = (within
        .iter()
        .map(|row| (row[0] - slope * row[axis]).powi(2))
        .sum::<f64>()
        / count)
        .sqrt();
    (slope, covariance / (var_x * var_y).sqrt(), residual)
}

/// The standard deviation about the mean, which is the statistic a readout
/// displacement moves: it varies round the seam circle, where the calibration
/// residual it sits on top of does not.
fn spread(values: impl Iterator<Item = f64>) -> f64 {
    let values: Vec<f64> = values.collect();
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt()
}

#[allow(dead_code)]
fn rms(values: impl Iterator<Item = f64>) -> f64 {
    let values: Vec<f64> = values.collect();
    match values.is_empty() {
        true => 0.0,
        false => (values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64).sqrt(),
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|axis| a[axis] * b[axis]).sum()
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let length = norm(v).max(f64::MIN_POSITIVE);
    v.map(|c| c / length)
}
