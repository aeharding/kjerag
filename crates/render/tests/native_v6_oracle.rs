//! Native ONE X2 warm solver-boundary oracle.
//!
//! The V6 capture holds, for one exact frame (source PTS 210577.033333 ms), both ends of the
//! selected Flow-On solver: the two 3240-by-180 `CV_8UC1` staging belts that go in, and the two
//! 1080-by-60 `CV_32FC2` fields that come out. The tracked PII-free manifest binds those detached
//! payloads and the deterministic reduction/blur stages by exact byte count and SHA-256.
//!
//! This is intentionally not a complete warm replay oracle. It does not contain the exact source
//! frames, final masks, solver history, intermediate solver state, or production renderer path.
//!
//! Corpus tests are ignored by default because the large owner-approved payloads stay outside
//! Git. Point `KJERAG_STUDIO_ORACLE_DIR` directly at the V6 capture directory, then run:
//!
//! ```text
//! cargo test -p kjerag-render --test native_v6_oracle -- --include-ignored --nocapture
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use kjerag_render::flow::one_xs::temporal::gaussian_blur;
use kjerag_render::flow::one_xs::{self, Lens, LensPair};
use kjerag_render::flow::one_xs_belt::SourceBelts;

const MANIFEST_TEXT: &str = include_str!("../../../docs/research/studio-v6-warm-boundary.oracle");

#[derive(Debug)]
struct Artifact {
    id: String,
    origin: String,
    locator: String,
    bytes: usize,
    dtype: String,
    rows: usize,
    columns: usize,
    channels: usize,
    row_step: usize,
    axes: String,
    components: String,
    units: String,
    sha256: String,
}

#[derive(Debug)]
struct Manifest {
    metadata: BTreeMap<String, String>,
    artifacts: BTreeMap<String, Artifact>,
}

impl Manifest {
    fn parse(text: &str) -> Result<Self, String> {
        let mut metadata = BTreeMap::new();
        let mut artifacts = BTreeMap::new();

        for (index, raw) in text.lines().enumerate() {
            let line_number = index + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("manifest line {line_number} has no '='"))?;
            let key = key.trim();
            let value = value.trim();
            if key != "artifact" {
                if metadata.insert(key.to_owned(), value.to_owned()).is_some() {
                    return Err(format!("manifest line {line_number} repeats {key}"));
                }
                continue;
            }

            let fields: Vec<_> = value.split('|').map(str::trim).collect();
            if fields.len() != 13 {
                return Err(format!(
                    "manifest line {line_number} has {} artifact fields, expected 13",
                    fields.len()
                ));
            }
            let number = |field: usize, name: &str| {
                fields[field].parse::<usize>().map_err(|error| {
                    format!("manifest line {line_number} has invalid {name}: {error}")
                })
            };
            let artifact = Artifact {
                id: fields[0].to_owned(),
                origin: fields[1].to_owned(),
                locator: fields[2].to_owned(),
                bytes: number(3, "byte count")?,
                dtype: fields[4].to_owned(),
                rows: number(5, "row count")?,
                columns: number(6, "column count")?,
                channels: number(7, "channel count")?,
                row_step: number(8, "row step")?,
                axes: fields[9].to_owned(),
                components: fields[10].to_owned(),
                units: fields[11].to_owned(),
                sha256: fields[12].to_owned(),
            };
            let id = artifact.id.clone();
            if artifacts.insert(id.clone(), artifact).is_some() {
                return Err(format!("manifest line {line_number} repeats artifact {id}"));
            }
        }

        Ok(Self {
            metadata,
            artifacts,
        })
    }

    fn artifact(&self, id: &str) -> &Artifact {
        self.artifacts
            .get(id)
            .unwrap_or_else(|| panic!("tracked manifest is missing artifact {id}"))
    }
}

fn manifest() -> Manifest {
    Manifest::parse(MANIFEST_TEXT).expect("tracked V6 oracle manifest must parse")
}

fn corpus() -> PathBuf {
    std::env::var_os("KJERAG_STUDIO_ORACLE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            panic!(
                "set KJERAG_STUDIO_ORACLE_DIR to the detached V6 directory containing the manifest's external files"
            )
        })
}

fn read_external(root: &Path, artifact: &Artifact) -> Vec<u8> {
    assert_eq!(
        artifact.origin, "external",
        "{} is not an external corpus artifact",
        artifact.id
    );
    let path = root.join(&artifact.locator);
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    verify_payload(artifact, &bytes);
    bytes
}

