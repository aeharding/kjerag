//! Does the horizon stay level all the way **round** a pan? Rendered frames,
//! measured, and fitted to the shape the defect has.
//!
//! The verification harness for issue #45. `horizon.rs` measures one view
//! over a run of frames, which catches sway; this one measures a run of
//! views at one instant, which catches a constant tilt in the estimated
//! world vertical. The two defects look nothing alike and neither instrument
//! sees the other's:
//!
//! ```sh
//! cargo run --release -p kyerag-spike --bin dip -- <file.insv> from=1500
//! cargo run --release -p kyerag-spike --bin dip -- <file.insv> from=1500 inject=1
//! ```
//!
//! Arguments after the path are `key=value`. `from` is where to start in
//! seconds, `count` how many frames, `steps` how many yaws the circle is cut
//! into; `pitch` and `fov` shape the view and `width`/`height` size it.
//! `inject=deg` tilts the held orientation by a known angle about the world
//! axis `about=deg` names, which is the positive control: the fit has to read
//! the injected tilt back.
//!
//! `only=name` runs one variant, `bias=a_b_c_d_e_f` hands in six `gyro_calib`
//! doubles to test as a bias, and `png=n` writes every nth render into
//! `scratch/`, which is gitignored: these are frames of somebody's real
//! flights and this repo is public.
//!
//! ## The measurement this instrument threw away
//!
//! It shipped with the equivalent of `limit=20`: a found line more than 20
//! degrees off level was taken to be a wing or a field boundary and dropped.
//! The defect it exists to measure is 40 to 50 degrees over the first seconds
//! of the April 10 capture, so every view of the real thing was dropped and
//! the fit ran on what was left near the zero crossings. It reported 1.9 to
//! 8.5 degrees with a wandering phase while the owner was looking at 45
//! degrees with a fixed one, and issue #57 was prescribed from those numbers.
//!
//! The gate is off unless a run asks for one, and counting stands in its
//! place: every view it drops is printed, so a run measuring a fraction of
//! what it rendered says so. The injection controls carry the other half of
//! the lesson. `inject=1` passed on a stretch whose baseline was already
//! small, which is a control that never left the range the instrument
//! happened to work in; a control has to span the regime being measured, so
//! `inject=45` is the one that matters here and it is in the acceptance run.
//!
//! ## What is measured, and why there are two of it
//!
//! **The tilt vector** is the physics. Two points on the fitted horizon are
//! two directions in the frame the camera reads its rays in, and with the
//! lock on that frame is the stabilized world, so the normal of the plane
//! they span is where the picture says up is. Where the estimate says up is
//! is `[0, -1, 0]` by construction. The angle between them is the whole
//! defect, it is measured in **one** render, and it does not depend on which
//! way the view was pointed.
//!
//! **The sinusoid** is what a pilot sees. A tilt of `e` towards azimuth
//! `phi` puts the horizon `atan2(sin e sin(yaw - phi), cos e)` off level and
//! about `e cos(yaw - phi)` off centre, so panning a circle dips it once each
//! way. The fit of angle against yaw is that curve, and for a small `e` it is
//! `e sin(yaw - phi)`, so the amplitude has to come back equal to the tilt
//! vector's length or one of the two measurements is wrong.
//!
//! At the tens of degrees issue #45 turned out to be, the two stop agreeing
//! and the tilt column is the one to read: the curve is still periodic but no
//! longer a sinusoid, and a three-coefficient fit reports its fundamental
//! rather than its peak. At `e = 45` that fundamental is 47.4 degrees, which
//! is what `inject=45` reads back in the fit column against 45.0 in the tilt
//! column. The height also leaves the frame there, which is the other reason
//! the fit thins out: at a 100 degree field of view the picture reaches 34
//! degrees above and below its own axis, so the yaws where a 45 degree tilt
//! puts the horizon furthest off centre have no horizon in them at all.

use std::f64::consts::TAU;
use std::fs;
use std::path::PathBuf;

use kyerag_media::Fallible;
use kyerag_meta::{
    CalibrationSet, Filter, GyroSample, GyroTrack, Mat3, OrientationTrack, Pose, Quat, axis_map,
};
use kyerag_render::{Camera, Cue, Horizon, Scene, ScenePipeline, Size};
use kyerag_spike::{Gpu, Offscreen, skyline};

/// Not sRGB, so the shader writes the video's own numbers straight out and
/// the measurement reads what the window shows.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Where the estimate says up is, in the frame the camera reads its rays in.
/// y is down, so up is its negative.
const UP: [f64; 3] = [0.0, -1.0, 0.0];

