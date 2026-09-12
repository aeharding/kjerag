//! Saved-input arithmetic controls only. These do not select production
//! behavior or constitute a visual acceptance test.

use super::*;
use std::{collections::BTreeMap, fs, io::Write, path::PathBuf};

/// Selected arm64 Eigen finish at 0x322ed44..0x322ed48. Products and
/// accumulator updates are separate fmul/fadd, not fused multiply-add.
fn native_dot(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), chroma::NODES);
    assert_eq!(b.len(), chroma::NODES);
    let mut acc = [[0.0_f32; 4]; 2];
    for (a, b) in a.chunks_exact(8).zip(b.chunks_exact(8)) {
        for (packet, lanes) in acc.iter_mut().enumerate() {
            for (lane, sum) in lanes.iter_mut().enumerate() {
                let i = packet * 4 + lane;
                *sum += a[i] * b[i];
            }
        }
    }
    let lanes: [f32; 4] = std::array::from_fn(|i| acc[0][i] + acc[1][i]);
    (lanes[0] + lanes[1]) + (lanes[2] + lanes[3])
}

/// Selected arm64 centering at 0x322dfc8..0x322e250. The native allocation
/// is aligned and N=5088 has no scalar remainder. It seeds its accumulators
/// from the first eight elements, then uses the adjacent-pair finish.
fn native_centre(x: &mut [f32]) {
    assert_eq!(x.len(), chroma::NODES);
    let mut acc = [[x[0], x[1], x[2], x[3]], [x[4], x[5], x[6], x[7]]];
    for chunk in x[8..].chunks_exact(8) {
        for (packet, lanes) in acc.iter_mut().enumerate() {
            for (lane, sum) in lanes.iter_mut().enumerate() {
                *sum += chunk[packet * 4 + lane];
            }
        }
    }
    let lanes: [f32; 4] = std::array::from_fn(|i| acc[1][i] + acc[0][i]);
    let mean = ((lanes[0] + lanes[1]) + (lanes[2] + lanes[3])) / chroma::NODES as f32;
    for value in x {
        *value -= mean;
    }
}

/// Test-only copy of the readable CG recurrence with an injectable dot
/// reduction. The node-order control asserts its output and exits against
/// the ordinary reference at every saved observation.
fn cg(
    apply: impl Fn(&[f32], &mut [f32]),
    pre: &[f32],
    q: &[f32],
    x: &mut [f32],
    budget: u32,
    dot: fn(&[f32], &[f32]) -> f32,
) -> chromatic::Exit {
    use chromatic::Exit;
    let q2 = dot(q, q);
    if q2 == 0.0 {
        x.fill(0.0);
        return Exit::Converged { iterations: 0 };
    }
    let threshold = f32::MIN_POSITIVE.max(chromatic::TOLERANCE * chromatic::TOLERANCE * q2);
    let mut nx = vec![0.0; x.len()];
    apply(x, &mut nx);
    let mut r: Vec<_> = q.iter().zip(nx).map(|(q, nx)| q - nx).collect();
    if dot(&r, &r) < threshold {
        return Exit::Converged { iterations: 0 };
    }
    let mut z: Vec<_> = pre.iter().zip(&r).map(|(p, r)| p * r).collect();
    let mut d = z.clone();
    let mut rho = dot(&r, &z);
    let mut v = vec![0.0; x.len()];
    for step in 0..budget {
        v.fill(0.0);
        apply(&d, &mut v);
        let alpha = rho / dot(&d, &v);
        for i in 0..x.len() {
            x[i] += alpha * d[i];
            r[i] -= alpha * v[i];
        }
        if dot(&r, &r) < threshold {
            return Exit::Converged {
                iterations: step + 1,
            };
        }
        for i in 0..x.len() {
            z[i] = pre[i] * r[i];
        }
        let rho2 = dot(&r, &z);
        let beta = rho2 / rho;
        for i in 0..x.len() {
            d[i] = z[i] + beta * d[i];
        }
        rho = rho2;
    }
    Exit::Truncated { iterations: budget }
}

