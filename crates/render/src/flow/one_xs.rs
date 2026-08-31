//! The selected native ONE X2 optical-flow coordinate and payload contract.
//!
//! This is intentionally not another interpretation of the legacy belt. The
//! retained native Mats are 1080 rows along the seam by 60 columns across it,
//! row-major `CV_32FC2`. Component zero is `dcol` across the seam and component
//! one is `drow` along it. Their units are already pixels of the retained grid;
//! Studio applies them 1:1 and does not divide them by the 3x source-grid ratio.
//! The shared `+0x810` body-to-line map supplies the initial coordinate `c`.
//! Flow produces `q`, then the per-lens `+0x8d0/+0x930` base UV maps consume
//! that displaced coordinate; those base maps do not register a ray into the
//! common line-coordinate frame.

use std::error::Error;
use std::fmt;

use super::dis::FlowField;
use super::{FlowLayout, GridAxis, SeamAxis, StorageOrder};

/// The selected input reduction and prior-image motion-reference front end.
pub mod temporal;

/// The selected two-pass sparse patch-search CPU oracle.
///
/// This remains inactive until the dedicated ONE X2 producer can wire the
/// complete pyramid, densification, variational and GPU path together.
pub mod pis;

/// The selected finest-level, component-zero temporal median.
///
/// This remains an inactive building block until the dedicated ONE X2 FDS
/// producer can place it between patch solving and densification.
pub mod temporal_median;

/// The selected scalar sparse-to-dense and pyramid-resize oracle.
pub mod dense;

#[cfg(test)]
mod native_chain;
/// Readable cold pair-level estimator used by the correctness oracle.
pub mod scalar;

/// Readable target-checkpoint warm pair composition.
pub mod warm;

/// Pair-atomic cold-first and warm-sequential estimator ownership.
pub mod owner;

/// The selected inactive scalar variational derivative preparation.
pub mod derivative_prep;

/// The selected inactive scalar variational-refinement oracle.
pub mod variational;

/// The selected inactive post-VR retained-public-flow update.
pub mod post_update;

/// The selected CPU periodic-boundary blend on each public flow field.
pub mod public_blend;

/// The selected level-two post-update to level-one PIS seed adapter.
pub mod l2_seed;

/// Inactive readable oracle for the ordinary right line-map patch call.
pub mod map_patch;

/// Inactive finite-coordinate specialization of Studio's pre-filter map merge.
pub mod base_map;

/// Static resources shared by every frame of a selected ONE X2 capture.
pub mod resources;

/// The capture-owned, cold-first sequential map transaction.
pub(crate) mod player;

/// Readable source-level transcription of Studio's selected Metal parent map.
///
/// This remains an inactive diagnostic until the general per-frame input owner
/// is wired.  It intentionally models the recovered Metal source, not the
/// separate binary64 CPU alternative.
#[doc(hidden)]
pub mod metal_calc_map;

/// Read selected ONE X2 calibration packing and captured parent schedule.
///
/// This remains an inactive diagnostic.  It stops before Studio's unresolved
/// pose provider and retained mapping operand.
#[doc(hidden)]
pub mod parent_inputs;

/// Per-frame selected ONE X2 parent-map construction.
///
/// The implementation stays private. Its narrow exported builder accepts a
/// `FrameStamp` directly, or an exact center which a frame owner has already
/// authenticated and remains responsible for binding to the result.
mod parent;

#[doc(hidden)]
pub use parent::{ParentMapBuilder, ParentMapError};

/// Studio's `LensTypeOneXS`, selected for the ONE X2 by its camera dispatcher.
pub const LENS_TYPE: u32 = 0x29;

/// The retained field has 1080 rows around the seam and 60 columns across it.
pub const ROWS: usize = 1080;
pub const COLS: usize = 60;

/// FDS's selected sparse-patch geometry, shared by PIS and temporal median.
pub const PATCH_SIZE: usize = 8;
pub const PATCH_STRIDE: usize = 3;

/// The captured staging images are exactly 3x the retained grid on both axes.
/// The direct route area-reduces 3240 by 180 into [`ROWS`] by [`COLS`].
pub const SOURCE_ROWS: usize = 3240;
pub const SOURCE_COLS: usize = 180;

/// The two physical images in Studio's selected ONE X2 pair.
///
/// `A` is native lens 0 and `B` is native lens 1. Keeping that identity in the
/// type avoids reintroducing the numeric lens swaps that the V6 capture closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lens {
    A,
    B,
}

impl Lens {
    pub const ALL: [Self; 2] = [Self::A, Self::B];

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
        }
    }
}

impl fmt::Display for Lens {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::A => out.write_str("A"),
            Self::B => out.write_str("B"),
        }
    }
}

/// A value for each native lens, named rather than positionally ordered.
///
/// Construct this with a struct literal so `a` and `b` remain visible at every
/// raw-payload boundary. There is deliberately no positional `new(a, b)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LensPair<T> {
    pub a: T,
    pub b: T,
}

impl<T> LensPair<T> {
    pub const fn get(&self, lens: Lens) -> &T {
        match lens {
            Lens::A => &self.a,
            Lens::B => &self.b,
        }
    }
}

/// The semantic direction of a selected ONE X2 solver result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    AtoB,
    BtoA,
}

impl fmt::Display for Direction {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AtoB => out.write_str("A-to-B"),
            Self::BtoA => out.write_str("B-to-A"),
        }
    }
}