/// A captured `CV_32FC2` payload as (across, along) pairs in native row-major order.
fn read_field(artifact: &Artifact, bytes: &[u8]) -> Vec<[f32; 2]> {
    assert_eq!(artifact.dtype, "f32le");
    assert_eq!(artifact.channels, 2);
    assert_eq!(
        bytes.len(),
        one_xs::ROWS * one_xs::COLS * 2 * 4,
        "{} is not a 1080-by-60 CV_32FC2 payload",
        artifact.id,
    );
    bytes
        .chunks_exact(8)
        .map(|c| {
            [
                f32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                f32::from_le_bytes([c[4], c[5], c[6], c[7]]),
            ]
        })
        .collect()
}

fn verify_payload(artifact: &Artifact, bytes: &[u8]) {
    assert_eq!(
        bytes.len(),
        artifact.bytes,
        "{} byte count differs from the tracked native contract",
        artifact.id
    );
    let actual = sha256_hex(bytes);
    assert_eq!(
        actual, artifact.sha256,
        "{} SHA-256 differs from the tracked native contract",
        artifact.id
    );
}

fn sha256_hex(input: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let bit_len = u64::try_from(input.len())
        .expect("payload length fits u64")
        .checked_mul(8)
        .expect("payload bit length fits u64");
    let mut padded = input.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut schedule = [0_u32; 64];
        for (word, bytes) in schedule[..16].iter_mut().zip(chunk.chunks_exact(4)) {
            *word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for (&constant, &word) in K.iter().zip(&schedule) {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temporary1 = h
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(constant)
                .wrapping_add(word);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temporary2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temporary1);
            d = c;
            c = b;
            b = a;
            a = temporary1.wrapping_add(temporary2);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = (*slot).wrapping_add(value);
        }
    }

    let mut output = String::with_capacity(64);
    for value in state {
        write!(&mut output, "{value:08x}").expect("writing to String cannot fail");
    }
    output
}

fn percentile(sorted: &[f32], q: f64) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[idx]
}

