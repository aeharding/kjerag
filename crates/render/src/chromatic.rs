//! The chromatic seam correction: a seam-local source-matching field, per
//! channel, estimated from the current frame's own overlap and spread on a
//! fixed angular kernel (issue #103, stage 10).
//!
//! **Studio-equivalent.** This reproduces what Insta360 Studio's "Chromatic
//! Calibration" toggle does, as measured from Studio's own output on the
//! owner's two on/off pairs (docs/research/chromatic.md M-8): the correction
//! is ODD about the seam line and cancels ON it (even-over-odd energy 0.003
//! to 0.023), its shape is FIXED IN DEGREES on the sphere (the same
//! normalized profile at every azimuth, half-max half-width 11.8 to 13.3
//! degrees, zero by 30, while the same kernel spans 145 to 471 output pixels
//! across one render), and all of its along-seam variation is AMPLITUDE.
//!
//! **The form, and the two refusals that shaped it** (chromatic.md M-9):
//!
//! - Each lens is pulled toward the local mean of what the two lenses deliver,
//!   half the local disagreement each, equal and opposite. The crossfade then
//!   cancels the correction exactly at the 50/50 line - `(l0 + d/2) * w0 +
//!   (l1 - d/2) * w1` is the uncorrected average wherever the weights are
//!   equal - which is the oracle's own dark-line fingerprint (M-8.2) by
//!   construction rather than by fit.
//! - **A constant per lens is only the DC term.** The hemisphere-scale split
//!   is real on blue-amber only, -0.006 to -0.022 ln at 2.5 to 8.8 standard
//!   errors (M-1.4), and it accounts for about half the ring's variance and no
//!   more (M-1.5). It is not built as a separate mechanism; it emerges as the
//!   field's own DC through the pooling below.
//! - **No stored per-direction field.** The per-direction structure is real at
//!   an instant - 96 to 100 percent of the azimuth-to-azimuth spread survives
//!   the noise correction - and mostly stale minutes later: the same
//!   body-frame directions read at two places in one file correlate at only
//!   r +0.13 to +0.70, twice not at all (M-7). So the local value here is the
//!   current frame's own reading, smoothed only in time at the tone gain's
//!   class ([`super::band::TAU_GAIN_S`]; the pooled reading's frame-to-frame
//!   rms is 0.0016 to 0.0067 ln, so a 2 s ease has three orders of margin,
//!   M-6), and nothing is carried per direction across sessions.
//!
//! **The estimator reads what the correction changes, upstream by
//! construction.** Stage P.1's finding binds this file: anything applied in
//! the fragment shader is invisible to the band, because the band samples the
//! decoded planes (docs/research/seam-blending.md 21). Here that is the whole
//! of the stability story rather than a hazard: the estimator is a pure
//! function of the decoded source, the correction is applied downstream at
//! the crossfade, there is no feedback path and nothing to oscillate. The
//! loop is OPEN by construction, and the only dynamics anywhere in the
//! mechanism are one first-order ease and one seek re-seed.
//!
//! **Off by default, behind [`KJERAG_CHROMATIC`].** With the switch off the
//! pooling pass is never dispatched, the field stays at the zero the buffer
//! was created with, and the fragment shader's early-out returns the very
//! expression the picture was drawn with before this file existed - the
//! byte-identity of the null instrument, by equality rather than by trusting
//! `exp(0.0)`-class arithmetic.

use std::sync::OnceLock;

use super::band::{AZIMUTHS, SMOOTH_DEG};

/// Where the spreading kernel reaches zero, in degrees of world angle from
/// the seam great circle.
///
/// Measured on Studio's own output, both oracle pairs: the normalized odd
/// profile decays to zero by thirty degrees at every azimuth of both scenes
/// (chromatic.md M-8.3). Degrees on the sphere and NEVER output pixels: the
/// same kernel is 145 px at one end of the owner's render and 471 px at the
/// other, and "doesn't seem to be a fixed distance from the seam" is exactly
/// what a fixed angular kernel looks like through a reframe whose scale
/// varies 3.6 times.
pub const EDGE_DEG: f32 = 30.0;

/// The kernel is `cos^3` of the angle scaled to reach zero at [`EDGE_DEG`],
/// which puts its half-max half-width at 12.48 degrees - inside the 11.82 to
/// 13.30 the two pairs measure (M-8.3, half-max rows) - with no second
/// constant to keep in step with the first. Smooth to its edge: the first and
/// second derivatives of `cos^3` both vanish at the zero.
const KERNEL_POWER: i32 = 3;

/// The runaway guard per channel: the widest local pull that is a lens
/// difference rather than the estimator coming apart, as a fraction of the
/// level it sits on.
///
/// M-4's derivation, kept as measured: the widest fitted chroma coordinate
/// over the corpus is 0.0475 ln (hard mode), times four is **0.19 ln**, 21
/// percent of hue. M-4 offers 0.087 for an estimator that pools with `lit`
/// squared alone; this field's per-direction support carries real local
/// structure of 0.044 to 0.111 ln (M-7, `between`), which 0.087 would clip
/// and 0.19 admits whole. Nothing measured is clipped by it, which is what
/// keeps it a guard rather than a knob.
pub const LIMIT_CHROMA_LN: f32 = 0.19;

