// SPDX-License-Identifier: AGPL-3.0-only

const BW = 800u;
const BH = 16u;
const W = 200u;
const H = 100u;
const EW = 212u;
const NODES = 5088u;
const CG_STRIDE = 4u * NODES;
const BLOCK = 2544u;
const QSQ = 0.010000000707805157;
const TOL = 0.00009999999747378752;

@group(0) @binding(0) var<storage, read> band0: array<u32>;
@group(0) @binding(1) var<storage, read> band1: array<u32>;
@group(0) @binding(2) var<storage, read> invalid: array<u32>;
@group(0) @binding(3) var<storage, read> global_validity: array<u32>;
@group(0) @binding(4) var<storage, read_write> state: array<u32>;
@group(0) @binding(5) var<storage, read_write> working: array<u32>;
@group(0) @binding(6) var<storage, read_write> field: array<f32>;
@group(0) @binding(7) var<storage, read_write> work: array<f32>;
@group(0) @binding(8) var<storage, read_write> ratios: array<vec4<f32>>;
@group(0) @binding(9) var<storage, read_write> blurred: array<vec4<f32>>;
@group(0) @binding(10) var<storage, read_write> out0: array<vec4<f32>>;
@group(0) @binding(11) var<storage, read_write> out1: array<vec4<f32>>;
@group(0) @binding(12) var<storage, read> fixed: array<u32>;
@group(0) @binding(13) var texture_out0: texture_storage_2d<rgba32float, write>;
@group(0) @binding(14) var texture_out1: texture_storage_2d<rgba32float, write>;

const CHANGED = 19u;
const SOLVE_ACTIVE = 20u;
const CURRENT_METRIC = 21u;
const RETAINED_METRIC = 22u;
const RETAINED_BUDGET = 23u;
const FIRST_VALID = 24u;
const SEEDS = 25u;
const CONTROL_VALID = 26u;
const STICKY = 32u;
const CONTROL = 880u;

fn chan(p: u32, c: u32) -> u32 { return (p >> (8u * c)) & 255u; }
fn bgr(p: u32) -> vec3<f32> {
  return vec3<f32>(f32(chan(p, 0u)), f32(chan(p, 1u)), f32(chan(p, 2u)));
}
fn source(lens: u32, i: u32) -> u32 {
  if lens == 0u { return band0[i]; }
  return band1[i];
}
fn windex(lens: u32, row: u32, x: u32) -> u32 {
  return lens * EW * H + row * EW + x;
}
fn evidence_slot(row: u32, x: u32) -> u32 { return (row - 49u) * EW + x; }

fn round_div_even(sum: u32, divisor: u32) -> u32 {
  let q = sum / divisor;
  let r = sum % divisor;
  if 2u * r > divisor || (2u * r == divisor && (q & 1u) != 0u) { return q + 1u; }
  return q;
}

// Eighteen integer strip/channel sums, each reduced from 64 disjoint lanes.
// Integer addition preserves the scalar gate exactly; only scheduling changes.
var<workgroup> admission_sums: array<u32, 1152>;

@compute @workgroup_size(64)
fn admit(@builtin(local_invocation_index) lane: u32) {
  if global_validity[0] != 0xffffffffu { return; }
  // Validity is monotone and is accumulated even when content is skipped.
  for (var i = lane; i < 848u; i += 64u) {
    state[STICKY + i] |= invalid[i];
  }
  for (var lens = 0u; lens < 2u; lens += 1u) {
    for (var strip = 0u; strip < 3u; strip += 1u) {
      var sum = vec3<u32>(0u);
      for (var pixel = lane; pixel < 66u * BH; pixel += 64u) {
        let y = pixel / 66u;
        let x = strip * 66u + pixel % 66u;
        let packed = source(lens, y * BW + x);
        sum += vec3<u32>(chan(packed, 0u), chan(packed, 1u), chan(packed, 2u));
      }
      for (var c = 0u; c < 3u; c += 1u) {
        let slot = (lens * 3u + strip) * 3u + c;
        admission_sums[slot * 64u + lane] = sum[c];
      }
    }
  }
  workgroupBarrier();
  for (var half = 32u; half > 0u; half /= 2u) {
    if lane < half {
      for (var slot = 0u; slot < 18u; slot += 1u) {
        let index = slot * 64u + lane;
        admission_sums[index] += admission_sums[index + half];
      }
    }
    workgroupBarrier();
  }
  if lane != 0u { return; }
  var changed = state[18] == 0u;
  for (var i = 0u; i < 18u; i += 1u) {
    let a = admission_sums[i * 64u];
    let b = state[i];
    let delta = select(b - a, a - b, a >= b);
    changed = changed || delta > 3168u;
  }
  state[CHANGED] = select(0u, 1u, changed);
  if changed {
    for (var i = 0u; i < 18u; i += 1u) {
      state[i] = admission_sums[i * 64u];
    }
    state[18] = 1u;
  }
}

