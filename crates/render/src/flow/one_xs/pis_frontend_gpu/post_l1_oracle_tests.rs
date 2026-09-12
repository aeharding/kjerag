//! Active CPU/GPU oracle for the selected three-call cold post-L1 loop.
//!
//! This module is test-only and is included by `post_l1.rs`.  It deliberately
//! drives the production bind-group layout, pipelines and WGSL entry points;
//! string-shape assertions alone cannot establish this boundary.

use std::future::Future;
use std::sync::mpsc;

use super::*;
use crate::flow::one_xs::dense::{self, DirectedImages, PublicDenseField};
use crate::flow::one_xs::pis::{AtoB, BtoA, Flow, PatchGrid, PisDirection};
use crate::flow::one_xs::post_update::{
    MotionLevel, RetainedPublicPyramids, preserve_without_variational_or_retained,
    update_without_variational_with_retained,
};
use crate::flow::one_xs::public_blend::blend_periodic_boundary;
use crate::flow::one_xs::temporal_median::{
    AtoBMedian, BtoAMedian, FilteredPatchGrid, MedianState,
};
use crate::flow::one_xs::warm::HintPyramid;
use crate::flow::one_xs::{Direction, LensPair};

const PLANAR_BASES: [[usize; 4]; 2] = [
    [0, 64_800, 129_600, 133_650],
    [137_700, 202_500, 267_300, 271_350],
];

#[test]
fn warm_post_l1_matches_cpu_and_rejects_semantic_mutations_on_forced_radv() {
    let (device, queue, adapter) = match radv() {
        Ok(gpu) => gpu,
        Err(why) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => {
            eprintln!("skipping resident warm post-L1 RADV oracle: {why}");
            return;
        }
        Err(why) => panic!("RADV Vulkan GPU required for resident warm post-L1: {why}"),
    };
    let context = OneXsGpuContext::new(&device, &queue);
    let pipeline = GpuWarmPostL1Pipeline::new(context.clone())
        .unwrap_or_else(|error| panic!("resident warm post-L1 refused {adapter}: {error}"));
    let fixture = Fixture::new(&device, &queue);

    let mut ab_median = AtoBMedian::new();
    let mut ba_median = BtoAMedian::new();
    for calculation in 0..2 {
        ab_median.run(raw_grid::<AtoB>(calculation)).unwrap();
        ba_median.run(raw_grid::<BtoA>(calculation)).unwrap();
    }
    let initial_history = pack_history(&ab_median.state(), &ba_median.state());
    let ab_raw = raw_grid::<AtoB>(2);
    let ba_raw = raw_grid::<BtoA>(2);
    let expected_hints = pack_hints(
        &HintPyramid::from_current_finest(&fixture.ab_images, &ab_raw),
        &HintPyramid::from_current_finest(&fixture.ba_images, &ba_raw),
    );
    let terminal_words = pack_terminal(&ab_raw, &ba_raw);
    let ab_filtered = ab_median.run(raw_grid::<AtoB>(2)).unwrap();
    let ba_filtered = ba_median.run(raw_grid::<BtoA>(2)).unwrap();
    let expected_filtered = pack_filtered(&ab_filtered, &ba_filtered);
    let expected_history = pack_history(&ab_median.state(), &ba_median.state());

    // Distinct directions and components, plus retained exceptional values,
    // make layout swaps and special-value shortcuts observable.
    let ab_prior = prior_public::<AtoB>(1.0);
    let ba_prior = prior_public::<BtoA>(-3.0);
    let prior_words = pack_public(&ab_prior, &ba_prior);
    let ab_retained = RetainedPublicPyramids::from_public_ref(&ab_prior);
    let ba_retained = RetainedPublicPyramids::from_public_ref(&ba_prior);
    let motion = (0..L1_ROWS * L1_COLS)
        .map(|pixel| u8::from(pixel % 17 == 0 || pixel % 101 == 9 || [31, 62].contains(&pixel)))
        .collect::<Vec<_>>();
    for lane in 0..4 {
        assert!(
            motion.iter().skip(lane).step_by(4).any(|value| *value == 0)
                && motion.iter().skip(lane).step_by(4).any(|value| *value != 0),
            "packed motion byte lane {lane} lacks zero/nonzero coverage"
        );
    }
    let ab_post = update_without_variational_with_retained(
        dense::densify_finest(&fixture.ab_images, ab_filtered).unwrap(),
        &ab_retained,
        MotionLevel::<AtoB>::from_bytes(Level::One, motion.clone()).unwrap(),
    )
    .unwrap();
    let ba_post = update_without_variational_with_retained(
        dense::densify_finest(&fixture.ba_images, ba_filtered).unwrap(),
        &ba_retained,
        MotionLevel::<BtoA>::from_bytes(Level::One, motion.clone()).unwrap(),
    )
    .unwrap();
    let expected_public = pack_public(
        &blend_periodic_boundary(dense::finish_linear_x2(ab_post).unwrap()),
        &blend_periodic_boundary(dense::finish_linear_x2(ba_post).unwrap()),
    );
    let expected_retained = retained_l2_words(&expected_public);
    let classifier_words = (0..2 * PATCH_ROWS)
        .map(|row| u32::from(row % 11 == 3))
        .collect::<Vec<_>>();

    assert_failed_candidate_preserves_prior(
        &pipeline,
        &device,
        &queue,
        &fixture.images_gpu,
        &initial_history,
        &prior_words,
        &motion,
    );

    let actual = run_warm_call(
        &pipeline,
        &device,
        &queue,
        &fixture.images_gpu,
        &terminal_words,
        &initial_history,
        &prior_words,
        &motion,
        &classifier_words,
    );
    exact_words(
        "warm raw successor hints",
        &actual.hints,
        &expected_hints,
        &adapter,
    );
    exact_words(
        "warm dcol-only median",
        &actual.filtered,
        &expected_filtered,
        &adapter,
    );
    exact_words(
        "warm histogram",
        &actual.histogram,
        &expected_history.0,
        &adapter,
    );
    exact_words("warm FIFO", &actual.fifo, &expected_history.1, &adapter);
    exact_words(
        "successful warm predecessor histogram",
        &actual.prior_histogram,
        &initial_history.0,
        &adapter,
    );
    exact_words(
        "successful warm predecessor FIFO",
        &actual.prior_fifo,
        &initial_history.1,
        &adapter,
    );
    exact_words("warm public", &actual.public, &expected_public, &adapter);
    exact_words(
        "warm retained L2 direction-pixel-vec2 ABI",
        &actual.retained_l2,
        &expected_retained,
        &adapter,
    );
    exact_words(
        "sealed classifier rows",
        &actual.classified_rows,
        &classifier_words,
        &adapter,
    );
    assert_eq!(actual.validity, u32::MAX);
    assert_periodic_pairs(&actual.public);
    for direction in 0..2 {
        for patch in 0..PATCHES {
            assert_eq!(
                actual.filtered[2 * (direction * PATCHES + patch) + 1],
                terminal_words[2 * (direction * PATCHES + patch) + 1],
                "temporal median changed drow direction={direction} patch={patch} on {adapter}",
            );
        }
    }

    for (name, from, to) in [
        (
            "retained L1 association",
            "resized=((tl+tr)*0.5+(bl+br)*0.5)*0.5;",
            "resized=(((tl+tr)+bl)+br)*0.25;",
        ),
        (
            "hint before median",
            "let v=vote(dir,row,col,true);",
            "let v=vote(dir,row,col,false);",
        ),
        (
            "retained nonfinite times zero",
            "let retained_term=bitcast<f32>(retained_l1[at])*(1.0-fresh_weight);",
            "let retained_term=select(bitcast<f32>(retained_l1[at]),0.0,fresh_weight==1.0);",
        ),
        (
            "packed motion byte lanes",
            "let motion=(motion_l1[pixel/4u]>>(8u*(pixel%4u)))&0xffu;",
            "let motion=motion_l1[pixel/4u]&0xffu;",
        ),
    ] {
        assert!(SHADER.contains(from), "mutation source disappeared: {name}");
        let changed = SHADER.replacen(from, to, 1);
        let changed_pipeline = GpuWarmPostL1Pipeline::from_shader(context.clone(), &changed)
            .unwrap_or_else(|error| {
                panic!("{name} mutation did not compile on {adapter}: {error}")
            });
        let changed = run_warm_call(
            &changed_pipeline,
            &device,
            &queue,
            &fixture.images_gpu,
            &terminal_words,
            &initial_history,
            &prior_words,
            &motion,
            &classifier_words,
        );
        let observed = if name == "hint before median" {
            changed.hints != expected_hints
        } else {
            changed.public != expected_public
        };
        assert!(
            observed,
            "live {name} mutation was not observed on {adapter}"
        );
    }
}

