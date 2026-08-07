//! What the handover does to FAR content across the seam, in the picture the
//! app actually draws (issue #103, the epi fork; docs/research/stage9.md 9.4).
//!
//! The defect this measures is a **shear**. Where the two lenses disagree
//! across the seam by more than parallax can explain, the band still applies
//! the whole of that disagreement, and it applies it ramped from nothing at
//! one edge of the handover to all of it at the other. On near content that is
//! right. On the horizon it is a straight line drawn with a bend in it: at the
//! registry's BAD May-01 crossing the delivered picture moves far content by
//! 56 view pixels across the corridor and by nothing outside it.
//!
//! ```sh
//! # the reference: the same view with the handover cut to nothing
//! KJERAG_HANDOVER_DEG=0.1 cargo run --release -p kjerag-spike --bin epiramp -- \
//!   <file.insv> time=50.117 yaw=101.13 pitch=0.75 fov=62.79 lock=1 warm=6 \
//!   write=scratch/epiramp/bad-cut.luma
//! # the delivered picture, read against it
//! cargo run --release -p kjerag-spike --bin epiramp -- \
//!   <file.insv> time=50.117 yaw=101.13 pitch=0.75 fov=62.79 lock=1 warm=6 \
//!   against=scratch/epiramp/bad-cut.luma csv=scratch/epiramp/bad.csv
//! ```
//!
//! **Two renders and not one, because the reference has to be the same
//! pipeline.** `--bin reframe` draws the unbent projection and `--bin crossing`
//! builds its map with the band held off; neither is the picture the pilot
//! sees, and PR #167 is the record of what measuring the wrong domain costs.
//! Both arms here are the app's own path with the band live and warm. What
//! differs is `KJERAG_HANDOVER_DEG`: at 0.1 degrees each side of the seam is
//! drawn by one lens with no ramp on it, so the lag between the two renders at
//! a given distance from the contour is what the handover put there and
//! nothing else.
//!
//! **The reference moves with the arm, and it has to.** A build that displaces
//! lens 1's whole picture (`KJERAG_EPI_TERM`) displaces it in the cut render
//! too, so a lag read against that arm's own cut render is the RESIDUAL ramp -
//! what the corridor still does after the displacement - which is the
//! question. Reading one arm against the other arm's cut would measure the
//! displacement instead, which is not a defect and is not what anyone sees.
//!
//! **The plant is the control.** `plant=<view px>` slides the reference's lens
//! 1 side by a known amount before correlating; a probe that cannot see a shift
//! it was handed is not a probe, and the number it reads back is printed beside
//! what it was given.

use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::Fallible;
use kjerag_render::{Camera, Cue, Horizon, Sampling, Scene, ScenePipeline, Size};
use kjerag_spike::{FORMAT, Gpu, Picture, Render, seam_fit};

/// How far either side of the contour the ramp is read, in **degrees** off the
/// seam plane.
///
/// In degrees and not in view pixels, so that one number means the same thing
/// at every view in the registry, whose fields of view run 20 to 219. This is
/// the epi-probe's own 40 to 260 view pixels at the BAD crossing's scale, which
/// is where the method was validated.
///
/// The inner end clears the pixels at the contour where both lenses are at half
/// weight and a correlation is reading a blend of two pictures rather than one.
/// The outer end is just past an 8 degree handover's own edge, 4 degrees, so
/// the last two samples are outside the corridor and pin the far end of the
/// ramp at zero.
const READ_DEG: [f64; 9] = [0.7, 1.1, 1.6, 2.2, 2.7, 3.3, 3.8, 4.4, 4.8];

/// How far the correlation may slide, in view pixels, and how finely.
const REACH_PX: f64 = 80.0;
const STEP_PX: f64 = 0.25;

