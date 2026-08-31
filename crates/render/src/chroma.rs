//! Studio's chromatic calibration, as its own grid rather than as a ring.
//!
//! [`super::chromatic`] recovered the SOLVER - the penalty, the tolerance, the
//! budget, the metric's integer filter. This module is the PROBLEM that solver
//! is pointed at, which is the half three rebuilds of this arm did not have
//! (docs/research/chromatic.md P). Every constant here is address-cited in
//! docs/research/studio-seam-re.md.
//!
//! # The geometry, which is what unlocks the rest
//!
//! The fusion map is `212 x 100` for the selected export, and `0x1828f3f6a`
//! builds its directions with **latitude divided by `rows - 1`, so both poles
//! are included** (ledger 1138-1145). So the map is pole to pole, a row is
//! exactly `180 / 99` degrees, and the seam is the equator at row 49.5.
//!
//! That converts every row count in the ledger into an angle, and until it was
//! done this arm was reading its extent off a JPEG and rounding.

use std::sync::OnceLock;

/// Columns of the selected fusion map: `trunc(200 * 1.0600299835)`, the width
/// constant `0x3f87af10` at `0x185621ea8` applied at
/// `0x183c01614..0x183c01625`.
pub const COLUMNS: usize = 212;

/// Rows of the map, before any crop. Height stays 100 at `0x183c01645`.
pub const MAP_ROWS: usize = 100;

/// The active input ROI is `{x=0, y=40, width=212, height=20}`
/// (`0x183c1de55..0x183c1dead`), so the window starts at map row 40 and is 20
/// rows tall.
pub const ROI_TOP: usize = 40;
/// How many rows of the map the fusion window spans.
pub const WINDOW_ROWS: usize = 20;

/// Each lens's own block within that window. `0x183c1df22..0x183c1e094` fills
/// mask zero on rows `[0,12)` and mask one on rows `[8,20)`.
pub const BLOCK_ROWS: usize = 12;
/// Where the second lens's block starts inside the window, which is what makes
/// the two overlap on exactly rows `[8,12)`.
pub const LENS_STRIDE: usize = 8;

/// `2 * 12 * 212`, the count helper `+0x38` holds. The two lenses keep
/// SEPARATE nodes even where their blocks overlap (`0x183c1a620` assigns every
/// nonzero mask-zero pixel before every nonzero mask-one pixel), which is why
/// this is twice a block and not the union of two.
pub const NODES: usize = 2 * BLOCK_ROWS * COLUMNS;

/// The evidence rows, in window-local coordinates: whichever rows of the
/// window the two lenses are actually MIXED over, centred on the seam.
///
/// **Derived rather than fixed - but it does NOT reproduce Studio's answer,
/// and the claim that it did was wrong twice over.** Studio reads two rows,
/// `{49,50}`, which is `+-0.909` degrees. This rule returns:
///
/// ```text
/// a hard handover (a point)          4 rows
/// alphaNormalized, 8 or 10 degrees   4 rows
/// linear, 8 degrees  (the old ask)   6 rows
/// linear, 6 degrees  (SHIPS TODAY)   4 rows
/// ```
///
/// **The shipped row is the last one, and it changed on 2026-08-13 without
/// anyone noticing.** Moving `CROSSOVER_DEG` from 8 to Studio's 6
/// (`studio-seam-re.md` 19.3) took the mixed half-width from 3.60 to 2.70
/// degrees and `k` from 2 to 1, so this returns FOUR rows where every
/// paragraph below, and three documents, still said six. No test asserted the
/// count, which is why it went green;
/// `the_evidence_rows_are_the_four_the_shipped_handover_reaches` now pins it.
///
/// It never returns two. The floor `k >= (BLOCK_ROWS - LENS_STRIDE) / 2 - 1`
/// puts the minimum at four whatever the blend is, so "Studio's handover is a
/// point, so the rule returns Studio's own answer" was arithmetically false
/// with these constants on the day it was written - before
/// `studio-seam-re.md` section 9 also took away the premise that Studio's
/// curve is `alphaNormalized` at all.
///
/// So D1 is a real deviation and this is our rule, not Studio's. It is kept
/// because spanning our own linear blend is what the residual measurements
/// support (docs/seam-parity-plan.md 2), not because it derives Studio's two.
/// Those measurements were taken at six rows over an eight-degree blend and
/// have NOT been retaken at the four rows a six-degree blend gives.
///
/// Ours has to answer the same question against a different blend. The rule is
/// "the evidence spans the region where both lenses are shown", and it returns
/// Studio's answer when given Studio's blend:
///
/// | blend | mixes over | rows this returns |
/// | --- | ---: | ---: |
/// | linear, 6 degrees (ships) | `+-2.70` | 4 |
/// | linear, 8 degrees (the old ask) | `+-3.60` | 6 |
/// | Studio's curve, 8 degrees | `+-1.43` | 4 |
/// | Studio's curve, 10 degrees | `+-1.79` | 4 |
///
/// So enabling `KJERAG_BLEND_CURVE=studio` narrows the evidence by itself, and
/// the extra rows this reads over Studio's two exist only because our fade is
/// linear. That is the whole of the deviation recorded as D1 in
/// docs/seam-parity-plan.md 2: it is the linear ramp's cost, not a
/// disagreement with Studio.
///
/// Clamped to the window, and always at least the four rows the two blocks
/// share, because a row outside the overlap constrains a lens through the edge
/// its block is held at (`grid_at`) rather than through a node of its own.
pub fn evidence_rows() -> std::ops::Range<usize> {
    // The seam is the equator, which sits BETWEEN window rows 9 and 10 - map
    // row 49.5. So the rows either side of it are the innermost pair, at
    // `+-0.909` degrees, and a symmetric set is that pair widened by `k` rows
    // each way. Centring on a single row cannot be symmetric and reads
    // `+2.727` against `-4.545`, which is what the test caught.
    let inner = (WINDOW_ROWS - 1) as f64 / 2.0 - 0.5;
    let (below, above) = (inner as usize, inner as usize + 1);
    let step = degrees_per_row();
    let half = f64::from(super::projection::mixing_half_width().to_degrees());
    // How far past the innermost pair the mixing reaches, in whole rows.
    let k = (((half - step / 2.0) / step).ceil().max(0.0)) as usize;
    // Never narrower than the rows the two blocks share: inside those, both
    // lenses answer through nodes of their own rather than through a held
    // edge, and that is the evidence worth having.
    let k = k.max((BLOCK_ROWS - LENS_STRIDE) / 2 - 1);
    below.saturating_sub(k)..(above + k + 1).min(WINDOW_ROWS)
}

/// Research only: refuse a chromatic evidence cell whose two lenses disagree
/// GEOMETRICALLY, from `KJERAG_CHROMA_DISPARITY=<codes>`.
///
/// **Zero, which is every shipped run, is no gate at all** - the behaviour
/// `chroma_read` argues for and the behaviour that ships. Set to a number of
/// codes and a cell is dropped when the per-tap difference between the two
/// lenses varies across the cell by more than that.
///
/// **Why the argument for having no gate is conditional.** `chroma_read` says
/// it outright: *"two lenses looking at one direction either see the same
/// thing or differ by the thing being corrected"*. That holds at far field. It
/// fails at NEAR field, where the two lenses at one world direction are
/// looking at different objects, and the difference the arm reads is parallax
/// rather than colour. Studio is entitled to the premise because its Optical
/// Flow tier aligns near content BEFORE its chromatic pass samples; this
/// renderer copied the absence of the gate without the precondition that makes
/// it safe (docs/seam-parity-plan.md 8).
///
/// **The statistic, and why this one.** A genuine colour difference is a
/// smooth offset: over a cell's `CELL_SAMPLES` squared taps the per-tap
/// difference is nearly CONSTANT. Parallax is not - the taps land on different
/// objects and the difference scatters. So the spread of the difference within
/// a cell separates the two without a search, without a second pass, and
/// without any sample the read does not already take.
///
/// **A knob and not a constant, deliberately.** Every threshold in this arm is
/// address-cited to Studio; this one cannot be, because Studio has no such
/// gate to cite. Under the no-invented-constants rule that leaves a disclosed
/// live knob, defaulting to the shipped behaviour, until the owner's eye or
/// the belt makes it unnecessary.
const CHROMA_DISPARITY: &str = "KJERAG_CHROMA_DISPARITY";

fn disparity_codes() -> f32 {
    static CODES: OnceLock<f32> = OnceLock::new();
    *CODES.get_or_init(|| {
        let Ok(asked) = std::env::var(CHROMA_DISPARITY) else {
            return 0.0;
        };
        match asked.parse::<f32>() {
            Ok(codes) if codes.is_finite() && codes >= 0.0 => {
                println!(
                    "chroma: research disparity gate on, {CHROMA_DISPARITY}={codes}: an evidence \
                     cell whose per-tap lens difference spreads by more than {codes} codes is \
                     refused, because at near content that spread is parallax and not colour \
                     (docs/seam-parity-plan.md 8)"
                );
                codes
            }
            _ => {
                eprintln!(
                    "kjerag: {CHROMA_DISPARITY}={asked} is not a number of codes, so the gate \
                     stays off"
                );
                0.0
            }
        }
    })
}

/// The evidence columns, half open: `s = 18` from
/// `trunc((212 - 2*100)/2) + round(12.5)`, so `[18, 194)`.
pub const EVIDENCE_COLUMNS: std::ops::Range<usize> = 18..194;

/// Degrees of latitude per map row, `180 / (MAP_ROWS - 1)`.
pub fn degrees_per_row() -> f64 {
    180.0 / (MAP_ROWS - 1) as f64
}

/// How far off the seam plane a window row sits, in degrees, signed: positive
/// towards lens 0's pole.
///
/// The seam is the equator, which is map row `(MAP_ROWS - 1) / 2 = 49.5`.
pub fn elevation_deg(window_row: f64) -> f64 {
    let equator = (MAP_ROWS - 1) as f64 / 2.0;
    (equator - (ROI_TOP as f64 + window_row)) * degrees_per_row()
}

/// The azimuth a column sits at, in radians.
///
/// **Longitude divides by `columns`, not `columns - 1`** (ledger 1138-1145),
/// so the last column deliberately stops one step short of wrapping onto the
/// first. That is also why the regularizer below does not join them.
pub fn azimuth_rad(column: f64) -> f64 {
    column / COLUMNS as f64 * std::f64::consts::TAU
}

/// The unit direction a window cell looks along, in the body frame: the seam
/// is the `z = 0` great circle and `+z` is lens 0's axis, which is the same
/// convention [`super::band`] measures its ring in.
pub fn direction(window_row: f64, column: f64) -> [f32; 3] {
    let elevation = elevation_deg(window_row).to_radians();
    let azimuth = azimuth_rad(column);
    let (sin_e, cos_e) = elevation.sin_cos();
    let (sin_a, cos_a) = azimuth.sin_cos();
    [(cos_e * cos_a) as f32, (cos_e * sin_a) as f32, sin_e as f32]
}

/// Which window rows a lens's block covers, half open.
pub fn block(lens: usize) -> std::ops::Range<usize> {
    let top = lens * LENS_STRIDE;
    top..top + BLOCK_ROWS
}

/// The node a lens's `(window_row, column)` is, or `None` where that cell is
/// outside the lens's own block.
///
/// Row major within a block, and lens zero's whole block before lens one's,
/// which is the order `0x183c1a620` walks the two masks in.
pub fn node(lens: usize, window_row: usize, column: usize) -> Option<usize> {
    if !block(lens).contains(&window_row) || column >= COLUMNS {
        return None;
    }
    let local = window_row - lens * LENS_STRIDE;
    Some(lens * BLOCK_ROWS * COLUMNS + local * COLUMNS + column)
}

/// The regularizer's four-neighbour pairs, as node index pairs.
///
/// One row per horizontal or vertical pair whose two mask bytes are nonzero
/// (`0x183c17e40`), which per lens is `(BLOCK_ROWS - 1) * COLUMNS` vertical
/// plus `BLOCK_ROWS * (COLUMNS - 1)` horizontal.
///
/// **The horizontal count is `212 - 1` per row and not `212`**: column 211 is
/// NOT a neighbour of column 0, so the longitude wrap is left open. A ring
/// that closes it is a different operator, and the difference is recorded in
/// docs/research/chromatic.md P-4 rather than quietly taken.
pub fn edges() -> Vec<(usize, usize)> {
    let mut out = Vec::with_capacity(2 * ((BLOCK_ROWS - 1) * COLUMNS + BLOCK_ROWS * (COLUMNS - 1)));
    for lens in 0..2 {
        for row in block(lens) {
            for column in 0..COLUMNS {
                let here = node(lens, row, column).expect("inside the block");
                if let Some(below) = node(lens, row + 1, column) {
                    out.push((here, below));
                }
                if column + 1 < COLUMNS {
                    let right = node(lens, row, column + 1).expect("inside the block");
                    out.push((here, right));
                }
            }
        }
    }
    out
}

