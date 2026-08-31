//! CPU semantic oracle for Studio's captured type-2 sphere maps.
//!
//! This is instrument-only. It consumes authenticated payloads supplied by a
//! caller, rasterizes the READ 100-by-50 sphere under Kjerag's central
//! perspective view, and produces one dense record per output pixel. The CPU
//! ray/triangle selection below is disclosed Kjerag hybrid raster semantics,
//! not a claim about Metal's edge fill rules. It does not select playback
//! behavior or claim Studio output-pixel parity.

use std::f32::consts::{PI, TAU};

use kjerag_media::Size;

use crate::Reframe;

pub const MAP_WIDTH: usize = 200;
pub const MAP_HEIGHT: usize = 100;
pub const SLICES: usize = 100;
pub const STACKS: usize = 50;
pub const VERTICES: usize = (SLICES + 1) * (STACKS + 1);
pub const INDICES: usize = SLICES * STACKS * 6;
pub const PACKED_FLOATS: usize = MAP_WIDTH * MAP_HEIGHT * 4;
pub const ALPHA_FLOATS: usize = MAP_WIDTH * MAP_HEIGHT;

/// Captured Studio 6.0.2 `TextureParam` values for the selected type-2 draw.
///
/// These are bit anchors, not tuning knobs. The selected compiled helper uses
/// `1.1` (`0x3f8ccccd`) as its fast-path threshold. A bundled generic shader
/// text containing `2.0` is not the selected helper and is intentionally not
/// represented here.
pub const TYPE2_BOX_FAST_THRESHOLD: f32 = f32::from_bits(0x3f8c_cccd);
pub const TYPE2_BOX_SIZE: f32 = f32::from_bits(0x3fe4_e2f9);
pub const TYPE2_BOX_FALLBACK_AREA: f32 = f32::from_bits(0x3a83_126f);
pub const TYPE2_CHROMA_OFFSET: f32 = f32::from_bits(0x3f00_8081);
pub const TYPE2_LUMA_SIZE: [f32; 2] = [2880.0, 2880.0];
pub const TYPE2_CHROMA_SIZE: [f32; 2] = [1440.0, 1440.0];
pub const TYPE2_R_CR: f32 = f32::from_bits(0x3fb3_74bc);
pub const TYPE2_G_CB: f32 = f32::from_bits(0xbeb0_20c5);
pub const TYPE2_G_CR: f32 = f32::from_bits(0xbf36_c8b4);
pub const TYPE2_B_CB: f32 = f32::from_bits(0x3fe2_d0e5);

/// One clipped two-texel cell in the selected `boxSampling` area walk.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Type2BoxCell {
    /// Normalized coordinate of the clipped cell's midpoint.
    pub uv: [f32; 2],
    /// Clipped area in logical source texels.
    pub area: f32,
}

/// Build the readable selected `boxSampling` schedule.
///
/// The walk starts at `floor(center * size - box / 2)`, advances by two
/// logical texels on each axis, clips every cell to the box, and samples the
/// clipped midpoint. The returned order is row-major.
pub fn studio_type2_box_schedule(
    uv: [f32; 2],
    logical_size: [f32; 2],
    box_size: [f32; 2],
) -> Vec<Type2BoxCell> {
    let start: [f32; 2] =
        std::array::from_fn(|axis| uv[axis] * logical_size[axis] - box_size[axis] * 0.5);
    let end: [f32; 2] = std::array::from_fn(|axis| start[axis] + box_size[axis]);
    let mut cells = Vec::new();
    let mut cell_y = start[1].floor();
    while cell_y < end[1] {
        let low_y = cell_y.max(start[1]);
        let high_y = (cell_y + 2.0).min(end[1]);
        let mut cell_x = start[0].floor();
        while cell_x < end[0] {
            let low_x = cell_x.max(start[0]);
            let high_x = (cell_x + 2.0).min(end[0]);
            let area = (high_x - low_x) * (high_y - low_y);
            cells.push(Type2BoxCell {
                uv: [
                    (low_x + high_x) * 0.5 / logical_size[0],
                    (low_y + high_y) * 0.5 / logical_size[1],
                ],
                area,
            });
            cell_x += 2.0;
        }
        cell_y += 2.0;
    }
    cells
}

