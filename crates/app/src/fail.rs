//! Every failure the pilot is told about, and the one way it is told.
//!
//! The owner's ruling on issue #124 is that error surfacing be consistent by
//! code design rather than by discipline. Before it there were two ways a
//! failure could go: an alert, which a file that would not open put up, and
//! an `eprintln!` on a terminal a launcher-started Flatpak sends nowhere,
//! which is what a picture that died mid file got. Which one a failure took
//! was whatever its author happened to write.
//!
//! So the alert's line is private to this module, and the only thing that can
//! put one there is [`Alert::raise`], which takes a [`Failure`] and nothing
//! else. A new failure site cannot invent its own sentence for the window: it
//! adds a variant below, and the compiler then makes it give that variant a
//! title, a line for the pilot and a line for the terminal.
//!
//! The terminal echo is raised **with** the alert rather than beside it, from
//! this one call. That is what leaves nothing for a bare `eprintln!` to be
//! good for at a failure site: it is not a shortcut past this module, it is
//! strictly less than calling it.
//!
//! What this does not cover, deliberately: a capture that could not be
//! written says so in a toast, because the picture is still there and the
//! pilot is still watching it (docs/UI.md, "The capture toast"). The funnel
//! is for the failures that leave him with no video.

use std::error::Error;
use std::path::PathBuf;

use kjerag_render::{Foreign, MissingDecoder, Stall};

use crate::strings;

/// Something the pilot has to be told about, in the terms he met it in.
///
/// One variant per way this app can end up with no video on screen. They are
/// deliberately about the situation and not about the error: two of the three
/// carry the path, because "which file" is the first thing a pilot asks and
/// the terminal line is the first thing a bug report carries.
pub enum Failure {
    /// A file that would not open: another camera's format (issue #107), a
    /// codec this build has no decoder for (issue #69), or anything else the
    /// engine refused it for.
    Open(PathBuf, Box<dyn Error + Send + Sync + 'static>),
    /// A drag and drop that carried no local file.
    Dropped,
    /// The picture died part way through a file and the player was stopped
    /// with it, sound and all (issue #124).
    Stopped(PathBuf, Stall),
}

impl Failure {
    /// The alert's title, which says what happened.
    fn title(&self) -> &'static str {
        match self {
            Self::Open(..) | Self::Dropped => strings::CANNOT_OPEN,
            Self::Stopped(..) => strings::VIDEO_STOPPED,
        }
    }

    /// The alert's body, which says what the pilot can do about it.
    fn said(&self) -> String {
        match self {
            Self::Open(_, e) => refusal(&**e),
            // A drop carrying no local file is a file that would not open
            // with nothing known about why: a URL, a remote share, a
            // selection of text.
            Self::Dropped => strings::OPEN_FAILED.to_owned(),
            Self::Stopped(..) => strings::VIDEO_STOPPED_BODY.to_owned(),
        }
    }

    /// The terminal's line, which says what actually happened and names the
    /// file. `scripts/uitest.sh` reads the first of these.
    fn echoed(&self) -> String {
        match self {
            Self::Open(path, e) => format!("{} not shown: {e}", path.display()),
            Self::Dropped => "that drop carried no local file".to_owned(),
            Self::Stopped(path, stall) => format!("{} stopped: {stall}", path.display()),
        }
    }
}

/// What the window is saying about a failure, and the only place it can be
/// said from.
///
/// The line is private on purpose: nothing outside this module can build a
/// [`Said`], so nothing outside this module can put words in the alert
/// (issue #124).
#[derive(Default)]
pub struct Alert(Option<Said>);

struct Said {
    title: &'static str,
    body: String,
}

impl Alert {
    /// Say it: on screen, where the pilot is, and on the terminal, where a
    /// bug report is written from.
    pub fn raise(&mut self, why: Failure) {
        eprintln!("kjerag: {}", why.echoed());
        self.0 = Some(Said {
            title: why.title(),
            body: why.said(),
        });
    }

    /// Take it away: the alert's own button, Escape, and an open that worked.
    pub fn close(&mut self) {
        self.0 = None;
    }

    pub fn is_up(&self) -> bool {
        self.0.is_some()
    }

    /// The title and the body the dialog draws, and `None` with nothing to
    /// say.
    pub fn showing(&self) -> Option<(&'static str, &str)> {
        let said = self.0.as_ref()?;
        Some((said.title, &said.body))
    }
}