/// Studio's exact selected PIS interval for one direction and pyramid level.
///
/// The accepted row-6 packet reads all three A-to-B table entries. The pinned
/// worker first fills every entry from the raw pair, then divides entry `L`
/// twice by `2^L`, so each level is the raw pair divided by `4^L`. The same
/// worker initializes B-to-A with the exact reversed sign/slot exchange shown
/// below. Explicit bits keep this readable oracle tied to those READ values.
pub(super) const fn selected_pis_interval(
    direction: Direction,
    level: pis::Level,
) -> pis::DisparityInterval {
    match (direction, level) {
        (Direction::AtoB, pis::Level::One) => pis::DisparityInterval::new(
            [f32::from_bits(0xc170_0000), f32::from_bits(0xbeff_fc66)],
            [f32::from_bits(0x3f7f_fc66), f32::from_bits(0x3eff_fc66)],
        ),
        (Direction::AtoB, pis::Level::Two) => pis::DisparityInterval::new(
            [f32::from_bits(0xc070_0000), f32::from_bits(0xbdff_fc66)],
            [f32::from_bits(0x3e7f_fc66), f32::from_bits(0x3dff_fc66)],
        ),
        // The second FDS instance stores +A first and -B second. PIS's strict
        // gate is order-agnostic, so the reversed endpoint order is retained.
        (Direction::BtoA, pis::Level::One) => pis::DisparityInterval::new(
            [f32::from_bits(0x4170_0000), f32::from_bits(0x3eff_fc66)],
            [f32::from_bits(0xbf7f_fc66), f32::from_bits(0xbeff_fc66)],
        ),
        (Direction::BtoA, pis::Level::Two) => pis::DisparityInterval::new(
            [f32::from_bits(0x4070_0000), f32::from_bits(0x3dff_fc66)],
            [f32::from_bits(0xbe7f_fc66), f32::from_bits(0xbdff_fc66)],
        ),
    }
}

/// A full retained row step in the selected 400-degree unrolled line image.
pub const DEGREES_PER_ROW: f32 = 400.0 / (ROWS as f32 - 1.0);

/// The selected `SeamlessBlenderImpl+0x3d0` lateral gnomonic scale.
pub const LATERAL_SCALE: f32 = 1.0;

/// Pixels per unit of the across-seam gnomonic coordinate. Studio derives this
/// from the row angular pitch: `scale * 360 / pitch / (2*pi)`.
pub const GNOMONIC_PIXELS: f32 = LATERAL_SCALE * 360.0 / DEGREES_PER_ROW / std::f32::consts::TAU;

/// The centre of a 60-column pixel grid. The line law is node-centred, hence
/// `(COLS - 1) / 2 = 29.5`, rather than 30.
pub const CENTRE_COL: f32 = (COLS as f32 - 1.0) * 0.5;

/// A zero-sized witness for the ONE X2 coordinate law.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Layout;

impl Layout {
    /// Static layout metadata used at the route-selection boundary.
    pub const FLOW: FlowLayout = FlowLayout {
        rows: ROWS,
        cols: COLS,
        row_axis: SeamAxis::Along,
        col_axis: SeamAxis::Across,
        storage: StorageOrder::RowMajor,
        components: [GridAxis::Column, GridAxis::Row],
    };

    /// Describe the retained coordinate of a direction in Kjerag's body frame.
    ///
    /// The row unrolls 400 degrees, from -200 at row 0 to +200 at row 1079:
    /// the line-frame law is `phi = atan2(x,z)` and
    /// `row = (phi+200)/pitch`. The column is a gnomonic coordinate about 29.5:
    /// `col = 29.5 - K*y/hypot(x,z)`. The body direction is first registered
    /// back into that line frame with `line=[-body.x,body.z,body.y]`. Both
    /// coordinates clamp exactly where the later flow sampler clamps them. At
    /// runtime the shared `+0x810` map supplies the initial coordinate directly.
    /// The per-lens `+0x8d0/+0x930` base maps are downstream: they consume
    /// displaced `q` and produce UV.
    ///
    /// A [`LineRay`] is deliberately not accepted here. The explicit adapter
    /// keeps a retained-inverse intermediate from being passed to code that
    /// expects Kjerag's body axes.
    ///
    /// ```compile_fail
    /// use kjerag_render::flow::one_xs::{Layout, Sample};
    ///
    /// let retained = Sample::new(539.5, 29.5).unwrap();
    /// let line = Layout.line_ray(retained);
    /// let _ = Layout.sample(line);
    /// ```
    pub fn sample(self, ray: BodyRay) -> Option<Sample> {
        let [x, y, z] = ray.line_ray().0;
        let rho = x.hypot(z);
        if !rho.is_finite() || rho <= 0.0 {
            return None;
        }
        let phi_deg = x.atan2(z).to_degrees();
        let row = (phi_deg + 200.0) / DEGREES_PER_ROW;
        let col = CENTRE_COL - GNOMONIC_PIXELS * y / rho;
        Sample::new(row, col).map(Sample::clamped)
    }

    /// The native inverse retained-grid law in its line-image frame. The
    /// 400-degree row range intentionally overlaps itself by 40 degrees, so
    /// more than one row can name the same unit direction; this returns the
    /// direction for the supplied row without canonicalising that overlap.
    pub fn line_ray(self, sample: Sample) -> LineRay {
        let sample = sample.clamped();
        let (row, col) = (sample.row(), sample.col());
        let phi = (-200.0 + row * DEGREES_PER_ROW).to_radians();
        let t = (CENTRE_COL - col) / GNOMONIC_PIXELS;
        let rho = (1.0 + t * t).sqrt().recip();
        LineRay([phi.sin() * rho, t * rho, phi.cos() * rho])
    }