/// The endpoint expansion applied to each per-channel quantile tail before the
/// bounds are sign-conditioned: "the code subtracts ten from each lower
/// endpoint and adds ten to each upper endpoint" (ledger 8022-8026).
pub const ENDPOINT_SLACK: f32 = 10.0;

/// The fringe Studio admits outside `[L_c, U_c]`, and what it weighs it at.
///
/// ```text
/// b = 0                                   if any d_c is outside [L_c-p, U_c+p]
/// b = 1                                   if every d_c is inside [L_c, U_c]
/// b = max(1, max_c(d_c-U_c, L_c-d_c))     otherwise
/// w = f32(f32(b-1) * g + 1.0)
/// g = f32(4.0 / f64(f32(p)))
/// ```
///
/// `0x183c1d220..0x183c1d32c` for the byte, `0x183c1c686` for the weight, with
/// the `1.0f` at `0x18460b998` held in `xmm12` across the loop.
///
/// **`p` is the retained band metric**, recovered 2026-08-12 and written up as
/// `docs/research/chromatic.md` P-12. It was never a constant to find: it is
/// seeded to 20 by the MGP2 constructor at `0x183c171ba`, taken whole on a
/// reset call, and otherwise moved by `p <- round(0.98 p + 0.02 clamp(m, 20,
/// 100))` at `0x183c1d07a..0x183c1d0e6` - which is the same integer EMA, on the
/// same clamp, that [`GRID_METRIC_KEEP`] already runs for the budget. So `p` is
/// `control.metric` and no new state is needed.
///
/// Reading it as a WIDTH IN CODES is what makes the rest fall out. `p` is spent
/// twice in one call: the robust band is widened by `p` to decide admission
/// (`0x183c1d124..0x183c1d185` writes `OL = L - p`, `OU = U + p`), and `4/p`
/// normalises the excess so that the outermost admitted sample carries
/// `1 + (p-1)*4/p ~= 5` for EVERY `p`. A wider band does not make the fringe
/// worth more; it makes each code out there worth less.
///
/// The direction reads backwards and is deliberate: a sample further outside
/// the trusted band gets MORE weight, up to five times the interior's. Studio
/// trims the tails to find the band, drops anything past `p` codes as noise,
/// and then leans into what is left - which is the evidence a correction exists
/// to remove.
///
/// One place ours cannot be Studio's: its `d_c` are integer differences of
/// bytes, so its `b` is an integer by construction. Ours are float means over
/// [`CELL_SAMPLES`] taps, so `b` is the continuous excess. The `max(1, ...)`
/// that Studio's integer domain gives for free is written out here, because a
/// float excess below one would otherwise push `w` under one.
pub fn fringe_weight(excess: f32, metric: f32) -> f32 {
    let g = (4.0 / f64::from(metric)) as f32;
    (excess.max(1.0) - 1.0) * g + 1.0
}

/// Per-channel bounds on the difference field, from its own signed quantile
/// tails, expanded and then sign-conditioned.
///
/// The tails are the same 20 percent trim the band metric uses
/// (`0x183c1cea2..0x183c1cf55`). After the expansion, "it sums all six
/// endpoints; if the sum is nonnegative, it replaces every lower endpoint with
/// `min(L_c, 0)`, while a negative sum replaces every upper endpoint with
/// `max(U_c, 0)`" (`0x183c1cfd0..0x183c1d059`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub low: [f32; 3],
    pub high: [f32; 3],
}

impl Bounds {
    /// The trimmed tails of one channel's samples, expanded by
    /// [`ENDPOINT_SLACK`].
    ///
    /// `q = trunc(n * 0.2)`, and the two endpoints are the `q`-th and
    /// `(n-q-1)`-th of the sorted signed values, which is the same rank pair
    /// the metric reads.
    pub fn of(samples: &[[f32; 3]]) -> Self {
        let mut low = [0.0f32; 3];
        let mut high = [0.0f32; 3];
        for channel in 0..3 {
            let mut sorted: Vec<f32> = samples.iter().map(|d| d[channel]).collect();
            sorted.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in a difference field"));
            let n = sorted.len();
            if n == 0 {
                continue;
            }
            let trim = super::chromatic::trim(n);
            low[channel] = sorted[trim.min(n - 1)] - ENDPOINT_SLACK;
            high[channel] = sorted[n - trim - 1] + ENDPOINT_SLACK;
        }
        // The sign condition, which is what stops the band drifting off zero:
        // whichever way the six endpoints lean, the OTHER side is pulled back
        // to include zero.
        let sum: f32 = low.iter().chain(high.iter()).sum();
        match sum >= 0.0 {
            true => low.iter_mut().for_each(|l| *l = l.min(0.0)),
            false => high.iter_mut().for_each(|h| *h = h.max(0.0)),
        }
        Self { low, high }
    }

    /// Whether a sample is admitted, and at what weight.
    ///
    /// `None` past `metric` codes outside the band, `Some(1.0)` inside it, and
    /// the fringe between weighted by [`fringe_weight`]. **WGSL twin**: the
    /// tail of `chroma_admit`.
    pub fn weigh(&self, d: [f32; 3], metric: f32) -> Option<f32> {
        let excess = (0..3)
            .map(|c| (d[c] - self.high[c]).max(self.low[c] - d[c]))
            .fold(0.0f32, f32::max);
        if excess > metric {
            return None;
        }
        Some(match excess <= 0.0 {
            true => 1.0,
            false => fringe_weight(excess, metric),
        })
    }
}

/// The box blur Studio runs over both ratio regions before the give-back:
/// `cv::blur(input, output, cv::Size(3, 11), cv::Point(-1,-1), 4)` at
/// `0x183c02954` through `0x183c11260` (ledger 4222-4230).
///
/// Three columns wide ALONG the seam and eleven rows tall ACROSS it, so it is
/// overwhelmingly a smoother in latitude: eleven rows is 20 degrees. This is
/// the step this project has never had in any form, and a solved field is not
/// what Studio applies - a solved, blurred, ramped one is.
pub const BLUR_COLUMNS: usize = 3;
/// The blur's height in rows. See [`BLUR_COLUMNS`].
pub const BLUR_ROWS: usize = 11;

/// The outer fraction of a block the ratio is ramped back to neutral over:
/// `q = trunc(rows * 0.25f)` from the literal `0.25f` at `0x18460b990`, walked
/// as `left[i] = a*left[i] + (1-a)`, `a = i/q`, over rows `i in [0,q)`
/// against the neutral literal `1.0f` at `0x18460b998`
/// (`0x183c11eb0..0x183c11fb4`).
pub const GIVE_BACK: f32 = 0.25;

/// How many rows of a block the give-back covers: `trunc(BLOCK_ROWS * 0.25)`.
pub fn give_back_rows() -> usize {
    (BLOCK_ROWS as f32 * GIVE_BACK) as usize
}

/// What the ramp multiplies a block row's correction by, in `[0, 1]`.
///
/// Exactly zero on the block's OUTER row, because `a = 0/q = 0` writes the
/// neutral literal whole.
///
/// `0x183c11eb0` pairs row `i` with row `count - 1 - i`, and those are rows of
/// the two DIFFERENT ROIs: the lenses' maps are oriented oppositely about the
/// seam, so one loop walks each from its own outer edge. Ramping both ends of
/// ONE block instead puts a 3-row ramp over the evidence rows of a 12-row
/// block and closes only half the measured step - see the WGSL twin for the
/// three measurements that settled it.
pub fn give_back(block_row: usize) -> f32 {
    let q = give_back_rows();
    match block_row < q {
        true => block_row as f32 / q as f32,
        false => 1.0,
    }
}

/// One evidence sample: the two nodes it constrains, its weight, and the
/// per-channel difference it asks them to explain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Lens zero's node, which the row enters with `+w`.
    pub k0: usize,
    /// Lens one's node, which it enters with `-w`.
    pub k1: usize,
    pub weight: f32,
    /// `D_j = (Y1-Y0, Cb1-Cb0, Cr1-Cr0)`, in codes.
    pub difference: [f32; 3],
}

/// The normal equations `Q = A^T A + R^T R`, held as what they actually are.
///
/// **The whole system is one weighted graph Laplacian**, and seeing that is
/// what makes it cheap. An evidence row `(j,k0,+w),(j,k1,-w)` puts `+w^2` on
/// both diagonals and `-w^2` on the cross pair; a regularizer row
/// `(+0.1,-0.1)` puts `+0.01` on both diagonals and `-0.01` on the cross pair
/// (`0x183c17e40`, `0x183c1c66d..0x183c1c712`). Those are the same shape, so
/// `Q x` is `sum over couplings of c * (x[i] - x[other])` and neither a sparse
/// matrix nor a triangle needs to be materialized.
///
/// It is singular on the constants for exactly that reason, which is why
/// Studio centres after every solve rather than as a taste.
pub struct System {
    /// Every coupling, evidence and penalty together, as `(a, b, c)`.
    couplings: Vec<(usize, usize, f32)>,
    /// `Q`'s diagonal, which is also what the preconditioner inverts.
    diagonal: Vec<f32>,
}

impl System {
    /// Build from this frame's admitted samples. The penalty edges are static
    /// topology and the same every frame; only the evidence moves.
    pub fn new(samples: &[Sample]) -> Self {
        let penalty = super::chromatic::penalty_squared();
        let mut couplings: Vec<(usize, usize, f32)> =
            edges().into_iter().map(|(a, b)| (a, b, penalty)).collect();
        couplings.extend(samples.iter().map(|s| (s.k0, s.k1, s.weight * s.weight)));
        let mut diagonal = vec![0.0f32; NODES];
        for &(a, b, c) in &couplings {
            diagonal[a] += c;
            diagonal[b] += c;
        }
        Self {
            couplings,
            diagonal,
        }
    }

    /// `y = Q x`.
    pub fn apply(&self, x: &[f32], y: &mut [f32]) {
        y.iter_mut().for_each(|v| *v = 0.0);
        for &(a, b, c) in &self.couplings {
            let flow = c * (x[a] - x[b]);
            y[a] += flow;
            y[b] -= flow;
        }
    }

    /// `q_c = A^T D_c` for one channel: `+w*D` at `k0` and `-w*D` at `k1`.
    pub fn rhs(&self, samples: &[Sample], channel: usize) -> Vec<f32> {
        let mut q = vec![0.0f32; NODES];
        for s in samples {
            let term = s.weight * s.difference[channel];
            q[s.k0] += term;
            q[s.k1] -= term;
        }
        q
    }

    /// The diagonal preconditioner, with Studio's fallback: a zero diagonal
    /// becomes `1.0` rather than an infinity
    /// (`0x183c1ebff..0x183c1ecf6`).
    pub fn preconditioner(&self) -> Vec<f32> {
        super::chromatic::preconditioner(&self.diagonal)
    }
}

/// Subtract the field's own mean, in place.
///
/// Studio does this after every solve (`0x183c179ed..0x183c17ac3`) and stores
/// the centred result into BOTH the retained slot and the output, so the warm
/// start is the centred field. The system is a Laplacian and therefore
/// singular on the constants, so without this the correction is free to drift
/// by an arbitrary constant - which would move both hemispheres together
/// where Studio's answer changes nothing away from the seam.
pub fn centre(x: &mut [f32]) {
    if x.is_empty() {
        return;
    }
    let mean = x.iter().map(|v| f64::from(*v)).sum::<f64>() / x.len() as f64;
    x.iter_mut().for_each(|v| *v -= mean as f32);
}

/// Solve one channel, warm started from `x`.
pub fn solve(system: &System, q: &[f32], x: &mut [f32], budget: u32) -> super::chromatic::Exit {
    let pre = system.preconditioner();
    let exit =
        super::chromatic::conjugate_gradient(|v, out| system.apply(v, out), &pre, q, x, budget);
    centre(x);
    exit
}

/// How many threads the solve's single workgroup runs.
pub(crate) const THREADS: usize = 64;

