//! Authenticated Studio V3 cold-final oracle for the readable ONE X2 solver.
//!
//! The detached corpus records one exact frame-zero cold owner transaction.
//! This test restores the two pre-input-blur planes and selected masks, runs
//! [`ColdPair::transition`], and compares every output-semantic leaf retained
//! by the native transaction.
//!
//! Native also records temporal `cache_bins_u32le`, `cache_low_u8`, and
//! `cache_high_u8`. They are deliberately omitted here: Rust computes the same
//! semantic temporal median by scanning its authenticated histogram and FIFO
//! state, so those incremental-search caches are an implementation
//! optimization rather than output-semantic state.
//!
//! The corpus stays outside Git. Run this ignored test with:
//!
//! ```text
//! KJERAG_STUDIO_COLD_ORACLE_DIR=/path/to/frame0-c46fa87-04 \
//!   cargo test -p kjerag-render --test native_cold_final_oracle \
//!   -- --include-ignored --nocapture
//! ```

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kjerag_render::flow::one_xs::pis::Level;
use kjerag_render::flow::one_xs::scalar::{ColdInputs, ColdPair};
use kjerag_render::flow::one_xs::{Lens, LensPair};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
struct Artifact {
    label: &'static str,
    path: &'static str,
    bytes: usize,
    sha256: &'static str,
}

const TOP_LEVEL: &[Artifact] = &[
    Artifact {
        label: "V8 manifest",
        path: "manifest.json",
        bytes: 73_166,
        sha256: "00994de4839b0dcd38bb825d047c2df4fd379e8756d8c0d5851ad015b4d6d661",
    },
    Artifact {
        label: "cold manifest",
        path: "cold-final-manifest.json",
        bytes: 26_544,
        sha256: "63b75b23b5faa2cc65324edf3fa8f186d4c89a475562a70c726a5fdbd8a081f4",
    },
    Artifact {
        label: "terminal receipt",
        path: "cold-final-completion.receipt.json",
        bytes: 12_529,
        sha256: "7ddbda7052efe5c77f6f703cf32f5d136717d898418bef3c7ad8e6a7e2923a2e",
    },
    Artifact {
        label: "cold evidence",
        path: "cold_final/cold-final-evidence.json",
        bytes: 3_139_552,
        sha256: "b253f2be68f1e3bbe64452b1422854ecc2fa847c8a1d4cfd82d0481567d1552a",
    },
];

const PRECALC_A: Artifact = leaf(
    "precalc A",
    "cold_final/000_precalc_A.bin",
    64_800,
    "c1e404daa371b37a1285bf397370defa6ac162f342145e4dea86edf6118938d9",
);
const PRECALC_B: Artifact = leaf(
    "precalc B",
    "cold_final/001_precalc_B.bin",
    64_800,
    "80151a171e5aab56d63178d1964440b898fc9186bb1d229cf90c3233bf018f15",
);
const MASK_A: Artifact = leaf(
    "selected mask A",
    "00031_selected_final_valid_mask_0.bin",
    64_800,
    "20a85a8bf17041939185190513e14057ef31827a3c343e379ee8deea6f1a6d50",
);
const MASK_B: Artifact = leaf(
    "selected mask B",
    "00032_selected_final_valid_mask_1.bin",
    64_800,
    "20a85a8bf17041939185190513e14057ef31827a3c343e379ee8deea6f1a6d50",
);
const REFERENCE_A: Artifact = leaf(
    "reference A",
    "cold_final/002_reference_A.bin",
    64_800,
    "179ac96cf4a8aa16adec388c7bbcfa944842c7182155cdcc1c1fe2e9dbe10bbc",
);
const REFERENCE_B: Artifact = leaf(
    "reference B",
    "cold_final/003_reference_B.bin",
    64_800,
    "0cbdf7d80ee903c83175a2af57387e77d80d838ade1be5afeaa0d0b5ed00c5ec",
);
const PUBLIC_AB: Artifact = leaf(
    "public AB",
    "cold_final/004_public_ab.bin",
    518_400,
    "e4f499dbe4013c041b6496ac8f9cd8f3cfcd82966ae90d9ed6695e7b69595b98",
);
const PUBLIC_BA: Artifact = leaf(
    "public BA",
    "cold_final/005_public_ba.bin",
    518_400,
    "11d0c356889c3d3ee7b680a1b8967458b9ae0475f2d2eeedd19c6bffdb93e88d",
);
const AB_ROWS_120: Artifact = leaf(
    "AB +0x120 rows",
    "cold_final/006_ab_rows_120.bin",
    24,
    "e7ddce7bf0b5e15fdf297b3f361358f489d3126b57f04b6d8b6787a31899a4d9",
);
const BA_ROWS_120: Artifact = leaf(
    "BA +0x120 rows",
    "cold_final/017_ba_rows_120.bin",
    24,
    "48fd3590197582cb83a648977f95a47ff00a74af8f7014de494fcbffc363543f",
);

