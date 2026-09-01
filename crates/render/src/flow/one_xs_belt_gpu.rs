//! GPU sampling and reduction for selected ONE X2 solver inputs.
//!
//! This is the GPU-shaped equivalent of [`super::one_xs_belt::sample_source_belts`]
//! followed by [`SourceBelts::reduce_area_3x3`](super::one_xs_belt::SourceBelts::reduce_area_3x3).
//! It deliberately is not wired into playback yet. One invocation owns four
//! final U8 codes and packs them into one storage word, so no 3240-by-180
//! staging allocation or CPU luma readback lies between the imported R8
//! textures and the 1080-by-60 solver inputs.

use std::sync::mpsc;

use super::one_xs::{Lens, LensPair};
use super::one_xs_belt::{RetainedBaseMaps, SolverBelts};
use crate::Fallible;

const CODES_PER_WORD: usize = 4;
const OUTPUT_BYTES: u64 = SolverBelts::BYTES as u64;
const OUTPUT_WORDS: u32 = (SolverBelts::BYTES / CODES_PER_WORD) as u32;
const WORKGROUP_SIZE: u32 = 64;
const _: () = assert!(SolverBelts::BYTES.is_multiple_of(CODES_PER_WORD));
const _: () = assert!(RetainedBaseMaps::NODES_PER_LENS.is_multiple_of(CODES_PER_WORD));

/// The two exact R8 source textures in physical A/B order.
#[derive(Clone, Copy)]
pub(crate) struct SourceTextures<'a> {
    pub(crate) a: &'a wgpu::Texture,
    pub(crate) b: &'a wgpu::Texture,
}

impl SourceTextures<'_> {
    fn validate(self) -> Fallible<()> {
        for (lens, texture) in [(Lens::A, self.a), (Lens::B, self.b)] {
            if texture.format() != wgpu::TextureFormat::R8Unorm {
                return Err(format!(
                    "ONE X2 GPU belt source {lens} is {:?}, expected R8Unorm",
                    texture.format()
                )
                .into());
            }
            let size = texture.size();
            if size.width == 0 || size.height == 0 || size.depth_or_array_layers != 1 {
                return Err(format!(
                    "ONE X2 GPU belt source {lens} is {} by {} by {}, expected a nonempty 2D texture",
                    size.width, size.height, size.depth_or_array_layers
                )
                .into());
            }
        }
        Ok(())
    }
}

/// Lazily constructed compute state. Each submission owns fresh map, output,
/// bind-group and readback resources so overlapping frames cannot overwrite
/// one another.
pub(crate) struct GpuSolverBeltPipeline {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl GpuSolverBeltPipeline {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ONE X2 GPU solver belts"),
            entries: &[texture(0), texture(1), storage(2, true), storage(3, false)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 GPU solver belts"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 GPU solver belts"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ONE X2 GPU solver belts"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("build_solver_belts"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { pipeline, layout }
    }

    /// Submit one independent source/map transaction.
    ///
    /// The returned token retains every bind resource until the submission has
    /// completed. Its packed buffer is already in final A-then-B solver order
    /// and can become a later GPU solver's direct input; [`PendingSolverBelts::read`]
    /// exists for the exact CPU-oracle gate and the current CPU solver bridge.
    pub(crate) fn submit(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sources: SourceTextures<'_>,
        maps: &RetainedBaseMaps,
    ) -> Fallible<PendingSolverBelts<()>> {
        self.submit_retained(device, queue, sources, maps, ())
    }

