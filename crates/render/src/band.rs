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

/// The table travels four entries to a `vec4`, so a count that is not a
/// multiple of four would lose the last few directions with no error
/// anywhere ([`Table`]).
const _: () = assert!(AZIMUTHS.is_multiple_of(4));

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

/// How long the gate on the way out takes to answer a change in the evidence,
/// in seconds.
///
/// **Its own constant, and seconds rather than [`TAU_NEAR_S`], is the whole
/// point.** [`time_constant`] runs a direction fast when its disparity is
/// near-field sized, because *the wing moves* and a near reading has to track
/// it. The correlator losing the scene is not the wing moving: it is the same
/// content, still there, momentarily not correlating. Those two want opposite
/// responses and until 2026-08-08 they shared a knob. So the value follows the
/// wing at [`TAU_NEAR_S`] and the belief in it fades at 2 s, and a direction
/// that stops confirming its reading **fails towards the reading it held**
/// instead of towards nothing.
///
/// It is [`TAU_FAR_S`]'s value and not [`TAU_FAR_S`] itself, because the two
/// answer different questions and a later measurement may move one without the
/// other. [`KEEP`] is untouched and stays the gate on whether a reading may
/// *enter* the state; this is how much of the state *leaves* it, and those
/// were one constant doing two jobs.
///
/// **The defect it is aimed at** (docs/research/seam-temporal.md 2.2 and 8.2).
/// The state was smoothed and the gate on the way out was not: what reached
/// the picture was [`Cell::disparity`] times `clamp(confidence / KEEP, 0, 1)`
/// read that frame. On the owner's May-01 downward arc the held disparity sat
/// at -0.912 degrees for four seconds while the applied value swung between
/// 0.00 and -47.61 view pixels, **84 frame-to-frame steps over 10 view px and
/// a worst of 46.74**, because the content flickers the correlation and the
/// gate followed it whole.
const TAU_TRUST_S: f32 = 2.0;

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

/// The widest the **adaptive term** may ask for, in degrees.
///
/// It is not a taste and not a margin: it is the widest width the inequality
/// above can ever **ask for**. The search reports at most [`NEAR_DEG`] and
/// refuses anything that peaks against that edge, so `|disparity| / FOLD`
/// cannot exceed this, and a band opened past it would be carrying a reading
/// no frame can produce. Two consequences worth saying out loud: the clamp is
/// inert for every disparity this pass can measure, and widening the search
/// window widens the band with it, with no second number to keep in step.
///
/// **It is not the widest the crossover opens, and has not been since
/// 2026-08-05.** [`width`] applies the floor last and the floor is the
/// camera's - 8.00 degrees on an X4 Air, 3.99 on the ONE X2 - so on every
/// camera in the corpus the floor is what comes back and this ceiling is never
/// reached (`the_adaptive_width_is_inert_under_the_shipped_floor`). What it
/// still is, is the widest a camera whose overlap forced its floor under 2.89
/// can open to.
///
/// What bounds the crossover from the other side is the optics, and that bound
/// is no longer far away. This ceiling plus the bend it carries reaches 4.04
/// degrees off the seam, which inside the calibration fixture's 7.22 a side
/// left 3.18 to spare; the shipped 8 reaches **6.60** and leaves **0.62**, and
/// the ONE X2's 3.99 reaches 4.60 into its own 4.60 a side and leaves nothing
/// (`the_widest_band_and_its_bend_stay_inside_the_overlap`, which measures it
/// off the file's own calibration rather than quoting the format study).
/// `kjerag-spike --bin band` prints that pair **per file** and not one pair for
/// every file, because both numbers are the camera's now.
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
        let basis = terms(cos, sin);
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
            let basis = terms(cos, sin);
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

/// How much evidence a coefficient is shrunk against, in directions.
///
/// One direction's worth, which is a quantity and not a taste: it says a fit is
/// believed in proportion to how much of the ring is behind it, and it makes a
/// ring with nothing on it come out at exactly zero rather than at a division.
/// A term forty directions agree on gives up two percent of itself to this; a
/// term one direction has seen gives up half.
///
/// Public because the pooled field is the same five terms fitted over a ring of
/// azimuths instead of a ring of cells, and it is held against the same
/// evidence ([`super::seam::along_terms`]).
pub const RIDGE: f32 = 1.0;

/// A small symmetric positive definite system, by Gaussian elimination with no
/// pivoting.
///
/// No pivoting is safe here rather than a shortcut taken: the matrix is a Gram
/// matrix plus [`RIDGE`] on the diagonal, so it is positive definite whatever
/// the ring holds and its pivots are never zero, even with no evidence at all.
///
/// WGSL twin: `solve5`.
fn solve(mut normal: [[f32; 5]; 5], mut right: [f32; 5]) -> [f32; 5] {
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

/// One azimuth's worth of what a fitted pose left along the seam: the
/// observation [`Table`] is built out of.
///
/// It is a *leftover*, not a reading: the pose the camera is drawn with has
/// already been taken off it ([`super::seam::left`]), so what is here is the
/// part of the disagreement no pose can describe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leftover {
    /// Azimuth about the body's +x, in radians. The one label a site keeps
    /// under any calibration.
    pub phi: f32,
    /// What the pose left along [`Ring::perp`] there, in radians.
    pub perp: f32,
    /// How much this reading is believed: the correlation behind it, 0 up.
    pub weight: f32,
}

/// The along-seam correction a pose cannot describe: one number per
/// [`AZIMUTHS`] direction, in radians along [`Ring::perp`], read once per
/// camera and held still (issue #103, stage 9).
///
/// **Why a table and not more harmonics.** [`Along`] is five numbers because
/// three named calibration errors are all a *pose* can put on this axis. What
/// is left over is not a pose and has no such shape, so raising the order
/// would be guessing at one; and a harmonic reaches the whole circle, so an
/// azimuth with no evidence behind it would still be moved by the azimuths
/// that have some. A table is refused where it was never measured, and refused
/// means exactly zero.
///
/// **Why it is static.** The along-seam axis is the one parallax cannot reach
/// at any distance (docs/research/seam-two-axis.md 1), so what disagrees on it
/// is the camera rather than the scene, and a camera does not change between
/// sessions. Whether anything is left there that is also a function of azimuth
/// is a separate question, and it is the one below.
///
/// **It is never freer than its evidence.** Every entry is a weighted mean of
/// the readings within [`SMOOTH_DEG`] of it, shrunk towards zero by
/// [`TABLE_RIDGE`], fitted to readings the five terms [`Along`] already applies
/// have been taken off so the two cannot both correct the same thing. An entry
/// with no reading inside the kernel is exactly zero and its neighbours taper
/// into it.
///
/// **Nothing writes one and `Table::REST` ships**, and that survived a second
/// attempt (stage 9 layer 2, docs/research/stage9.md 9). A five-term field
/// pooled per camera was composed into this vehicle and withdrawn: it improves
/// the **unbent** projection every instrument here measures and does nothing in
/// the **delivered** picture, where the per-frame band already holds the
/// along-seam axis, and at one reference view it made the delivered axis about
/// two view pixels worse. Why the band does not simply absorb an applied table
/// is measured in
/// [`a_partial_ring_cannot_fit_away_a_table_over_the_whole_of_it`], and it
/// binds anything that ever fills this uniform.
///
/// **Nothing pooled per azimuth either, and that is a measurement** (stage 9,
/// docs/research/stage9.md). Over nine captures of two cameras, held out on
/// every arm: on the ONE X2 a table costs 4 to 6 percent under every reduction;
/// on the X4 Air the effect runs -1 to +2 percent depending on the estimator,
/// which is nothing either way. The kernel sweep is flat from 4 to 36 degrees on
/// both, in the table-alone arm. And what survives the five terms has an
/// amplitude of 0.004 to 0.005 degrees - the orthogonal part of two
/// root-mean-squares 0.0199 and 0.0195, not their difference - which is
/// **0.13 to 0.16 source px, an eighth of a pixel**
/// and two to three times finer than `--bin crossing` resolves. Removing all of
/// it perfectly would improve the held-out residual by 1.8 to 4 percent,
/// depending on which arm's residual it is measured against, and a fitted table
/// does not get it. The same test recovers the five-term field on
/// 9 captures of 9, so that is a refusal and not a blind spot.
///
/// **Above those terms**, and the qualifier is the whole finding. The five
/// terms themselves are a camera and do reproduce: at full density they agree
/// on 18 of 18 pairs of captures, and fitted on other flights only they take
/// the pooled leftover from 0.0536 to 0.0211 degrees on the X4 and 0.0606 to
/// 0.0249 on the X2, nine captures of nine improved. That is [`Along`]'s
/// territory, computed per session already, and measured per camera by
/// [`super::seam::along_terms`] whose answer the pool now stores without
/// applying it. What a per-azimuth table would carry is what is left over it,
/// and there is a hundredth of a pixel of that.
///
/// The per-azimuth mechanism stays here because the refusal had to be
/// checkable and because a camera that needs one may still turn up.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Table {
    /// Four to a `vec4`, because an array of scalars in a uniform block
    /// strides sixteen bytes and this one would be four times the size for
    /// nothing.
    packed: [[f32; 4]; AZIMUTHS / 4],
}

impl Default for Table {
    fn default() -> Self {
        Self::REST
    }
}

impl Table {
    /// Nothing measured, which is exactly the picture before this existed:
    /// every entry zero, so [`Self::at`] returns zero at every azimuth and the
    /// bend it adds is the zero vector.
    pub const REST: Self = Self {
        packed: [[0.0; 4]; AZIMUTHS / 4],
    };

    /// The entry at one azimuth, in radians, from that azimuth's own cosine
    /// and sine, interpolated between the two directions it lands between and
    /// wrapping: the field is a circle and a step between neighbours would be
    /// a step in the picture.
    ///
    /// WGSL twin: `table_at`, which is handed the `low` and `mix` the cell
    /// lookup has already worked out rather than taking a second `atan2`.
    pub fn at(&self, cos: f32, sin: f32) -> f32 {
        let turn = sin.atan2(cos) / std::f32::consts::TAU * AZIMUTHS as f32;
        let low = turn.floor();
        self.between(low as i32, turn - low)
    }

