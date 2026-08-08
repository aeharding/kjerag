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
//! which cites a first-party COSMIC app for every call it makes. It lives in
//! `src/lib.rs`, which says why it is reachable as a library.

use std::process::ExitCode;

use kjerag::{ab, app, args};

fn main() -> ExitCode {
    let played = |input, at, session| match app::run(input, at, session) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kjerag: {e}");
            ExitCode::FAILURE
        }
    };
    // The two research paths that open no window: they answer on stdout, and
    // a session they will not take is refused with the code a bad command
    // line gets, so a runner script can tell the two apart from a crash.
    let said = |answer: Result<String, String>| match answer {
        Ok(answer) => {
            print!("{answer}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("kjerag: {e}");
            ExitCode::from(2)
        }
    };
    match args::parse(std::env::args().skip(1)) {
        Ok(args::Args::Play(input, at)) => played(input, at, None),
        // The session is read and checked here, before a window opens, and a
        // session that asks for something the running pass cannot hand over
        // is refused with the same exit code a bad command line gets. An A/B
        // that half applied would be worse than one that never started.
        Ok(args::Args::Ab(file)) => match ab::open(&file) {
            Ok(session) => played(None, None, Some(session)),
            Err(said) => {
                eprintln!("kjerag: {said}");
                ExitCode::from(2)
            }
        },
        // The same read and the same refusal, and then nothing: no window, no
        // decode, no sound. It says what it found rather than only what is
        // wrong, because an agent that staged a session wants to see the
        // shape of what it staged, and it says no arm names, because the same
        // command run in the wrong terminal would unblind the session.
        Ok(args::Args::AbCheck(file)) => said(ab::open(&file).map(|session| session.shape())),
        // The key, off the same parse, because a second reader of the session
        // grammar is a key that can drift from the session it unblinds.
        Ok(args::Args::AbKey(file)) => said(ab::open(&file).map(|session| session.key())),
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
