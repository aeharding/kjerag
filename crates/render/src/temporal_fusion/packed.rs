//! Quartet fusion with shared motion/luma lookups and normalized YUV storage.
//! The full-plane [`super::GpuFuse`] remains the independent comparison path.

use super::{
    HorizontalBoundary, Inputs, Parameters, fusion_layout, output_texture, prepare_binding,
    validate,
};

pub(crate) struct Output {
    pub(crate) y: wgpu::Texture,
    pub(crate) uv: wgpu::Texture,
}

pub(crate) struct Encoder {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl Encoder {
    #[cfg(test)]
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        Self::with_boundary(device, HorizontalBoundary::Clamp)
    }

    pub(crate) fn with_boundary(device: &wgpu::Device, boundary: HorizontalBoundary) -> Self {
        let layout = fusion_layout(device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("packed temporal quartet fusion"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let source = format!(
            "{}\n{}",
            include_str!("../temporal_fusion.wgsl"),
            include_str!("packed.wgsl")
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("packed temporal quartet fusion"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("packed temporal quartet fusion"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("triangle"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fuse_packed"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[("PERIODIC_X", boundary.shader_value())],
                    ..Default::default()
                },
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rg8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { layout, pipeline }
    }

    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        inputs: Inputs<'_>,
        params: &Parameters,
        roi: [u32; 4],
    ) -> Result<Output, String> {
        validate(&inputs, params, roi)?;
        let prepared = prepare_binding(device, &self.layout, inputs, params, roi);
        let output = Output {
            y: output_texture(
                device,
                "packed temporally fused Y quartets",
                roi[2] / 2,
                roi[3] / 2,
                wgpu::TextureFormat::Rgba8Unorm,
            ),
            uv: output_texture(
                device,
                "packed temporally fused UV",
                roi[2] / 2,
                roi[3] / 2,
                wgpu::TextureFormat::Rg8Unorm,
            ),
        };
        let y = output.y.create_view(&Default::default());
        let uv = output.uv.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("packed temporal quartet fusion"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &y,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &uv,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &prepared.group, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        Ok(output)
    }
}

#[cfg(test)]
mod tests;
