//! Where a captured frame goes: a JPEG in the screenshots folder, or a PNG
//! on the clipboard.
//!
//! Two destinations rather than one, because docs/UI.md's File menu has two
//! items (`Save frame` and `Copy frame`, on `s` and `Ctrl+C`). They share
//! everything but the last step, and that step is where the two formats
//! part: a file is shared and double clicked months later, while a paste
//! goes straight into an editor and never touches a disk.
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
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use cosmic::iced::clipboard::mime::AsMimeTypes;
use jpeg_encoder::{ColorType, SamplingFactor};
use kjerag_render::{Fallible, Shot};

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
    match to {
        Destination::Copy => Ok(Done::Copied(Png(png(shot)?))),
        Destination::Save => {
            let bytes = jpeg(shot)?;
            let folder = folder()?;
            let path = write_new(&folder, &name(stem(video), shot.time), &bytes)?;
            Ok(Done::Saved(path))
        }
    }
}

/// How hard a saved still is compressed (issue #15).
///
/// Measured on this box over five real 3840x2160 captures (sky and wing,
/// the lens seam, dense ground detail, a low sun, a wide view): 0.57 to
/// 1.54 MB a still at this setting, against 5.3 to 13.3 MB as a lossless
/// PNG, at 51.3 to 53.6 dB PSNR and 0.9946 to 0.9975 SSIM against those
/// same pixels (ffmpeg's `psnr` and `ssim` filters, both inputs at
/// yuv444p). Every lossless candidate measured, PNG's own levels and
/// lossless WebP included, stayed above 2.8 MB. Was 93; the owner judged
/// the files still a bit big at that setting (2026-07-31).
const QUALITY: u8 = 90;

/// Full size chroma planes. 4:2:0 is 15% smaller and costs 1.5 dB of
/// chroma on the same captures, and chroma is where the wing's lines
/// against a flat sky are.
const SAMPLING: SamplingFactor = SamplingFactor::R_4_4_4;

fn jpeg(shot: &Shot) -> Fallible<Vec<u8>> {
    let edge =
        |px: u32| u16::try_from(px).map_err(|_| format!("a {px} px edge is past JPEG's 65535"));
    let mut jpeg = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut jpeg, QUALITY);
    encoder.set_sampling_factor(SAMPLING);
    // The alpha the pass writes is opaque everywhere, and JPEG has no
    // channel to put it in anyway.
    encoder.encode(
        &shot.rgba,
        edge(shot.width)?,
        edge(shot.height)?,
        ColorType::Rgba,
    )?;
    Ok(jpeg)
}

/// Named rather than left to the crate (which happens to default to the
/// same thing today), because it is a choice with a number behind it. One
/// 3840x2160 still is 33 MB of pixels, measured on this box over a real
/// captured frame: `Fast` 53 ms for 8.5 MB, `Default` 1.6 s for 6.8 MB,
/// `Best` 6.3 s for 6.4 MB. A paste the pilot waits seconds for is a paste
/// they ask for twice.
const COMPRESSION: png::Compression = png::Compression::Fast;

fn png(shot: &Shot) -> Fallible<Vec<u8>> {
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
        .unwrap_or("kjerag")
}

/// `<video>_<timecode>.jpg`: which video, and where in it.
fn name(stem: &str, at: Duration) -> String {
    let millis = at.as_millis();
    format!(
        "{stem}_{:02}-{:02}-{:02}.{:03}.jpg",
        millis / 3_600_000,
        millis / 60_000 % 60,
        millis / 1_000 % 60,
        millis % 1_000,
    )
}

