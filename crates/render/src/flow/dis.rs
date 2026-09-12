//! A faithful CPU reference of Dense Inverse Search (DIS) optical flow,
//! reproducing Insta360 Studio's `bcv::flow::FDSFlow`
//! (docs/research/studio-seam-re.md §35), which the reverse engineering
//! proved is a vendored copy of OpenCV's `DISOpticalFlow` (Kroeger et al.
//! 2016, "Fast Optical Flow using Dense Inverse Search") plus a variational
//! refinement stage. This is chunk 3 of the Studio-optical-flow rebuild:
//! CPU-first for verifiability, structured so a WGSL port is a transcription
//! and not a redesign (plain array math, no iterator chains that will not
//! translate).
//!
//! **Faithfulness note.** The algorithm here follows the *structure* of
//! OpenCV's `DISOpticalFlowImpl` and `VariationalRefinementImpl`, because
//! §35 proved FDSFlow *is* that code. The numeric constants FDSFlow was
//! recovered with are catalogued on [`DisConfig`], but their ROLES are
//! inferred: until a gdb read of the FDSFlow object resolves which offset is
//! which parameter, the OpenCV defaults are the baseline and every knob is a
//! named, documented field rather than a silent literal.
//!
//! **What is faithful to OpenCV specifically** (not to a generic DIS):
//! - the inverse-search Hessian is precomputed from the *reference* image's
//!   (I0's) gradients — inverse-compositional Lucas–Kanade, so the per-patch
//!   `2×2` inverse is built once and every descent step is a matrix-vector
//!   multiply (`processPatch`);
//! - patch residuals are *mean-normalized* (`processPatchMeanNorm`): the
//!   per-patch mean brightness difference is removed before the gradient dot
//!   product, which is `use_mean_normalization`;
//! - densification weights each covering patch by `1/max(1, |I0 - I1(warp)|)`
//!   evaluated *at the pixel*, not by a whole-patch SSD (`Densification`);
//! - the variational stage is Brox-style brightness + gradient constancy with
//!   a Charbonnier penalty, solved by Red-Black SOR (`RedBlackSOR`, §35's
//!   `RedBlackSOR_ParBody`).
//!
//! The input strips carry a NEGATIVE sentinel where a lens has no picture of a
//! ray (`band::strip`). Sentinels are treated as invalid everywhere: excluded
//! from structure tensors and SSD sums, and a patch that is majority-sentinel
//! is marked invalid rather than matched against noise. Invalidity propagates
//! through the pyramid and out to the flow field so a caller can paint
//! no-flow distinctly from zero-flow.

/// The DIS parameter set. Every field is a named knob with its OpenCV default
/// and its §35 note, so nothing in the algorithm is a bare literal.
///
/// **§35 recovered these offsets/values, roles INFERRED** (see the module
/// header). A future gdb read of the FDSFlow object at
/// `[owner+0x1e00]+0x1f0` resolves which offset is which; until then the
/// OpenCV defaults below are what runs, and the §35 candidates are recorded
/// beside the field they most plausibly are:
///
/// | §35 offset | value | plausible role (UNREAD) |
/// |-----------:|------:|-------------------------|
/// | `+0x18`    | 60    | working width or border |
/// | `+0x1c`    | 3     | coarsest scale count?   |
/// | `+0x44`    | 0.25f | pyramid downscale ratio |
/// | `+0x48`    | 0.75f | blend/overrelax factor  |
/// | `+0xd4`    | 20    | α (smoothness) or iters |
/// | `+0x5f8`   | 10    | γ (gradient constancy)  |
/// | `+0x58`    | 0.37f | edge/gradient threshold |
/// | `+0x780`   | 0.97f | forward/back consistency|
/// | `+0x784`   | 4.0f  | consistency distance    |
///
/// The two that line up with an OpenCV default by value — `+0xd4=20` == α
/// and `+0x5f8=10` == γ — are the only ones treated as more than coincidence,
/// and even those are left as the OpenCV default rather than "applied from
/// §35", because α and γ *equal* their §35 candidate so nothing is lost by
/// sourcing them from the algorithm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisConfig {
    /// Finest pyramid level the flow is computed on. Level 0 is full input
    /// resolution; level `k` is downsampled by `2^k`. OpenCV default `2`
    /// (the flow is computed at 1/4 resolution and upsampled), which is what
    /// makes DIS fast. A caller wanting sub-pixel accuracy on a known
    /// translation drops this to `0`.
    pub finest_scale: usize,
    /// Side of a square patch, in pixels of the level it is searched on.
    /// OpenCV default `8`.
    pub patch_size: usize,
    /// Grid stride between patch centres, in pixels. OpenCV default `4` — a
    /// 50% overlap at `patch_size = 8`.
    pub patch_stride: usize,
    /// Inverse-search Gauss–Newton iterations per patch. OpenCV default `16`.
    pub grad_descent_iters: usize,
    /// Outer fixed-point (warp) iterations of the variational refinement per
    /// pyramid level. OpenCV default `5`. Zero disables refinement.
    pub variational_iters: usize,
    /// Hard ceiling on the computed coarsest pyramid scale — Studio's FDSFlow
    /// override `[+0x10] = 2` (studio-seam-re.md §44.3, base-FDSFlow ctor
    /// `0x18419d130` / `0x183b21476`). `None` is OpenCV's own behaviour: the
    /// coarsest is whatever [`Self::coarsest_scale`]'s log2 rule computes.
    /// `Some(2)` caps it, so a 162/180-tall belt runs the two levels `{2, 1}`
    /// with `finest_scale = 1` (½-res finest) instead of OpenCV's `{0..4}`.
    /// This is the second half of §43's fix: the belt's overlap band is a few
    /// rows tall and a deeper pyramid downsamples it below a patch.
    pub coarsest_cap: Option<usize>,
    /// Inner Red-Black SOR sweeps per outer variational iteration. OpenCV's
    /// `VariationalRefinement` default `5`.
    pub sor_iters: usize,
    /// Smoothness weight α. OpenCV default `20.0`; §35 `+0xd4 = 20` is the
    /// same value.
    pub alpha: f32,
    /// Brightness-constancy data weight δ. OpenCV default `5.0`.
    pub delta: f32,
    /// Gradient-constancy data weight γ. OpenCV default `10.0`; §35
    /// `+0x5f8 = 10` is the same value.
    pub gamma: f32,
    /// Successive over-relaxation factor ω for the SOR sweeps. OpenCV uses
    /// `1.6`.
    pub omega: f32,
    /// Subtract each patch's mean brightness difference before the gradient
    /// dot product (`processPatchMeanNorm`). OpenCV default `true`.
    pub use_mean_normalization: bool,
    /// Seed each patch from its already-solved left/top neighbour when that
    /// gives a lower SSD (`use_spatial_propagation`). OpenCV default `true`.
    pub use_spatial_propagation: bool,
    /// Fraction of a patch that may be sentinel before the patch is abandoned
    /// as no-picture. Not an OpenCV parameter (OpenCV has no sentinel); it is
    /// this port's handling of the strip's no-picture rows. Half.
    pub max_sentinel_fraction: f32,
    /// Studio's mono-texture (aperture) reliability flag (§47.1, HARD): flag a
    /// belt (grid) row whose mean per-patch `Jxx/Jyy > `[`MONO_TEXTURE_RATIO`]`
    /// (4.0)` — the two diagonal structure-tensor components (8×8 sums of squared
    /// Sobel-3 gradients), NOT eigenvalues; its patches are dropped from the
    /// density vote. ON in the faithful default.
    pub gate_mono: bool,
    /// Studio's lack-of-texture reliability flag (§47.2, HARD): flag a belt (grid)
    /// row whose mean 8×8-summed gradient energy over valid patches (floor 1.0) is
    /// `< `[`LACK_OF_TEXTURE_ENERGY`]` (2000.0)`, in Studio's uint8 luma units; its
    /// patches are dropped from the density vote. ON in the faithful default.
    pub gate_lowtex: bool,
    /// Studio's small-disparity reliability flag (§45.2, block mask): flag an
    /// 8×8/stride-3 block whose small-disparity byte-sum `>= 7` (fraction
    /// [`SMALL_DISPARITY_FRAC`]); its patch is dropped from the density vote.
    ///
    /// **ON in the faithful default** (build #26, FIX C — the §47 gate). §4.x:
    /// `UpdateRowsOfSmallDisparityCuda` runs at the END of the coarse-to-fine
    /// loop and its mask is RETAINED for the NEXT calculation (`+0xe70` is
    /// "absent during the first calculation"), so on a still / first frame
    /// Studio's own mask contributes nothing. When set here it is instead
    /// computed IN-FRAME from the current finest field — a disclosed diagnostic
    /// departure from Studio's temporal retention (§4.x) — so it takes effect on
    /// a still; the per-pixel source is the CPU-INFERRED [`SMALL_DISPARITY_PX`].
    pub gate_small: bool,
}

impl Default for DisConfig {
    /// Studio's FDSFlow parameter block, RE'd HARD from the base-FDSFlow
    /// matcher ctor `0x18419d130` and its selector-4 overrides
    /// (studio-seam-re.md §44.3): `finest_scale = 1` (not OpenCV's 2 — §43's
    /// exact ¼-res collapse where the belt's thin overlap band dies),
    /// `patch_stride = 3` (not 4), `coarsest_cap = Some(2)` (`[+0x10] = 2`).
    /// `patch_size = 8`, `grad_descent_iters = 16`, `variational_iters = 5`
    /// are unchanged from OpenCV and match Studio. The synthetic-translation
    /// tests drop `finest_scale` to `0` for their sub-pixel bar.
    fn default() -> Self {
        Self {
            finest_scale: 1,
            patch_size: 8,
            patch_stride: 3,
            grad_descent_iters: 16,
            variational_iters: 5,
            coarsest_cap: Some(2),
            sor_iters: 5,
            alpha: 20.0,
            delta: 5.0,
            gamma: 10.0,
            omega: 1.6,
            use_mean_normalization: true,
            use_spatial_propagation: true,
            max_sentinel_fraction: 0.5,
            // The §47 gate (build #26): the corrected metrics, all three flags ON
            // (FIX C). §46's raw-λmax/λmin mono flag was pathological; §47.1
            // decoded the real metric as `Jxx/Jyy` (8×8 patch sum of squared
            // Sobel-3 gradients, mean-of-ratios per row > 4.0), which stays under
            // 4 on ordinary 2-D texture and exceeds it only on the mono-directional
            // vignette/skirt. Lack-of-texture is the RE'd `2000.0` on Studio's
            // uint8 scale (§47.2), not the retired `0.02` guess. Small-disparity is
            // the RE'd 8×8/stride-3/frac-0.1 block geometry (§45.2); on a still it
            // is computed IN-FRAME from the current finest field (a disclosed
            // diagnostic departure from Studio's NEXT-frame retention, §4.x). All
            // constants are HARD; nothing here is tuned to a seam target.
            gate_mono: true,
            gate_lowtex: true,
            gate_small: true,
        }
    }
}

/// A dense 2-D flow field: for each pixel of the input, the displacement
/// `(u, v)` such that `I1(x + u, y + v) ≈ I0(x, y)`.
///
/// `u` always displaces the input's column and `v` its row. On the legacy belt
/// those happen to mean along-strip/azimuth and across-strip/epipolar. The
/// selected ONE X2 grid reverses those physical axis meanings, so its boundary
/// immediately wraps this generic result as route-specific `dcol` across and
/// `drow` along fields. `valid[i]` is false where no patch could vote for the
/// pixel (all covering patches were sentinel), so a reader can tell no-flow
/// from zero-flow.
#[derive(Clone, Debug)]
pub struct FlowField {
    pub width: usize,
    pub height: usize,
    pub u: Vec<f32>,
    pub v: Vec<f32>,
    pub valid: Vec<bool>,
    /// Per-pixel fit residual carried WITH the field so a temporal warm-start
    /// can gate the re-search-SKIP (§75). This is the mean-normalized patch SSD
    /// (`patch_ssd`) in Studio's uint8-luma units (× [`GATE_LUMA_SCALE`]²) of the
    /// coarsest-level patch that produced this pixel's flow — the residual
    /// Studio stores per patch alongside the hint (`updateInverseSearchFlag`
    /// arg3). `f32::INFINITY` marks "no valid residual" (Studio's `0x501502f9`
    /// sentinel role): such a pixel's hint is never trusted, so it re-searches.
    /// Every zero/constant field leaves it `+inf`, so a caller that ignores the
    /// warm-start (cold still, flow-OFF) is unaffected.
    pub residual: Vec<f32>,
}

