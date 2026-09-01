//! Studio-derived optical-flow seam estimators and their coordinate contracts.
//!
//! Two evidence domains live here and must not share an untyped payload. The
//! existing Scene still runs the audited Windows/legacy belt through [`dis`],
//! [`compose`] and [`Cadence`]. Native Mac Studio 6.0.2's selected ONE X2 VIDEO
//! route instead uses the 1080-row by 60-column narrow field in [`one_xs`],
//! with its captured-staging-to-solver reduction closed in [`one_xs_belt`].
//! The ONE X2 route is wired into ordinary playback as a capture-owned,
//! sequential source-to-map transaction.
//!
//! **This module is the replacement for the deleted belt/strip scaffold.**
//! Everything the old scaffold invented - a patch-NCC search, a ring solve,
//! a 2-D cost volume, a median publish - is gone (commit "blow away the
//! non-RE'd displacement stack"). What is built here is Studio's own thing
//! and nothing else: each mechanism cites the address it was read from, and
//! anything not yet read is a disclosed hole, never a guess.
//!
//! **SUPERSEDED pre-§38 legacy build ledger, preserved for history.** The
//! following was the section-35 understanding of the Windows route while this
//! module was first built. Later sections 38-40 replace its single-field apply
//! account with two directed fields. Section 114 separately proves that the
//! selected ONE X2 VIDEO route does not use this period-30 wrapper path.
//!
//! 1. **Line-image strips** (`PreprocessBelt`, `getFisheye2LineMap`): each
//!    lens is rectified into a common along-seam strip. NOT built here yet -
//!    it needs a GPU pass that samples each lens's planes, Studio's
//!    `MapLineImage`. `Strip` was its placeholder at that point.
//! 2. **DIS flow** (`calcFDSFlowMap` -> `bcv::flow::FDSFlow`, ctor
//!    `0x182911140`): a Dense Inverse Search optical flow with variational
//!    refinement - image pyramid, structure tensor, SSD patch inverse
//!    search, densification, then a variational data+smoothness solve by
//!    Red-Black SOR, warm-started from the previous field. This is the
//!    engine, and it is the largest remaining build.
//! 3. **5x5 Gaussian blur** on the input strips (compute `0x182925e30`) and
//!    on each per-lens output map (`getOpMapA`/`getOpMapB`,
//!    `0x183b1a2b0`/`0x183b1dd60`). HARD, owner-tier.
//! 4. **The map composition** (`getOpFisheye2SphereMap` -> `getOpMapA/B`):
//!    the single flow field becomes two per-lens UV maps the fragment shader
//!    looks up. There is NO antisymmetric fuse on this tier
//!    (`UpdateLeftRightFlows` is AI-only, gated on `[StitcherImpl+0xc90]`);
//!    the two lenses' displacements sum to one disparity as a property of
//!    applying the single field, split by a fraction that is the one number
//!    §35 could not read statically.
//! 5. **The cadence** ([`Cadence`]): re-estimate every 30th enabled frame,
//!    hold the previous field between. HARD from the binary.
//!
//! The historical build order mirrored this list. It is not the current route
//! selection or implementation status.

/// The Dense Inverse Search engine itself (chunk 3): a faithful CPU reference
/// of OpenCV's `DISOpticalFlow`, which §35 proved FDSFlow is a vendored copy
/// of. Public because `kjerag-spike --bin band mode=flow` runs it on the real
/// seam strips, and because its constants are the reverse engineering's.
pub mod dis;

/// The selected native ONE X2 retained-flow grid. It is deliberately separate
/// from the audited legacy belt: the two routes disagree about dimensions,
/// axis order and coordinate law, so sharing one untyped payload makes a
/// transposition compile.
pub mod one_xs;

/// The typed U8 source-belt boundary and closed solver-belt reduction for the
/// selected ONE X2 route. Panotype 5 projects two ordered per-lens inputs
/// through the ordered `+0x8d0/+0x930` maps into 3240-by-180 `CV_8UC1`
/// staging belts, then reduces them exactly to ordered 1080-by-60 `CV_8UC1`
/// solver inputs. Ordinary ONE X2 playback consumes this boundary on every
/// frame through its capture-owned producer.
pub mod one_xs_belt;

/// GPU producer for the selected ONE X2 solver belts. Production playback
/// reads its compact exact output into the retained CPU estimator; its packed
/// buffer is also the boundary for migrating that estimator stage by stage.
pub(crate) mod one_xs_belt_gpu;

/// Chunk 4, the composition the draw applies: the TWO separately-estimated DIS
/// fields (lens 0 = r2l `[0xae8]`, lens 1 = l2r `[0xa88]`, §38/§40) carried in
/// belt/sample pixels and tapered independently. The apply itself — the
/// sample-coordinate displacement `q = c + (1 − alpha)·f`, the wide coverage
/// gate that is `alpha`, and the belt law — lives in
/// [`crate::projection::Reframe`] (`flow_shift`/`gate_alpha`/`blend_flow`,
/// §40/§42.3). No env split knob: the per-lens split IS the gate `(1 − alpha)`.
pub mod compose;

