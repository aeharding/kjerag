//! Capture-owned selected ONE X2 frame transaction.
//!
//! One owner binds source identity, parent geometry, estimator history and the
//! final type-2 payload. It accepts frame zero first and then every adjacent
//! delivery from the same decode epoch. There is no implicit reset: a seek,
//! duplicate, gap or reordered delivery is an error, so retained warm state
//! can never cross a discontinuity unnoticed.

use std::error::Error;
use std::fmt;

use kjerag_media::{FrameStamp, Size};
use kjerag_meta::{CalibrationSet, Filter, OrientationTrack, Readout};

use crate::flow::one_xs_belt::{
    BaseMapShapeError, CameraMaskError, CameraMaskReport, CameraMaskSupport, RetainedBaseMaps,
    sample_source_belts,
};
use crate::studio_type2::{OneXsMapFrame, PackedMap};
use crate::{Camera, Held, OneXsLumaFrame, Reframe, Sampling};

use super::base_map::{FilterError, MergeError, filter_fisheye_line_pair, map_merge};
use super::map_patch::{self, BaseMap, BilateralInputs, Census, FlowMap, PreimageMap, SideInputs};
use super::owner::{Continuity, PairOwner, PairPosition, Phase};
use super::resources::{OneXsResources, ResourceError};
use super::scalar::{ColdInputs, WorkRowCounts};
use super::{COLS, InvalidNodeCounts, LensPair, ParentMapBuilder, ParentMapError, ROWS};

/// The complete result for one exact delivered source pair.
pub(crate) struct FrameResult {
    pub map: OneXsMapFrame,
    pub phase: Phase,
    pub camera_mask: CameraMaskReport,
    pub invalid_nodes: InvalidNodeCounts,
    pub weighted_rows: WorkRowCounts,
    pub lens_a_census: Census,
    pub lens_b_census: Census,
}

enum State {
    NeedFrameZero,
    Running {
        previous: FrameStamp,
        estimator: Box<PairOwner>,
    },
}

/// All capture-static and retained per-frame state for one uninterrupted run.
pub(crate) struct FrameOwner {
    parent: ParentMapBuilder,
    resources: OneXsResources,
    orientation: OrientationTrack,
    readout: Readout,
    camera_mask_support: CameraMaskSupport,
    source_size: Size,
    continuity: Continuity,
    state: State,
}

impl FrameOwner {
    /// Build a fresh cold owner from one capture's factory calibration.
    pub fn new(calibration: &CalibrationSet) -> Result<Self, FrameOwnerError> {
        let source_size = Size {
            width: calibration.dimension.width,
            height: calibration.dimension.height,
        };
        let parent = ParentMapBuilder::new(calibration).map_err(FrameOwnerError::Parent)?;
        let resources =
            OneXsResources::new(&calibration.lenses).map_err(FrameOwnerError::Resource)?;
        let camera_mask_reframe = Reframe::new(
            &calibration.lenses,
            source_size,
            Camera::default(),
            Held::default(),
            1.0,
            false,
            Sampling::default(),
        );
        let camera_mask_support = CameraMaskSupport::from_reframe(&camera_mask_reframe)
            .map_err(FrameOwnerError::CameraMask)?;
        Ok(Self {
            parent,
            resources,
            orientation: calibration.orientation(Filter::default()),
            readout: calibration.readout(),
            camera_mask_support,
            source_size,
            continuity: Continuity::new(),
            state: State::NeedFrameZero,
        })
    }

