//! Does the horizon stay level? Rendered frames, measured.
//!
//! The verification harness for issue #8. It renders a run of consecutive
//! frames through the app's own pass and reads the angle of the horizon out
//! of each one ([`kyerag_spike::skyline`]), several ways at once, so that the
//! ways can be compared on the same pixels:
//!
//! ```sh
//! cargo run --release -p kyerag-spike --bin horizon -- <file.insv> from=600 count=120
//! cargo run --release -p kyerag-spike --bin horizon -- <file.insv> find=6
//! ```
//!
//! Arguments after the path are `key=value`. `from` is where to start in
//! seconds and `count` is how many frames; `yaw`, `pitch` and `fov` aim the
//! view, in degrees; `width` and `height` size it; `png=n` writes every nth
//! frame of every variant into `scratch/`. `find=n` skips the rendering and
//! prints the n rolliest stretches of the file instead, which is how a
//! stretch worth measuring gets chosen.
//!
//! `tilt=` and `yaw_seconds=` add a filter setting to the comparison, which
//! is how the shipped constants were picked.
//!
//! **Where a Studio export drops in.** Insta360 Studio can export the same
//! clip with its own horizon lock applied. Add it as one more [`Variant`]
//! whose frames come from that file rather than from this pass, measure it
//! with the same `skyline`, and the row it prints is directly comparable to
//! ours: same frames, same measurement, same units. Nothing else has to
//! change. Until then the reference is physics: a horizon is level and an
//! accelerometer at rest reads 1 g.
//!
//! PNGs land in ./scratch/, which is gitignored: they are frames of somebody's
//! real flights and this repo is public.

use std::fs;
use std::path::{Path, PathBuf};

use kyerag_media::Fallible;
use kyerag_meta::{
    CalibrationSet, ExposureTrack, Filter, OrientationTrack, Quat, axis_map, body_from_imu,
};
use kyerag_render::{Camera, Cue, FrameClock, Horizon, Scene, ScenePipeline, Size};
use kyerag_spike::{Gpu, Offscreen, Skyline, skyline};

/// Not sRGB, so the shader writes the video's own numbers straight out and
/// the measurement reads what the window shows.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let calibration = CalibrationSet::from_insv(&options.input)?;
    if let Some(count) = options.find {
        return rolliest(&calibration, count);
    }
    if calibration.imu.is_empty() {
        return Err("this file carries no IMU record, so there is nothing to lock to".into());
    }
    fs::create_dir_all("scratch")?;
    if options.sweep {
        return conventions_against_the_picture(&options, &calibration);
    }

    let variants = options.variants(&calibration);
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);

    let mut scene = Scene::still(&options.input, options.at())?;
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let target = Offscreen::new(&gpu.device, options.size, FORMAT);
    let aspect = options.size.width as f32 / options.size.height as f32;
    let mut runs: Vec<Vec<Option<Skyline>>> = vec![Vec::new(); variants.len()];

    for step in 0..options.count {
        let Some((index, _)) = scene.frame() else {
            break;
        };
        for (variant, run) in variants.iter().zip(&mut runs) {
            variant.apply(&scene, index, &calibration.exposure[0]);
            let primitive = scene.primitive(variant.aim.unwrap_or(options.camera));
            pipeline.prepare(&primitive, &gpu.device, &gpu.queue, aspect);
            target.render(&gpu.device, &gpu.queue, &pipeline)?;
            let pixels = target.read(&gpu.device, &gpu.queue)?;
            run.push(skyline(&pixels, options.size));
            if options.png.is_some_and(|every| step % every == 0) {
                let name = format!("horizon-{}-{index}.png", variant.name);
                target.write_png(&pixels, &PathBuf::from("scratch").join(name))?;
            }
        }
        if !scene.advance()? {
            break;
        }
    }

    println!(
        "view:   {}x{} at yaw {:.0}, pitch {:.0}, fov {:.0}, {} frames from {:.1} s",
        options.size.width,
        options.size.height,
        options.camera.yaw.to_degrees(),
        options.camera.pitch.to_degrees(),
        options.camera.fov.to_degrees(),
        runs.first().map_or(0, Vec::len),
        options.from,
    );
    println!(
        "{:<22} {:>7} {:>9} {:>9} {:>9} {:>9}",
        "variant", "frames", "mean deg", "sd deg", "p-p deg", "worst/f"
    );
    for (variant, run) in variants.iter().zip(&runs) {
        println!("{}", Report::of(&variant.name, run));
    }
    Ok(())
}

