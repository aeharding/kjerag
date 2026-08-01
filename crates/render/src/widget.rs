//! How the shell hands this pass to iced, and how the mouse reaches it.
//!
//! These three impls put a foreign trait (`iced::widget::shader`) on types
//! this crate owns, so Rust's coherence rules require them to live here
//! rather than in `kjerag`: that, and nothing else, is why the render layer
//! names libcosmic. Nothing above this file decides anything about the pass
//! or the view direction; the shell only builds a `shader::Shader` around a
//! [`Scene`], which is where the [`Viewpoint`] lives.

use std::time::Instant;

use cosmic::iced::widget::shader::{self, Action};
use cosmic::iced::{Event, Point, Rectangle, mouse, window};

use super::{Next, Scene, ScenePipeline, ScenePrimitive, Viewpoint};

/// Wheels report scroll in lines and touchpads report it in pixels, and iced
/// passes both through as they came. A feel constant, not a measurement.
const PIXELS_PER_LINE: f32 = 40.0;

impl<Message> shader::Program<Message> for Scene {
    /// Nothing at all. The camera lived here until issue #77: iced keeps
    /// widget state in the widget tree, and the tree is rebuilt from the
    /// shell's `view` whenever the window changes shape, which takes the
    /// state with it. It is the [`Scene`]'s now.
    type State = ();
    type Primitive = ScenePrimitive;

    fn update(
        &self,
        _state: &mut (),
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        match event {
            Event::Mouse(event) => mouse_update(self, event, bounds, cursor),
            Event::Window(window::Event::RedrawRequested(now)) => {
                // The `View` menu's zoom items have no cursor and no event of
                // their own, so the shell leaves them on the scene and this is
                // where they reach the camera: a redraw is the first place the
                // shape of the output is known.
                if let Some(nudge) = self.take_nudge() {
                    self.steer(|viewpoint| viewpoint.nudge(nudge, aspect(bounds)));
                }
                tick(self, *now)
            }
            // Alt-tabbing away mid-drag takes the release with it, and a grab
            // nothing can end is a camera glued to the cursor.
            Event::Window(window::Event::Unfocused) => {
                self.steer(Viewpoint::release);
                None
            }
            _ => None,
        }
    }

    /// The one place that knows a redraw reached this widget, which is why
    /// the shape it drew into is recorded here (issue #102).
    fn draw(&self, _state: &(), _cursor: mouse::Cursor, bounds: Rectangle) -> ScenePrimitive {
        self.drew(bounds.width, bounds.height);
        self.primitive(self.viewpoint().camera())
    }

    /// `Hidden` is how a pointer disappears in iced: the winit conversion maps
    /// it to no cursor icon and the window then calls
    /// `set_cursor_visible(false)`. It answers only over the video, so the
    /// pointer comes back the moment it is over the controls.
    fn mouse_interaction(
        &self,
        _state: &(),
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match (
            self.viewpoint().is_dragging(),
            self.is_cursor_hidden(),
            cursor.is_over(bounds),
        ) {
            (true, _, _) => mouse::Interaction::Grabbing,
            (false, true, true) => mouse::Interaction::Hidden,
            (false, false, true) => mouse::Interaction::Grab,
            (false, _, false) => mouse::Interaction::default(),
        }
    }
}

impl shader::Primitive for ScenePrimitive {
    type Pipeline = ScenePipeline;

    fn prepare(
        &self,
        pipeline: &mut ScenePipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        _viewport: &shader::Viewport,
    ) {
        // Logical pixels on both axes, so the ratio is the physical one. The
        // floor keeps the first layout pass, where the widget can still be
        // zero high, from dividing by zero.
        let aspect = bounds.width / bounds.height.max(1.0);
        pipeline.prepare(self, device, queue, aspect);
    }

    /// Drawing into iced's own pass rather than opening a second one: the
    /// widget's viewport and scissor are already set to the widget bounds.
    fn draw(&self, pipeline: &ScenePipeline, pass: &mut wgpu::RenderPass<'_>) -> bool {
        pipeline.draw(pass);
        true
    }
}

