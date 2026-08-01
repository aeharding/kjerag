//! SCRATCH INSTRUMENT (issue #102), not for merge.
//!
//! A pass-through widget that says when it was updated and when it was drawn,
//! with the bounds and viewport it got. Wrapped round two places in the
//! window's tree, it says which level of the tree stopped drawing on the
//! redraw that left a paused window empty.

use cosmic::iced::advanced::widget::{Operation, Tree, Widget, tree};
use cosmic::iced::advanced::{Clipboard, Layout, Shell, layout, mouse, overlay, renderer};
use cosmic::iced::{Event, Length, Rectangle, Size, Vector};
use cosmic::{Element, Renderer, Theme};

pub struct Probe<'a, Message> {
    label: &'static str,
    content: Element<'a, Message>,
}

pub fn probe<'a, Message>(
    label: &'static str,
    content: impl Into<Element<'a, Message>>,
) -> Probe<'a, Message> {
    Probe {
        label,
        content: content.into(),
    }
}

fn say(label: &str, what: &str, bounds: Rectangle, viewport: &Rectangle) {
    kjerag_render::trace(|| {
        format!(
            "probe[{label}] {what} bounds={}x{}+{}+{} viewport={}x{}+{}+{}",
            bounds.width,
            bounds.height,
            bounds.x,
            bounds.y,
            viewport.width,
            viewport.height,
            viewport.x,
            viewport.y,
        )
    });
}

impl<Message> Widget<Message, Theme, Renderer> for Probe<'_, Message> {
    fn tag(&self) -> tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.content.as_widget().children()
    }

    fn diff(&mut self, tree: &mut Tree) {
        self.content.as_widget_mut().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }

    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if matches!(
            event,
            Event::Window(cosmic::iced::window::Event::RedrawRequested(_))
        ) {
            say(self.label, "update", layout.bounds(), viewport);
        }
        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        say(self.label, "draw", layout.bounds(), viewport);
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

impl<'a, Message: 'a> From<Probe<'a, Message>> for Element<'a, Message> {
    fn from(probe: Probe<'a, Message>) -> Self {
        Element::new(probe)
    }
}
