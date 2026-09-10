//! Source-stamped resident NV12 history for temporal fusion.
//!
//! This owner records copies but never submits or waits. A caller must submit
//! every command buffer which consumes a [`Window`] before submitting a later
//! command buffer that reuses its oldest physical layer. A decode-epoch change
//! requires a fresh owner; this type deliberately does not choose seek policy.
//!
//! All devices and resources must come from one `wgpu::Instance`, as in Scene.
//! The pinned native wgpu compares device IDs without their Instance identity;
//! the synchronous foreign-device guard can only distinguish devices within
//! that Instance. Mixing separate Instances is outside this owner's contract.

use std::collections::VecDeque;
use std::fmt;
use std::ops::Range;

use crate::FrameStamp;

use super::color::Nv12;
use super::{Inputs, Parameters};

const LAYERS: u32 = 7;
const CENTER: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidSize,
    UnsupportedSize,
    ForeignDevice,
    SourceTextures,
    DecodeEpoch,
    NonContiguous,
    IncompleteWindow,
    WindowPosition,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSize => "temporal history needs positive even NV12 dimensions",
            Self::UnsupportedSize => {
                "temporal history dimensions or seven array layers exceed graphics limits"
            }
            Self::ForeignDevice => {
                "temporal history and its source NV12 must belong to this graphics device"
            }
            Self::SourceTextures => {
                "temporal history needs matching sampled copyable single-layer NV12 textures"
            }
            Self::DecodeEpoch => "temporal history cannot cross a decode epoch",
            Self::NonContiguous => "temporal history sources must be contiguous",
            Self::IncompleteWindow => {
                "temporal history needs seven sources before selecting a window"
            }
            Self::WindowPosition => {
                "temporal history needs a center from 0 through 6 and a radius from 0 through 3"
            }
        })
    }
}

impl std::error::Error for Error {}

struct Slot {
    stamp: FrameStamp,
    layer: u32,
}

/// Seven full-resolution source images retained in fixed array textures.
pub struct History {
    device: wgpu::Device,
    full: [u32; 2],
    y: wgpu::Texture,
    uv: wgpu::Texture,
    slots: VecDeque<Slot>,
}

impl History {
    pub fn new(device: &wgpu::Device, full: [u32; 2]) -> Result<Self, Error> {
        if full
            .into_iter()
            .any(|value| value == 0 || !value.is_multiple_of(2))
        {
            return Err(Error::InvalidSize);
        }
        let limits = device.limits();
        if full
            .into_iter()
            .any(|value| value > limits.max_texture_dimension_2d)
            || limits.max_texture_array_layers < LAYERS
        {
            return Err(Error::UnsupportedSize);
        }

        Ok(Self {
            device: device.clone(),
            full,
            y: array_texture(
                device,
                "resident temporal Y history",
                full,
                wgpu::TextureFormat::R8Unorm,
            ),
            uv: array_texture(
                device,
                "resident temporal UV history",
                [full[0] / 2, full[1] / 2],
                wgpu::TextureFormat::Rg8Unorm,
            ),
            slots: VecDeque::with_capacity(LAYERS as usize),
        })
    }

    /// Record one exact arriving source after validating continuity and storage.
    ///
    /// Once full, the oldest physical layer is reused. Queue submission order
    /// must keep this write after every earlier consumer of that layer.
    /// The encoder and source planes must belong to the supplied device;
    /// the source planes must not have been replaced with foreign textures.
    pub fn encode_push(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        stamp: &FrameStamp,
        nv12: &Nv12,
    ) -> Result<(), Error> {
        self.validate_source(device, stamp, nv12)?;

        let layer = self
            .slots
            .front()
            .filter(|_| self.slots.len() == LAYERS as usize)
            .map_or(self.slots.len() as u32, |slot| slot.layer);
        copy_layer(encoder, &nv12.y, &self.y, layer);
        copy_layer(encoder, &nv12.uv, &self.uv, layer);

        if self.slots.len() == LAYERS as usize {
            self.slots.pop_front();
        }
        self.slots.push_back(Slot {
            stamp: stamp.clone(),
            layer,
        });
        Ok(())
    }

