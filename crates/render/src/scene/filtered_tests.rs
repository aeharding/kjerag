//! Real-source coverage for the automatic filtered Scene route.
//!
//! These tests require automatic selection before the first delivery. They
//! deliberately drive the real decoder,
//! resident panorama worker, temporal stream, Scene acknowledgement gate and
//! final draw rather than constructing a `Stream` in isolation.

use super::*;
use std::io::Write;

const DEADLINE: Duration = Duration::from_secs(60);

/// Capture the selected player's actual current/filtered fields, without
/// rebuilding either term through an offline approximation. A reported line
/// can then be compared with the panorama boundary and the real shown view.
#[test]
fn reported_filtered_correction_fields() {
    let Some(output) = std::env::var_os("KJERAG_CORRECTION_FIELDS_DIR").map(PathBuf::from) else {
        return;
    };
    let line = std::env::var("KJERAG_REPORTED_SEAM_VIEW").expect("review needs a full view line");
    let (path, view) = crate::Framing::read_line(&line).expect("invalid review view line");
    assert_eq!(view.horizon, Horizon::Locked);
    std::fs::create_dir(&output).expect("review output must be a new directory");
    for arm in ["current", "filtered", "shown", "temporal-off"] {
        std::fs::create_dir(output.join(arm)).unwrap();
    }
    std::fs::write(output.join("request.txt"), format!("{line}\n")).unwrap();
    let ((device, queue), _) = super::tests::test_import_gpu_and_foreign().unwrap();
    let mut scene = Scene::open(&path).unwrap();
    scene.set_muted(true);
    scene.enable_temporal_for_review().unwrap();
    scene.pause(Instant::now());
    scene.set_horizon(Horizon::Locked);
    let timing = scene.player(Player::timing).unwrap();
    let first_index = timing.index_at(view.at);
    scene.seek(view.at, Accuracy::Exact);
    let mut pipeline = ScenePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut previous = None;
    let mut log =
        std::io::BufWriter::new(std::fs::File::create_new(output.join("sources.tsv")).unwrap());
    writeln!(log, "source\ttime_ns\twidth\theight\tstamp").unwrap();
    for ordinal in 0..31 {
        let stamp = super::tests::wait_for_new_scene_frame(&scene, previous.as_ref());
        assert_eq!(stamp.index(), first_index + ordinal);
        settle_filtered(
            &scene,
            &mut pipeline,
            &device,
            &queue,
            &stamp,
            view.camera,
            None,
        );
        let installed = scene
            .primitive(view.camera)
            .filtered_capture
            .unwrap()
            .installed()
            .unwrap()
            .expect("review lost the installed correction");
        assert_eq!(installed.frame(), &stamp);
        let correction = installed.correction_for_review();
        assert_eq!(correction.frame(), &stamp);
        assert!(correction.belongs_to(&device));
        let mut encoder = device.create_command_encoder(&Default::default());
        let reads = [
            ("current", correction.current_texture()),
            ("filtered", correction.filtered_texture()),
        ]
        .map(|(arm, texture)| {
            (
                arm,
                texture.width(),
                texture.height(),
                super::panorama_review::PendingReadback::encode(&device, &mut encoder, texture),
            )
        });
        assert_eq!((reads[0].1, reads[0].2), (reads[1].1, reads[1].2));
        let submission = queue.submit([encoder.finish()]);
        for (arm, width, height, read) in reads {
            let rgba = read.read(&device, submission.clone());
            super::tests::write_review_ppm_sized(
                &output.join(arm),
                stamp.index(),
                width,
                height,
                &rgba,
            );
        }
        let shot = capture_shown(&scene, &mut pipeline, &device, &queue, view.camera);
        assert_eq!(shot.index, stamp.index());
        assert_eq!(scene.displayed_frame_stamp().as_ref(), Some(&stamp));
        super::tests::write_review_ppm_sized(
            &output.join("shown"),
            stamp.index(),
            shot.width,
            shot.height,
            &shot.rgba,
        );
        let primitive = scene.primitive(view.camera);
        let shown = primitive.shown.get().expect("review lost its shown source");
        assert_eq!(shown.frames.stamp(), stamp);
        let source_reframe = super::filtered::filtered_source_reframe(&shown, primitive.sampling);
        let coordinates = installed.coordinates_for_review();
        assert_eq!(coordinates, source_reframe.view_to_body_columns());
        std::fs::write(
            output.join(format!("frame-{:010}.coordinates.bin", stamp.index())),
            coordinate_bytes(coordinates),
        )
        .unwrap();
        let reframe = pipeline.resident_reframe(&primitive, &shown, 16.0 / 9.0);
        std::fs::write(
            output.join(format!("frame-{:010}.reframe.bin", stamp.index())),
            reframe.bytes(),
        )
        .unwrap();
        let normal = installed
            .prepare_view(&device, &reframe, wgpu::TextureFormat::Rgba8Unorm)
            .unwrap();
        assert_eq!(
            capture_correction_draw(&normal, &device, &queue, shot.width, shot.height),
            shot.rgba,
            "diagnostic draw must reproduce the exact shown pixels before removing the temporal term"
        );
        let temporal_off = installed
            .prepare_view_without_temporal_for_review(
                &device,
                &reframe,
                wgpu::TextureFormat::Rgba8Unorm,
            )
            .unwrap();
        assert_eq!(temporal_off.frame(), &stamp);
        let pixels =
            capture_correction_draw(&temporal_off, &device, &queue, shot.width, shot.height);
        super::tests::write_review_ppm_sized(
            &output.join("temporal-off"),
            stamp.index(),
            shot.width,
            shot.height,
            &pixels,
        );
        writeln!(
            log,
            "{}\t{}\t{}\t{}\t{stamp:?}",
            stamp.index(),
            stamp.timestamp().as_nanos(),
            correction.current_texture().width(),
            correction.current_texture().height()
        )
        .unwrap();
        log.flush().unwrap();
        eprintln!(
            "correction-fields: source {} exact installed current/filtered/shown/temporal-off",
            stamp.index()
        );
        previous = Some(stamp);
        if ordinal < 30 {
            scene.pump(Instant::now());
            scene.step(Instant::now(), 1);
        }
    }
}