/// The grid's own shader half: the node arithmetic, the matvec, and the
/// conjugate gradient over it.
///
/// **The matvec is a GATHER and not a scatter.** Written per edge it would
/// need an atomic add per endpoint; written per node it is
/// `y[k] = sum over k's neighbours of c * (x[k] - x[n])`, every thread
/// independent and no atomics at all. The neighbours are pure arithmetic on
/// `(lens, row, column)` - the four in-block neighbours for the penalty, and
/// the cross-lens partner at the same cell for the evidence - so nothing has
/// to be stored to walk them.
pub(crate) fn wgsl() -> String {
    format!(
        "const NODES = {NODES}u;\n\
         const COLUMNS = {COLUMNS}u;\n\
         const BLOCK_ROWS = {BLOCK_ROWS}u;\n\
         const LENS_STRIDE = {LENS_STRIDE}u;\n\
         const WINDOW_ROWS = {WINDOW_ROWS}u;\n\
         const BLOCK_NODES = {block}u;\n\
         const EVIDENCE_TOP = {top}u;\n\
         const EVIDENCE_ROWS_N = {rows_n}u;\n\
         const EVIDENCE_FIRST = {first}u;\n\
         const EVIDENCE_LAST = {last}u;\n\
         const MAP_ROWS = {MAP_ROWS}u;\n\
         const ROI_TOP = {ROI_TOP}u;\n\
         const DEG_PER_ROW = {per_row:?};\n\
         const RADIANS = {radians:?};\n\
         const GRID_QSQ = {qsq:?};\n\
         const GRID_TOLERANCE = {tolerance:?};\n\
         const CHROMA_THREADS = {THREADS}u;\n\
         const GIVE_BACK_ROWS = {ramp}u;\n\
         const TRIM_AT = {trim}u;\n\
         const STRIP_MEANS = {STRIP_MEANS}u;\n\
         const STRIP_THRESHOLD = {STRIP_THRESHOLD:?};\n\
         const GATE_COLUMNS = {gcols}u;\n\
         const GATE_ROWS = {grows}u;\n\
         const GATE_SAMPLES = {gsamples}u;\n\
         const WINDOW_CELLS = {cells}u;\n\
         const CELL_SAMPLES = {taps}u;\n\
         const GRID_METRIC_LOW = {mlow:?};\n\
         const GRID_METRIC_HIGH = {mhigh:?};\n\
         const GRID_METRIC_KEEP = {mkeep:?};\n\
         const GRID_METRIC_TAKE = {mtake:?};\n\
         const ENDPOINT_SLACK = {ENDPOINT_SLACK:?};\n\
         const BLUR_COLUMNS = {BLUR_COLUMNS}u;\n\
         const BLUR_ROWS = {BLUR_ROWS}u;\n\
         const DISPARITY_CODES = {disparity:?};\n{SOLVE}",
        block = BLOCK_ROWS * COLUMNS,
        top = evidence_rows().start,
        rows_n = evidence_rows().len(),
        first = EVIDENCE_COLUMNS.start,
        last = EVIDENCE_COLUMNS.end,
        qsq = super::chromatic::penalty_squared(),
        ramp = give_back_rows(),
        per_row = degrees_per_row() as f32,
        trim = super::chromatic::trim(evidence_rows().len() * COLUMNS),
        tolerance = super::chromatic::TOLERANCE,
        gcols = GATE_COLUMNS,
        grows = GATE_ROWS,
        gsamples = GATE_COLUMNS * GATE_ROWS,
        cells = WINDOW_ROWS * COLUMNS,
        disparity = disparity_codes(),
        taps = CELL_SAMPLES,
        mlow = super::chromatic::LOW as f32,
        mhigh = super::chromatic::HIGH as f32,
        mkeep = super::chromatic::KEEP as f32,
        mtake = super::chromatic::TAKE as f32,
        radians = (std::f64::consts::PI / 180.0) as f32,
    )
}

const SOLVE: &str = r#"
// The two lenses' chroma planes, at the numbers the draw already binds. They
// are declared HERE and not with the band's own bindings because this is the
// only pass that reads them: the correlation wants luma alone - a doubled
// edge is geometry and geometry is in the luma, and a chroma plane is a
// quarter of the resolution the search needs - while a hue step between the
// two sensors is exactly what the grid is solving for.
@group(0) @binding(2) var chroma0: texture_2d<f32>;
@group(0) @binding(4) var chroma1: texture_2d<f32>;

// This frame's difference field: `(D_y, D_cb, D_cr, weight)` per evidence
// cell, two rows of `COLUMNS`.
@group(1) @binding(2) var<storage, read_write> evidence: array<vec4<f32>>;

// The solved field, three channels of `NODES`, and the next frame's warm
// start.
@group(1) @binding(3) var<storage, read_write> field: array<f32>;

// The conjugate gradient's own vectors, four of `NODES`. In STORAGE and not
// workgroup memory: five vectors over 5,088 nodes is 102 KB against a 16 KB
// workgroup budget, so the ring's trick of holding the whole solve in
// `var<workgroup>` does not carry to this grid.
@group(1) @binding(4) var<storage, read_write> work: array<f32>;

// What the admission pass measured and the solve reads: the band metric this
// frame, which buys the iteration budget.
struct Control {
  metric: f32,
  budget: f32,
  // Whether this frame's content moved enough to re-solve at all. Zero
  // republishes what the last solve left, which is Studio's own behaviour and
  // the reason its correction does not move every frame.
  changed: f32,
  // How many frames have triggered a solve, and how many have run at all.
  // Instrument only: the gate's whole value is in how often it says no, and
  // that is not visible from the picture.
  triggers: f32,
  frames: f32,
  // How far the APPLIED field moved on the last frame that moved it, summed
  // over every node and channel in codes. This is the flicker, as a number:
  // whatever the gate lets through is what the picture sees change.
  motion: f32,
  // The seam's own colour step, in codes, and what the correction leaves of
  // it: mean |D| and mean |D - (S0 - S1)| over the admitted evidence cells.
  // This is the question the whole arm exists to answer, and until 2026-08-12
  // nothing measured it.
  step: f32,
  residual: f32,
  // The same residual at each window row, not only at the two the evidence
  // was read from. The handover is 8 degrees wide and the evidence band is
  // 1.8, so most of what a viewer sees hand over was never measured - only
  // extrapolated into by the penalty.
  by_row: array<f32, WINDOW_ROWS>,
  // The frozen strip means the gate compares against: two lenses, three
  // vertical strips, three channels.
  baseline: array<f32, STRIP_MEANS>,
};
@group(1) @binding(5) var<storage, read_write> control: Control;

// `work`'s four vectors, each `NODES` long.
const RES = 0u;
const DIR = NODES;
const ND = 2u * NODES;
const PRE = 3u * NODES;

// The direction a window cell looks along, in the body frame. Rust twin:
// `chroma::direction`.
//
// The map is pole to pole - latitude divides by `rows - 1` - so a row is
// exactly `180/99` degrees and the seam is the equator at row 49.5. Longitude
// divides by `COLUMNS` and not `COLUMNS - 1`, so the last column stops one
// step short of wrapping onto the first.
fn cell_direction(window_row: f32, column: f32) -> vec3<f32> {
  let equator = 0.5 * f32(MAP_ROWS - 1u);
  let elevation = (equator - (f32(ROI_TOP) + window_row)) * DEG_PER_ROW * RADIANS;
  let azimuth = column / f32(COLUMNS) * TAU;
  let ce = cos(elevation);
  return vec3<f32>(ce * cos(azimuth), ce * sin(azimuth), sin(elevation));
}

// Which lens's block a node belongs to, and where in it.
fn node_lens(node: u32) -> u32 {
  return node / BLOCK_NODES;
}

fn node_row(node: u32) -> u32 {
  // Window row, not block row: the block starts at `lens * LENS_STRIDE`.
  return (node % BLOCK_NODES) / COLUMNS + node_lens(node) * LENS_STRIDE;
}

fn node_column(node: u32) -> u32 {
  return node % COLUMNS;
}

// The node at a lens's window cell, or NODES where that cell is outside the
// lens's own block. NODES rather than a signed -1 so the callers stay
// unsigned, and every use tests against it.
fn node_at(lens: u32, row: u32, column: u32) -> u32 {
  let top = lens * LENS_STRIDE;
  if row < top || row >= top + BLOCK_ROWS || column >= COLUMNS {
    return NODES;
  }
  return lens * BLOCK_NODES + (row - top) * COLUMNS + column;
}

// Whether a cell carries evidence, which is the four rows the two blocks
// share crossed with the admitted columns.
fn has_evidence(row: u32, column: u32) -> bool {
  return row >= EVIDENCE_TOP && row < EVIDENCE_TOP + EVIDENCE_ROWS_N
    && column >= EVIDENCE_FIRST && column < EVIDENCE_LAST;
}

// The node each lens answers an evidence row with. Past a block's edge that is
// the EDGE node, because past a block's edge the draw holds the edge value
// (`grid_at`) - so a row out there still constrains something, and it
// constrains exactly what the picture will show.
fn answering(lens: u32, row: u32) -> u32 {
  let top = lens * LENS_STRIDE;
  return clamp(row, top, top + BLOCK_ROWS - 1u);
}

// Whether this node is the one `lens` answers evidence row `row` with.
fn answers(lens: u32, row: u32, mine: u32) -> bool {
  return answering(lens, row) == mine;
}

// `y = Q x` at one node. The penalty's four neighbours, then the cross-lens
// partner if this cell was read.
//
// The column direction does NOT wrap: `node_at` refuses `column >= COLUMNS`
// and there is no modulo, which is the ledger's `12 * (212 - 1)` horizontal
// pairs rather than `12 * 212`. Studio leaves the longitude seam open and so
// does this.
// `Q`'s diagonal at one node: the same couplings the matvec walks, summed
// without reference to any vector.
fn diagonal_at(node: u32) -> f32 {
  let lens = node_lens(node);
  let row = node_row(node);
  let column = node_column(node);
  var out = 0.0;
  if row > 0u && node_at(lens, row - 1u, column) != NODES { out += GRID_QSQ; }
  if node_at(lens, row + 1u, column) != NODES { out += GRID_QSQ; }
  if column > 0u && node_at(lens, row, column - 1u) != NODES { out += GRID_QSQ; }
  if node_at(lens, row, column + 1u) != NODES { out += GRID_QSQ; }
  // Every evidence row this node answers for, which past a block's edge is
  // more than one because the draw holds that edge over them.
  for (var r = EVIDENCE_TOP; r < EVIDENCE_TOP + EVIDENCE_ROWS_N; r += 1u) {
    if !has_evidence(r, column) || !answers(lens, r, row) {
      continue;
    }
    let w = weight_at(r, column);
    out += w * w;
  }
  return out;
}

// `q_c = A^T D_c` at one node: `+w*D` where this node is lens zero's and
// `-w*D` where it is lens one's. Gathered, so the node owns its own write.
fn rhs_at(node: u32, channel: u32) -> f32 {
  let lens = node_lens(node);
  let row = node_row(node);
  let column = node_column(node);
  var out = 0.0;
  for (var r = EVIDENCE_TOP; r < EVIDENCE_TOP + EVIDENCE_ROWS_N; r += 1u) {
    if !has_evidence(r, column) || !answers(lens, r, row) {
      continue;
    }
    let cell = evidence[3u * evidence_slot(r, column) + 2u];
    let term = weight_at(r, column) * cell[channel];
    out += select(-term, term, lens == 0u);
  }
  return out;
}

// `y = Q x` at one node, over `field` at a channel's offset.
//
// Two functions rather than one over a pointer: WGSL will not take a storage
// array by pointer without an extension this pass does not ask for, which is
// the same reason `band.rs` has `apply_to_x` and `apply_to_dir`.
fn quadratic_at(node: u32, offset: u32) -> f32 {
  let lens = node_lens(node);
  let row = node_row(node);
  let column = node_column(node);
  let here = field[offset + node];
  var out = 0.0;

  let up = node_at(lens, row - 1u, column);
  if row > 0u && up != NODES { out += GRID_QSQ * (here - field[offset + up]); }
  let down = node_at(lens, row + 1u, column);
  if down != NODES { out += GRID_QSQ * (here - field[offset + down]); }
  let left = node_at(lens, row, column - 1u);
  if column > 0u && left != NODES { out += GRID_QSQ * (here - field[offset + left]); }
  let right = node_at(lens, row, column + 1u);
  if right != NODES { out += GRID_QSQ * (here - field[offset + right]); }

  for (var r = EVIDENCE_TOP; r < EVIDENCE_TOP + EVIDENCE_ROWS_N; r += 1u) {
    if !has_evidence(r, column) || !answers(lens, r, row) {
      continue;
    }
    let partner = node_at(1u - lens, answering(1u - lens, r), column);
    let w = weight_at(r, column);
    out += w * w * (here - field[offset + partner]);
  }
  return out;
}

