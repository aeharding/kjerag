//! Direct compact body-panorama output for the temporal path.
//!
//! This primitive removes the full-size RGBA panorama between type-2 sampling
//! and the existing full-range NV12 representation. One half-resolution MRT
//! fragment evaluates the four exact full-resolution texel centres. RGBA holds
//! Y in top-left, top-right, bottom-left, bottom-right order; RG holds the UV
//! value formed from the arithmetic mean of those same four RGB samples.
//!
//! The former Rgba8Unorm panorama quantized gamma RGB before RGB-to-NV12.
//! `pack4x8unorm` followed by `unpack4x8unorm` retains an explicit 8-bit
//! boundary here. Attachment conversion and WGSL packing have shown backend
//! rounding differences elsewhere, so this candidate requires a real old-RGB
//! versus direct-output comparison before selection and makes no exactness
//! claim. It records no submission, wait, readback, history, or cadence policy.

use wgpu::util::DeviceExt;

use super::super::{DirectType2Pipeline, draw_wgsl_with_fusion_mode};
use crate::temporal_fusion::color::MatrixCoefficients;
use crate::{Extent, Fallible, FrameStamp, Size};

/// Compact source-stamped full-range YUV body panorama.
///
/// The textures and stamp have no public constructor and always come from one
/// draw of one bound source/map snapshot. `packed_y` has half the full width
/// and height but contains four Y bytes per texel; `uv` contains one UV pair.
pub(crate) struct CompactNv12Panorama {
    device: wgpu::Device,
    packed_y: wgpu::Texture,
    uv: wgpu::Texture,
    frame: FrameStamp,
    full: Size,
    coefficients: MatrixCoefficients,
}

impl CompactNv12Panorama {
    pub(crate) fn packed_y(&self) -> &wgpu::Texture {
        &self.packed_y
    }

    pub(crate) fn uv(&self) -> &wgpu::Texture {
        &self.uv
    }

    pub(crate) fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub(crate) fn size(&self) -> Size {
        self.full
    }

    pub(crate) fn coefficients(&self) -> MatrixCoefficients {
        self.coefficients
    }

    pub(crate) fn belongs_to(&self, device: &wgpu::Device) -> bool {
        self.device == *device
    }
}

/// Detached producer only. A later selected owner must keep its exact source,
/// map, fusion resources, and retirement boundary alive through completion.
pub(in crate::direct_type2) struct Producer {
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    parameters_layout: wgpu::BindGroupLayout,
    parameters_group: u32,
    fusion: bool,
}

impl Producer {
    pub(in crate::direct_type2) fn new(
        device: &wgpu::Device,
        direct: &DirectType2Pipeline,
    ) -> Fallible<Self> {
        if direct.device != *device {
            return Err(
                "compact NV12 panorama producer belongs to a different graphics device".into(),
            );
        }
        let fusion = direct.fusion_layout.is_some();
        let parameters_group = if fusion { 3 } else { 2 };
        let parameters_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compact NV12 panorama parameters"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(32),
                },
                count: None,
            }],
        });
        let source = shader_source(fusion, direct.fusion_sampler.is_some(), parameters_group);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("direct compact NV12 body panorama"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let mut layouts = vec![&direct.picture_layout, &direct.map_layout];
        layouts.extend(direct.fusion_layout.as_ref());
        layouts.push(&parameters_layout);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("direct compact NV12 body panorama"),
            bind_group_layouts: &layouts,
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("direct compact NV12 body panorama"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("panorama_nv12_fs"),
                compilation_options: Default::default(),
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
        Ok(Self {
            device: device.clone(),
            pipeline,
            parameters_layout,
            parameters_group,
            fusion,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::direct_type2) fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        picture: &wgpu::BindGroup,
        map: &wgpu::BindGroup,
        fusion: Option<&wgpu::BindGroup>,
        frame: FrameStamp,
        full: Size,
        coefficients: MatrixCoefficients,
    ) -> Fallible<CompactNv12Panorama> {
        validate(
            device,
            &self.device,
            full,
            coefficients,
            self.fusion,
            fusion.is_some(),
        )?;
        let half = Size::new(full.width / 2, full.height / 2);
        let target = |label, format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: half.extent(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let packed_y = target(
            "source-stamped compact panorama Y quartet",
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let uv = target(
            "source-stamped compact panorama UV",
            wgpu::TextureFormat::Rg8Unorm,
        );
        let mut words = [0u32; 8];
        words[0] = coefficients.r_cr.to_bits();
        words[1] = coefficients.g_cb.to_bits();
        words[2] = coefficients.g_cr.to_bits();
        words[3] = coefficients.b_cb.to_bits();
        words[4] = full.width;
        words[5] = full.height;
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let parameters = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("compact NV12 panorama matrix and size"),
            contents: &bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let parameters = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compact NV12 panorama matrix and size"),
            layout: &self.parameters_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: parameters.as_entire_binding(),
            }],
        });
        let y_view = packed_y.create_view(&Default::default());
        let uv_view = uv.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("direct compact NV12 body panorama"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &y_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &uv_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, picture, &[]);
            pass.set_bind_group(1, map, &[]);
            if let Some(fusion) = fusion {
                pass.set_bind_group(2, fusion, &[]);
            }
            pass.set_bind_group(self.parameters_group, &parameters, &[]);
            pass.draw(0..3, 0..1);
        }
        Ok(CompactNv12Panorama {
            device: device.clone(),
            packed_y,
            uv,
            frame,
            full,
            coefficients,
        })
    }
}

