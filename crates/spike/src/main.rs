//! M0 pipeline proof: the frame path from docs/ARCHITECTURE.md, measured.
//!
//! VA-API decode -> `av_hwframe_map` to DRM_PRIME -> per-plane wgpu import
//! -> one WGSL pass (NV12 to RGB, no projection) -> PNG. The same run is
//! repeated over the `av_hwframe_transfer_data` copy path, so the zero-copy
//! saving is a measured number rather than a claim.
//!
//! ```sh
//! cargo run --release -p kjerag-spike -- <file.insv> [frames] [stream]
//! ```
//!
//! PNGs land in ./scratch/, which is gitignored: frames from real footage
//! are personal video and this repo is public.

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ffmpeg_next as ff;
use kjerag_media::{DrmFrame, Fallible, HwDevice, SwFrame, open_decoder};
use kjerag_render::{Extent, Planes, Size, dmabuf};

/// Offscreen render target edge. Small on purpose: the spike measures the
/// frame path, not a display, and reading back a 3840 square costs 59 MB.
const OUT_EDGE: u32 = 1024;

fn main() -> Fallible<()> {
    let args: Vec<String> = std::env::args().collect();
    let input = PathBuf::from(
        args.get(1)
            .ok_or("usage: spike <file.insv> [frames] [stream]")?,
    );
    let frames: u32 = parse_arg(&args, 2, 300)?;
    let stream: usize = parse_arg(&args, 3, 0)?;

    let scratch = PathBuf::from("scratch");
    fs::create_dir_all(&scratch)?;

    ff::init()?;
    let gpu = Gpu::new()?;
    println!("gpu:    {}", gpu.adapter.get_info().name);
    println!("device: {}", dmabuf::device_report(&gpu.device));
    println!(
        "input:  {} stream {stream}, {frames} frames",
        input.display()
    );

    let zero_copy = run(&gpu, &input, stream, frames, Delivery::ZeroCopy, &scratch)?;
    let copy = run(&gpu, &input, stream, frames, Delivery::Copy, &scratch)?;

    println!("\n{}", Stats::header());
    println!("{}", zero_copy.row(Delivery::ZeroCopy.tag()));
    println!("{}", copy.row(Delivery::Copy.tag()));
    Ok(())
}

