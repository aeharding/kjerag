//! What the two lenses still disagree about after the calibration, measured
//! on every frame the pass draws, and carried into the pass as a bend
//! (issue #103, stage 2).
//!
//! [`super::seam`] takes out what belongs to the **camera**: a tilt and a
//! principal point, one answer per camera, the same at every azimuth for the
//! life of the file. What it leaves on flight footage is 0.12 to 0.36 degrees
//! along the seam and **0.57 to 0.84 across** it (docs/research/insv-format.md
//! 6.8), and the second of those is not calibration and cannot be: the
//! baseline between the two lenses is 33 mm, so content at 3 m is displaced
//! 0.64 degrees towards the front lens and content at 1 m is displaced 1.9,
//! and no rotation of a lens moves content that is at two distances at once.
//!
//! So it is measured instead, per direction and per frame. The overlap band
//! is a stereo pair: both lenses image the same 14 degrees around the seam
//! from two centres a baseline apart, the disagreement there is a
//! **disparity**, and a disparity is a distance (issue #80 phase A, merged as
//! #94 research, whose instrument `kjerag-spike --bin depth` is where the
//! geometry below was checked against pixels).
//!
//! Three things make it affordable and steady:
//!
//! - **It runs on the GPU, on the textures the pass has already imported.**
//!   Phase A measured the same solve at 30 to 73 ms per frame on one core
//!   against a 33 ms budget, and a playback frame has no CPU pixels at all
//!   (it is a dmabuf the decoder still owns), so a CPU harvest would have to
//!   copy 15 MB a lens before it could start. [`wgsl`] is a compute pass over
//!   [`AZIMUTHS`] directions, one workgroup each.
//! - **The search is one-dimensional and the axis is the file's own.** The
//!   baseline is what makes a stereo pair, so the only axis a distance can
//!   displace content along is the epipolar one, [`Ring::epi`], which is 3.6
//!   degrees off the across-seam tangent every earlier instrument searched
//!   along. The off-epipolar channel is searched narrowly as well and is
//!   never applied: it is the control that says the reading is depth
//!   ([`Cell::off_epi`]).
//! - **The time constant is per direction and depends on the disparity.**
//!   This is the whole of why a per-frame measurement can beat a per-clip
//!   table rather than flicker against it. Far field is where the horizon
//!   lives: disparity near zero, static, and heavy smoothing there is free
//!   and drives the reading's own noise to nothing. Near field is the wing
//!   and the cage, disparity of degrees, moving, and it has to track. One
//!   knee between the two ([`NEAR_KNEE_DEG`]) buys both, and it is the answer
//!   to phase A's finding that a naive per-frame table flickers 0.22 to 0.54
//!   degrees against the 0.2 to 0.4 it was meant to remove.
//!
//! **The bend is four lines and both of its properties are free.** Each
//! lens's ray is bent along the epipolar axis by the **other** lens's blend
//! weight times the disparity. The two bends then differ by exactly the
//! disparity wherever the weights sum to one, so the two lenses agree
//! everywhere in the band; and each lens's own bend is zero wherever its
//! weight is one, so nothing outside the band moves and there is no edge to
//! feather. Neither is arranged: both fall out of the weights
//! [`super::projection::Reframe::blend`] already computes. A file with one
//! lens stream has a front weight of exactly 1 everywhere and therefore no
//! bend anywhere, which is the byte-identity of issue #39 by construction
//! rather than by care.
//!
//! **Nothing here is user-facing** (AGENTS.md, zero-config playback). There
//! is no switch, no menu and no report line a pilot has to read: the state is
//! zero until a direction has been measured, and zero is exactly the picture
//! stage 1 drew.

use kjerag_meta::Lens;

/// How many directions round the seam circle are measured, one compute
/// workgroup each.
///
/// Phase A read 72 and filled the rest from neighbours. A GPU has no reason
/// to be that sparse: 128 is 2.8 degrees apart, which is finer than the
/// patch the correlation reads over ([`SPAN_DEG`]), so neighbouring cells
/// overlap and the field cannot carry a step the picture does not have.
pub const AZIMUTHS: usize = 128;

/// How wide a patch is, in degrees of world angle.
///
/// Phase A's number. Wide enough to hold structure at the scale the seam
/// shows it, narrow enough that the disparity is one value across it: at 2.0
/// degrees, content at 3 m and content at 4 m inside one patch differ by 0.16
/// degrees, which is two correlation steps.
const SPAN_DEG: f32 = 2.0;

/// How finely the correlation is stepped, in degrees. Phase A's number, and
/// the same one [`super::seam::Probe`] uses.
const STEP_DEG: f32 = 0.08;

/// The most disparity the search reports, in degrees, and the least.
///
/// One-sided, because parallax is: the baseline is along the lens axis, so a
/// near subject is displaced **towards the front lens** at every azimuth
/// (6.8). The far side is not zero because the calibration does not land
/// exactly: after the pooled per-camera fit the across-seam residual on
/// flights is 0.57 to 0.84 degrees, and the window has to hold the far-field
/// part of that or the horizon cannot be reached.
const NEAR_DEG: f32 = 3.5;
const FAR_DEG: f32 = -1.2;

