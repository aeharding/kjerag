//! Historical component diagnostics on rescued ONE X2 inputs.
//!
//! These ignored tests preserve earlier stage measurements, including the
//! optional variational reference and pre-unification base-support masks. They
//! are not the selected route after sections 150-151. The readable current
//! cold owner is [`super::scalar::ColdPair`], which uses six descents per pass,
//! unified masks and no derivative or variational execution. This module stays
//! in-crate and needs the rescued V6 corpus.
//!
//! ```text
//! cargo test -p kjerag-render --lib native_chain -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use super::dense::{self, DirectedImages};
use super::derivative_prep::{PreparationImages, StridedImage, prepare};
use super::pis::{AtoB, BtoA, CostMode, InitialGrid, Input, Level};
use super::variational::refine_prepared;
use super::{self as one_xs, Lens, LensPair};

const DEFAULT_DIR: &str = "scratch/mac602-captures/studio602-head-v6.20260818.uDZbCF";
const STAGING_ROWS: usize = one_xs::SOURCE_ROWS;
const STAGING_COLS: usize = one_xs::SOURCE_COLS;
const BELT_ROWS: usize = one_xs::ROWS;
const BELT_COLS: usize = one_xs::COLS;

fn corpus() -> PathBuf {
    let root = std::env::var("KJERAG_V6_DIR").unwrap_or_else(|_| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .expect("workspace root")
            .join(DEFAULT_DIR)
            .to_string_lossy()
            .into_owned()
    });
    PathBuf::from(root)
}

fn read_bytes(name: &str, expected: usize) -> Vec<u8> {
    let path = corpus().join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(bytes.len(), expected, "{name} has the wrong size");
    bytes
}