impl FlowField {
    fn zeros(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            u: vec![0.0; width * height],
            v: vec![0.0; width * height],
            valid: vec![false; width * height],
            residual: vec![f32::INFINITY; width * height],
        }
    }

    /// Bilinearly resample this field to `(width, height)` and scale the
    /// vectors by the same ratio, so a field solved at one pyramid level
    /// becomes the initial guess at the next finer one (×2 size, ×2
    /// magnitude in the isotropic case). Invalidity is carried by nearest
    /// sampling: an upsampled pixel is valid if its nearest source was.
    fn resample(&self, width: usize, height: usize) -> Self {
        let mut out = Self::zeros(width, height);
        let sx = self.width as f32 / width as f32;
        let sy = self.height as f32 / height as f32;
        // The vector-magnitude scale is the inverse of the size ratio: a
        // displacement measured in coarse pixels is that many fine pixels
        // times (fine/coarse).
        let mag_x = width as f32 / self.width as f32;
        let mag_y = height as f32 / self.height as f32;
        for y in 0..height {
            for x in 0..width {
                let fx = (x as f32 + 0.5) * sx - 0.5;
                let fy = (y as f32 + 0.5) * sy - 0.5;
                let (u, v) = self.sample_bilinear(fx, fy);
                let out_i = y * width + x;
                out.u[out_i] = u * mag_x;
                out.v[out_i] = v * mag_y;
                let nx = fx.round().clamp(0.0, self.width as f32 - 1.0) as usize;
                let ny = fy.round().clamp(0.0, self.height as f32 - 1.0) as usize;
                out.valid[out_i] = self.valid[ny * self.width + nx];
                // Residual is an SSD, not a displacement: carry it by NEAREST
                // (never magnitude-scaled, never bilinear — a bilinear tap onto
                // an `+inf` invalid neighbour would poison a valid pixel). §75.
                out.residual[out_i] = self.residual[ny * self.width + nx];
            }
        }
        out
    }

    /// Nearest-resample only the [`Self::residual`] channel to `(width, height)`,
    /// for the warm-start re-search-SKIP reference at a pyramid level (§75). The
    /// residual is an SSD, never magnitude-scaled and never bilinear (an `+inf`
    /// invalid tap must not poison a valid neighbour).
    fn resample_residual(&self, width: usize, height: usize) -> Vec<f32> {
        let mut out = vec![f32::INFINITY; width * height];
        let sx = self.width as f32 / width as f32;
        let sy = self.height as f32 / height as f32;
        for y in 0..height {
            for x in 0..width {
                let fx = (x as f32 + 0.5) * sx - 0.5;
                let fy = (y as f32 + 0.5) * sy - 0.5;
                let nx = fx.round().clamp(0.0, self.width as f32 - 1.0) as usize;
                let ny = fy.round().clamp(0.0, self.height as f32 - 1.0) as usize;
                out[y * width + x] = self.residual[ny * self.width + nx];
            }
        }
        out
    }

    fn sample_bilinear(&self, fx: f32, fy: f32) -> (f32, f32) {
        let x0 = fx.floor().clamp(0.0, self.width as f32 - 1.0) as usize;
        let y0 = fy.floor().clamp(0.0, self.height as f32 - 1.0) as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let tx = (fx - x0 as f32).clamp(0.0, 1.0);
        let ty = (fy - y0 as f32).clamp(0.0, 1.0);
        let mix = |a: &[f32]| {
            let a00 = a[y0 * self.width + x0];
            let a10 = a[y0 * self.width + x1];
            let a01 = a[y1 * self.width + x0];
            let a11 = a[y1 * self.width + x1];
            let top = a00 * (1.0 - tx) + a10 * tx;
            let bot = a01 * (1.0 - tx) + a11 * tx;
            top * (1.0 - ty) + bot * ty
        };
        (mix(&self.u), mix(&self.v))
    }
}

/// A single-channel image with a per-pixel validity mask. Sentinel (no
/// picture) pixels are `valid = false`; their `data` is a neutral zero so
/// arithmetic that forgets to gate does not read a huge negative sentinel.
#[derive(Clone)]
struct Plane {
    width: usize,
    height: usize,
    data: Vec<f32>,
    valid: Vec<bool>,
}

impl Plane {
    /// Split an input strip (negative == sentinel) into value + validity.
    fn from_sentinel(src: &[f32], width: usize, height: usize) -> Self {
        let mut data = vec![0.0; width * height];
        let mut valid = vec![false; width * height];
        for i in 0..width * height {
            if src[i] >= 0.0 {
                data[i] = src[i];
                valid[i] = true;
            }
        }
        Self {
            width,
            height,
            data,
            valid,
        }
    }

    #[inline]
    fn at(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.width + x]
    }

    #[inline]
    fn ok(&self, x: usize, y: usize) -> bool {
        self.valid[y * self.width + x]
    }

    /// Bilinear sample with validity: returns `None` if any of the four taps
    /// needed is sentinel or out of bounds, so a warp that reaches into
    /// no-picture is rejected rather than reading zero.
    #[inline]
    fn sample(&self, fx: f32, fy: f32) -> Option<f32> {
        if fx < 0.0 || fy < 0.0 || fx > (self.width - 1) as f32 || fy > (self.height - 1) as f32 {
            return None;
        }
        let x0 = fx.floor() as usize;
        let y0 = fy.floor() as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        if !(self.ok(x0, y0) && self.ok(x1, y0) && self.ok(x0, y1) && self.ok(x1, y1)) {
            return None;
        }
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;
        let top = self.at(x0, y0) * (1.0 - tx) + self.at(x1, y0) * tx;
        let bot = self.at(x0, y1) * (1.0 - tx) + self.at(x1, y1) * tx;
        Some(top * (1.0 - ty) + bot * ty)
    }
}

/// The five-tap 1-D Gaussian (`σ ≈ 1`) whose separable square is the 5×5
/// blur §35 marks HARD as applied to the strips before the pyramid, and the
/// same kernel this port downsamples with. Normalized: `[1,4,6,4,1]/16`.
const GAUSS5: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];

/// Charbonnier penalty derivative `Ψ'(s²) = 1 / (2·√(s² + ε²))` — the robust
/// weight the variational data and smoothness terms are reweighted by. OpenCV
/// uses `ε² = 1e-6` inside its `VariationalRefinement`.
const CHARBONNIER_EPS_SQ: f32 = 1e-6;

// ----------------------------------------------------------------------------
// Studio's FDSFlow structure-tensor RELIABILITY GATE (studio-seam-re.md §45).
//
// Run at the FINEST processed pyramid level, INSIDE the densification (§45.2),
// this flags degenerate rows/blocks — mono-texture (aperture), lack-of-texture,
// and small-disparity — so the mask-aware densification drops their patches and
// the flagged pixels fill from valid neighbours + the variational smoothness.
// It is the one faithful layer §45.3 marked kjerag as missing: our photometric
// densification weight `1/max(1,|I0-I1|)` does NOT down-weight a flat/dark patch
// matched at a WRONG disparity (dark-matches-dark keeps `|I0-I1|` low), so the
// single-lens skirt overshoot survives without a STRUCTURE-based reject.
//
// §45.1 CORRECTIONS carried through: there is NO forward-backward consistency
// threshold and NO max-flow magnitude cap in Studio (the `0.97f`/`4.0f` engine
// bytes are vestigial / the aperture ratio, not those). So this gate is the
// ONLY reliability mechanism added; no FB reject, no flow cap.
// ----------------------------------------------------------------------------

/// Mono-texture (aperture) ratio threshold, **HARD** from Studio's
/// `PrecomputeStructureTensor` consumer `fcn.1841a9320`, the `4.0f` at read-only
/// `0x184666138` (`0x1841a955a`, studio-seam-re.md §47.1). The metric it gates is
/// `Jxx/Jyy` — the two DIAGONAL structure-tensor components, NOT eigenvalues:
///
/// - `Jxx = Σ over an 8×8 patch of (∂I/∂x)²`, `Jyy = Σ over 8×8 of (∂I/∂y)²`,
///   with `∂I/∂x`,`∂I/∂y` **Sobel ksize-3** gradients of the reference belt image
///   `I0` at the finest processed level (`cv::spatialGradient`, `0x1841a6ff6`), on
///   the **stride-3** patch grid. No ε on the denominator (`0x1841a56c7`/`…6da`).
/// - Per belt (grid) row: `mean = mean over patches p with (Jxx>0 && Jyy>0) of
///   Jxx(p)/Jyy(p)` — **mean-of-per-patch-ratios** (`divss` `0x1841a9486`), NOT
///   ratio-of-sums. Flag the row when `mean > 4.0`. A row with zero valid patches
///   (all Jxx≤0 or Jyy≤0) is NOT mono-flagged — that is lack-of-texture's job.
///
/// **Why bounded (fixes §46, the raw-λmax/λmin pathology):** per-pixel `Iy²≈0` on
/// a horizontally-coherent belt edge sends a raw eigenvalue ratio to 1e12 and
/// flags every row; the 8×8 SUM `Jyy` is nonzero for any patch with y-texture, so
/// `Jxx/Jyy` stays under 4 on ordinary 2-D texture and exceeds 4 only where the
/// patch is genuinely mono-directional. On the belt `∂I/∂x` is along-seam and
/// `∂I/∂y` across-seam (§37), so `Jxx/Jyy>4` = "this row lacks the across-seam
/// gradient that constrains the across-seam disparity" = the vignette/skirt. The
/// ratio is scale-invariant, so 4.0 holds at any luma scale.
pub const MONO_TEXTURE_RATIO: f32 = 4.0;

/// Small-disparity block fraction, **HARD** from `GenerateBlockMaskKel`'s
/// `fraction_threshold`, the `0.1f` (`cd cc cc 3d`) at `0x18474581c`
/// (studio-seam-re.md §45.2 / §4.x). Over an 8×8 window at stride 3 (the patch
/// grid), a block is a small-disparity block when the mean of its 64 source
/// bytes is `>= 0.1`, i.e. the integer byte-sum `>= 7` (`ceil(0.1·64)` — the
/// exact RE'd cutoff for extent 8). See [`SMALL_DISPARITY_PX`] for the per-pixel
/// source, which is the CPU-INFERRED half of this flag.
pub const SMALL_DISPARITY_FRAC: f32 = 0.1;

/// The 8×8 window / stride-3 geometry of `GenerateBlockMaskKel` (§45.2): extent
/// `accelerator+0x14 = 8`, stride `+0x18 = 3` — identical to `patch_size` /
/// `patch_stride`, so a small-disparity *block* is one *patch* footprint.
const SMALL_DISPARITY_EXTENT: usize = 8;

/// Per-pixel "small disparity" threshold in belt pixels — the source byte
/// `GenerateBlockMaskKel` sums. **CPU-INFERRED, disclosed, NOT a Studio-read
/// value:** Studio's per-pixel small-disparity map is produced upstream into a
/// retained device buffer (`+0xd28`/`+0xe70`, §4.x) whose exact definition is
/// UNREAD. A pixel here is "small disparity" when its nearest patch's flow
/// magnitude is below this (essentially no parallax). Only used when
/// [`DisConfig::gate_small`] is set, which is ON by default (see that field).
const SMALL_DISPARITY_PX: f32 = 0.5;

/// Lack-of-texture per-patch validity floor — Studio's `valid_threshold`,
/// `1.0f` (`0x1841a9278`, `0x18460b998`), read by the lack consumer
/// `fcn.1841a9160` (studio-seam-re.md §47.2). A grid-patch joins the row's mean
/// energy only when its 8×8-summed gradient energy exceeds this, so all-sentinel
/// / all-black patches (energy 0) are excluded. This floor is on Studio's
/// **uint8 luma [0,255]** scale, the same scale [`GATE_LUMA_SCALE`] puts the
/// gate's gradients on, so the `1.0` is faithful with no rescaling.
const LACK_OF_TEXTURE_VALID: f64 = 1.0;

/// The luma scale the gate's Sobel gradients are computed on: Studio's
/// `cv::spatialGradient` runs on **uint8 luma [0,255]** (§47.3), but kjerag's
/// belt strips carry luma in `[0,1]` (`band.rs` `CLIP_HIGH = 252/255` etc.). The
/// mono ratio `Jxx/Jyy` is scale-invariant so it is indifferent to this, but the
/// lack-of-texture energy threshold [`LACK_OF_TEXTURE_ENERGY`] `= 2000.0` is
/// scale-DEPENDENT (§47.3): the gate therefore multiplies each `[0,1]` gradient
/// by `255` before squaring (equivalently scales the summed energy by `255²`), so
/// the box-summed energy is in Studio's exact uint8 units and `2000.0` applies
/// directly. This is option (a) of §47.3, done at the gradient so both metrics
/// share one Sobel-3 pass.
const GATE_LUMA_SCALE: f32 = 255.0;

/// Lack-of-texture row energy threshold, **HARD** from the CPU lack consumer
/// `fcn.1841a9160`: flag a belt (grid) row when its mean 8×8-summed gradient
/// energy over valid patches (floor [`LACK_OF_TEXTURE_VALID`] `= 1.0`) is
/// `< 2000.0f` (`0x1841a92a2`, `0x184779cd8`, studio-seam-re.md §47.2). §45.2/§46
/// mis-scoped this to GPU-only and guessed `0.02` on `[0,1]` luma — that was
/// WRONG (§47.2): it is the SAME `2000.0`, on Studio's **uint8 [0,255]** scale.
///
/// **Units (the trap, §47.3).** `2000.0` assumes `cv::spatialGradient` (ksize-3
/// Sobel) on uint8 luma with an 8×8 SUM (not mean) of per-pixel energy `Ix²+Iy²`.
/// kjerag's belt luma is `[0,1]`, `1/255` of Studio's, so the raw `[0,1]` energy
/// would be `(1/255)² ≈ 1.5e-5×` Studio's — which is exactly why the old `0.02`
/// guess was a no-op. The gate closes the gap at the gradient via
/// [`GATE_LUMA_SCALE`] (×255 per gradient ⇒ ×255² per energy), so the energy is
/// in Studio's units and this `2000.0` is faithful with no per-threshold rescale.
///
/// The reliability gate reads it directly (§47.2): the RE'd value, not the
/// retired `0.02` guess.
pub const LACK_OF_TEXTURE_ENERGY: f32 = 2000.0;

/// The residual-gated re-search-SKIP per-row threshold (studio-seam-re.md §75),
/// RE'd from Android `libarvbmg.so`. Studio's `FDSFlow::prepareBuffers`
/// (0x676c8e4) initializes the per-row threshold vector `ctx[0x470]` to `1.0f`
/// (fill `0x3f800000`, 0x676ce88) and nothing else writes it, so the per-row
/// threshold is the CONSTANT `1.0`. The warm-start re-search flag
/// (`updateInverseSearchFlag` 0x6770708) is set — i.e. a patch is re-searched —
/// only when `patch_SSD − hint_residual > 1.0`; otherwise the hint is KEPT.
///
/// **Units.** `1.0` is on Studio's uint8-luma SSD. kjerag's belt luma is `[0,1]`,
/// so the re-search residual is computed in uint8 units (× [`GATE_LUMA_SCALE`]²
/// per the same §47.3 rule the energy gate uses) and this `1.0` applies directly.
/// The re-search gate reads it directly (§75).
pub const RE_SEARCH_THRESHOLD: f32 = 1.0;

