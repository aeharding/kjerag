use std::time::Duration;

use kjerag_meta::DenoiseIsoObservation;

use super::*;
use crate::temporal_fusion::tests::{copy_texture, gpu, read_copy};

const FULL: [u32; 2] = [4_096, 2_048];
const BASE: [u32; 2] = [2_048, 1_024];
const MATRIX: MatrixCoefficients =
    MatrixCoefficients::from_source_rgb([1.5748, 0.1873, 0.4681, 1.8556]);
const ISO: [u32; 11] = [400, 400, 400, 401, 2_200, 300, 300, 300, 300, 300, 300];

#[test]
fn lazy_radius_transition_matches_eager_inputs_before_and_after_ring_reuse() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let observations: Vec<_> = ISO
        .iter()
        .enumerate()
        .map(|(at, &iso)| DenoiseIsoObservation {
            offset_ms: (at as i64) * 100,
            iso,
        })
        .collect();
    let mut lazy = Stream::new(
        &device,
        &queue,
        FULL,
        Provider::for_test_common(&observations).unwrap(),
    )
    .unwrap();
    let mut eager = Stream::new(
        &device,
        &queue,
        FULL,
        Provider::for_test_common(&observations).unwrap(),
    )
    .unwrap();
    let conversion = GpuColorConversion::new(&device);

    for source in 0..SOURCES {
        seed_pair(&mut lazy, &mut eager, &conversion, source as u64);
    }
    for (at, retained) in lazy.window.iter().enumerate() {
        assert_eq!(retained.effective.radius == 0, retained.motion.is_none());
        if let Some(motion) = retained.motion.as_ref() {
            assert_eq!(motion.finest().logical_size(), BASE);
            assert_eq!(
                [motion.luma().width(), motion.luma().height()],
                [BASE[0] / 8, BASE[1] / 8]
            );
        }
        assert_eq!(retained.effective.iso, ISO[at] as i32);
    }

    for center in 0..=CENTER {
        let expected = lazy.window[center].stamp.clone();
        assert_output_pair(&mut lazy, &mut eager, center, &expected);
        if lazy.window[center].effective.radius == 0 {
            assert!(
                lazy.window[center].motion.is_none(),
                "a radius-zero center built resident motion for its own output"
            );
        }
        lazy.next_center += 1;
        eager.next_center += 1;
    }

    // Center 3 has radius one. Its radius-zero source 2 is reconstructed,
    // while unrelated radius-zero sources remain untouched.
    assert!(lazy.window[2].motion.is_some());
    for at in [0, 1, 5, 6] {
        assert!(
            lazy.window[at].motion.is_none(),
            "unexpected resident motion at {at}"
        );
    }
    let retained_handle = lazy.window[2]
        .motion
        .as_ref()
        .unwrap()
        .finest()
        .texture()
        .clone();
    lazy.prepare_references(3).unwrap();
    assert_eq!(
        lazy.window[2].motion.as_ref().unwrap().finest().texture(),
        &retained_handle,
        "a repeated preparation replaced an immutable packed source",
    );

    // Keep source zero's eager packed bytes across physical history-layer
    // reuse. Source seven has a deliberately different image and must be the
    // value reconstructed from the newly rotated history slot.
    let stale_zero = eager.window[0]
        .motion
        .as_ref()
        .unwrap()
        .finest()
        .texture()
        .clone();
    for source in SOURCES as u64..ISO.len() as u64 {
        lazy.window.pop_front();
        eager.window.pop_front();
        lazy.next_center -= 1;
        eager.next_center -= 1;
        seed_pair(&mut lazy, &mut eager, &conversion, source);

        let center = lazy.next_center;
        let expected = lazy.window[center].stamp.clone();
        assert_output_pair(&mut lazy, &mut eager, center, &expected);
        lazy.next_center += 1;
        eager.next_center += 1;

        if source == SOURCES as u64 {
            let fresh = packed_bytes(&device, &queue, &lazy.window[6]);
            assert_eq!(fresh, packed_bytes(&device, &queue, &eager.window[6]));
            assert_ne!(fresh, packed_texture_bytes(&device, &queue, &stale_zero));
        }
    }

    // The final radius-zero centers were never motion references and remain
    // genuinely lazy after their own output was copied.
    for retained in lazy.window.iter().rev().take(3) {
        assert_eq!(retained.effective.radius, 0);
        assert!(retained.motion.is_none());
    }
}