fn prior_public<D: PisDirection>(bias: f32) -> PublicDenseField<D> {
    let mut dcol = Vec::with_capacity(PUB_ROWS * PUB_COLS);
    let mut drow = Vec::with_capacity(PUB_ROWS * PUB_COLS);
    for pixel in 0..PUB_ROWS * PUB_COLS {
        let row = pixel / PUB_COLS;
        let col = pixel % PUB_COLS;
        dcol.push(bias + row as f32 * 0.00390625 + col as f32 * 0.0625);
        drow.push(-bias + row as f32 * 0.001953125 - col as f32 * 0.03125);
    }
    let association_operands =
        [0x3eff_4786, 0x3eff_6895, 0x3f00_3890, 0x3f00_48ff].map(f32::from_bits);
    for col in [0, 28] {
        let top = 2 * 10 * PUB_COLS;
        let left = 2 * col;
        dcol[top + left] = association_operands[0];
        dcol[top + left + 1] = association_operands[1];
        dcol[top + PUB_COLS + left] = association_operands[2];
        dcol[top + PUB_COLS + left + 1] = association_operands[3];
    }
    // Motion is one at these L1 destinations. Exact arithmetic still
    // evaluates retained * 0, so NaN and Inf contaminate the blend rather
    // than being erased by a motion fast path.
    dcol[2 * PUB_COLS + 2] = f32::NAN;
    drow[4 * PUB_COLS + 4] = f32::INFINITY;
    PublicDenseField::from_row_major_components(dcol, drow).unwrap()
}