/// No gate on how far from level a line may be. [`skyline`] answers in
/// (-90, 90], so nothing it finds can exceed this.
const NO_LIMIT: f64 = 90.0;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let calibration = CalibrationSet::from_insv(&options.input)?;
    if calibration.imu.is_empty() {
        return Err("this file carries no IMU record, so there is nothing to lock to".into());
    }
    if let Some(count) = options.worst {
        return worst(&calibration, count);
    }
    if options.settle {
        return settling(&calibration);
    }
    fs::create_dir_all("scratch")?;

    let variants = options.variants(&calibration);
    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);
    let mut scene = Scene::still(&options.input, options.at())?;
    scene.set_horizon(Horizon::Locked);
    let mut pipeline = ScenePipeline::new(&gpu.device, FORMAT);
    let target = Offscreen::new(&gpu.device, options.size, FORMAT);
    let aspect = options.size.width as f32 / options.size.height as f32;
    let mut runs: Vec<Vec<Point>> = vec![Vec::new(); variants.len()];
    let mut rendered = 0usize;
    let mut found = 0usize;
    let mut inverted = 0usize;
    let mut gated = 0usize;

    for _ in 0..options.count {
        let Some((index, _)) = scene.frame() else {
            break;
        };
        let at = calibration.exposure[0].frame_time_us(index).unwrap_or(0);
        for (variant, run) in variants.iter().zip(&mut runs) {
            let held = options.injected(variant.held(at));
            scene.hold_at(Some(held));
            for turn in 0..options.steps {
                let yaw = turn as f64 * TAU / options.steps as f64;
                if !variant.reaches(yaw) {
                    continue;
                }
                let camera = Camera {
                    yaw: yaw as f32,
                    pitch: options.pitch,
                    fov: options.fov,
                };
                pipeline.prepare(&scene.primitive(camera), &gpu.device, &gpu.queue, aspect);
                target.render(&gpu.device, &gpu.queue, &pipeline)?;
                let pixels = target.read(&gpu.device, &gpu.queue)?;
                if options
                    .png
                    .is_some_and(|every| rendered.is_multiple_of(every))
                {
                    let name = format!("dip-{}-{index}-{:.0}.png", variant.name, yaw.to_degrees());
                    target.write_png(&pixels, &PathBuf::from("scratch").join(name))?;
                }
                rendered += 1;
                let Some(line) = skyline(&pixels, options.size) else {
                    continue;
                };
                found += 1;
                // The sky has to be above the line, or what was found is not a
                // horizon this way up.
                if line.sky[1] >= 0.0 {
                    inverted += 1;
                    continue;
                }
                // And a run may ask for the old gate back. It is off by
                // default because a line far from level is only a wing or a
                // field boundary where the defect is known to be small, and
                // that is the assumption this instrument was built to test.
                if line.degrees.abs() > options.limit {
                    gated += 1;
                    continue;
                }
                let look = |uv: [f64; 2]| camera.look(uv.map(|c| c as f32), aspect).map(f64::from);
                let normal = unit(cross(look(line.through[0]), look(line.through[1])));
                let up = match dot(normal, UP) > 0.0 {
                    true => normal,
                    false => normal.map(std::ops::Neg::neg),
                };
                run.push(Point {
                    yaw,
                    body_yaw: wrap(yaw - held.heading()),
                    angle: line.degrees,
                    // How far the horizon sits below the middle of the view,
                    // in degrees: the view axis is this far above the plane
                    // the horizon lies in.
                    height: -dot(up, look([0.5, 0.5]))
                        .clamp(-1.0, 1.0)
                        .asin()
                        .to_degrees(),
                    up,
                });
            }
        }
        if !scene.advance()? {
            break;
        }
    }

    // What was dropped, said out loud. An instrument that measures a fraction
    // of what it rendered and does not report the fraction is how issue #45
    // was mis-attributed for a day.
    println!(
        "views:  {rendered} rendered, {found} with a line in them, {inverted} of those \
         upside down and {gated} past limit={:.0}, leaving {} measured",
        options.limit,
        found - inverted - gated,
    );
    println!(
        "view:   {}x{} at pitch {:.0}, fov {:.0}, {} yaws of {} frames from {:.1} s",
        options.size.width,
        options.size.height,
        options.pitch.to_degrees(),
        options.fov.to_degrees(),
        options.steps,
        options.count,
        options.from,
    );
    if options.inject != 0.0 {
        println!(
            "inject: {:.2} deg about the world axis at {:.0} deg",
            options.inject.to_degrees(),
            options.about.to_degrees(),
        );
    }
    println!(
        "\n{:<16} {:>6} {:>9} {:>9} {:>13} {:>9} {:>9} {:>11} {:>10}",
        "variant",
        "views",
        "tilt deg",
        "toward",
        "angle fit deg",
        "view phase",
        "rms",
        "body phase",
        "height fit"
    );
    for (variant, run) in variants.iter().zip(&runs) {
        println!("{}", Row::of(&variant.name, run));
    }
    if options.dump {
        // The fit is a model; this is what it was fitted to. A defect that
        // lives in the view reads the same shape in this table whatever the
        // flying was doing, and one that lives in the world does not.
        for (variant, run) in variants.iter().zip(&runs) {
            println!(
                "\nview yaw against horizon angle and height, {}:",
                variant.name
            );
            println!(
                "{:>9} {:>7} {:>11} {:>11}",
                "view yaw", "views", "angle deg", "height deg"
            );
            for turn in 0..options.steps {
                let yaw = turn as f64 * TAU / options.steps as f64;
                let at: Vec<&Point> = run
                    .iter()
                    .filter(|point| (point.yaw - yaw).abs() < 1e-9)
                    .collect();
                if at.is_empty() {
                    continue;
                }
                let mean = |of: fn(&Point) -> f64| {
                    at.iter().map(|point| of(point)).sum::<f64>() / at.len() as f64
                };
                println!(
                    "{:>9.0} {:>7} {:>11.3} {:>11.3}",
                    yaw.to_degrees(),
                    at.len(),
                    mean(|point| point.angle),
                    mean(|point| point.height),
                );
            }
        }
    }
    println!(
        "\ntilt is the angle between where the picture says up is and where the estimate does, \n\
         averaged over every view that found a horizon; toward is the bearing it leans, in the \n\
         same frame the camera reads its rays in. angle fit is the amplitude of the sinusoid \n\
         fitted to the horizon's angle against yaw, which is the same defect seen the way a \n\
         pilot sees it, and rms is what that fit leaves behind."
    );
    Ok(())
}

