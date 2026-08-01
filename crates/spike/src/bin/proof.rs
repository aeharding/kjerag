//! The view a screenshot was taken at, and that same view drawn again.
//!
//! A before and after only means anything if both pictures point the same
//! way, and a screenshot carries no camera angles: the owner's stills are a
//! file name and a timestamp. So the view is **fitted**. The app's own pass is
//! drawn under candidate yaw, pitch and field of view, and the candidate whose
//! picture correlates best with his is the one his window was at. Everything
//! after that is the same view drawn through whichever seam path is named, so
//! two renders differ by the calibration and by nothing else.
//!
//! ```sh
//! ffmpeg -i shot.jpg -vf scale=320:-1,format=gray -f rawvideo -y shot.gray
//! cargo run --release -p kyerag-spike --bin proof -- <file.insv> \
//!   time=9.576 lock=1 shot=shot.gray shape=320x225 \
//!   seam=roll:0.789,yaw:-2.450,pitch:-0.668,cx:-2.55,cy:-13.84 out=after.png
//! ```
//!
//! The screenshot arrives as raw 8-bit luma at a stated size rather than as a
//! JPEG: ffmpeg is already under every command here, and a second image
//! decoder to read one picture is not worth carrying.
//!
//! Once the angles are known they are given back as `yaw=`, `pitch=` and
//! `fov=`, and the fit is skipped. That is how the pair is rendered: fit once
//! against the screenshot, then render each seam path at the numbers the fit
//! printed.
//!
//! PNGs land in ./scratch/, which is gitignored: frames of real footage are
//! personal video and this repo is public.

use std::path::{Path, PathBuf};
use std::time::Duration;

use kyerag_media::Fallible;
use kyerag_render::{Camera, Cue, Horizon, Sampling, Scene, ScenePipeline, SeamFit, Size};
use kyerag_spike::{FORMAT, Gpu, Picture, Render};

/// How wide the rendered proof is, in pixels. 1920 because that is the width
/// the seam residuals are quoted in view pixels at (docs 6.8), so what is
/// measured there and what is looked at here are the same picture.
const DEFAULT_WIDTH: u32 = 1920;

/// The coarse sweep: every azimuth, most elevations, five zooms. It has to
/// cover the whole sphere because a screenshot of a flight can point anywhere,
/// and it is cheap only because the frame is decoded once and every candidate
/// after that is one small pass.
const COARSE_STEP_DEG: f64 = 6.0;
const COARSE_PITCH_DEG: f64 = 66.0;
const COARSE_FOVS: [f64; 6] = [20.0, 30.0, 45.0, 60.0, 75.0, 90.0];

/// Where the pattern search starts and stops, in degrees. A tenth of a degree
/// is under two view pixels at 1920 and 90, which is finer than the thing
/// being looked at.
const REFINE_FROM_DEG: f64 = 4.0;
const REFINE_TO_DEG: f64 = 0.02;

/// The zoom range the app itself has (`camera::FOV_MIN` and the ceiling a
/// window leaves): a view the player cannot reach is not a view the owner's
/// screenshot was taken at. Unbounded, the fit walks down to seven degrees of
/// smeared magnification, because a blurred picture correlates with a blur.
const FOV_RANGE_DEG: (f64, f64) = (20.0, 120.0);

/// Which seam correction a render draws with. Verbatim `reframe`'s three
/// paths, because the point of this instrument is to draw the same view
/// through each of them.
enum Seam {
    /// The calibration the camera wrote, uncorrected.
    Factory,
    /// Fitted off this file's own frames, which is what a camera with no
    /// stored calibration gets.
    File,
    /// A stored per-camera calibration, applied as the app applies one at
    /// open.
    Stored(SeamFit),
}

impl Seam {
    fn hold(&self, scene: &Scene) {
        match self {
            Self::Factory => println!("seam:   factory calibration, no correction"),
            Self::File => scene.fit_seam(),
            Self::Stored(fit) => scene.use_seam(*fit),
        }
    }
}

