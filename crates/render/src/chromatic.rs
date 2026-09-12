//! Studio's chromatic correction field: the regularized solve that turns a
//! thin band of per-channel evidence into a smooth correction over the seam.
//!
//! This is the Studio-equivalent mechanism, built to the spec recovered in
//! docs/research/studio-seam-re.md, and it is here rather than in the
//! chromatic line's own increment because of what that line found and what the
//! reverse engineering then supplied. docs/research/chromatic.md section 2
//! measured Studio's toggle as **a corridor on the seam, antisymmetric across
//! it, whose size and hue both vary along the seam**, and section 2.4 declined
//! to copy it: *"the difference between their field and stage 8's is not the
//! order, it is the estimation discipline. Copying the order without copying
//! the discipline is exactly how stage 8 ended."* That refusal was right when
//! it was written. The estimation discipline is now recovered in full — the
//! control mask, the weights, this regularizer, the solver's tolerance and its
//! accumulation order, the warm start and the temporal filter — so what is
//! built here is the mechanism rather than a fit of our own shaped like it.
//!
//! **Nothing in this file is chosen.** Every constant carries the address it
//! was read from. Where the recovered evidence stops, the comment says so
//! rather than filling the gap, because a plausible number is exactly what the
//! seam campaign has twice failed on.
//!
//! # What it solves
//!
//! The correction is a scalar field over a node grid laid on the seam band,
//! one grid per lens, solved once per channel. The system is a regularized
//! least squares: sparse evidence rows from the band's own measurements, plus
//! a smoothness penalty over the grid's four-neighbour edges, so that a band
//! which measures nothing at a node still gets a value from its neighbours
//! rather than a hole. Studio retains the penalty's normal matrix across
//! frames and rebuilds only the evidence.

/// One lens's node grid: the shape of the correction field over that lens's
/// half of the seam band.
///
/// Studio's selected shape is 12 rows by 212 columns per lens, which is not
/// arbitrary and is not a tuning knob. The 212 is the fusion map's own width
/// scaled by an exact `1.0600299835` and truncated (`0x3f87af10` at
/// `0x185621ea8`, applied at `0x183c01614..0x183c01625`); the 12 is the number
/// of rows the band's two overlapping masks each fill inside a 20-row active
/// window (`0x183c1df22..0x183c1e094`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Band {
    pub rows: usize,
    pub cols: usize,
}

impl Band {
    /// How many nodes one lens's grid carries.
    pub const fn nodes(self) -> usize {
        self.rows * self.cols
    }

    /// How many four-neighbour pairs it carries: every horizontal pair inside
    /// a row, plus every vertical pair inside a column.
    ///
    /// This is the count of smoothness rows the regularizer contributes for
    /// one lens. Studio's `0x183c17e40` adds exactly one row per pair whose
    /// two mask bytes are both nonzero, and on the selected connected grid
    /// that is every pair.
    pub const fn pairs(self) -> usize {
        (self.rows - 1) * self.cols + self.rows * (self.cols - 1)
    }
}

/// The selected band, from the reverse engineering. Both lenses use it.
pub const SELECTED: Band = Band {
    rows: 12,
    cols: 212,
};

/// How many lenses contribute a grid. The two lenses keep **separate** nodes
/// even where their masks overlap: `0x183c1a620` scans row-major and assigns a
/// distinct node to every nonzero mask-zero pixel before every nonzero
/// mask-one pixel, so the overlap is two nodes deep rather than one shared.
pub const LENSES: usize = 2;

/// Total nodes in one solve, over both lenses.
pub const fn nodes(band: Band) -> usize {
    LENSES * band.nodes()
}

/// The smoothness weight, as the compiled code holds it.
///
/// Studio initializes it to `0.1f` at `0x183c170a9` and writes `+0.1` at one
/// end of each penalty row and `-0.1` at the other. What reaches the normal
/// matrix is therefore the square, and the square is taken in binary32:
/// `fl32(0.1f * 0.1f)`, bit pattern `0x3c23d70b`. Writing `0.01` here instead
/// would be a different number.
pub const PENALTY: f32 = 0.1;

/// `PENALTY * PENALTY`, evaluated the way the machine evaluates it.
///
/// Checked against the recovered bit pattern below rather than trusted.
pub fn penalty_squared() -> f32 {
    PENALTY * PENALTY
}

/// The solver's relative residual tolerance, as exact bits.
///
/// `0x38d1b717`, loaded from `0x184d8a8c4`. The ledger prints it as
/// `9.999999747e-5`, which reads like a deliberately odd number and is not:
/// it is simply the nearest binary32 to `1e-4` written out in full, and
/// `1.0e-4f32` compiles to the same pattern. Kept as bits anyway, so the
/// provenance is on the constant rather than in a comment somebody later
/// deletes.
pub const TOLERANCE: f32 = f32::from_bits(0x38d1_b717);

