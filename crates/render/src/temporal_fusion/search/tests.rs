use super::*;
use sha2::{Digest as _, Sha256};
use std::{fs, path::Path};

fn constant_levels(width: usize, height: usize, value: u8) -> Vec<Level> {
    (0..LEVELS)
        .map(|level| {
            let width = width >> level;
            let height = height >> level;
            Level {
                width,
                height,
                pixels: vec![value; width * height],
            }
        })
        .collect()
}

#[test]
fn selected_contract_is_explicit() {
    assert_eq!(SELECTED_CONFIGURATION.levels, 7);
    assert_eq!(SELECTED_CONFIGURATION.block, [16, 16]);
    assert_eq!(SELECTED_CONFIGURATION.finest_search, "Hex2");
    assert_eq!(SELECTED_CONFIGURATION.finest_radius, 1);
    assert_eq!(SELECTED_CONFIGURATION.coarse_search, "Exhaustive");
    assert_eq!(SELECTED_CONFIGURATION.coarse_radius, 2);
    assert_eq!(SELECTED_CONFIGURATION.lsad, 1_600);
    assert_eq!(SELECTED_CONFIGURATION.penalty_new, 50);
    assert_eq!(SELECTED_CONFIGURATION.penalty_zero, 50);
    assert_eq!(SELECTED_CONFIGURATION.penalty_global, 0);
    assert_eq!(SELECTED_CONFIGURATION.bad_sad, 40_000);
    assert_eq!(SELECTED_CONFIGURATION.bad_range, 24);
}

#[test]
fn equal_images_choose_zero_and_report_zero_sad() {
    let levels = constant_levels(1024, 1024, 73);
    let vectors = selected(&levels, &levels).unwrap();
    assert_eq!(vectors.len(), 64 * 64);
    assert!(vectors.iter().all(|vector| *vector == [0, 0, 0]));
}

#[test]
fn finest_preparation_preserves_the_complete_reference_search() {
    let bytes: Vec<_> = (0..1024usize * 1024)
        .map(|at| ((at.wrapping_mul(37) ^ (at >> 6).wrapping_mul(173)) >> 3) as u8)
        .collect();
    let current = super::super::pyramid::build(&bytes, 1024, 1024, 7).unwrap();
    let shifted: Vec<_> = (0..bytes.len())
        .map(|at| bytes[(at + 29) % bytes.len()])
        .collect();
    let reference = super::super::pyramid::build(&shifted, 1024, 1024, 7).unwrap();
    let input = prepare_finest(&current, &reference).unwrap();
    assert_eq!(
        finish_finest(&current[0], &reference[0], &input),
        selected(&current, &reference).unwrap()
    );
}

#[test]
fn concurrent_finest_preparation_matches_serial_and_preserves_order() {
    let bytes: Vec<_> = (0..1024usize * 1024)
        .map(|at| ((at.wrapping_mul(37) ^ (at >> 6).wrapping_mul(173)) >> 3) as u8)
        .collect();
    let current = super::super::pyramid::build(&bytes, 1024, 1024, LEVELS).unwrap();
    let references: Vec<_> = [7usize, 19, 43, 79, 131, 211]
        .map(|shift| {
            let shifted: Vec<_> = (0..bytes.len())
                .map(|at| bytes[(at + shift) % bytes.len()])
                .collect();
            super::super::pyramid::build(&shifted, 1024, 1024, LEVELS).unwrap()
        })
        .into();
    let serial: Vec<_> = references
        .iter()
        .map(|reference| prepare_finest(&current, reference).unwrap())
        .collect();

    for count in [1, 3, 6] {
        let borrowed: Vec<_> = references[..count].iter().map(Vec::as_slice).collect();
        let concurrent = prepare_finest_ordered(&current, &borrowed).unwrap();
        let coarse_borrowed: Vec<_> = references[..count]
            .iter()
            .map(|reference| &reference[1..])
            .collect();
        let coarse = prepare_finest_ordered_coarse(
            [current[0].width, current[0].height],
            &current[1..],
            &coarse_borrowed,
        )
        .unwrap();
        assert_eq!(concurrent.len(), count);
        assert_eq!(coarse.len(), count);
        for ((actual, coarse), expected) in concurrent.iter().zip(&coarse).zip(&serial[..count]) {
            assert_eq!(actual.global, expected.global);
            assert_eq!(actual.seeds, expected.seeds);
            assert_eq!(coarse.global, expected.global);
            assert_eq!(coarse.seeds, expected.seeds);
        }
    }
    assert!(serial.windows(2).any(|pair| pair[0].seeds != pair[1].seeds));
}

