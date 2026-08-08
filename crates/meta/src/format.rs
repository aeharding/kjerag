//! What a file is, read off its bytes, before anything tries to play it
//! (issue #107).
//!
//! Kjerag plays Insta360 `.insv` and nothing else, and the other 360 cameras
//! write MP4s too. Handed one, ffmpeg opens it, the trailer read finds no
//! trailer, and the pilot gets the line a corrupt file gets. This module is
//! the step in front of that: name the maker from the container, so the
//! refusal can say which format the file is.
//!
//! **Structure, not a byte search.** Every signature below is a box at a
//! known place in the box tree, never a string anywhere in the file. A raw
//! grep for `st3d` over the sample corpus hits two genuine Insta360 captures,
//! both inside `mdat` (one at offset 493,930,231 of a 592 MB X3 rear file,
//! measured 2026-08-01), so a file the pilot's own camera wrote would be
//! refused as somebody else's.
//!
//! What each signature is, and where it was measured (`~/Videos/samples`,
//! 2026-08-01, 18 files from seven cameras):
//!
//! - **Insta360**: the trailer magic on the last 32 bytes. Checked first, so
//!   a capture that carries it plays whatever it is named.
//! - **GoPro**: `moov/udta` holds GoPro's own boxes, `FIRM` `GPMF` `CAME`
//!   `MUID`. Present in all six GoPro files here: a Max `.360`, its `.LRV`
//!   proxy, a Max hero-mode `.MP4`, two of GoPro's own published samples, and
//!   a Fusion `.mp4`.
//! - **DJI**: a track whose sample entry is `djmd` or `dbgi`, DJI's telemetry
//!   and debug tracks. The Osmo 360 `.OSV` writes two of each; its handler
//!   names and `©too` tag say `CAM` and `Osmo 360`, which are weaker. That
//!   arm refused until the `djmd` calibration could be read, and now it
//!   accepts: [`super::osmo`] is what reads it.
//! - **Spherical**: Google's spherical metadata, `st3d`/`sv3d` in the video
//!   sample entry (v2) or the `GSpherical` `uuid` box on a track (v1). That
//!   is a stitched equirectangular MP4, which is what an export from any of
//!   these cameras' desktop apps is. **Unlike the three above, this arm has
//!   never been run against a real file**: no spherical MP4 exists in the
//!   corpus and ffmpeg 7.1 has no way to write one, so it is built from the
//!   published layout and tested on hand-built boxes only.
//!
//! A capture with no signature at all is left alone. The rear file of an
//! X2-class pair is exactly that (no trailer, no maker's boxes), and it is a
//! file Kjerag plays: [`super::sibling`] finds its front half.

use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use super::trailer::MAGIC;

/// What a file turned out to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// An Insta360 capture, by the magic on the end of it.
    Insta360,
    /// A DJI capture carrying a `djmd` telemetry track, which on an Osmo 360
    /// is where the lens calibration lives ([`super::osmo`]).
    Osmo,
    /// A 360 format Kjerag does not read, which is refused by name.
    Foreign(Foreign),
    /// Nothing this recognizes, which includes the second file of an
    /// Insta360 pair. Left to the open to accept or refuse.
    Unknown,
}

/// A 360 format that is somebody else's, as far as the container can tell.
///
/// This is the error a refused open carries, so that the shell can say which
/// one it was by matching a type rather than a message, the way it tells a
/// missing decoder apart today. What the pilot reads is the shell's copy;
/// what is below is the terminal's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Foreign {
    /// GoPro: a `.360` off a Max, or any other capture off a GoPro.
    GoPro,
    /// An MP4 carrying spherical metadata: stitched equirectangular video,
    /// not the raw dual fisheye Kjerag reprojects.
    Spherical,
}

impl fmt::Display for Foreign {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self {
            Self::GoPro => "a GoPro capture",
            Self::Spherical => "a stitched 360 video",
        };
        write!(f, "{what}, not an Insta360 .insv")
    }
}

impl std::error::Error for Foreign {}

