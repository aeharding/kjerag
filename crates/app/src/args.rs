//! `kjerag [options] [file] [view]`.
//!
//! A path, not a URL. cosmic-player parses its freestanding arguments as URLs
//! (`src/argparse.rs:70-79`) because GStreamer streams from the network; we
//! decode local files only, so a path stays a path all the way to the
//! demuxer.
//!
//! After the path come the view's own terms, read by the same code the
//! clipboard is read with ([`Framing::read`]). That is what makes the line
//! the window prints beside every capture a command as well as a report:
//! select it out of the terminal, put `kjerag` in front of it, and the
//! window opens at that frame pointing that way.
//!
//! Hand rolled rather than through a parser crate: two flags, one path and
//! five keys is the whole grammar, and a terminal user tries the flags before
//! anything else.
//!
//! [`AB_SESSION`] is a third flag and is deliberately not in [`help`]. It is
//! the research path (`crate::ab`), it is staged by an agent rather than
//! typed by a pilot, and the same rule keeps `KJERAG_HANDOVER_DEG` out of the
//! window: not a setting, not a key and not a menu item. `~/kjerag-ab/
//! AGENTS-AB.md` is where it is written down.

use std::path::PathBuf;

use kjerag_render::Framing;

#[derive(Debug, PartialEq)]
pub enum Args {
    /// The file to open, and where in it to land.
    Play(Option<PathBuf>, Option<Framing>),
    /// A staged blind A/B session file, and nothing else: the file names its
    /// own clips and its own views (`crate::ab`). Research path only.
    Ab(PathBuf),
    Help,
    Version,
}

/// The research path's one flag.
const AB_SESSION: &str = "--ab-session";

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut input = None;
    let mut view = Vec::new();
    let mut session = None;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Args::Help),
            "-V" | "--version" => return Ok(Args::Version),
            flag if flag.starts_with(AB_SESSION) => {
                let file = flag
                    .strip_prefix(AB_SESSION)
                    .and_then(|rest| rest.strip_prefix('='))
                    .filter(|file| !file.is_empty())
                    .ok_or_else(|| format!("{AB_SESSION} needs a file: {AB_SESSION}=<file>"))?;
                session = Some(PathBuf::from(file));
            }
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            // One of the view's five keys, and nothing else with an `=` in
            // it: a file whose name has one is still a file.
            term if Framing::is_term(term) => view.push(arg),
            _ if input.is_some() => return Err("only one file can be opened at a time".to_owned()),
            path => input = Some(PathBuf::from(path)),
        }
    }
    let at = Framing::read(view.iter().map(String::as_str))?;
    // Refused rather than resolved. A session names the clip and the view of
    // every trial it has, so a file or a view beside it is two answers to one
    // question and one of them would be quietly dropped.
    if let Some(file) = session {
        if input.is_some() || at.is_some() {
            return Err(format!(
                "{AB_SESSION} names its own clips and its own views, so nothing else goes on the \
                 line with it"
            ));
        }
        return Ok(Args::Ab(file));
    }
    if at.is_some() && input.is_none() {
        return Err("a view needs the file it is a view of".to_owned());
    }
    Ok(Args::Play(input, at))
}

pub fn help() -> String {
    format!(
        "{}\n\n\
         Usage: kjerag [options] [file.insv] [view]\n\n\
         Options:\n  \
         -h, --help     Show this message\n  \
         -V, --version  Show the version\n\n\
         A view is the line `i` copies in the window, which is the same line\n\
         the window prints beside every capture. Paste one after `kjerag` and\n\
         it opens the file at that frame, pointing that way:\n\n  \
         kjerag flight.insv time=9.576 yaw=144.40 pitch=0.90 fov=24.10 lock=1",
        version()
    )
}

