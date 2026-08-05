//! What the seam band displaces the picture by, and how much of that changes
//! from one frame to the next (issue #103, the motion half).
//!
//! ```sh
//! # four bands across the seam: what was applied at each, frame by frame
//! cargo run --release -p kjerag-spike --bin shear -- <file.insv> \
//!   time=36.303 yaw=3.78 pitch=5.44 fov=20.00 lock=1 frames=90
//! # the same view with a thin patch walked across the seam: the field's shape
//! cargo run --release -p kjerag-spike --bin shear -- <file.insv> \
//!   time=36.303 yaw=3.78 pitch=5.44 fov=20.00 lock=1 frames=90 mode=profile
//! # the null, which has to read exactly zero everywhere
//! cargo run --release -p kjerag-spike --bin shear -- <file.insv> \
//!   time=36.303 yaw=3.78 pitch=5.44 fov=20.00 lock=1 frames=90 null=1
//! ```
//!
//! **Two arms of one frame, and not two runs of one file.** Every frame is
//! decoded once and drawn twice, through two [`ScenePipeline`]s: the delivered
//! one, and a second held from its first frame by [`ScenePipeline::hold_band`],
//! which leaves the band at the zero that bends nothing. The two pictures hold
//! the same content by construction, so what separates them is the applied
//! field and nothing else, and there is no motion estimate anywhere in here to
//! be wrong about that. `null=1` holds both arms, and then the two pictures are
//! the same picture and every reading is exactly zero: the instrument's own
//! floor, measured rather than assumed.
//!
//! **Seam-relative and not picture-relative.** Under a locked horizon the body
//! turns beneath the view, so the seam walks across the picture: 350 px over
//! the three seconds this was written for. A row pinned to the picture would be
//! measuring that sweep. Every patch here is placed against the seam's own row,
//! read out of the shipped map (`Reframe`) by walking down the gradient of the
//! angle off the seam plane, which is the same walk `--bin corridor` reports.
//!
//! **The band is a filter, so a measurement starts warm.** Its state carries
//! frame to frame, so a run that begins at the frame it wants to measure is
//! measuring the state converging. `warm=` seconds are drawn before the first
//! measured frame and thrown away; the measured window begins at the first
//! frame whose own timestamp reaches `time=`, so the window is the same window
//! however the warm-up seek rounded.
//!
//! **Whether it repeats is measured and not designed.** Six runs of the
//! reference command below, across three builds, wrote identical readings on
//! one box, the live arm included. That is a reading rather than a guarantee:
//! the band's state is an IIR filled by a GPU pass, and the campaign this came
//! out of saw two live renders of one view differ in the third decimal of a
//! pixel. Run a comparison twice before believing a difference that small.
//!
//! The reference reading, on the shimmer view
//! (docs/research/reference-views.md), on one AMD Radeon 760M:
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin shear -- \
//!   ~/Videos/Insta/VID_20260714_193252_00_006.insv \
//!   time=36.303 yaw=3.78 pitch=5.44 fov=20.00 lock=1 frames=90 warm=6.0 \
//!   seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91
//! ```
//!
//! Lens 1's interior (`-150`) reads 0.3641 deg applied at 0.0048 deg step rms;
//! the seam itself (`+0`) 0.3584 at 0.0605 with a worst single step of 0.41;
//! the far side of the handover (`+60`) 0.0417 at 0.1132; and lens 0's picture
//! (`+150`), which the band never bends, 0.0003 at 0.0003, which is this
//! instrument's floor on a live arm. The `seam=` in that command is not
//! decoration: fitted from the file instead, the same view reads 0.025 deg at
//! `-150` rather than 0.364, because what the band applies is what the
//! calibration left it.
//!
//! CSVs land in gitignored `scratch/`, stamped with the file they were read off
//! and the whole command line that read them: a table of numbers with no source
//! on it is a table nobody can attribute later.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_render::{
    AZIMUTHS, Along, Camera, Cell, Cue, Framing, Horizon, Reframe, Sampling, Scene, ScenePipeline,
    SeamFit, Size,
};
use kjerag_spike::{FORMAT, Gpu, Picture, Render, seam_fit};

/// How far the match searches across the seam, in rows. The applied field is
/// small and the search only has to hold it; a wider one costs time and finds
/// the same peak.
const REACH_ROWS: usize = 12;

/// The same along the seam, in columns. Four times the rows because that is the
/// band's big axis: the along-seam correction reaches lens 1's whole picture.
const REACH_COLS: usize = 48;

/// How well a patch has to correlate for its reading to be counted. Below this
/// the two arms are not looking at the same content and the peak is a shape in
/// the noise rather than a displacement.
const KEEP_PEAK: f64 = 0.8;

/// How many frames a band needs before its statistics are printed instead of
/// its frame count. A step rms over a handful of frames is not a step rms.
const MIN_FRAMES: usize = 20;

/// What counts as a step, in degrees: the size at which a single frame's change
/// is a jump in the picture rather than the field being carried across it.
const STEP_DEG: f64 = 0.1;

