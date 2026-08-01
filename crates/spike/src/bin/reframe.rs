//! Headless reframe: the app's own projection pass, rendered to a PNG.
//!
//! The instrument the angle conventions were settled with, and the only way
//! to look at a reframed frame without a compositor. It builds the same
//! [`ScenePipeline`] the shader widget builds and feeds it the same
//! primitive, so what lands in the PNG is what the window would show.
//!
//! ```sh
//! cargo run --release -p kyerag-spike --bin reframe -- <file.insv>
//! cargo run --release -p kyerag-spike --bin reframe -- <file.insv> yaw=40 pitch=-15 fov=60
//! cargo run --release -p kyerag-spike --bin reframe -- <file.insv> frame=1500
//! ```
//!
//! Arguments after the path are `key=value`: `yaw`, `pitch` and `fov` in
//! degrees, `size` as the output edge in pixels, `out` as the file name,
//! and `frame` or `time` (seconds) to pick which frame is rendered. The
//! frame is no longer always frame 0 (issue #4's first comment): a seek and
//! a decode-forward walk get to any of them.
//!
//! `srgb=1` renders into an sRGB target instead, which is what the window
//! and its captures use. It exists to be compared against the default:
//! the shader linearizes for an sRGB target and the target re-encodes on
//! store, so the two PNGs are the same picture if and only if that round
//! trip is neither doubled nor dropped (issue #15).
//!
//! `lock=1` holds the horizon (issue #8). The **default is off**, which is
//! not the player's default: this is the instrument the lens conventions of
//! 4.8 and 4.9 were settled with, and those are questions about the camera's
//! own frame. A locked view answers a different question and would have
//! silently changed what every command in that section renders.
//!
//! PNGs land in ./scratch/, which is gitignored: frames from real footage
//! are personal video and this repo is public.

use std::fs;
use std::path::{Path, PathBuf};

use std::time::Duration;

use kyerag_media::Fallible;
use kyerag_render::{Camera, Cue, Horizon, Scene, ScenePipeline, Size};
use kyerag_spike::{Gpu, Offscreen};

/// Not sRGB, so the shader writes the video's own gamma-encoded numbers
/// straight out and a PNG viewer shows what the window shows. `srgb=1`
/// swaps it for the format a window surface has.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SRGB: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// The output is square, which is load bearing for the roll check: at yaw
/// and pitch 0 the candidate roll conventions differ by a rotation about the
/// output centre, and only a square output can be compared to its own
/// rotation.
const DEFAULT_EDGE: u32 = 1024;

struct Options {
    input: PathBuf,
    camera: Camera,
    at: Cue,
    edge: u32,
    format: wgpu::TextureFormat,
    horizon: Horizon,
    out: PathBuf,
}

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    fs::create_dir_all(options.out.parent().unwrap_or(Path::new(".")))?;

    let gpu = Gpu::open()?;
    println!("gpu:    {}", gpu.name);

    let scene = Scene::still(&options.input, options.at)?;
    scene.set_horizon(options.horizon);
    let primitive = scene.primitive(options.camera);
    let mut pipeline = ScenePipeline::new(&gpu.device, options.format);
    pipeline.prepare(&primitive, &gpu.device, &gpu.queue, 1.0);

    let target = Offscreen::new(
        &gpu.device,
        Size::new(options.edge, options.edge),
        options.format,
    );
    target.render(&gpu.device, &gpu.queue, &pipeline)?;
    target.write_png(&target.read(&gpu.device, &gpu.queue)?, &options.out)?;

    println!(
        "wrote {} at yaw {:.1}, pitch {:.1}, fov {:.1}, horizon {:?}",
        options.out.display(),
        options.camera.yaw.to_degrees(),
        options.camera.pitch.to_degrees(),
        options.camera.fov.to_degrees(),
        options.horizon,
    );
    Ok(())
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Fallible<Self> {
        let input = PathBuf::from(args.next().ok_or(USAGE)?);
        let mut camera = Camera::default();
        let mut at = Cue::Index(0);
        let mut edge = DEFAULT_EDGE;
        let mut format = FORMAT;
        let mut horizon = Horizon::Free;
        let mut out = None;

        for arg in args {
            let (key, value) = arg.split_once('=').ok_or(USAGE)?;
            match key {
                "yaw" => camera.yaw = value.parse::<f32>()?.to_radians(),
                "pitch" => camera.pitch = value.parse::<f32>()?.to_radians(),
                "fov" => camera.fov = value.parse::<f32>()?.to_radians(),
                "frame" => at = Cue::Index(value.parse()?),
                "time" => at = Cue::Time(Duration::from_secs_f64(value.parse()?)),
                "size" => edge = value.parse()?,
                "srgb" => {
                    format = match value.parse::<u32>()? {
                        0 => FORMAT,
                        _ => SRGB,
                    }
                }
                "lock" => {
                    horizon = match value.parse::<u32>()? {
                        0 => Horizon::Free,
                        _ => Horizon::Locked,
                    }
                }
                "out" => out = Some(value.to_owned()),
                _ => return Err(format!("unknown argument {key}. {USAGE}").into()),
            }
        }

        let name = out.unwrap_or_else(|| {
            format!(
                "reframe-yaw{:.0}-pitch{:.0}-fov{:.0}-{}.png",
                camera.yaw.to_degrees(),
                camera.pitch.to_degrees(),
                camera.fov.to_degrees(),
                match at {
                    Cue::Index(index) => format!("frame{index}"),
                    Cue::Time(time) => format!("t{:.3}", time.as_secs_f64()),
                },
            )
        });
        Ok(Self {
            input,
            camera,
            at,
            edge,
            format,
            horizon,
            out: PathBuf::from("scratch").join(name),
        })
    }
}