impl Format {
    /// What the file at `path` is. Content decides; the name is only asked
    /// when the content said nothing.
    ///
    /// Never an error: a file that cannot be read is not a file this can
    /// name, and the open that follows says so in its own words. The cost on
    /// an `.insv` is one seek and 32 bytes.
    pub fn sniff(path: &Path) -> Self {
        let content = File::open(path)
            .and_then(|mut file| read(&mut file))
            .unwrap_or(Self::Unknown);
        match content {
            // A `.360` that is not a GoPro file, or an `.osv` that is not
            // DJI's, is still a file whose name says whose it is: the maker
            // wrote the container differently, or somebody renamed it. Naming
            // it beats the line a broken file gets, and the trailer check
            // above means no Insta360 capture can land here.
            Self::Unknown => named(path).unwrap_or(Self::Unknown),
            found => found,
        }
    }
}

/// `moov` is metadata: 104 KB on the GoPro Max capture measured and 130 KB on
/// the DJI one. This is where a file stops being read into memory and starts
/// being left unnamed, which costs the pilot a specific message and nothing
/// else.
const MOOV_LIMIT: u64 = 64 << 20;

/// `moov/trak/mdia/minf/stbl/stsd/<entry>/sv3d/st3d` is nine levels, so the
/// walk stops one past the deepest thing it looks for. A file cannot spin it
/// deeper than this by nesting.
const DEPTH: u8 = 10;

/// The Google spherical v1 `uuid`, from the RFC published with the metadata
/// injector: `ffcc8263-f855-4a93-8814-587a02521fdd`.
const GSPHERICAL: [u8; 16] = [
    0xff, 0xcc, 0x82, 0x63, 0xf8, 0x55, 0x4a, 0x93, 0x88, 0x14, 0x58, 0x7a, 0x02, 0x52, 0x1d, 0xdd,
];

/// A `VisualSampleEntry`'s own boxes start after its 78 fixed bytes: 8 of
/// `SampleEntry`, then 70 of resolution, depth and the rest (ISO 14496-12).
/// A `djmd` entry is shorter than that, which is what keeps this from
/// reading one as a container.
const VISUAL_ENTRY: usize = 78;

/// The bytes half of [`Format::sniff`], over anything that reads and seeks so
/// the tests need no files.
fn read<S: Read + Seek>(source: &mut S) -> io::Result<Format> {
    let len = source.seek(SeekFrom::End(0))?;
    if insta360(source, len)? {
        return Ok(Format::Insta360);
    }
    let Some(moov) = moov(source, len)? else {
        return Ok(Format::Unknown);
    };
    Ok(search(&moov, DEPTH, Parent::Anything).unwrap_or(Format::Unknown))
}

/// The trailer footer's magic on the last 32 bytes (`super::trailer`).
fn insta360<S: Read + Seek>(source: &mut S, len: u64) -> io::Result<bool> {
    let Some(at) = len.checked_sub(MAGIC.len() as u64) else {
        return Ok(false);
    };
    source.seek(SeekFrom::Start(at))?;
    let mut tail = [0u8; 32];
    source.read_exact(&mut tail)?;
    Ok(tail.as_slice() == MAGIC)
}

/// The `moov` box's payload, found by walking the top-level boxes.
///
/// It is walked rather than looked for at the front: every camera in the
/// corpus writes `mdat` first and `moov` after it, which is what a recorder
/// that cannot know the file's length ahead of time has to do.
pub(crate) fn moov<S: Read + Seek>(source: &mut S, len: u64) -> io::Result<Option<Vec<u8>>> {
    let mut at = 0;
    while at + 8 <= len {
        source.seek(SeekFrom::Start(at))?;
        let mut header = [0u8; 8];
        source.read_exact(&mut header)?;
        let (size, kind, header_len) = match u32::from_be_bytes(header[..4].try_into().unwrap()) {
            // 1 is the escape to a 64-bit size, in eight bytes of its own.
            1 => {
                let mut large = [0u8; 8];
                source.read_exact(&mut large)?;
                (u64::from_be_bytes(large), &header[4..8], 16)
            }
            // 0 is "to the end of the file", which only the last box may say.
            0 => (len - at, &header[4..8], 8),
            size => (u64::from(size), &header[4..8], 8),
        };
        if size < header_len || at + size > len {
            return Ok(None);
        }
        if kind == b"moov" {
            let body = size - header_len;
            if body > MOOV_LIMIT {
                return Ok(None);
            }
            let mut moov = vec![0u8; body as usize];
            source.read_exact(&mut moov)?;
            return Ok(Some(moov));
        }
        at += size;
    }
    Ok(None)
}

