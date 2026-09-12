struct Sampling {
    base_row: i32,
    base_col: i32,
    coefficients: vec4<f32>,
}

struct SourceModel {
    gradient_sum: vec2<f32>,
    inverse_col_col: f32,
    inverse_col_row: f32,
    inverse_row_row: f32,
    hessian: vec3<f32>,
    determinant: f32,
}

struct DescentStep {
    delta: vec2<f32>,
    residual: f32,
}

struct LaneScore {
    sum: f32,
    sum_sq: f32,
    survivors: u32,
}

@group(0) @binding(0) var<storage, read> words: array<u32>;
@group(0) @binding(1) var<storage, read> floats: array<f32>;
@group(0) @binding(2) var<storage, read_write> terminal_bits: array<u32>;
@group(0) @binding(3) var<storage, read> prepared_images: array<u32>;
@group(0) @binding(4) var<storage, read> prepared_masks: array<u32>;
@group(0) @binding(5) var<storage, read> prepared_gradient_bits: array<u32>;
@group(0) @binding(6) var<storage, read> prepared_weight_bits: array<u32>;
@group(0) @binding(7) var<storage, read> prepared_patch_sum_bits: array<u32>;
@group(0) @binding(8) var<storage, read> prepared_model_bits: array<u32>;
struct Wavefront { phase: u32, sweep: u32, diagonal: u32, stripe_rows: u32 }
@group(1) @binding(0) var<uniform> wave: Wavefront;
var<private> word_base: u32;
var<private> float_base: u32;
var<private> output_base: u32;
var<workgroup> lane_sums: array<f32, 16>;
var<workgroup> lane_square_sums: array<f32, 16>;
var<workgroup> lane_survivors: array<u32, 16>;
var<workgroup> candidate_seeds: array<vec2<f32>, 4>;
var<workgroup> candidate_present: array<u32, 4>;
var<workgroup> selected_seeds: array<vec2<f32>, 1>;
var<workgroup> selected_scores: array<f32, 1>;
var<workgroup> descent_values: array<vec4<f32>, 64>;
var<workgroup> descent_active: array<u32, 1>;

const PATCH_SIZE = 8u;
const PATCH_STRIDE = 3u;
const SENTINEL = 1.0e10;

fn materialize(value: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(value) ^ local_word(word(22u)));
}
fn mul_rn(a: f32, b: f32) -> f32 { return materialize(fma(a, b, -0.0)); }
fn add_rn(a: f32, b: f32) -> f32 { return materialize(fma(a, 1.0, b)); }
fn sub_rn(a: f32, b: f32) -> f32 { return materialize(fma(-1.0, b, a)); }
fn fma_rn(a: f32, b: f32, c: f32) -> f32 { return materialize(fma(a, b, c)); }

fn word(index: u32) -> u32 { return words[word_base + index]; }
fn local_word(index: u32) -> u32 { return words[word_base + index]; }
fn local_float(index: u32) -> f32 { return floats[float_base + index]; }
fn direct_prepared() -> bool { return word(30u) != 0u; }
fn rows() -> u32 { return word(0u); }
fn cols() -> u32 { return word(1u); }
fn patch_rows() -> u32 { return word(2u); }
fn patch_cols() -> u32 { return word(3u); }
fn patches() -> u32 { return word(5u); }

fn flow_at(offset: u32, cell: u32) -> vec2<f32> {
    return vec2<f32>(local_float(offset + 2u * cell), local_float(offset + 2u * cell + 1u));
}

fn stored_flow(cell: u32) -> vec2<f32> {
    return vec2<f32>(bitcast<f32>(terminal_bits[output_base + 2u * cell]), bitcast<f32>(terminal_bits[output_base + 2u * cell + 1u]));
}

fn store_flow(cell: u32, flow: vec2<f32>) {
    terminal_bits[output_base + 2u * cell] = bitcast<u32>(flow.x);
    terminal_bits[output_base + 2u * cell + 1u] = bitcast<u32>(flow.y);
}

fn clamped_index(value: i32, extent: u32) -> u32 {
    return u32(clamp(value, 0, i32(extent) - 1));
}

