//! How the shell hands this pass to iced, and how the mouse reaches it.
//!
//! These three impls put a foreign trait (`iced::widget::shader`) on types
//! this crate owns, so Rust's coherence rules require them to live here
//! rather than in `kyerag`: that, and nothing else, is why the render layer
//! names libcosmic. Nothing above this file decides anything about the pass
//! or the view direction; the shell only builds a `shader::Shader` around a
//! [`Scene`], and iced keeps the [`Viewpoint`] in its widget tree.

use cosmic::iced::widget::shader::{self, Action};
use cosmic::iced::{Event, Rectangle, mouse};

use super::{Scene, ScenePipeline, ScenePrimitive, Viewpoint};

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
        let Event::Mouse(event) = event else {
            return None;
        };
        match event {
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                let at = cursor.position_over(bounds)?;
                viewpoint.grab(at.x, at.y);
                Some(Action::request_redraw().and_capture())
            }
            // Deliberately not gated on the cursor being over the widget: a
            // pan that stopped at the window edge, mid-drag, would feel
            // broken.
            mouse::Event::CursorMoved { position } => viewpoint
                .drag_to(position.x, position.y, bounds.width)
                .then(Action::request_redraw),
            mouse::Event::ButtonReleased(mouse::Button::Left) => {
                viewpoint.release();
                None
            }
            mouse::Event::WheelScrolled { delta } => cursor.is_over(bounds).then(|| {
                viewpoint.zoom(steps(*delta));
                Action::request_redraw().and_capture()
            }),
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

fn steps(delta: mouse::ScrollDelta) -> f32 {
    match delta {
        mouse::ScrollDelta::Lines { y, .. } => y,
        mouse::ScrollDelta::Pixels { y, .. } => y / PIXELS_PER_LINE,
    }
}
