//! Rendering the app's pass with no compositor: a device, a colour target,
//! and the readback that turns it into pixels.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use kjerag_media::Fallible;
use kjerag_render::{Extent, ScenePipeline, Size, dmabuf};

/// A `copy_texture_to_buffer` row is padded to this, whatever the picture is
/// wide. The 1024 px square `reframe` writes needs no padding and a 960 px
/// wide one does, so nothing may assume the tight stride.
const ROW_ALIGNMENT: u32 = 256;

/// A Vulkan device that can import the decoder's dmabufs, which is the same
/// device `dmabuf::open_device` builds for the app.
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub name: String,
}

impl Gpu {
    pub fn open() -> Fallible<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))?;
        let (device, queue) = dmabuf::open_device(&adapter)?;
        Ok(Self {
            device,
            queue,
            name: adapter.get_info().name,
        })
    }
}

/// One colour target and the buffer its pixels are read back into.
pub struct Offscreen {
    texture: wgpu::Texture,
    readback: wgpu::Buffer,
    size: Size,
    stride: u32,
}

impl Offscreen {
    pub fn new(device: &wgpu::Device, size: Size, format: wgpu::TextureFormat) -> Self {
        let stride = (size.width * 4).next_multiple_of(ROW_ALIGNMENT);
        Self {
            texture: device.create_texture(&wgpu::TextureDescriptor {
                label: Some("offscreen"),
                size: size.extent(),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: u64::from(stride) * u64::from(size.height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            size,
            stride,
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// The pass, drawn and waited for.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &ScenePipeline,
    ) -> Fallible<()> {
        let view = self.texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("offscreen"),
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

    /// The picture as tightly packed RGBA rows, with the copy's row padding
    /// taken back out.
    pub fn read(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Fallible<Vec<u8>> {
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.stride),
                    rows_per_image: Some(self.size.height),
                },
            },
            self.size.extent(),
        );
        queue.submit([encoder.finish()]);
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely())?;

        let padded = self.readback.slice(..).get_mapped_range();
        let width = self.size.width as usize * 4;
        let pixels = padded
            .chunks_exact(self.stride as usize)
            .flat_map(|row| &row[..width])
            .copied()
            .collect();
        drop(padded);
        self.readback.unmap();
        Ok(pixels)
    }

    pub fn write_png(&self, pixels: &[u8], path: &Path) -> Fallible<()> {
        let mut png = png::Encoder::new(
            BufWriter::new(File::create(path)?),
            self.size.width,
            self.size.height,
        );
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()?.write_image_data(pixels)?;
        Ok(())
    }
}
