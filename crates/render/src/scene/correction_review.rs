//! One bounded quality experiment, not a selected player path.
//!
//! Keep the exact direct high-resolution viewport and add the half-linear
//! panorama's temporal RGB residual. Both residual terms pass through the same
//! RGB/NV12/RGB conversion, so a radius-zero source has no correction. Motion
//! blocks keep their pixel dimensions, hence change angular support. This is
//! explicitly a new algorithm, not an equivalent lower-cost implementation.
//!
//! The full-resolution selected Stream runs alongside it on the very same
//! source/map/color transaction. Readbacks, diagnostic map uploads and retained
//! viewport images make this a quality test, never a playback timing benchmark.

use std::collections::VecDeque;
use std::io::Write;

use super::panorama_review::PendingReadback;
use super::*;
use crate::direct_type2::panorama::correction_review::CorrectionReview as Corrector;
use crate::temporal_fusion::color::{GpuColorConversion, MatrixCoefficients};
use crate::temporal_fusion::settings::Provider;
use crate::temporal_fusion::stream::{FilteredPanorama, Stream};

const VIEW: Size = Size {
    width: 1280,
    height: 720,
};

struct SourceView {
    stamp: FrameStamp,
    reframe: Reframe,
    direct: wgpu::Texture,
    low_current: wgpu::Texture,
}

pub(super) struct CorrectionReview {
    projector: PanoramaProjector,
    corrector: Corrector,
    color: GpuColorConversion,
    draw: Option<DirectMapDraw>,
    providers: Option<[Provider; 2]>,
    full: Option<Stream>,
    low: Option<Stream>,
    retained: VecDeque<SourceView>,
    previous: Option<FrameStamp>,
    inputs: usize,
    outputs: usize,
    output: PathBuf,
    log: std::io::BufWriter<std::fs::File>,
}

impl CorrectionReview {
    pub(super) fn new(
        device: &wgpu::Device,
        output: &Path,
        calibration: &kjerag_meta::CalibrationSet,
        source_fps: f32,
    ) -> Self {
        for arm in ["full-filtered", "half-correction", "half-filtered"] {
            std::fs::create_dir(output.join(arm)).unwrap();
        }
        let mut log = std::io::BufWriter::new(
            std::fs::File::create_new(output.join("correction-sources.tsv")).unwrap(),
        );
        writeln!(
            log,
            "source\ttime_ns\tfull_width\tfull_height\tlow_width\tlow_height"
        )
        .unwrap();
        Self {
            projector: PanoramaProjector::new(device),
            corrector: Corrector::new(device),
            color: GpuColorConversion::new(device),
            draw: None,
            providers: Some([
                Provider::new(calibration, source_fps).unwrap(),
                Provider::new(calibration, source_fps).unwrap(),
            ]),
            full: None,
            low: None,
            retained: VecDeque::new(),
            previous: None,
            inputs: 0,
            outputs: 0,
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
        let stamp = prepared.frame().clone();
        if let Some(previous) = &self.previous {
            assert!(stamp.same_decode_epoch(previous));
            assert_eq!(stamp.index(), previous.index() + 1);
        }
        let reframe = prepared.reframe();
        assert!(!reframe.linearizes_output());
        let source = reframe.frame_size();
        let full = Size::new(source[0] as u32 * 2, source[1] as u32);
        assert!(matches!(
            (full.width, full.height),
            (7680, 3840) | (5760, 2880)
        ));
        let low = Size::new(full.width / 2, full.height / 2);
        if let Some([full_provider, low_provider]) = self.providers.take() {
            self.full =
                Some(Stream::new(device, queue, [full.width, full.height], full_provider).unwrap());
            self.low = Some(
                Stream::new_half_resolution_review(
                    device,
                    queue,
                    [low.width, low.height],
                    low_provider,
                )
                .unwrap(),
            );
        }
        assert_eq!(ordinary_rgba.len(), (VIEW.width * VIEW.height * 4) as usize);
        let direct = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("exact source-stamped direct viewport quality control"),
            size: VIEW.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        queue.write_texture(
            direct.as_image_copy(),
            ordinary_rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(VIEW.width * 4),
                rows_per_image: Some(VIEW.height),
            },
            VIEW.extent(),
        );
        let draw = self.draw.get_or_insert_with(|| {
            DirectMapDraw::new_panorama(device, &pipeline.layout, map.fusion().is_some()).unwrap()
        });
        draw.upload(queue, map);
        let mut encoder = device.create_command_encoder(&Default::default());
        let low_body = draw
            .encode_panorama(device, &mut encoder, &pipeline.bind_group, &prepared, low)
            .unwrap();
        assert_eq!(low_body.frame(), &stamp);
        let matrix = MatrixCoefficients::from_source_rgb(reframe.source_color_matrix());
        let low_nv12 = self
            .color
            .encode_rgb_to_nv12(&mut encoder, low_body.texture(), matrix)
            .unwrap();
        let low_current = self
            .color
            .encode_nv12_to_rgb(&mut encoder, &low_nv12, matrix)
            .unwrap();
        let compact = draw
            .prepare_vertex_cached_compact_nv12(device)
            .unwrap()
            .encode(device, &mut encoder, &pipeline.bind_group, &prepared, full)
            .unwrap();
        assert_eq!(compact.frame(), &stamp);
        queue.submit([encoder.finish()]);
        self.retained.push_back(SourceView {
            stamp: stamp.clone(),
            reframe,
            direct,
            low_current,
        });
        assert!(self.retained.len() <= 7);
        self.inputs += 1;
        let full_outputs = self.full.as_mut().unwrap().push(compact).unwrap();
        let low_outputs = self
            .low
            .as_mut()
            .unwrap()
            .push_rgb(low_body, matrix)
            .unwrap();
        self.publish(device, queue, full_outputs, low_outputs);
        std::fs::write(
            self.output
                .join(format!("frame-{:010}.reframe.bin", stamp.index())),
            reframe.bytes(),
        )
        .unwrap();
        writeln!(
            self.log,
            "{}\t{}\t{}\t{}\t{}\t{}",
            stamp.index(),
            stamp.timestamp().as_nanos(),
            full.width,
            full.height,
            low.width,
            low.height
        )
        .unwrap();
        self.log.flush().unwrap();
        self.previous = Some(stamp);
    }