fn read_f32(name: &str, expected: usize) -> Vec<f32> {
    let bytes = read_bytes(name, expected * 4);
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn reflect101(index: isize, extent: usize) -> usize {
    let last = extent as isize - 1;
    let mut i = index;
    if last == 0 {
        return 0;
    }
    while i < 0 || i > last {
        if i < 0 {
            i = -i;
        }
        if i > last {
            i = 2 * last - i;
        }
    }
    i as usize
}

/// OpenCV `getGaussianKernel(ksize, sigma)`.
fn gaussian_kernel(ksize: usize, sigma: f64) -> Vec<f64> {
    let centre = (ksize - 1) as f64 * 0.5;
    let scale = -0.5 / (sigma * sigma);
    let mut k: Vec<f64> = (0..ksize)
        .map(|i| {
            let x = i as f64 - centre;
            (scale * x * x).exp()
        })
        .collect();
    let sum: f64 = k.iter().sum();
    for v in &mut k {
        *v /= sum;
    }
    k
}

fn separable(src: &[f64], cols: usize, rows: usize, kernel: &[f64]) -> Vec<f64> {
    let radius = (kernel.len() / 2) as isize;
    let mut mid = vec![0.0; src.len()];
    for r in 0..rows {
        for c in 0..cols {
            let mut acc = 0.0;
            for (k, w) in kernel.iter().enumerate() {
                let cc = reflect101(c as isize + k as isize - radius, cols);
                acc += src[r * cols + cc] * w;
            }
            mid[r * cols + c] = acc;
        }
    }
    let mut out = vec![0.0; src.len()];
    for r in 0..rows {
        for c in 0..cols {
            let mut acc = 0.0;
            for (k, w) in kernel.iter().enumerate() {
                let rr = reflect101(r as isize + k as isize - radius, rows);
                acc += mid[rr * cols + c] * w;
            }
            out[r * cols + c] = acc;
        }
    }
    out
}

fn gaussian_f32(src: &[f32], cols: usize, rows: usize, ksize: usize, sigma: f64) -> Vec<f32> {
    let k = gaussian_kernel(ksize, sigma);
    let wide: Vec<f64> = src.iter().map(|v| f64::from(*v)).collect();
    separable(&wide, cols, rows, &k)
        .into_iter()
        .map(|v| v as f32)
        .collect()
}

/// Exact-integer-factor `INTER_AREA` on `CV_8UC1`: the block mean, rounded half away from zero.
fn inter_area(src: &[u8], cols: usize, rows: usize, factor: usize) -> (Vec<u8>, usize, usize) {
    let (nc, nr) = (cols / factor, rows / factor);
    let area = (factor * factor) as u32;
    let mut out = vec![0u8; nc * nr];
    for r in 0..nr {
        for c in 0..nc {
            let mut sum = 0u32;
            for dr in 0..factor {
                for dc in 0..factor {
                    sum += u32::from(src[(r * factor + dr) * cols + c * factor + dc]);
                }
            }
            out[r * nc + c] = ((sum * 2 + area) / (area * 2)) as u8;
        }
    }
    (out, nc, nr)
}

fn inter_nearest(src: &[u8], cols: usize, rows: usize, factor: usize) -> (Vec<u8>, usize, usize) {
    let (nc, nr) = (cols / factor, rows / factor);
    let mut out = vec![0u8; nc * nr];
    for r in 0..nr {
        for c in 0..nc {
            let sr = ((r as f32 + 0.5) * factor as f32) as usize;
            let sc = ((c as f32 + 0.5) * factor as f32) as usize;
            out[r * nc + c] = src[sr.min(rows - 1) * cols + sc.min(cols - 1)];
        }
    }
    (out, nc, nr)
}

/// `cv::spatialGradient` with ksize 3 and `BORDER_REFLECT_101`.
fn sobel3(src: &[u8], cols: usize, rows: usize) -> (Vec<f32>, Vec<f32>) {
    let kx: [f32; 3] = [-1.0, 0.0, 1.0];
    let smooth: [f32; 3] = [1.0, 2.0, 1.0];
    let mut dx = vec![0.0; src.len()];
    let mut dy = vec![0.0; src.len()];
    for r in 0..rows {
        for c in 0..cols {
            let (mut gx, mut gy) = (0.0f32, 0.0f32);
            for j in 0..3isize {
                for i in 0..3isize {
                    let rr = reflect101(r as isize + j - 1, rows);
                    let cc = reflect101(c as isize + i - 1, cols);
                    let v = f32::from(src[rr * cols + cc]);
                    gx += v * kx[i as usize] * smooth[j as usize];
                    gy += v * smooth[i as usize] * kx[j as usize];
                }
            }
            dx[r * cols + c] = gx;
            dy[r * cols + c] = gy;
        }
    }
    (dx, dy)
}

/// Section 117: seed `0 < u <= 1 && 0 < v <= 1` on each base map.
fn mask_seed(base: &[f32]) -> Vec<u8> {
    base.chunks_exact(2)
        .map(|uv| {
            let (u, v) = (uv[0], uv[1]);
            u8::from(u > 0.0 && u <= 1.0 && v > 0.0 && v <= 1.0) * 255
        })
        .collect()
}

/// Section 117B: the selected Optical-Flow tier erodes with a 9-by-9 rectangle, not 5-by-5.
/// `BORDER_CONSTANT` at `+inf` means the frame does not eat into the mask.
fn erode_rect(mask: &[u8], cols: usize, rows: usize, ksize: usize) -> Vec<u8> {
    let radius = (ksize / 2) as isize;
    let mut out = vec![0u8; mask.len()];
    for r in 0..rows {
        for c in 0..cols {
            let mut smallest = u8::MAX;
            for dr in -radius..=radius {
                for dc in -radius..=radius {
                    let (rr, cc) = (r as isize + dr, c as isize + dc);
                    if rr < 0 || cc < 0 || rr >= rows as isize || cc >= cols as isize {
                        continue; // BORDER_CONSTANT +inf: outside never lowers the minimum
                    }
                    smallest = smallest.min(mask[rr as usize * cols + cc as usize]);
                }
            }
            out[r * cols + c] = smallest;
        }
    }
    out
}

struct Retained {
    image: LensPair<Vec<u8>>,
    mask: LensPair<Vec<u8>>,
}

fn retained() -> Retained {
    let staging_len = STAGING_ROWS * STAGING_COLS;
    let reduce = |name: &str| {
        let staging = read_bytes(name, staging_len);
        let (belt, cols, rows) = inter_area(&staging, STAGING_COLS, STAGING_ROWS, 3);
        assert_eq!((cols, rows), (BELT_COLS, BELT_ROWS));
        // Section 114 preprocessing: Gaussian 5x5 sigma 0.8 after the 3x3 INTER_AREA.
        let wide: Vec<f32> = belt.iter().map(|v| f32::from(*v)).collect();
        gaussian_f32(&wide, BELT_COLS, BELT_ROWS, 5, 0.8)
            .into_iter()
            .map(|v| v.round().clamp(0.0, 255.0) as u8)
            .collect::<Vec<u8>>()
    };
    let belt_pixels = BELT_ROWS * BELT_COLS;
    let seed = |name: &str| {
        let base = read_f32(name, belt_pixels * 2);
        erode_rect(&mask_seed(&base), BELT_COLS, BELT_ROWS, 9)
    };
    Retained {
        image: LensPair {
            a: reduce("00038_target_input_belt_b70.bin"),
            b: reduce("00039_target_input_belt_bd0.bin"),
        },
        mask: LensPair {
            a: seed("00041_target_base_lookup_left_8d0.bin"),
            b: seed("00042_target_base_lookup_right_930.bin"),
        },
    }
}

struct LevelInputs {
    rows: usize,
    cols: usize,
    image: LensPair<Vec<u8>>,
    mask: LensPair<Vec<u8>>,
    gradient_col: Vec<f32>,
    gradient_row: Vec<f32>,
    raw_weight: Vec<f32>,
}

fn level_inputs(retained: &Retained, level: Level, source_is_b: bool) -> LevelInputs {
    let factor = 1usize << level.index();
    let (image_a, cols, rows) = inter_area(&retained.image.a, BELT_COLS, BELT_ROWS, factor);
    let (image_b, _, _) = inter_area(&retained.image.b, BELT_COLS, BELT_ROWS, factor);
    let (mask_a, _, _) = inter_nearest(&retained.mask.a, BELT_COLS, BELT_ROWS, factor);
    let (mask_b, _, _) = inter_nearest(&retained.mask.b, BELT_COLS, BELT_ROWS, factor);
    assert_eq!((rows, cols), (level.rows(), level.cols()));

    let source = if source_is_b { &image_b } else { &image_a };
    let (mut dx, mut dy) = sobel3(source, cols, rows);
    for i in 0..dx.len() {
        if mask_a[i] == 0 {
            dx[i] = 0.0;
            dy[i] = 0.0;
        }
    }
    let magnitude: Vec<f32> = dx.iter().zip(&dy).map(|(x, y)| x.abs() + y.abs()).collect();
    let mut raw_weight = gaussian_f32(&magnitude, cols, rows, 3, 1.0);
    for i in 0..raw_weight.len() {
        if mask_a[i] == 0 || raw_weight[i] < 0.0 {
            raw_weight[i] = 0.0;
        }
    }
    LevelInputs {
        rows,
        cols,
        image: LensPair {
            a: image_a,
            b: image_b,
        },
        mask: LensPair {
            a: mask_a,
            b: mask_b,
        },
        gradient_col: dx,
        gradient_row: dy,
        raw_weight,
    }
}

/// Section 118C's live classifier, on the cold configuration.
///
/// The per-patch box sum of `GaussianBlur(|Ix|+|Iy|, 3x3, sigma 1)` over the 8-by-8 patch at
/// stride 3; a row is marked when the mean of its entries greater than 1.0 falls below 2000.0.
/// An empty count yields mean 0 and therefore a marked row.
fn lack_of_texture_rows(inputs: &LevelInputs, level: Level) -> Vec<bool> {
    let (pr, pc) = (level.patch_rows(), level.patch_cols());
    let mut marked = Vec::with_capacity(pr);
    for r in 0..pr {
        let (mut sum, mut count) = (0.0f32, 0u32);
        for c in 0..pc {
            let mut box_sum = 0.0f32;
            for dr in 0..one_xs::PATCH_SIZE {
                for dc in 0..one_xs::PATCH_SIZE {
                    let rr = r * one_xs::PATCH_STRIDE + dr;
                    let cc = c * one_xs::PATCH_STRIDE + dc;
                    box_sum += inputs.raw_weight[rr * inputs.cols + cc];
                }
            }
            if box_sum > 1.0 {
                sum += box_sum;
                count += 1;
            }
        }
        let mean = if count > 0 { sum / count as f32 } else { 0.0 };
        marked.push(mean < 2000.0);
    }
    marked
}

#[test]
#[ignore = "needs the rescued V6 capture corpus"]
fn native_inputs_build_a_valid_selected_pis_input_at_both_levels() {
    let retained = retained();
    for lens in [Lens::A, Lens::B] {
        let m = retained.mask.get(lens);
        let covered = m.iter().filter(|&&v| v != 0).count();
        println!(
            "mask {lens:?}: {covered}/{} covered ({:.1}%)",
            m.len(),
            100.0 * covered as f64 / m.len() as f64
        );
    }

    for level in [Level::Two, Level::One] {
        let inputs = level_inputs(&retained, level, false);
        let rows = lack_of_texture_rows(&inputs, level);
        let marked = rows.iter().filter(|&&v| v).count();
        let cost_modes: Vec<CostMode> = rows
            .iter()
            .map(|&low| {
                if low {
                    CostMode::Weighted
                } else {
                    CostMode::Unweighted
                }
            })
            .collect();

        let input = Input::<AtoB>::from_native_order(
            level,
            LensPair {
                a: inputs.image.a.clone(),
                b: inputs.image.b.clone(),
            },
            LensPair {
                a: inputs.mask.a.clone(),
                b: inputs.mask.b.clone(),
            },
            inputs.gradient_col.clone(),
            inputs.gradient_row.clone(),
            inputs.raw_weight.clone(),
            cost_modes,
        )
        .expect("native-derived planes must satisfy the selected PIS input contract");

        println!(
            "{level:?}: {}x{} image, patch grid {}x{}, {marked}/{} rows lack texture -> weighted",
            inputs.rows,
            inputs.cols,
            level.patch_rows(),
            level.patch_cols(),
            rows.len(),
        );
        let _ = input;
    }
}

#[test]
#[ignore = "needs the rescued V6 capture corpus"]
fn cold_l2_solve_runs_on_native_inputs() {
    let retained = retained();
    let level = Level::Two;
    let inputs = level_inputs(&retained, level, false);
    let cost_modes: Vec<CostMode> = lack_of_texture_rows(&inputs, level)
        .into_iter()
        .map(|low| {
            if low {
                CostMode::Weighted
            } else {
                CostMode::Unweighted
            }
        })
        .collect();
    let mask_a_level = inputs.mask.a.clone();
    let input = Input::<AtoB>::from_native_order(
        level,
        LensPair {
            a: inputs.image.a,
            b: inputs.image.b,
        },
        LensPair {
            a: inputs.mask.a,
            b: inputs.mask.b,
        },
        inputs.gradient_col,
        inputs.gradient_row,
        inputs.raw_weight,
        cost_modes,
    )
    .expect("valid input");

    let grid = one_xs::pis::solve(&input, InitialGrid::coarse_zeros(), None).expect("L2 solve");
    let flows: Vec<_> = grid.patches().iter().map(|p| p.flow()).collect();
    let moving = flows
        .iter()
        .filter(|f| f.dcol() != 0.0 || f.drow() != 0.0)
        .count();
    let max_across = flows.iter().map(|f| f.dcol().abs()).fold(0.0f32, f32::max);
    let max_along = flows.iter().map(|f| f.drow().abs()).fold(0.0f32, f32::max);
    let pct = |v: &mut Vec<f32>, q: f64| {
        v.sort_by(f32::total_cmp);
        v[((v.len() - 1) as f64 * q).round() as usize]
    };
    let mut across: Vec<f32> = flows.iter().map(|f| f.dcol().abs()).collect();
    let mut along: Vec<f32> = flows.iter().map(|f| f.drow().abs()).collect();
    println!(
        "cold L2 solve: {moving}/{} moved | across p50 {:.3} p90 {:.3} p99 {:.3} max {max_across:.3}",
        flows.len(),
        pct(&mut across, 0.50),
        pct(&mut across, 0.90),
        pct(&mut across, 0.99),
    );
    println!(
        "                          | along  p50 {:.3} p90 {:.3} p99 {:.3} max {max_along:.3}",
        pct(&mut along, 0.50),
        pct(&mut along, 0.90),
        pct(&mut along, 0.99),
    );
    // Do the runaway patches coincide with thin mask coverage? Section 114G records that
    // native keeps eight-or-fewer-survivor patches in competition on a finite 1e10 sentinel,
    // so low-coverage patches are the first suspect for a heavy tail.
    let (pr, pc) = (level.patch_rows(), level.patch_cols());
    let mut wild = Vec::new();
    let mut calm = Vec::new();
    for r in 0..pr {
        for c in 0..pc {
            let mut survivors = 0usize;
            for dr in 0..one_xs::PATCH_SIZE {
                for dc in 0..one_xs::PATCH_SIZE {
                    let rr = r * one_xs::PATCH_STRIDE + dr;
                    let cc = c * one_xs::PATCH_STRIDE + dc;
                    if mask_a_level[rr * level.cols() + cc] != 0 {
                        survivors += 1;
                    }
                }
            }
            let f = flows[r * pc + c];
            if f.drow().abs() > 8.0 {
                wild.push(survivors)
            } else {
                calm.push(survivors)
            }
        }
    }
    let mean = |v: &Vec<usize>| {
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<usize>() as f64 / v.len() as f64
        }
    };
    println!(
        "runaway patches (|along|>8): {}, mean mask survivors {:.1}/64; calm: {}, mean {:.1}/64",
        wild.len(),
        mean(&wild),
        calm.len(),
        mean(&calm)
    );
    println!("runaway survivor counts: {:?}", {
        let mut w = wild.clone();
        w.sort();
        w
    });
    assert_eq!(flows.len(), level.patches());
}

