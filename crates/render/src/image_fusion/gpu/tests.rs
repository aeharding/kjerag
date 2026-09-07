use super::*;
use std::{path::PathBuf, sync::mpsc};
use wgpu::util::DeviceExt;

const RATIO_TOLERANCE: f32 = 1.0 / 510.0;
const TEXTURE_ROW_BYTES: u32 = 3_328;
const TEXTURE_BYTES: u64 = TEXTURE_ROW_BYTES as u64 * 100;
const PROFILE_ENV: &str = "KJERAG_FUSION_PROFILE";
const STAGES: [&str; 8] = [
    "admit",
    "prepare",
    "measure",
    "solve",
    "make_ratios",
    "blur",
    "retain_blur",
    "remap",
];

#[test]
fn shader_constructs_on_required_vulkan_adapter() {
    let Some((device, _)) = gpu() else { return };
    let _ = Producer::new(&device).unwrap();
}

#[test]
fn synthetic_observations_match_reference_and_retain_skips() {
    let Some((device, queue)) = gpu() else { return };
    let mut producer = Producer::new(&device).unwrap();
    let pixels = 800 * 16;
    let mut left = [80u8, 100, 120].repeat(pixels);
    let right = [110u8, 100, 90].repeat(pixels);
    let invalid = vec![0u8; 4 * 212];
    let mut reference = super::super::spatial::Reference::new();

    let expected = reference
        .observe_bands([&left, &right], &invalid)
        .unwrap()
        .unwrap()
        .ratios;
    let first = observe(
        &device,
        &queue,
        &mut producer,
        [&left, &right],
        &invalid,
        u32::MAX,
    );
    compare(
        &first,
        [&expected.left.values()[..], &expected.right.values()[..]],
        RATIO_TOLERANCE,
    );

    // The unchanged observation is content-skipped and republishes exactly.
    assert!(
        reference
            .observe_bands([&left, &right], &invalid)
            .unwrap()
            .is_none()
    );
    let skipped = observe(
        &device,
        &queue,
        &mut producer,
        [&left, &right],
        &invalid,
        u32::MAX,
    );
    assert_eq!(first, skipped);

    for byte in left.iter_mut() {
        *byte = byte.saturating_add(4);
    }
    let expected = reference
        .observe_bands([&left, &right], &invalid)
        .unwrap()
        .unwrap()
        .ratios;
    let repeated = observe(
        &device,
        &queue,
        &mut producer,
        [&left, &right],
        &invalid,
        u32::MAX,
    );
    compare(
        &repeated,
        [&expected.left.values()[..], &expected.right.values()[..]],
        RATIO_TOLERANCE,
    );
}

#[test]
fn integer_content_boundary_skips_3168_and_admits_3169() {
    let Some((device, queue)) = gpu() else { return };
    let pixels = 800 * 16;
    let base_left = [80u8, 100, 120].repeat(pixels);
    let right = [110u8, 100, 90].repeat(pixels);
    let invalid = vec![0u8; 4 * 212];
    let mut producer = Producer::new(&device).unwrap();
    let baseline = observe(
        &device,
        &queue,
        &mut producer,
        [&base_left, &right],
        &invalid,
        u32::MAX,
    );

    let mut at_threshold = base_left.clone();
    for y in 0..16 {
        for x in 0..66 {
            at_threshold[(y * 800 + x) * 3] += 3;
        }
    }
    let skipped = observe(
        &device,
        &queue,
        &mut producer,
        [&at_threshold, &right],
        &invalid,
        u32::MAX,
    );
    assert_eq!(skipped, baseline, "aggregate delta 3168 was not skipped");

    at_threshold[0] += 1;
    let admitted = observe(
        &device,
        &queue,
        &mut producer,
        [&at_threshold, &right],
        &invalid,
        u32::MAX,
    );
    assert_ne!(admitted, baseline, "aggregate delta 3169 was not admitted");
}

