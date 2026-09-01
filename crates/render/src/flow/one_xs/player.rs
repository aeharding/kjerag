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
};
use crate::studio_type2::{OneXsMapFrame, PackedMap};
use crate::{Camera, Held, Reframe, Sampling};

#[cfg(test)]
use crate::OneXsLumaFrame;
#[cfg(test)]
use crate::flow::one_xs_belt::{SolverBelts, sample_source_belts};

use super::base_map::{FilterError, MergeError, filter_fisheye_line_pair, map_merge};
use super::map_patch::{self, BaseMap, BilateralInputs, Census, FlowMap, PreimageMap, SideInputs};
use super::owner::{Continuity, PairOwner, PairPosition, Phase};
use super::resources::{OneXsResources, ResourceError};
use super::scalar::{ColdInputs, WorkRowCounts};
use super::temporal::BlurredBelts;
#[cfg(test)]
use super::temporal::gaussian_blur;
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

/// Fallible per-frame geometry, bound to one exact decoded delivery.
///
/// This value is deliberately linear: it cannot be cloned, and committing it
/// consumes it. The retained numeric owner is untouched until [`FrameOwner::commit`].
pub(crate) struct PreparedFrame {
    frame: FrameStamp,
    retained: RetainedBaseMaps,
    patch_base: LensPair<BaseMap>,
    preimage: LensPair<PreimageMap>,
    masks: LensPair<Vec<u8>>,
    camera_mask: CameraMaskReport,
}

impl PreparedFrame {
    /// The exact delivery this preparation is allowed to commit.
    pub(crate) fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    /// The exact retained maps from which a CPU or GPU belt producer samples.
    pub(crate) fn retained_base_maps(&self) -> &RetainedBaseMaps {
        &self.retained
    }

    #[cfg(test)]
    fn sample_solver_belts(&self, frame: &OneXsLumaFrame) -> Result<SolverBelts, FrameOwnerError> {
        if frame.frame() != self.frame() {
            return Err(FrameOwnerError::PreparedSourceMismatch);
        }
        Ok(sample_source_belts(frame.sources(), self.retained_base_maps()).reduce_area_3x3())
    }

    #[cfg(test)]
    fn sample_blurred_belts(
        &self,
        frame: &OneXsLumaFrame,
    ) -> Result<BlurredBelts, FrameOwnerError> {
        Ok(gaussian_blur(&self.sample_solver_belts(frame)?))
    }
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

    /// Validate and prepare all fallible geometry without consuming history.
    pub(crate) fn prepare(
        &self,
        frame: &FrameStamp,
        source_size: Size,
    ) -> Result<PreparedFrame, FrameOwnerError> {
        self.validate_delivery(frame)?;
        if source_size != self.source_size {
            return Err(FrameOwnerError::SourceSize {
                expected: self.source_size,
                actual: source_size,
            });
        }

        let parents = self
            .parent
            .build_for_frame(&self.orientation, frame, self.readout)
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
        let (masks, camera_mask) = self.camera_mask_support.apply(&retained);

        Ok(PreparedFrame {
            frame: frame.clone(),
            retained,
            patch_base,
            preimage,
            masks,
            camera_mask,
        })
    }