#[test]
fn coarse_only_preparation_needs_no_level_zero_pixels_and_keeps_odd_halving() {
    let finest = [2880, 1440];
    let current = constant_levels(finest[0], finest[1], 73);
    let reference = constant_levels(finest[0], finest[1], 94);
    let expected = prepare_finest(&current, &reference).unwrap();
    let actual = prepare_finest_ordered_coarse(finest, &current[1..], &[&reference[1..]])
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(actual.global, expected.global);
    assert_eq!(actual.seeds, expected.seeds);
}

#[test]
fn concurrent_finest_preparation_rejects_count_and_inputs_before_work() {
    let current = constant_levels(1024, 1024, 73);
    assert!(matches!(
        prepare_finest_ordered(&current, &[]),
        Err(Error::ReferenceCount(0))
    ));
    let seven: Vec<_> = (0..7).map(|_| current.as_slice()).collect();
    assert!(matches!(
        prepare_finest_ordered(&current, &seven),
        Err(Error::ReferenceCount(7))
    ));

    let valid = current.clone();
    let mut invalid = current.clone();
    invalid.pop();
    assert!(matches!(
        prepare_finest_ordered(&current, &[valid.as_slice(), invalid.as_slice()]),
        Err(Error::LevelCount { reference: 6, .. })
    ));

    assert!(matches!(
        prepare_finest_ordered_coarse(
            [current[0].width, current[0].height],
            &current[1..],
            &[&invalid[1..]],
        ),
        Err(Error::CoarseLevelCount { reference: 5, .. })
    ));
    assert!(matches!(
        prepare_finest_ordered_coarse([2048, 1024], &current[1..], &[&valid[1..]]),
        Err(Error::Geometry { level: 1, .. })
    ));
}

#[test]
fn ordered_workers_return_the_first_error_and_resume_panics() {
    let error: Result<Vec<usize>, Error> = run_ordered(4, |index| match index {
        1 => Err(Error::OutputCostOverflow(11)),
        2 => Err(Error::OutputCostOverflow(22)),
        _ => Ok(index),
    });
    assert_eq!(error, Err(Error::OutputCostOverflow(11)));

    let panic = std::panic::catch_unwind(|| {
        let _: Result<Vec<usize>, Error> = run_ordered(3, |index| {
            if index == 1 {
                panic!("bounded worker panic");
            }
            Ok(index)
        });
    });
    assert!(panic.is_err());
}

/// The same serial finest-level controller, exposed only to GPU tests so
/// adversarial seeds can exercise bounds, ties and the adaptive UMH branch.
pub(super) fn finish_finest(
    current: &Level,
    reference: &Level,
    input: &FinestInput,
) -> Vec<[i32; 3]> {
    let mut plane = Plane {
        current,
        reference,
        blocks_x: current.width / BLOCK,
        blocks_y: current.height / BLOCK,
        vectors: input
            .seeds
            .iter()
            .map(|record| Vector {
                x: record[0],
                y: record[1],
                sad: i64::from(record[2]),
            })
            .collect(),
        smallest: false,
        global: Vector {
            x: input.global[0],
            y: input.global[1],
            sad: -1,
        },
        bad_count: 0,
    };
    plane.search(Refine::HexTwoOne).unwrap();
    raw_records(plane.vectors).unwrap()
}

