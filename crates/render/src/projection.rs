//! The Mei/UCM forward map: one output ray to one lens pixel.
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
//! Written from the model description in `docs/research/insv-format.md` 5.1
//! (Mei and Rives 2007, as OpenCV's `cv::omnidir` states it). Nothing here
//! is transcribed from Gyroflow's `insta360.wgsl`, so this file is plain
//! AGPL-3.0 with no GPL header.

use kyerag_meta::{Intrinsics, Lens, Pose};

use super::{Camera, Size};

/// The uniform block, mirrored field for field by `struct Reframe` in
/// `WGSL`. All `f32`: the calibration is `f64` on the way in and the
/// composition below is done in `f64`, but a GPU uniform is `f32` and there
/// is nothing here that 24 bits of mantissa cannot hold (the largest number
/// is a pixel coordinate under 4096).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Reframe {
    /// A `mat3x3<f32>` as WGSL lays one out: three columns, each padded to
    /// 16 bytes. Takes a view-space ray to the lens frame.
    view_to_lens: [[f32; 4]; 3],
    tan_half_fov: f32,
    /// Output width over output height. The vertical field of view is
    /// whatever this leaves.
    aspect: f32,
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
    frame_width: f32,
    frame_height: f32,
    image_radius: f32,
    has_frame: f32,
    linearize: f32,
    elapsed: f32,
    /// A uniform struct's size rounds up to its 16-byte alignment. WGSL does
    /// that itself; `repr(C)` does not.
    _pad: [f32; 2],
}

/// Where a view ray lands in the lens image, in delivered-frame pixels.
///
/// `inside` false means the ray missed the lens, and the shader paints
/// [`OUTSIDE_GRAY`] rather than whatever the clamped sample returned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    pub pixel: [f32; 2],
    pub inside: bool,
}

/// What the shader paints where the lens has no picture. Neutral and dark,
/// in the same gamma-encoded space as the video, so the sRGB branch treats
/// it the same way it treats a sampled pixel.
pub const OUTSIDE_GRAY: f32 = 0.10;

impl Reframe {
    /// The block for one lens and one camera pose.
    pub fn new(lens: &Lens, frame: Size, camera: Camera, aspect: f32, linearize: bool) -> Self {
        let Intrinsics { xi, fx, fy, cx, cy } = lens.intrinsics;
        let distortion = lens.distortion;
        Self {
            view_to_lens: view_to_lens(&lens.pose, camera).columns(),
            tan_half_fov: (camera.fov * 0.5).tan(),
            aspect,
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
            frame_width: frame.width as f32,
            frame_height: frame.height as f32,
            image_radius: image_radius(&lens.intrinsics, frame) as f32,
            has_frame: 1.0,
            linearize: f32::from(u8::from(linearize)),
            elapsed: 0.0,
            _pad: [0.0; 2],
        }
    }

