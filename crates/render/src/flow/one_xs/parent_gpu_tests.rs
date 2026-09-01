use std::future::Future;
use std::sync::mpsc;

use kjerag_meta::{
    CalibrationSet, ExposureTrack, GyroConfig, GyroEncoding, GyroTrack, OrientationSample,
    OrientationTrack, Quat, Size,
};

use super::*;
use crate::flow::one_xs::LensPair;
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

fn orientation(scale: f64) -> OrientationTrack {
    OrientationTrack::from_samples(
        (1_980_000..=2_020_000)
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

fn readback(token: &ResidentParentMaps) -> Fallible<Vec<u32>> {
    let device = token.context.device();
    let size = (OUTPUT_WORDS * 4) as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ONE X2 parent qualification readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("ONE X2 parent qualification readback"),
    });
    encoder.copy_buffer_to_buffer(&token.storage, 0, &staging, 0, size);
    let submission = token.context.queue().submit([encoder.finish()]);
    let slice = staging.slice(..);
    let (sender, receiver) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |answer| {
        let _ = sender.send(answer);
    });
    device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: None,
    })?;
    receiver.recv()??;
    let mapped = slice.get_mapped_range();
    let words = mapped
        .chunks_exact(4)
        .map(|bytes| u32::from_ne_bytes(bytes.try_into().unwrap()))
        .collect();
    drop(mapped);
    staging.unmap();
    Ok(words)
}

fn assert_case(
    pipeline: &GpuParentMapPipeline,
    builder: &ParentMapBuilder,
    orientation: &OrientationTrack,
    center: Duration,
) {
    let readout = calibration().readout();
    let prepared = builder.prepare(orientation, center, readout).unwrap();
    let expected = expected_words(&prepared);
    let resident = pipeline
        .produce(builder, orientation, center, readout)
        .unwrap();
    let actual = readback(&resident).unwrap();
    if let Some((word, (&actual, &expected))) = actual
        .iter()
        .zip(&expected)
        .enumerate()
        .find(|(_, (actual, expected))| actual != expected)
    {
        panic!("ONE X2 GPU parent word {word} is {actual:#010x}, expected {expected:#010x}");
    }
    assert_eq!(actual.len(), expected.len());
}

#[test]
fn radv_parent_maps_are_exact_cold_warm_and_after_live_mutations() {
    let (device, queue, adapter) = match gpu() {
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
    let pipeline = GpuParentMapPipeline::new(OneXsGpuContext::new(&device, &queue)).unwrap();
    let builder = ParentMapBuilder::new(&calibration()).unwrap();
    let base_orientation = orientation(1.0);

    assert_case(&pipeline, &builder, &base_orientation, CENTER);
    assert_case(&pipeline, &builder, &base_orientation, CENTER);
    assert_case(
        &pipeline,
        &builder,
        &base_orientation,
        CENTER + Duration::from_micros(1),
    );
    assert_case(&pipeline, &builder, &orientation(1.125), CENTER);
    eprintln!("qualified ONE X2 GPU parent maps on {adapter}: 4/4 exact cases");
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

fn gpu() -> Result<(wgpu::Device, wgpu::Queue, String), String> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .map_err(|error| error.to_string())?;
    let info = adapter.get_info();
    let name = format!("{} ({})", info.name, info.driver);
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("ONE X2 GPU parent qualification"),
        ..Default::default()
    }))
    .map_err(|error| error.to_string())?;
    Ok((device, queue, name))
}
