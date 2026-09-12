use super::*;
use std::{io::Write, path::PathBuf, sync::mpsc};
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
    let _ = Producer::new(&device, StitchCamera::OneX2).unwrap();
}

#[test]
fn x4_publishes_native_corrections_in_the_delivered_lens_and_body_chart() {
    let Some((device, queue)) = gpu() else { return };
    let mut native = Producer::new(&device, StitchCamera::OneX2).unwrap();
    let mut x4 = Producer::new(&device, StitchCamera::CalibratedMei).unwrap();
    // Both producers receive the same native-ordinal observations. Spatially
    // varying, non-gray evidence distinguishes both the lens swap and the
    // fixed sphere Ry(pi) datum from a coincidentally neutral publication.
    let bands: [Vec<u8>; 2] = std::array::from_fn(|lens| {
        (0..800 * 16)
            .flat_map(|pixel| {
                let column = (pixel % 800) / 8;
                let row = pixel / 800;
                if lens == 0 {
                    [70 + column as u8, 100 + row as u8, 90]
                } else {
                    [100 + column as u8, 85, 120 + row as u8]
                }
            })
            .collect()
    });
    let invalid = vec![0u8; 4 * 212];
    let original = observe(
        &device,
        &queue,
        &mut native,
        [&bands[0], &bands[1]],
        &invalid,
        u32::MAX,
    );
    let delivered = observe(
        &device,
        &queue,
        &mut x4,
        [&bands[0], &bands[1]],
        &invalid,
        u32::MAX,
    );
    assert_ne!(original, delivered);
    let expected = super::super::spatial::Reference::for_camera(StitchCamera::CalibratedMei)
        .observe_bands([&bands[0], &bands[1]], &invalid)
        .unwrap()
        .unwrap()
        .ratios;
    // All nodes, including both poles and the discontinuous native azimuth
    // clamp, must match the readable camera-boundary calculation.
    compare(
        &delivered,
        [&expected.left.values()[..], &expected.right.values()[..]],
        RATIO_TOLERANCE,
    );
    for lens in 0..2 {
        for row in 0..100 {
            for column in 0..200 {
                // The consumer reflects a texel-centered texture, not the
                // producer's endpoint sphere. The same native coordinates
                // must therefore produce an exact permutation, including
                // poles and the producer's discontinuous azimuth clamp.
                let expected = original[1 - lens][(99 - row) * 200 + (299 - column) % 200];
                let actual = delivered[lens][row * 200 + column];
                for channel in 0..4 {
                    assert_eq!(
                        actual[channel].to_bits(),
                        expected[channel].to_bits(),
                        "X4 texture rebase lens {lens} row {row} column {column} channel {channel}",
                    );
                }
            }
        }
    }
    let held = observe(
        &device,
        &queue,
        &mut x4,
        [&bands[0], &bands[1]],
        &invalid,
        u32::MAX,
    );
    assert_eq!(delivered, held, "rebasing must not add a new update policy");
}