/// What a box's own 4cc means depends on the box it sits in: `FIRM` is
/// GoPro's only inside `udta`, and `djmd` is DJI's only as a sample entry.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Parent {
    Anything,
    Udta,
    Stsd,
}

/// The first signature in this subtree, depth first.
fn search(body: &[u8], depth: u8, parent: Parent) -> Option<Format> {
    if depth == 0 {
        return None;
    }
    for (kind, payload) in Boxes::new(body) {
        if let Some(found) = signature(kind, payload, parent) {
            return Some(found);
        }
        if let Some((inner, parent)) = inside(kind, payload, parent)
            && let Some(found) = search(inner, depth - 1, parent)
        {
            return Some(found);
        }
    }
    None
}

/// One box, read as a maker's mark or not at all.
fn signature(kind: &[u8; 4], payload: &[u8], parent: Parent) -> Option<Format> {
    match (parent, kind) {
        // The camera's firmware string, its GPMF telemetry, its serial and
        // its media id. Any one of them is GoPro's `udta` and nobody else's.
        (Parent::Udta, b"FIRM" | b"GPMF" | b"CAME" | b"MUID") => {
            Some(Format::Foreign(Foreign::GoPro))
        }
        (Parent::Stsd, b"djmd" | b"dbgi") => Some(Format::Osmo),
        (_, b"st3d" | b"sv3d") => Some(Format::Foreign(Foreign::Spherical)),
        (_, b"uuid") if payload.starts_with(&GSPHERICAL) => {
            Some(Format::Foreign(Foreign::Spherical))
        }
        _ => None,
    }
}

/// The children of a box that has any, and what they are children of.
fn inside<'a>(kind: &[u8; 4], payload: &'a [u8], parent: Parent) -> Option<(&'a [u8], Parent)> {
    match kind {
        b"trak" | b"mdia" | b"minf" | b"stbl" => Some((payload, Parent::Anything)),
        b"udta" => Some((payload, Parent::Udta)),
        // Both are full boxes: `meta` carries a version and flags before its
        // children, `stsd` carries an entry count after those.
        b"meta" => Some((payload.get(4..)?, Parent::Anything)),
        b"stsd" => Some((payload.get(8..)?, Parent::Stsd)),
        // A sample entry, whose own boxes are where `sv3d` lives.
        _ if parent == Parent::Stsd => Some((payload.get(VISUAL_ENTRY..)?, Parent::Anything)),
        _ => None,
    }
}

/// The boxes of one container, in order, stopping at the first one whose
/// header does not fit what is left.
pub(crate) struct Boxes<'a> {
    body: &'a [u8],
    at: usize,
}

impl<'a> Boxes<'a> {
    pub(crate) fn new(body: &'a [u8]) -> Self {
        Self { body, at: 0 }
    }
}

impl<'a> Iterator for Boxes<'a> {
    type Item = (&'a [u8; 4], &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let header: &[u8; 8] = self.body.get(self.at..self.at + 8)?.try_into().ok()?;
        let kind: &[u8; 4] = header[4..].try_into().ok()?;
        let (size, header_len) = match u32::from_be_bytes(header[..4].try_into().unwrap()) {
            1 => {
                let large: [u8; 8] = self.body.get(self.at + 8..self.at + 16)?.try_into().ok()?;
                (u64::from_be_bytes(large) as usize, 16)
            }
            0 => (self.body.len() - self.at, 8),
            size => (size as usize, 8),
        };
        let end = self.at.checked_add(size)?;
        if size < header_len || end > self.body.len() {
            return None;
        }
        let payload = &self.body[self.at + header_len..end];
        self.at = end;
        Some((kind, payload))
    }
}