struct WarmActual {
    filtered: Vec<u32>,
    histogram: Vec<u32>,
    fifo: Vec<u32>,
    hints: Vec<u32>,
    public: Vec<u32>,
    retained_l2: Vec<u32>,
    classified_rows: Vec<u32>,
    prior_histogram: Vec<u32>,
    prior_fifo: Vec<u32>,
    validity: u32,
}

struct QualifiedClassifiedRows {
    rows: wgpu::Buffer,
    present: bool,
}

impl classified_rows::Sealed for QualifiedClassifiedRows {}
impl GpuWarmClassifiedRows for QualifiedClassifiedRows {}

fn pack_motion(motion: &[u8]) -> Vec<u32> {
    motion
        .chunks(4)
        .map(|bytes| {
            bytes.iter().enumerate().fold(0u32, |word, (lane, byte)| {
                word | (u32::from(*byte) << (8 * lane))
            })
        })
        .collect()
}

fn assert_failed_candidate_preserves_prior(
    pipeline: &GpuWarmPostL1Pipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    images: &wgpu::Buffer,
    history: &(Vec<u32>, Vec<u32>),
    prior_public: &[u32],
    motion: &[u8],
) {
    let prior_histogram = upload_words(device, queue, "failed warm prior histogram", &history.0);
    let prior_fifo = upload_words(device, queue, "failed warm prior FIFO", &history.1);
    let histogram_copy = prior_histogram.clone();
    let fifo_copy = prior_fifo.clone();
    let refused = pipeline.encode_until_classification(GpuWarmPostL1Inputs {
        context: pipeline.context.clone(),
        terminal: upload_words(device, queue, "undersized warm terminal", &[0]),
        images: images.clone(),
        prior_histogram,
        prior_fifo,
        prior_public: GpuWarmPriorPublic::for_qualification(upload_words(
            device,
            queue,
            "failed warm prior public",
            prior_public,
        )),
        motion_l1: upload_words(
            device,
            queue,
            "failed warm packed motion",
            &pack_motion(motion),
        ),
        validity: upload_words(device, queue, "failed warm validity", &[u32::MAX]),
    });
    assert!(refused.is_err(), "undersized warm terminal was accepted");
    exact_words(
        "failed warm predecessor histogram",
        &super::super::read_buffer_words(&pipeline.context, &histogram_copy, HIST_WORDS).unwrap(),
        &history.0,
        "forced RADV",
    );
    exact_words(
        "failed warm predecessor FIFO",
        &super::super::read_buffer_words(&pipeline.context, &fifo_copy, FIFO_WORDS).unwrap(),
        &history.1,
        "forced RADV",
    );
}

#[allow(clippy::too_many_arguments)]
fn run_warm_call(
    pipeline: &GpuWarmPostL1Pipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    images: &wgpu::Buffer,
    terminal: &[u32],
    history: &(Vec<u32>, Vec<u32>),
    prior_public: &[u32],
    motion: &[u8],
    classified_rows: &[u32],
) -> WarmActual {
    let prior_histogram = upload_words(device, queue, "warm oracle histogram", &history.0);
    let prior_fifo = upload_words(device, queue, "warm oracle FIFO", &history.1);
    let prior_histogram_copy = prior_histogram.clone();
    let prior_fifo_copy = prior_fifo.clone();
    let pending = pipeline
        .encode_until_classification(GpuWarmPostL1Inputs {
            context: pipeline.context.clone(),
            terminal: upload_words(device, queue, "warm oracle terminal", terminal),
            images: images.clone(),
            prior_histogram,
            prior_fifo,
            prior_public: GpuWarmPriorPublic::for_qualification(upload_words(
                device,
                queue,
                "warm oracle prior public",
                prior_public,
            )),
            motion_l1: upload_words(device, queue, "warm oracle motion L1", &pack_motion(motion)),
            validity: upload_words(device, queue, "warm oracle validity", &[u32::MAX]),
        })
        .unwrap();
    let encoded = pending.continue_with(QualifiedClassifiedRows {
        rows: upload_words(
            device,
            queue,
            "warm oracle classified rows",
            classified_rows,
        ),
        present: true,
    });
    assert!(encoded.classified_rows.present);
    let words = read_sections(
        device,
        queue,
        encoded.encoder,
        &[
            (&encoded._filtered, PATCH_WORDS),
            (&encoded._histogram, HIST_WORDS),
            (&encoded._fifo, FIFO_WORDS),
            (&encoded._hints, HINT_WORDS),
            (&encoded.public, PUBLIC_WORDS),
            (
                encoded.retained_l2_direction_pixel_vec2.buffer(),
                RETAINED_L2_WORDS,
            ),
            (&encoded.classified_rows.rows, 2 * PATCH_ROWS),
            (&encoded.validity, 1),
            (&prior_histogram_copy, HIST_WORDS),
            (&prior_fifo_copy, FIFO_WORDS),
        ],
    );
    let mut at = 0;
    let mut take = |count| {
        let result = words[at..at + count].to_vec();
        at += count;
        result
    };
    WarmActual {
        filtered: take(PATCH_WORDS),
        histogram: take(HIST_WORDS),
        fifo: take(FIFO_WORDS),
        hints: take(HINT_WORDS),
        public: take(PUBLIC_WORDS),
        retained_l2: take(RETAINED_L2_WORDS),
        classified_rows: take(2 * PATCH_ROWS),
        validity: take(1)[0],
        prior_histogram: take(HIST_WORDS),
        prior_fifo: take(FIFO_WORDS),
    }
}