#[test]
fn invalid_and_global_failure_state_transitions_match_contract() {
    let Some((device, queue)) = gpu() else { return };
    let pixels = 800 * 16;
    let left = [70u8, 90, 115].repeat(pixels);
    let right = [105u8, 95, 80].repeat(pixels);
    let clean = vec![0u8; 4 * 212];
    let all_invalid = vec![1u8; 4 * 212];

    let mut invalid_first = Producer::new(&device).unwrap();
    let neutral = observe(
        &device,
        &queue,
        &mut invalid_first,
        [&left, &right],
        &all_invalid,
        u32::MAX,
    );
    let expected_neutral = vec![[1.0, 1.0, 1.0, 0.0]; MAP_NODES as usize];
    compare(&neutral, [&expected_neutral, &expected_neutral], 1.0e-6);

    // A globally failed observation must not alter the baseline or any inner
    // history. Compare the next valid observation with an untouched producer.
    let mut fresh = Producer::new(&device).unwrap();
    let direct = observe(
        &device,
        &queue,
        &mut fresh,
        [&left, &right],
        &clean,
        u32::MAX,
    );
    for failure in [0, 1, u32::MAX - 1] {
        let mut after_failure = Producer::new(&device).unwrap();
        let _ = observe(
            &device,
            &queue,
            &mut after_failure,
            [&left, &right],
            &all_invalid,
            failure,
        );
        let recovered = observe(
            &device,
            &queue,
            &mut after_failure,
            [&left, &right],
            &clean,
            u32::MAX,
        );
        assert_eq!(
            recovered, direct,
            "failure sentinel {failure:#010x} committed state"
        );
    }

    // Invalidity arriving on a content-skipped observation is nevertheless
    // sticky and excludes a later changed observation.
    let mut sticky = Producer::new(&device).unwrap();
    let mut sticky_reference = super::super::spatial::Reference::new();
    let _ = observe(
        &device,
        &queue,
        &mut sticky,
        [&left, &right],
        &clean,
        u32::MAX,
    );
    sticky_reference
        .observe_bands([&left, &right], &clean)
        .unwrap();
    let _ = observe(
        &device,
        &queue,
        &mut sticky,
        [&left, &right],
        &all_invalid,
        u32::MAX,
    );
    assert!(
        sticky_reference
            .observe_bands([&left, &right], &all_invalid)
            .unwrap()
            .is_none()
    );
    let changed_left = [74u8, 94, 119].repeat(pixels);
    let held = observe(
        &device,
        &queue,
        &mut sticky,
        [&changed_left, &right],
        &clean,
        u32::MAX,
    );
    let expected = sticky_reference
        .observe_bands([&changed_left, &right], &clean)
        .unwrap()
        .unwrap()
        .ratios;
    compare(
        &held,
        [&expected.left.values()[..], &expected.right.values()[..]],
        RATIO_TOLERANCE,
    );
}

#[test]
fn captured_x4_and_x2_bands_track_cpu_reference_outputs() {
    let Some((device, queue)) = gpu() else { return };
    let Some(fixture) = std::env::var_os("KJERAG_FUSION_FIXTURE") else {
        eprintln!("skipping private captured fusion fixture: KJERAG_FUSION_FIXTURE is unset");
        return;
    };
    for camera in ["x4", "x2"] {
        let root = PathBuf::from(&fixture).join(camera);
        let left = std::fs::read(root.join("fusion-band-left.bgr8")).unwrap();
        let right = std::fs::read(root.join("fusion-band-right.bgr8")).unwrap();
        let invalid = std::fs::read(root.join("fusion-invalid.bin")).unwrap();
        let expected = [
            read_float4(root.join("fusion-left.float4")),
            read_float4(root.join("fusion-right.float4")),
        ];
        let mut producer = Producer::new(&device).unwrap();
        let actual = observe(
            &device,
            &queue,
            &mut producer,
            [&left, &right],
            &invalid,
            u32::MAX,
        );
        compare(&actual, [&expected[0], &expected[1]], RATIO_TOLERANCE);
        let identity = vec![[1.0, 1.0, 1.0, 0.0]; MAP_NODES as usize];
        assert!(max_error(&actual, [&identity, &identity]) > RATIO_TOLERANCE);
        assert!(max_error(&actual, [&expected[1], &expected[0]]) > RATIO_TOLERANCE);
        let swizzled: [Vec<[f32; 4]>; 2] = expected
            .each_ref()
            .map(|map| map.iter().map(|v| [v[2], v[1], v[0], v[3]]).collect());
        assert!(max_error(&actual, [&swizzled[0], &swizzled[1]]) > RATIO_TOLERANCE);
    }
}

