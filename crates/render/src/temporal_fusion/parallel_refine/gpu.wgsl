// SPDX-License-Identifier: GPL-3.0-or-later
// Author: Manao
// Copyright(c)2006 A.G.Balakhnin aka Fizick - global motion, overlap, mode, refineMVs
//
// Kjerag-specific constant-work finest-plane search. The SAD arithmetic,
// penalties, bounds, candidate order, and radius-one refinement derive from
// MVTools commit 17250aa979616ac48dfb0e18abfdcf2bd4e3afc0
// (GPL-2.0-or-later); this adaptation elects GPL-3.0-or-later. Unlike Studio's
// selected search, spatial predictors below are read from immutable interpolated
// coarse seeds. There is deliberately no prefix-dependent bad search, UMH,
// vector smoothing, temporal policy, or claim of Studio predictor parity.

struct RawMotion {
    dx: i32,
    dy: i32,
    // This field stores unpenalized 16x16 SAD.
    cost: i32,
}

struct Params {
    width: u32,
    height: u32,
    blocks_x: u32,
    blocks_y: u32,
    _padding0: u32,
    _padding1: u32,
    _padding2: u32,
    _padding3: u32,
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
}

@group(0) @binding(0) var current_image: texture_2d<u32>;
@group(0) @binding(1) var reference_0: texture_2d<u32>;
@group(0) @binding(2) var reference_1: texture_2d<u32>;
@group(0) @binding(3) var reference_2: texture_2d<u32>;
@group(0) @binding(4) var reference_3: texture_2d<u32>;
@group(0) @binding(5) var reference_4: texture_2d<u32>;
@group(0) @binding(6) var reference_5: texture_2d<u32>;
@group(0) @binding(7) var<storage, read> seeds: array<RawMotion>;
@group(0) @binding(8) var<storage, read_write> output: array<RawMotion>;
@group(0) @binding(9) var<uniform> params: Params;
@group(0) @binding(10) var<storage, read> globals: array<vec2<i32>>;

const BLOCK: u32 = 16u;
const CANDIDATES: u32 = 8u;
const LANES_PER_CANDIDATE: u32 = 32u;
const PENALTY_NEW: u32 = 50u;

var<workgroup> current_pixels: array<u32, 256>;
var<workgroup> partials: array<u32, 256>;
var<workgroup> candidates: array<Candidate, 8>;
var<workgroup> best: RawMotion;
var<workgroup> minimum_cost: i32;

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

fn block_bounds(block: vec2<u32>) -> Bounds {
    let source = vec2<i32>(block * BLOCK);
    return Bounds(
        -source.x,
        i32(params.width) - source.x - i32(BLOCK),
        -source.y,
        i32(params.height) - source.y - i32(BLOCK),
    );
}

fn clipped(value: vec2<i32>, bounds: Bounds) -> vec2<i32> {
    return vec2<i32>(
        clamp(value.x, bounds.min_x, bounds.max_x),
        clamp(value.y, bounds.min_y, bounds.max_y),
    );
}

// Checked predictors and ring candidates exclude the upper bound. The first
// three seeds bypass this predicate after inclusive clipping, as in start_seed.
fn is_candidate(value: vec2<i32>, bounds: Bounds) -> bool {
    return value.x >= bounds.min_x && value.x < bounds.max_x &&
           value.y >= bounds.min_y && value.y < bounds.max_y;
}

fn median3(a: i32, b: i32, c: i32) -> i32 {
    return max(min(a, b), min(max(a, b), c));
}

fn clear_candidates() {
    for (var slot = 0u; slot < CANDIDATES; slot += 1u) {
        candidates[slot] = Candidate(0, 0, 0u, 0u, 0u);
    }
}

fn put_candidate(slot: u32, value: vec2<i32>, valid: bool, penalize: bool) {
    candidates[slot] = Candidate(
        value.x,
        value.y,
        0u,
        select(0u, 1u, valid),
        select(0u, 1u, penalize),
    );
}