/// How far off the epipolar axis the search looks, in degrees.
///
/// Not applied, ever. It is here because the axis the file's own baseline
/// names is a **prediction**: if the band is a stereo pair, the disagreement
/// lies along it and the off-epipolar channel reads near zero. Phase A
/// measured 0.11 to 0.17 degrees off it against 0.56 to 0.98 along it, a
/// ratio of 4.3 to 7.8. Searching it costs [`PERP_STEPS`] times the
/// correlations and buys the control that says the reading is depth, plus
/// tolerance for whatever the calibration left on that axis (0.12 to 0.36
/// degrees, which is why the range is a little wider than the residual).
const PERP_DEG: f32 = 0.48;

/// How many off-epipolar offsets are tried, either side of zero.
///
/// Coarse on purpose: the epipolar peak barely moves with a small
/// misregistration on the near-orthogonal axis, so this axis needs enough
/// steps to find the correlation and not enough to resolve it.
const PERP_STEPS: usize = 2;

/// The correlation a reading has to reach to move the state.
///
/// Below [`super::seam::Probe`]'s 0.80, and deliberately: that gate protects
/// a **fit**, where one bad patch moves five knobs over the whole sphere.
/// This one protects one direction of one frame, the reading is smoothed over
/// many frames before it is worth anything, and a gate too high on a hazy
/// horizon is how the far field goes unmeasured.
const KEEP: f32 = 0.65;

/// How much picture a patch needs, in 8-bit codes of standard deviation.
/// Phase A's number and 6.8's: flat sky correlates with anything.
const CONTRAST: f32 = 6.0;

/// Where a direction stops counting as far field, in degrees of disparity.
///
/// 0.19 degrees is 10 m at this baseline (6.1), which is where a disparity is
/// over a fifth of a degree and the blend stops hiding it, and it is phase
/// A's own `NEAR_M`. Under it the reading is the horizon and whatever else is
/// at infinity, it does not move, and it is smoothed hard. Over it the
/// reading is the wing, the lines or the cage, it moves, and it is tracked.
const NEAR_KNEE_DEG: f32 = 0.19;

/// How long a far-field direction takes to answer a change, in seconds.
///
/// Long, because nothing at infinity moves and the only thing a shorter
/// constant buys there is the correlator's own noise. At 30 fps this averages
/// about sixty readings, which divides a per-reading spread of 0.05 degrees
/// by about eight.
const TAU_FAR_S: f32 = 2.0;

/// How long a near-field direction takes to answer a change, in seconds.
///
/// Short, because the wing does move: a 0.1 s constant is three frames, which
/// tracks a line crossing the seam without following the correlator frame to
/// frame.
const TAU_NEAR_S: f32 = 0.10;

/// How long a direction that has stopped correlating takes to give its
/// reading up, in seconds.
///
/// A direction goes quiet for two reasons and they want the same answer. Sky
/// drifts into it, and sky is at infinity, so zero is right. Or the near
/// object that was there has moved on, and zero is right again, because what
/// is behind it is further away. Holding the last reading instead would print
/// a stale bend on new content, so it decays, and it decays slowly enough
/// that a lens flare or a frame of motion blur costs nothing.
const TAU_STALE_S: f32 = 1.5;

/// How much of the crossover the bend may spend, as a fraction of it.
///
/// The bend varies from zero to the whole disparity across the band, so its
/// own gradient is the disparity divided by the band width: **the shear**.
/// Above 1 the mapping folds and the picture is printed back over itself,
/// which is the fold that decided the crossover could not narrow before the
/// calibration landed (`super::projection::CROSSOVER_DEG`). This is the first
/// time that number is computable at runtime rather than quoted, and it is
/// what a clamp needs. 0.9 leaves the Jacobian at a tenth rather than at
/// nothing, and what it clamps is content nearer than about 1.9 m, which is
/// nearer than the camera maker's own manual asks a subject to be. Widening
/// the band where it bites is stage 4's, not this stage's.
const FOLD: f32 = 0.9;

/// Threads per workgroup. One workgroup reads one direction, and every thread
/// in it scores its share of the candidate shifts.
const THREADS: usize = 64;

/// One direction's state, as the compute pass writes it and the fragment
/// shader reads it.
///
/// Sixteen bytes and every one of them is read by something: the first two by
/// the pass, the second two only by an instrument. Zero is the state a file
/// opens in and the state a direction that has never correlated stays in, and
/// a zero disparity is no bend at all.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cell {
    /// The smoothed disparity, in **radians** along [`Ring::epi`]. Positive
    /// is lens 1's picture displaced towards the front lens, which is what a
    /// near subject does.
    pub disparity: f32,
    /// How well this direction is correlating, smoothed the same way: 0 for a
    /// direction that has never been read, up to 1.
    pub confidence: f32,
    /// How far the baseline reaches at this direction, in metres, so that
    /// `reach_m / disparity` is the distance to what is there. A property of
    /// the geometry rather than of the picture; it is written here so an
    /// instrument reading the buffer back needs nothing else.
    pub reach_m: f32,
    /// What the search read on the axis a distance **cannot** displace
    /// content along, in radians. Never applied. The control: if this is not
    /// far smaller than [`Self::disparity`] where the disparity is large, the
    /// band is not being read as a stereo pair.
    pub off_epi: f32,
}