fn seed_pair(lazy: &mut Stream, eager: &mut Stream, conversion: &GpuColorConversion, source: u64) {
    assert_eq!(lazy.window.len(), eager.window.len());
    let previous = lazy.window.back().map(|retained| &retained.stamp);
    let stamp = FrameStamp::for_test(
        10_000 + source,
        Duration::from_millis(source * 100),
        previous,
    );
    let device = lazy.device.clone();
    let queue = lazy.queue.clone();
    let rgb = synthetic_rgb(&device, &queue, source as u8);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("stream radius transition source"),
    });
    let nv12 = conversion
        .encode_rgb_to_nv12(&mut encoder, &rgb, MATRIX)
        .unwrap();
    lazy.history
        .encode_push(&device, &mut encoder, &stamp, &nv12)
        .unwrap();
    eager
        .history
        .encode_push(&device, &mut encoder, &stamp, &nv12)
        .unwrap();
    let lazy_effective = lazy
        .provider
        .parameters_at(stamp.timestamp().as_secs_f64() * 1_000.0)
        .unwrap();
    let eager_effective = eager
        .provider
        .parameters_at(stamp.timestamp().as_secs_f64() * 1_000.0)
        .unwrap();
    assert_eq!(lazy_effective, eager_effective);

    let lazy_motion =
        (lazy_effective.radius > 0).then(|| lazy.prepare_pyramid(&mut encoder, &nv12.y).unwrap());
    let eager_motion = Some(eager.prepare_pyramid(&mut encoder, &nv12.y).unwrap());
    queue.submit([encoder.finish()]);
    lazy.window.push_back(Retained {
        stamp: stamp.clone(),
        effective: lazy_effective,
        motion: lazy_motion,
    });
    eager.window.push_back(Retained {
        stamp,
        effective: eager_effective,
        motion: eager_motion,
    });
    lazy.matrix = Some(MATRIX);
    eager.matrix = Some(MATRIX);
}

fn assert_output_pair(lazy: &mut Stream, eager: &mut Stream, center: usize, expected: &FrameStamp) {
    let lazy_output = lazy.process(center).unwrap();
    let eager_output = eager.process(center).unwrap();
    assert_eq!(lazy_output.frame(), expected);
    assert_eq!(eager_output.frame(), expected);
    let device = lazy.device.clone();
    let queue = lazy.queue.clone();
    let mut encoder = device.create_command_encoder(&Default::default());
    let lazy_copy = copy_texture(&device, &mut encoder, lazy_output.texture(), FULL, 4);
    let eager_copy = copy_texture(&device, &mut encoder, eager_output.texture(), FULL, 4);
    queue.submit([encoder.finish()]);
    assert_eq!(
        read_copy(&device, &lazy_copy, FULL, 4),
        read_copy(&device, &eager_copy, FULL, 4),
        "lazy output differs at source {}",
        expected.index(),
    );
}

fn packed_bytes(device: &wgpu::Device, queue: &wgpu::Queue, retained: &Retained) -> Vec<u8> {
    packed_texture_bytes(
        device,
        queue,
        retained.motion.as_ref().unwrap().finest().texture(),
    )
}

fn packed_texture_bytes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Vec<u8> {
    let size = [BASE[0].div_ceil(4), BASE[1]];
    let mut encoder = device.create_command_encoder(&Default::default());
    let copy = copy_texture(device, &mut encoder, texture, size, 4);
    queue.submit([encoder.finish()]);
    read_copy(device, &copy, size, 4)
}

fn synthetic_rgb(device: &wgpu::Device, queue: &wgpu::Queue, source: u8) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("stream radius transition synthetic RGB"),
        size: wgpu::Extent3d {
            width: FULL[0],
            height: FULL[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let pixels: Vec<u8> = (0..FULL[1])
        .flat_map(|y| {
            (0..FULL[0]).flat_map(move |x| {
                [
                    source.wrapping_mul(31).wrapping_add((x * 7 + y * 3) as u8),
                    source.wrapping_mul(47).wrapping_add((x * 5 + y * 11) as u8),
                    source.wrapping_mul(59).wrapping_add((x * 13 + y) as u8),
                    255,
                ]
            })
        })
        .collect();
    queue.write_texture(
        texture.as_image_copy(),
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(FULL[0] * 4),
            rows_per_image: Some(FULL[1]),
        },
        texture.size(),
    );
    texture
}