const USAGE: &str = "usage: reframe <file.insv> [yaw=deg] [pitch=deg] [fov=deg] \
     [frame=n | time=seconds] [size=px] [srgb=1] [lock=1] [out=name.png]";

/// The app's copied view line is a command line for this binary, and the only
/// way to know that is to hand one to the parser above.
///
/// [`Framing`] writes the line and [`Options::parse`] reads it, so nothing
/// here restates the format: a field renamed on either side stops parsing or
/// stops matching, and both are this test failing.
#[cfg(test)]
mod tests {
    use kyerag_render::Framing;

    use super::*;

    /// Half a unit in the last place each side prints, which is the whole of
    /// what the round trip can lose.
    const TIME_SLACK: f64 = 0.000_5;
    const ANGLE_SLACK: f32 = 0.005;

    fn parse(line: &str) -> Options {
        Options::parse(line.split_whitespace().map(str::to_owned))
            .unwrap_or_else(|e| panic!("reframe would not take {line:?}: {e}"))
    }

    /// Copied in the window, pasted after `reframe`, and the same view comes
    /// out: the file, the frame, all three angles and the horizon.
    #[test]
    fn a_copied_view_line_is_a_reframe_command() {
        let framing = Framing {
            at: Duration::from_millis(754_321),
            camera: Camera {
                yaw: (-37.42_f32).to_radians(),
                pitch: 8.06_f32.to_radians(),
                fov: 64.3_f32.to_radians(),
            },
            horizon: Horizon::Locked,
        };
        let options = parse(&framing.copied(Path::new("/home/pilot/Videos/VID_0001.insv")));

        assert_eq!(options.input, PathBuf::from("VID_0001.insv"));
        assert!(matches!(options.horizon, Horizon::Locked));
        match options.at {
            Cue::Time(time) => assert!(
                (time.as_secs_f64() - framing.at.as_secs_f64()).abs() < TIME_SLACK,
                "{time:?}"
            ),
            other => panic!("a copied view seeks by time, not {other:?}"),
        }
        for (parsed, wanted, axis) in [
            (options.camera.yaw, framing.camera.yaw, "yaw"),
            (options.camera.pitch, framing.camera.pitch, "pitch"),
            (options.camera.fov, framing.camera.fov, "fov"),
        ] {
            let off = (parsed - wanted).to_degrees().abs();
            assert!(off < ANGLE_SLACK, "{axis} is {off} degrees out");
        }
    }

    /// The terminal line is the same command with the path in front of it,
    /// which is what makes it runnable from anywhere.
    #[test]
    fn a_printed_view_line_carries_the_whole_path() {
        let framing = Framing {
            at: Duration::ZERO,
            camera: Camera::default(),
            horizon: Horizon::Free,
        };
        let file = Path::new("/home/pilot/Videos/VID_0001.insv");
        let options = parse(&framing.printed(file));

        assert_eq!(options.input, file);
        assert!(matches!(options.horizon, Horizon::Free));
        assert_eq!(options.at, Cue::Time(Duration::ZERO));
    }

    /// And the view a line reproduces is drawn from the same numbers the
    /// window drew: the parse lands in the very [`Camera`] the render pass
    /// takes, not in a copy of it.
    #[test]
    fn the_parsed_view_is_the_default_view_where_the_window_left_it() {
        let framing = Framing {
            at: Duration::from_millis(1_500),
            camera: Camera::default(),
            horizon: Horizon::Locked,
        };
        assert_eq!(
            parse(&framing.copied(Path::new("f.insv"))).camera,
            Camera::default()
        );
    }
}
