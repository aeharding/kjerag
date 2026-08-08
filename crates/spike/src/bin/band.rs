//! What the shipped band pass reads, how steady it is, how wide it opens the
//! crossover, and what all of that does to the picture (issue #103, stages 2
//! and 4).
//!
//! ```sh
//! # what the band reads on one stretch, and its controls
//! cargo run --release -p kjerag-spike --bin band -- <file.insv> from=9.0 count=60
//! # the same with the pass switched off, which is stage 1's own picture
//! cargo run --release -p kjerag-spike --bin band -- <file.insv> from=9.0 off=1
//! # a stretch rendered as a sequence, with the flicker of the applied bend
//! cargo run --release -p kjerag-spike --bin band -- <file.insv> mode=sequence \
//!   from=9.0 count=90 yaw=90 pitch=-60 lock=1 out=scratch/stage2-proof
//! # before and after at one view, and what moved between them
//! cargo run --release -p kjerag-spike --bin band -- <file.insv> mode=render \
//!   from=9.0 count=60 yaw=90 fov=60 lock=1 out=scratch/stage2-proof
//! ```
//!
//! **`lock=1` is written out because it is the default and a bare `yaw=` does
//! not say which frame it is in.** Since 2026-08-06 that frame is world-fixed:
//! its zero is the file's opening heading rather than the followed one, so a
//! `yaw` copied from before that date points somewhere else and runs without a
//! word. `new_yaw = old_yaw + carried(t)`, rule and method in
//! docs/research/reference-views.md.
//!
//! **This is the shipped path and not a model of it.** Every number below is
//! read out of the very buffer `kjerag_render::ScenePipeline` dispatches into
//! while it draws, on the very frames it draws, through
//! `ScenePipeline::band_state`. Phase A's `--bin depth` is the CPU study this
//! came out of and it stays where it is; nothing here re-derives it.
//!
//! **What the columns are.** `flicker` is measured where the bend is
//! **applied** rather than where it was read, which is phase A's own ruling:
//! most of the band is filled between measured directions, and watching only
//! the directions that correlated would report the flicker of the readings and
//! call it the flicker of the picture. `width` is the same statistic for the
//! crossover width the same reading opens (stage 4), at the same directions
//! and in the same units. `control=1` is the positive control for both, and it
//! is the one thing a flicker column may not be believed without: a known step
//! is put in each frame, alternating sign, and a step of `s` has to read back
//! at `2s`.
//!
//! **`crossover` is measured over the whole run** and not over the state the
//! run ended in. A direction is near field for the second or two something
//! near it is crossing the seam and far field on either side of that, so a
//! table of where the circle settled would miss exactly the frames stage 4 is
//! for.
//!
//! PNGs land in gitignored `scratch/`: these are frames of somebody's real
//! flights and this repo is public.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_render::Cue;
use kjerag_render::{
    AZIMUTHS, Camera, Cell, Horizon, Reframe, Sampling, Scene, ScenePipeline, Size,
};
use kjerag_spike::{FORMAT, Gpu, Picture, Render, Seam};

/// How many view pixels one degree is at the width the seam residuals are
/// quoted in: 1920 across 90 degrees, at the centre of a rectilinear view
/// (docs/research/insv-format.md 6.8). The same conversion phase A printed, so
/// the two instruments' view-pixel columns are the same statistic.
const VIEW_PX_PER_DEG: f64 = 16.8;

/// Where the band is watched, in directions round the circle.
///
/// Deliberately not [`AZIMUTHS`]: the bend is applied everywhere and read at
/// [`AZIMUTHS`] places, so watching the cells alone would report the readings'
/// own steadiness and call it the picture's.
const WATCHED: usize = 360;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    match options.mode {
        Mode::Field => field(&options),
        Mode::Trace => trace(&options),
        Mode::Sequence => sequence(&options),
        Mode::Render => render(&options),
        Mode::Cost => cost(&options),
        Mode::Coverage => over_time(&options),
        Mode::Snap => snap(&options),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// What the band reads over a stretch, and whether it is depth.
    Field,
    /// One region of the screen, direction by direction and frame by frame:
    /// what the bend applied there, and where that number came from.
    Trace,
    /// A stretch drawn frame by frame, with the flicker of what was applied.
    Sequence,
    /// One view before and after, and the difference at 8x.
    Render,
    /// What the measurement costs, with the decode taken out of the timing.
    Cost,
    /// Which directions of the circle a normal play ever gets a reading at,
    /// and how that fills in over minutes.
    Coverage,
    /// What the pass DELIVERS at the pixels of one view, frame by frame, and
    /// which of three things moved it.
    Snap,
}

// ------------------------------------------------------------ the run

/// One frame of a run: the state the pass drew with, and when.
struct Read {
    at: Duration,
    cells: Vec<Cell>,
    /// The along-seam field fitted over the whole ring, which is what the
    /// pass applies on that axis (issue #103, stage 5).
    along: kjerag_render::Along,
    picture: Picture,
    /// The map this frame was drawn through. Kept because the horizon lock
    /// turns the body under the view, so which pixels are near the seam is a
    /// question about one frame and not about the run.
    mapped: Reframe,
}

/// Plays `count` frames from `from`, letting the shipped pass measure and draw
/// each one, and keeps what the band held on every frame.
///
/// The frames are advanced through [`Scene`] exactly as the player advances
/// them, and drawn through [`ScenePipeline`] exactly as the widget draws them,
/// because the state is the pass's own and only the pass fills it.
fn play(
    gpu: &Gpu,
    options: &Options,
    pipeline: &mut ScenePipeline,
    mut each: impl FnMut(&Picture, usize) -> Fallible<()>,
) -> Fallible<Vec<Read>> {
    // `still` and not `open`: it walks CONSECUTIVE frames off one reader,
    // which is what a temporal measurement needs and what a seek per frame
    // would cost a keyframe walk each time. The band's own state does not know
    // the difference: it is fed by the pass, and the pass is the same one.
    let mut scene = Scene::still(&options.input, options.at())?;
    scene.set_horizon(match options.lock {
        true => Horizon::Locked,
        false => Horizon::Free,
    });
    options.seam.hold(&scene);
    pipeline.hold_band(options.off);
    let mut reads = Vec::with_capacity(options.count);
    while let Some((_, at)) = scene.frame() {
        let picture = Render {
            gpu,
            scene: &scene,
            pipeline,
        }
        .frame(options.camera(), Sampling::default(), options.size())?;
        each(&picture, reads.len())?;
        let (along, cells) = pipeline.band_state(&gpu.device, &gpu.queue)?;
        reads.push(Read {
            at,
            cells,
            along,
            picture,
            mapped: scene
                .mapped(options.camera(), 1.0)
                .ok_or("no frame to map")?,
        });
        if reads.len() >= options.count || !scene.advance()? {
            break;
        }
    }
    if reads.is_empty() {
        return Err("no frame decoded at that instant".into());
    }
    Ok(reads)
}

// ------------------------------------------------------------ the field

fn field(options: &Options) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let reads = play(&gpu, options, &mut pipeline, |_, _| Ok(()))?;

    let last = reads.last().expect("play returns at least one frame");
    println!(
        "\nband:   {AZIMUTHS} directions, {} frames from {:.2} s to {:.2} s",
        reads.len(),
        reads[0].at.as_secs_f64(),
        last.at.as_secs_f64(),
    );

    if let Some(path) = &options.save {
        std::fs::write(path, Cell::write(&last.cells))?;
        println!("state:  written to {}", path.display());
    }
    table(last);
    coverage(&reads);
    geometry(last);
    crossover(&reads);
    flicker(&reads, options);
    Ok(())
}

/// What the shader actually bends by at one direction, in radians: the reading
/// taxed by how well it is being confirmed.
///
/// The same arithmetic `band_bend` does, on a cell rather than on a ray, and
/// the number both the width and the clamp are decided from.
fn applied(cell: &Cell) -> f32 {
    // Out of the cell rather than recomputed, so this reads the picture the
    // pass drew rather than a model of it: the gate on the way out is
    // filtered and lives in the state (`Cell::trust`).
    cell.disparity * cell.trust
}

/// What the band settled on, direction by direction, at the end of the run.
fn table(last: &Read) {
    println!(
        "\nwhat the band settled on. `view px` is the disagreement a 1920-wide 90 degree view \n\
         would show, at {VIEW_PX_PER_DEG} px per degree; `metres` is the distance the disparity \n\
         stands for; `band` is how wide the crossover opened to carry the reading; `cut` is what \n\
         a band held at this camera's own floor would have thrown away, in view px, which is \n\
         the width of the doubled edge it would leave (the floor is 8 deg on an X4 Air and \n\
         4.18 on a ONE X2, not the fixed 2 of stage 2); `off epi` is the axis a distance \n\
         CANNOT displace content along, which is measured and never applied.\n"
    );
    println!(
        "   phi  disparity    view px     metres       band        cut  confidence    off epi"
    );
    for (index, cell) in last.cells.iter().enumerate() {
        if cell.confidence <= 0.0 {
            continue;
        }
        let degrees = f64::from(cell.disparity.to_degrees());
        let applied = applied(cell);
        let floor = last.mapped.crossover_at(0.0);
        let cut = applied - kjerag_render::band::carried(applied, floor);
        println!(
            "{:>6.0} {:>9.3}d {:>10.2} {:>10} {:>9.3}d {:>10.2} {:>11.3} {:>9.3}d",
            index as f64 / AZIMUTHS as f64 * 360.0,
            degrees,
            degrees * VIEW_PX_PER_DEG,
            cell.metres()
                .map_or_else(|| "-".to_owned(), |m| format!("{m:.1}")),
            f64::from(last.mapped.crossover_at(applied).to_degrees()),
            f64::from(cut.to_degrees()) * VIEW_PX_PER_DEG,
            cell.confidence,
            f64::from(cell.off_epi.to_degrees()),
        );
    }
}

/// How far the crossover opened, and what a band held at this camera's floor
/// would be throwing away (issue #103, stage 4).
///
/// The two columns are the same measurement read two ways: the width solves
/// `|disparity| <= FOLD * width` for the width, and the clamp solves it for
/// the disparity. Everything `cut` reports is alignment the pass had measured,
/// believed, and then declined to apply because the band could not carry it -
/// a doubled edge that much wide, on content that near.
///
/// **The floor is the camera's and not stage 2's fixed 2 degrees**, so what
/// this recovers depends on the width the run is drawing: at 8 degrees the
/// floor carries every reading the search can report and both columns are zero
/// on every file in the corpus, and at `KJERAG_HANDOVER_DEG=2` the same file
/// and stretch open the band - 2 direction-frames of 40 x 128 to 2.531 deg,
/// recovering 8.0 view px on content at 0.84 m, on the owner's May-01 file at
/// `from=550` (2026-08-06).
fn crossover(reads: &[Read]) {
    let last = reads.last().expect("play returns at least one frame");
    let floor = last.mapped.crossover_at(0.0);
    // Over the whole run and not over the settled state, because a direction
    // is near field for the second or two its own gear is crossing the seam
    // and far field on either side of that. A table of where the circle ended
    // up would miss exactly the frames this stage is for.
    let seen: Vec<(usize, usize, f32)> = reads
        .iter()
        .enumerate()
        .flat_map(|(frame, read)| {
            read.cells
                .iter()
                .enumerate()
                .map(move |(index, cell)| (frame, index, applied(cell)))
        })
        .collect();
    let cut = |applied: f32| {
        f64::from((applied.abs() - kjerag_render::band::carried(applied, floor).abs()).to_degrees())
    };
    let open = seen
        .iter()
        .filter(|(_, _, applied)| last.mapped.crossover_at(*applied) > floor)
        .count();
    let frames = reads.len();
    let widest = seen
        .iter()
        .map(|(_, _, applied)| last.mapped.crossover_at(*applied))
        .fold(floor, f32::max);
    let worst = seen
        .iter()
        .map(|(_, _, applied)| cut(*applied))
        .fold(0.0, f64::max);
    // The direction the worst cut happened at, so the distance below is that
    // reading's own geometry rather than a representative one.
    let reach_m = seen
        .iter()
        .find(|(_, _, applied)| cut(*applied) >= worst)
        .map_or(0.0, |(frame, index, _)| reads[*frame].cells[*index].reach_m);
    println!(
        "\ncrossover: over {frames} frames of {AZIMUTHS} directions, {open} direction-frames \n\
         asked for more than the {:.2} deg floor, which is {:.2} percent of them, and the widest \n\
         band any of them asked for is {:.3} deg. what a band held at that floor would have cut \n\
         from those: {:.3} deg at worst, which is {:.1} view px of doubled edge on content at \n\
         {}. this stage cuts nothing the search can report, so that is what it recovers.",
        f64::from(floor.to_degrees()),
        100.0 * open as f64 / seen.len() as f64,
        f64::from(widest.to_degrees()),
        worst,
        worst * VIEW_PX_PER_DEG,
        match worst > 0.0 {
            // The reading a cut that size came off, back through the geometry.
            true => format!(
                "{:.2} m",
                f64::from(reach_m) / (f64::from(floor.to_degrees()) * 0.9 + worst).to_radians()
            ),
            false => "no distance, because nothing was cut".to_owned(),
        },
    );
    // Where it happened, so a render can be pointed at it rather than hunted
    // for: the widest few, with the frame and the azimuth each was read at.
    let mut widest_first: Vec<&(usize, usize, f32)> = seen
        .iter()
        .filter(|(_, _, applied)| last.mapped.crossover_at(*applied) > floor)
        .collect();
    widest_first.sort_by(|a, b| b.2.abs().total_cmp(&a.2.abs()));
    if !widest_first.is_empty() {
        println!(
            "\n           the widest of them, to point a render at. `at` is seconds into the \n\
             file. `view` is where to stand to see that azimuth: the seam circle runs through \n\
             the zenith, because the two lens axes are horizontal, so it is reached by PITCH \n\
             and not by yaw (measured 2026-08-01 with mode=trace, which is what to check it \n\
             with again).\n\n\
             \x20          {:>7} {:>8} {:>7} {:>10} {:>9} {:>9}   view (lock=0)",
            "frame", "at", "phi", "applied", "band", "cut px",
        );
        for (frame, index, applied) in widest_first.iter().take(8) {
            let phi = *index as f64 / AZIMUTHS as f64 * 360.0;
            // phi = -pitch at yaw 90, and the half of the circle a pitch
            // cannot reach is the same view turned round.
            let (yaw, pitch) = match phi > 90.0 && phi < 270.0 {
                true => (270.0, phi - 180.0),
                false => (90.0, -phi),
            };
            println!(
                "           {frame:>7} {:>7.2}s {phi:>6.0}d {:>9.3}d {:>8.3}d {:>9.1}   \
                 yaw {yaw:.0} pitch {pitch:.0}",
                reads[*frame].at.as_secs_f64(),
                f64::from(applied.to_degrees()),
                f64::from(last.mapped.crossover_at(*applied).to_degrees()),
                cut(*applied) * VIEW_PX_PER_DEG,
            );
        }
    }
    let Some(overlap) = last.mapped.overlap() else {
        println!("\n           one lens stream: no seam, no overlap, and no band to open.",);
        return;
    };
    // The ceiling's own safety, measured on this camera rather than assumed
    // from the fixture: the widest band plus the whole bend it carries has to
    // sit inside the ring both lenses have a picture of. The width is the
    // camera's since 2026-08-05, so it is asked of the map rather than quoted
    // from `WIDEST_DEG`, which stopped being the widest the band opens the
    // moment the floor went above it.
    //
    // **And it does not always fit, since 2026-08-08.** `affordable` bounds
    // the FLOOR and `width` may open past it, which nothing could while
    // `WIDEST_DEG` was 2.89 and every camera's floor was over it. At 4.33 a
    // camera overlapping by under 9.53 degrees is under the line and the ONE X2
    // is one, so this says which side of it this file is on rather than
    // asserting the answer.
    let widest = last
        .mapped
        .crossover_at(kjerag_render::band::WIDEST_DEG.to_radians());
    let reach = f64::from(kjerag_render::band::reach(widest).to_degrees());
    let half = f64::from(overlap.to_degrees()) * 0.5;
    let spare = match half - reach >= 0.0 {
        true => format!(
            "stays inside the overlap with {:.2} deg to spare",
            half - reach
        ),
        false => format!(
            "reaches {:.2} deg PAST the overlap, where the coverage depth hands the picture \n\
             over instead of the ramp",
            reach - half,
        ),
    };
    println!(
        "           these two lenses overlap by {:.2} deg, {half:.2} a side, which affords a \n\
         handover of {:.2}. the widest this camera's band opens is {:.2} deg, and that band plus \n\
         the whole bend it carries reaches {reach:.2} deg off the seam, so the handover {spare}.",
        f64::from(overlap.to_degrees()),
        f64::from(kjerag_render::band::affordable(overlap).to_degrees()),
        f64::from(widest.to_degrees()),
    );
}

