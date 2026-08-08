//! The forward map: one output ray to one lens pixel, for each lens of the
//! camera, and how much of each is shown.
//!
//! **Two lens models, chosen per lens** (`kjerag_meta::Model`). Insta360's
//! `offset_v3` is Mei/UCM with Brown-Conrady distortion on the normalized
//! plane; a DJI Osmo 360's `djmd` calibration is a plain equidistant fisheye,
//! `r = fx * theta`, with no distortion terms at all
//! (`kjerag_meta::osmo` has the measurement that refused the four the file
//! carries). Both are one function of a unit ray in the lens's own frame and
//! nothing else, so the model is a branch inside [`lens_pixel`] and the rest
//! of the pass - the caps, the crossover, the band, the readout - does not
//! know which one it is running.
//!
//! It exists twice on purpose. `WGSL` below is the copy the GPU runs, once
//! per output pixel; [`Reframe::project`] is the same arithmetic in Rust, so
//! the model can be checked against known angles by `cargo test` on a box
//! with no GPU and no footage. The two read the same [`Reframe`] block, whose
//! binding is declared next to the shader source, so a field added on one
//! side has one obvious home on the other. `wgpu` checks the layouts agree:
//! the bind group declares `min_binding_size` from this type's size, and
//! pipeline creation rejects a shader whose struct wants more.
//!
//! Two lenses cover the sphere and overlap by about 14 degrees around the
//! seam, so most rays near it are in both pictures and the shader has to
//! decide how much of each to show. [`Reframe::blend`] is that decision: a
//! weight per lens, one outside the crossover and a smooth handover across
//! [`CROSSOVER_DEG`] of it (issues #7 and #48). A ray is dropped only where
//! **no** lens has it.
//!
//! The map also carries **when** each ray was seen (issue #9). A frame comes
//! off the sensor a row at a time over 15.9 ms, so the orientation a ray is
//! carried through belongs to the row it lands on rather than to the frame,
//! and [`Reframe::solve`] is that: reframing, stabilization and the readout
//! in one backward mapping per output pixel, with nothing resampled and no
//! pass added. Which way the sensor reads is not in the file, so it is
//! measured per camera and the correction is switched off on any camera it
//! has not been measured on (`kjerag_meta::Sweep`).
//!
//! The Mei half is written from the model description in
//! `docs/research/insv-format.md` 5.1 (Mei and Rives 2007, as OpenCV's
//! `cv::omnidir` states it). Nothing here is transcribed from Gyroflow's
//! `insta360.wgsl`, so this file is plain AGPL-3.0 with no GPL header.
//!
//! The other half of the map is the **output** projection, [`Screen`]: how a
//! point of the frame becomes the ray this file then projects. A flat window
//! on the world is what a player wants until the view gets wide, and past
//! about 110 degrees it stops being one, so the frame bends from there out
//! until the whole sphere is a ball with room around it (issue #47).

use std::f32::consts::PI;
use std::sync::OnceLock;

use kjerag_meta::{Intrinsics, Lens, Model, Quat};

use super::sampling::Sampling;
use super::{Camera, Size};

/// How many times the landing row is solved for before it is believed
/// (issue #9).
///
/// The row a ray lands on decides which instant its orientation is read at,
/// and that orientation decides the row: the map is its own input. So it is
/// solved for, from the frame's own instant outwards, and the question is how
/// many rounds it takes. Each round multiplies what is left over by the share
/// of the readout the round before moved the landing across, which is a couple
/// of percent at 500 deg/s and a tenth of that in ordinary flight.
///
/// **One round, measured** (`kjerag-spike --bin rolling model=1`): against a
/// solve run until it stops moving, at the hardest instant of a 30-minute
/// capture, 551 deg/s, one round leaves **4.5 px** of a 112 px correction and
/// two leave 0.24 px. The median rate on that footage is 20 deg/s, where the
/// correction is 4 px and one round leaves a hundredth of one. The second
/// round is another pass through the model per lens per pixel and costs about
/// as much again as the first, for a quarter of a pixel at an instant that
/// happens once in half an hour.
const READOUT_STEPS: usize = 1;

/// How far past a lens's own coverage the pre-test that skips it still lets
/// the model run, in degrees ([`LensBlock::axis_min`]).
///
/// It covers three things, none of them a feather width. The cap is solved
/// for at [`CAP_AZIMUTHS`] azimuths and the boundary's widest direction can
/// fall between two of them; the shader reads the ray's axis as a row of the
/// mounting against the unnormalized ray while the model reads it off the
/// normalized one; and the bisection stops a billionth of a cosine short.
/// Measured on the X4 Air fixture, the first of those is 0.016 degrees on
/// lens 0 and 0.007 on lens 1 and the other two are far below it
/// (`the_cap_is_tight_against_the_support`), so half a degree is thirty times
/// the worst of them. What it costs is 0.4% of the sphere in projections that
/// turn out to weigh nothing.
const CAP_MARGIN_DEG: f32 = 0.5;

/// Azimuths the coverage cap is solved at.
///
/// The boundary is not a circle: `fx` and `fy` differ and the tangential
/// terms are not radially symmetric at all, and on the X4 Air fixture it runs
/// 0.47 degrees of spread on lens 0 and 0.66 on lens 1. Eight samples land
/// within 0.02 degrees of its widest point anyway, which
/// `the_cap_is_tight_against_the_support` measures rather than assumes, and
/// [`CAP_MARGIN_DEG`] is what covers the rest.
const CAP_AZIMUTHS: usize = 8;

/// How wide the handover between the two lenses asks to be, in degrees of
/// world angle, centred on the seam (issues #48 and #103).
///
/// The overlap is 14 degrees and before issue #48 it was the band: the weights
/// crossed over across the whole of it, so anything the two lenses disagree
/// about was drawn twice across 10 degrees of picture. Issue #48 cut that to
/// **2**, which is what the recorded blend table
/// (docs/research/insv-format.md 6.8) says the sharpest handover is: scored
/// against the front lens alone, 2 degrees keeps 0.687 of that sharpness where
/// the whole-overlap weights keep 0.518 and a hard cut would keep 0.721.
///
/// **8 is what the owner's eye chose, against those numbers rather than with
/// them** (2026-08-05, label-blind, two arms of one binary). He ran both arms
/// without being told which was which and said *"2 is way better. Def not
/// perfect but way better"* of the 8, while the corridor's own step statistics
/// got worse rather than better. Every instrument in the sweep behind that call
/// is **monotone** in this number - sharpness falls, the doubled band grows,
/// the shear falls, all smoothly and with no knee at any of 2, 4, 6, 8 and 12 -
/// so no instrument could have picked a width and none was asked to.
///
/// What widening costs, measured through **this map** rather than through the
/// instrument's own linear ramp (`--bin seam mode=blend`, the `shipped` row, at
/// the July-14 anchor moment, yaw 90, fov 60, the file's own fit): the band
/// where both lenses are over a tenth of the picture goes from 1.50 degrees at
/// 2 to 4.78 at 8, and that band's gradient energy against the front lens alone
/// falls 12 percent over the same pixels, 1.309 to 1.150. What the sweep did
/// settle is the other end: 12 is refused by the optics on every camera in the
/// corpus (`Reframe::overlap`, which is what bounds it now that the fold
/// apparatus is gone).
///
/// What bounds it from below is **shear**, the two lenses' disagreement
/// divided by the band: above 1 the crossover folds the picture rather than
/// blending it. That is why 2 could not ship before the calibration fit above
/// it, and it is the axis widening buys on - but not as `1 / width`, which is
/// what issue #161 assumed. The weights are cosines of the two lens axes and
/// not a distance, so the walk from nine tenths of the correction to one tenth
/// spends 0.75 of a 2 degree crossover and 0.61 of an 8
/// (`the_along_seam_correction_hands_over_across_the_whole_crossover`): four
/// times the width spreads the disagreement over 3.2 times as much picture,
/// and the shear falls by that rather than by four.
///
/// This is what the picture **asks for** and not always what it draws. One
/// thing sits between: the camera's own overlap, which clamps it per file
/// ([`Reframe::crossover`]). It used to be two - stage 4 made this a **floor**
/// rather than a width, and a near-field reading could open the band past it -
/// and since the flat seam it is a width again. Nothing the band measures
/// changes how wide the handover is any more, because nothing the band
/// measures reaches the picture at all: see [`Reframe::blend`].
///
/// **The owner re-confirmed the width on 2026-08-08**, live, at three widths
/// in one window with the arms swapped on the frame he was looking at
/// (`~/kjerag-ab/sessions/handover-demo.ab`, 3 / 8 / 12 degrees), and again
/// when he authorized this architecture: *"you can merge the existing stitch
/// with a wide band"*. Wide it is, and 8 is what wide has meant here since
/// 2026-08-05.
pub(crate) const CROSSOVER_DEG: f32 = 8.0;

/// Research only: what this run asks the handover for instead of
/// [`CROSSOVER_DEG`], from `KJERAG_HANDOVER_DEG`, in degrees.
///
/// Unset, which is every shipped run and every run that does not name it, is
/// [`CROSSOVER_DEG`]. Set to a width, the whole handover opens to it: the
/// weights cross over across that many degrees, and so does everything the
/// weights carry - the epipolar bend, which is split by them, and the
/// along-seam correction, which lens 1 takes whole and the weights hand over.
/// Either way the camera's own overlap still has the last word
/// ([`Reframe::crossover`]).
///
/// **One knob and not two, because there is only one support.** This used to
/// carry a longer argument, about an along-seam term applied over a whole lens
/// and ramped into the picture by the weights, which is why it could not be
/// given a support of its own. Nothing is applied over a lens any more
/// ([`Reframe::blend`]: the seam is flat), so the argument survives in its
/// short form: the crossover is the only thing here with a width, and this is
/// what sets it.
///
/// **It stays because it is how this width was chosen.** The 8 above is one
/// label-blind verdict at one pair of widths, staged as two arms of one binary
/// through this variable; the next question about the width will be asked the
/// same way, and a rebuild per arm is what makes a session take a day instead
/// of an evening. Not a setting, not a key and not a menu item (AGENTS.md,
/// zero-config playback): an environment variable, read once, written nowhere.
const HANDOVER_DEG: &str = "KJERAG_HANDOVER_DEG";

/// The widest width the research switch will take, in degrees: the whole
/// overlap of the camera family it was written for.
///
/// A guard against a typo and not the bound that matters. What actually caps
/// the handover is the file's own calibration, which is a smaller number on
/// every camera in the corpus ([`Reframe::overlap`]): 14.44 to 15.02 degrees
/// over six X4 Air files and 9.19 on the ONE X2. Those figures were 9.36 to
/// 9.82 and 4.18 until the flat seam, when the bound stopped being the overlap
/// minus a bend's reach and became the bare overlap.
const OVERLAP_DEG: f32 = 14.0;

/// How wide the handover asks to be on this run, in degrees, which is
/// [`CROSSOVER_DEG`] unless [`HANDOVER_DEG`] asked for another width.
///
/// Read once, and read only by [`Reframe::crossover`], which is where the
/// camera clamps it and where both halves of the map take it from.
///
/// **The line it prints is about the ask and says nothing about the width.**
/// No file is open when this runs, so the width drawn is not known here and
/// cannot be: at `KJERAG_HANDOVER_DEG=12` on a file that affords 9.69 the ask
/// and the width differ by more than the whole change this switch was built to
/// stage. What is drawn is said per file by the shell, off the lenses the pass
/// will draw with (`Scene::handover_deg`, printed by the app's `say_handover`
/// after the stored calibration lands, and again by `fit_into` if a fallback
/// fit moves it).
fn crossover_deg() -> f32 {
    static WIDTH: OnceLock<f32> = OnceLock::new();
    *WIDTH.get_or_init(|| {
        let Ok(asked) = std::env::var(HANDOVER_DEG) else {
            return CROSSOVER_DEG;
        };
        match handover(&asked) {
            Ok(width) => {
                println!(
                    "blend:  research handover on, {HANDOVER_DEG}={width}: the handover asks for \
                     {width} degrees of world angle instead of {CROSSOVER_DEG}. what each file \
                     draws is that clamped by its own two lenses, on its own blend line at open"
                );
                width
            }
            Err(said) => {
                eprintln!(
                    "kjerag: {said}, so the handover stays at the {CROSSOVER_DEG} degrees it ships \
                     with"
                );
                CROSSOVER_DEG
            }
        }
    })
}

/// The width [`HANDOVER_DEG`] asked for, or what is wrong with the ask.
fn handover(asked: &str) -> Result<f32, String> {
    let width = asked
        .parse::<f32>()
        .map_err(|e| format!("{HANDOVER_DEG}={asked}: {e}"))?;
    match width.is_finite() && width > 0.0 && width <= OVERLAP_DEG {
        true => Ok(width),
        false => Err(format!(
            "{HANDOVER_DEG}={asked} is not a width between 0 and the {OVERLAP_DEG} degrees the two \
             lenses overlap by"
        )),
    }
}

/// How many lenses one pass can sample.
///
/// Every camera in the format study is a back-to-back pair, and the two
/// bindings per lens are declared in WGSL rather than indexed, so this is a
/// constant rather than a length. A file that describes more lenses than
/// this has the rest ignored, which is a picture with a hole in it and not a
/// crash.
pub const MAX_LENSES: usize = 2;

/// The field of view a flat frame stops being a window at, and so where the
/// output projection starts to bend (issue #47).
///
/// It is the cap the zoom used to stop at, read as what it was: a rectilinear
/// view stretches its corners by `1 / cos` of the angle out to them, which is
/// 3.1x at the corners of a 110-degree 16:9 view and runs away to infinity at
/// 180. Under it nothing about the picture changes, and [`Screen::shrink`] is
/// exactly 1.
pub(crate) const FOV_FLAT: f32 = 110.0 * PI / 180.0;

/// How much of the frame's shorter side the whole sphere fills at the far end
/// of the zoom, which is what caps it ([`fov_ceiling`]).
///
/// The ball is round and a window is not, so this is measured against the
/// shorter side: 0.8 leaves a tenth of it as room on the two near edges and
/// more on the others. It is a look rather than a measurement, and the one
/// number in this file the owner is expected to have an opinion about.
const BALL_FILL: f32 = 0.8;

/// The output projection: how a point of the frame becomes a ray, at one
/// field of view and one window shape.
///
/// **The family.** A plane radius `r` from the middle of the frame is the
/// direction `theta` off the view axis with `r = tan(shrink * theta) /
/// shrink`. At `shrink` 1 that is `r = tan(theta)`, the flat window every
/// perspective view is; at 1/2 it is `r = 2 tan(theta / 2)`, which is
/// stereographic, which is the tiny planet; and below that the whole sphere
/// closes into a disc of finite radius with nothing outside it. One parameter
/// walks all three, and every one of them meets the next in value and in
/// slope, so a scroll through the range has nowhere to pop.
///
/// **The schedule.** `shrink` is `FOV_FLAT / fov`, held at 1 until the view
/// is wider than that. Past there the product `shrink * fov / 2` is constant,
/// which is worth reading twice: the frame keeps the half angle of the widest
/// flat view, and widening the field of view shrinks the world into it
/// instead of stretching it. That is what makes the zoom keep meaning zoom
/// out through the bend, and `the_picture_only_ever_shrinks` is the check.
///
/// The mirror of `struct Screen` in `WGSL`, and part of the uniform block.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Screen {
    /// The plane radius the middle of the frame's left and right edges sit
    /// at: `tan(shrink * fov / 2) / shrink`.
    half_extent: f32,
    /// How much of a real angle the flat frame sees. 1 is the plain
    /// perspective view, 1/2 is stereographic, and the far end of the zoom is
    /// about 0.18.
    shrink: f32,
    /// The plane radius the sphere ends at, which is where `theta` reaches
    /// half a turn. Past it the frame is looking at nothing at all, and the
    /// pass writes nothing at all: the room the ball sits in is transparent
    /// and what fills it is whatever the shell put behind the video
    /// (issue #100).
    ///
    /// [`f32::MAX`] wherever the sphere has no edge in the plane: at `shrink`
    /// of 1/2 and above, `theta` cannot reach half a turn however far out the
    /// frame goes, and a stereographic or flatter view fills its frame.
    ball_radius: f32,
    /// Output width over output height. The vertical field of view is
    /// whatever this leaves.
    aspect: f32,
}

impl Screen {
    fn new(camera: Camera, aspect: f32) -> Self {
        let shrink = (FOV_FLAT / camera.fov).min(1.0);
        Self {
            half_extent: (shrink * camera.fov * 0.5).tan() / shrink,
            shrink,
            ball_radius: ball_radius(shrink),
            aspect,
        }
    }

    /// The ray a point of the output looks along, in view space: x right, y
    /// down, z forward. `uv` runs 0 to 1 across the output, y down.
    ///
    /// `None` is the room around the ball, where the frame has run off the
    /// sphere and there is no direction to answer with. Nothing else in this
    /// crate can return it: a flat frame is all sphere.
    ///
    /// WGSL twin: `view_ray`, whose `w` is this `Option`.
    fn ray(self, uv: [f32; 2]) -> Option<[f32; 3]> {
        let plane = [
            (uv[0] * 2.0 - 1.0) * self.half_extent,
            (uv[1] * 2.0 - 1.0) * self.half_extent / self.aspect,
        ];
        // The flat window, and the whole of it: two multiplies, the ray at z
        // of 1 and unnormalized, exactly the instructions this was before
        // issue #47. Ahead of the ball test rather than after it because a
        // flat frame is all sphere -- [`Self::ball_radius`] is [`f32::MAX`]
        // wherever `shrink` is 1 -- so the length below is work the range the
        // player already had would be paying for nothing.
        if self.shrink == 1.0 {
            return Some([plane[0], plane[1], 1.0]);
        }
        let radius = norm(plane);
        if radius > self.ball_radius {
            return None;
        }
        let theta = (self.shrink * radius).atan() / self.shrink;
        let (sin, cos) = theta.sin_cos();
        // The middle of the frame, where the azimuth is not defined and the
        // ray is the view axis itself.
        let out = match radius > 0.0 {
            true => sin / radius,
            false => 0.0,
        };
        Some([plane[0] * out, plane[1] * out, cos])
    }
}

/// The plane radius the sphere's far side lands at, for one [`Screen::shrink`].
///
/// `tan(shrink * pi) / shrink`, which is where `theta` reaches half a turn.
/// At a `shrink` of 1/2 or more that angle is a quarter turn or more into the
/// tangent's own asymptote: the far side is at infinity, the frame is all
/// picture, and there is no ball to leave room around.
fn ball_radius(shrink: f32) -> f32 {
    match shrink < 0.5 {
        true => (shrink * PI).tan() / shrink,
        false => f32::MAX,
    }
}

/// The far end of the zoom: the widest field of view worth offering, which is
/// the one where the whole ball sits in the frame at [`BALL_FILL`] of its
/// shorter side.
///
/// It depends on the window shape because the ball does not: a wide window
/// has to be zoomed out further than a square one before a round picture
/// clears its top and bottom. The solve is closed: past [`FOV_FLAT`] the ball
/// is `tan(shrink * pi) / shrink` across a frame `tan(FOV_FLAT / 2) / shrink`
/// wide, so the `shrink` cancels and what fraction of the frame the ball
/// fills depends on `shrink` alone.
pub(crate) fn fov_ceiling(aspect: f32) -> f32 {
    let shorter = aspect.max(1.0);
    let shrink = (BALL_FILL * (FOV_FLAT * 0.5).tan() / shorter).atan() / PI;
    FOV_FLAT / shrink
}

/// The uniform block, mirrored field for field by `struct Reframe` in
/// `WGSL`. All `f32`: the calibration is `f64` on the way in and the
/// composition below is done in `f64`, but a GPU uniform is `f32` and there
/// is nothing here that 24 bits of mantissa cannot hold (the largest number
/// is a pixel coordinate under 4096).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Reframe {
    /// One per lens, always [`MAX_LENSES`] of them however many the camera
    /// has: a uniform block cannot change size between draws. The ones past
    /// `lens_count` are [`LensBlock::EMPTY`].
    lenses: [LensBlock; MAX_LENSES],
    /// A `mat3x3<f32>` as WGSL lays one out. Takes a view-space ray to the
    /// camera **body**'s own frame, which is where the seam circle and the
    /// baseline are fixed (issue #103): the two lenses are glued to the body,
    /// so a direction on the seam is the same direction whatever the view is
    /// pointed at and however the horizon lock is turning under it.
    ///
    /// It is the middle two steps of [`view_to_lens`] with the mounting left
    /// off, so the composition is the pass's own and not a second convention.
    /// [`super::band`] is the only thing that reads it, on both sides: the
    /// compute pass turns it round to get from the body to each lens, and the
    /// fragment shader uses it to ask a ray which azimuth of the seam it is
    /// near.
    view_to_body: [[f32; 4]; 3],
    /// Where lens 1 sits relative to lens 0, in the body's frame, in metres:
    /// 33 mm of z on this camera family. What makes the overlap band a stereo
    /// pair, and zero for a file with one lens stream, which switches the
    /// band off rather than dividing by it.
    baseline: [f32; 3],
    /// A `vec3` in a uniform block is padded to sixteen bytes. WGSL does that
    /// itself; `repr(C)` does not.
    _baseline_pad: f32,
    /// How a point of the frame becomes a ray (issue #47). Sixteen bytes at a
    /// sixteen-byte offset, which is what a uniform block asks of a struct
    /// inside it.
    screen: Screen,
    /// The delivered frame size, shared: the streams of one file decode at
    /// one size, which `scene::calibrated` checks against the trailer.
    frame_width: f32,
    frame_height: f32,
    /// How many of [`Self::lenses`] have a decoded stream behind them. The
    /// older cameras write one lens per file, and then this is 1 and the
    /// picture is one hemisphere, exactly as it was before issue #27.
    lens_count: f32,
    linearize: f32,
    /// Which way across the delivered frame the sensor's rows advance
    /// (`kjerag_meta::Sweep`), and whether the correction runs at all: both
    /// components are zero for a file with no IMU record, and then the pass
    /// is what it was before issue #9, down to the instruction count.
    row_axis: [f32; 2],
    /// How far the magnification upgrade may engage on each plane (issue
    /// #11), luma first: 1 where it may, 0 for bilinear whatever the
    /// magnification. Two numbers rather than one because NV12's two planes
    /// are two grids and reach 1:1 an octave of zoom apart. [`Sampling`] is
    /// the names they come in.
    sharpen: [f32; 2],
    /// How wide this camera hands the picture over, in **radians**: what
    /// [`CROSSOVER_DEG`] asks for, or what these two lenses' overlap can carry
    /// if that is less ([`Self::afforded`]).
    ///
    /// In the block rather than written into the shader source, because it is
    /// a property of the file and the shader is compiled once before any file
    /// is open (`ScenePipeline::new`). Both halves of the map read it from
    /// here - [`Reframe::handover_width`] on this side and `handover` on the
    /// shader's - so the two cannot disagree about it.
    crossover: f32,
    /// How far across the seam the drawn 50/50 handover line is moved from
    /// where the pure geometry of the two axis cosines puts it, in **radians**
    /// ([`SeamAnchor`]).
    ///
    /// Zero is the geometric handover, which is the picture the player drew
    /// before the anchor and what `KJERAG_ANCHOR=off` still draws. Every
    /// caller that does not run the follow - every instrument, every test, the
    /// blank pane - gets that zero without asking for it, and zero is a
    /// literal `+ 0.0` in both twins.
    ///
    /// **It is one number because the law that produces it has one.** See
    /// [`SeamAnchor`] for why there are no states, no dissolves and no events
    /// behind it, and docs/research/studio-parity.md for the eye that ruled on
    /// it.
    ///
    /// **Sibling of [`Self::crossover`] and not a new field at the end.** It
    /// takes the first of the three padding words the table's alignment
    /// already needed, so the block is the size it always was and the table
    /// has not moved.
    ///
    /// **In radians, and not divided by the band it is about to be divided
    /// by.** A review asked for `shift / crossover` to be folded into this
    /// field, since the shader divides by the band anyway and the two are both
    /// in the same block: one division per fragment for free. It is not free.
    /// Measured on this box 2026-08-09, RADV's f32 divide is not the CPU's:
    /// over 40000 pairs drawn from every width a camera can draw and every
    /// shift the clamp allows, **11653 of them - 29 percent - differ, by up to
    /// 2 ulp** (the Vulkan spec allows 2.5 for `FDiv`). Precomputing the
    /// quotient would therefore feed the ramp a different number on about a
    /// third of the frames, which is enough to flip an output code, and the
    /// picture in this block is one the owner approved by eye and this branch
    /// is byte-identical to. The division stays where it is until there is a
    /// reason for it to move that is worth a re-approval.
    ///
    /// WGSL twin: `reframe.handover_shift`, read by `handover`.
    handover_shift: f32,
    /// What puts the table below on a sixteen-byte offset.
    ///
    /// **WGSL's alignment and not this struct's.** Every member of this block
    /// is an `f32` or an array of them, so `repr(C)` gives the whole thing an
    /// alignment of 4 and would happily start the table at 340. WGSL lays an
    /// `array<vec4<f32>, N>` out at 16, so the two definitions would then
    /// describe different bytes.
    ///
    /// **Nothing catches that at run time.** `min_binding_size` checks the
    /// block's total size and not one offset in it, and the sizes agree either
    /// way, so the shader would read the table shifted by twelve bytes and
    /// draw a wrong picture rather than refuse a pipeline. The test
    /// `the_uniform_block_is_the_size_wgsl_lays_it_out` is what checks it, and
    /// it checks the offset as well as the size for exactly that reason.
    ///
    /// Two words rather than three since the seam anchor took the first of
    /// them ([`Self::handover_shift`]).
    _pad: [f32; 2],
    /// What the along-seam axis still disagrees by after a pose, direction by
    /// direction, in radians (issue #103, stage 9).
    ///
    /// Last in the block because WGSL gives it a sixteen-byte alignment - it is
    /// an array of `vec4` there, whatever `repr(C)` makes of it here, which is
    /// an alignment of 4. Put anywhere else it would need padding in front of it
    /// as well as behind.
    ///
    /// It is a calibration and it travels with the calibration. Every caller
    /// that builds a map with a camera's correction in it gets this along with
    /// it, which is what lets an instrument reading the raw planes through
    /// [`Reframe::project`] read what the picture is drawn with rather than
    /// what it would have been without ([`Reframe::tabled`]).
    ///
    /// **No shader reads it since the flat seam.** It was read by `band_bend`,
    /// which applied it to lens 1 across the whole picture; that application
    /// retired with the rest of the bend (#164 had already refused the table
    /// on the evidence, and no shipped run ever set one). It stays in the
    /// block, rather than being moved to a CPU-only field, because this struct
    /// **is** the block: one definition, laid out once, checked against WGSL by
    /// one test. Splitting it in two to save 512 bytes a redraw would trade
    /// that invariant for nothing measurable.
    table: super::band::Table,
}

/// One lens's half of the block: the camera model, and where the lens is
/// pointing after the camera's own rotation.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct LensBlock {
    /// A `mat3x3<f32>` as WGSL lays one out: three columns, each padded to
    /// 16 bytes. Takes a view-space ray to this lens's frame.
    view_to_lens: [[f32; 4]; 3],
    xi: f32,
    fx: f32,
    fy: f32,
    cx: f32,
    cy: f32,
    k1: f32,
    k2: f32,
    k3: f32,
    p1: f32,
    p2: f32,
    image_radius: f32,
    /// The cosine of the widest angle off this lens's axis that can still be
    /// in its picture, widened by [`CAP_MARGIN_DEG`] and by whatever the
    /// readout turns the ray through (issue #10).
    ///
    /// A ray further off the axis than this weighs exactly nothing, so the
    /// pass does not run the model for it: one dot product decides, and the
    /// majority of the sphere that only one lens can see costs one projection
    /// instead of two. It comes out of the calibration by solving the model's
    /// own coverage boundary ([`coverage_floor`]), not out of a chosen angle:
    /// the band it bounds is the overlap the weights already blend across.
    ///
    /// 2 for a slot with no picture in it, which no ray can reach.
    axis_min: f32,
    /// The turn the body makes across one whole readout, in **this lens's**
    /// frame: a rotation vector, so a row's share of it is a multiplication
    /// (issue #9). Zero where there is no IMU record to read it from.
    ///
    /// Per lens rather than per camera because the two lenses are mounted a
    /// half turn apart, which is exactly why a readout displacement does not
    /// cancel between them at the seam.
    turn: [f32; 3],
    /// Which model [`lens_pixel`] runs for this lens: [`MEI`] or
    /// [`EQUIDISTANT`]. An `f32` because every member of this block is one,
    /// and a uniform block with a mixed member type is a layout to get wrong
    /// for nothing.
    model: f32,
}

/// Where a view ray lands in one lens's image, in delivered-frame pixels.
///
/// `inside` false means the ray missed this lens. Missing every lens is the
/// room around the ball, which the shader writes transparent; missing one of
/// two is ordinary, and is most of what [`Reframe::blend`] is weighing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    pub pixel: [f32; 2],
    pub inside: bool,
    /// The cosine of the angle between the ray and this lens's optical axis.
    /// 1 is straight down the axis and 0 is the seam great circle.
    pub axis: f32,
    /// How far the sample sits inside this lens's coverage: the distance in
    /// delivered-frame pixels from it to the edge of the image circle,
    /// positive inside and negative out. This is the distance transform from
    /// the lens's validity boundary that [`claim`] weighs with.
    pub depth: f32,
}

impl Landing {
    /// A lens the pre-test skipped, which is a lens the model was never run
    /// for (issue #10). Nothing reads it: it is paired with a weight of zero.
    ///
    /// WGSL twin: the zero-initialized `var landing: Landing` in `blend`.
    pub const MISSED: Self = Self {
        pixel: [0.0; 2],
        inside: false,
        axis: 0.0,
        depth: 0.0,
    };
}

/// How much of the picture at one output pixel comes from each lens, and
/// where in each lens's frame it comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blend {
    /// Meaningful only where the matching weight is above zero. A lens a ray
    /// cannot reach is [`Landing::MISSED`] rather than a projection of it,
    /// because the whole point of issue #10's pre-test is that the model does
    /// not run there.
    pub landings: [Landing; MAX_LENSES],
    /// One per lens, summing to 1 wherever any lens has the ray and all zero
    /// where none does.
    pub weights: [f32; MAX_LENSES],
}

impl Blend {
    /// Whether any lens has this ray at all. False is the room around the
    /// ball, which the shader writes transparent rather than painting.
    pub fn is_covered(&self) -> bool {
        self.weights.iter().any(|weight| *weight > 0.0)
    }
}

/// Where the camera body was when a frame was taken, and how the view is to
/// be held against it.
///
/// `body_from_world` is the inverse of the orientation `kjerag-meta`
/// integrated: it takes a direction in the stabilized world frame to the
/// body's own. Identity is horizon lock switched off, and then the view is in
/// body coordinates exactly as it was before issue #8.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Held {
    pub body_from_world: Quat,
    /// How the body moved **during** the frame, which is a different question
    /// from where it was (issue #9) and is answered whether or not the
    /// horizon is locked: the readout is the camera's own motion and not the
    /// display's. `None` is a file with no IMU record, and then the pass is
    /// what it was before issue #9.
    pub rolling: Option<Rolling>,
}

/// One frame's rolling shutter: the turn the camera body makes between the
/// first row of the readout and the last, and which way across the delivered
/// picture those rows run.
///
/// The turn is a rotation vector in the **body's own frame**
/// (`OrientationTrack::turn` over the readout window, centred on the frame's
/// instant), so a row's share of the readout scales it, and the ends of the
/// window are where it is exact.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rolling {
    pub turn: [f64; 3],
    /// A unit direction in delivered-frame pixels, from `kjerag_meta::Sweep`.
    pub axis: [f64; 2],
}