/// How far the seam may lean off the rows before a row offset stops meaning a
/// distance across it, in degrees. The instrument refuses rather than quoting
/// an across-seam profile that is partly along one.
const TILT_LIMIT: f64 = 30.0;

/// How many directions the band's own state is watched at.
///
/// Deliberately not [`AZIMUTHS`], and for `--bin band`'s reason: the bend is
/// applied everywhere and read at [`AZIMUTHS`] places, so watching the cells
/// alone would report the readings' steadiness and call it the field's.
const WATCHED: usize = 360;

/// How many Newton steps the walk from the picture's centre onto the seam
/// takes. The angle off the seam plane is smooth and very nearly linear over a
/// picture this wide, so three is convergence.
const STEPS: usize = 3;

/// How close to `time=` a frame counts as the frame it names.
///
/// A view line quotes the time to the millisecond (`framing`), so the frame it
/// was written off can sit up to half a millisecond before the number in it,
/// and a plain `>=` would start the window one frame late. One frame of 29.97
/// fps content is 33.4 ms, so nothing else is inside this.
const NAMED: Duration = Duration::from_millis(1);

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args())?;
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let taken = walk(&gpu, &options)?;
    report(&options, &taken)
}

// ------------------------------------------------------------ the two arms

/// One measured frame: where the seam ran, what the band displaced at every
/// band, and the state the delivered arm drew with.
struct Sample {
    at: Duration,
    seam: Line,
    /// One reading per offset. `None` where the patch did not fit inside the
    /// picture, which is what the far offsets do once the seam has swept.
    fits: Vec<Option<Fit>>,
    cells: Vec<Cell>,
    along: Along,
    /// Whether the two arms' pictures came back byte for byte the same.
    same: bool,
}

/// One patch of one frame, matched between the arms.
#[derive(Clone, Copy)]
struct Fit {
    /// The row displacement, in pixels: across the seam, where the seam lies
    /// along the rows. Positive is towards the bottom of the picture.
    across: f64,
    /// The column displacement, in pixels: along the seam, which is the band's
    /// big axis.
    along: f64,
    peak: f64,
    /// Whether the peak sat against the edge of the search, which means the
    /// displacement is at least this and possibly more.
    pinned: bool,
}

/// Draws the run both ways and reads every band on every frame.
///
/// One [`Scene`], so one decode: the frame is decoded once and handed to both
/// pipelines. The correction walk is landed by `fit_seam` on a stepped scene,
/// so the second `primitive` of a frame builds the same map as the first.
fn walk(gpu: &Gpu, options: &Options) -> Fallible<Vec<Sample>> {
    let mut live = ScenePipeline::new(&gpu.device, FORMAT);
    let mut plain = ScenePipeline::new(&gpu.device, FORMAT);
    plain.hold_band(true);
    live.hold_band(options.null);
    let mut scene = Scene::still(&options.input, options.start())?;
    scene.set_horizon(options.view.horizon);
    options.seam.hold(&scene);

    let size = options.size();
    let offsets = options.mode.offsets();
    let patch = options.mode.patch();
    let mut taken: Vec<Sample> = Vec::with_capacity(options.frames);
    while let Some((_, at)) = scene.frame() {
        // Drawn on every frame, warm-up included: the band's state is filled by
        // the pass, so a frame not drawn is a frame not measured.
        let banded = draw(gpu, &scene, &mut live, options)?;
        if at + NAMED >= options.view.at {
            let held = draw(gpu, &scene, &mut plain, options)?;
            let mapped = scene
                .mapped(options.view.camera, 1.0)
                .ok_or("no frame to map")?;
            let seam = line(&mapped, size).ok_or("the seam does not cross this view")?;
            let (along, cells) = live.band_state(&gpu.device, &gpu.queue)?;
            taken.push(Sample {
                at,
                seam,
                fits: read(&held, &banded, seam.y, &offsets, patch),
                cells,
                along,
                same: held.against(&banded).is_identical(),
            });
        }
        if taken.len() >= options.frames || !scene.advance()? {
            break;
        }
    }
    if taken.is_empty() {
        return Err("no frame decoded in that window".into());
    }
    let leaning = taken
        .iter()
        .filter(|s| s.seam.tilt_deg > TILT_LIMIT)
        .count();
    if leaning > 0 {
        return Err(format!(
            "the seam leans more than {TILT_LIMIT:.0} degrees off the rows on {leaning} of \
             {} frames, so an offset in rows is not a distance across it. this view is not \
             one this instrument can read.",
            taken.len(),
        )
        .into());
    }
    Ok(taken)
}

fn draw(
    gpu: &Gpu,
    scene: &Scene,
    pipeline: &mut ScenePipeline,
    options: &Options,
) -> Fallible<Picture> {
    Render {
        gpu,
        scene,
        pipeline,
    }
    .frame(options.view.camera, Sampling::default(), options.size())
}