/// How much of the circle the band reached, and how much of what it reached is
/// near enough to matter. The refusals are the story on real footage: most of
/// a seam is sky.
fn coverage(reads: &[Read]) {
    let last = reads.last().expect("play returns at least one frame");
    let read = last.cells.iter().filter(|c| c.confidence > 0.0).count();
    let near = last
        .cells
        .iter()
        .filter(|c| c.metres().is_some_and(|m| m < 10.0))
        .count();
    let ever = (0..AZIMUTHS)
        .filter(|index| reads.iter().any(|read| read.cells[*index].confidence > 0.0))
        .count();
    println!(
        "\ncoverage: {read} of {AZIMUTHS} directions were holding a reading at the end and {ever} \n\
         held one at some point in the run; {near} of the {read} are nearer than 10 m, which is \n\
         where a disparity is over a fifth of a degree and the blend stops hiding it. a direction \n\
         that never reads keeps a disparity of zero, which is the picture stage 1 drew.",
    );
}

/// The control that says the reading is depth: parallax is one-signed round the
/// circle and lies along the epipolar axis, and a residual rotation is neither.
fn geometry(last: &Read) {
    let held: Vec<&Cell> = last.cells.iter().filter(|c| c.confidence > 0.0).collect();
    if held.is_empty() {
        println!("\ngeometry: nothing correlated, so there is nothing to check.");
        return;
    }
    let towards = held.iter().filter(|c| c.disparity > 0.0).count();
    let rms = |values: Vec<f64>| match values.is_empty() {
        true => 0.0,
        false => (values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64).sqrt(),
    };
    let along = rms(held
        .iter()
        .map(|c| f64::from(c.disparity.to_degrees()))
        .collect());
    let across = rms(held
        .iter()
        .map(|c| f64::from(c.off_epi.to_degrees()))
        .collect());
    println!(
        "\ngeometry: {along:.3} deg along the epipolar axis against {across:.3} deg off it, a ratio \n\
         of {:.1}. if the band were not a stereo pair the two would be the same size and every \n\
         distance above would be a coincidence.\n\
         {towards} of {} directions read towards the front lens and {} the other way. parallax is \n\
         one-signed round the circle and a residual rotation is not, so a lopsided count is depth \n\
         and an even one is what the calibration left.",
        match across > 0.0 {
            true => along / across,
            false => f64::INFINITY,
        },
        held.len(),
        held.len() - towards,
    );
    along_seam(last);
}

/// The along-seam channel: what the ring read, what the field fitted to it,
/// and the control that replaced not applying it (issue #103, stage 5).
///
/// The old control was that this axis was never applied, so a reading far
/// smaller than the disparity said the band was a stereo pair. That claim went
/// when the channel started reaching the picture. Its replacement is the
/// **correlation between the two channels round the ring**: parallax is
/// epipolar by construction, so if any of it were reaching an axis that cannot
/// hold it - a wrong baseline, a mis-built ring - the two would move together.
fn along_seam(last: &Read) {
    let live: Vec<&Cell> = last.cells.iter().filter(|c| c.off_conf > 0.0).collect();
    if live.is_empty() {
        println!(
            "
along the seam: nothing correlated on that axis."
        );
        return;
    }
    let read: Vec<f64> = live
        .iter()
        .map(|c| f64::from(c.off_epi.to_degrees()))
        .collect();
    let rail = f64::from(kjerag_render::PERP_DEG) - 1e-3;
    let railed = read.iter().filter(|deg| deg.abs() >= rail).count();
    let terms = last.along.terms.map(|t| f64::from(t.to_degrees()));
    println!(
        "
along the seam: {} of {} directions read it, worst {:.3} deg, {railed} against the 
         {:.2} deg search limit. the axis a distance CANNOT displace content along, so what is 
         here is the camera and nothing else.
         the field: roll {:+.3} deg, one cycle {:.3} deg at phase {:.0}, two cycles {:.3} at 
         {:.0}, over {:.1} directions of evidence. what the field leaves on the ring it was 
         fitted to is {:.3} deg rms.",
        live.len(),
        last.cells.len(),
        read.iter()
            .fold(0.0, |worst: f64, deg| worst.max(deg.abs())),
        rail,
        terms[0],
        terms[1].hypot(terms[2]),
        terms[2].atan2(terms[1]).to_degrees(),
        terms[3].hypot(terms[4]),
        terms[4].atan2(terms[3]).to_degrees() / 2.0,
        f64::from(last.along.evidence),
        left(last),
    );
    match kjerag_render::depth_leak(&last.cells) {
        Some(leak) => println!(
            "the control: the two channels correlate at {leak:+.3} round the ring. parallax is 
             epipolar by construction, so anything but zero here is depth reaching an axis that 
             cannot hold it.",
        ),
        None => println!("the control: too few directions carry both channels to say."),
    }
}

/// What the fitted field leaves on the readings it was fitted to, in degrees
/// root mean square: the part of the along-seam residual a constant and two
/// cycles cannot describe.
fn left(last: &Read) -> f64 {
    let mut total = 0.0f64;
    let mut count = 0.0f64;
    for (index, cell) in last.cells.iter().enumerate() {
        if cell.off_conf <= 0.0 {
            continue;
        }
        let (sin, cos) = (index as f32 / last.cells.len() as f32 * std::f32::consts::TAU).sin_cos();
        let left = f64::from((cell.off_epi - last.along.at(cos, sin)).to_degrees());
        total += left * left;
        count += 1.0;
    }
    (total / count.max(1.0)).sqrt()
}

/// How much the applied bend moves frame to frame, and the control that says
/// the column can see a step.
fn flicker(reads: &[Read], options: &Options) {
    if reads.len() < 2 {
        println!("\nflicker: one frame, so there is nothing to say about it.");
        return;
    }
    let measured = stepped(reads, 0.0);
    println!(
        "\nflicker: {:.4} deg rms frame to frame at {WATCHED} directions round the circle, which \n\
         is {:.2} view px, and a worst single step of {:.4} deg. measured where the bend is \n\
         APPLIED and not where it was read: most of the band is filled between the directions \n\
         that correlated, and watching only those would report the flicker of the readings and \n\
         call it the flicker of the picture.",
        measured.0,
        measured.0 * VIEW_PX_PER_DEG,
        measured.1,
    );
    // The far field on its own, which is where the horizon is and where the
    // pixel-perfect claim lives. A direction reading under the near knee is
    // looking at something that does not move, so its frame-to-frame step is
    // the alignment's own repeatability and nothing else: the residual.
    let far = stepped_far(reads);
    println!(
        "\nhorizon: {:.4} deg rms frame to frame at the {} directions the band reads as far \n\
         field, which is {:.2} view px at 1920 across 90 degrees and {:.2} at the benchmark \n\
         view's 24.1. the bend at those directions IS what was measured, so what is left over \n\
         is the measurement's own repeatability.",
        far.0,
        far.1,
        far.0 * VIEW_PX_PER_DEG,
        far.0 * 1920.0 / 24.1,
    );
    // The along-seam field is the third thing the band puts in the picture
    // (stage 5), and it is the one that reaches a whole hemisphere rather than
    // a two-degree band, so it has more reason to be still than either.
    let along = stepped_along(reads, 0.0);
    println!(
        "\nalong:   {:.4} deg rms frame to frame at the same {WATCHED} directions, which is \n\
         {:.2} view px, and a worst single step of {:.4} deg. this one is applied over lens 1's \n\
         whole picture rather than across the band, so it is the column with the most to lose.",
        along.0,
        along.0 * VIEW_PX_PER_DEG,
        along.1,
    );
    // The band's WIDTH is the second thing a reading decides (stage 4), and it
    // moves the weights of every pixel of the crossover, so it has to be as
    // steady as the bend. It has no filter of its own: it is a function of the
    // same smoothed reading, so this column is the temporal design's own
    // consequence rather than a second design.
    let opened = stepped_width(reads, 0.0);
    let open = (0..WATCHED)
        .filter(|direction| {
            let last = reads.last().expect("play returns at least one frame");
            last.mapped.crossover_at(held(last, *direction) as f32) > last.mapped.crossover_at(0.0)
        })
        .count();
    println!(
        "\nwidth:   {:.4} deg rms frame to frame at the same {WATCHED} directions, worst single \n\
         step {:.4} deg. {open} of them have the band open past its floor at the end of the run; \n\
         the rest sit on the floor exactly, where the width cannot move at all, which is what \n\
         holds this column down and is also what keeps the far field the picture it was.",
        opened.0, opened.1,
    );
    if !options.control {
        println!("         (control=1 puts a known step in and reads it back.)");
        return;
    }
    println!(
        "\n         the positive control, on both columns. a step of `s` alternating sign each \n\
         frame has to come back at 2s, added in quadrature to what the file already had. a \n\
         flicker column is a negative result and means nothing until it is shown able to read \n\
         a positive one.\n\
         \n             step        bend    expected       along    expected       width    expected"
    );
    for step in [0.05f64, 0.20] {
        println!(
            "         {step:>8.2}d {:>11.4} {:>11.4} {:>11.4} {:>11.4} {:>11.4} {:>11.4}",
            stepped(reads, step.to_radians()).0,
            measured.0.hypot(2.0 * step),
            stepped_along(reads, step.to_radians()).0,
            along.0.hypot(2.0 * step),
            stepped_width(reads, step.to_radians()).0,
            opened.0.hypot(2.0 * step),
        );
    }
}

/// The frame-to-frame step of the settled disparity at the directions the band
/// reads as far field, and how many of them there were.
///
/// Far field is where nothing moves, so the step is not the scene changing: it
/// is what the same measurement of the same unchanging thing comes back as
/// twice, which is the residual an alignment cannot get below.
fn stepped_far(reads: &[Read]) -> (f64, usize) {
    // Judged on where each direction ENDED UP, so a direction that was near
    // field for part of the run is not counted as far field for the rest.
    let last = reads.last().expect("play returns at least one frame");
    let far: Vec<usize> = (0..AZIMUTHS)
        .filter(|index| {
            let cell = last.cells[*index];
            cell.confidence > 0.0 && f64::from(cell.disparity.to_degrees()).abs() < 0.19
        })
        .collect();
    let mut sum = 0.0;
    let mut count = 0.0;
    for frame in 1..reads.len() {
        for index in &far {
            let step = f64::from(reads[frame].cells[*index].disparity)
                - f64::from(reads[frame - 1].cells[*index].disparity);
            sum += step * step;
            count += 1.0;
        }
    }
    let rms = match count > 0.0 {
        true => (sum / count).sqrt().to_degrees(),
        false => 0.0,
    };
    (rms, far.len())
}

/// What the band holds at one of the [`WATCHED`] directions, in radians.
///
/// The same lookup the fragment shader does: between two cells, linearly,
/// wrapping. `kjerag_render::Reframe::bend` is the shipped one; this is its
/// arithmetic over a buffer already read back.
fn held(read: &Read, direction: usize) -> f64 {
    let turn = direction as f64 / WATCHED as f64 * AZIMUTHS as f64;
    let low = turn.floor() as usize;
    let mix = turn - low as f64;
    let cell = |index: usize| f64::from(read.cells[index % AZIMUTHS].disparity);
    cell(low) + (cell(low + 1) - cell(low)) * mix
}

/// A known step, alternating sign each frame: the positive control every
/// flicker column here is read beside.
fn shaken(frame: usize, step: f64) -> f64 {
    match frame % 2 {
        0 => step,
        _ => -step,
    }
}

/// The rms and worst frame-to-frame step of the bend, at [`WATCHED`]
/// directions, with `shake` radians put into every other frame.
fn stepped(reads: &[Read], shake: f64) -> (f64, f64) {
    stepped_by(reads, |read, frame, direction| {
        held(read, direction) + shaken(frame, shake)
    })
}

/// The same for the ALONG-SEAM field (issue #103, stage 5).
///
/// The same directions, the same units and the same control, because it is the
/// same kind of quantity: a number the shader reads off the band that moves
/// the picture if it moves. What is watched is the field's own answer at each
/// direction, which is what a pixel there is actually bent by, and not the
/// per-direction readings it was fitted to.
fn stepped_along(reads: &[Read], shake: f64) -> (f64, f64) {
    stepped_by(reads, |read, frame, direction| {
        let (sin, cos) = (direction as f32 / WATCHED as f32 * std::f32::consts::TAU).sin_cos();
        f64::from(read.along.at(cos, sin)) + shaken(frame, shake)
    })
}

