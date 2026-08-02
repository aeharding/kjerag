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
//!   from=9.0 count=90 yaw=90 pitch=-60 out=scratch/stage2-proof
//! # before and after at one view, and what moved between them
//! cargo run --release -p kjerag-spike --bin band -- <file.insv> mode=render \
//!   from=9.0 count=60 yaw=90 fov=60 out=scratch/stage2-proof
//! ```
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

use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_render::Cue;
use kjerag_render::{
    AZIMUTHS, Camera, Cell, Horizon, Reframe, Sampling, Scene, ScenePipeline, Size,
};
use kjerag_spike::{FORMAT, Gpu, Picture, Render, seam_fit};

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
    cell.disparity * (cell.confidence / kjerag_render::KEEP).clamp(0.0, 1.0)
}

/// What the band settled on, direction by direction, at the end of the run.
fn table(last: &Read) {
    println!(
        "\nwhat the band settled on. `view px` is the disagreement a 1920-wide 90 degree view \n\
         would show, at {VIEW_PX_PER_DEG} px per degree; `metres` is the distance the disparity \n\
         stands for; `band` is how wide the crossover opened to carry the reading; `cut` is what \n\
         the fixed 2 degree band of stage 2 would have thrown away, in view px, which is the \n\
         width of the doubled edge it left; `off epi` is the axis a distance CANNOT displace \n\
         content along, which is measured and never applied.\n"
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
        let floor = last.mapped.crossover_at(0.0, 0.0, 0.0);
        let cut = applied - kjerag_render::band::carried(applied, floor);
        println!(
            "{:>6.0} {:>9.3}d {:>10.2} {:>10} {:>9.3}d {:>10.2} {:>11.3} {:>9.3}d",
            index as f64 / AZIMUTHS as f64 * 360.0,
            degrees,
            degrees * VIEW_PX_PER_DEG,
            cell.metres()
                .map_or_else(|| "-".to_owned(), |m| format!("{m:.1}")),
            f64::from(last.mapped.crossover_at(applied, 0.0, 0.0).to_degrees()),
            f64::from(cut.to_degrees()) * VIEW_PX_PER_DEG,
            cell.confidence,
            f64::from(cell.off_epi.to_degrees()),
        );
    }
}

/// How far the crossover opened, and what the fixed band was throwing away
/// (issue #103, stage 4).
///
/// The two columns are the same measurement read two ways: the width solves
/// `|disparity| <= FOLD * width` for the width, and the clamp solves it for
/// the disparity. Everything `cut` reports is alignment the pass had measured,
/// believed, and then declined to apply because the band could not carry it -
/// a doubled edge that much wide, on content that near.
fn crossover(reads: &[Read]) {
    let last = reads.last().expect("play returns at least one frame");
    let floor = last.mapped.crossover_at(0.0, 0.0, 0.0);
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
        .filter(|(_, _, applied)| last.mapped.crossover_at(*applied, 0.0, 0.0) > floor)
        .count();
    let frames = reads.len();
    let widest = seen
        .iter()
        .map(|(_, _, applied)| last.mapped.crossover_at(*applied, 0.0, 0.0))
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
         band any of them asked for is {:.3} deg. what stage 2's fixed band cut from those: \n\
         {:.3} deg at worst, which is {:.1} view px of doubled edge on content at {}. this stage \n\
         cuts nothing the search can report, so that is what it recovers.",
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
        .filter(|(_, _, applied)| last.mapped.crossover_at(*applied, 0.0, 0.0) > floor)
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
                f64::from(last.mapped.crossover_at(*applied, 0.0, 0.0).to_degrees()),
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
    // sit inside the ring both lenses have a picture of.
    let ceiling = kjerag_render::band::WIDEST_DEG;
    let reach = 0.5 * ceiling + 0.9 * ceiling;
    println!(
        "           these two lenses overlap by {:.2} deg, {:.2} a side. the widest the band may \n\
         open is {ceiling:.2} deg, and that band plus the whole bend it carries reaches {reach:.2} \n\
         deg off the seam, so the handover stays inside the overlap with {:.2} deg to spare.",
        f64::from(overlap.to_degrees()),
        f64::from(overlap.to_degrees()) * 0.5,
        f64::from(overlap.to_degrees()) * 0.5 - f64::from(reach),
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
            last.mapped
                .crossover_at(held(last, *direction) as f32, 0.0, 0.0)
                > last.mapped.crossover_at(0.0, 0.0, 0.0)
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
        let opened = read
            .mapped
            .crossover_at(held(read, direction) as f32, 0.0, 0.0);
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
            "  {:>6} {:>10} {:>10} {:>9} {:>7} {:>7} {:>11}",
            "frame", "held", "applied", "view px", "conf", "tau s", "read"
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
            let applied = f64::from(held.disparity)
                * f64::from((held.confidence / kjerag_render::KEEP).clamp(0.0, 1.0));
            println!(
                "  {frame:>6} {:>9.4}d {:>9.4}d {:>9.2} {:>7.3} {:>7.2} {:>10.4}d",
                f64::from(held.disparity).to_degrees(),
                applied.to_degrees(),
                applied.to_degrees() * px_per_deg,
                held.confidence,
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
fn covering(reframe: &Reframe, size: Size, region: [u32; 4]) -> Vec<usize> {
    let [x, y, width, height] = region;
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
            if past > 1.5 {
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

/// Which of the app's three seam paths the band is read through, and the same
/// three words `--bin step` uses.
///
/// It is an argument because it has to be: the band's readings and a step
/// measured on the picture are only comparable when both are taken through the
/// same calibration, and until stage 6 this instrument always fitted the file
/// while `--bin seam mode=residual` always took the factory numbers, so the two
/// were read side by side across two different calibration paths
/// (docs/research/seam-two-axis.md).
#[derive(Clone)]
enum Seam {
    Factory,
    File,
    Stored(kjerag_render::SeamFit),
}

impl Seam {
    fn hold(&self, scene: &Scene) {
        match self {
            Self::Factory => println!("seam:   factory calibration, no correction"),
            Self::File => scene.fit_seam(true),
            Self::Stored(fit) => scene.use_seam(*fit),
        }
    }
}

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
    /// Which calibration the band is read through.
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
            seam: Seam::File,
        };
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("mode", value)) => {
                    options.mode = match value {
                        "field" => Mode::Field,
                        "trace" => Mode::Trace,
                        "sequence" => Mode::Sequence,
                        "render" => Mode::Render,
                        "cost" => Mode::Cost,
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
                Some(("seam", value)) => {
                    options.seam = match value {
                        "factory" => Seam::Factory,
                        "file" => Seam::File,
                        _ => Seam::Stored(seam_fit(value)?),
                    }
                }
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

const USAGE: &str = "usage: band <file.insv> [mode=field|sequence|render] [from=seconds] \
     [count=frames] [yaw=deg] [pitch=deg] [fov=deg] [size=px] [lock=0] [control=1] [off=1] \
     [out=dir] [save=state.txt] [box=x,y,w,h] \
     [seam=factory|file|roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9]";
