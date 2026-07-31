//! Where the view points, and what the mouse does to it.
//!
//! No iced in this file: `src/widget.rs` turns events into these calls. The
//! rules themselves are arithmetic, and arithmetic is testable without a
//! window.

use std::f32::consts::PI;

use super::projection::{fov_ceiling, normalize, view_ray, world_ray};

/// Where the view points and how wide it is. Radians throughout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Right of the lens axis is positive.
    pub yaw: f32,
    /// Up is positive.
    pub pitch: f32,
    /// Horizontal: the angle from the view axis out to the middle of the
    /// frame's left and right edges, twice. The vertical field of view is
    /// whatever the output's aspect ratio leaves.
    ///
    /// **Past a full turn it keeps meaning that** (issue #47). At 360 degrees
    /// the frame's own edges are half a turn out, which is as far as a
    /// direction goes, so the whole sphere is exactly as wide as the frame;
    /// wider than that and the frame reaches past the sphere altogether,
    /// which is the room the ball sits in.
    pub fov: f32,
}

/// The near end of the zoom: under 20 degrees a 3840 px lens is being
/// magnified about 8x and has nothing left to show. The far end is
/// `projection::fov_ceiling`, which depends on the window shape, because
/// fitting a round picture in a wide window is not the same as fitting it in
/// a square one.
const FOV_MIN: f32 = 20.0 * PI / 180.0;

/// Straight up and straight down are where a yaw/pitch camera loses its
/// horizon, so the view stops just short of both.
const PITCH_LIMIT: f32 = 89.0 * PI / 180.0;

/// Field of view per scroll step, as a ratio. Multiplicative, so a notch
/// covers the same fraction of the range wherever it is used.
const ZOOM_PER_STEP: f32 = 0.12;

/// Scroll steps one press of the zoom key is worth. A wheel notch is a small
/// adjustment under a cursor that is already pointing at something; a key
/// press has no cursor and wants to cross the range in a handful of presses.
const STEPS_PER_KEY: f32 = 3.0;

