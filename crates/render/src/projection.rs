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
//! weight per lens, one outside the overlap and a smooth crossover inside it
//! (issue #7). A ray is dropped only where **no** lens has it.
//!
//! The map also carries **when** each ray was seen (issue #9). A frame comes
//! off the sensor a row at a time over 15.9 ms, so the orientation a ray is
//! carried through belongs to the row it lands on rather than to the frame,
//! and [`Reframe::solve`] is that: reframing, stabilization and the readout
//! in one backward mapping per output pixel, with nothing resampled and no
//! pass added. Which way the sensor reads is not in the file, so it is
//! measured per camera and the correction is switched off on any camera it
//! has not been measured on (`kyerag_meta::Sweep`).
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
//! until the earth has curled into a ball inside the picture and the sky is
//! wrapped round it into every corner (issue #47). **Every point of the frame
//! looks at the sphere, at every field of view the zoom offers**: there is no
//! state where the picture is a disc with empty room around it.

use std::f32::consts::PI;

use kyerag_meta::{Intrinsics, Lens, Pose, Quat};

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
/// **One round, measured** (`kyerag-spike --bin rolling model=1`): against a
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

/// How far off the view axis the frame's own corner is allowed to look, which
/// is what caps the zoom ([`fov_ceiling`]).
///
/// The corner is the furthest point of the frame, and what stops the zoom is
/// that the map runs out of picture there before it runs out of angle.
/// Approaching half a turn the antipode spreads over a whole circle of the
/// frame -- the plane radius keeps growing while `sin(theta)` closes -- so the
/// corner is stretched along the way it wraps by `r / sin(theta)` against
/// `sec^2(shrink * theta)` radially: 2.3x at 150 degrees, 4.8x at 165, 7.4x
/// here, 19x at 176 and 78x at 179.
///
/// **170 is where that stops showing**, measured by eye on real footage at
/// 2560x1440 rather than argued: sensor grain in a clear sky still reads as
/// grain at 165 and 170 and is visibly combed into arcs at 173, and by 179 the
/// outer third of the frame is smear. The framing that comes with it is the
/// one being asked for: looking down, the horizon circle is 0.75 of the frame
/// height, so the earth is a ball inside the picture with the sky wrapped
/// round it into every corner.
///
/// It is stated as the corner's own angle rather than as a field of view
/// because that is what makes the far end look the same in every window: a
/// square window and an ultrawide one reach it at 294 and 334 degrees of
/// horizontal field of view, and both have the same 10 degrees of margin off
/// the antipode.
pub(crate) const CORNER_MAX: f32 = 170.0 * PI / 180.0;

/// The output projection: how a point of the frame becomes a ray, at one
/// field of view and one window shape.
///
/// **The family.** A plane radius `r` from the middle of the frame is the
/// direction `theta` off the view axis with `r = tan(shrink * theta) /
/// shrink`. At `shrink` 1 that is `r = tan(theta)`, the flat window every
/// perspective view is; at 1/2 it is `r = 2 tan(theta / 2)`, which is
/// stereographic, which is the tiny planet; and below that the world keeps
/// shrinking into the same frame. One parameter walks all three, and every
/// one of them meets the next in value and in slope, so a scroll through the
/// range has nowhere to pop.
///
/// **The schedule.** `shrink` is `FOV_FLAT / fov`, held at 1 until the view
/// is wider than that. Past there the product `shrink * fov / 2` is constant,
/// which is worth reading twice: the frame keeps the half angle of the widest
/// flat view, and widening the field of view shrinks the world into it
/// instead of stretching it. That is what makes the zoom keep meaning zoom
/// out through the bend, and `the_picture_only_ever_shrinks` is the check.
///
/// **The frame is all picture.** The zoom stops at [`fov_ceiling`], which is
/// where the corner reaches [`CORNER_MAX`], so no point of any frame the
/// player can ask for is further out than that: every one of them is a real
/// direction and the sky fills the corners. The far end of the zoom is the
/// tiny planet, not a ball with empty room around it.
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
    /// about 0.35.
    shrink: f32,
    /// Output width over output height. The vertical field of view is
    /// whatever this leaves.
    aspect: f32,
    /// A uniform block aligns a struct inside it to 16 bytes and rounds its
    /// size up to the same, which `repr(C)` does not: three floats here and
    /// four in the shader would be a layout the pipeline rejects.
    _pad: f32,
}

