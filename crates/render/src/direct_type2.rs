//! Direct GPU consumer for Studio's selected ONE X2 type-2 resources.
//!
//! The packed UV and alpha grids stay at their native 200 by 100 shape. The
//! fragment reconstructs the readable [`crate::map_oracle::Mesh`] from the
//! current `Reframe` body ray, rather than accepting a dense output-sized map.

use std::num::NonZeroU64;
use std::sync::Arc;

use kjerag_media::Frames;

use crate::dmabuf;
use crate::flow::one_xs::gpu_context::OneXsGpuContext;
use crate::flow::one_xs::one_xs_belt_gpu::{
    ImportedOneXsSource, ResidentImportedFront, ResidentSourceBinder, ResidentSourceCapture,
    ResidentSourceIdentity, SourceTextures,
};
use crate::projection;
use crate::studio_type2::{ALPHA_BYTES, MAP_HEIGHT, MAP_WIDTH, OneXsMapFrame, PACKED_BYTES};
use crate::{Fallible, FrameStamp, MAX_LENSES, Planes};

/// One exact decoded ONE X2 pair imported for resident processing and drawing.
///
/// This is deliberately private to the selected resident path. Its only
/// production constructor consumes the decoder owner and imports both of that
/// owner's lens descriptors before publishing the aggregate. There is no
/// constructor from planes or a frame stamp, so another allocation cannot be
/// associated with already-imported textures afterward.
///
/// Field order is load-bearing Rust drop order: the imported textures are
/// released before the decoder surfaces they alias. The context and opaque
/// session identity are last and have no source allocation to release.
/// The generic parameters exist solely so the unit test can exercise that exact
/// struct's drop order without fabricating decoder allocations or dmabufs.
#[allow(dead_code)]
pub(crate) struct ImportedOneXsPicture<
    P = [Planes; 2],
    F = Arc<Frames>,
    C = OneXsGpuContext,
    S = ResidentSourceIdentity,
> {
    planes: P,
    frames: F,
    context: C,
    session: S,
}

#[allow(dead_code)]
impl ImportedOneXsPicture {
    fn import_for_capture(capture: &ResidentSourceCapture, frames: Arc<Frames>) -> Fallible<Self> {
        let context = capture.import_context();
        let device = context.device();
        let [a, b] = exact_one_xs_lenses(&frames.lenses)?;
        // Array construction drops an already-imported A if B refuses. The
        // consumed `frames` parameter remains alive until that cleanup ends.
        let planes = [
            dmabuf::import(device, a.descriptor(), frames.size)?,
            dmabuf::import(device, b.descriptor(), frames.size)?,
        ];
        Ok(Self {
            planes,
            frames,
            context: context.clone(),
            session: capture.source_identity(),
        })
    }

    /// Consume this exact imported pair into the existing resident
    /// parent/geometry/belt front half.
    ///
    /// Context and frame identity are not caller inputs. They come from this
    /// sealed owner and are checked before reservation or command encoding.
    /// The temporary texture clones are made only so Rust can end the field
    /// borrow before the complete owner moves into the submission lease; they
    /// name the same two imported luma allocations and never escape.
    pub(crate) fn submit_resident_front(
        self,
        capture: &ResidentSourceCapture,
    ) -> Fallible<ResidentImportedFront> {
        capture.submit_imported(self)
    }

