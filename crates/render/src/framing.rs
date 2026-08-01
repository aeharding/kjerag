//! The view as one line of text: which video, which frame, and where it
//! points.
//!
//! A report about a 360 video is unanswerable without the framing. "The seam
//! looks wrong here" names a moment; it does not name the direction the view
//! was pointing when it looked wrong, and nobody can point a second player at
//! it. So the app can hand over a line that says all of it at once, and the
//! line is written as `reframe`'s own arguments (`crates/spike/src/bin/
//! reframe.rs`), which makes it a command as well as a sentence:
//!
//! ```text
//! VID_20260410_185407_00_004.insv time=754.321 yaw=-37.42 pitch=8.06 fov=64.30 lock=1
//! ```
//!
//! That is why this lives here rather than in the app: the app writes the
//! line and the instrument parses it, both crates already depend on this one,
//! and a format written twice is a format that drifts. reframe's own tests
//! feed a line from here to its real argument parser.
//!
//! What the line does not carry is the output size, because reframe renders
//! square and a window is not. The horizontal field of view is the same
//! either way (`Camera::fov`); the vertical is whatever the output's shape
//! leaves, so a square render of a view framed in a wide window shows more
//! above and below it.

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::camera::Camera;
use super::scene::Horizon;

/// Where the view was: which frame of the file, where it pointed, and whether
/// the horizon was held.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framing {
    /// The frame's own media time, which is what `time=` seeks to.
    pub at: Duration,
    pub camera: Camera,
    pub horizon: Horizon,
}

/// Milliseconds. One frame of 29.97 fps content is 33.4 ms, so a time printed
/// this finely names one frame and no other.
const TIME_PLACES: usize = 3;

/// Hundredths of a degree. At the near end of the zoom a 1920 px wide window
/// spans 20 degrees, which is 0.0104 degrees a pixel: a tenth of a degree is
/// ten pixels of the picture and a hundredth is one.
const ANGLE_PLACES: usize = 2;

impl Framing {
    /// The line that goes on the clipboard: the file's own name, and no
    /// directories around it.
    ///
    /// A pilot's report lands in a public issue, and the path above a video
    /// says where he keeps his flights and what his user name is. The name
    /// alone is enough to say which video, and running the line is a `cd`
    /// away.
    pub fn copied(&self, file: &Path) -> String {
        self.line(&name(file).to_string_lossy())
    }

    /// The same view for the terminal, with the whole path in front of it, so
    /// the line can be run from anywhere. Nothing here is read at a glance,
    /// which is where the path is allowed to be.
    pub fn printed(&self, file: &Path) -> String {
        self.line(&file.to_string_lossy())
    }

    fn line(&self, file: &str) -> String {
        let degrees = |radians: f32| format!("{:.*}", ANGLE_PLACES, radians.to_degrees());
        format!(
            "{file} time={:.*} yaw={} pitch={} fov={} lock={}",
            TIME_PLACES,
            self.at.as_secs_f64(),
            degrees(self.camera.yaw),
            degrees(self.camera.pitch),
            degrees(self.camera.fov),
            match self.horizon {
                Horizon::Locked => 1,
                Horizon::Free => 0,
            },
        )
    }
}

/// The last component of a path. `file_name` is `None` only for a path that
/// ends in `..` or is a root, and the fallback names no directory either,
/// which is the property [`Framing::copied`] is keeping.
fn name(file: &Path) -> &OsStr {
    file.file_name().unwrap_or(OsStr::new("video"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framing() -> Framing {
        Framing {
            at: Duration::from_millis(754_321),
            camera: Camera {
                yaw: -37.421_f32.to_radians(),
                pitch: 8.06_f32.to_radians(),
                fov: 64.3_f32.to_radians(),
            },
            horizon: Horizon::Locked,
        }
    }

    /// The whole format, in one string, so changing it takes a diff someone
    /// has to read. reframe's own tests are what say it still parses.
    #[test]
    fn the_line_is_the_video_the_time_and_the_framing() {
        assert_eq!(
            framing().copied(Path::new("/home/pilot/Videos/VID_0001.insv")),
            "VID_0001.insv time=754.321 yaw=-37.42 pitch=8.06 fov=64.30 lock=1"
        );
    }

    /// The one difference between the two lines, and the reason there are
    /// two: the copy is going somewhere public.
    #[test]
    fn only_the_terminal_line_carries_the_path() {
        let file = Path::new("/home/pilot/Videos/VID_0001.insv");
        let copied = framing().copied(file);
        assert!(!copied.contains('/'), "{copied}");
        assert!(!copied.contains("pilot"), "{copied}");
        assert_eq!(
            framing().printed(file),
            format!("/home/pilot/Videos/{copied}")
        );
    }

    /// A file named on the command line with no directory in front of it is
    /// already its own name, and a path that ends in `..` still leaks
    /// nothing.
    #[test]
    fn a_bare_name_survives_and_a_directory_never_appears() {
        let starts = |file| framing().copied(Path::new(file));
        assert!(starts("VID_0001.insv").starts_with("VID_0001.insv "));
        assert!(starts("/home/pilot/..").starts_with("video "));
    }

    /// A free horizon is the other value of the flag reframe reads, and the
    /// two have to be told apart by the line rather than by remembering.
    #[test]
    fn the_horizon_is_a_flag_either_way() {
        let free = Framing {
            horizon: Horizon::Free,
            ..framing()
        };
        assert!(free.copied(Path::new("f.insv")).ends_with(" lock=0"));
        assert!(framing().copied(Path::new("f.insv")).ends_with(" lock=1"));
    }

    /// The time is the frame's, to the millisecond, hours in and at the very
    /// start: `time=` is seconds all the way up, which is what reframe reads,
    /// and never a clock.
    #[test]
    fn the_time_is_seconds_to_the_millisecond() {
        let at = |at| Framing { at, ..framing() }.copied(Path::new("f.insv"));
        assert!(at(Duration::ZERO).contains(" time=0.000 "));
        assert!(at(Duration::from_millis(5_400_500)).contains(" time=5400.500 "));
    }
}