/// A chroma reading on content darker than this is noise, not a measurement:
/// the level floor, in the planes' own 0..1 scale.
///
/// Eight codes, the measuring phase's own floor (`--bin colour`,
/// `LEVEL_FLOOR`), under which M-3 found the dark half's disagreement running
/// four times the signal. Fail-upward: a patch under it keeps what it had and
/// loses the evidence behind it, which is less correction, never more.
pub const LEVEL_FLOOR: f32 = 8.0 / 255.0;

/// BT.709, the same matrix the fragment shader's `ycbcr` applies
/// (`scene.rs`). Written once here and emitted into the WGSL below, with the
/// Rust mirror [`rgb_of`] reading the same four names, so the estimator
/// converts its means through the very arithmetic the draw converts its
/// samples through. The draw's own copy in `ycbcr` is untouched - it is the
/// shipped picture - and `tests::the_matrix_is_the_fragment_shaders` holds
/// the two to each other.
const CR_R: f32 = 1.5748;
const CB_G: f32 = -0.1873;
const CR_G: f32 = -0.4681;
const CB_B: f32 = 1.8556;

/// BT.709's luma weights: what [`field_target`] projects OUT of every
/// reading, so the field is luminance-neutral by construction - the memo's
/// 3.1 discipline, kept for the seam-local field and measured to be needed:
/// with the luma component left in, the along-seam smoothing carries the
/// bright near-horizon cells' luma offset onto the dark dirt azimuths and
/// the dirt view's common-mode step across the seam GREW by 1.3 codes
/// (R -4.22 to -5.57, G -3.90 to -4.84, B -5.08 to -6.61 at the fov-60
/// dirt reference, measured 2026-08-09) - a level step laid across the
/// handover, which is the artifact class P.1 was refused over
/// (docs/research/seam-blending.md 19). Luminance is stage 3's business
/// and stays with the pooled gain; what this field closes is the HUE step,
/// which is the owner's complaint and Studio's dominant axis (chromatic.md
/// 2.2: the chroma carries about twice what the luminance does).
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// One direction's chroma reading: the two lenses' mean picture of the same
/// patch, per channel, and the evidence behind it.
///
/// Means per LENS rather than one difference, so the pooling can apply the
/// tone split the draw will actually apply before it differences them
/// ([`field_target`]): what the field corrects is what the drawn picture
/// still disagrees by after stage 3's gain, not what the raw planes do.
///
/// In the video's own gamma-coded scale, 0..1, like every photometric
/// quantity in this campaign: no transfer function is assumed at either end.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChromaCell {
    /// Lens 0's mean R, G, B over the patch's unclipped pairs.
    pub m0: [f32; 3],
    /// Lens 1's, over the same pairs.
    pub m1: [f32; 3],
    /// Lens 0's mean luma over the same pairs, the weight's own level:
    /// the pooling weighs a direction by `evidence * lit * lit`, which is the
    /// least-squares weight the tone pool measured best on nine captures and
    /// M-3 kept ("keep `lit` squared for any pooled term"; the fork's whole
    /// disagreement was the dark half's own noise, and a dark-reaching weight
    /// recovers noise four times the signal).
    pub lit: f32,
    /// How much of the patch answered, 0 to 1, eased in time; what decays
    /// when a direction stops being readable. Zero is a direction that was
    /// never read, and zero evidence is zero correction by arithmetic.
    pub evidence: f32,
    /// Which population the reading came from, kept distinct as the
    /// measurement phase kept them (M-1.1): 0 none, [`POP_CORRELATED`] read
    /// at the winning shift, [`POP_ZERO_SHIFT`] read at zero shift - the
    /// never-correlated and flat directions, 77 to 89 percent of the ring on
    /// real views, which agree with the far field to 0.002 to 0.007 ln
    /// wherever both speak (M-5), so pooling them buys evidence and not bias.
    pub population: f32,
    /// Storage-buffer stride padding; see `Tone`'s own note.
    pub _pad: [f32; 3],
}

/// The reading came from a correlated direction, at the shift that made the
/// two patches the same content.
pub const POP_CORRELATED: f32 = 1.0;

/// The reading came from a direction the geometry refused - never correlated,
/// or flat under the contrast gate - read at zero shift per chromatic.md
/// 4.3's rule: a direction may be refused for the geometry and still be read
/// for the colour, where what a misregistration costs is proportional to the
/// content's own gradient.
pub const POP_ZERO_SHIFT: f32 = 2.0;

