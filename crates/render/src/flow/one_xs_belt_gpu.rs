//! GPU sampling and reduction for selected ONE X2 solver inputs.
//!
//! This is the GPU-shaped equivalent of [`super::one_xs_belt::sample_source_belts`]
//! followed by [`SourceBelts::reduce_area_3x3`](super::one_xs_belt::SourceBelts::reduce_area_3x3)
//! and Studio's selected 5-by-5 input Gaussian. Production playback consumes
//! its compact post-blur readback at the CPU estimator boundary. Horizontal
//! Q7 sums retain one u32 per logical byte; the vertical pass rounds, packs
//! four final U8 codes into one word and preserves the 129,600-byte A-then-B
//! boundary. No staging allocation or full CPU luma readback lies between the
//! imported R8 textures and those inputs.

use std::sync::mpsc;
use std::{error::Error, fmt};

use super::one_xs::gpu_context::OneXsGpuContext;
use super::one_xs::pis::gpu::GpuPisFlight;
use super::one_xs::temporal::{BlurredBelts, gaussian_blur};
use super::one_xs::{Lens, LensPair};
use super::one_xs_belt::{RetainedBaseMaps, SolverBelts, SourceImage, sample_source_belts};
use crate::Fallible;

const CODES_PER_WORD: usize = 4;
const OUTPUT_BYTES: u64 = SolverBelts::BYTES as u64;
const OUTPUT_WORDS: u32 = (SolverBelts::BYTES / CODES_PER_WORD) as u32;
const HORIZONTAL_BYTES: u64 = SolverBelts::BYTES as u64 * size_of::<u32>() as u64;
const WITNESS_BYTES: u64 = 2 * size_of::<u32>() as u64;
const WORKGROUP_SIZE: u32 = 64;
const _: () = assert!(SolverBelts::BYTES.is_multiple_of(CODES_PER_WORD));
const _: () = assert!(super::one_xs::COLS.is_multiple_of(CODES_PER_WORD));
const _: () = assert!(RetainedBaseMaps::NODES_PER_LENS.is_multiple_of(CODES_PER_WORD));

const QUALIFICATION_A_ROWS: usize = 127;
const QUALIFICATION_A_COLS: usize = 259;
const QUALIFICATION_B_ROWS: usize = 131;
const QUALIFICATION_B_COLS: usize = 263;
const QUALIFICATION_STRIDE: usize = 512;
const RETAINED_FMA_BITS: [u32; 2] = [1_064_967_376, 1_051_445_982];

/// A deterministic CPU/native oracle that qualifies the actual adapter before
/// selected playback can consume this shader. WGSL does not promise the FMA
/// and exceptional-float behavior the estimator needs, so construction fails
/// closed if this exact workload disagrees even once.
struct QualificationFixture {
    sources: LensPair<SourceImage>,
    maps: RetainedBaseMaps,
    expected_preblur: SolverBelts,
    expected_blurred: BlurredBelts,
}

#[derive(Debug, PartialEq, Eq)]
enum GpuQualificationError {
    SolverByte {
        lens: Lens,
        row: usize,
        col: usize,
        actual: u8,
        expected: u8,
    },
    RetainedMap {
        actual: [u32; 2],
        expected: [u32; 2],
    },
    BlurredByte {
        lens: Lens,
        row: usize,
        col: usize,
        actual: u8,
        expected: u8,
    },
}

impl fmt::Display for GpuQualificationError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SolverByte {
                lens,
                row,
                col,
                actual,
                expected,
            } => write!(
                output,
                "ONE X2 GPU arithmetic is not exact on this graphics device: lens {lens} solver row {row} column {col} is {actual}, expected {expected}"
            ),
            Self::RetainedMap { actual, expected } => write!(
                output,
                "ONE X2 GPU arithmetic is not exact on this graphics device: retained-map FMA wrote {actual:?}, expected {expected:?}"
            ),
            Self::BlurredByte {
                lens,
                row,
                col,
                actual,
                expected,
            } => write!(
                output,
                "ONE X2 GPU arithmetic is not exact on this graphics device: lens {lens} blurred row {row} column {col} is {actual}, expected {expected}"
            ),
        }
    }
}

impl Error for GpuQualificationError {}