fn coordinate_bytes(columns: [[f32; 4]; 3]) -> Vec<u8> {
    columns
        .into_iter()
        .flatten()
        .flat_map(f32::to_ne_bytes)
        .collect()
}

fn capture_correction_draw(
    draw: &PreparedCorrectionDraw,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filtered Scene temporal-off diagnostic target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("filtered Scene temporal-off diagnostic draw"),
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
        draw.draw(&mut pass);
    }
    let read = super::panorama_review::PendingReadback::encode(device, &mut encoder, &texture);
    let submission = queue.submit([encoder.finish()]);
    read.read(device, submission)
}

#[test]
fn one_x2_real_iso_transition_filters_exact_sources() {
    let Some(path) = std::env::var_os("KJERAG_ONE_X2_TEST_MEDIA") else {
        return;
    };
    let ((device, queue), _) = super::tests::test_import_gpu_and_foreign().unwrap();
    let mut scene = Scene::open(Path::new(&path)).unwrap();
    scene.set_muted(true);
    let timing = scene.player(Player::timing).unwrap();
    let capture = scene.show.as_ref().unwrap().one_xs.as_ref().unwrap();
    let mut provider = crate::temporal_fusion::settings::Provider::new(
        capture.diagnostic_calibration(),
        timing.fps() as f32,
    )
    .unwrap();
    let transition = (0..timing.frames)
        .find(|&index| {
            provider
                .parameters_at(timing.time_of(index).as_secs_f64() * 1_000.0)
                .unwrap()
                .radius
                > 0
        })
        .expect("the selected ONE X2 fixture has no nonzero-radius source");
    assert!(transition >= 3 && transition + 12 < timing.frames);
    eprintln!(
        "real ONE X2 first nonzero radius: frame {transition}, time {:.9}s",
        timing.time_of(transition).as_secs_f64()
    );
    scene.enable_temporal_for_review().unwrap();
    scene.pause(Instant::now());
    scene.seek(timing.time_of(transition - 3), Accuracy::Exact);
    let mut current = super::tests::wait_for_new_scene_frame(&scene, None);
    assert_eq!(current.index(), transition - 3);
    let mut pipeline = ScenePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    for offset in 0..12 {
        assert_eq!(current.index(), transition - 3 + offset);
        settle_filtered(
            &scene,
            &mut pipeline,
            &device,
            &queue,
            &current,
            Camera::default(),
            None,
        );
        assert_eq!(scene.displayed_frame_stamp().as_ref(), Some(&current));
        if offset < 11 {
            scene.pump(Instant::now());
            scene.step(Instant::now(), 1);
            current = super::tests::wait_for_new_scene_frame(&scene, Some(&current));
        }
    }
}