/// How thick a strip is read at one distance, in view pixels, and how many
/// samples along the seam it takes.
const HALF_PX: f64 = 8.0;
const ALONG_PX: f64 = 2.0;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);

    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let mut scene = Scene::still(&options.input, options.start())?;
    options.seam.hold(&scene);
    scene.set_horizon(match options.lock {
        true => Horizon::Locked,
        false => Horizon::Free,
    });

    // Every frame from `warm` seconds before the view to the view itself,
    // through the pass that draws them: the band's state is the pass's own and
    // only the pass fills it. `--bin step`'s loop, and for its reason.
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
    let (_, at) = scene.frame().ok_or("no frame")?;
    let seam = Seam::of(&mapped, options.size(), options.fov);
    println!(
        "played: {frames} frame(s), ending at {:.3} s, handover {:.2} deg, {:.2} view px per deg",
        at.as_secs_f64(),
        mapped.crossover_at(0.0).to_degrees(),
        seam.px_per_deg,
    );
    let luma = Plane::of(&picture);

    if let Some(path) = &options.write {
        luma.save(path)?;
        println!("wrote:  {} as the reference render", path.display());
        return Ok(());
    }
    let Some(path) = &options.against else {
        return Err("nothing to do: name write= or against=. {USAGE}".into());
    };
    let mut reference = Plane::load(path)?;
    if reference.size != luma.size {
        return Err(format!(
            "the reference is {}x{} and this render is {}x{}",
            reference.size.width, reference.size.height, luma.size.width, luma.size.height
        )
        .into());
    }
    if options.plant != 0.0 {
        reference = reference.planted(&seam, options.plant);
        println!(
            "plant:  the reference's lens 1 side slid {:+.2} view px",
            options.plant
        );
    }

    println!(
        "\n  {:>7} {:>9} {:>7}   {:>7} {:>9} {:>7}",
        "d px", "lag", "r", "d px", "lag", "r"
    );
    let mut rows = Vec::new();
    for degrees in READ_DEG {
        let distance = degrees * seam.px_per_deg;
        let low = lag_at(&luma, &reference, &seam, -distance, options.window);
        let high = lag_at(&luma, &reference, &seam, distance, options.window);
        println!(
            "  {:>7.0} {:>9.2} {:>7.3}   {:>7.0} {:>9.2} {:>7.3}",
            -distance, low.0, low.1, distance, high.0, high.1,
        );
        rows.push((-distance, low));
        rows.push((distance, high));
    }

    let fits = [fitted(&rows, -1.0), fitted(&rows, 1.0)];
    for (side, fit) in ["lens 0 side", "lens 1 side"].iter().zip(&fits) {
        match fit {
            Some((at_contour, slope, spread)) => println!(
                "ramp:   {side}: {at_contour:+.2} view px at the contour, {slope:+.4} px per px, \
                 {spread:.2} px rms about the line",
            ),
            None => println!("ramp:   {side}: no line fitted"),
        }
    }
    // Lens 1's side doubled, and that is the definition, not a convenience.
    // `Reframe::blend_bent` gives each lens the OTHER one's weight times the
    // disagreement, so at the contour the two are half of it each and in
    // opposite directions: the swing across the whole corridor is twice either
    // side's own. Lens 1's is the one taken because it is the side the research
    // term displaces and, at every view read so far, the side whose line
    // describes its own points - the other side's rms is printed above and is
    // what says whether it agrees.
    let ramp = fits[1].map_or(f64::NAN, |fit| 2.0 * fit.0.abs());
    println!(
        "RAMP:   {ramp:.2} view px, {:.4} deg, lens 1's side doubled. {}",
        ramp / seam.px_per_deg,
        match options.plant == 0.0 {
            true => "the far-field shear the handover delivers here",
            false => "AGAINST A PLANTED REFERENCE, so this is the plant plus the shear",
        },
    );

    if let Some(path) = &options.csv {
        write_csv(path, &options, &seam, &rows, &fits, ramp)?;
        println!("wrote:  {}", path.display());
    }
    Ok(())
}

// ------------------------------------------------------------ the geometry

