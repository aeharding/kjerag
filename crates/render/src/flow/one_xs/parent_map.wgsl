const PI: f32 = 3.14159265358979323846;
const COLS: u32 = 200u;
const ROWS: u32 = 100u;
const PARAM_WORDS: u32 = 33u;
const POSE_BASE: u32 = 66u;
const LENS_OUTPUT_WORDS: u32 = 40000u;
// GENERATED_TABLES

@group(0) @binding(0) var<storage, read> inputs: array<u32>;
@group(0) @binding(1) var<storage, read_write> outputs: array<u32>;

struct RemapResult {
    valid: bool,
    position: vec2f,
}

fn materialize(value: f32) -> f32 { return bitcast<f32>(bitcast<u32>(value) ^ inputs[20u]); }
fn mul_rn(a: f32, b: f32) -> f32 { return materialize(fma(a, b, -0.0)); }
fn add_rn(a: f32, b: f32) -> f32 { return materialize(fma(a, 1.0, b)); }
fn sub_rn(a: f32, b: f32) -> f32 { return materialize(fma(-1.0, b, a)); }

fn div_f32_bits(a: u32, b: u32) -> u32 {
    let sign = (a ^ b) & 0x80000000u;
    let a_abs = a & 0x7fffffffu; let b_abs = b & 0x7fffffffu;
    let a_exp = a_abs >> 23u; let b_exp = b_abs >> 23u;
    let a_frac = a_abs & 0x007fffffu; let b_frac = b_abs & 0x007fffffu;
    if a_exp == 0xffu && a_frac != 0u { return a | 0x00400000u; }
    if b_exp == 0xffu && b_frac != 0u { return b | 0x00400000u; }
    if (a_abs == 0u && b_abs == 0u) || (a_exp == 0xffu && b_exp == 0xffu) { return 0xffc00000u; }
    if b_abs == 0u || a_exp == 0xffu { return sign | 0x7f800000u; }
    if a_abs == 0u || b_exp == 0xffu { return sign; }
    var ma = a_frac; var mb = b_frac;
    var ea = i32(a_exp) - 127; var eb = i32(b_exp) - 127;
    if a_exp == 0u { let top = 31u - countLeadingZeros(a_frac); ma = a_frac << (23u - top); ea = i32(top) - 149; } else { ma |= 0x00800000u; }
    if b_exp == 0u { let top = 31u - countLeadingZeros(b_frac); mb = b_frac << (23u - top); eb = i32(top) - 149; } else { mb |= 0x00800000u; }
    var remainder = ma; var quotient_exponent = ea - eb;
    if remainder < mb { remainder <<= 1u; quotient_exponent -= 1; }
    var quotient = 0u;
    for (var step = 0u; step < 24u; step++) { let bit = 23u - step; if remainder >= mb { remainder -= mb; quotient |= 1u << bit; } if step != 23u { remainder <<= 1u; } }
    if quotient_exponent >= -126 {
        let twice_remainder = remainder << 1u;
        if twice_remainder > mb || (twice_remainder == mb && (quotient & 1u) != 0u) { quotient += 1u; }
        if quotient == 0x01000000u { quotient = 0x00800000u; quotient_exponent += 1; }
        if quotient_exponent > 127 { return sign | 0x7f800000u; }
        return sign | (u32(quotient_exponent + 127) << 23u) | (quotient & 0x007fffffu);
    }
    let shift = u32(-126 - quotient_exponent); if shift >= 25u { return sign; }
    var subnormal = quotient >> shift; let mask = (1u << shift) - 1u; let low = quotient & mask; let half = 1u << (shift - 1u);
    if low > half || (low == half && (remainder != 0u || (subnormal & 1u) != 0u)) { subnormal += 1u; }
    return sign | subnormal;
}
fn div_rn(a: f32, b: f32) -> f32 { return bitcast<f32>(div_f32_bits(bitcast<u32>(a), bitcast<u32>(b))); }
fn sqrt_rn(value: f32) -> f32 {
    let bits = bitcast<u32>(value);
    let exponent_bits = (bits >> 23u) & 0xffu;
    if exponent_bits == 0u || exponent_bits == 0xffu || (bits & 0x80000000u) != 0u {
        return materialize(sqrt(value));
    }
    var exponent = i32(exponent_bits) - 127;
    var mantissa = (bits & 0x007fffffu) | 0x00800000u;
    if (exponent % 2) != 0 { mantissa <<= 1u; exponent -= 1; }
    var root = 0u;
    var remainder = 0u;
    for (var pair_index = 23i; pair_index >= 0i; pair_index--) {
        let source = pair_index * 2i - 23i;
        var pair = 0u;
        if source >= 0i { pair = (mantissa >> u32(source)) & 3u; }
        else if source == -1i { pair = (mantissa << 1u) & 3u; }
        remainder = (remainder << 2u) | pair;
        root <<= 1u;
        let trial = (root << 1u) | 1u;
        if remainder >= trial { remainder -= trial; root += 1u; }
    }
    if remainder > root { root += 1u; }
    var output_exponent = exponent / 2 + 127;
    if root == 0x01000000u { root >>= 1u; output_exponent += 1; }
    return bitcast<f32>((u32(output_exponent) << 23u) | (root & 0x007fffffu));
}