/// Every band of one frame: the same rectangle out of both arms, matched.
fn read(
    held: &Picture,
    banded: &Picture,
    seam: f64,
    offsets: &[i32],
    patch: (usize, usize),
) -> Vec<Option<Fit>> {
    let (rows, cols) = patch;
    let width = held.size.width as usize;
    let height = held.size.height as usize;
    // Centred on the picture's own column, which is where the seam is walked
    // onto: a patch off to one side would be reading a different part of a
    // field that has a gradient along the seam as well as across it.
    let left = (width - cols) / 2;
    let (a, b) = (held.luma(), banded.luma());
    offsets
        .iter()
        .map(|offset| {
            let top = (seam + f64::from(*offset)).round() - rows as f64 / 2.0;
            if top < 0.0 || top as usize + rows > height {
                return None;
            }
            let top = top as usize;
            let cut = |luma: &[f32]| crop(luma, width, left, top, rows, cols);
            Some(matched(&cut(&a), &cut(&b), rows, cols))
        })
        .collect()
}

fn crop(
    luma: &[f32],
    stride: usize,
    left: usize,
    top: usize,
    rows: usize,
    cols: usize,
) -> Vec<f64> {
    (0..rows)
        .flat_map(|row| {
            let start = (top + row) * stride + left;
            luma[start..start + cols].iter().map(|v| f64::from(*v))
        })
        .collect()
}

/// What puts `b` onto `a`, by zero-mean normalized cross correlation over
/// [`REACH_ROWS`] rows and [`REACH_COLS`] columns, with a parabola through the
/// peak for the subpixel part.
///
/// Normalized rather than plain correlation because the band moves the exposure
/// as well as the geometry (issue #103, stage 3), and a match that could be won
/// by a gain is not a match.
fn matched(a: &[f64], b: &[f64], rows: usize, cols: usize) -> Fit {
    // Identical patches have no displacement to fit, and the parabola through a
    // peak of exactly one would report the texture's own asymmetry as one. The
    // null arm is the reading that has to come back exactly zero.
    if a == b {
        return Fit {
            across: 0.0,
            along: 0.0,
            peak: 1.0,
            pinned: false,
        };
    }
    let (inner_rows, inner_cols) = (rows - 2 * REACH_ROWS, cols - 2 * REACH_COLS);
    let inner: Vec<f64> = (0..inner_rows)
        .flat_map(|row| {
            let start = (REACH_ROWS + row) * cols + REACH_COLS;
            a[start..start + inner_cols].iter().copied()
        })
        .collect();
    let mean = inner.iter().sum::<f64>() / inner.len() as f64;
    let inner: Vec<f64> = inner.iter().map(|v| v - mean).collect();
    let norm = inner.iter().map(|v| v * v).sum::<f64>().sqrt();
    if norm == 0.0 {
        return Fit {
            across: 0.0,
            along: 0.0,
            peak: 0.0,
            pinned: false,
        };
    }
    let sums = Sums::of(b, rows, cols);
    let mut scores = vec![-1.0f64; (2 * REACH_ROWS + 1) * (2 * REACH_COLS + 1)];
    for row in 0..=2 * REACH_ROWS {
        for col in 0..=2 * REACH_COLS {
            let (total, square) = sums.over(row, col, inner_rows, inner_cols);
            let count = (inner_rows * inner_cols) as f64;
            let spread = (square - total * total / count).max(0.0).sqrt();
            if spread <= 0.0 {
                continue;
            }
            let mut dot = 0.0;
            for line in 0..inner_rows {
                let from = (row + line) * cols + col;
                for column in 0..inner_cols {
                    dot += inner[line * inner_cols + column] * b[from + column];
                }
            }
            scores[row * (2 * REACH_COLS + 1) + col] = dot / (norm * spread);
        }
    }
    peak(&scores)
}

/// The peak of a score surface, in displacement, with the subpixel part from a
/// parabola through its two neighbours on each axis.
fn peak(scores: &[f64]) -> Fit {
    let stride = 2 * REACH_COLS + 1;
    let best = scores
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or(0, |(index, _)| index);
    let (row, col) = (best / stride, best % stride);
    let height = scores.len() / stride;
    let pinned = row == 0 || col == 0 || row + 1 == height || col + 1 == stride;
    let at = |row: usize, col: usize| scores[row * stride + col];
    let (across, along) = match pinned {
        // A peak against the wall has no neighbour on one side, and it is a
        // displacement the search could not hold: reported at the wall, and
        // counted, so a table of them says it was pinned rather than measured.
        true => (0.0, 0.0),
        false => (
            parabola(at(row - 1, col), at(row, col), at(row + 1, col)),
            parabola(at(row, col - 1), at(row, col), at(row, col + 1)),
        ),
    };
    Fit {
        across: (row as f64 - REACH_ROWS as f64) + across,
        along: (col as f64 - REACH_COLS as f64) + along,
        peak: at(row, col),
        pinned,
    }
}

fn parabola(low: f64, here: f64, high: f64) -> f64 {
    let bottom = low - 2.0 * here + high;
    match bottom == 0.0 {
        true => 0.0,
        false => -0.5 * (high - low) / bottom,
    }
}

