//! `kyerag [options] [file]`.
//!
//! A path, not a URL. cosmic-player parses its freestanding arguments as URLs
//! (`src/argparse.rs:70-79`) because GStreamer streams from the network; we
//! decode local files only, so a path stays a path all the way to the
//! demuxer.
//!
//! Hand rolled rather than through a parser crate: `--help` and `--version`
//! and one positional argument is the whole grammar, and a terminal user
//! tries the first two before anything else.

use std::path::PathBuf;

#[derive(Debug, Eq, PartialEq)]
pub enum Args {
    Play(Option<PathBuf>),
    Help,
    Version,
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut input = None;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Args::Help),
            "-V" | "--version" => return Ok(Args::Version),
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            _ if input.is_some() => return Err("only one file can be opened at a time".to_owned()),
            path => input = Some(PathBuf::from(path)),
        }
    }
    Ok(Args::Play(input))
}

pub fn help() -> String {
    format!(
        "{}\n\n\
         Usage: kyerag [options] [file.insv]\n\n\
         Options:\n  \
         -h, --help     Show this message\n  \
         -V, --version  Show the version",
        version()
    )
}

pub fn version() -> String {
    format!("kyerag {}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(line: &str) -> Result<Args, String> {
        parse(line.split_whitespace().map(str::to_owned))
    }

    #[test]
    fn a_bare_path_is_the_file_to_play() {
        assert_eq!(
            parse_words("/home/pilot/a.insv"),
            Ok(Args::Play(Some(PathBuf::from("/home/pilot/a.insv"))))
        );
        assert_eq!(parse_words(""), Ok(Args::Play(None)));
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
}
