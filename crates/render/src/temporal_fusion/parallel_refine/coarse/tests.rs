use super::*;
use crate::temporal_fusion::{parallel_refine, pyramid};

fn patterned(width: usize, height: usize, salt: u32) -> Level {
    Level {
        width,
        height,
        pixels: (0..width * height)
            .map(|at| {
                let x = (at % width) as u32;
                let y = (at / width) as u32;
                x.wrapping_mul(1_664_525)
                    .wrapping_add(y.wrapping_mul(1_013_904_223))
                    .wrapping_add(salt)
                    .rotate_left((x ^ y) & 15) as u8
            })
            .collect(),
    }
}

fn zero_input(level: &Level) -> FinestInput {
    FinestInput {
        seeds: vec![[0, 0, 0]; (level.width / BLOCK) * (level.height / BLOCK)],
        global: [0, 0],
    }
}

fn copy_block(current: &Level, reference: &mut Level, source: [usize; 2], destination: [usize; 2]) {
    for row in 0..BLOCK {
        let current_at = (source[1] + row) * current.width + source[0];
        let reference_at = (destination[1] + row) * reference.width + destination[0];
        reference.pixels[reference_at..reference_at + BLOCK]
            .copy_from_slice(&current.pixels[current_at..current_at + BLOCK]);
    }
}

#[test]
fn small_odd_plane_uses_only_complete_blocks() {
    let current = patterned(47, 35, 7);
    let reference = patterned(47, 35, 19);
    let output = refine_level(&current, &reference, &zero_input(&current), false).unwrap();
    assert_eq!(output.len(), 2 * 2);
}

#[test]
fn ring_two_finds_an_exact_match_around_the_fixed_start_center() {
    let current = patterned(64, 48, 7);
    let mut reference = patterned(64, 48, 0x9e37_79b9);
    copy_block(&current, &mut reference, [16, 16], [18, 16]);
    let output = refine_level(&current, &reference, &zero_input(&current), false).unwrap();
    assert_eq!(output[1 + 4], [2, 0, 0]);
}

#[test]
fn equal_costs_keep_the_first_zero_start() {
    let level = Level {
        width: 33,
        height: 17,
        pixels: vec![73; 33 * 17],
    };
    let output = refine_level(&level, &level, &zero_input(&level), false).unwrap();
    assert_eq!(output, vec![[0, 0, 0]; 2]);
}

#[test]
fn smallest_plane_uses_median_seed_and_omits_future_diagonal() {
    let current = patterned(64, 48, 7);
    let mut reference = patterned(64, 48, 0x9e37_79b9);
    copy_block(&current, &mut reference, [0, 0], [4, 0]);

    let mut own = zero_input(&current);
    own.seeds[0] = [4, 0, 0];
    let ordinary = refine_level(&current, &reference, &own, false).unwrap();
    let smallest = refine_level(&current, &reference, &own, true).unwrap();
    assert_eq!(ordinary[0], [4, 0, 0]);
    assert_ne!(smallest[0], ordinary[0]);

    let mut diagonal = zero_input(&current);
    diagonal.seeds[5] = [4, 0, 0];
    let ordinary = refine_level(&current, &reference, &diagonal, false).unwrap();
    let smallest = refine_level(&current, &reference, &diagonal, true).unwrap();
    assert_eq!(ordinary[0], [4, 0, 0]);
    assert_ne!(smallest[0], ordinary[0]);
}

#[test]
fn changing_a_searched_neighbor_cannot_mutate_the_next_block_predictors() {
    let current = patterned(64, 32, 7);
    let reference_a = patterned(64, 32, 0x9e37_79b9);
    let mut reference_b = Level {
        width: reference_a.width,
        height: reference_a.height,
        pixels: reference_a.pixels.clone(),
    };
    // Block one finds -2 only in B. The modified reference footprint ends at
    // x=29, before block two's legal candidate footprints begin at x=30.
    copy_block(&current, &mut reference_b, [16, 0], [14, 0]);
    let input = zero_input(&current);
    let a = refine_level(&current, &reference_a, &input, false).unwrap();
    let b = refine_level(&current, &reference_b, &input, false).unwrap();
    assert_ne!(a[1], b[1]);
    assert_eq!(b[1], [-2, 0, 0]);
    assert_eq!(a[2], b[2]);
}

#[test]
fn seven_level_preparation_matches_explicit_coarse_to_fine_order() {
    let width = 1031;
    let height = 1027;
    let current_base = patterned(width, height, 7).pixels;
    let reference_base = patterned(width, height, 0x9e37_79b9).pixels;
    let current = pyramid::build(&current_base, width, height, LEVELS).unwrap();
    let reference = pyramid::build(&reference_base, width, height, LEVELS).unwrap();

    let actual = prepare_finest(&current, &reference).unwrap();
    let mut previous: Option<Vec<[i32; 3]>> = None;
    let mut previous_blocks = [0, 0];
    for level in (1..LEVELS).rev() {
        let blocks = [current[level].width / BLOCK, current[level].height / BLOCK];
        let input = previous.as_deref().map_or_else(
            || FinestInput {
                seeds: vec![[0, 0, 0]; blocks[0] * blocks[1]],
                global: [0, 0],
            },
            |records| prepare_next(records, previous_blocks, blocks).unwrap(),
        );
        previous = Some(
            refine_level(
                &current[level],
                &reference[level],
                &input,
                level == LEVELS - 1,
            )
            .unwrap(),
        );
        previous_blocks = blocks;
    }
    let expected = prepare_next(
        previous.as_deref().unwrap(),
        previous_blocks,
        [width / BLOCK, height / BLOCK],
    )
    .unwrap();
    assert_eq!(actual.global, expected.global);
    assert_eq!(actual.seeds, expected.seeds);
}

#[test]
fn existing_finest_still_uses_only_its_radius_one_ring() {
    let current = patterned(1024, 1024, 7);
    let mut reference = patterned(1024, 1024, 0x9e37_79b9);
    copy_block(&current, &mut reference, [32, 32], [34, 32]);
    let input = zero_input(&current);
    let finest = parallel_refine::finest(&current, &reference, &input).unwrap();
    let coarse = refine_level(&current, &reference, &input, false).unwrap();
    let at = 2 * 64 + 2;
    assert_eq!(coarse[at], [2, 0, 0]);
    assert_ne!(finest[at], coarse[at]);
}
