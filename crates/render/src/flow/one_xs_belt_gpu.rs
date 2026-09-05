//! GPU sampling and reduction for selected ONE X2 solver inputs.
//!
//! This is the GPU-shaped equivalent of [`crate::flow::one_xs_belt::sample_source_belts`]
//! followed by [`SourceBelts::reduce_area_3x3`](crate::flow::one_xs_belt::SourceBelts::reduce_area_3x3)
//! and Studio's selected 5-by-5 input Gaussian. Production playback carries
//! its compact post-blur result into the resident estimator. Horizontal
//! Q7 sums retain one u32 per logical byte; the vertical pass rounds, packs
//! four final U8 codes into one word and preserves the 129,600-byte A-then-B
//! boundary. No staging allocation or full CPU luma readback lies between the
//! imported R8 textures and those inputs.

use std::sync::{Arc, Mutex, mpsc};
use std::{error::Error, fmt};

use super::gpu_context::OneXsGpuContext;
use super::parent::ParentMapBuilder;
use super::pis::gpu::GpuPisFlight;
use super::resources::OneXsResources;
use super::temporal::{BlurredBelts, gaussian_blur};
#[cfg(test)]
use super::{COLS, ROWS};
use super::{Lens, LensPair};
use crate::Fallible;
use crate::direct_type2::DirectType2Pipeline;
use crate::direct_type2::ImportedOneXsPicture;
use crate::draw_retirement::{DrawPermit, DrawRetirementError, IcedDrawRetirements};
use crate::flow::one_xs_belt::{RetainedBaseMaps, SolverBelts, SourceImage, sample_source_belts};
use kjerag_media::{FrameStamp, Frames};
use kjerag_meta::{CalibrationSet, OrientationTrack, Readout};

/// Resident PIS preparation is nested under the belt owner so its only
/// boundary can consume the whole private producer token atomically.
#[path = "one_xs/pis_frontend_gpu.rs"]
#[allow(dead_code)]
pub(super) mod pis_frontend_gpu;

/// Parent arithmetic is private to the same pre-submission owner as geometry.
#[path = "one_xs/parent_gpu.rs"]
#[allow(dead_code)]
mod parent_gpu;

/// Device-resident final bilateral-map materializer.
///
/// Its input boundary stays inside this private resident owner. Scene consumes
/// the result only through the capture facade's installed draw.
#[path = "one_xs/map_patch_gpu.rs"]
mod map_patch_gpu;

/// Capture-root reservation, successor and ready-install ownership.
///
/// The types stay private to this resident owner. Scene can publish only
/// through the facade, and the motion child receives only a linear reservation.
#[path = "one_xs/resident_frame_gpu.rs"]
#[allow(dead_code)]
mod resident_frame_gpu;

/// Geometry is nested under the belt owner so its only production boundary
/// can append belt work and mint the one lease atomically.
#[path = "one_xs/geometry_gpu.rs"]
#[allow(dead_code)]
mod geometry_gpu;

/// The only production admission point for a resident frame front half.
/// Reservation happens before parent preparation, allocation or encoding.
#[allow(dead_code)]
struct GpuResidentFramePipeline {
    parent: parent_gpu::GpuParentMapPipeline,
}

#[allow(dead_code)]
impl GpuResidentFramePipeline {
    fn new(context: OneXsGpuContext) -> Fallible<Self> {
        Ok(Self {
            parent: parent_gpu::GpuParentMapPipeline::new(context)?,
        })
    }

    fn begin_parent(
        &self,
        capture: &resident_frame_gpu::GpuResidentCapture,
        frame: FrameStamp,
        builder: &ParentMapBuilder,
        orientation: &OrientationTrack,
        readout: Readout,
    ) -> Fallible<parent_gpu::EncodedGpuParentMaps> {
        let reservation = capture.reserve(frame)?;
        self.parent
            .encode(builder, orientation, reservation, readout)
    }
}

/// Concrete sealed imported-source capability used by the resident front
/// half. The trait exposes identity checks and one consuming callback, never a
/// texture, plane, bind group, frame owner or wgpu handle.
#[allow(dead_code)]
pub(crate) trait ImportedOneXsSource: Sized {
    fn ensure_resident_context(&self, context: &OneXsGpuContext) -> Fallible<()>;
    fn ensure_resident_session(&self, session: &ResidentSourceIdentity) -> Fallible<()>;
    fn resident_frame(&self) -> FrameStamp;
    fn submit_with(self, binder: ResidentSourceBinder<'_>) -> Fallible<ResidentImportedFront>;
}

/// Opaque allocation identity minted once for one resident capture session.
/// It has no public constructor or value representation.
#[derive(Clone)]
pub(crate) struct ResidentSourceIdentity(Arc<()>);

impl ResidentSourceIdentity {
    fn new() -> Self {
        Self(Arc::new(()))
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self::new()
    }

    fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn ensure_matches(&self, expected: &Self) -> Fallible<()> {
        if self.matches(expected) {
            Ok(())
        } else {
            Err("ONE X2 imported source belongs to a different resident capture session".into())
        }
    }
}

/// Capture-private parent, geometry, belt and final-map producers.
///
/// This value can only be constructed inside [`ResidentSourceCapture`] from
/// one calibration and its one orientation track. Keeping it private prevents
/// a same-device capture root from being paired with another capture's static
/// resources or parent inputs.
#[allow(dead_code)]
struct ResidentSourceFrontPipeline {
    context: OneXsGpuContext,
    identity: ResidentSourceIdentity,
    root: resident_frame_gpu::GpuResidentCapture,
    parent_inputs: ParentMapBuilder,
    orientation: OrientationTrack,
    parent: GpuResidentFramePipeline,
    geometry: geometry_gpu::GpuGeometryPipeline,
    belts: GpuSolverBeltPipeline,
    final_map: map_patch_gpu::GpuMapMaterializer,
}

#[allow(dead_code)]
impl ResidentSourceFrontPipeline {
    fn new(
        context: OneXsGpuContext,
        calibration: &CalibrationSet,
        orientation: OrientationTrack,
    ) -> Fallible<Self> {
        let parent_inputs = ParentMapBuilder::new(calibration)?;
        let resources = OneXsResources::new(&calibration.lenses)?;
        let support = crate::flow::one_xs_belt::CameraMaskSupport::from_reframe(
            &crate::projection::Reframe::new(
                &calibration.lenses,
                kjerag_media::Size {
                    width: calibration.dimension.width,
                    height: calibration.dimension.height,
                },
                crate::Camera::default(),
                crate::Held::default(),
                1.0,
                false,
                crate::Sampling::default(),
            ),
        )?;
        let identity = ResidentSourceIdentity::new();
        Ok(Self {
            context: context.clone(),
            identity: identity.clone(),
            root: resident_frame_gpu::GpuResidentCapture::new_bound(context.clone(), identity),
            parent_inputs,
            orientation,
            parent: GpuResidentFramePipeline::new(context.clone())?,
            geometry: geometry_gpu::GpuGeometryPipeline::new(
                context.clone(),
                resources.static_coordinates(),
                &support,
            )?,
            belts: GpuSolverBeltPipeline::new(context.clone())?,
            final_map: map_patch_gpu::GpuMapMaterializer::new(context, &resources)?,
        })
    }

    fn begin_parent(&self, frame: FrameStamp) -> Fallible<parent_gpu::EncodedGpuParentMaps> {
        self.parent.begin_parent(
            &self.root,
            frame,
            &self.parent_inputs,
            &self.orientation,
            self.parent_inputs.readout(),
        )
    }

    /// Identity refusal precedes reservation, allocation and encoding. The
    /// exact frame comes from the sealed imported owner rather than a caller.
    fn submit_source<S: ImportedOneXsSource>(&self, source: S) -> Fallible<ResidentImportedFront> {
        source.ensure_resident_session(&self.identity)?;
        source.ensure_resident_context(&self.context)?;
        let frame = source.resident_frame();
        let parent = self.begin_parent(frame)?;
        let geometry = self.geometry.encode_resident_parents(parent)?;
        source.submit_with(ResidentSourceBinder {
            geometry,
            belts: &self.belts,
        })
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn materialize_completed_cold(
        &self,
        checkpoint: pis_frontend_gpu::GpuCompletedColdCheckpoint<ImportedOneXsPicture>,
    ) -> Fallible<
        map_patch_gpu::PendingGpuPackedMapFrame<
            pis_frontend_gpu::GpuFinalOperands<
                geometry_gpu::temporal_gpu::GpuColdPriorPublicLevelTwo,
            >,
        >,
    > {
        let operands = pis_frontend_gpu::admit_completed_cold_final(checkpoint, &self.context)?;
        self.final_map.materialize_final(operands)
    }
}

/// One open capture's inseparable calibration, orientation and resident root.
///
/// Submission names only this owner. There is no independently selectable
/// pipeline, parent builder, readout, orientation or static resource argument,
/// so two sessions sharing a GPU context cannot be cross-composed.
#[allow(dead_code)]
pub(crate) struct ResidentSourceCapture {
    pipeline: ResidentSourceFrontPipeline,
}

#[allow(dead_code)]
impl ResidentSourceCapture {
    pub(crate) fn new(
        context: OneXsGpuContext,
        calibration: &CalibrationSet,
        orientation: OrientationTrack,
    ) -> Fallible<Self> {
        Ok(Self {
            pipeline: ResidentSourceFrontPipeline::new(context, calibration, orientation)?,
        })
    }

    pub(crate) fn import_context(&self) -> &OneXsGpuContext {
        &self.pipeline.context
    }

    pub(crate) fn source_identity(&self) -> ResidentSourceIdentity {
        self.pipeline.identity.clone()
    }