    /// Seal one draw-private Reframe allocation and picture binding around
    /// this exact imported pair. A later queue write for another pass cannot
    /// change what this binding samples.
    pub(crate) fn prepare_resident_draw(
        &self,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        reframe: &crate::Reframe,
    ) -> ImportedOneXsDrawBinding {
        let usage = wgpu::BufferUsages::UNIFORM
            | wgpu::BufferUsages::COPY_DST
            | if cfg!(test) {
                wgpu::BufferUsages::COPY_SRC
            } else {
                wgpu::BufferUsages::empty()
            };
        let uniforms = self
            .context
            .device()
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("ONE X2 resident draw-private uniforms"),
                size: std::mem::size_of::<crate::Reframe>() as u64,
                usage,
                mapped_at_creation: false,
            });
        self.context
            .queue()
            .write_buffer(&uniforms, 0, reframe.bytes());
        let picture = bind_picture(
            self.context.device(),
            layout,
            &uniforms,
            [&self.planes[0], &self.planes[1]],
            sampler,
        );
        ImportedOneXsDrawBinding {
            picture,
            _uniforms: uniforms,
            rectilinear: reframe.is_rectilinear(),
        }
    }

    /// Append source-band sampling while this exact imported picture remains
    /// inside its resident submission lease. Neither the decoder owner nor a
    /// raw plane handle crosses this boundary.
    pub(crate) fn encode_fusion_inputs(
        &self,
        context: &OneXsGpuContext,
        expected: &FrameStamp,
        encoder: &mut wgpu::CommandEncoder,
        sampler: &crate::image_fusion::sample::FusionInputPipeline,
        packed: &wgpu::Buffer,
    ) -> Fallible<ResidentGpuBandInputs> {
        self.context.ensure_same(context)?;
        self.ensure_resident_frame(expected)?;
        if !context.is_worker_thread() {
            return Err("image fusion source sampling was called outside its stitch worker".into());
        }
        if packed.size() != PACKED_BYTES as u64 {
            return Err(format!(
                "image fusion packed source map is {} bytes, expected {PACKED_BYTES}",
                packed.size()
            )
            .into());
        }

        // This pass reads only source size, plane encoding/range and colour
        // matrix from the uniform. Camera/view fields are deliberately empty:
        // the frame-bound packed map has already performed projection.
        let reframe = crate::Reframe::new(
            &[],
            self.frames.size,
            crate::Camera::default(),
            crate::Held::default(),
            1.0,
            false,
            crate::Sampling::Bilinear,
        )
        .with_samples(self.frames.samples);
        let uniforms = context.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("image fusion capture-owned source metadata"),
            size: std::mem::size_of::<crate::Reframe>() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        {
            let mut mapped = uniforms.slice(..).get_mapped_range_mut();
            mapped.copy_from_slice(reframe.bytes());
        }
        uniforms.unmap();
        let picture = sampler.bind_source(
            context.device(),
            &uniforms,
            [&self.planes[0], &self.planes[1]],
        );
        let inputs = sampler.encode(context.device(), encoder, &picture, packed);
        Ok(ResidentGpuBandInputs {
            inputs,
            _picture: picture,
            _uniforms: uniforms,
        })
    }

    pub(crate) fn draw_resident_binding(
        &self,
        pipeline: &DirectType2Pipeline,
        binding: &ImportedOneXsDrawBinding,
        map: &wgpu::BindGroup,
        fusion: Option<&wgpu::BindGroup>,
        pass: &mut wgpu::RenderPass<'_>,
    ) {
        assert_eq!(
            pipeline.fusion_layout().is_some(),
            fusion.is_some(),
            "resident direct map pipeline and photometric binding presence differ"
        );
        if let Some(fusion) = fusion {
            pass.set_bind_group(2, fusion, &[]);
        }
        if binding.rectilinear {
            pipeline.draw_mesh(pass, &binding.picture, map);
        } else {
            pipeline.draw(pass, &binding.picture, map);
        }
    }

    pub(crate) fn ensure_resident_frame(&self, frame: &FrameStamp) -> Fallible<()> {
        if &self.frames.stamp() != frame {
            return Err("ONE X2 final map names a different imported picture allocation".into());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn resident_test_owner(
        context: &OneXsGpuContext,
        session: ResidentSourceIdentity,
        frame: FrameStamp,
    ) -> Self {
        let device = context.device();
        let texture = |label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let planes = [
            Planes {
                luma: texture("ONE X2 resident test source A luma"),
                chroma: texture("ONE X2 resident test source A chroma"),
            },
            Planes {
                luma: texture("ONE X2 resident test source B luma"),
                chroma: texture("ONE X2 resident test source B chroma"),
            },
        ];
        Self {
            planes,
            frames: Arc::new(Frames::empty_for_test(frame, crate::Size::new(1, 1))),
            context: context.clone(),
            session,
        }
    }
}

/// Per-render-pass picture resources. The bind group is dropped before its
/// uniform allocation; the enclosing retired payload keeps both alive through
/// exact submitted-work completion.
pub(crate) struct ImportedOneXsDrawBinding {
    picture: wgpu::BindGroup,
    _uniforms: wgpu::Buffer,
    rectilinear: bool,
}

/// Capture-owned source-band inputs and every binding used to encode them.
/// The imported picture itself remains outside this value in the submission
/// lease; this owner only prevents its GPU bindings from being dropped early.
pub(crate) struct ResidentGpuBandInputs {
    inputs: crate::image_fusion::sample::GpuBandInputs,
    _picture: wgpu::BindGroup,
    _uniforms: wgpu::Buffer,
}

impl ResidentGpuBandInputs {
    pub(crate) fn bands(&self) -> [&wgpu::Buffer; 2] {
        self.inputs.bands()
    }

    pub(crate) fn invalid(&self) -> &wgpu::Buffer {
        self.inputs.invalid()
    }
}

#[cfg(test)]
impl ImportedOneXsDrawBinding {
    pub(crate) fn uniform_for_test(&self) -> wgpu::Buffer {
        self._uniforms.clone()
    }
}

impl ImportedOneXsSource for ImportedOneXsPicture {
    fn ensure_resident_context(&self, context: &OneXsGpuContext) -> Fallible<()> {
        self.context.ensure_same(context)
    }

    fn ensure_resident_session(&self, session: &ResidentSourceIdentity) -> Fallible<()> {
        self.session.ensure_matches(session)
    }

    fn resident_frame(&self) -> FrameStamp {
        self.frames.stamp()
    }

    fn submit_with(self, binder: ResidentSourceBinder<'_>) -> Fallible<ResidentImportedFront> {
        let luma = [self.planes[0].luma.clone(), self.planes[1].luma.clone()];
        binder.submit_exact(
            SourceTextures {
                a: &luma[0],
                b: &luma[1],
            },
            self,
        )
    }
}

#[allow(dead_code)]
impl ResidentSourceCapture {
    /// Import one exact decoder-owned pair under this capture's unforgeable
    /// resident identity. The resulting owner can return only to this session.
    pub(crate) fn import_picture(&self, frames: Arc<Frames>) -> Fallible<ImportedOneXsPicture> {
        ImportedOneXsPicture::import_for_capture(self, frames)
    }
}

fn exact_one_xs_lenses<T>(lenses: &[T]) -> Fallible<[&T; 2]> {
    match lenses {
        [a, b] => Ok([a, b]),
        _ => Err(format!(
            "ONE X2 source import requires exactly 2 lens frames, got {}",
            lenses.len()
        )
        .into()),
    }
}

/// Immutable direct type-2 shader, pipeline and native-map layout.
///
/// A capture can share this object across every installed result. Map and
/// source ownership live in separate per-result bindings, so replacing one
/// result cannot mutate the resources sampled by an older in-flight draw.
pub(crate) struct DirectType2Pipeline {
    device: wgpu::Device,
    pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    picture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    map_layout: wgpu::BindGroupLayout,
    fusion_layout: Option<wgpu::BindGroupLayout>,
    fusion_sampler: Option<wgpu::Sampler>,
}

impl DirectType2Pipeline {
    #[cfg(test)]
    pub(crate) fn new(
        device: &wgpu::Device,
        picture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self::with_fusion(device, picture_layout, format, false)
    }

    pub(crate) fn new_resident_fused(
        device: &wgpu::Device,
        picture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> Self {
        Self::with_fusion(device, picture_layout, format, true)
    }

    fn with_fusion(
        device: &wgpu::Device,
        picture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        fusion: bool,
    ) -> Self {
        let hardware_fusion = fusion
            && device
                .features()
                .contains(wgpu::Features::FLOAT32_FILTERABLE);
        let map_layout = layout(
            device,
            wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 direct type-2 map"),
            source: wgpu::ShaderSource::Wgsl(
                draw_wgsl_with_fusion_mode(fusion, hardware_fusion).into(),
            ),
        });
        let fusion_layout = fusion.then(|| {
            let mut entries = (0..2)
                .map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float {
                            filterable: hardware_fusion,
                        },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                })
                .collect::<Vec<_>>();
            if hardware_fusion {
                entries.push(wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                });
            }
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("per-lens image fusion ratios"),
                entries: &entries,
            })
        });
        let fusion_sampler = hardware_fusion.then(|| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("image fusion bilinear sampler"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            })
        });
        let mut layouts = vec![picture_layout, &map_layout];
        layouts.extend(fusion_layout.as_ref());
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 direct type-2 map"),
            bind_group_layouts: &layouts,
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ONE X2 direct type-2 map"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ONE X2 native sphere rasterization"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("mesh_vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("mesh_fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
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
            pipeline,
            mesh_pipeline,
            picture_layout: picture_layout.clone(),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            map_layout,
            fusion_layout,
            fusion_sampler,
        }
    }

    pub(crate) fn prepare_resident_picture(
        &self,
        source: &ImportedOneXsPicture,
        reframe: &crate::Reframe,
    ) -> ImportedOneXsDrawBinding {
        source.prepare_resident_draw(&self.picture_layout, &self.sampler, reframe)
    }

    pub(crate) fn map_layout(&self) -> &wgpu::BindGroupLayout {
        &self.map_layout
    }

    pub(crate) fn fusion_layout(&self) -> Option<&wgpu::BindGroupLayout> {
        self.fusion_layout.as_ref()
    }

    pub(crate) fn bind_fusion_textures(
        &self,
        device: &wgpu::Device,
        ratios: [&wgpu::Texture; 2],
    ) -> Fallible<wgpu::BindGroup> {
        // The resident producer's Output is sealed to its capture context.
        // wgpu exposes no owning device from a Texture at this boundary.
        if self.device != *device {
            return Err(
                "ONE X2 image fusion binding belongs to a different graphics device".into(),
            );
        }
        let Some(layout) = self.fusion_layout() else {
            return Err("ONE X2 direct type-2 pipeline has image fusion disabled".into());
        };
        for (lens, ratio) in ratios.iter().enumerate() {
            if ratio.size()
                != (wgpu::Extent3d {
                    width: MAP_WIDTH as u32,
                    height: MAP_HEIGHT as u32,
                    depth_or_array_layers: 1,
                })
            {
                return Err(format!(
                    "ONE X2 lens {lens} image fusion texture is {} by {} by {}, expected {MAP_WIDTH} by {MAP_HEIGHT} by 1",
                    ratio.width(),
                    ratio.height(),
                    ratio.depth_or_array_layers()
                )
                .into());
            }
            if ratio.format() != wgpu::TextureFormat::Rgba32Float
                || ratio.dimension() != wgpu::TextureDimension::D2
                || ratio.sample_count() != 1
            {
                return Err(format!(
                    "ONE X2 lens {lens} image fusion texture must be single-sampled 2D Rgba32Float"
                )
                .into());
            }
            if !ratio.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING) {
                return Err(format!(
                    "ONE X2 lens {lens} image fusion texture is not GPU sampleable"
                )
                .into());
            }
        }
        let views = ratios.map(|ratio| ratio.create_view(&Default::default()));
        let mut entries = views
            .iter()
            .enumerate()
            .map(|(binding, view)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: wgpu::BindingResource::TextureView(view),
            })
            .collect::<Vec<_>>();
        if let Some(sampler) = self.fusion_sampler.as_ref() {
            entries.push(wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            });
        }
        Ok(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("resident frame-bound image fusion ratio pair"),
            layout,
            entries: &entries,
        }))
    }

    pub(crate) fn ensure_device(&self, context: &OneXsGpuContext) -> Fallible<()> {
        if self.device == *context.device() {
            Ok(())
        } else {
            Err("ONE X2 direct type-2 pipeline belongs to a different graphics device".into())
        }
    }

    pub(crate) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        picture: &wgpu::BindGroup,
        map: &wgpu::BindGroup,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, picture, &[]);
        pass.set_bind_group(1, map, &[]);
        pass.draw(0..3, 0..1);
    }

    fn draw_mesh(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        picture: &wgpu::BindGroup,
        map: &wgpu::BindGroup,
    ) {
        pass.set_pipeline(&self.mesh_pipeline);
        pass.set_bind_group(0, picture, &[]);
        pass.set_bind_group(1, map, &[]);
        pass.draw(0..(100 * 50 * 6), 0..1);
    }
}

