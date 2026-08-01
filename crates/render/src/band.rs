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

/// How finely the correlation is stepped, in degrees.
///
/// Phase A and [`super::seam::Probe`] both read at 0.08. This is a little
/// coarser because the grid is what the pass **fetches**, and the fetch is the
/// whole cost: measured on the shipped pass, the correlation itself is free
/// and filling the two grids is not. A step buys resolution quadratically and
/// costs it quadratically, and what actually resolves the answer is the
/// parabola between whole steps and the seconds of averaging over it, neither
/// of which the step bounds.
const STEP_DEG: f32 = 0.10;

/// The most disparity the search reports, in degrees, and the least.
///
/// One-sided, because parallax is: the baseline is along the lens axis, so a
/// near subject is displaced **towards the front lens** at every azimuth
/// (6.8). The far side is not zero because the calibration does not land
/// exactly: after the pooled per-camera fit the across-seam residual on
/// flights is 0.57 to 0.84 degrees, and the window has to hold the far-field
/// part of that or the horizon cannot be reached.
///
/// The near side is set by what the bend can **carry**, not by what a lens can
/// see. Since stage 4 the band opens to carry whatever the search reports
/// ([`width`]), so this is now the near end of the whole correction: 2.6
/// degrees is content at 0.73 m, and a reading that peaks against the edge of
/// the window is refused for being pinned rather than reported at the limit.
/// Widening it costs correlations, which are the pass's cheap half; widening
/// the two grids the correlations read is what is expensive.
const NEAR_DEG: f32 = 2.6;
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
const PERP_DEG: f32 = 0.30;

/// How many off-epipolar offsets are tried, either side of zero.
///
/// Coarse on purpose: the epipolar peak barely moves with a small
/// misregistration on the near-orthogonal axis, so this axis needs enough
/// steps to find the correlation and not enough to resolve it.
const PERP_STEPS: usize = 1;

/// The correlation a reading has to reach to move the state.
///
/// Below [`super::seam::Probe`]'s 0.80, and deliberately: that gate protects
/// a **fit**, where one bad patch moves five knobs over the whole sphere.
/// This one protects one direction of one frame, the reading is smoothed over
/// many frames before it is worth anything, and a gate too high on a hazy
/// horizon is how the far field goes unmeasured.
pub const KEEP: f32 = 0.65;

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

/// How much of the crossover the bend may spend, as a fraction of it.
///
/// The bend varies from zero to the whole disparity across the band, so its
/// own gradient is the disparity divided by the band width: **the shear**.
/// Above 1 the mapping folds and the picture is printed back over itself,
/// which is the fold that decided the crossover could not narrow before the
/// calibration landed (`super::projection::CROSSOVER_DEG`). 0.9 leaves the
/// Jacobian at a tenth rather than at nothing.
///
/// **One inequality, read two ways** (issue #103, stage 4). `|disparity| <=
/// FOLD * width` is the whole of it. Stage 2 held the width at the fixed
/// 2-degree crossover and solved for the disparity, which is [`carried`], and
/// so threw alignment away on everything nearer than 1.06 m. Stage 4 solves
/// the same line for the width, which is [`width`], and throws nothing away
/// until the width runs out of room. The clamp is still here and still the
/// guarantee; it is simply no longer what decides, because the band opens
/// first.
const FOLD: f32 = 0.9;

/// The widest the crossover may open, in degrees.
///
/// It is not a taste and not a margin: it is the widest width the inequality
/// above can ever **ask for**. The search reports at most [`NEAR_DEG`] and
/// refuses anything that peaks against that edge, so `|disparity| / FOLD`
/// cannot exceed this, and a band opened past it would be carrying a reading
/// no frame can produce. Two consequences worth saying out loud: the clamp is
/// inert for every disparity this pass can measure, and widening the search
/// window widens the band with it, with no second number to keep in step.
///
/// What bounds it from the other side is the optics, and that bound is not
/// close. The widest band plus the whole bend it carries reaches **4.04
/// degrees** off the seam; the two lenses of the calibration fixture overlap
/// by 14.44, which is 7.22 a side
/// (`the_widest_band_and_its_bend_stay_inside_the_overlap`, which measures it
/// off the file's own calibration rather than quoting the format study).
/// `kjerag-spike --bin band` reports the same two numbers for whatever file it
/// is given, because the overlap is a property of the camera and this ceiling
/// is not.
pub const WIDEST_DEG: f32 = NEAR_DEG / FOLD;

/// Threads per workgroup. One workgroup reads one direction, and every thread
/// in it scores its share of the candidate shifts.
const THREADS: usize = 64;

/// How many frames it takes to read the whole circle.
///
/// Every frame measures every SLICES-th direction, starting one further round
/// each time, so a direction is read every SLICES frames and the ring is
/// covered continuously rather than in bursts. **Nothing about a reading
/// changes**; only how often each one is taken, and that is the one axis the
/// temporal regularizer was built to trade on: the filter is paced in seconds
/// of media time, so a direction read at 15 Hz and one read at 30 settle in
/// the same wall time and only the near field notices.
///
/// What it buys is the cost. The pass's whole expense is fetching the two
/// grids, and this halves how many are fetched per frame: measured at 2560x1440
/// under live decode, 3.0 ms per redraw reading the whole ring every frame
/// against 0.9 reading half of it.
const SLICES: u32 = 2;

