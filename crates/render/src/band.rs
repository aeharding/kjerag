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

/// How far along the seam the search looks, in degrees, either side of zero.
///
/// **This is the axis a horizon shows** (issue #103, stage 5,
/// docs/research/seam-two-axis.md). At the owner's fov-20 reference view one
/// degree on the epipolar axis moves the horizon 0.6 rows and one degree along
/// the seam moves it 53, because the seam circle is near vertical, the
/// baseline is along the lens axes, and the ground's edge runs along the
/// azimuth. A horizon is the worst detector of epipolar error there is and the
/// best detector of this one.
///
/// Parallax cannot reach it: the baseline is perpendicular to every direction
/// on the seam circle, so a subject's distance displaces it across the seam
/// and never along it, at any distance. What is left here is the camera - what
/// the five-knob fit could not describe - and it is fixed in the camera's frame
/// for the life of the file, which is what [`TAU_FAR_S`] below is claiming.
///
/// 0.90 is the corpus range with margin. The best per-file fit available
/// leaves **0.17 to 0.67 degrees** across three shooters and three camera
/// models, and the owner's pooled path leaves 0.30 to 0.47 (2026-08-01,
/// `--bin seam mode=residual`). The old 0.30 did not measure that: it clipped
/// it, on 44 percent of measured directions cold and 67 percent warm, and
/// reported the limit as if it were a reading.
///
/// The near end of the epipolar search is set by what the bend can carry; this
/// one is not, and that is the difference between the axes. A bend along
/// [`Ring::perp`] is a **shear perpendicular to its own gradient**, whose
/// Jacobian determinant is exactly 1, so it cannot fold however wide it opens
/// ([`width`] is not asked about it); and `perp` is the seam circle's own
/// tangent, so the bend slides content along the circle rather than off it and
/// spends none of the overlap the two lenses share
/// (`the_along_seam_bend_costs_no_overlap_and_cannot_fold`).
pub const PERP_DEG: f32 = 0.90;

/// How many along-seam offsets are tried, either side of zero.
///
/// Nine at [`STEP_DEG`], which makes the grid **square**: the same 0.10 degrees
/// on both axes, with the same parabola between whole steps on both, and no
/// second resolution constant to keep in step with the first. Today's readings
/// quantize to 0.30 degrees, which is 15 view pixels of horizon at the view the
/// owner complained about; a third of a step is what stage 2 already resolves
/// on the other axis and it is what a horizon at fov 20 needs here.
const PERP_STEPS: usize = 9;

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
pub const NEAR_KNEE_DEG: f32 = 0.19;

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
/// close. The widest band plus the whole bend it carries stays inside the
/// overlap the two lenses of the calibration fixture share, 14.44 degrees or
/// 7.22 a side (`the_widest_band_and_its_bend_stay_inside_the_overlap`, which
/// measures it off the file's own calibration rather than quoting the format
/// study). `kjerag-spike --bin band` reports the same two numbers for whatever
/// file it is given, because the overlap is a property of the camera and this
/// ceiling is not.
///
/// [`SLOPE`] is in it since stage 8, and it is the same inequality: the shear
/// is the bend's own gradient across the band, and a profile that is steeper
/// in the middle than a straight line needs proportionally more room to carry
/// the same reading.
pub const WIDEST_DEG: f32 = NEAR_DEG * SLOPE / FOLD;

/// The steepest the handover profile ever gets, as a multiple of one over its
/// width (issue #103, stage 8).
///
/// The profile is `t^3 (6t^2 - 15t + 10)`, whose derivative is `30 t^2
/// (1 - t)^2` and whose largest value is `15/8` at the middle. It is here as a
/// number rather than as a comment because it is what [`width`] solves the
/// shear inequality with: everything stage 4 derived holds unchanged, at the
/// slope the profile actually has instead of at the straight line's 1.
///
/// **Why the profile is not a straight line.** A handover that runs from one
/// lens to the other along a straight line has a corner at each end, and a
/// corner in a brightness gradient is a **Mach band**: the eye's own lateral
/// inhibition draws a line there that the picture does not have. That is the
/// residual artifact on a wide sky after the photometry is corrected, and no
/// amount of correction reaches it, because it is not in the numbers. This
/// profile meets 0 and 1 with zero first AND second derivative, so there is no
/// corner at either end to draw one.
pub const SLOPE: f32 = 15.0 / 8.0;

/// One code of an 8-bit picture, which is the floor of the medium and the unit
/// [`open`](Cell::open) trades in.
const ONE_CODE: f32 = 1.0 / 255.0;

/// The widest additive term that is a scene's glare rather than a measurement
/// coming apart, in codes of 1.
///
/// [`LIMIT_LN`]'s own doctrine, on the other parameter: four times the widest
/// well-sampled reading. Fitted in ratio space over whole captures at the
/// directions this pass pools, the offset beside the gain runs to **5.9 codes**
/// of 255 on the owner's own reference (docs/research/seam-blending.md 2), and
/// four times that is 23.6 codes, which is 0.0925 of 1.
///
/// It guards the case none of those captures is, which is the same case
/// [`LIMIT_LN`] guards: a seam correlating on content that is not the same
/// content at all. What it cannot do is move a hemisphere's black level,
/// because it is not applied over a hemisphere - it lives inside the blend
/// region and eases to nothing by the overlap ([`fade`]).
const LIMIT_OFF: f32 = 0.0925;

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
/// `RUNAWAY_DEG` is - from what was measured, times four. Fitted over whole
/// captures at the directions this pass actually pools,
/// `kjerag-spike --bin expose` reads **0.946 to 1.004** on the seven with
/// more than seventy far-field readings behind them, which is four of the
/// owner's flights and an X5, an X4 and an X3 from other shooters. That is
/// -0.0558 to +0.0044 ln, and four times the widest is 0.223. Two thin
/// captures of the owner's, with two and four readings each, ask for 0.908
/// and 0.948; this admits both rather than clipping them, because a capture
/// whose seam has almost nothing far-field on it is a capture this should be
/// quiet about and not one it should be half-correcting.
///
/// Nothing measured is clipped by this, which is what keeps it a guard rather
/// than a tuning knob. What it is here for is the case none of those captures
/// is: a seam correlating on content that is not the same content at all.
///
/// What it is here for is the case none of those captures is: a seam
/// correlating on content that is not the same content at all, which would
/// otherwise reach the picture as a hemisphere washing out.
const LIMIT_LN: f32 = 0.25;

/// The colour the two lenses hand the same content over at, pooled over the
/// whole ring, smoothed, and split between them (issue #103, stages 3 and 7).
///
/// **Three numbers where stage 3 had one, and the third one is the point.**
/// Stage 3 measured brightness and multiplied all three channels by the same
/// gain, so whatever the two lenses disagree about that is **not** common to R,
/// G and B survived it exactly, however well it was fitted. What survives a
/// brightness correction is a hue, and it is measured across nine captures from
/// four camera models at 1.6 to 15.6 codes of spread between the channels -
/// over the one code an 8-bit picture can carry on every one of them. On one
/// corpus camera the spread is 10.3 codes with the sun in one lens and 0.47
/// with the sun in neither, which is the owner's own report in somebody else's
/// footage (docs/research/insv-format.md 6.11).
///
/// **One number per channel for the picture, not one per direction.** A gain
/// that varied round the seam would be a brightness that changes as the view
/// pans, which is a worse artifact than the step: the two hemispheres of a
/// sphere should have one exposure and one white balance between them, and the
/// seam is where that becomes visible rather than where it lives.
///
/// It is the header of the state buffer, so the fragment shader reaches it in
/// one read at a fixed offset rather than averaging 128 cells per pixel. It is
/// the same sixteen bytes stage 3's was: the padding became the two channels.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tone {
    /// Per channel, in R, G, B order, the natural log of lens 1's picture over
    /// lens 0's on the same content, smoothed at [`TAU_GAIN_S`]. Zero is no
    /// correction at all, and zero is what a file opens in, what a one-lens
    /// file stays in, and what a seam that has never correlated stays in.
    ///
    /// In the video's own gamma-coded space, decoded through the fragment
    /// shader's own BT.709 matrix, which is the space the correction is applied
    /// in: no transfer function is assumed at either end.
    pub log_gain: [f32; 3],
    /// What share of the ring was behind the last reading, 0 to 1. Never
    /// applied: the gain is eased in rather than taxed, and this is what an
    /// instrument reads to say how much of the circle answered.
    pub evidence: f32,
}

impl Tone {
    /// The four numbers a readback finds in the buffer's header, as a `Tone`.
    pub fn read(log_gain: [f32; 3], evidence: f32) -> Self {
        Self { log_gain, evidence }
    }

    /// What each lens's picture is multiplied by, per channel, lens 0 first:
    /// the symmetric split, so neither hemisphere carries the whole change and
    /// neither is preferred.
    ///
    /// **Exactly one on both sides in every channel when nothing has been
    /// measured**, which is the byte-identity of every picture this stage does
    /// not touch, and it is an equality rather than an `exp` that ought to
    /// return 1.
    ///
    /// WGSL twin: `tone_split`.
    pub fn split(&self) -> [[f32; 3]; 2] {
        let half: [f32; 3] =
            std::array::from_fn(|channel| 0.5 * self.log_gain[channel].clamp(-LIMIT_LN, LIMIT_LN));
        match half == [0.0; 3] {
            true => [[1.0; 3], [1.0; 3]],
            false => [half.map(f32::exp), half.map(|h| (-h).exp())],
        }
    }

    /// The one number stage 3 had, for an instrument that is still asking its
    /// question: what the two lenses differ by in **brightness**, as the luma
    /// the three gains imply.
    pub fn luma_gain(&self) -> f32 {
        let split = self.split();
        (LUMA[0] * split[1][0] + LUMA[1] * split[1][1] + LUMA[2] * split[1][2])
            .max(f32::MIN_POSITIVE)
            .ln()
            * 2.0
    }
}

/// What each channel is worth to brightness: BT.709's own luma weights, which
/// is the matrix the fragment shader decodes NV12 through read the other way.
const LUMA: [f32; 3] = [0.212_6, 0.715_2, 0.072_2];

/// The along-seam correction as one field over the whole seam circle: a
/// constant and the first two cycles of the azimuth (issue #103, stage 5).
///
/// **Five numbers for 128 directions, and each of the three terms has a
/// name.** A relative rotation `w` displaces a seam direction `d` by `w x d`,
/// whose along-seam component is `w.z` for every direction on the circle, so a
/// **constant** along the seam is relative roll and nothing else can reach it.
/// A principal-point shift is a fixed direction in the image plane, so its
/// tangential part turns **once** round the rim. A focal aspect maps the rim
/// circle to an ellipse, which is **twice**. That decomposition is not this
/// stage's: `kjerag-spike --bin seam` has printed it since issue #48, and
/// docs/research/seam-two-axis.md records it.
///
/// It exists because the per-direction field was built first and measured
/// second. Applied cell by cell it **scallops**: the ring is read at 128
/// azimuths, far fewer than that correlate on any real frame, and a horizon
/// drawn through a field with holes in it comes out visibly warped - 18.5 view
/// px of correction at one end of a four-degree fit and 4.7 at the other, on
/// the owner's own reference view, which bent the horizon instead of moving
/// it. Fitting the shape the phenomenon actually has fills the holes from the
/// readings that exist, and does it by construction rather than by smoothing
/// over them.
///
/// What it leaves is measured, not assumed: on the owner's July file under his
/// own pooled calibration the along-seam residual round the circle is 0.314
/// deg root mean square, 0.275 after the constant, 0.101 after one cycle and
/// **0.094 after two** (`--bin seam mode=residual`, the structure table). A
/// third cycle is not indicated and is not fitted.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Along {
    /// The five coefficients, in radians: the constant, then the cosine and
    /// sine of one cycle, then of two.
    pub terms: [f32; 5],
    /// How many directions' worth of evidence is behind the fit, 0 up to
    /// [`AZIMUTHS`]. Read by instruments; the fit is already shrunk by its own
    /// ridge and is not taxed twice.
    pub evidence: f32,
    /// A storage buffer's struct is laid out by its own alignment rules and
    /// `repr(C)` does not do it for us.
    _pad: [f32; 2],
}

impl Along {
    /// The correction at one azimuth, in radians, from that azimuth's own
    /// cosine and sine - which a fragment already has, because a direction
    /// flattened into the seam plane **is** `(cos, sin)`. No trig reaches the
    /// fragment shader.
    ///
    /// WGSL twin: `along_at`.
    pub fn at(&self, cos: f32, sin: f32) -> f32 {
        let basis = [1.0, cos, sin, cos * cos - sin * sin, 2.0 * cos * sin];
        (0..5).map(|term| self.terms[term] * basis[term]).sum()
    }

    /// The whole ring's along-seam readings as one field: evidence-weighted
    /// least squares over the five basis functions, with a ridge.
    ///
    /// **Rust twin of the `pool` entry point's second half**, and a twin
    /// rather than a description: the pass solves this on the GPU where no
    /// test can reach it, and every property claimed for it is claimed about a
    /// function `cargo test` can call with no device and no footage.
    ///
    /// The ridge is [`RIDGE`] and it is what makes a thin ring safe: a term
    /// supported by `n` directions is shrunk by about `n / (n + 1)`, which is
    /// nothing at forty and everything at none, so a file's first frames walk
    /// the correction in by arithmetic rather than by a second time constant.
    /// With no evidence at all every coefficient is exactly zero, which is the
    /// picture stage 4 drew.
    ///
    /// **Trust is the same `off_conf / KEEP` the bend applies itself.** No
    /// second threshold: a direction the along-seam channel has stopped
    /// believing is not one this should be believing either.
    pub fn fit(cells: &[Cell]) -> Self {
        let mut normal = [[0.0f32; 5]; 5];
        let mut right = [0.0f32; 5];
        let mut evidence = 0.0;
        for (index, cell) in cells.iter().enumerate() {
            let trust = (cell.off_conf / KEEP).clamp(0.0, 1.0);
            if trust <= 0.0 {
                continue;
            }
            let (sin, cos) = (index as f32 / cells.len() as f32 * std::f32::consts::TAU).sin_cos();
            let basis = [1.0, cos, sin, cos * cos - sin * sin, 2.0 * cos * sin];
            for row in 0..5 {
                for (column, term) in basis.iter().enumerate() {
                    normal[row][column] += trust * basis[row] * term;
                }
                right[row] += trust * basis[row] * cell.off_epi;
            }
            evidence += trust;
        }
        for (term, row) in normal.iter_mut().enumerate() {
            row[term] += RIDGE;
        }
        Self {
            terms: solve(normal, right),
            evidence,
            _pad: [0.0; 2],
        }
    }

    /// The eight floats a readback finds in the buffer, as an `Along`.
    pub fn read(terms: [f32; 5], evidence: f32) -> Self {
        Self {
            terms,
            evidence,
            _pad: [0.0; 2],
        }
    }
}

/// The additive term round the seam, per channel, as a shape that **cannot**
/// stripe (issue #103, stage 8, after the owner rejected the per-direction
/// form).
///
/// **What went wrong.** Stage 8's first two forms applied [`Cell::offset`] at
/// the direction it was measured at. Each direction's reading carries its own
/// noise, and a field applied over a wide support paints that noise along the
/// whole sweep of the direction it belongs to: the owner saw **dark streaks
/// running away from the seam** across his soil and rejected the branch. It is
/// stage 5's scalloping, on the photometric axis, and it was invisible to every
/// acceptance statistic in the campaign because all of them straddle the seam
/// and none of them looked at the field's own interior
/// (`kjerag-spike --bin colour`, the interior block).
///
/// **What the measurement actually supported.** The per-direction form was
/// justified by a residual of 4.2 to 5.5 codes rms round the ring against a
/// frame-to-frame noise floor of 0.8 to 1.0. That floor was measured between
/// CONSECUTIVE frames, where the content at a direction is nearly the same, so
/// it measured the sensor and not the content: what a misregistration costs a
/// photometry is the content's own gradient across the window, and on textured
/// soil that is most of the residual. Read at instants far enough apart for the
/// content to have changed, what persists at a direction is far less than what
/// the ring residual claimed.
///
/// So the shape is the one the geometry uses and stage 7 used: a constant, one
/// cycle of the azimuth and two. Five terms per channel is not a taste - it is
/// the harmonic content a relative property of two cameras can have - and
/// nothing outside it can be drawn as a stripe because nothing outside it
/// exists.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Glare {
    /// Per channel, in R, G, B order, five coefficients in codes of 1: the
    /// constant, then the cosine and sine of one cycle, then of two.
    pub terms: [f32; 15],
    /// How many directions' worth of evidence is behind the fit.
    pub evidence: f32,
}

impl Glare {
    /// The additive term at one azimuth, per channel, in codes of 1, from that
    /// azimuth's own cosine and sine - which a fragment already has.
    ///
    /// WGSL twin: `glare_at`.
    pub fn at(&self, cos: f32, sin: f32) -> [f32; 3] {
        let basis = [1.0, cos, sin, cos * cos - sin * sin, 2.0 * cos * sin];
        std::array::from_fn(|channel| {
            (0..5)
                .map(|term| self.terms[5 * channel + term] * basis[term])
                .sum::<f32>()
                .clamp(-LIMIT_OFF, LIMIT_OFF)
        })
    }

    /// The whole ring's additive readings as one field per channel: the same
    /// weighted least squares [`Along::fit`] runs, over [`Cell::offset`].
    ///
    /// **Rust twin of the `pool_glare` entry point.**
    pub fn fit(cells: &[Cell]) -> Self {
        let mut terms = [0.0f32; 15];
        let mut evidence = 0.0;
        for channel in 0..3 {
            let mut normal = [[0.0f32; 5]; 5];
            let mut right = [0.0f32; 5];
            evidence = 0.0;
            for (index, cell) in cells.iter().enumerate() {
                let trust = (cell.hue_conf / KEEP).clamp(0.0, 1.0);
                if trust <= 0.0 {
                    continue;
                }
                let (sin, cos) =
                    (index as f32 / cells.len() as f32 * std::f32::consts::TAU).sin_cos();
                let basis = [1.0, cos, sin, cos * cos - sin * sin, 2.0 * cos * sin];
                for row in 0..5 {
                    for (column, term) in basis.iter().enumerate() {
                        normal[row][column] += trust * basis[row] * term;
                    }
                    right[row] += trust * basis[row] * cell.offset[channel];
                }
                evidence += trust;
            }
            for (term, row) in normal.iter_mut().enumerate() {
                row[term] += RIDGE;
            }
            let fitted = solve(normal, right);
            for term in 0..5 {
                terms[5 * channel + term] = fitted[term];
            }
        }
        Self { terms, evidence }
    }

