//! The DIS flow field carried to the draw as Studio carries it: the RAW
//! two-component field, in belt/sample pixels, seam-tapered — nothing else
//! (docs/research/studio-seam-re.md §40, `gpu_getLineSphereMap`, §3.3 SASS).
//!
//! **This is the audited Windows/legacy Studio apply, decoded
//! instruction-exact (§40).** Native ONE X2 Flow On uses the same displacement
//! split with its captured 16-degree common SphereAlpha gate. The earlier
//! composition — per-lens colatitude maps, a scalar split, a 5×5 blur on the
//! displacement — was refuted: §37 pointed at `getOpMapA`, and §38/§40 read
//! that `getOpMapA` is the DIS estimator FRONT-END (the 5×5 is the optical-flow
//! INPUT prefilter, which [`super::dis`] already applies to the strips), NOT the
//! apply. The apply is `gpu_getLineSphereMap`, and per output pixel per lens it
//! is (eps `1e-8`):
//!
//! ```text
//! c = coordinate(ray)          # output ray -> belt/sample px (col,row)
//! a = alpha_L                  # this lens's selected flow-gate share
//! if !(0 < a < 1): keep the UNDISPLACED base
//! f = bilinear(flow, clamp(c)) # BOTH components, in belt/sample px
//! q = c + (1 - a) * f_signed   # displace the SAMPLE coord 1:1, both components
//! uv = lookup_L(q)             # belt px -> lens fisheye UV
//! ```
//!
//! **TWO separately-estimated fields, not one negated** (§38/§41 item 1). Studio
//! runs DIS twice with the two belt colour images SWAPPED: **l2r `[0xa88]`** =
//! `DIS(lens0_strip, lens1_strip)` and **r2l `[0xae8]`** = `DIS(lens1_strip,
//! lens0_strip)`. **Lens 0 samples r2l `[0xae8]`, lens 1 samples l2r `[0xa88]`**
//! — a POSITIVE add for both (§40). Negating one field to fake the other only
//! equals Studio if `r2l = −l2r`, which is false in occlusion/parallax (the near
//! riser), so both are carried and TAPERED INDEPENDENTLY (two `fcn.18287e5e0`
//! calls in the binary). The split is the per-pixel `(1 − alpha)`, NOT a
//! constant: at the seam centre (alpha = 0.5) each lens moves half; where its
//! flow-gate share → 1 it stops. The legacy route selects 32 degrees and also
//! uses this as the final colour share. ONE X2 selects 16 degrees for
//! displacement and carries a separate final Template alpha. The apply itself
//! lives in [`crate::projection::Reframe::flow_shift`]; this module only carries
//! the two fields it samples.
//!
//! **What this module holds** (the two things read from the binary that shape
//! the field before the draw sees it):
//!
//! 1. **The field is the DIS flow in belt/sample PIXELS** — no unit change. The
//!    DIS engine runs on the two rectified strips [`crate::band::strip`] builds,
//!    which ARE the belt/line image: `u` is strip columns (along-seam), `v` is
//!    strip rows (across-seam). Studio adds the flow to the sample coordinate
//!    1:1 (§40; its own resolution-ratio pre-scale is the identity here because
//!    our field is already at strip resolution), so no radians conversion and no
//!    rescale — invalid/sentinel votes are zero.
//!
//! 2. **A per-column seam taper** (`fcn.18287e5e0`, HARD, §37/§23.9): a
//!    continuity feather toward the belt's own longitude-wrap seam, applied
//!    identically to the field (both components). See [`taper`].

use super::dis::FlowField;
use crate::band::{STRIP_H, STRIP_W};

/// How far either side of the seam column the continuity feather reaches, in
/// degrees of azimuth: the `2.0` of `fcn.18287e5e0` (§37, §23.9 read it as
/// `-180 - arg3/2`, a two-degree wrap margin on the equirect longitude seam).
const TAPER_DEG: f32 = 2.0;

