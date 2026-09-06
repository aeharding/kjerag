use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use kjerag_media::FrameStamp;
use kjerag_meta::{
    CalibrationSet, ExposureTrack, GyroConfig, GyroEncoding, GyroTrack, OrientationSample,
    OrientationTrack, Quat, Size,
};

use super::super::geometry_gpu::GpuGeometryPipeline;
use super::super::resident_frame_gpu::GpuResidentCapture;
use super::super::{GpuResidentFramePipeline, GpuSolverBeltPipeline, SourceTextures};
use super::*;
use crate::flow::one_xs::LensPair;
use crate::flow::one_xs::base_map::one_xs_static_coordinates;
use crate::flow::one_xs::pis::gpu::GpuPisFlight;
use crate::projection::tests::{ONE_XS_FRAME, one_xs_lenses};

const CENTER: Duration = Duration::from_micros(2_000_000);

fn calibration() -> CalibrationSet {
    CalibrationSet {
        camera_model: "Insta360 ONE X2".to_owned(),
        firmware: "gpu-parent-qualification".to_owned(),
        dimension: Size {
            width: ONE_XS_FRAME.width,
            height: ONE_XS_FRAME.height,
        },
        lenses: one_xs_lenses(),
        model6: None,
        rolling_shutter_ms: 20.0,
        gyro: GyroConfig {
            encoding: GyroEncoding::Scaled,
            imu_orientation: "Zxy",
            first_frame_timestamp: 0,
            gyro_timestamp: None,
        },
        exposure: [ExposureTrack::default(), ExposureTrack::default()],
        imu: GyroTrack::default(),
        fused: OrientationTrack::default(),
        calibration_canvas: Size {
            width: 6_080,
            height: 3_040,
        },
    }
}

fn calibration_with_readout(milliseconds: f64) -> CalibrationSet {
    let mut value = calibration();
    value.rolling_shutter_ms = milliseconds;
    value
}

fn orientation(scale: f64) -> OrientationTrack {
    OrientationTrack::from_samples(
        (1_960_000..=2_040_000)
            .step_by(2_000)
            .map(|offset_us| OrientationSample {
                offset_us,
                world_from_body: Quat::from_rotation_vector([
                    (offset_us - 2_000_000) as f64 * 1.0e-7 * scale,
                    (offset_us - 2_000_000) as f64 * -0.5e-7 * scale,
                    (offset_us - 2_000_000) as f64 * 0.25e-7 * scale,
                ]),
            })
            .collect(),
    )
}

fn expected_words(prepared: &PreparedParentMap) -> Vec<u32> {
    let LensPair { a, b } = prepared.scalar_maps();
    a.row_major_values()
        .iter()
        .chain(b.row_major_values())
        .flat_map(|value| value.map(f32::to_bits))
        .collect()
}

fn flight(generation: u64, index: u64, center: Duration) -> GpuPisFlight {
    GpuPisFlight {
        generation,
        frame: FrameStamp::for_test(index, center, None),
    }
}

fn refused<T>(answer: Fallible<T>) -> Box<dyn std::error::Error + Send + Sync> {
    match answer {
        Ok(_) => panic!("resident parent transition unexpectedly succeeded"),
        Err(error) => error,
    }
}

fn assert_case(
    context: &OneXsGpuContext,
    pipeline: &GpuParentMapPipeline,
    builder: &ParentMapBuilder,
    orientation: &OrientationTrack,
    flight: GpuPisFlight,
) -> Vec<u32> {
    assert_case_readout(
        context,
        pipeline,
        builder,
        orientation,
        flight,
        calibration().readout(),
    )
}

#[allow(clippy::too_many_arguments)]
fn assert_case_readout(
    context: &OneXsGpuContext,
    pipeline: &GpuParentMapPipeline,
    builder: &ParentMapBuilder,
    orientation: &OrientationTrack,
    flight: GpuPisFlight,
    readout: kjerag_meta::Readout,
) -> Vec<u32> {
    let prepared = builder
        .prepare(orientation, flight.frame.timestamp(), readout)
        .unwrap();
    let expected = expected_words(&prepared);
    let capture = GpuResidentCapture::new();
    let reservation = capture.reserve(flight.frame.clone()).unwrap();
    let encoded = pipeline
        .encode(builder, orientation, reservation, readout)
        .unwrap();
    assert_eq!(encoded.flight.generation, 1);
    assert_eq!(encoded.flight.frame, flight.frame);
    let actual = encoded.read_qualification(context).unwrap();
    if let Some((word, (&actual, &expected))) = actual
        .iter()
        .zip(&expected)
        .enumerate()
        .find(|(_, (actual, expected))| actual != expected)
    {
        panic!("ONE X2 GPU parent word {word} is {actual:#010x}, expected {expected:#010x}");
    }
    assert_eq!(actual.len(), expected.len());
    expected
}