fn sampling_plan(source_row: u32, source_col: u32, flow: vec2<f32>) -> Sampling {
    let padding = 16.0;
    let translated_row = add_rn(add_rn(f32(source_row), flow.y), padding);
    let translated_col = add_rn(add_rn(f32(source_col), flow.x), padding);
    let origin_row = sub_rn(clamp(translated_row, padding + 1.0 - 8.0, padding + f32(rows()) - 1.0), padding);
    let origin_col = sub_rn(clamp(translated_col, padding + 1.0 - 8.0, padding + f32(cols()) - 1.0), padding);
    let floor_row = floor(origin_row);
    let floor_col = floor(origin_col);
    let row_fraction = sub_rn(origin_row, floor_row);
    let col_fraction = sub_rn(origin_col, floor_col);
    let inverse_row = sub_rn(1.0, row_fraction);
    let inverse_col = sub_rn(1.0, col_fraction);
    return Sampling(
        i32(floor_row),
        i32(floor_col),
        vec4<f32>(
            mul_rn(inverse_row, inverse_col),
            mul_rn(inverse_row, col_fraction),
            mul_rn(row_fraction, inverse_col),
            mul_rn(row_fraction, col_fraction),
        ),
    );
}

fn image_at(offset: u32, row: u32, col: u32) -> f32 {
    if direct_prepared() {
        return f32(prepared_images[offset + row * cols() + col]);
    }
    return f32(local_word(offset + row * cols() + col));
}

fn mask_at(offset: u32, index: u32) -> u32 {
    if direct_prepared() { return prepared_masks[offset + index]; }
    return local_word(offset + index);
}

fn gradient_at(index: u32) -> vec2<f32> {
    if direct_prepared() {
        let at = word(14u) + 2u * index;
        return vec2<f32>(bitcast<f32>(prepared_gradient_bits[at]), bitcast<f32>(prepared_gradient_bits[at + 1u]));
    }
    return vec2<f32>(local_float(word(14u) + index), local_float(word(15u) + index));
}

fn weight_at(index: u32) -> f32 {
    if direct_prepared() { return bitcast<f32>(prepared_weight_bits[word(16u) + index]); }
    return local_float(word(16u) + index);
}

fn patch_sum_at(cell: u32) -> f32 {
    if direct_prepared() { return bitcast<f32>(prepared_patch_sum_bits[word(17u) + cell]); }
    return local_float(word(17u) + cell);
}

fn target_index(plan: Sampling, patch_row: u32, patch_col: u32) -> u32 {
    let row = clamped_index(plan.base_row + i32(patch_row), rows());
    let col = clamped_index(plan.base_col + i32(patch_col), cols());
    return row * cols() + col;
}

fn candidate_residual(plan: Sampling, patch_row: u32, patch_col: u32, source: f32, survives: bool) -> f32 {
    let row0 = clamped_index(plan.base_row + i32(patch_row), rows());
    let col0 = clamped_index(plan.base_col + i32(patch_col), cols());
    let row1 = clamped_index(plan.base_row + i32(patch_row) + 1, rows());
    let col1 = clamped_index(plan.base_col + i32(patch_col) + 1, cols());
    let target_offset = word(10u);
    let top_left = mul_rn(image_at(target_offset, row0, col0), plan.coefficients.x);
    let top_right = mul_rn(image_at(target_offset, row0, col1), plan.coefficients.y);
    let top = add_rn(top_left, top_right);
    let bottom_left = mul_rn(image_at(target_offset, row1, col0), plan.coefficients.z);
    let bottom_right = mul_rn(image_at(target_offset, row1, col1), plan.coefficients.w);
    let bottom = add_rn(bottom_left, bottom_right);
    let bottom_minus_source = sub_rn(bottom, source);
    let difference = add_rn(bottom_minus_source, top);
    return mul_rn(difference, f32(u32(survives)));
}

