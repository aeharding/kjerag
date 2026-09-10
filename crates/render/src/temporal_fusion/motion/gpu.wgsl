struct RawMotion {
    dx: i32,
    dy: i32,
    cost: i32,
}

struct Geometry {
    raw_width: u32,
    raw_height: u32,
    output_width: u32,
    output_height: u32,
    full_width: u32,
    full_height: u32,
    _padding0: u32,
    _padding1: u32,
}

@group(0) @binding(0) var<storage, read> raw: array<RawMotion>;
@group(0) @binding(1) var<storage, read> luma: array<u32>;
@group(0) @binding(2) var<storage, read> thresholds: array<i32>;
@group(0) @binding(3) var<uniform> geometry: Geometry;

@vertex
fn triangle(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn raw_channel(record: RawMotion, channel: u32) -> i32 {
    if channel == 0u {
        return record.dx;
    }
    if channel == 1u {
        return record.dy;
    }
    return record.cost;
}

fn interpolated(coordinate: vec2<u32>, channel: u32) -> f32 {
    let source = vec2<f32>(coordinate) * 0.5;
    let low = vec2<u32>(source);
    let high = min(low + vec2<u32>(1u), vec2<u32>(geometry.raw_width - 1u, geometry.raw_height - 1u));
    let fraction = source - vec2<f32>(low);
    let inverse = vec2<f32>(1.0) - fraction;
    let weights = vec4<f32>(
        inverse.y * inverse.x,
        inverse.y * fraction.x,
        fraction.y * inverse.x,
        fraction.y * fraction.x,
    );
    let top_left = raw[low.y * geometry.raw_width + low.x];
    let top_right = raw[low.y * geometry.raw_width + high.x];
    let bottom_left = raw[high.y * geometry.raw_width + low.x];
    let bottom_right = raw[high.y * geometry.raw_width + high.x];

    // Native order: top-right multiply, then three fused multiply-adds.
    var value = f32(raw_channel(top_right, channel)) * weights.y;
    value = fma(f32(raw_channel(top_left, channel)), weights.x, value);
    value = fma(f32(raw_channel(bottom_left, channel)), weights.z, value);
    return fma(f32(raw_channel(bottom_right, channel)), weights.w, value);
}

fn confidence(threshold: i32, cost: i32) -> i32 {
    if threshold <= cost {
        return 0;
    }
    let threshold_square = u32(threshold) * u32(threshold);
    let cost_square = u32(cost) * u32(cost);
    let difference = threshold_square - cost_square;
    let denominator = threshold_square + cost_square;
    if cost_square == 0u {
        return 256;
    }

    // Eight binary long-division steps compute floor(256*difference/denominator).
    // The split test doubles the remainder only when that addition cannot
    // overflow, avoiding a 40-bit numerator without optional shader integers.
    var remainder = difference;
    var quotient = 0u;
    for (var step = 0u; step < 8u; step++) {
        quotient = quotient << 1u;
        let gap = denominator - remainder;
        if remainder >= gap {
            remainder = remainder - gap;
            quotient = quotient | 1u;
        } else {
            remainder = remainder + remainder;
        }
    }
    return i32(quotient);
}

@fragment
fn pack_motion(@builtin(position) position: vec4<f32>) -> @location(0) vec4<i32> {
    let coordinate = vec2<u32>(position.xy);
    let expanded_dx = i32(interpolated(coordinate, 0u) / 0.5);
    let expanded_dy = i32(interpolated(coordinate, 1u) / 0.5);
    let cost = i32(interpolated(coordinate, 2u));
    let origin = vec2<i32>(coordinate * 16u);
    let maximum = vec2<i32>(vec2<u32>(geometry.full_width - 16u, geometry.full_height - 16u));
    let displacement = clamp(origin + vec2<i32>(expanded_dx, expanded_dy), vec2<i32>(0), maximum) - origin;
    let index = coordinate.y * geometry.output_width + coordinate.x;
    let table_index = luma[index];
    let threshold_y = thresholds[table_index];
    let threshold_uv = thresholds[256u + table_index];
    return vec4<i32>(
        displacement.x,
        displacement.y,
        confidence(threshold_y, cost),
        confidence(threshold_uv, cost),
    );
}