/// Running sums of a patch and of its squares, so the denominator of every
/// window in the search costs four reads instead of a pass over the pixels.
struct Sums {
    total: Vec<f64>,
    square: Vec<f64>,
    stride: usize,
}

impl Sums {
    fn of(from: &[f64], rows: usize, cols: usize) -> Self {
        let stride = cols + 1;
        let mut total = vec![0.0; (rows + 1) * stride];
        let mut square = vec![0.0; (rows + 1) * stride];
        for row in 0..rows {
            for col in 0..cols {
                let value = from[row * cols + col];
                let at = (row + 1) * stride + col + 1;
                total[at] = value + total[at - 1] + total[at - stride] - total[at - stride - 1];
                square[at] =
                    value * value + square[at - 1] + square[at - stride] - square[at - stride - 1];
            }
        }
        Self {
            total,
            square,
            stride,
        }
    }

    fn over(&self, top: usize, left: usize, rows: usize, cols: usize) -> (f64, f64) {
        let corners = |of: &[f64]| {
            of[(top + rows) * self.stride + left + cols] + of[top * self.stride + left]
                - of[top * self.stride + left + cols]
                - of[(top + rows) * self.stride + left]
        };
        (corners(&self.total), corners(&self.square))
    }
}

// ------------------------------------------------------------ the seam

/// Where the seam runs at one frame, in the picture's own pixels.
#[derive(Clone, Copy)]
struct Line {
    x: f64,
    y: f64,
    /// The seam's direction in the picture, in degrees off the rows.
    tilt_deg: f64,
}

/// The signed angle off the seam plane one pixel is looking at, in degrees.
/// Zero on the seam, and its sign says which lens the ray belongs to.
fn past(mapped: &Reframe, size: Size, at: [f64; 2]) -> Option<f64> {
    let uv = [
        (at[0] / f64::from(size.width)) as f32,
        (at[1] / f64::from(size.height)) as f32,
    ];
    let ray = mapped.view_ray(uv)?;
    let body = mapped.body_ray(ray);
    let length = (body[0] * body[0] + body[1] * body[1] + body[2] * body[2]).sqrt();
    match length > 0.0 {
        true => Some(f64::from((body[2] / length).asin().to_degrees())),
        false => None,
    }
}

/// The seam point nearest the picture's centre, walked onto rather than scanned
/// for.
///
/// A scan along a row finds nothing when the seam lies along the rows, which is
/// the case this instrument was written for. The gradient does not care which
/// way the seam runs: the angle off the seam plane is smooth, so a step down
/// its own gradient lands on the contour whatever its direction.
fn line(mapped: &Reframe, size: Size) -> Option<Line> {
    let mut at = [f64::from(size.width) / 2.0, f64::from(size.height) / 2.0];
    let mut gradient = [0.0, 0.0];
    for _ in 0..STEPS {
        let here = past(mapped, size, at)?;
        gradient = [
            (past(mapped, size, [at[0] + 1.0, at[1]])? - past(mapped, size, [at[0] - 1.0, at[1]])?)
                / 2.0,
            (past(mapped, size, [at[0], at[1] + 1.0])? - past(mapped, size, [at[0], at[1] - 1.0])?)
                / 2.0,
        ];
        let steepness = gradient[0].hypot(gradient[1]);
        if steepness <= 0.0 {
            return None;
        }
        at = [
            at[0] - here * gradient[0] / (steepness * steepness),
            at[1] - here * gradient[1] / (steepness * steepness),
        ];
    }
    Some(Line {
        x: at[0],
        y: at[1],
        // The seam runs across its own gradient, so a gradient straight down
        // the columns is a seam lying along the rows.
        tilt_deg: gradient[0].atan2(gradient[1]).to_degrees().abs(),
    })
}

// ------------------------------------------------------------ the statistics

/// What one band came to over the run, in the picture's pixels and in degrees
/// of view.
struct Band {
    offset: i32,
    frames: usize,
    /// The mean size of the applied displacement, `hypot(across, along)`.
    size_px: f64,
    along_px: f64,
    across_px: f64,
    /// How far the size spread over the run: how much of the field is standing
    /// still and how much is being carried through the patch.
    spread_px: f64,
    /// The frame-to-frame change of the displacement, rms and worst.
    step_px: f64,
    worst_px: f64,
    /// The rms second difference. A field that stands still in the world while
    /// the seam sweeps it across the picture moves smoothly, and a smooth ramp
    /// has no second difference; a single-frame step has all of its size here.
    bend_px: f64,
    steps: usize,
    peak: f64,
    pinned: usize,
}