/// Whether the drawn handover line is held on world content instead of being
/// carried across it by the body's own turning, from `KJERAG_ANCHOR`.
///
/// **On is the shipped player.** `KJERAG_ANCHOR=off` (or `0`) is the research
/// escape that puts the 50/50 line back on the raw geometry, which is the
/// picture every build before 2026-08-08 drew and the arm every measurement of
/// the follow is read against. It is a way to answer "is the anchor doing
/// this?" in one run and it is not a setting: nothing in the window offers it,
/// it is read once, and it is written nowhere.
///
/// **`KJERAG_ANCHOR=` with nothing after it is UNSET, and says so.** It used to
/// mean off, on the reasoning that anything that is not a yes is a no, and that
/// cost a review a whole pass on 2026-08-09: a harness wrote
/// `env KJERAG_ANCHOR="$mode"` with `$mode` empty for its "leave it alone" arm,
/// every run of that arm silently drew the unanchored picture, and the digests
/// were compared against an anchored reference. An empty variable is what a
/// shell produces when a variable it is expanding is itself unset, so it is
/// overwhelmingly a mistake rather than a request; the way to ask for the
/// default is to not set it, which is `env -u KJERAG_ANCHOR`. A line on stderr
/// says which of the two happened, because a silent reinterpretation is the
/// thing that cost the pass.
///
/// Read once, because a value that changed mid-run would change it between two
/// frames of one pan.
pub fn anchoring() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let Ok(asked) = std::env::var(ANCHOR) else {
            return true;
        };
        if asked.is_empty() {
            eprintln!(
                "blend:  {ANCHOR} is set to nothing, which is read as UNSET and leaves the seam \
                 anchor ON. To turn it off say {ANCHOR}=off; to ask for the default say \
                 `env -u {ANCHOR}`"
            );
            return true;
        }
        let on = asked != "0" && !asked.eq_ignore_ascii_case("off");
        if !on {
            println!(
                "blend:  research seam anchor OFF, {ANCHOR}={asked}: the 50/50 handover line sits \
                 on the body's own geometry and is carried across the picture as the body turns, \
                 which is what the player drew before 2026-08-08"
            );
        }
        on
    })
}

/// The variable [`anchoring`] reads.
const ANCHOR: &str = "KJERAG_ANCHOR";

/// Research only, from `KJERAG_ANCHOR_TRACE`: whether every redraw says what
/// the held line is doing. Off in the app, on under the instruments that
/// measure the hold.
fn tracing() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("KJERAG_ANCHOR_TRACE").is_ok_and(|v| v != "0" && !v.is_empty())
    })
}

/// How hard the follow pulls the drawn line back toward the geometric handover
/// when the line is sitting AT the allowance, in reciprocal seconds.
///
/// The reciprocal of the shortest time constant the follow ever has, which is
/// a tenth of a second: at the rail the line is carried by the geometry with a
/// lag of one tenth of a second and no more.
const ANCHOR_FOLLOW_RATE: f32 = 10.0;

/// How steeply that gain falls away as the line comes in from the allowance.
///
/// **This is the whole of the deadband, and it is one term.** The gain is
/// `ANCHOR_FOLLOW_RATE * (delta / allowance)^ANCHOR_FOLLOW_POWER`, so at the
/// rail it is ten per second, at three quarters of the way out it is a half
/// per second, at half way out it is a hundredth, and at a quarter of the way
/// out it is a hundred-thousandth - one part in ten million of a degree per
/// frame, which is a line that does not move. There is no threshold in that
/// and nothing to click on: it is a single even power of one number, so it is
/// smooth everywhere including at zero, and so is every derivative of it.
///
/// Ten because that is what this corpus asks for. The owner's paramotor shakes
/// the geometry through 3.0 degrees of a 4.0 degree allowance at a couple of
/// hertz (measured at his `down1` line), so the deadband has to reach three
/// quarters of the way out and still be dead there, and the gain has to be
/// worth something by the time the line is at the rail. A tenth power does
/// both; a sixth leaks 0.14 degrees a frame at the shake's peak, and a
/// fourteenth makes the settle out of a hard turn abrupt.
const ANCHOR_FOLLOW_POWER: i32 = 10;

/// The longest step of film the hold is carried across, in seconds. Past this,
/// in **either** direction, the redraw is a discontinuity and the hold starts
/// again on the geometry ([`SeamAnchor::hold`]).
///
/// **Why there is one number here and not two.** This used to cap the step the
/// follow was charged for - `(at - was.at).min(CAP)` - which is a blunting and
/// not an answer: it still charged the follow with a target read off a world
/// direction from wherever the film used to be. Capping the step is what a
/// stale target needs least, because the leak at this step is already most of
/// the way to that target (the divisor is `(1 + POWER * RATE * dt)^(1/POWER)`,
/// which is 1.39 here, so a target a quadrant wide still slams the line to the
/// rail). So the cap became the threshold, the `min` is gone, and every step
/// the follow is charged with is now a step of film the picture actually ran.
///
/// **Why 0.25, honestly.** It has to be far above one frame and far below the
/// smallest seek the app can be asked for, and it is both by a wide margin:
///
/// - one frame is 1/30 s on every file in the corpus, 1/24 on the slowest film
///   anyone shoots and 1/120 on the modes both cameras have, so this is 7.5,
///   6.0 and 30 frames of continuous play;
/// - the jump keys and the jump buttons move `key_bind::JUMP`, which is 10
///   seconds, forty times this;
/// - the scrubber can ask for less, and a scrubber seek shorter than this is a
///   step of film the follow can honestly be charged for.
///
/// Nothing in a presentation time distinguishes a seek from a stall this long
/// or from a run of dropped frames, and this does not try to: all three are a
/// stretch of film that went by without the picture being drawn, and on the
/// far side of all three the content under the seam is not the content the
/// line was held on.
const ANCHOR_SEEK_SECS: f64 = 0.25;

/// Where the drawn handover line is, and what it costs the handover this
/// redraw.
///
/// **The problem.** Under a world-locked view the body turns and the view does
/// not, so the seam locus itself sweeps across the picture: the 50/50 line
/// walks over world content at whatever rate the aircraft is yawing, up to
/// 21.8 degrees a second on this corpus. Everything the seam gets wrong -
/// every doubled edge, every exposure step, every millimetre of misalignment -
/// therefore travels, and a defect that travels reads as a defect where a
/// defect that sits still reads as the picture. This is the owner's own theory
/// of why Insta360 Studio's seam is so much less visible in playback with
/// everything off, and it is what he judged this against.
///
/// **What it is not.** It does not make the seam more correct. Static
/// misalignment still doubles content either way; what this changes is whether
/// the doubling swims.
///
/// **One anchor and one offset for the whole ring**, not one per azimuth. The
/// line elsewhere on the seam circle still crawls, and that is deliberate:
/// what the owner is looking at is the piece of seam in front of him, and a
/// per-azimuth offset is a field, and a field that varies along the seam is a
/// warp of the picture rather than a slide of a line.
///
/// **What it cost to get here, and why there is no state in it.** flat4 held
/// the line and slewed it to a new anchor when it ran out of allowance, which
/// is a line that travels across the picture; the owner's word for it was
/// lurching. flat5 held TWO lines and dissolved between them, so that nothing
/// on screen ever travelled, and he refused that too: *"every now and then it
/// glitches. We need it to be smooth, that is a requirement. Perhaps we just
/// need the seam to be smoothly transitioning instead, probably simpler too,
/// with some fixing to prevent small movements when stopped at one position."*
///
/// He is right, and he is right about which way it is simpler. A dissolve is
/// an EVENT. An event has a first frame, a first frame is where a velocity
/// changes, and a velocity that changes inside one frame is the thing the eye
/// catches. The same goes for a promote, a retarget, a state and a clamp. So
/// there is not one of them in here. There is [`Self::delta`], and every
/// redraw it becomes one smooth function of what it was ([`Self::follow`]),
/// and that is the whole of the machinery.
///
/// Measured against the two-line version over the same thirty seconds of the
/// same film (the July-14 fast segment, 900 redraws): the drawn line's own
/// velocity changes by at most 11.3 degrees a second from one frame to the
/// next where the dissolving one changed by 59.2, and 11.3 is BELOW the 14.2
/// the unanchored geometry changes by. **This line is never rougher than the
/// picture it is drawn from.**
///
/// State lives here, on the CPU, and reaches the shader as one float
/// ([`Reframe::handover_shift`]).
#[derive(Clone, Copy, Debug)]
pub struct SeamAnchor {
    /// The world direction the drawn line stood on when this state was made.
    ///
    /// It is a world direction and not a number for the reason the whole
    /// mechanism exists: a frozen offset is fixed in the VIEW, and under a
    /// world-locked view the seam locus sweeps across the view as the body
    /// turns, so a frozen offset would walk the line across the content. Read
    /// back through the next redraw's pose it answers the one question the
    /// follow is about - what would it cost to leave the line exactly where it
    /// is on the content it is on - and that answer is [`Self::target`].
    ///
    /// Placed at the view centre's own piece of seam every redraw, which is
    /// what a pan and a seek both look like and what neither needs to report:
    /// [`Reframe::seam_ray_at`] inverts [`Reframe::across_seam`] exactly, so
    /// re-placing the anchor at the same offset on a different azimuth is the
    /// same offset and costs nothing.
    on: [f64; 3],
    /// Where the drawn 50/50 line is, as an offset across the seam from where
    /// the pure geometry puts it, in radians. The one number that goes to the
    /// GPU, and the one number that is state.
    delta: f32,
    /// The media instant this state was computed for, in seconds.
    ///
    /// What makes the follow a length of FILM and not a count of redraws: a
    /// redraw that arrives with no new frame behind it advances the clock by
    /// nothing, and [`Self::follow`] at a step of nothing is exactly the
    /// identity, so a run at 30 or at 300 fps follows over the same stretch of
    /// picture.
    at: f64,
    /// What the follow was aiming at and how hard it pulled this redraw, in
    /// radians and in reciprocal seconds. For the trace, and for nothing else.
    target: f32,
    gain: f32,
}

impl SeamAnchor {
    /// This redraw's offset, and the state that produced it.
    ///
    /// `held` is the pose the block was built for, so the world frame here is
    /// the one the view is locked to. With the horizon free that frame IS the
    /// body, the anchor never drifts, the offset stays zero, and the picture is
    /// the one `KJERAG_ANCHOR=off` draws - which is right: a body-fixed view
    /// has no crawl to hold against.
    ///
    /// `at` is the presentation time of the frame being drawn, in seconds.
    pub fn hold(state: Option<Self>, reframe: &Reframe, held: Held, at: f64) -> Self {
        let allowance = 0.5 * reframe.handover_width();
        if reframe.lens_count <= 1.0 || allowance <= 0.0 {
            return Self::rest(at);
        }
        let world_from_body = held.body_from_world.conjugate();
        let body_of = |world: [f64; 3]| held.body_from_world.rotate(world).map(|c| c as f32);
        // What one world direction costs the handover right now: minus its
        // across-seam angle is the offset that puts it back at 50/50.
        let offset_of =
            |world: [f64; 3]| -reframe.across_seam(reframe.view_ray_from_body(body_of(world)));
        // The 50/50 locus nearest the view centre, which is the piece of seam
        // the owner is looking at and where the anchor is kept.
        let centre = reframe.seam_nearest([0.0, 0.0, 1.0]);
        let world_of =
            |view: [f32; 3]| world_from_body.rotate(reframe.body_ray(view).map(f64::from));

        // The geometric target: the offset that would leave the drawn line
        // exactly where it is on the content it was drawn on last redraw. The
        // first redraw of a run has no line yet, and its target is the geometry
        // itself, which is where the line would have been anyway.
        //
        // **A redraw whose film is DISCONTINUOUS starts the run again**, and
        // that is the whole of the seek path. `was.on` is a world direction
        // from wherever the film was last drawn, so reading it back through
        // this pose after a seek answers a target that can be a quadrant wide.
        // Charging the follow with that slams the line to the rail on the seek
        // frame and then walks it back over the next second, which is a lurch
        // laid over the one frame where the whole picture already changed.
        // Anchoring afresh puts the line on the geometry there instead, which
        // is where it would have been if the file had been opened at that
        // instant. It is an event, and it is the one event this mechanism has:
        // a seek is already a discontinuity in every pixel, so there is no
        // velocity in the picture for it to break.
        //
        // **In EITHER direction, since 2026-08-09.** Until then only backward
        // film took this arm, on the reasoning that film time can only go down
        // if the pilot has seeked - which is true and is not the whole set. A
        // FORWARD seek reuses the stale anchor exactly the way a backward one
        // used to: measured on this fixture, a forward seek of six seconds
        // lands the line 2.90 degrees off the geometry on the seek frame and
        // then walks it back, which is the same defect with the same shape.
        // The argument the backward arm was given - a seek is a discontinuity
        // in every pixel - never mentioned which way the clock moved.
        //
        // What separates the two cases is therefore the SIZE of the step and
        // not its sign ([`ANCHOR_SEEK_SECS`], which says how 0.25 s was
        // picked). Nothing in a presentation time tells a seek from a stall
        // that long or from a run of dropped frames, and nothing here tries
        // to: on the far side of all three the content under the seam is not
        // the content the line was held on.
        //
        // `at == was.at` is NOT that case and stays in the second arm: it is a
        // redraw with no new frame behind it, the step is zero, and the law at
        // a step of zero is exactly the identity (`Self::follow`).
        let continuous = |was: &Self| {
            let step = at - was.at;
            (0.0..=ANCHOR_SEEK_SECS).contains(&step)
        };
        let (target, step) = match state.filter(|was| was.on != [0.0; 3] && continuous(was)) {
            None => (0.0, 0.0),
            // Uncapped, because the arm above is now what bounds it: every
            // step that reaches here is between zero and `ANCHOR_SEEK_SECS`,
            // which is a stretch of film the picture really ran.
            Some(was) => (offset_of(was.on), (at - was.at) as f32),
        };
        let (delta, gain) = Self::follow(target, step, allowance);
        // **Clamped HERE, and that is what makes the anchor and the picture
        // agree about where the line is.** `Reframe::with_shift` clamps on the
        // way to the shader, so a `delta` past the allowance draws at the rail;
        // placing `on` at the unclamped value would then record the line as
        // standing somewhere it is not, and the next redraw's target would be
        // read off that fiction. The error does not decay - it is re-made every
        // frame the clamp fires - so it is a standing bias and not a transient.
        //
        // At film's 30 fps this cannot fire: the follow's own ceiling is
        // `allowance / (POWER * RATE * dt)^(1/POWER)`, which is 3.55 of these
        // 4.00 degrees at `dt = 1/30` whatever the drift
        // (`Self::follow`), so the picture the owner approved is
        // untouched by this line. It fires above 100 fps, where that ceiling
        // passes the allowance, and both cameras in the corpus have 120 fps
        // modes.
        let delta = delta.clamp(-allowance, allowance);
        Self {
            on: world_of(reframe.seam_ray_at(centre, -delta)),
            delta,
            at,
            target,
            gain,
        }
        .traced(allowance)
    }

    /// **THE UPDATE LAW, and the whole of it.**
    ///
    /// `target` is where the line has to be put to stand exactly still on the
    /// content it is on. `step` is how much film has gone by. The answer is
    /// where the line is drawn, and how hard it was pulled to get there.
    ///
    /// It is the closed-form flow of one first-order equation,
    ///
    /// ```text
    /// d(delta)/dt = -RATE * (delta / allowance)^POWER * delta
    /// ```
    ///
    /// which says: the line is carried by the geometry, and leaks back toward
    /// the geometric handover at a rate that is a very high power of how far
    /// out it has got. Integrated over a step of `dt` from `target` that is
    ///
    /// ```text
    /// delta = target / (1 + POWER * gain * dt)^(1 / POWER)
    /// ```
    ///
    /// and this is that line of arithmetic. Four things follow from it, and
    /// they are the four things the owner asked for.
    ///
    /// **It is one function and there is nothing else.** No states, no
    /// dissolves, no promotes, no retargets, no branch that fires on one frame
    /// and not the next. The only `match` in [`Self::hold`] is which of
    /// `target` and `step` a run's very first redraw gets, and both of its arms
    /// are numbers rather than behaviours.
    ///
    /// **It never jumps.** The flow is exact rather than a step of an
    /// integrator, so no size of `dt` can overshoot: the divisor is at least
    /// one, so `delta` is always between `target` and zero and never past
    /// either. At `dt = 0` the divisor is exactly one and the law is exactly
    /// the identity, which is why a redraw with no new frame behind it changes
    /// nothing at all.
    ///
    /// **Standing still costs nothing.** The gain is a tenth power, so at a
    /// quarter of the allowance it is one hundred-thousandth per second and the
    /// line moves by a ten-millionth of a degree a frame. The knee that turns
    /// that into a real follow is a single even power of one number: smooth
    /// everywhere, smooth at zero, and smooth in every derivative, so motion
    /// starting and motion stopping have nothing to click on.
    ///
    /// **The allowance is approached and not hit, AT 30 FPS.** The offset a
    /// sustained drift of `w` can hold is `allowance * (w / (RATE *
    /// allowance))^(1 / (POWER + 1))`, an eleventh root: 25 times the drift
    /// buys 34 percent more offset. Over the two segments this was tuned on the
    /// line reaches 3.53 of the 4.00 degrees it is allowed.
    ///
    /// **That eleventh root is the continuum answer and the law is applied per
    /// frame, so the rail depends on the frame rate.** The geometry's sweep
    /// arrives as a jump of `w * dt` and the leak is then charged at a gain
    /// read AFTER that jump, which over-charges it, and the over-charge is
    /// larger the coarser the step. Letting the drift run away gives the
    /// ceiling in closed form: as `target` grows the divisor grows with it, and
    ///
    /// ```text
    /// delta -> allowance / (POWER * RATE * dt)^(1 / POWER)
    /// ```
    ///
    /// so the largest offset this law can hold at all is a property of the film
    /// rate and nothing else. `POWER * RATE` is 100 per second, so it equals
    /// the allowance at exactly `dt = 0.01 s`:
    ///
    /// | film fps | 24 | 30 | 60 | **100** | 120 | 240 |
    /// | --- | ---: | ---: | ---: | ---: | ---: | ---: |
    /// | ceiling, deg, of a 4.00 allowance | 3.47 | **3.55** | 3.80 | **4.00** | 4.07 | 4.37 |
    ///
    /// **Read as a known characteristic and not as a defect with a fix
    /// pending.** Every file the owner has judged this on is 30 fps, where the
    /// ceiling is 3.55 and the map's own clamp is unreachable; that is also why
    /// [`Self::hold`]'s clamp is provably inert on the arm he approved. Both
    /// cameras in the corpus shoot 120 fps modes, and there the rail is the
    /// clamp rather than the law, which is a fade held one-sided at its widest
    /// for as long as the drift lasts. Making the law dt-invariant is a
    /// different picture at 30 fps as well, so it is not a change this merge
    /// may make; it is written down here, pinned by
    /// `tests::the_follow_has_a_ceiling_and_the_frame_rate_sets_it`, and it is
    /// the belt's neighbour on the list.
    fn follow(target: f32, step: f32, allowance: f32) -> (f32, f32) {
        // An EVEN power, so this is the magnitude without an `abs` and without
        // a branch, and the whole law is a polynomial in `target` divided by a
        // root of one. `debug_assert` rather than a comment because an odd
        // power here would silently push the line the wrong way on one side.
        debug_assert_eq!(ANCHOR_FOLLOW_POWER % 2, 0);
        let gain = ANCHOR_FOLLOW_RATE * (target / allowance).powi(ANCHOR_FOLLOW_POWER);
        let power = ANCHOR_FOLLOW_POWER as f32;
        (target / (1.0 + power * gain * step).powf(1.0 / power), gain)
    }

    /// The state a map with no seam in it leaves behind: no line, and the zero
    /// the block builds itself with.
    fn rest(at: f64) -> Self {
        Self {
            on: [0.0; 3],
            delta: 0.0,
            at,
            target: 0.0,
            gain: 0.0,
        }
    }

    /// One line per redraw under `KJERAG_ANCHOR_TRACE`, and the state
    /// unchanged. Parseable on purpose: the instruments that measure the follow
    /// read this and nothing else.
    fn traced(self, allowance: f32) -> Self {
        if tracing() {
            println!(
                "anchor: t={:.4} delta={:+.4} target={:+.4} gain={:.5} allow={:.4}",
                self.at,
                self.delta.to_degrees(),
                self.target.to_degrees(),
                self.gain,
                allowance.to_degrees(),
            );
        }
        self
    }

    /// Where the drawn 50/50 line is, in radians across the seam.
    pub fn shift(&self) -> f32 {
        self.delta
    }
}

/// A direction's own unit vector, or the direction where it has no length.
fn unit(ray: [f32; 3]) -> [f32; 3] {
    let reach = norm3(ray);
    match reach > 0.0 {
        true => ray.map(|c| c / reach),
        false => ray,
    }
}

impl Reframe {
    /// The block for one camera pose and the lenses of one file, in file
    /// order. Anything past [`MAX_LENSES`] is dropped.
    pub fn new(
        lenses: &[Lens],
        frame: Size,
        camera: Camera,
        held: Held,
        aspect: f32,
        linearize: bool,
        sampling: Sampling,
    ) -> Self {
        let mut block = Self {
            lenses: std::array::from_fn(|index| match lenses.get(index) {
                Some(lens) => LensBlock::new(lens, index, frame, camera, held),
                None => LensBlock::EMPTY,
            }),
            view_to_body: body_from_view(camera, held).columns(),
            baseline: super::band::baseline(lenses),
            _baseline_pad: 0.0,
            screen: Screen::new(camera, aspect),
            frame_width: frame.width as f32,
            frame_height: frame.height as f32,
            lens_count: lenses.len().min(MAX_LENSES) as f32,
            linearize: f32::from(u8::from(linearize)),
            row_axis: held
                .rolling
                .map_or([0.0; 2], |rolling| rolling.axis.map(|c| c as f32)),
            sharpen: sampling.limits(),
            // Filled below, because it is read off the lenses this block has
            // just laid out and there is nowhere earlier to read them from.
            crossover: 0.0,
            // No line held until a caller says otherwise
            // ([`Self::with_shift`]), and no shift is the geometric handover.
            handover_shift: 0.0,
            _pad: [0.0; 2],
            // Nothing measured until a caller says otherwise
            // ([`Self::with_table`]), which is the picture stage 6 drew.
            table: super::band::Table::REST,
        };
        block.crossover = block.afforded();
        block
    }

    /// The same map with a camera's along-seam table in it (issue #103, stage
    /// 9).
    ///
    /// A step of its own rather than an argument to [`Self::new`], because
    /// every caller that has no table at all - every instrument that is not
    /// asking about this stage, and the blank pane - would otherwise have to
    /// say so, and the thing they would be saying is that the picture is what
    /// it always was.
    pub fn with_table(mut self, table: super::band::Table) -> Self {
        self.table = table;
        self
    }

    /// The same map with the drawn handover line moved `shift` radians across
    /// the seam ([`Self::handover_shift`]).
    ///
    /// Clamped here as well as by [`SeamAnchor`], because the allowance is the
    /// map's own property and a shift past half the drawn fusion width would
    /// put the 50/50 line outside the fade it is supposed to live inside.
    ///
    /// **It is also the only thing standing between the anchor and a hole in
    /// the picture, which is worth writing down.** The handover's support is
    /// centred on the drawn line, so with a shift the ramp closes at
    /// `-band / 2 - shift` on one side and opens at `band / 2 - shift` on the
    /// other ([`crossover`]). A lens weighed at exactly zero where the OTHER
    /// lens has no picture either is a transparent pixel, and that needs the
    /// closing end to fall outside the shared picture, which is
    /// `|shift| > band / 2 + overlap / 2`. This clamp allows `band / 2`, so the
    /// margin is the whole of `overlap / 2` - 7.22 degrees on the X4 Air
    /// fixture and 4.59 on an X2-class camera - and the picture cannot have a
    /// hole in it while it holds.
    /// `tests::the_anchored_handover_leaves_no_hole_and_no_cliff` measures
    /// both halves of that: no hole at every shift this clamp allows, and a
    /// hole the moment a shift past it is planted straight into the block.
    ///
    /// In normal running it is a guard and not a mechanism: at film's 30 fps
    /// the follow's own ceiling is 3.55 degrees of these 4.00
    /// ([`SeamAnchor::follow`]), so no drift an aircraft can produce reaches
    /// this clamp at all. Above 100 fps that stops being true, which is why
    /// [`SeamAnchor::hold`] clamps as well and places its anchor on the clamped
    /// value rather than on the raw one.
    ///
    /// A step of its own, like [`Self::with_table`]: every caller that is not
    /// running the follow - every instrument, every test, the blank pane - gets
    /// zero without saying so, and zero is the geometric handover.
    pub fn with_shift(mut self, shift: f32) -> Self {
        let allowance = 0.5 * self.crossover;
        self.handover_shift = shift.clamp(-allowance, allowance);
        self
    }

    /// The direction the 50/50 handover locus is perpendicular to, in view
    /// space.
    ///
    /// The handover reads `axis0 - axis1`, which is one fixed vector against
    /// the ray ([`Self::axis_of`] is a dot product with a row of each
    /// mounting), so the locus where the two lenses claim the ray equally is
    /// the great circle perpendicular to the difference of those two rows.
    /// This is that difference, normalized.
    ///
    /// Zero for a one-stream file, which has no seam.
    pub fn seam_normal(&self) -> [f32; 3] {
        let apart = self.seam_apart();
        let reach = norm3(apart);
        match reach > 0.0 {
            true => apart.map(|c| c / reach),
            false => [0.0; 3],
        }
    }

    /// The un-normalized difference of the two mounting rows the handover
    /// reads, in view space. [`Self::seam_normal`]'s direction and
    /// [`Self::seam_spread`]'s length, in one place so the two cannot
    /// disagree.
    fn seam_apart(&self) -> [f32; 3] {
        std::array::from_fn(|c| {
            self.lenses[0].view_to_lens[c][2] - self.lenses[1].view_to_lens[c][2]
        })
    }

    /// How long that difference is: the scale between an angle off the seam
    /// plane and what [`Self::across_seam`] reports for it.
    ///
    /// For a unit ray, `across_seam(ray)` is `spread / 2` times the ray's
    /// component along [`Self::seam_normal`], so with the two lenses of a 360
    /// camera back to back the spread is very near 2 and the report is very
    /// near the angle itself. This is what [`Self::seam_ray_at`] inverts.
    pub fn seam_spread(&self) -> f32 {
        norm3(self.seam_apart())
    }

    /// How far a view ray is across the seam from the 50/50 locus, in radians,
    /// positive on lens 0's side.
    ///
    /// **The handover's own measure and not a second one.** [`crossover`]
    /// reads `apart / (2 * reach * band)`; this is that first quotient, so a
    /// shift of exactly minus this value puts this ray at 50/50 by
    /// construction. The two cosines stand in for the two angles the same way
    /// and to the same accuracy the handover already relies on.
    pub fn across_seam(&self, view_ray: [f32; 3]) -> f32 {
        let reach = norm3(view_ray);
        match reach > 0.0 {
            true => (self.axis_of(0, view_ray) - self.axis_of(1, view_ray)) / (2.0 * reach),
            false => 0.0,
        }
    }

    /// The point of the 50/50 locus nearest a view ray, as a unit view-space
    /// direction.
    ///
    /// The locus is a great circle, so the nearest point on it is the ray with
    /// its component along [`Self::seam_normal`] taken out. Down either lens's
    /// own axis there is no nearest point and the ray comes back unchanged;
    /// nothing there is near a seam anyway.
    pub fn seam_nearest(&self, view_ray: [f32; 3]) -> [f32; 3] {
        let normal = self.seam_normal();
        let along: f32 = (0..3).map(|c| normal[c] * view_ray[c]).sum();
        let flat: [f32; 3] = std::array::from_fn(|c| view_ray[c] - normal[c] * along);
        let reach = norm3(flat);
        match reach > 0.0 {
            true => flat.map(|c| c / reach),
            false => view_ray,
        }
    }

    /// A unit view direction beside `view_ray`'s piece of seam whose
    /// [`Self::across_seam`] is exactly `across`.
    ///
    /// The inverse of [`Self::across_seam`] restricted to the great circle
    /// through [`Self::seam_nearest`] and [`Self::seam_normal`]: take the
    /// nearest point on the 50/50 locus and tilt it off the seam plane by
    /// `asin(2 * across / spread)`. That is how the anchor's world direction
    /// gets PLACED rather than merely found, and it is exact, which is what
    /// lets [`SeamAnchor::hold`] re-place the anchor on the view centre's own
    /// azimuth every redraw for nothing.
    ///
    /// Past what the spread can reach, and on a one-stream file, it clamps to
    /// the seam plane's own pole rather than returning a direction that is not
    /// one.
    pub fn seam_ray_at(&self, view_ray: [f32; 3], across: f32) -> [f32; 3] {
        let spread = self.seam_spread();
        if spread <= 0.0 {
            return unit(view_ray);
        }
        let normal = self.seam_normal();
        let base = self.seam_nearest(view_ray);
        let rise = (2.0 * across / spread).clamp(-1.0, 1.0);
        let run = (1.0 - rise * rise).max(0.0).sqrt();
        std::array::from_fn(|c| base[c] * run + normal[c] * rise)
    }

    /// The table this map is drawing with.
    pub fn table(&self) -> super::band::Table {
        self.table
    }

    /// How wide this camera can hand the picture over, in radians: what
    /// [`CROSSOVER_DEG`] asks for, or what its own two lenses overlap by if
    /// that is less.
    ///
    /// A file with one lens stream has no overlap and no seam, and takes the
    /// ask untouched: nothing is ever handed over, so the number is never used
    /// and a clamp would be inventing a bound out of a camera that is not
    /// there.
    ///
    /// **THIS IS NOT A SAFETY BOUND, AND SAYING IT WAS COST A REVIEW ROUND.**
    /// The clamp keeps the handover's support inside the shared picture *when
    /// the drawn line sits on the seam*, which is the only case there was
    /// before [`SeamAnchor`]. With a line held on the world the support is
    /// centred on the DRAWN line and not on the seam, so it runs
    /// `band / 2 - shift` one way and `band / 2 + shift` the other
    /// ([`crossover`]), and `|shift|` is allowed up to `band / 2`
    /// ([`Self::with_shift`]). At the rail that is a **whole band** off the
    /// seam on one side, against `overlap / 2` of shared picture: 8.00 degrees
    /// into 7.22 on the X4 Air fixture and into 4.59 on a camera that overlaps
    /// the way the ONE X2 does. Measured on the owner's own footage through the
    /// held line's own trace, 2026-08-09: at `down1` the support never leaves
    /// the coverage (0 of 300 frames, worst 7.13 degrees into 7.28 a side), and
    /// over the July-14 fast segment it does on **202 of 900 frames**, by at
    /// most 0.09 degrees. Replay the same offsets on a camera that overlaps the
    /// way the X2 does and it is **866 of 900**, by up to 2.94.
    ///
    /// **What makes that safe is not this number.** A lens's claim is its share
    /// of the handover times its own coverage depth ([`claim`]), and the depth
    /// falls to zero exactly where that lens runs out of picture, so the outer
    /// lens is faded out by its own rim before the ramp ever asks it for a
    /// sample it does not have. The share the ramp is still handing it there is
    /// spent on nothing and the pair renormalizes to the lens that does have
    /// the ray. The two properties that matter - the weights always sum to
    /// one, and the delivered weight never steps - are asserted over the whole
    /// ring at the rail on both camera classes by
    /// `tests::the_anchored_handover_leaves_no_hole_and_no_cliff`, with a
    /// planted hole and a planted cliff as its controls. **The guard against a
    /// hole is [`Self::with_shift`]'s clamp**, and the margin it holds is
    /// `overlap / 2`, which that test measures rather than assumes.
    ///
    /// So what this clamp is actually for is the fade's SHAPE: it is what makes
    /// the crossfade a ramp the two lenses can both pay for rather than one the
    /// rim truncates. Widening it past the overlap would not put a hole in the
    /// picture; it would hand the outer edge of the handover to the coverage
    /// taper instead of to the ramp.
    fn afforded(&self) -> f32 {
        let asked = crossover_deg().to_radians();
        match self.overlap() {
            // The overlap itself, and that is the whole bound. An UNSHIFTED
            // handover of width `w` reaches `w / 2` off the seam on either side
            // (`super::band::reach`) and the shared picture runs `overlap / 2`
            // off it, so a band as wide as the overlap ends exactly on the rim
            // and a wider one would be asking a lens for picture it does not
            // have. There used to be a second term: the bend the band carried
            // moved the SAMPLE further off the seam than the ray was, so the
            // width had to leave room for it, and that room was worth 2.6
            // degrees on a roomy camera and a whole regime change on a tight
            // one (`super::band::affordable`, deleted with the bend). Nothing
            // moves a sample now.
            //
            // **The shift is not in this arithmetic and deliberately is not.**
            // Taking `|shift|` off the width here would narrow the fade every
            // time the line moved, which is the breathing width the owner
            // refused (`Self::handover_width`), and it would change the picture
            // he approved. The doc above says what carries the overshoot
            // instead.
            Some(overlap) => asked.min(overlap),
            None => asked,
        }
    }

