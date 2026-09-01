//! Readback of the exact R8 lens pair a prepared picture samples.
//!
//! Imported dmabuf textures are sampling resources, not copy sources. This
//! instrument therefore reads their luma planes through a tiny compute pass,
//! packs four byte codes into each storage word, and copies that buffer back.
//! Other camera paths never construct the pipeline or allocate a buffer. The
//! first correctness-first selected ONE X2 player waits for this readback on
//! the render thread so no later frame can pass its causal estimator. That is
//! a disclosed performance limitation, not the final scheduling design; the
//! same exact token can move to a bounded worker without changing semantics.

use std::sync::{Arc, mpsc};

use kjerag_media::{FrameStamp, Frames, Size};

use crate::flow::one_xs::{Lens, LensPair};
use crate::flow::one_xs_belt::SourceImage;
use crate::{Fallible, Planes};

const PIXELS_PER_WORD: u32 = 4;
const WORKGROUP_EDGE: u32 = 16;

/// Two compact source images from one exact delivered lens pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OneXsLumaFrame {
    frame: FrameStamp,
    size: Size,
    sources: LensPair<SourceImage>,
}

impl OneXsLumaFrame {
    #[cfg(test)]
    pub(crate) fn for_test(frame: FrameStamp, size: Size, sources: LensPair<SourceImage>) -> Self {
        Self {
            frame,
            size,
            sources,
        }
    }

    pub fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub const fn size(&self) -> Size {
        self.size
    }

    pub fn sources(&self) -> &LensPair<SourceImage> {
        &self.sources
    }

    pub fn into_sources(self) -> LensPair<SourceImage> {
        self.sources
    }
}

/// One submitted source readback.
///
/// The decoded [`Frames`] stay owned until the GPU finishes sampling their
/// imported textures. Reading consumes the token, so one submitted delivery
/// cannot be published twice.
#[must_use = "the submitted ONE X2 source readback has not been consumed"]
pub struct PendingOneXsLuma {
    device: wgpu::Device,
    _frames: Arc<Frames>,
    _packed: wgpu::Buffer,
    readback: wgpu::Buffer,
    submission: wgpu::SubmissionIndex,
    frame: FrameStamp,
    shape: LumaShape,
}

impl PendingOneXsLuma {
    pub fn frame(&self) -> &FrameStamp {
        &self.frame
    }

    pub const fn size(&self) -> Size {
        self.shape.size
    }