/// Build the exact picture group from resources owned by one draw result.
///
/// The returned bind group is inseparable from the passed planes only when
/// its caller retains those planes. The selected CPU path continues to retain
/// them in Scene; the resident path does not yet have an authenticated owner
/// for that association.
pub(crate) fn bind_picture(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    lenses: [&Planes; MAX_LENSES],
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let views: Vec<wgpu::TextureView> = lenses
        .iter()
        .flat_map(|planes| [&planes.luma, &planes.chroma])
        .map(|texture| texture.create_view(&Default::default()))
        .collect();
    let mut entries = vec![wgpu::BindGroupEntry {
        binding: 0,
        resource: uniforms.as_entire_binding(),
    }];
    entries.extend(
        views
            .iter()
            .enumerate()
            .map(|(plane, view)| wgpu::BindGroupEntry {
                binding: 1 + plane as u32,
                resource: wgpu::BindingResource::TextureView(view),
            }),
    );
    entries.push(wgpu::BindGroupEntry {
        binding: 5,
        resource: wgpu::BindingResource::Sampler(sampler),
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene"),
        layout,
        entries: &entries,
    })
}

/// CPU-uploaded native map binding retained by the selected shipping path.
struct DirectType2CpuBinding {
    read: wgpu::BindGroup,
    packed: wgpu::Buffer,
    alpha: wgpu::Buffer,
    bound_frame: Option<FrameStamp>,
    fusion: Option<FusionBinding>,
}

/// Explicit replay resources. The bind group drops before its textures.
struct FusionBinding {
    read: wgpu::BindGroup,
    ratios: [wgpu::Texture; 2],
}

impl FusionBinding {
    fn new(device: &wgpu::Device, pipeline: &DirectType2Pipeline) -> Self {
        let ratios = std::array::from_fn(|_| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("image fusion ratio map"),
                size: wgpu::Extent3d {
                    width: MAP_WIDTH as u32,
                    height: MAP_HEIGHT as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        });
        let read = pipeline
            .bind_fusion_textures(device, [&ratios[0], &ratios[1]])
            .expect("new replay fusion textures match their pipeline");
        Self { read, ratios }
    }
}

impl DirectType2CpuBinding {
    fn new(device: &wgpu::Device, pipeline: &DirectType2Pipeline) -> Self {
        let packed = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 native packed map"),
            size: PACKED_BYTES as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let alpha = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 native alpha map"),
            size: ALPHA_BYTES as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 native type-2 resources"),
            layout: pipeline.map_layout(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: packed.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: alpha.as_entire_binding(),
                },
            ],
        });
        Self {
            read,
            packed,
            alpha,
            bound_frame: None,
            fusion: pipeline
                .fusion_layout
                .as_ref()
                .map(|_| FusionBinding::new(device, pipeline)),
        }
    }

    fn upload(&mut self, queue: &wgpu::Queue, map: &OneXsMapFrame) {
        queue.write_buffer(&self.packed, 0, map.packed().bytes());
        queue.write_buffer(&self.alpha, 0, map.alpha().bytes());
        match (&self.fusion, map.fusion()) {
            (Some(binding), Some(pair)) => {
                for (texture, ratio) in binding.ratios.iter().zip([&pair.left, &pair.right]) {
                    queue.write_texture(
                        texture.as_image_copy(),
                        ratio.bytes(),
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(
                                (MAP_WIDTH * std::mem::size_of::<[f32; 4]>()) as u32,
                            ),
                            rows_per_image: Some(MAP_HEIGHT as u32),
                        },
                        texture.size(),
                    );
                }
            }
            (None, None) => {}
            _ => panic!("direct map pipeline and photometric map presence differ"),
        }
        self.bound_frame = Some(map.frame().clone());
    }
}

pub(crate) struct DirectMapDraw {
    pipeline: Arc<DirectType2Pipeline>,
    binding: DirectType2CpuBinding,
}

impl DirectMapDraw {
    pub(crate) fn new(
        device: &wgpu::Device,
        picture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
        fusion: bool,
    ) -> Self {
        let pipeline = Arc::new(DirectType2Pipeline::with_fusion(
            device,
            picture_layout,
            format,
            fusion,
        ));
        let binding = DirectType2CpuBinding::new(device, &pipeline);
        Self { pipeline, binding }
    }

    pub(crate) fn upload(&mut self, queue: &wgpu::Queue, map: &OneXsMapFrame) {
        self.binding.upload(queue, map);
    }

    pub(crate) fn bound_frame(&self) -> Option<&FrameStamp> {
        self.binding.bound_frame.as_ref()
    }