/// The two DIS flow fields the draw applies, ready to upload: the one buffer the
/// draw binds and [`crate::projection::Reframe::flow_shift`] samples per lens.
#[derive(Clone, Debug, PartialEq)]
pub struct Displacement {
    /// `4 * STRIP_W * STRIP_H` floats, row-major, in **belt/sample pixels**, in
    /// per-lens plane order: `[lens0.u, lens0.v, lens1.u, lens1.v]`, where lens
    /// 0's field is **r2l `[0xae8]`** and lens 1's is **l2r `[0xa88]`** (§40).
    /// `u` is along-seam (strip columns), `v` is across-seam (strip rows). Each
    /// field is seam-tapered independently; invalid/sentinel votes are zero. The
    /// same layout the WGSL twin (`flow_sample`) reads.
    field: Vec<f32>,
}

impl Displacement {
    /// A field that displaces nothing: the picture the draw draws with the flow
    /// switched off, and what a strip with no valid flow composes to.
    pub fn zeros() -> Self {
        Self {
            field: vec![0.0; 4 * STRIP_W * STRIP_H],
        }
    }

    /// Carry the TWO DIS fields to the draw as Studio does (§38/§40): lens 0's
    /// field is **r2l `[0xae8]`** = `DIS(lens1, lens0)`, lens 1's is **l2r
    /// `[0xa88]`** = `DIS(lens0, lens1)`. Keep both components in belt/sample
    /// pixels, zero the invalid votes, and seam-taper EACH field independently.
    /// No split, no blur, no unit change — the per-lens `(1 − alpha)` split is
    /// applied at sample time in [`crate::projection::Reframe::flow_shift`].
    pub fn compose(l2r: &FlowField, r2l: &FlowField) -> Self {
        let n = STRIP_W * STRIP_H;
        let mut out = vec![0.0f32; 4 * n];
        // Lens 0 samples r2l `[0xae8]`, lens 1 samples l2r `[0xa88]` (§40).
        for (lens, field) in [r2l, l2r].into_iter().enumerate() {
            assert_eq!(field.width, STRIP_W, "flow field is not the belt width");
            assert_eq!(field.height, STRIP_H, "flow field is not the belt height");
            let mut u = vec![0.0f32; n];
            let mut v = vec![0.0f32; n];
            for i in 0..n {
                if field.valid[i] {
                    u[i] = field.u[i];
                    v[i] = field.v[i];
                }
            }
            // The continuity feather toward the longitude-wrap seam column,
            // applied to both components of THIS field identically (§37/§38: two
            // taper calls in the binary, one per field).
            taper(&mut u);
            taper(&mut v);
            let base = lens * 2 * n;
            out[base..base + n].copy_from_slice(&u);
            out[base + n..base + 2 * n].copy_from_slice(&v);
        }
        Self { field: out }
    }

    /// Bilinearly sample one lens's flow field at fractional belt coordinates,
    /// returning both components `(u, v)` in belt/sample pixels: BOTH axes clamp
    /// to the belt dims — Studio's kernel clamps the sample coordinate rather
    /// than wrapping the column (§40/§41 MED). Lens 0 reads r2l `[0xae8]`, lens 1
    /// reads l2r `[0xa88]` (§40). WGSL twin: `flow_sample`.
    pub fn sample(&self, lens: usize, col: f32, row: f32) -> [f32; 2] {
        let n = STRIP_W * STRIP_H;
        let base = lens * 2 * n;
        let at = |plane: usize, c: i64, r: i64| {
            let c = c.clamp(0, STRIP_W as i64 - 1) as usize;
            let r = r.clamp(0, STRIP_H as i64 - 1) as usize;
            self.field[base + plane * n + r * STRIP_W + c]
        };
        let c0 = col.floor();
        let r0 = row.floor();
        let (fc, fr) = (col - c0, row - r0);
        let (c0, r0) = (c0 as i64, r0 as i64);
        std::array::from_fn(|plane| {
            let top = at(plane, c0, r0) + (at(plane, c0 + 1, r0) - at(plane, c0, r0)) * fc;
            let bot =
                at(plane, c0, r0 + 1) + (at(plane, c0 + 1, r0 + 1) - at(plane, c0, r0 + 1)) * fc;
            top + (bot - top) * fr
        })
    }

