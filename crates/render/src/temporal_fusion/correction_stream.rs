//! Reduced temporal correction ownership, without a display policy.
//!
//! The selected live path keeps the seven-source cadence and motion/fusion
//! laws over an explicit reduced field with five real levels.
//! Each output carries the exact
//! unfiltered RGB/NV12/RGB control for its filtered source. It does not own a
//! high-resolution source snapshot, interpolate fields, or select playback.

use std::collections::VecDeque;

use crate::direct_type2::RgbPanorama;
use crate::{Fallible, FrameStamp};

use super::HorizontalBoundary;
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

/// Sequential owner for the selected reduced-resolution temporal correction.
pub(crate) struct CorrectionStream {
    device: wgpu::Device,
    queue: wgpu::Queue,
    size: [u32; 2],
    color: GpuColorConversion,
    stream: Stream,
    pending: VecDeque<Control>,
    finished: bool,
    failure: Option<String>,
    #[cfg(test)]
    review_shift: u32,
}

impl CorrectionStream {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        field_size: [u32; 2],
        provider: Provider,
    ) -> Fallible<Self> {
        let stream = Stream::new_quarter_resolution_review(device, queue, field_size, provider)?;
        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
            size: field_size,
            // The control and filtered term must reconstruct chroma with the
            // same cylindrical boundary, including radius-zero sources.
            color: GpuColorConversion::with_boundary(device, HorizontalBoundary::Periodic),
            stream,
            pending: VecDeque::with_capacity(SOURCES),
            finished: false,
            failure: None,
            #[cfg(test)]
            review_shift: if std::env::var_os("KJERAG_CORRECTION_CYCLIC_REVIEW").is_some() {
                assert!(field_size[0] > 512);
                512
            } else {
                0
            },
        })
    }

    /// Admit one exact gamma-RGB panorama. Coordinate ownership remains with
    /// the source/display pairing, not the image-domain filter. The encoded
    /// control takes the same RGB/NV12/RGB path as the Stream's history input.
    pub(crate) fn push(
        &mut self,
        body: impl Into<RgbPanorama>,
        matrix: MatrixCoefficients,
    ) -> Fallible<Vec<CorrectionFrame>> {
        let result = self.push_inner(body.into(), matrix);
        self.remember_failure(result)
    }

    pub(crate) fn finish(&mut self) -> Fallible<Vec<CorrectionFrame>> {
        let result = self.finish_inner();
        self.remember_failure(result)
    }

    fn push_inner(
        &mut self,
        body: RgbPanorama,
        matrix: MatrixCoefficients,
    ) -> Fallible<Vec<CorrectionFrame>> {
        self.ensure_active()?;
        self.validate_body(&body)?;
        // Exact texel permutation before either RGB/NV12 path. The final
        // fields are inversely permuted before display, leaving its source,
        // map, prefilter, view shader and all texture coordinates unchanged.
        #[cfg(test)]
        let body = if self.review_shift != 0 {
            let mut encoder = self.device.create_command_encoder(&Default::default());
            let body = body.cyclic_shift_for_review(&self.device, &mut encoder, self.review_shift);
            self.queue.submit([encoder.finish()]);
            body
        } else {
            body
        };
        let frame = body.frame().clone();
        if let Some(previous) = self.pending.back()
            && (!previous.frame.same_decode_epoch(&frame)
                || previous.frame.index().checked_add(1) != Some(frame.index()))
        {
            return Err(
                "correction stream controls must be contiguous within one decode epoch".into(),
            );
        }
        if self.pending.len() >= SOURCES {
            return Err("correction stream exceeded its seven-source control bound".into());
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("reduced temporal correction control"),
            });
        #[cfg(test)]
        let mut gpu_profile = crate::gpu_profile::Profile::begin(
            &self.device,
            &mut encoder,
            "control",
            &["control_conversion"],
            frame.index(),
        );
        let nv12 = self
            .color
            .encode_rgb_to_nv12(&mut encoder, body.texture(), matrix)?;
        let current = self.color.encode_nv12_to_rgb(&mut encoder, &nv12, matrix)?;
        #[cfg(test)]
        {
            gpu_profile.mark(&mut encoder, "control_conversion");
            gpu_profile.resolve(&mut encoder);
        }
        self.queue.submit([encoder.finish()]);
        #[cfg(test)]
        gpu_profile.report_after_submit(&self.queue);
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
            #[cfg(test)]
            let (control, filtered) = if self.review_shift != 0 {
                let mut encoder = self.device.create_command_encoder(&Default::default());
                let inverse = self.size[0] - self.review_shift;
                let current =
                    cyclic_shift_for_review(&self.device, &mut encoder, &control.texture, inverse);
                let filtered =
                    filtered.cyclic_shift_for_review(&self.device, &mut encoder, inverse);
                self.queue.submit([encoder.finish()]);
                (
                    Control {
                        texture: current,
                        ..control
                    },
                    filtered,
                )
            } else {
                (control, filtered)
            };
            paired.push(CorrectionFrame {
                frame: control.frame,
                current: control.texture,
                filtered,
                device: self.device.clone(),
            });
        }
        Ok(paired)
    }

    fn validate_body(&self, body: &RgbPanorama) -> Fallible<()> {
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

/// Test-only exact rightward cyclic permutation; no sampler or color math.
#[cfg(test)]
pub(crate) fn cyclic_shift_for_review(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    source: &wgpu::Texture,
    shift: u32,
) -> wgpu::Texture {
    let width = source.width();
    assert!(shift < width);
    assert_eq!(source.format(), wgpu::TextureFormat::Rgba8Unorm);
    assert_eq!(source.depth_or_array_layers(), 1);
    assert!(source.usage().contains(wgpu::TextureUsages::COPY_SRC));
    if shift == 0 {
        return source.clone();
    }
    let output = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("exact cyclic temporal diagnostic permutation"),
        size: source.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: source.format(),
        usage: wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    for (src_x, dst_x, count) in [(0, shift, width - shift), (width - shift, 0, shift)] {
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: source,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: src_x,
                    y: 0,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &output,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: dst_x,
                    y: 0,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: count,
                height: source.height(),
                depth_or_array_layers: 1,
            },
        );
    }
    output
}