#[test]
fn parallel_references_preserve_each_serial_result_and_input_order() {
    let current = constant_levels(1024, 1024, 73);
    let references = [10, 42, 73, 94, 121, 255].map(|value| constant_levels(1024, 1024, value));
    let parallel = selected_six(&current, references.each_ref().map(Vec::as_slice)).unwrap();
    for (reference, actual) in references.iter().zip(parallel) {
        let expected = selected(&current, reference).unwrap();
        assert_eq!(actual, expected);
    }
}

#[test]
fn parallel_references_validate_every_input_before_spawning() {
    let current = constant_levels(1024, 1024, 73);
    let mut references = std::array::from_fn::<_, 6, _>(|_| current.clone());
    references[4].pop();
    assert!(matches!(
        selected_six(&current, references.each_ref().map(Vec::as_slice)),
        Err(Error::LevelCount { .. })
    ));
}

#[test]
fn inclusive_seed_clip_and_strict_candidate_boundary_are_distinct() {
    let bounds = Bounds {
        min_x: -16,
        max_x: 16,
        min_y: -8,
        max_y: 8,
    };
    assert_eq!(
        bounds.clip(Vector {
            x: 17,
            y: 9,
            sad: 3
        }),
        Vector {
            x: 16,
            y: 8,
            sad: 3
        }
    );
    assert!(!bounds.candidate(16, 0));
    assert!(!bounds.candidate(0, 8));
    assert!(bounds.candidate(15, 7));
}

#[test]
fn interpolation_preserves_public_integer_weighting_and_edges() {
    let coarse = [
        Vector {
            x: -3,
            y: 1,
            sad: 7,
        },
        Vector {
            x: 5,
            y: 2,
            sad: 11,
        },
        Vector {
            x: 9,
            y: 3,
            sad: 13,
        },
        Vector {
            x: 13,
            y: 4,
            sad: 17,
        },
    ];
    let fine = interpolate(&coarse, [2, 2], [4, 4]).unwrap();
    assert_eq!(
        fine[0],
        Vector {
            x: -6,
            y: 2,
            sad: 7
        }
    );
    assert_eq!(fine[5].x, (9 * -3 + 3 * 5 + 3 * 9 + 13) >> 3);
    assert_eq!(fine[5].sad, (9 * 7 + 3 * 11 + 3 * 13 + 17 + 8) >> 4);
    assert_eq!(
        fine[15],
        Vector {
            x: 26,
            y: 8,
            sad: 17
        }
    );
}

#[test]
fn global_mode_uses_lowest_value_on_equal_frequency() {
    let vectors = [Vector { x: 2, y: 4, sad: 0 }, Vector { x: 3, y: 5, sad: 0 }];
    // Both histogram bins occur once; the public scan retains the lower one.
    // Both values are then within the six-pixel averaging window.
    assert_eq!(
        estimate_global_doubled(&vectors).unwrap(),
        Vector {
            x: 5,
            y: 9,
            sad: -1
        }
    );
}

#[test]
fn global_mean_rejects_anisotropic_outliers_jointly() {
    let vectors = [
        Vector { x: 0, y: 0, sad: 0 },
        Vector { x: 0, y: 0, sad: 0 },
        Vector { x: 0, y: 0, sad: 0 },
        Vector {
            x: 5,
            y: 100,
            sad: 0,
        },
        Vector {
            x: 5,
            y: 100,
            sad: 0,
        },
        Vector {
            x: 100,
            y: 5,
            sad: 0,
        },
        Vector {
            x: 100,
            y: 5,
            sad: 0,
        },
    ];
    // The modes are (0, 0). Component-wise filtering would include the four
    // anisotropic points and return (4, 4); the source admits only the three
    // records that satisfy both component bounds.
    assert_eq!(
        estimate_global_doubled(&vectors).unwrap(),
        Vector {
            x: 0,
            y: 0,
            sad: -1
        }
    );
}