/// One way of holding the picture, measured alongside the others on the same
/// frames.
struct Variant {
    name: String,
    /// `None` is horizon lock switched off, which is the picture as it was
    /// before issue #8.
    track: Option<OrientationTrack>,
    clock: FrameClock,
    /// Where to point, when pointing straight ahead would point at nothing.
    ///
    /// A locked variant needs none: pitch zero is the world's own horizon by
    /// construction, so the horizon is across the middle whatever the camera
    /// is doing. An unlocked one is in the body's frame, and this camera is
    /// clamped rolled and pitched down, so straight ahead is the ground.
    /// Aiming it at the horizon on the first frame is what makes the two
    /// comparable: the same content, and then one holds still and the other
    /// swings.
    aim: Option<Camera>,
}

impl Variant {
    fn apply(&self, scene: &Scene, frame: u64, exposure: &ExposureTrack) {
        let Some(track) = &self.track else {
            scene.set_horizon(Horizon::Free);
            scene.hold_at(None);
            return;
        };
        scene.set_horizon(Horizon::Locked);
        scene.hold_at(Some(track.at(self.at(frame, exposure))));
    }

    /// The instant this variant reads the orientation at, which is the whole
    /// of the clock question: the camera's own timestamp for the frame, or
    /// the container's nominal grid.
    fn at(&self, frame: u64, exposure: &ExposureTrack) -> i64 {
        let container = (frame as f64 * 1_001.0 / 30_000.0 * 1e6) as i64;
        match self.clock {
            FrameClock::Exposure => exposure.frame_time_us(frame).unwrap_or(container),
            FrameClock::Container => container,
        }
    }
}

/// What one variant's run of frames came to.
struct Report {
    name: String,
    measured: usize,
    total: usize,
    mean: f64,
    sd: f64,
    swing: f64,
    step: f64,
}

impl Report {
    fn of(name: &str, run: &[Option<Skyline>]) -> Self {
        let angles: Vec<f64> = run.iter().flatten().map(|found| found.degrees).collect();
        let count = angles.len().max(1) as f64;
        let mean = angles.iter().sum::<f64>() / count;
        let sd = (angles.iter().map(|a| (a - mean).powi(2)).sum::<f64>() / count).sqrt();
        // Frame to frame, and only where both frames were measured: a step
        // across a gap is not a step.
        let step = run
            .windows(2)
            .filter_map(|pair| Some((pair[0]?.degrees - pair[1]?.degrees).abs()))
            .fold(0.0f64, f64::max);
        Self {
            name: name.to_owned(),
            measured: angles.len(),
            total: run.len(),
            mean,
            sd,
            swing: angles.iter().fold(f64::MIN, |a, b| a.max(*b))
                - angles.iter().fold(f64::MAX, |a, b| a.min(*b)),
            step,
        }
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.measured == 0 {
            return write!(
                f,
                "{:<22} {:>3}/{:<3} no horizon found in any frame",
                self.name, 0, self.total
            );
        }
        write!(
            f,
            "{:<22} {:>3}/{:<3} {:>9.2} {:>9.2} {:>9.2} {:>9.2}",
            self.name, self.measured, self.total, self.mean, self.sd, self.swing, self.step
        )
    }
}

struct Options {
    input: PathBuf,
    camera: Camera,
    from: f64,
    count: usize,
    size: Size,
    png: Option<usize>,
    find: Option<usize>,
    /// Extra filter settings to compare against the shipped one.
    tilts: Vec<f64>,
    yaws: Vec<f64>,
    /// Extra axis conventions to compare against the file's own.
    axes: Vec<String>,
    sweep: bool,
}

/// The 24 three-letter axis conventions that are rotations.
///
/// A letter names the sensor axis that feeds an output axis and lower case
/// negates it, so there are 48 strings with three different letters in them
/// and half of them are reflections. A reflection cannot be how a sensor is
/// bolted to a camera: the three axes of an IMU are right handed by
/// construction.
fn conventions() -> Vec<String> {
    let mut out = Vec::new();
    for axes in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        for signs in 0..8u32 {
            let name: String = axes
                .iter()
                .enumerate()
                .map(|(slot, axis)| {
                    let letter = ['x', 'y', 'z'][*axis];
                    match signs & (1 << slot) == 0 {
                        true => letter.to_ascii_uppercase(),
                        false => letter,
                    }
                })
                .collect();
            if axis_map(&name).determinant() > 0.0 {
                out.push(name);
            }
        }
    }
    out
}