/// The header the fragment shader reads before it reads the field: the arm's
/// scale, and how much of the ring is behind the field.
///
/// Zero scale is the OFF state and the state a buffer is created in, and the
/// lookup's first test; it is only ever written by the pooling pass, which is
/// only ever dispatched with the arm on.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Chromatic {
    /// What the applied field is multiplied by: 0 off, 1 the estimate, 2 the
    /// sensitivity arm (`KJERAG_CHROMATIC=double`, memo 7.2's A/B shape).
    pub scale: f32,
    /// The share of the ring with evidence behind it, 0 to 1. Never applied;
    /// what an instrument reads.
    pub evidence: f32,
    pub _pad: [f32; 2],
}

// ------------------------------------------------------------ the switch

/// The variable [`arm`] reads. OFF by default in this increment: the owner's
/// eye gates the arm on, and nothing in the player reaches it.
pub const KJERAG_CHROMATIC: &str = "KJERAG_CHROMATIC";

/// The arm, read once: 0.0 off (the default), 1.0 on, 2.0 double.
///
/// The same shape as `projection::anchoring`: a `OnceLock`, one line to the
/// terminal explaining a non-default arm, and empty-or-unset is the default
/// rather than an answer.
pub fn arm() -> f32 {
    static SCALE: OnceLock<f32> = OnceLock::new();
    *SCALE.get_or_init(|| {
        let Ok(asked) = std::env::var(KJERAG_CHROMATIC) else {
            return 0.0;
        };
        match asked.to_ascii_lowercase().as_str() {
            "" | "0" | "off" => 0.0,
            "1" | "on" => {
                println!(
                    "blend:  research chromatic seam correction ON, {KJERAG_CHROMATIC}={asked}: \
                     each lens is pulled toward the local per-channel mean of the two, half the \
                     current frame's own overlap disagreement each, on a fixed angular kernel \
                     that is zero by {EDGE_DEG} degrees from the seam"
                );
                1.0
            }
            "2" | "double" => {
                println!(
                    "blend:  research chromatic seam correction at DOUBLE size, \
                     {KJERAG_CHROMATIC}={asked}: the sensitivity arm, twice the estimate"
                );
                2.0
            }
            _ => {
                eprintln!(
                    "blend:  {KJERAG_CHROMATIC}={asked} is not an arm this build knows \
                     (off, on, double) and is read as OFF, which is the fail-upward direction"
                );
                0.0
            }
        }
    })
}

// ------------------------------------------------------------ the twins

/// The spreading kernel at `off` radians from the seam great circle: 1 on the
/// circle, half at 12.48 degrees, exactly zero at and past [`EDGE_DEG`].
///
/// WGSL twin: `chromatic_kernel`.
pub fn kernel(off: f32) -> f32 {
    let edge = EDGE_DEG.to_radians();
    match off.abs() < edge {
        true => (off.abs() / edge * std::f32::consts::FRAC_PI_2)
            .cos()
            .powi(KERNEL_POWER),
        false => 0.0,
    }
}

/// An angle brought into `-PI..PI`, which is what makes the along-seam
/// smoothing wrap. The same arithmetic as the band's own `wrapped`.
///
/// WGSL twin: `chromatic_wrap`.
pub fn wrap(angle: f32) -> f32 {
    let turn = std::f32::consts::TAU;
    (angle + std::f32::consts::PI).rem_euclid(turn) - std::f32::consts::PI
}

/// The along-seam smoothing weight at `apart` radians of azimuth: a raised
/// cosine of half-width [`SMOOTH_DEG`], zero at and past it.
///
/// The ring's own smoothing scale, reused rather than invented: the same
/// raised cosine the along-seam table smooths with, holding about nine of the
/// ring's 2.8-degree cells. It is what keeps the field smooth along azimuth -
/// the stage-7/8 lesson, per-direction offsets over wide support paint noise -
/// while amplitude structure slower than about twelve degrees survives, which
/// is the scale the oracle's own amplitude walks at (chromatic.md 2.3: 3.8,
/// 4.4, 5.0, 4.2, 2.8, 1.6, 1.3 codes without oscillating).
///
/// WGSL twin: `chromatic_smooth`.
pub fn smooth(apart: f32) -> f32 {
    let width = SMOOTH_DEG.to_radians();
    match apart.abs() < width {
        true => 0.5 * (1.0 + (std::f32::consts::PI * apart / width).cos()),
        false => 0.0,
    }
}

/// Where a body-frame ray sits against the seam ring: the cell index below
/// it, the fraction toward the next, and the unsigned angle off the seam
/// great circle in radians.
///
/// The seam ring is the body frame's own z = 0 circle (`Ring::of`), so the
/// off-seam angle is `asin(|z|)` of the normalized ray and the azimuth is
/// `atan2(y, x)`, exactly `Table::at`'s resolution of the same circle. A ray
/// with no length answers the edge, which the lookup refuses.
///
/// WGSL twin: `chromatic_cell`.
pub fn cell_of(body: [f32; 3]) -> (i32, f32, f32) {
    let len = (body[0] * body[0] + body[1] * body[1] + body[2] * body[2]).sqrt();
    if len <= 0.0 {
        return (0, 0.0, EDGE_DEG.to_radians());
    }
    let off = (body[2].abs() / len).clamp(0.0, 1.0).asin();
    let turn = body[1].atan2(body[0]) / std::f32::consts::TAU * AZIMUTHS as f32;
    let low = turn.floor();
    (low as i32, turn - low, off)
}