fn qualification_fixture() -> QualificationFixture {
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
            // Native-order source-FMA discriminator: at row .251, column
            // .871 the selected answer is 190; top-left-first writes 189.
            pixels[0] = 17;
            pixels[1] = 201;
            pixels[cols] = 93;
            pixels[cols + 1] = 248;
        }
        SourceImage::from_compact(rows, cols, pixels)
            .expect("the static GPU qualification source has its declared shape")
    };
    let sources = LensPair {
        a: source(Lens::A, QUALIFICATION_A_ROWS, QUALIFICATION_A_COLS),
        b: source(Lens::B, QUALIFICATION_B_ROWS, QUALIFICATION_B_COLS),
    };
    let map = |lens: Lens| {
        (0..RetainedBaseMaps::NODES_PER_LENS)
            .map(|index| {
                let row = index / super::one_xs::COLS;
                let col = index % super::one_xs::COLS;
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
                            Lens::A => (QUALIFICATION_A_ROWS, QUALIFICATION_A_COLS),
                            Lens::B => (QUALIFICATION_B_ROWS, QUALIFICATION_B_COLS),
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
    let retained_fma_quad = [
        [
            [f32::from_bits(1_064_954_653), f32::from_bits(1_051_416_063)],
            [f32::from_bits(1_064_974_894), f32::from_bits(1_051_403_380)],
        ],
        [
            [f32::from_bits(1_064_972_601), f32::from_bits(1_051_518_451)],
            [f32::from_bits(1_064_992_780), f32::from_bits(1_051_505_921)],
        ],
    ];
    for dr in 0..2 {
        for dc in 0..2 {
            a[(1 + dr) * super::one_xs::COLS + 47 + dc] = retained_fma_quad[dr][dc];
        }
    }
    let fma_uv = [
        0.871 / QUALIFICATION_A_COLS as f32,
        0.251 / QUALIFICATION_A_ROWS as f32,
    ];
    for row in 10..=11 {
        for col in 10..=11 {
            a[row * super::one_xs::COLS + col] = fma_uv;
        }
    }
    let maps = RetainedBaseMaps::from_lenses(LensPair { a, b })
        .expect("the static GPU qualification maps have the retained shape");
    let expected_preblur = sample_source_belts(&sources, &maps).reduce_area_3x3();
    assert_eq!(
        expected_preblur.pixel(Lens::A, 10, 10),
        190,
        "the static GPU qualification source-FMA discriminator changed"
    );
    let expected_blurred = gaussian_blur(&expected_preblur);
    QualificationFixture {
        sources,
        maps,
        expected_preblur,
        expected_blurred,
    }
}

/// Direct retained-grid input for qualifying the integer Gaussian separately
/// from sampling. It plants isolated corner, edge and centre impulses; keeps
/// opposite A/B storage boundaries at distinct constants; and fills the rest
/// with alternating, ramp, constant and deterministic pseudorandom regions.
/// The complete comparison consequently exercises both reflect-101 axes and a
/// wide set of final Q14 rounding residues rather than relying on the sampled
/// source fixture to happen to cover them.
fn blur_qualification_fixture() -> SolverBelts {
    let centre = (super::one_xs::ROWS / 2, super::one_xs::COLS / 2);
    SolverBelts::from_fn(|lens, row, col| {
        let in_box = |at: (usize, usize), radius: usize| {
            row.abs_diff(at.0) <= radius && col.abs_diff(at.1) <= radius
        };
        match lens {
            Lens::A if in_box((0, 0), 3) => u8::from(row == 0 && col == 0) * 255,
            Lens::A if in_box((0, super::one_xs::COLS / 2), 3) => {
                u8::from(row == 0 && col == super::one_xs::COLS / 2) * 173
            }
            Lens::A if in_box(centre, 3) => u8::from((row, col) == centre) * 255,
            Lens::A if row >= super::one_xs::ROWS - 5 && col >= super::one_xs::COLS - 5 => 11,
            Lens::B if row < 5 && col < 5 => 241,
            Lens::B if row >= super::one_xs::ROWS - 4 && col >= super::one_xs::COLS - 4 => {
                u8::from(row == super::one_xs::ROWS - 1 && col == super::one_xs::COLS - 1) * 199
            }
            _ if row < 32 => u8::from((row + col + lens.index()).is_multiple_of(2)) * 255,
            _ if row < 96 => ((5 * row + 17 * col + 31 * lens.index()) % 256) as u8,
            _ if row < 128 => 137 + lens.index() as u8 * 41,
            _ => {
                let mut value = (row * super::one_xs::COLS + col) as u32
                    ^ (0x9e37_79b9u32.wrapping_mul(lens.index() as u32 + 1));
                value ^= value >> 16;
                value = value.wrapping_mul(0x7feb_352d);
                value ^= value >> 15;
                (value >> 24) as u8
            }
        }
    })
}

fn qualification_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    source: &SourceImage,
) -> wgpu::Texture {
    let mut padded = vec![0xee; QUALIFICATION_STRIDE * source.rows()];
    for row in 0..source.rows() {
        padded[row * QUALIFICATION_STRIDE..row * QUALIFICATION_STRIDE + source.cols()]
            .copy_from_slice(&source.pixels()[row * source.cols()..(row + 1) * source.cols()]);
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
            bytes_per_row: Some(QUALIFICATION_STRIDE as u32),
            rows_per_image: Some(source.rows() as u32),
        },
        texture.size(),
    );
    texture
}

/// The two exact R8 source textures in physical A/B order.
#[derive(Clone, Copy)]
pub(crate) struct SourceTextures<'a> {
    pub(crate) a: &'a wgpu::Texture,
    pub(crate) b: &'a wgpu::Texture,
}

enum SubmissionInput<'a> {
    Sampled { qualify_intermediates: bool },
    Preblurred(&'a SolverBelts),
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
    context: OneXsGpuContext,
    pipeline: wgpu::ComputePipeline,
    horizontal_pipeline: wgpu::ComputePipeline,
    vertical_pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    witness: wgpu::Buffer,
}

impl GpuSolverBeltPipeline {
    /// Build and qualify the exact arithmetic on the actual device.
    ///
    /// WGSL permits transformations that change native solver bytes. The
    /// qualification is therefore part of construction, not merely a test;
    /// an adapter that disagrees is refused with no CPU or approximate path.
    pub(crate) fn new(context: OneXsGpuContext) -> Fallible<Self> {
        Self::from_shader(context, SHADER)
    }

