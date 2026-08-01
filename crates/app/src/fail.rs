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
//! What the alert says is the failure's own words (owner's ruling,
//! 2026-08-01). The funnel decides which surface a failure lands on and which
//! title sits over it; it does not rewrite the reason, because a sentence
//! written up here can only ever say less than the one written where the
//! failure happened. The line that started it was "That file could not be
//! opened.", shown over a terminal that was saying "trailer says lens frames
//! are 2880x2880 but the stream decodes 736x368".
//!
//! What this does not cover, deliberately: a capture that could not be
//! written says so in a toast, because the picture is still there and the
//! pilot is still watching it (docs/UI.md, "The capture toast"). The funnel
//! is for the failures that leave him with no video.

use std::error::Error;
use std::path::{Path, PathBuf};

use kjerag_render::{Foreign, MissingDecoder, Stall};

use crate::strings;

/// Something the pilot has to be told about, in the terms he met it in.
///
/// One variant per way this app can end up with no video on screen. They are
/// deliberately about the situation and not about the error: two of the three
/// carry the path, because "which file" is the first thing a pilot asks, the
/// terminal line is the first thing a bug report carries, and a refusal reads
/// the path itself to tell a file outside the sandbox from a missing one
/// (issue #118).
pub enum Failure {
    /// A file that would not open: another camera's format (issue #107), a
    /// path the sandbox was never shown (issue #118), a codec this build has
    /// no decoder for (issue #69), or anything else the engine refused it
    /// for.
    Open(PathBuf, Box<dyn Error + Send + Sync + 'static>),
    /// A drag and drop that carried no local file, and the one failure here
    /// with no error behind it: libcosmic parses the payload and hands over
    /// what it got or nothing at all (`src/widget/dnd_destination.rs:119-120`
    /// calls `.ok()` on the conversion), so the reason never reaches this
    /// process.
    Dropped,
    /// A drop whose files the document portal would not hand over (issue
    /// #118). The portal's own message, which is the only account of it
    /// anything on this side has.
    Portal(String),
    /// The picture died part way through a file and the player was stopped
    /// with it, sound and all (issue #124).
    Stopped(PathBuf, Stall),
}

impl Failure {
    /// The alert's title, which says what happened.
    fn title(&self) -> &'static str {
        match self {
            Self::Open(..) | Self::Dropped | Self::Portal(..) => strings::CANNOT_OPEN,
            Self::Stopped(..) => strings::VIDEO_STOPPED,
        }
    }

    /// The alert's body, which is the reason: the underlying error's own
    /// message wherever there is one (owner's ruling, 2026-08-01).
    fn said(&self) -> String {
        match self {
            Self::Open(path, e) => refusal(&**e, path),
            // A drop with nothing openable in it is the one line here that is
            // not an error's own, because no error survives libcosmic's
            // conversion: what a URL, a remote share or a selection of text
            // leaves this app is `None`.
            Self::Dropped => strings::DROPPED_NOTHING.to_owned(),
            Self::Portal(e) => e.clone(),
            // The stall's own line, and after it the one thing the stall
            // cannot know: that this open is over and the way on is to open
            // the file again. The terminal gets the same first half.
            Self::Stopped(_, stall) => format!("{stall}. {}", strings::VIDEO_STOPPED_ACTION),
        }
    }