impl Band {
    /// `None` where too few frames correlated to say anything.
    fn of(samples: &[Sample], column: usize, offset: i32, scale: f64) -> Option<Self> {
        let kept: Vec<Fit> = samples
            .iter()
            .filter_map(|sample| sample.fits[column])
            .filter(|fit| fit.peak > KEEP_PEAK)
            .collect();
        if kept.len() < MIN_FRAMES {
            return None;
        }
        let mean = |of: &[f64]| of.iter().sum::<f64>() / of.len() as f64;
        let rms = |of: &[f64]| mean(&of.iter().map(|v| v * v).collect::<Vec<f64>>()).sqrt();
        let each = |at: fn(&Fit) -> f64| kept.iter().map(at).collect::<Vec<f64>>();
        let sizes = each(|fit| fit.across.hypot(fit.along));
        let size_px = mean(&sizes);
        // Over the readings that correlated and not over the frames of the run:
        // a frame nothing was measured on has no step to it, and a gap crossed
        // as though it were one frame would be counted as a jump.
        let steps: Vec<f64> = kept
            .windows(2)
            .map(|pair| (pair[1].across - pair[0].across).hypot(pair[1].along - pair[0].along))
            .collect();
        let bends: Vec<f64> = kept
            .windows(3)
            .map(|three| {
                let second =
                    |at: fn(&Fit) -> f64| at(&three[2]) - 2.0 * at(&three[1]) + at(&three[0]);
                second(|fit| fit.across).hypot(second(|fit| fit.along))
            })
            .collect();
        Some(Self {
            offset,
            frames: kept.len(),
            size_px,
            along_px: mean(&each(|fit| fit.along)),
            across_px: mean(&each(|fit| fit.across)),
            spread_px: rms(&sizes.iter().map(|v| v - size_px).collect::<Vec<f64>>()),
            step_px: rms(&steps),
            worst_px: steps.iter().copied().fold(0.0, f64::max),
            bend_px: rms(&bends),
            steps: steps
                .iter()
                .filter(|step| **step / scale > STEP_DEG)
                .count(),
            peak: mean(&each(|fit| fit.peak)),
            pinned: kept.iter().filter(|fit| fit.pinned).count(),
        })
    }
}

/// How much of the band's own state changed between frames, at [`WATCHED`]
/// directions, in degrees.
///
/// The companion every displacement table is read beside: a field that is
/// applied and never updated shows here as exactly zero while the probes still
/// read whatever structure it has, which is what tells a static correction
/// apart from one moving under the picture.
fn stepped(samples: &[Sample], at: impl Fn(&Sample, usize) -> f64) -> (f64, f64) {
    let mut sum = 0.0;
    let mut count = 0.0;
    let mut worst: f64 = 0.0;
    for pair in samples.windows(2) {
        for direction in 0..WATCHED {
            let step = at(&pair[1], direction) - at(&pair[0], direction);
            sum += step * step;
            count += 1.0;
            worst = worst.max(step.abs());
        }
    }
    match count > 0.0 {
        true => ((sum / count).sqrt().to_degrees(), worst.to_degrees()),
        false => (0.0, 0.0),
    }
}

/// What the band bends by at one of the [`WATCHED`] directions, in radians: the
/// same lookup the fragment shader does, between two cells, linearly, wrapping.
fn bend(sample: &Sample, direction: usize) -> f64 {
    let turn = direction as f64 / WATCHED as f64 * AZIMUTHS as f64;
    let low = turn.floor() as usize;
    let mix = turn - low as f64;
    let cell = |index: usize| f64::from(sample.cells[index % AZIMUTHS].disparity);
    cell(low) + (cell(low + 1) - cell(low)) * mix
}

/// The along-seam field's own answer at one direction, in radians. Read off the
/// fitted field rather than off the readings behind it, because the field is
/// what a pixel there is bent by.
fn slide(sample: &Sample, direction: usize) -> f64 {
    let (sin, cos) = (direction as f32 / WATCHED as f32 * std::f32::consts::TAU).sin_cos();
    f64::from(sample.along.at(cos, sin))
}

// ------------------------------------------------------------ the report

fn report(options: &Options, samples: &[Sample]) -> Fallible<()> {
    let offsets = options.mode.offsets();
    let bands: Vec<Option<Band>> = offsets
        .iter()
        .enumerate()
        .map(|(column, offset)| Band::of(samples, column, *offset, options.scale()))
        .collect();
    heading(options, samples);
    match options.mode {
        Mode::Probe => {
            frames(samples, &offsets);
            table(options, &bands);
        }
        Mode::Profile => {
            table(options, &bands);
            handover(options, &bands);
        }
    }
    updates(samples);
    written(options, samples, &offsets, &bands)
}

