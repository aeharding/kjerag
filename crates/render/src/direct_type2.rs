//! Direct GPU consumer for Studio's selected ONE X2 type-2 resources.
//!
//! The packed UV and alpha grids stay at their native 200 by 100 shape. The
//! fragment reconstructs the readable [`crate::map_oracle::Mesh`] from the
//! current `Reframe` body ray, rather than accepting a dense output-sized map.

use std::num::NonZeroU64;

use crate::FrameStamp;
use crate::projection;
use crate::studio_type2::{ALPHA_BYTES, OneXsMapFrame, PACKED_BYTES};

pub(crate) struct DirectMapDraw {
    pub(crate) pipeline: wgpu::RenderPipeline,
    pub(crate) read: wgpu::BindGroup,
    packed: wgpu::Buffer,
    alpha: wgpu::Buffer,
    bound_frame: Option<FrameStamp>,
}

impl DirectMapDraw {
    pub(crate) fn new(
        device: &wgpu::Device,
        picture_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> Self {
        let packed = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 native packed map"),
            size: PACKED_BYTES as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let alpha = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ONE X2 native alpha map"),
            size: ALPHA_BYTES as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let reading = layout(device, wgpu::ShaderStages::FRAGMENT);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ONE X2 direct type-2 map"),
            source: wgpu::ShaderSource::Wgsl(draw_wgsl().into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ONE X2 direct type-2 map"),
            bind_group_layouts: &[picture_layout, &reading],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ONE X2 direct type-2 map"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let read = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ONE X2 native type-2 resources"),
            layout: &reading,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: packed.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: alpha.as_entire_binding(),
                },
            ],
        });
        Self {
            pipeline,
            read,
            packed,
            alpha,
            bound_frame: None,
        }
    }

    pub(crate) fn upload(&mut self, queue: &wgpu::Queue, map: &OneXsMapFrame) {
        queue.write_buffer(&self.packed, 0, map.packed().bytes());
        queue.write_buffer(&self.alpha, 0, map.alpha().bytes());
        self.bound_frame = Some(map.frame().clone());
    }

    pub(crate) fn bound_frame(&self) -> Option<&FrameStamp> {
        self.bound_frame.as_ref()
    }
}

fn layout(device: &wgpu::Device, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayout {
    let storage = |binding, bytes| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(bytes),
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ONE X2 native type-2 resources"),
        entries: &[
            storage(0, PACKED_BYTES as u64),
            storage(1, ALPHA_BYTES as u64),
        ],
    })
}

pub(crate) fn draw_wgsl() -> String {
    format!("{}\n{}\n{DRAW}", projection::wgsl(), map_wgsl())
}

pub(crate) fn map_wgsl() -> &'static str {
    MAP
}

const MAP: &str = r#"
@group(1) @binding(0) var<storage, read> type2_packed: array<vec4<f32>>;
@group(1) @binding(1) var<storage, read> type2_alpha: array<f32>;

const TYPE2_MAP_W = 200i;
const TYPE2_MAP_H = 100i;
const TYPE2_SLICES = 100i;
const TYPE2_STACKS = 50i;
const TYPE2_PI = 3.14159265358979323846;
const TYPE2_TAU = 6.28318530717958647692;

struct Type2Sample {
  packed: vec4<f32>,
  alpha: f32,
  covered: f32,
};

fn type2_map_index(x: i32, y: i32) -> u32 {
  let periodic_x = ((x % TYPE2_MAP_W) + TYPE2_MAP_W) % TYPE2_MAP_W;
  let pole_y = clamp(y, 0, TYPE2_MAP_H - 1);
  return u32(pole_y * TYPE2_MAP_W + periodic_x);
}

fn type2_bilinear_weights(uv: vec2<f32>) -> vec4<f32> {
  let p = uv * vec2<f32>(f32(TYPE2_MAP_W), f32(TYPE2_MAP_H));
  let f = fract(p);
  let w = select(f - vec2<f32>(0.5), f + vec2<f32>(0.5), f < vec2<f32>(0.5));
  return vec4<f32>(floor(p - select(vec2<f32>(0.0), vec2<f32>(1.0), f < vec2<f32>(0.5))), w);
}