    /// The retained-grid inverse registered into Kjerag's body frame.
    ///
    /// The exact proper rotation is `body=[-line.x,line.z,line.y]`. Keeping the
    /// line result in [`LineRay`] until this call makes the registration visible
    /// at the type boundary.
    ///
    /// The 400-degree row range intentionally
    /// overlaps itself by 40 degrees, so more than one row can name the same
    /// unit direction; this returns the direction for the supplied row without
    /// canonicalising that overlap.
    pub fn body_ray(self, sample: Sample) -> BodyRay {
        self.line_ray(sample).body_ray()
    }
}

/// A direction in the selected retained inverse's native line-image frame.
///
/// This intermediate is not a Kjerag body direction. It must cross the explicit
/// [`LineRay::body_ray`] adapter before body-frame projection code can consume
/// it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineRay([f32; 3]);

impl LineRay {
    /// Admit a finite nonzero line-frame direction. Magnitude is immaterial.
    pub fn new(ray: [f32; 3]) -> Option<Self> {
        (ray.iter().all(|v| v.is_finite()) && ray.iter().any(|v| *v != 0.0)).then_some(Self(ray))
    }

    pub const fn components(self) -> [f32; 3] {
        self.0
    }

    /// Apply the closed line-to-body proper rotation.
    pub const fn body_ray(self) -> BodyRay {
        let [x, y, z] = self.0;
        BodyRay([-x, z, y])
    }
}

/// A direction in Kjerag's body frame after the selected ONE X2 registration.
/// This is not a raw lens ray. Studio's shared `+0x810` map supplies its initial
/// line coordinate `c`; the per-lens `+0x8d0/+0x930` base maps instead consume
/// displaced `q` and return source UV, so they are not registration transforms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyRay([f32; 3]);

impl BodyRay {
    /// Admit a finite nonzero direction. Magnitude is immaterial to the law.
    pub fn new(ray: [f32; 3]) -> Option<Self> {
        (ray.iter().all(|v| v.is_finite()) && ray.iter().any(|v| *v != 0.0)).then_some(Self(ray))
    }

    pub const fn components(self) -> [f32; 3] {
        self.0
    }

    /// Undo the closed registration before applying the retained line law.
    pub const fn line_ray(self) -> LineRay {
        let [x, y, z] = self.0;
        LineRay([-x, z, y])
    }
}

/// Fractional retained-grid coordinates. Rows run along the seam; columns run
/// across it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    row: f32,
    col: f32,
}

impl Sample {
    /// Construct a finite coordinate. Coordinates outside the retained grid
    /// remain representable until a sampling or displacement boundary clamps
    /// them.
    pub fn new(row: f32, col: f32) -> Option<Self> {
        (row.is_finite() && col.is_finite()).then_some(Self { row, col })
    }

    pub const fn row(self) -> f32 {
        self.row
    }

    pub const fn col(self) -> f32 {
        self.col
    }

    pub fn clamped(self) -> Self {
        Self {
            row: self.row.clamp(0.0, ROWS as f32 - 1.0),
            col: self.col.clamp(0.0, COLS as f32 - 1.0),
        }
    }

    /// Apply a retained flow vector 1:1. Component zero moves the column and
    /// component one moves the row; the result clamps at the native grid edge.
    /// A non-finite weight or non-finite result is rejected.
    pub fn checked_displaced(self, flow: FlowVector, weight: f32) -> Option<Self> {
        if !weight.is_finite() {
            return None;
        }
        Self::new(
            self.row + weight * flow.drow(),
            self.col + weight * flow.dcol(),
        )
        .map(Self::clamped)
    }
}

/// One sampled native flow vector, already in retained-grid pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FlowVector {
    /// Across-seam column displacement, native component zero.
    dcol: f32,
    /// Along-seam row displacement, native component one.
    drow: f32,
}

impl FlowVector {
    /// Construct a displacement whose two retained-pixel components are finite.
    pub fn new(dcol: f32, drow: f32) -> Option<Self> {
        (dcol.is_finite() && drow.is_finite()).then_some(Self { dcol, drow })
    }

    pub const fn dcol(self) -> f32 {
        self.dcol
    }

    pub const fn drow(self) -> f32 {
        self.drow
    }
}

/// A selected ONE X2 solver field with route-specific component semantics.
///
/// The shared DIS engine calls its horizontal and vertical components `u` and
/// `v`. At this boundary those names stop: on the ONE X2 grid they are column
/// displacement across the seam and row displacement along it, respectively.
#[derive(Clone, Debug)]
struct NativeField {
    dcol: Vec<f32>,
    drow: Vec<f32>,
    valid: Vec<bool>,
}

impl NativeField {
    fn from_solver(field: FlowField, direction: Direction) -> Result<Self, FieldShapeError> {
        let pixels = ROWS * COLS;
        for (part, expected, actual) in [
            ("columns", COLS, field.width),
            ("rows", ROWS, field.height),
            ("dcol plane", pixels, field.u.len()),
            ("drow plane", pixels, field.v.len()),
            ("validity plane", pixels, field.valid.len()),
        ] {
            if actual != expected {
                return Err(FieldShapeError {
                    direction,
                    part,
                    expected,
                    actual,
                });
            }
        }
        Ok(Self {
            dcol: field.u,
            drow: field.v,
            valid: field.valid,
        })
    }

    #[cfg(test)]
    fn from_public<D: pis::PisDirection>(field: dense::PublicDenseField<D>) -> (Self, usize) {
        let (dcol, drow) = field.into_components();
        let valid = dcol
            .iter()
            .zip(&drow)
            .map(|(dcol, drow)| dcol.is_finite() && drow.is_finite())
            .collect::<Vec<_>>();
        let invalid = valid.iter().filter(|valid| !**valid).count();
        (
            Self {
                dcol: dcol.into_vec(),
                drow: drow.into_vec(),
                valid,
            },
            invalid,
        )
    }