#[test]
fn profile_captured_producer_stages_when_requested() {
    if std::env::var_os(PROFILE_ENV).is_none() {
        eprintln!("skipping image fusion GPU profile: {PROFILE_ENV} is unset");
        return;
    }
    let fixture = PathBuf::from(
        std::env::var_os("KJERAG_FUSION_FIXTURE")
            .expect("KJERAG_FUSION_FIXTURE is required with KJERAG_FUSION_PROFILE"),
    );
    let Some((device, queue)) = gpu() else { return };
    let timestamps = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("image fusion producer timestamps"),
        ty: wgpu::QueryType::Timestamp,
        count: 16,
    });
    for camera in ["x4", "x2"] {
        let root = fixture.join(camera);
        let left = std::fs::read(root.join("fusion-band-left.bgr8")).unwrap();
        let right = std::fs::read(root.join("fusion-band-right.bgr8")).unwrap();
        let invalid = std::fs::read(root.join("fusion-invalid.bin")).unwrap();
        let mut changed = left.clone();
        changed
            .iter_mut()
            .for_each(|byte| *byte = byte.saturating_add(4));
        let mut producer = Producer::new(&device).unwrap();
        let mut reference = super::super::spatial::Reference::new();
        for (observation, bands) in [
            ("cold", [&left[..], &right[..]]),
            ("skip", [&left[..], &right[..]]),
            ("changed-warm", [&changed[..], &right[..]]),
        ] {
            let reference_output = reference.observe_bands(bands, &invalid).unwrap();
            let budget = reference_output
                .as_ref()
                .and_then(|output| output.diagnostics.budget);
            let cpu_exits = reference_output
                .as_ref()
                .map(|output| output.diagnostics.exits);
            let times =
                profile_observation(&device, &queue, &mut producer, bands, &invalid, &timestamps);
            let milliseconds =
                |ticks: u64| ticks as f64 * f64::from(queue.get_timestamp_period()) / 1_000_000.0;
            let total = milliseconds(times[15] - times[0]);
            eprintln!(
                "fusion profile camera={camera} observation={observation} budget={budget:?} cpu_exits={cpu_exits:?} total_ms={total:.6}"
            );
            for (stage, name) in STAGES.iter().enumerate() {
                let elapsed = milliseconds(times[2 * stage + 1] - times[2 * stage]);
                eprintln!(
                    "fusion profile camera={camera} observation={observation} stage={name} ms={elapsed:.6}"
                );
            }
        }
    }

    // Exercise the maximum retained warm budget through the ordinary support,
    // metric, and content laws. Proportional colors retain correlation while
    // their large signed differences select the 25-step budget.
    let pixels = 800 * 16;
    let left = [60u8, 70, 80].repeat(pixels);
    let right = [180u8, 210, 240].repeat(pixels);
    let mut changed = left.clone();
    changed.iter_mut().for_each(|byte| *byte += 4);
    let invalid = vec![0u8; 4 * 212];
    let mut reference = super::super::spatial::Reference::new();
    reference
        .observe_bands([&left, &right], &invalid)
        .unwrap()
        .unwrap();
    let expected = reference
        .observe_bands([&changed, &right], &invalid)
        .unwrap()
        .unwrap();
    assert_eq!(expected.diagnostics.budget, Some(25));
    let mut producer = Producer::new(&device).unwrap();
    let _ = profile_observation(
        &device,
        &queue,
        &mut producer,
        [&left, &right],
        &invalid,
        &timestamps,
    );
    let times = profile_observation(
        &device,
        &queue,
        &mut producer,
        [&changed, &right],
        &invalid,
        &timestamps,
    );
    let milliseconds =
        |ticks: u64| ticks as f64 * f64::from(queue.get_timestamp_period()) / 1_000_000.0;
    eprintln!(
        "fusion profile camera=synthetic observation=changed-warm-high-budget budget=25 cpu_exits={:?} total_ms={:.6}",
        expected.diagnostics.exits,
        milliseconds(times[15] - times[0])
    );
    for (stage, name) in STAGES.iter().enumerate() {
        eprintln!(
            "fusion profile camera=synthetic observation=changed-warm-high-budget stage={name} ms={:.6}",
            milliseconds(times[2 * stage + 1] - times[2 * stage])
        );
    }
}