impl shader::Pipeline for ScenePipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        ScenePipeline::new(device, format)
    }
}

/// The pan is a grab: it starts on a press over the widget, it lasts exactly
/// as long as the button is held, and every way it can end ends it.
fn mouse_update<Message>(
    scene: &Scene,
    event: &mouse::Event,
    bounds: Rectangle,
    cursor: mouse::Cursor,
) -> Option<Action<Message>> {
    match event {
        // The press is not captured. The shell wraps this widget in a
        // `mouse_area` whose double press toggles fullscreen, and iced's
        // `mouse_area` gives up on any event a child captured
        // (`iced/widget/src/mouse_area.rs`, `update`), so capturing here would
        // take double click to fullscreen away. Nothing else in the window
        // wants a press over the video: the two extra grabs a double click
        // starts move nothing, because neither is dragged anywhere.
        mouse::Event::ButtonPressed(mouse::Button::Left) => {
            let at = cursor.position_over(bounds)?;
            scene.steer(|viewpoint| viewpoint.grab(uv(at, bounds), aspect(bounds)));
            Some(Action::request_redraw())
        }
        // Deliberately not gated on the cursor being over the widget: a pan
        // that stopped at the window edge, mid-drag, would feel broken. It is
        // the drag that decides whether this moves anything, not the cursor.
        // The `uv` of a cursor outside the widget is outside 0 to 1, which is
        // a direction outside the view, which is exactly what the drag means.
        mouse::Event::CursorMoved { position } => scene
            .steer(|viewpoint| viewpoint.drag_to(uv(*position, bounds), aspect(bounds)))
            .then(Action::request_redraw),
        // The release ends the grab wherever it lands, including off the
        // widget. `CursorLeft` is the case where it cannot land here at all:
        // the pointer left the window still held, so the release will be some
        // other window's.
        mouse::Event::ButtonReleased(mouse::Button::Left) | mouse::Event::CursorLeft => {
            scene.steer(Viewpoint::release);
            None
        }
        mouse::Event::WheelScrolled { delta } => {
            let at = cursor.position_over(bounds)?;
            scene.steer(|viewpoint| viewpoint.zoom(steps(*delta), uv(at, bounds), aspect(bounds)));
            Some(Action::request_redraw().and_capture())
        }
        _ => None,
    }
}

/// The presentation clock ticks on the window's own redraw event, inside the
/// pass that then draws the result: the scene takes the frame that is due and
/// says when the next one is, and the returned [`Action`] is what makes iced
/// sleep until exactly that instant. Waking per frame rather than per refresh
/// is what keeps 29.97 fps content off a 60 Hz grid; `kjerag::app` documents
/// the pacing, and the measurement that rejected the alternative.
fn tick<Message>(scene: &Scene, now: Instant) -> Option<Action<Message>> {
    match scene.pump(now) {
        Next::At(due) => Some(Action::request_redraw_at(due)),
        Next::Refresh => Some(Action::request_redraw()),
        Next::Never => None,
    }
}

fn steps(delta: mouse::ScrollDelta) -> f32 {
    match delta {
        mouse::ScrollDelta::Lines { y, .. } => y,
        mouse::ScrollDelta::Pixels { y, .. } => y / PIXELS_PER_LINE,
    }
}

/// Where a cursor position falls in the widget: 0 to 1 across it, y down,
/// which is how the projection reads a point of the output.
fn uv(at: Point, bounds: Rectangle) -> [f32; 2] {
    [
        (at.x - bounds.x) / bounds.width.max(1.0),
        (at.y - bounds.y) / bounds.height.max(1.0),
    ]
}

/// Width over height, read the same way [`ScenePipeline::prepare`] reads it,
/// because the two have to agree on where a pixel is looking.
///
/// [`ScenePipeline::prepare`]: super::ScenePipeline::prepare
fn aspect(bounds: Rectangle) -> f32 {
    bounds.width / bounds.height.max(1.0)
}