// Full 4 by 4 area reduction and the selected periodic/current-row extension.
@compute @workgroup_size(64)
fn prepare(@builtin(global_invocation_id) id: vec3<u32>) {
  if global_validity[0] != 0xffffffffu || state[CHANGED] == 0u || id.x >= 1696u { return; }
  let lens = id.x / 848u;
  let cell = id.x % 848u;
  let r = cell / EW;
  let ex = cell % EW;
  let x = (ex + 194u) % W;
  var sums = vec3<u32>(0u);
  for (var dy = 0u; dy < 4u; dy += 1u) {
    for (var dx = 0u; dx < 4u; dx += 1u) {
      let p = source(lens, (4u * r + dy) * BW + 4u * x + dx);
      sums += vec3<u32>(chan(p, 0u), chan(p, 1u), chan(p, 2u));
    }
  }
  let v = vec3<u32>(round_div_even(sums.x, 16u), round_div_even(sums.y, 16u), round_div_even(sums.z, 16u));
  let packed = v.x | (v.y << 8u) | (v.z << 16u);
  if r == 0u {
    for (var y = 39u; y <= 48u; y += 1u) { working[windex(lens, y, ex)] = packed; }
  } else if r == 3u {
    for (var y = 51u; y <= 60u; y += 1u) { working[windex(lens, y, ex)] = packed; }
  } else {
    working[windex(lens, 48u + r, ex)] = packed;
  }
}

fn supported(row: u32, x: u32) -> vec2<bool> {
  if x == 0u || x + 1u >= EW { return vec2<bool>(false); }
  var ea = 1e-8; var eb = 1e-8; var cross = 0.0; var error = 0.0;
  for (var yy = row - 1u; yy <= row + 1u; yy += 1u) {
    for (var xx = x - 1u; xx <= x + 1u; xx += 1u) {
      let a = bgr(working[windex(0u, yy, xx)]);
      let b = bgr(working[windex(1u, yy, xx)]);
      ea += dot(a, a); eb += dot(b, b); cross += dot(a, b);
      let d = a - b; error += dot(d, d);
    }
  }
  let correlated = cross / sqrt(ea * eb) > 0.96;
  return vec2<bool>(correlated, correlated && error / 27.0 < 6400.0);
}

fn difference(row: u32, x: u32, c: u32) -> i32 {
  return i32(chan(working[windex(1u, row, x)], c)) - i32(chan(working[windex(0u, row, x)], c));
}

var<workgroup> measure_support: array<u32, 424>;
var<workgroup> measure_support_counts: array<atomic<u32>, 2>;
var<workgroup> measure_histograms: array<atomic<u32>, 1533>;
var<workgroup> measure_admitted: atomic<u32>;
var<workgroup> measure_use_correlation: u32;
var<workgroup> measure_bounds: array<i32, 6>;

fn histogram_value(channel: u32, ordinal: u32) -> i32 {
  var accumulated = 0u;
  for (var bin = 0u; bin < 511u; bin += 1u) {
    accumulated += atomicLoad(&measure_histograms[channel * 511u + bin]);
    if accumulated > ordinal { return i32(bin) - 255; }
  }
  return 255;
}

