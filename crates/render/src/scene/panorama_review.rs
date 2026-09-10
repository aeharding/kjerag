//! Offline actual-Scene controls for the missing post-stitch image path.
//!
//! No playback policy is selected here. The caller retains its imported source
//! until this diagnostic's complete submission finishes. Every intermediate
//! keeps the same source association; the source clock and color solve do not
//! advance while the comparison arms are being drawn.

use std::io::Write;

use sha2::{Digest, Sha256};

use super::*;
use crate::direct_type2::panorama::PanoramaProjector;
use crate::temporal_fusion::color::{GpuColorConversion, MatrixCoefficients};

mod temporal;

pub(super) struct PanoramaReview {
    projector: PanoramaProjector,
    conversion: GpuColorConversion,
    draw: Option<DirectMapDraw>,
    size: Option<Size>,
    previous: Option<FrameStamp>,
    temporal: Option<temporal::TemporalReview>,
    gpu_pyramid: Option<crate::temporal_fusion::pyramid::gpu::Builder>,
    parallel_refine: bool,
    output: PathBuf,
    log: std::io::BufWriter<std::fs::File>,
}

impl PanoramaReview {
    pub(super) fn new(device: &wgpu::Device, output: &Path) -> Self {
        for arm in ["panorama-rgb", "panorama-nv12-control"] {
            std::fs::create_dir(output.join(arm)).unwrap();
        }
        let mut log = std::io::BufWriter::new(
            std::fs::File::create_new(output.join("panorama-controls.tsv")).unwrap(),
        );
        writeln!(
            log,
            "source\ttime_ns\tpanorama_width\tpanorama_height\tarm\trgba_sha256\tmax_code_difference\tmean_absolute_code_difference\tp99_code_difference"
        )
        .unwrap();
        let temporal_enabled = std::env::var_os("KJERAG_PANORAMA_TEMPORAL_ISO100")
            .map(|value| {
                assert_eq!(value, "1", "set the explicit ISO100 diagnostic flag to 1");
                true
            })
            .unwrap_or(false);
        let gpu_motion = std::env::var_os("KJERAG_PANORAMA_GPU_MOTION")
            .map(|value| {
                assert_eq!(
                    value, "1",
                    "set the explicit GPU motion diagnostic flag to 1"
                );
                assert!(
                    temporal_enabled,
                    "GPU motion review needs the temporal diagnostic"
                );
                true
            })
            .unwrap_or(false);
        let parallel_search = std::env::var_os("KJERAG_PANORAMA_PARALLEL_SEARCH")
            .map(|value| {
                assert_eq!(
                    value, "1",
                    "set the explicit parallel search diagnostic flag to 1"
                );
                assert!(
                    temporal_enabled,
                    "parallel search review needs the temporal diagnostic"
                );
                true
            })
            .unwrap_or(false);
        let gpu_pyramid_enabled = std::env::var_os("KJERAG_PANORAMA_GPU_PYRAMID")
            .map(|value| {
                assert_eq!(
                    value, "1",
                    "set the explicit GPU pyramid diagnostic flag to 1"
                );
                assert!(
                    temporal_enabled,
                    "GPU pyramid review needs the temporal diagnostic"
                );
                true
            })
            .unwrap_or(false);
        let parallel_refine = std::env::var_os("KJERAG_PANORAMA_PARALLEL_REFINE")
            .map(|value| {
                assert_eq!(
                    value, "1",
                    "set the explicit parallel-refine diagnostic flag to 1"
                );
                assert!(
                    temporal_enabled,
                    "parallel refinement review needs the temporal diagnostic"
                );
                assert!(
                    gpu_pyramid_enabled,
                    "parallel refinement review needs GPU pyramids"
                );
                assert!(
                    gpu_motion,
                    "parallel refinement review needs GPU motion packing"
                );
                true
            })
            .unwrap_or(false);
        let view_scissors = std::env::var_os("KJERAG_PANORAMA_VIEW_SCISSORS")
            .map(|value| {
                assert_eq!(
                    value, "1",
                    "set the explicit view-scissor diagnostic flag to 1"
                );
                assert!(
                    temporal_enabled,
                    "view scissors need the temporal diagnostic"
                );
                true
            })
            .unwrap_or(false);
        let temporal = temporal_enabled.then(|| {
            temporal::TemporalReview::new(
                device,
                output,
                gpu_motion,
                parallel_search,
                parallel_refine,
                view_scissors,
            )
        });
        let gpu_pyramid = gpu_pyramid_enabled.then(|| {
            std::fs::write(
                output.join("panorama-pyramid.txt"),
                "GPU half-Y and separable pyramid; CPU search still reads all logical levels.\n\
                 This changes preparation execution only, not temporal parameters or source history.\n",
            ).unwrap();
            crate::temporal_fusion::pyramid::gpu::Builder::new(device)
        });
        Self {
            projector: PanoramaProjector::new(device),
            conversion: GpuColorConversion::new(device),
            draw: None,
            size: None,
            previous: None,
            temporal,
            gpu_pyramid,
            parallel_refine,
            output: output.to_owned(),
            log,
        }
    }