    /// Submit while retaining the owner of imported source images.
    ///
    /// A dmabuf texture aliases a decoder surface. Scene integration must pass
    /// the corresponding frame owner here; retaining only wgpu handles does
    /// not keep that external surface out of the decoder's pool.
    pub(crate) fn submit_retained<K>(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sources: SourceTextures<'_>,
        maps: &RetainedBaseMaps,
        source_owner: K,
    ) -> Fallible<PendingSolverBelts<K>> {
        sources.validate()?;
        let map = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 retained base maps"),
            size: maps.bytes().len() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&map, 0, maps.bytes());
        let packed = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 packed solver belts"),
            size: OUTPUT_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 solver belt readback"),
            size: OUTPUT_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let view_a = sources.a.create_view(&Default::default());
        let view_b = sources.b.create_view(&Default::default());
        let resources = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 GPU solver belt resources"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view_a),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view_b),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: map.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: packed.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 GPU solver belts"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 GPU solver belts"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &resources, &[]);
            pass.dispatch_workgroups(OUTPUT_WORDS.div_ceil(WORKGROUP_SIZE), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&packed, 0, &readback, 0, OUTPUT_BYTES);
        let submission = queue.submit([encoder.finish()]);
        Ok(PendingSolverBelts {
            device: device.clone(),
            _source_owner: source_owner,
            _map: map,
            packed,
            readback,
            _resources: resources,
            submission,
        })
    }
}

/// One submitted GPU solver-belt transaction.
#[must_use = "the submitted ONE X2 solver belts have not been consumed"]
pub(crate) struct PendingSolverBelts<K> {
    device: wgpu::Device,
    _source_owner: K,
    _map: wgpu::Buffer,
    packed: wgpu::Buffer,
    readback: wgpu::Buffer,
    _resources: wgpu::BindGroup,
    submission: wgpu::SubmissionIndex,
}

impl<K> PendingSolverBelts<K> {
    /// The compact GPU-resident A-then-B payload, four U8 codes per word.
    pub(crate) fn packed(&self) -> &wgpu::Buffer {
        &self.packed
    }

    /// Wait for and consume the exact compact payload.
    pub(crate) fn read(self) -> Fallible<SolverBelts> {
        let slice = self.readback.slice(..);
        let (mapped, answer) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = mapped.send(result);
        });
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(self.submission),
            timeout: None,
        })?;
        answer.recv()??;
        let mapped = slice.get_mapped_range();
        let bytes = mapped
            .chunks_exact(size_of::<u32>())
            .flat_map(|word| u32::from_ne_bytes(word.try_into().unwrap()).to_le_bytes())
            .collect::<Vec<_>>();
        drop(mapped);
        self.readback.unmap();
        debug_assert_eq!(bytes.len(), SolverBelts::BYTES);
        SolverBelts::from_lenses(LensPair {
            a: bytes[..RetainedBaseMaps::NODES_PER_LENS].to_vec(),
            b: bytes[RetainedBaseMaps::NODES_PER_LENS..].to_vec(),
        })
        .map_err(Into::into)
    }
}

// The explicit `fma` chain and native `(1-coordinate)+floor(coordinate)`
// weights mirror the CPU oracle. WGSL permits a backend to expand `fma`, so
// byte identity remains an adapter-tested contract rather than a promise made
// from source spelling alone. NaN, infinity and subnormal handling can also
// vary with backend finite-math and flush-to-zero policy. The required GPU
// twin below is the gate for the target RADV adapter.
const SHADER: &str = r#"
const ROWS = 1080u;
const COLS = 60u;
const PIXELS_PER_LENS = ROWS * COLS;
const TOTAL_CODES = 2u * PIXELS_PER_LENS;
const AREA = 3u;
const THIRD_BITS: u32 = 0x3eaaaaabu;

@group(0) @binding(0) var source_a: texture_2d<f32>;
@group(0) @binding(1) var source_b: texture_2d<f32>;
@group(0) @binding(2) var<storage, read> base_maps: array<vec2<f32>>;
@group(0) @binding(3) var<storage, read_write> output_words: array<u32>;

fn weights(value: f32, maximum: f32) -> vec4<f32> {
    let clamped = clamp(value, 0.0, maximum);
    let low = floor(clamped);
    let low_side = (1.0 - clamped) + low;
    return vec4<f32>(low_side, 1.0 - low_side, low, clamped);
}

