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
//!   time=2.836 yaw=111.83 pitch=4.12 fov=20.00 lock=1 warm=2.0 \
//!   seam=pool
//! # the same view with the band held off, which is stage 1's own picture
//! cargo run --release -p kjerag-spike --bin step -- <file.insv> ... off=1
//! ```
//!
//! **The `seam=` there was a literal string until 2026-08-07, and it was the
//! wrong pose.** It said `roll:0.577,yaw:-2.077,pitch:-0.936,cx:-9.53,cy:-11.91`,
//! which is the knob-by-knob median of the owner's pool and no member of it:
//! the combination `SeamPool::answer` stopped shipping on 2026-08-05
//! (docs/research/seam-two-axis.md 4), so the app had not drawn it since.
//! `seam=pool` asks for the pose the app draws rather than copying it, and a
//! run prints the five knobs it applied. **Nothing recorded below has been
//! re-read at the drawn pose.**
//!
//! **That `yaw` is re-derived and a stale one runs without a word.** The lock
//! became world-fixed on 2026-08-06, so the frame a `lock=1` yaw is measured
//! in no longer follows the aircraft's slow heading and its zero is the file's
//! opening heading instead. The line above said `yaw=93.99` until that date
//! and is the same picture at `111.83`. `new_yaw = old_yaw + carried(t)`,
//! where `carried` is how far the old follow had been taken by then; `--bin
//! carried` computes it for a line and docs/research/reference-views.md has
//! the rule and the re-derived registry.
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
//! across the handover, in view pixels and in degrees.
//!
//! **Two windows, and they do not agree** (issue #103, stage 6). This was
//! written on the argument that a great circle projects to a straight line, so
//! a horizon is straight and a step in it is the seam's. What the trace
//! follows on real footage is a treeline or a ridge a few kilometres off,
//! which is not a great circle, and extrapolating a straight line to the seam
//! from four degrees out turns that curvature into step: the owner's own
//! reference frame with the band held off reads 10.4, 20.9, 30.5, 32.8 and
//! 37.8 view px at `guard` 1.2, 1.6, 2.0, 2.5 and 3.5. So `step:` is the
//! campaign's own window, kept so its earlier numbers stay readable, and
//! `close:` is the same measurement over the two degrees just outside the
//! crossover, where the fits' own rms says a line describes the points. A
//! DIFFERENCE between two builds is trustworthy in either window, because the
//! along-seam correction rotates one hemisphere and moves that side's whole
//! trace by a constant (23.2 px in all three windows, measured).
//!
//! `trace=1` writes the trace and both fits as a table, and every run writes
//! an overlay with the seam, the crossover edges, the fitted lines and the
//! trace drawn on the picture. PNGs land in gitignored `scratch/`: these are
//! frames of somebody's real flights and this repo is public.

use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_render::{
    Along, Camera, Cell, Cue, Horizon, Reframe, Sampling, Scene, ScenePipeline, Size,
};
use kjerag_spike::{FORMAT, Gpu, Picture, Render, Seam};

/// How far either side of the seam the horizon is fitted, in degrees.
///
/// Wide enough that a fit is over many pixels and short enough that the fit
/// is local to the seam: the terrain a horizon runs along is straight over a
/// few degrees and the view is 20 across.
const FIT_DEG: f64 = 4.0;

/// How far the close-in fit reaches, in degrees (issue #103, stage 6).
///
/// Two, because the line has to describe its own points and a ridge does not
/// stay straight for longer than that: on the owner's reference frame the fit
/// rms is 0.97 and 0.51 px over this window against 2.07 and 0.84 over
/// [`FIT_DEG`] from `guard=2.5`, which was [`GUARD_DEG`] when that was
/// measured.
const CLOSE_DEG: f64 = 2.0;

/// How far past the crossover's own edge the close-in fit starts.
///
/// Small, and it can be: it is measured against the crossover this frame drew
/// rather than against the widest one stage 4 may open, so what it has to
/// clear is the taper and not a worst case.
const CLOSE_MARGIN_DEG: f64 = 0.2;

/// How far either side of the seam is left out of both fits, in degrees.
///
/// The crossover is what the handover happens across and the bend runs to its
/// edge, so a fit that reached into it would be fitting the artifact it is
/// measuring. What it has to clear is therefore the crossover the pass
/// **draws**, and since 2026-08-05 that is 8 degrees on an X4-class file - 4
/// either side - rather than the 2.89 of `kjerag_render::band` WIDEST_DEG the
/// old value was derived from. This is that 4 plus the same
/// [`CLOSE_MARGIN_DEG`] the close-in window clears the taper by.
///
/// **Every number this instrument printed before that date was taken at 2.5**,
/// which was a degree and a half outside a 2 degree crossover and is a degree
/// and a half inside an 8 degree one. Absolute steps from the two windows are
/// not comparable; a difference between two builds is, for the reason
/// [`Trace::close`] gives.
const GUARD_DEG: f64 = 4.2;

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