/// The same for the crossover WIDTH that reading opens (issue #103, stage 4).
///
/// Watched at the same directions and reported in the same units, because it
/// is the same kind of quantity: a number the shader reads off the band that
/// moves every weight in the crossover if it moves. The shake goes into the
/// width itself rather than into the disparity behind it, which is what stage
/// 2's control does with the bend: what a control has to show is that the
/// column can see a step of a size it is told, and a step put in one place and
/// read in another would be measuring the rule instead (`band::width` has its
/// own tests for that).
fn stepped_width(reads: &[Read], shake: f64) -> (f64, f64) {
    stepped_by(reads, |read, frame, direction| {
        let opened = read.mapped.crossover_at(held(read, direction) as f32);
        f64::from(opened) + shaken(frame, shake)
    })
}

/// The rms and worst frame-to-frame step of whatever `at` reports, over
/// [`WATCHED`] directions and every consecutive pair of frames.
fn stepped_by(reads: &[Read], at: impl Fn(&Read, usize, usize) -> f64) -> (f64, f64) {
    let mut sum = 0.0;
    let mut count = 0.0;
    let mut worst: f64 = 0.0;
    for frame in 1..reads.len() {
        for direction in 0..WATCHED {
            let step =
                at(&reads[frame], frame, direction) - at(&reads[frame - 1], frame - 1, direction);
            sum += step * step;
            count += 1.0;
            worst = worst.max(step.abs());
        }
    }
    let rms = match count > 0.0 {
        true => (sum / count).sqrt().to_degrees(),
        false => 0.0,
    };
    (rms, worst.to_degrees())
}

// ------------------------------------------------------------ the trace

/// One region of the screen, direction by direction and frame by frame.
///
/// The question an owner-reported seam artifact asks is always the same one:
/// **what did the warp apply there, and where did that number come from.** The
/// field table answers the first for the whole circle at the end of a run; this
/// answers both for the handful of directions that cover the pixels he pointed
/// at, on the two frames he pointed at.
///
/// `read` is the number the correlation must have returned to move the state
/// as far as it moved, recovered from the filter's own law rather than
/// re-measured: the state moves `(read - held) * step` and every term but
/// `read` is known. That is what makes this an attribution and not a second
/// opinion.
fn trace(options: &Options) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let reads = play(&gpu, options, &mut pipeline, |_, _| Ok(()))?;
    let last = reads.last().expect("play returns at least one frame");

    let [x, y, width, height] = options.region;
    let covered = covering(&last.mapped, options.size(), options.region);
    println!(
        "\nbox:    x {x} y {y}, {width} by {height} px of a {} px view at yaw {:.1}, pitch {:.1}, \n\
         \tfov {:.1}. {} of {AZIMUTHS} directions land in it.",
        options.size().width,
        options.yaw,
        options.pitch,
        options.fov,
        covered.len(),
    );
    if covered.is_empty() {
        println!("nothing in that box is inside the crossover, so the band bends none of it.");
        return Ok(());
    }

    println!(
        "\nwhat the bend applied there, frame by frame. `applied` is the disparity the shader \n\
         used, in degrees and in view px at this view's own scale; `tau` is the time constant \n\
         the state's own value selected, which is the whole of the temporal design; `read` is \n\
         what the correlation must have returned to move the state that far, recovered from the \n\
         filter's law. a `read` far from `applied` on a frame is a direction being pulled.\n"
    );
    let px_per_deg = f64::from(options.size().width) / options.fov;
    for cell in covered {
        println!(
            "  direction {} of {AZIMUTHS}, azimuth {:.1} deg",
            cell,
            cell as f64 / AZIMUTHS as f64 * 360.0
        );
        println!(
            "  {:>6} {:>10} {:>10} {:>9} {:>7} {:>7} {:>7} {:>11}",
            "frame", "held", "applied", "view px", "conf", "trust", "tau s", "read"
        );
        for (frame, at) in reads.iter().enumerate() {
            let held = at.cells[cell];
            let before = match frame {
                0 => held,
                _ => reads[frame - 1].cells[cell],
            };
            let step = kjerag_render::ease(
                1.0 / 30.0 * 2.0,
                kjerag_render::time_constant(before.disparity),
            );
            let read = match step > 0.0 {
                true => f64::from(before.disparity + (held.disparity - before.disparity) / step),
                false => f64::NAN,
            };
            // What the shader ACTUALLY applies, gate included, which is not
            // the same as what the cell holds: a direction whose evidence has
            // gone contributes proportionally less and eventually nothing.
            //
            // Read out of the cell rather than recomputed here: the pass
            // stores the gate it applied (`Cell::trust`), which is filtered
            // and which no expression on this side could reproduce without
            // keeping the same yesterday the pass keeps.
            let applied = f64::from(held.disparity) * f64::from(held.trust);
            println!(
                "  {frame:>6} {:>9.4}d {:>9.4}d {:>9.2} {:>7.3} {:>7.3} {:>7.2} {:>10.4}d",
                f64::from(held.disparity).to_degrees(),
                applied.to_degrees(),
                applied.to_degrees() * px_per_deg,
                held.confidence,
                held.trust,
                kjerag_render::time_constant(before.disparity),
                read.to_degrees(),
            );
        }
        println!();
    }
    Ok(())
}

/// Which directions of the circle the crossover covers inside a screen box.
///
/// A pixel is bent only where both lenses claim it, so a box outside the
/// crossover has no directions at all and the band cannot be what moved it.
///
/// **The half-corridor is asked of the map and no longer written down here.**
/// It was a literal 1.5 degrees, which was half of a 2 degree crossover and
/// stopped being half of anything when the handover went to 8 on 2026-08-05
/// (issue #162): this reported the middle 3 degrees of an 8 degree corridor
/// and called it the corridor. `crossover_at(0.0)` is the width the file's own
/// two lenses afford, which is the number the pass hands over across.
///
/// **How much it was costing, measured rather than assumed**, at the owner's
/// first banked downward view on May-01 with the same stored fit: **9
/// directions of 128 before and 10 after** at fov 20, and **34 either way** at
/// fov 90. So it was a real defect and a small one at these views: a box
/// narrow enough to matter is also narrow enough that its own azimuth span,
/// not the corridor's width, is what decides. The diagnosis it was under
/// (docs/research/seam-temporal.md 2.2) is one direction short and not
/// wrong.
fn covering(reframe: &Reframe, size: Size, region: [u32; 4]) -> Vec<usize> {
    let [x, y, width, height] = region;
    let half = 0.5 * reframe.crossover_at(0.0).to_degrees();
    let mut seen = [false; AZIMUTHS];
    for row in y..(y + height).min(size.height) {
        for column in x..(x + width).min(size.width) {
            let uv = [
                column as f32 / size.width as f32,
                row as f32 / size.height as f32,
            ];
            let Some(ray) = reframe.view_ray(uv) else {
                continue;
            };
            let Some(at) = reframe.seam_at(ray) else {
                continue;
            };
            // Only where the handover is actually mixing the two lenses: that
            // is the only place a disparity reaches the picture.
            let body = reframe.body_ray(ray);
            let length = (body[0] * body[0] + body[1] * body[1] + body[2] * body[2]).sqrt();
            let past = (body[2] / length).asin().to_degrees().abs();
            if past > half {
                continue;
            }
            let turn =
                at.phi.rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU * AZIMUTHS as f32;
            for step in [turn.floor(), turn.floor() + 1.0] {
                seen[(step as usize) % AZIMUTHS] = true;
            }
        }
    }
    (0..AZIMUTHS).filter(|index| seen[*index]).collect()
}

// ------------------------------------------------------------ the snap

/// How many probes `mode=snap` lays across the view.
///
/// They sit on the seam's own centre line, one per evenly spaced column, and
/// they are held fixed in the VIEW: under `lock=1` that is a fixed world
/// direction, so the body turns under them and their azimuth sweeps. That
/// sweep is the thing H1 is about.
const PROBES: usize = 21;

/// What counts as a step the attribution has to name, in view px.
///
/// The owner's percept is a jump in a picture he is looking at; three px of a
/// 1024 px view is the floor this instrument is willing to call one, and it is
/// the same floor the temporal memo counted its ten-px steps against.
const STEP_PX: f64 = 3.0;

/// One probe: a pixel of the view, held fixed while the body turns under it.
struct Probe {
    x: u32,
    y: u32,
    ray: [f32; 3],
    /// How far off the seam plane it sits on the frame it was chosen at, in
    /// degrees. Zero is the middle of the handover.
    past_deg: f64,
}

/// What the pass delivers at one probe, out of one frame's map and one frame's
/// state - which need not be the same frame, and that is the whole of the
/// attribution: the sweep term reads this frame's geometry against last
/// frame's state.
struct Delivered {
    /// The epipolar reading the shader applies there, in view px at this
    /// view's own scale. The two lenses are moved apart by this much; each
    /// one moves by the other's weight times it.
    epi_px: f64,
    /// The along-seam correction the shader applies there, same units. Lens 1
    /// takes it whole.
    along_px: f64,
    phi_deg: f64,
    low: usize,
    mix: f64,
    /// The two cells the lookup landed between, as evidence and as gate.
    conf: [f32; 2],
    trust: [f32; 2],
    /// What each lens is weighted by there, so a reader can turn the epipolar
    /// column into per-lens motion.
    weight: [f32; 2],
}

/// A known defect put into the state a run already read back, so that the
/// attribution below can be shown to catch the thing it claims to catch
/// before it is believed about the things it found.
#[derive(Clone, Copy, PartialEq)]
enum Plant {
    /// Nothing: the null, which says what the attribution reports off the
    /// footage alone.
    None,
    /// One cell's held reading moved by a known amount on EVERY frame: a
    /// discontinuity fixed in the ring, which a sweeping probe must meet as a
    /// sweep step and never as a state one.
    Cell(usize, f64),
    /// Every cell's held reading moved by a known amount from one frame on: a
    /// state re-commit, which every probe must meet on that frame and on no
    /// other.
    Commit(usize, f64),
    /// One cell held empty until a named frame, so its own real reading
    /// arrives whole there.
    Arrive(usize, usize),
    /// The whole state frozen at one frame while the map runs: every step the
    /// picture then takes is the sweep and nothing else, so this is both the
    /// control for that term and the measurement of it on its own.
    Hold(usize),
    /// The map frozen at the first frame while the state runs: the mirror of
    /// [`Self::Hold`], and the same two jobs on the other term.
    Still,
}

impl Plant {
    fn parse(value: &str) -> Fallible<Self> {
        let mut parts = value.split(':');
        let what = parts.next().unwrap_or_default();
        let mut number = || parts.next().ok_or("plant wants two numbers after its name");
        match what {
            "none" => Ok(Self::None),
            "cell" => Ok(Self::Cell(number()?.parse()?, number()?.parse()?)),
            "commit" => Ok(Self::Commit(number()?.parse()?, number()?.parse()?)),
            "arrive" => Ok(Self::Arrive(number()?.parse()?, number()?.parse()?)),
            "hold" => Ok(Self::Hold(number()?.parse()?)),
            "still" => Ok(Self::Still),
            _ => Err(format!("no plant called {what}").into()),
        }
    }

    /// Put it into the state the run read back, which is what the attribution
    /// then runs over.
    fn into(self, reads: &mut [Read]) {
        match self {
            Self::None => {}
            Self::Cell(cell, degrees) => {
                for read in reads.iter_mut() {
                    read.cells[cell % AZIMUTHS].disparity += degrees.to_radians() as f32;
                }
            }
            Self::Commit(frame, degrees) => {
                for read in reads.iter_mut().skip(frame) {
                    for cell in &mut read.cells {
                        cell.disparity += degrees.to_radians() as f32;
                    }
                }
            }
            Self::Arrive(cell, frame) => {
                for read in reads.iter_mut().take(frame) {
                    read.cells[cell % AZIMUTHS] = Cell::default();
                }
            }
            Self::Hold(frame) => {
                let (cells, along) = match reads.get(frame) {
                    Some(read) => (read.cells.clone(), read.along),
                    None => return,
                };
                for read in reads.iter_mut() {
                    read.cells = cells.clone();
                    read.along = along;
                }
            }
            Self::Still => {
                let Some(map) = reads.first().map(|read| read.mapped) else {
                    return;
                };
                for read in reads.iter_mut() {
                    read.mapped = map;
                }
            }
        }
    }

    fn says(self) -> String {
        match self {
            Self::None => "plant:  none, so this run is the null".to_owned(),
            Self::Cell(cell, degrees) => format!(
                "plant:  cell {cell} moved {degrees:+.3} deg on every frame. it must show up as \
                 SWEEP, at the frames a probe crosses that cell"
            ),
            Self::Commit(frame, degrees) => format!(
                "plant:  every cell moved {degrees:+.3} deg from frame {frame} on. it must show up \
                 as STATE, on frame {frame} and on no other"
            ),
            Self::Arrive(cell, frame) => format!(
                "plant:  cell {cell} held empty until frame {frame}. it must show up as ARRIVAL, \
                 on frame {frame}"
            ),
            Self::Hold(frame) => format!(
                "plant:  the state frozen at frame {frame} while the map runs. every step must be \
                 SWEEP, and what they measure is that term on its own"
            ),
            Self::Still => "plant:  the map frozen at the first frame while the state runs. every \
                 step must be STATE"
                .to_owned(),
        }
    }
}

/// Which of the two axes a step is measured on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    /// Across the seam: the depth channel, read per CELL and interpolated
    /// between two of them. The only axis with the ring's granularity in it.
    Epi,
    /// Along the seam: five harmonics over the whole circle, plus the stored
    /// table where one is loaded. No cells unless that table is loaded.
    Along,
}

impl Axis {
    fn of(self, at: &Delivered) -> f64 {
        match self {
            Self::Epi => at.epi_px,
            Self::Along => at.along_px,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Epi => "across the seam (per cell)",
            Self::Along => "along the seam (five harmonics)",
        }
    }
}

/// Which class of the three the memo names a step belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    /// H1: the geometry moved under a state that did not. A probe crossing
    /// the ring's own cells is this, and so is any other motion of the map.
    Sweep,
    /// H2: the state moved under a geometry that did not.
    Commit,
    /// H3: the same, where one of the two cells behind the probe had no
    /// evidence at all on the frame before.
    Arrival,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Self::Sweep => "sweep",
            Self::Commit => "commit",
            Self::Arrival => "arrival",
        }
    }
}

