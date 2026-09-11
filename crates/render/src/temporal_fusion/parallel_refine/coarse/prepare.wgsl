// SPDX-License-Identifier: GPL-3.0-or-later
// Author: Manao
// Copyright(c)2006 A.G.Balakhnin aka Fizick - global motion, overlap, mode, refineMVs
//
// Global-mode selection and 9/3/3/1 predictor interpolation derive from
// MVTools commit 17250aa979616ac48dfb0e18abfdcf2bd4e3afc0
// (GPL-2.0-or-later); this adaptation elects GPL-3.0-or-later.

struct RawMotion {
    dx: i32,
    dy: i32,
    cost: i32,
}

struct Params {
    coarse_x: u32,
    coarse_y: u32,
    next_x: u32,
    next_y: u32,
    coarse_count: u32,
    next_count: u32,
    references: u32,
    periodic_grid_x: u32,
}

@group(0) @binding(0) var<storage, read> coarse: array<RawMotion>;
@group(0) @binding(1) var<storage, read_write> histograms: array<atomic<u32>>;
@group(0) @binding(2) var<storage, read_write> globals: array<vec2<i32>>;
@group(0) @binding(3) var<storage, read_write> seeds: array<RawMotion>;
@group(0) @binding(4) var<uniform> params: Params;

const FREQUENCY_SIZE: u32 = 16384u;
const MIDPOINT: i32 = 8192;
const LANES: u32 = 256u;

var<workgroup> x_counts: array<u32, 256>;
var<workgroup> x_indices: array<u32, 256>;
var<workgroup> y_counts: array<u32, 256>;
var<workgroup> y_indices: array<u32, 256>;
var<workgroup> sums_x: array<i32, 256>;
var<workgroup> sums_y: array<i32, 256>;
var<workgroup> selected_counts: array<u32, 256>;

fn histogram_at(reference: u32, component: u32, index: u32) -> u32 {
    return (reference * 2u + component) * FREQUENCY_SIZE + index;
}

@compute @workgroup_size(256)
fn build_histograms(
    @builtin(global_invocation_id) gid: vec3<u32>,
) {
    let reference = gid.y;
    let at = gid.x;
    if reference >= params.references || at >= params.coarse_count {
        return;
    }
    let value = coarse[reference * params.coarse_count + at];
    let x = value.dx + MIDPOINT;
    let y = value.dy + MIDPOINT;
    if x >= 0 && x < i32(FREQUENCY_SIZE) {
        atomicAdd(&histograms[histogram_at(reference, 0u, u32(x))], 1u);
    }
    if y >= 0 && y < i32(FREQUENCY_SIZE) {
        atomicAdd(&histograms[histogram_at(reference, 1u, u32(y))], 1u);
    }
}

fn lower_mode(
    left_count: u32,
    left_index: u32,
    right_count: u32,
    right_index: u32,
) -> vec2<u32> {
    if right_count > left_count ||
       (right_count == left_count && right_index < left_index) {
        return vec2<u32>(right_count, right_index);
    }
    return vec2<u32>(left_count, left_index);
}