#[test]
fn synthetic_observations_match_reference_and_retain_skips() {
    let Some((device, queue)) = gpu() else { return };
    let mut producer = Producer::new(&device, StitchCamera::OneX2).unwrap();
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
fn full_overlap_equations_match_reference_for_outer_rows_and_edges() {
    let Some((device, queue)) = gpu() else { return };
    // The quantile evidence remains in the two central rows and inner 176
    // columns. Give the other two rows and wrapped edge columns distinct,
    // non-gray corrections so a solver which reuses that evidence mask for
    // equation emission cannot accidentally agree with the reference.
    let bands: [Vec<u8>; 2] = std::array::from_fn(|lens| {
        (0..800 * 16)
            .flat_map(|pixel| {
                let source_row = pixel / 800;
                let source_column = pixel % 800;
                let reduced_row = source_row / 4;
                let reduced_column = source_column / 4;
                let left = [
                    60 + (reduced_column % 31) as u8,
                    95 + (7 * reduced_row + reduced_column % 17) as u8,
                    130 + (reduced_column % 23) as u8,
                ];
                if lens == 0 {
                    left
                } else {
                    let wrapped_edge = !(6..194).contains(&reduced_column);
                    let delta: [i16; 3] = if reduced_row == 0 || reduced_row == 3 {
                        [-7, 8, -5]
                    } else if wrapped_edge {
                        [14, -11, 10]
                    } else {
                        [6, -4, 5]
                    };
                    std::array::from_fn(|channel| (i16::from(left[channel]) + delta[channel]) as u8)
                }
            })
            .collect()
    });
    let invalid = vec![0u8; 4 * 212];
    let expected = super::super::spatial::Reference::new()
        .observe_bands([&bands[0], &bands[1]], &invalid)
        .unwrap()
        .unwrap()
        .ratios;
    let mut producer = Producer::new(&device, StitchCamera::OneX2).unwrap();
    let actual = observe(
        &device,
        &queue,
        &mut producer,
        [&bands[0], &bands[1]],
        &invalid,
        u32::MAX,
    );
    compare(
        &actual,
        [&expected.left.values()[..], &expected.right.values()[..]],
        RATIO_TOLERANCE,
    );
}

#[test]
fn periodic_edge_join_uses_solved_extensions_and_truncates_half_codes() {
    let Some((device, queue)) = gpu() else { return };
    let producer = Producer::new(&device, StitchCamera::OneX2).unwrap();
    let storage = |label, contents: &[u8], copy_src| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            usage: wgpu::BufferUsages::STORAGE
                | if copy_src {
                    wgpu::BufferUsages::COPY_SRC
                } else {
                    wgpu::BufferUsages::empty()
                },
        })
    };

    let mut state = vec![0u32; STATE_WORDS];
    state[19] = 1; // CHANGED
    state[20] = 1; // SOLVE_ACTIVE
    let state = storage("periodic join state", words_as_bytes(&state), false);
    let packed_gray = 100u32 | (100u32 << 8) | (100u32 << 16);
    let working_words = vec![packed_gray; 2 * 212 * 100];
    let working = storage(
        "periodic join working images",
        words_as_bytes(&working_words),
        false,
    );
    let mut fields = vec![0.0f32; 3 * 5_088];
    // Lens zero's row-49, extended-column-206 node solves 11 codes brighter
    // than the separately corrected center-crop node at extended column 6.
    fields[9 * 212 + 206] = 11.0;
    let field = storage("periodic join fields", floats_as_bytes(&fields), false);
    let initial_ratios = vec![[1.0f32, 1.0, 1.0, 0.0]; 2 * MAP_NODES as usize];
    let ratios = storage(
        "periodic join ratios",
        float4_as_bytes(&initial_ratios),
        true,
    );
    let zeros = vec![0u32; 800 * 16];
    let band = storage("periodic join band", words_as_bytes(&zeros), false);
    let invalid_words = vec![0u32; 4 * 212];
    let invalid = storage(
        "periodic join invalid",
        words_as_bytes(&invalid_words),
        false,
    );
    let validity = storage("periodic join validity", words_as_bytes(&[u32::MAX]), false);
    let textures: [wgpu::Texture; 2] = std::array::from_fn(|_| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("periodic join unused output"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        })
    });
    let views = textures
        .each_ref()
        .map(|texture| texture.create_view(&wgpu::TextureViewDescriptor::default()));
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("periodic join focused regression"),
        layout: &producer.layout,
        entries: &[
            entry(0, &band),
            entry(1, &band),
            entry(2, &invalid),
            entry(3, &validity),
            entry(4, &state),
            entry(5, &working),
            entry(6, &field),
            entry(7, &producer.cg),
            entry(8, &ratios),
            entry(9, &producer.blurred),
            entry(10, &producer.fixed),
            texture_entry(11, &views[0]),
            texture_entry(12, &views[1]),
        ],
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("periodic join ratio readback"),
        size: ratios.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("periodic join focused regression"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("periodic join focused regression"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&producer.pipelines[4]);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(625, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&ratios, 0, &readback, 0, ratios.size());
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
    let node = 49 * 200;
    let actual: [f32; 4] = std::array::from_fn(|channel| {
        let at = node * 16 + channel * 4;
        f32::from_ne_bytes(mapped[at..at + 4].try_into().unwrap())
    });
    let expected = (105.0f32 + 255.0) / (100.0 + 255.0);
    let rounded = (106.0f32 + 255.0) / (100.0 + 255.0);
    for (channel, value) in actual[..3].iter().copied().enumerate() {
        assert!(
            (value - expected).abs() <= 1.0e-6,
            "joined channel {channel} ratio {value} differs from truncated 105-code result {expected}"
        );
        assert!(
            (value - rounded).abs() > 1.0e-3,
            "joined channel {channel} used rounded 106-code result"
        );
    }
    assert_eq!(actual[3].to_bits(), 0);
}