    /// The terminal's line, which says the same reason and names the file.
    /// `scripts/uitest.sh` reads the first of these.
    fn echoed(&self) -> String {
        match self {
            Self::Open(path, e) => format!("{} not shown: {e}", path.display()),
            Self::Dropped => "that drop carried no local file".to_owned(),
            Self::Portal(e) => format!("that drop's files stayed with the portal: {e}"),
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

/// What the alert says a failed open failed for, which is the error's own
/// message unless the app knows something the error does not.
///
/// It knows three things, and nothing else here is a sentence of the app's
/// (owner's ruling, 2026-08-01): the file is another camera's format
/// (issue #107), the sandbox was never shown it (issue #118), or this build
/// of ffmpeg has no decoder for it and the pilot can install one (issue #69).
/// Each of those names the failure AND what to do about it, which is more
/// than the error underneath says; every other failure gets the error
/// verbatim, because a line that only says the file did not open is a line
/// that hides the one that said why.
///
/// The path decides the sandbox arm rather than the error, and on purpose. A
/// file the sandbox has no mount for fails somewhere inside libav, which
/// answers "No such file or directory" for a file the pilot is looking at in
/// their file manager; asking the filesystem whether the path is there at all
/// is the same question with none of libav's spelling in it, and it keeps the
/// sandbox out of the layers below the shell (docs/ARCHITECTURE.md).
fn refusal(e: &(dyn Error + Send + Sync + 'static), path: &Path) -> String {
    if let Some(foreign) = e.downcast_ref::<Foreign>() {
        return strings::foreign(*foreign);
    }
    if let Some(codec) = missing_decoder(e) {
        return strings::missing_decoder(codec);
    }
    if sandboxed() && !path.exists() {
        return strings::out_of_reach();
    }
    e.to_string()
}

/// Whether this is running inside a Flatpak, which is a fact about the run and
/// not about the platform: `/.flatpak-info` is the file flatpak mounts into
/// every sandbox, and asking for it is how every toolkit asks this.
fn sandboxed() -> bool {
    Path::new("/.flatpak-info").exists()
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
        assert!(strings::missing_decoder("hevc").contains("HEVC"));

        let broken: Box<dyn Error + Send + Sync> = "file has no video stream".into();
        assert_eq!(missing_decoder(&*broken), None);
    }

    /// The owner's own case (2026-08-01): the reason the terminal carried was
    /// worth reading and the alert said "That file could not be opened." over
    /// the top of it. There is no line to substitute any more, so this asserts
    /// the whole body rather than a phrase inside it, and the terminal echo
    /// carries the same words.
    #[test]
    fn the_alert_says_what_the_error_said() {
        let why = "trailer says lens frames are 2880x2880 but the stream decodes 736x368";
        let refused = Failure::Open(file(), why.into());
        assert_eq!(refused.said(), why);
        assert!(refused.echoed().contains(why), "{}", refused.echoed());
        assert_eq!(refused.title(), strings::CANNOT_OPEN);
    }

    /// And the lines a failed open can leave, told apart by the type in the
    /// box rather than by anything in the message (issue #107). The foreign
    /// one names the format, the missing decoder names the install, and
    /// anything else is itself.
    ///
    /// The path is one that exists, so nothing here takes the sandbox arm:
    /// that one is a fact about the run and the test below says what it is.
    #[test]
    fn another_cameras_format_gets_a_line_of_its_own() {
        let here = Path::new(file!());
        let gopro = boxed(Foreign::GoPro);
        assert_eq!(refusal(&*gopro, here), strings::foreign(Foreign::GoPro));
        assert!(
            refusal(&*gopro, here).contains("GoPro"),
            "{}",
            refusal(&*gopro, here)
        );

        let missing = boxed(MissingDecoder { codec: "hevc" });
        assert!(refusal(&*missing, here).contains("HEVC"));

        let broken: Box<dyn Error + Send + Sync> = "file has no video stream".into();
        assert_eq!(refusal(&*broken, here), "file has no video stream");
    }

    /// A path that is not there says so in the sandbox's words and nowhere
    /// else (issue #118). Outside a Flatpak the same open is a file that was
    /// deleted or renamed, and "Kjerag cannot reach that file from inside its
    /// sandbox" would be a sentence about a sandbox that is not there.
    ///
    /// Both arms are exercised wherever this runs, because what decides is
    /// `/.flatpak-info` and not the test: in CI and on a developer box the
    /// first is what a run gets, and inside the Flatpak the second is, which
    /// is the build the line was written for.
    #[test]
    fn a_path_the_sandbox_cannot_see_is_not_a_missing_file() {
        let gone = Path::new("/nowhere/at/all/flight.insv");
        let broken: Box<dyn Error + Send + Sync> = "No such file or directory".into();
        let expected = match sandboxed() {
            true => strings::out_of_reach(),
            // Outside a sandbox libav is right and gets to say so itself.
            false => "No such file or directory".to_owned(),
        };
        assert_eq!(refusal(&*broken, gone), expected);
        assert!(strings::out_of_reach().contains(strings::OPEN_TITLE));
    }

    /// Issue #124's own case, at the level the shell sees it: a picture that
    /// died mid file is an alert like any other failure, with a title of its
    /// own, because "cannot open file" is not what happened.
    ///
    /// And the body is the stall's own line with the way out on the end of it
    /// (coordinator's call, 2026-08-01, on the ruling this branch is about).
    /// It was a sentence of the shell's that knew less than the stall it stood
    /// over, which is the thing this branch exists to delete.
    #[test]
    fn a_stopped_video_is_an_alert_and_not_a_refusal() {
        let mut alert = Alert::default();
        assert!(!alert.is_up());

        alert.raise(Failure::Stopped(file(), stall()));
        let (title, body) = alert.showing().expect("nothing on screen");
        assert_eq!(title, strings::VIDEO_STOPPED);
        assert_eq!(
            body,
            "61 frames could not be imported. Open the file again."
        );
        assert_ne!(title, strings::CANNOT_OPEN);

        alert.close();
        assert!(!alert.is_up());
        assert_eq!(alert.showing(), None);
    }

    /// The two halves are the two things the pilot needs and neither knows the
    /// other: the stall says what happened, in the words the terminal echo
    /// carries, and the shell says what to do, which it is the only one in a
    /// position to know (the capture is over and nothing is retrying).
    #[test]
    fn a_stopped_video_says_the_stalls_own_words_and_then_what_to_do() {
        let raw = "17 frames could not be imported over 2.0 s, last: Too many open files";
        let stopped = Failure::Stopped(file(), Stall::new(raw));
        let body = stopped.said();
        assert!(body.starts_with(raw), "{body}");
        assert!(body.ends_with(strings::VIDEO_STOPPED_ACTION), "{body}");
        assert!(stopped.echoed().ends_with(raw), "{}", stopped.echoed());
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

    /// The one failure with no error under it says what it does know, which
    /// is that the drop held no file. libcosmic discards the conversion's own
    /// error before this app is called, so there is nothing rawer to show.
    #[test]
    fn a_drop_with_no_file_in_it_still_says_so() {
        let dropped = Failure::Dropped;
        assert_eq!(dropped.said(), strings::DROPPED_NOTHING);
        assert_eq!(dropped.title(), strings::CANNOT_OPEN);
        assert!(dropped.echoed().contains("no local file"));
    }

    /// And the other half of a drop that did not open: the portal answered,
    /// and what it answered is what the pilot reads (issue #118). Before this
    /// the reason went to the terminal and the window said "That file could
    /// not be opened."
    #[test]
    fn a_portal_that_would_not_hand_the_files_over_says_why() {
        let refused = Failure::Portal("Not allowed in sandbox".to_owned());
        assert_eq!(refused.said(), "Not allowed in sandbox");
        assert_eq!(refused.title(), strings::CANNOT_OPEN);
        assert!(refused.echoed().contains("Not allowed in sandbox"));
    }

    /// A stall shaped like the ones the render layer raises, minus the
    /// numbers that change with the run.
    fn stall() -> Stall {
        Stall::new("61 frames could not be imported")
    }
}
