//! Which seam correction an instrument draws with, and the parser they all
//! take it through.
//!
//! One copy, because there were six: `band`, `crossing`, `proof`, `reframe`,
//! `shear` and `step` each carried their own `enum Seam` and its own `match`
//! over `factory` / the knobs, and `proof` a second copy of the knob parser as
//! well.
//!
//! Two paths, since the non-parity per-capture fit was blown away (2026-08-15,
//! parity mandate): the **factory** calibration the trailer gives, which is the
//! parity base and the default, and the five **knobs** written out, a disclosed
//! knob for RE and testing that is never a default. The old `file` (fit off this
//! capture's own frames) and `pool` (the app's saved per-camera fit) paths were
//! the non-parity mechanism and are gone with it.

use kjerag_media::Fallible;
use kjerag_render::{Scene, SeamFit};

/// The two seam paths an instrument can draw through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seam {
    /// The factory extrinsics as the trailer gives them, uncorrected. The
    /// parity base, and the default an instrument draws unless it is handed
    /// knobs.
    Factory,
    /// Five knobs, named on the command line: a disclosed override for RE and
    /// testing, never a default.
    Stored(SeamFit),
}

impl Seam {
    /// `factory`, or `roll:..,yaw:..,pitch:..,cx:..,cy:..`.
    pub fn parse(value: &str) -> Fallible<Self> {
        match value {
            "factory" => Ok(Self::Factory),
            knobs => Ok(Self::Stored(knobs_fit(knobs)?)),
        }
    }

    /// Put it on the scene, and say which pose that was.
    ///
    /// `Stored` says it because a run whose numbers are quoted later has to be
    /// able to say what pose it drew them at. `Factory` says it draws the
    /// calibration and corrects nothing.
    pub fn hold(self, scene: &Scene) {
        self.hold_as("seam", scene);
    }

    /// The same, under a name of the caller's choosing.
    ///
    /// `proof` draws one view through two of these in a row, and two lines
    /// both saying `seam:` say which poses were drawn and not which drew
    /// which, which is the whole purpose of printing them.
    pub fn hold_as(self, what: &str, scene: &Scene) {
        let label = format!("{what}:");
        match self {
            Self::Factory => println!("{label:<8}factory calibration, no correction"),
            Self::Stored(fit) => {
                println!("{label:<8}{}", knobs_of(fit));
                scene.use_seam(fit);
            }
        }
    }
}

/// A fit written the way `seam=` takes one, for the one instrument (`table`)
/// that takes a bare fit rather than a whole [`Seam`] and has no `factory` to
/// offer.
pub fn fit_arg(value: &str) -> Fallible<SeamFit> {
    knobs_fit(value)
}

/// A fit written the way `seam=` takes one, so what a run says it drew can be
/// pasted straight back in as the argument that draws it again.
fn knobs_of(fit: SeamFit) -> String {
    format!(
        "roll:{:.3},yaw:{:.3},pitch:{:.3},cx:{:.2},cy:{:.2}",
        fit.roll_deg, fit.yaw_deg, fit.pitch_deg, fit.cx_px, fit.cy_px,
    )
}

/// `roll:0.71,yaw:-2.35,pitch:-1.61,cx:-1.26,cy:-14.60`, in each knob's own
/// units, as the app's config stores them.
fn knobs_fit(value: &str) -> Fallible<SeamFit> {
    let mut fit = SeamFit::default();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The two paths: `factory`, and the five knobs. A typo in a knob is
    /// refused rather than drawn.
    #[test]
    fn the_paths_are_factory_and_the_knobs() {
        assert_eq!(Seam::parse("factory").unwrap(), Seam::Factory);
        assert_eq!(
            Seam::parse("roll:0.8,yaw:-2.3").unwrap(),
            Seam::Stored(SeamFit {
                roll_deg: 0.8,
                yaw_deg: -2.3,
                ..SeamFit::default()
            }),
        );
        assert!(Seam::parse("roll:0.8,tilt:1").is_err());
        // The non-parity paths are gone: their names are just unknown knobs now.
        assert!(Seam::parse("file").is_err());
        assert!(Seam::parse("pool").is_err());
    }

    /// What a run prints of its own pose is the argument that draws it again.
    #[test]
    fn what_a_run_says_it_drew_parses_back_to_what_it_drew() {
        let fit = SeamFit {
            roll_deg: 0.795,
            yaw_deg: -2.310,
            pitch_deg: -0.936,
            cx_px: -3.28,
            cy_px: -11.91,
        };
        let said = knobs_of(fit);
        assert_eq!(
            Seam::parse(&said).unwrap(),
            Seam::Stored(knobs_fit(&said).unwrap()),
        );
    }
}