/// One rendered view and what the horizon in it says.
#[derive(Clone, Copy)]
struct Point {
    yaw: f64,
    /// The same view, measured from where the **camera body** is pointing
    /// rather than from where the stabilized frame's own zero is.
    ///
    /// The two are not the same question and issue #45 turned on it. The
    /// frame the camera reads its yaw in is the one the heading follow has
    /// carried, and the follow lags the body by its own time constant times
    /// the turn rate, which on this flying is tens of degrees. So a defect
    /// bolted to the aircraft reads at a phase that wanders with the turning
    /// when it is fitted against `yaw`, and at a fixed one when it is fitted
    /// against this.
    body_yaw: f64,
    /// The horizon's angle in the picture, degrees, from [`skyline`].
    angle: f64,
    /// How far below the middle of the view it sits, in degrees.
    height: f64,
    /// Where the picture says up is, in the frame the camera reads its rays
    /// in, which with the lock on is the stabilized world.
    up: [f64; 3],
}

/// What one variant's sweep came to.
struct Row {
    name: String,
    views: usize,
    tilt: f64,
    toward: f64,
    angle: Option<Wave>,
    height: Option<Wave>,
    /// The same fit, against the yaw measured from the body's own heading.
    body: Option<Wave>,
}

impl Row {
    fn of(name: &str, run: &[Point]) -> Self {
        // The mean of the measured verticals, which is a mean of directions
        // and therefore a sum renormalized rather than an average of angles.
        let mean = run.iter().fold([0.0; 3], |held, point| {
            std::array::from_fn(|axis| held[axis] + point.up[axis])
        });
        let up = unit(mean);
        Self {
            name: name.to_owned(),
            views: run.len(),
            tilt: dot(up, UP).clamp(-1.0, 1.0).acos().to_degrees(),
            // Which way it leans, as a bearing in the same frame: x right and
            // z forward, so this is the azimuth the vertical tips towards.
            toward: match run.is_empty() {
                true => 0.0,
                false => up[0].atan2(up[2]).to_degrees(),
            },
            angle: Wave::of(run.iter().map(|point| (point.yaw, point.angle))),
            height: Wave::of(run.iter().map(|point| (point.yaw, point.height))),
            body: Wave::of(run.iter().map(|point| (point.body_yaw, point.angle))),
        }
    }
}

impl std::fmt::Display for Row {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Some(angle) = &self.angle else {
            return write!(
                f,
                "{:<16} {:>6} no horizon in enough of the circle to fit",
                self.name, self.views
            );
        };
        write!(
            f,
            "{:<16} {:>6} {:>9.3} {:>9.0} {:>13.3} {:>9.0} {:>9.3} {:>11.0} {:>10.3}",
            self.name,
            self.views,
            self.tilt,
            self.toward,
            angle.amplitude,
            angle.phase.to_degrees(),
            angle.residual,
            self.body
                .as_ref()
                .map_or(f64::NAN, |wave| wave.phase.to_degrees()),
            self.height.as_ref().map_or(f64::NAN, |wave| wave.amplitude),
        )
    }
}