/// Where the seam is in this picture and which way is across it, pixel by
/// pixel.
struct Seam {
    size: Size,
    /// How far past the seam plane each pixel is, in **view pixels**: the
    /// angle off the body's own xy plane, which is what the two lenses hand
    /// over across, taken to pixels at the middle of the picture.
    ///
    /// `--bin step`'s `Field` in the unit this instrument reads in. Infinite
    /// where the frame is not looking at the sphere.
    past: Vec<f64>,
    px_per_deg: f64,
}

impl Seam {
    fn of(mapped: &kjerag_render::Reframe, size: Size, fov_deg: f64) -> Self {
        let width = size.width as usize;
        // `--bin step`'s own scale: how many view pixels one degree is worth
        // at the middle of the picture.
        let px_per_deg =
            f64::from(size.width) / 2.0 / (fov_deg / 2.0).to_radians().tan() * 1f64.to_radians();
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
                    true => f64::from((body[2] / length).asin().to_degrees()) * px_per_deg,
                    false => f64::INFINITY,
                }
            })
            .collect();
        Self {
            size,
            past,
            px_per_deg,
        }
    }

    fn at(&self, x: usize, y: usize) -> f64 {
        self.past[y * self.size.width as usize + x]
    }

    /// The unit normal across the seam at one pixel, pointing towards
    /// increasing `past`, from the field's own gradient. `None` at the edge of
    /// the picture and wherever the field has no value.
    fn normal(&self, x: usize, y: usize) -> Option<[f64; 2]> {
        let (w, h) = (self.size.width as usize, self.size.height as usize);
        if x == 0 || y == 0 || x + 1 >= w || y + 1 >= h {
            return None;
        }
        let step = |a: f64, b: f64| (a.is_finite() && b.is_finite()).then_some(0.5 * (a - b));
        let dx = step(self.at(x + 1, y), self.at(x - 1, y))?;
        let dy = step(self.at(x, y + 1), self.at(x, y - 1))?;
        let length = dx.hypot(dy);
        (length > 1e-9).then_some([dx / length, dy / length])
    }

    /// How far along the seam one pixel is from the middle of the picture, in
    /// view pixels, signed.
    ///
    /// The seam's own tangent is the normal turned a quarter turn, so this is
    /// one dot product. It exists so that a run can be told to read FAR
    /// content only: parallax is what the across-seam axis carries, so a
    /// stretch of seam with the wing or the lines in it answers a different
    /// question from the one this instrument asks, and at the registry's BAD
    /// crossing it is the stretch that breaks the correlation's lock.
    fn along(&self, x: usize, y: usize, normal: [f64; 2]) -> f64 {
        let (dx, dy) = (
            x as f64 - f64::from(self.size.width) / 2.0,
            y as f64 - f64::from(self.size.height) / 2.0,
        );
        dx * -normal[1] + dy * normal[0]
    }

    /// Every pixel whose `past` is within [`HALF_PX`] of `distance` and whose
    /// along-seam position is inside `window`, thinned to one in
    /// [`ALONG_PX`] along the seam, with the normal there.
    fn strip(&self, distance: f64, window: (f64, f64)) -> Vec<([f64; 2], [f64; 2])> {
        let (w, h) = (self.size.width as usize, self.size.height as usize);
        let mut out = Vec::new();
        let stride = ALONG_PX.max(1.0) as usize;
        for y in (0..h).step_by(stride) {
            for x in (0..w).step_by(stride) {
                let past = self.at(x, y);
                if !past.is_finite() || (past - distance).abs() > HALF_PX {
                    continue;
                }
                let Some(normal) = self.normal(x, y) else {
                    continue;
                };
                let along = self.along(x, y, normal);
                if along < window.0 || along > window.1 {
                    continue;
                }
                out.push(([x as f64, y as f64], normal));
            }
        }
        out
    }
}

// ------------------------------------------------------------ the pictures