/// Which coordinate/payload contract an optical-flow estimate belongs to.
///
/// This gives producers a typed selection boundary so a ONE X2 field cannot be
/// interpreted with the legacy grid law.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowRoute {
    /// The existing 2916 by 486 rotated-equirect belt.
    Legacy,
    /// Mac Studio's selected ONE X2 1080-row by 60-column retained field.
    OneXs,
}

impl FlowRoute {
    /// Select the one camera for which the native retained-flow layout has
    /// direct evidence. `0x29` is Studio's `LensTypeOneXS`, the ONE X2; every
    /// other type stays on the existing route until equivalent evidence exists.
    pub const fn for_lens_type(lens_type: u32) -> Self {
        match lens_type {
            one_xs::LENS_TYPE => Self::OneXs,
            _ => Self::Legacy,
        }
    }

    /// The grid's dimensions and semantic axes. This is descriptive metadata,
    /// not a common payload: [`compose::Displacement`] and
    /// [`one_xs::Displacement`] remain distinct types.
    pub const fn layout(self) -> FlowLayout {
        match self {
            Self::Legacy => FlowLayout {
                rows: crate::band::STRIP_H,
                cols: crate::band::STRIP_W,
                row_axis: SeamAxis::Across,
                col_axis: SeamAxis::Along,
                storage: StorageOrder::RowMajor,
                components: [GridAxis::Column, GridAxis::Row],
            },
            Self::OneXs => one_xs::Layout::FLOW,
        }
    }
}

/// The meaning of a grid dimension relative to the seam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeamAxis {
    Along,
    Across,
}

/// The sample-grid axis one vector component displaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridAxis {
    Column,
    Row,
}

/// How consecutive grid samples are arranged inside each component plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageOrder {
    RowMajor,
}

/// Static shape and axis metadata for a flow route. Both components are always
/// measured in pixels of this very grid; there is no angular or source-pixel
/// rescale at the apply boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlowLayout {
    pub rows: usize,
    pub cols: usize,
    pub row_axis: SeamAxis,
    pub col_axis: SeamAxis,
    pub storage: StorageOrder,
    pub components: [GridAxis; 2],
}

/// How the flow re-estimates in time: Studio's produce-off-draw, hold-last
/// cadence, read instruction-exact from both flow drivers (`0x182885690`,
/// `0x182885d90`, §35).
///
/// Each driver (`0x1828856ee`, `0x182885dee`) opens with, instruction-exact
/// (the listing corrected 2026-08-14 after an adversarial read of the bytes
/// refuted an earlier version of this comment):
///
/// ```text
/// xor  esi, esi
/// cmp  byte [owner+0x4cc], sil   ; Optical Flow enabled? (sil = 0)
/// je   skip                      ; off -> hold, produce nothing
/// mov  eax, [owner+0x4c0]        ; the frame counter
/// cdq                            ; SIGNED: sign-extend into edx
/// idiv dword [owner+0x4c4]       ; / the period
/// test edx, edx / jne skip       ; remainder != 0 -> hold the last field
/// ... build strips, calcFDSFlowMap ...
/// ```
///
/// The `SeamlessBlenderBase` constructor (`0x182860ab3`) sets the counter
/// `[+0x4c0] = 0` (`edi`, zeroed at `0x18286072c`), the period
/// `[+0x4c4] = 0x1e`, and the enable `[+0x4cc]` to zero until Optical Flow
/// is turned on.
///
/// **The counter is NOT advanced by the driver, and NOT gated on enable.**
/// This is the correction. The increment lives in the CALLERS, unconditional
/// after the driver returns - the five sites include `0x18289cdbb`,
/// `0x182894cef`, `0x1828bfcf5`, each the instruction right after the call.
/// So the counter counts every frame through the blend routine, enabled or
/// not, and the enable byte gates only whether THIS frame re-estimates. A
/// disable/enable toggle therefore SHIFTS the phase by the count of disabled
/// frames - the field is not on a fixed 30-frame grid across a toggle, which
/// an earlier version of this type got wrong by freezing the counter while
/// disabled.
///
/// Temporal continuity within an estimate is carried separately by FDSFlow's
/// own `HintFlow` warm start (§35), not by this counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cadence {
    /// `[owner+0x4c0]`: how many frames have passed through the blend
    /// routine. `i32` and not `u32` because the driver divides it with a
    /// SIGNED `cdq`/`idiv`; the two agree below 2^31 and this is the value
    /// `eax` actually holds.
    counter: i32,
}

impl Cadence {
    /// `[owner+0x4c4] = 0x1e`, read from the constructor. Thirty frames
    /// between re-estimates - at 30 fps, once a second, while enabled
    /// continuously.
    pub const PERIOD: i32 = 30;

    /// A blender whose flow has never been produced: the counter Studio
    /// zero-initialises. The first enabled frame at a multiple of 30
    /// re-estimates (`0 % 30 == 0`), so a seam enabled from the start
    /// produces a field on frame zero rather than holding an empty one.
    pub const fn start() -> Self {
        Self { counter: 0 }
    }

