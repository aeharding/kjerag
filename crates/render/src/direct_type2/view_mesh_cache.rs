//! Native map vertices evaluated once per source, never once per screen pixel.
//!
//! This does not rasterize or resample the picture. Curved-view fragments keep
//! the native cell geometry, barycentrics, alpha and source sampling. Endpoint
//! positions, packed samples and triangle adjugates are source-owned GPU data.
//! A fragment evaluates ray dot products, with the watertight reference retained
//! as the fallback when neither compiled triangle admits the ray.

use std::num::NonZeroU64;

use crate::flow::one_xs::gpu_context::OneXsGpuContext;
use crate::studio_type2::{ALPHA_BYTES, PACKED_BYTES};
use crate::{Fallible, FrameStamp};

pub(crate) const VERTICES: u32 = 51 * 101;
pub(crate) const CACHE_BYTES: u64 = VERTICES as u64 * std::mem::size_of::<[f32; 8]>() as u64;
pub(crate) const TRIANGLES: u32 = 50 * 100 * 2;
pub(crate) const TRIANGLE_BYTES: u64 = TRIANGLES as u64 * std::mem::size_of::<[f32; 12]>() as u64;

pub(crate) struct Pipeline {
    device: wgpu::Device,
    compute_layout: wgpu::BindGroupLayout,
    pub(super) draw_layout: wgpu::BindGroupLayout,
    compute: wgpu::ComputePipeline,
    triangles: wgpu::ComputePipeline,
}

/// GPU-owned data and map binding inseparably named by the source delivery.
pub(crate) struct CachedMap {
    frame: FrameStamp,
    context: OneXsGpuContext,
    read: wgpu::BindGroup,
}

impl CachedMap {
    pub(crate) fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub(crate) fn ensure_context(&self, expected: &OneXsGpuContext) -> Fallible<()> {
        self.context.ensure_same(expected)
    }

    pub(crate) fn read(&self) -> &wgpu::BindGroup {
        &self.read
    }
}

impl Pipeline {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let compute_layout = layout(device, wgpu::ShaderStages::COMPUTE, false);
        let draw_layout = layout(
            device,
            wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            true,
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("native view mesh vertex preparation"),
            source: wgpu::ShaderSource::Wgsl(super::compiled_triangle_prepass_wgsl().into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("native view mesh vertex preparation"),
            bind_group_layouts: &[&compute_layout],
            immediate_size: 0,
        });
        let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("native view mesh vertex preparation"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cache_type2_vertices"),
            compilation_options: Default::default(),
            cache: None,
        });
        let triangles = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("native view triangle preparation"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cache_type2_triangles"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            device: device.clone(),
            compute_layout,
            draw_layout,
            compute,
            triangles,
        }
    }

    pub(crate) fn encode(
        &self,
        context: &OneXsGpuContext,
        encoder: &mut wgpu::CommandEncoder,
        frame: FrameStamp,
        packed: &wgpu::Buffer,
        alpha: &wgpu::Buffer,
    ) -> Fallible<CachedMap> {
        if context.device() != &self.device {
            return Err("view mesh cache belongs to a different graphics device".into());
        }
        let cache = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("source-owned native view mesh vertices"),
            size: CACHE_BYTES,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let triangles = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("source-owned native view triangle coefficients"),
            size: TRIANGLE_BYTES,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let compute = binding(
            &self.device,
            &self.compute_layout,
            packed,
            alpha,
            &cache,
            &triangles,
        );
        let read = binding(
            &self.device,
            &self.draw_layout,
            packed,
            alpha,
            &cache,
            &triangles,
        );
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("source-owned native view mesh preparation"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.compute);
            pass.set_bind_group(0, &compute, &[]);
            pass.dispatch_workgroups(VERTICES.div_ceil(64), 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("source-owned native view triangle preparation"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.triangles);
            pass.set_bind_group(0, &compute, &[]);
            pass.dispatch_workgroups(TRIANGLES.div_ceil(64), 1, 1);
        }
        // Both bind groups retain the cache allocation through submitted work.
        // The displayed owner retains the read binding after source retirement.
        Ok(CachedMap {
            frame,
            context: context.clone(),
            read,
        })
    }
}

fn layout(
    device: &wgpu::Device,
    visibility: wgpu::ShaderStages,
    cache_read_only: bool,
) -> wgpu::BindGroupLayout {
    let entry = |binding, bytes, read_only| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(bytes),
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("source-owned native view mesh map"),
        entries: &[
            entry(0, PACKED_BYTES as u64, true),
            entry(1, ALPHA_BYTES as u64, true),
            entry(2, CACHE_BYTES, cache_read_only),
            entry(3, TRIANGLE_BYTES, cache_read_only),
        ],
    })
}

fn binding(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    packed: &wgpu::Buffer,
    alpha: &wgpu::Buffer,
    cache: &wgpu::Buffer,
    triangles: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("source-owned native view mesh map"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: packed.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: alpha.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: cache.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: triangles.as_entire_binding(),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

    #[test]
    fn cache_retains_native_endpoints_and_record_layout() {
        assert_eq!(VERTICES, 5151);
        assert_eq!(CACHE_BYTES, 164_832);
        assert_eq!(TRIANGLES, 10_000);
        assert_eq!(TRIANGLE_BYTES, 480_000);
        let source = super::super::compiled_triangle_prepass_wgsl();
        assert!(source.contains("type2_position(row, col)"));
        assert!(source.contains("type2_sample4(uv)"));
        assert!(source.contains("index / 101u"));
        assert!(source.contains("index % 101u"));
        let module = wgpu::naga::front::wgsl::parse_str(&source).unwrap();
        Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .unwrap();
    }
}