impl Screen {
    fn new(camera: Camera, aspect: f32) -> Self {
        // The ceiling is a property of the **window**, so it is applied here
        // as well as in `Camera::zoom`: a window narrowed after the scroll
        // has a corner further out than the one the scroll was clamped
        // against, and the frame would run off the sphere at the edges of a
        // view nobody touched.
        let fov = camera.fov.min(fov_ceiling(aspect));
        let shrink = (FOV_FLAT / fov).min(1.0);
        Self {
            half_extent: (shrink * fov * 0.5).tan() / shrink,
            shrink,
            aspect,
            _pad: 0.0,
        }
    }

    /// The ray a point of the output looks along, in view space: x right, y
    /// down, z forward. `uv` runs 0 to 1 across the output, y down.
    ///
    /// Every point of the frame has one. The far end of the zoom is capped
    /// where the corner is [`CORNER_MAX`] off the axis, which is short of the
    /// half turn a direction runs out at.
    ///
    /// WGSL twin: `view_ray`.
    fn ray(self, uv: [f32; 2]) -> [f32; 3] {
        let plane = [
            (uv[0] * 2.0 - 1.0) * self.half_extent,
            (uv[1] * 2.0 - 1.0) * self.half_extent / self.aspect,
        ];
        // The flat window, and the whole of it: two multiplies, the ray at z
        // of 1 and unnormalized, exactly the instructions this was before
        // issue #47. The length and the trig below are what the bend costs,
        // and the range the player already had does not pay for them.
        if self.shrink == 1.0 {
            return [plane[0], plane[1], 1.0];
        }
        let radius = norm(plane);
        let theta = (self.shrink * radius).atan() / self.shrink;
        let (sin, cos) = theta.sin_cos();
        // The middle of the frame, where the azimuth is not defined and the
        // ray is the view axis itself.
        let out = match radius > 0.0 {
            true => sin / radius,
            false => 0.0,
        };
        [plane[0] * out, plane[1] * out, cos]
    }
}

/// The far end of the zoom: the widest field of view worth offering, which is
/// the one whose corner looks [`CORNER_MAX`] off the view axis.
///
/// It depends on the window shape because the corner does: a wide window's
/// corner is a smaller share of a turn from the middle of its own edge than a
/// square window's is, so it can be zoomed out further before it reaches the
/// same angle. The solve is closed, because past [`FOV_FLAT`] the whole
/// schedule is one division by `shrink`: the corner sits at
/// `tan(FOV_FLAT / 2) * diagonal / shrink` in the plane and `theta` is
/// `atan(shrink * r) / shrink`, so the `shrink` inside the `atan` cancels and
/// the angle out to the corner is [`corner_at_flat`] over `shrink`.
pub(crate) fn fov_ceiling(aspect: f32) -> f32 {
    FOV_FLAT * CORNER_MAX / corner_at_flat(aspect)
}

/// How far off the view axis the corner of a window this shape looks in the
/// widest flat view, in radians. Around 59 degrees on 16:9 and 64 on a square
/// window.
fn corner_at_flat(aspect: f32) -> f32 {
    let diagonal = (1.0 + 1.0 / (aspect * aspect)).sqrt();
    ((FOV_FLAT * 0.5).tan() * diagonal).atan()
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
    has_frame: f32,
    linearize: f32,
    elapsed: f32,
    /// Which way across the delivered frame the sensor's rows advance
    /// (`kyerag_meta::Sweep`), and whether the correction runs at all: both
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
    _pad: [f32; 2],
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
/// `inside` false means the ray missed this lens. Missing every lens is what
/// the shader paints [`OUTSIDE_GRAY`] for; missing one of two is ordinary,
/// and is most of what [`Reframe::blend`] is weighing.
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
    /// Whether any lens has this ray at all. False is what the shader paints
    /// [`OUTSIDE_GRAY`] for.
    pub fn is_covered(&self) -> bool {
        self.weights.iter().any(|weight| *weight > 0.0)
    }
}