/// A separable 5×5 Gaussian that respects validity: each output is the
/// validity-weighted average of the in-bounds valid taps, and a pixel with no
/// valid support stays sentinel. Two passes (rows then columns) over a shared
/// scratch, so it stays a plain-loop stencil a shader can mirror.
fn gaussian_blur_5(plane: &Plane) -> Plane {
    let (w, h) = (plane.width, plane.height);
    // Horizontal pass.
    let mut hx = vec![0.0f32; w * h];
    let mut hv = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            if !plane.ok(x, y) {
                continue;
            }
            let mut sum = 0.0;
            let mut wsum = 0.0;
            for (k, weight) in GAUSS5.iter().enumerate() {
                let xx = x as isize + k as isize - 2;
                if xx < 0 || xx >= w as isize {
                    continue;
                }
                let xx = xx as usize;
                if plane.ok(xx, y) {
                    sum += weight * plane.at(xx, y);
                    wsum += weight;
                }
            }
            if wsum > 0.0 {
                hx[y * w + x] = sum / wsum;
                hv[y * w + x] = true;
            }
        }
    }
    // Vertical pass over the horizontal result.
    let mut out = vec![0.0f32; w * h];
    let mut ov = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            if !hv[y * w + x] {
                continue;
            }
            let mut sum = 0.0;
            let mut wsum = 0.0;
            for (k, weight) in GAUSS5.iter().enumerate() {
                let yy = y as isize + k as isize - 2;
                if yy < 0 || yy >= h as isize {
                    continue;
                }
                let yy = yy as usize;
                if hv[yy * w + x] {
                    sum += weight * hx[yy * w + x];
                    wsum += weight;
                }
            }
            if wsum > 0.0 {
                out[y * w + x] = sum / wsum;
                ov[y * w + x] = true;
            }
        }
    }
    Plane {
        width: w,
        height: h,
        data: out,
        valid: ov,
    }
}

/// Downsample by 2 with the same validity-weighted 5-tap Gaussian: blur then
/// take even samples. Output size is `ceil(w/2) × ceil(h/2)`.
fn downsample(plane: &Plane) -> Plane {
    let blurred = gaussian_blur_5(plane);
    let w = plane.width.div_ceil(2);
    let h = plane.height.div_ceil(2);
    let mut data = vec![0.0f32; w * h];
    let mut valid = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let sx = (2 * x).min(plane.width - 1);
            let sy = (2 * y).min(plane.height - 1);
            if blurred.ok(sx, sy) {
                data[y * w + x] = blurred.at(sx, sy);
                valid[y * w + x] = true;
            }
        }
    }
    Plane {
        width: w,
        height: h,
        data,
        valid,
    }
}

/// Downsample a boolean coverage MASK by 2, matching [`downsample`]'s
/// even-sample geometry (take the pixel at `(2x, 2y)`), so the per-level mask
/// edge stays aligned with the belt colour at that level and the inward clip is
/// never regrown by a blur. Output size `ceil(w/2) × ceil(h/2)`. Kept SEPARATE
/// from the colour so the mask never pollutes the correlation footprint (§63).
fn downsample_mask(mask: &[bool], w: usize, h: usize) -> Vec<bool> {
    let nw = w.div_ceil(2);
    let nh = h.div_ceil(2);
    let mut out = vec![false; nw * nh];
    for y in 0..nh {
        for x in 0..nw {
            let sx = (2 * x).min(w - 1);
            let sy = (2 * y).min(h - 1);
            out[y * nw + x] = mask[sy * w + sx];
        }
    }
    out
}

/// Central-difference spatial gradients of a plane, gated by validity: a
/// gradient is only formed where both neighbours across it are valid,
/// otherwise it is zero and the location is treated as gradient-invalid.
/// Returns `(Ix, Iy, grad_valid)`.
fn gradients(plane: &Plane) -> (Vec<f32>, Vec<f32>, Vec<bool>) {
    let (w, h) = (plane.width, plane.height);
    let mut ix = vec![0.0f32; w * h];
    let mut iy = vec![0.0f32; w * h];
    let mut gv = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            if !plane.ok(x, y) {
                continue;
            }
            let xm = x.saturating_sub(1);
            let xp = (x + 1).min(w - 1);
            let ym = y.saturating_sub(1);
            let yp = (y + 1).min(h - 1);
            let mut ok = true;
            let gx = if plane.ok(xm, y) && plane.ok(xp, y) {
                0.5 * (plane.at(xp, y) - plane.at(xm, y))
            } else {
                ok = false;
                0.0
            };
            let gy = if plane.ok(x, ym) && plane.ok(x, yp) {
                0.5 * (plane.at(x, yp) - plane.at(x, ym))
            } else {
                ok = false;
                0.0
            };
            ix[y * w + x] = gx;
            iy[y * w + x] = gy;
            gv[y * w + x] = ok;
        }
    }
    (ix, iy, gv)
}

/// `cv::spatialGradient`-equivalent Sobel ksize-3 gradients of `plane` (§47.1,
/// `0x1841a6ff6`), used ONLY by the structure-tensor reliability gate — the DIS
/// inverse-search core keeps its own central-difference [`gradients`]. Two
/// differences make this the gate's own pass rather than a reuse of those:
///
/// - **Sobel-3, not central difference.** The unnormalized Sobel-3 masks (no 1/8,
///   matching OpenCV's raw `CV_16S` output):
///   `Kx = [[-1,0,1],[-2,0,2],[-1,0,1]]`, `Ky = Kxᵀ`.
/// - **Studio's uint8 luma scale.** kjerag's belt luma is `[0,1]`, Studio's
///   `spatialGradient` runs on uint8 `[0,255]`; each gradient is scaled by
///   [`GATE_LUMA_SCALE`] (×255) so the box-summed energy lands in Studio's units
///   and the `2000.0` lack threshold is faithful (§47.3). (The mono ratio
///   `Jxx/Jyy` is scale-free and indifferent to this.)
///
/// Border pixels reflect (`BORDER_REFLECT_101`, OpenCV's default); the gradient
/// is marked invalid where any of the 3×3 taps is a sentinel (no-picture) pixel,
/// so structure is never fabricated across the picture edge. Returns
/// `(Ix, Iy, valid)` on the uint8 scale.
fn sobel3_gradients(plane: &Plane) -> (Vec<f32>, Vec<f32>, Vec<bool>) {
    let (w, h) = (plane.width, plane.height);
    let mut ix = vec![0.0f32; w * h];
    let mut iy = vec![0.0f32; w * h];
    let mut valid = vec![false; w * h];
    // BORDER_REFLECT_101: index -1 -> 1, w -> w-2 (mirror without repeating the
    // edge). Offsets here are only ±1, so at most one fold is ever needed.
    let reflect = |i: isize, n: usize| -> usize {
        if n == 1 {
            return 0;
        }
        let m = (n - 1) as isize;
        let mut i = i;
        while i < 0 || i > m {
            if i < 0 {
                i = -i;
            }
            if i > m {
                i = 2 * m - i;
            }
        }
        i as usize
    };
    for y in 0..h {
        for x in 0..w {
            if !plane.ok(x, y) {
                continue;
            }
            // Gather the 3×3 neighbourhood (reflected at image borders); bail if
            // any tap is a sentinel — a gradient across the picture edge is not a
            // real belt gradient. `t[j][i]`: j is the row offset (0=y−1,1=y,2=y+1),
            // i the column offset (0=x−1,1=x,2=x+1).
            let mut t = [[0.0f32; 3]; 3];
            let mut all_valid = true;
            'gather: for (j, trow) in t.iter_mut().enumerate() {
                let yy = reflect(y as isize + j as isize - 1, h);
                for (i, tval) in trow.iter_mut().enumerate() {
                    let xx = reflect(x as isize + i as isize - 1, w);
                    if !plane.ok(xx, yy) {
                        all_valid = false;
                        break 'gather;
                    }
                    *tval = plane.at(xx, yy);
                }
            }
            if !all_valid {
                continue;
            }
            // Sobel-3 (right col − left col, and bottom row − top row, weighted
            // 1,2,1), then to Studio's uint8 luma scale.
            let gx = (t[0][2] + 2.0 * t[1][2] + t[2][2]) - (t[0][0] + 2.0 * t[1][0] + t[2][0]);
            let gy = (t[2][0] + 2.0 * t[2][1] + t[2][2]) - (t[0][0] + 2.0 * t[0][1] + t[0][2]);
            ix[y * w + x] = gx * GATE_LUMA_SCALE;
            iy[y * w + x] = gy * GATE_LUMA_SCALE;
            valid[y * w + x] = true;
        }
    }
    (ix, iy, valid)
}

/// One pyramid level: both frames at this scale, plus I0's gradients (the
/// inverse-search template gradients).
struct Level {
    i0: Plane,
    i1: Plane,
    i0x: Vec<f32>,
    i0y: Vec<f32>,
    grad_valid: Vec<bool>,
    /// Per-lens COVERAGE MASK at this pyramid level (§59/§61/§63,
    /// `FDSFlow::calc` arg4), if the caller supplied one. `m0` is the
    /// src/reference lens, `m1` the target lens. These are consulted ONLY by the
    /// `IsInMask` gate ([`DisFlow::in_mask`]) and the densification NaN
    /// ([`DisFlow::densify`]); they NEVER touch the belt COLOUR (`i0`/`i1`),
    /// which stays full to the 105.6° cap so a seam patch keeps a full-texture
    /// footprint (§63 BUG 1 fix). `None` ⇒ no clip (`IsInMask` null ⇒ true).
    m0: Option<Vec<bool>>,
    m1: Option<Vec<bool>>,
}

/// The DIS engine. Stateless apart from its config; `calc` allocates its own
/// pyramid per call.
#[derive(Clone, Debug)]
pub struct DisFlow {
    config: DisConfig,
}

