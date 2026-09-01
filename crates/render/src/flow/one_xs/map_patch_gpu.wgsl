// SPDX-License-Identifier: AGPL-3.0-only

const OUTPUT_NODES = 20000u;
const RETAINED_ROWS = 1080u;
const RETAINED_COLS = 60u;
const RETAINED_NODES = RETAINED_ROWS * RETAINED_COLS;

// One side is preimage, base, flow, gate, coordinate in that order. Every
// float2 occupies two words. Lens A / B-to-A begins at zero; lens B / A-to-B
// follows it. Rust constructs this fixed layout and checks its size.
const PREIMAGE_WORDS = OUTPUT_NODES * 2u;
const BASE_WORDS = RETAINED_NODES * 2u;
const FLOW_WORDS = RETAINED_NODES * 2u;
const GATE_WORDS = OUTPUT_NODES;
const COORDINATE_WORDS = OUTPUT_NODES * 2u;
const SIDE_WORDS = PREIMAGE_WORDS + BASE_WORDS + FLOW_WORDS + GATE_WORDS + COORDINATE_WORDS;

@group(0) @binding(0) var<storage, read> input_words: array<u32>;
@group(0) @binding(1) var<storage, read_write> packed_map: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> action_receipt: array<vec2<u32>>;

struct Taps {
  indices: vec4<u32>,
  weights: vec4<f32>,
};

struct Materialized {
  uv: vec2<f32>,
  action: u32,
};

fn read_f32(word: u32) -> f32 {
  return bitcast<f32>(input_words[word]);
}

fn positive_epsilon() -> f32 {
  return bitcast<f32>(0x322bcc77u);
}

fn is_nan(value: f32) -> bool {
  let bits = bitcast<u32>(value);
  return (bits & 0x7f800000u) == 0x7f800000u && (bits & 0x007fffffu) != 0u;
}

fn read_f32x2(word: u32) -> vec2<f32> {
  return vec2<f32>(read_f32(word), read_f32(word + 1u));
}

fn preimage_base(side: u32) -> u32 { return side * SIDE_WORDS; }
fn base_base(side: u32) -> u32 { return preimage_base(side) + PREIMAGE_WORDS; }
fn flow_base(side: u32) -> u32 { return base_base(side) + BASE_WORDS; }
fn gate_base(side: u32) -> u32 { return flow_base(side) + FLOW_WORDS; }
fn coordinate_base(side: u32) -> u32 { return gate_base(side) + GATE_WORDS; }

fn preimage(side: u32, node: u32) -> vec2<f32> {
  return read_f32x2(preimage_base(side) + node * 2u);
}

fn base_value(side: u32, node: u32) -> vec2<f32> {
  return read_f32x2(base_base(side) + node * 2u);
}

fn flow_value(side: u32, node: u32) -> vec2<f32> {
  return read_f32x2(flow_base(side) + node * 2u);
}

fn gate_value(side: u32, node: u32) -> f32 {
  return read_f32(gate_base(side) + node);
}

fn coordinate(side: u32, node: u32) -> vec2<f32> {
  return read_f32x2(coordinate_base(side) + node * 2u);
}

// Rust f32::min/max return the numeric operand for one-NaN inputs. Spell that
// out because WGSL deliberately leaves more latitude to backend lowering.
fn rust_min(left: f32, right: f32) -> f32 {
  if is_nan(left) { return right; }
  if is_nan(right) { return left; }
  return min(left, right);
}

fn rust_max(left: f32, right: f32) -> f32 {
  if is_nan(left) { return right; }
  if is_nan(right) { return left; }
  return max(left, right);
}

fn make_taps(row: f32, col: f32) -> Taps {
  let col0 = u32(floor(col));
  let row0 = u32(floor(row));
  let col1 = min(col0 + 1u, RETAINED_COLS - 1u);
  let row1 = min(row0 + 1u, RETAINED_ROWS - 1u);
  let row_inverse = (1.0 - row) + f32(row0);
  let col_inverse = (1.0 - col) + f32(col0);
  let tl = row_inverse * col_inverse;
  let tr = row_inverse - tl;
  let bl = col_inverse - tl;
  let tmp = (1.0 - col_inverse) - row_inverse;
  let br = tmp + tl;
  return Taps(
    vec4<u32>(
      row0 * RETAINED_COLS + col0,
      row0 * RETAINED_COLS + col1,
      row1 * RETAINED_COLS + col0,
      row1 * RETAINED_COLS + col1,
    ),
    vec4<f32>(tl, tr, bl, br),
  );
}

