//! Which arc of the seam ring a world-fixed view is looking at, frame by
//! frame, while the aircraft moves under it.
//!
//! ```sh
//! cargo run --release -p kjerag-spike --bin arcs -- <file.insv> \
//!   time=65.666 yaw=179.00 pitch=-36.97 fov=20.00 lock=1 window=5
//! ```
//!
//! The seam is camera-fixed and a `lock=1` view is world-fixed, so which
//! azimuths of the ring fall inside a named view is a fact about the
//! aircraft's attitude at one instant and not about the view line. Every other
//! seam instrument in this crate answers at one instant
//! (`--bin crossing`), which is enough while the crossing is being measured
//! and not enough to say what an eye watching for five seconds was shown.
//!
//! Nothing is decoded. The orientation track comes out of the trailer and the
//! map is [`Reframe::new`] over the same lenses, camera and [`Held`] the pass
//! builds, so the view rays and the body rays are the app's own; the frame
//! instants are the exposure record's, the way `--bin carried` reads them.
//!
//! What it prints per frame: how far the view centre sits off the seam plane,
//! whether the handover corridor is in the view at all, and the arc of ring
//! azimuths inside it. Azimuth is the band's own: radians from the body's +x
//! through the body's +y, reported in degrees, so a number here is a `Cell`
//! index times 360/128 (`kjerag_render::band::Ring`).
//!
//! It also prints where the world's own vertical sits on that ring, because
//! the only near content in a flight is what hangs under the camera and the
//! question of whether a direction is looking at it is answered by the IMU and
//! not by a guess.

use std::path::PathBuf;
use std::time::Duration;

use kjerag_media::{Cue, Fallible, Reader, Size};
use kjerag_meta::{CalibrationSet, Filter, OrientationTrack, Quat};
use kjerag_render::{Camera, Held, Reframe, Sampling};

/// How finely the view is sampled, per side. Odd, so the centre pixel is a
/// sample and the reported centre elevation is a reading rather than an
/// interpolation.
const GRID: usize = 129;

/// How finely the arc is binned before its ends are read off, in degrees. One
/// degree is a third of a `Cell`, so an arc reported here names the cells it
/// covers with no rounding of its own worth arguing about.
const ARC_BIN_DEG: f64 = 1.0;

/// How thick a marked line is drawn, in output pixels either side, so that the
/// same mark reads the same at any field of view.
const LINE_PX: f64 = 1.5;

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let calibration = CalibrationSet::from_insv(&options.input)?;
    if calibration.imu.is_empty() {
        return Err("this file carries no IMU record, so a lock=1 view has no frame".into());
    }
    let timing = Reader::open(&options.input)?.timing();
    let track = calibration.orientation(Filter::default());
    let frame = Size::new(calibration.dimension.width, calibration.dimension.height);
    let camera = options.camera();
    let half_deg = f64::from(
        Reframe::new(
            &calibration.lenses,
            frame,
            camera,
            Held::default(),
            options.aspect,
            false,
            Sampling::default(),
        )
        .crossover_at(0.0)
        .to_degrees(),
    ) / 2.0;

    println!(
        "view:   {} time {:.3} yaw {:.2} pitch {:.2} fov {:.2} lock {}",
        options
            .input
            .file_name()
            .unwrap_or_default()
            .to_string_lossy(),
        options.time,
        options.yaw,
        options.pitch,
        options.fov,
        u8::from(options.lock),
    );
    println!(
        "seam:   the handover is {:.2} deg wide, so the corridor is {half_deg:+.2} deg either \
         side of the ring",
        half_deg * 2.0,
    );
    println!(
        "window: {:+.1} to {:+.1} s round the line, {GRID}x{GRID} rays per frame at aspect {:.2}",
        -options.window, options.window, options.aspect,
    );

    let rows = sweep(&options, &calibration, &track, timing, frame, half_deg)?;
    report(&rows);
    if let Some(name) = &options.out {
        write(&options, &rows, half_deg, name)?;
    }
    if let Some((from, onto)) = options.mark.as_ref().zip(options.marked.as_ref()) {
        mark(
            &options,
            &calibration,
            &track,
            timing,
            frame,
            half_deg,
            from,
            onto,
        )?;
    }
    Ok(())
}

