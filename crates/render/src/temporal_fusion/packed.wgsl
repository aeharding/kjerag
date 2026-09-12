// SPDX-License-Identifier: AGPL-3.0-only
// One fragment fuses the four Y pixels and shared UV pixel of an NV12 quartet.

struct PackedFusionOutput {
    @location(0) y: vec4<f32>,
    @location(1) uv: vec2<f32>,
}

override PERIODIC_X: bool = false;

fn bounded(coordinate: vec2<i32>, size: vec2<i32>) -> vec2<i32> {
    let x = select(clamp(coordinate.x, 0, size.x - 1),
                   ((coordinate.x % size.x) + size.x) % size.x, PERIODIC_X);
    return vec2<i32>(x, clamp(coordinate.y, 0, size.y - 1));
}

@fragment
fn fuse_packed(@builtin(position) position: vec4<f32>) -> PackedFusionOutput {
    let uv_pixel = vec2<u32>(position.xy) + parameters.roi.xy / 2u;
    let y_pixel = uv_pixel * 2u;
    let block = uv_pixel >> vec2<u32>(3u);
    let flow_block = vec2<i32>(min(block, textureDimensions(motion) - vec2<u32>(1u)));
    let luma_block = vec2<i32>(min(block, textureDimensions(luma) - vec2<u32>(1u)));
    let luma_index = textureLoad(luma, luma_block, 0).x;

    let y00 = vec2<i32>(y_pixel);
    let y01 = y00 + vec2<i32>(1, 0);
    let y10 = y00 + vec2<i32>(0, 1);
    let y11 = y00 + vec2<i32>(1, 1);
    let current_y = vec4<f32>(
        textureLoad(images_y, y00, i32(parameters.current), 0).x,
        textureLoad(images_y, y01, i32(parameters.current), 0).x,
        textureLoad(images_y, y10, i32(parameters.current), 0).x,
        textureLoad(images_y, y11, i32(parameters.current), 0).x,
    );
    let current_uv = textureLoad(images_uv, vec2<i32>(uv_pixel), i32(parameters.current), 0).xy;
    let y_bound = parameters.limit * limits[luma_index];
    let uv_bound = parameters.limit * limits[256u + luma_index];
    let y_size = vec2<i32>(textureDimensions(images_y));
    let uv_size = vec2<i32>(textureDimensions(images_uv));
    var y_total = current_y;
    var y_weight_sum = vec4<f32>(1.0);
    var uv_total = current_uv;
    var uv_weight_sum = vec2<f32>(1.0);

    for (var i = 0u; i < parameters.count; i += 1u) {
        let flow = textureLoad(motion, flow_block, i32(i), 0);
        let reference = layer(i);
        if flow.z > 0 {
            let neighbor_y = vec4<f32>(
                textureLoad(images_y, bounded(y00 + flow.xy, y_size), reference, 0).x,
                textureLoad(images_y, bounded(y01 + flow.xy, y_size), reference, 0).x,
                textureLoad(images_y, bounded(y10 + flow.xy, y_size), reference, 0).x,
                textureLoad(images_y, bounded(y11 + flow.xy, y_size), reference, 0).x,
            );
            let confidence = clamp(f32(flow.z) * (1.0 / 255.0), 0.0, 1.0);
            let weight = vec4<f32>(
                contribution(confidence, current_y.x, neighbor_y.x),
                contribution(confidence, current_y.y, neighbor_y.y),
                contribution(confidence, current_y.z, neighbor_y.z),
                contribution(confidence, current_y.w, neighbor_y.w),
            );
            y_total = fma(weight, neighbor_y, y_total);
            y_weight_sum += weight;
        }
        if flow.w > 0 {
            let coordinate = bounded(
                vec2<i32>(uv_pixel) + (flow.xy >> vec2<u32>(1u)), uv_size);
            let neighbor_uv = textureLoad(images_uv, coordinate, reference, 0).xy;
            let confidence = clamp(f32(flow.w) * (1.0 / 255.0), 0.0, 1.0);
            let weight = vec2<f32>(
                contribution(confidence, current_uv.x, neighbor_uv.x),
                contribution(confidence, current_uv.y, neighbor_uv.y),
            );
            uv_total = fma(weight, neighbor_uv, uv_total);
            uv_weight_sum += weight;
        }
    }

    let y_result = clamp(y_total / y_weight_sum, current_y - y_bound, current_y + y_bound);
    let uv_result = clamp(uv_total / uv_weight_sum, current_uv - uv_bound, current_uv + uv_bound);
    var output: PackedFusionOutput;
    output.y = clamp(y_result, vec4<f32>(0.0), vec4<f32>(1.0));
    output.uv = clamp(uv_result, vec2<f32>(0.0), vec2<f32>(1.0));
    return output;
}
