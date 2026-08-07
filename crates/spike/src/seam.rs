//! Which seam correction an instrument draws with, and the parser they all
//! take it through.
//!
//! One copy, because there were six: `band`, `crossing`, `proof`, `reframe`,
//! `shear` and `step` each carried their own `enum Seam` and its own `match`
//! over `factory` / `file` / the five knobs, and `proof` a second copy of the
//! knob parser as well. A path added to one of them was a path five of them
//! did not have, which is how `pool` would have gone in.
//!
//! `pool` is the one that matters for the record. An acceptance line in
//! docs/research/reference-views.md carries a `seam=` because a reading is
//! only a reading at the pose it was taken through, and a literal string is a
//! copy of that pose taken on a date: the app's pool grows, its answer moves,
//! and the line goes on quoting the old one. Two acceptance lines did exactly
//! that between 2026-08-05 and 2026-08-07, along with four copies of them in
//! docs and instrument headers, all quoting a knob-by-knob median of the
//! owner's pool that `SeamPool::answer` had stopped shipping. `pool` reads the
//! app's own saved state at run time instead, so a line written with it cannot
//! go stale against the app at all.

use std::path::Path;

use kjerag_media::Fallible;
use kjerag_meta::CalibrationSet;
use kjerag_render::{Scene, SeamFit};

/// The app's own three seam paths, which is all an instrument can draw
/// through.
///
/// `pool` is not a fourth: it is where the five knobs come from, so it
/// resolves to [`Seam::Stored`] at parse time and nothing downstream has to
/// know it was ever anything else.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seam {
    /// The factory extrinsics as the trailer gives them, uncorrected.
    Factory,
    /// Fitted off this file, at open, the way the app fits one it has not
    /// pooled yet. Per capture, so it absorbs that capture's own parallax.
    File,
    /// Five knobs: named on the command line, or read off the pool.
    Stored(SeamFit),
}

impl Seam {
    /// `factory`, `file`, `pool`, or `roll:..,yaw:..,pitch:..,cx:..,cy:..`.
    ///
    /// Takes the input file because `pool` is resolved here, before anything
    /// downstream sees the choice: the pool is keyed by camera and the camera
    /// is a fact about the file. Resolving it at the door means every
    /// instrument that reads the fit back out - `crossing`'s dither and its
    /// `plant=` both do - gets the fit itself and needs to know nothing about
    /// where it came from.
    pub fn parse(value: &str, input: &Path) -> Fallible<Self> {
        match value {
            "factory" => Ok(Self::Factory),
            "file" => Ok(Self::File),
            knobs => Ok(Self::Stored(fit_arg(knobs, Some(input))?)),
        }
    }

    /// Put it on the scene, and say which pose that was.
    ///
    /// `Stored` says it because `seam=pool` does not carry its answer on the
    /// command line: a run whose numbers are quoted later has to be able to
    /// say what pose it drew them at, and with `pool` the line above is the
    /// only place that pose is written down. `File` says nothing here because
    /// `Scene::fit_seam` prints its own fit when it lands.
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
            Self::File => scene.fit_seam(true),
            Self::Stored(fit) => {
                println!("{label:<8}{}", knobs_of(fit));
                scene.use_seam(fit);
            }
        }
    }
}

/// The two `seam=` values that answer with a fit rather than with a way of
/// getting one: `pool`, and the five knobs written out.
///
/// `table` takes this one rather than [`Seam`], because one pose for every
/// capture is that instrument's whole premise and it has no `factory` or
/// `file` to offer. It is also the one instrument that can be run with no
/// capture at all (`read=` and `plant=`), which is why the input is an
/// `Option` and why only the `pool` arm looks at it: knobs written out are
/// answerable with no file, and asking one for a file it does not need was a
/// refusal this argument invented.
pub fn fit_arg(value: &str, input: Option<&Path>) -> Fallible<SeamFit> {
    match value {
        "pool" => pooled(input.ok_or(NEEDS_A_CAPTURE)?),
        knobs => knobs_fit(knobs),
    }
}

/// `seam=pool` is filed under the camera a capture names, so with no capture
/// on the line there is nothing to look it up by.
const NEEDS_A_CAPTURE: &str = "seam=pool is read out of the saved state under the camera a \
     capture names, so it needs a capture on the line to name one";

/// What the app itself draws this file's camera with: `SeamPool::answer` over
/// the pool this box has watched into its own saved state.
///
/// The app's reader and the app's rule, not a second copy of either. A pool is
/// RON on disk and a medoid in arithmetic, and either one written out again
/// here would be a second answer to the only question an acceptance line asks.
pub fn pooled(input: &Path) -> Fallible<SeamFit> {
    // The trailer is read for one number, the camera key the pool is filed
    // under, and the failure says so: on its own the error is a bare "No such
    // file or directory" from a path the caller may not have known was read.
    let calibration = CalibrationSet::from_insv(input).map_err(|e| {
        format!(
            "seam=pool reads {}'s own camera key, and: {e}",
            input.display(),
        )
    })?;
    from_pool(
        &kjerag::config::state(kjerag::APP_ID),
        calibration.camera_key(),
    )
}