/// Reserve and write the first free name. Checking existence before writing
/// races other captures and follows dangling symlinks; exclusive creation does
/// neither. A write failure is returned without deleting a path that another
/// process could already have replaced. This does not promise atomic publication
/// of the complete JPEG: a failed write may leave our newly created partial file.
fn write_new(folder: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let (stem, extension) = name.rsplit_once('.').unwrap_or((name, "jpg"));
    let mut number = 1u64;
    loop {
        let path = folder.join(match number {
            1 => name.to_owned(),
            _ => format!("{stem}-{number}.{extension}"),
        });
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(bytes)?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let Some(next) = number.checked_add(1) else {
                    return Err(error);
                };
                number = next;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Set the real save destination in a child rather than changing the
    /// environment of parallel tests. Only synthetic images live here; the
    /// directory is newly created inside this checkout's ignored scratch area.
    fn isolated_save_folder(test: &str) -> Option<PathBuf> {
        const CHILD: &str = "KJERAG_TEST_SAVE_CHILD";
        if std::env::var(CHILD).as_deref() == Ok(test) {
            return Some(std::env::var_os("XDG_SCREENSHOTS_DIR").unwrap().into());
        }
        struct Folder(PathBuf);
        impl Drop for Folder {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch/frame-save-tests");
        fs::create_dir_all(&root).unwrap();
        let path = root.join(format!("{test}-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let folder = Folder(path);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("shot::tests::{test}"), "--nocapture"])
            .env(CHILD, test)
            .env("XDG_SCREENSHOTS_DIR", folder.0.join("Screenshots"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        None
    }

    #[test]
    fn saving_after_a_thousand_name_collisions_keeps_every_old_capture() {
        let Some(folder) =
            isolated_save_folder("saving_after_a_thousand_name_collisions_keeps_every_old_capture")
        else {
            return;
        };
        fs::create_dir_all(&folder).unwrap();
        let original = name("flight", Duration::ZERO);
        fs::write(folder.join(&original), b"previous capture").unwrap();
        for number in 2..=1_000 {
            fs::write(
                folder.join(format!("flight_00-00-00.000-{number}.jpg")),
                b"previous capture",
            )
            .unwrap();
        }
        let Done::Saved(path) =
            finish(&shot(), Path::new("flight.insv"), Destination::Save).unwrap()
        else {
            panic!("save returned a clipboard image");
        };
        assert_eq!(
            fs::read(folder.join(&original)).unwrap(),
            b"previous capture"
        );
        assert_eq!(path, folder.join("flight_00-00-00.000-1001.jpg"));
        assert_eq!(fs::read(path).unwrap(), jpeg(&shot()).unwrap());
        for number in 2..=1_000 {
            assert_eq!(
                fs::read(folder.join(format!("flight_00-00-00.000-{number}.jpg"))).unwrap(),
                b"previous capture"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_capture_symlink_is_not_followed_or_replaced() {
        let Some(folder) =
            isolated_save_folder("a_dangling_capture_symlink_is_not_followed_or_replaced")
        else {
            return;
        };
        fs::create_dir_all(&folder).unwrap();
        let original = folder.join(name("flight", Duration::ZERO));
        let target = folder.join("do-not-create.jpg");
        std::os::unix::fs::symlink(&target, &original).unwrap();
        let Done::Saved(path) =
            finish(&shot(), Path::new("flight.insv"), Destination::Save).unwrap()
        else {
            panic!("save returned a clipboard image");
        };
        assert!(!target.exists(), "saving followed the existing symlink");
        assert_eq!(fs::read_link(&original).unwrap(), target);
        assert_eq!(path, folder.join("flight_00-00-00.000-2.jpg"));
        assert_eq!(fs::read(path).unwrap(), jpeg(&shot()).unwrap());
    }

    /// The question a still has to answer months later: which video, and
    /// which moment.
    #[test]
    fn a_name_carries_the_video_and_the_timecode() {
        assert_eq!(
            name("VID_20260410_185407_00_004", Duration::from_millis(754_321)),
            "VID_20260410_185407_00_004_00-12-34.321.jpg"
        );
        assert_eq!(
            name("flight", Duration::from_secs(3661)),
            "flight_01-01-01.000.jpg"
        );
        assert_eq!(name("flight", Duration::ZERO), "flight_00-00-00.000.jpg");
    }

    /// Over an hour is hours, not minutes: a 90-minute flight is one file.
    #[test]
    fn the_timecode_carries_hours() {
        assert_eq!(name("f", Duration::from_secs(5400)), "f_01-30-00.000.jpg");
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
        let Some(folder) = isolated_save_folder("a_taken_name_gets_a_number") else {
            return;
        };
        fs::create_dir_all(&folder).unwrap();
        let taken = ["f_00-00-00.000.jpg", "f_00-00-00.000-2.jpg"];
        for name in taken {
            fs::write(folder.join(name), b"previous capture").unwrap();
        }
        let Done::Saved(path) = finish(&shot(), Path::new("f.insv"), Destination::Save).unwrap()
        else {
            panic!("save returned a clipboard image");
        };
        assert_eq!(path, folder.join("f_00-00-00.000-3.jpg"));
        assert_eq!(fs::read(path).unwrap(), jpeg(&shot()).unwrap());
        for name in taken {
            assert_eq!(fs::read(folder.join(name)).unwrap(), b"previous capture");
        }
    }

    /// The suffix goes before the extension, not after it: a `.jpg-2` is
    /// not a picture to anything that reads names.
    #[test]
    fn the_number_keeps_the_extension_last() {
        let Some(folder) = isolated_save_folder("the_number_keeps_the_extension_last") else {
            return;
        };
        fs::create_dir_all(&folder).unwrap();
        assert_eq!(
            write_new(&folder, "f.jpg", b"first").unwrap(),
            folder.join("f.jpg")
        );
        let second = write_new(&folder, "f.jpg", b"second").unwrap();
        assert_eq!(second, folder.join("f-2.jpg"));
        assert_eq!(fs::read(folder.join("f.jpg")).unwrap(), b"first");
        assert_eq!(fs::read(second).unwrap(), b"second");
    }

    #[test]
    fn concurrent_captures_each_keep_their_own_jpeg() {
        let Some(folder) = isolated_save_folder("concurrent_captures_each_keep_their_own_jpeg")
        else {
            return;
        };
        let barrier = std::sync::Barrier::new(8);
        let saved = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|color| {
                    let barrier = &barrier;
                    scope.spawn(move || {
                        let mut shot = shot();
                        for pixel in shot.rgba.chunks_exact_mut(4) {
                            pixel[1] = color * 30;
                        }
                        let expected = jpeg(&shot).unwrap();
                        barrier.wait();
                        let Done::Saved(path) =
                            finish(&shot, Path::new("flight.insv"), Destination::Save).unwrap()
                        else {
                            panic!("save returned a clipboard image");
                        };
                        (path, expected)
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        let mut paths = std::collections::BTreeSet::new();
        for (path, expected) in saved {
            assert_eq!(fs::read(&path).unwrap(), expected);
            assert!(paths.insert(path), "two saves returned the same path");
        }
        assert!(paths.contains(&folder.join("flight_00-00-00.000.jpg")));
        for number in 2..=8 {
            assert!(paths.contains(&folder.join(format!("flight_00-00-00.000-{number}.jpg"))));
        }
    }

    #[test]
    fn encoding_failure_creates_no_folder_or_file() {
        let Some(folder) = isolated_save_folder("encoding_failure_creates_no_folder_or_file")
        else {
            return;
        };
        let mut shot = shot();
        shot.width = 65_536;
        let error = finish(&shot, Path::new("flight.insv"), Destination::Save).unwrap_err();
        assert_eq!(error.to_string(), "a 65536 px edge is past JPEG's 65535");
        assert!(!folder.exists());
    }

    #[test]
    fn noncollision_file_errors_are_returned_unchanged() {
        let Some(folder) = isolated_save_folder("noncollision_file_errors_are_returned_unchanged")
        else {
            return;
        };
        fs::write(&folder, b"not a directory").unwrap();
        let expected = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(folder.join("f.jpg"))
            .unwrap_err();
        let actual = write_new(&folder, "f.jpg", b"capture").unwrap_err();
        assert_eq!(actual.kind(), expected.kind());
        assert_eq!(actual.raw_os_error(), expected.raw_os_error());
        assert_eq!(actual.to_string(), expected.to_string());
        assert_eq!(fs::read(&folder).unwrap(), b"not a directory");
    }

    /// A capture small enough to encode in a test. The encoders below run
    /// over every byte, and a real capture is 33 MB of them.
    fn shot() -> Shot {
        let mut rgba = Vec::with_capacity(16 * 16 * 4);
        for pixel in 0..16 * 16u32 {
            rgba.extend_from_slice(&[pixel as u8, 40, 200, 255]);
        }
        Shot {
            width: 16,
            height: 16,
            rgba,
            index: 0,
            time: Duration::ZERO,
        }
    }

    /// The file the pilot double clicks is a JPEG, start to end marker.
    #[test]
    fn a_saved_still_is_a_jpeg() {
        let bytes = jpeg(&shot()).unwrap();
        assert_eq!(bytes[..2], [0xFF, 0xD8], "no start of image");
        assert_eq!(bytes[bytes.len() - 2..], [0xFF, 0xD9], "no end of image");
    }

    /// And its chroma planes are the size of its luma plane. This is what
    /// [`SAMPLING`] buys, read back out of the frame header the decoder
    /// reads: three components, each with sampling factors of one by one.
    #[test]
    fn a_saved_still_keeps_its_chroma() {
        let bytes = jpeg(&shot()).unwrap();
        let frame = bytes
            .windows(2)
            .position(|marker| marker == [0xFF, 0xC0])
            .expect("a baseline frame header");
        assert_eq!(bytes[frame + 9], 3, "not three components");
        for component in 0..3 {
            assert_eq!(bytes[frame + 11 + component * 3], 0x11, "{component}");
        }
    }

    /// The clipboard is untouched by the format the file takes: cosmic-files
    /// reads `image/png` off it, and what this hands over is a PNG.
    #[test]
    fn the_clipboard_still_gets_a_png() {
        assert_eq!(
            png(&shot()).unwrap()[..8],
            [137, 80, 78, 71, 13, 10, 26, 10]
        );
        assert_eq!(Png(Vec::new()).available().as_ref(), [MIME.to_owned()]);
    }
}