#[test]
fn tail_preroll_requires_seven_real_sources_and_only_fills_the_deficit() {
    assert_eq!(filtered_preroll_start(93, 99), None);
    assert_eq!(filtered_preroll_start(94, 99), Some(93));
    assert_eq!(filtered_preroll_start(99, 99), Some(93));
    assert_eq!(filtered_preroll_start(4, 4), None);
}

#[test]
fn x4_filtered_scene_preserves_exact_source_ownership() {
    let Some(path) = std::env::var_os("KJERAG_X4_TEST_MEDIA") else {
        return;
    };
    assert_filtered_scene(
        Path::new(&path),
        Duration::from_millis(612_078),
        Camera {
            yaw: -80.71_f32.to_radians(),
            pitch: -48.46_f32.to_radians(),
            fov: 95.45_f32.to_radians(),
        },
        std::env::var_os("KJERAG_FILTERED_REVIEW_DIR").map(PathBuf::from),
    );
}

#[test]
fn x4_filtered_scene_preserves_607_blotch_review_sources() {
    let Some(path) = std::env::var_os("KJERAG_X4_TEST_MEDIA") else {
        return;
    };
    // Begin at the same real source as the accepted offline 607 sequence.
    // Its first complete seven-source center is 18211; the first three
    // startup outputs are not used in that moving comparison.
    assert_filtered_scene(
        Path::new(&path),
        Duration::from_micros(607_540_267),
        Camera {
            yaw: -76.84_f32.to_radians(),
            pitch: -55.87_f32.to_radians(),
            fov: 108.79_f32.to_radians(),
        },
        std::env::var_os("KJERAG_FILTERED_REVIEW_607_DIR").map(PathBuf::from),
    );
}

#[test]
fn one_x2_filtered_scene_preserves_exact_source_ownership() {
    let Some(path) = std::env::var_os("KJERAG_ONE_X2_TEST_MEDIA") else {
        return;
    };
    assert_filtered_scene(
        Path::new(&path),
        Duration::from_millis(212_512),
        Camera {
            yaw: 71.13_f32.to_radians(),
            pitch: -13.99_f32.to_radians(),
            fov: 57.95_f32.to_radians(),
        },
        std::env::var_os("KJERAG_FILTERED_REVIEW_X2_DIR").map(PathBuf::from),
    );
}