fn lane(lens: u32, offset: u32) -> f32 {
    return bitcast<f32>(inputs[lens * PARAM_WORDS + offset]);
}

fn quat_param(lens: u32, offset: u32) -> vec4f {
    return vec4f(lane(lens, offset), lane(lens, offset + 1u), lane(lens, offset + 2u), lane(lens, offset + 3u));
}

fn pose(index: u32) -> vec4f {
    let base = POSE_BASE + index * 4u;
    return vec4f(bitcast<f32>(inputs[base]), bitcast<f32>(inputs[base + 1u]), bitcast<f32>(inputs[base + 2u]), bitcast<f32>(inputs[base + 3u]));
}

fn cross_product(left: vec3f, right: vec3f) -> vec3f {
    return vec3f(sub_rn(mul_rn(left.y, right.z), mul_rn(left.z, right.y)),
                 sub_rn(mul_rn(left.z, right.x), mul_rn(left.x, right.z)),
                 sub_rn(mul_rn(left.x, right.y), mul_rn(left.y, right.x)));
}

fn quaternion_multiply(q1: vec4f, q2: vec4f) -> vec4f {
    let x = add_rn(sub_rn(add_rn(mul_rn(q2.w,q1.x),mul_rn(q2.z,q1.y)),mul_rn(q2.y,q1.z)),mul_rn(q2.x,q1.w));
    let y = add_rn(add_rn(add_rn(mul_rn(-q2.z,q1.x),mul_rn(q2.w,q1.y)),mul_rn(q2.x,q1.z)),mul_rn(q2.y,q1.w));
    let z = add_rn(add_rn(sub_rn(mul_rn(q2.y,q1.x),mul_rn(q2.x,q1.y)),mul_rn(q2.w,q1.z)),mul_rn(q2.z,q1.w));
    let w = add_rn(sub_rn(sub_rn(mul_rn(-q2.x,q1.x),mul_rn(q2.y,q1.y)),mul_rn(q2.z,q1.z)),mul_rn(q2.w,q1.w));
    return vec4f(x, y, z, w);
}

fn quaternion_rotate(q: vec4f, vector: vec3f) -> vec3f {
    let first_cross = cross_product(q.xyz, vector);
    let t = vec3f(mul_rn(2.0, first_cross.x), mul_rn(2.0, first_cross.y), mul_rn(2.0, first_cross.z));
    let second_cross = cross_product(q.xyz, t);
    return vec3f(add_rn(add_rn(vector.x, mul_rn(q.w,t.x)), second_cross.x),
                 add_rn(add_rn(vector.y, mul_rn(q.w,t.y)), second_cross.y),
                 add_rn(add_rn(vector.z, mul_rn(q.w,t.z)), second_cross.z));
}