// Eight 32-lane teams evaluate eight candidates without candidate-dependent
// barriers. Team leaders make exact integer sums, then lane zero applies the
// CPU reference's strict ordinal tie rule.
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
            let candidate = candidates[ordinal];
            if candidate.valid != 0u {
                let penalty = select(
                    0u,
                    (PENALTY_NEW * candidate.sad) >> 8u,
                    candidate.penalize != 0u,
                );
                let cost = i32(candidate.sad + penalty);
                if cost < minimum_cost {
                    best = RawMotion(candidate.dx, candidate.dy, i32(candidate.sad));
                    minimum_cost = cost;
                }
            }
        }
    }
    workgroupBarrier();
}

fn put_ring(center: vec2<i32>, bounds: Bounds) {
    clear_candidates();
    // expanding(1, 1): N, S, W, E, NW, SW, NE, SE.
    let offsets = array<vec2<i32>, 8>(
        vec2<i32>(0, -1),
        vec2<i32>(0, 1),
        vec2<i32>(-1, 0),
        vec2<i32>(1, 0),
        vec2<i32>(-1, -1),
        vec2<i32>(-1, 1),
        vec2<i32>(1, -1),
        vec2<i32>(1, 1),
    );
    for (var slot = 0u; slot < CANDIDATES; slot += 1u) {
        let value = center + offsets[slot];
        put_candidate(slot, value, is_candidate(value, bounds), true);
    }
}

@compute @workgroup_size(256)
fn refine_blocks(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    let block = group.xy;
    let reference = group.z;
    let block_count = params.blocks_x * params.blocks_y;
    let local_index = block.y * params.blocks_x + block.x;
    let index = reference * block_count + local_index;
    let source_u = block * BLOCK;
    let source = vec2<i32>(source_u);
    let pixel = vec2<u32>(lane % BLOCK, lane / BLOCK);
    current_pixels[lane] = textureLoad(
        current_image,
        vec2<i32>(source_u + pixel),
        0,
    ).x;
    workgroupBarrier();

    let bounds = block_bounds(block);
    if lane == 0u {
        let own_record = seeds[index];
        let own = clipped(vec2<i32>(own_record.dx, own_record.dy), bounds);
        let zero = clipped(vec2<i32>(0, 0), bounds);

        var left = zero;
        if block.x > 0u {
            let value = seeds[index - 1u];
            left = clipped(vec2<i32>(value.dx, value.dy), bounds);
        }
        var up = zero;
        if block.y > 0u {
            let value = seeds[index - params.blocks_x];
            up = clipped(vec2<i32>(value.dx, value.dy), bounds);
        }
        var diagonal = zero;
        if block.y + 1u < params.blocks_y && block.x + 1u < params.blocks_x {
            let value = seeds[index + params.blocks_x + 1u];
            diagonal = clipped(vec2<i32>(value.dx, value.dy), bounds);
        } else if block.y > 0u && block.x + 1u < params.blocks_x {
            let value = seeds[index - params.blocks_x + 1u];
            diagonal = clipped(vec2<i32>(value.dx, value.dy), bounds);
        }

        var median = left;
        if block.y > 0u {
            median = vec2<i32>(
                median3(left.x, up.x, diagonal.x),
                median3(left.y, up.y, diagonal.y),
            );
        }

        best = RawMotion(0, 0, 0);
        minimum_cost = 0x7fffffffi;
        clear_candidates();
        put_candidate(0u, vec2<i32>(0, 0), true, true);
        put_candidate(1u, clipped(globals[reference], bounds), true, false);
        put_candidate(2u, own, true, false);
        put_candidate(3u, median, is_candidate(median, bounds), false);
        put_candidate(4u, left, is_candidate(left, bounds), false);
        put_candidate(5u, up, is_candidate(up, bounds), false);
        put_candidate(6u, diagonal, is_candidate(diagonal, bounds), false);
    }
    execute_batch(lane, reference, source);

    if lane == 0u {
        put_ring(vec2<i32>(best.dx, best.dy), bounds);
    }
    execute_batch(lane, reference, source);

    if lane == 0u {
        output[index] = best;
    }
}