/// One render as a single luma plane, which is what a doubled edge lives in.
struct Plane {
    luma: Vec<f32>,
    size: Size,
}

impl Plane {
    fn of(picture: &Picture) -> Self {
        let luma = picture
            .rgba
            .chunks_exact(4)
            .map(|px| (f32::from(px[0]) + f32::from(px[1]) + f32::from(px[2])) / 3.0)
            .collect();
        Self {
            luma,
            size: picture.size,
        }
    }

    /// Bilinear, NaN off the picture.
    fn at(&self, x: f64, y: f64) -> f64 {
        let (w, h) = (self.size.width as f64, self.size.height as f64);
        if !(x >= 0.0 && y >= 0.0 && x < w - 1.0 && y < h - 1.0) {
            return f64::NAN;
        }
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let stride = self.size.width as usize;
        let index = y0 as usize * stride + x0 as usize;
        let tap = |at: usize| f64::from(self.luma[at]);
        tap(index) * (1.0 - fx) * (1.0 - fy)
            + tap(index + 1) * fx * (1.0 - fy)
            + tap(index + stride) * (1.0 - fx) * fy
            + tap(index + stride + 1) * fx * fy
    }

    /// The same picture with everything on lens 1's side of the seam slid
    /// across it by `by` view pixels. The positive control, and a mock: what a
    /// probe that cannot see a shift it was handed is worth.
    fn planted(&self, seam: &Seam, by: f64) -> Self {
        let (w, h) = (self.size.width as usize, self.size.height as usize);
        let mut luma = self.luma.clone();
        for y in 0..h {
            for x in 0..w {
                let past = seam.at(x, y);
                if !past.is_finite() || past <= 0.0 {
                    continue;
                }
                let Some(normal) = seam.normal(x, y) else {
                    continue;
                };
                let moved = self.at(x as f64 + normal[0] * by, y as f64 + normal[1] * by);
                if moved.is_finite() {
                    luma[y * w + x] = moved as f32;
                }
            }
        }
        Self {
            luma,
            size: self.size,
        }
    }

    fn save(&self, path: &PathBuf) -> Fallible<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::with_capacity(8 + self.luma.len() * 4);
        bytes.extend_from_slice(&self.size.width.to_le_bytes());
        bytes.extend_from_slice(&self.size.height.to_le_bytes());
        for value in &self.luma {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        std::fs::write(path, bytes)?;
        Ok(())
    }

    fn load(path: &PathBuf) -> Fallible<Self> {
        let bytes = std::fs::read(path)?;
        if bytes.len() < 8 {
            return Err(format!("{} is not a reference render", path.display()).into());
        }
        let word = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let size = Size::new(word(0), word(4));
        let luma: Vec<f32> = bytes[8..]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        if luma.len() != (size.width * size.height) as usize {
            return Err(format!("{} is truncated", path.display()).into());
        }
        Ok(Self { luma, size })
    }
}

// ------------------------------------------------------------ the reading