/// A least-squares `offset + amplitude * cos(yaw - phase)` through the
/// points, which is the one shape a constant tilt in the vertical can draw.
struct Wave {
    amplitude: f64,
    phase: f64,
    /// What the fit does not explain, root mean square, in the same units.
    residual: f64,
}

impl Wave {
    /// `None` where there are too few points, or where they all sit at too
    /// few yaws for three coefficients to mean anything.
    fn of(points: impl Iterator<Item = (f64, f64)> + Clone) -> Option<Self> {
        let count = points.clone().count();
        if count < 8 {
            return None;
        }
        // Three basis functions, so the normal equations are a 3x3 solved by
        // elimination. Linear in the coefficients even though the answer is
        // read out as an amplitude and a phase, which is the whole reason the
        // fit is written this way round.
        let basis = |yaw: f64| [1.0, yaw.cos(), yaw.sin()];
        let mut normal = [[0.0f64; 4]; 3];
        for (yaw, value) in points.clone() {
            let row = basis(yaw);
            for (index, cell) in normal.iter_mut().enumerate() {
                for (column, term) in row.iter().enumerate() {
                    cell[column] += row[index] * term;
                }
                cell[3] += row[index] * value;
            }
        }
        let solved = solve(normal)?;
        let residual = (points
            .map(|(yaw, value)| {
                let fitted: f64 = basis(yaw)
                    .iter()
                    .zip(&solved)
                    .map(|(term, coefficient)| term * coefficient)
                    .sum();
                (value - fitted).powi(2)
            })
            .sum::<f64>()
            / count as f64)
            .sqrt();
        Some(Self {
            amplitude: solved[1].hypot(solved[2]),
            phase: solved[2].atan2(solved[1]),
            residual,
        })
    }
}

/// Gauss-Jordan on a 3x4 augmented matrix. `None` where the columns are not
/// independent, which here means the sweep did not cover enough of the
/// circle.
fn solve(mut rows: [[f64; 4]; 3]) -> Option<[f64; 3]> {
    for step in 0..3 {
        let pivot =
            (step..3).max_by(|a, b| rows[*a][step].abs().total_cmp(&rows[*b][step].abs()))?;
        rows.swap(step, pivot);
        if rows[step][step].abs() < 1e-9 {
            return None;
        }
        let scale = rows[step][step];
        for cell in rows[step].iter_mut().skip(step) {
            *cell /= scale;
        }
        let pivoted = rows[step];
        for (row, cells) in rows.iter_mut().enumerate() {
            if row == step {
                continue;
            }
            let factor = cells[step];
            for (cell, above) in cells.iter_mut().zip(&pivoted).skip(step) {
                *cell -= factor * above;
            }
        }
    }
    Some([rows[0][3], rows[1][3], rows[2][3]])
}

/// One way of estimating the vertical, measured alongside the others on the
/// same frames and the same yaws.
struct Variant {
    name: String,
    track: OrientationTrack,
    /// A rotation applied to the track's answer so that a candidate which
    /// changes the **lens** mounting as well as the IMU's can be measured
    /// without rebuilding the shader's own copy of it: the pass composes
    /// `lens_from_body * body_from_world`, so pre-multiplying the held
    /// orientation by `candidate_lens^-1 * shipped_lens` renders exactly
    /// what the candidate would have rendered.
    ///
    /// Exact for lens 0. Lens 1 sits in the nominal opposed arrangement,
    /// which does not commute with this rotation, so a variant that carries
    /// one is measured only where the fitted horizon is lens 0's alone.
    rebased: Option<Quat>,
    /// How far either side of straight ahead this variant is measured, in
    /// degrees.
    front: f64,
}

/// How far off the view axis a rebased variant may be measured.
///
/// The horizon is fitted across the middle 70 percent of the picture, so at a
/// 100 degree field of view its far end sits 35 degrees off the axis, and
/// lens 0 has the picture to itself out to about 83. What is left is this.
const REBASED_FRONT: f64 = 45.0;

impl Variant {
    fn held(&self, at: i64) -> Quat {
        match self.rebased {
            Some(rebase) => self.track.at(at).times(rebase),
            None => self.track.at(at),
        }
    }

    /// Whether this variant's answer is exact at this yaw.
    fn reaches(&self, yaw: f64) -> bool {
        wrap(yaw).to_degrees().abs() <= self.front
    }
}

