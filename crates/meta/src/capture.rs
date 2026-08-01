//! Which files one capture is made of, and what is honestly known about the
//! ones that are not there (issue #123).
//!
//! [`sibling`](super::sibling) answers "is the other lens on disk", which is
//! all a reader needs: it either opens two files or one. Saying anything to
//! the pilot needs a third answer, because "the mate is not there" and "I
//! cannot see the folder it would be in" look identical from a failed
//! `is_file` and mean opposite things. A Flatpak makes the difference
//! routine: a file picked in the chooser can arrive as
//! `/run/user/<uid>/doc/<id>/<name>`, a directory holding that one file and
//! nothing else, and claiming the capture's other half is missing on that
//! evidence would be a lie about the pilot's card.
//!
//! So [`resolve`] separates them, and the caller decides what to say.
//!
//! One thing it deliberately does not do is decide how many files a capture
//! has. Today the rule names one other file, the lens mate; issue #127's
//! split recordings name several, sequential chunks of one flight. That is
//! why this hands back a list and a set of absences rather than an `Option`
//! of a mate: the shape does not change when the rule learns to count.

use std::path::{Path, PathBuf};

use crate::pair;

/// A capture, as far as the filesystem was willing to say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    /// The files that are there and can be opened, the picked one first.
    /// Never empty: the picked file is always in it, because it was picked.
    pub files: Vec<PathBuf>,
    /// What became of the rest.
    pub missing: Missing,
}

/// What is known about the files the naming rule named and the filesystem did
/// not produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Missing {
    /// Everything the rule names is in [`Capture::files`]. An X4-class file,
    /// whose two lenses are two streams of the one file, ends here too: the
    /// rule names a mate, nothing is on disk under that name, and nothing is
    /// missing because that camera never wrote one.
    Nothing,
    /// The directory beside the picked file could be listed and these were
    /// not in it.
    ///
    /// That is a fact about **that directory**, not about the capture. Whether
    /// the directory is the capture's own is the caller's to know: a document
    /// portal path lists exactly one file whatever is on the card behind it.
    NotBeside(Vec<PathBuf>),
    /// The directory could not be listed at all, so nothing is known about
    /// what is beside the picked file. Outside a sandbox this is a permission
    /// or a vanished mount; inside one it is the ordinary answer for a path
    /// no grant covers.
    Unreadable,
}

/// What the picked file's capture is made of.
///
/// The picked file is taken as given: this does not check that it exists,
/// because whoever picked it is about to open it and will find out better
/// than a `stat` here would.
pub fn resolve(picked: &Path) -> Capture {
    let files = vec![picked.to_path_buf()];
    let named = named_members(picked);
    if named.is_empty() {
        return Capture {
            files,
            missing: Missing::Nothing,
        };
    }
    let Some(directory) = picked.parent() else {
        return Capture {
            files,
            missing: Missing::Nothing,
        };
    };
    // The listing is the question, not the shortcut. `is_file` on a member
    // says false for a directory it cannot see and for a member that is not
    // there, and those are the two answers this exists to tell apart.
    if std::fs::read_dir(directory).is_err() {
        return Capture {
            files,
            missing: Missing::Unreadable,
        };
    }
    gather(files, named)
}

/// Split the named members into the ones on disk and the ones not, given a
/// directory that has already answered that it can be listed.
fn gather(mut files: Vec<PathBuf>, named: Vec<PathBuf>) -> Capture {
    let mut absent = Vec::new();
    for member in named {
        match member.is_file() {
            true => files.push(member),
            false => absent.push(member),
        }
    }
    let missing = match absent.is_empty() {
        true => Missing::Nothing,
        false => Missing::NotBeside(absent),
    };
    Capture { files, missing }
}

/// The other files this capture's name says it is made of, whether or not any
/// of them exist. One today, the lens mate.
fn named_members(picked: &Path) -> Vec<PathBuf> {
    pair::sibling_path(picked).into_iter().collect()
}