fn parse_arg<T: std::str::FromStr>(args: &[String], i: usize, fallback: T) -> Fallible<T>
where
    T::Err: std::fmt::Display,
{
    match args.get(i) {
        None => Ok(fallback),
        Some(raw) => raw
            .parse()
            .map_err(|e| format!("bad argument {i}: {e}").into()),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Delivery {
    /// `av_hwframe_map` to DRM_PRIME, then import the dmabuf planes.
    ZeroCopy,
    /// `av_hwframe_transfer_data` to system memory, then upload the planes.
    Copy,
}

impl Delivery {
    fn tag(self) -> &'static str {
        match self {
            Delivery::ZeroCopy => "zero-copy",
            Delivery::Copy => "copy",
        }
    }
}

// ---------------------------------------------------------------- the run

#[derive(Default)]
struct Stats {
    frames: u32,
    demux: Duration,
    decode: Duration,
    deliver: Duration,
    import: Duration,
    render: Duration,
    wall: Duration,
}

impl Stats {
    fn header() -> String {
        format!(
            "{:<10} {:>6} {:>8} {:>9} {:>9} {:>8} {:>8} {:>7}",
            "path", "frames", "demux", "decode", "deliver", "import", "render", "fps"
        )
    }

    fn row(&self, name: &str) -> String {
        let ms = |d: Duration| d.as_secs_f64() * 1000.0 / f64::from(self.frames.max(1));
        format!(
            "{:<10} {:>6} {:>8.2} {:>9.2} {:>9.2} {:>8.2} {:>8.2} {:>7.1}",
            name,
            self.frames,
            ms(self.demux),
            ms(self.decode),
            ms(self.deliver),
            ms(self.import),
            ms(self.render),
            f64::from(self.frames) / self.wall.as_secs_f64(),
        )
    }
}

fn run(
    gpu: &Gpu,
    input: &Path,
    stream: usize,
    want: u32,
    delivery: Delivery,
    scratch: &Path,
) -> Fallible<Stats> {
    let hw = HwDevice::vaapi()?;
    let mut ictx = ff::format::input(&input)?;
    let mut decoder = open_decoder(&ictx, stream, &hw)?;
    let luma = Size::new(decoder.width(), decoder.height());
    let staging = (delivery == Delivery::Copy).then(|| gpu.staging_planes(luma));

    let mut frame = ff::frame::Video::empty();
    let mut stats = Stats::default();
    let start = Instant::now();
    let mut packets = ictx.packets();

    while stats.frames < want {
        let t = Instant::now();
        let Some((from, packet)) = packets.next() else {
            break;
        };
        stats.demux += t.elapsed();
        if from.index() != stream {
            continue;
        }

        let t = Instant::now();
        decoder.send_packet(&packet)?;
        stats.decode += t.elapsed();

        while stats.frames < want {
            let t = Instant::now();
            let got = decoder.receive_frame(&mut frame).is_ok();
            stats.decode += t.elapsed();
            if !got {
                break;
            }

            match delivery {
                Delivery::ZeroCopy => zero_copy_frame(gpu, &frame, luma, &mut stats)?,
                Delivery::Copy => {
                    let planes = staging.as_ref().ok_or("copy path has no staging planes")?;
                    copy_frame(gpu, &frame, luma, planes, &mut stats)?;
                }
            }

            if stats.frames == 0 {
                gpu.write_png(&scratch.join(format!("{}-frame0.png", delivery.tag())))?;
            }
            stats.frames += 1;
        }
    }
    stats.wall = start.elapsed();

    if stats.frames < want {
        return Err(format!("only {} of {want} frames decoded", stats.frames).into());
    }
    println!("{}: done", delivery.tag());
    Ok(stats)
}

fn zero_copy_frame(
    gpu: &Gpu,
    frame: &ff::frame::Video,
    luma: Size,
    stats: &mut Stats,
) -> Fallible<()> {
    // `av_hwframe_map` with MAP_READ calls `vaSyncSurface` before exporting
    // (ffmpeg 6.1 libavutil/hwcontext_vaapi.c:1337), so this stage is mostly
    // the wait for the decode this spike never overlaps.
    let t = Instant::now();
    let mapped = DrmFrame::map(frame)?;
    stats.deliver += t.elapsed();

    if stats.frames == 0 {
        println!("drm:    {}", mapped.describe());
    }

    let t = Instant::now();
    let planes = dmabuf::import(&gpu.device, mapped.descriptor(), luma)?;
    stats.import += t.elapsed();

    let t = Instant::now();
    gpu.render(&planes)?;
    stats.render += t.elapsed();
    // `render` waited for the pass, so the decoder's surface is free again.
    Ok(())
}

fn copy_frame(
    gpu: &Gpu,
    frame: &ff::frame::Video,
    luma: Size,
    planes: &Planes,
    stats: &mut Stats,
) -> Fallible<()> {
    let t = Instant::now();
    let transferred = SwFrame::transfer(frame)?;
    stats.deliver += t.elapsed();

    let t = Instant::now();
    gpu.upload(planes, &transferred, luma);
    stats.import += t.elapsed();

    let t = Instant::now();
    gpu.render(planes)?;
    stats.render += t.elapsed();
    Ok(())
}

// ----------------------------------------------------------------- gpu side

struct Gpu {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    target: wgpu::Texture,
    readback: wgpu::Buffer,
}

impl Gpu {
    fn new() -> Fallible<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))?;
        let (device, queue) = dmabuf::open_device(&adapter)?;

        let (pipeline, layout) = build_pipeline(&device);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("target"),
            size: Size::new(OUT_EDGE, OUT_EDGE).extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(OUT_EDGE) * u64::from(OUT_EDGE) * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Ok(Self {
            adapter,
            device,
            queue,
            pipeline,
            layout,
            sampler,
            target,
            readback,
        })
    }

    /// Ordinary textures for the copy path to upload into.
    fn staging_planes(&self, luma: Size) -> Planes {
        let make = |format, size: Size| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("staging"),
                size: size.extent(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        Planes {
            luma: make(wgpu::TextureFormat::R8Unorm, luma),
            chroma: make(wgpu::TextureFormat::Rg8Unorm, luma.halved()),
        }
    }

    fn upload(&self, planes: &Planes, frame: &SwFrame, luma: Size) {
        let targets = [(&planes.luma, luma), (&planes.chroma, luma.halved())];
        for (index, (texture, size)) in targets.into_iter().enumerate() {
            let (bytes, stride) = frame.plane(index, size.height);
            self.queue.write_texture(
                texture.as_image_copy(),
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(size.height),
                },
                size.extent(),
            );
        }
    }

    fn render(&self, planes: &Planes) -> Fallible<()> {
        let view = |texture: &wgpu::Texture| texture.create_view(&Default::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("planes"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view(&planes.luma)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view(&planes.chroma)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let target = view(&self.target);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nv12"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
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
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        let index = self.queue.submit([encoder.finish()]);
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: None,
        })?;
        Ok(())
    }

    fn write_png(&self, path: &Path) -> Fallible<()> {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(OUT_EDGE * 4),
                    rows_per_image: Some(OUT_EDGE),
                },
            },
            Size::new(OUT_EDGE, OUT_EDGE).extent(),
        );
        self.queue.submit([encoder.finish()]);
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely())?;

        let view = self.readback.slice(..).get_mapped_range();
        let mut png = png::Encoder::new(BufWriter::new(File::create(path)?), OUT_EDGE, OUT_EDGE);
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()?.write_image_data(&view)?;
        drop(view);
        self.readback.unmap();
        println!("wrote {}", path.display());
        Ok(())
    }
}

const SHADER: &str = r#"
struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
  let x = f32((i << 1u) & 2u);
  let y = f32(i & 2u);
  var out: VsOut;
  out.uv = vec2<f32>(x, y);
  out.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  return out;
}

@group(0) @binding(0) var luma: texture_2d<f32>;
@group(0) @binding(1) var chroma: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

// BT.709 full range: ffprobe reports bt709 and the camera writes yuvj420p.
// DRM_FORMAT_GR88 is little endian G:R, so .r is Cb and .g is Cr.
@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
  let y = textureSample(luma, samp, in.uv).r;
  let c = textureSample(chroma, samp, in.uv).rg - vec2<f32>(0.5, 0.5);
  return vec4<f32>(
    y + 1.5748 * c.g,
    y - 0.1873 * c.r - 0.4681 * c.g,
    y + 1.8556 * c.r,
    1.0,
  );
}
"#;

fn build_pipeline(device: &wgpu::Device) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("nv12"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
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
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("planes"),
        entries: &[
            texture(0),
            texture(1),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("nv12"),
        bind_group_layouts: &[&layout],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("nv12"),
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
            targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    (pipeline, layout)
}