/// One frame's answer.
struct Row {
    index: u64,
    seconds: f64,
    /// Degrees the view centre sits off the seam plane, signed the way
    /// `Field::of` signs it: positive is toward the body's +z, lens 0's axis.
    centre_deg: f64,
    /// The arc of ring azimuths inside both the view and the corridor.
    arc: Option<(f64, f64)>,
    /// How many of the sampled rays landed inside the corridor.
    inside: usize,
    /// Where the world's own down sits on the ring, and how far off the seam
    /// plane it is.
    down_phi: f64,
    down_deg: f64,
}

/// Every frame in the window, read off the orientation track.
fn sweep(
    options: &Options,
    calibration: &CalibrationSet,
    track: &OrientationTrack,
    timing: kjerag_media::Timing,
    frame: Size,
    half_deg: f64,
) -> Fallible<Vec<Row>> {
    let at = |seconds: f64| Cue::Time(Duration::from_secs_f64(seconds.max(0.0))).index(timing);
    let first = at(options.time - options.window);
    let last = at(options.time + options.window);
    let mut rows = Vec::new();
    for index in first..=last {
        let Some(at_us) = calibration.exposure[0].frame_time_us(index) else {
            break;
        };
        let held = Held {
            body_from_world: match options.lock {
                true => track.at(at_us).conjugate(),
                false => Quat::IDENTITY,
            },
            rolling: None,
        };
        let map = Reframe::new(
            &calibration.lenses,
            frame,
            options.camera(),
            held,
            options.aspect,
            false,
            Sampling::default(),
        );
        let down = held.body_from_world.rotate([0.0, 1.0, 0.0]);
        rows.push(Row {
            index,
            seconds: at_us as f64 * 1e-6,
            centre_deg: centre_off(&map),
            arc: arc(&map, half_deg),
            inside: counted(&map, half_deg),
            down_phi: azimuth(down),
            down_deg: elevation(down),
        });
    }
    match rows.is_empty() {
        true => Err("that window holds no frame of this file's exposure record".into()),
        false => Ok(rows),
    }
}

/// The corridor drawn onto a rendered view of the same line.
///
/// A picture of `--bin reframe`'s, marked rather than re-rendered: this
/// instrument decodes nothing, and a second renderer here would be a second
/// answer to what the pass draws. The two agree because they build the same
/// [`Reframe`] from the same camera and the same frame instant, which is the
/// one thing that has to hold and is asserted by the seam landing where the
/// two lenses visibly hand over.
#[allow(clippy::too_many_arguments)]
fn mark(
    options: &Options,
    calibration: &CalibrationSet,
    track: &OrientationTrack,
    timing: kjerag_media::Timing,
    frame: Size,
    half_deg: f64,
    from: &str,
    onto: &str,
) -> Fallible<()> {
    let (mut pixels, width, height, channels) = read_png(&PathBuf::from("scratch").join(from))?;
    let index = Cue::Time(Duration::from_secs_f64(options.time.max(0.0))).index(timing);
    let at_us = calibration.exposure[0]
        .frame_time_us(index)
        .ok_or("that time is past the end of this file's exposure record")?;
    let map = Reframe::new(
        &calibration.lenses,
        frame,
        options.camera(),
        Held {
            body_from_world: match options.lock {
                true => track.at(at_us).conjugate(),
                false => Quat::IDENTITY,
            },
            rolling: None,
        },
        width as f32 / height as f32,
        false,
        Sampling::default(),
    );
    let past: Vec<f64> = (0..width * height)
        .map(|cell| {
            let uv = [
                (cell % width) as f32 / width as f32,
                (cell / width) as f32 / height as f32,
            ];
            map.view_ray(uv).map_or(f64::INFINITY, |ray| {
                elevation(map.body_ray(ray).map(f64::from))
            })
        })
        .collect();
    // A line of constant PIXEL thickness rather than of constant angle, so
    // the same mark reads the same at fov 20 and at fov 90. Painting where
    // the field is within half a line of an edge, rather than where it
    // changes sign between neighbours, is what stops a near-horizontal line
    // coming out as scattered dots.
    let thick = LINE_PX * f64::from(options.fov as f32) / width as f64;
    for (cell, off) in past.iter().enumerate() {
        if !off.is_finite() {
            continue;
        }
        for (edge, colour) in [
            (0.0, [255u8, 40, 40]),
            (half_deg, [255, 170, 0]),
            (-half_deg, [255, 170, 0]),
        ] {
            if (off - edge).abs() <= thick {
                let at = cell * channels;
                pixels[at..at + 3].copy_from_slice(&colour);
            }
        }
    }
    let out = PathBuf::from("scratch").join(onto);
    let mut png = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(&out)?),
        width as u32,
        height as u32,
    );
    png.set_color(match channels {
        4 => png::ColorType::Rgba,
        _ => png::ColorType::Rgb,
    });
    png.set_depth(png::BitDepth::Eight);
    png.write_header()?.write_image_data(&pixels)?;
    println!("marked {} onto {}", from, out.display());
    Ok(())
}

