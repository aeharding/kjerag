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
//! rounding differences elsewhere. The real old-RGB comparison is not exact;
//! moving-output acceptance of this integration remains pending. It records
//! no submission, wait, readback, history, or cadence policy.

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

/// Shared recording primitive. Each caller owns the exact source/map/fusion
/// lifetime and submission; resident callers use their existing retirement gate.
pub(in crate::direct_type2) struct Producer {
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    fusion: bool,
}

pub(crate) struct Prepared {
    output: CompactNv12Panorama,
}

impl Prepared {
    pub(crate) fn views(&self) -> [wgpu::TextureView; 2] {
        [
            self.output.packed_y.create_view(&Default::default()),
            self.output.uv.create_view(&Default::default()),
        ]
    }

    pub(crate) fn into_output(self) -> CompactNv12Panorama {
        self.output
    }
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
        let source = shader_source(fusion, direct.fusion_sampler.is_some());
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("direct compact NV12 body panorama"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let mut layouts = vec![&direct.picture_layout, &direct.map_layout];
        layouts.extend(direct.fusion_layout.as_ref());
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
            fusion,
        })
    }

    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
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
        let prepared = self.prepare(device, frame, full, coefficients, fusion.is_some())?;
        let [y_view, uv_view] = prepared.views();
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("direct compact NV12 body panorama"),
                color_attachments: &[Some(attachment(&y_view)), Some(attachment(&uv_view))],
                ..Default::default()
            });
            self.draw(&mut pass, picture, map, fusion);
        }
        Ok(prepared.into_output())
    }

    pub(in crate::direct_type2) fn prepare(
        &self,
        device: &wgpu::Device,
        frame: FrameStamp,
        full: Size,
        coefficients: MatrixCoefficients,
        supplied_fusion: bool,
    ) -> Fallible<Prepared> {
        validate(
            device,
            &self.device,
            full,
            coefficients,
            self.fusion,
            supplied_fusion,
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
        Ok(Prepared {
            output: CompactNv12Panorama {
                device: device.clone(),
                packed_y,
                uv,
                frame,
                full,
                coefficients,
            },
        })
    }

    pub(in crate::direct_type2) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        picture: &wgpu::BindGroup,
        map: &wgpu::BindGroup,
        fusion: Option<&wgpu::BindGroup>,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, picture, &[]);
        pass.set_bind_group(1, map, &[]);
        if let Some(fusion) = fusion {
            pass.set_bind_group(2, fusion, &[]);
        }
        pass.draw(0..3, 0..1);
    }
}

pub(crate) fn attachment(view: &wgpu::TextureView) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            store: wgpu::StoreOp::Store,
        },
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

/// The shader obtains geometry from the exact picture uniform. Reject scaled
/// diagnostic targets before allocation instead of silently moving sample centres.
pub(in crate::direct_type2) fn require_full_source(full: Size, source: [f32; 2]) -> Fallible<()> {
    if [full.width as f32, full.height as f32] != [source[0] * 2.0, source[1]] {
        return Err("compact NV12 panorama must use the bound source's full body size".into());
    }
    Ok(())
}

fn shader_source(fusion: bool, hardware_fusion: bool) -> String {
    format!(
        "{}\n{}",
        draw_wgsl_with_fusion_mode(fusion, hardware_fusion),
        include_str!("nv12.wgsl")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

    #[test]
    fn compact_pipeline_fits_native_three_bind_group_device() {
        let (device, _) = match crate::direct_type2::tests::gpu() {
            Ok(gpu) => gpu,
            Err(error) => {
                assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none(), "{error}");
                eprintln!("skipping compact native-limit test: {error}");
                return;
            }
        };
        assert_eq!(device.limits().max_bind_groups, 3);
        let picture = crate::scene::bind_group_layout(&device);
        for fusion in [false, true] {
            let direct = DirectType2Pipeline::with_fusion(
                &device,
                &picture,
                wgpu::TextureFormat::Rgba8Unorm,
                fusion,
            );
            Producer::new(&device, &direct).unwrap();
        }
    }

    #[test]
    fn direct_compact_nv12_shaders_validate_with_and_without_fusion() {
        for (fusion, hardware) in [(false, false), (true, false), (true, true)] {
            let source = shader_source(fusion, hardware);
            let module = wgpu::naga::front::wgsl::parse_str(&source).unwrap_or_else(|error| {
                panic!("compact NV12 shader ({fusion}, {hardware}) did not parse: {error}")
            });
            Validator::new(ValidationFlags::all(), Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|error| {
                    panic!("compact NV12 shader ({fusion}, {hardware}) did not validate: {error}")
                });
            for (_, global) in module.global_variables.iter() {
                if let Some(binding) = &global.binding {
                    assert!(
                        binding.group < 3,
                        "native renderer permits only three bind groups"
                    );
                }
            }
        }
    }

    #[test]
    fn compact_geometry_must_match_bound_source_uniform() {
        assert!(require_full_source(Size::new(7680, 3840), [3840.0, 3840.0]).is_ok());
        assert!(require_full_source(Size::new(5760, 2880), [2880.0, 2880.0]).is_ok());
        assert!(require_full_source(Size::new(3840, 1920), [3840.0, 3840.0]).is_err());
        assert!(require_full_source(Size::new(7680, 3840), [f32::NAN, 3840.0]).is_err());
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