    pub(crate) fn has_fusion(&self) -> bool {
        self.binding.fusion.is_some()
    }

    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, picture: &wgpu::BindGroup) {
        if let Some(fusion) = &self.binding.fusion {
            pass.set_bind_group(2, &fusion.read, &[]);
        }
        self.pipeline.draw(pass, picture, &self.binding.read);
    }

    #[cfg(test)]
    pub(crate) fn draw_mesh_for_test(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        picture: &wgpu::BindGroup,
    ) {
        if let Some(fusion) = &self.binding.fusion {
            pass.set_bind_group(2, &fusion.read, &[]);
        }
        self.pipeline.draw_mesh(pass, picture, &self.binding.read);
    }
}

fn layout(device: &wgpu::Device, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayout {
    let storage = |binding, bytes| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(bytes),
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ONE X2 native type-2 resources"),
        entries: &[
            storage(0, PACKED_BYTES as u64),
            storage(1, ALPHA_BYTES as u64),
        ],
    })
}

/// Shared source metadata and texture declarations, without final-map bindings
/// or draw entry points. Compute consumers can name their own map resources.
pub(crate) fn source_wgsl() -> String {
    format!("{}\n{SOURCE_BINDINGS}", projection::wgsl())
}

#[cfg(test)]
pub(crate) fn draw_wgsl() -> String {
    draw_wgsl_with_fusion(false)
}

#[cfg(test)]
fn draw_wgsl_with_fusion(fusion: bool) -> String {
    draw_wgsl_with_fusion_mode(fusion, false)
}

fn draw_wgsl_with_fusion_mode(fusion: bool, hardware_fusion: bool) -> String {
    let correction = if fusion {
        if hardware_fusion {
            crate::image_fusion::FILTERED_WGSL
        } else {
            crate::image_fusion::WGSL
        }
    } else {
        // An actual bypass, so disabled mode does not round or clamp RGB and
        // introduces no resource binding or texture read.
        "fn type2_correct(color: vec3<f32>, uv: vec2<f32>, lens: u32) -> vec3<f32> { return color; }"
    };
    format!("{}\n{}\n{correction}\n{DRAW}", source_wgsl(), map_wgsl())
}

pub(crate) fn map_wgsl() -> &'static str {
    MAP
}

const MAP: &str = r#"
@group(1) @binding(0) var<storage, read> type2_packed: array<vec4<f32>>;
@group(1) @binding(1) var<storage, read> type2_alpha: array<f32>;

const TYPE2_MAP_W = 200i;
const TYPE2_MAP_H = 100i;
const TYPE2_SLICES = 100i;
const TYPE2_STACKS = 50i;
const TYPE2_PI = 3.14159265358979323846;
const TYPE2_TAU = 6.28318530717958647692;

struct Type2Sample {
  packed: vec4<f32>,
  alpha: f32,
  covered: f32,
  fusion_uv: vec2<f32>,
};

fn type2_map_index(x: i32, y: i32) -> u32 {
  let periodic_x = ((x % TYPE2_MAP_W) + TYPE2_MAP_W) % TYPE2_MAP_W;
  let pole_y = clamp(y, 0, TYPE2_MAP_H - 1);
  return u32(pole_y * TYPE2_MAP_W + periodic_x);
}

fn type2_bilinear_weights(uv: vec2<f32>) -> vec4<f32> {
  let p = uv * vec2<f32>(f32(TYPE2_MAP_W), f32(TYPE2_MAP_H));
  let f = fract(p);
  let w = select(f - vec2<f32>(0.5), f + vec2<f32>(0.5), f < vec2<f32>(0.5));
  return vec4<f32>(floor(p - select(vec2<f32>(0.0), vec2<f32>(1.0), f < vec2<f32>(0.5))), w);
}

fn type2_sample4(uv: vec2<f32>) -> vec4<f32> {
  let q = type2_bilinear_weights(uv);
  let i = vec2<i32>(q.xy);
  let a = type2_packed[type2_map_index(i.x, i.y)];
  let b = type2_packed[type2_map_index(i.x + 1, i.y)];
  let c = type2_packed[type2_map_index(i.x, i.y + 1)];
  let d = type2_packed[type2_map_index(i.x + 1, i.y + 1)];
  return mix(mix(a, b, q.z), mix(c, d, q.z), q.w);
}

fn type2_sample1(uv: vec2<f32>) -> f32 {
  let q = type2_bilinear_weights(uv);
  let i = vec2<i32>(q.xy);
  let a = type2_alpha[type2_map_index(i.x, i.y)];
  let b = type2_alpha[type2_map_index(i.x + 1, i.y)];
  let c = type2_alpha[type2_map_index(i.x, i.y + 1)];
  let d = type2_alpha[type2_map_index(i.x + 1, i.y + 1)];
  return mix(mix(a, b, q.z), mix(c, d, q.z), q.w);
}

fn type2_position(row: i32, col: i32) -> vec3<f32> {
  let phi = TYPE2_PI * f32(row) / f32(TYPE2_STACKS);
  let theta = TYPE2_TAU * f32(col) / f32(TYPE2_SLICES);
  return vec3<f32>(-sin(phi) * sin(theta), cos(phi), sin(phi) * cos(theta));
}

fn type2_varying(row: i32, col: i32) -> vec2<f32> {
  return vec2<f32>(
    (f32(col) / f32(TYPE2_SLICES) + 0.5) + 0.5 / f32(TYPE2_MAP_W),
    ((f32(row) / f32(TYPE2_STACKS) * 99.0) + 0.5) / f32(TYPE2_MAP_H),
  );
}

fn type2_fusion_varying(row: i32, col: i32) -> vec2<f32> {
  return vec2<f32>(f32(col) / f32(TYPE2_SLICES) + 0.5,
    f32(row) / f32(TYPE2_STACKS));
}

// Watertight dominant-axis ray/triangle intersection against a ray from the
// sphere centre. Shared edges evaluate through the same two-product edge
// function in opposite order, so f32 rounding cannot leave a crack between
// adjacent triangles. xyz are the barycentrics in vertex order and w is
// exact admission; there is no epsilon or extrapolation.
fn type2_triangle(ray: vec3<f32>, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>) -> vec4<f32> {
  let absolute = abs(ray);
  var kz = 0u;
  if absolute.y > absolute.x { kz = 1u; }
  if absolute.z > absolute[kz] { kz = 2u; }
  var kx = (kz + 1u) % 3u;
  var ky = (kx + 1u) % 3u;
  if ray[kz] < 0.0 {
    let swap = kx;
    kx = ky;
    ky = swap;
  }
  let sx = ray[kx] / ray[kz];
  let sy = ray[ky] / ray[kz];
  let sz = 1.0 / ray[kz];
  let ax = a[kx] - sx * a[kz];
  let ay = a[ky] - sy * a[kz];
  let bx = b[kx] - sx * b[kz];
  let by = b[ky] - sy * b[kz];
  let cx = c[kx] - sx * c[kz];
  let cy = c[ky] - sy * c[kz];
  let wa = cx * by - cy * bx;
  let wb = ax * cy - ay * cx;
  let wc = bx * ay - by * ax;
  let has_negative = wa < 0.0 || wb < 0.0 || wc < 0.0;
  let has_positive = wa > 0.0 || wb > 0.0 || wc > 0.0;
  if has_negative && has_positive { return vec4<f32>(0.0); }
  let determinant = wa + wb + wc;
  if determinant == 0.0 { return vec4<f32>(0.0); }
  let distance_numerator = wa * (sz * a[kz]) + wb * (sz * b[kz]) + wc * (sz * c[kz]);
  if distance_numerator * determinant <= 0.0 { return vec4<f32>(0.0); }
  let inverse = 1.0 / determinant;
  return vec4<f32>(wa * inverse, wb * inverse, wc * inverse, 1.0);
}

