//! What a horizon does where it crosses the seam, in the picture the app
//! draws (issue #103, the owner's 2026-08-01 retest of the merged campaign).
//!
//! The owner tested `main` by eye at a named view and reported a visible
//! offset edge on the horizon, "almost a refraction effect". Every acceptance
//! number the campaign carries is a **disparity** - what the two lenses
//! disagree about along the epipolar axis, read off the band's own state at
//! 1920 across 90 degrees. This measures the other thing: where the horizon
//! LANDS in the delivered picture either side of the seam, at the view and
//! the zoom he was looking at.
//!
//! ```sh
//! # the owner's own view line, plus how the state was reached
//! cargo run --release -p kjerag-spike --bin step -- <file.insv> \
//!   time=2.836 yaw=93.99 pitch=4.12 fov=20.00 lock=1 warm=2.0 \
//!   seam=roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91
//! # the same view with the band held off, which is stage 1's own picture
//! cargo run --release -p kjerag-spike --bin step -- <file.insv> ... off=1
//! ```
//!
//! **`warm` is the argument this instrument exists for.** The band's state is
//! per direction and paced in media time, a seek throws it away, and half the
//! circle is read per frame, so what the pass is drawing with depends on how
//! many frames of film ran into this one. `warm=0` is a direct seek and one
//! frame, which is what a `reframe` render and a launch by view line both
//! are; `warm=2.0` is two seconds of playback arriving at the same frame.
//!
//! **What is measured.** The horizon is traced column by column as the
//! sub-pixel row of the strongest sky-to-ground step, a straight line is
//! fitted to the trace on each side of the seam with the crossover and a
//! guard left out, and the two lines are extrapolated to the seam. Their
//! difference there is the **step**: how far the picture moves the horizon
//! across the handover, in view pixels and in degrees. A great circle
//! projects to a straight line in a rectilinear view, so a horizon really is
//! straight and a step in it is the seam's and not the terrain's.
//!
//! `trace=1` writes the trace and both fits as a table, and every run writes
//! an overlay with the seam, the crossover edges, the fitted lines and the
//! trace drawn on the picture. PNGs land in gitignored `scratch/`: these are
//! frames of somebody's real flights and this repo is public.

use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_render::{Camera, Cell, Cue, Horizon, Reframe, Sampling, Scene, ScenePipeline, Size};
use kjerag_spike::{FORMAT, Gpu, Picture, Render, seam_fit};

/// How far either side of the seam the horizon is fitted, in degrees.
///
/// Wide enough that a fit is over many pixels and short enough that the fit
/// is local to the seam: the terrain a horizon runs along is straight over a
/// few degrees and the view is 20 across.
const FIT_DEG: f64 = 4.0;

/// How far either side of the seam is left out of both fits, in degrees.
///
/// The crossover is what the handover happens across and the bend runs to
/// its edge, so a fit that reached into it would be fitting the artifact it
/// is measuring. The widest the band may open is
/// `kjerag_render::band` WIDEST_DEG / 2 either side, and this is comfortably
/// past it.
const GUARD_DEG: f64 = 2.5;

/// How many rows either side of a candidate are averaged before the step
/// across it is taken. `spike::skyline`'s own smoothing, for the same reason:
/// sensor noise and one branch must not out-vote a horizon.
const SMOOTH: usize = 3;

/// How many codes of sky-to-ground step a column has to show before it counts
/// as having a horizon in it.
const CONTRAST: f64 = 8.0;

/// How far a traced column may sit from its own side's line, in multiples of
/// that line's own spread, before it is dropped and the line refitted.
///
/// Relative and not absolute, which is `spike::skyline`'s rule and for its
/// reason: a treeline holds tens of pixels of relief and a fixed tolerance in
/// pixels either keeps a tall tree or throws the horizon away, depending on
/// which horizon it is handed.
const OUTLIER: f64 = 2.0;

