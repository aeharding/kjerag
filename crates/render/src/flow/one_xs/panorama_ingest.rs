//! Sequential resident source ingestion for offline body-panorama consumers.
//!
//! This owner has its own stitch/map/colour history and never publishes a raw
//! resident display future. It shares the established worker implementation,
//! source import, exact map binding and bounded draw-retirement mechanism.

use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use kjerag_media::{FrameStamp, Frames};
use kjerag_meta::OrientationTrack;

use super::resident_worker::PanoramaJob;
use super::{
    ImportedOneXsSource, InstalledOneXsReady, ResidentCameraProfile, ResidentCaptureSession,
    ResidentReadyMap, prepare_resident_bound,
};
use crate::direct_type2::{BodyPanorama, CompactNv12Panorama};
use crate::draw_retirement::DrawRetirementError;
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
use crate::{Fallible, Reframe, Size};

struct State {
    pending: Option<FrameStamp>,
    previous: Option<FrameStamp>,
    ready: Option<BodyPanorama>,
    failure: Option<String>,
}

pub(super) struct ResidentPanoramaIngestInner {
    pub(super) session: Arc<ResidentCaptureSession>,
    state: Mutex<State>,
}

/// One linear offline producer of source-stamped gamma-RGB body panoramas.
#[allow(dead_code)]
pub(crate) struct ResidentPanoramaIngest {
    inner: Arc<ResidentPanoramaIngestInner>,
    size: Size,
}

#[allow(dead_code)]
impl ResidentPanoramaIngest {
    pub(crate) fn new(
        context: OneXsGpuContext,
        profile: Arc<ResidentCameraProfile>,
        orientation: OrientationTrack,
        size: Size,
    ) -> Fallible<Self> {
        validate_panorama_size(size)?;
        // The ordinary surface pipeline is constructed with the session but
        // this owner only executes BodyPanorama's fixed Rgba8Unorm pass.
        let session = Arc::new(ResidentCaptureSession::new(
            context,
            wgpu::TextureFormat::Rgba8Unorm,
            &profile,
            orientation,
        )?);
        Ok(Self {
            inner: Arc::new(ResidentPanoramaIngestInner {
                session,
                state: Mutex::new(State {
                    pending: None,
                    previous: None,
                    ready: None,
                    failure: None,
                }),
            }),
            size,
        })
    }

