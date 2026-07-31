//! What the headless instruments share: a GPU with no window on it, an
//! offscreen target to draw the app's own pass into, and the measurement that
//! reads the horizon back out of the result.
//!
//! `reframe` writes one view; `horizon` writes a run of them and measures
//! each. Both want the same device and the same target, and only the second
//! wants [`skyline`]. [`Picture`] is the third of them: a rendered view and
//! what separates two of them, which `zoom` and `ball` both ask for.
//!
//! [`Walk`] is the other half. The instruments that measure the **delivered**
//! picture rather than a rendered one want frames in system memory, every
//! stream at one instant, and `rolling` (issue #9) and `seam` (issue #48) want
//! the same ones.

mod frames;
mod offscreen;
mod picture;
mod skyline;

pub use frames::{Pair, Plane, Walk};
pub use offscreen::{Gpu, Offscreen};
pub use picture::{Difference, FORMAT, Picture, Render, aspect};
pub use skyline::{Skyline, skyline};