    pub(crate) fn submit_imported(
        &self,
        source: ImportedOneXsPicture,
    ) -> Fallible<ResidentImportedFront> {
        self.pipeline.submit_source(source)
    }
}

/// Retryable states are values, not terminal engine failures. A caller keeps
/// the last installed picture and asks again on a later redraw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ResidentRetry {
    InFlight,
    DrawRetirementFull,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) enum ResidentSubmit {
    Submitted,
    AlreadyInstalled(FrameStamp),
    Retry(ResidentRetry),
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) enum ResidentPrepare {
    Empty,
    Pending {
        installed: Option<FrameStamp>,
    },
    Staged {
        installed: FrameStamp,
    },
    Retry {
        reason: ResidentRetry,
        installed: Option<FrameStamp>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResidentDrain {
    Pending,
    Drained,
    FailClosedRetained,
}

#[allow(dead_code)]
type ColdPending = map_patch_gpu::PendingGpuPackedMapFrame<
    pis_frontend_gpu::GpuFinalOperands<geometry_gpu::temporal_gpu::GpuColdPriorPublicLevelTwo>,
>;
#[allow(dead_code)]
type WarmPending = map_patch_gpu::PendingGpuPackedMapFrame<
    pis_frontend_gpu::GpuFinalOperands<geometry_gpu::temporal_gpu::GpuWarmPriorPublicLevelTwo>,
>;
#[allow(dead_code)]
type ColdReady = map_patch_gpu::GpuPackedMapFrame<
    pis_frontend_gpu::GpuFinalOperands<geometry_gpu::temporal_gpu::GpuColdPriorPublicLevelTwo>,
>;
#[allow(dead_code)]
type WarmReady = map_patch_gpu::GpuPackedMapFrame<
    pis_frontend_gpu::GpuFinalOperands<geometry_gpu::temporal_gpu::GpuWarmPriorPublicLevelTwo>,
>;

#[allow(dead_code)]
enum ResidentPendingMap {
    Cold(Box<ColdPending>),
    Warm(Box<WarmPending>),
}

#[allow(dead_code)]
enum ResidentReadyMap {
    Cold(Box<ColdReady>),
    Warm(Box<WarmReady>),
}

#[allow(dead_code)]
enum ResidentTransaction {
    Idle,
    Starting,
    Pending(ResidentPendingMap),
    Ready(ResidentReadyMap),
    Quarantined,
}

enum ResidentPoll {
    Continue(Box<ResidentTransaction>),
    Refused(Box<dyn Error + Send + Sync>),
    Quarantined(Box<dyn Error + Send + Sync>),
}

impl ResidentTransaction {
    fn quarantine_uncertain(self) {
        match self {
            Self::Pending(ResidentPendingMap::Cold(pending)) => (*pending).quarantine_uncertain(),
            Self::Pending(ResidentPendingMap::Warm(pending)) => (*pending).quarantine_uncertain(),
            // Ready has already acknowledged mapped completion. Every other
            // state has no uncertain submission carrier.
            other => drop(other),
        }
    }
}

struct ResidentStartGuard {
    inner: Arc<ResidentCaptureFacadeInner>,
    quarantine: bool,
    armed: bool,
}

impl ResidentStartGuard {
    fn rollback(inner: Arc<ResidentCaptureFacadeInner>) -> Self {
        Self {
            inner,
            quarantine: false,
            armed: true,
        }
    }

    fn quarantine(inner: Arc<ResidentCaptureFacadeInner>) -> Self {
        Self {
            inner,
            quarantine: true,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ResidentStartGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(mut state) = self.inner.state.lock()
            && matches!(state.transaction, ResidentTransaction::Starting)
        {
            state.transaction = if self.quarantine {
                ResidentTransaction::Quarantined
            } else {
                ResidentTransaction::Idle
            };
        }
    }
}

fn validate_resident_sequence(
    previous: Option<&FrameStamp>,
    offered: &FrameStamp,
) -> Result<(), super::player::SequenceError> {
    super::player::validate_position(
        previous.map(FrameStamp::index),
        offered.index(),
        previous.is_none_or(|previous| previous.same_decode_epoch(offered)),
    )
}

struct ResidentCaptureState {
    session: Option<Arc<ResidentCaptureSession>>,
    transaction: ResidentTransaction,
    installed: Option<FrameStamp>,
}

/// Capture-owned execution and draw resources. The picture layout, sampler,
/// direct pipeline and retirement queue are intentionally not ScenePipeline
/// fields. An installed source/map pair therefore keeps the exact association
/// that created it when iced recreates its renderer pipeline.
struct ResidentCaptureSession {
    context: OneXsGpuContext,
    format: wgpu::TextureFormat,
    source_size: kjerag_meta::Size,
    capture: ResidentSourceCapture,
    motion: geometry_gpu::temporal_gpu::GpuMotionStage,
    front: pis_frontend_gpu::GpuPisFrontEnd,
    solver: super::pis::gpu::GpuPisPipeline,
    bridge: pis_frontend_gpu::GpuL2PostPisBridge,
    picture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    direct: Arc<DirectType2Pipeline>,
    retirements: Arc<IcedDrawRetirements<InstalledOneXsPass>>,
    #[cfg(test)]
    cold_blurred_probe: Mutex<Option<TestColdBlurredProbe>>,
}

#[cfg(test)]
struct TestColdBlurredProbe {
    frame: FrameStamp,
    packed: wgpu::Buffer,
    physical_masks: wgpu::Buffer,
    shared_masks: Option<wgpu::Buffer>,
    cold0_l2_terminal: Option<wgpu::Buffer>,
    cold0_l1_initial: Option<wgpu::Buffer>,
    cold0_l1_initial_plane_stride_words: Option<u64>,
    l1_terminals: Vec<wgpu::Buffer>,
}

impl ResidentCaptureSession {
    /// Lazily construct one capture session. The existing resident stage
    /// constructors synchronously qualify arithmetic on this target device;
    /// those one-time diagnostic readbacks may wait. Once this returns,
    /// per-frame submit and redraw never use that initialization path.
    fn new(
        context: OneXsGpuContext,
        format: wgpu::TextureFormat,
        calibration: &CalibrationSet,
        orientation: OrientationTrack,
    ) -> Fallible<Self> {
        let picture_layout = crate::scene::bind_group_layout(context.device());
        let sampler = context.device().create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let direct = Arc::new(DirectType2Pipeline::new(
            context.device(),
            &picture_layout,
            format,
        ));
        Ok(Self {
            capture: ResidentSourceCapture::new(context.clone(), calibration, orientation)?,
            motion: geometry_gpu::temporal_gpu::GpuMotionStage::new(context.clone())?,
            front: pis_frontend_gpu::GpuPisFrontEnd::new(context.clone())?,
            solver: super::pis::gpu::GpuPisPipeline::new(context.clone())?,
            bridge: pis_frontend_gpu::GpuL2PostPisBridge::new(context.clone())
                .map_err(|error| error.to_string())?,
            picture_layout,
            sampler,
            direct,
            retirements: Arc::new(IcedDrawRetirements::new(
                context.device(),
                IcedInstalledDrawAdapter::RETIREMENT_CAPACITY,
            )),
            #[cfg(test)]
            cold_blurred_probe: Mutex::new(None),
            context,
            format,
            source_size: calibration.dimension,
        })
    }

    fn ensure_renderer(
        &self,
        context: &OneXsGpuContext,
        format: wgpu::TextureFormat,
    ) -> Fallible<()> {
        self.context.ensure_same(context)?;
        if self.format != format {
            return Err(format!(
                "ONE X2 resident capture render format changed from {:?} to {:?}",
                self.format, format
            )
            .into());
        }
        Ok(())
    }

    fn controls(&self) -> pis_frontend_gpu::GpuColdLoopControls {
        use super::pis::Level;
        use super::{Direction, selected_pis_interval};
        pis_frontend_gpu::GpuColdLoopControls::new(
            pis_frontend_gpu::GpuL2Controls::resident(
                selected_pis_interval(Direction::AtoB, Level::Two),
                selected_pis_interval(Direction::BtoA, Level::Two),
            ),
            pis_frontend_gpu::GpuL1Controls::resident(
                selected_pis_interval(Direction::AtoB, Level::One),
                selected_pis_interval(Direction::BtoA, Level::One),
            ),
        )
    }

    fn submit(
        &self,
        frames: Arc<Frames>,
        reframe: &crate::Reframe,
    ) -> Fallible<ResidentPendingMap> {
        let warm = self.capture.pipeline.root.has_installed_successor()?;
        #[cfg(test)]
        let frame = frames.stamp();
        let source =
            self.capture
                .import_picture(&self.picture_layout, &self.sampler, reframe, frames)?;
        let source = source.submit_resident_front(&self.capture)?;
        #[cfg(test)]
        if !warm && std::env::var_os("KJERAG_ONE_X2_COLD_STAGE_PROBE").is_some() {
            let mut probe = self
                .cold_blurred_probe
                .lock()
                .map_err(|_| "ONE X2 cold blurred stage probe is poisoned")?;
            if probe.is_some() {
                return Err("ONE X2 cold blurred stage probe was recorded more than once".into());
            }
            *probe = Some(TestColdBlurredProbe {
                frame,
                packed: source.blurred_buffer_for_test(),
                physical_masks: source.physical_masks_for_test(),
                shared_masks: None,
                cold0_l2_terminal: None,
                cold0_l1_initial: None,
                cold0_l1_initial_plane_stride_words: None,
                l1_terminals: Vec::with_capacity(3),
            });
        }
        let motion = source.prepare_motion(&self.motion)?;
        let controls = self.controls();
        if warm {
            let terminal = motion.submit_resident_warm(
                &self.front,
                &self.solver,
                &self.bridge,
                controls.l2(),
                controls.l1(),
            )?;
            let operands = terminal.complete_warm_final(&self.bridge)?;
            Ok(ResidentPendingMap::Warm(Box::new(
                self.capture
                    .pipeline
                    .final_map
                    .materialize_final(operands)?,
            )))
        } else {
            #[cfg(test)]
            let probing = std::env::var_os("KJERAG_ONE_X2_COLD_STAGE_PROBE").is_some();
            let cold0 = motion.submit_resident_cold0(
                &self.front,
                &self.solver,
                &self.bridge,
                geometry_gpu::temporal_gpu::GpuColdPriorPublicLevelTwo::new(self.context.clone()),
                controls,
            )?;
            #[cfg(test)]
            if probing {
                let mut probe = self
                    .cold_blurred_probe
                    .lock()
                    .map_err(|_| "ONE X2 cold stage probe is poisoned")?;
                probe
                    .as_mut()
                    .ok_or("ONE X2 cold stage probe lost its geometry owner")?
                    .shared_masks = Some(cold0.shared_masks_for_test());
            }
            #[cfg(test)]
            let cold = if probing {
                let (cold0_l2_terminal, cold0_l1_initial, cold0_l1_initial_plane_stride_words) =
                    cold0.cold0_input_buffers_for_test();
                let cold0_terminal = cold0.l1_terminal_buffer_for_test();
                let cold_after0 = cold0.complete(&self.bridge)?;
                let (cold_after1, cold1_terminal) =
                    cold_after0.resume_with_l1_probe_for_test(&self.bridge, &self.solver)?;
                let (cold, cold2_terminal) =
                    cold_after1.resume_with_l1_probe_for_test(&self.bridge, &self.solver)?;
                let mut probe = self
                    .cold_blurred_probe
                    .lock()
                    .map_err(|_| "ONE X2 cold stage probe is poisoned")?;
                let probe = probe
                    .as_mut()
                    .ok_or("ONE X2 cold stage probe lost its blurred owner")?;
                probe.cold0_l2_terminal = Some(cold0_l2_terminal);
                probe.cold0_l1_initial = Some(cold0_l1_initial);
                probe.cold0_l1_initial_plane_stride_words =
                    Some(cold0_l1_initial_plane_stride_words);
                probe.l1_terminals = vec![cold0_terminal, cold1_terminal, cold2_terminal];
                cold
            } else {
                cold0
                    .complete(&self.bridge)?
                    .resume(&self.bridge, &self.solver)?
                    .resume(&self.bridge, &self.solver)?
            };
            #[cfg(not(test))]
            let cold = cold0
                .complete(&self.bridge)?
                .resume(&self.bridge, &self.solver)?
                .resume(&self.bridge, &self.solver)?;
            Ok(ResidentPendingMap::Cold(Box::new(
                self.capture.pipeline.materialize_completed_cold(cold)?,
            )))
        }
    }
}

#[allow(dead_code)]
struct ResidentCaptureFacadeInner {
    calibration: Arc<CalibrationSet>,
    orientation: OrientationTrack,
    state: Mutex<ResidentCaptureState>,
}

/// One open capture's resident transaction owner. Clones are renderer
/// attachments to the same root, pending validity word and retirement queue;
/// they do not clone numeric history or a decoder surface owner.
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct ResidentCaptureFacade {
    inner: Arc<ResidentCaptureFacadeInner>,
}

#[allow(dead_code)]
impl ResidentCaptureFacade {
    pub(crate) fn new(calibration: Arc<CalibrationSet>, orientation: OrientationTrack) -> Self {
        Self {
            inner: Arc::new(ResidentCaptureFacadeInner {
                calibration,
                orientation,
                state: Mutex::new(ResidentCaptureState {
                    session: None,
                    transaction: ResidentTransaction::Idle,
                    installed: None,
                }),
            }),
        }
    }

    /// Attach one renderer generation without making renderer lifetime the
    /// lifetime of resident history or retirement proof.
    pub(crate) fn attach_renderer(
        &self,
        context: OneXsGpuContext,
        format: wgpu::TextureFormat,
    ) -> Fallible<ResidentSceneFacade> {
        let session = self.bind_session(context, format)?;
        Ok(ResidentSceneFacade {
            capture: self.clone(),
            draw: IcedInstalledDrawAdapter::with_shared_retirements(Arc::clone(
                &session.retirements,
            )),
        })
    }

    fn bind_session(
        &self,
        context: OneXsGpuContext,
        format: wgpu::TextureFormat,
    ) -> Fallible<Arc<ResidentCaptureSession>> {
        let mut state = self.state()?;
        if let Some(session) = &state.session {
            session.ensure_renderer(&context, format)?;
            return Ok(Arc::clone(session));
        }
        let session = Arc::new(ResidentCaptureSession::new(
            context,
            format,
            &self.inner.calibration,
            self.inner.orientation.clone(),
        )?);
        state.session = Some(Arc::clone(&session));
        Ok(session)
    }

    fn state(&self) -> Fallible<std::sync::MutexGuard<'_, ResidentCaptureState>> {
        self.inner
            .state
            .lock()
            .map_err(|_| "ONE X2 resident transaction facade is poisoned".into())
    }

    /// Exact capture-side acknowledgement for replay/pump code that has no
    /// renderer attachment. Readable indices alone never authorize reuse.
    pub(crate) fn acknowledged(&self, frame: &FrameStamp) -> Fallible<bool> {
        Ok(self.state()?.installed.as_ref() == Some(frame))
    }

    pub(crate) fn same_capture(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub(crate) fn installed_stamp(&self) -> Fallible<Option<FrameStamp>> {
        Ok(self.state()?.installed.clone())
    }

    /// Explicit instrument-only bulk readback of the exact installed resident
    /// result. Ordinary prepare and draw never call this path.
    pub(crate) fn diagnostic_installed_map(
        &self,
        frame: &FrameStamp,
    ) -> Fallible<Option<crate::OneXsMapFrame>> {
        let (installed, session) = {
            let state = self.state()?;
            (state.installed.clone(), state.session.clone())
        };
        if installed.as_ref() != Some(frame) {
            return Ok(None);
        }
        let Some(session) = session else {
            return Ok(None);
        };
        let Some(draw) = session.capture.pipeline.root.diagnostic_ready()? else {
            return Ok(None);
        };
        if draw.frame() != *frame {
            return Err("ONE X2 diagnostic ready differs from the installed frame".into());
        }
        Ok(Some(draw.map.diagnostic_readback()?))
    }

    /// Test-only snapshot of the exact production post-Gaussian belt buffer
    /// retained at cold frame zero. The buffer is cloned while the ordinary
    /// resident ownership chain is intact and copied only after installation.
    #[cfg(test)]
    pub(crate) fn diagnostic_cold_blurred_probe(
        &self,
        frame: &FrameStamp,
    ) -> Fallible<Option<BlurredBelts>> {
        let Some(session) = self.state()?.session.clone() else {
            return Ok(None);
        };
        let probe = session
            .cold_blurred_probe
            .lock()
            .map_err(|_| "ONE X2 cold blurred stage probe is poisoned")?;
        let Some(probe) = probe.as_ref() else {
            return Ok(None);
        };
        if &probe.frame != frame {
            return Err("ONE X2 cold blurred stage probe names a different frame".into());
        }
        read_blurred_probe(&session.context, &probe.packed).map(Some)
    }

    #[cfg(test)]
    pub(crate) fn diagnostic_cold_masks(
        &self,
        frame: &FrameStamp,
    ) -> Fallible<Option<(Vec<u8>, Vec<u32>)>> {
        let Some(session) = self.state()?.session.clone() else {
            return Ok(None);
        };
        let probe = session
            .cold_blurred_probe
            .lock()
            .map_err(|_| "ONE X2 cold mask probe is poisoned")?;
        let Some(probe) = probe.as_ref() else {
            return Ok(None);
        };
        if &probe.frame != frame {
            return Err("ONE X2 cold mask probe names a different frame".into());
        }
        let packed = read_word_probe(&session.context, &probe.physical_masks)?;
        let mut physical = Vec::with_capacity(2 * ROWS * COLS);
        for word in packed.into_iter().take(2 * ROWS * COLS / 4) {
            physical.extend(word.to_ne_bytes());
        }
        let shared = probe
            .shared_masks
            .as_ref()
            .ok_or("ONE X2 cold mask probe lost its shared masks")?;
        Ok(Some((physical, read_word_probe(&session.context, shared)?)))
    }

    #[cfg(test)]
    pub(crate) fn diagnostic_cold_l1_terminals(
        &self,
        frame: &FrameStamp,
    ) -> Fallible<Option<Vec<Vec<u32>>>> {
        let Some(session) = self.state()?.session.clone() else {
            return Ok(None);
        };
        let probe = session
            .cold_blurred_probe
            .lock()
            .map_err(|_| "ONE X2 cold stage probe is poisoned")?;
        let Some(probe) = probe.as_ref() else {
            return Ok(None);
        };
        if &probe.frame != frame {
            return Err("ONE X2 cold L1 terminal probe names a different frame".into());
        }
        if probe.l1_terminals.len() != 3 {
            return Err("ONE X2 cold L1 terminal probe did not retain three calls".into());
        }
        probe
            .l1_terminals
            .iter()
            .map(|buffer| read_word_probe(&session.context, buffer))
            .collect::<Fallible<Vec<_>>>()
            .map(Some)
    }

    #[cfg(test)]
    pub(crate) fn diagnostic_cold0_pis_inputs(
        &self,
        frame: &FrameStamp,
    ) -> Fallible<Option<(Vec<u32>, Vec<u32>)>> {
        let Some(session) = self.state()?.session.clone() else {
            return Ok(None);
        };
        let probe = session
            .cold_blurred_probe
            .lock()
            .map_err(|_| "ONE X2 cold stage probe is poisoned")?;
        let Some(probe) = probe.as_ref() else {
            return Ok(None);
        };
        if &probe.frame != frame {
            return Err("ONE X2 Cold0 PIS probe names a different frame".into());
        }
        let l2 = probe
            .cold0_l2_terminal
            .as_ref()
            .ok_or("ONE X2 Cold0 probe lost its L2 terminal")?;
        let initial = probe
            .cold0_l1_initial
            .as_ref()
            .ok_or("ONE X2 Cold0 probe lost its L1 initial")?;
        let initial_stride = probe
            .cold0_l1_initial_plane_stride_words
            .ok_or("ONE X2 Cold0 probe lost its L1 initial plane stride")?;
        let initial_words = read_word_probe(&session.context, initial)?;
        Ok(Some((
            read_word_probe(&session.context, l2)?,
            compact_l1_initial_planes(&initial_words, initial_stride)?,
        )))
    }

    #[cfg(test)]
    pub(crate) fn diagnostic_cold_final_inputs(
        &self,
        frame: &FrameStamp,
    ) -> Fallible<Option<map_patch_gpu::DiagnosticFinalInputs>> {
        let (installed, session) = {
            let state = self.state()?;
            (state.installed.clone(), state.session.clone())
        };
        if installed.as_ref() != Some(frame) {
            return Ok(None);
        }
        let Some(session) = session else {
            return Ok(None);
        };
        let Some(draw) = session.capture.pipeline.root.diagnostic_ready()? else {
            return Ok(None);
        };
        if draw.frame() != *frame {
            return Err("ONE X2 diagnostic final inputs differ from the installed frame".into());
        }
        draw.map.diagnostic_final_inputs().map(Some)
    }
}

impl std::fmt::Debug for ResidentCaptureFacade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResidentCaptureFacade")
            .field("identity", &Arc::as_ptr(&self.inner))
            .finish_non_exhaustive()
    }
}

/// Pipeline-local one-shot staging backed by capture-shared transaction and
/// retirement owners. Dropping this value returns an undispatched permit;
/// already armed payloads remain in the capture's shared queue.
#[allow(dead_code)]
pub(crate) struct ResidentSceneFacade {
    capture: ResidentCaptureFacade,
    draw: IcedInstalledDrawAdapter,
}

#[allow(dead_code)]
impl ResidentSceneFacade {
    pub(crate) fn needs_poll(&self) -> bool {
        let state = match self.capture.state() {
            Ok(state) => state,
            Err(error) => {
                eprintln!("{error}");
                return false;
            }
        };
        if matches!(state.transaction, ResidentTransaction::Quarantined) {
            return false;
        }
        matches!(state.transaction, ResidentTransaction::Pending(_))
            || state
                .session
                .as_ref()
                .is_some_and(|session| !session.retirements.is_empty())
    }

    pub(crate) fn quarantine_after_external_poll_failure(&self) {
        self.draw.staged().take();
        if let Ok(mut state) = self.capture.state() {
            let transaction =
                std::mem::replace(&mut state.transaction, ResidentTransaction::Quarantined);
            transaction.quarantine_uncertain();
            if let Some(session) = &state.session {
                session.retirements.quarantine_after_external_poll_failure();
            }
        }
    }

    pub(crate) fn submit_frame(
        &self,
        context: &OneXsGpuContext,
        format: wgpu::TextureFormat,
        frames: Arc<Frames>,
        reframe: &crate::Reframe,
    ) -> Fallible<ResidentSubmit> {
        let session = self.capture.bind_session(context.clone(), format)?;
        let stamp = frames.stamp();
        {
            let mut state = self.capture.state()?;
            if state.installed.as_ref() == Some(&stamp) {
                return Ok(ResidentSubmit::AlreadyInstalled(stamp));
            }
            if matches!(state.transaction, ResidentTransaction::Quarantined) {
                return Err("ONE X2 resident transaction facade is quarantined".into());
            }
            if !matches!(state.transaction, ResidentTransaction::Idle) {
                return Ok(ResidentSubmit::Retry(ResidentRetry::InFlight));
            }
            if (frames.size.width, frames.size.height)
                != (session.source_size.width, session.source_size.height)
            {
                return Err(format!(
                    "ONE X2 source frame is {}x{} but calibration requires {}x{}",
                    frames.size.width,
                    frames.size.height,
                    session.source_size.width,
                    session.source_size.height
                )
                .into());
            }
            validate_resident_sequence(state.installed.as_ref(), &stamp)?;
            state.transaction = ResidentTransaction::Starting;
        }
        let mut start = ResidentStartGuard::quarantine(Arc::clone(&self.capture.inner));
        let pending = session.submit(frames, reframe);
        let mut state = self.capture.state()?;
        match pending {
            Ok(pending) => {
                state.transaction = ResidentTransaction::Pending(pending);
                start.disarm();
                Ok(ResidentSubmit::Submitted)
            }
            Err(error) => {
                state.transaction = ResidentTransaction::Quarantined;
                start.disarm();
                Err(error)
            }
        }
    }

    /// Drive the capture exactly once for this redraw, collect every callback
    /// made visible by that poll, then stage either the newly installed frame
    /// or the old exact ready frame. No wait or polling loop exists here.
    pub(crate) fn prepare_redraw(
        &self,
        context: &OneXsGpuContext,
        format: wgpu::TextureFormat,
        reframe_for: impl Fn(&FrameStamp) -> Fallible<crate::Reframe>,
    ) -> Fallible<ResidentPrepare> {
        self.prepare_redraw_inner(context, format, false, reframe_for)
    }

    pub(crate) fn prepare_redraw_after_external_poll(
        &self,
        context: &OneXsGpuContext,
        format: wgpu::TextureFormat,
        reframe_for: impl Fn(&FrameStamp) -> Fallible<crate::Reframe>,
    ) -> Fallible<ResidentPrepare> {
        self.prepare_redraw_inner(context, format, true, reframe_for)
    }

    fn prepare_redraw_inner(
        &self,
        context: &OneXsGpuContext,
        format: wgpu::TextureFormat,
        externally_polled: bool,
        reframe_for: impl Fn(&FrameStamp) -> Fallible<crate::Reframe>,
    ) -> Fallible<ResidentPrepare> {
        let session = self.capture.bind_session(context.clone(), format)?;
        self.draw.staged().take();
        let transaction = {
            let mut state = self.capture.state()?;
            std::mem::replace(&mut state.transaction, ResidentTransaction::Starting)
        };
        let mut prepare_start = ResidentStartGuard::rollback(Arc::clone(&self.capture.inner));
        let (transaction, externally_polled) = match transaction {
            ResidentTransaction::Pending(pending) => {
                if !externally_polled {
                    let polled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        session.context.device().poll(wgpu::PollType::Poll)
                    }));
                    let poll_error: Option<Box<dyn Error + Send + Sync>> = match polled {
                        Ok(Ok(_)) => None,
                        Ok(Err(error)) => Some(Box::new(error)),
                        Err(payload) => Some(
                            payload
                                .downcast_ref::<&str>()
                                .map(|message| (*message).to_owned())
                                .or_else(|| payload.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "ONE X2 GPU device poll panicked".to_owned())
                                .into(),
                        ),
                    };
                    if let Some(error) = poll_error {
                        session.retirements.quarantine_after_external_poll_failure();
                        self.capture.state()?.transaction = ResidentTransaction::Quarantined;
                        prepare_start.disarm();
                        ResidentTransaction::Pending(pending).quarantine_uncertain();
                        return Err(error);
                    }
                }
                let result = match pending {
                    ResidentPendingMap::Cold(pending) => {
                        match (*pending).finish_after_poll_classified() {
                            map_patch_gpu::ClassifiedValidityPoll::Pending(value) => {
                                ResidentPoll::Continue(Box::new(ResidentTransaction::Pending(
                                    ResidentPendingMap::Cold(Box::new(value)),
                                )))
                            }
                            map_patch_gpu::ClassifiedValidityPoll::Ready(value) => {
                                ResidentPoll::Continue(Box::new(ResidentTransaction::Ready(
                                    ResidentReadyMap::Cold(Box::new(value)),
                                )))
                            }
                            map_patch_gpu::ClassifiedValidityPoll::Refused(error) => {
                                ResidentPoll::Refused(error)
                            }
                            map_patch_gpu::ClassifiedValidityPoll::Quarantined(error) => {
                                ResidentPoll::Quarantined(error)
                            }
                        }
                    }
                    ResidentPendingMap::Warm(pending) => {
                        match (*pending).finish_after_poll_classified() {
                            map_patch_gpu::ClassifiedValidityPoll::Pending(value) => {
                                ResidentPoll::Continue(Box::new(ResidentTransaction::Pending(
                                    ResidentPendingMap::Warm(Box::new(value)),
                                )))
                            }
                            map_patch_gpu::ClassifiedValidityPoll::Ready(value) => {
                                ResidentPoll::Continue(Box::new(ResidentTransaction::Ready(
                                    ResidentReadyMap::Warm(Box::new(value)),
                                )))
                            }
                            map_patch_gpu::ClassifiedValidityPoll::Refused(error) => {
                                ResidentPoll::Refused(error)
                            }
                            map_patch_gpu::ClassifiedValidityPoll::Quarantined(error) => {
                                ResidentPoll::Quarantined(error)
                            }
                        }
                    }
                };
                match result {
                    ResidentPoll::Continue(transaction) => (*transaction, true),
                    ResidentPoll::Refused(error) => {
                        self.capture.state()?.transaction = ResidentTransaction::Idle;
                        prepare_start.disarm();
                        return Err(error);
                    }
                    ResidentPoll::Quarantined(error) => {
                        self.capture.state()?.transaction = ResidentTransaction::Quarantined;
                        prepare_start.disarm();
                        return Err(error);
                    }
                }
            }
            ResidentTransaction::Quarantined => {
                self.capture.state()?.transaction = ResidentTransaction::Quarantined;
                prepare_start.disarm();
                return Err("ONE X2 resident transaction facade is quarantined".into());
            }
            other => (other, externally_polled),
        };
        let retirement_result = if externally_polled {
            session.retirements.collect_after_external_poll()
        } else {
            session.retirements.poll()
        };
        if let Err(error) = retirement_result {
            transaction.quarantine_uncertain();
            self.capture.state()?.transaction = ResidentTransaction::Quarantined;
            prepare_start.disarm();
            return Err(error);
        }