    pub(super) fn capture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &mut ScenePipeline,
        map: &OneXsMapFrame,
        ordinary_rgba: &[u8],
    ) {
        let prepared = pipeline.prepared_picture.clone().unwrap();
        MapBindError::require_frame(map.frame(), Some(&prepared)).unwrap();
        require_successor(self.previous.as_ref(), prepared.frame()).unwrap();
        let reframe = prepared.reframe();
        assert!(!reframe.linearizes_output());
        let source_size = reframe.frame_size();
        assert!(
            source_size.into_iter().all(|value| {
                value.is_finite() && value > 0.0 && value == (value as u32) as f32
            })
        );
        let size = Size::new(
            (source_size[0] as u32).checked_mul(2).unwrap(),
            source_size[1] as u32,
        );
        assert_eq!(
            size.width,
            2 * size.height,
            "review needs square lens images"
        );
        if let Some(previous) = self.size {
            assert_eq!(
                size, previous,
                "panorama geometry changed within the source epoch"
            );
        }
        self.size = Some(size);
        let view = Size::new(1280, 720);
        assert_eq!(ordinary_rgba.len(), (view.width * view.height * 4) as usize);

        // Authenticate the separate diagnostic picture binding against the
        // ordinary resident draw before adding any panorama resampling.
        let direct = super::tests::render_direct_map_pixels_sized(
            device,
            queue,
            pipeline,
            map,
            view.width,
            view.height,
        );
        assert!(
            ordinary_rgba
                .iter()
                .zip(&direct)
                .all(|(&a, &b)| a.abs_diff(b) <= 1)
        );

        if self.draw.is_none() {
            self.draw = Some(
                DirectMapDraw::new_panorama(device, &pipeline.layout, map.fusion().is_some())
                    .unwrap(),
            );
            std::fs::write(
                self.output.join("panorama-representation.txt"),
                format!(
                    "diagnostic only, no denoising or color-update smoothing\n\
                     body panorama: {}x{}, Rgba8Unorm gamma RGB\n\
                     source decode matrix: {:?}\n\
                     NV12 control: inverse of this matrix, neutral chroma 128/255, \
                     each UV sample averages its exact 2x2 RGB footprint, \
                     GPU UNorm storage, centered bilinear chroma reconstruction\n\
                     This is a disclosed Kjerag conversion, not the recovered Studio conversion.\n",
                    size.width,
                    size.height,
                    reframe.source_color_matrix(),
                ),
            )
            .unwrap();
        }
        let draw = self.draw.as_mut().unwrap();
        assert_eq!(draw.has_fusion(), map.fusion().is_some());
        draw.upload(queue, map);
        assert_eq!(draw.bound_frame(), Some(prepared.frame()));

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("same-source panorama representation controls"),
        });
        let panorama = draw
            .encode_panorama(device, &mut encoder, &pipeline.bind_group, &prepared, size)
            .unwrap();
        assert_eq!(panorama.frame(), prepared.frame());
        let projected = self
            .projector
            .encode(device, &mut encoder, panorama.texture(), &reframe, view)
            .unwrap();
        let matrix = MatrixCoefficients::from_source_rgb(reframe.source_color_matrix());
        let nv12 = self
            .conversion
            .encode_rgb_to_nv12(&mut encoder, panorama.texture(), matrix)
            .unwrap();
        let restored = self
            .conversion
            .encode_nv12_to_rgb(&mut encoder, &nv12, matrix)
            .unwrap();
        // Both derivatives above came from this exact stamped panorama. The
        // pure conversion/projector primitives neither mint nor choose stamps.
        let roundtrip = self
            .projector
            .encode(device, &mut encoder, &restored, &reframe, view)
            .unwrap();
        let rgb_read = PendingReadback::encode(device, &mut encoder, &projected);
        let roundtrip_read = PendingReadback::encode(device, &mut encoder, &roundtrip);
        let luma_read = (self.temporal.is_some() && self.gpu_pyramid.is_none())
            .then(|| PendingReadback::encode(device, &mut encoder, &nv12.y));
        let mut gpu_base = None;
        let pyramid_reads = self.gpu_pyramid.as_ref().map(|builder| {
            let pyramid = builder
                .encode_luma(device, &mut encoder, &nv12.y, 7)
                .unwrap();
            if self.parallel_refine {
                gpu_base = Some(pyramid.levels[0].clone());
            }
            pyramid
                .levels
                .iter()
                .map(|level| {
                    (
                        level.width(),
                        level.height(),
                        PendingReadback::encode(device, &mut encoder, level),
                    )
                })
                .collect::<Vec<_>>()
        });
        let submission = queue.submit([encoder.finish()]);
        let rgb = rgb_read.read(device, submission.clone());
        let roundtrip = roundtrip_read.read(device, submission.clone());
        let full_luma = luma_read.map(|read| read.read(device, submission.clone()));
        let gpu_levels = pyramid_reads.map(|reads| {
            reads
                .into_iter()
                .map(
                    |(width, height, read)| crate::temporal_fusion::pyramid::Level {
                        width: width as usize,
                        height: height as usize,
                        pixels: read.read(device, submission.clone()),
                    },
                )
                .collect()
        });
        // The imported decoded source is still owned by `pipeline` here.
        assert_eq!(
            pipeline.prepared_picture.as_ref().unwrap().frame(),
            panorama.frame()
        );
        assert_eq!(panorama.frame(), map.frame());

        for (arm, pixels, reference) in [
            ("panorama-rgb", &rgb, ordinary_rgba),
            ("panorama-nv12-control", &roundtrip, rgb.as_slice()),
        ] {
            super::tests::write_review_ppm_sized(
                &self.output.join(arm),
                prepared.frame().index(),
                view.width,
                view.height,
                pixels,
            );
            let (maximum, mean, p99) = differences(reference, pixels);
            let digest: String = Sha256::digest(pixels)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            writeln!(
                self.log,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.9}\t{}",
                prepared.frame().index(),
                prepared.frame().timestamp().as_nanos(),
                size.width,
                size.height,
                arm,
                digest,
                maximum,
                mean,
                p99,
            )
            .unwrap();
            eprintln!(
                "panorama-control: source {} {arm}: max {maximum}, mean {mean:.6}, p99 {p99}",
                prepared.frame().index(),
            );
        }
        self.log.flush().unwrap();
        if let Some(temporal) = self.temporal.as_mut() {
            let levels = gpu_levels.unwrap_or_else(|| {
                let base = half_size_luma_linear(&full_luma.unwrap(), size);
                crate::temporal_fusion::pyramid::build(
                    &base,
                    size.width as usize / 2,
                    size.height as usize / 2,
                    7,
                )
                .unwrap()
            });
            temporal.push(device, queue, &prepared, nv12, levels, gpu_base);
        }
        self.previous = Some(prepared.frame().clone());
    }
}