    /// No frame to draw: one lens with no picture in it, so the map still runs
    /// and every ray misses.
    ///
    /// Missing every lens is the room around the ball, which the pass leaves
    /// transparent (issue #100), so a pane with no frame is all room and what
    /// shows is the backdrop the shell paints behind the widget. That is the
    /// whole of what this block does, and it is why nothing here says which
    /// case it is: a file that has not delivered its first frame yet and a
    /// window with no file are the same picture, and it is the same one the
    /// far end of the zoom already draws around the ball.
    pub fn blank(aspect: f32, linearize: bool) -> Self {
        Self {
            lenses: [LensBlock::EMPTY; MAX_LENSES],
            view_to_body: Mat3::IDENTITY.columns(),
            // No file, so no camera and no baseline: every ray misses every
            // lens and the band is never asked anything.
            baseline: [0.0; 3],
            _baseline_pad: 0.0,
            screen: Screen::new(Camera::default(), aspect),
            frame_width: 1.0,
            frame_height: 1.0,
            lens_count: 1.0,
            linearize: f32::from(u8::from(linearize)),
            row_axis: [0.0; 2],
            // Every ray misses every lens, so no plane is ever sampled.
            sharpen: Sampling::default().limits(),
            // One lens and no overlap, so nothing is ever handed over: the ask
            // itself, which is what a camera with room for it would get.
            crossover: crossover_deg().to_radians(),
            // No seam, so no line to hold anywhere.
            handover_shift: 0.0,
            _pad: [0.0; 2],
            // No file, so no camera and no calibration to carry.
            table: super::band::Table::REST,
        }
    }

    /// The block as the GPU reads it. Every field is an `f32` and `repr(C)`
    /// packs them, so there are no padding bytes and no invalid patterns.
    pub fn bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                std::ptr::from_ref(self).cast::<u8>(),
                std::mem::size_of::<Self>(),
            )
        }
    }

    /// The ray a point in the output looks along, in view space: x right,
    /// y down, z forward. `uv` runs 0 to 1 across the output, y down.
    ///
    /// `None` in the room around the ball (issue #47), which only the widest
    /// views have any of.
    pub fn view_ray(&self, uv: [f32; 2]) -> Option<[f32; 3]> {
        self.screen.ray(uv)
    }

    /// How much of this ray each lens shows, and where in its frame.
    ///
    /// Each lens stakes a [`claim`] on the ray and the claims are normalized
    /// against each other, so the weights sum to 1 wherever anything has the
    /// ray. Outside the overlap only one lens claims anything and its weight
    /// is exactly 1 (see [`share`]), so the pass takes the one sample the
    /// hard pick took before issue #7 and multiplies it by an exact one.
    ///
    /// The loop still runs [`MAX_LENSES`] times whatever the file holds, and
    /// the array writes in it are still unconditional, because a loop the
    /// shader compiler cannot unroll indexes its arrays dynamically and they
    /// go to scratch memory: measured on RADV 2026-07-31, that alone is 1.82
    /// ms per redraw against 1.68 at 2560x1440, more than the second texture
    /// fetch the blend actually needs costs. What is conditional is the
    /// **model**: [`Self::within`] is one dot product and it decides whether
    /// the projection runs at all (issue #10).
    ///
    /// **THE RAY IS THE RAY, and this is the flat seam** (owner's ruling,
    /// 2026-08-08). Nothing the band measures moves a sample here. Between
    /// stage 2 and 2026-08-08 it did: each lens was projected at a ray bent by
    /// the other lens's weight times what the two lenses disagreed by at this
    /// ray's azimuth, on both of the seam's axes, and the width of the
    /// handover was itself a function of that disagreement. All of it is gone,
    /// and what replaced it is nothing - the two pictures are fused, at the
    /// calibration they were fused at, and the eye judged that better than
    /// every morphing arm it was shown against.
    ///
    /// **The argument, in one line: a correction that is wrong is worse when
    /// it moves.** The corridor bend was a live per-frame estimate, and where
    /// it was right it bought alignment the eye did not miss when it went, and
    /// where it was wrong it swam - the same picture drawn a slightly
    /// different shape every frame. The owner has been calling that shimmer
    /// since 2026-08-05 and refusing arms over it. Turning it off cost the
    /// near-field alignment the instruments could measure and bought a seam
    /// that stands still, and he chose the still one, blind, on his own
    /// footage. docs/research/studio-parity.md is the whole record.
    ///
    /// **What is left is the honest doubling.** With no morphing, content the
    /// two lenses genuinely disagree about is drawn twice across the handover
    /// rather than smeared into one wrong shape. At the bad crossing that is
    /// visible as a doubled object; the wide band is what softens it, and the
    /// belt - a displacement learned over the whole picture rather than a ramp
    /// across a corridor - is what is meant to fix it, later and behind its own
    /// switch.
    ///
    /// Each lens stakes a [`claim`] on the ray and the claims are normalized
    /// against each other, so the weights sum to 1 wherever anything has the
    /// ray. Outside the overlap only one lens claims anything and its weight
    /// is exactly 1 (see [`share`]), so the pass takes the one sample the
    /// hard pick took before issue #7 and multiplies it by an exact one.
    ///
    /// Where the 50/50 line falls is [`Self::handover`]'s, and since the seam
    /// anchor that is the geometry plus [`Self::handover_shift`].
    ///
    /// WGSL twin: `blend`.
    pub fn blend(&self, view_ray: [f32; 3]) -> Blend {
        let mut landings = [Landing::MISSED; MAX_LENSES];
        let mut weights = [0.0; MAX_LENSES];
        let reach = norm3(view_ray);
        // Both axis cosines, once: the crossover below needs them together
        // and the cap test needs them one at a time, and computing them here
        // is what keeps this pass costing what it cost before the crossover
        // existed ([`Self::handover`]).
        let axis: [f32; MAX_LENSES] = std::array::from_fn(|lens| self.axis_of(lens, view_ray));
        let front = self.handover(axis, reach, self.crossover);
        for lens in 0..MAX_LENSES {
            if !self.covers(lens, axis[lens], reach) {
                continue;
            }
            let share = match lens {
                0 => front,
                _ => 1.0 - front,
            };
            landings[lens] = self.project(lens, view_ray);
            if lens < self.lens_count as usize {
                weights[lens] = claim(landings[lens], share);
            }
        }
        let total: f32 = weights.iter().sum();
        if total > 0.0 {
            for weight in &mut weights {
                *weight = share(*weight, total);
            }
        }
        Blend { landings, weights }
    }

    /// The front lens's share of this ray, which is what hands the picture
    /// from one lens to the other across the seam (issue #48).
    ///
    /// Taken from the **mounting** rather than from the two landings, and
    /// that is the whole reason it is a step of its own rather than two lines
    /// inside [`Self::blend`]'s loop. A value read back out of the `Blend`
    /// array after the loop that filled it cannot stay in registers: measured
    /// on RADV 2026-07-31 at 2560x1440 under live decode, doing it that way
    /// costs **5.5 ms per redraw against 3.6**, which is the same scratch
    /// memory trap the loop's own comment describes. `kjerag-spike --bin
    /// zoom`, which renders the pass with nothing else on the GPU, reads the
    /// two versions as equal; `--bin playback`, which runs it under live
    /// decode, is where the difference is.
    ///
    /// The cosines are the ray's own, before the readout turns it, where
    /// [`Landing::axis`] is the turned ray's. That is the better question
    /// anyway: both lenses read down their own pictures, which is one world
    /// direction, so the readout moves no content across the seam at all
    /// (0.000 degrees measured, docs/research/insv-format.md 6.7) and a
    /// crossover that followed it would swing with the camera for nothing.
    ///
    /// 1 for a file with one lens stream: it has no seam and takes no
    /// crossover, and its picture runs to the edge of its own coverage, 7
    /// degrees past where a seam would have been
    /// (`one_stream_keeps_the_whole_of_its_picture`).
    ///
    /// WGSL twin: `handover`.
    fn handover(&self, axis: [f32; MAX_LENSES], reach: f32, band: f32) -> f32 {
        match self.lens_count > 1.0 {
            true => crossover(axis[0] - axis[1], reach, band, self.handover_shift),
            false => 1.0,
        }
    }

    /// How wide this camera hands the picture over, in radians: what
    /// [`CROSSOVER_DEG`] asked for, clamped by what these two lenses overlap
    /// by ([`Self::afforded`]).
    ///
    /// **One number for the whole picture, and it does not breathe.** From
    /// issue #103's stage 4 until 2026-08-08 this took a measured disparity
    /// and opened the band wide enough to carry that reading's bend without
    /// folding, so the fade's own width moved with the near field, frame by
    /// frame. There is no bend to carry now, so there is nothing to open for,
    /// and a fade whose width breathes is one more thing at the seam that
    /// moves when the picture does not. The owner named that class of fault
    /// directly; docs/research/studio-parity.md carries the ruling.
    ///
    /// It is a plain accessor on purpose: it used to take an argument, and
    /// every caller that still passed one would have been passing something
    /// this no longer reads.
    pub fn handover_width(&self) -> f32 {
        self.crossover
    }

    /// A view-space ray in the camera body's own frame, which is where the
    /// seam circle and the baseline stand still (issue #103).
    ///
    /// WGSL twin: `reframe.view_to_body * ray`.
    pub fn body_ray(&self, view_ray: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|row| {
            (0..3)
                .map(|c| self.view_to_body[c][row] * view_ray[c])
                .sum()
        })
    }

    /// The inverse of [`Self::body_ray`]: a camera-body direction expressed
    /// in the named view's frame.
    ///
    /// `view_to_body` is a rotation, so its transpose is its inverse. A seam
    /// instrument holds its measurement sites in body coordinates, because
    /// that is where the seam circle and the baseline stand still, while
    /// [`Self::project`] takes the renderer's view-space ray like the shader
    /// does. This is that boundary, rather than each caller transposing the
    /// matrix again.
    pub fn view_ray_from_body(&self, body_ray: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|row| {
            (0..3)
                .map(|c| self.view_to_body[row][c] * body_ray[c])
                .sum()
        })
    }

    /// Which azimuth of the seam circle a ray is over, in radians from the
    /// body's +x, and the geometry of the band there.
    ///
    /// `None` straight down a lens's own axis, where there is no seam to be
    /// near and no azimuth to name.
    pub fn seam_at(&self, view_ray: [f32; 3]) -> Option<super::band::Ring> {
        let body = self.body_ray(view_ray);
        let reach = body[0].hypot(body[1]);
        (reach > 0.0)
            .then(|| super::band::Ring::at([body[0] / reach, body[1] / reach, 0.0], self.baseline))
    }

    /// What the band holds at a ray's azimuth, in radians, interpolated
    /// between the two cells it lands between.
    ///
    /// The field is a circle, so the lookup wraps: a step between neighbouring
    /// cells would be a step in the picture.
    ///
    /// WGSL twin: the `band[..]` lookup inside `band_bend`.
    pub fn reading_at(
        &self,
        view_ray: [f32; 3],
        cells: &[super::band::Cell],
        along: super::band::Along,
    ) -> super::band::Reading {
        let body = self.body_ray(view_ray);
        let reach = body[0].hypot(body[1]);
        if cells.is_empty() || reach <= 0.0 {
            return super::band::Reading::default();
        }
        let turn = body[1].atan2(body[0]) / std::f32::consts::TAU * cells.len() as f32;
        let low = turn.floor();
        let mix = turn - low;
        let cell =
            |step: usize| cells[(low.rem_euclid(cells.len() as f32) as usize + step) % cells.len()];
        let (a, b) = (cell(0), cell(1));
        super::band::Reading {
            epi: Self::channel(a, b, mix),
            // One fitted field over the whole circle rather than a cell
            // lookup: see `Along`. This azimuth's cosine and sine are the ray
            // flattened into the seam plane.
            along: along.at(body[0] / reach, body[1] / reach),
        }
    }

    /// The epipolar channel of one ray, weighted by the evidence behind it in
    /// each cell and taxed by how much of that evidence has reached
    /// [`KEEP`](super::band::KEEP).
    ///
    /// A direction that has stopped correlating stops contributing, and with
    /// no evidence at all the answer is zero, which is the picture before the
    /// band existed.
    ///
    /// The tax is each cell's own filtered gate
    /// ([`Cell::trust`](super::band::Cell::trust)), mixed - already clamped,
    /// and clamped on the cell side of the mix rather than after it, because a
    /// filter needs somewhere to keep yesterday and a fragment has nowhere.
    ///
    /// **That move of the clamp across the mix deepens the comb, and it is
    /// disclosed rather than fixed** (docs/research/seam-temporal.md 9.4).
    /// Until 2026-08-08 the tax was `clamp(mix(confidence) / KEEP, 0, 1)`, so a
    /// live cell beside a dead one was taxed by the PAIR's mixed confidence;
    /// now each side is clamped first and the mix is between a 1 and a 0.
    /// Measured on the halfway ray at a confidence of 0.95 beside 0.00, the tax
    /// falls 0.731 to 0.500, and at the owner's `down1` pair the notch in the
    /// middle of the corrected patch goes from 0.62 of the correction to 0.41,
    /// which is 34 percent deeper. It is the behaviour the owner picked blind
    /// on 2026-08-08 and it is what the named next build has to answer, on
    /// depth as well as on how many frames carry one.
    ///
    /// WGSL twin: `carry`.
    fn channel(a: super::band::Cell, b: super::band::Cell, mix: f32) -> f32 {
        let (ea, eb) = (a.confidence * (1.0 - mix), b.confidence * mix);
        let total = ea + eb;
        if total <= 0.0 {
            return 0.0;
        }
        let strength = a.trust + (b.trust - a.trust) * mix;
        (ea * a.disparity + eb * b.disparity) / total * strength
    }

    /// A body-frame axis, scaled, expressed in the named view's frame.
    ///
    /// `view_to_body` is a rotation, so its transpose is its inverse.
    fn out(&self, axis: [f32; 3], scale: f32) -> [f32; 3] {
        std::array::from_fn(|row| {
            scale
                * (0..3)
                    .map(|c| self.view_to_body[row][c] * axis[c])
                    .sum::<f32>()
        })
    }

    /// Where the stored table alone sends one lens's ray, before projection.
    ///
    /// Lens 1 takes it whole and lens 0 does not take it at all, which is how
    /// the retired bend applied it and how the calibration it belongs to
    /// is applied ([`super::seam::SeamFit`]). This is that one step on its
    /// own, for a reader that samples the raw planes through [`Self::project`]
    /// rather than drawing a picture: `seam::measure` reads a ring through the
    /// map the picture is drawn through, so what it answers is what is
    /// **still** wrong.
    ///
    /// The ray back unchanged on lens 0, on a map with no table, and straight
    /// down a lens's own axis, where there is no azimuth to look one up at.
    pub fn tabled(&self, lens: usize, view_ray: [f32; 3]) -> [f32; 3] {
        if lens != 1 || self.table.is_rest() {
            return view_ray;
        }
        let Some(at) = self.seam_at(view_ray) else {
            return view_ray;
        };
        let body = self.body_ray(view_ray);
        let reach = body[0].hypot(body[1]);
        let along = self.out(
            at.perp,
            self.table.at(body[0] / reach, body[1] / reach) * reach,
        );
        std::array::from_fn(|axis| view_ray[axis] + along[axis])
    }

    /// How far a ray is off one lens's axis, as an unnormalized cosine: one
    /// row of the mounting against the ray.
    ///
    /// WGSL twin: `axis_of`.
    fn axis_of(&self, lens: usize, view_ray: [f32; 3]) -> f32 {
        let block = &self.lenses[lens];
        (0..3).map(|c| block.view_to_lens[c][2] * view_ray[c]).sum()
    }

    /// How many delivered-frame texels one output pixel covers where it
    /// lands in this lens's picture: the local Jacobian of the whole backward
    /// map, which is what says whether the view is magnifying the source
    /// (issue #11).
    ///
    /// Under 1 an output pixel sits inside one texel and the picture is being
    /// magnified, which is what [`super::sampling`] upgrades for; over 1 it
    /// spans several and bilinear is the right answer. Taken as the longer of
    /// the two screen axes' steps, so a landing that is magnified one way and
    /// minified the other counts as not magnified: the upgrade is for
    /// pictures that have run out of texels, and the axis that has not is the
    /// one that would show the resampling.
    ///
    /// It is a **local** number and has to be. The fisheye's own density
    /// varies across its picture (1106 texels per radian down the X4 Air's
    /// axis, 948 radially at the rim), the rectilinear output's varies across
    /// the view, and the lens's landing is what carries both. `output` is the
    /// target's size in pixels; the value scales with it, which is why a
    /// screenshot magnifies less than the window it was taken from.
    ///
    /// WGSL twin: `texel_ratio`, which reads the same two steps off the
    /// hardware's own quad derivatives. That is the same finite difference,
    /// and it needs no output size at all: whatever target the pass draws
    /// into, a quad of it steps the share of the picture it steps.
    pub fn texels_per_pixel(&self, lens: usize, uv: [f32; 2], output: Size) -> f32 {
        let landing = |uv: [f32; 2]| Some(self.project(lens, self.view_ray(uv)?).pixel);
        let Some(here) = landing(uv) else {
            return f32::INFINITY;
        };
        let step = |to: [f32; 2]| match landing(to) {
            Some(moved) => (moved[0] - here[0]).hypot(moved[1] - here[1]),
            // A quad that straddles the edge of the ball, where the step
            // across is the whole picture. The WGSL twin reads exactly that
            // off its own derivative, because the lane outside has no landing
            // in it, and a huge ratio is a magnification of none: the upgrade
            // switches off at the rim rather than guessing.
            None => f32::INFINITY,
        };
        let across = step([uv[0] + 1.0 / output.width as f32, uv[1]]);
        let down = step([uv[0], uv[1] + 1.0 / output.height as f32]);
        across.max(down)
    }

    /// Whether **any** ray of the whole output can be in this lens's picture,
    /// with `margin` radians of slack on top.
    ///
    /// [`Self::within`] asked per pixel; this asks it once for the view, which
    /// is what a decision about the decoder rather than about a fragment
    /// would need. The output sits inside a cone about the view axis whose
    /// half angle is [`Self::cone`], so the ray of it nearest this lens's axis
    /// is that much nearer than the view axis is, and the whole test is one
    /// cosine against one dot product of the mounting.
    ///
    /// Conservative in the same direction as `within` and for the same
    /// reason: the cone contains the output rather than being it, so a corner
    /// that would only have grazed the lens still counts as reaching it.
    ///
    /// **Nothing in the player calls this.** The decoder is not gated on it:
    /// issue #10's other half was built as far as this test, measured, and
    /// cut, because on real footage with the horizon locked the answer holds
    /// for 9% of the time at the default field of view and letting go of it
    /// costs 195 to 340 ms of stale far hemisphere. `kjerag-spike --bin
    /// gating` is that measurement and this is what it reads; the numbers and
    /// the reasoning are in docs/ROADMAP.md.
    pub fn reaches(&self, lens: usize, margin: f32) -> bool {
        let block = &self.lenses[lens];
        // A slot with no picture in it. No cone reaches a cap that no ray is
        // inside.
        if block.axis_min > 1.0 {
            return false;
        }
        let cap = block.axis_min.acos() + self.cone() + margin;
        cap >= std::f32::consts::PI || block.view_to_lens[2][2] > cap.cos()
    }

    /// The half angle of the cone that holds the whole output, in radians:
    /// the corner ray, which is the furthest from the view axis a rectangle
    /// reaches.
    ///
    /// Half a turn once the corner has run off the sphere, which is a cone
    /// that holds everything: a view with the ball inside it is looking at
    /// the whole world at once.
    pub fn cone(&self) -> f32 {
        match self.view_ray([0.0, 0.0]) {
            Some(corner) => normalize(corner)[2].clamp(-1.0, 1.0).acos(),
            None => PI,
        }
    }

    /// How far off its own axis this lens can still see, in radians, cap
    /// margin included. `None` for a slot with no picture in it.
    ///
    /// For the instruments: `kjerag-spike --bin gating` reports it, and it is
    /// [`LensBlock::axis_min`] read back as an angle.
    pub fn coverage(&self, lens: usize) -> Option<f32> {
        let axis_min = self.lenses[lens].axis_min;
        (axis_min <= 1.0).then(|| axis_min.acos())
    }

    /// How wide the ring is where **both** lenses have the picture, in
    /// radians: the two caps' angles added and half a turn taken off.
    ///
    /// It is what bounds how far the crossover may open (issue #103, stage 4),
    /// which is why it is measured off the file's own calibration rather than
    /// quoted at 14 degrees from the format study. The caps are read tight
    /// here, without [`CAP_MARGIN_DEG`] and without the readout's own share:
    /// a bound computed off a generous cap does not bind.
    ///
    /// `None` for a file with one lens stream, which has no overlap and no
    /// seam, and for two lenses that do not reach each other at all.
    pub fn overlap(&self) -> Option<f32> {
        if self.lens_count <= 1.0 {
            return None;
        }
        let caps = cap(&self.lenses[0])? + cap(&self.lenses[1])?;
        (caps > PI).then_some(caps - PI)
    }

    /// Whether this lens can have any of this ray, decided before the model
    /// runs (issue #10).
    ///
    /// A lens's picture is one cap around its own axis, and
    /// [`LensBlock::axis_min`] is how wide that cap is. The mounting is a
    /// rotation, so the cosine the model would end up reading is one row of
    /// it against the ray over the ray's own length, which is a dot product
    /// and a compare against a division, a square root and a Mei evaluation.
    /// It is a **conservative** test and not the weight field's own support:
    /// false means the weight is exactly zero, true means it might not be.
    /// That asymmetry is what keeps the picture the picture. A lens kept and
    /// weighed zero is written and multiplied by nothing, which is what it
    /// was before; a lens wrongly dropped would be a hole.
    ///
    /// WGSL twin: `within`.
    pub fn within(&self, lens: usize, view_ray: [f32; 3]) -> bool {
        self.covers(lens, self.axis_of(lens, view_ray), norm3(view_ray))
    }

    /// The same test with the ray's own numbers already in hand, which is how
    /// [`Self::blend`] asks it: a dot product it has computed once for the
    /// crossover is not computed again here.
    fn covers(&self, lens: usize, axis: f32, reach: f32) -> bool {
        axis >= self.lenses[lens].axis_min * reach
    }

    /// The forward map: a view ray, through one lens's extrinsics and the
    /// Mei/UCM model, to a pixel of that lens's delivered frame.
    ///
    /// **Where the rolling shutter is taken out (issue #9).** The lens saw
    /// this ray when it read the row the ray lands on, not when the frame
    /// nominally began, so the orientation the ray is carried through has to
    /// be the one at that row's own instant. That is circular: the row picks
    /// the instant and the instant moves the row. It is solved by iteration
    /// from the frame's instant, [`READOUT_STEPS`] rounds of it, and each
    /// round is one more turn of the ray and one more pass through the model
    /// rather than a second sample of the picture. Nothing is resampled and
    /// no pass is added: this is the same backward map, with the camera's
    /// motion during the readout inside it.
    ///
    /// WGSL twin: `project`. The shader adds one line the mirror does not,
    /// turning the pixel into a texture coordinate (`frame_uv`).
    pub fn project(&self, lens: usize, view_ray: [f32; 3]) -> Landing {
        self.solve(lens, view_ray, READOUT_STEPS)
    }

    /// The same map with the row solved for a chosen number of rounds, which
    /// is how [`READOUT_STEPS`] came to be the number it is rather than a
    /// guess: zero rounds is the map as it was before issue #9, and a solve
    /// run until it stops moving is what every other count is measured
    /// against (`kjerag-spike --bin rolling model=1`).
    ///
    /// The shader always runs [`READOUT_STEPS`] of them.
    pub fn solve(&self, lens: usize, view_ray: [f32; 3], rounds: usize) -> Landing {
        let block = &self.lenses[lens];
        let aimed = block.lens_ray(view_ray);
        let mut landing = self.lens_pixel(lens, normalize(aimed));
        if self.is_rolling() {
            for _ in 0..rounds {
                let share = self.readout_share(landing.pixel);
                let turned = turned(aimed, block.turn.map(|axis| axis * share));
                landing = self.lens_pixel(lens, normalize(turned));
            }
        }
        landing
    }

    /// Whether the readout correction runs at all. Off for a file with no IMU
    /// record, and then [`Self::project`] is what it was before issue #9.
    ///
    /// WGSL twin: the `reframe.row_axis` test in `project`.
    ///
    /// `pub(crate)` for one reader, [`crate::twin`]: this is the test the
    /// whole readout branch sits behind on both halves, so a fixture that
    /// leaves it false compares a `project` with its second half deleted. The
    /// twin asserts on it rather than assuming it.
    pub(crate) fn is_rolling(&self) -> bool {
        self.row_axis != [0.0; 2]
    }

    /// Where in the readout the row a landing sits on is exposed: -1/2 at the
    /// first row of the sensor, +1/2 at the last, and clamped, because a ray
    /// that missed this lens still has to answer.
    ///
    /// WGSL twin: `readout_share`.
    pub fn readout_share(&self, pixel: [f32; 2]) -> f32 {
        let across = pixel[0] / self.frame_width - 0.5;
        let down = pixel[1] / self.frame_height - 0.5;
        (across * self.row_axis[0] + down * self.row_axis[1]).clamp(-0.5, 0.5)
    }

    /// The lens model itself, for one of this block's lenses.
    ///
    /// WGSL twin: `lens_pixel`.
    fn lens_pixel(&self, lens: usize, p: [f32; 3]) -> Landing {
        lens_pixel(&self.lenses[lens], p)
    }
}

/// [`LensBlock::model`] for Mei/UCM, which is what every Insta360 capture is.
const MEI: f32 = 0.0;

/// [`LensBlock::model`] for the equidistant fisheye, which is what a DJI
/// `.OSV` is.
const EQUIDISTANT: f32 = 1.0;

/// A unit ray in one lens's own frame, to a pixel of that lens's delivered
/// frame, through whichever model that lens's calibration is written in.
///
/// Free of [`Reframe`] because [`coverage_floor`] runs it against a block that
/// is still being built, before there is a `Reframe` to index.
///
/// WGSL twin: `lens_pixel`.
fn lens_pixel(lens: &LensBlock, p: [f32; 3]) -> Landing {
    match lens.model == EQUIDISTANT {
        true => landed(lens, equidistant(lens, p), p[2], true),
        false => mei(lens, p),
    }
}

/// The equidistant fisheye: the angle off the axis, straight onto a radius.
///
/// `r = fx * theta`, and `fy / fx` squeezes the one axis against the other,
/// which is the whole model. No distortion polynomial: the four coefficients a
/// DJI `.OSV` carries do not describe this lens under any standard reading of
/// them, and `kjerag_meta::osmo` has the measurement that refused them.
///
/// The offset in pixels, before the principal point. WGSL twin: `equidistant`.
fn equidistant(lens: &LensBlock, p: [f32; 3]) -> [f32; 2] {
    // The ray is a unit vector, so its z IS the cosine of the angle off the
    // axis. `atan2` of the perpendicular reach against it rather than `acos`,
    // because `acos` loses its precision exactly where this model is used
    // most, which is the far half of a 199 degree lens.
    let reach = p[0].hypot(p[1]);
    let theta = reach.atan2(p[2]);
    // Straight down the axis, where the azimuth is not defined and the
    // radius is zero anyway.
    let scale = match reach > 0.0 {
        true => theta / reach,
        false => 0.0,
    };
    [lens.fx * p[0] * scale, lens.fy * p[1] * scale]
}

/// One model's pixel offset, as the [`Landing`] the rest of the pass reads.
///
/// `injective` is whether the map can be believed this far round, which is a
/// question the two models answer differently: Mei folds past a turning point
/// it computes, and the equidistant map is monotone in `theta` over the whole
/// sphere and never folds.
fn landed(lens: &LensBlock, offset: [f32; 2], axis: f32, injective: bool) -> Landing {
    let depth = lens.image_radius - norm(offset);
    Landing {
        pixel: [offset[0] + lens.cx, offset[1] + lens.cy],
        inside: injective && depth > 0.0,
        axis,
        depth,
    }
}

/// The Mei/UCM model: a unit ray in one lens's own frame, to a pixel of that
/// lens's delivered frame.
///
/// WGSL twin: `mei`.
fn mei(lens: &LensBlock, p: [f32; 3]) -> Landing {
    // The mirror parameter is why a ray past 90 degrees off axis still
    // has a finite projection: it only needs `z + xi > 0`. On this
    // camera family xi is above 1, so the guard never fires; it is here
    // for a model where xi is smaller than 1.
    let denom = p[2] + lens.xi;
    let x = p[0] / denom;
    let y = p[1] / denom;

    let r2 = x * x + y * y;
    let radial = 1.0 + r2 * (lens.k1 + r2 * (lens.k2 + r2 * lens.k3));
    let xd = x * radial + 2.0 * lens.p1 * x * y + lens.p2 * (r2 + 2.0 * x * x);
    let yd = y * radial + 2.0 * lens.p2 * x * y + lens.p1 * (r2 + 2.0 * y * y);

    let offset = [lens.fx * xd, lens.fy * yd];
    // How far round the map can be believed, which is not as far as it
    // answers. The distance from the principal point grows with the angle
    // off the axis only up to `cos(theta) = -1/xi`; past that turning
    // point it comes back down, re-enters the image circle, and a ray
    // from behind the lens lands a second time on a pixel that belongs
    // to a ray in front of it. That second landing is issue #30's ghost,
    // a raw circular fisheye hanging behind the reframed view, and the
    // radius test cannot see it because the fold puts it well inside the
    // circle. Every lens needs it: with two of them the fold is a ghost
    // of the other hemisphere, printed over a picture that is otherwise
    // correct. Vacuous where xi is below 1: there is no turning point
    // there, the radius runs away to infinity instead, and `denom` is the
    // limit that binds.
    let injective = p[2] * lens.xi > -1.0;
    landed(lens, offset, p[2], denom > 0.0 && injective)
}

