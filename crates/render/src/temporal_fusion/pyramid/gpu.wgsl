@group(0) @binding(0) var full_y: texture_2d<f32>;
@group(0) @binding(1) var gray: texture_2d<u32>;

@vertex
fn triangle(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn output_coordinate(position: vec4<f32>) -> vec2<u32> {
    return vec2<u32>(position.xy);
}

fn load_gray(coordinate: vec2<u32>) -> u32 {
    return textureLoad(gray, vec2<i32>(coordinate), 0).x;
}

fn rounded_half(a: u32, b: u32) -> u32 {
    return (a + b + 1u) >> 1u;
}

fn filtered(a: u32, b: u32, c: u32, d: u32) -> u32 {
    return (a + 3u * b + 3u * c + d + 4u) >> 3u;
}

@fragment
fn half_y(@builtin(position) position: vec4<f32>) -> @location(0) u32 {
    let source = output_coordinate(position) * 2u;
    let y00 = u32(textureLoad(full_y, vec2<i32>(source), 0).x * 255.0 + 0.5);
    let y10 = u32(textureLoad(full_y, vec2<i32>(source + vec2<u32>(1u, 0u)), 0).x * 255.0 + 0.5);
    let y01 = u32(textureLoad(full_y, vec2<i32>(source + vec2<u32>(0u, 1u)), 0).x * 255.0 + 0.5);
    let y11 = u32(textureLoad(full_y, vec2<i32>(source + vec2<u32>(1u, 1u)), 0).x * 255.0 + 0.5);
    return (y00 + y10 + y01 + y11 + 2u) >> 2u;
}

@fragment
fn copy_gray(@builtin(position) position: vec4<f32>) -> @location(0) u32 {
    return load_gray(output_coordinate(position));
}

@fragment
fn reduce_vertical(@builtin(position) position: vec4<f32>) -> @location(0) u32 {
    let destination = output_coordinate(position);
    let source_size = textureDimensions(gray);
    let destination_height = source_size.y / 2u;
    if destination.y == 0u {
        return rounded_half(load_gray(destination), load_gray(destination + vec2<u32>(0u, 1u)));
    }
    if destination.y + 1u == destination_height {
        return rounded_half(
            load_gray(vec2<u32>(destination.x, source_size.y - 2u)),
            load_gray(vec2<u32>(destination.x, source_size.y - 1u)),
        );
    }
    let center = 2u * destination.y;
    return filtered(
        load_gray(vec2<u32>(destination.x, center - 1u)),
        load_gray(vec2<u32>(destination.x, center)),
        load_gray(vec2<u32>(destination.x, center + 1u)),
        load_gray(vec2<u32>(destination.x, center + 2u)),
    );
}

@fragment
fn reduce_horizontal(@builtin(position) position: vec4<f32>) -> @location(0) u32 {
    let destination = output_coordinate(position);
    let source_size = textureDimensions(gray);
    let destination_width = source_size.x / 2u;
    if destination.x == 0u {
        return rounded_half(load_gray(destination), load_gray(destination + vec2<u32>(1u, 0u)));
    }
    if destination.x + 1u == destination_width {
        return rounded_half(
            load_gray(vec2<u32>(source_size.x - 2u, destination.y)),
            load_gray(vec2<u32>(source_size.x - 1u, destination.y)),
        );
    }
    let center = 2u * destination.x;
    return filtered(
        load_gray(vec2<u32>(center - 1u, destination.y)),
        load_gray(vec2<u32>(center, destination.y)),
        load_gray(vec2<u32>(center + 1u, destination.y)),
        load_gray(vec2<u32>(center + 2u, destination.y)),
    );
}
