//! Single-pass direct view with a half-resolution temporal RGB residual.
//!
//! The high-frequency picture remains the native type-2 draw. Only the
//! accepted low-frequency term comes from the temporal panorama:
//! `clamp(rgb8(high) + filtered - current, 0, 1)`. The two low inputs are
//! inseparably owned by one [`CorrectionFrame`], and this module neither
//! schedules them nor interpolates between their source times.

use crate::temporal_fusion::correction_stream::CorrectionFrame;
use crate::{Fallible, MAX_LENSES, Planes, Reframe};

use super::{DirectType2Pipeline, draw_wgsl_with_fusion_mode};

const LOW_CURRENT_BINDING: u32 = 6;
const LOW_FILTERED_BINDING: u32 = 7;
const LOW_SAMPLER_BINDING: u32 = 8;

/// Immutable target-format pipelines and bindings shared by corrected draws.
///
/// The installed native map stays in group 1 and the optional photometric map
/// stays in group 2. Extending the source picture in group 0 therefore keeps
/// the complete draw within the native three-bind-group limit.
pub(crate) struct CorrectionPipeline {
    device: wgpu::Device,
    output_format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    picture_layout: wgpu::BindGroupLayout,
    source_sampler: wgpu::Sampler,
    low_sampler: wgpu::Sampler,
    fusion: bool,
}

/// One immutable view/source/correction binding.
///
/// Construction remains inside the sealed `SourceSnapshot` wrapper. That
/// wrapper authenticates the source frame before lending its private planes to
/// [`CorrectionPipeline::prepare_picture`]. The uniform allocation is kept
/// beside the bind group through every redraw and submitted-work retirement.
pub(crate) struct CorrectionPictureBinding {
    read: wgpu::BindGroup,
    _uniforms: wgpu::Buffer,
    rectilinear: bool,
}

impl CorrectionPipeline {
    pub(crate) fn new(
        device: &wgpu::Device,
        direct: &DirectType2Pipeline,
        output_format: wgpu::TextureFormat,
    ) -> Fallible<Self> {
        if direct.device != *device {
            return Err("corrected direct view belongs to a different graphics device".into());
        }
        if !matches!(
            output_format,
            wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Rgba8UnormSrgb
                | wgpu::TextureFormat::Bgra8Unorm
                | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            return Err(format!(
                "corrected direct view target format {output_format:?} is not a renderable RGBA surface format"
            )
            .into());
        }

        let fusion = direct.fusion_layout.is_some();
        let source = format!(
            "{}\n{CORRECTION_WGSL}",
            draw_wgsl_with_fusion_mode(fusion, direct.fusion_sampler.is_some())
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 corrected direct view"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let picture_layout = picture_layout(device);
        let mut layouts = vec![&picture_layout, &direct.map_layout];
        layouts.extend(direct.fusion_layout.as_ref());
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 corrected direct view"),
            bind_group_layouts: &layouts,
            immediate_size: 0,
        });
        let create_pipeline = |label, vertex, fragment| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: output_format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = create_pipeline("ONE X2 corrected direct view", "vs", "corrected_fs");
        let mesh_pipeline = create_pipeline(
            "ONE X2 corrected native sphere rasterization",
            "corrected_mesh_vs",
            "corrected_mesh_fs",
        );
        let low_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("periodic half-resolution temporal correction"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Ok(Self {
            device: device.clone(),
            output_format,
            pipeline,
            mesh_pipeline,
            picture_layout,
            source_sampler: direct.sampler.clone(),
            low_sampler,
            fusion,
        })
    }