    fn from_shader(context: OneXsGpuContext, shader: &str) -> Fallible<Self> {
        let device = context.device();
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
            entries: &[
                texture(0),
                texture(1),
                storage(2, true),
                storage(3, false),
                storage(4, false),
                storage(5, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 GPU solver belts"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 GPU solver belts"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ONE X2 GPU solver belts"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("build_solver_belts"),
            compilation_options: Default::default(),
            cache: None,
        });
        let horizontal_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ONE X2 GPU horizontal Gaussian"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("blur_horizontal"),
                compilation_options: Default::default(),
                cache: None,
            });
        let vertical_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ONE X2 GPU vertical Gaussian"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("blur_vertical"),
            compilation_options: Default::default(),
            cache: None,
        });
        // Qualification reads this once before construction returns. Later
        // overlapping submissions may overwrite it because ordinary playback
        // deliberately never reads the witness.
        let witness = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 retained-map FMA witness"),
            size: WITNESS_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let built = Self {
            context,
            pipeline,
            horizontal_pipeline,
            vertical_pipeline,
            layout,
            witness,
        };
        built.qualify()?;
        Ok(built)
    }

    fn qualify(&self) -> Fallible<()> {
        let device = self.context.device();
        let queue = self.context.queue();
        let fixture = qualification_fixture();
        let texture_a = qualification_texture(
            device,
            queue,
            "ONE X2 GPU qualification source A",
            &fixture.sources.a,
        );
        let texture_b = qualification_texture(
            device,
            queue,
            "ONE X2 GPU qualification source B",
            &fixture.sources.b,
        );
        let pending = self.submit_inner(
            SourceTextures {
                a: &texture_a,
                b: &texture_b,
            },
            &fixture.maps,
            (),
            SubmissionInput::Sampled {
                qualify_intermediates: true,
            },
            true,
        )?;
        let (actual_blurred, actual_preblur, retained_bits) = pending.read_qualification()?;
        if let Some(index) = actual_preblur
            .bytes()
            .iter()
            .zip(fixture.expected_preblur.bytes())
            .position(|(actual, expected)| actual != expected)
        {
            let lens = if index < RetainedBaseMaps::NODES_PER_LENS {
                Lens::A
            } else {
                Lens::B
            };
            let local = index % RetainedBaseMaps::NODES_PER_LENS;
            let row = local / super::one_xs::COLS;
            let col = local % super::one_xs::COLS;
            return Err(GpuQualificationError::SolverByte {
                lens,
                row,
                col,
                actual: actual_preblur.bytes()[index],
                expected: fixture.expected_preblur.bytes()[index],
            }
            .into());
        }

        if retained_bits != RETAINED_FMA_BITS {
            return Err(GpuQualificationError::RetainedMap {
                actual: retained_bits,
                expected: RETAINED_FMA_BITS,
            }
            .into());
        }
        if let Some(index) = actual_blurred
            .bytes()
            .iter()
            .zip(fixture.expected_blurred.bytes())
            .position(|(actual, expected)| actual != expected)
        {
            let lens = if index < RetainedBaseMaps::NODES_PER_LENS {
                Lens::A
            } else {
                Lens::B
            };
            let local = index % RetainedBaseMaps::NODES_PER_LENS;
            let row = local / super::one_xs::COLS;
            let col = local % super::one_xs::COLS;
            return Err(GpuQualificationError::BlurredByte {
                lens,
                row,
                col,
                actual: actual_blurred.bytes()[index],
                expected: fixture.expected_blurred.bytes()[index],
            }
            .into());
        }
        let blur_input = blur_qualification_fixture();
        let expected_blur = gaussian_blur(&blur_input);
        let actual_blur = self
            .submit_inner(
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                &fixture.maps,
                (),
                SubmissionInput::Preblurred(&blur_input),
                true,
            )?
            .read()?;
        if let Some(index) = actual_blur
            .bytes()
            .iter()
            .zip(expected_blur.bytes())
            .position(|(actual, expected)| actual != expected)
        {
            let lens = if index < RetainedBaseMaps::NODES_PER_LENS {
                Lens::A
            } else {
                Lens::B
            };
            let local = index % RetainedBaseMaps::NODES_PER_LENS;
            let row = local / super::one_xs::COLS;
            let col = local % super::one_xs::COLS;
            return Err(GpuQualificationError::BlurredByte {
                lens,
                row,
                col,
                actual: actual_blur.bytes()[index],
                expected: expected_blur.bytes()[index],
            }
            .into());
        }
        Ok(())
    }

    /// Submit while retaining the owner of imported source images.
    ///
    /// A dmabuf texture aliases a decoder surface. Scene integration must pass
    /// the corresponding frame owner here; retaining only wgpu handles does
    /// not keep that external surface out of the decoder's pool.
    pub(crate) fn submit_retained<K>(
        &self,
        sources: SourceTextures<'_>,
        maps: &RetainedBaseMaps,
        source_owner: K,
    ) -> Fallible<PendingBlurredBelts<K>> {
        self.submit_inner(
            sources,
            maps,
            source_owner,
            SubmissionInput::Sampled {
                qualify_intermediates: false,
            },
            true,
        )
    }

    /// Submit the exact production producer without allocating or copying a
    /// CPU readback. The returned token retains the decoded source owner and
    /// exact capture flight until the device has completed the post-Gaussian
    /// payload.
    ///
    /// This is a staged GPU-estimator boundary. Selected playback does not use
    /// it until a qualified downstream consumer exists.
    #[allow(dead_code)]
    pub(crate) fn submit_resident_retained<K>(
        &self,
        sources: SourceTextures<'_>,
        maps: &RetainedBaseMaps,
        source_owner: K,
        flight: GpuPisFlight,
    ) -> Fallible<GpuBlurredBelts<K>> {
        let pending = self.submit_inner(
            sources,
            maps,
            source_owner,
            SubmissionInput::Sampled {
                qualify_intermediates: false,
            },
            false,
        )?;
        Ok(pending.into_resident(flight))
    }

    #[allow(clippy::too_many_arguments)]
    fn submit_inner<K>(
        &self,
        sources: SourceTextures<'_>,
        maps: &RetainedBaseMaps,
        source_owner: K,
        input: SubmissionInput<'_>,
        copy_to_cpu: bool,
    ) -> Fallible<PendingBlurredBelts<K>> {
        let device = self.context.device();
        let queue = self.context.queue();
        let qualify_intermediates = matches!(
            input,
            SubmissionInput::Sampled {
                qualify_intermediates: true
            }
        );
        let initial_preblur = match input {
            SubmissionInput::Sampled { .. } => None,
            SubmissionInput::Preblurred(belts) => Some(belts),
        };
        sources.validate()?;
        let map = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 retained base maps"),
            size: maps.bytes().len() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&map, 0, maps.bytes());
        let packed = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 packed blurred solver belts"),
            size: OUTPUT_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if let Some(initial) = initial_preblur {
            queue.write_buffer(&packed, 0, &pack_belts(initial));
        }
        let horizontal = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 horizontal Gaussian sums"),
            size: HORIZONTAL_BYTES,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let readback = copy_to_cpu.then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ONE X2 blurred solver belt readback"),
                size: OUTPUT_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });
        let preblur_readback = qualify_intermediates.then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ONE X2 pre-blur qualification readback"),
                size: OUTPUT_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });
        let witness_readback = qualify_intermediates.then(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ONE X2 retained-map FMA witness readback"),
                size: WITNESS_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
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
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.witness.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: horizontal.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ONE X2 GPU solver belts"),
        });
        if initial_preblur.is_none() {
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("ONE X2 GPU solver belts"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &resources, &[]);
                pass.dispatch_workgroups(OUTPUT_WORDS.div_ceil(WORKGROUP_SIZE), 1, 1);
            }
            if let Some(readback) = &preblur_readback {
                encoder.copy_buffer_to_buffer(&packed, 0, readback, 0, OUTPUT_BYTES);
            }
            if let Some(readback) = &witness_readback {
                encoder.copy_buffer_to_buffer(&self.witness, 0, readback, 0, WITNESS_BYTES);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 GPU horizontal Gaussian"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.horizontal_pipeline);
            pass.set_bind_group(0, &resources, &[]);
            pass.dispatch_workgroups((SolverBelts::BYTES as u32).div_ceil(WORKGROUP_SIZE), 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ONE X2 GPU vertical Gaussian"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.vertical_pipeline);
            pass.set_bind_group(0, &resources, &[]);
            pass.dispatch_workgroups(OUTPUT_WORDS.div_ceil(WORKGROUP_SIZE), 1, 1);
        }
        if let Some(readback) = &readback {
            encoder.copy_buffer_to_buffer(&packed, 0, readback, 0, OUTPUT_BYTES);
        }
        let submission = queue.submit([encoder.finish()]);
        Ok(PendingBlurredBelts {
            lease: SubmissionLease::new(self.context.clone(), submission, source_owner),
            _map: map,
            _packed: packed,
            _horizontal: horizontal,
            readback,
            preblur_readback,
            witness_readback,
            _resources: resources,
        })
    }
}

/// The exact device-owned submission whose source surface cannot be reused
/// until completion has been proved.
enum ExactSubmission {
    Device {
        context: OneXsGpuContext,
        index: wgpu::SubmissionIndex,
        #[cfg(test)]
        observer: Option<std::sync::Arc<std::sync::atomic::AtomicU8>>,
    },
    #[cfg(test)]
    Injected {
        outcome: InjectedWait,
        observer: std::sync::Arc<std::sync::atomic::AtomicU8>,
    },
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum InjectedWait {
    Success,
    Error,
    Panic,
}

impl ExactSubmission {
    fn wait(self) -> Fallible<()> {
        match self {
            Self::Device {
                context,
                index,
                #[cfg(test)]
                observer,
            } => {
                #[cfg(test)]
                if let Some(observer) = &observer {
                    observer.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                let result = context
                    .device()
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(index),
                        timeout: None,
                    })
                    .map(|_| ())
                    .map_err(Box::<dyn Error + Send + Sync>::from);
                #[cfg(test)]
                if let Some(observer) = &observer {
                    observer.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                result
            }
            #[cfg(test)]
            Self::Injected { outcome, observer } => {
                observer.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let result = match outcome {
                    InjectedWait::Success => Ok(()),
                    InjectedWait::Error => Err("injected ONE X2 submission poll failure".into()),
                    InjectedWait::Panic => panic!("injected ONE X2 submission poll panic"),
                };
                observer.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                result
            }
        }
    }

