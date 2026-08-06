//! What the seam band displaces the picture by, and how much of that changes
//! from one frame to the next (issue #103, the motion half).
//!
//! ```sh
//! # four bands across the seam: what was applied at each, frame by frame
//! cargo run --release -p kjerag-spike --bin shear -- <file.insv> \
//!   time=36.303 yaw=160.63 pitch=5.44 fov=20.00 lock=1 frames=90
//! # the same view with a thin patch walked across the seam: the field's shape
//! cargo run --release -p kjerag-spike --bin shear -- <file.insv> \
//!   time=36.303 yaw=160.63 pitch=5.44 fov=20.00 lock=1 frames=90 mode=profile
//! # the null, which has to read exactly zero everywhere
//! cargo run --release -p kjerag-spike --bin shear -- <file.insv> \
//!   time=36.303 yaw=160.63 pitch=5.44 fov=20.00 lock=1 frames=90 null=1
//! # the plant, which has to read back a displacement it was given
//! cargo run --release -p kjerag-spike --bin shear -- <file.insv> \
//!   time=36.303 yaw=160.63 pitch=5.44 fov=20.00 lock=1 frames=90 mode=plant
//! ```
//!
//! **Two arms of one frame, and not two runs of one file.** Every frame is
//! decoded once and drawn twice, through two [`ScenePipeline`]s: the delivered
//! one, and a second held from its first frame by [`ScenePipeline::hold_band`],
//! which leaves the band at the zero that bends nothing. The two pictures hold
//! the same content by construction, so what separates them is the applied
//! field and nothing else, and there is no motion estimate anywhere in here to
//! be wrong about that.
//!
//! **A zero and a known answer, both measured.** `null=1` holds both arms, and
//! then the two pictures are the same picture and every reading is exactly
//! zero: the instrument's own floor. `mode=plant` holds both arms too and draws
//! the second at a camera yawed by a known angle, so every band has a
//! displacement it must read back, at two sizes, and a chain that reads a zero
//! is shown able to read a number as well. Neither is a formality: they are the
//! only two readings in the set whose right answer is known before the run.
//!
//! **Seam-relative and not picture-relative.** Under a locked horizon the body
//! turns beneath the view, so the seam walks across the picture: 330 px over
//! the three seconds this was written for. A row pinned to the picture would be
//! measuring that sweep. Every patch here is placed against the seam's own row,
//! read out of the shipped map (`Reframe`) by walking down the gradient of the
//! angle off the seam plane, so the walk lands on the seam whichever way it
//! runs.
//!
//! **What the patch does not follow is the seam's other two freedoms.** The
//! patch is centred on the picture's own column and lies along its rows. On the
//! reference view the seam's nearest point wanders up to 38.5 px off that
//! column, and at the widest lean the seam rises 54 rows over half the probe
//! patch's 384, so a band's reading is one translation fitted over a strip the
//! seam is not parallel to. That is the summary the corridor bands are, and it
//! is why `mode=profile` exists; a frame whose seam has swung past
//! [`TILT_LIMIT`] is not read at all, and a run where every frame has is
//! refused.
//!
//! **The band is a filter, so a measurement starts warm.** Its state carries
//! frame to frame, so a run that begins at the frame it wants to measure is
//! measuring the state converging. `warm=` seconds are drawn before the first
//! measured frame and thrown away; the measured window begins at the first
//! frame whose own timestamp reaches `time=`, so the window is the same window
//! however the warm-up seek rounded.
//!
//! **Whether it repeats is measured and not designed.** Three runs of the
//! reference command below wrote byte-identical CSVs on one box, the live arm
//! included, and two of `null=1` did too. That is a reading rather than a
//! guarantee: the band's state is an IIR filled by a GPU pass, and the campaign
//! this came out of saw two live renders of one view differ in the third
//! decimal of a pixel. Run a comparison twice before believing a difference
//! that small.
//!
//! The reference reading, on the shimmer view
//! (docs/research/reference-views.md), on one AMD Radeon 760M:
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin shear -- \
//!   ~/Videos/Insta/VID_20260714_193252_00_006.insv \
//!   time=36.303 yaw=160.63 pitch=5.44 fov=20.00 lock=1 frames=90 warm=6.0 \
//!   seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91
//! ```
//!
//! `-150` reads 0.3663 deg applied at 0.0047 deg step rms over 89 pairs; the
//! seam itself (`+0`) 0.3623 at 0.0619 with a worst single step of 0.42; `+60`
//! 0.0578 at 0.1350, over 23 pairs of 30 readings, which is the one band here
//! whose statistic is fragile; and `+150` 0.0003 at 0.0003, which is this
//! instrument's floor on a live arm. The band's own state moves 0.0449 deg rms
//! between frames on the bend and 0.0008 on the along-seam field.
//!
//! **Those four rows were read at a 2 degree handover and they are not where
//! they were.** At the 8 the pass hands over across since 2026-08-05, all four
//! sit inside it (`Mode::offsets`), so `+150` is no longer an unbent floor:
//! at width 8 it reads 0.0243 deg applied. Take the four as distances from the
//! seam and not as places in the handover.
//!
//! **Those readings are stated against a main, and the horizon is why.** Under
//! `lock=1` the view is held against the orientation track, so a change to how
//! that track is seeded moves which part of the sphere the view is pointed at
//! and therefore where the seam lands in the picture. Merging #158 moved the
//! seam's row 23 to 45 px down this window, a mean of about 35, and took
//! `-150` from 0.3641 deg to 0.3663, `+60` from 0.0417 to 0.0578 and its
//! readings from 43 to 30, while the band's own state moved by 0.000002: the
//! band works in the body's frame and the bands work in the view's. A reading here that has moved is a question about
//! what the view is now looking at before it is a question about the band.
//!
//! **The lock going world-fixed on 2026-08-06 is that same caveat at its
//! largest, and the yaw above has been re-derived for it.** The stabilized
//! frame's zero stopped following the aircraft's slow heading, so at
//! `time=36.303` on this file the old zero and the new one are 157 degrees
//! apart: every command here said `yaw=3.78` until that date, and `yaw=160.63`
//! is the same picture, to about 1.6 degrees. **A `lock=1` line older than
//! that date points somewhere else entirely and will run without a word.**
//!
//! Re-read at the new aim, the field is the field it was, which is what says
//! the lock change did not reach the seam: `-150` 0.3646 -> 0.3341 deg applied,
//! `+0` 0.3490 -> 0.3350, `+60` 0.1225 -> 0.0990, `+150` -0.0021 -> 0.0020,
//! the differences being the 1.6 degrees the aim is out. `null=1` reads
//! exactly 0.000000 on every band either side, because the null holds both
//! arms and has no view in it at all.
//!
//! **What did move is this instrument's own floor, and it moved the right
//! way.** The body used to sweep the seam 330 px across the picture in three
//! seconds, and the patch had to be read on a seam that was tilting past
//! [`TILT_LIMIT`] while it did. With the view parked in the world instead, the
//! seam band's step rms falls from 0.0773 deg to 0.0099 and its worst single
//! step from 0.468 to 0.026, and three of the four bands yield more frames
//! (90/79/35/85 -> 90/90/87/78). The seam ladder is worth re-baselining on
//! that, once the reference lines are re-derived.
//!
//! The `seam=` in that command is not decoration either: fitted from the file
//! instead, the same view reads 0.027 deg at `-150` rather than 0.366, because
//! what the band applies is what the calibration left it.
//!
//! CSVs land in gitignored `scratch/`, stamped with the file they were read off
//! and the whole command line that read them: a table of numbers with no source
//! on it is a table nobody can attribute later.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_render::{
    Along, Camera, Cell, Cue, Framing, Horizon, Reframe, Sampling, Scene, ScenePipeline, SeamFit,
    Size,
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