/// The threshold the temporal memo counted its own steps at, in view px.
///
/// Printed beside [`STEP_PX`] so that a delivered count can be read against
/// the cell count the A/B was staged on (83 at `down1` on main against 4
/// with the gate filtered, docs/research/seam-temporal.md 8.2).
const MEMO_PX: f64 = 10.0;

/// One frame-to-frame step at one probe, decomposed.
struct Step {
    frame: usize,
    probe: usize,
    total: f64,
    sweep: f64,
    state: f64,
    class: Class,
    /// Whether either cell behind the probe had no evidence at all on the
    /// frame before. Kept beside [`Self::class`] rather than folded into it,
    /// because a class is decided by which TERM is larger and this is a fact
    /// about the step whatever won that comparison: an arrival under a bigger
    /// sweep is still an arrival, and [`terms`] has to be able to say so.
    arrived: bool,
    phi_deg: f64,
    low: usize,
}

/// **What the owner's snap points are made of** (the 2026-08-08 percept: "the
/// seam snaps to snap points that are too far apart").
///
/// The question this answers is not what the band reads. It is what the pass
/// **delivers** at the pixels he is looking at, frame by frame, and which of
/// three things moved it: the map sweeping a fixed view direction across the
/// ring's cells, the held state re-committing under it, or a direction with no
/// picture behind it taking its first answer whole.
///
/// The decomposition is exact and not a model. The delivered value is
/// `Q(geometry, state)`; a step is `Q(f, f) - Q(f-1, f-1)`; and the two terms
/// below add to it by construction, because the middle evaluation `Q(f, f-1)`
/// is subtracted once and added once:
///
/// ```text
/// sweep = Q(f, f-1) - Q(f-1, f-1)      the map moved, the state did not
/// state = Q(f, f)   - Q(f, f-1)        the state moved, the map did not
/// ```
///
/// `Q` is [`Reframe::reading_at`], which is the shader's own lookup, called
/// with one frame's map and another frame's cells. The closure line at the end
/// prints the worst `total - (sweep + state)` over the whole run, which is
/// float noise or the instrument is wrong.
fn snap(options: &Options) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let mut reads = play(&gpu, options, &mut pipeline, |_, _| Ok(()))?;
    // Kept before the plant goes in, so the run can be attributed twice and
    // the plant's own contribution read as the difference rather than as a
    // change in a total the footage dominates.
    //
    // **The map is saved with the cells.** It was not, and [`Plant::Still`] is
    // the plant that moves the map: its null was built out of the frozen map
    // it was meant to be a null of, so the difference came back exactly zero
    // and the control read as a clean pass when it had measured nothing at
    // all. A control that cannot fail is not a control, and this one could
    // not: see `plant_check`, which now refuses a zero it was handed under a
    // plant.
    let saved: Vec<(Vec<Cell>, kjerag_render::Along, Reframe)> = reads
        .iter()
        .map(|read| (read.cells.clone(), read.along, read.mapped))
        .collect();
    options.plant.into(&mut reads);

    let opened = reads[0].mapped;
    let first = &reads[0];
    let last = reads.last().expect("play returns at least one frame");
    let size = options.size();
    let px_per_deg = f64::from(size.width) / options.fov;
    let probes = probes(&first.mapped, size);
    println!(
        "\nview:   {} px at yaw {:.2}, pitch {:.2}, fov {:.2}, lock={}. {:.1} view px per degree.\n\
         run:    {} frames, {:.3} s to {:.3} s of media time.\n{}",
        size.width,
        options.yaw,
        options.pitch,
        options.fov,
        u8::from(options.lock),
        px_per_deg,
        reads.len(),
        first.at.as_secs_f64(),
        last.at.as_secs_f64(),
        options.plant.says(),
    );
    println!(
        "table:  the stored along-seam table is {}. it is the ONLY per-cell field on the \n\
         \talong-seam axis; the band's own along-seam term is five harmonics over the whole \n\
         \tcircle and has no cells in it at all.",
        match last.mapped.table().is_rest() {
            true => "AT REST, so nothing on that axis is read per cell in this run",
            false => "loaded",
        },
    );
    if probes.is_empty() {
        println!("nothing on this view is inside the handover, so the band delivers none of it.");
        return Ok(());
    }

    sweep_rate(&reads, &probes, px_per_deg);
    profile(&reads, &probes, px_per_deg);
    section(&reads, size, px_per_deg);
    reaches(&reads, &probes, px_per_deg);
    support(&reads, &first.mapped, size);
    weight(&reads, &probes, &first.mapped, size, px_per_deg);
    arrivals(&reads, &first.mapped, size);
    for axis in [Axis::Epi, Axis::Along] {
        let steps = attribute(&reads, &probes, px_per_deg, axis);
        verdict(&steps, &reads, probes.len(), axis);
    }
    // The same run again with the plant taken back out, which is the only
    // null worth having: one run's geometry, one run's decode, one run's
    // footage, and the plant as the single difference between them.
    let planted = attribute(&reads, &probes, px_per_deg, Axis::Epi);
    for (read, (cells, along, mapped)) in reads.iter_mut().zip(saved) {
        read.cells = cells;
        read.along = along;
        read.mapped = mapped;
    }
    let null = attribute(&reads, &probes, px_per_deg, Axis::Epi);
    plant_check(options.plant, &planted, &null);
    let last = reads.last().expect("play returns at least one frame");
    neighbours(last, &opened, size, px_per_deg);
    spread(&reads, &opened, size, px_per_deg);
    Ok(())
}

/// The probes: one per evenly spaced column, at the row nearest the middle of
/// the handover, and only where the handover actually reaches.
///
/// On the seam's own centre line because that is where both lenses are half
/// weighted, which is where a disagreement is drawn twice at equal strength
/// and where the owner is looking.
fn probes(reframe: &Reframe, size: Size) -> Vec<Probe> {
    probes_at(reframe, size, PROBES)
}

/// The same at any spacing, which is how [`section`] asks for one per column:
/// [`PROBES`] is enough to attribute a step and nowhere near enough to say how
/// far apart the places it steps BETWEEN are.
fn probes_at(reframe: &Reframe, size: Size, count: usize) -> Vec<Probe> {
    let half = 0.5 * f64::from(reframe.crossover_at(0.0).to_degrees());
    (0..count)
        .filter_map(|index| {
            let x = ((index as f32 + 0.5) / count as f32 * size.width as f32) as u32;
            let mut best: Option<Probe> = None;
            for y in 0..size.height {
                let uv = [x as f32 / size.width as f32, y as f32 / size.height as f32];
                let Some(ray) = reframe.view_ray(uv) else {
                    continue;
                };
                let past = past_deg(reframe, ray);
                if past.abs() > half {
                    continue;
                }
                if best
                    .as_ref()
                    .is_some_and(|held| held.past_deg.abs() <= past.abs())
                {
                    continue;
                }
                best = Some(Probe {
                    x,
                    y,
                    ray,
                    past_deg: past,
                });
            }
            best
        })
        .collect()
}

/// How far past the seam plane a ray is, in degrees: the angle off the body's
/// own xy plane, which is what the two lenses hand over across.
fn past_deg(reframe: &Reframe, ray: [f32; 3]) -> f64 {
    let body = reframe.body_ray(ray);
    let length = (body[0] * body[0] + body[1] * body[1] + body[2] * body[2]).sqrt();
    match length > 0.0 {
        true => f64::from((body[2] / length).asin().to_degrees()),
        false => f64::INFINITY,
    }
}

/// What the pass delivers at one probe, out of `geometry`'s map and `state`'s
/// cells.
fn deliver(geometry: &Read, state: &Read, probe: &Probe, px_per_deg: f64) -> Delivered {
    let reframe = &geometry.mapped;
    let reading = reframe.reading_at(probe.ray, &state.cells, state.along);
    let body = reframe.body_ray(probe.ray);
    let reach = body[0].hypot(body[1]);
    let turn = body[1].atan2(body[0]) / std::f32::consts::TAU * AZIMUTHS as f32;
    let low = turn.floor();
    let mix = f64::from(turn - low);
    let index = |step: usize| (low.rem_euclid(AZIMUTHS as f32) as usize + step) % AZIMUTHS;
    let table = match reach > 0.0 {
        true => reframe.table().at(body[0] / reach, body[1] / reach),
        false => 0.0,
    };
    let cells = [state.cells[index(0)], state.cells[index(1)]];
    Delivered {
        epi_px: f64::from(reading.epi.to_degrees()) * px_per_deg,
        along_px: f64::from(((reading.along + table) * reach).to_degrees()) * px_per_deg,
        phi_deg: f64::from(body[1].atan2(body[0]).to_degrees()).rem_euclid(360.0),
        low: index(0),
        mix,
        conf: [cells[0].confidence, cells[1].confidence],
        trust: [cells[0].trust, cells[1].trust],
        weight: reframe.blend_bent(probe.ray, reading).weights[..2]
            .try_into()
            .expect("two lenses"),
    }
}

/// How fast the world-fixed view sweeps the ring, which is what turns a field
/// that varies round the circle into a picture that moves.
fn sweep_rate(reads: &[Read], probes: &[Probe], px_per_deg: f64) {
    let middle = &probes[probes.len() / 2];
    let first = deliver(&reads[0], &reads[0], middle, px_per_deg);
    let last = reads.last().expect("a run has frames");
    let end = deliver(last, last, middle, px_per_deg);
    let seconds = last.at.as_secs_f64() - reads[0].at.as_secs_f64();
    let turned = (end.phi_deg - first.phi_deg + 540.0).rem_euclid(360.0) - 180.0;
    let cell_deg = 360.0 / AZIMUTHS as f64;
    // Frame to frame as well as end to end: a view that comes back where it
    // started has a net drift of nothing and may still have crossed a cell
    // boundary and come back on every frame in between.
    let mut steps: Vec<f64> = Vec::new();
    for frame in 1..reads.len() {
        let now = deliver(&reads[frame], &reads[frame], middle, px_per_deg);
        let was = deliver(&reads[frame - 1], &reads[frame - 1], middle, px_per_deg);
        steps.push((now.phi_deg - was.phi_deg + 540.0).rem_euclid(360.0) - 180.0);
    }
    let rms =
        (steps.iter().map(|step| step * step).sum::<f64>() / steps.len().max(1) as f64).sqrt();
    let worst = steps.iter().map(|step| step.abs()).fold(0.0, f64::max);
    let crossings = (1..reads.len())
        .filter(|frame| {
            let now = deliver(&reads[*frame], &reads[*frame], middle, px_per_deg);
            let was = deliver(&reads[frame - 1], &reads[frame - 1], middle, px_per_deg);
            now.low != was.low
        })
        .count();
    println!(
        "\nsweep:  the middle probe's own azimuth ran {:.2} to {:.2} deg over {seconds:.2} s, \n\
         \twhich is {:.2} deg/s net, or one of the ring's {cell_deg:.2} deg cells every {:.1} s. \n\
         \tframe to frame it moves {rms:.3} deg rms and {worst:.3} deg at worst, and it changed \n\
         \twhich cell pair it reads on {crossings} of {} frames. one cell of azimuth is {:.0} \n\
         \tview px of this picture.",
        first.phi_deg,
        end.phi_deg,
        turned / seconds.max(1e-9),
        cell_deg / (turned.abs() / seconds.max(1e-9)).max(1e-9),
        steps.len(),
        cell_deg * px_per_deg,
    );
}

/// The delivered field ACROSS the picture on one frame, probe by probe.
///
/// This is the spatial half of the question and it is printed first, because a
/// field that is flat across the view cannot snap however it moves, and a
/// field with a shape in it snaps by the shape passing under a fixed pixel.
fn profile(reads: &[Read], probes: &[Probe], px_per_deg: f64) {
    println!(
        "\nwhat the pass delivers across the picture. `epi px` is the across-seam correction at \n\
         that pixel, in view px of THIS view; the two lenses are moved apart by it and each moves \n\
         by the other's weight times it. `w0`/`w1` are those weights. `conf`/`trust` are the two \n\
         cells the lookup landed between: a zero there is a direction with no picture behind it, \n\
         and the correction goes to nothing over it whatever its neighbours read.\n"
    );
    for frame in [0, reads.len() / 2, reads.len() - 1] {
        let read = &reads[frame];
        println!("  frame {frame} at {:.3} s", read.at.as_secs_f64(),);
        println!(
            "  {:>5} {:>5} {:>8} {:>6} {:>5} {:>9} {:>10} {:>13} {:>13} {:>11}",
            "x", "y", "phi", "cell", "mix", "epi px", "along px", "conf", "trust", "w0/w1",
        );
        let mut across: Vec<f64> = Vec::new();
        for probe in probes {
            let at = deliver(read, read, probe, px_per_deg);
            across.push(at.epi_px);
            println!(
                "  {:>5} {:>5} {:>7.2}d {:>6} {:>5.2} {:>9.2} {:>10.2} {:>6.3}/{:<6.3} \
                 {:>6.3}/{:<6.3} {:>5.2}/{:<5.2}",
                probe.x,
                probe.y,
                at.phi_deg,
                at.low,
                at.mix,
                at.epi_px,
                at.along_px,
                at.conf[0],
                at.conf[1],
                at.trust[0],
                at.trust[1],
                at.weight[0],
                at.weight[1],
            );
        }
        shape(&across, probes);
        println!();
    }
}