impl Cell {
    /// The distance to whatever is in this direction, in metres, or `None`
    /// where the disparity is zero or the wrong way round, which is
    /// everything far enough away to be at infinity as far as a 33 mm
    /// baseline is concerned.
    pub fn metres(&self) -> Option<f32> {
        (self.disparity > 0.0).then(|| self.reach_m / self.disparity)
    }
}

/// What the compute pass is told about this frame, over and above the
/// [`Reframe`](super::projection::Reframe) block it shares with the pass.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Watch {
    /// Media time since the frame the state was last updated on, in seconds.
    /// Media time and not wall clock, so a paused window does not age the
    /// state and a slow box does not smooth harder than a fast one.
    pub seconds: f32,
    /// 1 to throw the state away and start from this frame, which is what a
    /// seek, a new file and a first frame all want.
    pub reset: f32,
    _pad: [f32; 2],
}

impl Watch {
    /// The state ages by `seconds` of media time, or starts again.
    ///
    /// A gap that is not a play forward is a `reset`: the state is a running
    /// average over what the seam has been showing, and after a seek it is an
    /// average over somewhere else.
    pub fn new(seconds: f32, reset: bool) -> Self {
        Self {
            seconds,
            reset: f32::from(u8::from(reset)),
            _pad: [0.0; 2],
        }
    }

    /// The longest gap that is still the same stretch of film. Past it the
    /// state is thrown away: two seconds of far-field smoothing is worth
    /// nothing if the two seconds were somewhere else.
    pub const GAP_S: f32 = 0.5;

    pub fn bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                std::ptr::from_ref(self).cast::<u8>(),
                std::mem::size_of::<Self>(),
            )
        }
    }
}

/// The seam circle as the band reads it: one direction, and the axis a
/// distance displaces content along there.
///
/// The **Rust twin** of the shader's own `ring`, and the reason this is a
/// type rather than a comment: an instrument reading the state buffer back
/// has to be able to say what cell 37 is a direction of, and it must say the
/// same thing the shader said.
#[derive(Clone, Copy, Debug)]
pub struct Ring {
    /// Where round the circle, in radians from the body's +x.
    pub phi: f32,
    /// The direction itself, in the camera body's frame.
    pub centre: [f32; 3],
    /// The epipolar axis at this direction, unit, in the body's frame: the
    /// baseline with the part along the view direction taken out, negated
    /// because lens 1 sits **behind** lens 0, so its picture of a near point
    /// is displaced towards the front lens.
    pub epi: [f32; 3],
    /// The axis a distance cannot reach, unit: across the other two.
    pub perp: [f32; 3],
    /// How much of the baseline is seen from this direction, in metres, which
    /// is what turns a disparity into a distance.
    pub reach_m: f32,
}

impl Ring {
    /// Cell `index` of [`AZIMUTHS`], for a camera whose second lens sits at
    /// `baseline` metres from its first.
    pub fn cell(index: usize, baseline: [f32; 3]) -> Self {
        Self::of(
            index as f32 / AZIMUTHS as f32 * std::f32::consts::TAU,
            baseline,
        )
    }

    /// The same at any azimuth. WGSL twin: `ring_of`.
    pub fn of(phi: f32, baseline: [f32; 3]) -> Self {
        let (sin, cos) = phi.sin_cos();
        Self::at([cos, sin, 0.0], baseline)
    }

    /// The same for a direction on the circle already in hand, which is what
    /// a fragment has: the ray's own azimuth is that direction flattened into
    /// the seam plane, so no trig is needed to reach it, and a ray between two
    /// cells takes the axis of where it actually is rather than of the nearer
    /// cell.
    ///
    /// WGSL twin: `ring_at`.
    pub fn at(centre: [f32; 3], baseline: [f32; 3]) -> Self {
        let phi = centre[1].atan2(centre[0]);
        let along = dot(baseline, centre);
        let seen: [f32; 3] = std::array::from_fn(|axis| baseline[axis] - along * centre[axis]);
        let reach_m = norm(seen);
        let epi = unit([-seen[0], -seen[1], -seen[2]]);
        Self {
            phi,
            centre,
            epi,
            perp: unit(cross(centre, epi)),
            reach_m,
        }
    }
}

/// Where the second lens sits, in the camera body's frame, in metres.
///
/// 33 mm of it is along the body's z on every camera in the format study,
/// which is what makes the seam a stereo pair at all. A file with one lens
/// stream has no second pose and no baseline, and everything above then reads
/// zero, which switches the band off rather than dividing by it.
pub fn baseline(lenses: &[Lens]) -> [f32; 3] {
    lenses
        .get(1)
        .map_or([0.0; 3], |lens| lens.pose.translation_m.map(|c| c as f32))
}