/// The iteration budget for a solve, from the band's own current metric.
///
/// Studio recomputes this per evidence update as
/// `clamp(floor(m/5) + 1, 5, 25)` over a metric `m` that is necessarily in
/// `[0,255]` (`0x183c1d0ef..0x183c1d11d`). The ceiling of 25 is the whole
/// reason the solve is a filter rather than a solver: 25 diagonally
/// preconditioned steps do not converge a five-thousand-unknown system, so the
/// returned iterate is deliberately partial and the warm start carries it.
/// The rank the 20 percent trim reads its tails at: `q = trunc(n * 0.2f)`,
/// the ledger's own expression at 7860-7864 (`0x183c1cea2..0x183c1cf55`).
///
/// One helper because two callers want the same rank for two purposes - the
/// band metric's trimmed extreme, and the admission bounds' signed tails - and
/// they are the same sort in the same block.
pub fn trim(count: usize) -> usize {
    (count as f32 * 0.2) as usize
}

pub fn budget(metric: f32) -> u32 {
    let metric = metric.clamp(0.0, 255.0);
    ((metric / 5.0).floor() as u32 + 1).clamp(5, 25)
}

/// The budget a solve gets when it has no previous iterate to warm-start from.
///
/// The generic path initializes its own control to 100 at `0x183c1ebb5`, and
/// only replaces it with the retained per-band budget once the channel slots
/// are populated (`0x183c1ed11..0x183c1edc1`). So the first solve after
/// construction or reset is allowed four times the steady-state ceiling,
/// because it is the one solve with nothing behind it.
pub const COLD_BUDGET: u32 = 100;

/// The smoothness penalty's normal matrix, `R^T R`, retained across frames.
///
/// Symmetric and sparse: one diagonal entry per node, and one off-diagonal
/// entry per four-neighbour pair. Studio forms `R` and immediately stores
/// `R^T R` rather than `R` (`0x183c18407..0x183c18435`), which is why the
/// penalty reaches the solve squared.
///
/// Stored as the upper triangle. The diagonal of node `i` is its degree times
/// [`penalty_squared`], accumulated by **repeated addition** rather than by
/// multiplying — which matters, because `3 * q` and `q + q + q` are not the
/// same binary32 number, and the recovered diagonals are the repeated-addition
/// values.
pub struct Regularizer {
    band: Band,
    diagonal: Vec<f32>,
    /// One entry per four-neighbour pair, as `(low node, high node)`. The
    /// value is always `-`[`penalty_squared`], so it is not stored per edge.
    edges: Vec<(u32, u32)>,
}

impl Regularizer {
    /// Build the retained matrix for a **ring** of nodes: one row of `count`
    /// nodes whose two ends are neighbours.
    ///
    /// This is the shape our own evidence comes in, and it differs from
    /// Studio's grid in one dimension and one way. Studio solves a 12-row band
    /// because its evidence is a raw strip of pixels either side of the seam;
    /// ours is already reduced to one reading per direction by a correlation
    /// that matched the content first, so the row dimension has been collapsed
    /// upstream rather than dropped here. What is kept is the estimation
    /// discipline — the same penalty, the same normal matrix, the same solver
    /// and the same warm start — which is the part
    /// docs/research/chromatic.md section 2.4 said we were missing.
    ///
    /// The wrap is not decoration. A seam is a circle, and a field solved on a
    /// line would be free to step across the join by any amount, which is
    /// precisely the artifact the smoothness penalty exists to forbid.
    pub fn ring(count: usize) -> Self {
        let mut diagonal = vec![0.0f32; count];
        let mut edges = Vec::with_capacity(count);
        let q = penalty_squared();
        for node in 0..count {
            let next = (node + 1) % count;
            edges.push((node as u32, next as u32));
            diagonal[node] += q;
            diagonal[next] += q;
        }
        Self {
            band: Band {
                rows: 1,
                cols: count,
            },
            diagonal,
            edges,
        }
    }

    /// Build the retained matrix for a band, for both lenses.
    ///
    /// The two lenses are independent blocks: no penalty row crosses from one
    /// lens's nodes to the other's, because the masks that generate the rows
    /// are per lens. That is what makes the correction free to step across the
    /// seam, which is the whole point of a per-lens field.
    pub fn new(band: Band) -> Self {
        let per_lens = band.nodes();
        let mut diagonal = vec![0.0f32; LENSES * per_lens];
        let mut edges = Vec::with_capacity(LENSES * band.pairs());
        let q = penalty_squared();

        for lens in 0..LENSES {
            let base = lens * per_lens;
            let at = |row: usize, col: usize| (base + row * band.cols + col) as u32;
            for row in 0..band.rows {
                for col in 0..band.cols {
                    let here = at(row, col);
                    // Right neighbour, then the one below: every pair is
                    // visited once, from its lower-indexed end.
                    if col + 1 < band.cols {
                        edges.push((here, at(row, col + 1)));
                        diagonal[here as usize] += q;
                        diagonal[at(row, col + 1) as usize] += q;
                    }
                    if row + 1 < band.rows {
                        edges.push((here, at(row + 1, col)));
                        diagonal[here as usize] += q;
                        diagonal[at(row + 1, col) as usize] += q;
                    }
                }
            }
        }

        Self {
            band,
            diagonal,
            edges,
        }
    }

    pub fn band(&self) -> Band {
        self.band
    }

    /// How many nodes the matrix spans.
    pub fn order(&self) -> usize {
        self.diagonal.len()
    }