/// How far apart the places the correction steps BETWEEN are, in view px of
/// his own picture, measured across the screen rather than across time.
///
/// **The owner's complaint is spatial and this is the column that answers it**
/// ("the seam snaps to snap points that are too far apart. Needs closer, maybe
/// overlap? idk"). Every other column here watches one pixel over many frames;
/// this one watches every column of one frame. It builds one probe per two
/// screen columns off THAT frame's own map, asks the shipped lookup what it
/// delivers at each, and reports three things about the answer:
///
/// - **reach**: the run of columns corrected by a pixel or more, in screen px.
///   That is how wide the corrected patch he is looking at actually is.
/// - **the steepest hundred px**: the largest change in the delivered
///   correction over any hundred columns, which is the gradient an eye reads
///   as an edge.
/// - **the cell boundaries inside the view**, with the delivered value either
///   side of each. A boundary is where the lookup changes which pair of cells
///   it is mixing, so it is the only place the field is allowed a corner, and
///   the table says how big a corner each one actually has.
///
/// One cell of the ring is `360/AZIMUTHS` degrees, which at fov 20 across 1024
/// px is 144 screen px: **that is the spacing his "too far apart" is a
/// complaint about, and it is a number this prints rather than assumes.**
fn section(reads: &[Read], size: Size, px_per_deg: f64) {
    println!(
        "\nthe delivered field ACROSS one frame, at one probe per two columns. `reach` is the \n\
         columns corrected by 1 px or more; `steepest` is the largest change over any 100 \n\
         columns. one of the ring's cells is {:.0} screen px of this view.\n",
        360.0 / AZIMUTHS as f64 * px_per_deg,
    );
    for frame in [0, reads.len() / 2, reads.len() - 1] {
        let read = &reads[frame];
        let dense = probes_at(&read.mapped, size, size.width as usize / 2);
        if dense.len() < 2 {
            println!("  frame {frame}: nothing of this view is inside the handover.");
            continue;
        }
        let at: Vec<(u32, Delivered)> = dense
            .iter()
            .map(|probe| (probe.x, deliver(read, read, probe, px_per_deg)))
            .collect();
        let reached: Vec<u32> = at
            .iter()
            .filter(|(_, value)| value.epi_px.abs() >= 1.0)
            .map(|(x, _)| *x)
            .collect();
        let peak = at
            .iter()
            .map(|(_, value)| value.epi_px.abs())
            .fold(0.0, f64::max);
        // Over a hundred columns and not over one, because a gradient read at
        // the sampling interval is a gradient read at the sampling interval:
        // a hundred px is about a tenth of his picture and is the scale an
        // edge is seen at.
        let span = 100 / 2;
        let (steep, edge) = at
            .windows(span + 1)
            .map(|run| {
                (
                    (run[span].1.epi_px - run[0].1.epi_px).abs(),
                    (run[0].0, run[span].0),
                )
            })
            .fold((0.0, (0, 0)), |held, next| match next.0 > held.0 {
                true => next,
                false => held,
            });
        // **The interior trough**, which is the whole of the owner's percept
        // as a single number. A correction that ramps up at one edge of the
        // corrected patch and down at the other has its smallest values at the
        // edges and that is a lobe. One that comes back to nothing in the
        // MIDDLE of the patch, with full-strength correction either side of
        // the place it does, is a comb, and the depth of that trough is how
        // far apart the two values the picture is snapping between are.
        let inside: Vec<&(u32, Delivered)> = match (reached.first(), reached.last()) {
            (Some(low), Some(high)) => at
                .iter()
                .filter(|(x, _)| x > low && x < high)
                .collect::<Vec<&(u32, Delivered)>>(),
            _ => Vec::new(),
        };
        let trough = inside
            .iter()
            .map(|(x, value)| (value.epi_px.abs(), *x))
            .fold((f64::INFINITY, 0), |held, next| match next.0 < held.0 {
                true => next,
                false => held,
            });
        println!(
            "  frame {frame} at {:.3} s: reach {} of {} columns ({} to {} px), peak {peak:.1} px, \n\
             \tsteepest {steep:.1} px over the 100 columns {} to {}. inside that reach the \n\
             \tcorrection falls as low as {} px, at column {}.",
            read.at.as_secs_f64(),
            reached.len() * 2,
            size.width,
            reached.first().map_or(0, |x| *x),
            reached.last().map_or(0, |x| *x),
            edge.0,
            edge.1,
            match trough.0.is_finite() {
                true => format!("{:.1}", trough.0),
                false => "-".to_owned(),
            },
            trough.1,
        );
        println!(
            "  {:>8} {:>9} {:>12} {:>12} {:>12}   the cell boundaries inside the view",
            "at px", "phi", "left px", "right px", "corner px",
        );
        for pair in at.windows(2) {
            if pair[0].1.low == pair[1].1.low {
                continue;
            }
            println!(
                "  {:>8} {:>8.2}d {:>12.2} {:>12.2} {:>12.2}",
                pair[1].0,
                pair[1].1.phi_deg,
                pair[0].1.epi_px,
                pair[1].1.epi_px,
                pair[1].1.epi_px - pair[0].1.epi_px,
            );
        }
        println!();
    }
}

/// How much of the picture the correction reaches, frame by frame, over the
/// whole run.
///
/// **The number the down1 divergence is about.** The temporal memo counted its
/// steps at the ring's CELLS: ten directions of a hundred and twenty frames,
/// and eighty-three of those pairs moved the applied value by more than ten
/// view px at `down1`. What an eye sees is not a cell, it is a patch of
/// screen, and a cell that steps while it is delivering nothing to any pixel
/// of his view steps in a place he cannot look at. This says how many pixels
/// were being moved at all while those steps were happening.
fn reaches(reads: &[Read], probes: &[Probe], px_per_deg: f64) {
    let mut width: Vec<usize> = Vec::with_capacity(reads.len());
    // Frames whose delivered field comes back to nothing INSIDE its own
    // corrected patch while carrying a real correction either side of the
    // place it does. See `section`: a lobe has its small values at its edges
    // and a comb has one in the middle, and only the second is two values a
    // picture can snap between.
    let mut combed = 0;
    for read in reads {
        let across: Vec<f64> = probes
            .iter()
            .map(|probe| deliver(read, read, probe, px_per_deg).epi_px)
            .collect();
        width.push(across.iter().filter(|value| value.abs() >= 1.0).count());
        let live = |value: &&f64| value.abs() >= 1.0;
        let (Some(low), Some(high)) = (
            across.iter().position(|value| live(&value)),
            across.iter().rposition(|value| live(&value)),
        ) else {
            continue;
        };
        let peak = across.iter().map(|value| value.abs()).fold(0.0, f64::max);
        let trough = across[low + 1..high.max(low + 1)]
            .iter()
            .map(|value| value.abs())
            .fold(f64::INFINITY, f64::min);
        if peak >= COMB_PEAK_PX && trough <= COMB_TROUGH_PX {
            combed += 1;
        }
    }
    let mut sorted = width.clone();
    sorted.sort_unstable();
    let changed = width.windows(2).filter(|pair| pair[0] != pair[1]).count();
    let dead = width.iter().filter(|count| **count == 0).count();
    println!(
        "\nreach:  of the {} probes across the picture, the number carrying 1 view px or more of \n\
         \tcorrection runs {} at worst, {} in the middle and {} at best over the {} frames. \n\
         \t{dead} frames have NO corrected pixel at any probe at all, and the count changes on \n\
         \t{changed} of {} frame pairs. a step at a cell that is delivering nothing to any of \n\
         \tthese probes is a step in a place the owner is not looking. \n\
         \tand on {combed} of {} frames the field is COMBED: it peaks past {COMB_PEAK_PX:.0} px \n\
         \tand comes back under {COMB_TROUGH_PX:.0} px somewhere strictly inside its own reach.",
        probes.len(),
        sorted.first().map_or(0, |count| *count),
        sorted[sorted.len() / 2],
        sorted.last().map_or(0, |count| *count),
        reads.len(),
        width.len() - 1,
        reads.len(),
    );
}

/// What a delivered field has to peak at before the hole in the middle of it
/// is worth counting, in view px.
///
/// Ten, which is the threshold the temporal memo counted its own steps at, so
/// a combed frame here is a frame carrying a correction of the size that A/B
/// was staged on.
const COMB_PEAK_PX: f64 = 10.0;

/// And what it has to fall back to for the hole to be a hole.
///
/// Two, which is under the 3 px this instrument is willing to call a step at
/// all ([`STEP_PX`]): a correction that has fallen this far has effectively
/// gone, and the picture either side of it has not.
const COMB_TROUGH_PX: f64 = 2.0;

/// What the delivered field looks like ACROSS the picture on one frame: how
/// much of the view it reaches at all, and how steeply it changes where it
/// does.
///
/// The two numbers the owner's percept is about. A correction that is applied
/// over part of the picture and not the rest has an edge in it, and the width
/// of the part it reaches is how far apart the places it snaps to are.
fn shape(across: &[f64], probes: &[Probe]) {
    let reached = across.iter().filter(|value| value.abs() >= 1.0).count();
    let peak = across.iter().map(|value| value.abs()).fold(0.0, f64::max);
    let gap = probes
        .windows(2)
        .map(|pair| f64::from(pair[1].x - pair[0].x))
        .fold(0.0, f64::max);
    let (steepest, at) = across
        .windows(2)
        .enumerate()
        .map(|(index, pair)| ((pair[1] - pair[0]).abs(), index))
        .fold((0.0, 0), |held, next| match next.0 > held.0 {
            true => next,
            false => held,
        });
    println!(
        "  shape: {reached} of {} probes are corrected by 1 px or more, the peak is {peak:.1} px, \
         and the\n         steepest neighbouring pair is {steepest:.1} px over the {gap:.0} px \
         between probes {at} and {}.",
        across.len(),
        at + 1,
    );
}

/// Which of the ring's cells the view stands on, and what each one is holding
/// over the run.
fn support(reads: &[Read], reframe: &Reframe, size: Size) {
    let covered = covering(reframe, size, [0, 0, size.width, size.height]);
    println!(
        "the {} cells this view stands on, over the {} frames of the run. `live` is the frames \n\
         that cell had any evidence at all; `first` is the frame its evidence arrived on.\n",
        covered.len(),
        reads.len(),
    );
    println!(
        "  {:>5} {:>8} {:>7} {:>7} {:>11} {:>11} {:>11} {:>11}",
        "cell", "phi", "live", "first", "disp first", "disp last", "conf last", "trust last",
    );
    for cell in covered {
        let live = reads
            .iter()
            .filter(|read| read.cells[cell].confidence > 0.0)
            .count();
        let arrived = reads
            .iter()
            .position(|read| read.cells[cell].confidence > 0.0);
        let last = reads.last().expect("a run has frames").cells[cell];
        println!(
            "  {cell:>5} {:>7.1}d {live:>7} {:>7} {:>10.3}d {:>10.3}d {:>11.3} {:>11.3}",
            cell as f64 / AZIMUTHS as f64 * 360.0,
            arrived.map_or_else(|| "never".to_owned(), |frame| frame.to_string()),
            f64::from(reads[0].cells[cell].disparity.to_degrees()),
            f64::from(last.disparity.to_degrees()),
            last.confidence,
            last.trust,
        );
    }
}

/// How MUCH correction is delivered at his pixels, frame by frame, and how
/// much of the reading it is.
///
/// Every other column in `mode=snap` is about the STEPS the delivered field
/// takes; this one is about the SIZE of the thing that is stepping. Two arms
/// can agree on every step and disagree on how much correction is on the
/// screen the whole time, and that difference is invisible to a step census
/// by construction. It is the column the arrival-staging question needs: what
/// staging costs is not a jump, it is magnitude, and magnitude has to be
/// measured over time to say whether the cost is transient or steady.
///
/// `epi mean` and `epi peak` are over the probes, in view px of this view.
/// `gate` is the mean of [`Cell::trust`] over the cells this view stands on
/// that hold evidence at some point in the run - a fixed denominator, so a
/// gate that walks in is a column that rises rather than a mean over a
/// changing set. `read px` is what those same cells would deliver with the
/// gate wide open, so `epi mean / read px` is the fraction of its own measured
/// answer the pass is actually spending.
fn weight(reads: &[Read], probes: &[Probe], reframe: &Reframe, size: Size, px_per_deg: f64) {
    let covered = covering(reframe, size, [0, 0, size.width, size.height]);
    // The cells that ever hold evidence, fixed over the run: a mean taken over
    // whichever cells happen to be live on a frame rises when cells drop out,
    // which is the opposite of what it would be read as.
    let ever: Vec<usize> = covered
        .iter()
        .copied()
        .filter(|cell| reads.iter().any(|read| read.cells[*cell].confidence > 0.0))
        .collect();
    let each: Vec<(f64, f64, f64, f64, usize)> = reads
        .iter()
        .map(|read| {
            let across: Vec<f64> = probes
                .iter()
                .map(|probe| deliver(read, read, probe, px_per_deg).epi_px.abs())
                .collect();
            let mean = across.iter().sum::<f64>() / across.len().max(1) as f64;
            let peak = across.iter().copied().fold(0.0, f64::max);
            let gate = match ever.is_empty() {
                true => 0.0,
                false => {
                    ever.iter()
                        .map(|cell| f64::from(read.cells[*cell].trust))
                        .sum::<f64>()
                        / ever.len() as f64
                }
            };
            // The same probes with the gate taken out, which is the correction
            // the band has MEASURED rather than the one it is spending.
            let held = probes
                .iter()
                .map(|probe| {
                    let at = deliver(read, read, probe, px_per_deg);
                    let gate = 0.5 * f64::from(at.trust[0] + at.trust[1]);
                    match gate > 0.0 {
                        true => at.epi_px.abs() / gate,
                        false => 0.0,
                    }
                })
                .sum::<f64>()
                / probes.len().max(1) as f64;
            let live = ever
                .iter()
                .filter(|cell| read.cells[**cell].confidence > 0.0)
                .count();
            (mean, peak, gate, held, live)
        })
        .collect();
    println!(
        "\nthe correction's own SIZE at his pixels, frame by frame over {} probes. `epi mean` and \n\
         \t`epi peak` are |across-seam| in view px; `gate` is the mean trust over the {} cells \n\
         \tthis view stands on that ever hold evidence; `read px` is what those probes would \n\
         \tdeliver with the gate wide open; `spent` is the first over the last.\n",
        probes.len(),
        ever.len(),
    );
    println!(
        "  {:>6} {:>9} {:>10} {:>10} {:>8} {:>10} {:>8} {:>7}",
        "frame", "at s", "epi mean", "epi peak", "gate", "read px", "spent", "live",
    );
    for (frame, (mean, peak, gate, held, live)) in each.iter().enumerate() {
        println!(
            "  {frame:>6} {:>9.3} {mean:>10.2} {peak:>10.2} {gate:>8.3} {held:>10.2} {:>7.1}% \
             {live:>7}",
            reads[frame].at.as_secs_f64(),
            match *held > 0.0 {
                true => 100.0 * mean / held,
                false => 0.0,
            },
        );
    }
    // Windows of a second, so transient and steady state are two rows of the
    // same table rather than an argument about a graph.
    let window = 30;
    println!(
        "\n  and the same in {window}-frame (one second) windows, which is what a transient and a \n\
         \tsteady state look like beside each other.\n"
    );
    println!(
        "  {:>12} {:>10} {:>10} {:>8} {:>10}",
        "window", "epi mean", "epi peak", "gate", "read px",
    );
    for start in (0..each.len()).step_by(window) {
        let slice = &each[start..(start + window).min(each.len())];
        let over = |of: fn(&(f64, f64, f64, f64, usize)) -> f64| {
            slice.iter().map(of).sum::<f64>() / slice.len().max(1) as f64
        };
        println!(
            "  {:>5}..{:<5} {:>10.2} {:>10.2} {:>8.3} {:>10.2}",
            start,
            (start + window).min(each.len()) - 1,
            over(|row| row.0),
            slice.iter().map(|row| row.1).fold(0.0, f64::max),
            over(|row| row.2),
            over(|row| row.3),
        );
    }
}

