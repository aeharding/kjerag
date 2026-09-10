use super::{Error, build};
use sha2::{Digest as _, Sha256};
use std::{fs, path::PathBuf};

#[test]
fn preserves_base_when_no_reduction_is_requested() {
    let levels = build(&[1, 2, 3], 3, 1, 1).unwrap();
    assert_eq!(levels[0].pixels, [1, 2, 3]);
}

#[test]
fn matches_fixed_two_pass_rounding_example() {
    let base = [
        0, 37, 74, 111, 148, 185, 222, 3, 61, 111, 161, 211, 5, 55, 105, 155, 122, 185, 248, 55,
        118, 181, 244, 51, 183, 3, 79, 155, 231, 51, 127, 203, 244, 77, 166, 255, 88, 177, 10, 99,
        49, 151, 253, 99, 201, 47, 149, 251, 110, 225, 84, 199, 58, 173, 32, 147, 171, 43, 171, 43,
        171, 43, 171, 43,
    ];
    let levels = build(&base, 8, 8, 2).unwrap();
    assert_eq!(levels[1].width, 4);
    assert_eq!(levels[1].height, 4);
    assert_eq!(
        levels[1].pixels,
        [
            53, 124, 115, 122, 124, 143, 134, 141, 131, 166, 129, 128, 138, 125, 112, 99
        ]
    );
}

#[test]
fn rounds_pair_average_edges_up() {
    let levels = build(&[0, 1, 2, 3], 2, 2, 2).unwrap();
    // Vertical: [1, 2], then horizontal: (1 + 2 + 1) / 2.
    assert_eq!(levels[1].pixels, [2]);
}

#[test]
fn rejects_an_odd_level_only_when_it_must_be_reduced() {
    assert!(build(&[0; 15], 5, 3, 1).is_ok());
    assert_eq!(
        build(&[0; 15], 5, 3, 2),
        Err(Error::OddReduction {
            level: 0,
            width: 5,
            height: 3,
        })
    );
}

#[test]
fn rejects_invalid_shape_and_level_count() {
    assert_eq!(
        build(&[0; 3], 2, 2, 1),
        Err(Error::Length {
            expected: 4,
            actual: 3,
        })
    );
    assert_eq!(build(&[0; 4], 2, 2, 0), Err(Error::EmptyPyramid));
    assert_eq!(build(&[], 0, 2, 1), Err(Error::EmptyDimensions));
    assert_eq!(build(&[], usize::MAX, 2, 1), Err(Error::DimensionOverflow));
    assert!(matches!(
        build(&[0; 4], 2, 2, usize::MAX),
        Err(Error::OddReduction { .. })
    ));
}

#[test]
#[ignore = "requires authenticated local Studio motion captures"]
fn matches_every_logical_pixel_in_the_native_seven_frame_capture() {
    for (name, expected) in native_captures() {
        let base = &expected[0];
        let actual = build(&base.pixels, base.width, base.height, expected.len()).unwrap();
        for (index, (a, b)) in actual.iter().zip(&expected).enumerate() {
            assert_eq!((a.width, a.height), (b.width, b.height));
            assert_eq!(
                a.pixels.iter().zip(&b.pixels).position(|(a, b)| a != b),
                None,
                "{name} level {index} first differing pixel"
            );
        }
    }
}

/// Shared CPU/GPU fixture authority: hash-sealed native pixels, not a
/// reconstruction using either implementation under test.
pub(super) fn native_captures() -> impl Iterator<Item = (&'static str, Vec<super::Level>)> {
    const WIDTH: usize = 3840;
    const HEIGHT: usize = 1920;
    const LEVEL_COUNT: usize = 7;
    const RECORDS: [(&str, &str); 7] = [
        (
            "current-super-0.bin",
            "2931cc96bafd9c03f8ee80e545f258b9f3d281412316c328adc3eb807ddbe337",
        ),
        (
            "reference-0-super-0.bin",
            "2950b95e19085249b27efde3d92afa8317c8bd5219d4a0ccdcebcb5f30a9eadd",
        ),
        (
            "reference-1-super-0.bin",
            "e5af9029b7a1ee468cb6db41f7c288adc1d0d5e2473cb04357440a17ab7f9468",
        ),
        (
            "reference-2-super-0.bin",
            "79666820492462a23f762276f234f68e0e90ee2e5b5b9c854fadecb6b0700b63",
        ),
        (
            "reference-3-super-0.bin",
            "80c8fffac488c16e6db256c8337ddc8e034c66ab8a217a6f25267400343834a0",
        ),
        (
            "reference-4-super-0.bin",
            "786db1150110a83dd62d2fffcde14c790331b37a17aba1caf9f9fc60f0d71e31",
        ),
        (
            "reference-5-super-0.bin",
            "17b9fbce46e1dab6c5a1ce09604ff5ea77c094e841170c4b409749c0c8e9d07a",
        ),
    ];
    let capture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../scratch/studio-seam-flicker-612-20260909-01/temporal-motion-capture-01/run-01",
    );
    let packed_height: usize = (0..LEVEL_COUNT).map(|level| HEIGHT >> level).sum();

    RECORDS.into_iter().map(move |(name, expected_hash)| {
        let packed = fs::read(capture_dir.join(name)).unwrap();
        assert_eq!(
            packed.len(),
            WIDTH * packed_height,
            "{name} geometry differs"
        );
        assert_eq!(sha256_hex(&packed), expected_hash, "{name} SHA-256 differs");

        let mut levels = Vec::new();
        let mut row_offset = 0;
        for index in 0..LEVEL_COUNT {
            let width = WIDTH >> index;
            let height = HEIGHT >> index;
            let mut pixels = Vec::with_capacity(width * height);
            for y in 0..height {
                pixels.extend_from_slice(
                    &packed[(row_offset + y) * WIDTH..(row_offset + y) * WIDTH + width],
                );
            }
            levels.push(super::Level {
                width,
                height,
                pixels,
            });
            row_offset += height;
        }
        (name, levels)
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
