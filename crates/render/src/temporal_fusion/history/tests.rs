use std::time::Duration;

use super::*;
use crate::temporal_fusion::GpuFuse;
use crate::temporal_fusion::color::{GpuColorConversion, MatrixCoefficients};
use crate::temporal_fusion::pyramid::gpu::Builder as Pyramid;
use crate::temporal_fusion::tests::{Copy, copy_texture, gpu, gpu_pair, read_copy};

const Y_SIZE: [u32; 2] = [32, 16];
const UV_SIZE: [u32; 2] = [16, 8];
const MATRIX: MatrixCoefficients =
    MatrixCoefficients::from_source_rgb([1.5748, 0.1873, 0.4681, 1.8556]);
const REFERENCE_POSITIONS: [usize; 6] = [0, 1, 2, 4, 5, 6];

struct Copies {
    center: u64,
    ring_y: Copy,
    ring_uv: Copy,
    rebuilt_y: Copy,
    rebuilt_uv: Copy,
}

#[test]
fn direct_rgb_push_matches_allocated_conversion_and_copy_through_wrap() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let conversion = GpuColorConversion::new(&device);
    let pyramid = Pyramid::new(&device);
    let mut direct = History::new(&device, Y_SIZE).unwrap();
    let mut copied = History::new(&device, Y_SIZE).unwrap();
    let mut stamps = Vec::new();
    let mut pyramid_copies = Vec::new();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("direct resident history RGB regression"),
    });
    for ordinal in 0..14_u64 {
        let stamp = FrameStamp::for_test(
            700 + ordinal,
            Duration::from_millis(ordinal * 33),
            stamps.last(),
        );
        let rgb = patterned_rgb(&device, &queue, ordinal as u8);
        let nv12 = conversion
            .encode_rgb_to_nv12(&mut encoder, &rgb, MATRIX)
            .unwrap();
        copied
            .encode_push(&device, &mut encoder, &stamp, &nv12)
            .unwrap();
        let pushed = direct
            .encode_push_rgb(&device, &mut encoder, &conversion, &stamp, &rgb, MATRIX)
            .unwrap();
        assert_eq!(pushed.size(), Y_SIZE);
        assert_eq!(pushed.device(), &device);
        let direct_pyramid = pyramid
            .encode_history_luma(&device, &mut encoder, &pushed, 3)
            .unwrap();
        let copied_pyramid = pyramid
            .encode_luma(&device, &mut encoder, &nv12.y, 3)
            .unwrap();
        for (level, (direct_level, copied_level)) in direct_pyramid
            .levels
            .iter()
            .zip(&copied_pyramid.levels)
            .enumerate()
        {
            let size = [Y_SIZE[0] >> (level + 1), Y_SIZE[1] >> (level + 1)];
            pyramid_copies.push((
                size,
                copy_texture(&device, &mut encoder, direct_level, size, 1),
                copy_texture(&device, &mut encoder, copied_level, size, 1),
            ));
        }
        stamps.push(stamp);
    }

    let mut copies = Vec::new();
    for center in 0..7 {
        let direct_window = direct.window_at(center, 0).unwrap();
        let copied_window = copied.window_at(center, 0).unwrap();
        assert_eq!(direct_window.stamps().center, copied_window.stamps().center);
        let direct_output = direct_window
            .encode_copy_current(&device, &mut encoder)
            .unwrap();
        let copied_output = copied_window
            .encode_copy_current(&device, &mut encoder)
            .unwrap();
        copies.push((
            copy_texture(&device, &mut encoder, &direct_output.y, Y_SIZE, 1),
            copy_texture(&device, &mut encoder, &copied_output.y, Y_SIZE, 1),
            copy_texture(&device, &mut encoder, &direct_output.uv, UV_SIZE, 2),
            copy_texture(&device, &mut encoder, &copied_output.uv, UV_SIZE, 2),
        ));
    }
    queue.submit([encoder.finish()]);
    for (size, direct_level, copied_level) in pyramid_copies {
        assert_eq!(
            read_copy(&device, &direct_level, size, 1),
            read_copy(&device, &copied_level, size, 1)
        );
    }
    for (direct_y, copied_y, direct_uv, copied_uv) in copies {
        assert_eq!(
            read_copy(&device, &direct_y, Y_SIZE, 1),
            read_copy(&device, &copied_y, Y_SIZE, 1)
        );
        assert_eq!(
            read_copy(&device, &direct_uv, UV_SIZE, 2),
            read_copy(&device, &copied_uv, UV_SIZE, 2)
        );
    }
}