    pub fn window(&self) -> Option<Window<'_>> {
        self.window_at(CENTER, CENTER).ok()
    }

    /// Borrow an explicit center and clipped reference interval in a full ring.
    ///
    /// Studio's selected seven-source route uses centers 0 through 3 at startup,
    /// 3 in steady state, and 4 through 6 when flushing. Its effective radius is
    /// supplied per source. This method selects bindings, not availability or
    /// publication: the caller still owns that scheduling and submission order.
    /// It neither duplicates missing neighbors nor admits a shorter input ring.
    /// A zero radius exposes no references and cannot be sent to `GpuFuse`.
    pub fn window_at(&self, center: usize, radius: usize) -> Result<Window<'_>, Error> {
        if self.slots.len() != LAYERS as usize {
            return Err(Error::IncompleteWindow);
        }
        Ok(Window {
            history: self,
            center,
            interval: reference_interval(center, radius)?,
        })
    }

    fn validate_source(
        &self,
        device: &wgpu::Device,
        stamp: &FrameStamp,
        nv12: &Nv12,
    ) -> Result<(), Error> {
        if self.device != *device || !nv12.belongs_to(device) {
            return Err(Error::ForeignDevice);
        }
        if let Some(previous) = self.slots.back() {
            if !previous.stamp.same_decode_epoch(stamp) {
                return Err(Error::DecodeEpoch);
            }
            if previous.stamp.index().checked_add(1) != Some(stamp.index()) {
                return Err(Error::NonContiguous);
            }
        }
        if !source_texture(&nv12.y, self.full, wgpu::TextureFormat::R8Unorm)
            || !source_texture(
                &nv12.uv,
                [self.full[0] / 2, self.full[1] / 2],
                wgpu::TextureFormat::Rg8Unorm,
            )
        {
            return Err(Error::SourceTextures);
        }
        Ok(())
    }
}

/// An explicit center and its clipped neighbors backed by resident arrays.
pub struct Window<'a> {
    history: &'a History,
    center: usize,
    interval: Range<usize>,
}

pub struct WindowStamps<'a> {
    pub center: &'a FrameStamp,
    pub references: Vec<&'a FrameStamp>,
}

impl Window<'_> {
    pub fn stamps(&self) -> WindowStamps<'_> {
        WindowStamps {
            center: &self.history.slots[self.center].stamp,
            references: self
                .reference_positions()
                .map(|at| &self.history.slots[at].stamp)
                .collect(),
        }
    }

    pub fn inputs<'a>(&'a self, flow: &'a wgpu::Texture, luma: &'a wgpu::Texture) -> Inputs<'a> {
        Inputs {
            y: &self.history.y,
            uv: &self.history.uv,
            flow,
            luma,
        }
    }

    pub fn parameters(
        &self,
        noise: f32,
        limit: f32,
        y_limits: [f32; 256],
        uv_limits: [f32; 256],
    ) -> Parameters {
        Parameters {
            noise,
            limit,
            y_limits,
            uv_limits,
            current_layer: self.history.slots[self.center].layer,
            reference_layers: self
                .reference_positions()
                .map(|at| self.history.slots[at].layer)
                .collect(),
        }
    }

    /// Logical source positions in the exact order consumed by fusion.
    pub fn reference_positions(&self) -> impl Iterator<Item = usize> + '_ {
        ordered_reference_positions(self.center, &self.interval)
    }

    /// Native UpdateRefData's binary64 raised-cosine reference weights.
    /// These weight neighbouring images, not lens-colour coefficient updates.
    pub fn reference_phases(&self) -> impl Iterator<Item = f64> + '_ {
        let radius = effective_radius(self.center, &self.interval);
        self.reference_positions()
            .map(move |at| reference_phase(self.center.abs_diff(at), radius))
    }

    /// Copy a zero-reference window's current physical ring layer into an
    /// independent sampled result. This is the selected radius-zero operation
    /// after the same complete-seven-source scheduling gate as other windows;
    /// it does not allocate dummy flow or references.
    pub fn encode_copy_current(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<super::Output, Error> {
        if self.history.device != *device {
            return Err(Error::ForeignDevice);
        }
        if self.interval != (self.center..self.center + 1) {
            return Err(Error::WindowPosition);
        }
        let y = output_texture(
            device,
            "zero-reference temporal Y",
            self.history.full,
            wgpu::TextureFormat::R8Unorm,
        );
        let uv = output_texture(
            device,
            "zero-reference temporal UV",
            [self.history.full[0] / 2, self.history.full[1] / 2],
            wgpu::TextureFormat::Rg8Unorm,
        );
        let layer = self.history.slots[self.center].layer;
        copy_array_layer(encoder, &self.history.y, layer, &y);
        copy_array_layer(encoder, &self.history.uv, layer, &uv);
        Ok(super::Output { y, uv })
    }
}

fn effective_radius(center: usize, interval: &Range<usize>) -> usize {
    (center - interval.start).max(interval.end - 1 - center)
}

