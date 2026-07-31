//! The Mei/UCM forward map: one output ray to one lens pixel, for each lens
//! of the camera, and the choice between them.
//!
//! It exists twice on purpose. `WGSL` below is the copy the GPU runs, once
//! per output pixel; [`Reframe::project`] is the same arithmetic in Rust, so
//! the model can be checked against known angles by `cargo test` on a box
//! with no GPU and no footage. The two read the same [`Reframe`] block, whose
//! binding is declared next to the shader source, so a field added on one
//! side has one obvious home on the other. `wgpu` checks the layouts agree:
//! the bind group declares `min_binding_size` from this type's size, and
//! pipeline creation rejects a shader whose struct wants more.
//!
//! Two lenses cover the sphere and overlap by about 15 degrees around the
//! seam, so most rays near it are in both pictures and the shader has to
//! choose. [`Reframe::pick`] is that choice: whichever lens has the ray at
//! all, and the one whose optical axis is nearer it where both do (issue
//! #27). A ray is dropped only where **no** lens has it.
//!
//! Written from the model description in `docs/research/insv-format.md` 5.1
//! (Mei and Rives 2007, as OpenCV's `cv::omnidir` states it). Nothing here
//! is transcribed from Gyroflow's `insta360.wgsl`, so this file is plain
//! AGPL-3.0 with no GPL header.

use kyerag_meta::{Intrinsics, Lens, Pose};

use super::{Camera, Size};

/// How many lenses one pass can sample.
///
/// Every camera in the format study is a back-to-back pair, and the two
/// bindings per lens are declared in WGSL rather than indexed, so this is a
/// constant rather than a length. A file that describes more lenses than
/// this has the rest ignored, which is a picture with a hole in it and not a
/// crash.
pub const MAX_LENSES: usize = 2;

/// The uniform block, mirrored field for field by `struct Reframe` in
/// `WGSL`. All `f32`: the calibration is `f64` on the way in and the
/// composition below is done in `f64`, but a GPU uniform is `f32` and there
/// is nothing here that 24 bits of mantissa cannot hold (the largest number
/// is a pixel coordinate under 4096).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Reframe {
    /// One per lens, always [`MAX_LENSES`] of them however many the camera
    /// has: a uniform block cannot change size between draws. The ones past
    /// `lens_count` are [`LensBlock::EMPTY`].
    lenses: [LensBlock; MAX_LENSES],
    tan_half_fov: f32,
    /// Output width over output height. The vertical field of view is
    /// whatever this leaves.
    aspect: f32,
    /// The delivered frame size, shared: the streams of one file decode at
    /// one size, which `scene::calibrated` checks against the trailer.
    frame_width: f32,
    frame_height: f32,
    /// How many of [`Self::lenses`] have a decoded stream behind them. The
    /// older cameras write one lens per file, and then this is 1 and the
    /// picture is one hemisphere, exactly as it was before issue #27.
    lens_count: f32,
    has_frame: f32,
    linearize: f32,
    elapsed: f32,
}

/// One lens's half of the block: the Mei/UCM model, and where the lens is
/// pointing after the camera's own rotation.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct LensBlock {
    /// A `mat3x3<f32>` as WGSL lays one out: three columns, each padded to
    /// 16 bytes. Takes a view-space ray to this lens's frame.
    view_to_lens: [[f32; 4]; 3],
    xi: f32,
    fx: f32,
    fy: f32,
    cx: f32,
    cy: f32,
    k1: f32,
    k2: f32,
    k3: f32,
    p1: f32,
    p2: f32,
    image_radius: f32,
    /// A uniform array's element stride rounds up to the element's 16-byte
    /// alignment. WGSL does that itself; `repr(C)` does not.
    _pad: f32,
}

/// Where a view ray lands in one lens's image, in delivered-frame pixels.
///
/// `inside` false means the ray missed this lens. Missing every lens is what
/// the shader paints [`OUTSIDE_GRAY`] for; missing one of two is ordinary,
/// and is most of what [`Reframe::pick`] is deciding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    pub pixel: [f32; 2],
    pub inside: bool,
    /// The cosine of the angle between the ray and this lens's optical axis,
    /// which is what "nearest axis" compares. 1 is straight down the axis
    /// and 0 is the seam great circle.
    pub axis: f32,
}

/// The lens a ray is shown from, and where in its frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    pub lens: usize,
    pub landing: Landing,
}

/// What the shader paints where no lens has a picture. Neutral and dark,
/// in the same gamma-encoded space as the video, so the sRGB branch treats
/// it the same way it treats a sampled pixel.
pub const OUTSIDE_GRAY: f32 = 0.10;