/// The correction one lens receives at a ray, given the field's two
/// neighbouring entries: the interpolated amplitude, on the kernel, halved
/// (each lens carries half, equal and opposite), at the arm's scale.
///
/// WGSL twin: `chromatic_reach`.
pub fn reach(e0: [f32; 3], e1: [f32; 3], mix: f32, off: f32, scale: f32) -> [f32; 3] {
    let weight = 0.5 * scale * kernel(off);
    std::array::from_fn(|c| (e0[c] + (e1[c] - e0[c]) * mix) * weight)
}

/// A mean in the planes' own raw units decoded to gamma-coded R, G, B: the
/// levels, then BT.709 - the same affine map `ycbcr` applies to a sample,
/// applied to a mean, which commutes because the map is affine.
///
/// Eight-bit paths only in this increment: the estimator refuses `wide`
/// (P010) frames outright, because the band's own `luma_at` reads such planes
/// a byte at a time and a reading through half a word is not a measurement.
/// The owner's whole photometric reference corpus is 8-bit `.insv`.
///
/// WGSL twin: `chromatic_rgb`.
pub fn rgb_of(raw: [f32; 3], limited: bool) -> [f32; 3] {
    let (luma, chroma) = match limited {
        false => ([1.0, 0.0], [1.0, -0.5]),
        true => (
            [255.0 / 219.0, -16.0 / 219.0],
            [255.0 / 224.0, -128.0 / 224.0],
        ),
    };
    let y = raw[0] * luma[0] + luma[1];
    let cb = raw[1] * chroma[0] + chroma[1];
    let cr = raw[2] * chroma[0] + chroma[1];
    [y + CR_R * cr, y + CB_G * cb + CR_G * cr, y + CB_B * cb]
}

/// A per-channel disagreement with its luminance projected out: what is
/// left carries `LUMA . d = 0` exactly, so the field cannot brighten or
/// darken anything, cannot double-correct with the pooled gain, and cannot
/// lay a level step across the handover (see [`LUMA`]).
///
/// WGSL twin: `chromatic_neutral`.
pub fn neutral(d: [f32; 3]) -> [f32; 3] {
    let luma: f32 = (0..3).map(|c| LUMA[c] * d[c]).sum();
    d.map(|v| v - luma)
}

/// The field the pooling pass would assemble from these readings, before the
/// temporal ease: per entry, the smoothed weighted mean of the ring's
/// tone-adjusted per-channel disagreements, luminance projected out of each,
/// shrunk by its own evidence.
///
/// **Rust twin of the `pool_chroma` entry point's per-entry loop**, and a twin
/// rather than a description: the pass solves this on the GPU where no test
/// can reach it, and every property claimed for it is claimed about a
/// function `cargo test` can call with no device and no footage
/// (`chromatic_guard` then holds the two to each other on a real GPU).
///
/// The pieces, each with its measurement:
/// - `d = t1 * m1 - t0 * m0`: the disagreement of what the draw will actually
///   composite, after the same clamped tone split `tone_split` applies, so
///   the field and stage 3's gain cannot correct the same thing twice.
///   Expressed in codes, which is what carries M-2's finding - gain AND
///   offset, the offset the bigger half (23 of 24 channel fits) - without a
///   family choice: a pull toward the local mean reproduces whatever the
///   local disagreement is (M-9.3).
/// - clamped per channel at [`LIMIT_CHROMA_LN`] of the level it sits on.
/// - weighted `evidence * lit * lit` (M-3's verdict), on the raised cosine.
/// - shrunk by `E / (E + RIDGE)` where `E` is the kernel-weighted evidence in
///   directions: the band's own fail-upward rule, `RIDGE` one direction's
///   worth, so an entry with nothing behind it is exactly zero and thin
///   support is less correction, never more.
pub fn field_target(cells: &[ChromaCell], tone_log_gain: f32) -> Vec<[f32; 3]> {
    let split = tone_pair(tone_log_gain);
    (0..cells.len())
        .map(|index| {
            let phi = index as f32 / cells.len() as f32 * std::f32::consts::TAU;
            let mut value = [0.0f32; 3];
            let mut weight = 0.0f32;
            let mut evidence = 0.0f32;
            for (other, cell) in cells.iter().enumerate() {
                if cell.evidence <= 0.0 {
                    continue;
                }
                let at = other as f32 / cells.len() as f32 * std::f32::consts::TAU;
                let near = smooth(wrap(at - phi));
                if near <= 0.0 {
                    continue;
                }
                let w = near * cell.evidence * cell.lit * cell.lit;
                let limit = LIMIT_CHROMA_LN * cell.lit;
                let d = neutral(std::array::from_fn(|channel| {
                    (split[1] * cell.m1[channel] - split[0] * cell.m0[channel]).clamp(-limit, limit)
                }));
                for (channel, slot) in value.iter_mut().enumerate() {
                    *slot += w * d[channel];
                }
                weight += w;
                evidence += near * cell.evidence;
            }
            match weight > 0.0 {
                true => {
                    let shrink = evidence / (evidence + super::band::RIDGE);
                    value.map(|v| v / weight * shrink)
                }
                false => [0.0; 3],
            }
        })
        .collect()
}