/// Readable selected `boxSampling` semantics over a normalized-linear source.
pub fn studio_type2_box_sample(
    uv: [f32; 2],
    logical_size: [f32; 2],
    box_size: [f32; 2],
    mut sample: impl FnMut([f32; 2]) -> [f32; 4],
) -> [f32; 4] {
    if box_size[0] <= TYPE2_BOX_FAST_THRESHOLD && box_size[1] <= TYPE2_BOX_FAST_THRESHOLD {
        return sample(uv);
    }
    let cells = studio_type2_box_schedule(uv, logical_size, box_size);
    let area: f32 = cells.iter().map(|cell| cell.area).sum();
    if area <= TYPE2_BOX_FALLBACK_AREA {
        return sample(uv);
    }
    let mut sum = [0.0; 4];
    for cell in cells {
        let value = sample(cell.uv);
        for channel in 0..4 {
            sum[channel] += value[channel] * cell.area;
        }
    }
    sum.map(|channel| channel / area)
}

/// Normalized linear sampling of the virtual A-then-B aggregate atlas.
///
/// Clamping happens against the full `2 * lens_width` atlas before each tap
/// is routed to a physical lens. The internal split is therefore crossable.
pub fn studio_type2_atlas_linear(
    a: &[[f32; 4]],
    b: &[[f32; 4]],
    lens_size: [usize; 2],
    uv: [f32; 2],
) -> [f32; 4] {
    let [width, height] = lens_size;
    assert!(
        width > 0 && height > 0,
        "aggregate lenses must not be empty"
    );
    assert_eq!(a.len(), width * height);
    assert_eq!(b.len(), width * height);
    let p = [
        uv[0] * (2 * width) as f32 - 0.5,
        uv[1] * height as f32 - 0.5,
    ];
    let floor = [p[0].floor(), p[1].floor()];
    let base = [floor[0] as i32, floor[1] as i32];
    let fraction = [p[0] - floor[0], p[1] - floor[1]];
    let load = |x: i32, y: i32| {
        let x = x.clamp(0, (2 * width - 1) as i32) as usize;
        let y = y.clamp(0, (height - 1) as i32) as usize;
        if x < width {
            a[y * width + x]
        } else {
            b[y * width + x - width]
        }
    };
    let aa = load(base[0], base[1]);
    let ab = load(base[0] + 1, base[1]);
    let ba = load(base[0], base[1] + 1);
    let bb = load(base[0] + 1, base[1] + 1);
    std::array::from_fn(|channel| {
        let top = aa[channel] + (ab[channel] - aa[channel]) * fraction[0];
        let bottom = ba[channel] + (bb[channel] - ba[channel]) * fraction[0];
        top + (bottom - top) * fraction[1]
    })
}

/// Selected type-2 full-range Y/Cb/Cr matrix, with the captured `128/255`
/// chroma offset. This is deliberately separate from Kjerag's production
/// range and BT.709 conversion.
pub fn studio_type2_yuv(y: f32, uv: [f32; 2]) -> [f32; 3] {
    let cb = uv[0] - TYPE2_CHROMA_OFFSET;
    let cr = uv[1] - TYPE2_CHROMA_OFFSET;
    [
        y + TYPE2_R_CR * cr,
        y + TYPE2_G_CB * cb + TYPE2_G_CR * cr,
        y + TYPE2_B_CB * cb,
    ]
}

/// Whole selected type-2 NV12 source law over readable normalized-linear
/// aggregate samplers. Both planes receive the identical normalized map
/// coordinate; their logical sizes alone differ.
pub fn studio_type2_source_sample(
    uv: [f32; 2],
    luma_sample: impl FnMut([f32; 2]) -> [f32; 4],
    chroma_sample: impl FnMut([f32; 2]) -> [f32; 4],
) -> [f32; 3] {
    let box_size = [TYPE2_BOX_SIZE; 2];
    let luma = studio_type2_box_sample(uv, TYPE2_LUMA_SIZE, box_size, luma_sample);
    let chroma = studio_type2_box_sample(uv, TYPE2_CHROMA_SIZE, box_size, chroma_sample);
    studio_type2_yuv(luma[0], [chroma[0], chroma[1]])
}

/// One captured packed UV resource and its selected left-alpha resource.
#[derive(Clone, Debug, PartialEq)]
pub struct CapturedMap {
    pub packed: Vec<[f32; 4]>,
    pub alpha: Vec<f32>,
}