        let mut state = self.capture.state()?;
        state.transaction = transaction;
        prepare_start.disarm();
        if matches!(state.transaction, ResidentTransaction::Ready(_)) {
            let permit = match session.retirements.reserve() {
                Ok(permit) => permit,
                Err(DrawRetirementError::Full) => {
                    return Ok(ResidentPrepare::Retry {
                        reason: ResidentRetry::DrawRetirementFull,
                        installed: state.installed.clone(),
                    });
                }
                Err(error) => return Err(error.into()),
            };
            let ready_map =
                match std::mem::replace(&mut state.transaction, ResidentTransaction::Starting) {
                    ResidentTransaction::Ready(ResidentReadyMap::Cold(map)) => {
                        ResidentReadyMap::Cold(map)
                    }
                    ResidentTransaction::Ready(ResidentReadyMap::Warm(map)) => {
                        ResidentReadyMap::Warm(map)
                    }
                    _ => unreachable!("checked resident ready transaction"),
                };
            drop(state);
            let mut install_start = ResidentStartGuard::quarantine(Arc::clone(&self.capture.inner));
            let install = match ready_map {
                ResidentReadyMap::Cold(map) => {
                    prepare_resident_install_with_permit(*map, Arc::clone(&session.direct), permit)
                }
                ResidentReadyMap::Warm(map) => {
                    prepare_resident_install_with_permit(*map, Arc::clone(&session.direct), permit)
                }
            };
            let install = match install {
                Ok(install) => install,
                Err(error) => {
                    let mut state = self.capture.state()?;
                    state.transaction = ResidentTransaction::Quarantined;
                    install_start.disarm();
                    return Err(error);
                }
            };
            let installed = install.frame();
            let reframe = match reframe_for(&installed) {
                Ok(reframe) => reframe,
                Err(error) => {
                    drop(install);
                    let mut state = self.capture.state()?;
                    state.transaction = ResidentTransaction::Idle;
                    install_start.disarm();
                    return Err(error);
                }
            };
            // Lock order is façade then root: screenshot takes the same order.
            // Keep the acknowledgement hidden until the exact ready has also
            // been staged, so no observer can see an undrawable publication.
            let mut state = self.capture.state()?;
            let ready = match install.install() {
                Ok(ready) => ready,
                Err(error) => {
                    state.transaction = ResidentTransaction::Quarantined;
                    install_start.disarm();
                    return Err(error);
                }
            };
            self.draw.prepare_installed(ready, &reframe);
            state.installed = Some(installed.clone());
            state.transaction = ResidentTransaction::Idle;
            install_start.disarm();
            drop(state);
            return Ok(ResidentPrepare::Staged { installed });
        }
        let pending = matches!(state.transaction, ResidentTransaction::Pending(_));
        let installed = state.installed.clone();
        let Some(installed_frame) = installed.as_ref() else {
            drop(state);
            return if pending {
                Ok(ResidentPrepare::Pending { installed })
            } else {
                Ok(ResidentPrepare::Empty)
            };
        };
        let ready = match session
            .capture
            .pipeline
            .root
            .ready_for_draw(&session.retirements)
        {
            Ok(ready) => ready,
            Err(DrawRetirementError::Full) => {
                drop(state);
                return Ok(ResidentPrepare::Retry {
                    reason: ResidentRetry::DrawRetirementFull,
                    installed,
                });
            }
            Err(error) => {
                drop(state);
                return Err(error.into());
            }
        };
        let Some(mut ready) = ready else {
            drop(state);
            return if pending {
                Ok(ResidentPrepare::Pending { installed })
            } else {
                Ok(ResidentPrepare::Empty)
            };
        };
        let ready_frame = ready.draw.frame();
        if &ready_frame != installed_frame {
            state.transaction = ResidentTransaction::Quarantined;
            return Err("ONE X2 redraw ready names a different installed frame".into());
        }
        drop(state);
        let reframe = reframe_for(&ready_frame)?;
        ready.write_reframe(&reframe);
        self.draw.staged().replace(ready);
        Ok(ResidentPrepare::Staged {
            installed: ready_frame,
        })
    }

    pub(crate) fn arm_and_draw(&self, pass: &mut wgpu::RenderPass<'_>) -> bool {
        self.draw.arm_and_draw(pass)
    }

    pub(crate) fn acknowledged(&self) -> Fallible<Option<FrameStamp>> {
        Ok(self.capture.state()?.installed.clone())
    }

    /// Nonblocking normal replacement drain. Completion-proven candidates are
    /// discarded without publication; uncertain submissions remain owned and
    /// are polled again on a later redraw.
    pub(crate) fn drain_replaced(&self) -> Fallible<ResidentDrain> {
        self.drain_replaced_inner(false)
    }

    pub(crate) fn drain_replaced_after_external_poll(&self) -> Fallible<ResidentDrain> {
        self.drain_replaced_inner(true)
    }

    fn drain_replaced_inner(&self, externally_polled: bool) -> Fallible<ResidentDrain> {
        self.draw.staged().take();
        let Some(session) = self.capture.state()?.session.clone() else {
            return Ok(ResidentDrain::Drained);
        };
        let transaction = {
            let mut state = self.capture.state()?;
            std::mem::replace(&mut state.transaction, ResidentTransaction::Starting)
        };
        if matches!(transaction, ResidentTransaction::Quarantined) {
            self.capture.state()?.transaction = ResidentTransaction::Quarantined;
            return Ok(ResidentDrain::FailClosedRetained);
        }
        let polled = (!externally_polled).then(|| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                session.context.device().poll(wgpu::PollType::Poll)
            }))
        });
        if let Some(Err(payload)) = polled.as_ref() {
            let message = payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "ONE X2 GPU device poll panicked".to_owned());
            session.retirements.quarantine_after_external_poll_failure();
            transaction.quarantine_uncertain();
            self.capture.state()?.transaction = ResidentTransaction::Quarantined;
            eprintln!("{message}");
            return Ok(ResidentDrain::FailClosedRetained);
        }
        if let Some(Ok(Err(error))) = polled {
            session.retirements.quarantine_after_external_poll_failure();
            transaction.quarantine_uncertain();
            self.capture.state()?.transaction = ResidentTransaction::Quarantined;
            eprintln!("{error}");
            return Ok(ResidentDrain::FailClosedRetained);
        }
        let transaction = match transaction {
            ResidentTransaction::Pending(pending) => match pending {
                ResidentPendingMap::Cold(pending) => classify_retired_cold(*pending)?,
                ResidentPendingMap::Warm(pending) => classify_retired_warm(*pending)?,
            },
            ResidentTransaction::Ready(ready) => {
                drop(ready);
                ResidentTransaction::Idle
            }
            other => other,
        };
        session.retirements.collect_after_external_poll()?;
        let fail_closed = matches!(transaction, ResidentTransaction::Quarantined);
        let idle =
            matches!(transaction, ResidentTransaction::Idle) && session.retirements.is_empty();
        self.capture.state()?.transaction = transaction;
        Ok(if fail_closed {
            ResidentDrain::FailClosedRetained
        } else if idle {
            ResidentDrain::Drained
        } else {
            ResidentDrain::Pending
        })
    }

    /// Reserve a separate render-pass proof for an offscreen screenshot. It
    /// cannot steal or reuse the one-shot window capability.
    pub(crate) fn prepare_screenshot(
        &self,
        context: &OneXsGpuContext,
        format: wgpu::TextureFormat,
        reframe_for: impl FnOnce(&FrameStamp) -> Fallible<crate::Reframe>,
    ) -> Fallible<ResidentScreenshotPrepare> {
        let session = self.capture.bind_session(context.clone(), format)?;
        // The façade acknowledgement and root ready snapshot are observed
        // under the same façade-then-root lock order as installation.
        let state = self.capture.state()?;
        let ready = match session
            .capture
            .pipeline
            .root
            .ready_for_draw(&session.retirements)
        {
            Ok(ready) => ready,
            Err(DrawRetirementError::Full) => return Ok(ResidentScreenshotPrepare::RetryFull),
            Err(error) => return Err(error.into()),
        };
        let Some(mut ready) = ready else {
            return Ok(ResidentScreenshotPrepare::Empty);
        };
        let frame = ready.draw.frame();
        let installed = state.installed.clone();
        if installed.as_ref() != Some(&frame) {
            return Err("ONE X2 screenshot ready names a different installed frame".into());
        }
        drop(state);
        let reframe = reframe_for(&frame)?;
        ready.write_reframe(&reframe);
        Ok(ResidentScreenshotPrepare::Ready(ResidentScreenshotDraw {
            ready: Some(ready),
            retirements: Arc::clone(&session.retirements),
        }))
    }
}

fn classify_retired_cold(pending: ColdPending) -> Fallible<ResidentTransaction> {
    Ok(match pending.finish_after_poll_classified() {
        map_patch_gpu::ClassifiedValidityPoll::Pending(value) => {
            ResidentTransaction::Pending(ResidentPendingMap::Cold(Box::new(value)))
        }
        map_patch_gpu::ClassifiedValidityPoll::Ready(value) => {
            drop(value);
            ResidentTransaction::Idle
        }
        map_patch_gpu::ClassifiedValidityPoll::Refused(error) => {
            eprintln!("{error}");
            ResidentTransaction::Idle
        }
        map_patch_gpu::ClassifiedValidityPoll::Quarantined(error) => {
            eprintln!("{error}");
            ResidentTransaction::Quarantined
        }
    })
}

fn classify_retired_warm(pending: WarmPending) -> Fallible<ResidentTransaction> {
    Ok(match pending.finish_after_poll_classified() {
        map_patch_gpu::ClassifiedValidityPoll::Pending(value) => {
            ResidentTransaction::Pending(ResidentPendingMap::Warm(Box::new(value)))
        }
        map_patch_gpu::ClassifiedValidityPoll::Ready(value) => {
            drop(value);
            ResidentTransaction::Idle
        }
        map_patch_gpu::ClassifiedValidityPoll::Refused(error) => {
            eprintln!("{error}");
            ResidentTransaction::Idle
        }
        map_patch_gpu::ClassifiedValidityPoll::Quarantined(error) => {
            eprintln!("{error}");
            ResidentTransaction::Quarantined
        }
    })
}

#[allow(dead_code)]
pub(crate) enum ResidentScreenshotPrepare {
    Empty,
    Ready(ResidentScreenshotDraw),
    RetryFull,
}

#[must_use = "the resident screenshot draw has not been armed"]
#[allow(dead_code)]
pub(crate) struct ResidentScreenshotDraw {
    ready: Option<InstalledOneXsReady>,
    retirements: Arc<IcedDrawRetirements<InstalledOneXsPass>>,
}

#[allow(dead_code)]
impl ResidentScreenshotDraw {
    pub(crate) fn arm_and_draw(mut self, pass: &mut wgpu::RenderPass<'_>) {
        self.ready
            .take()
            .expect("resident screenshot draw is linear")
            .arm_and_draw(&self.retirements, pass);
    }
}

/// One-shot callback handed only to the concrete imported-source owner after
/// identity checks and front-half encoding. Its fields are private, so no
/// caller can manufacture another owner/source association.
#[allow(dead_code)]
pub(crate) struct ResidentSourceBinder<'a> {
    geometry: geometry_gpu::EncodedGpuGeometry,
    belts: &'a GpuSolverBeltPipeline,
}

#[allow(dead_code)]
impl ResidentSourceBinder<'_> {
    pub(crate) fn submit_exact(
        self,
        sources: SourceTextures<'_>,
        owner: ImportedOneXsPicture,
    ) -> Fallible<ResidentImportedFront> {
        self.geometry
            .submit_belts(self.belts, sources, owner)
            .map(|inner| ResidentImportedFront { inner })
    }
}

/// Opaque front-half result. Its only continuation consumes the exact
/// geometry/source lease into resident motion; no component is returned.
#[must_use = "the imported ONE X2 resident front half has not been consumed"]
#[allow(dead_code)]
pub(crate) struct ResidentImportedFront {
    inner: geometry_gpu::GpuGeometryBelts<ImportedOneXsPicture>,
}

/// One nonconstructible installed source/map pair. Its sole rendering surface
/// binds and draws the exact imported picture through the exact final map.
/// Cloning is possible only as `Arc<InstalledOneXsDraw>`.
#[allow(dead_code)] // private prerequisite; Scene selection is intentionally excluded
pub(crate) struct InstalledOneXsDraw {
    source: ImportedOneXsPicture,
    map: map_patch_gpu::InstalledGpuMapBinding,
    pipeline: Arc<DirectType2Pipeline>,
    #[cfg(test)]
    drop_witness: Option<InstalledDrawDropWitness>,
}

#[cfg(test)]
struct InstalledDrawDropWitness(Arc<std::sync::atomic::AtomicU8>);

#[cfg(test)]
impl Drop for InstalledDrawDropWitness {
    fn drop(&mut self) {
        self.0.store(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[allow(dead_code)]
impl InstalledOneXsDraw {
    fn frame(&self) -> FrameStamp {
        self.source.resident_frame()
    }

    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.source.draw(&self.pipeline, self.map.read(), pass);
    }

    /// Allocate and bind one immutable Reframe for one exact render pass.
    fn prepare_pass(self: &Arc<Self>, reframe: &crate::Reframe) -> Arc<InstalledOneXsPass> {
        Arc::new(InstalledOneXsPass {
            binding: self
                .pipeline
                .prepare_resident_picture(&self.source, reframe),
            draw: Arc::clone(self),
        })
    }

    fn ensure_install_identity(
        &self,
        context: &OneXsGpuContext,
        session: &ResidentSourceIdentity,
        root: &resident_frame_gpu::GpuResidentIdentity,
        flight: &GpuPisFlight,
    ) -> Fallible<()> {
        self.source.ensure_resident_context(context)?;
        self.source.ensure_resident_session(session)?;
        self.source.ensure_resident_frame(&flight.frame)?;
        if self.map.frame() != &flight.frame {
            return Err("ONE X2 installed map names a different capture flight".into());
        }
        if !self.map.matches_root(root) {
            return Err("ONE X2 installed map belongs to a different capture root".into());
        }
        Ok(())
    }
}

/// One render-pass-private binding. Declaration order releases its bind group
/// and uniform before the complete installed source/map carrier.
#[allow(dead_code)]
struct InstalledOneXsPass {
    binding: crate::direct_type2::ImportedOneXsDrawBinding,
    draw: Arc<InstalledOneXsDraw>,
}

impl InstalledOneXsPass {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw.source.draw_resident_binding(
            &self.draw.pipeline,
            &self.binding,
            self.draw.map.read(),
            pass,
        );
    }
}

/// Carrier-first whole-frame install. Declaration order is load-bearing:
/// ordinary drop and unwind release the source/map carrier, then roll back the
/// root candidate, then return the unused draw permit.
#[must_use = "the resident installed draw must be atomically published or rolled back"]
#[allow(dead_code)]
pub(crate) struct ResidentInstallCandidate {
    draw: Option<Arc<InstalledOneXsDraw>>,
    root: Option<resident_frame_gpu::GpuResidentCandidate>,
    #[cfg(test)]
    panic_before_root_install: bool,
    permit: Option<DrawPermit>,
}

/// Bound source/map carrier and exact unpublished successor. Declaration
/// order makes retirement backpressure release the offered carrier before the
/// root candidate rolls its pending reservation back.
#[must_use = "the bound resident install must reserve retirement capacity"]
#[allow(dead_code)]
struct ResidentBoundInstall {
    draw: Option<Arc<InstalledOneXsDraw>>,
    root: Option<resident_frame_gpu::GpuResidentCandidate>,
}

#[allow(dead_code)]
impl ResidentBoundInstall {
    fn reserve(
        mut self,
        retirements: &IcedDrawRetirements<InstalledOneXsPass>,
    ) -> Result<ResidentInstallCandidate, DrawRetirementError> {
        let permit = retirements.reserve()?;
        Ok(ResidentInstallCandidate {
            draw: self.draw.take(),
            root: self.root.take(),
            #[cfg(test)]
            panic_before_root_install: false,
            permit: Some(permit),
        })
    }

    fn with_permit(mut self, permit: DrawPermit) -> ResidentInstallCandidate {
        ResidentInstallCandidate {
            draw: self.draw.take(),
            root: self.root.take(),
            #[cfg(test)]
            panic_before_root_install: false,
            permit: Some(permit),
        }
    }
}

#[allow(dead_code)]
pub(crate) struct InstalledOneXsReady {
    draw: Arc<InstalledOneXsDraw>,
    permit: DrawPermit,
    pass: Option<Arc<InstalledOneXsPass>>,
}

#[allow(dead_code)]
impl InstalledOneXsReady {
    pub(crate) fn write_reframe(&mut self, reframe: &crate::Reframe) {
        self.pass = Some(self.draw.prepare_pass(reframe));
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn arm_and_draw<'pass>(
        self,
        retirements: &IcedDrawRetirements<InstalledOneXsPass>,
        pass: &mut wgpu::RenderPass<'pass>,
    ) {
        let draw = self
            .pass
            .expect("resident draw must retain its exact private picture binding");
        retirements.arm_and_draw(self.permit, pass, draw, |draw, pass| draw.draw(pass));
    }

    #[cfg(test)]
    fn uniform_for_test(&self) -> wgpu::Buffer {
        self.pass
            .as_ref()
            .expect("resident test draw has an exact picture binding")
            .binding
            .uniform_for_test()
    }
}

/// Iced's bounded, one-redraw staging owner for an installed resident draw.
///
/// Preparation polls completed render submissions without waiting, then
/// seals a draw-private uniform and picture binding into one linear draw
/// capability. Drawing takes that capability and
/// attaches its retirement proof to iced's live render pass. A second prepare
/// before draw drops the undispatched permit before reserving another one.
/// It changes no capture history.
pub(crate) struct IcedInstalledDrawAdapter {
    retirements: Arc<IcedDrawRetirements<InstalledOneXsPass>>,
    staged: Mutex<Option<InstalledOneXsReady>>,
}

impl IcedInstalledDrawAdapter {
    /// The qualified adapter bounds in-flight decoder-backed pictures at two.
    const RETIREMENT_CAPACITY: usize = 2;