fn heading(options: &Options, samples: &[Sample]) {
    let first = samples.first().expect("a run has frames");
    let last = samples.last().expect("a run has frames");
    let same = samples.iter().filter(|sample| sample.same).count();
    println!(
        "\nview:   {}\nband:   {}, {} frames from {:.3} s to {:.3} s, {} px square at {:.3} \
         px per degree",
        options.view.printed(&options.input),
        match options.null {
            true => "BOTH ARMS HELD OFF, which is the null: the two pictures are one picture",
            false => "the delivered arm against the same frames with the band held off",
        },
        samples.len(),
        first.at.as_secs_f64(),
        last.at.as_secs_f64(),
        options.size,
        options.scale(),
    );
    println!(
        "seam:   row {:.1} to {:.1}, leaning {:.1} to {:.1} degrees off the rows\n\
         arms:   {same} of {} frame pairs came back byte for byte the same",
        samples
            .iter()
            .map(|sample| sample.seam.y)
            .fold(f64::INFINITY, f64::min),
        samples
            .iter()
            .map(|sample| sample.seam.y)
            .fold(f64::NEG_INFINITY, f64::max),
        samples
            .iter()
            .map(|sample| sample.seam.tilt_deg)
            .fold(f64::INFINITY, f64::min),
        samples
            .iter()
            .map(|sample| sample.seam.tilt_deg)
            .fold(f64::NEG_INFINITY, f64::max),
        samples.len(),
    );
}

/// The per-frame displacements, which is what a step statistic is a summary of.
fn frames(samples: &[Sample], offsets: &[i32]) {
    println!(
        "\nwhat the band displaced, frame by frame, in pixels. `across` is the row \
         displacement,\nwhich is across the seam, and `along` the column one, which is the \
         band's big axis.\n"
    );
    let mut header = format!("{:>7}{:>10}{:>9}", "frame", "time", "seam y");
    for offset in offsets {
        let _ = write!(
            header,
            " |{:>10}{:>8}",
            format!("{offset:+} across"),
            "along"
        );
    }
    println!("{header}");
    for (index, sample) in samples.iter().enumerate() {
        let mut line = format!(
            "{index:>7}{:>10.3}{:>9.1}",
            sample.at.as_secs_f64(),
            sample.seam.y,
        );
        for fit in &sample.fits {
            match fit {
                Some(fit) if fit.peak > KEEP_PEAK => {
                    let _ = write!(line, " |{:>10.2}{:>8.2}", fit.across, fit.along);
                }
                Some(_) => {
                    let _ = write!(line, " |{:>18}", "not correlated");
                }
                None => {
                    let _ = write!(line, " |{:>18}", "off the picture");
                }
            }
        }
        println!("{line}");
    }
}

fn table(options: &Options, bands: &[Option<Band>]) {
    println!(
        "\nwhat each band came to over the run. `size` is the applied displacement, `step` \
         its\nframe-to-frame change, and `bend` the second difference, which a field carried \
         smoothly\nacross the picture has none of.\n"
    );
    println!(
        "{:>9}{:>11}{:>8}{:>10}{:>10}{:>10}{:>10}{:>11}{:>11}{:>11}{:>11}{:>11}{:>7}{:>7}",
        "offset px",
        "offset deg",
        "frames",
        "size px",
        "size deg",
        "along px",
        "along deg",
        "across px",
        "spread px",
        "step deg",
        "worst deg",
        "bend deg",
        "steps",
        "peak",
    );
    let scale = options.scale();
    for band in bands.iter().flatten() {
        println!(
            "{:>9}{:>11.2}{:>8}{:>10.3}{:>10.4}{:>10.3}{:>10.4}{:>11.3}{:>11.3}{:>11.4}\
             {:>11.4}{:>11.4}{:>7}{:>7.2}",
            band.offset,
            f64::from(band.offset) / scale,
            band.frames,
            band.size_px,
            band.size_px / scale,
            band.along_px,
            band.along_px / scale,
            band.across_px,
            band.spread_px,
            band.step_px / scale,
            band.worst_px / scale,
            band.bend_px / scale,
            band.steps,
            band.peak,
        );
    }
    refused(bands);
}

/// The bands that read nothing, and the readings that hit the wall. Both are
/// what a table of numbers alone would not say.
fn refused(bands: &[Option<Band>]) {
    let quiet = bands.iter().filter(|band| band.is_none()).count();
    if quiet > 0 {
        println!(
            "\n{quiet} of {} bands had fewer than {MIN_FRAMES} readings correlate and are not \
             printed.",
            bands.len(),
        );
    }
    let pinned: usize = bands.iter().flatten().map(|band| band.pinned).sum();
    if pinned > 0 {
        println!(
            "{pinned} readings sat against the edge of the search, so their displacement is at \
             least what is printed and possibly more."
        );
    }
}