/// The screenshot, as luma at the size the fit is scored at.
struct Shot {
    luma: Vec<f32>,
    size: Size,
}

impl Shot {
    fn load(path: &Path, size: Size) -> Fallible<Self> {
        let bytes = std::fs::read(path)?;
        let wanted = (size.width * size.height) as usize;
        if bytes.len() != wanted {
            return Err(format!(
                "{} holds {} bytes, which is not the {wanted} of a {}x{} gray frame",
                path.display(),
                bytes.len(),
                size.width,
                size.height,
            )
            .into());
        }
        Ok(Self {
            luma: bytes.into_iter().map(f32::from).collect(),
            size,
        })
    }

    /// Zero-mean normalized cross-correlation against a render of the same
    /// size. Zero-mean and normalized because the screenshot has been through
    /// a JPEG and a scale and this render has not: what is being matched is
    /// where the content sits, not what tone it came out at.
    fn agrees_with(&self, picture: &Picture) -> f64 {
        let ours = picture.luma();
        let count = self.luma.len().min(ours.len());
        if count == 0 {
            return 0.0;
        }
        let mean = |values: &[f32]| values[..count].iter().map(|v| f64::from(*v)).sum::<f64>();
        let (mean_a, mean_b) = (mean(&self.luma) / count as f64, mean(&ours) / count as f64);
        let (mut covariance, mut var_a, mut var_b) = (0.0, 0.0, 0.0);
        for (a, b) in self.luma[..count].iter().zip(&ours[..count]) {
            let (a, b) = (f64::from(*a) - mean_a, f64::from(*b) - mean_b);
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

/// The angles whose render agrees best with the screenshot.
///
/// A sweep of the whole sphere, then a pattern search from its winner. The
/// sweep is what makes this an answer rather than a refinement of a guess: a
/// flight's view can point anywhere, and a local search started at the wrong
/// azimuth would find the best wrong answer and report it confidently.
fn fitted(render: &mut Render, shot: &Shot, from: Camera) -> Fallible<(Camera, f64)> {
    let mut scored = |camera: Camera| -> Fallible<f64> {
        let picture = render.frame(camera, Sampling::default(), shot.size)?;
        Ok(shot.agrees_with(&picture))
    };
    let mut best = (from, f64::MIN);
    let steps = (360.0 / COARSE_STEP_DEG) as i32;
    let rows = (COARSE_PITCH_DEG / COARSE_STEP_DEG) as i32;
    for step in 0..steps {
        for row in -rows..=rows {
            for fov in COARSE_FOVS {
                let camera = Camera {
                    yaw: (f64::from(step) * COARSE_STEP_DEG).to_radians() as f32,
                    pitch: (f64::from(row) * COARSE_STEP_DEG).to_radians() as f32,
                    fov: fov.to_radians() as f32,
                };
                let r = scored(camera)?;
                if r > best.1 {
                    best = (camera, r);
                }
            }
        }
    }
    println!(
        "sweep:  yaw {:.0}, pitch {:.0}, fov {:.0}, correlating {:.4}",
        best.0.yaw.to_degrees(),
        best.0.pitch.to_degrees(),
        best.0.fov.to_degrees(),
        best.1,
    );

    let mut step = REFINE_FROM_DEG;
    while step > REFINE_TO_DEG {
        let mut moved = false;
        for axis in 0..3 {
            for sign in [1.0, -1.0] {
                let mut camera = best.0;
                let amount = (sign * step).to_radians() as f32;
                match axis {
                    0 => camera.yaw += amount,
                    1 => camera.pitch += amount,
                    _ => camera.fov += amount,
                }
                let fov = f64::from(camera.fov.to_degrees());
                if fov < FOV_RANGE_DEG.0 || fov > FOV_RANGE_DEG.1 {
                    continue;
                }
                let r = scored(camera)?;
                if r > best.1 {
                    best = (camera, r);
                    moved = true;
                }
            }
        }
        if !moved {
            step /= 2.0;
        }
    }
    Ok(best)
}

/// How far the difference between the two renders is amplified before it is
/// written out. A correction worth looking at moves the picture by a few codes
/// over most of the band, which is invisible at 1x and is the whole question.
const MOVED_GAIN: f64 = 8.0;

/// How big a step counts as the correction having reached a pixel, in 8-bit
/// codes. One code is the rounding of the pass itself.
const MOVED_FLOOR: u8 = 2;

/// The same file open twice, once under each calibration.
///
/// Two opens because the correction lands in a `OnceLock` a scene fills at
/// open, which is the app's own shape: a file plays with one calibration for
/// as long as it is open. The decode is paid twice and then any number of
/// views is free, which is what makes a scan of the whole azimuth circle
/// affordable.
struct Both {
    before: (Scene, ScenePipeline),
    after: (Scene, ScenePipeline),
}

impl Both {
    fn open(gpu: &Gpu, options: &Options) -> Fallible<Self> {
        let opened = |seam: &Seam| -> Fallible<(Scene, ScenePipeline)> {
            let scene = Scene::still(&options.input, options.at)?;
            seam.hold(&scene);
            scene.set_horizon(options.horizon);
            Ok((scene, ScenePipeline::new(&gpu.device, FORMAT)))
        };
        Ok(Self {
            before: opened(&options.before)?,
            after: opened(&options.after)?,
        })
    }

    fn frame(&mut self, gpu: &Gpu, camera: Camera, shape: Size) -> Fallible<(Picture, Picture)> {
        let drawn = |(scene, pipeline): &mut (Scene, ScenePipeline)| {
            Render {
                gpu,
                scene,
                pipeline,
            }
            .frame(camera, Sampling::default(), shape)
        };
        Ok((drawn(&mut self.before)?, drawn(&mut self.after)?))
    }
}

/// The two renders' difference, amplified about mid grey, and the box it falls
/// in.
///
/// The box is the honest answer to "where do I look": it is the region the
/// calibration reached, read off the pixels rather than predicted from the
/// geometry. A correction that moved nothing has no box, and this says so.
fn moved(before: &Picture, after: &Picture) -> (Picture, Option<[u32; 4]>) {
    let width = before.size.width;
    let mut rgba = Vec::with_capacity(before.rgba.len());
    let mut box_of: Option<[u32; 4]> = None;
    for (index, (a, b)) in before
        .rgba
        .chunks_exact(4)
        .zip(after.rgba.chunks_exact(4))
        .enumerate()
    {
        let step = (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0);
        if step >= MOVED_FLOOR {
            let (x, y) = (index as u32 % width, index as u32 / width);
            box_of = Some(match box_of {
                None => [x, y, x, y],
                Some(held) => [
                    held[0].min(x),
                    held[1].min(y),
                    held[2].max(x),
                    held[3].max(y),
                ],
            });
        }
        for channel in 0..3 {
            let step = MOVED_GAIN * (f64::from(a[channel]) - f64::from(b[channel]));
            rgba.push((128.0 + step).clamp(0.0, 255.0) as u8);
        }
        rgba.push(255);
    }
    (
        Picture {
            rgba,
            size: before.size,
        },
        box_of,
    )
}

/// Where round the sphere the correction reaches, one view at a time.
///
/// A calibration only shows up where both lenses are in the picture, and at a
/// narrow zoom most views have nothing of the seam in them at all. This is how
/// the views worth looking at are chosen: rendered both ways, scored by how
/// much of the picture the correction touched, and reported without a picture
/// being written for any of them.
fn scan(gpu: &Gpu, both: &mut Both, options: &Options) -> Fallible<()> {
    let shape = Size::new(SCAN_WIDTH, SCAN_WIDTH * 9 / 16);
    println!(
        "\n{:>6} {:>6} {:>9} {:>9} {:>9}",
        "yaw", "pitch", "moved", "mean", "worst"
    );
    for step in 0..(360 / SCAN_STEP_DEG) {
        let camera = Camera {
            yaw: f32::from(step * SCAN_STEP_DEG).to_radians(),
            ..options.camera
        };
        let (before, after) = both.frame(gpu, camera, shape)?;
        let difference = before.against(&after);
        println!(
            "{:>6.0} {:>6.0} {:>8.1}% {:>9.3} {:>9}",
            camera.yaw.to_degrees(),
            camera.pitch.to_degrees(),
            100.0 * difference.moved as f64 / difference.pixels as f64,
            difference.mean,
            difference.worst,
        );
    }
    Ok(())
}

/// How wide the scan's throwaway renders are, and how far apart its views sit.
const SCAN_WIDTH: u32 = 480;
const SCAN_STEP_DEG: i16 = 10;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    std::fs::create_dir_all("scratch")?;

    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);

    let mut both = Both::open(&gpu, &options)?;
    if options.scan {
        return scan(&gpu, &mut both, &options);
    }

    let camera = match &options.shot {
        None => options.camera,
        Some((path, size)) => {
            let shot = Shot::load(path, *size)?;
            let mut render = Render {
                gpu: &gpu,
                scene: &both.before.0,
                pipeline: &mut both.before.1,
            };
            let (found, r) = fitted(&mut render, &shot, options.camera)?;
            println!(
                "view:   yaw={:.2} pitch={:.2} fov={:.2}, correlating {r:.4} with {}",
                found.yaw.to_degrees(),
                found.pitch.to_degrees(),
                found.fov.to_degrees(),
                path.display(),
            );
            found
        }
    };
    let (before, after) = both.frame(&gpu, camera, options.shape())?;
    let (difference, box_of) = moved(&before, &after);

    for (picture, what) in [
        (&before, "before"),
        (&after, "after"),
        (&difference, "moved"),
    ] {
        let out = picture.write(&gpu, &format!("{}-{what}.png", options.out))?;
        println!("wrote {}", out.display());
    }
    println!(
        "view:   yaw {:.2}, pitch {:.2}, fov {:.2}, {}x{}, horizon {:?}",
        camera.yaw.to_degrees(),
        camera.pitch.to_degrees(),
        camera.fov.to_degrees(),
        options.shape().width,
        options.shape().height,
        options.horizon,
    );
    println!("moved:  {}", before.against(&after).report());
    match box_of {
        Some([x0, y0, x1, y1]) => println!(
            "box:    x {x0} to {x1}, y {y0} to {y1}, which is {} by {} pixels of {} by {}",
            x1 - x0 + 1,
            y1 - y0 + 1,
            options.shape().width,
            options.shape().height,
        ),
        None => println!("box:    the correction reached no pixel of this view"),
    }
    Ok(())
}

struct Options {
    input: PathBuf,
    camera: Camera,
    at: Cue,
    horizon: Horizon,
    /// The two calibrations the same view is drawn through. Nothing else
    /// differs between the two pictures, which is what makes their difference
    /// the correction and not the weather.
    before: Seam,
    after: Seam,
    /// The screenshot to fit the view against, and the size its raw luma is
    /// in. Absent where the angles are given outright.
    shot: Option<(PathBuf, Size)>,
    width: u32,
    /// The shape of the render: the screenshot's where there is one, and this
    /// where there is not.
    aspect: f32,
    /// Report where round the circle the correction reaches, and write no
    /// pictures.
    scan: bool,
    out: String,
}

impl Options {
    fn shape(&self) -> Size {
        let aspect = match &self.shot {
            Some((_, size)) => f64::from(size.width) / f64::from(size.height),
            None => f64::from(self.aspect),
        };
        Size::new(self.width, (f64::from(self.width) / aspect) as u32)
    }
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Fallible<Self> {
        let input = PathBuf::from(args.next().ok_or(USAGE)?);
        let mut camera = Camera::default();
        let mut at = Cue::Index(0);
        let mut horizon = Horizon::Locked;
        let mut before = Seam::Factory;
        let mut after = Seam::Factory;
        let mut scan = false;
        let mut shot = None;
        let mut shape = None;
        let mut width = DEFAULT_WIDTH;
        let mut aspect = 16.0 / 9.0;
        let mut out = None;

        for arg in args {
            let (key, value) = arg.split_once('=').ok_or(USAGE)?;
            match key {
                "yaw" => camera.yaw = value.parse::<f32>()?.to_radians(),
                "pitch" => camera.pitch = value.parse::<f32>()?.to_radians(),
                "fov" => camera.fov = value.parse::<f32>()?.to_radians(),
                "frame" => at = Cue::Index(value.parse()?),
                "time" => at = Cue::Time(Duration::from_secs_f64(value.parse()?)),
                "lock" => {
                    horizon = match value.parse::<u32>()? {
                        0 => Horizon::Free,
                        _ => Horizon::Locked,
                    }
                }
                "before" => before = seam_of(value)?,
                "after" => after = seam_of(value)?,
                "scan" => scan = value.parse::<u32>()? > 0,
                "shot" => shot = Some(PathBuf::from(value)),
                "shape" => shape = Some(pixels(value)?),
                "size" => width = value.parse()?,
                "aspect" => aspect = value.parse()?,
                "out" => out = Some(value.to_owned()),
                _ => return Err(format!("unknown argument {key}. {USAGE}").into()),
            }
        }

        let shot = match (shot, shape) {
            (Some(path), Some(size)) => Some((path, size)),
            (Some(_), None) => return Err("shot= wants the shape= its luma is in".into()),
            (None, _) => None,
        };
        Ok(Self {
            input,
            camera,
            at,
            horizon,
            before,
            after,
            scan,
            shot,
            width,
            aspect,
            out: out.unwrap_or_else(|| "proof.png".to_owned()),
        })
    }
}

/// `320x225`.
fn pixels(value: &str) -> Fallible<Size> {
    let (width, height) = value.split_once('x').ok_or("a shape is WIDTHxHEIGHT")?;
    Ok(Size::new(width.parse()?, height.parse()?))
}

fn seam_of(value: &str) -> Fallible<Seam> {
    Ok(match value {
        "factory" => Seam::Factory,
        "file" => Seam::File,
        _ => Seam::Stored(stored(value)?),
    })
}

/// `roll:0.71,yaw:-2.35,pitch:-1.61,cx:-1.26,cy:-14.60`, in each knob's own
/// units, as the app's config stores them.
fn stored(value: &str) -> Fallible<SeamFit> {
    let mut fit = SeamFit::default();
    for term in value.split(',') {
        let (name, amount) = term.split_once(':').ok_or("a stored knob is knob:amount")?;
        let amount: f64 = amount.parse()?;
        match name {
            "roll" => fit.roll_deg = amount,
            "yaw" => fit.yaw_deg = amount,
            "pitch" => fit.pitch_deg = amount,
            "cx" => fit.cx_px = amount,
            "cy" => fit.cy_px = amount,
            _ => return Err(format!("no stored knob called {name}").into()),
        }
    }
    Ok(fit)
}

const USAGE: &str = "usage: proof <file.insv> [time=seconds | frame=n] [lock=0] \
     [shot=<gray.raw> shape=320x225] [yaw=deg] [pitch=deg] [fov=deg] [size=px] [aspect=w/h] \
     [scan=1] [out=prefix] \
     before=factory|file|roll:..,yaw:..,pitch:..,cx:..,cy:.. after=<the same>";
