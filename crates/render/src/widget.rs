//! How the shell hands this pass to iced.
//!
//! These three impls put a foreign trait (`iced::widget::shader`) on types
//! this crate owns, so Rust's coherence rules require them to live here
//! rather than in `kyerag`: that, and nothing else, is why the render layer
//! names libcosmic. Nothing above this file decides anything about the pass;
//! the shell only builds a `shader::Shader` around a [`Scene`].

use cosmic::iced::widget::shader;
use cosmic::iced::{Rectangle, mouse};

use super::{Scene, ScenePipeline, ScenePrimitive};

impl<Message> shader::Program<Message> for Scene {
    type State = ();
    type Primitive = ScenePrimitive;

    fn draw(&self, _state: &(), _cursor: mouse::Cursor, _bounds: Rectangle) -> ScenePrimitive {
        self.primitive()
    }
}

impl shader::Primitive for ScenePrimitive {
    type Pipeline = ScenePipeline;

    fn prepare(
        &self,
        pipeline: &mut ScenePipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _bounds: &Rectangle,
        _viewport: &shader::Viewport,
    ) {
        pipeline.prepare(self, device, queue);
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