fn base_at(lens: u32, row: i32, col: i32) -> vec2<f32> {
    let r = u32(clamp(row, 0, i32(ROWS) - 1));
    let c = u32(clamp(col, 0, i32(COLS) - 1));
    return base_maps[lens * PIXELS_PER_LENS + r * COLS + c];
}

fn sample_base(lens: u32, row: f32, col: f32) -> vec2<f32> {
    let rw = weights(row, f32(ROWS - 1u));
    let cw = weights(col, f32(COLS - 1u));
    let top_left = cw.x * rw.x;
    let top_right = rw.x - top_left;
    let bottom_left = cw.x - top_left;
    let bottom_right = ((1.0 - cw.x) - rw.x) + top_left;
    let ri = i32(rw.z);
    let ci = i32(cw.z);
    let tl = base_at(lens, ri, ci);
    let tr = base_at(lens, ri, ci + 1);
    let bl = base_at(lens, ri + 1, ci);
    let br = base_at(lens, ri + 1, ci + 1);
    var value = top_right * tr;
    value = fma(vec2<f32>(top_left), tl, value);
    value = fma(vec2<f32>(bottom_left), bl, value);
    return fma(vec2<f32>(bottom_right), br, value);
}

fn texel(lens: u32, row: i32, col: i32, dimensions: vec2<i32>) -> f32 {
    let at = vec2<i32>(clamp(col, 0, dimensions.x - 1), clamp(row, 0, dimensions.y - 1));
    var value: f32;
    if lens == 0u {
        value = textureLoad(source_a, at, 0).r;
    } else {
        value = textureLoad(source_b, at, 0).r;
    }
    return round(value * 255.0);
}

fn sample_source(lens: u32, uv: vec2<f32>) -> u32 {
    if !(uv.x > 0.0 && uv.y > 0.0) {
        return 0u;
    }
    var dimensions: vec2<i32>;
    if lens == 0u {
        dimensions = vec2<i32>(textureDimensions(source_a));
    } else {
        dimensions = vec2<i32>(textureDimensions(source_b));
    }
    let xw = weights(uv.x * f32(dimensions.x), f32(dimensions.x - 1));
    let yw = weights(uv.y * f32(dimensions.y), f32(dimensions.y - 1));
    let top_left = xw.x * yw.x;
    let top_right = yw.x - top_left;
    let bottom_left = xw.x - top_left;
    let bottom_right = ((1.0 - xw.x) - yw.x) + top_left;
    let xi = i32(xw.z);
    let yi = i32(yw.z);
    var value = top_right * texel(lens, yi, xi + 1, dimensions);
    value = fma(top_left, texel(lens, yi, xi, dimensions), value);
    value = fma(bottom_left, texel(lens, yi + 1, xi, dimensions), value);
    value = fma(bottom_right, texel(lens, yi + 1, xi + 1, dimensions), value);
    return u32(value);
}

fn solver_code(index: u32) -> u32 {
    let lens = index / PIXELS_PER_LENS;
    let local = index - lens * PIXELS_PER_LENS;
    let row = local / COLS;
    let col = local - row * COLS;
    var sum = 0u;
    for (var dr = 0u; dr < AREA; dr += 1u) {
        for (var dc = 0u; dc < AREA; dc += 1u) {
            let source_row = row * AREA + dr;
            let source_col = col * AREA + dc;
            let third = bitcast<f32>(THIRD_BITS);
            let uv = sample_base(lens, f32(source_row) * third, f32(source_col) * third);
            sum += sample_source(lens, uv);
        }
    }
    return (sum + 4u) / 9u;
}