// The same on the search direction, which is the one the loop repeats.
fn quadratic_of_dir(node: u32) -> f32 {
  let lens = node_lens(node);
  let row = node_row(node);
  let column = node_column(node);
  let here = work[DIR + node];
  var out = 0.0;

  let up = node_at(lens, row - 1u, column);
  if row > 0u && up != NODES { out += GRID_QSQ * (here - work[DIR + up]); }
  let down = node_at(lens, row + 1u, column);
  if down != NODES { out += GRID_QSQ * (here - work[DIR + down]); }
  let left = node_at(lens, row, column - 1u);
  if column > 0u && left != NODES { out += GRID_QSQ * (here - work[DIR + left]); }
  let right = node_at(lens, row, column + 1u);
  if right != NODES { out += GRID_QSQ * (here - work[DIR + right]); }

  for (var r = EVIDENCE_TOP; r < EVIDENCE_TOP + EVIDENCE_ROWS_N; r += 1u) {
    if !has_evidence(r, column) || !answers(lens, r, row) {
      continue;
    }
    let partner = node_at(1u - lens, answering(1u - lens, r), column);
    let w = weight_at(r, column);
    out += w * w * (here - work[DIR + partner]);
  }
  return out;
}

// ------------------------------------------------------ the evidence band
//
// One cell per thread over the two evidence rows and the admitted columns:
// window rows 9 and 10, which are +-0.909 degrees off the seam. Both lenses
// are sampled at the SAME world direction and the difference between them is
// the whole reading.
//
// No correlation gate, and that is the point. The ring read colour only where
// a geometric search had already matched content, which on flat dark ground is
// almost nowhere - measured at 24 percent of the ring on the owner's soil.
// Studio does not search here at all: two lenses looking at one direction
// either see the same thing or differ by the thing being corrected.

// Which slot of the evidence buffer a cell writes.
fn evidence_slot(row: u32, column: u32) -> u32 {
  return row * COLUMNS + column;
}

// The `i`th admitted-row cell, over the two evidence rows only. The admission
// ranks and the solve read those two rows; the other eighteen exist so the
// eleven-row blur has something to average over.
fn evidence_cell(i: u32) -> u32 {
  return (EVIDENCE_TOP + i / COLUMNS) * COLUMNS + (i % COLUMNS);
}

fn weight_at(row: u32, column: u32) -> f32 {
  return evidence[3u * evidence_slot(row, column)].w;
}

// One lens's Y, Cb and Cr at a direction, in CODES, or a negative alpha where
// the lens has no picture there.
//
// **This reassembles P010 and the ring's reader did not.** `dmabuf` aliases a
// 16-bit plane as two 8-bit components, and the draw puts it back together
// with `plane_word` while the band read `.r` raw - which on a 10-bit capture
// is the LOW byte of a word whose ten bits sit at the top, a four-level
// sawtooth rather than a brightness. That never bit on the owner's 8-bit
// captures and would have been silent nonsense on any `.OSV`.
fn codes_at(index: u32, aim: mat3x3<f32>, ray: vec3<f32>) -> vec4<f32> {
  let landing = look(index, aim, ray);
  if !landing.inside {
    return vec4<f32>(0.0, 0.0, 0.0, -1.0);
  }
  let uv = frame_uv(landing.pixel);
  var raw: vec3<f32>;
  if index == 0u {
    let l = textureSampleLevel(luma0, samp, uv, 0.0);
    let c = textureSampleLevel(chroma0, samp, uv, 0.0);
    raw = vec3<f32>(l.r, c.r, c.g);
    if reframe.wide > 0.5 {
      raw = vec3<f32>(plane_word(l.rg), plane_word(c.rg), plane_word(c.ba));
    }
  } else {
    let l = textureSampleLevel(luma1, samp, uv, 0.0);
    let c = textureSampleLevel(chroma1, samp, uv, 0.0);
    raw = vec3<f32>(l.r, c.r, c.g);
    if reframe.wide > 0.5 {
      raw = vec3<f32>(plane_word(l.rg), plane_word(c.rg), plane_word(c.ba));
    }
  }
  // Full-range 0..1 through the capture's own levels, then codes, because
  // Studio's difference field and its `+-10` endpoint slack are in codes.
  let range = levels();
  let y = raw.x * range.luma.x + range.luma.y;
  let c = raw.yz * range.chroma.x + vec2<f32>(range.chroma.y);
  return vec4<f32>(255.0 * y, 255.0 * c.x, 255.0 * c.y, 1.0);
}

// One lens's Y, Cb, Cr in CODES at a uv of its DELIVERED FRAME, with no lens
// model in the way.
//
// The content gate means the lens's whole working image, so it wants the frame
// and not the sphere: `cell_direction` and `look` are for the evidence band,
// which is a thin strip at the seam, and a mean taken there is far too noisy
// to gate on. Measured 2026-08-12: strips cut from the evidence band let 45
// per cent of frames through, which is every other frame re-solving and is
// exactly the flicker this gate exists to stop.
fn frame_codes(index: u32, uv: vec2<f32>) -> vec3<f32> {
  var raw: vec3<f32>;
  if index == 0u {
    let l = textureSampleLevel(luma0, samp, uv, 0.0);
    let c = textureSampleLevel(chroma0, samp, uv, 0.0);
    raw = vec3<f32>(l.r, c.r, c.g);
    if reframe.wide > 0.5 {
      raw = vec3<f32>(plane_word(l.rg), plane_word(c.rg), plane_word(c.ba));
    }
  } else {
    let l = textureSampleLevel(luma1, samp, uv, 0.0);
    let c = textureSampleLevel(chroma1, samp, uv, 0.0);
    raw = vec3<f32>(l.r, c.r, c.g);
    if reframe.wide > 0.5 {
      raw = vec3<f32>(plane_word(l.rg), plane_word(c.rg), plane_word(c.ba));
    }
  }
  let range = levels();
  let y = raw.x * range.luma.x + range.luma.y;
  let c = raw.yz * range.chroma.x + vec2<f32>(range.chroma.y);
  return 255.0 * vec3<f32>(y, c.x, c.y);
}

// Studio's content gate, on the whole working image the way it takes it.
//
// `0x183beaf10` cuts each current lens Mat into three FULL-HEIGHT vertical
// strips, takes `cv::mean` of each, and compares channels zero through two
// against retained baselines: eighteen binary64 absolute differences against
// the exact `3.0` at `0x184666110`. An empty baseline or any delta STRICTLY
// greater than three triggers; equality and unordered values do not.
//
// A skip leaves the baselines FROZEN, so drift accumulates against the last
// trigger and not against the previous frame. That is what makes it a filter
// rather than a threshold, and it is why a scene that is only breathing never
// moves the correction.
@compute @workgroup_size(CHROMA_THREADS)
fn chroma_gate(@builtin(local_invocation_index) lane: u32) {
  var moved = false;
  for (var lens = 0u; lens < 2u; lens += 1u) {
    for (var strip = 0u; strip < 3u; strip += 1u) {
      var sum = vec3<f32>(0.0);
      for (var i = lane; i < GATE_SAMPLES; i += CHROMA_THREADS) {
        let sx = i % GATE_COLUMNS;
        let sy = i / GATE_COLUMNS;
        // Thirds of the frame's width, full height.
        let u = (f32(strip) + (f32(sx) + 0.5) / f32(GATE_COLUMNS)) / 3.0;
        let v = (f32(sy) + 0.5) / f32(GATE_ROWS);
        sum += frame_codes(lens, vec2<f32>(u, v));
      }
      let mean = vec3<f32>(
        total_over(lane, sum.x),
        total_over(lane, sum.y),
        total_over(lane, sum.z),
      ) / f32(GATE_SAMPLES);
      for (var channel = 0u; channel < 3u; channel += 1u) {
        let slot = (lens * 3u + strip) * 3u + channel;
        // Strictly greater, so equality does not trigger.
        if abs(mean[channel] - control.baseline[slot]) > STRIP_THRESHOLD {
          moved = true;
        }
        gate_means[slot] = mean[channel];
      }
    }
  }
  if lane == 0u {
    // A reset has no baseline to be continuous with and always triggers.
    control.changed = select(0.0, 1.0, moved || watch.reset != 0.0);
    control.frames += 1.0;
    control.triggers += control.changed;
    // ONLY a trigger advances the baselines. A skip leaves them frozen, which
    // is the whole mechanism.
    if control.changed != 0.0 {
      for (var slot = 0u; slot < STRIP_MEANS; slot += 1u) {
        control.baseline[slot] = gate_means[slot];
      }
    }
  }
  workgroupBarrier();
}

@compute @workgroup_size(CHROMA_THREADS)
fn chroma_read(@builtin(global_invocation_id) at: vec3<u32>) {
  // Nothing downstream of this runs on a skipped frame, so nothing upstream
  // of it should either. Studio's content gate is OUTER: a call inside the
  // three-strip threshold never invokes MGP2 at all and the old maps are
  // republished whole (ledger 8016-8018). The gate is decided on the GPU, so
  // the CPU cannot drop the dispatch and the early-out has to live here.
  if control.changed == 0.0 {
    return;
  }
  let slot = at.x;
  if slot >= WINDOW_CELLS {
    return;
  }
  let column = slot % COLUMNS;
  // The WHOLE window, not the two evidence rows alone. Studio blurs its bias
  // map regions 3 by 11 BEFORE the solve reads rows 49 and 50 out of them
  // (`0x183c02954` then `0x183c029d6`), and an eleven-row kernel needs eleven
  // rows to average over.
  let row = slot / COLUMNS;
  // Outside the admitted columns there is no sample and no weight, which is
  // what `[18, 194)` means.
  if column < EVIDENCE_FIRST || column >= EVIDENCE_LAST {
    evidence[3u * slot] = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    evidence[3u * slot + 1u] = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    return;
  }
  // A cell is AREA AVERAGED, not point sampled.
  //
  // Studio's `GetYUVBiasMapFast` resizes each lens image into the 212 x 100
  // map before `ComputeYUVBiasMap` differences them, so every map cell is an
  // average over the source pixels that fall in it - at the seam that is
  // roughly 36 by 38 of a 3840-wide fisheye. Taking one bilinear sample per
  // cell instead, which is what this did first, is the same statistic with
  // hundreds of times the variance, and that noise goes straight into the
  // solve and out again as a field that moves every time the gate lets a
  // frame through.
  //
  // `CELL_SAMPLES` squared taps over the cell's own angular footprint is not
  // the resize, but it is the same average at a coarser rate, and its variance
  // falls with the count.
  var zero_sum = vec3<f32>(0.0);
  var one_sum = vec3<f32>(0.0);
  var taps = 0.0;
  // The per-tap difference and its square, for the disparity gate. Free: both
  // codes are already in hand at every tap.
  var diff_sum = 0.0;
  var diff_sq = 0.0;
  let aim0 = body_to_lens(0u);
  let aim1 = body_to_lens(1u);
  for (var sy = 0u; sy < CELL_SAMPLES; sy += 1u) {
    for (var sx = 0u; sx < CELL_SAMPLES; sx += 1u) {
      let dr = (f32(sy) + 0.5) / f32(CELL_SAMPLES) - 0.5;
      let dc = (f32(sx) + 0.5) / f32(CELL_SAMPLES) - 0.5;
      let at = cell_direction(f32(row) + dr, f32(column) + dc);
      let a = codes_at(0u, aim0, at);
      let b = codes_at(1u, aim1, at);
      if a.w < 0.0 || b.w < 0.0 {
        continue;
      }
      zero_sum += a.xyz;
      one_sum += b.xyz;
      // Luma only: parallax moves STRUCTURE, and structure is in the luma.
      let d = b.x - a.x;
      diff_sum += d;
      diff_sq += d * d;
      taps += 1.0;
    }
  }
  let ray = cell_direction(f32(row), f32(column));
  let zero = vec4<f32>(zero_sum / max(taps, 1.0), select(-1.0, 1.0, taps > 0.0));
  let one = vec4<f32>(one_sum / max(taps, 1.0), select(-1.0, 1.0, taps > 0.0));
  if zero.w < 0.0 || one.w < 0.0 {
    evidence[3u * slot] = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    evidence[3u * slot + 1u] = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    return;
  }
  // **The disparity gate, off unless asked for** (`KJERAG_CHROMA_DISPARITY`).
  // A colour difference is a smooth offset and its per-tap spread is near
  // zero; parallax lands the taps on different objects and the spread opens
  // up. Refusing the cell is exactly what an out-of-window sample does above.
  if DISPARITY_CODES > 0.0 && taps > 1.0 {
    let mean = diff_sum / taps;
    let spread = sqrt(max(diff_sq / taps - mean * mean, 0.0));
    if spread > DISPARITY_CODES {
      evidence[3u * slot] = vec4<f32>(0.0, 0.0, 0.0, 0.0);
      evidence[3u * slot + 1u] = vec4<f32>(0.0, 0.0, 0.0, 0.0);
      return;
    }
  }
  // `D_j = (Y1-Y0, Cb1-Cb0, Cr1-Cr0)`. The weight is left at zero here and
  // set by the admission pass, which cannot run until every difference is in.
  evidence[3u * slot] = vec4<f32>(one.xyz - zero.xyz, 0.0);
  // Lens zero's own codes as well, because the content gate means the IMAGES
  // and not their difference. Lens one's are these plus the difference.
  evidence[3u * slot + 1u] = vec4<f32>(zero.xyz, 1.0);
}