#[test]
fn adjacent_reduction_finish_is_not_the_windows_cross_pair_choice() {
    let mut a = vec![0.0; chroma::NODES];
    a[..4].copy_from_slice(&[16_777_216.0, 1.0, -16_777_216.0, 1.0]);
    let ones = vec![1.0; chroma::NODES];
    assert_eq!(native_dot(&a, &ones), 1.0);
    assert_eq!(chromatic::dot(&a, &ones), 2.0);
    native_centre(&mut a);
    assert_eq!(a[8], -1.0 / chroma::NODES as f32);
}

/// Materialize the same normal equations, then multiply separate diagonal
/// and off-diagonal entries in ascending native column order. This isolates
/// the edge-flow factorization's rounding; it does NOT claim to duplicate
/// every Eigen sparse-product instruction or fused multiply-add.
fn sparse_rows(samples: &[chroma::Sample], inverse: &[usize]) -> Vec<Vec<(usize, f32)>> {
    let mut rows = vec![BTreeMap::<usize, f32>::new(); chroma::NODES];
    let penalty = chromatic::penalty_squared();
    let couplings = chroma::edges()
        .into_iter()
        .map(|(a, b)| (a, b, penalty))
        .chain(samples.iter().map(|s| (s.k0, s.k1, s.weight * s.weight)));
    for (a, b, coefficient) in couplings {
        let (a, b) = (inverse[a], inverse[b]);
        *rows[a].entry(a).or_default() += coefficient;
        *rows[b].entry(b).or_default() += coefficient;
        *rows[a].entry(b).or_default() -= coefficient;
        *rows[b].entry(a).or_default() -= coefficient;
    }
    rows.into_iter()
        .map(|row| row.into_iter().collect())
        .collect()
}

#[test]
fn materialized_normal_matrix_preserves_diagonal_and_symmetry() {
    let inverse: Vec<_> = (0..chroma::NODES).collect();
    let samples = [chroma::Sample {
        k0: chroma::node(0, 9, 60).unwrap(),
        k1: chroma::node(1, 9, 60).unwrap(),
        weight: 3.2,
        difference: [5.0, -3.0, 2.0],
    }];
    let rows = sparse_rows(&samples, &inverse);
    let pre = chroma::System::new(&samples).preconditioner();
    for (i, row) in rows.iter().enumerate() {
        for &(j, coefficient) in row {
            assert_eq!(
                rows[j].iter().find(|&&(k, _)| k == i).unwrap().1,
                coefficient
            );
            if i == j {
                assert_eq!(1.0 / coefficient, pre[i]);
            }
        }
    }
}

