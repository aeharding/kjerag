use super::tests::{copy_texture, gpu, read_copy};
use super::*;

const Y_SIZE: [u32; 2] = [32, 16];
const UV_SIZE: [u32; 2] = [16, 8];

#[test]
fn scissored_full_size_output_matches_full_fusion_only_inside_two_regions() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let fixture = Fixture::new(&device, &queue);
    let fuse = GpuFuse::new(&device);
    let rectangles = [[0, 0, 8, 8], [20, 8, 12, 8]];
    let mut encoder = device.create_command_encoder(&Default::default());
    let full = fuse
        .encode(
            &device,
            &mut encoder,
            fixture.inputs(),
            &fixture.parameters,
            [0, 0, Y_SIZE[0], Y_SIZE[1]],
        )
        .unwrap();
    let scissored = fuse
        .encode_scissored(
            &device,
            &mut encoder,
            fixture.inputs(),
            &fixture.parameters,
            &rectangles,
        )
        .unwrap();
    let full_y = copy_texture(&device, &mut encoder, &full.y, Y_SIZE, 1);
    let full_uv = copy_texture(&device, &mut encoder, &full.uv, UV_SIZE, 2);
    let scissored_y = copy_texture(&device, &mut encoder, &scissored.y, Y_SIZE, 1);
    let scissored_uv = copy_texture(&device, &mut encoder, &scissored.uv, UV_SIZE, 2);
    queue.submit([encoder.finish()]);

    assert_scissored(
        &read_copy(&device, &full_y, Y_SIZE, 1),
        &read_copy(&device, &scissored_y, Y_SIZE, 1),
        Y_SIZE,
        1,
        &rectangles,
    );
    assert_scissored(
        &read_copy(&device, &full_uv, UV_SIZE, 2),
        &read_copy(&device, &scissored_uv, UV_SIZE, 2),
        UV_SIZE,
        2,
        &rectangles,
    );
}

#[test]
fn scissored_encode_refuses_invalid_regions_before_recording_draws() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let fixture = Fixture::new(&device, &queue);
    let fuse = GpuFuse::new(&device);
    let invalid = [
        vec![],
        vec![[0, 0, 2, 2], [2, 0, 2, 2], [4, 0, 2, 2]],
        vec![[1, 0, 2, 2]],
        vec![[0, 0, 0, 2]],
        vec![[30, 0, 4, 2]],
        vec![[0, 0, 8, 8], [6, 6, 8, 8]],
    ];
    for rectangles in invalid {
        let mut encoder = device.create_command_encoder(&Default::default());
        assert!(
            fuse.encode_scissored(
                &device,
                &mut encoder,
                fixture.inputs(),
                &fixture.parameters,
                &rectangles,
            )
            .is_err(),
            "accepted invalid rectangles {rectangles:?}",
        );
    }
}

fn assert_scissored(
    full: &[u8],
    scissored: &[u8],
    size: [u32; 2],
    bytes_per_pixel: usize,
    rectangles: &[[u32; 4]],
) {
    assert_eq!(full.len(), scissored.len());
    let divisor = bytes_per_pixel as u32;
    for y in 0..size[1] {
        for x in 0..size[0] {
            let inside = rectangles.iter().any(|rectangle| {
                let [rx, ry, width, height] = rectangle.map(|value| value / divisor);
                x >= rx && x < rx + width && y >= ry && y < ry + height
            });
            let at = (y as usize * size[0] as usize + x as usize) * bytes_per_pixel;
            for channel in 0..bytes_per_pixel {
                let expected = if inside { full[at + channel] } else { 0 };
                assert_eq!(
                    scissored[at + channel],
                    expected,
                    "pixel ({x},{y}) channel {channel}",
                );
            }
        }
    }
}

struct Fixture {
    y: wgpu::Texture,
    uv: wgpu::Texture,
    flow: wgpu::Texture,
    luma: wgpu::Texture,
    parameters: Parameters,
}

impl Fixture {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let y = array_texture(
            device,
            "scissored temporal Y",
            Y_SIZE,
            2,
            wgpu::TextureFormat::R8Unorm,
        );
        let uv = array_texture(
            device,
            "scissored temporal UV",
            UV_SIZE,
            2,
            wgpu::TextureFormat::Rg8Unorm,
        );
        let flow = array_texture(
            device,
            "scissored temporal flow",
            [2, 1],
            1,
            wgpu::TextureFormat::Rgba16Sint,
        );
        let luma = array_texture(
            device,
            "scissored temporal luma",
            [2, 1],
            1,
            wgpu::TextureFormat::R8Uint,
        );

        let current_y: Vec<u8> = (0..Y_SIZE[1])
            .flat_map(|row| (0..Y_SIZE[0]).map(move |column| (20 + 3 * row + column) as u8))
            .collect();
        let reference_y: Vec<u8> = (0..Y_SIZE[1])
            .flat_map(|row| (0..Y_SIZE[0]).map(move |column| (40 + 5 * column + 7 * row) as u8))
            .collect();
        let current_uv: Vec<u8> = (0..UV_SIZE[1])
            .flat_map(|row| {
                (0..UV_SIZE[0]).flat_map(move |column| {
                    [
                        (50 + 2 * column + row) as u8,
                        (180 - column - 2 * row) as u8,
                    ]
                })
            })
            .collect();
        let reference_uv: Vec<u8> = (0..UV_SIZE[1])
            .flat_map(|row| {
                (0..UV_SIZE[0]).flat_map(move |column| {
                    [(80 + column + 3 * row) as u8, (30 + 4 * column + row) as u8]
                })
            })
            .collect();
        write_layer(queue, &y, 0, Y_SIZE, 1, &current_y);
        write_layer(queue, &y, 1, Y_SIZE, 1, &reference_y);
        write_layer(queue, &uv, 0, UV_SIZE, 2, &current_uv);
        write_layer(queue, &uv, 1, UV_SIZE, 2, &reference_uv);
        let flow_bytes: Vec<u8> = (0..2)
            .flat_map(|_| [1_i16, 0, 255, 255].into_iter().flat_map(i16::to_le_bytes))
            .collect();
        write_layer(queue, &flow, 0, [2, 1], 8, &flow_bytes);
        write_layer(queue, &luma, 0, [2, 1], 1, &[11, 173]);

        Self {
            y,
            uv,
            flow,
            luma,
            parameters: Parameters {
                noise: 1.0,
                limit: 1.0,
                y_limits: [1.0; 256],
                uv_limits: [1.0; 256],
                current_layer: 0,
                reference_layers: vec![1],
            },
        }
    }

    fn inputs(&self) -> Inputs<'_> {
        Inputs {
            y: &self.y,
            uv: &self.uv,
            flow: &self.flow,
            luma: &self.luma,
        }
    }
}

fn array_texture(
    device: &wgpu::Device,
    label: &'static str,
    size: [u32; 2],
    layers: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn write_layer(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    layer: u32,
    size: [u32; 2],
    bytes_per_pixel: u32,
    bytes: &[u8],
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0] * bytes_per_pixel),
            rows_per_image: Some(size[1]),
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
}
