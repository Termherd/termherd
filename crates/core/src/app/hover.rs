//! What the pointer is hovering in a terminal grid with the link modifier
//! (Ctrl/Cmd) held: the span to underline, and what a click would open.
//!
//! The span is *found* in the shell — only it holds the grid — but the answer
//! lives here, so exactly one place decides what is underlined. That matters
//! the moment a target's validity is not a pure function of the row text: a
//! file path has to be checked against the filesystem, which is an adapter's
//! job and comes back as an [`Event`], and an event can only land in `core`.

use super::*;

/// A clickable target under the pointer: which cells to underline, and what
/// activating it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermHover {
    /// The session whose grid the span belongs to. The canvas widget is reused
    /// across tabs, so a hover carries its owner rather than being implicitly
    /// the focused one.
    pub session: SessionId,
    /// The grid row, in viewport coordinates.
    pub row: u16,
    /// Column span `[start, end)` — the terminal stores one char per cell, so
    /// these are also character indices into the row's text.
    pub start: u16,
    pub end: u16,
    /// What a Ctrl/Cmd+click on the span would open.
    pub target: HoverTarget,
}

/// What activating a hovered span opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoverTarget {
    /// A URL, handed to the OS default handler.
    Url(String),
}

impl App {
    /// Record what the pointer is hovering, or [`None`] when it left every
    /// target. Pure state: the underline is drawn from it and the pointer turns
    /// into a hand over it.
    pub(super) fn set_term_hover(&mut self, hover: Option<TermHover>) -> Vec<Effect> {
        self.hover = hover;
        Vec::new()
    }

    /// The clickable target under the pointer, if any.
    #[must_use]
    pub const fn term_hover(&self) -> Option<&TermHover> {
        self.hover.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::*;

    fn url_hover(session: SessionId) -> TermHover {
        TermHover {
            session,
            row: 3,
            start: 4,
            end: 17,
            target: HoverTarget::Url("https://ex.io".into()),
        }
    }

    #[test]
    fn hovering_a_url_records_it_and_yields_no_effects() {
        let mut app = App::new();
        let hover = url_hover(sid(1));
        assert!(
            app.apply(Event::TermHover(Some(hover.clone()))).is_empty(),
            "a hover is pure state — nothing to perform"
        );
        assert_eq!(app.term_hover(), Some(&hover));
    }

    #[test]
    fn leaving_every_target_clears_the_hover() {
        let mut app = App::new();
        app.apply(Event::TermHover(Some(url_hover(sid(1)))));
        assert!(app.apply(Event::TermHover(None)).is_empty());
        assert_eq!(app.term_hover(), None);
    }

    #[test]
    fn opening_a_link_consumes_the_hover() {
        // The OS handoff often steals focus, so no later pointer event can be
        // relied on to reconcile the hover: activating clears it, or the
        // underline and the hand cursor outlive the gesture.
        let mut app = App::new();
        app.apply(Event::TermHover(Some(url_hover(sid(1)))));
        app.apply(Event::OpenUrl("https://ex.io".into()));
        assert_eq!(app.term_hover(), None);
    }

    #[test]
    fn a_hover_replaces_the_previous_one() {
        // Moving from one link to another leaves exactly one underlined span,
        // not two — the field is the single source of truth, not an accumulator.
        let mut app = App::new();
        app.apply(Event::TermHover(Some(url_hover(sid(1)))));
        let next = TermHover {
            row: 9,
            ..url_hover(sid(2))
        };
        app.apply(Event::TermHover(Some(next.clone())));
        assert_eq!(app.term_hover(), Some(&next));
    }
}