    /// The external counter writer, virtual thunk `0x18284e5e0`
    /// (`mov [owner+0x4c0], edx; ret`): something outside the drivers sets
    /// the counter to an arbitrary value. Modelled as a setter rather than a
    /// zeroing because the instruction stores a register, not an immediate.
    /// The callers of the thunk are not yet read, so WHEN Studio resets is
    /// open; the mechanism is here so the draw side can drive it once it is.
    pub fn set(&mut self, counter: i32) {
        self.counter = counter;
    }

    /// Advance one frame and say whether it re-estimates the flow or holds
    /// the last field.
    ///
    /// **The counter advances every frame, disabled included** - the
    /// caller-side unconditional increment (see the type's doc). `enabled`
    /// (`[owner+0x4cc]`) gates production only: a disabled frame holds but
    /// still moves the phase, which is the behaviour a toggle exposes.
    ///
    /// Signed remainder against `test edx, edx`: zero exactly when the
    /// counter is divisible, whatever its sign, matching the binary above
    /// the 2^31 wrap as well as below.
    pub fn tick(&mut self, enabled: bool) -> Estimate {
        let due = enabled && self.counter % Self::PERIOD == 0;
        self.counter = self.counter.wrapping_add(1);
        match due {
            true => Estimate::Reestimate,
            false => Estimate::Hold,
        }
    }
}

/// What one draw does about the flow field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Estimate {
    /// Build the strips and run `calcFDSFlowMap` this draw.
    Reestimate,
    /// Draw with the field the last re-estimate produced.
    Hold,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_blender_re_estimates_on_the_very_first_enabled_draw() {
        // `0 % 30 == 0`: the ctor's zero counter makes the first tick due,
        // so a just-enabled seam does not hold an empty field for a second.
        let mut cadence = Cadence::start();
        assert_eq!(cadence.tick(true), Estimate::Reestimate);
    }

    #[test]
    fn it_re_estimates_once_every_thirty_frames_and_holds_between() {
        let mut cadence = Cadence::start();
        let mut due = Vec::new();
        for _ in 0..90 {
            due.push(cadence.tick(true) == Estimate::Reestimate);
        }
        // Exactly frames 0, 30, 60 re-estimate; the other 87 hold.
        assert_eq!(due.iter().filter(|d| **d).count(), 3);
        assert!(due[0] && due[30] && due[60]);
        assert!(!due[1] && !due[29] && !due[31]);
    }

    #[test]
    fn a_disabled_draw_holds_but_still_advances_the_phase() {
        // The refuted claim, corrected: the caller increments the counter
        // regardless of enable, so disabled frames move the phase. After one
        // enabled re-estimate and 29 DISABLED frames the counter is at 30, so
        // the next enabled frame is due again - the disabled frames counted.
        let mut cadence = Cadence::start();
        assert_eq!(cadence.tick(true), Estimate::Reestimate); // counter 0 -> 1
        for _ in 0..29 {
            assert_eq!(cadence.tick(false), Estimate::Hold); // 1..30
        }
        assert_eq!(cadence.tick(true), Estimate::Reestimate); // counter 30
    }

    #[test]
    fn a_toggle_shifts_the_phase_by_the_disabled_frame_count() {
        // The behaviour the binary has and a frozen counter would not: enable
        // for 5, disable for 7, re-enable - the next re-estimate lands 30
        // frames from frame 0 in ABSOLUTE frame count, not 30 enabled frames.
        let mut cadence = Cadence::start();
        let mut estimates = Vec::new();
        for frame in 0..40 {
            let enabled = !(5..12).contains(&frame); // off for 7 frames
            estimates.push((frame, cadence.tick(enabled) == Estimate::Reestimate));
        }
        // Re-estimates at absolute frames 0 and 30 (both enabled, both
        // multiples of 30); frame 12's re-enable is not itself a multiple.
        let due: Vec<i32> = estimates
            .iter()
            .filter(|(_, d)| *d)
            .map(|(f, _)| *f)
            .collect();
        assert_eq!(due, vec![0, 30]);
    }

    #[test]
    fn the_counter_wraps_without_panicking() {
        // `idiv` on a wrapping `eax` never traps; a debug `+` would. `i32`
        // matches the register and `wrapping_add` matches the wrap.
        let mut cadence = Cadence { counter: i32::MAX };
        let _ = cadence.tick(true);
        assert_eq!(cadence.counter, i32::MIN);
    }

    #[test]
    fn the_external_setter_moves_the_phase_like_the_reset_thunk() {
        // Thunk `0x18284e5e0` stores an arbitrary value; set to one below a
        // multiple of 30 and the next tick is due.
        let mut cadence = Cadence::start();
        cadence.set(29);
        assert_eq!(cadence.tick(true), Estimate::Hold); // 29, not divisible
        assert_eq!(cadence.tick(true), Estimate::Reestimate); // 30
    }
}