fn type2_sample4(uv: vec2<f32>) -> vec4<f32> {
  let q = type2_bilinear_weights(uv);
  let i = vec2<i32>(q.xy);
  let a = type2_packed[type2_map_index(i.x, i.y)];
  let b = type2_packed[type2_map_index(i.x + 1, i.y)];
  let c = type2_packed[type2_map_index(i.x, i.y + 1)];
  let d = type2_packed[type2_map_index(i.x + 1, i.y + 1)];
  return mix(mix(a, b, q.z), mix(c, d, q.z), q.w);
}

fn type2_sample1(uv: vec2<f32>) -> f32 {
  let q = type2_bilinear_weights(uv);
  let i = vec2<i32>(q.xy);
  let a = type2_alpha[type2_map_index(i.x, i.y)];
  let b = type2_alpha[type2_map_index(i.x + 1, i.y)];
  let c = type2_alpha[type2_map_index(i.x, i.y + 1)];
  let d = type2_alpha[type2_map_index(i.x + 1, i.y + 1)];
  return mix(mix(a, b, q.z), mix(c, d, q.z), q.w);
}

fn type2_position(row: i32, col: i32) -> vec3<f32> {
  let phi = TYPE2_PI * f32(row) / f32(TYPE2_STACKS);
  let theta = TYPE2_TAU * f32(col) / f32(TYPE2_SLICES);
  return vec3<f32>(-sin(phi) * sin(theta), cos(phi), sin(phi) * cos(theta));
}

fn type2_varying(row: i32, col: i32) -> vec2<f32> {
  return vec2<f32>(
    (f32(col) / f32(TYPE2_SLICES) + 0.5) + 0.5 / f32(TYPE2_MAP_W),
    ((f32(row) / f32(TYPE2_STACKS) * 99.0) + 0.5) / f32(TYPE2_MAP_H),
  );
}

// Watertight dominant-axis ray/triangle intersection against a ray from the
// sphere centre. Shared edges evaluate through the same two-product edge
// function in opposite order, so f32 rounding cannot leave a crack between
// adjacent triangles. xyz are the barycentrics in vertex order and w is
// exact admission; there is no epsilon or extrapolation.
fn type2_triangle(ray: vec3<f32>, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>) -> vec4<f32> {
  let absolute = abs(ray);
  var kz = 0u;
  if absolute.y > absolute.x { kz = 1u; }
  if absolute.z > absolute[kz] { kz = 2u; }
  var kx = (kz + 1u) % 3u;
  var ky = (kx + 1u) % 3u;
  if ray[kz] < 0.0 {
    let swap = kx;
    kx = ky;
    ky = swap;
  }
  let sx = ray[kx] / ray[kz];
  let sy = ray[ky] / ray[kz];
  let sz = 1.0 / ray[kz];
  let ax = a[kx] - sx * a[kz];
  let ay = a[ky] - sy * a[kz];
  let bx = b[kx] - sx * b[kz];
  let by = b[ky] - sy * b[kz];
  let cx = c[kx] - sx * c[kz];
  let cy = c[ky] - sy * c[kz];
  let wa = cx * by - cy * bx;
  let wb = ax * cy - ay * cx;
  let wc = bx * ay - by * ax;
  let has_negative = wa < 0.0 || wb < 0.0 || wc < 0.0;
  let has_positive = wa > 0.0 || wb > 0.0 || wc > 0.0;
  if has_negative && has_positive { return vec4<f32>(0.0); }
  let determinant = wa + wb + wc;
  if determinant == 0.0 { return vec4<f32>(0.0); }
  let distance_numerator = wa * (sz * a[kz]) + wb * (sz * b[kz]) + wc * (sz * c[kz]);
  if distance_numerator * determinant <= 0.0 { return vec4<f32>(0.0); }
  let inverse = 1.0 / determinant;
  return vec4<f32>(wa * inverse, wb * inverse, wc * inverse, 1.0);
}