fn assert_filtered_scene(
    path: &Path,
    seek: Duration,
    review_camera: Camera,
    review_dir: Option<PathBuf>,
) {
    let ((device, queue), _) = super::tests::test_import_gpu_and_foreign().unwrap();
    let mut scene = Scene::open(path).unwrap();
    scene.set_muted(true);
    scene.enable_temporal_for_review().unwrap();
    scene.pause(Instant::now());
    scene.set_horizon(Horizon::Locked);

    let first = super::tests::wait_for_new_scene_frame(&scene, None);
    assert_eq!(first.index(), 0);
    assert!(!scene.is_playing());
    assert_eq!(scene.player(Player::is_playing), Some(false));
    let stats = scene.stats().unwrap();
    let position = scene.position(Instant::now());
    let primitive = scene.primitive(Camera::default());
    let filtered = primitive.filtered_capture.clone().unwrap();
    let raw = primitive.resident_capture.clone().unwrap();
    let mut pipeline = ScenePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);

    settle_filtered(
        &scene,
        &mut pipeline,
        &device,
        &queue,
        &first,
        Camera::default(),
        None,
    );
    let prepared_stats = scene.stats().unwrap();
    assert_eq!(
        Stats {
            audio: None,
            ..prepared_stats
        },
        Stats {
            audio: None,
            ..stats
        },
        "paused preparation changed video accounting"
    );
    // Audio decoding may fill the ring while paused. Only its queued level
    // may grow; playback accounting and the presentation clock must not move.
    match (stats.audio, prepared_stats.audio) {
        (Some(before), Some(after)) => {
            assert_eq!(
                (after.underruns, after.dropped, after.offset, after.worst),
                (
                    before.underruns,
                    before.dropped,
                    before.offset,
                    before.worst
                ),
                "paused preparation changed audio accounting"
            );
            assert!(
                after.queued >= before.queued,
                "paused preparation consumed queued audio"
            );
        }
        (None, None) => {}
        _ => panic!("paused preparation changed audio-device ownership"),
    }
    assert_eq!(scene.position(Instant::now()), position);
    assert_eq!(filtered.accepted_stamp().unwrap().unwrap().index(), 6);
    assert!(!raw.acknowledged(&first).unwrap());
    assert!(raw.installed_stamp().unwrap().is_none());
    assert_installed_source_coordinates(&scene, Camera::default(), &first);
    let before_recreation =
        capture_shown(&scene, &mut pipeline, &device, &queue, Camera::default());
    assert_eq!(before_recreation.index, first.index());
    assert!(
        before_recreation
            .rgba
            .chunks_exact(4)
            .any(|pixel| pixel[..3] != [0, 0, 0]),
        "filtered Scene screenshot is entirely black"
    );

    // A view-only redraw consumes the exact installed filtered panorama. It
    // must neither admit another source nor move the presentation head.
    let accepted = filtered.accepted_stamp().unwrap();
    let installed = filtered
        .installed()
        .unwrap()
        .expect("the first filtered source is not installed");
    let coordinates = installed.coordinates_for_review();
    let locked = scene
        .primitive(Camera::default())
        .shown
        .get()
        .expect("the first filtered source is not shown");
    assert_eq!(locked.held.body_from_world, locked.body_from_world);
    let changed = Camera {
        yaw: 0.31,
        pitch: -0.17,
        fov: Camera::default().fov,
    };
    prepare_and_draw_at_aspect(&scene, &mut pipeline, &device, &queue, changed, 1.0);
    assert_eq!(filtered.accepted_stamp().unwrap(), accepted);
    assert_eq!(filtered.installed_stamp().unwrap().as_ref(), Some(&first));
    assert_eq!(scene.displayed_frame_stamp().as_ref(), Some(&first));
    let changed_primitive = scene.primitive(changed);
    let changed_shown = changed_primitive
        .shown
        .get()
        .expect("the changed-camera redraw lost its shown source");
    assert!(Arc::ptr_eq(&changed_shown.frames, &locked.frames));
    let changed_installed = filtered.installed().unwrap().unwrap();
    assert!(Arc::ptr_eq(&changed_installed, &installed));
    assert_eq!(changed_installed.coordinates_for_review(), coordinates);

    scene.set_horizon(Horizon::Free);
    prepare_and_draw_at_aspect(&scene, &mut pipeline, &device, &queue, changed, 4.0 / 3.0);
    assert_eq!(filtered.accepted_stamp().unwrap(), accepted);
    let free_primitive = scene.primitive(changed);
    let free_shown = free_primitive
        .shown
        .get()
        .expect("the free-horizon redraw lost its shown source");
    assert_eq!(free_shown.held.body_from_world, Quat::IDENTITY);
    assert_eq!(free_shown.body_from_world, locked.body_from_world);
    assert!(Arc::ptr_eq(&free_shown.frames, &locked.frames));
    let free_installed = filtered.installed().unwrap().unwrap();
    assert!(Arc::ptr_eq(&free_installed, &installed));
    assert_eq!(free_installed.coordinates_for_review(), coordinates);
    assert_installed_source_coordinates(&scene, changed, &first);

    // The pixel comparison below names the original locked display policy.
    scene.set_horizon(Horizon::Locked);
    prepare_and_draw(&scene, &mut pipeline, &device, &queue, Camera::default());
    let relocked = scene
        .primitive(Camera::default())
        .shown
        .get()
        .expect("restoring horizon lock lost the shown source");
    assert_eq!(relocked.held.body_from_world, relocked.body_from_world);
    assert_eq!(filtered.accepted_stamp().unwrap(), accepted);
    assert!(Arc::ptr_eq(
        &filtered.installed().unwrap().unwrap(),
        &installed
    ));

    // Recreating iced's renderer state on the same context restores the
    // capture-owned panorama. It does not rerun or acknowledge the raw path.
    drop(pipeline);
    let mut pipeline = ScenePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    prepare_and_draw(&scene, &mut pipeline, &device, &queue, Camera::default());
    let after_recreation = capture_shown(&scene, &mut pipeline, &device, &queue, Camera::default());
    assert_eq!(after_recreation.index, first.index());
    assert_eq!(after_recreation.width, before_recreation.width);
    assert_eq!(after_recreation.height, before_recreation.height);
    assert_eq!(
        after_recreation.rgba, before_recreation.rgba,
        "same-context renderer recreation changed the exact filtered picture"
    );
    assert_eq!(filtered.installed_stamp().unwrap().as_ref(), Some(&first));
    assert!(!raw.acknowledged(&first).unwrap());

    // Cross enough center publications to wrap every seven-source history
    // slot. Each offered source remains blocked until its exact filtered
    // output, while the raw map facade remains uninvolved.
    let mut current = first;
    for _ in 0..9 {
        scene.pump(Instant::now());
        scene.step(Instant::now(), 1);
        let next = super::tests::wait_for_new_scene_frame(&scene, Some(&current));
        assert!(next.same_decode_epoch(&current));
        assert_eq!(next.index(), current.index() + 1);
        settle_filtered(
            &scene,
            &mut pipeline,
            &device,
            &queue,
            &next,
            Camera::default(),
            None,
        );
        assert!(!raw.acknowledged(&next).unwrap());
        assert!(raw.installed_stamp().unwrap().is_none());
        assert_installed_source_coordinates(&scene, Camera::default(), &next);
        current = next;
    }

    // An exact seek replaces the filter lineage but retains the old complete
    // Shown until the requested target's own seven-source window is ready.
    let old_shown = scene.displayed_frame_stamp().unwrap();
    let old_filtered = filtered;
    scene.seek(seek, Accuracy::Exact);
    assert_eq!(scene.displayed_frame_stamp().as_ref(), Some(&old_shown));
    let landing = super::tests::wait_for_new_scene_frame(&scene, Some(&current));
    assert!(!landing.same_decode_epoch(&current));
    let fresh = scene
        .primitive(review_camera)
        .filtered_capture
        .clone()
        .unwrap();
    assert!(!fresh.same_capture(&old_filtered));
    fresh.assert_restart_ownership(&old_filtered);
    assert_eq!(scene.displayed_frame_stamp().as_ref(), Some(&old_shown));
    settle_filtered(
        &scene,
        &mut pipeline,
        &device,
        &queue,
        &landing,
        review_camera,
        Some(&old_shown),
    );
    assert_eq!(scene.displayed_frame_stamp().as_ref(), Some(&landing));
    assert!(fresh.acknowledged(&landing).unwrap());
    assert!(!old_filtered.acknowledged(&landing).unwrap());
    assert_installed_source_coordinates(&scene, review_camera, &landing);

    if let Some(output) = review_dir {
        save_review_sequence(
            &mut scene,
            &mut pipeline,
            &device,
            &queue,
            landing,
            review_camera,
            &output,
        );
    }
}