#[test]
fn radv_parent_maps_are_exact_cold_warm_and_after_live_mutations() {
    let (device, queue, adapter, limits) = match gpu() {
        Ok(gpu) => gpu,
        Err(why) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => {
            eprintln!("skipping ONE X2 GPU parent qualification: {why}");
            return;
        }
        Err(why) => panic!("GPU required for ONE X2 parent qualification: {why}"),
    };
    if std::env::var_os("KJERAG_REQUIRE_RADV").is_some() {
        assert!(
            adapter.to_ascii_lowercase().contains("radv"),
            "forced RADV qualification selected {adapter}"
        );
    }
    eprintln!("adapter: {adapter}\nlimits: {limits}");
    let context = OneXsGpuContext::new(&device, &queue);
    let pipeline = GpuParentMapPipeline::new(context.clone()).unwrap();
    let builder = ParentMapBuilder::new(&calibration()).unwrap();
    let base_orientation = orientation(1.0);
    let cold = assert_case(
        &context,
        &pipeline,
        &builder,
        &base_orientation,
        flight(7, 41, CENTER),
    );
    let warm = assert_case(
        &context,
        &pipeline,
        &builder,
        &base_orientation,
        flight(8, 42, CENTER),
    );
    assert_eq!(cold, warm);
    let shifted = assert_case(
        &context,
        &pipeline,
        &builder,
        &base_orientation,
        flight(9, 43, CENTER + Duration::from_micros(1)),
    );
    let moved = assert_case(
        &context,
        &pipeline,
        &builder,
        &orientation(1.125),
        flight(10, 44, CENTER),
    );
    assert_ne!(cold, shifted, "the +1 us live center mutation was inert");
    assert_ne!(cold, moved, "the live orientation mutation was inert");
    let early = assert_case(
        &context,
        &pipeline,
        &builder,
        &base_orientation,
        flight(11, 45, Duration::from_micros(1_970_000)),
    );
    let late = assert_case(
        &context,
        &pipeline,
        &builder,
        &base_orientation,
        flight(12, 46, Duration::from_micros(2_030_000)),
    );
    assert_ne!(early, late, "endpoint/clamp qualification cases were inert");
    let readout_calibration = calibration_with_readout(23.516);
    let readout_builder = ParentMapBuilder::new(&readout_calibration).unwrap();
    let changed_readout = assert_case_readout(
        &context,
        &pipeline,
        &readout_builder,
        &base_orientation,
        flight(13, 47, CENTER),
        readout_calibration.readout(),
    );
    assert_ne!(cold, changed_readout, "the live readout mutation was inert");
    eprintln!("qualified ONE X2 GPU parent maps on {adapter}: 7/7 exact cases");
}

#[test]
fn parent_transition_seals_flight_and_refuses_foreign_geometry_and_nonlinear_slerp() {
    let adapter = match gpu_adapter() {
        Ok(adapter) => adapter,
        Err(_) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => return,
        Err(why) => panic!("GPU required: {why}"),
    };
    let (device, queue) = request_device(&adapter).unwrap();
    let context = OneXsGpuContext::new(&device, &queue);
    let pipeline = GpuParentMapPipeline::new(context.clone()).unwrap();
    let builder = ParentMapBuilder::new(&calibration()).unwrap();
    let capture = GpuResidentCapture::new();
    let owner = flight(20, 50, CENTER);
    let encoded = pipeline
        .encode(
            &builder,
            &orientation(1.0),
            capture.reserve(owner.frame.clone()).unwrap(),
            calibration().readout(),
        )
        .unwrap();
    assert_eq!(encoded.flight.generation, 1);
    assert_eq!(encoded.flight.frame, owner.frame);
    assert_eq!(pipeline.encoded_transitions(), 1);

    let (foreign_device, foreign_queue) = request_device(&adapter).unwrap();
    let foreign_context = OneXsGpuContext::new(&foreign_device, &foreign_queue);
    let foreign_geometry = GpuGeometryPipeline::new(
        foreign_context,
        &one_xs_static_coordinates(),
        &crate::flow::one_xs_belt::CameraMaskSupport::for_test(),
    )
    .unwrap();
    let error = refused(foreign_geometry.encode_resident_parents(encoded));
    assert!(error.to_string().contains("different device or queue"));

    let error = refused(pipeline.encode(
        &builder,
        &orientation(3_000.0),
        capture.reserve(owner.frame).unwrap(),
        calibration().readout(),
    ));
    assert!(error.to_string().contains("nonlinear interpolation"));
    assert_eq!(pipeline.encoded_transitions(), 1);
}

