//! Offline seven-source temporal diagnostic with native startup/tail selection.
//!
//! This is deliberately CPU motion search with explicit GPU waits. It is not
//! playback policy or a performance path. The caller supplies already-associated
//! NV12 and gray levels, and either the captured regime or source-track settings.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use sha2::{Digest, Sha256};

use super::PendingReadback;
use crate::direct_type2::panorama::PanoramaProjector;
use crate::studio_type2::PreparedPicture;
use crate::temporal_fusion::GpuFuse;
use crate::temporal_fusion::color::{GpuColorConversion, MatrixCoefficients, Nv12};
use crate::temporal_fusion::history::History;
use crate::temporal_fusion::motion::{self, Geometry};
use crate::temporal_fusion::parallel_refine;
use crate::temporal_fusion::pyramid::{Level, gpu::PackedGray};
use crate::temporal_fusion::settings::{EffParams, FusionParams, Provider};
use crate::{FrameStamp, Reframe, Size};

mod profile;

const FULL: [u32; 2] = [7680, 3840];
const RAW_GRID: [u32; 2] = [240, 120];
const FLOW_GRID: [u32; 2] = [480, 240];
const VIEW: Size = Size {
    width: 1280,
    height: 720,
};
const CENTER: usize = 3;

struct Retained {
    stamp: FrameStamp,
    levels: Vec<Level>,
    gpu_base: Option<PackedGray>,
    reframe: Reframe,
}

struct PendingHistory {
    encoder: wgpu::CommandEncoder,
    profile: profile::Profile,
    host_ms: f64,
    copied_sources: u32,
}

impl PendingHistory {
    fn new(device: &wgpu::Device) -> Self {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("resident seven-source temporal diagnostic"),
        });
        let profile = profile::Profile::begin(device, &mut encoder);
        Self {
            encoder,
            profile,
            host_ms: 0.0,
            copied_sources: 0,
        }
    }
}

pub(super) struct TemporalReview {
    device: wgpu::Device,
    window: VecDeque<Retained>,
    history: History,
    pending_history: Option<PendingHistory>,
    next_center: usize,
    finished: bool,
    settings: Option<Provider>,
    fuse: GpuFuse,
    color: GpuColorConversion,
    projector: PanoramaProjector,
    gpu_motion: Option<motion::gpu::Builder>,
    parallel_refine: Option<parallel_refine::gpu::Builder>,
    parallel_search: bool,
    view_scissors: bool,
    output: PathBuf,
    log: std::io::BufWriter<std::fs::File>,
}