#[test]
fn startup_and_tail_intervals_clip_neighbors_without_padding() {
    let expected = [
        vec![1, 2, 3],
        vec![0, 2, 3, 4],
        vec![0, 1, 3, 4, 5],
        vec![0, 1, 2, 4, 5, 6],
        vec![1, 2, 3, 5, 6],
        vec![2, 3, 4, 6],
        vec![3, 4, 5],
    ];
    for (center, expected) in expected.into_iter().enumerate() {
        let actual: Vec<_> = reference_interval(center, 3)
            .unwrap()
            .filter(|&at| at != center)
            .collect();
        assert_eq!(actual, expected);
        for radius in 0..=3 {
            let actual: Vec<_> = reference_interval(center, radius)
                .unwrap()
                .filter(|&at| at != center)
                .collect();
            let expected: Vec<_> = (0usize..7)
                .filter(|&at| at != center && at.abs_diff(center) <= radius)
                .collect();
            assert_eq!(actual, expected);
        }
    }
    for (center, radius) in [(7, 3), (3, 4), (usize::MAX, 0), (0, usize::MAX)] {
        assert_eq!(
            reference_interval(center, radius),
            Err(Error::WindowPosition)
        );
    }
}

#[test]
fn clipped_windows_match_rebuilt_sources_before_and_after_ring_reuse() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let conversion = GpuColorConversion::new(&device);
    let fuse = GpuFuse::new(&device);
    let mut history = History::new(&device, Y_SIZE).unwrap();
    let mut stamps = Vec::new();
    let mut sources = Vec::new();
    let mut copies = Vec::new();
    for ordinal in 0..14_u64 {
        assert!(ordinal >= 7 || matches!(history.window_at(0, 3), Err(Error::IncompleteWindow)));
        let stamp = FrameStamp::for_test(
            500 + ordinal,
            Duration::from_millis(ordinal * 33),
            stamps.last(),
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let source = converted_source(&device, &queue, &conversion, &mut encoder, ordinal as u8);
        history
            .encode_push(&device, &mut encoder, &stamp, &source)
            .unwrap();
        stamps.push(stamp);
        sources.push(source);
        if ordinal == 6 || ordinal == 10 || ordinal == 13 {
            let start = ordinal as usize + 1 - 7;
            let rebuilt_y = array_texture(
                &device,
                "clipped reference Y",
                Y_SIZE,
                wgpu::TextureFormat::R8Unorm,
            );
            let rebuilt_uv = array_texture(
                &device,
                "clipped reference UV",
                UV_SIZE,
                wgpu::TextureFormat::Rg8Unorm,
            );
            for (layer, source) in sources[start..start + 7].iter().enumerate() {
                copy_layer(&mut encoder, &source.y, &rebuilt_y, layer as u32);
                copy_layer(&mut encoder, &source.uv, &rebuilt_uv, layer as u32);
            }
            assert!(matches!(
                history.window_at(7, 3),
                Err(Error::WindowPosition)
            ));
            assert!(matches!(
                history.window_at(3, 4),
                Err(Error::WindowPosition)
            ));
            for center in 0usize..7 {
                for radius in 0..=3 {
                    let window = history.window_at(center, radius).unwrap();
                    let expected_positions: Vec<_> = (0usize..7)
                        .filter(|&at| at != center && at.abs_diff(center) <= radius)
                        .collect();
                    let actual_stamps = window.stamps();
                    assert_eq!(actual_stamps.center, &stamps[start + center]);
                    let expected_stamps: Vec<_> = expected_positions
                        .iter()
                        .map(|&at| &stamps[start + at])
                        .collect();
                    assert_eq!(actual_stamps.references, expected_stamps);
                    let parameters = window.parameters(1.0, 1.0, [1.0; 256], [1.0; 256]);
                    assert_eq!(parameters.current_layer, ((start + center) % 7) as u32);
                    assert_eq!(
                        parameters.reference_layers,
                        expected_positions
                            .iter()
                            .map(|&at| ((start + at) % 7) as u32)
                            .collect::<Vec<_>>()
                    );
                    if radius == 0 {
                        assert!(parameters.reference_layers.is_empty());
                        continue;
                    }
                    let (flow, luma) =
                        motion_inputs_count(&device, &queue, expected_positions.len());
                    let ring = fuse
                        .encode(
                            &device,
                            &mut encoder,
                            window.inputs(&flow, &luma),
                            &parameters,
                            [0, 0, Y_SIZE[0], Y_SIZE[1]],
                        )
                        .unwrap();
                    let rebuilt = fuse
                        .encode(
                            &device,
                            &mut encoder,
                            Inputs {
                                y: &rebuilt_y,
                                uv: &rebuilt_uv,
                                flow: &flow,
                                luma: &luma,
                            },
                            &Parameters {
                                current_layer: center as u32,
                                reference_layers: expected_positions
                                    .iter()
                                    .map(|&at| at as u32)
                                    .collect(),
                                ..parameters
                            },
                            [0, 0, Y_SIZE[0], Y_SIZE[1]],
                        )
                        .unwrap();
                    copies.push(Copies {
                        center: stamps[start + center].index(),
                        ring_y: copy_texture(&device, &mut encoder, &ring.y, Y_SIZE, 1),
                        ring_uv: copy_texture(&device, &mut encoder, &ring.uv, UV_SIZE, 2),
                        rebuilt_y: copy_texture(&device, &mut encoder, &rebuilt.y, Y_SIZE, 1),
                        rebuilt_uv: copy_texture(&device, &mut encoder, &rebuilt.uv, UV_SIZE, 2),
                    });
                }
            }
        }
        queue.submit([encoder.finish()]);
    }
    assert_eq!(copies.len(), 63);
    for copy in copies {
        assert_eq!(
            read_copy(&device, &copy.ring_y, Y_SIZE, 1),
            read_copy(&device, &copy.rebuilt_y, Y_SIZE, 1),
            "clipped Y center {}",
            copy.center
        );
        assert_eq!(
            read_copy(&device, &copy.ring_uv, UV_SIZE, 2),
            read_copy(&device, &copy.rebuilt_uv, UV_SIZE, 2),
            "clipped UV center {}",
            copy.center
        );
    }
}