/// The same clamped symmetric split `Tone::split` computes, from the raw log
/// gain: what lens 0 and lens 1 are multiplied by. One expression, two twins
/// (`Tone::split` on the Rust side of the draw, `tone_split` in WGSL), and
/// re-derived here rather than imported because this file's copy takes the
/// gain as a number where `Tone::split` takes the struct the readback built.
fn tone_pair(log_gain: f32) -> [f32; 2] {
    let half = 0.5 * log_gain.clamp(-super::band::LIMIT_LN, super::band::LIMIT_LN);
    match half == 0.0 {
        true => [1.0, 1.0],
        false => [half.exp(), (-half).exp()],
    }
}

/// The whole fragment-side lookup, mirrored: what is ADDED to the composited
/// picture at a view ray, per channel, given the field and the normalized
/// blend weights - `(w0 - w1)` times the per-lens half-pull.
///
/// This is the number the twin guard compares against the GPU, and the number
/// every property below is proved on: zero wherever the field is zero (the
/// null, by equality), zero at the 50/50 line whatever the field holds (the
/// oracle's dark-line fingerprint), and zero past [`EDGE_DEG`] (the compact
/// support).
///
/// WGSL twin: `chromatic_half` (the lookup glue in `band::LOOKUP`) times the
/// weight difference `picture` applies.
pub fn pull(
    reframe: &super::projection::Reframe,
    field: &[[f32; 3]],
    scale: f32,
    view_ray: [f32; 3],
) -> [f32; 3] {
    if scale == 0.0 {
        return [0.0; 3];
    }
    let body = reframe.body_ray(view_ray);
    let (low, mix, off) = cell_of(body);
    if off >= EDGE_DEG.to_radians() {
        return [0.0; 3];
    }
    let entry = |step: i32| field[(low + step).rem_euclid(field.len() as i32) as usize];
    reach(entry(0), entry(1), mix, off, scale)
}

// ------------------------------------------------------------ the shader

/// The WGSL these constants and functions exist twice as: pure functions
/// only, no bindings and no entry points, concatenated into the draw module,
/// the band module and the twin guard's probe so all three read one emission.
///
/// `CHROMATIC_CELLS` repeats [`AZIMUTHS`] under its own name because this
/// string is also compiled where the band's halves - which declare
/// `AZIMUTHS` - are not (the twin guard), and a module-scope constant may be
/// declared once.
pub(crate) fn wgsl() -> String {
    format!(
        "const CHROMATIC_CELLS = {AZIMUTHS}u;\n\
         const CHROMATIC_EDGE = {edge:?};\n\
         const CHROMATIC_SMOOTH = {smooth:?};\n\
         const CHROMATIC_LIMIT = {LIMIT_CHROMA_LN:?};\n\
         const CHROMATIC_FLOOR = {LEVEL_FLOOR:?};\n\
         const CHROMATIC_TURN = {turn:?};\n\
         const CHROMATIC_HALF_PI = {half_pi:?};\n\
         const CHROMATIC_PI = {pi:?};\n\
         const CR_R = {CR_R:?};\n\
         const CB_G = {CB_G:?};\n\
         const CR_G = {CR_G:?};\n\
         const CB_B = {CB_B:?};\n\
         const LUMA_R = {lr:?};\n\
         const LUMA_G = {lg:?};\n\
         const LUMA_B = {lb:?};\n\
         {WGSL}",
        edge = EDGE_DEG.to_radians(),
        lr = LUMA[0],
        lg = LUMA[1],
        lb = LUMA[2],
        smooth = SMOOTH_DEG.to_radians(),
        turn = std::f32::consts::TAU,
        half_pi = std::f32::consts::FRAC_PI_2,
        pi = std::f32::consts::PI,
    )
}

const WGSL: &str = r#"
// The spreading kernel: 1 on the seam great circle, half at 12.48 degrees,
// exactly zero at and past CHROMATIC_EDGE. cos^3, smooth to its own edge.
// Rust twin: `chromatic::kernel`.
fn chromatic_kernel(off: f32) -> f32 {
  let apart = abs(off);
  if apart >= CHROMATIC_EDGE {
    return 0.0;
  }
  let c = cos(apart / CHROMATIC_EDGE * CHROMATIC_HALF_PI);
  return c * c * c;
}

