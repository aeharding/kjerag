//! What the shipped band pass reads, how steady it is, and what it does to the
//! picture (issue #103, stage 2).
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
//! call it the flicker of the picture. `control=1` is the positive control for
//! it, and it is the one thing a flicker column may not be believed without: a
//! known step is put into the state each frame, alternating sign, and a step of
//! `s` has to read back at `2s`.
//!
//! PNGs land in gitignored `scratch/`: these are frames of somebody's real
//! flights and this repo is public.

use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_render::Cue;
use kjerag_render::{AZIMUTHS, Camera, Cell, Horizon, Sampling, Scene, ScenePipeline, Size};
use kjerag_spike::{FORMAT, Gpu, Picture, Render};

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
        Mode::Sequence => sequence(&options),
        Mode::Render => render(&options),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// What the band reads over a stretch, and whether it is depth.
    Field,
    /// A stretch drawn frame by frame, with the flicker of what was applied.
    Sequence,
    /// One view before and after, and the difference at 8x.
    Render,
}

// ------------------------------------------------------------ the run

/// One frame of a run: the state the pass drew with, and when.
struct Read {
    at: Duration,
    cells: Vec<Cell>,
    picture: Picture,
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
    scene.fit_seam(true);
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
        reads.push(Read {
            at,
            cells: pipeline.band_state(&gpu.device, &gpu.queue)?,
            picture,
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

    table(last);
    coverage(&reads);
    geometry(last);
    flicker(&reads, options);
    Ok(())
}

/// What the band settled on, direction by direction, at the end of the run.
fn table(last: &Read) {
    println!(
        "\nwhat the band settled on. `view px` is the disagreement a 1920-wide 90 degree view \n\
         would show, at {VIEW_PX_PER_DEG} px per degree; `metres` is the distance the disparity \n\
         stands for; `off epi` is the axis a distance CANNOT displace content along, which is \n\
         measured and never applied.\n"
    );
    println!("   phi  disparity    view px     metres  confidence    off epi");
    for (index, cell) in last.cells.iter().enumerate() {
        if cell.confidence <= 0.0 {
            continue;
        }
        let degrees = f64::from(cell.disparity.to_degrees());
        println!(
            "{:>6.0} {:>9.3}d {:>10.2} {:>10} {:>11.3} {:>9.3}d",
            index as f64 / AZIMUTHS as f64 * 360.0,
            degrees,
            degrees * VIEW_PX_PER_DEG,
            cell.metres()
                .map_or_else(|| "-".to_owned(), |m| format!("{m:.1}")),
            cell.confidence,
            f64::from(cell.off_epi.to_degrees()),
        );
    }
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
    if !options.control {
        println!("         (control=1 puts a known step in and reads it back.)");
        return;
    }
    println!(
        "\n         the positive control. a step of `s` alternating sign each frame has to come \n\
         back at 2s, added in quadrature to what the file already had. a flicker column is \n\
         a negative result and means nothing until it is shown able to read a positive one.\n\
         \n             step    expected        read"
    );
    for step in [0.05f64, 0.20] {
        let shaken = stepped(reads, step.to_radians());
        println!(
            "         {step:>8.2}d {:>11.4} {:>11.4}",
            measured.0.hypot(2.0 * step),
            shaken.0,
        );
    }
}

/// The rms and worst frame-to-frame step of the bend, at [`WATCHED`]
/// directions, with `shake` radians put into every other frame.
fn stepped(reads: &[Read], shake: f64) -> (f64, f64) {
    let at = |read: &Read, frame: usize, direction: usize| {
        // The same lookup the fragment shader does: between two cells,
        // linearly, wrapping. `kjerag_render::Reframe::bend` is the shipped
        // one; this is its arithmetic over a buffer already read back.
        let turn = direction as f64 / WATCHED as f64 * AZIMUTHS as f64;
        let low = turn.floor() as usize;
        let mix = turn - low as f64;
        let cell = |index: usize| f64::from(read.cells[index % AZIMUTHS].disparity);
        let held = cell(low) + (cell(low + 1) - cell(low)) * mix;
        held + match frame % 2 {
            0 => shake,
            _ => -shake,
        }
    };
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

    let draw = |off: bool| -> Fallible<Picture> {
        let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
        let mut options = options.clone();
        options.off = off;
        let mut reads = play(&gpu, &options, &mut pipeline, |_, _| Ok(()))?;
        Ok(reads
            .pop()
            .expect("play returns at least one frame")
            .picture)
    };
    let plain = draw(true)?;
    let banded = draw(false)?;
    plain.save(&gpu, &out.join(format!("{stem}-1-stage1.png")))?;
    banded.save(&gpu, &out.join(format!("{stem}-2-band.png")))?;
    banded
        .amplified(&plain)
        .save(&gpu, &out.join(format!("{stem}-4-what-moved.png")))?;
    let against = plain.against(&banded);
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
        };
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("mode", value)) => {
                    options.mode = match value {
                        "field" => Mode::Field,
                        "sequence" => Mode::Sequence,
                        "render" => Mode::Render,
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
     [out=dir]";