    /// The same from the azimuth already resolved into a direction index and
    /// the fraction past it.
    pub fn between(&self, low: i32, mix: f32) -> f32 {
        let entry = |step: i32| {
            let index = (low + step).rem_euclid(AZIMUTHS as i32) as usize;
            self.packed[index / 4][index % 4]
        };
        entry(0) + (entry(1) - entry(0)) * mix
    }

    /// One entry per direction, in radians, for an instrument.
    pub fn entries(&self) -> [f32; AZIMUTHS] {
        std::array::from_fn(|index| self.packed[index / 4][index % 4])
    }

    /// The same, refused whole where any entry is larger than a calibration or
    /// is not a number.
    ///
    /// **Whole support or nothing**, which is the acceptance rule a later
    /// applied candidate inherits (docs/research/stage9.md 7): a table is one
    /// field over one ring, and a ring with one direction knocked out of it is
    /// the hole-filling this type exists to refuse. What builds one this way is
    /// [`super::seam::along_table`], where a single direction the projection
    /// cannot reach is a camera whose seam circle leaves a lens's picture and
    /// not a reading to be patched around.
    pub fn plausible(entries: [f32; AZIMUTHS]) -> Option<Self> {
        entries
            .iter()
            .all(|entry| entry.is_finite() && entry.abs() <= TABLE_LIMIT_RAD)
            .then(|| Self::of_entries(entries))
    }

    /// A table straight from its entries, which is how a planted control and a
    /// stored one are both built.
    pub fn of_entries(entries: [f32; AZIMUTHS]) -> Self {
        Self {
            packed: std::array::from_fn(|group| {
                std::array::from_fn(|lane| entries[group * 4 + lane])
            }),
        }
    }

    /// Whether this table moves anything at all. A table with nothing in it is
    /// the picture stage 6 drew, byte for byte.
    pub fn is_rest(&self) -> bool {
        *self == Self::REST
    }

    /// The table these leftovers support, and nothing more.
    ///
    /// Three steps, each of which can be looked at on its own: smooth the
    /// readings onto the ring, take back out the five terms [`Along`] already
    /// applies, and refuse anything larger than a calibration.
    ///
    /// The width is [`SMOOTH_DEG`] everywhere the app builds one. It is an
    /// argument because it is the one number in here that had to be measured
    /// against held-out captures rather than argued for, and the instrument
    /// that measured it has to be able to ask for a different one.
    pub fn of(left: &[Leftover], smooth_deg: f32) -> Self {
        let (mut values, _) = smoothed(&levelled(left), smooth_deg);
        for value in &mut values {
            *value = value.clamp(-TABLE_LIMIT_RAD, TABLE_LIMIT_RAD);
        }
        Self::of_entries(values)
    }

    /// How much evidence each direction has behind it, in readings: the same
    /// kernel [`Self::of`] smooths with, with the readings' values left out.
    ///
    /// An entry with less than one reading's worth is one the ridge is taking
    /// more than half of, which is the taper rather than a measurement. For an
    /// instrument; the pass has no use for it, because the shrinking is
    /// already in the entry.
    pub fn evidence(left: &[Leftover], smooth_deg: f32) -> [f32; AZIMUTHS] {
        smoothed(left, smooth_deg).1
    }

    /// The table written down, one entry per line in radians. This is how a
    /// pooled calibration reaches an instrument, and how a fitted one is kept
    /// between runs.
    pub fn write(&self) -> String {
        self.entries()
            .iter()
            .map(|entry| format!("{entry}\n"))
            .collect()
    }

    /// The same, read back. `None` unless there are exactly [`AZIMUTHS`] of
    /// them and every one is a number: a short table is a truncated file, and
    /// filling the rest from neighbours is the hole-filling this type exists
    /// to refuse.
    pub fn read(text: &str) -> Option<Self> {
        let entries: Vec<f32> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.trim().parse::<f32>().ok())
            .collect::<Option<_>>()?;
        let entries: [f32; AZIMUTHS] = entries.try_into().ok()?;
        entries
            .iter()
            .all(|e| e.is_finite())
            .then(|| Self::of_entries(entries))
    }
}

/// How far along the seam one reading is allowed to speak, in degrees of
/// azimuth.
///
/// **A half-width**: the kernel reaches this far either side, so its window is
/// twice this and it holds about five of the ring's readings.
///
/// A reading is a correlation over a patch as wide as
/// [`super::seam::Probe::span`] - 3.7 degrees - and the ring is read at 72
/// azimuths five degrees apart, so neighbouring readings already share content
/// and nothing here can resolve below that. What one reading is worth on its own is measured: a
/// planted per-azimuth field comes back with 0.05 degrees of scatter per
/// reading against a leftover of 0.064 to 0.128 degrees rms per capture
/// (docs/research/stage9.md 4 and 5), so an entry resting on one azimuth would
/// be mostly that scatter.
///
/// **It is not a width chosen for predicting anything, because on the corpus
/// that decides nothing predicts.** On the owner's six flights no width beats
/// no table on a held-out capture at all. Ten to twelve degrees is what the
/// one corpus where a table is even marginally positive prefers, and it is
/// what a table would be built at if one ever were. No shipped table uses it,
/// because no table is fitted.
pub const SMOOTH_DEG: f32 = 12.0;

/// How much evidence an entry is shrunk against, in readings.
///
/// [`RIDGE`]'s argument, one axis over: an entry is believed in proportion to
/// how many readings are behind it, an entry with none comes out at exactly
/// zero rather than at a division, and the edge of the support tapers in by
/// arithmetic rather than by a second rule.
const TABLE_RIDGE: f32 = 1.0;

/// The largest entry a table may carry, in radians.
///
/// Half a degree, which is the argument `seam`'s own runaway guard makes at
/// this scale. Over the owner's six flights, 299 readings that passed the
/// along-seam plausibility gate, the largest single leftover is 0.332 degrees
/// and twelve of them are past 0.2; ungated the tail reaches 2.47. So this sits
/// above every reading a calibration produced on that corpus and below what
/// the correlations that found the wrong feature produced, and an entry past
/// it is the second kind (docs/research/stage9.md 5).
const TABLE_LIMIT_RAD: f32 = 0.5 * std::f32::consts::PI / 180.0;

/// Every entry's weighted mean of the readings near it, and how much evidence
/// that was, one per direction.
///
/// The kernel is a raised cosine over [`SMOOTH_DEG`], which is zero at its own
/// edge and has no step anywhere: a top hat would put a corner in the picture
/// wherever a reading walked in or out of one entry's window.
fn smoothed(left: &[Leftover], smooth_deg: f32) -> ([f32; AZIMUTHS], [f32; AZIMUTHS]) {
    let mut values = [0.0f32; AZIMUTHS];
    let mut weights = [0.0f32; AZIMUTHS];
    let width = smooth_deg.to_radians();
    for (index, (value, weight)) in values.iter_mut().zip(&mut weights).enumerate() {
        let phi = index as f32 / AZIMUTHS as f32 * std::f32::consts::TAU;
        let mut total = 0.0;
        for reading in left {
            let near = kernel(wrapped(reading.phi - phi), width) * reading.weight.max(0.0);
            *weight += near;
            total += near * reading.perp;
        }
        *value = total / (*weight + TABLE_RIDGE);
    }
    (values, weights)
}

/// A raised cosine of half-width `width`, zero at and past it.
fn kernel(apart: f32, width: f32) -> f32 {
    match apart.abs() < width {
        true => 0.5 * (1.0 + (std::f32::consts::PI * apart / width).cos()),
        false => 0.0,
    }
}

/// An angle brought into `-PI..PI`, which is what makes the kernel wrap.
fn wrapped(angle: f32) -> f32 {
    let turn = std::f32::consts::TAU;
    (angle + std::f32::consts::PI).rem_euclid(turn) - std::f32::consts::PI
}

/// The readings with the five terms [`Along`] already applies taken back out
/// of them.
///
/// Without this the two would both correct the low-order part and the picture
/// would be over-turned by however much they agreed on.
///
/// **Almost orthogonal to the pass's own field, not exactly.** The fit is
/// shrunk by [`RIDGE`] like every other fit in this file, so a cycle term the
/// readings agree on keeps about `2/n` of itself, which is half a percent of a
/// pose over three hundred readings and half of one over two
/// (`the_five_terms_the_pass_already_applies_are_taken_back_out`). What the
/// table carries is what a pose and a five-term fit cannot say, plus that.
///
/// **Off the readings and not off the smoothed ring**, which is not a detail:
/// a harmonic reaches the whole circle, so subtracting one from the ring
/// would move every direction the readings never reached, and those are
/// exactly the directions this type exists to leave alone. Taken off the
/// readings, a direction with no reading near it has a numerator of zero and
/// stays at zero.
fn levelled(left: &[Leftover]) -> Vec<Leftover> {
    let mut normal = [[0.0f32; 5]; 5];
    let mut right = [0.0f32; 5];
    for reading in left {
        let (sin, cos) = reading.phi.sin_cos();
        let basis = terms(cos, sin);
        let weight = reading.weight.max(0.0);
        for row in 0..5 {
            for (column, term) in basis.iter().enumerate() {
                normal[row][column] += weight * basis[row] * term;
            }
            right[row] += weight * basis[row] * reading.perp;
        }
    }
    for (term, row) in normal.iter_mut().enumerate() {
        row[term] += RIDGE;
    }
    let low = Along::read(solve(normal, right), 0.0);
    left.iter()
        .map(|reading| {
            let (sin, cos) = reading.phi.sin_cos();
            Leftover {
                perp: reading.perp - low.at(cos, sin),
                ..*reading
            }
        })
        .collect()
}

/// The five basis functions [`Along`] is written in, from an azimuth's own
/// cosine and sine.
fn terms(cos: f32, sin: f32) -> [f32; 5] {
    [1.0, cos, sin, cos * cos - sin * sin, 2.0 * cos * sin]
}

/// The same five functions at one azimuth, in `f64`.
///
/// A second expression of one identity, which is what
/// [`the_two_bases_are_the_same_five_functions`] is for. It exists because the
/// two live at different precisions for different reasons: [`terms`] is the
/// twin of what a fragment shader computes and may not be widened without
/// changing every pixel the CPU map draws, while the pooled field
/// ([`super::seam::along_terms`]) is a least squares over hundreds of readings
/// and is fitted in `f64` like every other fit in this repository.
pub fn basis(phi: f64) -> [f64; 5] {
    let (sin, cos) = phi.sin_cos();
    [1.0, cos, sin, cos * cos - sin * sin, 2.0 * cos * sin]
}

