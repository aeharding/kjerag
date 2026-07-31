//! Where the other half of a capture is, on the cameras that write one lens
//! per file.
//!
//! An X4-class `.insv` carries both lenses as two video streams of one MP4.
//! The ONE X2 and the models before it write **one file per lens**, named
//! `VID_<date>_<time>_00_<clip>.insv` and `VID_<date>_<time>_10_<clip>.insv`,
//! and neither file is the whole sphere on its own. This module is the rule
//! that turns one of those names into the other.
//!
//! **Only one of the two files carries a trailer** (measured 2026-07-31 on
//! all three ONE X2 captures on this box, firmware `v1.0.62_build2`): the
//! `_00_` file ends in the Insta360 magic and the `_10_` file does not, so
//! the calibration, the IMU and the exposure track exist once for the pair
//! and live with lens 0. That is why [`sibling`] is not enough by itself and
//! [`super::CalibrationSet::from_capture`] exists: opening the `_10_` file
//! has to read the `_00_` file's trailer or there is no calibration at all.
//!
//! The `LRV_..._11_....insv` proxy alongside them is deliberately **not** a
//! sibling of anything: its marker is `11`, it is one track with both lenses
//! side by side, and it carries a full copy of the trailer. Matching only
//! `00` and `10` is what keeps it out.

use std::path::{Path, PathBuf};

/// The two lens markers, in lens order. Lens 0 is the file that carries the
/// trailer.
const MARKERS: [&str; 2] = ["00", "10"];

/// Where the marker sits in an underscore-separated name: second from last,
/// i.e. the field before the clip number. `VID_20000101_100000_00_001.insv`
/// is `VID`, the date, the time, the **marker**, and the clip.
const MARKER_FROM_END: usize = 2;

/// The file holding this capture's other lens, if the name says there is one
/// and it is on disk.
///
/// `None` for every X4-class file, whose two lenses are two streams of the
/// one file: nothing is named `_10_` next to them, so the lookup is one
/// `Path::exists` that fails.
pub fn sibling(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    let beside = path.with_file_name(sibling_name(name)?);
    beside.is_file().then_some(beside)
}

/// Which lens of the pair this file holds, from its marker alone: 0 for
/// `_00_`, 1 for `_10_`, and `None` for a name that carries neither.
///
/// The order is the trailer's own. Lens 0 is the extrinsic reference
/// (`offset_v3` gives it a translation of exactly zero) and it is the lens
/// whose shutter track record 4 holds; the ONE X2 writes record 4 and no
/// record 12, so the file that carries the trailer is the file that carries
/// lens 0. Rendered, the other assignment breaks the seam: see the PR for
/// issue #79.
pub fn lens_index(path: &Path) -> Option<usize> {
    let marker = field(path.file_name()?.to_str()?)?;
    MARKERS.iter().position(|known| *known == marker)
}

/// The same name with the lens marker swapped, or `None` for a name that has
/// no marker in it.
fn sibling_name(name: &str) -> Option<String> {
    let marker = field(name)?;
    let other = match marker {
        "00" => "10",
        "10" => "00",
        _ => return None,
    };
    let mut fields: Vec<&str> = name.split('_').collect();
    let at = fields.len().checked_sub(MARKER_FROM_END)?;
    fields[at] = other;
    Some(fields.join("_"))
}

/// The marker field of a name, or `None` where the name has too few fields
/// to have one.
fn field(name: &str) -> Option<&str> {
    let fields: Vec<&str> = name.split('_').collect();
    fields
        .len()
        .checked_sub(MARKER_FROM_END)
        .and_then(|at| fields.get(at).copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The convention, on the names the box actually holds.
    #[test]
    fn the_marker_swaps_and_the_rest_of_the_name_does_not() {
        assert_eq!(
            sibling_name("VID_20000101_100000_00_001.insv").as_deref(),
            Some("VID_20000101_100000_10_001.insv")
        );
        assert_eq!(
            sibling_name("VID_20000101_100000_10_001.insv").as_deref(),
            Some("VID_20000101_100000_00_001.insv")
        );
    }

    /// The clip number is part of the name and not part of the marker: three
    /// captures from one evening are three pairs, not one six-file capture.
    #[test]
    fn a_different_clip_or_time_is_a_different_capture() {
        let pairs = [
            (
                "VID_20000101_100000_00_001.insv",
                "VID_20000101_100000_10_001.insv",
            ),
            (
                "VID_20000101_110000_00_002.insv",
                "VID_20000101_110000_10_002.insv",
            ),
            (
                "VID_20000101_120000_00_003.insv",
                "VID_20000101_120000_10_003.insv",
            ),
        ];
        for (lens0, lens1) in pairs {
            assert_eq!(sibling_name(lens0).as_deref(), Some(lens1));
            assert_eq!(lens_index(Path::new(lens0)), Some(0));
            assert_eq!(lens_index(Path::new(lens1)), Some(1));
        }
        // And no name maps onto another capture's file.
        assert_ne!(
            sibling_name(pairs[0].0).as_deref(),
            Some("VID_20000101_110000_10_002.insv")
        );
    }

    /// The proxy the camera writes beside the pair is not a lens of it. It is
    /// one track with both fisheyes side by side and it carries its own copy
    /// of the trailer, so pairing it with anything would decode the wrong
    /// pixels at the right size.
    #[test]
    fn the_lrv_proxy_is_not_a_sibling() {
        assert_eq!(sibling_name("LRV_20000101_184419_11_001.insv"), None);
        assert_eq!(
            lens_index(Path::new("LRV_20000101_184419_11_001.insv")),
            None
        );
    }

    /// An X4-class capture is one file with two streams. Its name carries a
    /// `00` marker all the same, so the rule answers a name; what makes it a
    /// no-op there is that nothing is on disk under it (`sibling`) and that
    /// the reader only looks when the container decodes one lens.
    #[test]
    fn an_x4_name_has_a_marker_but_no_file_beside_it() {
        assert_eq!(
            sibling_name("VID_20260501_183417_00_001.insv").as_deref(),
            Some("VID_20260501_183417_10_001.insv")
        );
        assert_eq!(
            sibling(Path::new("/nowhere/VID_20260501_183417_00_001.insv")),
            None
        );
    }

    #[test]
    fn a_name_with_no_marker_pairs_with_nothing() {
        assert_eq!(sibling_name("holiday.insv"), None);
        assert_eq!(
            sibling_name("VID_00_001.insv").as_deref(),
            Some("VID_10_001.insv")
        );
        assert_eq!(sibling_name("00_001.insv").as_deref(), Some("10_001.insv"));
        assert_eq!(sibling_name("001.insv"), None);
        assert_eq!(sibling_name(""), None);
        assert_eq!(lens_index(Path::new("holiday.insv")), None);
    }

    /// A marker in the wrong field is not a marker. The rule reads the field
    /// before the clip number and nothing else, so a date that happens to
    /// contain `00` cannot move the lens.
    #[test]
    fn only_the_field_before_the_clip_number_is_the_marker() {
        assert_eq!(sibling_name("VID_00_184419_11_001.insv"), None);
        assert_eq!(
            sibling_name("VID_00_184419_00_001.insv").as_deref(),
            Some("VID_00_184419_10_001.insv"),
            "the marker field moves and the date field does not"
        );
    }
}
