//! Where the view points, and what the mouse does to it.
//!
//! No iced in this file: `src/widget.rs` turns events into these calls. The
//! rules themselves are arithmetic, and arithmetic is testable without a
//! window.

use std::f32::consts::{PI, TAU};

/// Where the view points and how wide it is. Radians throughout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Right of the lens axis is positive.
    pub yaw: f32,
    /// Up is positive.
    pub pitch: f32,
    /// Horizontal. The vertical field of view is whatever the output's
    /// aspect ratio leaves.
    pub fov: f32,
}

/// The zoom range. Past about 110 degrees a rectilinear view stretches the
/// corners into nonsense, and under 20 degrees a 3840 px lens is being
/// magnified about 8x and has nothing left to show.
const FOV_MIN: f32 = 20.0 * PI / 180.0;
const FOV_MAX: f32 = 110.0 * PI / 180.0;

/// Straight up and straight down are where a yaw/pitch camera loses its
/// horizon, so the view stops just short of both.
const PITCH_LIMIT: f32 = 89.0 * PI / 180.0;

/// Field of view per scroll step, as a ratio. Multiplicative, so a notch
/// covers the same fraction of the range wherever it is used.
const ZOOM_PER_STEP: f32 = 0.12;

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            fov: 90.0 * PI / 180.0,
        }
    }
}

impl Camera {
    /// Pan by a cursor movement of `dx`, `dy` output pixels across an output
    /// `width` pixels wide.
    ///
    /// Grab-the-world: the content follows the cursor, so the camera turns
    /// the other way. The `atan` is what makes that literally true rather
    /// than approximately true; a linear degrees-per-pixel rate drifts from
    /// the cursor at wide fields of view, where the edge of the view is a
    /// long way from the middle in angle.
    pub fn pan(&mut self, dx: f32, dy: f32, width: f32) {
        let per_pixel = 2.0 * (self.fov * 0.5).tan() / width.max(1.0);
        self.yaw = wrap(self.yaw - (dx * per_pixel).atan());
        self.pitch = (self.pitch + (dy * per_pixel).atan()).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// Zoom by scroll steps, positive being a scroll away from the user.
    pub fn zoom(&mut self, steps: f32) {
        self.fov = (self.fov * (-steps * ZOOM_PER_STEP).exp()).clamp(FOV_MIN, FOV_MAX);
    }
}

/// Yaw back into (-pi, pi]. A drag that crosses behind the camera a few
/// hundred times would otherwise keep counting, and an f32 radian loses its
/// last useful bit somewhere past a thousand turns.
fn wrap(yaw: f32) -> f32 {
    (yaw + PI).rem_euclid(TAU) - PI
}

/// The shader widget's state: a [`Camera`], and where the cursor was when
/// the drag last moved.
#[derive(Clone, Copy, Debug, Default)]
pub struct Viewpoint {
    camera: Camera,
    anchor: Option<(f32, f32)>,
}

impl Viewpoint {
    pub fn camera(&self) -> Camera {
        self.camera
    }

    pub fn is_dragging(&self) -> bool {
        self.anchor.is_some()
    }

    pub fn grab(&mut self, x: f32, y: f32) {
        self.anchor = Some((x, y));
    }

    pub fn release(&mut self) {
        self.anchor = None;
    }

    /// Continue a drag to a new cursor position. `true` when the camera
    /// moved, which is the caller's cue to ask for a redraw.
    ///
    /// A move with no grab held must leave the anchor unset. `Option::replace`
    /// reads like the right tool and is not: it arms the anchor on its way
    /// past, so the move after it pans with no button down (issue #26).
    pub fn drag_to(&mut self, x: f32, y: f32, width: f32) -> bool {
        let Some((from_x, from_y)) = self.anchor else {
            return false;
        };
        self.anchor = Some((x, y));
        self.camera.pan(x - from_x, y - from_y, width);
        true
    }

    pub fn zoom(&mut self, steps: f32) {
        self.camera.zoom(steps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragging_right_turns_the_camera_left() {
        let mut camera = Camera::default();
        camera.pan(100.0, 0.0, 1000.0);
        assert!(camera.yaw < 0.0);
    }

    #[test]
    fn the_pitch_stops_short_of_straight_up() {
        let mut camera = Camera::default();
        for _ in 0..100 {
            camera.pan(0.0, 1000.0, 1000.0);
        }
        assert_eq!(camera.pitch, PITCH_LIMIT);
    }

    #[test]
    fn scrolling_away_narrows_the_field_of_view_and_stops() {
        let mut camera = Camera::default();
        camera.zoom(1.0);
        assert!(camera.fov < Camera::default().fov);

        for _ in 0..100 {
            camera.zoom(1.0);
        }
        assert_eq!(camera.fov, FOV_MIN);

        for _ in 0..200 {
            camera.zoom(-1.0);
        }
        assert_eq!(camera.fov, FOV_MAX);
    }

    /// A full turn of dragging comes back to where it started rather than
    /// counting up.
    #[test]
    fn yaw_wraps_instead_of_growing() {
        let mut camera = Camera::default();
        for _ in 0..400 {
            camera.pan(100.0, 0.0, 1000.0);
        }
        assert!(camera.yaw.abs() <= PI);
    }

    #[test]
    fn a_drag_needs_a_grab_first() {
        let mut viewpoint = Viewpoint::default();
        assert!(!viewpoint.drag_to(10.0, 10.0, 1000.0));
        assert_eq!(viewpoint.camera(), Camera::default());

        viewpoint.grab(10.0, 10.0);
        assert!(viewpoint.drag_to(20.0, 10.0, 1000.0));
        assert_ne!(viewpoint.camera(), Camera::default());
    }

    /// Issue #26. One move with nothing held used to arm the anchor, so the
    /// move after it panned: the second call is the whole test.
    #[test]
    fn a_move_with_nothing_held_arms_nothing() {
        let mut viewpoint = Viewpoint::default();

        for x in 0..10 {
            assert!(!viewpoint.drag_to(x as f32 * 20.0, 10.0, 1000.0));
        }

        assert!(!viewpoint.is_dragging());
        assert_eq!(viewpoint.camera(), Camera::default());
    }

    /// Releasing outside the widget must not leave the camera glued to the
    /// cursor: the next press starts a fresh drag from wherever it lands.
    #[test]
    fn releasing_ends_the_drag() {
        let mut viewpoint = Viewpoint::default();
        viewpoint.grab(10.0, 10.0);
        assert!(viewpoint.drag_to(20.0, 10.0, 1000.0));
        let parked = viewpoint.camera();
        viewpoint.release();

        assert!(!viewpoint.is_dragging());
        assert!(!viewpoint.drag_to(500.0, 500.0, 1000.0));
        assert!(!viewpoint.drag_to(600.0, 500.0, 1000.0));
        assert_eq!(viewpoint.camera(), parked);
    }
}