    /// The sixteen floats a readback finds in the buffer, as a `Glare`.
    pub fn read(terms: [f32; 15], evidence: f32) -> Self {
        Self { terms, evidence }
    }
}

/// How much evidence a coefficient is shrunk against, in directions.
///
/// One direction's worth, which is a quantity and not a taste: it says a fit is
/// believed in proportion to how much of the ring is behind it, and it makes a
/// ring with nothing on it come out at exactly zero rather than at a division.
/// A term forty directions agree on gives up two percent of itself to this; a
/// term one direction has seen gives up half.
const RIDGE: f32 = 1.0;

/// A small symmetric positive definite system, by Gaussian elimination with no
/// pivoting.
///
/// No pivoting is safe here rather than a shortcut taken: the matrix is a Gram
/// matrix plus [`RIDGE`] on the diagonal, so it is positive definite whatever
/// the ring holds and its pivots are never zero, even with no evidence at all.
///
/// WGSL twin: `solve5`.
pub fn solve(mut normal: [[f32; 5]; 5], mut right: [f32; 5]) -> [f32; 5] {
    for pivot in 0..5 {
        let scale = normal[pivot][pivot];
        let leading = normal[pivot];
        for row in (pivot + 1)..5 {
            let factor = normal[row][pivot] / scale;
            for (column, above) in leading.iter().enumerate().skip(pivot) {
                normal[row][column] -= factor * above;
            }
            right[row] -= factor * right[pivot];
        }
    }
    let mut out = [0.0f32; 5];
    for row in (0..5).rev() {
        let mut total = right[row];
        for column in (row + 1)..5 {
            total -= normal[row][column] * out[column];
        }
        out[row] = total / normal[row][row];
    }
    out
}

/// One direction's state, as the compute pass writes it and the fragment
/// shader reads it.
///
/// Seven floats and every one of them is read by something: the two axes and
/// their two confidences by the pass, the reach only by an instrument, and the
/// last two by the pooling that follows the measurement. Zero is the state a
/// file opens in and the state a direction that has never correlated stays in,
/// and a zero on either axis is no bend at all.
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
    /// The smoothed along-seam offset, in **radians** along [`Ring::perp`]:
    /// what the two lenses disagree about on the axis a distance cannot reach
    /// (issue #103, stage 5).
    ///
    /// Smoothed at [`TAU_FAR_S`] whatever this direction's disparity is, and
    /// that is the one place this channel deliberately differs from
    /// [`Self::disparity`]. [`time_constant`] tracks the near field fast
    /// because the near field is the thing that **moves**; this quantity
    /// cannot move, because parallax is epipolar by construction and what is
    /// left on this axis is the camera. A boot crossing the seam changes the
    /// disparity of that direction by degrees and changes this by nothing, so
    /// a constant that tracked it would be tracking the correlator's own
    /// noise. Measured before it was chosen (`kjerag-spike --bin band`, the
    /// `leak` line): across the corpus this reads no dependence on the
    /// disparity at all.
    pub off_epi: f32,
    /// How well the along-seam channel is correlating, 0 for a direction that
    /// has never been read, up to 1.
    ///
    /// Its own and not [`Self::confidence`], because the two axes are refused
    /// separately: a reading pinned against the near end of the epipolar
    /// search is a depth the band cannot carry and says nothing about either
    /// axis, but a reading pinned against the end of the **along-seam** search
    /// is a camera outside anything measured, and refusing the epipolar
    /// channel for it would throw away stage 2 on that footage. What decays
    /// when nothing confirms this is the evidence and not the value, which is
    /// stage 2's rule and for stage 2's reason.
    pub off_conf: f32,
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
    /// How bright lens 0's half of that patch was, 0 to 1, over the same
    /// samples.
    ///
    /// Here because the pooling is a **least squares in codes** and needs the
    /// brightness as well as the ratio. That is not a taste between two
    /// averages: three poolings were run over the same readings on nine
    /// captures from three camera models and two shooters, and weighting each
    /// direction by its own brightness squared leaves the smallest step at
    /// the seam on all nine, while an equal-weight average of log ratios
    /// leaves a larger one than doing nothing at all on four of them
    /// (`kjerag-spike --bin expose`, the `models` table). The reason is in
    /// that table too: what a dark patch's ratio carries is not only the
    /// exposure, so a pooling that leans on the dark patches reads the part
    /// that is not.
    pub lit: f32,
    /// Each lens's mean Cb and Cr over that patch, signed about neutral, lens 0
    /// first: `[cb0, cr0, cb1, cr1]` (issue #103, stage 7).
    ///
    /// **Chroma and not RGB, because the frame is NV12 and a mean is linear.**
    /// The decode is a matrix, so the mean of the three channels is that matrix
    /// applied to the mean luma and the mean chroma, exactly. Storing what the
    /// planes hold rather than what they decode to means the luma half of the
    /// reading is [`Self::tone`] and [`Self::lit`] unchanged - stage 3's own
    /// numbers, from stage 3's own full-resolution sums - and the chroma half
    /// is two more numbers per lens.
    ///
    /// Read on a coarser grid than the luma, because the chroma plane is a
    /// quarter of the resolution to begin with and what is wanted from it is
    /// one mean over two degrees rather than an edge.
    pub chroma: [f32; 4],
    /// How much this direction's **colour** reading is believed, 0 to 1
    /// (issue #103, stage 7).
    ///
    /// Its own and not [`Self::confidence`], because the two answer different
    /// questions and one of them has an answer where the other has none. A
    /// direction of flat sky cannot be correlated - the band refuses it at
    /// `CONTRAST`, and on a real seam that is a fifth to two thirds of the
    /// ring - but its photometry needs no correlation to be trustworthy: what
    /// a window displaced by `e` degrees costs is `e` times the content's own
    /// gradient across it, so the flattest content is the cheapest of all to
    /// read. Measured rather than argued: at the 0.2 degree residual the pass
    /// leaves, one lens against its own displaced picture reads 0.33 to 0.76
    /// codes rms round the ring on flat content, against a colour difference of
    /// 2 to 10 codes there (docs/research/insv-format.md 6.11).
    ///
    /// So a flat direction takes 1 and a correlating one takes its own
    /// correlation, and what decays where neither is available is this and not
    /// the reading, which is the rule everywhere else in this file.
    pub hue_conf: f32,
    /// How far this direction may open the handover past its floor, 0 to 1
    /// (issue #103, stage 8).
    ///
    /// **The one thing that decides how wide the seam is, and it is a traded
    /// quantity rather than a taste.** Spreading a handover over more of the
    /// picture is how a photometric difference stops being an edge, and the
    /// price of it is that a wider handover draws more of the picture twice: a
    /// misregistration that costs nothing over two degrees is a ghost over ten.
    /// So a direction opens in proportion to how little a wider handover would
    /// cost it there, and what that costs is a quantity this pass already
    /// measures - **the content's own gradient across the patch, times the
    /// angle the pass cannot correct**:
    ///
    /// ```text
    /// ghost = (STEP_DEG + |disparity| * (1 - strength)) * texture / SPAN_DEG
    /// ```
    ///
    /// The first factor is what is left uncorrected: the correlation's own grid
    /// step wherever a direction is being tracked, and the whole disparity
    /// wherever it is not, because a reading nothing is confirming is not
    /// applied. The second is the patch's own standard deviation over its own
    /// two degrees. The product is in codes, and it is compared against one
    /// code of 255, which is the floor of the medium and this campaign's own
    /// unit since stage 3. Flat sky costs a tenth of a code and opens whole;
    /// a wing crossing the seam before the tracking has caught it costs tens of
    /// codes and does not open at all; ploughed soil at infinity is in between
    /// and opens most of the way.
    ///
    /// **It opens slowly and shuts quickly**, at [`TAU_FAR_S`] and
    /// [`TAU_NEAR_S`], which are the two constants this file already has. The
    /// asymmetry is the safety argument written as arithmetic: the cost of
    /// opening late is a seam that stays sharp for a moment longer, and the
    /// cost of shutting late is a wing drawn twice.
    pub open: f32,
    /// What is left between the two lenses at THIS direction after the pooled
    /// gain, per channel, in codes of 1: the **additive** term, per direction
    /// (issue #103, stage 8).
    ///
    /// **A gain and an offset are different phenomena, and the seam holds one
    /// of each.** A gain is two auto-exposure loops disagreeing; it is a ratio,
    /// it is a property of the two cameras, and it is therefore ONE number for
    /// the whole ring and the whole hemisphere ([`Tone`]). An offset is light
    /// scattered inside one lens onto everything it images; it is a count of
    /// photons and not a ratio, it is a property of where the SCENE's light is
    /// coming from, and it is therefore different at every azimuth. On dark
    /// content it is the whole of the difference: a step of 6.5 codes on
    /// 21-code soil needs a gain of 1.35, which is four times the widest gain
    /// ever measured and which would move the sky in the same frame by 66 codes
    /// (docs/research/seam-blending.md 2).
    ///
    /// **Per direction, because the ring was measured and it is not a shape.**
    /// Stage 7 fitted the ring's colour through a constant, one cycle and two,
    /// which is the basis the geometry uses. Measured at the owner's own
    /// reference instant that basis leaves **4.2 to 5.5 codes rms** round the
    /// ring against a frame-to-frame noise floor of 0.8 to 1.0
    /// (`kjerag-spike --bin colour`, the `rings` table): what varies round a
    /// seam is not a low-order shape and no low-order shape reaches it. So this
    /// is the reading itself, at the direction it was read at, and stage 7's
    /// five-term field is deleted rather than extended.
    ///
    /// **Applied inside the blend region only, and eased out of it.** The owner
    /// reserved on whether a player may move a hemisphere's black level, and
    /// this does not: what is corrected is the handover, so the correction is
    /// carried where the handover is and faded to nothing by the angle the two
    /// lenses stop sharing a picture at ([`fade`]). Away from the seam every
    /// hemisphere keeps the black it was delivered with. That is also what
    /// makes a per-direction number safe here where stage 3 refused one: a
    /// field over body directions does not move when the view does, and this
    /// one is gone before it reaches anything but the handover.
    ///
    /// Smoothed at [`TAU_GAIN_S`], which is the constant this file smooths
    /// things that do not move by. It needs one and stage 7 measured why: the
    /// per-direction reading is one frame's, its own noise is about a code, and
    /// a colour that breathes is motion where the scene has none.
    pub offset: [f32; 3],
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
                    "{} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}\n",
                    cell.disparity,
                    cell.confidence,
                    cell.reach_m,
                    cell.off_epi,
                    cell.off_conf,
                    cell.tone,
                    cell.lit,
                    cell.chroma[0],
                    cell.chroma[1],
                    cell.chroma[2],
                    cell.chroma[3],
                    cell.hue_conf,
                    cell.open,
                    cell.offset[0],
                    cell.offset[1],
                    cell.offset[2],
                )
            })
            .collect()
    }

    /// The same, read back. `None` on any line that is not sixteen numbers.
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
                    off_conf: next()?,
                    tone: next()?,
                    lit: next()?,
                    chroma: [next()?, next()?, next()?, next()?],
                    hue_conf: next()?,
                    open: next()?,
                    offset: [next()?, next()?, next()?],
                })
            })
            .collect()
    }

    /// What the two lenses' pictures of this patch decoded to, per channel, in
    /// codes of 1: lens 0 first.
    ///
    /// BT.709 full range, the fragment shader's own matrix, applied to the
    /// means. A mean commutes with a matrix, so this is the mean of the decoded
    /// samples exactly and not an approximation of it.
    ///
    /// WGSL twin: `decoded`.
    pub fn decoded(&self) -> [[f32; 3]; 2] {
        let lens = |luma: f32, cb: f32, cr: f32| {
            [
                luma + 1.5748 * cr,
                luma - 0.1873 * cb - 0.4681 * cr,
                luma + 1.8556 * cb,
            ]
        };
        [
            lens(self.lit, self.chroma[0], self.chroma[1]),
            lens(self.lit * self.tone.exp(), self.chroma[2], self.chroma[3]),
        ]
    }

    /// The distance to whatever is in this direction, in metres, or `None`
    /// where the disparity is zero or the wrong way round, which is
    /// everything far enough away to be at infinity as far as a 33 mm
    /// baseline is concerned.
    pub fn metres(&self) -> Option<f32> {
        (self.disparity > 0.0).then(|| self.reach_m / self.disparity)
    }
}

/// What the band says at one direction: both axes, together (issue #103,
/// stage 5).
///
/// One type rather than two arguments because they come out of one
/// correlation, at one candidate shift, and a caller that took one without the
/// other would be drawing half a measurement. `Default` is the picture stage 1
/// drew: no bend on either axis.
///
/// WGSL twin: the two locals `band_bend` computes before it builds its offset.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    /// Along [`Ring::epi`], in radians. Depth, plus what the calibration left
    /// across the seam.
    pub epi: f32,
    /// Along [`Ring::perp`], in radians. The camera, and nothing else can
    /// reach it.
    pub along: f32,
    /// How far this direction may open its handover, 0 to 1 (issue #103,
    /// stage 8). See [`Cell::open`].
    pub open: f32,
}