fn validate(
    device: &wgpu::Device,
    owner: &wgpu::Device,
    full: Size,
    coefficients: MatrixCoefficients,
    expected_fusion: bool,
    supplied_fusion: bool,
) -> Fallible<()> {
    let denominator =
        1.0 + coefficients.g_cb / coefficients.b_cb + coefficients.g_cr / coefficients.r_cr;
    if ![
        coefficients.r_cr,
        coefficients.g_cb,
        coefficients.g_cr,
        coefficients.b_cb,
        denominator,
    ]
    .into_iter()
    .all(f32::is_finite)
        || coefficients.r_cr == 0.0
        || coefficients.b_cb == 0.0
        || denominator == 0.0
    {
        return Err("compact panorama RGB/NV12 matrix must have a finite inverse".into());
    }
    if owner != device {
        return Err("compact NV12 panorama producer belongs to a different graphics device".into());
    }
    if full.width == 0
        || full.height == 0
        || !full.width.is_multiple_of(2)
        || !full.height.is_multiple_of(2)
        || full.width != full.height.saturating_mul(2)
        || full.width > device.limits().max_texture_dimension_2d
        || full.height > device.limits().max_texture_dimension_2d
    {
        return Err(format!(
            "compact NV12 body panorama must be even nonzero 2:1 within graphics limits, got {} by {}",
            full.width, full.height
        )
        .into());
    }
    if expected_fusion != supplied_fusion {
        return Err("compact NV12 panorama map and photometric binding presence differ".into());
    }
    Ok(())
}

fn shader_source(fusion: bool, hardware_fusion: bool, parameters_group: u32) -> String {
    format!(
        "{}\n{}",
        draw_wgsl_with_fusion_mode(fusion, hardware_fusion),
        include_str!("nv12.wgsl").replace("NV12_PARAMETERS_GROUP", &parameters_group.to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

    #[test]
    fn direct_compact_nv12_shaders_validate_with_and_without_fusion() {
        for (fusion, hardware, group) in [(false, false, 2), (true, false, 3), (true, true, 3)] {
            let source = shader_source(fusion, hardware, group);
            let module = wgpu::naga::front::wgsl::parse_str(&source).unwrap_or_else(|error| {
                panic!("compact NV12 shader ({fusion}, {hardware}) did not parse: {error}")
            });
            Validator::new(ValidationFlags::all(), Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|error| {
                    panic!("compact NV12 shader ({fusion}, {hardware}) did not validate: {error}")
                });
        }
    }

    #[test]
    fn shader_keeps_four_centres_rgb_quantization_and_conversion_order() {
        let source = include_str!("nv12.wgsl");
        assert_eq!(source.matches("panorama_gamma_rgb(base +").count(), 4);
        assert!(source.contains("unpack4x8unorm(pack4x8unorm"));
        assert!(source.contains("(a + b + c + d) * 0.25"));
        assert!(source.contains("convert_rgb_to_ycbcr(average).yz"));
    }
}