/// How many rounds of that.
const REFITS: usize = 3;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);

    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    pipeline.hold_band(options.off);
    let mut scene = Scene::still(&options.input, options.start())?;
    options.seam.hold(&scene);
    scene.set_horizon(match options.lock {
        true => Horizon::Locked,
        false => Horizon::Free,
    });

    // Every frame from `warm` seconds before the view to the view itself,
    // through the pass that draws them, because the state is the pass's own
    // and only the pass fills it.
    let mut frames = 0usize;
    let mut picture = None;
    loop {
        let Some((_, at)) = scene.frame() else {
            break;
        };
        picture = Some(
            Render {
                gpu: &gpu,
                scene: &scene,
                pipeline: &mut pipeline,
            }
            .frame(options.camera(), Sampling::default(), options.size())?,
        );
        frames += 1;
        if at.as_secs_f64() >= options.time || !scene.advance()? {
            break;
        }
    }
    let picture = picture.ok_or("no frame decoded at that instant")?;
    let mapped = scene
        .mapped(options.camera(), 1.0)
        .ok_or("no frame to map")?;
    let cells = pipeline.band_state(&gpu.device, &gpu.queue)?;
    let (_, at) = scene.frame().ok_or("no frame")?;
    println!(
        "played: {frames} frame(s), ending at {:.3} s, band {}",
        at.as_secs_f64(),
        match options.off {
            true => "HELD OFF",
            false => "live",
        },
    );

    let field = Field::of(&mapped, options.size());
    let trace = Trace::of(&picture, &field, &options);
    report(&field, &trace, &cells, &options);
    if options.trace {
        trace.print();
    }
    let out = options.out();
    std::fs::create_dir_all(out.parent().unwrap_or(&PathBuf::from(".")))?;
    overlay(&picture, &field, &trace).save(&gpu, &out)?;
    println!("wrote:  {}", out.display());
    Ok(())
}

// ------------------------------------------------------------ the geometry

/// Where the seam is in this picture, pixel by pixel.
struct Field {
    size: Size,
    /// How far past the seam plane each pixel is, in degrees, signed: the
    /// angle off the body's own xy plane, which is what the two lenses hand
    /// over across.
    past: Vec<f64>,
    /// The crossover's own half width at the seam, in degrees, as the band
    /// has opened it there.
    half_deg: f64,
}

impl Field {
    fn of(mapped: &Reframe, size: Size) -> Self {
        let width = size.width as usize;
        let past = (0..(size.width * size.height) as usize)
            .map(|index| {
                let uv = [
                    (index % width) as f32 / size.width as f32,
                    (index / width) as f32 / size.height as f32,
                ];
                let Some(ray) = mapped.view_ray(uv) else {
                    return f64::INFINITY;
                };
                let body = mapped.body_ray(ray);
                let length = (body[0] * body[0] + body[1] * body[1] + body[2] * body[2]).sqrt();
                match length > 0.0 {
                    true => f64::from((body[2] / length).asin().to_degrees()),
                    false => f64::INFINITY,
                }
            })
            .collect();
        Self {
            size,
            past,
            half_deg: 0.5 * f64::from(mapped.crossover_at(0.0).to_degrees()),
        }
    }

    fn at(&self, x: usize, y: usize) -> f64 {
        self.past[y * self.size.width as usize + x]
    }

    /// How many view pixels one degree is worth at the middle of this
    /// picture, which is what the step is quoted in as well as in degrees.
    fn px_per_deg(&self, camera: Camera) -> f64 {
        let half = f64::from(camera.fov) / 2.0;
        f64::from(self.size.width) / 2.0 / half.tan() * (1.0_f64).to_radians()
    }
}

// ------------------------------------------------------------ the trace

/// The horizon, column by column, and the line each side of the seam fits.
struct Trace {
    /// One entry per column that had a horizon in it: the column, the
    /// sub-pixel row, and how far past the seam that pixel is.
    points: Vec<(f64, f64, f64)>,
    /// The two fits, as slope and intercept in `row = a * past + b`, where
    /// `past` is degrees past the seam. Fitted against the seam angle rather
    /// than against the column so that the extrapolation to the seam is a
    /// read of `b` and the two sides are directly comparable.
    fits: [Option<(f64, f64)>; 2],
    kept: [usize; 2],
}