@compute @workgroup_size(64)
fn measure(@builtin(local_invocation_index) lane: u32) {
  if global_validity[0] != 0xffffffffu || state[CHANGED] == 0u { return; }

  for (var i = lane; i < 1533u; i += 64u) {
    atomicStore(&measure_histograms[i], 0u);
  }
  if lane < 2u { atomicStore(&measure_support_counts[lane], 0u); }
  if lane == 0u { atomicStore(&measure_admitted, 0u); }
  workgroupBarrier();

  for (var slot = lane; slot < 424u; slot += 64u) {
    let row = 49u + slot / EW;
    let x = slot % EW;
    let site = supported(row, x);
    measure_support[slot] = select(0u, 1u, site.x) | (select(0u, 1u, site.y) << 1u);
    if site.x { atomicAdd(&measure_support_counts[0], 1u); }
    if site.y { atomicAdd(&measure_support_counts[1], 1u); }
  }
  workgroupBarrier();

  if lane == 0u {
    let correlated = atomicLoad(&measure_support_counts[0]);
    let strict = atomicLoad(&measure_support_counts[1]);
    measure_use_correlation = select(
      0u,
      1u,
      f32(strict) / (f32(correlated) + 1e-6) <= 0.5,
    );
  }
  workgroupBarrier();

  for (var cell = lane; cell < 352u; cell += 64u) {
    let row = 49u + cell / 176u;
    let x = 18u + cell % 176u;
    let slot = evidence_slot(row, x);
    let flags = measure_support[slot];
    let site_supported = select(
      (flags & 2u) != 0u,
      (flags & 1u) != 0u,
      measure_use_correlation != 0u,
    );
    let admitted = site_supported && state[STICKY + (row - 48u) * EW + x] == 0u;
    measure_support[slot] = select(0u, 1u, admitted);
    if admitted {
      atomicAdd(&measure_admitted, 1u);
      for (var channel = 0u; channel < 3u; channel += 1u) {
        let bin = u32(difference(row, x, channel) + 255);
        atomicAdd(&measure_histograms[channel * 511u + bin], 1u);
      }
    }
  }
  workgroupBarrier();

  let count = atomicLoad(&measure_admitted);
  if count == 0u {
    if lane == 0u {
      state[CURRENT_METRIC] = 0xffffffffu;
      state[SOLVE_ACTIVE] = state[CONTROL_VALID];
    }
    return;
  }

  if lane == 0u {
    let rank = u32(f32(count) * 0.2);
    var low = vec3<i32>();
    var high = vec3<i32>();
    var metric = 0u;
    for (var channel = 0u; channel < 3u; channel += 1u) {
      let lower_tail = histogram_value(channel, rank);
      let upper_tail = histogram_value(channel, count - rank - 1u);
      low[channel] = lower_tail - 10;
      high[channel] = upper_tail + 10;
      metric = max(metric, u32(abs(lower_tail)));
      metric = max(metric, u32(abs(upper_tail)));
    }
    if low.x + low.y + low.z + high.x + high.y + high.z >= 0 {
      low = min(low, vec3<i32>(0));
    } else {
      high = max(high, vec3<i32>(0));
    }
    measure_bounds[0] = low.x;
    measure_bounds[1] = low.y;
    measure_bounds[2] = low.z;
    measure_bounds[3] = high.x;
    measure_bounds[4] = high.y;
    measure_bounds[5] = high.z;

    var retained = state[RETAINED_METRIC];
    if state[FIRST_VALID] != 0u {
      retained = clamp(metric, 20u, 100u);
    } else {
      retained = fixed[(retained - 20u) * 81u + clamp(metric, 20u, 100u) - 20u];
    }
    state[CURRENT_METRIC] = metric;
    state[RETAINED_METRIC] = retained;
    state[RETAINED_BUDGET] = clamp(metric / 5u + 1u, 5u, 25u);
    state[SOLVE_ACTIVE] = 1u;
    state[CONTROL_VALID] = 1u;
    state[FIRST_VALID] = 0u;
  }
  storageBarrier();
  workgroupBarrier();

  let low = vec3<i32>(measure_bounds[0], measure_bounds[1], measure_bounds[2]);
  let high = vec3<i32>(measure_bounds[3], measure_bounds[4], measure_bounds[5]);
  let retained = i32(state[RETAINED_METRIC]);
  for (var cell = lane; cell < 352u; cell += 64u) {
    let row = 49u + cell / 176u;
    let x = 18u + cell % 176u;
    let slot = evidence_slot(row, x);
    if measure_support[slot] == 0u {
      state[CONTROL + slot] = 0u;
      continue;
    }
    let d = vec3<i32>(
      difference(row, x, 0u),
      difference(row, x, 1u),
      difference(row, x, 2u),
    );
    let excess = max(
      max(d.x - high.x, low.x - d.x),
      max(max(d.y - high.y, low.y - d.y), max(d.z - high.z, low.z - d.z)),
    );
    var distance = select(u32(max(excess, 1)), 1u, excess <= 0);
    if excess > retained { distance = 0u; }
    state[CONTROL + slot] = distance;
  }
}