struct Options {
    input: PathBuf,
    from: f64,
    count: usize,
    steps: usize,
    pitch: f32,
    fov: f32,
    size: Size,
    /// A known tilt added to every held orientation, in radians, about the
    /// horizontal world axis at bearing `about`. The positive control for a
    /// defect that lives in the **world**.
    inject: f64,
    about: f64,
    /// A known roll added about the **body's own** optical axis, in radians.
    /// The positive control for a defect that lives in the **view**: it has
    /// to come back pinned to view yaw 0 and 180 whatever the flying is
    /// doing, where `inject` comes back pinned to the world.
    roll: f64,
    /// Print the horizon angle and height at every yaw, averaged over the
    /// frames, instead of only the fit through them.
    dump: bool,
    /// Print the stretches where the estimated vertical is furthest from the
    /// accelerometer's, instead of rendering anything.
    worst: Option<usize>,
    /// Print how far apart the two are over the first minutes of the file,
    /// which is where an estimate that started wrong shows.
    settle: bool,
    /// How far from level a found line may be before it is taken to be
    /// something other than the horizon, in degrees.
    ///
    /// Off by default: this gate was set to 20 and it discarded the whole of
    /// issue #45, which is 40 to 50 degrees over the first seconds of a file.
    /// A run that re-imposes one is asking a narrower question and the header
    /// line says how many views it cost.
    limit: f64,
    /// How far either side of straight ahead the sweep is measured. The whole
    /// circle by default; a smaller arc puts every variant on the terms a
    /// rebased one is stuck with, which is what makes them comparable.
    front: f64,
    png: Option<usize>,
    only: Option<String>,
    /// `gyro_calib`'s six doubles, to be tested as a bias.
    bias: Option<[f64; 6]>,
    /// Filter settings to compare against the shipped ones, each of them an
    /// `accel_seconds`, a `tilt_seconds` and the two edges of the trust
    /// window.
    filters: Vec<[f64; 4]>,
}

impl Options {
    fn at(&self) -> Cue {
        Cue::Time(std::time::Duration::from_secs_f64(self.from))
    }

    /// The held orientation with the positive control's tilt in it. Applied
    /// on the left, which is a rotation of the estimated world rather than of
    /// the body: that is what a wrong vertical is.
    fn injected(&self, held: Quat) -> Quat {
        // On the left is a rotation of the estimated world, which is what a
        // wrong vertical is; on the right is a rotation of the body, which is
        // what a wrong mounting is. The two are told apart by exactly the
        // question issue #45 turned into: whether the dip stays where the
        // pilot is looking or where the aircraft is pointing.
        let (sin, cos) = self.about.sin_cos();
        let tilted = match self.inject == 0.0 {
            true => held,
            false => {
                Quat::from_rotation_vector([cos * self.inject, 0.0, sin * self.inject]).times(held)
            }
        };
        match self.roll == 0.0 {
            true => tilted,
            false => tilted.times(Quat::from_rotation_vector([0.0, 0.0, -self.roll])),
        }
    }