    /// Copy one borrowed public field into the existing displacement adapter.
    ///
    /// This narrow checkpoint-only bridge lets the warm transition retain its
    /// linear public-field token without making [`dense::PublicDenseField`]
    /// generally cloneable.
    fn from_public_ref<D: pis::PisDirection>(field: &dense::PublicDenseField<D>) -> (Self, usize) {
        let dcol = field.dcol().to_vec();
        let drow = field.drow().to_vec();
        let valid = dcol
            .iter()
            .zip(&drow)
            .map(|(dcol, drow)| dcol.is_finite() && drow.is_finite())
            .collect::<Vec<_>>();
        let invalid = valid.iter().filter(|valid| !**valid).count();
        (Self { dcol, drow, valid }, invalid)
    }
}

/// A checked A-to-B result. It is consumed by lens B after composition.
#[derive(Clone, Debug)]
pub struct AtoBField(NativeField);

impl AtoBField {
    /// Mark the output of the solver call whose source is lens A and target is
    /// lens B. Shape is checked before the semantic marker can be constructed.
    pub fn from_solver(field: FlowField) -> Result<Self, FieldShapeError> {
        NativeField::from_solver(field, Direction::AtoB).map(Self)
    }
}

/// A checked B-to-A result. It is consumed by lens A after composition.
#[derive(Clone, Debug)]
pub struct BtoAField(NativeField);

impl BtoAField {
    /// Mark the output of the solver call whose source is lens B and target is
    /// lens A. Shape is checked before the semantic marker can be constructed.
    pub fn from_solver(field: FlowField) -> Result<Self, FieldShapeError> {
        NativeField::from_solver(field, Direction::BtoA).map(Self)
    }
}

/// Both directed solver results, with a constructor whose arguments cannot be
/// exchanged accidentally.
///
/// ```compile_fail
/// use kjerag_render::flow::one_xs::{AtoBField, BtoAField, DirectedFields};
///
/// fn swapped(a_to_b: AtoBField, b_to_a: BtoAField) {
///     let _ = DirectedFields::new(b_to_a, a_to_b);
/// }
/// ```
#[derive(Clone, Debug)]
pub struct DirectedFields {
    a_to_b: AtoBField,
    b_to_a: BtoAField,
}

impl DirectedFields {
    pub const fn new(a_to_b: AtoBField, b_to_a: BtoAField) -> Self {
        Self { a_to_b, b_to_a }
    }

    /// Consume the scalar producer's direction-labelled public fields.
    ///
    /// Selected main densification extends the final sparse patch across its
    /// residual bottom and right fringe, so ordinary selected output is finite.
    /// A vector is nevertheless valid only when both components are finite;
    /// one non-finite component invalidates the whole vector, and composition
    /// performs the only zero-fill. This is a scalar-oracle boundary, not a
    /// native validity law: captured V6 public fields are finite at every node.
    #[cfg(test)]
    pub(super) fn from_public_dense(
        a_to_b: dense::PublicDenseField<pis::AtoB>,
        b_to_a: dense::PublicDenseField<pis::BtoA>,
    ) -> (Self, InvalidNodeCounts) {
        let (a_to_b, a_to_b_invalid) = NativeField::from_public(a_to_b);
        let (b_to_a, b_to_a_invalid) = NativeField::from_public(b_to_a);
        (
            Self::new(AtoBField(a_to_b), BtoAField(b_to_a)),
            InvalidNodeCounts {
                a_to_b: a_to_b_invalid,
                b_to_a: b_to_a_invalid,
            },
        )
    }

    /// Borrow the scalar producer's direction-labelled public fields.
    ///
    /// Only the checkpoint transition needs this bridge: it returns the
    /// original linear tokens as known next state while composing a diagnostic
    /// displacement from private copies.
    pub(super) fn from_public_dense_ref(
        a_to_b: &dense::PublicDenseField<pis::AtoB>,
        b_to_a: &dense::PublicDenseField<pis::BtoA>,
    ) -> (Self, InvalidNodeCounts) {
        let (a_to_b, a_to_b_invalid) = NativeField::from_public_ref(a_to_b);
        let (b_to_a, b_to_a_invalid) = NativeField::from_public_ref(b_to_a);
        (
            Self::new(AtoBField(a_to_b), BtoAField(b_to_a)),
            InvalidNodeCounts {
                a_to_b: a_to_b_invalid,
                b_to_a: b_to_a_invalid,
            },
        )
    }
}

/// Non-finite public nodes suppressed at the scalar-to-apply boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InvalidNodeCounts {
    pub a_to_b: usize,
    pub b_to_a: usize,
}

/// A directed solver result did not have the selected 1080-row by 60-column
/// shape. Constructing the route-specific field fails before composition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldShapeError {
    direction: Direction,
    part: &'static str,
    expected: usize,
    actual: usize,
}

impl fmt::Display for FieldShapeError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "ONE X2 {} flow has {} {}, expected {}",
            self.direction, self.actual, self.part, self.expected,
        )
    }
}

impl Error for FieldShapeError {}

/// The two selected native directional fields in their upload-ready planar
/// layout: `[lensA.dcol, lensA.drow, lensB.dcol, lensB.drow]`.
///
/// Lens A consumes B-to-A (`c90`); lens B consumes A-to-B (`c30`). This is a
/// distinct type from the legacy displacement so neither route can silently
/// sample the other's dimensions or coordinate law.
#[derive(Clone, Debug, PartialEq)]
pub struct Displacement {
    planes: Vec<f32>,
}