/// How fast one direction answers a change, in seconds, from what it is
/// currently reading.
///
/// **The whole of stage 2's temporal design is this function.** It is read
/// off the smoothed state rather than off the new reading, on purpose: a
/// noisy reading on far-field content would otherwise look near for one frame
/// and unlock the smoothing that was keeping it still, which is the failure
/// the constant is supposed to prevent.
///
/// WGSL twin: `time_constant`.
pub fn time_constant(disparity_rad: f32) -> f32 {
    // A straight line in the disparity between the two knees, so a direction
    // that drifts from far to near does not change character at a step. The
    // far knee is zero and the near one is NEAR_KNEE_DEG.
    let near = (disparity_rad.abs() / NEAR_KNEE_DEG.to_radians()).clamp(0.0, 1.0);
    TAU_FAR_S + (TAU_NEAR_S - TAU_FAR_S) * near
}

/// One step of the exponential filter: how much of the new reading to take,
/// over `seconds` of media time, at a time constant of `tau`.
///
/// Per second rather than per frame, so a file at 24 fps and a file at 60
/// settle in the same wall time, and a stretch the decoder dropped frames in
/// does not settle slower for it.
///
/// WGSL twin: `ease`.
pub fn ease(seconds: f32, tau: f32) -> f32 {
    match seconds > 0.0 {
        true => seconds / (tau + seconds),
        false => 0.0,
    }
}

/// The disparity the shader may actually bend by, in radians: what was
/// measured, clamped to what the crossover can carry without folding.
///
/// `band` is the crossover width in radians. See [`FOLD`].
///
/// WGSL twin: `carried`.
pub fn carried(disparity_rad: f32, band_rad: f32) -> f32 {
    let limit = FOLD * band_rad;
    disparity_rad.clamp(-limit, limit)
}

// ------------------------------------------------------------ the shader

/// The compute half: one workgroup per direction, reading both lenses'
/// luma and writing one [`Cell`].
///
/// It is concatenated after `super::projection::wgsl`, whose `Reframe` block,
/// `mei` and helpers it reads, and after `super::sampling::wgsl`. What it
/// adds is its own bindings and its own entry point.
pub(crate) fn wgsl() -> String {
    let span = (SPAN_DEG / STEP_DEG).round() as usize;
    // An odd count, so a patch has a centre sample.
    let half = span / 2;
    let near = (NEAR_DEG / STEP_DEG).round() as isize;
    let far = (FAR_DEG / STEP_DEG).round() as isize;
    let perp = (PERP_DEG / STEP_DEG / PERP_STEPS as f32).round() as isize;
    let epi_shifts = (near - far + 1) as usize;
    let perp_shifts = 2 * PERP_STEPS + 1;
    format!(
        "const AZIMUTHS = {AZIMUTHS}u;\n\
         const THREADS = {THREADS}u;\n\
         const HALF = {half}i;\n\
         const STEP = {step:?};\n\
         const EPI_FAR = {far}i;\n\
         const EPI_NEAR = {near}i;\n\
         const EPI_SHIFTS = {epi_shifts}u;\n\
         const PERP_STEP = {perp}i;\n\
         const PERP_STEPS = {perp_steps}i;\n\
         const PERP_SHIFTS = {perp_shifts}u;\n\
         const KEEP = {keep:?};\n\
         const CONTRAST = {contrast:?};\n\
         const NEAR_KNEE = {knee:?};\n\
         const TAU_FAR = {far_s:?};\n\
         const TAU_NEAR = {near_s:?};\n\
         const TAU_STALE = {stale:?};\n\
         const FOLD = {fold:?};\n\
         const PATCH = {patch}u;\n\
         const BACK_ALONG = {back_along}u;\n\
         const BACK_ACROSS = {back_across}u;\n\
         const TAU = {tau:?};\n\
         {CELL}{RING}{WGSL}",
        tau = std::f32::consts::TAU,
        step = STEP_DEG.to_radians(),
        perp_steps = PERP_STEPS,
        keep = KEEP,
        contrast = CONTRAST / 255.0,
        knee = NEAR_KNEE_DEG.to_radians(),
        far_s = TAU_FAR_S,
        near_s = TAU_NEAR_S,
        stale = TAU_STALE_S,
        fold = FOLD,
        patch = (2 * half + 1) * (2 * half + 1),
        back_along = (2 * half + 1) as isize + 2 * PERP_STEPS as isize * perp,
        back_across = (2 * half + 1) as isize + near - far,
    )
}

/// The lookup half, which the fragment shader reads: the bend one ray takes.
///
/// Separate from [`wgsl`] because the two pipelines want different halves.
/// The render pass never runs the correlation and the compute pass never
/// bends a ray, and each declares the storage buffer with the access it
/// needs: `read` in the fragment shader, `read_write` in the compute one.
pub(crate) fn lookup_wgsl() -> String {
    format!(
        "const AZIMUTHS = {AZIMUTHS}u;\nconst FOLD = {FOLD:?};\nconst TAU = {:?};\n{CELL}{RING}{LOOKUP}",
        std::f32::consts::TAU,
    )
}

/// The state buffer's binding, on a group of its own.
///
/// A group of its own for two reasons and either would be enough. The same
/// buffer is `read` on the draw's side and `read_write` on the band's, and a
/// bind group layout declares one or the other; and wgpu refuses a dispatch
/// that carries both usages of one buffer at once, which is what the first
/// version of this did and what it says when it does
/// (`Attempted to use Buffer with conflicting usages`). Two layouts and two
/// bind groups over one buffer, bound in two different passes, is the whole
/// of the fix.
pub(crate) const STATE_BINDING: u32 = 0;

