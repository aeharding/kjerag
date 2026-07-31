//! The libcosmic shell: one window whose whole body is an iced shader widget.
//!
//! This is the M0 bring-up surface, not the player. It exists to prove two
//! things under cosmic-comp: that a custom wgpu pass runs inside libcosmic at
//! all, and that a VA-API frame imported from a dmabuf can be sampled by that
//! pass on the device iced created (see [`crate::render::dmabuf`]).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cosmic::app::{Core, Settings, Task};
use cosmic::iced::widget::shader;
use cosmic::iced::{Length, Rectangle, Subscription, mouse};
use cosmic::{Element, executor};

use crate::render::{Frame, Scene, ScenePipeline, ScenePrimitive};

/// Runs the shell. With no path the widget draws an animated WGSL gradient
/// and nothing is decoded; with one it shows the first frame of stream 0.
pub fn run(input: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let limits = cosmic::iced::Limits::NONE
        .min_width(360.0)
        .min_height(240.0);
    cosmic::app::run::<App>(Settings::default().size_limits(limits), input)?;
    Ok(())
}

/// 60 Hz. The player will drive redraws off the frame clock; the bring-up
/// surface only needs the gradient to visibly move.
const TICK: Duration = Duration::from_millis(16);

#[derive(Clone, Debug)]
pub enum Message {
    Tick,
}

pub struct App {
    core: Core,
    scene: Scene,
}

impl cosmic::Application for App {
    type Executor = executor::Default;
    type Flags = Option<PathBuf>;
    type Message = Message;

    const APP_ID: &'static str = "app.kyerag.Kyerag";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(mut core: Core, input: Self::Flags) -> (Self, Task<Self::Message>) {
        // libcosmic's content container insets the app's view by
        // `border_padding` on the right and, because `nav_bar.active`
        // defaults to true even for an app with no nav model, by nothing on
        // the left (`app/mod.rs`, `main_content_padding`). Measured at scale
        // 1.25: 1 physical px of window border on the left against 10 on the
        // right. Video wants both edges, so the container comes off.
        core.window.content_container = false;
        let frame = input.map(|path| Arc::new(Frame::pending(path)));
        (
            App {
                core,
                scene: Scene::new(frame),
            },
            Task::none(),
        )
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::Tick => self.scene.advance(TICK),
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        cosmic::iced::time::every(TICK).map(|_| Message::Tick)
    }

    fn view(&self) -> Element<'_, Self::Message> {
        Element::from(
            shader::Shader::new(&self.scene)
                .width(Length::Fill)
                .height(Length::Fill),
        )
    }
}

impl shader::Program<Message> for Scene {
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