/// Where the along-seam field hands over from one lens to the other.
///
/// Bracketed rather than resolved: the fits inside the handover are one
/// translation through a field that has a gradient there. What is solid is the
/// last offset still on the plateau and the first one already on the floor.
fn handover(options: &Options, bands: &[Option<Band>]) {
    let held: Vec<&Band> = bands.iter().flatten().collect();
    let quarter = held.len() / 4;
    if quarter == 0 {
        return;
    }
    let mean = |of: &[&Band]| of.iter().map(|band| band.along_px).sum::<f64>() / of.len() as f64;
    let plateau = mean(&held[..quarter]);
    let floor = mean(&held[held.len() - quarter..]);
    let on = held
        .iter()
        .filter(|band| band.along_px.abs() > 0.9 * plateau.abs())
        .map(|band| band.offset)
        .max();
    let off = held
        .iter()
        .filter(|band| (band.along_px - floor).abs() < 0.1 * plateau.abs())
        .map(|band| band.offset)
        .filter(|offset| Some(*offset) > on)
        .min();
    let (Some(on), Some(off)) = (on, off) else {
        println!("\nno handover inside these offsets: the field never reaches its floor.");
        return;
    };
    println!(
        "\nthe pane: the along-seam field sits at {:.2} px ({:+.4} deg) out to {:+.2} deg and \
         is\ndown to {:.2} px by {:+.2} deg, so the handover is inside {on:+} to {off:+} px, \
         which is\n{:.2} degrees of view.",
        plateau,
        plateau / options.scale(),
        f64::from(on) / options.scale(),
        floor,
        f64::from(off) / options.scale(),
        f64::from(off - on) / options.scale(),
    );
}

fn updates(samples: &[Sample]) {
    let bend = stepped(samples, bend);
    let slide = stepped(samples, slide);
    println!(
        "\nupdate: the band's own state, frame to frame at {WATCHED} directions. {:.6} deg rms \
         on the\n        bend and {:.6} deg rms on the along-seam field, worst single steps \
         {:.4} and {:.4}.\n        A state that is applied and never updated reads exactly \
         zero on both while the\n        bands above still read whatever it is holding.",
        bend.0, slide.0, bend.1, slide.1,
    );
}

// ------------------------------------------------------------ what is written

/// The two CSVs, both stamped with the file they were read off and the command
/// line that read it.
fn written(
    options: &Options,
    samples: &[Sample],
    offsets: &[i32],
    bands: &[Option<Band>],
) -> Fallible<()> {
    std::fs::create_dir_all(&options.out)?;
    let stamp = stamp(options, samples);
    let mut rows = format!(
        "{stamp}offset_px,frame,time_s,seam_x_px,seam_y_px,seam_tilt_deg,across_px,along_px,peak,pinned,kept\n"
    );
    for (column, offset) in offsets.iter().enumerate() {
        for (index, sample) in samples.iter().enumerate() {
            let Some(fit) = sample.fits[column] else {
                continue;
            };
            writeln!(
                rows,
                "{offset},{index},{:.3},{:.2},{:.2},{:.2},{:.6},{:.6},{:.6},{},{}",
                sample.at.as_secs_f64(),
                sample.seam.x,
                sample.seam.y,
                sample.seam.tilt_deg,
                fit.across,
                fit.along,
                fit.peak,
                u8::from(fit.pinned),
                u8::from(fit.peak > KEEP_PEAK),
            )?;
        }
    }
    let mut summary = format!(
        "{stamp}offset_px,offset_deg,frames,size_px,size_deg,along_px,along_deg,across_px,\
         spread_px,step_rms_px,step_rms_deg,worst_step_deg,bend_rms_deg,steps_over_{STEP_DEG}deg,\
         mean_peak,pinned\n"
    );
    for band in bands.iter().flatten() {
        let scale = options.scale();
        writeln!(
            summary,
            "{},{:.4},{},{:.4},{:.6},{:.4},{:.6},{:.4},{:.4},{:.4},{:.6},{:.6},{:.6},{},{:.4},{}",
            band.offset,
            f64::from(band.offset) / scale,
            band.frames,
            band.size_px,
            band.size_px / scale,
            band.along_px,
            band.along_px / scale,
            band.across_px,
            band.spread_px,
            band.step_px,
            band.step_px / scale,
            band.worst_px / scale,
            band.bend_px / scale,
            band.steps,
            band.peak,
            band.pinned,
        )?;
    }
    let stem = options.stem();
    let mode = options.mode.name();
    for (name, body) in [("frames", rows), ("bands", summary)] {
        let path = options.out.join(format!("{stem}-{mode}-{name}.csv"));
        std::fs::write(&path, body)?;
        println!("wrote:  {}", path.display());
    }
    Ok(())
}

/// The header every CSV carries: which file, which command, which view, and
/// which of the band's two arms.
///
/// A table of numbers with no source on it cannot be attributed once it has
/// been copied out of the directory it was written in, and the instruments'
/// output outlives their runs.
fn stamp(options: &Options, samples: &[Sample]) -> String {
    let source = std::fs::canonicalize(&options.input).unwrap_or_else(|_| options.input.clone());
    let bend = stepped(samples, bend);
    let slide = stepped(samples, slide);
    format!(
        "# instrument: kjerag-spike --bin shear\n\
         # source: {}\n\
         # args: {}\n\
         # view: {}\n\
         # mode: {} over {} frames, {} px square, {:.3} px per degree\n\
         # arms: {}\n\
         # update: bend {:.6} deg rms, along {:.6} deg rms, over {WATCHED} directions\n\
         # note: the band is an IIR filled by a GPU pass, so repeat a run before believing a \
         difference in the fourth decimal\n",
        source.display(),
        options.args,
        options.view.printed(&source),
        options.mode.name(),
        samples.len(),
        options.size,
        options.scale(),
        match options.null {
            true => "both held off (the null)",
            false => "delivered against held off",
        },
        bend.0,
        slide.0,
    )
}

