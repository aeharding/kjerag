//! Worker-driven full-panorama temporal filtering.
//!
//! This owner has no thread of its own. Motion images, searches and predictor
//! handoffs remain on the GPU. Only completed filtered outputs publish; the
//! current completion boundary drives wgpu with nonblocking polls. Seven real contiguous sources gate
//! startup centers 0 through 3, steady center 3, and tail centers 4 through 6.

use std::collections::VecDeque;
use std::sync::mpsc;
use std::time::Duration;

#[cfg(test)]
use crate::direct_type2::BodyPanorama;
use crate::direct_type2::CompactNv12Panorama;
use crate::{Fallible, FrameStamp};

use super::color::{GpuColorConversion, MatrixCoefficients};
use super::history::History;
use super::motion::{self, Geometry};
use super::parallel_refine::coarse::gpu::{self as coarse_gpu, MotionPyramid};
use super::pyramid::gpu as pyramid_gpu;
use super::settings::{EffParams, Provider};
use super::{GpuFuse, Output};

const SOURCES: usize = 7;
const CENTER: usize = 3;
const LEVELS: usize = 7;
const BLOCK: u32 = 16;
const SCALE_BASE: i32 = 4;
const TEMPORAL: f32 = 1.25;

struct Retained {
    stamp: FrameStamp,
    effective: EffParams,
    motion: Option<MotionPyramid>,
}

/// One completed full gamma-RGB panorama bound to its actual history center.
pub(crate) struct FilteredPanorama {
    texture: wgpu::Texture,
    frame: FrameStamp,
    device: wgpu::Device,
}

impl FilteredPanorama {
    pub(crate) fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub(crate) fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub(crate) fn belongs_to(&self, device: &wgpu::Device) -> bool {
        self.device == *device
    }
}

/// A single capture epoch's sequential temporal-filter state.
pub(crate) struct Stream {
    device: wgpu::Device,
    queue: wgpu::Queue,
    full: [u32; 2],
    provider: Provider,
    matrix: Option<MatrixCoefficients>,
    history: History,
    window: VecDeque<Retained>,
    next_center: usize,
    finished: bool,
    failure: Option<String>,
    color: GpuColorConversion,
    pyramid: pyramid_gpu::Builder,
    coarse: coarse_gpu::Builder,
    motion: motion::gpu::Builder,
    fuse: GpuFuse,
}