// The 3 by 11 box blur Studio runs over both bias-map regions BEFORE the
// solve reads its evidence out of them.
//
// `0x183c02954` constructs `cv::Size(3, 11)` and calls `0x183c11260` twice, on
// the two current bias-map REGIONS - the difference field - and only then are
// rows 49 and 50 handed to the field solves at `0x183c029d6` and
// `0x183c02a18`. An earlier pass here put this on the solved field instead,
// which smooths the answer but not the question: the evidence still moved with
// every frame's noise and the solve moved with it.
//
// Eleven rows over a twenty-row window, three columns along the seam.
// Normalized over the taps that exist, so the window's own edges are not faded
// by arithmetic.
@compute @workgroup_size(CHROMA_THREADS)
fn chroma_blur(@builtin(global_invocation_id) at: vec3<u32>) {
  if control.changed == 0.0 {
    return;
  }
  let slot = at.x;
  if slot >= WINDOW_CELLS {
    return;
  }
  let column = slot % COLUMNS;
  let row = slot / COLUMNS;
  var total = vec3<f32>(0.0);
  var taps = 0.0;
  for (var dr = 0u; dr < BLUR_ROWS; dr += 1u) {
    let rr = i32(row) + i32(dr) - i32(BLUR_ROWS / 2u);
    if rr < 0 || rr >= i32(WINDOW_ROWS) {
      continue;
    }
    for (var dc = 0u; dc < BLUR_COLUMNS; dc += 1u) {
      let cc = i32(column) + i32(dc) - i32(BLUR_COLUMNS / 2u);
      if cc < 0 || cc >= i32(COLUMNS) {
        continue;
      }
      let tap = evidence[3u * (u32(rr) * COLUMNS + u32(cc))];
      // Only cells both lenses actually saw contribute.
      if evidence[3u * (u32(rr) * COLUMNS + u32(cc)) + 1u].w > 0.0 {
        total += tap.xyz;
        taps += 1.0;
      }
    }
  }
  // Into a slot of its own, so the raw field stays whole while every other
  // cell is still reading it.
  evidence[3u * slot + 2u] =
    vec4<f32>(select(vec3<f32>(0.0), total / taps, taps > 0.0), taps);
}

// A monotone `u32` key for an `f32`, so an order statistic can be found by
// bisecting the key space instead of by counting every cell against every
// other. Floats are sign-and-magnitude, so a positive gets its sign bit set
// and a negative is inverted whole, and the results compare as unsigned
// integers exactly where the floats compare.
//
// `-0.0` and `+0.0` take different keys where the floats compare equal. That
// cannot move an endpoint: whichever wins, the value is a zero, and a zero
// with the slack added or taken is the same number either way.
fn order_key(v: f32) -> u32 {
  let bits = bitcast<u32>(v);
  if (bits & 0x80000000u) != 0u {
    return ~bits;
  }
  return bits | 0x80000000u;
}

fn order_value(key: u32) -> f32 {
  if (key & 0x80000000u) != 0u {
    return bitcast<f32>(key & 0x7fffffffu);
  }
  return bitcast<f32>(~key);
}

// The `k`-th smallest difference in one channel, over the whole evidence band.
//
// **This replaced a rank count that compared all 1,272 cells against all 1,272
// of them, three times over, in one workgroup** - 4.9 million comparisons, and
// 7.66 ms of every triggering frame's 33 when it was measured on 2026-08-12.
// The answer is the same: the old ranks were unique only because they were
// tie-broken by index, and the VALUE at a rank does not depend on which of the
// tied cells took it.
//
// `count(key <= mid) > k` is monotone in `mid`, so bisection lands on a key
// that is PRESENT in the data rather than near it. The loop always runs its
// full thirty-two halvings - enough to settle any `u32` - because an early
// exit would make the barrier inside `total_over` non-uniform; once `low`
// meets `high` the remaining steps are idempotent.
fn select_key(channel: u32, k: u32, lane: u32, total: u32) -> u32 {
  var low = 0u;
  var high = 0xffffffffu;
  for (var step = 0u; step < 32u; step += 1u) {
    let mid = low + (high - low) / 2u;
    var count = 0u;
    for (var i = lane; i < total; i += CHROMA_THREADS) {
      if order_key(evidence[3u * evidence_cell(i) + 2u][channel]) <= mid {
        count += 1u;
      }
    }
    if u32(total_over(lane, f32(count))) > k {
      high = mid;
    } else {
      low = mid + 1u;
    }
  }
  return low;
}

// --------------------------------------------------------- the admission
//
// Studio bounds the difference field by its own signed quantile tails before
// it will solve on it: a 20 percent trim per channel, each endpoint pushed out
// by ten, and then whichever way the six endpoints lean, the other side is
// pulled back to include zero (`0x183c1cfd0..0x183c1d059`).
//
// Outside those bounds it keeps a FRINGE, `p` codes wide and weighted up to
// nearly five, where `p` is the retained band metric - the very number
// `control.metric` already holds. Rust twin: `Bounds::weigh` and
// `chroma::fringe_weight`, and see them for why the weight rises outward.

var<workgroup> tails_low: array<f32, 3>;
var<workgroup> tails_high: array<f32, 3>;
var<workgroup> lean: f32;

@compute @workgroup_size(CHROMA_THREADS)
fn chroma_admit(@builtin(local_invocation_index) lane: u32) {
  // See `chroma_read`. This one matters most: on a skipped frame it was the
  // single most expensive thing the player did, and Studio does not run it at
  // all - the band metric, the budget and the admission are all left exactly
  // as the last triggering call set them, which is what "republished" means.
  if control.changed == 0.0 {
    return;
  }
  let total = EVIDENCE_ROWS_N * COLUMNS;
  for (var channel = 0u; channel < 3u; channel += 1u) {
    // The two order statistics. A cell that was never read carries a zero
    // difference and still occupies a rank, the same way Studio's trim is over
    // its whole crop rather than over its admitted part.
    let low = order_value(select_key(channel, TRIM_AT, lane, total));
    let high = order_value(select_key(channel, total - TRIM_AT - 1u, lane, total));
    if lane == 0u {
      tails_low[channel] = low - ENDPOINT_SLACK;
      tails_high[channel] = high + ENDPOINT_SLACK;
    }
    workgroupBarrier();
  }

  // The band metric, off the SAME trimmed tails, before the slack widens them:
  // `max over c of max(|sorted[q]|, |sorted[n-q-1]|)`, bounded into [20,100]
  // (ledger 7860-7869). It buys the iteration budget on Studio's own clamp.
  //
  // Retained through the integer deadband rather than recomputed raw, because
  // the rounding IS the filter: for a retained level and a current one, a
  // change of 24 or less stores the level unchanged, and that is what stops
  // the solve hunting frame to frame on a scene that is only breathing.
  if lane == 0u {
    var worst = 0.0;
    for (var channel = 0u; channel < 3u; channel += 1u) {
      worst = max(worst, max(
        abs(tails_low[channel] + ENDPOINT_SLACK),
        abs(tails_high[channel] - ENDPOINT_SLACK),
      ));
    }
    let current = clamp(round(worst), GRID_METRIC_LOW, GRID_METRIC_HIGH);
    if watch.reset != 0.0 || control.metric < GRID_METRIC_LOW {
      control.metric = current;
    } else {
      control.metric = round(GRID_METRIC_KEEP * control.metric + GRID_METRIC_TAKE * current);
    }
    control.budget = clamp(floor(control.metric / 5.0) + 1.0, 5.0, 25.0);
  }
  workgroupBarrier();

  // The sign condition, which is what stops the admitted band drifting off
  // zero: it sums all six endpoints and pulls the lighter side back over it.
  if lane == 0u {
    lean = 0.0;
    for (var channel = 0u; channel < 3u; channel += 1u) {
      lean += tails_low[channel] + tails_high[channel];
    }
    for (var channel = 0u; channel < 3u; channel += 1u) {
      if lean >= 0.0 {
        tails_low[channel] = min(tails_low[channel], 0.0);
      } else {
        tails_high[channel] = max(tails_high[channel], 0.0);
      }
    }
  }
  workgroupBarrier();

  // `p` is the band metric, and it is a width in CODES: the band is widened by
  // it to decide admission, and `4/p` normalises the excess so the outermost
  // admitted sample is worth `~5` whatever `p` is.
  let p = control.metric;
  let g = 4.0 / p;
  for (var i = lane; i < total; i += CHROMA_THREADS) {
    let column = i % COLUMNS;
    var admitted = column >= EVIDENCE_FIRST && column < EVIDENCE_LAST;
    // A cell whose two lenses did not both see the direction wrote a zero
    // difference AND a zero weight; a zero difference is a legitimate reading
    // elsewhere, so the column test above is what separates them.
    //
    // The excess is how far the worst channel lies past its own endpoint, and
    // it is zero or less for the interior.
    var excess = 0.0;
    for (var channel = 0u; channel < 3u; channel += 1u) {
      let d = evidence[3u * evidence_cell(i) + 2u][channel];
      excess = max(excess, max(d - tails_high[channel], tails_low[channel] - d));
    }
    if excess > p {
      admitted = false;
    }
    // `max(excess, 1)` is what Studio's integer byte gives for free and a
    // float excess does not: without it a sample a fraction of a code outside
    // the band would weigh LESS than the interior.
    let w = (max(excess, 1.0) - 1.0) * g + 1.0;
    evidence[3u * evidence_cell(i)].w =
      select(0.0, select(w, 1.0, excess <= 0.0), admitted);
  }
}

// ------------------------------------------------------------- the solve
//
// One workgroup, and the CG's vectors live in STORAGE rather than workgroup
// memory: five vectors over 5,088 nodes is 102 KB against a 16 KB workgroup
// budget, so the ring's trick of holding everything in `var<workgroup>` does
// not carry over. What stays in workgroup memory is only the reduction.

// ------------------------------------------------- the two spatial controls
//
// Studio does not apply a solved field. It applies a solved, BLURRED, ramped
// one, and this project has never had either step in any form.

// What the draw reads: the field blurred, ramped and eased.
@group(1) @binding(6) var<storage, read_write> shown: array<f32>;

