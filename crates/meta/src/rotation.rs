//! The rotation algebra the trailer layer needs: a 3x3 matrix for the fixed
//! mountings, and a quaternion for the orientation that moves.
//!
//! Two types rather than one because they are used for different things. A
//! mounting is a constant and a matrix multiplies a vector by it in nine
//! products; an integrated orientation is stepped a million times per file
//! and renormalized on every step, and a quaternion is what stays a rotation
//! under that.
//!
//! `kjerag-render` has its own `Mat3` for the shader block, in `f32` columns
//! the way WGSL lays one out. This one is the `f64` half that the
//! calibration is read in, and [`Mat3::rows`] is how the two meet.

/// A 3x3 matrix, row major: `m[row][column]`, and `v_out = M * v_in`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3([[f64; 3]; 3]);

impl Mat3 {
    pub const IDENTITY: Self = Self([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    pub const fn new(rows: [[f64; 3]; 3]) -> Self {
        Self(rows)
    }

    /// Row major, which is how `kjerag-render` reads one into its own matrix.
    pub fn rows(self) -> [[f64; 3]; 3] {
        self.0
    }

    pub fn rot_x(angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        Self([[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]])
    }

    pub fn rot_y(angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        Self([[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]])
    }

    pub fn rot_z(angle: f64) -> Self {
        let (s, c) = angle.sin_cos();
        Self([[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]])
    }

    pub fn times(self, rhs: Self) -> Self {
        let mut out = [[0.0; 3]; 3];
        for (r, row) in out.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| self.0[r][k] * rhs.0[k][c]).sum();
            }
        }
        Self(out)
    }

    pub fn mul_vec(self, v: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|row| (0..3).map(|k| self.0[row][k] * v[k]).sum())
    }

    /// Which is the inverse, for a rotation.
    pub fn transpose(self) -> Self {
        Self(std::array::from_fn(|r| {
            std::array::from_fn(|c| self.0[c][r])
        }))
    }

    /// Positive for a rotation and negative for a rotation with a reflection
    /// in it. Only a test asks, and what it asks is whether an axis map read
    /// off a three-letter convention string is a rotation at all.
    pub fn determinant(self) -> f64 {
        let m = self.0;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }
}

/// A unit quaternion: `w + xi + yj + zk`, with the vector part kept as one
/// array because everything that touches it treats it as a vector.
///
/// `q.rotate(v)` takes `v` from the frame `q` is written in to the frame it
/// rotates into, and `a.times(b)` applies `b` first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub w: f64,
    pub v: [f64; 3],
}

impl Default for Quat {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Quat {
    pub const IDENTITY: Self = Self {
        w: 1.0,
        v: [0.0; 3],
    };

    /// The rotation a body turning at a constant rate makes in one step:
    /// `v` is the axis, and its length is the angle in radians.
    ///
    /// The small-angle case is not a special case for accuracy, it is one for
    /// arithmetic: `sin(a/2) / a` is 0/0 at a step with no rotation in it, and
    /// a step with no rotation in it is most of a file recorded on the ground.
    pub fn from_rotation_vector(v: [f64; 3]) -> Self {
        let angle = norm(v);
        if angle < 1e-12 {
            return Self::IDENTITY;
        }
        let (sin, cos) = (angle * 0.5).sin_cos();
        Self {
            w: cos,
            v: v.map(|c| c * sin / angle),
        }
    }

    /// The same rotation as an axis whose length is the angle in radians,
    /// which is what [`Self::from_rotation_vector`] reads.
    ///
    /// Rolling-shutter correction is the caller (issue #9): the turn the body
    /// makes across one frame's readout is scaled by where in the readout a
    /// row sits, and scaling wants a vector rather than a quaternion.
    pub fn rotation_vector(self) -> [f64; 3] {
        let length = norm(self.v);
        if length < 1e-12 {
            return [0.0; 3];
        }
        // `q` and `-q` are one rotation, and only one of the two is the short
        // way round: without this a turn of a degree logs as 359.
        let (w, v) = match self.w < 0.0 {
            true => (-self.w, self.v.map(std::ops::Neg::neg)),
            false => (self.w, self.v),
        };
        let angle = 2.0 * length.atan2(w);
        v.map(|axis| axis * angle / length)
    }

    /// A turn about the world vertical, which in this frame is +y (down). The
    /// yaw half of an orientation, and the only part of it a heading filter
    /// touches.
    pub fn about_down(angle: f64) -> Self {
        let (sin, cos) = (angle * 0.5).sin_cos();
        Self {
            w: cos,
            v: [0.0, sin, 0.0],
        }
    }

    /// This rotation after `rhs`.
    pub fn times(self, rhs: Self) -> Self {
        Self {
            w: self.w * rhs.w - dot(self.v, rhs.v),
            v: std::array::from_fn(|i| {
                self.w * rhs.v[i] + rhs.w * self.v[i] + cross(self.v, rhs.v)[i]
            }),
        }
    }