fn node_lens(n: u32) -> u32 {
  return n / BLOCK;
}

fn node_row(n: u32) -> u32 {
  return (n % BLOCK) / EW + node_lens(n) * 8u;
}

fn node_x(n: u32) -> u32 {
  return n % EW;
}

fn node_at(lens: u32, row: u32, x: u32) -> u32 {
  let top = lens * 8u;
  if row < top || row >= top + 12u || x >= EW { return NODES; }
  return lens * BLOCK + (row - top) * EW + x;
}

fn answers(lens: u32, absolute_row: u32, node_row_: u32) -> bool {
  return clamp(absolute_row - 40u, lens * 8u, lens * 8u + 11u) == node_row_;
}

fn weight(row: u32, x: u32) -> f32 {
  let d = state[CONTROL + evidence_slot(row, x)];
  if d == 0u { return 0.0; }
  if d == 1u { return 1.0; }
  return (f32(d) - 1.0) * (4.0 / f32(state[RETAINED_METRIC])) + 1.0;
}

fn ycc(p: u32) -> vec3<f32> {
  let q = bgr(p);
  return vec3<f32>(
    q.z * 0.299 + q.y * 0.587 + q.x * 0.114,
    q.z * -0.168736 - q.y * 0.331264 + q.x * 0.5 + 128.0,
    q.z * 0.5 - q.y * 0.418688 - q.x * 0.081312 + 128.0,
  );
}

fn rhs_at(n: u32, c: u32) -> f32 {
  let lens = node_lens(n);
  let nr = node_row(n);
  let x = node_x(n);
  var out = 0.0;
  if x < 18u || x >= 194u { return 0.0; }
  for (var row = 49u; row <= 50u; row += 1u) {
    if answers(lens, row, nr) {
      let d = ycc(working[windex(1u, row, x)]) - ycc(working[windex(0u, row, x)]);
      out += select(-weight(row, x) * d[c], weight(row, x) * d[c], lens == 0u);
    }
  }
  return out;
}

fn diagonal_at(n: u32) -> f32 {
  let lens = node_lens(n);
  let row = node_row(n);
  let x = node_x(n);
  var d = 0.0;
  if row > lens * 8u { d += QSQ; }
  if row < lens * 8u + 11u { d += QSQ; }
  if x > 0u { d += QSQ; }
  if x + 1u < EW { d += QSQ; }
  if x >= 18u && x < 194u {
    for (var ar = 49u; ar <= 50u; ar += 1u) {
      if answers(lens, ar, row) {
        let w = weight(ar, x);
        d += w * w;
      }
    }
  }
  return d;
}

fn vector_at(node: u32, field_base: u32, work_base: u32, direction: bool) -> f32 {
  return select(field[field_base + node], work[work_base + NODES + node], direction);
}