impl DisFlow {
    pub fn new(config: DisConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &DisConfig {
        &self.config
    }

    /// Compute the flow from `i0` to `i1`, each a single-channel `width ×
    /// height` strip with a negative sentinel for no-picture. `hint`, if
    /// given, is a previous field (any size) used to warm-start the coarsest
    /// level, which is FDSFlow's `HintFlow` (§35).
    pub fn calc(
        &self,
        i0: &[f32],
        i1: &[f32],
        width: usize,
        height: usize,
        hint: Option<&FlowField>,
        masks: Option<(&[bool], &[bool])>,
    ) -> FlowField {
        assert_eq!(i0.len(), width * height);
        assert_eq!(i1.len(), width * height);
        if let Some((m0, m1)) = masks {
            assert_eq!(m0.len(), width * height);
            assert_eq!(m1.len(), width * height);
        }
        let cfg = &self.config;

        // §35 HARD: the strips are pre-blurred 5×5 before the pyramid. The belt
        // COLOUR is built to the full 105.6° cap (§58, `BELT_COS_CAP` unchanged)
        // and blurred here; the per-lens coverage MASK (below) is kept SEPARATE
        // and NEVER folded into the colour — Studio runs DIS on the whole belt
        // Mat and a seam patch sits mid-image with real texture above and below
        // (§63 BUG 1 fix). Clipping the colour to the mask made seam-band patches
        // majority-sentinel and starved the estimate at narrow extents; do not.
        let p0 = gaussian_blur_5(&Plane::from_sentinel(i0, width, height));
        let p1 = gaussian_blur_5(&Plane::from_sentinel(i1, width, height));

        // How many levels: OpenCV's coarsest scale is chosen so the coarsest
        // level is a few patches across. `coarsest = round(log2(2·dim /
        // (4·patch_size)))`, the min over the two dims, clamped so the finest
        // requested level exists.
        let coarsest = self.coarsest_scale(width, height);
        let finest = cfg.finest_scale.min(coarsest);

        // Build the pyramid, finest (0) first.
        let mut planes0 = vec![p0];
        let mut planes1 = vec![p1];
        for _ in 1..=coarsest {
            planes0.push(downsample(planes0.last().unwrap()));
            planes1.push(downsample(planes1.last().unwrap()));
        }

        // Per-lens COVERAGE MASK pyramid (§59/§61/§63) — SEPARATE from the colour
        // planes above, never folded into `Plane::valid`, so the belt colour
        // stays full and a seam patch keeps a full-texture footprint (§63 BUG 1).
        // Each level's mask is the even-sample downsample of the finer one,
        // aligned to the colour downsample's geometry so the inward clip is not
        // regrown by a blur. `None` throughout when no masks are supplied.
        let mut mask0_pyr: Vec<Option<Vec<bool>>> = vec![masks.map(|(m0, _)| m0.to_vec())];
        let mut mask1_pyr: Vec<Option<Vec<bool>>> = vec![masks.map(|(_, m1)| m1.to_vec())];
        for s in 1..=coarsest {
            let (pw, ph) = (planes0[s - 1].width, planes0[s - 1].height);
            mask0_pyr.push(
                mask0_pyr[s - 1]
                    .as_deref()
                    .map(|m| downsample_mask(m, pw, ph)),
            );
            mask1_pyr.push(
                mask1_pyr[s - 1]
                    .as_deref()
                    .map(|m| downsample_mask(m, pw, ph)),
            );
        }

        let levels: Vec<Level> = (0..=coarsest)
            .map(|s| {
                let (i0x, i0y, grad_valid) = gradients(&planes0[s]);
                Level {
                    i0: planes0[s].clone(),
                    i1: planes1[s].clone(),
                    i0x,
                    i0y,
                    grad_valid,
                    m0: mask0_pyr[s].clone(),
                    m1: mask1_pyr[s].clone(),
                }
            })
            .collect();

        // Coarse to fine.
        let mut flow: Option<FlowField> = None;
        for s in (finest..=coarsest).rev() {
            let level = &levels[s];
            let (lw, lh) = (level.i0.width, level.i0.height);
            let mut field = match &flow {
                // Finer initial guess from the level above, ×2 size & vectors.
                Some(prev) => prev.resample(lw, lh),
                // Coarsest: from the temporal hint if any, else zero.
                None => match hint {
                    Some(h) => h.resample(lw, lh),
                    None => FlowField::zeros(lw, lh),
                },
            };
            // The coverage-mask IsInMask gate is consulted in the search at every
            // pyramid level (§59/§61/§63, Studio-faithful). The
            // densification-output NaN in `densify` is separate — it always drops
            // uncovered OUTPUT pixels.
            //
            // The residual-gated re-search-SKIP (§75) runs at EVERY pyramid level
            // of a WARM-STARTED call — Studio's `PatchInverseSearch_ParBody`
            // consults the flag at each level of the motion pyramid (§60). The
            // reference is the temporal HINT's residual resampled to THIS level
            // (so seed and reference are level-consistent and the RE'd `thr = 1.0`
            // applies); it is independent of the intra-frame densified residual.
            // A cold call (no hint) leaves the reference `None`, so the gate is
            // inert and the pass is byte-identical to the un-warm-started build.
            // Pinning at the FINEST level is what actually stops the drift, since
            // that is where the ill-conditioned belt ramp is walked frame-to-frame.
            let hint_ref = hint.map(|h| h.resample_residual(lw, lh));
            let reskip = hint_ref.as_deref().map(|r| (RE_SEARCH_THRESHOLD, r));
            let patches = self.patch_search(level, &field, reskip);
            // The structure-tensor reliability gate runs at the FINEST processed
            // pyramid level, inside densification (§45.2): flagged rows/blocks'
            // patches are excluded from the vote so a degenerate (flat / aperture
            // / small-disparity) match cannot survive the photometric weight.
            let reliable = if s == finest {
                self.reliability(level, &patches)
            } else {
                vec![true; patches.len()]
            };
            // The per-pixel residual channel ([`FlowField::residual`], §75) is
            // written at the FINEST level — the field a later frame warm-starts
            // from is the finest OUTPUT, so storing the finest residual makes the
            // reference level-consistent when the hint is resampled back to each
            // level next frame.
            self.densify(level, &patches, &reliable, &mut field, s == finest);
            if cfg.variational_iters > 0 {
                self.variational(level, &mut field);
            }
            flow = Some(field);
        }

        // Upsample the finest computed level to full input resolution.
        let mut out = flow.unwrap_or_else(|| FlowField::zeros(width, height));
        if out.width != width || out.height != height {
            out = out.resample(width, height);
        }
        out
    }

    /// OpenCV's coarsest-scale rule, clamped to at least `finest_scale` and to
    /// levels that stay larger than a patch.
    fn coarsest_scale(&self, width: usize, height: usize) -> usize {
        let cfg = &self.config;
        let by = |dim: usize| {
            let v = ((2 * dim) as f32 / (4.0 * cfg.patch_size as f32)).log2() + 0.5;
            v.floor().max(0.0) as usize
        };
        let mut coarsest = by(width).min(by(height));
        // Studio caps the computed coarsest at `[+0x10] = 2` (§44.3): the belt
        // is a few rows tall in its overlap band and a deeper pyramid pushes
        // that band below a patch. Applied before the patch-fit clamp so the
        // cap, not the log2 rule, decides the top of the pyramid.
        if let Some(cap) = cfg.coarsest_cap {
            coarsest = coarsest.min(cap);
        }
        // Never coarser than a level that still holds a patch on both axes.
        while coarsest > cfg.finest_scale {
            let w = width >> coarsest;
            let h = height >> coarsest;
            if w >= cfg.patch_size && h >= cfg.patch_size {
                break;
            }
            coarsest -= 1;
        }
        coarsest.max(cfg.finest_scale)
    }
}

/// One patch's converged result: its flow and whether it matched at all. The
/// grid position is implicit in the patch's index, so densification recovers
/// which pixels it covers without the centre being stored.
struct Patch {
    u: f32,
    v: f32,
    valid: bool,
    /// The patch's mean-normalized SSD residual in Studio uint8-luma units
    /// (× [`GATE_LUMA_SCALE`]²), or `f32::INFINITY` when the patch never matched
    /// (sentinel / masked / degenerate). Densified into [`FlowField::residual`]
    /// so a later frame's warm-start can gate the re-search-SKIP (§75).
    residual: f32,
}

/// One grid row's structure-tensor gate verdict and the means behind it (§47):
/// the mono-texture and lack-of-texture flags [`DisFlow::reliability`] reads, and
/// the `Jxx/Jyy` ratio-mean / patch-energy-mean the measurement instruments
/// report. One per grid row (`ny = (h − ps)/stride + 1`).
#[derive(Clone, Copy)]
struct GridRowStat {
    mono: bool,
    lowtex: bool,
    energy_mean: f64,
    ratio_mean: f64,
}

impl DisFlow {
    /// Studio's `IsInMask` (0x67706d0): the SINGLE reference-pixel mutual coverage
    /// gate (§61/§63). `null ⇒ true` (no clip). Otherwise a patch is in-mask iff
    /// the src-lens mask `m0` is set at the patch anchor `(ox, oy)` AND the
    /// target-lens mask `m1` is set at the flow-displaced anchor `(ox+u, oy+v)`.
    /// One pixel per lens — NOT a footprint majority — so a patch whose 8×8
    /// footprint is mostly outside the mask is still kept (its footprint reads the
    /// full, unmasked belt colour). Studio samples `m1` at the flow-displaced dst;
    /// when that leaves the belt we fall back to the anchor (the disclosed first
    /// approximation) so a valid-src patch is not dropped merely because its seed
    /// points off-image.
    fn in_mask(&self, level: &Level, ox: usize, oy: usize, u: f32, v: f32) -> bool {
        let (w, h) = (level.i0.width, level.i0.height);
        // Src lens at the anchor: outside ⇒ abandon.
        match level.m0.as_deref() {
            Some(m0) if !m0[oy * w + ox] => return false,
            Some(_) => {}
            None => return true, // null ⇒ true (no clip)
        }
        // Target lens at the flow-displaced anchor.
        let Some(m1) = level.m1.as_deref() else {
            return true; // src set, no target clip
        };
        let dx = (ox as f32 + u).round();
        let dy = (oy as f32 + v).round();
        if dx < 0.0 || dy < 0.0 || dx > (w - 1) as f32 || dy > (h - 1) as f32 {
            // Seed points off the belt: fall back to the anchor (first approx).
            return m1[oy * w + ox];
        }
        m1[dy as usize * w + dx as usize]
    }

    /// The DIS core: overlapping patches on a `patch_stride` grid, each run
    /// through `grad_descent_iters` of inverse-compositional Lucas–Kanade
    /// (`processPatchMeanNorm`), warm-started from the densified upscale and
    /// optionally from an already-solved neighbour.
    fn patch_search(
        &self,
        level: &Level,
        field: &FlowField,
        reskip: Option<(f32, &[f32])>,
    ) -> Vec<Patch> {
        let cfg = &self.config;
        let ps = cfg.patch_size;
        let stride = cfg.patch_stride;
        let (w, h) = (level.i0.width, level.i0.height);
        if w < ps || h < ps {
            return Vec::new();
        }
        let nx = (w - ps) / stride + 1;
        let ny = (h - ps) / stride + 1;
        let mut patches: Vec<Patch> = Vec::with_capacity(nx * ny);

        for gy in 0..ny {
            for gx in 0..nx {
                let ox = gx * stride;
                let oy = gy * stride;
                let cx = ox as f32 + (ps as f32 - 1.0) * 0.5;
                let cy = oy as f32 + (ps as f32 - 1.0) * 0.5;

                // The seed this patch warm-starts from (densified upscale, or the
                // temporal hint / zero at the coarsest level). Also the flow the
                // target-lens mask is sampled displaced by, in the `IsInMask` gate.
                let (seed_u, seed_v) = field.sample_bilinear(cx, cy);

                // `IsInMask` (§61/§63, 0x67706d0): the per-lens coverage gate — a
                // SINGLE reference-pixel test, NOT a footprint-majority abandon
                // (§63 BUG 2). Keep the patch iff the src-lens mask is set at the
                // patch anchor AND the target-lens mask is set at the flow-displaced
                // anchor. A patch whose 8×8 footprint is mostly OUTSIDE the mask is
                // KEPT — its footprint still reads full (unmasked) belt colour, so
                // it matches on real texture rather than starving. `null ⇒ true`.
                // This kills the one-sided vignette-ramp seed (the anchor there is
                // outside the mask ⇒ abandoned, so spatial propagation never carries
                // it down a column, §51/§54) WITHOUT clipping the colour.
                if !self.in_mask(level, ox, oy, seed_u, seed_v) {
                    patches.push(Patch {
                        u: 0.0,
                        v: 0.0,
                        valid: false,
                        residual: f32::INFINITY,
                    });
                    continue;
                }

                // Precompute this patch's inverse Hessian from I0's gradients
                // over the valid taps (inverse-compositional: the template is
                // fixed, so this is built once per patch).
                let mut hxx = 0.0f32;
                let mut hxy = 0.0f32;
                let mut hyy = 0.0f32;
                let mut gx_sum = 0.0f32;
                let mut gy_sum = 0.0f32;
                let mut valid_count = 0usize;
                for j in 0..ps {
                    for i in 0..ps {
                        let px = ox + i;
                        let py = oy + j;
                        if !(level.i0.ok(px, py) && level.grad_valid[py * w + px]) {
                            continue;
                        }
                        let gxv = level.i0x[py * w + px];
                        let gyv = level.i0y[py * w + px];
                        hxx += gxv * gxv;
                        hxy += gxv * gyv;
                        hyy += gyv * gyv;
                        gx_sum += gxv;
                        gy_sum += gyv;
                        valid_count += 1;
                    }
                }
                // The NO-PICTURE sentinel guard (belt COLOUR, not the mask): a
                // patch that is majority no-picture — a ray neither lens sampled —
                // is abandoned rather than matched against absent content. This is
                // the strip's own −1 sentinel, independent of the coverage mask.
                let total = ps * ps;
                let sentinel_frac = 1.0 - valid_count as f32 / total as f32;
                if sentinel_frac > cfg.max_sentinel_fraction || valid_count < 3 {
                    patches.push(Patch {
                        u: 0.0,
                        v: 0.0,
                        valid: false,
                        residual: f32::INFINITY,
                    });
                    continue;
                }
                // Regularize (OpenCV adds a small ridge so a flat patch does
                // not invert to a huge step) and invert the 2×2.
                let reg = 1e-3 * (hxx + hyy) + 1e-6;
                let a = hxx + reg;
                let d = hyy + reg;
                let b = hxy;
                let det = a * d - b * b;
                if det.abs() < 1e-9 {
                    patches.push(Patch {
                        u: 0.0,
                        v: 0.0,
                        valid: false,
                        residual: f32::INFINITY,
                    });
                    continue;
                }
                let inv = det.recip();
                let (i11, i12, i22) = (d * inv, -b * inv, a * inv);

                // Candidate seeds: the densified field at the centre (computed
                // above for the mask gate), plus the left/top already-solved
                // neighbours (spatial propagation).
                let mut best_u = seed_u;
                let mut best_v = seed_v;
                let mut best_ssd = self
                    .patch_ssd(
                        level,
                        ox,
                        oy,
                        ps,
                        gx_sum,
                        gy_sum,
                        valid_count,
                        best_u,
                        best_v,
                    )
                    .unwrap_or(f32::INFINITY);

                // The residual-gated re-search-SKIP (§75, Studio
                // `updateInverseSearchFlag` 0x6770708). When a temporal warm-start
                // hint carries a reference residual at this patch, the free inverse
                // search runs ONLY where the hint no longer fits: re-search iff
                // `seed_SSD − hint_residual > threshold`; otherwise the hint is
                // PINNED — spatial propagation AND the descent are skipped, so the
                // §51 vignette skirt cannot be re-injected frame-over-frame (the
                // §73 runaway). `reskip` is `Some(thr)` only at the coarsest
                // (hint-seeded) level of a warm-started call; `None` elsewhere and
                // on every cold frame, so cold stays byte-identical. The residual
                // is compared in Studio's uint8-luma units (× GATE_LUMA_SCALE²), so
                // the RE'd `thr = 1.0` applies as read.
                let seed_res = best_ssd * GATE_LUMA_SCALE * GATE_LUMA_SCALE;
                if let Some((thr, ref_res)) = reskip {
                    let ci = (cy.round().clamp(0.0, (h - 1) as f32) as usize) * w
                        + (cx.round().clamp(0.0, (w - 1) as f32) as usize);
                    let hint_res = ref_res[ci];
                    if hint_res.is_finite() && seed_res - hint_res <= thr {
                        patches.push(Patch {
                            u: seed_u,
                            v: seed_v,
                            valid: true,
                            residual: seed_res,
                        });
                        continue;
                    }
                }
                if cfg.use_spatial_propagation {
                    let mut consider = |cand: Option<&Patch>| {
                        let Some(p) = cand.filter(|p| p.valid) else {
                            return;
                        };
                        let Some(ssd) = self.patch_ssd(
                            level,
                            ox,
                            oy,
                            ps,
                            gx_sum,
                            gy_sum,
                            valid_count,
                            p.u,
                            p.v,
                        ) else {
                            return;
                        };
                        if ssd < best_ssd {
                            best_ssd = ssd;
                            best_u = p.u;
                            best_v = p.v;
                        }
                    };
                    let idx = patches.len();
                    if gx > 0 {
                        consider(patches.get(idx - 1));
                    }
                    if gy > 0 {
                        consider(patches.get(idx - nx));
                    }
                }

                // Inverse-compositional descent.
                let mut u = best_u;
                let mut v = best_v;
                let mut cur_ssd = best_ssd;
                for _ in 0..cfg.grad_descent_iters {
                    // b = Σ ∇I0 · (diff - mean_diff), with diff = I1(warp) - I0.
                    let mut bx = 0.0f32;
                    let mut by = 0.0f32;
                    let mut diff_sum = 0.0f32;
                    let mut diff_sq = 0.0f32;
                    let mut left_bounds = false;
                    for j in 0..ps {
                        for i in 0..ps {
                            let px = ox + i;
                            let py = oy + j;
                            if !(level.i0.ok(px, py) && level.grad_valid[py * w + px]) {
                                continue;
                            }
                            let warp = level.i1.sample(px as f32 + u, py as f32 + v);
                            let Some(iw) = warp else {
                                left_bounds = true;
                                break;
                            };
                            let diff = iw - level.i0.at(px, py);
                            diff_sum += diff;
                            diff_sq += diff * diff;
                            bx += level.i0x[py * w + px] * diff;
                            by += level.i0y[py * w + px] * diff;
                        }
                        if left_bounds {
                            break;
                        }
                    }
                    if left_bounds {
                        break;
                    }
                    let mean = diff_sum / valid_count as f32;
                    let ssd = if cfg.use_mean_normalization {
                        bx -= gx_sum * mean;
                        by -= gy_sum * mean;
                        diff_sq - diff_sum * mean
                    } else {
                        diff_sq
                    };
                    // Reject a step that raised the residual (divergence).
                    if ssd > cur_ssd * 1.0001 && ssd > 1e-6 {
                        break;
                    }
                    cur_ssd = ssd;
                    // ΔU = Hinv · b, and U -= ΔU (Gauss–Newton on Σ diff²).
                    let du = i11 * bx + i12 * by;
                    let dv = i12 * bx + i22 * by;
                    let nu = u - du;
                    let nv = v - dv;
                    // Reject a step that would carry the patch centre out of
                    // the image, as OpenCV does.
                    if cx + nu < 0.0
                        || cy + nv < 0.0
                        || cx + nu > (w - 1) as f32
                        || cy + nv > (h - 1) as f32
                    {
                        break;
                    }
                    u = nu;
                    v = nv;
                    if du * du + dv * dv < 1e-6 {
                        break;
                    }
                }

                patches.push(Patch {
                    u,
                    v,
                    valid: true,
                    // The converged residual in Studio uint8-luma units (§75),
                    // carried so a later frame's warm-start can gate the skip.
                    residual: cur_ssd * GATE_LUMA_SCALE * GATE_LUMA_SCALE,
                });
            }
        }
        patches
    }