/// How long the pooled exposure gain takes to answer a change, in seconds.
///
/// [`TAU_FAR_S`] and not a constant of its own, for the reason that constant
/// exists: it is what this file smooths things that **do not move** by, and
/// the ratio between two lenses' auto-exposure loops is one of them. Each is
/// a slow closed loop on a whole hemisphere's worth of light, and the reading
/// is already pooled over up to [`AZIMUTHS`] directions before it is smoothed
/// at all.
///
/// A gain has more reason to be smoothed hard than a bend does: a bend that
/// flickers moves the picture inside the crossover and a gain that flickers
/// changes the brightness of **everything**, which is the one artifact worse
/// than the step it is correcting (issue #103, stage 3).
const TAU_GAIN_S: f32 = TAU_FAR_S;

/// The widest gain that is an exposure difference rather than a measurement
/// coming apart, as a natural log.
///
/// Not a taste and not a margin, and derived the way [`super::seam`]'s
/// `RUNAWAY_DEG` is: measured across every capture on this box and every
/// camera in the sample corpus, the pooled reading spans -0.033 to +0.036 ln,
/// which is 3.6 percent, and no single azimuth-frame of any of them reaches
/// 0.14. This is four times the widest pooled reading and above every single
/// reading, so nothing this stage measured is clipped by it; what it is here
/// for is the case none of those captures is - a seam correlating on
/// something that is not the same content at all, which would otherwise reach
/// the picture as a hemisphere washing out.
const LIMIT_LN: f32 = 0.15;

/// The exposure the two lenses hand the same content over at, pooled over the
/// whole ring, smoothed, and split between them (issue #103, stage 3).
///
/// **One number for the picture, not one per direction.** A gain that varied
/// round the seam would be a brightness that changes as the view pans, which
/// is a worse artifact than the step: the two hemispheres of a sphere should
/// have one exposure between them, and the seam is where that becomes
/// visible rather than where it lives.
///
/// It is the header of the state buffer, so the fragment shader reaches it in
/// one read at a fixed offset rather than averaging 128 cells per pixel.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tone {
    /// The natural log of lens 1's brightness over lens 0's, on the same
    /// content, smoothed at [`TAU_GAIN_S`]. Zero is no correction at all, and
    /// zero is what a file opens in, what a one-lens file stays in, and what
    /// a seam that has never correlated stays in.
    pub log_gain: f32,
    /// What share of the ring was behind the last reading, 0 to 1. Never
    /// applied: the gain is eased in rather than taxed, and this is what an
    /// instrument reads to say how much of the circle answered.
    pub evidence: f32,
    /// A storage buffer's struct is laid out by its own alignment rules and
    /// `repr(C)` does not do it for us. Four floats keeps the cells that
    /// follow on a sixteen-byte boundary whatever [`Cell`] grows into.
    _pad: [f32; 2],
}

impl Tone {
    /// The two numbers a readback finds in the buffer's header, as a `Tone`.
    pub fn read(log_gain: f32, evidence: f32) -> Self {
        Self {
            log_gain,
            evidence,
            _pad: [0.0; 2],
        }
    }

    /// What each lens's picture is multiplied by, lens 0 first: the symmetric
    /// split, so neither hemisphere carries the whole change and neither is
    /// preferred.
    ///
    /// **Exactly one on both sides when nothing has been measured**, which is
    /// the byte-identity of every picture this stage does not touch, and it is
    /// an equality rather than an `exp` that ought to return 1.
    ///
    /// WGSL twin: `tone_split`.
    pub fn split(&self) -> [f32; 2] {
        let half = 0.5 * self.log_gain.clamp(-LIMIT_LN, LIMIT_LN);
        match half == 0.0 {
            true => [1.0, 1.0],
            false => [half.exp(), (-half).exp()],
        }
    }
}

/// One direction's state, as the compute pass writes it and the fragment
/// shader reads it.
///
/// Twenty bytes and every one of them is read by something: the first two by
/// the pass, the third and fourth only by an instrument, and the fifth by the
/// pooling that follows the measurement. Zero is the state a file opens in and
/// the state a direction that has never correlated stays in, and a zero
/// disparity is no bend at all.
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
    /// What the two lenses' pictures of this direction's patch differ by in
    /// brightness, as a natural log, read at the shift that made them the
    /// **same content** (issue #103, stage 3).
    ///
    /// Not smoothed here: it is one reading of 441 samples, and the only
    /// filter in the whole of stage 3 is the one on [`Tone::log_gain`] that
    /// this is pooled into. A direction that stops correlating keeps the
    /// reading it had and loses the confidence that weighs it, which is the
    /// same rule the disparity takes and for the same reason.
    pub tone: f32,
}