    /// How many penalty rows generated it: one per four-neighbour pair.
    pub fn rows(&self) -> usize {
        self.edges.len()
    }

    /// Nonzeros in the full symmetric matrix: the diagonal, plus both
    /// triangles of every edge.
    pub fn nonzeros(&self) -> usize {
        self.order() + 2 * self.edges.len()
    }

    /// Nonzeros in the upper triangle alone: the diagonal plus one per edge.
    pub fn upper_nonzeros(&self) -> usize {
        self.order() + self.edges.len()
    }

    /// `out += (R^T R) * v`, accumulating into an existing vector.
    ///
    /// **Traversal order is not recovered.** The ledger records the multiply
    /// as "the retained self-adjoint upper traversal in `0x183c16bc0`" without
    /// enumerating the order, and with a truncated solve the order is
    /// reachable in the result. This walks the diagonal and then the edges in
    /// construction order; if the traversal is ever recovered and differs,
    /// this is the function that changes.
    pub fn apply(&self, v: &[f32], out: &mut [f32]) {
        debug_assert_eq!(v.len(), self.order());
        debug_assert_eq!(out.len(), self.order());
        let q = penalty_squared();
        for (node, &d) in self.diagonal.iter().enumerate() {
            out[node] += d * v[node];
        }
        for &(a, b) in &self.edges {
            let (a, b) = (a as usize, b as usize);
            out[a] -= q * v[b];
            out[b] -= q * v[a];
        }
    }
}

/// A dot product in the machine's accumulation order.
///
/// Studio's length-5,088 reductions use **two four-lane accumulators over
/// eight-float chunks**, combined in the horizontal order
/// `(lane0 + lane2) + (lane1 + lane3)` (`0x183c1e760..0x183c1e9b5` and its two
/// siblings). Summing left to right instead gives a different binary32 answer,
/// and with a truncated solve that difference does not wash out — it is
/// carried into the next frame by the warm start. So this reproduces the shape
/// rather than calling a library sum.
///
/// How the two accumulators combine with each other is **not** stated by the
/// recovered evidence; lane-wise addition before the horizontal step is this
/// file's choice, and is the one place in this function that is not read off
/// the machine.
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut acc0 = [0.0f32; 4];
    let mut acc1 = [0.0f32; 4];
    let mut i = 0;
    while i + 8 <= a.len() {
        for lane in 0..4 {
            acc0[lane] += a[i + lane] * b[i + lane];
            acc1[lane] += a[i + 4 + lane] * b[i + 4 + lane];
        }
        i += 8;
    }
    let mut lanes = [0.0f32; 4];
    for lane in 0..4 {
        lanes[lane] = acc0[lane] + acc1[lane];
    }
    let mut total = (lanes[0] + lanes[2]) + (lanes[1] + lanes[3]);
    while i < a.len() {
        total += a[i] * b[i];
        i += 1;
    }
    total
}

/// Why a solve stopped.
///
/// Worth carrying rather than discarding: the exit tells an implementation
/// whether its error against Studio is bounded by [`TOLERANCE`] or is a
/// path-dependent iterate that the warm start will carry into the next frame.
/// Under the steady-state [`budget`] of 5 to 25 steps on a five-thousand-node
/// system, [`Exit::Truncated`] is expected to be the normal outcome, and
/// seeing [`Exit::Converged`] instead would be information worth acting on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Exit {
    /// The residual test was satisfied.
    Converged { iterations: u32 },
    /// The iteration budget ran out first.
    Truncated { iterations: u32 },
}

/// Diagonally preconditioned conjugate gradient, in Studio's order.
///
/// Solves `N x = q` for the symmetric `N` supplied by `apply`, starting from
/// whatever is already in `x` — which is the warm start, and is why the caller
/// owns that vector across frames.
///
/// The order below is the recovered one (`0x183c14aa0`) and is not the
/// textbook arrangement: the residual test comes **after** the update to `x`
/// and `r` but **before** the preconditioner is applied again, and the
/// threshold is relative to the right-hand side rather than to the initial
/// residual. There is no zero-denominator guard and no NaN guard; the machine
/// has none, and adding one here would be a different filter.
pub fn conjugate_gradient(
    apply: impl Fn(&[f32], &mut [f32]),
    preconditioner: &[f32],
    q: &[f32],
    x: &mut [f32],
    budget: u32,
) -> Exit {
    let n = q.len();
    let q2 = dot(q, q);

    // `q2 == 0` forces the zero solution with no iterations, rather than
    // dividing by it (`0x183c14aa0`'s early arm).
    if q2 == 0.0 {
        x.iter_mut().for_each(|v| *v = 0.0);
        return Exit::Converged { iterations: 0 };
    }

    let threshold = f32::MIN_POSITIVE.max(TOLERANCE * TOLERANCE * q2);

    let mut nx = vec![0.0f32; n];
    apply(x, &mut nx);
    let mut r: Vec<f32> = (0..n).map(|i| q[i] - nx[i]).collect();

    // The pre-loop test is ordered and strict: equality, or an unordered
    // comparison against a NaN, continues into the loop rather than stopping.
    if dot(&r, &r) < threshold {
        return Exit::Converged { iterations: 0 };
    }

    let mut z: Vec<f32> = (0..n).map(|i| preconditioner[i] * r[i]).collect();
    let mut d = z.clone();
    let mut rho = dot(&r, &z);
    let mut v = vec![0.0f32; n];

    for step in 0..budget {
        v.iter_mut().for_each(|e| *e = 0.0);
        apply(&d, &mut v);
        let alpha = rho / dot(&d, &v);
        for i in 0..n {
            x[i] += alpha * d[i];
            r[i] -= alpha * v[i];
        }
        if dot(&r, &r) < threshold {
            return Exit::Converged {
                iterations: step + 1,
            };
        }
        for i in 0..n {
            z[i] = preconditioner[i] * r[i];
        }
        let rho2 = dot(&r, &z);
        let beta = rho2 / rho;
        for i in 0..n {
            d[i] = z[i] + beta * d[i];
        }
        rho = rho2;
    }

    Exit::Truncated { iterations: budget }
}

