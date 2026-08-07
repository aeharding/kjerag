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

pub mod crossing;
mod offscreen;
mod picture;
pub mod registration;
mod skyline;

pub use kjerag_media::{Chroma, Pair, Plane, Walk};
pub use offscreen::{Gpu, Offscreen};
pub use picture::{Difference, FORMAT, Picture, Render, aspect};
pub use skyline::{Skyline, skyline};

/// `roll:0.71,yaw:-2.35,pitch:-1.61,cx:-1.26,cy:-14.60`, in each knob's own
/// units, as the app's config stores them.
///
/// Here rather than inside one instrument because more than one of them takes
/// a stored per-camera calibration on the command line, and a second copy of
/// this parser is a second chance for two instruments to disagree about what
/// a pilot's own config says.
pub fn seam_fit(value: &str) -> kjerag_media::Fallible<kjerag_render::SeamFit> {
    let mut fit = kjerag_render::SeamFit::default();
    for term in value.split(',') {
        let (name, amount) = term.split_once(':').ok_or("a stored knob is knob:amount")?;
        let amount: f64 = amount.parse()?;
        match name {
            "roll" => fit.roll_deg = amount,
            "yaw" => fit.yaw_deg = amount,
            "pitch" => fit.pitch_deg = amount,
            "cx" => fit.cx_px = amount,
            "cy" => fit.cy_px = amount,
            _ => return Err(format!("no stored knob called {name}").into()),
        }
    }
    Ok(fit)
}

/// One camera's along-seam table, off the file `kjerag-spike --bin table`
/// writes ([`kjerag_render::Table::write`]).
///
/// Here rather than inside one instrument for `seam_fit`'s reason: four of
/// them now draw a picture through a stored table, and a second copy of this
/// reader is a second chance for two of them to disagree about what a stored
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