    #[allow(
        dead_code,
        reason = "explicit legacy installed-draw oracle constructor"
    )]
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        Self::with_shared_retirements(Arc::new(IcedDrawRetirements::new(
            device,
            Self::RETIREMENT_CAPACITY,
        )))
    }

    fn with_shared_retirements(retirements: Arc<IcedDrawRetirements<InstalledOneXsPass>>) -> Self {
        Self {
            retirements,
            staged: Mutex::new(None),
        }
    }

    /// Legacy adapter oracle: poll without making draw-time polling part of
    /// the selected contract.
    #[allow(dead_code, reason = "explicit legacy installed-draw oracle polling")]
    pub(crate) fn poll_prepare(&self) -> Fallible<usize> {
        match self.retirements.poll() {
            Ok(retired) => Ok(retired),
            Err(error) => {
                self.staged().take();
                Err(error)
            }
        }
    }

    /// Stage the capability returned by a successful atomic install.
    ///
    /// The permit was reserved before publication. This creates an immutable
    /// per-pass picture binding that cannot alias another output's Reframe.
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn prepare_installed(
        &self,
        mut ready: InstalledOneXsReady,
        reframe: &crate::Reframe,
    ) {
        self.staged().take();
        ready.write_reframe(reframe);
        self.staged().replace(ready);
    }

    /// Stage a fresh draw of the root's exact installed payload.
    ///
    /// Retirement exhaustion remains typed retryable backpressure. The
    /// staged cell is empty before admission, so refusal cannot draw an older
    /// capability or become a terminal playback failure.
    #[allow(dead_code, reason = "explicit legacy installed-draw oracle staging")]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn prepare_redraw(
        &self,
        root: &resident_frame_gpu::GpuResidentCapture,
        reframe: &crate::Reframe,
    ) -> Result<bool, DrawRetirementError> {
        self.staged().take();
        let Some(mut ready) = root.ready_for_draw(&self.retirements)? else {
            return Ok(false);
        };
        ready.write_reframe(reframe);
        self.staged().replace(ready);
        Ok(true)
    }

    /// Consume the one staged capability into iced's exact render pass.
    pub(crate) fn arm_and_draw(&self, pass: &mut wgpu::RenderPass<'_>) -> bool {
        let Some(ready) = self.staged().take() else {
            return false;
        };
        ready.arm_and_draw(&self.retirements, pass);
        true
    }

    #[allow(dead_code, reason = "explicit legacy installed-draw oracle inspection")]
    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn retirements(
        &self,
    ) -> &IcedDrawRetirements<InstalledOneXsPass> {
        &self.retirements
    }

    fn staged(&self) -> std::sync::MutexGuard<'_, Option<InstalledOneXsReady>> {
        self.staged
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    #[cfg(test)]
    fn staged_uniform_for_test(&self) -> wgpu::Buffer {
        self.staged()
            .as_ref()
            .expect("resident adapter has a staged draw")
            .uniform_for_test()
    }
}

#[allow(dead_code)]
impl ResidentInstallCandidate {
    fn frame(&self) -> FrameStamp {
        self.draw
            .as_ref()
            .expect("resident install lost its draw carrier")
            .frame()
    }

    pub(crate) fn install(mut self) -> Fallible<InstalledOneXsReady> {
        #[cfg(test)]
        if self.panic_before_root_install {
            panic!("injected resident install unwind");
        }
        self.root
            .as_mut()
            .expect("resident install lost its root candidate")
            .install(
                self.draw
                    .as_ref()
                    .expect("resident install lost its draw carrier"),
            )?;
        let draw = self
            .draw
            .take()
            .expect("resident install lost its draw carrier");
        drop(
            self.root
                .take()
                .expect("installed root candidate is disarmed"),
        );
        Ok(InstalledOneXsReady {
            draw,
            permit: self
                .permit
                .take()
                .expect("resident install lost its draw permit"),
            pass: None,
        })
    }

    #[cfg(test)]
    fn observe_draw_drop(&mut self, witness: Arc<std::sync::atomic::AtomicU8>) {
        Arc::get_mut(
            self.draw
                .as_mut()
                .expect("test install retains unique draw carrier"),
        )
        .expect("test install has not cloned its draw carrier")
        .drop_witness = Some(InstalledDrawDropWitness(Arc::clone(&witness)));
    }

    #[cfg(test)]
    fn observe_root_rollback(&mut self, witness: Arc<std::sync::atomic::AtomicU8>) {
        self.root
            .as_mut()
            .expect("test install retains root candidate")
            .observe_carrier_drop(witness);
    }

    #[cfg(test)]
    fn inject_stale_root(&mut self) {
        self.root
            .as_ref()
            .expect("test install retains root candidate")
            .clear_pending_for_stale_test();
    }

    #[cfg(test)]
    fn probe_install_refusal(&mut self) -> Fallible<()> {
        self.root
            .as_mut()
            .expect("test install retains root candidate")
            .install(
                self.draw
                    .as_ref()
                    .expect("test install retains draw carrier"),
            )
    }

    #[cfg(test)]
    fn inject_install_panic(&mut self) {
        self.panic_before_root_install = true;
    }
}

#[allow(dead_code)]
pub(in crate::flow::one_xs::one_xs_belt_gpu) fn prepare_resident_install<P>(
    map: map_patch_gpu::GpuPackedMapFrame<pis_frontend_gpu::GpuFinalOperands<P>>,
    pipeline: Arc<DirectType2Pipeline>,
    retirements: &IcedDrawRetirements<InstalledOneXsPass>,
) -> Fallible<ResidentInstallCandidate>
where
    P: geometry_gpu::temporal_gpu::GpuPriorPublicLevelTwo + Send + Sync + 'static,
{
    let context = map.install_context();
    pipeline.ensure_device(&context)?;
    let bound = map.bind_for_install(&context, pipeline.map_layout())?;
    Ok(ResidentBoundInstall {
        draw: Some(Arc::new(InstalledOneXsDraw {
            source: bound.source,
            map: bound.binding,
            pipeline,
            #[cfg(test)]
            drop_witness: None,
        })),
        root: Some(bound.candidate),
    }
    .reserve(retirements)?)
}

#[allow(dead_code)]
fn prepare_resident_install_with_permit<P>(
    map: map_patch_gpu::GpuPackedMapFrame<pis_frontend_gpu::GpuFinalOperands<P>>,
    pipeline: Arc<DirectType2Pipeline>,
    permit: DrawPermit,
) -> Fallible<ResidentInstallCandidate>
where
    P: geometry_gpu::temporal_gpu::GpuPriorPublicLevelTwo + Send + Sync + 'static,
{
    let context = map.install_context();
    pipeline.ensure_device(&context)?;
    let bound = map.bind_for_install(&context, pipeline.map_layout())?;
    Ok(ResidentBoundInstall {
        draw: Some(Arc::new(InstalledOneXsDraw {
            source: bound.source,
            map: bound.binding,
            pipeline,
            #[cfg(test)]
            drop_witness: None,
        })),
        root: Some(bound.candidate),
    }
    .with_permit(permit))
}

#[allow(dead_code)]
impl ResidentImportedFront {
    #[cfg(test)]
    fn blurred_buffer_for_test(&self) -> wgpu::Buffer {
        self.inner.belts.packed.clone()
    }

    #[cfg(test)]
    fn physical_masks_for_test(&self) -> wgpu::Buffer {
        self.inner.physical_masks_for_test()
    }

    pub(in crate::flow::one_xs::one_xs_belt_gpu) fn prepare_motion(
        self,
        stage: &geometry_gpu::temporal_gpu::GpuMotionStage,
    ) -> Fallible<
        geometry_gpu::temporal_gpu::GpuMotionTransaction<
            geometry_gpu::GpuGeometryBelts<ImportedOneXsPicture>,
        >,
    > {
        self.inner.prepare_motion(stage)
    }
}

#[cfg(test)]
fn read_blurred_probe(context: &OneXsGpuContext, packed: &wgpu::Buffer) -> Fallible<BlurredBelts> {
    let readback = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("ONE X2 cold blurred stage probe"),
        size: OUTPUT_BYTES,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context.device().create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(packed, 0, &readback, 0, OUTPUT_BYTES);
    let submission = context.queue().submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (mapped, answer) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = mapped.send(result);
    });
    context.device().poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    answer.recv()??;
    let bytes = slice.get_mapped_range();
    let belts = unpack_blurred_belts(&bytes)?;
    drop(bytes);
    readback.unmap();
    Ok(belts)
}

#[cfg(test)]
fn read_word_probe(context: &OneXsGpuContext, source: &wgpu::Buffer) -> Fallible<Vec<u32>> {
    let bytes = source.size();
    if !bytes.is_multiple_of(4) {
        return Err("ONE X2 cold stage word probe has a non-word buffer size".into());
    }
    let readback = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("ONE X2 cold stage word probe readback"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context.device().create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &readback, 0, bytes);
    let submission = context.queue().submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (sent, received) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sent.send(result);
    });
    context.device().poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    received.recv()??;
    let mapped = slice.get_mapped_range();
    let words = mapped
        .chunks_exact(4)
        .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
        .collect();
    drop(mapped);
    readback.unmap();
    Ok(words)
}

#[cfg(test)]
fn compact_l1_initial_planes(words: &[u32], plane_stride_words: u64) -> Fallible<Vec<u32>> {
    let stride = usize::try_from(plane_stride_words)?;
    let logical = super::pis::Level::One.patches();
    if stride < logical || words.len() != 4 * stride {
        return Err(format!(
            "ONE X2 Cold0 L1 initial allocation is {} words with plane stride {stride}, expected four planes of at least {logical} words",
            words.len()
        )
        .into());
    }
    Ok((0..4)
        .flat_map(|plane| {
            words[plane * stride..plane * stride + logical]
                .iter()
                .copied()
        })
        .collect())
}

const CODES_PER_WORD: usize = 4;
const OUTPUT_BYTES: u64 = SolverBelts::BYTES as u64;
const OUTPUT_WORDS: u32 = (SolverBelts::BYTES / CODES_PER_WORD) as u32;
const HORIZONTAL_BYTES: u64 = SolverBelts::BYTES as u64 * size_of::<u32>() as u64;
const WITNESS_BYTES: u64 = 2 * size_of::<u32>() as u64;
const WORKGROUP_SIZE: u32 = 64;
const _: () = assert!(SolverBelts::BYTES.is_multiple_of(CODES_PER_WORD));
const _: () = assert!(super::COLS.is_multiple_of(CODES_PER_WORD));
const _: () = assert!(RetainedBaseMaps::NODES_PER_LENS.is_multiple_of(CODES_PER_WORD));

const QUALIFICATION_A_ROWS: usize = 127;
const QUALIFICATION_A_COLS: usize = 259;
const QUALIFICATION_B_ROWS: usize = 131;
const QUALIFICATION_B_COLS: usize = 263;
const QUALIFICATION_STRIDE: usize = 512;
const RETAINED_FMA_BITS: [u32; 2] = [1_064_967_376, 1_051_445_982];

/// A deterministic CPU/native oracle that qualifies the actual adapter before
/// selected playback can consume this shader. WGSL does not promise the FMA
/// and exceptional-float behavior the estimator needs, so construction fails
/// closed if this exact workload disagrees even once.
struct QualificationFixture {
    sources: LensPair<SourceImage>,
    maps: RetainedBaseMaps,
    expected_preblur: SolverBelts,
    expected_blurred: BlurredBelts,
}

#[derive(Debug, PartialEq, Eq)]
enum GpuQualificationError {
    SolverByte {
        lens: Lens,
        row: usize,
        col: usize,
        actual: u8,
        expected: u8,
    },
    RetainedMap {
        actual: [u32; 2],
        expected: [u32; 2],
    },
    BlurredByte {
        lens: Lens,
        row: usize,
        col: usize,
        actual: u8,
        expected: u8,
    },
}

impl fmt::Display for GpuQualificationError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SolverByte {
                lens,
                row,
                col,
                actual,
                expected,
            } => write!(
                output,
                "ONE X2 GPU arithmetic is not exact on this graphics device: lens {lens} solver row {row} column {col} is {actual}, expected {expected}"
            ),
            Self::RetainedMap { actual, expected } => write!(
                output,
                "ONE X2 GPU arithmetic is not exact on this graphics device: retained-map FMA wrote {actual:?}, expected {expected:?}"
            ),
            Self::BlurredByte {
                lens,
                row,
                col,
                actual,
                expected,
            } => write!(
                output,
                "ONE X2 GPU arithmetic is not exact on this graphics device: lens {lens} blurred row {row} column {col} is {actual}, expected {expected}"
            ),
        }
    }
}

impl Error for GpuQualificationError {}

fn qualification_fixture() -> QualificationFixture {
    let source = |lens: Lens, rows: usize, cols: usize| {
        let mut pixels = (0..rows * cols)
            .map(|index| {
                let row = index / cols;
                let col = index % cols;
                match lens {
                    Lens::A => ((17 * row + 29 * col + 3) % 256) as u8,
                    Lens::B => ((43 * row + 11 * col + 197) % 256) as u8,
                }
            })
            .collect::<Vec<_>>();
        if lens == Lens::A {
            // Native-order source-FMA discriminator: at row .251, column
            // .871 the selected answer is 190; top-left-first writes 189.
            pixels[0] = 17;
            pixels[1] = 201;
            pixels[cols] = 93;
            pixels[cols + 1] = 248;
        }
        SourceImage::from_compact(rows, cols, pixels)
            .expect("the static GPU qualification source has its declared shape")
    };
    let sources = LensPair {
        a: source(Lens::A, QUALIFICATION_A_ROWS, QUALIFICATION_A_COLS),
        b: source(Lens::B, QUALIFICATION_B_ROWS, QUALIFICATION_B_COLS),
    };
    let map = |lens: Lens| {
        (0..RetainedBaseMaps::NODES_PER_LENS)
            .map(|index| {
                let row = index / super::COLS;
                let col = index % super::COLS;
                let selector = (31 * row + 47 * col + lens.index()) % 997;
                match selector {
                    0 => [0.0, 0.5],
                    1 => [-0.25, 0.75],
                    2 => [f32::NAN, 0.5],
                    3 => [0.5, f32::NAN],
                    4 => [1.25, 1.5],
                    5 => [f32::INFINITY, f32::INFINITY],
                    _ => {
                        let (rows, cols) = match lens {
                            Lens::A => (QUALIFICATION_A_ROWS, QUALIFICATION_A_COLS),
                            Lens::B => (QUALIFICATION_B_ROWS, QUALIFICATION_B_COLS),
                        };
                        let x = 1 + (13 * row + 7 * col + 19 * lens.index()) % (cols - 2);
                        let y = 1 + (5 * row + 23 * col + 29 * lens.index()) % (rows - 2);
                        [
                            (x as f32 + (col % 3) as f32 * 0.21) / cols as f32,
                            (y as f32 + (row % 3) as f32 * 0.37) / rows as f32,
                        ]
                    }
                }
            })
            .collect::<Vec<_>>()
    };
    let mut a = map(Lens::A);
    let b = map(Lens::B);
    let retained_fma_quad = [
        [
            [f32::from_bits(1_064_954_653), f32::from_bits(1_051_416_063)],
            [f32::from_bits(1_064_974_894), f32::from_bits(1_051_403_380)],
        ],
        [
            [f32::from_bits(1_064_972_601), f32::from_bits(1_051_518_451)],
            [f32::from_bits(1_064_992_780), f32::from_bits(1_051_505_921)],
        ],
    ];
    for dr in 0..2 {
        for dc in 0..2 {
            a[(1 + dr) * super::COLS + 47 + dc] = retained_fma_quad[dr][dc];
        }
    }
    let fma_uv = [
        0.871 / QUALIFICATION_A_COLS as f32,
        0.251 / QUALIFICATION_A_ROWS as f32,
    ];
    for row in 10..=11 {
        for col in 10..=11 {
            a[row * super::COLS + col] = fma_uv;
        }
    }
    let maps = RetainedBaseMaps::from_lenses(LensPair { a, b })
        .expect("the static GPU qualification maps have the retained shape");
    let expected_preblur = sample_source_belts(&sources, &maps).reduce_area_3x3();
    assert_eq!(
        expected_preblur.pixel(Lens::A, 10, 10),
        190,
        "the static GPU qualification source-FMA discriminator changed"
    );
    let expected_blurred = gaussian_blur(&expected_preblur);
    QualificationFixture {
        sources,
        maps,
        expected_preblur,
        expected_blurred,
    }
}

/// Direct retained-grid input for qualifying the integer Gaussian separately
/// from sampling. It plants isolated corner, edge and centre impulses; keeps
/// opposite A/B storage boundaries at distinct constants; and fills the rest
/// with alternating, ramp, constant and deterministic pseudorandom regions.
/// The complete comparison consequently exercises both reflect-101 axes and a
/// wide set of final Q14 rounding residues rather than relying on the sampled
/// source fixture to happen to cover them.
fn blur_qualification_fixture() -> SolverBelts {
    let centre = (super::ROWS / 2, super::COLS / 2);
    SolverBelts::from_fn(|lens, row, col| {
        let in_box = |at: (usize, usize), radius: usize| {
            row.abs_diff(at.0) <= radius && col.abs_diff(at.1) <= radius
        };
        match lens {
            Lens::A if in_box((0, 0), 3) => u8::from(row == 0 && col == 0) * 255,
            Lens::A if in_box((0, super::COLS / 2), 3) => {
                u8::from(row == 0 && col == super::COLS / 2) * 173
            }
            Lens::A if in_box(centre, 3) => u8::from((row, col) == centre) * 255,
            Lens::A if row >= super::ROWS - 5 && col >= super::COLS - 5 => 11,
            Lens::B if row < 5 && col < 5 => 241,
            Lens::B if row >= super::ROWS - 4 && col >= super::COLS - 4 => {
                u8::from(row == super::ROWS - 1 && col == super::COLS - 1) * 199
            }
            _ if row < 32 => u8::from((row + col + lens.index()).is_multiple_of(2)) * 255,
            _ if row < 96 => ((5 * row + 17 * col + 31 * lens.index()) % 256) as u8,
            _ if row < 128 => 137 + lens.index() as u8 * 41,
            _ => {
                let mut value = (row * super::COLS + col) as u32
                    ^ (0x9e37_79b9u32.wrapping_mul(lens.index() as u32 + 1));
                value ^= value >> 16;
                value = value.wrapping_mul(0x7feb_352d);
                value ^= value >> 15;
                (value >> 24) as u8
            }
        }
    })
}

fn qualification_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    source: &SourceImage,
) -> wgpu::Texture {
    let mut padded = vec![0xee; QUALIFICATION_STRIDE * source.rows()];
    for row in 0..source.rows() {
        padded[row * QUALIFICATION_STRIDE..row * QUALIFICATION_STRIDE + source.cols()]
            .copy_from_slice(&source.pixels()[row * source.cols()..(row + 1) * source.cols()]);
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: source.cols() as u32,
            height: source.rows() as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        &padded,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(QUALIFICATION_STRIDE as u32),
            rows_per_image: Some(source.rows() as u32),
        },
        texture.size(),
    );
    texture
}

/// The two exact R8 source textures in physical A/B order.
#[derive(Clone, Copy)]
pub(crate) struct SourceTextures<'a> {
    pub(crate) a: &'a wgpu::Texture,
    pub(crate) b: &'a wgpu::Texture,
}

enum SubmissionInput<'a> {
    Sampled { qualify_intermediates: bool },
    Preblurred(&'a SolverBelts),
}

enum MapInput<'a> {
    Uploaded(&'a RetainedBaseMaps),
    Resident(&'a wgpu::Buffer),
}

impl SourceTextures<'_> {
    fn validate(self) -> Fallible<()> {
        for (lens, texture) in [(Lens::A, self.a), (Lens::B, self.b)] {
            if texture.format() != wgpu::TextureFormat::R8Unorm {
                return Err(format!(
                    "ONE X2 GPU belt source {lens} is {:?}, expected R8Unorm",
                    texture.format()
                )
                .into());
            }
            let size = texture.size();
            if size.width == 0 || size.height == 0 || size.depth_or_array_layers != 1 {
                return Err(format!(
                    "ONE X2 GPU belt source {lens} is {} by {} by {}, expected a nonempty 2D texture",
                    size.width, size.height, size.depth_or_array_layers
                )
                .into());
            }
        }
        Ok(())
    }
}