fn type2_cell(ray: vec3<f32>, row: i32, col_unwrapped: i32) -> Type2Sample {
  var out: Type2Sample;
  let col = ((col_unwrapped % TYPE2_SLICES) + TYPE2_SLICES) % TYPE2_SLICES;
  let p00 = type2_position(row, col);
  let p01 = type2_position(row, col + 1);
  let p10 = type2_position(row + 1, col);
  let p11 = type2_position(row + 1, col + 1);
  let uv00 = type2_varying(row, col);
  let uv01 = type2_varying(row, col + 1);
  let uv10 = type2_varying(row + 1, col);
  let uv11 = type2_varying(row + 1, col + 1);

  var weights = type2_triangle(ray, p00, p01, p10);
  var uv = weights.x * uv00 + weights.y * uv01 + weights.z * uv10;
  var packed = weights.x * type2_sample4(uv00) + weights.y * type2_sample4(uv01) + weights.z * type2_sample4(uv10);
  if weights.w == 0.0 {
    weights = type2_triangle(ray, p01, p11, p10);
    uv = weights.x * uv01 + weights.y * uv11 + weights.z * uv10;
    packed = weights.x * type2_sample4(uv01) + weights.y * type2_sample4(uv11) + weights.z * type2_sample4(uv10);
  }
  if weights.w > 0.0 {
    out.packed = packed;
    out.alpha = type2_sample1(uv);
    out.covered = 1.0;
  }
  return out;
}

fn type2_mesh(body: vec3<f32>) -> Type2Sample {
  var empty: Type2Sample;
  let ray = normalize(vec3<f32>(-body.x, body.y, -body.z));
  let row = i32(clamp(floor(acos(clamp(ray.y, -1.0, 1.0)) * f32(TYPE2_STACKS) / TYPE2_PI), 0.0, f32(TYPE2_STACKS - 1)));
  var theta = atan2(-ray.x, ray.z);
  if theta < 0.0 { theta += TYPE2_TAU; }
  let col = i32(floor(theta * f32(TYPE2_SLICES) / TYPE2_TAU)) % TYPE2_SLICES;
  let neighbours = array<vec2<i32>, 9>(
    vec2<i32>(0, 0), vec2<i32>(-1, -1), vec2<i32>(-1, 0),
    vec2<i32>(-1, 1), vec2<i32>(0, -1), vec2<i32>(0, 1),
    vec2<i32>(1, -1), vec2<i32>(1, 0), vec2<i32>(1, 1),
  );
  for (var candidate = 0u; candidate < 9u; candidate += 1u) {
    let at = vec2<i32>(row, col) + neighbours[candidate];
    if at.x < 0 || at.x >= TYPE2_STACKS { continue; }
    let found = type2_cell(ray, at.x, at.y);
    if found.covered > 0.5 { return found; }
  }
  return empty;
}
"#;

const DRAW: &str = r#"
struct Type2VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) index: u32) -> Type2VsOut {
  let x = f32((index << 1u) & 2u);
  let y = f32(index & 2u);
  var out: Type2VsOut;
  out.uv = vec2<f32>(x, y);
  out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  return out;
}

@group(0) @binding(1) var type2_luma0: texture_2d<f32>;
@group(0) @binding(2) var type2_chroma0: texture_2d<f32>;
@group(0) @binding(3) var type2_luma1: texture_2d<f32>;
@group(0) @binding(4) var type2_chroma1: texture_2d<f32>;
@group(0) @binding(5) var type2_sampler: sampler;

fn type2_atlas_load(a: texture_2d<f32>, b: texture_2d<f32>, p: vec2<i32>) -> vec4<f32> {
  let dims = textureDimensions(a);
  let x = clamp(p.x, 0, i32(2u * dims.x) - 1);
  let y = clamp(p.y, 0, i32(dims.y) - 1);
  if x < i32(dims.x) { return textureLoad(a, vec2<i32>(x, y), 0); }
  return textureLoad(b, vec2<i32>(x - i32(dims.x), y), 0);
}

