//! Where the view points, and what the mouse does to it.
//!
//! No iced in this file: `src/widget.rs` turns events into these calls. The
//! rules themselves are arithmetic, and arithmetic is testable without a
//! window.

use std::f32::consts::PI;

use super::projection::{normalize, view_ray, world_ray};

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

/// How much bearing a grabbed direction has to have before a drag turns the
/// view by it. The world vertical is the one direction with no bearing at
/// all, and a millionth of a radian either side of it is thousands of times
/// finer than a pixel: nothing but the pole itself trips this.
const BEARING_FLOOR: f32 = 1e-6;

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
    /// The world direction a point of the output looks along. `uv` runs 0 to
    /// 1 across the output, y down, and `aspect` is its width over its
    /// height.
    ///
    /// World here is the body frame the lens's mounting is measured against:
    /// x right, y **down**, z forward. Its y is the vertical the view yaws
    /// about and never rolls about, so a direction's height above the horizon
    /// is `-y` and its bearing is `atan2(x, z)`.
    pub fn look(&self, uv: [f32; 2], aspect: f32) -> [f32; 3] {
        normalize(world_ray(*self, view_ray(uv, self.tan_half_fov(), aspect)))
    }

    /// Turn the view until `direction` lands at `uv`.
    ///
    /// Grab-the-world, solved rather than stepped. Stepping yaw and pitch
    /// along with the cursor is only grab-the-world near the middle of the
    /// view: near the pole a yaw turns about an axis nearly along the view
    /// ray, so it spins the picture instead of panning it and the grabbed
    /// point slides out from under the cursor (issue #29). Two unknowns and a
    /// direction under a cursor is two equations, so there is no need to step
    /// at all. Height above the horizon fixes the pitch, and bearing then
    /// fixes the yaw.
    ///
    /// Exact wherever a level horizon can hold the direction there at all.
    /// Where it cannot, the pitch clamps: that leaves the direction on the
    /// cursor's own meridian but short of it, so the drag reads as a wall
    /// rather than as a slip.
    pub fn aim(&mut self, direction: [f32; 3], uv: [f32; 2], aspect: f32) {
        let ray = normalize(view_ray(uv, self.tan_half_fov(), aspect));

        // Pitch turns the view in one plane and yaw turns that plane about
        // the world vertical, so the cursor's ray splits in two: `sideways`
        // is the part of it no pitch can raise, and `across` is the length of
        // what is left, which is the plane pitch works in. `rise` is where in
        // that plane the ray already sits, measured from the view axis.
        let sideways = ray[0];
        let across = ray[1].hypot(ray[2]);
        let rise = (-ray[1]).atan2(ray[2]);

        // Height above the horizon fixes the tilt, and the tilt fixes the
        // pitch. A ray `rise` off an axis pitched to `p` sits
        // `across * sin(rise + p)` above the horizon; the direction sits at
        // `-direction.y`, y being down. Asking for more than `across` asks
        // for a direction closer to the pole than this ray reaches without
        // rolling, and that clamp, with the pitch limit under it, is the
        // whole of the degeneracy.
        let sine = (-direction[1] / across).clamp(-1.0, 1.0);
        let tilt = nearest_tilt(sine, self.held_tilt(direction));
        self.pitch = (tilt - rise).clamp(-PITCH_LIMIT, PITCH_LIMIT);

        // Bearing fixes the yaw, at the tilt the height asked for rather than
        // the one the pitch limit allowed: taking it from the clamped pitch
        // instead swings the view half a turn the moment a stopped drag
        // carries the cursor over the pole, because the cursor's own bearing
        // reverses there and the direction's does not. Read as one complex
        // quotient, so the answer arrives already wrapped into (-pi, pi] and
        // its length is how well defined the two bearings are. Only the world
        // vertical has none, and there the yaw the view holds is as good as
        // any.
        let along = across * tilt.cos();
        let turn = [
            direction[0] * along - direction[2] * sideways,
            direction[2] * along + direction[0] * sideways,
        ];
        if turn[0].hypot(turn[1]) > BEARING_FLOOR {
            self.yaw = turn[0].atan2(turn[1]);
        }
    }

    /// Zoom by scroll steps, positive being a scroll away from the user.
    pub fn zoom(&mut self, steps: f32) {
        self.fov = (self.fov * (-steps * ZOOM_PER_STEP).exp()).clamp(FOV_MIN, FOV_MAX);
    }

    /// The tilt `direction` is at in the view as it stands, which is which of
    /// the two tilts with its height above the horizon the drag is already
    /// on.
    fn held_tilt(&self, direction: [f32; 3]) -> f32 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        (-direction[1]).atan2(direction[0] * sin_yaw + direction[2] * cos_yaw)
    }

    fn tan_half_fov(&self) -> f32 {
        (self.fov * 0.5).tan()
    }
}

