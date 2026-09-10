// SPDX-License-Identifier: AGPL-3.0-only
// Original expression of the recovered normalized pixel-fusion law.
// No coefficient interpolation or default history policy lives here.

struct Parameters {
    roi: vec4<u32>,
    noise: f32,
    limit: f32,
    current: u32,
    count: u32,
    layers: array<vec4<u32>, 2>,
}
@group(0) @binding(0) var images_y: texture_2d_array<f32>;
@group(0) @binding(1) var images_uv: texture_2d_array<f32>;
@group(0) @binding(2) var motion: texture_2d_array<i32>;
@group(0) @binding(3) var luma: texture_2d<u32>;
@group(0) @binding(4) var<uniform> parameters: Parameters;
@group(0) @binding(5) var<storage, read> limits: array<f32, 512>;

@vertex
fn triangle(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

fn layer(index: u32) -> i32 {
    return i32(parameters.layers[index / 4u][index % 4u]);
}

fn contribution(confidence: f32, current: f32, neighbor: f32) -> f32 {
    let difference = abs(current - neighbor);
    let limited = clamp(1.5 * parameters.noise / max(difference, 0.000001) - 0.5,
                        0.0, confidence);
    return select(limited, confidence, difference < parameters.noise);
}

@fragment
fn fuse_y(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<u32>(position.xy) + parameters.roi.xy;
    let block = pixel >> vec2<u32>(4u);
    let flow_block = vec2<i32>(min(block, textureDimensions(motion) - vec2<u32>(1u)));
    let luma_block = vec2<i32>(min(block, textureDimensions(luma) - vec2<u32>(1u)));
    let current = textureLoad(images_y, vec2<i32>(pixel), i32(parameters.current), 0).x;
    let bound = parameters.limit * limits[textureLoad(luma, luma_block, 0).x];
    let last = vec2<i32>(textureDimensions(images_y)) - vec2<i32>(1);
    var total = current;
    var weight_sum = 1.0;
    for (var i = 0u; i < parameters.count; i += 1u) {
        let flow = textureLoad(motion, flow_block, i32(i), 0);
        // Normalized source samples are finite. A nonpositive confidence
        // gives exactly zero weight, so neither the neighbor fetch nor its
        // difference/division can change total or weight_sum.
        if flow.z <= 0 {
            continue;
        }
        let coordinate = clamp(vec2<i32>(pixel) + flow.xy, vec2<i32>(0), last);
        let neighbor = textureLoad(images_y, coordinate, layer(i), 0).x;
        let weight = contribution(clamp(f32(flow.z) * (1.0 / 255.0), 0.0, 1.0), current, neighbor);
        total = fma(weight, neighbor, total);
        weight_sum += weight;
    }
    let result = clamp(total / weight_sum, current - bound, current + bound);
    return vec4<f32>(clamp(result, 0.0, 1.0), 0.0, 0.0, 1.0);
}

@fragment
fn fuse_uv(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<u32>(position.xy) + parameters.roi.xy / 2u;
    let block = pixel >> vec2<u32>(3u);
    let flow_block = vec2<i32>(min(block, textureDimensions(motion) - vec2<u32>(1u)));
    let luma_block = vec2<i32>(min(block, textureDimensions(luma) - vec2<u32>(1u)));
    let current = textureLoad(images_uv, vec2<i32>(pixel), i32(parameters.current), 0).xy;
    let bound = parameters.limit * limits[256u + textureLoad(luma, luma_block, 0).x];
    let last = vec2<i32>(textureDimensions(images_uv)) - vec2<i32>(1);
    var total = current;
    var weight_sum = vec2<f32>(1.0);
    for (var i = 0u; i < parameters.count; i += 1u) {
        let flow = textureLoad(motion, flow_block, i32(i), 0);
        if flow.w <= 0 {
            continue;
        }
        let coordinate = clamp(vec2<i32>(pixel) + (flow.xy >> vec2<u32>(1u)), vec2<i32>(0), last);
        let neighbor = textureLoad(images_uv, coordinate, layer(i), 0).xy;
        let confidence = clamp(f32(flow.w) * (1.0 / 255.0), 0.0, 1.0);
        let weight = vec2<f32>(contribution(confidence, current.x, neighbor.x),
                               contribution(confidence, current.y, neighbor.y));
        total = fma(weight, neighbor, total);
        weight_sum += weight;
    }
    let result = clamp(total / weight_sum, current - bound, current + bound);
    return vec4<f32>(clamp(result, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0, 1.0);
}