/// How much a band needs before its statistics are printed instead of its
/// counts, applied twice: this many readings that correlated, and this many
/// pairs of them on neighbouring frames.
///
/// Both, because they are not the same number and the second is what the step
/// columns are over. A step rms over a handful of steps is not a step rms, and
/// a band can keep plenty of readings while keeping almost no neighbours: at
/// the handover of the reference view, 30 readings carry 23 pairs and 36 carry
/// 24, and one band of the profile is refused on the pairs alone.
const MIN_FRAMES: usize = 20;

/// What counts as a step, in degrees: the size at which a single frame's change
/// is a jump in the picture rather than the field being carried across it.
const STEP_DEG: f64 = 0.1;

/// How far the seam may lean off the rows before a row offset stops meaning a
/// distance across it, in degrees.
///
/// Per frame and not per run: the lean changes while a capture circles, and on
/// the October X2 view 2 frames of 30 pass this while the other 28 sit between
/// 16 and 30. Those 2 are not read, which leaves a hole the step statistics
/// already refuse to step across, rather than costing the other 28.
const TILT_LIMIT: f64 = 30.0;

/// How many directions the band's own state is watched at.
///
/// Deliberately not [`kjerag_render::AZIMUTHS`], and for `--bin band`'s reason:
/// the bend is applied everywhere and read at `AZIMUTHS` places, so watching
/// the cells alone would report the readings' steadiness and call it the
/// field's.
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
    match options.mode {
        Mode::Plant => planted(&gpu, &options),
        _ => {
            let taken = walk(&gpu, &options)?;
            report(&options, &taken)
        }
    }
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
    /// The map this frame was drawn through. Kept because the lock turns the
    /// body under the view, so which ray looks along a direction of the seam
    /// circle is a question about one frame and not about the run.
    mapped: Reframe,
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
    live.hold_band(options.held());
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
        let banded = draw(gpu, &scene, &mut live, options, options.second())?;
        if at + NAMED >= options.view.at {
            let held = draw(gpu, &scene, &mut plain, options, options.view.camera)?;
            let mapped = scene
                .mapped(options.view.camera, 1.0)
                .ok_or("no frame to map")?;
            let seam = line(&mapped, size).ok_or("the seam does not cross this view")?;
            let (along, cells) = live.band_state(&gpu.device, &gpu.queue)?;
            taken.push(Sample {
                at,
                seam,
                // A frame whose seam has swung past the limit is not read, and
                // the rest of the run still is: the lean is a property of one
                // frame rather than of the view, and a capture that circles
                // passes through it. What that leaves behind is a hole in the
                // readings, which the step statistics already refuse to cross.
                fits: match seam.tilt_deg > TILT_LIMIT {
                    true => vec![None; offsets.len()],
                    false => read(&held, &banded, seam.y, &offsets, patch),
                },
                cells,
                along,
                mapped,
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
    if taken.iter().all(|sample| sample.seam.tilt_deg > TILT_LIMIT) {
        return Err(format!(
            "the seam leans more than {TILT_LIMIT:.0} degrees off the rows on every one of {} \
             frames, so an offset in rows is nowhere a distance across it. this view is not one \
             this instrument can read.",
            taken.len(),
        )
        .into());
    }
    Ok(taken)
}

/// How many frames of a run had their seam past [`TILT_LIMIT`] and were not
/// read.
fn leaning(samples: &[Sample]) -> usize {
    samples
        .iter()
        .filter(|sample| sample.seam.tilt_deg > TILT_LIMIT)
        .count()
}

fn draw(
    gpu: &Gpu,
    scene: &Scene,
    pipeline: &mut ScenePipeline,
    options: &Options,
    camera: Camera,
) -> Fallible<Picture> {
    Render {
        gpu,
        scene,
        pipeline,
    }
    .frame(camera, Sampling::default(), options.size())
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
        tilt_deg: lean(gradient),
    })
}

