//! The Mei/UCM forward map: one output ray to one lens pixel, for each lens
//! of the camera, and how much of each is shown.
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
//! Written from the model description in `docs/research/insv-format.md` 5.1
//! (Mei and Rives 2007, as OpenCV's `cv::omnidir` states it). Nothing here
//! is transcribed from Gyroflow's `insta360.wgsl`, so this file is plain
//! AGPL-3.0 with no GPL header.
//!
//! The other half of the map is the **output** projection, [`Screen`]: how a
//! point of the frame becomes the ray this file then projects. A flat window
//! on the world is what a player wants until the view gets wide, and past
//! about 110 degrees it stops being one, so the frame bends from there out
//! until the whole sphere is a ball with room around it (issue #47).

use std::f32::consts::PI;
use std::sync::OnceLock;

use kjerag_meta::{Intrinsics, Lens, Pose, Quat};

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

/// How wide the handover between the two lenses is, in degrees of world
/// angle, centred on the seam (issue #48).
///
/// The overlap is 14 degrees and until this it was the band: the weights
/// crossed over across the whole of it, so anything the two lenses disagree
/// about was drawn twice across 10 degrees of picture. Two degrees is what
/// the owner validated, and it is a trade with a number on each side
/// (docs/research/insv-format.md 6.8): scored against the front lens alone,
/// a 2 degree band keeps 0.687 of that sharpness where the shipped weights
/// keep 0.518 and a hard cut would keep 0.721, so it takes 80 percent of what
/// a cut would give while staying a blend.
///
/// What bounds it from below is **shear**, the two lenses' disagreement
/// divided by the band: above 1 the crossover folds the picture rather than
/// blending it. That is why this constant could not ship before the
/// calibration fit above it. At the 1.7 degrees the factory calibration
/// leaves, a 2 degree band sits at 1.07, on the fold; at the 0.5 to 0.8 the
/// fitted correction leaves it sits near 0.4, and a 1 degree band would be
/// the next thing to measure rather than the next thing to assume.
///
/// Since issue #103's stage 4 this is the **floor** rather than the width.
/// The shear that bounds it is measured per direction on every frame instead
/// of quoted, so the band opens where a reading would otherwise fold it and
/// stays at exactly this everywhere else, which is the whole far field, every
/// direction that has never correlated, and every file with one lens stream
/// ([`super::band::width`]).
const CROSSOVER_DEG: f32 = 2.0;

/// Research only: how wide this run opens the handover, from
/// `KJERAG_HANDOVER_DEG`, in degrees.
///
/// Unset, which is every shipped run and every run that does not name it, is
/// [`CROSSOVER_DEG`] and the picture is the one on `main`, bit for bit. Set to
/// a width, the whole handover opens to it: the weights cross over across that
/// many degrees instead of two, and so does everything the weights carry - the
/// epipolar bend, which is split by them, and the along-seam correction, which
/// lens 1 takes whole and the weights hand over.
///
/// **One knob and not two, because there can only be one.** The along-seam
/// term is applied over a whole lens rather than across the band
/// ([`Reframe::bent`]), so what ramps it from nothing to all of it in the
/// picture is the handover itself, and it cannot be given a wider support of
/// its own. The two lenses draw one piece of content in one place only while
/// the share lens 1 takes and the share lens 0 takes differ by exactly the
/// disagreement the fit measured, so wherever both lenses are in the picture
/// that difference is pinned at one whole correction, and what the picture
/// shows walks from none of it to all of it exactly as the weights do. A ramp
/// spread wider than the weights is a ramp that un-corrects the seam over the
/// width it spread. So the support of the along-seam handover **is** the
/// crossover, and this widens the crossover.
///
/// Not a setting, not a key and not a menu item (AGENTS.md, zero-config
/// playback): an environment variable, read once, written nowhere.
const HANDOVER_DEG: &str = "KJERAG_HANDOVER_DEG";

/// The widest the research handover may be asked to open, in degrees: the
/// overlap the two lenses have.
///
/// Past it the crossover would be asking for a share of a lens outside its own
/// picture, which the coverage test already refuses, so the picture would stop
/// answering the width it was given.
const OVERLAP_DEG: f32 = 14.0;