impl Displacement {
    pub const FLOATS: usize = 4 * ROWS * COLS;
    pub const BYTES: usize = Self::FLOATS * std::mem::size_of::<f32>();

    pub fn zeros() -> Self {
        Self {
            planes: vec![0.0; Self::FLOATS],
        }
    }

    /// Compose Studio's two typed directed fields without a taper, sign change,
    /// axis swap or scale change. Invalid estimator votes become zero.
    pub fn compose(fields: &DirectedFields) -> Self {
        let n = ROWS * COLS;
        let mut planes = vec![0.0; 4 * n];
        // Apply ownership is cross-directed: lens A reads B->A, lens B A->B.
        for (lens, field) in [(Lens::A, &fields.b_to_a.0), (Lens::B, &fields.a_to_b.0)] {
            let base = lens.index() * 2 * n;
            for index in 0..n {
                if field.valid[index] {
                    let vector = FlowVector::new(field.dcol[index], field.drow[index])
                        .expect("valid ONE X2 flow vector is not finite");
                    planes[base + index] = vector.dcol();
                    planes[base + n + index] = vector.drow();
                }
            }
        }
        Self { planes }
    }

    /// Bilinearly sample one lens's field. Both coordinates clamp, matching the
    /// native apply. Returned units remain pixels of this 1080 by 60 grid.
    pub fn sample(&self, lens: Lens, sample: Sample) -> FlowVector {
        let sample = sample.clamped();
        let (row, col) = (sample.row(), sample.col());
        let n = ROWS * COLS;
        let base = lens.index() * 2 * n;
        let at = |plane: usize, row: i64, col: i64| {
            let row = row.clamp(0, ROWS as i64 - 1) as usize;
            let col = col.clamp(0, COLS as i64 - 1) as usize;
            self.planes[base + plane * n + row * COLS + col]
        };
        let row0 = row.floor();
        let col0 = col.floor();
        let (fr, fc) = (row - row0, col - col0);
        let (row0, col0) = (row0 as i64, col0 as i64);
        let component = |plane| {
            let top =
                at(plane, row0, col0) + (at(plane, row0, col0 + 1) - at(plane, row0, col0)) * fc;
            let bottom = at(plane, row0 + 1, col0)
                + (at(plane, row0 + 1, col0 + 1) - at(plane, row0 + 1, col0)) * fc;
            top + (bottom - top) * fr
        };
        FlowVector::new(component(0), component(1))
            .expect("composed ONE X2 displacement sampled a non-finite vector")
    }

    /// Native-endian f32 planes ready for the future dedicated GPU buffer.
    pub fn bytes(&self) -> &[u8] {
        // `[f32]` has no padding and every bit pattern is uploadable.
        unsafe {
            std::slice::from_raw_parts(
                self.planes.as_ptr().cast::<u8>(),
                std::mem::size_of_val(self.planes.as_slice()),
            )
        }
    }