fn type2_cell(ray: vec3<f32>, row: i32, col_unwrapped: i32) -> Type2Sample {
  var out: Type2Sample;
  let col = ((col_unwrapped % TYPE2_SLICES) + TYPE2_SLICES) % TYPE2_SLICES;
  let p00 = type2_position(row, col);
  let p01 = type2_position(row, col + 1);
  let p10 = type2_position(row + 1, col);
  let p11 = type2_position(row + 1, col + 1);
  let uv00 = type2_varying(row, col);
  let uv01 = type2_varying(row, col + 1);
  let uv10 = type2_varying(row + 1, col);
  let uv11 = type2_varying(row + 1, col + 1);
  let fusion00 = type2_fusion_varying(row, col);
  let fusion01 = type2_fusion_varying(row, col + 1);
  let fusion10 = type2_fusion_varying(row + 1, col);
  let fusion11 = type2_fusion_varying(row + 1, col + 1);

  var weights = type2_triangle(ray, p00, p01, p10);
  var uv = weights.x * uv00 + weights.y * uv01 + weights.z * uv10;
  var fusion_uv = weights.x * fusion00 + weights.y * fusion01 + weights.z * fusion10;
  var packed = weights.x * type2_sample4(uv00) + weights.y * type2_sample4(uv01) + weights.z * type2_sample4(uv10);
  if weights.w == 0.0 {
    weights = type2_triangle(ray, p01, p11, p10);
    uv = weights.x * uv01 + weights.y * uv11 + weights.z * uv10;
    fusion_uv = weights.x * fusion01 + weights.y * fusion11 + weights.z * fusion10;
    packed = weights.x * type2_sample4(uv01) + weights.y * type2_sample4(uv11) + weights.z * type2_sample4(uv10);
  }
  if weights.w > 0.0 {
    out.packed = packed;
    out.alpha = type2_sample1(uv);
    out.covered = 1.0;
    out.fusion_uv = fusion_uv;
  }
  return out;
}

fn type2_mesh(body: vec3<f32>) -> Type2Sample {
  var empty: Type2Sample;
  let ray = normalize(vec3<f32>(-body.x, body.y, -body.z));
  let row = i32(clamp(floor(acos(clamp(ray.y, -1.0, 1.0)) * f32(TYPE2_STACKS) / TYPE2_PI), 0.0, f32(TYPE2_STACKS - 1)));
  var theta = atan2(-ray.x, ray.z);
  if theta < 0.0 { theta += TYPE2_TAU; }
  let col = i32(floor(theta * f32(TYPE2_SLICES) / TYPE2_TAU)) % TYPE2_SLICES;
  let neighbours = array<vec2<i32>, 9>(
    vec2<i32>(0, 0), vec2<i32>(-1, -1), vec2<i32>(-1, 0),
    vec2<i32>(-1, 1), vec2<i32>(0, -1), vec2<i32>(0, 1),
    vec2<i32>(1, -1), vec2<i32>(1, 0), vec2<i32>(1, 1),
  );
  for (var candidate = 0u; candidate < 9u; candidate += 1u) {
    let at = vec2<i32>(row, col) + neighbours[candidate];
    if at.x < 0 || at.x >= TYPE2_STACKS { continue; }
    let found = type2_cell(ray, at.x, at.y);
    if found.covered > 0.5 { return found; }
  }
  return empty;
}
"#;

const SOURCE_BINDINGS: &str = r#"
@group(0) @binding(1) var type2_luma0: texture_2d<f32>;
@group(0) @binding(2) var type2_chroma0: texture_2d<f32>;
@group(0) @binding(3) var type2_luma1: texture_2d<f32>;
@group(0) @binding(4) var type2_chroma1: texture_2d<f32>;
@group(0) @binding(5) var type2_sampler: sampler;
"#;

const DRAW: &str = r#"
struct Type2VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) index: u32) -> Type2VsOut {
  let x = f32((index << 1u) & 2u);
  let y = f32(index & 2u);
  var out: Type2VsOut;
  out.uv = vec2<f32>(x, y);
  out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  return out;
}

fn type2_atlas_load(a: texture_2d<f32>, b: texture_2d<f32>, p: vec2<i32>) -> vec4<f32> {
  let dims = textureDimensions(a);
  let x = clamp(p.x, 0, i32(2u * dims.x) - 1);
  let y = clamp(p.y, 0, i32(dims.y) - 1);
  if x < i32(dims.x) { return textureLoad(a, vec2<i32>(x, y), 0); }
  return textureLoad(b, vec2<i32>(x - i32(dims.x), y), 0);
}

fn type2_atlas_linear(a: texture_2d<f32>, b: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
  let dims = textureDimensions(a);
  let p = uv * vec2<f32>(f32(2u * dims.x), f32(dims.y)) - vec2<f32>(0.5);
  // The atlas is two imported textures, not a physically joined image.
  // Hardware filtering is valid inside either lens, including the outside
  // clamp edges. Only a footprint straddling the join needs four loads.
  if p.x <= f32(dims.x - 1u) {
    return textureSampleLevel(a, type2_sampler, vec2<f32>(uv.x * 2.0, uv.y), 0.0);
  }
  if p.x >= f32(dims.x) {
    return textureSampleLevel(b, type2_sampler, vec2<f32>(uv.x * 2.0 - 1.0, uv.y), 0.0);
  }
  let base = vec2<i32>(floor(p));
  let f = fract(p);
  let top = mix(type2_atlas_load(a, b, base), type2_atlas_load(a, b, base + vec2<i32>(1, 0)), f.x);
  let bottom = mix(type2_atlas_load(a, b, base + vec2<i32>(0, 1)), type2_atlas_load(a, b, base + vec2<i32>(1, 1)), f.x);
  return mix(top, bottom, f.y);
}

fn type2_box(a: texture_2d<f32>, b: texture_2d<f32>, uv: vec2<f32>, logical: vec2<f32>) -> vec4<f32> {
  let box_size = vec2<f32>(1.7881766557693481);
  let start = uv * logical - box_size * 0.5;
  let end = start + box_size;
  var sum = vec4<f32>(0.0);
  var area = 0.0;
  var y = floor(start.y);
  loop {
    if y >= end.y { break; }
    let low_y = max(y, start.y);
    let high_y = min(y + 2.0, end.y);
    var x = floor(start.x);
    loop {
      if x >= end.x { break; }
      let low_x = max(x, start.x);
      let high_x = min(x + 2.0, end.x);
      let cell_area = (high_x - low_x) * (high_y - low_y);
      let sample_uv = vec2<f32>((low_x + high_x) * 0.5 / logical.x, (low_y + high_y) * 0.5 / logical.y);
      sum += type2_atlas_linear(a, b, sample_uv) * cell_area;
      area += cell_area;
      x += 2.0;
    }
    y += 2.0;
  }
  return select(type2_atlas_linear(a, b, uv), sum / area, area > 0.0010000000474974513);
}

