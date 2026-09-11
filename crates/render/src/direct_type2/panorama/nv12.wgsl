// SPDX-License-Identifier: AGPL-3.0-only
// Direct compact body panorama using Kjerag's disclosed full-range NV12 law.

struct CompactNv12Output {
  @location(0) y: vec4<f32>,
  @location(1) uv: vec2<f32>,
}

fn panorama_gamma_rgb(sample_uv: vec2<f32>) -> vec3<f32> {
  let phi = TYPE2_PI * sample_uv.y;
  let theta = TYPE2_TAU * sample_uv.x;
  let body = vec3<f32>(sin(phi) * sin(theta), cos(phi),
    -sin(phi) * cos(theta));
  let map = type2_mesh(body);
  if map.covered <= 0.5 { return vec3<f32>(0.0); }
  let gamma = type2_gamma_color(map).rgb;
  // The removed intermediate was Rgba8Unorm. Keep that quantization explicit;
  // attachment conversion versus pack rounding remains a measured test gate.
  return unpack4x8unorm(pack4x8unorm(vec4<f32>(gamma, 1.0))).rgb;
}

fn convert_rgb_to_ycbcr(rgb: vec3<f32>) -> vec3<f32> {
  let m = reframe.source_matrix;
  let y = (rgb.g + m.y * rgb.b / m.w + m.z * rgb.r / m.x) /
    (1.0 + m.y / m.w + m.z / m.x);
  return vec3<f32>(y, (rgb.b - y) / m.w, (rgb.r - y) / m.x);
}

@fragment
fn panorama_nv12_fs(in: Type2VsOut) -> CompactNv12Output {
  // Mirror the old panorama's interpolated coordinate path, then offset from
  // this 2x2 tile centre to each full-resolution pixel centre.
  let base = in.uv;
  // Both matrix and size come from the same source uniform as lens sampling.
  let full_size = vec2<f32>(reframe.frame_width * 2.0, reframe.frame_height);
  let half_texel = vec2<f32>(0.5) / full_size;
  let a = panorama_gamma_rgb(base + vec2<f32>(-half_texel.x, -half_texel.y));
  let b = panorama_gamma_rgb(base + vec2<f32>( half_texel.x, -half_texel.y));
  let c = panorama_gamma_rgb(base + vec2<f32>(-half_texel.x,  half_texel.y));
  let d = panorama_gamma_rgb(base + vec2<f32>( half_texel.x,  half_texel.y));
  let average = (a + b + c + d) * 0.25;
  var out: CompactNv12Output;
  out.y = clamp(vec4<f32>(convert_rgb_to_ycbcr(a).x,
                          convert_rgb_to_ycbcr(b).x,
                          convert_rgb_to_ycbcr(c).x,
                          convert_rgb_to_ycbcr(d).x),
                vec4<f32>(0.0), vec4<f32>(1.0));
  out.uv = clamp(convert_rgb_to_ycbcr(average).yz +
                 vec2<f32>(0.50196081399917603),
                 vec2<f32>(0.0), vec2<f32>(1.0));
  return out;
}