    /// Build group 0 after the sealed source owner has authenticated the frame
    /// and lent its exact four imported lens planes.
    pub(super) fn prepare_picture(
        &self,
        reframe: &Reframe,
        lenses: [&Planes; MAX_LENSES],
        correction: &CorrectionFrame,
    ) -> Fallible<CorrectionPictureBinding> {
        if !correction.belongs_to(&self.device) {
            return Err("temporal correction belongs to a different graphics device".into());
        }
        if reframe.linearizes_output() != self.output_format.is_srgb() {
            return Err(
                "corrected direct view transfer does not match the render target format".into(),
            );
        }
        validate_low(correction.current_texture(), "current")?;
        let low = validate_low(correction.filtered_texture(), "filtered")?;
        if low
            != [
                correction.current_texture().width(),
                correction.current_texture().height(),
            ]
        {
            return Err("temporal correction panoramas have different geometry".into());
        }

        let uniforms = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("corrected direct view-private uniforms"),
            size: std::mem::size_of::<Reframe>() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        {
            let mut mapped = uniforms.slice(..).get_mapped_range_mut();
            mapped.copy_from_slice(reframe.bytes());
        }
        uniforms.unmap();

        let lens_views = lenses
            .iter()
            .flat_map(|planes| [&planes.luma, &planes.chroma])
            .map(|texture| texture.create_view(&Default::default()))
            .collect::<Vec<_>>();
        let current = correction
            .current_texture()
            .create_view(&Default::default());
        let filtered = correction
            .filtered_texture()
            .create_view(&Default::default());
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        }];
        entries.extend(
            lens_views
                .iter()
                .enumerate()
                .map(|(plane, view)| wgpu::BindGroupEntry {
                    binding: 1 + plane as u32,
                    resource: wgpu::BindingResource::TextureView(view),
                }),
        );
        entries.extend([
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(&self.source_sampler),
            },
            wgpu::BindGroupEntry {
                binding: LOW_CURRENT_BINDING,
                resource: wgpu::BindingResource::TextureView(&current),
            },
            wgpu::BindGroupEntry {
                binding: LOW_FILTERED_BINDING,
                resource: wgpu::BindingResource::TextureView(&filtered),
            },
            wgpu::BindGroupEntry {
                binding: LOW_SAMPLER_BINDING,
                resource: wgpu::BindingResource::Sampler(&self.low_sampler),
            },
        ]);
        let read = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("source-stamped corrected direct view"),
            layout: &self.picture_layout,
            entries: &entries,
        });
        Ok(CorrectionPictureBinding {
            read,
            _uniforms: uniforms,
            rectilinear: reframe.is_rectilinear(),
        })
    }

    pub(crate) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        picture: &CorrectionPictureBinding,
        map: &wgpu::BindGroup,
        fusion: Option<&wgpu::BindGroup>,
    ) {
        assert_eq!(
            self.fusion,
            fusion.is_some(),
            "corrected direct map and photometric binding presence differ"
        );
        pass.set_pipeline(if picture.rectilinear {
            &self.mesh_pipeline
        } else {
            &self.pipeline
        });
        pass.set_bind_group(0, &picture.read, &[]);
        pass.set_bind_group(1, map, &[]);
        if let Some(fusion) = fusion {
            pass.set_bind_group(2, fusion, &[]);
        }
        if picture.rectilinear {
            pass.draw(0..(100 * 50 * 6), 0..1);
        } else {
            pass.draw(0..3, 0..1);
        }
    }
}

fn picture_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<Reframe>() as u64),
        },
        count: None,
    }];
    entries.extend((1..=4).map(texture));
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: 5,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    });
    entries.extend([texture(LOW_CURRENT_BINDING), texture(LOW_FILTERED_BINDING)]);
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: LOW_SAMPLER_BINDING,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    });
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ONE X2 corrected direct picture"),
        entries: &entries,
    })
}

fn validate_low(texture: &wgpu::Texture, role: &str) -> Fallible<[u32; 2]> {
    let size = [texture.width(), texture.height()];
    if texture.format() != wgpu::TextureFormat::Rgba8Unorm
        || texture.dimension() != wgpu::TextureDimension::D2
        || texture.depth_or_array_layers() != 1
        || texture.mip_level_count() != 1
        || texture.sample_count() != 1
        || !texture
            .usage()
            .contains(wgpu::TextureUsages::TEXTURE_BINDING)
    {
        return Err(format!(
            "temporal correction {role} must be a single-sampled sampleable 2D Rgba8Unorm texture"
        )
        .into());
    }
    if size[0] == 0 || size[1] == 0 || size[0] != size[1].saturating_mul(2) {
        return Err(format!(
            "temporal correction {role} must be nonzero 2:1, got {} by {}",
            size[0], size[1]
        )
        .into());
    }
    Ok(size)
}

#[cfg(test)]
fn shader_source(fusion: bool, hardware_fusion: bool) -> String {
    format!(
        "{}\n{CORRECTION_WGSL}",
        draw_wgsl_with_fusion_mode(fusion, hardware_fusion)
    )
}

const CORRECTION_WGSL: &str = r#"
@group(0) @binding(6) var correction_current: texture_2d<f32>;
@group(0) @binding(7) var correction_filtered: texture_2d<f32>;
@group(0) @binding(8) var correction_sampler: sampler;

fn correction_body_uv(body_unscaled: vec3<f32>) -> vec2<f32> {
  let body = normalize(body_unscaled);
  var theta = atan2(body.x, -body.z);
  if theta < 0.0 { theta += TYPE2_TAU; }
  return vec2<f32>(theta / TYPE2_TAU,
    acos(clamp(body.y, -1.0, 1.0)) / TYPE2_PI);
}