fn require_successor(previous: Option<&FrameStamp>, current: &FrameStamp) -> Result<(), String> {
    if let Some(previous) = previous {
        if !previous.same_decode_epoch(current) {
            return Err("panorama review crossed a decode epoch".into());
        }
        if previous.index().checked_add(1) != Some(current.index()) {
            return Err("panorama review sources are not contiguous".into());
        }
    }
    Ok(())
}

/// Half-size CV_8UC1 INTER_LINEAR, selected before Studio's gray pyramid.
/// Exact 2x reduction puts the source sample at (2x+0.5, 2y+0.5).
/// OpenCV 4.7.0 resize.cpp's fast-area dispatch rounds the four-pixel mean
/// upward at halves. The source here is a tightly packed GPU Y readback,
/// not a decoder allocation whose pitch could be inferred from width.
/// This bridge has static/source authority, not a same-input native receipt.
fn half_size_luma_linear(source: &[u8], size: Size) -> Vec<u8> {
    assert!(size.width > 0 && size.height > 0);
    assert!(size.width.is_multiple_of(2) && size.height.is_multiple_of(2));
    let width = size.width as usize;
    let height = size.height as usize;
    assert_eq!(source.len(), width * height);
    let mut result = Vec::with_capacity(width * height / 4);
    for rows in source.chunks_exact(width * 2) {
        for column in (0..width).step_by(2) {
            let sum: u16 = [column, column + 1, width + column, width + column + 1]
                .map(|index| u16::from(rows[index]))
                .into_iter()
                .sum();
            result.push(((sum + 2) / 4) as u8);
        }
    }
    result
}

