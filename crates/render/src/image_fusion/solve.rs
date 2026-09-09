//! Readable reference for the inner selected Windows X4 MGP2 solve.
//!
//! This begins at the two aligned `212 x 100` BGR8 working images and ends at
//! copies of those same-ordinal images with MGP2's solved Y/Cb/Cr offsets
//! applied. It deliberately does not implement Studio's outer content gate,
//! lens sampling, ratio construction, blur, give-back, coordinate remap, or
//! ratio-map application. It is neither a Mac 6.0.2 equivalence claim nor the
//! shipping GPU implementation.
//!
//! The support law is `0x183c1a7a0`, admission is
//! `0x183c1cb20..0x183c1d32c`, the normal equations and solve are
//! `0x183c1c4a0..0x183c1e9c0`, and the BGR byte consumer is
//! `0x183c18fa0`. The existing Rust sparse traversal and reduction order are
//! readable equivalents, not bit-exact reproductions of Studio's truncated
//! warm-started solve.

use super::super::{chroma, chromatic};

const IMAGE_PIXELS: usize = chroma::COLUMNS * chroma::MAP_ROWS;
const IMAGE_BYTES: usize = 3 * IMAGE_PIXELS;
const SAMPLE_TOP: usize = 48;
const SAMPLE_ROWS: usize = 4;
const VALIDITY_BYTES: usize = chroma::COLUMNS * SAMPLE_ROWS;
const SUPPORT_ROWS: std::ops::Range<usize> = 49..51;

#[cfg(test)]
#[path = "solve_arithmetic_probe.rs"]
mod arithmetic_probe;

/// Shape-checked inputs to one inner selected-X4 observation.
pub struct Inputs<'a> {
    lenses: [&'a [u8]; 2],
    /// Nonzero excludes the matching pixel in rows 48 through 51. The owner
    /// folds this into a sticky invalid-coordinate mask across observations.
    invalid: &'a [u8],
}

impl<'a> Inputs<'a> {
    pub fn new(left: &'a [u8], right: &'a [u8], invalid: &'a [u8]) -> Result<Self, String> {
        for (name, image) in [("left", left), ("right", right)] {
            if image.len() != IMAGE_BYTES {
                return Err(format!(
                    "selected X4 fusion {name} image has {} bytes, expected {IMAGE_BYTES}",
                    image.len()
                ));
            }
        }
        validate_invalid(invalid)?;
        Ok(Self {
            lenses: [left, right],
            invalid,
        })
    }
}