/// The diagonal preconditioner: the reciprocal of each diagonal entry.
///
/// A missing or signed-zero diagonal falls back to `1.0` rather than to an
/// infinity (`0x183c1ebff..0x183c1ecf6`). On the selected connected grid every
/// node has at least two neighbours, so the fallback is not reached — but it
/// is reproduced because a band that is ever built disconnected would other-
/// wise divide by zero here and not there.
pub fn preconditioner(diagonal: &[f32]) -> Vec<f32> {
    diagonal
        .iter()
        .map(|&d| if d == 0.0 { 1.0 } else { 1.0 / d })
        .collect()
}

/// The retained band metric, and the temporal filter that moves it.
///
/// Studio prints this as a `0.98 / 0.02` exponential average, and it is not
/// one. The state is an **integer**, the average is computed in binary64 and
/// then rounded back to an integer every call, and the rounding swallows every
/// small change: for a retained `p` and a current `c`, `|c - p| <= 24` stores
/// `p` unchanged. That deadband is the mechanism, not an artifact of it — it
/// is what stops the correction hunting frame to frame on a scene that is
/// merely breathing, and a continuously converging float average in its place
/// is a different filter with a different look.
///
/// The state also carries the solve's weight scale, which is why the two live
/// together: the scale is a function of the metric and is recomputed whenever
/// it moves.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Metric {
    /// The retained integer, in `[LOW, HIGH]`.
    level: i32,
    /// `4.0 / level`, in binary32. Studio stores this beside the level rather
    /// than dividing at each use.
    scale: f32,
}

/// The metric's admitted range. Current readings are bounded into it before
/// the filter sees them, and the recovered deadband table is exhaustive over
/// exactly this domain.
pub const LOW: i32 = 20;
/// The top of the admitted range.
pub const HIGH: i32 = 100;

/// The filter's two coefficients, as the exact binary64 constants the machine
/// holds rather than as decimals that would round differently.
pub(crate) const KEEP: f64 = f64::from_bits(0x3fef_5c28_f5c2_8f5c);
pub(crate) const TAKE: f64 = f64::from_bits(0x3f94_7ae1_47ae_147b);

impl Metric {
    /// Seed the state from a first reading, which is also what a reset does.
    ///
    /// The seed takes the **bounded** current integer directly rather than
    /// filtering toward it from nothing, so the first frame after a reset is
    /// not dragged from wherever the previous scene left the level.
    pub fn seed(current: f32) -> Self {
        let level = bound(current);
        Self {
            level,
            scale: scale_for(level),
        }
    }

    pub fn level(self) -> i32 {
        self.level
    }

    /// The solve's weight scale, `4.0 / level`.
    pub fn scale(self) -> f32 {
        self.scale
    }

    /// The iteration budget this metric buys the solve.
    pub fn budget(self) -> u32 {
        budget(self.level as f32)
    }

    /// Advance the filter by one successful evidence update.
    ///
    /// Only successful updates move it: an invalid or empty band leaves the
    /// state alone rather than decaying it toward anything, because the
    /// absence of evidence is not evidence of a change.
    ///
    /// The arithmetic is the emitted one, and the order matters. Both operands
    /// are promoted from binary32 to binary64, the two products are formed
    /// separately and then added, and the sum is rounded to an integer with
    /// ties to even. Folding the two multiplies into one expression, or
    /// staying in binary32, moves the deadband's edges.
    pub fn advance(&mut self, current: f32) {
        let c = bound(current);
        let kept = KEEP * f64::from(self.level as f32);
        let taken = TAKE * f64::from(c as f32);
        let next = (kept + taken).round_ties_even() as i32;
        if next != self.level {
            self.level = next;
            self.scale = scale_for(next);
        }
    }
}

/// Bound a reading into the admitted integer range.
fn bound(current: f32) -> i32 {
    (current.round_ties_even() as i32).clamp(LOW, HIGH)
}

/// `4.0 / level`, taken in binary32 through a binary64 divide, as stored.
fn scale_for(level: i32) -> f32 {
    (4.0f64 / f64::from(level as f32)) as f32
}