const AB_HINTS: [[Artifact; 2]; 2] = [
    [
        leaf(
            "AB hint L1 U",
            "cold_final/007_ab_hint_l1_u.bin",
            64_800,
            "5dbfe1e5abddd0cb2c32fa1ee18432b578b0b8cc7b6df0ef5f0b1b2681d5d2d8",
        ),
        leaf(
            "AB hint L1 V",
            "cold_final/009_ab_hint_l1_v.bin",
            64_800,
            "6eed88e6dea506661505da703e8cabd2ff222e7975afc1bf4cbb39b497da75a5",
        ),
    ],
    [
        leaf(
            "AB hint L2 U",
            "cold_final/008_ab_hint_l2_u.bin",
            16_200,
            "d5eac6897e0853dcceec1bbc51185ae5a67b2c7df8aa428aa7faf04d71d489d0",
        ),
        leaf(
            "AB hint L2 V",
            "cold_final/010_ab_hint_l2_v.bin",
            16_200,
            "407c4914f4dc11565c5288ed04c06873a2def11eb4a15af3bb136d6034261939",
        ),
    ],
];
const BA_HINTS: [[Artifact; 2]; 2] = [
    [
        leaf(
            "BA hint L1 U",
            "cold_final/018_ba_hint_l1_u.bin",
            64_800,
            "2b567758b7508ff8e7662c38bbe695bd74f80a94b97a93126ce167fd20170a9a",
        ),
        leaf(
            "BA hint L1 V",
            "cold_final/020_ba_hint_l1_v.bin",
            64_800,
            "763401b8e589e83ffabfaef76ce4e4cdf2fdbb117f112ee79dba9aacccfba0dd",
        ),
    ],
    [
        leaf(
            "BA hint L2 U",
            "cold_final/019_ba_hint_l2_u.bin",
            16_200,
            "f1fc49c733b3b793842edca2f4540710ea54006db75b997a8bfb8c5c8ce4b4b4",
        ),
        leaf(
            "BA hint L2 V",
            "cold_final/021_ba_hint_l2_v.bin",
            16_200,
            "4f7009b1a44f0ae3c976bfdc6f2960bed7668531b990352ba080776f33d79248",
        ),
    ],
];

const AB_TEMPORAL: [Artifact; 3] = [
    leaf(
        "AB temporal histogram",
        "cold_final/011_ab_temporal_histograms_u8.bin",
        226_416,
        "96393ffdbf6382d565189c00ca2adbc807512c6689c5c18c724647b939f11fa6",
    ),
    leaf(
        "AB temporal offsets",
        "cold_final/012_ab_temporal_history_offsets_u32le.bin",
        5_700,
        "6766b11f7b2e6307feded8bc831e23795470b25fa20f260d5c888fdc03913604",
    ),
    leaf(
        "AB temporal values",
        "cold_final/013_ab_temporal_history_values_f32le.bin",
        17_088,
        "561039ce6547495946ef82628a8a9a9db7fce19bed4807d99edc778de020cae6",
    ),
];
const BA_TEMPORAL: [Artifact; 3] = [
    leaf(
        "BA temporal histogram",
        "cold_final/022_ba_temporal_histograms_u8.bin",
        226_416,
        "8fdc7d4cf158519076a8fc612203f6b0c3326a9112cc171533d01f4aa6fbec25",
    ),
    leaf(
        "BA temporal offsets",
        "cold_final/023_ba_temporal_history_offsets_u32le.bin",
        5_700,
        "6766b11f7b2e6307feded8bc831e23795470b25fa20f260d5c888fdc03913604",
    ),
    leaf(
        "BA temporal values",
        "cold_final/024_ba_temporal_history_values_f32le.bin",
        17_088,
        "950dcd5db4357ca84d81593322d97e3316335d30b7d72db309d598673acea8a3",
    ),
];