impl TemporalReview {
    pub(super) fn new(
        device: &wgpu::Device,
        output: &Path,
        gpu_motion: bool,
        parallel_search: bool,
        parallel_refine: bool,
        view_scissors: bool,
        settings: Option<Provider>,
    ) -> Self {
        let frames = output.join("panorama-denoised-diagnostic");
        std::fs::create_dir(&frames).unwrap_or_else(|error| {
            panic!("create {}: {error}", frames.display());
        });
        let log_path = output.join("panorama-denoised-diagnostic.csv");
        let mut log = std::io::BufWriter::new(
            std::fs::File::create_new(&log_path)
                .unwrap_or_else(|error| panic!("create {}: {error}", log_path.display())),
        );
        writeln!(
            log,
            "center_source,center_time_ns,reference_sources,reference_times_ns,search_ms,pack_ms,gpu_ms,rgba_sha256,iso,radius,reference_phases,copied_sources"
        )
        .unwrap();
        let contract = output.join("panorama-denoised-diagnostic.txt");
        let coverage_route = if view_scissors {
            "Coverage route: conservative per-view RGB scissors and separately padded fusion scissors, on full-sized targets.\n\
             The unchanged projector may only draw this exact prepared view; pixels outside the scissors are not filtered image data.\n\
             Region planning precedes the timed GPU section; full unfiltered panorama history and motion search remain unchanged.\n"
        } else {
            "Coverage route: full-panorama fusion and RGB conversion.\n"
        };
        let search_route = if parallel_refine && parallel_search {
            "Search route: one scoped CPU worker per reference runs search::prepare_finest through levels6..1, preserving supplied order.\n"
        } else if parallel_refine {
            "Search route: serial CPU search::prepare_finest through levels6..1 for each supplied reference.\n"
        } else if parallel_search {
            "Search route: one scoped CPU search::selected worker per reference, preserving supplied order.\n"
        } else {
            "Search route: serial CPU search::selected for each supplied reference.\n"
        };
        let motion_route = if parallel_refine {
            "Motion route: Kjerag-specific parallel GPU finest refinement and direct GPU motion packing.\n\
             This changes the spatial motion-search algorithm: finest blocks use immutable coarse neighbors and no UMH. It is not Studio-exact.\n\
             search_ms is CPU coarse preparation only; pack_ms is zero; GPU+wait includes finest refinement, direct packing/copy, fusion, conversion, projection and readback completion.\n"
        } else if gpu_motion {
            "Motion route: GPU render-pass packing from explicit raw/luma CPU uploads.\n\
             pack_ms is zero because there is no CPU packing stage; GPU+wait includes motion encode/upload/copy, fusion, conversion, projection and readback completion.\n"
        } else {
            "Motion route: CPU motion::pack_motion, then explicit packed-flow upload.\n\
             pack_ms measures that CPU stage; GPU+wait begins with GPU texture preparation/upload and includes fusion, conversion, projection and readback completion.\n"
        };
        let settings_route = if settings.is_some() {
            "Settings: authenticated camera/source selection and source-time ISO lookup."
        } else {
            "Settings: explicit captured ISO100/noise700/radius3/limit10 regime."
        };
        std::fs::write(
            &contract,
            format!(
                "Experimental offline temporal diagnostic. {settings_route}\n\
             Seven real same-epoch sources before any output; startup centers0..3, steady3, flush4..6.\n\
             References are the radius-clipped interval excluding current, in ascending source order.\n\
             Radius0 copies current after the same gate. Fewer than7 inputs yield no filter outputs.\n\
             No padding or gradual color update; no claim about Studio's higher exporter short-seek policy.\n\
             Resident seven-layer NV12 history; copies only arriving sources, with unchanged logical reference order.\n\
             History copies precede fusion in the same encoder; GPU+wait also includes their host encoding time.\n\
             CPU preparation and explicit GPU completion are diagnostic, not playback performance.\n\
             {search_route}{motion_route}{coverage_route}"
            ),
        )
        .unwrap_or_else(|error| panic!("write {}: {error}", contract.display()));
        Self {
            device: device.clone(),
            window: VecDeque::with_capacity(7),
            history: History::new(device, FULL)
                .unwrap_or_else(|error| panic!("create resident temporal history: {error}")),
            pending_history: None,
            next_center: 0,
            finished: false,
            settings,
            fuse: GpuFuse::new(device),
            color: GpuColorConversion::new(device),
            projector: PanoramaProjector::new(device),
            gpu_motion: gpu_motion.then(|| motion::gpu::Builder::new(device)),
            parallel_refine: parallel_refine.then(|| parallel_refine::gpu::Builder::new(device)),
            parallel_search,
            view_scissors,
            output: frames,
            log,
        }
    }

    pub(super) fn push(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        prepared: &PreparedPicture,
        nv12: Nv12,
        levels: Vec<Level>,
        gpu_base: Option<PackedGray>,
    ) {
        assert!(
            !self.finished,
            "temporal review received a source after flush"
        );
        assert!(
            self.device == *device,
            "temporal review changed graphics devices"
        );
        validate_input(
            prepared,
            &nv12,
            &levels,
            gpu_base.as_ref(),
            self.parallel_refine.is_some(),
        );
        if let Some(previous) = self.window.back() {
            assert!(
                previous.stamp.same_decode_epoch(prepared.frame()),
                "temporal review crossed a decode epoch"
            );
            assert_eq!(
                previous.stamp.index().checked_add(1),
                Some(prepared.frame().index()),
                "temporal review sources are not contiguous"
            );
        }
        if self.window.len() == 7 {
            assert_eq!(self.next_center, CENTER + 1, "undrained temporal outputs");
            self.window.pop_front().unwrap();
            self.next_center -= 1;
        }
        let history_started = Instant::now();
        let pending = self
            .pending_history
            .get_or_insert_with(|| PendingHistory::new(device));
        self.history
            .encode_push(device, &mut pending.encoder, prepared.frame(), &nv12)
            .unwrap_or_else(|error| panic!("retain temporal source: {error}"));
        pending.host_ms += history_started.elapsed().as_secs_f64() * 1000.0;
        pending.copied_sources += 1;
        // The encoder retains the NV12 copy sources. Stamped gray levels and
        // view metadata stay here; completed NV12 copies live in shared arrays.
        self.window.push_back(Retained {
            stamp: prepared.frame().clone(),
            levels,
            gpu_base,
            reframe: prepared.reframe(),
        });
        if self.window.len() < 7 {
            return;
        }
        assert_eq!(
            self.window.len(),
            7,
            "temporal review retained too many sources"
        );
        while self.next_center <= CENTER {
            self.process(device, queue);
            self.next_center += 1;
        }
    }