/// One direction's state, as the compute pass writes it and the fragment
/// shader reads it.
///
/// Eight floats and every one of them is read by something: the two axes and
/// their two confidences by the pass, the reach only by an instrument, the
/// next two by the pooling that follows the measurement, and the last by the
/// bend itself. Zero is the state a file opens in and the state a direction
/// that has never correlated stays in, and a zero on either axis is no bend at
/// all.
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
    /// **What the pass actually applies of [`Self::disparity`]**, 0 to 1: the
    /// gate on the way out, filtered at [`TAU_TRUST_S`] and stored rather than
    /// recomputed per fragment (docs/research/seam-temporal.md 8.2).
    ///
    /// It eases towards `clamp(confidence / KEEP, 0, 1)`, which is the whole
    /// of what the fragment shader used to compute for itself every frame.
    /// [`Self::believe`] is the one step, and the reason it is a field rather
    /// than an expression is that a filter needs somewhere to keep yesterday.
    ///
    /// Last in the struct rather than beside [`Self::confidence`] where it
    /// belongs, because [`Self::write`] is a file format two instruments hand
    /// to each other, and appending leaves the seven columns they already read
    /// where they are.
    pub trust: f32,
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
                    "{} {} {} {} {} {} {} {}\n",
                    cell.disparity,
                    cell.confidence,
                    cell.reach_m,
                    cell.off_epi,
                    cell.off_conf,
                    cell.tone,
                    cell.lit,
                    cell.trust,
                )
            })
            .collect()
    }

    /// The same, read back. `None` on any line that is not seven numbers or
    /// eight.
    ///
    /// The eighth is [`Self::trust`] and a file written before it existed has
    /// none. What such a file gets is the value that build applied,
    /// `clamp(confidence / KEEP, 0, 1)`, rather than a zero: zero is "apply
    /// nothing" and would quietly turn an old trace into a picture with no
    /// band in it.
    pub fn read(text: &str) -> Option<Vec<Self>> {
        text.lines()
            .map(|line| {
                let mut numbers = line.split_whitespace().map(str::parse::<f32>);
                let mut next = || numbers.next()?.ok();
                let mut cell = Self {
                    disparity: next()?,
                    confidence: next()?,
                    reach_m: next()?,
                    off_epi: next()?,
                    off_conf: next()?,
                    tone: next()?,
                    lit: next()?,
                    trust: 0.0,
                };
                cell.trust = next().unwrap_or((cell.confidence / KEEP).clamp(0.0, 1.0));
                Some(cell)
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

    /// One step of the gate on the way out, over `seconds` of media time.
    ///
    /// **WGSL twin of `believe`**, and a twin rather than a description: the
    /// pass runs this on the GPU where no test can reach it, and what
    /// [`TAU_TRUST_S`] claims about a confidence dropout is claimed about a
    /// function `cargo test` can call with no device and no footage.
    ///
    /// `fresh` is a **reset frame** - a seek, a new file, the first frame -
    /// and it takes the answer whole for the same reason the disparity and the
    /// confidence do: there is no picture behind a cut for an ease to be
    /// continuous with, and creeping in from zero over two seconds would draw
    /// the first seconds after every seek with a correction of nearly nothing
    /// (issue #103, stage 6, whose argument this keeps exactly where it was
    /// made).
    ///
    /// **A direction arriving mid-shot is not a reset frame and walks in.**
    /// That is the whole of the arrival staging: until 2026-08-08 a direction
    /// whose trust was nothing took its answer whole wherever it was, and at
    /// the owner's `down1` and `down3` that was a single delivered step of
    /// **56.7 and 56.2 view px**, the largest the band delivers anywhere.
    /// Mid-film there IS a picture behind an arriving direction - the
    /// uncorrected one - so the walk starts from what is on the screen. It is
    /// not only the first arrival: a direction that stops correlating gives up
    /// its evidence and its next reading is an arrival again, and the same
    /// line covers both because both are the same line.
    ///
    /// What this does **not** change: the disparity and the confidence still
    /// arrive whole, so the state holds the right answer from the first frame
    /// it has one and nothing is learned twice. What walks is only how much of
    /// it reaches the picture.
    pub fn believe(&mut self, seconds: f32, fresh: bool) {
        let want = (self.confidence / KEEP).clamp(0.0, 1.0);
        let learn = match fresh {
            true => 1.0,
            false => ease(seconds, TAU_TRUST_S),
        };
        self.trust += (want - self.trust) * learn;
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
pub fn pooled_gain(cells: &[Cell]) -> Option<(f32, f32)> {
    let mut weight = 0.0;
    let mut total = 0.0;
    let mut count = 0.0;
    for cell in cells {
        if cell.disparity.abs() >= NEAR_KNEE_DEG.to_radians() {
            continue;
        }
        let believed = (cell.confidence / KEEP).clamp(0.0, 1.0);
        let trust = believed * cell.lit * cell.lit;
        weight += trust;
        total += trust * cell.tone.exp();
        count += believed;
    }
    match weight > 0.0 {
        true => Some((
            (total / weight).ln().clamp(-LIMIT_LN, LIMIT_LN),
            count / AZIMUTHS as f32,
        )),
        false => None,
    }
}

/// The disparity the shader may actually bend by, in radians: what was
/// measured, clamped to what the crossover can carry without folding.
///
/// `band` is the crossover width in radians, which since stage 4 is
/// [`width`]'s answer for that same disparity rather than a constant. See
/// [`FOLD`].
///
/// **Divided by the blend curve's own peak gradient.** The inequality the
/// clamp comes out of was written against a **linear** crossfade, whose share
/// walks one whole unit across one whole band, so the gradient was 1 and
/// dropped out of it. Since 2026-08-08 the crossfade is a power curve
/// ([`super::projection::BLEND_POWER`]) whose peak slope at the seam is its
/// exponent, so it walks the same unit that many times faster and the
/// disparity that sits on the clamp is that many times smaller. Without this
/// division the curve folds the ONE X2, whose support is 3.99 degrees against
/// a search that reads out to 2.6
/// (`projection::tests::the_blend_curve_cannot_fold_the_narrowest_camera`).
///
/// WGSL twin: `carried`.
pub fn carried(disparity_rad: f32, band_rad: f32) -> f32 {
    let limit = FOLD * band_rad / super::projection::BLEND_POWER;
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
///   `FOLD * floor` already satisfies the inequality at the floor, so the
///   floor is what comes back, bit for bit. A file with one lens stream and a
///   direction that has never correlated both read zero and both get the
///   floor.
/// - **It never opens further than it has to.** A wider handover draws more
///   of the picture twice, so the narrowest width that does not fold is also
///   the sharpest one available.
/// - **It is inert at the width that ships today.** This term cannot exceed
///   [`WIDEST_DEG`], 2.89 degrees, and `super::projection::CROSSOVER_DEG` is
///   8, so on every camera whose overlap affords that floor the floor is the
///   answer at every disparity the search can report and this function is a
///   constant (`the_adaptive_width_is_inert_under_the_shipped_floor`). It is
///   kept rather than deleted because the floor is not a constant of the
///   picture any more but of the camera ([`affordable`]), and a camera whose
///   overlap forces it under 2.89 gets stage 4 back.
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

/// How far off the seam a handover of this width reaches, in radians: half the
/// band, plus the widest bend that band can carry.
///
/// The bend is bounded twice and the tighter bound is the answer. [`carried`]
/// clamps it to [`FOLD`] of the width, and the search cannot report more than
/// [`NEAR_DEG`] however wide the band is, so past [`WIDEST_DEG`] a band that
/// widens reaches further by only half of what it widened by.
pub fn reach(width_rad: f32) -> f32 {
    0.5 * width_rad + (FOLD * width_rad).min(NEAR_DEG.to_radians())
}

/// The widest handover a camera whose two lenses overlap by `overlap_rad` can
/// carry, in radians: the width whose [`reach`] lands exactly on the edge of
/// the picture both lenses have.
///
/// **The optics bound the handover and the camera is not always the one the
/// width was chosen on.** Past this bound the crossover stops being what hands
/// the picture over, and the way it stops is not a bad fetch. The coverage
/// test is taken on the unbent ray (`super::projection::Reframe::covers`) and
/// the bend then moves the sample, but a bent ray that lands outside that
/// lens's own boundary comes back `inside == false`,
/// `super::projection::claim` returns exactly zero for it, and the fragment
/// shader reads a lens only where its weight is positive (`super::scene`,
/// `picture`). So what a ray past this edge gets is the other lens alone,
/// taken there by the **coverage depth** - a distance transform that reaches
/// zero at the rim - instead of by the ramp the width was chosen as, and where
/// both lenses miss the pixel is transparent. The price of crossing this bound
/// is a handover cut short by the optics on one side while it is still open on
/// the other; the bound is what keeps the crossover's own ramp the thing that
/// decides the blend.
///
/// Measured off the owner's own captures with `kjerag-spike --bin band`
/// (2026-08-05): six X4 Air files overlap by 14.56 to 15.02 degrees and afford
/// **9.36 to 9.82**, and the calibration fixture overlaps by 14.44 and affords
/// 9.24. The ONE X2 overlaps by 9.19 and affords **3.99**, which is under the
/// 8 the picture asks for, so that camera hands over across 3.99
/// (`the_narrow_overlap_camera_gets_the_width_it_can_pay`).
///
/// **Every one of those is under the file's own seam correction**, which is
/// what the pass draws with, and it is not the same answer as the factory
/// calibration's: a fit moves the principal point, which moves each lens's
/// coverage boundary, which moves the overlap. On the X2 the factory
/// calibration affords 4.91 and its own pooled fit affords 3.99, so the width
/// follows the calibration in and the app reports it after the fit lands
/// rather than before.
///
/// Two regimes because [`reach`] has two. A camera with room to spare pays
/// half a degree of overlap per degree of width, because the bend it carries
/// has stopped growing at [`NEAR_DEG`]; one without pays `0.5 + FOLD`, because
/// there the fold clamp is still what bounds the bend.
pub fn affordable(overlap_rad: f32) -> f32 {
    let half = 0.5 * overlap_rad;
    match half >= reach(WIDEST_DEG.to_radians()) {
        true => 2.0 * (half - NEAR_DEG.to_radians()),
        false => half / (0.5 + FOLD),
    }
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
         const TAU_TRUST = {trust_s:?};\n\
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
        trust_s = TAU_TRUST_S,
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

/// How many bytes the state buffer is: the pooled [`Tone`], the pooled
/// [`Along`], then one [`Cell`] per direction.
pub(crate) const BYTES: u64 = (CELLS_AT + AZIMUTHS * std::mem::size_of::<Cell>()) as u64;

/// Where the along-seam field starts in that buffer.
pub(crate) const ALONG_AT: usize = std::mem::size_of::<Tone>();

/// Where the cells start in that buffer, for the readback that unpacks it.
pub(crate) const CELLS_AT: usize = ALONG_AT + std::mem::size_of::<Along>();

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
  off_conf: f32,
  tone: f32,
  lit: f32,
  trust: f32,
};

// What the pass applies of a direction's reading, out of the evidence behind
// it, before the filter on the way out gets to it. Rust twin: the `want` in
// `Cell::believe`.
fn believed(confidence: f32) -> f32 {
  return clamp(confidence / KEEP, 0.0, 1.0);
}

struct Along {
  terms: array<f32, 5>,
  evidence: f32,
  pad0: f32,
  pad1: f32,
};

struct State {
  tone: Tone,
  along: Along,
  cells: array<Cell, AZIMUTHS>,
};

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
  out.along = vec3<f32>(0.0);
  out.crossover = reframe.crossover;
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
  let applied = carry(a, b, mix);
  // The along-seam axis is NOT read cell by cell. It is one fitted field over
  // the whole circle, because the phenomenon is one - a relative pose error
  // with a constant, a one-cycle and a two-cycle term - and because a field
  // with holes in it, applied over a whole hemisphere, warps a horizon instead
  // of moving it. `flat / reach` is this azimuth's cosine and sine already.
  // Rust twins: `Along::at` and `Reframe::reading_at`.
  //
  // Plus what no pose can describe, read off this camera and held still
  // (stage 9). It is the same displacement on the same axis at a higher
  // order, and the table is levelled against the five terms above when it is
  // built, so the two never correct the same thing twice. Rust twins:
  // `Table::at` and `Reframe::bent`.
  let along = along_at(band.along, flat.x / reach, flat.y / reach) + table_at(low, mix);
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
  out.crossover = band_width(applied);
  // Divided by the blend curve's peak gradient: the fold inequality was
  // written against a linear crossfade, whose gradient was 1. Rust twin:
  // `carried`.
  let limit = FOLD * out.crossover / BLEND_POWER;
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

// The epipolar channel of one ray: the two cells' values mixed at their own
// evidence, then taxed by how much of that evidence has reached `KEEP`.
//
// `KEEP` is the correlation a single reading has to reach before it may move
// the state at all, and a confidence is the smoothed value of that same
// number, so a direction whose recent readings have not been reaching that
// gate is applied proportionally less. No new constant: the threshold a
// reading must pass is the threshold a smoothed reading is trusted at. Zero
// evidence gives exactly zero, by arithmetic. Rust twin: `Reframe::channel`.
//
// The tax is each cell's OWN filtered gate (`Cell.trust`), mixed - already
// clamped, and clamped on the cell side of the mix rather than after it,
// because a filter needs somewhere to keep yesterday and a fragment has
// nowhere. Rust twin: `Reframe::strength`.
fn carry(a: Cell, b: Cell, mix: f32) -> f32 {
  let ea = a.confidence * (1.0 - mix);
  let eb = b.confidence * mix;
  let total = ea + eb;
  if total <= 0.0 {
    return 0.0;
  }
  return (ea * a.disparity + eb * b.disparity) / total * mix2(a.trust, b.trust, mix);
}

// How wide the handover has to be to carry this disparity without folding,
// never narrower than the crossover the projection ships and never wider than
// the widest reading the search can return. Rust twin: `width`.
//
// The EPIPOLAR reading only, which is what keeps stage 4's acceptance exactly
// where stage 4 measured it: the along-seam bend does not fold and therefore
// does not ask the band for room, so this function is called with the same
// argument it was called with before stage 5 and answers the same width.
fn band_width(disparity: f32) -> f32 {
  return max(min(abs(disparity) / FOLD, WIDEST), reframe.crossover);
}

fn mix2(a: f32, b: f32, t: f32) -> f32 {
  return a + (b - a) * t;
}

// The stored along-seam table at one azimuth, in radians, between the two
// directions the cell lookup already resolved and wrapping with it: one
// `atan2` decides both fields, so this one is a pair of loads and a mix.
//
// Zero everywhere on a camera nothing has been pooled for, and zero at any
// azimuth the readings never reached, which is the picture stage 6 drew.
// Rust twin: `Table::between`.
fn table_at(low: i32, mix: f32) -> f32 {
  let a = table_entry(low);
  let b = table_entry(low + 1);
  return mix2(a, b, mix);
}

fn table_entry(index: i32) -> f32 {
  let at = u32(index + i32(AZIMUTHS)) % AZIMUTHS;
  return reframe.table[at / 4u][at % 4u];
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
  // Which of this frame's `stride` rounds of the circle it reads.
  slice: f32,
  // How many directions apart the ones this frame reads are: 1 on a reset
  // frame, which sweeps the whole ring so that a reset reaches every
  // direction and not only the slice it landed on. Rust twin: `Watch::stride`.
  stride: f32,
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
    textured = has_picture();
  }
  workgroupBarrier();
  if !textured {
    // Nothing to read here. The state keeps what it had and gives up the
    // evidence behind it, which is the same rule a refusal takes.
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
  let perp = (found % PERP_SHIFTS) * u32(PERP_STEP);
  let width = u32(2 * HALF + 1);
  var sum0 = 0.0;
  var sum1 = 0.0;
  var count = 0.0;
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
fn has_picture() -> bool {
  var sum = 0.0;
  var square = 0.0;
  var count = 0.0;
  for (var i = 0u; i < PATCH; i += 1u) {
    let a = front[i];
    if a < 0.0 {
      // No picture of part of the patch. The correlation refuses that anyway,
      // one candidate at a time, and this refuses it once.
      return false;
    }
    sum += a;
    square += a * a;
    count += 1.0;
  }
  let spread = square - sum * sum / count;
  return spread > 0.0 && sqrt(spread / count) >= CONTRAST;
}

// A direction with nothing in the picture to read. What it keeps is the
// measurement and what it gives up is the evidence, which is the rule
// everywhere else in this file: the reading was true when it was taken and may
// be true still, but nothing is confirming it.
fn forget(cell: u32, at: Ring) {
  var held = band.cells[cell];
  if watch.reset != 0.0 {
    held = Cell(0.0, 0.0, at.reach_m, 0.0, 0.0, 0.0, 0.0, 0.0);
  }
  held.reach_m = at.reach_m;
  held.confidence -= held.confidence * ease(watch.seconds, time_constant(held.disparity));
  held.off_conf -= held.off_conf * ease(watch.seconds, TAU_FAR);
  believe(&held);
  band.cells[cell] = held;
}

// The gate on the way out, one step. Rust twin: `Cell::believe`.
//
// It eases towards this frame's evidence at TAU_TRUST, which is the constant
// the belief in a reading fades at, deliberately not the constant the reading
// itself tracks at. A RESET frame takes it whole: a seek, a new file or the
// first frame has no picture behind it for an ease to be continuous with, and
// creeping in from zero over two seconds would draw the first seconds after
// every seek with a correction of nearly nothing (issue #103, stage 6).
//
// A direction ARRIVING mid-shot is not a reset frame and walks in like every
// other change to the gate, because what is behind it there is the
// uncorrected picture the owner has been looking at.
fn believe(held: ptr<function, Cell>) {
  let want = believed((*held).confidence);
  let learn = select(ease(watch.seconds, TAU_TRUST), 1.0, watch.reset != 0.0);
  (*held).trust += (want - (*held).trust) * learn;
}

// The peak, the gates, and one step of the filter. One thread, because it is
// a few dozen operations over a table the whole workgroup has already filled.
fn settle(cell: u32, at: Ring) {
  var held = band.cells[cell];
  if watch.reset != 0.0 {
    held = Cell(0.0, 0.0, at.reach_m, 0.0, 0.0, 0.0, 0.0, 0.0);
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
  // Stage 3's own gate, unchanged: the photometry is read at the shift that
  // made the two patches the same content, so a shift that did not establish
  // that is not one to read a brightness at.
  if !epi_pinned {
    read_photometry(&held);
  }
  believe(&held);
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
fn read_photometry(held: ptr<function, Cell>) {
  var sum0 = 0.0;
  var sum1 = 0.0;
  var count = 0.0;
  for (var i = 0u; i < THREADS; i += 1u) {
    sum0 += lit0[i];
    sum1 += lit1[i];
    count += lit_n[i];
  }
  // Clipping left nothing to read. The direction keeps what it had, which is
  // the same rule as a refusal: what is absent is a confirmation, not a
  // reason to believe the opposite.
  if count <= 0.0 || sum0 <= 0.0 || sum1 <= 0.0 {
    return;
  }
  (*held).tone = log(sum1 / sum0);
  (*held).lit = sum0 / count;
}

// The pooled exposure, over the whole ring and over media time. Rust twin:
// `pooled_gain`.
//
// One workgroup, dispatched straight after the measurement and in the same
// pass, so what it pools is what was just written. A direction contributes at
// the weight the bend trusts it at, so a direction whose evidence has faded
// fades out of the exposure too and one that never correlated was never in it.
// It is THIS FRAME's evidence and deliberately not the filtered `Cell.trust`
// the bend applies: an exposure is not a geometry and nothing has measured
// that it wants the same constant.
@compute @workgroup_size(THREADS)
fn pool(@builtin(local_invocation_index) lane: u32) {
  var weight = 0.0;
  var total = 0.0;
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
    if abs(cell.disparity) >= NEAR_KNEE {
      continue;
    }
    // Least squares in codes: each direction weighs its own brightness
    // squared, which is what makes this the exposure ratio and not an average
    // over whichever patches happened to be dark.
    let believed = clamp(cell.confidence / KEEP, 0.0, 1.0);
    let trust = believed * cell.lit * cell.lit;
    weight += trust;
    total += trust * exp(cell.tone);
    count += believed;
  }
  pooled_weight[lane] = weight;
  pooled_total[lane] = total;
  pooled_count[lane] = count;
  workgroupBarrier();
  if lane != 0u {
    return;
  }
  var sum_weight = 0.0;
  var sum_total = 0.0;
  var sum_count = 0.0;
  for (var i = 0u; i < THREADS; i += 1u) {
    sum_weight += pooled_weight[i];
    sum_total += pooled_total[i];
    sum_count += pooled_count[i];
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
  // `sum_total / sum_weight` is the least-squares gain itself, because the
  // weights already carry the brightness squared. Its log is what the split
  // halves.
  let read = clamp(log(sum_total / sum_weight), -LIMIT_LN, LIMIT_LN);
  let step = ease(watch.seconds, TAU_GAIN);
  held.log_gain += (read - held.log_gain) * step;
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

    /// The two expressions of the five basis functions are one identity, and
    /// only their precision differs.
    #[test]
    fn the_two_bases_are_the_same_five_functions() {
        for step in 0..AZIMUTHS {
            let phi = step as f64 / AZIMUTHS as f64 * std::f64::consts::TAU;
            let (sin, cos) = (phi as f32).sin_cos();
            for (wide, narrow) in basis(phi).iter().zip(terms(cos, sin)) {
                assert!(
                    (*wide as f32 - narrow).abs() < 1e-6,
                    "at {phi:.4} rad the bases read {wide} and {narrow}",
                );
            }
        }
    }

    /// **Why reading the table through the band does not make the band cancel
    /// it**, which is the measurement that took the applied field out of
    /// PR #167 (docs/research/stage9.md 9).
    ///
    /// If the pass applies a table `T` and the band measures the residual
    /// through it, the delivered correction is `T + fit(L - T)` against `fit(L)`
    /// with no table, so by linearity the two differ by exactly **`T - fit(T)`**.
    /// That is zero only if the band's own five-term least squares can
    /// reproduce `T`, and it cannot when the ring it is fitted over is an arc:
    /// the fit is unconstrained where there is no evidence and the ridge pulls
    /// it towards zero there, so the table's own value is delivered whole at
    /// exactly the directions the session never read.
    ///
    /// **The field planted here is not the one section 9.2's table was
    /// measured with**, and the two are reported side by side there. That table
    /// is the real pooled field `--bin table field=` wrote (0.2735 deg rms
    /// composed); this one is a plain five-term field of 0.2163 deg rms, chosen
    /// so the test needs no footage and no scratch file. At 27 directions of
    /// evidence it leaves **0.0856 deg rms and 0.1710 worst** against the real
    /// field's 0.0333 and 0.0696, so it makes the point a fortiori and not
    /// more weakly.
    ///
    /// **The sweep is not monotone in coverage** on either field - here 64 and
    /// 48 directions read 0.0677 and 0.0676 rms while their worst entries go
    /// 0.1403 to 0.1440 - because what is left depends on where the arc sits
    /// against the field's own phase and not only on how wide it is. The
    /// assertions below are therefore about the two ends and not about the
    /// shape between them.
    ///
    /// On real footage `--bin step` reports 27 of 128 directions with evidence.
    #[test]
    fn a_partial_ring_cannot_fit_away_a_table_over_the_whole_of_it() {
        let planted: [f32; AZIMUTHS] = std::array::from_fn(|index| {
            let phi = index as f32 / AZIMUTHS as f32 * std::f32::consts::TAU;
            (0.20 + 0.10 * phi.cos() - 0.06 * (2.0 * phi).sin()).to_radians()
        });
        let left = |covered: usize| {
            let cells: Vec<Cell> = (0..AZIMUTHS)
                .map(|index| Cell {
                    off_epi: -planted[index],
                    off_conf: f32::from(u8::from(index < covered)) * 0.9,
                    ..Cell::default()
                })
                .collect();
            let fitted = Along::fit(&cells);
            (0..AZIMUTHS).fold(0.0f32, |worst, index| {
                let phi = index as f32 / AZIMUTHS as f32 * std::f32::consts::TAU;
                let (sin, cos) = phi.sin_cos();
                worst.max((planted[index] + fitted.at(cos, sin)).abs())
            })
        };
        let whole = left(AZIMUTHS).to_degrees();
        let arc = left(27).to_degrees();
        assert!(
            whole < 0.005,
            "a ring with evidence everywhere left {whole:.4} deg of the table",
        );
        assert!(
            arc > 10.0 * whole,
            "an arc of 27 directions left {arc:.4} deg against {whole:.4} for the whole ring, \
             so this test no longer shows what it is for",
        );
    }

    /// A table is accepted over its whole support or not at all.
    #[test]
    fn a_table_with_an_entry_larger_than_a_calibration_is_refused() {
        let flat = [0.2f32.to_radians(); AZIMUTHS];
        assert!(Table::plausible(flat).is_some());
        let mut wild = flat;
        wild[57] = 0.9f32.to_radians();
        assert_eq!(
            Table::plausible(wild),
            None,
            "0.9 deg at one direction was kept"
        );
        let mut broken = flat;
        broken[3] = f32::NAN;
        assert_eq!(Table::plausible(broken), None);
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
                trust: 0.0,
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
        //
        // The trust comes from `believe` on a reset frame rather than being
        // typed, because since 2026-08-08 the gate the bend reads is the
        // FILTERED one and a fixture that set it by hand would be asserting
        // its own arithmetic. A reset frame is where a full reading is applied
        // whole, and this checks that it is.
        let live: Vec<Cell> = dead
            .iter()
            .map(|cell| {
                let mut cell = Cell {
                    confidence: KEEP,
                    ..*cell
                };
                cell.believe(1.0 / 30.0, true);
                assert_eq!(cell.trust, 1.0);
                cell
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

    /// A floor narrow enough for [`width`]'s adaptive term to have something
    /// to do, which is what the tests below are about.
    ///
    /// It was the shipped crossover until 2026-08-05 and is not one any more:
    /// the projection now asks for 8 degrees and the narrowest camera in the
    /// corpus affords 3.99, both of them over [`WIDEST_DEG`], so on real
    /// footage this function is a constant
    /// (`the_adaptive_width_is_inert_under_the_shipped_floor`). What is tested
    /// here is the function and not the picture, and its floor is an argument.
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
    ///
    /// **At the width the app ships**, which is where that line has to hold,
    /// and which is not where this used to read it. Until 2026-08-08 the fold
    /// limit was `FOLD * band` and this ran at the 2 degree fixture floor
    /// above; the blend curve divides that limit by its own peak gradient
    /// ([`super::projection::BLEND_POWER`]), so a floor of 2 now clamps at
    /// 1.20 degrees and the promise is false there. It is true at 8, by a
    /// margin: the limit is 4.80 degrees against a search that cannot return
    /// more than [`NEAR_DEG`].
    ///
    /// **Where it is NOT true, and the number, because the ONE X2 is a real
    /// camera and not a hypothetical**: at a 3.99 degree support the limit is
    /// 2.394 degrees, so a reading between there and [`NEAR_DEG`] is cut. That
    /// is measured below rather than left to a reader, and it is the price of
    /// not folding that camera's picture
    /// (`projection::tests::the_blend_curve_cannot_fold_the_narrowest_camera`).
    #[test]
    fn the_band_carries_every_disparity_the_search_can_report() {
        let shipped = super::super::projection::CROSSOVER_DEG.to_radians();
        // The search refuses a peak against either edge of its window, so what
        // it can actually hand over is strictly inside [FAR_DEG, NEAR_DEG].
        for step in 0..=200 {
            let degrees = FAR_DEG + (NEAR_DEG - FAR_DEG) * step as f32 / 200.0;
            let radians = degrees.to_radians();
            let carried = carried(radians, width(radians, shipped));
            assert!(
                (carried - radians).abs() < 1e-6,
                "{degrees:.2} deg was cut to {:.2} at the shipped handover",
                carried.to_degrees(),
            );
        }
        // The narrowest support in the corpus, where it stops being true, with
        // the cut printed rather than described.
        let x2 = 3.99f32.to_radians();
        let limit = FOLD * x2 / super::super::projection::BLEND_POWER;
        let near = NEAR_DEG.to_radians();
        assert!(near > limit, "the X2's support no longer clamps the search");
        assert_eq!(carried(near, width(near, x2)).to_bits(), limit.to_bits());
        assert!(
            ((near - limit).to_degrees() - 0.206).abs() < 0.001,
            "the X2 now gives up {:.3} deg at the near end",
            (near - limit).to_degrees(),
        );
        // And stage 2's fixed band gave up more than either, which is what
        // this stage is for.
        let stage2 = carried(2.4f32.to_radians(), FLOOR_DEG.to_radians());
        assert!(
            (2.4f32.to_radians() - stage2).to_degrees() > 0.5,
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

    /// One direction of the calibration fixture's own map, so this module can
    /// ask the shipped pass what it hands over rather than asking its own copy
    /// of a constant.
    fn shipped() -> crate::projection::Reframe {
        use crate::projection::tests::{FRAME, fixture_lenses};
        crate::projection::Reframe::new(
            &fixture_lenses(),
            FRAME,
            crate::Camera::default(),
            crate::projection::Held::default(),
            1.0,
            false,
            crate::sampling::Sampling::default(),
        )
    }

    /// Stage 4 answers the floor and nothing but the floor at the width the
    /// picture ships with, at every disparity the search can report.
    ///
    /// Not a regression: the reason stage 4 opened the band was to carry a
    /// near-field reading without folding, and a floor of 8 degrees carries
    /// every one of them with 4.6 to spare - `carried` clamps at `FOLD * 8`,
    /// 7.2 degrees, and the search cannot report past [`NEAR_DEG`]. What stage
    /// 4 recovered is still recovered; it is the floor doing it now. What is
    /// lost is the other half of stage 4's design, that the band never opens
    /// further than it has to: near-field content is now drawn twice across
    /// the same 8 degrees as everything else, where stage 4 would have given
    /// it at most 2.89.
    #[test]
    fn the_adaptive_width_is_inert_under_the_shipped_floor() {
        let reframe = shipped();
        let floor = reframe.crossover_at(0.0);
        assert!(
            floor > WIDEST_DEG.to_radians(),
            "the floor is not above the widest reading"
        );
        for step in 0..=200 {
            let degrees = FAR_DEG + (NEAR_DEG - FAR_DEG) * step as f32 / 200.0;
            let opened = reframe.crossover_at(degrees.to_radians());
            assert_eq!(
                opened.to_bits(),
                floor.to_bits(),
                "{degrees:.2} deg opened the band to {:.4} deg",
                opened.to_degrees(),
            );
            // And the clamp is inert with it: nothing the search can report is
            // cut, which is the property stage 4 shipped.
            let radians = degrees.to_radians();
            assert!((carried(radians, opened) - radians).abs() < 1e-9);
        }
    }

    /// A camera whose lenses do not overlap enough for the width the picture
    /// asks for gets the width it can pay for, and stage 4 with it.
    ///
    /// The ONE X2 is that camera: 9.19 degrees of overlap, which affords 3.99
    /// against the 8 asked for. The X4 Air files are the other end and they
    /// are **not one number**: the overlap is read off each file's own
    /// calibration and the corpus spreads over half a degree of it, so every
    /// row here names the file it was measured on rather than quoting a family
    /// (`kjerag-spike --bin band`, the owner's own captures, 2026-08-05).
    #[test]
    fn the_narrow_overlap_camera_gets_the_width_it_can_pay() {
        for (file, overlap, affords) in [
            ("VID_20251018_191318_00_002 (ONE X2)", 9.19f32, 3.99f32),
            ("VID_20260501_183417_00_002", 14.56, 9.36),
            ("VID_20260725_194424_00_002", 14.60, 9.40),
            ("VID_20260802_191029_00_002", 14.61, 9.41),
            ("VID_20260526_191025_00_004", 14.68, 9.48),
            ("VID_20260714_193252_00_006", 14.89, 9.69),
            ("VID_20260725_194424_00_001", 15.02, 9.82),
        ] {
            let width = affordable(overlap.to_radians()).to_degrees();
            assert!(
                (width - affords).abs() < 0.005,
                "{file} overlaps by {overlap} deg, which affords {width:.2} and not {affords}",
            );
        }
        // And what it affords is what fits, at any overlap either regime of
        // `reach` can be in, the seam between them included.
        for step in 0..=400 {
            let overlap = (1.0 + step as f32 * 0.05).to_radians();
            let width = affordable(overlap);
            assert!(width > 0.0, "{overlap:?} affords nothing");
            assert!(
                reach(width) <= 0.5 * overlap + 1e-6,
                "an overlap of {:.2} deg affords {:.2}, which reaches {:.2} into {:.2} a side",
                overlap.to_degrees(),
                width.to_degrees(),
                reach(width).to_degrees(),
                0.5 * overlap.to_degrees(),
            );
        }
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
            trust: 1.0,
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
            let (read, evidence) = pooled_gain(&ring).expect("a ring that correlated");
            assert!(
                (read - gain.ln()).abs() < 1e-5,
                "a ring at gain {gain} read back {}",
                read.exp(),
            );
            assert!((evidence - 1.0).abs() < 1e-5, "evidence {evidence}");
        }
    }

    #[test]
    fn nothing_measured_is_no_correction_and_the_picture_is_the_one_before() {
        // Every path that reaches an untouched picture, and the equality that
        // makes it byte-identical rather than nearly so. A multiply by
        // exactly 1.0 is exact in IEEE; a multiply by whatever `exp(0.0)`
        // happens to return on a driver is not, which is why `split` answers
        // with a match and not with an exponential.
        assert!(pooled_gain(&[]).is_none());
        let dark: Vec<Cell> = (0..AZIMUTHS)
            .map(|_| Cell {
                confidence: 0.0,
                ..lit_cell(0.02, 0.5, 1.05)
            })
            .collect();
        assert!(pooled_gain(&dark).is_none());
        assert_eq!(Tone::default().split(), [1.0, 1.0]);
        assert_eq!(Tone::read(0.0, 0.9).split(), [1.0, 1.0]);
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
        assert!(pooled_gain(&near).is_none());
        // And a ring of both reports the far field's answer alone, not an
        // average of the two.
        let mixed: Vec<Cell> = (0..AZIMUTHS)
            .map(|index| match index % 2 {
                0 => lit_cell(0.02, 0.5, 1.02),
                _ => lit_cell(1.0, 0.5, 0.50),
            })
            .collect();
        let (read, _) = pooled_gain(&mixed).expect("half the ring is far field");
        assert!((read - 1.02f32.ln()).abs() < 1e-5, "read {}", read.exp());
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
        let (read, _) = pooled_gain(&ring).expect("a ring that correlated");
        // 0.81 of weight against 127 * 0.0025, which is 72 percent of the way
        // from the many to the one. An equal-weight pooling would have landed
        // at 0.9008, which is the number this has to beat.
        assert!(
            (read.exp() - 0.9718).abs() < 1e-3,
            "one bright direction left the answer at {}",
            read.exp(),
        );
    }

    #[test]
    fn a_runaway_reading_cannot_wash_a_hemisphere_out() {
        // The guard, and what it bounds the damage to. A ring correlating on
        // content that is not the same content at all cannot move either
        // hemisphere by more than an eighth, whatever it reads.
        let broken: Vec<Cell> = (0..AZIMUTHS).map(|_| lit_cell(0.02, 0.5, 4.0)).collect();
        let (read, _) = pooled_gain(&broken).expect("a ring that correlated");
        assert_eq!(read, LIMIT_LN);
        let split = Tone::read(read, 1.0).split();
        assert!(
            split[0] < 1.14 && split[1] > 0.88,
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
            let split = Tone::read(gain.ln().clamp(-LIMIT_LN, LIMIT_LN), 1.0).split();
            let lens0 = 100.0 * split[0];
            let lens1 = 100.0 * gain.clamp(-LIMIT_LN.exp(), LIMIT_LN.exp()) * split[1];
            assert!(
                (lens0 - lens1).abs() < 1e-3,
                "at gain {gain} the two sides land at {lens0} and {lens1}",
            );
            // Symmetric: the two multipliers are reciprocals, so the picture's
            // own mean brightness is left where it was.
            assert!((split[0] * split[1] - 1.0).abs() < 1e-6);
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
        // Plus stage 3's own: three per lane for the photometry the winning
        // shift is read at, and three more for the pooling.
        let bytes = 4 * (patch + back as usize + shifts + 6 * THREADS);
        assert!(
            bytes <= 16352,
            "the workgroup wants {bytes} bytes of shared memory",
        );
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
            trust: 1.0,
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

    // ------------------------------------------------- the along-seam table

    /// Readings at every azimuth, all saying the same thing.
    fn round_the_ring(perp_deg: f32, cycles: f32, count: usize) -> Vec<Leftover> {
        (0..count)
            .map(|index| {
                let phi = index as f32 / count as f32 * std::f32::consts::TAU;
                Leftover {
                    phi,
                    perp: perp_deg.to_radians() * (cycles * phi).cos(),
                    weight: 1.0,
                }
            })
            .collect()
    }

    #[test]
    fn a_table_with_nothing_in_it_moves_no_ray_at_all() {
        // Rung 1 of stage 9's ladder, at the one place a test can reach it:
        // an empty table is not a small correction, it is the same arithmetic
        // the pass ran before it existed. The rendered half is measured
        // against `origin/main` and reported in docs/research/stage9.md.
        use crate::projection::tests::{FRAME, fixture_lenses};
        let reframe = crate::seam::mapped(&fixture_lenses(), FRAME);
        assert!(reframe.table().is_rest());
        assert_eq!(reframe.table(), Table::default());
        for ray in [[0.6, 0.1, 0.8], [-0.2, 0.9, 0.1], [0.0, 0.0, 1.0]] {
            assert_eq!(reframe.tabled(0, ray), ray);
            assert_eq!(reframe.tabled(1, ray), ray);
            let bend = reframe.bend(ray, Reading::default());
            assert_eq!(bend.along, [0.0; 3]);
        }
    }

    #[test]
    fn a_planted_ripple_comes_back_at_every_direction_it_was_planted_at() {
        // The table has to reproduce what it was given wherever the evidence
        // is dense and even, which is the control the corpus measurement is
        // read against: a field the fit cannot return is a fit whose refusals
        // cannot be believed either.
        let planted = 0.10f32;
        let left = round_the_ring(planted, 6.0, 360);
        let table = Table::of(&left, SMOOTH_DEG);
        for index in 0..AZIMUTHS {
            let phi = index as f32 / AZIMUTHS as f32 * std::f32::consts::TAU;
            let want = planted.to_radians() * (6.0 * phi).cos();
            let got = table.at(phi.cos(), phi.sin());
            // Smoothed by a kernel wider than a sixth of the circle, so what
            // comes back is the ripple attenuated rather than the ripple: the
            // check is that it is the same shape, in phase, and large.
            assert!(
                (got / want).is_finite() && got * want > 0.0 || want.abs() < 1e-6,
                "direction {index}: planted {want}, read {got}",
            );
        }
        let amplitude = table.entries().iter().fold(0.0f32, |m, e| m.max(e.abs()));
        assert!(
            amplitude > 0.3 * planted.to_radians(),
            "a 12 degree kernel kept {amplitude} of a 6 cycle ripple",
        );
    }

    #[test]
    fn twice_the_evidence_asks_for_twice_the_correction() {
        // Linear in what it is given, which is what makes a planted control
        // readable at two amplitudes.
        let one = Table::of(&round_the_ring(0.05, 6.0, 360), SMOOTH_DEG);
        let two = Table::of(&round_the_ring(0.10, 6.0, 360), SMOOTH_DEG);
        for (a, b) in one.entries().iter().zip(two.entries()) {
            assert!((2.0 * a - b).abs() < 1e-6, "{a} doubled is not {b}");
        }
    }

    #[test]
    fn the_five_terms_the_pass_already_applies_are_taken_back_out() {
        // Otherwise the band's own field and this one would both correct the
        // low orders and the picture would be turned by however much the two
        // of them agreed.
        for cycles in [0.0, 1.0, 2.0] {
            let planted = 0.20f32.to_radians();
            let table = Table::of(&round_the_ring(0.20, cycles, 360), SMOOTH_DEG);
            let worst = table.entries().iter().fold(0.0f32, |m, e| m.max(e.abs()));
            // Not exactly nothing: the levelling fit is shrunk by [`RIDGE`]
            // like every other fit in this file. A cycle term's own diagonal
            // is half the reading count, so 360 readings give up about 2/360
            // of it, which is the 0.5 percent measured here. That is the
            // ridge and not a leak, and it is under a hundredth of a source
            // pixel at the size a calibration comes in.
            assert!(
                worst < 0.01 * planted,
                "{cycles} cycles is a pose and left {worst} rad in the table",
            );
        }
    }

    #[test]
    fn an_azimuth_no_reading_reached_is_left_exactly_alone() {
        // The refusal stage 9 is built on: a table speaks for the directions
        // it was measured at and tapers to exactly zero everywhere else, so a
        // starved ring cannot warp a hemisphere the way a harmonic would.
        let left: Vec<Leftover> = round_the_ring(0.20, 0.0, 360)
            .into_iter()
            .filter(|l| l.phi.to_degrees() < 90.0)
            .collect();
        let entries = table_entries(&left);
        let far = (AZIMUTHS as f32 * 200.0 / 360.0) as usize;
        assert_eq!(entries[far], 0.0, "an unmeasured direction was moved");
        let evidence = Table::evidence(&left, SMOOTH_DEG);
        assert_eq!(evidence[far], 0.0);
        assert!(
            evidence[AZIMUTHS / 8] > 1.0,
            "the measured arc has evidence"
        );
    }

    #[test]
    fn the_edge_of_the_evidence_is_a_taper_and_not_a_cliff() {
        // A top hat would put a corner in the picture wherever a reading
        // walked in or out of one direction's window, which is the stage 5
        // scallop and the stage 7 stripe in a third costume. So the entries
        // beside the zeros have to be small: what the field does inside its
        // own support is the field's business, and what it does at the edge of
        // it is this stage's.
        let left: Vec<Leftover> = round_the_ring(0.20, 6.0, 720)
            .into_iter()
            .filter(|l| l.phi.to_degrees() < 120.0)
            .collect();
        let entries = table_entries(&left);
        let largest = entries.iter().fold(0.0f32, |m, e| m.max(e.abs()));
        assert!(largest > 0.0, "the measured arc carries nothing");
        for index in 0..AZIMUTHS {
            let (here, next) = (entries[index], entries[(index + 1) % AZIMUTHS]);
            if here != 0.0 && next != 0.0 {
                continue;
            }
            assert!(
                here.abs().max(next.abs()) < 0.2 * largest,
                "direction {index} steps from {here} to {next} against {largest}",
            );
        }
    }

    fn table_entries(left: &[Leftover]) -> [f32; AZIMUTHS] {
        Table::of(left, SMOOTH_DEG).entries()
    }

    #[test]
    fn a_reading_larger_than_a_calibration_is_refused_rather_than_applied() {
        let left = round_the_ring(4.0, 6.0, 360);
        for entry in Table::of(&left, SMOOTH_DEG).entries() {
            assert!(
                entry.abs() <= TABLE_LIMIT_RAD,
                "{entry} rad is not a camera"
            );
        }
    }

    #[test]
    fn the_lookup_wraps_and_lands_between_its_neighbours() {
        let mut entries = [0.0f32; AZIMUTHS];
        entries[0] = 1.0;
        entries[AZIMUTHS - 1] = -1.0;
        let table = Table::of_entries(entries);
        assert_eq!(table.between(0, 0.0), 1.0);
        assert_eq!(table.between(-1, 0.0), -1.0);
        assert_eq!(table.between(AZIMUTHS as i32, 0.0), 1.0);
        assert!((table.between(-1, 0.5) - 0.0).abs() < 1e-6);
        let phi = std::f32::consts::TAU / AZIMUTHS as f32 * (AZIMUTHS - 1) as f32;
        assert!((table.at(phi.cos(), phi.sin()) + 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_table_survives_being_written_down_and_read_back() {
        let table = Table::of(&round_the_ring(0.10, 6.0, 360), SMOOTH_DEG);
        assert_eq!(Table::read(&table.write()), Some(table));
        assert_eq!(
            Table::read("1.0\n2.0\n"),
            None,
            "a short table is not a table"
        );
        assert_eq!(Table::read(&"nan\n".repeat(AZIMUTHS)), None);
    }

    #[test]
    fn only_lens_one_takes_the_table_and_it_takes_it_whole() {
        // The convention the calibration it belongs to already uses: the seam
        // cannot say which lens is wrong, so one of them is turned and the
        // other is left exactly alone (`SeamFit::applied`).
        use crate::projection::tests::{FRAME, fixture_lenses};
        let table = Table::of(&round_the_ring(0.10, 6.0, 360), SMOOTH_DEG);
        let reframe = crate::seam::mapped(&fixture_lenses(), FRAME).with_table(table);
        let ray = [0.6, 0.1, 0.0];
        assert_eq!(reframe.tabled(0, ray), ray);
        assert_ne!(reframe.tabled(1, ray), ray);
    }

    #[test]
    fn the_table_goes_to_nothing_at_the_poles_where_an_azimuth_does_not_exist() {
        // Same argument and the same factor as the band's own along-seam
        // field: `w x d` is `|w| cos(elevation)` along the seam's tangent, so
        // a per-azimuth correction cannot swirl where there is no azimuth.
        use crate::projection::tests::{FRAME, fixture_lenses};
        let table = Table::of(&round_the_ring(0.10, 6.0, 360), SMOOTH_DEG);
        let reframe = crate::seam::mapped(&fixture_lenses(), FRAME).with_table(table);
        let moved = |ray: [f32; 3]| {
            let out = reframe.tabled(1, ray);
            norm(std::array::from_fn(|axis| out[axis] - ray[axis]))
        };
        let seam = moved([1.0, 0.0, 0.0]);
        assert!(moved([0.1, 0.0, 1.0]) < 0.2 * seam, "the pole still moves");
        assert_eq!(reframe.tabled(1, [0.0, 0.0, 1.0]), [0.0, 0.0, 1.0]);
    }
}

#[cfg(test)]
mod trust_tests {
    use super::*;

    /// The owner's May-01 downward arc, in the units the diagnosis quoted it
    /// in: a held disparity of -0.912 degrees at 51.2 view px per degree, so
    /// the whole correction is 46.7 view px (docs/research/seam-temporal.md
    /// 2.2).
    const HIS_ARC_DEG: f32 = -0.912;
    const VIEW_PX_PER_DEG: f32 = 51.2;

    /// A direction is visited every [`SLICES`] frames, so this is what one
    /// step of the filter spans at 30 fps - the same 2/30 the diagnosis
    /// recovered its readings through.
    const VISIT_S: f32 = SLICES as f32 / 30.0;

    /// **The law this PR replaced**, written out here rather than left behind
    /// a branch in the pass.
    ///
    /// It is the positive control for everything below, and it has to live
    /// somewhere: the shipped code no longer contains it, and a claim that the
    /// filter fixes a defect is worth nothing without the defect. `main` at
    /// a7b6930 computed exactly this expression per fragment, every frame.
    fn as_main_did(cell: &mut Cell, _seconds: f32, _fresh: bool) {
        cell.trust = (cell.confidence / KEEP).clamp(0.0, 1.0);
    }

    /// And the law that ships: [`Cell::believe`], with the same signature so
    /// the two are interchangeable in [`plant`].
    fn as_it_ships(cell: &mut Cell, seconds: f32, fresh: bool) {
        cell.believe(seconds, fresh);
    }

    /// What the applied correction does over a planted confidence flicker, in
    /// view px per visit: every step, and the state underneath at the end.
    ///
    /// `readings` is what the correlator returns on each visit, or `None` for
    /// a visit the gates refused, which is the shape the trace on his arc has:
    /// the content flickers the correlation across [`KEEP`] and the reading
    /// itself never moves.
    fn plant(readings: &[Option<f32>], gate: impl Fn(&mut Cell, f32, bool)) -> (Vec<f32>, Cell) {
        let mut cell = Cell {
            disparity: HIS_ARC_DEG.to_radians(),
            confidence: 0.0,
            reach_m: 0.033,
            ..Cell::default()
        };
        let mut applied = Vec::new();
        let mut last = 0.0;
        for (visit, reading) in readings.iter().enumerate() {
            match reading {
                // The shipped law, both branches, out of `settle` and
                // `forget`: a refused visit gives up the evidence and keeps
                // the measurement; a read one eases both, and takes its first
                // whole.
                Some(best) => {
                    let learn = match cell.confidence <= 0.0 {
                        true => 1.0,
                        false => ease(VISIT_S, time_constant(cell.disparity)),
                    };
                    cell.disparity += (HIS_ARC_DEG.to_radians() - cell.disparity) * learn;
                    cell.confidence += (best - cell.confidence) * learn;
                }
                None => {
                    cell.confidence -=
                        cell.confidence * ease(VISIT_S, time_constant(cell.disparity));
                }
            }
            gate(&mut cell, VISIT_S, visit == 0);
            let now = cell.disparity.to_degrees() * cell.trust * VIEW_PX_PER_DEG;
            applied.push(now - last);
            last = now;
        }
        (applied, cell)
    }

    /// The plant itself: eight visits correlating well, eight the gates
    /// refuse, eight correlating again. Nothing about the disparity moves.
    fn flicker() -> Vec<Option<f32>> {
        let mut readings = vec![Some(0.90); 8];
        readings.extend(vec![None; 8]);
        readings.extend(vec![Some(0.90); 8]);
        readings
    }

    /// The OTHER plant: a direction that is sky for four visits and then
    /// starts correlating, in the middle of a shot rather than on a reset
    /// frame.
    ///
    /// **This is the shape of the largest step the band delivers.** At
    /// `down1` the cell the owner is looking through first correlates on
    /// frame 70 of 120 and at `down3` on frame 26, and each was one delivered
    /// step of about 56 view px on every arm of the 2026-08-08 A/B.
    /// [`flicker`] cannot show it: its first visit is a reset frame, where
    /// every build applies whole by design and always will.
    fn late_arrival(visits: usize) -> Vec<Option<f32>> {
        let mut readings = vec![None; 4];
        readings.extend(vec![Some(0.90); visits]);
        readings
    }

    /// How much of a correction one visit of the filter may move, as a
    /// fraction: the walk rate an arrival is bounded by, and it is
    /// [`TAU_TRUST_S`] and the visit interval and nothing else.
    fn walk_rate() -> f32 {
        ease(VISIT_S, TAU_TRUST_S)
    }

    /// **The positive control, and it runs first.** With the gate `main`
    /// applied, a planted confidence dropout snaps the correction off and back
    /// on: the defect the memo diagnosed, reproduced with no GPU and no
    /// footage.
    ///
    /// If this stops failing the way the trace failed, the test below has
    /// stopped being evidence of anything.
    ///
    /// Measured, in view px per visit after the arrival: **+7.9, +15.5, +9.3,
    /// +5.6, +3.4, +2.0, +1.2, +0.7** going out and **-25.4, -15.3, -4.9**
    /// coming back, three of them over 10.
    #[test]
    fn the_gate_main_shipped_snaps_a_planted_flicker() {
        let (steps, cell) = plant(&flicker(), as_main_did);
        let worst = steps
            .iter()
            .fold(0.0f32, |worst, step| worst.max(step.abs()));
        assert!(
            worst > 10.0,
            "the planted flicker moves the picture by at most {worst} view px on main's gate, \
             so it is not the defect the trace measured"
        );
        // And the state underneath never moved, which is what makes it a gate
        // artifact rather than a measurement.
        near(cell.disparity.to_degrees(), HIS_ARC_DEG, 1e-4);
    }

    /// And with the gate filtered, the same plant moves the picture by under a
    /// view pixel a visit while the state underneath still tracks.
    ///
    /// The bar is the memo's own statistic: 84 steps over **10 view px** in
    /// four seconds is what the trace on his arc reported, so a step over 10
    /// is the defect. This plant measures **1.27**, and the bar is set at 2 to
    /// leave the number room to be a number rather than a threshold.
    ///
    /// Measured, in view px per visit after the arrival, against the control
    /// above: **+0.25, +0.75, +1.02, +1.17, +1.24, +1.27, +1.26, +1.25** going
    /// out and **+0.39, -0.12, -0.27, -0.26** coming back. **None over 10.**
    #[test]
    fn the_filtered_gate_fades_a_planted_flicker() {
        let (steps, cell) = plant(&flicker(), as_it_ships);
        let worst = steps
            .iter()
            .skip(1)
            .fold(0.0f32, |worst, step| worst.max(step.abs()));
        assert!(
            worst < 2.0,
            "the planted flicker still moves the picture by {worst} view px in one visit"
        );
        near(cell.disparity.to_degrees(), HIS_ARC_DEG, 1e-4);
        // The first visit is the one that takes its answer whole, on both
        // laws: it is a RESET frame, and a reset frame has no picture behind
        // it for an ease to hide.
        near(steps[0], HIS_ARC_DEG * VIEW_PX_PER_DEG, 1e-2);
    }

    /// During the dropout the applied correction only ever fades **towards the
    /// reading it holds**, one direction, never off it and back.
    #[test]
    fn a_filtered_gate_decays_monotonically_while_the_evidence_goes() {
        let (steps, _) = plant(&flicker(), as_it_ships);
        for (visit, step) in steps.iter().enumerate().skip(9).take(7) {
            assert!(
                *step >= 0.0,
                "visit {visit} of the dropout moves the correction {step} view px, which is \
                 away from the held reading and not towards it"
            );
        }
    }

    /// The state underneath keeps measuring through all of it: a filtered gate
    /// is a filter on what is applied and not on what is read.
    #[test]
    fn the_filtered_gate_does_not_stop_the_band_measuring() {
        // A direction whose disparity really does change, with the evidence
        // steady: the wing crossing rather than the correlator blinking.
        let mut cell = Cell {
            disparity: HIS_ARC_DEG.to_radians(),
            confidence: 0.90,
            reach_m: 0.033,
            trust: 1.0,
            ..Cell::default()
        };
        let target = 0.2f32.to_radians();
        for _ in 0..30 {
            let learn = ease(VISIT_S, time_constant(cell.disparity));
            cell.disparity += (target - cell.disparity) * learn;
            cell.believe(VISIT_S, false);
        }
        assert!(
            (cell.disparity - target).abs() < 0.1 * (HIS_ARC_DEG.to_radians() - target).abs(),
            "the state stopped tracking at {} degrees",
            cell.disparity.to_degrees()
        );
        near(cell.trust, 1.0, 1e-3);
    }

    /// **The positive control for the staging, and it runs first.** With the
    /// gate `main` applied, a direction that starts correlating mid-shot
    /// applies its whole correction on one visit.
    ///
    /// This is the 56 px step at `down1` and `down3` reproduced with no GPU
    /// and no footage. If it stops failing this way the test below has stopped
    /// being evidence of anything.
    ///
    /// Measured: **-46.69 view px on the arriving visit**.
    #[test]
    fn the_gate_main_shipped_applies_a_late_arrival_whole() {
        let (steps, _) = plant(&late_arrival(4), as_main_did);
        near(steps[4], HIS_ARC_DEG * VIEW_PX_PER_DEG, 1e-2);
    }

    /// And as it ships, the same arrival reaches the picture at the walk rate:
    /// no visit of the whole plant moves it further than the filter's own
    /// step, which is [`TAU_TRUST_S`] and the visit interval and nothing else.
    ///
    /// The bound is computed rather than typed, so it cannot be a threshold
    /// tuned on the answer: it is `full * ease(VISIT_S, TAU_TRUST_S)`, which
    /// is **1.53 view px** against the control's 46.69.
    #[test]
    fn a_staged_arrival_walks_in_at_the_filter_rate() {
        let (steps, _) = plant(&late_arrival(4), as_it_ships);
        let bound = (HIS_ARC_DEG * VIEW_PX_PER_DEG).abs() * walk_rate();
        let worst = steps
            .iter()
            .fold(0.0f32, |worst, step| worst.max(step.abs()));
        assert!(
            worst <= bound + 1e-3,
            "the staged arrival moves the picture {worst} view px in one visit, past the \
             {bound} the walk rate allows"
        );
    }

    /// Staging is a delay and not an attenuation: the correction still gets
    /// all the way to the reading, on the constant it declares.
    ///
    /// One [`TAU_TRUST_S`] is 30 visits and must deliver over half of it;
    /// three of them over nine tenths. A walk that stopped short would be the
    /// stage 6 defect this is closest to and the thing to catch.
    ///
    /// The plain bars are there to be read; the line under them is the exact
    /// one, and it says the walk is the filter's own geometry and not
    /// something near it. [`ease`] is `dt / (tau + dt)`, so a visit leaves
    /// `tau / (tau + dt)` of the gap and n of them leave that to the n-th:
    /// **62.6 percent delivered after one time constant and 94.8 after
    /// three**, which is what those two bars are under.
    #[test]
    fn a_staged_arrival_still_gets_all_the_way_there() {
        let full = HIS_ARC_DEG * VIEW_PX_PER_DEG;
        for (visits, share) in [(30, 0.5), (90, 0.9)] {
            let (steps, cell) = plant(&late_arrival(visits), as_it_ships);
            let applied: f32 = steps.iter().sum();
            assert!(
                applied / full >= share,
                "after {visits} visits the staged arrival has delivered {applied} view px of \
                 {full}, which is short of the {share} its own time constant promises"
            );
            near(
                applied / full,
                1.0 - (1.0 - walk_rate()).powi(visits as i32),
                1e-4,
            );
            // And what it walked in on is the whole reading, not a filtered
            // one: the state took its answer whole as it always did.
            near(cell.disparity.to_degrees(), HIS_ARC_DEG, 1e-4);
        }
    }

    /// A **seek** takes the arrival whole, and that is BY DESIGN.
    ///
    /// Stated as a test because it is the one behaviour of this change a
    /// reader could reasonably assume the other way: a walk after a cut would
    /// draw the first two seconds of every seek with nearly no correction, and
    /// there is no picture behind a cut for the walk to be continuous with
    /// anyway (issue #103, stage 6).
    #[test]
    fn a_seek_applies_the_arrival_whole() {
        let mut cell = Cell {
            disparity: HIS_ARC_DEG.to_radians(),
            confidence: 0.90,
            reach_m: 0.033,
            ..Cell::default()
        };
        cell.believe(VISIT_S, true);
        near(cell.trust, 1.0, 1e-6);
    }

    /// A direction that stops correlating for long enough to lose its trust
    /// entirely **re-arrives**, and it is the same line and the same walk.
    ///
    /// `main`'s rule was `trust <= 0`, not "has never been read", so a cell
    /// whose evidence had gone all the way to nothing applied its next reading
    /// whole however long it had been on screen. The staging covers both
    /// because both were that one clause, and this is the half a reader is
    /// least likely to notice.
    #[test]
    fn a_re_arrival_is_staged_like_a_first_one() {
        let cold = || Cell {
            disparity: HIS_ARC_DEG.to_radians(),
            confidence: 0.90,
            reach_m: 0.033,
            trust: 0.0,
            ..Cell::default()
        };
        let mut was = cold();
        as_main_did(&mut was, VISIT_S, false);
        near(was.trust, 1.0, 1e-6);
        let mut now = cold();
        now.believe(VISIT_S, false);
        near(now.trust, walk_rate(), 1e-6);
    }

    /// An old trace, written before the eighth column existed, reads back
    /// applying what the build that wrote it applied - not zero, which would
    /// be a picture with no band in it.
    #[test]
    fn a_seven_column_trace_reads_back_the_gate_it_was_written_under() {
        let mut cell = Cell {
            disparity: 0.01,
            confidence: 0.5 * KEEP,
            reach_m: 0.033,
            trust: 0.75,
            ..Cell::default()
        };
        let eight = Cell::read(&Cell::write(std::slice::from_ref(&cell))).expect("eight columns");
        assert_eq!(eight[0], cell);
        let seven: String = Cell::write(std::slice::from_ref(&cell))
            .split_whitespace()
            .take(7)
            .collect::<Vec<_>>()
            .join(" ");
        cell.trust = 0.5;
        assert_eq!(Cell::read(&seven).expect("seven columns")[0], cell);
    }

    fn near(read: f32, want: f32, tolerance: f32) {
        assert!(
            (read - want).abs() <= tolerance,
            "{read} is not within {tolerance} of {want}"
        );
    }
}