impl Cell {
    /// The whole state as one line per direction, for an instrument to hand to
    /// another instrument.
    ///
    /// The band lives on the GPU and no shipped path ever writes it down. This
    /// is how `kjerag-spike --bin band` gives what the pass measured to
    /// `--bin seam`, whose parity render is a CPU one: the camera maker's own
    /// export is in a projection family the app's own pass does not draw, so
    /// scoring against it has to go through [`super::Reframe::blend_bent`]
    /// rather than through the window.
    pub fn write(cells: &[Self]) -> String {
        cells
            .iter()
            .map(|cell| {
                format!(
                    "{} {} {} {} {}\n",
                    cell.disparity, cell.confidence, cell.reach_m, cell.off_epi, cell.tone,
                )
            })
            .collect()
    }

    /// The same, read back. `None` on any line that is not five numbers.
    pub fn read(text: &str) -> Option<Vec<Self>> {
        text.lines()
            .map(|line| {
                let mut numbers = line.split_whitespace().map(str::parse::<f32>);
                let mut next = || numbers.next()?.ok();
                Some(Self {
                    disparity: next()?,
                    confidence: next()?,
                    reach_m: next()?,
                    off_epi: next()?,
                    tone: next()?,
                })
            })
            .collect()
    }

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
    /// Which of the [`SLICES`] rounds of the circle this frame reads.
    pub slice: f32,
    _pad: f32,
}

