//! A thin wrapper widget that turns the platform input method on over the
//! embedded terminal and forwards composed text — dead/accent keys, CJK, any
//! sequence the OS resolves through composition — as a message.
//!
//! The terminal is an [`iced::widget::Canvas`], whose `Program` API can't reach
//! the [`Shell`] to request an input method. Without that request macOS (and the
//! IME daemons on Linux/Windows) never composes: a dead key like `^` is swallowed
//! waiting for a composition that never starts, so non-US layouts can type
//! nothing. This wrapper requests the IME while the terminal holds focus and
//! turns each `Commit` into a message carrying the resolved text; ordinary
//! keystrokes keep flowing through the keyboard path untouched (with the IME on,
//! the platform still delivers un-composed keys as normal key events).
//!
//! While a composition is pending (a dead key waiting for its second key) the
//! wrapper hands its text back with the request, and the runtime draws it
//! underlined over the cursor cell ("over-the-spot"), at the terminal's text
//! size but in the UI font.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{
    Clipboard, InputMethod, Shell, Widget, input_method, mouse, overlay, renderer,
};
use iced::{Element, Event, Length, Pixels, Rectangle, Size, Vector, keyboard, window};
use std::ops::Range;

/// Wrap `content` (the terminal canvas) so the IME is requested while `enabled`
/// (i.e. the terminal holds focus) and each composed `Commit` is mapped to a
/// message by `on_commit`. `caret`/`cell` place the IME candidate window over
/// the terminal cursor so a composing overlay does not cover the typed cell;
/// `text_size` is the terminal's, for the pending composition drawn there.
pub(super) fn ime_area<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    enabled: bool,
    caret: Option<(u16, u16)>,
    cell: Size,
    text_size: f32,
    on_commit: impl Fn(String) -> Message + 'a,
) -> ImeArea<'a, Message> {
    ImeArea {
        content: content.into(),
        enabled,
        caret,
        cell,
        text_size: Pixels(text_size),
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
    text_size: Pixels,
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

/// The input method the focused terminal asks the runtime for: anchored on
/// the `cursor` cell, with whatever is still composing for the runtime to draw
/// over that cell.
pub(super) fn requested_input_method(
    cursor: Rectangle,
    composition: &Composition,
    text_size: Pixels,
) -> InputMethod {
    InputMethod::Enabled {
        cursor,
        purpose: input_method::Purpose::Terminal,
        preedit: composition.preedit(text_size),
    }
}

/// The text the input method is composing and has not committed yet: what a
/// dead key holds until its second key resolves it.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Composition {
    pending: Option<(String, Option<Range<usize>>)>,
}

impl Composition {
    /// Follow `event` while the area is `enabled`; whether the pending text
    /// changed, so the caller knows a redraw must show it. Anything that ends
    /// or abandons the composition clears it: the commit, an emptied pre-edit,
    /// the input method closing, Escape, the window losing focus, and the
    /// area no longer taking terminal input.
    pub(super) fn follow(&mut self, enabled: bool, event: &Event) -> bool {
        let next = match event {
            _ if !enabled => None,
            Event::InputMethod(input_method::Event::Preedit(text, selection))
                if !text.is_empty() =>
            {
                // The runtime slices the text with the selection, so one off
                // its char boundaries would panic there.
                let selection = selection
                    .clone()
                    .filter(|range| text.get(range.clone()).is_some());
                Some((text.clone(), selection))
            }
            Event::InputMethod(
                input_method::Event::Preedit(..)
                | input_method::Event::Commit(_)
                | input_method::Event::Closed,
            )
            | Event::Window(window::Event::Unfocused)
            | Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) => None,
            _ => return false,
        };
        let changed = self.pending != next;
        self.pending = next;
        changed
    }

    /// The pending text as the runtime's over-the-spot pre-edit, at the
    /// terminal's `text_size`; `Preedit` carries no font, so the runtime draws
    /// it in the UI one.
    pub(super) fn preedit(&self, text_size: Pixels) -> Option<input_method::Preedit> {
        self.pending
            .as_ref()
            .map(|(content, selection)| input_method::Preedit {
                content: content.clone(),
                selection: selection.clone(),
                text_size: Some(text_size),
            })
    }
}

/// What the area does with one event: redraw to show a changed composition,
/// send a commit to the terminal, or (re)assert its input method.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Reaction {
    redraw: bool,
    commit: Option<String>,
    request: Option<InputMethod>,
}

