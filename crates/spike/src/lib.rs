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

mod offscreen;
mod picture;
mod skyline;

pub use kjerag_media::{Chroma, Pair, Plane, Walk};
pub use offscreen::{Gpu, Offscreen};
pub use picture::{Difference, FORMAT, Picture, Render, aspect};
pub use skyline::{Skyline, skyline};