#[test]
fn resident_ring_matches_rebuilt_arrays_through_three_wraps_without_waits() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let conversion = GpuColorConversion::new(&device);
    let fuse = GpuFuse::new(&device);
    let (flow, luma) = motion_inputs(&device, &queue);
    let mut history = History::new(&device, Y_SIZE).unwrap();
    let mut stamps = Vec::with_capacity(28);
    let mut sources = Vec::with_capacity(28);
    let mut copies = Vec::new();
    let mut sensitivity = None;

    for ordinal in 0..28_u64 {
        let stamp = FrameStamp::for_test(
            100 + ordinal,
            Duration::from_millis(ordinal * 33),
            stamps.last(),
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("resident history wrap regression"),
        });
        let source = converted_source(&device, &queue, &conversion, &mut encoder, ordinal as u8);
        history
            .encode_push(&device, &mut encoder, &stamp, &source)
            .unwrap();
        stamps.push(stamp);
        sources.push(source);

        if let Some(window) = history.window() {
            let start = ordinal as usize + 1 - 7;
            assert_window_stamps(&window, &stamps[start..start + 7]);
            let parameters = window.parameters(1.0, 1.0, [1.0; 256], [1.0; 256]);
            assert_layer_mapping(&parameters);
            let ring = fuse
                .encode(
                    &device,
                    &mut encoder,
                    window.inputs(&flow, &luma),
                    &parameters,
                    [0, 0, Y_SIZE[0], Y_SIZE[1]],
                )
                .unwrap();

            let (rebuilt_y, rebuilt_uv) =
                rebuilt_window(&device, &mut encoder, &sources[start..start + 7], None);
            let rebuilt_parameters = logical_parameters();
            let rebuilt = fuse
                .encode(
                    &device,
                    &mut encoder,
                    Inputs {
                        y: &rebuilt_y,
                        uv: &rebuilt_uv,
                        flow: &flow,
                        luma: &luma,
                    },
                    &rebuilt_parameters,
                    [0, 0, Y_SIZE[0], Y_SIZE[1]],
                )
                .unwrap();
            copies.push(Copies {
                center: stamps[start + 3].index(),
                ring_y: copy_texture(&device, &mut encoder, &ring.y, Y_SIZE, 1),
                ring_uv: copy_texture(&device, &mut encoder, &ring.uv, UV_SIZE, 2),
                rebuilt_y: copy_texture(&device, &mut encoder, &rebuilt.y, Y_SIZE, 1),
                rebuilt_uv: copy_texture(&device, &mut encoder, &rebuilt.uv, UV_SIZE, 2),
            });

            if ordinal == 13 {
                let mut reordered = logical_parameters();
                reordered.reference_layers.swap(0, 5);
                let wrong_order = fuse
                    .encode(
                        &device,
                        &mut encoder,
                        Inputs {
                            y: &rebuilt_y,
                            uv: &rebuilt_uv,
                            flow: &flow,
                            luma: &luma,
                        },
                        &reordered,
                        [0, 0, Y_SIZE[0], Y_SIZE[1]],
                    )
                    .unwrap();
                let (stale_y, stale_uv) = rebuilt_window(
                    &device,
                    &mut encoder,
                    &sources[start..start + 7],
                    Some((1, 3)),
                );
                let stale = fuse
                    .encode(
                        &device,
                        &mut encoder,
                        Inputs {
                            y: &stale_y,
                            uv: &stale_uv,
                            flow: &flow,
                            luma: &luma,
                        },
                        &rebuilt_parameters,
                        [0, 0, Y_SIZE[0], Y_SIZE[1]],
                    )
                    .unwrap();
                sensitivity = Some((
                    copy_texture(&device, &mut encoder, &rebuilt.y, Y_SIZE, 1),
                    copy_texture(&device, &mut encoder, &wrong_order.y, Y_SIZE, 1),
                    copy_texture(&device, &mut encoder, &stale.y, Y_SIZE, 1),
                ));
            }
        }
        // Queue order protects an earlier consumer before a later submission
        // reuses its physical layer. No submission is waited here.
        queue.submit([encoder.finish()]);
    }

    assert_eq!(copies.len(), 22);
    for copy in copies {
        let ring_y = read_copy(&device, &copy.ring_y, Y_SIZE, 1);
        let ring_uv = read_copy(&device, &copy.ring_uv, UV_SIZE, 2);
        let rebuilt_y = read_copy(&device, &copy.rebuilt_y, Y_SIZE, 1);
        let rebuilt_uv = read_copy(&device, &copy.rebuilt_uv, UV_SIZE, 2);
        assert_eq!(ring_y, rebuilt_y, "ring Y at center {}", copy.center);
        assert_eq!(ring_uv, rebuilt_uv, "ring UV at center {}", copy.center);
    }
    let (good, reordered, stale) = sensitivity.unwrap();
    let good = read_copy(&device, &good, Y_SIZE, 1);
    assert_ne!(good, read_copy(&device, &reordered, Y_SIZE, 1));
    assert_ne!(good, read_copy(&device, &stale, Y_SIZE, 1));
}