fn m_atan2(y: f32, x: f32) -> f32 {
    if x > 0.0 { return atan(y / x); }
    if x < 0.0 {
        if y >= 0.0 { return atan(y / x) + PI; }
        return atan(y / x) - PI;
    }
    if y > 0.0 { return PI / 2.0; }
    if y < 0.0 { return -(PI / 2.0); }
    return -1.0;
}

fn quaternion_slerp(q1: vec4f, incoming_q2: vec4f, t: f32) -> vec4f {
    var q2 = incoming_q2;
    var cosa = add_rn(add_rn(add_rn(mul_rn(q1.x,q2.x),mul_rn(q1.y,q2.y)),mul_rn(q1.z,q2.z)),mul_rn(q1.w,q2.w));
    if cosa < 0.0 { q2 = -q2; cosa = -cosa; }
    var k0: f32;
    var k1: f32;
    if cosa > 0.9995 {
        k0 = sub_rn(1.0, t);
        k1 = t;
    } else {
        let sina = sqrt_rn(sub_rn(1.0, mul_rn(cosa,cosa)));
        let angle = m_atan2(sina, cosa);
        k0 = sin((1.0 - t) * angle) / sina;
        k1 = sin(t * angle) / sina;
    }
    return vec4f(add_rn(mul_rn(k0,q1.x),mul_rn(k1,q2.x)), add_rn(mul_rn(k0,q1.y),mul_rn(k1,q2.y)),
                 add_rn(mul_rn(k0,q1.z),mul_rn(k1,q2.z)), add_rn(mul_rn(k0,q1.w),mul_rn(k1,q2.w)));
}

fn get_quat(factor: f32) -> vec4f {
    let factor_index = mul_rn(factor, 50.0);
    let floored = floor(factor_index);
    if floored >= 0.0 && floored < 50.0 {
        let index = u32(floored);
        return quaternion_slerp(pose(index), pose(index + 1u), sub_rn(factor_index, floored));
    }
    if floored < 0.0 { return pose(0u); }
    return pose(50u);
}

fn back_project(column: u32, row: u32) -> vec3f {
    let radius = bitcast<f32>(THETA_COS[row]);
    return vec3f(mul_rn(radius, bitcast<f32>(PHI_COS[column])),
                 mul_rn(radius, bitcast<f32>(PHI_SIN[column])),
                 bitcast<f32>(THETA_SIN[row]));
}

fn radtan_distort(position: vec2f, lens: u32) -> vec2f {
    let mx2 = mul_rn(position.x, position.x); let my2 = mul_rn(position.y, position.y); let mxy = mul_rn(position.x, position.y);
    let rho2 = add_rn(mx2, my2); let rho4 = mul_rn(rho2, rho2); let rho6 = mul_rn(rho4, rho2);
    let radial = add_rn(add_rn(mul_rn(lane(lens,26u),rho2),mul_rn(mul_rn(lane(lens,27u),rho2),rho2)),mul_rn(lane(lens,28u),rho6));
    return vec2f(add_rn(add_rn(add_rn(position.x,mul_rn(position.x,radial)),mul_rn(mul_rn(2.0,lane(lens,29u)),mxy)),mul_rn(lane(lens,30u),add_rn(rho2,mul_rn(2.0,mx2)))),
                 add_rn(add_rn(add_rn(position.y,mul_rn(position.y,radial)),mul_rn(mul_rn(2.0,lane(lens,30u)),mxy)),mul_rn(lane(lens,29u),add_rn(rho2,mul_rn(2.0,my2)))));
}