#[test]
fn duplicate_front_half_is_refused_before_a_second_parent_encode() {
    let (device, queue, _, _) = match gpu() {
        Ok(gpu) => gpu,
        Err(_) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => return,
        Err(why) => panic!("GPU required: {why}"),
    };
    let context = OneXsGpuContext::new(&device, &queue);
    let resident = GpuResidentFramePipeline::new(context).unwrap();
    let capture = GpuResidentCapture::new();
    let builder = ParentMapBuilder::new(&calibration()).unwrap();
    let first = resident
        .begin_parent(
            &capture,
            flight(40, 40, CENTER).frame,
            &builder,
            &orientation(1.0),
            calibration().readout(),
        )
        .unwrap();
    assert_eq!(resident.parent.encoded_transitions(), 1);
    let error = refused(resident.begin_parent(
        &capture,
        flight(41, 41, CENTER).frame,
        &builder,
        &orientation(1.0),
        calibration().readout(),
    ));
    assert!(error.to_string().contains("in-flight frame"), "{error}");
    assert_eq!(resident.parent.encoded_transitions(), 1);
    let state = capture.snapshot();
    assert_eq!(state.generation, 1);
    assert!(state.pending);
    assert!(state.committed.is_none());
    assert!(!state.ready);
    drop(first);
    assert!(!capture.snapshot().pending);

    let error = refused(resident.begin_parent(
        &capture,
        flight(42, 42, CENTER).frame,
        &builder,
        &orientation(3_000.0),
        calibration().readout(),
    ));
    assert!(error.to_string().contains("nonlinear interpolation"));
    let state = capture.snapshot();
    assert_eq!(state.generation, 2);
    assert!(!state.pending);
    assert_eq!(resident.parent.encoded_transitions(), 1);
}

#[test]
fn qualification_rejects_planted_parent_semantic_mutations() {
    let (device, queue, _, _) = match gpu() {
        Ok(gpu) => gpu,
        Err(_) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => return,
        Err(why) => panic!("GPU required: {why}"),
    };
    let context = OneXsGpuContext::new(&device, &queue);
    let builder = ParentMapBuilder::new(&calibration()).unwrap();
    let poses = orientation(20.0);
    let prepared = builder
        .prepare(&poses, CENTER, calibration().readout())
        .unwrap();
    prepared.require_linear_gpu_slerp().unwrap();
    let expected = expected_words(&prepared);
    let mutations = [
        (
            "quaternion order",
            "quaternion_multiply(quat_param(lens, 8u), get_quat(scan_factor(src_pos, lens)))",
            "quaternion_multiply(get_quat(scan_factor(src_pos, lens)), quat_param(lens, 8u))",
        ),
        (
            "raster orientation",
            "mul_rn(radius, bitcast<f32>(PHI_SIN[column]))",
            "mul_rn(-radius, bitcast<f32>(PHI_SIN[column]))",
        ),
        (
            "scan axis",
            "if inputs[lens * PARAM_WORDS + 23u] != 0u",
            "if inputs[lens * PARAM_WORDS + 23u] == 0u",
        ),
        ("four iterations", "iteration < 4u", "iteration < 1u"),
        ("movement threshold", "movement < 4.0", "movement < 4000.0"),
        ("FOV", "MAX_FOV_COS),0.01", "MAX_FOV_COS),0.25"),
        ("A/B output base", "gid.z * LENS_OUTPUT_WORDS", "gid.z * 0u"),
        (
            "pose layout",
            "const POSE_BASE: u32 = 94u",
            "const POSE_BASE: u32 = 95u",
        ),
        (
            "low endpoint clamp",
            "if floored < 0.0 { return pose(0u); }",
            "if floored < 0.0 { return pose(50u); }",
        ),
        (
            "high endpoint clamp",
            "return pose(50u);\n}",
            "return pose(0u);\n}",
        ),
        (
            "division",
            "return bitcast<f32>(div_f32_bits(bitcast<u32>(a), bitcast<u32>(b)));",
            "return a / b;",
        ),
        (
            "square root",
            "let bits = bitcast<u32>(value);",
            "return sqrt(value);\n    let bits = bitcast<u32>(value);",
        ),
        (
            "rounding",
            "return materialize(fma(a, b, -0.0));",
            "return a * b;",
        ),
    ];
    for (number, (name, needle, replacement)) in mutations.into_iter().enumerate() {
        let source = shader_source();
        assert!(
            source.contains(needle),
            "missing planted {name} mutation site"
        );
        let source = source.replacen(needle, replacement, 1);
        let pipeline = GpuParentMapPipeline::new_with_shader(context.clone(), source).unwrap();
        let owner = flight(100 + number as u64, 100 + number as u64, CENTER);
        let capture = GpuResidentCapture::new();
        let reservation = capture.reserve(owner.frame).unwrap();
        let encoded = pipeline
            .encode(&builder, &poses, reservation, calibration().readout())
            .unwrap();
        let actual = encoded.read_qualification(&context).unwrap();
        assert_ne!(
            actual, expected,
            "planted {name} mutation escaped qualification"
        );
        eprintln!("rejected planted parent mutation: {name}");
    }
}