impl Reframe {
    /// The block for one camera pose and the lenses of one file, in file
    /// order. Anything past [`MAX_LENSES`] is dropped.
    pub fn new(lenses: &[Lens], frame: Size, camera: Camera, aspect: f32, linearize: bool) -> Self {
        Self {
            lenses: std::array::from_fn(|index| match lenses.get(index) {
                Some(lens) => LensBlock::new(lens, index, frame, camera),
                None => LensBlock::EMPTY,
            }),
            tan_half_fov: (camera.fov * 0.5).tan(),
            aspect,
            frame_width: frame.width as f32,
            frame_height: frame.height as f32,
            lens_count: lenses.len().min(MAX_LENSES) as f32,
            has_frame: 1.0,
            linearize: f32::from(u8::from(linearize)),
            elapsed: 0.0,
        }
    }

    /// No file open: the shader draws its bring-up gradient. One lens that
    /// has no picture in it, so the map still runs and every ray still misses.
    pub fn gradient(elapsed: f32, aspect: f32, linearize: bool) -> Self {
        Self {
            lenses: [LensBlock::EMPTY; MAX_LENSES],
            tan_half_fov: 1.0,
            aspect,
            frame_width: 1.0,
            frame_height: 1.0,
            lens_count: 1.0,
            has_frame: 0.0,
            linearize: f32::from(u8::from(linearize)),
            elapsed,
        }
    }

    /// The block as the GPU reads it. Every field is an `f32` and `repr(C)`
    /// packs them, so there are no padding bytes and no invalid patterns.
    pub fn bytes(&self) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                std::ptr::from_ref(self).cast::<u8>(),
                std::mem::size_of::<Self>(),
            )
        }
    }

    /// The ray a point in the output looks along, in view space: x right,
    /// y down, z forward. `uv` runs 0 to 1 across the output, y down.
    pub fn view_ray(&self, uv: [f32; 2]) -> [f32; 3] {
        view_ray(uv, self.tan_half_fov, self.aspect)
    }

    /// Which lens shows this ray, and where in its frame.
    ///
    /// Nearest axis wins, which puts the seam on the great circle equidistant
    /// from the two axes and hands each hemisphere the lens that sees it
    /// squarest. A lens that has the ray beats one that does not, so the
    /// pick falls back into the overlap where a lens runs out of coverage
    /// before the halfway line. `landing.inside` false means no lens had it.
    ///
    /// WGSL twin: `pick`.
    pub fn pick(&self, view_ray: [f32; 3]) -> Pick {
        let mut best = Pick {
            lens: 0,
            landing: self.project(0, view_ray),
        };
        for lens in 1..self.lens_count as usize {
            let landing = self.project(lens, view_ray);
            if wins(landing, best.landing) {
                best = Pick { lens, landing };
            }
        }
        best
    }

    /// The forward map: a view ray, through one lens's extrinsics and the
    /// Mei/UCM model, to a pixel of that lens's delivered frame.
    ///
    /// WGSL twin: `project`. The shader adds one line the mirror does not,
    /// turning the pixel into a texture coordinate (`frame_uv`).
    pub fn project(&self, lens: usize, view_ray: [f32; 3]) -> Landing {
        let lens = &self.lenses[lens];
        let p = normalize(lens.lens_ray(view_ray));

        // The mirror parameter is why a ray past 90 degrees off axis still
        // has a finite projection: it only needs `z + xi > 0`. On this
        // camera family xi is above 1, so the guard never fires; it is here
        // for a model where xi is smaller than 1.
        let denom = p[2] + lens.xi;
        let x = p[0] / denom;
        let y = p[1] / denom;

        let r2 = x * x + y * y;
        let radial = 1.0 + r2 * (lens.k1 + r2 * (lens.k2 + r2 * lens.k3));
        let xd = x * radial + 2.0 * lens.p1 * x * y + lens.p2 * (r2 + 2.0 * x * x);
        let yd = y * radial + 2.0 * lens.p2 * x * y + lens.p1 * (r2 + 2.0 * y * y);

        let offset = [lens.fx * xd, lens.fy * yd];
        // How far round the map can be believed, which is not as far as it
        // answers. The distance from the principal point grows with the angle
        // off the axis only up to `cos(theta) = -1/xi`; past that turning
        // point it comes back down, re-enters the image circle, and a ray
        // from behind the lens lands a second time on a pixel that belongs
        // to a ray in front of it. That second landing is issue #30's ghost,
        // a raw circular fisheye hanging behind the reframed view, and the
        // radius test cannot see it because the fold puts it well inside the
        // circle. Every lens needs it: with two of them the fold is a ghost
        // of the other hemisphere, printed over a picture that is otherwise
        // correct. Vacuous where xi is below 1: there is no turning point
        // there, the radius runs away to infinity instead, and `denom` is the
        // limit that binds.
        let injective = p[2] * lens.xi > -1.0;
        Landing {
            pixel: [offset[0] + lens.cx, offset[1] + lens.cy],
            inside: denom > 0.0 && injective && norm(offset) <= lens.image_radius,
            axis: p[2],
        }
    }
}