fn type2_ycbcr(uv: vec2<f32>) -> vec3<f32> {
  let source_size = vec2<f32>(reframe.frame_width, reframe.frame_height);
  let luma = type2_box(type2_luma0, type2_luma1, uv, source_size);
  let chroma = type2_box(type2_chroma0, type2_chroma1, uv, source_size * 0.5);
  let c = chroma.rg - vec2<f32>(0.50196081399917603);
  return source_rgb(luma.r, c);
}

fn type2_color(map: Type2Sample) -> vec4<f32> {
  var rgb: vec3<f32>;
  if map.alpha == 0.0 {
    rgb = type2_correct(type2_ycbcr(map.packed.zw), map.fusion_uv, 1u);
  } else if map.alpha == 1.0 {
    rgb = type2_correct(type2_ycbcr(map.packed.xy), map.fusion_uv, 0u);
  } else {
    let b = type2_correct(type2_ycbcr(map.packed.zw), map.fusion_uv, 1u);
    let a = type2_correct(type2_ycbcr(map.packed.xy), map.fusion_uv, 0u);
    rgb = mix(b, a, map.alpha);
  }
  let linear = select(
    rgb / 12.92,
    pow((rgb + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)),
    rgb > vec3<f32>(0.04045),
  );
  return vec4<f32>(select(rgb, linear, reframe.linearize > 0.5), 1.0);
}

@fragment
fn fs(in: Type2VsOut) -> @location(0) vec4<f32> {
  let view = view_ray(in.uv);
  if view.w <= 0.0 { return vec4<f32>(0.0); }
  let map = type2_mesh(reframe.view_to_body * view.xyz);
  if map.covered <= 0.5 { return vec4<f32>(0.0); }
  return type2_color(map);
}

struct Type2MeshOut {
  @builtin(position) position: vec4<f32>,
  @location(0) packed: vec4<f32>,
  @location(1) map_uv: vec2<f32>,
  @location(2) fusion_uv: vec2<f32>,
};

// The same native triangles and vertex UV law as type2_cell. On a flat
// perspective view, hardware rasterization performs the ray intersection
// and perspective-correct interpolation for us. Curved/ball views retain fs.
@vertex
fn mesh_vs(@builtin(vertex_index) index: u32) -> Type2MeshOut {
  let cell = index / 6u;
  let offsets = array<vec2<i32>, 6>(
    vec2<i32>(0, 0), vec2<i32>(0, 1), vec2<i32>(1, 0),
    vec2<i32>(0, 1), vec2<i32>(1, 1), vec2<i32>(1, 0),
  );
  let at = vec2<i32>(i32(cell / 100u), i32(cell % 100u)) + offsets[index % 6u];
  let sphere = type2_position(at.x, at.y);
  let body = vec3<f32>(-sphere.x, sphere.y, -sphere.z);
  let view = transpose(reframe.view_to_body) * body;
  var out: Type2MeshOut;
  out.position = vec4<f32>(view.x / reframe.screen.half_extent,
    -view.y * reframe.screen.aspect / reframe.screen.half_extent,
    view.z - 0.0001, view.z);
  out.map_uv = type2_varying(at.x, at.y);
  out.fusion_uv = type2_fusion_varying(at.x, at.y);
  out.packed = type2_sample4(out.map_uv);
  return out;
}

@fragment
fn mesh_fs(in: Type2MeshOut) -> @location(0) vec4<f32> {
  return type2_color(Type2Sample(in.packed, type2_sample1(in.map_uv), 1.0, in.fusion_uv));
}
"#;

#[cfg(test)]
mod sampling_tests;

#[cfg(test)]
mod fusion_tests;