/// The tilt with this sine lying nearest the one the drag is already at.
///
/// A view pitched close to the vertical sees past the pole, and content past
/// it solves at a tilt outside the quarter turn `asin` answers in. Both tilts
/// really do put the direction under the cursor, the far one by pitching the
/// other way through a view turned around, so this is not a choice about
/// correctness: it is the choice between following the drag and flipping.
fn nearest_tilt(sine: f32, held: f32) -> f32 {
    let principal = sine.asin();
    // The two mirrored tilts are `pi - principal` and `-pi - principal`, and
    // only the one on the held tilt's side of zero can be the nearer.
    let mirrored = PI.copysign(principal + held) - principal;
    if (mirrored - held).abs() < (principal - held).abs() {
        mirrored
    } else {
        principal
    }
}

/// The shader widget's state: a [`Camera`], and the world direction a held
/// drag has hold of.
#[derive(Clone, Copy, Debug, Default)]
pub struct Viewpoint {
    camera: Camera,
    anchor: Option<[f32; 3]>,
}

impl Viewpoint {
    pub fn camera(&self) -> Camera {
        self.camera
    }

    pub fn is_dragging(&self) -> bool {
        self.anchor.is_some()
    }

    /// Take hold of whatever the cursor is over. A direction, and not a
    /// cursor position: a position only says how far the last move went, and
    /// the same move means a different turn depending on where in the view it
    /// happens.
    pub fn grab(&mut self, uv: [f32; 2], aspect: f32) {
        self.anchor = Some(self.camera.look(uv, aspect));
    }

    pub fn release(&mut self) {
        self.anchor = None;
    }

    /// Continue a drag to a new cursor position. `true` when the camera
    /// moved, which is the caller's cue to ask for a redraw.
    ///
    /// A move with no grab held moves nothing and arms nothing: [`grab`] is
    /// the only thing that sets the anchor, because an anchor armed by a move
    /// on its way past panned the view with no button down (issue #26).
    ///
    /// [`grab`]: Self::grab
    pub fn drag_to(&mut self, uv: [f32; 2], aspect: f32) -> bool {
        let Some(anchor) = self.anchor else {
            return false;
        };
        let parked = self.camera;
        self.camera.aim(anchor, uv, aspect);
        self.camera != parked
    }