impl Watch {
    /// The state ages by `seconds` of media time, or starts again.
    ///
    /// A gap that is not a play forward is a `reset`: the state is a running
    /// average over what the seam has been showing, and after a seek it is an
    /// average over somewhere else.
    pub fn new(seconds: f32, reset: bool, slice: u32) -> Self {
        Self {
            // What a direction of this slice has actually aged by: it was last
            // read SLICES frames ago, not one. The filter is paced in seconds,
            // so telling it the truth is the whole of what slicing costs.
            seconds: seconds * SLICES as f32,
            reset: f32::from(u8::from(reset)),
            slice: slice as f32,
            _pad: 0.0,
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
/// **It is also how fast the direction FORGETS.** A direction that stops
/// correlating gives its reading up at this same constant, and that
/// symmetry is not tidiness, it is the whole of the occlusion story
/// (issue #103, owner-reported 2026-08-01). A near reading is a reading of
/// something between the camera and the background - a selfie stick, a hand,
/// a boot, a passer-by - and the reason it correlates fast is the reason it
/// expires fast: it is the thing that moves. A far reading is the background,
/// which does not move, so it is worth keeping. One knee decides both, and it
/// is the geometric one: 0.19 degrees is 10 m at this baseline (6.1).
///
/// A separate stale constant was tried first and is what the owner caught. At
/// 1.5 seconds, a direction that had read the pilot's boot at 3.2 m went on
/// applying 4.5 view px of that reading to the treeline behind it for 45
/// frames, while the seam swept across the scene at 197 deg/s and the content
/// in that direction changed completely in under one frame.
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
/// `band` is the crossover width in radians, which since stage 4 is
/// [`width`]'s answer for that same disparity rather than a constant. See
/// [`FOLD`].
///
/// WGSL twin: `carried`.
pub fn carried(disparity_rad: f32, band_rad: f32) -> f32 {
    let limit = FOLD * band_rad;
    disparity_rad.clamp(-limit, limit)
}

/// How wide the crossover has to be at one direction to carry `disparity_rad`
/// without folding, in radians (issue #103, stage 4).
///
/// [`carried`]'s twin, out of the same inequality: the shear is the disparity
/// over the width, so a width of `|disparity| / FOLD` sits exactly on the
/// clamp and nothing is thrown away. Three things follow and none of them is
/// a choice:
///
/// - **The far field is untouched.** Every direction reading under
///   `FOLD * floor` - 1.8 degrees, which is everything past 1.06 m - already
///   satisfies the inequality at the floor, so the floor is what comes back,
///   bit for bit, and the crossover there is the 2 degrees the owner
///   validated. A file with one lens stream and a direction that has never
///   correlated both read zero and both get the floor.
/// - **It never opens further than it has to.** A wider handover draws more
///   of the picture twice, which is what the 2 degrees was narrowed to stop
///   (`super::projection::CROSSOVER_DEG`), so the narrowest width that does
///   not fold is also the sharpest one available.
/// - **It needs no time constant of its own.** The disparity handed in is the
///   smoothed, evidence-weighted one the bend itself uses, so the width
///   inherits that direction's own constant exactly: a far-field width cannot
///   move faster than a far-field reading, and a direction that stops
///   correlating narrows back to the floor as its confidence fades.
///
/// The floor is passed in rather than read here because the crossover belongs
/// to the projection and the shear belongs to this file, which is also why
/// `carried` takes the width rather than assuming it.
///
/// WGSL twin: `band_width`.
pub fn width(disparity_rad: f32, floor_rad: f32) -> f32 {
    // The floor last, so that a floor set wider than [`WIDEST_DEG`] would
    // still be honoured: a band narrower than the validated crossover is a
    // change to the picture everywhere, and a fold is arithmetic `carried`
    // still catches.
    (disparity_rad.abs() / FOLD)
        .min(WIDEST_DEG.to_radians())
        .max(floor_rad)
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
    let photometry = format!(
        "const TAU_GAIN = {TAU_GAIN_S:?};\n\
         const LIMIT_LN = {LIMIT_LN:?};\n\
         const CLIP_HIGH = {high:?};\n\
         const CLIP_LOW = {low:?};\n",
        high = CLIP_HIGH,
        low = CLIP_LOW,
    );
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
         const SLICES = {SLICES}u;\n\
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
         const FOLD = {fold:?};\n\
         const PATCH = {patch}u;\n\
         const BACK_ALONG = {back_along}u;\n\
         const BACK_ACROSS = {back_across}u;\n\
         const TAU = {tau:?};\n\
         {photometry}{CELL}{RING}{WGSL}",
        tau = std::f32::consts::TAU,
        step = STEP_DEG.to_radians(),
        perp_steps = PERP_STEPS,
        keep = KEEP,
        contrast = CONTRAST / 255.0,
        knee = NEAR_KNEE_DEG.to_radians(),
        far_s = TAU_FAR_S,
        near_s = TAU_NEAR_S,
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
        "const AZIMUTHS = {AZIMUTHS}u;\nconst FOLD = {FOLD:?};\nconst KEEP = {KEEP:?};\n\
         const WIDEST = {widest:?};\nconst TAU = {tau:?};\nconst LIMIT_LN = {LIMIT_LN:?};\n\
         {CELL}{RING}{LOOKUP}",
        widest = WIDEST_DEG.to_radians(),
        tau = std::f32::consts::TAU,
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

/// How many bytes the state buffer is: the pooled [`Tone`], then one [`Cell`]
/// per direction.
pub(crate) const BYTES: u64 =
    (std::mem::size_of::<Tone>() + AZIMUTHS * std::mem::size_of::<Cell>()) as u64;

/// Where the cells start in that buffer, for the readback that unpacks it.
pub(crate) const CELLS_AT: usize = std::mem::size_of::<Tone>();

/// How many workgroups one frame's measurement dispatches: one per direction.
pub(crate) const GROUPS: u32 = AZIMUTHS as u32 / SLICES;

/// A sample at or above this is a clipped highlight and not a brightness.
///
/// A ratio needs both sides to be measurements, and a highlight at the
/// ceiling is the ceiling however much light fell on it: if one lens clips
/// and the other does not, their difference is the sensor's range and not
/// their exposure. The pair is dropped together, so dropping it biases
/// nothing. The sun is in shot at the owner's own reference view, which is
/// where this stopped being hypothetical.
const CLIP_HIGH: f32 = 252.0 / 255.0;
const CLIP_LOW: f32 = 2.0 / 255.0;

/// How many frames it takes to read the whole circle, for the caller that has
/// to count them.
pub(crate) const ROUNDS: u32 = SLICES;

/// Declared by both shaders, with the access each needs. Rust twins: [`Tone`],
/// [`Cell`] and the buffer they are laid out in.
const CELL: &str = r#"
struct Tone {
  log_gain: f32,
  evidence: f32,
  pad0: f32,
  pad1: f32,
};

struct Cell {
  disparity: f32,
  confidence: f32,
  reach_m: f32,
  off_epi: f32,
  tone: f32,
};

struct State {
  tone: Tone,
  cells: array<Cell, AZIMUTHS>,
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
@group(1) @binding(0) var<storage, read> band: State;

// What each lens's picture is multiplied by, lens 0 first (issue #103,
// stage 3). Rust twin: `Tone::split`.
//
// The split is symmetric because the seam cannot say which lens is wrong: a
// correction of +x on one and -x on the other is the same picture at the
// handover, and halving it is what keeps either hemisphere from carrying the
// whole change. It is applied to the RGB the two planes decode to rather than
// to the luma alone, so a hue is scaled with its own brightness and nothing
// shifts colour.
//
// Exactly one on both sides when nothing has been measured, and by an
// equality rather than by trusting `exp(0.0)`: a file with one lens stream, a
// seam that has never correlated and every frame before the first reading all
// reach that line, and every pixel they draw is the one stage 2 drew.
fn tone_split() -> vec2<f32> {
  let half = 0.5 * clamp(band.tone.log_gain, -LIMIT_LN, LIMIT_LN);
  if half == 0.0 {
    return vec2<f32>(1.0, 1.0);
  }
  return vec2<f32>(exp(half), exp(-half));
}

// The band with nothing behind it: no bend, and the crossover at the width it
// has always been. This is what a file with one lens stream takes, what a
// direction that has never correlated takes, and what a ray straight down a
// lens's own axis takes, and it is the picture stage 1 drew.
fn band_rest() -> Band {
  var out: Band;
  out.offset = vec3<f32>(0.0);
  out.crossover = CROSSOVER;
  return out;
}

// The bend a ray takes, in view space, scaled by the ray's own length so that
// adding it turns the ray by the disparity in radians, and how wide the
// handover has to be to carry it. Rust twin: `Reframe::blend_bent`, which
// computes the same two things from `Reframe::disparity_at`.
fn band_bend(ray: vec3<f32>) -> Band {
  let body = reframe.view_to_body * ray;
  let flat = vec2<f32>(body.x, body.y);
  let reach = length(flat);
  if reach <= 0.0 {
    // Straight down a lens's own axis, where there is no seam and no azimuth.
    return band_rest();
  }
  let at = ring_at(vec3<f32>(flat / reach, 0.0));
  // Between two cells, linearly, wrapping: the field is a circle and a step
  // between neighbouring cells would be a step in the picture.
  let turn = atan2(body.y, body.x) / TAU * f32(AZIMUTHS);
  let low = i32(floor(turn));
  let mix = turn - f32(low);
  let a = band.cells[u32(low + i32(AZIMUTHS)) % AZIMUTHS];
  let b = band.cells[u32(low + 1 + i32(AZIMUTHS)) % AZIMUTHS];
  // Weighted by the evidence behind each cell, not just by which is nearer.
  // A direction that has stopped correlating stops contributing, both to what
  // the disparity is and to how much of it is applied, and a ray between one
  // live cell and one dead one takes the live one's answer at the dead one's
  // strength. With no evidence at all the bend is zero, which is exactly the
  // picture before this existed: the fallback is stage 1 and it is reached by
  // arithmetic rather than by a branch. Rust twin: `Reframe::disparity_at`.
  let wa = a.confidence * (1.0 - mix);
  let wb = b.confidence * mix;
  let total = wa + wb;
  if total <= 0.0 {
    return band_rest();
  }
  let disparity = (wa * a.disparity + wb * b.disparity) / total;
  // How much of it to believe. `KEEP` is the correlation a single reading has
  // to reach before it may move the state at all, and confidence is the
  // smoothed value of that same number, so a direction whose recent readings
  // have not been reaching that gate is applied proportionally less. No new
  // constant: the threshold a reading must pass is the threshold a smoothed
  // reading is trusted at.
  let strength = clamp(mix2(a.confidence, b.confidence, mix) / KEEP, 0.0, 1.0);
  let applied = disparity * strength;
  // The bend's own gradient across the band is the disparity over the band
  // width, and past 1 the mapping folds. The band opens far enough to carry
  // this reading, and the clamp holds where it cannot. Rust twins: `width`
  // and `carried`.
  var out: Band;
  out.crossover = band_width(applied);
  let limit = FOLD * out.crossover;
  let carried = clamp(applied, -limit, limit);
  // Back into view space: view_to_body is a rotation, so its transpose is its
  // inverse, and `v * m` is `transpose(m) * v`.
  out.offset = (carried * length(ray)) * (at.epi * reframe.view_to_body);
  return out;
}

// How wide the handover has to be to carry this disparity without folding,
// never narrower than the crossover the projection ships and never wider than
// the widest reading the search can return. Rust twin: `width`.
fn band_width(disparity: f32) -> f32 {
  return max(min(abs(disparity) / FOLD, WIDEST), CROSSOVER);
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
@group(1) @binding(0) var<storage, read_write> band: State;
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
  // Which of the SLICES rounds of the circle this frame reads.
  slice: f32,
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
// Which candidate won, so every lane can read the same answer without
// scoring the table twice.
var<workgroup> winner: u32;
// The photometry, summed cooperatively over the patch at that one winning
// shift (issue #103, stage 3). One entry per lane, reduced by lane 0.
//
// After the peak and not during the search, and that is the whole cost
// argument: the correlation scores EPI_SHIFTS * PERP_SHIFTS candidates and
// only one of them is the same content, so a sum taken inside `correlate`
// would be 117 sums thrown away to keep one, and it would have to carry the
// clip test through the hot loop as well.
var<workgroup> lit0: array<f32, THREADS>;
var<workgroup> lit1: array<f32, THREADS>;
var<workgroup> lit_n: array<f32, THREADS>;
// The pooling's own two, because it is a second entry point over the same
// buffer and not a second use of the same patch: what these hold is one
// number per LANE over the whole ring, not one per sample of one direction.
var<workgroup> pooled_weight: array<f32, THREADS>;
var<workgroup> pooled_total: array<f32, THREADS>;

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
  // Every SLICES-th direction, one further round each frame.
  let cell = group.x * SLICES + u32(watch.slice);
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
    winner = peak();
  }
  workgroupBarrier();

  // The two lenses' brightness on the SAME content, which is what the shift
  // above just established and what no earlier exposure measurement in this
  // project had. Cooperative, so it costs a seventh of a sample per lane.
  photometry(lane, winner);
  workgroupBarrier();

  if lane == 0u {
    settle(cell, at);
  }
}

// The best-scoring candidate shift.
fn peak() -> u32 {
  var best = -2.0;
  var found = 0u;
  for (var i = 0u; i < EPI_SHIFTS * PERP_SHIFTS; i += 1u) {
    if scores[i] > best {
      best = scores[i];
      found = i;
    }
  }
  return found;
}

// Each lane's share of the two patches' brightness at the winning shift,
// clipped samples left out in pairs.
fn photometry(lane: u32, found: u32) {
  let epi = found / PERP_SHIFTS;
  let perp = found % PERP_SHIFTS;
  let width = u32(2 * HALF + 1);
  var sum0 = 0.0;
  var sum1 = 0.0;
  var count = 0.0;
  for (var i = lane; i < PATCH; i += THREADS) {
    let row = i / width;
    let column = i % width;
    let a = front[i];
    let b = back[(row + epi) * BACK_ALONG + perp * u32(PERP_STEP) + column];
    // A pair, both ways: a sample either lens has no picture of is not a
    // pair, and a pair with a clipped or crushed side is the sensor's range
    // rather than its exposure. Dropped together, so nothing is biased by
    // dropping it.
    if a < CLIP_LOW || b < CLIP_LOW || a > CLIP_HIGH || b > CLIP_HIGH {
      continue;
    }
    sum0 += a;
    sum1 += b;
    count += 1.0;
  }
  lit0[lane] = sum0;
  lit1[lane] = sum1;
  lit_n[lane] = count;
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
  var held = band.cells[cell];
  if watch.reset != 0.0 {
    held = Cell(0.0, 0.0, at.reach_m, 0.0, 0.0);
  }
  held.reach_m = at.reach_m;

  let found = winner;
  let best = scores[found];
  let epi = i32(found / PERP_SHIFTS);
  let perp = i32(found % PERP_SHIFTS) - PERP_STEPS;
  // A peak against the edge of the search is not a peak, it is the search
  // running out: near-field content moves further across than the band is
  // wide, and a reading pinned at the limit would report the limit.
  let pinned = epi == 0 || epi == i32(EPI_SHIFTS) - 1;
  if best < KEEP || pinned {
    // Nothing to read here this frame. What decays is the EVIDENCE and not
    // the measurement: the reading was true when it was taken and may be true
    // still, but nothing is confirming it, and the pass applies a reading in
    // proportion to how well it is being confirmed (`band_bend`). So the bend
    // fades out on its own, and a direction that starts correlating again has
    // its answer already in hand rather than having to learn it twice.
    //
    // It fades at the SAME rate the direction learns, which is the whole of
    // the occlusion story: a near reading is a reading of something between
    // the camera and the background - a selfie stick, a hand, a boot, someone
    // walking past - and the reason it correlates fast is the reason it
    // expires fast. A far reading is the background, which has not gone
    // anywhere. One knee decides both.
    held.confidence -= held.confidence * ease(watch.seconds, time_constant(held.disparity));
    band.cells[cell] = held;
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
  held.tone = read_tone(held.tone);
  band.cells[cell] = held;
}

// What the two lenses' pictures of this patch differ by in brightness at the
// shift that made them the same content, as a natural log, or the reading
// this direction already had where clipping left no pair to read.
//
// A ratio of MEANS rather than a mean of ratios or a regression slope: it is
// the statistic the correction inverts. What the pass applies is one
// multiplier over a whole hemisphere, so the number it wants is the one whose
// inverse makes the two patches' totals equal, and that is this and nothing
// else. The instrument prints the other two beside it
// (`kjerag-spike --bin expose`) precisely so that claim can be checked rather
// than believed.
//
// In the video's own gamma-coded luma, and the correction is applied in that
// same space, so no transfer function is assumed at either end: what is
// measured is a brightness match and it is inverted as one.
fn read_tone(held: f32) -> f32 {
  var sum0 = 0.0;
  var sum1 = 0.0;
  var count = 0.0;
  for (var i = 0u; i < THREADS; i += 1u) {
    sum0 += lit0[i];
    sum1 += lit1[i];
    count += lit_n[i];
  }
  if count <= 0.0 || sum0 <= 0.0 || sum1 <= 0.0 {
    return held;
  }
  return log(sum1 / sum0);
}

// The pooled exposure, over the whole ring and over media time.
//
// One workgroup, dispatched straight after the measurement and in the same
// pass, so what it pools is what was just written. Every direction that is
// correlating contributes at the weight the bend already trusts it at
// (`band_bend`'s `strength`), so no threshold is added here: a direction whose
// evidence has faded fades out of the exposure too, and one that never
// correlated was never in it.
@compute @workgroup_size(THREADS)
fn pool(@builtin(local_invocation_index) lane: u32) {
  var weight = 0.0;
  var total = 0.0;
  for (var i = lane; i < AZIMUTHS; i += THREADS) {
    let cell = band.cells[i];
    let trust = clamp(cell.confidence / KEEP, 0.0, 1.0);
    weight += trust;
    total += trust * cell.tone;
  }
  pooled_weight[lane] = weight;
  pooled_total[lane] = total;
  workgroupBarrier();
  if lane != 0u {
    return;
  }
  var sum_weight = 0.0;
  var sum_total = 0.0;
  for (var i = 0u; i < THREADS; i += 1u) {
    sum_weight += pooled_weight[i];
    sum_total += pooled_total[i];
  }
  var held = band.tone;
  // A seek starts the gain again from no correction at all rather than from
  // an average of somewhere else, and walks in from there over TAU_GAIN. It
  // is the one place this differs from the bend, which takes its first
  // reading whole: a bend that arrives late leaves a doubled edge for a
  // second and a gain that arrives instantly is a hemisphere changing
  // brightness in one frame, which is the artifact this stage exists to not
  // create.
  if watch.reset != 0.0 {
    held = Tone(0.0, 0.0, 0.0, 0.0);
  }
  if sum_weight <= 0.0 {
    // Nothing on the ring is confirming anything this frame. The exposure of
    // two lenses does not change because we stopped being able to see it, so
    // the value is kept and only the evidence behind it is given up.
    held.evidence -= held.evidence * ease(watch.seconds, TAU_GAIN);
    band.tone = held;
    return;
  }
  let read = clamp(sum_total / sum_weight, -LIMIT_LN, LIMIT_LN);
  let step = ease(watch.seconds, TAU_GAIN);
  held.log_gain += (read - held.log_gain) * step;
  held.evidence += (sum_weight / f32(AZIMUTHS) - held.evidence) * step;
  band.tone = held;
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

    /// The occlusion rule, and the owner-reported defect it answers
    /// (2026-08-01): a near reading is a reading of something that MOVES, so
    /// the rate that tracks it is also the rate that must forget it. A boot,
    /// a selfie stick, a hand and a passer-by are one class, and none of them
    /// is still there a second later.
    #[test]
    fn a_near_reading_expires_as_fast_as_it_was_learned() {
        let near = 1.0f32.to_radians();
        let reads = 5.0;
        // Five readings after it stops correlating, a near-field direction has
        // given up nearly all of what it held. The old fixed 1.5 s constant
        // left 81 percent of it there.
        let mut held = near;
        for _ in 0..reads as usize {
            held -= held * ease(2.0 / 30.0, time_constant(held));
        }
        assert!(
            held / near < 0.15,
            "a near reading still holds {:.0} percent after five reads",
            100.0 * held / near,
        );
        // And the far field keeps what it has, because the background it is
        // looking at has not gone anywhere.
        let far = 0.05f32.to_radians();
        let mut held = far;
        for _ in 0..reads as usize {
            held -= held * ease(2.0 / 30.0, time_constant(held));
        }
        assert!(
            held / far > 0.80,
            "a far reading gave up {:.0} percent after five reads",
            100.0 * (1.0 - held / far),
        );
    }

    /// With nothing behind it, the bend is nothing, and nothing is exactly the
    /// picture stage 1 drew. The fallback is reached by arithmetic and not by
    /// a branch, which is why it cannot be forgotten.
    #[test]
    fn a_direction_with_no_evidence_bends_nothing() {
        use crate::projection::tests::{FRAME, fixture_lenses};
        let lenses = fixture_lenses();
        let reframe = crate::projection::Reframe::new(
            &lenses,
            FRAME,
            crate::Camera::default(),
            crate::projection::Held::default(),
            1.0,
            false,
            crate::sampling::Sampling::default(),
        );
        let ray = [1.0, 0.0, 0.0];
        // A whole degree of disparity, and no confidence anywhere.
        let dead = vec![
            Cell {
                disparity: 1.0f32.to_radians(),
                confidence: 0.0,
                reach_m: 0.033,
                off_epi: 0.0,
            };
            AZIMUTHS
        ];
        assert_eq!(reframe.disparity_at(ray, &dead), 0.0);
        assert_eq!(
            reframe.bend(ray, reframe.disparity_at(ray, &dead)),
            [0.0; 3]
        );
        // And with full confidence it is applied whole: the gate is a gate,
        // not a tax on every reading.
        let live: Vec<Cell> = dead
            .iter()
            .map(|cell| Cell {
                confidence: KEEP,
                ..*cell
            })
            .collect();
        let held = reframe.disparity_at(ray, &live);
        assert!(
            (held - 1.0f32.to_radians()).abs() < 1e-6,
            "a fully trusted direction applied {held}",
        );
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

    /// The floor the shipped pass hands [`width`], which is the projection's
    /// own crossover. Written here rather than imported because that constant
    /// is private to its own module, and `the_floor_is_the_shipped_crossover`
    /// is what keeps the two honest.
    const FLOOR_DEG: f32 = 2.0;

    #[test]
    fn the_bend_never_folds_the_crossover() {
        // Shear is the disparity over the band width and above 1 the mapping
        // prints the picture back over itself. What the pair has to promise
        // is that the Jacobian stays positive at any disparity the search can
        // report, the near limit included - now that the band opens as well
        // as the clamp closing, both halves are in the promise.
        let floor = FLOOR_DEG.to_radians();
        for degrees in [-10.0f32, -1.2, 0.0, 0.19, 1.9, 3.5, 100.0] {
            let band = width(degrees.to_radians(), floor);
            let shear = carried(degrees.to_radians(), band) / band;
            assert!(
                (1.0 + shear) > 0.05,
                "{degrees} deg leaves a Jacobian of {:.3}",
                1.0 + shear,
            );
            assert!(
                shear.abs() <= FOLD + 1e-6,
                "{degrees} deg shears the band by {shear:.3}",
            );
        }
    }

    /// Stage 4 in one line: the width and the clamp are the same inequality.
    #[test]
    fn the_band_carries_every_disparity_the_search_can_report() {
        let floor = FLOOR_DEG.to_radians();
        // The search refuses a peak against either edge of its window, so what
        // it can actually hand over is strictly inside [FAR_DEG, NEAR_DEG].
        for step in 0..=200 {
            let degrees = FAR_DEG + (NEAR_DEG - FAR_DEG) * step as f32 / 200.0;
            let radians = degrees.to_radians();
            let carried = carried(radians, width(radians, floor));
            assert!(
                (carried - radians).abs() < 1e-6,
                "{degrees:.2} deg was cut to {:.2}",
                carried.to_degrees(),
            );
        }
        // And stage 2's fixed band did not, which is what this stage is for.
        let near = 2.4f32.to_radians();
        let stage2 = carried(near, floor);
        assert!(
            (near - stage2).to_degrees() > 0.5,
            "the fixed band was already carrying {:.2} deg",
            stage2.to_degrees(),
        );
    }

    #[test]
    fn the_far_field_keeps_the_crossover_it_had() {
        // Bit for bit, and that matters more than it looks: the far field is
        // where the horizon is, the pixels off the seam are supposed to be
        // byte-identical to the picture before this stage, and a width that
        // came back a float ulp from the floor would move every one of them.
        let floor = FLOOR_DEG.to_radians();
        for degrees in [-1.2f32, -0.84, -0.19, 0.0, 0.19, 0.64, 1.79, 1.8] {
            let opened = width(degrees.to_radians(), floor);
            assert_eq!(
                opened.to_bits(),
                floor.to_bits(),
                "{degrees} deg opened the band to {:.4} deg",
                opened.to_degrees(),
            );
        }
        // A direction with no evidence reads zero and takes the floor too,
        // which is every direction of a one-lens file and every direction of a
        // file's first frame.
        assert_eq!(width(0.0, floor).to_bits(), floor.to_bits());
    }

    #[test]
    fn the_band_opens_no_further_than_the_reading_needs() {
        let floor = FLOOR_DEG.to_radians();
        // Monotone, so a direction drifting nearer does not step, and never
        // past the widest reading the search can return.
        let mut last = 0.0f32;
        for step in 0..400 {
            let opened = width(step as f32 * 0.01f32.to_radians(), floor);
            assert!(opened >= last - 1e-9, "step {step}: {opened} after {last}");
            assert!(opened <= WIDEST_DEG.to_radians() + 1e-9);
            last = opened;
        }
        // In between it is exactly what the inequality asks for and not a
        // rounded-up version of it: at 2.4 degrees of disparity the band is
        // 2.67 and the shear is exactly FOLD.
        let near = 2.4f32.to_radians();
        assert!((width(near, floor) - near / FOLD).abs() < 1e-9);
    }

    #[test]
    fn the_width_cannot_flicker_faster_than_the_reading_it_comes_from() {
        // The whole of stage 4's temporal design, and the reason it adds no
        // filter and no constant. The width is 1/FOLD-Lipschitz in the
        // disparity, so the per-direction time constants stage 2 measured
        // bound the width's own steadiness as well: 0.02 deg rms of disparity
        // flicker cannot become more than 0.022 deg rms of width flicker,
        // whatever the content is.
        let floor = FLOOR_DEG.to_radians();
        let mut worst = 0.0f64;
        for a in -300..300 {
            for b in -300..300 {
                let (one, two) = (a as f32 * 0.01, b as f32 * 0.01);
                let moved = f64::from(
                    (width(one.to_radians(), floor) - width(two.to_radians(), floor)).abs(),
                );
                let read = f64::from((one - two).abs().to_radians());
                // Slack for the f32 rounding in the two differences
                // themselves, which is what is being compared and not what is
                // being claimed: at a hundredth of a degree apart the two sides
                // are 1.9e-4 and the last bits of each are noise.
                assert!(
                    moved <= read / f64::from(FOLD) * (1.0 + 1e-4) + 1e-12,
                    "{one} to {two} deg moved the band by {moved}",
                );
                worst = worst.max(match read > 0.0 {
                    true => moved / read,
                    false => 0.0,
                });
            }
        }
        // And the bound is reached, so it is the truth about this function
        // rather than a loose statement that happens to hold.
        assert!(
            (worst - 1.0 / f64::from(FOLD)).abs() < 1e-3,
            "worst ratio {worst}",
        );
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
        // And the near side has to reach past what the crossover carries at
        // its floor, or the band has nothing to open for and stage 4 is a
        // no-op. This is the same line stage 2 wrote as `NEAR_DEG > FOLD *
        // 2.0`, read from the other end.
        const {
            assert!(WIDEST_DEG > FLOOR_DEG);
        }
    }

    #[test]
    fn the_floor_is_the_shipped_crossover() {
        // `FLOOR_DEG` above is a copy of a constant this module cannot see,
        // and a copy that drifts would make every test above test nothing.
        // The shipped pass is what is asked, rather than the constant.
        use crate::projection::tests::{FRAME, fixture_lenses};
        let reframe = crate::projection::Reframe::new(
            &fixture_lenses(),
            FRAME,
            crate::Camera::default(),
            crate::projection::Held::default(),
            1.0,
            false,
            crate::sampling::Sampling::default(),
        );
        assert_eq!(
            reframe.crossover_at(0.0).to_bits(),
            FLOOR_DEG.to_radians().to_bits(),
        );
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
