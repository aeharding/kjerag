//! What a file dropped on the window arrives as.
//!
//! cosmic-player implements no drag and drop at all, so the precedent is
//! cosmic-files' (`src/clipboard.rs:108-160`): a small type that names the
//! mime types it can read, handed to `dnd_destination_for_data`, which does
//! the offer dance and calls [`TryFrom`] on the bytes.
//!
//! `application/vnd.portal.filetransfer`, which is how the portal hands over
//! files from a sandboxed source, is deliberately not handled: it is not a
//! payload but a key that has to be exchanged for paths over D-Bus, and
//! nothing about this app is sandboxed yet. docs/UI.md hoped it would come
//! for free; it does not.

use std::borrow::Cow;
use std::path::PathBuf;

use cosmic::iced::clipboard::mime::AllowedMimeTypes;
use url::Url;

const URI_LIST: &str = "text/uri-list";

/// The local files a drop carried, in the order the source listed them.
#[derive(Clone, Debug)]
pub struct Dropped(pub Vec<PathBuf>);

impl AllowedMimeTypes for Dropped {
    fn allowed() -> Cow<'static, [String]> {
        Cow::Owned(vec![URI_LIST.to_owned()])
    }
}

impl TryFrom<(Vec<u8>, String)> for Dropped {
    type Error = String;

    fn try_from((data, mime): (Vec<u8>, String)) -> Result<Self, Self::Error> {
        if mime != URI_LIST {
            return Err(format!("dropped as {mime}, which this app cannot read"));
        }
        let text = std::str::from_utf8(&data).map_err(|e| e.to_string())?;
        text.lines().filter(is_uri).map(local).collect()
    }
}

impl FromIterator<PathBuf> for Dropped {
    fn from_iter<T: IntoIterator<Item = PathBuf>>(paths: T) -> Self {
        Self(paths.into_iter().collect())
    }
}

/// A `text/uri-list` may carry comment lines and blank ones (RFC 2483).
fn is_uri(line: &&str) -> bool {
    !line.trim().is_empty() && !line.starts_with('#')
}

/// We decode files, not streams, so a drop that is not a local file is one
/// this app cannot do anything with.
fn local(line: &str) -> Result<PathBuf, String> {
    let url = Url::parse(line.trim()).map_err(|e| format!("{line}: {e}"))?;
    url.to_file_path()
        .map_err(|()| format!("{url} is not a local file"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dropped(list: &str) -> Result<Dropped, String> {
        Dropped::try_from((list.as_bytes().to_vec(), URI_LIST.to_owned()))
    }

    /// What a file manager sends: CRLF line endings, and often more than one
    /// file. The caller takes the first; this only has to deliver them in
    /// order.
    #[test]
    fn a_uri_list_becomes_paths() {
        let list = "#comment\r\nfile:///home/pilot/a%20b.insv\r\nfile:///tmp/c.insv\r\n";
        assert_eq!(
            dropped(list).unwrap().0,
            [
                PathBuf::from("/home/pilot/a b.insv"),
                PathBuf::from("/tmp/c.insv")
            ]
        );
    }

    #[test]
    fn a_remote_drop_is_refused_rather_than_guessed_at() {
        assert!(dropped("https://example.invalid/a.insv").is_err());
        assert!(dropped("/home/pilot/a.insv").is_err());
    }

    #[test]
    fn an_unexpected_mime_type_is_refused() {
        assert!(Dropped::try_from((b"whatever".to_vec(), "text/plain".to_owned())).is_err());
    }
}