pub(super) fn validate_invalid(invalid: &[u8]) -> Result<(), String> {
    if invalid.len() != VALIDITY_BYTES {
        return Err(format!(
            "selected X4 fusion validity has {} bytes, expected {VALIDITY_BYTES}",
            invalid.len()
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostics {
    /// The unfiltered metric `m`; this, not `retained_metric`, sets a fresh
    /// continuing solve's 5-to-25 iteration budget.
    pub current_metric: Option<u16>,
    pub retained_metric: i32,
    pub budget: Option<u32>,
    pub admitted: usize,
    pub used_stale_control: bool,
    pub exits: [Option<chromatic::Exit>; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    /// BGR8 images in the same ordinal order as the inputs.
    pub prepared: [Vec<u8>; 2],
    pub diagnostics: Diagnostics,
}

/// Retained state of the selected inner MGP2 helper.
pub struct Reference {
    metric: chromatic::Metric,
    retained_budget: u32,
    /// A valid update seeds the retained metric directly while this is set.
    /// Invalid stale-control solves do not clear it.
    first_valid: bool,
    /// The channel vector is populated by any actual solve, including an
    /// invalid post-reset solve which reuses stale control.
    seeds_populated: bool,
    /// Selected dynamic distance/control bytes over rows 49 and 50 and all
    /// columns. They survive invalid/empty observations and reset.
    control: Option<Vec<u8>>,
    /// The outer validity input is monotone for the lifetime of its owner.
    sticky_invalid: Vec<u8>,
    /// One centred warm-start vector per Y/Cb/Cr channel.
    fields: [Vec<f32>; 3],
}

impl Default for Reference {
    fn default() -> Self {
        Self::new()
    }
}

impl Reference {
    pub fn new() -> Self {
        Self {
            // The constructor owns metric 20 and its corresponding 0.2 scale.
            // A first valid update replaces it directly rather than filtering.
            metric: chromatic::Metric::seed(20.0),
            // Constructor iteration control is independent of that metric. A
            // first actual solve still uses the cold budget below.
            retained_budget: 1,
            first_valid: true,
            seeds_populated: false,
            control: None,
            sticky_invalid: vec![0; VALIDITY_BYTES],
            fields: std::array::from_fn(|_| vec![0.0; chroma::NODES]),
        }
    }

    /// Selected reset clears channel seeds and makes the next valid update a
    /// first update. It does not discard the allocated dynamic control mask or
    /// its retained scalar and iteration control. The next valid update seeds
    /// the scalar directly because `first_valid` is set. An intervening empty
    /// update consumes the retained control with a cold solve, populating the
    /// channel seeds without clearing that separate metric flag.
    pub fn reset(&mut self) {
        self.first_valid = true;
        self.seeds_populated = false;
        for field in &mut self.fields {
            field.fill(0.0);
        }
    }

    pub(super) fn accumulate_invalid(&mut self, invalid: &[u8]) {
        debug_assert_eq!(invalid.len(), VALIDITY_BYTES);
        for (held, &current) in self.sticky_invalid.iter_mut().zip(invalid) {
            *held |= current;
        }
    }

    pub fn observe(&mut self, input: Inputs<'_>) -> Output {
        self.accumulate_invalid(input.invalid);

        let support = support_masks(input.lenses, &self.sticky_invalid);
        let differences = supported_byte_differences(input.lenses, &support);
        let current_metric = metric(&differences);
        let mut used_stale_control = false;

        let active_budget = if let Some(m) = current_metric {
            if self.first_valid {
                self.metric = chromatic::Metric::seed(f32::from(m));
            } else {
                self.metric.advance(f32::from(m));
            }
            let retained = self.metric;
            let distances = distances(&differences, retained.level());
            self.control = Some(distances);
            self.retained_budget = chromatic::budget(f32::from(m));
            self.first_valid = false;
            Some(if self.seeds_populated {
                self.retained_budget
            } else {
                chromatic::COLD_BUDGET
            })
        } else if self.control.is_some() {
            used_stale_control = true;
            Some(if self.seeds_populated {
                self.retained_budget
            } else {
                chromatic::COLD_BUDGET
            })
        } else {
            None
        };

        let mut prepared = [input.lenses[0].to_vec(), input.lenses[1].to_vec()];
        let mut exits = [None; 3];
        let mut admitted = 0;
        if let (Some(control), Some(budget)) = (&self.control, active_budget) {
            let scale = self.metric.scale();
            let samples = samples(input.lenses, control, scale);
            admitted = samples.len();
            let system = chroma::System::new(&samples);
            for (channel, field) in self.fields.iter_mut().enumerate() {
                let q = system.rhs(&samples, channel);
                exits[channel] = Some(chroma::solve(&system, &q, field, budget));
            }
            self.seeds_populated = true;
            apply_fields(&mut prepared, &self.fields);
        }

        Output {
            prepared,
            diagnostics: Diagnostics {
                current_metric,
                retained_metric: self.metric.level(),
                budget: active_budget,
                admitted,
                used_stale_control,
                exits,
            },
        }
    }
}

#[derive(Clone, Copy)]
struct Support {
    correlated: bool,
    strict: bool,
}

fn support_at(lenses: [&[u8]; 2], row: usize, column: usize) -> Support {
    if row == 0 || row + 1 >= chroma::MAP_ROWS || column == 0 || column + 1 >= chroma::COLUMNS {
        return Support {
            correlated: false,
            strict: false,
        };
    }
    let mut ea = 1.0e-8f32;
    let mut eb = 1.0e-8f32;
    let mut cross = 0.0f32;
    let mut squared_error = 0.0f32;
    for y in row - 1..=row + 1 {
        for x in column - 1..=column + 1 {
            for channel in 0..3 {
                let a = f32::from(pixel(lenses[0], y, x)[channel]);
                let b = f32::from(pixel(lenses[1], y, x)[channel]);
                ea += a * a;
                eb += b * b;
                cross += a * b;
                let d = a - b;
                squared_error += d * d;
            }
        }
    }
    let correlation = cross / (ea * eb).sqrt();
    let mse = squared_error / 27.0;
    let correlated = correlation > 0.96;
    Support {
        correlated,
        strict: correlated && mse < 6400.0,
    }
}

/// Model 23 first builds strict and correlation-only masks. When no more than
/// half of correlation support survives the MSE test, correlation-only support
/// replaces the strict mask (`0x183c1ac89..0x183c1aced`).
fn support_masks(lenses: [&[u8]; 2], invalid: &[u8]) -> Vec<bool> {
    let mut measured = Vec::with_capacity(2 * chroma::COLUMNS);
    let mut correlated = 0usize;
    let mut strict = 0usize;
    for row in SUPPORT_ROWS {
        for column in 0..chroma::COLUMNS {
            let support = support_at(lenses, row, column);
            correlated += usize::from(support.correlated);
            strict += usize::from(support.strict);
            measured.push(support);
        }
    }
    let use_correlation = (strict as f64 / (correlated as f64 + 1.0e-6)) as f32 <= 0.5;
    measured
        .into_iter()
        .enumerate()
        .map(|(slot, support)| {
            let row = SUPPORT_ROWS.start + slot / chroma::COLUMNS;
            let column = slot % chroma::COLUMNS;
            if invalid[validity_slot(row, column)] != 0 {
                false
            } else if use_correlation {
                support.correlated
            } else {
                support.strict
            }
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Difference {
    slot: usize,
    bgr: [i16; 3],
}

fn supported_byte_differences(lenses: [&[u8]; 2], support: &[bool]) -> Vec<Difference> {
    let mut out = Vec::new();
    for row in SUPPORT_ROWS {
        for column in chroma::EVIDENCE_COLUMNS {
            let slot = evidence_slot(row, column);
            if !support[slot] {
                continue;
            }
            let zero = pixel(lenses[0], row, column);
            let one = pixel(lenses[1], row, column);
            out.push(Difference {
                slot,
                bgr: std::array::from_fn(|channel| {
                    i16::from(one[channel]) - i16::from(zero[channel])
                }),
            });
        }
    }
    out
}

fn metric(differences: &[Difference]) -> Option<u16> {
    if differences.is_empty() {
        return None;
    }
    let rank = chromatic::trim(differences.len());
    let mut worst = 0i16;
    for channel in 0..3 {
        let mut sorted: Vec<i16> = differences.iter().map(|d| d.bgr[channel]).collect();
        sorted.sort_unstable();
        worst = worst.max(sorted[rank].abs());
        worst = worst.max(sorted[sorted.len() - rank - 1].abs());
    }
    Some(worst as u16)
}

fn distances(differences: &[Difference], retained_metric: i32) -> Vec<u8> {
    let mut control = vec![0u8; 2 * chroma::COLUMNS];
    let rank = chromatic::trim(differences.len());
    let mut low = [0i16; 3];
    let mut high = [0i16; 3];
    for channel in 0..3 {
        let mut sorted: Vec<i16> = differences.iter().map(|d| d.bgr[channel]).collect();
        sorted.sort_unstable();
        low[channel] = sorted[rank] - 10;
        high[channel] = sorted[sorted.len() - rank - 1] + 10;
    }
    let lean: i32 = low
        .iter()
        .chain(high.iter())
        .map(|&endpoint| i32::from(endpoint))
        .sum();
    if lean >= 0 {
        low.iter_mut()
            .for_each(|endpoint| *endpoint = (*endpoint).min(0));
    } else {
        high.iter_mut()
            .for_each(|endpoint| *endpoint = (*endpoint).max(0));
    }

    for difference in differences {
        let excess = (0..3)
            .map(|channel| {
                (i32::from(difference.bgr[channel]) - i32::from(high[channel]))
                    .max(i32::from(low[channel]) - i32::from(difference.bgr[channel]))
            })
            .max()
            .unwrap_or(0);
        control[difference.slot] = if excess > retained_metric {
            0
        } else if excess <= 0 {
            1
        } else {
            excess.max(1) as u8
        };
    }
    control
}

fn samples(lenses: [&[u8]; 2], control: &[u8], scale: f32) -> Vec<chroma::Sample> {
    let mut out = Vec::new();
    for row in SUPPORT_ROWS {
        let window_row = row - chroma::ROI_TOP;
        for column in chroma::EVIDENCE_COLUMNS {
            let distance = control[evidence_slot(row, column)];
            if distance == 0 {
                continue;
            }
            let zero = bgr_to_ycc(pixel(lenses[0], row, column));
            let one = bgr_to_ycc(pixel(lenses[1], row, column));
            out.push(chroma::Sample {
                k0: chroma::node(0, window_row, column).expect("selected evidence is in lens zero"),
                k1: chroma::node(1, window_row, column).expect("selected evidence is in lens one"),
                weight: if distance == 1 {
                    1.0
                } else {
                    (f32::from(distance) - 1.0) * scale + 1.0
                },
                difference: std::array::from_fn(|channel| one[channel] - zero[channel]),
            });
        }
    }
    out
}

fn apply_fields(prepared: &mut [Vec<u8>; 2], fields: &[Vec<f32>; 3]) {
    for (lens, image) in prepared.iter_mut().enumerate() {
        for window_row in chroma::block(lens) {
            let row = chroma::ROI_TOP + window_row;
            for column in 0..chroma::COLUMNS {
                let node = chroma::node(lens, window_row, column).expect("inside selected block");
                let mut ycc = bgr_to_ycc(pixel(image, row, column));
                for channel in 0..3 {
                    ycc[channel] += fields[channel][node];
                }
                let corrected = ycc_to_bgr(ycc);
                let at = pixel_offset(row, column);
                image[at..at + 3].copy_from_slice(&corrected);
            }
        }
    }
}

fn bgr_to_ycc(bgr: &[u8]) -> [f32; 3] {
    let [b, g, r] = [f32::from(bgr[0]), f32::from(bgr[1]), f32::from(bgr[2])];
    let y = r * 0.299 + g * 0.587 + b * 0.114;
    let cb = r * -0.168_736 - g * 0.331_264 + b * 0.5 + 128.0;
    let cr = r * 0.5 - g * 0.418_688 - b * 0.081_312 + 128.0;
    [y, cb, cr]
}

fn ycc_to_bgr(ycc: [f32; 3]) -> [u8; 3] {
    let [y, cb, cr] = ycc;
    let dcb = cb - 128.0;
    let dcr = cr - 128.0;
    [
        byte(dcb * 1.772 + y),
        byte(y - dcb * 0.344_14 - dcr * 0.714_14),
        byte(dcr * 1.402 + y),
    ]
}

fn byte(value: f32) -> u8 {
    if !value.is_finite() || value < i32::MIN as f32 || value >= i32::MAX as f32 {
        return 0;
    }
    (value.round_ties_even() as i32).clamp(0, 255) as u8
}

fn pixel(image: &[u8], row: usize, column: usize) -> &[u8] {
    let at = pixel_offset(row, column);
    &image[at..at + 3]
}

const fn pixel_offset(row: usize, column: usize) -> usize {
    3 * (row * chroma::COLUMNS + column)
}

const fn validity_slot(row: usize, column: usize) -> usize {
    (row - SAMPLE_TOP) * chroma::COLUMNS + column
}

const fn evidence_slot(row: usize, column: usize) -> usize {
    (row - SUPPORT_ROWS.start) * chroma::COLUMNS + column
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Isolate the recovered Mac node permutation without changing physical
    /// equations, support, sparse accumulation, centering or warm history.
    #[test]
    #[ignore = "requires the native prepared-stage replay inputs"]
    fn replay_native_node_order() {
        use std::{fs, io::Write, path::PathBuf};
        let root = PathBuf::from(std::env::var_os("KJERAG_FUSION_NODE_ORDER_INPUT").unwrap());
        let output = PathBuf::from(std::env::var_os("KJERAG_FUSION_NODE_ORDER_OUTPUT").unwrap());
        fs::create_dir(&output).expect("node-order output must be a new directory");
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
        let mut fields: [Vec<f32>; 3] = std::array::from_fn(|_| vec![0.0; chroma::NODES]);
        let mut report = fs::File::create_new(output.join("report.tsv")).unwrap();
        writeln!(
            report,
            "frame\tlens\tchanged_prepared_bytes\tmax_prepared_byte_delta"
        )
        .unwrap();
        // This fixture contains the three authenticated admitted observations.
        // Its skipped observations have zero invalidity and do not advance MGP.
        for frame in [0, 7, 13] {
            let current = [0, 1].map(|lens| {
                fs::read(root.join(format!("frame-{frame:03}.reference-current-{lens}.bgr8")))
                    .unwrap()
            });
            let valid = vec![0; VALIDITY_BYTES];
            let ordinary = reference.observe(input(&current[0], &current[1], &valid));
            let samples = samples(
                [&current[0], &current[1]],
                reference.control.as_ref().unwrap(),
                reference.metric.scale(),
            );
            let system = chroma::System::new(&samples);
            let pre = system.preconditioner();
            let native_pre: Vec<_> = order.iter().map(|&i| pre[i]).collect();
            for (channel, field) in fields.iter_mut().enumerate() {
                let q = system.rhs(&samples, channel);
                let native_q: Vec<_> = order.iter().map(|&i| q[i]).collect();
                let mut native_x: Vec<_> = order.iter().map(|&i| field[i]).collect();
                chromatic::conjugate_gradient(
                    |x, y| {
                        let physical: Vec<_> = inverse.iter().map(|&i| x[i]).collect();
                        let mut product = vec![0.0; chroma::NODES];
                        system.apply(&physical, &mut product);
                        for (native, &i) in order.iter().enumerate() {
                            y[native] = product[i];
                        }
                    },
                    &native_pre,
                    &native_q,
                    &mut native_x,
                    ordinary.diagnostics.budget.unwrap(),
                );
                for (native, &i) in order.iter().enumerate() {
                    field[i] = native_x[native];
                }
                chroma::centre(field);
            }
            let mut prepared = current;
            apply_fields(&mut prepared, &fields);
            for (lens, image) in prepared.iter().enumerate() {
                let baseline =
                    fs::read(root.join(format!("frame-{frame:03}.reference-prepared-{lens}.bgr8")))
                        .unwrap();
                assert_eq!(baseline, ordinary.prepared[lens]);
                let differences: Vec<_> = image
                    .iter()
                    .zip(&baseline)
                    .map(|(&a, &b)| a.abs_diff(b))
                    .collect();
                writeln!(
                    report,
                    "{frame}\t{lens}\t{}\t{}",
                    differences.iter().filter(|&&d| d != 0).count(),
                    differences.iter().max().unwrap()
                )
                .unwrap();
                fs::write(
                    output.join(format!("frame-{frame:03}.permuted-prepared-{lens}.bgr8")),
                    image,
                )
                .unwrap();
            }
        }
    }

    fn image(value: u8) -> Vec<u8> {
        vec![value; IMAGE_BYTES]
    }

    fn input<'a>(left: &'a [u8], right: &'a [u8], invalid: &'a [u8]) -> Inputs<'a> {
        Inputs::new(left, right, invalid).unwrap()
    }

    #[test]
    fn first_invalid_observation_is_byte_identical_and_neutral() {
        let left = image(73);
        let right = image(119);
        let invalid = vec![1; VALIDITY_BYTES];
        let output = Reference::new().observe(input(&left, &right, &invalid));
        assert_eq!(output.prepared, [left, right]);
        assert_eq!(output.diagnostics.current_metric, None);
        assert_eq!(output.diagnostics.retained_metric, 20);
        assert_eq!(output.diagnostics.budget, None);
        assert_eq!(output.diagnostics.admitted, 0);
        assert!(!output.diagnostics.used_stale_control);
    }

    #[test]
    fn support_uses_strict_correlation_and_mse_boundaries() {
        let equal = image(100);
        let plus_79 = image(179);
        let plus_80 = image(180);
        let correlated = support_at([&equal, &plus_79], 49, 18);
        assert!(correlated.correlated);
        assert!(correlated.strict, "79-code error is below 6400 MSE");
        let boundary = support_at([&equal, &plus_80], 49, 18);
        assert!(boundary.correlated);
        assert!(
            !boundary.strict,
            "6400 is excluded by the strict comparison"
        );
        assert!(!support_at([&equal, &equal], 0, 18).correlated);
        assert!(!support_at([&equal, &equal], 49, 0).correlated);
    }

    #[test]
    fn continuing_budget_uses_current_metric_not_filtered_metric() {
        let left = image(100);
        let first_right = image(120);
        let second_right = image(144);
        let valid = vec![0; VALIDITY_BYTES];
        let mut reference = Reference::new();
        let first = reference.observe(input(&left, &first_right, &valid));
        assert_eq!(first.diagnostics.budget, Some(chromatic::COLD_BUDGET));
        assert_eq!(first.diagnostics.retained_metric, 20);
        let second = reference.observe(input(&left, &second_right, &valid));
        assert_eq!(second.diagnostics.current_metric, Some(44));
        assert_eq!(second.diagnostics.retained_metric, 20);
        assert_eq!(second.diagnostics.budget, Some(chromatic::budget(44.0)));
    }

    #[test]
    fn solve_corrects_each_ordinal_towards_the_other_without_swapping() {
        let left = image(80);
        let right = image(120);
        let valid = vec![0; VALIDITY_BYTES];
        let output = Reference::new().observe(input(&left, &right, &valid));
        let left_seam = pixel(&output.prepared[0], 49, 100)[0];
        let right_seam = pixel(&output.prepared[1], 49, 100)[0];
        assert!(
            left_seam > 80,
            "lens zero should receive its own positive field"
        );
        assert!(
            right_seam < 120,
            "lens one should receive its own negative field"
        );
        assert_eq!(pixel(&output.prepared[0], 20, 100), &[80, 80, 80]);
        assert_eq!(pixel(&output.prepared[1], 80, 100), &[120, 120, 120]);
    }

    #[test]
    fn validity_rows_outside_the_two_evidence_rows_do_not_exclude_support() {
        let left = image(80);
        let right = image(120);
        let clean = vec![0; VALIDITY_BYTES];
        let mut outside = clean.clone();
        for row in [48, 51] {
            for column in 0..chroma::COLUMNS {
                outside[validity_slot(row, column)] = 1;
            }
        }
        assert_eq!(
            support_masks([&left, &right], &clean),
            support_masks([&left, &right], &outside)
        );
    }

    #[test]
    fn evidence_metric_reads_only_rows_49_50_and_columns_18_through_193() {
        let left = image(100);
        let mut right = left.clone();
        for row in [48, 51] {
            for column in 0..chroma::COLUMNS {
                right[pixel_offset(row, column)] = 200;
            }
        }
        for row in SUPPORT_ROWS {
            for column in [17, 194] {
                right[pixel_offset(row, column)] = 200;
            }
        }
        let support = vec![true; 2 * chroma::COLUMNS];
        let differences = supported_byte_differences([&left, &right], &support);
        assert_eq!(differences.len(), 2 * chroma::EVIDENCE_COLUMNS.len());
        assert_eq!(metric(&differences), Some(0));
    }

    #[test]
    fn non_gray_channels_correct_the_same_ordinal_towards_its_peer() {
        let mut left = Vec::with_capacity(IMAGE_BYTES);
        let mut right = Vec::with_capacity(IMAGE_BYTES);
        for _ in 0..IMAGE_PIXELS {
            left.extend_from_slice(&[80, 100, 120]);
            right.extend_from_slice(&[110, 100, 90]);
        }
        assert!(support_at([&left, &right], 49, 100).strict);
        let valid = vec![0; VALIDITY_BYTES];
        let output = Reference::new().observe(input(&left, &right, &valid));
        assert_eq!(output.diagnostics.current_metric, Some(30));
        assert!(output.diagnostics.admitted > 0);
        let left_seam = pixel(&output.prepared[0], 49, 100);
        let right_seam = pixel(&output.prepared[1], 49, 100);
        assert!(left_seam[0] > 80 && left_seam[2] < 120);
        assert!(right_seam[0] < 110 && right_seam[2] > 90);
    }

    #[test]
    fn empty_after_success_reuses_control_and_reset_clears_only_solve_seeds() {
        let left = image(80);
        let right = image(120);
        let empty = image(0);
        let valid = vec![0; VALIDITY_BYTES];
        let mut reference = Reference::new();
        let first = reference.observe(input(&left, &right, &valid));
        let retained_budget =
            chromatic::budget(f32::from(first.diagnostics.current_metric.unwrap()));

        let stale = reference.observe(input(&empty, &empty, &valid));
        assert!(stale.diagnostics.used_stale_control);
        assert_eq!(stale.diagnostics.budget, Some(retained_budget));
        assert_eq!(stale.diagnostics.current_metric, None);

        reference.reset();
        let after_reset_empty = reference.observe(input(&empty, &empty, &valid));
        assert!(after_reset_empty.diagnostics.used_stale_control);
        assert_eq!(after_reset_empty.diagnostics.retained_metric, 40);
        assert_eq!(
            after_reset_empty.diagnostics.budget,
            Some(chromatic::COLD_BUDGET)
        );

        let landed = reference.observe(input(&left, &right, &valid));
        assert_eq!(landed.diagnostics.budget, Some(retained_budget));
        assert_eq!(landed.diagnostics.retained_metric, 40);
    }

    #[test]
    fn byte_conversion_rejects_the_positive_cvtss2si_overflow_boundary() {
        let overflow = i32::MAX as f32;
        let last_finite_integer = f32::from_bits(overflow.to_bits() - 1);
        assert_eq!(byte(last_finite_integer), 255);
        assert_eq!(byte(overflow), 0);
        assert_eq!(byte(f32::INFINITY), 0);
    }
}