/// The refusal is the point: a run that quietly drew factory extrinsics
/// because the pool was empty would print numbers, and they would be numbers
/// about a pose nobody asked for.
fn from_pool(state: &kjerag::config::ConfigState, camera: u64) -> Fallible<SeamFit> {
    state.seam(camera).ok_or_else(|| {
        format!(
            "seam=pool: the saved state of {} holds no pooled fit for camera {camera:016x}. \
             Play a few captures off that camera in kjerag and it fills itself; \
             seam=factory, seam=file or the five knobs need no pool.",
            kjerag::APP_ID,
        )
        .into()
    })
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
    use kjerag::config::ConfigState;
    use kjerag_render::Harvest;

    use super::*;

    /// The owner's own pool, as `seam_pool` holds it on 2026-08-06: three
    /// distinct fits over five samples, two of the three stored twice.
    ///
    /// The same five, to the same digits, as the fixture in
    /// `crates/app/src/config.rs`. Two copies because the crates cannot share
    /// a `#[cfg(test)]` fixture and the app is not going to ship one for the
    /// instruments; both ends assert a string off it, and those strings are
    /// what would catch them drifting apart.
    const OWNERS_CAMERA: u64 = 0xd8a3_9338_9b7b_8639;
    const OWNERS_FITS: [(SeamFit, usize, f64, usize); 3] = [
        (
            SeamFit {
                roll_deg: 0.5770177572311984,
                yaw_deg: -1.693547826643539,
                pitch_deg: -0.796449725529272,
                cx_px: -9.531358691231077,
                cy_px: -5.414553495776632,
            },
            27,
            0.7979799684676536,
            2,
        ),
        (
            SeamFit {
                roll_deg: 0.4592518809185011,
                yaw_deg: -2.0772194092771397,
                pitch_deg: -2.219459668631724,
                cx_px: -14.786100683560385,
                cy_px: -20.659845193073906,
            },
            12,
            0.760502617023373,
            1,
        ),
        (
            SeamFit {
                roll_deg: 0.7954311295817457,
                yaw_deg: -2.309572216062777,
                pitch_deg: -0.9358779752048013,
                cx_px: -3.2814366126974686,
                cy_px: -11.91227998928906,
            },
            41,
            0.49833332566304156,
            2,
        ),
    ];

    fn owners_pool() -> ConfigState {
        let mut state = ConfigState::default();
        for (fit, patches, residual_deg, copies) in OWNERS_FITS {
            for _ in 0..copies {
                assert!(state.harvest(
                    OWNERS_CAMERA,
                    Harvest {
                        fit,
                        patches,
                        residual_deg,
                        along: None,
                    },
                ));
            }
        }
        state
    }

    /// What `seam=pool` hands an instrument off this pool: a fit some capture
    /// took, and specifically not the knob-by-knob middle two acceptance lines
    /// quoted until 2026-08-07.
    ///
    /// That the middle of this pool IS that string is asserted where the rule
    /// it came from lives, in `crates/app/src/config.rs`
    /// (`the_pooled_answer_is_a_fit_some_capture_actually_took`). This end
    /// asserts what the reader returns, which is the other half.
    #[test]
    fn the_pool_answers_with_a_member_and_not_with_a_knobwise_middle() {
        let members: Vec<SeamFit> = OWNERS_FITS.iter().map(|(fit, ..)| *fit).collect();
        let answer = from_pool(&owners_pool(), OWNERS_CAMERA).unwrap();
        assert!(members.contains(&answer), "{answer:?} is nobody's fit");
        assert_eq!(
            knobs_of(answer),
            "roll:0.795,yaw:-2.310,pitch:-0.936,cx:-3.28,cy:-11.91",
        );
    }

    /// A pool with nothing in it for this camera refuses rather than falling
    /// back to factory: the numbers a silent fallback would print are about a
    /// pose nobody asked for.
    #[test]
    fn a_camera_with_no_pooled_fit_is_refused_by_name() {
        let refusal = from_pool(&owners_pool(), 0x0102_0304_0506_0708)
            .expect_err("a pool with nothing for this camera answered anyway");
        let refusal = refusal.to_string();
        assert!(refusal.contains("0102030405060708"), "{refusal}");
        assert!(refusal.contains("seam=pool"), "{refusal}");
    }

    /// What a run prints of its own pose is the argument that draws it again.
    #[test]
    fn what_a_run_says_it_drew_parses_back_to_what_it_drew() {
        let answer = from_pool(&owners_pool(), OWNERS_CAMERA).unwrap();
        let said = knobs_of(answer);
        assert_eq!(
            Seam::parse(&said, Path::new("/nonexistent.insv")).unwrap(),
            Seam::Stored(knobs_fit(&said).unwrap()),
        );
    }

    /// Everything but `pool` answers without a file, so a typo in the knobs is
    /// refused before anything is opened.
    #[test]
    fn the_paths_that_need_no_pool_read_no_file() {
        let nowhere = Path::new("/nonexistent.insv");
        assert_eq!(Seam::parse("factory", nowhere).unwrap(), Seam::Factory);
        assert_eq!(Seam::parse("file", nowhere).unwrap(), Seam::File);
        assert_eq!(
            Seam::parse("roll:0.8,yaw:-2.3", nowhere).unwrap(),
            Seam::Stored(SeamFit {
                roll_deg: 0.8,
                yaw_deg: -2.3,
                ..SeamFit::default()
            }),
        );
        assert!(Seam::parse("roll:0.8,tilt:1", nowhere).is_err());
    }
}