fn remap(dst_pos: vec2f, column: u32, row: u32, quaternion: vec4f, lens: u32) -> RemapResult {
    let ray = quaternion_rotate(quaternion, back_project(column, row));
    let rho2 = add_rn(mul_rn(ray.x,ray.x),mul_rn(ray.y,ray.y));
    var valid = true;
    var pixel: vec2f;
    if rho2 > 0.0 {
        let radius = sqrt_rn(add_rn(rho2,mul_rn(ray.z,ray.z)));
        if div_rn(ray.z,radius) < sub_rn(bitcast<f32>(MAX_FOV_COS),0.01) {
            valid = false;
            pixel = vec2f(-1.0, -1.0);
        } else {
            let reciprocal = div_rn(1.0,add_rn(ray.z,mul_rn(lane(lens,22u),radius)));
            let distorted = radtan_distort(vec2f(mul_rn(reciprocal,ray.x),mul_rn(reciprocal,ray.y)),lens);
            pixel = vec2f(add_rn(mul_rn(mul_rn(distorted.x,lane(lens,2u)),lane(lens,24u)),lane(lens,0u)),
                          add_rn(mul_rn(distorted.y,lane(lens,3u)),lane(lens,1u)));
        }
    } else {
        pixel = vec2f(lane(lens, 0u), lane(lens, 1u));
    }
    return RemapResult(valid, vec2f(mul_rn(pixel.x,div_rn(1.0,sub_rn(lane(lens,4u),1.0))),
                                    mul_rn(pixel.y,div_rn(1.0,sub_rn(lane(lens,5u),1.0)))));
}

fn scan_factor(position: vec2f, lens: u32) -> f32 {
    if inputs[lens * PARAM_WORDS + 23u] != 0u { return mul_rn(position.x,lane(lens,6u)); }
    return mul_rn(position.y,lane(lens,7u));
}

fn calc_cell(column: u32, row: u32, lens: u32) -> vec2f {
    let dst_pos = vec2f(bitcast<f32>(DST_X[column]), bitcast<f32>(DST_Y[row]));
    let initial = remap(dst_pos, column, row, quat_param(lens, 16u), lens);
    if !initial.valid { return vec2f(-1.0, -1.0); }
    var src_pos = initial.position;
    for (var iteration = 0u; iteration < 4u; iteration++) {
        let pre_pos = src_pos;
        let corrected = quaternion_multiply(quaternion_multiply(quat_param(lens, 8u), get_quat(scan_factor(src_pos, lens))), quat_param(lens, 12u));
        src_pos = remap(dst_pos, column, row, corrected, lens).position;
        let difference = vec2f(mul_rn(sub_rn(src_pos.x,pre_pos.x),lane(lens,31u)),mul_rn(sub_rn(src_pos.y,pre_pos.y),lane(lens,32u)));
        let movement = sqrt_rn(add_rn(mul_rn(difference.x,difference.x),mul_rn(difference.y,difference.y)));
        if movement < 4.0 { break; }
    }
    let corrected = quaternion_multiply(quaternion_multiply(quat_param(lens, 8u), get_quat(scan_factor(src_pos, lens))), quat_param(lens, 12u));
    let final_result = remap(dst_pos, column, row, corrected, lens);
    if final_result.valid {
        return vec2f(add_rn(final_result.position.x,lane(lens,20u)),add_rn(final_result.position.y,lane(lens,21u)));
    }
    return vec2f(-1.0, -1.0);
}

@compute @workgroup_size(8, 8, 1)
fn calc_parent_maps(@builtin(global_invocation_id) gid: vec3u) {
    if gid.x >= COLS || gid.y >= ROWS || gid.z >= 2u { return; }
    let value = calc_cell(gid.x, gid.y, gid.z);
    let cell = gid.y * COLS + gid.x;
    let base = gid.z * LENS_OUTPUT_WORDS + cell * 2u;
    outputs[base] = bitcast<u32>(value.x);
    outputs[base + 1u] = bitcast<u32>(value.y);
}