#[test]
fn tracked_manifest_is_strict_small_and_pii_free() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "the dependency-free digest implementation must match the SHA-256 standard vector"
    );

    let manifest = manifest();
    let required_metadata = BTreeMap::from([
        ("schema", "kjerag-studio-oracle/1"),
        ("oracle_id", "studio602-onex2-flow-on-frame6317-v6-boundary"),
        ("scope", "warm_solver_boundary_only"),
        ("studio_version", "6.0.2"),
        (
            "worker_arm64_sha256",
            "0a34f593198a0a7a29239ccded91231af3d7bc94e04213c67a2d44a04076a452",
        ),
        ("camera", "one_x2"),
        ("lens_type", "0x29"),
        ("stitching_optimization", "on"),
        ("output_frame_index", "6317"),
        ("source_pts_ticks", "6317311"),
        ("source_timescale_hz", "30000"),
        ("source_pts_ms", "210577.03333333333"),
        ("process_ordinal_observed", "6312"),
        ("output_width", "1920"),
        ("output_height", "1080"),
        (
            "capture_archive_sha256",
            "f9ef15168459325f5dd1f96ea17f5405dc7753471fa7a8c81d4df12d6d0ff430",
        ),
        (
            "capture_events_sha256",
            "efac13866b8c07d5917d73eb9edc2883b806b321a0457fc60d1edab498aaa7db",
        ),
        (
            "capture_manifest_sha256",
            "7e48275b87e6486f91769f8ad82df525a16e759a4cfd289085ada6c5e1e50b15",
        ),
        (
            "capture_session_sha256",
            "de533a46ffc336381217bf2ca673ce268cb07e1a828fafe6d817f3c402f2a623",
        ),
        ("studio_export_bytes", "860845465"),
        (
            "studio_export_sha256",
            "fb003968e3e56dedbca15f02e855ba8cfeb22ea81f78354e732fefca115e9aed",
        ),
    ]);
    assert_eq!(manifest.metadata.len(), required_metadata.len());
    for (key, expected) in required_metadata {
        assert_eq!(
            manifest.metadata.get(key).map(String::as_str),
            Some(expected),
            "tracked metadata {key} is missing or changed"
        );
    }

    let expected_ids = BTreeSet::from([
        "staging_a",
        "staging_b",
        "preblur_a",
        "preblur_b",
        "postblur_a",
        "postblur_b",
        "ordinary_b_to_a_lens0",
        "ordinary_a_to_b_lens1",
    ]);
    assert_eq!(
        manifest
            .artifacts
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        expected_ids
    );
    for artifact in manifest.artifacts.values() {
        let element_bytes = match artifact.dtype.as_str() {
            "u8" => 1,
            "f32le" => 4,
            other => panic!("{} has unsupported dtype {other}", artifact.id),
        };
        assert_eq!(artifact.bytes, artifact.rows * artifact.row_step);
        assert_eq!(
            artifact.row_step,
            artifact.columns * artifact.channels * element_bytes
        );
        assert_eq!(artifact.sha256.len(), 64);
        assert!(
            artifact
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "{} has a non-canonical SHA-256",
            artifact.id
        );
        assert_eq!(artifact.axes, "along_seam,across_seam");
        assert!(!artifact.components.is_empty());
        assert!(!artifact.units.is_empty());
        match artifact.origin.as_str() {
            "external" => {
                let path = Path::new(&artifact.locator);
                assert!(
                    !artifact.locator.contains(['/', '\\']),
                    "{} locator must not contain a path separator",
                    artifact.id
                );
                assert_eq!(
                    path.components().count(),
                    1,
                    "{} locator must be one detached-corpus filename",
                    artifact.id
                );
                assert!(matches!(
                    path.components().next(),
                    Some(Component::Normal(_))
                ));
            }
            "derived" => assert!(artifact.locator.contains(':')),
            other => panic!("{} has unsupported origin {other}", artifact.id),
        }
    }

    for (id, origin, locator, components, units) in [
        (
            "staging_a",
            "external",
            "00038_target_input_belt_b70.bin",
            "luma",
            "intensity_u8",
        ),
        (
            "staging_b",
            "external",
            "00039_target_input_belt_bd0.bin",
            "luma",
            "intensity_u8",
        ),
        (
            "preblur_a",
            "derived",
            "area3x3:staging_a",
            "luma",
            "intensity_u8",
        ),
        (
            "preblur_b",
            "derived",
            "area3x3:staging_b",
            "luma",
            "intensity_u8",
        ),
        (
            "postblur_a",
            "derived",
            "gaussian5_sigma0p8_reflect101:preblur_a",
            "luma",
            "intensity_u8",
        ),
        (
            "postblur_b",
            "derived",
            "gaussian5_sigma0p8_reflect101:preblur_b",
            "luma",
            "intensity_u8",
        ),
        (
            "ordinary_b_to_a_lens0",
            "external",
            "00043_target_ordinary_flow_left_c90.bin",
            "across_seam,along_seam",
            "final_grid_cells",
        ),
        (
            "ordinary_a_to_b_lens1",
            "external",
            "00044_target_ordinary_flow_right_c30.bin",
            "across_seam,along_seam",
            "final_grid_cells",
        ),
    ] {
        let artifact = manifest.artifact(id);
        assert_eq!(artifact.origin, origin);
        assert_eq!(artifact.locator, locator);
        assert_eq!(artifact.components, components);
        assert_eq!(artifact.units, units);
    }

    let forbidden = [
        "serial",
        "gps",
        "latitude",
        "longitude",
        ".insv",
        "/home/",
        "/users/",
        "/private/",
        "/volumes/",
        "/tmp/",
        "source_media_path",
        "recording_time",
    ];
    let lowercase = MANIFEST_TEXT.to_ascii_lowercase();
    for word in forbidden {
        assert!(
            !lowercase.contains(word),
            "tracked manifest contains {word}"
        );
    }
}

