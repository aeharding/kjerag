//! Worker-driven full-panorama temporal filtering.
//!
//! This owner has no thread of its own. Its CPU pyramid readbacks and final
//! GPU completion are intentionally synchronous for the calling worker, but
//! drive wgpu only with nonblocking polls. Seven real contiguous sources gate
//! startup centers 0 through 3, steady center 3, and tail centers 4 through 6.

use std::collections::VecDeque;
use std::sync::mpsc;
use std::time::Duration;

use crate::direct_type2::BodyPanorama;
use crate::{Fallible, FrameStamp};

use super::color::{GpuColorConversion, MatrixCoefficients};
use super::history::History;
use super::motion::{self, Geometry};
use super::parallel_refine;
use super::pyramid::{Level, gpu as pyramid_gpu};
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
    levels: Option<Vec<Level>>,
    gpu_base: Option<pyramid_gpu::PackedGray>,
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
    refine: parallel_refine::gpu::Builder,
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
            refine: parallel_refine::gpu::Builder::new(device),
            motion: motion::gpu::Builder::new(device),
            fuse: GpuFuse::new(device),
        })
    }

    pub(crate) fn push(
        &mut self,
        body: BodyPanorama,
        matrix: MatrixCoefficients,
    ) -> Fallible<Vec<FilteredPanorama>> {
        let result = self.push_inner(body, matrix);
        self.remember_failure(result)
    }

    pub(crate) fn finish(&mut self) -> Fallible<Vec<FilteredPanorama>> {
        let result = self.finish_inner();
        self.remember_failure(result)
    }

    fn push_inner(
        &mut self,
        body: BodyPanorama,
        matrix: MatrixCoefficients,
    ) -> Fallible<Vec<FilteredPanorama>> {
        self.ensure_active()?;
        validate_body(&body, &self.device, self.full)?;
        let stamp = body.frame().clone();
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
        let nv12 = self
            .color
            .encode_rgb_to_nv12(&mut encoder, body.texture(), matrix)?;
        self.history
            .encode_push(&self.device, &mut encoder, &stamp, &nv12)?;
        let (gpu_base, reads) = if effective.radius == 0 {
            (None, Vec::new())
        } else {
            let pyramid = self
                .pyramid
                .encode_luma(&self.device, &mut encoder, &nv12.y, LEVELS)?;
            let packed =
                self.pyramid
                    .encode_packed_base(&self.device, &mut encoder, &pyramid.levels[0])?;
            let reads = pyramid
                .levels
                .iter()
                .map(|level| PendingLevel::encode(&self.device, &mut encoder, level))
                .collect();
            (Some(packed), reads)
        };
        self.queue.submit([encoder.finish()]);
        let levels = if reads.is_empty() {
            wait_for_queue(&self.device, &self.queue)?;
            None
        } else {
            Some(read_levels(&self.device, reads)?)
        };
        self.window.push_back(Retained {
            stamp,
            effective,
            levels,
            gpu_base,
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

    fn process(&self, center: usize) -> Fallible<FilteredPanorama> {
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
        self.queue.submit([encoder.finish()]);
        wait_for_queue(&self.device, &self.queue)?;
        Ok(FilteredPanorama {
            texture,
            frame: retained.stamp.clone(),
            device: self.device.clone(),
        })
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
        let center_levels = center
            .levels
            .as_ref()
            .ok_or("temporal stream nonzero radius has no current pyramid")?;
        let current = center
            .gpu_base
            .as_ref()
            .ok_or("temporal stream nonzero radius has no current packed base")?;
        let reference_levels: Vec<_> = references
            .iter()
            .map(|&at| {
                self.window[at]
                    .levels
                    .as_deref()
                    .ok_or("temporal stream nonzero radius has a missing reference pyramid")
            })
            .collect::<Result<_, _>>()?;
        let coarse: Vec<_> = reference_levels
            .iter()
            .map(|reference| super::search::prepare_finest(center_levels, reference))
            .collect::<Result<_, _>>()?;
        let reference_bases: Vec<_> = references
            .iter()
            .map(|&at| {
                self.window[at]
                    .gpu_base
                    .as_ref()
                    .ok_or("temporal stream nonzero radius has a missing packed reference")
            })
            .collect::<Result<_, _>>()?;
        let seeds: Vec<_> = coarse.iter().map(|input| input.seeds.as_slice()).collect();
        let globals: Vec<_> = coarse.iter().map(|input| input.global).collect();
        let refined = self.refine.encode_finest(
            &self.device,
            encoder,
            current,
            &reference_bases,
            &seeds,
            &globals,
        )?;

        let geometry = geometry(self.full, center_levels)?;
        let flow = array_texture(
            &self.device,
            "streaming temporal packed motion",
            geometry.output_grid,
            references.len() as u32,
            wgpu::TextureFormat::Rgba16Sint,
        );
        let luma_texture = array_texture(
            &self.device,
            "streaming temporal luma indices",
            geometry.output_grid,
            1,
            wgpu::TextureFormat::R8Uint,
        );
        write_layer(
            &self.queue,
            &luma_texture,
            0,
            geometry.output_grid,
            1,
            &center_levels[3].pixels,
        );
        for (ordinal, &phase) in phases.iter().enumerate() {
            let parameters = motion::Parameters {
                geometry,
                luma: &center_levels[3].pixels,
                confidence_y: &center.effective.confidence_y,
                confidence_uv: &center.effective.confidence_uv,
                scale_base: SCALE_BASE,
                scale_extra: center.effective.noise_integer,
                temporal: TEMPORAL,
                phase,
            };
            let packed = self.motion.encode_refined(
                &self.device,
                encoder,
                &refined,
                ordinal as u32,
                &parameters,
            )?;
            copy_layer(encoder, &packed, &flow, ordinal as u32);
        }
        let inputs = history.inputs(&flow, &luma_texture);
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

fn validate_motion_full(full: [u32; 2]) -> Fallible<()> {
    let base = [full[0] / 2, full[1] / 2];
    if base
        .into_iter()
        .any(|value| !(1_024..8_192).contains(&value) || !value.is_multiple_of(64))
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

fn geometry(full: [u32; 2], levels: &[Level]) -> Fallible<Geometry> {
    if levels.len() != LEVELS
        || full.into_iter().any(|value| !value.is_multiple_of(32))
        || levels[0].width != full[0] as usize / 2
        || levels[0].height != full[1] as usize / 2
        || levels[3].width != full[0] as usize / 16
        || levels[3].height != full[1] as usize / 16
        || levels[3].pixels.len() != (full[0] as usize / 16) * (full[1] as usize / 16)
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

struct PendingLevel {
    width: u32,
    height: u32,
    row: u32,
    buffer: wgpu::Buffer,
}

impl PendingLevel {
    fn encode(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> Self {
        let width = texture.width();
        let height = texture.height();
        let row =
            width.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("streaming temporal CPU pyramid readback"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        Self {
            width,
            height,
            row,
            buffer,
        }
    }

    fn finish(self) -> Level {
        let mapped = self.buffer.slice(..).get_mapped_range();
        let pixels = mapped
            .chunks_exact(self.row as usize)
            .flat_map(|row| row[..self.width as usize].iter().copied())
            .collect();
        drop(mapped);
        self.buffer.unmap();
        Level {
            width: self.width as usize,
            height: self.height as usize,
            pixels,
        }
    }
}

fn read_levels(device: &wgpu::Device, reads: Vec<PendingLevel>) -> Fallible<Vec<Level>> {
    let (send, receive) = mpsc::channel();
    for read in &reads {
        let send = send.clone();
        read.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
    }
    drop(send);
    for _ in 0..reads.len() {
        loop {
            match receive.try_recv() {
                Ok(result) => {
                    result?;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    device.poll(wgpu::PollType::Poll)?;
                    std::thread::park_timeout(Duration::from_micros(100));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err("temporal pyramid readback callback disconnected".into());
                }
            }
        }
    }
    Ok(reads.into_iter().map(PendingLevel::finish).collect())
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

fn write_layer(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    layer: u32,
    size: [u32; 2],
    bytes_per_pixel: u32,
    bytes: &[u8],
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0] * bytes_per_pixel),
            rows_per_image: Some(size[1]),
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
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
mod tests {
    use super::*;

    #[test]
    fn geometry_gate_distinguishes_full_filter_from_radius_zero_only_sizes() {
        assert!(validate_full([7_680, 3_840]).is_ok());
        assert!(validate_motion_full([7_680, 3_840]).is_ok());
        assert!(validate_full([5_760, 2_880]).is_ok());
        assert!(validate_motion_full([5_760, 2_880]).is_err());
    }

    #[test]
    fn panorama_geometry_rejects_empty_odd_and_non_two_to_one_inputs() {
        for full in [[0, 0], [2, 0], [7, 4], [8, 6], [6, 8]] {
            assert!(validate_full(full).is_err(), "{full:?}");
        }
    }
}
