//! Screen-space broad phase for exact curved-view ray intersections.
//!
//! A regular indexed screen grid estimates native cell coordinates at vertices.
//! Fragments use the interpolated coordinates only as a search hint. The real
//! pixel ray must pass the unchanged watertight cell test; a missed hint uses
//! the original complete search. No lens UV, alpha, color or temporal value is
//! interpolated from this grid, and its density cannot create a picture hole.

const SIDE: u32 = 128;

pub(super) struct Grid {
    indices: wgpu::Buffer,
    count: u32,
}

impl Grid {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        use wgpu::util::DeviceExt;
        let indices = indices();
        let bytes = indices
            .iter()
            .flat_map(|index| index.to_ne_bytes())
            .collect::<Vec<_>>();
        Self {
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("curved view search-hint screen grid"),
                contents: &bytes,
                usage: wgpu::BufferUsages::INDEX,
            }),
            count: indices.len() as u32,
        }
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.count, 0, 0..1);
    }
}

fn indices() -> Vec<u16> {
    let stride = SIDE + 1;
    let mut indices = Vec::with_capacity((SIDE * SIDE * 6) as usize);
    for row in 0..SIDE {
        for col in 0..SIDE {
            let at = row * stride + col;
            indices.extend(
                [
                    at,
                    at + 1,
                    at + stride,
                    at + 1,
                    at + stride + 1,
                    at + stride,
                ]
                .map(|index| u16::try_from(index).expect("screen-grid index fits u16")),
            );
        }
    }
    indices
}

pub(super) fn shader_source() -> String {
    format!("const CURVED_GRID_SIDE: u32 = {SIDE}u;\n{WGSL}")
}

const WGSL: &str = r#"
struct CurvedViewOut {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) cell_hint: vec2<f32>,
};

@vertex
fn curved_vs(@builtin(vertex_index) index: u32) -> CurvedViewOut {
  let stride = CURVED_GRID_SIDE + 1u;
  let uv = vec2<f32>(f32(index % stride), f32(index / stride)) / f32(CURVED_GRID_SIDE);
  var out: CurvedViewOut;
  out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
  out.uv = uv;
  let view = view_ray(uv);
  let body = reframe.view_to_body * view.xyz;
  let ray = normalize(vec3<f32>(-body.x, body.y, -body.z));
  var theta = atan2(-ray.x, ray.z);
  if theta < 0.0 { theta += TYPE2_TAU; }
  out.cell_hint = vec2<f32>(
    acos(clamp(ray.y, -1.0, 1.0)) * f32(TYPE2_STACKS) / TYPE2_PI,
    theta * f32(TYPE2_SLICES) / TYPE2_TAU,
  );
  return out;
}

fn curved_exact_map(body: vec3<f32>, hint: vec2<f32>) -> Type2Sample {
  let ray = normalize(vec3<f32>(-body.x, body.y, -body.z));
  let row = clamp(i32(floor(hint.x)), 0, TYPE2_STACKS - 1);
  let col = i32(floor(hint.y));
  let found = type2_cell(ray, row, col);
  if found.covered > 0.5 { return found; }
  // Chart cuts, poles and interpolation error affect only hint hit rate.
  // Never extrapolate or sample a picture from the approximate coordinates.
  return type2_mesh(body);
}

@fragment
fn corrected_curved_fs(in: CurvedViewOut) -> @location(0) vec4<f32> {
  let view = view_ray(in.uv);
  if view.w <= 0.0 { return vec4<f32>(0.0); }
  let body = reframe.view_to_body * view.xyz;
  let map = curved_exact_map(body, in.cell_hint);
  if map.covered <= 0.5 { return vec4<f32>(0.0); }
  return corrected_color(type2_gamma_color(map).rgb, body);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_grid_covers_each_screen_cell_without_crossing_rows() {
        let indices = indices();
        assert_eq!(indices.len(), (SIDE * SIDE * 6) as usize);
        let stride = SIDE + 1;
        for (cell, triangle_pair) in indices.chunks_exact(6).enumerate() {
            let row = cell as u32 / SIDE;
            let col = cell as u32 % SIDE;
            let at = row * stride + col;
            assert_eq!(
                triangle_pair,
                [
                    at,
                    at + 1,
                    at + stride,
                    at + 1,
                    at + stride + 1,
                    at + stride
                ]
                .map(|index| index as u16),
            );
        }
        assert_eq!(
            u32::from(*indices.iter().max().unwrap()),
            stride * stride - 1
        );
    }

    #[test]
    fn hints_select_only_exact_cells_and_retain_complete_fallback() {
        assert!(WGSL.contains("let found = type2_cell(ray, row, col);"));
        assert!(WGSL.contains("if found.covered > 0.5 { return found; }"));
        assert!(WGSL.contains("return type2_mesh(body);"));
        assert!(!WGSL.contains("textureSample"));
        assert!(!WGSL.contains("out.packed"));
        assert!(!WGSL.contains("out.alpha"));
        assert!(WGSL.contains("return corrected_color(type2_gamma_color(map).rgb, body);"));
    }
}
