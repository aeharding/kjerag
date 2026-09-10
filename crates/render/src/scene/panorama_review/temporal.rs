//! Offline seven-source temporal diagnostic for the captured ISO-100 regime.
//!
//! This is deliberately CPU motion search with explicit GPU waits. It is not
//! playback policy, a performance path, startup/end padding, or a general ISO
//! calibration. The caller supplies already-associated NV12 and gray levels.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use sha2::{Digest, Sha256};

use super::PendingReadback;
use crate::direct_type2::panorama::PanoramaProjector;
use crate::studio_type2::PreparedPicture;
use crate::temporal_fusion::color::{GpuColorConversion, MatrixCoefficients, Nv12};
use crate::temporal_fusion::motion::{self, Geometry};
use crate::temporal_fusion::parallel_refine;
use crate::temporal_fusion::pyramid::Level;
use crate::temporal_fusion::{GpuFuse, Inputs, Parameters};
use crate::{FrameStamp, Reframe, Size};

const FULL: [u32; 2] = [7680, 3840];
const RAW_GRID: [u32; 2] = [240, 120];
const FLOW_GRID: [u32; 2] = [480, 240];
const VIEW: Size = Size {
    width: 1280,
    height: 720,
};
const CENTER: usize = 3;
const REFERENCES: [usize; 6] = [0, 1, 2, 4, 5, 6];
const PHASES: [f64; 6] = [1.0, 0.49999999999999994, 0.0, 0.0, 0.49999999999999994, 1.0];
const CONFIDENCE_Y: [f32; 256] = [1.0; 256];
const CONFIDENCE_UV: [f32; 256] = [2.0; 256];
const LIMIT_Y: [f32; 256] = [1.0; 256];
const LIMIT_UV: [f32; 256] = [0.5; 256];

struct Retained {
    stamp: FrameStamp,
    nv12: Nv12,
    levels: Vec<Level>,
    gpu_base: Option<wgpu::Texture>,
    reframe: Reframe,
}

