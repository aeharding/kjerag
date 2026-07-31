//! Every string the pilot can read, in one place.
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

pub const APP_NAME: &str = "Kyerag";
pub const COMMENTS: &str = "360 video player for the COSMIC desktop";
pub const LICENSE: &str = "AGPL-3.0-only";
pub const AUTHOR: &str = "Alexander Harding";
pub const REPOSITORY: &str = "Repository";
pub const REPOSITORY_URL: &str = "https://github.com/aeharding/kyerag";
pub const SUPPORT: &str = "Support";
pub const SUPPORT_URL: &str = "https://github.com/aeharding/kyerag/issues";

/// The welcome view, and the line that says an open did not work.
pub const NOTHING_OPEN: &str = "No video open";
pub const OPEN_BUTTON: &str = "Open video";
pub const OPEN_FAILED: &str = "That file could not be opened.";

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
pub const FULLSCREEN: &str = "Fullscreen";
/// The ellipsis is on the menu item, which opens something; the page it opens
/// is titled without one.
pub const SETTINGS: &str = "Settings...";
pub const SETTINGS_TITLE: &str = "Settings";

/// The Settings page.
pub const APPEARANCE: &str = "Appearance";
pub const THEME: &str = "Theme";
pub const THEME_SYSTEM: &str = "Match desktop";
pub const THEME_DARK: &str = "Dark";
pub const THEME_LIGHT: &str = "Light";

/// `About Kyerag...`, and the drawer's own title.
pub fn about_item() -> String {
    format!("About {APP_NAME}...")
}

/// `{file name} - Kyerag`, and plain `Kyerag` with nothing open.
///
/// cosmic-files writes its equivalent with an em dash
/// (`src/app.rs:1888-1898`); a window title is UI copy, so ours is a hyphen.
pub fn window_title(open: Option<&Path>) -> String {
    match open.and_then(Path::file_name) {
        Some(name) => format!("{} - {APP_NAME}", name.display()),
        None => APP_NAME.to_owned(),
    }
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

    use super::*;

    #[test]
    fn the_title_carries_the_file_name_and_no_em_dash() {
        let path = PathBuf::from("/home/pilot/Videos/VID_20260731_120000_00_007.insv");
        assert_eq!(
            window_title(Some(&path)),
            "VID_20260731_120000_00_007.insv - Kyerag"
        );
        assert_eq!(window_title(None), "Kyerag");
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
            OPEN_FAILED,
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
            QUIT,
            PLAY_PAUSE,
            BACK_10,
            FORWARD_10,
            PREVIOUS_FRAME,
            NEXT_FRAME,
            ZOOM_IN,
            DEFAULT_VIEW,
            ZOOM_OUT,
            FULLSCREEN,
            SETTINGS,
            APPEARANCE,
            THEME,
            THEME_SYSTEM,
            THEME_DARK,
            THEME_LIGHT,
        ];
        for line in copy {
            assert!(!line.contains('\u{2014}'), "em dash in {line:?}");
        }
        assert!(!about_item().contains('\u{2014}'));
    }

    #[test]
    fn the_clock_is_hours_minutes_seconds() {
        assert_eq!(clock(Duration::ZERO), "00:00:00");
        assert_eq!(clock(Duration::from_secs_f64(754.4)), "00:12:34");
        assert_eq!(clock(Duration::from_secs(3661)), "01:01:01");
    }
}