impl Trace {
    fn of(picture: &Picture, field: &Field, options: &Options) -> Self {
        let luma = picture.luma();
        let (w, h) = (field.size.width as usize, field.size.height as usize);
        let top = SMOOTH;
        let bottom = h - SMOOTH - 1;
        let mut points = Vec::with_capacity(w);
        for x in 0..w {
            let column = |y: usize| f64::from(luma[y * w + x]);
            let mean = |y: usize| {
                (0..SMOOTH).map(|d| column(y - d)).sum::<f64>() / SMOOTH as f64
                    - (1..=SMOOTH).map(|d| column(y + d)).sum::<f64>() / SMOOTH as f64
            };
            let mut best = (0.0, 0usize);
            for y in top..bottom {
                let step = mean(y);
                if step > best.0 {
                    best = (step, y);
                }
            }
            if best.0 < CONTRAST {
                continue;
            }
            // Sub-pixel, by the parabola through the step's own peak: an
            // integer row cannot resolve what this exists to argue about.
            let (y, peak) = (best.1, best.0);
            let (minus, plus) = (mean(y - 1), mean(y + 1));
            let curve = minus - 2.0 * peak + plus;
            let refined = match curve < 0.0 {
                true => (0.5 * (minus - plus) / curve).clamp(-1.0, 1.0),
                false => 0.0,
            };
            let past = field.at(x, y);
            if !past.is_finite() {
                continue;
            }
            points.push((x as f64, y as f64 + refined, past));
        }
        let mut trace = Self {
            points,
            fits: [None, None],
            kept: [0, 0],
        };
        for side in 0..2 {
            let (fit, kept) = trace.fit(side, options);
            trace.fits[side] = fit;
            trace.kept[side] = kept;
        }
        trace
    }

    /// Side 0 is the near hemisphere's, side 1 the far one's: `past` negative
    /// and positive. A straight line, refitted with the points furthest from
    /// it dropped, because a treeline holds one tall tree and a horizon does
    /// not bend.
    fn fit(&self, side: usize, options: &Options) -> (Option<(f64, f64)>, usize) {
        let inside = |past: f64| {
            let far = past.abs();
            far >= options.guard && far <= options.guard + FIT_DEG && (past < 0.0) == (side == 0)
        };
        let mut kept: Vec<(f64, f64)> = self
            .points
            .iter()
            .filter(|(_, _, past)| inside(*past))
            .map(|(_, row, past)| (*past, *row))
            .collect();
        for _ in 0..REFITS {
            let Some(line) = line(&kept) else {
                return (None, 0);
            };
            let spread = rms(&kept, line);
            let before = kept.len();
            kept.retain(|(past, row)| (row - (line.0 * past + line.1)).abs() <= OUTLIER * spread);
            if kept.len() == before {
                break;
            }
        }
        (line(&kept), kept.len())
    }

    /// The trace as a median row per quarter degree of seam angle, which is
    /// what a terrain that is not a razor can be read through: the seam is a
    /// straight line in this picture, so a step in the profile at zero is the
    /// handover's and a slope either side of it is the hill's.
    fn print(&self) {
        println!(
            "\nthe horizon, binned by how far past the seam it sits. `rows` is the median \n\
             sub-pixel row of the sky-to-ground step over the columns in that bin.\n"
        );
        println!("{:>10} {:>8} {:>8}", "past deg", "columns", "row");
        let mut bin = -100i32;
        let mut rows: Vec<f64> = Vec::new();
        let flush = |bin: i32, rows: &mut Vec<f64>| {
            if rows.is_empty() {
                return;
            }
            rows.sort_by(f64::total_cmp);
            println!(
                "{:>10.2} {:>8} {:>8.2}",
                f64::from(bin) * 0.25,
                rows.len(),
                rows[rows.len() / 2],
            );
            rows.clear();
        };
        let mut ordered: Vec<&(f64, f64, f64)> = self.points.iter().collect();
        ordered.sort_by(|a, b| a.2.total_cmp(&b.2));
        for (_, row, past) in ordered {
            let at = (past / 0.25).floor() as i32;
            if at != bin {
                flush(bin, &mut rows);
                bin = at;
            }
            rows.push(*row);
        }
        flush(bin, &mut rows);
    }
}