    /// Consume one exact decoded luma pair and produce its exact-bound map.
    ///
    /// Every operation which can fail runs before the retained estimator is
    /// consumed. After that point only exact-shape internal constructors and
    /// the already-validated adjacent `PairOwner` transition remain. Thus an
    /// error leaves the owner at the previously committed frame.
    pub fn process(&mut self, frame: &OneXsLumaFrame) -> Result<FrameResult, FrameOwnerError> {
        self.validate_delivery(frame.frame())?;
        if frame.size() != self.source_size {
            return Err(FrameOwnerError::SourceSize {
                expected: self.source_size,
                actual: frame.size(),
            });
        }

        let parents = self
            .parent
            .build_for_frame(&self.orientation, frame.frame(), self.readout)
            .map_err(FrameOwnerError::Parent)?;
        let prefiltered = LensPair {
            a: map_merge(&self.resources.static_coordinates().a, &parents.a)
                .map_err(|error| FrameOwnerError::Merge { lens: 'A', error })?,
            b: map_merge(&self.resources.static_coordinates().b, &parents.b)
                .map_err(|error| FrameOwnerError::Merge { lens: 'B', error })?,
        };
        let filtered = filter_fisheye_line_pair(prefiltered).map_err(FrameOwnerError::Filter)?;

        // The same post-filter bytes own both consumers. Splitting before the
        // filter would let source sampling and final patching disagree.
        let base_values = LensPair {
            a: filtered.a.row_major_values().to_vec(),
            b: filtered.b.row_major_values().to_vec(),
        };
        let retained =
            RetainedBaseMaps::from_lenses(base_values.clone()).map_err(FrameOwnerError::BaseMap)?;
        let patch_base = LensPair {
            a: BaseMap::new(base_values.a)
                .expect("filtered ONE X2 lens A map has the retained shape"),
            b: BaseMap::new(base_values.b)
                .expect("filtered ONE X2 lens B map has the retained shape"),
        };
        let preimage = LensPair {
            a: PreimageMap::new(parents.a.row_major_values().to_vec())
                .expect("selected parent lens A ROI has the type-2 shape"),
            b: PreimageMap::new(parents.b.row_major_values().to_vec())
                .expect("selected parent lens B ROI has the type-2 shape"),
        };

        let staging = sample_source_belts(frame.sources(), &retained);
        let (input, camera_mask) = ColdInputs::from_staging_and_camera_mask_support(
            &staging,
            &retained,
            &self.camera_mask_support,
        );

        // All fallible source, geometry and mask work is complete. Consume the
        // retained estimator only now, then finish through fixed-size values.
        let position = PairPosition::new(&self.continuity, frame.frame().index());
        let previous_state = std::mem::replace(&mut self.state, State::NeedFrameZero);
        let step = match previous_state {
            State::NeedFrameZero => PairOwner::start(position, input),
            State::Running {
                previous,
                estimator,
            } => match (*estimator).advance(position, input) {
                Ok(step) => step,
                Err(rejected) => {
                    let reason = rejected.reason;
                    self.state = State::Running {
                        previous,
                        estimator: Box::new(rejected.owner),
                    };
                    return Err(FrameOwnerError::EstimatorContinuity(reason));
                }
            },
        };

        let planes = step.output.displacement.planes();
        let nodes = ROWS * COLS;
        let flow = LensPair {
            // Displacement is already cross-owned: planes 0/1 are B-to-A for
            // lens A, and planes 2/3 are A-to-B for lens B.
            a: FlowMap::new(planar_flow(&planes[..2 * nodes], nodes))
                .expect("lens A displacement has the retained shape"),
            b: FlowMap::new(planar_flow(&planes[2 * nodes..], nodes))
                .expect("lens B displacement has the retained shape"),
        };
        let maps = map_patch::materialize(BilateralInputs {
            b_to_a: SideInputs {
                preimage: &preimage.a,
                base: &patch_base.a,
                flow: &flow.a,
                gate: &self.resources.gates().a,
                coordinate: &self.resources.coordinates().a,
            },
            a_to_b: SideInputs {
                preimage: &preimage.b,
                base: &patch_base.b,
                flow: &flow.b,
                gate: &self.resources.gates().b,
                coordinate: &self.resources.coordinates().b,
            },
        });
        let packed =
            PackedMap::new(maps.packed).expect("bilateral materializer has the fixed type-2 shape");
        let result = FrameResult {
            map: OneXsMapFrame::new(
                frame.frame().clone(),
                packed,
                self.resources.alpha().clone(),
            ),
            phase: step.output.phase,
            camera_mask,
            invalid_nodes: step.output.invalid_nodes,
            weighted_rows: step.output.weighted_rows,
            lens_a_census: maps.lens_a_census,
            lens_b_census: maps.lens_b_census,
        };
        self.state = State::Running {
            previous: frame.frame().clone(),
            estimator: Box::new(step.owner),
        };
        Ok(result)
    }

    fn validate_delivery(&self, offered: &FrameStamp) -> Result<(), FrameOwnerError> {
        match &self.state {
            State::NeedFrameZero => validate_position(None, offered.index(), true),
            State::Running { previous, .. } => validate_position(
                Some(previous.index()),
                offered.index(),
                previous.same_decode_epoch(offered),
            ),
        }
        .map_err(FrameOwnerError::Sequence)
    }
}

fn planar_flow(planes: &[f32], nodes: usize) -> Vec<[f32; 2]> {
    debug_assert_eq!(planes.len(), 2 * nodes);
    (0..nodes)
        .map(|index| [planes[index], planes[nodes + index]])
        .collect()
}