/// The cosine of the widest angle off a lens's axis that can still be in its
/// picture, widened so that no ray the model would have kept falls outside
/// it. What [`LensBlock::axis_min`] holds, and issue #10's whole shader half.
///
/// It is **solved rather than stated**: the boundary is wherever the model's
/// own landing leaves the image circle, which the calibration decides through
/// the mirror parameter, the focal lengths and three radial coefficients. A
/// number written here instead would be right for one camera.
///
/// The solve is a bisection, and what makes that legal is that a lens's
/// picture is one cap: swept from its axis outwards, `inside` goes off once
/// and stays off, which is issue #30's guard and
/// `each_lens_picture_stops_once`.
///
/// It runs per redraw rather than once per file, because what it widens by is
/// per frame, and it is cheap enough that keeping it beside the block it
/// describes beats caching it: a whole [`Reframe::new`], both lenses solved,
/// measured at 4.7 us against the 0.20 ms it takes off the pass.
///
/// `widen` is the readout's share, in radians: with issue #9's correction on,
/// the model is handed a ray turned by up to half of `turn`, so the cap has
/// to cover where that ray can land as well as where this one does.
fn coverage_floor(block: &LensBlock, widen: f32) -> f32 {
    // A slot with no picture in it, which is every lens past the file's own
    // count: no ray is ever in it and none is worth projecting.
    let Some(cap) = cap(block) else {
        return 2.0;
    };
    (cap + widen + CAP_MARGIN_DEG.to_radians())
        .min(std::f32::consts::PI)
        .cos()
}

/// The same before anything is added to it: how far off its own axis this
/// lens can actually see, in radians. `None` for a slot with no picture in it.
///
/// Separate from [`coverage_floor`] because the two questions are different.
/// The pass wants a cap nothing it would have kept falls outside of, so it
/// takes the generous one; [`Reframe::overlap`] wants the boundary itself,
/// because a bound computed off a generous cap does not bind.
fn cap(block: &LensBlock) -> Option<f32> {
    if !inside_anywhere(block, 1.0) {
        return None;
    }
    let (mut outside, mut inside) = (-1.0f32, 1.0f32);
    for _ in 0..CAP_BISECTIONS {
        let middle = 0.5 * (outside + inside);
        match inside_anywhere(block, middle) {
            true => inside = middle,
            false => outside = middle,
        }
    }
    Some(outside.clamp(-1.0, 1.0).acos())
}

/// Halvings of the coverage bisection. Thirty leaves a billionth of a cosine,
/// which is a thousand times finer than the float the shader compares in.
const CAP_BISECTIONS: usize = 30;

/// Whether any direction this far off the lens's axis is in its picture. The
/// boundary is not a circle, so this is the round of it that
/// [`CAP_AZIMUTHS`] can see.
fn inside_anywhere(block: &LensBlock, axis: f32) -> bool {
    let rim = (1.0 - axis * axis).max(0.0).sqrt();
    (0..CAP_AZIMUTHS).any(|step| {
        let (sin, cos) = (step as f32 * std::f32::consts::TAU / CAP_AZIMUTHS as f32).sin_cos();
        lens_pixel(block, [rim * cos, rim * sin, axis]).inside
    })
}

/// One lens's unnormalized claim on a ray, which [`Reframe::blend`] weighs
/// against the other lens's.
///
/// Two factors, and neither of them is a feather width chosen by taste:
///
/// - this lens's **share of the crossover**, [`crossover`], which is
///   [`CROSSOVER_DEG`] wide and centred where the two lenses are equally far
///   off their own axes;
/// - **coverage depth**, `landing.depth`, the distance transform from this
///   lens's own validity boundary. It reaches zero exactly where the picture
///   stops, so a lens fades out as it runs out of picture, and the rim of
///   the image circle, which is where vignetting lands and where the
///   distortion polynomial is least trustworthy (5.3), is down-weighted for
///   free. Outside the crossover it is multiplied by a share of exactly 1 or
///   exactly 0, so it decides nothing there and the rim it protects is the
///   band's own edge.
///
/// WGSL twin: `claim`.
fn claim(landing: Landing, share: f32) -> f32 {
    match landing.inside {
        true => share * landing.depth,
        false => 0.0,
    }
}

/// One claim's share of all of them. `total` must be positive; the caller
/// has nothing to normalize otherwise.
///
/// The lone claimant's share is written rather than divided out. Measured on
/// RADV 2026-07-31: a GPU `x / x` is a reciprocal multiply and lands an ulp
/// under 1.0, and multiplying a sample by that reaches an 8-bit picture as
/// one code on 6 pixels of a million. Writing it is what lets a one-stream
/// ONE X2 file render bit for bit what it rendered before the blend existed,
/// which it does at every yaw tested.
///
/// WGSL twin: `share`.
fn share(claim: f32, total: f32) -> f32 {
    match claim == total {
        true => 1.0,
        false => claim / total,
    }
}

/// A ray turned by a rotation vector: its direction is the axis and its
/// length is the angle, which is Rodrigues' formula (issue #9).
///
/// Written out rather than first-ordered as `v + turn x v`, which is two
/// instructions and looks tempting for an angle this small. It is not small
/// enough: the worst rate in 30 minutes of this footage is 523 deg/s, which
/// is 4 degrees over half a readout, and the term the first order drops is
/// then 0.14 degrees, or three pixels of a 2560-wide view. A trig pair per
/// lens per pixel is what the exact form costs and it is cheaper than being
/// wrong by more than the thing being corrected is worth near the seam.
///
/// WGSL twin: `turned`.
fn turned(v: [f32; 3], turn: [f32; 3]) -> [f32; 3] {
    let angle = (turn[0] * turn[0] + turn[1] * turn[1] + turn[2] * turn[2]).sqrt();
    // A still camera, and every file with no IMU record: the axis is not
    // defined and there is nothing to turn by anyway.
    if angle < 1e-9 {
        return v;
    }
    let axis = turn.map(|component| component / angle);
    let (sin, cos) = angle.sin_cos();
    let across = cross(axis, v);
    let along = dot(axis, v);
    std::array::from_fn(|i| v[i] * cos + across[i] * sin + axis[i] * along * (1.0 - cos))
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|axis| a[axis] * b[axis]).sum()
}

/// The **front** lens's share of a ray, from how far apart the two lenses'
/// axis dot products are: 1 well inside its own hemisphere, 1/2 on the seam,
/// 0 once the ray is half a crossover past it (issue #48).
///
/// `apart` is the difference of the two unnormalized dot products and `reach`
/// is the ray's length, so the division that normalizes them happens once
/// here rather than twice at the call site. `band` is how wide this camera
/// hands the picture over ([`Reframe::handover_width`]), which is what
/// [`CROSSOVER_DEG`] asks for unless the two lenses' overlap cannot pay for
/// it.
///
/// How far past the seam a ray looks is half the difference of the two
/// lenses' angles off their own axes. Written that way rather than as "ninety
/// degrees off the front lens" so that it still names the crossover when the
/// two axes are not exactly opposed, which is not hypothetical: the per-file
/// fit in [`super::seam`] moves one axis by a couple of degrees, and a band
/// centred on the front lens alone would then sit off the overlap.
///
/// The angles arrive as their cosines and stay there. Near the seam
/// `cos(theta) = -sin(theta - 90 deg)`, so the difference of the two cosines
/// **is** the difference of the two angles in radians, and what the third
/// term of the sine costs at the edge of a 2 degree band is 0.00005 degrees
/// of band width (`the_crossover_is_the_width_it_says_it_is` measures the
/// band itself at 2.00). Past the band the clamp has closed and how it got
/// there does not matter. No trig anywhere, and one multiply fewer than the
/// `cos^2(theta / 2)` preference this replaces.
///
/// `shift` is the [`SeamAnchor`]'s, in radians across the seam
/// ([`Reframe::handover_shift`]): a whole term of its own beside the
/// geometry's, divided by the same band, so a shift of half the band puts the
/// 50/50 line at the edge of the fade and a shift of zero leaves it exactly
/// where the two axis cosines put it.
///
/// **The support moves with the line, and it is not narrowed to compensate.**
/// This closes at `-band / 2 - shift` off the seam and opens at
/// `band / 2 - shift`, so the far end reaches `band / 2 + |shift|` and, at the
/// rail, a **whole band**. That is past the shared picture on every camera in
/// the corpus and it is deliberate: what refuses the sample out there is the
/// outer lens's own coverage depth inside [`claim`], not this width.
/// [`Reframe::afforded`] carries the argument and
/// `tests::the_anchored_handover_leaves_no_hole_and_no_cliff`
/// carries the measurement.
///
/// **The ramp is the whole curve, and there is no exponent on it.** Between
/// 2026-08-08 and the flat seam this share was re-spent on `s^n / (s^n +
/// (1-s)^n)` at `n = 1.5`, to hand the picture over inside a narrower part of
/// the same support. That curve existed to make a *bend* fold-free over less
/// picture; with no bend to shear there is nothing for it to buy, and the
/// owner judged the linear one on his own footage. `n = 1` is the identity, so
/// deleting it is deleting a function that had become one - see
/// docs/research/studio-parity.md.
///
/// WGSL twin: `crossover`.
fn crossover(apart: f32, reach: f32, band: f32, shift: f32) -> f32 {
    (0.5 + apart / (2.0 * reach * band) + shift / band).clamp(0.0, 1.0)
}

impl LensBlock {
    /// A lens with no picture in it: `xi` of 1 keeps the denominator
    /// positive and a zero image radius puts every ray outside. What an
    /// unfilled slot holds, so that a stray index costs a grey pixel rather
    /// than a garbage sample, and since issue #10 not even that: an
    /// [`Self::axis_min`] of 2 is a cap no ray can be inside, so the pass
    /// skips the slot instead of projecting into it.
    const EMPTY: Self = Self {
        view_to_lens: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
        xi: 1.0,
        fx: 1.0,
        fy: 1.0,
        cx: 0.0,
        cy: 0.0,
        k1: 0.0,
        k2: 0.0,
        k3: 0.0,
        p1: 0.0,
        p2: 0.0,
        image_radius: 0.0,
        axis_min: 2.0,
        turn: [0.0; 3],
        model: MEI,
    };

    fn new(lens: &Lens, index: usize, frame: Size, camera: Camera, held: Held) -> Self {
        let Intrinsics { xi, fx, fy, cx, cy } = lens.intrinsics;
        let distortion = lens.distortion;
        let mut block = Self {
            view_to_lens: view_to_lens(lens, index, camera, held).columns(),
            xi: xi as f32,
            fx: fx as f32,
            fy: fy as f32,
            cx: cx as f32,
            cy: cy as f32,
            k1: distortion.k1 as f32,
            k2: distortion.k2 as f32,
            k3: distortion.k3 as f32,
            p1: distortion.p1 as f32,
            p2: distortion.p2 as f32,
            image_radius: image_radius(&lens.intrinsics, frame) as f32,
            // The body's turn across the readout, carried into this lens's
            // own frame, which is where the ray it corrects is expressed.
            // Conjugating the rotation by the mounting is what rotating its
            // axis by the mounting does, and it is why the two lenses'
            // corrections run opposite ways in the world.
            turn: held.rolling.map_or([0.0; 3], |rolling| {
                lens_from_body(lens, index).mul_vec(rolling.turn.map(|axis| axis as f32))
            }),
            // Solved for below: the cap is a property of the model this block
            // has just been filled with, and the readout it widens by is the
            // `turn` above.
            axis_min: 2.0,
            model: match lens.model {
                Model::Mei => MEI,
                Model::Equidistant => EQUIDISTANT,
            },
        };
        // Half of it, because `readout_share` runs -1/2 to +1/2 and the model
        // is handed the ray turned by that share of the whole readout.
        block.axis_min = coverage_floor(&block, 0.5 * norm3(block.turn));
        block
    }

    fn lens_ray(&self, ray: [f32; 3]) -> [f32; 3] {
        let column = |c: usize| self.view_to_lens[c];
        std::array::from_fn(|row| {
            ray[0] * column(0)[row] + ray[1] * column(1)[row] + ray[2] * column(2)[row]
        })
    }
}

/// The ray a point of the output looks along for one camera, in view space:
/// x right, y down, z forward. `uv` runs 0 to 1 across the output, y down,
/// and `aspect` is the output's width over its height.
///
/// The drag in `super::camera` reads its rays from here rather than assuming
/// a projection, which is the whole of what issue #47 asked of it: the anchor
/// solve inverts whichever map the view is currently in, because it is handed
/// the rays that map makes.
pub(crate) fn view_ray(uv: [f32; 2], camera: Camera, aspect: f32) -> Option<[f32; 3]> {
    Screen::new(camera, aspect).ray(uv)
}

/// Where a view-space ray points in the world: the camera's own rotation,
/// with none of the lens's mounting.
///
/// The drag solve in `super::camera` inverts this, so it reads the
/// composition from here rather than assuming one.
pub(crate) fn world_ray(camera: Camera, ray: [f32; 3]) -> [f32; 3] {
    camera_rotation(camera).mul_vec(ray)
}

/// The rotation that takes a view-space ray to lens `index`'s frame.
///
/// Three steps, right to left: where the view is pointing in the world, where
/// the camera body was when the frame was taken, and where this lens sits on
/// that body. Every one of them is right-handed in the frame the projection
/// uses: x right, y down, z along the axis being pointed. Positive camera yaw
/// turns right, positive camera pitch looks up.
///
/// The middle step is horizon lock (issue #8), and it is the whole of it: the
/// camera's own yaw and pitch are read in the **stabilized world** frame
/// rather than the body's, so a body that rolls under a level view leaves the
/// view level. It is also why the drag needed no change at all. `Camera::look`
/// answers in whatever frame `camera_rotation` lands in, the drag anchors a
/// direction in that frame and solves for the view that puts it back there,
/// and with lock on that frame is the world: the anchor stays on the world
/// and the picture turns under it.
fn view_to_lens(lens: &Lens, index: usize, camera: Camera, held: Held) -> Mat3 {
    lens_from_body(lens, index).mul(body_from_view(camera, held))
}

/// The same composition with the lens's own mounting left off: a view-space
/// ray in the camera **body**'s frame (issue #103).
///
/// The seam circle and the baseline are fixed to the body, so this is the
/// frame the band is measured and looked up in. Taken from [`view_to_lens`]
/// rather than written out beside it, so the two cannot drift: whatever the
/// pass thinks the view is pointing at, the band thinks the same.
fn body_from_view(camera: Camera, held: Held) -> Mat3 {
    Mat3::from(held.body_from_world.matrix().rows()).mul(camera_rotation(camera))
}

/// Yaw about the world vertical, then pitch about the view's own horizontal.
/// Never roll: the horizon stays level, which is the whole reason a drag near
/// the pole has to give something up (issue #29).
fn camera_rotation(camera: Camera) -> Mat3 {
    Mat3::rot_y(camera.yaw as f64).mul(Mat3::rot_x(camera.pitch as f64))
}

/// The lens's own mounting, over the nominal arrangement it is mounted in
/// ([`opposed`]).
///
/// The three angles and the quarter-turn datum they are measured against live
/// in `kjerag_meta::Pose::lens_from_body`, because the IMU needs the same
/// rotation to get out of the front lens's frame and into the body's, and one
/// settled convention wants one definition.
///
/// A file that records the **whole** rotation rather than a residual against
/// the arrangement takes it verbatim and gets no [`opposed`] composed onto it:
/// a DJI `.OSV`'s two quaternions already point opposite ways, and turning one
/// of them a further half turn would point both lenses forward
/// (`kjerag_meta::Lens::mounting`).
fn lens_from_body(lens: &Lens, index: usize) -> Mat3 {
    match lens.mounting {
        Some(whole) => Mat3::from(whole.rows()),
        None => Mat3::from(lens.pose.lens_from_body().rows()).mul(opposed(index)),
    }
}

/// The nominal pose lens `index` is mounted in, which its extrinsics are a
/// residual against.
///
/// The back-to-back flip is **not** in the file: lens 1's recorded yaw is
/// 0.039 degrees, not 180, and applying the block as an absolute pose points
/// both lenses the same way (docs/research/insv-format.md 4.3). A half turn
/// about the body's vertical is what puts it back: it takes body-forward to
/// lens-backward and leaves body-down as lens-down, so the rear picture comes
/// out the same way up as the front one. The other half turn that points the
/// same way, about x, differs from it by exactly 180 degrees of roll, which is
/// a rear sensor mounted upside down; `roll` is what records that, and `roll`
/// is already applied.
///
/// It multiplies on the right, so the block's own angles are a residual in the
/// lens's own frame rather than in the body's. That is a choice and not a
/// rearrangement: the two orders differ by twice lens 1's roll residual, 1.85
/// degrees on the X4 Air fixture. Measured against pixels 2026-07-31, the way
/// the roll datum was: both orders and the half turn about x rendered across
/// the seam on real footage, and the far-field content correlated between the
/// two lenses' pictures of it. This order leaves 0.4 degrees of the seam
/// unaligned along its own circle, the other 1.5, and the turn about x
/// correlates with nothing. Method and numbers: docs/research/insv-format.md
/// 4.9.
fn opposed(index: usize) -> Mat3 {
    match index {
        0 => Mat3::IDENTITY,
        _ => Mat3::rot_y(std::f64::consts::PI),
    }
}

/// The largest circle centred on the principal point that fits in the
/// delivered frame.
///
/// The file records no image-circle radius and the model does not bound
/// itself: past the lens's real coverage the radial polynomial keeps
/// returning finite pixel coordinates, so something has to say where the
/// picture stops. On the X4 Air fixture this radius is 1913 px, which the
/// model reaches at about 97.4 degrees off axis, so two lenses overlap by
/// about 14 degrees around the seam. That circle is the validity boundary
/// [`claim`] measures its coverage depth from, so it sets the blend band as
/// well as the picture's edge.
fn image_radius(intrinsics: &Intrinsics, frame: Size) -> f64 {
    let (width, height) = (f64::from(frame.width), f64::from(frame.height));
    intrinsics
        .cx
        .min(intrinsics.cy)
        .min(width - intrinsics.cx)
        .min(height - intrinsics.cy)
        .max(0.0)
}

fn norm(v: [f32; 2]) -> f32 {
    v[0].hypot(v[1])
}

/// WGSL twin: `length` on a `vec3<f32>`, which is what `within` divides the
/// ray's axis by. Written the same way round as [`normalize`] rather than as
/// a `hypot` chain, so the two answer the same number.
fn norm3(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

pub(crate) fn normalize(v: [f32; 3]) -> [f32; 3] {
    v.map(|component| component / norm3(v))
}

/// A 3x3 rotation, row major: `m[row][column]`, and `v_out = M * v_in`.
#[derive(Clone, Copy, Debug)]
struct Mat3([[f64; 3]; 3]);

impl From<[[f64; 3]; 3]> for Mat3 {
    fn from(rows: [[f64; 3]; 3]) -> Self {
        Self(rows)
    }
}

impl Mat3 {
    const IDENTITY: Self = Self([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    fn rot_x(angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        Self([[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]])
    }

    fn rot_y(angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        Self([[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]])
    }

    fn mul_vec(self, v: [f32; 3]) -> [f32; 3] {
        let v = v.map(f64::from);
        std::array::from_fn(|row| (0..3).map(|k| self.0[row][k] * v[k]).sum::<f64>() as f32)
    }

    fn mul(self, rhs: Self) -> Self {
        let mut out = [[0.0; 3]; 3];
        for (r, row) in out.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| self.0[r][k] * rhs.0[k][c]).sum();
            }
        }
        Self(out)
    }

    /// WGSL stores a `mat3x3<f32>` as three columns, each padded to a
    /// `vec4`.
    fn columns(self) -> [[f32; 4]; 3] {
        std::array::from_fn(|c| {
            [
                self.0[0][c] as f32,
                self.0[1][c] as f32,
                self.0[2][c] as f32,
                0.0,
            ]
        })
    }
}

/// The half of the shader that mirrors this file, with the constants this
/// file owns written into it. `crates/render/src/scene.rs` concatenates the
/// result with the pass that samples NV12 and writes the target.
pub(crate) fn wgsl() -> String {
    // `{:?}` rather than `{}`: Rust's Display drops the decimal point on a
    // whole number, and `vec3<f32>(1)` is a type error in WGSL.
    // The crossover is NOT here. It was a constant while it was the picture's;
    // it is the camera's now, and the shader is compiled once before any file
    // is open, so it travels in the block instead (`Reframe::crossover`).
    // The table's length is in `vec4`s, which is how the block carries it and
    // what an array's size in WGSL has to be written in. `AZIMUTHS` itself is
    // the band's own name and is declared by the band's own half, which only
    // one of the two pipelines compiled from this file is given.
    // The blend curve's exponent used to be here, unlike the crossover,
    // because it was a property of the map rather than of the file. There is
    // no exponent any more: the crossfade is the linear ramp, which is what
    // `crossover` computes and nothing re-spends.
    let lanes = super::band::AZIMUTHS / 4;
    format!(
        "const MAX_LENSES = {MAX_LENSES}u;\nconst READOUT_STEPS = {READOUT_STEPS}u;\n\
         const TABLE_LANES = {lanes}u;\nconst EQUIDISTANT = {EQUIDISTANT:?};\n{WGSL}"
    )
}

const WGSL: &str = r#"
struct LensBlock {
  view_to_lens: mat3x3<f32>,
  xi: f32,
  fx: f32,
  fy: f32,
  cx: f32,
  cy: f32,
  k1: f32,
  k2: f32,
  k3: f32,
  p1: f32,
  p2: f32,
  image_radius: f32,
  // The cosine of the widest angle off this lens's axis that can still be in
  // its picture. Rust twin: `LensBlock::axis_min`.
  axis_min: f32,
  // The body's turn across one readout, in this lens's frame. Rust twin:
  // `LensBlock::turn`.
  turn_x: f32,
  turn_y: f32,
  turn_z: f32,
  // Which model `lens_pixel` runs for this lens: MEI or EQUIDISTANT. Rust
  // twin: `LensBlock::model`.
  model: f32,
};

// How a point of the frame becomes a ray. Rust twin: `Screen`.
struct Screen {
  half_extent: f32,
  shrink: f32,
  ball_radius: f32,
  aspect: f32,
};

struct Reframe {
  lenses: array<LensBlock, MAX_LENSES>,
  // A view-space ray in the camera body's own frame, which is where the seam
  // circle and the baseline stand still. Rust twin: `Reframe::view_to_body`.
  view_to_body: mat3x3<f32>,
  // Where lens 1 sits relative to lens 0, in metres, in the body's frame.
  // Zero for a file with one lens stream, which is a band that measures
  // nothing and bends nothing. Rust twin: `Reframe::baseline`.
  baseline_x: f32,
  baseline_y: f32,
  baseline_z: f32,
  // A `vec3` in a uniform block is padded to sixteen bytes. Rust twin:
  // `Reframe::_baseline_pad`.
  baseline_pad: f32,
  screen: Screen,
  frame_width: f32,
  frame_height: f32,
  lens_count: f32,
  linearize: f32,
  // Which way across the delivered frame the sensor reads, and zero on both
  // components where there is no readout to correct.
  row_axis_x: f32,
  row_axis_y: f32,
  // How far the magnification upgrade may engage on each plane. Rust twin:
  // `Reframe::sharpen`.
  sharpen_luma: f32,
  sharpen_chroma: f32,
  // How wide this camera hands the picture over, in radians. Rust twin:
  // `Reframe::crossover`. Read by `handover`.
  crossover: f32,
  // How far across the seam the drawn 50/50 handover line is moved from where
  // the geometry puts it, in radians. One number, because the law that
  // produces it has one and there is no event in it. Zero is the geometric
  // handover. Rust twin: `Reframe::handover_shift`. Read by `handover`.
  handover_shift: f32,
  // What puts the table below on its own 16-byte boundary. Rust twin:
  // `Reframe::_pad`, which is what makes the two layouts agree.
  pad1: f32,
  pad2: f32,
  // What the along-seam axis still disagrees by after a pose, direction by
  // direction, in radians, four to a lane. Rust twin: `Reframe::table`.
  //
  // NOTHING READS IT. It is declared because the block is one layout and both
  // sides have to describe the same bytes; its reader was `table_at`, inside
  // the bend, and the bend is gone.
  table: array<vec4<f32>, TABLE_LANES>,
};

@group(0) @binding(0) var<uniform> reframe: Reframe;

struct Landing {
  pixel: vec2<f32>,
  inside: bool,
  axis: f32,
  depth: f32,
};

struct Blend {
  landings: array<Landing, MAX_LENSES>,
  weights: array<f32, MAX_LENSES>,
};

// x right, y down, z forward, matching the lens frame the model projects in.
// Rust twin: `Screen::ray`, whose `Option` this `w` is: 1 where the frame is
// looking at the sphere and 0 in the room around the ball, which no lens can
// have and the pass leaves transparent.
fn view_ray(uv: vec2<f32>) -> vec4<f32> {
  let screen = reframe.screen;
  let extent = (uv * 2.0 - vec2<f32>(1.0)) * screen.half_extent;
  let plane = vec2<f32>(extent.x, extent.y / screen.aspect);
  // The flat window, which is every view the player had before issue #47:
  // the same two multiplies it always was, and neither the length below nor
  // the trig under that. A flat frame is all sphere, so the ball test it
  // skips could not have fired.
  if screen.shrink == 1.0 {
    return vec4<f32>(plane, 1.0, 1.0);
  }
  let radius = length(plane);
  if radius > screen.ball_radius {
    return vec4<f32>(0.0, 0.0, 1.0, 0.0);
  }
  let theta = atan(screen.shrink * radius) / screen.shrink;
  let out = select(0.0, sin(theta) / radius, radius > 0.0);
  return vec4<f32>(plane * out, cos(theta), 1.0);
}

// Every lens's claim on the ray, normalized. Rust twin: `Reframe::blend`.
//
// THE RAY IS THE RAY. It used to be bent first: `band_bend` answered what the
// two lenses disagreed by at this direction and how wide the handover had to
// open to carry that without folding, and each lens was sampled at a ray moved
// by the other lens's weight. None of that is here. The seam is flat, the
// picture is fused and not morphed, and what the band measures reaches the
// instruments and the future belt but never this function. See
// `Reframe::blend` for the ruling and what it cost.
//
// The loop runs MAX_LENSES times whatever the file holds, and the lens count
// zeroes the claim of a slot that has no stream rather than shortening the
// loop. A loop this compiler cannot unroll indexes `out` dynamically, which
// puts it in scratch memory and costs more than the blend does; the numbers
// are on the Rust twin. The array writes stay unconditional for the same
// reason; what `within` skips is the model, not the bookkeeping.
fn blend(ray: vec3<f32>) -> Blend {
  var out: Blend;
  var total = 0.0;
  let reach = length(ray);
  // Both axis cosines before the loop: the crossover needs them together,
  // the cap test needs them one at a time, and reading them back out of
  // `out` after the loop instead costs 5.5 ms a redraw against 3.6. Rust
  // twin: `Reframe::blend`.
  let axis0 = axis_of(reframe.lenses[0], ray);
  let axis1 = axis_of(reframe.lenses[1], ray);
  let front = handover(axis0, axis1, reach, reframe.crossover);
  for (var index = 0u; index < MAX_LENSES; index += 1u) {
    let lens = reframe.lenses[index];
    // Zero, which is `Landing::MISSED`: a lens the ray cannot reach is never
    // projected and its landing is never read.
    var landing: Landing;
    var claimed = 0.0;
    if within(lens, select(axis1, axis0, index == 0u), reach) {
      let share = select(1.0 - front, front, index == 0u);
      landing = project(lens, ray);
      claimed = select(0.0, claim(landing, share), f32(index) < reframe.lens_count);
    }
    out.landings[index] = landing;
    out.weights[index] = claimed;
    total += claimed;
  }
  if total > 0.0 {
    for (var index = 0u; index < MAX_LENSES; index += 1u) {
      out.weights[index] = share(out.weights[index], total);
    }
  }
  return out;
}

// Whether this lens can have any of this ray, before the model runs. Rust
// twin: `Reframe::covers`.
//
// The mounting is a rotation, so the cosine `mei` would read off the
// normalized ray is one row of it against the ray over the ray's own length.
// Multiplying the cap by the length rather than dividing keeps it to a
// compare. `reach` is the same for every lens.
fn within(lens: LensBlock, axis: f32, reach: f32) -> bool {
  return axis >= lens.axis_min * reach;
}

// How far a ray is off one lens's axis, as an unnormalized cosine: one row of
// the mounting against the ray. Rust twin: `Reframe::axis_of`.
fn axis_of(lens: LensBlock, ray: vec3<f32>) -> f32 {
  return dot(vec3<f32>(
    lens.view_to_lens[0].z,
    lens.view_to_lens[1].z,
    lens.view_to_lens[2].z,
  ), ray);
}

// One claim's share of all of them, the lone claimant's written rather than
// divided out. Rust twin: `share`.
fn share(claim: f32, total: f32) -> f32 {
  if claim == total {
    return 1.0;
  }
  return claim / total;
}

// This lens's share of the crossover times its coverage depth. Rust twin:
// `claim`.
fn claim(landing: Landing, share: f32) -> f32 {
  if !landing.inside {
    return 0.0;
  }
  return share * landing.depth;
}

// The front lens's share of the ray, and 1 for a one-stream file, which has
// no seam to hand over at.
//
// The drawn 50/50 line is moved across the seam by `reframe.handover_shift`,
// which is the seam anchor's one number and a whole term of its own inside
// `crossover`. Rust twin: `Reframe::handover`.
fn handover(axis0: f32, axis1: f32, reach: f32, band: f32) -> f32 {
  if reframe.lens_count <= 1.0 {
    return 1.0;
  }
  return crossover(axis0 - axis1, reach, band, reframe.handover_shift);
}

// The front lens's share, from how far apart the two dot products are, across
// the band the camera hands over on, and moved across the seam by `shift`
// radians. A linear ramp and nothing on top of it: the exponent that used to
// re-spend this share inside a narrower part of the same support existed to
// keep a bend fold-free, and there is no bend. Rust twin: `crossover`.
fn crossover(apart: f32, reach: f32, band: f32, shift: f32) -> f32 {
  return clamp(0.5 + apart / (2.0 * reach * band) + shift / band, 0.0, 1.0);
}

// The forward map, with the readout taken out of it. Rust twin:
// `Reframe::project`.
//
// The row a ray lands on decides the instant its orientation is read at, and
// that instant moves the row, so the landing is solved for rather than
// computed: `READOUT_STEPS` rounds from the frame's own instant. The loop
// runs a fixed number of times and the whole of it is behind one uniform
// test, so a file with no IMU record costs what it cost before issue #9.
fn project(lens: LensBlock, ray: vec3<f32>) -> Landing {
  let aimed = lens.view_to_lens * ray;
  var landing = lens_pixel(lens, normalize(aimed));
  if reframe.row_axis_x != 0.0 || reframe.row_axis_y != 0.0 {
    let turn = vec3<f32>(lens.turn_x, lens.turn_y, lens.turn_z);
    for (var step = 0u; step < READOUT_STEPS; step += 1u) {
      landing = lens_pixel(lens, normalize(turned(aimed, turn * readout_share(landing.pixel))));
    }
  }
  return landing;
}

// A unit ray in one lens's frame to a pixel, through whichever model that
// lens's calibration is written in. Rust twin: `lens_pixel`.
fn lens_pixel(lens: LensBlock, p: vec3<f32>) -> Landing {
  if lens.model == EQUIDISTANT {
    // The equidistant map is monotone in theta over the whole sphere, so
    // unlike Mei's it has no turning point to stay behind.
    return landed(lens, equidistant(lens, p), p.z, true);
  }
  return mei(lens, p);
}

// The equidistant fisheye: r = fx * theta, and no distortion polynomial. Rust
// twin: `equidistant`.
fn equidistant(lens: LensBlock, p: vec3<f32>) -> vec2<f32> {
  let reach = length(p.xy);
  let theta = atan2(reach, p.z);
  // Straight down the axis, where the azimuth is not defined and the radius
  // is zero anyway.
  let scale = select(0.0, theta / reach, reach > 0.0);
  return vec2<f32>(lens.fx * p.x, lens.fy * p.y) * scale;
}