/// Whether `candidate` shows the ray better than the lens already held.
///
/// WGSL twin: `wins`.
fn wins(candidate: Landing, held: Landing) -> bool {
    match (candidate.inside, held.inside) {
        (true, false) => true,
        (false, true) => false,
        _ => candidate.axis > held.axis,
    }
}

impl LensBlock {
    /// A lens with no picture in it: `xi` of 1 keeps the denominator
    /// positive and a zero image radius puts every ray outside. What an
    /// unfilled slot holds, so that a stray index costs a grey pixel rather
    /// than a garbage sample.
    const EMPTY: Self = Self {
        view_to_lens: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
        xi: 1.0,
        fx: 1.0,
        fy: 1.0,
        cx: 0.0,
        cy: 0.0,
        k1: 0.0,
        k2: 0.0,
        k3: 0.0,
        p1: 0.0,
        p2: 0.0,
        image_radius: 0.0,
        _pad: 0.0,
    };

    fn new(lens: &Lens, index: usize, frame: Size, camera: Camera) -> Self {
        let Intrinsics { xi, fx, fy, cx, cy } = lens.intrinsics;
        let distortion = lens.distortion;
        Self {
            view_to_lens: view_to_lens(&lens.pose, index, camera).columns(),
            xi: xi as f32,
            fx: fx as f32,
            fy: fy as f32,
            cx: cx as f32,
            cy: cy as f32,
            k1: distortion.k1 as f32,
            k2: distortion.k2 as f32,
            k3: distortion.k3 as f32,
            p1: distortion.p1 as f32,
            p2: distortion.p2 as f32,
            image_radius: image_radius(&lens.intrinsics, frame) as f32,
            _pad: 0.0,
        }
    }

    fn lens_ray(&self, ray: [f32; 3]) -> [f32; 3] {
        let column = |c: usize| self.view_to_lens[c];
        std::array::from_fn(|row| {
            ray[0] * column(0)[row] + ray[1] * column(1)[row] + ray[2] * column(2)[row]
        })
    }
}

/// The ray a point of the output looks along, in view space: x right, y down,
/// z forward. `uv` runs 0 to 1 across the output, y down, and `aspect` is the
/// output's width over its height.
///
/// WGSL twin: `view_ray`.
pub(crate) fn view_ray(uv: [f32; 2], tan_half_fov: f32, aspect: f32) -> [f32; 3] {
    [
        (uv[0] * 2.0 - 1.0) * tan_half_fov,
        (uv[1] * 2.0 - 1.0) * tan_half_fov / aspect,
        1.0,
    ]
}

/// Where a view-space ray points in the world: the camera's own rotation,
/// with none of the lens's mounting.
///
/// The drag solve in `super::camera` inverts this, so it reads the
/// composition from here rather than assuming one.
pub(crate) fn world_ray(camera: Camera, ray: [f32; 3]) -> [f32; 3] {
    camera_rotation(camera).mul_vec(ray)
}

/// The rotation that takes a view-space ray to lens `index`'s frame.
///
/// Both halves are right-handed in the frame the projection uses: x right,
/// y down, z along the axis being pointed. Positive camera yaw turns right,
/// positive camera pitch looks up.
fn view_to_lens(pose: &Pose, index: usize, camera: Camera) -> Mat3 {
    lens_from_body(pose, index).mul(camera_rotation(camera))
}

/// Yaw about the world vertical, then pitch about the view's own horizontal.
/// Never roll: the horizon stays level, which is the whole reason a drag near
/// the pole has to give something up (issue #29).
fn camera_rotation(camera: Camera) -> Mat3 {
    Mat3::rot_y(camera.yaw as f64).mul(Mat3::rot_x(camera.pitch as f64))
}

/// The quarter turn between `offset_v3`'s roll and the delivered frame's own
/// vertical. Applying roll as the file writes it renders an X4 Air a quarter
/// turn on its side; subtracting this datum first renders it upright, and
/// renders a ONE X2 upright as well.
///
/// Measured 2026-07-31 by rendering all four candidate rotations of lens 0
/// against plumb references in real footage from both cameras. The method,
/// the references and the result table are in docs/research/insv-format.md
/// 4.8, which is also where the open question this closes was written down.
const ROLL_DATUM_DEG: f64 = -90.0;

/// The lens's own mounting, as `offset_v3` records it: roll about the optical
/// axis, then the sub-degree yaw and pitch, applied over the nominal
/// arrangement the lens is mounted in ([`opposed`]).
///
/// The **order** of the three angles is not settled, and neither camera can
/// settle it: yaw and pitch are 0.103 and 0.07 degrees on the X4 Air, and
/// near the axis the model's effective focal length is `fx / (1 + xi)` =
/// 1106 px/rad, so every ordering agrees to about 2 px. A camera with a
/// large yaw or pitch would tell them apart; none is known to exist.
fn lens_from_body(pose: &Pose, index: usize) -> Mat3 {
    Mat3::rot_z((pose.roll_deg + ROLL_DATUM_DEG).to_radians())
        .mul(Mat3::rot_y(pose.yaw_deg.to_radians()))
        .mul(Mat3::rot_x(pose.pitch_deg.to_radians()))
        .mul(opposed(index))
}