/// How much of the along-seam channel is parallax leaking onto an axis that
/// cannot hold any, as a correlation coefficient over the ring: **the control
/// that replaces not applying it** (issue #103, stage 5).
///
/// Until stage 5, [`Cell::off_epi`] was the control by being unused: it was
/// searched narrowly, never applied, and the claim it backed was that a
/// reading far smaller than the disparity means the band is being read as a
/// stereo pair. That claim conflated two things - no leak, and a small
/// calibration residual - and it stops being available the moment the channel
/// is applied.
///
/// This is the same discrimination, measured rather than assumed, and it is
/// strictly the sharper of the two. Parallax is one-signed and scales with
/// nearness, so if any of it were reaching this axis - a wrong baseline, a
/// mis-built [`Ring`], an epipolar axis that is not the file's - the two
/// channels would move together round the circle. They must not. `None` where
/// nothing on the ring has evidence, or where one channel does not vary.
///
/// It is reported by `kjerag-spike --bin band` on every run and it is what
/// says a corpus camera is being read as a stereo pair, not the ratio.
pub fn depth_leak(cells: &[Cell]) -> Option<f32> {
    let held: Vec<&Cell> = cells
        .iter()
        .filter(|cell| cell.confidence > 0.0 && cell.off_conf > 0.0)
        .collect();
    if held.len() < 4 {
        return None;
    }
    let count = held.len() as f32;
    let mean = |of: fn(&Cell) -> f32| held.iter().map(|cell| of(cell)).sum::<f32>() / count;
    let (mean_epi, mean_along) = (mean(|c| c.disparity), mean(|c| c.off_epi));
    let mut covariance = 0.0;
    let (mut var_epi, mut var_along) = (0.0, 0.0);
    for cell in held {
        let (epi, along) = (cell.disparity - mean_epi, cell.off_epi - mean_along);
        covariance += epi * along;
        var_epi += epi * epi;
        var_along += along * along;
    }
    match var_epi > 0.0 && var_along > 0.0 {
        true => Some(covariance / (var_epi * var_along).sqrt()),
        false => None,
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
    /// Which of this frame's [`Self::stride`] rounds of the circle it reads.
    pub slice: f32,
    /// How many frames this frame's sweep takes to cover the whole ring, which
    /// is also how many directions apart the ones it reads are.
    ///
    /// **1 on a reset frame, and that is not an optimization** (issue #103,
    /// stage 6). `reset` is a property of a FRAME and the state it throws away
    /// is per DIRECTION, so a reset frame that visits half the ring resets half
    /// the ring: the other half kept a reading of wherever the file was before
    /// the seek and decayed towards the new content over [`TAU_FAR_S`]. On a
    /// fresh file the same defect reads as attenuation - half the circle eased
    /// in from zero instead of taking its first reading whole - and it was
    /// measured at **one third of the truth after 40 frames** while the other
    /// half read it exactly (docs/research/seam-two-axis.md). A sweep of the
    /// whole ring on the one frame that resets it costs one frame's worth of
    /// the other half and is the only place the two can be made to agree.
    pub stride: f32,
    /// 1 to leave the photometry alone: the ring is still measured and the
    /// bend is still applied, and neither the pooled gain nor the
    /// per-direction offset reaches the picture.
    ///
    /// An instrument's, and only an instrument's
    /// (`ScenePipeline::hold_tone`). It is what makes a before and after
    /// differ by this stage and by nothing else, and since stage 8 it has to
    /// reach the compute pass rather than only the dispatch list, because the
    /// correction the picture is drawn with is written per direction by the
    /// measurement itself.
    pub hold: f32,
    /// A uniform block's size rounds up to sixteen bytes. WGSL does that
    /// itself; `repr(C)` does not, and the two sizes have to agree.
    _pad: [f32; 3],
}

impl Watch {
    /// The first frame of a file, and the first after a seek: the **whole
    /// ring**, thrown away and read again.
    pub fn start(seconds: f32) -> Self {
        Self {
            seconds,
            reset: 1.0,
            slice: 0.0,
            stride: 1.0,
            hold: 0.0,
            _pad: [0.0; 3],
        }
    }

    /// One slice of the ring, aged by what that slice has actually aged by.
    pub fn track(seconds: f32, slice: u32) -> Self {
        Self {
            // What a direction of this slice has actually aged by: it was last
            // read SLICES frames ago, not one. The filter is paced in seconds,
            // so telling it the truth is the whole of what slicing costs.
            seconds: seconds * SLICES as f32,
            reset: 0.0,
            slice: slice as f32,
            stride: SLICES as f32,
            hold: 0.0,
            _pad: [0.0; 3],
        }
    }

    /// How many workgroups this frame dispatches: one per direction it reads.
    pub fn groups(&self) -> u32 {
        AZIMUTHS as u32 / (self.stride as u32).max(1)
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
    ///
    /// The same axis and the same sign as [`super::seam::Where::across`],
    /// turned 0.6 to 3.5 degrees off it by the baseline's own tilt.
    pub epi: [f32; 3],
    /// The axis a distance cannot reach, unit: the seam circle's own tangent
    /// towards increasing azimuth, with the epipolar tilt taken out.
    ///
    /// **The same axis and the same SIGN as [`super::seam::Where::along`]**,
    /// and that is a requirement rather than an observation
    /// (`the_two_instruments_name_the_same_two_axes`). Two instruments measure
    /// this axis - this pass, and `--bin seam mode=residual` through
    /// `seam::ring` - and the harmonic decomposition that names a constant
    /// along it a relative roll is stated in the second one's convention. Built
    /// as `centre x epi` it comes out **negated**, which is a picture the pass
    /// still draws correctly, because it measures and applies through the same
    /// axis and the two signs cancel; what it is not is comparable. That cost
    /// issue #103 stage 5 its diagnosis: the two instruments were read side by
    /// side, and a sign is invisible where a quantity passes through zero and
    /// total where it does not (docs/research/seam-two-axis.md).
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
            // `epi x centre` and not `centre x epi`: the second is the same
            // line the other way up, and the way up is the one `seam::ring`
            // already publishes. See [`Self::perp`].
            perp: unit(cross(epi, centre)),
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

/// The exposure the whole ring's far field agrees on, as a natural log, and
/// what share of the ring was behind it (issue #103, stage 3).
///
/// **Rust twin of the `pool` entry point**, and a twin rather than a
/// description: the pass runs this arithmetic on the GPU where no test can
/// reach it, and every property claimed for it below is claimed about a
/// function `cargo test` can call with no device and no footage.
///
/// Three things decide it and each is measured rather than chosen
/// (`kjerag-spike --bin expose`, run over six of the owner's flights and an
/// X5, an X4 and an X3 from other shooters):
///
/// - **Far field only**, at [`NEAR_KNEE_DEG`], which is the knee this file
///   already has. Past it a direction is looking at something inside 10 m,
///   which is the hardest content to line up and the darkest content on a
///   flight, and a photometry taken there reads the alignment rather than the
///   exposure. Including it makes the step a fitted gain leaves 15.7 codes on
///   the worst capture instead of 3.6, and it produces an apparent additive
///   term that disappears the moment the knee is applied.
/// - **Least squares in codes**, so a direction weighs its own brightness
///   squared. Of three poolings tried on the same readings this is the only
///   one that lowers the step at the seam on every capture; an equal-weight
///   average of log ratios raises it on most of them, because it leans on
///   exactly the dark patches where the difference is least an exposure.
/// - **Trust**, which is the same `confidence / KEEP` the bend applies
///   itself. No second threshold: a direction the bend has stopped believing
///   is not one this should be believing either.
///
/// `None` where nothing on the ring is confirming anything, which is a
/// one-lens file, a file's first frame, and a seam with nothing far-field on
/// it. The caller keeps what it had; the exposure of two lenses does not
/// change because we stopped being able to see it.
/// Whether this direction is looking at something inside the near knee, and is
/// therefore no place to read a photometry (issue #103, stage 3).
///
/// **Measured on the disparity the pass is actually drawing with**, which is
/// the reading times the strength its own evidence has earned (issue #103,
/// stage 7). A disparity is kept when a direction stops correlating and only
/// the evidence behind it is given up, which is this file's rule everywhere -
/// so a direction that once read a boot at 3 m and is now flat sky still
/// carries the boot's number, and a cut that read the number alone kept that
/// direction out of the colour for the rest of the file. That is the same
/// defect stage 2 was sent back for, an expired reading applied at full
/// strength, and it is the same fix.
///
/// No new constant: the strength is `super::projection::Reframe::channel`'s
/// own, so a direction fades out of the near field exactly as its bend fades
/// out of the picture. It changes nothing about stage 3's gain, because a
/// direction with no confidence weighed exactly zero there anyway; what it
/// lets in is the flat content stage 7 added, whose confidence is zero
/// because a flat patch cannot be correlated and whose colour is the best on
/// the ring.
///
/// WGSL twin: `near_field`.
fn near_field(cell: &Cell) -> bool {
    let strength = (cell.confidence / KEEP).clamp(0.0, 1.0);
    cell.disparity.abs() * strength >= NEAR_KNEE_DEG.to_radians()
}

/// The gain AND the offset the whole ring's far field agrees on, per channel,
/// **in ratio space**, and what share of the ring was behind them (issue #103,
/// stages 3 and 8).
///
/// **Rust twin of the `pool` entry point**, and a twin rather than a
/// description: the pass runs this arithmetic on the GPU where no test can
/// reach it, and every property claimed for it below is claimed about a
/// function `cargo test` can call with no device and no footage.
///
/// **THE LOSS IS THE ONE AN EYE USES, AND THAT IS THE WHOLE CHANGE.** Stage 3
/// chose least squares in codes on nine captures, so a direction weighed its
/// own brightness squared: a direction on 20-code soil carried one percent of
/// the weight of one on 190-code sky, and the pass was therefore fitted almost
/// entirely on the content where its artifact is invisible. An eye judges a
/// step against what it is a step OF - 6.5 codes is 31 percent of soil and 3.4
/// percent of sky - so the residual here is divided by the level it sits on
/// before it is squared. Nothing else about the estimator moved; the weight
/// went from `L^2` to `1/L^2`, which is four orders of magnitude and is why
/// stage 3's answer and this one are different numbers on the same readings.
///
/// **Two parameters, because the seam holds two phenomena.** `high = gain *
/// low + offset` per channel. They are separated by the ring's own range of
/// brightness and by nothing else, so where a ring has no range the ridge
/// splits the difference between them - which gives the offset half, and the
/// offset is the correction that moves less of the picture ([`Tone`]).
///
/// **Far field only**, at [`NEAR_KNEE_DEG`], and **trust**, which is the same
/// `hue_conf / KEEP` the colour is believed at: both are stage 3's, both are
/// unchanged, and both were measured before they were chosen.
///
/// `None` where nothing on the ring is confirming anything, which is a
/// one-lens file, a file's first frame, and a seam with nothing far-field on
/// it. The caller keeps what it had; the exposure of two lenses does not
/// change because we stopped being able to see it.
pub fn pooled_tone(cells: &[Cell]) -> Option<([f32; 3], [f32; 3], f32)> {
    let mut level = [0.0f32; 3];
    let mut count = 0.0;
    let held: Vec<(f32, [f32; 3], [f32; 3])> = cells
        .iter()
        .filter(|cell| !near_field(cell))
        .filter_map(|cell| {
            let believed = (cell.hue_conf / KEEP).clamp(0.0, 1.0);
            let [low, high] = cell.decoded();
            (believed > 0.0).then_some((believed, low, high))
        })
        .collect();
    // The ring's own level, per channel, which is what the two parameters are
    // measured against: an offset in codes and a gain in nothing are not
    // comparable until one of them is divided by a brightness, and the ridge
    // below cannot mean one direction's worth on both columns until they are.
    for (believed, low, high) in &held {
        for channel in 0..3 {
            level[channel] += believed * 0.5 * (low[channel] + high[channel]);
        }
        count += believed;
    }
    if count <= 0.0 {
        return None;
    }
    let level = level.map(|total| total / count);
    let mut log_gain = [0.0f32; 3];
    let mut offset = [0.0f32; 3];
    for channel in 0..3 {
        if level[channel] <= 0.0 {
            return None;
        }
        // The two-by-two normal system for `high = gain * low + offset`, with
        // the ridge already on its diagonal and already shrinking the gain
        // towards exactly 1 and the offset towards exactly 0.
        let (mut aa, mut ab, mut bb, mut ya, mut yb) = (RIDGE, 0.0f32, RIDGE, RIDGE, 0.0f32);
        for (believed, low, high) in &held {
            let mid = 0.5 * (low[channel] + high[channel]);
            if mid <= 0.0 || low[channel] <= 0.0 || high[channel] <= 0.0 {
                continue;
            }
            // Ratio space: the residual is divided by the level it sits on
            // before it is squared, which in a least squares is a weight of one
            // over that level squared.
            let weight = believed * (level[channel] / mid).powi(2);
            let (u, v) = (
                low[channel] / level[channel],
                high[channel] / level[channel],
            );
            aa += weight * u * u;
            ab += weight * u;
            bb += weight;
            ya += weight * u * v;
            yb += weight * v;
        }
        let det = aa * bb - ab * ab;
        if det <= 0.0 {
            return None;
        }
        let gain = (ya * bb - yb * ab) / det;
        if gain <= 0.0 {
            return None;
        }
        log_gain[channel] = gain.ln().clamp(-LIMIT_LN, LIMIT_LN);
        offset[channel] = ((aa * yb - ab * ya) / det * level[channel]).clamp(-LIMIT_OFF, LIMIT_OFF);
    }
    Some((log_gain, offset, count / AZIMUTHS as f32))
}

/// The disparity the shader may actually bend by, in radians: what was
/// measured, clamped to what the crossover can carry without folding.
///
/// `band` is the crossover width in radians, which since stage 4 is
/// [`width`]'s answer for that same disparity rather than a constant. See
/// [`FOLD`].
///
/// WGSL twin: `carried`.
/// How far a direction may open its handover, from what a wider one would cost
/// it there (issue #103, stage 8). See [`Cell::open`], which is what this is
/// smoothed into.
///
/// `texture` is the patch's own standard deviation in codes of 1, and
/// `strength` is how much of this direction's reading the pass is actually
/// drawing with, which is [`super::projection::Reframe::channel`]'s own.
///
/// WGSL twin: `openness`.
pub fn openness(disparity_rad: f32, strength: f32, texture: f32) -> f32 {
    let left = STEP_DEG.to_radians() + disparity_rad.abs() * (1.0 - strength.clamp(0.0, 1.0));
    let ghost = left / SPAN_DEG.to_radians() * texture;
    let t = (ghost / ONE_CODE).clamp(0.0, 1.0);
    1.0 - t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

pub fn carried(disparity_rad: f32, band_rad: f32) -> f32 {
    // [`SLOPE`] since stage 8, and it is the same inequality: the shear is the
    // bend's own gradient across the band, which is the disparity times how
    // steep the profile carrying it gets. A straight line's steepest is 1 and
    // this profile's is 15/8, so the same band carries eight fifteenths as
    // much - and [`width`] opens it by exactly that factor to compensate, so
    // nothing the search can report is thrown away.
    let limit = FOLD * band_rad / SLOPE;
    disparity_rad.clamp(-limit, limit)
}

/// How wide the handover is at one direction, in radians: **the one width**
/// (issue #103, stages 4 and 8).
///
/// Three questions used to have three answers near the seam - how far the two
/// lenses are mixed over (stage 4), how far the colour field reaches (stage 7),
/// and how far a photometric correction is carried. They are one question, and
/// this is the answer to it. Everything downstream reads this number:
/// [`super::projection::crossover`] hands the picture over across it,
/// [`carried`] clamps the bend to it, and [`fade`] eases the photometry out
/// past it.
///
/// Four things decide it and each one is a measurement:
///
/// - **The fold**, `|disparity| * SLOPE / FOLD`, which is stage 4's inequality
///   at the profile's real slope. A width under this prints the picture back
///   over itself, so it is a floor and not a preference. It is the whole of
///   why near-field content keeps a narrow-band character: a direction reading
///   degrees of disparity is opened by its own arithmetic and not by an eye's
///   wish.
/// - **What the content allows**, [`Cell::open`], which prices the ghost a
///   wider handover would cost in that direction. Zero leaves the floor exactly
///   where stage 4 left it; one asks for the whole of what the optics have.
/// - **The floor and the ceiling.** The floor is the crossover the projection
///   ships, so no view ever gets a handover narrower than the two degrees the
///   owner validated. The ceiling is half the angle the two lenses share a
///   picture over, which is all there is: past it one of them has no picture to
///   hand over with.
///
/// **A count of pixels of the delivered view was built here and then measured
/// out.** Stage 8 opened on the complaint that two degrees is 102 pixels at fov
/// 20 and 18 at fov 114, so the first form of this asked for a fixed number of
/// pixels. It decided nothing: at every field of view the player offers, the
/// optics' ceiling or the content's own price is reached first, and the one
/// place it did bite it made the handover NARROWER than the content would have
/// borne (4.70 degrees against 5.39 at the owner's own wide view). It is
/// deleted with its constant.
///
/// **It still needs no time constant of its own**, which is stage 4's property
/// and is kept: both readings it moves with, the disparity and the openness,
/// are already smoothed per direction, so a width cannot flicker faster than
/// what it is made of.
///
/// WGSL twin: `band_width`.
pub fn width(disparity_rad: f32, open: f32, floor_rad: f32, ceiling_rad: f32) -> f32 {
    let ceiling = ceiling_rad.max(floor_rad);
    // What the reading needs, or the picture folds. Stage 4's inequality
    // exactly, at the slope the profile actually has.
    let fold = (disparity_rad.abs() * SLOPE / FOLD).min(WIDEST_DEG.to_radians());
    // As wide as the content will bear, with the optics as the other end of
    // it. There is no third term: a count of pixels of the delivered view was
    // built here and measured out (see above).
    (floor_rad + open.clamp(0.0, 1.0) * (ceiling - floor_rad))
        .max(fold)
        .min(ceiling)
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
         const RIDGE = {RIDGE:?};\n\
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
         const HUE_STEP = {hue_step}u;\n\
         const SPAN = {span_rad:?};\n\
         const ONE_CODE = {ONE_CODE:?};\n\
         const LIMIT_OFF = {LIMIT_OFF:?};\n\
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
        hue_step = ((2 * half + 1) / HUE_TAPS).max(1),
        span_rad = SPAN_DEG.to_radians(),
    )
}

/// How many chroma samples a patch is read along each axis (issue #103,
/// stage 7).
///
/// The luma grids are already in workgroup memory and a chroma sample has to be
/// fetched, so this is the whole cost of the colour reading: seven a side is 49
/// taps a lens against the patch's 441, which is three percent of what the pass
/// already fetches per direction. What is wanted from the chroma plane is one
/// mean over two degrees, on a plane the encoder carries at a quarter of the
/// luma's resolution because an eye cannot see edges in it, and 49 samples put
/// the mean's own standard error an order under the differences being measured.
const HUE_TAPS: usize = 7;

/// The lookup half, which the fragment shader reads: the bend one ray takes.
///
/// Separate from [`wgsl`] because the two pipelines want different halves.
/// The render pass never runs the correlation and the compute pass never
/// bends a ray, and each declares the storage buffer with the access it
/// needs: `read` in the fragment shader, `read_write` in the compute one.
pub(crate) fn lookup_wgsl() -> String {
    format!(
        "const AZIMUTHS = {AZIMUTHS}u;\nconst FOLD = {FOLD:?};\nconst KEEP = {KEEP:?};\n\
         const WIDEST = {widest:?};\nconst SLOPE = {SLOPE:?};\n\
         const TAU = {tau:?};\nconst LIMIT_LN = {LIMIT_LN:?};\nconst LIMIT_OFF = {LIMIT_OFF:?};\n\
         {CELL}{RING}{LOOKUP}",
        widest = WIDEST_DEG.to_radians(),
        tau = std::f32::consts::TAU,
    )
}

/// How much of the photometric correction reaches a direction whose sine out of
/// the seam plane is `off_seam` (issue #103, stages 7 and 8).
///
/// **Whole across the handover, and eased to nothing AT THE POLE.** Whole
/// across the handover because that is what makes the step vanish rather than
/// move: the two lenses' pictures are mixed there, and a correction that faded
/// across the mix would leave the difference behind, spread out.
///
/// **The outer end is the pole, and that is the owner's own ruling**
/// (2026-08-01, on stage 8's first form: *"I dont think its aggressive enough
/// with blending"*). That form ended at the overlap, seven degrees off the seam,
/// on the argument that past it "how these two differ here" is not a statement
/// anything can check. What it leaves is the whole correction ramped over four
/// degrees, which is what he was looking at. **The symmetric split is what
/// dissolves the objection that shape was protecting against**: each hemisphere
/// moves HALF the mismatch towards the other, so neither is given a black level
/// that is not its own, and that is the same argument stage 3 used to split a
/// gain between two hemispheres.
///
/// The pole is the only end here that is not a taste. An azimuth is what the
/// field is read at and a pole has none, so a field carried to one has to
/// arrive single-valued; arriving at zero is how, and it costs no constant.
///
/// The inner end is [`width`]'s answer at this direction, which is what makes
/// it one region rather than a second one: stage 7 faded from [`WIDEST_DEG`], a
/// constant that had nothing to do with how wide the handover at that direction
/// actually was.
///
/// **A fifth-order ease and not a `smoothstep`**, so the profile has no corner
/// AND no kink in its slope at either end. A corner in a gradient is a Mach
/// band, which is the artifact this stage exists to remove rather than to move,
/// and a jump in the second derivative is a fainter one.
///
/// Exactly zero on a camera whose lenses do not overlap at all, which is every
/// file with one lens stream, at every view: issue #39's byte-identity.
///
/// WGSL twin: `tint_fade`.
pub fn fade(off_seam: f32, half_floor_rad: f32, half_overlap_rad: f32) -> f32 {
    if half_overlap_rad <= 0.0 {
        // A file with one lens stream: the two never share a picture, so there
        // is no handover to correct and nothing to correct it with. The only
        // question the overlap is still asked.
        return 0.0;
    }
    let inner = half_floor_rad.sin().min(1.0);
    if inner >= 1.0 {
        return f32::from(u8::from(off_seam.abs() < 1.0));
    }
    let t = ((off_seam.abs() - inner) / (1.0 - inner)).clamp(0.0, 1.0);
    1.0 - t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
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

/// How many bytes the state buffer is: the pooled [`Tone`], the pooled
/// [`Along`], then one [`Cell`] per direction.
///
/// One header shorter than stage 7's: the ring's colour field is deleted and
/// what it was reaching for is [`Cell::offset`], at the direction it was
/// measured at (issue #103, stage 8).
pub(crate) const BYTES: u64 = (CELLS_AT + AZIMUTHS * std::mem::size_of::<Cell>()) as u64;

/// Where the along-seam field starts in that buffer.
pub(crate) const ALONG_AT: usize = std::mem::size_of::<Tone>();

/// Where the smooth additive field starts in that buffer.
pub(crate) const GLARE_AT: usize = ALONG_AT + std::mem::size_of::<Along>();

/// Where the cells start in that buffer, for the readback that unpacks it.
pub(crate) const CELLS_AT: usize = GLARE_AT + std::mem::size_of::<Glare>();

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
  log_gain: vec3<f32>,
  evidence: f32,
};

struct Cell {
  disparity: f32,
  confidence: f32,
  reach_m: f32,
  off_epi: f32,
  off_conf: f32,
  tone: f32,
  lit: f32,
  chroma: array<f32, 4>,
  hue_conf: f32,
  // Three floats and not a `vec3<f32>`: a vec3 in a storage buffer aligns to
  // sixteen bytes, which would pad every cell of the array out from 64 to 80
  // and make the shader's idea of this buffer disagree with `repr(C)`'s.
  open: f32,
  offset: array<f32, 3>,
};

// What one lens's mean luma and mean chroma decode to, per channel, in codes
// of 1. BT.709 full range, the fragment shader's own matrix. A mean commutes
// with a matrix, so this is the mean of the decoded samples and not an
// approximation of it. Rust twin: `Cell::decoded`.
fn decoded(luma: f32, cb: f32, cr: f32) -> vec3<f32> {
  return vec3<f32>(
    luma + 1.5748 * cr,
    luma - 0.1873 * cb - 0.4681 * cr,
    luma + 1.8556 * cb,
  );
}

struct Along {
  terms: array<f32, 5>,
  evidence: f32,
  pad0: f32,
  pad1: f32,
};

struct Glare {
  terms: array<f32, 15>,
  evidence: f32,
};

struct State {
  tone: Tone,
  along: Along,
  glare: Glare,
  cells: array<Cell, AZIMUTHS>,
};

// The additive term at one azimuth, per channel, in codes of 1. A direction
// flattened into the seam plane IS (cos, sin), so no trig reaches the fragment
// shader. Rust twin: `Glare::at`.
fn glare_at(field: Glare, cos: f32, sin: f32) -> vec3<f32> {
  let basis = array<f32, 5>(1.0, cos, sin, cos * cos - sin * sin, 2.0 * cos * sin);
  var out: vec3<f32>;
  for (var channel = 0u; channel < 3u; channel += 1u) {
    let at = 5u * channel;
    out[channel] = field.terms[at] * basis[0]
      + field.terms[at + 1u] * basis[1]
      + field.terms[at + 2u] * basis[2]
      + field.terms[at + 3u] * basis[3]
      + field.terms[at + 4u] * basis[4];
  }
  return clamp(out, vec3<f32>(-LIMIT_OFF), vec3<f32>(LIMIT_OFF));
}

// The along-seam correction at one azimuth, from that azimuth's own cosine and
// sine. A direction flattened into the seam plane IS (cos, sin), so no trig
// reaches the fragment shader. Rust twin: `Along::at`.
//
// Written out rather than looped because it runs per FRAGMENT and a loop
// indexing an array by a running variable is what `blend`'s own comment warns
// about. On RADV the two measure the same, 1.48 against 1.47 ms per redraw at
// 2560x1440, so this is the trap avoided rather than a cost recovered.
fn along_at(field: Along, cos: f32, sin: f32) -> f32 {
  return field.terms[0]
    + field.terms[1] * cos
    + field.terms[2] * sin
    + field.terms[3] * (cos * cos - sin * sin)
    + field.terms[4] * (2.0 * cos * sin);
}
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
  // `epi x centre`, which is the seam circle's own tangent towards increasing
  // azimuth and the sign `seam::ring` publishes. Rust twin: `Ring::at`.
  out.perp = normalize(cross(out.epi, centre));
  return out;
}
"#;

const LOOKUP: &str = r#"
@group(1) @binding(0) var<storage, read> band: State;

// What lens 0's picture is multiplied by, per channel (issue #103, stages 3
// and 7). Lens 1 takes the reciprocal, which is `tone_split_back`.
//
// The split is symmetric because the seam cannot say which lens is wrong: a
// correction of +x on one and -x on the other is the same picture at the
// handover, and halving it is what keeps either hemisphere from carrying the
// whole change. Three channels rather than stage 3's one, because a single
// multiplier common to R, G and B cannot change what the two lenses disagree
// about in HUE, however well it is fitted, and that residue is 1.6 to 15.6
// codes across the corpus.
//
// Exactly one on every channel of both sides when nothing has been measured,
// and by an equality rather than by trusting `exp(0.0)`: a file with one lens
// stream, a seam that has never correlated and every frame before the first
// reading all reach that line, and every pixel they draw is the one stage 2
// drew. Rust twin: `Tone::split`.
fn tone_half() -> vec3<f32> {
  return 0.5 * clamp(band.tone.log_gain, vec3<f32>(-LIMIT_LN), vec3<f32>(LIMIT_LN));
}

fn tone_split() -> mat2x3<f32> {
  let half = tone_half();
  if all(half == vec3<f32>(0.0)) {
    return mat2x3<f32>(vec3<f32>(1.0), vec3<f32>(1.0));
  }
  return mat2x3<f32>(exp(half), exp(-half));
}

// The same split with what the ring says at THIS direction added to it, faded
// out by how far the direction is from the seam (issue #103, stage 7). Rust
// twin: `Reframe::colour_split`.
//
// `band.azimuth` is the ray flattened into the seam plane and `band.off_seam`
// is the sine of its angle out of that plane, both computed by `band_bend` for
// the geometry and reused here rather than recomputed.
//
// Exactly one on every channel of both sides when nothing has been measured,
// which is the byte-identity `tone_split` already promises: a zero field times
// any fade is zero, and `tone_half` of zero takes the equality above.
fn colour_split(at: Band) -> mat2x3<f32> {
  return tone_split();
}

// What is ADDED to each lens's picture at this ray, per channel, lens 0 first
// (issue #103, stage 8): the offset, split symmetrically like the gain, and
// carried only where the handover is.
//
// Whole across the blend region, because a correction that faded across the
// mix would leave the difference behind rather than remove it, then eased to
// nothing by the angle the two lenses stop sharing a picture at. So no
// hemisphere's black level moves: away from the seam every picture keeps the
// black it was delivered with, which is the reservation the owner made on
// stage 3.
//
// Exactly zero on both sides when nothing has been measured, and by an
// equality rather than by trusting a multiply. Rust twin: `Tone::lift`.
fn tone_lift(at: Band) -> mat2x3<f32> {
  let half = 0.5 * tint_fade(at) * glare_at(band.glare, at.azimuth.x, at.azimuth.y);
  if all(half == vec3<f32>(0.0)) {
    return mat2x3<f32>(vec3<f32>(0.0), vec3<f32>(0.0));
  }
  return mat2x3<f32>(half, -half);
}

// How much of the photometric correction reaches a direction this far out of
// the seam plane, given as the sine of that angle.
//
// Whole across THIS ray's own handover, so the correction is one number over
// the width the two lenses are actually mixed at and the step it removes is
// removed exactly rather than spread out. Then out to where the two lenses stop
// sharing a picture, which the file's own calibration says and this pass is
// already told (`Reframe::overlap`).
//
// Since stage 8 the inner end is the handover's own width at this direction
// and not a constant, which is what makes the colour region and the crossover
// ONE region instead of two; and the outer end is the pole, because the
// correction is SPLIT between the two hemispheres and half of a mismatch is not
// a black level a hemisphere has to be protected from.
fn tint_fade(at: Band) -> f32 {
  if reframe.half_overlap <= 0.0 {
    return 0.0;
  }
  // THE POLE, which is the one end that is not a taste: an azimuth is what the
  // field is read at and a pole has none. Rust twin: `fade`.
  //
  // The inner end is the SHIPPED crossover and not this direction's own width,
  // and that is the second half of the anti-striping rule (issue #103, stage 8,
  // after the rejection). The width is a per-direction quantity; multiplying a
  // smooth field by a per-direction shape puts the per-direction shape straight
  // back into the picture, which is what the first salvage still measured at
  // 0.90 percent of interior roughness. It costs nothing to give up: at the
  // widest handover the two lenses can share, this fade is still 0.999.
  let inner = min(sin(0.5 * CROSSOVER), 1.0);
  let away = abs(at.off_seam);
  if inner >= 1.0 {
    return select(0.0, 1.0, away < 1.0);
  }
  let t = clamp((away - inner) / (1.0 - inner), 0.0, 1.0);
  return 1.0 - t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

// The band with nothing behind it: no bend, and the crossover at the width it
// has always been. This is what a file with one lens stream takes, what a
// direction that has never correlated takes, and what a ray straight down a
// lens's own axis takes, and it is the picture stage 1 drew.
fn band_rest() -> Band {
  var out: Band;
  out.offset = vec3<f32>(0.0);
  out.along = vec3<f32>(0.0);
  out.crossover = CROSSOVER;
  // Straight down a lens's own axis there is no azimuth, so there is no ring
  // reading either, and the fade answers zero out there anyway.
  out.azimuth = vec2<f32>(1.0, 0.0);
  out.off_seam = 1.0;
  return out;
}

// The bend a ray takes, in view space, scaled by the ray's own length so that
// adding it turns the ray by the reading in radians, and how wide the
// handover has to be to carry it. Rust twin: `Reframe::blend_bent`, which
// computes the same two things from `Reframe::reading_at`.
//
// TWO AXES since stage 5. The epipolar term is depth and the along-seam term
// is the camera, they are read from one correlation at one candidate shift,
// and each is applied at its own channel's evidence. Only the epipolar one
// can fold and only the epipolar one opens the band: see `band_width`.
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
  // Each axis weighted by the evidence behind ITS OWN channel, not just by
  // which cell is nearer. A direction that has stopped correlating stops
  // contributing, both to what the reading is and to how much of it is
  // applied, and a ray between one live cell and one dead one takes the live
  // one's answer at the dead one's strength. With no evidence at all the bend
  // is zero and `band_width` returns the shipped crossover, which is exactly
  // the picture before this existed: the fallback is stage 1 and it is reached
  // by arithmetic rather than by a branch. Rust twin: `Reframe::reading_at`.
  let applied = carry(a.disparity, a.confidence, b.disparity, b.confidence, mix);
  // Straight between the two cells: the openness is already a smoothed state
  // and it means what it says at a direction that never correlated, which is
  // most of a sky seam. Rust twin: `Reframe::reading_at`.
  let open = mix2(a.open, b.open, mix);
  // The along-seam axis is NOT read cell by cell. It is one fitted field over
  // the whole circle, because the phenomenon is one - a relative pose error
  // with a constant, a one-cycle and a two-cycle term - and because a field
  // with holes in it, applied over a whole hemisphere, warps a horizon instead
  // of moving it. `flat / reach` is this azimuth's cosine and sine already.
  // Rust twins: `Along::at` and `Reframe::reading_at`.
  let along = along_at(band.along, flat.x / reach, flat.y / reach);
  // The epipolar bend's own gradient across the band is the disparity over the
  // band width, and past 1 the mapping folds. The band opens far enough to
  // carry this reading, and the clamp holds where it cannot. Rust twins:
  // `width` and `carried`.
  //
  // The along-seam bend asks neither of them. Its gradient is across the band
  // and its displacement is along it, so the Jacobian it adds is off-diagonal
  // and the determinant stays exactly 1: a shear perpendicular to its own
  // gradient cannot fold, however wide it opens. Rust twin: `Reframe::bent`.
  var out: Band;
  // The seam geometry this fragment has already worked out, carried rather
  // than recomputed: the azimuth is what the colour field is read at and the
  // sine out of the seam plane is what fades it (issue #103, stage 7).
  out.azimuth = flat / reach;
  out.off_seam = body.z / length(body);
  out.crossover = band_width(applied, open);
  let limit = FOLD * out.crossover / SLOPE;
  let carried = clamp(applied, -limit, limit);
  // Back into view space: view_to_body is a rotation, so its transpose is its
  // inverse, and `v * m` is `transpose(m) * v`.
  out.offset = (carried * length(ray)) * (at.epi * reframe.view_to_body);
  // Scaled by the FLATTENED length and not the whole one, which is the
  // `cos(elevation)` a relative roll about the body's z produces: `w x d` is
  // `|w| cos(elevation)` along the seam's own tangent everywhere, and exactly
  // zero at both lens poles, where an azimuth does not exist and a per-azimuth
  // correction would otherwise swirl. Rust twin: `Reframe::bent`.
  out.along = (along * reach) * (at.perp * reframe.view_to_body);
  return out;
}

// One channel of one ray: the two cells' values mixed at their own evidence,
// then taxed by how much of that evidence reaches `KEEP`.
//
// `KEEP` is the correlation a single reading has to reach before it may move
// the state at all, and a confidence is the smoothed value of that same
// number, so a direction whose recent readings have not been reaching that
// gate is applied proportionally less. No new constant: the threshold a
// reading must pass is the threshold a smoothed reading is trusted at. Zero
// evidence gives exactly zero, by arithmetic. Rust twin: `Reframe::channel`.
fn carry(a: f32, wa: f32, b: f32, wb: f32, mix: f32) -> f32 {
  let ea = wa * (1.0 - mix);
  let eb = wb * mix;
  let total = ea + eb;
  if total <= 0.0 {
    return 0.0;
  }
  let strength = clamp(mix2(wa, wb, mix) / KEEP, 0.0, 1.0);
  return (ea * a + eb * b) / total * strength;
}

// The ONE width: the fold's demand, the eye's demand in pixels of this view,
// what the content there will bear, the crossover as the floor and half the
// lenses' shared angle as the ceiling. Rust twin: `width`, which argues all
// four.
//
// The EPIPOLAR reading only, which is what keeps stage 4's fold guarantee
// exactly where stage 4 put it: the along-seam bend does not fold and therefore
// does not ask the band for room.
fn band_width(disparity: f32, open: f32) -> f32 {
  let ceiling = max(reframe.half_overlap, CROSSOVER);
  let fold = min(abs(disparity) * SLOPE / FOLD, WIDEST);
  let allowed = CROSSOVER + clamp(open, 0.0, 1.0) * (ceiling - CROSSOVER);
  return min(max(allowed, fold), ceiling);
}

fn mix2(a: f32, b: f32, t: f32) -> f32 {
  return a + (b - a) * t;
}
"#;

const WGSL: &str = r#"
// The same group the draw binds, so the band correlates the very pictures the
// frame after it will sample. The chroma planes are declared since stage 7 and
// read only by the photometry: a doubled edge is geometry, geometry is in the
// luma, and the correlation never touches these.
@group(0) @binding(1) var luma0: texture_2d<f32>;
@group(0) @binding(2) var chroma0: texture_2d<f32>;
@group(0) @binding(3) var luma1: texture_2d<f32>;
@group(0) @binding(4) var chroma1: texture_2d<f32>;
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

// Cb and Cr, signed about neutral, for the photometry alone (issue #103,
// stage 7). DRM_FORMAT_GR88 is little endian G:R, so .r is Cb and .g is Cr -
// the same reading `scene`'s own `nv12` makes of the same two textures.
fn chroma_at(index: u32, uv: vec2<f32>) -> vec2<f32> {
  if index == 0u {
    return textureSampleLevel(chroma0, samp, uv, 0.0).rg - vec2<f32>(0.5);
  }
  return textureSampleLevel(chroma1, samp, uv, 0.0).rg - vec2<f32>(0.5);
}

// One sample of one lens's colour, or the neutral where the lens has no
// picture there. The caller has already established from the luma grids that
// this sample is a pair, so what is left is the fetch.
fn hue_tap(index: u32, aim: mat3x3<f32>, ray: vec3<f32>) -> vec2<f32> {
  let landing = look(index, aim, ray);
  if !landing.inside {
    return vec2<f32>(0.0);
  }
  return chroma_at(index, frame_uv(landing.pixel));
}

struct Watch {
  seconds: f32,
  reset: f32,
  // Which of this frame's `stride` rounds of the circle it reads.
  slice: f32,
  // How many directions apart the ones this frame reads are: 1 on a reset
  // frame, which sweeps the whole ring so that a reset reaches every
  // direction and not only the slice it landed on. Rust twin: `Watch::stride`.
  stride: f32,
  // 1 to leave the photometry alone, which is an instrument's and only an
  // instrument's. Rust twin: `Watch::hold`.
  hold: f32,
  pad0: f32,
  pad1: f32,
  pad2: f32,
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
// Whether the front patch has enough picture in it to correlate at all. Read
// ONCE per direction rather than inside every candidate: flat sky correlates
// with anything, and on a real seam most of the ring is sky, so this is the
// difference between a flat direction costing one patch and costing the whole
// table (issue #103, stage 6).
var<workgroup> textured: bool;
// How much picture that patch has, in codes of 1, or -1 where part of it is
// outside the lens. The correlation reads it as a gate and the width reads it
// as a price (issue #103, stage 8).
var<workgroup> spread: f32;
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
// The colour half of the same reading, on a coarser grid (issue #103,
// stage 7). The chroma plane is a quarter of the luma's resolution to begin
// with and what is wanted from it is one mean over two degrees, so it is
// sampled every HUE_STEP along each axis - about fifty taps a lens against the
// patch's 441, which is what keeps this a few percent of the pass rather than
// a third of it.
var<workgroup> hue0: array<vec2<f32>, THREADS>;
var<workgroup> hue1: array<vec2<f32>, THREADS>;
var<workgroup> hue_n: array<f32, THREADS>;
// The pooling's own two, because it is a second entry point over the same
// buffer and not a second use of the same patch: what these hold is one
// number per LANE over the whole ring, not one per sample of one direction.
var<workgroup> pooled_total: array<vec3<f32>, THREADS>;
var<workgroup> pooled_count: array<f32, THREADS>;

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
  // Every `stride`-th direction, one further round each frame - and every
  // direction on a reset frame, where the stride is 1.
  let cell = group.x * u32(watch.stride) + u32(watch.slice);
  let at = ring_of(f32(cell) / f32(AZIMUTHS) * TAU);
  let aim0 = body_to_lens(0u);
  let aim1 = body_to_lens(1u);

  // The front lens's patch first, and on its own: whether there is anything in
  // it to correlate decides whether the rest of this workgroup runs at all.
  for (var i = lane; i < PATCH; i += THREADS) {
    let a = f32(i32(i % u32(2 * HALF + 1)) - HALF) * STEP;
    let b = f32(i32(i / u32(2 * HALF + 1)) - HALF) * STEP;
    front[i] = tap(0u, aim0, at.centre + a * at.perp + b * at.epi);
  }
  if lane == 0u {
    spread = texture();
    textured = spread >= CONTRAST;
  }
  workgroupBarrier();
  if !textured {
    // No GEOMETRY to read here, and that is all the correlation was ever
    // going to answer. The colour is still readable, and this is the content
    // most of a real seam is made of: a fifth to two thirds of the ring on the
    // nine captures measured (issue #103, stage 7). What a displaced window
    // costs a photometry is the content's own gradient across it, so the patch
    // the correlation refuses is the one whose colour is cheapest to trust,
    // and the shift it is read at is zero because zero is what the calibration
    // says and nothing here can improve on it.
    //
    // Only the patch, not the search grid: this is PATCH taps against the 2301
    // the textured path fills, so a flat direction stays the cheap one.
    let width = u32(2 * HALF + 1);
    for (var i = lane; i < PATCH; i += THREADS) {
      let row = i / width;
      let column = i % width;
      let a = f32(i32(column) - HALF) * STEP;
      let b = f32(i32(row) - HALF) * STEP;
      back[(row + u32(-EPI_FAR)) * BACK_ALONG + u32(PERP_STEPS * PERP_STEP) + column] =
        tap(1u, aim1, at.centre + a * at.perp + b * at.epi);
    }
    workgroupBarrier();
    photometry(lane, LEVEL, at, aim0, aim1);
    workgroupBarrier();
    if lane == 0u {
      forget(cell, at);
    }
    return;
  }

  // The back lens's grid: the patch widened by everywhere the search may
  // slide it.
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

  // The two lenses' brightness and colour on the SAME content, which is what
  // the shift above just established and what no earlier exposure measurement
  // in this project had. Cooperative, so it costs a seventh of a sample per
  // lane.
  photometry(lane, winner, at, aim0, aim1);
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

// Each lane's share of the two patches' brightness and colour at the winning
// shift, clipped samples left out in pairs.
fn photometry(lane: u32, found: u32, at: Ring, aim0: mat3x3<f32>, aim1: mat3x3<f32>) {
  let epi = found / PERP_SHIFTS;
  let perp = (found % PERP_SHIFTS) * u32(PERP_STEP);
  let width = u32(2 * HALF + 1);
  var sum0 = 0.0;
  var sum1 = 0.0;
  var count = 0.0;
  var chroma0 = vec2<f32>(0.0);
  var chroma1 = vec2<f32>(0.0);
  var colours = 0.0;
  for (var i = lane; i < PATCH; i += THREADS) {
    let row = i / width;
    let column = i % width;
    let a = front[i];
    let b = back[(row + epi) * BACK_ALONG + perp + column];
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
    // The colour, on every HUE_STEP-th sample of each axis, and on the same
    // pairs the luma is read on so the two halves of one reading describe the
    // same population. The luma grids are already in workgroup memory and the
    // chroma has to be fetched, which is the whole reason for the stride.
    if row % HUE_STEP != 0u || column % HUE_STEP != 0u {
      continue;
    }
    let front_along = f32(i32(column) - HALF) * STEP;
    let front_across = f32(i32(row) - HALF) * STEP;
    let back_along = f32(i32(perp + column) - HALF - PERP_STEPS * PERP_STEP) * STEP;
    let back_across = f32(i32(row + epi) - HALF + EPI_FAR) * STEP;
    chroma0 += hue_tap(0u, aim0, at.centre + front_along * at.perp + front_across * at.epi);
    chroma1 += hue_tap(1u, aim1, at.centre + back_along * at.perp + back_across * at.epi);
    colours += 1.0;
  }
  lit0[lane] = sum0;
  lit1[lane] = sum1;
  lit_n[lane] = count;
  hue0[lane] = chroma0;
  hue1[lane] = chroma1;
  hue_n[lane] = colours;
}

// Where in the score table the shift of zero sits, which is the shift a patch
// with nothing in it to correlate is read at (issue #103, stage 7).
const LEVEL: u32 = u32(-EPI_FAR) * PERP_SHIFTS + u32(PERP_STEPS);

// Zero-mean normalized cross-correlation of the two patches at candidate
// shift `i`, or -2 where either patch is short of picture. -2 rather than -1
// so that "no answer" loses to any answer, including a perfectly
// anti-correlated one.
fn correlate(i: u32) -> f32 {
  let epi = i / PERP_SHIFTS;
  let perp = (i % PERP_SHIFTS) * u32(PERP_STEP);
  var sum_a = 0.0;
  var sum_b = 0.0;
  var sum_aa = 0.0;
  var sum_bb = 0.0;
  var sum_ab = 0.0;
  var count = 0.0;
  for (var row = 0u; row < u32(2 * HALF + 1); row += 1u) {
    let source = (row + epi) * BACK_ALONG + perp;
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
  return (sum_ab - sum_a * sum_b / count) / sqrt(var_a * var_b);
}

// Whether the front patch has enough picture in it to correlate: the gate that
// keeps flat sky out, which correlates with anything and what it correlates
// with is noise.
//
// One test for the whole direction rather than one per candidate. It was
// written inside `correlate` and reached only after that candidate's whole
// double loop had run, so a direction of blank sky paid for the entire table
// to be told there was nothing in it - which on a real seam is most of the
// ring (issue #103, stage 6).
// How much picture the front patch has in it, as a standard deviation in codes
// of 1, or -1 where part of the patch is outside this lens's picture.
//
// TWO readers since stage 8, and they ask different questions of the same
// number. The correlation asks whether there is enough here to match on, which
// is `>= CONTRAST` and is phase A's gate unchanged. The width asks how much a
// wider handover would cost here, and the answer to that is the number itself:
// what a misregistration draws twice is the content's own gradient, and a patch
// with none has none to draw (`Cell::open`).
fn texture() -> f32 {
  var sum = 0.0;
  var square = 0.0;
  var count = 0.0;
  for (var i = 0u; i < PATCH; i += 1u) {
    let a = front[i];
    if a < 0.0 {
      // No picture of part of the patch. The correlation refuses that anyway,
      // one candidate at a time, and this refuses it once.
      return -1.0;
    }
    sum += a;
    square += a * a;
    count += 1.0;
  }
  let spread = square - sum * sum / count;
  return sqrt(max(spread, 0.0) / count);
}

// How far this direction may open its handover past the floor, from what a
// wider one would cost it. Rust twin: `openness`.
fn openness(disparity: f32, strength: f32, spread: f32) -> f32 {
  let left = STEP + abs(disparity) * (1.0 - clamp(strength, 0.0, 1.0));
  let ghost = left / SPAN * spread;
  let t = clamp(ghost / ONE_CODE, 0.0, 1.0);
  return 1.0 - t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

// One step of this direction's own additive term: what the two lenses still
// differ by here after the pooled gain, eased into what the direction was
// holding (issue #103, stage 8). Rust twin: `Cell::offset`'s own docstring.
//
// AT TAU_GAIN, which is what this file smooths things that do not move by. It
// needs one, and stage 7 measured why: one frame's reading of one direction
// carries about a code of its own noise, and a colour that breathes is motion
// where the scene has none.
//
// ALWAYS eased, with no first-reading-whole path, which is where this channel
// differs from every geometry channel in this file and it is `pool`'s own rule
// for `pool`'s own reason: a bend that arrives late leaves a doubled edge for a
// second and a photometry that arrives instantly is a picture changing
// brightness in one frame. A reset empties the cell, so a seek walks the
// correction in from nothing rather than carrying somewhere else's.
//
// Measured before it was chosen: taking the first reading whole made this
// column pump at 0.044 to 0.057 ln rms per frame at the azimuths where the
// correlation comes and goes, which is seven times a code, because a reading
// that passes through zero is not the same thing as a direction that has never
// been read.
//
// WHAT DECAYS WHERE NOTHING CONFIRMS IT IS THE EVIDENCE AND NOT THE VALUE,
// which is this file's rule on every other channel: `hue_conf` is what weighs
// this in `band_bend`, so a direction that stops being read fades out of the
// picture on its own and does not have to be unlearned. The gain it is measured
// against is last frame's, which is smoothed over two seconds and cannot move
// inside one.
fn read_offset(held: ptr<function, Cell>) {
  if watch.hold != 0.0 {
    (*held).offset = array<f32, 3>(0.0, 0.0, 0.0);
    return;
  }
  if (*held).hue_conf <= 0.0 {
    return;
  }
  let low = decoded((*held).lit, (*held).chroma[0], (*held).chroma[1]);
  let high = decoded((*held).lit * exp((*held).tone), (*held).chroma[2], (*held).chroma[3]);
  let want = clamp(
    high - exp(band.tone.log_gain) * low,
    vec3<f32>(-LIMIT_OFF),
    vec3<f32>(LIMIT_OFF),
  );
  let learn = ease(watch.seconds, TAU_GAIN);
  for (var channel = 0u; channel < 3u; channel += 1u) {
    (*held).offset[channel] += (want[channel] - (*held).offset[channel]) * learn;
  }
}

// One step of the openness: what this frame's patch says, eased into what the
// direction was holding, AT THE CONSTANT THE DISPARITY DRIVING IT IS SMOOTHED
// AT.
//
// `time_constant` and not a rule of its own, which is stage 4's property kept:
// a width cannot flicker faster than the reading it is made of. It gives the
// safety argument for free rather than by an asymmetry - a near-field direction
// answers in a tenth of a second, which is where a wing crossing the seam is,
// and a far-field one takes two seconds, which is where nothing moves - and a
// near-field direction is the one that needs it least, because the fold opens
// its band whatever this says.
//
// An asymmetric rule was tried first and measured worse: shutting at TAU_NEAR
// everywhere moved the width by 5 percent of its range between frames on
// footage panning at 197 deg/s, which is four times the bend's own flicker.
fn read_open(held: ptr<function, Cell>, spread: f32) {
  if spread < 0.0 {
    return;
  }
  let strength = clamp((*held).confidence / KEEP, 0.0, 1.0);
  let want = openness((*held).disparity, strength, spread);
  let learn = select(
    ease(watch.seconds, time_constant((*held).disparity)),
    1.0,
    watch.reset != 0.0,
  );
  (*held).open += (want - (*held).open) * learn;
}

// The state a direction starts from: no bend on either axis, no colour, and
// this direction's own reach. Rust twin: `Cell::default` with `reach_m` set.
fn empty(at: Ring) -> Cell {
  return Cell(
    0.0, 0.0, at.reach_m, 0.0, 0.0, 0.0, 0.0,
    array<f32, 4>(0.0, 0.0, 0.0, 0.0), 0.0, 0.0, array<f32, 3>(0.0, 0.0, 0.0),
  );
}

// A direction with nothing in the picture to CORRELATE. Both geometry channels
// give up their evidence, which is the rule everywhere else in this file: the
// reading was true when it was taken and may be true still, but nothing is
// confirming it.
//
// The colour is a different question and it has an answer here (issue #103,
// stage 7). A flat patch is the easiest photometry on the ring and the hardest
// alignment, so this is where the two parts of one reading stop agreeing about
// whether the direction was worth looking at.
fn forget(cell: u32, at: Ring) {
  var held = band.cells[cell];
  if watch.reset != 0.0 {
    held = empty(at);
  }
  held.reach_m = at.reach_m;
  held.confidence -= held.confidence * ease(watch.seconds, time_constant(held.disparity));
  held.off_conf -= held.off_conf * ease(watch.seconds, TAU_FAR);
  read_colour(&held, 1.0);
  read_offset(&held);
  read_open(&held, spread);
  band.cells[cell] = held;
}

// One step of the colour reading: the photometry the workgroup just summed, and
// the confidence it is believed at.
//
// `reading` is 1 on a flat patch, where no correlation was needed and none was
// made, and the correlation's own peak on a textured one. What decays where
// clipping left no pair to read is the confidence and not the value, which is
// this file's rule for every channel.
//
// A direction with NO colour evidence takes its reading whole, for the reason
// the two geometry channels do: there is no picture behind it to move under, so
// there is nothing for an ease to hide.
fn read_colour(held: ptr<function, Cell>, reading: f32) {
  let unread = (*held).hue_conf <= 0.0;
  let learn = select(ease(watch.seconds, TAU_FAR), 1.0, watch.reset != 0.0 || unread);
  if read_photometry(held) {
    (*held).hue_conf += (reading - (*held).hue_conf) * learn;
  } else {
    (*held).hue_conf -= (*held).hue_conf * ease(watch.seconds, TAU_FAR);
  }
}

// The peak, the gates, and one step of the filter. One thread, because it is
// a few dozen operations over a table the whole workgroup has already filled.
fn settle(cell: u32, at: Ring) {
  var held = band.cells[cell];
  if watch.reset != 0.0 {
    held = empty(at);
  }
  held.reach_m = at.reach_m;

  let found = winner;
  let best = scores[found];
  let epi = i32(found / PERP_SHIFTS);
  let perp = i32(found % PERP_SHIFTS) - PERP_STEPS;
  // A peak against the edge of the search is not a peak, it is the search
  // running out, and a reading pinned at the limit would report the limit.
  // Each axis runs out for its own reason and is refused on its own, which is
  // stage 5's one piece of new bookkeeping. The epipolar axis runs out on
  // near-field content that moves further across than the band is wide. The
  // along-seam axis runs out on a camera whose calibration residual is outside
  // anything measured, and refusing the epipolar channel for THAT would throw
  // stage 2 away on that footage for a reason that has nothing to do with it.
  let quiet = best < KEEP;
  let epi_pinned = quiet || epi == 0 || epi == i32(EPI_SHIFTS) - 1;
  // The along-seam channel also stands down whenever the epipolar one does:
  // a match at the edge of the depth window has not established what content
  // is being compared, and an along-seam offset read off content that is not
  // the same content is not a reading of anything.
  let along_pinned = epi_pinned || perp == -PERP_STEPS || perp == PERP_STEPS;

  // What decays where a channel is refused is the EVIDENCE and not the
  // measurement: the reading was true when it was taken and may be true still,
  // but nothing is confirming it, and the pass applies a reading in proportion
  // to how well it is being confirmed (`band_bend`). So the bend fades out on
  // its own, and a direction that starts correlating again has its answer
  // already in hand rather than having to learn it twice.
  //
  // The epipolar channel fades at the SAME rate the direction learns, which is
  // the whole of the occlusion story: a near reading is a reading of something
  // between the camera and the background - a selfie stick, a hand, a boot,
  // someone walking past - and the reason it correlates fast is the reason it
  // expires fast. A far reading is the background, which has not gone
  // anywhere. One knee decides both.
  //
  // The along-seam channel fades at TAU_FAR wherever it is, because what it
  // holds is the camera and the camera has not gone anywhere either. Rust
  // twin: `Cell::off_epi`'s own docstring, and the `leak` line that measured
  // it before it was chosen.
  //
  // A direction with NO EVIDENCE takes its reading whole for the same reason a
  // reset frame does, and it is the same sentence: there is no picture behind
  // it to move under, so there is nothing for an ease to hide, and easing
  // anyway leaves it drawn with a correction of nearly nothing for two
  // seconds. Stage 2 wrote that argument for the reset frame and applied it
  // only there, so a direction that first correlated on any LATER frame - most
  // of the ring, on real footage, where a seam is mostly sky until something
  // crosses it - crept in from zero at TAU_FAR instead (issue #103, stage 6).
  // Each channel asks its own, because they are refused separately.
  let unread = held.confidence <= 0.0;
  let unread_along = held.off_conf <= 0.0;
  let fresh = watch.reset != 0.0;
  let learn = select(ease(watch.seconds, time_constant(held.disparity)), 1.0, fresh || unread);
  let learn_along = select(ease(watch.seconds, TAU_FAR), 1.0, fresh || unread_along);
  if epi_pinned {
    held.confidence -= held.confidence * ease(watch.seconds, time_constant(held.disparity));
  } else {
    // Between whole steps, because a third of a step is exactly the size this
    // is trying to resolve. Rust twin: `super::seam::best_shift`'s `peak`.
    let read = f32(epi + EPI_FAR) + parabola(scores[found - PERP_SHIFTS], best, scores[found + PERP_SHIFTS]);
    // The time constant is read off what this direction has been showing, not
    // off what it showed this frame: a noisy far-field reading must not unlock
    // the smoothing that is keeping the horizon still. Rust twin:
    // `time_constant`.
    //
    // The first frame of a file, and the first after a seek, take the reading
    // whole instead. There is no picture behind them to move under, so there
    // is nothing for an ease to hide, and easing anyway would leave the first
    // two seconds of film drawn with a correction of nearly nothing. The same
    // argument, and the same answer, as `seam::Correction::land`.
    held.disparity += (read * STEP - held.disparity) * learn;
    held.confidence += (best - held.confidence) * learn;
  }
  if along_pinned {
    held.off_conf -= held.off_conf * ease(watch.seconds, TAU_FAR);
  } else {
    // The same parabola on the same grid: the along-seam neighbours are one
    // score apart because the table runs perp fastest, where the epipolar ones
    // are PERP_SHIFTS apart. Without it this channel quantizes to PERP_STEP,
    // which is 15 view px of horizon at the view stage 5 exists for.
    let read = f32(perp) + parabola(scores[found - 1u], best, scores[found + 1u]);
    held.off_epi += (read * f32(PERP_STEP) * STEP - held.off_epi) * learn_along;
    held.off_conf += (best - held.off_conf) * learn_along;
  }
  // The photometry is read at the shift that made the two patches the same
  // content, so a shift that did not establish that is worth less - but it is
  // not worth nothing, and stage 7 threw it away (issue #103, stage 8). What a
  // displaced window costs a photometry is the content's own gradient across
  // it, which is exactly the number `openness` already prices, so a direction
  // whose correlation was refused reads at the calibration's own shift and is
  // believed at the price of being wrong there. On this footage that is 50 of
  // 128 directions, and it was a continuous ARC of them: the ring's photometry
  // had a hole in it the size of the owner's own complaint.
  //
  // It degrades into the two neighbours rather than stepping between them: at
  // the contrast gate the flat path already reads whole, and past it this
  // falls away as the content's own gradient rises, reaching zero on content
  // that would cost more than a code to be wrong about.
  let priced = openness(held.disparity, clamp(held.confidence / KEEP, 0.0, 1.0), spread);
  read_colour(&held, select(best, priced, epi_pinned));
  read_offset(&held);
  read_open(&held, spread);
  band.cells[cell] = held;
}

// Where between three scores the peak really is, in steps, from the parabola
// through them. Zero where they do not curve down, which is not a peak.
// Rust twin: `super::seam::best_shift`'s `peak`.
fn parabola(minus: f32, best: f32, plus: f32) -> f32 {
  let curve = minus - 2.0 * best + plus;
  if curve >= 0.0 {
    return 0.0;
  }
  return clamp(0.5 * (minus - plus) / curve, -1.0, 1.0);
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
fn read_photometry(held: ptr<function, Cell>) -> bool {
  var sum0 = 0.0;
  var sum1 = 0.0;
  var count = 0.0;
  var chroma0 = vec2<f32>(0.0);
  var chroma1 = vec2<f32>(0.0);
  var colours = 0.0;
  for (var i = 0u; i < THREADS; i += 1u) {
    sum0 += lit0[i];
    sum1 += lit1[i];
    count += lit_n[i];
    chroma0 += hue0[i];
    chroma1 += hue1[i];
    colours += hue_n[i];
  }
  // Clipping left nothing to read. The direction keeps what it had, which is
  // the same rule as a refusal: what is absent is a confirmation, not a
  // reason to believe the opposite.
  if count <= 0.0 || sum0 <= 0.0 || sum1 <= 0.0 || colours <= 0.0 {
    return false;
  }
  (*held).tone = log(sum1 / sum0);
  (*held).lit = sum0 / count;
  let mean0 = chroma0 / colours;
  let mean1 = chroma1 / colours;
  (*held).chroma = array<f32, 4>(mean0.x, mean0.y, mean1.x, mean1.y);
  return true;
}

// Whether the disparity this direction is DRAWN with is inside the near knee:
// the reading times the strength its evidence has earned, so an expired
// near-field reading is not one. Rust twin: `near_field`.
fn near_field(cell: Cell) -> bool {
  return abs(cell.disparity) * clamp(cell.confidence / KEEP, 0.0, 1.0) >= NEAR_KNEE;
}

// The pooled exposure, over the whole ring and over media time. Rust twin:
// `pooled_gain`.
//
// One workgroup, dispatched straight after the measurement and in the same
// pass, so what it pools is what was just written. A direction contributes at
// the weight the bend already trusts it at (`band_bend`'s `strength`), so a
// direction whose evidence has faded fades out of the exposure too and one
// that never correlated was never in it.
@compute @workgroup_size(THREADS)
fn pool(@builtin(local_invocation_index) lane: u32) {
  // The ring's own level per channel first, because the two parameters below
  // are not comparable until one of them is divided by a brightness. Rust
  // twin: `pooled_tone`'s first pass.
  var level = vec3<f32>(0.0);
  var count = 0.0;
  for (var i = lane; i < AZIMUTHS; i += THREADS) {
    let cell = band.cells[i];
    // Far field only, at the band's own knee and not at a second one. A
    // direction reading past NEAR_KNEE is looking at something inside 10 m,
    // which is a boot, a hand or a wing: the hardest content to line up, the
    // darkest content on a flight, and where the two lenses' difference stops
    // being their exposure. Measured across nine captures, cutting at this
    // knee takes the step a fitted gain leaves from 15.7 codes to 3.6 on the
    // worst of them (`kjerag-spike --bin expose`, the near and far rows of
    // the `models` table).
    if near_field(cell) {
      continue;
    }
    // Least squares in codes, per channel: each direction weighs its own
    // brightness squared IN THAT CHANNEL, which is what makes this the ratio
    // the two lenses actually differ by and not an average over whichever
    // patches happened to be dark. Its own confidence and not the bend's,
    // because a direction of flat sky has a colour and no correlation
    // (`Cell::hue_conf`).
    let believed = clamp(cell.hue_conf / KEEP, 0.0, 1.0);
    if believed <= 0.0 {
      continue;
    }
    let low = decoded(cell.lit, cell.chroma[0], cell.chroma[1]);
    let high = decoded(cell.lit * exp(cell.tone), cell.chroma[2], cell.chroma[3]);
    level += believed * 0.5 * (low + high);
    count += believed;
  }
  pooled_total[lane] = level;
  pooled_count[lane] = count;
  workgroupBarrier();
  var sum_level = vec3<f32>(0.0);
  var sum_count = 0.0;
  for (var i = 0u; i < THREADS; i += 1u) {
    sum_level += pooled_total[i];
    sum_count += pooled_count[i];
  }
  if lane != 0u {
    return;
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
    held = Tone(vec3<f32>(0.0), 0.0);
  }
  if sum_count <= 0.0 || any(sum_level <= vec3<f32>(0.0)) {
    // Nothing on the ring is confirming anything this frame. The exposure of
    // two lenses does not change because we stopped being able to see it, so
    // the value is kept and only the evidence behind it is given up.
    held.evidence -= held.evidence * ease(watch.seconds, TAU_GAIN);
    band.tone = held;
    return;
  }
  let ring_level = sum_level / sum_count;
  // The two-by-two normal system for `high = gain * low + offset`, per
  // channel, in RATIO space: the residual is divided by the level it sits on
  // before it is squared, which is a weight of one over that level squared.
  // The ridge is already on the diagonal and already shrinking the gain
  // towards exactly 1 and the offset towards exactly 0. Rust twin:
  // `pooled_tone`.
  var gain = vec3<f32>(1.0);
  for (var channel = 0u; channel < 3u; channel += 1u) {
    var aa = RIDGE;
    var ab = 0.0;
    var bb = RIDGE;
    var ya = RIDGE;
    var yb = 0.0;
    for (var i = 0u; i < AZIMUTHS; i += 1u) {
      let cell = band.cells[i];
      if near_field(cell) {
        continue;
      }
      let believed = clamp(cell.hue_conf / KEEP, 0.0, 1.0);
      if believed <= 0.0 {
        continue;
      }
      let low = decoded(cell.lit, cell.chroma[0], cell.chroma[1])[channel];
      let high = decoded(cell.lit * exp(cell.tone), cell.chroma[2], cell.chroma[3])[channel];
      let mid = 0.5 * (low + high);
      if mid <= 0.0 || low <= 0.0 || high <= 0.0 {
        continue;
      }
      let weight = believed * (ring_level[channel] / mid) * (ring_level[channel] / mid);
      let u = low / ring_level[channel];
      let v = high / ring_level[channel];
      aa += weight * u * u;
      ab += weight * u;
      bb += weight;
      ya += weight * u * v;
      yb += weight * v;
    }
    let det = aa * bb - ab * ab;
    if det <= 0.0 {
      continue;
    }
    let read = (ya * bb - yb * ab) / det;
    if read <= 0.0 {
      continue;
    }
    gain[channel] = read;
  }
  // Only the GAIN is kept. The offset is a nuisance parameter here and it is
  // fitted for one reason: a gain fitted alone in ratio space would absorb the
  // additive part and come back wrong on both ends of the ring. What the
  // picture is drawn with is the per-direction offset, at the direction it was
  // read at (`Cell::offset`).
  let read_gain = clamp(log(gain), vec3<f32>(-LIMIT_LN), vec3<f32>(LIMIT_LN));
  let step = ease(watch.seconds, TAU_GAIN);
  held.log_gain += (read_gain - held.log_gain) * step;
  held.evidence += (sum_count / f32(AZIMUTHS) - held.evidence) * step;
  band.tone = held;
}

// The along-seam field, fitted over the whole ring (issue #103, stage 5).
// Rust twin: `Along::fit`.
//
// One lane, and unapologetically: this is one workgroup once per frame, over
// 128 cells, against a correlation that reads two grids per direction over
// half of them. What it costs is not where this pass spends anything.
//
// NO TIME CONSTANT OF ITS OWN, for the reason `width` has none: what it is
// fitted to is already the smoothed, evidence-weighted per-direction state,
// so the field inherits every direction's own constant exactly. A reset
// empties the cells and the fit then answers zero by arithmetic, which is the
// picture stage 4 drew.
@compute @workgroup_size(1)
fn pool_along() {
  var normal = array<f32, 25>();
  var right = array<f32, 5>();
  var evidence = 0.0;
  for (var index = 0u; index < AZIMUTHS; index += 1u) {
    let cell = band.cells[index];
    let trust = clamp(cell.off_conf / KEEP, 0.0, 1.0);
    if trust <= 0.0 {
      continue;
    }
    let phi = f32(index) / f32(AZIMUTHS) * TAU;
    let cosine = cos(phi);
    let sine = sin(phi);
    let basis = array<f32, 5>(1.0, cosine, sine, cosine * cosine - sine * sine, 2.0 * cosine * sine);
    for (var row = 0u; row < 5u; row += 1u) {
      for (var column = 0u; column < 5u; column += 1u) {
        normal[row * 5u + column] += trust * basis[row] * basis[column];
      }
      right[row] += trust * basis[row] * cell.off_epi;
    }
    evidence += trust;
  }
  // The ridge, which is what makes a thin ring safe and a bare one exactly
  // zero. Rust twin: `RIDGE`.
  for (var term = 0u; term < 5u; term += 1u) {
    normal[term * 5u + term] += RIDGE;
  }
  var out: Along;
  out.terms = solve5(&normal, &right);
  out.evidence = evidence;
  band.along = out;
}

// The additive term round the ring, fitted over the same cells the geometry is
// fitted over (issue #103, stage 8). Rust twin: `Glare::fit`.
//
// One lane, for `pool_along`'s reason. NO TIME CONSTANT OF ITS OWN, for
// `pool_along`'s reason too: what it is fitted to is already the smoothed,
// evidence-weighted per-direction state, so the field inherits every
// direction's own constant exactly. A reset empties the cells and the fit then
// answers zero by arithmetic, which is the picture before this existed.
//
// This is what the picture is drawn with, and `Cell::offset` is only what it is
// fitted from. Five terms cannot carry a stripe.
@compute @workgroup_size(1)
fn pool_glare() {
  var terms = array<f32, 15>();
  var evidence = 0.0;
  if watch.hold != 0.0 {
    band.glare = Glare(terms, 0.0);
    return;
  }
  for (var channel = 0u; channel < 3u; channel += 1u) {
    var normal = array<f32, 25>();
    var right = array<f32, 5>();
    evidence = 0.0;
    for (var index = 0u; index < AZIMUTHS; index += 1u) {
      let cell = band.cells[index];
      let trust = clamp(cell.hue_conf / KEEP, 0.0, 1.0);
      if trust <= 0.0 {
        continue;
      }
      let phi = f32(index) / f32(AZIMUTHS) * TAU;
      let cosine = cos(phi);
      let sine = sin(phi);
      let basis = array<f32, 5>(1.0, cosine, sine, cosine * cosine - sine * sine, 2.0 * cosine * sine);
      for (var row = 0u; row < 5u; row += 1u) {
        for (var column = 0u; column < 5u; column += 1u) {
          normal[row * 5u + column] += trust * basis[row] * basis[column];
        }
        right[row] += trust * basis[row] * cell.offset[channel];
      }
      evidence += trust;
    }
    for (var term = 0u; term < 5u; term += 1u) {
      normal[term * 5u + term] += RIDGE;
    }
    let fitted = solve5(&normal, &right);
    for (var term = 0u; term < 5u; term += 1u) {
      terms[5u * channel + term] = fitted[term];
    }
  }
  band.glare = Glare(terms, evidence);
}

// Gaussian elimination with no pivoting on a 5x5. Safe without pivoting
// because the matrix is a Gram matrix plus RIDGE on its diagonal, so it is
// positive definite whatever the ring holds. Rust twin: `solve`.
fn solve5(normal: ptr<function, array<f32, 25>>, right: ptr<function, array<f32, 5>>) -> array<f32, 5> {
  for (var pivot = 0u; pivot < 5u; pivot += 1u) {
    let scale = (*normal)[pivot * 5u + pivot];
    for (var row = pivot + 1u; row < 5u; row += 1u) {
      let factor = (*normal)[row * 5u + pivot] / scale;
      for (var column = pivot; column < 5u; column += 1u) {
        (*normal)[row * 5u + column] -= factor * (*normal)[pivot * 5u + column];
      }
      (*right)[row] -= factor * (*right)[pivot];
    }
  }
  var out = array<f32, 5>();
  for (var step = 0u; step < 5u; step += 1u) {
    let row = 4u - step;
    var total = (*right)[row];
    for (var column = row + 1u; column < 5u; column += 1u) {
      total -= (*normal)[row * 5u + column] * out[column];
    }
    out[row] = total / (*normal)[row * 5u + row];
  }
  return out;
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
    fn a_reset_frame_reads_every_direction_and_a_tracking_frame_reads_its_slice() {
        // What `reset` has to mean, said in the arithmetic the dispatch uses
        // (issue #103, stage 6). `reset` throws away state that is held PER
        // DIRECTION, so a reset frame that visits half the ring leaves the
        // other half holding whatever it held before the seek and decaying
        // towards the new content over TAU_FAR. Measured on the owner's July
        // file before this was fixed: half the circle read ONE THIRD of what
        // the other half read, on the same frames, of the same content.
        let cover = |watch: Watch| {
            let mut seen = vec![0usize; AZIMUTHS];
            for group in 0..watch.groups() {
                seen[(group * watch.stride as u32 + watch.slice as u32) as usize] += 1;
            }
            seen
        };
        assert!(
            cover(Watch::start(0.03)).iter().all(|times| *times == 1),
            "a reset frame does not read every direction exactly once",
        );
        let mut over_the_rounds = vec![0usize; AZIMUTHS];
        for slice in 0..SLICES {
            for (index, times) in cover(Watch::track(0.03, slice)).iter().enumerate() {
                assert!(*times <= 1, "a tracking frame reads a direction twice");
                over_the_rounds[index] += times;
            }
        }
        assert!(
            over_the_rounds.iter().all(|times| *times == 1),
            "the {SLICES} rounds of the circle do not cover it exactly once",
        );
    }

    #[test]
    fn the_two_instruments_name_the_same_two_axes() {
        // Two instruments measure the seam's two axes - this pass, through
        // `Ring`, and `--bin seam mode=residual`, through `seam::ring` - and
        // until stage 6 they named the along-seam one with opposite signs.
        // Neither drew a wrong picture for it: each measures and applies
        // through its own axis, so the two signs cancel inside each. What it
        // cost was the ability to read one beside the other, which is how
        // stage 5's cap was misdiagnosed as a disagreement about content
        // (docs/research/seam-two-axis.md).
        //
        // The two are not the same axis to the last decimal and cannot be:
        // `epi` is the baseline's own line and the baseline is tilted off the
        // body's z. What they must be is the same axis to within that tilt,
        // and pointing the same way.
        let ring = crate::seam::ring(AZIMUTHS);
        let mut worst_along = 1.0f64;
        let mut worst_across = 1.0f64;
        for (index, at) in ring.iter().enumerate() {
            let cell = Ring::cell(index, BASELINE);
            let along = dot(cell.perp, at.along.map(|c| c as f32));
            let across = dot(cell.epi, at.across.map(|c| c as f32));
            worst_along = worst_along.min(f64::from(along));
            worst_across = worst_across.min(f64::from(across));
        }
        // The baseline's tilt is 3.6 degrees at its worst on the fixture, and
        // `cos(3.6 deg)` is 0.998.
        assert!(
            worst_along > 0.99,
            "the two along-seam axes agree only to {worst_along:.4} at worst",
        );
        assert!(
            worst_across > 0.99,
            "the two across-seam axes agree only to {worst_across:.4} at worst",
        );
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
                off_conf: 0.0,
                tone: 0.0,
                lit: 0.0,
                chroma: [0.0; 4],
                hue_conf: 0.0,
                open: 0.0,
                offset: [0.0; 3],
            };
            AZIMUTHS
        ];
        let field = Along::fit(&dead);
        assert_eq!(reframe.reading_at(ray, &dead, field), Reading::default());
        assert_eq!(
            reframe.bend(ray, reframe.reading_at(ray, &dead, field)),
            crate::projection::Bend::default(),
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
        let held = reframe.reading_at(ray, &live, Along::fit(&live)).epi;
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

    /// How wide the two lenses of the calibration fixture overlap, halved:
    /// what the shipped pass hands [`width`] as the ceiling. Read off the
    /// fixture in `the_widest_band_and_its_bend_stay_inside_the_overlap` and
    /// written here as a number for the arithmetic tests, which have no file.
    const CEILING_DEG: f32 = 7.22;

    /// Stage 4's own question of [`width`]: a direction with nothing to open
    /// for, at a view with no pixels to ask about. Every property stage 4
    /// measured is a property of THIS call, and it is why they still hold.
    fn shut(disparity_rad: f32, floor_rad: f32) -> f32 {
        width(disparity_rad, 0.0, floor_rad, CEILING_DEG.to_radians())
    }

    #[test]
    fn the_bend_never_folds_the_crossover() {
        // Shear is the disparity over the band width and above 1 the mapping
        // prints the picture back over itself. What the pair has to promise
        // is that the Jacobian stays positive at any disparity the search can
        // report, the near limit included - now that the band opens as well
        // as the clamp closing, both halves are in the promise.
        let floor = FLOOR_DEG.to_radians();
        for degrees in [-10.0f32, -1.2, 0.0, 0.19, 1.9, 3.5, 100.0] {
            let band = shut(degrees.to_radians(), floor);
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
            let carried = carried(radians, shut(radians, floor));
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
        for degrees in [-0.84f32, -0.19, 0.0, 0.19, 0.64, 0.95] {
            let opened = shut(degrees.to_radians(), floor);
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
        assert_eq!(shut(0.0, floor).to_bits(), floor.to_bits());
    }

    #[test]
    fn the_band_opens_no_further_than_the_reading_needs() {
        let floor = FLOOR_DEG.to_radians();
        // Monotone, so a direction drifting nearer does not step, and never
        // past the widest reading the search can return.
        let mut last = 0.0f32;
        for step in 0..400 {
            let opened = shut(step as f32 * 0.01f32.to_radians(), floor);
            assert!(opened >= last - 1e-9, "step {step}: {opened} after {last}");
            assert!(opened <= WIDEST_DEG.to_radians() + 1e-9);
            last = opened;
        }
        // In between it is exactly what the inequality asks for and not a
        // rounded-up version of it: the shear at the profile's steepest point
        // is exactly FOLD.
        let near = 2.4f32.to_radians();
        assert!((shut(near, floor) - near * SLOPE / FOLD).abs() < 1e-9);
    }

    #[test]
    fn the_width_cannot_flicker_faster_than_the_reading_it_comes_from() {
        // The whole of stage 4's temporal design, and the reason it adds no
        // filter and no constant. The width is 1/FOLD-Lipschitz in the
        // disparity, so the per-direction time constants stage 2 measured
        // bound the width's own steadiness as well: the width is
        // SLOPE/FOLD-Lipschitz in the disparity, so 0.02 deg rms of disparity
        // flicker cannot become more than 0.042 deg rms of width flicker,
        // whatever the content is.
        let floor = FLOOR_DEG.to_radians();
        let mut worst = 0.0f64;
        for a in -300..300 {
            for b in -300..300 {
                let (one, two) = (a as f32 * 0.01, b as f32 * 0.01);
                let moved = f64::from(
                    (shut(one.to_radians(), floor) - shut(two.to_radians(), floor)).abs(),
                );
                let read = f64::from((one - two).abs().to_radians());
                // Slack for the f32 rounding in the two differences
                // themselves, which is what is being compared and not what is
                // being claimed: at a hundredth of a degree apart the two sides
                // are 1.9e-4 and the last bits of each are noise.
                assert!(
                    moved <= read * f64::from(SLOPE / FOLD) * (1.0 + 1e-4) + 1e-12,
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
            (worst - f64::from(SLOPE / FOLD)).abs() < 1e-3,
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
            reframe.crossover_at(0.0, 0.0).to_bits(),
            FLOOR_DEG.to_radians().to_bits(),
        );
    }

    /// One direction of the ring, made rather than measured, so the pooling
    /// can be asked questions with no GPU and no footage.
    fn lit_cell(disparity_deg: f32, brightness: f32, gain: f32) -> Cell {
        Cell {
            disparity: disparity_deg.to_radians(),
            confidence: KEEP,
            reach_m: 0.033,
            off_epi: 0.0,
            off_conf: 0.0,
            tone: gain.ln(),
            lit: brightness,
            // Neutral both sides, so the three channels are the luma and a
            // reading that is one number is one number in all of them.
            chroma: [0.0; 4],
            hue_conf: KEEP,
            open: 0.0,
            offset: [0.0; 3],
        }
    }

    /// The same direction with a colour in it: lens 1's picture differs from
    /// lens 0's by `gain` per channel, arrived at through the two chroma planes
    /// the frame actually carries rather than by writing the answer in.
    fn hue_cell(brightness: f32, gain: [f32; 3]) -> Cell {
        // The decode is a matrix, so what makes lens 1's three channels
        // `gain` times lens 0's is that matrix run backwards: Y from the luma
        // weights, then Cb and Cr from what is left.
        let low = [brightness; 3];
        let high: [f32; 3] = std::array::from_fn(|channel| brightness * gain[channel]);
        let coded = |rgb: [f32; 3]| {
            let luma = LUMA[0] * rgb[0] + LUMA[1] * rgb[1] + LUMA[2] * rgb[2];
            (luma, (rgb[2] - luma) / 1.8556, (rgb[0] - luma) / 1.5748)
        };
        let (luma0, cb0, cr0) = coded(low);
        let (luma1, cb1, cr1) = coded(high);
        Cell {
            tone: (luma1 / luma0).ln(),
            lit: luma0,
            chroma: [cb0, cr0, cb1, cr1],
            ..lit_cell(0.02, brightness, 1.0)
        }
    }

    #[test]
    fn a_ring_that_agrees_reads_back_the_gain_it_was_given() {
        // The positive control the flicker columns of stage 2 taught this
        // file to insist on: a pooling is a negative result until it is shown
        // able to read a positive one.
        for gain in [0.94f32, 0.99, 1.0, 1.02] {
            let ring: Vec<Cell> = (0..AZIMUTHS)
                .map(|index| lit_cell(0.02, 0.2 + 0.003 * index as f32, gain))
                .collect();
            let (read, lift, evidence) = pooled_tone(&ring).expect("a ring that correlated");
            // What the PAIR has to reproduce, which is the thing the picture
            // is drawn with: `gain * low + offset` on every direction of the
            // ring, in the ratio the eye reads it in. The two parameters are
            // checked under that, because a ridge that shrinks a gain towards
            // 1 and an offset towards 0 moves them against each other and
            // leaves what they do together where it was.
            for cell in &ring {
                let [low, high] = cell.decoded();
                for channel in 0..3 {
                    let drawn = read[channel].exp() * low[channel] + lift[channel];
                    assert!(
                        (drawn / high[channel] - 1.0).abs() < 5e-3,
                        "a ring at gain {gain} draws {drawn} where it reads {}",
                        high[channel],
                    );
                }
            }
            for (channel, held) in read.iter().enumerate() {
                assert!(
                    (held - gain.ln()).abs() < 0.01,
                    "a ring at gain {gain} read channel {channel} back at {}",
                    held.exp(),
                );
            }
            assert!((evidence - 1.0).abs() < 1e-5, "evidence {evidence}");
        }
    }

    #[test]
    fn a_ring_that_differs_in_colour_reads_the_colour_back_channel_by_channel() {
        // The whole of stage 7 in one assertion, and the one stage 3's pooling
        // cannot pass: a difference that is not common to R, G and B has to
        // come back as three numbers, in the channels it was put in and not
        // smeared across them. The cell is built by running the fragment
        // shader's own decode backwards, so what is written into the state is
        // the two chroma planes a frame carries and not the answer.
        for gain in [[1.0f32, 1.0, 1.03], [0.97, 1.0, 1.02], [1.0, 1.0, 1.0]] {
            let ring: Vec<Cell> = (0..AZIMUTHS)
                .map(|index| hue_cell(0.2 + 0.003 * index as f32, gain))
                .collect();
            let (read, _, _) = pooled_tone(&ring).expect("a ring that correlated");
            for channel in 0..3 {
                assert!(
                    (read[channel] - gain[channel].ln()).abs() < 0.01,
                    "channel {channel} of {gain:?} read back {}",
                    read[channel].exp(),
                );
            }
        }
    }

    #[test]
    fn a_flat_direction_has_a_colour_and_no_correlation_and_is_pooled_anyway() {
        // The capability stage 7 adds, as the arithmetic that decides it. The
        // band refuses a patch with under CONTRAST codes in it, so a seam of
        // sky carries no `confidence` at all - a fifth to two thirds of the
        // ring on the nine captures measured - and stage 3's pooling therefore
        // read nothing there and applied to the sky a gain measured on the
        // ground. What a flat patch does have is a colour, because what a
        // displaced window costs a photometry is the content's own gradient
        // across it, and that is what `hue_conf` is separate for.
        let sky: Vec<Cell> = (0..AZIMUTHS)
            .map(|index| Cell {
                confidence: 0.0,
                off_conf: 0.0,
                ..hue_cell(0.35 + 0.004 * index as f32, [1.0, 1.0, 1.04])
            })
            .collect();
        let (read, _, evidence) = pooled_tone(&sky).expect("a ring of sky still has a colour");
        assert!((read[2] - 1.04f32.ln()).abs() < 0.01, "read {read:?}");
        assert!((evidence - 1.0).abs() < 1e-5, "evidence {evidence}");
        // And it is `hue_conf` that decides it: a direction nothing has read at
        // all is still nothing.
        let unread: Vec<Cell> = sky
            .iter()
            .map(|cell| Cell {
                hue_conf: 0.0,
                ..*cell
            })
            .collect();
        assert!(pooled_tone(&unread).is_none());
    }

    /// What stage 8 replaced stage 7's ring field with, and why it had to.
    #[test]
    fn what_a_ring_leaves_is_not_a_shape_and_the_offset_is_kept_where_it_was_read() {
        // A ring whose additive term turns once round it, on content whose
        // brightness turns twice. No single gain describes that and no single
        // offset does either, and neither does a five-term fit of either one:
        // measured on the owner's own reference instant, the basis stage 7
        // fitted through leaves 4.2 to 5.5 codes rms round the ring against a
        // frame noise of 0.8 to 1.0 (`--bin colour`, the `rings` table). What
        // does describe it is the reading at the direction it was read at.
        let ring: Vec<Cell> = (0..AZIMUTHS)
            .map(|index| {
                let phi = index as f32 / AZIMUTHS as f32 * std::f32::consts::TAU;
                let brightness = 0.35 + 0.25 * (2.0 * phi).cos();
                let mut cell = hue_cell(brightness, [1.0; 3]);
                let [low, _] = cell.decoded();
                // Put the lift in through the planes a frame actually carries,
                // which is what the pass reads it back out of.
                cell.tone = ((low[1] + 0.02 * phi.cos()) / low[1]).ln();
                cell
            })
            .collect();
        let (gain, _, _) = pooled_tone(&ring).expect("a ring that correlated");
        // The pooled gain finds almost no gain, because there is none to find:
        // what is there is additive and the estimator can tell the difference.
        for held in &gain {
            assert!(held.abs() < 0.03, "the gain took {gain:?} of an offset");
        }
        // And what each direction is left holding is its own lift, which is
        // what `read_offset` writes into the cell and what the picture is
        // drawn with.
        for (index, cell) in ring.iter().enumerate() {
            let phi = index as f32 / AZIMUTHS as f32 * std::f32::consts::TAU;
            let [low, high] = cell.decoded();
            let read = high[1] - gain[1].exp() * low[1];
            let want = 0.02 * phi.cos();
            assert!(
                (read - want).abs() < 0.01,
                "direction {index} reads {read} where it was given {want}",
            );
        }
    }

    #[test]
    fn the_correction_is_whole_across_the_handover_and_gone_at_the_pole() {
        // What the fade has to do, in the four places it has to do it. The
        // fixture's two lenses overlap by 14.4 degrees; the handover at this
        // direction is four of them and the correction reaches far past both.
        let half = CEILING_DEG.to_radians();
        let band = 4.0f32.to_radians();
        let sine = |degrees: f32| degrees.to_radians().sin();
        let at = |degrees: f32| fade(sine(degrees), 0.5 * band, half);
        // Whole across the handover's own width, so the step it removes is
        // removed exactly and the mixed picture carries one number.
        for degrees in [0.0, 1.0, 2.0] {
            assert_eq!(at(degrees), 1.0, "at {degrees}");
            assert_eq!(fade(-sine(degrees), 0.5 * band, half), 1.0, "at -{degrees}");
        }
        // WIDE, which is the owner's ruling: past the overlap, most of the
        // correction is still being applied, and it is still being applied at
        // twenty and thirty degrees off the seam. Half of it goes to each
        // hemisphere, so neither is handed a black level that is not its own.
        assert!(at(7.22) > 0.98, "at the overlap it is {}", at(7.22));
        assert!(
            (0.75..0.95).contains(&at(20.0)),
            "at 20 deg it is {}",
            at(20.0)
        );
        assert!(
            (0.40..0.75).contains(&at(30.0)),
            "at 30 deg it is {}",
            at(30.0)
        );
        assert!(at(60.0) < 0.20, "at 60 deg it is {}", at(60.0));
        // And gone at the pole, where an azimuth does not exist and a field
        // read at one has to arrive single-valued.
        assert_eq!(at(90.0), 0.0);
        // Monotone in between, with no corner and no kink at either end.
        let mut held = 1.0;
        let mut steepest: f32 = 0.0;
        // To 85 and not to 90: past there the sine of the angle is flat to
        // four decimals and what moves between two steps is the float and not
        // the fade.
        for step in 0..=800 {
            let degrees = 2.0 + 83.0 * step as f32 / 800.0;
            let now = at(degrees);
            assert!(now <= held + 1e-6, "the fade rose at {degrees}");
            steepest = steepest.max((held - now) / (83.0 / 800.0));
            held = now;
        }
        // The steepest it can be, per degree. Over the whole hemisphere that
        // is a fortieth of what the overlap-wide form left, which is the whole
        // of what the owner asked for.
        assert!(steepest < 0.04, "the fade peaks at {steepest} per degree");
        // A file with one lens stream has no overlap and takes nothing
        // anywhere, at any view.
        assert_eq!(fade(0.0, 0.5 * band, 0.0), 0.0);
        assert_eq!(fade(sine(1.0), 0.5 * band, 0.0), 0.0);
    }

    /// The one width, at the three things that decide it (issue #103, stage 8).
    #[test]
    fn the_handover_is_as_wide_as_the_content_allows_and_the_optics_have() {
        let floor = FLOOR_DEG.to_radians();
        let ceiling = CEILING_DEG.to_radians();
        // Content that will not bear a wider handover gets the floor exactly,
        // bit for bit. That is stage 4's picture and it is the ghosting guard:
        // structural, not a taste.
        assert_eq!(
            width(0.0, 0.0, floor, ceiling).to_bits(),
            floor.to_bits(),
            "closed content did not get the floor",
        );
        // Content that will bear it gets the whole of what the two lenses
        // share, which is all there is.
        assert_eq!(width(0.0, 1.0, floor, ceiling), ceiling);
        // Halfway between, so a direction drifting from one to the other has
        // nowhere to step.
        let half = width(0.0, 0.5, floor, ceiling).to_degrees();
        assert!((half - 4.61).abs() < 0.02, "half open is {half}");
        // Monotone in the openness, and never past the ceiling.
        let mut last = 0.0f32;
        for step in 0..=100 {
            let now = width(0.0, step as f32 / 100.0, floor, ceiling);
            assert!(now >= last - 1e-9 && now <= ceiling + 1e-9);
            last = now;
        }
        // And the fold wins over all of it, closed content included: a reading
        // the handover cannot carry opens it whatever the content says.
        let near = 2.4f32.to_radians();
        assert!((width(near, 0.0, floor, ceiling) - near * SLOPE / FOLD).abs() < 1e-9);
    }

    /// What a direction opens for, and what it does not (issue #103, stage 8).
    #[test]
    fn a_direction_opens_for_flat_content_and_shuts_for_a_ghost() {
        // Flat sky: the correlation refuses it at CONTRAST, and its own
        // gradient across two degrees is a tenth of a code at the residual the
        // pass leaves. It opens whole.
        assert!(openness(0.0, 0.0, 2.0 / 255.0) > 0.99);
        // Ploughed soil at infinity, correlating: eight codes of texture, the
        // grid step left over, which is four tenths of a code of ghost. Most
        // of the way open.
        let soil = openness(0.0, 1.0, 8.0 / 255.0);
        assert!((0.5..0.95).contains(&soil), "soil opened {soil}");
        // A wing at 0.8 m the tracking has not caught yet: the whole disparity
        // uncorrected across twenty codes of texture is tens of codes of
        // ghost. Shut.
        assert_eq!(openness(2.4f32.to_radians(), 0.0, 20.0 / 255.0), 0.0);
        // And the same wing once it IS tracked still shuts, because what it
        // would draw twice is what the correlation could not resolve.
        assert_eq!(openness(2.4f32.to_radians(), 1.0, 20.0 / 255.0), 0.0);
        // Monotone in the texture, so a direction watching the light change
        // has nowhere to step.
        let mut held = 1.0;
        for step in 0..200 {
            let now = openness(0.0, 1.0, step as f32 / 255.0 / 10.0);
            assert!(now <= held + 1e-6, "the openness rose at step {step}");
            held = now;
        }
    }

    #[test]
    fn nothing_measured_is_no_correction_and_the_picture_is_the_one_before() {
        // Every path that reaches an untouched picture, and the equality that
        // makes it byte-identical rather than nearly so. A multiply by
        // exactly 1.0 is exact in IEEE; a multiply by whatever `exp(0.0)`
        // happens to return on a driver is not, which is why `split` answers
        // with a match and not with an exponential.
        assert!(pooled_tone(&[]).is_none());
        let dark: Vec<Cell> = (0..AZIMUTHS)
            .map(|_| Cell {
                confidence: 0.0,
                hue_conf: 0.0,
                ..lit_cell(0.02, 0.5, 1.05)
            })
            .collect();
        assert!(pooled_tone(&dark).is_none());
        assert_eq!(Tone::default().split(), [[1.0; 3]; 2]);
        assert_eq!(Tone::read([0.0; 3], 0.9).split(), [[1.0; 3]; 2]);
    }

    #[test]
    fn the_near_field_is_not_pooled_at_all() {
        // The measured rule, and the one that made an additive term appear
        // and then disappear: a direction inside 10 m is the hardest content
        // to line up and the darkest content on a flight, and its photometry
        // is reading the alignment. A ring of nothing but near field has no
        // exposure to report.
        let near: Vec<Cell> = (0..AZIMUTHS)
            .map(|_| lit_cell(NEAR_KNEE_DEG + 0.01, 0.5, 1.10))
            .collect();
        assert!(pooled_tone(&near).is_none());
        // And a ring of both reports the far field's answer alone, not an
        // average of the two.
        let mixed: Vec<Cell> = (0..AZIMUTHS)
            .map(|index| match index % 2 {
                0 => lit_cell(0.02, 0.2 + 0.003 * index as f32, 1.02),
                _ => lit_cell(1.0, 0.5, 0.50),
            })
            .collect();
        let (read, _, _) = pooled_tone(&mixed).expect("half the ring is far field");
        assert!(
            (read[1] - 1.02f32.ln()).abs() < 0.01,
            "read {}",
            read[1].exp()
        );
    }

    #[test]
    fn a_bright_direction_outweighs_a_dark_one_by_its_brightness_squared() {
        // Least squares in codes, which is the pooling measured to leave the
        // smallest step on every capture tried. The claim under test is the
        // weighting itself: what an exposure ratio is measured on is light,
        // so one direction on sky has to outweigh a hundred and twenty seven
        // on soil, and by their brightnesses squared rather than by their
        // count.
        let mut ring: Vec<Cell> = (0..AZIMUTHS).map(|_| lit_cell(0.02, 0.05, 0.90)).collect();
        ring[0] = lit_cell(0.02, 0.90, 1.00);
        let (read, _, _) = pooled_tone(&ring).expect("a ring that correlated");
        // 0.81 of weight against 127 * 0.0025, which is 72 percent of the way
        // from the many to the one. An equal-weight pooling would have landed
        // at 0.9008, which is the number this has to beat.
        assert!(
            (read[1].exp() - 0.9718).abs() < 1e-3,
            "one bright direction left the answer at {}",
            read[1].exp(),
        );
    }

    #[test]
    fn a_runaway_reading_cannot_wash_a_hemisphere_out() {
        // The guard, and what it bounds the damage to. A ring correlating on
        // content that is not the same content at all cannot move either
        // hemisphere by more than an eighth, whatever it reads.
        let broken: Vec<Cell> = (0..AZIMUTHS).map(|_| lit_cell(0.02, 0.5, 4.0)).collect();
        let (read, _, _) = pooled_tone(&broken).expect("a ring that correlated");
        assert_eq!(read, [LIMIT_LN; 3]);
        let split = Tone::read(read, 1.0).split();
        assert!(
            split[0][1] < 1.14 && split[1][1] > 0.88,
            "the guard let through {split:?}",
        );
        // And the widest thing any capture measured is nowhere near it: the
        // bound is four times the widest well-sampled reading, the same
        // multiple `seam`'s own runaway bound uses, and it admits the two
        // thin captures whole rather than half-correcting them.
        assert!(0.9457f32.ln().abs() * 4.0 < LIMIT_LN);
        assert!(0.9076f32.ln().abs() < LIMIT_LN);
    }

    #[test]
    fn the_split_is_symmetric_and_undoes_exactly_what_was_measured() {
        // Neither hemisphere carries the whole change, and the two meet in
        // the middle: that is the entire correction, and it is one line of
        // arithmetic that has to hold at every size.
        for gain in [0.90f32, 0.98, 1.0, 1.05, 1.15] {
            let split = Tone::read([gain.ln().clamp(-LIMIT_LN, LIMIT_LN); 3], 1.0).split();
            for (channel, low) in split[0].iter().enumerate() {
                let lens0 = 100.0 * low;
                let lens1 = 100.0 * gain.clamp(-LIMIT_LN.exp(), LIMIT_LN.exp()) * split[1][channel];
                assert!(
                    (lens0 - lens1).abs() < 1e-3,
                    "at gain {gain} channel {channel} lands at {lens0} and {lens1}",
                );
                // Symmetric: the two multipliers are reciprocals, so the
                // picture's own mean brightness is left where it was.
                assert!((low * split[1][channel] - 1.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn the_gain_settles_at_the_constant_the_far_field_already_uses() {
        // No constant was added for the exposure. It is smoothed at the same
        // rate the band smooths a direction that is not moving, because two
        // auto-exposure loops on two hemispheres are not moving either, and a
        // brightness that breathes is worse than the step it is correcting.
        assert_eq!(TAU_GAIN_S, TAU_FAR_S);
        // One frame of a 3 percent gain arriving, at 30 fps, is under a
        // quarter of a code at a mid grey of 128: below what an 8-bit picture
        // can even carry, which is what "eased below perception" has to mean.
        let step = ease(1.0 / 30.0, TAU_GAIN_S);
        let arriving = 128.0 * (0.5 * 0.03 * step).exp() - 128.0;
        assert!(arriving < 0.25, "the first frame moves {arriving} codes");
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
        // PER ENTRY POINT, because that is what a pipeline is validated
        // against and the three of them reach different variables. `measure`
        // is the one that is anywhere near the limit.
        //
        // Its own, past the two grids and the score table: `winner` and
        // `textured`, then the photometry's per-lane reduction - three floats
        // for the luma sums and, since stage 7, two `vec2` for the chroma sums
        // and one more float for their count. A `vec2<f32>` in workgroup
        // memory is eight bytes aligned to eight, so the colour costs 1280 of
        // these bytes and not the 2048 a `vec3` would have.
        let measure = 4 * (patch + back as usize + shifts) + 8 + 4 * 4 * THREADS + 2 * 8 * THREADS;
        assert!(
            measure <= 16352,
            "`measure` wants {measure} bytes of shared memory",
        );
        // What is left, which is the budget any further per-lane number comes
        // out of: 91 floats a lane, or one more `vec2` array and change.
        assert!(16352 - measure >= 360);
        // `pool` reduces two `vec3` and a float per lane and reaches nothing
        // else. A `vec3` in workgroup memory is sixteen bytes.
        let pool = 2 * 16 * THREADS + 4 * THREADS;
        assert!(pool <= 16352, "`pool` wants {pool} bytes");
    }

    // -------------------------------------------------- stage 5: the other axis

    /// One direction's state with an along-seam reading in it and nothing
    /// else, so the field can be asked questions with no GPU and no footage.
    fn along_cell(index: usize, field: impl Fn(f32) -> f32) -> Cell {
        let phi = index as f32 / AZIMUTHS as f32 * std::f32::consts::TAU;
        Cell {
            disparity: 0.0,
            confidence: KEEP,
            reach_m: 0.033,
            off_epi: field(phi),
            off_conf: KEEP,
            tone: 0.0,
            lit: 0.0,
            chroma: [0.0; 4],
            hue_conf: 0.0,
            open: 0.0,
            offset: [0.0; 3],
        }
    }

    #[test]
    fn a_ring_with_nothing_on_it_asks_for_no_along_seam_correction() {
        // The byte-identity of stage 4, reached by arithmetic and not by a
        // branch: with no evidence the normal matrix is the ridge alone and
        // the right-hand side is zero, so every coefficient is exactly zero.
        let field = Along::fit(&vec![Cell::default(); AZIMUTHS]);
        assert_eq!(field.terms, [0.0; 5]);
        assert_eq!(field.evidence, 0.0);
        assert_eq!(field.at(1.0, 0.0), 0.0);
        assert_eq!(field.at(-0.6, 0.8), 0.0);
    }

    #[test]
    fn the_field_reads_back_each_of_the_three_terms_it_was_given() {
        // The positive control. Each harmonic is planted alone at a size the
        // corpus actually shows and read back at four azimuths, because a fit
        // that cannot recover what it was handed has not measured anything.
        type Shape = fn(f32) -> f32;
        let planted: [(&str, Shape); 3] = [
            ("relative roll", |_| 0.4f32.to_radians()),
            ("principal point", |phi| 0.4f32.to_radians() * phi.cos()),
            ("focal aspect", |phi| {
                0.4f32.to_radians() * (2.0 * phi).sin()
            }),
        ];
        for (name, shape) in planted {
            let cells: Vec<Cell> = (0..AZIMUTHS)
                .map(|index| along_cell(index, shape))
                .collect();
            let field = Along::fit(&cells);
            for step in 0..4 {
                let phi = step as f32 / 4.0 * std::f32::consts::TAU + 0.3;
                let (sin, cos) = phi.sin_cos();
                let read = field.at(cos, sin);
                // The ridge shrinks a whole ring by AZIMUTHS / (AZIMUTHS + 1),
                // which is under a percent and is the only thing between this
                // and an equality.
                assert!(
                    (read - shape(phi)).abs() < 0.02 * shape(phi).abs().max(1e-3) + 1e-5,
                    "{name} at {phi:.2} rad read {} deg for {} deg",
                    read.to_degrees(),
                    shape(phi).to_degrees(),
                );
            }
        }
    }

    #[test]
    fn a_third_cycle_is_not_fitted_and_does_not_reach_the_picture() {
        // What the model does NOT claim. Two cycles is where the measurement
        // stopped indicating structure (docs/research/seam-two-axis.md), and a
        // basis that stops there must leave a third cycle on the floor rather
        // than aliasing it into the terms below.
        let cells: Vec<Cell> = (0..AZIMUTHS)
            .map(|index| along_cell(index, |phi| 0.5f32.to_radians() * (3.0 * phi).cos()))
            .collect();
        let field = Along::fit(&cells);
        for term in field.terms {
            assert!(
                term.to_degrees().abs() < 0.01,
                "a three-cycle ring produced {} deg of a fitted term",
                term.to_degrees(),
            );
        }
    }

    #[test]
    fn one_direction_is_believed_by_half_and_forty_are_believed_whole() {
        // What the ridge is for, stated as the number it produces: a fit is
        // believed in proportion to how much of the ring is behind it, so a
        // file's first frames walk the correction in by arithmetic rather than
        // by a second time constant.
        let held = 0.4f32.to_radians();
        let one = Along::fit(
            &(0..AZIMUTHS)
                .map(|index| match index {
                    0 => along_cell(0, |_| held),
                    _ => Cell::default(),
                })
                .collect::<Vec<_>>(),
        );
        let read = one.at(1.0, 0.0);
        assert!(
            (0.4 * held..0.8 * held).contains(&read),
            "one direction alone applied {} deg of {} deg at its own azimuth",
            read.to_degrees(),
            held.to_degrees(),
        );
        let many = Along::fit(
            &(0..AZIMUTHS)
                .map(|index| match index % 3 {
                    0 => along_cell(index, |_| held),
                    _ => Cell::default(),
                })
                .collect::<Vec<_>>(),
        );
        assert!(
            (many.at(1.0, 0.0) - held).abs() < 0.05 * held,
            "forty directions applied {} deg of {} deg",
            many.at(1.0, 0.0).to_degrees(),
            held.to_degrees(),
        );
    }

    #[test]
    fn the_along_seam_bend_is_exactly_a_relative_roll() {
        // The claim the application law rests on, checked against the
        // calibration's own roll knob rather than against arithmetic written
        // twice. A constant along-seam field displaces lens 1's ray by
        // `w x d`, and `w x d` for a roll of `w` about the body's z is what
        // `super::seam::turned` produces when it turns lens 1's roll by w.
        //
        // **How far to turn it is derived and not chosen**, from
        // `seam::moved`, which is the residual instrument's own prediction of
        // what a knob does to the reading. That is the whole point of the
        // test since stage 6: a field of `f` has to be the roll that the OTHER
        // instrument would say leaves a reading of `-f`, sign included, or the
        // two are measuring in conventions that cannot be read side by side.
        use crate::projection::tests::{FRAME, fixture_lenses};
        let lenses = fixture_lenses();
        let turn_of = |field_deg: f64| {
            let base = crate::seam::mapped(&lenses, FRAME);
            let one = crate::seam::mapped(
                &crate::seam::turned(&lenses, crate::seam::Knob::Roll, 1.0),
                FRAME,
            );
            let at = crate::seam::ring(AZIMUTHS)[0];
            let per_degree =
                crate::seam::moved(&base, &one, 1, &at).expect("the seam is in view")[0];
            -field_deg / per_degree
        };
        let turn = turn_of(0.4);
        assert!(
            turn.abs() > 0.3 && turn.abs() < 0.5,
            "a 0.4 degree field asked for a roll of {turn:.3} degrees",
        );
        let base = crate::seam::mapped(&lenses, FRAME);
        let rolled = crate::seam::mapped(
            &crate::seam::turned(&lenses, crate::seam::Knob::Roll, turn),
            FRAME,
        );
        let cells: Vec<Cell> = (0..AZIMUTHS)
            .map(|index| along_cell(index, |_| 0.4f32.to_radians()))
            .collect();
        let field = Along::fit(&cells);
        // Off the seam plane as well as on it, because the `cos(elevation)`
        // scale is the half of this that is not obvious.
        for theta in [90.0f32, 75.0, 120.0] {
            for phi in [0.0f32, 70.0, 200.0] {
                let (sin_t, cos_t) = theta.to_radians().sin_cos();
                let (sin_p, cos_p) = phi.to_radians().sin_cos();
                let ray = [sin_t * cos_p, sin_t * sin_p, cos_t];
                let bend = base.bend(ray, base.reading_at(ray, &cells, field));
                let bent: [f32; 3] = std::array::from_fn(|c| ray[c] + bend.along[c]);
                let (here, there) = (rolled.project(1, ray), base.project(1, bent));
                if !here.inside || !there.inside {
                    continue;
                }
                let apart = (0..2)
                    .map(|c| (here.pixel[c] - there.pixel[c]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                assert!(
                    apart < 1.0,
                    "at theta {theta} phi {phi} the bend lands {apart:.2} px from the roll",
                );
            }
        }
    }

    #[test]
    fn the_along_seam_bend_costs_no_overlap_and_cannot_fold() {
        // The two properties that let this axis be applied over a whole
        // hemisphere while the epipolar one may only be applied across the
        // band. `perp` is the seam circle's own tangent, so the bend slides
        // content ALONG the circle and never off it, which is why it spends
        // none of the 7.22 degrees the two lenses share; and its gradient is
        // across the band while its displacement is along it, so the Jacobian
        // it adds is off-diagonal and the determinant stays 1.
        let widest = PERP_DEG.to_radians();
        let mut off_the_plane = 0.0f32;
        for index in 0..AZIMUTHS {
            let at = Ring::cell(index, BASELINE);
            let bent: [f32; 3] = std::array::from_fn(|c| at.centre[c] + widest * at.perp[c]);
            let unit = unit(bent);
            off_the_plane = off_the_plane.max(unit[2].asin().abs().to_degrees());
            // Orthogonal to the direction itself, so it is a turn and not a
            // stretch: `|d + a * perp|` is `sqrt(1 + a * a)`, which grows only
            // to second order in `a` and is what makes the bend a rotation
            // rather than a magnification.
            assert!(
                (norm(bent) - (1.0 + widest * widest).sqrt()).abs() < 1e-6,
                "the widest bend changes the ray's length by {}",
                norm(bent) - 1.0,
            );
        }
        // The whole search width spent at once moves a ray off the seam plane
        // by this much, against the 7.22 degrees a side the two lenses share
        // (`the_widest_band_and_its_bend_stay_inside_the_overlap`). It is not
        // exactly zero because the epipolar axis is 0.4 degrees off the body's
        // z, and it is four orders under what the overlap could pay.
        assert!(
            off_the_plane < 0.01,
            "the widest along-seam bend leaves the seam plane by {off_the_plane:.4} deg",
        );
    }

    #[test]
    fn the_depth_control_reads_zero_on_a_stereo_ring_and_one_on_a_leaking_one() {
        // The control that replaces not applying the channel. A ring whose
        // along-seam readings are the camera and whose disparities are the
        // scene must show no relation between them; one where parallax has
        // reached the wrong axis shows all of it.
        let stereo: Vec<Cell> = (0..AZIMUTHS)
            .map(|index| Cell {
                // A scene that varies round the circle, and a camera term
                // that varies differently.
                disparity: (index as f32 * 0.11).sin() * 0.02,
                off_epi: (index as f32 / AZIMUTHS as f32 * std::f32::consts::TAU).cos() * 0.007,
                ..along_cell(index, |_| 0.0)
            })
            .collect();
        let leak = depth_leak(&stereo).expect("a full ring says something");
        assert!(leak.abs() < 0.35, "a stereo ring leaked {leak:+.3}");
        let leaking: Vec<Cell> = stereo
            .iter()
            .map(|cell| Cell {
                off_epi: 0.3 * cell.disparity,
                ..*cell
            })
            .collect();
        let leak = depth_leak(&leaking).expect("a full ring says something");
        assert!(leak > 0.99, "a leaking ring read only {leak:+.3}");
        assert_eq!(depth_leak(&[]), None);
    }

    #[test]
    fn a_file_with_one_lens_stream_is_still_drawn_exactly_as_stage_one_drew_it() {
        // Issue #39's byte-identity, now over two axes. Nothing in the band
        // may reach a picture with no seam in it, and the fallback is
        // arithmetic: no baseline, no ring, no bend, on either axis.
        use crate::projection::tests::{FRAME, fixture_lenses};
        let lenses = fixture_lenses();
        let reframe = crate::seam::mapped(&lenses[..1], FRAME);
        let ray = [0.6, 0.1, 0.8];
        let cells: Vec<Cell> = (0..AZIMUTHS)
            .map(|index| along_cell(index, |_| 0.5f32.to_radians()))
            .collect();
        let bend = reframe.bend(ray, reframe.reading_at(ray, &cells, Along::fit(&cells)));
        assert_eq!(bend.epi, [0.0; 3]);
        assert_eq!(bend.along, [0.0; 3]);
    }
}