/// How far the delivered picture has moved the content at `distance` from the
/// contour, in view pixels, and how well the two correlate there.
///
/// A strip of the reference against the same strip of the delivered render,
/// slid along the seam NORMAL. The reference draws one lens unbent there, so
/// the lag is what the handover put into the picture at that distance and
/// nothing else. A positive lag means the delivered picture carries, at
/// `distance`, the content the reference draws at `distance + lag`.
///
/// The mean is taken out of both sides before the product, so a photometric
/// step between the two lenses cannot be paid for with geometry.
fn lag_at(
    delivered: &Plane,
    reference: &Plane,
    seam: &Seam,
    distance: f64,
    window: (f64, f64),
) -> (f64, f64) {
    let strip = seam.strip(distance, window);
    if strip.len() < 200 {
        return (f64::NAN, -2.0);
    }
    let base: Vec<f64> = strip
        .iter()
        .map(|(at, _)| reference.at(at[0], at[1]))
        .collect();
    let mut scores = Vec::new();
    let mut lag = -REACH_PX;
    while lag <= REACH_PX + STEP_PX / 2.0 {
        let moved: Vec<f64> = strip
            .iter()
            .map(|(at, normal)| delivered.at(at[0] + normal[0] * lag, at[1] + normal[1] * lag))
            .collect();
        scores.push((lag, correlation(&base, &moved)));
        lag += STEP_PX;
    }
    let best = scores
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.1.total_cmp(&b.1.1))
        .map(|(index, _)| index)
        .unwrap_or(0);
    if best == 0 || best + 1 == scores.len() {
        return scores[best];
    }
    // The parabola through the peak and its two neighbours, which is what puts
    // a quarter-pixel grid onto a sub-pixel answer.
    let (a, b, c) = (scores[best - 1].1, scores[best].1, scores[best + 1].1);
    let bottom = a - 2.0 * b + c;
    let off = match bottom == 0.0 {
        true => 0.0,
        false => 0.5 * (a - c) / bottom,
    };
    (scores[best].0 + off * STEP_PX, b)
}

fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let live: Vec<(f64, f64)> = a
        .iter()
        .zip(b)
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .map(|(x, y)| (*x, *y))
        .collect();
    if live.len() < 200 {
        return -2.0;
    }
    let count = live.len() as f64;
    let mean = |pick: fn(&(f64, f64)) -> f64| live.iter().map(pick).sum::<f64>() / count;
    let (ma, mb) = (mean(|p| p.0), mean(|p| p.1));
    let mut top = 0.0;
    let (mut va, mut vb) = (0.0, 0.0);
    for (x, y) in &live {
        let (dx, dy) = (x - ma, y - mb);
        top += dx * dy;
        va += dx * dx;
        vb += dy * dy;
    }
    let bottom = (va * vb).sqrt();
    match bottom > 0.0 {
        true => top / bottom,
        false => -2.0,
    }
}

/// The straight line through one side's lags, extrapolated to the contour.
///
/// Returns the lag AT the contour, the slope in pixels per pixel, and the rms
/// about the line. A ramp is what a handover does with a disagreement it
/// cannot remove: zero outside the corridor, the whole disagreement at the
/// contour, straight in between. The lag at the contour is therefore the size
/// of the shear, and the rms is what says whether calling it a line was fair.
fn fitted(rows: &[(f64, (f64, f64))], side: f64) -> Option<(f64, f64, f64)> {
    let points: Vec<(f64, f64)> = rows
        .iter()
        .filter(|(distance, (lag, r))| distance * side > 0.0 && lag.is_finite() && *r > 0.5)
        .map(|(distance, (lag, _))| (*distance, *lag))
        .collect();
    if points.len() < 4 {
        return None;
    }
    let count = points.len() as f64;
    let (mx, my) = (
        points.iter().map(|p| p.0).sum::<f64>() / count,
        points.iter().map(|p| p.1).sum::<f64>() / count,
    );
    let top: f64 = points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
    let bottom: f64 = points.iter().map(|p| (p.0 - mx).powi(2)).sum();
    if bottom <= 0.0 {
        return None;
    }
    let slope = top / bottom;
    let at_contour = my - slope * mx;
    let spread = (points
        .iter()
        .map(|p| (p.1 - (at_contour + slope * p.0)).powi(2))
        .sum::<f64>()
        / count)
        .sqrt();
    Some((at_contour, slope, spread))
}

// ------------------------------------------------------------ the record