@compute @workgroup_size(CHROMA_THREADS)
fn chroma_finish(@builtin(local_invocation_index) lane: u32) {
  if control.changed == 0.0 {
    return;
  }
  var moved_by = 0.0;
  for (var channel = 0u; channel < 3u; channel += 1u) {
    let base = channel * NODES;
    for (var node = lane; node < NODES; node += CHROMA_THREADS) {
      let lens = node_lens(node);
      let row = node_row(node);
      let column = node_column(node);

      // NOT blurred here. Studio's 3 by 11 runs on the bias-map REGIONS
      // before the solve reads them (`chroma_blur`), not on the field after -
      // blurring the answer smooths the answer, while blurring the question
      // stops the answer moving with every frame's noise in the first place.
      // An earlier pass here had it the wrong way round.
      var value = field[base + node];

      // The give-back (`0x183c0e470`): rows `i in [0,q)` with
      // `q = trunc(rows * 0.25f)` are walked as `left[i] = a*left[i] + (1-a)`,
      // `a = i/q`, against the neutral literal `1.0f`. In this pass's additive
      // units neutral is zero, so the same ramp is a multiply by `a`.
      //
      // **BOTH ends of each block, which the disassembly settled.** The body
      // at `0x183c11eb0` computes, per iteration,
      //
      //   mov ecx, [rax]   ; the ROI's row count
      //   sub ecx, r11d    ; count - i
      //   dec r9           ; count - i - 1   <- the mirrored row
      //
      // so "the paired rows" is row `i` paired with row `rows-1-i` inside ONE
      // ROI, not row `i` of two. Each ratio region is ramped to neutral at its
      // top AND its bottom. `a = i/q` is the `divss` and `1-a` the `subss`
      // against the literal `1.0f` at `0x18460b998`, read back as exactly 1.0.
      //
      // Read on 2026-08-12 rather than reasoned about: an earlier pass took
      // the outer edge alone because that was the reading which fit the
      // oracle, and it was wrong. Both blocks fading at both ends is a
      // CROSSFADE across the seam - where lens zero's correction dies out its
      // partner's is coming up, because the two blocks overlap by four rows.
      // The give-back, at each lens's OUTER edge alone.
      //
      // `0x183c11eb0`'s `dec r9` on `count - i` pairs row `i` with row
      // `count - 1 - i`, and the question that took three readings to settle
      // is which ROI that mirrored row belongs to. It is the OTHER one: the
      // two lenses' maps are oriented oppositely about the seam - lens zero's
      // block runs rows [40,52) and lens one's [48,60) - so pairing `i` with
      // its mirror walks each map from its own outer edge inwards, and
      // `0x183c01450` passing BOTH ROIs in one call is what makes that a
      // single loop.
      //
      // Measurement settled it where reading did not, and both wrong readings
      // are worth keeping written down because each looked right:
      //
      //   ramp both ends of one block   50% of the seam step closed, edge 0
      //   no ramp at all               100% closed, but -1.40 codes at the
      //                                block edge, which is a ring at 17.27
      //   outer edge only              100% closed AND zero at the edge
      //
      // The first fails because the evidence rows sit inside a 3-row ramp on a
      // 12-row block, so it attenuates the correction exactly where it was
      // measured - and no version of Studio half-corrects its own reading.
      let block_row = row - lens * LENS_STRIDE;
      let from_outer = select(BLOCK_ROWS - 1u - block_row, block_row, lens == 0u);
      if from_outer < GIVE_BACK_ROWS {
        value *= f32(from_outer) / f32(GIVE_BACK_ROWS);
      }

      // Written straight through, with NO temporal filter.
      //
      // A first-order ease at TAU_GAIN stood here and was removed on the
      // owner's ruling (2026-08-12): it came from this project's own M-6 and
      // memo 4.4 rather than from Studio's bytes, and the parity mandate does
      // not admit a behaviour that cannot be traced to the disassembly.
      // Studio's temporal mechanism is the band metric's integer deadband -
      // `Metric::advance`, whose rounding swallows every change of 24 or less
      // - and that is what this arm gets, on the budget, and nothing else.
      moved_by += abs(value - shown[base + node]);
      shown[base + node] = value;
    }
    workgroupBarrier();
  }
  let total = total_over(lane, moved_by);
  if lane == 0u {
    control.motion = total / f32(3u * NODES);
  }

  // What the seam still disagrees by after the correction. For every admitted
  // evidence cell, `D` is what the two lenses differed by and `S0 - S1` is
  // what the field now puts between them, so the residual is what a viewer is
  // still left looking at.
  var raw = 0.0;
  var left = 0.0;
  var seen = 0.0;
  for (var i = lane; i < EVIDENCE_ROWS_N * COLUMNS; i += CHROMA_THREADS) {
    let cell = evidence_cell(i);
    let row = cell / COLUMNS;
    let column = cell % COLUMNS;
    if weight_at(row, column) <= 0.0 {
      continue;
    }
    let d = evidence[3u * cell + 2u].xyz;
    let k0 = node_at(0u, row, column);
    let k1 = node_at(1u, row, column);
    if k0 == NODES || k1 == NODES {
      continue;
    }
    var applied = vec3<f32>(0.0);
    for (var channel = 0u; channel < 3u; channel += 1u) {
      let base = channel * NODES;
      applied[channel] = shown[base + k0] - shown[base + k1];
    }
    raw += length(d);
    left += length(d - applied);
    seen += 1.0;
  }
  // And the same across every window row, where nothing was measured.
  for (var row = 0u; row < WINDOW_ROWS; row += 1u) {
    var mine = 0.0;
    var count = 0.0;
    for (var column = EVIDENCE_FIRST + lane; column < EVIDENCE_LAST; column += CHROMA_THREADS) {
      let d = evidence[3u * (row * COLUMNS + column) + 2u];
      if d.w <= 0.0 {
        continue;
      }
      // What the DRAW would apply here, which past a block's edge is the
      // edge row held rather than nothing (`grid_at`). Measuring it any other
      // way measures a pass that does not exist: this read zero for a missing
      // node until 2026-08-12 and so reported the step that the hold removes.
      let r0 = clamp(i32(row), 0, i32(BLOCK_ROWS) - 1);
      let r1 = clamp(i32(row), i32(LENS_STRIDE), i32(LENS_STRIDE + BLOCK_ROWS) - 1);
      let k0 = node_at(0u, u32(r0), column);
      let k1 = node_at(1u, u32(r1), column);
      var applied = vec3<f32>(0.0);
      for (var channel = 0u; channel < 3u; channel += 1u) {
        applied[channel] = shown[channel * NODES + k0] - shown[channel * NODES + k1];
      }
      mine += length(d.xyz - applied);
      count += 1.0;
    }
    let sum = total_over(lane, mine);
    let n = total_over(lane, count);
    if lane == 0u {
      control.by_row[row] = select(0.0, sum / n, n > 0.0);
    }
    workgroupBarrier();
  }

  let raw_total = total_over(lane, raw);
  let left_total = total_over(lane, left);
  let seen_total = total_over(lane, seen);
  if lane == 0u && seen_total > 0.0 {
    control.step = raw_total / seen_total;
    control.residual = left_total / seen_total;
  }
}

var<workgroup> sums: array<f32, CHROMA_THREADS>;
var<workgroup> gate_means: array<f32, STRIP_MEANS>;

fn total_over(lane: u32, mine: f32) -> f32 {
  sums[lane] = mine;
  workgroupBarrier();
  for (var half = CHROMA_THREADS / 2u; half > 0u; half /= 2u) {
    if lane < half {
      sums[lane] += sums[lane + half];
    }
    workgroupBarrier();
  }
  let out = sums[0];
  workgroupBarrier();
  return out;
}

fn dot_over(lane: u32, a: u32, b: u32) -> f32 {
  var mine = 0.0;
  for (var i = lane; i < NODES; i += CHROMA_THREADS) {
    mine += work[a + i] * work[b + i];
  }
  return total_over(lane, mine);
}

@compute @workgroup_size(CHROMA_THREADS)
fn chroma_solve(@builtin(local_invocation_index) lane: u32) {
  // Republish rather than re-solve. The field and the eased field both stand
  // exactly as the last triggering frame left them.
  if control.changed == 0.0 {
    return;
  }
  for (var channel = 0u; channel < 3u; channel += 1u) {
    let base = channel * NODES;

    // The diagonal, and its preconditioner. Studio's fallback is a 1.0 for a
    // zero diagonal rather than an infinity (`0x183c1ebff..0x183c1ecf6`),
    // which on this topology is never reached - every node has at least two
    // neighbours - but is reproduced because a grid ever built disconnected
    // would divide by zero here and not there.
    for (var i = lane; i < NODES; i += CHROMA_THREADS) {
      work[PRE + i] = 1.0 / max(diagonal_at(i), 1e-30);
      if diagonal_at(i) == 0.0 {
        work[PRE + i] = 1.0;
      }
    }
    workgroupBarrier();

    // `q_c = A^T D_c`, formed straight into the residual: `+w*D` at k0 and
    // `-w*D` at k1. Gathered per node, so each thread owns its own writes.
    for (var i = lane; i < NODES; i += CHROMA_THREADS) {
      work[RES + i] = rhs_at(i, channel);
    }
    workgroupBarrier();

    var qq = 0.0;
    for (var i = lane; i < NODES; i += CHROMA_THREADS) {
      qq += work[RES + i] * work[RES + i];
    }
    qq = total_over(lane, qq);

    // The `q == 0` early arm (`0x183c14aa0`): a frame that admitted nothing
    // forces the zero solution outright, rather than dividing by it or letting
    // a warm start stand in for evidence that is gone.
    if qq == 0.0 {
      for (var i = lane; i < NODES; i += CHROMA_THREADS) {
        field[base + i] = 0.0;
      }
      workgroupBarrier();
      continue;
    }

    // `r = q - Q x` off the warm start, which is the field this left last
    // frame.
    for (var i = lane; i < NODES; i += CHROMA_THREADS) {
      work[RES + i] -= quadratic_at(i, base);
    }
    workgroupBarrier();

    let threshold = max(GRID_TOLERANCE * GRID_TOLERANCE * qq, 1.1754944e-38);
    var rho = 0.0;
    for (var i = lane; i < NODES; i += CHROMA_THREADS) {
      let z = work[PRE + i] * work[RES + i];
      work[DIR + i] = z;
      rho += work[RES + i] * z;
    }
    rho = total_over(lane, rho);

    for (var step = 0u; step < u32(control.budget); step += 1u) {
      for (var i = lane; i < NODES; i += CHROMA_THREADS) {
        work[ND + i] = quadratic_of_dir(i);
      }
      workgroupBarrier();
      let denominator = dot_over(lane, DIR, ND);
      if denominator == 0.0 {
        break;
      }
      let alpha = rho / denominator;
      var residual = 0.0;
      for (var i = lane; i < NODES; i += CHROMA_THREADS) {
        field[base + i] += alpha * work[DIR + i];
        work[RES + i] -= alpha * work[ND + i];
        residual += work[RES + i] * work[RES + i];
      }
      residual = total_over(lane, residual);
      if residual < threshold {
        break;
      }
      var rho2 = 0.0;
      for (var i = lane; i < NODES; i += CHROMA_THREADS) {
        rho2 += work[RES + i] * work[PRE + i] * work[RES + i];
      }
      rho2 = total_over(lane, rho2);
      if rho == 0.0 {
        break;
      }
      let beta = rho2 / rho;
      for (var i = lane; i < NODES; i += CHROMA_THREADS) {
        work[DIR + i] = work[PRE + i] * work[RES + i] + beta * work[DIR + i];
      }
      workgroupBarrier();
      rho = rho2;
    }

    // Centred, because the system is a Laplacian and singular on the
    // constants, so the field is otherwise free to drift by one - and a
    // drifting constant moves both hemispheres where Studio's answer changes
    // nothing away from the seam (`0x183c179ed..0x183c17ac3`).
    var mean = 0.0;
    for (var i = lane; i < NODES; i += CHROMA_THREADS) {
      mean += field[base + i];
    }
    mean = total_over(lane, mean) / f32(NODES);
    for (var i = lane; i < NODES; i += CHROMA_THREADS) {
      field[base + i] -= mean;
    }
    workgroupBarrier();
  }
}
"#;

/// The draw's half: what a fragment reads out of the solved grid.
///
/// Separate from [`wgsl`] for the same reason `band::lookup_wgsl` is separate
/// from `band::wgsl` - the two pipelines want different halves, and each
/// declares the field with the access it needs.
pub(crate) fn lookup_wgsl() -> String {
    format!(
        "const GRID_NODES = {NODES}u;\n\
         const GRID_COLUMNS = {COLUMNS}u;\n\
         const GRID_BLOCK_ROWS = {BLOCK_ROWS}u;\n\
         const GRID_LENS_STRIDE = {LENS_STRIDE}u;\n\
         const GRID_MAP_ROWS = {MAP_ROWS}u;\n\
         const GRID_ROI_TOP = {ROI_TOP}u;\n\
         const GRID_DEG_PER_ROW = {per_row:?};\n{LOOKUP}",
        per_row = degrees_per_row() as f32,
    )
}

const LOOKUP: &str = r#"
@group(1) @binding(2) var<storage, read> grid: array<f32>;

// Where a body ray lands in the fusion window, as a fractional
// (window row, column). The exact inverse of `cell_direction`.
fn grid_place(ray: vec3<f32>) -> vec2<f32> {
  let length = length(ray);
  if length <= 0.0 {
    return vec2<f32>(-1.0, -1.0);
  }
  let unit = ray / length;
  // Latitude off the equator, in rows. The seam is map row (rows-1)/2.
  let elevation = asin(clamp(unit.z, -1.0, 1.0)) * 57.29578;
  let equator = 0.5 * f32(GRID_MAP_ROWS - 1u);
  let row = equator - elevation / GRID_DEG_PER_ROW - f32(GRID_ROI_TOP);
  // Longitude, wrapped into [0, COLUMNS).
  let phi = atan2(unit.y, unit.x);
  let turns = fract(phi / 6.2831853 + 1.0);
  return vec2<f32>(row, turns * f32(GRID_COLUMNS));
}