/// How wide the handover is on this run, in degrees, which is
/// [`CROSSOVER_DEG`] unless [`HANDOVER_DEG`] asked for another width.
///
/// Read once. The shader takes its copy as a constant written into the source
/// ([`wgsl`]) and this side takes it here, so the two halves of the map cannot
/// disagree about it.
fn crossover_deg() -> f32 {
    static WIDTH: OnceLock<f32> = OnceLock::new();
    *WIDTH.get_or_init(|| {
        let Ok(asked) = std::env::var(HANDOVER_DEG) else {
            return CROSSOVER_DEG;
        };
        match handover(&asked) {
            Ok(width) => {
                println!(
                    "blend:  research handover on, {HANDOVER_DEG}={width} deg: the two lenses \
                     cross over across {width} degrees of world angle instead of {CROSSOVER_DEG}"
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
    /// A uniform block's size rounds up to its own alignment, which the
    /// matrices in [`LensBlock`] make 16 bytes. WGSL does that itself;
    /// `repr(C)` does not, and the two sizes have to agree or
    /// `min_binding_size` rejects the pipeline.
    _pad: [f32; 4],
}

/// One lens's half of the block: the Mei/UCM model, and where the lens is
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
    /// A uniform array's element stride rounds up to the element's 16-byte
    /// alignment. WGSL does that itself; `repr(C)` does not.
    _pad: [f32; 1],
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

/// What the band moves one ray by, in view space, on each of the seam's two
/// axes (issue #103, stage 5).
///
/// Two vectors and not one, because the two are applied by different laws:
/// the epipolar one across the handover with the other lens's weight, the
/// along-seam one to lens 1 over its whole picture. [`Reframe::bent`] says
/// why. `Default` is no bend on either, which is the picture stage 1 drew.
///
/// WGSL twin: the `Band` struct's `offset` and `along`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bend {
    /// Along [`Ring::epi`](super::band::Ring::epi), scaled by the ray's length
    /// so that adding it turns the ray by the disparity in radians.
    pub epi: [f32; 3],
    /// Along [`Ring::perp`](super::band::Ring::perp), scaled by the ray
    /// flattened into the seam plane, which is the `cos(elevation)` a relative
    /// roll produces and what takes it to zero at both lens poles.
    pub along: [f32; 3],
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
        Self {
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
            _pad: [0.0; 4],
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
            _pad: [0.0; 4],
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
    /// WGSL twin: `blend`.
    pub fn blend(&self, view_ray: [f32; 3]) -> Blend {
        self.blend_bent(view_ray, super::band::Reading::default())
    }

    /// The same with the band's own correction in it (issue #103): each lens's
    /// ray bent by the **other** lens's weight times what the two lenses
    /// disagree by at this ray's azimuth, on **both** of the seam's axes since
    /// stage 5 - the epipolar one, which is depth, and the along-seam one,
    /// which is the camera.
    ///
    /// The two bends then differ by exactly the disparity wherever the weights
    /// sum to 1, so the two lenses show the same content across the whole
    /// band; and each lens's own bend is zero wherever its weight is 1, so
    /// nothing outside the band moves and there is no edge to feather. Neither
    /// property is arranged: both fall out of the weights this function was
    /// already computing.
    ///
    /// The crossover is taken from the **unbent** ray, on purpose. The bend is
    /// what the handover asked for, so a handover that then followed the bend
    /// would be its own input.
    ///
    /// **How wide the handover is, is the same question** (issue #103, stage
    /// 4). The bend runs from zero to the whole disparity across the band, so
    /// the band has to be wide enough to carry it, and
    /// [`Self::crossover_at`] is that width. It is the floor everywhere the
    /// disparity is small, so the far field is the picture it always was.
    ///
    /// WGSL twin: `blend`, whose `band` argument is `band_bend`'s answer.
    pub fn blend_bent(&self, view_ray: [f32; 3], reading: super::band::Reading) -> Blend {
        let mut landings = [Landing::MISSED; MAX_LENSES];
        let mut weights = [0.0; MAX_LENSES];
        let reach = norm3(view_ray);
        // Both axis cosines, once: the crossover below needs them together
        // and the cap test needs them one at a time, and computing them here
        // is what keeps this pass costing what it cost before the crossover
        // existed ([`Self::handover`]).
        let axis: [f32; MAX_LENSES] = std::array::from_fn(|lens| self.axis_of(lens, view_ray));
        let band = self.crossover_at(reading.epi);
        let front = self.handover(axis, reach, band);
        let bend = self.bent(view_ray, reading, band);
        for lens in 0..MAX_LENSES {
            if !self.covers(lens, axis[lens], reach) {
                continue;
            }
            let share = match lens {
                0 => front,
                _ => 1.0 - front,
            };
            // This lens's share of the bend is the OTHER lens's weight, with
            // the sign that puts the two of them one whole disparity apart.
            let carry = match lens {
                0 => share - 1.0,
                _ => 1.0 - share,
            };
            // The along-seam term is not shared out: it goes to lens 1 whole,
            // which is the convention the calibration already uses
            // ([`super::seam::SeamFit`] turns lens 1 and leaves lens 0 alone),
            // and it is applied over the whole picture rather than across the
            // handover. See [`Self::bent`].
            let turn = f32::from(u8::from(lens == 1));
            let bent =
                std::array::from_fn(|c| view_ray[c] + carry * bend.epi[c] + turn * bend.along[c]);
            landings[lens] = self.project(lens, bent);
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
            true => crossover(axis[0] - axis[1], reach, band),
            false => 1.0,
        }
    }

    /// How wide the crossover opens at a ray whose measured disparity is
    /// `disparity`, in radians (issue #103, stage 4).
    ///
    /// [`CROSSOVER_DEG`] wherever that already carries the reading without
    /// folding, which is every direction under 1.8 degrees of disparity, so
    /// the far field's handover is the one it has always had. Wider exactly
    /// where the reading would otherwise be clamped, and only by as much as
    /// the reading needs.
    ///
    /// WGSL twin: `band_width`.
    pub fn crossover_at(&self, disparity: f32) -> f32 {
        super::band::width(disparity, crossover_deg().to_radians())
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
            epi: Self::channel(a.disparity, a.confidence, b.disparity, b.confidence, mix),
            // One fitted field over the whole circle rather than a cell
            // lookup: see `Along`. This azimuth's cosine and sine are the ray
            // flattened into the seam plane.
            along: along.at(body[0] / reach, body[1] / reach),
        }
    }

    /// One channel of one ray, weighted by the evidence behind that channel in
    /// each cell and taxed by how much of it reaches
    /// [`KEEP`](super::band::KEEP).
    ///
    /// A direction that has stopped correlating stops contributing, and with
    /// no evidence at all the answer is zero, which is the picture before the
    /// band existed. WGSL twin: `carry`.
    fn channel(a: f32, wa: f32, b: f32, wb: f32, mix: f32) -> f32 {
        let (ea, eb) = (wa * (1.0 - mix), wb * mix);
        let total = ea + eb;
        if total <= 0.0 {
            return 0.0;
        }
        let strength = ((wa + (wb - wa) * mix) / super::band::KEEP).clamp(0.0, 1.0);
        (ea * a + eb * b) / total * strength
    }

    /// The offset one lens's ray takes for a whole disparity, in view space,
    /// scaled by the ray's own length so that adding it turns the ray by
    /// `disparity` radians.
    ///
    /// Clamped to what the crossover can carry without folding
    /// (`super::band::carried`), which is the guard the record has wanted
    /// since the band narrowed: the bend's own gradient across the band **is**
    /// the shear, and past 1 the mapping prints the picture back over itself.
    /// Since stage 4 the crossover it is clamped against is the one that
    /// direction's own reading opened ([`Self::crossover_at`]), so the clamp
    /// bites only where the band has run out of room to open.
    ///
    /// WGSL twin: `band_bend`.
    pub fn bend(&self, view_ray: [f32; 3], reading: super::band::Reading) -> Bend {
        self.bent(view_ray, reading, self.crossover_at(reading.epi))
    }

    /// The same with the width already in hand, which is how [`Self::blend_bent`]
    /// asks for it: the handover needs the same number and neither of them may
    /// have its own copy.
    ///
    /// **The two axes are applied by different laws, because they are
    /// different phenomena** (issue #103, stage 5).
    ///
    /// The epipolar term is **parallax**: the two lenses genuinely see
    /// different things and neither is wrong, so it is split across the
    /// handover by the other lens's weight. That makes the two agree inside
    /// the band and moves nothing outside it, which is the whole of stage 2.
    /// It is what folds and what opens the crossover.
    ///
    /// The along-seam term is **the camera**: parallax cannot reach that axis
    /// at any distance, so what is left there is a relative pose error the
    /// static five-knob fit could not describe, and a pose error is wrong
    /// everywhere and not only at the handover. Correcting it only across the
    /// band would make the two pictures agree over two degrees and leave the
    /// horizon still drawn in two places, which is measured: the band-local
    /// form moves the owner's reference view by 0.03 view px of 32.8. So it is
    /// applied the way the calibration it belongs to is applied - to lens 1,
    /// over its whole picture, with lens 0 left exactly alone.
    ///
    /// The scale is `reach`, the ray flattened into the seam plane, and that
    /// is not a taper chosen to be safe. A relative roll `w` about the body's
    /// z displaces a direction `d` by `w x d`, which is `|w| cos(elevation)`
    /// along the seam's own tangent at every elevation and exactly zero at
    /// both lens poles, where an azimuth does not exist. Scaling by the
    /// flattened length rather than the whole one **is** that factor, for
    /// free, and it makes a constant reading exactly a relative roll - which
    /// is what the harmonic decomposition says a constant along-seam residual
    /// is (`kjerag-spike --bin seam`).
    fn bent(&self, view_ray: [f32; 3], reading: super::band::Reading, band: f32) -> Bend {
        let Some(at) = self.seam_at(view_ray) else {
            return Bend::default();
        };
        let body = self.body_ray(view_ray);
        let epi = super::band::carried(reading.epi, band) * norm3(view_ray);
        let along = reading.along * body[0].hypot(body[1]);
        // Back out of the body's frame. `view_to_body` is a rotation, so its
        // transpose is its inverse.
        let out = |axis: [f32; 3], scale: f32| {
            std::array::from_fn(|row| {
                scale
                    * (0..3)
                        .map(|c| self.view_to_body[row][c] * axis[c])
                        .sum::<f32>()
            })
        };
        Bend {
            epi: out(at.epi, epi),
            along: out(at.perp, along),
        }
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
        let mut landing = self.mei(lens, normalize(aimed));
        if self.is_rolling() {
            for _ in 0..rounds {
                let share = self.readout_share(landing.pixel);
                let turned = turned(aimed, block.turn.map(|axis| axis * share));
                landing = self.mei(lens, normalize(turned));
            }
        }
        landing
    }

    /// Whether the readout correction runs at all. Off for a file with no IMU
    /// record, and then [`Self::project`] is what it was before issue #9.
    ///
    /// WGSL twin: the `reframe.row_axis` test in `project`.
    fn is_rolling(&self) -> bool {
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

    /// The Mei/UCM model itself, for one of this block's lenses.
    ///
    /// WGSL twin: `mei`.
    fn mei(&self, lens: usize, p: [f32; 3]) -> Landing {
        mei(&self.lenses[lens], p)
    }
}

/// The Mei/UCM model itself: a unit ray in one lens's own frame, to a pixel
/// of that lens's delivered frame.
///
/// Free of [`Reframe`] because [`coverage_floor`] runs it against a block
/// that is still being built, before there is a `Reframe` to index.
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
    let depth = lens.image_radius - norm(offset);
    Landing {
        pixel: [offset[0] + lens.cx, offset[1] + lens.cy],
        inside: denom > 0.0 && injective && depth > 0.0,
        axis: p[2],
        depth,
    }
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
        mei(block, [rim * cos, rim * sin, axis]).inside
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
/// here rather than twice at the call site. `band` is how wide the crossover
/// is at this ray, which since issue #103's stage 4 is a measurement rather
/// than [`CROSSOVER_DEG`] itself, and is that constant exactly wherever the
/// reading is small enough for it ([`Reframe::crossover_at`]).
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
/// WGSL twin: `crossover`.
fn crossover(apart: f32, reach: f32, band: f32) -> f32 {
    (0.5 + apart / (2.0 * reach * band)).clamp(0.0, 1.0)
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
        _pad: [0.0; 1],
    };

    fn new(lens: &Lens, index: usize, frame: Size, camera: Camera, held: Held) -> Self {
        let Intrinsics { xi, fx, fy, cx, cy } = lens.intrinsics;
        let distortion = lens.distortion;
        let mut block = Self {
            view_to_lens: view_to_lens(&lens.pose, index, camera, held).columns(),
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
                lens_from_body(&lens.pose, index).mul_vec(rolling.turn.map(|axis| axis as f32))
            }),
            // Solved for below: the cap is a property of the model this block
            // has just been filled with, and the readout it widens by is the
            // `turn` above.
            axis_min: 2.0,
            _pad: [0.0; 1],
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
fn view_to_lens(pose: &Pose, index: usize, camera: Camera, held: Held) -> Mat3 {
    lens_from_body(pose, index).mul(body_from_view(camera, held))
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
fn lens_from_body(pose: &Pose, index: usize) -> Mat3 {
    Mat3::from(pose.lens_from_body().rows()).mul(opposed(index))
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
    format!(
        "const MAX_LENSES = {MAX_LENSES}u;\n\
         const READOUT_STEPS = {READOUT_STEPS}u;\nconst CROSSOVER = {:?};\n{WGSL}",
        crossover_deg().to_radians(),
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
  // WGSL rounds this block's size up to its own 16-byte alignment. Rust twin:
  // `Reframe::_pad`, which is what makes the two sizes agree.
  pad0: f32,
  pad1: f32,
  pad2: f32,
  pad3: f32,
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

// What the band says about one ray: the offset that puts the two lenses'
// pictures on top of each other, and how wide the handover has to be to carry
// it without folding. Both come out of one measurement and neither may be
// taken without the other, which is why they arrive together.
//
// Declared here rather than beside `band_bend`, which fills it, because the
// compute half of the band is compiled with this file and without that one,
// and a shader that names a type it has not been given does not compile.
// Rust twins: `Reframe::bend` and `Reframe::crossover_at`.
struct Band {
  offset: vec3<f32>,
  // The along-seam correction, which lens 1 takes whole and lens 0 does not
  // take at all: it is the camera and not the scene, so it is applied the way
  // the calibration is. Rust twin: `Bend::along`.
  along: vec3<f32>,
  crossover: f32,
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

// Every lens's claim on the ray, normalized. Rust twin: `Reframe::blend_bent`.
//
// `band` is what the band says at this direction (`band_bend`): the offset the
// two lenses disagree by, and how wide the handover has to be to carry it. On
// a file with one lens stream and on every direction the band has not
// measured, the offset is zero and the width is the shipped crossover, and
// then this pass is what it was before issue #103. Both are taken from the
// UNBENT ray: a bend that moved its own lookup would be its own input.
//
// The loop runs MAX_LENSES times whatever the file holds, and the lens count
// zeroes the claim of a slot that has no stream rather than shortening the
// loop. A loop this compiler cannot unroll indexes `out` dynamically, which
// puts it in scratch memory and costs more than the blend does; the numbers
// are on the Rust twin. The array writes stay unconditional for the same
// reason; what `within` skips is the model, not the bookkeeping.
fn blend(ray: vec3<f32>, band: Band) -> Blend {
  var out: Blend;
  var total = 0.0;
  let reach = length(ray);
  // Both axis cosines before the loop: the crossover needs them together,
  // the cap test needs them one at a time, and reading them back out of
  // `out` after the loop instead costs 5.5 ms a redraw against 3.6. Rust
  // twin: `Reframe::blend`.
  let axis0 = axis_of(reframe.lenses[0], ray);
  let axis1 = axis_of(reframe.lenses[1], ray);
  let front = handover(axis0, axis1, reach, band.crossover);
  for (var index = 0u; index < MAX_LENSES; index += 1u) {
    let lens = reframe.lenses[index];
    // Zero, which is `Landing::MISSED`: a lens the ray cannot reach is never
    // projected and its landing is never read.
    var landing: Landing;
    var claimed = 0.0;
    if within(lens, select(axis1, axis0, index == 0u), reach) {
      let share = select(1.0 - front, front, index == 0u);
      // This lens's share of the bend is the OTHER lens's weight, with the
      // sign that puts the two of them one whole disparity apart. Rust twin:
      // `Reframe::blend_bent`.
      let carry = select(1.0 - share, share - 1.0, index == 0u);
      // The along-seam term is not shared out: lens 1 takes it whole and lens
      // 0 does not take it at all. Rust twin: `Reframe::blend_bent`'s `turn`.
      let turn = select(0.0, 1.0, index == 1u);
      landing = project(lens, ray + carry * band.offset + turn * band.along);
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
// no seam to hand over at. Rust twin: `Reframe::handover`.
fn handover(axis0: f32, axis1: f32, reach: f32, band: f32) -> f32 {
  if reframe.lens_count <= 1.0 {
    return 1.0;
  }
  return crossover(axis0 - axis1, reach, band);
}

// The front lens's share, from how far apart the two dot products are, across
// a band this ray's own reading decided the width of (`band_width`). Rust
// twin: `crossover`.
fn crossover(apart: f32, reach: f32, band: f32) -> f32 {
  return clamp(0.5 + apart / (2.0 * reach * band), 0.0, 1.0);
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
  var landing = mei(lens, normalize(aimed));
  if reframe.row_axis_x != 0.0 || reframe.row_axis_y != 0.0 {
    let turn = vec3<f32>(lens.turn_x, lens.turn_y, lens.turn_z);
    for (var step = 0u; step < READOUT_STEPS; step += 1u) {
      landing = mei(lens, normalize(turned(aimed, turn * readout_share(landing.pixel))));
    }
  }
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
  let depth = lens.image_radius - length(offset);
  var landing: Landing;
  landing.pixel = offset + vec2<f32>(lens.cx, lens.cy);
  landing.inside = denom > 0.0 && injective && depth > 0.0;
  landing.axis = p.z;
  landing.depth = depth;
  return landing;
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
                pose: Pose {
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
                pose: Pose {
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
    /// it runs [`CROSSOVER_DEG`], so the offsets that are mixed and the
    /// offsets that are not have swapped places. What has not changed is that
    /// the lens the ray leans toward is the one that leads.
    #[test]
    fn the_seam_is_a_mix_of_two_pictures_and_not_a_gap() {
        let reframe = fixture(Camera::default());

        for phi in 0..360 {
            let phi = phi as f32;
            // Well inside the crossover and well outside it. The edges
            // themselves are half a degree of lens tilt away, which is what
            // `the_crossover_is_the_width_it_says_it_is` measures rather than
            // asserts.
            for offset in [-5.0, -1.5, -0.3, 0.0, 0.3, 1.5, 5.0] {
                let ray = direction(90.0 + offset, phi);
                let blend = reframe.blend(ray);
                let mixed = blend.weights.iter().all(|weight| *weight > 0.0);
                assert!(blend.is_covered(), "nothing has {offset} degrees at {phi}");
                assert_eq!(
                    mixed,
                    offset.abs() < 1.0,
                    "{offset} degrees from the seam at {phi} weighs {:?}",
                    blend.weights,
                );
                // Which side of the seam leads is only settled a lens tilt
                // away from it: the two axes are 0.3 degrees off exactly
                // opposed, so on the halfway line itself either lens is a
                // fair answer.
                if offset == 0.0 {
                    continue;
                }
                let leader = usize::from(blend.weights[1] > blend.weights[0]);
                assert_eq!(
                    leader,
                    usize::from(offset > 0.0),
                    "{offset} degrees past the seam at {phi} leads with lens {leader}",
                );
            }
        }
    }

    /// What the picture carries of the along-seam correction at one offset
    /// from the seam, in units of the correction itself.
    ///
    /// Lens 1 takes the along-seam term whole and lens 0 takes none of it
    /// ([`Reframe::blend_bent`]), so the share of it the picture shows **is**
    /// lens 1's weight. Nothing else in the pass carries that axis.
    fn along_carried(reframe: &Reframe, offset: f32, phi: f32) -> f32 {
        let reading = super::super::band::Reading {
            epi: 0.0,
            along: 0.006,
        };
        reframe
            .blend_bent(direction(90.0 + offset, phi), reading)
            .weights[1]
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
    /// rather than fitted, and it is [`crossover_deg`] wide.
    #[test]
    fn the_along_seam_correction_hands_over_across_the_whole_crossover() {
        let reframe = fixture(Camera::default());
        let width = crossover_deg();

        for phi in (0..360).step_by(15) {
            let phi = phi as f32;
            // Positive offsets are lens 1's side, which is the side that takes
            // the correction ([`the_seam_is_a_mix_of_two_pictures_and_not_a_gap`]).
            let carried = |offset: f32| along_carried(&reframe, offset, phi);
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
                most - least > 0.6 * width,
                "the handover at {phi} spends {} degrees of the {width} it opened going from \
                 nine tenths of the correction to one tenth",
                most - least,
            );
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
    /// seam correction on lens 1, the narrow crossover still hands the picture
    /// over and still leaves nothing grey.
    ///
    /// The correction turns one lens by a couple of degrees, so the seam and
    /// the crossover on it turn with it. What has to survive is the margin:
    /// the band is 2 degrees wide inside an overlap of 14, so the lens the
    /// crossover hands to has 6 degrees of its own picture in hand. This is
    /// the check that the fit cannot eat that margin at the size it comes in.
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
    /// Not exactly half, and further off than it was before issue #48: this
    /// fixture's two axes are 0.3 degrees from opposed, so a direction 90
    /// degrees off lens 0 is up to 0.3 degrees off the line where the two
    /// lenses are equally far off theirs. A 2-degree crossover turns that into
    /// 0.06 of weight where the 14-degree one turned it into 0.008. The
    /// picture is centred on the lenses either way; what moved is how quickly
    /// weight answers an angle.
    #[test]
    fn the_crossover_sits_on_the_seam() {
        let reframe = fixture(Camera::default());

        for phi in 0..36 {
            let blend = reframe.blend(direction(90.0, phi as f32 * 10.0));
            near(blend.weights[0], 0.5, 0.08);
            near(blend.weights[1], 0.5, 0.08);
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

    /// And it opens to the width a near-field reading asks for, still centred
    /// on the seam (issue #103, stage 4).
    ///
    /// The same sweep as above, drawn through the bent blend, so what is
    /// measured is the band the shipped pass actually hands over across and
    /// not the arithmetic that decided it.
    #[test]
    fn a_near_reading_opens_the_crossover_to_what_it_needs() {
        let reframe = fixture(Camera::default());
        for disparity_deg in [0.0f32, 1.8, 2.2, 2.6] {
            let disparity = disparity_deg.to_radians();
            let wanted = reframe.crossover_at(disparity).to_degrees();
            for phi in [0.0, 90.0, 180.0, 270.0] {
                let mixed: Vec<f32> = (0..3000)
                    .map(|step| 70.0 + step as f32 * 0.01)
                    .filter(|theta| {
                        let weights = reframe
                            .blend_bent(direction(*theta, phi), reading(disparity))
                            .weights;
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

    /// The bound the widest band is not allowed to cross, measured off the
    /// calibration fixture rather than quoted from the format study.
    ///
    /// A band that opened past the overlap would hand over to a lens that has
    /// no picture there, and a lens with no picture is a weight that steps to
    /// zero rather than fading, which is a seam of its own. What has to fit is
    /// half the widest band **plus the whole bend it carries**: at the edge of
    /// the band one lens's weight is 1, so the other lens is sampled a whole
    /// disparity away from where the ray points.
    #[test]
    fn the_widest_band_and_its_bend_stay_inside_the_overlap() {
        let reframe = fixture(Camera::default());
        let overlap = reframe
            .overlap()
            .expect("the fixture has two lenses")
            .to_degrees();
        let widest = crate::band::WIDEST_DEG;
        let reach = 0.5 * widest + widest * 0.9;
        // Measured on the fixture 2026-08-01: 14.44 degrees of overlap, 7.22
        // a side, against a reach of 4.04.
        assert!(
            reach < 0.5 * overlap,
            "the widest band reaches {reach:.2} deg off the seam into an overlap of \
             {overlap:.2} deg, which is {:.2} deg a side",
            0.5 * overlap,
        );
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
            reframe.crossover_at(0.0),
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
                mei(block, [sin * cos_phi, sin * sin_phi, cos]).inside
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
    const READOUT_TURN: f64 = 90.0 * 0.015_883 * std::f64::consts::PI / 180.0;

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
                let again = reframe.mei(
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
                    reframe.mei(lens, normalize(reframe.lenses[lens].lens_ray(ray))),
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
        // padded vec3 (issue #103).
        assert_eq!(std::mem::size_of::<Reframe>(), 288 + 48 + 16);
    }

    fn radius(reframe: &Reframe, lens: usize, landing: Landing) -> f32 {
        let block = &reframe.lenses[lens];
        norm([landing.pixel[0] - block.cx, landing.pixel[1] - block.cy])
    }
}
