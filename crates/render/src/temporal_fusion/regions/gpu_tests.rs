use super::ViewRegions;
use crate::{
    Camera, Held, Reframe, Sampling, Size,
    direct_type2::panorama::PanoramaProjector,
    temporal_fusion::{
        GpuFuse, Inputs, Parameters,
        color::{GpuColorConversion, MatrixCoefficients},
        tests::{copy_texture, gpu, read_copy},
    },
};

const FULL: [u32; 2] = [1024, 512];
const FLOW: [u32; 2] = [64, 32];
const MATRIX: MatrixCoefficients = MatrixCoefficients {
    r_cr: 1.5748,
    g_cb: 0.1873,
    g_cr: 0.4681,
    b_cb: 1.8556,
};

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
            "region test Y",
            FULL,
            7,
            wgpu::TextureFormat::R8Unorm,
        );
        let uv = array_texture(
            device,
            "region test UV",
            [FULL[0] / 2, FULL[1] / 2],
            7,
            wgpu::TextureFormat::Rg8Unorm,
        );
        let flow = array_texture(
            device,
            "region test flow",
            FLOW,
            6,
            wgpu::TextureFormat::Rgba16Sint,
        );
        let luma = array_texture(
            device,
            "region test luma",
            FLOW,
            1,
            wgpu::TextureFormat::R8Uint,
        );

        for layer in 0..7u32 {
            let y_bytes: Vec<u8> = (0..FULL[1])
                .flat_map(|row| {
                    (0..FULL[0]).map(move |column| {
                        column
                            .wrapping_mul(17)
                            .wrapping_add(row.wrapping_mul(29))
                            .wrapping_add(layer.wrapping_mul(43))
                            .wrapping_add((column ^ row).wrapping_mul(3))
                            as u8
                    })
                })
                .collect();
            write_layer(queue, &y, layer, FULL, 1, &y_bytes);

            let uv_size = [FULL[0] / 2, FULL[1] / 2];
            let uv_bytes: Vec<u8> = (0..uv_size[1])
                .flat_map(|row| {
                    (0..uv_size[0]).flat_map(move |column| {
                        [
                            column
                                .wrapping_mul(11)
                                .wrapping_add(row.wrapping_mul(7))
                                .wrapping_add(layer.wrapping_mul(31))
                                as u8,
                            column
                                .wrapping_mul(5)
                                .wrapping_add(row.wrapping_mul(19))
                                .wrapping_add(layer.wrapping_mul(47))
                                as u8,
                        ]
                    })
                })
                .collect();
            write_layer(queue, &uv, layer, uv_size, 2, &uv_bytes);
        }

        for layer in 0..6u32 {
            let flow_bytes: Vec<u8> = (0..FLOW[1])
                .flat_map(|row| {
                    (0..FLOW[0]).flat_map(move |column| {
                        let dx = ((column + 2 * row + layer) % 7) as i16 - 3;
                        let dy = ((3 * column + row + layer) % 5) as i16 - 2;
                        let y_weight = (31 + (column * 13 + row * 17 + layer * 23) % 225) as i16;
                        let uv_weight = (19 + (column * 29 + row * 5 + layer * 37) % 237) as i16;
                        [dx, dy, y_weight, uv_weight]
                            .into_iter()
                            .flat_map(i16::to_le_bytes)
                    })
                })
                .collect();
            write_layer(queue, &flow, layer, FLOW, 8, &flow_bytes);
        }
        let luma_bytes: Vec<u8> = (0..FLOW[1])
            .flat_map(|row| {
                (0..FLOW[0]).map(move |column| {
                    column.wrapping_mul(53).wrapping_add(row.wrapping_mul(97)) as u8
                })
            })
            .collect();
        write_layer(queue, &luma, 0, FLOW, 1, &luma_bytes);

        Self {
            y,
            uv,
            flow,
            luma,
            parameters: Parameters {
                noise: 0.08,
                limit: 0.21,
                y_limits: std::array::from_fn(|index| 0.15 + (index % 23) as f32 / 31.0),
                uv_limits: std::array::from_fn(|index| 0.11 + (index % 29) as f32 / 37.0),
                current_layer: 3,
                reference_layers: vec![0, 1, 2, 4, 5, 6],
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
    assert_eq!(bytes.len(), (size[0] * size[1] * bytes_per_pixel) as usize);
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

fn reframe(camera: Camera, output: Size) -> Reframe {
    Reframe::new(
        &[],
        Size::new(FULL[0], FULL[1]),
        camera,
        Held::default(),
        output.width as f32 / output.height as f32,
        false,
        Sampling::default(),
    )
}

fn compare_view(camera: Camera, output_size: Size) {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let fixture = Fixture::new(&device, &queue);
    let fuse = GpuFuse::new(&device);
    let color = GpuColorConversion::new(&device);
    let projector = PanoramaProjector::new(&device);
    let reframe = reframe(camera, output_size);
    let regions = ViewRegions::for_view(FULL, &reframe).unwrap();
    assert!(!regions.rgb().is_empty());
    assert!(!regions.fusion().is_empty());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("full and scissored temporal view comparison"),
    });
    let full = fuse
        .encode(
            &device,
            &mut encoder,
            fixture.inputs(),
            &fixture.parameters,
            [0, 0, FULL[0], FULL[1]],
        )
        .unwrap();
    let full_rgb = color
        .encode_planes_to_rgb(&mut encoder, &full.y, &full.uv, MATRIX)
        .unwrap();
    let full_view = projector
        .encode(&device, &mut encoder, &full_rgb, &reframe, output_size)
        .unwrap();

    let scissored = fuse
        .encode_scissored(
            &device,
            &mut encoder,
            fixture.inputs(),
            &fixture.parameters,
            regions.fusion(),
        )
        .unwrap();
    let scissored_rgb = color
        .encode_planes_to_rgb_scissored(
            &mut encoder,
            &scissored.y,
            &scissored.uv,
            MATRIX,
            regions.rgb(),
        )
        .unwrap();
    let scissored_view = projector
        .encode(&device, &mut encoder, &scissored_rgb, &reframe, output_size)
        .unwrap();

    let full_copy = copy_texture(
        &device,
        &mut encoder,
        &full_view,
        [output_size.width, output_size.height],
        4,
    );
    let scissored_copy = copy_texture(
        &device,
        &mut encoder,
        &scissored_view,
        [output_size.width, output_size.height],
        4,
    );
    queue.submit([encoder.finish()]);
    let full_bytes = read_copy(
        &device,
        &full_copy,
        [output_size.width, output_size.height],
        4,
    );
    let scissored_bytes = read_copy(
        &device,
        &scissored_copy,
        [output_size.width, output_size.height],
        4,
    );
    assert_eq!(
        scissored_bytes, full_bytes,
        "scissored temporal projection must be byte-exact"
    );
}