    fn submit_after<F>(&mut self, producer: &OneXsGpuContext, encode: F) -> Fallible<()>
    where
        F: FnOnce(&wgpu::Device) -> wgpu::CommandBuffer,
    {
        match self {
            Self::Device { context, index, .. } => {
                context.ensure_same(producer)?;
                let command = encode(context.device());
                *index = context.queue().submit([command]);
                Ok(())
            }
            #[cfg(test)]
            Self::Injected { .. } => {
                Err("injected ONE X2 GPU submission cannot accept a consumer".into())
            }
        }
    }

    #[cfg(test)]
    fn observe(&mut self, state: std::sync::Arc<std::sync::atomic::AtomicU8>) {
        match self {
            Self::Device { observer, .. } => *observer = Some(state),
            Self::Injected { observer, .. } => *observer = state,
        }
    }
}

/// Linear ownership of one external source through one exact GPU submission.
///
/// Completion success releases the source owner and disarms the lease. A poll
/// failure instead intentionally leaks that owner: without completion proof,
/// returning an aliased decoder surface to its pool would permit GPU/decoder
/// reuse races. Drop performs the same fail-closed completion when a caller
/// abandons a pending submission before normal readback.
struct SubmissionLease<K> {
    completion: Option<ExactSubmission>,
    source_owner: Option<K>,
}

impl<K> SubmissionLease<K> {
    fn new(context: OneXsGpuContext, index: wgpu::SubmissionIndex, source_owner: K) -> Self {
        Self {
            completion: Some(ExactSubmission::Device {
                context,
                index,
                #[cfg(test)]
                observer: None,
            }),
            source_owner: Some(source_owner),
        }
    }

    fn complete(&mut self) -> Fallible<()> {
        let Some(completion) = self.completion.take() else {
            return Ok(());
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| completion.wait())) {
            Ok(Ok(())) => {
                drop(self.source_owner.take());
                Ok(())
            }
            Ok(Err(error)) => {
                self.quarantine_owner();
                Err(error)
            }
            Err(payload) => {
                self.quarantine_owner();
                std::panic::resume_unwind(payload)
            }
        }
    }

    fn validate_provenance(&self, producer: &OneXsGpuContext) -> Fallible<()> {
        match &self.completion {
            Some(ExactSubmission::Device { context, .. }) => context.ensure_same(producer),
            #[cfg(test)]
            Some(ExactSubmission::Injected { .. }) => {
                Err("injected ONE X2 GPU submission has no device or queue provenance".into())
            }
            None => Err("ONE X2 GPU submission lease was already completed".into()),
        }
    }

    fn submit_after<F>(&mut self, producer: &OneXsGpuContext, encode: F) -> Fallible<()>
    where
        F: FnOnce(&wgpu::Device) -> wgpu::CommandBuffer,
    {
        let completion = self
            .completion
            .as_mut()
            .ok_or("ONE X2 GPU submission lease was already completed")?;
        completion.submit_after(producer, encode)
    }

    #[cfg(test)]
    fn injected(
        source_owner: K,
        outcome: InjectedWait,
        observer: std::sync::Arc<std::sync::atomic::AtomicU8>,
    ) -> Self {
        Self {
            completion: Some(ExactSubmission::Injected { outcome, observer }),
            source_owner: Some(source_owner),
        }
    }

    fn quarantine_owner(&mut self) {
        if let Some(owner) = self.source_owner.take() {
            // A failed or panicking poll is not proof that the GPU stopped
            // reading the imported decoder surface. Intentionally retain it
            // for the process lifetime so it cannot return to decoder reuse.
            std::mem::forget(owner);
        }
    }

    fn complete_for_drop(&mut self) {
        let Some(completion) = self.completion.take() else {
            return;
        };
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| completion.wait())) {
            Ok(Ok(())) => drop(self.source_owner.take()),
            Ok(Err(_)) | Err(_) => self.quarantine_owner(),
        }
    }

    #[cfg(test)]
    fn observe(&mut self, state: std::sync::Arc<std::sync::atomic::AtomicU8>) {
        if let Some(completion) = &mut self.completion {
            completion.observe(state);
        }
    }
}

impl<K> Drop for SubmissionLease<K> {
    fn drop(&mut self) {
        // Destructors cannot safely propagate a native-backend panic: doing so
        // while already unwinding would abort the process. Quarantine the
        // source on either an error or panic and swallow only at this terminal
        // cancellation boundary. Explicit completion preserves the original
        // error or panic after performing the same quarantine.
        self.complete_for_drop();
    }
}

/// One submitted GPU solver-belt transaction.
#[must_use = "the submitted ONE X2 solver belts have not been consumed"]
pub(crate) struct PendingBlurredBelts<K> {
    lease: SubmissionLease<K>,
    _map: wgpu::Buffer,
    /// Retained through either the CPU copy or the resident consumer.
    _packed: wgpu::Buffer,
    _horizontal: wgpu::Buffer,
    readback: Option<wgpu::Buffer>,
    preblur_readback: Option<wgpu::Buffer>,
    witness_readback: Option<wgpu::Buffer>,
    _resources: wgpu::BindGroup,
}

impl<K> PendingBlurredBelts<K> {
    /// The compact GPU-resident A-then-B payload, four U8 codes per word.
    #[cfg(test)]
    pub(crate) fn packed(&self) -> &wgpu::Buffer {
        &self._packed
    }

    #[cfg(test)]
    pub(crate) fn observe_completion(
        &mut self,
        state: std::sync::Arc<std::sync::atomic::AtomicU8>,
    ) {
        self.lease.observe(state);
    }

    /// Wait for and consume the exact compact post-Gaussian payload.
    ///
    /// The distinct return type prevents the CPU estimator from applying the
    /// selected Gaussian for a second time.
    pub(crate) fn read(self) -> Fallible<BlurredBelts> {
        Ok(self.read_inner()?.0)
    }

    /// Make a no-readback submission a GPU-resident, frame-bound post-Gaussian
    /// resource without waiting on the CPU.
    ///
    /// A later same-queue submission orders its reads after this producer.
    /// The token retains every producer resource and the imported source owner
    /// until that consumer reaches an explicit CPU re-entry boundary.
    fn into_resident(self, flight: GpuPisFlight) -> GpuBlurredBelts<K> {
        debug_assert!(self.readback.is_none());
        debug_assert!(self.preblur_readback.is_none());
        debug_assert!(self.witness_readback.is_none());
        GpuBlurredBelts {
            flight: Some(flight),
            lease: self.lease,
            packed: self._packed,
            _producer_map: self._map,
            _horizontal: self._horizontal,
            _resources: self._resources,
        }
    }