fn settle_filtered(
    scene: &Scene,
    pipeline: &mut ScenePipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    expected: &FrameStamp,
    camera: Camera,
    retained_shown: Option<&FrameStamp>,
) {
    let capture = scene
        .primitive(camera)
        .filtered_capture
        .clone()
        .expect("temporal review lost its capture");
    let deadline = Instant::now() + DEADLINE;
    loop {
        let primitive = scene.primitive(camera);
        pipeline.prepare(&primitive, device, queue, 16.0 / 9.0);
        if capture.acknowledged(expected).unwrap() && pipeline.filtered_draw.is_some() {
            assert_eq!(capture.installed_stamp().unwrap().as_ref(), Some(expected));
            draw_test_pass(pipeline, device, queue);
            return;
        }
        if let Some(retained) = retained_shown {
            assert_eq!(scene.displayed_frame_stamp().as_ref(), Some(retained));
        }
        assert_eq!(scene.frame_stamp().as_ref(), Some(expected));
        assert!(
            Instant::now() < deadline,
            "filtered source {expected:?} did not install"
        );
        if let Next::Stopped(error) = scene.pump(Instant::now()) {
            panic!("filtered Scene stopped before {expected:?}: {error}");
        }
        device.poll(wgpu::PollType::Poll).unwrap();
        std::thread::yield_now();
    }
}