// An angle brought into -PI..PI. Rust twin: `chromatic::wrap`.
fn chromatic_wrap(angle: f32) -> f32 {
  let shifted = angle + CHROMATIC_PI;
  return shifted - floor(shifted / CHROMATIC_TURN) * CHROMATIC_TURN - CHROMATIC_PI;
}

// The along-seam smoothing weight: a raised cosine of half-width
// CHROMATIC_SMOOTH, zero at and past it. Rust twin: `chromatic::smooth`.
fn chromatic_smooth(apart: f32) -> f32 {
  if abs(apart) >= CHROMATIC_SMOOTH {
    return 0.0;
  }
  return 0.5 * (1.0 + cos(CHROMATIC_PI * apart / CHROMATIC_SMOOTH));
}

// Where a body-frame ray sits against the seam ring: (cell below, fraction
// toward the next, unsigned angle off the seam circle in radians). The seam
// ring is the body's own z = 0 circle. Rust twin: `chromatic::cell_of`.
fn chromatic_cell(body: vec3<f32>) -> vec3<f32> {
  let len = length(body);
  if len <= 0.0 {
    return vec3<f32>(0.0, 0.0, CHROMATIC_EDGE);
  }
  let off = asin(clamp(abs(body.z) / len, 0.0, 1.0));
  let turn = atan2(body.y, body.x) / CHROMATIC_TURN * f32(CHROMATIC_CELLS);
  let low = floor(turn);
  return vec3<f32>(low, turn - low, off);
}

// The correction one lens receives at a ray, from the field's two
// neighbouring entries: interpolated, on the kernel, halved, at the arm's
// scale. Rust twin: `chromatic::reach`.
fn chromatic_reach(e0: vec3<f32>, e1: vec3<f32>, mix: f32, off: f32, scale: f32) -> vec3<f32> {
  return (e0 + (e1 - e0) * mix) * (0.5 * scale * chromatic_kernel(off));
}

// A per-channel disagreement with its luminance projected out, so the field
// cannot brighten or darken anything and cannot double-correct with the
// pooled gain: the memo's 3.1 discipline kept for the local field. Rust
// twin: `chromatic::neutral`.
fn chromatic_neutral(d: vec3<f32>) -> vec3<f32> {
  let luma = LUMA_R * d.x + LUMA_G * d.y + LUMA_B * d.z;
  return d - vec3<f32>(luma);
}