/// Subtract the field's own mean, in place.
///
/// Studio does this after every solve (`0x183c179ed..0x183c17ac3`), and stores
/// the zero-mean result into **both** the retained slot and the current
/// output — so the warm start is the centred field, not the raw one. Skipping
/// it would leave the correction free to drift by a constant, which is exactly
/// the far-field change section 2.4 of the chromatic memo says Studio's answer
/// does not make.
pub fn centre(field: &mut [f32]) {
    if field.is_empty() {
        return;
    }
    let mut total = 0.0f32;
    for &v in field.iter() {
        total += v;
    }
    let mean = total / field.len() as f32;
    for v in field.iter_mut() {
        *v -= mean;
    }
}

/// The chromatic correction round the seam, one value per direction per chroma
/// coordinate, carried across frames.
///
/// One field per coordinate rather than one vector field, because M-1 measured
/// the two coordinates behaving differently and there is no evidence they move
/// together.
///
/// The correction is an **offset**, applied to the two lenses symmetrically:
/// half of the measured difference is taken off one lens and half added to the
/// other, so the seam closes without either hemisphere being declared right.
/// That symmetry is what keeps the far field where it was, which is the one
/// property docs/research/chromatic.md section 2.4 proved Studio's own answer
/// has and a hemisphere-wide constant does not.
pub struct Field {
    penalty: Regularizer,
    /// The retained solution per coordinate, and the warm start for the next
    /// solve. Zero everywhere is the picture the pass drew before this
    /// existed.
    blue: Vec<f32>,
    red: Vec<f32>,
    metric: Option<Metric>,
}

impl Field {
    /// A field over `count` directions with no correction in it yet.
    pub fn new(count: usize) -> Self {
        Self {
            penalty: Regularizer::ring(count),
            blue: vec![0.0; count],
            red: vec![0.0; count],
            metric: None,
        }
    }

    pub fn directions(&self) -> usize {
        self.blue.len()
    }

    /// What to take off lens 0 and add to lens 1 at direction `index`, as
    /// `(Cb, Cr)`. Half the field each way, which is the symmetric split.
    pub fn correction(&self, index: usize) -> (f32, f32) {
        (0.5 * self.blue[index], 0.5 * self.red[index])
    }