/// The nominal pose lens `index` is mounted in, which its extrinsics are a
/// residual against.
///
/// The back-to-back flip is **not** in the file: lens 1's recorded yaw is
/// 0.039 degrees, not 180, and applying the block as an absolute pose points
/// both lenses the same way (docs/research/insv-format.md 4.3). A half turn
/// about the body's vertical is what puts it back: it takes body-forward to
/// lens-backward and leaves body-down as lens-down, so the rear picture comes
/// out the same way up as the front one. The other half turn that points the
/// same way, about x, differs from it by exactly 180 degrees of roll, which is
/// a rear sensor mounted upside down; `roll` is what records that, and `roll`
/// is already applied.
///
/// It multiplies on the right, so the block's own angles are a residual in the
/// lens's own frame rather than in the body's. That is a choice and not a
/// rearrangement: the two orders differ by twice lens 1's roll residual, 1.85
/// degrees on the X4 Air fixture. Measured against pixels 2026-07-31, the way
/// the roll datum was: both orders and the half turn about x rendered across
/// the seam on real footage, and the far-field content correlated between the
/// two lenses' pictures of it. This order leaves 0.4 degrees of the seam
/// unaligned along its own circle, the other 1.5, and the turn about x
/// correlates with nothing. Method and numbers: docs/research/insv-format.md
/// 4.9.
fn opposed(index: usize) -> Mat3 {
    match index {
        0 => Mat3::IDENTITY,
        _ => Mat3::rot_y(std::f64::consts::PI),
    }
}

/// The largest circle centred on the principal point that fits in the
/// delivered frame.
///
/// The file records no image-circle radius and the model does not bound
/// itself: past the lens's real coverage the radial polynomial keeps
/// returning finite pixel coordinates, so something has to say where the
/// picture stops. On the X4 Air fixture this radius is 1913 px, which the
/// model reaches at about 97.5 degrees off axis, so two lenses overlap by
/// about 15 degrees around the seam and the seam blend (issue #7) is what
/// will want that band.
fn image_radius(intrinsics: &Intrinsics, frame: Size) -> f64 {
    let (width, height) = (f64::from(frame.width), f64::from(frame.height));
    intrinsics
        .cx
        .min(intrinsics.cy)
        .min(width - intrinsics.cx)
        .min(height - intrinsics.cy)
        .max(0.0)
}

fn norm(v: [f32; 2]) -> f32 {
    v[0].hypot(v[1])
}

pub(crate) fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|component| component / length)
}

/// A 3x3 rotation, row major: `m[row][column]`, and `v_out = M * v_in`.
#[derive(Clone, Copy, Debug)]
struct Mat3([[f64; 3]; 3]);

impl Mat3 {
    const IDENTITY: Self = Self([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    fn rot_x(angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        Self([[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]])
    }

    fn rot_y(angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        Self([[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]])
    }

    fn rot_z(angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        Self([[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]])
    }

    fn mul_vec(self, v: [f32; 3]) -> [f32; 3] {
        let v = v.map(f64::from);
        std::array::from_fn(|row| (0..3).map(|k| self.0[row][k] * v[k]).sum::<f64>() as f32)
    }

    fn mul(self, rhs: Self) -> Self {
        let mut out = [[0.0; 3]; 3];
        for (r, row) in out.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| self.0[r][k] * rhs.0[k][c]).sum();
            }
        }
        Self(out)
    }

    /// WGSL stores a `mat3x3<f32>` as three columns, each padded to a
    /// `vec4`.
    fn columns(self) -> [[f32; 4]; 3] {
        std::array::from_fn(|c| {
            [
                self.0[0][c] as f32,
                self.0[1][c] as f32,
                self.0[2][c] as f32,
                0.0,
            ]
        })
    }
}

/// The half of the shader that mirrors this file, with the constants this
/// file owns written into it. `crates/render/src/scene.rs` concatenates the
/// result with the pass that samples NV12 and writes the target.
pub(crate) fn wgsl() -> String {
    // `{:?}` rather than `{}`: Rust's Display drops the decimal point on a
    // whole number, and `vec3<f32>(1)` is a type error in WGSL.
    format!(
        "const OUTSIDE_GRAY = vec3<f32>({OUTSIDE_GRAY:?});\nconst MAX_LENSES = {MAX_LENSES}u;\n{WGSL}"
    )
}

const WGSL: &str = r#"
struct LensBlock {
  view_to_lens: mat3x3<f32>,
  xi: f32,
  fx: f32,
  fy: f32,
  cx: f32,
  cy: f32,
  k1: f32,
  k2: f32,
  k3: f32,
  p1: f32,
  p2: f32,
  image_radius: f32,
};

