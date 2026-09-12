use super::*;

fn level(width: usize, height: usize, value: u8) -> Level {
    Level {
        width,
        height,
        pixels: vec![value; width * height],
    }
}

fn input(width: usize, height: usize) -> FinestInput {
    FinestInput {
        seeds: vec![[0, 0, 0]; (width / BLOCK) * (height / BLOCK)],
        global: [0, 0],
    }
}

#[test]
fn equality_retains_the_first_zero_candidate_and_unpenalized_sad() {
    let current = level(1024, 1024, 73);
    let reference = current.clone();
    let mut input = input(1024, 1024);
    for (index, seed) in input.seeds.iter_mut().enumerate() {
        *seed = [index as i32 % 31 - 15, index as i32 % 19 - 9, 65_280];
    }
    input.global = [7, -5];
    let actual = finest(&current, &reference, &input).unwrap();
    assert!(actual.iter().all(|record| *record == [0, 0, 0]));
}

#[test]
fn every_block_reads_the_immutable_seed_grid() {
    let width = 1024;
    let height = 1024;
    let mut current = level(width, height, 0);
    for (index, pixel) in current.pixels.iter_mut().enumerate() {
        *pixel = (((index * 37) ^ ((index >> 5) * 173)) >> 2) as u8;
    }
    let mut reference = level(width, height, 255);
    let previous_x = 9 * BLOCK;
    let target_x = 10 * BLOCK;
    for y in 0..BLOCK {
        reference.pixels[y * width + previous_x..y * width + previous_x + BLOCK].copy_from_slice(
            &current.pixels[y * width + previous_x..y * width + previous_x + BLOCK],
        );
        reference.pixels[y * width + target_x + 2..y * width + target_x + 2 + BLOCK]
            .copy_from_slice(&current.pixels[y * width + target_x..y * width + target_x + BLOCK]);
    }
    let mut seeds = input(width, height);
    seeds.seeds[9] = [2, 0, 0];
    let actual = finest(&current, &reference, &seeds).unwrap();
    assert_eq!(actual[9], [0, 0, 0]);
    assert_eq!(actual[10], [2, 0, 0]);

    // Replacing only the immutable left seed changes block ten even though
    // block nine's refined output was zero in both runs.
    seeds.seeds[9] = [0, 0, 0];
    assert_ne!(finest(&current, &reference, &seeds).unwrap()[10], [2, 0, 0]);
}

#[test]
fn inclusive_starts_and_strict_checked_bound_are_distinct() {
    let bounds = Bounds {
        min_x: 0,
        max_x: 1008,
        min_y: 0,
        max_y: 1008,
    };
    assert_eq!(
        bounds
            .clip(Vector {
                x: 1009,
                y: 1009,
                sad: 0
            })
            .x,
        1008
    );
    assert!(!bounds.candidate(Vector {
        x: 1008,
        y: 0,
        sad: 0
    }));
    assert!(bounds.candidate(Vector {
        x: 1007,
        y: 1007,
        sad: 0
    }));

    let mut current = level(1024, 1024, 0);
    let mut reference = level(1024, 1024, 255);
    for y in 0..BLOCK {
        for x in 0..BLOCK {
            let value = (y * BLOCK + x) as u8;
            current.pixels[y * 1024 + x] = value;
            reference.pixels[(1008 + y) * 1024 + 1008 + x] = value;
        }
    }
    let mut input = input(1024, 1024);
    input.seeds[0] = [1008, 1008, 0];
    assert_eq!(
        finest(&current, &reference, &input).unwrap()[0],
        [1008, 1008, 0]
    );
}

#[test]
fn ring_order_keeps_the_first_equal_minimum() {
    let mut current = level(1024, 1024, 20);
    let mut reference = level(1024, 1024, 0);
    // At the interior block, north and south are equally exact. The fixed ring
    // order tests north first, so strict-lower replacement retains it.
    let block = [20usize, 20usize];
    let source = [block[0] * BLOCK, block[1] * BLOCK];
    for y in 0..BLOCK {
        current.pixels
            [(source[1] + y) * 1024 + source[0]..(source[1] + y) * 1024 + source[0] + BLOCK]
            .fill(if y % 2 == 0 { 20 } else { 40 });
    }
    for [dx, dy] in [[0isize, -1isize], [0, 1]] {
        for y in 0..BLOCK {
            for x in 0..BLOCK {
                let at_x = (source[0] as isize + dx + x as isize) as usize;
                let at_y = (source[1] as isize + dy + y as isize) as usize;
                reference.pixels[at_y * 1024 + at_x] =
                    current.pixels[(source[1] + y) * 1024 + source[0] + x];
            }
        }
    }
    let actual = finest(&current, &reference, &input(1024, 1024)).unwrap();
    assert_eq!(actual[block[1] * 64 + block[0]], [0, -1, 0]);
}

#[test]
fn invalid_geometry_pixels_seeds_and_globals_are_refused() {
    let valid = level(1024, 1024, 0);
    let valid_input = input(1024, 1024);
    assert_eq!(
        finest(&level(1023, 1024, 0), &level(1023, 1024, 0), &valid_input),
        Err(Error::Geometry)
    );
    assert_eq!(
        finest(&valid, &level(1025, 1024, 0), &valid_input),
        Err(Error::Geometry)
    );

    let mut short = valid.clone();
    short.pixels.pop();
    assert!(matches!(
        finest(&short, &valid, &valid_input),
        Err(Error::Length {
            which: "current",
            ..
        })
    ));
    let mut bad = input(1024, 1024);
    bad.seeds.pop();
    assert!(matches!(
        finest(&valid, &valid, &bad),
        Err(Error::SeedCount { .. })
    ));
    let mut bad = input(1024, 1024);
    bad.seeds[17] = [8192, 0, 0];
    assert_eq!(
        finest(&valid, &valid, &bad),
        Err(Error::SeedValue { index: 17 })
    );
    let mut bad = input(1024, 1024);
    bad.seeds[17] = [0, 0, 65_281];
    assert_eq!(
        finest(&valid, &valid, &bad),
        Err(Error::SeedValue { index: 17 })
    );
    let mut bad = input(1024, 1024);
    bad.global = [-8193, 0];
    assert_eq!(finest(&valid, &valid, &bad), Err(Error::GlobalValue));
}
