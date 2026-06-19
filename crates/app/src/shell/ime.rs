//! A thin wrapper widget that turns the platform input method on over the
//! embedded terminal and forwards composed text — dead/accent keys, CJK, any
//! sequence the OS resolves through composition — as a message (#34).
//!
//! The terminal is an [`iced::widget::Canvas`], whose `Program` API can't reach
//! the [`Shell`] to request an input method. Without that request macOS (and the
//! IME daemons on Linux/Windows) never composes: a dead key like `^` is swallowed
//! waiting for a composition that never starts, so non-US layouts can type
//! nothing. This wrapper requests the IME while the terminal holds focus and
//! turns each `Commit` into a message carrying the resolved text; ordinary
//! keystrokes keep flowing through the keyboard path untouched (with the IME on,
//! the platform still delivers un-composed keys as normal key events).

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{
    Clipboard, InputMethod, Shell, Widget, input_method, mouse, overlay, renderer,
};
use iced::{Element, Event, Length, Rectangle, Size, Vector, window};

/// Wrap `content` (the terminal canvas) so the IME is requested while `enabled`
/// (i.e. the terminal holds focus) and each composed `Commit` is mapped to a
/// message by `on_commit`. `caret`/`cell` place the IME candidate window over
/// the terminal cursor so a composing overlay does not cover the typed cell.
pub(super) fn ime_area<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    enabled: bool,
    caret: Option<(u16, u16)>,
    cell: Size,
    on_commit: impl Fn(String) -> Message + 'a,
) -> ImeArea<'a, Message> {
    ImeArea {
        content: content.into(),
        enabled,
        caret,
        cell,
        on_commit: Box::new(on_commit),
    }
}

/// See [`ime_area`]. Holds the wrapped terminal element plus what it needs to
/// request the IME and translate commits.
pub(super) struct ImeArea<'a, Message> {
    content: Element<'a, Message>,
    enabled: bool,
    caret: Option<(u16, u16)>,
    cell: Size,
    on_commit: Box<dyn Fn(String) -> Message + 'a>,
}

impl<Message> ImeArea<'_, Message> {
    /// The screen rectangle of the terminal cursor cell, where the OS should
    /// anchor the IME candidate window; the top-left cell when no cursor shows.
    fn ime_cursor(&self, bounds: Rectangle) -> Rectangle {
        let (col, row) = self.caret.unwrap_or((0, 0));
        Rectangle {
            x: bounds.x + f32::from(col) * self.cell.width,
            y: bounds.y + f32::from(row) * self.cell.height,
            width: self.cell.width,
            height: self.cell.height,
        }
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for ImeArea<'_, Message> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::stateless()
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );

        if !self.enabled {
            return;
        }
        match event {
            // The composed result of a dead-key / IME sequence: hand it to the
            // PTY as typed text. The empty pre-edit that always precedes a commit
            // is ignored — we do not render an on-the-spot pre-edit overlay.
            Event::InputMethod(input_method::Event::Commit(text)) if !text.is_empty() => {
                shell.publish((self.on_commit)(text.clone()));
                shell.capture_event();
            }
            // The IME request is only honoured during a redraw, so (re)assert it
            // every frame the terminal is focused; dropping it would let the
            // runtime disable the IME again and re-break dead keys.
            Event::Window(window::Event::RedrawRequested(_)) => {
                let ime = InputMethod::<String>::Enabled {
                    cursor: self.ime_cursor(layout.bounds()),
                    purpose: input_method::Purpose::Terminal,
                    preedit: None,
                };
                shell.request_input_method(&ime);
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: 'a> From<ImeArea<'a, Message>> for Element<'a, Message> {
    fn from(area: ImeArea<'a, Message>) -> Self {
        Element::new(area)
    }
}