    /// Consume one prepared delivery and its exact post-blur solver belts.
    ///
    /// The second delivery check rejects a stale prepared transaction before
    /// retained history is touched. Once history is taken, the existing
    /// fixed-shape transition and materializer cannot fail; state is committed
    /// at the same final point as the original monolithic transaction.
    pub(crate) fn commit(
        &mut self,
        prepared: PreparedFrame,
        blurred_belts: BlurredBelts,
    ) -> Result<FrameResult, FrameOwnerError> {
        self.validate_delivery(&prepared.frame)?;
        let PreparedFrame {
            frame,
            patch_base,
            preimage,
            masks,
            camera_mask,
            ..
        } = prepared;

        let input = ColdInputs::from_blurred_belts_and_masks(blurred_belts, masks);

        // All fallible source, geometry and mask work is complete. Consume the
        // retained estimator only now, then finish through fixed-size values.
        let position = PairPosition::new(&self.continuity, frame.index());
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
            map: OneXsMapFrame::new(frame.clone(), packed, self.resources.alpha().clone()),
            phase: step.output.phase,
            camera_mask,
            invalid_nodes: step.output.invalid_nodes,
            weighted_rows: step.output.weighted_rows,
            lens_a_census: maps.lens_a_census,
            lens_b_census: maps.lens_b_census,
        };
        self.state = State::Running {
            previous: frame,
            estimator: Box::new(step.owner),
        };
        Ok(result)
    }

    /// Process one exact decoded luma pair through the synchronous CPU route.
    #[cfg(test)]
    pub fn process(&mut self, frame: &OneXsLumaFrame) -> Result<FrameResult, FrameOwnerError> {
        let prepared = self.prepare(frame.frame(), frame.size())?;
        let blurred_belts = prepared.sample_blurred_belts(frame)?;
        self.commit(prepared, blurred_belts)
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
    Merge {
        lens: char,
        error: MergeError,
    },
    Filter(FilterError),
    BaseMap(BaseMapShapeError),
    CameraMask(CameraMaskError),
    Sequence(SequenceError),
    EstimatorContinuity(super::owner::ContinuityError),
    #[cfg(test)]
    PreparedSourceMismatch,
    SourceSize {
        expected: Size,
        actual: Size,
    },
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
            #[cfg(test)]
            Self::PreparedSourceMismatch => {
                out.write_str("ONE X2 prepared geometry and source belts name different frames")
            }
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
    use std::time::Duration;

    use kjerag_meta::{
        CalibrationSet, ExposureTrack, GyroConfig, GyroEncoding, GyroTrack, OrientationSample,
        OrientationTrack, Quat,
    };

    use super::*;
    use crate::flow::one_xs::Lens;
    use crate::flow::one_xs_belt::SourceImage;
    use crate::projection::tests::{ONE_XS_FRAME, one_xs_lenses};

    fn calibration() -> CalibrationSet {
        CalibrationSet {
            camera_model: "Insta360 ONE X2".to_owned(),
            firmware: "synthetic".to_owned(),
            dimension: kjerag_meta::Size {
                width: ONE_XS_FRAME.width,
                height: ONE_XS_FRAME.height,
            },
            lenses: one_xs_lenses(),
            rolling_shutter_ms: 23.516_071_319_580_078,
            gyro: GyroConfig {
                encoding: GyroEncoding::Scaled,
                imu_orientation: "Zxy",
                first_frame_timestamp: 0,
                gyro_timestamp: None,
            },
            exposure: [ExposureTrack::default(), ExposureTrack::default()],
            imu: GyroTrack::default(),
            fused: OrientationTrack::from_samples(
                (1_900_000..=2_100_000)
                    .step_by(2_000)
                    .map(|offset_us| OrientationSample {
                        offset_us,
                        world_from_body: Quat::IDENTITY,
                    })
                    .collect(),
            ),
            calibration_canvas: kjerag_meta::Size {
                width: 6_080,
                height: 3_040,
            },
        }
    }

    fn frame(index: u64, previous: Option<&FrameStamp>, code: u8) -> OneXsLumaFrame {
        let stamp = FrameStamp::for_test(index, Duration::from_secs(2), previous);
        let source = |lens_code: u8| {
            let pixels = (0..32 * 32)
                .map(|index| {
                    lens_code
                        .wrapping_add(((index / 32) as u8).wrapping_mul(31))
                        .wrapping_add(((index % 32) as u8).wrapping_mul(17))
                })
                .collect();
            SourceImage::from_compact(32, 32, pixels).unwrap()
        };
        OneXsLumaFrame::for_test(
            stamp,
            Size {
                width: ONE_XS_FRAME.width,
                height: ONE_XS_FRAME.height,
            },
            LensPair {
                a: source(code),
                b: source(code.wrapping_add(83)),
            },
        )
    }

    fn assert_distinct_blur_stages(raw: &SolverBelts, once: &BlurredBelts) {
        assert_ne!(raw.lens(Lens::A), once.lens(Lens::A));
        let once_as_solver = SolverBelts::from_lenses(LensPair {
            a: once.lens(Lens::A).to_vec(),
            b: once.lens(Lens::B).to_vec(),
        })
        .unwrap();
        let twice = gaussian_blur(&once_as_solver);
        assert_ne!(once.lens(Lens::A), twice.lens(Lens::A));
    }

    fn assert_same_result(left: &FrameResult, right: &FrameResult) {
        assert_eq!(left.map.frame(), right.map.frame());
        assert_eq!(left.map.packed().bytes(), right.map.packed().bytes());
        assert_eq!(left.map.alpha().bytes(), right.map.alpha().bytes());
        assert_eq!(left.phase, right.phase);
        assert_eq!(left.camera_mask, right.camera_mask);
        assert_eq!(left.invalid_nodes, right.invalid_nodes);
        assert_eq!(left.weighted_rows, right.weighted_rows);
        assert_eq!(left.lens_a_census, right.lens_a_census);
        assert_eq!(left.lens_b_census, right.lens_b_census);
    }

    #[test]
    fn explicit_prepare_commit_matches_synchronous_process_and_next_state() {
        let calibration = calibration();
        let first = frame(0, None, 113);
        let second = frame(1, Some(first.frame()), 117);
        let mut synchronous = FrameOwner::new(&calibration).unwrap();
        let mut split = FrameOwner::new(&calibration).unwrap();

        let synchronous_first = synchronous.process(&first).unwrap();
        let prepared = split.prepare(first.frame(), first.size()).unwrap();
        let raw = prepared.sample_solver_belts(&first).unwrap();
        let belts = gaussian_blur(&raw);
        assert_distinct_blur_stages(&raw, &belts);
        let split_first = split.commit(prepared, belts).unwrap();
        assert_same_result(&synchronous_first, &split_first);

        let synchronous_second = synchronous.process(&second).unwrap();
        let prepared = split.prepare(second.frame(), second.size()).unwrap();
        let raw = prepared.sample_solver_belts(&second).unwrap();
        let belts = gaussian_blur(&raw);
        assert_distinct_blur_stages(&raw, &belts);
        let split_second = split.commit(prepared, belts).unwrap();
        assert_same_result(&synchronous_second, &split_second);
    }

    #[test]
    fn stale_preparation_fails_without_consuming_the_owner() {
        let calibration = calibration();
        let first = frame(0, None, 113);
        let second = frame(1, Some(first.frame()), 117);
        let mut owner = FrameOwner::new(&calibration).unwrap();
        let accepted = owner.prepare(first.frame(), first.size()).unwrap();
        let stale = owner.prepare(first.frame(), first.size()).unwrap();
        let belts = gaussian_blur(&SolverBelts::from_fn(|lens, row, col| {
            (lens.index() as u8)
                .wrapping_add(row as u8)
                .wrapping_add(col as u8)
        }));

        owner.commit(accepted, belts.clone()).unwrap();
        let error = match owner.commit(stale, belts.clone()) {
            Ok(_) => panic!("stale prepared transaction was accepted"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            FrameOwnerError::Sequence(SequenceError::Duplicate { index: 0 })
        ));

        let prepared = owner.prepare(second.frame(), second.size()).unwrap();
        let result = owner.commit(prepared, belts).unwrap();
        assert_eq!(result.phase, Phase::Warm);
        assert_eq!(result.map.frame(), second.frame());
    }

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