/// Every row this run read, stamped with what produced it: the build, the
/// file, the aim, the arm and the reference. A CSV nobody can tell the
/// provenance of is a CSV nobody can check.
fn write_csv(
    path: &PathBuf,
    options: &Options,
    seam: &Seam,
    rows: &[(f64, (f64, f64))],
    fits: &[Option<(f64, f64, f64)>; 2],
    ramp: f64,
) -> Fallible<()> {
    use std::fmt::Write as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = String::new();
    writeln!(
        text,
        "# source: kjerag-spike --bin epiramp ({})",
        env!("CARGO_PKG_VERSION")
    )?;
    writeln!(text, "# file: {}", options.input.display())?;
    writeln!(
        text,
        "# aim: time={} yaw={} pitch={} fov={} lock={} warm={} size={}",
        options.time,
        options.yaw,
        options.pitch,
        options.fov,
        options.lock as u32,
        options.warm,
        options.size,
    )?;
    writeln!(
        text,
        "# arm: KJERAG_EPI_TERM={} KJERAG_HANDOVER_DEG={}",
        std::env::var("KJERAG_EPI_TERM").unwrap_or_else(|_| "(unset)".into()),
        std::env::var("KJERAG_HANDOVER_DEG").unwrap_or_else(|_| "(unset)".into()),
    )?;
    writeln!(
        text,
        "# reference: {}",
        options
            .against
            .as_ref()
            .map_or_else(String::new, |p| p.display().to_string()),
    )?;
    writeln!(text, "# plant: {} view px", options.plant)?;
    writeln!(
        text,
        "# window: {:?} view px along the seam",
        options.window
    )?;
    writeln!(text, "# scale: {:.4} view px per degree", seam.px_per_deg)?;
    for (side, fit) in ["lens0", "lens1"].iter().zip(fits) {
        match fit {
            Some((at, slope, spread)) => writeln!(
                text,
                "# fit {side}: at_contour={at:.4} slope={slope:.6} rms={spread:.4}",
            )?,
            None => writeln!(text, "# fit {side}: none")?,
        }
    }
    writeln!(text, "# ramp: {ramp:.4} view px")?;
    writeln!(text, "distance_px,lag_px,correlation")?;
    for (distance, (lag, r)) in rows {
        writeln!(text, "{distance:.1},{lag:.4},{r:.4}")?;
    }
    std::fs::write(path, text)?;
    Ok(())
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
    plant: f64,
    /// Which stretch of the seam is read, in view pixels along it from the
    /// middle of the picture. The whole of it unless a run says otherwise.
    window: (f64, f64),
    seam: SeamArg,
    write: Option<PathBuf>,
    against: Option<PathBuf>,
    csv: Option<PathBuf>,
}

/// Which of the app's seam paths this render draws with. `--bin step`'s three,
/// and the same words.
enum SeamArg {
    Factory,
    File,
    Stored(kjerag_render::SeamFit),
}

impl SeamArg {
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
            warm: 6.0,
            yaw: 90.0,
            pitch: 0.0,
            fov: 60.0,
            size: 3840,
            lock: true,
            plant: 0.0,
            window: (f64::NEG_INFINITY, f64::INFINITY),
            seam: SeamArg::File,
            write: None,
            against: None,
            csv: None,
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
                Some(("plant", value)) => options.plant = value.parse()?,
                Some(("window", value)) => {
                    let (low, high) = value.split_once(',').ok_or("window=low,high")?;
                    options.window = (low.parse()?, high.parse()?);
                }
                Some(("write", value)) => options.write = Some(PathBuf::from(value)),
                Some(("against", value)) => options.against = Some(PathBuf::from(value)),
                Some(("csv", value)) => options.csv = Some(PathBuf::from(value)),
                Some(("seam", value)) => {
                    options.seam = match value {
                        "factory" => SeamArg::Factory,
                        "file" => SeamArg::File,
                        _ => SeamArg::Stored(seam_fit(value)?),
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
}

const USAGE: &str = "usage: epiramp <file.insv> [time=seconds] [warm=seconds] [yaw=deg] \
     [pitch=deg] [fov=deg] [size=px] [lock=0] [plant=view px] \
     [window=low,high] [seam=factory|file|roll:0.6,yaw:-2.1,pitch:-0.9,cx:-9.5,cy:-11.9] \
     write=reference.luma | against=reference.luma [csv=out.csv]";
