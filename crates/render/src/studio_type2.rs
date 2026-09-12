//! Frame-bound resources for Studio's selected ONE X2 type-2 map.
//!
//! The map and alpha payloads stay in their recovered 200-by-100 grids. A
//! [`OneXsMapFrame`] binds production or diagnostic resources to one exact
//! delivered pair. Ordinary playback uploads it through the direct type-2
//! draw. Dense rasterization and submission remain detached diagnostics made
//! from the exact final projection written by one
//! [`crate::ScenePipeline::prepare_one_xs_picture`] call.

use std::fmt;
use std::sync::Arc;

use kjerag_media::{FrameStamp, Size};

use crate::Reframe;
use crate::map_oracle::{CapturedMap, DenseMap};

pub const MAP_WIDTH: usize = crate::map_oracle::MAP_WIDTH;
pub const MAP_HEIGHT: usize = crate::map_oracle::MAP_HEIGHT;
pub const MAP_NODES: usize = MAP_WIDTH * MAP_HEIGHT;
pub const PACKED_BYTES: usize = MAP_NODES * size_of::<[f32; 4]>();
pub const ALPHA_BYTES: usize = MAP_NODES * size_of::<f32>();

/// Exact-sized type-2 packed UV payload.
///
/// No range, finiteness or sentinel rule is added here. The renderer resource
/// is an arbitrary float4 grid and every component bit reaches the oracle
/// unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct PackedMap(Box<[[f32; 4]; MAP_NODES]>);

impl PackedMap {
    pub fn new(nodes: Vec<[f32; 4]>) -> Result<Self, MapShapeError> {
        let count = nodes.len();
        let nodes = nodes
            .into_boxed_slice()
            .try_into()
            .map_err(|_| MapShapeError::Packed { count })?;
        Ok(Self(nodes))
    }

    pub fn nodes(&self) -> &[[f32; 4]; MAP_NODES] {
        &self.0
    }

    pub fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.0.as_ptr().cast::<u8>(), PACKED_BYTES) }
    }
}

/// Exact-sized selected left-alpha payload.
///
/// It remains separate from [`PackedMap`]: the packed resource and the alpha
/// resource have different producers and provenance in Studio.
#[derive(Clone, Debug, PartialEq)]
pub struct AlphaMap(Box<[f32; MAP_NODES]>);

impl AlphaMap {
    pub fn new(nodes: Vec<f32>) -> Result<Self, MapShapeError> {
        let count = nodes.len();
        let nodes = nodes
            .into_boxed_slice()
            .try_into()
            .map_err(|_| MapShapeError::Alpha { count })?;
        Ok(Self(nodes))
    }

    pub fn nodes(&self) -> &[f32; MAP_NODES] {
        &self.0
    }

    pub fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.0.as_ptr().cast::<u8>(), ALPHA_BYTES) }
    }
}

/// Two selected type-2 resources associated with one exact decoded pair.
///
/// Construction establishes association, not native provenance. Evidence
/// callers must still authenticate or compute each resource before binding
/// it; the type prevents that association from drifting afterward.
#[derive(Clone, Debug, PartialEq)]
pub struct OneXsMapFrame {
    frame: FrameStamp,
    packed: PackedMap,
    alpha: AlphaMap,
    pis_backend: PisBackend,
    fusion: Option<crate::image_fusion::RatioPair>,
}

/// Sparse-solver backend that produced one committed production map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PisBackend {
    Cpu,
    Gpu,
}

impl PisBackend {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
        }
    }
}

impl OneXsMapFrame {
    pub fn new(
        frame: FrameStamp,
        packed: PackedMap,
        alpha: AlphaMap,
        pis_backend: PisBackend,
    ) -> Self {
        Self {
            frame,
            packed,
            alpha,
            pis_backend,
            fusion: None,
        }
    }

    pub fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub fn packed(&self) -> &PackedMap {
        &self.packed
    }

    pub fn alpha(&self) -> &AlphaMap {
        &self.alpha
    }

    pub const fn pis_backend(&self) -> PisBackend {
        self.pis_backend
    }

    /// Attach explicit renderer-ordinal photometric maps to this same source.
    /// As with the geometric constructor, association is not authentication.
    /// This is a replay boundary; live playback has no captured-map selector.
    pub fn with_fusion(mut self, fusion: crate::image_fusion::RatioPair) -> Self {
        self.fusion = Some(fusion);
        self
    }

    pub fn fusion(&self) -> Option<&crate::image_fusion::RatioPair> {
        self.fusion.as_ref()
    }