/// Lazily constructed compute state. Each submission owns fresh map, output,
/// bind-group and readback resources so overlapping frames cannot overwrite
/// one another.
pub(crate) struct GpuSolverBeltPipeline {
    context: OneXsGpuContext,
    pipeline: wgpu::ComputePipeline,
    horizontal_pipeline: wgpu::ComputePipeline,
    vertical_pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    witness: wgpu::Buffer,
}

impl GpuSolverBeltPipeline {
    /// Build and qualify the exact arithmetic on the actual device.
    ///
    /// WGSL permits transformations that change native solver bytes. The
    /// qualification is therefore part of construction, not merely a test;
    /// an adapter that disagrees is refused with no CPU or approximate path.
    pub(crate) fn new(context: OneXsGpuContext) -> Fallible<Self> {
        Self::from_shader(context, SHADER)
    }

    fn from_shader(context: OneXsGpuContext, shader: &str) -> Fallible<Self> {
        let device = context.device();
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ONE X2 GPU solver belts"),
            entries: &[
                texture(0),
                texture(1),
                storage(2, true),
                storage(3, false),
                storage(4, false),
                storage(5, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 GPU solver belts"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 GPU solver belts"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ONE X2 GPU solver belts"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("build_solver_belts"),
            compilation_options: Default::default(),
            cache: None,
        });
        let horizontal_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ONE X2 GPU horizontal Gaussian"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("blur_horizontal"),
                compilation_options: Default::default(),
                cache: None,
            });
        let vertical_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ONE X2 GPU vertical Gaussian"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("blur_vertical"),
            compilation_options: Default::default(),
            cache: None,
        });
        // Qualification reads this once before construction returns. Later
        // overlapping submissions may overwrite it because ordinary playback
        // deliberately never reads the witness.
        let witness = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 retained-map FMA witness"),
            size: WITNESS_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let built = Self {
            context,
            pipeline,
            horizontal_pipeline,
            vertical_pipeline,
            layout,
            witness,
        };
        built.qualify()?;
        Ok(built)
    }

    fn qualify(&self) -> Fallible<()> {
        let device = self.context.device();
        let queue = self.context.queue();
        let fixture = qualification_fixture();
        let texture_a = qualification_texture(
            device,
            queue,
            "ONE X2 GPU qualification source A",
            &fixture.sources.a,
        );
        let texture_b = qualification_texture(
            device,
            queue,
            "ONE X2 GPU qualification source B",
            &fixture.sources.b,
        );
        let pending = self.submit_inner(
            SourceTextures {
                a: &texture_a,
                b: &texture_b,
            },
            &fixture.maps,
            (),
            SubmissionInput::Sampled {
                qualify_intermediates: true,
            },
            true,
        )?;
        let (actual_blurred, actual_preblur, retained_bits) = pending.read_qualification()?;
        if let Some(index) = actual_preblur
            .bytes()
            .iter()
            .zip(fixture.expected_preblur.bytes())
            .position(|(actual, expected)| actual != expected)
        {
            let lens = if index < RetainedBaseMaps::NODES_PER_LENS {
                Lens::A
            } else {
                Lens::B
            };
            let local = index % RetainedBaseMaps::NODES_PER_LENS;
            let row = local / super::COLS;
            let col = local % super::COLS;
            return Err(GpuQualificationError::SolverByte {
                lens,
                row,
                col,
                actual: actual_preblur.bytes()[index],
                expected: fixture.expected_preblur.bytes()[index],
            }
            .into());
        }

        if retained_bits != RETAINED_FMA_BITS {
            return Err(GpuQualificationError::RetainedMap {
                actual: retained_bits,
                expected: RETAINED_FMA_BITS,
            }
            .into());
        }
        if let Some(index) = actual_blurred
            .bytes()
            .iter()
            .zip(fixture.expected_blurred.bytes())
            .position(|(actual, expected)| actual != expected)
        {
            let lens = if index < RetainedBaseMaps::NODES_PER_LENS {
                Lens::A
            } else {
                Lens::B
            };
            let local = index % RetainedBaseMaps::NODES_PER_LENS;
            let row = local / super::COLS;
            let col = local % super::COLS;
            return Err(GpuQualificationError::BlurredByte {
                lens,
                row,
                col,
                actual: actual_blurred.bytes()[index],
                expected: fixture.expected_blurred.bytes()[index],
            }
            .into());
        }
        let blur_input = blur_qualification_fixture();
        let expected_blur = gaussian_blur(&blur_input);
        let actual_blur = self
            .submit_inner(
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                &fixture.maps,
                (),
                SubmissionInput::Preblurred(&blur_input),
                true,
            )?
            .read()?;
        if let Some(index) = actual_blur
            .bytes()
            .iter()
            .zip(expected_blur.bytes())
            .position(|(actual, expected)| actual != expected)
        {
            let lens = if index < RetainedBaseMaps::NODES_PER_LENS {
                Lens::A
            } else {
                Lens::B
            };
            let local = index % RetainedBaseMaps::NODES_PER_LENS;
            let row = local / super::COLS;
            let col = local % super::COLS;
            return Err(GpuQualificationError::BlurredByte {
                lens,
                row,
                col,
                actual: actual_blur.bytes()[index],
                expected: expected_blur.bytes()[index],
            }
            .into());
        }
        Ok(())
    }

    /// Submit while retaining the owner of imported source images.
    ///
    /// A dmabuf texture aliases a decoder surface. Scene integration must pass
    /// the corresponding frame owner here; retaining only wgpu handles does
    /// not keep that external surface out of the decoder's pool.
    #[allow(dead_code, reason = "explicit legacy solver-belt oracle boundary")]
    pub(crate) fn submit_retained<K>(
        &self,
        sources: SourceTextures<'_>,
        maps: &RetainedBaseMaps,
        source_owner: K,
    ) -> Fallible<PendingBlurredBelts<K>> {
        self.submit_inner(
            sources,
            maps,
            source_owner,
            SubmissionInput::Sampled {
                qualify_intermediates: false,
            },
            true,
        )
    }

    /// Submit the exact production producer without allocating or copying a
    /// CPU readback. The returned token retains the decoded source owner and
    /// exact capture flight until the device has completed the post-Gaussian
    /// payload.
    ///
    /// This is a staged GPU-estimator boundary. Selected playback does not use
    /// it until a qualified downstream consumer exists.
    #[allow(dead_code)]
    pub(crate) fn submit_resident_retained<K>(
        &self,
        sources: SourceTextures<'_>,
        maps: &RetainedBaseMaps,
        source_owner: K,
        flight: GpuPisFlight,
    ) -> Fallible<GpuBlurredBelts<K>> {
        let pending = self.submit_inner(
            sources,
            maps,
            source_owner,
            SubmissionInput::Sampled {
                qualify_intermediates: false,
            },
            false,
        )?;
        Ok(pending.into_resident(flight))
    }

    #[allow(clippy::too_many_arguments)]
    fn submit_inner<K>(
        &self,
        sources: SourceTextures<'_>,
        maps: &RetainedBaseMaps,
        source_owner: K,
        input: SubmissionInput<'_>,
        copy_to_cpu: bool,
    ) -> Fallible<PendingBlurredBelts<K>> {
        self.submit_inner_with_map(
            sources,
            MapInput::Uploaded(maps),
            source_owner,
            input,
            copy_to_cpu,
            None,
        )
    }

    fn submit_inner_with_map<K>(
        &self,
        sources: SourceTextures<'_>,
        map_input: MapInput<'_>,
        source_owner: K,
        input: SubmissionInput<'_>,
        copy_to_cpu: bool,
        encoder: Option<wgpu::CommandEncoder>,
    ) -> Fallible<PendingBlurredBelts<K>> {
        let device = self.context.device();
        let queue = self.context.queue();
        let qualify_intermediates = matches!(
            input,
            SubmissionInput::Sampled {
                qualify_intermediates: true
            }
        );
        let initial_preblur = match input {
            SubmissionInput::Sampled { .. } => None,
            SubmissionInput::Preblurred(belts) => Some(belts),
        };
        sources.validate()?;
        let uploaded_map = match map_input {
            MapInput::Uploaded(maps) => {
                let map = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("ONE X2 retained base maps"),
                    size: maps.bytes().len() as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&map, 0, maps.bytes());
                Some(map)
            }
            MapInput::Resident(_) => None,
        };
        let map = match map_input {
            MapInput::Uploaded(_) => uploaded_map
                .as_ref()
                .expect("uploaded map was allocated before binding"),
            MapInput::Resident(map) => map,
        };
        let packed = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 packed blurred solver belts"),
            size: OUTPUT_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if let Some(initial) = initial_preblur {
            queue.write_buffer(&packed, 0, &pack_belts(initial));
        }
        let horizontal = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 horizontal Gaussian sums"),
            size: HORIZONTAL_BYTES,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let readback = copy_to_cpu.then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ONE X2 blurred solver belt readback"),
                size: OUTPUT_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });
        let preblur_readback = qualify_intermediates.then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ONE X2 pre-blur qualification readback"),
                size: OUTPUT_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });
        let witness_readback = qualify_intermediates.then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ONE X2 retained-map FMA witness readback"),
                size: WITNESS_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });
        let view_a = sources.a.create_view(&Default::default());
        let view_b = sources.b.create_view(&Default::default());
        let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 GPU solver belt resources"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view_a),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view_b),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: map.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: packed.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.witness.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: horizontal.as_entire_binding(),
                },
            ],
        });
        let mut encoder = encoder.unwrap_or_else(|| {
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ONE X2 GPU solver belts"),
            })
        });
        if initial_preblur.is_none() {
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("ONE X2 GPU solver belts"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &resources, &[]);
                pass.dispatch_workgroups(OUTPUT_WORDS.div_ceil(WORKGROUP_SIZE), 1, 1);
            }
            if let Some(readback) = &preblur_readback {
                encoder.copy_buffer_to_buffer(&packed, 0, readback, 0, OUTPUT_BYTES);
            }
            if let Some(readback) = &witness_readback {
                encoder.copy_buffer_to_buffer(&self.witness, 0, readback, 0, WITNESS_BYTES);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 GPU horizontal Gaussian"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.horizontal_pipeline);
            pass.set_bind_group(0, &resources, &[]);
            pass.dispatch_workgroups((SolverBelts::BYTES as u32).div_ceil(WORKGROUP_SIZE), 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 GPU vertical Gaussian"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.vertical_pipeline);
            pass.set_bind_group(0, &resources, &[]);
            pass.dispatch_workgroups(OUTPUT_WORDS.div_ceil(WORKGROUP_SIZE), 1, 1);
        }
        if let Some(readback) = &readback {
            encoder.copy_buffer_to_buffer(&packed, 0, readback, 0, OUTPUT_BYTES);
        }
        let submission = queue.submit([encoder.finish()]);
        Ok(PendingBlurredBelts {
            lease: SubmissionLease::new(self.context.clone(), submission, source_owner),
            _map: uploaded_map,
            _packed: packed,
            _horizontal: horizontal,
            readback,
            preblur_readback,
            witness_readback,
            _resources: resources,
        })
    }
}

/// The exact device-owned submission whose source surface cannot be reused
/// until completion has been proved.
enum ExactSubmission {
    Device {
        context: OneXsGpuContext,
        index: wgpu::SubmissionIndex,
        #[cfg(test)]
        observer: Option<std::sync::Arc<std::sync::atomic::AtomicU8>>,
        #[cfg(test)]
        submit_observer: Option<std::sync::Arc<std::sync::atomic::AtomicU8>>,
    },
    #[cfg(test)]
    Injected {
        outcome: InjectedWait,
        observer: std::sync::Arc<std::sync::atomic::AtomicU8>,
    },
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum InjectedWait {
    Success,
    Error,
    Panic,
}

impl ExactSubmission {
    fn wait(self) -> Fallible<()> {
        match self {
            Self::Device {
                context,
                index,
                #[cfg(test)]
                observer,
                #[cfg(test)]
                    submit_observer: _,
            } => {
                #[cfg(test)]
                if let Some(observer) = &observer {
                    observer.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                let result = context
                    .device()
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(index),
                        timeout: None,
                    })
                    .map(|_| ())
                    .map_err(Box::<dyn Error + Send + Sync>::from);
                #[cfg(test)]
                if let Some(observer) = &observer {
                    observer.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                result
            }
            #[cfg(test)]
            Self::Injected { outcome, observer } => {
                observer.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let result = match outcome {
                    InjectedWait::Success => Ok(()),
                    InjectedWait::Error => Err("injected ONE X2 submission poll failure".into()),
                    InjectedWait::Panic => panic!("injected ONE X2 submission poll panic"),
                };
                observer.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                result
            }
        }
    }

    fn submit_after<F>(&mut self, producer: &OneXsGpuContext, encode: F) -> Fallible<()>
    where
        F: FnOnce(&wgpu::Device) -> wgpu::CommandBuffer,
    {
        match self {
            Self::Device { context, index, .. } => {
                context.ensure_same(producer)?;
                let command = encode(context.device());
                *index = context.queue().submit([command]);
                #[cfg(test)]
                if let Self::Device {
                    submit_observer: Some(observer),
                    ..
                } = self
                {
                    observer.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                Ok(())
            }
            #[cfg(test)]
            Self::Injected { .. } => {
                Err("injected ONE X2 GPU submission cannot accept a consumer".into())
            }
        }
    }

    #[cfg(test)]
    fn observe(&mut self, state: std::sync::Arc<std::sync::atomic::AtomicU8>) {
        match self {
            Self::Device { observer, .. } => *observer = Some(state),
            Self::Injected { observer, .. } => *observer = state,
        }
    }

    #[cfg(test)]
    fn observe_submit(&mut self, state: std::sync::Arc<std::sync::atomic::AtomicU8>) {
        if let Self::Device {
            submit_observer, ..
        } = self
        {
            *submit_observer = Some(state);
        }
    }
}

/// Linear ownership of one external source through one exact GPU submission.
///
/// Completion success releases the source owner and disarms the lease. A poll
/// failure instead intentionally leaks that owner: without completion proof,
/// returning an aliased decoder surface to its pool would permit GPU/decoder
/// reuse races. Drop performs the same fail-closed completion when a caller
/// abandons a pending submission before normal acknowledgement. Drop never
/// polls or waits: exceptional cancellation retains the source for process
/// life and discards the unusable proof token.
struct SubmissionLease<K> {
    completion: Option<ExactSubmission>,
    source_owner: Option<K>,
}

impl<K> SubmissionLease<K> {
    fn new(context: OneXsGpuContext, index: wgpu::SubmissionIndex, source_owner: K) -> Self {
        Self {
            completion: Some(ExactSubmission::Device {
                context,
                index,
                #[cfg(test)]
                observer: None,
                #[cfg(test)]
                submit_observer: None,
            }),
            source_owner: Some(source_owner),
        }
    }

    fn complete(&mut self) -> Fallible<()> {
        let Some(completion) = self.completion.take() else {
            return Ok(());
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| completion.wait())) {
            Ok(Ok(())) => {
                drop(self.source_owner.take());
                Ok(())
            }
            Ok(Err(error)) => {
                self.quarantine_owner();
                Err(error)
            }
            Err(payload) => {
                self.quarantine_owner();
                std::panic::resume_unwind(payload)
            }
        }
    }

    /// Return the imported source after the final validity mapping callback
    /// has proved completion of the same latest submission. This is callable
    /// only through the sealed ready-map path; polling that four-byte copy is
    /// the completion proof, so waiting on the same submission again would
    /// turn ordinary redraw into a blocking operation.
    fn acknowledge_mapped_completion(&mut self) -> Fallible<()> {
        self.completion
            .take()
            .ok_or("ONE X2 GPU submission lease was already completed")?;
        Ok(())
    }

    fn complete_into_owner_after_mapped_validity(&mut self) -> Fallible<K> {
        if self.completion.is_some() {
            return Err("ONE X2 GPU submission has no mapped completion proof".into());
        }
        self.source_owner
            .take()
            .ok_or_else(|| "ONE X2 GPU submission lease lost its source owner".into())
    }

    fn validate_provenance(&self, producer: &OneXsGpuContext) -> Fallible<()> {
        match &self.completion {
            Some(ExactSubmission::Device { context, .. }) => context.ensure_same(producer),
            #[cfg(test)]
            Some(ExactSubmission::Injected { .. }) => {
                Err("injected ONE X2 GPU submission has no device or queue provenance".into())
            }
            None => Err("ONE X2 GPU submission lease was already completed".into()),
        }
    }

    fn submit_after<F>(&mut self, producer: &OneXsGpuContext, encode: F) -> Fallible<()>
    where
        F: FnOnce(&wgpu::Device) -> wgpu::CommandBuffer,
    {
        let completion = self
            .completion
            .as_mut()
            .ok_or("ONE X2 GPU submission lease was already completed")?;
        completion.submit_after(producer, encode)
    }

    #[cfg(test)]
    fn injected(
        source_owner: K,
        outcome: InjectedWait,
        observer: std::sync::Arc<std::sync::atomic::AtomicU8>,
    ) -> Self {
        Self {
            completion: Some(ExactSubmission::Injected { outcome, observer }),
            source_owner: Some(source_owner),
        }
    }

    fn quarantine_owner(&mut self) {
        if let Some(owner) = self.source_owner.take() {
            // A failed or panicking poll is not proof that the GPU stopped
            // reading the imported decoder surface. Intentionally retain it
            // for the process lifetime so it cannot return to decoder reuse.
            std::mem::forget(owner);
        }
    }

    fn quarantine_for_drop(&mut self) {
        if self.completion.take().is_some() {
            self.quarantine_owner();
        }
    }

    #[cfg(test)]
    fn observe(&mut self, state: std::sync::Arc<std::sync::atomic::AtomicU8>) {
        if let Some(completion) = &mut self.completion {
            completion.observe(state);
        }
    }

    #[cfg(test)]
    fn observe_submit(&mut self, state: std::sync::Arc<std::sync::atomic::AtomicU8>) {
        if let Some(completion) = &mut self.completion {
            completion.observe_submit(state);
        }
    }
}

impl SubmissionLease<geometry_gpu::GpuGeometryFrameOwner<ImportedOneXsPicture>> {
    fn ensure_final_map_sources(&self, frame: &FrameStamp) -> Fallible<()> {
        self.source_owner
            .as_ref()
            .ok_or("ONE X2 final map lost its imported source owner")?
            .ensure_final_map_sources(frame)
    }

    fn copy_final_map_dynamic_inputs(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: map_patch_gpu::resident::DynamicInputTarget<'_>,
        public: &wgpu::Buffer,
    ) -> Fallible<()> {
        self.source_owner
            .as_ref()
            .ok_or("ONE X2 final map lost its imported source owner")?
            .copy_final_map_dynamic_inputs(encoder, target, public);
        Ok(())
    }
}

impl<K> Drop for SubmissionLease<K> {
    fn drop(&mut self) {
        // Cancellation cannot block a redraw or unwind. Without an explicit
        // completion proof the source stays unavailable to decoder reuse for
        // process life. Explicit diagnostic completion still preserves its
        // original error or panic.
        self.quarantine_for_drop();
    }
}

/// One submitted GPU solver-belt transaction.
#[must_use = "the submitted ONE X2 solver belts have not been consumed"]
pub(crate) struct PendingBlurredBelts<K> {
    lease: SubmissionLease<K>,
    _map: Option<wgpu::Buffer>,
    /// Retained through either the CPU copy or the resident consumer.
    _packed: wgpu::Buffer,
    _horizontal: wgpu::Buffer,
    readback: Option<wgpu::Buffer>,
    preblur_readback: Option<wgpu::Buffer>,
    witness_readback: Option<wgpu::Buffer>,
    _resources: wgpu::BindGroup,
}

impl<K> PendingBlurredBelts<K> {
    /// The compact GPU-resident A-then-B payload, four U8 codes per word.
    #[cfg(test)]
    pub(crate) fn packed(&self) -> &wgpu::Buffer {
        &self._packed
    }