struct Reframe {
  lenses: array<LensBlock, MAX_LENSES>,
  tan_half_fov: f32,
  aspect: f32,
  frame_width: f32,
  frame_height: f32,
  lens_count: f32,
  has_frame: f32,
  linearize: f32,
  elapsed: f32,
};

@group(0) @binding(0) var<uniform> reframe: Reframe;

struct Landing {
  pixel: vec2<f32>,
  inside: bool,
  axis: f32,
};

struct Pick {
  lens: u32,
  landing: Landing,
};

// x right, y down, z forward, matching the lens frame the model projects in.
fn view_ray(uv: vec2<f32>) -> vec3<f32> {
  let plane = (uv * 2.0 - vec2<f32>(1.0)) * reframe.tan_half_fov;
  return vec3<f32>(plane.x, plane.y / reframe.aspect, 1.0);
}

// Nearest axis wins, and a lens that has the ray beats one that does not.
// Rust twin: `Reframe::pick`.
fn pick(ray: vec3<f32>) -> Pick {
  var best = Pick(0u, project(0u, ray));
  for (var lens = 1u; f32(lens) < reframe.lens_count; lens += 1u) {
    let landing = project(lens, ray);
    if wins(landing, best.landing) {
      best = Pick(lens, landing);
    }
  }
  return best;
}

// Rust twin: `wins`.
fn wins(candidate: Landing, held: Landing) -> bool {
  if candidate.inside != held.inside {
    return candidate.inside;
  }
  return candidate.axis > held.axis;
}

// Mei/UCM forward map. Rust twin: `Reframe::project`.
fn project(index: u32, ray: vec3<f32>) -> Landing {
  let lens = reframe.lenses[index];
  let p = normalize(lens.view_to_lens * ray);
  let denom = p.z + lens.xi;
  let n = p.xy / denom;

  let r2 = dot(n, n);
  let radial = 1.0 + r2 * (lens.k1 + r2 * (lens.k2 + r2 * lens.k3));
  let tangential = vec2<f32>(
    2.0 * lens.p1 * n.x * n.y + lens.p2 * (r2 + 2.0 * n.x * n.x),
    2.0 * lens.p2 * n.x * n.y + lens.p1 * (r2 + 2.0 * n.y * n.y),
  );
  let d = n * radial + tangential;

  let offset = vec2<f32>(lens.fx * d.x, lens.fy * d.y);
  // Past `cos(theta) = -1/xi` the map folds and lands rays from behind this
  // lens back inside its image circle. Rust twin: `injective`.
  let injective = p.z * lens.xi > -1.0;
  var landing: Landing;
  landing.pixel = offset + vec2<f32>(lens.cx, lens.cy);
  landing.inside = denom > 0.0 && injective && length(offset) <= lens.image_radius;
  landing.axis = p.z;
  return landing;
}