    /// Rasterize the READ type-2 sphere for the exact picture a pipeline has
    /// prepared and the requested origin-zero target size.
    /// This returns geometric coordinates and alpha only; photometric ratios
    /// are consumed by the direct draw, not by this geometry/trace raster.
    ///
    /// This is the first readable implementation, intentionally kept behind
    /// an inactive typed boundary so a later GPU implementation can be checked
    /// against it without changing the producer contract.
    pub fn rasterize(
        &self,
        prepared: &PreparedPicture,
        size: Size,
    ) -> Result<OneXsMapRaster, FrameMapMismatch> {
        if self.frame != prepared.binding.frame {
            return Err(FrameMapMismatch::new(
                "map",
                &self.frame,
                "prepared picture",
                &prepared.binding.frame,
            ));
        }
        let map = CapturedMap::new(self.packed.0.to_vec(), self.alpha.0.to_vec())
            .expect("typed type-2 resources already have exact shape");
        Ok(OneXsMapRaster {
            prepared: prepared.binding.clone(),
            dense: map.rasterize(&prepared.reframe, size),
        })
    }
}

/// One exact picture preparation, including the final projection uniform.
///
/// Construction is private to [`crate::ScenePipeline`]. A fresh identity is
/// minted on every typed preparation, even when the same frame and view are
/// drawn again, so a raster cannot survive a later preparation accidentally.
#[derive(Clone, Debug)]
pub struct PreparedPicture {
    binding: PreparedBinding,
    reframe: Reframe,
    aspect: f32,
}

impl PreparedPicture {
    pub(crate) fn new(frame: FrameStamp, reframe: Reframe, aspect: f32) -> Self {
        Self {
            binding: PreparedBinding {
                identity: Preparation::new(),
                frame,
            },
            reframe,
            aspect,
        }
    }

    pub fn frame(&self) -> &FrameStamp {
        &self.binding.frame
    }

    /// Whether this preparation carries the projection contract consumed by
    /// a selected ONE X2 type-2 map.
    ///
    /// Detached evidence tools decode a fresh exact source delivery rather
    /// than restoring playback state. They use this narrow check to refuse a
    /// future ordinary-path change that would add a legacy seam shift or
    /// prepare a different camera family under an authenticated type-2 map.
    pub fn uses_one_xs_type2_projection(&self) -> bool {
        self.reframe.is_one_xs_pair() && self.reframe.handover_shift().to_bits() == 0.0f32.to_bits()
    }

    pub(crate) fn aspect(&self) -> f32 {
        self.aspect
    }

    pub(crate) const fn reframe(&self) -> Reframe {
        self.reframe
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedBinding {
    identity: Preparation,
    frame: FrameStamp,
}

impl PartialEq for PreparedBinding {
    fn eq(&self, other: &Self) -> bool {
        self.frame == other.frame && self.identity == other.identity
    }
}

impl Eq for PreparedBinding {}

#[derive(Clone, Debug)]
struct Preparation(Arc<()>);

impl Preparation {
    fn new() -> Self {
        Self(Arc::new(()))
    }
}

impl PartialEq for Preparation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Preparation {}

/// Dense first implementation of one frame-bound type-2 map.
///
/// Fields are private so a caller cannot attach arbitrary dense pixels to a
/// delivery. Only [`OneXsMapFrame::rasterize`] constructs this value.
#[derive(Clone, Debug, PartialEq)]
pub struct OneXsMapRaster {
    prepared: PreparedBinding,
    dense: DenseMap,
}

impl OneXsMapRaster {
    pub fn frame(&self) -> &FrameStamp {
        &self.prepared.frame
    }

    pub fn size(&self) -> Size {
        self.dense.size
    }

    pub fn uncovered(&self) -> usize {
        self.dense.uncovered
    }

    pub fn dense(&self) -> &DenseMap {
        &self.dense
    }

    pub(crate) fn prepared(&self) -> &PreparedBinding {
        &self.prepared
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapShapeError {
    Packed { count: usize },
    Alpha { count: usize },
}

impl fmt::Display for MapShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Packed { count } => write!(
                f,
                "type-2 packed map has {count} nodes, expected {MAP_WIDTH} by {MAP_HEIGHT}"
            ),
            Self::Alpha { count } => write!(
                f,
                "type-2 alpha map has {count} nodes, expected {MAP_WIDTH} by {MAP_HEIGHT}"
            ),
        }
    }
}

impl std::error::Error for MapShapeError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameMapMismatch {
    first_label: &'static str,
    first: FrameStamp,
    second_label: &'static str,
    second: FrameStamp,
}

impl FrameMapMismatch {
    pub(crate) fn new(
        first_label: &'static str,
        first: &FrameStamp,
        second_label: &'static str,
        second: &FrameStamp,
    ) -> Self {
        Self {
            first_label,
            first: first.clone(),
            second_label,
            second: second.clone(),
        }
    }
}

impl fmt::Display for FrameMapMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ONE X2 {} is frame {} at {:.6} s, but {} is frame {} at {:.6} s from another delivery",
            self.first_label,
            self.first.index(),
            self.first.timestamp().as_secs_f64(),
            self.second_label,
            self.second.index(),
            self.second.timestamp().as_secs_f64(),
        )
    }
}

