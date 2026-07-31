//! Kyerag: a 360 video player for the COSMIC desktop.
//!
//! ```sh
//! cargo run --release                 # animated shader pass, no decode
//! cargo run --release -- <file.insv>  # play it: drag to look, space to pause
//! ```
//!
//! `s` saves a still of the view to the screenshots folder and `Ctrl+C`
//! copies one to the clipboard (issue #15).

mod app;
mod shot;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run(std::env::args().nth(1).map(std::path::PathBuf::from))
}
