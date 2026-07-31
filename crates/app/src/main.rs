//! Kyerag: a 360 video player for the COSMIC desktop.
//!
//! ```sh
//! cargo run --release                 # animated shader pass, no decode
//! cargo run --release -- <file.insv>  # frame 0 of stream 0, imported zero-copy
//! ```

mod app;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app::run(std::env::args().nth(1).map(std::path::PathBuf::from))
}