    /// Admit exactly one decoded source and its already-prepared projection.
    /// `false` is bounded backpressure; it never advances processing history.
    pub(crate) fn try_submit(&self, frames: Arc<Frames>, reframe: Reframe) -> Fallible<bool> {
        validate_reframe(&frames, &reframe)?;
        let stamp = frames.stamp();

        if let Err(error) = self.inner.session.retirements.poll() {
            self.inner.fail(&error.to_string());
            return Err(error);
        }
        let permit = match self.inner.session.retirements.reserve() {
            Ok(permit) => permit,
            Err(DrawRetirementError::Full) => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        {
            let mut state = self.inner.state()?;
            if let Some(error) = &state.failure {
                return Err(error.clone().into());
            }
            if state.pending.is_some() || state.ready.is_some() {
                return Ok(false);
            }
            validate_ingest_sequence(state.previous.as_ref(), &stamp)?;
            state.pending = Some(stamp.clone());
        }

        let job = Box::new(PanoramaJob {
            ingest: Arc::clone(&self.inner),
            frames,
            reframe,
            stamp: stamp.clone(),
            size: self.size,
            permit,
        });
        match self.inner.session.worker.try_kick_panorama(job) {
            Ok(()) => Ok(true),
            Err(mpsc::TrySendError::Full(job)) => {
                self.inner.cancel(&job.stamp)?;
                Ok(false)
            }
            Err(mpsc::TrySendError::Disconnected(job)) => {
                let message = "ONE X2 stitch worker stopped before accepting panorama ingestion";
                job.ingest.fail(message);
                Err(message.into())
            }
        }
    }

    /// Collect completed source lifetimes and take the one finished panorama.
    pub(crate) fn poll(&self) -> Fallible<Option<BodyPanorama>> {
        if let Err(error) = self.inner.session.retirements.poll() {
            self.inner.fail(&error.to_string());
            return Err(error);
        }
        let mut state = self.inner.state()?;
        if let Some(error) = &state.failure {
            return Err(error.clone().into());
        }
        Ok(state.ready.take())
    }

    pub(crate) fn is_drained(&self) -> Fallible<bool> {
        let state = self.inner.state()?;
        if let Some(error) = &state.failure {
            return Err(error.clone().into());
        }
        Ok(state.pending.is_none()
            && state.ready.is_none()
            && self.inner.session.retirements.is_empty())
    }

    #[cfg(test)]
    pub(crate) fn root_slots_empty_for_test(&self) -> Fallible<bool> {
        Ok(!self.inner.session.capture.pipeline.root.has_future()?
            && self
                .inner
                .session
                .capture
                .pipeline
                .root
                .diagnostic_ready()?
                .is_none())
    }
}

pub(super) fn service_panorama(job: Box<PanoramaJob>) -> Fallible<()> {
    job.ingest.ensure_pending(&job.stamp)?;
    let session = Arc::clone(&job.ingest.session);
    let stamp = job.stamp;
    let output = prepare_panorama(
        &session,
        job.frames,
        job.reframe,
        &stamp,
        job.size,
        job.permit,
    )?;
    job.ingest.complete(stamp, output)
}

/// Prepare one exact decoded source through the established resident
/// stitch/map/colour transaction and materialize its body panorama. The
/// caller owns admission and publication; this helper submits no raw future
/// or display-ready map.
pub(super) fn prepare_panorama(
    session: &Arc<ResidentCaptureSession>,
    frames: Arc<Frames>,
    reframe: Reframe,
    stamp: &FrameStamp,
    size: Size,
    permit: crate::draw_retirement::DrawPermit,
) -> Fallible<BodyPanorama> {
    let draw = prepare_panorama_draw(session, frames, reframe, stamp, permit)?;
    let mut encoder =
        session
            .context
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("resident panorama ingestion"),
            });
    let output = draw.arm_and_encode_panorama(
        &session.retirements,
        session.context.device(),
        &mut encoder,
        size,
    )?;
    session.context.queue().submit(Some(encoder.finish()));
    Ok(output)
}

/// Prepare the same exact resident source/map transaction as
/// [`prepare_panorama`], but materialize its compact YUV representation.
pub(super) fn prepare_compact_panorama(
    session: &Arc<ResidentCaptureSession>,
    frames: Arc<Frames>,
    reframe: Reframe,
    stamp: &FrameStamp,
    size: Size,
    permit: crate::draw_retirement::DrawPermit,
) -> Fallible<CompactNv12Panorama> {
    let draw = prepare_panorama_draw(session, frames, reframe, stamp, permit)?;
    let mut encoder =
        session
            .context
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("resident compact panorama ingestion"),
            });
    let output = draw.arm_and_encode_compact_panorama(
        &session.retirements,
        session.context.device(),
        &mut encoder,
        size,
    )?;
    session.context.queue().submit(Some(encoder.finish()));
    Ok(output)
}