/// The area's step for `event`, with the IME anchored on the `cursor` cell.
pub(super) fn react(
    composition: &mut Composition,
    enabled: bool,
    event: &Event,
    cursor: Rectangle,
    text_size: Pixels,
) -> Reaction {
    let redraw = composition.follow(enabled, event);
    let commit = terminal_commit(enabled, event).map(str::to_owned);
    // The IME request is only honoured during a redraw, so (re)assert it
    // every frame the terminal is focused; dropping it would let the
    // runtime disable the IME again and re-break dead keys.
    let request = (enabled && matches!(event, Event::Window(window::Event::RedrawRequested(_))))
        .then(|| requested_input_method(cursor, composition, text_size));
    Reaction {
        redraw,
        commit,
        request,
    }
}

/// The composed text the terminal takes from `event`, if any. A disabled area
/// takes nothing, so while an overlay owns the keyboard the commit travels on
/// to the focused input field instead of being captured for the PTY. A
/// pre-edit is not a commit: [`Composition`] keeps it for display only.
pub(super) fn terminal_commit(enabled: bool, event: &Event) -> Option<&str> {
    match event {
        Event::InputMethod(input_method::Event::Commit(text)) if enabled && !text.is_empty() => {
            Some(text)
        }
        _ => None,
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for ImeArea<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Composition>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Composition::default())
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

        let reaction = react(
            tree.state.downcast_mut::<Composition>(),
            self.enabled,
            event,
            self.ime_cursor(layout.bounds()),
            self.text_size,
        );
        if reaction.redraw {
            shell.request_redraw();
        }
        if let Some(text) = reaction.commit {
            shell.publish((self.on_commit)(text));
            shell.capture_event();
        }
        if let Some(ime) = reaction.request {
            shell.request_input_method(&ime);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(text: &str) -> Event {
        Event::InputMethod(input_method::Event::Commit(text.to_owned()))
    }

    #[test]
    fn an_enabled_area_takes_a_commit_for_the_terminal() {
        assert_eq!(terminal_commit(true, &commit("ê")), Some("ê"));
    }

    #[test]
    fn a_disabled_area_takes_no_commit() {
        assert_eq!(terminal_commit(false, &commit("🚀")), None);
    }

    #[test]
    fn neither_an_empty_commit_nor_a_preedit_is_taken() {
        assert_eq!(terminal_commit(true, &commit("")), None);
        let preedit = Event::InputMethod(input_method::Event::Preedit("^".to_owned(), None));
        assert_eq!(terminal_commit(true, &preedit), None);
    }

    fn preedit(text: &str, selection: Option<Range<usize>>) -> Event {
        Event::InputMethod(input_method::Event::Preedit(text.to_owned(), selection))
    }

    fn composing(text: &str) -> Composition {
        let mut composition = Composition::default();
        assert!(composition.follow(true, &preedit(text, Some(text.len()..text.len()))));
        composition
    }

    fn escape() -> Event {
        use iced::keyboard::key::{Code, Named, Physical};
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(Named::Escape),
            modified_key: keyboard::Key::Named(Named::Escape),
            physical_key: Physical::Code(Code::Escape),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::default(),
            text: None,
            repeat: false,
        })
    }

    #[test]
    fn a_dead_key_preedit_is_offered_at_the_terminal_text_size() {
        let composition = composing("^");
        assert_eq!(
            composition.preedit(Pixels(15.0)),
            Some(input_method::Preedit {
                content: "^".to_owned(),
                selection: Some(1..1),
                text_size: Some(Pixels(15.0)),
            })
        );
    }

    #[test]
    fn the_requested_input_method_carries_the_preedit() {
        let cursor = Rectangle::new(iced::Point::new(9.0, 18.0), Size::new(9.0, 18.0));
        let composition = composing("¨");
        let InputMethod::Enabled {
            cursor: at,
            purpose,
            preedit,
        } = requested_input_method(cursor, &composition, Pixels(14.0))
        else {
            panic!("the focused terminal asks for an enabled input method");
        };
        assert_eq!(at, cursor);
        assert_eq!(purpose, input_method::Purpose::Terminal);
        assert_eq!(preedit, composition.preedit(Pixels(14.0)));
        assert_eq!(preedit.map(|p| p.content), Some("¨".to_owned()));
    }

    #[test]
    fn nothing_composing_requests_no_preedit() {
        let cursor = Rectangle::new(iced::Point::ORIGIN, Size::new(9.0, 18.0));
        let ime = requested_input_method(cursor, &Composition::default(), Pixels(14.0));
        assert!(matches!(ime, InputMethod::Enabled { preedit: None, .. }));
    }

    #[test]
    fn a_commit_clears_the_preedit() {
        let mut composition = composing("^");
        assert!(composition.follow(true, &commit("ê")));
        assert_eq!(composition.preedit(Pixels(14.0)), None);
    }

    #[test]
    fn an_empty_preedit_clears_the_preedit() {
        let mut composition = composing("`");
        assert!(composition.follow(true, &preedit("", None)));
        assert_eq!(composition, Composition::default());
    }

    #[test]
    fn closing_the_input_method_escape_and_focus_loss_clear_the_preedit() {
        let closed = Event::InputMethod(input_method::Event::Closed);
        let unfocused = Event::Window(window::Event::Unfocused);
        for cancel in [closed, escape(), unfocused] {
            let mut composition = composing("~");
            assert!(composition.follow(true, &cancel), "{cancel:?} cancels");
            assert_eq!(composition, Composition::default(), "{cancel:?} cancels");
        }
    }

    #[test]
    fn a_disabled_area_keeps_no_preedit() {
        let mut composition = composing("^");
        let redraw = Event::Window(window::Event::RedrawRequested(std::time::Instant::now()));
        assert!(composition.follow(false, &redraw));
        assert_eq!(composition, Composition::default());

        assert!(!composition.follow(false, &preedit("^", None)));
        assert_eq!(composition, Composition::default());
    }

    #[test]
    fn an_unrelated_event_leaves_the_preedit_alone() {
        let mut composition = composing("^");
        let opened = Event::InputMethod(input_method::Event::Opened);
        assert!(!composition.follow(true, &opened));
        assert_eq!(composition, composing("^"));
    }

    #[test]
    fn a_repeated_preedit_is_no_change() {
        let mut composition = composing("^");
        assert!(!composition.follow(true, &preedit("^", Some(1..1))));
        assert!(composition.follow(true, &preedit("^", None)));
    }

    #[test]
    fn a_selection_off_the_char_boundaries_is_dropped() {
        for selection in [1..1, 0..9] {
            let mut composition = Composition::default();
            assert!(composition.follow(true, &preedit("ê", Some(selection.clone()))));
            let kept = composition.preedit(Pixels(14.0)).map(|p| p.selection);
            assert_eq!(kept, Some(None), "{selection:?}");
        }
        let kept = composing("ê").preedit(Pixels(14.0)).map(|p| p.selection);
        assert_eq!(kept, Some(Some(2..2)), "a boundary selection is kept");
    }

    fn redraw() -> Event {
        Event::Window(window::Event::RedrawRequested(std::time::Instant::now()))
    }

    const CELL: Rectangle = Rectangle {
        x: 18.0,
        y: 36.0,
        width: 9.0,
        height: 18.0,
    };

    #[test]
    fn a_focused_redraw_reasserts_the_input_method_with_the_preedit() {
        let mut composition = composing("^");
        let reaction = react(&mut composition, true, &redraw(), CELL, Pixels(14.0));
        let expected = requested_input_method(CELL, &composing("^"), Pixels(14.0));
        assert_eq!(
            reaction,
            Reaction {
                redraw: false,
                commit: None,
                request: Some(expected),
            }
        );
    }

    #[test]
    fn only_a_focused_redraw_requests_the_input_method() {
        let mut composition = Composition::default();
        let disabled = react(&mut composition, false, &redraw(), CELL, Pixels(14.0));
        assert_eq!(disabled.request, None);
        let opened = Event::InputMethod(input_method::Event::Opened);
        let other = react(&mut composition, true, &opened, CELL, Pixels(14.0));
        assert_eq!(other, Reaction::default());
    }

    #[test]
    fn a_dead_key_asks_for_a_redraw_and_its_commit_reaches_the_terminal() {
        let mut composition = Composition::default();
        let dead_key = react(
            &mut composition,
            true,
            &preedit("^", None),
            CELL,
            Pixels(14.0),
        );
        assert!(dead_key.redraw && dead_key.commit.is_none());

        let resolved = react(&mut composition, true, &commit("ê"), CELL, Pixels(14.0));
        assert_eq!(
            resolved,
            Reaction {
                redraw: true,
                commit: Some("ê".to_owned()),
                request: None,
            }
        );
    }
}
