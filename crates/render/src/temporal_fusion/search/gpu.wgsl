// SPDX-License-Identifier: GPL-3.0-or-later
// Author: Manao
// Copyright(c)2006 A.G.Balakhnin aka Fizick - global motion, overlap, mode, refineMVs
//
// Finest-plane GPU controller for the selected MVTools-derived CPU reference in
// `temporal_fusion/search.rs`, based on MVTools commit
// 17250aa979616ac48dfb0e18abfdcf2bd4e3afc0 (GPL-2.0-or-later). This adaptation
// elects GPL-3.0-or-later for compatibility with Kjerag. Each workgroup owns one
// reference and preserves that reference's row-major block, predictor, bad-count,
// candidate, and strict tie order. The host dispatches six workgroups for one
// explicit row.

struct RawMotion {
    dx: i32,
    dy: i32,
    // Despite the binding's historical field name, this remains unpenalized SAD.
    cost: i32,
}

struct Params {
    width: u32,
    height: u32,
    blocks_x: u32,
    blocks_y: u32,
    row: u32,
    _padding0: u32,
    _padding1: u32,
    _padding2: u32,
}

struct Bounds {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

struct Candidate {
    dx: i32,
    dy: i32,
    sad: u32,
    valid: u32,
    penalize: u32,
    update_best: u32,
    improved: u32,
}

@group(0) @binding(0) var current_image: texture_2d<u32>;
@group(0) @binding(1) var reference_0: texture_2d<u32>;
@group(0) @binding(2) var reference_1: texture_2d<u32>;
@group(0) @binding(3) var reference_2: texture_2d<u32>;
@group(0) @binding(4) var reference_3: texture_2d<u32>;
@group(0) @binding(5) var reference_4: texture_2d<u32>;
@group(0) @binding(6) var reference_5: texture_2d<u32>;
@group(0) @binding(7) var<storage, read_write> raw: array<RawMotion>;
@group(0) @binding(8) var<storage, read_write> bad_counts: array<u32>;
@group(0) @binding(9) var<uniform> params: Params;
@group(0) @binding(10) var<storage, read> globals: array<vec2<i32>>;

const BLOCK: u32 = 16u;
const PENALTY_NEW: u32 = 50u;
const BAD_SAD: u32 = 40000u;
const CANDIDATES: u32 = 8u;
const LANES_PER_CANDIDATE: u32 = 32u;

var<workgroup> current_pixels: array<u32, 256>;
var<workgroup> partials: array<u32, 256>;
var<workgroup> candidates: array<Candidate, 8>;
var<workgroup> best: RawMotion;
var<workgroup> minimum_cost: i32;
var<workgroup> bad_count: u32;
var<workgroup> run_bad_search: u32;
var<workgroup> hex_center: vec2<i32>;
var<workgroup> hex_direction: i32;
var<workgroup> hex_active: u32;

fn reference_pixel(reference: u32, coordinate: vec2<i32>) -> u32 {
    switch reference {
        case 0u: { return textureLoad(reference_0, coordinate, 0).x; }
        case 1u: { return textureLoad(reference_1, coordinate, 0).x; }
        case 2u: { return textureLoad(reference_2, coordinate, 0).x; }
        case 3u: { return textureLoad(reference_3, coordinate, 0).x; }
        case 4u: { return textureLoad(reference_4, coordinate, 0).x; }
        default: { return textureLoad(reference_5, coordinate, 0).x; }
    }
}

fn block_bounds(block_x: u32, block_y: u32) -> Bounds {
    let x = i32(block_x * BLOCK);
    let y = i32(block_y * BLOCK);
    return Bounds(-x, i32(params.width) - x - i32(BLOCK),
                  -y, i32(params.height) - y - i32(BLOCK));
}

fn clipped(value: vec2<i32>, bounds: Bounds) -> vec2<i32> {
    return vec2<i32>(clamp(value.x, bounds.min_x, bounds.max_x),
                     clamp(value.y, bounds.min_y, bounds.max_y));
}

// Checked candidates deliberately exclude the upper bound. The three starting
// seeds bypass this test, matching `start_seed` in the CPU reference.
fn is_candidate(value: vec2<i32>, bounds: Bounds) -> bool {
    return value.x >= bounds.min_x && value.x < bounds.max_x &&
           value.y >= bounds.min_y && value.y < bounds.max_y;
}

fn median3(a: i32, b: i32, c: i32) -> i32 {
    return max(min(a, b), min(max(a, b), c));
}

fn clear_candidates() {
    for (var slot = 0u; slot < CANDIDATES; slot += 1u) {
        candidates[slot] = Candidate(0, 0, 0u, 0u, 0u, 0u, 0u);
    }
}

fn put_candidate(slot: u32, value: vec2<i32>, valid: bool,
                 penalize: bool, update_best: bool) {
    candidates[slot] = Candidate(
        value.x,
        value.y,
        0u,
        select(0u, 1u, valid),
        select(0u, 1u, penalize),
        select(0u, 1u, update_best),
        0u,
    );
}

// Eight 32-lane teams evaluate up to eight candidates. There are no barriers
// in candidate-dependent control flow. Candidate leaders sum their 32 exact
// integer partials; lane zero then applies the original strict ordinal order.
fn execute_batch(lane: u32, reference: u32, source: vec2<i32>) {
    workgroupBarrier();
    let slot = lane / LANES_PER_CANDIDATE;
    let candidate_lane = lane % LANES_PER_CANDIDATE;
    let candidate = candidates[slot];
    var partial = 0u;
    if candidate.valid != 0u {
        for (var pixel = candidate_lane; pixel < 256u; pixel += LANES_PER_CANDIDATE) {
            let offset = vec2<i32>(i32(pixel % BLOCK), i32(pixel / BLOCK));
            let wanted = reference_pixel(
                reference,
                source + vec2<i32>(candidate.dx, candidate.dy) + offset,
            );
            partial += u32(abs(i32(current_pixels[pixel]) - i32(wanted)));
        }
    }
    partials[lane] = partial;
    workgroupBarrier();

    if candidate_lane == 0u {
        var sad = 0u;
        let first = slot * LANES_PER_CANDIDATE;
        for (var index = 0u; index < LANES_PER_CANDIDATE; index += 1u) {
            sad += partials[first + index];
        }
        candidates[slot].sad = sad;
    }
    workgroupBarrier();

    if lane == 0u {
        for (var ordinal = 0u; ordinal < CANDIDATES; ordinal += 1u) {
            candidates[ordinal].improved = 0u;
            if candidates[ordinal].valid != 0u {
                let sad = candidates[ordinal].sad;
                let penalty = select(0u, (PENALTY_NEW * sad) >> 8u,
                                     candidates[ordinal].penalize != 0u);
                let cost = i32(sad + penalty);
                if cost < minimum_cost {
                    if candidates[ordinal].update_best != 0u {
                        best.dx = candidates[ordinal].dx;
                        best.dy = candidates[ordinal].dy;
                    }
                    best.cost = i32(sad);
                    minimum_cost = cost;
                    candidates[ordinal].improved = 1u;
                }
            }
        }
    }
    workgroupBarrier();
}

fn put_ring(center: vec2<i32>, bounds: Bounds) {
    clear_candidates();
    // Same order as expanding(1, 1): N, S, W, E, NW, SW, NE, SE.
    let offsets = array<vec2<i32>, 8>(
        vec2<i32>(0, -1), vec2<i32>(0, 1),
        vec2<i32>(-1, 0), vec2<i32>(1, 0),
        vec2<i32>(-1, -1), vec2<i32>(-1, 1),
        vec2<i32>(1, -1), vec2<i32>(1, 1),
    );
    for (var slot = 0u; slot < CANDIDATES; slot += 1u) {
        let value = center + offsets[slot];
        put_candidate(slot, value, is_candidate(value, bounds), true, true);
    }
}

fn run_axes(lane: u32, reference: u32, source: vec2<i32>,
            bounds: Bounds, horizontal: bool) {
    for (var first = 0u; first < 24u; first += CANDIDATES) {
        if lane == 0u {
            clear_candidates();
            for (var slot = 0u; slot < CANDIDATES; slot += 1u) {
                let ordinal = first + slot;
                let distance = i32(1u + 2u * (ordinal / 2u));
                let signed_distance = select(-distance, distance, ordinal % 2u == 1u);
                let value = select(vec2<i32>(0, signed_distance),
                                   vec2<i32>(signed_distance, 0), horizontal);
                put_candidate(slot, value, is_candidate(value, bounds), true, true);
            }
        }
        execute_batch(lane, reference, source);
    }
}

fn hex4(index: u32) -> vec2<i32> {
    switch index {
        case 0u: { return vec2<i32>(-4, 2); }
        case 1u: { return vec2<i32>(-4, 1); }
        case 2u: { return vec2<i32>(-4, 0); }
        case 3u: { return vec2<i32>(-4, -1); }
        case 4u: { return vec2<i32>(-4, -2); }
        case 5u: { return vec2<i32>(4, -2); }
        case 6u: { return vec2<i32>(4, -1); }
        case 7u: { return vec2<i32>(4, 0); }
        case 8u: { return vec2<i32>(4, 1); }
        case 9u: { return vec2<i32>(4, 2); }
        case 10u: { return vec2<i32>(2, 3); }
        case 11u: { return vec2<i32>(0, 4); }
        case 12u: { return vec2<i32>(-2, 3); }
        case 13u: { return vec2<i32>(-2, -3); }
        case 14u: { return vec2<i32>(0, -4); }
        default: { return vec2<i32>(2, -3); }
    }
}

fn hex(index: u32) -> vec2<i32> {
    switch index {
        case 0u: { return vec2<i32>(-1, -2); }
        case 1u: { return vec2<i32>(-2, 0); }
        case 2u: { return vec2<i32>(-1, 2); }
        case 3u: { return vec2<i32>(1, 2); }
        case 4u: { return vec2<i32>(2, 0); }
        case 5u: { return vec2<i32>(1, -2); }
        case 6u: { return vec2<i32>(-1, -2); }
        default: { return vec2<i32>(-2, 0); }
    }
}

fn mod_minus_one(index: u32) -> i32 {
    switch index {
        case 0u: { return 5; }
        case 1u: { return 0; }
        case 2u: { return 1; }
        case 3u: { return 2; }
        case 4u: { return 3; }
        case 5u: { return 4; }
        case 6u: { return 5; }
        default: { return 0; }
    }
}

fn run_hex_two(lane: u32, reference: u32, source: vec2<i32>, bounds: Bounds) {
    if lane == 0u {
        clear_candidates();
        hex_center = vec2<i32>(best.dx, best.dy);
        hex_direction = -2;
        for (var slot = 0u; slot < 6u; slot += 1u) {
            let value = hex_center + hex(slot + 1u);
            put_candidate(slot, value, is_candidate(value, bounds), true, false);
        }
    }
    execute_batch(lane, reference, source);

    if lane == 0u {
        for (var slot = 0u; slot < 6u; slot += 1u) {
            if candidates[slot].improved != 0u {
                hex_direction = i32(slot);
            }
        }
        hex_active = select(0u, 1u, hex_direction != -2);
        if hex_active != 0u {
            hex_center += hex(u32(hex_direction + 1));
        }
    }

    // `1..range/2` for range 24 is eleven iterations. Inactive iterations
    // retain a fixed barrier shape and evaluate no candidate.
    for (var step = 0u; step < 11u; step += 1u) {
        if lane == 0u {
            clear_candidates();
            if hex_active != 0u && is_candidate(hex_center, bounds) {
                let old_direction = mod_minus_one(u32(hex_direction + 1));
                for (var slot = 0u; slot < 3u; slot += 1u) {
                    let value = hex_center + hex(u32(old_direction) + slot);
                    put_candidate(slot, value, is_candidate(value, bounds), true, false);
                }
            } else {
                hex_active = 0u;
            }
        }
        execute_batch(lane, reference, source);
        if lane == 0u && hex_active != 0u {
            var next_direction = -2;
            let old_direction = mod_minus_one(u32(hex_direction + 1));
            for (var slot = 0u; slot < 3u; slot += 1u) {
                if candidates[slot].improved != 0u {
                    next_direction = old_direction + i32(slot) - 1;
                }
            }
            hex_direction = next_direction;
            if next_direction == -2 {
                hex_active = 0u;
            } else {
                hex_center += hex(u32(next_direction + 1));
            }
        }
    }

    if lane == 0u {
        best.dx = hex_center.x;
        best.dy = hex_center.y;
        put_ring(hex_center, bounds);
    }
    execute_batch(lane, reference, source);
}

fn run_uneven_multi_hexagon(lane: u32, reference: u32,
                            source: vec2<i32>, bounds: Bounds) {
    run_axes(lane, reference, source, bounds, true);
    run_axes(lane, reference, source, bounds, false);

    // Six scales times sixteen offsets, in the source's nested-loop order.
    for (var first = 0u; first < 96u; first += CANDIDATES) {
        if lane == 0u {
            clear_candidates();
            for (var slot = 0u; slot < CANDIDATES; slot += 1u) {
                let ordinal = first + slot;
                let scale = i32(ordinal / 16u + 1u);
                let value = hex4(ordinal % 16u) * scale;
                put_candidate(slot, value, is_candidate(value, bounds), true, true);
            }
        }
        execute_batch(lane, reference, source);
    }
    run_hex_two(lane, reference, source, bounds);
}

@compute @workgroup_size(256)
fn search_row(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    let reference = group.x;
    let block_count = params.blocks_x * params.blocks_y;
    let reference_base = reference * block_count;
    let row = params.row;

    if lane == 0u {
        bad_count = bad_counts[reference];
    }
    workgroupBarrier();

    for (var block_x = 0u; block_x < params.blocks_x; block_x += 1u) {
        let block_index = row * params.blocks_x + block_x;
        let raw_index = reference_base + block_index;
        let source_u = vec2<u32>(block_x * BLOCK, row * BLOCK);
        let source = vec2<i32>(source_u);
        let pixel = vec2<u32>(lane % BLOCK, lane / BLOCK);
        current_pixels[lane] = textureLoad(current_image, vec2<i32>(source_u + pixel), 0).x;
        workgroupBarrier();

        let bounds = block_bounds(block_x, row);
        if lane == 0u {
            let coarse = clipped(vec2<i32>(raw[raw_index].dx, raw[raw_index].dy), bounds);
            let zero = clipped(vec2<i32>(0, 0), bounds);
            var left = zero;
            if block_x > 0u {
                let value = raw[raw_index - 1u];
                left = clipped(vec2<i32>(value.dx, value.dy), bounds);
            }
            var up = zero;
            if row > 0u {
                let value = raw[raw_index - params.blocks_x];
                up = clipped(vec2<i32>(value.dx, value.dy), bounds);
            }
            var diagonal = zero;
            if row + 1u < params.blocks_y && block_x + 1u < params.blocks_x {
                let value = raw[raw_index + params.blocks_x + 1u];
                diagonal = clipped(vec2<i32>(value.dx, value.dy), bounds);
            } else if row > 0u && block_x + 1u < params.blocks_x {
                let value = raw[raw_index - params.blocks_x + 1u];
                diagonal = clipped(vec2<i32>(value.dx, value.dy), bounds);
            }
            var predictor = left;
            if row > 0u {
                predictor = vec2<i32>(
                    median3(left.x, up.x, diagonal.x),
                    median3(left.y, up.y, diagonal.y),
                );
            }

            best = RawMotion(0, 0, 0);
            minimum_cost = 0x7fffffffi;
            clear_candidates();
            // Zero is valid without clipping in the CPU implementation. The
            // clipped value is identical for every valid searched block.
            put_candidate(0u, vec2<i32>(0, 0), true, true, true);
            put_candidate(1u, clipped(globals[reference], bounds), true, false, true);
            put_candidate(2u, coarse, true, false, true);
            put_candidate(3u, predictor, is_candidate(predictor, bounds), false, true);
            put_candidate(4u, left, is_candidate(left, bounds), false, true);
            put_candidate(5u, up, is_candidate(up, bounds), false, true);
            put_candidate(6u, diagonal, is_candidate(diagonal, bounds), false, true);
        }
        execute_batch(lane, reference, source);

        if lane == 0u {
            put_ring(vec2<i32>(best.dx, best.dy), bounds);
        }
        execute_batch(lane, reference, source);

        if lane == 0u {
            let threshold = BAD_SAD + 2500u * bad_count;
            run_bad_search = select(
                0u,
                1u,
                block_index > 1u && u32(best.cost) > threshold,
            );
            if run_bad_search != 0u {
                bad_count += 1u;
            }
        }
        if workgroupUniformLoad(&run_bad_search) != 0u {
            run_uneven_multi_hexagon(lane, reference, source, bounds);
        }

        if lane == 0u {
            raw[raw_index] = best;
        }
        workgroupBarrier();
    }

    if lane == 0u {
        bad_counts[reference] = bad_count;
    }
}