    pub(super) fn finish(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.finished {
            return;
        }
        if self.window.len() == 7 {
            while self.next_center < self.window.len() {
                self.process(device, queue);
                self.next_center += 1;
            }
        } else {
            // Native NAP checks the seven-real-source gate before its flush flag.
            // Do not invent repeated images or an early pass-through here.
            self.pending_history.take();
        }
        self.finished = true;
        self.log.flush().unwrap();
    }

    fn process(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let PendingHistory {
            mut encoder,
            mut profile,
            host_ms: history_host_ms,
            copied_sources,
        } = self
            .pending_history
            .take()
            .unwrap_or_else(|| PendingHistory::new(device));
        profile.mark(&mut encoder, "history");
        let center = &self.window[self.next_center];
        let effective = self
            .settings
            .as_mut()
            .map_or_else(captured_settings, |provider| {
                provider
                    .parameters_at(center.stamp.timestamp().as_secs_f64() * 1000.0)
                    .unwrap_or_else(|error| panic!("temporal source settings: {error}"))
            });
        let history = self
            .history
            .window_at(self.next_center, effective.radius as usize)
            .expect("seven resident temporal sources");
        let references: Vec<_> = history.reference_positions().collect();
        let phases: Vec<_> = history.reference_phases().collect();
        let stamps = history.stamps();
        assert_eq!(stamps.center, &center.stamp);
        for (stamp, &index) in stamps.references.into_iter().zip(&references) {
            assert_eq!(stamp, &self.window[index].stamp);
        }
        let regions = self.view_scissors.then(|| {
            crate::temporal_fusion::regions::ViewRegions::for_view(FULL, &center.reframe)
                .unwrap_or_else(|error| panic!("plan temporal view scissors: {error}"))
        });
        let (fused, search_ms, pack_ms, gpu_started) = if references.is_empty() {
            let gpu_started = Instant::now();
            profile.mark(&mut encoder, "refine");
            profile.mark(&mut encoder, "motion");
            let copied = history
                .encode_copy_current(device, &mut encoder)
                .unwrap_or_else(|error| panic!("copy radius-zero temporal current: {error}"));
            (copied, 0.0, 0.0, gpu_started)
        } else {
            let luma = &center.levels[3].pixels;
            let search_started = Instant::now();
            let reference_levels: Vec<_> = references
                .iter()
                .map(|&at| self.window[at].levels.as_slice())
                .collect();
            let (raw, finest) = if self.parallel_refine.is_some() {
                let inputs = search_references(
                    &center.levels,
                    &reference_levels,
                    self.parallel_search,
                    crate::temporal_fusion::search::prepare_finest,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "coarse motion preparation for center {}: {error}",
                        center.stamp.index()
                    )
                });
                (None, Some(inputs))
            } else {
                let raw = search_references(
                    &center.levels,
                    &reference_levels,
                    self.parallel_search,
                    crate::temporal_fusion::search::selected,
                )
                .unwrap_or_else(|error| {
                    panic!("motion search for center {}: {error}", center.stamp.index(),)
                });
                (Some(raw), None)
            };
            let search_ms = search_started.elapsed().as_secs_f64() * 1000.0;

            let geometry = Geometry {
                full: FULL,
                raw_grid: RAW_GRID,
                output_grid: FLOW_GRID,
                block: [16, 16],
            };
            let motion_parameters = |phase| motion::Parameters {
                geometry,
                luma,
                confidence_y: &effective.confidence_y,
                confidence_uv: &effective.confidence_uv,
                scale_base: 4,
                scale_extra: effective.noise_integer,
                temporal: 1.25,
                phase,
            };
            let (packed, pack_ms) = if self.gpu_motion.is_none() {
                let pack_started = Instant::now();
                let packed: Vec<Vec<[i16; 4]>> = raw
                    .as_ref()
                    .expect("CPU packing has complete CPU search records")
                    .iter()
                    .zip(phases.iter().copied())
                    .map(|(raw, phase)| {
                        motion::pack_motion(raw, &motion_parameters(phase))
                            .unwrap_or_else(|error| panic!("pack captured-regime motion: {error}"))
                    })
                    .collect();
                (Some(packed), pack_started.elapsed().as_secs_f64() * 1000.0)
            } else {
                (None, 0.0)
            };