    /// Wait for and consume the exact compact post-Gaussian payload.
    ///
    /// The distinct return type prevents the CPU estimator from applying the
    /// selected Gaussian for a second time.
    pub(crate) fn read(self) -> Fallible<BlurredBelts> {
        Ok(self.read_inner()?.0)
    }

    /// Make a no-readback submission a GPU-resident, frame-bound post-Gaussian
    /// resource without waiting on the CPU.
    ///
    /// A later same-queue submission orders its reads after this producer.
    /// The token retains every producer resource and the imported source owner
    /// until that consumer reaches an explicit CPU re-entry boundary.
    fn into_resident(self, flight: GpuPisFlight) -> GpuBlurredBelts<K> {
        debug_assert!(self.readback.is_none());
        debug_assert!(self.preblur_readback.is_none());
        debug_assert!(self.witness_readback.is_none());
        GpuBlurredBelts {
            flight: Some(flight),
            lease: self.lease,
            packed: self._packed,
            _producer_map: self._map,
            _horizontal: self._horizontal,
            _resources: self._resources,
        }
    }

    fn read_qualification(self) -> Fallible<(BlurredBelts, SolverBelts, [u32; 2])> {
        let (blurred, preblur, witness) = self.read_inner()?;
        Ok((
            blurred,
            preblur.expect("qualification requested its pre-blur payload"),
            witness.expect("qualification requested its retained-map FMA witness"),
        ))
    }

    fn read_inner(mut self) -> Fallible<(BlurredBelts, Option<SolverBelts>, Option<[u32; 2]>)> {
        let readback = self
            .readback
            .as_ref()
            .ok_or("ONE X2 GPU-resident blurred solver belts have no CPU readback")?;
        let slice = readback.slice(..);
        let (mapped, answer) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = mapped.send(result);
        });
        let witness = self.witness_readback.as_ref().map(|buffer| {
            let slice = buffer.slice(..);
            let (mapped, answer) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = mapped.send(result);
            });
            (slice, answer)
        });
        let preblur = self.preblur_readback.as_ref().map(|buffer| {
            let slice = buffer.slice(..);
            let (mapped, answer) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = mapped.send(result);
            });
            (slice, answer)
        });
        self.lease.complete()?;
        answer.recv()??;
        if let Some((_, answer)) = &witness {
            answer.recv()??;
        }
        if let Some((_, answer)) = &preblur {
            answer.recv()??;
        }
        let mapped = slice.get_mapped_range();
        let belts = unpack_blurred_belts(&mapped)?;
        drop(mapped);
        readback.unmap();
        let preblur = preblur
            .map(|(slice, _)| {
                let mapped = slice.get_mapped_range();
                let belts = unpack_solver_belts(&mapped);
                drop(mapped);
                self.preblur_readback
                    .as_ref()
                    .expect("mapped pre-blur payload has its buffer")
                    .unmap();
                belts
            })
            .transpose()?;
        let witness = witness.map(|(slice, _)| {
            let mapped = slice.get_mapped_range();
            let bits = [
                u32::from_ne_bytes(mapped[0..4].try_into().unwrap()),
                u32::from_ne_bytes(mapped[4..8].try_into().unwrap()),
            ];
            drop(mapped);
            self.witness_readback
                .as_ref()
                .expect("mapped witness has its buffer")
                .unmap();
            bits
        });
        Ok((belts, preblur, witness))
    }
}

/// Exact post-Gaussian solver belts that have never crossed into CPU memory.
///
/// The token is deliberately neither cloneable nor publicly constructible.
/// Its capture generation and opaque frame identity travel with the storage
/// allocation, so a later preprocessing result cannot be admitted by numeric
/// frame index alone.
#[must_use = "the GPU-resident post-Gaussian belts have not been consumed"]
pub(crate) struct GpuBlurredBelts<K> {
    flight: Option<GpuPisFlight>,
    lease: SubmissionLease<K>,
    packed: wgpu::Buffer,
    _producer_map: Option<wgpu::Buffer>,
    _horizontal: wgpu::Buffer,
    _resources: wgpu::BindGroup,
}

impl<K> GpuBlurredBelts<K> {
    #[cfg(test)]
    pub(crate) fn observe_completion(
        &mut self,
        state: std::sync::Arc<std::sync::atomic::AtomicU8>,
    ) {
        self.lease.observe(state);
    }
}

#[cfg(test)]
pub(crate) fn resident_qualification_fixture<K>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source_owner: K,
    flight: GpuPisFlight,
) -> Fallible<(GpuBlurredBelts<K>, BlurredBelts)> {
    let fixture = qualification_fixture();
    let expected = fixture.expected_blurred.clone();
    let texture_a = qualification_texture(
        device,
        queue,
        "ONE X2 composed resident source A",
        &fixture.sources.a,
    );
    let texture_b = qualification_texture(
        device,
        queue,
        "ONE X2 composed resident source B",
        &fixture.sources.b,
    );
    let pipeline = GpuSolverBeltPipeline::new(OneXsGpuContext::new(device, queue))?;
    let belts = pipeline.submit_resident_retained(
        SourceTextures {
            a: &texture_a,
            b: &texture_b,
        },
        &fixture.maps,
        source_owner,
        flight,
    )?;
    Ok((belts, expected))
}

#[cfg(test)]
fn resident_blurred_fixture<K>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source_owner: K,
    flight: GpuPisFlight,
    blurred: &BlurredBelts,
) -> Fallible<(GpuBlurredBelts<K>, BlurredBelts)> {
    let fixture = qualification_fixture();
    let texture_a = qualification_texture(
        device,
        queue,
        "ONE X2 resident blurred fixture source A",
        &fixture.sources.a,
    );
    let texture_b = qualification_texture(
        device,
        queue,
        "ONE X2 resident blurred fixture source B",
        &fixture.sources.b,
    );
    let pipeline = GpuSolverBeltPipeline::new(OneXsGpuContext::new(device, queue))?;
    let blurred = SolverBelts::from_lenses(LensPair {
        a: blurred.bytes()[..RetainedBaseMaps::NODES_PER_LENS].to_vec(),
        b: blurred.bytes()[RetainedBaseMaps::NODES_PER_LENS..].to_vec(),
    })?;
    let expected = gaussian_blur(&blurred);
    let belts = pipeline
        .submit_inner(
            SourceTextures {
                a: &texture_a,
                b: &texture_b,
            },
            &fixture.maps,
            source_owner,
            SubmissionInput::Preblurred(&blurred),
            false,
        )?
        .into_resident(flight);
    Ok((belts, expected))
}

fn unpack_belt_lenses(words: &[u8]) -> LensPair<Vec<u8>> {
    let bytes = words
        .chunks_exact(size_of::<u32>())
        .flat_map(|word| u32::from_ne_bytes(word.try_into().unwrap()).to_le_bytes())
        .collect::<Vec<_>>();
    debug_assert_eq!(bytes.len(), SolverBelts::BYTES);
    LensPair {
        a: bytes[..RetainedBaseMaps::NODES_PER_LENS].to_vec(),
        b: bytes[RetainedBaseMaps::NODES_PER_LENS..].to_vec(),
    }
}

fn unpack_solver_belts(words: &[u8]) -> Fallible<SolverBelts> {
    SolverBelts::from_lenses(unpack_belt_lenses(words))
        .map_err(Box::<dyn Error + Send + Sync>::from)
}

fn unpack_blurred_belts(words: &[u8]) -> Fallible<BlurredBelts> {
    BlurredBelts::from_lenses(unpack_belt_lenses(words))
        .map_err(Box::<dyn Error + Send + Sync>::from)
}

fn pack_belts(belts: &SolverBelts) -> Vec<u8> {
    belts
        .bytes()
        .chunks_exact(CODES_PER_WORD)
        .flat_map(|codes| u32::from_le_bytes(codes.try_into().unwrap()).to_ne_bytes())
        .collect()
}

// The explicit `fma` chain and native `(1-coordinate)+floor(coordinate)`
// weights mirror the CPU oracle. WGSL permits a backend to expand `fma`, so
// byte identity remains an adapter-tested contract rather than a promise made
// from source spelling alone. NaN and infinity handling can also vary with
// backend finite-math policy. Runtime qualification and the required GPU twin
// below gate the actual adapter.
const SHADER: &str = r#"
const ROWS = 1080u;
const COLS = 60u;
const PIXELS_PER_LENS = ROWS * COLS;
const TOTAL_CODES = 2u * PIXELS_PER_LENS;
const AREA = 3u;
const THIRD_BITS: u32 = 0x3eaaaaabu;

@group(0) @binding(0) var source_a: texture_2d<f32>;
@group(0) @binding(1) var source_b: texture_2d<f32>;
@group(0) @binding(2) var<storage, read> base_maps: array<vec2<f32>>;
@group(0) @binding(3) var<storage, read_write> output_words: array<u32>;
@group(0) @binding(4) var<storage, read_write> witness_words: array<u32>;
@group(0) @binding(5) var<storage, read_write> horizontal_codes: array<u32>;

fn weights(value: f32, maximum: f32) -> vec4<f32> {
    let clamped = clamp(value, 0.0, maximum);
    let low = floor(clamped);
    let low_side = (1.0 - clamped) + low;
    return vec4<f32>(low_side, 1.0 - low_side, low, clamped);
}

fn base_at(lens: u32, row: i32, col: i32) -> vec2<f32> {
    let r = u32(clamp(row, 0, i32(ROWS) - 1));
    let c = u32(clamp(col, 0, i32(COLS) - 1));
    return base_maps[lens * PIXELS_PER_LENS + r * COLS + c];
}

fn sample_base(lens: u32, row: f32, col: f32) -> vec2<f32> {
    let rw = weights(row, f32(ROWS - 1u));
    let cw = weights(col, f32(COLS - 1u));
    let top_left = cw.x * rw.x;
    let top_right = rw.x - top_left;
    let bottom_left = cw.x - top_left;
    let bottom_right = ((1.0 - cw.x) - rw.x) + top_left;
    let ri = i32(rw.z);
    let ci = i32(cw.z);
    let tl = base_at(lens, ri, ci);
    let tr = base_at(lens, ri, ci + 1);
    let bl = base_at(lens, ri + 1, ci);
    let br = base_at(lens, ri + 1, ci + 1);
    var value = top_right * tr;
    value = fma(vec2<f32>(top_left), tl, value);
    value = fma(vec2<f32>(bottom_left), bl, value);
    return fma(vec2<f32>(bottom_right), br, value);
}

fn texel(lens: u32, row: i32, col: i32, dimensions: vec2<i32>) -> f32 {
    let at = vec2<i32>(clamp(col, 0, dimensions.x - 1), clamp(row, 0, dimensions.y - 1));
    var value: f32;
    if lens == 0u {
        value = textureLoad(source_a, at, 0).r;
    } else {
        value = textureLoad(source_b, at, 0).r;
    }
    return round(value * 255.0);
}

fn sample_source(lens: u32, uv: vec2<f32>) -> u32 {
    if !(uv.x > 0.0 && uv.y > 0.0) {
        return 0u;
    }
    var dimensions: vec2<i32>;
    if lens == 0u {
        dimensions = vec2<i32>(textureDimensions(source_a));
    } else {
        dimensions = vec2<i32>(textureDimensions(source_b));
    }
    let xw = weights(uv.x * f32(dimensions.x), f32(dimensions.x - 1));
    let yw = weights(uv.y * f32(dimensions.y), f32(dimensions.y - 1));
    let top_left = xw.x * yw.x;
    let top_right = yw.x - top_left;
    let bottom_left = xw.x - top_left;
    let bottom_right = ((1.0 - xw.x) - yw.x) + top_left;
    let xi = i32(xw.z);
    let yi = i32(yw.z);
    var value = top_right * texel(lens, yi, xi + 1, dimensions);
    value = fma(top_left, texel(lens, yi, xi, dimensions), value);
    value = fma(bottom_left, texel(lens, yi + 1, xi, dimensions), value);
    value = fma(bottom_right, texel(lens, yi + 1, xi + 1, dimensions), value);
    return u32(value);
}

fn solver_code(index: u32) -> u32 {
    let lens = index / PIXELS_PER_LENS;
    let local = index - lens * PIXELS_PER_LENS;
    let row = local / COLS;
    let col = local - row * COLS;
    var sum = 0u;
    for (var dr = 0u; dr < AREA; dr += 1u) {
        for (var dc = 0u; dc < AREA; dc += 1u) {
            let source_row = row * AREA + dr;
            let source_col = col * AREA + dc;
            let third = bitcast<f32>(THIRD_BITS);
            let uv = sample_base(lens, f32(source_row) * third, f32(source_col) * third);
            if index == COLS + 47u && dr == 1u && dc == 1u {
                witness_words[0] = bitcast<u32>(uv.x);
                witness_words[1] = bitcast<u32>(uv.y);
            }
            sum += sample_source(lens, uv);
        }
    }
    return (sum + 4u) / 9u;
}

@compute @workgroup_size(64)
fn build_solver_belts(@builtin(global_invocation_id) id: vec3<u32>) {
    let first = id.x * 4u;
    if first >= TOTAL_CODES {
        return;
    }
    var packed = 0u;
    for (var lane = 0u; lane < 4u; lane += 1u) {
        packed |= solver_code(first + lane) << (8u * lane);
    }
    output_words[id.x] = packed;
}

fn packed_code(index: u32) -> u32 {
    let word = output_words[index / 4u];
    return (word >> (8u * (index % 4u))) & 255u;
}

fn reflect_101(position: i32, length: i32) -> u32 {
    if position < 0 {
        return u32(-position);
    }
    if position >= length {
        return u32(2 * length - position - 2);
    }
    return u32(position);
}

@compute @workgroup_size(64)
fn blur_horizontal(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if index >= TOTAL_CODES {
        return;
    }
    let lens = index / PIXELS_PER_LENS;
    let local = index - lens * PIXELS_PER_LENS;
    let row = local / COLS;
    let col = local - row * COLS;
    var sum = 0u;
    sum += 3u * packed_code(lens * PIXELS_PER_LENS + row * COLS + reflect_101(i32(col) - 2, i32(COLS)));
    sum += 29u * packed_code(lens * PIXELS_PER_LENS + row * COLS + reflect_101(i32(col) - 1, i32(COLS)));
    sum += 64u * packed_code(index);
    sum += 29u * packed_code(lens * PIXELS_PER_LENS + row * COLS + reflect_101(i32(col) + 1, i32(COLS)));
    sum += 3u * packed_code(lens * PIXELS_PER_LENS + row * COLS + reflect_101(i32(col) + 2, i32(COLS)));
    horizontal_codes[index] = sum;
}

fn vertical_code(index: u32) -> u32 {
    let lens = index / PIXELS_PER_LENS;
    let local = index - lens * PIXELS_PER_LENS;
    let row = local / COLS;
    let col = local - row * COLS;
    var sum = 0u;
    sum += 3u * horizontal_codes[lens * PIXELS_PER_LENS + reflect_101(i32(row) - 2, i32(ROWS)) * COLS + col];
    sum += 29u * horizontal_codes[lens * PIXELS_PER_LENS + reflect_101(i32(row) - 1, i32(ROWS)) * COLS + col];
    sum += 64u * horizontal_codes[index];
    sum += 29u * horizontal_codes[lens * PIXELS_PER_LENS + reflect_101(i32(row) + 1, i32(ROWS)) * COLS + col];
    sum += 3u * horizontal_codes[lens * PIXELS_PER_LENS + reflect_101(i32(row) + 2, i32(ROWS)) * COLS + col];
    return (sum + 8192u) >> 14u;
}

