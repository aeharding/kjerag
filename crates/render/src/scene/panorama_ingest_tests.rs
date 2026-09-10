//! Real decoded-source coverage for non-presenting temporal preparation.
//!
//! This deliberately does not select the temporal filter in ordinary playback.
//! It checks its source producer against the existing displayed-source consumer,
//! including all six paused successors and a fresh seek lineage on each camera.

use super::*;
use crate::flow::one_xs_belt_gpu::ResidentPanoramaIngest;
use sha2::{Digest, Sha256};

#[test]
fn x4_panorama_preparation_keeps_the_player_paused_and_matches_displayed_sources() {
    let Some(path) = std::env::var_os("KJERAG_X4_TEST_MEDIA") else {
        return;
    };
    assert_preparation(Path::new(&path), Duration::from_millis(612_078));
}

#[test]
fn x2_panorama_preparation_keeps_the_player_paused_and_matches_displayed_sources() {
    let Some(path) = std::env::var_os("KJERAG_ONE_X2_TEST_MEDIA") else {
        return;
    };
    assert_preparation(Path::new(&path), Duration::from_millis(212_512));
}

fn assert_preparation(path: &Path, seek: Duration) {
    let ((device, queue), _) = super::tests::test_import_gpu_and_foreign().unwrap();
    let mut scene = Scene::open(path).unwrap();
    scene.disable_temporal_for_review().unwrap();
    scene.set_muted(true);
    scene.pause(Instant::now());
    let mut previous_epoch: Option<FrameStamp> = None;
    for landing in [None, Some(seek)] {
        if let Some(landing) = landing {
            scene.seek(landing, Accuracy::Exact);
        }
        let first = super::tests::wait_for_new_scene_frame(&scene, previous_epoch.as_ref());
        assert!(
            !scene.is_playing(),
            "paused landing retained autoplay intent"
        );
        if let Some(previous) = &previous_epoch {
            assert!(!first.same_decode_epoch(previous));
        } else {
            assert_eq!(first.index(), 0);
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        while scene.player_mut().unwrap().prepare_ahead(6).unwrap() < 6 {
            assert!(
                Instant::now() < deadline,
                "six paused successors did not decode"
            );
            assert_eq!(scene.frame_stamp().as_ref(), Some(&first));
            assert!(!scene.player(Player::is_playing).unwrap());
            assert!(!scene.is_playing());
            std::thread::sleep(Duration::from_millis(1));
        }

        let show = scene.show.as_ref().unwrap();
        let profile = show.one_xs_profile.as_ref().unwrap();
        let capture = show.one_xs.clone().unwrap();
        let mut pipeline = ScenePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        let size = Size::new(256, 128);
        let ingest = ResidentPanoramaIngest::new(
            pipeline.one_xs_gpu.clone(),
            Arc::clone(profile),
            show.held.orientation.clone(),
            size,
        )
        .unwrap();
        let now = Instant::now();
        let position = scene.position(now);
        let stats = scene.player(Player::stats).unwrap();
        let held = Holding {
            horizon: scene.horizon.get(),
            clock: scene.clock.get(),
            forced: scene.forced.get(),
            readout: scene.readout.get(),
        };
        let mut expected = Vec::new();
        for at in 0..7 {
            let frames = if at == 0 {
                scene.primitive(Camera::default()).view.unwrap().frames
            } else {
                scene
                    .player(|player| player.prepared_ahead(at - 1))
                    .flatten()
                    .unwrap()
            };
            let stamp = frames.stamp();
            let source_lifetime = Arc::downgrade(&frames);
            assert_eq!(stamp.index(), first.index() + at as u64);
            assert!(stamp.same_decode_epoch(&first));
            let view = scene
                .show
                .as_ref()
                .unwrap()
                .view_for(Arc::clone(&frames), held);
            let primitive = scene.primitive(Camera::default());
            let reframe = pipeline.resident_reframe(&primitive, &view, 1.0);
            assert!(ingest.try_submit(frames, reframe).unwrap());
            let deadline = Instant::now() + Duration::from_secs(15);
            let panorama = loop {
                if let Some(panorama) = ingest.poll().unwrap() {
                    break panorama;
                }
                assert!(
                    Instant::now() < deadline,
                    "panorama worker did not finish {stamp:?}"
                );
                assert!(!matches!(scene.pump(Instant::now()), Next::Stopped(_)));
                assert_eq!(scene.frame_stamp().as_ref(), Some(&first));
                std::thread::sleep(Duration::from_millis(1));
            };
            assert_eq!(panorama.frame(), &stamp);
            // Readback is test-only and starts after the worker has submitted
            // this source. No blocking read can obstruct its L1 chunk submits.
            let rgba = read_texture(&device, &queue, panorama.texture());
            assert_eq!(rgba.len(), (size.width * size.height * 4) as usize);
            assert!(rgba.chunks_exact(4).any(|pixel| pixel[..3] != [0, 0, 0]));
            expected.push((stamp, Sha256::digest(&rgba), source_lifetime, panorama));
            assert_eq!(scene.frame_stamp().as_ref(), Some(&first));
            assert_eq!(scene.player(Player::stats).unwrap(), stats);
            assert!(!scene.player(Player::is_playing).unwrap());
            assert!(!scene.is_playing());
            assert_eq!(scene.position(now + Duration::from_secs(30)), position);
            assert!(!capture.acknowledged(&first).unwrap());
            assert!(capture.installed_stamp().unwrap().is_none());
            assert!(ingest.root_slots_empty_for_test().unwrap());
        }
        assert!(ingest.poll().unwrap().is_none());
        assert!(ingest.is_drained().unwrap());

        // Now let the unchanged display path install and consume these exact
        // retained sources. Its independent stitch history must produce the
        // same pixels, not merely the same source labels.
        for (at, (stamp, expected_hash, _, _)) in expected.iter().enumerate() {
            if at != 0 {
                scene.step(Instant::now(), 1);
                let offered =
                    super::tests::wait_for_new_scene_frame(&scene, Some(&expected[at - 1].0));
                assert_eq!(&offered, stamp);
            }
            let installed = super::tests::prepare_and_draw_exact_resident_frame(
                &scene,
                &mut pipeline,
                &device,
                &queue,
                stamp,
            );
            assert!(installed.same_capture(&capture));
            let primitive = scene.primitive(Camera::default());
            let view = primitive.view.as_ref().unwrap();
            let reframe = pipeline.resident_reframe(&primitive, view, 1.0);
            let deadline = Instant::now() + Duration::from_secs(15);
            let draw = loop {
                device.poll(wgpu::PollType::Poll).unwrap();
                let (_, attachment) = pipeline.resident_one_xs.as_ref().unwrap();
                match attachment
                    .prepare_screenshot(&pipeline.one_xs_gpu, pipeline.format, |frame| {
                        assert_eq!(frame, stamp);
                        Ok(reframe)
                    })
                    .unwrap()
                {
                    ResidentScreenshotPrepare::Ready(draw) => break draw,
                    ResidentScreenshotPrepare::RetryFull => {
                        assert!(
                            Instant::now() < deadline,
                            "display panorama retirement did not clear"
                        );
                        // Ordinary preparation owns collection of completed
                        // draw permits, not the screenshot reservation itself.
                        pipeline.prepare(&primitive, &device, &queue, 1.0);
                    }
                    ResidentScreenshotPrepare::Empty => {
                        panic!("installed source has no screenshot")
                    }
                }
            };
            let mut encoder = device.create_command_encoder(&Default::default());
            let panorama = draw.encode_panorama(&device, &mut encoder, size).unwrap();
            assert_eq!(panorama.frame(), stamp);
            queue.submit([encoder.finish()]);
            let rgba = read_texture(&device, &queue, panorama.texture());
            assert_eq!(
                &Sha256::digest(&rgba),
                expected_hash,
                "processing/display pixels differ at {stamp:?}"
            );
            eprintln!(
                "panorama prepare/display source={} epoch={} rgba_sha256={}",
                stamp.index(),
                landing.is_some(),
                expected_hash
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            );
        }
        // Retaining all seven independent panorama textures must not retain
        // their decoder surfaces. By source six the first ordinary display
        // retirement has also completed and been collected.
        assert!(
            expected[0].2.upgrade().is_none(),
            "first prepared decoder source was retained after display advanced"
        );
        let first_panorama = read_texture(&device, &queue, expected[0].3.texture());
        assert_eq!(
            Sha256::digest(&first_panorama),
            expected[0].1,
            "retired decoder surface changed its retained panorama"
        );
        previous_epoch = Some(first);
    }
}

fn read_texture(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let mut encoder = device.create_command_encoder(&Default::default());
    let read = super::panorama_review::PendingReadback::encode(device, &mut encoder, texture);
    read.read(device, queue.submit([encoder.finish()]))
}
