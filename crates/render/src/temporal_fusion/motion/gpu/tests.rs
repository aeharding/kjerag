use super::{Builder, Error, Prepared};
use crate::temporal_fusion::motion::{self, Geometry, Parameters};
use crate::temporal_fusion::tests::{copy_texture, gpu, read_copy};
use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

fn parameters<'a>(
    raw_grid: [u32; 2],
    luma: &'a [u8],
    y: &'a [f32; 256],
    uv: &'a [f32; 256],
) -> Parameters<'a> {
    Parameters {
        geometry: Geometry {
            full: raw_grid.map(|value| value * 32),
            raw_grid,
            output_grid: raw_grid.map(|value| value * 2),
            block: [16, 16],
        },
        luma,
        confidence_y: y,
        confidence_uv: uv,
        scale_base: 1,
        scale_extra: 1,
        temporal: 1.0,
        phase: 0.0,
    }
}

fn bytes(records: &[[i16; 4]]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|record| record.iter().flat_map(|value| value.to_le_bytes()))
        .collect()
}

fn check(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    builder: &Builder,
    raw: &[[i32; 3]],
    parameters: &Parameters<'_>,
    label: &str,
) {
    let expected = bytes(&motion::pack_motion(raw, parameters).unwrap());
    let mut encoder = device.create_command_encoder(&Default::default());
    let output = builder
        .encode(device, &mut encoder, raw, parameters)
        .unwrap();
    let size = parameters.geometry.output_grid;
    let copy = copy_texture(device, &mut encoder, &output, size, 8);
    queue.submit([encoder.finish()]);
    let actual = read_copy(device, &copy, size, 8);
    assert!(
        actual == expected,
        "{label}: first unequal byte {:?}, lengths {} and {}",
        actual.iter().zip(&expected).position(|(a, b)| a != b),
        actual.len(),
        expected.len()
    );
}

#[test]
fn shader_validates_without_optional_capabilities() {
    let module = wgpu::naga::front::wgsl::parse_str(include_str!("../gpu.wgsl")).unwrap();
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap();
}

#[test]
fn interpolation_truncation_clamping_and_separate_tables_match_reference() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    let table = [4.0; 256];
    let p = parameters([2, 2], &[0; 16], &table, &table);
    for raw in [
        [[0, 0, 0], [2, 4, 3], [4, 8, 5], [6, 12, 7]],
        [[-1, 0, 0], [-1, 0, 0], [-1, 0, 0], [0, 0, 0]],
        [
            [-32768, 32767, 0],
            [32767, -32768, 1],
            [0, 0, 3],
            [1, -1, 4],
        ],
    ] {
        check(
            &device,
            &queue,
            &builder,
            &raw,
            &p,
            "interpolation and edges",
        );
    }
    let luma: Vec<_> = (0..=255).collect();
    let y = std::array::from_fn(|at| at as f32 - 64.0);
    let uv = std::array::from_fn(|at| 512.0 - at as f32 * 2.0);
    let raw: Vec<_> = (0..64).map(|at| [at - 32, 32 - at, at * 4]).collect();
    check(
        &device,
        &queue,
        &builder,
        &raw,
        &parameters([8, 8], &luma, &y, &uv),
        "all luma indices and independent signed thresholds",
    );
}

#[test]
fn confidence_matches_reference_for_every_cost_at_captured_and_limit_thresholds() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    // Every possible 16x16 byte SAD appears at an even output coordinate.
    // Odd coordinates additionally test interpolated fractional costs.
    let raw: Vec<_> = (0..65536).map(|at| [0, 0, at % 65281]).collect();
    let luma = vec![0; 1024 * 256];
    for threshold in [0, 1, 2, 3, 2800, 3150, 3500, 5600, 6300, 7000, 46340] {
        let y = [threshold as f32; 256];
        let uv = [(threshold - 1) as f32; 256];
        check(
            &device,
            &queue,
            &builder,
            &raw,
            &parameters([512, 128], &luma, &y, &uv),
            &format!("exhaustive costs, threshold {threshold}"),
        );
    }
}