#[test]
fn rejects_unselected_pyramid_shapes() {
    let current = constant_levels(1024, 1024, 0);
    let mut reference = current.clone();
    reference.pop();
    assert!(matches!(
        selected(&current, &reference),
        Err(Error::LevelCount { .. })
    ));

    let mut reference = current.clone();
    reference[3].width += 1;
    assert!(matches!(
        selected(&current, &reference),
        Err(Error::Geometry { level: 3, .. })
    ));
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn read_hashed(directory: &Path, name: &str, expected_hash: &str) -> Vec<u8> {
    let bytes = fs::read(directory.join(name)).unwrap();
    assert_eq!(
        sha256(&bytes),
        expected_hash,
        "fixture hash changed for {name}"
    );
    bytes
}

fn captured_levels(bytes: &[u8]) -> Vec<Level> {
    const WIDTH: usize = 3_840;
    const HEIGHT: usize = 1_920;
    assert_eq!(bytes.len(), WIDTH * 3_810);
    let mut row = 0;
    (0..LEVELS)
        .map(|level| {
            let width = WIDTH >> level;
            let height = HEIGHT >> level;
            let mut pixels = Vec::with_capacity(width * height);
            for y in 0..height {
                let start = (row + y) * WIDTH;
                pixels.extend_from_slice(&bytes[start..start + width]);
            }
            row += height;
            Level {
                width,
                height,
                pixels,
            }
        })
        .collect()
}

fn packed_i32x3(records: &[[i32; 3]]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|record| record.iter().flat_map(|lane| lane.to_le_bytes()))
        .collect()
}

#[test]
#[ignore = "requires sealed private inputs and combined adapter outputs"]
fn matches_combined_adapter_for_all_six_saved_references() {
    let fixture = native_fixture();
    let parallel = selected_six(
        &fixture.current,
        fixture.references.each_ref().map(Vec::as_slice),
    )
    .unwrap();
    for (ordinal, reference) in fixture.references.iter().enumerate() {
        let records = selected(&fixture.current, reference).unwrap();
        let actual = packed_i32x3(&records);
        assert_eq!(
            parallel[ordinal], records,
            "parallel reference {ordinal} differs"
        );
        let wanted = &fixture.expected[ordinal];
        assert_eq!(
            actual.len(),
            wanted.len(),
            "reference {ordinal} byte count differs"
        );
        if &actual != wanted {
            let differing: Vec<_> = actual
                .chunks_exact(12)
                .zip(wanted.chunks_exact(12))
                .enumerate()
                .filter_map(|(index, (got, expected))| (got != expected).then_some(index))
                .collect();
            panic!(
                "combined adapter reference {ordinal} differs at {}/{} records; first {:?}; actual SHA-256 {}; expected SHA-256 {}",
                differing.len(),
                records.len(),
                &differing[..differing.len().min(8)],
                sha256(&actual),
                sha256(wanted),
            );
        }
    }
}

pub(in crate::temporal_fusion) struct NativeFixture {
    pub current: Vec<Level>,
    pub references: [Vec<Level>; 6],
    pub expected: [Vec<u8>; 6],
}