    /// The inverse, for a unit quaternion.
    pub fn conjugate(self) -> Self {
        Self {
            w: self.w,
            v: self.v.map(std::ops::Neg::neg),
        }
    }

    pub fn rotate(self, v: [f64; 3]) -> [f64; 3] {
        // v + 2 * q_v x (q_v x v + w * v), which is the form with no
        // intermediate matrix in it.
        let t = cross(self.v, v).map(|c| c * 2.0);
        std::array::from_fn(|i| v[i] + self.w * t[i] + cross(self.v, t)[i])
    }

    pub fn normalized(self) -> Self {
        let length = (self.w * self.w + dot(self.v, self.v)).sqrt();
        match length > 0.0 {
            true => Self {
                w: self.w / length,
                v: self.v.map(|c| c / length),
            },
            false => Self::IDENTITY,
        }
    }

    /// How far round the world vertical this orientation has turned.
    ///
    /// Any orientation splits into a turn about the vertical and a tilt off
    /// it, `q = about_down(heading) * tilt`, and the product of those two
    /// leaves `w` and the y component of the vector part carrying the heading
    /// alone: both come out multiplied by the tilt's own cosine, which
    /// divides out of the `atan2`. That is the whole derivation, and the
    /// [`about_down`] test is the check.
    ///
    /// Ill defined for a camera turned exactly upside down, where that cosine
    /// is zero. A camera hanging under a wing does not reach it.
    ///
    /// [`about_down`]: Self::about_down
    pub fn heading(self) -> f64 {
        2.0 * self.v[1].atan2(self.w)
    }

    /// The rotation as a matrix, which is what the shader block wants.
    pub fn matrix(self) -> Mat3 {
        Mat3(std::array::from_fn(|row| {
            let mut basis = [0.0; 3];
            basis[row] = 1.0;
            // Rows of R are R^T's columns, which are R^T applied to the basis:
            // the conjugate rotation of each axis.
            self.conjugate().rotate(basis)
        }))
    }

    /// Between two orientations, `t` of the way from this one to `other`.
    ///
    /// Normalized linear rather than spherical: the samples this interpolates
    /// between are a millisecond or two apart, where the two differ by less
    /// than a millionth of a degree, and `slerp` costs a trig pair per output
    /// row of a rolling-shutter pass (issue #9).
    pub fn nlerp(self, other: Self, t: f64) -> Self {
        // The two ends of a shortest path: `q` and `-q` are the same rotation,
        // and mixing the far representations interpolates the long way round.
        let other = match self.w * other.w + dot(self.v, other.v) < 0.0 {
            true => Self {
                w: -other.w,
                v: other.v.map(std::ops::Neg::neg),
            },
            false => other,
        };
        Self {
            w: self.w + (other.w - self.w) * t,
            v: std::array::from_fn(|i| self.v[i] + (other.v[i] - self.v[i]) * t),
        }
        .normalized()
    }

    /// The angle between two orientations, in radians. For tests and reports.
    pub fn angle_to(self, other: Self) -> f64 {
        let between = self.conjugate().times(other);
        2.0 * norm(between.v).min(1.0).asin().min(std::f64::consts::PI)
    }
}

pub fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

pub fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub fn norm(v: [f64; 3]) -> f64 {
    dot(v, v).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    #[track_caller]
    fn near(actual: [f64; 3], expected: [f64; 3], tolerance: f64) {
        for axis in 0..3 {
            assert!(
                (actual[axis] - expected[axis]).abs() <= tolerance,
                "{actual:?} is not within {tolerance} of {expected:?}"
            );
        }
    }

    #[test]
    fn a_quarter_turn_about_each_axis_moves_the_axes_it_should() {
        near(
            Mat3::rot_x(FRAC_PI_2).mul_vec([0.0, 1.0, 0.0]),
            [0.0, 0.0, 1.0],
            1e-12,
        );
        near(
            Mat3::rot_y(FRAC_PI_2).mul_vec([0.0, 0.0, 1.0]),
            [1.0, 0.0, 0.0],
            1e-12,
        );
        near(
            Mat3::rot_z(FRAC_PI_2).mul_vec([1.0, 0.0, 0.0]),
            [0.0, 1.0, 0.0],
            1e-12,
        );
    }

    #[test]
    fn a_rotation_matrix_undoes_itself_by_transpose() {
        let m = Mat3::rot_z(0.4)
            .times(Mat3::rot_y(-1.1))
            .times(Mat3::rot_x(0.2));
        near(
            m.transpose().mul_vec(m.mul_vec([0.3, -0.5, 0.8])),
            [0.3, -0.5, 0.8],
            1e-12,
        );
        assert!((m.determinant() - 1.0).abs() < 1e-12);
    }

    /// The quaternion and the matrix have to be the same rotation, or the
    /// integrated orientation and the shader block disagree about which way is
    /// up.
    #[test]
    fn a_quaternion_and_its_matrix_rotate_alike() {
        let q = Quat::from_rotation_vector([0.3, -0.7, 0.2]);
        for v in [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.4, 0.5, -0.6],
        ] {
            near(q.matrix().mul_vec(v), q.rotate(v), 1e-12);
        }
    }