/// The rendered view to mark, as bytes, its shape, and how many channels a
/// pixel is.
fn read_png(path: &std::path::Path) -> Fallible<(Vec<u8>, usize, usize, usize)> {
    let decoder = png::Decoder::new(std::fs::File::open(path)?);
    let mut reader = decoder.read_info()?;
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels)?;
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        other => return Err(format!("{} is {other:?}, not RGB or RGBA", path.display()).into()),
    };
    pixels.truncate(info.buffer_size());
    Ok((pixels, info.width as usize, info.height as usize, channels))
}

/// Degrees off the seam plane at the middle of the picture.
fn centre_off(map: &Reframe) -> f64 {
    map.view_ray([0.5, 0.5])
        .map_or(f64::NAN, |ray| elevation(map.body_ray(ray).map(f64::from)))
}

/// How many of the sampled rays are inside the corridor.
fn counted(map: &Reframe, half_deg: f64) -> usize {
    rays(map)
        .filter(|ray| elevation(*ray).abs() <= half_deg)
        .count()
}

/// The arc of azimuths inside both the view and the corridor, as the two ends
/// of the widest run of them.
///
/// Read off a one-degree histogram by its **widest gap** rather than by its
/// smallest and largest member, which is `--bin crossing`'s rule
/// (`crossings`) and is the only one that survives an arc straddling the back
/// of the circle at +/-180.
fn arc(map: &Reframe, half_deg: f64) -> Option<(f64, f64)> {
    let bins = (360.0 / ARC_BIN_DEG) as usize;
    let mut seen = vec![false; bins];
    let mut any = false;
    for ray in rays(map) {
        if elevation(ray).abs() > half_deg {
            continue;
        }
        let bin = (azimuth(ray).rem_euclid(360.0) / ARC_BIN_DEG) as usize;
        seen[bin.min(bins - 1)] = true;
        any = true;
    }
    if !any {
        return None;
    }
    let filled: Vec<usize> = (0..bins).filter(|bin| seen[*bin]).collect();
    // The widest gap between neighbours, round the circle. The arc is what is
    // left when that gap is cut out.
    let (mut cut, mut widest) = (0, 0);
    for (place, bin) in filled.iter().enumerate() {
        let next = filled[(place + 1) % filled.len()];
        let gap = (next + bins - bin) % bins;
        let gap = match gap {
            0 => bins,
            gap => gap,
        };
        if gap > widest {
            (cut, widest) = (place, gap);
        }
    }
    let start = filled[(cut + 1) % filled.len()] as f64 * ARC_BIN_DEG;
    let end = (filled[cut] + 1) as f64 * ARC_BIN_DEG;
    Some((wrapped(start), wrapped(end)))
}

/// Every sampled view ray as a body direction.
fn rays(map: &Reframe) -> impl Iterator<Item = [f64; 3]> + '_ {
    (0..GRID * GRID).filter_map(move |cell| {
        let uv = [
            (cell % GRID) as f32 / (GRID - 1) as f32,
            (cell / GRID) as f32 / (GRID - 1) as f32,
        ];
        let ray = map.view_ray(uv)?;
        Some(map.body_ray(ray).map(f64::from))
    })
}