    fn read_qualification(self) -> Fallible<(BlurredBelts, SolverBelts, [u32; 2])> {
        let (blurred, preblur, witness) = self.read_inner()?;
        Ok((
            blurred,
            preblur.expect("qualification requested its pre-blur payload"),
            witness.expect("qualification requested its retained-map FMA witness"),
        ))
    }

    fn read_inner(mut self) -> Fallible<(BlurredBelts, Option<SolverBelts>, Option<[u32; 2]>)> {
        let readback = self
            .readback
            .as_ref()
            .ok_or("ONE X2 GPU-resident blurred solver belts have no CPU readback")?;
        let slice = readback.slice(..);
        let (mapped, answer) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = mapped.send(result);
        });
        let witness = self.witness_readback.as_ref().map(|buffer| {
            let slice = buffer.slice(..);
            let (mapped, answer) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = mapped.send(result);
            });
            (slice, answer)
        });
        let preblur = self.preblur_readback.as_ref().map(|buffer| {
            let slice = buffer.slice(..);
            let (mapped, answer) = mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = mapped.send(result);
            });
            (slice, answer)
        });
        self.lease.complete()?;
        answer.recv()??;
        if let Some((_, answer)) = &witness {
            answer.recv()??;
        }
        if let Some((_, answer)) = &preblur {
            answer.recv()??;
        }
        let mapped = slice.get_mapped_range();
        let belts = unpack_blurred_belts(&mapped)?;
        drop(mapped);
        readback.unmap();
        let preblur = preblur
            .map(|(slice, _)| {
                let mapped = slice.get_mapped_range();
                let belts = unpack_solver_belts(&mapped);
                drop(mapped);
                self.preblur_readback
                    .as_ref()
                    .expect("mapped pre-blur payload has its buffer")
                    .unmap();
                belts
            })
            .transpose()?;
        let witness = witness.map(|(slice, _)| {
            let mapped = slice.get_mapped_range();
            let bits = [
                u32::from_ne_bytes(mapped[0..4].try_into().unwrap()),
                u32::from_ne_bytes(mapped[4..8].try_into().unwrap()),
            ];
            drop(mapped);
            self.witness_readback
                .as_ref()
                .expect("mapped witness has its buffer")
                .unmap();
            bits
        });
        Ok((belts, preblur, witness))
    }
}

/// Exact post-Gaussian solver belts that have never crossed into CPU memory.
///
/// The token is deliberately neither cloneable nor publicly constructible.
/// Its capture generation and opaque frame identity travel with the storage
/// allocation, so a later preprocessing result cannot be admitted by numeric
/// frame index alone.
#[must_use = "the GPU-resident post-Gaussian belts have not been consumed"]
pub(crate) struct GpuBlurredBelts<K> {
    flight: Option<GpuPisFlight>,
    lease: SubmissionLease<K>,
    packed: wgpu::Buffer,
    _producer_map: wgpu::Buffer,
    _horizontal: wgpu::Buffer,
    _resources: wgpu::BindGroup,
}

impl<K> GpuBlurredBelts<K> {
    pub(crate) fn take_flight(&mut self) -> GpuPisFlight {
        self.flight
            .take()
            .expect("GPU-resident belts transfer their flight exactly once")
    }

    pub(crate) fn packed(&self) -> &wgpu::Buffer {
        &self.packed
    }

    pub(crate) fn validate_provenance(&self, context: &OneXsGpuContext) -> Fallible<()> {
        self.lease.validate_provenance(context)
    }

    /// Submit the concrete prepared-source transition on the lease's exact
    /// queue and replace its completion fence with that later submission.
    /// No caller can provide, omit or regress a detached submission index.
    pub(crate) fn submit_front_end<F>(
        &mut self,
        context: &OneXsGpuContext,
        encode: F,
    ) -> Fallible<()>
    where
        F: FnOnce(&wgpu::Device) -> wgpu::CommandBuffer,
    {
        self.lease.submit_after(context, encode)
    }

    pub(crate) fn complete(&mut self) -> Fallible<()> {
        self.lease.complete()
    }

    #[cfg(test)]
    pub(crate) fn observe_completion(
        &mut self,
        state: std::sync::Arc<std::sync::atomic::AtomicU8>,
    ) {
        self.lease.observe(state);
    }
}

#[cfg(test)]
pub(crate) fn resident_qualification_fixture<K>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source_owner: K,
    flight: GpuPisFlight,
) -> Fallible<(GpuBlurredBelts<K>, BlurredBelts)> {
    let fixture = qualification_fixture();
    let expected = fixture.expected_blurred.clone();
    let texture_a = qualification_texture(
        device,
        queue,
        "ONE X2 composed resident source A",
        &fixture.sources.a,
    );
    let texture_b = qualification_texture(
        device,
        queue,
        "ONE X2 composed resident source B",
        &fixture.sources.b,
    );
    let pipeline = GpuSolverBeltPipeline::new(OneXsGpuContext::new(device, queue))?;
    let belts = pipeline.submit_resident_retained(
        SourceTextures {
            a: &texture_a,
            b: &texture_b,
        },
        &fixture.maps,
        source_owner,
        flight,
    )?;
    Ok((belts, expected))
}

fn unpack_belt_lenses(words: &[u8]) -> LensPair<Vec<u8>> {
    let bytes = words
        .chunks_exact(size_of::<u32>())
        .flat_map(|word| u32::from_ne_bytes(word.try_into().unwrap()).to_le_bytes())
        .collect::<Vec<_>>();
    debug_assert_eq!(bytes.len(), SolverBelts::BYTES);
    LensPair {
        a: bytes[..RetainedBaseMaps::NODES_PER_LENS].to_vec(),
        b: bytes[RetainedBaseMaps::NODES_PER_LENS..].to_vec(),
    }
}

fn unpack_solver_belts(words: &[u8]) -> Fallible<SolverBelts> {
    SolverBelts::from_lenses(unpack_belt_lenses(words))
        .map_err(Box::<dyn Error + Send + Sync>::from)
}

fn unpack_blurred_belts(words: &[u8]) -> Fallible<BlurredBelts> {
    BlurredBelts::from_lenses(unpack_belt_lenses(words))
        .map_err(Box::<dyn Error + Send + Sync>::from)
}

fn pack_belts(belts: &SolverBelts) -> Vec<u8> {
    belts
        .bytes()
        .chunks_exact(CODES_PER_WORD)
        .flat_map(|codes| u32::from_le_bytes(codes.try_into().unwrap()).to_ne_bytes())
        .collect()
}