fn type2_atlas_linear(a: texture_2d<f32>, b: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
  let dims = textureDimensions(a);
  let p = uv * vec2<f32>(f32(2u * dims.x), f32(dims.y)) - vec2<f32>(0.5);
  let base = vec2<i32>(floor(p));
  let f = fract(p);
  let top = mix(type2_atlas_load(a, b, base), type2_atlas_load(a, b, base + vec2<i32>(1, 0)), f.x);
  let bottom = mix(type2_atlas_load(a, b, base + vec2<i32>(0, 1)), type2_atlas_load(a, b, base + vec2<i32>(1, 1)), f.x);
  return mix(top, bottom, f.y);
}

fn type2_box(a: texture_2d<f32>, b: texture_2d<f32>, uv: vec2<f32>, logical: vec2<f32>) -> vec4<f32> {
  let box_size = vec2<f32>(1.7881766557693481);
  let start = uv * logical - box_size * 0.5;
  let end = start + box_size;
  var sum = vec4<f32>(0.0);
  var area = 0.0;
  var y = floor(start.y);
  loop {
    if y >= end.y { break; }
    let low_y = max(y, start.y);
    let high_y = min(y + 2.0, end.y);
    var x = floor(start.x);
    loop {
      if x >= end.x { break; }
      let low_x = max(x, start.x);
      let high_x = min(x + 2.0, end.x);
      let cell_area = (high_x - low_x) * (high_y - low_y);
      let sample_uv = vec2<f32>((low_x + high_x) * 0.5 / logical.x, (low_y + high_y) * 0.5 / logical.y);
      sum += type2_atlas_linear(a, b, sample_uv) * cell_area;
      area += cell_area;
      x += 2.0;
    }
    y += 2.0;
  }
  return select(type2_atlas_linear(a, b, uv), sum / area, area > 0.0010000000474974513);
}

fn type2_ycbcr(uv: vec2<f32>) -> vec3<f32> {
  let luma = type2_box(type2_luma0, type2_luma1, uv, vec2<f32>(2880.0));
  let chroma = type2_box(type2_chroma0, type2_chroma1, uv, vec2<f32>(1440.0));
  let c = chroma.rg - vec2<f32>(0.50196081399917603);
  return vec3<f32>(
    luma.r + 1.4019999504089355 * c.g,
    luma.r - 0.34400001168251038 * c.r - 0.71399998664855957 * c.g,
    luma.r + 1.7719999551773071 * c.r,
  );
}

