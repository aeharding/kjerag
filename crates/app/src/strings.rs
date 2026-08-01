//! Every string this app writes for the pilot, in one place.
//!
//! Not every string he reads: what a failure says is the failure's own
//! message, written where it happened and shown verbatim (AGENTS.md,
//! `crate::fail`). Three lines here are the exception and each says why it is
//! one.
//!
//! No i18n in the first landing. All three first-party COSMIC apps put their
//! copy behind `i18n-embed` + fluent and an `fl!` macro, which is real
//! machinery for a pre-alpha with one user; keeping the strings together is
//! what makes that move mechanical later. Treat this as a deviation to
//! revisit before any public release rather than as a decision that strings
//! live inline forever (docs/UI.md, "Copy").
//!
//! The rules the strings below follow: plain words and no em dashes
//! (AGENTS.md), sentence case for labels, an ellipsis only on an item that
//! opens a dialog, and a space before a unit (System76 HIG).

use std::path::Path;

use kjerag_render::Foreign;

use crate::shot::Destination;

pub const APP_NAME: &str = "Kjerag";
pub const COMMENTS: &str = "360 video player for the COSMIC desktop";
pub const LICENSE: &str = "AGPL-3.0-only";
pub const AUTHOR: &str = "Alexander Harding";
pub const REPOSITORY: &str = "Repository";
pub const REPOSITORY_URL: &str = "https://github.com/aeharding/kjerag";
pub const SUPPORT: &str = "Support";
pub const SUPPORT_URL: &str = "https://github.com/aeharding/kjerag/issues";

/// The welcome view.
pub const NOTHING_OPEN: &str = "No video open";
pub const OPEN_BUTTON: &str = "Open video";

/// The alert a file that would not open puts up, which is a stock dialog in
/// the middle of the window (owner's call, 2026-08-01). The title says what
/// happened, the body below says why, and the one button takes it away.
///
/// There is no line here for "it did not open", and that is the point: the
/// body is the failure's own message (`crate::fail`, AGENTS.md).
pub const CANNOT_OPEN: &str = "Cannot open file";
pub const CLOSE: &str = "Close";

/// A drop that held nothing this app can open, which is the one failure with
/// no error behind it to show instead: libcosmic converts the payload and
/// keeps only what converted (`src/widget/dnd_destination.rs:119-120`), so a
/// URL, a remote share or a selection of text all arrive as nothing at all.
pub const DROPPED_NOTHING: &str =
    "That drop carried no file. Drag a .insv file in from your file manager.";

/// The same alert, for a video that was playing and stopped (issue #124).
///
/// A title of its own, because "Cannot open file" is not what happened: the
/// file opened, it played, and then the pictures stopped arriving. What
/// actually failed goes to the terminal, where a bug report is written from
/// (`crate::fail`).
///
/// The body says to open the file again because that is the only thing that
/// works, and the app has to mean it: this open is over, and nothing behind
/// the alert is retrying (`kjerag_render`'s `Stalled`). It was written before
/// that was true, and the owner met the difference as five of these in one
/// sitting.
pub const VIDEO_STOPPED: &str = "Video stopped";
pub const VIDEO_STOPPED_BODY: &str =
    "The picture could not be drawn, so playback stopped. Open the file again.";

/// The file chooser.
pub const OPEN_TITLE: &str = "Open video";
pub const INSV_FILTER: &str = "Insta360 video";

/// Menu roots.
pub const FILE: &str = "File";
pub const PLAYBACK: &str = "Playback";
pub const VIEW: &str = "View";

/// `File`.
pub const OPEN_VIDEO: &str = "Open video...";
pub const OPEN_RECENT: &str = "Open recent";
pub const CLEAR_RECENT: &str = "Clear recent list";
pub const CLOSE_VIDEO: &str = "Close video";
pub const SAVE_FRAME: &str = "Save frame";
pub const COPY_FRAME: &str = "Copy frame";
/// The owner's own wording, verbatim: `Copy view` said nothing to anyone who
/// had not already been told what it did. Its counterpart mirrors it word for
/// word, because the two are one idea and half a name would hide that.
///
/// The vocabulary the rest of these follow: the **reference** is the text and
/// the **view** is the place it names. A reference is copied; a view is gone
/// to.
pub const COPY_VIEW: &str = "Copy current view reference";
pub const GO_TO_VIEW: &str = "Go to copied view reference";
pub const QUIT: &str = "Quit";

/// `Playback`.
pub const PLAY_PAUSE: &str = "Play / Pause";
pub const BACK_10: &str = "Back 10 seconds";
pub const FORWARD_10: &str = "Forward 10 seconds";
pub const PREVIOUS_FRAME: &str = "Previous frame";
pub const NEXT_FRAME: &str = "Next frame";

