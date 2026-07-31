//! The Mei/UCM forward map: one output ray to one lens pixel, for each lens
//! of the camera, and how much of each is shown.
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
//! Two lenses cover the sphere and overlap by about 14 degrees around the
//! seam, so most rays near it are in both pictures and the shader has to
//! decide how much of each to show. [`Reframe::blend`] is that decision: a
//! weight per lens, one outside the overlap and a smooth crossover inside it
//! (issue #7). A ray is dropped only where **no** lens has it.
//!
//! Written from the model description in `docs/research/insv-format.md` 5.1
//! (Mei and Rives 2007, as OpenCV's `cv::omnidir` states it). Nothing here
//! is transcribed from Gyroflow's `insta360.wgsl`, so this file is plain
//! AGPL-3.0 with no GPL header.

use kyerag_meta::{Intrinsics, Lens, Pose, Quat};

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
/// and is most of what [`Reframe::blend`] is weighing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    pub pixel: [f32; 2],
    pub inside: bool,
    /// The cosine of the angle between the ray and this lens's optical axis.
    /// 1 is straight down the axis and 0 is the seam great circle.
    pub axis: f32,
    /// How far the sample sits inside this lens's coverage: the distance in
    /// delivered-frame pixels from it to the edge of the image circle,
    /// positive inside and negative out. This is the distance transform from
    /// the lens's validity boundary that [`claim`] weighs with.
    pub depth: f32,
}

/// How much of the picture at one output pixel comes from each lens, and
/// where in each lens's frame it comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blend {
    pub landings: [Landing; MAX_LENSES],
    /// One per lens, summing to 1 wherever any lens has the ray and all zero
    /// where none does.
    pub weights: [f32; MAX_LENSES],
}

impl Blend {
    /// Whether any lens has this ray at all. False is what the shader paints
    /// [`OUTSIDE_GRAY`] for.
    pub fn is_covered(&self) -> bool {
        self.weights.iter().any(|weight| *weight > 0.0)
    }
}

/// What the shader paints where no lens has a picture. Neutral and dark,
/// in the same gamma-encoded space as the video, so the sRGB branch treats
/// it the same way it treats a sampled pixel.
pub const OUTSIDE_GRAY: f32 = 0.10;

/// Where the camera body was when a frame was taken, and how the view is to
/// be held against it.
///
/// `body_from_world` is the inverse of the orientation `kyerag-meta`
/// integrated: it takes a direction in the stabilized world frame to the
/// body's own. Identity is horizon lock switched off, and then the view is in
/// body coordinates exactly as it was before issue #8.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Held {
    pub body_from_world: Quat,
}

impl Default for Held {
    fn default() -> Self {
        Self {
            body_from_world: Quat::IDENTITY,
        }
    }
}