/// Degrees off the seam plane, which is the body's `z = 0` great circle.
fn elevation(body: [f64; 3]) -> f64 {
    let length = (body[0] * body[0] + body[1] * body[1] + body[2] * body[2]).sqrt();
    match length > 0.0 {
        true => (body[2] / length).asin().to_degrees(),
        false => f64::NAN,
    }
}

/// The band's own azimuth: degrees from the body's +x through its +y
/// (`kjerag_render::band::Ring::at`).
fn azimuth(body: [f64; 3]) -> f64 {
    wrapped(body[1].atan2(body[0]).to_degrees())
}

/// Degrees wrapped into (-180, 180].
fn wrapped(degrees: f64) -> f64 {
    let wrapped = (degrees + 180.0).rem_euclid(360.0) - 180.0;
    match wrapped == -180.0 {
        true => 180.0,
        false => wrapped,
    }
}

fn report(rows: &[Row]) {
    println!(
        "\n{:>8}{:>10}{:>12}{:>10}{:>10}{:>9}{:>12}{:>11}",
        "frame", "time s", "centre deg", "arc from", "arc to", "rays", "down phi", "down deg",
    );
    for row in rows {
        let (from, to) = match row.arc {
            Some((from, to)) => (format!("{from:9.1}"), format!("{to:9.1}")),
            None => ("        -".into(), "        -".into()),
        };
        println!(
            "{:>8}{:>10.3}{:>12.2}{from:>10}{to:>10}{:>9}{:>12.1}{:>11.2}",
            row.index, row.seconds, row.centre_deg, row.inside, row.down_phi, row.down_deg,
        );
    }
    let showing = rows.iter().filter(|row| row.arc.is_some()).count();
    println!(
        "\ncorridor: in the view on {showing} of {} frames.",
        rows.len(),
    );
    println!(
        "centre:   {:.2} to {:.2} deg off the seam plane over the window.",
        rows.iter()
            .map(|row| row.centre_deg)
            .fold(f64::MAX, f64::min),
        rows.iter()
            .map(|row| row.centre_deg)
            .fold(f64::MIN, f64::max),
    );
    println!(
        "down:     the world's vertical sits {:.2} to {:.2} deg off the seam plane, at azimuth \
         {:.1} to {:.1}.",
        rows.iter().map(|row| row.down_deg).fold(f64::MAX, f64::min),
        rows.iter().map(|row| row.down_deg).fold(f64::MIN, f64::max),
        rows.iter().map(|row| row.down_phi).fold(f64::MAX, f64::min),
        rows.iter().map(|row| row.down_phi).fold(f64::MIN, f64::max),
    );
}

fn write(options: &Options, rows: &[Row], half_deg: f64, name: &str) -> Fallible<()> {
    let out = PathBuf::from("scratch").join(name);
    std::fs::create_dir_all("scratch")?;
    let mut text = format!(
        "# instrument: kjerag-spike --bin arcs\n\
         # source: {}\n\
         # args: {}\n\
         # corridor: +/-{half_deg:.2} deg off the seam plane, the drawn handover's own half \
         width (Reframe::crossover_at)\n\
         # reduction: {GRID}x{GRID} view rays per frame through Reframe::view_ray and \
         Reframe::body_ray; the arc is the widest run of filled {ARC_BIN_DEG:.0}-degree bins, \
         cut at its widest gap\n\
         # azimuth: degrees from the body's +x through its +y, so cell = azimuth * 128 / 360 \
         (kjerag_render::band::Ring)\n\
         # nothing is decoded: the orientation is the trailer's track and the instants are the \
         exposure record's\n\
         frame,time_s,centre_deg,arc_from_deg,arc_to_deg,rays_inside,down_phi_deg,down_deg\n",
        std::fs::canonicalize(&options.input)
            .unwrap_or_else(|_| options.input.clone())
            .display(),
        options.args,
    );
    for row in rows {
        let (from, to) = row.arc.map_or((f64::NAN, f64::NAN), |arc| arc);
        text.push_str(&format!(
            "{},{:.3},{:.4},{:.2},{:.2},{},{:.2},{:.4}\n",
            row.index,
            row.seconds,
            row.centre_deg,
            from,
            to,
            row.inside,
            row.down_phi,
            row.down_deg,
        ));
    }
    std::fs::write(&out, text)?;
    println!("wrote {}", out.display());
    Ok(())
}

