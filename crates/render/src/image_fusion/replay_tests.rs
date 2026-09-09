//! Opt-in replay of authenticated native fusion inputs through the readable
//! CPU reference. This is deliberately test-only and owns no capture policy.

use super::{RatioMap, RatioPair, spatial};
use crate::stitch_camera::StitchCamera;
use crate::studio_type2::MAP_NODES;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const BAND_BYTES: usize = 800 * 16 * 3;
const INVALID_BYTES: usize = 212 * 4;
const NATIVE_RATIO_BYTES: usize = MAP_NODES * 3 * size_of::<f32>();

#[derive(Debug)]
struct Row {
    frame: u64,
    left_band: PathBuf,
    right_band: PathBuf,
    invalid: PathBuf,
    native_left: Option<PathBuf>,
    native_right: Option<PathBuf>,
}

/// Replay one complete native capture prefix without writing diagnostics.
///
/// The returned vector is indexed by native Process ordinal and holds the
/// selected camera's published coefficients after that observation. Skipped
/// observations repeat the preceding coefficients. The native-chart arm is
/// checked against both authenticated native maps at every ordinal before its
/// camera-rebased twin is returned to a Scene diagnostic.
pub(crate) fn replay_bands_for_camera(root: &Path, camera: StitchCamera) -> Vec<RatioPair> {
    let rows = read_manifest(root).expect("native fusion replay manifest must be readable");
    validate_contiguous_ordinals(&rows)
        .expect("native fusion replay must contain contiguous ordinals starting at zero");

    let neutral = || {
        let map = RatioMap::new(vec![[1.0, 1.0, 1.0, 0.0]; MAP_NODES]).unwrap();
        RatioPair {
            left: map.clone(),
            right: map,
        }
    };
    let mut native_held = neutral();
    let mut camera_held = neutral();
    let mut native_reference = spatial::Reference::new();
    let mut camera_reference = spatial::Reference::for_camera(camera);
    let mut saw_first_solve = false;
    let mut output = Vec::with_capacity(rows.len());

    for row in rows {
        let left = read_exact(&root.join(&row.left_band), BAND_BYTES)
            .expect("native left fusion band must have its recorded shape");
        let right = read_exact(&root.join(&row.right_band), BAND_BYTES)
            .expect("native right fusion band must have its recorded shape");
        let invalid = read_exact(&root.join(&row.invalid), INVALID_BYTES)
            .expect("native fusion validity must have its recorded shape");
        let native = native_reference
            .observe_bands([&left, &right], &invalid)
            .expect("native-chart fusion replay inputs must be valid");
        let rebased = camera_reference
            .observe_bands([&left, &right], &invalid)
            .expect("camera-chart fusion replay inputs must be valid");
        assert_eq!(
            native.is_some(),
            rebased.is_some(),
            "native and camera references disagreed on admission at ordinal {}",
            row.frame
        );
        assert_eq!(
            native.as_ref().map(|value| &value.diagnostics),
            rebased.as_ref().map(|value| &value.diagnostics),
            "native and camera references disagreed on solve admission at ordinal {}",
            row.frame
        );
        if row.frame == 0 {
            assert_eq!(
                native.as_ref().and_then(|value| value.diagnostics.budget),
                Some(100),
                "native replay ordinal zero must be an admitted cold solve"
            );
        }

        if let Some(value) = native {
            if !saw_first_solve && value.diagnostics.budget.is_some() {
                assert_eq!(
                    value.diagnostics.budget,
                    Some(100),
                    "native replay's first actual solve must use the cold budget"
                );
                saw_first_solve = true;
            }
            native_held = value.ratios;
        }
        if let Some(value) = rebased {
            camera_held = value.ratios;
        }

        for (lens, (path, actual)) in [
            (row.native_left.as_deref(), &native_held.left),
            (row.native_right.as_deref(), &native_held.right),
        ]
        .into_iter()
        .enumerate()
        {
            let path = path.unwrap_or_else(|| {
                panic!(
                    "native replay ordinal {} is missing published map {lens}",
                    row.frame
                )
            });
            let (_, max_abs) = compare_required(root, path, actual)
                .expect("native published ratio must be finite and have its recorded shape");
            assert!(
                max_abs <= 1.0e-5,
                "native replay ordinal {} map {lens} differs by {max_abs}, above 1e-5",
                row.frame
            );
        }
        output.push(camera_held.clone());
    }
    assert!(
        saw_first_solve,
        "native replay contained no actual fusion solve"
    );
    output
}