#[test]
fn integer_content_boundary_skips_3168_and_admits_3169() {
    let Some((device, queue)) = gpu() else { return };
    let pixels = 800 * 16;
    let base_left = [80u8, 100, 120].repeat(pixels);
    let right = [110u8, 100, 90].repeat(pixels);
    let invalid = vec![0u8; 4 * 212];
    let mut producer = Producer::new(&device, StitchCamera::OneX2).unwrap();
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

    let mut invalid_first = Producer::new(&device, StitchCamera::OneX2).unwrap();
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
    let mut fresh = Producer::new(&device, StitchCamera::OneX2).unwrap();
    let direct = observe(
        &device,
        &queue,
        &mut fresh,
        [&left, &right],
        &clean,
        u32::MAX,
    );
    for failure in [0, 1, u32::MAX - 1] {
        let mut after_failure = Producer::new(&device, StitchCamera::OneX2).unwrap();
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
    let mut sticky = Producer::new(&device, StitchCamera::OneX2).unwrap();
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
        let expected = super::super::spatial::Reference::new()
            .observe_bands([&left, &right], &invalid)
            .unwrap()
            .unwrap()
            .ratios;
        let mut producer = Producer::new(&device, StitchCamera::OneX2).unwrap();
        let actual = observe(
            &device,
            &queue,
            &mut producer,
            [&left, &right],
            &invalid,
            u32::MAX,
        );
        compare(
            &actual,
            [&expected.left.values()[..], &expected.right.values()[..]],
            RATIO_TOLERANCE,
        );
        let identity = vec![[1.0, 1.0, 1.0, 0.0]; MAP_NODES as usize];
        assert!(max_error(&actual, [&identity, &identity]) > RATIO_TOLERANCE);
        assert!(
            max_error(
                &actual,
                [&expected.right.values()[..], &expected.left.values()[..]],
            ) > RATIO_TOLERANCE
        );
        let swizzled: [Vec<[f32; 4]>; 2] = [&expected.left, &expected.right].map(|map| {
            map.values()
                .iter()
                .map(|v| [v[2], v[1], v[0], v[3]])
                .collect()
        });
        assert!(max_error(&actual, [&swizzled[0], &swizzled[1]]) > RATIO_TOLERANCE);
    }
}

#[test]
#[ignore = "requires a saved multi-frame fusion-input sequence"]
fn captured_warm_sequence_matches_reference_on_every_publish_and_hold() {
    let root = PathBuf::from(
        std::env::var_os("KJERAG_FUSION_SEQUENCE_FIXTURE")
            .expect("KJERAG_FUSION_SEQUENCE_FIXTURE must name a fusion-inputs directory"),
    );
    let Some((device, queue)) = gpu() else { return };
    let manifest = std::fs::read_to_string(root.join("manifest.tsv")).unwrap();
    let rows: Vec<_> = manifest
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("frame\t"))
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), 6, "malformed fusion sequence row: {line}");
            (
                fields[0].parse::<u64>().unwrap(),
                fields[1].to_owned(),
                fields[2].to_owned(),
                fields[3].to_owned(),
            )
        })
        .collect();
    assert!(rows.len() > 2, "fusion sequence has no warm observations");
    assert!(
        rows.windows(2).all(|pair| pair[1].0 == pair[0].0 + 1),
        "fusion sequence source indices are not contiguous"
    );
    let observations = rows.len();
    let first_source = rows.first().unwrap().0;
    let last_source = rows.last().unwrap().0;

    let mut producer = Producer::new(&device, StitchCamera::CalibratedMei).unwrap();
    let mut reference = super::super::spatial::Reference::for_camera(StitchCamera::CalibratedMei);
    let mut held = None;
    let mut previous_actual = None;
    let mut admissions = 0;
    let mut holds = 0;
    let mut worst_error = 0.0f32;
    for (frame, left, right, invalid) in rows {
        let left = std::fs::read(root.join(left)).unwrap();
        let right = std::fs::read(root.join(right)).unwrap();
        let invalid = std::fs::read(root.join(invalid)).unwrap();
        let reference_output = reference.observe_bands([&left, &right], &invalid).unwrap();
        let admitted = reference_output.is_some();
        if let Some(output) = reference_output {
            admissions += 1;
            held = Some(output.ratios);
        } else {
            holds += 1;
        }
        let expected = held
            .as_ref()
            .expect("the first fusion sequence observation was not admitted");
        let actual = observe(
            &device,
            &queue,
            &mut producer,
            [&left, &right],
            &invalid,
            u32::MAX,
        );
        let worst = max_error(
            &actual,
            [&expected.left.values()[..], &expected.right.values()[..]],
        );
        worst_error = worst_error.max(worst);
        assert!(
            worst <= RATIO_TOLERANCE,
            "frame {frame} GPU/reference worst ratio error {worst} exceeds {RATIO_TOLERANCE}"
        );
        if !admitted {
            assert_bit_exact_hold(
                frame,
                previous_actual
                    .as_ref()
                    .expect("the first fusion sequence observation was held"),
                &actual,
            );
        }
        previous_actual = Some(actual);
    }
    assert!(admissions > 1, "fusion sequence has no warm update");
    assert!(
        admissions < observations,
        "fusion sequence has no retained hold"
    );
    eprintln!(
        "fusion warm sequence sources={first_source}..={last_source} observations={observations} admissions={admissions} holds={holds} worst_error={worst_error}"
    );
}