    /// Wait for and consume this exact-frame readback.
    ///
    /// The correctness-first ONE X2 player deliberately runs this on the
    /// render thread to preserve the simplest source/map transaction. It can
    /// visibly lower frame rate. A later bounded worker may consume the same
    /// token, but it must preserve every-frame order and exact [`FrameStamp`]
    /// association.
    pub fn read(self) -> Fallible<OneXsLumaFrame> {
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(self.submission),
            timeout: None,
        })?;

        let (mapped, answer) = mpsc::channel();
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = mapped.send(result);
            });
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        answer.recv()??;

        let view = self.readback.slice(..).get_mapped_range();
        let compact = unpack_lenses(&view, self.shape)?;
        drop(view);
        self.readback.unmap();

        let rows = self.shape.size.height as usize;
        let cols = self.shape.size.width as usize;
        Ok(OneXsLumaFrame {
            frame: self.frame,
            size: self.shape.size,
            sources: LensPair {
                a: SourceImage::from_compact(rows, cols, compact.a)?,
                b: SourceImage::from_compact(rows, cols, compact.b)?,
            },
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LumaShape {
    size: Size,
    words_per_row: u32,
    bytes: u64,
}

/// Validate the actual imported pair before lazily constructing GPU state.
pub(crate) fn validate(planes: &[Planes], frames: &Frames) -> Fallible<LumaShape> {
    if frames.lenses.len() != 2 {
        return Err(format!(
            "ONE X2 source readback found {} delivered lenses, expected 2",
            frames.lenses.len()
        )
        .into());
    }
    if planes.len() != 2 {
        return Err(format!(
            "ONE X2 source readback found {} lens planes, expected 2",
            planes.len()
        )
        .into());
    }
    for lens in Lens::ALL {
        let texture = &planes[lens.index()].luma;
        if texture.format() != wgpu::TextureFormat::R8Unorm {
            return Err(format!(
                "ONE X2 source readback needs R8 luma, but lens {lens} is {:?}",
                texture.format()
            )
            .into());
        }
        let actual = texture.size();
        if actual.width != frames.size.width
            || actual.height != frames.size.height
            || actual.depth_or_array_layers != 1
        {
            return Err(format!(
                "ONE X2 source readback lens {lens} is {} by {} by {}, expected {} by {} by 1",
                actual.width,
                actual.height,
                actual.depth_or_array_layers,
                frames.size.width,
                frames.size.height,
            )
            .into());
        }
    }

    let words_per_row = frames.size.width.div_ceil(PIXELS_PER_WORD);
    let bytes = u64::from(words_per_row)
        .checked_mul(u64::from(frames.size.height))
        .and_then(|one_lens| one_lens.checked_mul(2))
        .and_then(|words| words.checked_mul(size_of::<u32>() as u64))
        .ok_or("ONE X2 source readback size overflows")?;
    Ok(LumaShape {
        size: frames.size,
        words_per_row,
        bytes,
    })
}

/// Lazily constructed compute resources. Each submitted frame owns fresh
/// output and staging buffers so completion order cannot overwrite another
/// frame's bytes.
pub(crate) struct LumaReadbackPipeline {
    pipeline: wgpu::ComputePipeline,
    output_layout: wgpu::BindGroupLayout,
}

impl LumaReadbackPipeline {
    pub(crate) fn new(device: &wgpu::Device, scene_layout: &wgpu::BindGroupLayout) -> Self {
        let output_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ONE X2 source readback output"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 source readback"),
            bind_group_layouts: &[scene_layout, &output_layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 source readback"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ONE X2 source readback"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("read_luma"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            pipeline,
            output_layout,
        }
    }

    pub(crate) fn submit(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &wgpu::BindGroup,
        frames: Arc<Frames>,
        shape: LumaShape,
    ) -> PendingOneXsLuma {
        let submitted = self.encode(device, queue, scene, shape);
        let frame = frames.stamp();
        PendingOneXsLuma {
            device: device.clone(),
            _frames: frames,
            _packed: submitted.packed,
            readback: submitted.readback,
            submission: submitted.submission,
            frame,
            shape,
        }
    }

    fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &wgpu::BindGroup,
        shape: LumaShape,
    ) -> SubmittedLuma {
        let packed = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 source packed luma"),
            size: shape.bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 source readback"),
            size: shape.bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let output = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 source readback output"),
            layout: &self.output_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: packed.as_entire_binding(),
            }],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 source readback"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 source readback"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, scene, &[]);
            pass.set_bind_group(1, &output, &[]);
            pass.dispatch_workgroups(
                shape.words_per_row.div_ceil(WORKGROUP_EDGE),
                shape.size.height.div_ceil(WORKGROUP_EDGE),
                2,
            );
        }
        encoder.copy_buffer_to_buffer(&packed, 0, &readback, 0, shape.bytes);
        SubmittedLuma {
            packed,
            readback,
            submission: queue.submit([encoder.finish()]),
        }
    }
}

struct SubmittedLuma {
    packed: wgpu::Buffer,
    readback: wgpu::Buffer,
    submission: wgpu::SubmissionIndex,
}

fn unpack_lenses(mapped: &[u8], shape: LumaShape) -> Fallible<LensPair<Vec<u8>>> {
    if mapped.len() != shape.bytes as usize {
        return Err(format!(
            "ONE X2 source readback mapped {} bytes, expected {}",
            mapped.len(),
            shape.bytes,
        )
        .into());
    }
    let lens_words = shape.words_per_row as usize * shape.size.height as usize;
    let compact = |lens: Lens| {
        let mut pixels = Vec::with_capacity(shape.size.width as usize * shape.size.height as usize);
        let first_word = lens.index() * lens_words;
        for row in 0..shape.size.height as usize {
            let row_word = first_word + row * shape.words_per_row as usize;
            let before = pixels.len();
            for word in 0..shape.words_per_row as usize {
                let at = (row_word + word) * size_of::<u32>();
                let packed = u32::from_le_bytes(mapped[at..at + 4].try_into().unwrap());
                pixels.extend_from_slice(&packed.to_le_bytes());
            }
            pixels.truncate(before + shape.size.width as usize);
        }
        pixels
    };
    Ok(LensPair {
        a: compact(Lens::A),
        b: compact(Lens::B),
    })
}

/// R8 normalized conversion has far less than half a code of error, so
/// round(sample * 255) recovers every possible source byte. Each invocation
/// packs four adjacent columns and handles both physical lenses under one
/// dispatch. The row tail is zero-filled and discarded on the CPU.
const SHADER: &str = r#"
@group(0) @binding(1) var luma_a: texture_2d<f32>;
@group(0) @binding(3) var luma_b: texture_2d<f32>;
@group(1) @binding(0) var<storage, read_write> output_words: array<u32>;