#[test]
fn parent_geometry_and_belts_share_one_pre_submission_owner_chain() {
    let (device, queue, adapter, _) = match gpu() {
        Ok(gpu) => gpu,
        Err(_) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => return,
        Err(why) => panic!("GPU required: {why}"),
    };
    let context = OneXsGpuContext::new(&device, &queue);
    let initial_flight = flight(900, 901, CENTER);
    let capture = GpuResidentCapture::new();
    let resident = GpuResidentFramePipeline::new(context.clone()).unwrap();
    let parent = resident
        .begin_parent(
            &capture,
            initial_flight.frame,
            &ParentMapBuilder::new(&calibration()).unwrap(),
            &orientation(1.0),
            calibration().readout(),
        )
        .unwrap();
    let geometry = GpuGeometryPipeline::new(
        context.clone(),
        &one_xs_static_coordinates(),
        &crate::flow::one_xs_belt::CameraMaskSupport::for_test(),
    )
    .unwrap()
    .encode_resident_parents(parent)
    .unwrap();
    let texture = |label| {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: 256,
                height: 256,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &vec![137; 256 * 256],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(256),
            },
            texture.size(),
        );
        texture
    };
    let texture_a = texture("parent geometry belt A");
    let texture_b = texture("parent geometry belt B");
    let source_owner = Arc::new(());
    let belts = geometry
        .submit_belts(
            &GpuSolverBeltPipeline::new(context.clone()).unwrap(),
            SourceTextures {
                a: &texture_a,
                b: &texture_b,
            },
            Arc::clone(&source_owner),
        )
        .unwrap();
    assert_eq!(Arc::strong_count(&source_owner), 2);
    let state = capture.snapshot();
    assert_eq!(state.generation, 1);
    assert!(state.pending);
    let error = refused(resident.begin_parent(
        &capture,
        flight(901, 902, CENTER).frame,
        &ParentMapBuilder::new(&calibration()).unwrap(),
        &orientation(1.0),
        calibration().readout(),
    ));
    assert!(error.to_string().contains("in-flight frame"), "{error}");
    assert_eq!(resident.parent.encoded_transitions(), 1);
    drop(belts);
    assert_eq!(Arc::strong_count(&source_owner), 2);
    assert!(!capture.snapshot().pending);
    eprintln!("ONE X2 GPU parent, geometry and belts retained one root reservation on {adapter}");
}

const MODEL6_COEFFICIENTS: [[f32; 13]; 2] = [
    [
        0.96564066,
        -1.893_707_5,
        3.895_677,
        -0.13278545,
        0.0,
        0.00176593,
        -0.00078501,
        0.00636060,
        0.00263606,
        -0.00638099,
        -0.00193177,
        -0.02387245,
        -0.00347206,
    ],
    [
        0.927_919_8,
        -1.358_195_3,
        -0.19015622,
        9.317_827,
        0.0,
        -0.00192407,
        -0.00334321,
        0.00950197,
        -0.00726822,
        0.00276126,
        0.00455220,
        0.01397504,
        0.01839879,
    ],
];