@fragment
fn fs(in: Type2VsOut) -> @location(0) vec4<f32> {
  let view = view_ray(in.uv);
  if view.w <= 0.0 { return vec4<f32>(0.0); }
  let map = type2_mesh(reframe.view_to_body * view.xyz);
  if map.covered <= 0.5 { return vec4<f32>(0.0); }
  let b = type2_ycbcr(map.packed.zw);
  let a = type2_ycbcr(map.packed.xy);
  let rgb = mix(b, a, map.alpha);
  let linear = select(
    rgb / 12.92,
    pow((rgb + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)),
    rgb > vec3<f32>(0.04045),
  );
  return vec4<f32>(select(rgb, linear, reframe.linearize > 0.5), 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use std::future::Future;

    use super::*;
    use crate::map_oracle::{CapturedMap, DensePixel, MAP_HEIGHT, MAP_WIDTH};
    use crate::projection::{Held, Reframe};
    use crate::{Camera, Size};

    #[test]
    fn direct_shader_parses_and_uses_only_two_native_map_bindings() {
        use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

        let source = draw_wgsl();
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("direct type-2 WGSL did not parse: {error}"));
        Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|error| panic!("direct type-2 WGSL did not validate: {error}"));
        assert!(source.contains("type2_mesh(reframe.view_to_body * view.xyz)"));
        assert!(source.contains("@group(1) @binding(0)"));
        assert!(source.contains("@group(1) @binding(1)"));
        assert!(!source.contains("@group(2)"));
        assert!(
            !source.contains("@builtin(position) position: vec4<f32>,\n  @location(0) uv")
                || source.contains("view_ray(in.uv)")
        );
        assert!(source.contains("select(rgb, linear, reframe.linearize > 0.5)"));
    }

    #[test]
    fn native_grid_gpu_matches_dense_cpu_across_views_and_poles() {
        let (device, queue) = match gpu() {
            Ok(open) => open,
            Err(why) => {
                assert!(
                    std::env::var("KJERAG_REQUIRE_GPU").is_err(),
                    "KJERAG_REQUIRE_GPU is set and there is no GPU to answer with: {why}",
                );
                eprintln!("direct type-2 twin: skipped, no GPU on this box ({why})");
                return;
            }
        };
        let packed: Vec<[f32; 4]> = (0..MAP_HEIGHT)
            .flat_map(|row| {
                (0..MAP_WIDTH).map(move |col| {
                    let longitude = col as f32 / MAP_WIDTH as f32;
                    let latitude = row as f32 / MAP_HEIGHT as f32;
                    [longitude, latitude, 0.5 + 0.5 * longitude, 1.0 - latitude]
                })
            })
            .collect();
        let alpha: Vec<f32> = (0..MAP_HEIGHT)
            .flat_map(|row| {
                (0..MAP_WIDTH).map(move |col| {
                    0.5 + 0.25 * (std::f32::consts::TAU * col as f32 / MAP_WIDTH as f32).sin()
                        + 0.2 * row as f32 / MAP_HEIGHT as f32
                })
            })
            .collect();
        let map = CapturedMap::new(packed, alpha).unwrap();
        let size = Size::new(48, 36);
        for (label, camera) in [
            (
                "ordinary",
                Camera {
                    yaw: 0.73,
                    pitch: -0.21,
                    fov: 1.15,
                },
            ),
            (
                "north pole",
                Camera {
                    yaw: 2.1,
                    pitch: std::f32::consts::FRAC_PI_2,
                    fov: 1.0,
                },
            ),
            (
                "south pole and periodic seam",
                Camera {
                    yaw: -std::f32::consts::PI,
                    pitch: -std::f32::consts::FRAC_PI_2,
                    fov: 1.0,
                },
            ),
        ] {
            let reframe = Reframe::new(
                &crate::projection::tests::one_xs_lenses(),
                crate::projection::tests::ONE_XS_FRAME,
                camera,
                Held::default(),
                size.width as f32 / size.height as f32,
                false,
                crate::Sampling::Bilinear,
            );
            let cpu = map.rasterize(&reframe, size);
            let gpu = on_gpu(&device, &queue, &reframe, &map, size);
            assert_eq!(gpu.len(), cpu.pixels.len());
            let mut worst = 0.0f32;
            let mut coverage_mismatch = 0usize;
            for (expected, actual) in cpu.pixels.iter().zip(&gpu) {
                if expected.covered != actual.covered {
                    coverage_mismatch += 1;
                    continue;
                }
                if expected.covered == 0.0 {
                    continue;
                }
                for (a, b) in expected
                    .packed_uv
                    .iter()
                    .flatten()
                    .chain(std::iter::once(&expected.alpha))
                    .zip(
                        actual
                            .packed_uv
                            .iter()
                            .flatten()
                            .chain(std::iter::once(&actual.alpha)),
                    )
                {
                    worst = worst.max((a - b).abs());
                }
            }
            assert_eq!(coverage_mismatch, 0, "{label}: triangle admission differs");
            assert!(
                worst <= 2.0e-4,
                "{label}: worst native-grid residue is {worst}"
            );
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            match future.as_mut().poll(&mut context) {
                std::task::Poll::Ready(answer) => return answer,
                std::task::Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn gpu() -> Result<(wgpu::Device, wgpu::Queue), String> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..Default::default()
        });
        let adapter = block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
            .into_iter()
            .next()
            .ok_or("no Vulkan adapter")?;
        block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("direct type-2 twin"),
            required_features: wgpu::Features::empty(),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .map_err(|error| error.to_string())
    }

    fn bytes_of<T>(values: &[T]) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
        }
    }

    fn on_gpu(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        reframe: &Reframe,
        map: &CapturedMap,
        size: Size,
    ) -> Vec<DensePixel> {
        let probe = format!(
            r#"
@group(1) @binding(2) var<storage, read_write> type2_answers: array<vec4<f32>>;
const PROBE_WIDTH = {width}u;
const PROBE_HEIGHT = {height}u;

@compute @workgroup_size(64)
fn direct_type2_twin(@builtin(global_invocation_id) id: vec3<u32>) {{
  let count = PROBE_WIDTH * PROBE_HEIGHT;
  if id.x >= count {{ return; }}
  let x = id.x % PROBE_WIDTH;
  let y = id.x / PROBE_WIDTH;
  let uv = vec2<f32>((f32(x) + 0.5) / f32(PROBE_WIDTH), (f32(y) + 0.5) / f32(PROBE_HEIGHT));
  let view = view_ray(uv);
  var answer: Type2Sample;
  if view.w > 0.0 {{ answer = type2_mesh(reframe.view_to_body * view.xyz); }}
  type2_answers[id.x * 2u] = answer.packed;
  type2_answers[id.x * 2u + 1u] = vec4<f32>(answer.alpha, answer.covered, 0.0, 0.0);
}}
"#,
            width = size.width,
            height = size.height,
        );
        let source = format!("{}\n{}\n{probe}", projection::wgsl(), map_wgsl());
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("direct type-2 twin"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("direct type-2 twin uniform"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Reframe>() as u64),
                },
                count: None,
            }],
        });
        let storage = |binding, read_only, bytes| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(bytes),
            },
            count: None,
        };
        let answer_bytes = u64::from(size.width) * u64::from(size.height) * 32;
        let map_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("direct type-2 twin maps"),
            entries: &[
                storage(0, true, PACKED_BYTES as u64),
                storage(1, true, ALPHA_BYTES as u64),
                storage(2, false, answer_bytes),
            ],
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("direct type-2 twin"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("direct type-2 twin"),
                    bind_group_layouts: &[&uniform_layout, &map_layout],
                    immediate_size: 0,
                }),
            ),
            module: &module,
            entry_point: Some("direct_type2_twin"),
            compilation_options: Default::default(),
            cache: None,
        });
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let uniform = buffer(
            "direct type-2 twin uniform",
            std::mem::size_of::<Reframe>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let packed = buffer(
            "direct type-2 twin packed",
            PACKED_BYTES as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let alpha = buffer(
            "direct type-2 twin alpha",
            ALPHA_BYTES as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let answers = buffer(
            "direct type-2 twin answers",
            answer_bytes,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let readback = buffer(
            "direct type-2 twin readback",
            answer_bytes,
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        queue.write_buffer(&uniform, 0, reframe.bytes());
        queue.write_buffer(&packed, 0, bytes_of(&map.packed));
        queue.write_buffer(&alpha, 0, bytes_of(&map.alpha));
        let group0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("direct type-2 twin uniform"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let group1 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("direct type-2 twin maps"),
            layout: &map_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: packed.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: alpha.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: answers.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group0, &[]);
            pass.set_bind_group(1, &group1, &[]);
            pass.dispatch_workgroups((size.width * size.height).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&answers, 0, &readback, 0, answer_bytes);
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = readback.slice(..).get_mapped_range();
        let floats: Vec<f32> = mapped
            .chunks_exact(4)
            .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
            .collect();
        let pixels = floats
            .chunks_exact(8)
            .map(DensePixel::from_test_lanes)
            .collect();
        drop(mapped);
        readback.unmap();
        pixels
    }
}