/// Where the compute pass reads [`Watch`]. Compute only: the draw has no use
/// for how old the state is.
pub(crate) const WATCH_BINDING: u32 = 1;

/// How many bytes the state buffer is.
pub(crate) const BYTES: u64 = (AZIMUTHS * std::mem::size_of::<Cell>()) as u64;

/// How many workgroups one frame's measurement dispatches: one per direction.
pub(crate) const GROUPS: u32 = AZIMUTHS as u32;

/// Declared by both shaders, with the access each needs.
const CELL: &str = r#"
struct Cell {
  disparity: f32,
  confidence: f32,
  reach_m: f32,
  off_epi: f32,
};
"#;

/// The seam circle's geometry, shared by both shaders. Rust twin: `Ring`.
const RING: &str = r#"
struct Ring {
  centre: vec3<f32>,
  epi: vec3<f32>,
  perp: vec3<f32>,
  reach_m: f32,
};

// Rust twin: `Ring::of`.
fn ring_of(phi: f32) -> Ring {
  let centre = vec3<f32>(cos(phi), sin(phi), 0.0);
  return ring_at(centre);
}

// The same for a direction already in hand, which is what a fragment has: the
// ray's own azimuth is the direction, normalized into the seam plane, so no
// trig is needed to reach it.
fn ring_at(centre: vec3<f32>) -> Ring {
  let baseline = vec3<f32>(reframe.baseline_x, reframe.baseline_y, reframe.baseline_z);
  let seen = baseline - dot(baseline, centre) * centre;
  var out: Ring;
  out.centre = centre;
  out.reach_m = length(seen);
  out.epi = select(vec3<f32>(0.0), -seen / out.reach_m, out.reach_m > 0.0);
  out.perp = normalize(cross(centre, out.epi));
  return out;
}
"#;

const LOOKUP: &str = r#"
@group(1) @binding(0) var<storage, read> band: array<Cell, AZIMUTHS>;

// The bend a ray takes, in view space, scaled by the ray's own length so that
// adding it turns the ray by the disparity in radians. Zero everywhere the
// band has never been measured, which is a file with one lens stream, a file
// still on its first frame, and every direction with nothing in it to
// correlate. Rust twin: `Reframe::bend`.
fn band_bend(ray: vec3<f32>) -> vec3<f32> {
  let body = reframe.view_to_body * ray;
  let flat = vec2<f32>(body.x, body.y);
  let reach = length(flat);
  if reach <= 0.0 {
    // Straight down a lens's own axis, where there is no seam and no azimuth.
    return vec3<f32>(0.0);
  }
  let at = ring_at(vec3<f32>(flat / reach, 0.0));
  // Between two cells, linearly, wrapping: the field is a circle and a step
  // between neighbouring cells would be a step in the picture.
  let turn = atan2(body.y, body.x) / TAU * f32(AZIMUTHS);
  let low = i32(floor(turn));
  let mix = turn - f32(low);
  let a = band[u32(low + i32(AZIMUTHS)) % AZIMUTHS];
  let b = band[u32(low + 1 + i32(AZIMUTHS)) % AZIMUTHS];
  let disparity = mix2(a.disparity, b.disparity, mix);
  // The bend's own gradient across the band is the disparity over the band
  // width, and past 1 the mapping folds. Rust twin: `carried`.
  let limit = FOLD * CROSSOVER;
  let carried = clamp(disparity, -limit, limit);
  // Back into view space: view_to_body is a rotation, so its transpose is its
  // inverse, and `v * m` is `transpose(m) * v`.
  return (carried * length(ray)) * (at.epi * reframe.view_to_body);
}

fn mix2(a: f32, b: f32, t: f32) -> f32 {
  return a + (b - a) * t;
}
"#;

const WGSL: &str = r#"
// The same group the draw binds, so the band correlates the very pictures the
// frame after it will sample. The chroma planes are not declared: a doubled
// edge is geometry and geometry is in the luma, and a bind group may carry
// bindings a shader has no use for.
@group(0) @binding(1) var luma0: texture_2d<f32>;
@group(0) @binding(3) var luma1: texture_2d<f32>;
@group(0) @binding(5) var samp: sampler;

// A group of its own, because the two pipelines want the same buffer with
// different access: `read` in the fragment shader, `read_write` here, and a
// bind group layout declares one or the other. It also keeps a writable
// storage buffer out of the fragment stage, which not every device allows.
@group(1) @binding(0) var<storage, read_write> band: array<Cell, AZIMUTHS>;
@group(1) @binding(1) var<uniform> watch: Watch;

// Luma only. A doubled edge is geometry and geometry is in the luma, and the
// chroma planes are a quarter of the resolution the correlation wants.
fn luma_at(index: u32, uv: vec2<f32>) -> f32 {
  if index == 0u {
    return textureSampleLevel(luma0, samp, uv, 0.0).r;
  }
  return textureSampleLevel(luma1, samp, uv, 0.0).r;
}