/// `KJERAG_FUSION_CPU_REPLAY` names a directory containing `manifest.tsv`.
/// Each non-comment row is:
///
/// `frame  left-band  right-band  invalid  native-left|-  native-right|-`
///
/// Paths are relative to the directory. Rows are consumed in file order by
/// one freshly constructed Reference, so a prefix is the explicit warm-up
/// history if a later subset is the comparison target. A capture beginning at
/// the first native Process instead exercises the cold state. The report and
/// generated maps are written below `cpu-replay/` in the capture directory.
/// Native maps are contiguous little-endian CV_32FC3 BGR; the comparison
/// performs the authenticated BGR-to-RGB reorder without padding the payload.
#[test]
#[ignore = "requires an authenticated native fusion sequence"]
fn replay_native_fusion_sequence() {
    let root = PathBuf::from(
        std::env::var_os("KJERAG_FUSION_CPU_REPLAY")
            .expect("KJERAG_FUSION_CPU_REPLAY must name the capture directory"),
    );
    let rows = read_manifest(&root).unwrap();
    assert!(!rows.is_empty(), "fusion replay manifest is empty");

    let output = root.join("cpu-replay");
    fs::create_dir_all(&output).unwrap();
    let mut report = fs::File::create(output.join("report.tsv")).unwrap();
    writeln!(
        report,
        "frame\touter_admitted\tcurrent_metric\tretained_metric\tbudget\tsamples\tstale_control\texits\tleft_bit_diffs\tleft_max_abs\tright_bit_diffs\tright_max_abs"
    )
    .unwrap();

    let neutral = RatioMap::new(vec![[1.0, 1.0, 1.0, 0.0]; MAP_NODES]).unwrap();
    let mut held = RatioPair {
        left: neutral.clone(),
        right: neutral,
    };
    let mut reference = spatial::Reference::new();
    let mut saw_first_solve = false;

    for row in rows {
        let left = read_exact(&root.join(&row.left_band), BAND_BYTES).unwrap();
        let right = read_exact(&root.join(&row.right_band), BAND_BYTES).unwrap();
        let invalid = read_exact(&root.join(&row.invalid), INVALID_BYTES).unwrap();
        let observation = reference.observe_bands([&left, &right], &invalid).unwrap();
        let admitted = observation.is_some();
        let diagnostics = observation.as_ref().map(|value| value.diagnostics.clone());
        if let Some(value) = observation {
            held = value.ratios;
        }
        if !saw_first_solve && diagnostics.as_ref().is_some_and(|d| d.budget.is_some()) {
            assert_eq!(
                diagnostics.as_ref().unwrap().budget,
                Some(100),
                "a fresh replay's first actual solve must use the cold budget"
            );
            saw_first_solve = true;
        }

        let left_path = output.join(format!("frame-{:010}.ratio-left.float4", row.frame));
        let right_path = output.join(format!("frame-{:010}.ratio-right.float4", row.frame));
        fs::write(left_path, held.left.bytes()).unwrap();
        fs::write(right_path, held.right.bytes()).unwrap();
        let left_diff = compare_optional(&root, row.native_left.as_deref(), &held.left).unwrap();
        let right_diff = compare_optional(&root, row.native_right.as_deref(), &held.right).unwrap();

        let (current, retained, budget, samples, stale, exits) = diagnostics.map_or_else(
            || {
                (
                    "-".into(),
                    "-".into(),
                    "-".into(),
                    "-".into(),
                    "-".into(),
                    "-".into(),
                )
            },
            |d| {
                (
                    option(d.current_metric),
                    d.retained_metric.to_string(),
                    option(d.budget),
                    d.admitted.to_string(),
                    d.used_stale_control.to_string(),
                    format!("{:?}", d.exits),
                )
            },
        );
        writeln!(
            report,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            row.frame,
            admitted,
            current,
            retained,
            budget,
            samples,
            stale,
            exits,
            left_diff.0,
            left_diff.1,
            right_diff.0,
            right_diff.1,
        )
        .unwrap();
    }
    assert!(saw_first_solve, "capture contained no actual fusion solve");
}

fn option<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(|| "-".into(), |value| value.to_string())
}

fn read_manifest(root: &Path) -> Result<Vec<Row>, String> {
    let text = fs::read_to_string(root.join("manifest.tsv")).map_err(|e| e.to_string())?;
    let rows: Vec<Row> = text
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with('#') && !line.starts_with("frame\t")
        })
        .map(|(line_index, line)| {
            let fields: Vec<_> = line.split('\t').collect();
            if fields.len() != 6 {
                return Err(format!(
                    "manifest line {} has {} fields, expected 6",
                    line_index + 1,
                    fields.len()
                ));
            }
            let optional = |value: &str| (value != "-").then(|| PathBuf::from(value));
            Ok(Row {
                frame: fields[0]
                    .parse()
                    .map_err(|e| format!("manifest line {} frame: {e}", line_index + 1))?,
                left_band: fields[1].into(),
                right_band: fields[2].into(),
                invalid: fields[3].into(),
                native_left: optional(fields[4]),
                native_right: optional(fields[5]),
            })
        })
        .collect::<Result<_, _>>()?;
    validate_order(&rows)?;
    Ok(rows)
}

fn validate_order(rows: &[Row]) -> Result<(), String> {
    for pair in rows.windows(2) {
        if pair[0].frame >= pair[1].frame {
            return Err(format!(
                "manifest frames must be strictly increasing, got {} then {}",
                pair[0].frame, pair[1].frame
            ));
        }
    }
    Ok(())
}