fn prepare_panorama_draw(
    session: &Arc<ResidentCaptureSession>,
    frames: Arc<Frames>,
    reframe: Reframe,
    stamp: &FrameStamp,
    permit: crate::draw_retirement::DrawPermit,
) -> Fallible<InstalledOneXsReady> {
    let started = Instant::now();
    let source = retry_source_import(
        || session.capture.import_picture(Arc::clone(&frames)),
        || started.elapsed(),
        std::thread::sleep,
    )?;
    if source.resident_frame() != *stamp {
        return Err("ONE X2 panorama import differs from its decoded source delivery".into());
    }
    let ready = super::resident_worker::finish_pending(session, session.submit(source)?)?;
    if ready.frame() != stamp {
        return Err("ONE X2 panorama map differs from its decoded source delivery".into());
    }
    let bound = match ready {
        ResidentReadyMap::Cold(map) => prepare_resident_bound(*map, Arc::clone(&session.direct)),
        ResidentReadyMap::Warm(map) => prepare_resident_bound(*map, Arc::clone(&session.direct)),
    }?;
    let mut draw = bound.commit_processing(permit)?;
    draw.write_reframe(&reframe);
    Ok(draw)
}

/// Retry only resource exhaustion before a source enters the stitch transaction.
/// The exact decoded pair and retirement permit stay on the bounded stitch
/// worker. No history, source admission or UI thread advances during a retry.
/// Other errors, and resource exhaustion lasting the existing import bound,
/// propagate their original error to the capture's one-shot failure handoff.
fn retry_source_import<T>(
    mut import: impl FnMut() -> Fallible<T>,
    mut elapsed: impl FnMut() -> Duration,
    mut wait: impl FnMut(Duration),
) -> Fallible<T> {
    let mut failed_at = None;
    loop {
        let error = match import() {
            Ok(source) => return Ok(source),
            Err(error) => error,
        };
        if !crate::dmabuf::retryable_import_error(error.as_ref()) {
            return Err(error);
        }
        let now = elapsed();
        let since = *failed_at.get_or_insert(now);
        if now.saturating_sub(since) >= crate::STUCK_FOR {
            return Err(error);
        }
        // This is a worker-side scarcity backoff, not source-frame skipping
        // or a GPU completion fence. Healthy imports never sleep.
        wait(Duration::from_millis(1));
        if elapsed().saturating_sub(since) >= crate::STUCK_FOR {
            return Err(error);
        }
    }
}

impl ResidentPanoramaIngestInner {
    fn state(&self) -> Fallible<std::sync::MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| "ONE X2 panorama ingestion state is poisoned".into())
    }

    fn cancel(&self, stamp: &FrameStamp) -> Fallible<()> {
        let mut state = self.state()?;
        if state.pending.as_ref() != Some(stamp) {
            return Err("ONE X2 panorama ingestion backpressure changed its pending source".into());
        }
        state.pending = None;
        Ok(())
    }

    pub(super) fn ensure_pending(&self, stamp: &FrameStamp) -> Fallible<()> {
        if self.state()?.pending.as_ref() == Some(stamp) {
            Ok(())
        } else {
            Err("ONE X2 panorama worker source differs from its pending request".into())
        }
    }

    pub(super) fn complete(&self, stamp: FrameStamp, output: BodyPanorama) -> Fallible<()> {
        if output.frame() != &stamp {
            return Err("ONE X2 panorama output differs from its source delivery".into());
        }
        let mut state = self.state()?;
        if state.pending.as_ref() != Some(&stamp) || state.ready.is_some() {
            return Err("ONE X2 panorama completion differs from its pending request".into());
        }
        state.pending = None;
        state.previous = Some(stamp);
        state.ready = Some(output);
        Ok(())
    }

    pub(super) fn fail(&self, message: &str) {
        match self.state.lock() {
            Ok(mut state) => {
                state.pending = None;
                state.ready = None;
                state.failure = Some(message.to_owned());
            }
            Err(poison) => {
                let mut state = poison.into_inner();
                state.pending = None;
                state.ready = None;
                state.failure = Some(message.to_owned());
            }
        }
        eprintln!("{message}");
    }
}

fn validate_panorama_size(size: Size) -> Fallible<()> {
    if size.width == 0 || size.height == 0 || size.width != size.height.saturating_mul(2) {
        return Err(format!(
            "body panorama must be nonzero 2:1, got {} by {}",
            size.width, size.height
        )
        .into());
    }
    Ok(())
}