#[test]
#[ignore = "requires sealed private motion inputs and an idle CPU for timing"]
fn six_worker_coarse_preparation_is_exact_and_timed() {
    let fixture = native_fixture();
    let serial_started = std::time::Instant::now();
    let serial = fixture
        .references
        .each_ref()
        .map(|reference| prepare_finest(&fixture.current, reference).unwrap());
    eprintln!(
        "coarse benchmark serial six: {:.3}ms",
        serial_started.elapsed().as_secs_f64() * 1000.0
    );
    // Verify complete-search output against the already sealed independent
    // adapter, not just another invocation of this build's coarse function.
    for (ordinal, input) in serial.iter().enumerate() {
        let finished = finish_finest(&fixture.current[0], &fixture.references[ordinal][0], input);
        assert_eq!(packed_i32x3(&finished), fixture.expected[ordinal]);
        let mut bytes = packed_i32x3(&input.seeds);
        bytes.extend(input.global.into_iter().flat_map(i32::to_le_bytes));
        eprintln!("coarse benchmark reference {ordinal}: {}", sha256(&bytes));
    }
    for repeat in 1..=5 {
        let started = std::time::Instant::now();
        let references: Vec<_> = fixture.references.iter().map(Vec::as_slice).collect();
        let parallel = prepare_finest_ordered(&fixture.current, &references).unwrap();
        let elapsed = started.elapsed();
        for (actual, expected) in parallel.iter().zip(&serial) {
            assert_eq!(actual.global, expected.global);
            assert_eq!(actual.seeds, expected.seeds);
        }
        eprintln!(
            "coarse benchmark parallel repeat {repeat}: {:.3}ms",
            elapsed.as_secs_f64() * 1000.0
        );
    }
}

pub(in crate::temporal_fusion) fn native_fixture() -> NativeFixture {
    let capture = std::env::var_os("KJERAG_SEARCH_FIXTURE_DIR")
        .map(std::path::PathBuf::from)
        .expect("set KJERAG_SEARCH_FIXTURE_DIR to temporal-motion-capture-01/run-01");
    let expected = std::env::var_os("KJERAG_SEARCH_EXPECTED_DIR")
        .map(std::path::PathBuf::from)
        .expect("set KJERAG_SEARCH_EXPECTED_DIR to temporal-search-adapter-01/combined");
    read_hashed(
        &capture,
        "events.jsonl",
        "72d7233f89e3ff9749c33eaa405cc408787ae9299260fa15a9f1cf1fba4560c4",
    );
    let current = captured_levels(&read_hashed(
        &capture,
        "current-super-0.bin",
        "2931cc96bafd9c03f8ee80e545f258b9f3d281412316c328adc3eb807ddbe337",
    ));
    let reference_hashes = [
        "2950b95e19085249b27efde3d92afa8317c8bd5219d4a0ccdcebcb5f30a9eadd",
        "e5af9029b7a1ee468cb6db41f7c288adc1d0d5e2473cb04357440a17ab7f9468",
        "79666820492462a23f762276f234f68e0e90ee2e5b5b9c854fadecb6b0700b63",
        "80c8fffac488c16e6db256c8337ddc8e034c66ab8a217a6f25267400343834a0",
        "786db1150110a83dd62d2fffcde14c790331b37a17aba1caf9f9fc60f0d71e31",
        "17b9fbce46e1dab6c5a1ce09604ff5ea77c094e841170c4b409749c0c8e9d07a",
    ];
    let output_hashes = [
        "4d18478cdbfe343a699fb9742f80f654cde7ca4ca5f5c2ab75162f589096e9b7",
        "5f0b4ed94c5c14b904b5823f3a1b65f5ce3654823616672cffad020e81cce863",
        "c9e2eb1732525412df88eef6d9036c2ea089e871fbb8736c800ef924451a232e",
        "7def1fbf7e8578eb89c6453cdb48b176841ded27b7867e04bb26492c826a772a",
        "fe07f8e5da9e5029f0ca4aa46e2acdcd0b22c163da899779b2eff7164691ee59",
        "e79b0904de5ebd55627d678fecf9a98e77ea5818edb4853b76a44cdbee3281b4",
    ];

    let references: [Vec<Level>; 6] = std::array::from_fn(|ordinal| {
        let name = format!("reference-{ordinal}-super-0.bin");
        captured_levels(&read_hashed(&capture, &name, reference_hashes[ordinal]))
    });
    let expected = std::array::from_fn(|ordinal| {
        let expected_name = format!("actual-reference-{ordinal}-raw.bin");
        read_hashed(&expected, &expected_name, output_hashes[ordinal])
    });
    NativeFixture {
        current,
        references,
        expected,
    }
}