fn validate_position(
    previous: Option<u64>,
    offered: u64,
    same_decode_epoch: bool,
) -> Result<(), SequenceError> {
    let Some(previous) = previous else {
        return if offered == 0 {
            Ok(())
        } else {
            Err(SequenceError::FirstFrame { offered })
        };
    };
    let Some(expected) = previous.checked_add(1) else {
        return Err(SequenceError::Exhausted { previous });
    };
    if !same_decode_epoch {
        return Err(SequenceError::DecodeEpochChanged);
    }
    if offered == expected {
        return Ok(());
    }
    if offered == previous {
        return Err(SequenceError::Duplicate { index: offered });
    }
    if offered < expected {
        return Err(SequenceError::Backward { previous, offered });
    }
    Err(SequenceError::Gap { expected, offered })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SequenceError {
    FirstFrame { offered: u64 },
    DecodeEpochChanged,
    Duplicate { index: u64 },
    Backward { previous: u64, offered: u64 },
    Gap { expected: u64, offered: u64 },
    Exhausted { previous: u64 },
}

impl fmt::Display for SequenceError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FirstFrame { offered } => write!(
                out,
                "ONE X2 stitching must start at frame 0; first offered frame is {offered}"
            ),
            Self::DecodeEpochChanged => {
                out.write_str("ONE X2 stitching cannot continue after a seek or decoder restart")
            }
            Self::Duplicate { index } => {
                write!(out, "ONE X2 stitching repeats frame {index}")
            }
            Self::Backward { previous, offered } => write!(
                out,
                "ONE X2 stitching moved backward from frame {previous} to frame {offered}"
            ),
            Self::Gap { expected, offered } => write!(
                out,
                "ONE X2 stitching skipped frame {expected}; next offered frame is {offered}"
            ),
            Self::Exhausted { previous } => {
                write!(out, "ONE X2 stitching cannot advance past frame {previous}")
            }
        }
    }
}

impl Error for SequenceError {}

#[derive(Debug)]
pub(crate) enum FrameOwnerError {
    Resource(ResourceError),
    Parent(ParentMapError),
    Merge { lens: char, error: MergeError },
    Filter(FilterError),
    BaseMap(BaseMapShapeError),
    CameraMask(CameraMaskError),
    Sequence(SequenceError),
    EstimatorContinuity(super::owner::ContinuityError),
    SourceSize { expected: Size, actual: Size },
}

impl fmt::Display for FrameOwnerError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resource(error) => error.fmt(out),
            Self::Parent(error) => error.fmt(out),
            Self::Merge { lens, error } => write!(out, "ONE X2 lens {lens} {error}"),
            Self::Filter(error) => error.fmt(out),
            Self::BaseMap(error) => error.fmt(out),
            Self::CameraMask(error) => error.fmt(out),
            Self::Sequence(error) => error.fmt(out),
            Self::EstimatorContinuity(error) => error.fmt(out),
            Self::SourceSize { expected, actual } => write!(
                out,
                "ONE X2 source frame is {}x{} but calibration requires {}x{}",
                actual.width, actual.height, expected.width, expected.height
            ),
        }
    }
}

impl Error for FrameOwnerError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_delivery_must_be_frame_zero() {
        assert_eq!(validate_position(None, 0, true), Ok(()));
        assert_eq!(
            validate_position(None, 1, true),
            Err(SequenceError::FirstFrame { offered: 1 })
        );
    }

    #[test]
    fn running_delivery_requires_same_epoch_and_exact_adjacency() {
        assert_eq!(validate_position(Some(41), 42, true), Ok(()));
        assert_eq!(
            validate_position(Some(41), 42, false),
            Err(SequenceError::DecodeEpochChanged)
        );
        assert_eq!(
            validate_position(Some(41), 41, true),
            Err(SequenceError::Duplicate { index: 41 })
        );
        assert_eq!(
            validate_position(Some(41), 19, true),
            Err(SequenceError::Backward {
                previous: 41,
                offered: 19
            })
        );
        assert_eq!(
            validate_position(Some(41), 44, true),
            Err(SequenceError::Gap {
                expected: 42,
                offered: 44
            })
        );
    }

    #[test]
    fn exhausted_sequence_refuses_every_successor_without_overflow() {
        assert_eq!(
            validate_position(Some(u64::MAX), u64::MAX, true),
            Err(SequenceError::Exhausted { previous: u64::MAX })
        );
    }

    #[test]
    fn planar_displacement_preserves_component_order() {
        assert_eq!(
            planar_flow(&[10.0, 11.0, 12.0, 20.0, 21.0, 22.0], 3),
            vec![[10.0, 20.0], [11.0, 21.0], [12.0, 22.0]]
        );
    }
}