// The explicit `fma` chain and native `(1-coordinate)+floor(coordinate)`
// weights mirror the CPU oracle. WGSL permits a backend to expand `fma`, so
// byte identity remains an adapter-tested contract rather than a promise made
// from source spelling alone. NaN and infinity handling can also vary with
// backend finite-math policy. Runtime qualification and the required GPU twin
// below gate the actual adapter.
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
@group(0) @binding(4) var<storage, read_write> witness_words: array<u32>;
@group(0) @binding(5) var<storage, read_write> horizontal_codes: array<u32>;

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
            if index == COLS + 47u && dr == 1u && dc == 1u {
                witness_words[0] = bitcast<u32>(uv.x);
                witness_words[1] = bitcast<u32>(uv.y);
            }
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

fn packed_code(index: u32) -> u32 {
    let word = output_words[index / 4u];
    return (word >> (8u * (index % 4u))) & 255u;
}

fn reflect_101(position: i32, length: i32) -> u32 {
    if position < 0 {
        return u32(-position);
    }
    if position >= length {
        return u32(2 * length - position - 2);
    }
    return u32(position);
}

@compute @workgroup_size(64)
fn blur_horizontal(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if index >= TOTAL_CODES {
        return;
    }
    let lens = index / PIXELS_PER_LENS;
    let local = index - lens * PIXELS_PER_LENS;
    let row = local / COLS;
    let col = local - row * COLS;
    var sum = 0u;
    sum += 3u * packed_code(lens * PIXELS_PER_LENS + row * COLS + reflect_101(i32(col) - 2, i32(COLS)));
    sum += 29u * packed_code(lens * PIXELS_PER_LENS + row * COLS + reflect_101(i32(col) - 1, i32(COLS)));
    sum += 64u * packed_code(index);
    sum += 29u * packed_code(lens * PIXELS_PER_LENS + row * COLS + reflect_101(i32(col) + 1, i32(COLS)));
    sum += 3u * packed_code(lens * PIXELS_PER_LENS + row * COLS + reflect_101(i32(col) + 2, i32(COLS)));
    horizontal_codes[index] = sum;
}

fn vertical_code(index: u32) -> u32 {
    let lens = index / PIXELS_PER_LENS;
    let local = index - lens * PIXELS_PER_LENS;
    let row = local / COLS;
    let col = local - row * COLS;
    var sum = 0u;
    sum += 3u * horizontal_codes[lens * PIXELS_PER_LENS + reflect_101(i32(row) - 2, i32(ROWS)) * COLS + col];
    sum += 29u * horizontal_codes[lens * PIXELS_PER_LENS + reflect_101(i32(row) - 1, i32(ROWS)) * COLS + col];
    sum += 64u * horizontal_codes[index];
    sum += 29u * horizontal_codes[lens * PIXELS_PER_LENS + reflect_101(i32(row) + 1, i32(ROWS)) * COLS + col];
    sum += 3u * horizontal_codes[lens * PIXELS_PER_LENS + reflect_101(i32(row) + 2, i32(ROWS)) * COLS + col];
    return (sum + 8192u) >> 14u;
}