/// Which of these files, if any, is the picked file's other lens, by name
/// alone.
///
/// The names are compared and the directories are not, which is the whole
/// point of it: this is the case where the pilot picked both halves in the
/// chooser and each came back as a document of its own, in a directory of its
/// own, with nothing beside it. Two files the naming rule pairs are two halves
/// of one capture wherever they now sit.
///
/// Agreeing names is a claim about the capture and not a promise about the
/// pixels. The reader still opens the candidate and checks that its shape
/// pairs with the first before it reads a frame out of it.
pub fn mate_among<'a>(picked: &Path, candidates: &'a [PathBuf]) -> Option<&'a Path> {
    let named = pair::sibling_path(picked)?;
    let wanted = named.file_name()?;
    candidates
        .iter()
        .find(|candidate| candidate.file_name() == Some(wanted))
        .map(PathBuf::as_path)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// A directory of this test's own, under the system temporary directory,
    /// removed when the test is done with it. `kjerag-meta` takes no
    /// dependency to get one: the crate builds on a box with nothing
    /// installed and a test helper is not the thing to break that with.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(what: &str) -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let at = std::env::temp_dir().join(format!(
                "kjerag-capture-{}-{what}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&at).expect("a temporary directory");
            Self(at)
        }

        fn file(&self, name: &str) -> PathBuf {
            let at = self.0.join(name);
            fs::write(&at, b"not really an insv").expect("a file");
            at
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The bar: both files of a two-file capture, found with nothing said.
    #[test]
    fn a_pair_on_disk_is_the_whole_capture() {
        let scratch = Scratch::new("pair");
        let lens0 = scratch.file("VID_20000101_100000_00_001.insv");
        let lens1 = scratch.file("VID_20000101_100000_10_001.insv");
        let capture = resolve(&lens0);
        assert_eq!(capture.files, vec![lens0, lens1]);
        assert_eq!(capture.missing, Missing::Nothing);
    }

    /// Either file of the pair resolves to the same capture, because the
    /// pilot picks whichever one the file manager showed first.
    #[test]
    fn the_second_lens_finds_the_first() {
        let scratch = Scratch::new("second");
        let lens0 = scratch.file("VID_20000101_100000_00_001.insv");
        let lens1 = scratch.file("VID_20000101_100000_10_001.insv");
        let capture = resolve(&lens1);
        assert_eq!(capture.files, vec![lens1, lens0]);
        assert_eq!(capture.missing, Missing::Nothing);
    }

    /// The mate really is not there, in a directory that answered. This is
    /// the one case that has earned a word to the pilot.
    #[test]
    fn a_lens_alone_in_a_readable_directory_is_absent() {
        let scratch = Scratch::new("alone");
        let lens0 = scratch.file("VID_20000101_100000_00_001.insv");
        let capture = resolve(&lens0);
        assert_eq!(capture.files, vec![lens0.clone()]);
        assert_eq!(
            capture.missing,
            Missing::NotBeside(vec![
                lens0.with_file_name("VID_20000101_100000_10_001.insv")
            ])
        );
    }

    /// A directory that cannot be listed says nothing about the capture, and
    /// this is the answer a document portal path produces outside a sandbox
    /// too: the picked file's folder is not there to read.
    #[test]
    fn a_directory_that_cannot_be_listed_is_not_evidence_of_anything() {
        let capture = resolve(Path::new("/nowhere/at/all/VID_20000101_100000_00_001.insv"));
        assert_eq!(capture.files.len(), 1);
        assert_eq!(capture.missing, Missing::Unreadable);
    }

    /// An X4-class capture is one file with two streams. Its name carries a
    /// marker, so the rule names a mate; nothing is on disk under that name
    /// and nothing is missing, because that camera never wrote one. Saying
    /// "half your capture is gone" here would be the loudest possible bug.
    #[test]
    fn a_file_whose_name_names_no_mate_is_whole() {
        let scratch = Scratch::new("whole");
        let holiday = scratch.file("holiday.insv");
        let capture = resolve(&holiday);
        assert_eq!(capture.files, vec![holiday]);
        assert_eq!(capture.missing, Missing::Nothing);
    }

    /// The proxy beside a pair is nobody's lens, and picking it is not half a
    /// capture.
    #[test]
    fn the_lrv_proxy_is_a_capture_of_its_own() {
        let scratch = Scratch::new("proxy");
        scratch.file("VID_20000101_100000_00_001.insv");
        let proxy = scratch.file("LRV_20000101_100000_11_001.insv");
        let capture = resolve(&proxy);
        assert_eq!(capture.files, vec![proxy]);
        assert_eq!(capture.missing, Missing::Nothing);
    }

    /// Two halves picked together are one capture even when each is in a
    /// directory of its own, which is what the document portal does to a
    /// multiple pick: one directory per file, nothing beside either.
    #[test]
    fn a_mate_is_found_among_picked_files_wherever_they_sit() {
        let picked = Path::new("/run/user/1000/doc/aaaa/VID_20000101_100000_00_001.insv");
        let candidates = [
            PathBuf::from("/run/user/1000/doc/aaaa/VID_20000101_100000_00_001.insv"),
            PathBuf::from("/run/user/1000/doc/bbbb/VID_20000101_100000_10_001.insv"),
        ];
        assert_eq!(
            mate_among(picked, &candidates),
            Some(candidates[1].as_path())
        );
        // And the other way round, because the pilot picks in any order.
        assert_eq!(
            mate_among(&candidates[1], &candidates),
            Some(candidates[0].as_path())
        );
    }

    /// Another capture's file is not this one's other half, however it was
    /// picked. A pilot who selects a whole folder gets the capture he clicked
    /// and not a stitched-together one.
    #[test]
    fn another_captures_file_is_not_a_mate() {
        let picked = Path::new("/doc/a/VID_20000101_100000_00_001.insv");
        let candidates = [
            PathBuf::from("/doc/b/VID_20000101_110000_10_002.insv"),
            PathBuf::from("/doc/c/LRV_20000101_100000_11_001.insv"),
        ];
        assert_eq!(mate_among(picked, &candidates), None);
        assert_eq!(mate_among(picked, &[]), None);
        assert_eq!(mate_among(Path::new("holiday.insv"), &candidates), None);
    }

    /// One capture's file does not pull in another capture's, which is what
    /// keeps a folder of a whole afternoon from resolving to one enormous
    /// set.
    #[test]
    fn a_folder_of_captures_resolves_one_of_them() {
        let scratch = Scratch::new("folder");
        let lens0 = scratch.file("VID_20000101_100000_00_001.insv");
        let lens1 = scratch.file("VID_20000101_100000_10_001.insv");
        scratch.file("VID_20000101_110000_00_002.insv");
        scratch.file("VID_20000101_110000_10_002.insv");
        let capture = resolve(&lens0);
        assert_eq!(capture.files, vec![lens0, lens1]);
        assert_eq!(capture.missing, Missing::Nothing);
    }
}