const fn leaf(
    label: &'static str,
    path: &'static str,
    bytes: usize,
    sha256: &'static str,
) -> Artifact {
    Artifact {
        label,
        path,
        bytes,
        sha256,
    }
}

fn corpus() -> PathBuf {
    std::env::var_os("KJERAG_STUDIO_COLD_ORACLE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            panic!(
                "set KJERAG_STUDIO_COLD_ORACLE_DIR to the detached V3 cold-final corpus directory"
            )
        })
}

fn authenticate(root: &Path, artifact: Artifact) -> Vec<u8> {
    let path = root.join(artifact.path);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("{} ({}): {error}", artifact.label, path.display()));
    assert_eq!(
        bytes.len(),
        artifact.bytes,
        "{} byte count differs",
        artifact.label
    );
    let actual =
        Sha256::digest(&bytes)
            .iter()
            .fold(String::with_capacity(64), |mut output, byte| {
                write!(output, "{byte:02x}").expect("writing to a String cannot fail");
                output
            });
    assert_eq!(
        actual, artifact.sha256,
        "{} SHA-256 differs",
        artifact.label
    );
    bytes
}

fn assert_bytes(label: &str, actual: &[u8], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len(), "{label} length differs");
    if let Some(index) = actual.iter().zip(expected).position(|(a, b)| a != b) {
        panic!(
            "{label} byte {index} differs: native 0x{:02x}, Rust 0x{:02x}",
            expected[index], actual[index]
        );
    }
}

fn assert_f32le(label: &str, actual: &[f32], expected: &[u8]) {
    assert_eq!(
        expected.len(),
        actual.len() * 4,
        "{label} native byte count differs"
    );
    for (index, (actual, expected)) in actual.iter().zip(expected.chunks_exact(4)).enumerate() {
        let native = u32::from_le_bytes(expected.try_into().expect("four-byte f32 chunk"));
        assert_eq!(
            actual.to_bits(),
            native,
            "{label} value {index} differs: native bits 0x{native:08x}, Rust bits 0x{:08x}",
            actual.to_bits()
        );
    }
}

fn assert_u32le(label: &str, actual: &[u32], expected: &[u8]) {
    assert_eq!(
        expected.len(),
        actual.len() * 4,
        "{label} native byte count differs"
    );
    for (index, (actual, expected)) in actual.iter().zip(expected.chunks_exact(4)).enumerate() {
        let native = u32::from_le_bytes(expected.try_into().expect("four-byte u32 chunk"));
        assert_eq!(*actual, native, "{label} value {index} differs");
    }
}

fn assert_interleaved_field(label: &str, dcol: &[f32], drow: &[f32], expected: &[u8]) {
    assert_eq!(dcol.len(), drow.len(), "{label} component lengths differ");
    assert_eq!(
        expected.len(),
        dcol.len() * 8,
        "{label} native byte count differs"
    );
    for (index, ((dcol, drow), expected)) in dcol
        .iter()
        .zip(drow)
        .zip(expected.chunks_exact(8))
        .enumerate()
    {
        let native_col = u32::from_le_bytes(expected[0..4].try_into().unwrap());
        let native_row = u32::from_le_bytes(expected[4..8].try_into().unwrap());
        assert_eq!(
            dcol.to_bits(),
            native_col,
            "{label} node {index} dcol differs"
        );
        assert_eq!(
            drow.to_bits(),
            native_row,
            "{label} node {index} drow differs"
        );
    }
}

fn canonical_lsb0_rows(rows: &[bool]) -> Vec<u8> {
    let mut bytes = vec![0; rows.len().div_ceil(64) * 8];
    for (bit, set) in rows.iter().copied().enumerate() {
        if set {
            bytes[bit / 8] |= 1 << (bit % 8);
        }
    }
    bytes
}