#[cfg(test)]
mod color_tests;

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::Mutex;

    use super::*;
    use crate::map_oracle::{CapturedMap, DensePixel, MAP_HEIGHT, MAP_WIDTH};
    use crate::projection::{Held, Reframe};
    use crate::{Camera, Size};

    struct DropWitness {
        name: &'static str,
        dropped: Arc<Mutex<Vec<&'static str>>>,
    }

    impl DropWitness {
        fn new(name: &'static str, dropped: &Arc<Mutex<Vec<&'static str>>>) -> Self {
            Self {
                name,
                dropped: Arc::clone(dropped),
            }
        }
    }

    impl Drop for DropWitness {
        fn drop(&mut self) {
            self.dropped.lock().unwrap().push(self.name);
        }
    }

    #[test]
    fn one_xs_source_import_requires_exactly_two_lenses() {
        let none: [u8; 0] = [];
        let one = [1];
        let pair = [1, 2];
        let three = [1, 2, 3];

        assert_eq!(
            exact_one_xs_lenses(&none).unwrap_err().to_string(),
            "ONE X2 source import requires exactly 2 lens frames, got 0"
        );
        assert_eq!(
            exact_one_xs_lenses(&one).unwrap_err().to_string(),
            "ONE X2 source import requires exactly 2 lens frames, got 1"
        );
        assert_eq!(exact_one_xs_lenses(&pair).unwrap(), [&1, &2]);
        assert_eq!(
            exact_one_xs_lenses(&three).unwrap_err().to_string(),
            "ONE X2 source import requires exactly 2 lens frames, got 3"
        );
    }

    #[test]
    fn imported_one_xs_picture_drops_planes_before_decoder_frames() {
        let dropped = Arc::new(Mutex::new(Vec::new()));
        let imported = ImportedOneXsPicture {
            planes: [
                DropWitness::new("planes A", &dropped),
                DropWitness::new("planes B", &dropped),
            ],
            frames: DropWitness::new("frames", &dropped),
            context: DropWitness::new("context", &dropped),
            session: DropWitness::new("session", &dropped),
        };

        drop(imported);
        assert_eq!(
            *dropped.lock().unwrap(),
            ["planes A", "planes B", "frames", "context", "session"]
        );
    }

    #[test]
    fn imported_one_xs_picture_binds_only_when_a_draw_is_prepared() {
        let source = include_str!("direct_type2.rs");
        let owner = source
            .split_once("impl ImportedOneXsPicture {")
            .unwrap()
            .1
            .split_once("fn exact_one_xs_lenses")
            .unwrap()
            .0;
        let fields = source
            .split_once("pub(crate) struct ImportedOneXsPicture")
            .unwrap()
            .1
            .split_once("impl ImportedOneXsPicture")
            .unwrap()
            .0;
        let import = owner
            .split_once("fn import_for_capture(")
            .unwrap()
            .1
            .split_once("pub(crate) fn submit_resident_front")
            .unwrap()
            .0;

        assert!(!owner.contains("-> &wgpu::BindGroup"));
        assert!(!owner.contains("-> wgpu::BindGroup"));
        assert!(!owner.contains("-> &wgpu::Texture"));
        assert!(!owner.contains("-> wgpu::Texture"));
        assert!(owner.contains("fn import_for_capture("));
        assert!(!owner.contains("pub(crate) fn import("));
        assert!(source.contains("pub(crate) fn import_picture("));
        assert!(owner.contains("capture.submit_imported(self)"));
        assert!(fields.contains("planes: P"));
        assert!(fields.contains("frames: F"));
        assert!(!fields.contains("picture:"));
        assert!(!fields.contains("uniforms:"));
        for forbidden in [
            "create_buffer",
            "write_buffer",
            "bind_picture",
            "BindGroupLayout",
            "Sampler",
            "Reframe",
        ] {
            assert!(
                !import.contains(forbidden),
                "source import contains {forbidden}"
            );
        }
        assert!(import.contains("dmabuf::import"));
        assert!(owner.contains("pub(crate) fn prepare_resident_draw("));
        assert!(owner.contains("pub(crate) fn draw_resident_binding("));
        assert!(owner.contains("pipeline.draw(pass, &binding.picture, map);"));
    }

    #[test]
    fn direct_pipeline_accepts_cloned_device_and_refuses_independent_device() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let Some(adapter) = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
        else {
            assert!(std::env::var_os("KJERAG_REQUIRE_GPU").is_none());
            return;
        };
        let request = || {
            block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("direct type-2 device identity"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            }))
            .unwrap()
        };
        let (device, queue) = request();
        let (foreign_device, foreign_queue) = request();
        let layout = crate::scene::bind_group_layout(&device);
        let pipeline = DirectType2Pipeline::new(&device, &layout, wgpu::TextureFormat::Rgba8Unorm);
        pipeline
            .ensure_device(&OneXsGpuContext::new(&device, &queue))
            .unwrap();
        let error = pipeline
            .ensure_device(&OneXsGpuContext::new(&foreign_device, &foreign_queue))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ONE X2 direct type-2 pipeline belongs to a different graphics device"
        );

        let resident = include_str!("flow/one_xs_belt_gpu.rs");
        let install = resident
            .split_once("fn prepare_resident_install<")
            .unwrap()
            .1
            .split_once("fn prepare_resident_bound<")
            .unwrap()
            .0;
        let bound = resident
            .split_once("fn prepare_resident_bound<")
            .unwrap()
            .1
            .split_once("impl ResidentImportedFront")
            .unwrap()
            .0;
        assert!(
            install.find("prepare_resident_bound").unwrap()
                < install.find(".reserve(retirements)").unwrap()
        );
        assert!(
            bound.find("pipeline.ensure_device").unwrap() < bound.find("bind_for_install").unwrap()
        );
        assert_eq!(
            resident.matches("Ok(ResidentBoundInstall {").count(),
            1,
            "another bound-install constructor bypasses direct-pipeline validation"
        );
        assert!(
            resident.contains("bound.and_then(ResidentBoundInstall::commit_future)"),
            "future commit bypasses the validated bound-install boundary"
        );
    }

    #[test]
    fn direct_shader_parses_and_uses_only_two_native_map_bindings() {
        use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

        let source = draw_wgsl();
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("direct type-2 WGSL did not parse: {error}"));
        Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|error| panic!("direct type-2 WGSL did not validate: {error}"));
        assert!(source.contains("type2_mesh(reframe.view_to_body * view.xyz)"));
        assert!(source.contains("@group(1) @binding(0)"));
        assert!(source.contains("@group(1) @binding(1)"));
        assert!(!source.contains("@group(2)"));
        assert!(
            !source.contains("@builtin(position) position: vec4<f32>,\n  @location(0) uv")
                || source.contains("view_ray(in.uv)")
        );
        assert!(source.contains("select(rgb, linear, reframe.linearize > 0.5)"));
        assert!(source.contains("vec2<f32>(reframe.frame_width, reframe.frame_height)"));
        assert!(source.contains("source_size * 0.5"));
        assert!(!source.contains("2880.0"));
        assert!(!source.contains("1440.0"));
    }

    #[test]
    fn native_grid_gpu_matches_dense_cpu_across_views_and_poles() {
        let (device, queue) = match gpu() {
            Ok(open) => open,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}",
                );
                eprintln!("direct type-2 twin: skipped, no GPU on this box ({why})");
                return;
            }
        };
        let packed: Vec<[f32; 4]> = (0..MAP_HEIGHT)
            .flat_map(|row| {
                (0..MAP_WIDTH).map(move |col| {
                    let longitude = col as f32 / MAP_WIDTH as f32;
                    let latitude = row as f32 / MAP_HEIGHT as f32;
                    [longitude, latitude, 0.5 + 0.5 * longitude, 1.0 - latitude]
                })
            })
            .collect();
        let alpha: Vec<f32> = (0..MAP_HEIGHT)
            .flat_map(|row| {
                (0..MAP_WIDTH).map(move |col| {
                    0.5 + 0.25 * (std::f32::consts::TAU * col as f32 / MAP_WIDTH as f32).sin()
                        + 0.2 * row as f32 / MAP_HEIGHT as f32
                })
            })
            .collect();
        let map = CapturedMap::new(packed, alpha).unwrap();
        let size = Size::new(48, 36);
        for (label, camera) in [
            (
                "ordinary",
                Camera {
                    yaw: 0.73,
                    pitch: -0.21,
                    fov: 1.15,
                },
            ),
            (
                "north pole",
                Camera {
                    yaw: 2.1,
                    pitch: std::f32::consts::FRAC_PI_2,
                    fov: 1.0,
                },
            ),
            (
                "south pole and periodic seam",
                Camera {
                    yaw: -std::f32::consts::PI,
                    pitch: -std::f32::consts::FRAC_PI_2,
                    fov: 1.0,
                },
            ),
            (
                "widest flat view and periodic seam",
                Camera {
                    yaw: std::f32::consts::PI,
                    pitch: 0.0,
                    fov: crate::projection::FOV_FLAT,
                },
            ),
        ] {
            let reframe = Reframe::new(
                &crate::projection::tests::one_xs_lenses(),
                crate::projection::tests::ONE_XS_FRAME,
                camera,
                Held::default(),
                size.width as f32 / size.height as f32,
                false,
                crate::Sampling::Bilinear,
            );
            let cpu = map.rasterize(&reframe, size);
            let mesh = crate::map_oracle::Mesh::new(&map);
            for rasterized in [false, true] {
                let gpu = on_gpu(&device, &queue, &reframe, &map, size, rasterized);
                assert_eq!(gpu.len(), cpu.pixels.len());
                let mut worst = 0.0f32;
                let mut coverage_mismatch = 0usize;
                for (index, (expected, actual)) in cpu.pixels.iter().zip(&gpu).enumerate() {
                    if expected.covered != actual.covered {
                        coverage_mismatch += 1;
                        continue;
                    }
                    if expected.covered == 0.0 {
                        continue;
                    }
                    // Fixed-function interpolation snaps projected vertices
                    // to a subpixel grid. Bound its difference in output
                    // pixels, including the fixture's discontinuous UV ramp.
                    // The ray path keeps its original numeric bound below.
                    let neighbors: Vec<_> = if rasterized {
                        [-1.0f32, 1.0]
                            .into_iter()
                            .flat_map(|dy| [-1.0f32, 1.0].into_iter().map(move |dx| (dx, dy)))
                            .map(|(dx, dy)| {
                                mesh.sample_view(
                                    &reframe,
                                    &map.alpha,
                                    [
                                        ((index % size.width as usize) as f32 + 0.5 + dx / 64.0)
                                            / size.width as f32,
                                        ((index / size.width as usize) as f32 + 0.5 + dy / 64.0)
                                            / size.height as f32,
                                    ],
                                )
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    for (channel, (a, b)) in expected
                        .packed_uv
                        .iter()
                        .flatten()
                        .chain(std::iter::once(&expected.alpha))
                        .zip(
                            actual
                                .packed_uv
                                .iter()
                                .flatten()
                                .chain(std::iter::once(&actual.alpha)),
                        )
                        .enumerate()
                    {
                        let difference = (a - b).abs();
                        if rasterized {
                            let values = neighbors.iter().map(|pixel| {
                                if channel == 4 {
                                    pixel.alpha
                                } else {
                                    pixel.packed_uv[channel / 2][channel % 2]
                                }
                            });
                            let (low, high) = values.fold((*a, *a), |(low, high), value| {
                                (low.min(value), high.max(value))
                            });
                            assert!(
                                *b >= low - 2.0e-4 && *b <= high + 2.0e-4,
                                "{label}, pixel {index}, channel {channel}: rasterized {b} outside 1/64-pixel reference [{low}, {high}]"
                            );
                        }
                        worst = worst.max(difference);
                    }
                }
                assert_eq!(
                    coverage_mismatch, 0,
                    "{label}, rasterized={rasterized}: triangle admission differs"
                );
                assert!(
                    rasterized || worst <= 2.0e-4,
                    "{label}, rasterized={rasterized}: worst native-grid residue is {worst}"
                );
                eprintln!("{label}, rasterized={rasterized}: worst native-grid residue {worst}");
            }
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            match future.as_mut().poll(&mut context) {
                std::task::Poll::Ready(answer) => return answer,
                std::task::Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    pub(super) fn gpu() -> Result<(wgpu::Device, wgpu::Queue), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
            .ok_or("no Vulkan adapter")?;
        block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("direct type-2 twin"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())
    }

    fn bytes_of<T>(values: &[T]) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
        }
    }

    fn on_gpu(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        reframe: &Reframe,
        map: &CapturedMap,
        size: Size,
        rasterized: bool,
    ) -> Vec<DensePixel> {
        let probe = format!(
            r#"
@group(1) @binding(2) var<storage, read_write> type2_answers: array<vec4<f32>>;
const PROBE_WIDTH = {width}u;
const PROBE_HEIGHT = {height}u;

@compute @workgroup_size(64)
fn direct_type2_twin(@builtin(global_invocation_id) id: vec3<u32>) {{
  let count = PROBE_WIDTH * PROBE_HEIGHT;
  if id.x >= count {{ return; }}
  let x = id.x % PROBE_WIDTH;
  let y = id.x / PROBE_WIDTH;
  let uv = vec2<f32>((f32(x) + 0.5) / f32(PROBE_WIDTH), (f32(y) + 0.5) / f32(PROBE_HEIGHT));
  let view = view_ray(uv);
  var answer: Type2Sample;
  if view.w > 0.0 {{ answer = type2_mesh(reframe.view_to_body * view.xyz); }}
  type2_answers[id.x * 2u] = answer.packed;
  type2_answers[id.x * 2u + 1u] = vec4<f32>(answer.alpha, answer.covered, 0.0, 0.0);
}}

@fragment
fn mesh_probe(in: Type2MeshOut) -> @location(0) vec4<f32> {{
  let index = u32(in.position.y) * PROBE_WIDTH + u32(in.position.x);
  type2_answers[index * 2u] = in.packed;
  type2_answers[index * 2u + 1u] = vec4<f32>(type2_sample1(in.map_uv), 1.0, 0.0, 0.0);
  return vec4<f32>(1.0);
}}
"#,
            width = size.width,
            height = size.height,
        );
        let source = format!("{}\n{probe}", draw_wgsl());
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("direct type-2 twin"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("direct type-2 twin uniform"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Reframe>() as u64),
                },
                count: None,
            }],
        });
        let storage = |binding, read_only, bytes| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE
                | wgpu::ShaderStages::FRAGMENT
                | if read_only {
                    wgpu::ShaderStages::VERTEX
                } else {
                    wgpu::ShaderStages::empty()
                },
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(bytes),
            },
            count: None,
        };
        let answer_bytes = u64::from(size.width) * u64::from(size.height) * 32;
        let map_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("direct type-2 twin maps"),
            entries: &[
                storage(0, true, PACKED_BYTES as u64),
                storage(1, true, ALPHA_BYTES as u64),
                storage(2, false, answer_bytes),
            ],
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("direct type-2 twin"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("direct type-2 twin"),
                    bind_group_layouts: &[&uniform_layout, &map_layout],
                    immediate_size: 0,
                }),
            ),
            module: &module,
            entry_point: Some("direct_type2_twin"),
            compilation_options: Default::default(),
            cache: None,
        });
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let uniform = buffer(
            "direct type-2 twin uniform",
            std::mem::size_of::<Reframe>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let packed = buffer(
            "direct type-2 twin packed",
            PACKED_BYTES as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let alpha = buffer(
            "direct type-2 twin alpha",
            ALPHA_BYTES as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let answers = buffer(
            "direct type-2 twin answers",
            answer_bytes,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let readback = buffer(
            "direct type-2 twin readback",
            answer_bytes,
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        queue.write_buffer(&uniform, 0, reframe.bytes());
        queue.write_buffer(&packed, 0, bytes_of(&map.packed));
        queue.write_buffer(&alpha, 0, bytes_of(&map.alpha));
        let group0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("direct type-2 twin uniform"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let group1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("direct type-2 twin maps"),
            layout: &map_layout,
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
                    resource: answers.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        if rasterized {
            let raster = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("native mesh rasterization regression"),
                layout: Some(
                    &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: None,
                        bind_group_layouts: &[&uniform_layout, &map_layout],
                        immediate_size: 0,
                    }),
                ),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("mesh_vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("mesh_probe"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
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
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("native mesh regression target"),
                size: wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = target.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("native mesh regression"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&raster);
            pass.set_bind_group(0, &group0, &[]);
            pass.set_bind_group(1, &group1, &[]);
            pass.draw(0..30_000, 0..1);
        } else {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group0, &[]);
            pass.set_bind_group(1, &group1, &[]);
            pass.dispatch_workgroups((size.width * size.height).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&answers, 0, &readback, 0, answer_bytes);
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = readback.slice(..).get_mapped_range();
        let floats: Vec<f32> = mapped
            .chunks_exact(4)
            .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
            .collect();
        let pixels = floats
            .chunks_exact(8)
            .map(DensePixel::from_test_lanes)
            .collect();
        drop(mapped);
        readback.unmap();
        pixels
    }
}