fn profile_observation(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    producer: &mut Producer,
    bands: [&[u8]; 2],
    invalid: &[u8],
    timestamps: &wgpu::QuerySet,
) -> Vec<u64> {
    let packed = bands.map(|bytes| {
        let words: Vec<u32> = bytes
            .chunks_exact(3)
            .map(|p| u32::from(p[0]) | u32::from(p[1]) << 8 | u32::from(p[2]) << 16)
            .collect();
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fusion profile band"),
            contents: words_as_bytes(&words),
            usage: wgpu::BufferUsages::STORAGE,
        })
    });
    let invalid_words: Vec<u32> = invalid.iter().map(|&value| u32::from(value)).collect();
    let invalid = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("fusion profile invalid"),
        contents: words_as_bytes(&invalid_words),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let validity = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("fusion profile global validity"),
        contents: words_as_bytes(&[u32::MAX]),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fusion profile timestamp resolve"),
        size: 16 * 8,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fusion profile timestamp readback"),
        size: 16 * 8,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("fusion producer profile"),
    });
    let _output = producer
        .encode_profiled(
            device,
            &mut encoder,
            [&packed[0], &packed[1]],
            &invalid,
            &validity,
            timestamps,
        )
        .unwrap();
    encoder.resolve_query_set(timestamps, 0..16, &resolve, 0);
    encoder.copy_buffer_to_buffer(&resolve, 0, &readback, 0, 16 * 8);
    let submission = queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (send, receive) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |answer| {
        let _ = send.send(answer);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    receive.recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range();
    let times = mapped
        .chunks_exact(8)
        .map(|bytes| u64::from_ne_bytes(bytes.try_into().unwrap()))
        .collect();
    drop(mapped);
    readback.unmap();
    times
}