/// What the shader paints where no lens has a picture. Neutral and dark,
/// in the same gamma-encoded space as the video, so the sRGB branch treats
/// it the same way it treats a sampled pixel.
pub const OUTSIDE_GRAY: f32 = 0.10;

/// Where the camera body was when a frame was taken, and how the view is to
/// be held against it.
///
/// `body_from_world` is the inverse of the orientation `kyerag-meta`
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
    /// A unit direction in delivered-frame pixels, from `kyerag_meta::Sweep`.
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
            screen: Screen::new(camera, aspect),
            frame_width: frame.width as f32,
            frame_height: frame.height as f32,
            lens_count: lenses.len().min(MAX_LENSES) as f32,
            has_frame: 1.0,
            linearize: f32::from(u8::from(linearize)),
            elapsed: 0.0,
            row_axis: held
                .rolling
                .map_or([0.0; 2], |rolling| rolling.axis.map(|c| c as f32)),
            sharpen: sampling.limits(),
            _pad: [0.0; 2],
        }
    }

    /// No file open: the shader draws its bring-up gradient. One lens that
    /// has no picture in it, so the map still runs and every ray still misses.
    pub fn gradient(elapsed: f32, aspect: f32, linearize: bool) -> Self {
        Self {
            lenses: [LensBlock::EMPTY; MAX_LENSES],
            screen: Screen::new(Camera::default(), aspect),
            frame_width: 1.0,
            frame_height: 1.0,
            lens_count: 1.0,
            has_frame: 0.0,
            linearize: f32::from(u8::from(linearize)),
            elapsed,
            row_axis: [0.0; 2],
            // Every ray misses every lens, so no plane is ever sampled and
            // the gradient reaches the target either way.
            sharpen: Sampling::default().limits(),
            _pad: [0.0; 2],
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
    pub fn view_ray(&self, uv: [f32; 2]) -> [f32; 3] {
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
        let mut landings = [Landing::MISSED; MAX_LENSES];
        let mut weights = [0.0; MAX_LENSES];
        for lens in 0..MAX_LENSES {
            if !self.within(lens, view_ray) {
                continue;
            }
            landings[lens] = self.project(lens, view_ray);
            if lens < self.lens_count as usize {
                weights[lens] = claim(landings[lens]);
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
        let landing = |uv: [f32; 2]| self.project(lens, self.view_ray(uv)).pixel;
        let here = landing(uv);
        let step = |to: [f32; 2]| {
            let moved = landing(to);
            (moved[0] - here[0]).hypot(moved[1] - here[1])
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
    /// costs 195 to 340 ms of stale far hemisphere. `kyerag-spike --bin
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
    /// reaches. [`CORNER_MAX`] at the far end of the zoom, which is a cone
    /// holding all but a cap around the antipode.
    pub fn cone(&self) -> f32 {
        normalize(self.view_ray([0.0, 0.0]))[2]
            .clamp(-1.0, 1.0)
            .acos()
    }

    /// How far off its own axis this lens can still see, in radians, cap
    /// margin included. `None` for a slot with no picture in it.
    ///
    /// For the instruments: `kyerag-spike --bin gating` reports it, and it is
    /// [`LensBlock::axis_min`] read back as an angle.
    pub fn coverage(&self, lens: usize) -> Option<f32> {
        let axis_min = self.lenses[lens].axis_min;
        (axis_min <= 1.0).then(|| axis_min.acos())
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
        let block = &self.lenses[lens];
        let axis: f32 = (0..3).map(|c| block.view_to_lens[c][2] * view_ray[c]).sum();
        axis >= block.axis_min * norm3(view_ray)
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
    /// against (`kyerag-spike --bin rolling model=1`).
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
    if !inside_anywhere(block, 1.0) {
        return 2.0;
    }
    let (mut outside, mut inside) = (-1.0f32, 1.0f32);
    for _ in 0..CAP_BISECTIONS {
        let middle = 0.5 * (outside + inside);
        match inside_anywhere(block, middle) {
            true => inside = middle,
            false => outside = middle,
        }
    }
    let cap = outside.clamp(-1.0, 1.0).acos() + widen + CAP_MARGIN_DEG.to_radians();
    cap.min(std::f32::consts::PI).cos()
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
/// Two factors, per docs/research/insv-format.md 6.6, and neither of them is
/// a feather width to be chosen:
///
/// - **longitude preference**, [`longitude`], which is what puts the
///   crossover on the seam great circle rather than wherever the two
///   coverages happen to meet;
/// - **coverage depth**, `landing.depth`, the distance transform from this
///   lens's own validity boundary. It reaches zero exactly where the picture
///   stops, so a lens fades out as it runs out of picture, and the rim of
///   the image circle, which is where vignetting lands and where the
///   distortion polynomial is least trustworthy (5.3), is down-weighted for
///   free.
///
/// The band the product blends over is therefore the overlap itself: on the
/// X4 Air fixture it runs 83.4 to 97.4 degrees off the front axis, because
/// that is where both lenses have any picture at all.
///
/// WGSL twin: `claim`.
fn claim(landing: Landing) -> f32 {
    match landing.inside {
        true => longitude(landing.axis) * landing.depth,
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

/// How much this lens is preferred for a ray `theta` off its axis, from
/// `cos(theta)`: `cos^2(theta / 2)`, i.e. 1 straight down the axis, 1/2 on
/// the seam great circle, 0 straight out the back.
///
/// It is never zero anywhere in the overlap, so it only tilts the crossover;
/// the coverage depth is what closes it. Being exactly 1/2 for both lenses
/// on the seam is the whole job, and it is the reason the crossover does not
/// drift to wherever the two image circles happen to end.
///
/// WGSL twin: `longitude`.
fn longitude(axis: f32) -> f32 {
    0.5 * (1.0 + axis)
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
pub(crate) fn view_ray(uv: [f32; 2], camera: Camera, aspect: f32) -> [f32; 3] {
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
    lens_from_body(pose, index)
        .mul(Mat3::from(held.body_from_world.matrix().rows()))
        .mul(camera_rotation(camera))
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
/// in `kyerag_meta::Pose::lens_from_body`, because the IMU needs the same
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
        "const OUTSIDE_GRAY = vec3<f32>({OUTSIDE_GRAY:?});\nconst MAX_LENSES = {MAX_LENSES}u;\n\
         const READOUT_STEPS = {READOUT_STEPS}u;\n{WGSL}"
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
  aspect: f32,
  // WGSL rounds a struct in a uniform block up to a 16-byte size. Rust twin:
  // `Screen::_pad`.
  pad: f32,
};

struct Reframe {
  lenses: array<LensBlock, MAX_LENSES>,
  screen: Screen,
  frame_width: f32,
  frame_height: f32,
  lens_count: f32,
  has_frame: f32,
  linearize: f32,
  elapsed: f32,
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
// Every point of the frame has a ray: the zoom stops where the corner is
// CORNER_MAX off the axis, short of the half turn a direction runs out at.
// Rust twin: `Screen::ray`.
fn view_ray(uv: vec2<f32>) -> vec3<f32> {
  let screen = reframe.screen;
  let extent = (uv * 2.0 - vec2<f32>(1.0)) * screen.half_extent;
  let plane = vec2<f32>(extent.x, extent.y / screen.aspect);
  // The flat window, which is every view the player had before issue #47:
  // the same two multiplies it always was, and neither the length below nor
  // the trig under that.
  if screen.shrink == 1.0 {
    return vec3<f32>(plane, 1.0);
  }
  let radius = length(plane);
  let theta = atan(screen.shrink * radius) / screen.shrink;
  let out = select(0.0, sin(theta) / radius, radius > 0.0);
  return vec3<f32>(plane * out, cos(theta));
}

// Every lens's claim on the ray, normalized. Rust twin: `Reframe::blend`.
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
  for (var index = 0u; index < MAX_LENSES; index += 1u) {
    let lens = reframe.lenses[index];
    // Zero, which is `Landing::MISSED`: a lens the ray cannot reach is never
    // projected and its landing is never read.
    var landing: Landing;
    var claimed = 0.0;
    if within(lens, ray, reach) {
      landing = project(lens, ray);
      claimed = select(0.0, claim(landing), f32(index) < reframe.lens_count);
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
// twin: `Reframe::within`.
//
// The mounting is a rotation, so the cosine `mei` would read off the
// normalized ray is one row of it against the ray over the ray's own length.
// Multiplying the cap by the length rather than dividing keeps it to a dot
// product and a compare. `reach` is the same for every lens.
fn within(lens: LensBlock, ray: vec3<f32>, reach: f32) -> bool {
  let axis = dot(vec3<f32>(
    lens.view_to_lens[0].z,
    lens.view_to_lens[1].z,
    lens.view_to_lens[2].z,
  ), ray);
  return axis >= lens.axis_min * reach;
}

// One claim's share of all of them, the lone claimant's written rather than
// divided out. Rust twin: `share`.
fn share(claim: f32, total: f32) -> f32 {
  if claim == total {
    return 1.0;
  }
  return claim / total;
}

// Longitude preference times coverage depth. Rust twin: `claim`.
fn claim(landing: Landing) -> f32 {
  if !landing.inside {
    return 0.0;
  }
  return longitude(landing.axis) * landing.depth;
}

// cos^2(theta / 2), from cos(theta). Rust twin: `longitude`.
fn longitude(axis: f32) -> f32 {
  return 0.5 * (1.0 + axis);
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
mod tests {
    use super::*;
    use kyerag_meta::{Distortion, Sweep};

    use crate::sampling;

    const FRAME: Size = Size {
        width: 3840,
        height: 3840,
    };

    /// The X4 Air fixture in delivered-frame pixels: what `kyerag-meta`
    /// produces from `docs/research/x4air-calibration.json`, and what its own
    /// tests assert. Copied rather than parsed because the path from the
    /// fixture to a `CalibrationSet` runs through a private constructor in a
    /// crate this one only reads types from.
    fn fixture_lenses() -> Vec<Lens> {
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

    /// A direction in the body frame, `theta` degrees off the front lens's
    /// axis and turned `phi` degrees about it: `theta` of 90 is the seam
    /// great circle and 180 is straight out the back.
    fn direction(theta: f32, phi: f32) -> [f32; 3] {
        let (sin_theta, cos_theta) = theta.to_radians().sin_cos();
        let (sin_phi, cos_phi) = phi.to_radians().sin_cos();
        [sin_theta * cos_phi, sin_theta * sin_phi, cos_theta]
    }

    /// The lens carrying most of an output pixel, and where it lands, which
    /// is the question the hard pick answered before issue #7. `None` where
    /// no lens has the ray, which is what the shader paints grey.
    /// The ray at a point of the output, which every view in these tests is
    /// flat enough to have one of.
    fn ray(reframe: &Reframe, uv: [f32; 2]) -> [f32; 3] {
        reframe.view_ray(uv)
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

    /// Round the seam great circle, at the seam and either side of it: both
    /// lenses have the ray, because the overlap is about 14 degrees wide, and
    /// the one that leads is the one the ray leans toward.
    #[test]
    fn the_seam_is_a_mix_of_two_pictures_and_not_a_gap() {
        let reframe = fixture(Camera::default());

        for phi in 0..360 {
            let phi = phi as f32;
            for offset in [-5.0, -1.0, 0.0, 1.0, 5.0] {
                let ray = direction(90.0 + offset, phi);
                let blend = reframe.blend(ray);
                assert!(
                    blend.weights.iter().all(|weight| *weight > 0.0),
                    "the overlap does not reach {offset} degrees from the seam at {phi}",
                );
                // Which side of the seam leads is only settled a lens tilt
                // away from it: the two axes are 0.2 degrees off exactly
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

    /// The crossover sits on the seam great circle, which is what the
    /// longitude preference buys: without it the two coverage depths cross
    /// wherever the two image circles happen to end, which is 0.2 degrees off
    /// on this fixture and camera-dependent in general.
    ///
    /// Not exactly half: lens 1's image circle is 8 px smaller than lens 0's,
    /// so it runs out of coverage marginally sooner and carries marginally
    /// less of the seam.
    #[test]
    fn the_crossover_sits_on_the_seam() {
        let reframe = fixture(Camera::default());

        for phi in 0..36 {
            let blend = reframe.blend(direction(90.0, phi as f32 * 10.0));
            near(blend.weights[0], 0.5, 0.03);
            near(blend.weights[1], 0.5, 0.03);
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

    /// And the band is the overlap itself rather than a width chosen here:
    /// the weights are mixed exactly where both lenses have a picture, which
    /// on this fixture is 83.4 to 97.4 degrees off the front axis, and the
    /// two lenses hand over across the whole of it.
    #[test]
    fn the_blend_band_is_the_overlap_itself() {
        let reframe = fixture(Camera::default());
        let mixed: Vec<f32> = (0..3000)
            .map(|step| 70.0 + step as f32 * 0.01)
            .filter(|theta| {
                let weights = reframe.blend(direction(*theta, 0.0)).weights;
                weights.iter().all(|weight| *weight > 0.0)
            })
            .collect();

        near(*mixed.first().expect("nothing is mixed at all"), 83.2, 0.2);
        near(*mixed.last().expect("nothing is mixed at all"), 97.4, 0.2);
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
        let mut weights: [f32; MAX_LENSES] =
            std::array::from_fn(|lens| match lens < reframe.lens_count as usize {
                true => claim(landings[lens]),
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
        dragged.aim(camera.look([0.5, 0.5], 1.0), [0.6, 0.5], 1.0);
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
        dragged.aim(camera.look([0.5, 0.5], 1.0), [0.5, 0.6], 1.0);
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
        dragged.aim(camera.look(from, aspect), to, aspect);

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
        dragged.aim(camera.look(from, aspect), to, aspect);
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
    fn off_axis(screen: Screen, uv: [f32; 2]) -> f32 {
        normalize(screen.ray(uv))[2].clamp(-1.0, 1.0).acos()
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
                let tan_half_fov = (fov_deg.to_radians() * 0.5).tan();
                for uv in places() {
                    assert_eq!(
                        screen.ray(uv),
                        [
                            (uv[0] * 2.0 - 1.0) * tan_half_fov,
                            (uv[1] * 2.0 - 1.0) * tan_half_fov / aspect,
                            1.0,
                        ],
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
            let theta = off_axis(screen, uv);
            let plane = [
                (uv[0] * 2.0 - 1.0) * screen.half_extent,
                (uv[1] * 2.0 - 1.0) * screen.half_extent / screen.aspect,
            ];
            near(norm(plane), 2.0 * (theta * 0.5).tan(), 1e-4);
        }
    }

    /// Zooming out only ever zooms out. Every point of the frame looks
    /// further off the axis as the field of view widens, all the way from the
    /// narrowest view to the tiny planet.
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
                for step in 0..=steps {
                    let fov =
                        FOV_MIN_DEG * (ceiling / FOV_MIN_DEG).powf(step as f32 / steps as f32);
                    let theta = off_axis(screen(fov, aspect), uv);
                    if let Some(held) = held {
                        assert!(
                            theta >= held - 1e-5,
                            "{uv:?} looked back in from {held} to {theta} at fov {fov:.1}",
                        );
                    }
                    held = Some(theta);
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
                .map(|&uv| angle_between(normalize(before.ray(uv)), normalize(after.ray(uv))))
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

    /// The far end of the zoom, which is what issue #47 asks for and what the
    /// owner's own test of it corrected: **the tiny planet**. The earth curls
    /// into a ball inside the picture, the sky wraps round it and fills the
    /// corners, and the frame is video edge to edge at every field of view
    /// the zoom offers.
    ///
    /// Three claims, on every window shape. The corner is [`CORNER_MAX`] off
    /// the axis, which is the cap and is short of the half turn the map runs
    /// out at. The horizon circle -- everything level with the camera, which
    /// is the rim of the planet when the view is looking down -- is inside
    /// the frame's shorter side, so the ball really is a ball in the picture
    /// rather than a wide view of the ground. And the corner is further out
    /// than that rim, which is the sky wrapped round it.
    #[test]
    fn the_far_end_is_the_tiny_planet() {
        for aspect in [0.6, 1.0, WIDE] {
            let screen = Screen::new(
                Camera {
                    fov: fov_ceiling(aspect),
                    ..Camera::default()
                },
                aspect,
            );
            near(
                off_axis(screen, [0.0, 0.0]).to_degrees(),
                CORNER_MAX.to_degrees(),
                1e-2,
            );

            // Out from the middle along the frame's shorter side, which is
            // the direction a round picture runs out of frame first.
            let toward = [(1.0 / aspect).min(1.0), aspect.min(1.0)];
            for axis in 0..2 {
                let edge = |at: f32| {
                    let mut uv = [0.5, 0.5];
                    uv[axis] += at * toward[axis];
                    uv
                };
                let rim = off_axis(screen, edge(0.5)).to_degrees();
                assert!(
                    rim > 90.0,
                    "the horizon is outside the frame at aspect {aspect}: the shorter side \
                     reaches only {rim} degrees, so the planet has no sky around it",
                );
            }
        }
    }

    /// A window that narrows after the scroll has a corner further out than
    /// the one the scroll was clamped against, so the map itself holds the
    /// ceiling as well: the frame stays full of video through a resize that
    /// nobody zoomed for.
    #[test]
    fn a_window_that_narrows_keeps_the_frame_full() {
        let widest = Camera {
            fov: fov_ceiling(21.0 / 9.0),
            ..Camera::default()
        };
        for aspect in [0.5, 0.6, 1.0, WIDE] {
            let corner = off_axis(Screen::new(widest, aspect), [0.0, 0.0]);
            near(corner.to_degrees(), CORNER_MAX.to_degrees(), 1e-2);
        }
    }

    /// The widest view holds all but a cap of the sphere at once, which is
    /// the first time one pass has had to: **every pixel of it is picture**,
    /// the seam blend still sums to one across it, and the far side of each
    /// lens is carried by the other one rather than by the fold the model
    /// would otherwise land there (issue #30's guard, now on the hot path).
    ///
    /// The no-grey claim is this test: `is_covered` is exactly what the
    /// shader paints [`OUTSIDE_GRAY`] for, and nothing in the frame answers
    /// false.
    #[test]
    fn every_pixel_of_the_widest_view_is_picture() {
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
        let (mut lit, mut furthest) = (0, 0.0f32);

        for down in 0..=120 {
            for across in 0..=120 {
                let uv = [across as f32 / 120.0, down as f32 / 120.0];
                let ray = reframe.view_ray(uv);
                lit += 1;
                let blend = reframe.blend(ray);
                assert!(
                    blend.is_covered(),
                    "no lens has {uv:?}, which would be grey"
                );
                near(blend.weights.iter().sum(), 1.0, 1e-5);
                for lens in 0..MAX_LENSES {
                    let landing = blend.landings[lens];
                    assert!(
                        blend.weights[lens] == 0.0 || landing.inside,
                        "the widest view is showing a folded landing at {uv:?}",
                    );
                }
                furthest = furthest.max(normalize(ray)[2].clamp(-1.0, 1.0).acos());
            }
        }

        assert_eq!(lit, 121 * 121, "the whole frame was not walked");
        near(furthest.to_degrees(), CORNER_MAX.to_degrees(), 1e-2);
    }

    /// The size the WGSL struct rounds up to, which is what the bind group
    /// declares as `min_binding_size`: pipeline creation is where a
    /// disagreement between the two definitions surfaces.
    #[test]
    fn the_uniform_block_is_the_size_wgsl_lays_it_out() {
        assert_eq!(std::mem::size_of::<LensBlock>(), 112);
        assert_eq!(std::mem::size_of::<Screen>(), 16);
        assert_eq!(std::mem::size_of::<Reframe>(), 288);
    }

    fn radius(reframe: &Reframe, lens: usize, landing: Landing) -> f32 {
        let block = &reframe.lenses[lens];
        norm([landing.pixel[0] - block.cx, landing.pixel[1] - block.cy])
    }
}