fn flow_taps(c: vec2<f32>) -> Taps {
  let col = rust_max(rust_min(c.x, f32(RETAINED_COLS) - 1.0), 0.0);
  let row = rust_max(rust_min(c.y, f32(RETAINED_ROWS) - 1.0), 0.0);
  return make_taps(row, col);
}

fn base_taps(c: vec2<f32>) -> Taps {
  // Preserve both native clamp pairs even though their extents are equal in
  // this selected fixed shape.
  var col = rust_max(c.x, 0.0);
  col = rust_min(col, f32(RETAINED_COLS) - 1.0);
  col = rust_min(col, f32(RETAINED_COLS) - 1.0);
  col = rust_max(col, 0.0);
  var row = rust_max(c.y, 0.0);
  row = rust_min(row, f32(RETAINED_ROWS) - 1.0);
  row = rust_min(row, f32(RETAINED_ROWS) - 1.0);
  row = rust_max(row, 0.0);
  return make_taps(row, col);
}

fn interpolate_flow(side: u32, taps: Taps) -> vec2<f32> {
  let top_right = flow_value(side, taps.indices.y) * taps.weights.y;
  let with_top_left = fma(flow_value(side, taps.indices.x), vec2<f32>(taps.weights.x), top_right);
  let with_bottom_left = fma(flow_value(side, taps.indices.z), vec2<f32>(taps.weights.z), with_top_left);
  return fma(flow_value(side, taps.indices.w), vec2<f32>(taps.weights.w), with_bottom_left);
}

fn interpolate_base(side: u32, taps: Taps) -> vec2<f32> {
  let top_right = base_value(side, taps.indices.y) * taps.weights.y;
  let with_top_left = fma(base_value(side, taps.indices.x), vec2<f32>(taps.weights.x), top_right);
  let with_bottom_left = fma(base_value(side, taps.indices.z), vec2<f32>(taps.weights.z), with_top_left);
  return fma(base_value(side, taps.indices.w), vec2<f32>(taps.weights.w), with_bottom_left);
}

fn materialize_side(side: u32, node: u32) -> Materialized {
  let retained = preimage(side, node);
  let gate = gate_value(side, node);
  // Ordered comparisons make an unordered gate continue.
  if gate <= 0.0 || gate >= 1.0 { return Materialized(retained, 1u); }

  let c = coordinate(side, node);
  let sampled_flow = interpolate_flow(side, flow_taps(c));
  let weight = 1.0 - gate;
  let q = fma(sampled_flow, vec2<f32>(weight), c);
  let taps = base_taps(q);

  // The tap guard checks only U. Unordered U values continue.
  if base_value(side, taps.indices.x).x <= positive_epsilon() ||
     base_value(side, taps.indices.y).x <= positive_epsilon() ||
     base_value(side, taps.indices.z).x <= positive_epsilon() ||
     base_value(side, taps.indices.w).x <= positive_epsilon() {
    return Materialized(retained, 2u);
  }
  let uv = interpolate_base(side, taps);
  // This final conjunction rejects unordered U or V.
  if !(uv.x > positive_epsilon() && uv.y > positive_epsilon()) {
    return Materialized(retained, 3u);
  }
  return Materialized(uv, 4u);
}

@compute @workgroup_size(64)
fn materialize_type2(@builtin(global_invocation_id) gid: vec3<u32>) {
  let node = gid.x;
  if node >= OUTPUT_NODES { return; }
  // Side zero is lens A / B-to-A. Side one is lens B / A-to-B.
  let left_result = materialize_side(0u, node);
  let right_result = materialize_side(1u, node);
  let left = left_result.uv;
  let right = right_result.uv;
  let left_u = left.x * 0.5;
  let right_plus_one = right.x + 1.0;
  let right_u = right_plus_one * 0.5;
  packed_map[node] = vec4<f32>(left_u, left.y, right_u, right.y);
  action_receipt[node] = vec2<u32>(left_result.action, right_result.action);
}