/// `View`.
pub const ZOOM_IN: &str = "Zoom in";
pub const DEFAULT_VIEW: &str = "Default view";
pub const ZOOM_OUT: &str = "Zoom out";
/// The horizon lock toggle (issue #8). "Lock horizon" is what Insta360's own
/// app and Studio call it, so it is the words this pilot already has.
pub const LOCK_HORIZON: &str = "Lock horizon";
pub const FULLSCREEN: &str = "Fullscreen";
/// The ellipsis is on the menu item, which opens something; the page it opens
/// is titled without one.
pub const SETTINGS: &str = "Settings...";
pub const SETTINGS_TITLE: &str = "Settings";

/// What a capture says when it lands (issue #15). The noun is the menu's
/// own: the pilot pressed `Copy frame`, so the toast says frame.
pub const FRAME_COPIED: &str = "Frame copied to the clipboard";

/// And the same sentence for the line of text, with the menu item's own noun
/// in it. It names the destination for the reason the frame's does: a copy
/// that does not say where it went is a copy nobody trusts enough to paste.
pub const VIEW_COPIED: &str = "View reference copied to the clipboard";

/// What a paste that landed says. This one is about the place rather than the
/// text, which is why it is the shorter noun: nobody goes to a reference.
pub const WENT_TO_VIEW: &str = "Went to the copied view";

/// The Settings page.
pub const APPEARANCE: &str = "Appearance";
pub const THEME: &str = "Theme";
pub const THEME_SYSTEM: &str = "Match desktop";
pub const THEME_DARK: &str = "Dark";
pub const THEME_LIGHT: &str = "Light";

/// `About Kjerag...`, and the drawer's own title.
pub fn about_item() -> String {
    format!("About {APP_NAME}...")
}

/// A codec this ffmpeg has no decoder for, which is a different failure from
/// a file that will not open and has a different answer (issue #69): nothing
/// is wrong with the file, and one install fixes every file of that kind at
/// once. Inside a Flatpak it is the way this happens, and the extension is
/// named because that name is the whole of the pilot's fix
/// (docs/DISTRIBUTION.md 3.3). The sentence stays true off Flatpak, where a
/// stripped ffmpeg has the same shape.
///
/// One of the three lines the shell is still allowed to write over an error
/// (`crate::fail::refusal`), because which package carries the decoder is
/// something `kjerag-media` has no way of knowing: what it says is "no hevc
/// decoder in this libavcodec", which is true and leaves the pilot nowhere.
///
/// The codec is ffmpeg's own short name, upper-cased: `HEVC`, `H264`. Not a
/// table of prettier spellings, because the reason to print it is that it can
/// be searched for and repeated in a bug report.
pub fn missing_decoder(codec: &str) -> String {
    format!(
        "Kjerag has no {} decoder here, so that file cannot be played. \
         In a Flatpak, the decoder comes from the codecs-extra runtime extension.",
        codec.to_uppercase()
    )
}

/// A file the sandbox cannot see, which is not the same as a file that is not
/// there (issue #118).
///
/// A path that arrives from a drag out of cosmic-files, from the command line
/// or from a double click is the host's own, and inside a Flatpak most of the
/// host is not mounted: the open fails with "No such file or directory" for a
/// file the pilot can see in their file manager, which reads as the app
/// calling them a liar. The manifest grants the videos folder read-only, so
/// what is left of this is footage kept somewhere else, and the answer to that
/// is one menu item away and costs no permission at all, so the line names it.
///
/// Said only inside a Flatpak, because outside one a file that is not there
/// really is not there and this would be a lie (`app::refusal`).
pub fn out_of_reach() -> String {
    // The menu item carries an ellipsis, because it opens a dialog; a
    // sentence naming it does not, because the sentence ends there.
    format!(
        "Kjerag cannot reach that file from inside its sandbox. Open it with {FILE} > {OPEN_TITLE}."
    )
}

/// The other 360 cameras, named (issue #107).
///
/// A GoPro `.360` or a DJI `.osv` is a valid MP4 that Kjerag cannot read a
/// lens calibration out of, so before this it got the words a corrupt file
/// gets. Naming the format is the whole point: it says the file is fine and
/// the player is the wrong one, which is a thing the pilot can act on.
///
/// Second sentence over the three, because a refusal that does not say what
/// Kjerag does take leaves the pilot to guess at it.
pub fn foreign(format: Foreign) -> String {
    let what = match format {
        Foreign::GoPro => "a GoPro video",
        Foreign::Dji => "a DJI video",
        // The spherical arm: an MP4 with 360 metadata in it, which is what
        // every one of these cameras' desktop apps exports. Kjerag reads the
        // raw dual fisheye and reprojects it; a stitched file has already had
        // that done to it.
        Foreign::Spherical => "a stitched 360 video",
    };
    format!("That is {what}. Kjerag plays Insta360 .insv only.")
}