fn differences(reference: &[u8], actual: &[u8]) -> (u8, f64, u8) {
    assert_eq!(reference.len(), actual.len());
    assert_eq!(reference.len() % 4, 0);
    let mut histogram = [0_u64; 256];
    for (a, b) in reference.chunks_exact(4).zip(actual.chunks_exact(4)) {
        for channel in 0..3 {
            histogram[a[channel].abs_diff(b[channel]) as usize] += 1;
        }
    }
    let count: u64 = histogram.iter().sum();
    assert!(count > 0);
    let maximum = histogram.iter().rposition(|&value| value != 0).unwrap() as u8;
    let sum: u64 = histogram
        .iter()
        .enumerate()
        .map(|(code, &n)| code as u64 * n)
        .sum();
    let mut cumulative = 0;
    let p99 = histogram
        .iter()
        .position(|&value| {
            cumulative += value;
            cumulative * 100 >= count * 99
        })
        .unwrap() as u8;
    (maximum, sum as f64 / count as f64, p99)
}

struct PendingReadback {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    bytes_per_pixel: u32,
    stride: u32,
}

impl PendingReadback {
    fn encode(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> Self {
        let bytes_per_pixel = match texture.format() {
            wgpu::TextureFormat::R8Unorm | wgpu::TextureFormat::R8Uint => 1,
            wgpu::TextureFormat::Rgba8Unorm => 4,
            other => panic!("offline panorama readback does not support {other:?}"),
        };
        let width = texture.width();
        let height = texture.height();
        let stride = (width * bytes_per_pixel).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("offline panorama control readback"),
            size: u64::from(stride) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        Self {
            buffer,
            width,
            height,
            bytes_per_pixel,
            stride,
        }
    }

    fn read(self, device: &wgpu::Device, submission: wgpu::SubmissionIndex) -> Vec<u8> {
        let slice = self.buffer.slice(..);
        let (send, receive) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .unwrap();
        receive.recv().unwrap().unwrap();
        let mapped = slice.get_mapped_range();
        let row_bytes = (self.width * self.bytes_per_pixel) as usize;
        let mut result = Vec::with_capacity(row_bytes * self.height as usize);
        for row in mapped
            .chunks_exact(self.stride as usize)
            .take(self.height as usize)
        {
            result.extend_from_slice(&row[..row_bytes]);
        }
        drop(mapped);
        self.buffer.unmap();
        result
    }
}

#[test]
fn review_rejects_gaps_duplicates_and_seek_epochs() {
    let first = FrameStamp::for_test(10, Duration::ZERO, None);
    let next = FrameStamp::for_test(11, Duration::from_millis(33), Some(&first));
    let gap = FrameStamp::for_test(12, Duration::from_millis(66), Some(&first));
    let sought = FrameStamp::for_test(11, Duration::from_millis(33), None);
    assert!(require_successor(None, &first).is_ok());
    assert!(require_successor(Some(&first), &next).is_ok());
    assert!(require_successor(Some(&first), &first).is_err());
    assert!(require_successor(Some(&first), &gap).is_err());
    assert!(require_successor(Some(&first), &sought).is_err());
}

#[test]
fn control_metrics_compare_rgb_without_alpha() {
    assert_eq!(differences(&[0, 5, 100, 0], &[1, 3, 103, 255]), (3, 2.0, 3));
}

#[test]
fn half_size_luma_uses_nonoverlapping_two_by_two_centres() {
    let ramp: Vec<u8> = (0..16).collect();
    assert_eq!(
        half_size_luma_linear(&ramp, Size::new(4, 4)),
        [3, 5, 11, 13]
    );
    assert_eq!(half_size_luma_linear(&[255; 16], Size::new(4, 4)), [255; 4]);
}

#[test]
fn half_size_luma_rounds_ties_up_without_intermediate_axis_rounding() {
    for site in 0..4 {
        let mut source = [0; 4];
        source[site] = 2;
        assert_eq!(half_size_luma_linear(&source, Size::new(2, 2)), [1]);
    }
    // Rounding horizontal pairs separately would incorrectly return one.
    assert_eq!(half_size_luma_linear(&[0, 1, 0, 0], Size::new(2, 2)), [0]);
}