impl Options {
    fn at(&self) -> Cue {
        Cue::Time(std::time::Duration::from_secs_f64(self.from))
    }

    /// The ways of holding the picture that get compared: the shipped one,
    /// no lock at all, the losing clock, the axis convention
    /// telemetry-parser falls through to for an X4 Air, and whatever filter
    /// settings were asked for.
    fn variants(&self, calibration: &CalibrationSet) -> Vec<Variant> {
        let shipped = Filter::default();
        let solve = |filter: Filter| calibration.orientation(filter);
        let mut variants = vec![
            Variant {
                name: "locked".to_owned(),
                track: Some(solve(shipped)),
                clock: FrameClock::Exposure,
                aim: None,
            },
            Variant {
                name: "free".to_owned(),
                track: None,
                clock: FrameClock::Exposure,
                aim: Some(self.pointed_at_the_horizon(calibration, &solve(shipped))),
            },
            Variant {
                name: "container-clock".to_owned(),
                track: Some(solve(shipped)),
                clock: FrameClock::Container,
                aim: None,
            },
        ];
        // The negative control. If a deliberately wrong axis convention does
        // not read worse than the right one, the instrument is measuring
        // something other than the horizon.
        for axes in self.axes.iter().map(String::as_str).chain(["Xyz", "xZy"]) {
            if axes == calibration.gyro.imu_orientation {
                continue;
            }
            variants.push(Variant {
                name: format!("axes-{axes}"),
                track: Some(shipped.solve(
                    &calibration.imu,
                    body_from_imu(axes, &calibration.lenses[0].pose),
                )),
                clock: FrameClock::Exposure,
                aim: None,
            });
        }
        for tilt_seconds in &self.tilts {
            variants.push(Variant {
                name: format!("tilt-{tilt_seconds}s"),
                track: Some(solve(Filter {
                    tilt_seconds: *tilt_seconds,
                    ..shipped
                })),
                clock: FrameClock::Exposure,
                aim: None,
            });
        }
        for yaw_seconds in &self.yaws {
            variants.push(Variant {
                name: format!("yaw-{yaw_seconds}s"),
                track: Some(solve(Filter {
                    yaw_seconds: *yaw_seconds,
                    ..shipped
                })),
                clock: FrameClock::Exposure,
                aim: None,
            });
        }
        variants
    }

    /// The camera that points at the horizon in the body's own frame, on the
    /// first frame of the run. It stops pointing there immediately, which is
    /// the point.
    fn pointed_at_the_horizon(
        &self,
        calibration: &CalibrationSet,
        track: &OrientationTrack,
    ) -> Camera {
        let at = (self.from * 1e6) as i64;
        let at = calibration.exposure[0]
            .frame_time_us((self.from * 30_000.0 / 1_001.0) as u64)
            .unwrap_or(at);
        // Where the world's straight ahead is in the body's frame, as a yaw
        // and a pitch. The roll it also wants is one the camera does not
        // have, which is why an unlocked view of this footage has its horizon
        // running down the picture rather than across it.
        let ahead = track.at(at).conjugate().rotate([0.0, 0.0, 1.0]);
        Camera {
            yaw: ahead[0].atan2(ahead[2]) as f32,
            pitch: (-ahead[1]).clamp(-1.0, 1.0).asin() as f32,
            ..self.camera
        }
    }