#[test]
fn active_cold012_state_matches_cpu_on_forced_radv() {
    let (device, queue, adapter) = match radv() {
        Ok(gpu) => gpu,
        Err(why) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => {
            eprintln!("skipping resident cold post-L1 RADV oracle: {why}");
            return;
        }
        Err(why) => panic!("RADV Vulkan GPU required for resident cold post-L1: {why}"),
    };
    let context = OneXsGpuContext::new(&device, &queue);
    let pipeline = GpuColdPostL1Pipeline::new(context)
        .unwrap_or_else(|error| panic!("resident cold post-L1 refused {adapter}: {error}"));
    let fixture = Fixture::new(&device, &queue);
    let histogram = upload_words(
        &device,
        &queue,
        "cold oracle histogram",
        &vec![0; HIST_WORDS],
    );
    let fifo = upload_words(&device, &queue, "cold oracle FIFO", &vec![0; FIFO_WORDS]);
    let mut ab_median = AtoBMedian::new();
    let mut ba_median = BtoAMedian::new();
    let mut cold_public = Vec::new();

    for calculation in 0..3 {
        let ab_raw = raw_grid::<AtoB>(calculation);
        let ba_raw = raw_grid::<BtoA>(calculation);

        // Native makes the successor hints from the raw L1 terminal before
        // either direction enters its temporal median.
        let expected_hints = pack_hints(
            &HintPyramid::from_current_finest(&fixture.ab_images, &ab_raw),
            &HintPyramid::from_current_finest(&fixture.ba_images, &ba_raw),
        );
        assert_hint_non_vacuity(&expected_hints, calculation);

        let terminal = pack_terminal(&ab_raw, &ba_raw);
        let ab_filtered = ab_median.run(ab_raw).unwrap();
        let ba_filtered = ba_median.run(ba_raw).unwrap();
        let expected_filtered = pack_filtered(&ab_filtered, &ba_filtered);
        let expected_history = pack_history(&ab_median.state(), &ba_median.state());
        assert_history_depth(&expected_history.1, calculation + 1);

        let ab_post = preserve_without_variational_or_retained(
            dense::densify_finest(&fixture.ab_images, ab_filtered).unwrap(),
        );
        let ba_post = preserve_without_variational_or_retained(
            dense::densify_finest(&fixture.ba_images, ba_filtered).unwrap(),
        );
        let mut ab_public = dense::finish_linear_x2(ab_post).unwrap();
        let mut ba_public = dense::finish_linear_x2(ba_post).unwrap();
        if calculation == 2 {
            ab_public = blend_periodic_boundary(ab_public);
            ba_public = blend_periodic_boundary(ba_public);
        }
        let expected_public = pack_public(&ab_public, &ba_public);
        if calculation < 2 {
            assert_unrepaired_pair_exists(&expected_public);
        } else {
            assert_periodic_pairs(&expected_public);
        }
        let expected_retained = if calculation == 2 {
            retained_l2_words(&expected_public)
        } else {
            vec![0; RETAINED_L2_WORDS]
        };

        let actual = run_cold_call(
            &pipeline,
            &device,
            &queue,
            &fixture.images_gpu,
            &histogram,
            &fifo,
            &terminal,
            calculation == 2,
        );
        exact_words(
            "filtered terminal",
            &actual.filtered,
            &expected_filtered,
            &adapter,
        );
        exact_words(
            "histogram",
            &actual.histogram,
            &expected_history.0,
            &adapter,
        );
        exact_words("FIFO", &actual.fifo, &expected_history.1, &adapter);
        exact_words(
            "full planar hints",
            &actual.hints,
            &expected_hints,
            &adapter,
        );
        exact_words("public scratch", &actual.public, &expected_public, &adapter);
        exact_words(
            "retained L2",
            &actual.retained_l2,
            &expected_retained,
            &adapter,
        );
        assert_eq!(
            actual.validity,
            u32::MAX,
            "finite cold call {calculation} invalidated on {adapter}"
        );
        cold_public.push(actual.public);
    }

    assert_ne!(
        cold_public[0], cold_public[1],
        "distinct Cold0/Cold1 grids produced identical scratch"
    );
    assert_ne!(
        cold_public[1], cold_public[2],
        "Cold2 failed to replace Cold1 scratch"
    );

    // Live mutations execute on the same production entry points and must
    // break this witness, proving that the comparisons are not vacuous.
    for (name, from, to) in [
        (
            "A L1 row-plane base",
            "select(0u, 64800u, component == 1u)",
            "select(0u, 64801u, component == 1u)",
        ),
        ("L1-to-L2 scale", "let propagated=", "let propagated=2.0*"),
    ] {
        assert!(SHADER.contains(from), "mutation source disappeared: {name}");
        let changed = SHADER.replacen(from, to, 1);
        let changed_pipeline =
            GpuColdPostL1Pipeline::from_shader(OneXsGpuContext::new(&device, &queue), &changed)
                .unwrap_or_else(|error| {
                    panic!("{name} mutation did not compile on {adapter}: {error}")
                });
        let ab = raw_grid::<AtoB>(0);
        let ba = raw_grid::<BtoA>(0);
        let expected = pack_hints(
            &HintPyramid::from_current_finest(&fixture.ab_images, &ab),
            &HintPyramid::from_current_finest(&fixture.ba_images, &ba),
        );
        let changed_hist = upload_words(
            &device,
            &queue,
            "mutated cold histogram",
            &vec![0; HIST_WORDS],
        );
        let changed_fifo = upload_words(&device, &queue, "mutated cold FIFO", &vec![0; FIFO_WORDS]);
        let actual = run_cold_call(
            &changed_pipeline,
            &device,
            &queue,
            &fixture.images_gpu,
            &changed_hist,
            &changed_fifo,
            &pack_terminal(&ab, &ba),
            false,
        );
        assert!(
            actual
                .hints
                .iter()
                .zip(&expected)
                .any(|(actual, expected)| actual != expected),
            "live {name} mutation was not observed on {adapter}"
        );
    }
}