impl CapturedMap {
    pub fn new(packed: Vec<[f32; 4]>, alpha: Vec<f32>) -> Result<Self, &'static str> {
        if packed.len() != MAP_WIDTH * MAP_HEIGHT {
            return Err("packed type-2 map must contain 200 by 100 float4 nodes");
        }
        if alpha.len() != MAP_WIDTH * MAP_HEIGHT {
            return Err("type-2 alpha map must contain 200 by 100 floats");
        }
        Ok(Self { packed, alpha })
    }

    /// Apply local-source-UV deltas while preserving every authenticated
    /// V5/V6 corpus sentinel pair.
    pub fn with_delta(&self, delta: &[[[f32; 2]; 2]]) -> Self {
        assert_eq!(delta.len(), self.packed.len());
        let mut out = self.clone();
        for (node, (packed, delta)) in out.packed.iter_mut().zip(delta).enumerate() {
            for (lens, lens_delta) in delta.iter().enumerate() {
                if corpus_sentinel_pair(self.packed[node], lens) {
                    continue;
                }
                let mut uv = local_uv(*packed, lens);
                uv[0] += lens_delta[0];
                uv[1] += lens_delta[1];
                set_local_uv(packed, lens, uv);
            }
        }
        out
    }

    /// Native ON minus OFF in each lens's local source coordinate system.
    /// Nodes not eligible under the authenticated corpus sentinel set in both
    /// maps receive a literal zero delta.
    pub fn delta_to(&self, on: &Self) -> Vec<[[f32; 2]; 2]> {
        assert_eq!(self.packed.len(), on.packed.len());
        self.packed
            .iter()
            .zip(&on.packed)
            .map(|(&off, &on)| {
                std::array::from_fn(|lens| {
                    if !corpus_sentinel_pair(off, lens) && !corpus_sentinel_pair(on, lens) {
                        let a = local_uv(off, lens);
                        let b = local_uv(on, lens);
                        [b[0] - a[0], b[1] - a[1]]
                    } else {
                        [0.0; 2]
                    }
                })
            })
            .collect()
    }

    pub fn negated_delta_arm(&self, on: &Self) -> Self {
        let delta = self
            .delta_to(on)
            .into_iter()
            .map(|pair| pair.map(|d| [-d[0], -d[1]]))
            .collect::<Vec<_>>();
        self.with_delta(&delta)
    }

    /// Exchange local-UV deltas only where neither source nor destination lens
    /// pair is in the authenticated corpus sentinel set. Everywhere else the
    /// delta is zero.
    pub fn swapped_delta_arm(&self, on: &Self) -> Self {
        let native = self.delta_to(on);
        let swapped = self
            .packed
            .iter()
            .zip(&on.packed)
            .zip(native)
            .map(|((&off, &on), delta)| {
                if (0..2)
                    .all(|lens| !corpus_sentinel_pair(off, lens) && !corpus_sentinel_pair(on, lens))
                {
                    [delta[1], delta[0]]
                } else {
                    [[0.0; 2]; 2]
                }
            })
            .collect::<Vec<_>>();
        self.with_delta(&swapped)
    }

    pub fn rasterize(&self, reframe: &Reframe, size: Size) -> DenseMap {
        let mesh = Mesh::new(self);
        let mut pixels = Vec::with_capacity(size.width as usize * size.height as usize);
        for y in 0..size.height {
            for x in 0..size.width {
                let uv = [
                    (x as f32 + 0.5) / size.width as f32,
                    (y as f32 + 0.5) / size.height as f32,
                ];
                let Some(view) = reframe.view_ray(uv) else {
                    pixels.push(DensePixel::EMPTY);
                    continue;
                };
                let body = normalize(reframe.body_ray(view));
                pixels.push(mesh.sample(mesh_ray(body), &self.alpha));
            }
        }
        let uncovered = pixels.iter().filter(|pixel| pixel.covered == 0.0).count();
        DenseMap {
            size,
            pixels,
            uncovered,
        }
    }
}

/// GPU upload record. Coordinates remain in the captured aggregate atlas;
/// authentic OFF/ON never take a cancellation-prone local-UV round trip.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DensePixel {
    pub packed_uv: [[f32; 2]; 2],
    /// Effective selected colour weight. The selected fisheye-alpha resource
    /// is the statically READ neutral 1x1 constant one, so the authenticated
    /// final left-alpha raster reaches the mix unchanged.
    pub alpha: f32,
    pub covered: f32,
    _pad: [f32; 2],
}

impl DensePixel {
    pub const EMPTY: Self = Self {
        packed_uv: [[0.0; 2]; 2],
        alpha: 0.0,
        covered: 0.0,
        _pad: [0.0; 2],
    };