/// How far the kept points sit from their own line, in pixels.
fn rms(points: &[(f64, f64)], line: (f64, f64)) -> f64 {
    let total: f64 = points
        .iter()
        .map(|(past, row)| (row - (line.0 * past + line.1)).powi(2))
        .sum();
    (total / points.len().max(1) as f64).sqrt()
}

/// Least squares of `row` against `past`, or `None` with too few points to
/// mean anything.
fn line(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    if points.len() < 8 {
        return None;
    }
    let n = points.len() as f64;
    let (sx, sy) = points
        .iter()
        .fold((0.0, 0.0), |(sx, sy), (x, y)| (sx + x, sy + y));
    let (mx, my) = (sx / n, sy / n);
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (x, y) in points {
        sxy += (x - mx) * (y - my);
        sxx += (x - mx) * (x - mx);
    }
    if sxx <= 0.0 {
        return None;
    }
    let slope = sxy / sxx;
    Some((slope, my - slope * mx))
}

// ------------------------------------------------------------ the report

fn report(field: &Field, trace: &Trace, cells: &[Cell], options: &Options) {
    let px_per_deg = field.px_per_deg(options.camera());
    println!(
        "\nseam:   crossover {:.2} deg wide at zero disparity, {:.1} view px; \
         the view is {:.1} px per degree",
        2.0 * field.half_deg,
        2.0 * field.half_deg * px_per_deg,
        px_per_deg,
    );
    let (Some(near), Some(far)) = (trace.fits[0], trace.fits[1]) else {
        println!("step:   no horizon fitted on both sides of the seam");
        return;
    };
    let step = far.1 - near.1;
    println!(
        "fits:   near side slope {:+.3} px/deg over {} columns, far side {:+.3} over {}",
        near.0, trace.kept[0], far.0, trace.kept[1],
    );
    println!(
        "step:   {:+.2} view px at the seam, which is {:+.4} deg. \
         Slopes differ by {:+.3} px/deg.",
        step,
        step / px_per_deg,
        far.0 - near.0,
    );
    band_says(cells, px_per_deg);
}

/// What the band's own state says about the same seam, so the picture's
/// answer and the pass's answer are printed side by side.
fn band_says(cells: &[Cell], px_per_deg: f64) {
    let mut measured = 0;
    let (mut sum, mut worst) = (0.0f64, 0.0f64);
    let (mut off_sum, mut off_worst) = (0.0f64, 0.0f64);
    for cell in cells {
        if cell.confidence <= 0.0 {
            continue;
        }
        measured += 1;
        let applied = f64::from(cell.disparity).to_degrees().abs();
        sum += applied;
        worst = worst.max(applied);
        let off = f64::from(cell.off_epi).to_degrees().abs();
        off_sum += off;
        off_worst = off_worst.max(off);
    }
    if measured == 0 {
        println!("band:   nothing measured: the state is the zero a file opens in");
        return;
    }
    println!(
        "band:   {measured} of {} directions have evidence; mean |disparity| {:.3} deg \
         ({:.1} px), worst {:.3} deg ({:.1} px)",
        cells.len(),
        sum / measured as f64,
        sum / measured as f64 * px_per_deg,
        worst,
        worst * px_per_deg,
    );
    println!(
        "        off-epi, measured and NEVER applied: mean {:.3} deg ({:.1} px), \
         worst {:.3} deg ({:.1} px)",
        off_sum / measured as f64,
        off_sum / measured as f64 * px_per_deg,
        off_worst,
        off_worst * px_per_deg,
    );
}