fn grid_at(lens: u32, row: i32, column: i32, channel: u32) -> f32 {
  // A row past the block's end is HELD at the edge rather than dropped to
  // zero, and the two ends mean different things.
  //
  // On the OUTER side the give-back has already walked the field to exactly
  // neutral by the block's last row, so holding it holds zero and the far
  // field is bit-identical to what it was - which is the property O-A
  // measured of Studio's own answer.
  //
  // On the INNER side, past the seam, the block simply stops: lens zero's
  // field was measured at -2.88 codes on its last row and would fall to
  // nothing across the 1.8 degrees to the next, while that lens still carried
  // a quarter of the picture. That is a step INSIDE the blend, and a step
  // inside the blend is what shows when the view sweeps past it. Studio never
  // meets it because its alpha map "writes exactly zero or one. There is no
  // interpolation or feather" - away from the seam exactly one lens is shown,
  // so a correction that stops at the block's edge stops where nobody is
  // looking. The owner's handover is 8 degrees wide by his own A/B, so ours
  // stops in the middle of what everybody is looking at.
  //
  // Holding the edge value is the smallest thing that removes it: the other
  // lens's weight is already falling to nothing over those rows, so what is
  // held reaches almost no pixel, and it reaches them continuously.
  let top = i32(lens * GRID_LENS_STRIDE);
  let held = clamp(row, top, top + i32(GRID_BLOCK_ROWS) - 1);
  // The columns wrap for the LOOKUP even though the penalty leaves them open:
  // a fragment at azimuth just under a full turn has real neighbours on both
  // sides, and refusing the wrap here would draw a seam in the correction at
  // longitude zero. Studio's own remap wraps its right bilinear neighbour to
  // column zero (ledger 1464-1471).
  let wrapped = u32((column % i32(GRID_COLUMNS) + i32(GRID_COLUMNS)) % i32(GRID_COLUMNS));
  let node = lens * GRID_BLOCK_ROWS * GRID_COLUMNS
    + u32(held - top) * GRID_COLUMNS + wrapped;
  return grid[channel * GRID_NODES + node];
}

// One lens's correction at a ray, bilinear over the four surrounding cells.
//
// Bilinear and NOT a smoother reconstruction: Studio samples its own maps
// bilinearly (ledger 915-948, 1464-1471) and shows no cell boundaries,
// because its field is already smooth when it is sampled. A Catmull-Rom was
// written here on 2026-08-12 to hide boundaries and refused the same day - it
// is an invented kernel, and the defect it hid was the field.
fn grid_fix(lens: u32, ray: vec3<f32>) -> vec3<f32> {
  let place = grid_place(ray);
  if place.x < 0.0 {
    return vec3<f32>(0.0);
  }
  let row = floor(place.x);
  let column = floor(place.y);
  let fr = place.x - row;
  let fc = place.y - column;
  var out = vec3<f32>(0.0);
  for (var channel = 0u; channel < 3u; channel += 1u) {
    let a = grid_at(lens, i32(row), i32(column), channel);
    let b = grid_at(lens, i32(row), i32(column) + 1, channel);
    let c = grid_at(lens, i32(row) + 1, i32(column), channel);
    let d = grid_at(lens, i32(row) + 1, i32(column) + 1, channel);
    let top = mix(a, b, fc);
    let bottom = mix(c, d, fc);
    out[channel] = mix(top, bottom, fr);
  }
  return out;
}
"#;

/// How many bytes the evidence band is: `(D_y, D_cb, D_cr, weight)` per cell,
/// two rows of [`COLUMNS`].
pub const EVIDENCE_BYTES: u64 =
    (WINDOW_ROWS * COLUMNS * EVIDENCE_STRIDE * 4 * std::mem::size_of::<f32>()) as u64;

/// How many `vec4`s each window cell carries: the raw difference and its
/// weight, lens zero's own codes for the content gate, and the 3-by-11 blurred
/// difference the solve actually reads.
pub const EVIDENCE_STRIDE: usize = 3;

/// Three channels of [`NODES`], which is both the solved field and the eased
/// one the draw reads.
pub const FIELD_BYTES: u64 = (3 * NODES * std::mem::size_of::<f32>()) as u64;

/// The conjugate gradient's four vectors of [`NODES`]. In a buffer rather than
/// workgroup memory because 102 KB does not fit in 16.
pub const WORK_BYTES: u64 = (4 * NODES * std::mem::size_of::<f32>()) as u64;

/// The band metric and the budget it buys.
pub const CONTROL_BYTES: u64 =
    ((8 + STRIP_MEANS + WINDOW_ROWS) * std::mem::size_of::<f32>()) as u64;

/// Two lenses times three vertical strips times three channels, which is the
/// eighteen binary64 differences the content gate compares
/// (`0x183beb140..0x183beb1f8`).
pub const STRIP_MEANS: usize = 2 * STRIPS * 3;

/// How many full-height vertical strips the content gate cuts each lens's
/// working image into (`0x183beaf10`).
pub const STRIPS: usize = 3;

/// The gate's threshold, the exact binary64 `3.0` at `0x184666110`.
///
/// **Strictly greater triggers**: "an empty retained baseline or any ordered
/// delta strictly greater than three triggers; equality and unordered values
/// do not."
pub const STRIP_THRESHOLD: f32 = 3.0;

/// How finely each strip is sampled to stand in for `cv::mean` over it.
///
/// Studio means every pixel of the strip; this means a grid over it, which is
/// the same statistic at a coarser sample and enormously cheaper. 32 by 96 per
/// strip is 3,072 samples, and a mean over that many has a standard error far
/// under the gate's own 3.0 threshold on any real picture - which is the only
/// property that matters, since a mean too noisy to be under the threshold is
/// a gate that never says no.
pub const GATE_COLUMNS: usize = 32;
/// See [`GATE_COLUMNS`].
pub const GATE_ROWS: usize = 96;

/// How many taps across a map cell's own footprint the evidence averages over,
/// squared.
///
/// Studio resizes its lens images into the map, so a cell is an area average
/// over every source pixel that lands in it - about 36 by 38 at the seam of a
/// 3840-wide fisheye. Six by six is not that, but it is the same average at a
/// coarser rate, and it is what separates a reading from a sample.
pub const CELL_SAMPLES: usize = 6;

/// The grid's own bind group, on the compute side: the evidence, the field,
/// the solver's scratch, the control block, and the eased field.
pub(crate) fn entries() -> [wgpu::BindGroupLayoutEntry; 5] {
    let entry = |binding: u32, size: u64| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size),
        },
        count: None,
    };
    [
        entry(2, EVIDENCE_BYTES),
        entry(3, FIELD_BYTES),
        entry(4, WORK_BYTES),
        entry(5, CONTROL_BYTES),
        entry(6, FIELD_BYTES),
    ]
}

/// The draw's read-only view of the eased field, as an entry to be appended
/// to the band's own read layout.
///
/// **A layout of its own is not available.** The app's device reports
/// `max_bind_groups = 2`, so groups zero and one are the whole budget and a
/// third is a validation error - which is exactly how this shipped: the
/// instrument's device allowed three and the app's did not, so the crash
/// appeared only under the real binary.
pub(crate) fn read_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 2,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(FIELD_BYTES),
        },
        count: None,
    }
}