// ------------------------------------------------------------ the arguments

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Four bands across the seam, per frame and in summary.
    Probe,
    /// A thin patch walked across the seam: the applied field's own shape.
    Profile,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Profile => "profile",
        }
    }

    /// Rows and columns of the patch a reading is fitted over.
    ///
    /// The probe's patch is most of the corridor's thickness, so its answer
    /// there is one translation summarising a field that has a gradient; the
    /// profile's is thin across the seam and wide along it, which is what
    /// resolves the same field into a shape.
    fn patch(self) -> (usize, usize) {
        match self {
            Self::Probe => (128, 384),
            Self::Profile => (48, 512),
        }
    }

    /// Where the patches sit, in rows from the seam.
    fn offsets(self) -> Vec<i32> {
        match self {
            // Lens 1's interior, the seam itself, the far side of the
            // handover, and lens 0's picture, which the band never bends.
            Self::Probe => vec![-150, 0, 60, 150],
            Self::Profile => (-240..=240).step_by(12).collect(),
        }
    }
}

/// Which seam correction the map is built with, exactly as `reframe` and `band`
/// take it, so a run here is read through the calibration those two draw.
enum Seam {
    Factory,
    File,
    Stored(SeamFit),
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

struct Options {
    input: PathBuf,
    view: Framing,
    mode: Mode,
    frames: usize,
    /// Seconds of film drawn before the first measured frame, so the band's
    /// filter is converged by the time anything is read off it.
    warm: f64,
    size: u32,
    /// Hold the band on BOTH arms, which makes the two pictures one picture.
    null: bool,
    out: PathBuf,
    seam: Seam,
    /// The whole command line, for the CSV header.
    args: String,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let args: Vec<String> = args.collect();
        let mut options = Self {
            input: PathBuf::new(),
            view: Framing {
                at: Duration::ZERO,
                camera: Camera {
                    yaw: 0.0,
                    pitch: 0.0,
                    fov: 20.0f32.to_radians(),
                },
                horizon: Horizon::Locked,
            },
            mode: Mode::Probe,
            frames: 90,
            warm: 6.0,
            size: 1024,
            null: false,
            out: PathBuf::from("scratch/shear"),
            seam: Seam::File,
            args: args.iter().skip(1).cloned().collect::<Vec<_>>().join(" "),
        };
        let mut view = Vec::new();
        for arg in args.iter().skip(1) {
            if Framing::is_term(arg) {
                view.push(arg.as_str());
                continue;
            }
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("mode", value)) => {
                    options.mode = match value {
                        "probe" => Mode::Probe,
                        "profile" => Mode::Profile,
                        _ => return Err(format!("no mode called {value}. {USAGE}").into()),
                    }
                }
                Some(("frames", value)) => options.frames = value.parse()?,
                Some(("warm", value)) => options.warm = value.parse()?,
                Some(("size", value)) => options.size = value.parse()?,
                Some(("null", value)) => options.null = value.parse::<u32>()? != 0,
                Some(("out", value)) => options.out = PathBuf::from(value),
                Some(("seam", value)) => {
                    options.seam = match value {
                        "factory" => Seam::Factory,
                        "file" => Seam::File,
                        _ => Seam::Stored(seam_fit(value)?),
                    }
                }
                Some((key, _)) => return Err(format!("no argument called {key}. {USAGE}").into()),
            }
        }
        options.view = Framing::read(view)?.ok_or(USAGE)?;
        if options.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        let (rows, cols) = options.mode.patch();
        if options.size as usize <= rows.max(cols) {
            return Err(format!(
                "a {rows}x{cols} patch does not fit in a {} px picture. {USAGE}",
                options.size,
            )
            .into());
        }
        Ok(options)
    }

    /// Where the render starts, which is the warm-up ahead of the window.
    fn start(&self) -> Cue {
        Cue::Time(Duration::from_secs_f64(
            (self.view.at.as_secs_f64() - self.warm.max(0.0)).max(0.0),
        ))
    }

    fn size(&self) -> Size {
        Size::new(self.size, self.size)
    }

    /// View pixels per degree: the picture's width over its field of view.
    ///
    /// The nominal scale of the view and not the rectilinear centre one, which
    /// is 1% smaller at a 20 degree field; every degrees column here is this
    /// scale, so they compare across runs of this instrument.
    fn scale(&self) -> f64 {
        f64::from(self.size) / f64::from(self.view.camera.fov.to_degrees())
    }

    fn stem(&self) -> String {
        self.input
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    }
}

const USAGE: &str = "usage: shear <file.insv> time=seconds yaw=deg pitch=deg fov=deg lock=0|1 \
     [mode=probe|profile] [frames=90] [warm=seconds] [size=px] [null=1] [out=dir] \
     [seam=factory|file|roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9]";