    /// The mean-normalized SSD of a patch at a candidate `(u, v)`, or `None`
    /// if the warp reaches out of the valid image. This is the seed-selection
    /// score for spatial propagation.
    #[allow(clippy::too_many_arguments)]
    fn patch_ssd(
        &self,
        level: &Level,
        ox: usize,
        oy: usize,
        ps: usize,
        _gx_sum: f32,
        _gy_sum: f32,
        valid_count: usize,
        u: f32,
        v: f32,
    ) -> Option<f32> {
        let mut diff_sum = 0.0f32;
        let mut diff_sq = 0.0f32;
        for j in 0..ps {
            for i in 0..ps {
                let px = ox + i;
                let py = oy + j;
                if !level.i0.ok(px, py) {
                    continue;
                }
                let iw = level.i1.sample(px as f32 + u, py as f32 + v)?;
                let diff = iw - level.i0.at(px, py);
                diff_sum += diff;
                diff_sq += diff * diff;
            }
        }
        if self.config.use_mean_normalization {
            let mean = diff_sum / valid_count as f32;
            Some(diff_sq - diff_sum * mean)
        } else {
            Some(diff_sq)
        }
    }

    /// The structure-tensor reliability gate (§47), returning `reliable[p]` for
    /// each patch on the finest level's grid: false where the patch sits on a
    /// mono-texture / lack-of-texture row or a small-disparity block, so
    /// [`Self::densify`] drops it from the vote.
    ///
    /// Studio's `PrecomputeStructureTensor` (`fcn.1841a54f0`) computes the
    /// diagonal tensor components `Jxx = Σ_8×8 Ix²`, `Jyy = Σ_8×8 Iy²` and the
    /// energy on a stride-3 grid of Sobel-3 gradients, then the consumers
    /// `fcn.1841a9320` (mono) / `fcn.1841a9160` (lack) set a per-row bit. This
    /// mirrors that in [`Self::grid_row_stats`], indexed by grid row `gy` — the
    /// row whose bit decides every patch `gy·nx + gx`.
    fn reliability(&self, level: &Level, patches: &[Patch]) -> Vec<bool> {
        let cfg = &self.config;
        let ps = cfg.patch_size;
        let stride = cfg.patch_stride;
        let (w, h) = (level.i0.width, level.i0.height);
        if patches.is_empty() {
            return Vec::new();
        }
        if !(cfg.gate_mono || cfg.gate_lowtex || cfg.gate_small) {
            return vec![true; patches.len()];
        }
        let nx = (w - ps) / stride + 1;
        let ny = (h - ps) / stride + 1;

        // Per grid-row mono / lack flags from the Sobel-3 structure tensor (§47).
        let grid = if cfg.gate_mono || cfg.gate_lowtex {
            self.grid_row_stats(level)
        } else {
            Vec::new()
        };

        // Per-block small-disparity flags (§45.2): a per-pixel "small disparity"
        // byte (nearest patch's |flow| < SMALL_DISPARITY_PX), summed over each
        // 8×8/stride-3 window; block flagged where the sum ≥ the fraction cutoff.
        let small_blocks = if cfg.gate_small {
            self.small_disparity_blocks(patches, nx, ny, w, h)
        } else {
            vec![false; nx * ny]
        };

        let mut reliable = vec![true; patches.len()];
        for gy in 0..ny {
            let row = grid.get(gy).copied();
            for gx in 0..nx {
                let p = gy * nx + gx;
                let mono = cfg.gate_mono && row.is_some_and(|r| r.mono);
                let lowtex = cfg.gate_lowtex && row.is_some_and(|r| r.lowtex);
                let small = cfg.gate_small && small_blocks[p];
                if mono || lowtex || small {
                    reliable[p] = false;
                }
            }
        }
        reliable
    }

    /// Studio's structure-tensor gate metrics (§47), on the stride-3 patch grid
    /// of the finest level's reference image `I0`. One [`GridRowStat`] per grid
    /// row (`ny = (h − ps)/stride + 1`), the exact rows the packed mask indexes.
    ///
    /// The gradients are `cv::spatialGradient` (Sobel ksize-3, `0x1841a6ff6`) of
    /// `I0` on Studio's **uint8** luma scale ([`GATE_LUMA_SCALE`], §47.3). Per
    /// grid patch (its `ps×ps` footprint, top-left `gx·stride, gy·stride`) the two
    /// diagonal components `Jxx = Σ Ix²` (`imul A,A` `0x1841a56c7`), `Jyy = Σ Iy²`
    /// (`imul B,B` `0x1841a56da`) and the energy `Σ (Ix²+Iy²) = Jxx + Jyy` are
    /// box-summed over the patch's VALID gradient pixels — **no ε** on `Jyy`. Then
    /// per grid row:
    /// - **mono** = mean over patches with `Jxx>0 && Jyy>0` of `Jxx/Jyy`
    ///   (mean-of-per-patch-ratios, `divss` `0x1841a9486`); flag `> 4.0`
    ///   ([`MONO_TEXTURE_RATIO`], `0x1841a955a`). Zero eligible patches ⇒ NOT
    ///   mono-flagged (§47.1: "that's lack-of-texture's job").
    /// - **lack** = mean over patches with `energy > 1.0` ([`LACK_OF_TEXTURE_VALID`],
    ///   `0x1841a9278`) of that patch energy; flag `< `[`LACK_OF_TEXTURE_ENERGY`]
    ///   (`2000.0`, `0x1841a92a2`). A row whose every patch is sub-floor has no
    ///   texture at all and is lack-flagged.
    fn grid_row_stats(&self, level: &Level) -> Vec<GridRowStat> {
        let cfg = &self.config;
        let ps = cfg.patch_size;
        let stride = cfg.patch_stride;
        let (w, h) = (level.i0.width, level.i0.height);
        if w < ps || h < ps {
            return Vec::new();
        }
        let nx = (w - ps) / stride + 1;
        let ny = (h - ps) / stride + 1;

        // Sobel-3 gradients of I0 on Studio's uint8 luma scale, and the per-pixel
        // squared diagonals + a validity count, summed-area-tabled so an 8×8 patch
        // sum is four lookups. Ix,Iy are already ×GATE_LUMA_SCALE so the energy is
        // in Studio's units and the 2000.0 threshold is faithful (§47.3).
        let (ix, iy, gv) = sobel3_gradients(&level.i0);
        let n = w * h;
        let mut gxx = vec![0.0f64; n];
        let mut gyy = vec![0.0f64; n];
        let mut cnt = vec![0.0f64; n];
        for i in 0..n {
            if gv[i] {
                let (fx, fy) = (f64::from(ix[i]), f64::from(iy[i]));
                gxx[i] = fx * fx;
                gyy[i] = fy * fy;
                cnt[i] = 1.0;
            }
        }
        let sxx = sat(&gxx, w, h);
        let syy = sat(&gyy, w, h);
        let scn = sat(&cnt, w, h);

        let mut rows = Vec::with_capacity(ny);
        for gy in 0..ny {
            let oy = gy * stride;
            let (y0, y1) = (oy, oy + ps - 1);
            let (mut ratio_sum, mut ratio_cnt) = (0.0f64, 0usize);
            let (mut energy_sum, mut energy_cnt) = (0.0f64, 0usize);
            for gx in 0..nx {
                let ox = gx * stride;
                let (x0, x1) = (ox, ox + ps - 1);
                // Fully-sentinel patch: no gradient pixels, no vote.
                if box_sum(&scn, w, x0, y0, x1, y1) < 1.0 {
                    continue;
                }
                let jxx = box_sum(&sxx, w, x0, y0, x1, y1);
                let jyy = box_sum(&syy, w, x0, y0, x1, y1);
                let energy = jxx + jyy;
                // Lack: patch energy above the 1.0 floor joins the row mean.
                if energy > LACK_OF_TEXTURE_VALID {
                    energy_sum += energy;
                    energy_cnt += 1;
                }
                // Mono: Jxx/Jyy of patches whose both diagonals are strictly > 0.
                if jxx > 0.0 && jyy > 0.0 {
                    ratio_sum += jxx / jyy;
                    ratio_cnt += 1;
                }
            }
            let (energy_mean, lowtex) = if energy_cnt > 0 {
                let m = energy_sum / energy_cnt as f64;
                (m, m < f64::from(LACK_OF_TEXTURE_ENERGY))
            } else {
                // No patch above the floor: the row is textureless -> lack.
                (0.0, true)
            };
            let (ratio_mean, mono) = if ratio_cnt > 0 {
                let m = ratio_sum / ratio_cnt as f64;
                (m, m > f64::from(MONO_TEXTURE_RATIO))
            } else {
                (0.0, false)
            };
            rows.push(GridRowStat {
                mono,
                lowtex,
                energy_mean,
                ratio_mean,
            });
        }
        rows
    }

    /// The small-disparity block mask (`GenerateBlockMaskKel`, §45.2): a
    /// per-pixel byte (1 where the nearest patch's flow magnitude is below
    /// [`SMALL_DISPARITY_PX`]) summed over each 8×8/stride-3 window; a block is
    /// flagged where the byte-sum reaches the fraction cutoff (`≥ 7` for extent
    /// 8, `ceil(`[`SMALL_DISPARITY_FRAC`]`·64)`). CPU-INFERRED per §45.2.
    fn small_disparity_blocks(
        &self,
        patches: &[Patch],
        nx: usize,
        ny: usize,
        w: usize,
        h: usize,
    ) -> Vec<bool> {
        let cfg = &self.config;
        let ps = cfg.patch_size;
        let stride = cfg.patch_stride;
        let half = ((ps - 1) / 2) as f32;
        // Per-pixel small-disparity byte from the nearest patch's displacement.
        let mut byte = vec![0u8; w * h];
        for y in 0..h {
            let gy = (((y as f32 - half) / stride as f32).round() as isize)
                .clamp(0, ny as isize - 1) as usize;
            for x in 0..w {
                let gx = (((x as f32 - half) / stride as f32).round() as isize)
                    .clamp(0, nx as isize - 1) as usize;
                let p = &patches[gy * nx + gx];
                if p.valid && (p.u * p.u + p.v * p.v).sqrt() < SMALL_DISPARITY_PX {
                    byte[y * w + x] = 1;
                }
            }
        }
        let ext = SMALL_DISPARITY_EXTENT.min(ps);
        let min_count = (SMALL_DISPARITY_FRAC * (ext * ext) as f32).ceil() as u32;
        let mut blocks = vec![false; nx * ny];
        for by in 0..ny {
            for bx in 0..nx {
                let (ox, oy) = (bx * stride, by * stride);
                let mut sum = 0u32;
                for j in 0..ext {
                    for i in 0..ext {
                        sum += u32::from(byte[(oy + j) * w + (ox + i)]);
                    }
                }
                blocks[by * nx + bx] = sum >= min_count;
            }
        }
        blocks
    }