/// Replay one exact captured sequence twice. The control preserves ordinary
/// sparse admission; the diagnostic clears only the admission-initialized
/// word before each observation, forcing that observation through the
/// existing solve without resetting its metric, seed or warm-field history.
#[test]
#[ignore = "requires a saved multi-frame fusion-input sequence and output directory"]
fn captured_x4_sequence_isolates_sparse_admission() {
    let root = PathBuf::from(
        std::env::var_os("KJERAG_FUSION_SEQUENCE_FIXTURE")
            .expect("KJERAG_FUSION_SEQUENCE_FIXTURE must name a fusion-inputs directory"),
    );
    let output = PathBuf::from(
        std::env::var_os("KJERAG_FUSION_FORCE_ADMIT_DIR")
            .expect("KJERAG_FUSION_FORCE_ADMIT_DIR must name a new output directory"),
    );
    std::fs::create_dir(&output).expect("force-admit output must be a new directory");
    let normal_dir = output.join("normal");
    let forced_dir = output.join("force-admit");
    std::fs::create_dir(&normal_dir).unwrap();
    std::fs::create_dir(&forced_dir).unwrap();
    let review_100 = std::env::var_os("KJERAG_FUSION_REVIEW_100_BUDGET").is_some();
    let normal_100_dir = output.join("normal-100");
    let forced_100_dir = output.join("force-admit-100");
    if review_100 {
        std::fs::create_dir(&normal_100_dir).unwrap();
        std::fs::create_dir(&forced_100_dir).unwrap();
    }

    let manifest = std::fs::read_to_string(root.join("manifest.tsv")).unwrap();
    let rows: Vec<_> = manifest
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("frame\t"))
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), 6, "malformed fusion sequence row: {line}");
            (
                fields[0].parse::<u64>().unwrap(),
                fields[1].to_owned(),
                fields[2].to_owned(),
                fields[3].to_owned(),
            )
        })
        .collect();
    assert!(!rows.is_empty(), "fusion sequence has no observations");
    assert!(
        rows.windows(2).all(|pair| pair[1].0 == pair[0].0 + 1),
        "fusion sequence source indices are not contiguous"
    );
    let retained = root
        .parent()
        .expect("fusion-inputs directory has no retained-output parent");
    let Some((device, queue)) = gpu() else { return };
    let mut normal = Producer::new(&device, StitchCamera::CalibratedMei).unwrap();
    let mut forced = Producer::new(&device, StitchCamera::CalibratedMei).unwrap();
    normal.state = diagnostic_state(&device, "normal fusion replay state");
    forced.state = diagnostic_state(&device, "force-admit fusion replay state");
    let mut normal_100 = review_100.then(|| {
        let mut producer = Producer::new(&device, StitchCamera::CalibratedMei).unwrap();
        producer.state = diagnostic_state(&device, "normal 100-step fusion replay state");
        replace_solve_with_100_step_review(&device, &mut producer);
        producer
    });
    let mut forced_100 = review_100.then(|| {
        let mut producer = Producer::new(&device, StitchCamera::CalibratedMei).unwrap();
        producer.state = diagnostic_state(&device, "force-admit 100-step fusion replay state");
        replace_solve_with_100_step_review(&device, &mut producer);
        producer
    });
    let mut states =
        std::io::BufWriter::new(std::fs::File::create_new(output.join("state.tsv")).unwrap());
    writeln!(
        states,
        "frame\tnormal_initialized\tnormal_changed\tnormal_solve_active\tnormal_current_metric\tnormal_retained_metric\tnormal_retained_budget\tnormal_first_valid\tnormal_seeds\tnormal_control_valid\tforce_initialized\tforce_changed\tforce_solve_active\tforce_current_metric\tforce_retained_metric\tforce_retained_budget\tforce_first_valid\tforce_seeds\tforce_control_valid"
    )
    .unwrap();
    let mut budget_deltas = review_100.then(|| {
        let mut log = std::io::BufWriter::new(
            std::fs::File::create_new(output.join("budget-100-max-delta.tsv")).unwrap(),
        );
        writeln!(
            log,
            "frame\tnormal_100_vs_normal\tforce_admit_100_vs_force_admit"
        )
        .unwrap();
        log
    });

    for (ordinal, (frame, left, right, invalid)) in rows.into_iter().enumerate() {
        let left = std::fs::read(root.join(left)).unwrap();
        let right = std::fs::read(root.join(right)).unwrap();
        let invalid = std::fs::read(root.join(invalid)).unwrap();
        let normal_ratios = observe(
            &device,
            &queue,
            &mut normal,
            [&left, &right],
            &invalid,
            u32::MAX,
        );
        for (lens, name) in ["left", "right"].into_iter().enumerate() {
            let expected =
                std::fs::read(retained.join(format!("frame-{frame:010}.fusion-{name}.float4")))
                    .unwrap();
            assert_eq!(
                float4_as_bytes(&normal_ratios[lens]),
                expected.as_slice(),
                "normal replay differs from retained GPU output at frame {frame} lens {name}"
            );
        }
        write_ratios(&normal_dir, frame, &normal_ratios);
        let normal_state = read_replay_state(&device, &queue, &normal.state);

        queue.write_buffer(&forced.state, 18 * 4, words_as_bytes(&[0]));
        let forced_ratios = observe(
            &device,
            &queue,
            &mut forced,
            [&left, &right],
            &invalid,
            u32::MAX,
        );
        if ordinal == 0 {
            for lens in 0..2 {
                assert_eq!(
                    float4_as_bytes(&forced_ratios[lens]),
                    float4_as_bytes(&normal_ratios[lens]),
                    "forcing the already-cold first observation changed lens {lens}"
                );
            }
        }
        write_ratios(&forced_dir, frame, &forced_ratios);
        let forced_state = read_replay_state(&device, &queue, &forced.state);
        assert_eq!(
            forced_state[0], 1,
            "frame {frame} did not restore initialized state"
        );
        assert_eq!(forced_state[1], 1, "frame {frame} was not force-admitted");
        if let (Some(normal_100), Some(forced_100), Some(budget_deltas)) = (
            normal_100.as_mut(),
            forced_100.as_mut(),
            budget_deltas.as_mut(),
        ) {
            let normal_100_ratios = observe(
                &device,
                &queue,
                normal_100,
                [&left, &right],
                &invalid,
                u32::MAX,
            );
            queue.write_buffer(&forced_100.state, 18 * 4, words_as_bytes(&[0]));
            let forced_100_ratios = observe(
                &device,
                &queue,
                forced_100,
                [&left, &right],
                &invalid,
                u32::MAX,
            );
            if ordinal == 0 {
                for lens in 0..2 {
                    assert_eq!(
                        float4_as_bytes(&normal_100_ratios[lens]),
                        float4_as_bytes(&normal_ratios[lens]),
                        "100-step normal cold output changed lens {lens}"
                    );
                    assert_eq!(
                        float4_as_bytes(&forced_100_ratios[lens]),
                        float4_as_bytes(&forced_ratios[lens]),
                        "100-step force-admit cold output changed lens {lens}"
                    );
                }
            }
            write_ratios(&normal_100_dir, frame, &normal_100_ratios);
            write_ratios(&forced_100_dir, frame, &forced_100_ratios);
            let normal_delta = max_error(
                &normal_100_ratios,
                [&normal_ratios[0][..], &normal_ratios[1][..]],
            );
            let forced_delta = max_error(
                &forced_100_ratios,
                [&forced_ratios[0][..], &forced_ratios[1][..]],
            );
            writeln!(budget_deltas, "{frame}\t{normal_delta}\t{forced_delta}").unwrap();
        }
        writeln!(
            states,
            "{frame}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            normal_state[0],
            normal_state[1],
            normal_state[2],
            normal_state[3],
            normal_state[4],
            normal_state[5],
            normal_state[6],
            normal_state[7],
            normal_state[8],
            forced_state[0],
            forced_state[1],
            forced_state[2],
            forced_state[3],
            forced_state[4],
            forced_state[5],
            forced_state[6],
            forced_state[7],
            forced_state[8],
        )
        .unwrap();
    }
    states.flush().unwrap();
    if let Some(log) = budget_deltas.as_mut() {
        log.flush().unwrap();
    }
}

