//! Kyerag: a 360 video player for the COSMIC desktop.
//!
//! ```sh
//! cargo run --release                 # the window, with nothing open
//! cargo run --release -- <file.insv>  # play it: drag to look, space to pause
//! ```
//!
//! The shell is docs/UI.md's, which is the design this crate implements and
//! which cites a first-party COSMIC app for every call it makes.

mod app;
mod args;
mod config;
mod dnd;
mod key_bind;
mod menu;
mod shot;
mod strings;

use std::process::ExitCode;

fn main() -> ExitCode {
    match args::parse(std::env::args().skip(1)) {
        Ok(args::Args::Play(input)) => match app::run(input) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("kyerag: {e}");
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
            eprintln!("kyerag: {e}\n\n{}", args::help());
            ExitCode::from(2)
        }
    }
}
