//! What the headless instruments share: a GPU with no window on it, an
//! offscreen target to draw the app's own pass into, and the measurement that
//! reads the horizon back out of the result.
//!
//! `reframe` writes one view; `horizon` writes a run of them and measures
//! each. Both want the same device and the same target, and only the second
//! wants [`skyline`]. [`Picture`] is the third of them: a rendered view and
//! what separates two of them, which `zoom` and `ball` both ask for.
//!
//! The frames themselves come from `kjerag_media::Walk`, re-exported here
//! because the instruments that read the **delivered** picture rather than a
//! rendered one all reach for it: it moved into `media` when the seam fit
//! started running at open in the app as well (issue #48).
//!
//! [`Seam`] is the fourth: the instruments take the same `seam=` argument, now
//! `factory` (the parity base, and the default) or the five knobs written out
//! (`seam.rs`). The `file` and `pool` values that read a per-capture content
//! fit were the non-parity mechanism and were removed with it (2026-08-15), and
//! the workspace's one upward edge to the shell crate went with them.

pub mod crossing;
mod offscreen;
mod picture;
pub mod registration;
mod seam;
pub mod seam_trace;
mod skyline;

pub use kjerag_media::{Chroma, Pair, Plane, Walk};
pub use offscreen::{Gpu, Offscreen};
pub use picture::{Difference, FORMAT, Picture, Render, aspect};
pub use seam::{Seam, fit_arg};
pub use skyline::{Skyline, skyline};

/// One camera's along-seam table, off the file `kjerag-spike --bin table`
/// writes ([`kjerag_render::Table::write`]).
///
/// Here rather than inside one instrument for [`Seam`]'s reason: four of them
/// now draw a picture through a stored table, and a second copy of this reader
/// is a second chance for two of them to disagree about what a stored
/// calibration says.
pub fn seam_table(path: &str) -> kjerag_media::Fallible<kjerag_render::Table> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    kjerag_render::Table::read(&text).ok_or_else(|| {
        format!(
            "{path} is not {} numbers, one per line",
            kjerag_render::AZIMUTHS,
        )
        .into()
    })
}