@compute @workgroup_size(64)
fn blur_vertical(@builtin(global_invocation_id) id: vec3<u32>) {
    let first = id.x * 4u;
    if first >= TOTAL_CODES {
        return;
    }
    var packed = 0u;
    for (var lane = 0u; lane < 4u; lane += 1u) {
        packed |= vertical_code(first + lane) << (8u * lane);
    }
    output_words[id.x] = packed;
}
"#;

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::time::Duration;

    use kjerag_media::FrameStamp;

    use super::*;

    struct DropProbe {
        wait_state: Arc<AtomicU8>,
        dropped: mpsc::Sender<u8>,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            let _ = self.dropped.send(self.wait_state.load(Ordering::SeqCst));
        }
    }

    #[test]
    fn resident_submission_returns_without_poll_and_early_drop_waits_before_owner_release() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}"
                );
                eprintln!("skipping ONE X2 resident ownership test: {why}");
                return;
            }
        };
        let fixture = qualification_fixture();
        let texture_a =
            qualification_texture(&device, &queue, "ONE X2 resident A", &fixture.sources.a);
        let texture_b =
            qualification_texture(&device, &queue, "ONE X2 resident B", &fixture.sources.b);
        let pipeline = GpuSolverBeltPipeline::new(OneXsGpuContext::new(&device, &queue))
            .unwrap_or_else(|error| panic!("GPU qualification failed on {adapter}: {error}"));
        let wait_state = Arc::new(AtomicU8::new(0));
        let (dropped, answer) = mpsc::channel();
        let mut resident = pipeline
            .submit_resident_retained(
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                &fixture.maps,
                DropProbe {
                    wait_state: Arc::clone(&wait_state),
                    dropped,
                },
                GpuPisFlight {
                    generation: 7,
                    frame: FrameStamp::for_test(11, Duration::from_secs(2), None),
                },
            )
            .unwrap();
        assert_eq!(
            wait_state.load(Ordering::SeqCst),
            0,
            "resident submission polled before returning"
        );
        assert!(
            matches!(answer.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "resident submission released its source owner before token drop"
        );
        resident.observe_completion(Arc::clone(&wait_state));
        drop(resident);
        assert_eq!(wait_state.load(Ordering::SeqCst), 2);
        assert_eq!(
            answer.recv().unwrap(),
            2,
            "source owner preceded exact wait"
        );
    }

    #[test]
    fn submission_lease_abandon_waits_before_releasing_source_owner() {
        let state = Arc::new(AtomicU8::new(0));
        let (dropped, answer) = mpsc::channel();
        let lease = SubmissionLease::injected(
            DropProbe {
                wait_state: Arc::clone(&state),
                dropped,
            },
            InjectedWait::Success,
            Arc::clone(&state),
        );
        assert_eq!(state.load(Ordering::SeqCst), 0);
        drop(lease);
        assert_eq!(answer.recv().unwrap(), 2, "owner preceded exact wait");
    }

    #[test]
    fn submission_lease_success_disarms_drop_without_a_second_wait() {
        let state = Arc::new(AtomicU8::new(0));
        let (dropped, answer) = mpsc::channel();
        let mut lease = SubmissionLease::injected(
            DropProbe {
                wait_state: Arc::clone(&state),
                dropped,
            },
            InjectedWait::Success,
            Arc::clone(&state),
        );
        lease.complete().unwrap();
        assert_eq!(answer.recv().unwrap(), 2, "owner preceded successful wait");
        state.store(7, Ordering::SeqCst);
        drop(lease);
        assert_eq!(
            state.load(Ordering::SeqCst),
            7,
            "disarmed lease waited again during drop"
        );
    }

    #[test]
    fn submission_lease_poll_failure_retains_source_owner_for_process_lifetime() {
        let state = Arc::new(AtomicU8::new(0));
        let owner = Arc::new(());
        let retained = Arc::downgrade(&owner);
        let mut lease =
            SubmissionLease::injected(owner.clone(), InjectedWait::Error, Arc::clone(&state));
        let error = lease.complete().unwrap_err();
        assert_eq!(error.to_string(), "injected ONE X2 submission poll failure");
        assert_eq!(state.load(Ordering::SeqCst), 2);
        drop(lease);
        drop(owner);
        assert!(
            retained.upgrade().is_some(),
            "poll failure returned the source owner to possible decoder reuse"
        );
    }

    #[test]
    fn submission_lease_explicit_panic_retains_owner_and_preserves_panic() {
        let state = Arc::new(AtomicU8::new(0));
        let owner = Arc::new(());
        let retained = Arc::downgrade(&owner);
        let mut lease =
            SubmissionLease::injected(owner.clone(), InjectedWait::Panic, Arc::clone(&state));
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = lease.complete();
        }))
        .expect_err("injected native-backend panic was not preserved");
        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&"injected ONE X2 submission poll panic")
        );
        assert_eq!(state.load(Ordering::SeqCst), 1);
        drop(lease);
        drop(owner);
        assert!(
            retained.upgrade().is_some(),
            "poll panic returned the source owner to possible decoder reuse"
        );
    }

    #[test]
    fn submission_lease_drop_swallows_poll_panic_and_retains_owner() {
        let state = Arc::new(AtomicU8::new(0));
        let owner = Arc::new(());
        let retained = Arc::downgrade(&owner);
        let lease =
            SubmissionLease::injected(owner.clone(), InjectedWait::Panic, Arc::clone(&state));
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(lease)))
            .expect("lease Drop propagated a native-backend panic");
        assert_eq!(state.load(Ordering::SeqCst), 1);
        drop(owner);
        assert!(
            retained.upgrade().is_some(),
            "panicking Drop returned the source owner to possible decoder reuse"
        );
    }

    const DOUBLE_UNWIND_CHILD: &str = "KJERAG_TEST_BELT_LEASE_DOUBLE_UNWIND_CHILD";
    const DOUBLE_UNWIND_MARKER: &str = "ONE X2 submission lease double unwind passed";

    struct LeaseDropGuard(Option<SubmissionLease<Arc<()>>>);

    impl Drop for LeaseDropGuard {
        fn drop(&mut self) {
            // This runs while the outer panic is already unwinding. If the
            // lease ever lets its injected poll panic escape, Rust aborts this
            // process for the double panic. The controller test deliberately
            // confines that failure to a child test process.
            drop(self.0.take());
        }
    }

    #[test]
    fn submission_lease_double_unwind_child() {
        if std::env::var_os(DOUBLE_UNWIND_CHILD).is_none() {
            return;
        }

        let state = Arc::new(AtomicU8::new(0));
        let owner = Arc::new(());
        let retained = Arc::downgrade(&owner);
        let lease =
            SubmissionLease::injected(owner.clone(), InjectedWait::Panic, Arc::clone(&state));
        let outer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = LeaseDropGuard(Some(lease));
            panic!("injected ONE X2 outer unwind");
        }))
        .expect_err("the outer panic did not reach its catch boundary");
        assert_eq!(
            outer.downcast_ref::<&str>(),
            Some(&"injected ONE X2 outer unwind"),
            "the lease replaced the active outer panic payload"
        );
        assert_eq!(
            state.load(Ordering::SeqCst),
            1,
            "the injected poll panic did not occur during lease Drop"
        );
        drop(owner);
        assert!(
            retained.upgrade().is_some(),
            "double-unwind quarantine released the source owner"
        );
        eprintln!("{DOUBLE_UNWIND_MARKER}");
    }

    #[test]
    fn submission_lease_drop_during_outer_unwind_is_process_safe() {
        let helper = "flow::one_xs_belt_gpu::tests::submission_lease_double_unwind_child";
        let output = std::process::Command::new(
            std::env::current_exe().expect("the test harness has an executable path"),
        )
        .args(["--exact", helper, "--nocapture"])
        .env(DOUBLE_UNWIND_CHILD, "1")
        .output()
        .expect("could not start the isolated double-unwind helper");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "isolated double-unwind helper failed with {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status
        );
        assert!(
            stdout.contains(DOUBLE_UNWIND_MARKER) || stderr.contains(DOUBLE_UNWIND_MARKER),
            "isolated helper did not prove outer-payload and quarantine checks\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }

    #[test]
    fn submission_lease_accepts_cloned_pair_and_refuses_foreign_pair_before_encoding() {
        let ((device, queue), (foreign_device, foreign_queue)) = match two_gpu_pairs() {
            Ok(gpu) => gpu,
            Err(why) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => {
                eprintln!("skipping ONE X2 GPU context identity: {why}");
                return;
            }
            Err(why) => panic!("Vulkan GPU required for ONE X2 context identity: {why}"),
        };
        assert_ne!(
            device, foreign_device,
            "same-instance requests reused one device handle"
        );
        let context = OneXsGpuContext::new(&device, &queue);
        let cloned = OneXsGpuContext::new(&device, &queue);
        context.ensure_same(&cloned).unwrap();
        let first = queue.submit(std::iter::empty());
        let mut lease = SubmissionLease::new(context.clone(), first, ());
        let encoded = Arc::new(AtomicU8::new(0));
        let foreign = OneXsGpuContext::new(&foreign_device, &foreign_queue);
        let encoded_by_foreign = Arc::clone(&encoded);
        let error = lease
            .submit_after(&foreign, move |_| {
                encoded_by_foreign.fetch_add(1, Ordering::SeqCst);
                panic!("foreign ONE X2 context reached command encoding")
            })
            .expect_err("foreign ONE X2 context was accepted");
        assert_eq!(
            error.to_string(),
            "ONE X2 GPU submission crossed a different device or queue"
        );
        assert_eq!(
            encoded.load(Ordering::SeqCst),
            0,
            "foreign context encoded work"
        );
        lease
            .submit_after(&cloned, |device| {
                device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("ONE X2 cloned-context acceptance"),
                    })
                    .finish()
            })
            .unwrap();
        lease.complete().unwrap();
    }

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

        let fixture = qualification_fixture();
        let texture_a =
            qualification_texture(&device, &queue, "ONE X2 odd padded A", &fixture.sources.a);
        let texture_b =
            qualification_texture(&device, &queue, "ONE X2 odd padded B", &fixture.sources.b);
        let pipeline = GpuSolverBeltPipeline::new(OneXsGpuContext::new(&device, &queue))
            .unwrap_or_else(|error| panic!("GPU qualification failed on {adapter}: {error}"));
        let pending = pipeline
            .submit_inner(
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                &fixture.maps,
                (),
                SubmissionInput::Sampled {
                    qualify_intermediates: true,
                },
                true,
            )
            .unwrap();
        assert_eq!(pending.packed().size(), OUTPUT_BYTES);
        let (actual_blurred, actual_preblur, retained_bits) = pending.read_qualification().unwrap();
        assert_eq!(
            actual_preblur.pixel(Lens::A, 10, 10),
            190,
            "GPU changed the source-FMA discriminator on {adapter}"
        );
        assert_eq!(
            actual_preblur.bytes(),
            fixture.expected_preblur.bytes(),
            "GPU pre-blur solver belts differ from the scalar/native schedule on {adapter}"
        );
        assert_eq!(retained_bits, RETAINED_FMA_BITS);
        assert_eq!(
            actual_blurred.bytes(),
            fixture.expected_blurred.bytes(),
            "GPU blurred solver belts differ from the CPU/native schedule on {adapter}"
        );
    }

    #[test]
    fn gpu_gaussian_matches_complete_direct_retained_fixture() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}"
                );
                eprintln!("skipping direct ONE X2 GPU Gaussian twin: {why}");
                return;
            }
        };
        let fixture = qualification_fixture();
        let texture_a =
            qualification_texture(&device, &queue, "ONE X2 blur fixture A", &fixture.sources.a);
        let texture_b =
            qualification_texture(&device, &queue, "ONE X2 blur fixture B", &fixture.sources.b);
        let pipeline = GpuSolverBeltPipeline::new(OneXsGpuContext::new(&device, &queue))
            .unwrap_or_else(|error| panic!("GPU qualification failed on {adapter}: {error}"));
        let input = blur_qualification_fixture();
        assert_eq!(input.pixel(Lens::A, 0, 0), 255, "corner impulse");
        assert_eq!(
            input.pixel(Lens::A, 0, super::super::one_xs::COLS / 2),
            173,
            "edge impulse"
        );
        assert_eq!(
            input.pixel(
                Lens::A,
                super::super::one_xs::ROWS / 2,
                super::super::one_xs::COLS / 2,
            ),
            255,
            "centre impulse"
        );
        assert_eq!(
            input.pixel(
                Lens::A,
                super::super::one_xs::ROWS - 1,
                super::super::one_xs::COLS - 1,
            ),
            11,
            "lens A storage boundary"
        );
        assert_eq!(input.pixel(Lens::B, 0, 0), 241, "lens B storage boundary");
        let actual = pipeline
            .submit_inner(
                SourceTextures {
                    a: &texture_a,
                    b: &texture_b,
                },
                &fixture.maps,
                (),
                SubmissionInput::Preblurred(&input),
                true,
            )
            .unwrap()
            .read()
            .unwrap();
        assert_eq!(
            actual.bytes(),
            gaussian_blur(&input).bytes(),
            "GPU Gaussian differs from the complete CPU/native fixture on {adapter}"
        );
    }

    #[test]
    fn runtime_qualification_refuses_changed_solver_arithmetic() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}"
                );
                eprintln!("skipping ONE X2 GPU qualification refusal test: {why}");
                return;
            }
        };
        let broken = SHADER.replacen("return (sum + 4u) / 9u;", "return 0u;", 1);
        assert_ne!(
            broken, SHADER,
            "the solver mutation did not find its target"
        );
        let error = match GpuSolverBeltPipeline::from_shader(
            OneXsGpuContext::new(&device, &queue),
            &broken,
        ) {
            Ok(_) => panic!("changed ONE X2 GPU arithmetic was accepted on {adapter}"),
            Err(error) => error,
        };
        assert!(
            matches!(
                error.downcast_ref::<GpuQualificationError>(),
                Some(GpuQualificationError::SolverByte { .. })
            ),
            "changed arithmetic returned the wrong failure on {adapter}: {error}"
        );
    }

    #[test]
    fn runtime_qualification_uses_production_entry_for_retained_fma() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}"
                );
                eprintln!("skipping ONE X2 production-entry qualification test: {why}");
                return;
            }
        };
        if let Err(error) = GpuSolverBeltPipeline::new(OneXsGpuContext::new(&device, &queue)) {
            assert!(
                std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                "KJERAG_REQUIRE_GPU is set and {adapter} fails the baseline ONE X2 GPU qualification: {error}"
            );
            eprintln!(
                "skipping production-entry mutation on an adapter that fails baseline qualification: {adapter}: {error}"
            );
            return;
        }
        let broken = SHADER.replacen(
            "witness_words[0] = bitcast<u32>(uv.x);",
            "witness_words[0] = bitcast<u32>(uv.x) + 1u;",
            1,
        );
        assert_ne!(
            broken, SHADER,
            "the production discriminator mutation did not find its target"
        );
        let error = match GpuSolverBeltPipeline::from_shader(
            OneXsGpuContext::new(&device, &queue),
            &broken,
        ) {
            Ok(_) => panic!("changed ONE X2 production discriminator was accepted on {adapter}"),
            Err(error) => error,
        };
        assert!(
            matches!(
                error.downcast_ref::<GpuQualificationError>(),
                Some(GpuQualificationError::RetainedMap { .. })
            ),
            "changed production discriminator returned the wrong failure on {adapter}: {error}"
        );
    }

    #[test]
    fn runtime_qualification_refuses_changed_gaussian_rounding() {
        let (device, queue, adapter) = match gpu() {
            Ok(gpu) => gpu,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}"
                );
                eprintln!("skipping ONE X2 Gaussian qualification test: {why}");
                return;
            }
        };
        if let Err(error) = GpuSolverBeltPipeline::new(OneXsGpuContext::new(&device, &queue)) {
            assert!(
                std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                "KJERAG_REQUIRE_GPU is set and {adapter} fails the baseline ONE X2 GPU qualification: {error}"
            );
            eprintln!(
                "skipping Gaussian mutation on an adapter that fails baseline qualification: {adapter}: {error}"
            );
            return;
        }
        let broken = SHADER.replacen("return (sum + 8192u) >> 14u;", "return sum >> 14u;", 1);
        assert_ne!(broken, SHADER, "the Gaussian mutation found no target");
        let error = match GpuSolverBeltPipeline::from_shader(
            OneXsGpuContext::new(&device, &queue),
            &broken,
        ) {
            Ok(_) => panic!("changed ONE X2 Gaussian was accepted on {adapter}"),
            Err(error) => error,
        };
        assert!(
            matches!(
                error.downcast_ref::<GpuQualificationError>(),
                Some(GpuQualificationError::BlurredByte { .. })
            ),
            "changed Gaussian returned the wrong failure on {adapter}: {error}"
        );
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

    type GpuPair = (wgpu::Device, wgpu::Queue);

    fn two_gpu_pairs() -> Result<(GpuPair, GpuPair), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
            .ok_or("no Vulkan adapter")?;
        let request = |label| {
            block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some(label),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            }))
            .map_err(|error| error.to_string())
        };
        Ok((
            request("exact ONE X2 primary GPU context")?,
            request("exact ONE X2 foreign GPU context")?,
        ))
    }
}