#[test]
fn retained_l2_preserves_selected_signed_zero_on_forced_radv() {
    let (device, queue, adapter) = match radv() {
        Ok(gpu) => gpu,
        Err(why) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => {
            eprintln!("skipping retained-L2 signed-zero RADV oracle: {why}");
            return;
        }
        Err(why) => panic!("RADV Vulkan GPU required for retained-L2 signed zero: {why}"),
    };
    let pipeline = GpuColdPostL1Pipeline::new(OneXsGpuContext::new(&device, &queue)).unwrap();
    let mut public = vec![0u32; PUBLIC_WORDS];
    for (dir, component, row, col) in [(0, 0, 7, 3), (1, 1, 11, 5)] {
        let l2_row = row;
        let l2_col = col;
        for source_row in [4 * l2_row + 1, 4 * l2_row + 2] {
            for source_col in [4 * l2_col + 1, 4 * l2_col + 2] {
                public[vec_word(dir, source_row, source_col, component, PUB_ROWS, PUB_COLS)] =
                    (-0.0f32).to_bits();
            }
        }
    }
    let expected = retained_l2_words(&public);
    let actual = run_retained_only(&pipeline, &device, &queue, &public);
    exact_words("signed-zero retained L2", &actual, &expected, &adapter);
    for (dir, component, row, col) in [(0, 0, 7, 3), (1, 1, 11, 5)] {
        let at = ((dir * L2_ROWS + row) * L2_COLS + col) * 2 + component;
        assert_eq!(
            actual[at],
            (-0.0f32).to_bits(),
            "retained signed zero was canonicalized on {adapter}"
        );
    }
}

#[test]
fn nonfinite_hint_vote_closes_resident_validity_on_forced_radv() {
    let (device, queue, adapter) = match radv() {
        Ok(gpu) => gpu,
        Err(why) if std::env::var_os("KJERAG_REQUIRE_GPU").is_none() => {
            eprintln!("skipping cold validity RADV oracle: {why}");
            return;
        }
        Err(why) => panic!("RADV Vulkan GPU required for cold validity: {why}"),
    };
    let pipeline = GpuColdPostL1Pipeline::new(OneXsGpuContext::new(&device, &queue)).unwrap();
    let fixture = Fixture::new(&device, &queue);
    let histogram = upload_words(
        &device,
        &queue,
        "invalid cold histogram",
        &vec![0; HIST_WORDS],
    );
    let fifo = upload_words(&device, &queue, "invalid cold FIFO", &vec![0; FIFO_WORDS]);
    let ab = raw_grid::<AtoB>(0);
    let ba = raw_grid::<BtoA>(0);
    let mut terminal = pack_terminal(&ab, &ba);
    terminal[0] = f32::INFINITY.to_bits();
    let actual = run_cold_call(
        &pipeline,
        &device,
        &queue,
        &fixture.images_gpu,
        &histogram,
        &fifo,
        &terminal,
        false,
    );
    assert_eq!(
        actual.validity,
        0x8000_0000 | (4 * L1_COLS + 4) as u32,
        "generated L1 A-to-B dcol hint failure code differs on {adapter}"
    );
}

struct Fixture {
    images_gpu: wgpu::Buffer,
    // The vectors are leaked only for this test's process lifetime so the
    // directed image borrows can live beside their GPU upload without a
    // self-referential test struct.
    ab_images: DirectedImages<'static, AtoB>,
    ba_images: DirectedImages<'static, BtoA>,
}