fn model6_parameters(coefficients: [f32; 13]) -> MetalCalcMapParams {
    MetalCalcMapParams {
        center: [1920.0, 1920.0],
        focal: [3668.0, 3668.0],
        src_size: [3840.0, 3840.0],
        inv_src_size: [1.0, 1.0],
        qci: [0.0, 0.0, 0.0, 1.0],
        qwm: [0.0, 0.0, 0.0, 1.0],
        q_c0_f0: [0.0, 0.0, 0.0, 1.0],
        shift: [0.0, 0.0],
        xi: 2.31494,
        is_horizon_sweep: false,
        flip: 1.0,
        max_fov: f32::from_bits(0x4006_0a92),
        distort_coeffs: coefficients[..5].try_into().unwrap(),
        model6_distortion: Some(coefficients),
        pos_scale: [3840.0, 3840.0],
    }
}

fn model6_prepared() -> PreparedParentMap {
    PreparedParentMap {
        parameters: LensPair {
            a: model6_parameters(MODEL6_COEFFICIENTS[0]),
            b: model6_parameters(MODEL6_COEFFICIENTS[1]),
        },
        poses: [[0.0, 0.0, 0.0, 1.0]; 51],
    }
}

#[test]
fn pack_preserves_model3_prefix_and_appends_model6_payloads() {
    let prepared = model6_prepared();
    let words = pack(&prepared);
    assert_eq!(&words[..PARAM_WORDS], &{
        let mut expected = Vec::new();
        pack_parameters(&mut expected, &prepared.parameters.a);
        expected
    });
    assert_eq!(&words[PARAM_WORDS..2 * PARAM_WORDS], &{
        let mut expected = Vec::new();
        pack_parameters(&mut expected, &prepared.parameters.b);
        expected
    });
    assert_eq!(words[2 * PARAM_WORDS], 1);
    assert_eq!(words[2 * PARAM_WORDS + MODEL6_WORDS], 1);
    assert_eq!(words.len(), INPUT_WORDS);
}

#[test]
fn gpu_model6_maps_match_the_scalar_projector_bit_for_bit() {
    let (device, queue, adapter, _) = match gpu() {
        Ok(gpu) => gpu,
        Err(why) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => {
            eprintln!("skipping model-6 GPU parent qualification: {why}");
            return;
        }
        Err(why) => panic!("GPU required for model-6 parent qualification: {why}"),
    };
    let context = OneXsGpuContext::new(&device, &queue);
    let pipeline = GpuParentMapPipeline::new(context.clone()).unwrap();
    let prepared = model6_prepared();
    let LensPair { a, b } = prepared.scalar_maps();
    let expected: Vec<u32> = a
        .row_major_values()
        .iter()
        .chain(b.row_major_values())
        .flat_map(|position| position.map(f32::to_bits))
        .collect();
    let capture = GpuResidentCapture::new();
    let frame = FrameStamp::for_test(6, Duration::from_secs(1), None);
    let reservation = capture.reserve(frame.clone()).unwrap();
    let encoded = pipeline.encode_prepared(
        &prepared,
        GpuPisFlight {
            generation: 1,
            frame,
        },
        reservation,
    );
    let actual = encoded.read_qualification(&context).unwrap();
    if let Some((word, (&actual, &expected))) = actual
        .iter()
        .zip(&expected)
        .enumerate()
        .find(|(_, (actual, expected))| actual != expected)
    {
        panic!("model-6 GPU parent word {word} is {actual:#010x}, expected {expected:#010x}");
    }
    assert_eq!(actual.len(), expected.len());
    eprintln!("qualified model-6 GPU parent maps on {adapter}");
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(answer) => return answer,
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn gpu() -> Result<(wgpu::Device, wgpu::Queue, String, String), String> {
    let adapter = gpu_adapter()?;
    let info = adapter.get_info();
    let name = format!("{} ({})", info.name, info.driver);
    let limits = format!("{:?}", adapter.limits());
    let (device, queue) = request_device(&adapter)?;
    Ok((device, queue, name, limits))
}

fn gpu_adapter() -> Result<wgpu::Adapter, String> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .map_err(|error| error.to_string())
}

fn request_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), String> {
    block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("ONE X2 GPU parent qualification"),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .map_err(|error| error.to_string())
}