/// Carry the cold L2 stage through densification and variational refinement, which is what
/// native's final field has been through. The raw patch grid is NOT comparable to the captured
/// field on its own.
#[test]
#[ignore = "needs the rescued V6 capture corpus"]
fn cold_l2_refined_field_is_measured_after_densification() {
    let retained = retained();
    let level = Level::Two;
    let inputs = level_inputs(&retained, level, false);
    let cost_modes: Vec<CostMode> = lack_of_texture_rows(&inputs, level)
        .into_iter()
        .map(|low| {
            if low {
                CostMode::Weighted
            } else {
                CostMode::Unweighted
            }
        })
        .collect();
    let images_a = inputs.image.a.clone();
    let images_b = inputs.image.b.clone();
    let input = Input::<AtoB>::from_native_order(
        level,
        LensPair {
            a: inputs.image.a,
            b: inputs.image.b,
        },
        LensPair {
            a: inputs.mask.a,
            b: inputs.mask.b,
        },
        inputs.gradient_col,
        inputs.gradient_row,
        inputs.raw_weight,
        cost_modes,
    )
    .expect("valid input");

    let grid = one_xs::pis::solve(&input, InitialGrid::coarse_zeros(), None).expect("L2 solve");
    let images = DirectedImages::<AtoB>::from_native_order(
        level,
        LensPair {
            a: &images_a[..],
            b: &images_b[..],
        },
    )
    .expect("directed images");
    let field = dense::densify_coarse(&images, grid).expect("densify");

    let report = |label: &str, dcol: &[f32], drow: &[f32]| {
        let pct = |v: &[f32], q: f64| {
            let mut w: Vec<f32> = v.iter().map(|x| x.abs()).collect();
            w.sort_by(f32::total_cmp);
            w[((w.len() - 1) as f64 * q).round() as usize]
        };
        println!(
            "{label}: across p50 {:.3} p99 {:.3} max {:.3} | along p50 {:.3} p99 {:.3} max {:.3}",
            pct(dcol, 0.50),
            pct(dcol, 0.99),
            pct(dcol, 1.0),
            pct(drow, 0.50),
            pct(drow, 0.99),
            pct(drow, 1.0),
        );
    };
    report("densified L2   ", field.dcol(), field.drow());

    let prep_images = PreparationImages::<AtoB>::from_native_order(
        level,
        LensPair {
            a: StridedImage::new(&images_a, level.cols()),
            b: StridedImage::new(&images_b, level.cols()),
        },
    )
    .expect("preparation images");
    match prepare(&prep_images, field) {
        Ok(prepared) => {
            let refined = refine_prepared(prepared);
            let field = refined.field();
            report("refined L2     ", field.dcol(), field.drow());
            println!("variational rolled back: {}", refined.rolled_back());
        }
        Err(e) => println!("variational preparation refused the dense field: {e}"),
    }
}