fn byte_code(value: f32) -> u32 {
    return u32(round(value * 255.0));
}

@compute @workgroup_size(16, 16, 1)
fn read_luma(@builtin(global_invocation_id) at: vec3<u32>) {
    let size = textureDimensions(luma_a);
    let words_per_row = (size.x + 3u) / 4u;
    if at.x >= words_per_row || at.y >= size.y || at.z >= 2u {
        return;
    }

    let first_x = at.x * 4u;
    var packed = 0u;
    for (var lane = 0u; lane < 4u; lane += 1u) {
        let x = first_x + lane;
        if x < size.x {
            var value = 0.0;
            if at.z == 0u {
                value = textureLoad(luma_a, vec2<i32>(i32(x), i32(at.y)), 0).x;
            } else {
                value = textureLoad(luma_b, vec2<i32>(i32(x), i32(at.y)), 0).x;
            }
            packed |= byte_code(value) << (8u * lane);
        }
    }

    let lens_words = words_per_row * size.y;
    output_words[at.z * lens_words + at.y * words_per_row + at.x] = packed;
}
"#;

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::*;

    #[test]
    fn packed_words_preserve_lens_rows_and_odd_width_tail() {
        let size = Size::new(5, 2);
        let shape = LumaShape {
            size,
            words_per_row: 2,
            bytes: 32,
        };
        let words = [
            0x0403_0201u32,
            0xeeee_ee05,
            0x0908_0706,
            0xeeee_ee0a,
            0x6867_6665,
            0xeeee_ee69,
            0x6d6c_6b6a,
            0xeeee_ee6e,
        ];
        let mapped = words
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        let unpacked = unpack_lenses(&mapped, shape).unwrap();
        assert_eq!(unpacked.a, (1..=10).collect::<Vec<_>>());
        assert_eq!(unpacked.b, (101..=110).collect::<Vec<_>>());
        assert!(!unpacked.a.contains(&0xee));
        assert!(!unpacked.b.contains(&0xee));
    }

    #[test]
    fn synthetic_r8_readback_is_byte_exact_for_both_lenses() {
        let Ok((device, queue)) = gpu() else {
            eprintln!("no GPU available for the ONE X2 source readback test");
            return;
        };
        let size = Size::new(259, 5);
        let stride = 512usize;
        let source = |lens: Lens| {
            let mut padded = vec![0xee; stride * size.height as usize];
            let mut compact = Vec::with_capacity(size.width as usize * size.height as usize);
            for row in 0..size.height as usize {
                for col in 0..size.width as usize {
                    let base = (17 * row + 29 * col + 3) as u8;
                    let value = match lens {
                        Lens::A => base,
                        Lens::B => base ^ 0xa5,
                    };
                    padded[row * stride + col] = value;
                    compact.push(value);
                }
            }
            (padded, compact)
        };
        let (padded_a, expected_a) = source(Lens::A);
        let (padded_b, expected_b) = source(Lens::B);
        let texture = |label: &str, bytes: &[u8]| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride as u32),
                    rows_per_image: Some(size.height),
                },
                texture.size(),
            );
            texture
        };
        let texture_a = texture("ONE X2 synthetic source A", &padded_a);
        let texture_b = texture("ONE X2 synthetic source B", &padded_b);
        let view_a = texture_a.create_view(&Default::default());
        let view_b = texture_b.create_view(&Default::default());
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ONE X2 synthetic sources"),
            entries: &[1, 3].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }),
        });
        let scene = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 synthetic sources"),
            layout: &scene_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view_a),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&view_b),
                },
            ],
        });
        let shape = LumaShape {
            size,
            words_per_row: size.width.div_ceil(PIXELS_PER_WORD),
            bytes: u64::from(size.width.div_ceil(PIXELS_PER_WORD)) * u64::from(size.height) * 2 * 4,
        };
        let pipeline = LumaReadbackPipeline::new(&device, &scene_layout);
        let submitted = pipeline.encode(&device, &queue, &scene, shape);
        let slice = submitted.readback.slice(..);
        let (mapped, answer) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            mapped.send(result).unwrap();
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted.submission),
                timeout: None,
            })
            .unwrap();
        answer.recv().unwrap().unwrap();
        let mapped = slice.get_mapped_range();
        let actual = unpack_lenses(&mapped, shape).unwrap();
        assert_eq!(actual.a, expected_a);
        assert_eq!(actual.b, expected_b);
        drop(mapped);
        submitted.readback.unmap();
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

    fn gpu() -> Result<(wgpu::Device, wgpu::Queue), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
            .ok_or("no Vulkan adapter")?;
        block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ONE X2 source readback test"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())
    }
}