    /// The field as bytes, for the GPU upload: `f32` little-endian, native to
    /// the strip buffer the draw binds.
    pub fn bytes(&self) -> &[u8] {
        // `[f32]` has no padding and no invalid pattern; the same cast
        // `Reframe::bytes` makes for the uniform block.
        unsafe {
            std::slice::from_raw_parts(
                self.field.as_ptr().cast::<u8>(),
                std::mem::size_of_val(self.field.as_slice()),
            )
        }
    }

    /// The fields themselves, for a test to read what was composed: per-lens
    /// plane order `[lens0.u, lens0.v, lens1.u, lens1.v]`, in belt/sample pixels.
    pub fn field(&self) -> &[f32] {
        &self.field
    }
}

/// The per-column continuity feather at the belt's longitude wrap
/// (`fcn.18287e5e0`, §37), **corrected 2026-08-14 after an adversarial read of
/// the bytes** refuted the first version on two counts:
///
/// - **Window width.** `arg3 = 2.0` is the FULL window (`-180 - arg3/2 ..
///   -180 + arg3/2`, §23.9), so the feather reaches `arg3/2 = 1°` either side
///   of the wrap, not `±2°`.
/// - **Mechanism.** The binary blends each edge column with its 360°-WRAP
///   PARTNER (the mirror column across the wrap) and writes BOTH — a symmetric
///   cross-fade that merges the two wrap edges to continuity.
///
/// **What is read and what is modelled.** The `±1°` window and the
/// symmetric-wrap-partner mechanism are RE'd (§23.9, the two `movss` writes
/// `[rcx]`/`[r9+rcx]`). The exact weight RAMP inside the window was not read off
/// the instructions, so a linear ramp to a 50/50 merge at the wrap is a
/// disclosed MODEL — it affects only azimuth-wrap continuity at the strip's
/// longitude cut, never the mid-circle near content (the riser), so its exact
/// shape is cosmetically irrelevant to the seam fix. Applied to the field
/// (both components), a continuity feather, NOT a per-lens split (§37).
fn taper(field: &mut [f32]) {
    let deg_per_col = 360.0 / STRIP_W as f32;
    // arg3/2 = 1°: the half-window either side of the wrap.
    let half_win_deg = TAPER_DEG * 0.5;
    let ncol = (half_win_deg / deg_per_col).ceil() as usize;
    // Column `c` (just after the wrap) and column `W-1-c` (its mirror, just
    // before the wrap) are the two edges that meet at the longitude cut.
    for c in 0..ncol.min(STRIP_W / 2) {
        let partner = STRIP_W - 1 - c;
        // Half-pixel-centred distance from the wrap join, in degrees.
        let dist_deg = (c as f32 + 0.5) * deg_per_col;
        // 1.0 at the window edge, 0.5 at the wrap → a 50/50 merge there, so the
        // field is continuous across the cut. (The 0.5 floor is the modelled
        // part; the window and the wrap-partner pairing are read.)
        let w = 0.5 + 0.5 * (dist_deg / half_win_deg).clamp(0.0, 1.0);
        for row in 0..STRIP_H {
            let a = field[row * STRIP_W + c];
            let b = field[row * STRIP_W + partner];
            field[row * STRIP_W + c] = w * a + (1.0 - w) * b;
            field[row * STRIP_W + partner] = w * b + (1.0 - w) * a;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::dis::FlowField;

    /// A field with a constant `u` and `v` everywhere, valid, for the tests.
    fn constant(u: f32, v: f32) -> FlowField {
        FlowField {
            width: STRIP_W,
            height: STRIP_H,
            u: vec![u; STRIP_W * STRIP_H],
            v: vec![v; STRIP_W * STRIP_H],
            valid: vec![true; STRIP_W * STRIP_H],
            residual: vec![f32::INFINITY; STRIP_W * STRIP_H],
        }
    }

    #[test]
    fn each_lens_samples_its_own_field_both_components() {
        // Two DISTINCT fields, so a lens reading the wrong one is caught.
        // Mid-circle, away from the wrap feather: the sample is the raw flow,
        // 1:1, both components, no unit change.
        let d = Displacement::compose(&constant(7.0, -4.0), &constant(1.0, 2.0));
        // Lens 0 samples r2l (the second arg); lens 1 samples l2r (the first).
        let s0 = d.sample(0, 1000.5, 80.5);
        assert!(
            (s0[0] - 1.0).abs() < 1e-4 && (s0[1] - 2.0).abs() < 1e-4,
            "lens0 {s0:?}"
        );
        let s1 = d.sample(1, 1000.5, 80.5);
        assert!(
            (s1[0] - 7.0).abs() < 1e-4 && (s1[1] + 4.0).abs() < 1e-4,
            "lens1 {s1:?}"
        );
    }

    #[test]
    fn a_zero_field_is_zero_and_invalid_flow_moves_nothing() {
        assert_eq!(Displacement::zeros().field().len(), 4 * STRIP_W * STRIP_H);
        assert!(Displacement::zeros().field().iter().all(|m| *m == 0.0));
        let mut field = constant(9.0, 50.0);
        field.valid.iter_mut().for_each(|v| *v = false);
        let d = Displacement::compose(&field, &field);
        assert!(d.field().iter().all(|m| m.abs() < 1e-7));
    }

    #[test]
    fn the_layout_is_per_lens_u_then_v() {
        // l2r = (3, 11) → lens 1; r2l = (5, 13) → lens 0.
        let d = Displacement::compose(&constant(3.0, 11.0), &constant(5.0, 13.0));
        let n = STRIP_W * STRIP_H;
        assert_eq!(d.field().len(), 4 * n);
        // Mid-circle cell, untouched by the taper.
        let mid = 80 * STRIP_W + 1000;
        // Lens 0 planes (r2l): u then v.
        assert!((d.field()[mid] - 5.0).abs() < 1e-6);
        assert!((d.field()[n + mid] - 13.0).abs() < 1e-6);
        // Lens 1 planes (l2r): u then v.
        assert!((d.field()[2 * n + mid] - 3.0).abs() < 1e-6);
        assert!((d.field()[3 * n + mid] - 11.0).abs() < 1e-6);
    }

    #[test]
    fn the_taper_is_a_symmetric_wrap_partner_crossfade_over_one_degree() {
        // The two wrap edges disagree: the first columns are +1, the last
        // columns (their mirror partners across the cut) are -1. The taper
        // cross-fades each edge with its partner over the ±1° window, merging
        // them to 0 at the cut and leaving mid-circle untouched.
        let mut field = vec![1.0f32; STRIP_W * STRIP_H];
        for row in 0..STRIP_H {
            for c in 0..8 {
                field[row * STRIP_W + (STRIP_W - 1 - c)] = -1.0; // the far edge
            }
        }
        taper(&mut field);
        let deg_per_col = 360.0 / STRIP_W as f32;
        let w0 = 0.5 + 0.5 * (0.5 * deg_per_col / (TAPER_DEG * 0.5)).clamp(0.0, 1.0);
        let expect0 = w0 - (1.0 - w0);
        assert!(
            (field[0] - expect0).abs() < 1e-4 && field[0].abs() < 0.2,
            "col 0 merged to {}, expected ~{expect0}",
            field[0],
        );
        assert!((field[STRIP_W - 1] + expect0).abs() < 1e-4);
        // A quarter turn away — far outside the ±1° window — is untouched.
        assert!((field[STRIP_W / 4] - 1.0).abs() < 1e-6);
        let past = ((1.2 / deg_per_col).ceil() as usize).min(STRIP_W / 4);
        assert!((field[past] - 1.0).abs() < 1e-6, "past 1°: {}", field[past]);
    }
}