pub(super) struct TemporalReview {
    device: wgpu::Device,
    window: VecDeque<Retained>,
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
            "center_source,center_time_ns,reference_sources,reference_times_ns,search_ms,pack_ms,gpu_ms,rgba_sha256"
        )
        .unwrap();
        let contract = output.join("panorama-denoised-diagnostic.txt");
        let coverage_route = if view_scissors {
            "Coverage route: conservative per-view RGB scissors and separately padded fusion scissors, on full-sized targets.\n\
             The unchanged projector may only draw this exact prepared view; pixels outside the scissors are not filtered image data.\n\
             GPU+wait includes region planning; full unfiltered panorama history and motion search remain unchanged.\n"
        } else {
            "Coverage route: full-panorama fusion and RGB conversion.\n"
        };
        let search_route = if parallel_refine && parallel_search {
            "Search route: six scoped CPU workers run search::prepare_finest through levels6..1, preserving supplied reference order.\n"
        } else if parallel_refine {
            "Search route: serial CPU search::prepare_finest through levels6..1 for each supplied reference.\n"
        } else if parallel_search {
            "Search route: CPU search::selected_six, six workers preserving supplied reference order.\n"
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
        std::fs::write(
            &contract,
            format!(
                "Experimental offline ISO100 regime only.\n\
             Seven contiguous same-epoch sources; center is position3.\n\
             References are c-3,c-2,c-1,c+1,c+2,c+3.\n\
             No startup/end padding, automatic policy, or gradual color update.\n\
             CPU preparation and explicit GPU completion are diagnostic, not playback performance.\n\
             {search_route}{motion_route}{coverage_route}"
            ),
        )
        .unwrap_or_else(|error| panic!("write {}: {error}", contract.display()));
        Self {
            device: device.clone(),
            window: VecDeque::with_capacity(7),
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
        gpu_base: Option<wgpu::Texture>,
    ) {
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
        self.window.push_back(Retained {
            stamp: prepared.frame().clone(),
            nv12,
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
        self.process(device, queue);
        self.window
            .pop_front()
            .expect("seven-frame window has a front");
    }

    fn process(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let center = &self.window[CENTER];
        let luma = &center.levels[3].pixels;
        let search_started = Instant::now();
        let reference_levels = REFERENCES.map(|at| self.window[at].levels.as_slice());
        let (raw, finest) = if self.parallel_refine.is_some() {
            let inputs: [crate::temporal_fusion::search::FinestInput; 6] =
                if self.parallel_search {
                    prepare_finest_six(&center.levels, reference_levels)
                } else {
                    reference_levels
                        .map(|reference| {
                            crate::temporal_fusion::search::prepare_finest(
                                &center.levels,
                                reference,
                            )
                        })
                        .into_iter()
                        .collect::<Result<Vec<_>, _>>()
                        .map(|inputs| {
                            inputs.try_into().unwrap_or_else(|_| {
                                panic!("six references produce six finest inputs")
                            })
                        })
                }
                .unwrap_or_else(|error| {
                    panic!(
                        "coarse motion preparation for center {}: {error}",
                        center.stamp.index()
                    )
                });
            (None, Some(inputs))
        } else if self.parallel_search {
            let raw =
                crate::temporal_fusion::search::selected_six(&center.levels, reference_levels)
                    .unwrap_or_else(|error| {
                        panic!(
                            "parallel motion search for center {}: {error}",
                            center.stamp.index()
                        )
                    });
            (Some(raw.into_iter().collect::<Vec<_>>()), None)
        } else {
            let raw = REFERENCES
                .iter()
                .map(|&at| {
                    crate::temporal_fusion::search::selected(
                        &center.levels,
                        &self.window[at].levels,
                    )
                    .unwrap_or_else(|error| {
                        panic!(
                            "motion search for center {} reference {}: {error}",
                            center.stamp.index(),
                            self.window[at].stamp.index()
                        )
                    })
                })
                .collect();
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
            confidence_y: &CONFIDENCE_Y,
            confidence_uv: &CONFIDENCE_UV,
            scale_base: 4,
            scale_extra: 700,
            temporal: 1.25,
            phase,
        };
        let (packed, pack_ms) = if self.gpu_motion.is_none() {
            let pack_started = Instant::now();
            let packed: Vec<Vec<[i16; 4]>> = raw
                .as_ref()
                .expect("CPU packing has complete CPU search records")
                .iter()
                .zip(PHASES)
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
        let regions = self.view_scissors.then(|| {
            crate::temporal_fusion::regions::ViewRegions::for_view(FULL, &center.reframe)
                .unwrap_or_else(|error| panic!("plan temporal view scissors: {error}"))
        });
        if let Some(regions) = &regions {
            eprintln!(
                "panorama-temporal-regions: center {} RGB {:?} fusion {:?}",
                center.stamp.index(),
                regions.rgb(),
                regions.fusion(),
            );
        }
        let y = array_texture(
            device,
            "offline temporal Y window",
            FULL,
            7,
            wgpu::TextureFormat::R8Unorm,
        );
        let uv = array_texture(
            device,
            "offline temporal UV window",
            [FULL[0] / 2, FULL[1] / 2],
            7,
            wgpu::TextureFormat::Rg8Unorm,
        );
        let flow = array_texture(
            device,
            "offline temporal packed motion",
            FLOW_GRID,
            6,
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

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("offline seven-source temporal diagnostic"),
        });
        for (layer, record) in self.window.iter().enumerate() {
            copy_layer(&mut encoder, &record.nv12.y, &y, layer as u32);
            copy_layer(&mut encoder, &record.nv12.uv, &uv, layer as u32);
        }
        if let Some(refiner) = &self.parallel_refine {
            let inputs = finest
                .as_ref()
                .expect("parallel refinement has coarse CPU inputs");
            let current = center
                .gpu_base
                .as_ref()
                .expect("parallel refinement retains the current GPU base");
            let references = REFERENCES.map(|at| {
                self.window[at]
                    .gpu_base
                    .as_ref()
                    .expect("parallel refinement retains every reference GPU base")
            });
            let refined = refiner
                .encode_finest(
                    device,
                    &mut encoder,
                    current,
                    references,
                    inputs.each_ref().map(|input| input.seeds.as_slice()),
                    inputs.each_ref().map(|input| input.global),
                )
                .unwrap_or_else(|error| {
                    panic!("parallel finest refinement for captured-regime motion: {error}")
                });
            let builder = self
                .gpu_motion
                .as_ref()
                .expect("parallel refinement requires GPU motion packing");
            for (layer, phase) in PHASES.into_iter().enumerate() {
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
                .zip(PHASES)
                .enumerate()
            {
                let output = builder
                    .encode(device, &mut encoder, raw, &motion_parameters(phase))
                    .unwrap_or_else(|error| panic!("GPU-pack captured-regime motion: {error}"));
                // The recorded copy retains its source texture until completion.
                copy_layer(&mut encoder, &output, &flow, layer as u32);
            }
        }
        let inputs = Inputs {
            y: &y,
            uv: &uv,
            flow: &flow,
            luma: &luma_texture,
        };
        let parameters = Parameters {
            // Exact f32 words consumed by every selected Y/UV UBO in
            // `denoise-parameters-01/analysis.json`: noise integer
            // 700 scaled by 1/16320, and limit integer 10 by 1/255.
            // The authenticated analysis SHA-256 is
            // f319e651c5077b0e952f62138bbbde0ca8bd6a46e76093fd31fa86fcb823f3aa.
            noise: f32::from_bits(0x3d2f_afb0),
            limit: f32::from_bits(0x3d20_a0a1),
            y_limits: LIMIT_Y,
            uv_limits: LIMIT_UV,
            current_layer: CENTER as u32,
            reference_layers: REFERENCES.map(|value| value as u32).to_vec(),
        };
        let fused = if let Some(regions) = &regions {
            self.fuse
                .encode_scissored(device, &mut encoder, inputs, &parameters, regions.fusion())
        } else {
            self.fuse.encode(
                device,
                &mut encoder,
                inputs,
                &parameters,
                [0, 0, FULL[0], FULL[1]],
            )
        }
        .unwrap_or_else(|error| panic!("fuse captured ISO100 regime: {error}"));
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
        let projected = self
            .projector
            .encode(device, &mut encoder, &rgb, &center.reframe, VIEW)
            .unwrap_or_else(|error| panic!("project fused panorama diagnostic: {error}"));
        let readback = PendingReadback::encode(device, &mut encoder, &projected);
        let submission = queue.submit([encoder.finish()]);
        let rgba = readback.read(device, submission);
        let gpu_ms = gpu_started.elapsed().as_secs_f64() * 1000.0;

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
        let reference_sources = REFERENCES
            .map(|at| self.window[at].stamp.index().to_string())
            .join("|");
        let reference_times = REFERENCES
            .map(|at| self.window[at].stamp.timestamp().as_nanos().to_string())
            .join("|");
        writeln!(
            self.log,
            "{},{},{},{},{search_ms:.6},{pack_ms:.6},{gpu_ms:.6},{}",
            center.stamp.index(),
            center.stamp.timestamp().as_nanos(),
            reference_sources,
            reference_times,
            digest,
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
    gpu_base: Option<&wgpu::Texture>,
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
        assert_eq!(base.format(), wgpu::TextureFormat::R8Uint);
        assert_eq!(base.dimension(), wgpu::TextureDimension::D2);
        assert_eq!(base.size().width, levels[0].width as u32);
        assert_eq!(base.size().height, levels[0].height as u32);
        assert_eq!(base.size().depth_or_array_layers, 1);
        assert_eq!(base.mip_level_count(), 1);
        assert_eq!(base.sample_count(), 1);
        assert!(base.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING));
    }
}

fn prepare_finest_six(
    current: &[Level],
    references: [&[Level]; 6],
) -> Result<[crate::temporal_fusion::search::FinestInput; 6], crate::temporal_fusion::search::Error>
{
    std::thread::scope(|scope| {
        let jobs = references.map(|reference| {
            scope.spawn(move || crate::temporal_fusion::search::prepare_finest(current, reference))
        });
        let results: Result<Vec<_>, _> = jobs
            .into_iter()
            .map(|job| {
                job.join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect();
        Ok(results?
            .try_into()
            .unwrap_or_else(|_| panic!("six workers produce six finest inputs")))
    })
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