    /// Densification (`Densification_ParBody`): every output pixel is the
    /// weighted mean of the flows of the patches covering it, each weighted by
    /// `1/max(1, |I0 - I1(warp)|)` evaluated at the pixel. A pixel no valid
    /// patch covers is left zero and marked invalid.
    ///
    /// **Mask-aware (§45.2 action, `DensificationKel<…,BOOL>`).** `reliable[p]`
    /// is false for a patch the structure-tensor gate flagged (mono-texture /
    /// lack-of-texture / small-disparity); such patches are EXCLUDED from the
    /// vote, so a flagged output pixel is filled from the valid neighbouring
    /// patches that still overlap it (patch_size 8 > stride 3 gives that
    /// overlap), and if none do it becomes invalid — a no-vote pixel the
    /// variational stage then smooths in, exactly as a sentinel-invalid pixel is
    /// handled. `reliable` is all-true at every level but the finest.
    ///
    /// **Coverage-mask NaN (§61/§63).** The src-lens mask `m0` is consulted HERE
    /// (never folded into the colour plane): a pixel outside `m0` is DROPPED —
    /// Studio's `Densification_ParBody` writing NaN (`0x7fc00000`) where the
    /// densi-mask is 0 — our equivalent `valid = false`, filled from valid
    /// neighbours + the variational stage downstream, the same path a no-picture
    /// sentinel takes. Inside `m0` the colour is FULL, so the vote reads real
    /// texture and does not starve at a narrow extent (§63 BUG 1 fix).
    fn densify(
        &self,
        level: &Level,
        patches: &[Patch],
        reliable: &[bool],
        field: &mut FlowField,
        write_residual: bool,
    ) {
        let cfg = &self.config;
        let ps = cfg.patch_size;
        let stride = cfg.patch_stride;
        let (w, h) = (level.i0.width, level.i0.height);
        let m0 = level.m0.as_deref();
        // The MUTUAL densi-NaN (§74 finding 4 / §61c): the output is dropped where
        // EITHER lens mask is 0, matching `IsInMask`'s mutual test — not just the
        // reference lens. This removes the one-sided skirt leak (in r2l the
        // reference lens1 still covers rows 86-94 but target lens0 has faded out
        // there, so the reference-only NaN let lens0's high-θ vignette skirt through;
        // the mutual test drops those rows → 0) while leaving the clean-band median
        // unchanged (both masks set inside the band). Sampled at the OUTPUT pixel,
        // not the flow-displaced position: the densi-mask is per-output-pixel
        // coverage.
        let m1 = level.m1.as_deref();
        let covered = |i: usize| m0.is_none_or(|m| m[i]) && m1.is_none_or(|m| m[i]);
        if w < ps || h < ps || patches.is_empty() {
            for i in 0..w * h {
                field.valid[i] = level.i0.valid[i] && covered(i);
                if write_residual {
                    field.residual[i] = f32::INFINITY;
                }
            }
            return;
        }
        let nx = (w - ps) / stride + 1;

        for y in 0..h {
            for x in 0..w {
                let out_i = y * w + x;
                // Coverage-mask OUTPUT NaN (§61/§63/§74): a pixel outside EITHER
                // lens mask (mutual) is dropped, never trusted from a degenerate
                // patch. The mask is read HERE only, not in the colour footprint the
                // vote correlates over.
                if !covered(out_i) {
                    field.valid[out_i] = false;
                    if write_residual {
                        field.residual[out_i] = f32::INFINITY;
                    }
                    continue;
                }
                // Which grid patches cover this pixel: gx with ox ≤ x < ox+ps.
                let gx_lo = (x + 1).saturating_sub(ps).div_ceil(stride);
                let gx_hi = (x / stride).min(nx - 1);
                let gy_lo = (y + 1).saturating_sub(ps).div_ceil(stride);
                let ny = (h - ps) / stride + 1;
                let gy_hi = (y / stride).min(ny - 1);

                let mut sum_u = 0.0f32;
                let mut sum_v = 0.0f32;
                let mut sum_w = 0.0f32;
                // Best (lowest) residual among the covering patches — the
                // per-pixel reference residual carried for a later frame's
                // warm-start re-search-SKIP (§75). Taking the MIN (not the
                // photometric-weighted mean the flow uses) keeps the value close
                // to the covering patch's own SSD, so next frame's per-patch
                // seed SSD is compared on the same footing (the RE'd `thr = 1.0`
                // is a tight per-patch delta).
                let mut best_res = f32::INFINITY;
                if level.i0.ok(x, y) {
                    for gy in gy_lo..=gy_hi {
                        for gx in gx_lo..=gx_hi {
                            let gi = gy * nx + gx;
                            let p = &patches[gi];
                            // Sentinel-invalid patches never voted; the gate
                            // additionally excludes structure-degenerate ones.
                            if !p.valid || !reliable[gi] {
                                continue;
                            }
                            let diff = match level.i1.sample(x as f32 + p.u, y as f32 + p.v) {
                                Some(iw) => (iw - level.i0.at(x, y)).abs(),
                                None => 1.0,
                            };
                            let coef = 1.0 / diff.max(1e-3);
                            sum_u += coef * p.u;
                            sum_v += coef * p.v;
                            sum_w += coef;
                            best_res = best_res.min(p.residual);
                        }
                    }
                }
                if sum_w > 0.0 {
                    field.u[out_i] = sum_u / sum_w;
                    field.v[out_i] = sum_v / sum_w;
                    field.valid[out_i] = true;
                    if write_residual {
                        field.residual[out_i] = best_res;
                    }
                } else {
                    // Keep whatever the upscaled guess held, but say it is not
                    // supported here.
                    field.valid[out_i] = false;
                    if write_residual {
                        field.residual[out_i] = f32::INFINITY;
                    }
                }
            }
        }
    }

    /// Variational refinement (OpenCV `VariationalRefinement`, solved by
    /// `RedBlackSOR`): Brox-style brightness + gradient constancy data terms
    /// with a Charbonnier penalty and an α-weighted Charbonnier smoothness,
    /// minimized by outer warp iterations around inner Red-Black SOR sweeps.
    ///
    /// Sentinel pixels are frozen (their flow is not updated and they
    /// contribute no data or smoothness coupling), so no-picture regions
    /// neither move nor drag their neighbours.
    fn variational(&self, level: &Level, field: &mut FlowField) {
        let cfg = &self.config;
        let (w, h) = (level.i0.width, level.i0.height);
        let n = w * h;
        // I1's gradients, needed for the linearization and the gradient
        // constancy term.
        let (i1x, i1y, _) = gradients(&level.i1);
        let i1xp = Plane {
            width: w,
            height: h,
            data: i1x.clone(),
            valid: level.i1.valid.clone(),
        };
        let i1yp = Plane {
            width: w,
            height: h,
            data: i1y.clone(),
            valid: level.i1.valid.clone(),
        };

        // Which pixels the solve is allowed to move: valid in both frames'
        // support at the current warp. Computed per outer iteration below via
        // the warp, but the base validity gate is I0's.
        let solve = level.i0.valid.clone();

        for _ in 0..cfg.variational_iters {
            // Warp I1 and its gradients by the current flow, and form the
            // per-pixel constancy residuals and their spatial derivatives.
            let mut iz = vec![0.0f32; n]; // I1(w) - I0
            let mut ix = vec![0.0f32; n]; // ∂x I1(w)
            let mut iy = vec![0.0f32; n]; // ∂y I1(w)
            let mut ixz = vec![0.0f32; n]; // I1x(w) - I0x
            let mut iyz = vec![0.0f32; n]; // I1y(w) - I0y
            let mut ixx = vec![0.0f32; n]; // ∂x I1x(w)
            let mut ixy = vec![0.0f32; n]; // ∂y I1x(w)
            let mut iyy = vec![0.0f32; n]; // ∂y I1y(w)
            let mut movable = vec![false; n];
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    if !solve[i] {
                        continue;
                    }
                    let fx = x as f32 + field.u[i];
                    let fy = y as f32 + field.v[i];
                    let (Some(iw), Some(iwx), Some(iwy)) = (
                        level.i1.sample(fx, fy),
                        i1xp.sample(fx, fy),
                        i1yp.sample(fx, fy),
                    ) else {
                        continue;
                    };
                    iz[i] = iw - level.i0.at(x, y);
                    ix[i] = iwx;
                    iy[i] = iwy;
                    ixz[i] = iwx - level.i0x[i];
                    iyz[i] = iwy - level.i0y[i];
                    // Second derivatives of the warped I1, by sampling the
                    // warped gradient a pixel either side.
                    let ixx_s = (i1xp.sample(fx + 1.0, fy), i1xp.sample(fx - 1.0, fy));
                    let ixy_s = (i1xp.sample(fx, fy + 1.0), i1xp.sample(fx, fy - 1.0));
                    let iyy_s = (i1yp.sample(fx, fy + 1.0), i1yp.sample(fx, fy - 1.0));
                    if let (Some(a), Some(b)) = ixx_s {
                        ixx[i] = 0.5 * (a - b);
                    }
                    if let (Some(a), Some(b)) = ixy_s {
                        ixy[i] = 0.5 * (a - b);
                    }
                    if let (Some(a), Some(b)) = iyy_s {
                        iyy[i] = 0.5 * (a - b);
                    }
                    movable[i] = true;
                }
            }

            // Increment we solve for this warp; the flow update is u += du.
            let mut du = vec![0.0f32; n];
            let mut dv = vec![0.0f32; n];

            for _ in 0..cfg.sor_iters {
                // Reweight the robust penalties from the current increment.
                // Data weight ψ = Ψ'((Iz + Ix·du + Iy·dv)²); gradient weight
                // ψ2 similarly on the gradient residual.
                let mut wd = vec![0.0f32; n];
                let mut wg = vec![0.0f32; n];
                let mut ws = vec![0.0f32; n]; // smoothness weight at each pixel
                for i in 0..n {
                    if !movable[i] {
                        continue;
                    }
                    let rb = iz[i] + ix[i] * du[i] + iy[i] * dv[i];
                    wd[i] = cfg.delta * charbonnier(rb * rb);
                    let rgx = ixz[i] + ixx[i] * du[i] + ixy[i] * dv[i];
                    let rgy = iyz[i] + ixy[i] * du[i] + iyy[i] * dv[i];
                    wg[i] = cfg.gamma * charbonnier(rgx * rgx + rgy * rgy);
                }
                // Smoothness weight from the gradient of the *total* flow
                // (u+du). Central differences over movable neighbours.
                for y in 0..h {
                    for x in 0..w {
                        let i = y * w + x;
                        if !movable[i] {
                            continue;
                        }
                        let (gux, guy) = flow_grad(&field.u, &du, &movable, w, h, x, y);
                        let (gvx, gvy) = flow_grad(&field.v, &dv, &movable, w, h, x, y);
                        ws[i] =
                            cfg.alpha * charbonnier(gux * gux + guy * guy + gvx * gvx + gvy * gvy);
                    }
                }

                // Red then Black SOR sweeps.
                for color in 0..2 {
                    for y in 0..h {
                        for x in 0..w {
                            if (x + y) % 2 != color {
                                continue;
                            }
                            let i = y * w + x;
                            if !movable[i] {
                                continue;
                            }
                            // Smoothness coupling to the four neighbours.
                            let mut sw_sum = 0.0f32;
                            let mut su = 0.0f32;
                            let mut sv = 0.0f32;
                            let mut couple = |xx: usize, yy: usize, here: f32| {
                                let j = yy * w + xx;
                                if !movable[j] {
                                    return;
                                }
                                let sij = 0.5 * (here + ws[j]);
                                sw_sum += sij;
                                su += sij * (field.u[j] + du[j] - field.u[i]);
                                sv += sij * (field.v[j] + dv[j] - field.v[i]);
                            };
                            let here = ws[i];
                            if x > 0 {
                                couple(x - 1, y, here);
                            }
                            if x + 1 < w {
                                couple(x + 1, y, here);
                            }
                            if y > 0 {
                                couple(x, y - 1, here);
                            }
                            if y + 1 < h {
                                couple(x, y + 1, here);
                            }

                            // Data-term normal equations (brightness + grad).
                            let a11 = wd[i] * ix[i] * ix[i]
                                + wg[i] * (ixx[i] * ixx[i] + ixy[i] * ixy[i])
                                + sw_sum;
                            let a12 =
                                wd[i] * ix[i] * iy[i] + wg[i] * (ixx[i] * ixy[i] + ixy[i] * iyy[i]);
                            let a22 = wd[i] * iy[i] * iy[i]
                                + wg[i] * (ixy[i] * ixy[i] + iyy[i] * iyy[i])
                                + sw_sum;
                            let b1 = su
                                - wd[i] * ix[i] * iz[i]
                                - wg[i] * (ixx[i] * ixz[i] + ixy[i] * iyz[i]);
                            let b2 = sv
                                - wd[i] * iy[i] * iz[i]
                                - wg[i] * (ixy[i] * ixz[i] + iyy[i] * iyz[i]);
                            let det = a11 * a22 - a12 * a12;
                            if det.abs() < 1e-12 {
                                continue;
                            }
                            // Solve the 2×2 for the Gauss–Seidel target, then
                            // over-relax.
                            let tu = (a22 * b1 - a12 * b2) / det;
                            let tv = (a11 * b2 - a12 * b1) / det;
                            du[i] += cfg.omega * (tu - du[i]);
                            dv[i] += cfg.omega * (tv - dv[i]);
                        }
                    }
                }
            }

            for i in 0..n {
                if movable[i] {
                    field.u[i] += du[i];
                    field.v[i] += dv[i];
                }
            }
        }
    }
}

/// Central-difference gradient of the total flow `base + inc` at `(x, y)`,
/// over movable neighbours only (a non-movable neighbour reuses the centre so
/// the difference across it is zero).
#[inline]
fn flow_grad(
    base: &[f32],
    inc: &[f32],
    movable: &[bool],
    w: usize,
    h: usize,
    x: usize,
    y: usize,
) -> (f32, f32) {
    let i = y * w + x;
    let here = base[i] + inc[i];
    let val = |xx: usize, yy: usize| {
        let j = yy * w + xx;
        if movable[j] { base[j] + inc[j] } else { here }
    };
    let xm = if x > 0 { val(x - 1, y) } else { here };
    let xp = if x + 1 < w { val(x + 1, y) } else { here };
    let ym = if y > 0 { val(x, y - 1) } else { here };
    let yp = if y + 1 < h { val(x, y + 1) } else { here };
    (0.5 * (xp - xm), 0.5 * (yp - ym))
}

/// Charbonnier penalty derivative `Ψ'(s²) = 1 / (2√(s² + ε²))`.
#[inline]
fn charbonnier(s_sq: f32) -> f32 {
    0.5 / (s_sq + CHARBONNIER_EPS_SQ).sqrt()
}

/// A summed-area table of `src` (`w × h`, row-major), size `(w+1) × (h+1)` with
/// a zero first row and column, so a box sum is four lookups. Used by the
/// structure-tensor gate to box-sum gradient energies in O(1) per pixel.
fn sat(src: &[f64], w: usize, h: usize) -> Vec<f64> {
    let sw = w + 1;
    let mut s = vec![0.0f64; sw * (h + 1)];
    for y in 0..h {
        let mut row = 0.0f64;
        for x in 0..w {
            row += src[y * w + x];
            s[(y + 1) * sw + (x + 1)] = s[y * sw + (x + 1)] + row;
        }
    }
    s
}

