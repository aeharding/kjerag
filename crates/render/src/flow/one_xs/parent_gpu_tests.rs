use std::future::Future;

use kjerag_media::FrameStamp;
use kjerag_meta::{
    CalibrationSet, ExposureTrack, GyroConfig, GyroEncoding, GyroTrack, OrientationSample,
    OrientationTrack, Quat, Size,
};

use super::*;
use crate::flow::one_xs::LensPair;
use crate::flow::one_xs::pis::gpu::GpuPisFlight;
use crate::flow::one_xs_belt_gpu::resident_qualification_fixture;
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
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    context: &OneXsGpuContext,
    pipeline: &GpuParentMapPipeline,
    builder: &ParentMapBuilder,
    orientation: &OrientationTrack,
    flight: GpuPisFlight,
) -> Vec<u32> {
    assert_case_readout(
        device,
        queue,
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
    device: &wgpu::Device,
    queue: &wgpu::Queue,
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
    let (belts, _) = resident_qualification_fixture(device, queue, (), flight.clone()).unwrap();
    let resident = belts
        .produce_parent_maps(pipeline, builder, orientation, &flight, readout)
        .unwrap();
    assert_eq!(resident.flight(), &flight);
    let actual = resident.read_qualification(context).unwrap();
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
        &device,
        &queue,
        &context,
        &pipeline,
        &builder,
        &base_orientation,
        flight(7, 41, CENTER),
    );
    let warm = assert_case(
        &device,
        &queue,
        &context,
        &pipeline,
        &builder,
        &base_orientation,
        flight(8, 42, CENTER),
    );
    assert_eq!(cold, warm);
    let shifted = assert_case(
        &device,
        &queue,
        &context,
        &pipeline,
        &builder,
        &base_orientation,
        flight(9, 43, CENTER + Duration::from_micros(1)),
    );
    let moved = assert_case(
        &device,
        &queue,
        &context,
        &pipeline,
        &builder,
        &orientation(1.125),
        flight(10, 44, CENTER),
    );
    assert_ne!(cold, shifted, "the +1 us live center mutation was inert");
    assert_ne!(cold, moved, "the live orientation mutation was inert");
    let early = assert_case(
        &device,
        &queue,
        &context,
        &pipeline,
        &builder,
        &base_orientation,
        flight(11, 45, Duration::from_micros(1_970_000)),
    );
    let late = assert_case(
        &device,
        &queue,
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
        &device,
        &queue,
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
fn parent_transition_refuses_wrong_flight_foreign_context_and_nonlinear_slerp() {
    let adapter = match gpu_adapter() {
        Ok(adapter) => adapter,
        Err(_) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => return,
        Err(why) => panic!("GPU required: {why}"),
    };
    let (device, queue) = request_device(&adapter).unwrap();
    let context = OneXsGpuContext::new(&device, &queue);
    let pipeline = GpuParentMapPipeline::new(context.clone()).unwrap();
    let builder = ParentMapBuilder::new(&calibration()).unwrap();
    let owner = flight(20, 50, CENTER);
    let wrong = flight(20, 50, CENTER);
    let (belts, _) = resident_qualification_fixture(&device, &queue, (), owner.clone()).unwrap();
    let error = refused(belts.produce_parent_maps(
        &pipeline,
        &builder,
        &orientation(1.0),
        &wrong,
        calibration().readout(),
    ));
    assert!(error.to_string().contains("does not match"));
    assert_eq!(pipeline.encoded_transitions(), 0);

    let (foreign_device, foreign_queue) = request_device(&adapter).unwrap();
    let foreign =
        GpuParentMapPipeline::new(OneXsGpuContext::new(&foreign_device, &foreign_queue)).unwrap();
    let (belts, _) = resident_qualification_fixture(&device, &queue, (), owner.clone()).unwrap();
    let error = refused(belts.produce_parent_maps(
        &foreign,
        &builder,
        &orientation(1.0),
        &owner,
        calibration().readout(),
    ));
    assert!(error.to_string().contains("different device or queue"));
    assert_eq!(foreign.encoded_transitions(), 0);

    let (belts, _) = resident_qualification_fixture(&device, &queue, (), owner.clone()).unwrap();
    let error = refused(belts.produce_parent_maps(
        &pipeline,
        &builder,
        &orientation(3_000.0),
        &owner,
        calibration().readout(),
    ));
    assert!(error.to_string().contains("nonlinear interpolation"));
    assert_eq!(pipeline.encoded_transitions(), 0);
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
            "const POSE_BASE: u32 = 66u",
            "const POSE_BASE: u32 = 67u",
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
        let (belts, _) =
            resident_qualification_fixture(&device, &queue, (), owner.clone()).unwrap();
        let resident = belts
            .produce_parent_maps(&pipeline, &builder, &poses, &owner, calibration().readout())
            .unwrap();
        let actual = resident.read_qualification(&context).unwrap();
        assert_ne!(
            actual, expected,
            "planted {name} mutation escaped qualification"
        );
        eprintln!("rejected planted parent mutation: {name}");
    }
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
        ..Default::default()
    }))
    .map_err(|error| error.to_string())
}