struct Watch {
  seconds: f32,
  reset: f32,
  pad0: f32,
  pad1: f32,
};

// One lens's picture of the patch, and the other's picture of the patch plus
// everywhere the search may slide it to. Both in workgroup memory, because a
// candidate shift re-reads the same samples and a texture fetch per candidate
// per sample is a hundred times the work.
var<workgroup> front: array<f32, PATCH>;
var<workgroup> back: array<f32, BACK_ALONG * BACK_ACROSS>;
// One score per candidate shift, so the peak and its two neighbours are all
// in hand when the parabola is taken and no shift is scored twice.
var<workgroup> scores: array<f32, EPI_SHIFTS * PERP_SHIFTS>;

// The map the band is read through: the camera left where it stands and the
// view pointed nowhere, so a direction is a direction in the body's own frame
// and both lenses answer about the same one. Rust twin: `seam::mapped`.
//
// No readout correction in it, and that is a measurement rather than an
// omission: both lenses read down the delivered frame, which is one world
// direction, so the readout moves no content across the seam at all
// (docs/research/insv-format.md 6.7).
fn body_to_lens(index: u32) -> mat3x3<f32> {
  return reframe.lenses[index].view_to_lens * transpose(reframe.view_to_body);
}

// Where one lens sees a direction, in its own delivered frame. `inside` false
// means this lens has no picture of it.
fn look(index: u32, aim: mat3x3<f32>, ray: vec3<f32>) -> Landing {
  return mei(reframe.lenses[index], normalize(aim * ray));
}

// One sample of one lens's picture, in luma, or a negative number where the
// lens has no picture there. Negative rather than zero because zero is black
// and black is a picture.
fn tap(index: u32, aim: mat3x3<f32>, ray: vec3<f32>) -> f32 {
  let landing = look(index, aim, ray);
  if !landing.inside {
    return -1.0;
  }
  return luma_at(index, frame_uv(landing.pixel));
}

@compute @workgroup_size(THREADS)
fn measure(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
  let cell = group.x;
  let at = ring_of(f32(cell) / f32(AZIMUTHS) * TAU);
  let aim0 = body_to_lens(0u);
  let aim1 = body_to_lens(1u);

  // Both grids, cooperatively. The front lens's is the patch itself; the back
  // lens's is the patch widened by everywhere the search may slide it.
  for (var i = lane; i < PATCH; i += THREADS) {
    let a = f32(i32(i % u32(2 * HALF + 1)) - HALF) * STEP;
    let b = f32(i32(i / u32(2 * HALF + 1)) - HALF) * STEP;
    front[i] = tap(0u, aim0, at.centre + a * at.perp + b * at.epi);
  }
  for (var i = lane; i < BACK_ALONG * BACK_ACROSS; i += THREADS) {
    let a = f32(i32(i % BACK_ALONG) - HALF - PERP_STEPS * PERP_STEP) * STEP;
    let b = f32(i32(i / BACK_ALONG) - HALF + EPI_FAR) * STEP;
    back[i] = tap(1u, aim1, at.centre + a * at.perp + b * at.epi);
  }
  workgroupBarrier();

  for (var i = lane; i < EPI_SHIFTS * PERP_SHIFTS; i += THREADS) {
    scores[i] = correlate(i);
  }
  workgroupBarrier();

  if lane == 0u {
    settle(cell, at);
  }
}

// Zero-mean normalized cross-correlation of the two patches at candidate
// shift `i`, or -2 where either patch is short of picture. -2 rather than -1
// so that "no answer" loses to any answer, including a perfectly
// anti-correlated one.
fn correlate(i: u32) -> f32 {
  let epi = i / PERP_SHIFTS;
  let perp = i % PERP_SHIFTS;
  var sum_a = 0.0;
  var sum_b = 0.0;
  var sum_aa = 0.0;
  var sum_bb = 0.0;
  var sum_ab = 0.0;
  var count = 0.0;
  for (var row = 0u; row < u32(2 * HALF + 1); row += 1u) {
    let source = (row + epi) * BACK_ALONG + perp * u32(PERP_STEP);
    for (var column = 0u; column < u32(2 * HALF + 1); column += 1u) {
      let a = front[row * u32(2 * HALF + 1) + column];
      let b = back[source + column];
      if a < 0.0 || b < 0.0 {
        return -2.0;
      }
      sum_a += a;
      sum_b += b;
      sum_aa += a * a;
      sum_bb += b * b;
      sum_ab += a * b;
      count += 1.0;
    }
  }
  let var_a = sum_aa - sum_a * sum_a / count;
  let var_b = sum_bb - sum_b * sum_b / count;
  if var_a <= 0.0 || var_b <= 0.0 {
    return -2.0;
  }
  // The front patch's own contrast is the gate that keeps flat sky out: it
  // correlates with anything, and what it correlates with is noise.
  if sqrt(var_a / count) < CONTRAST {
    return -2.0;
  }
  return (sum_ab - sum_a * sum_b / count) / sqrt(var_a * var_b);
}