/// `{file name} - Kjerag`, and plain `Kjerag` with nothing open.
///
/// cosmic-files writes its equivalent with an em dash
/// (`src/app.rs:1888-1898`); a window title is UI copy, so ours is a hyphen.
pub fn window_title(open: Option<&Path>) -> String {
    match open.and_then(Path::file_name) {
        Some(name) => format!("{} - {APP_NAME}", name.display()),
        None => APP_NAME.to_owned(),
    }
}

/// Where a still went, named the way cosmic-files names a destination in its
/// own toasts: the folder's own name in quotes, and no path around it
/// (`i18n/en/cosmic_files.ftl:231-234` `copied = ... to "{$to}"`, built from
/// `file_name(to)` in `src/operation/mod.rs:563-568` and `309-312`). The
/// whole path is on the terminal, where it does not have to be read at a
/// glance.
///
/// The folder is whatever the capture resolved to, which is usually
/// `Screenshots` but is `XDG_SCREENSHOTS_DIR`'s last part when that is set.
pub fn frame_saved(path: &Path) -> String {
    match path.parent().and_then(Path::file_name) {
        Some(folder) => format!("Frame saved to \"{}\"", folder.display()),
        None => "Frame saved".to_owned(),
    }
}

/// A capture that did not happen, with the reason it did not. Which half
/// failed matters to the pilot: nothing was written, or nothing can be
/// pasted.
pub fn capture_failed(to: Destination, reason: &str) -> String {
    match to {
        Destination::Save => format!("Frame not saved: {reason}"),
        Destination::Copy => format!("Frame not copied: {reason}"),
    }
}

/// A pasted reference that names a video this window is not showing, and
/// carries no directories to find it in. There is nowhere to go, so all this
/// does is say which video it belongs to, which is the one thing the pilot
/// cannot see for himself.
///
/// The file is quoted and not pathed, which is how `frame_saved` above names
/// a destination and how cosmic-files names one.
pub fn view_is_from(file: &Path) -> String {
    format!("That view reference is from \"{}\"", file.display())
}