fn candidate_lane_score(cell: u32, flow: vec2<f32>, lane: u32, present: bool) -> LaneScore {
    if !present { return LaneScore(0.0, 0.0, 0u); }
    let patch_row = cell / patch_cols();
    let patch_col = cell % patch_cols();
    let source_row = patch_row * PATCH_STRIDE;
    let source_col = patch_col * PATCH_STRIDE;
    let plan = sampling_plan(source_row, source_col, flow);
    let source_offset = word(9u);
    let source_mask_offset = word(11u);
    let target_mask_offset = word(12u);
    let weight_offset = word(16u);
    let patch_sum = patch_sum_at(cell);
    var reciprocal_sum = 0.0;
    if patch_sum > 0.0 { reciprocal_sum = div_rn(1.0, patch_sum); }
    let weighted = local_word(word(13u) + patch_row) != 0u;
    var sum = 0.0;
    var sum_sq = 0.0;
    var survivors = 0u;
    for (var row = 0u; row < PATCH_SIZE; row++) {
        let low_col = lane;
        let high_col = lane + 4u;
        let low_at = (source_row + row) * cols() + source_col + low_col;
        let high_at = (source_row + row) * cols() + source_col + high_col;
        let low_survives = mask_at(source_mask_offset, low_at) != 0u && mask_at(target_mask_offset, target_index(plan, row, low_col)) != 0u;
        let high_survives = mask_at(source_mask_offset, high_at) != 0u && mask_at(target_mask_offset, target_index(plan, row, high_col)) != 0u;
        var low = candidate_residual(plan, row, low_col, image_at(source_offset, source_row + row, source_col + low_col), low_survives);
        var high = candidate_residual(plan, row, high_col, image_at(source_offset, source_row + row, source_col + high_col), high_survives);
        if weighted {
            low = mul_rn(mul_rn(low, weight_at(low_at)), reciprocal_sum);
            high = mul_rn(mul_rn(high, weight_at(high_at)), reciprocal_sum);
        }
        sum = add_rn(sum, add_rn(low, high));
        sum_sq = add_rn(sum_sq, add_rn(mul_rn(low, low), mul_rn(high, high)));
        survivors += u32(low_survives) + u32(high_survives);
    }
    return LaneScore(sum, sum_sq, survivors);
}

fn source_model(cell: u32) -> SourceModel {
    let at = word(21u) + 5u * cell;
    if direct_prepared() {
        return SourceModel(
            vec2<f32>(bitcast<f32>(prepared_model_bits[at]), bitcast<f32>(prepared_model_bits[at + 1u])),
            bitcast<f32>(prepared_model_bits[at + 2u]),
            bitcast<f32>(prepared_model_bits[at + 3u]),
            bitcast<f32>(prepared_model_bits[at + 4u]),
            vec3<f32>(0.0),
            0.0,
        );
    }
    return SourceModel(
        vec2<f32>(local_float(at), local_float(at + 1u)),
        local_float(at + 2u),
        local_float(at + 3u),
        local_float(at + 4u),
        vec3<f32>(0.0),
        0.0,
    );
}

// Sampling has no cross-pixel dependency. The sixteen lanes prepare its
// values together; one lane retains the original row-major FMA accumulation.
fn prepare_descent_samples(cell: u32, flow: vec2<f32>, slot: u32, lane: u32, enabled: bool) {
    if !enabled { return; }
    let patch_row = cell / patch_cols();
    let patch_col = cell % patch_cols();
    let source_row = patch_row * PATCH_STRIDE;
    let source_col = patch_col * PATCH_STRIDE;
    let plan = sampling_plan(source_row, source_col, flow);
    let source_offset = word(9u);
    let target_offset = word(10u);
    let source_mask_offset = word(11u);
    let target_mask_offset = word(12u);
    let gx_offset = word(14u);
    let gy_offset = word(15u);
    let weight_offset = word(16u);
    let patch_sum = patch_sum_at(cell);
    var reciprocal_sum = 0.0;
    if patch_sum > 0.0 { reciprocal_sum = div_rn(1.0, patch_sum); }
    let weighted = local_word(word(13u) + patch_row) != 0u;
    for (var pixel = lane; pixel < 64u; pixel += 16u) {
        let row = pixel / 8u;
        let col = pixel % 8u;
        let source_at = (source_row + row) * cols() + source_col + col;
        if mask_at(source_mask_offset, source_at) == 0u || mask_at(target_mask_offset, target_index(plan, row, col)) == 0u {
            descent_values[slot * 64u + pixel] = vec4<f32>(0.0);
            continue;
        }
        let row0 = clamped_index(plan.base_row + i32(row), rows());
        let col0 = clamped_index(plan.base_col + i32(col), cols());
        let row1 = clamped_index(plan.base_row + i32(row) + 1, rows());
        let col1 = clamped_index(plan.base_col + i32(col) + 1, cols());
        let top_right = mul_rn(image_at(target_offset, row0, col1), plan.coefficients.y);
        let top = fma_rn(image_at(target_offset, row0, col0), plan.coefficients.x, top_right);
        let bottom_left = fma_rn(image_at(target_offset, row1, col0), plan.coefficients.z, top);
        let target_sample = fma_rn(image_at(target_offset, row1, col1), plan.coefficients.w, bottom_left);
        let difference = sub_rn(target_sample, image_at(source_offset, source_row + row, source_col + col));
        var residual = difference;
        if weighted {
            let normalized_weight = mul_rn(weight_at(source_at), reciprocal_sum);
            residual = mul_rn(difference, normalized_weight);
        }
        let gradient = gradient_at(source_at);
        descent_values[slot * 64u + pixel] = vec4<f32>(residual, gradient.x, gradient.y, 1.0);
    }
}