    pub fn planes(&self) -> &[f32] {
        &self.planes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{FlowRoute, GridAxis, SeamAxis, StorageOrder};

    fn near(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} is not within {tolerance} of {expected}",
        );
    }

    fn constant(dcol: f32, drow: f32) -> FlowField {
        let n = ROWS * COLS;
        FlowField {
            width: COLS,
            height: ROWS,
            u: vec![dcol; n],
            v: vec![drow; n],
            valid: vec![true; n],
            residual: vec![f32::INFINITY; n],
        }
    }

    fn coord(row: f32, col: f32) -> Sample {
        Sample::new(row, col).unwrap()
    }

    fn vector(dcol: f32, drow: f32) -> FlowVector {
        FlowVector::new(dcol, drow).unwrap()
    }

    fn pattern(base: f32, row: f32, col: f32) -> f32 {
        base + row * 64.0 + col
    }

    fn patterned(dcol_base: f32, drow_base: f32) -> FlowField {
        let mut field = constant(0.0, 0.0);
        for row in 0..ROWS {
            for col in 0..COLS {
                let index = row * COLS + col;
                field.u[index] = pattern(dcol_base, row as f32, col as f32);
                field.v[index] = pattern(drow_base, row as f32, col as f32);
            }
        }
        field
    }

    fn directed(a_to_b: FlowField, b_to_a: FlowField) -> DirectedFields {
        DirectedFields::new(
            AtoBField::from_solver(a_to_b).unwrap(),
            BtoAField::from_solver(b_to_a).unwrap(),
        )
    }

    #[test]
    fn selected_pis_intervals_have_the_read_per_level_bits() {
        let f = f32::from_bits;
        assert_eq!(
            selected_pis_interval(Direction::AtoB, pis::Level::One),
            pis::DisparityInterval::new(
                [f(0xc170_0000), f(0xbeff_fc66)],
                [f(0x3f7f_fc66), f(0x3eff_fc66)],
            ),
        );
        assert_eq!(
            selected_pis_interval(Direction::AtoB, pis::Level::Two),
            pis::DisparityInterval::new(
                [f(0xc070_0000), f(0xbdff_fc66)],
                [f(0x3e7f_fc66), f(0x3dff_fc66)],
            ),
        );
        assert_eq!(
            selected_pis_interval(Direction::BtoA, pis::Level::One),
            pis::DisparityInterval::new(
                [f(0x4170_0000), f(0x3eff_fc66)],
                [f(0xbf7f_fc66), f(0xbeff_fc66)],
            ),
        );
        assert_eq!(
            selected_pis_interval(Direction::BtoA, pis::Level::Two),
            pis::DisparityInterval::new(
                [f(0x4070_0000), f(0x3dff_fc66)],
                [f(0xbe7f_fc66), f(0xbdff_fc66)],
            ),
        );
    }

    #[test]
    fn lens_type_selects_the_native_axis_contract_only_for_one_xs() {
        assert_eq!(FlowRoute::for_lens_type(LENS_TYPE), FlowRoute::OneXs);
        assert_eq!(FlowRoute::for_lens_type(0), FlowRoute::Legacy);
        assert_eq!(FlowRoute::for_lens_type(0x2a), FlowRoute::Legacy);

        let native = FlowRoute::OneXs.layout();
        assert_eq!(native.rows, 1080);
        assert_eq!(native.cols, 60);
        assert_eq!(native.row_axis, SeamAxis::Along);
        assert_eq!(native.col_axis, SeamAxis::Across);
        assert_eq!(native.storage, StorageOrder::RowMajor);
        assert_eq!(native.components, [GridAxis::Column, GridAxis::Row]);

        // The descriptor names the old semantics but does not alter the old
        // payload, buffer or shader: its shape remains the band constants.
        let legacy = FlowRoute::Legacy.layout();
        assert_eq!(legacy.rows, crate::band::STRIP_H);
        assert_eq!(legacy.cols, crate::band::STRIP_W);
        assert_eq!(legacy.row_axis, SeamAxis::Across);
        assert_eq!(legacy.col_axis, SeamAxis::Along);
    }

    #[test]
    fn retained_coordinates_round_trip_without_transposing_the_axes() {
        // Stay outside the deliberate 40-degree row overlap, where two rows
        // can name the same direction and an inverse cannot pick both.
        for original in [coord(100.25, 2.5), coord(539.5, 29.5), coord(999.75, 56.5)] {
            let line = Layout.line_ray(original);
            let body = Layout.body_ray(original);
            let round_trip = Layout.sample(body).unwrap();
            near(round_trip.row(), original.row(), 2e-4);
            near(round_trip.col(), original.col(), 2e-5);
            for ray in [line.components(), body.components()] {
                let length = ray.into_iter().map(|v| v * v).sum::<f32>().sqrt();
                near(length, 1.0, 2e-6);
            }
        }
    }

    #[test]
    fn line_to_body_adapter_is_the_closed_proper_rotation() {
        let body_x = LineRay::new([1.0, 0.0, 0.0])
            .unwrap()
            .body_ray()
            .components();
        let body_y = LineRay::new([0.0, 1.0, 0.0])
            .unwrap()
            .body_ray()
            .components();
        let body_z = LineRay::new([0.0, 0.0, 1.0])
            .unwrap()
            .body_ray()
            .components();
        assert_eq!(body_x, [-1.0, 0.0, 0.0]);
        assert_eq!(body_y, [0.0, 0.0, 1.0]);
        assert_eq!(body_z, [0.0, 1.0, 0.0]);

        // A proper rotation preserves handedness: R(X) cross R(Y) = R(Z).
        let cross = [
            body_x[1] * body_y[2] - body_x[2] * body_y[1],
            body_x[2] * body_y[0] - body_x[0] * body_y[2],
            body_x[0] * body_y[1] - body_x[1] * body_y[0],
        ];
        assert_eq!(cross, body_z);

        // This fixed asymmetric vector proves the independently decoded
        // inverse order and signs, not merely adapter(adapter(x)) == x.
        let recovered = BodyRay::new([-2.0, 5.0, 3.0])
            .unwrap()
            .line_ray()
            .components();
        assert_eq!(recovered, [2.0, 3.0, 5.0]);
    }

    #[test]
    fn retained_inverse_exposes_line_axes_then_registered_body_axes() {
        let at_phi = |phi: f32, t: f32| {
            coord(
                (phi + 200.0) / DEGREES_PER_ROW,
                CENTRE_COL - GNOMONIC_PIXELS * t,
            )
        };

        let forward_line = Layout.line_ray(at_phi(0.0, 0.0)).components();
        near(forward_line[0], 0.0, 2e-6);
        near(forward_line[1], 0.0, 2e-6);
        near(forward_line[2], 1.0, 2e-6);
        let forward = Layout.body_ray(at_phi(0.0, 0.0)).components();
        near(forward[0], 0.0, 2e-6);
        near(forward[1], 1.0, 2e-6);
        near(forward[2], 0.0, 2e-6);

        let positive_x_line = Layout.line_ray(at_phi(90.0, 0.0)).components();
        near(positive_x_line[0], 1.0, 2e-6);
        near(positive_x_line[1], 0.0, 2e-6);
        near(positive_x_line[2], 0.0, 2e-6);
        let positive_x = Layout.body_ray(at_phi(90.0, 0.0)).components();
        near(positive_x[0], -1.0, 2e-6);
        near(positive_x[1], 0.0, 2e-6);
        near(positive_x[2], 0.0, 2e-6);

        // An asymmetric point fixes all three line slots, then all three body
        // slots, without relying on a round trip between coupled functions.
        let t = 0.125;
        let rho = (1.0_f32 + t * t).sqrt().recip();
        let asymmetric_line = Layout.line_ray(at_phi(-30.0, t)).components();
        near(asymmetric_line[0], -0.5 * rho, 2e-6);
        near(asymmetric_line[1], t * rho, 2e-6);
        near(asymmetric_line[2], 30.0_f32.to_radians().cos() * rho, 2e-6);
        let asymmetric = Layout.body_ray(at_phi(-30.0, t)).components();
        near(asymmetric[0], 0.5 * rho, 2e-6);
        near(asymmetric[1], 30.0_f32.to_radians().cos() * rho, 2e-6);
        near(asymmetric[2], t * rho, 2e-6);
    }

    #[test]
    fn body_sample_uses_the_independent_inverse_adapter() {
        let centre = Layout
            .sample(BodyRay::new([0.0, 1.0, 0.0]).unwrap())
            .unwrap();
        near(centre.row(), 200.0 / DEGREES_PER_ROW, 2e-4);
        near(centre.col(), CENTRE_COL, 2e-5);

        let positive_quarter = Layout
            .sample(BodyRay::new([-1.0, 0.0, 0.0]).unwrap())
            .unwrap();
        near(positive_quarter.row(), 290.0 / DEGREES_PER_ROW, 2e-4);
        near(positive_quarter.col(), CENTRE_COL, 2e-5);

        let t = 0.125;
        let rho = (1.0_f32 + t * t).sqrt().recip();
        let asymmetric_body = [0.5 * rho, 30.0_f32.to_radians().cos() * rho, t * rho];
        let sampled = Layout
            .sample(BodyRay::new(asymmetric_body).unwrap())
            .unwrap();
        near(sampled.row(), 170.0 / DEGREES_PER_ROW, 2e-4);
        near(sampled.col(), CENTRE_COL - GNOMONIC_PIXELS * t, 2e-5);
    }

    #[test]
    fn plus_minus_180_rays_land_on_opposite_sides_of_the_interior_cut() {
        let at_angle = |degrees: f32| {
            let radians = degrees.to_radians();
            // line=[sin(phi),0,cos(phi)] registers to
            // body=[-sin(phi),cos(phi),0].
            BodyRay::new([-radians.sin(), radians.cos(), 0.0]).unwrap()
        };
        let positive = Layout.sample(at_angle(179.99)).unwrap();
        let negative = Layout.sample(at_angle(-179.99)).unwrap();

        near(positive.row(), (200.0 + 179.99) / DEGREES_PER_ROW, 2e-3);
        near(negative.row(), (200.0 - 179.99) / DEGREES_PER_ROW, 2e-3);
        near(positive.col(), CENTRE_COL, 1e-5);
        near(negative.col(), CENTRE_COL, 1e-5);
        assert!(positive.row() > 1000.0);
        assert!(negative.row() < 60.0);
    }

    #[test]
    fn twenty_degree_padding_clamps_only_at_the_retained_grid_edges() {
        let positive_cut_row = 20.0 / DEGREES_PER_ROW;
        let negative_cut_row = (400.0 - 20.0) / DEGREES_PER_ROW;
        near(positive_cut_row * DEGREES_PER_ROW, 20.0, 1e-5);
        near(
            (ROWS as f32 - 1.0 - negative_cut_row) * DEGREES_PER_ROW,
            20.0,
            1e-4,
        );

        let toward_top = coord(positive_cut_row, CENTRE_COL)
            .checked_displaced(vector(0.0, -100.0), 1.0)
            .unwrap();
        let toward_bottom = coord(negative_cut_row, CENTRE_COL)
            .checked_displaced(vector(0.0, 100.0), 1.0)
            .unwrap();
        assert_eq!(toward_top.row(), 0.0);
        assert_eq!(toward_bottom.row(), ROWS as f32 - 1.0);

        // The padded endpoints are unrolled ±200 degrees. In the canonical
        // atan2 range those same directions appear as ∓160 degrees.
        let canonical_angle = |sample| {
            let [x, _, z] = Layout.line_ray(sample).components();
            x.atan2(z).to_degrees()
        };
        near(canonical_angle(toward_top), 160.0, 2e-5);
        near(canonical_angle(toward_bottom), -160.0, 2e-5);
    }

    #[test]
    fn public_coordinates_vectors_and_weights_reject_nonfinite_values() {
        assert!(Sample::new(f32::NAN, 0.0).is_none());
        assert!(Sample::new(0.0, f32::INFINITY).is_none());
        assert!(LineRay::new([0.0, 0.0, 0.0]).is_none());
        assert!(LineRay::new([0.0, f32::NAN, 1.0]).is_none());
        assert!(BodyRay::new([0.0, 0.0, 0.0]).is_none());
        assert!(BodyRay::new([0.0, f32::NAN, 1.0]).is_none());
        assert!(FlowVector::new(f32::NEG_INFINITY, 0.0).is_none());
        assert!(FlowVector::new(0.0, f32::NAN).is_none());

        let sample = coord(100.0, 20.0);
        assert!(
            sample
                .checked_displaced(vector(1.0, 1.0), f32::NAN)
                .is_none()
        );
        assert!(
            sample
                .checked_displaced(vector(1.0, 1.0), f32::INFINITY)
                .is_none()
        );
        assert!(
            sample
                .checked_displaced(vector(f32::MAX, 0.0), 2.0)
                .is_none()
        );
    }

    #[test]
    fn component_zero_moves_across_and_component_one_moves_along_in_grid_pixels() {
        let start = coord(400.0, 20.0);
        let moved = start.checked_displaced(vector(4.0, -6.0), 0.25).unwrap();
        assert_eq!(moved, coord(398.5, 21.0));

        // No hidden 3x rescale: one retained pixel is one applied pixel.
        let one = start.checked_displaced(vector(1.0, 1.0), 1.0).unwrap();
        assert_eq!(one, coord(401.0, 21.0));
    }

    #[test]
    fn compose_keeps_cross_ownership_component_order_and_native_units() {
        let a_to_b = constant(3.0, 11.0);
        let b_to_a = constant(5.0, 13.0);
        let displacement = Displacement::compose(&directed(a_to_b, b_to_a));
        assert_eq!(displacement.planes().len(), Displacement::FLOATS);
        assert_eq!(displacement.bytes().len(), Displacement::BYTES);

        // Lens A samples B->A/c90; lens B samples A->B/c30.
        assert_eq!(
            displacement.sample(Lens::A, coord(700.5, 20.25)),
            vector(5.0, 13.0),
        );
        assert_eq!(
            displacement.sample(Lens::B, coord(700.5, 20.25)),
            vector(3.0, 11.0),
        );

        let n = ROWS * COLS;
        let index = 700 * COLS + 20;
        assert_eq!(displacement.planes()[index], 5.0); // lens A dcol
        assert_eq!(displacement.planes()[n + index], 13.0); // lens A drow
        assert_eq!(displacement.planes()[2 * n + index], 3.0); // lens B dcol
        assert_eq!(displacement.planes()[3 * n + index], 11.0); // lens B drow
    }

    #[test]
    fn scalar_public_adapter_invalidates_whole_vectors_and_keeps_cross_ownership() {
        let nodes = ROWS * COLS;
        let mut a_to_b_dcol = vec![1.0; nodes];
        let a_to_b_drow = vec![2.0; nodes];
        let b_to_a_dcol = vec![3.0; nodes];
        let mut b_to_a_drow = vec![4.0; nodes];
        a_to_b_dcol[0] = f32::NAN;
        b_to_a_drow[1] = f32::INFINITY;

        let a_to_b =
            dense::PublicDenseField::<pis::AtoB>::from_test_components(a_to_b_dcol, a_to_b_drow);
        let b_to_a =
            dense::PublicDenseField::<pis::BtoA>::from_test_components(b_to_a_dcol, b_to_a_drow);
        let (fields, invalid) = DirectedFields::from_public_dense(a_to_b, b_to_a);
        let displacement = Displacement::compose(&fields);

        assert_eq!(
            invalid,
            InvalidNodeCounts {
                a_to_b: 1,
                b_to_a: 1,
            },
        );
        // Lens A consumes B-to-A, whose second vector is invalid.
        assert_eq!(displacement.planes()[0], 3.0);
        assert_eq!(displacement.planes()[nodes], 4.0);
        assert_eq!(displacement.planes()[1], 0.0);
        assert_eq!(displacement.planes()[nodes + 1], 0.0);
        // Lens B consumes A-to-B, whose first vector is invalid.
        assert_eq!(displacement.planes()[2 * nodes], 0.0);
        assert_eq!(displacement.planes()[3 * nodes], 0.0);
        assert_eq!(displacement.planes()[2 * nodes + 1], 1.0);
        assert_eq!(displacement.planes()[3 * nodes + 1], 2.0);
    }

    #[test]
    fn nonuniform_fields_fix_row_col_order_and_every_planar_byte_offset() {
        let a_to_b = patterned(10_000.0, 20_000.0);
        let b_to_a = patterned(30_000.0, 40_000.0);
        let displacement = Displacement::compose(&directed(a_to_b, b_to_a));

        let row = 7.25;
        let col = 11.5;
        assert_eq!(
            displacement.sample(Lens::A, coord(row, col)),
            vector(pattern(30_000.0, row, col), pattern(40_000.0, row, col)),
        );
        assert_eq!(
            displacement.sample(Lens::B, coord(row, col)),
            vector(pattern(10_000.0, row, col), pattern(20_000.0, row, col)),
        );

        let n = ROWS * COLS;
        let (row, col) = (17usize, 23usize);
        let index = row * COLS + col;
        for (plane, base) in [30_000.0, 40_000.0, 10_000.0, 20_000.0]
            .into_iter()
            .enumerate()
        {
            let expected = pattern(base, row as f32, col as f32);
            let float_offset = plane * n + index;
            assert_eq!(displacement.planes()[float_offset], expected);

            let byte_offset = float_offset * std::mem::size_of::<f32>();
            let encoded: [u8; 4] = displacement.bytes()[byte_offset..byte_offset + 4]
                .try_into()
                .unwrap();
            assert_eq!(f32::from_ne_bytes(encoded), expected);
        }
    }

    #[test]
    fn invalid_votes_and_the_zero_payload_move_nothing() {
        let mut invalid = constant(8.0, -9.0);
        invalid.valid.fill(false);
        let composed = Displacement::compose(&directed(invalid.clone(), invalid));
        assert!(composed.planes().iter().all(|v| *v == 0.0));
        assert!(Displacement::zeros().planes().iter().all(|v| *v == 0.0));
    }

    #[test]
    fn route_markers_reject_transposed_and_truncated_solver_fields() {
        let mut transposed = constant(0.0, 0.0);
        (transposed.width, transposed.height) = (ROWS, COLS);
        let error = AtoBField::from_solver(transposed).unwrap_err();
        assert_eq!(
            error.to_string(),
            "ONE X2 A-to-B flow has 1080 columns, expected 60"
        );

        let mut truncated = constant(0.0, 0.0);
        truncated.v.pop();
        let error = BtoAField::from_solver(truncated).unwrap_err();
        assert_eq!(
            error.to_string(),
            "ONE X2 B-to-A flow has 64799 drow plane, expected 64800"
        );
    }

    #[test]
    fn source_and_retained_grids_are_exactly_three_to_one() {
        assert_eq!(SOURCE_ROWS, 3 * ROWS);
        assert_eq!(SOURCE_COLS, 3 * COLS);
        near(GNOMONIC_PIXELS, 154.555_36, 2e-5);
        near(DEGREES_PER_ROW, 0.370_713_62, 1e-8);
    }
}