pub(super) fn validate_reframe(frames: &Frames, reframe: &Reframe) -> Fallible<()> {
    let expected = [frames.size.width as f32, frames.size.height as f32];
    if reframe.frame_size() != expected {
        let actual = reframe.frame_size();
        return Err(format!(
            "ONE X2 panorama reframe names {} by {} source pixels but the delivered frame is {} by {}",
            actual[0], actual[1], frames.size.width, frames.size.height
        )
        .into());
    }
    if reframe.linearizes_output() {
        return Err("resident body panorama requires gamma-encoded source output".into());
    }
    Ok(())
}

fn validate_ingest_sequence(previous: Option<&FrameStamp>, offered: &FrameStamp) -> Fallible<()> {
    if let Some(previous) = previous {
        super::validate_resident_sequence(Some(previous), offered)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn source_import_recovers_without_replacing_its_source() {
        let source = Arc::new(612);
        let mut attempts = 0;
        let now = Cell::new(Duration::ZERO);
        let imported = retry_source_import(
            || {
                attempts += 1;
                if attempts <= 400 {
                    Err(std::io::Error::from_raw_os_error(libc::EMFILE).into())
                } else {
                    Ok(Arc::clone(&source))
                }
            },
            || now.get(),
            |delay| now.set(now.get() + delay),
        )
        .unwrap();
        assert_eq!(attempts, 401);
        assert_eq!(now.get(), Duration::from_millis(400));
        assert!(Arc::ptr_eq(&source, &imported));
    }

    #[test]
    fn source_import_exhaustion_returns_the_underlying_error_at_the_bound() {
        let now = Cell::new(Duration::ZERO);
        let error = retry_source_import::<()>(
            || Err(std::io::Error::from_raw_os_error(libc::EMFILE).into()),
            || now.get(),
            |delay| now.set(now.get() + delay),
        )
        .unwrap_err();
        assert_eq!(now.get(), crate::STUCK_FOR);
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .raw_os_error(),
            Some(libc::EMFILE)
        );
    }

    #[test]
    fn source_import_does_not_attempt_again_after_an_overslept_deadline() {
        let now = Cell::new(Duration::ZERO);
        let mut attempts = 0;
        let error = retry_source_import(
            || {
                attempts += 1;
                if attempts == 1 {
                    Err(std::io::Error::from_raw_os_error(libc::EMFILE).into())
                } else {
                    Ok(())
                }
            },
            || now.get(),
            |_| now.set(crate::STUCK_FOR),
        )
        .unwrap_err();
        assert_eq!(attempts, 1);
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .raw_os_error(),
            Some(libc::EMFILE)
        );
    }

    #[test]
    fn source_import_never_retries_deterministic_errors_or_waits_on_success() {
        let error = retry_source_import::<()>(
            || Err("invalid source descriptor".into()),
            || panic!("deterministic errors have no retry clock"),
            |_| panic!("deterministic errors must not wait"),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "invalid source descriptor");
        assert_eq!(
            retry_source_import(
                || Ok(42),
                || panic!("healthy imports have no retry clock"),
                |_| panic!("healthy imports must not wait"),
            )
            .unwrap(),
            42
        );
    }

    #[test]
    fn ingest_may_begin_at_a_seek_landing_but_then_requires_exact_continuity() {
        let landing = FrameStamp::for_test(612, Duration::from_secs(20), None);
        let next = FrameStamp::for_test(613, Duration::from_secs(21), Some(&landing));
        let gap = FrameStamp::for_test(615, Duration::from_secs(22), Some(&landing));
        let restarted = FrameStamp::for_test(614, Duration::from_secs(22), None);

        validate_ingest_sequence(None, &landing).unwrap();
        validate_ingest_sequence(Some(&landing), &next).unwrap();
        assert!(validate_ingest_sequence(Some(&next), &gap).is_err());
        assert!(validate_ingest_sequence(Some(&next), &restarted).is_err());
    }
}
