//! Headless reframe: the app's own projection pass, rendered to a PNG.
//!
//! The instrument the angle conventions were settled with, and the only way
//! to look at a reframed frame without a compositor. It builds the same
//! [`ScenePipeline`] the shader widget builds and feeds it the same
//! primitive, so what lands in the PNG is what the window would show.
//!
//! ```sh
//! cargo run --release -p kyerag-spike --bin reframe -- <file.insv>
//! cargo run --release -p kyerag-spike --bin reframe -- <file.insv> yaw=40 pitch=-15 fov=60
//! ```
//!
//! Arguments after the path are `key=value`: `yaw`, `pitch` and `fov` in
//! degrees, `size` as the output edge in pixels, `out` as the file name.
//!
//! PNGs land in ./scratch/, which is gitignored: frames from real footage
//! are personal video and this repo is public.

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kyerag_media::Fallible;
use kyerag_render::{Camera, Extent, Frame, Scene, ScenePipeline, Size, dmabuf};

/// Not sRGB, so the shader writes the video's own gamma-encoded numbers
/// straight out and a PNG viewer shows what the window shows.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The output is square, which is load bearing for the roll check: at yaw
/// and pitch 0 the candidate roll conventions differ by a rotation about the
/// output centre, and only a square output can be compared to its own
/// rotation.
const DEFAULT_EDGE: u32 = 1024;

struct Options {
    input: PathBuf,
    camera: Camera,
    edge: u32,
    out: PathBuf,
}

fn main() -> Fallible<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    fs::create_dir_all(options.out.parent().unwrap_or(Path::new(".")))?;

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))?;
    let (device, queue) = dmabuf::open_device(&adapter)?;
    println!("gpu:    {}", adapter.get_info().name);

    let scene = Scene::new(Some(Arc::new(Frame::pending(options.input.clone()))));
    let primitive = scene.primitive(options.camera);
    let mut pipeline = ScenePipeline::new(&device, FORMAT);
    pipeline.prepare(&primitive, &device, &queue, 1.0);

    let target = Target::new(&device, options.edge);
    target.render(&device, &queue, &pipeline)?;
    target.write_png(&device, &queue, &options.out)?;

    println!(
        "wrote {} at yaw {:.1}, pitch {:.1}, fov {:.1}",
        options.out.display(),
        options.camera.yaw.to_degrees(),
        options.camera.pitch.to_degrees(),
        options.camera.fov.to_degrees(),
    );
    Ok(())
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Fallible<Self> {
        let input = PathBuf::from(args.next().ok_or(USAGE)?);
        let mut camera = Camera::default();
        let mut edge = DEFAULT_EDGE;
        let mut out = None;

        for arg in args {
            let (key, value) = arg.split_once('=').ok_or(USAGE)?;
            match key {
                "yaw" => camera.yaw = value.parse::<f32>()?.to_radians(),
                "pitch" => camera.pitch = value.parse::<f32>()?.to_radians(),
                "fov" => camera.fov = value.parse::<f32>()?.to_radians(),
                "size" => edge = value.parse()?,
                "out" => out = Some(value.to_owned()),
                _ => return Err(format!("unknown argument {key}. {USAGE}").into()),
            }
        }

        let name = out.unwrap_or_else(|| {
            format!(
                "reframe-yaw{:.0}-pitch{:.0}-fov{:.0}.png",
                camera.yaw.to_degrees(),
                camera.pitch.to_degrees(),
                camera.fov.to_degrees(),
            )
        });
        Ok(Self {
            input,
            camera,
            edge,
            out: PathBuf::from("scratch").join(name),
        })
    }
}

const USAGE: &str =
    "usage: reframe <file.insv> [yaw=deg] [pitch=deg] [fov=deg] [size=px] [out=name.png]";

/// An offscreen colour target and the buffer its pixels are read back into.
struct Target {
    texture: wgpu::Texture,
    readback: wgpu::Buffer,
    edge: u32,
}

impl Target {
    fn new(device: &wgpu::Device, edge: u32) -> Self {
        Self {
            texture: device.create_texture(&wgpu::TextureDescriptor {
                label: Some("reframe"),
                size: Size::new(edge, edge).extent(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: u64::from(edge) * u64::from(edge) * 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            edge,
        }
    }

    fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &ScenePipeline,
    ) -> Fallible<()> {
        let view = self.texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reframe"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pipeline.draw(&mut pass);
        }
        let index = queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: None,
        })?;
        Ok(())
    }

    fn write_png(&self, device: &wgpu::Device, queue: &wgpu::Queue, path: &Path) -> Fallible<()> {
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.edge * 4),
                    rows_per_image: Some(self.edge),
                },
            },
            Size::new(self.edge, self.edge).extent(),
        );
        queue.submit([encoder.finish()]);
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely())?;

        let view = self.readback.slice(..).get_mapped_range();
        let mut png = png::Encoder::new(BufWriter::new(File::create(path)?), self.edge, self.edge);
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()?.write_image_data(&view)?;
        drop(view);
        self.readback.unmap();
        Ok(())
    }
}
