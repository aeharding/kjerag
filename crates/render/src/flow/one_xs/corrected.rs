//! Exact source ownership for direct views with a reduced temporal correction.
//!
//! Only the resident install transaction can create an input. Original lens
//! planes are GPU-owned copies, while immutable map/colour bindings detach
//! from the heavier processing carrier. Temporal output moves its matching
//! original source into the ready frame, never retaining decoder leases in
//! the seven-source temporal window.

use std::collections::VecDeque;
use std::sync::Arc;

use super::map_patch_gpu::MapSnapshot;
use super::native_capacity;
use crate::direct_type2::correction::{CorrectionPictureBinding, CorrectionPipeline};
use crate::direct_type2::{BodyPanorama, DirectType2Pipeline, SourceSnapshot};
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
use crate::temporal_fusion::color::MatrixCoefficients;
use crate::temporal_fusion::correction_stream::{CorrectionFrame, CorrectionStream};
use crate::temporal_fusion::settings::Provider;
use crate::{Fallible, FrameStamp, Reframe};

struct DisplaySource {
    source: SourceSnapshot,
    map: MapSnapshot,
    pipeline: Arc<DirectType2Pipeline>,
    context: OneXsGpuContext,
}

pub(super) struct CorrectionInput {
    body: BodyPanorama,
    display: DisplaySource,
}

impl CorrectionInput {
    pub(super) fn new(
        body: BodyPanorama,
        source: SourceSnapshot,
        map: MapSnapshot,
        pipeline: Arc<DirectType2Pipeline>,
        context: &OneXsGpuContext,
    ) -> Fallible<Self> {
        source.ensure_context(context)?;
        map.ensure_context(context)?;
        if !body.belongs_to(context.device()) {
            return Err("temporal correction body belongs to a different graphics device".into());
        }
        if source.frame() != body.frame() || map.frame() != body.frame() {
            return Err("temporal correction source, map and body name different frames".into());
        }
        Ok(Self {
            body,
            display: DisplaySource {
                source,
                map,
                pipeline,
                context: context.clone(),
            },
        })
    }

    pub(super) fn frame(&self) -> &FrameStamp {
        self.body.frame()
    }
}

/// Worker-private source association around the shared correction algorithm.
pub(super) struct CorrectionSequence {
    stream: CorrectionStream,
    pending: VecDeque<DisplaySource>,
}

impl CorrectionSequence {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        field_size: [u32; 2],
        provider: Provider,
    ) -> Fallible<Self> {
        Ok(Self {
            stream: CorrectionStream::new(device, queue, field_size, provider)?,
            pending: VecDeque::with_capacity(7),
        })
    }

    pub(super) fn push(&mut self, input: CorrectionInput) -> Fallible<Vec<CorrectedFrame>> {
        if self.pending.len() >= 7 {
            return Err("temporal correction retained more than seven original pictures".into());
        }
        let CorrectionInput { body, display } = input;
        let matrix = MatrixCoefficients::from_source_rgb(display.source.source_matrix());
        self.pending.push_back(display);
        let outputs = self.stream.push(body, matrix)?;
        self.pair(outputs)
    }

    pub(super) fn finish(&mut self) -> Fallible<Vec<CorrectedFrame>> {
        let outputs = self.stream.finish()?;
        let outputs = self.pair(outputs)?;
        // The reference Stream emits nothing for a window shorter than seven
        // real sources. Match that boundary without inventing padded inputs.
        self.pending.clear();
        Ok(outputs)
    }

    fn pair(&mut self, outputs: Vec<CorrectionFrame>) -> Fallible<Vec<CorrectedFrame>> {
        let mut paired = Vec::with_capacity(outputs.len());
        for correction in outputs {
            let display = self
                .pending
                .pop_front()
                .ok_or("temporal correction has no matching original picture")?;
            if correction.frame() != display.source.frame()
                || correction.frame() != display.map.frame()
            {
                return Err("temporal correction differs from its original source and map".into());
            }
            if !correction.belongs_to(display.context.device()) {
                return Err("temporal correction belongs to a different graphics device".into());
            }
            paired.push(CorrectedFrame {
                display,
                correction,
            });
        }
        Ok(paired)
    }
}

/// Completed temporal correction and its exact original source/map/colour.
pub(crate) struct CorrectedFrame {
    display: DisplaySource,
    correction: CorrectionFrame,
}

impl CorrectedFrame {
    pub(crate) fn frame(&self) -> &FrameStamp {
        self.correction.frame()
    }

    pub(crate) fn prepare_view(
        &self,
        device: &wgpu::Device,
        reframe: &Reframe,
        format: wgpu::TextureFormat,
    ) -> Fallible<PreparedCorrectionDraw> {
        if self.display.context.device() != device {
            return Err("corrected view belongs to a different graphics device".into());
        }
        let pipeline = self.display.pipeline.correction_pipeline(device, format)?;
        let picture =
            self.display
                .source
                .prepare_correction_picture(&pipeline, reframe, &self.correction)?;
        Ok(PreparedCorrectionDraw {
            picture,
            map: self.display.map.read().clone(),
            fusion: self.display.map.fusion_read().cloned(),
            pipeline,
            frame: self.frame().clone(),
            native_capacity: native_capacity::DrawMarker::for_reframe(reframe),
        })
    }
}

/// Immutable bindings own all sampled GPU allocations. No decoder carrier
/// or reusable uniform is needed while the shell submits this view pass.
pub(crate) struct PreparedCorrectionDraw {
    picture: CorrectionPictureBinding,
    map: wgpu::BindGroup,
    fusion: Option<wgpu::BindGroup>,
    pipeline: Arc<CorrectionPipeline>,
    frame: FrameStamp,
    native_capacity: Option<native_capacity::DrawMarker>,
}

impl PreparedCorrectionDraw {
    pub(crate) fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.pipeline
            .draw(pass, &self.picture, &self.map, self.fusion.as_ref());
        if let Some(marker) = self.native_capacity {
            marker.record(&self.frame, pass);
        }
    }
}