fn observe(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    producer: &mut Producer,
    bands: [&[u8]; 2],
    invalid: &[u8],
    global: u32,
) -> [Vec<[f32; 4]>; 2] {
    let packed = bands.map(|bytes| {
        let words: Vec<u32> = bytes
            .chunks_exact(3)
            .map(|p| u32::from(p[0]) | u32::from(p[1]) << 8 | u32::from(p[2]) << 16)
            .collect();
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fusion test band"),
            contents: words_as_bytes(&words),
            usage: wgpu::BufferUsages::STORAGE,
        })
    });
    let invalid_words: Vec<u32> = invalid.iter().map(|&v| u32::from(v)).collect();
    let invalid = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("fusion test invalid"),
        contents: words_as_bytes(&invalid_words),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let global = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("fusion test global validity"),
        contents: words_as_bytes(&[global]),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fusion test readback"),
        size: 2 * TEXTURE_BYTES,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("fusion test"),
    });
    let output = producer
        .encode(
            device,
            &mut encoder,
            [&packed[0], &packed[1]],
            &invalid,
            &global,
        )
        .unwrap();
    for lens in 0..2 {
        encoder.copy_texture_to_buffer(
            output.textures[lens].as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: lens as u64 * TEXTURE_BYTES,
                    bytes_per_row: Some(TEXTURE_ROW_BYTES),
                    rows_per_image: Some(100),
                },
            },
            output.textures[lens].size(),
        );
    }
    let submission = queue.submit([encoder.finish()]);
    let slice = staging.slice(..);
    let (send, recv) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |answer| {
        let _ = send.send(answer);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    recv.recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range();
    let mut output: [Vec<[f32; 4]>; 2] =
        std::array::from_fn(|_| Vec::with_capacity(MAP_NODES as usize));
    for (lens, values) in output.iter_mut().enumerate() {
        for row in 0..100u64 {
            let texture_at = lens as u64 * TEXTURE_BYTES + row * u64::from(TEXTURE_ROW_BYTES);
            let texture_row = &mapped[texture_at as usize..(texture_at + 200 * 16) as usize];
            values.extend(texture_row.chunks_exact(16).map(|pixel| {
                std::array::from_fn(|channel| {
                    let at = channel * 4;
                    f32::from_ne_bytes(pixel[at..at + 4].try_into().unwrap())
                })
            }));
        }
    }
    drop(mapped);
    staging.unmap();
    output
}

fn compare(actual: &[Vec<[f32; 4]>; 2], expected: [&[[f32; 4]]; 2], tolerance: f32) {
    let worst = max_error(actual, expected);
    eprintln!("GPU/reference worst ratio error {worst}");
    assert!(
        worst <= tolerance,
        "GPU/reference worst ratio error {worst} exceeds {tolerance}"
    );
}

fn max_error(actual: &[Vec<[f32; 4]>; 2], expected: [&[[f32; 4]]; 2]) -> f32 {
    let mut worst = 0.0f32;
    for lens in 0..2 {
        for (a, e) in actual[lens].iter().zip(expected[lens]) {
            for c in 0..4 {
                let error = (a[c] - e[c]).abs();
                assert!(
                    a[c].is_finite() && e[c].is_finite(),
                    "nonfinite ratio at lens {lens} channel {c}: GPU={} reference={}",
                    a[c],
                    e[c]
                );
                worst = worst.max(error);
            }
        }
    }
    worst
}

fn read_float4(path: PathBuf) -> Vec<[f32; 4]> {
    std::fs::read(path)
        .unwrap()
        .chunks_exact(16)
        .map(|p| {
            std::array::from_fn(|c| f32::from_le_bytes(p[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect()
}

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let Some(adapter) = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
        .into_iter()
        .next()
    else {
        assert!(
            std::env::var_os("KJERAG_REQUIRE_GPU").is_none(),
            "KJERAG_REQUIRE_GPU test has no Vulkan adapter"
        );
        eprintln!("skipping image fusion GPU producer: no Vulkan adapter");
        return None;
    };
    eprintln!("image fusion GPU adapter: {:?}", adapter.get_info());
    let required_features = if std::env::var_os(PROFILE_ENV).is_some() {
        let timestamps =
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES;
        assert!(
            adapter.features().contains(timestamps),
            "image fusion profile adapter lacks pass timestamp queries"
        );
        timestamps
    } else {
        wgpu::Features::empty()
    };
    match block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("image fusion GPU producer qualification"),
        required_features,
        required_limits: adapter.limits(),
        ..Default::default()
    })) {
        Ok(device) => Some(device),
        Err(error) => {
            assert!(
                std::env::var_os("KJERAG_REQUIRE_GPU").is_none(),
                "image fusion GPU producer device: {error}"
            );
            eprintln!("skipping image fusion GPU producer: {error}");
            None
        }
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(answer) => return answer,
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}