pub fn version() -> String {
    format!("kjerag {}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(line: &str) -> Result<Args, String> {
        parse(line.split_whitespace().map(str::to_owned))
    }

    fn played(line: &str) -> (Option<PathBuf>, Option<Framing>) {
        match parse_words(line) {
            Ok(Args::Play(input, at)) => (input, at),
            other => panic!("{line:?} did not play: {other:?}"),
        }
    }

    #[test]
    fn a_bare_path_is_the_file_to_play() {
        assert_eq!(
            parse_words("/home/pilot/a.insv"),
            Ok(Args::Play(Some(PathBuf::from("/home/pilot/a.insv")), None))
        );
        assert_eq!(parse_words(""), Ok(Args::Play(None, None)));
    }

    #[test]
    fn help_and_version_win_wherever_they_are() {
        assert_eq!(parse_words("a.insv --help"), Ok(Args::Help));
        assert_eq!(parse_words("-V a.insv"), Ok(Args::Version));
    }

    /// A player that silently ignores an option it does not have sends the
    /// pilot looking for a bug in the player.
    #[test]
    fn an_unknown_option_is_an_error() {
        assert!(parse_words("--fullscreen").is_err());
        assert!(parse_words("a.insv b.insv").is_err());
    }

    /// The whole point: the printed line, with `kjerag` in front of it, is a
    /// command that opens the file at that view.
    #[test]
    fn the_printed_line_is_a_launch_command() {
        let (input, at) =
            played("/home/pilot/a.insv time=9.576 yaw=144.40 pitch=0.90 fov=24.10 lock=1");
        assert_eq!(input, Some(PathBuf::from("/home/pilot/a.insv")));
        let at = at.expect("a view");
        assert!((at.at.as_secs_f64() - 9.576).abs() < 0.000_5);
        assert!((at.camera.yaw.to_degrees() - 144.40).abs() < 0.005);
        assert!((at.camera.fov.to_degrees() - 24.10).abs() < 0.005);
    }

    /// The terms are the view's, wherever they are: a pilot who types the
    /// path last still gets what he asked for.
    #[test]
    fn the_terms_are_not_the_file_wherever_they_sit() {
        let (input, at) = played("time=1.000 yaw=0.00 pitch=0.00 fov=90.00 lock=1 a.insv");
        assert_eq!(input, Some(PathBuf::from("a.insv")));
        assert!(at.is_some());
    }

    /// A path with an `=` in its name is a path. Only the five keys are
    /// terms, so nothing else is taken out of the file's place.
    #[test]
    fn a_file_named_with_an_equals_is_still_the_file() {
        let (input, at) = played("/home/pilot/a=b.insv");
        assert_eq!(input, Some(PathBuf::from("/home/pilot/a=b.insv")));
        assert_eq!(at, None);
    }

    /// The research path: one flag, one file, and the file says the rest.
    #[test]
    fn a_session_is_the_whole_line() {
        assert_eq!(
            parse_words("--ab-session=/tmp/s.ab"),
            Ok(Args::Ab(PathBuf::from("/tmp/s.ab")))
        );
        // Two answers to one question, one of which would be dropped.
        assert!(parse_words("--ab-session=/tmp/s.ab a.insv").is_err());
        assert!(parse_words("--ab-session=/tmp/s.ab time=1 yaw=0 pitch=0 fov=90 lock=1").is_err());
        // A flag that names nothing names nothing.
        assert!(parse_words("--ab-session").is_err());
        assert!(parse_words("--ab-session=").is_err());
        // Help and version still win wherever they are.
        assert_eq!(parse_words("--ab-session=/tmp/s.ab -V"), Ok(Args::Version));
    }

    /// Loud, per the existing rule for anything the command line does not
    /// understand: a half view, a term that is not a number, and a key that
    /// is not one of the five.
    #[test]
    fn a_view_that_is_not_one_is_an_error() {
        assert!(parse_words("a.insv time=1.0").is_err());
        assert!(parse_words("a.insv time=x yaw=0 pitch=0 fov=90 lock=1").is_err());
        assert!(parse_words("a.insv zoom=2").is_err());
    }

    /// A view of nothing has nowhere to land, and opening the welcome view
    /// at yaw 144 is not a thing that means anything.
    #[test]
    fn a_view_with_no_file_is_an_error() {
        assert!(parse_words("time=1.000 yaw=0 pitch=0 fov=90 lock=1").is_err());
    }
}