    fn parse(mut args: impl Iterator<Item = String>) -> Fallible<Self> {
        let input = PathBuf::from(args.next().ok_or(USAGE)?);
        let mut options = Self {
            input,
            camera: Camera {
                fov: 100f32.to_radians(),
                ..Camera::default()
            },
            from: 0.0,
            count: 120,
            size: Size::new(960, 540),
            png: None,
            find: None,
            tilts: Vec::new(),
            yaws: Vec::new(),
            axes: Vec::new(),
            sweep: false,
        };
        for arg in args {
            let (key, value) = arg.split_once('=').ok_or(USAGE)?;
            match key {
                "yaw" => options.camera.yaw = value.parse::<f32>()?.to_radians(),
                "pitch" => options.camera.pitch = value.parse::<f32>()?.to_radians(),
                "fov" => options.camera.fov = value.parse::<f32>()?.to_radians(),
                "from" => options.from = value.parse()?,
                "count" => options.count = value.parse()?,
                "width" => options.size.width = value.parse()?,
                "height" => options.size.height = value.parse()?,
                "png" => options.png = Some(value.parse()?),
                "find" => options.find = Some(value.parse()?),
                "tilt" => options.tilts.push(value.parse()?),
                "yaw_seconds" => options.yaws.push(value.parse()?),
                "axes" => options.axes.push(value.to_owned()),
                "sweep" => options.sweep = value.parse::<u32>()? != 0,
                _ => return Err(format!("unknown argument {key}. {USAGE}").into()),
            }
        }
        Ok(options)
    }
}

const USAGE: &str = "usage: horizon <file.insv> [from=seconds] [count=frames] [yaw=deg] \
     [pitch=deg] [fov=deg] [width=px] [height=px] [png=every] [find=n] [tilt=s] \
     [yaw_seconds=s] [axes=yzX] [sweep=1]";

/// The stretches of the file where the camera rolls hardest, which are the
/// ones worth pointing this at: a horizon that stays level through a stretch
/// where nothing moved says nothing.
fn rolliest(calibration: &CalibrationSet, count: usize) -> Fallible<()> {
    const WINDOW_S: f64 = 4.0;
    let to_body = calibration.body_from_imu();
    let samples = calibration.imu.samples();
    let rate = calibration.imu.rate_hz();
    let width = (WINDOW_S * rate) as usize;
    if width == 0 || samples.len() < width {
        return Err("the IMU track is too short to look through".into());
    }

    let mut windows: Vec<(f64, f64)> = samples
        .chunks_exact(width)
        .map(|window| {
            let roll: f64 = window
                .iter()
                .step_by(7)
                .map(|sample| to_body.mul_vec(sample.rate_dps)[2].powi(2))
                .sum();
            (
                (roll / (window.len() / 7) as f64).sqrt(),
                window[0].offset_us as f64 * 1e-6,
            )
        })
        .collect();
    windows.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!("the {count} rolliest {WINDOW_S:.0} s stretches:");
    for (rms, at) in windows.iter().take(count) {
        println!("  from={at:<8.1} roll rate {rms:6.1} deg/s rms");
    }
    Ok(())
}

/// Unused, and kept so the path a Studio export would take is written down
/// rather than described: the reference frames come out of a second file and
/// go through the same measurement.
#[allow(dead_code)]
fn studio_reference(_export: &Path, _frame: u64) -> Option<Quat> {
    None
}

/// Which axis convention puts the accelerometer's "up" where the picture's
/// own horizon says up is.
///
/// The picture is the reference and it needs no lock at all: rendered
/// unlocked, the view is in the camera body's own frame, so a horizon found
/// in it names a great circle in that frame and the normal of that circle is
/// the true vertical in body coordinates, to within the fit. Every
/// convention's answer for the same instant is then just an angle away from
/// it, in degrees, and 23 of the 24 have to be far away or this measurement
/// is not measuring anything.
///
/// Frames whose view holds no horizon contribute nothing, which is why it
/// looks at four yaws and takes the one that fits best.
fn conventions_against_the_picture(
    options: &Options,
    calibration: &CalibrationSet,
) -> Fallible<()> {
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let mut scene = Scene::still(&options.input, options.at())?;
    scene.set_horizon(Horizon::Free);
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let target = Offscreen::new(&gpu.device, options.size, FORMAT);
    let aspect = options.size.width as f32 / options.size.height as f32;

    let names = conventions();
    let maps: Vec<_> = names
        .iter()
        .map(|axes| body_from_imu(axes, &calibration.lenses[0].pose))
        .collect();
    let mut error = vec![0.0f64; names.len()];
    let mut worst = vec![0.0f64; names.len()];
    let mut found = 0usize;

    for _ in 0..options.count {
        let Some((index, _)) = scene.frame() else {
            break;
        };
        let at = calibration.exposure[0].frame_time_us(index).unwrap_or(0);
        let Some(up) =
            vertical_in_the_picture(options, &scene, &gpu, &mut pipeline, &target, aspect)?
        else {
            scene.advance()?;
            continue;
        };
        found += 1;
        let accel = settled(calibration, at);
        for (index, map) in maps.iter().enumerate() {
            let off = angle_between(map.mul_vec(accel), up);
            error[index] += off;
            worst[index] = worst[index].max(off);
        }
        if !scene.advance()? {
            break;
        }
    }
    if found == 0 {
        return Err("no frame in this stretch had a horizon across it".into());
    }

    let mut ranked: Vec<usize> = (0..names.len()).collect();
    ranked.sort_by(|a, b| error[*a].total_cmp(&error[*b]));
    println!(
        "gravity against the picture, {found} of {} frames had a horizon:",
        options.count
    );
    println!("{:<10} {:>12} {:>12}", "axes", "mean deg", "worst deg");
    for index in ranked {
        println!(
            "{:<10} {:>12.2} {:>12.2}{}",
            names[index],
            error[index] / found as f64,
            worst[index],
            match names[index] == calibration.gyro.imu_orientation {
                true => "   <- this camera's",
                false => "",
            }
        );
    }
    Ok(())
}