fn assert_installed_source_coordinates(scene: &Scene, camera: Camera, expected: &FrameStamp) {
    let primitive = scene.primitive(camera);
    let shown = primitive
        .shown
        .get()
        .expect("an installed filtered source must also be shown");
    assert_eq!(&shown.frames.stamp(), expected);
    let capture = primitive
        .filtered_capture
        .as_ref()
        .expect("the filtered Scene lost its capture");
    let installed = capture
        .installed()
        .unwrap()
        .expect("the acknowledged filtered source is not installed");
    assert_eq!(installed.frame(), expected);
    let source = super::filtered::filtered_source_reframe(&shown, primitive.sampling);
    assert_eq!(
        installed.coordinates_for_review(),
        source.view_to_body_columns(),
        "installed correction retained another source pose"
    );
}

fn prepare_and_draw(
    scene: &Scene,
    pipeline: &mut ScenePipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    camera: Camera,
) {
    prepare_and_draw_at_aspect(scene, pipeline, device, queue, camera, 16.0 / 9.0);
}

fn prepare_and_draw_at_aspect(
    scene: &Scene,
    pipeline: &mut ScenePipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    camera: Camera,
    aspect: f32,
) {
    pipeline.prepare(&scene.primitive(camera), device, queue, aspect);
    assert!(pipeline.filtered_draw.is_some());
    draw_test_pass(pipeline, device, queue);
}

fn draw_test_pass(pipeline: &ScenePipeline, device: &wgpu::Device, queue: &wgpu::Queue) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filtered Scene integration target"),
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("filtered Scene integration draw"),
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
    queue.submit([encoder.finish()]);
}

fn save_review_sequence(
    scene: &mut Scene,
    pipeline: &mut ScenePipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    first: FrameStamp,
    camera: Camera,
    output: &Path,
) {
    std::fs::create_dir(output).unwrap();
    let mut stamps =
        std::io::BufWriter::new(std::fs::File::create_new(output.join("sources.tsv")).unwrap());
    writeln!(stamps, "frame\ttime_seconds\tstamp").unwrap();
    let mut current = first;
    for ordinal in 0..31 {
        if ordinal != 0 {
            scene.pump(Instant::now());
            scene.step(Instant::now(), 1);
            let next = super::tests::wait_for_new_scene_frame(scene, Some(&current));
            assert!(next.same_decode_epoch(&current));
            assert_eq!(next.index(), current.index() + 1);
            settle_filtered(scene, pipeline, device, queue, &next, camera, None);
            current = next;
        }
        let shot = capture_shown(scene, pipeline, device, queue, camera);
        assert_eq!(shot.index, current.index());
        super::tests::write_review_ppm_sized(
            output,
            current.index(),
            shot.width,
            shot.height,
            &shot.rgba,
        );
        writeln!(
            stamps,
            "{}\t{:.9}\t{current:?}",
            current.index(),
            shot.time.as_secs_f64()
        )
        .unwrap();
    }
    stamps.flush().unwrap();
}

fn capture_shown(
    scene: &Scene,
    pipeline: &mut ScenePipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    camera: Camera,
) -> capture::Shot {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    scene.capture(Request {
        width: 1280,
        then: Box::new(move |shot| {
            let _ = send.send(shot);
        }),
    });
    let deadline = Instant::now() + DEADLINE;
    loop {
        pipeline.prepare(&scene.primitive(camera), device, queue, 16.0 / 9.0);
        if let Ok(shot) = receive.try_recv() {
            return shot.unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "filtered screenshot did not complete"
        );
        device.poll(wgpu::PollType::Poll).unwrap();
        std::thread::yield_now();
    }
}

#[test]
fn x4_near_eof_seek_uses_real_preroll_without_publishing_it() {
    let Some(path) = std::env::var_os("KJERAG_X4_TEST_MEDIA") else {
        return;
    };
    assert_near_eof_preroll(Path::new(&path), &[3, 0]);
}

#[test]
fn one_x2_last_frame_seek_uses_real_preroll_without_publishing_it() {
    let Some(path) = std::env::var_os("KJERAG_ONE_X2_TEST_MEDIA") else {
        return;
    };
    assert_near_eof_preroll(Path::new(&path), &[0]);
}