#[test]
fn scale_wrap_and_multiple_encodes_keep_independent_inputs() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    let y = [1.0; 256];
    let uv = [2.0; 256];
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut copies = Vec::new();
    for (scale_base, scale_extra, temporal, phase, cost) in [
        (i32::MAX, 2, 1.0, 0.0, 0),
        (4, 700, 1.25, 0.49999999999999994, 3150),
        (4, 700, 1.25, 0.0, 1),
        (4, 700, 1.25, 1.0, 2799),
    ] {
        let mut p = parameters([1, 1], &[0; 4], &y, &uv);
        p.scale_base = scale_base;
        p.scale_extra = scale_extra;
        p.temporal = temporal;
        p.phase = phase;
        let raw = [[-20, 20, cost]];
        let expected = bytes(&motion::pack_motion(&raw, &p).unwrap());
        let output = builder.encode(&device, &mut encoder, &raw, &p).unwrap();
        copies.push((
            copy_texture(&device, &mut encoder, &output, [2, 2], 8),
            expected,
        ));
    }
    queue.submit([encoder.finish()]);
    for (copy, expected) in copies {
        assert_eq!(read_copy(&device, &copy, [2, 2], 8), expected);
    }
}

#[test]
fn host_rejects_unsupported_numeric_inputs_before_encoding() {
    let table = [1.0; 256];
    let mut p = parameters([1, 1], &[0; 4], &table, &table);
    for raw in [[[0, 0, -1]], [[0, 0, 65281]]] {
        assert!(matches!(Prepared::new(&raw, &p), Err(Error::RawCost)));
    }
    for raw in [[[32768, 0, 0]], [[0, -32769, 0]]] {
        assert!(matches!(
            Prepared::new(&raw, &p),
            Err(Error::RawDisplacement)
        ));
    }
    let outside = [65536.0; 256];
    p.confidence_uv = &outside;
    assert!(matches!(
        Prepared::new(&[[0, 0, 1]], &p),
        Err(Error::Threshold)
    ));
    // The readable reference intentionally retains native wrapping squares;
    // the GPU subset must refuse this case rather than silently differ.
    assert_eq!(motion::pack_motion(&[[0, 0, 1]], &p).unwrap()[0][3], -256);
    p.confidence_uv = &table;
    p.phase = f64::NAN;
    assert!(matches!(
        Prepared::new(&[[0, 0, 0]], &p),
        Err(Error::Reference(_))
    ));
    let luma = vec![0; 1025 * 4];
    let p = parameters([1025, 1], &luma, &table, &table);
    assert!(matches!(
        Prepared::new(&vec![[0, 0, 0]; 1025], &p),
        Err(Error::FullDimensions)
    ));
}

#[test]
fn rejected_encode_does_not_poison_following_valid_work() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    let table = [1.0; 256];
    let p = parameters([1, 1], &[0; 4], &table, &table);
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(matches!(
        builder.encode(&device, &mut encoder, &[[0, 0, -1]], &p),
        Err(Error::RawCost)
    ));
    let output = builder
        .encode(&device, &mut encoder, &[[0, 0, 0]], &p)
        .unwrap();
    let copy = copy_texture(&device, &mut encoder, &output, [2, 2], 8);
    queue.submit([encoder.finish()]);
    assert_eq!(
        read_copy(&device, &copy, [2, 2], 8),
        bytes(&[[0, 0, 256, 256]; 4])
    );
}

#[test]
#[ignore = "requires private Studio motion capture named by KJERAG_MOTION_FIXTURE_DIR"]
fn captured_six_reference_motion_matrices_are_exact() {
    let Some((device, queue)) = gpu() else { return };
    let builder = Builder::new(&device);
    let fixture = motion::tests::native_fixture();
    let mut encoder = device.create_command_encoder(&Default::default());
    let copies: Vec<_> = fixture
        .references
        .iter()
        .map(|reference| {
            let parameters = fixture.parameters(reference.phase);
            let output = builder
                .encode(&device, &mut encoder, &reference.raw, &parameters)
                .unwrap();
            copy_texture(
                &device,
                &mut encoder,
                &output,
                parameters.geometry.output_grid,
                8,
            )
        })
        .collect();
    queue.submit([encoder.finish()]);
    for (ordinal, (reference, copy)) in fixture.references.iter().zip(copies).enumerate() {
        let actual = read_copy(&device, &copy, [480, 240], 8);
        assert!(
            actual == reference.expected,
            "native reference {ordinal}: first unequal byte {:?}",
            actual
                .iter()
                .zip(&reference.expected)
                .position(|(a, b)| a != b)
        );
    }
    eprintln!("six native motion fields: all 2,764,800 signed lanes exact");
}