    /// Zoom by scroll steps, with the cursor at `uv`.
    ///
    /// A held drag takes hold again at the new field of view. The direction
    /// under the cursor moved when the view widened, and a drag still solving
    /// for the old one would jump the picture on its next move.
    pub fn zoom(&mut self, steps: f32, uv: [f32; 2], aspect: f32) {
        self.camera.zoom(steps);
        if let Some(anchor) = &mut self.anchor {
            *anchor = self.camera.look(uv, aspect);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use super::*;

    const MIDDLE: [f32; 2] = [0.5, 0.5];

    /// A drag is exact when the direction it grabbed comes back within a
    /// pixel of the cursor, so the tolerance is what a pixel of a 1000 px
    /// wide output subtends at this field of view.
    fn pixel(camera: Camera) -> f32 {
        (2.0 * camera.tan_half_fov() / 1000.0).atan()
    }

    /// The angle between two unit directions, by cross and dot rather than by
    /// `acos` alone: near zero an `acos` reads its answer off the flat top of
    /// the cosine, and an f32 there cannot resolve an angle under about a
    /// third of a milliradian, which is the very range this file is measured
    /// in.
    fn angle_between(a: [f32; 3], b: [f32; 3]) -> f32 {
        let cross = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let dot: f32 = (0..3).map(|i| a[i] * b[i]).sum();
        (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2])
            .sqrt()
            .atan2(dot)
    }

    /// How far the drag left the grabbed direction from the cursor.
    fn slip(camera: Camera, direction: [f32; 3], uv: [f32; 2], aspect: f32) -> f32 {
        angle_between(camera.look(uv, aspect), direction)
    }

    fn degrees(camera: Camera) -> (f32, f32) {
        (camera.yaw.to_degrees(), camera.pitch.to_degrees())
    }

    #[test]
    fn dragging_right_turns_the_camera_left() {
        let mut viewpoint = Viewpoint::default();
        viewpoint.grab(MIDDLE, 1.0);
        assert!(viewpoint.drag_to([0.6, 0.5], 1.0));
        assert!(viewpoint.camera().yaw < 0.0);
    }

    /// Dragging down shows more sky: the axis whose sign is easiest to get
    /// backwards.
    #[test]
    fn dragging_down_looks_up() {
        let mut viewpoint = Viewpoint::default();
        viewpoint.grab(MIDDLE, 1.0);
        assert!(viewpoint.drag_to([0.5, 0.6], 1.0));
        assert!(viewpoint.camera().pitch > 0.0);
    }

    /// One drag can only ask for so much, so this is a hundred of them: grab
    /// the middle, haul it to the bottom, let go, grab the middle again.
    #[test]
    fn the_pitch_stops_short_of_straight_up() {
        let mut viewpoint = Viewpoint::default();
        for _ in 0..100 {
            viewpoint.grab(MIDDLE, 1.0);
            viewpoint.drag_to([0.5, 1.0], 1.0);
            viewpoint.release();
        }
        assert_eq!(viewpoint.camera().pitch, PITCH_LIMIT);
    }

    /// A full turn of dragging comes back to where it started rather than
    /// counting up: the yaw is read off an `atan2`, which cannot leave
    /// (-pi, pi].
    #[test]
    fn yaw_wraps_instead_of_growing() {
        let mut viewpoint = Viewpoint::default();
        for _ in 0..400 {
            viewpoint.grab(MIDDLE, 1.0);
            viewpoint.drag_to([0.9, 0.5], 1.0);
            viewpoint.release();
        }
        assert!(viewpoint.camera().yaw.abs() <= PI);
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

    #[test]
    fn a_drag_needs_a_grab_first() {
        let mut viewpoint = Viewpoint::default();
        assert!(!viewpoint.drag_to([0.6, 0.5], 1.0));
        assert_eq!(viewpoint.camera(), Camera::default());

        viewpoint.grab(MIDDLE, 1.0);
        assert!(viewpoint.drag_to([0.6, 0.5], 1.0));
        assert_ne!(viewpoint.camera(), Camera::default());
    }

    /// Issue #26. One move with nothing held used to arm the anchor, so the
    /// move after it panned: the second call is the whole test.
    #[test]
    fn a_move_with_nothing_held_arms_nothing() {
        let mut viewpoint = Viewpoint::default();

        for x in 0..10 {
            assert!(!viewpoint.drag_to([x as f32 / 10.0, 0.5], 1.0));
        }

        assert!(!viewpoint.is_dragging());
        assert_eq!(viewpoint.camera(), Camera::default());
    }

    /// Releasing outside the widget must not leave the camera glued to the
    /// cursor: the next press starts a fresh drag from wherever it lands.
    #[test]
    fn releasing_ends_the_drag() {
        let mut viewpoint = Viewpoint::default();
        viewpoint.grab(MIDDLE, 1.0);
        assert!(viewpoint.drag_to([0.6, 0.5], 1.0));
        let parked = viewpoint.camera();
        viewpoint.release();

        assert!(!viewpoint.is_dragging());
        assert!(!viewpoint.drag_to([0.9, 0.9], 1.0));
        assert!(!viewpoint.drag_to([0.1, 0.9], 1.0));
        assert_eq!(viewpoint.camera(), parked);
    }

    /// Issue #29's whole point, over a grid of view states, grab points and
    /// drop points: wherever a level horizon can hold the grabbed direction
    /// under the cursor, it is under the cursor.
    ///
    /// Where it cannot, one of the two limits is what says so and the
    /// direction is asked for no more: either it is nearer the pole than the
    /// cursor's own ray reaches, or the pitch it wants is past the limit.
    #[test]
    fn the_grabbed_direction_stays_under_the_cursor() {
        let places: Vec<[f32; 2]> = [0.02, 0.3, 0.5, 0.72, 0.98]
            .iter()
            .flat_map(|&x| [0.02, 0.3, 0.5, 0.72, 0.98].map(|y| [x, y]))
            .collect();
        let mut exact = 0;
        let mut clamped = 0;

        for yaw in [-2.9, -0.4, 0.0, 1.1, 3.0] {
            for pitch in [-1.55, -1.2, -0.6, 0.0, 0.85, 1.5] {
                for fov in [FOV_MIN, 1.0, FOV_MAX] {
                    for aspect in [0.6, 1.0, 16.0 / 9.0] {
                        let camera = Camera { yaw, pitch, fov };
                        for &from in &places {
                            let direction = camera.look(from, aspect);
                            for &to in &places {
                                let mut aimed = camera;
                                aimed.aim(direction, to, aspect);

                                let short = reach(direction, to, camera, aspect) < 0.0;
                                let stopped = aimed.pitch.abs() >= PITCH_LIMIT;
                                if short || stopped {
                                    clamped += 1;
                                    continue;
                                }
                                exact += 1;
                                let slip = slip(aimed, direction, to, aspect);
                                assert!(
                                    slip <= pixel(camera),
                                    "{:?} slipped {slip} rad aiming {direction:?} from \
                                     {from:?} to {to:?} at aspect {aspect}",
                                    degrees(aimed),
                                );
                            }
                        }
                    }
                }
            }
        }

        assert!(
            exact > 10_000,
            "{exact} solvable cases is too few to mean much"
        );
        assert!(clamped > 0, "the grid never reached the degenerate cases");
    }

    /// How much angle a level horizon has to spare in putting `direction` at
    /// `uv`: the direction is this far from the pole, and the cursor's own
    /// ray leans this far off the plane the pole lives in. Negative is a
    /// direction no view without roll can put there.
    fn reach(direction: [f32; 3], uv: [f32; 2], camera: Camera, aspect: f32) -> f32 {
        let ray = normalize(view_ray(uv, camera.tan_half_fov(), aspect));
        let from_pole = FRAC_PI_2 - direction[1].abs().asin();
        from_pole - ray[0].abs().asin()
    }

    /// The pilot's body: a view pitched most of the way down sees past the
    /// nadir along the bottom of the output, and content past it solves at a
    /// pitch `asin` cannot name. Dragging it has to follow the cursor while
    /// the pitch limit allows, then stop dead, and never flip to the mirrored
    /// view that fits equally well.
    #[test]
    fn content_past_the_nadir_follows_the_cursor_then_stops() {
        let aspect = 16.0 / 9.0;
        let start = Camera {
            yaw: 0.0,
            pitch: -80f32.to_radians(),
            fov: 100f32.to_radians(),
        };
        let direction = start.look([0.5, 0.95], aspect);
        assert!(direction[1] > 0.0, "grabbed something above the horizon");
        assert!(direction[2] < 0.0, "grabbed something in front of the view");

        let mut camera = start;
        let mut followed = 0;
        for step in 1..=90 {
            let to = [0.5, 0.95 - 0.01 * step as f32];
            camera.aim(direction, to, aspect);

            assert!(
                camera.yaw.abs() < 1e-3,
                "the view turned around: {:?}",
                degrees(camera)
            );
            assert!(camera.pitch >= -PITCH_LIMIT, "{:?}", degrees(camera));
            assert!(
                camera.pitch < -1.0,
                "the view came back up: {:?}",
                degrees(camera)
            );
            if slip(camera, direction, to, aspect) <= pixel(camera) {
                followed += 1;
            }
        }
        assert!(
            (10..80).contains(&followed),
            "{followed} of 90 steps followed the cursor"
        );
    }

    /// The world vertical itself has no bearing to hold, so a drag on it
    /// pitches and never spins.
    #[test]
    fn grabbing_the_pole_pitches_without_turning() {
        // The zenith sits on the vertical centre line, ten degrees above the
        // axis of a view pitched to eighty.
        let camera = Camera {
            yaw: 0.7,
            pitch: 80f32.to_radians(),
            fov: FRAC_PI_2,
        };
        let up = [0.5, 0.5 - 10f32.to_radians().tan() / 2.0];
        let direction = camera.look(up, 1.0);
        assert!(direction[1] < -0.9999, "{direction:?} is not straight up");

        for x in 0..=10 {
            for y in 0..=10 {
                let mut aimed = camera;
                aimed.aim(direction, [x as f32 / 10.0, y as f32 / 10.0], 1.0);
                assert_eq!(aimed.yaw, camera.yaw);
                assert!(aimed.pitch.abs() <= PITCH_LIMIT);
            }
        }
    }

    /// A cursor swept clean across the pole's own place on the output, with
    /// hold of a direction a degree away from it. The turn is fast, because
    /// hauling a point that close to the axis really does swing the world,
    /// but it stays a turn: level, finite, and inside the pitch limit.
    #[test]
    fn dragging_across_the_pole_stays_level_and_finite() {
        let mut camera = Camera {
            yaw: 0.0,
            pitch: 85f32.to_radians(),
            fov: FRAC_PI_2,
        };
        let direction = camera.look([0.51, 0.46], 1.0);

        for step in 0..=400 {
            let along = step as f32 / 400.0;
            camera.aim(direction, [along, 1.0 - along], 1.0);
            assert!(
                camera.yaw.is_finite() && camera.yaw.abs() <= PI,
                "{:?}",
                degrees(camera)
            );
            assert!(camera.pitch.abs() <= PITCH_LIMIT, "{:?}", degrees(camera));
        }
    }

    /// A scroll in the middle of a drag re-reads what the cursor is over, so
    /// the move after it is still a move from here. Without that the next
    /// move solves for a direction the cursor stopped being over the moment
    /// the view widened, and the picture jumps.
    #[test]
    fn zooming_mid_drag_keeps_the_grab() {
        let held = [0.6, 0.4];
        let mut viewpoint = Viewpoint::default();
        viewpoint.grab([0.8, 0.2], 1.6);
        viewpoint.drag_to(held, 1.6);

        viewpoint.zoom(3.0, held, 1.6);
        let zoomed = viewpoint.camera();
        viewpoint.drag_to(held, 1.6);

        let moved = viewpoint.camera();
        assert!(
            (moved.yaw - zoomed.yaw).abs() < 1e-4 && (moved.pitch - zoomed.pitch).abs() < 1e-4,
            "the cursor did not move but the view went from {:?} to {:?}",
            degrees(zoomed),
            degrees(moved),
        );
    }
}