/// The outermost along-seam offset the band's search can return, in degrees,
/// read off the band itself rather than written down here: a reading at the
/// rail is the search running out and not an answer, and this instrument's
/// whole point is to count them.
fn rail_deg() -> f64 {
    f64::from(kjerag_render::PERP_DEG) - 1e-3
}

/// How many columns either side of the crossing the horizon's own slope is
/// read over, so the attribution below knows what this edge can and cannot
/// show. Wide enough to average a treeline, narrow enough to be local.
const CROSSING_PX: f64 = 120.0;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);

    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    pipeline.hold_band(options.off);
    let mut scene = Scene::still(&options.input, options.start())?;
    scene.use_table(options.table);
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
    while let Some((_, at)) = scene.frame() {
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
    let (along, cells) = pipeline.band_state(&gpu.device, &gpu.queue)?;
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
    report(&mapped, &field, &trace, &cells, along, &options);
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
    /// The two fits over [`FIT_DEG`] starting at [`Options::guard`], which is
    /// the window every acceptance number in this campaign has been quoted at.
    fits: [Option<Fitted>; 2],
    /// The same two over [`CLOSE_DEG`], starting just outside this frame's own
    /// crossover (issue #103, stage 6).
    ///
    /// **The wide window's premise is that a horizon is straight, and on real
    /// footage it is not.** What the trace follows is a treeline or a ridge at
    /// a few kilometres, which is not a great circle: on the owner's own
    /// reference frame one side reads +5.32 px/deg of slope over the two
    /// degrees outside the crossover and +2.03 over `guard=2.5`'s window, at
    /// twice the fit rms. Extrapolating a straight line from four degrees out
    /// turns that curvature into step, and the same frame with the band held
    /// off reads 10.4, 20.9, 30.5, 32.8 and 37.8 view px at `guard` 1.2, 1.6,
    /// 2.0, 2.5 and 3.5.
    ///
    /// The DIFFERENCE between two builds is window-independent, because the
    /// along-seam correction is a rotation of one hemisphere and moves that
    /// side's whole trace by a constant: 23.2 px in all three windows,
    /// measured. So the campaign's before-and-after deltas stand and its
    /// absolute numbers carry the terrain as well as the seam. This column is
    /// the absolute one: the shortest window a line still describes.
    close: [Option<Fitted>; 2],
}