/// Replace only the test-owned solve pipeline. This is a non-production
/// counterfactual and makes no claim about Studio's iteration policy.
fn replace_solve_with_100_step_review(device: &wgpu::Device, producer: &mut Producer) {
    const SELECTED_BUDGET: &str =
        "let budget = select(state[RETAINED_BUDGET], 100u, state[SEEDS] == 0u);";
    let source = include_str!("../gpu.wgsl");
    assert_eq!(
        source.matches(SELECTED_BUDGET).count(),
        1,
        "100-step review did not find exactly one selected solve budget"
    );
    let source = source.replacen(SELECTED_BUDGET, "let budget = 100u;", 1);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("image fusion 100-step review solve"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("image fusion 100-step review solve"),
        bind_group_layouts: &[&producer.layout],
        immediate_size: 0,
    });
    assert_eq!(STAGES[3], "solve");
    producer.pipelines[3] = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("image fusion 100-step review solve"),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("solve"),
        compilation_options: Default::default(),
        cache: None,
    });
}

fn diagnostic_state(device: &wgpu::Device, label: &str) -> wgpu::Buffer {
    let mut state = vec![0u32; STATE_WORDS];
    state[22] = 20;
    state[23] = 1;
    state[24] = 1;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: words_as_bytes(&state),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
    })
}