impl std::error::Error for FrameMapMismatch {}

#[derive(Clone, Debug, PartialEq)]
pub enum MapBindError {
    NoPreparedFrame {
        map: FrameStamp,
    },
    PreparedMismatch {
        map: FrameStamp,
        prepared: FrameStamp,
    },
    ExtentMismatch {
        map: Size,
        target: Size,
    },
    TargetShape {
        dimension: wgpu::TextureDimension,
        depth_or_array_layers: u32,
        mip_levels: u32,
        samples: u32,
    },
    TargetFormat {
        prepared: wgpu::TextureFormat,
        target: wgpu::TextureFormat,
    },
    TargetUsage {
        target: wgpu::TextureUsages,
    },
    TargetAspect {
        prepared: f32,
        target: f32,
    },
    Uncovered {
        count: usize,
    },
}

impl MapBindError {
    pub(crate) fn require_frame(
        map: &FrameStamp,
        prepared: Option<&PreparedPicture>,
    ) -> Result<(), Self> {
        let Some(prepared) = prepared else {
            return Err(Self::NoPreparedFrame { map: map.clone() });
        };
        if map != &prepared.binding.frame {
            return Err(Self::PreparedMismatch {
                map: map.clone(),
                prepared: prepared.binding.frame.clone(),
            });
        }
        Ok(())
    }

    pub(crate) fn require(
        map: &PreparedBinding,
        prepared: Option<&PreparedPicture>,
    ) -> Result<(), Self> {
        let Some(prepared) = prepared else {
            return Err(Self::NoPreparedFrame {
                map: map.frame.clone(),
            });
        };
        if map != &prepared.binding {
            return Err(Self::PreparedMismatch {
                map: map.frame.clone(),
                prepared: prepared.binding.frame.clone(),
            });
        }
        Ok(())
    }

    pub(crate) fn require_extent(map: Size, target: Size) -> Result<(), Self> {
        if map != target {
            return Err(Self::ExtentMismatch { map, target });
        }
        Ok(())
    }

    pub(crate) fn require_target(
        map: Size,
        target: &wgpu::Texture,
        prepared_format: wgpu::TextureFormat,
    ) -> Result<(), Self> {
        let extent = target.size();
        if target.dimension() != wgpu::TextureDimension::D2
            || extent.depth_or_array_layers != 1
            || target.mip_level_count() != 1
            || target.sample_count() != 1
        {
            return Err(Self::TargetShape {
                dimension: target.dimension(),
                depth_or_array_layers: extent.depth_or_array_layers,
                mip_levels: target.mip_level_count(),
                samples: target.sample_count(),
            });
        }
        if target.format() != prepared_format {
            return Err(Self::TargetFormat {
                prepared: prepared_format,
                target: target.format(),
            });
        }
        if !target
            .usage()
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return Err(Self::TargetUsage {
                target: target.usage(),
            });
        }
        Self::require_extent(map, Size::new(extent.width, extent.height))
    }

    pub(crate) fn require_direct_target(
        prepared: &PreparedPicture,
        target: &wgpu::Texture,
        prepared_format: wgpu::TextureFormat,
    ) -> Result<(), Self> {
        let extent = target.size();
        Self::require_target(
            Size::new(extent.width, extent.height),
            target,
            prepared_format,
        )?;
        Self::require_aspect(
            prepared.aspect(),
            extent.width as f32 / extent.height as f32,
        )
    }

    fn require_aspect(prepared: f32, target: f32) -> Result<(), Self> {
        if prepared != target {
            return Err(Self::TargetAspect { prepared, target });
        }
        Ok(())
    }
}