/// The middle of the output, which is where a keyboard is pointing.
const MIDDLE: [f32; 2] = [0.5, 0.5];

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
    ///
    /// `None` where the output is looking at nothing: the room around the
    /// ball at the far end of the zoom (issue #47), which a narrower view has
    /// none of. The projection answers that, not this: whatever map the view
    /// is in is the map this reads its rays from.
    pub fn look(&self, uv: [f32; 2], aspect: f32) -> Option<[f32; 3]> {
        Some(normalize(world_ray(*self, view_ray(uv, *self, aspect)?)))
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
    ///
    /// A cursor in the room around the ball (issue #47) is asking for a
    /// direction to be put where there are no directions, so the view holds
    /// still until it comes back over the picture.
    pub fn aim(&mut self, direction: [f32; 3], uv: [f32; 2], aspect: f32) {
        let Some(ray) = view_ray(uv, *self, aspect) else {
            return;
        };
        let ray = normalize(ray);

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
        let tilt = match across > 0.0 {
            true => nearest_tilt(
                (-direction[1] / across).clamp(-1.0, 1.0),
                self.held_tilt(direction),
            ),
            // A cursor a quarter turn out along the frame's own horizontal
            // axis, which only a view past 180 degrees has any of (issue
            // #47): its ray is the axis pitch turns about, so no pitch moves
            // it and the height solve is zero over zero. The pitch the view
            // holds is as good as any, and the bearing below still answers.
            false => self.pitch + rise,
        };
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
    ///
    /// One scroll out of the far end is one scroll back in, because the range
    /// is multiplicative and both ends are clamps rather than states: the
    /// notch that lands on the ceiling is undone by the notch after it.
    pub fn zoom(&mut self, steps: f32, aspect: f32) {
        self.fov = (self.fov * (-steps * ZOOM_PER_STEP).exp()).clamp(FOV_MIN, fov_ceiling(aspect));
    }

    /// The tilt `direction` is at in the view as it stands, which is which of
    /// the two tilts with its height above the horizon the drag is already
    /// on.
    fn held_tilt(&self, direction: [f32; 3]) -> f32 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        (-direction[1]).atan2(direction[0] * sin_yaw + direction[2] * cos_yaw)
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

/// A view change with no cursor behind it: the `View` menu and its keys.
///
/// The mouse reaches the camera through the widget, which is where iced keeps
/// it. A menu item has no cursor to anchor a zoom on and no widget to send an
/// event to, so the shell leaves one of these on the [`Scene`] instead and the
/// next redraw applies it.
///
/// [`Scene`]: super::Scene
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Nudge {
    ZoomIn,
    ZoomOut,
    /// Yaw, pitch and field of view together, in one action: `Ctrl+0` in
    /// cosmic-files and cosmic-edit resets only zoom, because zoom is all
    /// those apps have (docs/UI.md open question 4).
    Reset,
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
    ///
    /// A press on the room around the ball takes hold of nothing, because
    /// there is nothing there to take hold of.
    pub fn grab(&mut self, uv: [f32; 2], aspect: f32) {
        self.anchor = self.camera.look(uv, aspect);
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
        self.camera.zoom(steps, aspect);
        if self.anchor.is_some() {
            self.anchor = self.camera.look(uv, aspect);
        }
    }

    /// Apply a [`Nudge`], which zooms about the middle of the view because
    /// that is where a keyboard is pointing.
    pub fn nudge(&mut self, nudge: Nudge, aspect: f32) {
        match nudge {
            Nudge::ZoomIn => self.zoom(STEPS_PER_KEY, MIDDLE, aspect),
            Nudge::ZoomOut => self.zoom(-STEPS_PER_KEY, MIDDLE, aspect),
            // The drag keeps its hold: the direction it grabbed is not where
            // the reset leaves the view, and re-anchoring here would haul the
            // picture straight back on the next move.
            Nudge::Reset => {
                self.camera = Camera::default();
                self.anchor = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use super::super::projection::FOV_FLAT;
    use super::*;

    /// A drag is exact when the direction it grabbed comes back within a
    /// pixel of the cursor, so the tolerance is what a pixel of a 1000 px
    /// wide output subtends **there**.
    ///
    /// Measured off the projection rather than off the field of view, because
    /// past issue #47's threshold the two are not the same question: a
    /// bent map spends its pixels unevenly, and the ball's rim holds a whole
    /// turn of azimuth in a pixel or two. A place with no ray next to it is a
    /// place the drag cannot be exact at, and half a turn says so.
    fn pixel(camera: Camera, uv: [f32; 2], aspect: f32) -> f32 {
        let step = [uv[0] + 0.001, uv[1]];
        match (camera.look(uv, aspect), camera.look(step, aspect)) {
            (Some(here), Some(along)) => angle_between(here, along),
            _ => PI,
        }
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

    /// How far the drag left the grabbed direction from the cursor. Half a
    /// turn where the cursor is over no direction at all, which is as far
    /// apart as two directions get.
    fn slip(camera: Camera, direction: [f32; 3], uv: [f32; 2], aspect: f32) -> f32 {
        camera
            .look(uv, aspect)
            .map_or(PI, |under| angle_between(under, direction))
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
        let aspect = 16.0 / 9.0;
        let mut camera = Camera::default();
        camera.zoom(1.0, aspect);
        assert!(camera.fov < Camera::default().fov);

        for _ in 0..100 {
            camera.zoom(1.0, aspect);
        }
        assert_eq!(camera.fov, FOV_MIN);

        for _ in 0..200 {
            camera.zoom(-1.0, aspect);
        }
        assert_eq!(camera.fov, fov_ceiling(aspect));
    }

    /// Both ends are clamps and not states, so the notch that lands on one is
    /// undone by the notch after it: a scroll out to the ball and back in is
    /// one gesture, not a gesture and a stuck view (issue #47).
    #[test]
    fn the_scroll_comes_back_from_the_ball() {
        let aspect = 16.0 / 9.0;
        let mut camera = Camera::default();
        for _ in 0..40 {
            camera.zoom(-1.0, aspect);
        }
        assert_eq!(camera.fov, fov_ceiling(aspect));

        let mut widths = vec![camera.fov];
        for _ in 0..40 {
            camera.zoom(1.0, aspect);
            widths.push(camera.fov);
        }
        assert!(
            widths
                .windows(2)
                .all(|pair| pair[1] < pair[0] || pair[1] == FOV_MIN),
            "a scroll in did not narrow the view",
        );
        assert_eq!(camera.fov, FOV_MIN);
    }

    /// A wider window has to zoom out further before a round picture clears
    /// its top and bottom, so the far end depends on the window and the near
    /// end does not.
    #[test]
    fn a_wider_window_zooms_out_further() {
        let ceilings: Vec<f32> = [1.0, 4.0 / 3.0, 16.0 / 9.0, 21.0 / 9.0]
            .iter()
            .map(|&aspect| fov_ceiling(aspect))
            .collect();
        assert!(
            ceilings.windows(2).all(|pair| pair[1] > pair[0]),
            "{ceilings:?} is not one ceiling per window shape",
        );
        // Portrait windows are held by their width, which is the same
        // question turned on its side and the same answer.
        assert_eq!(fov_ceiling(0.5), fov_ceiling(1.0));
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
                for aspect in [0.6, 1.0, 16.0 / 9.0] {
                    // The whole range and not the old one: the flat views, the
                    // threshold the bend starts at, stereographic, and the ball
                    // itself (issue #47).
                    let ceiling = fov_ceiling(aspect);
                    for fov in [FOV_MIN, 1.0, FOV_FLAT, 2.0 * FOV_FLAT, ceiling] {
                        let camera = Camera { yaw, pitch, fov };
                        for &from in &places {
                            let Some(direction) = camera.look(from, aspect) else {
                                continue;
                            };
                            for &to in &places {
                                let mut aimed = camera;
                                aimed.aim(direction, to, aspect);

                                let short = reach(direction, to, camera, aspect)
                                    .is_none_or(|spare| spare < 0.0);
                                let stopped = aimed.pitch.abs() >= PITCH_LIMIT;
                                if short || stopped {
                                    clamped += 1;
                                    continue;
                                }
                                exact += 1;
                                let slip = slip(aimed, direction, to, aspect);
                                assert!(
                                    slip <= pixel(camera, to, aspect),
                                    "{:?} at fov {:.0} slipped {slip} rad aiming \
                                     {direction:?} from {from:?} to {to:?} at aspect {aspect}",
                                    degrees(aimed),
                                    fov.to_degrees(),
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
    /// direction no view without roll can put there, and `None` is a cursor
    /// over no direction at all, which is nothing to spare either.
    fn reach(direction: [f32; 3], uv: [f32; 2], camera: Camera, aspect: f32) -> Option<f32> {
        let ray = normalize(view_ray(uv, camera, aspect)?);
        let from_pole = FRAC_PI_2 - direction[1].abs().asin();
        Some(from_pole - ray[0].abs().asin())
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
        let direction = start
            .look([0.5, 0.95], aspect)
            .expect("a flat view is all sphere");
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
            if slip(camera, direction, to, aspect) <= pixel(camera, to, aspect) {
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
        let direction = camera.look(up, 1.0).expect("a flat view is all sphere");
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
        let direction = camera
            .look([0.51, 0.46], 1.0)
            .expect("a flat view is all sphere");

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

    /// The `View` menu's three items: two that change the field of view and
    /// one that puts the whole camera back where it opened.
    #[test]
    fn a_nudge_zooms_about_the_middle_and_resets_everything() {
        let mut viewpoint = Viewpoint::default();
        viewpoint.nudge(Nudge::ZoomIn, 1.6);
        assert!(viewpoint.camera().fov < Camera::default().fov);

        viewpoint.nudge(Nudge::ZoomOut, 1.6);
        assert!((viewpoint.camera().fov - Camera::default().fov).abs() < 1e-4);

        viewpoint.grab([0.2, 0.8], 1.6);
        viewpoint.drag_to([0.7, 0.3], 1.6);
        viewpoint.nudge(Nudge::ZoomIn, 1.6);
        assert_ne!(viewpoint.camera(), Camera::default());

        viewpoint.nudge(Nudge::Reset, 1.6);
        assert_eq!(viewpoint.camera(), Camera::default());
        // A drag still held has let go of a direction that is no longer under
        // the cursor, so the next move must not haul the view back to it.
        assert!(!viewpoint.is_dragging());
    }

    /// The ball can be grabbed and turned like anything else (issue #47): the
    /// drag is one solve over the whole range, and the projection it inverts
    /// is whichever one the view is in.
    #[test]
    fn the_ball_turns_under_the_cursor() {
        let aspect = 16.0 / 9.0;
        let mut viewpoint = Viewpoint::default();
        for _ in 0..40 {
            viewpoint.zoom(-1.0, MIDDLE, aspect);
        }
        assert_eq!(viewpoint.camera().fov, fov_ceiling(aspect));

        // Across the middle of the ball, which at this zoom is a couple of
        // tenths of the frame either side of the centre.
        let from = [0.46, 0.52];
        viewpoint.grab(from, aspect);
        let held = viewpoint
            .camera()
            .look(from, aspect)
            .expect("the middle of the ball is picture");
        for step in 1..=20 {
            let to = [from[0] + 0.004 * step as f32, from[1] - 0.002 * step as f32];
            assert!(viewpoint.drag_to(to, aspect), "the ball did not turn");
            let camera = viewpoint.camera();
            assert!(camera.yaw.is_finite() && camera.pitch.is_finite());
            let slip = slip(camera, held, to, aspect);
            assert!(
                slip <= pixel(camera, to, aspect),
                "the ball slipped {slip} rad under the cursor at step {step}",
            );
        }
    }

    /// The room around the ball holds nothing, so a press on it takes hold of
    /// nothing and a drag into it holds the view still rather than hauling it
    /// somewhere arbitrary.
    #[test]
    fn the_room_around_the_ball_is_not_a_handle() {
        let aspect = 16.0 / 9.0;
        let mut viewpoint = Viewpoint::default();
        for _ in 0..40 {
            viewpoint.zoom(-1.0, MIDDLE, aspect);
        }
        let corner = [0.01, 0.01];
        assert!(
            viewpoint.camera().look(corner, aspect).is_none(),
            "the corner of the ball view is still picture",
        );

        viewpoint.grab(corner, aspect);
        assert!(!viewpoint.is_dragging());
        assert!(!viewpoint.drag_to([0.5, 0.5], aspect));

        // And a drag that starts on the ball and wanders off it stops rather
        // than jumping.
        viewpoint.grab([0.5, 0.5], aspect);
        assert!(viewpoint.is_dragging());
        viewpoint.drag_to([0.53, 0.5], aspect);
        let parked = viewpoint.camera();
        assert!(!viewpoint.drag_to(corner, aspect));
        assert_eq!(viewpoint.camera(), parked);
    }

    /// Ctrl+0 comes back from the ball in one press, which is the way out of
    /// the far end that does not need fourteen scrolls.
    #[test]
    fn the_default_view_comes_back_from_the_ball() {
        let aspect = 16.0 / 9.0;
        let mut viewpoint = Viewpoint::default();
        for _ in 0..40 {
            viewpoint.nudge(Nudge::ZoomOut, aspect);
        }
        assert_eq!(viewpoint.camera().fov, fov_ceiling(aspect));

        viewpoint.nudge(Nudge::Reset, aspect);
        assert_eq!(viewpoint.camera(), Camera::default());
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
