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

    fn reference_positions(&self) -> impl Iterator<Item = usize> + '_ {
        self.interval.clone().filter(|&at| at != self.center)
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
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
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

#[cfg(test)]
mod tests;