@compute @workgroup_size(64)
fn build_solver_belts(@builtin(global_invocation_id) id: vec3<u32>) {
    let first = id.x * 4u;
    if first >= TOTAL_CODES {
        return;
    }
    var packed = 0u;
    for (var lane = 0u; lane < 4u; lane += 1u) {
        packed |= solver_code(first + lane) << (8u * lane);
    }
    output_words[id.x] = packed;
}
"#;

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::*;
    use crate::flow::one_xs_belt::{SourceImage, sample_source_belts};

    #[test]
    fn gpu_solver_belts_are_byte_exact_on_adversarial_odd_padded_sources() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}"
                );
                eprintln!("skipping exact ONE X2 GPU solver-belt twin: {why}");
                return;
            }
        };

        const A_ROWS: usize = 127;
        const A_COLS: usize = 259;
        const B_ROWS: usize = 131;
        const B_COLS: usize = 263;
        const STRIDE: usize = 512;
        let source = |lens: Lens, rows: usize, cols: usize| {
            let mut pixels = (0..rows * cols)
                .map(|index| {
                    let row = index / cols;
                    let col = index % cols;
                    match lens {
                        Lens::A => ((17 * row + 29 * col + 3) % 256) as u8,
                        Lens::B => ((43 * row + 11 * col + 197) % 256) as u8,
                    }
                })
                .collect::<Vec<_>>();
            if lens == Lens::A {
                // This is the native-order source-FMA discriminator from the
                // scalar oracle. At row .251, column .871 it is 190; a
                // top-left-first accumulation is 189.
                pixels[0] = 17;
                pixels[1] = 201;
                pixels[cols] = 93;
                pixels[cols + 1] = 248;
            }
            SourceImage::from_compact(rows, cols, pixels).unwrap()
        };
        let sources = LensPair {
            a: source(Lens::A, A_ROWS, A_COLS),
            b: source(Lens::B, B_ROWS, B_COLS),
        };
        let texture = |label: &str, source: &SourceImage| {
            let mut padded = vec![0xee; STRIDE * source.rows()];
            for row in 0..source.rows() {
                padded[row * STRIDE..row * STRIDE + source.cols()].copy_from_slice(
                    &source.pixels()[row * source.cols()..(row + 1) * source.cols()],
                );
            }
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: source.cols() as u32,
                    height: source.rows() as u32,
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
                &padded,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(STRIDE as u32),
                    rows_per_image: Some(source.rows() as u32),
                },
                texture.size(),
            );
            texture
        };
        let texture_a = texture("ONE X2 odd padded A", &sources.a);
        let texture_b = texture("ONE X2 odd padded B", &sources.b);

        let map = |lens: Lens| {
            (0..RetainedBaseMaps::NODES_PER_LENS)
                .map(|index| {
                    let row = index / 60;
                    let col = index % 60;
                    let selector = (31 * row + 47 * col + lens.index()) % 997;
                    match selector {
                        0 => [0.0, 0.5],
                        1 => [-0.25, 0.75],
                        2 => [f32::NAN, 0.5],
                        3 => [0.5, f32::NAN],
                        4 => [1.25, 1.5],
                        5 => [f32::INFINITY, f32::INFINITY],
                        _ => {
                            let (rows, cols) = match lens {
                                Lens::A => (A_ROWS, A_COLS),
                                Lens::B => (B_ROWS, B_COLS),
                            };
                            let x = 1 + (13 * row + 7 * col + 19 * lens.index()) % (cols - 2);
                            let y = 1 + (5 * row + 23 * col + 29 * lens.index()) % (rows - 2);
                            [
                                (x as f32 + (col % 3) as f32 * 0.21) / cols as f32,
                                (y as f32 + (row % 3) as f32 * 0.37) / rows as f32,
                            ]
                        }
                    }
                })
                .collect::<Vec<_>>()
        };
        let mut a = map(Lens::A);
        let b = map(Lens::B);
        let fma_uv = [0.871 / A_COLS as f32, 0.251 / A_ROWS as f32];
        for row in 10..=11 {
            for col in 10..=11 {
                a[row * 60 + col] = fma_uv;
            }
        }
        let maps = RetainedBaseMaps::from_lenses(LensPair { a, b }).unwrap();
        let expected = sample_source_belts(&sources, &maps).reduce_area_3x3();
        assert_eq!(
            expected.pixel(Lens::A, 10, 10),
            190,
            "scalar fixture no longer pins the source-FMA discriminator"
        );
        let pipeline = GpuSolverBeltPipeline::new(&device);
        let pending = pipeline
            .submit(
                &device,
                &queue,
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                &maps,
            )
            .unwrap();
        assert_eq!(pending.packed().size(), OUTPUT_BYTES);
        let actual = pending.read().unwrap();
        assert_eq!(
            actual.pixel(Lens::A, 10, 10),
            190,
            "GPU changed the source-FMA discriminator on {adapter}"
        );
        assert_eq!(
            actual.bytes(),
            expected.bytes(),
            "GPU solver belts differ from the scalar/native schedule on {adapter}"
        );

        // Pin the retained-map FMA schedule before source sampling can wash a
        // one-ULP UV difference out. This is the scalar oracle's discriminator
        // from `retained_map_preserves_studio_top_right_first_fma_order`.
        let quad = [
            [
                [f32::from_bits(1_064_954_653), f32::from_bits(1_051_416_063)],
                [f32::from_bits(1_064_974_894), f32::from_bits(1_051_403_380)],
            ],
            [
                [f32::from_bits(1_064_972_601), f32::from_bits(1_051_518_451)],
                [f32::from_bits(1_064_992_780), f32::from_bits(1_051_505_921)],
            ],
        ];
        let mut probe_a = vec![[0.0; 2]; RetainedBaseMaps::NODES_PER_LENS];
        for dr in 0..2 {
            for dc in 0..2 {
                probe_a[(1 + dr) * 60 + 47 + dc] = quad[dr][dc];
            }
        }
        let probe_maps = RetainedBaseMaps::from_lenses(LensPair {
            b: probe_a.clone(),
            a: probe_a,
        })
        .unwrap();
        let probe_map = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 retained FMA probe map"),
            size: probe_maps.bytes().len() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&probe_map, 0, probe_maps.bytes());
        let probe_output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 retained FMA probe output"),
            size: 8,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let probe_readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 retained FMA probe readback"),
            size: 8,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let probe_view_a = texture_a.create_view(&Default::default());
        let probe_view_b = texture_b.create_view(&Default::default());
        let probe_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 retained FMA probe"),
            layout: &pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&probe_view_a),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&probe_view_b),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: probe_map.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: probe_output.as_entire_binding(),
                },
            ],
        });
        let probe_source = format!("{SHADER}\n{RETAINED_FMA_PROBE}");
        let probe_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 retained FMA probe"),
            source: wgpu::ShaderSource::Wgsl(probe_source.into()),
        });
        let probe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 retained FMA probe"),
            bind_group_layouts: &[&pipeline.layout],
            immediate_size: 0,
        });
        let probe_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ONE X2 retained FMA probe"),
            layout: Some(&probe_layout),
            module: &probe_module,
            entry_point: Some("probe_retained_fma"),
            compilation_options: Default::default(),
            cache: None,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&probe_pipeline);
            pass.set_bind_group(0, &probe_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&probe_output, 0, &probe_readback, 0, 8);
        let submission = queue.submit([encoder.finish()]);
        let slice = probe_readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        let mapped = slice.get_mapped_range();
        let bits = [
            u32::from_ne_bytes(mapped[0..4].try_into().unwrap()),
            u32::from_ne_bytes(mapped[4..8].try_into().unwrap()),
        ];
        assert_eq!(bits, [1_064_967_376, 1_051_445_982]);
        drop(mapped);
        probe_readback.unmap();
    }

    const RETAINED_FMA_PROBE: &str = r#"
@compute @workgroup_size(1)
fn probe_retained_fma() {
    let third = bitcast<f32>(THIRD_BITS);
    let uv = sample_base(0u, 4.0 * third, 142.0 * third);
    output_words[0] = bitcast<u32>(uv.x);
    output_words[1] = bitcast<u32>(uv.y);
}
"#;

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

    fn gpu() -> Result<(wgpu::Device, wgpu::Queue, String), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
            .ok_or("no Vulkan adapter")?;
        let name = adapter.get_info().name;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("exact ONE X2 GPU solver-belt twin"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())?;
        Ok((device, queue, name))
    }
}