    /// Fold one frame's readings in.
    ///
    /// `evidence` is one `(blue, red, weight)` per direction, where the weight
    /// is how much of that direction's reading the pass believes — a direction
    /// that never correlated contributes nothing and is filled by its
    /// neighbours through the penalty rather than by a guess.
    ///
    /// The solve is warm-started from the retained field, so a frame that
    /// measures little moves it little; that persistence is Studio's, and it
    /// is why the iteration budget can be as small as it is.
    pub fn observe(&mut self, evidence: &[(f32, f32, f32)]) {
        debug_assert_eq!(evidence.len(), self.blue.len());

        // The band metric drives the budget the same way Studio's does: more
        // evidence buys more iterations, and it is filtered so the budget does
        // not chatter frame to frame.
        let seen: f32 = evidence.iter().map(|(_, _, w)| *w).sum();
        let reading = 255.0 * seen / evidence.len() as f32;
        let metric = match &mut self.metric {
            Some(metric) => {
                metric.advance(reading);
                *metric
            }
            None => {
                let seeded = Metric::seed(reading);
                self.metric = Some(seeded);
                seeded
            }
        };

        let mut diagonal = self.penalty.diagonal.clone();
        for (node, (_, _, weight)) in evidence.iter().enumerate() {
            diagonal[node] += weight * weight;
        }
        let pre = preconditioner(&diagonal);
        let budget = metric.budget();

        for (retained, pick) in [(&mut self.blue, 0usize), (&mut self.red, 1usize)] {
            let mut q = vec![0.0f32; evidence.len()];
            for (node, reading) in evidence.iter().enumerate() {
                let value = if pick == 0 { reading.0 } else { reading.1 };
                q[node] = reading.2 * reading.2 * value;
            }
            let penalty = &self.penalty;
            let apply = |v: &[f32], out: &mut [f32]| {
                penalty.apply(v, out);
                for (node, (_, _, weight)) in evidence.iter().enumerate() {
                    out[node] += weight * weight * v[node];
                }
            };
            conjugate_gradient(apply, &pre, &q, retained, budget);
            // Deliberately NOT centred; see the WGSL twin for why. Studio's
            // centring preserves the inter-lens difference because its field
            // is per lens; ours IS that difference, so centring would delete
            // it.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recovered topology, re-derived here so a change to [`Band`] that
    /// silently altered the grid would fail rather than drift.
    ///
    /// Every number is from docs/research/studio-seam-re.md at the selected
    /// 200-by-100 shape.
    #[test]
    fn the_selected_band_reproduces_studios_node_and_row_counts() {
        assert_eq!(SELECTED.nodes(), 12 * 212);
        assert_eq!(nodes(SELECTED), 5_088, "total nodes over both lenses");
        assert_eq!(SELECTED.pairs(), 11 * 212 + 12 * 211);
        assert_eq!(SELECTED.pairs(), 4_864, "penalty rows for one lens");
        assert_eq!(LENSES * SELECTED.pairs(), 9_728, "penalty rows in total");
    }

    #[test]
    fn the_regularizer_has_the_recovered_sparsity() {
        let r = Regularizer::new(SELECTED);
        assert_eq!(r.order(), 5_088);
        assert_eq!(r.rows(), 9_728);
        assert_eq!(r.nonzeros(), 24_544, "full symmetric nonzeros");
        assert_eq!(r.upper_nonzeros(), 14_816, "upper triangle");
        assert_eq!(
            r.upper_nonzeros() / LENSES,
            7_408,
            "upper triangle per lens block",
        );
    }

    /// The penalty constants, as bit patterns rather than as decimals.
    ///
    /// `q10` is the square of `0.1f` taken in binary32, and the degree sums are
    /// that square accumulated by repeated addition. The degree-three value is
    /// the one that matters: it cannot be reached by scaling, so it is the only
    /// one of the three that actually tests the accumulation order.
    #[test]
    fn the_penalty_constants_match_the_recovered_bit_patterns() {
        let q = penalty_squared();
        assert_eq!(q.to_bits(), 0x3c23_d70b, "fl32(0.1f * 0.1f)");

        let degree_two = q + q;
        let degree_three = degree_two + q;
        let degree_four = degree_three + q;
        assert_eq!(degree_two.to_bits(), 0x3ca3_d70b, "two neighbours");
        assert_eq!(degree_three.to_bits(), 0x3cf5_c290, "three neighbours");
        assert_eq!(degree_four.to_bits(), 0x3d23_d70b, "four neighbours");
    }

    /// The diagonal a node actually receives is its degree's repeated-addition
    /// sum, so a corner, an edge and an interior node must carry exactly the
    /// three patterns above.
    #[test]
    fn every_node_carries_its_degrees_repeated_addition_diagonal() {
        let band = Band { rows: 4, cols: 5 };
        let r = Regularizer::new(band);
        let q = penalty_squared();
        let two = (q + q).to_bits();
        let three = (q + q + q).to_bits();
        let four = (q + q + q + q).to_bits();

        let at = |row: usize, col: usize| r.diagonal[row * band.cols + col].to_bits();
        assert_eq!(at(0, 0), two, "corner has two neighbours");
        assert_eq!(at(0, 2), three, "top edge has three");
        assert_eq!(at(1, 2), four, "interior has four");
    }

    /// The tolerance is the recovered pattern, and that pattern is the nearest
    /// binary32 to `1e-4`. Both halves are asserted because the ledger's
    /// decimal spelling of it invites the opposite conclusion.
    #[test]
    fn the_tolerance_is_the_recovered_pattern() {
        assert_eq!(TOLERANCE.to_bits(), 0x38d1_b717);
        assert_eq!(
            TOLERANCE.to_bits(),
            1.0e-4f32.to_bits(),
            "9.999999747e-5 is how the nearest float to 1e-4 prints",
        );
    }

    /// The steady-state budget, at both ends of its clamp and at the knee.
    #[test]
    fn the_iteration_budget_clamps_where_the_machine_clamps() {
        assert_eq!(budget(0.0), 5, "floor");
        assert_eq!(budget(20.0), 5, "still the floor at the knee");
        assert_eq!(budget(25.0), 6, "one step past it");
        assert_eq!(budget(120.0), 25, "ceiling");
        assert_eq!(budget(255.0), 25, "and above it");
        assert_eq!(COLD_BUDGET, 100, "the first solve after a reset");
    }

    /// The regularizer is singular on a constant field *in exact arithmetic* —
    /// a flat correction should cost smoothness nothing — which is why the
    /// solve is centred afterwards rather than trusted to pin its own level.
    ///
    /// In the machine's arithmetic it is singular only to within a rounding,
    /// and that is a property of Studio's matrix rather than of this port. The
    /// diagonal is the degree's **repeated-addition** sum while each
    /// off-diagonal is an exact `-q`, so at a node of odd degree the two do
    /// not cancel: `fl(q+q+q)` is not `3q`. The residue is bounded by one ulp
    /// of the node's own diagonal, and the assertion says exactly that rather
    /// than picking a tolerance that happens to pass.
    #[test]
    fn a_constant_field_costs_the_penalty_nothing_but_a_rounding() {
        let band = Band { rows: 4, cols: 5 };
        let r = Regularizer::new(band);
        let ones = vec![1.0f32; r.order()];
        let mut out = vec![0.0f32; r.order()];
        r.apply(&ones, &mut out);
        for (node, &value) in out.iter().enumerate() {
            let ulp = r.diagonal[node] * f32::EPSILON;
            assert!(
                value.abs() <= ulp,
                "node {node} sees {value} from a flat field, over one ulp {ulp}",
            );
        }
        // And the residue is genuinely nonzero somewhere, or the test above
        // would be asserting nothing: an odd-degree node has to show it.
        assert!(
            out.iter().any(|v| *v != 0.0),
            "no rounding residue at all means the diagonal was not accumulated",
        );
    }

    /// With evidence on one node and smoothness everywhere, the solve should
    /// spread that evidence and then centre to zero mean.
    #[test]
    fn evidence_on_one_node_spreads_and_the_result_is_centred() {
        let band = Band { rows: 4, cols: 5 };
        let r = Regularizer::new(band);
        let n = r.order();

        // One unit of evidence on a single node, weighted so the system is
        // not singular: N = R^T R + w on that node alone.
        let anchor = 7usize;
        let weight = 1.0f32;
        let apply = |v: &[f32], out: &mut [f32]| {
            r.apply(v, out);
            out[anchor] += weight * v[anchor];
        };
        let mut diagonal: Vec<f32> = r.diagonal.clone();
        diagonal[anchor] += weight;
        let pre = preconditioner(&diagonal);

        let mut q = vec![0.0f32; n];
        q[anchor] = weight;
        let mut x = vec![0.0f32; n];
        let exit = conjugate_gradient(apply, &pre, &q, &mut x, COLD_BUDGET);

        assert!(
            matches!(exit, Exit::Converged { .. }),
            "a twenty-node system should converge inside the cold budget, got {exit:?}",
        );
        assert!(x[anchor] > 0.0, "the anchored node takes the evidence");
        assert!(
            x.iter().all(|v| v.is_finite()),
            "no non-finite values in the field",
        );

        centre(&mut x);
        let mean: f32 = x.iter().sum::<f32>() / x.len() as f32;
        assert!(mean.abs() < 1e-6, "centred field still has mean {mean}");
    }

    /// A zero right-hand side takes the early arm: zero field, zero
    /// iterations, and no division by the zero norm.
    #[test]
    fn a_zero_right_hand_side_returns_the_zero_field() {
        let band = Band { rows: 3, cols: 3 };
        let r = Regularizer::new(band);
        let pre = preconditioner(&r.diagonal);
        let q = vec![0.0f32; r.order()];
        let mut x = vec![1.0f32; r.order()];
        let exit = conjugate_gradient(|v, out| r.apply(v, out), &pre, &q, &mut x, 25);
        assert_eq!(exit, Exit::Converged { iterations: 0 });
        assert!(x.iter().all(|&v| v == 0.0), "field forced to zero");
    }

    /// The reduction's shape, not just its value: a vector whose exact sum is
    /// order-sensitive must come back with the machine's answer rather than a
    /// left-to-right one.
    #[test]
    fn the_dot_product_uses_the_machines_accumulation_order() {
        // Eight terms where a big first element hides the small ones from a
        // naive left-to-right sum but not from four separate lanes.
        let mut a = vec![1.0f32; 16];
        a[0] = 1.0e8;
        let b = vec![1.0f32; 16];

        let mut sequential = 0.0f32;
        for i in 0..a.len() {
            sequential += a[i] * b[i];
        }

        let lanes = dot(&a, &b);
        assert_ne!(
            lanes.to_bits(),
            sequential.to_bits(),
            "lane order must be observable, or this test proves nothing",
        );
    }

    /// The deadband, over the whole admitted domain.
    ///
    /// Anything inside 24 units stores the retained value unchanged. This is
    /// the property that makes the filter a filter, and it is asserted over
    /// every reachable pair rather than at a few samples.
    #[test]
    fn the_filter_ignores_every_change_inside_twenty_four_units() {
        for p in LOW..=HIGH {
            for c in LOW..=HIGH {
                if (c - p).abs() > 24 {
                    continue;
                }
                let mut metric = Metric {
                    level: p,
                    scale: scale_for(p),
                };
                metric.advance(c as f32);
                assert_eq!(
                    metric.level(),
                    p,
                    "retained {p} moved on a current of {c}, inside the deadband",
                );
            }
        }
    }

    /// The boundary at exactly 25, which is where the two separately rounded
    /// binary64 multiplies stop behaving like an ideal half-integer rule.
    ///
    /// This is the test that actually proves the arithmetic: the exceptions
    /// below were produced by exhaustive evaluation of the emitted sequence,
    /// and no ordinary implementation of a `0.98/0.02` average reproduces
    /// them by accident. If this passes, the promotion order, the separate
    /// multiplies and the tie-to-even rounding are all right.
    #[test]
    fn the_boundary_at_twenty_five_is_asymmetric_exactly_where_the_machine_is() {
        // Rising: move iff the retained value is odd, with three exceptions.
        for p in LOW..=(HIGH - 25) {
            let mut metric = Metric {
                level: p,
                scale: scale_for(p),
            };
            metric.advance((p + 25) as f32);
            let moves = p % 2 != 0 && !matches!(p, 33 | 49 | 73);
            let want = if moves { p + 1 } else { p };
            assert_eq!(metric.level(), want, "retained {p}, current {}", p + 25);
        }

        // Falling: move iff odd, plus four even values that also move.
        for p in (LOW + 25)..=HIGH {
            let mut metric = Metric {
                level: p,
                scale: scale_for(p),
            };
            metric.advance((p - 25) as f32);
            let moves = p % 2 != 0 || matches!(p, 56 | 58 | 66 | 98);
            let want = if moves { p - 1 } else { p };
            assert_eq!(metric.level(), want, "retained {p}, current {}", p - 25);
        }
    }

    /// Past the boundary the filter always moves, and never by more than two.
    #[test]
    fn past_the_boundary_it_always_moves_and_never_by_more_than_two() {
        for p in LOW..=HIGH {
            for c in LOW..=HIGH {
                if (c - p).abs() < 26 {
                    continue;
                }
                let mut metric = Metric {
                    level: p,
                    scale: scale_for(p),
                };
                metric.advance(c as f32);
                let step = metric.level() - p;
                assert!(step != 0, "retained {p} stalled on a current of {c}");
                assert!(
                    step.signum() == (c - p).signum(),
                    "retained {p} moved away from {c}",
                );
                assert!(
                    step.abs() <= 2,
                    "retained {p} jumped {step} toward {c}, over the two-step cap",
                );
            }
        }
    }

    /// A seed takes the reading whole, and bounds it.
    #[test]
    fn a_seed_takes_the_bounded_reading_rather_than_filtering_toward_it() {
        assert_eq!(Metric::seed(70.0).level(), 70);
        assert_eq!(Metric::seed(0.0).level(), LOW, "bounded from below");
        assert_eq!(Metric::seed(4_000.0).level(), HIGH, "and from above");
    }

    /// The weight scale travels with the level and is the machine's divide.
    #[test]
    fn the_weight_scale_follows_the_level() {
        let metric = Metric::seed(50.0);
        assert_eq!(metric.scale(), (4.0f64 / 50.0f64) as f32);
        assert_eq!(metric.budget(), budget(50.0), "and so does the budget");
    }

    /// A ring's ends are neighbours, so every node has exactly two of them and
    /// there is no seam in the seam's own field.
    #[test]
    fn a_ring_gives_every_direction_two_neighbours() {
        let r = Regularizer::ring(8);
        assert_eq!(r.order(), 8);
        assert_eq!(r.rows(), 8, "one edge per node, including the join");
        let q = penalty_squared();
        for (node, &d) in r.diagonal.iter().enumerate() {
            assert_eq!(d.to_bits(), (q + q).to_bits(), "node {node}");
        }
    }

    /// A field nobody has measured is exactly the picture the pass drew
    /// before it existed. This is the byte-identity the arm's off state needs.
    #[test]
    fn an_unmeasured_field_corrects_nothing() {
        let field = Field::new(16);
        for index in 0..field.directions() {
            assert_eq!(field.correction(index), (0.0, 0.0));
        }
    }

    /// Evidence with no weight behind it moves nothing, which is the same
    /// refusal rule the band takes: what is absent is not a reason to believe
    /// the opposite.
    #[test]
    fn evidence_nobody_believes_moves_the_field_nowhere() {
        let mut field = Field::new(16);
        let evidence: Vec<_> = (0..16).map(|_| (0.2, -0.1, 0.0)).collect();
        field.observe(&evidence);
        for index in 0..field.directions() {
            let (b, r) = field.correction(index);
            assert!(b.abs() < 1e-6 && r.abs() < 1e-6, "direction {index}");
        }
    }

    /// A ring that all reads the same offset gets that offset corrected, half
    /// off one lens and half onto the other.
    ///
    /// This test asserted the opposite until 2026-08-12, and the opposite was
    /// wrong. It required a uniform reading to leave the ring's mean alone,
    /// which is what a mean-centred field does — and centring a field that IS
    /// the inter-lens difference deletes the correction rather than levelling
    /// it. M-1 measured the difference as largely constant round the ring, so
    /// the old behaviour threw away most of it and the seam stayed visible.
    #[test]
    fn a_uniform_reading_is_corrected_by_half_each_way() {
        let mut field = Field::new(32);
        let evidence: Vec<_> = (0..32).map(|_| (0.04, 0.0, 1.0)).collect();
        for _ in 0..24 {
            field.observe(&evidence);
        }
        for index in 0..field.directions() {
            let (blue, _) = field.correction(index);
            assert!(
                (blue - 0.02).abs() < 2.0e-3,
                "direction {index} corrects by {blue}, not half of 0.04",
            );
        }
    }

    /// A step in the evidence comes back as a smoothed step rather than a
    /// cliff, and it does not ring: that is what the penalty buys, and a
    /// correction that oscillated round the seam would be visible as banding.
    #[test]
    fn a_step_in_the_evidence_comes_back_smoothed_and_does_not_oscillate() {
        let mut field = Field::new(64);
        let evidence: Vec<_> = (0..64)
            .map(|i| {
                if i < 32 {
                    (0.05, 0.0, 1.0)
                } else {
                    (-0.05, 0.0, 1.0)
                }
            })
            .collect();
        for _ in 0..16 {
            field.observe(&evidence);
        }
        let values: Vec<f32> = (0..64).map(|i| field.correction(i).0).collect();
        // Same sign as the evidence on each half, well away from the two joins.
        assert!(values[16] > 0.0 && values[48] < 0.0, "sides swapped");
        // And monotone along one half, so no ringing.
        for i in 33..47 {
            assert!(
                values[i + 1] <= values[i] + 1e-6,
                "field rises again at {i}: {:?}",
                &values[i - 1..=i + 1],
            );
        }
    }

    /// The preconditioner's fallback, which the selected grid never reaches
    /// but a disconnected one would.
    #[test]
    fn a_zero_diagonal_falls_back_to_one() {
        let pre = preconditioner(&[0.0, 4.0]);
        assert_eq!(pre[0], 1.0);
        assert_eq!(pre[1], 0.25);
    }
}