/// How many workgroups the evidence read dispatches: one thread per cell over
/// the two evidence rows.
pub(crate) const READ_GROUPS: u32 = (WINDOW_ROWS * COLUMNS).div_ceil(THREADS) as u32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_topology_is_the_one_the_ledger_counts() {
        // Every count in ledger 8095-8105, re-derived from this module's own
        // geometry rather than copied from the paragraph.
        assert_eq!(NODES, 5_088);
        let edges = edges();
        let per_lens = (BLOCK_ROWS - 1) * COLUMNS + BLOCK_ROWS * (COLUMNS - 1);
        assert_eq!(per_lens, 4_864);
        assert_eq!(edges.len(), 9_728);
        // `R^T R` nonzeros are `nodes + 2*edges`, upper triangle
        // `nodes + edges`.
        assert_eq!(NODES + 2 * edges.len(), 24_544);
        assert_eq!(NODES + edges.len(), 14_816);

        // The two lenses keep separate nodes in the overlap, which is the
        // thing that makes this 2 * 12 * 212 and not a union.
        let overlap: Vec<usize> = (LENS_STRIDE..BLOCK_ROWS).collect();
        assert_eq!(overlap, vec![8, 9, 10, 11]);
        for row in overlap {
            let zero = node(0, row, 100).expect("lens 0 covers it");
            let one = node(1, row, 100).expect("lens 1 covers it too");
            assert_ne!(zero, one, "row {row} shares a node between lenses");
        }
        // And every node is hit exactly once.
        let mut seen = vec![false; NODES];
        for lens in 0..2 {
            for row in block(lens) {
                for column in 0..COLUMNS {
                    let at = node(lens, row, column).expect("inside");
                    assert!(!seen[at], "node {at} assigned twice");
                    seen[at] = true;
                }
            }
        }
        assert!(seen.into_iter().all(|hit| hit));
    }

    /// **The count follows the handover's width, and it MOVED when the width
    /// did.** At `CROSSOVER_DEG = 8` this rule returned six rows, `7..13`; at
    /// Studio's 6 it returns four, `8..12`. The mixed span is 0.90 of the band
    /// and a row is 1.818 degrees, so the half-width fell 3.60 -> 2.70 and `k`
    /// with it, 2 -> 1.
    ///
    /// **Nothing asserted the count before**, which is why the width change
    /// went green while three documents and this file's own `evidence_rows`
    /// comment went on saying six (`studio-seam-re.md` 19.9). It is pinned
    /// here so the next width change has to say so out loud rather than be
    /// discovered by a reviewer counting rows.
    ///
    /// D1 is the deviation whose case was "six rows against Studio's two". It
    /// is four against two now. That is still a deviation and still the
    /// linear ramp's cost, but the number in the argument is not the number in
    /// the tree, and the argument is the thing that has to be re-made.
    #[test]
    fn the_evidence_rows_are_the_four_the_shipped_handover_reaches() {
        assert_eq!(evidence_rows(), 8..12, "the shipped rule at CROSSOVER_DEG");
        // And the count is the RULE's, not a literal: re-derive it from the
        // width so this fails when the rule changes as well as when the width
        // does.
        let half = f64::from(super::super::projection::mixing_half_width().to_degrees());
        let step = degrees_per_row();
        let k = ((half - step / 2.0) / step).ceil().max(0.0) as usize;
        let k = k.max((BLOCK_ROWS - LENS_STRIDE) / 2 - 1);
        assert_eq!(evidence_rows().len(), 2 * k + 2);
    }

    #[test]
    fn the_map_is_pole_to_pole_so_the_rows_are_angles() {
        // 100 rows over 180 degrees, both poles included.
        assert!((degrees_per_row() - 180.0 / 99.0).abs() < 1e-12);
        assert!((degrees_per_row() - 1.818_181_818).abs() < 1e-6);

        // The evidence rows span the HANDOVER, symmetrically about the seam,
        // which is wider than the rows the two blocks share. A row past a
        // block's edge still constrains that lens, because past its edge the
        // draw holds the edge value (`grid_at`), so what the row constrains is
        // exactly what the picture will show.
        let handover = f64::from(super::super::projection::CROSSOVER_DEG) / 2.0;
        for row in evidence_rows() {
            let off = elevation_deg(row as f64).abs();
            assert!(
                off <= handover + degrees_per_row(),
                "evidence row {row} is {off} degrees off a {handover} degree handover",
            );
        }
        // Symmetric: the seam has two sides and neither gets more evidence.
        let rows: Vec<usize> = evidence_rows().collect();
        let first = elevation_deg(rows[0] as f64);
        let last = elevation_deg(rows[rows.len() - 1] as f64);
        assert!((first + last).abs() < 1e-9, "{first} against {last}");
        // And they reach past the blocks' own overlap, which is the point.
        let overlap: Vec<usize> = block(0).filter(|row| block(1).contains(row)).collect();
        assert!(evidence_rows().len() >= overlap.len());
        assert!((elevation_deg(9.0) - 0.909_090_9).abs() < 1e-5);
        assert!((elevation_deg(10.0) + 0.909_090_9).abs() < 1e-5);

        // The window reaches +-17.27, not the +-30 an earlier pass took off a
        // JPEG through a fitted stereographic model.
        assert!((elevation_deg(0.0) - 17.272_727).abs() < 1e-5);
        assert!((elevation_deg((WINDOW_ROWS - 1) as f64) + 17.272_727).abs() < 1e-5);

        // Each lens reaches far into its own hemisphere and barely across.
        let far = elevation_deg(block(0).start as f64);
        let near = elevation_deg((block(0).end - 1) as f64);
        assert!((far - 17.272_727).abs() < 1e-5, "lens 0 outer edge {far}");
        assert!((near + 2.727_272).abs() < 1e-5, "lens 0 inner edge {near}");
    }

    #[test]
    fn the_directions_are_unit_and_the_seam_is_the_equator() {
        for row in [0.0, 9.0, 9.5, 10.0, 19.0] {
            for column in [0.0, 53.0, 211.0] {
                let dir = direction(row, column);
                let length = dir.iter().map(|c| c * c).sum::<f32>().sqrt();
                assert!((length - 1.0).abs() < 1e-6, "{dir:?} is not unit");
            }
        }
        // Row 9.5 is the equator, so it lies in the seam plane exactly.
        for column in [0.0, 100.0, 211.0] {
            assert!(direction(9.5, column)[2].abs() < 1e-6);
        }
        // Longitude divides by COLUMNS, so the last column stops one step
        // short of wrapping rather than landing back on the first.
        let first = direction(9.5, 0.0);
        let last = direction(9.5, COLUMNS as f64);
        for axis in 0..3 {
            assert!((first[axis] - last[axis]).abs() < 1e-6);
        }
        assert!(azimuth_rad(COLUMNS as f64 - 1.0) < std::f64::consts::TAU);
    }

    #[test]
    fn the_give_back_reaches_exactly_neutral_at_the_outer_edge() {
        assert_eq!(give_back_rows(), 3);
        // Zero on the outermost row is the whole point: it is what makes the
        // far field bit-identical rather than nearly so.
        assert_eq!(give_back(0), 0.0);
        assert!((give_back(1) - 1.0 / 3.0).abs() < 1e-6);
        assert!((give_back(2) - 2.0 / 3.0).abs() < 1e-6);
        // The INNER edge is not ramped: the evidence rows live there, and a
        // ramp over them closes only half the step it measured.
        assert_eq!(give_back(BLOCK_ROWS - 1), 1.0);
        for row in 3..BLOCK_ROWS {
            assert_eq!(give_back(row), 1.0, "row {row} is inside the block");
        }
    }

    /// Evidence at every admitted column of both evidence rows, carrying one
    /// planted per-channel difference in codes.
    fn planted(difference: [f32; 3]) -> Vec<Sample> {
        evidence_rows()
            .flat_map(|row| {
                EVIDENCE_COLUMNS.map(move |column| Sample {
                    k0: node(0, row.min(BLOCK_ROWS - 1), column).expect("held at the edge"),
                    k1: node(1, row.max(LENS_STRIDE), column).expect("held at the edge"),
                    weight: 1.0,
                    difference,
                })
            })
            .collect()
    }

    #[test]
    fn the_solve_recovers_a_planted_split_and_spreads_it_off_the_seam() {
        let samples = planted([6.0, -2.0, 3.0]);
        assert_eq!(samples.len(), evidence_rows().len() * (194 - 18));
        let system = System::new(&samples);

        for channel in 0..3 {
            let want = [6.0f32, -2.0, 3.0][channel];
            let q = system.rhs(&samples, channel);
            let mut x = vec![0.0f32; NODES];
            // Cold, at Studio's own cold budget, so this measures the solve
            // and not the warm start.
            solve(&system, &q, &mut x, super::super::chromatic::COLD_BUDGET);

            // The correction is the DIFFERENCE between the two lenses at the
            // same cell, and it has to come back as what was planted. On the
            // evidence rows, where the constraint is direct.
            for row in evidence_rows() {
                for column in [20, 100, 190] {
                    let split = x[node(0, row.min(BLOCK_ROWS - 1), column).unwrap()]
                        - x[node(1, row.max(LENS_STRIDE), column).unwrap()];
                    assert!(
                        (split - want).abs() < 0.05 * want.abs().max(1.0),
                        "channel {channel} row {row} column {column} recovered \
                         {split} of a planted {want}",
                    );
                }
            }

            // And the penalty has carried it OFF the evidence rows, which is
            // the mechanism that lets 424 samples in a band under a degree
            // wide correct 17 degrees of picture. Lens 0's outermost row is
            // the hardest place for it to reach.
            let outer = block(0).start;
            let far = x[node(0, outer, 100).unwrap()];
            let near = x[node(0, evidence_rows().start, 100).unwrap()];
            assert!(
                far.abs() > 0.1 * near.abs(),
                "channel {channel}: the field died before the block's edge, \
                 {far} against {near} on the evidence row",
            );
            // Centred, because the system is singular on the constants and
            // Studio centres after every solve.
            let mean = x.iter().map(|v| f64::from(*v)).sum::<f64>() / x.len() as f64;
            assert!(mean.abs() < 1e-3, "channel {channel} left a mean of {mean}");
        }
    }

    /// The WGSL's `node_lens` / `node_row` / `node_column` / `node_at`, in
    /// Rust, so the twin below compares arithmetic rather than prose.
    fn shader_node_at(lens: usize, row: usize, column: usize) -> usize {
        let top = lens * LENS_STRIDE;
        if row < top || row >= top + BLOCK_ROWS || column >= COLUMNS {
            return NODES;
        }
        lens * (BLOCK_ROWS * COLUMNS) + (row - top) * COLUMNS + column
    }

    #[test]
    fn the_shaders_node_arithmetic_is_the_rust_one() {
        // Every node, both directions: index to (lens, row, column) and back.
        // `Cell`'s layout bug and the BYTES bug were both this shape - two
        // halves that agreed in prose and not in arithmetic - and neither
        // failed to compile.
        for lens in 0..2 {
            for row in block(lens) {
                for column in 0..COLUMNS {
                    let want = node(lens, row, column).expect("inside the block");
                    assert_eq!(shader_node_at(lens, row, column), want);

                    // And the shader's decomposition inverts it.
                    let block_nodes = BLOCK_ROWS * COLUMNS;
                    assert_eq!(want / block_nodes, lens, "node {want} lens");
                    assert_eq!(
                        (want % block_nodes) / COLUMNS + lens * LENS_STRIDE,
                        row,
                        "node {want} row",
                    );
                    assert_eq!(want % COLUMNS, column, "node {want} column");
                }
            }
        }
        // Outside a lens's own block is refused, not wrapped. Lens 1 has
        // nothing on window row 0 and lens 0 nothing on row 19.
        assert_eq!(shader_node_at(1, 0, 5), NODES);
        assert_eq!(shader_node_at(0, WINDOW_ROWS - 1, 5), NODES);
        assert_eq!(shader_node_at(0, 0, COLUMNS), NODES);
    }

    #[test]
    fn the_shaders_neighbours_are_the_rust_edge_list() {
        // The gather the shader walks has to visit exactly the edges the
        // scatter built, or the two halves solve different systems. Rebuild
        // the edge set from the shader's own four-neighbour rule and compare
        // as undirected pairs.
        let mut gathered = std::collections::HashSet::new();
        for lens in 0..2usize {
            for row in block(lens) {
                for column in 0..COLUMNS {
                    let here = shader_node_at(lens, row, column);
                    let mut visit = |r: usize, c: usize| {
                        let there = shader_node_at(lens, r, c);
                        if there != NODES {
                            gathered.insert((here.min(there), here.max(there)));
                        }
                    };
                    if row > 0 {
                        visit(row - 1, column);
                    }
                    visit(row + 1, column);
                    if column > 0 {
                        visit(row, column - 1);
                    }
                    visit(row, column + 1);
                }
            }
        }
        let scattered: std::collections::HashSet<(usize, usize)> = edges()
            .into_iter()
            .map(|(a, b)| (a.min(b), a.max(b)))
            .collect();
        assert_eq!(
            gathered.len(),
            scattered.len(),
            "the gather walks {} edges and the scatter built {}",
            gathered.len(),
            scattered.len(),
        );
        assert_eq!(gathered, scattered);
    }

    #[test]
    fn no_evidence_is_no_correction_rather_than_a_guess() {
        // The q == 0 early arm (`0x183c14aa0`): a frame that admitted nothing
        // forces the zero solution outright rather than dividing by it or
        // letting a warm start persist.
        let system = System::new(&[]);
        let q = vec![0.0f32; NODES];
        let mut x = vec![0.5f32; NODES];
        let exit = solve(&system, &q, &mut x, 25);
        assert_eq!(
            exit,
            super::super::chromatic::Exit::Converged { iterations: 0 }
        );
        assert!(
            x.iter().all(|v| *v == 0.0),
            "a guess survived an empty frame"
        );
    }

    #[test]
    fn the_bounds_trim_their_tails_and_then_lean_towards_zero() {
        // A field that is entirely positive: the sum of the six endpoints is
        // positive, so every LOWER endpoint is pulled back to zero and the
        // band admits everything between zero and the upper tail.
        let samples: Vec<[f32; 3]> = (0..100).map(|i| [i as f32, i as f32, i as f32]).collect();
        let bounds = Bounds::of(&samples);
        assert_eq!(
            bounds.low, [0.0; 3],
            "the lean did not pull the low side in"
        );
        // 20 percent trim of 100 is 20, so the upper tail is the 79th value,
        // plus the ten of slack.
        assert_eq!(bounds.high, [89.0; 3]);
        // The interior weighs exactly one; 111 codes past the upper endpoint is
        // outside any admitted band, because `p` is at most 100.
        assert_eq!(bounds.weigh([50.0, 50.0, 50.0], 40.0), Some(1.0));
        assert_eq!(bounds.weigh([0.0, 0.0, 200.0], 40.0), None);
        // Every sample inside the band weighs one; the ten above the upper
        // endpoint are the fringe, and they weigh more the further out they
        // are - `i - 89` codes of excess at `4/40` a code.
        for (i, sample) in samples.iter().enumerate() {
            let want = match i <= 89 {
                true => 1.0,
                false => 1.0 + (i as f32 - 89.0 - 1.0) * 0.1,
            };
            assert_eq!(bounds.weigh(*sample, 40.0), Some(want), "sample {i}");
        }
    }

    #[test]
    fn the_fringe_is_p_codes_wide_and_worth_five_at_its_edge() {
        let bounds = Bounds {
            low: [0.0; 3],
            high: [89.0; 3],
        };
        // The invariant that says `4/p` is the right reciprocal and not one of
        // the others: whatever the band metric, the outermost sample Studio
        // will still admit is worth very close to five, and the width of the
        // fringe in codes is `p` itself.
        for p in [20.0f32, 33.0, 50.0, 77.0, 100.0] {
            assert_eq!(bounds.weigh([44.0, 44.0, 44.0], p), Some(1.0));
            // One code out is `b = 1`, which is the interior's own weight: the
            // fringe joins the band continuously rather than stepping at it.
            assert_eq!(bounds.weigh([90.0, 0.0, 0.0], p), Some(1.0));

            let edge = bounds.weigh([89.0 + p, 0.0, 0.0], p).expect("the edge");
            let want = 1.0 + (p - 1.0) * (4.0 / p);
            assert!((edge - want).abs() < 1e-5, "{edge} at p = {p}");
            assert!(
                (4.6..=5.0).contains(&edge),
                "the edge is worth {edge} at p = {p}"
            );

            // And a hair past it is gone entirely, not merely small.
            assert_eq!(bounds.weigh([89.0 + p + 0.5, 0.0, 0.0], p), None);
        }
    }

    #[test]
    fn the_fringe_weight_rises_with_the_excess_and_never_dips_below_one() {
        // Monotone outward, which is the direction that reads backwards and is
        // Studio's: the further past the trusted band a sample lies, the harder
        // the solve works to explain it (docs/research/chromatic.md P-12).
        let p = 50.0;
        let mut last = 0.0;
        for step in 0..=500 {
            let excess = step as f32 * 0.1;
            let w = fringe_weight(excess, p);
            assert!(w >= 1.0, "{w} at an excess of {excess}");
            assert!(w >= last, "the weight fell at an excess of {excess}");
            last = w;
        }
    }
}