fn accumulate_descent(model: SourceModel, slot: u32) -> DescentStep {
    var sum = 0.0;
    var sum_sq = 0.0;
    var rhs_col = 0.0;
    var rhs_row = 0.0;
    var survivors = 0u;
    for (var pixel = 0u; pixel < 64u; pixel++) {
        let sample = descent_values[slot * 64u + pixel];
        if sample.w == 0.0 { continue; }
        let residual = sample.x;
        let gradient = sample.yz;
        rhs_col = fma_rn(residual, gradient.x, rhs_col);
        sum = add_rn(sum, residual);
        rhs_row = fma_rn(residual, gradient.y, rhs_row);
        sum_sq = fma_rn(residual, residual, sum_sq);
        survivors += 1u;
    }
    if survivors == 0u {
        return DescentStep(vec2<f32>(0.0), SENTINEL);
    }
    let count = f32(survivors);
    let correction_col = div_rn(mul_rn(sum, model.gradient_sum.x), count);
    let correction_row = div_rn(mul_rn(sum, model.gradient_sum.y), count);
    let corrected_col = sub_rn(rhs_col, correction_col);
    let corrected_row = sub_rn(rhs_row, correction_row);
    let cross_row = mul_rn(model.inverse_col_row, corrected_row);
    let delta_col = fma_rn(model.inverse_col_col, corrected_col, cross_row);
    let row_row = mul_rn(model.inverse_row_row, corrected_row);
    let delta_row = fma_rn(model.inverse_col_row, corrected_col, row_row);
    let square_sum = mul_rn(sum, sum);
    let residual = sub_rn(sum_sq, div_rn(square_sum, count));
    return DescentStep(vec2<f32>(delta_col, delta_row), residual);
}

fn add_word(accumulator: ptr<function, array<u32, 10>>, initial_word: u32, initial_value: u32) {
    var word = initial_word;
    var value = initial_value;
    loop {
        if value == 0u || word >= 10u { return; }
        let before = (*accumulator)[word];
        let after = before + value;
        (*accumulator)[word] = after;
        value = select(0u, 1u, after < before);
        word += 1u;
    }
}

fn add_48(accumulator: ptr<function, array<u32, 10>>, low: u32, high: u32, start: u32) {
    let word = start >> 5u;
    let shift = start & 31u;
    if shift == 0u {
        add_word(accumulator, word, low);
        add_word(accumulator, word + 1u, high);
    } else {
        add_word(accumulator, word, low << shift);
        add_word(accumulator, word + 1u, (low >> (32u - shift)) | (high << shift));
        add_word(accumulator, word + 2u, high >> (32u - shift));
    }
}

// Add the exact square of one finite f32. Limb bit zero represents 2^-298.
fn add_f32_square(accumulator: ptr<function, array<u32, 10>>, bits: u32) {
    let exponent = bits >> 23u;
    let fraction = bits & 0x007fffffu;
    var mantissa = fraction;
    var start = 0u;
    if exponent != 0u {
        mantissa |= 0x00800000u;
        start = 2u * exponent - 2u;
    }
    let low_half = mantissa & 0xffffu;
    let high_half = mantissa >> 16u;
    let low_square = low_half * low_half;
    let cross = low_half * high_half;
    let low = low_square + (cross << 17u);
    let high = high_half * high_half + (cross >> 15u) + select(0u, 1u, low < low_square);
    add_48(accumulator, low, high, start);
}