fn corrected_color(gamma_unquantized: vec3<f32>, body: vec3<f32>) -> vec4<f32> {
  // The accepted review first wrote its direct viewport to Rgba8Unorm. Keep
  // that high-detail boundary explicit. pack/unpack is the shader's defined
  // RGB8 quantizer; attachment conversion may round a half-code differently.
  let high = unpack4x8unorm(pack4x8unorm(vec4<f32>(gamma_unquantized, 1.0))).rgb;
  let uv = correction_body_uv(body);
  let current = textureSample(correction_current, correction_sampler, uv).rgb;
  let filtered = textureSample(correction_filtered, correction_sampler, uv).rgb;
  let gamma = clamp(high + (filtered - current), vec3<f32>(0.0), vec3<f32>(1.0));
  let linear = select(
    gamma / 12.92,
    pow((gamma + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)),
    gamma > vec3<f32>(0.04045),
  );
  return vec4<f32>(select(gamma, linear, reframe.linearize > 0.5), 1.0);
}

@fragment
fn corrected_fs(in: Type2VsOut) -> @location(0) vec4<f32> {
  let view = view_ray(in.uv);
  if view.w <= 0.0 { return vec4<f32>(0.0); }
  let body = reframe.view_to_body * view.xyz;
  let map = type2_mesh(body);
  if map.covered <= 0.5 { return vec4<f32>(0.0); }
  return corrected_color(type2_gamma_color(map).rgb, body);
}

struct CorrectedType2MeshOut {
  @builtin(position) position: vec4<f32>,
  @location(0) packed: vec4<f32>,
  @location(1) map_uv: vec2<f32>,
  @location(2) fusion_uv: vec2<f32>,
  @location(3) body: vec3<f32>,
};

@vertex
fn corrected_mesh_vs(@builtin(vertex_index) index: u32) -> CorrectedType2MeshOut {
  let cell = index / 6u;
  let offsets = array<vec2<i32>, 6>(
    vec2<i32>(0, 0), vec2<i32>(0, 1), vec2<i32>(1, 0),
    vec2<i32>(0, 1), vec2<i32>(1, 1), vec2<i32>(1, 0),
  );
  let at = vec2<i32>(i32(cell / 100u), i32(cell % 100u)) + offsets[index % 6u];
  let sphere = type2_position(at.x, at.y);
  let body = vec3<f32>(-sphere.x, sphere.y, -sphere.z);
  let view = transpose(reframe.view_to_body) * body;
  var out: CorrectedType2MeshOut;
  out.position = vec4<f32>(view.x / reframe.screen.half_extent,
    -view.y * reframe.screen.aspect / reframe.screen.half_extent,
    view.z - 0.0001, view.z);
  out.map_uv = type2_varying(at.x, at.y);
  out.fusion_uv = type2_fusion_varying(at.x, at.y);
  out.packed = type2_sample4(out.map_uv);
  out.body = body;
  return out;
}

@fragment
fn corrected_mesh_fs(in: CorrectedType2MeshOut) -> @location(0) vec4<f32> {
  let map = Type2Sample(in.packed, type2_sample1(in.map_uv), 1.0, in.fusion_uv);
  return corrected_color(type2_gamma_color(map).rgb, in.body);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

    #[test]
    fn corrected_shaders_validate_and_keep_three_bind_groups() {
        for (fusion, hardware) in [(false, false), (true, false), (true, true)] {
            let source = shader_source(fusion, hardware);
            let module = wgpu::naga::front::wgsl::parse_str(&source).unwrap_or_else(|error| {
                panic!("corrected direct shader ({fusion}, {hardware}) did not parse: {error}")
            });
            Validator::new(ValidationFlags::all(), Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|error| {
                    panic!(
                        "corrected direct shader ({fusion}, {hardware}) did not validate: {error}"
                    )
                });
            assert!(module.global_variables.iter().all(|(_, global)| {
                global
                    .binding
                    .as_ref()
                    .is_none_or(|binding| binding.group < 3)
            }));
        }
    }

    #[test]
    fn correction_is_direct_rgb8_plus_one_uninterpolated_residual() {
        assert!(CORRECTION_WGSL.contains("unpack4x8unorm(pack4x8unorm"));
        assert!(CORRECTION_WGSL.contains("high + (filtered - current)"));
        assert_eq!(
            CORRECTION_WGSL
                .matches("textureSample(correction_current")
                .count(),
            1
        );
        assert_eq!(
            CORRECTION_WGSL
                .matches("textureSample(correction_filtered")
                .count(),
            1
        );
        assert!(!CORRECTION_WGSL.contains("mix(current, filtered"));
    }

    #[test]
    fn correction_boundary_modes_are_explicit() {
        let descriptor = wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        };
        assert_eq!(descriptor.address_mode_u, wgpu::AddressMode::Repeat);
        assert_eq!(descriptor.address_mode_v, wgpu::AddressMode::ClampToEdge);
        assert!(CORRECTION_WGSL.contains("atan2(body.x, -body.z)"));
        assert!(CORRECTION_WGSL.contains("acos(clamp(body.y, -1.0, 1.0))"));
    }
}