/// How often the cells this view stands on LOSE their evidence and get it
/// back, and what their gate did while they were away.
///
/// [`support`] says how many frames a cell was live and which frame it first
/// read on. Neither answers the question arrival staging raises, which is how
/// many times a cell arrives: every one of those is a walk restarted on the
/// staged arm and a whole correction switched on on the arm that does not
/// stage. `up` is a dead-to-live transition after the first frame, `down` is
/// the other way, and the frames the ups happened on are printed because a
/// count cannot say whether they are spread over the run or bunched at one
/// moment.
///
/// **The gate columns are the price.** `gate min` is the lowest trust the cell
/// held on any frame it was live, and `under half` is how many of those frames
/// it spent below half of the reading it holds. On the arm that applies an
/// arrival whole those two are 1.000 and 0 by construction unless the
/// confidence itself sags; on the staged arm they are what the walk costs.
fn arrivals(reads: &[Read], reframe: &Reframe, size: Size) {
    let covered = covering(reframe, size, [0, 0, size.width, size.height]);
    println!(
        "\nthe arrivals at the {} cells this view stands on, over {} frames. `up` counts the \n\
         \tframes a cell went from no evidence to some AFTER the first, which is one restarted \n\
         \twalk each on the staged arm; `down` is the other way.\n",
        covered.len(),
        reads.len(),
    );
    println!(
        "  {:>5} {:>8} {:>6} {:>5} {:>6} {:>9} {:>10} {:>11}   up frames",
        "cell", "phi", "live", "up", "down", "gate min", "gate last", "under half",
    );
    let (mut ups, mut sagged) = (0usize, 0usize);
    for cell in covered {
        let live: Vec<bool> = reads
            .iter()
            .map(|read| read.cells[cell].confidence > 0.0)
            .collect();
        let up: Vec<usize> = (1..live.len())
            .filter(|f| !live[f - 1] && live[*f])
            .collect();
        let down = (1..live.len()).filter(|f| live[f - 1] && !live[*f]).count();
        let gates: Vec<f64> = reads
            .iter()
            .enumerate()
            .filter(|(frame, _)| live[*frame])
            .map(|(_, read)| f64::from(read.cells[cell].trust))
            .collect();
        let under = gates.iter().filter(|gate| **gate < 0.5).count();
        ups += up.len();
        sagged += under;
        println!(
            "  {cell:>5} {:>7.1}d {:>6} {:>5} {:>6} {:>9.3} {:>10.3} {:>11}   {}",
            cell as f64 / AZIMUTHS as f64 * 360.0,
            live.iter().filter(|on| **on).count(),
            up.len(),
            down,
            gates.iter().copied().fold(f64::INFINITY, f64::min),
            f64::from(reads.last().expect("a run has frames").cells[cell].trust),
            under,
            up.iter()
                .take(8)
                .map(usize::to_string)
                .collect::<Vec<String>>()
                .join(","),
        );
    }
    println!(
        "\n  {ups} arrivals after the first frame over the whole arc, and {sagged} cell-frames \n\
         \tspent live with the gate under half."
    );
}

/// Every frame-to-frame step at every probe, decomposed and classed.
fn attribute(reads: &[Read], probes: &[Probe], px_per_deg: f64, axis: Axis) -> Vec<Step> {
    let mut steps = Vec::new();
    for frame in 1..reads.len() {
        for (index, probe) in probes.iter().enumerate() {
            let now = deliver(&reads[frame], &reads[frame], probe, px_per_deg);
            let was = deliver(&reads[frame - 1], &reads[frame - 1], probe, px_per_deg);
            // This frame's map, last frame's state: the one evaluation that
            // splits the two, and the reason this is an attribution rather
            // than a second opinion.
            let cross = deliver(&reads[frame], &reads[frame - 1], probe, px_per_deg);
            let sweep = axis.of(&cross) - axis.of(&was);
            let state = axis.of(&now) - axis.of(&cross);
            let arrived = [now.low, (now.low + 1) % AZIMUTHS].iter().any(|cell| {
                reads[frame - 1].cells[*cell].confidence <= 0.0
                    && reads[frame].cells[*cell].confidence > 0.0
            });
            let class = match (sweep.abs() >= state.abs(), arrived) {
                (true, _) => Class::Sweep,
                (false, true) => Class::Arrival,
                (false, false) => Class::Commit,
            };
            steps.push(Step {
                frame,
                probe: index,
                total: axis.of(&now) - axis.of(&was),
                sweep,
                state,
                class,
                arrived,
                phi_deg: now.phi_deg,
                low: now.low,
            });
        }
    }
    steps
}

/// How the steps of at least `floor` view px divide between the three classes.
fn census(steps: &[Step], floor: f64) {
    let counted: Vec<&Step> = steps
        .iter()
        .filter(|step| step.total.abs() >= floor)
        .collect();
    println!(
        "\n  {:>9} {:>8} {:>10} {:>10} {:>10}   steps of {floor:.0} px or more",
        "class", "steps", "share", "worst px", "sum px",
    );
    for class in [Class::Sweep, Class::Commit, Class::Arrival] {
        let mine: Vec<&&Step> = counted.iter().filter(|step| step.class == class).collect();
        let worst = mine.iter().map(|step| step.total.abs()).fold(0.0, f64::max);
        let sum: f64 = mine.iter().map(|step| step.total.abs()).sum();
        println!(
            "  {:>9} {:>8} {:>9.1}% {:>10.1} {:>10.1}",
            class.name(),
            mine.len(),
            100.0 * mine.len() as f64 / counted.len().max(1) as f64,
            worst,
            sum,
        );
    }
}

/// The two terms on their own, over every probe-frame, rather than the total
/// of the steps a class won.
///
/// **The census above answers a different question from the one an attribution
/// asks.** It sorts each step by which term was larger and then reports that
/// step's TOTAL, so a 18.9 px step made of 12.8 px of sweep and 6.1 px of
/// state is filed under sweep at 18.9 px and neither number in it is the
/// sweep. This is the decomposition itself: how much of the delivered motion,
/// summed over the run, each of the three hypotheses is actually carrying.
///
/// The state term is split by whether a cell behind the probe arrived on that
/// frame, which is H3 against H2, and the split is by [`Step::arrived`] rather
/// than by the class so that an arrival hiding under a larger sweep still
/// counts as one.
fn terms(steps: &[Step]) {
    let sum = |over: &[&Step], of: fn(&Step) -> f64| -> (f64, f64) {
        (
            over.iter().map(|step| of(step).abs()).sum(),
            over.iter().map(|step| of(step).abs()).fold(0.0, f64::max),
        )
    };
    let all: Vec<&Step> = steps.iter().collect();
    let arriving: Vec<&Step> = steps.iter().filter(|step| step.arrived).collect();
    let holding: Vec<&Step> = steps.iter().filter(|step| !step.arrived).collect();
    println!(
        "\n  the three hypotheses as TERMS and not as classes: every probe-frame counted, each \n\
         \tone contributing its own two numbers rather than its total to whichever won.\n"
    );
    println!(
        "  {:>26} {:>10} {:>10} {:>10}",
        "term", "steps", "sum px", "worst px",
    );
    for (name, over, of) in [
        (
            "H1 sweep (map moved)",
            &all,
            (|step: &Step| step.sweep) as fn(&Step) -> f64,
        ),
        ("H2 state, no arrival", &holding, |step: &Step| step.state),
        ("H3 state, on arrival", &arriving, |step: &Step| step.state),
    ] {
        let (total, worst) = sum(over, of);
        println!(
            "  {name:>26} {:>10} {total:>10.1} {worst:>10.1}",
            over.len()
        );
    }
}

/// The verdict: how the steps divide between the three, and the largest few
/// with their own numbers beside them.
fn verdict(steps: &[Step], reads: &[Read], probes: usize, axis: Axis) {
    let closure = steps
        .iter()
        .map(|step| (step.total - (step.sweep + step.state)).abs())
        .fold(0.0, f64::max);
    let counted: Vec<&Step> = steps
        .iter()
        .filter(|step| step.total.abs() >= STEP_PX)
        .collect();
    println!(
        "\nthe steps on the axis {}, over {} probe-frames ({probes} probes across {} frames). a \n\
         step is counted where the delivered correction moved {STEP_PX:.0} view px or more between \n\
         two frames. closure, the worst |total - (sweep + state)| over every pair: \n\
         {closure:.2e} px.\n",
        axis.name(),
        steps.len(),
        reads.len(),
    );
    census(steps, STEP_PX);
    // The memo's own threshold as well as this instrument's, because that is
    // the number the A/B was staged on: 83 steps of over ten view px at
    // `down1`, counted at a CELL, against 4 on the arm that filters the gate.
    census(steps, MEMO_PX);
    terms(steps);
    let all = (steps
        .iter()
        .map(|step| step.total * step.total)
        .sum::<f64>()
        / steps.len().max(1) as f64)
        .sqrt();
    let busy = |floor: f64| {
        let mut frames: Vec<usize> = steps
            .iter()
            .filter(|step| step.total.abs() >= floor)
            .map(|step| step.frame)
            .collect();
        frames.sort_unstable();
        frames.dedup();
        frames.len()
    };
    println!(
        "\n  the pace: over every probe-frame, not only the counted ones, the delivered step is \n\
         \t{all:.2} px rms. {} of {} frames carry a step of {STEP_PX:.0} px or more somewhere in \n\
         \tthe picture, and {} carry one of {MEMO_PX:.0} px or more.",
        busy(STEP_PX),
        reads.len() - 1,
        busy(MEMO_PX),
    );

    let mut largest: Vec<&&Step> = counted.iter().collect();
    largest.sort_by(|a, b| b.total.abs().partial_cmp(&a.total.abs()).expect("finite"));
    println!("\n  the twelve largest, with the two terms they are made of\n");
    println!(
        "  {:>6} {:>6} {:>8} {:>6} {:>10} {:>10} {:>10} {:>9}",
        "frame", "probe", "phi", "cell", "total px", "sweep px", "state px", "class",
    );
    for step in largest.iter().take(12) {
        println!(
            "  {:>6} {:>6} {:>7.2}d {:>6} {:>10.2} {:>10.2} {:>10.2} {:>9}",
            step.frame,
            step.probe,
            step.phi_deg,
            step.low,
            step.total,
            step.sweep,
            step.state,
            step.class.name(),
        );
    }
}

/// Where the plant landed, as the difference between the run with it and the
/// same run without.
///
/// **This is the positive control, and it is a difference and not a total.**
/// The footage's own steps are large and busy at these views, so a plant read
/// out of a summary is a plant read out of noise. Differenced step by step,
/// against a null that shares its geometry and its decode exactly, what is
/// left is the plant and nothing else, and the question is which of the two
/// terms it went into.
fn plant_check(plant: Plant, planted: &[Step], null: &[Step]) {
    let mut sweep = 0.0;
    let mut state = 0.0;
    let mut worst: Option<(f64, &Step, f64, f64)> = None;
    // Where the difference sits in TIME. A plant with a frame in its name is
    // only caught if the frame it names is the frame that moved, and a total
    // cannot say that.
    let mut per_frame: BTreeMap<usize, f64> = BTreeMap::new();
    for (with, without) in planted.iter().zip(null) {
        let (moved, held) = (with.sweep - without.sweep, with.state - without.state);
        let size = (moved + held).abs();
        sweep += moved.abs();
        state += held.abs();
        *per_frame.entry(with.frame).or_default() += size;
        if worst.is_none_or(|(largest, ..)| size > largest) {
            worst = Some((size, with, moved, held));
        }
    }
    let total = sweep + state;
    // What the PLANTED run's own two terms weigh, which is the freeze plants'
    // whole point: [`Plant::Hold`] must leave zero state and [`Plant::Still`]
    // zero sweep, exactly, and neither is a claim about a difference.
    let (pure_sweep, pure_state) = planted.iter().fold((0.0, 0.0), |(a, b), step| {
        (a + step.sweep.abs(), b + step.state.abs())
    });
    if plant == Plant::None {
        println!(
            "\nnull:   no plant, so the run and its own null are the same run: {total:.2e} px of \n\
             \tdifference over every probe-frame. a difference reported below a plant is the \n\
             \tplant, and never the instrument reading one run twice."
        );
        return;
    }
    // A control that cannot fail is not a control. `Still` used to land here
    // with an exact zero because the null was rebuilt out of the frozen map
    // rather than the real one, so it reported a clean pass having measured
    // nothing; the saved map fixed that, and this line is what would have said
    // so at the time.
    if total <= 0.0 {
        println!(
            "\nplant:  REFUSED. this run carries a plant and its difference against its own null \n\
             \tis exactly zero, so the null is not a null: whatever the plant changed was \n\
             \talso in the run it was differenced against. nothing below is a control."
        );
        return;
    }
    let mut busiest: Vec<(&usize, &f64)> = per_frame.iter().collect();
    busiest.sort_by(|a, b| b.1.partial_cmp(a.1).expect("finite"));
    let worst = worst.expect("a run has steps");
    println!(
        "\nplant:  what the plant moved, differenced against the same run without it: \n\
         \t{:.1} px went into SWEEP and {:.1} px into STATE, which is {:.0}% and {:.0}%. \n\
         \tits largest single step is {:.1} px at frame {}, probe {} ({:.1} sweep, {:.1} state), \n\
         \tand the attribution called THAT step {}. the three frames carrying the most of it \n\
         \tare {}.",
        sweep,
        state,
        100.0 * sweep / total,
        100.0 * state / total,
        worst.0,
        worst.1.frame,
        worst.1.probe,
        worst.2,
        worst.3,
        worst.1.class.name(),
        busiest
            .iter()
            .take(3)
            .map(|(frame, px)| format!("{frame} ({px:.1} px)"))
            .collect::<Vec<String>>()
            .join(", "),
    );
    // The freeze plants read out here and not above, because what they assert
    // is about the planted run's own arithmetic and an exact zero in it, not
    // about a difference against a null: `Hold` freezes the state, so every
    // step it leaves is the map moving under it, and `Still` is the mirror.
    // Their difference against the null is the term they REMOVED and is
    // reported as such rather than as the term they went into.
    match plant {
        Plant::Hold(_) => println!(
            "\t--- and the assertion this plant is here for: with the state frozen the planted \n\
             \trun's own STATE term weighs {pure_state:.2e} px over every probe-frame, against \n\
             \t{pure_sweep:.1} px of sweep. the {state:.1} px of state in the difference above is \n\
             \tthe term the freeze TOOK OUT of the run, which is the null's and not the plant's."
        ),
        Plant::Still => println!(
            "\t--- and the assertion this plant is here for: with the map frozen the planted \n\
             \trun's own SWEEP term weighs {pure_sweep:.2e} px over every probe-frame, against \n\
             \t{pure_state:.1} px of state. the {sweep:.1} px of sweep in the difference above is \n\
             \tthe term the freeze TOOK OUT of the run, which is the null's and not the plant's."
        ),
        _ => {}
    }
}