/// Section 119D's decisive test: run the cold chain to a full 1080-by-60 public field and
/// measure it against the captured native field, with refinement and without.
///
/// The captured fields are the compositor's own input, so this is the arbiter both for how far
/// the reconstruction is from native and for whether variational refinement does anything on
/// this route.
#[test]
#[ignore = "needs the rescued V6 capture corpus"]
fn cold_chain_to_public_field_measured_against_native() {
    use super::l2_seed::into_l1_initial_grid;
    use super::post_update::preserve_without_retained;
    use super::temporal_median::TemporalMedians;

    let retained = retained();

    let build = |level: Level| {
        let inputs = level_inputs(&retained, level, false);
        let cost_modes: Vec<CostMode> = lack_of_texture_rows(&inputs, level)
            .into_iter()
            .map(|low| {
                if low {
                    CostMode::Weighted
                } else {
                    CostMode::Unweighted
                }
            })
            .collect();
        let (ia, ib) = (inputs.image.a.clone(), inputs.image.b.clone());
        let input = Input::<AtoB>::from_native_order(
            level,
            LensPair {
                a: inputs.image.a,
                b: inputs.image.b,
            },
            LensPair {
                a: inputs.mask.a,
                b: inputs.mask.b,
            },
            inputs.gradient_col,
            inputs.gradient_row,
            inputs.raw_weight,
            cost_modes,
        )
        .expect("valid input")
        .with_disparity_interval(one_xs::selected_pis_interval(
            one_xs::Direction::AtoB,
            level,
        ));
        (input, ia, ib)
    };

    // L2, cold.
    let (l2_input, l2_a, l2_b) = build(Level::Two);
    let l2_grid = one_xs::pis::solve(&l2_input, InitialGrid::coarse_zeros(), None).expect("L2");
    let l2_images = DirectedImages::<AtoB>::from_native_order(
        Level::Two,
        LensPair {
            a: &l2_a[..],
            b: &l2_b[..],
        },
    )
    .expect("L2 images");
    let l2_dense = dense::densify_coarse(&l2_images, l2_grid).expect("densify L2");
    let l2_prep_images = PreparationImages::<AtoB>::from_native_order(
        Level::Two,
        LensPair {
            a: StridedImage::new(&l2_a, Level::Two.cols()),
            b: StridedImage::new(&l2_b, Level::Two.cols()),
        },
    )
    .expect("L2 preparation images");
    let l2_refined = refine_prepared(prepare(&l2_prep_images, l2_dense).expect("L2 prepare"));
    println!("L2 variational rolled back: {}", l2_refined.rolled_back());
    let l2_post = preserve_without_retained(l2_refined);
    let l1_initial = into_l1_initial_grid(l2_post).expect("L2 seed");

    // L1, cold, through the finest-level temporal median with empty history.
    let (l1_input, l1_a, l1_b) = build(Level::One);
    let l1_grid = one_xs::pis::solve(&l1_input, l1_initial, None).expect("L1");
    let mut medians = TemporalMedians::new();
    let (a_median, _) = medians.split_mut();
    let filtered = a_median.run(l1_grid).expect("finest median");
    let l1_images = DirectedImages::<AtoB>::from_native_order(
        Level::One,
        LensPair {
            a: &l1_a[..],
            b: &l1_b[..],
        },
    )
    .expect("L1 images");
    let l1_dense = dense::densify_finest(&l1_images, filtered).expect("densify L1");
    let l1_prep_images = PreparationImages::<AtoB>::from_native_order(
        Level::One,
        LensPair {
            a: StridedImage::new(&l1_a, Level::One.cols()),
            b: StridedImage::new(&l1_b, Level::One.cols()),
        },
    )
    .expect("L1 preparation images");
    let l1_refined = refine_prepared(prepare(&l1_prep_images, l1_dense).expect("L1 prepare"));
    println!("L1 variational rolled back: {}", l1_refined.rolled_back());
    let public = dense::finish_linear_x2(preserve_without_retained(l1_refined)).expect("finish");

    // A->B at +0xc30 is the direction we built (source lens A).
    let native = read_f32(
        "00044_target_ordinary_flow_right_c30.bin",
        BELT_ROWS * BELT_COLS * 2,
    );
    let (ours_col, ours_row) = (public.dcol(), public.drow());
    assert_eq!(ours_col.len(), BELT_ROWS * BELT_COLS);

    let (mut se_col, mut se_row, mut n) = (0.0f64, 0.0f64, 0usize);
    let (mut max_col, mut max_row) = (0.0f32, 0.0f32);
    for i in 0..ours_col.len() {
        let (nc, nr) = (native[i * 2], native[i * 2 + 1]);
        let (oc, or) = (ours_col[i], ours_row[i]);
        if !(oc.is_finite() && or.is_finite() && nc.is_finite() && nr.is_finite()) {
            continue;
        }
        se_col += f64::from(oc - nc).powi(2);
        se_row += f64::from(or - nr).powi(2);
        max_col = max_col.max((oc - nc).abs());
        max_row = max_row.max((or - nr).abs());
        n += 1;
    }
    // §121B predicts native's field is a long temporal average and ours a single cold frame, so
    // native should be measurably SMOOTHER in space. Roughness = mean |grad| / std, which is scale
    // free, so it compares fields of different magnitude fairly.
    {
        let rough = |v: &[f32]| {
            let (r, c) = (BELT_ROWS, BELT_COLS);
            let mean = v.iter().filter(|x| x.is_finite()).sum::<f32>() / v.len() as f32;
            let var = v
                .iter()
                .filter(|x| x.is_finite())
                .map(|x| (x - mean).powi(2))
                .sum::<f32>()
                / v.len() as f32;
            let mut acc = 0.0f64;
            let mut cnt = 0u32;
            for y in 1..r - 1 {
                for x in 1..c - 1 {
                    let (a, b) = (v[y * c + x + 1], v[y * c + x - 1]);
                    let (u2, d2) = (v[(y + 1) * c + x], v[(y - 1) * c + x]);
                    if a.is_finite() && b.is_finite() && u2.is_finite() && d2.is_finite() {
                        acc +=
                            f64::from((((a - b) * 0.5).powi(2) + ((u2 - d2) * 0.5).powi(2)).sqrt());
                        cnt += 1;
                    }
                }
            }
            (acc / f64::from(cnt.max(1))) / f64::from(var.sqrt().max(1e-9))
        };
        let native_col: Vec<f32> = (0..BELT_ROWS * BELT_COLS).map(|i| native[i * 2]).collect();
        println!(
            "spatial roughness (mean|grad|/std): ours {:.4}, native {:.4}",
            rough(ours_col),
            rough(&native_col),
        );
    }

    println!(
        "vs native A->B over {n}/{} finite nodes: across RMS {:.3} max {max_col:.3} | along RMS {:.3} max {max_row:.3}",
        ours_col.len(),
        (se_col / n as f64).sqrt(),
        (se_row / n as f64).sqrt(),
    );

    // DIAGNOSTIC ONLY, not a mechanism: the exact per-level gate already runs inside PIS. This
    // separate output clamp asks how much of the remaining error lies outside the captured field's
    // observed range. It measures the size of that description and changes no solver behavior.
    let (mut nlo_c, mut nhi_c, mut nlo_r, mut nhi_r) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for i in 0..ours_col.len() {
        nlo_c = nlo_c.min(native[i * 2]);
        nhi_c = nhi_c.max(native[i * 2]);
        nlo_r = nlo_r.min(native[i * 2 + 1]);
        nhi_r = nhi_r.max(native[i * 2 + 1]);
    }
    let (mut se_col2, mut se_row2, mut n2) = (0.0f64, 0.0f64, 0usize);
    for i in 0..ours_col.len() {
        let (nc, nr) = (native[i * 2], native[i * 2 + 1]);
        let (oc, or) = (ours_col[i], ours_row[i]);
        if !(oc.is_finite() && or.is_finite()) {
            continue;
        }
        let cc = oc.clamp(nlo_c, nhi_c);
        let cr = or.clamp(nlo_r, nhi_r);
        se_col2 += f64::from(cc - nc).powi(2);
        se_row2 += f64::from(cr - nr).powi(2);
        n2 += 1;
    }
    // Is the across residual structured or noise? Native's own column profile is smooth, so a
    // systematic offset points at a bias in the chain while scatter points at matching quality.
    let mut ours_prof = vec![0.0f64; BELT_COLS];
    let mut theirs_prof = vec![0.0f64; BELT_COLS];
    let mut counts = vec![0u32; BELT_COLS];
    for r in 0..BELT_ROWS {
        for c in 0..BELT_COLS {
            let i = r * BELT_COLS + c;
            if ours_col[i].is_finite() {
                ours_prof[c] += f64::from(ours_col[i]);
                theirs_prof[c] += f64::from(native[i * 2]);
                counts[c] += 1;
            }
        }
    }
    let fmt = |v: &[f64], n: &[u32]| {
        (0..BELT_COLS)
            .step_by(6)
            .map(|c| format!("{:+.2}", v[c] / f64::from(n[c].max(1))))
            .collect::<Vec<_>>()
            .join(" ")
    };
    println!("across col-profile ours  : {}", fmt(&ours_prof, &counts));
    println!("across col-profile native: {}", fmt(&theirs_prof, &counts));

    println!(
        "clamped to native's own range [{nlo_c:.3},{nhi_c:.3}]x[{nlo_r:.3},{nhi_r:.3}] over {n2}: across RMS {:.3} | along RMS {:.3}",
        (se_col2 / n2 as f64).sqrt(),
        (se_row2 / n2 as f64).sqrt(),
    );
}