@compute @workgroup_size(256)
fn estimate_globals(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    let reference = group.x;
    if reference >= params.references {
        return;
    }

    var best_x = vec2<u32>(0u, 0u);
    var best_y = vec2<u32>(0u, 0u);
    for (var index = lane; index < FREQUENCY_SIZE; index += LANES) {
        best_x = lower_mode(
            best_x.x,
            best_x.y,
            atomicLoad(&histograms[histogram_at(reference, 0u, index)]),
            index,
        );
        best_y = lower_mode(
            best_y.x,
            best_y.y,
            atomicLoad(&histograms[histogram_at(reference, 1u, index)]),
            index,
        );
    }
    x_counts[lane] = best_x.x;
    x_indices[lane] = best_x.y;
    y_counts[lane] = best_y.x;
    y_indices[lane] = best_y.y;
    workgroupBarrier();

    for (var stride = LANES / 2u; stride > 0u; stride /= 2u) {
        if lane < stride {
            let x = lower_mode(
                x_counts[lane],
                x_indices[lane],
                x_counts[lane + stride],
                x_indices[lane + stride],
            );
            let y = lower_mode(
                y_counts[lane],
                y_indices[lane],
                y_counts[lane + stride],
                y_indices[lane + stride],
            );
            x_counts[lane] = x.x;
            x_indices[lane] = x.y;
            y_counts[lane] = y.x;
            y_indices[lane] = y.y;
        }
        workgroupBarrier();
    }

    let mode_x = i32(x_indices[0]) - MIDPOINT;
    let mode_y = i32(y_indices[0]) - MIDPOINT;
    var sum_x = 0;
    var sum_y = 0;
    var count = 0u;
    for (var at = lane; at < params.coarse_count; at += LANES) {
        let value = coarse[reference * params.coarse_count + at];
        if abs(value.dx - mode_x) < 6 && abs(value.dy - mode_y) < 6 {
            sum_x += value.dx;
            sum_y += value.dy;
            count += 1u;
        }
    }
    sums_x[lane] = sum_x;
    sums_y[lane] = sum_y;
    selected_counts[lane] = count;
    workgroupBarrier();

    for (var stride = LANES / 2u; stride > 0u; stride /= 2u) {
        if lane < stride {
            sums_x[lane] += sums_x[lane + stride];
            sums_y[lane] += sums_y[lane + stride];
            selected_counts[lane] += selected_counts[lane + stride];
        }
        workgroupBarrier();
    }
    if lane == 0u {
        let count = selected_counts[0];
        if count == 0u {
            globals[reference] = vec2<i32>(2 * mode_x, 2 * mode_y);
        } else {
            globals[reference] = vec2<i32>(
                (2 * sums_x[0]) / i32(count),
                (2 * sums_y[0]) / i32(count),
            );
        }
    }
}

fn coarse_at(reference: u32, x: i32, y: i32) -> RawMotion {
    var column = x;
    if params.periodic_grid_x != 0u {
        let width = i32(params.coarse_x);
        column = ((x % width) + width) % width;
    }
    return coarse[
        reference * params.coarse_count +
        u32(y) * params.coarse_x + u32(column)
    ];
}

fn mixed(a: RawMotion, b: RawMotion, c: RawMotion, d: RawMotion) -> RawMotion {
    return RawMotion(
        (9 * a.dx + 3 * b.dx + 3 * c.dx + d.dx) >> 3,
        (9 * a.dy + 3 * b.dy + 3 * c.dy + d.dy) >> 3,
        (9 * a.cost + 3 * b.cost + 3 * c.cost + d.cost + 8) >> 4,
    );
}

@compute @workgroup_size(256)
fn interpolate_seeds(
    @builtin(global_invocation_id) gid: vec3<u32>,
) {
    let reference = gid.y;
    let at = gid.x;
    if reference >= params.references || at >= params.next_count {
        return;
    }
    let x = at % params.next_x;
    let y = at / params.next_x;
    let i = min(x, 2u * params.coarse_x - 1u);
    let j = min(y, 2u * params.coarse_y - 1u);
    let offset_x = select(-1, 1, i % 2u == 1u);
    let offset_y = select(-1, 1, j % 2u == 1u);
    let base_x = i32(i / 2u);
    let base_y = i32(j / 2u);

    var a: RawMotion;
    var b: RawMotion;
    var c: RawMotion;
    var d: RawMotion;
    if params.periodic_grid_x == 0u && (i == 0u || i >= 2u * params.coarse_x - 1u) {
        if j == 0u || j >= 2u * params.coarse_y - 1u {
            let value = coarse_at(reference, base_x, base_y);
            a = value;
            b = value;
            c = value;
            d = value;
        } else {
            let top = coarse_at(reference, base_x, base_y);
            let bottom = coarse_at(reference, base_x, base_y + offset_y);
            a = top;
            b = top;
            c = bottom;
            d = bottom;
        }
    } else if j == 0u || j >= 2u * params.coarse_y - 1u {
        let left = coarse_at(reference, base_x, base_y);
        let right = coarse_at(reference, base_x + offset_x, base_y);
        a = left;
        b = left;
        c = right;
        d = right;
    } else {
        a = coarse_at(reference, base_x, base_y);
        b = coarse_at(reference, base_x + offset_x, base_y);
        c = coarse_at(reference, base_x, base_y + offset_y);
        d = coarse_at(reference, base_x + offset_x, base_y + offset_y);
    }
    seeds[reference * params.next_count + at] = mixed(a, b, c, d);
}