/// How far the seam leans off the rows, in degrees, 0 to 90.
///
/// The seam runs across its own gradient, so a gradient straight down the
/// columns is a seam lying along the rows. Both components are taken absolute
/// first, and that is the whole of this function's history: `atan2` on the
/// signed pair answers over the half turn, and the sign of the column gradient
/// is which lens the camera has at the bottom of the picture rather than
/// anything about the lean. A capture mounted the other way up read every flat
/// seam as 180 degrees off the rows, so [`TILT_LIMIT`] refused the views this
/// instrument is for and passed the ones it happened to be tried on.
fn lean(gradient: [f64; 2]) -> f64 {
    gradient[0].abs().atan2(gradient[1].abs()).to_degrees()
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
    /// How many of the readings sit on frames next to each other in the film,
    /// which is the count every step statistic above is over. Printed beside
    /// them because it is not `frames`: a band that drops readings has fewer
    /// steps than it has readings, and at the handover it has far fewer.
    pairs: usize,
    /// How many times the readings break, and the longest run of frames a
    /// break covers.
    breaks: usize,
    gap: usize,
}

impl Band {
    /// `None` where too few frames correlated, or too few of them landed next
    /// to each other, to say anything.
    fn of(samples: &[Sample], column: usize, offset: i32, scale: f64) -> Option<Self> {
        let kept: Vec<(usize, Fit)> = samples
            .iter()
            .enumerate()
            .filter_map(|(frame, sample)| Some((frame, sample.fits[column]?)))
            .filter(|(_, fit)| fit.peak > KEEP_PEAK)
            .collect();
        if kept.len() < MIN_FRAMES {
            return None;
        }
        let mean = |of: &[f64]| of.iter().sum::<f64>() / of.len() as f64;
        let rms = |of: &[f64]| mean(&of.iter().map(|v| v * v).collect::<Vec<f64>>()).sqrt();
        let each = |at: fn(&Fit) -> f64| kept.iter().map(|(_, fit)| at(fit)).collect::<Vec<f64>>();
        let sizes = each(|fit| fit.across.hypot(fit.along));
        let size_px = mean(&sizes);
        // Between frames that are next to each other in the film, and no
        // others. A band that lost the frames in between still has readings
        // either side of the gap, and differencing across one would report the
        // field's whole excursion over that gap as a single frame's step.
        let steps: Vec<f64> = kept
            .windows(2)
            .filter(|pair| pair[1].0 == pair[0].0 + 1)
            .map(|pair| {
                (pair[1].1.across - pair[0].1.across).hypot(pair[1].1.along - pair[0].1.along)
            })
            .collect();
        if steps.len() < MIN_FRAMES {
            return None;
        }
        let bends: Vec<f64> = kept
            .windows(3)
            .filter(|three| three[2].0 == three[0].0 + 2)
            .map(|three| {
                let second =
                    |at: fn(&Fit) -> f64| at(&three[2].1) - 2.0 * at(&three[1].1) + at(&three[0].1);
                second(|fit| fit.across).hypot(second(|fit| fit.along))
            })
            .collect();
        let jumps: Vec<usize> = kept.windows(2).map(|pair| pair[1].0 - pair[0].0).collect();
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
            pinned: kept.iter().filter(|(_, fit)| fit.pinned).count(),
            pairs: steps.len(),
            breaks: jumps.iter().filter(|jump| **jump > 1).count(),
            gap: jumps.iter().copied().max().unwrap_or(1),
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

/// What the band applies at one of the [`WATCHED`] directions, in radians, on
/// both of its axes.
///
/// [`Reframe::reading_at`] and not a lookup of our own: what a pixel is bent by
/// is the pair of cells it lands between weighted by the evidence in each and
/// taxed by how much of that reaches [`kjerag_render::KEEP`], and on the
/// reference view 119 of the 128 cells sit under `KEEP`, so the applied
/// strength moves where the raw disparity does not. A straight interpolation of
/// the disparities under-reads this column by 1.75x there.
fn applied(sample: &Sample, direction: usize) -> kjerag_render::Reading {
    sample.mapped.reading_at(
        towards(&sample.mapped, direction),
        &sample.cells,
        sample.along,
    )
}

/// A view ray looking along one direction of the seam circle.
///
/// [`Reframe::reading_at`] asks a ray which azimuth it is over, and what is
/// watched here is the azimuth, so the ray has to be built backwards from it.
/// `body_ray` is a rotation, so its inverse is its transpose, and its columns
/// are what it answers on the three basis rays.
fn towards(mapped: &Reframe, direction: usize) -> [f32; 3] {
    let (sin, cos) = (direction as f32 / WATCHED as f32 * std::f32::consts::TAU).sin_cos();
    let body = [cos, sin, 0.0];
    std::array::from_fn(|axis| {
        let column = mapped.body_ray(std::array::from_fn(|k| f32::from(k == axis)));
        (0..3).map(|row| column[row] * body[row]).sum()
    })
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
        Mode::Profile => {
            table(options, &bands);
            handover(options, &bands);
        }
        _ => {
            frames(samples, &offsets);
            table(options, &bands);
        }
    }
    updates(samples);
    written(options, samples, &offsets, &bands)
}

/// The instrument's own positive control: the same picture twice, the second
/// drawn at a camera yawed by a known angle, so every band has a displacement
/// it must read back.
///
/// Both arms are held, so the band contributes nothing and what is left is the
/// chain this instrument is: the correlation, the parabola under it, and the
/// pixels-per-degree the tables are quoted in. Twice, at one angle and at
/// double it, because a chain that reads one number can be reading a constant.
///
/// The expected displacement is the rectilinear one and not the nominal scale
/// the tables use: a yaw of `d` takes on-axis content `f * tan(d)` columns,
/// where `f` is the half width over the tangent of the half field. Across a
/// probe patch that varies by 0.4 percent, which is under a hundredth of a
/// pixel at these sizes.
fn planted(gpu: &Gpu, options: &Options) -> Fallible<()> {
    let mut lines = String::new();
    let mut passes: Vec<Vec<(i32, f64)>> = Vec::new();
    let mut rows = String::new();
    let mut header = String::new();
    for step in [options.plant, 2.0 * options.plant] {
        let asked = Options {
            plant: step,
            ..options.clone()
        };
        let expected = asked.expected_px();
        let taken = walk(gpu, &asked)?;
        header = stamp(&asked, &taken);
        let mut pass = Vec::new();
        for (column, offset) in options.mode.offsets().iter().enumerate() {
            let Some(band) = Band::of(&taken, column, *offset, options.scale()) else {
                let _ = writeln!(
                    lines,
                    "{offset:>9}{step:>12.3}   too few readings correlate"
                );
                continue;
            };
            let _ = writeln!(
                lines,
                "{offset:>9}{step:>12.3}{expected:>13.4}{:>13.4}{:>12.4}{:>12.6}{:>9}",
                band.along_px,
                band.along_px - expected,
                (band.along_px - expected) / options.scale(),
                band.frames,
            );
            writeln!(
                rows,
                "{offset},{step},{expected:.6},{:.6},{:.6},{:.8},{}",
                band.along_px,
                band.along_px - expected,
                (band.along_px - expected) / options.scale(),
                band.frames,
            )?;
            pass.push((*offset, band.along_px));
        }
        passes.push(pass);
    }
    println!(
        "\nplant:  both arms held off and the second drawn at a known yaw, so every band has \
         a\n        displacement it has to read back. what is under test is the correlation, \
         the\n        parabola and the scale, and nothing of the band at all.\n"
    );
    println!(
        "{:>9}{:>12}{:>13}{:>13}{:>12}{:>12}{:>9}",
        "offset px", "yaw deg", "expected px", "read px", "error px", "error deg", "frames",
    );
    print!("{lines}");
    let ratios: Vec<f64> = passes[0]
        .iter()
        .filter_map(|(offset, small)| {
            let (_, big) = passes[1].iter().find(|(band, _)| band == offset)?;
            Some(big / small)
        })
        .collect();
    println!(
        "\n        doubling the yaw doubles the reading: {} bands read a ratio between {:.4} \
         and {:.4},\n        where a chain answering with a constant would not move at all.",
        ratios.len(),
        ratios.iter().copied().fold(f64::INFINITY, f64::min),
        ratios.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    );
    std::fs::create_dir_all(&options.out)?;
    let path = options
        .out
        .join(format!("{}-plant-bands.csv", options.stem()));
    std::fs::write(
        &path,
        format!("{header}offset_px,yaw_deg,expected_px,read_px,error_px,error_deg,frames\n{rows}"),
    )?;
    println!("wrote:  {}", path.display());
    Ok(())
}

fn heading(options: &Options, samples: &[Sample]) {
    let first = samples.first().expect("a run has frames");
    let last = samples.last().expect("a run has frames");
    let same = samples.iter().filter(|sample| sample.same).count();
    println!(
        "\nview:   {}\nband:   {}, {} frames from {:.3} s to {:.3} s, {} px square at {:.3} \
         px per degree",
        options.view.printed(&options.input),
        options.arms(),
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
    let past = leaning(samples);
    if past > 0 {
        println!(
            "lean:   {past} of {} frames had the seam past {TILT_LIMIT:.0} degrees off the rows \
             and were not read.",
            samples.len(),
        );
    }
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
        "{:>9}{:>11}{:>8}{:>7}{:>10}{:>10}{:>10}{:>10}{:>11}{:>11}{:>11}{:>11}{:>11}{:>7}{:>7}",
        "offset px",
        "offset deg",
        "frames",
        "pairs",
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
            "{:>9}{:>11.2}{:>8}{:>7}{:>10.3}{:>10.4}{:>10.3}{:>10.4}{:>11.3}{:>11.3}{:>11.4}\
             {:>11.4}{:>11.4}{:>7}{:>7.2}",
            band.offset,
            f64::from(band.offset) / scale,
            band.frames,
            band.pairs,
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

/// The bands that read nothing, the readings that hit the wall, and where the
/// readings a step statistic is made of have holes in them. None of the three
/// is something a table of numbers alone would say.
fn refused(bands: &[Option<Band>]) {
    let quiet = bands.iter().filter(|band| band.is_none()).count();
    if quiet > 0 {
        println!(
            "\n{quiet} of {} bands are not printed: fewer than {MIN_FRAMES} readings correlated, \
             or fewer\nthan {MIN_FRAMES} of them landed on frames next to each other, which is \
             what a step is between.",
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
    let broken: Vec<&Band> = bands
        .iter()
        .flatten()
        .filter(|band| band.breaks > 0)
        .collect();
    if broken.is_empty() {
        return;
    }
    println!(
        "\nwhere the readings have holes in them. a step is only ever taken between frames next \
         to\neach other, so a band's step statistics are over `pairs` and not over `frames`, and \
         a\nband that drops readings tells less about the run than its frame count suggests.\n"
    );
    println!(
        "{:>9}{:>9}{:>8}{:>9}{:>11}",
        "offset px", "frames", "pairs", "breaks", "worst gap"
    );
    for band in broken {
        println!(
            "{:>9}{:>9}{:>8}{:>9}{:>11}",
            band.offset, band.frames, band.pairs, band.breaks, band.gap,
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
    let bend = stepped(samples, |sample, at| f64::from(applied(sample, at).epi));
    let slide = stepped(samples, |sample, at| f64::from(applied(sample, at).along));
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
         mean_peak,pinned,step_pairs,breaks,worst_gap\n"
    );
    for band in bands.iter().flatten() {
        let scale = options.scale();
        writeln!(
            summary,
            "{},{:.4},{},{:.4},{:.6},{:.4},{:.6},{:.4},{:.4},{:.4},{:.6},{:.6},{:.6},{},{:.4},\
             {},{},{},{}",
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
            band.pairs,
            band.breaks,
            band.gap,
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
    let bend = stepped(samples, |sample, at| f64::from(applied(sample, at).epi));
    let slide = stepped(samples, |sample, at| f64::from(applied(sample, at).along));
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
        options.arms(),
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
    /// The probe's bands over a displacement the instrument was given.
    Plant,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Profile => "profile",
            Self::Plant => "plant",
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
            Self::Profile => (48, 512),
            _ => (128, 384),
        }
    }

    /// Where the patches sit, in rows from the seam.
    ///
    /// The four fixed rows are the same rows they always were and the handover
    /// has moved out past them. At the view this instrument is quoted at (fov
    /// 20 over 1024 px, 51.2 px per degree) they are **-2.93, 0, +1.17 and
    /// +2.93 degrees** off the seam, so at the 8 degrees the pass hands over
    /// across - 4 either side - every one of them is INSIDE the handover;
    /// positive is lens 0's side. They were named for a 2 degree crossover, and
    /// the names said the last two were past the handover and unbent. Neither
    /// is true any more, and the `+150` column moving with the width is the
    /// measurement that says so.
    fn offsets(self) -> Vec<i32> {
        match self {
            Self::Profile => (-240..=240).step_by(12).collect(),
            _ => vec![-150, 0, 60, 150],
        }
    }
}

/// Which seam correction the map is built with, exactly as `reframe` and `band`
/// take it, so a run here is read through the calibration those two draw.
#[derive(Clone)]
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

#[derive(Clone)]
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
    /// The yaw the second arm is drawn at, in degrees, under `mode=plant`.
    plant: f64,
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
            plant: 0.05,
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
                        "plant" => Mode::Plant,
                        _ => return Err(format!("no mode called {value}. {USAGE}").into()),
                    }
                }
                Some(("frames", value)) => options.frames = value.parse()?,
                Some(("warm", value)) => options.warm = value.parse()?,
                Some(("size", value)) => options.size = value.parse()?,
                Some(("null", value)) => options.null = value.parse::<u32>()? != 0,
                Some(("plant", value)) => options.plant = value.parse()?,
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

    /// Whether the second arm's band is held too, which is what makes a run a
    /// reading with an answer known before it.
    fn held(&self) -> bool {
        self.null || self.mode == Mode::Plant
    }

    /// What the two arms are, for the heading and for every CSV that carries
    /// numbers taken off them.
    fn arms(&self) -> String {
        match (self.mode, self.null) {
            (Mode::Plant, _) => format!("both held off, the second yawed {:+.3} deg", self.plant),
            (_, true) => {
                "both held off, which is the null: the two pictures are one picture".to_owned()
            }
            (_, false) => {
                "the delivered arm against the same frames with the band held off".to_owned()
            }
        }
    }

    /// The camera the second arm is drawn at. The first is always the view as
    /// asked for, and so is this one outside `mode=plant`: `plant` carries a
    /// default so the control has a size to use, and a default that reached
    /// the other modes would put a yaw into every reading they take.
    fn second(&self) -> Camera {
        match self.mode {
            Mode::Plant => Camera {
                yaw: self.view.camera.yaw + (self.plant as f32).to_radians(),
                ..self.view.camera
            },
            _ => self.view.camera,
        }
    }

    /// What a plant of this size displaces on-axis content by, in columns.
    ///
    /// Rectilinear and not the nominal scale: the half width over the tangent
    /// of the half field, times the tangent of the yaw. Negative because a
    /// camera turned one way takes the picture the other.
    fn expected_px(&self) -> f64 {
        let half = f64::from(self.view.camera.fov.to_degrees()) / 2.0;
        -f64::from(self.size) / 2.0 / half.to_radians().tan() * self.plant.to_radians().tan()
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
     [mode=probe|profile|plant] [frames=90] [warm=seconds] [size=px] [null=1] [plant=deg] \
     [out=dir] \
     [seam=factory|file|roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9]";

#[cfg(test)]
mod tests {
    use super::*;

    /// A seam lying along the rows leans by nothing, whichever lens the camera
    /// has at the bottom of the picture.
    ///
    /// The sign of the column gradient is the mounting and not the lean, and
    /// `atan2` on the signed pair answers over the half turn, so the flipped
    /// mounting used to read 180 degrees here and be refused by
    /// [`TILT_LIMIT`]. Both signs, because a test of one is what let it ship.
    #[test]
    fn a_flat_seam_is_flat_whichever_way_up_the_camera_is() {
        assert!(lean([0.0, 1.0]).abs() < 1e-9);
        assert!(lean([0.0, -1.0]).abs() < 1e-9);
    }

    /// And the lean itself is the same both ways up, and is the angle off the
    /// rows rather than its supplement.
    #[test]
    fn the_lean_is_the_angle_off_the_rows() {
        let expect = |gradient: [f64; 2], degrees: f64| {
            assert!(
                (lean(gradient) - degrees).abs() < 1e-9,
                "{gradient:?} read {} and not {degrees}",
                lean(gradient),
            );
        };
        for column in [1.0, -1.0] {
            for row in [1.0, -1.0] {
                expect([row, column], 45.0);
                expect([row * 3.0_f64.sqrt(), column], 60.0);
            }
        }
        // Straight up the columns is a seam standing on end, which is the far
        // end of the range and the one TILT_LIMIT is measured towards.
        expect([1.0, 0.0], 90.0);
        expect([-1.0, 0.0], 90.0);
    }
}