impl Fixture {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let image_a = (0..L1_ROWS * L1_COLS)
            .map(|i| ((3 * (i / L1_COLS) + 7 * (i % L1_COLS) + 19) % 251) as u8)
            .collect::<Vec<_>>();
        let image_b = (0..L1_ROWS * L1_COLS)
            .map(|i| ((11 * (i / L1_COLS) + 5 * (i % L1_COLS) + 37) % 253) as u8)
            .collect::<Vec<_>>();
        let mut words = image_a
            .iter()
            .chain(&image_b)
            .map(|value| u32::from(*value))
            .collect::<Vec<_>>();
        words.resize(2 * (L1_ROWS * L1_COLS + L2_ROWS * L2_COLS), 0);
        let images_gpu = upload_words(device, queue, "cold oracle images", &words);
        let image_a: &'static [u8] = Box::leak(image_a.into_boxed_slice());
        let image_b: &'static [u8] = Box::leak(image_b.into_boxed_slice());
        let pair = LensPair {
            a: image_a,
            b: image_b,
        };
        Self {
            images_gpu,
            ab_images: DirectedImages::<AtoB>::from_native_order(Level::One, pair.clone()).unwrap(),
            ba_images: DirectedImages::<BtoA>::from_native_order(Level::One, pair).unwrap(),
        }
    }
}

struct ColdActual {
    filtered: Vec<u32>,
    histogram: Vec<u32>,
    fifo: Vec<u32>,
    hints: Vec<u32>,
    public: Vec<u32>,
    retained_l2: Vec<u32>,
    validity: u32,
}