// CPU computes each component subtraction in f32, promotes those exact
// results, then applies correctly-rounded f64 hypot. The RN-even boundary for
// a returned value greater than 8 is (8 + 2^-50)^2, whose exact square has
// bits 304, 252 and 198 in this 2^-298 fixed-point representation.
fn terminal_distance_exceeds_eight(current: vec2<f32>, seed: vec2<f32>) -> bool {
    let difference = current - seed;
    let x_bits = bitcast<u32>(abs(difference.x));
    let y_bits = bitcast<u32>(abs(difference.y));
    if x_bits == 0x7f800000u || y_bits == 0x7f800000u { return true; }
    if x_bits > 0x7f800000u || y_bits > 0x7f800000u { return false; }
    if x_bits > 0x41000000u || y_bits > 0x41000000u { return true; }
    var square = array<u32, 10>();
    add_f32_square(&square, x_bits);
    add_f32_square(&square, y_bits);
    var threshold = array<u32, 10>();
    threshold[9] = 1u << 16u;
    threshold[7] = 1u << 28u;
    threshold[6] = 1u << 6u;
    for (var ordinal = 0u; ordinal < 10u; ordinal++) {
        let limb = 9u - ordinal;
        if square[limb] > threshold[limb] { return true; }
        if square[limb] < threshold[limb] { return false; }
    }
    return false;
}

fn disparity_rejects(flow: vec2<f32>) -> bool {
    if word(7u) == 0u { return false; }
    let at = word(20u);
    let first = vec2<f32>(local_float(at), local_float(at + 1u));
    let second = vec2<f32>(local_float(at + 2u), local_float(at + 3u));
    let col_ok = (flow.x > first.x && flow.x < second.x) || (flow.x > second.x && flow.x < first.x);
    let row_ok = (flow.y > first.y && flow.y < second.y) || (flow.y > second.y && flow.y < first.y);
    return !(col_ok && row_ok);
}

fn write_qualification_probes(local_index: u32) {
    if word(29u) == 0u || local_index != 0u { return; }
    let base = output_base + 2u * patches();
    terminal_bits[base] = local_word(word(22u));
    let division_at = word(23u);
    let division_count = word(24u);
    for (var probe = 0u; probe < division_count; probe++) {
        let at = division_at + 2u * probe;
        terminal_bits[base + 1u + probe] = div_f32_bits(local_word(at), local_word(at + 1u));
    }
    let distance_at = word(25u);
    let distance_count = word(26u);
    let distance_output = base + 1u + division_count;
    for (var probe = 0u; probe < distance_count; probe++) {
        let at = distance_at + 4u * probe;
        let current = vec2<f32>(local_float(at), local_float(at + 1u));
        let seed = vec2<f32>(local_float(at + 2u), local_float(at + 3u));
        terminal_bits[distance_output + probe] = u32(terminal_distance_exceeds_eight(current, seed));
    }
    let disparity_at = word(27u);
    let disparity_count = word(28u);
    let disparity_output = distance_output + distance_count;
    for (var probe = 0u; probe < disparity_count; probe++) {
        let at = disparity_at + 2u * probe;
        terminal_bits[disparity_output + probe] = u32(disparity_rejects(vec2<f32>(local_float(at), local_float(at + 1u))));
    }
}