#[test]
fn rejected_sources_record_no_copy_and_leave_the_window_unchanged() {
    let Some([(device, queue), (foreign_device, foreign_queue)]) = gpu_pair() else {
        return;
    };
    // Pinned wgpu Device equality diagnoses distinct devices within one
    // Instance. Its native IDs are not globally unique across Instances.
    assert_ne!(device, foreign_device);
    assert!(matches!(
        History::new(&device, [0, 16]),
        Err(Error::InvalidSize)
    ));
    assert!(matches!(
        History::new(&device, [31, 16]),
        Err(Error::InvalidSize)
    ));
    assert!(matches!(
        History::new(&device, [u32::MAX - 1, 16]),
        Err(Error::UnsupportedSize)
    ));
    let conversion = GpuColorConversion::new(&device);
    let fuse = GpuFuse::new(&device);
    let (flow, luma) = motion_inputs(&device, &queue);
    let mut history = History::new(&device, Y_SIZE).unwrap();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("resident history rejected-source regression"),
    });
    let mut stamps = Vec::new();
    let mut sources = Vec::new();
    for ordinal in 0..7_u64 {
        let stamp = FrameStamp::for_test(
            200 + ordinal,
            Duration::from_millis(ordinal * 33),
            stamps.last(),
        );
        let source = converted_source(&device, &queue, &conversion, &mut encoder, ordinal as u8);
        history
            .encode_push(&device, &mut encoder, &stamp, &source)
            .unwrap();
        stamps.push(stamp);
        sources.push(source);
    }
    let sentinel = converted_source(&device, &queue, &conversion, &mut encoder, 251);
    let gap = FrameStamp::for_test(208, Duration::from_millis(8 * 33), stamps.last());
    let foreign = FrameStamp::for_test(207, Duration::from_millis(7 * 33), None);
    assert_eq!(
        history.encode_push(&device, &mut encoder, &stamps[6], &sentinel),
        Err(Error::NonContiguous)
    );
    assert_eq!(
        history.encode_push(&device, &mut encoder, &gap, &sentinel),
        Err(Error::NonContiguous)
    );
    assert_eq!(
        history.encode_push(&device, &mut encoder, &foreign, &sentinel),
        Err(Error::DecodeEpoch)
    );

    let valid_next = FrameStamp::for_test(207, Duration::from_millis(7 * 33), stamps.last());
    let mut invalid_storage = converted_source(&device, &queue, &conversion, &mut encoder, 249);
    // A matching-format array is not a legal single-layer arriving source.
    invalid_storage.uv = array_texture(
        &device,
        "invalid arriving UV array",
        UV_SIZE,
        wgpu::TextureFormat::Rg8Unorm,
    );
    assert_eq!(
        history.encode_push(&device, &mut encoder, &valid_next, &invalid_storage),
        Err(Error::SourceTextures)
    );

    let rgb = patterned_rgb(&device, &queue, 248);
    assert!(matches!(
        history.encode_push_rgb(&device, &mut encoder, &conversion, &gap, &rgb, MATRIX,),
        Err(Error::NonContiguous)
    ));
    assert!(matches!(
        history.encode_push_rgb(
            &device,
            &mut encoder,
            &conversion,
            &valid_next,
            &rgb,
            MatrixCoefficients::from_source_rgb([f32::NAN, 0.0, 0.0, 1.0]),
        ),
        Err(Error::Color(
            crate::temporal_fusion::color::Error::Coefficients
        ))
    ));
    let wrong_rgb = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("invalid resident history RGB dimensions"),
        size: wgpu::Extent3d {
            width: Y_SIZE[0] / 2,
            height: Y_SIZE[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    assert!(matches!(
        history.encode_push_rgb(
            &device,
            &mut encoder,
            &conversion,
            &valid_next,
            &wrong_rgb,
            MATRIX,
        ),
        Err(Error::SourceRgb)
    ));

    let foreign_conversion = GpuColorConversion::new(&foreign_device);
    let mut foreign_encoder = foreign_device.create_command_encoder(&Default::default());
    let foreign_nv12 = converted_source(
        &foreign_device,
        &foreign_queue,
        &foreign_conversion,
        &mut foreign_encoder,
        247,
    );
    assert_eq!(
        history.encode_push(&device, &mut encoder, &valid_next, &foreign_nv12),
        Err(Error::ForeignDevice)
    );
    assert_eq!(
        history.encode_push(&foreign_device, &mut encoder, &valid_next, &sentinel),
        Err(Error::ForeignDevice)
    );
    assert!(matches!(
        history.encode_push_rgb(
            &device,
            &mut encoder,
            &foreign_conversion,
            &valid_next,
            &rgb,
            MATRIX,
        ),
        Err(Error::ForeignDevice)
    ));

    let window = history.window().unwrap();
    assert_window_stamps(&window, &stamps);
    let parameters = window.parameters(1.0, 1.0, [1.0; 256], [1.0; 256]);
    assert_layer_mapping(&parameters);
    let actual = fuse
        .encode(
            &device,
            &mut encoder,
            window.inputs(&flow, &luma),
            &parameters,
            [0, 0, Y_SIZE[0], Y_SIZE[1]],
        )
        .unwrap();
    let (rebuilt_y, rebuilt_uv) = rebuilt_window(&device, &mut encoder, &sources, None);
    let expected = fuse
        .encode(
            &device,
            &mut encoder,
            Inputs {
                y: &rebuilt_y,
                uv: &rebuilt_uv,
                flow: &flow,
                luma: &luma,
            },
            &logical_parameters(),
            [0, 0, Y_SIZE[0], Y_SIZE[1]],
        )
        .unwrap();
    let actual_y = copy_texture(&device, &mut encoder, &actual.y, Y_SIZE, 1);
    let expected_y = copy_texture(&device, &mut encoder, &expected.y, Y_SIZE, 1);
    let actual_uv = copy_texture(&device, &mut encoder, &actual.uv, UV_SIZE, 2);
    let expected_uv = copy_texture(&device, &mut encoder, &expected.uv, UV_SIZE, 2);
    queue.submit([encoder.finish()]);
    assert_eq!(
        read_copy(&device, &actual_y, Y_SIZE, 1),
        read_copy(&device, &expected_y, Y_SIZE, 1)
    );
    assert_eq!(
        read_copy(&device, &actual_uv, UV_SIZE, 2),
        read_copy(&device, &expected_uv, UV_SIZE, 2)
    );
}

fn assert_window_stamps(window: &Window<'_>, stamps: &[FrameStamp]) {
    assert_eq!(stamps.len(), 7);
    let actual = window.stamps();
    assert_eq!(actual.center, &stamps[3]);
    for (actual, expected) in actual.references.into_iter().zip(REFERENCE_POSITIONS) {
        assert_eq!(actual, &stamps[expected]);
    }
}

fn assert_layer_mapping(parameters: &Parameters) {
    assert_eq!(parameters.reference_layers.len(), 6);
    assert!(
        !parameters
            .reference_layers
            .contains(&parameters.current_layer)
    );
    for (index, layer) in parameters.reference_layers.iter().enumerate() {
        assert!(!parameters.reference_layers[..index].contains(layer));
    }
}

fn logical_parameters() -> Parameters {
    Parameters {
        noise: 1.0,
        limit: 1.0,
        y_limits: [1.0; 256],
        uv_limits: [1.0; 256],
        current_layer: 0,
        reference_layers: (1..=6).collect(),
    }
}

fn converted_source(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    conversion: &GpuColorConversion,
    encoder: &mut wgpu::CommandEncoder,
    source: u8,
) -> Nv12 {
    let rgb = patterned_rgb(device, queue, source);
    conversion
        .encode_rgb_to_nv12(encoder, &rgb, MATRIX)
        .unwrap()
}

fn patterned_rgb(device: &wgpu::Device, queue: &wgpu::Queue, source: u8) -> wgpu::Texture {
    let rgb = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("resident history patterned RGB source"),
        size: wgpu::Extent3d {
            width: Y_SIZE[0],
            height: Y_SIZE[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let pixels: Vec<u8> = (0..Y_SIZE[1])
        .flat_map(|y| {
            (0..Y_SIZE[0]).flat_map(move |x| {
                [
                    source.wrapping_mul(37).wrapping_add((x * 13 + y * 3) as u8),
                    source.wrapping_mul(19).wrapping_add((x * 5 + y * 17) as u8),
                    source.wrapping_mul(53).wrapping_add((x * 7 + y * 11) as u8),
                    255,
                ]
            })
        })
        .collect();
    queue.write_texture(
        rgb.as_image_copy(),
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(Y_SIZE[0] * 4),
            rows_per_image: Some(Y_SIZE[1]),
        },
        rgb.size(),
    );
    rgb
}

fn rebuilt_window(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    sources: &[Nv12],
    replacement: Option<(usize, usize)>,
) -> (wgpu::Texture, wgpu::Texture) {
    assert_eq!(sources.len(), 7);
    let y = array_texture(
        device,
        "rebuilt logical Y history",
        Y_SIZE,
        wgpu::TextureFormat::R8Unorm,
    );
    let uv = array_texture(
        device,
        "rebuilt logical UV history",
        UV_SIZE,
        wgpu::TextureFormat::Rg8Unorm,
    );
    let logical = [3, 0, 1, 2, 4, 5, 6];
    for (layer, source_at) in logical.into_iter().enumerate() {
        let source_at = replacement
            .filter(|(replace_layer, _)| *replace_layer == layer)
            .map_or(source_at, |(_, replacement)| replacement);
        copy_layer(encoder, &sources[source_at].y, &y, layer as u32);
        copy_layer(encoder, &sources[source_at].uv, &uv, layer as u32);
    }
    (y, uv)
}

fn motion_inputs(device: &wgpu::Device, queue: &wgpu::Queue) -> (wgpu::Texture, wgpu::Texture) {
    motion_inputs_count(device, queue, 6)
}

fn motion_inputs_count(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    count: usize,
) -> (wgpu::Texture, wgpu::Texture) {
    assert!((1..=6).contains(&count));
    let flow = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("resident history ordered flow"),
        size: wgpu::Extent3d {
            width: 2,
            height: 1,
            depth_or_array_layers: count as u32,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Sint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let luma = array_texture(
        device,
        "resident history luma",
        [2, 1],
        wgpu::TextureFormat::R8Uint,
    );
    let confidence = [255_i16, 211, 167, 113, 71, 29];
    for (layer, value) in confidence.into_iter().take(count).enumerate() {
        let texel: Vec<u8> = [layer as i16 - 2, 0, value, 255 - value]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect();
        let bytes: Vec<u8> = texel.iter().copied().chain(texel.iter().copied()).collect();
        write_layer(queue, &flow, layer as u32, [2, 1], 8, &bytes);
    }
    write_layer(queue, &luma, 0, [2, 1], 1, &[0, 1]);
    (flow, luma)
}

fn array_texture(
    device: &wgpu::Device,
    label: &'static str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: if format == wgpu::TextureFormat::R8Uint {
                1
            } else if format == wgpu::TextureFormat::Rgba16Sint {
                6
            } else {
                7
            },
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn copy_layer(
    encoder: &mut wgpu::CommandEncoder,
    source: &wgpu::Texture,
    destination: &wgpu::Texture,
    layer: u32,
) {
    encoder.copy_texture_to_texture(
        source.as_image_copy(),
        wgpu::TexelCopyTextureInfo {
            texture: destination,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        source.size(),
    );
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
