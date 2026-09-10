//! Real decoded-source coverage for non-presenting temporal preparation.
//!
//! This deliberately does not select the temporal filter in ordinary playback.
//! It checks its source producer against the existing displayed-source consumer,
//! including all six paused successors and a fresh seek lineage on each camera.

use super::*;
use crate::direct_type2::panorama::PanoramaProjector;
use crate::flow::one_xs_belt_gpu::ResidentPanoramaIngest;
use crate::temporal_fusion::color::{GpuColorConversion, MatrixCoefficients};
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

#[test]
#[ignore = "full X4 source comparison and GPU wall-time diagnostic"]
fn x4_direct_compact_nv12_compares_old_rgb_conversion_and_reports_wall_time() {
    let path = std::env::var_os("KJERAG_X4_TEST_MEDIA")
        .expect("direct compact NV12 review needs real X4 footage");
    let ((device, queue), _) = super::tests::test_import_gpu_and_foreign().unwrap();
    let mut scene = Scene::open(Path::new(&path)).unwrap();
    scene.disable_temporal_for_review().unwrap();
    scene.set_muted(true);
    scene.pause(Instant::now());
    scene.set_horizon(Horizon::Locked);
    scene.seek(Duration::from_micros(612_078_000), Accuracy::Exact);
    let frame = super::tests::wait_for_new_scene_frame(&scene, None);
    eprintln!("direct compact source: {frame:?}");
    let mut selected = ScenePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    super::tests::prepare_and_draw_exact_resident_frame(
        &scene,
        &mut selected,
        &device,
        &queue,
        &frame,
    );
    scene.pump(Instant::now());
    let map = scene
        .diagnostic_one_xs_displayed_map()
        .unwrap()
        .expect("direct compact NV12 review has no installed map");
    assert_eq!(map.frame(), &frame);

    let mut diagnostic = ScenePipeline::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
    let camera = Camera {
        yaw: -80.71_f32.to_radians(),
        pitch: -48.46_f32.to_radians(),
        fov: 95.45_f32.to_radians(),
    };
    let primitive = scene.primitive(camera);
    let prepared = diagnostic
        .prepare_one_xs_picture(&primitive, 16.0 / 9.0)
        .expect("direct compact NV12 review lost the displayed source");
    assert_eq!(prepared.frame(), &frame);
    let source = prepared.reframe().frame_size();
    let size = Size::new((source[0] as u32) * 2, source[1] as u32);
    assert_eq!(size, Size::new(7680, 3840));

    let mut draw =
        DirectMapDraw::new_panorama(&device, &diagnostic.layout, map.fusion().is_some()).unwrap();
    draw.upload(&queue, &map);
    assert_eq!(draw.bound_frame(), Some(&frame));
    let compact = draw.prepare_compact_nv12(&device).unwrap();
    let matrix = MatrixCoefficients::from_source_rgb(prepared.reframe().source_color_matrix());
    let conversion = GpuColorConversion::new(&device);
    // Flush all source/map uploads and finish all pipeline construction before
    // either the equality submission or a timed interval.
    queue.submit([]);
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();

    let mut encoder = device.create_command_encoder(&Default::default());
    let old_rgb = draw
        .encode_panorama(
            &device,
            &mut encoder,
            &diagnostic.bind_group,
            &prepared,
            size,
        )
        .unwrap();
    let old = conversion
        .encode_rgb_to_nv12(&mut encoder, old_rgb.texture(), matrix)
        .unwrap();
    let direct = compact
        .encode(
            &device,
            &mut encoder,
            &diagnostic.bind_group,
            &prepared,
            size,
        )
        .unwrap();
    assert_eq!(old_rgb.frame(), direct.frame());
    assert_eq!(direct.frame(), &frame);
    assert_eq!(direct.size(), size);
    assert_eq!(direct.coefficients(), matrix);
    assert!(direct.belongs_to(&device));
    let old_y = TextureReadback::encode(&device, &mut encoder, &old.y, 1);
    let old_uv = TextureReadback::encode(&device, &mut encoder, &old.uv, 2);
    let direct_y = TextureReadback::encode(&device, &mut encoder, direct.packed_y(), 4);
    let direct_uv = TextureReadback::encode(&device, &mut encoder, direct.uv(), 2);
    let submission = queue.submit([encoder.finish()]);
    let old_y = old_y.read(&device, submission.clone());
    let old_uv = old_uv.read(&device, submission.clone());
    let direct_y = unpack_y_quartets(&direct_y.read(&device, submission.clone()), size);
    let direct_uv = direct_uv.read(&device, submission);
    report_code_differences("Y", &old_y, &direct_y);
    report_code_differences("UV", &old_uv, &direct_uv);
    let mut outliers: Vec<_> = old_y
        .iter()
        .zip(&direct_y)
        .enumerate()
        .filter_map(|(at, (&a, &b))| (a.abs_diff(b) > 1).then_some((a.abs_diff(b), at, a, b)))
        .collect();
    outliers.sort_unstable_by_key(|&(difference, at, _, _)| (std::cmp::Reverse(difference), at));
    for (difference, at, a, b) in outliers.iter().take(16) {
        eprintln!(
            "direct compact Y outlier x={} y={} old={a} direct={b} difference={difference}",
            at % size.width as usize,
            at / size.width as usize,
        );
    }

    if let Some(output) = std::env::var_os("KJERAG_DIRECT_COMPACT_REVIEW_DIR") {
        let output = PathBuf::from(output);
        let old_dir = output.join("old-rgb-roundtrip");
        let direct_dir = output.join("direct-compact-roundtrip");
        std::fs::create_dir_all(&old_dir).unwrap();
        std::fs::create_dir_all(&direct_dir).unwrap();

        let unpack = PackedYUnpack::new(&device);
        let projector = PanoramaProjector::new(&device);
        let view = Size::new(1280, 720);
        let mut encoder = device.create_command_encoder(&Default::default());
        let old_rgb = conversion
            .encode_nv12_to_rgb(&mut encoder, &old, matrix)
            .unwrap();
        let expanded_y = unpack.encode(&device, &mut encoder, direct.packed_y(), size);
        let direct_rgb = conversion
            .encode_planes_to_rgb(&mut encoder, &expanded_y, direct.uv(), matrix)
            .unwrap();
        let old_view = projector
            .encode(&device, &mut encoder, &old_rgb, &prepared.reframe(), view)
            .unwrap();
        let direct_view = projector
            .encode(
                &device,
                &mut encoder,
                &direct_rgb,
                &prepared.reframe(),
                view,
            )
            .unwrap();
        let expanded_y = TextureReadback::encode(&device, &mut encoder, &expanded_y, 1);
        let old_view = TextureReadback::encode(&device, &mut encoder, &old_view, 4);
        let direct_view = TextureReadback::encode(&device, &mut encoder, &direct_view, 4);
        let submission = queue.submit([encoder.finish()]);
        let expanded_y = expanded_y.read(&device, submission.clone());
        let old_view = old_view.read(&device, submission.clone());
        let direct_view = direct_view.read(&device, submission);
        assert_eq!(expanded_y, direct_y, "GPU packed-Y unpack changed a code");
        for (x, y) in [(0, 0), (2430, 300), (3841, 1919), (7679, 3839)] {
            let at = (y * size.width + x) as usize;
            eprintln!(
                "direct compact expanded Y x={x} y={y} code={}",
                expanded_y[at]
            );
        }
        super::tests::write_review_ppm_sized(
            &old_dir,
            frame.index(),
            view.width,
            view.height,
            &old_view,
        );
        super::tests::write_review_ppm_sized(
            &direct_dir,
            frame.index(),
            view.width,
            view.height,
            &direct_view,
        );
        let sample_receipt = [(0, 0), (2430, 300), (3841, 1919), (7679, 3839)]
            .map(|(x, y)| {
                let code = expanded_y[(y * size.width + x) as usize];
                format!("expanded_y\t{x}\t{y}\t{code}")
            })
            .join("\n");
        let digest = |bytes: &[u8]| {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        std::fs::File::create_new(output.join("receipt.txt"))
            .and_then(|mut file| {
                use std::io::Write;
                writeln!(file, "source={frame:?}")?;
                writeln!(file, "seek_us=612078000")?;
                writeln!(file, "view=1280x720")?;
                writeln!(file, "yaw_deg=-80.71")?;
                writeln!(file, "pitch_deg=-48.46")?;
                writeln!(file, "fov_deg=95.45")?;
                writeln!(file, "horizon_locked=true")?;
                writeln!(file, "old_rgba_sha256={}", digest(&old_view))?;
                writeln!(file, "direct_rgba_sha256={}", digest(&direct_view))?;
                writeln!(file, "{sample_receipt}")
            })
            .unwrap();
        report_code_differences("projected RGBA", &old_view, &direct_view);
    }

    eprintln!(
        "direct compact timing excludes the later packed-Y unpack and UV copy into temporal history"
    );
    eprintln!(
        "old RGB evaluates raster-interpolated full-resolution in.uv; direct compact evaluates half-resolution in.uv plus four half-texel full-resolution offsets, so the code comparison above measures that interpolant arithmetic difference too"
    );
    for pair in 0..4 {
        let start = Instant::now();
        let mut encoder = device.create_command_encoder(&Default::default());
        let rgb = draw
            .encode_panorama(
                &device,
                &mut encoder,
                &diagnostic.bind_group,
                &prepared,
                size,
            )
            .unwrap();
        let nv12 = conversion
            .encode_rgb_to_nv12(&mut encoder, rgb.texture(), matrix)
            .unwrap();
        let submission = queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        let milliseconds = start.elapsed().as_secs_f64() * 1000.0;
        drop(nv12);
        drop(rgb);
        eprintln!("direct compact pair {pair} old RGB then NV12 {milliseconds:.3} ms");

        let start = Instant::now();
        let mut encoder = device.create_command_encoder(&Default::default());
        let output = compact
            .encode(
                &device,
                &mut encoder,
                &diagnostic.bind_group,
                &prepared,
                size,
            )
            .unwrap();
        let submission = queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        let milliseconds = start.elapsed().as_secs_f64() * 1000.0;
        drop(output);
        eprintln!("direct compact pair {pair} direct packed YUV {milliseconds:.3} ms");
    }
}

struct PackedYUnpack {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl PackedYUnpack {
    fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test packed panorama Y unpack"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("test packed panorama Y unpack"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("test packed panorama Y unpack"),
            source: wgpu::ShaderSource::Wgsl(PACKED_Y_UNPACK_WGSL.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("test packed panorama Y unpack"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("triangle"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("unpack_y"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::R8Unorm,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            device: device.clone(),
            layout,
            pipeline,
        }
    }

    fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        packed: &wgpu::Texture,
        full: Size,
    ) -> wgpu::Texture {
        assert_eq!(self.device, *device);
        assert_eq!(
            [packed.width() * 2, packed.height() * 2],
            [full.width, full.height]
        );
        assert_eq!(packed.format(), wgpu::TextureFormat::Rgba8Unorm);
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test expanded panorama Y"),
            size: full.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let packed_view = packed.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test packed panorama Y unpack"),
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&packed_view),
            }],
        });
        let output_view = output.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("test packed panorama Y unpack"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &output_view,
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
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        output
    }
}

