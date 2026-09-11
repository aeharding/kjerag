//! GPU-resident coarse-to-fine motion preparation.
//!
//! All immutable gray levels and the downstream luma grid stay on the device.
//! Each level searches independent blocks, then derives the next level's
//! global predictor and seeds on the GPU. There is no host pixel/vector transfer,
//! submission, completion wait, source scheduling or colour policy here.
//!
//! This is the independent-block coarse candidate, not the serial Studio search.
//! The unchanged CPU serial path remains available as a quality comparison.

use crate::Fallible;
use crate::temporal_fusion::HorizontalBoundary;
use crate::temporal_fusion::parallel_refine::gpu as refine;
use crate::temporal_fusion::pyramid::gpu as pyramid;

use super::prepare::{self, Prepared};

const FULL_RESOLUTION_LEVELS: usize = 7;
const HALF_RESOLUTION_LEVELS: usize = 6;
const FIVE_LEVEL_REVIEW_LEVELS: usize = 5;
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
        Self::encode_with_levels(device, encoder, builder, source, FULL_RESOLUTION_LEVELS)
    }

    pub(crate) fn encode_with_levels(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        builder: &pyramid::Builder,
        source: &pyramid::Output,
        expected_levels: usize,
    ) -> Fallible<Self> {
        if !matches!(
            expected_levels,
            FIVE_LEVEL_REVIEW_LEVELS | HALF_RESOLUTION_LEVELS | FULL_RESOLUTION_LEVELS
        ) || source.levels.len() != expected_levels
        {
            return Err(format!(
                "resident motion needs five, six or seven requested gray pyramid levels, requested {expected_levels} and got {}",
                source.levels.len()
            )
            .into());
        }
        let first = &source.levels[0];
        let mut expected = [first.width(), first.height()];
        if expected.iter().any(|dimension| *dimension >= 8_192) {
            return Err("resident motion pyramid base dimensions must be below 8192".into());
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
        let coarsest = source
            .levels
            .last()
            .expect("the validated motion pyramid has levels");
        if [coarsest.width(), coarsest.height()]
            .into_iter()
            .any(|dimension| dimension < BLOCK)
        {
            return Err("resident motion pyramid coarsest level needs a 16 by 16 block".into());
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
    boundary: HorizontalBoundary,
}

impl Builder {
    pub fn new(device: &wgpu::Device) -> Self {
        Self::with_boundary(device, HorizontalBoundary::Clamp)
    }

    pub(crate) fn with_boundary(device: &wgpu::Device, boundary: HorizontalBoundary) -> Self {
        Self {
            device: device.clone(),
            search: refine::Builder::with_boundary(device, boundary),
            prepare: prepare::Builder::new(device),
            boundary,
        }
    }

    pub(crate) fn prepare_pipelines(&self) {
        self.search.prepare_coarse_pipelines();
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
        let output = if matches!(
            current.levels.len(),
            FIVE_LEVEL_REVIEW_LEVELS | HALF_RESOLUTION_LEVELS
        ) {
            self.search.encode_finest_resident_half_resolution_review(
                device,
                encoder,
                current.finest(),
                &references,
                &prepared,
            )?
        } else {
            self.search.encode_finest_resident(
                device,
                encoder,
                current.finest(),
                &references,
                &prepared,
            )?
        };
        Ok(output)
    }

    /// Record the coarsest level through level1 and the final level0 seed
    /// preparation. Level barriers remain GPU command ordering; no CPU search
    /// runs between them.
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
        let level_count = matching_level_count(
            current.levels.len(),
            references.iter().map(|reference| reference.levels.len()),
        )?;
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
        for level in (1..level_count).rev() {
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
            let complete_ring = self.boundary == HorizontalBoundary::Periodic
                && raw.blocks()[0] * BLOCK == current.levels[level].logical_size()[0];
            prepared = Some(self.prepare.encode_with_periodic_grid(
                device,
                encoder,
                &raw,
                [next[0] / BLOCK, next[1] / BLOCK],
                complete_ring,
            )?);
        }
        Ok(prepared.expect("five through seven validated levels prepare the finest inputs"))
    }
}

fn matching_level_count(
    current: usize,
    references: impl IntoIterator<Item = usize>,
) -> Fallible<usize> {
    if !matches!(
        current,
        FIVE_LEVEL_REVIEW_LEVELS | HALF_RESOLUTION_LEVELS | FULL_RESOLUTION_LEVELS
    ) || references.into_iter().any(|levels| levels != current)
    {
        Err("resident coarse motion needs matching five-, six- or seven-level pyramids".into())
    } else {
        Ok(current)
    }
}

#[cfg(test)]
mod tests;