/// What one cell of the ring differs from the next by, in the state the run
/// ended in.
///
/// These are the sizes a sweeping probe collects as it crosses the ring, so if
/// the sweep term above is what dominates, this table is what its steps are
/// made of.
fn neighbours(last: &Read, reframe: &Reframe, size: Size, px_per_deg: f64) {
    let covered = covering(reframe, size, [0, 0, size.width, size.height]);
    let applied = |cell: usize| {
        let cell = last.cells[cell % AZIMUTHS];
        f64::from((cell.disparity * cell.trust).to_degrees()) * px_per_deg
    };
    let deltas: Vec<(usize, f64)> = (0..AZIMUTHS)
        .map(|cell| (cell, applied(cell + 1) - applied(cell)))
        .collect();
    let rms = |over: &[(usize, f64)]| match over.is_empty() {
        true => 0.0,
        false => (over.iter().map(|(_, d)| d * d).sum::<f64>() / over.len() as f64).sqrt(),
    };
    let arc: Vec<(usize, f64)> = deltas
        .iter()
        .filter(|(cell, _)| covered.contains(cell))
        .copied()
        .collect();
    let mut worst = deltas.clone();
    worst.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).expect("finite"));
    println!(
        "\nwhat one cell differs from the next by, in the state the run ended in, as the view px \n\
         a probe crossing that pair collects. ring-wide rms {:.1} px over {} pairs; over the {} \n\
         cells this view stands on, rms {:.1} px, worst {:.1} px.\n",
        rms(&deltas),
        deltas.len(),
        arc.len(),
        rms(&arc),
        arc.iter().map(|(_, d)| d.abs()).fold(0.0, f64::max),
    );
    println!(
        "  {:>10} {:>9} {:>10} {:>12}",
        "pair", "phi", "delta px", "in view"
    );
    for (cell, delta) in worst.iter().take(10) {
        println!(
            "  {:>4} -> {:<4} {:>8.1}d {:>10.1} {:>12}",
            cell,
            (cell + 1) % AZIMUTHS,
            *cell as f64 / AZIMUTHS as f64 * 360.0,
            delta,
            match covered.contains(cell) {
                true => "yes",
                false => "",
            },
        );
    }
}

/// The same over EVERY frame of the run rather than the one it ended on, and
/// as a distribution rather than a top ten.
///
/// The end state is one sample. What a probe crossing his arc actually
/// collects is drawn from the whole run's worth of them, and the shape of that
/// draw is the answer to "how big is one cell of quantization here".
///
/// **Two columns and not one, because they are two different questions.**
/// `read` is what the correlation put in the cells; `applied` is that taxed by
/// the gate on the way out ([`Cell::trust`], the filtered one the pass stores,
/// so this reads the picture rather than a model of it). A ring whose readings
/// agree and whose gates do not still delivers a step, and only the second
/// column can see it.
fn spread(reads: &[Read], reframe: &Reframe, size: Size, px_per_deg: f64) {
    let covered = covering(reframe, size, [0, 0, size.width, size.height]);
    let mut read: Vec<f64> = Vec::new();
    let mut applied: Vec<f64> = Vec::new();
    let mut live: Vec<f64> = Vec::new();
    for frame in reads {
        for cell in &covered {
            let (a, b) = (frame.cells[*cell], frame.cells[(cell + 1) % AZIMUTHS]);
            let degrees = |value: f32| f64::from(value.to_degrees()) * px_per_deg;
            read.push(degrees(b.disparity - a.disparity));
            applied.push(degrees(b.disparity * b.trust - a.disparity * a.trust));
            // A pair with nothing behind either cell delivers nothing whatever
            // its readings say, and counting those in with the rest is how a
            // distribution over a mostly-empty arc reports a zero it never
            // drew. This column is the pairs where at least one side is live.
            if a.confidence > 0.0 || b.confidence > 0.0 {
                live.push(degrees(b.disparity * b.trust - a.disparity * a.trust));
            }
        }
    }
    let quantile = |over: &mut Vec<f64>, at: f64| {
        if over.is_empty() {
            return 0.0;
        }
        over.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        over[((over.len() - 1) as f64 * at) as usize]
    };
    let size_of = |over: &[f64]| {
        let mut sizes: Vec<f64> = over.iter().map(|delta| delta.abs()).collect();
        let count = sizes.len();
        let rms = (sizes.iter().map(|d| d * d).sum::<f64>() / count.max(1) as f64).sqrt();
        (
            count,
            rms,
            quantile(&mut sizes, 0.5),
            quantile(&mut sizes, 0.9),
            quantile(&mut sizes, 0.99),
            sizes.last().copied().unwrap_or_default(),
        )
    };
    println!(
        "\nthe adjacent-cell delta at HIS ARC, over every frame of the run and every one of the \n\
         \t{} covered pairs, in view px of this view. `read` is what the cells hold; `applied` \n\
         \tis that times the gate the run is on; `live` is `applied` over the pairs with \n\
         \tevidence behind at least one side.\n",
        covered.len(),
    );
    println!(
        "  {:>9} {:>8} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "column", "pairs", "rms", "median", "p90", "p99", "worst",
    );
    for (name, over) in [("read", &read), ("applied", &applied), ("live", &live)] {
        let (count, rms, median, p90, p99, worst) = size_of(over);
        println!(
            "  {name:>9} {count:>8} {rms:>10.1} {median:>10.1} {p90:>10.1} {p99:>10.1} \
             {worst:>10.1}",
        );
    }
}

// ------------------------------------------------------------ the coverage

/// How long a stretch of coverage one column of the table covers, in seconds
/// of media time.
///
/// A minute is the unit the question is asked in - "does live accumulation
/// over minutes of playback close the arc" - and six columns of ten seconds
/// says whether an arc that closes closes early or late.
const COVERAGE_STEP_S: f64 = 10.0;

/// The correlation the #171 harvest required of a reading before it would
/// accumulate one, against this instrument's smoothed proxy for it.
///
/// **A proxy and not the number.** The harvest gated the raw peak of one
/// probe; what a cell carries is [`Cell::confidence`], the same peak smoothed
/// over that direction's own time constant. They are the same quantity at
/// different lags, so a `firm` column here is neither an upper nor a lower
/// bound on what the harvest would have taken, and it is here to say whether
/// the arc is anywhere near the floor rather than to count what would pass it.
const FIRM: f32 = 0.80;

/// **The C2 coverage gate** (docs/research/seam-temporal.md, increment 3):
/// which directions of the circle a normal play ever gets a reading at, and
/// how that fills in over minutes.
///
/// The question is one specific one. A 6x4 harvest over the whole of the
/// owner's May-01 file read **1 of the 11 cells his own downward arc sits on**
/// (`scratch/refusals.log` on `research/v6-player`), and a per-session field
/// that is identity where he is looking is #171's coverage failure again. The
/// band already measures all 128 directions every frame while the film plays,
/// so the memo's opening is that live accumulation might close what a harvest
/// could not. This measures whether it does. **It builds nothing.**
///
/// **What counts as a reading, and the domain that puts on every number
/// below.** The band writes a direction's disparity only on a visit where the
/// correlation passed [`kjerag_render::KEEP`] and peaked inside the search
/// window; a refused visit gives up evidence and leaves the measurement
/// exactly where it was (`settle` and `forget` in the render crate's band).
/// So a disparity that MOVED is a reading accepted, and that is what this
/// counts. It is **the band's own gate and not the harvest's**: #171 stacked a
/// far gate, a trimmed middle and a five-term shape gate on top, so these
/// counts are the most a live accumulator could ever see and not what it would
/// keep. A `no` here kills C2; a `yes` licenses only the next measurement.
fn over_time(options: &Options) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let mut scene = Scene::still(&options.input, options.at())?;
    scene.set_horizon(match options.lock {
        true => Horizon::Locked,
        false => Horizon::Free,
    });
    options.seam.hold(&scene);

    let mut held = vec![f32::NAN; AZIMUTHS];
    let mut read = vec![0usize; AZIMUTHS];
    let mut firm = vec![0usize; AZIMUTHS];
    let mut first = vec![f64::NAN; AZIMUTHS];
    let mut columns: Vec<(f64, Vec<usize>)> = Vec::new();
    let mut frames = 0usize;
    let mut elapsed = 0.0;
    let start = scene.frame().map(|(_, at)| at.as_secs_f64()).unwrap_or(0.0);

    while let Some((_, at)) = scene.frame() {
        Render {
            gpu: &gpu,
            scene: &scene,
            pipeline: &mut pipeline,
        }
        .frame(options.camera(), Sampling::default(), options.size())?;
        let (_, cells) = pipeline.band_state(&gpu.device, &gpu.queue)?;
        elapsed = at.as_secs_f64() - start;
        for (index, cell) in cells.iter().enumerate() {
            if held[index] == cell.disparity {
                continue;
            }
            held[index] = cell.disparity;
            // The first frame writes every direction's zero into `held`, and
            // that is not a reading of anything.
            if frames == 0 {
                continue;
            }
            read[index] += 1;
            if cell.confidence >= FIRM {
                firm[index] += 1;
            }
            if first[index].is_nan() {
                first[index] = elapsed;
            }
        }
        frames += 1;
        if elapsed >= (columns.len() + 1) as f64 * COVERAGE_STEP_S {
            columns.push((elapsed, read.clone()));
        }
        if frames >= options.count || !scene.advance()? {
            break;
        }
    }
    columns.push((elapsed, read.clone()));

    let arc = arc_cells(options);
    println!(
        "\nfile:   {}\nplayed: {frames} frames, {elapsed:.1} s of media time from {:.1} s\n\
         arc:    cells {} to {} of {AZIMUTHS}, azimuth {:.1} to {:.1} deg, the owner's downward \
         views\ngate:   a reading is a visit the band ACCEPTED, which is its own KEEP and not the \
         harvest's stack",
        options.input.display(),
        options.from,
        arc.start,
        arc.end - 1,
        arc.start as f64 / AZIMUTHS as f64 * 360.0,
        (arc.end - 1) as f64 / AZIMUTHS as f64 * 360.0,
    );

    println!("\n  cell  phi deg   first s     firm   reads at each {COVERAGE_STEP_S:.0} s");
    for cell in 0..AZIMUTHS {
        let inside = arc.contains(&cell);
        if !inside && read[cell] > 0 && cell % 8 != 0 {
            continue;
        }
        let over_time: String = columns
            .iter()
            .map(|(_, counts)| format!("{:>6}", counts[cell]))
            .collect();
        println!(
            "  {cell:>4} {:>8.1} {:>9} {:>8}  {over_time}{}",
            cell as f64 / AZIMUTHS as f64 * 360.0,
            match first[cell].is_nan() {
                true => "never".to_string(),
                false => format!("{:.1}", first[cell]),
            },
            firm[cell],
            match inside {
                true => "   <- his arc",
                false => "",
            },
        );
    }

    let ever =
        |counts: &[usize], want: usize| arc.clone().filter(|cell| counts[*cell] >= want).count();
    let whole =
        |counts: &[usize], want: usize| (0..AZIMUTHS).filter(|c| counts[*c] >= want).count();
    println!(
        "\narc:    {} of {} cells read at all, {} read 10 times or more, {} read 100 or more",
        ever(&read, 1),
        arc.len(),
        ever(&read, 10),
        ever(&read, 100),
    );
    println!(
        "ring:   {} of {AZIMUTHS} directions read at all, {} read 10 times or more, {} 100 or more",
        whole(&read, 1),
        whole(&read, 10),
        whole(&read, 100),
    );
    println!(
        "firm:   {} of {} arc cells reached {FIRM} smoothed confidence on a read, {} of {AZIMUTHS} \
         on the whole ring",
        ever(&firm, 1),
        arc.len(),
        whole(&firm, 1),
    );
    Ok(())
}

/// The cells the arc the owner is looking at lands on.
///
/// His three banked downward views sit between 93 and 125 degrees of azimuth,
/// which `--bin refusals` reported as cells 34 to 44 and read 1 of. Taken off
/// the same azimuths here so the two are the same eleven cells.
fn arc_cells(options: &Options) -> std::ops::Range<usize> {
    let cell = |deg: f64| (deg / 360.0 * AZIMUTHS as f64).round() as usize;
    let (low, high) = options.arc;
    cell(low)..cell(high) + 1
}

// ------------------------------------------------------------ the cost