/// One side's straight line through the trace, and how well it describes it.
struct Fitted {
    /// Slope and intercept in `row = a * past + b`, where `past` is degrees
    /// past the seam. Fitted against the seam angle rather than against the
    /// column so that the extrapolation to the seam is a read of `b` and the
    /// two sides are directly comparable.
    line: (f64, f64),
    kept: usize,
    /// How far the kept points sit from that line, in pixels. A step read off
    /// two lines that do not describe their own points is a step read off the
    /// terrain.
    rms: f64,
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
            close: [None, None],
        };
        // Just outside the crossover this frame actually drew, which is what
        // stage 4 may have opened rather than what it opens at zero disparity.
        let from = field.half_deg + CLOSE_MARGIN_DEG;
        for side in 0..2 {
            trace.fits[side] = trace.fit(side, options.guard, FIT_DEG);
            trace.close[side] = trace.fit(side, from, CLOSE_DEG);
        }
        trace
    }

    /// Side 0 is the near hemisphere's, side 1 the far one's: `past` negative
    /// and positive. A straight line, refitted with the points furthest from
    /// it dropped, because a treeline holds one tall tree and a horizon does
    /// not bend.
    fn fit(&self, side: usize, from: f64, reach: f64) -> Option<Fitted> {
        let inside = |past: f64| {
            let far = past.abs();
            far >= from && far <= from + reach && (past < 0.0) == (side == 0)
        };
        let mut kept: Vec<(f64, f64)> = self
            .points
            .iter()
            .filter(|(_, _, past)| inside(*past))
            .map(|(_, row, past)| (*past, *row))
            .collect();
        for _ in 0..REFITS {
            let line = line(&kept)?;
            let spread = rms(&kept, line);
            let before = kept.len();
            kept.retain(|(past, row)| (row - (line.0 * past + line.1)).abs() <= OUTLIER * spread);
            if kept.len() == before {
                break;
            }
        }
        let line = line(&kept)?;
        Some(Fitted {
            line,
            kept: kept.len(),
            rms: rms(&kept, line),
        })
    }

    /// Where the traced horizon crosses the seam, and how steeply it runs
    /// there: the slope in rows per column, fitted over the columns nearest
    /// the crossing on both sides of it.
    fn crossing(&self) -> Option<(f64, (f64, f64))> {
        let at = self
            .points
            .iter()
            .min_by(|a, b| a.2.abs().total_cmp(&b.2.abs()))?;
        let near: Vec<(f64, f64)> = self
            .points
            .iter()
            .filter(|(x, _, _)| (x - at.0).abs() <= CROSSING_PX)
            .map(|(x, row, _)| (*x, *row))
            .collect();
        let (slope, intercept) = line(&near)?;
        Some((slope, (at.0, slope * at.0 + intercept)))
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

fn report(
    mapped: &Reframe,
    field: &Field,
    trace: &Trace,
    cells: &[Cell],
    along: Along,
    options: &Options,
) {
    let px_per_deg = field.px_per_deg(options.camera());
    println!(
        "\nseam:   crossover {:.2} deg wide at zero disparity, {:.1} view px; \
         the view is {:.1} px per degree",
        2.0 * field.half_deg,
        2.0 * field.half_deg * px_per_deg,
        px_per_deg,
    );
    let ([Some(near), Some(far)], close) = (&trace.fits, &trace.close) else {
        println!("step:   no horizon fitted on both sides of the seam");
        return;
    };
    let step = far.line.1 - near.line.1;
    println!(
        "fits:   near side slope {:+.3} px/deg over {} columns at rms {:.2}, \
         far side {:+.3} over {} at {:.2}",
        near.line.0, near.kept, near.rms, far.line.0, far.kept, far.rms,
    );
    println!(
        "step:   {:+.2} view px at the seam, which is {:+.4} deg. \
         Slopes differ by {:+.3} px/deg.",
        step,
        step / px_per_deg,
        far.line.0 - near.line.0,
    );
    close_in(close, px_per_deg);
    if let Some(rows) = attribute(mapped, field, trace, step) {
        applied_at(mapped, trace, cells, along, field, rows);
    }
    band_says(cells, along, px_per_deg);
}

/// The same step off the two degrees just outside the crossover, where a
/// straight line still describes the trace (issue #103, stage 6).
///
/// Printed beside the wide one rather than instead of it: the wide window is
/// what every earlier acceptance number in this campaign was quoted at, and a
/// column that quietly changed meaning would make those numbers unreadable.
/// Where the two disagree, the rms columns say which line is describing its own
/// points and which is describing the hill.
fn close_in(close: &[Option<Fitted>; 2], px_per_deg: f64) {
    let [Some(near), Some(far)] = close else {
        println!("close:  no horizon fitted on both sides just outside the crossover");
        return;
    };
    let step = far.line.1 - near.line.1;
    println!(
        "close:  {:+.2} view px, {:+.4} deg, off the {:.1} deg starting {:.2} past the \
         crossover edge\n\
         \x20       (near {} columns at rms {:.2}, far {} at {:.2}; a ridge is not a great \
         circle, so the\n\
         \x20       wide window above extrapolates its curvature into step)",
        step,
        step / px_per_deg,
        CLOSE_DEG,
        CLOSE_MARGIN_DEG,
        near.kept,
        near.rms,
        far.kept,
        far.rms,
    );
}

/// Which of the seam's two axes the step is on.
///
/// A row difference is all an edge can report, so it has to be attributed
/// before it means anything. The epipolar axis is the one a distance
/// displaces content along and the one the band bends; the axis across it is
/// the one only the calibration can reach, and **nothing in the pass ever
/// corrects it** before issue #103 stage 5. What is printed is how many rows
/// one degree on each axis would move this horizon by, so the measured step
/// can be read as degrees of either, and those two numbers are returned so
/// that what the pass applied there can be read in the same units.
fn attribute(mapped: &Reframe, field: &Field, trace: &Trace, step: f64) -> Option<(f64, f64)> {
    let Some((slope, at)) = trace.crossing() else {
        println!("axes:   no crossing to attribute the step at");
        return None;
    };
    let (w, h) = (f64::from(field.size.width), f64::from(field.size.height));
    let uv = |x: f64, y: f64| [(x / w) as f32, (y / h) as f32];
    let (Some(ray), Some(right), Some(down)) = (
        mapped.view_ray(uv(at.0, at.1)),
        mapped.view_ray(uv(at.0 + 1.0, at.1)),
        mapped.view_ray(uv(at.0, at.1 + 1.0)),
    ) else {
        println!("axes:   the crossing is off the map");
        return None;
    };
    let Some(ring) = mapped.seam_at(ray) else {
        println!("axes:   the crossing is not on the seam circle");
        return None;
    };
    // The rotation `body_ray` applies, as three columns, so its transpose
    // takes a body direction back into the view's own frame.
    let columns: [[f32; 3]; 3] = std::array::from_fn(|axis| {
        mapped.body_ray(std::array::from_fn(|c| f32::from(u8::from(c == axis))))
    });
    let reach = (0..3)
        .map(|c| f64::from(ray[c]) * f64::from(ray[c]))
        .sum::<f64>()
        .sqrt();
    let radian = 1.0_f64.to_radians() * reach;
    let tangents = [right, down]
        .map(|other| std::array::from_fn::<f64, 3, _>(|c| f64::from(other[c]) - f64::from(ray[c])));
    // How many rows this edge shows of one degree along a body axis: the
    // displacement resolved into pixels, then across the edge.
    let shown = |axis: [f32; 3]| {
        let view: [f64; 3] = std::array::from_fn(|c| {
            (0..3)
                .map(|a| f64::from(columns[c][a]) * f64::from(axis[a]) * radian)
                .sum()
        });
        let (dx, dy) = resolve(tangents, view);
        dy - slope * dx
    };
    let (epi, perp) = (shown(ring.epi), shown(ring.perp));
    println!(
        "axes:   the horizon crosses the seam at phi {:.0} deg, drawn at {:+.3} px per px. \
         One degree epipolar moves it {:+.1} rows, one degree across the epipolar axis \
         {:+.1} rows.",
        ring.phi.to_degrees(),
        slope,
        epi,
        perp,
    );
    let read = |name: &str, rows: f64| match rows.abs() > 1.0 {
        true => println!("        as {name}, the step is {:+.3} deg", step / rows),
        false => println!("        {name} is edge-on here: this horizon cannot show it"),
    };
    read("epipolar (depth)", epi);
    read("along the seam (the camera)", perp);
    Some((epi, perp))
}

/// A view-space displacement as pixels, least squares over the two pixel
/// tangents, which are not orthogonal in general.
fn resolve(tangents: [[f64; 3]; 2], delta: [f64; 3]) -> (f64, f64) {
    let dot = |a: [f64; 3], b: [f64; 3]| (0..3).map(|c| a[c] * b[c]).sum::<f64>();
    let (a, b, c) = (
        dot(tangents[0], tangents[0]),
        dot(tangents[0], tangents[1]),
        dot(tangents[1], tangents[1]),
    );
    let (p, q) = (dot(tangents[0], delta), dot(tangents[1], delta));
    let determinant = a * c - b * b;
    match determinant != 0.0 {
        true => ((p * c - q * b) / determinant, (a * q - b * p) / determinant),
        false => (0.0, 0.0),
    }
}

/// What the band's own state says about the same seam, so the picture's
/// answer and the pass's answer are printed side by side.
///
/// Both channels, each with its own evidence, because since stage 5 they are
/// smoothed apart, refused apart and applied apart, and a single count would
/// hide exactly the case this instrument was built to catch.
fn band_says(cells: &[Cell], along: Along, px_per_deg: f64) {
    let channel = |live: fn(&Cell) -> bool, of: fn(&Cell) -> f32| {
        let read: Vec<f64> = cells
            .iter()
            .filter(|cell| live(cell))
            .map(|cell| f64::from(of(cell)).to_degrees().abs())
            .collect();
        let count = read.len();
        let sum: f64 = read.iter().sum();
        let worst = read.iter().copied().fold(0.0, f64::max);
        let railed = read.iter().filter(|deg| **deg >= rail_deg()).count();
        (count, sum / count.max(1) as f64, worst, railed)
    };
    let (epi_n, epi_mean, epi_worst, _) = channel(|c| c.confidence > 0.0, |c| c.disparity);
    let (off_n, off_mean, off_worst, railed) = channel(|c| c.off_conf > 0.0, |c| c.off_epi);
    if epi_n == 0 && off_n == 0 {
        println!("band:   nothing measured: the state is the zero a file opens in");
        return;
    }
    println!(
        "band:   epipolar (depth): {epi_n} of {} directions have evidence; mean {epi_mean:.3} deg \
         ({:.1} px), worst {epi_worst:.3} deg ({:.1} px)",
        cells.len(),
        epi_mean * px_per_deg,
        epi_worst * px_per_deg,
    );
    println!(
        "        along the seam (the camera): {off_n} with evidence; mean {off_mean:.3} deg \
         ({:.1} px), worst {off_worst:.3} deg ({:.1} px)",
        off_mean * px_per_deg,
        off_worst * px_per_deg,
    );
    println!(
        "        {railed} of those {off_n} sit ON the {:.2} deg search limit, which is {:.0} \
         percent",
        rail_deg(),
        100.0 * railed as f64 / off_n.max(1) as f64,
    );
    println!(
        "        the field: roll {:+.3} deg, one cycle {:.3} deg at phase {:.0}, two cycles \
         {:.3} at {:.0},\n        over {:.1} directions of evidence",
        f64::from(along.terms[0]).to_degrees(),
        f64::from(along.terms[1].hypot(along.terms[2])).to_degrees(),
        f64::from(along.terms[2].atan2(along.terms[1])).to_degrees(),
        f64::from(along.terms[3].hypot(along.terms[4])).to_degrees(),
        f64::from(along.terms[4].atan2(along.terms[3])).to_degrees() / 2.0,
        along.evidence,
    );
    match kjerag_render::depth_leak(cells) {
        Some(leak) => println!(
            "        leak: the two channels correlate at {leak:+.3} round the ring. Parallax is \
             epipolar by construction,\n        so anything but zero here is depth reaching an \
             axis that cannot hold it",
        ),
        None => println!("        leak: too few directions have both channels to say"),
    }
}

/// What the pass actually applies where the horizon crosses, which is not the
/// same question as what the ring holds on average.
///
/// The two cells a crossing falls between are read the way the shader reads
/// them - each channel at its own evidence, taxed by `KEEP` - and the answer
/// is turned into rows through the same two axis sensitivities the
/// attribution above prints. A correction the ring has and the crossing does
/// not is the failure this exists to name.
fn applied_at(
    mapped: &Reframe,
    trace: &Trace,
    cells: &[Cell],
    along: Along,
    field: &Field,
    rows: (f64, f64),
) {
    let Some((_, at)) = trace.crossing() else {
        return;
    };
    let (w, h) = (f64::from(field.size.width), f64::from(field.size.height));
    let Some(ray) = mapped.view_ray([(at.0 / w) as f32, (at.1 / h) as f32]) else {
        return;
    };
    let reading = mapped.reading_at(ray, cells, along);
    let (epi, along) = (
        f64::from(reading.epi).to_degrees(),
        f64::from(reading.along).to_degrees(),
    );
    println!(
        "at it:  the pass applies {epi:+.3} deg epipolar and {along:+.3} deg along the seam \
         where the horizon crosses,\n        which is {:+.1} and {:+.1} rows of this edge",
        epi * rows.0,
        along * rows.1,
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
    // Both windows' fits, so the overlay shows what the two numbers in the
    // report were read off: magenta is the wide one, cyan-green the close-in.
    for x in 0..w {
        for (fits, colour) in [(&trace.fits, [255, 0, 255]), (&trace.close, [0, 255, 128])] {
            for (side, fit) in fits.iter().enumerate() {
                let Some(fit) = fit else { continue };
                let mut column = None;
                for y in 0..h {
                    let past = field.at(x, y);
                    if past.is_finite() && (past < 0.0) == (side == 0) {
                        column = Some(past);
                        break;
                    }
                }
                let Some(past) = column else { continue };
                let (a, b) = fit.line;
                paint(x, (a * past + b).round().max(0.0) as usize, colour);
            }
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
    /// The along-seam table the picture is drawn with (issue #103, stage 9).
    /// `Table::REST` unless a run names one, so a run that does not is the
    /// picture this instrument has always measured, byte for byte.
    table: kjerag_render::Table,
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

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            table: kjerag_render::Table::REST,
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
        let mut seam = String::from("file");
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
                Some(("table", value)) => options.table = kjerag_spike::seam_table(value)?,
                Some(("seam", value)) => seam = value.to_string(),
                Some((key, _)) => return Err(format!("no argument called {key}. {USAGE}").into()),
            }
        }
        if options.input.as_os_str().is_empty() {
            return Err(USAGE.into());
        }
        // After the loop, because `seam=pool` is resolved against the file and
        // the file may be named anywhere on the line.
        options.seam = Seam::parse(&seam, &options.input)?;
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
     [table=table.txt] [seam=factory|file|pool|roll:0.8,yaw:-2.3,pitch:-0.9,cx:-3.3,cy:-11.9] \
     [out=name.png]";