    pub(super) fn finish(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let full = self.full.as_mut().unwrap().finish().unwrap();
        let low = self.low.as_mut().unwrap().finish().unwrap();
        self.publish(device, queue, full, low);
        assert!(self.retained.is_empty());
        assert_eq!(self.inputs, self.outputs);
    }

    fn publish(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        full: Vec<FilteredPanorama>,
        low: Vec<FilteredPanorama>,
    ) {
        assert_eq!(
            full.len(),
            low.len(),
            "correction candidate changed output cadence"
        );
        for (full, low) in full.into_iter().zip(low) {
            let source = self.retained.pop_front().unwrap();
            assert_eq!(full.frame(), &source.stamp);
            assert_eq!(low.frame(), &source.stamp);
            assert!(full.belongs_to(device) && low.belongs_to(device));
            let mut encoder = device.create_command_encoder(&Default::default());
            let reference = self
                .projector
                .encode(device, &mut encoder, full.texture(), &source.reframe, VIEW)
                .unwrap();
            let corrected = self
                .corrector
                .encode(
                    device,
                    &mut encoder,
                    &source.direct,
                    &source.low_current,
                    low.texture(),
                    &source.reframe,
                )
                .unwrap();
            let low_view = self
                .projector
                .encode(device, &mut encoder, low.texture(), &source.reframe, VIEW)
                .unwrap();
            let reads = [
                (
                    "full-filtered",
                    PendingReadback::encode(device, &mut encoder, &reference),
                ),
                (
                    "half-correction",
                    PendingReadback::encode(device, &mut encoder, &corrected),
                ),
                (
                    "half-filtered",
                    PendingReadback::encode(device, &mut encoder, &low_view),
                ),
            ];
            let submission = queue.submit([encoder.finish()]);
            for (arm, read) in reads {
                let pixels = read.read(device, submission.clone());
                super::tests::write_review_ppm_sized(
                    &self.output.join(arm),
                    source.stamp.index(),
                    VIEW.width,
                    VIEW.height,
                    &pixels,
                );
            }
            self.outputs += 1;
            eprintln!(
                "correction-review: source {} full/half histories agree; explicit RGB residual candidate, not a parity or performance pass",
                source.stamp.index()
            );
        }
    }
}