            let gpu_started = Instant::now();
            if let Some(regions) = &regions {
                eprintln!(
                    "panorama-temporal-regions: center {} RGB {:?} fusion {:?}",
                    center.stamp.index(),
                    regions.rgb(),
                    regions.fusion(),
                );
            }
            let flow = array_texture(
                device,
                "offline temporal packed motion",
                FLOW_GRID,
                references.len() as u32,
                wgpu::TextureFormat::Rgba16Sint,
            );
            let luma_texture = array_texture(
                device,
                "offline temporal luma indices",
                FLOW_GRID,
                1,
                wgpu::TextureFormat::R8Uint,
            );
            if let Some(packed) = &packed {
                for (layer, bytes) in packed
                    .iter()
                    .map(|records| packed_bytes(records))
                    .enumerate()
                {
                    write_layer(queue, &flow, layer as u32, FLOW_GRID, 8, &bytes);
                }
            }
            write_layer(queue, &luma_texture, 0, FLOW_GRID, 1, luma);

            if self.parallel_refine.is_none() {
                profile.mark(&mut encoder, "refine");
            }
            if let Some(refiner) = &self.parallel_refine {
                let inputs = finest
                    .as_ref()
                    .expect("parallel refinement has coarse CPU inputs");
                let current = center
                    .gpu_base
                    .as_ref()
                    .expect("parallel refinement retains the current GPU base");
                let references: Vec<_> = references
                    .iter()
                    .map(|&at| {
                        self.window[at]
                            .gpu_base
                            .as_ref()
                            .expect("parallel refinement retains every reference GPU base")
                    })
                    .collect();
                let seeds: Vec<_> = inputs.iter().map(|input| input.seeds.as_slice()).collect();
                let globals: Vec<_> = inputs.iter().map(|input| input.global).collect();
                let refined = refiner
                    .encode_finest(device, &mut encoder, current, &references, &seeds, &globals)
                    .unwrap_or_else(|error| {
                        panic!("parallel finest refinement for captured-regime motion: {error}")
                    });
                profile.mark(&mut encoder, "refine");
                let builder = self
                    .gpu_motion
                    .as_ref()
                    .expect("parallel refinement requires GPU motion packing");
                for (layer, phase) in phases.iter().copied().enumerate() {
                    let output = builder
                        .encode_refined(
                            device,
                            &mut encoder,
                            &refined,
                            layer as u32,
                            &motion_parameters(phase),
                        )
                        .unwrap_or_else(|error| {
                            panic!("GPU-pack parallel-refined captured-regime motion: {error}")
                        });
                    copy_layer(&mut encoder, &output, &flow, layer as u32);
                }
            } else if let Some(builder) = &self.gpu_motion {
                for (layer, (raw, phase)) in raw
                    .as_ref()
                    .expect("GPU packing has complete CPU search records")
                    .iter()
                    .zip(phases.iter().copied())
                    .enumerate()
                {
                    let output = builder
                        .encode(device, &mut encoder, raw, &motion_parameters(phase))
                        .unwrap_or_else(|error| panic!("GPU-pack captured-regime motion: {error}"));
                    // The recorded copy retains its source texture until completion.
                    copy_layer(&mut encoder, &output, &flow, layer as u32);
                }
            }
            profile.mark(&mut encoder, "motion");
            let inputs = history.inputs(&flow, &luma_texture);
            let parameters = history.parameters(
                effective.fusion.noise,
                effective.fusion.limit,
                effective.fusion.y_limits,
                effective.fusion.uv_limits,
            );
            let fused = if let Some(regions) = &regions {
                self.fuse.encode_scissored(
                    device,
                    &mut encoder,
                    inputs,
                    &parameters,
                    regions.fusion(),
                )
            } else {
                self.fuse.encode(
                    device,
                    &mut encoder,
                    inputs,
                    &parameters,
                    [0, 0, FULL[0], FULL[1]],
                )
            }
            .unwrap_or_else(|error| panic!("fuse temporal diagnostic: {error}"));
            (fused, search_ms, pack_ms, gpu_started)
        };
        profile.mark(&mut encoder, "fuse");
        let matrix = MatrixCoefficients::from_source_rgb(center.reframe.source_color_matrix());
        let rgb = if let Some(regions) = &regions {
            self.color.encode_planes_to_rgb_scissored(
                &mut encoder,
                &fused.y,
                &fused.uv,
                matrix,
                regions.rgb(),
            )
        } else {
            self.color
                .encode_planes_to_rgb(&mut encoder, &fused.y, &fused.uv, matrix)
                .map_err(|error| error.to_string())
        }
        .unwrap_or_else(|error| panic!("convert fused NV12 diagnostic: {error}"));
        profile.mark(&mut encoder, "color");
        let projected = self
            .projector
            .encode(device, &mut encoder, &rgb, &center.reframe, VIEW)
            .unwrap_or_else(|error| panic!("project fused panorama diagnostic: {error}"));
        profile.mark(&mut encoder, "project");
        let readback = PendingReadback::encode(device, &mut encoder, &projected);
        profile.mark(&mut encoder, "readback");
        profile.resolve(&mut encoder);
        let submission = queue.submit([encoder.finish()]);
        let rgba = readback.read(device, submission);
        let gpu_ms = gpu_started.elapsed().as_secs_f64() * 1000.0 + history_host_ms;
        profile.report(device, queue, center.stamp.index());
        eprintln!(
            "panorama-temporal-history: center {} copied_sources {copied_sources} logical_bytes {} host_encode {history_host_ms:.3}ms",
            center.stamp.index(),
            u64::from(copied_sources) * u64::from(FULL[0]) * u64::from(FULL[1]) * 3 / 2,
        );

        super::super::tests::write_review_ppm_sized(
            &self.output,
            center.stamp.index(),
            VIEW.width,
            VIEW.height,
            &rgba,
        );
        let digest: String = Sha256::digest(&rgba)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let reference_sources = references
            .iter()
            .map(|&at| self.window[at].stamp.index().to_string())
            .collect::<Vec<_>>()
            .join("|");
        let reference_times = references
            .iter()
            .map(|&at| self.window[at].stamp.timestamp().as_nanos().to_string())
            .collect::<Vec<_>>()
            .join("|");
        let reference_phases = phases
            .iter()
            .map(|phase| format!("{:016x}", phase.to_bits()))
            .collect::<Vec<_>>()
            .join("|");
        writeln!(
            self.log,
            "{},{},{},{},{search_ms:.6},{pack_ms:.6},{gpu_ms:.6},{},{},{},{},{copied_sources}",
            center.stamp.index(),
            center.stamp.timestamp().as_nanos(),
            reference_sources,
            reference_times,
            digest,
            effective.iso,
            effective.radius,
            reference_phases,
        )
        .unwrap();
        self.log.flush().unwrap();
        eprintln!(
            "panorama-temporal-diagnostic: center {} search {search_ms:.3}ms pack {pack_ms:.3}ms GPU+wait {gpu_ms:.3}ms {digest}",
            center.stamp.index()
        );
    }
}