    #[cfg(test)]
    pub(crate) fn from_test_lanes(lane: &[f32]) -> Self {
        Self {
            packed_uv: [[lane[0], lane[1]], [lane[2], lane[3]]],
            alpha: lane[4],
            covered: lane[5],
            _pad: [lane[6], lane[7]],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DenseMap {
    pub size: Size,
    pub pixels: Vec<DensePixel>,
    /// Pixel centres that did not intersect an exactly admitted triangle.
    /// The instrument refuses rather than filling these with a tolerance or
    /// extrapolated attributes.
    pub uncovered: usize,
}

impl DenseMap {
    pub const FLOATS_PER_PIXEL: usize = 8;

    pub fn bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                self.pixels.as_ptr().cast::<u8>(),
                self.pixels.len() * size_of::<DensePixel>(),
            )
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Vertex {
    position: [f32; 3],
    varying_uv: [f32; 2],
    packed: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct Mesh {
    vertices: Vec<Vertex>,
    indices: Vec<u16>,
}

impl Mesh {
    pub fn new(map: &CapturedMap) -> Self {
        let mut vertices = Vec::with_capacity(VERTICES);
        for r in 0..=STACKS {
            let phi = PI * r as f32 / STACKS as f32;
            let (sin_phi, cos_phi) = phi.sin_cos();
            for c in 0..=SLICES {
                let theta = TAU * c as f32 / SLICES as f32;
                let (sin_theta, cos_theta) = theta.sin_cos();
                // Corrected selected vertex order: map adjustment precedes
                // both packed-UV sampling and assignment to v_uv.
                let varying_uv = [
                    (c as f32 / SLICES as f32 + 0.5) + 0.5 / MAP_WIDTH as f32,
                    ((r as f32 / STACKS as f32 * 99.0) + 0.5) / MAP_HEIGHT as f32,
                ];
                vertices.push(Vertex {
                    position: [-sin_phi * sin_theta, cos_phi, sin_phi * cos_theta],
                    varying_uv,
                    packed: sample4(&map.packed, varying_uv),
                });
            }
        }
        let mut indices = Vec::with_capacity(INDICES);
        for r in 0..STACKS {
            for c in 0..SLICES {
                let a = (r * (SLICES + 1) + c) as u16;
                indices.extend_from_slice(&[a, a + 1, a + 101, a + 1, a + 102, a + 101]);
            }
        }
        Self { vertices, indices }
    }

    fn sample(&self, ray: [f32; 3], alpha: &[f32]) -> DensePixel {
        let (row, col) = mesh_cell(ray);
        // The latitude/longitude inverse locates the spherical grid cell, but
        // a central ray meets the planar triangle mesh and its projected cell
        // boundary can lie in an adjacent spherical cell. Search the located
        // cell first, then its eight immediate neighbours. Admission remains
        // exact f64 nonnegative barycentrics: no tolerance or extrapolation.
        for (dr, dc) in [
            (0, 0),
            (-1, -1),
            (-1, 0),
            (-1, 1),
            (0, -1),
            (0, 1),
            (1, -1),
            (1, 0),
            (1, 1),
        ] {
            let candidate_row = row as isize + dr;
            if !(0..STACKS as isize).contains(&candidate_row) {
                continue;
            }
            let candidate_col = (col as isize + dc).rem_euclid(SLICES as isize) as usize;
            if let Some(pixel) = self.sample_cell(ray, alpha, candidate_row as usize, candidate_col)
            {
                return pixel;
            }
        }
        DensePixel::EMPTY
    }

    fn sample_cell(
        &self,
        ray: [f32; 3],
        alpha: &[f32],
        row: usize,
        col: usize,
    ) -> Option<DensePixel> {
        let base = 6 * (row * SLICES + col);
        for tri in 0..2 {
            let ids = &self.indices[base + tri * 3..base + tri * 3 + 3];
            let ids: [u16; 3] = ids.try_into().expect("a triangle has three indices");
            let v = ids.map(|index| self.vertices[index as usize]);
            if let Some(w) = ray_triangle(ray, v.map(|v| v.position)) {
                let varying_uv = interpolate(v.map(|v| v.varying_uv), w);
                let packed = interpolate4(v.map(|v| v.packed), w);
                return Some(DensePixel {
                    packed_uv: [[packed[0], packed[1]], [packed[2], packed[3]]],
                    alpha: sample1(alpha, varying_uv),
                    covered: 1.0,
                    _pad: [0.0; 2],
                });
            }
        }
        None
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }
    pub fn indices(&self) -> &[u16] {
        &self.indices
    }
    pub fn vertex_position(&self, r: usize, c: usize) -> [f32; 3] {
        self.vertices[r * (SLICES + 1) + c].position
    }
}

pub fn local_uv(packed: [f32; 4], lens: usize) -> [f32; 2] {
    match lens {
        0 => [2.0 * packed[0], packed[1]],
        1 => [2.0 * packed[2] - 1.0, packed[3]],
        _ => panic!("type-2 map has exactly two lenses"),
    }
}

pub fn packed_uv(local: [f32; 2], lens: usize) -> [f32; 2] {
    match lens {
        0 => [0.5 * local[0], local[1]],
        1 => [0.5 * (local[0] + 1.0), local[1]],
        _ => panic!("type-2 map has exactly two lenses"),
    }
}

fn set_local_uv(packed: &mut [f32; 4], lens: usize, local: [f32; 2]) {
    let uv = packed_uv(local, lens);
    packed[2 * lens] = uv[0];
    packed[2 * lens + 1] = uv[1];
}

/// Exact authenticated V5/V6 corpus sentinel set, used only for disclosed
/// decoy eligibility. V5 carries `(-1,-1)` for both lenses, while V6 carries
/// `(-.5,-1)` for A and `(0,-1)` for B; why their encodings differ is not
/// fully reconciled. The selected draw has no validity suppression.
fn corpus_sentinel_pair(packed: [f32; 4], lens: usize) -> bool {
    match lens {
        0 => (packed[0] == -1.0 || packed[0] == -0.5) && packed[1] == -1.0,
        1 => (packed[2] == -1.0 || packed[2] == 0.0) && packed[3] == -1.0,
        _ => panic!("type-2 map has exactly two lenses"),
    }
}

fn sample4(map: &[[f32; 4]], uv: [f32; 2]) -> [f32; 4] {
    bilinear(uv, |x, y| map[y * MAP_WIDTH + x])
}

fn sample1(map: &[f32], uv: [f32; 2]) -> f32 {
    bilinear(uv, |x, y| [map[y * MAP_WIDTH + x]; 4])[0]
}

fn bilinear(uv: [f32; 2], at: impl Fn(usize, usize) -> [f32; 4]) -> [f32; 4] {
    let p = [uv[0] * MAP_WIDTH as f32, uv[1] * MAP_HEIGHT as f32];
    let mut i = [0i32; 2];
    let mut w = [0.0; 2];
    for axis in 0..2 {
        let f = p[axis].fract();
        i[axis] = p[axis].floor() as i32;
        if f < 0.5 {
            i[axis] -= 1;
            w[axis] = 0.5 + f;
        } else {
            w[axis] = f - 0.5;
        }
    }
    let x = |x: i32| x.rem_euclid(MAP_WIDTH as i32) as usize;
    let y = |y: i32| y.clamp(0, MAP_HEIGHT as i32 - 1) as usize;
    let a = at(x(i[0]), y(i[1]));
    let b = at(x(i[0] + 1), y(i[1]));
    let c = at(x(i[0]), y(i[1] + 1));
    let d = at(x(i[0] + 1), y(i[1] + 1));
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * w[0];
        let bottom = c[k] + (d[k] - c[k]) * w[0];
        top + (bottom - top) * w[1]
    })
}

fn ray_triangle(ray: [f32; 3], triangle: [[f32; 3]; 3]) -> Option<[f32; 3]> {
    let ray = ray.map(f64::from);
    let triangle = triangle.map(|point| point.map(f64::from));
    let e0 = sub64(triangle[1], triangle[0]);
    let e1 = sub64(triangle[2], triangle[0]);
    let p = cross64(ray, e1);
    let det = dot64(e0, p);
    if det == 0.0 {
        return None;
    }
    let inv = 1.0 / det;
    let t = [-triangle[0][0], -triangle[0][1], -triangle[0][2]];
    let u = dot64(t, p) * inv;
    let q = cross64(t, e0);
    let v = dot64(ray, q) * inv;
    let distance = dot64(e1, q) * inv;
    let w = 1.0 - u - v;
    (distance > 0.0 && u >= 0.0 && v >= 0.0 && w >= 0.0).then_some([w as f32, u as f32, v as f32])
}

fn interpolate(values: [[f32; 2]; 3], w: [f32; 3]) -> [f32; 2] {
    std::array::from_fn(|k| (0..3).map(|i| values[i][k] * w[i]).sum())
}
fn interpolate4(values: [[f32; 4]; 3], w: [f32; 3]) -> [f32; 4] {
    std::array::from_fn(|k| (0..3).map(|i| values[i][k] * w[i]).sum())
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let n = dot(v, v).sqrt();
    v.map(|x| x / n)
}

/// READ adapter from Kjerag's body convention to the selected mesh axes.
fn mesh_ray(body: [f32; 3]) -> [f32; 3] {
    [-body[0], body[1], -body[2]]
}

fn mesh_cell(ray: [f32; 3]) -> (usize, usize) {
    let row = (ray[1].clamp(-1.0, 1.0).acos() * STACKS as f32 / PI)
        .floor()
        .clamp(0.0, (STACKS - 1) as f32) as usize;
    let theta = (-ray[0]).atan2(ray[2]).rem_euclid(TAU);
    let col = (theta * SLICES as f32 / TAU).floor() as usize % SLICES;
    (row, col)
}

fn sub64(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot64(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn cross64(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labelled_map() -> CapturedMap {
        CapturedMap::new(
            (0..MAP_HEIGHT)
                .flat_map(|r| (0..MAP_WIDTH).map(move |c| [c as f32, r as f32, 0.0, 0.0]))
                .collect(),
            vec![0.5; ALPHA_FLOATS],
        )
        .unwrap()
    }

    #[test]
    fn mesh_has_the_read_census_topology_and_poles() {
        let mesh = Mesh::new(&labelled_map());
        assert_eq!(mesh.vertex_count(), 5151);
        assert_eq!(mesh.indices().len(), 30000);
        assert_eq!(&mesh.indices()[..6], &[0, 1, 101, 1, 102, 101]);
        assert_eq!(mesh.vertex_position(0, 0), [0.0, 1.0, 0.0]);
        assert!(mesh.vertex_position(50, 37)[1] < -0.99999);
    }

    #[test]
    fn corrected_adjusted_vertex_uv_samples_the_expected_nodes() {
        let map = labelled_map();
        let mesh = Mesh::new(&map);
        // p.x = 2*c + 100.5: c=0 is centred on node 100, not halfway 99/100.
        assert_eq!(mesh.vertices[0].packed[0], 100.0);
        // p.y = 1.98*r + .5: r=25 lies at 50 exactly, halfway nodes 49/50.
        let centre = mesh.vertices[25 * 101].packed;
        assert_eq!(centre[0], 100.0);
        assert_eq!(centre[1], 49.5);
        // c=100 repeats to the same horizontal sample as c=0.
        assert_eq!(mesh.vertices[100].packed, mesh.vertices[0].packed);
    }

    #[test]
    fn sampler_repeats_x_clamps_y_and_interpolates_interior() {
        let map = labelled_map();
        assert_eq!(sample4(&map.packed, [0.0, 0.0])[0], 99.5);
        assert_eq!(sample4(&map.packed, [1.0, 0.0])[0], 99.5);
        assert_eq!(sample4(&map.packed, [0.5 / 200.0, -10.0])[1], 0.0);
        assert_eq!(sample4(&map.packed, [1.5 / 200.0, 1.5 / 100.0])[0], 1.0);
    }

    #[test]
    fn inverse_atlas_adapter_orders_lens_a_then_b() {
        let packed = [0.25, 0.3, 0.75, 0.8];
        assert_eq!(local_uv(packed, 0), [0.5, 0.3]);
        assert_eq!(local_uv(packed, 1), [0.5, 0.8]);
        assert_eq!(packed_uv(local_uv(packed, 0), 0), [0.25, 0.3]);
        assert_eq!(packed_uv(local_uv(packed, 1), 1), [0.75, 0.8]);
    }

    #[test]
    fn arms_preserve_invalid_base_and_zero_is_byte_identical() {
        let mut off = labelled_map();
        off.packed[0] = [-0.5, -1.0, 0.7, 0.8];
        let mut on = off.clone();
        on.packed[0] = [0.9, -1.0, 0.75, 0.85];
        on.packed[1] = [1.0, 0.2, 0.8, 0.4];
        let zero = off.with_delta(&vec![[[0.0; 2]; 2]; off.packed.len()]);
        assert_eq!(zero.packed, off.packed);
        let negated = off.negated_delta_arm(&on);
        assert_eq!(&negated.packed[0][..2], &off.packed[0][..2]);
        let swapped = off.swapped_delta_arm(&on);
        assert_eq!(swapped.packed[0], off.packed[0]);
    }

    #[test]
    fn negative_y_is_not_an_invented_validity_gate() {
        let packed = [0.2, -0.25, 0.7, -0.5];
        assert!(!corpus_sentinel_pair(packed, 0));
        assert!(!corpus_sentinel_pair(packed, 1));
    }

    #[test]
    fn ray_triangle_returns_central_perspective_barycentrics() {
        let triangle = [[0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0]];
        let w = ray_triangle(normalize([0.25, 0.25, 1.0]), triangle).unwrap();
        for (got, want) in w.into_iter().zip([0.5, 0.25, 0.25]) {
            assert!((got - want).abs() < 1e-6);
        }
    }

    #[test]
    fn shared_triangle_edge_has_one_deterministic_sample() {
        let map = labelled_map();
        let mesh = Mesh::new(&map);
        let a = mesh.vertex_position(20, 20);
        let b = mesh.vertex_position(21, 21);
        let ray = normalize(std::array::from_fn(|i| a[i] + b[i]));
        assert_eq!(mesh.sample(ray, &map.alpha).covered, 1.0);
    }

    #[test]
    fn near_each_pole_survives_the_degenerate_fan_triangle() {
        let map = labelled_map();
        let mesh = Mesh::new(&map);
        for triangle in [[(0, 37), (1, 38), (1, 37)], [(49, 37), (49, 38), (50, 37)]] {
            let points = triangle.map(|(row, col)| mesh.vertex_position(row, col));
            let ray = normalize(std::array::from_fn(|axis| {
                points.iter().map(|point| point[axis]).sum()
            }));
            assert_eq!(mesh.sample(ray, &map.alpha).covered, 1.0);
        }
    }

    #[test]
    fn body_axes_take_the_read_mesh_half_turn_before_grid_lookup() {
        let map = labelled_map();
        let mesh = Mesh::new(&map);
        let forward = mesh_ray([0.0, 0.0, 1.0]);
        assert_eq!(mesh_cell(forward), (25, 50));
        assert_eq!(mesh.vertices[25 * 101 + 50].packed[0].round(), 0.0);
        let right = mesh_ray([1.0, 0.0, 0.0]);
        assert_eq!(mesh_cell(right), (25, 25));
        assert_eq!(mesh.vertices[25 * 101 + 25].packed[0].round(), 150.0);
    }

    #[test]
    fn selected_type2_texture_constants_keep_the_captured_bits() {
        assert_eq!(TYPE2_BOX_FAST_THRESHOLD.to_bits(), 0x3f8c_cccd);
        assert_eq!(TYPE2_BOX_SIZE.to_bits(), 0x3fe4_e2f9);
        assert_eq!(TYPE2_BOX_FALLBACK_AREA.to_bits(), 0x3a83_126f);
        assert_eq!(TYPE2_CHROMA_OFFSET.to_bits(), 0x3f00_8081);
        assert_eq!(TYPE2_R_CR.to_bits(), 0x3fb3_74bc);
        assert_eq!(TYPE2_G_CB.to_bits(), 0xbeb0_20c5);
        assert_eq!(TYPE2_G_CR.to_bits(), 0xbf36_c8b4);
        assert_eq!(TYPE2_B_CB.to_bits(), 0x3fe2_d0e5);
        assert_eq!(TYPE2_LUMA_SIZE, [2880.0, 2880.0]);
        assert_eq!(TYPE2_CHROMA_SIZE, [1440.0, 1440.0]);
    }

    #[test]
    fn selected_box_walk_clips_two_texel_cells_and_area_weights_them() {
        let box_size = [TYPE2_BOX_SIZE; 2];
        // Put each axis across a +2 cell boundary, yielding four clipped cells.
        let start = [3.8_f32, 5.6_f32];
        let logical_size = [8.0, 10.0];
        let uv =
            std::array::from_fn(|axis| (start[axis] + box_size[axis] * 0.5) / logical_size[axis]);
        let cells = studio_type2_box_schedule(uv, logical_size, box_size);
        assert_eq!(cells.len(), 4);
        let area: f32 = cells.iter().map(|cell| cell.area).sum();
        assert!((area - TYPE2_BOX_SIZE * TYPE2_BOX_SIZE).abs() < 2e-6);
        assert!(cells.iter().all(|cell| cell.area > 0.0));

        let got = studio_type2_box_sample(uv, logical_size, box_size, |at| {
            [at[0], at[1], at[0] + at[1], 1.0]
        });
        // An area average of a linear field over a symmetric box is its centre.
        assert!((got[0] - uv[0]).abs() < 1e-6);
        assert!((got[1] - uv[1]).abs() < 1e-6);
        assert!((got[2] - (uv[0] + uv[1])).abs() < 1e-6);
        assert!((got[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn selected_box_fast_gate_requires_both_axes_and_fallback_is_area_guarded() {
        let mut fast_calls = Vec::new();
        let uv = [0.25, 0.75];
        let fast = studio_type2_box_sample(uv, [8.0, 8.0], [TYPE2_BOX_FAST_THRESHOLD; 2], |at| {
            fast_calls.push(at);
            [7.0; 4]
        });
        assert_eq!(fast, [7.0; 4]);
        assert_eq!(fast_calls, vec![uv]);

        let mut fallback_calls = Vec::new();
        let fallback = studio_type2_box_sample(uv, [8.0, 8.0], [2.0, 0.0], |at| {
            fallback_calls.push(at);
            [9.0; 4]
        });
        assert_eq!(fallback, [9.0; 4]);
        assert_eq!(fallback_calls, vec![uv]);

        let mut active_calls = 0;
        let crossing_uv = [0.3125, uv[1]];
        studio_type2_box_sample(
            crossing_uv,
            [8.0, 8.0],
            [TYPE2_BOX_FAST_THRESHOLD + f32::EPSILON, 1.0],
            |_| {
                active_calls += 1;
                [1.0; 4]
            },
        );
        assert!(
            active_calls > 1,
            "one axis above 1.1 must enter the area walk"
        );
    }

    #[test]
    fn aggregate_atlas_matches_one_global_clamped_reference_across_the_split() {
        let a = [[0.0; 4], [2.0; 4]];
        let b = [[10.0; 4], [12.0; 4]];
        let joined = [a[0], a[1], b[0], b[1]];
        let reference = |uv: [f32; 2]| {
            let p = uv[0] * joined.len() as f32 - 0.5;
            let floor = p.floor();
            let base = floor as i32;
            let fraction = p - floor;
            let at = |x: i32| joined[x.clamp(0, joined.len() as i32 - 1) as usize];
            std::array::from_fn(|channel| {
                at(base)[channel] + (at(base + 1)[channel] - at(base)[channel]) * fraction
            })
        };
        for uv in [
            [-0.2, 0.5],
            [0.125, 0.5],
            [0.5, 0.5],
            [0.875, 0.5],
            [1.2, 0.5],
        ] {
            assert_eq!(studio_type2_atlas_linear(&a, &b, [2, 1], uv), reference(uv));
        }
        assert_eq!(
            studio_type2_atlas_linear(&a, &b, [2, 1], [0.5, 0.5]),
            [6.0; 4]
        );
    }

    #[test]
    fn nv12_uses_same_normalized_coordinate_and_half_sized_chroma_schedule() {
        let uv = [0.50017, 0.39981];
        let mut luma_calls = Vec::new();
        let mut chroma_calls = Vec::new();
        let _ = studio_type2_source_sample(
            uv,
            |at| {
                luma_calls.push(at);
                [0.4, 0.0, 0.0, 0.0]
            },
            |at| {
                chroma_calls.push(at);
                [TYPE2_CHROMA_OFFSET, TYPE2_CHROMA_OFFSET, 0.0, 0.0]
            },
        );
        let box_size = [TYPE2_BOX_SIZE; 2];
        let expected_luma = studio_type2_box_schedule(uv, TYPE2_LUMA_SIZE, box_size)
            .into_iter()
            .map(|cell| cell.uv)
            .collect::<Vec<_>>();
        let expected_chroma = studio_type2_box_schedule(uv, TYPE2_CHROMA_SIZE, box_size)
            .into_iter()
            .map(|cell| cell.uv)
            .collect::<Vec<_>>();
        assert_eq!(luma_calls, expected_luma);
        assert_eq!(chroma_calls, expected_chroma);
        assert_ne!(luma_calls, chroma_calls);
    }

    #[test]
    fn selected_yuv_matrix_has_neutral_and_cb_cr_basis_vectors() {
        let y = 0.25;
        assert_eq!(studio_type2_yuv(y, [TYPE2_CHROMA_OFFSET; 2]), [y, y, y]);
        let cb = studio_type2_yuv(y, [TYPE2_CHROMA_OFFSET + 1.0, TYPE2_CHROMA_OFFSET]);
        assert!((cb[0] - y).abs() < 1e-7);
        assert!((cb[1] - (y + TYPE2_G_CB)).abs() < 1e-7);
        assert!((cb[2] - (y + TYPE2_B_CB)).abs() < 1e-6);
        let cr = studio_type2_yuv(y, [TYPE2_CHROMA_OFFSET, TYPE2_CHROMA_OFFSET + 1.0]);
        assert!((cr[0] - (y + TYPE2_R_CR)).abs() < 1e-6);
        assert!((cr[1] - (y + TYPE2_G_CR)).abs() < 1e-7);
        assert!((cr[2] - y).abs() < 1e-7);
    }
}