    /// No file open: the shader draws its bring-up gradient. Every ray still
    /// runs the map, because `textureSample` needs uniform control flow, so
    /// the numbers here are chosen to make that harmless: `xi` of 1 keeps
    /// the denominator positive and a zero image radius puts every ray
    /// outside.
    pub fn gradient(elapsed: f32, aspect: f32, linearize: bool) -> Self {
        Self {
            view_to_lens: Mat3::IDENTITY.columns(),
            tan_half_fov: 1.0,
            aspect,
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
            frame_width: 1.0,
            frame_height: 1.0,
            image_radius: 0.0,
            has_frame: 0.0,
            linearize: f32::from(u8::from(linearize)),
            elapsed,
            _pad: [0.0; 2],
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

    /// The forward map: a view ray, through the lens's extrinsics and the
    /// Mei/UCM model, to a pixel of the delivered frame.
    ///
    /// WGSL twin: `project`. The shader adds one line the mirror does not,
    /// turning the pixel into a texture coordinate (`frame_uv`).
    pub fn project(&self, view_ray: [f32; 3]) -> Landing {
        let p = normalize(self.lens_ray(view_ray));

        // The mirror parameter is why a ray past 90 degrees off axis still
        // has a finite projection: it only needs `z + xi > 0`. On this
        // camera family xi is above 1, so the guard never fires; it is here
        // for a model where xi is smaller than 1.
        let denom = p[2] + self.xi;
        let x = p[0] / denom;
        let y = p[1] / denom;

        let r2 = x * x + y * y;
        let radial = 1.0 + r2 * (self.k1 + r2 * (self.k2 + r2 * self.k3));
        let xd = x * radial + 2.0 * self.p1 * x * y + self.p2 * (r2 + 2.0 * x * x);
        let yd = y * radial + 2.0 * self.p2 * x * y + self.p1 * (r2 + 2.0 * y * y);

        let offset = [self.fx * xd, self.fy * yd];
        // How far round the map can be believed, which is not as far as it
        // answers. The distance from the principal point grows with the angle
        // off the axis only up to `cos(theta) = -1/xi`; past that turning
        // point it comes back down, re-enters the image circle, and a ray
        // from behind the camera lands a second time on a pixel that belongs
        // to a ray in front of it. That second landing is issue #30's ghost,
        // a raw circular fisheye hanging behind the reframed view, and the
        // radius test cannot see it because the fold puts it well inside the
        // circle. Vacuous where xi is below 1: there is no turning point
        // there, the radius runs away to infinity instead, and `denom` is the
        // limit that binds.
        let injective = p[2] * self.xi > -1.0;
        Landing {
            pixel: [offset[0] + self.cx, offset[1] + self.cy],
            inside: denom > 0.0 && injective && norm(offset) <= self.image_radius,
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

/// The rotation that takes a view-space ray to the lens frame.
///
/// Both halves are right-handed in the frame the projection uses: x right,
/// y down, z along the axis being pointed. Positive camera yaw turns right,
/// positive camera pitch looks up.
fn view_to_lens(pose: &Pose, camera: Camera) -> Mat3 {
    lens_from_body(pose).mul(camera_rotation(camera))
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

/// The lens's own mounting, as `offset_v3` records it: roll about the
/// optical axis, then the sub-degree yaw and pitch of the mounting.
///
/// The **order** of the three is not settled, and neither camera can settle
/// it: yaw and pitch are 0.103 and 0.07 degrees on the X4 Air, and near the
/// axis the model's effective focal length is `fx / (1 + xi)` = 1106 px/rad,
/// so every ordering agrees to about 2 px. A camera with a large yaw or
/// pitch would tell them apart; none is known to exist.
fn lens_from_body(pose: &Pose) -> Mat3 {
    Mat3::rot_z((pose.roll_deg + ROLL_DATUM_DEG).to_radians())
        .mul(Mat3::rot_y(pose.yaw_deg.to_radians()))
        .mul(Mat3::rot_x(pose.pitch_deg.to_radians()))
}

/// The largest circle centred on the principal point that fits in the
/// delivered frame.
///
/// The file records no image-circle radius and the model does not bound
/// itself: past the lens's real coverage the radial polynomial keeps
/// returning finite pixel coordinates, so something has to say where the
/// picture stops. On the X4 Air fixture this radius is 1913 px, which the
/// model reaches at about 97.5 degrees off axis; the frame corners hold a
/// little more than that, and the seam blend (issue #7) is what will want
/// it.
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
    format!("const OUTSIDE_GRAY = vec3<f32>({OUTSIDE_GRAY:?});\n{WGSL}")
}

const WGSL: &str = r#"
struct Reframe {
  view_to_lens: mat3x3<f32>,
  tan_half_fov: f32,
  aspect: f32,
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
  frame_width: f32,
  frame_height: f32,
  image_radius: f32,
  has_frame: f32,
  linearize: f32,
  elapsed: f32,
};

@group(0) @binding(0) var<uniform> reframe: Reframe;

struct Landing {
  pixel: vec2<f32>,
  inside: bool,
};

// x right, y down, z forward, matching the lens frame the model projects in.
fn view_ray(uv: vec2<f32>) -> vec3<f32> {
  let plane = (uv * 2.0 - vec2<f32>(1.0)) * reframe.tan_half_fov;
  return vec3<f32>(plane.x, plane.y / reframe.aspect, 1.0);
}

// Mei/UCM forward map. Rust twin: `Reframe::project`.
fn project(ray: vec3<f32>) -> Landing {
  let p = normalize(reframe.view_to_lens * ray);
  let denom = p.z + reframe.xi;
  let n = p.xy / denom;

  let r2 = dot(n, n);
  let radial = 1.0 + r2 * (reframe.k1 + r2 * (reframe.k2 + r2 * reframe.k3));
  let tangential = vec2<f32>(
    2.0 * reframe.p1 * n.x * n.y + reframe.p2 * (r2 + 2.0 * n.x * n.x),
    2.0 * reframe.p2 * n.x * n.y + reframe.p1 * (r2 + 2.0 * n.y * n.y),
  );
  let d = n * radial + tangential;

  let offset = vec2<f32>(reframe.fx * d.x, reframe.fy * d.y);
  // Past `cos(theta) = -1/xi` the map folds and lands rays from behind the
  // camera back inside the image circle. Rust twin: `injective`.
  let injective = p.z * reframe.xi > -1.0;
  var landing: Landing;
  landing.pixel = offset + vec2<f32>(reframe.cx, reframe.cy);
  landing.inside = denom > 0.0 && injective && length(offset) <= reframe.image_radius;
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

    /// The X4 Air fixture's lens 0 in delivered-frame pixels: what
    /// `kyerag-meta` produces from `docs/research/x4air-calibration.json`,
    /// and what its own tests assert. Copied rather than parsed because the
    /// path from the fixture to a `CalibrationSet` runs through a private
    /// constructor in a crate this one only reads types from.
    fn fixture_lens() -> Lens {
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
        }
    }

    fn fixture(camera: Camera) -> Reframe {
        Reframe::new(&fixture_lens(), FRAME, camera, 1.0, false)
    }

    #[track_caller]
    fn near(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} is not within {tolerance} of {expected}"
        );
    }

    /// The sanity check the model has to pass before any pixel is believed:
    /// the middle of the view looks along the lens axis, and the lens axis
    /// is the principal point.
    ///
    /// Not exact, because the lens is not mounted exactly on the body axis:
    /// 0.103 degrees of yaw and 0.07 degrees of pitch tilt it by 0.125
    /// degrees, and near the axis the model's effective focal length is
    /// `fx / (1 + xi)` = 1106 px/rad, so 2.4 px.
    #[test]
    fn the_view_axis_lands_on_the_principal_point() {
        let reframe = fixture(Camera::default());
        let landing = reframe.project(reframe.view_ray([0.5, 0.5]));

        assert!(landing.inside);
        near(landing.pixel[0], 1918.94, 3.0);
        near(landing.pixel[1], 1927.21, 3.0);
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
        let landing = reframe.project([1.0, 0.0, 0.0]);

        assert!(landing.inside);
        near(radius(&reframe, landing), 1802.0, 8.0);
    }

    /// And a ray past the lens's coverage does not, so the shader has
    /// something to paint grey on. The polynomial happily returns a
    /// coordinate for it; only the circle test rejects it.
    #[test]
    fn a_ray_past_the_lens_lands_outside_the_image_circle() {
        let reframe = fixture(Camera::default());
        let (s, c) = 120f32.to_radians().sin_cos();
        let landing = reframe.project([s, 0.0, c]);

        assert!(!landing.inside);
        near(radius(&reframe, landing), 2038.0, 8.0);
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
        let anchor = before.project(before.view_ray([0.5, 0.5]));

        let mut dragged = camera;
        dragged.aim(camera.look([0.5, 0.5], 1.0), [0.6, 0.5], 1.0);
        assert!(dragged.yaw < 0.0, "dragging right turns the view left");

        let after = fixture(dragged);
        let moved = after.project(after.view_ray([0.6, 0.5]));

        near(moved.pixel[0], anchor.pixel[0], 0.05);
        near(moved.pixel[1], anchor.pixel[1], 0.05);
    }

    /// The same for the vertical axis, which is the one whose sign is easy
    /// to get backwards: dragging down shows more sky.
    #[test]
    fn a_vertical_drag_carries_the_content_with_the_cursor() {
        let camera = Camera::default();
        let before = fixture(camera);
        let anchor = before.project(before.view_ray([0.5, 0.5]));

        let mut dragged = camera;
        dragged.aim(camera.look([0.5, 0.5], 1.0), [0.5, 0.6], 1.0);
        assert!(dragged.pitch > 0.0, "dragging down looks up");

        let after = fixture(dragged);
        let moved = after.project(after.view_ray([0.5, 0.6]));

        near(moved.pixel[0], anchor.pixel[0], 0.05);
        near(moved.pixel[1], anchor.pixel[1], 0.05);
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
        let anchor = before.project(before.view_ray(from));
        assert!(anchor.inside, "grabbed a pixel the lens does not have");

        let mut dragged = camera;
        dragged.aim(camera.look(from, aspect), to, aspect);

        let after = fixture(dragged);
        let moved = after.project(after.view_ray(to));

        near(moved.pixel[0], anchor.pixel[0], 1.0);
        near(moved.pixel[1], anchor.pixel[1], 1.0);
    }

    /// The datum, pinned: on a lens rolled a quarter turn, the top of the
    /// output lands above the principal point rather than beside it.
    /// Dropping `ROLL_DATUM_DEG` swaps those two, which is exactly the
    /// quarter turn the frames in docs/research/insv-format.md 4.8 ruled
    /// out.
    #[test]
    fn roll_is_measured_from_the_frames_horizontal_axis() {
        let reframe = fixture(Camera::default());
        let top = reframe.project(reframe.view_ray([0.5, 0.1]));

        assert!(top.inside);
        assert!(top.pixel[1] < reframe.cy - 100.0, "{top:?}");
        near(top.pixel[0], reframe.cx, 40.0);
    }

    /// Issue #30: the ray straight out the back of the camera projects onto
    /// the principal point itself, which is as far inside the image circle as
    /// a pixel can get. Nothing but the domain test rejects it.
    #[test]
    fn a_ray_from_straight_behind_is_not_in_the_picture() {
        let reframe = fixture(Camera::default());
        let landing = reframe.project([0.0, 0.0, -1.0]);

        assert!(!landing.inside);
        assert!(radius(&reframe, landing) < reframe.image_radius);
        // The principal point itself, give or take the eighth of a degree the
        // lens is mounted off the body axis.
        near(radius(&reframe, landing), 0.0, 10.0);
    }

    /// And the picture is one cap around the axis, not a cap and a ghost.
    /// Swept from the axis to straight backward, `inside` goes off once and
    /// stays off. On the radius test alone it came back on at 131.5 degrees
    /// and stayed on all the way to 180, which is the fisheye the owner saw
    /// hanging behind the view.
    #[test]
    fn the_picture_stops_once() {
        let reframe = fixture(Camera::default());
        let mut edge = None;

        for step in 0..=1800 {
            let theta = step as f32 * 0.1;
            let (s, c) = theta.to_radians().sin_cos();
            match (reframe.project([s, 0.0, c]).inside, edge) {
                (false, None) => edge = Some(theta),
                (true, Some(stopped)) => {
                    panic!("the picture stopped at {stopped} degrees and came back at {theta}")
                }
                _ => {}
            }
        }

        // Where the model reaches the image circle, which is what decides
        // how much picture there is: 97.5 degrees, as the note on
        // `image_radius` says.
        near(edge.expect("the picture never stopped"), 97.5, 0.2);
    }

    /// The size the WGSL struct rounds up to, which is what the bind group
    /// declares as `min_binding_size`: pipeline creation is where a
    /// disagreement between the two definitions surfaces.
    #[test]
    fn the_uniform_block_is_the_size_wgsl_lays_it_out() {
        assert_eq!(std::mem::size_of::<Reframe>(), 128);
    }

    fn radius(reframe: &Reframe, landing: Landing) -> f32 {
        norm([landing.pixel[0] - reframe.cx, landing.pixel[1] - reframe.cy])
    }
}
