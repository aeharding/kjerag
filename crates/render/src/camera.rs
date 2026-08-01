//! Where the view points, and what the mouse does to it.
//!
//! No iced in this file: `src/widget.rs` turns events into these calls. The
//! rules themselves are arithmetic, and arithmetic is testable without a
//! window.

use std::f32::consts::{PI, TAU};

use super::projection::{FOV_FLAT, fov_ceiling, normalize, view_ray, world_ray};

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

/// Field of view per scroll step, as a ratio. Multiplicative, so a notch
/// covers the same fraction of the range wherever it is used.
const ZOOM_PER_STEP: f32 = 0.12;

/// Scroll steps one press of the zoom key is worth. A wheel notch is a small
/// adjustment on a hand that is already on the picture; a key press is a
/// standing instruction and wants to cross the range in a handful of presses.
const STEPS_PER_KEY: f32 = 3.0;

/// The middle of the output, which is where a keyboard with no hand on the
/// picture is pointing.
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
    /// **The pitch runs all the way round** (issue #63). Past a quarter turn
    /// the view is looking back over the top of itself and the world is
    /// upside down, which is a place the drag may go and keep going: there is
    /// no wall at the zenith or the nadir, and no snap. It stays a pitch and
    /// never becomes a roll, so the horizon stays a level line either way up,
    /// and the yaw is left alone by a vertical drag rather than swinging half
    /// a turn at the pole.
    ///
    /// Exact wherever a level horizon can hold the direction there at all.
    /// Where it cannot -- a direction nearer the pole than the cursor's own
    /// ray reaches without rolling -- the height clamps, which leaves the
    /// direction on the cursor's own meridian but short of it.
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
        // rolling, and that clamp is the whole of the degeneracy now that the
        // pitch has no limit of its own (issue #63).
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
        // Wrapped rather than clamped: a rotation is the same rotation a turn
        // later, so this is only about keeping the number readable. Nothing
        // reads the pitch back except this file.
        self.pitch = wrap(tilt - rise);

        // Bearing fixes the yaw, at the tilt the height asked for rather than
        // the one the height clamp allowed: taking it from the clamped tilt
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

    /// Turn the view by a cursor move, with nothing pinned to the cursor.
    ///
    /// The wide half of the zoom drags this way (issue #78). Pinning the
    /// world to the cursor is what makes the flat range feel like a hand on
    /// the picture, and it is also what makes the wide end twitchy: a pixel
    /// out there spans degrees, and near the ball's rim it spans a whole turn
    /// of azimuth, so an exact solve has no choice but to fling the view
    /// about. Past the flat range the drag stops asking for exact and asks
    /// for calm instead: the same travel of the hand turns the world by the
    /// same angle wherever the zoom is.
    ///
    /// Vertically that is all it is, because pitch turns about the view's own
    /// horizontal axis and a vertical drag is exactly a pitch. Horizontally
    /// the view has only the world's vertical to turn about, and that axis
    /// hangs upside down once the pitch is past a quarter turn (issue #63):
    /// the same turn of the yaw then moves the picture the other way, so the
    /// drag turns the other way with it and the picture keeps following the
    /// hand.
    pub fn turn(&mut self, from: [f32; 2], to: [f32; 2], aspect: f32) {
        // `aspect` on the vertical only, so that the two axes answer the same
        // angle per pixel: `uv` is a fraction of the width across and a
        // fraction of the height down, and a window is wider than it is tall.
        let sideways = (to[0] - from[0]) * wide_rate();
        let up = (to[1] - from[1]) * wide_rate() / aspect;

        self.yaw = wrap(self.yaw - sideways * self.pitch.cos().signum());
        self.pitch = wrap(self.pitch + up);
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
/// really do put the direction under the cursor, the far one through a view
/// that has gone over the top and is upside down, so this is not a choice
/// about correctness: it is the choice between following the drag and
/// jumping, and the drag is what a hand is holding.
///
/// Nearest the short way round, which is what carries a drag over the pole
/// (issue #63): each of the two tilts is a tilt every turn, and at the
/// crossing the nearer of them is the one a quarter turn past the vertical,
/// not the one the previous turn left behind.
fn nearest_tilt(sine: f32, held: f32) -> f32 {
    let principal = sine.asin();
    let (near, far) = (wrap(principal - held), wrap(PI - principal - held));
    match far.abs() < near.abs() {
        true => held + far,
        false => held + near,
    }
}

/// How far the world turns per frame width of cursor travel, once the view is
/// wider than the flat range (issue #78).
///
/// **This is the feel constant, and it is one number.** It is the rate the
/// glued drag itself turns at, at the middle of the widest flat view, which
/// is the last view before the handover: 2 tan(55 degrees), or 164 degrees of
/// world per width of window. So the drag hands over at the pace it was
/// already going, and then holds that pace all the way out to the ball
/// instead of running away with the zoom -- the glued rate at the middle of
/// the ball view is five and a half times this, and that is the twitch the
/// owner reported.
///
/// A first guess, to be tuned by the hand that felt the problem. Turning it
/// down calms the ball further at the price of a small step at the handover;
/// nothing else in the file depends on its value.
fn wide_rate() -> f32 {
    2.0 * (FOV_FLAT * 0.5).tan()
}

/// The same angle, in (-pi, pi].
fn wrap(angle: f32) -> f32 {
    let turned = angle.rem_euclid(TAU);
    match turned > PI {
        true => turned - TAU,
        false => turned,
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

/// A drag in progress: what it took hold of, and where the cursor was last
/// seen.
///
/// Both, because the two halves of the zoom drag differently (issue #78). A
/// flat view is dragged by the direction, which stays under the cursor; a
/// view wider than that is dragged by the cursor's own travel. The field of
/// view can change mid-drag, so a drag carries what each needs.
#[derive(Clone, Copy, Debug)]
struct Drag {
    anchor: [f32; 3],
    at: [f32; 2],
}

/// Where the view points, and any drag that has hold of it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Viewpoint {
    camera: Camera,
    drag: Option<Drag>,
}

impl Viewpoint {
    pub fn camera(&self) -> Camera {
        self.camera
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Take hold of whatever the cursor is over: the direction, which is what
    /// a flat view's drag pins to the cursor, and the position, which is what
    /// a wider one measures its travel from.
    ///
    /// A press on the room around the ball takes hold of nothing, because
    /// there is nothing there to take hold of.
    pub fn grab(&mut self, uv: [f32; 2], aspect: f32) {
        self.drag = self
            .camera
            .look(uv, aspect)
            .map(|anchor| Drag { anchor, at: uv });
    }

    pub fn release(&mut self) {
        self.drag = None;
    }

    /// Continue a drag to a new cursor position. `true` when the camera
    /// moved, which is the caller's cue to ask for a redraw.
    ///
    /// A move with no grab held moves nothing and arms nothing: [`grab`] is
    /// the only thing that starts a drag, because a drag armed by a move on
    /// its way past panned the view with no button down (issue #26).
    ///
    /// [`grab`]: Self::grab
    pub fn drag_to(&mut self, uv: [f32; 2], aspect: f32) -> bool {
        let Some(drag) = self.drag else {
            return false;
        };
        let parked = self.camera;
        match self.camera.fov > FOV_FLAT {
            true => self.camera.turn(drag.at, uv, aspect),
            false => self.camera.aim(drag.anchor, uv, aspect),
        }
        self.drag = Some(Drag { at: uv, ..drag });
        self.camera != parked
    }

    /// Zoom by scroll steps, with the cursor at `uv`.
    ///
    /// A held drag takes hold again at the new field of view. The direction
    /// under the cursor moved when the view widened, and a drag still solving
    /// for the old one would jump the picture on its next move.
    ///
    /// **A cursor in the room around the ball keeps the hold it has** (issue
    /// #92). There is no direction there to take hold of, and a drag is a
    /// button the hand is still holding down: letting go of it because the
    /// wheel turned over the room killed the pan outright, and the only way
    /// back was to release and press again. Nothing reads the kept direction
    /// while it is stale, because the room and the pinned drag cannot be seen
    /// at the same width: the room needs a view wider than `2 * FOV_FLAT`
    /// (past there the sphere closes into a disc with an edge, `Screen`),
    /// the pinned drag stops at `FOV_FLAT`, and the zoom re-takes the hold at
    /// the width it just arrived at. The wide drag between them reads the
    /// cursor's travel and not what is under it, which is why the pan worked
    /// in the room until the wheel touched it.
    pub fn zoom(&mut self, steps: f32, uv: [f32; 2], aspect: f32) {
        self.camera.zoom(steps, aspect);
        let Some(held) = self.drag else {
            return;
        };
        self.drag = Some(Drag {
            anchor: self.camera.look(uv, aspect).unwrap_or(held.anchor),
            at: uv,
        });
    }

    /// Apply a [`Nudge`], which zooms where the keyboard is pointing.
    pub fn nudge(&mut self, nudge: Nudge, aspect: f32) {
        match nudge {
            Nudge::ZoomIn => self.zoom(STEPS_PER_KEY, self.pointing(), aspect),
            Nudge::ZoomOut => self.zoom(-STEPS_PER_KEY, self.pointing(), aspect),
            // The drag keeps its hold: the direction it grabbed is not where
            // the reset leaves the view, and re-anchoring here would haul the
            // picture straight back on the next move.
            Nudge::Reset => {
                self.camera = Camera::default();
                self.drag = None;
            }
        }
    }

    /// Where a view change with no event of its own is aimed: at the cursor a
    /// held drag last saw, and otherwise at the middle of the frame.
    ///
    /// A key has no cursor of its own, but a hand on the picture has one, and
    /// that is where the zoom has to take its hold again (issue #83). Taking
    /// it at the middle while the drag is holding somewhere else hands the
    /// drag a direction the hand never grabbed, and the next move hauls the
    /// picture over to it. With nothing held there is no such place, and the
    /// middle is what a keyboard is pointing at.
    fn pointing(&self) -> [f32; 2] {
        self.drag.map_or(MIDDLE, |drag| drag.at)
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

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

    /// Which way the picture slid: positive when the direction that was in
    /// the middle of the frame has ended up right of the middle.
    fn slid(before: [f32; 3], camera: Camera, aspect: f32) -> f32 {
        slip(camera, before, [0.45, 0.5], aspect) - slip(camera, before, [0.55, 0.5], aspect)
    }

    /// Zoomed all the way out, which is as far as the clamp allows.
    fn at_the_ball(aspect: f32) -> Viewpoint {
        let mut viewpoint = Viewpoint::default();
        for _ in 0..40 {
            viewpoint.zoom(-1.0, MIDDLE, aspect);
        }
        assert_eq!(viewpoint.camera().fov, fov_ceiling(aspect));
        viewpoint
    }

    /// A viewpoint parked at one camera with nothing held, which is where a
    /// test that starts partway round the sphere starts.
    fn parked(camera: Camera) -> Viewpoint {
        Viewpoint {
            camera,
            ..Viewpoint::default()
        }
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

    /// Issue #63, in the owner's own words: "I should keep being able to look
    /// up until I see upside down, and keep going".
    ///
    /// One drag can only ask for so much, so this is a hundred of them: grab
    /// the middle, haul it to the bottom, let go, grab the middle again. The
    /// view is upside down partway through and back the right way up by the
    /// end, having gone round; what it never does is stop.
    #[test]
    fn the_pitch_keeps_going_past_straight_up() {
        let mut viewpoint = Viewpoint::default();
        let mut climbed = 0.0;
        let mut inverted = 0;

        for _ in 0..100 {
            let before = viewpoint.camera().pitch;
            viewpoint.grab(MIDDLE, 1.0);
            viewpoint.drag_to([0.5, 1.0], 1.0);
            viewpoint.release();
            let camera = viewpoint.camera();

            let step = wrap(camera.pitch - before);
            assert!(step > 0.0, "the drag stopped climbing at {before}");
            climbed += step;
            if camera.pitch.abs() > FRAC_PI_2 {
                inverted += 1;
            }
            // A vertical drag is a pitch and nothing else: no yaw anywhere,
            // least of all at the pole it just went over.
            assert_eq!(camera.yaw, 0.0, "the view turned at {:?}", degrees(camera));
        }

        assert!(
            climbed > TAU,
            "a hundred drags up climbed {climbed} rad, not a whole turn",
        );
        assert!(inverted > 0, "the view was never upside down");
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
    /// Where it cannot, there is one thing that says so and the direction is
    /// asked for no more: it is nearer the pole than the cursor's own ray
    /// reaches without rolling. There used to be a second, the pitch limit,
    /// and issue #63 took it away, so the pitches here run past the vertical
    /// and out the other side: the solve has to hold the world under the
    /// cursor in an upside down view exactly as it does in an upright one.
    #[test]
    fn the_grabbed_direction_stays_under_the_cursor() {
        let places: Vec<[f32; 2]> = [0.02, 0.3, 0.5, 0.72, 0.98]
            .iter()
            .flat_map(|&x| [0.02, 0.3, 0.5, 0.72, 0.98].map(|y| [x, y]))
            .collect();
        let mut exact = 0;
        let mut clamped = 0;
        let mut upside_down = 0;

        for yaw in [-2.9, -0.4, 0.0, 1.1, 3.0] {
            for pitch in [-3.0, -2.2, -1.55, -1.2, -0.6, 0.0, 0.85, 1.5, 2.2, 3.0] {
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
                                if short {
                                    clamped += 1;
                                    continue;
                                }
                                exact += 1;
                                if aimed.pitch.abs() > FRAC_PI_2 {
                                    upside_down += 1;
                                }
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
        assert!(clamped > 0, "the grid never reached the degenerate case");
        assert!(
            upside_down > 1_000,
            "{upside_down} solved views were upside down, which is too few to \
             say the far side of the pole was tested",
        );
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
    /// pitch `asin` cannot name. It used to follow the cursor as far as the
    /// pitch limit and then stop dead, which was the spec until issue #63;
    /// now it follows all the way, down through the nadir and up the far
    /// side, and it is the same one solve doing it.
    #[test]
    fn content_past_the_nadir_follows_the_cursor_over_the_pole() {
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
        let mut crossed = false;
        for step in 1..=90 {
            let to = [0.5, 0.95 - 0.01 * step as f32];
            camera.aim(direction, to, aspect);

            assert!(
                camera.yaw.abs() < 1e-3,
                "the view turned around: {:?}",
                degrees(camera)
            );
            let slip = slip(camera, direction, to, aspect);
            assert!(
                slip <= pixel(camera, to, aspect),
                "the body slipped {slip} rad off the cursor at {:?}",
                degrees(camera),
            );
            crossed |= camera.pitch.abs() > FRAC_PI_2;
        }
        assert!(crossed, "the drag never reached the far side of the nadir");
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
                assert!(aimed.pitch.is_finite() && aimed.pitch.abs() <= PI);
            }
        }
    }

    /// A cursor swept clean across the pole's own place on the output, with
    /// hold of a direction a degree away from it. The turn is fast, because
    /// hauling a point that close to the axis really does swing the world,
    /// but it stays a turn: level, finite, and a rotation rather than a
    /// number running away.
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
            assert!(
                camera.pitch.is_finite() && camera.pitch.abs() <= PI,
                "{:?}",
                degrees(camera)
            );
        }
    }

    /// Issue #63's acceptance, in one drag: grab something near the top of a
    /// wide view, haul it to the bottom, and the view goes up over the zenith
    /// and out the other side without the grabbed direction ever leaving the
    /// cursor.
    ///
    /// What "no wall, no snap" means as arithmetic: the grabbed direction
    /// stays within a pixel of the cursor at every step, the pitch climbs at
    /// every step, and it climbs past a quarter turn.
    #[test]
    fn a_drag_carries_the_view_through_the_zenith() {
        let aspect = 16.0 / 9.0;
        let mut camera = Camera {
            yaw: 0.4,
            pitch: 70f32.to_radians(),
            fov: FOV_FLAT,
        };
        let grab = [0.5, 0.06];
        let direction = camera
            .look(grab, aspect)
            .expect("a flat view is all sphere");

        let mut climbed = 0.0;
        for step in 1..=88 {
            let to = [0.5, grab[1] + 0.01 * step as f32];
            let before = camera.pitch;
            camera.aim(direction, to, aspect);

            let slip = slip(camera, direction, to, aspect);
            assert!(
                slip <= pixel(camera, to, aspect),
                "the world came off the cursor by {slip} rad at {:?}",
                degrees(camera),
            );
            let climb = wrap(camera.pitch - before);
            assert!(climb > 0.0, "the climb stalled at {:?}", degrees(camera));
            climbed += climb;
            // A drag straight down the middle turns nothing, at the pole
            // least of all.
            assert!(
                (camera.yaw - 0.4).abs() < 1e-3,
                "the view swung to {:?}",
                degrees(camera),
            );
        }

        assert!(
            camera.pitch.abs() > FRAC_PI_2,
            "one drag ended at {:?}, still the right way up",
            degrees(camera),
        );
        assert!(climbed > 1.0, "the drag only climbed {climbed} rad");
    }

    /// Upside down is a pitch and not a roll (issue #63): half a turn of
    /// pitch swaps the sky for the ground and leaves the horizon a level line
    /// across the output rather than a tilted one.
    #[test]
    fn a_view_past_the_pole_is_upside_down_and_level() {
        let aspect = 16.0 / 9.0;
        let camera = Camera {
            yaw: 0.9,
            pitch: PI,
            fov: FRAC_PI_2,
        };
        let look = |uv| camera.look(uv, aspect).expect("a flat view is all sphere");

        // y is down, so a smaller y is higher: the top of the output is the
        // ground and the bottom is the sky.
        assert!(
            look([0.5, 0.1])[1] > look([0.5, 0.9])[1],
            "the ends did not swap",
        );

        // Level: the horizon lands along one row of the output and stays
        // there, corner to corner, instead of running down the frame.
        for u in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let height = look([u, 0.5])[1];
            assert!(height.abs() < 1e-6, "the horizon sits at {height} at {u}");
        }
    }

    /// Round and round: a full turn of vertical dragging comes back to the
    /// view it started at, having been upside down in the middle of it.
    #[test]
    fn a_whole_vertical_turn_comes_back() {
        let start = Camera {
            yaw: -1.2,
            pitch: 0.0,
            fov: FRAC_PI_2,
        };
        let mut viewpoint = parked(start);
        let mut inverted = 0;

        // Each grab-drag-release pitches up by a fixed slice of the view, so
        // the loop is however many of those slices make a turn, and the test
        // is that the arithmetic closes.
        for _ in 0..1000 {
            viewpoint.grab(MIDDLE, 1.0);
            viewpoint.drag_to([0.5, 0.55], 1.0);
            viewpoint.release();
            if viewpoint.camera().pitch.abs() > FRAC_PI_2 {
                inverted += 1;
            }
            if inverted > 0 && wrap(viewpoint.camera().pitch - start.pitch).abs() < 1e-2 {
                assert!(
                    (viewpoint.camera().yaw - start.yaw).abs() < 1e-3,
                    "the yaw drifted to {:?}",
                    degrees(viewpoint.camera()),
                );
                return;
            }
        }
        panic!(
            "a thousand drags up never came back round: {:?}",
            degrees(viewpoint.camera())
        );
    }

    /// Ctrl+0 rights the view from anywhere, upside down included.
    #[test]
    fn the_default_view_comes_back_from_upside_down() {
        let mut viewpoint = parked(Camera {
            yaw: 2.5,
            pitch: 2.9,
            fov: 0.6,
        });
        viewpoint.nudge(Nudge::Reset, 1.6);
        assert_eq!(viewpoint.camera(), Camera::default());
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

    /// The ball can be grabbed and turned like anything else (issue #47), and
    /// since issue #78 it turns calmly rather than exactly: the picture
    /// follows the hand, at the rate the flat range handed over at, instead
    /// of being pinned to the cursor at a zoom where a pixel spans degrees.
    #[test]
    fn the_ball_turns_calmly_with_the_drag() {
        let aspect = 16.0 / 9.0;
        let mut viewpoint = at_the_ball(aspect);
        let start = viewpoint.camera();
        let middle = start
            .look(MIDDLE, aspect)
            .expect("the middle of the ball is picture");

        let (from, to) = ([0.46, 0.52], [0.56, 0.52]);
        viewpoint.grab(from, aspect);
        assert!(viewpoint.drag_to(to, aspect), "the ball did not turn");
        let turned = viewpoint.camera();
        assert!(turned.yaw.is_finite() && turned.pitch.is_finite());
        assert!(
            slid(middle, turned, aspect) > 0.0,
            "the ball did not follow the hand",
        );

        // Calm, measured against the drag that used to be here: pinning the
        // grabbed direction to the cursor turns this view several times as
        // far for the same move of the hand.
        let mut pinned = start;
        pinned.aim(
            start.look(from, aspect).expect("the grab was on the ball"),
            to,
            aspect,
        );
        let calm = wrap(turned.yaw - start.yaw).abs();
        let exact = wrap(pinned.yaw - start.yaw).abs();
        assert!(
            exact > 3.0 * calm,
            "the ball turned {calm} rad where the pinned solve turned {exact}, \
             which is not much calmer",
        );
    }

    /// The handover is not a step (issue #78). The widest flat view, which is
    /// pinned to the cursor, and the first view past it, which is not, turn
    /// by the same amount for the same move of the hand.
    #[test]
    fn the_wide_drag_takes_over_at_the_pace_the_glue_left_off() {
        let aspect = 16.0 / 9.0;
        let step = [0.53, 0.47];
        let turn = |fov| {
            let mut viewpoint = parked(Camera {
                fov,
                ..Camera::default()
            });
            viewpoint.grab(MIDDLE, aspect);
            assert!(viewpoint.drag_to(step, aspect));
            viewpoint.camera()
        };

        let glued = turn(FOV_FLAT);
        let rated = turn(FOV_FLAT * 1.001);
        let apart = (glued.yaw - rated.yaw)
            .abs()
            .max((glued.pitch - rated.pitch).abs());
        assert!(
            apart < 0.02 * glued.yaw.abs(),
            "the drag stepped by {apart} rad at the threshold, from {:?} to {:?}",
            degrees(glued),
            degrees(rated),
        );
    }

    /// Issue #63 in issue #78's drag: the wide view goes over the pole too,
    /// and once it is upside down a sideways drag still moves the picture the
    /// way the hand goes rather than the other way.
    #[test]
    fn the_wide_drag_goes_over_the_pole_the_right_way_round() {
        let aspect = 16.0 / 9.0;
        let mut viewpoint = at_the_ball(aspect);
        viewpoint.grab(MIDDLE, aspect);

        // Hauled down the window and let go, over and over, which is how a
        // hand asks for more than one window's worth of pitch.
        let mut seen = [false, false];
        for _ in 0..8 {
            viewpoint.grab([0.5, 0.2], aspect);
            for step in 1..=6 {
                let at = [0.5, 0.2 + 0.1 * step as f32];
                let before = viewpoint.camera();
                assert!(viewpoint.drag_to(at, aspect), "the drag stalled");
                assert!(
                    wrap(viewpoint.camera().pitch - before.pitch) > 0.0,
                    "the climb stalled at {:?}",
                    degrees(viewpoint.camera()),
                );

                // The first time the view is upright, and the first time it
                // is upside down: a sideways drag has to move the picture the
                // way the hand goes, either way up.
                let inverted = viewpoint.camera().pitch.abs() > FRAC_PI_2;
                if !seen[usize::from(inverted)] {
                    seen[usize::from(inverted)] = true;
                    let middle = viewpoint
                        .camera()
                        .look(MIDDLE, aspect)
                        .expect("the middle of the ball is picture");
                    let mut sideways = viewpoint;
                    assert!(sideways.drag_to([at[0] + 0.1, at[1]], aspect));
                    assert!(
                        slid(middle, sideways.camera(), aspect) > 0.0,
                        "the picture went the other way from the hand at {:?}, \
                         upside down: {inverted}",
                        degrees(viewpoint.camera()),
                    );
                }
            }
            viewpoint.release();
        }
        assert_eq!(seen, [true, true], "the drag never went over the pole");
    }

    /// The room around the ball holds nothing, so a press on it takes hold of
    /// nothing.
    ///
    /// A drag that starts on the ball and wanders out into the room keeps
    /// turning the view, which is issue #78's doing: the wide drag reads the
    /// hand's travel and not what is under the cursor, so there is nothing
    /// for the room to fail to answer. Before it, the same drag stopped dead
    /// on the way out and started again on the way back, because the solve
    /// had nothing to solve for; the room is most of the corners at this
    /// zoom, and stopping there is what a hand notices.
    #[test]
    fn the_room_around_the_ball_is_not_a_handle() {
        let aspect = 16.0 / 9.0;
        let mut viewpoint = at_the_ball(aspect);
        let corner = [0.01, 0.01];
        assert!(
            viewpoint.camera().look(corner, aspect).is_none(),
            "the corner of the ball view is still picture",
        );

        viewpoint.grab(corner, aspect);
        assert!(!viewpoint.is_dragging());
        assert!(!viewpoint.drag_to([0.5, 0.5], aspect));

        viewpoint.grab([0.5, 0.5], aspect);
        assert!(viewpoint.is_dragging());
        viewpoint.drag_to([0.53, 0.5], aspect);
        let parked = viewpoint.camera();
        assert!(viewpoint.drag_to(corner, aspect), "the drag stopped dead");
        assert_ne!(viewpoint.camera(), parked);
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

    /// A zoom in the middle of a drag re-reads what the cursor is over, so
    /// the move after it is still a move from here. Without that the next
    /// move solves for a direction the cursor stopped being over the moment
    /// the view widened, and the picture jumps.
    ///
    /// The drag is taken to [`HELD`] and left there, the zoom goes in, and
    /// the move after it asks for nothing: anything the view does then, it
    /// did on its own.
    #[track_caller]
    fn zoom_mid_drag_holds(start: Camera, zoom: impl FnOnce(&mut Viewpoint, f32)) {
        let aspect = 1.6;

        let mut viewpoint = parked(start);
        viewpoint.grab([0.8, 0.2], aspect);
        assert!(viewpoint.drag_to(HELD, aspect), "the drag moved nothing");

        zoom(&mut viewpoint, aspect);
        let zoomed = viewpoint.camera();
        assert_ne!(
            zoomed.fov, start.fov,
            "the zoom left the view as wide as it was"
        );

        viewpoint.drag_to(HELD, aspect);
        let moved = viewpoint.camera();
        assert!(
            (moved.yaw - zoomed.yaw).abs() < 1e-4 && (moved.pitch - zoomed.pitch).abs() < 1e-4,
            "the cursor did not move but the view went from {:?} to {:?}",
            degrees(zoomed),
            degrees(moved),
        );
    }

    /// Where the drag in [`zoom_mid_drag_holds`] is holding when the zoom
    /// arrives, which is nowhere near the middle of the frame: the wheel is
    /// handed it and the key has to know it already.
    const HELD: [f32; 2] = [0.6, 0.4];

    /// A view narrow enough that its drag stays pinned to the cursor, and one
    /// wide enough that its drag reads the hand's travel instead: the two
    /// regimes issue #78 left, which zoom alike and drag differently.
    const PINNED: Camera = Camera {
        yaw: 0.0,
        pitch: 0.0,
        fov: FOV_FLAT * 0.8,
    };
    const WIDE: Camera = Camera {
        yaw: 0.0,
        pitch: 0.0,
        fov: FOV_FLAT * 1.5,
    };

    #[test]
    fn scrolling_mid_drag_keeps_the_grab() {
        zoom_mid_drag_holds(PINNED, |viewpoint, aspect| {
            viewpoint.zoom(3.0, HELD, aspect)
        });
        zoom_mid_drag_holds(WIDE, |viewpoint, aspect| viewpoint.zoom(3.0, HELD, aspect));
    }

    /// Issue #83: the same invariant for the key, which used to take its hold
    /// again at the middle of the frame however far from there the cursor
    /// was. The pinned drag then solved for a direction the hand never
    /// grabbed and the wide one measured its travel from a place the hand
    /// never was, so both regimes jumped on the next move and both are here.
    #[test]
    fn keyboard_zooming_mid_drag_keeps_the_grab() {
        for nudge in [Nudge::ZoomIn, Nudge::ZoomOut] {
            zoom_mid_drag_holds(PINNED, |viewpoint, aspect| viewpoint.nudge(nudge, aspect));
            zoom_mid_drag_holds(WIDE, |viewpoint, aspect| viewpoint.nudge(nudge, aspect));
        }
    }

    /// A point of the ball view that is room rather than picture, and the one
    /// the drag starts from, which is picture at every width.
    const ROOM: [f32; 2] = [0.12, 0.2];

    /// Issue #92: a zoom while the cursor is out in the room does not let go
    /// of the drag. There is no direction out there to take hold of, and the
    /// wide drag was never holding one.
    ///
    /// The drag is grabbed on the ball and hauled out into the room, which is
    /// the owner's own gesture and works; the zoom lands there.
    #[track_caller]
    fn zoom_over_the_room_holds(zoom: impl FnOnce(&mut Viewpoint, f32)) {
        let aspect = 16.0 / 9.0;
        let mut viewpoint = at_the_ball(aspect);
        assert!(
            viewpoint.camera().look(ROOM, aspect).is_none(),
            "the test point is picture, not room",
        );

        viewpoint.grab(MIDDLE, aspect);
        assert!(viewpoint.drag_to(ROOM, aspect), "the drag stalled");

        zoom(&mut viewpoint, aspect);
        assert!(viewpoint.is_dragging(), "the zoom let go of the drag");

        let zoomed = viewpoint.camera();
        assert!(
            viewpoint.drag_to([ROOM[0] + 0.2, ROOM[1]], aspect),
            "the pan died at the zoom",
        );
        assert_ne!(viewpoint.camera().yaw, zoomed.yaw);
    }

    #[test]
    fn zooming_over_the_room_keeps_the_drag() {
        // Both ways the wheel goes, because the far end is a clamp rather than
        // a state and a scroll that changes nothing at all still re-takes the
        // hold. And both keys, which aim where the drag is holding (issue
        // #83), so a hand out in the room aims them into the room.
        for steps in [1.0, -1.0] {
            zoom_over_the_room_holds(|viewpoint, aspect| viewpoint.zoom(steps, ROOM, aspect));
        }
        for nudge in [Nudge::ZoomIn, Nudge::ZoomOut] {
            zoom_over_the_room_holds(|viewpoint, aspect| viewpoint.nudge(nudge, aspect));
        }
    }

    /// The same drag on the way back in (issue #92): the hand holds the button
    /// and scrolls the whole zoom range in from the ball, cursor parked out in
    /// the room, so the picture comes out to meet it partway.
    ///
    /// Two things at every notch. The drag is alive, which is what the room
    /// used to end. And it has not jumped: a move to where the cursor already
    /// is asks for nothing, so anything the view does then it did on its own.
    /// That covers both crossings the scroll makes -- the room's edge, where
    /// the hold starts answering again, and [`FOV_FLAT`], where the drag goes
    /// back to being pinned to the cursor -- and the second is why the kept
    /// direction is never read stale: the room is gone a whole `FOV_FLAT`
    /// before the pinned drag starts.
    #[test]
    fn scrolling_in_from_the_room_keeps_the_drag_and_does_not_jump() {
        let aspect = 16.0 / 9.0;
        let mut viewpoint = at_the_ball(aspect);
        viewpoint.grab(MIDDLE, aspect);
        assert!(viewpoint.drag_to(ROOM, aspect));

        let mut over_the_room = 0;
        let mut pinned = 0;
        while viewpoint.camera().fov > FOV_MIN {
            viewpoint.zoom(1.0, ROOM, aspect);
            let camera = viewpoint.camera();
            assert!(viewpoint.is_dragging(), "the zoom let go at {}", camera.fov);

            match camera.look(ROOM, aspect) {
                None => over_the_room += 1,
                Some(_) if camera.fov <= FOV_FLAT => pinned += 1,
                Some(_) => {}
            }

            viewpoint.drag_to(ROOM, aspect);
            let moved = viewpoint.camera();
            assert!(
                (moved.yaw - camera.yaw).abs() < 1e-4 && (moved.pitch - camera.pitch).abs() < 1e-4,
                "the cursor did not move but the view went from {:?} to {:?} at \
                 fov {:.0}",
                degrees(camera),
                degrees(moved),
                camera.fov.to_degrees(),
            );
        }

        assert!(over_the_room > 0, "the scroll never started in the room");
        assert!(pinned > 0, "the scroll never reached the pinned drag");
    }

    /// With no drag held the key is the only thing pointing, and it points at
    /// the middle: the zoom changes how wide the view is, moves it nowhere,
    /// and takes hold of nothing.
    #[test]
    fn a_keyboard_zoom_with_nothing_held_only_changes_the_width() {
        let start = Camera {
            yaw: 0.7,
            pitch: -0.3,
            fov: 1.2,
        };
        for nudge in [Nudge::ZoomIn, Nudge::ZoomOut] {
            let mut viewpoint = parked(start);
            viewpoint.nudge(nudge, 1.6);
            let zoomed = viewpoint.camera();

            assert_ne!(zoomed.fov, start.fov, "the key did not zoom");
            assert_eq!((zoomed.yaw, zoomed.pitch), (start.yaw, start.pitch));
            assert!(!viewpoint.is_dragging());
        }
    }
}