// One model's pixel offset, as the Landing the rest of the pass reads. Rust
// twin: `landed`.
fn landed(lens: LensBlock, offset: vec2<f32>, axis: f32, injective: bool) -> Landing {
  let depth = lens.image_radius - length(offset);
  var landing: Landing;
  landing.pixel = offset + vec2<f32>(lens.cx, lens.cy);
  landing.inside = injective && depth > 0.0;
  landing.axis = axis;
  landing.depth = depth;
  return landing;
}

// Where in the readout a landing's row is exposed, -1/2 to +1/2. Rust twin:
// `Reframe::readout_share`.
fn readout_share(pixel: vec2<f32>) -> f32 {
  let across = pixel / vec2<f32>(reframe.frame_width, reframe.frame_height) - vec2<f32>(0.5);
  return clamp(dot(across, vec2<f32>(reframe.row_axis_x, reframe.row_axis_y)), -0.5, 0.5);
}

// A ray turned by a rotation vector, exactly (Rodrigues). Rust twin: `turned`.
fn turned(v: vec3<f32>, turn: vec3<f32>) -> vec3<f32> {
  let angle = length(turn);
  if angle < 1e-9 {
    return v;
  }
  let axis = turn / angle;
  return v * cos(angle) + cross(axis, v) * sin(angle)
    + axis * dot(axis, v) * (1.0 - cos(angle));
}

// The Mei/UCM model. Rust twin: `Reframe::mei`.
fn mei(lens: LensBlock, p: vec3<f32>) -> Landing {
  let denom = p.z + lens.xi;
  let n = p.xy / denom;

  let r2 = dot(n, n);
  let radial = 1.0 + r2 * (lens.k1 + r2 * (lens.k2 + r2 * lens.k3));
  let tangential = vec2<f32>(
    2.0 * lens.p1 * n.x * n.y + lens.p2 * (r2 + 2.0 * n.x * n.x),
    2.0 * lens.p2 * n.x * n.y + lens.p1 * (r2 + 2.0 * n.y * n.y),
  );
  let d = n * radial + tangential;

  let offset = vec2<f32>(lens.fx * d.x, lens.fy * d.y);
  // Past `cos(theta) = -1/xi` the map folds and lands rays from behind this
  // lens back inside its image circle. Rust twin: `injective`.
  let injective = p.z * lens.xi > -1.0;
  return landed(lens, offset, p.z, denom > 0.0 && injective);
}

// Pixel centres sit at integer coordinates in the camera model and at
// (i + 0.5) / size in a texture.
fn frame_uv(pixel: vec2<f32>) -> vec2<f32> {
  return (pixel + vec2<f32>(0.5)) / vec2<f32>(reframe.frame_width, reframe.frame_height);
}