    /// A body turning at a constant rate, stepped a thousand times, arrives
    /// where one big rotation would put it. This is the integration in
    /// `orientation.rs` with the filter switched off.
    #[test]
    fn many_small_steps_add_up_to_one_big_rotation() {
        let axis = [0.0, 0.0, 1.0];
        let mut q = Quat::IDENTITY;
        for _ in 0..1000 {
            q = q.times(Quat::from_rotation_vector(axis.map(|c| c * PI / 1000.0)));
        }
        assert!(q.angle_to(Quat::from_rotation_vector([0.0, 0.0, PI])) < 1e-9);
    }

    /// The heading split, which the yaw filter rests on: a turn about the
    /// world vertical times any tilt reads back the turn.
    #[test]
    fn the_heading_of_a_turn_and_a_tilt_is_the_turn() {
        for heading in [-3.0, -0.5, 0.0, 0.9, 3.0] {
            for tilt in [
                [0.0, 0.0, 0.0],
                [0.4, 0.0, 0.0],
                [0.0, 0.0, -0.8],
                [0.3, 0.0, 0.5],
            ] {
                let q = Quat::about_down(heading).times(Quat::from_rotation_vector(tilt));
                assert!(
                    (q.heading() - heading).abs() < 1e-9,
                    "{heading} from {tilt:?}"
                );
            }
        }
    }

    /// And taking the heading off leaves a tilt with none: the vertical is
    /// where it was, and the rotation left over turns nothing about it.
    #[test]
    fn removing_the_heading_leaves_a_tilt_alone() {
        let q = Quat::about_down(1.2).times(Quat::from_rotation_vector([0.4, 0.0, -0.3]));
        let tilt = Quat::about_down(-q.heading()).times(q);

        assert!(tilt.heading().abs() < 1e-9);
        assert!(tilt.v[1].abs() < 1e-9, "the tilt turns about the vertical");
    }

    #[test]
    fn an_interpolation_between_two_orientations_stays_a_rotation() {
        let a = Quat::from_rotation_vector([0.1, 0.2, -0.3]);
        let b = Quat::from_rotation_vector([0.15, 0.18, -0.2]);

        for step in 0..=10 {
            let t = f64::from(step) / 10.0;
            let mixed = a.nlerp(b, t);
            assert!((mixed.w * mixed.w + dot(mixed.v, mixed.v) - 1.0).abs() < 1e-12);
        }
        assert!(a.nlerp(b, 0.0).angle_to(a) < 1e-12);
        assert!(a.nlerp(b, 1.0).angle_to(b) < 1e-12);
    }

    /// The log and the exponential are each other's inverse, which is what
    /// rolling-shutter correction rests on: a turn read off the orientation
    /// track as a vector, scaled by a row's share of the readout, has to be
    /// the rotation that share of the turn.
    #[test]
    fn a_rotation_vector_survives_the_round_trip() {
        for v in [
            [0.0, 0.0, 0.0],
            [1e-9, 0.0, 0.0],
            [0.3, -0.7, 0.2],
            [0.0, 0.0, PI - 0.01],
        ] {
            let back = Quat::from_rotation_vector(v).rotation_vector();
            near(back, v, 1e-9);
        }
    }

    /// And the short way round is the one it reads: a turn a degree short of a
    /// full circle is a degree back, not 359 forward. The orientation track
    /// stores whichever representation the integration landed on, so the sign
    /// of `w` is not the caller's to control.
    #[test]
    fn a_rotation_vector_takes_the_short_way_round() {
        let turn = Quat::from_rotation_vector([0.0, 0.0, 0.02]);
        let far = Quat {
            w: -turn.w,
            v: turn.v.map(std::ops::Neg::neg),
        };

        near(far.rotation_vector(), [0.0, 0.0, 0.02], 1e-12);
    }

    /// `q` and `-q` are one rotation, and an interpolation that does not
    /// notice takes the long way round: half a degree apart, this used to come
    /// out 179.5 degrees from either end.
    #[test]
    fn an_interpolation_takes_the_short_way_round() {
        let a = Quat::from_rotation_vector([0.0, 0.0, 0.01]);
        let far = Quat {
            w: -a.w,
            v: a.v.map(std::ops::Neg::neg),
        };

        assert!(a.nlerp(far, 0.5).angle_to(a).to_degrees() < 0.3);
    }
}