    /// The ways of estimating the vertical that get compared.
    ///
    /// The shipped one; the same thing routed through the rebasing path,
    /// which must come back identical or the rebasing proves nothing; the six
    /// orders the three mounting angles can be composed in (4.8's last open
    /// question); the mounting dropped and applied unturned; the `gyro_calib`
    /// doubles as a bias, both ways round; and two shorter tilt constants,
    /// which is what would shrink a bias the filter is settling against.
    fn variants(&self, calibration: &CalibrationSet) -> Vec<Variant> {
        let shipped = Filter::default();
        let pose = &calibration.lenses[0].pose;
        let axes = axis_map(calibration.gyro.imu_orientation);
        let held = |name: &str, track| Variant {
            name: name.to_owned(),
            track,
            rebased: None,
            front: self.front,
        };
        let mut variants = vec![held("shipped", calibration.orientation(shipped))];
        for order in Order::ALL {
            variants.push(Variant {
                name: format!("order-{}", order.name()),
                track: shipped.solve(
                    &calibration.imu,
                    order.compose(pose, 0.0).transpose().times(axes),
                ),
                // The shipped composition is `zyx`, and rebasing it against
                // itself is the identity: that variant is the check that this
                // path renders what it claims to.
                rebased: Some(
                    order
                        .quat(pose, ROLL_DATUM_DEG)
                        .conjugate()
                        .times(Order::Zyx.quat(pose, ROLL_DATUM_DEG)),
                ),
                front: self.front.min(REBASED_FRONT),
            });
        }
        variants.push(held("mount-none", shipped.solve(&calibration.imu, axes)));
        variants.push(held(
            "mount-unturned",
            shipped.solve(&calibration.imu, Order::Zyx.compose(pose, 0.0).times(axes)),
        ));
        if let Some(bias) = self.bias {
            let first = [bias[0], bias[1], bias[2]];
            let second = [bias[3], bias[4], bias[5]];
            // The gyro triple is tried as radians a second, which is the unit
            // the scaled encoding writes its rates in; at degrees a second
            // these numbers would be a fortieth of the drift the filter
            // already holds and could not matter.
            for (name, gyro, accel) in [
                ("bias-gyro-first", first, second),
                ("bias-accel-first", second, first),
            ] {
                variants.push(held(
                    name,
                    shipped.solve(
                        &debiased(&calibration.imu, gyro.map(f64::to_degrees), accel),
                        calibration.body_from_imu(),
                    ),
                ));
            }
        }
        // Filter settings named on the command line, because the shape of
        // this search is a question about flying and not about code: the
        // accelerometer's own smoothing, how fast it is believed, and how far
        // a coordinated turn has to be banked before it is not. The specific
        // force in a level turn is `1 / cos(bank)`, so the shipped 0.05 g
        // window is a bank of 18 degrees, and everything trusted leans the
        // estimated vertical into the turn.
        for [accel_seconds, tilt_seconds, full, none] in &self.filters {
            variants.push(held(
                &format!("filter-{accel_seconds}-{tilt_seconds}-{full}-{none}"),
                calibration.orientation(Filter {
                    accel_seconds: *accel_seconds,
                    tilt_seconds: *tilt_seconds,
                    trust_g: (*full, *none),
                    ..shipped
                }),
            ));
        }
        match &self.only {
            // By prefix and by list, so that one run can compare the shipped
            // answer against a sweep of one kind of candidate.
            Some(only) => variants
                .into_iter()
                .filter(|variant| only.split(',').any(|want| variant.name.starts_with(want)))
                .collect(),
            None => variants,
        }
    }

    fn parse(mut args: impl Iterator<Item = String>) -> Fallible<Self> {
        let input = PathBuf::from(args.next().ok_or(USAGE)?);
        let mut options = Self {
            input,
            from: 0.0,
            count: 12,
            steps: 24,
            pitch: 0.0,
            fov: 100f32.to_radians(),
            size: Size::new(960, 540),
            inject: 0.0,
            about: 0.0,
            roll: 0.0,
            dump: false,
            worst: None,
            settle: false,
            limit: NO_LIMIT,
            front: 180.0,
            png: None,
            only: None,
            bias: None,
            filters: Vec::new(),
        };
        for arg in args {
            let (key, value) = arg.split_once('=').ok_or(USAGE)?;
            match key {
                "from" => options.from = value.parse()?,
                "count" => options.count = value.parse()?,
                "steps" => options.steps = value.parse()?,
                "pitch" => options.pitch = value.parse::<f32>()?.to_radians(),
                "fov" => options.fov = value.parse::<f32>()?.to_radians(),
                "width" => options.size.width = value.parse()?,
                "height" => options.size.height = value.parse()?,
                "inject" => options.inject = value.parse::<f64>()?.to_radians(),
                "about" => options.about = value.parse::<f64>()?.to_radians(),
                "roll" => options.roll = value.parse::<f64>()?.to_radians(),
                "dump" => options.dump = value.parse::<u32>()? != 0,
                "worst" => options.worst = Some(value.parse()?),
                "settle" => options.settle = value.parse::<u32>()? != 0,
                "limit" => options.limit = value.parse()?,
                "front" => options.front = value.parse()?,
                "png" => options.png = Some(value.parse()?),
                "only" => options.only = Some(value.to_owned()),
                "bias" => options.bias = Some(six(value)?),
                "filter" => options.filters.push(numbers(value)?),
                _ => return Err(format!("unknown argument {key}. {USAGE}").into()),
            }
        }
        Ok(options)
    }
}

/// The quarter turn between `offset_v3`'s roll and the delivered frame's own
/// vertical, which `kyerag_meta::Pose` carries and does not hand out.
const ROLL_DATUM_DEG: f64 = -90.0;

const USAGE: &str = "usage: dip <file.insv> [from=seconds] [count=frames] [steps=yaws] \
     [pitch=deg] [fov=deg] [width=px] [height=px] [inject=deg] [about=deg] [limit=deg] \
     [front=deg] [png=every] [only=prefix,prefix] [bias=a_b_c_d_e_f] \
     [filter=accel_tilt_full_none]";

fn six(value: &str) -> Fallible<[f64; 6]> {
    numbers(value).map_err(|_| "bias wants six numbers separated by underscores".into())
}