const PACKED_Y_UNPACK_WGSL: &str = r#"
@group(0) @binding(0) var packed_y: texture_2d<f32>;

struct Vertex { @builtin(position) position: vec4<f32>, }
@vertex fn triangle(@builtin(vertex_index) vertex: u32) -> Vertex {
  let positions = array<vec2<f32>, 3>(
    vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
  return Vertex(vec4(positions[vertex], 0.0, 1.0));
}

@fragment fn unpack_y(@builtin(position) position: vec4<f32>) -> @location(0) f32 {
  let pixel = vec2<u32>(position.xy);
  let quartet = textureLoad(packed_y, vec2<i32>(pixel / vec2(2u)), 0);
  let lane = (pixel.y & 1u) * 2u + (pixel.x & 1u);
  return quartet[lane];
}
"#;

struct TextureReadback {
    buffer: wgpu::Buffer,
    packed_row: u32,
    padded_row: u32,
    height: u32,
}

impl TextureReadback {
    fn encode(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        bytes_per_pixel: u32,
    ) -> Self {
        let packed_row = texture.width() * bytes_per_pixel;
        let padded_row = packed_row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let height = texture.height();
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("direct compact NV12 comparison readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        Self {
            buffer,
            packed_row,
            padded_row,
            height,
        }
    }

    fn read(self, device: &wgpu::Device, submission: wgpu::SubmissionIndex) -> Vec<u8> {
        self.buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        let mapped = self.buffer.slice(..).get_mapped_range();
        let mut bytes = Vec::with_capacity((self.packed_row * self.height) as usize);
        for row in mapped.chunks_exact(self.padded_row as usize) {
            bytes.extend_from_slice(&row[..self.packed_row as usize]);
        }
        drop(mapped);
        self.buffer.unmap();
        bytes
    }
}

fn unpack_y_quartets(packed: &[u8], full: Size) -> Vec<u8> {
    let half_width = full.width / 2;
    let mut y = vec![0; (full.width * full.height) as usize];
    for (tile, quartet) in packed.chunks_exact(4).enumerate() {
        let tile = tile as u32;
        let x = (tile % half_width) * 2;
        let top = (tile / half_width) * 2 * full.width + x;
        let bottom = top + full.width;
        y[top as usize] = quartet[0];
        y[top as usize + 1] = quartet[1];
        y[bottom as usize] = quartet[2];
        y[bottom as usize + 1] = quartet[3];
    }
    y
}

fn report_code_differences(plane: &str, old: &[u8], direct: &[u8]) {
    assert_eq!(old.len(), direct.len());
    let mut histogram = [0u64; 256];
    for (&a, &b) in old.iter().zip(direct) {
        histogram[a.abs_diff(b) as usize] += 1;
    }
    let differing: u64 = histogram[1..].iter().sum();
    let max = histogram.iter().rposition(|&count| count != 0).unwrap_or(0);
    let nonzero = histogram
        .iter()
        .enumerate()
        .filter(|(_, count)| **count != 0)
        .map(|(difference, count)| format!("{difference}:{count}"))
        .collect::<Vec<_>>()
        .join(",");
    eprintln!(
        "direct compact {plane}: components={} differing={differing} max={max} histogram={nonzero}",
        old.len()
    );
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