#[test]
#[ignore = "requires saved native bands/prepared-stage replay inputs"]
fn replay_native_arithmetic() {
    let root = PathBuf::from(std::env::var_os("KJERAG_FUSION_ARITHMETIC_INPUT").unwrap());
    let native = PathBuf::from(std::env::var_os("KJERAG_FUSION_ARITHMETIC_NATIVE").unwrap());
    let output = PathBuf::from(std::env::var_os("KJERAG_FUSION_ARITHMETIC_OUTPUT").unwrap());
    fs::create_dir(&output).expect("arithmetic output must be a new directory");
    let order: Vec<_> = (0..chroma::WINDOW_ROWS)
        .flat_map(|row| {
            (0..chroma::COLUMNS).flat_map(move |column| {
                (0..2).filter_map(move |lens| chroma::node(lens, row, column))
            })
        })
        .collect();
    assert_eq!(order.len(), chroma::NODES);
    let mut inverse = vec![usize::MAX; chroma::NODES];
    for (native, &physical) in order.iter().enumerate() {
        assert_eq!(inverse[physical], usize::MAX);
        inverse[physical] = native;
    }
    let mut reference = Reference::new();
    let mut report = fs::File::create_new(output.join("report.tsv")).unwrap();
    writeln!(report, "frame\tarm\tlens\tchanged_reference_bytes\tmax_reference_byte_delta\tchanged_native_bytes\tmax_native_byte_delta\tmax_reference_field_delta\texits").unwrap();
    // Only the named factor differs from node-order. The final arm combines
    // the three controls without changing evidence, preconditioner or budget.
    let arms = [
        ("node-order", false, false, false),
        ("sparse-matvec", true, false, false),
        ("native-centre", false, true, false),
        ("native-dot", false, false, true),
        ("combined", true, true, true),
    ];
    let mut states: Vec<[Vec<f32>; 3]> = arms
        .iter()
        .map(|_| std::array::from_fn(|_| vec![0.0; chroma::NODES]))
        .collect();
    for frame in [0, 7, 13] {
        let current = [0, 1].map(|lens| {
            fs::read(root.join(format!("frame-{frame:03}.reference-current-{lens}.bgr8"))).unwrap()
        });
        let valid = vec![0; VALIDITY_BYTES];
        let ordinary = reference.observe(Inputs::new(&current[0], &current[1], &valid).unwrap());
        for (lens, prepared) in ordinary.prepared.iter().enumerate() {
            assert_eq!(
                *prepared,
                fs::read(root.join(format!("frame-{frame:03}.reference-prepared-{lens}.bgr8")))
                    .unwrap(),
                "the arithmetic control must reproduce the saved baseline"
            );
        }
        let samples = samples(
            [&current[0], &current[1]],
            reference.control.as_ref().unwrap(),
            reference.metric.scale(),
        );
        let system = chroma::System::new(&samples);
        let rows = sparse_rows(&samples, &inverse);
        let pre = system.preconditioner();
        let native_pre: Vec<_> = order.iter().map(|&i| pre[i]).collect();
        for ((name, sparse, centre, dot), fields) in arms.iter().zip(&mut states) {
            let mut exits = Vec::new();
            for (channel, field) in fields.iter_mut().enumerate() {
                let q = system.rhs(&samples, channel);
                let native_q: Vec<_> = order.iter().map(|&i| q[i]).collect();
                let mut native_x: Vec<_> = order.iter().map(|&i| field[i]).collect();
                exits.push(cg(
                    |x, y| {
                        if *sparse {
                            for (row, result) in rows.iter().zip(y) {
                                *result = row.iter().fold(0.0, |sum, &(j, a)| sum + a * x[j]);
                            }
                        } else {
                            let physical: Vec<_> = inverse.iter().map(|&i| x[i]).collect();
                            let mut product = vec![0.0; chroma::NODES];
                            system.apply(&physical, &mut product);
                            for (native, &i) in order.iter().enumerate() {
                                y[native] = product[i];
                            }
                        }
                    },
                    &native_pre,
                    &native_q,
                    &mut native_x,
                    ordinary.diagnostics.budget.unwrap(),
                    if *dot { native_dot } else { chromatic::dot },
                ));
                if *centre {
                    native_centre(&mut native_x);
                }
                for (native, &i) in order.iter().enumerate() {
                    field[i] = native_x[native];
                }
                if !*centre {
                    chroma::centre(field);
                }
            }
            assert!(fields.iter().flatten().all(|value| value.is_finite()));
            let max_field_delta = fields
                .iter()
                .flatten()
                .zip(reference.fields.iter().flatten())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f32, f32::max);
            let mut prepared = current.clone();
            apply_fields(&mut prepared, fields);
            if *name == "node-order" {
                assert_eq!(prepared, ordinary.prepared);
                assert_eq!(
                    exits.iter().copied().map(Some).collect::<Vec<_>>(),
                    ordinary.diagnostics.exits
                );
            }
            for (lens, image) in prepared.iter().enumerate() {
                let native_image =
                    fs::read(native.join(format!("frame-{frame:03}/prepared-{lens}.bgr8")))
                        .unwrap();
                assert_eq!(native_image.len(), image.len());
                let delta = |other: &[u8]| {
                    image
                        .iter()
                        .zip(other)
                        .fold((0, 0), |(changed, max), (&a, &b)| {
                            (changed + usize::from(a != b), max.max(a.abs_diff(b)))
                        })
                };
                let (changed_reference, max_reference) = delta(&ordinary.prepared[lens]);
                let (changed_native, max_native) = delta(&native_image);
                writeln!(report, "{frame}\t{name}\t{lens}\t{changed_reference}\t{max_reference}\t{changed_native}\t{max_native}\t{max_field_delta}\t{exits:?}").unwrap();
                fs::write(
                    output.join(format!("frame-{frame:03}.{name}.prepared-{lens}.bgr8")),
                    image,
                )
                .unwrap();
            }
        }
    }
}