@compute @workgroup_size(64)
fn blur_vertical(@builtin(global_invocation_id) id: vec3<u32>) {
    let first = id.x * 4u;
    if first >= TOTAL_CODES {
        return;
    }
    var packed = 0u;
    for (var lane = 0u; lane < 4u; lane += 1u) {
        packed |= vertical_code(first + lane) << (8u * lane);
    }
    output_words[id.x] = packed;
}
"#;

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::time::Duration;

    use kjerag_media::FrameStamp;
    use kjerag_meta::{
        ExposureTrack, GyroConfig, GyroEncoding, GyroTrack, OrientationSample, Quat, Size,
    };

    use super::*;
    use crate::projection::tests::{ONE_XS_FRAME, one_xs_lenses};

    const SESSION_CENTER: Duration = Duration::from_micros(2_000_000);

    #[test]
    fn cold_l1_initial_probe_compacts_plane_padding() {
        let logical = crate::flow::one_xs::pis::Level::One.patches();
        let stride = logical + 16;
        let mut allocation = vec![0xdead_beefu32; 4 * stride];
        for plane in 0..4 {
            for word in 0..logical {
                allocation[plane * stride + word] = ((plane as u32) << 24) | word as u32;
            }
        }

        let compact = compact_l1_initial_planes(&allocation, stride as u64).unwrap();
        assert_eq!(compact.len(), 4 * logical);
        for plane in 0..4 {
            assert_eq!(compact[plane * logical], (plane as u32) << 24);
            assert_eq!(
                compact[(plane + 1) * logical - 1],
                ((plane as u32) << 24) | (logical - 1) as u32
            );
        }
        assert!(!compact.contains(&0xdead_beef));
    }

    #[allow(dead_code)]
    fn mode_neutral_final_install_typechecks<P>(
        map: map_patch_gpu::GpuPackedMapFrame<pis_frontend_gpu::GpuFinalOperands<P>>,
        pipeline: Arc<DirectType2Pipeline>,
        retirements: &IcedDrawRetirements<InstalledOneXsPass>,
    ) -> Fallible<ResidentInstallCandidate>
    where
        P: geometry_gpu::temporal_gpu::GpuPriorPublicLevelTwo + Send + Sync + 'static,
    {
        prepare_resident_install(map, pipeline, retirements)
    }

    fn session_calibration(readout_ms: f64, principal_delta: f64) -> CalibrationSet {
        let mut lenses = one_xs_lenses();
        lenses[0].intrinsics.cx += principal_delta;
        CalibrationSet {
            camera_model: "Insta360 ONE X2".to_owned(),
            firmware: format!("resident-session-{readout_ms}"),
            dimension: Size {
                width: ONE_XS_FRAME.width,
                height: ONE_XS_FRAME.height,
            },
            lenses,
            rolling_shutter_ms: readout_ms,
            gyro: GyroConfig {
                encoding: GyroEncoding::Scaled,
                imu_orientation: "Zxy",
                first_frame_timestamp: 0,
                gyro_timestamp: None,
            },
            exposure: [ExposureTrack::default(), ExposureTrack::default()],
            imu: GyroTrack::default(),
            fused: OrientationTrack::default(),
            calibration_canvas: Size {
                width: 6_080,
                height: 3_040,
            },
        }
    }

    fn session_orientation(scale: f64) -> OrientationTrack {
        OrientationTrack::from_samples(
            (1_960_000..=2_040_000)
                .step_by(2_000)
                .map(|offset_us| OrientationSample {
                    offset_us,
                    world_from_body: Quat::from_rotation_vector([
                        (offset_us - 2_000_000) as f64 * 1.0e-7 * scale,
                        (offset_us - 2_000_000) as f64 * -0.5e-7 * scale,
                        (offset_us - 2_000_000) as f64 * 0.25e-7 * scale,
                    ]),
                })
                .collect(),
        )
    }

    struct DropProbe {
        wait_state: Arc<AtomicU8>,
        dropped: mpsc::Sender<u8>,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            let _ = self.dropped.send(self.wait_state.load(Ordering::SeqCst));
        }
    }

    struct SessionProbeSource {
        identity: ResidentSourceIdentity,
        context: OneXsGpuContext,
        frame: FrameStamp,
    }

    impl ImportedOneXsSource for SessionProbeSource {
        fn ensure_resident_context(&self, context: &OneXsGpuContext) -> Fallible<()> {
            self.context.ensure_same(context)
        }

        fn ensure_resident_session(&self, session: &ResidentSourceIdentity) -> Fallible<()> {
            self.identity.ensure_matches(session)
        }

        fn resident_frame(&self) -> FrameStamp {
            self.frame.clone()
        }

        fn submit_with(self, _binder: ResidentSourceBinder<'_>) -> Fallible<ResidentImportedFront> {
            panic!("a foreign-session source must refuse before binding")
        }
    }

    #[test]
    fn resident_front_admits_only_the_concrete_imported_owner() {
        let source = include_str!("one_xs_belt_gpu.rs");
        let production = source.split_once("#[cfg(test)]\nmod tests").unwrap().0;
        let admission = source
            .split_once("impl ResidentSourceCapture {")
            .unwrap()
            .1
            .split_once("pub(crate) fn submit_imported(")
            .unwrap()
            .1
            .split_once("}\n}")
            .unwrap()
            .0;

        assert!(admission.contains("source: ImportedOneXsPicture"));
        assert!(admission.contains("self.pipeline.submit_source(source)"));
        assert!(!admission.contains("SourceTextures"));
        assert!(!admission.contains("source_owner"));
        assert!(!admission.contains("ParentMapBuilder"));
        assert!(!admission.contains("OneXsResources"));
        assert!(!admission.contains("readout:"));
        assert!(!admission.contains("orientation:"));

        let pipeline_admission = production
            .split_once("impl ResidentSourceFrontPipeline {")
            .unwrap()
            .1
            .split_once("fn submit_source")
            .unwrap()
            .1
            .split_once("}\n}")
            .unwrap()
            .0;
        assert!(
            pipeline_admission.find("ensure_resident_context").unwrap()
                < pipeline_admission.find("begin_parent").unwrap()
        );
        assert!(
            pipeline_admission.find("resident_frame()").unwrap()
                < pipeline_admission.find("begin_parent").unwrap()
        );
    }

    #[test]
    fn capture_constructor_is_the_only_calibration_composition_boundary() {
        let source = include_str!("one_xs_belt_gpu.rs");
        let production = source.split_once("#[cfg(test)]\nmod tests").unwrap().0;
        let pipeline = production
            .split_once("impl ResidentSourceFrontPipeline {")
            .unwrap()
            .1
            .split_once("/// One open capture's inseparable")
            .unwrap()
            .0;
        let capture = production
            .split_once("impl ResidentSourceCapture {")
            .unwrap()
            .1
            .split_once("/// One-shot callback")
            .unwrap()
            .0;

        assert!(pipeline.contains("ParentMapBuilder::new(calibration)"));
        assert!(pipeline.contains("self.parent_inputs.readout()"));
        assert!(pipeline.contains("OneXsResources::new(&calibration.lenses)"));
        assert!(pipeline.contains("GpuResidentFramePipeline::new(context.clone())"));
        assert!(pipeline.contains("GpuResidentCapture::new_bound("));
        assert!(capture.contains("ResidentSourceFrontPipeline::new("));
        assert!(!production.contains("pub(crate) struct ResidentSourceFrontPipeline"));
        assert!(!pipeline.contains("root: &resident_frame_gpu::GpuResidentCapture"));
    }

    #[test]
    fn distinct_calibrations_with_colliding_frames_keep_separate_roots_and_encoders() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}"
                );
                eprintln!("skipping ONE X2 resident session test: {why}");
                return;
            }
        };
        let context = OneXsGpuContext::new(&device, &queue);
        let calibration_a = session_calibration(20.0, 0.0);
        let calibration_b = session_calibration(23.516, 0.25);
        let capture_a =
            ResidentSourceCapture::new(context.clone(), &calibration_a, session_orientation(1.0))
                .unwrap_or_else(|error| panic!("capture A failed on {adapter}: {error}"));
        let capture_b =
            ResidentSourceCapture::new(context, &calibration_b, session_orientation(2.0))
                .unwrap_or_else(|error| panic!("capture B failed on {adapter}: {error}"));
        let frame = FrameStamp::for_test(77, SESSION_CENTER, None);

        let error = capture_a
            .pipeline
            .submit_source(SessionProbeSource {
                identity: capture_b.source_identity(),
                context: capture_b.import_context().clone(),
                frame: frame.clone(),
            })
            .err()
            .expect("a foreign imported session must be refused");
        assert_eq!(
            error.to_string(),
            "ONE X2 imported source belongs to a different resident capture session"
        );
        assert_eq!(capture_a.pipeline.parent.parent.encoded_transitions(), 0);
        assert_eq!(capture_b.pipeline.parent.parent.encoded_transitions(), 0);
        assert_eq!(capture_a.pipeline.root.snapshot().generation, 0);
        assert_eq!(capture_b.pipeline.root.snapshot().generation, 0);

        let encoded_a = capture_a.pipeline.begin_parent(frame.clone()).unwrap();
        assert_eq!(capture_a.pipeline.parent.parent.encoded_transitions(), 1);
        assert_eq!(capture_b.pipeline.parent.parent.encoded_transitions(), 0);
        assert_eq!(capture_a.pipeline.root.snapshot().generation, 1);
        assert_eq!(capture_b.pipeline.root.snapshot().generation, 0);
        assert_ne!(
            capture_a.pipeline.parent_inputs.readout().seconds.to_bits(),
            capture_b.pipeline.parent_inputs.readout().seconds.to_bits()
        );
        assert_ne!(
            capture_a.pipeline.orientation,
            capture_b.pipeline.orientation
        );
        drop(encoded_a);
        assert!(!capture_a.pipeline.root.snapshot().pending);

        let encoded_b = capture_b.pipeline.begin_parent(frame).unwrap();
        assert_eq!(capture_a.pipeline.parent.parent.encoded_transitions(), 1);
        assert_eq!(capture_b.pipeline.parent.parent.encoded_transitions(), 1);
        assert_eq!(capture_a.pipeline.root.snapshot().generation, 1);
        assert_eq!(capture_b.pipeline.root.snapshot().generation, 1);
        drop(encoded_b);
    }

    #[test]
    fn resident_front_returns_only_an_opaque_consuming_continuation() {
        let source = include_str!("one_xs_belt_gpu.rs");
        let result = source
            .split_once("pub(crate) struct ResidentImportedFront")
            .unwrap()
            .1
            .split_once("const CODES_PER_WORD")
            .unwrap()
            .0;

        assert!(result.contains("fn prepare_motion("));
        assert!(!result.contains("fn texture"));
        assert!(!result.contains("fn planes"));
        assert!(!result.contains("fn bind_group"));
        assert!(!result.contains("fn source_owner"));
        assert!(!result.contains("fn device"));
        assert!(!result.contains("fn queue"));
    }

    #[test]
    fn resident_facade_api_keeps_renderer_resources_and_transactions_opaque() {
        let source = include_str!("one_xs_belt_gpu.rs");
        let facade = source
            .split_once("pub(crate) enum ResidentRetry")
            .unwrap()
            .1
            .split_once("pub(crate) struct ResidentSourceBinder")
            .unwrap()
            .0;
        assert!(facade.contains("Arc<ResidentCaptureFacadeInner>"));
        assert!(facade.contains("Arc<IcedDrawRetirements<InstalledOneXsPass>>"));
        assert!(facade.contains("picture_layout: wgpu::BindGroupLayout"));
        assert!(facade.contains("sampler: wgpu::Sampler"));
        assert!(facade.contains("direct: Arc<DirectType2Pipeline>"));
        assert!(facade.contains("ResidentTransaction::Pending"));
        assert!(facade.contains("finish_after_poll_classified()"));
        assert!(facade.contains("collect_after_external_poll()"));
        assert!(facade.contains("prepare_screenshot("));
        assert!(facade.contains("selected_pis_interval(Direction::AtoB, Level::Two)"));
        assert!(facade.contains("selected_pis_interval(Direction::BtoA, Level::One)"));
        assert!(facade.contains("one-time diagnostic readbacks may wait"));
        for forbidden in [
            "pub(crate) fn buffer",
            "pub(crate) fn texture",
            "pub(crate) fn bind_group",
            "pub(crate) fn map",
            "pub(crate) fn pending",
        ] {
            assert!(!facade.contains(forbidden), "facade exposes {forbidden}");
        }

        let imported = include_str!("../direct_type2.rs")
            .split_once("fn import_for_capture(")
            .unwrap()
            .1
            .split_once("pub(crate) fn submit_resident_front")
            .unwrap()
            .0;
        assert!(imported.contains("ONE X2 resident picture uniforms"));
        assert!(imported.contains("write_buffer(&uniforms, 0, reframe.bytes())"));
        assert!(!imported.contains("uniforms: &wgpu::Buffer"));
        assert!(source.contains("ONE X2 resident draw-private uniforms"));
        assert!(source.contains("InstalledOneXsPass"));
    }

    #[test]
    fn resident_facade_reuses_exact_session_and_refuses_renderer_mismatch() {
        let (device, queue, foreign_device, foreign_queue, adapter) = match gpu_pair() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}"
                );
                eprintln!("skipping ONE X2 resident facade test: {why}");
                return;
            }
        };
        let context = OneXsGpuContext::new(&device, &queue);
        let facade = ResidentCaptureFacade::new(
            Arc::new(session_calibration(20.0, 0.0)),
            session_orientation(1.0),
        );
        let first = facade
            .attach_renderer(context.clone(), wgpu::TextureFormat::Rgba8Unorm)
            .unwrap_or_else(|error| panic!("resident facade failed on {adapter}: {error}"));
        let first_session = facade.state().unwrap().session.as_ref().unwrap().clone();
        drop(first);
        let second = facade
            .attach_renderer(context.clone(), wgpu::TextureFormat::Rgba8Unorm)
            .unwrap();
        let second_session = facade.state().unwrap().session.as_ref().unwrap().clone();
        assert!(Arc::ptr_eq(&first_session, &second_session));
        assert!(Arc::ptr_eq(
            &first_session.retirements,
            &second_session.retirements
        ));
        assert_eq!(second.acknowledged().unwrap(), None);

        let error = facade
            .attach_renderer(context.clone(), wgpu::TextureFormat::Rgba8UnormSrgb)
            .err()
            .expect("changed resident surface format must refuse");
        assert_eq!(
            error.to_string(),
            "ONE X2 resident capture render format changed from Rgba8Unorm to Rgba8UnormSrgb"
        );

        let foreign_context = OneXsGpuContext::new(&foreign_device, &foreign_queue);
        let error = facade
            .attach_renderer(foreign_context, wgpu::TextureFormat::Rgba8Unorm)
            .err()
            .expect("a separately requested renderer device must refuse");
        assert_eq!(
            error.to_string(),
            "ONE X2 GPU submission crossed a different device or queue"
        );

        let stamp = FrameStamp::for_test(0, Duration::ZERO, None);
        let wrong_size = kjerag_media::Size {
            width: ONE_XS_FRAME.width - 1,
            height: ONE_XS_FRAME.height,
        };
        let error = second
            .submit_frame(
                &context,
                wgpu::TextureFormat::Rgba8Unorm,
                Arc::new(Frames::empty_for_test(stamp, wrong_size)),
                &crate::Reframe::blank(1.0, false),
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "ONE X2 source frame is {}x{} but calibration requires {}x{}",
                ONE_XS_FRAME.width - 1,
                ONE_XS_FRAME.height,
                ONE_XS_FRAME.width,
                ONE_XS_FRAME.height
            )
        );

        let quarantined_stamp = FrameStamp::for_test(0, Duration::ZERO, None);
        let error = second
            .submit_frame(
                &context,
                wgpu::TextureFormat::Rgba8Unorm,
                Arc::new(Frames::empty_for_test(
                    quarantined_stamp,
                    kjerag_media::Size {
                        width: ONE_XS_FRAME.width,
                        height: ONE_XS_FRAME.height,
                    },
                )),
                &crate::Reframe::blank(1.0, false),
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ONE X2 source import requires exactly 2 lens frames, got 0"
        );
        assert!(matches!(
            facade.state().unwrap().transaction,
            ResidentTransaction::Quarantined
        ));
        let retry_stamp = FrameStamp::for_test(0, Duration::ZERO, None);
        let error = second
            .submit_frame(
                &context,
                wgpu::TextureFormat::Rgba8Unorm,
                Arc::new(Frames::empty_for_test(
                    retry_stamp,
                    kjerag_media::Size {
                        width: ONE_XS_FRAME.width,
                        height: ONE_XS_FRAME.height,
                    },
                )),
                &crate::Reframe::blank(1.0, false),
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ONE X2 resident transaction facade is quarantined"
        );
    }

    #[test]
    fn resident_start_guard_quarantines_submit_and_install_unwind() {
        let facade = ResidentCaptureFacade::new(
            Arc::new(session_calibration(20.0, 0.0)),
            session_orientation(1.0),
        );
        facade.state().unwrap().transaction = ResidentTransaction::Starting;
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let inner = Arc::clone(&facade.inner);
            move || {
                let _start = ResidentStartGuard::quarantine(inner);
                panic!("injected resident submit unwind");
            }
        }));
        assert!(unwind.is_err());
        assert!(matches!(
            facade.state().unwrap().transaction,
            ResidentTransaction::Quarantined
        ));

        // Exercise the independent install-side guard from a fresh sentinel.
        facade.state().unwrap().transaction = ResidentTransaction::Starting;
        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let inner = Arc::clone(&facade.inner);
            move || {
                let _install = ResidentStartGuard::quarantine(inner);
                panic!("injected resident install unwind");
            }
        }));
        assert!(unwind.is_err());
        assert!(matches!(
            facade.state().unwrap().transaction,
            ResidentTransaction::Quarantined
        ));
    }

    #[test]
    fn resident_facade_sequence_gate_rejects_every_discontinuity() {
        let zero = FrameStamp::for_test(0, Duration::ZERO, None);
        let one = FrameStamp::for_test(1, Duration::from_millis(1), Some(&zero));
        let duplicate = FrameStamp::for_test(1, Duration::from_millis(1), Some(&zero));
        let gap = FrameStamp::for_test(3, Duration::from_millis(3), Some(&zero));
        let backward = FrameStamp::for_test(0, Duration::ZERO, Some(&zero));
        let changed_epoch = FrameStamp::for_test(2, Duration::from_millis(2), None);
        let exhausted = FrameStamp::for_test(u64::MAX, Duration::MAX, None);

        assert!(validate_resident_sequence(None, &zero).is_ok());
        assert_eq!(
            validate_resident_sequence(None, &one)
                .unwrap_err()
                .to_string(),
            "ONE X2 stitching must start at frame 0; first offered frame is 1"
        );
        assert!(validate_resident_sequence(Some(&zero), &one).is_ok());
        assert_eq!(
            validate_resident_sequence(Some(&one), &duplicate)
                .unwrap_err()
                .to_string(),
            "ONE X2 stitching repeats frame 1"
        );
        assert_eq!(
            validate_resident_sequence(Some(&one), &gap)
                .unwrap_err()
                .to_string(),
            "ONE X2 stitching skipped frame 2; next offered frame is 3"
        );
        assert_eq!(
            validate_resident_sequence(Some(&one), &backward)
                .unwrap_err()
                .to_string(),
            "ONE X2 stitching moved backward from frame 1 to frame 0"
        );
        assert_eq!(
            validate_resident_sequence(Some(&one), &changed_epoch)
                .unwrap_err()
                .to_string(),
            "ONE X2 stitching cannot continue after a seek or decoder restart"
        );
        assert_eq!(
            validate_resident_sequence(Some(&exhausted), &exhausted)
                .unwrap_err()
                .to_string(),
            "ONE X2 stitching cannot advance past frame 18446744073709551615"
        );
    }

    #[test]
    fn capture_acknowledgement_requires_the_exact_full_frame_stamp() {
        let facade = ResidentCaptureFacade::new(
            Arc::new(session_calibration(20.0, 0.0)),
            session_orientation(1.0),
        );
        let installed = FrameStamp::for_test(7, Duration::from_millis(7), None);
        let same_values_other_delivery = FrameStamp::for_test(7, Duration::from_millis(7), None);
        facade.state().unwrap().installed = Some(installed.clone());
        assert!(facade.acknowledged(&installed).unwrap());
        assert!(!facade.acknowledged(&same_values_other_delivery).unwrap());
    }

    #[test]
    fn installed_draw_api_is_whole_payload_only_and_carrier_first() {
        let source = include_str!("one_xs_belt_gpu.rs");
        let installed = source
            .split_once("pub(crate) struct InstalledOneXsDraw")
            .unwrap()
            .1
            .split_once("pub(crate) struct ResidentInstallCandidate")
            .unwrap()
            .0;
        assert!(installed.contains("source: ImportedOneXsPicture"));
        assert!(installed.contains("map: map_patch_gpu::InstalledGpuMapBinding"));
        assert!(installed.contains("pipeline: Arc<DirectType2Pipeline>"));
        assert!(installed.contains("fn draw("));
        assert!(installed.contains("fn prepare_pass("));
        for forbidden in [
            "fn source(",
            "fn map(",
            "fn bind_group(",
            "fn buffer(",
            "fn device(",
            "fn queue(",
            "fn pipeline(",
        ] {
            assert!(!installed.contains(forbidden), "found {forbidden}");
        }

        let adapter = source
            .split_once("pub(crate) struct IcedInstalledDrawAdapter")
            .unwrap()
            .1
            .split_once("impl ResidentInstallCandidate")
            .unwrap()
            .0;
        assert!(adapter.contains("IcedDrawRetirements<InstalledOneXsPass>"));
        assert!(adapter.contains("Mutex<Option<InstalledOneXsReady>>"));
        assert!(adapter.contains("fn poll_prepare(&self) -> Fallible<usize>"));
        assert!(adapter.contains("fn prepare_redraw("));
        assert!(adapter.contains("-> Result<bool, DrawRetirementError>"));
        assert!(
            adapter.find("self.staged().take();").unwrap()
                < adapter
                    .find("root.ready_for_draw(&self.retirements)?")
                    .unwrap()
        );
        assert!(adapter.contains("ready.write_reframe(reframe);"));
        assert!(adapter.contains("ready.arm_and_draw(&self.retirements, pass);"));
        assert!(!adapter.contains("PollType::Wait"));

        let scene = include_str!("../scene.rs");
        assert!(scene.contains("resident_one_xs: Option<(ResidentCaptureFacade"));
        assert!(!scene.contains("installed_one_xs_draw"));
        let prepare = scene
            .split_once("pub fn prepare(")
            .unwrap()
            .1
            .split_once("pub fn diagnostic_one_xs_direct_frame")
            .unwrap()
            .0;
        assert!(prepare.contains("prepare_redraw_after_external_poll"));
        let draw = scene
            .split_once("pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>)")
            .unwrap()
            .1
            .split_once("fn is_bound")
            .unwrap()
            .0;
        assert!(draw.contains("attachment.arm_and_draw(pass)"));

        let retirement = include_str!("../draw_retirement.rs");
        let poll = retirement
            .split_once("pub(crate) fn poll(&mut self) -> Fallible<usize>")
            .unwrap()
            .1
            .split_once("let polled =")
            .unwrap()
            .0;
        assert!(poll.contains("if self.pending.is_empty()"));
        assert!(poll.contains("return Ok(0);"));

        let candidate = source
            .split_once("pub(crate) struct ResidentInstallCandidate")
            .unwrap()
            .1
            .split_once("pub(crate) struct InstalledOneXsReady")
            .unwrap()
            .0;
        assert!(candidate.find("draw:").unwrap() < candidate.find("root:").unwrap());
        assert!(candidate.find("root:").unwrap() < candidate.find("permit:").unwrap());

        let root = include_str!("one_xs/resident_frame_gpu.rs");
        assert!(!root.contains("ResidentReadyPlaceholder"));
        assert!(root.contains("ready: Option<Arc<InstalledOneXsDraw>>"));
        assert!(root.contains("state.committed = Some(Arc::clone(&self.successor));"));
        assert!(root.contains("state.ready = Some(Arc::clone(draw));"));
        assert!(root.contains("state.pending = None;"));
    }

    #[test]
    fn final_bridge_is_mode_neutral_sealed_and_pins_the_cold_directional_copy_law() {
        let bridge = include_str!("one_xs/pis_frontend_gpu/post_l1.rs");
        let geometry = include_str!("one_xs/geometry_gpu.rs");
        let materializer = include_str!("one_xs/map_patch_gpu.rs");
        let owner = include_str!("one_xs_belt_gpu.rs");

        assert!(bridge.contains("struct GpuFinalOperands<P: GpuPriorPublicLevelTwo>"));
        assert!(bridge.contains("GpuCompletedColdCheckpoint<ImportedOneXsPicture>"));
        assert!(bridge.contains("Fallible<GpuFinalOperands<GpuColdPriorPublicLevelTwo>>"));
        assert!(
            bridge.contains(
                "impl<P: GpuPriorPublicLevelTwo> resident::Sealed for GpuFinalOperands<P>"
            )
        );
        assert!(bridge.contains("&self.flight.frame"));
        assert!(bridge.contains("copy_buffer_to_buffer(&self.validity.buffer"));
        assert!(!bridge.contains("CompletedColdFinalOperands"));
        assert!(!bridge.contains("CompletedColdDrawCarrier"));

        assert!(geometry.contains("const PREIMAGE_WORDS: u64 = 40_000;"));
        assert!(geometry.contains("const SIDE_FLOW_WORDS: u64 = 129_600;"));
        assert!(geometry.contains("DynamicSide::LensA"));
        assert!(geometry.contains("DynamicSide::LensB"));
        assert!(geometry.contains("PREIMAGE_WORDS * WORD_BYTES"));
        assert!(geometry.contains("SIDE_FLOW_WORDS * WORD_BYTES"));
        assert!(geometry.contains("DynamicBufferCopy::new(public, 0)"));

        let production = materializer
            .split_once("pub(super) fn materialize_final<P>(")
            .unwrap()
            .1
            .split_once("#[cfg(test)]")
            .unwrap()
            .0;
        assert!(production.contains("GpuFinalOperands<P>"));
        assert!(!production.contains("admit_completed_cold_final"));
        assert!(materializer.contains("fn bind_for_install("));
        assert!(materializer.contains("Box<dyn InstalledFinalCarrier>"));
        let install_bind = materializer
            .split_once("fn bind_for_install(")
            .unwrap()
            .1
            .split_once("pub(super) struct GpuMapBinding")
            .unwrap()
            .0;
        assert!(install_bind.contains("-> Fallible<GpuBoundFinalMap>"));
        assert!(!install_bind.contains("map_err"));
        assert!(!install_bind.contains("BindingError"));
        assert!(owner.contains("fn prepare_resident_install<P>("));
        assert!(owner.contains("admit_completed_cold_final(checkpoint, &self.context)"));
        for forbidden in ["pub fn buffer", "pub fn packed", "pub fn input"] {
            assert!(!bridge.contains(forbidden));
            assert!(!geometry.contains(forbidden));
        }
    }

    #[test]
    fn resident_submission_and_early_drop_never_poll_or_release_owner() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}"
                );
                eprintln!("skipping ONE X2 resident ownership test: {why}");
                return;
            }
        };
        let fixture = qualification_fixture();
        let texture_a =
            qualification_texture(&device, &queue, "ONE X2 resident A", &fixture.sources.a);
        let texture_b =
            qualification_texture(&device, &queue, "ONE X2 resident B", &fixture.sources.b);
        let pipeline = GpuSolverBeltPipeline::new(OneXsGpuContext::new(&device, &queue))
            .unwrap_or_else(|error| panic!("GPU qualification failed on {adapter}: {error}"));
        let wait_state = Arc::new(AtomicU8::new(0));
        let (dropped, answer) = mpsc::channel();
        let mut resident = pipeline
            .submit_resident_retained(
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                &fixture.maps,
                DropProbe {
                    wait_state: Arc::clone(&wait_state),
                    dropped,
                },
                GpuPisFlight {
                    generation: 7,
                    frame: FrameStamp::for_test(11, Duration::from_secs(2), None),
                },
            )
            .unwrap();
        assert_eq!(
            wait_state.load(Ordering::SeqCst),
            0,
            "resident submission polled before returning"
        );
        assert!(
            matches!(answer.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "resident submission released its source owner before token drop"
        );
        resident.observe_completion(Arc::clone(&wait_state));
        drop(resident);
        assert_eq!(wait_state.load(Ordering::SeqCst), 0);
        assert!(
            matches!(answer.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "cancelled resident submission returned its uncertain source owner"
        );
    }

    #[test]
    fn submission_lease_abandon_never_waits_and_retains_source_owner() {
        let state = Arc::new(AtomicU8::new(0));
        let owner = Arc::new(());
        let retained = Arc::downgrade(&owner);
        let lease = SubmissionLease::injected(
            Arc::clone(&owner),
            InjectedWait::Success,
            Arc::clone(&state),
        );
        assert_eq!(state.load(Ordering::SeqCst), 0);
        drop(lease);
        assert_eq!(state.load(Ordering::SeqCst), 0);
        drop(owner);
        assert!(retained.upgrade().is_some());
    }

    #[test]
    fn submission_lease_success_disarms_drop_without_a_second_wait() {
        let state = Arc::new(AtomicU8::new(0));
        let (dropped, answer) = mpsc::channel();
        let mut lease = SubmissionLease::injected(
            DropProbe {
                wait_state: Arc::clone(&state),
                dropped,
            },
            InjectedWait::Success,
            Arc::clone(&state),
        );
        lease.complete().unwrap();
        assert_eq!(answer.recv().unwrap(), 2, "owner preceded successful wait");
        state.store(7, Ordering::SeqCst);
        drop(lease);
        assert_eq!(
            state.load(Ordering::SeqCst),
            7,
            "disarmed lease waited again during drop"
        );
    }

    #[test]
    fn submission_lease_poll_failure_retains_source_owner_for_process_lifetime() {
        let state = Arc::new(AtomicU8::new(0));
        let owner = Arc::new(());
        let retained = Arc::downgrade(&owner);
        let mut lease =
            SubmissionLease::injected(owner.clone(), InjectedWait::Error, Arc::clone(&state));
        let error = lease.complete().unwrap_err();
        assert_eq!(error.to_string(), "injected ONE X2 submission poll failure");
        assert_eq!(state.load(Ordering::SeqCst), 2);
        drop(lease);
        drop(owner);
        assert!(
            retained.upgrade().is_some(),
            "poll failure returned the source owner to possible decoder reuse"
        );
    }

    #[test]
    fn submission_lease_explicit_panic_retains_owner_and_preserves_panic() {
        let state = Arc::new(AtomicU8::new(0));
        let owner = Arc::new(());
        let retained = Arc::downgrade(&owner);
        let mut lease =
            SubmissionLease::injected(owner.clone(), InjectedWait::Panic, Arc::clone(&state));
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = lease.complete();
        }))
        .expect_err("injected native-backend panic was not preserved");
        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&"injected ONE X2 submission poll panic")
        );
        assert_eq!(state.load(Ordering::SeqCst), 1);
        drop(lease);
        drop(owner);
        assert!(
            retained.upgrade().is_some(),
            "poll panic returned the source owner to possible decoder reuse"
        );
    }

    #[test]
    fn submission_lease_drop_does_not_invoke_poll_panic_and_retains_owner() {
        let state = Arc::new(AtomicU8::new(0));
        let owner = Arc::new(());
        let retained = Arc::downgrade(&owner);
        let lease =
            SubmissionLease::injected(owner.clone(), InjectedWait::Panic, Arc::clone(&state));
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(lease)))
            .expect("lease Drop propagated a native-backend panic");
        assert_eq!(state.load(Ordering::SeqCst), 0);
        drop(owner);
        assert!(
            retained.upgrade().is_some(),
            "panicking Drop returned the source owner to possible decoder reuse"
        );
    }

    const DOUBLE_UNWIND_CHILD: &str = "KJERAG_TEST_BELT_LEASE_DOUBLE_UNWIND_CHILD";
    const DOUBLE_UNWIND_MARKER: &str = "ONE X2 submission lease double unwind passed";

    struct LeaseDropGuard(Option<SubmissionLease<Arc<()>>>);

    impl Drop for LeaseDropGuard {
        fn drop(&mut self) {
            // This runs while the outer panic is already unwinding. If the
            // lease ever lets its injected poll panic escape, Rust aborts this
            // process for the double panic. The controller test deliberately
            // confines that failure to a child test process.
            drop(self.0.take());
        }
    }

    #[test]
    fn submission_lease_double_unwind_child() {
        if std::env::var_os(DOUBLE_UNWIND_CHILD).is_none() {
            return;
        }

        let state = Arc::new(AtomicU8::new(0));
        let owner = Arc::new(());
        let retained = Arc::downgrade(&owner);
        let lease =
            SubmissionLease::injected(owner.clone(), InjectedWait::Panic, Arc::clone(&state));
        let outer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = LeaseDropGuard(Some(lease));
            panic!("injected ONE X2 outer unwind");
        }))
        .expect_err("the outer panic did not reach its catch boundary");
        assert_eq!(
            outer.downcast_ref::<&str>(),
            Some(&"injected ONE X2 outer unwind"),
            "the lease replaced the active outer panic payload"
        );
        assert_eq!(
            state.load(Ordering::SeqCst),
            0,
            "lease Drop polled while an outer panic was unwinding"
        );
        drop(owner);
        assert!(
            retained.upgrade().is_some(),
            "double-unwind quarantine released the source owner"
        );
        eprintln!("{DOUBLE_UNWIND_MARKER}");
    }

    #[test]
    fn submission_lease_drop_during_outer_unwind_is_process_safe() {
        let module = module_path!();
        let module = module
            .split_once("::")
            .map_or(module, |(_, test_path)| test_path);
        let helper = format!("{module}::submission_lease_double_unwind_child");
        let output = std::process::Command::new(
            std::env::current_exe().expect("the test harness has an executable path"),
        )
        .args(["--exact", &helper, "--nocapture"])
        .env(DOUBLE_UNWIND_CHILD, "1")
        .output()
        .expect("could not start the isolated double-unwind helper");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "isolated double-unwind helper failed with {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status
        );
        assert!(
            stdout.contains(DOUBLE_UNWIND_MARKER) || stderr.contains(DOUBLE_UNWIND_MARKER),
            "isolated helper did not prove outer-payload and quarantine checks\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }

    #[test]
    fn gpu_solver_belts_are_byte_exact_on_adversarial_odd_padded_sources() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}"
                );
                eprintln!("skipping exact ONE X2 GPU solver-belt twin: {why}");
                return;
            }
        };

        let fixture = qualification_fixture();
        let texture_a =
            qualification_texture(&device, &queue, "ONE X2 odd padded A", &fixture.sources.a);
        let texture_b =
            qualification_texture(&device, &queue, "ONE X2 odd padded B", &fixture.sources.b);
        let pipeline = GpuSolverBeltPipeline::new(OneXsGpuContext::new(&device, &queue))
            .unwrap_or_else(|error| panic!("GPU qualification failed on {adapter}: {error}"));
        let pending = pipeline
            .submit_inner(
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                &fixture.maps,
                (),
                SubmissionInput::Sampled {
                    qualify_intermediates: true,
                },
                true,
            )
            .unwrap();
        assert_eq!(pending.packed().size(), OUTPUT_BYTES);
        let (actual_blurred, actual_preblur, retained_bits) = pending.read_qualification().unwrap();
        assert_eq!(
            actual_preblur.pixel(Lens::A, 10, 10),
            190,
            "GPU changed the source-FMA discriminator on {adapter}"
        );
        assert_eq!(
            actual_preblur.bytes(),
            fixture.expected_preblur.bytes(),
            "GPU pre-blur solver belts differ from the scalar/native schedule on {adapter}"
        );
        assert_eq!(retained_bits, RETAINED_FMA_BITS);
        assert_eq!(
            actual_blurred.bytes(),
            fixture.expected_blurred.bytes(),
            "GPU blurred solver belts differ from the CPU/native schedule on {adapter}"
        );
    }

    #[test]
    fn gpu_gaussian_matches_complete_direct_retained_fixture() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}"
                );
                eprintln!("skipping direct ONE X2 GPU Gaussian twin: {why}");
                return;
            }
        };
        let fixture = qualification_fixture();
        let texture_a =
            qualification_texture(&device, &queue, "ONE X2 blur fixture A", &fixture.sources.a);
        let texture_b =
            qualification_texture(&device, &queue, "ONE X2 blur fixture B", &fixture.sources.b);
        let pipeline = GpuSolverBeltPipeline::new(OneXsGpuContext::new(&device, &queue))
            .unwrap_or_else(|error| panic!("GPU qualification failed on {adapter}: {error}"));
        let input = blur_qualification_fixture();
        assert_eq!(input.pixel(Lens::A, 0, 0), 255, "corner impulse");
        assert_eq!(
            input.pixel(Lens::A, 0, super::super::COLS / 2),
            173,
            "edge impulse"
        );
        assert_eq!(
            input.pixel(Lens::A, super::super::ROWS / 2, super::super::COLS / 2,),
            255,
            "centre impulse"
        );
        assert_eq!(
            input.pixel(Lens::A, super::super::ROWS - 1, super::super::COLS - 1,),
            11,
            "lens A storage boundary"
        );
        assert_eq!(input.pixel(Lens::B, 0, 0), 241, "lens B storage boundary");
        let actual = pipeline
            .submit_inner(
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                &fixture.maps,
                (),
                SubmissionInput::Preblurred(&input),
                true,
            )
            .unwrap()
            .read()
            .unwrap();
        assert_eq!(
            actual.bytes(),
            gaussian_blur(&input).bytes(),
            "GPU Gaussian differs from the complete CPU/native fixture on {adapter}"
        );
    }

    #[test]
    fn runtime_qualification_refuses_changed_solver_arithmetic() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}"
                );
                eprintln!("skipping ONE X2 GPU qualification refusal test: {why}");
                return;
            }
        };
        let broken = SHADER.replacen("return (sum + 4u) / 9u;", "return 0u;", 1);
        assert_ne!(
            broken, SHADER,
            "the solver mutation did not find its target"
        );
        let error = match GpuSolverBeltPipeline::from_shader(
            OneXsGpuContext::new(&device, &queue),
            &broken,
        ) {
            Ok(_) => panic!("changed ONE X2 GPU arithmetic was accepted on {adapter}"),
            Err(error) => error,
        };
        assert!(
            matches!(
                error.downcast_ref::<GpuQualificationError>(),
                Some(GpuQualificationError::SolverByte { .. })
            ),
            "changed arithmetic returned the wrong failure on {adapter}: {error}"
        );
    }

    #[test]
    fn runtime_qualification_uses_production_entry_for_retained_fma() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}"
                );
                eprintln!("skipping ONE X2 production-entry qualification test: {why}");
                return;
            }
        };
        if let Err(error) = GpuSolverBeltPipeline::new(OneXsGpuContext::new(&device, &queue)) {
            assert!(
                std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                "KJERAG_REQUIRE_GPU is set and {adapter} fails the baseline ONE X2 GPU qualification: {error}"
            );
            eprintln!(
                "skipping production-entry mutation on an adapter that fails baseline qualification: {adapter}: {error}"
            );
            return;
        }
        let broken = SHADER.replacen(
            "witness_words[0] = bitcast<u32>(uv.x);",
            "witness_words[0] = bitcast<u32>(uv.x) + 1u;",
            1,
        );
        assert_ne!(
            broken, SHADER,
            "the production discriminator mutation did not find its target"
        );
        let error = match GpuSolverBeltPipeline::from_shader(
            OneXsGpuContext::new(&device, &queue),
            &broken,
        ) {
            Ok(_) => panic!("changed ONE X2 production discriminator was accepted on {adapter}"),
            Err(error) => error,
        };
        assert!(
            matches!(
                error.downcast_ref::<GpuQualificationError>(),
                Some(GpuQualificationError::RetainedMap { .. })
            ),
            "changed production discriminator returned the wrong failure on {adapter}: {error}"
        );
    }

    #[test]
    fn runtime_qualification_refuses_changed_gaussian_rounding() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}"
                );
                eprintln!("skipping ONE X2 Gaussian qualification test: {why}");
                return;
            }
        };
        if let Err(error) = GpuSolverBeltPipeline::new(OneXsGpuContext::new(&device, &queue)) {
            assert!(
                std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                "KJERAG_REQUIRE_GPU is set and {adapter} fails the baseline ONE X2 GPU qualification: {error}"
            );
            eprintln!(
                "skipping Gaussian mutation on an adapter that fails baseline qualification: {adapter}: {error}"
            );
            return;
        }
        let broken = SHADER.replacen("return (sum + 8192u) >> 14u;", "return sum >> 14u;", 1);
        assert_ne!(broken, SHADER, "the Gaussian mutation found no target");
        let error = match GpuSolverBeltPipeline::from_shader(
            OneXsGpuContext::new(&device, &queue),
            &broken,
        ) {
            Ok(_) => panic!("changed ONE X2 Gaussian was accepted on {adapter}"),
            Err(error) => error,
        };
        assert!(
            matches!(
                error.downcast_ref::<GpuQualificationError>(),
                Some(GpuQualificationError::BlurredByte { .. })
            ),
            "changed Gaussian returned the wrong failure on {adapter}: {error}"
        );
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            match future.as_mut().poll(&mut context) {
                std::task::Poll::Ready(answer) => return answer,
                std::task::Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn gpu() -> Result<(wgpu::Device, wgpu::Queue, String), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
            .ok_or("no Vulkan adapter")?;
        let name = adapter.get_info().name;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("exact ONE X2 GPU solver-belt twin"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        Ok((device, queue, name))
    }

    fn gpu_pair() -> Result<(wgpu::Device, wgpu::Queue, wgpu::Device, wgpu::Queue, String), String>
    {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
            .ok_or("no Vulkan adapter")?;
        let name = adapter.get_info().name;
        let request = || {
            block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("exact ONE X2 resident facade provenance"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            }))
            .map_err(|error| error.to_string())
        };
        let (device, queue) = request()?;
        let (foreign_device, foreign_queue) = request()?;
        Ok((device, queue, foreign_device, foreign_queue, name))
    }
}