fn validate_contiguous_ordinals(rows: &[Row]) -> Result<(), String> {
    if rows.is_empty() {
        return Err("fusion replay manifest is empty".into());
    }
    for (ordinal, row) in rows.iter().enumerate() {
        if row.frame != ordinal as u64 {
            return Err(format!(
                "manifest ordinal {ordinal} names frame {}, expected {ordinal}",
                row.frame
            ));
        }
    }
    Ok(())
}

fn read_exact(path: &Path, expected: usize) -> Result<Vec<u8>, String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() != expected {
        return Err(format!(
            "{} has {} bytes, expected {expected}",
            path.display(),
            bytes.len()
        ));
    }
    Ok(bytes)
}

fn compare_optional(
    root: &Path,
    path: Option<&Path>,
    actual: &RatioMap,
) -> Result<(String, String), String> {
    let Some(path) = path else {
        return Ok(("-".into(), "-".into()));
    };
    let (bit_diffs, max_abs) = compare_required(root, path, actual)?;
    Ok((bit_diffs.to_string(), max_abs.to_string()))
}

fn compare_required(root: &Path, path: &Path, actual: &RatioMap) -> Result<(usize, f32), String> {
    let path = root.join(path);
    let expected = read_exact(&path, NATIVE_RATIO_BYTES)?;
    let expected = decode_native_bgr(&expected)?;
    let mut bit_diffs = 0usize;
    let mut max_abs = 0.0f32;
    for (node, (&bgr, rgba)) in expected.iter().zip(actual.values()).enumerate() {
        if rgba[3].to_bits() != 0 {
            return Err(format!(
                "CPU ratio node {node} has nonzero fourth component"
            ));
        }
        for channel in 0..3 {
            let expected = bgr[2 - channel];
            let actual = rgba[channel];
            if !actual.is_finite() {
                return Err(format!(
                    "CPU ratio node {node} channel {channel} is non-finite"
                ));
            }
            bit_diffs += usize::from(expected.to_bits() != actual.to_bits());
            max_abs = max_abs.max((expected - actual).abs());
        }
    }
    Ok((bit_diffs, max_abs))
}

fn decode_native_bgr(bytes: &[u8]) -> Result<Vec<[f32; 3]>, String> {
    if bytes.len() != NATIVE_RATIO_BYTES {
        return Err(format!(
            "native ratio has {} bytes, expected {NATIVE_RATIO_BYTES}",
            bytes.len()
        ));
    }
    bytes
        .chunks_exact(3 * size_of::<f32>())
        .enumerate()
        .map(|(node, pixel)| {
            let bgr = std::array::from_fn(|channel| {
                let at = channel * size_of::<f32>();
                f32::from_le_bytes(pixel[at..at + size_of::<f32>()].try_into().unwrap())
            });
            if let Some(channel) = bgr.iter().position(|value| !value.is_finite()) {
                Err(format!(
                    "native ratio node {node} channel {channel} is non-finite"
                ))
            } else {
                Ok(bgr)
            }
        })
        .collect()
}

#[test]
fn native_ratio_decoder_rejects_bad_shape_and_nonfinite_values() {
    assert!(decode_native_bgr(&vec![0; NATIVE_RATIO_BYTES - 1]).is_err());
    let mut bytes = vec![0; NATIVE_RATIO_BYTES];
    bytes[..4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(decode_native_bgr(&bytes).is_err());
    bytes[..4].copy_from_slice(&f32::INFINITY.to_le_bytes());
    assert!(decode_native_bgr(&bytes).is_err());
}

#[test]
fn comparison_reports_a_missing_native_map() {
    let map = RatioMap::new(vec![[1.0, 1.0, 1.0, 0.0]; MAP_NODES]).unwrap();
    let error = compare_optional(
        Path::new("/"),
        Some(Path::new("kjerag-replay-test-native-map-does-not-exist")),
        &map,
    )
    .unwrap_err();
    assert!(error.contains("does-not-exist"));
}

#[test]
fn manifest_rejects_duplicate_or_backwards_frames() {
    let rows = |frames: &[u64]| {
        frames
            .iter()
            .map(|&frame| Row {
                frame,
                left_band: "l".into(),
                right_band: "r".into(),
                invalid: "i".into(),
                native_left: None,
                native_right: None,
            })
            .collect::<Vec<_>>()
    };
    assert!(validate_order(&rows(&[1, 2, 3])).is_ok());
    assert!(validate_order(&rows(&[1, 1])).is_err());
    assert!(validate_order(&rows(&[2, 1])).is_err());
}

#[test]
fn scene_replay_requires_a_nonempty_contiguous_zero_based_prefix() {
    let rows = |frames: &[u64]| {
        frames
            .iter()
            .map(|&frame| Row {
                frame,
                left_band: "l".into(),
                right_band: "r".into(),
                invalid: "i".into(),
                native_left: None,
                native_right: None,
            })
            .collect::<Vec<_>>()
    };
    assert!(validate_contiguous_ordinals(&rows(&[0, 1, 2])).is_ok());
    assert!(validate_contiguous_ordinals(&rows(&[])).is_err());
    assert!(validate_contiguous_ordinals(&rows(&[1, 2])).is_err());
    assert!(validate_contiguous_ordinals(&rows(&[0, 2])).is_err());
}