impl fmt::Display for MapBindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPreparedFrame { map } => write!(
                f,
                "ONE X2 map raster is frame {} at {:.6} s, but no picture is prepared",
                map.index(),
                map.timestamp().as_secs_f64(),
            ),
            Self::PreparedMismatch { map, prepared } => write!(
                f,
                "ONE X2 map raster was made for another prepared picture: map frame {} at {:.6} s, current frame {} at {:.6} s",
                map.index(),
                map.timestamp().as_secs_f64(),
                prepared.index(),
                prepared.timestamp().as_secs_f64(),
            ),
            Self::ExtentMismatch { map, target } => write!(
                f,
                "ONE X2 map raster is {} by {}, but the origin-zero draw target is {} by {}",
                map.width, map.height, target.width, target.height,
            ),
            Self::TargetShape {
                dimension,
                depth_or_array_layers,
                mip_levels,
                samples,
            } => write!(
                f,
                "ONE X2 map target must be one single-sampled 2D mip and layer, but it is {dimension:?} with {depth_or_array_layers} layers, {mip_levels} mips and {samples} samples"
            ),
            Self::TargetFormat { prepared, target } => write!(
                f,
                "ONE X2 map target format is {target:?}, but the prepared pipeline format is {prepared:?}"
            ),
            Self::TargetUsage { target } => write!(
                f,
                "ONE X2 map target usage is {target:?}, which lacks RENDER_ATTACHMENT"
            ),
            Self::TargetAspect { prepared, target } => write!(
                f,
                "ONE X2 map target aspect is {target:.6}, but the prepared picture aspect is {prepared:.6}"
            ),
            Self::Uncovered { count } => write!(
                f,
                "ONE X2 map raster left {count} output pixel centres uncovered"
            ),
        }
    }
}

impl std::error::Error for MapBindError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_2_resources_require_exact_shapes() {
        assert_eq!(
            PackedMap::new(vec![[0.0; 4]; MAP_NODES - 1]),
            Err(MapShapeError::Packed {
                count: MAP_NODES - 1
            })
        );
        assert_eq!(
            PackedMap::new(vec![[0.0; 4]; MAP_NODES + 1]),
            Err(MapShapeError::Packed {
                count: MAP_NODES + 1
            })
        );
        assert_eq!(
            AlphaMap::new(vec![0.0; MAP_NODES - 1]),
            Err(MapShapeError::Alpha {
                count: MAP_NODES - 1
            })
        );
        assert_eq!(
            AlphaMap::new(vec![0.0; MAP_NODES + 1]),
            Err(MapShapeError::Alpha {
                count: MAP_NODES + 1
            })
        );
        assert_eq!(
            PackedMap::new(vec![[0.0; 4]; MAP_NODES])
                .unwrap()
                .bytes()
                .len(),
            PACKED_BYTES
        );
        assert_eq!(
            AlphaMap::new(vec![0.0; MAP_NODES]).unwrap().bytes().len(),
            ALPHA_BYTES
        );
    }

    #[test]
    fn resource_wrappers_preserve_float_bits() {
        let mut packed = vec![[0.0; 4]; MAP_NODES];
        packed[0] = [
            f32::from_bits(0x7fc0_1234),
            -0.0,
            f32::INFINITY,
            f32::from_bits(1),
        ];
        let mut alpha = vec![0.0; MAP_NODES];
        alpha[MAP_NODES - 1] = f32::from_bits(0x7fc0_5678);

        let packed = PackedMap::new(packed).unwrap();
        let alpha = AlphaMap::new(alpha).unwrap();
        assert_eq!(
            packed.nodes()[0].map(f32::to_bits),
            [0x7fc0_1234, 0x8000_0000, 0x7f80_0000, 1]
        );
        assert_eq!(alpha.nodes()[MAP_NODES - 1].to_bits(), 0x7fc0_5678);
    }

    #[test]
    fn preparation_identity_survives_cloning_and_cannot_alias_while_held() {
        let held = Preparation::new();
        assert_eq!(held, held.clone());
        for _ in 0..10_000 {
            assert_ne!(held, Preparation::new());
        }
    }

    #[test]
    fn map_draw_extent_must_match_both_axes_exactly() {
        let map = Size::new(2160, 2160);
        assert_eq!(MapBindError::require_extent(map, map), Ok(()));
        assert_eq!(
            MapBindError::require_extent(map, Size::new(1080, 2160)),
            Err(MapBindError::ExtentMismatch {
                map,
                target: Size::new(1080, 2160),
            })
        );
        assert_eq!(
            MapBindError::require_extent(map, Size::new(2160, 1080)),
            Err(MapBindError::ExtentMismatch {
                map,
                target: Size::new(2160, 1080),
            })
        );
    }

    #[test]
    fn direct_map_target_must_keep_the_prepared_aspect() {
        assert_eq!(MapBindError::require_aspect(16.0 / 9.0, 16.0 / 9.0), Ok(()));
        assert_eq!(
            MapBindError::require_aspect(1.0, 16.0 / 9.0),
            Err(MapBindError::TargetAspect {
                prepared: 1.0,
                target: 16.0 / 9.0,
            })
        );
    }
}