/// The accelerometer at one instant, smoothed, in the sensor's own axes.
///
/// Smoothed over a second because one sample at 997 Hz is mostly engine
/// vibration, and this is being compared against a horizon, which is not.
fn settled(calibration: &CalibrationSet, at: i64) -> [f64; 3] {
    const WINDOW_US: i64 = 500_000;
    let samples = calibration.imu.samples();
    let from = samples.partition_point(|s| s.offset_us < at - WINDOW_US);
    let to = samples.partition_point(|s| s.offset_us < at + WINDOW_US);
    let window = &samples[from..to.max(from + 1).min(samples.len())];
    let count = window.len().max(1) as f64;
    window.iter().fold([0.0; 3], |held, sample| {
        std::array::from_fn(|axis| held[axis] + sample.accel_g[axis] / count)
    })
}

/// The true vertical in the camera body's frame, read off one unlocked
/// render: two points on the fitted horizon are two body directions, and the
/// normal of the plane they span is the vertical.
fn vertical_in_the_picture(
    options: &Options,
    scene: &Scene,
    gpu: &Gpu,
    pipeline: &mut ScenePipeline,
    target: &Offscreen,
    aspect: f32,
) -> Fallible<Option<[f64; 3]>> {
    let mut best: Option<(f64, [f64; 3])> = None;
    for quarter in 0..4 {
        for lift in [-40.0f32, 0.0, 40.0, 75.0] {
            let camera = Camera {
                yaw: quarter as f32 * std::f32::consts::FRAC_PI_2,
                pitch: lift.to_radians(),
                ..options.camera
            };
            let primitive = scene.primitive(camera);
            pipeline.prepare(&primitive, &gpu.device, &gpu.queue, aspect);
            target.render(&gpu.device, &gpu.queue, pipeline)?;
            let pixels = target.read(&gpu.device, &gpu.queue)?;
            let Some(line) = skyline(&pixels, options.size) else {
                continue;
            };
            if best.is_some_and(|(agreement, _)| agreement >= line.agreement) {
                continue;
            }
            // Two points a long way apart on the fitted line span the plane of
            // the horizon, so its normal is the vertical; a third point on the
            // sky side of the line is what says which of the two normals it is.
            let look = |uv: [f64; 2]| camera.look(uv.map(|c| c as f32), aspect).map(f64::from);
            let middle: [f64; 2] =
                std::array::from_fn(|axis| (line.through[0][axis] + line.through[1][axis]) / 2.0);
            let sky: [f64; 2] = std::array::from_fn(|axis| middle[axis] + line.sky[axis] * 0.2);
            let normal = unit(cross(look(line.through[0]), look(line.through[1])));
            let up = match dot(normal, look(sky)) > 0.0 {
                true => normal,
                false => normal.map(std::ops::Neg::neg),
            };
            best = Some((line.agreement, up));
        }
    }
    Ok(best.map(|(_, up)| up))
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let length = dot(v, v).sqrt();
    v.map(|c| c / length.max(f64::MIN_POSITIVE))
}

fn angle_between(a: [f64; 3], b: [f64; 3]) -> f64 {
    dot(unit(a), unit(b)).clamp(-1.0, 1.0).acos().to_degrees()
}