fn quadratic(n: u32, field_base: u32, work_base: u32, direction: bool) -> f32 {
  let lens = node_lens(n);
  let row = node_row(n);
  let x = node_x(n);
  let here = vector_at(n, field_base, work_base, direction);
  var out = 0.0;
  if row > lens * 8u {
    let other = node_at(lens, row - 1u, x);
    out += QSQ * (here - vector_at(other, field_base, work_base, direction));
  }
  if row < lens * 8u + 11u {
    let other = node_at(lens, row + 1u, x);
    out += QSQ * (here - vector_at(other, field_base, work_base, direction));
  }
  if x > 0u {
    let other = node_at(lens, row, x - 1u);
    out += QSQ * (here - vector_at(other, field_base, work_base, direction));
  }
  if x + 1u < EW {
    let other = node_at(lens, row, x + 1u);
    out += QSQ * (here - vector_at(other, field_base, work_base, direction));
  }
  if x >= 18u && x < 194u {
    for (var ar = 49u; ar <= 50u; ar += 1u) {
      if answers(lens, ar, row) {
        let other_lens = 1u - lens;
        let other_row = clamp(ar - 40u, other_lens * 8u, other_lens * 8u + 11u);
        let other = node_at(other_lens, other_row, x);
        let w = weight(ar, x);
        out += w * w * (here - vector_at(other, field_base, work_base, direction));
      }
    }
  }
  return out;
}

var<workgroup> sums: array<f32, 64>;

fn total(lane: u32, v: f32) -> f32 {
  sums[lane] = v;
  workgroupBarrier();
  for (var half = 32u; half > 0u; half /= 2u) {
    if lane < half { sums[lane] += sums[lane + half]; }
    workgroupBarrier();
  }
  let result = sums[0];
  workgroupBarrier();
  return result;
}

fn dot_work(lane: u32, a: u32, b: u32) -> f32 {
  var value = 0.0;
  for (var i = lane; i < NODES; i += 64u) { value += work[a + i] * work[b + i]; }
  return total(lane, value);
}

@compute @workgroup_size(64)
fn solve(
  @builtin(local_invocation_index) lane: u32,
  @builtin(workgroup_id) group: vec3<u32>,
) {
  if global_validity[0] != 0xffffffffu || state[CHANGED] == 0u || state[SOLVE_ACTIVE] == 0u { return; }
  let budget = select(state[RETAINED_BUDGET], 100u, state[SEEDS] == 0u);
  // Channels have independent fields and scratch. Each keeps the same
  // 64-lane reduction order; the following pass commits their shared seed flag.
  let channel = group.x;
  let field_base = channel * NODES;
  let work_base = channel * CG_STRIDE;
  for (var i = lane; i < NODES; i += 64u) {
    let diagonal = diagonal_at(i);
    work[work_base + 3u * NODES + i] = select(1.0, 1.0 / diagonal, diagonal != 0.0);
    work[work_base + i] = rhs_at(i, channel);
  }
  storageBarrier(); workgroupBarrier();
  var qq = 0.0;
  for (var i = lane; i < NODES; i += 64u) {
    qq += work[work_base + i] * work[work_base + i];
  }
  qq = total(lane, qq);
  if qq == 0.0 {
    for (var i = lane; i < NODES; i += 64u) { field[field_base + i] = 0.0; }
    storageBarrier(); workgroupBarrier();
    return;
  }
  for (var i = lane; i < NODES; i += 64u) {
    work[work_base + i] -= quadratic(i, field_base, work_base, false);
  }
  storageBarrier(); workgroupBarrier();
  let threshold = max(TOL * TOL * qq, 1.1754944e-38);
  var rho = 0.0;
  var initial_residual = 0.0;
  for (var i = lane; i < NODES; i += 64u) {
    let z = work[work_base + 3u * NODES + i] * work[work_base + i];
    work[work_base + NODES + i] = z;
    rho += work[work_base + i] * z;
    initial_residual += work[work_base + i] * work[work_base + i];
  }
  rho = total(lane, rho);
  initial_residual = total(lane, initial_residual);
  storageBarrier(); workgroupBarrier();
  for (var step = 0u; step < budget && initial_residual >= threshold; step += 1u) {
    for (var i = lane; i < NODES; i += 64u) {
      work[work_base + 2u * NODES + i] = quadratic(i, field_base, work_base, true);
    }
    storageBarrier(); workgroupBarrier();
    let denominator = dot_work(lane, work_base + NODES, work_base + 2u * NODES);
    if denominator == 0.0 { break; }
    let alpha = rho / denominator;
    var residual = 0.0;
    for (var i = lane; i < NODES; i += 64u) {
      field[field_base + i] += alpha * work[work_base + NODES + i];
      work[work_base + i] -= alpha * work[work_base + 2u * NODES + i];
      residual += work[work_base + i] * work[work_base + i];
    }
    storageBarrier();
    residual = total(lane, residual);
    if residual < threshold { break; }
    var rho2 = 0.0;
    for (var i = lane; i < NODES; i += 64u) {
      rho2 += work[work_base + i] * work[work_base + 3u * NODES + i] * work[work_base + i];
    }
    rho2 = total(lane, rho2);
    if rho == 0.0 { break; }
    let beta = rho2 / rho;
    for (var i = lane; i < NODES; i += 64u) {
      work[work_base + NODES + i] = work[work_base + 3u * NODES + i]
        * work[work_base + i] + beta * work[work_base + NODES + i];
    }
    storageBarrier(); workgroupBarrier();
    rho = rho2;
  }
  var mean = 0.0;
  for (var i = lane; i < NODES; i += 64u) { mean += field[field_base + i]; }
  mean = total(lane, mean) / f32(NODES);
  for (var i = lane; i < NODES; i += 64u) { field[field_base + i] -= mean; }
  storageBarrier(); workgroupBarrier();
}