/// The underscore-separated numbers of one argument, as the fixed-size array
/// its reader wants.
fn numbers<const N: usize>(value: &str) -> Fallible<[f64; N]> {
    let read: Vec<f64> = value
        .split('_')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| format!("{value} is not {N} numbers separated by underscores"))?;
    read.try_into()
        .map_err(|_| format!("{value} is not {N} numbers").into())
}

/// How far the estimated vertical is from the accelerometer's over the first
/// minutes of the file, and what the filter had to start from.
///
/// **No horizon finder and no gate**, which is the point: the instrument that
/// reads pixels can only report a dip it can still find a horizon inside, so
/// it goes quiet exactly where the defect is worst. This one reports every
/// window there is.
///
/// The first line is the whole of the initialisation, read out of the filter
/// itself rather than worked out again here: [`Filter::seed`] says where it
/// started the estimate, what that window weighed, and whether it is a reading
/// the running filter believes at all. Before the fix for issue #45 it was
/// always the first tenth of a second whatever it read, which on the April 10
/// capture is 1.281 g.
fn settling(calibration: &CalibrationSet) -> Fallible<()> {
    let filter = Filter::default();
    let track = calibration.orientation(filter);
    let to_body = calibration.body_from_imu();
    let samples = calibration.imu.samples();
    let rate = calibration.imu.rate_hz();

    match filter.seed(&calibration.imu, to_body) {
        Some(seed) => println!(
            "init:   the estimate starts from {:.1} s in, where {:.1} s of accelerometer \n\
             \tweighs {:.3} g. The running filter believes a reading completely only inside \n\
             \t{:.2} g of 1 g and not at all outside {:.2}, so this is a seed it {}.",
            seed.at_us as f64 * 1e-6,
            filter.accel_seconds,
            seed.magnitude_g,
            filter.trust_g.0,
            filter.trust_g.1,
            match seed.trusted {
                true => "believes",
                false => "would refuse, and no window in the search read any closer to gravity",
            },
        ),
        None => println!("init:   no IMU samples, so there is nothing to start from"),
    }

    println!("\n{:>9} {:>16} {:>10}", "seconds", "estimate vs g", "|a| g");
    let window = (2.0 * rate) as usize;
    for step in 0..46 {
        let at_s = match step {
            0..=20 => step as f64 * 2.0,
            _ => 40.0 + (step - 20) as f64 * 20.0,
        };
        let from = samples.partition_point(|s| (s.offset_us as f64 * 1e-6) < at_s);
        let to = (from + window).min(samples.len());
        if from >= to {
            break;
        }
        let mean = samples[from..to]
            .iter()
            .step_by(11)
            .fold([0.0; 3], |held, sample| {
                let body = to_body.mul_vec(sample.accel_g);
                std::array::from_fn(|axis| held[axis] + body[axis])
            });
        let at = samples[(from + to) / 2].offset_us;
        let up = track.at(at).rotate(unit(mean));
        println!(
            "{at_s:>9.0} {:>16.1} {:>10.3}",
            dot(up, UP).clamp(-1.0, 1.0).acos().to_degrees(),
            norm(mean) / (to - from).div_ceil(11) as f64,
        );
    }
    println!(
        "\nestimate vs g is the angle between where the filter says up is and where the 
         two seconds of accelerometer around that instant say it is. |a| is what that 
         reading weighs: near 1 g it is gravity and the disagreement is the estimate's, 
         far from it the aircraft is doing something and neither is."
    );
    Ok(())
}

/// Where in the file the estimated vertical is furthest from the one the
/// accelerometer names, in degrees, worst stretches first.
///
/// No pixels and no GPU: this is the cheap search that says where to point
/// the expensive instrument. Over ten seconds of flying the mean specific
/// force is gravity to within the turning, so a **sustained** disagreement
/// of tens of degrees is the estimate being wrong rather than the aircraft
/// accelerating. It is a proxy and not a measurement: what it is for is
/// choosing a `from=`.
fn worst(calibration: &CalibrationSet, count: usize) -> Fallible<()> {
    const WINDOW_S: f64 = 10.0;
    let track = calibration.orientation(Filter::default());
    let to_body = calibration.body_from_imu();
    let samples = calibration.imu.samples();
    let width = (WINDOW_S * calibration.imu.rate_hz()) as usize;
    if width == 0 || samples.len() < width {
        return Err("the IMU track is too short to look through".into());
    }

    let mut windows: Vec<(f64, f64)> = samples
        .chunks_exact(width)
        .map(|window| {
            // The mean reading of the window, in the body's frame, turned
            // into the world by the estimate that window was solved to.
            let mean = window.iter().step_by(11).fold([0.0; 3], |held, sample| {
                let body = to_body.mul_vec(sample.accel_g);
                std::array::from_fn(|axis| held[axis] + body[axis])
            });
            let at = window[window.len() / 2].offset_us;
            let up = track.at(at).rotate(unit(mean));
            (
                dot(up, UP).clamp(-1.0, 1.0).acos().to_degrees(),
                window[0].offset_us as f64 * 1e-6,
            )
        })
        .collect();
    windows.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!(
        "the {count} stretches where the estimated vertical is furthest from the \n\
         accelerometer's, over {WINDOW_S:.0} s windows:"
    );
    for (off, at) in windows.iter().take(count) {
        println!("  from={at:<8.1} {off:6.1} deg apart");
    }
    let mean = windows.iter().map(|(off, _)| off).sum::<f64>() / windows.len() as f64;
    println!("  {:.1} deg apart on average over the whole file", mean);
    Ok(())
}