fn raw_grid<D: PisDirection>(calculation: usize) -> PatchGrid<D> {
    let sign = if D::DIRECTION == Direction::AtoB {
        1.0
    } else {
        -1.0
    };
    let flows = (0..PATCHES)
        .map(|patch| {
            Flow::new(
                sign * (0.8 + calculation as f32 * 0.1 - (patch % 9) as f32 * 0.025),
                (patch % 7) as f32 * 0.03125 - calculation as f32 * 0.09375,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    PatchGrid::from_row_major_components(
        Level::One,
        flows.iter().map(|flow| flow.dcol()).collect(),
        flows.iter().map(|flow| flow.drow()).collect(),
    )
    .unwrap()
}

fn pack_terminal(ab: &PatchGrid<AtoB>, ba: &PatchGrid<BtoA>) -> Vec<u32> {
    let mut words = Vec::with_capacity(PATCH_WORDS);
    for patches in [ab.patches(), ba.patches()] {
        for patch in patches {
            words.extend([patch.flow().dcol().to_bits(), patch.flow().drow().to_bits()]);
        }
    }
    words
}

fn pack_filtered(ab: &FilteredPatchGrid<AtoB>, ba: &FilteredPatchGrid<BtoA>) -> Vec<u32> {
    let mut words = Vec::with_capacity(PATCH_WORDS);
    for (dcol, drow) in [(ab.dcol(), ab.drow()), (ba.dcol(), ba.drow())] {
        for pixel in 0..PATCHES {
            words.extend([dcol[pixel].to_bits(), drow[pixel].to_bits()]);
        }
    }
    words
}

fn pack_history(ab: &MedianState<AtoB>, ba: &MedianState<BtoA>) -> (Vec<u32>, Vec<u32>) {
    let mut histogram = Vec::with_capacity(HIST_WORDS);
    let mut fifo = Vec::with_capacity(FIFO_WORDS);
    append_history(ab, &mut histogram, &mut fifo);
    append_history(ba, &mut histogram, &mut fifo);
    (histogram, fifo)
}

fn append_history<D: PisDirection>(
    state: &MedianState<D>,
    histogram: &mut Vec<u32>,
    fifo: &mut Vec<u32>,
) {
    histogram.extend(state.histogram().iter().map(|value| u32::from(*value)));
    for patch in 0..PATCHES {
        let start = state.offsets()[patch] as usize;
        let end = state.offsets()[patch + 1] as usize;
        let values = &state.values()[start..end];
        fifo.extend(values.iter().map(|value| value.to_bits()));
        fifo.resize(fifo.len() + 3 - values.len(), 0);
        fifo.extend([0, values.len() as u32]);
    }
}

fn pack_hints(ab: &HintPyramid<AtoB>, ba: &HintPyramid<BtoA>) -> Vec<u32> {
    let mut words = vec![0; HINT_WORDS];
    pack_hint_direction(ab, PLANAR_BASES[0], &mut words);
    pack_hint_direction(ba, PLANAR_BASES[1], &mut words);
    words
}

fn pack_hint_direction<D: PisDirection>(
    hints: &HintPyramid<D>,
    bases: [usize; 4],
    words: &mut [u32],
) {
    let l1 = hints.level(Level::One);
    let l2 = hints.level(Level::Two);
    for (values, base) in [
        (l1.dcol(), bases[0]),
        (l1.drow(), bases[1]),
        (l2.dcol(), bases[2]),
        (l2.drow(), bases[3]),
    ] {
        for (index, value) in values.iter().enumerate() {
            words[base + index] = value.to_bits();
        }
    }
}

fn pack_public(ab: &PublicDenseField<AtoB>, ba: &PublicDenseField<BtoA>) -> Vec<u32> {
    let mut words = Vec::with_capacity(PUBLIC_WORDS);
    for (dcol, drow) in [(ab.dcol(), ab.drow()), (ba.dcol(), ba.drow())] {
        for pixel in 0..PUB_ROWS * PUB_COLS {
            words.extend([dcol[pixel].to_bits(), drow[pixel].to_bits()]);
        }
    }
    words
}

fn retained_l2_words(public: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(RETAINED_L2_WORDS);
    for dir in 0..2 {
        for row in 0..L2_ROWS {
            for col in 0..L2_COLS {
                for component in 0..2 {
                    let sample = |r, c| {
                        f32::from_bits(public[vec_word(dir, r, c, component, PUB_ROWS, PUB_COLS)])
                    };
                    let top =
                        (sample(4 * row + 1, 4 * col + 1) + sample(4 * row + 1, 4 * col + 2)) * 0.5;
                    let bottom =
                        (sample(4 * row + 2, 4 * col + 1) + sample(4 * row + 2, 4 * col + 2)) * 0.5;
                    out.push((((top + bottom) * 0.5) * 0.25).to_bits());
                }
            }
        }
    }
    out
}

fn vec_word(
    dir: usize,
    row: usize,
    col: usize,
    component: usize,
    rows: usize,
    cols: usize,
) -> usize {
    ((dir * rows + row) * cols + col) * 2 + component
}

#[allow(clippy::too_many_arguments)]
fn run_cold_call(
    pipeline: &GpuColdPostL1Pipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    images: &wgpu::Buffer,
    histogram: &wgpu::Buffer,
    fifo: &wgpu::Buffer,
    terminal_words: &[u32],
    final_call: bool,
) -> ColdActual {
    let terminal = upload_words(device, queue, "cold oracle terminal", terminal_words);
    let hints = buffer(device, "cold oracle hints", HINT_WORDS);
    let filtered = buffer(device, "cold oracle filtered", PATCH_WORDS);
    let dense = buffer(device, "cold oracle dense", L1_WORDS);
    let horizontal = buffer(device, "cold oracle horizontal", HORIZONTAL_PRODUCT_WORDS);
    let public = buffer(device, "cold oracle public", PUBLIC_WORDS);
    let retained = buffer(device, "cold oracle retained L2", RETAINED_L2_WORDS);
    let validity = upload_words(device, queue, "cold oracle validity", &[u32::MAX]);
    let bind = cold_bind(
        pipeline,
        device,
        &terminal,
        images,
        histogram,
        fifo,
        &hints,
        &filtered,
        &dense,
        &horizontal,
        &public,
        &retained,
        &validity,
    );
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("cold post-L1 oracle"),
    });
    encoder.clear_buffer(&hints, 0, None);
    encoder.clear_buffer(&retained, 0, None);
    encode_cold_passes(&mut encoder, pipeline, &bind, final_call);
    let words = read_sections(
        device,
        queue,
        encoder,
        &[
            (&filtered, PATCH_WORDS),
            (histogram, HIST_WORDS),
            (fifo, FIFO_WORDS),
            (&hints, HINT_WORDS),
            (&public, PUBLIC_WORDS),
            (&retained, RETAINED_L2_WORDS),
            (&validity, 1),
        ],
    );
    let mut at = 0;
    let mut take = |count| {
        let answer = words[at..at + count].to_vec();
        at += count;
        answer
    };
    ColdActual {
        filtered: take(PATCH_WORDS),
        histogram: take(HIST_WORDS),
        fifo: take(FIFO_WORDS),
        hints: take(HINT_WORDS),
        public: take(PUBLIC_WORDS),
        retained_l2: take(RETAINED_L2_WORDS),
        validity: take(1)[0],
    }
}

#[allow(clippy::too_many_arguments)]
fn cold_bind<'a>(
    pipeline: &GpuColdPostL1Pipeline,
    device: &wgpu::Device,
    terminal: &'a wgpu::Buffer,
    images: &'a wgpu::Buffer,
    histogram: &'a wgpu::Buffer,
    fifo: &'a wgpu::Buffer,
    hints: &'a wgpu::Buffer,
    filtered: &'a wgpu::Buffer,
    dense: &'a wgpu::Buffer,
    horizontal: &'a wgpu::Buffer,
    public: &'a wgpu::Buffer,
    retained: &'a wgpu::Buffer,
    validity: &'a wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cold post-L1 oracle"),
        layout: &pipeline.layout,
        entries: &[
            entry(0, terminal),
            entry(1, images),
            entry(3, histogram),
            entry(4, fifo),
            entry(7, hints),
            entry(9, filtered),
            entry(11, dense),
            entry(12, horizontal),
            entry(13, public),
            entry(14, &pipeline.quantized_values),
            entry(16, retained),
            entry(17, validity),
        ],
    })
}