fn validate_input(
    prepared: &PreparedPicture,
    nv12: &Nv12,
    levels: &[Level],
    gpu_base: Option<&PackedGray>,
    parallel_refine: bool,
) {
    assert!(
        !prepared.reframe().linearizes_output(),
        "temporal review needs gamma RGB preparation"
    );
    assert_eq!(nv12.size().width, FULL[0]);
    assert_eq!(nv12.size().height, FULL[1]);
    assert_eq!(
        levels.len(),
        7,
        "temporal review needs seven pyramid levels"
    );
    for (level, image) in levels.iter().enumerate() {
        let expected = [3840usize >> level, 1920usize >> level];
        assert_eq!(
            [image.width, image.height],
            expected,
            "pyramid level {level} geometry differs"
        );
        assert_eq!(
            image.pixels.len(),
            expected[0] * expected[1],
            "pyramid level {level} length differs"
        );
    }
    assert_eq!(
        gpu_base.is_some(),
        parallel_refine,
        "only parallel refinement retains the associated GPU pyramid base"
    );
    if let Some(base) = gpu_base {
        assert_eq!(
            base.logical_size(),
            [levels[0].width as u32, levels[0].height as u32]
        );
    }
}

fn search_references<T: Send>(
    current: &[Level],
    references: &[&[Level]],
    parallel: bool,
    search: fn(&[Level], &[Level]) -> Result<T, crate::temporal_fusion::search::Error>,
) -> Result<Vec<T>, crate::temporal_fusion::search::Error> {
    if !parallel {
        return references
            .iter()
            .map(|reference| search(current, reference))
            .collect();
    }
    std::thread::scope(|scope| {
        let jobs: Vec<_> = references
            .iter()
            .map(|reference| scope.spawn(move || search(current, reference)))
            .collect();
        jobs.into_iter()
            .map(|job| {
                job.join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    })
}

fn captured_settings() -> EffParams {
    // Exact selected UBO words from denoise-parameters-01/analysis.json.
    EffParams {
        iso: 100,
        radius: 3,
        noise_integer: 700,
        confidence_y: [1.0; 256],
        confidence_uv: [2.0; 256],
        fusion: FusionParams {
            noise: f32::from_bits(0x3d2f_afb0),
            limit: f32::from_bits(0x3d20_a0a1),
            y_limits: [1.0; 256],
            uv_limits: [0.5; 256],
        },
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

fn packed_bytes(records: &[[i16; 4]]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|record| record.iter().flat_map(|lane| lane.to_le_bytes()))
        .collect()
}
