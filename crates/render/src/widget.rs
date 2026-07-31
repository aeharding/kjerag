//! How the shell hands this pass to iced, and how the mouse reaches it.
//!
//! These three impls put a foreign trait (`iced::widget::shader`) on types
//! this crate owns, so Rust's coherence rules require them to live here
//! rather than in `kyerag`: that, and nothing else, is why the render layer
//! names libcosmic. Nothing above this file decides anything about the pass
//! or the view direction; the shell only builds a `shader::Shader` around a
//! [`Scene`], and iced keeps the [`Viewpoint`] in its widget tree.

use std::time::Instant;

use cosmic::iced::widget::shader::{self, Action};
use cosmic::iced::{Event, Point, Rectangle, mouse, window};

use super::{Next, Scene, ScenePipeline, ScenePrimitive, Viewpoint};

/// Wheels report scroll in lines and touchpads report it in pixels, and iced
/// passes both through as they came. A feel constant, not a measurement.
const PIXELS_PER_LINE: f32 = 40.0;

impl<Message> shader::Program<Message> for Scene {
    type State = Viewpoint;
    type Primitive = ScenePrimitive;

    fn update(
        &self,
        viewpoint: &mut Viewpoint,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        match event {
            Event::Mouse(event) => mouse_update(viewpoint, event, bounds, cursor),
            Event::Window(window::Event::RedrawRequested(now)) => tick(self, *now),
            // Alt-tabbing away mid-drag takes the release with it, and a grab
            // nothing can end is a camera glued to the cursor.
            Event::Window(window::Event::Unfocused) => {
                viewpoint.release();
                None
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        viewpoint: &Viewpoint,
        _cursor: mouse::Cursor,
        _bounds: Rectangle,
    ) -> ScenePrimitive {
        self.primitive(viewpoint.camera())
    }

    fn mouse_interaction(
        &self,
        viewpoint: &Viewpoint,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        match (viewpoint.is_dragging(), cursor.is_over(bounds)) {
            (true, _) => mouse::Interaction::Grabbing,
            (false, true) => mouse::Interaction::Grab,
            (false, false) => mouse::Interaction::default(),
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
    viewpoint: &mut Viewpoint,
    event: &mouse::Event,
    bounds: Rectangle,
    cursor: mouse::Cursor,
) -> Option<Action<Message>> {
    match event {
        mouse::Event::ButtonPressed(mouse::Button::Left) => {
            let at = cursor.position_over(bounds)?;
            viewpoint.grab(uv(at, bounds), aspect(bounds));
            Some(Action::request_redraw().and_capture())
        }
        // Deliberately not gated on the cursor being over the widget: a pan
        // that stopped at the window edge, mid-drag, would feel broken. It is
        // the drag that decides whether this moves anything, not the cursor.
        // The `uv` of a cursor outside the widget is outside 0 to 1, which is
        // a direction outside the view, which is exactly what the drag means.
        mouse::Event::CursorMoved { position } => viewpoint
            .drag_to(uv(*position, bounds), aspect(bounds))
            .then(Action::request_redraw),
        // The release ends the grab wherever it lands, including off the
        // widget. `CursorLeft` is the case where it cannot land here at all:
        // the pointer left the window still held, so the release will be some
        // other window's.
        mouse::Event::ButtonReleased(mouse::Button::Left) | mouse::Event::CursorLeft => {
            viewpoint.release();
            None
        }
        mouse::Event::WheelScrolled { delta } => {
            let at = cursor.position_over(bounds)?;
            viewpoint.zoom(steps(*delta), uv(at, bounds), aspect(bounds));
            Some(Action::request_redraw().and_capture())
        }
        _ => None,
    }
}

/// The presentation clock ticks on the window's own redraw event, inside the
/// pass that then draws the result: the scene takes the frame that is due and
/// says when the next one is, and the returned [`Action`] is what makes iced
/// sleep until exactly that instant. Waking per frame rather than per refresh
/// is what keeps 29.97 fps content off a 60 Hz grid; `kyerag::app` documents
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
    use crate::Camera;

    const BOUNDS: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1000.0,
        height: 500.0,
    };

    struct Widget {
        scene: Scene,
        viewpoint: Viewpoint,
        cursor: mouse::Cursor,
    }

    impl Widget {
        fn new() -> Self {
            Self {
                scene: Scene::blank(),
                viewpoint: Viewpoint::default(),
                cursor: mouse::Cursor::Available(Point::ORIGIN),
            }
        }

        /// Where iced says the pointer is when the next event is handled.
        fn cursor_at(&mut self, x: f32, y: f32) -> &mut Self {
            self.cursor = mouse::Cursor::Available(Point::new(x, y));
            self
        }

        fn send(&mut self, event: Event) -> &mut Self {
            let _: Option<Action<()>> = shader::Program::update(
                &self.scene,
                &mut self.viewpoint,
                &event,
                BOUNDS,
                self.cursor,
            );
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

        fn camera(&self) -> Camera {
            self.viewpoint.camera()
        }

        fn interaction(&self) -> mouse::Interaction {
            shader::Program::<()>::mouse_interaction(
                &self.scene,
                &self.viewpoint,
                BOUNDS,
                self.cursor,
            )
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