fn run_retained_only(
    pipeline: &GpuColdPostL1Pipeline,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    public_words: &[u32],
) -> Vec<u32> {
    let terminal = buffer(device, "retained oracle terminal", PATCH_WORDS);
    let images = buffer(
        device,
        "retained oracle images",
        2 * (L1_ROWS * L1_COLS + L2_ROWS * L2_COLS),
    );
    let histogram = buffer(device, "retained oracle histogram", HIST_WORDS);
    let fifo = buffer(device, "retained oracle FIFO", FIFO_WORDS);
    let hints = buffer(device, "retained oracle hints", HINT_WORDS);
    let filtered = buffer(device, "retained oracle filtered", PATCH_WORDS);
    let dense = buffer(device, "retained oracle dense", L1_WORDS);
    let horizontal = buffer(
        device,
        "retained oracle horizontal",
        HORIZONTAL_PRODUCT_WORDS,
    );
    let public = upload_words(device, queue, "retained oracle public", public_words);
    let retained = buffer(device, "retained oracle output", RETAINED_L2_WORDS);
    let validity = upload_words(device, queue, "retained oracle validity", &[u32::MAX]);
    let bind = cold_bind(
        pipeline,
        device,
        &terminal,
        &images,
        &histogram,
        &fifo,
        &hints,
        &filtered,
        &dense,
        &horizontal,
        &public,
        &retained,
        &validity,
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.clear_buffer(&retained, 0, None);
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline.passes[7]);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(((2 * L2_ROWS * L2_COLS) as u32).div_ceil(64), 1, 1);
    }
    read_sections(device, queue, encoder, &[(&retained, RETAINED_L2_WORDS)])
}

fn read_sections(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mut encoder: wgpu::CommandEncoder,
    sections: &[(&wgpu::Buffer, usize)],
) -> Vec<u32> {
    let total = sections.iter().map(|(_, words)| *words).sum::<usize>();
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cold post-L1 oracle readback"),
        size: words_bytes(total),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut offset = 0;
    for (source, words) in sections {
        encoder.copy_buffer_to_buffer(
            source,
            0,
            &readback,
            words_bytes(offset),
            words_bytes(*words),
        );
        offset += *words;
    }
    let submission = queue.submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (sent, received) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sent.send(result);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })
        .unwrap();
    received.recv().unwrap().unwrap();
    let mapped = slice.get_mapped_range();
    let words = mapped
        .chunks_exact(4)
        .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
        .collect::<Vec<_>>();
    drop(mapped);
    readback.unmap();
    words
}

fn assert_hint_non_vacuity(words: &[u32], calculation: usize) {
    assert_eq!(words.len(), HINT_WORDS);
    assert!(
        words.iter().any(|word| *word != 0),
        "Cold{calculation} hints are vacuously zero"
    );
    for bases in PLANAR_BASES {
        for base in bases[..2].iter().copied() {
            assert!(
                words[base + L1_ROWS * L1_COLS..base + PUB_ROWS * PUB_COLS]
                    .iter()
                    .all(|word| *word == 0)
            );
        }
    }
}

fn assert_history_depth(fifo: &[u32], expected: usize) {
    assert_eq!(fifo.len(), FIFO_WORDS);
    assert!(
        fifo.chunks_exact(5)
            .all(|pixel| pixel[4] == expected as u32)
    );
}

fn assert_periodic_pairs(public: &[u32]) {
    for dir in 0..2 {
        for (row, partner) in [(51, 1022), (52, 1023), (53, 1024), (54, 1025), (55, 1026)] {
            for lane in 0..PUB_COLS * 2 {
                assert_eq!(
                    public[(dir * PUB_ROWS + row) * PUB_COLS * 2 + lane],
                    public[(dir * PUB_ROWS + partner) * PUB_COLS * 2 + lane]
                );
            }
        }
    }
}

fn assert_unrepaired_pair_exists(public: &[u32]) {
    assert!((0..2).any(|dir| {
        (0..5).any(|pair| {
            let row = 51 + pair;
            let partner = 1022 + pair;
            (0..PUB_COLS * 2).any(|lane| {
                public[(dir * PUB_ROWS + row) * PUB_COLS * 2 + lane]
                    != public[(dir * PUB_ROWS + partner) * PUB_COLS * 2 + lane]
            })
        })
    }));
}

fn exact_words(label: &str, actual: &[u32], expected: &[u32], adapter: &str) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "CPU/GPU {label} length differs"
    );
    if let Some((index, (&gpu, &cpu))) = actual
        .iter()
        .zip(expected)
        .enumerate()
        .find(|(_, (gpu, cpu))| gpu != cpu)
    {
        panic!(
            "CPU/GPU {label} differs on {adapter}: word={index} gpu={gpu:#010x} cpu={cpu:#010x}"
        );
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(answer) => return answer,
            std::task::Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn radv() -> Result<(wgpu::Device, wgpu::Queue, String), String> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
        .into_iter()
        .find(|adapter| {
            adapter
                .get_info()
                .driver
                .to_ascii_lowercase()
                .contains("radv")
        })
        .ok_or("no RADV Vulkan adapter")?;
    let info = adapter.get_info();
    let name = format!(
        "{} vendor={:#06x} device={:#06x} driver={}",
        info.name, info.vendor, info.device, info.driver
    );
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("cold post-L1 RADV oracle"),
        required_features: wgpu::Features::empty(),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .map_err(|error| error.to_string())?;
    Ok((device, queue, name))
}