// How many delivered-frame texels one output pixel covers where it landed.
// Rust twin: `Reframe::texels_per_pixel`.
//
// The finite difference is the hardware's own, one quad at a time, which is
// why the entry point calls this and `blend` does not: a derivative needs
// uniform control flow and `blend` is nothing but branches. Reading the step
// off the quad rather than off a resolution in the uniform block is also
// what makes a still right without being told (issue #15): the capture draws
// this same pipeline into a target of its own size, and a quad of that
// target steps a smaller share of the picture all by itself.
//
// The longer of the two steps, so a landing stretched one way and squeezed
// the other counts as not magnified. That is the safe direction twice over.
// It leaves the axis that still has texels to spend sampling the way it
// always did, and where a quad straddles the edge of a lens's coverage one
// of its lanes has no landing at all and the step reads as most of the
// picture: a huge ratio, which disengages. That lane is within an output
// pixel of the edge of that lens's own picture, where its coverage depth and
// with it its weight have gone to zero and the other lens is carrying the
// ray.
fn texel_ratio(pixel: vec2<f32>) -> f32 {
  return max(length(dpdx(pixel)), length(dpdy(pixel)));
}
"#;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use kjerag_meta::{Distortion, Sweep};

    use crate::sampling;

    pub(crate) const FRAME: Size = Size {
        width: 3840,
        height: 3840,
    };

    /// The X4 Air fixture in delivered-frame pixels: what `kjerag-meta`
    /// produces from `docs/research/x4air-calibration.json`, and what its own
    /// tests assert. Copied rather than parsed because the path from the
    /// fixture to a `CalibrationSet` runs through a private constructor in a
    /// crate this one only reads types from.
    pub(crate) fn fixture_lenses() -> Vec<Lens> {
        vec![
            Lens {
                intrinsics: Intrinsics {
                    xi: 2.31494,
                    fx: 3665.9397,
                    fy: 3667.4194,
                    cx: 1918.94,
                    cy: 1927.21,
                },
                distortion: Distortion {
                    k1: 0.95820886,
                    k2: -1.80141151,
                    k3: 3.57555127,
                    p1: -0.0007338,
                    p2: -0.00115458,
                },
                model: Model::Mei,
                mounting: None,
                pose: kjerag_meta::Pose {
                    yaw_deg: -0.103,
                    pitch_deg: -0.07,
                    roll_deg: 90.534,
                    translation_m: [0.0, 0.0, 0.0],
                },
                lens_type: 131,
            },
            Lens {
                intrinsics: Intrinsics {
                    xi: 2.31494,
                    fx: 3671.9126,
                    fy: 3671.0823,
                    cx: 1935.35,
                    cy: 1935.09,
                },
                distortion: Distortion {
                    k1: 0.97158086,
                    k2: -2.08655882,
                    k3: 4.30578518,
                    p1: -0.0019249,
                    p2: 0.00054564,
                },
                model: Model::Mei,
                mounting: None,
                pose: kjerag_meta::Pose {
                    yaw_deg: 0.039,
                    pitch_deg: -0.193,
                    roll_deg: 89.076,
                    translation_m: [-0.002063, 0.000334, -0.033284],
                },
                lens_type: 131,
            },
        ]
    }

    fn fixture(camera: Camera) -> Reframe {
        held(camera, Held::default())
    }

    /// The same optics with the image circle cropped, which is the only knob
    /// there is for asking a narrower camera a question here.
    ///
    /// A lens's picture stops where its landing leaves the largest circle that
    /// fits in the delivered frame ([`image_radius`]), so a smaller frame round
    /// the same principal point is a lens that sees less, and two of them
    /// overlap by less. That is exactly what [`Reframe::overlap`] reads and
    /// exactly what [`Reframe::afforded`] clamps against.
    ///
    /// **Synthesized rather than checked in, and that is a rule and not a
    /// shortcut.** The ONE X2's own calibration lives in the owner's footage
    /// and a trailer dump carries his camera serial and his GPS track, which
    /// AGENTS.md forbids committing. The X4 Air fixture is the one calibration
    /// this repository has, so a narrow camera is made out of it by taking away
    /// picture, and the overlaps below are asserted rather than assumed so that
    /// the fixture cannot quietly stop being the camera class it says it is.
    fn cropped(frame: u32) -> Reframe {
        Reframe::new(
            &fixture_lenses(),
            Size {
                width: frame,
                height: frame,
            },
            Camera::default(),
            Held::default(),
            1.0,
            false,
            Sampling::default(),
        )
    }

    /// A camera that overlaps by 9.18 degrees, which is the ONE X2's 9.19 to a
    /// hundredth of a degree: the narrowest camera in the corpus, and the one
    /// the flat seam moved from 3.94 degrees of handover to the whole 8.00.
    ///
    /// **It reproduces that camera's OVERLAP and nothing else about its
    /// optics**, and every "X2-class" figure this file and
    /// docs/research/studio-parity.md 5.3 report means that. The crop is
    /// concentric with each lens's unchanged principal point, so the boundary
    /// it makes is the X4 Air's own shape scaled down; a real narrow camera's
    /// is raggeder, and the review that raised the width finding puts the ONE
    /// X2's lens 1 worst azimuth at 3.39 degrees against this fixture's 0.66.
    /// Neither property asserted here is read off that shape - a hole needs
    /// `|shift| > band / 2 + overlap / 2`, whose only camera term is the
    /// overlap, and a cliff is carried by [`claim`]'s per-lens taper, which
    /// runs out on whatever rim the lens has - so the conclusions hold and the
    /// SIZE of an overshoot quoted for "an X2-class camera" is this fixture's
    /// and not that camera's.
    const X2_CLASS: u32 = 3803;

    /// A camera that overlaps by 7.43 degrees, which is narrower than the 8 the
    /// picture asks for. Nothing in the corpus is this tight; it exists so that
    /// [`Reframe::afforded`]'s clamp has something to bind on.
    const UNDER_THE_ASK: u32 = 3790;

    /// The same fixture with the camera body somewhere other than level,
    /// which is what horizon lock has to take back out.
    fn held(camera: Camera, held: Held) -> Reframe {
        Reframe::new(
            &fixture_lenses(),
            FRAME,
            camera,
            held,
            1.0,
            false,
            Sampling::default(),
        )
    }

    /// The camera as it was before issue #27: one stream, one lens, one
    /// hemisphere. Legacy files that write a lens per file still render this
    /// way.
    fn one_lens(camera: Camera) -> Reframe {
        Reframe::new(
            &fixture_lenses()[..1],
            FRAME,
            camera,
            Held::default(),
            1.0,
            false,
            Sampling::default(),
        )
    }

    #[track_caller]
    fn near(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} is not within {tolerance} of {expected}"
        );
    }

    /// The two directions of the body/view boundary are one rotation, so a
    /// ray that goes out one side comes back unchanged through the other.
    /// The fixture is deliberately turned: at yaw 0 a transpose and a wrong
    /// copy of the matrix are the same numbers.
    #[test]
    fn a_body_ray_and_a_view_ray_are_each_other_inverted() {
        let reframe = fixture(Camera {
            yaw: 74.0_f32.to_radians(),
            pitch: -31.0_f32.to_radians(),
            fov: 55.0_f32.to_radians(),
        });
        for ray in [
            direction(90.0, 12.0),
            direction(70.0, 200.0),
            [0.0, 0.0, 1.0],
        ] {
            let round_trip = reframe.body_ray(reframe.view_ray_from_body(ray));
            for axis in 0..3 {
                near(round_trip[axis], ray[axis], 1e-6);
            }
        }
    }

    /// A direction in the body frame, `theta` degrees off the front lens's
    /// axis and turned `phi` degrees about it: `theta` of 90 is the seam
    /// great circle and 180 is straight out the back.
    fn direction(theta: f32, phi: f32) -> [f32; 3] {
        let (sin_theta, cos_theta) = theta.to_radians().sin_cos();
        let (sin_phi, cos_phi) = phi.to_radians().sin_cos();
        [sin_theta * cos_phi, sin_theta * sin_phi, cos_theta]
    }

    /// A reading on the epipolar axis alone, which is what every question
    /// about the crossover's width is about: the along-seam axis does not
    /// open it (`Reframe::bent`).
    fn reading(epi: f32) -> crate::band::Reading {
        crate::band::Reading { epi, along: 0.0 }
    }

    /// The lens carrying most of an output pixel, and where it lands, which
    /// is the question the hard pick answered before issue #7. `None` where
    /// no lens has the ray, which is what the shader paints grey.
    /// The ray at a point of the output, which every view in these tests is
    /// flat enough to have one of.
    fn ray(reframe: &Reframe, uv: [f32; 2]) -> [f32; 3] {
        reframe.view_ray(uv).expect("a flat view is all sphere")
    }

    fn shown(reframe: &Reframe, ray: [f32; 3]) -> Option<(usize, Landing)> {
        let blend = reframe.blend(ray);
        let lens =
            (0..MAX_LENSES).max_by(|a, b| blend.weights[*a].total_cmp(&blend.weights[*b]))?;
        blend.is_covered().then(|| (lens, blend.landings[lens]))
    }

    /// The sanity check the model has to pass before any pixel is believed:
    /// the middle of the view looks along the front lens's axis, and the lens
    /// axis is the principal point.
    ///
    /// Not exact, because the lens is not mounted exactly on the body axis:
    /// 0.103 degrees of yaw and 0.07 degrees of pitch tilt it by 0.125
    /// degrees, and near the axis the model's effective focal length is
    /// `fx / (1 + xi)` = 1106 px/rad, so 2.4 px.
    #[test]
    fn the_view_axis_lands_on_the_principal_point() {
        let reframe = fixture(Camera::default());
        let (lens, landing) =
            shown(&reframe, ray(&reframe, [0.5, 0.5])).expect("no lens has the view axis");

        assert_eq!(lens, 0);
        near(landing.pixel[0], 1918.94, 3.0);
        near(landing.pixel[1], 1927.21, 3.0);
    }

    /// And the other half of issue #27: the ray straight out the back is lens
    /// 1's own axis, so it is lens 1 that shows it and its principal point
    /// that it lands on. Without the nominal half turn in `opposed` this ray
    /// projects nowhere near lens 1's centre, and with the wrong half turn it
    /// lands there with the picture upside down.
    ///
    /// The tolerance is the same 3 px, off lens 1's own 0.2 degree mounting
    /// tilt.
    #[test]
    fn the_ray_out_the_back_lands_on_the_second_lens_principal_point() {
        let reframe = fixture(Camera::default());
        let (lens, landing) = shown(&reframe, [0.0, 0.0, -1.0]).expect("no lens has the back axis");

        assert_eq!(lens, 1);
        near(landing.pixel[0], 1935.35, 4.0);
        near(landing.pixel[1], 1935.09, 4.0);
    }

    /// The nominal arrangement is a rotation and not a reflection, which is
    /// what says the back hemisphere is not mirrored: a turn about the lens
    /// axis has to come out as a turn the same way round. Up in the body
    /// frame lands above lens 1's principal point and body-right lands to its
    /// left, which is what looking the other way means.
    #[test]
    fn the_back_hemisphere_is_turned_around_and_not_mirrored() {
        let reframe = fixture(Camera::default());
        // 20 degrees off the back axis, up and to the right in the body
        // frame. y is down, so up is negative.
        let up = shown(&reframe, normalize([0.0, -0.36, -1.0])).expect("nothing above the back");
        let right = shown(&reframe, normalize([0.36, 0.0, -1.0])).expect("nothing right of it");

        assert_eq!((up.0, right.0), (1, 1));
        assert!(up.1.pixel[1] < 1935.09 - 100.0, "{up:?}");
        near(up.1.pixel[0], 1935.35, 40.0);
        assert!(right.1.pixel[0] < 1935.35 - 100.0, "{right:?}");
        near(right.1.pixel[1], 1935.09, 40.0);
    }

    /// A ray 90 degrees off the axis lands inside the image circle, which is
    /// the whole point of the mirror parameter: an equidistant fisheye model
    /// cannot represent this ray at all. 1802 px of the 1913 px circle, so
    /// the frame holds roughly 195 degrees across.
    #[test]
    fn a_ray_at_ninety_degrees_lands_inside_the_image_circle() {
        let reframe = fixture(Camera::default());
        // The view ray straight out to the right is 90 degrees off the view
        // axis, and a rotation into the lens frame preserves that angle to
        // within the lens's own 0.125 degree tilt.
        let landing = reframe.project(0, [1.0, 0.0, 0.0]);

        assert!(landing.inside);
        near(radius(&reframe, 0, landing), 1802.0, 8.0);
    }

    /// And a ray past one lens's coverage does not, so the other lens has
    /// something to answer for. The polynomial happily returns a coordinate
    /// for it; only the circle test rejects it.
    #[test]
    fn a_ray_past_the_lens_lands_outside_its_image_circle() {
        let reframe = fixture(Camera::default());
        let landing = reframe.project(0, direction(120.0, 0.0));

        assert!(!landing.inside);
        near(radius(&reframe, 0, landing), 2038.0, 8.0);
        // And it is the back lens that shows it, on its own.
        let blend = reframe.blend(direction(120.0, 0.0));
        assert_eq!(blend.weights, [0.0, 1.0]);
    }

    /// The whole sphere, on a grid fine enough to walk through the seam: every
    /// direction is in some lens's picture. A gap here is the failure issue
    /// #27 names first, and it is invisible in any one rendered view because
    /// the seam is a great circle and a view only ever crosses part of it.
    #[test]
    fn no_direction_is_in_neither_lens() {
        let reframe = fixture(Camera::default());

        for theta in 0..=720 {
            for phi in 0..72 {
                let ray = direction(theta as f32 * 0.25, phi as f32 * 5.0);
                assert!(
                    reframe.blend(ray).is_covered(),
                    "no lens has {ray:?}, {} degrees off the front axis",
                    theta as f32 * 0.25
                );
            }
        }
    }

    /// Round the seam great circle, at the seam and either side of it: inside
    /// the crossover both lenses are in the picture, outside it one of them
    /// carries the ray alone, and either way something has it.
    ///
    /// The handover used to run the whole 14-degree overlap; since issue #48
    /// it runs the crossover, so the offsets that are mixed and the offsets
    /// that are not have swapped places. What has not changed is that the lens
    /// the ray leans toward is the one that leads.
    ///
    /// The offsets are taken off the width the fixture hands over across
    /// rather than written out, because that width is the camera's since
    /// 2026-08-05 and a fixture with a different overlap would silently make
    /// this test about nothing.
    #[test]
    fn the_seam_is_a_mix_of_two_pictures_and_not_a_gap() {
        let reframe = fixture(Camera::default());
        let half = 0.5 * reframe.handover_width().to_degrees();

        for phi in 0..360 {
            let phi = phi as f32;
            // Well inside the crossover and well outside it. The edges
            // themselves are half a degree of lens tilt away, which is what
            // `the_crossover_is_the_width_it_says_it_is` measures rather than
            // asserts.
            for share in [-2.0, -0.75, -0.15, 0.0, 0.15, 0.75, 2.0] {
                let offset = share * half;
                let ray = direction(90.0 + offset, phi);
                let blend = reframe.blend(ray);
                let mixed = blend.weights.iter().all(|weight| *weight > 0.0);
                assert!(blend.is_covered(), "nothing has {offset} degrees at {phi}");
                assert_eq!(
                    mixed,
                    share.abs() < 1.0,
                    "{offset} degrees from the seam at {phi} weighs {:?}",
                    blend.weights,
                );
                // Which side of the seam leads is only settled a lens tilt
                // away from it: the two axes are 0.3 degrees off exactly
                // opposed, so on the halfway line itself either lens is a
                // fair answer.
                if share == 0.0 {
                    continue;
                }
                let leader = usize::from(blend.weights[1] > blend.weights[0]);
                assert_eq!(
                    leader,
                    usize::from(share > 0.0),
                    "{offset} degrees past the seam at {phi} leads with lens {leader}",
                );
            }
        }
    }

    /// How much of the picture lens 1 holds at one offset from the seam.
    ///
    /// The handover, read where it is delivered: not `crossover`'s own share
    /// but that share after each lens's coverage depth has multiplied it and
    /// the pair has been renormalized ([`claim`]), which is what the fragment
    /// shader is actually handed.
    fn lens_one_holds(reframe: &Reframe, offset: f32, phi: f32) -> f32 {
        reframe.blend(direction(90.0 + offset, phi)).weights[1]
    }

    /// The map hands the along-seam correction over across the whole handover
    /// and not at a line inside it.
    ///
    /// This is the property `kjerag-spike --bin shear mode=profile` cannot
    /// state. That instrument reads the delivered arm against an arm with the
    /// band held off, and the held arm carries the two lenses' whole
    /// disagreement as a double image over this same corridor, so its match
    /// has two peaks in it and reports whichever leads: a step where the map
    /// has a ramp. What the map applies is here, where a weight can be read
    /// rather than fitted, and it is the whole crossover wide.
    ///
    /// **The ramp's shape is not the same at every width.** The weights are
    /// cosines of the two lens axes and not a distance, so a corridor four
    /// times as wide is a different slice of them. Measured on this fixture
    /// over 24 azimuths 2026-08-05, walking from nine tenths of the correction
    /// to one tenth, with the **linear** crossfade this shipped with until
    /// 2026-08-08:
    ///
    /// | crossover, deg | 2 | 4 | 6 | 8 | 12 |
    /// | --- | ---: | ---: | ---: | ---: | ---: |
    /// | span, deg, mean | 1.51 | 2.81 | 3.92 | 4.85 | 6.23 |
    /// | share of the width | 0.75 | 0.70 | 0.65 | 0.61 | 0.52 |
    ///
    /// Means over the 24, read at this test's own 400-step grid, which is
    /// coarse enough that the spread it reports is mostly the grid: the 8
    /// column is 4.80 to 4.92 here and 4.80 to 4.89 at ten times the
    /// resolution, and the mean moves by 0.005
    /// ([`the_blend_curve_spends_less_of_the_handover_in_view`], which is
    /// where a figure should be quoted from).
    ///
    /// So a handover four times as wide spreads the correction over 3.2 times
    /// as much picture and not four times as much. It is a ramp over the whole
    /// crossover at every one of them and never a step, which is what this
    /// asserts, and the blend curve did not change that: it is still a ramp
    /// and it still starts and ends where the support does
    /// ([`the_blend_curve_spends_less_of_the_handover_in_view`] carries the
    /// delivered figure).
    ///
    /// **The bar is a quarter of the width and it is not read off the
    /// answer.** Its whole job is to separate a ramp from a step, and the two
    /// are nowhere near each other: a step would put the two probes inside one
    /// grid step, which is 0.005 of the width, and the delivered figure is
    /// 0.482 of it under the curve and 0.61 under the ramp before it. It was
    /// half the width while there was one shape; it is a quarter now that
    /// there have been two, so that the next shape does not have to move it
    /// again.
    #[test]
    fn the_along_seam_correction_hands_over_across_the_whole_crossover() {
        let reframe = fixture(Camera::default());
        let width = reframe.handover_width().to_degrees();

        for phi in (0..360).step_by(15) {
            let phi = phi as f32;
            // Positive offsets are lens 1's side, which is the side that takes
            // the correction ([`the_seam_is_a_mix_of_two_pictures_and_not_a_gap`]).
            let carried = |offset: f32| lens_one_holds(&reframe, offset, phi);
            // The SUPPORT, which the curve leaves exactly where it was:
            // `steepen` is exact at both ends, so what the two lenses are
            // mixed over at all is the same picture it was before.
            near(carried(width), 1.0, 1e-6);
            near(carried(-width), 0.0, 1e-6);
            // Where the picture still holds nine tenths of the correction, and
            // where it is down to a tenth, walking out of lens 1. A step at the
            // halfway line would put the two within a grid step of each other.
            let steps = 400;
            let at = |share: f32| {
                (0..=steps)
                    .map(|step| width - 2.0 * width * step as f32 / steps as f32)
                    .find(|offset| carried(*offset) < share)
                    .unwrap_or(-width)
            };
            let (most, least) = (at(0.9), at(0.1));
            assert!(
                most > least,
                "the handover runs backwards at {phi}: 0.9 at {most}, 0.1 at {least}"
            );
            assert!(
                most - least > 0.25 * width,
                "the handover at {phi} spends {} degrees of the {width} it opened going from \
                 nine tenths of the correction to one tenth",
                most - least,
            );
        }
    }

    /// **How many degrees of the support the handover is visible over.** The
    /// number the blend curve existed to move, measured after the curve went.
    ///
    /// **What it asserted.** That the delivered 10-to-90 walk spans 3.81 to
    /// 3.90 degrees of the 8 degree support, and under 4.80 at every azimuth -
    /// the power-1.5 column of the table the test carried, against the linear
    /// column it replaced.
    ///
    /// **What it asserts now.** The linear column, which is the one that
    /// ships: 4.80 to 4.89 degrees, mean 4.85, over the same 24 azimuths of
    /// the same fixture at the same 4000-step grid. Both columns were measured
    /// here, on 2026-08-08, before either was chosen; the numbers are not
    /// re-derived to fit.
    ///
    /// | crossfade | linear (ships) | power 1.5 (retired) |
    /// | --- | ---: | ---: |
    /// | span, deg, mean | **4.85** | 3.85 |
    /// | span, deg, per azimuth | **4.80 to 4.89** | 3.82 to 3.88 |
    /// | share of the width | **0.61** | 0.482 |
    ///
    /// **Why it is not loosened.** The bracket is the same shape it was, a
    /// hundredth below the measured minimum and two above the measured
    /// maximum, so a number that drifts is still a failure and not a
    /// rounding. What
    /// changed is which column it brackets. The curve was chosen to spend less
    /// of the handover in view because a narrower visible handover made a
    /// *bend* less obvious; with no bend, the wider crossfade is the gentler
    /// one, and the owner picked it on his own footage against the other two.
    #[test]
    fn the_handover_spends_its_whole_support_in_view() {
        let reframe = fixture(Camera::default());
        let width = reframe.handover_width().to_degrees();

        for phi in (0..360).step_by(15) {
            let phi = phi as f32;
            let held = |offset: f32| lens_one_holds(&reframe, offset, phi);
            let steps = 4000;
            let at = |share: f32| {
                (0..=steps)
                    .map(|step| width - 2.0 * width * step as f32 / steps as f32)
                    .find(|offset| held(*offset) < share)
                    .unwrap_or(-width)
            };
            let span = at(0.9) - at(0.1);
            assert!(
                (4.79..4.91).contains(&span),
                "the handover at {phi} spends {span} degrees of {width} going from nine \
                 tenths to one tenth, not the 4.80 to 4.89 on record"
            );
        }
    }

    /// The handover is symmetric about the seam, monotone, and exact at both
    /// ends: the three things that make it a crossfade rather than a shift of
    /// the seam.
    ///
    /// **What it asserted.** The same three properties of `steepen`, the
    /// `s^n / (s^n + (1-s)^n)` curve the share was re-spent on at `n = 1.5`.
    ///
    /// **What it asserts now.** The same three properties of [`crossover`]
    /// itself, which is the ramp the curve used to sit on top of, read at real
    /// geometry rather than at a bare share: symmetric about the seam, exactly
    /// 0 and 1 at the two edges of the support, monotone in between, and the
    /// two lenses' shares summing to exactly 1 everywhere.
    ///
    /// **Why it is not loosened.** It is stated at the same tolerances on a
    /// function that is now the whole of the blend rather than half of it, so
    /// it covers strictly more than it did. `steepen` at `n = 1` was the
    /// identity, and a test of the identity is a test of nothing.
    #[test]
    fn the_handover_is_a_crossfade_and_not_a_moved_seam() {
        let band = 8.0f32.to_radians();
        let reach = 1.0f32;
        // `apart` is twice the across-seam angle, so the support runs from
        // minus to plus half a band and the ends are exact.
        assert_eq!(crossover(-band * reach, reach, band, 0.0), 0.0);
        assert_eq!(crossover(band * reach, reach, band, 0.0), 1.0);
        near(crossover(0.0, reach, band, 0.0), 0.5, 1e-6);
        let mut last = -1.0f32;
        for step in 0..=10_000 {
            let apart = (step as f32 / 10_000.0 * 2.0 - 1.0) * band * reach;
            let share = crossover(apart, reach, band, 0.0);
            assert!(share.is_finite() && (0.0..=1.0).contains(&share));
            assert!(share >= last, "the handover runs backwards at {apart}");
            // The other lens takes exactly the rest of the ray, at every
            // point of the support: this is what makes it a crossfade.
            near(share + crossover(-apart, reach, band, 0.0), 1.0, 1e-6);
            last = share;
        }
    }

    /// **Nothing can fold the picture, because nothing displaces a sample.**
    ///
    /// **What it asserted.** That the steep crossfade could not fold the
    /// narrowest camera in the corpus. Folding was a real hazard with a real
    /// failing case behind it: the corridor bend ran from zero to the whole
    /// measured disparity across the band, so its own gradient across the band
    /// **was** the shear, and above 1 the mapping printed the picture back
    /// over itself. A whole apparatus existed to keep that inequality true -
    /// `FOLD`, `SPEND`, `WIDEST_DEG`, `carried`, `width` - and this test was
    /// its acceptance, complete with a positive control that showed the
    /// undivided limit folding the ONE X2.
    ///
    /// **What it asserts now.** That the map from view ray to lens pixel is
    /// the projection alone. Every lens is sampled at the ray itself, so the
    /// Jacobian at any pixel is the lens model's own and the handover
    /// contributes nothing to it: the two lenses' landings at one ray are
    /// **exactly** what `project` returns for that ray, and the weights that
    /// mix them are a scalar per lens summing to one. A scalar blend of two
    /// samples cannot fold a mapping.
    ///
    /// **Why it is not loosened.** There is no tolerance here to loosen. The
    /// old test bounded a displacement; this one asserts the displacement is
    /// not merely small but absent, which is the stronger statement and the
    /// only one that stays true if the belt is ever built on top.
    #[test]
    fn nothing_displaces_a_sample_so_nothing_can_fold() {
        let reframe = fixture(Camera::default());
        for theta in [70.0f32, 85.0, 88.5, 90.0, 91.5, 95.0, 110.0] {
            for phi in [0.0f32, 47.0, 123.0, 250.0, 340.0] {
                let ray = direction(theta, phi);
                let blend = reframe.blend(ray);
                for lens in 0..MAX_LENSES {
                    // `Landing::MISSED` is a lens the pre-test skipped, which
                    // is a lens the model was never run for (issue #10) and a
                    // lens whose weight is zero. Everywhere else the landing
                    // is the projection of the ray and of nothing else.
                    assert!(
                        blend.landings[lens] == Landing::MISSED
                            || blend.landings[lens] == reframe.project(lens, ray),
                        "lens {lens} at theta {theta} phi {phi} was sampled off the ray",
                    );
                }
                let total: f32 = blend.weights.iter().sum();
                assert!(
                    (total - 1.0).abs() < 1e-6 || total == 0.0,
                    "the weights at theta {theta} phi {phi} sum to {total}",
                );
            }
        }
    }

    /// A run that does not ask draws the width the owner validated, and an ask
    /// the width cannot be read out of leaves it there too.
    #[test]
    fn the_handover_is_the_shipped_crossover_unless_a_width_is_asked_for() {
        assert_eq!(handover("4"), Ok(4.0));
        assert_eq!(handover("0.5"), Ok(0.5));
        assert_eq!(handover(&OVERLAP_DEG.to_string()), Ok(OVERLAP_DEG));
        for refused in ["0", "-2", "wide", "", "nan", "inf", "14.5", "90"] {
            assert!(
                handover(refused).is_err(),
                "{HANDOVER_DEG}={refused} was taken as a handover width"
            );
        }
    }

    /// The two halves of issue #48 against each other: with this file's own
    /// seam correction on lens 1, the crossover still hands the picture over
    /// and still leaves nothing grey.
    ///
    /// The correction turns one lens by a couple of degrees, so the seam and
    /// the crossover on it turn with it. What has to survive is the margin, and
    /// the margin is not what this was written against: at 2 degrees the lens
    /// the crossover hands to had 6 degrees of its own picture in hand, and at
    /// the 8 the fixture draws, the band plus the bend it carries reaches 6.60
    /// into 7.22 a side and the margin is **0.62**
    /// (`the_widest_band_and_its_bend_stay_inside_the_overlap`). This is the
    /// check that the fit cannot eat that margin at the size it comes in, and
    /// there is far less of it to eat.
    #[test]
    fn a_fitted_lens_still_hands_the_picture_over() {
        let correction = crate::seam::SeamFit {
            roll_deg: 0.801,
            yaw_deg: -2.293,
            pitch_deg: -0.817,
            ..crate::seam::SeamFit::default()
        };
        let reframe = Reframe::new(
            &correction.applied(&fixture_lenses()),
            FRAME,
            Camera::default(),
            Held::default(),
            1.0,
            false,
            Sampling::default(),
        );

        for theta in 0..=720 {
            for phi in 0..72 {
                let theta = theta as f32 * 0.25;
                let blend = reframe.blend(direction(theta, phi as f32 * 5.0));
                assert!(
                    blend.is_covered(),
                    "no lens has {theta} degrees off the front axis"
                );
                near(blend.weights.iter().sum::<f32>(), 1.0, 1e-6);
            }
        }
    }

    /// The blend's first invariant, over the whole sphere: an output pixel is
    /// one pixel's worth of picture. Anything else is a seam that reads as a
    /// bright or dark line, which is the artifact issue #7 exists to remove.
    #[test]
    fn the_weights_are_one_pixels_worth_of_picture_everywhere() {
        let reframe = fixture(Camera::default());

        for theta in 0..=720 {
            for phi in 0..72 {
                let theta = theta as f32 * 0.25;
                let blend = reframe.blend(direction(theta, phi as f32 * 5.0));
                let total: f32 = blend.weights.iter().sum();
                near(total, 1.0, 1e-6);
                assert!(
                    blend.weights.iter().all(|weight| *weight >= 0.0),
                    "{theta} degrees off the front axis weighs {:?}",
                    blend.weights
                );
            }
        }
    }

    /// Outside the overlap the second lens contributes nothing, and the first
    /// one's weight is exactly 1 rather than nearly it. That exactness is
    /// what makes the picture away from the seam the same bits it was before
    /// the blend, and it is what lets the shader skip the second fetch.
    #[test]
    fn one_lens_carries_everything_outside_the_overlap() {
        let reframe = fixture(Camera::default());

        for theta in [0.0, 30.0, 60.0, 80.0, 100.0, 130.0, 180.0] {
            for phi in 0..8 {
                let blend = reframe.blend(direction(theta, phi as f32 * 45.0));
                assert_eq!(
                    blend.weights,
                    match theta < 90.0 {
                        true => [1.0, 0.0],
                        false => [0.0, 1.0],
                    },
                    "{theta} degrees off the front axis"
                );
            }
        }
    }

    /// The crossover sits on the seam, which is what naming it by the two
    /// lenses' own angles buys: without that it would cross wherever the two
    /// image circles happen to end.
    ///
    /// Not exactly half: this fixture's two axes are 0.3 degrees from opposed,
    /// so a direction 90 degrees off lens 0 is up to 0.3 degrees off the line
    /// where the two lenses are equally far off theirs. What that is worth in
    /// weight is the width's business, and the width has moved twice. Measured
    /// on this fixture: **0.008** across the 14-degree overlap, **0.06** at the
    /// 2 issue #48 shipped, and **0.0264** at the 8 the picture draws since
    /// 2026-08-05. It is centred on the lenses at every one of them; what moves
    /// is how quickly weight answers an angle, and it answers 2.3 times less
    /// quickly at 8 than at 2 rather than four times, which is the same
    /// non-linearity
    /// `the_along_seam_correction_hands_over_across_the_whole_crossover` reads
    /// on the ramp.
    ///
    /// The bar is 0.04 because the effect is 0.0264. It was 0.08 when the
    /// effect was 0.06, and a bar three times what it watches is a test that
    /// has stopped watching.
    #[test]
    fn the_crossover_sits_on_the_seam() {
        let reframe = fixture(Camera::default());

        for phi in 0..36 {
            let blend = reframe.blend(direction(90.0, phi as f32 * 10.0));
            near(blend.weights[0], 0.5, 0.04);
            near(blend.weights[1], 0.5, 0.04);
        }
    }

    /// Continuity, which is the property the eye actually reads: swept
    /// through the whole band a hundred steps to the degree, no lens's weight
    /// ever moves more than a hundredth in one step. The hard pick this
    /// replaces moved 1.0 in one step, at the seam, which is the line issue
    /// #7 was filed about.
    ///
    /// The sweep runs well past both edges of the overlap, so it also covers
    /// the two places a weight arrives at 0: the band edge is where a blend
    /// with a feather width of its own would show a crease.
    #[test]
    fn no_weight_ever_steps() {
        let reframe = fixture(Camera::default());

        for phi in [0.0, 90.0, 180.0, 270.0] {
            let mut held = reframe.blend(direction(70.0, phi)).weights;
            let mut worst: f32 = 0.0;

            for step in 1..=4000 {
                let weights = reframe
                    .blend(direction(70.0 + step as f32 * 0.01, phi))
                    .weights;
                for lens in 0..MAX_LENSES {
                    worst = worst.max((weights[lens] - held[lens]).abs());
                }
                held = weights;
            }

            assert!(worst < 0.01, "a weight jumped by {worst} at phi {phi}");
        }
    }

    /// And the band is [`CROSSOVER_DEG`] wide, in degrees of world angle, at
    /// every azimuth: the number the owner validated is the number the
    /// picture gets (issue #48).
    ///
    /// This is also the check on the small-angle step in [`crossover`], which
    /// reads the two angles off their cosines and never takes an arc cosine:
    /// a band measured 0.01 degrees at a time comes out 2.00 degrees wide,
    /// and any error in that reading would show here as a band of the wrong
    /// size. Before issue #48 the same sweep read 83.2 to 97.4 degrees, the
    /// whole overlap.
    #[test]
    fn the_crossover_is_the_width_it_says_it_is() {
        let reframe = fixture(Camera::default());

        for phi in [0.0, 90.0, 180.0, 270.0] {
            let mixed: Vec<f32> = (0..3000)
                .map(|step| 70.0 + step as f32 * 0.01)
                .filter(|theta| {
                    let weights = reframe.blend(direction(*theta, phi)).weights;
                    weights.iter().all(|weight| *weight > 0.0)
                })
                .collect();
            let (first, last) = (
                *mixed.first().expect("nothing is mixed at all"),
                *mixed.last().expect("nothing is mixed at all"),
            );
            near(last - first, CROSSOVER_DEG, 0.02);
            near(0.5 * (first + last), 90.0, 0.2);
        }
    }

    /// **A near reading does not open the crossover, and that is the change.**
    ///
    /// **What it asserted.** That a direction reading 1.8, 2.2 or 2.6 degrees
    /// of disparity was drawn across a band widened to carry that reading
    /// without folding - measured in the picture, as the span of view over
    /// which both lenses have a positive weight, against the width
    /// `crossover_at` asked for. That was stage 4, and the width it produced
    /// moved frame by frame with the near field.
    ///
    /// **What it asserts now.** That the drawn span is the camera's own width
    /// at every one of those readings, to the same 0.02 degrees, and that it
    /// is still centred on the seam. Same fixture, same four readings, same
    /// four azimuths, same bar; the expected value is a constant instead of a
    /// function.
    ///
    /// **Why it is not loosened.** The 0.02 degree bracket is the one the
    /// stage-4 test used, and it is now being asked of a number that must not
    /// move at all rather than of one that was supposed to move. The reading
    /// is still constructed and still non-zero - a test that froze the width
    /// by feeding the map nothing would prove nothing.
    #[test]
    fn a_near_reading_does_not_open_the_crossover() {
        let mut reframe = fixture(Camera::default());
        reframe.crossover = 2.0f32.to_radians();
        let wanted = reframe.handover_width().to_degrees();
        for disparity_deg in [0.0f32, 1.8, 2.2, 2.6] {
            let disparity = disparity_deg.to_radians();
            // The reading exists and says something; the picture does not
            // listen. `reading` is the same helper stage 4 measured through.
            assert!((reading(disparity).epi - disparity).abs() < 1e-9);
            for phi in [0.0, 90.0, 180.0, 270.0] {
                let mixed: Vec<f32> = (0..3000)
                    .map(|step| 70.0 + step as f32 * 0.01)
                    .filter(|theta| {
                        let weights = reframe.blend(direction(*theta, phi)).weights;
                        weights.iter().all(|weight| *weight > 0.0)
                    })
                    .collect();
                let (first, last) = (
                    *mixed.first().expect("nothing is mixed at all"),
                    *mixed.last().expect("nothing is mixed at all"),
                );
                near(last - first, wanted, 0.02);
                near(0.5 * (first + last), 90.0, 0.2);
            }
        }
    }

    /// **The widest band stays inside the overlap while the line is on the
    /// seam - and runs past it as soon as the line is held.**
    ///
    /// **What it asserted.** That the widest band this camera could open to -
    /// its floor, or a near-field reading past that floor - plus the widest
    /// bend that band could carry, still landed inside the picture both lenses
    /// have. On the fixture that was a band of 8.00 reaching 6.60 into 7.22 a
    /// side: 0.62 degrees to spare.
    ///
    /// **What it asserts now, and the half of it that was missing.** The bend
    /// is struck out, so an unshifted band of 8.00 reaches 4.00 into the same
    /// 7.22 and has 3.22 degrees to spare where it had 0.62 - that part is
    /// unchanged and it is the headroom the flat seam bought back. What this
    /// test used to leave out, and what a review found in it on 2026-08-09, is
    /// that **the shipped picture does not draw an unshifted band**: the
    /// handover's support is centred on the drawn line, the line is held on
    /// world content ([`SeamAnchor`]), and at the allowance the support reaches
    /// `band / 2 + allowance` = a whole band off the seam. Written against the
    /// widths this camera actually uses, the old inequality is FALSE at the
    /// rail, and the second assertion below is the true one stated in the
    /// direction it is true in.
    ///
    /// **Why it is not loosened.** Nothing here got a wider tolerance: a claim
    /// that was made about the shipped picture and only held for one value of
    /// one field is now made about the case it holds in, and the case it does
    /// not hold in is asserted as the fact it is. What carries the overshoot
    /// safely is `the_anchored_handover_leaves_no_hole_and_no_cliff`, which is
    /// the test this one used to be mistaken for.
    #[test]
    fn the_widest_band_stays_inside_the_overlap() {
        let reframe = fixture(Camera::default());
        let overlap = reframe
            .overlap()
            .expect("the fixture has two lenses")
            .to_degrees();
        let widest = reframe.handover_width();
        let reach = crate::band::reach(widest).to_degrees();
        assert!(
            reach < 0.5 * overlap,
            "an unshifted band of {:.2} deg reaches {reach:.2} deg off the seam into an overlap \
             of {overlap:.2} deg, which is {:.2} deg a side",
            widest.to_degrees(),
            0.5 * overlap,
        );
        // And the room the bend used to want is measurably back: 3.22 degrees
        // a side on this fixture, against the 0.62 the bent band left.
        let spare = 0.5 * overlap - reach;
        assert!(
            spare > 3.0,
            "the flat band leaves only {spare:.2} deg a side, not the 3.22 on record",
        );
        // The other half, and the reason the sentence above needs its first
        // four words. At the allowance the support runs a whole band off the
        // seam on one side, which is 0.78 degrees PAST the shared picture on
        // the roomiest camera there is and 3.41 past it on an X2-class one.
        for (name, reframe) in [
            ("the fixture", fixture(Camera::default())),
            ("an X2-class camera", cropped(X2_CLASS)),
        ] {
            let overlap = reframe.overlap().expect("two lenses").to_degrees();
            let band = reframe.handover_width().to_degrees();
            let farthest = 0.5 * band + 0.5 * band;
            assert!(
                farthest > 0.5 * overlap,
                "{name} holds its line inside the overlap after all: a band of {band:.2} at the \
                 {:.2} degree allowance reaches {farthest:.2} against {:.2} a side, so the claim \
                 this test used to make would be true and the doc on Reframe::afforded is wrong",
                0.5 * band,
                0.5 * overlap,
            );
        }
    }

    /// **The held line puts the handover past the picture, and nothing falls
    /// through the gap.** The safety property the width clamp was mistaken for.
    ///
    /// **Why it exists.** `Reframe::afforded` clamps the handover to the
    /// camera's own overlap and said, until 2026-08-09, that this kept the
    /// handover inside the picture both lenses have, because "a handover of
    /// width `w` reaches `w / 2` off the seam". With [`SeamAnchor`] that is
    /// false: the support is centred on the DRAWN line, so it runs
    /// `band / 2 + |shift|` off the seam and `|shift|` is allowed up to
    /// `band / 2`. Over the July-14 fast segment 202 of 900 frames draw some
    /// support past the coverage on the owner's own X4 Air, and 866 of 900
    /// would on a camera that overlaps the way the ONE X2 does; at the rail
    /// there the ramp is still asking for a sample 8.00 degrees off a seam
    /// whose shared picture stops at 4.59. The two guards that existed were tautologies - one
    /// asserts `width / 2 < overlap / 2` for widths the clamp already caps at
    /// the overlap, the other is `min(8, o) / 2 <= o / 2` - and neither has a
    /// shift in it.
    ///
    /// **What it asserts, over the whole ring at the rail on both camera
    /// classes.** The two properties that are what "safe" means for a
    /// crossfade:
    ///
    /// - **no hole**: the delivered weights sum to one at every direction, so
    ///   no pixel is left transparent by a ramp that zeroed the only lens with
    ///   the ray;
    /// - **no cliff**: the delivered weight never steps by more than this
    ///   test's bar from one probe to the next, so the handover is still a fade
    ///   out there and not an edge.
    ///
    /// Measured 2026-08-09, this grid, both classes: the sum is one to a single
    /// ulp (0.99999988, which is `share`'s division and not a gap), and the
    /// worst step is **0.0025** per hundredth of a degree on the X4 Air fixture
    /// and **0.0034** on the X2-class one.
    ///
    /// **What 0.0040 is, said plainly: a regression bar and not a safety
    /// threshold.** It is set a sixth above the worst this grid measures - and
    /// above the 0.003485 a finer sweep of the same ring read in review - so
    /// that a change to the taper trips it and ordinary rounding does not.
    /// Nothing derives it, and in particular the fade's own mean slope over its
    /// delivered 10-to-90 walk, which is 0.0017, is **context for the size of
    /// the number and not the derivation of it**: this test cannot say at what
    /// step a fade stops reading as a fade, because no eye has been asked that
    /// question. What makes the bar worth asserting is the **gap to the
    /// control**: the planted cliff below steps the weight by 0.36, a hundred
    /// times this bar and a hundred times the worst measured, so the two are
    /// two orders apart and where in that gap the line is drawn changes no
    /// verdict this test has ever returned.
    ///
    /// **What actually carries it** is [`claim`]: a lens's share is multiplied
    /// by its own coverage depth, which reaches zero exactly where that lens
    /// runs out of picture, so the outer lens is faded to nothing by its own
    /// rim before the ramp can ask it for a sample it does not have. The
    /// controls say so by breaking exactly that:
    ///
    /// - **the planted cliff**: the same weights with the coverage depth
    ///   replaced by a hard in-or-out step. Everything else is the shipped map.
    ///   The step at the rim is then the whole share the ramp is still handing
    ///   the outer lens, which this grid reads at 0.36, a hundred times the
    ///   bar.
    /// - **the planted hole**: a shift written straight into the block past
    ///   what [`Reframe::with_shift`] allows, which is the one thing that can
    ///   put the ramp's closing end outside the shared picture. The weights
    ///   then sum to zero over a stretch of the ring, and the margin the
    ///   shipped clamp holds against it is `overlap / 2`.
    #[test]
    fn the_anchored_handover_leaves_no_hole_and_no_cliff() {
        for (name, reframe) in [
            ("the X4 Air fixture", fixture(Camera::default())),
            ("an X2-class camera", cropped(X2_CLASS)),
        ] {
            let overlap = reframe.overlap().expect("two lenses").to_degrees();
            let allowance = 0.5 * reframe.handover_width();
            let mut worst_sum = f32::INFINITY;
            let mut worst_step = 0.0f32;
            for shift in shifts(allowance) {
                let held = reframe.with_shift(shift);
                for phi in (0..360).step_by(5) {
                    let (sum, step) = walked(&held, phi as f32, delivered);
                    worst_sum = worst_sum.min(sum);
                    worst_step = worst_step.max(step);
                }
            }
            assert!(
                worst_sum > 1.0 - 1e-6,
                "{name} leaves a pixel weighing {worst_sum} of a whole one somewhere on the ring: \
                 the ramp zeroed the only lens that had the ray",
            );
            assert!(
                worst_step < 0.0040,
                "{name} steps the delivered weight by {worst_step:.6} in a hundredth of a degree, \
                 which is an edge and not a fade",
            );

            // The control for the cliff: the same map with the coverage taper
            // broken to a hard edge, which is the term that carries the
            // overshoot. It has to fail the bar this test just passed.
            let mut planted = 0.0f32;
            for shift in shifts(allowance) {
                let held = reframe.with_shift(shift);
                for phi in (0..360).step_by(5) {
                    planted = planted.max(walked(&held, phi as f32, hard_edged).1);
                }
            }
            assert!(
                planted > 0.10,
                "{name}: breaking the coverage taper only steps the weight by {planted:.6}, so \
                 this test would pass a map with no taper in it and proves nothing",
            );

            // The control for the hole, and the measurement of what stops it.
            // A shift past the clamp by more than half the overlap puts the
            // ramp's closing end outside the shared picture; the clamp allows
            // half the band, so the margin is half the overlap.
            let mut torn = reframe;
            torn.handover_shift =
                (0.5 * reframe.handover_width().to_degrees() + 0.5 * overlap + 0.5).to_radians();
            let mut lowest = f32::INFINITY;
            for phi in (0..360).step_by(5) {
                lowest = lowest.min(walked(&torn, phi as f32, delivered).0);
            }
            assert!(
                lowest < 1e-6,
                "{name}: a shift of {:.2} degrees, which is past everything, still leaves the \
                 weights summing to {lowest}, so the no-hole assertion above cannot fail",
                torn.handover_shift.to_degrees(),
            );
            assert_eq!(
                reframe.with_shift(torn.handover_shift).handover_shift,
                allowance,
                "{name}: the clamp that stands between the anchor and that hole did not fire",
            );
        }
    }

    /// **A camera that cannot pay for the width the picture asks for gets the
    /// width it can pay for**, asked of the code that ships rather than of a
    /// copy of its arithmetic.
    ///
    /// **Why it exists.** `Reframe::afforded` is `asked.min(overlap)` and
    /// nothing in the suite bound that `min` until 2026-08-09. Every fixture
    /// here overlaps by 14.44 and every file in the corpus by 9.19 or more,
    /// all of them over the 8 degrees the picture asks for, so `min` picked the
    /// ask in every test there was: deleting the clamp outright left the whole
    /// workspace green. `band::tests::the_width_a_camera_can_pay_for_is_its_own_overlap`
    /// looks like the missing one and is not - it recomputes `CROSSOVER_DEG.min(overlap)`
    /// beside the code instead of calling it, so it would pass a build whose
    /// `afforded` did anything at all.
    ///
    /// **What it asserts.** Three cameras through [`Reframe::new`], which is
    /// the only door `afforded` has, reading the answer back off the block the
    /// shader is handed:
    ///
    /// - the X4 Air fixture, roomy at 14.44, draws the ask;
    /// - an X2-class camera at 9.18 draws the ask **as well**, which is the
    ///   disclosed picture change of this merge (the ONE X2 went from 3.94 to
    ///   8.00) and is why the corpus alone cannot bind the clamp;
    /// - a camera at 7.43, narrower than the ask, draws **7.43** and not 8.
    ///
    /// The third is the one that binds, and it is written as an equality
    /// against the camera's own measured overlap rather than against a
    /// constant, so a build that clamps to something else - or to nothing -
    /// fails here.
    #[test]
    fn a_camera_narrower_than_the_ask_draws_what_it_can_pay_for() {
        let asked = CROSSOVER_DEG;
        for (name, reframe, overlap, draws) in [
            (
                "the X4 Air fixture",
                fixture(Camera::default()),
                14.44f32,
                asked,
            ),
            ("an X2-class camera", cropped(X2_CLASS), 9.18, asked),
            ("a camera under the ask", cropped(UNDER_THE_ASK), 7.43, 7.43),
        ] {
            let measured = reframe.overlap().expect("two lenses").to_degrees();
            assert!(
                (measured - overlap).abs() < 0.01,
                "{name} overlaps by {measured:.4} and not the {overlap} this fixture is here to \
                 stand for",
            );
            let width = reframe.handover_width().to_degrees();
            assert!(
                (width - draws).abs() < 0.01,
                "{name} overlaps by {measured:.2} and draws {width:.4}, not {draws}",
            );
            // And the clamp is the one that chose it: the width is the smaller
            // of the two, whichever that is on this camera.
            assert!(
                (width - asked.min(measured)).abs() < 0.01,
                "{name} draws {width:.4}, which is neither the {asked} asked for nor the \
                 {measured:.4} it overlaps by",
            );
        }
        // The negative control, and the whole reason the third camera is here:
        // with the clamp deleted, `afforded` would be the ask alone, and that
        // is a different answer on exactly one of the three.
        assert!(
            cropped(UNDER_THE_ASK).handover_width().to_degrees() < asked - 0.5,
            "the narrow camera draws the whole ask, so deleting the clamp would still pass",
        );
    }

    /// The shifts the safety sweep is run at: the rail both ways, three
    /// quarters of it both ways, and none.
    fn shifts(allowance: f32) -> [f32; 5] {
        [
            -allowance,
            -0.75 * allowance,
            0.0,
            0.75 * allowance,
            allowance,
        ]
    }

    /// Lens 1's delivered weight and whether the pair covers the ray at all:
    /// what the fragment shader is handed, after the coverage depth and the
    /// renormalization ([`claim`]).
    fn delivered(reframe: &Reframe, ray: [f32; 3]) -> (f32, f32) {
        let weights = reframe.blend(ray).weights;
        (weights[0] + weights[1], weights[1])
    }

    /// The same with the coverage depth broken to a hard in-or-out step, which
    /// is the control for the cliff. Every other term is the shipped map's.
    fn hard_edged(reframe: &Reframe, ray: [f32; 3]) -> (f32, f32) {
        let reach = norm3(ray);
        let axis: [f32; MAX_LENSES] = std::array::from_fn(|lens| reframe.axis_of(lens, ray));
        let front = reframe.handover(axis, reach, reframe.crossover);
        let mut weights = [0.0; MAX_LENSES];
        for (lens, weight) in weights.iter_mut().enumerate().take(2) {
            let share = match lens {
                0 => front,
                _ => 1.0 - front,
            };
            let landing = reframe.project(lens, ray);
            *weight = share * f32::from(u8::from(landing.inside));
        }
        let total: f32 = weights.iter().sum();
        match total > 0.0 {
            true => (1.0, weights[1] / total),
            false => (0.0, 0.0),
        }
    }

    /// Walks one azimuth across the whole seam and past both lenses' rims,
    /// returning the smallest coverage the pair ever showed and the largest
    /// step the weight took in a hundredth of a degree.
    fn walked(
        reframe: &Reframe,
        phi: f32,
        read: fn(&Reframe, [f32; 3]) -> (f32, f32),
    ) -> (f32, f32) {
        let (mut lowest, mut step, mut previous) = (f32::INFINITY, 0.0f32, None::<f32>);
        for probe in 0..=2600 {
            let theta = 90.0 + (-13.0 + probe as f32 * 0.01);
            let (sum, weight) = read(reframe, direction(theta, phi));
            lowest = lowest.min(sum);
            if let Some(before) = previous {
                step = step.max((weight - before).abs());
            }
            previous = Some(weight);
        }
        (lowest, step)
    }

    /// Issue #10's pre-test, and the only property it has to have: what it
    /// drops, the weight field was going to weigh at zero anyway.
    ///
    /// Checked over the whole sphere at four cameras, because `within` reads
    /// the ray in **view** space and the model reads it in the lens's, so a
    /// composition that agreed only at yaw zero would pass a body-frame sweep
    /// and put a hole in the picture the moment the view turned. The
    /// consequence is stated as the weights rather than as `inside`: a weight
    /// is what the shader multiplies a sample by.
    #[test]
    fn the_cap_never_drops_a_ray_a_lens_has() {
        for camera in cameras() {
            let reframe = fixture(camera);
            for theta in 0..=720 {
                for phi in 0..72 {
                    let ray = direction(theta as f32 * 0.25, phi as f32 * 5.0);
                    let weights = reframe.blend(ray).weights;
                    for (lens, weight) in weights.iter().enumerate() {
                        assert!(
                            reframe.within(lens, ray) || *weight == 0.0,
                            "lens {lens} is skipped at {} degrees off the front axis but weighs \
                             {weight}",
                            theta as f32 * 0.25,
                        );
                    }
                }
            }
        }
    }

    /// And the pass writes the same picture with it as without: the weights
    /// are the same **bits**, not nearly the same numbers, because the ulp
    /// guard in [`share`] exists for exactly this reason and a picture that
    /// moved by one code would undo it.
    ///
    /// The reference is the loop as it was before issue #10: every lens
    /// projected, whatever the cap says.
    #[test]
    fn skipping_a_lens_writes_the_weights_it_wrote_before() {
        for camera in cameras() {
            let reframe = fixture(camera);
            for theta in 0..=720 {
                for phi in 0..72 {
                    let ray = direction(theta as f32 * 0.25, phi as f32 * 5.0);
                    assert_eq!(
                        reframe.blend(ray).weights,
                        weighed_without_the_cap(&reframe, ray),
                        "{} degrees off the front axis at phi {}",
                        theta as f32 * 0.25,
                        phi * 5,
                    );
                }
            }
        }
    }

    /// A ray past the readout as well: with issue #9's correction forced on
    /// at a rate past anything this footage flies, the model is handed a ray
    /// turned by up to half a readout, and the cap has to cover where **that**
    /// ray lands rather than where this one does.
    #[test]
    fn the_cap_covers_the_ray_the_readout_turns_it_into() {
        for rate in [90.0f64, 250.0, 523.0] {
            let turn = (rate * 0.015_883).to_radians();
            let reframe = held(Camera::default(), rolling([turn * 0.3, turn, turn * 0.6]));
            for theta in 0..=720 {
                for phi in 0..36 {
                    let ray = direction(theta as f32 * 0.25, phi as f32 * 10.0);
                    assert_eq!(
                        reframe.blend(ray).weights,
                        weighed_without_the_cap(&reframe, ray),
                        "{rate} deg/s at {} degrees off the front axis",
                        theta as f32 * 0.25,
                    );
                }
            }
        }
    }

    /// How much the cap costs, which is the other half of choosing it: rays
    /// it keeps that turn out to weigh nothing.
    ///
    /// The two numbers this prints are what [`CAP_MARGIN_DEG`] and
    /// [`CAP_AZIMUTHS`] are set from. The support's own boundary is not a
    /// circle, and the spread between the widest and narrowest azimuth is the
    /// error eight samples can make; the gap is that spread plus the margin,
    /// and it is what the pass pays for.
    #[test]
    fn the_cap_is_tight_against_the_support() {
        let reframe = fixture(Camera::default());

        for lens in 0..MAX_LENSES {
            let edges: Vec<f32> = (0..360)
                .map(|phi| support_edge(&reframe, lens, phi as f32))
                .collect();
            let widest = edges.iter().copied().fold(f32::MIN, f32::max);
            let narrowest = edges.iter().copied().fold(f32::MAX, f32::min);
            let cap = reframe.lenses[lens].axis_min.acos().to_degrees();
            // What eight azimuths missed: the cap without its margin against
            // the widest azimuth of three hundred and sixty.
            let missed = widest - (cap - CAP_MARGIN_DEG);
            println!(
                "lens {lens}: support {narrowest:.3} to {widest:.3} degrees ({:.3} of spread), \
                 cap {cap:.3}, {:.3} past the widest, eight azimuths missed {missed:.3}",
                widest - narrowest,
                cap - widest,
            );

            assert!(cap > widest, "the cap {cap} is inside the support {widest}");
            // The margin has to cover what the sampling missed, and be worth
            // no more than that: everything between the two is projections
            // that weigh nothing.
            assert!(missed < 0.1, "eight azimuths missed {missed} degrees");
            assert!(
                cap - widest < CAP_MARGIN_DEG,
                "the cap is {} degrees past the support",
                cap - widest,
            );
        }
    }

    /// What the whole thing is for: looking down one lens's axis, no pixel of
    /// the output runs the other lens's model at all.
    ///
    /// 90 degrees of field of view at 16:9, which is the app's default, and
    /// the corners are the part of it nearest the other hemisphere.
    #[test]
    fn a_view_down_one_axis_projects_one_lens() {
        let aspect = 16.0 / 9.0;
        let reframe = Reframe::new(
            &fixture_lenses(),
            FRAME,
            Camera::default(),
            Held::default(),
            aspect,
            false,
            Sampling::default(),
        );

        for down in 0..=64 {
            for across in 0..=64 {
                let uv = [across as f32 / 64.0, down as f32 / 64.0];
                let ray = ray(&reframe, uv);
                assert!(
                    reframe.within(0, ray),
                    "the front lens is skipped at {uv:?}"
                );
                assert!(
                    !reframe.within(1, ray),
                    "the back lens is projected at {uv:?}"
                );
            }
        }
    }

    /// What ties the view-level question to the pixel-level one, and the
    /// property a decode gate would have rested on: where
    /// [`Reframe::reaches`] says no, no ray of the output reaches that lens
    /// either.
    ///
    /// The two are asked at different scales and answered by different
    /// arithmetic, one about a cone and one about a ray, so they can disagree
    /// in only one safe direction. This is that direction, checked at the
    /// corners and edges as well as the middle, because the corner is the
    /// part of a rectangle furthest from the view axis and the reason
    /// [`Reframe::cone`] is measured off it.
    #[test]
    fn no_ray_of_a_gated_view_reaches_the_lens() {
        for fov in [20.0f32, 45.0, 90.0, 110.0] {
            for yaw in (0..360).step_by(9) {
                for pitch in [-80.0f32, -35.0, 0.0, 35.0, 80.0] {
                    let camera = Camera {
                        yaw: (yaw as f32).to_radians(),
                        pitch: pitch.to_radians(),
                        fov: fov.to_radians(),
                    };
                    let reframe = Reframe::new(
                        &fixture_lenses(),
                        FRAME,
                        camera,
                        Held::default(),
                        16.0 / 9.0,
                        false,
                        Sampling::default(),
                    );
                    for lens in 0..MAX_LENSES {
                        if reframe.reaches(lens, 0.0) {
                            continue;
                        }
                        for down in 0..=16 {
                            for across in 0..=16 {
                                let uv = [across as f32 / 16.0, down as f32 / 16.0];
                                assert!(
                                    !reframe.within(lens, ray(&reframe, uv)),
                                    "lens {lens} is out of reach at fov {fov}, yaw {yaw}, pitch \
                                     {pitch}, but {uv:?} is inside its cap",
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// And it only ever loosens: a wider view or more margin reaches at least
    /// as far. A gate built on a test that tightened as the view opened would
    /// engage while the corners were still looking at the far lens.
    #[test]
    fn opening_the_view_or_the_margin_only_ever_reaches_further() {
        for yaw in (0..360).step_by(15) {
            let camera = |fov: f32| Camera {
                yaw: (yaw as f32).to_radians(),
                pitch: 0.2,
                fov: fov.to_radians(),
            };
            let build = |fov| {
                Reframe::new(
                    &fixture_lenses(),
                    FRAME,
                    camera(fov),
                    Held::default(),
                    16.0 / 9.0,
                    false,
                    Sampling::default(),
                )
            };
            for lens in 0..MAX_LENSES {
                for pair in [(20.0f32, 45.0f32), (45.0, 90.0), (90.0, 110.0)] {
                    assert!(
                        build(pair.1).reaches(lens, 0.0) || !build(pair.0).reaches(lens, 0.0),
                        "lens {lens} at yaw {yaw} reaches at fov {} and not at {}",
                        pair.0,
                        pair.1,
                    );
                }
                let wide = build(90.0);
                for margin in [0.1f32, 0.4, 1.0] {
                    assert!(
                        wide.reaches(lens, margin) || !wide.reaches(lens, 0.0),
                        "lens {lens} at yaw {yaw} reaches at no margin and not at {margin}",
                    );
                }
            }
        }
    }

    /// And a file with one stream never projects the slot that has no
    /// picture in it, wherever it looks. That slot was projected on every
    /// pixel before issue #10, for a weight of zero.
    #[test]
    fn an_empty_slot_is_never_projected() {
        for camera in cameras() {
            let reframe = one_lens(camera);
            for theta in (0..=180).step_by(3) {
                for phi in (0..360).step_by(15) {
                    let ray = direction(theta as f32, phi as f32);
                    assert!(!reframe.within(1, ray), "{theta} degrees, phi {phi}");
                }
            }
        }
    }

    /// A 2560x1440 window, which is where the player's own numbers are
    /// measured and what a texel-to-pixel ratio is a ratio against.
    const WINDOW: Size = Size {
        width: 2560,
        height: 1440,
    };

    fn windowed(fov_deg: f32) -> Reframe {
        Reframe::new(
            &fixture_lenses(),
            FRAME,
            Camera {
                fov: fov_deg.to_radians(),
                ..Camera::default()
            },
            Held::default(),
            WINDOW.width as f32 / WINDOW.height as f32,
            false,
            Sampling::default(),
        )
    }

    /// What the ratio has to be down the view axis, from two closed forms
    /// that owe the Jacobian nothing.
    ///
    /// Near its own axis the Mei model's focal length is `fx / (1 + xi)`
    /// texels per radian, because `sin(theta) / (cos(theta) + xi)` is
    /// `theta / (1 + xi)` there and the radial polynomial is 1. A
    /// rectilinear output's is `width / (2 tan(fov / 2))` pixels per radian,
    /// for the same reason in the other direction. The ratio of the two is
    /// the magnification, and it is 1105.7 over 1280 at the app's default
    /// field of view: the player is already magnifying this camera by 16%
    /// before anyone touches the wheel.
    fn paraxial_ratio(fov_deg: f32) -> f32 {
        let lens = fixture_lenses()[0].intrinsics;
        let source = (lens.fx / (1.0 + lens.xi)) as f32;
        let output = WINDOW.width as f32 / (2.0 * (fov_deg.to_radians() * 0.5).tan());
        source / output
    }

    /// The Jacobian the shader samples by, against arithmetic that shares no
    /// line with it. Down the view axis, where both closed forms hold, over
    /// the whole zoom range.
    ///
    /// A percent of tolerance covers the lens's own 0.125 degree mounting
    /// tilt and the finite difference being taken over a real output pixel
    /// rather than in the limit.
    #[test]
    fn the_texel_ratio_is_the_focal_length_the_model_has_on_its_axis() {
        for fov in [20.0f32, 45.0, 90.0, 110.0] {
            let ratio = windowed(fov).texels_per_pixel(0, [0.5, 0.5], WINDOW);
            let paraxial = paraxial_ratio(fov);
            assert!(
                (ratio / paraxial - 1.0).abs() < 0.01,
                "fov {fov}: the map magnifies by {ratio} against {paraxial} paraxial",
            );
        }
    }

    /// What the whole issue rests on: zoomed in, an output pixel is inside
    /// one source texel, and zoomed out it is not. Down the front lens's
    /// axis at a 2560 px window, 20 degrees of field of view is six and a
    /// half output pixels to the texel, the app's own default of 90 is one
    /// and a sixth, and only past 98 does an output pixel hold a whole texel
    /// again.
    #[test]
    fn a_narrow_view_magnifies_the_source_and_a_wide_one_does_not() {
        let middle = |fov| windowed(fov).texels_per_pixel(0, [0.5, 0.5], WINDOW);

        near(middle(20.0), 0.152, 0.002);
        near(middle(90.0), 0.864, 0.002);
        assert!(middle(110.0) > 1.0, "{} at fov 110", middle(110.0));
        // And it is monotone in the zoom, which is what makes one threshold
        // an answer at all.
        let mut held = 0.0;
        for fov in [20.0f32, 25.0, 35.0, 60.0, 90.0, 100.0, 110.0] {
            let ratio = middle(fov);
            assert!(
                ratio > held,
                "fov {fov} magnifies less than the view before"
            );
            held = ratio;
        }
    }

    /// And it is **local**, which is why the shader asks per fragment rather
    /// than per redraw. Two things vary and they do not cancel: the fisheye's
    /// own angular density, and the rectilinear output's, which rises towards
    /// a corner as the cosine squared of the angle off the view axis.
    ///
    /// At the widest view the player offers, the middle of the picture is
    /// past 1:1 (1.234) and the corners of the same picture are two thirds
    /// of the way inside it (0.743), which is 1.66 times. A single ratio for
    /// the view would have to be wrong at one end or the other.
    #[test]
    fn the_texel_ratio_is_not_uniform_across_the_frame() {
        let wide = windowed(110.0);
        let middle = wide.texels_per_pixel(0, [0.5, 0.5], WINDOW);
        let corner = wide.texels_per_pixel(0, [0.98, 0.98], WINDOW);

        assert!(
            middle > 1.0,
            "the middle of a 110 degree view does not magnify"
        );
        assert!(corner < 0.8, "the corner does: {corner}");
        assert!(
            middle / corner > 1.5,
            "{middle} in the middle against {corner} in the corner",
        );

        // It is not flat at the narrow end either, where the output's own
        // fall-off is small and the lens's density does the varying.
        let narrow = windowed(20.0);
        let spread = narrow.texels_per_pixel(0, [0.5, 0.5], WINDOW)
            / narrow.texels_per_pixel(0, [0.98, 0.98], WINDOW);
        assert!(
            (1.01..1.03).contains(&spread),
            "{spread} across a 20 degree view"
        );
    }

    /// The NV12 wrinkle where it actually lands: at the app's default view,
    /// on this camera, at this window, the chroma plane is magnified and the
    /// luma plane is not. The two planes need two thresholds because they
    /// really do answer differently over the range the player is used in.
    #[test]
    fn the_chroma_plane_is_magnified_where_the_luma_plane_is_not() {
        let engaged = |fov, plane: f32| {
            let ratio = windowed(fov).texels_per_pixel(0, [0.5, 0.5], WINDOW);
            sampling::sharpen(sampling::plane_ratio(ratio, plane, FRAME.width as f32), 1.0)
        };
        let luma = FRAME.width as f32;
        let chroma = luma * 0.5;

        for fov in [100.0f32, 110.0] {
            assert_eq!(engaged(fov, luma), 0.0, "luma at fov {fov}");
            assert!(engaged(fov, chroma) > 0.0, "chroma at fov {fov}");
        }
        // And zoomed in, both.
        assert_eq!(engaged(20.0, luma), 1.0);
        assert_eq!(engaged(20.0, chroma), 1.0);
    }

    /// Four views that between them point the cap in every direction it can
    /// be pointed: down a lens axis, along the seam, out the back, and off
    /// both centre lines.
    fn cameras() -> [Camera; 4] {
        [
            Camera::default(),
            Camera {
                yaw: std::f32::consts::FRAC_PI_2,
                ..Camera::default()
            },
            Camera {
                yaw: std::f32::consts::PI,
                pitch: 0.3,
                ..Camera::default()
            },
            Camera {
                yaw: -0.7,
                pitch: -1.1,
                fov: 110f32.to_radians(),
            },
        ]
    }

    /// The blend as it was before issue #10: every lens projected, whatever
    /// the cap says about it.
    fn weighed_without_the_cap(reframe: &Reframe, ray: [f32; 3]) -> [f32; MAX_LENSES] {
        let landings: [Landing; MAX_LENSES] =
            std::array::from_fn(|lens| reframe.project(lens, ray));
        let front = reframe.handover(
            std::array::from_fn(|lens| reframe.axis_of(lens, ray)),
            norm3(ray),
            // No band and no reading, so the width is the floor, which is the
            // width this test was written against.
            reframe.handover_width(),
        );
        let mut weights: [f32; MAX_LENSES] =
            std::array::from_fn(|lens| match lens < reframe.lens_count as usize {
                true => claim(
                    landings[lens],
                    match lens {
                        0 => front,
                        _ => 1.0 - front,
                    },
                ),
                false => 0.0,
            });
        let total: f32 = weights.iter().sum();
        if total > 0.0 {
            for weight in &mut weights {
                *weight = share(*weight, total);
            }
        }
        weights
    }

    /// The widest angle off this lens's own axis, at this azimuth of its own
    /// frame, that the model still has a picture at. A hundredth of a degree
    /// at a time, which is finer than the spread being measured by a factor
    /// of five.
    fn support_edge(reframe: &Reframe, lens: usize, phi: f32) -> f32 {
        let block = &reframe.lenses[lens];
        let (sin_phi, cos_phi) = phi.to_radians().sin_cos();
        (0..18_000)
            .map(|step| step as f32 * 0.01)
            .take_while(|theta| {
                let (sin, cos) = theta.to_radians().sin_cos();
                lens_pixel(block, [sin * cos_phi, sin * sin_phi, cos]).inside
            })
            .last()
            .expect("the lens has no picture at all")
    }

    /// Issue #30's guard, per lens: the picture each lens contributes is one
    /// cap around its own axis, not a cap and a ghost. Swept from that lens's
    /// own axis to straight behind it, `inside` goes off once and stays off.
    /// On the radius test alone it came back on at 131.5 degrees and stayed on
    /// all the way to 180, and with two lenses that ghost prints over a
    /// picture the other lens is drawing correctly.
    #[test]
    fn each_lens_picture_stops_once() {
        let reframe = fixture(Camera::default());

        for lens in 0..MAX_LENSES {
            let mut edge = None;

            for step in 0..=1800 {
                let theta = step as f32 * 0.1;
                // Off lens 1's axis is the supplement of off lens 0's, so the
                // same sweep runs each lens from its own axis to its own fold.
                let ray = match lens {
                    0 => direction(theta, 0.0),
                    _ => direction(180.0 - theta, 0.0),
                };
                match (reframe.project(lens, ray).inside, edge) {
                    (false, None) => edge = Some(theta),
                    (true, Some(stopped)) => {
                        panic!("lens {lens} stopped at {stopped} degrees and came back at {theta}")
                    }
                    _ => {}
                }
            }

            // Where the model reaches the image circle, which is what decides
            // how much picture there is: 97.5 degrees on lens 0, as the note
            // on `image_radius` says, and a shade less on lens 1, whose
            // principal point sits further off centre.
            near(edge.expect("the picture never stopped"), 97.2, 0.7);
        }
    }

    /// Grab-the-world, end to end and in lens pixels: whatever the middle of
    /// the output was showing is a tenth of the way right of the middle after
    /// a drag that far, reading the same lens pixel it did before.
    ///
    /// The camera's own tests measure the solve in angles; this is the one
    /// that says the angles and the lens agree about which way is which, so
    /// the drag and the picture cannot drift apart.
    #[test]
    fn a_horizontal_drag_carries_the_content_with_the_cursor() {
        let camera = Camera::default();
        let before = fixture(camera);
        let anchor = shown(&before, ray(&before, [0.5, 0.5])).expect("grabbed nothing");

        let mut dragged = camera;
        dragged.aim(
            camera.look([0.5, 0.5], 1.0).expect("grabbed nothing"),
            [0.6, 0.5],
            1.0,
        );
        assert!(dragged.yaw < 0.0, "dragging right turns the view left");

        let after = fixture(dragged);
        let moved = shown(&after, ray(&after, [0.6, 0.5])).expect("dragged onto nothing");

        assert_eq!(moved.0, anchor.0);
        near(moved.1.pixel[0], anchor.1.pixel[0], 0.05);
        near(moved.1.pixel[1], anchor.1.pixel[1], 0.05);
    }

    /// The same for the vertical axis, which is the one whose sign is easy
    /// to get backwards: dragging down shows more sky.
    #[test]
    fn a_vertical_drag_carries_the_content_with_the_cursor() {
        let camera = Camera::default();
        let before = fixture(camera);
        let anchor = shown(&before, ray(&before, [0.5, 0.5])).expect("grabbed nothing");

        let mut dragged = camera;
        dragged.aim(
            camera.look([0.5, 0.5], 1.0).expect("grabbed nothing"),
            [0.5, 0.6],
            1.0,
        );
        assert!(dragged.pitch > 0.0, "dragging down looks up");

        let after = fixture(dragged);
        let moved = shown(&after, ray(&after, [0.5, 0.6])).expect("dragged onto nothing");

        assert_eq!(moved.0, anchor.0);
        near(moved.1.pixel[0], anchor.1.pixel[0], 0.05);
        near(moved.1.pixel[1], anchor.1.pixel[1], 0.05);
    }

    /// And the same on the pilot's body, where issue #29 was reported: a view
    /// pitched most of the way down, a grab well off the middle of the
    /// output, and the lens pixel under the cursor stays the lens pixel under
    /// the cursor.
    #[test]
    fn a_drag_near_the_nadir_carries_the_content_with_the_cursor() {
        // `fixture` builds its block at aspect 1, which is what `view_ray`
        // below reads: the solve has to be told the same thing.
        let aspect = 1.0;
        let (from, to) = ([0.62, 0.6], [0.45, 0.45]);
        let camera = Camera {
            yaw: 0.4,
            pitch: -60f32.to_radians(),
            ..Camera::default()
        };
        let before = fixture(camera);
        let anchor = shown(&before, ray(&before, from)).expect("grabbed a pixel no lens has");

        let mut dragged = camera;
        dragged.aim(
            camera.look(from, aspect).expect("grabbed nothing"),
            to,
            aspect,
        );

        let after = fixture(dragged);
        let moved = shown(&after, ray(&after, to)).expect("dragged onto nothing");

        assert_eq!(moved.0, anchor.0);
        near(moved.1.pixel[0], anchor.1.pixel[0], 1.0);
        near(moved.1.pixel[1], anchor.1.pixel[1], 1.0);
    }

    /// The datum, pinned: on a lens rolled a quarter turn, the top of the
    /// output lands above the principal point rather than beside it.
    /// Dropping `ROLL_DATUM_DEG` swaps those two, which is exactly the
    /// quarter turn the frames in docs/research/insv-format.md 4.8 ruled
    /// out.
    #[test]
    fn roll_is_measured_from_the_frames_horizontal_axis() {
        let reframe = fixture(Camera::default());
        let (lens, landing) =
            shown(&reframe, ray(&reframe, [0.5, 0.1])).expect("nothing at the top of the view");

        assert_eq!(lens, 0);
        assert!(landing.pixel[1] < 1927.21 - 100.0, "{landing:?}");
        near(landing.pixel[0], 1918.94, 40.0);
    }

    /// A one-stream file keeps every ray its lens has, out past 96 degrees
    /// off its axis on this fixture, which is 6 degrees past where the seam
    /// would be. (The rim itself is 96.9 to 97.4 depending on the azimuth,
    /// because the boundary is not a circle; this stays inside the nearest of
    /// it, since what is being checked is the band and not the cap.)
    ///
    /// The crossover of issue #48 is two lenses handing over to each other,
    /// and with nothing to hand over to it would have cut this picture off a
    /// degree past the seam and painted the rest grey. That band is the
    /// picture the older cameras deliver.
    #[test]
    fn one_stream_keeps_the_whole_of_its_picture() {
        let reframe = one_lens(Camera::default());

        for theta in (0..=965).step_by(5) {
            for phi in (0..360).step_by(20) {
                let ray = direction(theta as f32 * 0.1, phi as f32);
                let blend = reframe.blend(ray);
                assert_eq!(
                    blend.weights,
                    [1.0, 0.0],
                    "{} degrees off the axis at phi {phi}",
                    theta as f32 * 0.1,
                );
            }
        }
    }

    /// A file with one stream is the camera it was before: one hemisphere,
    /// and grey behind it. The older cameras write a lens per file, and
    /// nothing about them changed with issue #27 or #7.
    #[test]
    fn one_stream_still_renders_one_hemisphere() {
        let reframe = one_lens(Camera::default());

        let front = reframe.blend(ray(&reframe, [0.5, 0.5]));
        assert_eq!(front.weights, [1.0, 0.0]);

        let back = reframe.blend([0.0, 0.0, -1.0]);
        assert_eq!(back.weights, [0.0, 0.0]);
        assert!(!back.is_covered());
    }

    /// Issue #30: the ray straight out the back of one lens projects onto its
    /// principal point, which is as far inside the image circle as a pixel can
    /// get. Nothing but the domain test rejects it, and with two lenses the
    /// ghost it would draw is the other hemisphere's picture printed over this
    /// one.
    #[test]
    fn a_ray_from_straight_behind_a_lens_is_not_in_its_picture() {
        let reframe = fixture(Camera::default());

        for (lens, ray) in [(0, [0.0, 0.0, -1.0]), (1, [0.0, 0.0, 1.0])] {
            let landing = reframe.project(lens, ray);
            assert!(!landing.inside);
            assert!(radius(&reframe, lens, landing) < reframe.lenses[lens].image_radius);
            // The principal point itself, give or take the fifth of a degree
            // the lens is mounted off the body axis.
            near(radius(&reframe, lens, landing), 0.0, 15.0);
        }
    }

    /// Horizon lock, in lens pixels: a body rolled a quarter turn shows the
    /// same world direction at the same place in the output.
    ///
    /// This is the whole claim of issue #8 reduced to one number. The camera
    /// is left alone, the body is turned under it, and the pixel the middle
    /// of the output reads has to be the pixel a body-frame direction 90
    /// degrees round predicts, because with the lock on the view is in the
    /// world frame and the world did not move.
    #[test]
    fn a_rolled_body_shows_the_same_world_direction_in_the_same_place() {
        // Off both centre lines, so a roll about the view axis moves it in
        // both directions rather than sliding it along one.
        const OFF_AXIS: [f32; 2] = [0.3, 0.3];
        let camera = Camera::default();
        let level = fixture(camera);
        let anchor = shown(&level, ray(&level, OFF_AXIS)).expect("grabbed nothing");

        for roll in [10.0f64, -35.0, 90.0, 179.0] {
            // The body rolled about its own forward axis, which is what a
            // camera swinging under a wing does.
            let world_from_body = Quat::from_rotation_vector([0.0, 0.0, roll.to_radians()]);
            let rolled = held(
                camera,
                Held {
                    body_from_world: world_from_body.conjugate(),
                    ..Held::default()
                },
            );
            let moved = shown(&rolled, ray(&rolled, OFF_AXIS)).expect("rolled onto nothing");

            // The world direction is unchanged, so it lands in whichever lens
            // pixel that direction has always landed in: the body turned, so
            // that pixel moved, and this is the check that it moved by
            // exactly the roll.
            let turned = world_from_body
                .conjugate()
                .rotate(ray(&level, OFF_AXIS).map(f64::from))
                .map(|axis| axis as f32);
            let expected = shown(&level, turned).expect("the turned ray is in no lens");
            assert_eq!(moved.0, expected.0, "{roll} degrees");
            near(moved.1.pixel[0], expected.1.pixel[0], 0.05);
            near(moved.1.pixel[1], expected.1.pixel[1], 0.05);
            // And it is not a no-op: a rolled body really does read a
            // different pixel than a level one, by hundreds of pixels here.
            let apart = norm([
                moved.1.pixel[0] - anchor.1.pixel[0],
                moved.1.pixel[1] - anchor.1.pixel[1],
            ]);
            assert!(
                apart > 50.0,
                "{roll} degrees of roll moved the sample {apart} px"
            );
        }
    }

    /// And with the lock off nothing changed: identity is exactly the
    /// composition the pass had before issue #8, down to the bits.
    #[test]
    fn an_identity_hold_is_the_pass_as_it_was() {
        let camera = Camera {
            yaw: 0.7,
            pitch: -0.4,
            ..Camera::default()
        };
        let plain = fixture(camera);
        let identity = held(camera, Held::default());

        for lens in 0..MAX_LENSES {
            assert_eq!(
                plain.lenses[lens].view_to_lens,
                identity.lenses[lens].view_to_lens
            );
        }
    }

    /// The drag composes with the lock, which is the other half of issue #8's
    /// requirement: the grabbed content stays under the cursor while the
    /// horizon is being held.
    ///
    /// It needs no code of its own, and that is the finding. `Camera::look`
    /// answers in whatever frame `camera_rotation` lands in; the lock moves
    /// that frame from the body to the world, so the anchor is a world
    /// direction and the solve puts a world direction back under the cursor.
    /// The check is in lens pixels, because angles agreeing while pixels do
    /// not is exactly the bug a frame composition can have.
    #[test]
    fn a_drag_still_carries_the_content_while_the_horizon_is_held() {
        let aspect = 1.0;
        let (from, to) = ([0.62, 0.6], [0.38, 0.42]);
        let camera = Camera {
            yaw: 0.4,
            pitch: -0.3,
            ..Camera::default()
        };
        // A body that is neither level nor pointing where the view is.
        let hold = Held {
            body_from_world: Quat::from_rotation_vector([0.15, -0.4, 0.7]).conjugate(),
            ..Held::default()
        };

        let before = held(camera, hold);
        let anchor = shown(&before, ray(&before, from)).expect("grabbed a pixel no lens has");

        let mut dragged = camera;
        dragged.aim(
            camera.look(from, aspect).expect("grabbed nothing"),
            to,
            aspect,
        );
        assert_ne!(dragged, camera, "the drag moved nothing");

        let after = held(dragged, hold);
        let moved = shown(&after, ray(&after, to)).expect("dragged onto nothing");

        assert_eq!(moved.0, anchor.0);
        near(moved.1.pixel[0], anchor.1.pixel[0], 1.0);
        near(moved.1.pixel[1], anchor.1.pixel[1], 1.0);
    }

    /// One frame's readout with the body turning `turn` radians about the
    /// body's own axes across the whole of it, and the sensor read the way
    /// the X4 Air reads it.
    fn rolling(turn: [f64; 3]) -> Held {
        Held {
            rolling: Some(Rolling {
                turn,
                axis: Sweep::Right.axis(),
            }),
            ..Held::default()
        }
    }

    /// 90 deg/s, a brisk but ordinary roll, across the X4 Air's 15.883 ms
    /// readout: 1.43 degrees from the first row of the sensor to the last.
    ///
    /// `pub(crate)` because [`crate::twin`]'s fixture rolls at the same rate,
    /// and one number with one derivation is better than the same number
    /// written twice.
    pub(crate) const READOUT_TURN: f64 = 90.0 * 0.015_883 * std::f64::consts::PI / 180.0;

    /// The whole of issue #9 as one analytic prediction: a camera rolling
    /// about a lens's own axis smears that lens's picture round the axis, by
    /// the angle it turned through between the middle row of the sensor and
    /// the row a pixel sits on, and the correction takes exactly that out.
    ///
    /// Tangentially, because a roll about the optical axis is a rotation of
    /// the image about the principal point: the radius is untouched and the
    /// displacement is the radius times the angle. The prediction is the
    /// still landing turned about that point, compared in pixels, because
    /// pixels are what a smear is measured in.
    #[test]
    fn a_constant_roll_is_taken_out_by_the_row_the_ray_lands_on() {
        let camera = Camera::default();
        let still = fixture(camera);
        let turning = held(camera, rolling([0.0, 0.0, READOUT_TURN]));
        let mut worst = 0.0f32;

        for phi in (0..360).step_by(30) {
            let ray = direction(60.0, phi as f32);
            let (before, after) = (still.project(0, ray), turning.project(0, ray));
            // The row this ray really came off, which is the fixed point the
            // map solved for rather than the row the frame's instant implies.
            let share = f64::from(turning.readout_share(after.pixel));
            let expected = turned_about(&still, 0, before.pixel, READOUT_TURN * share);

            near(after.pixel[0], expected[0], 0.5);
            near(after.pixel[1], expected[1], 0.5);
            // And it is not a no-op: this much roll moves a sample by more
            // than the 12 to 18 px the format study predicts for handheld
            // motion, at the rows furthest from the middle of the readout.
            worst = worst.max(norm([
                after.pixel[0] - before.pixel[0],
                after.pixel[1] - before.pixel[1],
            ]));
        }
        assert!(worst > 8.0, "the whole roll moved a sample {worst} px");
    }

    /// The map is its own input, so what it answers has to satisfy itself:
    /// the row the solve landed on is the row whose instant the solve used.
    /// This is the residual [`READOUT_STEPS`] is chosen against, and it is
    /// checked at rates past anything this footage flies.
    #[test]
    fn the_solved_landing_is_the_landing_its_own_row_implies() {
        let camera = Camera::default();

        for rate in [90.0f64, 250.0, 523.0] {
            let turn = (rate * 0.015_883).to_radians();
            let reframe = held(camera, rolling([0.0, turn * 0.3, turn]));
            for phi in (0..360).step_by(45) {
                let ray = direction(60.0, phi as f32);
                let solved = reframe.project(0, ray);
                // One more round of the same solve, which is what a converged
                // answer does not move under.
                let block = &reframe.lenses[0];
                let share = reframe.readout_share(solved.pixel);
                let again = reframe.lens_pixel(
                    0,
                    normalize(turned(
                        block.lens_ray(ray),
                        block.turn.map(|axis| axis * share),
                    )),
                );
                let apart = norm([
                    again.pixel[0] - solved.pixel[0],
                    again.pixel[1] - solved.pixel[1],
                ]);
                assert!(apart < 2.5, "{rate} deg/s at {phi} moved {apart} px again");
            }
        }
    }

    /// The row-time mapping, per lens, where the answer is known: the sensor
    /// reads across the delivered frame, so a ray landing left of centre came
    /// off early and one landing right of it came off late.
    ///
    /// **And the two lenses read the same world direction at opposite ends of
    /// their own readouts**, because lens 1 is mounted a half turn round. That
    /// is why a readout displacement does not cancel at the seam but doubles
    /// there, which is issue #7's open question and 4.9's reason for it.
    #[test]
    fn the_two_lenses_read_a_seam_direction_at_opposite_ends_of_the_readout() {
        let reframe = held(Camera::default(), rolling([0.0; 3]));
        let share = |lens: usize, ray| reframe.readout_share(reframe.project(lens, ray).pixel);

        // Straight out the right of the body, which is on the seam circle and
        // in both pictures.
        near(share(0, [1.0, 0.0, 0.0]), 0.47, 0.03);
        near(share(1, [1.0, 0.0, 0.0]), -0.47, 0.03);

        for phi in (0..360).step_by(15) {
            let ray = direction(90.0, phi as f32);
            let (front, back) = (share(0, ray), share(1, ray));
            assert!(
                front * back <= 0.0,
                "the seam at {phi} degrees is read at {front} of lens 0's readout and {back} of \
                 lens 1's, which is the same end"
            );
        }
    }

    /// A file with no IMU record has nothing to correct with, and then the
    /// pass is what it was before issue #9: not nearly the same landing, the
    /// same landing.
    #[test]
    fn without_a_gyro_track_the_map_is_what_it_was() {
        let camera = Camera {
            yaw: 0.7,
            pitch: -0.4,
            ..Camera::default()
        };
        let reframe = held(camera, Held::default());

        assert!(!reframe.is_rolling());
        for lens in 0..MAX_LENSES {
            for phi in (0..360).step_by(45) {
                let ray = direction(70.0, phi as f32);
                assert_eq!(
                    reframe.project(lens, ray),
                    reframe.lens_pixel(lens, normalize(reframe.lenses[lens].lens_ray(ray))),
                );
            }
        }
    }

    /// A landing turned about its own lens's principal point, which is what a
    /// roll about that lens's axis does to the picture.
    fn turned_about(reframe: &Reframe, lens: usize, pixel: [f32; 2], angle: f64) -> [f32; 2] {
        let block = &reframe.lenses[lens];
        let (x, y) = (
            f64::from(pixel[0] - block.cx),
            f64::from(pixel[1] - block.cy),
        );
        let (sin, cos) = angle.sin_cos();
        [
            (x * cos - y * sin) as f32 + block.cx,
            (x * sin + y * cos) as f32 + block.cy,
        ]
    }

    /// A window shape to ask the wide questions at, and the one the player is
    /// used at.
    const WIDE: f32 = 2560.0 / 1440.0;

    /// Points of the output the projection is walked at: the middle, the
    /// edges, the corners and a scatter between them.
    fn places() -> Vec<[f32; 2]> {
        let along = [0.02, 0.19, 0.37, 0.5, 0.63, 0.81, 0.98];
        along.iter().flat_map(|&x| along.map(|y| [x, y])).collect()
    }

    fn screen(fov_deg: f32, aspect: f32) -> Screen {
        Screen::new(
            Camera {
                fov: fov_deg.to_radians(),
                ..Camera::default()
            },
            aspect,
        )
    }

    /// How far off the view axis a point of the output looks, in radians.
    fn off_axis(screen: Screen, uv: [f32; 2]) -> Option<f32> {
        Some(normalize(screen.ray(uv)?)[2].clamp(-1.0, 1.0).acos())
    }

    /// Under the threshold the map is the flat window it always was, and not
    /// a bent one that happens to agree: same two multiplies, same
    /// unnormalized ray, no trig anywhere near it (issue #47).
    #[test]
    fn the_flat_range_is_the_map_it_always_was() {
        for fov_deg in [20.0, 45.0, 90.0, 109.9, 110.0] {
            for aspect in [0.6, 1.0, WIDE] {
                let screen = screen(fov_deg, aspect);
                assert_eq!(screen.shrink, 1.0);
                assert_eq!(screen.ball_radius, f32::MAX);
                let tan_half_fov = (fov_deg.to_radians() * 0.5).tan();
                for uv in places() {
                    assert_eq!(
                        screen.ray(uv),
                        Some([
                            (uv[0] * 2.0 - 1.0) * tan_half_fov,
                            (uv[1] * 2.0 - 1.0) * tan_half_fov / aspect,
                            1.0,
                        ]),
                    );
                }
            }
        }
    }

    /// Twice the threshold is a `shrink` of exactly a half, which is exactly
    /// stereographic: the plane radius is `2 tan(theta / 2)`, the tiny
    /// planet's own map, and it arrives without being written down anywhere.
    #[test]
    fn the_bend_passes_through_stereographic() {
        let screen = screen(2.0 * FOV_FLAT.to_degrees(), WIDE);
        assert_eq!(screen.shrink, 0.5);

        for uv in places() {
            let theta = off_axis(screen, uv).expect("stereographic fills its frame");
            let plane = [
                (uv[0] * 2.0 - 1.0) * screen.half_extent,
                (uv[1] * 2.0 - 1.0) * screen.half_extent / screen.aspect,
            ];
            near(norm(plane), 2.0 * (theta * 0.5).tan(), 1e-4);
        }
    }

    /// Zooming out only ever zooms out. Every point of the frame looks
    /// further off the axis as the field of view widens, all the way from the
    /// narrowest view to the ball, and a point that has run off the sphere
    /// does not come back.
    ///
    /// This is the whole of why the schedule is what it is. The bend and the
    /// widening pull the picture opposite ways -- a wider view spreads the
    /// world out, a harder bend pulls it in -- and a schedule that got the
    /// balance wrong would hand back a scroll that reverses in the middle.
    #[test]
    fn the_picture_only_ever_shrinks() {
        for aspect in [0.6, 1.0, WIDE] {
            let ceiling = fov_ceiling(aspect).to_degrees();
            let steps = 400;
            for uv in places() {
                let mut held: Option<f32> = None;
                let mut gone = false;
                for step in 0..=steps {
                    let fov =
                        FOV_MIN_DEG * (ceiling / FOV_MIN_DEG).powf(step as f32 / steps as f32);
                    let screen = screen(fov, aspect);
                    match off_axis(screen, uv) {
                        Some(theta) => {
                            assert!(!gone, "{uv:?} came back onto the sphere at fov {fov:.1}");
                            if let Some(held) = held {
                                assert!(
                                    theta >= held - 1e-5,
                                    "{uv:?} looked back in from {held} to {theta} at fov {fov:.1}",
                                );
                            }
                            held = Some(theta);
                        }
                        None => gone = true,
                    }
                }
            }
        }
    }

    /// The narrowest view, in degrees: `camera::FOV_MIN`, which this file
    /// cannot see and does not own.
    const FOV_MIN_DEG: f32 = 20.0;

    /// The bend starts without a step in it, which is issue #47's own bar:
    /// one continuous scroll, no pop where the projection changes.
    ///
    /// Continuity is asked the way it is defined rather than by eye. A scroll
    /// of `step` across the threshold moves the picture by some angle; halve
    /// the step and a continuous map halves the angle, while a map that
    /// jumped would keep the jump however small the step got. Then the rate
    /// itself: the same tiny scroll one side of the threshold and the other
    /// moves the picture by within a percent of the same amount, so the zoom
    /// does not change gear as it crosses.
    #[test]
    fn the_bend_starts_without_a_step() {
        let moved = |from: f32, step: f32| {
            let (before, after) = (screen(from / step, WIDE), screen(from * step, WIDE));
            places()
                .iter()
                .filter_map(|&uv| {
                    let (a, b) = (before.ray(uv)?, after.ray(uv)?);
                    Some(angle_between(normalize(a), normalize(b)))
                })
                .fold(0.0, f32::max)
        };

        let flat = FOV_FLAT.to_degrees();
        let mut halving: Option<f32> = None;
        for step in [1.04_f32, 1.02, 1.01, 1.005, 1.0025] {
            let jump = moved(flat, step.sqrt());
            if let Some(coarser) = halving {
                assert!(
                    jump < 0.55 * coarser,
                    "halving the scroll left {jump} rad of the {coarser} rad before it, which \
                     is a step in the map rather than a walk through it",
                );
            }
            halving = Some(jump);
        }

        // A hundredth of the field of view, which is about a twelfth of a
        // scroll notch: finer than a wheel can ask for and coarse enough to
        // read.
        let step = 1.01_f32.sqrt();
        let below = moved(flat / 1.005, step);
        let above = moved(flat * 1.005, step);
        near(above / below, 1.0, 0.05);
    }

    fn angle_between(a: [f32; 3], b: [f32; 3]) -> f32 {
        let crossed = cross(a, b);
        norm3(crossed).atan2(dot(a, b))
    }

    /// The far end of the zoom, which is the whole point of issue #47: the
    /// ball sits inside the frame, round, centred, with room around it, and
    /// the room is every direction no lens has, which is what the pass leaves
    /// transparent (issue #100).
    #[test]
    fn the_ball_sits_in_the_frame_with_room_around_it() {
        for aspect in [0.6, 1.0, WIDE] {
            let screen = Screen::new(
                Camera {
                    fov: fov_ceiling(aspect),
                    ..Camera::default()
                },
                aspect,
            );
            // The ball fills `BALL_FILL` of the frame's shorter side, so in
            // that side's own uv its rim is this far from the middle; the
            // longer side holds the same radius in fewer of its own units.
            let rim = 0.5 * BALL_FILL;
            let toward = [(1.0 / aspect).min(1.0), aspect.min(1.0)];
            for axis in 0..2 {
                let edge = |at: f32| {
                    let mut uv = [0.5, 0.5];
                    uv[axis] += at * toward[axis];
                    uv
                };
                assert!(
                    screen.ray(edge(rim * 0.98)).is_some(),
                    "the ball is smaller than {BALL_FILL} of the frame at aspect {aspect}",
                );
                assert!(
                    screen.ray(edge(rim * 1.02)).is_none(),
                    "the ball is larger than {BALL_FILL} of the frame at aspect {aspect}",
                );
            }
            assert!(screen.ray([0.02, 0.02]).is_none(), "no room in the corner");
        }
    }

    /// A ball view holds the whole sphere at once, which is the first time
    /// one pass has had to: every pixel of the ball is picture, the seam
    /// blend still sums to one across it, and the far side of each lens is
    /// carried by the other one rather than by the fold the model would
    /// otherwise land there (issue #30's guard, now on the hot path).
    #[test]
    fn every_pixel_of_the_ball_is_picture() {
        let camera = Camera {
            fov: fov_ceiling(WIDE),
            ..Camera::default()
        };
        let reframe = Reframe::new(
            &fixture_lenses(),
            FRAME,
            camera,
            Held::default(),
            WIDE,
            false,
            Sampling::default(),
        );
        let (mut lit, mut room, mut furthest) = (0, 0, 0.0f32);

        for down in 0..=120 {
            for across in 0..=120 {
                let uv = [across as f32 / 120.0, down as f32 / 120.0];
                let Some(ray) = reframe.view_ray(uv) else {
                    room += 1;
                    continue;
                };
                lit += 1;
                let blend = reframe.blend(ray);
                assert!(blend.is_covered(), "no lens has {uv:?} of the ball");
                near(blend.weights.iter().sum(), 1.0, 1e-5);
                for lens in 0..MAX_LENSES {
                    let landing = blend.landings[lens];
                    assert!(
                        blend.weights[lens] == 0.0 || landing.inside,
                        "the ball is showing a folded landing at {uv:?}",
                    );
                }
                furthest = furthest.max(normalize(ray)[2].clamp(-1.0, 1.0).acos());
            }
        }

        // The ball is round and the frame is not, so a wide window is
        // mostly room: at 16:9 the ball is an ellipse of 0.225 by 0.4 of the
        // frame, which is 28% of it.
        assert!(lit > 3_500 && room > 9_000, "{lit} lit and {room} room");
        near(furthest.to_degrees(), 180.0, 1.0);
    }

    /// The size the WGSL struct rounds up to, which is what the bind group
    /// declares as `min_binding_size`: pipeline creation is where a
    /// disagreement between the two definitions surfaces.
    #[test]
    fn the_uniform_block_is_the_size_wgsl_lays_it_out() {
        assert_eq!(std::mem::size_of::<LensBlock>(), 112);
        assert_eq!(std::mem::size_of::<Screen>(), 16);
        // 288 before the band's two fields, which add a padded mat3x3 and a
        // padded vec3 (issue #103), and the table's own lane per four
        // directions after them (stage 9).
        let table = super::super::band::AZIMUTHS / 4 * 16;
        assert_eq!(std::mem::size_of::<super::super::band::Table>(), table);
        assert_eq!(std::mem::size_of::<Reframe>(), 288 + 48 + 16 + table);
        // The offset, not arithmetic that cannot fail: WGSL starts the table
        // at a multiple of sixteen and `repr(C)` does not have to, and
        // `min_binding_size` checks the block's size rather than any offset
        // inside it, so a table that slid twelve bytes would draw a wrong
        // picture rather than refuse a pipeline.
        assert_eq!(std::mem::offset_of!(Reframe, table) % 16, 0);
        assert_eq!(std::mem::offset_of!(Reframe, table), 288 + 48 + 16);
        // The seam anchor's one number took the FIRST of the three padding
        // words the table's alignment already needed, rather than being
        // appended: the block is the size it was and the table has not moved,
        // which is what the two assertions above would otherwise have to be
        // rewritten to say.
        assert_eq!(
            std::mem::offset_of!(Reframe, handover_shift),
            std::mem::offset_of!(Reframe, crossover) + 4
        );
        assert_eq!(std::mem::size_of_val(&Reframe::blank(1.0, false)._pad), 8);
    }

    /// **The anchor's null.** A map nobody has held a line on draws the
    /// picture the geometry draws, and the term that carries the anchor is a
    /// literal zero.
    ///
    /// This is what `KJERAG_ANCHOR=off` gets, what every instrument gets, what
    /// every test above gets, and what the blank pane gets. It matters because
    /// the offset is an ADDED term inside [`crossover`] rather than a factor
    /// folded into the quotient beside it: at zero the arithmetic is the
    /// arithmetic that was there before the anchor existed, bit for bit, and
    /// the whole mechanism is provably absent from every picture that does not
    /// ask for it.
    #[test]
    fn a_map_with_no_line_held_on_it_draws_the_geometry() {
        let reframe = fixture(Camera::default());
        assert_eq!(reframe.handover_shift, 0.0);
        assert_eq!(Reframe::blank(1.0, false).handover_shift, 0.0);
        // `with_shift(0.0)` is the identity on the whole block, so a caller
        // that runs the follow and is handed a zero is the caller that does
        // not run it.
        assert_eq!(reframe.with_shift(0.0).bytes(), reframe.bytes());
        // And the shift is a real term rather than a decoration: it puts the
        // 50/50 line exactly where a ray whose `across_seam` is minus the
        // shift sits, which is the inverse [`SeamAnchor::hold`] relies on.
        // Read off the arithmetic and not off a rendered weight, because a
        // delivered weight is the share times each lens's own coverage depth
        // and renormalized after ([`claim`]), so the crossing of the WEIGHTS
        // is a different question from where the handover put its line.
        let band = reframe.handover_width();
        let reach = 1.0f32;
        for asked_deg in [-2.0f32, -0.4, 0.4, 2.0] {
            let shift = asked_deg.to_radians();
            // A ray at `across_seam = -shift` has `apart = 2 * reach * -shift`.
            let apart = 2.0 * reach * -shift;
            near(crossover(apart, reach, band, shift), 0.5, 1e-6);
            // And it is the same line the geometry alone would have put at
            // that offset, moved by exactly the shift: nothing else in the
            // ramp changed.
            near(
                crossover(apart, reach, band, shift),
                crossover(0.0, reach, band, 0.0),
                1e-6,
            );
        }
    }

    /// **THE UPDATE LAW, and the four properties the owner's ruling is.**
    /// Checked on the arithmetic itself rather than on a picture.
    ///
    /// *"Every now and then it glitches. We need it to be smooth, that is a
    /// requirement."* A glitch is a velocity that changes in one frame, and
    /// the three ways one frame's arithmetic can do that are a jump, an
    /// overshoot and a threshold. None of them is reachable here, and this is
    /// why.
    #[test]
    fn the_follow_is_smooth_and_cannot_overshoot() {
        let allowance = 4f32.to_radians();
        let step = 1.0 / 30.0;

        // A redraw with no new frame behind it is exactly the identity, so the
        // follow is a length of film and not a count of redraws, and running
        // it twice on one frame is running it once.
        for degrees in [-4.0, -1.0, 0.0, 0.7, 3.9, 12.0] {
            let target = (degrees as f32).to_radians();
            assert_eq!(SeamAnchor::follow(target, 0.0, allowance).0, target);
        }

        // It never overshoots and never changes sign, whatever the step: the
        // answer is always between the target and zero. A whole second of film
        // charged at once is well past anything a redraw can deliver.
        for degrees in [-30.0, -4.0, -2.5, -0.1, 0.1, 2.5, 4.0, 30.0] {
            let target = (degrees as f32).to_radians();
            for step in [0.001, 1.0 / 60.0, step, 0.25, 1.0] {
                let (delta, _) = SeamAnchor::follow(target, step, allowance);
                assert!(
                    delta.abs() <= target.abs() && delta.signum() == target.signum(),
                    "{degrees} deg over {step} s left the line at {} deg",
                    delta.to_degrees(),
                );
            }
        }

        // Standing still costs the line nothing. At a quarter of the allowance
        // - twice the shake this corpus puts on a parked airframe - one frame
        // moves the drawn line by under a ten-thousandth of a degree.
        let (delta, _) = SeamAnchor::follow(1f32.to_radians(), step, allowance);
        assert!(
            (1.0 - delta.to_degrees()) < 1e-4,
            "a still camera moved the line by {} deg in one frame",
            1.0 - delta.to_degrees(),
        );

        // And there is no knee to click on: over the whole range the drawn
        // offset is a monotone, smooth function of the target, so a first
        // difference of it can never change abruptly. Sampled finely, no
        // second difference is more than a hundredth of a degree.
        let sample = |i: i32| {
            SeamAnchor::follow((i as f32 * 0.01).to_radians(), step, allowance)
                .0
                .to_degrees()
        };
        let mut roughest = 0.0f32;
        for i in -600..600 {
            let (back, here, next) = (sample(i - 1), sample(i), sample(i + 1));
            assert!(next >= here, "the follow is not monotone at {i}");
            roughest = roughest.max((next - 2.0 * here + back).abs());
        }
        assert!(
            roughest < 0.01,
            "the follow has a knee worth {roughest} in it"
        );
    }

    /// The clamp is a guard and not the mechanism: the line stays inside the
    /// fade it lives in, however hard the geometry pulls.
    #[test]
    fn the_held_line_stays_inside_the_fade() {
        let reframe = fixture(Camera::default());
        let allowance = 0.5 * reframe.handover_width();
        for asked in [-10.0f32, -0.1, 0.0, 0.1, 10.0] {
            let shift = reframe.with_shift(asked.to_radians()).handover_shift;
            assert!(
                shift.abs() <= allowance + 1e-9,
                "a shift of {asked} deg put the line {} deg out of a {} deg allowance",
                shift.to_degrees(),
                allowance.to_degrees(),
            );
        }
        // And a sustained drift approaches the allowance without reaching it,
        // as an eleventh root of the drift rate: 25 times the drift buys about
        // a third more offset, which is why no aircraft manoeuvre makes the
        // clamp above the thing that decides where the line is drawn.
        let settled = |drift_dps: f32| {
            let mut delta = 0.0f32;
            for _ in 0..600 {
                let target = delta + drift_dps.to_radians() / 30.0;
                delta = SeamAnchor::follow(target, 1.0 / 30.0, allowance).0;
            }
            delta.to_degrees()
        };
        let (slow, fast) = (settled(1.0), settled(25.0));
        assert!(slow < fast, "a faster drift held less offset");
        assert!(
            fast < allowance.to_degrees(),
            "a 25 deg/s drift railed the line at {fast}",
        );
        assert!(
            fast < 1.4 * slow,
            "25 times the drift bought {:.2}x the offset, not the eleventh root",
            fast / slow,
        );
    }

    /// **The follow has a ceiling, and the film's frame rate is what sets
    /// it.** The characteristic behind `the_held_line_stays_inside_the_fade`'s
    /// "approaches and does not reach", written down rather than left implied.
    ///
    /// The eleventh root the law is described by is the continuum answer, and
    /// the law is charged once per frame: the geometry's sweep arrives as a
    /// jump and the leak is then read at a gain taken AFTER that jump, which
    /// over-charges the leak by more the coarser the step. Letting the drift
    /// run away isolates it, because the target then dominates and the answer
    /// stops depending on the drift at all:
    ///
    /// ```text
    /// delta -> allowance / (POWER * RATE * dt)^(1 / POWER)
    /// ```
    ///
    /// **Why it is worth a test of its own.** `POWER * RATE` is 100 per second,
    /// so the ceiling equals the allowance at exactly 100 fps, and both cameras
    /// in the corpus shoot 120 fps modes. Under 100 fps the follow is what
    /// decides where the line sits and `Reframe::with_shift`'s clamp is
    /// unreachable; over it the clamp is the rail. That is the whole reason
    /// [`SeamAnchor::hold`] clamps before it places its anchor, and this test
    /// is what says the 30 fps side of the line is the side every frame the
    /// owner has judged sits on.
    #[test]
    fn the_follow_has_a_ceiling_and_the_frame_rate_sets_it() {
        let allowance = 4f32.to_radians();
        let power = ANCHOR_FOLLOW_POWER as f32;
        // The table in `SeamAnchor::follow`'s doc, and the closed form it is
        // read off, checked against the law itself at a drift it can never
        // catch up with.
        for (fps, ceiling) in [
            (24.0f32, 3.47f32),
            (30.0, 3.55),
            (60.0, 3.80),
            (100.0, 4.00),
            (120.0, 4.07),
            (240.0, 4.37),
        ] {
            let step = 1.0 / fps;
            let closed_form =
                allowance.to_degrees() / (power * ANCHOR_FOLLOW_RATE * step).powf(1.0 / power);
            near(closed_form, ceiling, 0.005);
            // The law itself, asked for an offset a hundred times the
            // allowance: what comes back is the ceiling and not the ask.
            let reached = SeamAnchor::follow(100.0 * allowance, step, allowance)
                .0
                .to_degrees();
            near(reached, ceiling, 0.01);
            // And a runaway drift settles there rather than climbing past it.
            let mut delta = 0.0f32;
            for _ in 0..400 {
                delta = SeamAnchor::follow(delta + 40f32.to_radians(), step, allowance).0;
            }
            assert!(
                delta.to_degrees() <= ceiling + 0.01,
                "at {fps} fps a runaway drift held {} deg against a ceiling of {ceiling}",
                delta.to_degrees(),
            );
        }
        // The consequence, stated as the inequality it is: film's own rate
        // leaves the map's clamp unreachable, and a 120 fps mode does not.
        let ceiling = |fps: f32| {
            allowance.to_degrees() / (power * ANCHOR_FOLLOW_RATE / fps).powf(1.0 / power)
        };
        assert!(
            ceiling(30.0) < allowance.to_degrees(),
            "the 30 fps ceiling reaches the allowance, so the arm the owner approved could rail",
        );
        assert!(
            ceiling(120.0) > allowance.to_degrees(),
            "the 120 fps ceiling is under the allowance, so the clamp in `hold` guards nothing",
        );
        // The crossing is exactly where `POWER * RATE * dt` is one, which is
        // the only place the root can be.
        near(
            ceiling(power * ANCHOR_FOLLOW_RATE),
            allowance.to_degrees(),
            1e-4,
        );
    }

    /// **The anchor records the line the picture actually draws.**
    ///
    /// [`Reframe::with_shift`] clamps on the way to the shader, so an offset
    /// past the allowance is DRAWN at the allowance. Until 2026-08-09
    /// [`SeamAnchor::hold`] placed its world anchor on the unclamped value, so
    /// whenever the clamp fired the state said the line stood somewhere the
    /// picture had not put it, and the next redraw's target was read off that
    /// fiction. It is a standing bias and not a transient: the error is re-made
    /// every frame the clamp fires.
    ///
    /// The read-back is `hold` itself at a step of zero, where the law is the
    /// identity, so what comes back as the target is precisely where the state
    /// says the line is standing. Under the fix it is the drawn offset.
    ///
    /// **The control is the frame rate.** At 30 fps the follow's own ceiling is
    /// 3.55 of the 4.00 degrees allowed, so the clamp cannot fire and this test
    /// would pass on the broken code as well - which is exactly why the arm the
    /// owner approved is byte-identical either way. The step here is a 240 fps
    /// one, where the ceiling is 4.36, and the first assertion is that the
    /// unclamped law really does overshoot at it.
    #[test]
    fn the_held_line_is_placed_where_the_picture_draws_it() {
        let reframe = fixture(Camera::default());
        let allowance = 0.5 * reframe.handover_width();
        let step = 1.0f32 / 240.0;

        // The control: at this step the law itself goes past the allowance, so
        // there is something for the clamp to catch.
        let raw = SeamAnchor::follow(100.0 * allowance, step, allowance).0;
        assert!(
            raw > allowance,
            "the follow stops at {} deg of a {} deg allowance on its own, so this test is empty",
            raw.to_degrees(),
            allowance.to_degrees(),
        );

        let was = standing(&reframe, 30.0, 10.0);
        let anchor = SeamAnchor::hold(Some(was), &reframe, Held::default(), 10.0 + f64::from(step));
        assert!(
            anchor.shift().abs() <= allowance,
            "the anchor held {} deg of a {} deg allowance",
            anchor.shift().to_degrees(),
            allowance.to_degrees(),
        );
        // The shader is handed what the anchor says, untouched: the map's own
        // clamp is a guard behind this one and never a second opinion.
        assert_eq!(
            reframe.with_shift(anchor.shift()).handover_shift,
            anchor.shift(),
        );
        // And the world anchor stands on the drawn line: read back through the
        // same pose at a step of nothing, the target IS the drawn offset.
        let again = SeamAnchor::hold(Some(anchor), &reframe, Held::default(), anchor.at);
        near(again.target, anchor.shift(), 1e-6);
        near(again.shift(), anchor.shift(), 1e-6);
    }

    /// **A seek starts the hold again, whichever way the film jumped.**
    ///
    /// The step used to be `(at - was.at).clamp(0.0, CAP)`, so a backward seek
    /// came out as a step of zero - and a step of zero is the identity, which
    /// means the line was pinned to whatever the target said on that frame. The
    /// target after a seek is `was.on` read through a pose from a different
    /// part of the flight and can be a quadrant wide, so the line slammed to
    /// the rail on the seek frame and walked back over the next second: a lurch
    /// laid over the one frame where every pixel already changed.
    ///
    /// **The forward half of that was still there until 2026-08-09**, and this
    /// test now carries it. A forward seek took the ordinary arm with the step
    /// capped, which charges the follow with the same stale target - and the
    /// size of that is a closed form rather than an accident. The follow's own
    /// ceiling at a step of `dt` is `allowance / (POWER * RATE * dt)^(1/POWER)`
    /// (see [`Self::follow`] and the frame-rate rail), so at the capped step of
    /// 0.25 s **every** forward seek, of any length, drew the line **2.90 of
    /// the 4.00 degrees** on the seek frame and then walked it back: 72 percent
    /// of the whole allowance, laid over the one frame where the picture had
    /// already changed. That is the identical defect the backward arm was
    /// fixed for, and the argument it was given - a seek is already a
    /// discontinuity in every pixel - never said which way the clock had moved.
    /// So what separates a seek from a redraw is the SIZE of the step and not
    /// its sign ([`ANCHOR_SEEK_SECS`]).
    ///
    /// The four cases, and the middle two are the fix:
    ///
    /// - film moving on: the follow runs, and the line is where the law puts
    ///   it;
    /// - film going backwards: the anchor is placed afresh on the geometry,
    ///   which is where the line would be if the file had been opened there;
    /// - **film jumping forward past [`ANCHOR_SEEK_SECS`]**: the same;
    /// - **the same instant twice**: NOT a seek. It is a redraw with no new
    ///   frame behind it, it stays in the second arm, and the law at a step of
    ///   zero is the identity, which is the property that lets a 60 Hz window
    ///   and an offscreen instrument at one draw per frame hold the same line.
    #[test]
    fn a_seek_starts_the_hold_again_whichever_way_the_film_jumped() {
        let reframe = fixture(Camera::default());
        let was = standing(&reframe, 3.0, 10.0);
        let held = |at| SeamAnchor::hold(Some(was), &reframe, Held::default(), at);

        let onward = held(10.0 + 1.0 / 30.0);
        assert!(
            onward.shift().to_degrees() > 2.0,
            "the follow gave up {} of a 3.00 degree hold in one frame",
            onward.shift().to_degrees(),
        );

        // Backward, which is what the first half of this fix caught.
        let back = held(4.0);
        assert_eq!(
            back.shift(),
            0.0,
            "a seek back to 4.0 s left the line {} deg off the geometry",
            back.shift().to_degrees(),
        );
        assert_eq!(back.target, 0.0);

        // Forward, which is what the second half of it caught. A seek moves
        // the body as well as the clock, so the frame it lands on is held at a
        // pose from another part of the flight, and `was.on` read back through
        // THAT pose is the target the old code charged the follow with.
        let elsewhere = Held {
            body_from_world: Quat::from_rotation_vector([0.0, 0.9, 0.0]).conjugate(),
            ..Held::default()
        };
        let forward = SeamAnchor::hold(Some(was), &reframe, elsewhere, 16.0);
        assert_eq!(
            forward.shift(),
            0.0,
            "a seek on to 16.0 s left the line {} deg off the geometry",
            forward.shift().to_degrees(),
        );
        assert_eq!(forward.target, 0.0);

        // And what that was worth, by running the arithmetic the old arm ran:
        // the stale target at the step it capped to. The answer is the
        // follow's own ceiling at that step - `allowance / (POWER * RATE *
        // dt)^(1 / POWER)`, which is 2.90 of these 4.00 degrees at dt = 0.25 -
        // so it is the same 2.90 for a forward seek of any length, and it is
        // 72 percent of the whole allowance delivered on one frame.
        let allowance = 0.5 * reframe.handover_width();
        let stale = -reframe.across_seam(
            reframe.view_ray_from_body(
                elsewhere
                    .body_from_world
                    .rotate(was.on)
                    .map(|axis| axis as f32),
            ),
        );
        assert!(
            stale.abs() > allowance,
            "the stale target is only {} deg, so this is not the case the fix is about",
            stale.to_degrees(),
        );
        let drawn = SeamAnchor::follow(stale, ANCHOR_SEEK_SECS as f32, allowance)
            .0
            .clamp(-allowance, allowance);
        near(drawn.to_degrees(), 2.90, 0.01);
        near(
            drawn.to_degrees(),
            (allowance
                / (f32::from(ANCHOR_FOLLOW_POWER as i16)
                    * ANCHOR_FOLLOW_RATE
                    * ANCHOR_SEEK_SECS as f32)
                    .powf(1.0 / ANCHOR_FOLLOW_POWER as f32))
            .to_degrees(),
            0.01,
        );

        // The two boundaries of the arm, so the constant is pinned and not
        // decorative: a step of exactly `ANCHOR_SEEK_SECS` still follows, and
        // anything past it does not.
        assert_ne!(held(10.0 + ANCHOR_SEEK_SECS).target, 0.0);
        assert_eq!(held(10.0 + ANCHOR_SEEK_SECS + 1e-6).target, 0.0);

        // And the same instant twice is not a seek in either direction.
        let redrawn = held(10.0);
        near(redrawn.shift(), was.delta, 1e-6);
    }

    /// **What the held line delivers is a fraction of what it commands, and the
    /// fraction is the camera's.** The honesty this PR's own prose needed
    /// (2026-08-09 review).
    ///
    /// The anchor holds the SHARE's 50/50 line: [`crossover`] is a ramp in
    /// `across_seam - shift`, so the whole share profile translates rigidly by
    /// the shift and the anchor's arithmetic is exact about it. **The picture
    /// draws the WEIGHTS' crossing**, which is that share times each lens's own
    /// coverage depth, renormalized ([`claim`]) - and the depths are fixed to
    /// the lenses and do not translate. The crossing of the delivered weights
    /// therefore moves by less than the shift, and the shortfall is the
    /// camera's own overlap against the band it hands over on: a linear taper
    /// predicts `overlap / (overlap + band)`, which is an upper bound the real
    /// taper does not reach.
    ///
    /// **Measured here, mean over 24 azimuths, 2026-08-09:**
    ///
    /// | | X4 Air fixture | X2-class |
    /// | --- | ---: | ---: |
    /// | overlap / band, deg | 14.44 / 8.00 | 9.18 / 8.00 |
    /// | `overlap / (overlap + band)` | 0.643 | 0.534 |
    /// | **delivered per commanded degree** | **0.617** | **0.510** |
    /// | the same, spread over the 24 azimuths | 0.610 to 0.624 | 0.499 to 0.522 |
    /// | drawn offset at a 4.00 degree hold, deg | 2.54 | 2.13 |
    ///
    /// So the anchor removes about **62 percent** of the seam's crawl on the
    /// camera the owner judged it on and about **51 percent** on the narrowest
    /// one, not all of it. flat6 behaved identically - this is a property of
    /// the fusion the owner approved and not of anything this merge changed -
    /// and it is recorded in docs/research/studio-parity.md 6 and in the PR's
    /// accepted tradeoffs rather than fixed here.
    ///
    /// The zero-shift crossing is not exactly on the seam either (0.07 degrees
    /// on the fixture), because the two lenses are not the same lens: their
    /// depths differ slightly at the seam. The gain is read as a SLOPE across
    /// two shifts so that this standing offset cancels out of it.
    #[test]
    fn the_held_line_delivers_a_fraction_of_the_hold_it_commands() {
        for (name, reframe, predicted, gain, drawn) in [
            (
                "the X4 Air fixture",
                fixture(Camera::default()),
                0.643f32,
                0.617f32,
                2.54f32,
            ),
            ("an X2-class camera", cropped(X2_CLASS), 0.534, 0.510, 2.13),
        ] {
            let overlap = reframe.overlap().expect("two lenses").to_degrees();
            let band = reframe.handover_width().to_degrees();
            near(overlap / (overlap + band), predicted, 0.002);

            let allowance = 0.5 * band;
            let (mut mean, mut lowest, mut highest) = (0.0f32, f32::INFINITY, 0.0f32);
            let mut rail = 0.0f32;
            let azimuths = (0..360).step_by(15);
            let count = azimuths.clone().count() as f32;
            for phi in azimuths {
                let phi = phi as f32;
                let still = crossing(&reframe.with_shift(0.0), phi);
                let held = crossing(&reframe.with_shift(allowance.to_radians()), phi);
                let slope = (held - still) / allowance;
                mean += slope / count;
                lowest = lowest.min(slope);
                highest = highest.max(slope);
                rail += held / count;
            }
            assert!(
                (mean - gain).abs() < 0.01,
                "{name} delivers {mean:.4} of every degree it is asked to hold, not {gain}",
            );
            assert!(
                highest - lowest < 0.03,
                "{name}'s gain runs {lowest:.4} to {highest:.4} round the ring, so a mean of it \
                 says nothing",
            );
            assert!(
                (rail - drawn).abs() < 0.02,
                "{name} draws its line {rail:.4} deg off the seam at the {allowance:.2} degree \
                 rail, not {drawn}",
            );
            // And the finding, as an inequality: the delivered line moves by
            // materially less than the line the anchor is holding, so some of
            // the crawl survives the hold.
            assert!(
                mean < 0.7,
                "{name} delivers {mean:.4} of the hold, which is close enough to all of it that \
                 the disclosure this test exists for would be wrong",
            );
            assert!(
                mean < overlap / (overlap + band),
                "{name} delivers more than the linear-taper bound, which cannot happen",
            );
        }
    }

    /// Where the DELIVERED weights cross, in degrees past the seam, at one
    /// azimuth. Bisected rather than swept: the two weights are monotone
    /// against each other across the handover, and a sweep fine enough to place
    /// the crossing to a thousandth is a hundred times the work.
    fn crossing(reframe: &Reframe, phi: f32) -> f32 {
        let apart = |theta: f32| {
            let weights = reframe.blend(direction(theta, phi)).weights;
            weights[0] - weights[1]
        };
        let (mut lens_zero, mut lens_one) = (80.0f32, 100.0f32);
        assert!(
            apart(lens_zero) > 0.0 && apart(lens_one) < 0.0,
            "the handover does not run from lens 0 to lens 1 across {lens_zero} to {lens_one}",
        );
        for _ in 0..40 {
            let middle = 0.5 * (lens_zero + lens_one);
            match apart(middle) > 0.0 {
                true => lens_zero = middle,
                false => lens_one = middle,
            }
        }
        0.5 * (lens_zero + lens_one) - 90.0
    }

    /// A state that says the drawn line is standing `offset_deg` off the seam
    /// at `at` seconds, on the piece of world content it would be standing on.
    ///
    /// [`SeamAnchor::hold`] places its own anchor exactly this way
    /// ([`Reframe::seam_ray_at`] is the inverse of [`Reframe::across_seam`]),
    /// and with the horizon at rest the world frame and the body frame are the
    /// same one, so this is the state a run would have arrived at rather than a
    /// state invented beside the mechanism.
    fn standing(reframe: &Reframe, offset_deg: f32, at: f64) -> SeamAnchor {
        let delta = offset_deg.to_radians();
        let centre = reframe.seam_nearest([0.0, 0.0, 1.0]);
        SeamAnchor {
            on: reframe
                .body_ray(reframe.seam_ray_at(centre, -delta))
                .map(f64::from),
            delta,
            at,
            target: delta,
            gain: 0.0,
        }
    }

    fn radius(reframe: &Reframe, lens: usize, landing: Landing) -> f32 {
        let block = &reframe.lenses[lens];
        norm([landing.pixel[0] - block.cx, landing.pixel[1] - block.cy])
    }
}
