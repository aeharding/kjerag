//! A window-sized projection, clipped to the space left by COSMIC's chrome.
//!
//! The stock template puts the header above the content. When it auto-hides,
//! that content grows; projecting against its rectangle moves and rescales
//! the picture. The video instead looks through the same window rectangle
//! whether or not the header is there. Only visibility changes. Mouse rays
//! use that same rectangle, while presses and wheels still belong only to
//! the visible video. No header dimensions or theme measurements are cached.

use cosmic::iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, renderer, widget};
use cosmic::iced::widget::shader;
use cosmic::iced::{Element, Event, Length, Rectangle, Size};
use kjerag_render::Scene;

use crate::app::Message;

pub(crate) fn view(scene: &Scene) -> cosmic::Element<'_, Message> {
    Element::new(WindowProjection {
        video: shader::Shader::new(scene)
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
    })
}

/// Only wraps a leaf video widget, not the controls or the COSMIC shell.
struct WindowProjection<'a, Message, Theme, Renderer> {
    video: Element<'a, Message, Theme, Renderer>,
}

fn window_layout(viewport: &Rectangle) -> layout::Node {
    layout::Node::new(viewport.size()).move_to(viewport.position())
}

impl<Message, Theme, Renderer: renderer::Renderer> Widget<Message, Theme, Renderer>
    for WindowProjection<'_, Message, Theme, Renderer>
{
    fn tag(&self) -> widget::tree::Tag {
        self.video.as_widget().tag()
    }

    fn state(&self) -> widget::tree::State {
        self.video.as_widget().state()
    }

    fn children(&self) -> Vec<widget::Tree> {
        self.video.as_widget().children()
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        self.video.as_widget_mut().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.video.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.video.as_widget_mut().layout(tree, renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let cursor = if cursor.is_over(layout.bounds()) {
            cursor
        } else {
            mouse::Cursor::Unavailable
        };
        self.video.as_widget_mut().update(
            tree,
            event,
            Layout::new(&window_layout(viewport)),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        // Cursor visibility is a hit test, not a projection calculation.
        self.video
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if let Some(visible) = layout.bounds().intersection(viewport) {
            renderer.with_layer(visible, |renderer| {
                self.video.as_widget().draw(
                    tree,
                    renderer,
                    theme,
                    style,
                    Layout::new(&window_layout(viewport)),
                    cursor,
                    viewport,
                );
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use cosmic::iced::advanced::image;
    use cosmic::iced::{Background, Point, Transformation};

    use super::*;

    #[derive(Default)]
    struct Renderer {
        clips: Vec<Rectangle>,
    }

    impl renderer::Renderer for Renderer {
        fn start_layer(&mut self, bounds: Rectangle) {
            self.clips.push(bounds);
        }
        fn end_layer(&mut self) {
            self.clips.pop().unwrap();
        }
        fn start_transformation(&mut self, _transformation: Transformation) {}
        fn end_transformation(&mut self) {}
        fn reset(&mut self, _bounds: Rectangle) {}
        fn fill_quad(&mut self, _quad: renderer::Quad, _background: impl Into<Background>) {}
        fn allocate_image(
            &mut self,
            _handle: &image::Handle,
            _callback: impl FnOnce(Result<image::Allocation, image::Error>) + Send + 'static,
        ) {
            panic!("video layout does not allocate images");
        }
    }

    #[derive(Default)]
    struct Observed {
        input: RefCell<Vec<(Rectangle, mouse::Cursor)>>,
        draw: RefCell<Vec<(Rectangle, Rectangle)>>,
    }

    struct Video<'a>(&'a Observed);

    impl Widget<(), (), Renderer> for Video<'_> {
        fn size(&self) -> Size<Length> {
            Size::new(Length::Fill, Length::Fill)
        }
        fn layout(
            &mut self,
            _tree: &mut widget::Tree,
            _renderer: &Renderer,
            limits: &layout::Limits,
        ) -> layout::Node {
            layout::atomic(limits, Length::Fill, Length::Fill)
        }
        fn update(
            &mut self,
            _tree: &mut widget::Tree,
            _event: &Event,
            layout: Layout<'_>,
            cursor: mouse::Cursor,
            _renderer: &Renderer,
            _clipboard: &mut dyn Clipboard,
            _shell: &mut Shell<'_, ()>,
            _viewport: &Rectangle,
        ) {
            self.0.input.borrow_mut().push((layout.bounds(), cursor));
        }
        fn draw(
            &self,
            _tree: &widget::Tree,
            renderer: &mut Renderer,
            _theme: &(),
            _style: &renderer::Style,
            layout: Layout<'_>,
            _cursor: mouse::Cursor,
            _viewport: &Rectangle,
        ) {
            self.0
                .draw
                .borrow_mut()
                .push((layout.bounds(), *renderer.clips.last().unwrap()));
        }
    }

    struct NullClipboard;
    impl Clipboard for NullClipboard {
        fn read(&self, _kind: cosmic::iced::advanced::clipboard::Kind) -> Option<String> {
            None
        }
        fn write(&mut self, _kind: cosmic::iced::advanced::clipboard::Kind, _contents: String) {}
    }

    fn visit(observed: &Observed, window: Rectangle, visible: Rectangle, cursor: mouse::Cursor) {
        let mut projection = WindowProjection {
            video: Element::new(Video(observed)),
        };
        let mut tree = widget::Tree::new(&projection as &dyn Widget<(), (), Renderer>);
        let mut renderer = Renderer::default();
        let mut node = projection.layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, visible.size()),
        );
        node.move_to_mut(visible.position());
        assert_eq!(node.bounds(), visible, "chrome still owns content layout");
        let mut messages = Vec::new();
        projection.update(
            &mut tree,
            &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            Layout::new(&node),
            cursor,
            &renderer,
            &mut NullClipboard,
            &mut Shell::new(&mut messages),
            &window,
        );
        projection.draw(
            &tree,
            &mut renderer,
            &(),
            &renderer::Style::default(),
            Layout::new(&node),
            cursor,
            &window,
        );
        assert!(renderer.clips.is_empty());
    }

    #[test]
    fn showing_and_hiding_headers_preserves_draw_and_mouse_projection() {
        let observed = Observed::default();
        // Nonzero origin and fractional logical size also exercise scaled
        // windows. Header size comes from layout, never a hard-coded 48.
        let window = Rectangle::new(Point::new(1.5, 2.5), Size::new(853.5, 480.5));
        let cursor = mouse::Cursor::Available(Point::new(400.0, 300.0));
        for header in [0.0, 48.0, 64.0, 0.0] {
            let visible = Rectangle {
                y: window.y + header,
                height: window.height - header,
                ..window
            };
            visit(&observed, window, visible, cursor);
            assert_eq!(*observed.input.borrow().last().unwrap(), (window, cursor));
            assert_eq!(*observed.draw.borrow().last().unwrap(), (window, visible));
        }
    }

    #[test]
    fn the_header_does_not_start_a_video_grab_or_zoom() {
        let observed = Observed::default();
        let window = Rectangle::with_size(Size::new(1280.0, 720.0));
        let visible = Rectangle {
            y: 48.0,
            height: 672.0,
            ..window
        };
        visit(
            &observed,
            window,
            visible,
            mouse::Cursor::Available(Point::new(300.0, 20.0)),
        );
        assert_eq!(
            observed.input.borrow().as_slice(),
            &[(window, mouse::Cursor::Unavailable)]
        );
    }

    #[test]
    fn a_real_window_resize_changes_both_projections_without_cached_geometry() {
        let observed = Observed::default();
        for size in [Size::new(1280.0, 720.0), Size::new(960.0, 540.0)] {
            let window = Rectangle::with_size(size);
            let visible = Rectangle {
                x: 1.0,
                y: 49.0,
                width: size.width - 2.0,
                height: size.height - 50.0,
            };
            visit(&observed, window, visible, mouse::Cursor::Unavailable);
            assert_eq!(observed.input.borrow().last().unwrap().0, window);
            assert_eq!(observed.draw.borrow().last().unwrap().0, window);
        }
    }
}
