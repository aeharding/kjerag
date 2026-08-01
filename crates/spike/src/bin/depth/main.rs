//! The overlap band is a stereo pair: what it says about distance, and what
//! aligning the seam with it would take (issue #80, phase A).
//!
//! ```sh
//! # the disparity field, the geometry check and the controls
//! cargo run --release -p kjerag-spike --bin depth -- <file.insv> \
//!   fix=roll:0.839,yaw:-2.545,pitch:-0.627 control=1
//! # the alignment strategies, scored on the half of the band they never saw,
//! # and their flicker frame to frame
//! cargo run --release -p kjerag-spike --bin depth -- <file.insv> \
//!   fix=... mode=strategies count=16
//! # the prototype, rendered, with the disparity it applied beside it
//! cargo run --release -p kjerag-spike --bin depth -- <file.insv> \
//!   fix=... mode=render yaw=90 pitch=-60 out=scratch/depth
//! # our stitch with and without it, against the camera maker's own
//! cargo run --release -p kjerag-spike --bin depth -- <file.insv> \
//!   fix=... mode=parity against=<export.mp4> \
//!   look=yaw:88.7,pitch:1.2,roll:0.4,fov:95.3,compression:0.83
//! ```
//!
//! **What this is for.** Calibration has taken the seam as far as a static
//! warp goes: issue #48 fitted the camera's own 2.4 degree tilt out and left
//! about 0.4 degrees along the seam and 0.4 to 0.6 across it, and the across
//! column is where near-field content lives. It cannot be calibrated away,
//! because it is not an error: 33 mm of baseline at half a metre is 3.8
//! degrees of real parallax, and no rotation of a lens moves content that is
//! at two distances at once.
//!
//! **What is new.** The same 33 mm makes the overlap band a stereo pair. Both
//! lenses image those 20 degrees from two centres, so the disagreement there
//! is a disparity, the disparity is a distance, and the alignment follows from
//! the distance rather than from a fit. That is what the camera maker's own
//! "dynamic stitching" is.
//!
//! **What measuring it found.** Two things the plan did not have. The
//! one-dimensional search cannot run pinned at zero on the other axis, because
//! the calibration left 0.4 to 0.7 degrees there and the correlation follows
//! it ([`measure::Prealign`]); and a per-frame disparity **flickers**, 0.22 to
//! 0.54 degrees rms frame to frame on real footage, where a table pooled over
//! the clip aligns at least as well on six files of seven and cannot flicker
//! at all ([`strategy::Plan::pooled`]).
//!
//! **What this instrument does not do.** It changes no shader and no shipped
//! path. It reads the band, checks that what it reads is depth, scores the
//! ways of carrying that into the render, and writes pictures.
//! `crates/spike/src/bin/seam.rs` remains the one fitter in the tree; the
//! correction it produces is an input here (`fix=`), never a second fit.

mod band;
mod measure;
mod strategy;
mod view;

use std::path::{Path, PathBuf};

use kjerag_media::Fallible;
use kjerag_meta::Size as MetaSize;
use kjerag_render::Size;
use kjerag_spike::Walk;

use band::{Accumulator, Node, baseline, norm};
use measure::{Field, Prealign, body_up, deck, fixed, mapped, nodes, open, recover, sweep};
use strategy::{Plan, score};
use view::{Look, band_share, imported, paint};

const USAGE: &str =
    "usage: depth <file.insv> [mode=field|strategies|render|parity] [key=value ...]";

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    match options.mode {
        Mode::Field => field(&options),
        Mode::Strategies => strategies(&options),
        Mode::Render => render(&options),
        Mode::Parity => parity(&options),
    }
}

#[derive(Clone, Copy)]
enum Mode {
    /// What the band reads, whether it is depth, and how steady it is.
    Field,
    /// The three ways of carrying it into the render path, scored.
    Strategies,
    /// The prototype, drawn, so a correction can be looked at.
    Render,
    /// Ours with and without it, against the camera maker's own stitch.
    Parity,
}

// ------------------------------------------------------------ the field