#[test]
fn scissored_temporal_projection_is_exact_across_view_shapes_and_wraps() {
    let radians = |degrees: f32| degrees.to_radians();
    for (camera, output) in [
        (
            Camera {
                yaw: radians(179.8),
                pitch: radians(3.0),
                fov: radians(68.0),
            },
            Size::new(128, 72),
        ),
        (
            Camera {
                yaw: radians(-179.8),
                pitch: radians(-7.0),
                fov: radians(91.0),
            },
            Size::new(192, 108),
        ),
        (
            Camera {
                yaw: radians(42.0),
                pitch: radians(86.0),
                fov: radians(76.0),
            },
            Size::new(128, 72),
        ),
        (
            Camera {
                yaw: radians(-73.0),
                pitch: radians(-86.0),
                fov: radians(76.0),
            },
            Size::new(128, 72),
        ),
        (
            Camera {
                yaw: radians(31.0),
                pitch: radians(24.0),
                fov: radians(72.0),
            },
            Size::new(72, 128),
        ),
        (
            Camera {
                yaw: radians(15.0),
                pitch: radians(10.0),
                fov: radians(240.0),
            },
            Size::new(192, 108),
        ),
    ] {
        compare_view(camera, output);
    }
}

#[test]
fn omitting_the_centered_chroma_halo_changes_converted_boundary_pixels() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let fixture = Fixture::new(&device, &queue);
    let fuse = GpuFuse::new(&device);
    let color = GpuColorConversion::new(&device);
    let reframe = reframe(
        Camera {
            yaw: 0.4,
            pitch: -0.2,
            fov: 42_f32.to_radians(),
        },
        Size::new(128, 72),
    );
    let regions = ViewRegions::for_view(FULL, &reframe).unwrap();
    assert!(regions.rgb().iter().map(|r| r[2] * r[3]).sum::<u32>() < FULL[0] * FULL[1]);
    assert!(regions.rgb().iter().all(|&[x, y, width, height]| {
        x > 0 && y > 0 && x + width < FULL[0] && y + height < FULL[1]
    }));

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("missing centered-chroma halo control"),
    });
    let full = fuse
        .encode(
            &device,
            &mut encoder,
            fixture.inputs(),
            &fixture.parameters,
            [0, 0, FULL[0], FULL[1]],
        )
        .unwrap();
    let full_rgb = color
        .encode_planes_to_rgb(&mut encoder, &full.y, &full.uv, MATRIX)
        .unwrap();

    // Deliberately wrong: RGB coverage lacks the extra chroma support carried
    // by `fusion()`. Y is complete inside every RGB rectangle, so a boundary
    // difference here specifically observes missing centered-chroma taps.
    let missing_halo = fuse
        .encode_scissored(
            &device,
            &mut encoder,
            fixture.inputs(),
            &fixture.parameters,
            regions.rgb(),
        )
        .unwrap();
    let missing_halo_rgb = color
        .encode_planes_to_rgb_scissored(
            &mut encoder,
            &missing_halo.y,
            &missing_halo.uv,
            MATRIX,
            regions.rgb(),
        )
        .unwrap();
    let full_copy = copy_texture(&device, &mut encoder, &full_rgb, FULL, 4);
    let missing_copy = copy_texture(&device, &mut encoder, &missing_halo_rgb, FULL, 4);
    queue.submit([encoder.finish()]);
    let full_bytes = read_copy(&device, &full_copy, FULL, 4);
    let missing_bytes = read_copy(&device, &missing_copy, FULL, 4);

    let pixel_differs = |x: u32, y: u32| {
        let at = ((y * FULL[0] + x) * 4) as usize;
        full_bytes[at..at + 4] != missing_bytes[at..at + 4]
    };
    let mut observed = false;
    for &[x, y, width, height] in regions.rgb() {
        for column in x..x + width {
            observed |= pixel_differs(column, y);
            observed |= pixel_differs(column, y + height - 1);
        }
        for row in y..y + height {
            observed |= pixel_differs(x, row);
            observed |= pixel_differs(x + width - 1, row);
        }
    }
    assert!(
        observed,
        "the negative control must observe a missing centered-chroma boundary tap"
    );
}
