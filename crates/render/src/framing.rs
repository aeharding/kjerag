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
//! line, the app reads it back, and the instrument parses it too. All three
//! crates already depend on this one, and a format written twice is a format
//! that drifts. reframe keeps its own parser, because its syntax is a
//! superset of this one (`frame=`, `size=`, `srgb=`, `out=`); what holds the
//! two together is a test in reframe that feeds it a line from here.
//!
//! Reading a line back is what makes the line a place and not just a label:
//! `Ctrl+V` in the window goes there, and so does
//! `kyerag <file> time=... yaw=...` on the command line.
//!
//! What the line does not carry is the output size, because reframe renders
//! square and a window is not. The horizontal field of view is the same
//! either way (`Camera::fov`); the vertical is whatever the output's shape
//! leaves, so a square render of a view framed in a wide window shows more
//! above and below it.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::camera::Camera;
use super::scene::Horizon;

/// What the terminal line is labelled with, which is also what a pilot
/// selects along with it when he copies one out of a terminal, so
/// [`Framing::read_line`] takes it off again.
pub const LABEL: &str = "view:";

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
            "{file} {TIME}={:.*} {YAW}={} {PITCH}={} {FOV}={} {LOCK}={}",
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

    /// A whole line read back: which file it names, and where it was looking.
    ///
    /// `None` for anything that is not one of these lines, which is almost
    /// everything a clipboard ever holds. That is why this answers with an
    /// option rather than an error: the paste that lands on a shopping list
    /// has nothing to report, it just is not a view.
    ///
    /// The [`LABEL`] comes off first, so a line selected out of a terminal
    /// works as well as one taken off the clipboard.
    pub fn read_line(line: &str) -> Option<(PathBuf, Self)> {
        let line = line.trim();
        let line = line.strip_prefix(LABEL).unwrap_or(line).trim_start();
        let mut words = line.split_whitespace();
        let file = words.next().filter(|word| !Self::is_term(word))?;
        Some((PathBuf::from(file), Self::read(words).ok().flatten()?))
    }

    /// Whether a word is one of the view's own keys, which is how a file
    /// whose name happens to have an `=` in it is still a file.
    pub fn is_term(word: &str) -> bool {
        word.split_once('=')
            .is_some_and(|(key, _)| [TIME, YAW, PITCH, FOV, LOCK].contains(&key))
    }

    /// The five keys, read off words that have already had the file taken out
    /// of them: what a command line hands over after the path.
    ///
    /// `Ok(None)` is no view at all, which is `kyerag <file>` and has to stay
    /// a way to open a file. Anything begun and not finished is an error,
    /// because half a view is not a place: a line missing its `fov=` would
    /// otherwise open somewhere the pilot did not ask for and say nothing.
    pub fn read<'a>(terms: impl IntoIterator<Item = &'a str>) -> Result<Option<Self>, String> {
        let mut at = None;
        let mut yaw = None;
        let mut pitch = None;
        let mut fov = None;
        let mut lock = None;

        for term in terms {
            let (key, value) = term.split_once('=').ok_or(USAGE)?;
            let degrees = || {
                value
                    .parse::<f32>()
                    .map(f32::to_radians)
                    .map_err(|_| format!("{key}={value} is not a number of degrees"))
            };
            match key {
                TIME => {
                    let seconds = value
                        .parse::<f64>()
                        .map_err(|_| format!("{key}={value} is not a number of seconds"))?;
                    at = Some(Duration::try_from_secs_f64(seconds.max(0.0)).unwrap_or_default());
                }
                YAW => yaw = Some(degrees()?),
                PITCH => pitch = Some(degrees()?),
                FOV => fov = Some(degrees()?),
                LOCK => {
                    lock = Some(match value {
                        "0" => Horizon::Free,
                        "1" => Horizon::Locked,
                        _ => return Err(format!("{key}={value} is not 0 or 1")),
                    })
                }
                _ => return Err(format!("no view has a {key} in it. {USAGE}")),
            }
        }

        match (at, yaw, pitch, fov, lock) {
            (None, None, None, None, None) => Ok(None),
            (Some(at), Some(yaw), Some(pitch), Some(fov), Some(horizon)) => Ok(Some(Self {
                at,
                camera: Camera { yaw, pitch, fov },
                horizon,
            })),
            _ => Err(format!("half a view is not a place. {USAGE}")),
        }
    }
}

/// The keys, named once each, because the writer and the two readers all have
/// to spell them the same way.
const TIME: &str = "time";
const YAW: &str = "yaw";
const PITCH: &str = "pitch";
const FOV: &str = "fov";
const LOCK: &str = "lock";