fn field(options: &Options) -> Fallible<()> {
    let (calibration, pairs) = open(options, &options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = fixed(&calibration.lenses, &options.fix);
    let reframe = mapped(&lenses, frame);
    let t = baseline(&calibration);
    let grid = nodes(t, options);
    announce(options, &calibration, &grid);

    let prealign = align(options, &reframe, &grid, &pairs);
    let measured = sweep(&reframe, &grid, &pairs, options, 0.0, &prealign);
    println!(
        "frames: {} consecutive from {:.2} s, band read in {:.2} s\n\
         refused: {} outside a lens, {} too flat to correlate, {} peaked against the search limit",
        measured.frames.len(),
        options.from,
        measured.seconds,
        measured.outside,
        measured.flat,
        measured.pinned,
    );

    geometry(options, &reframe, &grid, &pairs, &measured, &prealign);
    report(options, &measured);
    trust(options, &measured);
    temporal(options, &measured);
    if options.control {
        controls(
            options,
            &reframe,
            &grid,
            &pairs,
            &calibration,
            &measured,
            &prealign,
        );
    }
    Ok(())
}

/// The per-file pre-alignment, fitted and announced.
///
/// It runs before every mode, because every reading below is taken through it.
fn align(
    options: &Options,
    reframe: &kjerag_render::Reframe,
    grid: &[Node],
    pairs: &[kjerag_spike::Pair],
) -> Prealign {
    if options.raw {
        println!("\nprealign: switched off, so the epipolar search runs pinned at zero");
        return Prealign::none(options.psis.len());
    }
    let one = &pairs[..1.min(pairs.len())];
    let fitted = Prealign::fit(reframe, grid, one, options);
    println!(
        "\nprealign: a free 2-D search on one frame read {} nodes on the axis depth cannot reach, \n\
         and a constant plus two cycles of the azimuth per row holds all but {:.3} deg of them. \n\
         that is calibration, not depth, and the epipolar search below runs offset by it.",
        fitted.read, fitted.residual_deg,
    );
    fitted
}

/// What the file is and what the band is, before anything is measured.
fn announce(options: &Options, calibration: &kjerag_meta::CalibrationSet, grid: &[Node]) {
    let t = baseline(calibration);
    println!(
        "\n{}: {} {}",
        options
            .input
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
        calibration.camera_model,
        calibration.firmware,
    );
    println!(
        "baseline: [{:+.6}, {:+.6}, {:+.6}] m, {:.2} mm, {:.2} deg off the body's z",
        t[0],
        t[1],
        t[2],
        norm(t) * 1e3,
        (t[0].hypot(t[1]) / t[2].abs()).atan().to_degrees(),
    );
    let reach = grid.first().map_or(0.0, |node| node.reach_m);
    println!(
        "band:   {} azimuths by {} rows at {} deg past the seam; {:.2} deg patches in {:.3} deg \
         steps, searched over disparities from {:+.1} to {:+.1} deg along the epipolar axis, \
         which is {:.2} m at the seam",
        options.phis,
        options.psis.len(),
        options
            .psis
            .iter()
            .map(|psi| format!("{psi:+.1}"))
            .collect::<Vec<_>>()
            .join(","),
        options.span,
        options.step,
        options.far,
        options.near,
        reach / options.near.to_radians(),
    );
}

/// The geometry check: is the disagreement in the band a disparity along the
/// axis the baseline names, or is it something else.
///
/// The instrument can fail this. A free two-dimensional search on the same
/// patches reads both axes, and no distance can put a signal on the
/// off-epipolar one. After the pre-alignment has taken the calibration out of
/// that axis, what is left there is the instrument's own floor on this scene's
/// own pixels: if the epipolar column stands well above it, the band is a
/// stereo pair with the baseline the file records. If both columns are the
/// same size, it is not, and every distance below is a coincidence.
fn geometry(
    options: &Options,
    reframe: &kjerag_render::Reframe,
    grid: &[Node],
    pairs: &[kjerag_spike::Pair],
    measured: &Field,
    prealign: &Prealign,
) {
    let skew: Vec<f64> = grid.iter().map(|node| node.skew_deg).collect();
    println!(
        "\ngeometry: the epipolar axis runs {:.2} to {:.2} deg off the across-seam tangent, which \n\
         is the baseline's own tilt out of the body's z. a search along the tangent instead \n\
         would misplace {:.1} percent of any disparity into the along-seam column.",
        skew.iter().copied().fold(f64::MAX, f64::min),
        skew.iter().copied().fold(f64::MIN, f64::max),
        100.0
            * skew
                .iter()
                .copied()
                .fold(f64::MIN, f64::max)
                .to_radians()
                .sin(),
    );
    let free = sweep(
        reframe,
        grid,
        &pairs[..1.min(pairs.len())],
        &Options {
            free: true,
            ..options.clone()
        },
        0.0,
        prealign,
    );
    let mut along = Accumulator::default();
    let mut across = Accumulator::default();
    let mut agreed = Accumulator::default();
    for node in 0..grid.len() {
        let Some(peak) = free.frames[0].peaks[node] else {
            continue;
        };
        if peak.r < options.keep {
            continue;
        }
        along.add(peak.perp.to_degrees());
        across.add(peak.epi.to_degrees());
        if let Some(constrained) = measured.frames[0].peaks[node] {
            agreed.add((constrained.epi - peak.epi).to_degrees());
        }
    }
    println!(
        "          measured on the pixels with a free 2-D search over {} nodes of one frame: \n\
         {:.3} deg rms along the epipolar axis against {:.3} deg rms across it, a ratio of \n\
         {:.1}. the 1-D search agrees with the free one to {:.3} deg rms.",
        across.count,
        across.rms(),
        along.rms(),
        match along.rms() > 0.0 {
            true => across.rms() / along.rms(),
            false => 0.0,
        },
        agreed.rms(),
    );
}

/// What the readings' own shape says about how far they can be trusted, which
/// is the failure-mode column of the strategy table.
fn trust(options: &Options, measured: &Field) {
    let found = measured.trust(options.keep);
    println!(
        "\ntrust: {} readings above r = {:.2} and {} below it. {} of them peak flat enough that a \n\
         hundredth of correlation moves the shift a whole step, which is what a repetitive \n\
         texture reads like; the median peak is {:.3} per step squared and the patches carry \n\
         {:.1} codes of contrast. flat sky is refused earlier, by contrast, and never reaches \n\
         this table.",
        found.kept,
        options.keep,
        found.weak,
        found.flat_peaks,
        found.median_curvature,
        found.contrast,
    );
}

/// The disparity field itself, one line per azimuth, at the seam row.
fn report(options: &Options, measured: &Field) {
    let middle = options
        .psis
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .map_or(0, |(index, _)| index);
    println!(
        "\nthe disparity field at the seam row, and the distance behind it. `view px` is the \n\
         disagreement a 1920-wide 90 degree view would show, at 16.8 px per degree.\n\n\
         {:>6} {:>10} {:>10} {:>9} {:>7} {:>7}",
        "phi", "disparity", "view px", "metres", "r", "frames"
    );
    let mut near = 0;
    let mut placed = 0;
    for phi in 0..options.phis {
        let index = phi * options.psis.len() + middle;
        let Some((shift, frames)) = measured.held(index, options.keep) else {
            continue;
        };
        let node = &measured.nodes[index];
        let metres = node.metres(shift);
        placed += 1;
        if metres < strategy::NEAR_M {
            near += 1;
        }
        let r = measured
            .track(index)
            .into_iter()
            .flatten()
            .map(|peak| peak.r)
            .fold(0.0f64, f64::max);
        println!(
            "{:>6.0} {:>9.3}d {:>10.1} {:>9.1} {:>7.3} {:>7}",
            node.phi.to_degrees(),
            shift.to_degrees(),
            shift.to_degrees() * 16.8,
            metres,
            r,
            frames,
        );
    }
    println!(
        "\n{placed} of {} azimuths placed at the seam row, {near} of them nearer than {:.0} m, \n\
         which is where a disparity is over a fifth of a degree and the blend stops hiding it.",
        options.phis,
        strategy::NEAR_M,
    );
    // Parallax is one-signed round the whole circle by construction: the
    // baseline points one way and a subject's distance displaces its picture
    // towards the front lens at every azimuth. A residual rotation is not.
    let signs: Vec<f64> = (0..options.phis)
        .filter_map(|phi| {
            measured
                .held(phi * options.psis.len() + middle, options.keep)
                .map(|(shift, _)| shift)
        })
        .collect();
    let positive = signs.iter().filter(|shift| **shift > 0.0).count();
    println!(
        "{positive} of {} read towards the front lens and {} the other way. parallax is \n\
         one-signed round the circle and a residual rotation is not, so a lopsided count is \n\
         depth and an even one is calibration.",
        signs.len(),
        signs.len() - positive,
    );
}

/// How steady the field is frame to frame, which is the failure mode a
/// per-frame warp is judged on.
fn temporal(options: &Options, measured: &Field) {
    if measured.frames.len() < 2 {
        println!("\ntemporal: one frame, so nothing to say about flicker");
        return;
    }
    let mut step = Accumulator::default();
    let mut spread = Accumulator::default();
    let mut worst: f64 = 0.0;
    for node in 0..measured.nodes.len() {
        let track: Vec<f64> = measured
            .track(node)
            .into_iter()
            .flatten()
            .filter(|peak| peak.r >= options.keep)
            .map(|peak| peak.epi.to_degrees())
            .collect();
        if track.len() < 2 {
            continue;
        }
        for pair in track.windows(2) {
            step.add(pair[1] - pair[0]);
            worst = worst.max((pair[1] - pair[0]).abs());
        }
        let mean = track.iter().sum::<f64>() / track.len() as f64;
        for value in &track {
            spread.add(value - mean);
        }
    }
    let span = measured
        .frames
        .last()
        .zip(measured.frames.first())
        .map_or(0.0, |(last, first)| last.at - first.at);
    println!(
        "\ntemporal: over {} consecutive frames spanning {:.2} s the field steps {:.3} deg rms \n\
         from one frame to the next, worst {:.3} deg, and each node's own spread about its mean \n\
         is {:.3} deg ({:.1} view px). that step is the flicker a per-frame warp would put in \n\
         the picture if nothing smoothed it.",
        measured.frames.len(),
        span,
        step.rms(),
        worst,
        spread.rms(),
        spread.rms() * 16.8,
    );
}

/// The two controls, both regime-sized.
fn controls(
    options: &Options,
    reframe: &kjerag_render::Reframe,
    grid: &[Node],
    pairs: &[kjerag_spike::Pair],
    calibration: &kjerag_meta::CalibrationSet,
    measured: &Field,
    prealign: &Prealign,
) {
    println!(
        "\nthe controls\n\n\
         a synthetic disparity of a stated size is put into lens 1's sampling and read back off \n\
         the same pixels. a slope of one says this instrument can see a disparity of that size \n\
         on this scene, which is what makes the column above a measurement.\n\n\
         {:>10} {:>10} {:>10} {:>10} {:>7}",
        "injected", "stands for", "read back", "spread", "nodes"
    );
    let one = &pairs[..1.min(pairs.len())];
    let base = sweep(reframe, grid, one, options, 0.0, prealign);
    for injected in &options.inject {
        let found = recover(reframe, grid, one, options, &base, *injected, prealign);
        println!(
            "{:>9.2}d {:>9.2}m {:>9.3}d {:>9.3}d {:>7}",
            found.injected_deg, found.metres, found.read_deg, found.spread_deg, found.nodes,
        );
    }

    let Some(up) = body_up(calibration) else {
        println!("\nno IMU record, so the deck control cannot run");
        return;
    };
    println!(
        "\nup is [{:+.4}, {:+.4}, {:+.4}] in the body's frame. on a still capture that is \n\
         gravity, so whatever the camera is standing on is a plane a fixed distance under it and \n\
         the whole downward half of the band is a one-parameter prediction: distance is that \n\
         height over the cosine, and disparity is the baseline over the distance.",
        up[0], up[1], up[2],
    );
    match deck(measured, up, options.keep) {
        Some(found) => println!(
            "the fit over {} nodes below the horizontal: a plane {:.2} m away, r = {:.3}, \n\
             leaving {:.3} deg. a height that is a height, fitted from disparity alone, is the \n\
             control that says the column above is distance and not something that correlates \n\
             with azimuth.",
            found.nodes, found.height_m, found.r, found.residual_deg,
        ),
        None => println!("too few nodes below the horizontal to fit a plane"),
    }
}

// ------------------------------------------------------------ strategies

fn strategies(options: &Options) -> Fallible<()> {
    let (calibration, pairs) = open(options, &options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = fixed(&calibration.lenses, &options.fix);
    let reframe = mapped(&lenses, frame);
    let grid = nodes(baseline(&calibration), options);
    announce(options, &calibration, &grid);
    let prealign = align(options, &reframe, &grid, &pairs);
    let measured = sweep(&reframe, &grid, &pairs, options, 0.0, &prealign);
    println!(
        "frames: {} consecutive from {:.2} s, band read in {:.2} s",
        measured.frames.len(),
        options.from,
        measured.seconds,
    );

    println!(
        "\neach plan is built from the even azimuths alone and scored at the odd ones, which no \n\
         plan ever saw. `left` is what is still misaligned there, `worst` is the worst single \n\
         direction, and `near` is the same over content inside {:.0} m, which is the harness and \n\
         the lines. `flicker` is how far the field moves at a fixed direction from one frame to \n\
         the next, before and after a {:.0} percent per frame filter.\n\n\
         {:<20} {:>7} {:>8} {:>8} {:>8} {:>9} {:>9} {:>7}",
        strategy::NEAR_M,
        strategy::SMOOTHING * 100.0,
        "plan",
        "cells",
        "left",
        "worst",
        "near",
        "flicker",
        "smoothed",
        "filled",
    );
    let plans = Plan::ladder(&options.psis);
    let mut scored = Vec::new();
    for plan in &plans {
        let found = score(plan, &measured, options.keep);
        println!(
            "{:<20} {:>7} {:>7.3}d {:>7.3}d {:>7.3}d {:>8.4}d {:>8.4}d {:>6.0}%",
            found.name,
            found.correlations,
            found.residual_deg,
            found.worst_deg,
            found.near_deg,
            found.flicker_deg,
            found.smoothed_flicker_deg,
            found.filled_share * 100.0,
        );
        scored.push(found);
    }
    println!(
        "\nthe flicker column, checked against a known step. one is put into the field every \n\
         frame, alternating sign, on the cheapest per-frame plan. it adds in quadrature to \n\
         whatever the file already had, so the expected column is that sum and not the step \n\
         alone:\n\n\
         {:>10} {:>12} {:>12}",
        "step", "expected", "read back"
    );
    let base = scored
        .get(1)
        .map_or(0.0, |plan: &strategy::Scored| plan.flicker_deg);
    for step in [0.05f64, 0.20] {
        let shaken = measured.shaken(step.to_radians());
        let read = score(&plans[1], &shaken, options.keep);
        println!(
            "{step:>9.2}d {:>11.3}d {:>11.3}d",
            base.hypot(2.0 * step),
            read.flicker_deg,
        );
    }

    if let Some(first) = scored.first() {
        println!(
            "\nwith no correction at all the same held-out nodes read {:.3} deg rms over {} \n\
             readings, {} of them inside {:.0} m.",
            first.uncorrected_deg,
            first.scored_nodes,
            first.near_nodes,
            strategy::NEAR_M,
        );
        println!(
            "the smoothed field leaves {:.3} deg where the raw one leaves {:.3}: that difference \n\
             is what the filter costs in alignment, against what it takes out of the flicker.",
            scored[0].smoothed_residual_deg, scored[0].residual_deg,
        );
    }

    // What a prepass would cost, in the arithmetic it would run rather than in
    // an opinion about it. One correlation is a search of `shifts` positions
    // over a patch of `samples`, and each position is a multiply-add per
    // sample plus the sums.
    let samples = ((options.span / options.step) as usize + 1).pow(2);
    let shifts = ((options.near - options.far) / options.step) as usize + 1;
    println!(
        "\ncost, as arithmetic rather than as an opinion: one node is {shifts} shifts over a \n\
         {samples}-sample patch, so about {:.1} M multiply-adds. the plans above run",
        (samples * shifts) as f64 / 1e6,
    );
    for (plan, found) in plans.iter().zip(&scored) {
        println!(
            "  {:<20} {:>6} nodes, {:>8.2} G multiply-adds per frame",
            plan.name,
            found.correlations,
            (found.correlations * samples * shifts) as f64 / 1e9,
        );
    }
    println!(
        "and this run measured {:.1} ms per frame for all {} nodes on one core of this box.",
        measured.seconds * 1e3 / measured.frames.len() as f64,
        grid.len(),
    );
    Ok(())
}

// ------------------------------------------------------------ pictures

/// What to say when `plan=` names nothing. The names are the ladder's own and
/// they carry spaces, so the message has to show how to type one.
fn named(asked: &str) -> String {
    format!(
        "no plan called \"{asked}\". the ladder is {}, and an underscore stands for the space: \
         plan=per-frame_dense",
        Plan::ladder(&[0.0])
            .iter()
            .map(|plan| plan.name)
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn render(options: &Options) -> Fallible<()> {
    let (calibration, pairs) = open(options, &options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = fixed(&calibration.lenses, &options.fix);
    let reframe = mapped(&lenses, frame);
    let t = baseline(&calibration);
    let grid = nodes(t, options);
    let prealign = align(options, &reframe, &grid, &pairs);
    let measured = sweep(&reframe, &grid, &pairs, options, 0.0, &prealign);
    let plan = Plan::ladder(&options.psis)
        .into_iter()
        .find(|plan| plan.name == options.plan)
        .ok_or_else(|| named(&options.plan))?;
    let warp = plan.build(&measured, 0, options.keep);

    let look = options.look();
    let out = options.out();
    let plain = paint(&reframe, &pairs[0], look, options.size, t, None);
    let warped = paint(&reframe, &pairs[0], look, options.size, t, Some(&warp));
    let stem = options
        .input
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let name = |what: &str| out.join(format!("{stem}-{what}.png"));
    plain.write(&name("1-today"))?;
    warped.write(&name("2-depth-aware"))?;
    warped.write_disparity(&name("3-disparity"), options.near)?;
    warped.write_difference(&plain, &name("4-what-moved"))?;
    println!(
        "wrote four pictures into {} at yaw {:.1}, pitch {:.1}, fov {:.1}, plan {}",
        out.display(),
        look.yaw,
        look.pitch,
        look.fov,
        options.plan,
    );
    println!(
        "the band is {:.1} percent of this view's pixels; the warp reached a peak of {:.2} deg \n\
         and the two pictures differ over {:.1} percent of it.",
        100.0 * band_share(&warped, 14.0),
        warped.applied.iter().copied().fold(0.0, f64::max),
        100.0
            * warped
                .luma
                .iter()
                .zip(&plain.luma)
                .filter(|(a, b)| (*a - *b).abs() > 1.0)
                .count() as f64
            / warped.luma.len() as f64,
    );
    println!(
        "band sharpness over its own surroundings: today {:.3}, depth-aware {:.3}, over {} \n\
         pixels in the band and {} either side of it.",
        plain.parity(),
        warped.parity(),
        warped.counted((0.0, 5.0)),
        warped.counted((9.0, 25.0)),
    );
    Ok(())
}

fn parity(options: &Options) -> Fallible<()> {
    let theirs = options
        .against
        .clone()
        .ok_or("parity wants against=<export.mp4>")?;
    let (calibration, pairs) = open(options, &options.input)?;
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let lenses = fixed(&calibration.lenses, &options.fix);
    let reframe = mapped(&lenses, frame);
    let t = baseline(&calibration);
    let grid = nodes(t, options);
    let prealign = align(options, &reframe, &grid, &pairs);
    let measured = sweep(&reframe, &grid, &pairs, options, 0.0, &prealign);
    let plan = Plan::ladder(&options.psis)
        .into_iter()
        .find(|plan| plan.name == options.plan)
        .ok_or_else(|| named(&options.plan))?;
    let warp = plan.build(&measured, 0, options.keep);

    let look = options.look();
    let export = export_frame(&theirs, options)?;
    let plain = paint(&reframe, &pairs[0], look, options.size, t, None);
    let warped = paint(&reframe, &pairs[0], look, options.size, t, Some(&warp));
    let theirs = imported(&export.0, export.1, look, options.size);
    println!(
        "\nthe view: yaw {:.2}, pitch {:.2}, roll {:.2}, fov {:.2}, compression {:.3}, at {:.2} s",
        look.yaw, look.pitch, look.roll, look.fov, look.compression, options.from,
    );
    println!(
        "\n{:<24} {:>12} {:>12} {:>10}",
        "picture", "in the band", "either side", "share"
    );
    for (name, picture) in [
        ("ours, today", &plain),
        ("ours, depth-aware", &warped),
        ("Insta360's export", &theirs),
    ] {
        println!(
            "{name:<24} {:>12.1} {:>12.1} {:>10.3}",
            picture.sharpness((0.0, 5.0)),
            picture.sharpness((9.0, 25.0)),
            picture.parity(),
        );
    }
    println!(
        "\nmean squared gradient, which a doubled edge lowers and a single one does not, over the \n\
         pixels within 5 degrees of the seam against the pixels 9 to 25 degrees off it in the \n\
         same picture. each stitch is measured against itself, so a tone curve and a sharpening \n\
         pass are in both terms and divide out."
    );
    Ok(())
}

/// One frame of a stitched export, and its size.
fn export_frame(path: &Path, options: &Options) -> Fallible<(kjerag_spike::Plane, u32)> {
    ffmpeg_next::init()?;
    let input = ffmpeg_next::format::input(&path)?;
    let stream = input
        .streams()
        .find(|s| s.parameters().medium() == ffmpeg_next::media::Type::Video)
        .ok_or("the export carries no video stream")?;
    let video = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters())?
        .decoder()
        .video()?;
    let size = MetaSize {
        width: video.width(),
        height: video.height(),
    };
    drop(input);
    let mut walk = Walk::open(
        path,
        options.from,
        kjerag_render::Size {
            width: size.width,
            height: size.height,
        },
    )?;
    let mut pair = walk
        .next_pair()?
        .ok_or("no frame decoded from the export")?;
    Ok((pair.lenses.remove(0), size.width))
}

// ------------------------------------------------------------ plumbing

#[derive(Clone)]
pub struct Options {
    mode: Mode,
    input: PathBuf,
    /// The camera maker's own export of the same capture.
    against: Option<PathBuf>,
    from: f64,
    /// How many consecutive frames the field is read on. Consecutive, because
    /// flicker is a question about neighbouring frames and nothing else.
    count: usize,
    phis: usize,
    /// Which distances past the seam are read, in degrees. The overlap is
    /// about 20 degrees wide, so this is the band.
    psis: Vec<f64>,
    span: f64,
    step: f64,
    /// The disparity window the search covers, in degrees: `far` is the least
    /// it will report and `near` the most. One-sided, because parallax is
    /// (`band.rs`), and 6 degrees is 0.32 m at the seam.
    near: f64,
    far: f64,
    keep: f64,
    contrast: f64,
    /// A free two-dimensional search instead of the epipolar one. The
    /// geometry check and the pre-alignment run it; nothing else wants it.
    free: bool,
    /// The epipolar search pinned at zero on the other axis, with no
    /// pre-alignment. What the instrument did before the pre-alignment
    /// existed, kept so that what it is worth can be measured.
    raw: bool,
    control: bool,
    /// Synthetic disparities to inject, in degrees.
    inject: Vec<f64>,
    /// The calibration correction, from `--bin seam`. Never fitted here.
    fix: Vec<(String, f64)>,
    plan: String,
    yaw: f64,
    pitch: f64,
    roll: f64,
    fov: f64,
    compression: f64,
    size: u32,
    out: Option<PathBuf>,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Fallible<Self> {
        let input = PathBuf::from(args.next().ok_or(USAGE)?);
        let mut options = Self {
            mode: Mode::Field,
            input,
            against: None,
            from: 0.0,
            count: 4,
            phis: 72,
            // Three rows, 2 degrees apart. The band is about 14 degrees wide
            // and the search window eats half of it: lens 1's grid reaches
            // `span / 2 + (near - far) / 2` past a node along the epipolar
            // axis, so a row at 3 degrees asks for content the second lens
            // does not have and is refused (measured: 296 of 720 nodes
            // outside a lens at the wider setting, 106 of 432 at this one).
            psis: vec![-2.0, 0.0, 2.0],
            span: 2.4,
            step: 0.08,
            // 4 degrees is 0.48 m at the seam, which is nearer than this
            // camera can stitch at all; 0.5 degrees the other way is the room
            // a far-field reading needs to come back at zero rather than
            // against a limit. Measured wider and narrower: 6 degrees costs
            // band coverage for distances nothing in the footage is at.
            near: 4.0,
            far: -0.5,
            keep: 0.60,
            contrast: 6.0,
            free: false,
            raw: false,
            control: false,
            inject: vec![0.5, 1.9, 3.8],
            fix: Vec::new(),
            plan: "per-clip table".to_owned(),
            yaw: 90.0,
            pitch: 0.0,
            roll: 0.0,
            fov: 60.0,
            compression: 1.0,
            size: 1024,
            out: None,
        };
        for arg in args {
            let (key, value) = arg.split_once('=').ok_or(USAGE)?;
            match key {
                "mode" => {
                    options.mode = match value {
                        "field" => Mode::Field,
                        "strategies" => Mode::Strategies,
                        "render" => Mode::Render,
                        "parity" => Mode::Parity,
                        _ => return Err(format!("no mode called {value}. {USAGE}").into()),
                    };
                }
                "against" => options.against = Some(PathBuf::from(value)),
                "out" => options.out = Some(PathBuf::from(value)),
                "from" => options.from = value.parse()?,
                "count" => options.count = value.parse()?,
                "phis" => options.phis = value.parse()?,
                "psis" => {
                    options.psis = value
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<Vec<f64>, _>>()?;
                }
                "span" => options.span = value.parse()?,
                "near" => options.near = value.parse()?,
                "far" => options.far = value.parse()?,
                "step" => options.step = value.parse()?,
                "keep" => options.keep = value.parse()?,
                "contrast" => options.contrast = value.parse()?,
                "control" => options.control = value.parse::<u32>()? != 0,
                "raw" => options.raw = value.parse::<u32>()? != 0,
                "inject" => {
                    options.inject = value
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<Vec<f64>, _>>()?;
                }
                "fix" => options.fix = turns(value)?,
                // Every plan name carries a hyphen of its own ("per-frame
                // mesh"), so a hyphen cannot be the word separator: replacing
                // them all left nothing that could match, and `plan=` selected
                // no plan at any value. `mode=render` and `mode=parity` then
                // ran the compiled-in default whatever they were asked for.
                "plan" => options.plan = value.replace('_', " "),
                "yaw" => options.yaw = value.parse()?,
                "pitch" => options.pitch = value.parse()?,
                "roll" => options.roll = value.parse()?,
                "fov" => options.fov = value.parse()?,
                "size" => options.size = value.parse()?,
                "look" => {
                    for (name, amount) in turns(value)? {
                        match name.as_str() {
                            "yaw" => options.yaw = amount,
                            "pitch" => options.pitch = amount,
                            "roll" => options.roll = amount,
                            "fov" => options.fov = amount,
                            "compression" => options.compression = amount,
                            _ => return Err(format!("no look field called {name}").into()),
                        }
                    }
                }
                _ => return Err(format!("no option called {key}. {USAGE}").into()),
            }
        }
        Ok(options)
    }

    fn look(&self) -> Look {
        Look {
            yaw: self.yaw,
            pitch: self.pitch,
            roll: self.roll,
            fov: self.fov,
            compression: self.compression,
        }
    }

    /// Pictures go into gitignored `scratch/`, because a frame of the owner's
    /// footage is personal video.
    fn out(&self) -> PathBuf {
        self.out
            .clone()
            .unwrap_or_else(|| PathBuf::from("scratch/depth"))
    }
}

/// `roll:0.84,yaw:-2.55`: a list of names and amounts.
fn turns(value: &str) -> Fallible<Vec<(String, f64)>> {
    value
        .split(',')
        .map(|part| {
            let (name, amount) = part.split_once(':').ok_or(USAGE)?;
            Ok((name.to_owned(), amount.parse()?))
        })
        .collect()
}