impl Reframe {
    /// The block for one camera pose and the lenses of one file, in file
    /// order. Anything past [`MAX_LENSES`] is dropped.
    pub fn new(
        lenses: &[Lens],
        frame: Size,
        camera: Camera,
        held: Held,
        aspect: f32,
        linearize: bool,
    ) -> Self {
        Self {
            lenses: std::array::from_fn(|index| match lenses.get(index) {
                Some(lens) => LensBlock::new(lens, index, frame, camera, held),
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

    /// How much of this ray each lens shows, and where in its frame.
    ///
    /// Each lens stakes a [`claim`] on the ray and the claims are normalized
    /// against each other, so the weights sum to 1 wherever anything has the
    /// ray. Outside the overlap only one lens claims anything and its weight
    /// is exactly 1 (see [`share`]), so the pass takes the one sample the
    /// hard pick took before issue #7 and multiplies it by an exact one.
    ///
    /// Every lens is projected, including a slot the file has no stream for,
    /// so that the shader's loop runs a constant number of times. A loop
    /// bounded by `lens_count` cannot be unrolled, and one that is not
    /// unrolled indexes its arrays dynamically, which puts them in scratch
    /// memory: measured on RADV 2026-07-31, that alone is 1.82 ms per redraw
    /// against 1.68 at 2560x1440, more than the second texture fetch the
    /// blend actually needs costs.
    ///
    /// WGSL twin: `blend`.
    pub fn blend(&self, view_ray: [f32; 3]) -> Blend {
        let landings = std::array::from_fn(|lens| self.project(lens, view_ray));
        let mut weights: [f32; MAX_LENSES] =
            std::array::from_fn(|lens| match lens < self.lens_count as usize {
                true => claim(landings[lens]),
                false => 0.0,
            });
        let total: f32 = weights.iter().sum();
        if total > 0.0 {
            for weight in &mut weights {
                *weight = share(*weight, total);
            }
        }
        Blend { landings, weights }
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
        let depth = lens.image_radius - norm(offset);
        Landing {
            pixel: [offset[0] + lens.cx, offset[1] + lens.cy],
            inside: denom > 0.0 && injective && depth > 0.0,
            axis: p[2],
            depth,
        }
    }
}

/// One lens's unnormalized claim on a ray, which [`Reframe::blend`] weighs
/// against the other lens's.
///
/// Two factors, per docs/research/insv-format.md 6.6, and neither of them is
/// a feather width to be chosen:
///
/// - **longitude preference**, [`longitude`], which is what puts the
///   crossover on the seam great circle rather than wherever the two
///   coverages happen to meet;
/// - **coverage depth**, `landing.depth`, the distance transform from this
///   lens's own validity boundary. It reaches zero exactly where the picture
///   stops, so a lens fades out as it runs out of picture, and the rim of
///   the image circle, which is where vignetting lands and where the
///   distortion polynomial is least trustworthy (5.3), is down-weighted for
///   free.
///
/// The band the product blends over is therefore the overlap itself: on the
/// X4 Air fixture it runs 83.4 to 97.4 degrees off the front axis, because
/// that is where both lenses have any picture at all.
///
/// WGSL twin: `claim`.
fn claim(landing: Landing) -> f32 {
    match landing.inside {
        true => longitude(landing.axis) * landing.depth,
        false => 0.0,
    }
}

/// One claim's share of all of them. `total` must be positive; the caller
/// has nothing to normalize otherwise.
///
/// The lone claimant's share is written rather than divided out. Measured on
/// RADV 2026-07-31: a GPU `x / x` is a reciprocal multiply and lands an ulp
/// under 1.0, and multiplying a sample by that reaches an 8-bit picture as
/// one code on 6 pixels of a million. Writing it is what lets a one-stream
/// ONE X2 file render bit for bit what it rendered before the blend existed,
/// which it does at every yaw tested.
///
/// WGSL twin: `share`.
fn share(claim: f32, total: f32) -> f32 {
    match claim == total {
        true => 1.0,
        false => claim / total,
    }
}

/// How much this lens is preferred for a ray `theta` off its axis, from
/// `cos(theta)`: `cos^2(theta / 2)`, i.e. 1 straight down the axis, 1/2 on
/// the seam great circle, 0 straight out the back.
///
/// It is never zero anywhere in the overlap, so it only tilts the crossover;
/// the coverage depth is what closes it. Being exactly 1/2 for both lenses
/// on the seam is the whole job, and it is the reason the crossover does not
/// drift to wherever the two image circles happen to end.
///
/// WGSL twin: `longitude`.
fn longitude(axis: f32) -> f32 {
    0.5 * (1.0 + axis)
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

    fn new(lens: &Lens, index: usize, frame: Size, camera: Camera, held: Held) -> Self {
        let Intrinsics { xi, fx, fy, cx, cy } = lens.intrinsics;
        let distortion = lens.distortion;
        Self {
            view_to_lens: view_to_lens(&lens.pose, index, camera, held).columns(),
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
/// Three steps, right to left: where the view is pointing in the world, where
/// the camera body was when the frame was taken, and where this lens sits on
/// that body. Every one of them is right-handed in the frame the projection
/// uses: x right, y down, z along the axis being pointed. Positive camera yaw
/// turns right, positive camera pitch looks up.
///
/// The middle step is horizon lock (issue #8), and it is the whole of it: the
/// camera's own yaw and pitch are read in the **stabilized world** frame
/// rather than the body's, so a body that rolls under a level view leaves the
/// view level. It is also why the drag needed no change at all. `Camera::look`
/// answers in whatever frame `camera_rotation` lands in, the drag anchors a
/// direction in that frame and solves for the view that puts it back there,
/// and with lock on that frame is the world: the anchor stays on the world
/// and the picture turns under it.
fn view_to_lens(pose: &Pose, index: usize, camera: Camera, held: Held) -> Mat3 {
    lens_from_body(pose, index)
        .mul(Mat3::from(held.body_from_world.matrix().rows()))
        .mul(camera_rotation(camera))
}

/// Yaw about the world vertical, then pitch about the view's own horizontal.
/// Never roll: the horizon stays level, which is the whole reason a drag near
/// the pole has to give something up (issue #29).
fn camera_rotation(camera: Camera) -> Mat3 {
    Mat3::rot_y(camera.yaw as f64).mul(Mat3::rot_x(camera.pitch as f64))
}

/// The lens's own mounting, over the nominal arrangement it is mounted in
/// ([`opposed`]).
///
/// The three angles and the quarter-turn datum they are measured against live
/// in `kyerag_meta::Pose::lens_from_body`, because the IMU needs the same
/// rotation to get out of the front lens's frame and into the body's, and one
/// settled convention wants one definition.
fn lens_from_body(pose: &Pose, index: usize) -> Mat3 {
    Mat3::from(pose.lens_from_body().rows()).mul(opposed(index))
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
/// model reaches at about 97.4 degrees off axis, so two lenses overlap by
/// about 14 degrees around the seam. That circle is the validity boundary
/// [`claim`] measures its coverage depth from, so it sets the blend band as
/// well as the picture's edge.
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

impl From<[[f64; 3]; 3]> for Mat3 {
    fn from(rows: [[f64; 3]; 3]) -> Self {
        Self(rows)
    }
}

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
  depth: f32,
};

struct Blend {
  landings: array<Landing, MAX_LENSES>,
  weights: array<f32, MAX_LENSES>,
};

// x right, y down, z forward, matching the lens frame the model projects in.
fn view_ray(uv: vec2<f32>) -> vec3<f32> {
  let plane = (uv * 2.0 - vec2<f32>(1.0)) * reframe.tan_half_fov;
  return vec3<f32>(plane.x, plane.y / reframe.aspect, 1.0);
}

// Every lens's claim on the ray, normalized. Rust twin: `Reframe::blend`.
//
// The loop runs MAX_LENSES times whatever the file holds, and the lens count
// zeroes the claim of a slot that has no stream rather than shortening the
// loop. A loop this compiler cannot unroll indexes `out` dynamically, which
// puts it in scratch memory and costs more than the blend does; the numbers
// are on the Rust twin.
fn blend(ray: vec3<f32>) -> Blend {
  var out: Blend;
  var total = 0.0;
  for (var lens = 0u; lens < MAX_LENSES; lens += 1u) {
    let landing = project(lens, ray);
    out.landings[lens] = landing;
    out.weights[lens] = select(0.0, claim(landing), f32(lens) < reframe.lens_count);
    total += out.weights[lens];
  }
  if total > 0.0 {
    for (var lens = 0u; lens < MAX_LENSES; lens += 1u) {
      out.weights[lens] = share(out.weights[lens], total);
    }
  }
  return out;
}

// One claim's share of all of them, the lone claimant's written rather than
// divided out. Rust twin: `share`.
fn share(claim: f32, total: f32) -> f32 {
  if claim == total {
    return 1.0;
  }
  return claim / total;
}

// Longitude preference times coverage depth. Rust twin: `claim`.
fn claim(landing: Landing) -> f32 {
  if !landing.inside {
    return 0.0;
  }
  return longitude(landing.axis) * landing.depth;
}

// cos^2(theta / 2), from cos(theta). Rust twin: `longitude`.
fn longitude(axis: f32) -> f32 {
  return 0.5 * (1.0 + axis);
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
  let depth = lens.image_radius - length(offset);
  var landing: Landing;
  landing.pixel = offset + vec2<f32>(lens.cx, lens.cy);
  landing.inside = denom > 0.0 && injective && depth > 0.0;
  landing.axis = p.z;
  landing.depth = depth;
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
        held(camera, Held::default())
    }

    /// The same fixture with the camera body somewhere other than level,
    /// which is what horizon lock has to take back out.
    fn held(camera: Camera, held: Held) -> Reframe {
        Reframe::new(&fixture_lenses(), FRAME, camera, held, 1.0, false)
    }

    /// The camera as it was before issue #27: one stream, one lens, one
    /// hemisphere. Legacy files that write a lens per file still render this
    /// way.
    fn one_lens(camera: Camera) -> Reframe {
        Reframe::new(
            &fixture_lenses()[..1],
            FRAME,
            camera,
            Held::default(),
            1.0,
            false,
        )
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

    /// The lens carrying most of an output pixel, and where it lands, which
    /// is the question the hard pick answered before issue #7. `None` where
    /// no lens has the ray, which is what the shader paints grey.
    fn shown(reframe: &Reframe, ray: [f32; 3]) -> Option<(usize, Landing)> {
        let blend = reframe.blend(ray);
        let lens =
            (0..MAX_LENSES).max_by(|a, b| blend.weights[*a].total_cmp(&blend.weights[*b]))?;
        blend.is_covered().then(|| (lens, blend.landings[lens]))
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
        let (lens, landing) =
            shown(&reframe, reframe.view_ray([0.5, 0.5])).expect("no lens has the view axis");

        assert_eq!(lens, 0);
        near(landing.pixel[0], 1918.94, 3.0);
        near(landing.pixel[1], 1927.21, 3.0);
    }

    /// And the other half of issue #27: the ray straight out the back is lens
    /// 1's own axis, so it is lens 1 that shows it and its principal point
    /// that it lands on. Without the nominal half turn in `opposed` this ray
    /// projects nowhere near lens 1's centre, and with the wrong half turn it
    /// lands there with the picture upside down.
    ///
    /// The tolerance is the same 3 px, off lens 1's own 0.2 degree mounting
    /// tilt.
    #[test]
    fn the_ray_out_the_back_lands_on_the_second_lens_principal_point() {
        let reframe = fixture(Camera::default());
        let (lens, landing) = shown(&reframe, [0.0, 0.0, -1.0]).expect("no lens has the back axis");

        assert_eq!(lens, 1);
        near(landing.pixel[0], 1935.35, 4.0);
        near(landing.pixel[1], 1935.09, 4.0);
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
        let up = shown(&reframe, normalize([0.0, -0.36, -1.0])).expect("nothing above the back");
        let right = shown(&reframe, normalize([0.36, 0.0, -1.0])).expect("nothing right of it");

        assert_eq!((up.0, right.0), (1, 1));
        assert!(up.1.pixel[1] < 1935.09 - 100.0, "{up:?}");
        near(up.1.pixel[0], 1935.35, 40.0);
        assert!(right.1.pixel[0] < 1935.35 - 100.0, "{right:?}");
        near(right.1.pixel[1], 1935.09, 40.0);
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
        // And it is the back lens that shows it, on its own.
        let blend = reframe.blend(direction(120.0, 0.0));
        assert_eq!(blend.weights, [0.0, 1.0]);
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
                assert!(
                    reframe.blend(ray).is_covered(),
                    "no lens has {ray:?}, {} degrees off the front axis",
                    theta as f32 * 0.25
                );
            }
        }
    }

    /// Round the seam great circle, at the seam and either side of it: both
    /// lenses have the ray, because the overlap is about 14 degrees wide, and
    /// the one that leads is the one the ray leans toward.
    #[test]
    fn the_seam_is_a_mix_of_two_pictures_and_not_a_gap() {
        let reframe = fixture(Camera::default());

        for phi in 0..360 {
            let phi = phi as f32;
            for offset in [-5.0, -1.0, 0.0, 1.0, 5.0] {
                let ray = direction(90.0 + offset, phi);
                let blend = reframe.blend(ray);
                assert!(
                    blend.weights.iter().all(|weight| *weight > 0.0),
                    "the overlap does not reach {offset} degrees from the seam at {phi}",
                );
                // Which side of the seam leads is only settled a lens tilt
                // away from it: the two axes are 0.2 degrees off exactly
                // opposed, so on the halfway line itself either lens is a
                // fair answer.
                if offset == 0.0 {
                    continue;
                }
                let leader = usize::from(blend.weights[1] > blend.weights[0]);
                assert_eq!(
                    leader,
                    usize::from(offset > 0.0),
                    "{offset} degrees past the seam at {phi} leads with lens {leader}",
                );
            }
        }
    }

    /// The blend's first invariant, over the whole sphere: an output pixel is
    /// one pixel's worth of picture. Anything else is a seam that reads as a
    /// bright or dark line, which is the artifact issue #7 exists to remove.
    #[test]
    fn the_weights_are_one_pixels_worth_of_picture_everywhere() {
        let reframe = fixture(Camera::default());

        for theta in 0..=720 {
            for phi in 0..72 {
                let theta = theta as f32 * 0.25;
                let blend = reframe.blend(direction(theta, phi as f32 * 5.0));
                let total: f32 = blend.weights.iter().sum();
                near(total, 1.0, 1e-6);
                assert!(
                    blend.weights.iter().all(|weight| *weight >= 0.0),
                    "{theta} degrees off the front axis weighs {:?}",
                    blend.weights
                );
            }
        }
    }

    /// Outside the overlap the second lens contributes nothing, and the first
    /// one's weight is exactly 1 rather than nearly it. That exactness is
    /// what makes the picture away from the seam the same bits it was before
    /// the blend, and it is what lets the shader skip the second fetch.
    #[test]
    fn one_lens_carries_everything_outside_the_overlap() {
        let reframe = fixture(Camera::default());

        for theta in [0.0, 30.0, 60.0, 80.0, 100.0, 130.0, 180.0] {
            for phi in 0..8 {
                let blend = reframe.blend(direction(theta, phi as f32 * 45.0));
                assert_eq!(
                    blend.weights,
                    match theta < 90.0 {
                        true => [1.0, 0.0],
                        false => [0.0, 1.0],
                    },
                    "{theta} degrees off the front axis"
                );
            }
        }
    }

    /// The crossover sits on the seam great circle, which is what the
    /// longitude preference buys: without it the two coverage depths cross
    /// wherever the two image circles happen to end, which is 0.2 degrees off
    /// on this fixture and camera-dependent in general.
    ///
    /// Not exactly half: lens 1's image circle is 8 px smaller than lens 0's,
    /// so it runs out of coverage marginally sooner and carries marginally
    /// less of the seam.
    #[test]
    fn the_crossover_sits_on_the_seam() {
        let reframe = fixture(Camera::default());

        for phi in 0..36 {
            let blend = reframe.blend(direction(90.0, phi as f32 * 10.0));
            near(blend.weights[0], 0.5, 0.03);
            near(blend.weights[1], 0.5, 0.03);
        }
    }

    /// Continuity, which is the property the eye actually reads: swept
    /// through the whole band a hundred steps to the degree, no lens's weight
    /// ever moves more than a hundredth in one step. The hard pick this
    /// replaces moved 1.0 in one step, at the seam, which is the line issue
    /// #7 was filed about.
    ///
    /// The sweep runs well past both edges of the overlap, so it also covers
    /// the two places a weight arrives at 0: the band edge is where a blend
    /// with a feather width of its own would show a crease.
    #[test]
    fn no_weight_ever_steps() {
        let reframe = fixture(Camera::default());

        for phi in [0.0, 90.0, 180.0, 270.0] {
            let mut held = reframe.blend(direction(70.0, phi)).weights;
            let mut worst: f32 = 0.0;

            for step in 1..=4000 {
                let weights = reframe
                    .blend(direction(70.0 + step as f32 * 0.01, phi))
                    .weights;
                for lens in 0..MAX_LENSES {
                    worst = worst.max((weights[lens] - held[lens]).abs());
                }
                held = weights;
            }

            assert!(worst < 0.01, "a weight jumped by {worst} at phi {phi}");
        }
    }

    /// And the band is the overlap itself rather than a width chosen here:
    /// the weights are mixed exactly where both lenses have a picture, which
    /// on this fixture is 83.4 to 97.4 degrees off the front axis, and the
    /// two lenses hand over across the whole of it.
    #[test]
    fn the_blend_band_is_the_overlap_itself() {
        let reframe = fixture(Camera::default());
        let mixed: Vec<f32> = (0..3000)
            .map(|step| 70.0 + step as f32 * 0.01)
            .filter(|theta| {
                let weights = reframe.blend(direction(*theta, 0.0)).weights;
                weights.iter().all(|weight| *weight > 0.0)
            })
            .collect();

        near(*mixed.first().expect("nothing is mixed at all"), 83.2, 0.2);
        near(*mixed.last().expect("nothing is mixed at all"), 97.4, 0.2);
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
        let anchor = shown(&before, before.view_ray([0.5, 0.5])).expect("grabbed nothing");

        let mut dragged = camera;
        dragged.aim(camera.look([0.5, 0.5], 1.0), [0.6, 0.5], 1.0);
        assert!(dragged.yaw < 0.0, "dragging right turns the view left");

        let after = fixture(dragged);
        let moved = shown(&after, after.view_ray([0.6, 0.5])).expect("dragged onto nothing");

        assert_eq!(moved.0, anchor.0);
        near(moved.1.pixel[0], anchor.1.pixel[0], 0.05);
        near(moved.1.pixel[1], anchor.1.pixel[1], 0.05);
    }

    /// The same for the vertical axis, which is the one whose sign is easy
    /// to get backwards: dragging down shows more sky.
    #[test]
    fn a_vertical_drag_carries_the_content_with_the_cursor() {
        let camera = Camera::default();
        let before = fixture(camera);
        let anchor = shown(&before, before.view_ray([0.5, 0.5])).expect("grabbed nothing");

        let mut dragged = camera;
        dragged.aim(camera.look([0.5, 0.5], 1.0), [0.5, 0.6], 1.0);
        assert!(dragged.pitch > 0.0, "dragging down looks up");

        let after = fixture(dragged);
        let moved = shown(&after, after.view_ray([0.5, 0.6])).expect("dragged onto nothing");

        assert_eq!(moved.0, anchor.0);
        near(moved.1.pixel[0], anchor.1.pixel[0], 0.05);
        near(moved.1.pixel[1], anchor.1.pixel[1], 0.05);
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
        let anchor = shown(&before, before.view_ray(from)).expect("grabbed a pixel no lens has");

        let mut dragged = camera;
        dragged.aim(camera.look(from, aspect), to, aspect);

        let after = fixture(dragged);
        let moved = shown(&after, after.view_ray(to)).expect("dragged onto nothing");

        assert_eq!(moved.0, anchor.0);
        near(moved.1.pixel[0], anchor.1.pixel[0], 1.0);
        near(moved.1.pixel[1], anchor.1.pixel[1], 1.0);
    }

    /// The datum, pinned: on a lens rolled a quarter turn, the top of the
    /// output lands above the principal point rather than beside it.
    /// Dropping `ROLL_DATUM_DEG` swaps those two, which is exactly the
    /// quarter turn the frames in docs/research/insv-format.md 4.8 ruled
    /// out.
    #[test]
    fn roll_is_measured_from_the_frames_horizontal_axis() {
        let reframe = fixture(Camera::default());
        let (lens, landing) =
            shown(&reframe, reframe.view_ray([0.5, 0.1])).expect("nothing at the top of the view");

        assert_eq!(lens, 0);
        assert!(landing.pixel[1] < 1927.21 - 100.0, "{landing:?}");
        near(landing.pixel[0], 1918.94, 40.0);
    }

    /// A file with one stream is the camera it was before: one hemisphere,
    /// and grey behind it. The older cameras write a lens per file, and
    /// nothing about them changed with issue #27 or #7.
    #[test]
    fn one_stream_still_renders_one_hemisphere() {
        let reframe = one_lens(Camera::default());

        let front = reframe.blend(reframe.view_ray([0.5, 0.5]));
        assert_eq!(front.weights, [1.0, 0.0]);

        let back = reframe.blend([0.0, 0.0, -1.0]);
        assert_eq!(back.weights, [0.0, 0.0]);
        assert!(!back.is_covered());
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

    /// Horizon lock, in lens pixels: a body rolled a quarter turn shows the
    /// same world direction at the same place in the output.
    ///
    /// This is the whole claim of issue #8 reduced to one number. The camera
    /// is left alone, the body is turned under it, and the pixel the middle
    /// of the output reads has to be the pixel a body-frame direction 90
    /// degrees round predicts, because with the lock on the view is in the
    /// world frame and the world did not move.
    #[test]
    fn a_rolled_body_shows_the_same_world_direction_in_the_same_place() {
        // Off both centre lines, so a roll about the view axis moves it in
        // both directions rather than sliding it along one.
        const OFF_AXIS: [f32; 2] = [0.3, 0.3];
        let camera = Camera::default();
        let level = fixture(camera);
        let anchor = shown(&level, level.view_ray(OFF_AXIS)).expect("grabbed nothing");

        for roll in [10.0f64, -35.0, 90.0, 179.0] {
            // The body rolled about its own forward axis, which is what a
            // camera swinging under a wing does.
            let world_from_body = Quat::from_rotation_vector([0.0, 0.0, roll.to_radians()]);
            let rolled = held(
                camera,
                Held {
                    body_from_world: world_from_body.conjugate(),
                },
            );
            let moved = shown(&rolled, rolled.view_ray(OFF_AXIS)).expect("rolled onto nothing");

            // The world direction is unchanged, so it lands in whichever lens
            // pixel that direction has always landed in: the body turned, so
            // that pixel moved, and this is the check that it moved by
            // exactly the roll.
            let turned = world_from_body
                .conjugate()
                .rotate(level.view_ray(OFF_AXIS).map(f64::from))
                .map(|axis| axis as f32);
            let expected = shown(&level, turned).expect("the turned ray is in no lens");
            assert_eq!(moved.0, expected.0, "{roll} degrees");
            near(moved.1.pixel[0], expected.1.pixel[0], 0.05);
            near(moved.1.pixel[1], expected.1.pixel[1], 0.05);
            // And it is not a no-op: a rolled body really does read a
            // different pixel than a level one, by hundreds of pixels here.
            let apart = norm([
                moved.1.pixel[0] - anchor.1.pixel[0],
                moved.1.pixel[1] - anchor.1.pixel[1],
            ]);
            assert!(
                apart > 50.0,
                "{roll} degrees of roll moved the sample {apart} px"
            );
        }
    }

    /// And with the lock off nothing changed: identity is exactly the
    /// composition the pass had before issue #8, down to the bits.
    #[test]
    fn an_identity_hold_is_the_pass_as_it_was() {
        let camera = Camera {
            yaw: 0.7,
            pitch: -0.4,
            ..Camera::default()
        };
        let plain = fixture(camera);
        let identity = held(camera, Held::default());

        for lens in 0..MAX_LENSES {
            assert_eq!(
                plain.lenses[lens].view_to_lens,
                identity.lenses[lens].view_to_lens
            );
        }
    }

    /// The drag composes with the lock, which is the other half of issue #8's
    /// requirement: the grabbed content stays under the cursor while the
    /// horizon is being held.
    ///
    /// It needs no code of its own, and that is the finding. `Camera::look`
    /// answers in whatever frame `camera_rotation` lands in; the lock moves
    /// that frame from the body to the world, so the anchor is a world
    /// direction and the solve puts a world direction back under the cursor.
    /// The check is in lens pixels, because angles agreeing while pixels do
    /// not is exactly the bug a frame composition can have.
    #[test]
    fn a_drag_still_carries_the_content_while_the_horizon_is_held() {
        let aspect = 1.0;
        let (from, to) = ([0.62, 0.6], [0.38, 0.42]);
        let camera = Camera {
            yaw: 0.4,
            pitch: -0.3,
            ..Camera::default()
        };
        // A body that is neither level nor pointing where the view is.
        let hold = Held {
            body_from_world: Quat::from_rotation_vector([0.15, -0.4, 0.7]).conjugate(),
        };

        let before = held(camera, hold);
        let anchor = shown(&before, before.view_ray(from)).expect("grabbed a pixel no lens has");

        let mut dragged = camera;
        dragged.aim(camera.look(from, aspect), to, aspect);
        assert_ne!(dragged, camera, "the drag moved nothing");

        let after = held(dragged, hold);
        let moved = shown(&after, after.view_ray(to)).expect("dragged onto nothing");

        assert_eq!(moved.0, anchor.0);
        near(moved.1.pixel[0], anchor.1.pixel[0], 1.0);
        near(moved.1.pixel[1], anchor.1.pixel[1], 1.0);
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