fn round_even(v: f32) -> i32 {
  let lower = floor(v);
  let fraction = v - lower;
  var rounded = i32(lower);
  if fraction > 0.5 || (fraction == 0.5 && (rounded & 1) != 0) {
    rounded += 1;
  }
  return rounded;
}

fn byte(v: f32) -> u32 {
  if !(v >= -2147483648.0 && v < 2147483648.0) { return 0u; }
  return u32(clamp(round_even(v), 0, 255));
}
fn corrected(lens: u32, row: u32, x: u32) -> vec3<f32> {
  let packed = working[windex(lens, row, x + 6u)];
  if state[SOLVE_ACTIVE] == 0u { return bgr(packed); }
  let source_ycc = ycc(packed);
  let node = node_at(lens, row - 40u, x + 6u);
  let value = source_ycc + vec3<f32>(field[node], field[NODES + node], field[2u * NODES + node]);
  let cb = value.y - 128.0;
  let cr = value.z - 128.0;
  return vec3<f32>(
    f32(byte(cb * 1.772 + value.x)),
    f32(byte(value.x - cb * 0.34414 - cr * 0.71414)),
    f32(byte(cr * 1.402 + value.x)),
  );
}

@compute @workgroup_size(64)
fn make_ratios(@builtin(global_invocation_id) id: vec3<u32>) {
  if global_validity[0] != 0xffffffffu || state[CHANGED] == 0u || id.x >= 40000u { return; }
  if id.x == 0u && state[SOLVE_ACTIVE] != 0u { state[SEEDS] = 1u; }
  let lens = id.x / 20000u;
  let index = id.x % 20000u;
  let row = index / W;
  let x = index % W;
  var write = false;
  var neutral = false;
  var source_row = row;
  if lens == 0u {
    neutral = row >= 35u && row < 40u;
    if row >= 41u && row < 60u {
      write = true;
      if row >= 50u { source_row = 49u; }
    }
  } else {
    neutral = row >= 60u && row < 66u;
    if row >= 40u && row < 60u {
      write = true;
      if row < 50u { source_row = 50u; }
    }
  }
  if neutral {
    ratios[lens * 20000u + index] = vec4<f32>(1.0, 1.0, 1.0, 0.0);
  } else if write {
    let current = bgr(working[windex(lens, source_row, x + 6u)]);
    let prepared = corrected(lens, source_row, x);
    ratios[lens * 20000u + index] = vec4<f32>((prepared + 255.0) / (current + 255.0), 0.0);
  }
}

