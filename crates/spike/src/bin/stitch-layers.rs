//! Same-source ablation of calibrated projection, parent maps and final maps.
//!
//! Diagnostic only: reads a map saved by the Scene sequence test. This does
//! not authenticate that payload as Studio output or reproduce live scheduling.
//! Every map arm uses the existing direct ray consumer, not the live mesh draw.
//! Compare `final.png` against the saved Scene picture before attributing a
//! visible difference to its map producer. Lens-only arms intentionally ignore
//! image-circle coverage outside the overlap; they are not playback candidates.
//!
//! Usage: stitch-layers INPUT FRAME YAW PITCH FOV SAVED_MAP_DIRECTORY OUTPUT_DIRECTORY
//! All views are locked, 1280x720; angles are degrees. Output must not exist.
//! Optional trailing LEFT_FUSION RIGHT_FUSION paths replay explicit 200x100
//! float4 ratio maps. They must name this same source and renderer chart;
//! loading them does not authenticate that association or run an estimator.
//! Instead, trailing `estimate-fusion` runs the explicit Windows selected-X4
//! CPU reference on this decoded source through this saved Kjerag map. This
//! is a diagnostic, not Studio source-sampler or other-camera equivalence.

use std::fs;
use std::path::Path;

use kjerag_media::{Cue, Fallible};
use kjerag_meta::{CalibrationSet, Filter};
use kjerag_render::flow::one_xs::ParentMapBuilder;
use kjerag_render::image_fusion::{RatioMap, RatioPair};
use kjerag_render::studio_type2::{AlphaMap, MAP_NODES, PackedMap};
use kjerag_render::{Camera, Horizon, OneXsMapFrame, PisBackend, Scene, ScenePipeline, Size};
use kjerag_spike::{Gpu, Offscreen, seam_trace::trace_alpha};
use sha2::{Digest, Sha256};