#[test]
#[ignore = "requires KJERAG_STUDIO_ORACLE_DIR with the detached V6 corpus"]
fn captured_staging_belts_reduce_to_the_selected_solver_input() {
    let manifest = manifest();
    let root = corpus();
    let b70 = read_external(&root, manifest.artifact("staging_a"));
    let bd0 = read_external(&root, manifest.artifact("staging_b"));

    for (label, bytes) in [("staging A", &b70), ("staging B", &bd0)] {
        let mean = bytes.iter().map(|&value| f64::from(value)).sum::<f64>() / bytes.len() as f64;
        let nonzero = bytes.iter().filter(|&&value| value != 0).count();
        assert!(
            (116.3..116.4).contains(&mean),
            "{label} mean {mean} does not have the captured luma distribution"
        );
        assert!(
            nonzero > bytes.len() * 9 / 10,
            "{label} is mostly black; the captured belt layout is not present"
        );
    }

    let belts = SourceBelts::from_lenses(LensPair { a: b70, b: bd0 }).expect("staging belt shape");

    // Panotype 5 reduces the staging belts by an exact 3-by-3 INTER_AREA, then applies
    // Gaussian 5-by-5 sigma 0.8 with REFLECT_101. Both steps are RE'd and implemented.
    let solver = belts.reduce_area_3x3();
    let blurred = gaussian_blur(&solver);

    for (lens, preblur_id, postblur_id) in [
        (Lens::A, "preblur_a", "postblur_a"),
        (Lens::B, "preblur_b", "postblur_b"),
    ] {
        assert_eq!(solver.lens(lens).len(), one_xs::ROWS * one_xs::COLS);
        let reduced = solver.lens(lens);
        let smoothed = blurred.lens(lens);
        verify_payload(manifest.artifact(preblur_id), reduced);
        verify_payload(manifest.artifact(postblur_id), smoothed);

        let mean =
            reduced.iter().map(|&value| f64::from(value)).sum::<f64>() / reduced.len() as f64;
        let changed = smoothed
            .iter()
            .zip(reduced)
            .filter(|(after, before)| after != before)
            .count();
        assert!(
            (116.3..116.4).contains(&mean),
            "reduced belt for {lens:?} has unexpected mean {mean}"
        );
        assert!(
            changed > reduced.len() / 2 && changed < reduced.len() * 2 / 3,
            "Gaussian blur changed {changed}/{} samples for {lens:?}; the native preprocessing stage is not represented",
            reduced.len()
        );
    }
}

#[test]
#[ignore = "requires KJERAG_STUDIO_ORACLE_DIR with the detached V6 corpus"]
fn native_fields_state_the_target_a_pair_level_estimator_must_reproduce() {
    let manifest = manifest();
    let root = corpus();
    // B->A at +0xc90 is consumed by lens 0; A->B at +0xc30 by lens 1. Component 0 runs across
    // the seam, component 1 along it, already in final grid units.
    for (id, label, mean_range, median_range, across_p99_range, along_max_range) in [
        (
            "ordinary_b_to_a_lens0",
            "B->A +0xc90",
            (0.84, 0.85),
            (-0.31, -0.30),
            (9.59, 9.61),
            (0.97, 0.98),
        ),
        (
            "ordinary_a_to_b_lens1",
            "A->B +0xc30",
            (-1.03, -1.01),
            (0.02, 0.03),
            (8.39, 8.41),
            (0.88, 0.90),
        ),
    ] {
        let artifact = manifest.artifact(id);
        let bytes = read_external(&root, artifact);
        let field = read_field(artifact, &bytes);
        assert_eq!(field.len(), one_xs::ROWS * one_xs::COLS);
        assert!(
            field.iter().all(|v| v[0].is_finite() && v[1].is_finite()),
            "{label} carries non-finite samples; the capture would be unusable as an oracle",
        );

        let mut signed_across: Vec<f32> = field.iter().map(|value| value[0]).collect();
        let mut absolute_across: Vec<f32> = field.iter().map(|value| value[0].abs()).collect();
        let mut absolute_along: Vec<f32> = field.iter().map(|value| value[1].abs()).collect();
        signed_across.sort_by(f32::total_cmp);
        absolute_across.sort_by(f32::total_cmp);
        absolute_along.sort_by(f32::total_cmp);
        let moving = field.iter().filter(|v| v[0] != 0.0 || v[1] != 0.0).count();
        assert_eq!(
            moving,
            field.len(),
            "{label} must retain the captured nonzero motion at every grid node"
        );

        let mean = field.iter().map(|value| f64::from(value[0])).sum::<f64>() / field.len() as f64;
        let signed_median = f64::from(percentile(&signed_across, 0.50));
        let across_p99 = f64::from(percentile(&absolute_across, 0.99));
        let along_max = f64::from(absolute_along[absolute_along.len() - 1]);
        for (statistic, value, range) in [
            ("signed across mean", mean, mean_range),
            ("signed across median", signed_median, median_range),
            ("absolute across p99", across_p99, across_p99_range),
            ("absolute along maximum", along_max, along_max_range),
        ] {
            assert!(
                (range.0..range.1).contains(&value),
                "{label} {statistic} {value} falls outside captured range {:?}",
                range
            );
        }
        assert!(
            across_p99 > 8.0 && along_max < 1.0,
            "{label} does not preserve the native across/along component interpretation"
        );
    }
}