/// The picture with the seam, the crossover edges, the trace and the two fits
/// drawn on it, because a number about a horizon is worth what the picture
/// beside it says.
fn overlay(picture: &Picture, field: &Field, trace: &Trace) -> Picture {
    let mut rgba = picture.rgba.clone();
    let (w, h) = (field.size.width as usize, field.size.height as usize);
    let mut paint = |x: usize, y: usize, colour: [u8; 3]| {
        if x >= w || y >= h {
            return;
        }
        let at = (y * w + x) * 4;
        rgba[at..at + 3].copy_from_slice(&colour);
    };
    for y in 0..h {
        for x in 1..w {
            let (before, now) = (field.at(x - 1, y), field.at(x, y));
            if !before.is_finite() || !now.is_finite() {
                continue;
            }
            for (edge, colour) in [
                (0.0, [255, 0, 0]),
                (field.half_deg, [255, 160, 0]),
                (-field.half_deg, [255, 160, 0]),
            ] {
                if (before - edge).signum() != (now - edge).signum() {
                    paint(x, y, colour);
                }
            }
        }
    }
    for (x, row, _) in &trace.points {
        paint(*x as usize, *row as usize, [0, 255, 255]);
    }
    for x in 0..w {
        for (side, fit) in trace.fits.iter().enumerate() {
            let Some((a, b)) = fit else { continue };
            let mut column = None;
            for y in 0..h {
                let past = field.at(x, y);
                if past.is_finite() && (past < 0.0) == (side == 0) {
                    column = Some(past);
                    break;
                }
            }
            let Some(past) = column else { continue };
            paint(x, (a * past + b).round().max(0.0) as usize, [255, 0, 255]);
        }
    }
    Picture {
        rgba,
        size: field.size,
    }
}

// ------------------------------------------------------------ the arguments

struct Options {
    input: PathBuf,
    time: f64,
    warm: f64,
    yaw: f64,
    pitch: f64,
    fov: f64,
    size: u32,
    lock: bool,
    off: bool,
    guard: f64,
    trace: bool,
    seam: Seam,
    out: Option<PathBuf>,
}

/// Which of the app's three seam paths this render draws with. `reframe`'s
/// own three, and the same words, because the whole question here is what the
/// owner's own config draws.
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

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            time: 0.0,
            warm: 0.0,
            yaw: 90.0,
            pitch: 0.0,
            fov: 20.0,
            size: 1024,
            lock: true,
            off: false,
            guard: GUARD_DEG,
            trace: false,
            seam: Seam::File,
            out: None,
        };
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("time", value)) => options.time = value.parse()?,
                Some(("warm", value)) => options.warm = value.parse()?,
                Some(("yaw", value)) => options.yaw = value.parse()?,
                Some(("pitch", value)) => options.pitch = value.parse()?,
                Some(("fov", value)) => options.fov = value.parse()?,
                Some(("size", value)) => options.size = value.parse()?,
                Some(("lock", value)) => options.lock = value.parse::<u32>()? != 0,
                Some(("off", value)) => options.off = value.parse::<u32>()? != 0,
                Some(("guard", value)) => options.guard = value.parse()?,
                Some(("trace", value)) => options.trace = value.parse::<u32>()? != 0,
                Some(("out", value)) => options.out = Some(PathBuf::from(value)),
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
        if options.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        Ok(options)
    }

    fn start(&self) -> Cue {
        Cue::Time(Duration::from_secs_f64((self.time - self.warm).max(0.0)))
    }

    fn size(&self) -> Size {
        Size::new(self.size, self.size)
    }

    fn camera(&self) -> Camera {
        Camera {
            yaw: self.yaw.to_radians() as f32,
            pitch: self.pitch.to_radians() as f32,
            fov: self.fov.to_radians() as f32,
        }
    }

    fn out(&self) -> PathBuf {
        self.out.clone().unwrap_or_else(|| {
            PathBuf::from("scratch").join(format!(
                "step-t{:.3}-yaw{:.0}-fov{:.0}-warm{:.1}{}.png",
                self.time,
                self.yaw,
                self.fov,
                self.warm,
                match self.off {
                    true => "-off",
                    false => "",
                },
            ))
        })
    }
}

const USAGE: &str = "usage: step <file.insv> [time=seconds] [warm=seconds] [yaw=deg] \
     [pitch=deg] [fov=deg] [size=px] [lock=0] [off=1] [guard=deg] [trace=1] \
     [seam=factory|file|roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9] [out=name.png]";