/// What the band's measurement costs per redraw, with the decode outside the
/// timing and the box's own load outside the answer (issue #103, stage 6).
///
/// `--bin playback` reports a whole-pass number that includes the decode, the
/// pacing and whatever else the box is doing, and on a loaded box its spread is
/// wider than the thing being measured: six alternating runs of two builds
/// under a load average of 21 came back 5.1 to 20.3 ms with the two builds
/// interleaved, which measures the box. This times **one call** - the prepare
/// that dispatches the compute pass, the draw, and the readback that waits for
/// both - over the same decoded frames, twice, with the band held and with it
/// live. The decode happens between the timed calls.
///
/// **The minimum is the answer and the median is the check.** A redraw cannot
/// go faster than the work in it, so the fastest of many is the least
/// contended; a median far above it says the box was busy and not that the pass
/// is slow.
///
/// **The first frame is priced on its own.** The shipped search is two
/// searches: a direction that has not got the along-seam axis looks over the
/// whole window, and one that has looks two steps either side of what it holds
/// (`NARROW_STEPS` in the render crate's `band`). The first frame after a seek
/// is every direction in the first state and every frame after it is nearly all
/// of them in the second, so one average over a run would price neither of the
/// two things the player actually does.
fn cost(options: &Options) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let mut taken: Vec<(u32, Vec<f64>)> = Vec::new();
    for repeats in [1u32, 1 + REPEATS] {
        let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
        let mut scene = Scene::still(&options.input, options.at())?;
        scene.set_horizon(match options.lock {
            true => Horizon::Locked,
            false => Horizon::Free,
        });
        options.seam.hold(&scene);
        pipeline.band_repeats(repeats);
        let mut each = Vec::with_capacity(options.count);
        while scene.frame().is_some() {
            let started = std::time::Instant::now();
            Render {
                gpu: &gpu,
                scene: &scene,
                pipeline: &mut pipeline,
            }
            .frame(options.camera(), Sampling::default(), options.size())?;
            each.push(started.elapsed().as_secs_f64() * 1e3);
            if each.len() >= options.count || !scene.advance()? {
                break;
            }
        }
        taken.push((repeats, each));
    }
    println!(
        "\ncost:   {} redraws at {}x{}, yaw {:.0} fov {:.0}, the decode outside the timing.\n\
         \x20       the minimum is the answer: a redraw cannot go faster than the work in it.\n",
        options.count,
        options.size().width,
        options.size().height,
        options.yaw,
        options.fov,
    );
    println!(
        "{:<16} {:>10} {:>10} {:>10} {:>10}",
        "dispatches", "seek ms", "then min", "median", "worst"
    );
    let mut floor = (0.0, 0.0);
    for (repeats, each) in &taken {
        let (seek, steady) = split(each);
        let mut sorted = steady.to_vec();
        sorted.sort_by(f64::total_cmp);
        let min = sorted.first().copied().unwrap_or(0.0);
        println!(
            "{repeats:<16} {seek:>10.3} {min:>10.3} {:>10.3} {:>10.3}",
            sorted[sorted.len() / 2],
            sorted.last().copied().unwrap_or(0.0),
        );
        match *repeats {
            1 => floor = (seek, min),
            _ => {
                let each = |now: f64, was: f64| (now - was) / f64::from(REPEATS);
                println!(
                    "\nthe band's measurement costs {:.3} ms on the frame a seek lands on, where \n\
                     every direction searches the whole along-seam window, and {:.3} ms on every \n\
                     frame after it, where the ones that have the axis look two steps either \n\
                     side. That is {:.1} and {:.1} percent of the 16.6 ms a 60 fps frame has. \n\
                     Both are slopes over {REPEATS} extra dispatches.",
                    each(seek, floor.0),
                    each(min, floor.1),
                    100.0 * each(seek, floor.0) / 16.6,
                    100.0 * each(min, floor.1) / 16.6,
                );
            }
        }
    }
    Ok(())
}

/// The frame a seek landed on, and every frame after it.
fn split(each: &[f64]) -> (f64, &[f64]) {
    match each.split_first() {
        Some((seek, rest)) if !rest.is_empty() => (*seek, rest),
        _ => (each.first().copied().unwrap_or(0.0), each),
    }
}

/// How many extra dispatches the slope is taken over.
///
/// Enough that the pass is much larger than the noise around it and few enough
/// that a run is quick: at a couple of milliseconds a dispatch, sixteen of them
/// is thirty milliseconds of work against a spread of a few.
const REPEATS: u32 = 16;

// ------------------------------------------------------------ pictures

/// A stretch drawn frame by frame, so a rolly moment can be looked at as film
/// rather than as a still.
fn sequence(options: &Options) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let out = options.out();
    std::fs::create_dir_all(&out)?;
    let stem = options.stem();
    let reads = play(&gpu, options, &mut pipeline, |picture, frame| {
        picture.save(&gpu, &out.join(format!("{stem}-{frame:04}.png")))
    })?;
    println!("wrote {} frames into {}", reads.len(), out.display(),);
    flicker(&reads, options);
    Ok(())
}

/// One view drawn with the band and without it, and the difference at 8x.
///
/// The two renders are the same frame, the same angles and the same pass, so
/// they differ by the band and by nothing else. `4-what-moved` is the one to
/// look at first: it has to be flat grey everywhere except a strip along the
/// seam.
fn render(options: &Options) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let out = options.out();
    std::fs::create_dir_all(&out)?;
    let stem = options.stem();

    // The same frame both ways, so the two differ by the band and by nothing
    // else: same file, same instant, same run length, same pass, two opens.
    let draw = |off: bool| -> Fallible<Read> {
        let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
        let mut options = options.clone();
        options.off = off;
        let mut reads = play(&gpu, &options, &mut pipeline, |_, _| Ok(()))?;
        Ok(reads.pop().expect("play returns at least one frame"))
    };
    let stage1 = draw(true)?;
    let last = draw(false)?;
    let (plain, banded) = (&stage1.picture, &last.picture);
    plain.save(&gpu, &out.join(format!("{stem}-1-stage1.png")))?;
    banded.save(&gpu, &out.join(format!("{stem}-2-band.png")))?;
    banded
        .amplified(plain)
        .save(&gpu, &out.join(format!("{stem}-4-what-moved.png")))?;
    let against = plain.against(banded);
    share(options, &last.mapped, plain, banded)?;
    println!(
        "\nwrote three pictures into {} at yaw {:.1}, pitch {:.1}, fov {:.1}.\n{}",
        out.display(),
        options.yaw,
        options.pitch,
        options.fov,
        against.report(),
    );
    Ok(())
}

/// The seam band's share of the picture's own sharpness, before and after.
///
/// The same statistic `--bin seam mode=parity` scores a rival stitch with, and
/// the same definition: mean squared horizontal luma gradient over the pixels
/// within 5 degrees of the seam, over the same statistic 9 to 25 degrees off
/// it in the same picture. A doubled edge lowers it and a single one does not,
/// so an alignment that stops drawing content twice RAISES it.
///
/// Scored here on **our own two pictures** rather than against the camera
/// maker's export, and that is the whole point of doing it here: each picture
/// is its own control, no projection has to be fitted, and there is no view
/// for a fit to get wrong. What it cannot say is how we compare to them.
///
/// **Both windows were drawn around a 2 degree handover and neither has
/// moved.** At the 8 the pass hands over across, the band plus the bend it
/// carries reaches 6.60 degrees off the seam, so the inner window stops 1.6
/// degrees short of it while the outer one still starts clear of it. That
/// biases one way only: a share read at two widths **understates** the wider
/// one's cost, because the part of its corridor past 5 degrees is scored in
/// neither window. Read a fall between two widths as a floor under the effect
/// rather than as its size.
fn share(options: &Options, reframe: &Reframe, plain: &Picture, banded: &Picture) -> Fallible<()> {
    let size = options.size();
    let width = size.width as usize;
    // How far past the seam each output pixel is, in degrees: the angle off
    // the plane the two lenses hand over across, which is the body's own
    // xy plane.
    let past: Vec<f64> = (0..(size.width * size.height) as usize)
        .map(|index| {
            let uv = [
                (index % width) as f32 / size.width as f32,
                (index / width) as f32 / size.height as f32,
            ];
            let Some(ray) = reframe.view_ray(uv) else {
                return f64::INFINITY;
            };
            let body = reframe.body_ray(ray);
            let length = (body[0] * body[0] + body[1] * body[1] + body[2] * body[2]).sqrt();
            match length > 0.0 {
                true => f64::from((body[2] / length).asin().to_degrees().abs()),
                false => f64::INFINITY,
            }
        })
        .collect();
    println!(
        "\n{:<14} {:>13} {:>13} {:>9} {:>9} {:>9}",
        "picture", "in the band", "either side", "share", "band px", "side px"
    );
    for (name, picture) in [("ours, stage 1", plain), ("ours, band", banded)] {
        let luma = picture.luma();
        let inside = gradient(&luma, &past, width, (0.0, 5.0));
        let outside = gradient(&luma, &past, width, (9.0, 25.0));
        println!(
            "{name:<14} {:>13.1} {:>13.1} {:>9.3} {:>9} {:>9}",
            inside.0,
            outside.0,
            inside.0 / outside.0,
            inside.1,
            outside.1,
        );
    }
    Ok(())
}

/// Mean squared horizontal luma gradient over the pixels whose distance past
/// the seam falls inside `band`, and how many pixels that was.
///
/// The count is printed beside every ratio because a band that lands on
/// nothing reads 0.000, which looks like a picture with no sharpness rather
/// than a mask with no pixels.
fn gradient(luma: &[f32], past: &[f64], width: usize, band: (f64, f64)) -> (f64, usize) {
    let mut total = 0.0;
    let mut count = 0;
    for index in 1..luma.len() - 1 {
        if index % width == 0 || index % width == width - 1 {
            continue;
        }
        if !(band.0..=band.1).contains(&past[index]) {
            continue;
        }
        // A pixel no lens reached is not a pixel, and its edge against the
        // room around the ball is not an edge in the picture.
        if [index - 1, index, index + 1]
            .iter()
            .any(|at| luma[*at] <= 0.0)
        {
            continue;
        }
        let step = f64::from(luma[index + 1] - luma[index - 1]);
        total += step * step;
        count += 1;
    }
    match count > 0 {
        true => (total / count as f64, count),
        false => (0.0, 0),
    }
}

// ------------------------------------------------------------ options

#[derive(Clone)]
struct Options {
    input: PathBuf,
    mode: Mode,
    from: f64,
    count: usize,
    yaw: f64,
    pitch: f64,
    fov: f64,
    size: u32,
    lock: bool,
    control: bool,
    /// Draw with the band switched off, which is stage 1's own picture: the
    /// pass is still built and still dispatched, and the state it fills is
    /// simply not applied.
    off: bool,
    out: Option<PathBuf>,
    /// Where to write the settled state, for `--bin seam band=`.
    save: Option<PathBuf>,
    /// The screen region `mode=trace` reports on: x, y, width, height, in
    /// pixels of the rendered view.
    region: [u32; 4],
    /// The azimuths `mode=coverage` calls the owner's arc, in degrees.
    ///
    /// 93 to 125 is where his three banked downward views sit, and it is what
    /// `--bin refusals` reported 1 of 11 cells read on.
    arc: (f64, f64),
    /// The known defect `mode=snap` puts into the state it read back, so the
    /// attribution can be shown to catch what it claims to catch.
    plant: Plant,
    /// Which calibration the band is read through.
    ///
    /// It is an argument because it has to be: the band's readings and a step
    /// measured on the picture are only comparable when both are taken through
    /// the same calibration, and until stage 6 this instrument always fitted
    /// the file while `--bin seam mode=residual` always took the factory
    /// numbers, so the two were read side by side across two different
    /// calibration paths (docs/research/seam-two-axis.md).
    seam: Seam,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            mode: Mode::Field,
            from: 0.0,
            count: 30,
            yaw: 90.0,
            pitch: 0.0,
            fov: 60.0,
            size: 1024,
            lock: true,
            control: false,
            off: false,
            out: None,
            save: None,
            region: [0, 0, 0, 0],
            arc: (93.0, 125.0),
            seam: Seam::File,
            plant: Plant::None,
        };
        let mut seam = String::from("file");
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("arc", value)) => {
                    let (low, high) = value.split_once(':').ok_or("arc=<low deg>:<high deg>")?;
                    options.arc = (low.parse()?, high.parse()?);
                }
                Some(("mode", value)) => {
                    options.mode = match value {
                        "field" => Mode::Field,
                        "trace" => Mode::Trace,
                        "sequence" => Mode::Sequence,
                        "render" => Mode::Render,
                        "cost" => Mode::Cost,
                        "coverage" => Mode::Coverage,
                        "snap" => Mode::Snap,
                        _ => return Err(format!("no mode called {value}").into()),
                    }
                }
                Some(("from", value)) => options.from = value.parse()?,
                Some(("count", value)) => options.count = value.parse()?,
                Some(("yaw", value)) => options.yaw = value.parse()?,
                Some(("pitch", value)) => options.pitch = value.parse()?,
                Some(("fov", value)) => options.fov = value.parse()?,
                Some(("size", value)) => options.size = value.parse()?,
                Some(("lock", value)) => options.lock = value.parse::<u32>()? != 0,
                Some(("control", value)) => options.control = value.parse::<u32>()? != 0,
                Some(("off", value)) => options.off = value.parse::<u32>()? != 0,
                Some(("out", value)) => options.out = Some(PathBuf::from(value)),
                Some(("save", value)) => options.save = Some(PathBuf::from(value)),
                Some(("seam", value)) => seam = value.to_string(),
                Some(("plant", value)) => options.plant = Plant::parse(value)?,
                Some(("box", value)) => {
                    let mut numbers = value.split(',').map(str::parse::<u32>);
                    let mut next = || numbers.next().transpose();
                    options.region = [next()?, next()?, next()?, next()?]
                        .map(|number| number.ok_or("box wants x,y,w,h"))
                        .into_iter()
                        .collect::<Result<Vec<u32>, _>>()?
                        .try_into()
                        .map_err(|_| "box wants x,y,w,h")?;
                }
                Some((key, _)) => return Err(format!("no argument called {key}").into()),
            }
        }
        if options.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        // Deferred out of the loop because `seam=pool` is resolved against the
        // file and the file may be named anywhere on the line, but resolved
        // before the rest of the checks so a bad `seam=` is still the first
        // thing a bad line is told about.
        options.seam = Seam::parse(&seam, &options.input)?;
        Ok(options)
    }

    fn at(&self) -> Cue {
        Cue::Time(Duration::from_secs_f64(self.from.max(0.0)))
    }

    fn size(&self) -> Size {
        Size::new(self.size, self.size)
    }

    fn camera(&self) -> Camera {
        Camera {
            yaw: (self.yaw.to_radians()) as f32,
            pitch: (self.pitch.to_radians()) as f32,
            fov: (self.fov.to_radians()) as f32,
        }
    }

    fn out(&self) -> PathBuf {
        self.out.clone().unwrap_or_else(|| PathBuf::from("scratch"))
    }

    fn stem(&self) -> String {
        self.input
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    }
}

const USAGE: &str = "usage: band <file.insv> [mode=field|trace|sequence|render|cost|coverage|snap] [arc=low:high] [from=seconds] \
     [count=frames] [yaw=deg] [pitch=deg] [fov=deg] [size=px] [lock=0] [control=1] [off=1] \
     [out=dir] [save=state.txt] [box=x,y,w,h] \
     [plant=none|cell:<cell>:<deg>|commit:<frame>:<deg>|arrive:<cell>:<frame>|hold:<frame>|still] \
     [seam=factory|file|pool|roll:0.8,yaw:-2.3,pitch:-0.9,cx:-3.3,cy:-11.9]";