/// The codec a failed open could find no decoder for, and `None` for every
/// other failure (issue #69).
///
/// The engine hands the shell one boxed error from the whole open, and the box
/// arrives with whatever was put in it: `kjerag-media` refuses a stream whose
/// codec has no decoder with a [`MissingDecoder`], and nothing between here
/// and there re-wraps it. So this is a downcast rather than a string match.
///
/// The `'static` on the trait object is what makes the downcast legal: without
/// it the reference's own lifetime becomes the object's, and `downcast_ref` is
/// only implemented for `dyn Error + Send + Sync + 'static`.
fn missing_decoder(e: &(dyn Error + Send + Sync + 'static)) -> Option<&'static str> {
    Some(e.downcast_ref::<MissingDecoder>()?.codec)
}

/// What the alert says a failed open failed for. Three lines, in the order of
/// how much they can tell the pilot: the file is another camera's format
/// (issue #107), this build has no decoder for it (issue #69), or nothing
/// more is known than that it did not open.
fn refusal(e: &(dyn Error + Send + Sync + 'static)) -> String {
    match e.downcast_ref::<Foreign>() {
        Some(foreign) => strings::foreign(*foreign),
        None => strings::open_failed(missing_decoder(e)),
    }
}

/// Which line a failure turns into, and which surface it lands on. No window:
/// what is under test is the funnel's arithmetic, which is the whole of what
/// the window then draws.
#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(e: impl Error + Send + Sync + 'static) -> Box<dyn Error + Send + Sync + 'static> {
        Box::new(e)
    }

    fn file() -> PathBuf {
        PathBuf::from("/home/pilot/Videos/VID_0001.insv")
    }

    /// The shell has to tell a build with no decoder apart from a file it
    /// cannot read, because they get different lines and only one of them is
    /// the pilot's to fix (issue #69). This is that test with the probe stood
    /// in for: the error is built by hand, the way `kjerag-media` builds it on
    /// a box whose ffmpeg has no HEVC in it.
    #[test]
    fn a_missing_decoder_is_told_apart_from_a_file_that_will_not_open() {
        let missing = boxed(MissingDecoder { codec: "hevc" });
        assert_eq!(missing_decoder(&*missing), Some("hevc"));
        assert!(strings::open_failed(missing_decoder(&*missing)).contains("HEVC"));

        let broken: Box<dyn Error + Send + Sync> = "file has no video stream".into();
        assert_eq!(missing_decoder(&*broken), None);
        assert_eq!(
            strings::open_failed(missing_decoder(&*broken)),
            strings::OPEN_FAILED
        );
    }

    /// And the three lines a failed open can leave, told apart by the type in
    /// the box rather than by anything in the message (issue #107). The
    /// foreign one names the format; the other two are what they were.
    #[test]
    fn another_cameras_format_gets_a_line_of_its_own() {
        let gopro = boxed(Foreign::GoPro);
        assert_eq!(refusal(&*gopro), strings::foreign(Foreign::GoPro));
        assert!(refusal(&*gopro).contains("GoPro"), "{}", refusal(&*gopro));

        let missing = boxed(MissingDecoder { codec: "hevc" });
        assert!(refusal(&*missing).contains("HEVC"));

        let broken: Box<dyn Error + Send + Sync> = "file has no video stream".into();
        assert_eq!(refusal(&*broken), strings::OPEN_FAILED);
    }

    /// Issue #124's own case, at the level the shell sees it: a picture that
    /// died mid file is an alert like any other failure, with a title of its
    /// own, because "cannot open file" is not what happened.
    #[test]
    fn a_stopped_video_is_an_alert_and_not_a_refusal() {
        let mut alert = Alert::default();
        assert!(!alert.is_up());

        alert.raise(Failure::Stopped(file(), stall()));
        let (title, body) = alert.showing().expect("nothing on screen");
        assert_eq!(title, strings::VIDEO_STOPPED);
        assert_eq!(body, strings::VIDEO_STOPPED_BODY);
        assert_ne!(title, strings::CANNOT_OPEN);

        alert.close();
        assert!(!alert.is_up());
        assert_eq!(alert.showing(), None);
    }

    /// The terminal keeps the detail the alert leaves out: which file, and
    /// what actually failed. A pilot reads the alert; a bug report carries
    /// this.
    #[test]
    fn the_terminal_line_names_the_file_and_the_reason() {
        let stopped = Failure::Stopped(file(), stall());
        assert_eq!(
            stopped.echoed(),
            "/home/pilot/Videos/VID_0001.insv stopped: 61 frames could not be imported"
        );

        // The wording `scripts/uitest.sh` greps for, unchanged since issue
        // #107: a refusal names the file and then says what it is.
        let refused = Failure::Open(file(), boxed(Foreign::GoPro));
        assert!(refused.echoed().contains("not shown: a GoPro capture"));
    }

    /// A failure with nothing to name still says something the pilot can act
    /// on, rather than nothing at all.
    #[test]
    fn a_drop_with_no_file_in_it_still_says_so() {
        let dropped = Failure::Dropped;
        assert_eq!(dropped.said(), strings::OPEN_FAILED);
        assert_eq!(dropped.title(), strings::CANNOT_OPEN);
        assert!(!dropped.echoed().is_empty());
    }

    /// A stall shaped like the ones the render layer raises, minus the
    /// numbers that change with the run.
    fn stall() -> Stall {
        Stall::new("61 frames could not be imported")
    }
}