fn assert_near_eof_preroll(path: &Path, offsets_from_last: &[u64]) {
    let ((device, queue), _) = super::tests::test_import_gpu_and_foreign().unwrap();
    let mut scene = Scene::open(path).unwrap();
    scene.set_muted(true);
    scene.enable_temporal_for_review().unwrap();
    scene.pause(Instant::now());
    let first = super::tests::wait_for_new_scene_frame(&scene, None);
    let mut pipeline = ScenePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    settle_filtered(
        &scene,
        &mut pipeline,
        &device,
        &queue,
        &first,
        Camera::default(),
        None,
    );
    let timing = scene.player(Player::timing).unwrap();
    let last = timing.frames - 1;
    let mut previous_epoch = first;
    for &offset in offsets_from_last {
        let target = last - offset;
        let old_shown = scene.displayed_frame_stamp().unwrap();
        let old_pixels = capture_shown(&scene, &mut pipeline, &device, &queue, Camera::default());
        scene.seek(timing.time_of(target), Accuracy::Exact);
        assert_eq!(scene.displayed_frame_stamp().as_ref(), Some(&old_shown));
        assert_eq!(scene.position(Instant::now()), timing.time_of(target));

        let mut current = super::tests::wait_for_new_scene_frame(&scene, Some(&previous_epoch));
        assert!(!current.same_decode_epoch(&previous_epoch));
        assert_eq!(current.index(), last.saturating_sub(6));
        let mut checked_retained_pixels = false;
        loop {
            let before_target = current.index() != target;
            settle_filtered(
                &scene,
                &mut pipeline,
                &device,
                &queue,
                &current,
                Camera::default(),
                before_target.then_some(&old_shown),
            );
            assert_eq!(scene.position(Instant::now()), timing.time_of(target));
            if before_target && !checked_retained_pixels {
                let retained =
                    capture_shown(&scene, &mut pipeline, &device, &queue, Camera::default());
                assert_eq!(retained.index, old_pixels.index);
                assert_eq!(retained.time, old_pixels.time);
                assert_eq!(retained.rgba, old_pixels.rgba);
                checked_retained_pixels = true;
            }
            if !before_target {
                break;
            }
            assert_eq!(scene.displayed_frame_stamp().as_ref(), Some(&old_shown));
            scene.pump(Instant::now());
            current = super::tests::wait_for_new_scene_frame(&scene, Some(&current));
        }
        assert_eq!(current.index(), target);
        assert_eq!(scene.displayed_frame_stamp().as_ref(), Some(&current));
        assert_ne!(current, old_shown);
        assert!(checked_retained_pixels);
        previous_epoch = current;
    }
}

#[test]
fn x4_complete_tail_presents_every_filtered_source_without_paused_retries() {
    let Some(path) = std::env::var_os("KJERAG_X4_TEST_MEDIA") else {
        return;
    };
    let ((device, queue), _) = super::tests::test_import_gpu_and_foreign().unwrap();
    let mut scene = Scene::open(Path::new(&path)).unwrap();
    scene.set_muted(true);
    scene.enable_temporal_for_review().unwrap();
    scene.pause(Instant::now());
    let timing = scene.player(|player| player.timing()).unwrap();
    let first_index = timing.frames - 7;
    scene.seek(timing.time_of(first_index), Accuracy::Exact);
    let mut current = super::tests::wait_for_new_scene_frame(&scene, None);
    assert_eq!(current.index(), first_index);
    let mut pipeline = ScenePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    for ordinal in 0..7 {
        if ordinal != 0 {
            scene.step(Instant::now(), 1);
            let next = super::tests::wait_for_new_scene_frame(&scene, Some(&current));
            assert!(next.same_decode_epoch(&current));
            assert_eq!(next.index(), first_index + ordinal);
            current = next;
        }
        settle_filtered(
            &scene,
            &mut pipeline,
            &device,
            &queue,
            &current,
            Camera::default(),
            None,
        );
        assert_eq!(scene.displayed_frame_stamp().as_ref(), Some(&current));
        // Retire the paused replay intent, then prepare again with that exact
        // installed source. Remaining ready tail pictures are not a reason
        // to keep waking a paused window whose current picture is complete.
        scene.pump(Instant::now());
        pipeline.prepare(
            &scene.primitive(Camera::default()),
            &device,
            &queue,
            16.0 / 9.0,
        );
        assert_eq!(scene.pump(Instant::now()), Next::Never);
    }
    assert!(
        scene
            .primitive(Camera::default())
            .filtered_capture
            .unwrap()
            .is_finished()
            .unwrap()
    );
    assert_eq!(current.index(), timing.frames - 1);
}