/// The inclusive box sum `[x0..=x1] × [y0..=y1]` from a [`sat`] table of a
/// `w`-wide source.
#[inline]
fn box_sum(s: &[f64], w: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> f64 {
    let sw = w + 1;
    s[(y1 + 1) * sw + (x1 + 1)] - s[y0 * sw + (x1 + 1)] - s[(y1 + 1) * sw + x0] + s[y0 * sw + x0]
}

/// A per-belt-row snapshot of the structure-tensor gate at the finest level, for
/// the measurement instruments (floorprobe / band): the mean 8×8-summed gradient
/// energy and mean per-patch `Jxx/Jyy` ratio of the grid row covering this belt
/// row (§47), and whether the mono / lack flags fired. Never used by the flow
/// itself — only to report which flag catches what. `energy_mean` is in Studio's
/// uint8 luma units (so it is directly comparable to the `2000.0` threshold).
#[derive(Clone, Copy, Debug)]
pub struct RowStat {
    pub energy_mean: f32,
    pub ratio_mean: f32,
    pub mono: bool,
    pub lowtex: bool,
}

/// The gate's per-row view and per-patch decision on a real pair, for the
/// measurement instruments. `finest` is the finest processed level; the stats
/// are that level's rows, and `reliable` is its patch grid.
#[derive(Clone, Debug)]
pub struct ReliabilityReport {
    pub level_width: usize,
    pub level_height: usize,
    pub finest_scale: usize,
    pub rows: Vec<RowStat>,
    pub patch_reliable: Vec<bool>,
    pub small_blocks_flagged: usize,
}

impl DisFlow {
    /// Build the finest-level pyramid the way [`Self::calc`] does and report the
    /// structure-tensor gate's per-row stats and per-patch decision on `(i0,
    /// i1)`. Measurement-only (floorprobe / band mode=flowrender); the flow path
    /// uses [`Self::reliability`] directly inside [`Self::calc`].
    pub fn reliability_report(
        &self,
        i0: &[f32],
        i1: &[f32],
        width: usize,
        height: usize,
    ) -> ReliabilityReport {
        let cfg = &self.config;
        let p0 = gaussian_blur_5(&Plane::from_sentinel(i0, width, height));
        let p1 = gaussian_blur_5(&Plane::from_sentinel(i1, width, height));
        let coarsest = self.coarsest_scale(width, height);
        let finest = cfg.finest_scale.min(coarsest);
        let mut planes0 = vec![p0];
        let mut planes1 = vec![p1];
        for _ in 1..=finest {
            planes0.push(downsample(planes0.last().unwrap()));
            planes1.push(downsample(planes1.last().unwrap()));
        }
        let (i0x, i0y, grad_valid) = gradients(&planes0[finest]);
        let level = Level {
            i0: planes0[finest].clone(),
            i1: planes1[finest].clone(),
            i0x,
            i0y,
            grad_valid,
            // Measurement path (structure-tensor gate report) runs maskless — the
            // coverage mask is a flow-path concern, not a reliability metric.
            m0: None,
            m1: None,
        };
        let (lw, lh) = (level.i0.width, level.i0.height);
        // The per-belt-row report is expanded from the grid-row stats the gate
        // actually decides on (§47), nearest grid row per belt row.
        let rows = self.row_stats(&level);
        // A zero field warm-starts the finest patch search, matching calc's
        // coarsest-from-zero path when finest == coarsest.
        let field = FlowField::zeros(lw, lh);
        // Maskless (m0/m1 = None) ⇒ `in_mask` is always true, so the mask gate is
        // a no-op here. No hint here, so the re-search-skip is off.
        let patches = self.patch_search(&level, &field, None);
        let reliable = self.reliability(&level, &patches);
        let small_blocks_flagged = if cfg.gate_small {
            let ps = cfg.patch_size;
            let stride = cfg.patch_stride;
            let nx = (lw - ps) / stride + 1;
            let ny = (lh - ps) / stride + 1;
            self.small_disparity_blocks(&patches, nx, ny, lw, lh)
                .iter()
                .filter(|b| **b)
                .count()
        } else {
            0
        };
        ReliabilityReport {
            level_width: lw,
            level_height: lh,
            finest_scale: finest,
            rows,
            patch_reliable: reliable,
            small_blocks_flagged,
        }
    }

    /// The per-belt-row report for [`Self::reliability_report`], expanded from the
    /// grid-row stats the gate actually decides on ([`Self::grid_row_stats`], §47):
    /// each belt row `y` takes the stat of the grid row whose 8×8 window is centred
    /// nearest it, so the instruments' per-belt-row printout matches the flags the
    /// flow path reads. Length `height`.
    fn row_stats(&self, level: &Level) -> Vec<RowStat> {
        let cfg = &self.config;
        let ps = cfg.patch_size;
        let stride = cfg.patch_stride;
        let h = level.i0.height;
        let grid = self.grid_row_stats(level);
        let ny = grid.len();
        let mut rows = Vec::with_capacity(h);
        for y in 0..h {
            let stat = if ny == 0 {
                None
            } else {
                // The grid row whose window (top-left gy·stride, extent ps) is
                // centred nearest belt row y.
                let gy = (((y as f32 - (ps as f32 - 1.0) * 0.5) / stride as f32).round() as isize)
                    .clamp(0, ny as isize - 1) as usize;
                Some(grid[gy])
            };
            rows.push(match stat {
                Some(s) => RowStat {
                    energy_mean: s.energy_mean as f32,
                    ratio_mean: s.ratio_mean as f32,
                    mono: s.mono,
                    lowtex: s.lowtex,
                },
                None => RowStat {
                    energy_mean: 0.0,
                    ratio_mean: 0.0,
                    mono: false,
                    lowtex: false,
                },
            });
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic textured image: a sum of a few sinusoids so it has
    /// gradient content on both axes at several frequencies, plus a little
    /// value-noise, clamped to [0, 1]. No sentinels.
    fn textured(width: usize, height: usize) -> Vec<f32> {
        let mut img = vec![0.0f32; width * height];
        for y in 0..height {
            for x in 0..width {
                let fx = x as f32;
                let fy = y as f32;
                let v = 0.5
                    + 0.20 * (fx * 0.20).sin() * (fy * 0.17).cos()
                    + 0.15 * (fx * 0.07 + fy * 0.05).sin()
                    + 0.10 * (fx * 0.33).cos()
                    + 0.08 * ((fx * 0.11).sin() * (fy * 0.09).sin());
                img[y * width + x] = v.clamp(0.0, 1.0);
            }
        }
        img
    }

    /// Sample `src` at `(x - dx, y - dy)` so the *content* moves by `(dx, dy)`
    /// (a feature at `p` in `src` lands at `p + (dx, dy)` in the shifted
    /// image). Out-of-range reads clamp to the edge, giving a valid interior.
    fn shift(src: &[f32], width: usize, height: usize, dx: f32, dy: f32) -> Vec<f32> {
        let mut out = vec![0.0f32; width * height];
        for y in 0..height {
            for x in 0..width {
                let sx = (x as f32 - dx).clamp(0.0, width as f32 - 1.0);
                let sy = (y as f32 - dy).clamp(0.0, height as f32 - 1.0);
                let x0 = sx.floor() as usize;
                let y0 = sy.floor() as usize;
                let x1 = (x0 + 1).min(width - 1);
                let y1 = (y0 + 1).min(height - 1);
                let tx = sx - x0 as f32;
                let ty = sy - y0 as f32;
                let top = src[y0 * width + x0] * (1.0 - tx) + src[y0 * width + x1] * tx;
                let bot = src[y1 * width + x0] * (1.0 - tx) + src[y1 * width + x1] * tx;
                out[y * width + x] = top * (1.0 - ty) + bot * ty;
            }
        }
        out
    }

    /// Median over an interior window, ignoring invalid pixels.
    fn median_interior(
        field: &[f32],
        valid: &[bool],
        width: usize,
        height: usize,
        margin: usize,
    ) -> f32 {
        let mut vals: Vec<f32> = Vec::new();
        for y in margin..height - margin {
            for x in margin..width - margin {
                let i = y * width + x;
                if valid[i] {
                    vals.push(field[i]);
                }
            }
        }
        vals.sort_by(|a, b| a.total_cmp(b));
        vals[vals.len() / 2]
    }

    #[test]
    fn recovers_a_large_shift_on_a_strip_shaped_frame() {
        // The real case: a strip 162 tall (STRIP_H) with a LARGE across-seam
        // shift — the scale the two lenses are actually offset by at the riser
        // (~35 rows). This is where DIS classically fails if the pyramid is too
        // shallow to reach the displacement. Reported so the pyramid depth can
        // be judged against Studio's FDSFlow rather than assumed.
        let (w, h) = (512, 162);
        let i0 = textured(w, h);
        for &dy in &[8.0f32, 20.0, 35.0] {
            let i1 = shift(&i0, w, h, 0.0, dy);
            let dis = DisFlow::new(DisConfig {
                finest_scale: 0,
                gate_mono: false,
                gate_lowtex: false,
                gate_small: false,
                ..DisConfig::default()
            });
            let flow = dis.calc(&i0, &i1, w, h, None, None);
            let mv = median_interior(&flow.v, &flow.valid, w, h, 24);
            assert!(
                (mv - dy).abs() < 1.0,
                "DIS failed a large shift: dy={dy} -> recovered v={mv:.2}"
            );
        }
    }

    #[test]
    fn recovers_a_known_whole_field_translation() {
        // A textured frame shifted by a known (dx, dy). The flow from I0 to
        // I1 must report (dx, dy): I1(x+u, y+v) ≈ I0(x, y) with the content
        // moved by (dx, dy) means (u, v) = (dx, dy).
        let (w, h) = (256, 256);
        let i0 = textured(w, h);
        let (dx, dy) = (7.0f32, -4.0f32);
        let i1 = shift(&i0, w, h, dx, dy);

        // finest_scale 0: full-resolution, for the sub-pixel bar.
        let dis = DisFlow::new(DisConfig {
            finest_scale: 0,
            // These exercise the DIS CORE (inverse search + densification +
            // variational) on a full-resolution translation; the §45 gate is a
            // separable flow-path layer with its own test below, and the
            // synthetic sinusoids here are pathologically 1-D at the 8×8 scale
            // (every patch reads as aperture), so isolate the core with it off.
            gate_mono: false,
            gate_lowtex: false,
            gate_small: false,
            ..DisConfig::default()
        });
        let flow = dis.calc(&i0, &i1, w, h, None, None);

        let mu = median_interior(&flow.u, &flow.valid, w, h, 24);
        let mv = median_interior(&flow.v, &flow.valid, w, h, 24);
        assert!(
            (mu - dx).abs() < 0.5 && (mv - dy).abs() < 0.5,
            "median recovered flow ({mu:.3}, {mv:.3}), expected ({dx}, {dy})"
        );
    }

    #[test]
    fn coverage_mask_drops_the_uncovered_band_and_keeps_the_shift() {
        // The per-lens coverage mask (§59/§61): a synthetic two-strip pair with a
        // known across-strip shift, where the SECOND lens (i1) is masked out in a
        // horizontal band — as if the far lens has no clean content there (the
        // one-sided vignette ramp). Both strips carry real content (no −1
        // sentinel), so the ONLY invalidator is the mask. The mutual `IsInMask`
        // gate must drop the estimate in the band (densification NaN → our
        // `valid = false`), while the fully-covered rows still recover the shift.
        let (w, h) = (128, 64);
        let i0 = textured(w, h);
        let (dx, dy) = (0.0f32, 3.0f32);
        let i1 = shift(&i0, w, h, dx, dy);

        // mask0 (i0 / reference lens): covered everywhere. mask1 (i1 / target
        // lens): covered everywhere EXCEPT rows [band_lo, band_hi).
        let (band_lo, band_hi) = (24usize, 40usize);
        let mask0 = vec![true; w * h];
        let mut mask1 = vec![true; w * h];
        for y in band_lo..band_hi {
            for x in 0..w {
                mask1[y * w + x] = false;
            }
        }

        // finest_scale 0 (full-res, sub-pixel bar) with the structure-tensor gate
        // off — the same isolation the other translation tests use, so the mask
        // is the only mechanism under test.
        let dis = DisFlow::new(DisConfig {
            finest_scale: 0,
            gate_mono: false,
            gate_lowtex: false,
            gate_small: false,
            ..DisConfig::default()
        });
        let masked = dis.calc(&i0, &i1, w, h, None, Some((&mask0, &mask1)));

        // 1. The interior of the masked band is INVALID (dropped, not trusted from
        //    a degenerate patch). Sampled a half-patch clear of the band edges,
        //    where every covering patch is fully inside the uncovered band.
        let (mut band_valid, mut band_total) = (0usize, 0usize);
        for y in (band_lo + 6)..(band_hi - 6) {
            for x in 16..w - 16 {
                band_total += 1;
                band_valid += usize::from(masked.valid[y * w + x]);
            }
        }
        assert_eq!(
            band_valid, 0,
            "the masked band should be fully invalid, got {band_valid}/{band_total} valid"
        );

        // 2. The fully-covered rows above the band still recover the planted shift.
        let mut covered: Vec<f32> = Vec::new();
        for y in 4..(band_lo - 4) {
            for x in 16..w - 16 {
                let i = y * w + x;
                if masked.valid[i] {
                    covered.push(masked.v[i]);
                }
            }
        }
        assert!(!covered.is_empty(), "the covered band must produce a flow");
        covered.sort_by(f32::total_cmp);
        let median = covered[covered.len() / 2];
        assert!(
            (median - dy).abs() < 0.75,
            "covered band recovered v={median:.2}, expected {dy}"
        );

        // 3. Null masks reproduce the pre-mask behaviour verbatim: the same call
        //    with no masks leaves the band valid (null ⇒ true).
        let unmasked = dis.calc(&i0, &i1, w, h, None, None);
        let mid = ((band_lo + band_hi) / 2) * w + w / 2;
        assert!(
            unmasked.valid[mid],
            "with no masks the band must stay valid (null ⇒ true)"
        );
    }

    #[test]
    fn a_patch_anchored_in_mask_votes_even_when_its_footprint_is_mostly_outside() {
        // §63 BUG 2: `IsInMask` is a SINGLE anchor-pixel test, not a
        // footprint-majority abandon. A patch whose anchor is JUST INSIDE the
        // coverage mask but whose 8×8 footprint is MOSTLY outside it must still be
        // kept (its footprint reads full, unmasked belt colour) and produce flow —
        // the exact case the old footprint-majority gate wrongly abandoned, which
        // starved the seam band at narrow extents. Here both lenses' masks clip at
        // the SAME inward edge and the shift is purely horizontal, so the mask
        // reasoning is on the across-strip axis alone.
        let (w, h) = (128usize, 48usize);
        let i0 = textured(w, h);
        let (dx, dy) = (2.0f32, 0.0f32);
        let i1 = shift(&i0, w, h, dx, dy);

        // Both masks: valid where row < edge; invalid at and beyond it. The patch
        // anchored at row `edge-1` has footprint rows `edge-1..=edge+6` — 7 of its
        // 8 rows are OUTSIDE the mask, only the anchor row is inside.
        let edge = 20usize;
        let mut mask0 = vec![true; w * h];
        for y in edge..h {
            for x in 0..w {
                mask0[y * w + x] = false;
            }
        }
        let mask1 = mask0.clone();

        let dis = DisFlow::new(DisConfig {
            finest_scale: 0,
            gate_mono: false,
            gate_lowtex: false,
            gate_small: false,
            ..DisConfig::default()
        });
        let f = dis.calc(&i0, &i1, w, h, None, Some((&mask0, &mask1)));

        // The rows JUST INSIDE the mask edge — whose covering patches have
        // footprints extending mostly OUTSIDE the mask — are still VALID and
        // recover the planted horizontal shift. Under the footprint-majority gate
        // (BUG 2) these near-edge patches were abandoned and these rows starved.
        let mut edge_u: Vec<f32> = Vec::new();
        for y in (edge - 4)..edge {
            for x in 16..w - 16 {
                let i = y * w + x;
                assert!(
                    f.valid[i],
                    "row {y} just inside the mask must stay valid (footprint mostly \
                     outside, anchor inside — single-pixel IsInMask keeps it)"
                );
                edge_u.push(f.u[i]);
            }
        }
        edge_u.sort_by(f32::total_cmp);
        let median = edge_u[edge_u.len() / 2];
        assert!(
            (median - dx).abs() < 0.75,
            "near-edge rows recovered u={median:.2}, expected {dx} (BUG 2: the \
             anchor-in-mask/footprint-outside patch must still vote)"
        );

        // Decoy: well OUTSIDE the mask the output is dropped (densification NaN /
        // our valid = false), so the correction is confined to the covered edge.
        for x in 16..w - 16 {
            let i = (edge + 6) * w + x;
            assert!(
                !f.valid[i],
                "row {} is outside the mask and must be dropped",
                edge + 6
            );
        }
    }

    #[test]
    fn recovers_a_depth_edge_between_two_shifts() {
        // Two vertical regions with different horizontal shifts, i.e. a depth
        // discontinuity down the middle. The flow must recover both shifts and
        // localize the edge near the true boundary.
        let (w, h) = (256, 256);
        let base = textured(w, h);
        let left_shift = 6.0f32;
        let right_shift = -3.0f32;
        let boundary = w / 2;

        // Build I1 so the left half of the content moved by +6 and the right
        // half by -3. Do it by shifting the whole image both ways and taking
        // each side from the matching shift.
        let left = shift(&base, w, h, left_shift, 0.0);
        let right = shift(&base, w, h, right_shift, 0.0);
        let mut i1 = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                i1[y * w + x] = if x < boundary {
                    left[y * w + x]
                } else {
                    right[y * w + x]
                };
            }
        }

        let dis = DisFlow::new(DisConfig {
            finest_scale: 0,
            // These exercise the DIS CORE (inverse search + densification +
            // variational) on a full-resolution translation; the §45 gate is a
            // separable flow-path layer with its own test below, and the
            // synthetic sinusoids here are pathologically 1-D at the 8×8 scale
            // (every patch reads as aperture), so isolate the core with it off.
            gate_mono: false,
            gate_lowtex: false,
            gate_small: false,
            ..DisConfig::default()
        });
        let flow = dis.calc(&base, &i1, w, h, None, None);

        // Median u well inside each region (away from the edge and borders).
        let side_median = |x_lo: usize, x_hi: usize| -> f32 {
            let mut vals: Vec<f32> = Vec::new();
            for y in 32..h - 32 {
                for x in x_lo..x_hi {
                    let i = y * w + x;
                    if flow.valid[i] {
                        vals.push(flow.u[i]);
                    }
                }
            }
            vals.sort_by(|a, b| a.total_cmp(b));
            vals[vals.len() / 2]
        };
        let ml = side_median(32, boundary - 32);
        let mr = side_median(boundary + 32, w - 32);
        assert!(
            (ml - left_shift).abs() < 0.8,
            "left region recovered {ml:.3}, expected {left_shift}"
        );
        assert!(
            (mr - right_shift).abs() < 0.8,
            "right region recovered {mr:.3}, expected {right_shift}"
        );

        // The edge should be localized near the true boundary: find, per row,
        // the x where u crosses the midpoint of the two shifts, and check the
        // median crossing is close to `boundary`.
        let mid = 0.5 * (left_shift + right_shift);
        let mut crossings: Vec<f32> = Vec::new();
        for y in 32..h - 32 {
            let mut prev: Option<(usize, f32)> = None;
            for x in 32..w - 32 {
                let i = y * w + x;
                if !flow.valid[i] {
                    continue;
                }
                let u = flow.u[i];
                if let Some((px, pu)) = prev
                    && (pu - mid) * (u - mid) <= 0.0
                    && (pu - u).abs() > 1e-4
                {
                    let t = (mid - pu) / (u - pu);
                    crossings.push(px as f32 + t * (x - px) as f32);
                    break;
                }
                prev = Some((x, u));
            }
        }
        assert!(!crossings.is_empty(), "no edge crossing found");
        crossings.sort_by(|a, b| a.total_cmp(b));
        let edge = crossings[crossings.len() / 2];
        assert!(
            (edge - boundary as f32).abs() < 12.0,
            "edge localized at x={edge:.1}, expected near {boundary}"
        );
    }

    #[test]
    fn a_majority_sentinel_patch_yields_invalid_flow() {
        // A frame whose right third is all sentinel. The flow there must be
        // marked invalid rather than matched against the no-picture region.
        let (w, h) = (128, 128);
        let mut i0 = textured(w, h);
        let mut i1 = shift(&i0, w, h, 3.0, 0.0);
        for y in 0..h {
            for x in 2 * w / 3..w {
                i0[y * w + x] = -1.0;
                i1[y * w + x] = -1.0;
            }
        }
        let dis = DisFlow::new(DisConfig {
            finest_scale: 0,
            // These exercise the DIS CORE (inverse search + densification +
            // variational) on a full-resolution translation; the §45 gate is a
            // separable flow-path layer with its own test below, and the
            // synthetic sinusoids here are pathologically 1-D at the 8×8 scale
            // (every patch reads as aperture), so isolate the core with it off.
            gate_mono: false,
            gate_lowtex: false,
            gate_small: false,
            ..DisConfig::default()
        });
        let flow = dis.calc(&i0, &i1, w, h, None, None);
        // Deep in the sentinel region, invalid.
        let deep = 64 * w + (w - 4);
        assert!(!flow.valid[deep], "sentinel region should be invalid");
        // The pictured interior still recovers the shift.
        let mu = median_interior(&flow.u[..], &flow.valid[..], w, h, 20);
        // Median over the full interior includes only valid (pictured) pixels.
        assert!((mu - 3.0).abs() < 0.8, "pictured region recovered {mu:.3}");
    }

    /// A smooth, broadly isotropic 2-D texture (gradient content on both axes),
    /// trackable by DIS and NOT lack-of-texture. `bias` shifts the mean so a band
    /// can be built brighter/darker without changing its texture.
    fn iso_texture(x: usize, y: usize, bias: f32) -> f32 {
        let (fx, fy) = (x as f32, y as f32);
        (bias
            + 0.24 * (fx * 0.6).sin()
            + 0.22 * (fy * 0.55).cos()
            + 0.16 * (fx * 0.31 + fy * 0.29).sin())
        .clamp(0.0, 1.0)
    }

    #[test]
    fn the_gate_flags_a_flat_band_and_leaves_good_textured_flow_intact() {
        // A strip with a textured band, a FLAT (low-texture) band, then a
        // textured band again. The lack-of-texture gate (the default) must flag
        // the flat band's rows and leave the textured bands' flow untouched — a
        // planted shift there is still recovered. This is §45.2's "drop the
        // degenerate rows, keep the good flow" on a controlled input.
        let (w, h) = (200usize, 120usize);
        let flat_lo = 40usize;
        let flat_hi = 80usize;
        let mut i0 = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                i0[y * w + x] = if (flat_lo..flat_hi).contains(&y) {
                    // Flat band: a constant plus a deterministic hair of noise,
                    // well under the lack-of-texture energy floor.
                    0.5 + 0.0006 * (((x * 7 + y * 13) % 5) as f32 - 2.0)
                } else {
                    iso_texture(x, y, 0.5)
                };
            }
        }
        let (dx, dy) = (3.0f32, 0.0f32);
        let i1 = shift(&i0, w, h, dx, dy);

        // The gate's own view: the flat rows are lack-flagged, the textured rows
        // are not. Isolate the lack flag (mono/small off) so the assertions read
        // exactly its behaviour; all three are on in the faithful default.
        let dis = DisFlow::new(DisConfig {
            finest_scale: 0,
            gate_mono: false,
            gate_lowtex: true,
            gate_small: false,
            ..DisConfig::default()
        });
        assert!(dis.config().gate_lowtex && !dis.config().gate_mono);
        let rep = dis.reliability_report(&i0, &i1, w, h);
        let flat_flagged = (flat_lo + 4..flat_hi - 4)
            .filter(|&y| rep.rows[y].lowtex)
            .count();
        assert!(
            flat_flagged >= (flat_hi - flat_lo - 8) * 3 / 4,
            "the flat band should be lack-flagged: {flat_flagged} of {} rows",
            flat_hi - flat_lo - 8,
        );
        let tex_flagged = (8..flat_lo - 4)
            .chain(flat_hi + 4..h - 8)
            .filter(|&y| rep.rows[y].lowtex)
            .count();
        assert_eq!(tex_flagged, 0, "no textured row should be lack-flagged");

        // The textured bands recover the planted shift WITH the gate on, and the
        // flow there is the same as with the gate fully off (the gate does not
        // touch good rows).
        let off = DisFlow::new(DisConfig {
            finest_scale: 0,
            gate_mono: false,
            gate_lowtex: false,
            gate_small: false,
            ..DisConfig::default()
        });
        let f_on = dis.calc(&i0, &i1, w, h, None, None);
        let f_off = off.calc(&i0, &i1, w, h, None, None);
        let band_u = |f: &FlowField| -> f32 {
            let mut v: Vec<f32> = Vec::new();
            for y in (8..flat_lo - 4).chain(flat_hi + 4..h - 8) {
                for x in 16..w - 16 {
                    let i = y * w + x;
                    if f.valid[i] {
                        v.push(f.u[i]);
                    }
                }
            }
            v.sort_by(f32::total_cmp);
            v[v.len() / 2]
        };
        let u_on = band_u(&f_on);
        let u_off = band_u(&f_off);
        assert!(
            (u_on - dx).abs() < 0.6,
            "textured band lost the planted shift under the gate: u={u_on:.3} (want {dx})"
        );
        assert!(
            (u_on - u_off).abs() < 0.2,
            "the gate changed good textured flow: on {u_on:.3} vs off {u_off:.3}"
        );
    }

    #[test]
    fn the_mono_flag_fires_on_a_one_directional_band_and_spares_isotropic() {
        // §47.1's Jxx/Jyy metric on the 8×8 patch SUM, and the proof §46's wall is
        // gone. A near-mono-directional band (dominant x-gradient with a hair of
        // y-variation — the real aperture/skirt case) has Jxx ≫ Jyy so Jxx/Jyy far
        // exceeds 4.0 and every such row is flagged. A genuinely ISOTROPIC 2-D
        // band has Jxx ≈ Jyy so the ratio stays well under 4.0 and is NOT flagged
        // — where §46's raw λmax/λmin sent per-pixel Iy²→0 on any coherent edge and
        // flagged EVERYTHING. The 8×8 SUM Jyy is what bounds it.
        let (w, h) = (200usize, 150usize);
        let ap_lo = 50usize; // near-mono-directional band …
        let ap_hi = 100usize; // … between two isotropic bands.
        let mut i0 = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                i0[y * w + x] = if (ap_lo..ap_hi).contains(&y) {
                    // Dominant horizontal stripes with a hair of vertical variation
                    // so Jyy > 0 (not the excluded Jyy==0 case) but Jxx/Jyy ≫ 4.
                    (0.5 + 0.3 * (x as f32 * 0.5).sin() + 0.01 * (y as f32 * 0.3).sin())
                        .clamp(0.0, 1.0)
                } else {
                    iso_texture(x, y, 0.5)
                };
            }
        }
        let dis = DisFlow::new(DisConfig {
            finest_scale: 0,
            gate_mono: true,
            gate_lowtex: false,
            gate_small: false,
            ..DisConfig::default()
        });
        let rep = dis.reliability_report(&i0, &i0, w, h);
        // The near-1-D band is mono-flagged …
        let mono_ap = (ap_lo + 4..ap_hi - 4).filter(|&y| rep.rows[y].mono).count();
        assert!(
            mono_ap >= (ap_hi - ap_lo - 8) * 3 / 4,
            "the near-1-D band should be mono-flagged: {mono_ap} of {} rows",
            ap_hi - ap_lo - 8,
        );
        // … while the isotropic bands are NOT (the §46 pathology — flagging good
        // 2-D texture — is gone), and the ratio they read is bounded under 4.0,
        // not the 1e2–1e12 the raw eigenvalue ratio produced.
        let iso_rows: Vec<usize> = (8..ap_lo - 6).chain(ap_hi + 6..h - 8).collect();
        let iso_flagged = iso_rows.iter().filter(|&&y| rep.rows[y].mono).count();
        assert_eq!(
            iso_flagged, 0,
            "isotropic 2-D texture must not be mono-flagged (that was §46's wall)"
        );
        let iso_ratio_max = iso_rows
            .iter()
            .map(|&y| rep.rows[y].ratio_mean)
            .fold(0.0f32, f32::max);
        assert!(
            iso_ratio_max < MONO_TEXTURE_RATIO,
            "isotropic Jxx/Jyy should stay under 4.0, got max {iso_ratio_max:.2}"
        );
    }

    #[test]
    fn zero_motion_reads_zero() {
        // Identical frames: the flow must be ~zero everywhere valid.
        let (w, h) = (128, 128);
        let i0 = textured(w, h);
        let dis = DisFlow::new(DisConfig {
            finest_scale: 0,
            // These exercise the DIS CORE (inverse search + densification +
            // variational) on a full-resolution translation; the §45 gate is a
            // separable flow-path layer with its own test below, and the
            // synthetic sinusoids here are pathologically 1-D at the 8×8 scale
            // (every patch reads as aperture), so isolate the core with it off.
            gate_mono: false,
            gate_lowtex: false,
            gate_small: false,
            ..DisConfig::default()
        });
        let flow = dis.calc(&i0, &i0, w, h, None, None);
        let mu = median_interior(&flow.u, &flow.valid, w, h, 20);
        let mv = median_interior(&flow.v, &flow.valid, w, h, 20);
        assert!(
            mu.abs() < 0.2 && mv.abs() < 0.2,
            "static scene read ({mu:.3}, {mv:.3}), expected ~0"
        );
    }
}