// ------------------------------------------------------------ the arguments

struct Options {
    input: PathBuf,
    time: f64,
    yaw: f64,
    pitch: f64,
    fov: f64,
    lock: bool,
    window: f64,
    aspect: f32,
    /// The whole command line, so a file this writes says what wrote it.
    args: String,
    out: Option<String>,
    /// A rendered view of the same line to draw the corridor onto, and where
    /// to put the result.
    mark: Option<String>,
    marked: Option<String>,
}

impl Options {
    fn parse(args: impl Iterator<Item = String>) -> Fallible<Self> {
        let mut options = Self {
            input: PathBuf::new(),
            time: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            fov: 60.0,
            lock: true,
            window: 5.0,
            aspect: 1.0,
            args: String::new(),
            out: None,
            mark: None,
            marked: None,
        };
        let args: Vec<String> = args.collect();
        options.args = args.join(" ");
        for arg in args {
            match arg.split_once('=') {
                None => options.input = PathBuf::from(arg),
                Some(("time", value)) => options.time = value.parse()?,
                Some(("yaw", value)) => options.yaw = value.parse()?,
                Some(("pitch", value)) => options.pitch = value.parse()?,
                Some(("fov", value)) => options.fov = value.parse()?,
                Some(("lock", value)) => options.lock = value.parse::<i64>()? != 0,
                Some(("window", value)) => options.window = value.parse()?,
                Some(("aspect", value)) => options.aspect = value.parse()?,
                Some(("out", value)) => options.out = Some(value.to_string()),
                Some(("mark", value)) => options.mark = Some(value.to_string()),
                Some(("marked", value)) => options.marked = Some(value.to_string()),
                Some((key, _)) => return Err(format!("no argument called {key}").into()),
            }
        }
        match options.input.as_os_str().is_empty() {
            true => Err(USAGE.into()),
            false => Ok(options),
        }
    }

    fn camera(&self) -> Camera {
        Camera {
            yaw: self.yaw.to_radians() as f32,
            pitch: self.pitch.to_radians() as f32,
            fov: self.fov.to_radians() as f32,
        }
    }
}

const USAGE: &str = "usage: arcs <file.insv> [time=seconds] [yaw=deg] [pitch=deg] [fov=deg] \
     [lock=0|1] [window=seconds] [aspect=ratio] [out=name.csv] [mark=view.png marked=name.png]";

#[cfg(test)]
mod tests {
    use super::*;

    /// The azimuth convention is the band's, asserted against `Ring::of`
    /// rather than against a copy of its arithmetic: a direction on the ring
    /// at a named azimuth reads that azimuth back.
    #[test]
    fn azimuth_is_the_bands_own() {
        for degrees in [-179.0_f64, -90.0, -0.5, 0.0, 37.5, 90.0, 179.0] {
            let ring = kjerag_render::Ring::of(degrees.to_radians() as f32, [0.0, 0.0, -0.033]);
            let read = azimuth(ring.centre.map(f64::from));
            assert!(
                (read - degrees).abs() < 1e-3,
                "{degrees} read back as {read}"
            );
        }
    }

    /// A direction on the ring is on the seam plane, and one off it is off it
    /// by the angle it was tilted by.
    #[test]
    fn elevation_is_degrees_off_the_seam_plane() {
        assert!(elevation([1.0, 0.0, 0.0]).abs() < 1e-9);
        assert!(elevation([0.0, 1.0, 0.0]).abs() < 1e-9);
        assert!((elevation([0.0, 0.0, 1.0]) - 90.0).abs() < 1e-9);
        let tilted = [
            30.0_f64.to_radians().cos(),
            0.0,
            30.0_f64.to_radians().sin(),
        ];
        assert!((elevation(tilted) - 30.0).abs() < 1e-9);
    }

    /// An arc straddling the back of the circle reads as one arc and not as
    /// the whole of it, which is what a smallest-and-largest rule would say.
    #[test]
    fn an_arc_over_the_back_of_the_circle_is_one_arc() {
        assert_eq!(wrapped(181.0), -179.0);
        assert_eq!(wrapped(-181.0), 179.0);
        assert_eq!(wrapped(180.0), 180.0);
    }
}