#[test]
#[ignore = "requires KJERAG_STUDIO_COLD_ORACLE_DIR with the detached V3 cold-final corpus"]
fn cold_transition_matches_authenticated_studio_v3_final_state() {
    let root = corpus();
    for artifact in TOP_LEVEL {
        authenticate(&root, *artifact);
    }

    let inputs = ColdInputs::from_before_input_blur(
        LensPair {
            a: authenticate(&root, PRECALC_A),
            b: authenticate(&root, PRECALC_B),
        },
        LensPair {
            a: authenticate(&root, MASK_A),
            b: authenticate(&root, MASK_B),
        },
    )
    .expect("authenticated cold inputs have the selected fixed shape");

    let transition = ColdPair::new().transition(&inputs);
    let actual = transition.candidate_next;

    assert_bytes(
        "reference A",
        actual.references.lens(Lens::A),
        &authenticate(&root, REFERENCE_A),
    );
    assert_bytes(
        "reference B",
        actual.references.lens(Lens::B),
        &authenticate(&root, REFERENCE_B),
    );
    assert_interleaved_field(
        "public AB",
        actual.a_to_b_public.dcol(),
        actual.a_to_b_public.drow(),
        &authenticate(&root, PUBLIC_AB),
    );
    assert_interleaved_field(
        "public BA",
        actual.b_to_a_public.dcol(),
        actual.b_to_a_public.drow(),
        &authenticate(&root, PUBLIC_BA),
    );

    assert!(
        actual.a_to_b_work_rows.small_disparity_rows().is_none(),
        "AB +0x108 must remain absent after cold counts 0, 1, and 2"
    );
    assert!(
        actual.b_to_a_work_rows.small_disparity_rows().is_none(),
        "BA +0x108 must remain absent after cold counts 0, 1, and 2"
    );
    assert_bytes(
        "AB +0x120 rows",
        &canonical_lsb0_rows(actual.a_to_b_work_rows.lack_of_texture_rows()),
        &authenticate(&root, AB_ROWS_120),
    );
    assert_bytes(
        "BA +0x120 rows",
        &canonical_lsb0_rows(actual.b_to_a_work_rows.lack_of_texture_rows()),
        &authenticate(&root, BA_ROWS_120),
    );

    for (level, artifacts) in [(Level::One, AB_HINTS[0]), (Level::Two, AB_HINTS[1])] {
        let hint = actual.a_to_b_hints.level(level);
        assert_f32le(
            artifacts[0].label,
            hint.dcol(),
            &authenticate(&root, artifacts[0]),
        );
        assert_f32le(
            artifacts[1].label,
            hint.drow(),
            &authenticate(&root, artifacts[1]),
        );
    }
    for (level, artifacts) in [(Level::One, BA_HINTS[0]), (Level::Two, BA_HINTS[1])] {
        let hint = actual.b_to_a_hints.level(level);
        assert_f32le(
            artifacts[0].label,
            hint.dcol(),
            &authenticate(&root, artifacts[0]),
        );
        assert_f32le(
            artifacts[1].label,
            hint.drow(),
            &authenticate(&root, artifacts[1]),
        );
    }

    assert_bytes(
        AB_TEMPORAL[0].label,
        actual.a_to_b_median.histogram(),
        &authenticate(&root, AB_TEMPORAL[0]),
    );
    assert_u32le(
        AB_TEMPORAL[1].label,
        actual.a_to_b_median.offsets(),
        &authenticate(&root, AB_TEMPORAL[1]),
    );
    assert_f32le(
        AB_TEMPORAL[2].label,
        actual.a_to_b_median.values(),
        &authenticate(&root, AB_TEMPORAL[2]),
    );
    assert_bytes(
        BA_TEMPORAL[0].label,
        actual.b_to_a_median.histogram(),
        &authenticate(&root, BA_TEMPORAL[0]),
    );
    assert_u32le(
        BA_TEMPORAL[1].label,
        actual.b_to_a_median.offsets(),
        &authenticate(&root, BA_TEMPORAL[1]),
    );
    assert_f32le(
        BA_TEMPORAL[2].label,
        actual.b_to_a_median.values(),
        &authenticate(&root, BA_TEMPORAL[2]),
    );

    assert_eq!(
        actual.a_to_b_cadence.calc_count(),
        3,
        "AB cold calculation counter differs"
    );
    assert_eq!(
        actual.b_to_a_cadence.calc_count(),
        3,
        "BA cold calculation counter differs"
    );
    assert_eq!(actual.a_to_b_cadence.cadence(), 10, "AB cadence differs");
    assert_eq!(actual.b_to_a_cadence.cadence(), 10, "BA cadence differs");
}