fn reflect_y(y: i32) -> u32 {
  if y < 0 { return u32(-y); }
  if y >= 20 { return u32(38 - y); }
  return u32(y);
}

@compute @workgroup_size(64)
fn blur(@builtin(global_invocation_id) id: vec3<u32>) {
  if global_validity[0] != 0xffffffffu || state[CHANGED] == 0u || id.x >= 40000u { return; }
  let lens = id.x / 20000u;
  let index = id.x % 20000u;
  let row = index / W;
  let x = index % W;
  let first = select(35u, 45u, lens == 1u);
  var value = ratios[lens * 20000u + index];
  if row >= first && row < first + 20u {
    var sum = vec3<f32>(0.0);
    let roi_row = i32(row - first);
    for (var dy = -5; dy <= 5; dy += 1) {
      let source_row = first + reflect_y(roi_row + dy);
      for (var dx = -1; dx <= 1; dx += 1) {
        let source_x = u32((i32(x) + dx + 200) % 200);
        sum += ratios[lens * 20000u + source_row * W + source_x].xyz;
      }
    }
    value = vec4<f32>(sum / 33.0, 0.0);
  }
  blurred[lens * 20000u + index] = value;
}

@compute @workgroup_size(64)
fn retain_blur(@builtin(global_invocation_id) id: vec3<u32>) {
  if global_validity[0] != 0xffffffffu || state[CHANGED] == 0u || id.x >= 40000u { return; }
  let lens = id.x / 20000u;
  let i = id.x % 20000u;
  let row = i / W;
  let first = select(35u, 45u, lens == 1u);
  if row >= first && row < first + 20u {
    ratios[lens * 20000u + i] = blurred[lens * 20000u + i];
  }
}

fn remap_coord(index: u32) -> vec2<f32> {
  let at = 6561u + 2u * index;
  return vec2<f32>(bitcast<f32>(fixed[at]), bitcast<f32>(fixed[at + 1u]));
}

fn blurred_at(lens: u32, x: u32, y: u32) -> vec3<f32> {
  let periodic_x = u32((i32(x) - 1 + 200) % 200);
  return blurred[lens * 20000u + y * W + periodic_x].xyz;
}

@compute @workgroup_size(64)
fn remap(@builtin(global_invocation_id) id: vec3<u32>) {
  if id.x >= 20000u { return; }
  for (var lens = 0u; lens < 2u; lens += 1u) {
    var value = vec4<f32>(1.0, 1.0, 1.0, 0.0);
    if global_validity[0] == 0xffffffffu {
      let p = remap_coord(id.x);
      let low = select(35.0, 45.0, lens == 1u);
      let high = select(55.0, 65.0, lens == 1u);
      if p.y >= low && p.y <= high {
        let ix = u32(p.x);
        let iy = u32(p.y);
        let fraction = fract(p);
        let a = blurred_at(lens, ix, iy);
        let b = blurred_at(lens, min(ix + 1u, 201u), iy);
        let c = blurred_at(lens, ix, min(iy + 1u, 99u));
        let d = blurred_at(lens, min(ix + 1u, 201u), min(iy + 1u, 99u));
        var mapped = a;
        if !(all(a == b) && all(a == c) && all(a == d)) {
          mapped = mix(mix(a, b, fraction.x), mix(c, d, fraction.x), fraction.y);
        }
        value = vec4<f32>(mapped.zyx, 0.0);
      }
    }
    let position = vec2<i32>(i32(id.x % W), i32(id.x / W));
    if lens == 0u {
      out0[id.x] = value;
      textureStore(texture_out0, position, value);
    } else {
      out1[id.x] = value;
      textureStore(texture_out1, position, value);
    }
  }
}