const SIZE: Size = Size {
    width: 1280,
    height: 720,
};
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn main() -> Fallible<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let estimate = args.len() == 8 && args[7] == "estimate-fusion";
    if args.len() != 7 && args.len() != 9 && !estimate {
        return Err(
            "usage: stitch-layers INPUT FRAME YAW PITCH FOV SAVED_MAP_DIRECTORY OUTPUT_DIRECTORY [LEFT_FUSION RIGHT_FUSION | estimate-fusion]"
                .into(),
        );
    }
    let input = Path::new(&args[0]);
    let frame = args[1].parse()?;
    let camera = Camera {
        yaw: args[2].parse::<f32>()?.to_radians(),
        pitch: args[3].parse::<f32>()?.to_radians(),
        fov: args[4].parse::<f32>()?.to_radians(),
    };
    if !camera.yaw.is_finite()
        || !camera.pitch.is_finite()
        || !camera.fov.is_finite()
        || camera.fov <= 0.0
    {
        return Err("stitch-layers needs finite view angles and a positive field of view".into());
    }
    let saved = Path::new(&args[5]);
    let out = Path::new(&args[6]);
    fs::create_dir(out)?;
    let prefix = format!("frame-{frame:010}");
    let packed = read_floats(
        &saved.join(format!("{prefix}.packed-f32le.bin")),
        MAP_NODES * 4,
    )?;
    let packed = PackedMap::new(
        packed
            .chunks_exact(4)
            .map(|v| v.try_into().unwrap())
            .collect(),
    )?;
    let alpha = AlphaMap::new(read_floats(
        &saved.join(format!("{prefix}.alpha-f32le.bin")),
        MAP_NODES,
    )?)?;
    let mut fusion = if args.len() == 9 {
        let read_ratio = |path: &str| -> Fallible<RatioMap> {
            Ok(RatioMap::new(
                read_floats(Path::new(path), MAP_NODES * 4)?
                    .chunks_exact(4)
                    .map(|v| v.try_into().unwrap())
                    .collect(),
            )?)
        };
        Some(RatioPair {
            left: read_ratio(&args[7])?,
            right: read_ratio(&args[8])?,
        })
    } else {
        None
    };
    let calibration = CalibrationSet::from_capture(input)?;
    let orientation = calibration.orientation(Filter::default());
    let gpu = Gpu::open()?;
    let scene = Scene::still(input, Cue::Index(frame))?;
    scene.set_horizon(Horizon::Locked);
    let mut pipeline = ScenePipeline::new(&gpu.device, &gpu.queue, FORMAT);
    let prepared = pipeline
        .prepare_one_xs_picture(
            &scene.primitive(camera),
            SIZE.width as f32 / SIZE.height as f32,
        )
        .ok_or("ordinary picture preparation produced no frame")?;
    if prepared.frame().index() != frame {
        return Err(format!(
            "requested frame {frame}, decoded {}",
            prepared.frame().index()
        )
        .into());
    }
    println!(
        "prepared: {:?}; view: {camera:?}; size: {SIZE:?}",
        prepared.frame()
    );
    let target = Offscreen::new(&gpu.device, SIZE, FORMAT);
    target.render(&gpu.device, &gpu.queue, &pipeline)?;
    target.write_png(
        &target.read(&gpu.device, &gpu.queue)?,
        &out.join("generic.png"),
    )?;

    let parent = ParentMapBuilder::new(&calibration)?.build_for_frame(
        &orientation,
        prepared.frame(),
        calibration.readout(),
    )?;
    // Same packing as map_patch::materialize, but with no flow displacement.
    let parent = PackedMap::new(
        parent
            .a
            .row_major_values()
            .iter()
            .zip(parent.b.row_major_values())
            .map(|(a, b)| [a[0] * 0.5, a[1], (b[0] + 1.0) * 0.5, b[1]])
            .collect(),
    )?;
    let final_map = OneXsMapFrame::new(
        prepared.frame().clone(),
        packed.clone(),
        alpha.clone(),
        PisBackend::Gpu,
    );
    if estimate {
        let pending = pipeline
            .prepare_one_xs_fusion_inputs(
                &scene.primitive(camera),
                SIZE.width as f32 / SIZE.height as f32,
                &final_map,
            )?
            .ok_or("fusion input preparation produced no frame")?;
        if pending.frame() != prepared.frame() {
            return Err("fusion input preparation changed the decoded frame".into());
        }
        let samples = pending.read()?;
        let mut reference = kjerag_render::image_fusion::spatial::Reference::new();
        let output = reference.observe(samples.images(), samples.invalid())?;
        for (name, bgr) in ["left", "right"].into_iter().zip(samples.images()) {
            fs::write(out.join(format!("fusion-input-{name}.bgr8")), bgr)?;
            write_bgr_png(bgr, &out.join(format!("fusion-input-{name}.png")))?;
        }
        fs::write(out.join("fusion-invalid.bin"), samples.invalid())?;
        for (name, map) in [
            ("left", &output.ratios.left),
            ("right", &output.ratios.right),
        ] {
            fs::write(out.join(format!("fusion-{name}.float4")), map.bytes())?;
        }
        println!(
            "fusion reference: frame {:?}, {:?}; saved Kjerag map association is caller-supplied, not a Studio oracle",
            samples.frame(),
            output.diagnostics
        );
        fusion = Some(output.ratios);
    }
    let dense = final_map.rasterize(&prepared, SIZE)?;
    for (name, geometry) in [("parent", parent), ("final", packed)] {
        for (suffix, weights) in [
            ("", alpha.clone()),
            ("-lens-a", AlphaMap::new(vec![1.0; MAP_NODES])?),
            ("-lens-b", AlphaMap::new(vec![0.0; MAP_NODES])?),
        ] {
            let map = OneXsMapFrame::new(
                prepared.frame().clone(),
                geometry.clone(),
                weights,
                PisBackend::Cpu,
            );
            target.render_map(&gpu.device, &gpu.queue, &mut pipeline, &map)?;
            let pixels = target.read(&gpu.device, &gpu.queue)?;
            target.write_png(&pixels, &out.join(format!("{name}{suffix}.png")))?;
            if suffix.is_empty() {
                target.write_png(
                    &trace_alpha(&pixels, dense.dense())?,
                    &out.join(format!("{name}-trace.png")),
                )?;
            }
            if name == "final"
                && let Some(fusion) = &fusion
            {
                let map = map.with_fusion(fusion.clone());
                target.render_map(&gpu.device, &gpu.queue, &mut pipeline, &map)?;
                let pixels = target.read(&gpu.device, &gpu.queue)?;
                target.write_png(&pixels, &out.join(format!("{name}{suffix}-fusion.png")))?;
                if suffix.is_empty() {
                    target.write_png(
                        &trace_alpha(&pixels, dense.dense())?,
                        &out.join("final-fusion-trace.png"),
                    )?;
                }
            }
        }
    }
    println!(
        "wrote {}: generic, parent/final blend and lens-only arms; red is the computed alpha=0.5 trace",
        out.display()
    );
    Ok(())
}

fn write_bgr_png(bgr: &[u8], path: &Path) -> Fallible<()> {
    let rgb: Vec<u8> = bgr
        .chunks_exact(3)
        .flat_map(|p| [p[2], p[1], p[0]])
        .collect();
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(fs::File::create_new(path)?),
        200,
        100,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&rgb)?;
    Ok(())
}

fn read_floats(path: &Path, count: usize) -> Fallible<Vec<f32>> {
    let bytes = fs::read(path)?;
    if bytes.len() != count * 4 {
        return Err(format!(
            "{} has {} bytes, expected {}",
            path.display(),
            bytes.len(),
            count * 4
        )
        .into());
    }
    println!(
        "input: {} sha256 {}",
        path.display(),
        Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    Ok(bytes
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
        .collect())
}