fn ordered_reference_positions(
    center: usize,
    interval: &Range<usize>,
) -> impl Iterator<Item = usize> + '_ {
    interval.clone().filter(move |&at| at != center)
}

// UpdateRefData 0x2bcba84; the selected binary64 phases and operation order
// are recorded in temporal-confidence-law-01 and the temporal research note.
fn reference_phase(distance: usize, radius: usize) -> f64 {
    let distance = distance.min(radius);
    if distance < 2 {
        0.0
    } else {
        0.5 * (1.0 - (std::f64::consts::PI * (distance - 1) as f64 / (radius - 1) as f64).cos())
    }
}

// ComputeFlowMetalFast 0x2c31314..0x2c31440 forms this clipped interval;
// ConfigFuseNormEncoder binds it in ascending order, excluding the center.
// docs/research/studio-image-fusion-temporal-602.md records the native audit.
fn reference_interval(center: usize, radius: usize) -> Result<Range<usize>, Error> {
    if center >= LAYERS as usize || radius > CENTER {
        return Err(Error::WindowPosition);
    }
    Ok(center.saturating_sub(radius)..(center + radius + 1).min(LAYERS as usize))
}

fn array_texture(
    device: &wgpu::Device,
    label: &'static str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: LAYERS,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn output_texture(
    device: &wgpu::Device,
    label: &'static str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn source_texture(texture: &wgpu::Texture, size: [u32; 2], format: wgpu::TextureFormat) -> bool {
    texture.format() == format
        && texture.dimension() == wgpu::TextureDimension::D2
        && texture.width() == size[0]
        && texture.height() == size[1]
        && texture.depth_or_array_layers() == 1
        && texture.mip_level_count() == 1
        && texture.sample_count() == 1
        && texture
            .usage()
            .contains(wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC)
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

fn copy_array_layer(
    encoder: &mut wgpu::CommandEncoder,
    source: &wgpu::Texture,
    layer: u32,
    destination: &wgpu::Texture,
) {
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: source,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        destination.as_image_copy(),
        destination.size(),
    );
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod phase_tests {
    use std::time::Duration;

    use super::*;
    use crate::temporal_fusion::color::{GpuColorConversion, MatrixCoefficients, Nv12};
    use crate::temporal_fusion::tests::{copy_texture, gpu, read_copy};

    const POSITIONS: [[&[usize]; 4]; 7] = [
        [&[], &[1], &[1, 2], &[1, 2, 3]],
        [&[], &[0, 2], &[0, 2, 3], &[0, 2, 3, 4]],
        [&[], &[1, 3], &[0, 1, 3, 4], &[0, 1, 3, 4, 5]],
        [&[], &[2, 4], &[1, 2, 4, 5], &[0, 1, 2, 4, 5, 6]],
        [&[], &[3, 5], &[2, 3, 5, 6], &[1, 2, 3, 5, 6]],
        [&[], &[4, 6], &[3, 4, 6], &[2, 3, 4, 6]],
        [&[], &[5], &[4, 5], &[3, 4, 5]],
    ];

    #[test]
    fn every_center_and_radius_keeps_clipped_ascending_reference_order() {
        for (center, radii) in POSITIONS.iter().enumerate() {
            for (radius, expected) in radii.iter().enumerate() {
                let interval = reference_interval(center, radius).unwrap();
                let actual: Vec<_> = ordered_reference_positions(center, &interval).collect();
                assert_eq!(&actual, expected, "center {center}, radius {radius}");
                assert_eq!(effective_radius(center, &interval), radius);
            }
        }
    }

    #[test]
    fn every_clipped_window_uses_its_actual_radius_for_ordered_phases() {
        for (center, radii) in POSITIONS.iter().enumerate() {
            for (radius, expected_positions) in radii.iter().enumerate() {
                let interval = reference_interval(center, radius).unwrap();
                let effective = effective_radius(center, &interval);
                let actual: Vec<_> = ordered_reference_positions(center, &interval)
                    .map(|at| reference_phase(center.abs_diff(at), effective).to_bits())
                    .collect();
                let expected: Vec<_> = expected_positions
                    .iter()
                    .map(|&at| expected_phase(center.abs_diff(at), effective).to_bits())
                    .collect();
                assert_eq!(actual, expected, "center {center}, radius {radius}");
            }
        }
    }

    #[test]
    fn center_three_full_radius_has_the_exact_native_midpoint_bits() {
        let interval = reference_interval(3, 3).unwrap();
        let phases: Vec<_> = ordered_reference_positions(3, &interval)
            .map(|at| reference_phase(3usize.abs_diff(at), 3).to_bits())
            .collect();
        assert_eq!(
            phases,
            [
                1.0_f64.to_bits(),
                0.49999999999999994_f64.to_bits(),
                0.0_f64.to_bits(),
                0.0_f64.to_bits(),
                0.49999999999999994_f64.to_bits(),
                1.0_f64.to_bits(),
            ]
        );
    }

    #[test]
    fn zero_reference_copy_is_stamped_and_independent_across_ring_reuse() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        let full = [32, 16];
        let uv = [16, 8];
        let conversion = GpuColorConversion::new(&device);
        let mut history = History::new(&device, full).unwrap();
        let mut stamps = Vec::new();
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut expected_first = None;
        let mut expected_last = None;
        let mut held_first = None;
        let mut copied_last = None;

        for ordinal in 0..8_u64 {
            let stamp = FrameStamp::for_test(
                612 + ordinal,
                Duration::from_millis(ordinal * 33),
                stamps.last(),
            );
            let source = converted_source(
                &device,
                &queue,
                &conversion,
                &mut encoder,
                ordinal as u8,
                full,
            );
            if ordinal == 0 {
                expected_first = Some((
                    copy_texture(&device, &mut encoder, &source.y, full, 1),
                    copy_texture(&device, &mut encoder, &source.uv, uv, 2),
                ));
            }
            if ordinal == 7 {
                expected_last = Some((
                    copy_texture(&device, &mut encoder, &source.y, full, 1),
                    copy_texture(&device, &mut encoder, &source.uv, uv, 2),
                ));
            }
            history
                .encode_push(&device, &mut encoder, &stamp, &source)
                .unwrap();
            stamps.push(stamp);

            if ordinal == 6 {
                let window = history.window_at(0, 0).unwrap();
                assert_eq!(window.stamps().center, &stamps[0]);
                assert!(window.stamps().references.is_empty());
                held_first = Some(window.encode_copy_current(&device, &mut encoder).unwrap());
                assert!(matches!(
                    history
                        .window_at(3, 1)
                        .unwrap()
                        .encode_copy_current(&device, &mut encoder),
                    Err(Error::WindowPosition)
                ));
            } else if ordinal == 7 {
                let window = history.window_at(6, 0).unwrap();
                assert_eq!(window.stamps().center, &stamps[7]);
                copied_last = Some(window.encode_copy_current(&device, &mut encoder).unwrap());
            }
        }

        let first = held_first.unwrap();
        let last = copied_last.unwrap();
        let first_copy = (
            copy_texture(&device, &mut encoder, &first.y, full, 1),
            copy_texture(&device, &mut encoder, &first.uv, uv, 2),
        );
        let last_copy = (
            copy_texture(&device, &mut encoder, &last.y, full, 1),
            copy_texture(&device, &mut encoder, &last.uv, uv, 2),
        );
        queue.submit([encoder.finish()]);

        for ((actual_y, actual_uv), (expected_y, expected_uv)) in [
            (first_copy, expected_first.unwrap()),
            (last_copy, expected_last.unwrap()),
        ] {
            assert_eq!(
                read_copy(&device, &actual_y, full, 1),
                read_copy(&device, &expected_y, full, 1)
            );
            assert_eq!(
                read_copy(&device, &actual_uv, uv, 2),
                read_copy(&device, &expected_uv, uv, 2)
            );
        }
    }

    fn expected_phase(distance: usize, radius: usize) -> f64 {
        match (radius, distance) {
            (_, 0 | 1) => 0.0,
            (2, 2) => 1.0,
            (3, 2) => 0.49999999999999994,
            (3, 3) => 1.0,
            _ => panic!("unexpected radius {radius} and distance {distance}"),
        }
    }

    fn converted_source(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        conversion: &GpuColorConversion,
        encoder: &mut wgpu::CommandEncoder,
        salt: u8,
        size: [u32; 2],
    ) -> Nv12 {
        let rgb = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("zero-reference copy source"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let pixels: Vec<_> = (0..size[0] * size[1])
            .flat_map(|at| {
                [
                    salt.wrapping_mul(37).wrapping_add(at as u8),
                    salt.wrapping_mul(19).wrapping_add((at * 3) as u8),
                    salt.wrapping_mul(53).wrapping_add((at * 7) as u8),
                    255,
                ]
            })
            .collect();
        queue.write_texture(
            rgb.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: Some(size[1]),
            },
            rgb.size(),
        );
        conversion
            .encode_rgb_to_nv12(
                encoder,
                &rgb,
                MatrixCoefficients::from_source_rgb([1.5748, 0.1873, 0.4681, 1.8556]),
            )
            .unwrap()
    }
}
