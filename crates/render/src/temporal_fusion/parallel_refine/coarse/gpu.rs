//! GPU-resident coarse-to-fine motion preparation.
//!
//! All seven immutable gray levels and the downstream luma grid stay on the
//! device. Each level searches independent blocks, then derives the next level's
//! global predictor and seeds on the GPU. There is no host pixel/vector transfer,
//! submission, completion wait, source scheduling or colour policy here.
//!
//! This is the independent-block coarse candidate, not the serial Studio search.
//! The unchanged CPU serial path remains available as a quality comparison.

use crate::Fallible;
use crate::temporal_fusion::parallel_refine::gpu as refine;
use crate::temporal_fusion::pyramid::gpu as pyramid;

use super::prepare::{self, Prepared};

const LEVELS: usize = 7;
const BLOCK: u32 = 16;

/// Immutable resident motion inputs for one source. Its caller owns the exact
/// source stamp and must retain this value with the corresponding history slot.
pub struct MotionPyramid {
    device: wgpu::Device,
    levels: Vec<pyramid::PackedGray>,
    luma: wgpu::Texture,
}

impl MotionPyramid {
    pub fn encode(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        builder: &pyramid::Builder,
        source: &pyramid::Output,
    ) -> Fallible<Self> {
        if source.levels.len() != LEVELS {
            return Err("resident motion needs seven gray pyramid levels".into());
        }
        let first = &source.levels[0];
        let mut expected = [first.width(), first.height()];
        if expected
            .iter()
            .any(|dimension| !(1_024..8_192).contains(dimension))
        {
            return Err(
                "resident motion pyramid base dimensions must be from 1024 through 8191".into(),
            );
        }
        // Validate every level before recording the first packing pass. Odd
        // camera dimensions halve downward, as in the existing pyramid.
        for level in &source.levels {
            if [level.width(), level.height()] != expected
                || level.format() != wgpu::TextureFormat::R8Uint
                || level.dimension() != wgpu::TextureDimension::D2
                || level.depth_or_array_layers() != 1
                || level.sample_count() != 1
                || level.mip_level_count() != 1
                || !level.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING)
            {
                return Err("resident motion pyramid level differs from its gray geometry".into());
            }
            expected = [expected[0] / 2, expected[1] / 2];
        }
        let levels = source
            .levels
            .iter()
            .map(|level| builder.encode_packed_base(device, encoder, level))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            device: device.clone(),
            levels,
            // Motion packing consumes precisely the old CPU coarse[2] image,
            // which is full pyramid level3, without its round trip.
            luma: source.levels[3].clone(),
        })
    }

    pub fn finest(&self) -> &pyramid::PackedGray {
        &self.levels[0]
    }

    pub(crate) fn luma(&self) -> &wgpu::Texture {
        &self.luma
    }

    pub(crate) fn device(&self) -> &wgpu::Device {
        &self.device
    }
}

pub struct Builder {
    device: wgpu::Device,
    search: refine::Builder,
    prepare: prepare::Builder,
}

impl Builder {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            device: device.clone(),
            search: refine::Builder::new(device),
            prepare: prepare::Builder::new(device),
        }
    }

    /// Record the complete independent coarse-to-finest motion field. Both
    /// halves share one search builder and consume only resident inputs.
    pub fn encode_motion(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        current: &MotionPyramid,
        references: &[&MotionPyramid],
    ) -> Fallible<refine::Output> {
        let prepared = self.encode_finest_inputs(device, encoder, current, references)?;
        let references: Vec<_> = references
            .iter()
            .map(|reference| reference.finest())
            .collect();
        Ok(self.search.encode_finest_resident(
            device,
            encoder,
            current.finest(),
            &references,
            &prepared,
        )?)
    }

    /// Record levels6 through1 and the final level0 seed preparation. Level
    /// barriers remain GPU command ordering; no CPU search runs between them.
    pub fn encode_finest_inputs(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        current: &MotionPyramid,
        references: &[&MotionPyramid],
    ) -> Fallible<Prepared> {
        if self.device != *device
            || current.device() != device
            || references
                .iter()
                .any(|reference| reference.device() != device)
        {
            return Err(
                "resident coarse motion inputs belong to a different graphics device".into(),
            );
        }
        if !(1..=6).contains(&references.len()) {
            return Err("resident coarse motion needs one through six references".into());
        }
        if references.iter().any(|reference| {
            reference
                .levels
                .iter()
                .zip(&current.levels)
                .any(|(reference, current)| reference.logical_size() != current.logical_size())
        }) {
            return Err("resident coarse motion reference geometry differs from its source".into());
        }
        let mut prepared = None;
        for level in (1..LEVELS).rev() {
            let reference_levels: Vec<_> = references
                .iter()
                .map(|reference| &reference.levels[level])
                .collect();
            let raw = self.search.encode_coarse_resident(
                device,
                encoder,
                &current.levels[level],
                &reference_levels,
                prepared.as_ref(),
            )?;
            let next = current.levels[level - 1].logical_size();
            prepared = Some(self.prepare.encode(
                device,
                encoder,
                &raw,
                [next[0] / BLOCK, next[1] / BLOCK],
            )?);
        }
        Ok(prepared.expect("seven validated levels prepare the finest inputs"))
    }
}

#[cfg(test)]
mod tests;