/// The same cold chain for the opposite direction, testing section 120L's exchange claim.
///
/// If the interval really is written into the two FDS instances with the pair exchanged, then
/// B-to-A must want the mirrored interval, and running it with that mirror should behave like
/// A-to-B did rather than degrading.
#[test]
#[ignore = "needs the rescued V6 capture corpus"]
fn cold_chain_reverse_direction_tests_the_exchange_claim() {
    use super::l2_seed::into_l1_initial_grid;
    use super::post_update::preserve_without_retained;
    use super::temporal_median::TemporalMedians;

    let retained = retained();

    let build = |level: Level| {
        let inputs = level_inputs(&retained, level, true);
        let cost_modes: Vec<CostMode> = lack_of_texture_rows(&inputs, level)
            .into_iter()
            .map(|low| {
                if low {
                    CostMode::Weighted
                } else {
                    CostMode::Unweighted
                }
            })
            .collect();
        let (ia, ib) = (inputs.image.a.clone(), inputs.image.b.clone());
        let input = Input::<BtoA>::from_native_order(
            level,
            LensPair {
                a: inputs.image.a,
                b: inputs.image.b,
            },
            LensPair {
                a: inputs.mask.a,
                b: inputs.mask.b,
            },
            inputs.gradient_col,
            inputs.gradient_row,
            inputs.raw_weight,
            cost_modes,
        )
        .expect("valid input")
        .with_disparity_interval(one_xs::selected_pis_interval(
            one_xs::Direction::BtoA,
            level,
        ));
        (input, ia, ib)
    };

    // L2, cold.
    let (l2_input, l2_a, l2_b) = build(Level::Two);
    let l2_grid = one_xs::pis::solve(&l2_input, InitialGrid::coarse_zeros(), None).expect("L2");
    let l2_images = DirectedImages::<BtoA>::from_native_order(
        Level::Two,
        LensPair {
            a: &l2_a[..],
            b: &l2_b[..],
        },
    )
    .expect("L2 images");
    let l2_dense = dense::densify_coarse(&l2_images, l2_grid).expect("densify L2");
    let l2_prep_images = PreparationImages::<BtoA>::from_native_order(
        Level::Two,
        LensPair {
            a: StridedImage::new(&l2_a, Level::Two.cols()),
            b: StridedImage::new(&l2_b, Level::Two.cols()),
        },
    )
    .expect("L2 preparation images");
    let l2_refined = refine_prepared(prepare(&l2_prep_images, l2_dense).expect("L2 prepare"));
    println!("L2 variational rolled back: {}", l2_refined.rolled_back());
    let l2_post = preserve_without_retained(l2_refined);
    let l1_initial = into_l1_initial_grid(l2_post).expect("L2 seed");

    // L1, cold, through the finest-level temporal median with empty history.
    let (l1_input, l1_a, l1_b) = build(Level::One);
    let l1_grid = one_xs::pis::solve(&l1_input, l1_initial, None).expect("L1");
    let mut medians = TemporalMedians::new();
    let (_, a_median) = medians.split_mut();
    let filtered = a_median.run(l1_grid).expect("finest median");
    let l1_images = DirectedImages::<BtoA>::from_native_order(
        Level::One,
        LensPair {
            a: &l1_a[..],
            b: &l1_b[..],
        },
    )
    .expect("L1 images");
    let l1_dense = dense::densify_finest(&l1_images, filtered).expect("densify L1");
    let l1_prep_images = PreparationImages::<BtoA>::from_native_order(
        Level::One,
        LensPair {
            a: StridedImage::new(&l1_a, Level::One.cols()),
            b: StridedImage::new(&l1_b, Level::One.cols()),
        },
    )
    .expect("L1 preparation images");
    let l1_refined = refine_prepared(prepare(&l1_prep_images, l1_dense).expect("L1 prepare"));
    println!("L1 variational rolled back: {}", l1_refined.rolled_back());
    let public = dense::finish_linear_x2(preserve_without_retained(l1_refined)).expect("finish");

    // A->B at +0xc30 is the direction we built (source lens A).
    let native = read_f32(
        "00043_target_ordinary_flow_left_c90.bin",
        BELT_ROWS * BELT_COLS * 2,
    );
    let (ours_col, ours_row) = (public.dcol(), public.drow());
    assert_eq!(ours_col.len(), BELT_ROWS * BELT_COLS);

    let (mut se_col, mut se_row, mut n) = (0.0f64, 0.0f64, 0usize);
    let (mut max_col, mut max_row) = (0.0f32, 0.0f32);
    for i in 0..ours_col.len() {
        let (nc, nr) = (native[i * 2], native[i * 2 + 1]);
        let (oc, or) = (ours_col[i], ours_row[i]);
        if !(oc.is_finite() && or.is_finite() && nc.is_finite() && nr.is_finite()) {
            continue;
        }
        se_col += f64::from(oc - nc).powi(2);
        se_row += f64::from(or - nr).powi(2);
        max_col = max_col.max((oc - nc).abs());
        max_row = max_row.max((or - nr).abs());
        n += 1;
    }
    println!(
        "vs native B->A over {n}/{} finite nodes: across RMS {:.3} max {max_col:.3} | along RMS {:.3} max {max_row:.3}",
        ours_col.len(),
        (se_col / n as f64).sqrt(),
        (se_row / n as f64).sqrt(),
    );

    // DIAGNOSTIC ONLY, not a mechanism: the exact per-level gate already runs inside PIS. This
    // separate output clamp asks how much of the remaining error lies outside the captured field's
    // observed range. It measures the size of that description and changes no solver behavior.
    let (mut nlo_c, mut nhi_c, mut nlo_r, mut nhi_r) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for i in 0..ours_col.len() {
        nlo_c = nlo_c.min(native[i * 2]);
        nhi_c = nhi_c.max(native[i * 2]);
        nlo_r = nlo_r.min(native[i * 2 + 1]);
        nhi_r = nhi_r.max(native[i * 2 + 1]);
    }
    let (mut se_col2, mut se_row2, mut n2) = (0.0f64, 0.0f64, 0usize);
    for i in 0..ours_col.len() {
        let (nc, nr) = (native[i * 2], native[i * 2 + 1]);
        let (oc, or) = (ours_col[i], ours_row[i]);
        if !(oc.is_finite() && or.is_finite()) {
            continue;
        }
        let cc = oc.clamp(nlo_c, nhi_c);
        let cr = or.clamp(nlo_r, nhi_r);
        se_col2 += f64::from(cc - nc).powi(2);
        se_row2 += f64::from(cr - nr).powi(2);
        n2 += 1;
    }
    // Is the across residual structured or noise? Native's own column profile is smooth, so a
    // systematic offset points at a bias in the chain while scatter points at matching quality.
    let mut ours_prof = vec![0.0f64; BELT_COLS];
    let mut theirs_prof = vec![0.0f64; BELT_COLS];
    let mut counts = vec![0u32; BELT_COLS];
    for r in 0..BELT_ROWS {
        for c in 0..BELT_COLS {
            let i = r * BELT_COLS + c;
            if ours_col[i].is_finite() {
                ours_prof[c] += f64::from(ours_col[i]);
                theirs_prof[c] += f64::from(native[i * 2]);
                counts[c] += 1;
            }
        }
    }
    let fmt = |v: &[f64], n: &[u32]| {
        (0..BELT_COLS)
            .step_by(6)
            .map(|c| format!("{:+.2}", v[c] / f64::from(n[c].max(1))))
            .collect::<Vec<_>>()
            .join(" ")
    };
    println!("across col-profile ours  : {}", fmt(&ours_prof, &counts));
    println!("across col-profile native: {}", fmt(&theirs_prof, &counts));

    println!(
        "clamped to native's own range [{nlo_c:.3},{nhi_c:.3}]x[{nlo_r:.3},{nhi_r:.3}] over {n2}: across RMS {:.3} | along RMS {:.3}",
        (se_col2 / n2 as f64).sqrt(),
        (se_row2 / n2 as f64).sqrt(),
    );
}
