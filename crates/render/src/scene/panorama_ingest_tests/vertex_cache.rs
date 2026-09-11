//! Same-source compact panorama control for per-vertex work reuse.
//! Timing includes per-source cache allocation/preparation, not pipeline setup.

use super::*;

#[test]
#[ignore = "full X4 source vertex-cache comparison and wall-time diagnostic"]
fn x4_vertex_cache_matches_compact_source_and_reports_wall_time() {
    compare(
        "KJERAG_X4_TEST_MEDIA",
        Duration::from_micros(612_078_000),
        Camera {
            yaw: -80.71_f32.to_radians(),
            pitch: -48.46_f32.to_radians(),
            fov: 95.45_f32.to_radians(),
        },
    );
}

#[test]
#[ignore = "full ONE X2 source vertex-cache comparison and wall-time diagnostic"]
fn one_x2_vertex_cache_matches_compact_source_and_reports_wall_time() {
    compare(
        "KJERAG_ONE_X2_TEST_MEDIA",
        Duration::from_micros(212_512_000),
        Camera {
            yaw: 71.13_f32.to_radians(),
            pitch: -13.99_f32.to_radians(),
            fov: 57.95_f32.to_radians(),
        },
    );
}

fn compare(media_env: &str, time: Duration, camera: Camera) {
    let path = std::env::var_os(media_env).expect("vertex-cache comparison needs real footage");
    let ((device, queue), _) = super::super::tests::test_import_gpu_and_foreign().unwrap();
    let mut scene = Scene::open(Path::new(&path)).unwrap();
    scene.disable_temporal_for_review().unwrap();
    scene.set_muted(true);
    scene.pause(Instant::now());
    scene.set_horizon(Horizon::Locked);
    scene.seek(time, Accuracy::Exact);
    let frame = super::super::tests::wait_for_new_scene_frame(&scene, None);
    let mut selected = ScenePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    super::super::tests::prepare_and_draw_exact_resident_frame(
        &scene,
        &mut selected,
        &device,
        &queue,
        &frame,
    );
    scene.pump(Instant::now());
    let map = scene.diagnostic_one_xs_displayed_map().unwrap().unwrap();
    assert_eq!(map.frame(), &frame);
    let mut diagnostic = ScenePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let prepared = diagnostic
        .prepare_one_xs_picture(&scene.primitive(camera), 16.0 / 9.0)
        .unwrap();
    let source = prepared.reframe().frame_size();
    let size = Size::new(source[0] as u32 * 2, source[1] as u32);
    let matrix = MatrixCoefficients::from_source_rgb(prepared.reframe().source_color_matrix());
    eprintln!("vertex-cache source={frame:?} body={size:?} camera={camera:?}");
    let mut draw =
        DirectMapDraw::new_panorama(&device, &diagnostic.layout, map.fusion().is_some()).unwrap();
    draw.upload(&queue, &map);
    let plain = draw.prepare_compact_nv12(&device).unwrap();
    let cached = draw.prepare_vertex_cached_compact_nv12(&device).unwrap();
    queue.submit([]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();

    let mut encoder = device.create_command_encoder(&Default::default());
    let plain_image = plain
        .encode(
            &device,
            &mut encoder,
            &diagnostic.bind_group,
            &prepared,
            size,
        )
        .unwrap();
    let cached_image = cached
        .encode(
            &device,
            &mut encoder,
            &diagnostic.bind_group,
            &prepared,
            size,
        )
        .unwrap();
    assert_eq!(plain_image.frame(), &frame);
    assert_eq!(cached_image.frame(), &frame);
    assert_eq!(cached_image.size(), size);
    assert_eq!(cached_image.coefficients(), matrix);
    assert!(cached_image.belongs_to(&device));
    let plain_y = TextureReadback::encode(&device, &mut encoder, plain_image.packed_y(), 4);
    let plain_uv = TextureReadback::encode(&device, &mut encoder, plain_image.uv(), 2);
    let cached_y = TextureReadback::encode(&device, &mut encoder, cached_image.packed_y(), 4);
    let cached_uv = TextureReadback::encode(&device, &mut encoder, cached_image.uv(), 2);
    let submission = queue.submit([encoder.finish()]);
    let plain_y = plain_y.read(&device, submission.clone());
    let plain_uv = plain_uv.read(&device, submission.clone());
    let cached_y = cached_y.read(&device, submission.clone());
    let cached_uv = cached_uv.read(&device, submission);
    report_code_differences("vertex-cache packed Y", &plain_y, &cached_y);
    report_code_differences("vertex-cache UV", &plain_uv, &cached_uv);
    let differences = |a: &[u8], b: &[u8]| {
        assert_eq!(a.len(), b.len());
        a.iter().zip(b).filter(|(a, b)| a != b).count()
    };
    let y_differences = differences(&plain_y, &cached_y);
    let uv_differences = differences(&plain_uv, &cached_uv);

    if let Some(output) = std::env::var_os("KJERAG_VERTEX_CACHE_REVIEW_DIR") {
        let output = PathBuf::from(output);
        let unpack = PackedYUnpack::new(&device);
        let conversion = GpuColorConversion::new(&device);
        let projector = PanoramaProjector::new(&device);
        let view_size = Size::new(1280, 720);
        let mut views = Vec::new();
        for (name, picture) in [("uncached", &plain_image), ("cached", &cached_image)] {
            let mut encoder = device.create_command_encoder(&Default::default());
            let full_y = unpack.encode(&device, &mut encoder, picture.packed_y(), size);
            let rgb = conversion
                .encode_planes_to_rgb(&mut encoder, &full_y, picture.uv(), matrix)
                .unwrap();
            let view = projector
                .encode(&device, &mut encoder, &rgb, &prepared.reframe(), view_size)
                .unwrap();
            let readback = TextureReadback::encode(&device, &mut encoder, &view, 4);
            let submission = queue.submit([encoder.finish()]);
            let pixels = readback.read(&device, submission);
            let directory = output.join(name);
            std::fs::create_dir_all(&directory).unwrap();
            super::super::tests::write_review_ppm_sized(
                &directory,
                frame.index(),
                view_size.width,
                view_size.height,
                &pixels,
            );
            let digest = Sha256::digest(&pixels)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            eprintln!("vertex-cache {name} projected SHA256 {digest}");
            views.push(pixels);
        }
        report_code_differences("vertex-cache projected RGBA", &views[0], &views[1]);
    }

    eprintln!(
        "vertex-cache timings include cache allocation and GPU prepass, exclude pipeline setup, history unpack and readbacks; isolated wall time, not player FPS"
    );
    for pair in 0..8 {
        // Alternate the first arm as well as retaining several warm pairs.
        // Early pairs may include clock ramp-up and are not steady-state FPS.
        let order = if pair % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        };
        for use_cache in order {
            let start = Instant::now();
            let mut encoder = device.create_command_encoder(&Default::default());
            let output = if use_cache {
                cached.encode(
                    &device,
                    &mut encoder,
                    &diagnostic.bind_group,
                    &prepared,
                    size,
                )
            } else {
                plain.encode(
                    &device,
                    &mut encoder,
                    &diagnostic.bind_group,
                    &prepared,
                    size,
                )
            }
            .unwrap();
            let submission = queue.submit([encoder.finish()]);
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: None,
                })
                .unwrap();
            eprintln!(
                "vertex-cache pair={pair} cached={use_cache} elapsed_ms={:.3}",
                start.elapsed().as_secs_f64() * 1000.0
            );
            drop(output);
        }
    }
    assert_eq!(y_differences, 0, "vertex-cache changes packed Y bytes");
    assert_eq!(uv_differences, 0, "vertex-cache changes UV bytes");
}