impl Stream {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        full: [u32; 2],
        provider: Provider,
    ) -> Fallible<Self> {
        validate_full(full)?;
        Ok(Self {
            device: device.clone(),
            queue: queue.clone(),
            full,
            provider,
            matrix: None,
            history: History::new(device, full)?,
            window: VecDeque::with_capacity(SOURCES),
            next_center: 0,
            finished: false,
            failure: None,
            color: GpuColorConversion::new(device),
            pyramid: pyramid_gpu::Builder::new(device),
            coarse: coarse_gpu::Builder::new(device),
            motion: motion::gpu::Builder::new(device),
            fuse: GpuFuse::new(device),
        })
    }

    pub(crate) fn push(&mut self, source: CompactNv12Panorama) -> Fallible<Vec<FilteredPanorama>> {
        let result = self.push_inner(Source::Compact(source));
        self.remember_failure(result)
    }

    /// Explicit old RGB ingestion oracle for tests of representation changes.
    #[cfg(test)]
    #[allow(dead_code, reason = "real-Scene representation comparison oracle")]
    pub(crate) fn push_rgb(
        &mut self,
        body: BodyPanorama,
        matrix: MatrixCoefficients,
    ) -> Fallible<Vec<FilteredPanorama>> {
        let result = self.push_inner(Source::Rgb { body, matrix });
        self.remember_failure(result)
    }

    pub(crate) fn finish(&mut self) -> Fallible<Vec<FilteredPanorama>> {
        let result = self.finish_inner();
        self.remember_failure(result)
    }

    fn push_inner(&mut self, source: Source) -> Fallible<Vec<FilteredPanorama>> {
        let started = trace_start();
        self.ensure_active()?;
        let (stamp, matrix) = match &source {
            Source::Compact(source) => {
                validate_compact(source, &self.device, self.full)?;
                (source.frame().clone(), source.coefficients())
            }
            #[cfg(test)]
            Source::Rgb { body, matrix } => {
                validate_body(body, &self.device, self.full)?;
                validate_matrix(*matrix)?;
                (body.frame().clone(), *matrix)
            }
        };
        if let Some(previous) = self.window.back() {
            if !previous.stamp.same_decode_epoch(&stamp) {
                return Err("temporal stream cannot cross a decode epoch".into());
            }
            if previous.stamp.index().checked_add(1) != Some(stamp.index()) {
                return Err("temporal stream sources must be contiguous".into());
            }
        }
        if self.matrix.is_some_and(|selected| selected != matrix) {
            return Err("temporal stream source matrix changed within one capture epoch".into());
        }
        self.matrix.get_or_insert(matrix);
        let effective = self
            .provider
            .parameters_at(stamp.timestamp().as_secs_f64() * 1000.0)?;
        if effective.radius > CENTER as u32 {
            return Err(format!(
                "temporal stream radius {} exceeds the seven-source window",
                effective.radius
            )
            .into());
        }
        if effective.radius > 0 {
            validate_motion_full(self.full)?;
        }

        if self.window.len() == SOURCES {
            if self.next_center != CENTER + 1 {
                return Err("temporal stream has undrained source centers".into());
            }
            self.window.pop_front();
            self.next_center -= 1;
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("streaming temporal source preparation"),
            });
        let luma = match source {
            Source::Compact(source) => {
                self.history
                    .encode_push_compact(&self.device, &mut encoder, &source)?
            }
            #[cfg(test)]
            Source::Rgb { body, matrix } => self.history.encode_push_rgb(
                &self.device,
                &mut encoder,
                &self.color,
                &stamp,
                body.texture(),
                matrix,
            )?,
        };
        let motion = if effective.radius == 0 {
            None
        } else {
            let pyramid =
                self.pyramid
                    .encode_history_luma(&self.device, &mut encoder, &luma, LEVELS)?;
            Some(MotionPyramid::encode(
                &self.device,
                &mut encoder,
                &self.pyramid,
                &pyramid,
            )?)
        };
        self.queue.submit([encoder.finish()]);
        // Subsequent consumers use this same queue. GPU ordering, not a CPU
        // readback fence, makes these images ready before motion search.
        // This timer now ends at submission, not preparation completion.
        trace_elapsed("prepare-submit", &stamp, started);
        self.window.push_back(Retained {
            stamp,
            effective,
            motion,
        });

        let mut output = Vec::new();
        if self.window.len() == SOURCES {
            while self.next_center <= CENTER {
                output.push(self.process(self.next_center)?);
                self.next_center += 1;
            }
        }
        Ok(output)
    }

    fn finish_inner(&mut self) -> Fallible<Vec<FilteredPanorama>> {
        self.ensure_healthy()?;
        if self.finished {
            return Ok(Vec::new());
        }
        let mut output = Vec::new();
        if self.window.len() == SOURCES {
            while self.next_center < SOURCES {
                output.push(self.process(self.next_center)?);
                self.next_center += 1;
            }
        }
        self.finished = true;
        Ok(output)
    }

    fn process(&mut self, center: usize) -> Fallible<FilteredPanorama> {
        self.prepare_references(center)?;
        let retained = &self.window[center];
        let history = self
            .history
            .window_at(center, retained.effective.radius as usize)?;
        let references: Vec<_> = history.reference_positions().collect();
        let phases: Vec<_> = history.reference_phases().collect();
        let stamps = history.stamps();
        if stamps.center != &retained.stamp
            || stamps
                .references
                .iter()
                .zip(&references)
                .any(|(stamp, &at)| *stamp != &self.window[at].stamp)
            || stamps.references.len() != references.len()
        {
            return Err("temporal stream history stamps differ from retained sources".into());
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("streaming full-panorama temporal filter"),
            });
        let fused = if references.is_empty() {
            history.encode_copy_current(&self.device, &mut encoder)?
        } else {
            self.encode_fusion(&mut encoder, retained, &history, &references, &phases)?
        };
        let texture = self.color.encode_planes_to_rgb(
            &mut encoder,
            &fused.y,
            &fused.uv,
            self.matrix
                .ok_or("temporal stream has no source color matrix")?,
        )?;
        let started = trace_start();
        self.queue.submit([encoder.finish()]);
        wait_for_queue(&self.device, &self.queue)?;
        trace_elapsed("filter-submit-complete", &retained.stamp, started);
        Ok(FilteredPanorama {
            texture,
            frame: retained.stamp.clone(),
            device: self.device.clone(),
        })
    }

    fn prepare_pyramid(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        luma: &wgpu::Texture,
    ) -> Fallible<MotionPyramid> {
        let pyramid = self
            .pyramid
            .encode_luma(&self.device, encoder, luma, LEVELS)?;
        MotionPyramid::encode(&self.device, encoder, &self.pyramid, &pyramid)
    }

    /// A source with radius zero needs no motion for its own output, but a
    /// neighbouring center may still reference it. Rebuild any missing motion
    /// inputs once from that exact source's retained NV12, never from a filtered
    /// picture or a source sampled again with different colour coefficients.
    fn prepare_references(&mut self, center: usize) -> Fallible<()> {
        let radius = self.window[center].effective.radius as usize;
        if radius == 0 {
            return Ok(());
        }
        let history = self.history.window_at(center, radius)?;
        let needed: Vec<_> = std::iter::once(center)
            .chain(history.reference_positions())
            .filter(|&at| self.window[at].motion.is_none())
            .collect();
        if needed.is_empty() {
            return Ok(());
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("temporal radius transition reference preparation"),
            });
        let mut pending = Vec::with_capacity(needed.len());
        for at in needed {
            let history = self.history.window_at(at, 0)?;
            if history.stamps().center != &self.window[at].stamp {
                return Err(
                    "temporal reference preparation differs from its retained source".into(),
                );
            }
            let source = history.encode_copy_current(&self.device, &mut encoder)?;
            let motion = self.prepare_pyramid(&mut encoder, &source.y)?;
            pending.push((at, motion));
        }
        self.queue.submit([encoder.finish()]);
        for (at, motion) in pending {
            self.window[at].motion = Some(motion);
        }
        Ok(())
    }

    fn encode_fusion(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        center: &Retained,
        history: &super::history::Window<'_>,
        references: &[usize],
        phases: &[f64],
    ) -> Fallible<Output> {
        if phases.len() != references.len() {
            return Err("temporal stream phase count differs from its references".into());
        }
        let current = center
            .motion
            .as_ref()
            .ok_or("temporal stream nonzero radius has no resident motion pyramid")?;
        let reference_pyramids: Vec<_> = references
            .iter()
            .map(|&at| {
                self.window[at]
                    .motion
                    .as_ref()
                    .ok_or("temporal stream nonzero radius has a missing resident reference")
            })
            .collect::<Result<_, _>>()?;
        let geometry = geometry(self.full, current)?;
        let refined =
            self.coarse
                .encode_motion(&self.device, encoder, current, &reference_pyramids)?;
        let flow = array_texture(
            &self.device,
            "streaming temporal packed motion",
            geometry.output_grid,
            references.len() as u32,
            wgpu::TextureFormat::Rgba16Sint,
        );
        for (ordinal, &phase) in phases.iter().enumerate() {
            let parameters = motion::ResidentParameters {
                geometry,
                confidence_y: &center.effective.confidence_y,
                confidence_uv: &center.effective.confidence_uv,
                scale_base: SCALE_BASE,
                scale_extra: center.effective.noise_integer,
                temporal: TEMPORAL,
                phase,
            };
            let packed = self.motion.encode_refined_resident(
                &self.device,
                encoder,
                &refined,
                ordinal as u32,
                current,
                &parameters,
            )?;
            copy_layer(encoder, &packed, &flow, ordinal as u32);
        }
        let inputs = history.inputs(&flow, current.luma());
        let parameters = history.parameters(
            center.effective.fusion.noise,
            center.effective.fusion.limit,
            center.effective.fusion.y_limits,
            center.effective.fusion.uv_limits,
        );
        Ok(self.fuse.encode(
            &self.device,
            encoder,
            inputs,
            &parameters,
            [0, 0, self.full[0], self.full[1]],
        )?)
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
            Err("temporal stream received a source after finish".into())
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

enum Source {
    Compact(CompactNv12Panorama),
    #[cfg(test)]
    Rgb {
        body: BodyPanorama,
        matrix: MatrixCoefficients,
    },
}

fn trace_start() -> Option<std::time::Instant> {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    ENABLED
        .get_or_init(|| std::env::var_os("KJERAG_NATIVE_LIFECYCLE_PROBE").is_some())
        .then(std::time::Instant::now)
}

fn trace_elapsed(stage: &str, stamp: &FrameStamp, started: Option<std::time::Instant>) {
    if let Some(started) = started {
        eprintln!(
            "temporal-stream: source={} stage={stage} elapsed_ms={:.6}",
            stamp.index(),
            started.elapsed().as_secs_f64() * 1000.0,
        );
    }
}

fn validate_full(full: [u32; 2]) -> Fallible<()> {
    if full[0] == 0
        || full[1] == 0
        || !full[0].is_multiple_of(2)
        || !full[1].is_multiple_of(2)
        || full[0] != full[1].saturating_mul(2)
    {
        return Err(format!(
            "temporal stream needs positive even 2:1 dimensions, got {} by {}",
            full[0], full[1]
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
fn validate_body(body: &BodyPanorama, device: &wgpu::Device, full: [u32; 2]) -> Fallible<()> {
    let texture = body.texture();
    if !body.belongs_to(device) {
        return Err("temporal stream body panorama belongs to a different graphics device".into());
    }
    if texture.width() != full[0]
        || texture.height() != full[1]
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
            "temporal stream body panorama has unsupported texture geometry or usage".into(),
        );
    }
    Ok(())
}

fn validate_compact(
    source: &CompactNv12Panorama,
    device: &wgpu::Device,
    full: [u32; 2],
) -> Fallible<()> {
    let packed = source.packed_y();
    let uv = source.uv();
    if !source.belongs_to(device) {
        return Err(
            "temporal stream compact panorama belongs to a different graphics device".into(),
        );
    }
    if [source.size().width, source.size().height] != full
        || packed.width() != full[0] / 2
        || packed.height() != full[1] / 2
        || uv.width() != full[0] / 2
        || uv.height() != full[1] / 2
        || packed.format() != wgpu::TextureFormat::Rgba8Unorm
        || uv.format() != wgpu::TextureFormat::Rg8Unorm
        || !sampled_single_2d(packed)
        || !sampled_single_2d(uv)
        || !uv.usage().contains(wgpu::TextureUsages::COPY_SRC)
    {
        return Err(
            "temporal stream compact panorama has unsupported texture geometry or usage".into(),
        );
    }
    validate_matrix(source.coefficients())
}

fn sampled_single_2d(texture: &wgpu::Texture) -> bool {
    texture.dimension() == wgpu::TextureDimension::D2
        && texture.depth_or_array_layers() == 1
        && texture.mip_level_count() == 1
        && texture.sample_count() == 1
        && texture
            .usage()
            .contains(wgpu::TextureUsages::TEXTURE_BINDING)
}

fn validate_matrix(matrix: MatrixCoefficients) -> Fallible<()> {
    let denominator = 1.0 + matrix.g_cb / matrix.b_cb + matrix.g_cr / matrix.r_cr;
    if [
        matrix.r_cr,
        matrix.g_cb,
        matrix.g_cr,
        matrix.b_cb,
        denominator,
    ]
    .into_iter()
    .all(f32::is_finite)
        && matrix.r_cr != 0.0
        && matrix.b_cb != 0.0
        && denominator != 0.0
    {
        Ok(())
    } else {
        Err("temporal stream source color matrix must have a finite inverse".into())
    }
}

fn validate_motion_full(full: [u32; 2]) -> Fallible<()> {
    let base = [full[0] / 2, full[1] / 2];
    // Both supported camera sizes have seven nonempty 16x16 search grids.
    // Successive dimensions floor-halve; an odd intermediate dimension does
    // not remove a usable level. Larger/smaller native level-count routes are
    // not implemented by this selected search.
    let mut shortest = base[0].min(base[1]);
    let mut levels = 0;
    while shortest >= BLOCK {
        levels += 1;
        shortest /= 2;
    }
    if base.into_iter().any(|value| value >= 8_192)
        || levels != LEVELS
        || full.into_iter().any(|value| !value.is_multiple_of(32))
    {
        return Err(format!(
            "temporal stream nonzero-radius motion does not support {} by {} panorama geometry",
            full[0], full[1]
        )
        .into());
    }
    Ok(())
}

fn geometry(full: [u32; 2], pyramid: &MotionPyramid) -> Fallible<Geometry> {
    if full.into_iter().any(|value| !value.is_multiple_of(32))
        || pyramid.finest().logical_size() != [full[0] / 2, full[1] / 2]
        || [pyramid.luma().width(), pyramid.luma().height()] != [full[0] / 16, full[1] / 16]
    {
        return Err("temporal stream geometry is unsupported by the selected motion path".into());
    }
    Ok(Geometry {
        full,
        raw_grid: [full[0] / 32, full[1] / 32],
        output_grid: [full[0] / 16, full[1] / 16],
        block: [BLOCK, BLOCK],
    })
}

fn wait_for_queue(device: &wgpu::Device, queue: &wgpu::Queue) -> Fallible<()> {
    let (send, receive) = mpsc::channel();
    queue.on_submitted_work_done(move || {
        let _ = send.send(());
    });
    loop {
        match receive.try_recv() {
            Ok(()) => return Ok(()),
            Err(mpsc::TryRecvError::Empty) => {
                device.poll(wgpu::PollType::Poll)?;
                std::thread::park_timeout(Duration::from_micros(100));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err("temporal GPU completion callback disconnected".into());
            }
        }
    }
}

fn array_texture(
    device: &wgpu::Device,
    label: &'static str,
    size: [u32; 2],
    layers: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn copy_layer(
    encoder: &mut wgpu::CommandEncoder,
    source: &wgpu::Texture,
    destination: &wgpu::Texture,
    layer: u32,
) {
    encoder.copy_texture_to_texture(
        source.as_image_copy(),
        wgpu::TexelCopyTextureInfo {
            texture: destination,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        source.size(),
    );
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod geometry_tests {
    use super::*;

    #[test]
    fn geometry_gate_accepts_both_cameras_with_seven_search_grids() {
        assert!(validate_full([7_680, 3_840]).is_ok());
        assert!(validate_motion_full([7_680, 3_840]).is_ok());
        assert!(validate_full([5_760, 2_880]).is_ok());
        assert!(validate_motion_full([5_760, 2_880]).is_ok());
        assert!(validate_motion_full([4_096, 2_048]).is_ok());
        assert!(validate_motion_full([2_048, 1_024]).is_err());
        assert!(validate_motion_full([8_192, 4_096]).is_err());
    }

    #[test]
    fn panorama_geometry_rejects_empty_odd_and_non_two_to_one_inputs() {
        for full in [[0, 0], [2, 0], [7, 4], [8, 6], [6, 8]] {
            assert!(validate_full(full).is_err(), "{full:?}");
        }
    }
}
