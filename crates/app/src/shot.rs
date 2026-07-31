//! Where a captured frame goes: a PNG in the screenshots folder, or the
//! clipboard.
//!
//! Two destinations rather than one, because docs/UI.md's File menu has two
//! items (`Save frame` and `Copy frame`, on `s` and `Ctrl+C`). They share
//! everything but the last step.
//!
//! **The folder** is the one the COSMIC screenshot portal writes to, read
//! off its source rather than guessed: `XDG_SCREENSHOTS_DIR` when it is set
//! and absolute, else the XDG pictures directory (or `~/Pictures`) with
//! `Screenshots` under it, created if it is missing
//! (`xdg-desktop-portal-cosmic` rev f211aa3, `src/screenshot.rs:235-266`,
//! down to the same `dirs` crate).
//!
//! **The name** is not the portal's `Screenshot_<date>.png`. A flight
//! review produces dozens of stills of one file, and what a pilot asks of
//! one later is which video and which moment, not which afternoon. So it is
//! the video's own name and the timecode of the frame.
//!
//! Everything in here runs on the capture's worker thread, off the render
//! path; only the clipboard has to go back to the shell, because on Wayland
//! the clipboard is the window's to offer.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cosmic::iced::clipboard::mime::AsMimeTypes;
use kyerag_render::{Fallible, Shot};

/// How wide a still is, whatever the window is.
///
/// A lens frame is 3840 px across and a reframed view samples a patch of
/// it, so this is roughly source sharpness at the default field of view and
/// past it when zoomed in. docs/UI.md parks a `Window size | 2x window |
/// Source` setting for this; it lands with the settings page, which is the
/// app shell's (issue #16), not this file's.
pub const WIDTH: u32 = 3840;

/// The two things a capture can be for.
#[derive(Clone, Copy, Debug)]
pub enum Destination {
    Save,
    Copy,
}

/// A finished capture, on its way back to the shell.
#[derive(Clone, Debug)]
pub enum Done {
    Saved(PathBuf),
    Copied(Png),
}

/// A PNG for the clipboard.
///
/// libcosmic's clipboard takes any mime type, so a paste-friendly copy costs
/// no portal and no new dependency: cosmic-files writes its own data the
/// same way (`src/app.rs:2957`, `clipboard::write_data`) and reads
/// `image/png` back off the clipboard in `ClipboardPasteImage`
/// (`src/clipboard.rs:165-200`), which is the same mime type this offers.
#[derive(Clone, Debug)]
pub struct Png(Vec<u8>);

impl AsMimeTypes for Png {
    fn available(&self) -> Cow<'static, [String]> {
        Cow::Owned(vec![MIME.to_owned()])
    }

    fn as_bytes(&self, mime: &str) -> Option<Cow<'static, [u8]>> {
        (mime == MIME).then(|| Cow::Owned(self.0.clone()))
    }
}

const MIME: &str = "image/png";

/// Encode, then save or hand back. Worker thread.
pub fn finish(shot: &Shot, video: &Path, to: Destination) -> Fallible<Done> {
    let png = encode(shot)?;
    match to {
        Destination::Copy => Ok(Done::Copied(Png(png))),
        Destination::Save => {
            let folder = folder()?;
            let path = folder.join(unused(&name(stem(video), shot.time), |name| {
                folder.join(name).exists()
            }));
            fs::write(&path, png)?;
            Ok(Done::Saved(path))
        }
    }
}

/// Named rather than left to the crate (which happens to default to the
/// same thing today), because it is a choice with a number behind it. One
/// 3840x2160 still is 33 MB of pixels, measured on this box over a real
/// captured frame: `Fast` 53 ms for 8.5 MB, `Default` 1.6 s for 6.8 MB,
/// `Best` 6.3 s for 6.4 MB. A capture the pilot waits seconds for is a
/// capture they press twice.
const COMPRESSION: png::Compression = png::Compression::Fast;

fn encode(shot: &Shot) -> Fallible<Vec<u8>> {
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, shot.width, shot.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(COMPRESSION);
    // The bytes are the surface's own, so the file says which space they
    // are in rather than leaving a viewer to assume it.
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    encoder.write_header()?.write_image_data(&shot.rgba)?;
    Ok(png)
}

fn folder() -> Fallible<PathBuf> {
    let folder = std::env::var_os("XDG_SCREENSHOTS_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| {
            dirs::picture_dir()
                .or_else(|| dirs::home_dir().map(|home| home.join("Pictures")))
                .map(|pictures| pictures.join("Screenshots"))
        })
        .ok_or("no screenshots folder: no XDG_SCREENSHOTS_DIR and no home directory")?;
    fs::create_dir_all(&folder)?;
    Ok(folder)
}

fn stem(video: &Path) -> &str {
    video
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("kyerag")
}

/// `<video>_<timecode>.png`: which video, and where in it.
fn name(stem: &str, at: Duration) -> String {
    let millis = at.as_millis();
    format!(
        "{stem}_{:02}-{:02}-{:02}.{:03}.png",
        millis / 3_600_000,
        millis / 60_000 % 60,
        millis / 1_000 % 60,
        millis % 1_000,
    )
}

/// How many names past the first one to try before giving up and writing
/// over something. Reaching it means a thousand stills of one frame.
const CROWDED: u32 = 1_000;

/// The first name nothing has taken. Two captures of one paused frame are
/// two different views of it, and the second must not replace the first
/// just because the timecode is the same.
fn unused(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_owned();
    }
    let (stem, extension) = name.rsplit_once('.').unwrap_or((name, "png"));
    (2..CROWDED)
        .map(|n| format!("{stem}-{n}.{extension}"))
        .find(|name| !taken(name))
        .unwrap_or_else(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The question a still has to answer months later: which video, and
    /// which moment.
    #[test]
    fn a_name_carries_the_video_and_the_timecode() {
        assert_eq!(
            name("VID_20260410_185407_00_004", Duration::from_millis(754_321)),
            "VID_20260410_185407_00_004_00-12-34.321.png"
        );
        assert_eq!(
            name("flight", Duration::from_secs(3661)),
            "flight_01-01-01.000.png"
        );
        assert_eq!(name("flight", Duration::ZERO), "flight_00-00-00.000.png");
    }

    /// Over an hour is hours, not minutes: a 90-minute flight is one file.
    #[test]
    fn the_timecode_carries_hours() {
        assert_eq!(name("f", Duration::from_secs(5400)), "f_01-30-00.000.png");
    }

    #[test]
    fn the_name_is_the_video_even_with_a_path_around_it() {
        assert_eq!(
            stem(Path::new("/home/pilot/Videos/VID_0001.insv")),
            "VID_0001"
        );
        assert_eq!(stem(Path::new("relative.insv")), "relative");
    }

    /// A paused frame captured twice from two directions is two stills.
    #[test]
    fn a_taken_name_gets_a_number() {
        let taken = ["f_00-00-01.000.png", "f_00-00-01.000-2.png"];
        let free = |name: &str| taken.contains(&name);

        assert_eq!(unused("f_00-00-02.000.png", free), "f_00-00-02.000.png");
        assert_eq!(unused(taken[0], free), "f_00-00-01.000-3.png");
    }

    /// The suffix goes before the extension, not after it: a `.png-2` is
    /// not a PNG to anything that reads names.
    #[test]
    fn the_number_keeps_the_extension_last() {
        assert!(unused("f.png", |_| true).ends_with(".png"));
    }
}
