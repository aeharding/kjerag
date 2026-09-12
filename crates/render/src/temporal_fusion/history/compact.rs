//! Packed-luma expansion into one already-reserved history layer.
//!
//! This records one render pass only. The parent history validates source
//! provenance and geometry, owns the destination, and commits chronology.

pub(super) struct Unpack {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl Unpack {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compact panorama Y history unpack"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("compact panorama Y history unpack"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compact panorama Y history unpack"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("compact panorama Y history unpack"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("triangle"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("unpack_y"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            device: device.clone(),
            layout,
            pipeline,
        }
    }

    pub(super) fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        packed: &wgpu::Texture,
        output: &wgpu::TextureView,
    ) {
        let source = packed.create_view(&Default::default());
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compact panorama Y history unpack"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&source),
            }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compact panorama Y history unpack"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

const SHADER: &str = r#"
@group(0) @binding(0) var packed_y: texture_2d<f32>;

struct Vertex { @builtin(position) position: vec4<f32>, }
@vertex fn triangle(@builtin(vertex_index) vertex: u32) -> Vertex {
  let positions = array<vec2<f32>, 3>(
    vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
  return Vertex(vec4(positions[vertex], 0.0, 1.0));
}

@fragment fn unpack_y(@builtin(position) position: vec4<f32>) -> @location(0) f32 {
  let pixel = vec2<u32>(position.xy);
  let quartet = textureLoad(packed_y, vec2<i32>(pixel / vec2(2u)), 0);
  let lane = (pixel.y & 1u) * 2u + (pixel.x & 1u);
  return quartet[lane];
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::temporal_fusion::history::{
        array_texture, copy_array_layer, layer_view, output_texture,
    };
    use crate::temporal_fusion::tests::{copy_texture, gpu, read_copy};

    #[test]
    fn every_quartet_lane_expands_to_its_full_resolution_position() {
        let Some((device, queue)) = gpu() else {
            return;
        };
        let packed_size = [3, 2];
        let full = [6, 4];
        let packed = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("compact Y unpack source"),
            size: wgpu::Extent3d {
                width: packed_size[0],
                height: packed_size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let bytes: Vec<u8> = (0u8..24).map(|value| 11 + value * 7).collect();
        queue.write_texture(
            packed.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(packed_size[0] * 4),
                rows_per_image: Some(packed_size[1]),
            },
            packed.size(),
        );

        let history = array_texture(
            &device,
            "compact Y unpack destination",
            full,
            wgpu::TextureFormat::R8Unorm,
        );
        let destination = layer_view(&history, 5);
        let standalone = output_texture(
            &device,
            "compact Y unpack standalone copy",
            full,
            wgpu::TextureFormat::R8Unorm,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        Unpack::new(&device).encode(&mut encoder, &packed, &destination);
        copy_array_layer(&mut encoder, &history, 5, &standalone);
        let copy = copy_texture(&device, &mut encoder, &standalone, full, 1);
        queue.submit([encoder.finish()]);
        let actual = read_copy(&device, &copy, full, 1);

        let mut expected = vec![0; (full[0] * full[1]) as usize];
        for (tile, quartet) in bytes.chunks_exact(4).enumerate() {
            let tile = tile as u32;
            let x = (tile % packed_size[0]) * 2;
            let y = (tile / packed_size[0]) * 2;
            for (lane, &(dx, dy)) in [(0, 0), (1, 0), (0, 1), (1, 1)].iter().enumerate() {
                expected[((y + dy) * full[0] + x + dx) as usize] = quartet[lane];
            }
        }
        assert_eq!(actual, expected);
    }
}
