//! Sequential resident source ingestion for offline body-panorama consumers.
//!
//! This owner has its own stitch/map/colour history and never publishes a raw
//! resident display future. It shares the established worker implementation,
//! source import, exact map binding and bounded draw-retirement mechanism.

use std::sync::{Arc, Mutex, mpsc};

use kjerag_media::{FrameStamp, Frames};
use kjerag_meta::OrientationTrack;

use super::resident_worker::PanoramaJob;
use super::{
    ImportedOneXsSource, ResidentCameraProfile, ResidentCaptureSession, ResidentReadyMap,
    prepare_resident_bound,
};
use crate::direct_type2::BodyPanorama;
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
    let source = session.capture.import_picture(job.frames)?;
    if source.resident_frame() != job.stamp {
        return Err("ONE X2 panorama import differs from its decoded source delivery".into());
    }
    let ready = super::resident_worker::finish_pending(&session, session.submit(source)?)?;
    if ready.frame() != &job.stamp {
        return Err("ONE X2 panorama map differs from its decoded source delivery".into());
    }
    let bound = match ready {
        ResidentReadyMap::Cold(map) => prepare_resident_bound(*map, Arc::clone(&session.direct)),
        ResidentReadyMap::Warm(map) => prepare_resident_bound(*map, Arc::clone(&session.direct)),
    }?;
    let mut draw = bound.commit_processing(job.permit)?;
    draw.write_reframe(&job.reframe);
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
        job.size,
    )?;
    session.context.queue().submit(Some(encoder.finish()));
    job.ingest.complete(job.stamp, output)
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

fn validate_reframe(frames: &Frames, reframe: &Reframe) -> Fallible<()> {
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
    use std::time::Duration;

    use super::*;

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