// The peak, the gates, and one step of the filter. One thread, because it is
// a few dozen operations over a table the whole workgroup has already filled.
fn settle(cell: u32, at: Ring) {
  var held = band[cell];
  if watch.reset != 0.0 {
    held = Cell(0.0, 0.0, at.reach_m, 0.0);
  }
  held.reach_m = at.reach_m;

  var best = -2.0;
  var found = 0u;
  for (var i = 0u; i < EPI_SHIFTS * PERP_SHIFTS; i += 1u) {
    if scores[i] > best {
      best = scores[i];
      found = i;
    }
  }
  let epi = i32(found / PERP_SHIFTS);
  let perp = i32(found % PERP_SHIFTS) - PERP_STEPS;
  // A peak against the edge of the search is not a peak, it is the search
  // running out: near-field content moves further across than the band is
  // wide, and a reading pinned at the limit would report the limit.
  let pinned = epi == 0 || epi == i32(EPI_SHIFTS) - 1;
  if best < KEEP || pinned {
    // Nothing to read here this frame. What was read before gives itself up
    // slowly rather than being held: whatever moved into this direction is
    // further away than what left it.
    let stale = ease(watch.seconds, TAU_STALE);
    held.disparity -= held.disparity * stale;
    held.confidence -= held.confidence * stale;
    band[cell] = held;
    return;
  }

  // Between whole steps, because a third of a step is exactly the size this
  // is trying to resolve. Rust twin: `super::seam::best_shift`'s `peak`.
  let minus = scores[found - PERP_SHIFTS];
  let plus = scores[found + PERP_SHIFTS];
  let curve = minus - 2.0 * best + plus;
  var refined = 0.0;
  if curve < 0.0 {
    refined = clamp(0.5 * (minus - plus) / curve, -1.0, 1.0);
  }
  let read = (f32(epi + EPI_FAR) + refined) * STEP;

  // The time constant is read off what this direction has been showing, not
  // off what it showed this frame: a noisy far-field reading must not unlock
  // the smoothing that is keeping the horizon still. Rust twin:
  // `time_constant`.
  //
  // The first frame of a file, and the first after a seek, take the reading
  // whole instead. There is no picture behind them to move under, so there is
  // nothing for an ease to hide, and easing anyway would leave the first two
  // seconds of film drawn with a correction of nearly nothing. The same
  // argument, and the same answer, as `seam::Correction::land`.
  let step = select(ease(watch.seconds, time_constant(held.disparity)), 1.0, watch.reset != 0.0);
  held.disparity += (read - held.disparity) * step;
  held.confidence += (best - held.confidence) * step;
  held.off_epi = f32(perp * PERP_STEP) * STEP;
  band[cell] = held;
}

// Rust twin: `time_constant`.
fn time_constant(disparity: f32) -> f32 {
  let near = clamp(abs(disparity) / NEAR_KNEE, 0.0, 1.0);
  return TAU_FAR + (TAU_NEAR - TAU_FAR) * near;
}

// Rust twin: `ease`.
fn ease(seconds: f32, tau: f32) -> f32 {
  if seconds <= 0.0 {
    return 0.0;
  }
  return seconds / (tau + seconds);
}
"#;

