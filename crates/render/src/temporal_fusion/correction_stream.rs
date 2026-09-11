//! Half-resolution temporal correction ownership, without a display policy.
//!
//! This candidate keeps the existing seven-source cadence and motion/fusion
//! laws over a six-level half-linear field. Each output carries the exact
//! unfiltered RGB/NV12/RGB control for its filtered source. It does not own a
//! high-resolution source snapshot, interpolate fields, or select playback.

#![allow(
    dead_code,
    reason = "unselected correction candidate consumed by the pending live experiment"
)]

use std::collections::VecDeque;

use crate::direct_type2::BodyPanorama;
use crate::{Fallible, FrameStamp};

use super::color::{GpuColorConversion, MatrixCoefficients};
use super::settings::Provider;
use super::stream::{FilteredPanorama, Stream};

const SOURCES: usize = 7;

struct Control {
    frame: FrameStamp,
    texture: wgpu::Texture,
}

/// One source-exact low-resolution control and filtered correction field.
pub(crate) struct CorrectionFrame {
    frame: FrameStamp,
    current: wgpu::Texture,
    filtered: FilteredPanorama,
    device: wgpu::Device,
}

impl CorrectionFrame {
    pub(crate) fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub(crate) fn belongs_to(&self, device: &wgpu::Device) -> bool {
        self.device == *device && self.filtered.belongs_to(device)
    }

    pub(crate) fn current_texture(&self) -> &wgpu::Texture {
        &self.current
    }

    pub(crate) fn filtered_texture(&self) -> &wgpu::Texture {
        self.filtered.texture()
    }
}

/// Sequential owner for the explicit half-resolution correction candidate.
pub(crate) struct CorrectionStream {
    device: wgpu::Device,
    queue: wgpu::Queue,
    size: [u32; 2],
    color: GpuColorConversion,
    stream: Stream,
    pending: VecDeque<Control>,
    finished: bool,
    failure: Option<String>,
}

impl CorrectionStream {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        half_size: [u32; 2],
        provider: Provider,
    ) -> Fallible<Self> {
        let stream = Stream::new_half_resolution_correction(device, queue, half_size, provider)?;
        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
            size: half_size,
            color: GpuColorConversion::new(device),
            stream,
            pending: VecDeque::with_capacity(SOURCES),
            finished: false,
            failure: None,
        })
    }

    /// Admit one exact gamma-RGB body panorama. The independently encoded
    /// control takes the same RGB/NV12/RGB path as the Stream's history input.
    pub(crate) fn push(
        &mut self,
        body: BodyPanorama,
        matrix: MatrixCoefficients,
    ) -> Fallible<Vec<CorrectionFrame>> {
        let result = self.push_inner(body, matrix);
        self.remember_failure(result)
    }

    pub(crate) fn finish(&mut self) -> Fallible<Vec<CorrectionFrame>> {
        let result = self.finish_inner();
        self.remember_failure(result)
    }

    fn push_inner(
        &mut self,
        body: BodyPanorama,
        matrix: MatrixCoefficients,
    ) -> Fallible<Vec<CorrectionFrame>> {
        self.ensure_active()?;
        self.validate_body(&body)?;
        let frame = body.frame().clone();
        if let Some(previous) = self.pending.back() {
            if !previous.frame.same_decode_epoch(&frame)
                || previous.frame.index().checked_add(1) != Some(frame.index())
            {
                return Err(
                    "correction stream controls must be contiguous within one decode epoch".into(),
                );
            }
        }
        if self.pending.len() >= SOURCES {
            return Err("correction stream exceeded its seven-source control bound".into());
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("half-resolution temporal correction control"),
            });
        let nv12 = self
            .color
            .encode_rgb_to_nv12(&mut encoder, body.texture(), matrix)?;
        let current = self.color.encode_nv12_to_rgb(&mut encoder, &nv12, matrix)?;
        self.queue.submit([encoder.finish()]);
        self.pending.push_back(Control {
            frame: frame.clone(),
            texture: current,
        });

        let filtered = match self.stream.push_rgb(body, matrix) {
            Ok(filtered) => filtered,
            Err(error) => {
                let removed = self
                    .pending
                    .pop_back()
                    .expect("correction control was retained before Stream ingestion");
                debug_assert_eq!(removed.frame, frame);
                return Err(error);
            }
        };
        self.pair(filtered)
    }

    fn finish_inner(&mut self) -> Fallible<Vec<CorrectionFrame>> {
        self.ensure_healthy()?;
        if self.finished {
            return Ok(Vec::new());
        }
        let outputs = self.stream.finish()?;
        let paired = self.pair(outputs)?;
        // Fewer than seven inputs produce no temporal output. Their controls
        // have no false output owner and are discarded at the same boundary.
        self.pending.clear();
        self.finished = true;
        Ok(paired)
    }

    fn pair(&mut self, outputs: Vec<FilteredPanorama>) -> Fallible<Vec<CorrectionFrame>> {
        let mut paired = Vec::with_capacity(outputs.len());
        for filtered in outputs {
            if !filtered.belongs_to(&self.device) {
                return Err(
                    "correction stream filtered output belongs to a different graphics device"
                        .into(),
                );
            }
            let control = self
                .pending
                .pop_front()
                .ok_or("correction stream produced output without a retained control")?;
            if filtered.frame() != &control.frame {
                return Err(
                    "correction stream output differs from its retained control source".into(),
                );
            }
            paired.push(CorrectionFrame {
                frame: control.frame,
                current: control.texture,
                filtered,
                device: self.device.clone(),
            });
        }
        Ok(paired)
    }

    fn validate_body(&self, body: &BodyPanorama) -> Fallible<()> {
        let texture = body.texture();
        if !body.belongs_to(&self.device)
            || [texture.width(), texture.height()] != self.size
            || texture.depth_or_array_layers() != 1
            || texture.format() != wgpu::TextureFormat::Rgba8Unorm
            || texture.dimension() != wgpu::TextureDimension::D2
            || texture.mip_level_count() != 1
            || texture.sample_count() != 1
            || !texture
                .usage()
                .contains(wgpu::TextureUsages::TEXTURE_BINDING)
        {
            return Err(
                "correction stream body panorama has unsupported device or texture geometry".into(),
            );
        }
        Ok(())
    }

    fn ensure_healthy(&self) -> Fallible<()> {
        if let Some(error) = &self.failure {
            Err(error.clone().into())
        } else {
            Ok(())
        }
    }

    fn ensure_active(&self) -> Fallible<()> {
        self.ensure_healthy()?;
        if self.finished {
            Err("correction stream received a source after finish".into())
        } else {
            Ok(())
        }
    }

    fn remember_failure<T>(&mut self, result: Fallible<T>) -> Fallible<T> {
        if let Err(error) = &result {
            self.failure = Some(error.to_string());
        }
        result
    }
}