/// A recent file as the menu shows it: under the home directory, `~` stands
/// in for it, which is what cosmic-player and cosmic-edit both do.
pub fn recent(path: &Path) -> String {
    let home = std::env::home_dir();
    match home
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// `HH:MM:SS`, the six-line formatter cosmic-player writes
/// (`src/main.rs:1668-1674`). The leading zeros are noise on a 30-minute
/// file and we copy them anyway: a player that formats time its own way for
/// no reason is a player that looks foreign.
pub fn clock(time: std::time::Duration) -> String {
    let seconds = time.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use kjerag_render::{MissingDecoder, Stall};

    use super::*;

    #[test]
    fn the_title_carries_the_file_name_and_no_em_dash() {
        let path = PathBuf::from("/home/pilot/Videos/VID_20260731_120000_00_007.insv");
        assert_eq!(
            window_title(Some(&path)),
            "VID_20260731_120000_00_007.insv - Kjerag"
        );
        assert_eq!(window_title(None), "Kjerag");
    }

    /// AGENTS.md forbids em dashes in anything the pilot reads, and a
    /// constant is easier to break than a rule is to remember.
    #[test]
    fn no_string_carries_an_em_dash() {
        let copy = [
            APP_NAME,
            COMMENTS,
            NOTHING_OPEN,
            OPEN_BUTTON,
            DROPPED_NOTHING,
            CANNOT_OPEN,
            VIDEO_STOPPED,
            VIDEO_STOPPED_BODY,
            CLOSE,
            OPEN_TITLE,
            INSV_FILTER,
            FILE,
            PLAYBACK,
            VIEW,
            OPEN_VIDEO,
            OPEN_RECENT,
            CLEAR_RECENT,
            CLOSE_VIDEO,
            SAVE_FRAME,
            COPY_FRAME,
            COPY_VIEW,
            GO_TO_VIEW,
            QUIT,
            PLAY_PAUSE,
            BACK_10,
            FORWARD_10,
            PREVIOUS_FRAME,
            NEXT_FRAME,
            ZOOM_IN,
            DEFAULT_VIEW,
            ZOOM_OUT,
            LOCK_HORIZON,
            FULLSCREEN,
            SETTINGS,
            APPEARANCE,
            THEME,
            THEME_SYSTEM,
            THEME_DARK,
            THEME_LIGHT,
            FRAME_COPIED,
            VIEW_COPIED,
            WENT_TO_VIEW,
        ];
        for line in copy {
            assert!(!line.contains('\u{2014}'), "em dash in {line:?}");
        }
        assert!(!about_item().contains('\u{2014}'));
        assert!(!frame_saved(Path::new("/tmp/Screenshots/a.png")).contains('\u{2014}'));
        assert!(!capture_failed(Destination::Save, "no").contains('\u{2014}'));
        assert!(!view_is_from(Path::new("a.insv")).contains('\u{2014}'));
        assert!(!missing_decoder("hevc").contains('\u{2014}'));
        assert!(!out_of_reach().contains('\u{2014}'));
        for format in [Foreign::GoPro, Foreign::Dji, Foreign::Spherical] {
            assert!(!foreign(format).contains('\u{2014}'));
        }
    }

    /// The pilot reads engine errors now, so they are pilot-facing copy and
    /// the same rule binds them (AGENTS.md, and `crate::fail` for how they
    /// get there). This is the app's half: the error types the shell can name.
    /// `kjerag-meta` tests its own `Error` in its own crate, which is where
    /// those sentences are written.
    #[test]
    fn no_engine_error_the_alert_can_show_carries_an_em_dash() {
        let said = [
            MissingDecoder { codec: "hevc" }.to_string(),
            Foreign::GoPro.to_string(),
            Foreign::Dji.to_string(),
            Foreign::Spherical.to_string(),
            Stall::new("61 frames could not be imported").to_string(),
        ];
        for line in said {
            assert!(!line.contains('\u{2014}'), "em dash in {line:?}");
        }
    }

    /// A refusal that names the format, one line per format, and every one of
    /// them saying what Kjerag does take (issue #107). The pilot is holding a
    /// file that is not broken and a player that is the wrong one, and only
    /// the second half of that is something he can act on.
    #[test]
    fn a_foreign_format_is_named_and_so_is_the_one_kjerag_takes() {
        assert_eq!(
            foreign(Foreign::GoPro),
            "That is a GoPro video. Kjerag plays Insta360 .insv only."
        );
        assert_eq!(
            foreign(Foreign::Dji),
            "That is a DJI video. Kjerag plays Insta360 .insv only."
        );
        assert_eq!(
            foreign(Foreign::Spherical),
            "That is a stitched 360 video. Kjerag plays Insta360 .insv only."
        );
    }

    /// A missing decoder is not a broken file, and the line has to say which
    /// codec is missing and where it comes from, or the pilot is left with a
    /// player that refuses a file for no stated reason (issue #69).
    ///
    /// This is the whole of the wording, checked with no ffmpeg in sight: the
    /// probe that decides which branch runs is one `avcodec_find_decoder` call
    /// in `kjerag-media`, and a box whose ffmpeg has HEVC cannot exercise the
    /// other branch of it honestly.
    #[test]
    fn a_missing_decoder_names_the_codec_and_the_extension() {
        let line = missing_decoder("hevc");
        assert!(line.contains("HEVC"), "{line}");
        assert!(line.contains("codecs-extra"), "{line}");
    }

    /// The toast answers one question, which is where to look for the still.
    /// The folder is named and the path is not: the pilot reads this over the
    /// video, in the two seconds before the control row hides.
    #[test]
    fn the_saved_toast_names_the_folder_and_no_path() {
        let toast = frame_saved(Path::new(
            "/home/pilot/Pictures/Screenshots/f_00-00-01.000.png",
        ));
        assert_eq!(toast, "Frame saved to \"Screenshots\"");
        assert!(!toast.contains('/'));
    }

    /// `XDG_SCREENSHOTS_DIR` can point anywhere, and the toast has to say
    /// where the still actually went rather than where it usually goes.
    #[test]
    fn the_saved_toast_follows_the_folder_that_was_used() {
        assert_eq!(
            frame_saved(Path::new("/mnt/flights/stills/f_00-00-01.000.png")),
            "Frame saved to \"stills\""
        );
        assert_eq!(frame_saved(Path::new("f.png")), "Frame saved");
    }

    /// A failure says which half failed: nothing was written, or nothing can
    /// be pasted.
    #[test]
    fn a_failed_capture_says_which_one_it_was() {
        assert_eq!(
            capture_failed(Destination::Save, "Permission denied (os error 13)"),
            "Frame not saved: Permission denied (os error 13)"
        );
        assert_eq!(
            capture_failed(Destination::Copy, "out of memory"),
            "Frame not copied: out of memory"
        );
    }

    #[test]
    fn the_clock_is_hours_minutes_seconds() {
        assert_eq!(clock(Duration::ZERO), "00:00:00");
        assert_eq!(clock(Duration::from_secs_f64(754.4)), "00:12:34");
        assert_eq!(clock(Duration::from_secs(3661)), "01:01:01");
    }
}
