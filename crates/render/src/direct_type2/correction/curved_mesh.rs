//! Native-triangle rasterization for finite curved views.
//!
//! Subdivision follows the original triangle diagonal, positions and packed
//! lens coordinates. Curved projection between subdivision vertices is an
//! explicit approximation; original lens textures retain their full detail.
//! The original ray renderer remains the diagnostic/rear-view reference.

const SUBDIVISIONS: u32 = 4;
const ROWS: u32 = 50 * SUBDIVISIONS;
const COLS: u32 = 100 * SUBDIVISIONS;

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
                label: Some("curved-view native triangle subdivisions"),
                contents: &bytes,
                usage: wgpu::BufferUsages::INDEX,
            }),
            count: indices.len() as u32,
        }
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.count, 0, 0..1);
    }
}

fn indices() -> Vec<u32> {
    let stride = COLS + 1;
    let mut indices = Vec::with_capacity((ROWS * COLS * 6) as usize);
    for row in 0..ROWS {
        for col in 0..COLS {
            let at = row * stride + col;
            indices.extend([
                at,
                at + 1,
                at + stride,
                at + 1,
                at + stride + 1,
                at + stride,
            ]);
        }
    }
    indices
}

pub(super) fn shader_source() -> String {
    format!(
        "const CURVED_ROWS: u32 = {ROWS}u;\nconst CURVED_COLS: u32 = {COLS}u;\nconst CURVED_SUBDIVISIONS: u32 = {SUBDIVISIONS}u;\n{WGSL}"
    )
}

const WGSL: &str = r#"
struct CurvedMeshOut {
  @builtin(position) position: vec4<f32>,
  @location(0) packed: vec4<f32>,
  @location(1) map_uv: vec2<f32>,
  @location(2) fusion_uv: vec2<f32>,
  @location(3) body: vec3<f32>,
  @location(4) frontness: f32,
};

@vertex
fn corrected_curved_mesh_vs(@builtin(vertex_index) index: u32) -> CurvedMeshOut {
  let row = index / (CURVED_COLS + 1u);
  let col = index % (CURVED_COLS + 1u);
  let native_row = i32(min(row / CURVED_SUBDIVISIONS, 49u));
  let native_col = i32(min(col / CURVED_SUBDIVISIONS, 99u));
  let local = vec2<f32>(
    f32(row - u32(native_row) * CURVED_SUBDIVISIONS),
    f32(col - u32(native_col) * CURVED_SUBDIVISIONS)
  ) / f32(CURVED_SUBDIVISIONS);
  var nodes = array<vec2<i32>, 3>(
    vec2<i32>(native_row, native_col),
    vec2<i32>(native_row, native_col + 1),
    vec2<i32>(native_row + 1, native_col)
  );
  var weights = vec3<f32>(1.0 - local.x - local.y, local.y, local.x);
  if local.x + local.y > 1.0 {
    nodes[0] = vec2<i32>(native_row, native_col + 1);
    nodes[1] = vec2<i32>(native_row + 1, native_col + 1);
    nodes[2] = vec2<i32>(native_row + 1, native_col);
    weights = vec3<f32>(1.0 - local.x, local.x + local.y - 1.0, 1.0 - local.y);
  }
  let a = type2_cached_vertex(nodes[0].x, nodes[0].y);
  let b = type2_cached_vertex(nodes[1].x, nodes[1].y);
  let c = type2_cached_vertex(nodes[2].x, nodes[2].y);
  let sphere = weights.x * a.position.xyz + weights.y * b.position.xyz + weights.z * c.position.xyz;
  let body = vec3<f32>(-sphere.x, sphere.y, -sphere.z);
  let view = transpose(reframe.view_to_body) * body;
  let radial = length(view.xy);
  let corner_radius = length(vec2<f32>(reframe.screen.half_extent,
    reframe.screen.half_extent / reframe.screen.aspect));
  let corner_angle = atan(reframe.screen.shrink * corner_radius) / reframe.screen.shrink;
  let domain_angle = TYPE2_PI * 0.5 / reframe.screen.shrink;
  let clip_angle = (corner_angle + domain_angle) * 0.5;
  let angle = min(atan2(radial, view.z), clip_angle);
  let radius = tan(reframe.screen.shrink * angle) / reframe.screen.shrink;
  let direction = select(vec2<f32>(1.0, 0.0), view.xy / max(radial, 1.0e-20), radial > 0.0);
  let ndc = vec2<f32>(direction.x, -direction.y * reframe.screen.aspect)
    * radius / reframe.screen.half_extent;
  var out: CurvedMeshOut;
  // Curved projection has no perspective w. Clip hidden cells to a finite
  // rim outside the visible cone instead of folding them through negative w
  // or letting tan cross the projection singularity. Screen admission leaves
  // a whole native cell between that rim and every visible ray.
  out.position = vec4<f32>(ndc, 0.5, 1.0);
  out.packed = weights.x * a.packed + weights.y * b.packed + weights.z * c.packed;
  out.map_uv = weights.x * type2_varying(nodes[0].x, nodes[0].y)
    + weights.y * type2_varying(nodes[1].x, nodes[1].y)
    + weights.z * type2_varying(nodes[2].x, nodes[2].y);
  out.fusion_uv = weights.x * type2_fusion_varying(nodes[0].x, nodes[0].y)
    + weights.y * type2_fusion_varying(nodes[1].x, nodes[1].y)
    + weights.z * type2_fusion_varying(nodes[2].x, nodes[2].y);
  out.body = body;
  out.frontness = view.z - cos(clip_angle) * length(body);
  return out;
}

@fragment
fn corrected_curved_mesh_fs(in: CurvedMeshOut) -> @location(0) vec4<f32> {
  if in.frontness <= 0.0 { discard; }
  let map = Type2Sample(in.packed, type2_sample1(in.map_uv), 1.0, in.fusion_uv);
  return corrected_color(type2_gamma_color(map).rgb, in.body);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sphere_grid_has_shared_vertices_and_no_cross_row_triangles() {
        let indices = indices();
        assert_eq!(indices.len(), (ROWS * COLS * 6) as usize);
        for (cell, pair) in indices.chunks_exact(6).enumerate() {
            let row = cell as u32 / COLS;
            let col = cell as u32 % COLS;
            let at = row * (COLS + 1) + col;
            assert_eq!(
                pair,
                [
                    at,
                    at + 1,
                    at + COLS + 1,
                    at + 1,
                    at + COLS + 2,
                    at + COLS + 1
                ]
            );
        }
        assert_eq!(*indices.iter().max().unwrap(), (ROWS + 1) * (COLS + 1) - 1);
    }

    #[test]
    fn fragment_samples_original_planes_without_per_pixel_ray_intersection() {
        assert!(
            WGSL.contains("Type2Sample(in.packed, type2_sample1(in.map_uv), 1.0, in.fusion_uv)")
        );
        assert!(WGSL.contains("corrected_color(type2_gamma_color(map).rgb, in.body)"));
        assert!(WGSL.contains("if local.x + local.y > 1.0"));
        assert!(!WGSL.contains("type2_mesh("));
        assert!(!WGSL.contains("view_ray("));
        assert!(!WGSL.contains("textureSample"));
        assert!(WGSL.contains("out.position = vec4<f32>(ndc, 0.5, 1.0);"));
        assert!(WGSL.contains("min(atan2(radial, view.z), clip_angle)"));
        assert!(WGSL.contains("out.frontness = view.z - cos(clip_angle) * length(body);"));
        assert!(WGSL.contains("if in.frontness <= 0.0 { discard; }"));
    }
}