fn write_ratios(output: &std::path::Path, frame: u64, ratios: &[Vec<[f32; 4]>; 2]) {
    for (lens, name) in ["left", "right"].into_iter().enumerate() {
        std::fs::write(
            output.join(format!("frame-{frame:010}.fusion-{name}.float4")),
            float4_as_bytes(&ratios[lens]),
        )
        .unwrap();
    }
}

/// Read state words 18 through 26 after an observation has completed.
fn read_replay_state(device: &wgpu::Device, queue: &wgpu::Queue, state: &wgpu::Buffer) -> [u32; 9] {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fusion replay state readback"),
        size: 9 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("fusion replay state readback"),
    });
    encoder.copy_buffer_to_buffer(state, 18 * 4, &staging, 0, 9 * 4);
    let submission = queue.submit([encoder.finish()]);
    let slice = staging.slice(..);
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
    let words = std::array::from_fn(|index| {
        let at = index * 4;
        u32::from_ne_bytes(mapped[at..at + 4].try_into().unwrap())
    });
    drop(mapped);
    staging.unmap();
    words
}

fn assert_bit_exact_hold(frame: u64, previous: &[Vec<[f32; 4]>; 2], actual: &[Vec<[f32; 4]>; 2]) {
    for lens in 0..2 {
        for (node, (previous, actual)) in previous[lens].iter().zip(&actual[lens]).enumerate() {
            for channel in 0..4 {
                assert_eq!(
                    previous[channel].to_bits(),
                    actual[channel].to_bits(),
                    "held frame {frame} changed GPU ratio at lens {lens}, node {node}, channel {channel}"
                );
            }
        }
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
        let mut producer = Producer::new(&device, StitchCamera::OneX2).unwrap();
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
    let mut producer = Producer::new(&device, StitchCamera::OneX2).unwrap();
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