/// The grab, driven through the event stream iced actually delivers rather
/// than through [`Viewpoint`]'s methods, because which event calls which
/// method is half of what issue #26 was about. No window and no GPU: a
/// [`Scene`] with no file is inert until something asks it for a primitive.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::fov_ceiling;
    use crate::{Camera, Nudge};

    const BOUNDS: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1000.0,
        height: 500.0,
    };

    struct Widget {
        scene: Scene,
        /// Whatever iced keeps for this widget, which since issue #77 is
        /// nothing. Kept here all the same, and thrown away in one test
        /// below, because a state iced can rebuild under the app is the whole
        /// of what that issue was.
        state: (),
        cursor: mouse::Cursor,
    }

    impl Widget {
        fn new() -> Self {
            Self {
                scene: Scene::blank(),
                state: (),
                cursor: mouse::Cursor::Available(Point::ORIGIN),
            }
        }

        /// Where iced says the pointer is when the next event is handled.
        fn cursor_at(&mut self, x: f32, y: f32) -> &mut Self {
            self.cursor = mouse::Cursor::Available(Point::new(x, y));
            self
        }

        fn send(&mut self, event: Event) -> &mut Self {
            let _: Option<Action<()>> =
                shader::Program::update(&self.scene, &mut self.state, &event, BOUNDS, self.cursor);
            self
        }

        fn press(&mut self, x: f32, y: f32) -> &mut Self {
            self.cursor_at(x, y)
                .send(Event::Mouse(mouse::Event::ButtonPressed(
                    mouse::Button::Left,
                )))
        }

        fn release(&mut self, x: f32, y: f32) -> &mut Self {
            self.cursor_at(x, y)
                .send(Event::Mouse(mouse::Event::ButtonReleased(
                    mouse::Button::Left,
                )))
        }

        fn move_to(&mut self, x: f32, y: f32) -> &mut Self {
            self.cursor_at(x, y)
                .send(Event::Mouse(mouse::Event::CursorMoved {
                    position: Point::new(x, y),
                }))
        }

        fn scroll(&mut self, lines: f32) -> &mut Self {
            self.send(Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { x: 0.0, y: lines },
            }))
        }

        /// Zoomed out to the far end, which is a ball with room around it,
        /// with the pointer parked in the middle and nothing held.
        fn scroll_to_the_ball(&mut self) -> &mut Self {
            self.cursor_at(500.0, 250.0);
            for _ in 0..40 {
                self.scroll(-1.0);
            }
            assert_eq!(self.camera().fov, fov_ceiling(aspect(BOUNDS)));
            self
        }

        /// What the view is looking at through a point of the window, in
        /// window pixels. `None` is the room around the ball.
        fn looking_at(&self, x: f32, y: f32) -> Option<[f32; 3]> {
            self.camera()
                .look(uv(Point::new(x, y), BOUNDS), aspect(BOUNDS))
        }

        /// Where the view points, read where `draw` reads it.
        fn camera(&self) -> Camera {
            self.scene.viewpoint().camera()
        }

        /// What iced does when the widget tree changes shape under a widget:
        /// the old state is dropped and a new one is built from scratch
        /// (`iced_core::widget::Tree::diff`).
        fn rebuild_state(&mut self) -> &mut Self {
            self.state = <Scene as shader::Program<()>>::State::default();
            self
        }

        fn interaction(&self) -> mouse::Interaction {
            shader::Program::<()>::mouse_interaction(&self.scene, &self.state, BOUNDS, self.cursor)
        }
    }

    /// Issue #26, as the pilot met it: the pointer crossing the window moved
    /// the view with no button held.
    #[test]
    fn a_bare_cursor_moves_nothing() {
        let mut widget = Widget::new();
        widget.move_to(100.0, 100.0).move_to(400.0, 250.0);
        assert_eq!(widget.camera(), Camera::default());
    }

    #[test]
    fn a_held_button_pans() {
        let mut widget = Widget::new();
        widget.press(100.0, 100.0).move_to(400.0, 100.0);
        assert_ne!(widget.camera(), Camera::default());
    }

    /// Issue #77, at the level it happens: the pilot pans, the window changes
    /// shape, and iced rebuilds this widget's state under him.
    ///
    /// Entering fullscreen is one of the changes that does it, because the
    /// header bar goes with it and libcosmic pushes the header into the same
    /// column as the content, so the content moves up a place and everything
    /// under it is built fresh. Leaving fullscreen is another, and so is the
    /// header bar hiding two seconds after the pointer stops. The camera has
    /// to be somewhere none of that reaches.
    #[test]
    fn a_rebuilt_widget_state_holds_the_view() {
        let mut widget = Widget::new();
        widget
            .press(100.0, 100.0)
            .move_to(400.0, 180.0)
            .release(400.0, 180.0);
        let panned = widget.camera();
        assert_ne!(panned, Camera::default(), "the pan moved nothing");

        widget.rebuild_state();
        assert_eq!(widget.camera(), panned);

        // And the drag still works from where it left off, rather than from
        // some camera the rebuild reset.
        widget.press(400.0, 180.0).move_to(500.0, 180.0);
        assert_ne!(widget.camera(), panned);
    }

    #[test]
    fn the_release_ends_the_pan() {
        let mut widget = Widget::new();
        widget
            .press(100.0, 100.0)
            .move_to(400.0, 100.0)
            .release(400.0, 100.0);
        let parked = widget.camera();

        widget.move_to(700.0, 300.0).move_to(200.0, 100.0);
        assert_eq!(widget.camera(), parked);
    }

    /// The release is only guaranteed to arrive somewhere, not here.
    #[test]
    fn a_release_off_the_widget_ends_the_pan() {
        let mut widget = Widget::new();
        widget
            .press(100.0, 100.0)
            .move_to(400.0, 100.0)
            .release(-40.0, 900.0);
        let parked = widget.camera();

        widget.move_to(700.0, 300.0).move_to(200.0, 100.0);
        assert_eq!(widget.camera(), parked);
    }

    /// Held out of the window, and held through an alt-tab: two ways the
    /// release never reaches this widget at all.
    #[test]
    fn a_grab_cannot_outlive_the_window() {
        for escape in [
            Event::Mouse(mouse::Event::CursorLeft),
            Event::Window(window::Event::Unfocused),
        ] {
            let mut widget = Widget::new();
            widget
                .press(100.0, 100.0)
                .move_to(400.0, 100.0)
                .send(escape);
            let parked = widget.camera();

            widget.move_to(700.0, 300.0).move_to(200.0, 100.0);
            assert_eq!(widget.camera(), parked);
        }
    }

    /// A press that misses the widget is not this widget's press.
    #[test]
    fn a_press_outside_the_widget_grabs_nothing() {
        let mut widget = Widget::new();
        widget.press(1200.0, 100.0).move_to(400.0, 100.0);
        assert_eq!(widget.camera(), Camera::default());
    }

    #[test]
    fn the_cursor_icon_follows_the_grab() {
        let mut widget = Widget::new();
        widget.cursor_at(100.0, 100.0);
        assert_eq!(widget.interaction(), mouse::Interaction::Grab);

        widget.press(100.0, 100.0);
        assert_eq!(widget.interaction(), mouse::Interaction::Grabbing);

        widget.move_to(1400.0, 100.0);
        assert_eq!(widget.interaction(), mouse::Interaction::Grabbing);

        widget.release(1400.0, 100.0);
        assert_eq!(widget.interaction(), mouse::Interaction::default());
    }

    /// Hiding the controls hides the pointer with them, over the video only:
    /// a pointer that vanished over the header bar could not aim at anything.
    #[test]
    fn hidden_controls_hide_the_cursor() {
        let mut widget = Widget::new();
        widget.scene.hide_cursor(true);

        widget.cursor_at(100.0, 100.0);
        assert_eq!(widget.interaction(), mouse::Interaction::Hidden);

        widget.cursor_at(1400.0, 100.0);
        assert_eq!(widget.interaction(), mouse::Interaction::default());

        widget.scene.hide_cursor(false);
        widget.cursor_at(100.0, 100.0);
        assert_eq!(widget.interaction(), mouse::Interaction::Grab);
    }

    /// The `View` menu has no cursor and no event, so its zoom reaches the
    /// camera through the scene, once, on the next redraw.
    #[test]
    fn a_nudge_reaches_the_camera_on_the_next_redraw() {
        let redraw = || Event::Window(window::Event::RedrawRequested(Instant::now()));
        let mut widget = Widget::new();

        widget.scene.nudge(Nudge::ZoomIn);
        assert_eq!(widget.camera(), Camera::default());

        widget.send(redraw());
        let zoomed = widget.camera();
        assert!(zoomed.fov < Camera::default().fov);

        widget.send(redraw());
        assert_eq!(widget.camera(), zoomed);
    }

    /// Issue #83, through the events iced delivers: a drag is held, the zoom
    /// key goes in, and the picture stays where the zoom left it until the
    /// hand asks for something else. The nudge used to re-take the drag's
    /// hold at the middle of the frame while the cursor was elsewhere, so the
    /// next move -- this one, which goes nowhere at all -- hauled the picture
    /// across.
    #[test]
    fn a_nudge_mid_drag_keeps_the_grab() {
        let mut widget = Widget::new();
        widget.press(200.0, 400.0).move_to(700.0, 150.0);

        widget.scene.nudge(Nudge::ZoomOut);
        widget.send(Event::Window(
            window::Event::RedrawRequested(Instant::now()),
        ));
        let zoomed = widget.camera();
        assert!(zoomed.fov > Camera::default().fov, "the key did not zoom");

        widget.move_to(700.0, 150.0);
        let moved = widget.camera();
        assert!(
            (moved.yaw - zoomed.yaw).abs() < 1e-4 && (moved.pitch - zoomed.pitch).abs() < 1e-4,
            "the cursor stayed put and the view went from {zoomed:?} to {moved:?}",
        );
    }

    /// Issue #92, the owner's steps in the events iced delivers: zoom all the
    /// way out to the ball, press on the picture, drag out into the room
    /// around it, scroll there, and keep dragging. The pan has to survive the
    /// scroll.
    ///
    /// It did not. The wheel re-takes the drag's hold at the cursor, the room
    /// has no direction under it to take hold of, and the drag was dropped
    /// altogether: the zoom worked and every move after it moved nothing.
    #[test]
    fn a_wheel_zoom_over_the_room_keeps_the_drag() {
        let mut widget = Widget::new();
        widget.scroll_to_the_ball();
        let ball = widget.camera();

        // Where the ball is, and where the room around it is: the picture
        // fills 40% of this window's width, so the middle is picture and a
        // point a seventh of the way across is not.
        let room = (150.0, 120.0);
        assert!(widget.looking_at(500.0, 250.0).is_some(), "no ball");
        assert!(
            widget.looking_at(room.0, room.1).is_none(),
            "the room around the ball is still picture",
        );

        widget.press(500.0, 250.0).move_to(room.0, room.1);
        let dragged = widget.camera();
        assert_ne!(dragged, ball, "the drag into the room moved nothing");

        widget.scroll(1.0);
        let zoomed = widget.camera();
        assert!(zoomed.fov < dragged.fov, "the scroll did not zoom");
        assert!(
            widget.scene.viewpoint().is_dragging(),
            "the scroll let go of the drag",
        );

        widget.move_to(room.0 + 200.0, room.1);
        assert_ne!(
            widget.camera(),
            zoomed,
            "the pan died at the scroll: the button is still down and the \
             cursor moved 200 px",
        );
    }

    /// The zoom is the widget's only other input, and it is not a grab: it
    /// answers where the pointer is, held or not.
    #[test]
    fn the_wheel_zooms_only_over_the_widget() {
        let scroll = Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
        });

        let mut widget = Widget::new();
        widget.cursor_at(1200.0, 100.0).send(scroll.clone());
        assert_eq!(widget.camera(), Camera::default());

        widget.cursor_at(100.0, 100.0).send(scroll);
        assert!(widget.camera().fov < Camera::default().fov);
    }
}