const USAGE: &str = "a view is time=seconds yaw=deg pitch=deg fov=deg lock=0|1";

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

    /// Half a unit in the last place each side prints, which is all a written
    /// line can lose.
    const TIME_SLACK: f64 = 0.000_5;
    const ANGLE_SLACK: f32 = 0.005;

    fn assert_same(read: Framing, wanted: Framing) {
        let off = (read.at.as_secs_f64() - wanted.at.as_secs_f64()).abs();
        assert!(off < TIME_SLACK, "{off} s out");
        assert_eq!(read.horizon, wanted.horizon);
        for (got, want, axis) in [
            (read.camera.yaw, wanted.camera.yaw, "yaw"),
            (read.camera.pitch, wanted.camera.pitch, "pitch"),
            (read.camera.fov, wanted.camera.fov, "fov"),
        ] {
            let off = (got - want).to_degrees().abs();
            assert!(off < ANGLE_SLACK, "{axis} is {off} degrees out");
        }
    }

    /// Copied in one window and pasted into another: both lines come back as
    /// the view that wrote them, and each brings its own file back with it.
    #[test]
    fn a_line_reads_back_as_the_view_that_wrote_it() {
        let file = Path::new("/home/pilot/Videos/VID_0001.insv");
        let (named, read) = Framing::read_line(&framing().copied(file)).expect("a copied line");
        assert_eq!(named, Path::new("VID_0001.insv"));
        assert_same(read, framing());

        let (named, read) = Framing::read_line(&framing().printed(file)).expect("a printed line");
        assert_eq!(named, file);
        assert_same(read, framing());
    }

    /// A pilot selecting the line out of a terminal takes its label with it,
    /// and the paste has to work anyway. Trailing whitespace comes with
    /// nearly every copy there is.
    #[test]
    fn the_terminal_label_and_the_whitespace_come_off() {
        let line = format!("  view:   {}\n", framing().copied(Path::new("f.insv")));
        let (file, read) = Framing::read_line(&line).expect("a labelled line");
        assert_eq!(file, Path::new("f.insv"));
        assert_same(read, framing());
    }

    /// What a clipboard actually holds, nearly always. None of it is a view
    /// and none of it may be read as one, because a paste that does
    /// something surprising is worse than a paste that does nothing.
    #[test]
    fn anything_else_on_the_clipboard_is_not_a_view() {
        for text in [
            "",
            "   \n ",
            "https://github.com/aeharding/kjerag/issues/99",
            "the seam looks wrong here",
            "f.insv",
            "f.insv time=1.000",
            "f.insv time=1.000 yaw=0.00 pitch=0.00 fov=90.00",
            "time=1.000 yaw=0.00 pitch=0.00 fov=90.00 lock=1",
            "f.insv time=soon yaw=0.00 pitch=0.00 fov=90.00 lock=1",
            "f.insv time=1.000 yaw=0.00 pitch=0.00 fov=90.00 lock=2",
            "f.insv time=1.000 yaw=0.00 pitch=0.00 fov=90.00 lock=1 seam=file",
        ] {
            assert!(
                Framing::read_line(text).is_none(),
                "read {text:?} as a view"
            );
        }
    }

    /// The command line's own reading: nothing to apply, a whole view, and
    /// the two ways to get it wrong. A file is opened by `kyerag <file>` with
    /// no view at all, so no terms cannot be an error.
    #[test]
    fn the_terms_are_all_of_them_or_none_of_them() {
        assert_eq!(Framing::read([]), Ok(None));

        let whole = [
            "time=754.321",
            "yaw=-37.42",
            "pitch=8.06",
            "fov=64.30",
            "lock=1",
        ];
        assert_same(Framing::read(whole).unwrap().unwrap(), framing());

        assert!(Framing::read(["time=754.321"]).is_err(), "half a view");
        assert!(Framing::read(["fov=wide"]).is_err(), "not a number");
        assert!(Framing::read(["zoom=2"]).is_err(), "not a key");
        assert!(Framing::read(["lock"]).is_err(), "not a term");
    }

    /// A view term is one of the five and nothing else, which is what keeps a
    /// file whose name has an `=` in it out of the view and in the path.
    #[test]
    fn only_the_five_keys_are_terms() {
        assert!(Framing::is_term("time=1"));
        assert!(Framing::is_term("lock=0"));
        assert!(!Framing::is_term("/home/pilot/a=b.insv"));
        assert!(!Framing::is_term("VID_0001.insv"));
        assert!(!Framing::is_term("--help"));
    }

    /// A negative time is not a time. It cannot come out of the writer, and a
    /// hand-edited line must not panic the window it is pasted into.
    #[test]
    fn a_time_before_the_start_is_the_start() {
        let line = "f.insv time=-5.000 yaw=0.00 pitch=0.00 fov=90.00 lock=0";
        let (_, read) = Framing::read_line(line).expect("a line");
        assert_eq!(read.at, Duration::ZERO);
    }
}
