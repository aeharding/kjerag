//! Kjerag: a 360 video player for the COSMIC desktop.
//!
//! ```sh
//! cargo run --release                 # the window, with nothing open
//! cargo run --release -- <file.insv>  # play it: drag to look, space to pause
//! cargo run --release -- <file.insv> time=9.576 yaw=144.40 pitch=0.90 fov=24.10 lock=1
//! ```
//!
//! The third is the line `i` copies in the window, which is what makes a
//! report about a 360 video into a command anyone can run (`crates/render/
//! src/framing.rs`).
//!
//! The shell is docs/UI.md's, which is the design this crate implements and
//! which cites a first-party COSMIC app for every call it makes.

mod app;
mod args;
mod config;
mod dnd;
mod fail;
mod key_bind;
mod menu;
mod shot;
mod strings;

use std::process::ExitCode;

fn main() -> ExitCode {
    match args::parse(std::env::args().skip(1)) {
        Ok(args::Args::Play(input, at)) => match app::run(input, at) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("kjerag: {e}");
                ExitCode::FAILURE
            }
        },
        Ok(args::Args::Help) => {
            println!("{}", args::help());
            ExitCode::SUCCESS
        }
        Ok(args::Args::Version) => {
            println!("{}", args::version());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("kjerag: {e}\n\n{}", args::help());
            ExitCode::from(2)
        }
    }
}