// ------------------------------------------------------------ arithmetic

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|axis| a[axis] * b[axis]).sum()
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(v: [f32; 3]) -> f32 {
    dot(v, v).sqrt()
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = norm(v);
    match length > 0.0 {
        true => v.map(|c| c / length),
        false => [0.0; 3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixture's own baseline: 33 mm, dominated by z.
    const BASELINE: [f32; 3] = [0.000_2, -0.000_1, -0.033_284];

    #[test]
    fn a_distance_displaces_content_towards_the_front_lens_at_every_azimuth() {
        // The one-signedness that tells parallax from a residual rotation
        // (6.8). The epipolar axis is the baseline's own, so it turns with the
        // azimuth; what must not turn is which way it points relative to the
        // front lens, which is the sign of its z.
        for index in 0..AZIMUTHS {
            let at = Ring::cell(index, BASELINE);
            assert!(
                at.epi[2] > 0.9,
                "cell {index} points its epipolar axis at {:?}",
                at.epi,
            );
        }
    }

    #[test]
    fn the_epipolar_axis_is_not_the_across_seam_tangent() {
        // Phase A's finding, which is why this file carries a baseline at all:
        // the axis the file's own geometry names is a few degrees off the
        // across-seam tangent every earlier instrument searched along.
        let across = [0.0, 0.0, 1.0];
        let worst = (0..AZIMUTHS)
            .map(|index| {
                let at = Ring::cell(index, BASELINE);
                dot(at.epi, across).clamp(-1.0, 1.0).acos().to_degrees()
            })
            .fold(0.0f32, f32::max);
        assert!(
            (0.1..10.0).contains(&worst),
            "the two axes are {worst:.2} deg apart at worst",
        );
    }

    #[test]
    fn the_three_axes_are_orthonormal_everywhere() {
        for index in 0..AZIMUTHS {
            let at = Ring::cell(index, BASELINE);
            for pair in [(at.centre, at.epi), (at.epi, at.perp), (at.perp, at.centre)] {
                assert!(
                    dot(pair.0, pair.1).abs() < 1e-4,
                    "cell {index}: {:?} against {:?}",
                    pair.0,
                    pair.1,
                );
            }
            assert!((norm(at.epi) - 1.0).abs() < 1e-5);
            assert!((norm(at.perp) - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn a_file_with_one_lens_stream_has_no_baseline_and_no_band() {
        let at = Ring::of(0.7, baseline(&[]));
        assert_eq!(at.reach_m, 0.0);
        assert_eq!(at.epi, [0.0; 3]);
    }

    #[test]
    fn the_far_field_is_smoothed_twenty_times_harder_than_the_near_field() {
        // The whole of the temporal design: what makes the horizon still is
        // that a direction reading nothing takes seconds to move and one
        // reading degrees takes a tenth of one.
        let far = time_constant(0.0);
        let near = time_constant(2.0f32.to_radians());
        assert!(far / near > 15.0, "{far} against {near}");
        // And nothing between them steps: the constant is monotone in the
        // disparity, so a direction drifting from far to near does not change
        // character at a jump.
        let mut last = f32::MAX;
        for step in 0..64 {
            let tau = time_constant(step as f32 * 0.001);
            assert!(tau <= last + 1e-6, "step {step}: {tau} after {last}");
            last = tau;
        }
    }

    #[test]
    fn the_filter_settles_at_the_time_constant_it_was_given() {
        // A one-pole filter is at 1 - 1/e of a step after one time constant.
        // Run at 30 fps, which is the footage's own rate, so this measures the
        // per-second constant rather than a per-frame one.
        // Stepped finely rather than at 30 fps, because the near constant is
        // three frames long and a three-step approximation of an exponential
        // is not one. What is being checked is the continuous law the filter
        // implements; how coarsely a 30 fps file samples it is a separate
        // question and the answer is in `ease`'s own docstring.
        for tau in [TAU_NEAR_S, TAU_FAR_S] {
            let mut held = 0.0f32;
            let steps = 1000;
            for _ in 0..steps {
                held += (1.0 - held) * ease(tau / steps as f32, tau);
            }
            assert!(
                (held - 0.632).abs() < 0.01,
                "at tau {tau} the filter reached {held}",
            );
        }
    }

    #[test]
    fn the_filter_is_paced_by_media_time_and_not_by_frames() {
        // The same second of film, at two frame rates, has to land in the same
        // place: a 60 fps file must not smooth twice as hard as a 30 fps one.
        let settle = |rate: f32| {
            let mut held = 0.0f32;
            for _ in 0..(rate as usize) {
                held += (1.0 - held) * ease(1.0 / rate, TAU_FAR_S);
            }
            held
        };
        assert!((settle(30.0) - settle(60.0)).abs() < 0.01);
    }

    #[test]
    fn the_bend_never_folds_the_crossover() {
        // Shear is the disparity over the band width and above 1 the mapping
        // prints the picture back over itself. What the clamp has to promise
        // is that the Jacobian stays positive at any disparity the search can
        // report, the near limit included.
        let band = 2.0f32.to_radians();
        for degrees in [-10.0f32, -1.2, 0.0, 0.19, 1.9, 3.5, 100.0] {
            let shear = carried(degrees.to_radians(), band) / band;
            assert!(
                (1.0 + shear) > 0.05,
                "{degrees} deg leaves a Jacobian of {:.3}",
                1.0 + shear,
            );
        }
    }

    #[test]
    fn the_search_window_holds_what_the_calibration_leaves() {
        // The far side of the window is not a taste: the pooled per-camera fit
        // leaves 0.57 to 0.84 deg across the seam on flights (6.8), and a
        // window that cannot reach the far-field part of that cannot reach the
        // horizon either.
        const {
            assert!(FAR_DEG <= -0.9);
        }
        // And the near side has to hold what the crossover can carry, or the
        // clamp is never what decides.
        const {
            assert!(NEAR_DEG > FOLD * 2.0);
        }
    }

    #[test]
    fn the_workgroup_memory_fits_the_smallest_device_wgpu_will_hand_us() {
        // iced asks for `Limits::default()` and falls back to
        // `downlevel_defaults()`, whose workgroup storage is 16352 bytes
        // (`iced_wgpu::window::compositor`). A pipeline that wants more does
        // not draw slower, it fails to create, so the arithmetic is checked
        // here rather than discovered on a weaker box.
        let span = (SPAN_DEG / STEP_DEG).round() as usize;
        let half = span / 2;
        let patch = (2 * half + 1) * (2 * half + 1);
        let near = (NEAR_DEG / STEP_DEG).round() as isize;
        let far = (FAR_DEG / STEP_DEG).round() as isize;
        let perp = (PERP_DEG / STEP_DEG / PERP_STEPS as f32).round() as isize;
        let back = ((2 * half + 1) as isize + 2 * PERP_STEPS as isize * perp)
            * ((2 * half + 1) as isize + near - far);
        let shifts = (near - far + 1) as usize * (2 * PERP_STEPS + 1);
        let bytes = 4 * (patch + back as usize + shifts);
        assert!(
            bytes <= 16352,
            "the workgroup wants {bytes} bytes of shared memory",
        );
    }
}