// Pixel centres sit at integer coordinates in the camera model and at
// (i + 0.5) / size in a texture.
fn frame_uv(pixel: vec2<f32>) -> vec2<f32> {
  return (pixel + vec2<f32>(0.5)) / vec2<f32>(reframe.frame_width, reframe.frame_height);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use kyerag_meta::Distortion;

    const FRAME: Size = Size {
        width: 3840,
        height: 3840,
    };

    /// The X4 Air fixture in delivered-frame pixels: what `kyerag-meta`
    /// produces from `docs/research/x4air-calibration.json`, and what its own
    /// tests assert. Copied rather than parsed because the path from the
    /// fixture to a `CalibrationSet` runs through a private constructor in a
    /// crate this one only reads types from.
    fn fixture_lenses() -> Vec<Lens> {
        vec![
            Lens {
                intrinsics: Intrinsics {
                    xi: 2.31494,
                    fx: 3665.9397,
                    fy: 3667.4194,
                    cx: 1918.94,
                    cy: 1927.21,
                },
                distortion: Distortion {
                    k1: 0.95820886,
                    k2: -1.80141151,
                    k3: 3.57555127,
                    p1: -0.0007338,
                    p2: -0.00115458,
                },
                pose: Pose {
                    yaw_deg: -0.103,
                    pitch_deg: -0.07,
                    roll_deg: 90.534,
                    translation_m: [0.0, 0.0, 0.0],
                },
                lens_type: 131,
            },
            Lens {
                intrinsics: Intrinsics {
                    xi: 2.31494,
                    fx: 3671.9126,
                    fy: 3671.0823,
                    cx: 1935.35,
                    cy: 1935.09,
                },
                distortion: Distortion {
                    k1: 0.97158086,
                    k2: -2.08655882,
                    k3: 4.30578518,
                    p1: -0.0019249,
                    p2: 0.00054564,
                },
                pose: Pose {
                    yaw_deg: 0.039,
                    pitch_deg: -0.193,
                    roll_deg: 89.076,
                    translation_m: [-0.002063, 0.000334, -0.033284],
                },
                lens_type: 131,
            },
        ]
    }

    fn fixture(camera: Camera) -> Reframe {
        Reframe::new(&fixture_lenses(), FRAME, camera, 1.0, false)
    }

    /// The camera as it was before issue #27: one stream, one lens, one
    /// hemisphere. Legacy files that write a lens per file still render this
    /// way.
    fn one_lens(camera: Camera) -> Reframe {
        Reframe::new(&fixture_lenses()[..1], FRAME, camera, 1.0, false)
    }

    #[track_caller]
    fn near(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} is not within {tolerance} of {expected}"
        );
    }

    /// A direction in the body frame, `theta` degrees off the front lens's
    /// axis and turned `phi` degrees about it: `theta` of 90 is the seam
    /// great circle and 180 is straight out the back.
    fn direction(theta: f32, phi: f32) -> [f32; 3] {
        let (sin_theta, cos_theta) = theta.to_radians().sin_cos();
        let (sin_phi, cos_phi) = phi.to_radians().sin_cos();
        [sin_theta * cos_phi, sin_theta * sin_phi, cos_theta]
    }

    /// The sanity check the model has to pass before any pixel is believed:
    /// the middle of the view looks along the front lens's axis, and the lens
    /// axis is the principal point.
    ///
    /// Not exact, because the lens is not mounted exactly on the body axis:
    /// 0.103 degrees of yaw and 0.07 degrees of pitch tilt it by 0.125
    /// degrees, and near the axis the model's effective focal length is
    /// `fx / (1 + xi)` = 1106 px/rad, so 2.4 px.
    #[test]
    fn the_view_axis_lands_on_the_principal_point() {
        let reframe = fixture(Camera::default());
        let pick = reframe.pick(reframe.view_ray([0.5, 0.5]));

        assert_eq!(pick.lens, 0);
        assert!(pick.landing.inside);
        near(pick.landing.pixel[0], 1918.94, 3.0);
        near(pick.landing.pixel[1], 1927.21, 3.0);
    }

    /// And the other half of issue #27: the ray straight out the back is lens
    /// 1's own axis, so it is lens 1 that is picked and its principal point
    /// that it lands on. Without the nominal half turn in `opposed` this ray
    /// projects nowhere near lens 1's centre, and with the wrong half turn it
    /// lands there with the picture upside down.
    ///
    /// The tolerance is the same 3 px, off lens 1's own 0.2 degree mounting
    /// tilt.
    #[test]
    fn the_ray_out_the_back_lands_on_the_second_lens_principal_point() {
        let reframe = fixture(Camera::default());
        let pick = reframe.pick([0.0, 0.0, -1.0]);

        assert_eq!(pick.lens, 1);
        assert!(pick.landing.inside);
        near(pick.landing.pixel[0], 1935.35, 4.0);
        near(pick.landing.pixel[1], 1935.09, 4.0);
    }

    /// The nominal arrangement is a rotation and not a reflection, which is
    /// what says the back hemisphere is not mirrored: a turn about the lens
    /// axis has to come out as a turn the same way round. Up in the body
    /// frame lands above lens 1's principal point and body-right lands to its
    /// left, which is what looking the other way means.
    #[test]
    fn the_back_hemisphere_is_turned_around_and_not_mirrored() {
        let reframe = fixture(Camera::default());
        // 20 degrees off the back axis, up and to the right in the body
        // frame. y is down, so up is negative.
        let up = reframe.pick(normalize([0.0, -0.36, -1.0]));
        let right = reframe.pick(normalize([0.36, 0.0, -1.0]));

        assert_eq!((up.lens, right.lens), (1, 1));
        assert!(up.landing.pixel[1] < 1935.09 - 100.0, "{up:?}");
        near(up.landing.pixel[0], 1935.35, 40.0);
        assert!(right.landing.pixel[0] < 1935.35 - 100.0, "{right:?}");
        near(right.landing.pixel[1], 1935.09, 40.0);
    }

    /// A ray 90 degrees off the axis lands inside the image circle, which is
    /// the whole point of the mirror parameter: an equidistant fisheye model
    /// cannot represent this ray at all. 1802 px of the 1913 px circle, so
    /// the frame holds roughly 195 degrees across.
    #[test]
    fn a_ray_at_ninety_degrees_lands_inside_the_image_circle() {
        let reframe = fixture(Camera::default());
        // The view ray straight out to the right is 90 degrees off the view
        // axis, and a rotation into the lens frame preserves that angle to
        // within the lens's own 0.125 degree tilt.
        let landing = reframe.project(0, [1.0, 0.0, 0.0]);

        assert!(landing.inside);
        near(radius(&reframe, 0, landing), 1802.0, 8.0);
    }

    /// And a ray past one lens's coverage does not, so the other lens has
    /// something to answer for. The polynomial happily returns a coordinate
    /// for it; only the circle test rejects it.
    #[test]
    fn a_ray_past_the_lens_lands_outside_its_image_circle() {
        let reframe = fixture(Camera::default());
        let landing = reframe.project(0, direction(120.0, 0.0));

        assert!(!landing.inside);
        near(radius(&reframe, 0, landing), 2038.0, 8.0);
        // And it is the back lens that shows it.
        assert_eq!(reframe.pick(direction(120.0, 0.0)).lens, 1);
    }

    /// The whole sphere, on a grid fine enough to walk through the seam: every
    /// direction is in some lens's picture. A gap here is the failure issue
    /// #27 names first, and it is invisible in any one rendered view because
    /// the seam is a great circle and a view only ever crosses part of it.
    #[test]
    fn no_direction_is_in_neither_lens() {
        let reframe = fixture(Camera::default());

        for theta in 0..=720 {
            for phi in 0..72 {
                let ray = direction(theta as f32 * 0.25, phi as f32 * 5.0);
                let pick = reframe.pick(ray);
                assert!(
                    pick.landing.inside,
                    "no lens has {ray:?}, {} degrees off the front axis",
                    theta as f32 * 0.25
                );
            }
        }
    }

    /// Round the seam great circle, at the seam and either side of it: both
    /// lenses have the ray, because the overlap is about 15 degrees wide, and
    /// the one that wins is the one the ray leans toward.
    #[test]
    fn the_seam_is_a_choice_between_two_pictures_and_not_a_gap() {
        let reframe = fixture(Camera::default());

        for phi in 0..360 {
            let phi = phi as f32;
            for offset in [-5.0, -1.0, 0.0, 1.0, 5.0] {
                let ray = direction(90.0 + offset, phi);
                assert!(
                    reframe.project(0, ray).inside && reframe.project(1, ray).inside,
                    "the overlap does not reach {offset} degrees from the seam at {phi}",
                );
                // Which side of the seam wins is only settled a lens tilt away
                // from it: the two axes are 0.2 degrees off exactly opposed,
                // so on the halfway line itself either lens is a fair answer.
                if offset == 0.0 {
                    continue;
                }
                let picked = reframe.pick(ray).lens;
                assert_eq!(
                    picked,
                    usize::from(offset > 0.0),
                    "{offset} degrees past the seam at {phi} picked lens {picked}",
                );
            }
        }
    }

    /// Issue #30's guard, per lens: the picture each lens contributes is one
    /// cap around its own axis, not a cap and a ghost. Swept from that lens's
    /// own axis to straight behind it, `inside` goes off once and stays off.
    /// On the radius test alone it came back on at 131.5 degrees and stayed on
    /// all the way to 180, and with two lenses that ghost prints over a
    /// picture the other lens is drawing correctly.
    #[test]
    fn each_lens_picture_stops_once() {
        let reframe = fixture(Camera::default());

        for lens in 0..MAX_LENSES {
            let mut edge = None;

            for step in 0..=1800 {
                let theta = step as f32 * 0.1;
                // Off lens 1's axis is the supplement of off lens 0's, so the
                // same sweep runs each lens from its own axis to its own fold.
                let ray = match lens {
                    0 => direction(theta, 0.0),
                    _ => direction(180.0 - theta, 0.0),
                };
                match (reframe.project(lens, ray).inside, edge) {
                    (false, None) => edge = Some(theta),
                    (true, Some(stopped)) => {
                        panic!("lens {lens} stopped at {stopped} degrees and came back at {theta}")
                    }
                    _ => {}
                }
            }

            // Where the model reaches the image circle, which is what decides
            // how much picture there is: 97.5 degrees on lens 0, as the note
            // on `image_radius` says, and a shade less on lens 1, whose
            // principal point sits further off centre.
            near(edge.expect("the picture never stopped"), 97.2, 0.7);
        }
    }

    /// Grab-the-world, end to end and in lens pixels: whatever the middle of
    /// the output was showing is a tenth of the way right of the middle after
    /// a drag that far, reading the same lens pixel it did before.
    ///
    /// The camera's own tests measure the solve in angles; this is the one
    /// that says the angles and the lens agree about which way is which, so
    /// the drag and the picture cannot drift apart.
    #[test]
    fn a_horizontal_drag_carries_the_content_with_the_cursor() {
        let camera = Camera::default();
        let before = fixture(camera);
        let anchor = before.pick(before.view_ray([0.5, 0.5]));

        let mut dragged = camera;
        dragged.aim(camera.look([0.5, 0.5], 1.0), [0.6, 0.5], 1.0);
        assert!(dragged.yaw < 0.0, "dragging right turns the view left");

        let after = fixture(dragged);
        let moved = after.pick(after.view_ray([0.6, 0.5]));

        assert_eq!(moved.lens, anchor.lens);
        near(moved.landing.pixel[0], anchor.landing.pixel[0], 0.05);
        near(moved.landing.pixel[1], anchor.landing.pixel[1], 0.05);
    }

    /// The same for the vertical axis, which is the one whose sign is easy
    /// to get backwards: dragging down shows more sky.
    #[test]
    fn a_vertical_drag_carries_the_content_with_the_cursor() {
        let camera = Camera::default();
        let before = fixture(camera);
        let anchor = before.pick(before.view_ray([0.5, 0.5]));

        let mut dragged = camera;
        dragged.aim(camera.look([0.5, 0.5], 1.0), [0.5, 0.6], 1.0);
        assert!(dragged.pitch > 0.0, "dragging down looks up");

        let after = fixture(dragged);
        let moved = after.pick(after.view_ray([0.5, 0.6]));

        assert_eq!(moved.lens, anchor.lens);
        near(moved.landing.pixel[0], anchor.landing.pixel[0], 0.05);
        near(moved.landing.pixel[1], anchor.landing.pixel[1], 0.05);
    }

    /// And the same on the pilot's body, where issue #29 was reported: a view
    /// pitched most of the way down, a grab well off the middle of the
    /// output, and the lens pixel under the cursor stays the lens pixel under
    /// the cursor.
    #[test]
    fn a_drag_near_the_nadir_carries_the_content_with_the_cursor() {
        // `fixture` builds its block at aspect 1, which is what `view_ray`
        // below reads: the solve has to be told the same thing.
        let aspect = 1.0;
        let (from, to) = ([0.62, 0.6], [0.45, 0.45]);
        let camera = Camera {
            yaw: 0.4,
            pitch: -60f32.to_radians(),
            ..Camera::default()
        };
        let before = fixture(camera);
        let anchor = before.pick(before.view_ray(from));
        assert!(anchor.landing.inside, "grabbed a pixel no lens has");

        let mut dragged = camera;
        dragged.aim(camera.look(from, aspect), to, aspect);

        let after = fixture(dragged);
        let moved = after.pick(after.view_ray(to));

        assert_eq!(moved.lens, anchor.lens);
        near(moved.landing.pixel[0], anchor.landing.pixel[0], 1.0);
        near(moved.landing.pixel[1], anchor.landing.pixel[1], 1.0);
    }

    /// The datum, pinned: on a lens rolled a quarter turn, the top of the
    /// output lands above the principal point rather than beside it.
    /// Dropping `ROLL_DATUM_DEG` swaps those two, which is exactly the
    /// quarter turn the frames in docs/research/insv-format.md 4.8 ruled
    /// out.
    #[test]
    fn roll_is_measured_from_the_frames_horizontal_axis() {
        let reframe = fixture(Camera::default());
        let top = reframe.pick(reframe.view_ray([0.5, 0.1]));

        assert_eq!(top.lens, 0);
        assert!(top.landing.inside);
        assert!(top.landing.pixel[1] < 1927.21 - 100.0, "{top:?}");
        near(top.landing.pixel[0], 1918.94, 40.0);
    }

    /// A file with one stream is the camera it was before: one hemisphere,
    /// and grey behind it. The older cameras write a lens per file, and
    /// nothing about them changed with issue #27.
    #[test]
    fn one_stream_still_renders_one_hemisphere() {
        let reframe = one_lens(Camera::default());

        let front = reframe.pick(reframe.view_ray([0.5, 0.5]));
        assert_eq!(front.lens, 0);
        assert!(front.landing.inside);

        let back = reframe.pick([0.0, 0.0, -1.0]);
        assert_eq!(back.lens, 0);
        assert!(!back.landing.inside);
    }

    /// Issue #30: the ray straight out the back of one lens projects onto its
    /// principal point, which is as far inside the image circle as a pixel can
    /// get. Nothing but the domain test rejects it, and with two lenses the
    /// ghost it would draw is the other hemisphere's picture printed over this
    /// one.
    #[test]
    fn a_ray_from_straight_behind_a_lens_is_not_in_its_picture() {
        let reframe = fixture(Camera::default());

        for (lens, ray) in [(0, [0.0, 0.0, -1.0]), (1, [0.0, 0.0, 1.0])] {
            let landing = reframe.project(lens, ray);
            assert!(!landing.inside);
            assert!(radius(&reframe, lens, landing) < reframe.lenses[lens].image_radius);
            // The principal point itself, give or take the fifth of a degree
            // the lens is mounted off the body axis.
            near(radius(&reframe, lens, landing), 0.0, 15.0);
        }
    }

    /// The size the WGSL struct rounds up to, which is what the bind group
    /// declares as `min_binding_size`: pipeline creation is where a
    /// disagreement between the two definitions surfaces.
    #[test]
    fn the_uniform_block_is_the_size_wgsl_lays_it_out() {
        assert_eq!(std::mem::size_of::<LensBlock>(), 96);
        assert_eq!(std::mem::size_of::<Reframe>(), 224);
    }

    fn radius(reframe: &Reframe, lens: usize, landing: Landing) -> f32 {
        let block = &reframe.lenses[lens];
        norm([landing.pixel[0] - block.cx, landing.pixel[1] - block.cy])
    }
}