/// The IMU track with a constant taken off both triples.
fn debiased(track: &GyroTrack, rate_dps: [f64; 3], accel_g: [f64; 3]) -> GyroTrack {
    GyroTrack::from_samples(
        track
            .samples()
            .iter()
            .map(|sample| GyroSample {
                offset_us: sample.offset_us,
                rate_dps: std::array::from_fn(|axis| sample.rate_dps[axis] - rate_dps[axis]),
                accel_g: std::array::from_fn(|axis| sample.accel_g[axis] - accel_g[axis]),
            })
            .collect(),
    )
}

/// The order the three mounting angles are composed in, named by the axes
/// left to right: `Zyx` is `Rz(roll) Ry(yaw) Rx(pitch)`, which is what
/// `kyerag_meta::Pose` ships (docs/research/insv-format.md 4.8).
#[derive(Clone, Copy)]
enum Order {
    Zyx,
    Zxy,
    Yzx,
    Yxz,
    Xzy,
    Xyz,
}

impl Order {
    const ALL: [Self; 6] = [
        Self::Zyx,
        Self::Zxy,
        Self::Yzx,
        Self::Yxz,
        Self::Xzy,
        Self::Xyz,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Zyx => "zyx",
            Self::Zxy => "zxy",
            Self::Yzx => "yzx",
            Self::Yxz => "yxz",
            Self::Xzy => "xzy",
            Self::Xyz => "xyz",
        }
    }

    /// The three rotations in this order, left to right.
    fn axes(self) -> [usize; 3] {
        match self {
            Self::Zyx => [2, 1, 0],
            Self::Zxy => [2, 0, 1],
            Self::Yzx => [1, 2, 0],
            Self::Yxz => [1, 0, 2],
            Self::Xzy => [0, 2, 1],
            Self::Xyz => [0, 1, 2],
        }
    }

    /// Which angle turns about which axis, with `datum` added to the roll:
    /// zero is the sensor's own frame and -90 is the delivered picture's
    /// (`Pose::sensor_from_body` and `Pose::lens_from_body`).
    fn turns(self, pose: &Pose, datum: f64) -> [(usize, f64); 3] {
        self.axes().map(|axis| {
            (
                axis,
                match axis {
                    0 => pose.pitch_deg.to_radians(),
                    1 => pose.yaw_deg.to_radians(),
                    _ => (pose.roll_deg + datum).to_radians(),
                },
            )
        })
    }

    /// This mounting as a matrix, which is what the IMU path composes with.
    fn compose(self, pose: &Pose, datum: f64) -> Mat3 {
        self.turns(pose, datum)
            .into_iter()
            .map(|(axis, angle)| match axis {
                0 => Mat3::rot_x(angle),
                1 => Mat3::rot_y(angle),
                _ => Mat3::rot_z(angle),
            })
            .fold(Mat3::IDENTITY, Mat3::times)
    }

    /// The same rotation as a quaternion, which is what the rebasing needs.
    fn quat(self, pose: &Pose, datum: f64) -> Quat {
        self.turns(pose, datum)
            .into_iter()
            .map(|(axis, angle)| {
                let mut vector = [0.0; 3];
                vector[axis] = angle;
                Quat::from_rotation_vector(vector)
            })
            .fold(Quat::IDENTITY, Quat::times)
    }
}

/// An angle wrapped into (-pi, pi].
fn wrap(angle: f64) -> f64 {
    use std::f64::consts::PI;
    (angle + PI).rem_euclid(TAU) - PI
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

/// The length of a vector.
fn norm(v: [f64; 3]) -> f64 {
    dot(v, v).sqrt()
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let length = dot(v, v).sqrt();
    v.map(|c| c / length.max(f64::MIN_POSITIVE))
}