/// The maker a file name claims, for the file whose bytes claimed nothing.
fn named(path: &Path) -> Option<Format> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "360" => Some(Format::Foreign(Foreign::GoPro)),
        "osv" => Some(Format::Osmo),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    /// One box: `size u32 | kind | payload`.
    fn mp4box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = (8 + payload.len()) as u32;
        let mut out = size.to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    fn nest(kinds: &[&[u8; 4]], inner: Vec<u8>) -> Vec<u8> {
        kinds
            .iter()
            .rev()
            .fold(inner, |body, kind| mp4box(kind, &body))
    }

    /// A file the way every camera in the corpus writes one: the pictures
    /// first and the index after them.
    fn file(mdat: &[u8], moov: Vec<u8>) -> Vec<u8> {
        let mut out = mp4box(b"ftyp", b"mp41\0\0\0\0mp41");
        out.extend(mp4box(b"mdat", mdat));
        out.extend(moov);
        out
    }

    /// `stsd` is a full box: version and flags, then the entry count.
    fn stsd(entries: &[Vec<u8>]) -> Vec<u8> {
        let mut body = vec![0, 0, 0, 0];
        body.extend((entries.len() as u32).to_be_bytes());
        for entry in entries {
            body.extend_from_slice(entry);
        }
        mp4box(b"stsd", &body)
    }

    /// A video sample entry: 78 fixed bytes, then the boxes that describe it.
    fn visual(kind: &[u8; 4], children: Vec<u8>) -> Vec<u8> {
        let mut body = vec![0u8; VISUAL_ENTRY - 8];
        body.extend(children);
        mp4box(kind, &body)
    }

    /// The bytes half of a sniff, with [`Format::sniff`]'s own answer to a
    /// read that failed: a file that ends in the middle of a box header is
    /// not a file this names.
    fn sniffed(bytes: &[u8]) -> Format {
        read(&mut Cursor::new(bytes)).unwrap_or(Format::Unknown)
    }

    /// GoPro's `udta`, as all six GoPro files in the corpus write it. The
    /// maker is in the container rather than in the extension, so a Max
    /// `.360` and a hero-mode `.MP4` are the same answer.
    #[test]
    fn gopros_own_boxes_name_gopro() {
        for kind in [b"FIRM", b"GPMF", b"CAME", b"MUID"] {
            let udta = mp4box(b"udta", &mp4box(kind, b"H19.03.02.00.75"));
            let bytes = file(b"pictures", mp4box(b"moov", &udta));
            assert_eq!(sniffed(&bytes), Format::Foreign(Foreign::GoPro), "{kind:?}");
        }
    }

    /// DJI's telemetry track, as the Osmo 360 `.OSV` writes it: the maker is
    /// the sample entry's own format, because the handler names on that file
    /// are the generic ones ffmpeg writes for anybody. It is a file Kjerag
    /// plays, and [`super::super::osmo`] is what reads the calibration out of
    /// that track.
    #[test]
    fn djis_own_tracks_name_an_osmo() {
        for kind in [b"djmd", b"dbgi"] {
            let track = nest(
                &[b"trak", b"mdia", b"minf", b"stbl"],
                stsd(&[mp4box(kind, &[0; 12])]),
            );
            let bytes = file(b"pictures", mp4box(b"moov", &track));
            assert_eq!(sniffed(&bytes), Format::Osmo, "{kind:?}");
        }
    }

    /// Google's spherical metadata, both versions: v2's `sv3d` inside the
    /// video sample entry, and v1's `uuid` on the track. A file carrying
    /// either is already stitched, which is not what Kjerag reprojects.
    ///
    /// Hand-built, and never run against a real file: see this module's own
    /// doc comment.
    #[test]
    fn spherical_metadata_names_a_stitched_video() {
        let v2 = nest(
            &[b"trak", b"mdia", b"minf", b"stbl"],
            stsd(&[visual(b"hvc1", mp4box(b"sv3d", &mp4box(b"st3d", &[0; 8])))]),
        );
        assert_eq!(
            sniffed(&file(b"pictures", mp4box(b"moov", &v2))),
            Format::Foreign(Foreign::Spherical)
        );

        let mut uuid = GSPHERICAL.to_vec();
        uuid.extend_from_slice(b"<GSpherical:Spherical>true</GSpherical:Spherical>");
        let v1 = mp4box(b"trak", &mp4box(b"uuid", &uuid));
        assert_eq!(
            sniffed(&file(b"pictures", mp4box(b"moov", &v1))),
            Format::Foreign(Foreign::Spherical)
        );
    }

    /// The `uuid` box is a general extension point, so the sixteen bytes are
    /// the whole of the claim: somebody else's uuid is somebody else's.
    #[test]
    fn another_uuid_is_not_spherical_metadata() {
        let uuid = mp4box(b"uuid", &[0x11; 32]);
        let bytes = file(b"pictures", mp4box(b"moov", &mp4box(b"trak", &uuid)));
        assert_eq!(sniffed(&bytes), Format::Unknown);
    }

    /// The trailer is checked before the container, so an Insta360 capture
    /// is one whatever else is in it and whatever it is named.
    #[test]
    fn the_trailer_magic_beats_everything() {
        let mut bytes = file(
            b"pictures",
            mp4box(b"moov", &mp4box(b"udta", &mp4box(b"GPMF", b""))),
        );
        bytes.extend_from_slice(MAGIC);
        assert_eq!(sniffed(&bytes), Format::Insta360);
    }

    /// The X2-class rear file: no trailer, no maker's boxes, and a file
    /// Kjerag plays by pairing it with its front half. Nothing may refuse it.
    #[test]
    fn a_plain_mp4_is_left_alone() {
        let track = nest(
            &[b"trak", b"mdia", b"minf", b"stbl"],
            stsd(&[visual(b"hvc1", mp4box(b"hvcC", &[0; 16]))]),
        );
        let bytes = file(b"pictures", mp4box(b"moov", &track));
        assert_eq!(sniffed(&bytes), Format::Unknown);
    }

    /// Why the search walks the box tree instead of grepping the file: a
    /// grep for `st3d` over the sample corpus hits two genuine Insta360
    /// captures inside their compressed video, and refusing a pilot's own
    /// footage is the one failure this must not have.
    #[test]
    fn a_signature_in_the_video_data_is_not_a_signature() {
        let mut mdat = b"...".to_vec();
        mdat.extend_from_slice(b"st3dsv3dGPMFdjmdFIRM");
        mdat.extend_from_slice(&GSPHERICAL);
        let track = nest(
            &[b"trak", b"mdia", b"minf", b"stbl"],
            stsd(&[visual(b"hvc1", mp4box(b"hvcC", &[0; 16]))]),
        );
        let bytes = file(&mdat, mp4box(b"moov", &track));
        assert_eq!(sniffed(&bytes), Format::Unknown);
    }

    /// A `moov` at the front is legal and some remuxers write one, so the
    /// walk finds it wherever it is. `mdat` in front of it is what every
    /// camera in the corpus writes.
    #[test]
    fn the_index_is_found_at_either_end() {
        let moov = mp4box(b"moov", &mp4box(b"udta", &mp4box(b"FIRM", b"H19")));
        let mut front = mp4box(b"ftyp", b"mp41\0\0\0\0mp41");
        front.extend(moov.clone());
        front.extend(mp4box(b"mdat", b"pictures"));
        assert_eq!(sniffed(&front), Format::Foreign(Foreign::GoPro));
        assert_eq!(
            sniffed(&file(b"pictures", moov)),
            Format::Foreign(Foreign::GoPro)
        );
    }

    /// A big file's `mdat` carries a 64-bit size, and the box after it is
    /// only reached by reading that size rather than the 32-bit one.
    #[test]
    fn a_64_bit_box_size_is_read() {
        let payload = b"pictures";
        let mut mdat = 1u32.to_be_bytes().to_vec();
        mdat.extend_from_slice(b"mdat");
        mdat.extend(((16 + payload.len()) as u64).to_be_bytes());
        mdat.extend_from_slice(payload);

        let mut bytes = mp4box(b"ftyp", b"mp41\0\0\0\0mp41");
        bytes.extend(mdat);
        bytes.extend(mp4box(b"moov", &mp4box(b"udta", &mp4box(b"GPMF", b""))));
        assert_eq!(sniffed(&bytes), Format::Foreign(Foreign::GoPro));
    }

    /// Nothing a broken or hostile file says makes this panic or spin: it
    /// stops walking and the file goes unnamed.
    #[test]
    fn a_malformed_file_is_unnamed_and_nothing_else() {
        let cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"not an mp4 at all".to_vec(),
            // A box that claims more than the file holds.
            [&u32::MAX.to_be_bytes()[..], b"moov"].concat(),
            // A box smaller than its own header.
            [&3u32.to_be_bytes()[..], b"moov"].concat(),
            // A `moov` whose children claim more than it holds.
            mp4box(b"moov", &[&u32::MAX.to_be_bytes()[..], b"udta"].concat()),
            // A 64-bit size with nothing behind it.
            [&1u32.to_be_bytes()[..], b"mdat"].concat(),
            // `stsd` too short to hold its own count.
            mp4box(
                b"moov",
                &nest(
                    &[b"trak", b"mdia", b"minf", b"stbl"],
                    mp4box(b"stsd", &[0; 2]),
                ),
            ),
            // A sample entry too short to be a visual one.
            mp4box(
                b"moov",
                &nest(
                    &[b"trak", b"mdia", b"minf", b"stbl"],
                    stsd(&[mp4box(b"hvc1", &[0; 4])]),
                ),
            ),
        ];
        for bytes in cases {
            assert_eq!(sniffed(&bytes), Format::Unknown, "{bytes:?}");
        }
    }

    /// A `moov` nobody would write is not read into memory. The fixture is a
    /// sparse file, so it costs no disk, and the GoPro boxes at the front of
    /// that `moov` are what makes the check fail rather than pass if the
    /// limit ever goes: without it this answers GoPro, and allocates 64 MB to
    /// do it.
    #[test]
    fn an_enormous_index_is_left_unread() {
        let inside = mp4box(b"udta", &mp4box(b"GPMF", b""));
        let claimed = MOOV_LIMIT + 9;
        let mut head = mp4box(b"ftyp", b"mp41\0\0\0\0mp41");
        head.extend_from_slice(&(claimed as u32).to_be_bytes());
        head.extend_from_slice(b"moov");
        head.extend(inside);
        let file = Scratch::written("enormous-moov.mp4", &head);
        File::options()
            .write(true)
            .open(&file.path)
            .and_then(|f| f.set_len(20 + claimed))
            .expect("a sparse file of the claimed length");
        assert_eq!(Format::sniff(&file.path), Format::Unknown);
    }

    /// The name is the last word and only where the bytes had none.
    #[test]
    fn the_extension_names_a_maker_the_container_did_not() {
        assert_eq!(
            named(Path::new("holiday.360")),
            Some(Format::Foreign(Foreign::GoPro))
        );
        assert_eq!(named(Path::new("holiday.OSV")), Some(Format::Osmo));
        assert_eq!(named(Path::new("VID_00_001.insv")), None);
        assert_eq!(named(Path::new("holiday.mp4")), None);
        assert_eq!(named(Path::new("no-extension")), None);
    }

    /// The two halves together, over a real file: the bytes decide, and the
    /// name is asked only when they said nothing.
    #[test]
    fn content_beats_the_name() {
        let mut insv = file(b"pictures", mp4box(b"moov", &Vec::new()));
        insv.extend_from_slice(MAGIC);
        let renamed = Scratch::written("renamed.360", &insv);
        assert_eq!(Format::sniff(&renamed.path), Format::Insta360);

        let plain = file(b"pictures", mp4box(b"moov", &Vec::new()));
        let named = Scratch::written("nothing-in-it.360", &plain);
        assert_eq!(
            Format::sniff(&named.path),
            Format::Foreign(Foreign::GoPro),
            "the name is what is left when the container says nothing"
        );

        assert_eq!(
            Format::sniff(Path::new("/nowhere/at/all.mp4")),
            Format::Unknown,
            "a file that cannot be opened is not a file this names"
        );
    }

    /// A file under a name of this test's own, taken away when it ends.
    struct Scratch {
        path: std::path::PathBuf,
    }

    impl Scratch {
        fn written(name: &str, bytes: &[u8]) -> Self {
            let path =
                std::env::temp_dir().join(format!("kjerag-format-{}-{name}", std::process::id()));
            std::fs::write(&path, bytes).expect("the temporary directory is writable");
            Self { path }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