// A mean in the planes' own raw units decoded to gamma-coded R, G, B: the
// levels, then the same BT.709 the fragment shader's `ycbcr` applies to a
// sample. Affine, so a converted mean is the mean of converted samples.
// Eight-bit levels only: the estimator refuses `wide` frames before this
// runs. Rust twin: `chromatic::rgb_of`.
fn chromatic_rgb(raw: vec3<f32>, limited: f32) -> vec3<f32> {
  var luma = vec2<f32>(1.0, 0.0);
  var chroma = vec2<f32>(1.0, -0.5);
  if limited > 0.5 {
    luma = vec2<f32>(255.0 / 219.0, -16.0 / 219.0);
    chroma = vec2<f32>(255.0 / 224.0, -128.0 / 224.0);
  }
  let y = raw.x * luma.x + luma.y;
  let cb = raw.y * chroma.x + chroma.y;
  let cr = raw.z * chroma.x + chroma.y;
  return vec3<f32>(
    y + CR_R * cr,
    y + CB_G * cb + CR_G * cr,
    y + CB_B * cb,
  );
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32, within: f32) {
        assert!((a - b).abs() <= within, "{a} against {b}");
    }

    /// The kernel is the measured shape: 1 on the line, half-max at 12.48
    /// degrees (inside the oracle pairs' 11.82 to 13.30), zero at and past
    /// 30, and it never rises.
    #[test]
    fn the_kernel_is_the_measured_shape() {
        near(kernel(0.0), 1.0, 1e-6);
        near(kernel(12.48f32.to_radians()), 0.5, 2e-3);
        assert_eq!(kernel(EDGE_DEG.to_radians()), 0.0);
        assert_eq!(kernel(45f32.to_radians()), 0.0);
        assert_eq!(kernel(-45f32.to_radians()), 0.0);
        let mut last = f32::INFINITY;
        for step in 0..=300 {
            let now = kernel((step as f32 * 0.1f32).to_radians());
            assert!(now <= last + 1e-6, "the kernel rises at {step}");
            last = now;
        }
    }

    /// The null is an equality: a zero field, or a zero scale, or a ray past
    /// the edge, each puts exactly nothing on the picture.
    #[test]
    fn the_null_is_an_equality() {
        let zero = [[0.0f32; 3]; AZIMUTHS];
        let one = {
            let mut field = [[0.0f32; 3]; AZIMUTHS];
            field[7] = [0.01, -0.02, 0.03];
            field
        };
        assert_eq!(reach([0.0; 3], [0.0; 3], 0.3, 0.1, 1.0), [0.0; 3]);
        assert_eq!(reach(one[7], one[8], 0.5, 0.1, 0.0), [0.0; 3]);
        assert_eq!(
            reach(one[7], one[8], 0.5, EDGE_DEG.to_radians(), 1.0),
            [0.0; 3]
        );
        for entry in zero {
            assert_eq!(entry, [0.0; 3]);
        }
    }

    /// The smoothing cannot stripe at the cell scale: a single-cell spike is
    /// spread over the kernel's nine-cell window and no adjacent pair of
    /// entries steps by more than the kernel's own slope allows, while a
    /// well-supported constant field comes back as itself, shrunk only by the
    /// ridge.
    #[test]
    fn the_smoothing_is_smooth_and_keeps_a_constant() {
        let mut cells = vec![ChromaCell::default(); AZIMUTHS];
        for cell in &mut cells {
            cell.lit = 0.4;
            cell.evidence = 1.0;
            cell.m0 = [0.30, 0.30, 0.30];
            cell.m1 = [0.32, 0.30, 0.28];
        }
        let field = field_target(&cells, 0.0);
        // Every direction weighs the same, so the target is the constant
        // disagreement (0.02, 0, -0.02), luminance projected out, shrunk by
        // E/(E+1) with E the summed kernel-weighted evidence.
        let e: f32 = (0..AZIMUTHS)
            .map(|j| smooth(wrap(j as f32 / AZIMUTHS as f32 * std::f32::consts::TAU)))
            .sum();
        let shrink = e / (e + crate::band::RIDGE);
        let wanted = neutral([0.02, 0.0, -0.02]);
        for entry in &field {
            near(entry[0], wanted[0] * shrink, 1e-4);
            near(entry[1], wanted[1] * shrink, 1e-4);
            near(entry[2], wanted[2] * shrink, 1e-4);
        }

        // One loud cell in an otherwise silent, evidence-free ring: the spike
        // spreads over the window and tapers by the raised cosine, with no
        // step anywhere near the spike's own size.
        let mut spiked = vec![ChromaCell::default(); AZIMUTHS];
        spiked[64].lit = 0.4;
        spiked[64].evidence = 1.0;
        spiked[64].m0 = [0.30; 3];
        spiked[64].m1 = [0.34, 0.30, 0.30];
        let field = field_target(&spiked, 0.0);
        let peak = field[64][0];
        assert!(peak > 0.0);
        // One reading's evidence against the ridge takes over half of it.
        assert!(peak < 0.04 * 0.55, "{peak} of a 0.04 spike survives");
        for pair in field.windows(2) {
            let step = (pair[1][0] - pair[0][0]).abs();
            assert!(step < peak * 0.5, "a {step} step beside a {peak} peak");
        }
    }

    /// Fail-upward: less evidence is less correction, never more, and no
    /// evidence is exactly zero.
    #[test]
    fn thin_evidence_shrinks_the_correction() {
        let reading = |evidence: f32| {
            let mut cells = vec![ChromaCell::default(); AZIMUTHS];
            for cell in &mut cells {
                cell.lit = 0.4;
                cell.evidence = evidence;
                cell.m0 = [0.30; 3];
                cell.m1 = [0.34, 0.30, 0.30];
            }
            field_target(&cells, 0.0)[0][0]
        };
        let full = reading(1.0);
        let half = reading(0.5);
        let thin = reading(0.05);
        assert!(full > half && half > thin && thin > 0.0);
        assert_eq!(reading(0.0), 0.0);
    }

    /// The guard clamps a runaway reading at [`LIMIT_CHROMA_LN`] of the level
    /// it sits on, and clips nothing the corpus measured (the widest local
    /// structure is 0.111 ln, M-7).
    #[test]
    fn the_guard_is_wider_than_anything_measured() {
        const _: () = assert!(LIMIT_CHROMA_LN > 0.111);
        let mut cells = vec![ChromaCell::default(); AZIMUTHS];
        for cell in &mut cells {
            cell.lit = 0.2;
            cell.evidence = 1.0;
            cell.m0 = [0.10; 3];
            // A whole tenth of full scale of disagreement on 0.2 of level:
            // far past the guard.
            cell.m1 = [0.20, 0.10, 0.10];
        }
        let field = field_target(&cells, 0.0);
        let e: f32 = (0..AZIMUTHS)
            .map(|j| smooth(wrap(j as f32 / AZIMUTHS as f32 * std::f32::consts::TAU)))
            .sum();
        let shrink = e / (e + crate::band::RIDGE);
        near(
            field[0][0],
            neutral([LIMIT_CHROMA_LN * 0.2, 0.0, 0.0])[0] * shrink,
            1e-4,
        );
    }

    /// The field is a hue and carries no level: a disagreement that is all
    /// luminance - every channel moved together - pools to exactly nothing,
    /// and what any reading contributes is luminance-free to the weights'
    /// own arithmetic.
    #[test]
    fn the_field_carries_no_luminance() {
        let mut cells = vec![ChromaCell::default(); AZIMUTHS];
        for cell in &mut cells {
            cell.lit = 0.4;
            cell.evidence = 1.0;
            cell.m0 = [0.30; 3];
            // Four codes of the same sign on every channel: a level step,
            // stage 3's business and not this field's.
            cell.m1 = [0.30 + 4.0 / 255.0; 3];
        }
        for entry in field_target(&cells, 0.0) {
            for channel in entry {
                assert!(channel.abs() < 1e-7, "{channel} of level leaked");
            }
        }
        // And a hue survives the projection with its luma taken out: the
        // planted (+2, 0, -2) keeps its R-B opposition.
        let hue = neutral([2.0 / 255.0, 0.0, -2.0 / 255.0]);
        let luma: f32 = (0..3).map(|c| LUMA[c] * hue[c]).sum();
        assert!(luma.abs() < 1e-9);
        assert!(hue[0] > 0.0 && hue[2] < 0.0);
    }

    /// The pooling differences what the draw composites - the tone split is
    /// applied to the two means before they are differenced, with the same
    /// clamp `Tone::split` keeps - and the luminance projection makes an
    /// achromatic disagreement invisible to the field whatever the gain
    /// says: two mechanisms, two axes, no double correction on either.
    #[test]
    fn an_achromatic_step_is_no_business_of_the_fields() {
        let mut cells = vec![ChromaCell::default(); AZIMUTHS];
        for cell in &mut cells {
            cell.lit = 0.4;
            cell.evidence = 1.0;
            // The two lenses differ by exactly a gain of e^0.01 on every
            // channel: an achromatic difference the tone split owns.
            cell.m0 = [0.30; 3];
            cell.m1 = [0.30 * (0.01f32).exp(); 3];
        }
        for tone in [0.0, 0.01] {
            let field = field_target(&cells, tone);
            for channel in field[0] {
                assert!(channel.abs() < 1e-6, "{channel} of level leaked");
            }
        }
        // And a hue rides through the tone term essentially unchanged: the
        // split scales each lens by under a percent, which moves a hue by
        // its second order and no more.
        for cell in &mut cells {
            cell.m1 = [0.30 + 2.0 / 255.0, 0.30, 0.30 - 2.0 / 255.0];
        }
        let still = field_target(&cells, 0.0);
        let toned = field_target(&cells, 0.05);
        for channel in 0..3 {
            let apart = (still[0][channel] - toned[0][channel]).abs();
            assert!(
                apart < 0.03 * (2.0 / 255.0),
                "the tone term moved a hue by {apart}",
            );
        }
    }

    /// The matrix here is the fragment shader's: the same four BT.709
    /// numbers `ycbcr` multiplies by, checked as values so a drift in either
    /// copy fails a test rather than a picture.
    #[test]
    fn the_matrix_is_the_fragment_shaders() {
        assert_eq!([CR_R, CB_G, CR_G, CB_B], [1.5748, -0.1873, -0.4681, 1.8556]);
        // Full range: mid grey with centred chroma is grey through the whole
        // affine map.
        let grey = rgb_of([0.5, 0.5, 0.5], false);
        for channel in grey {
            near(channel, 0.5, 1e-6);
        }
        // Limited range: luma 16 with centred chroma at 128 is black, which
        // is the studio-swing convention `levels` keeps.
        let black = rgb_of([16.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0], true);
        for channel in black {
            near(channel, 0.0, 1e-6);
        }
    }

    /// The lookup respects the geometry: on the seam circle the kernel is
    /// full, at the pole it is zero, and the azimuth resolves to the cell the
    /// ring names - the same convention `Table::at` resolves.
    #[test]
    fn the_lookup_reads_the_ring_the_band_writes() {
        // Straight along +x: azimuth 0, on the circle.
        let (low, mix, off) = cell_of([1.0, 0.0, 0.0]);
        assert_eq!(low, 0);
        near(mix, 0.0, 1e-6);
        near(off, 0.0, 1e-6);
        // A quarter turn: cell 32 of 128.
        let (low, mix, _) = cell_of([0.0, 1.0, 0.0]);
        assert_eq!(low, 32);
        near(mix, 0.0, 1e-4);
        // The pole: 90 degrees off the circle, refused by the kernel.
        let (_, _, off) = cell_of([0.0, 0.0, 1.0]);
        near(off, std::f32::consts::FRAC_PI_2, 1e-6);
        assert_eq!(kernel(off), 0.0);
        // Negative azimuth wraps the way the table wraps.
        let (low, mix, _) = cell_of([1.0, -0.1, 0.0]);
        let entry = (low).rem_euclid(AZIMUTHS as i32);
        assert!(entry > 120, "{entry}");
        assert!((0.0..1.0).contains(&mix));
    }
}