// Each patch's four candidates are independent until the ordered selection.
// Give each candidate the same four reduction lanes as before, concurrently.
@compute @workgroup_size(16)
fn solve_pis_wavefront(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) local_lane: u32,
) {
    let local_index = local_lane;
    if group.x >= words[1] || words[0] != 0x50495301u { return; }
    let descriptor = 2u + 3u * group.x;
    word_base = words[descriptor];
    float_base = words[descriptor + 1u];
    output_base = words[descriptor + 2u];

    let slot = 0u;
    let candidate = (local_index >> 2u) & 3u;
    let lane = local_index & 3u;
    if wave.phase == 0u {
        if group.z != 0u { return; }
        if group.y == 0u { write_qualification_probes(local_index); }
        for (var cell = group.y * 16u + local_index; cell < patches(); cell += 128u) {
            store_flow(cell, flow_at(word(18u), cell));
        }
        return;
    }

    let stripe_rows = min(wave.stripe_rows, patch_rows());
    let row_start = group.z * stripe_rows;
    let local_rows = min(stripe_rows, patch_rows() - row_start);
    let diagonal_count = local_rows + patch_cols() - 1u;
    if wave.diagonal >= diagonal_count { return; }
    let sweep = wave.sweep;
    let diagonal_ordinal = wave.diagonal;
    let diagonal = select(diagonal_ordinal, diagonal_count - 1u - diagonal_ordinal, sweep != 0u);
    var col_min = 0u;
    if diagonal >= local_rows { col_min = diagonal - (local_rows - 1u); }
    let col_max = min(patch_cols() - 1u, diagonal);
    let active_count = col_max - col_min + 1u;
    let participating = group.y < active_count;
    let col = col_min + group.y;
    let local_row = diagonal - col;
    let row = row_start + local_row;
    let cell = row * patch_cols() + col;

    var candidate_flow = vec2<f32>(0.0);
    var present = participating;
    if participating {
        if candidate == 0u {
            candidate_flow = stored_flow(cell);
        } else if candidate == 1u {
            present = word(6u) != 0u;
            candidate_flow = flow_at(word(19u), cell);
        } else if candidate == 2u {
            if sweep == 0u {
                present = col > 0u;
                if present { candidate_flow = stored_flow(cell - 1u); }
            } else {
                present = col + 1u < patch_cols();
                if present { candidate_flow = stored_flow(cell + 1u); }
            }
        } else {
            if sweep == 0u {
                present = local_row > 0u;
                if present { candidate_flow = stored_flow(cell - patch_cols()); }
            } else {
                present = local_row + 1u < local_rows;
                if present { candidate_flow = stored_flow(cell + patch_cols()); }
            }
        }
    }

    let lane_score = candidate_lane_score(cell, candidate_flow, lane, present);
    lane_sums[local_index] = lane_score.sum;
    lane_square_sums[local_index] = lane_score.sum_sq;
    lane_survivors[local_index] = lane_score.survivors;
    if lane == 0u {
        candidate_seeds[slot * 4u + candidate] = candidate_flow;
        candidate_present[slot * 4u + candidate] = u32(present);
    }
    workgroupBarrier();

    if participating && candidate == 0u && lane == 0u {
        selected_seeds[slot] = stored_flow(cell);
        selected_scores[slot] = SENTINEL;
        // Keep candidate order and strict tie selection unchanged.
        for (var candidate = 0u; candidate < 4u; candidate++) {
            if candidate_present[slot * 4u + candidate] == 0u { continue; }
            let base = slot * 16u + candidate * 4u;
            let survivors = (lane_survivors[base] + lane_survivors[base + 1u]) + (lane_survivors[base + 2u] + lane_survivors[base + 3u]);
            var score = SENTINEL;
            if survivors >= 9u {
                let sum = add_rn(add_rn(lane_sums[base], lane_sums[base + 1u]), add_rn(lane_sums[base + 2u], lane_sums[base + 3u]));
                let sum_sq = add_rn(add_rn(lane_square_sums[base], lane_square_sums[base + 1u]), add_rn(lane_square_sums[base + 2u], lane_square_sums[base + 3u]));
                score = sub_rn(sum_sq, div_rn(mul_rn(sum, sum), f32(survivors)));
            }
            if candidate == 0u || score < selected_scores[slot] {
                selected_seeds[slot] = candidate_seeds[slot * 4u + candidate];
                selected_scores[slot] = score;
            }
        }

    }
    if candidate == 0u && lane == 0u {
        descent_active[slot] = u32(participating && word(8u) != 0u);
        selected_scores[slot] = SENTINEL;
    }
    workgroupBarrier();

    let seed = selected_seeds[slot];
    var model: SourceModel;
    if participating && candidate == 0u && lane == 0u {
        model = source_model(cell);
    }
    // All lanes traverse the same barriers. Each patch stops its own
    // arithmetic at the original comparison, after storing that step.
    for (var descent = 0u; descent < 6u; descent++) {
        // One workgroup owns one patch, so its completed descent no
        // longer has to wait through the other patches' six rounds.
        if workgroupUniformLoad(&descent_active[slot]) == 0u { break; }
        prepare_descent_samples(cell, selected_seeds[slot], slot, local_index & 15u, descent_active[slot] != 0u);
        workgroupBarrier();
        if candidate == 0u && lane == 0u && descent_active[slot] != 0u {
            let step = accumulate_descent(model, slot);
            let current = selected_seeds[slot];
            selected_seeds[slot] = vec2<f32>(sub_rn(current.x, step.delta.x), sub_rn(current.y, step.delta.y));
            if step.residual >= selected_scores[slot] {
                descent_active[slot] = 0u;
            }
            selected_scores[slot] = step.residual;
        }
        workgroupBarrier();
    }
    if participating && candidate == 0u && lane == 0u {
        let current = selected_seeds[slot];
        if terminal_distance_exceeds_eight(current, seed) || disparity_rejects(current) {
            store_flow(cell, seed);
        } else {
            store_flow(cell, current);
        }
    }
}
