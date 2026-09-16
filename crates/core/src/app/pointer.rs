//! A pointer event addressed to a terminal by **cell**, and what it does to the
//! terminal's own text selection when the child is not reading the mouse.
//!
//! Cell-addressed because a terminal is a grid and a grid is what a mouse
//! report carries — so the same event serves a caller that has no pixels (an
//! agent over MCP) and, later, the encoder that forwards it to the child. The
//! grid-line conversion (`row − display_offset`) is done by whoever holds the
//! *live* offset, which is why [`pointer_select`] takes it as an argument.

use super::{SelectOp, SelectSide};

/// What kind of pointer gesture landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    /// The button went down.
    Press,
    /// The button came up.
    Release,
    /// A press and release at the same cell, as one event.
    Click,
    /// The pointer moved with the button held.
    Drag,
    /// The pointer moved with no button held.
    Move,
}

/// Which button the gesture is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PointerButton {
    #[default]
    Left,
    Middle,
    Right,
}

/// Modifier keys held during the gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PointerModifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// One pointer event at a visible cell of a session's terminal. `col`/`row`
/// are 0-based in the **visible** screen, not the scrollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerEvent {
    pub kind: PointerKind,
    pub col: u16,
    pub row: u16,
    pub button: PointerButton,
    pub modifiers: PointerModifiers,
}

/// The selection change a pointer event drives when the child is **not**
/// reading the mouse, or `None` when it drives nothing locally. The one place
/// that rule lives: the terminal applies it and the shell reports it, so the
/// answer a caller gets cannot drift from what the grid did.
#[must_use]
pub fn pointer_select(pointer: &PointerEvent, display_offset: usize) -> Option<SelectOp> {
    if pointer.button != PointerButton::Left {
        return None;
    }
    // The offset can exceed `i32` only past two billion lines of scrollback,
    // where saturating is the honest answer.
    let line = i32::from(pointer.row) - i32::try_from(display_offset).unwrap_or(i32::MAX);
    let col = usize::from(pointer.col);
    // A cell has no halves for a caller addressing it by index, so the sides
    // are chosen to make a forward drag **inclusive** of both cells: the
    // terminal reads a `Left` endpoint as "before this cell" and a `Right`
    // one as "after it".
    match pointer.kind {
        PointerKind::Press => Some(SelectOp::Start {
            line,
            col,
            side: SelectSide::Left,
        }),
        PointerKind::Drag => Some(SelectOp::Update {
            line,
            col,
            side: SelectSide::Right,
        }),
        PointerKind::Click => Some(SelectOp::Clear),
        PointerKind::Release | PointerKind::Move => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn at(kind: PointerKind, button: PointerButton, col: u16, row: u16) -> PointerEvent {
        PointerEvent {
            kind,
            col,
            row,
            button,
            modifiers: PointerModifiers::default(),
        }
    }

    #[test]
    fn a_left_press_starts_a_selection_at_the_cell() {
        assert_eq!(
            pointer_select(&at(PointerKind::Press, PointerButton::Left, 4, 2), 0),
            Some(SelectOp::Start {
                line: 2,
                col: 4,
                side: SelectSide::Left,
            })
        );
    }

    #[test]
    fn a_left_drag_extends_the_selection_through_the_cell() {
        // `Right`, so the cell dragged to is part of the selection — a caller
        // naming a cell means that cell, not the gap before it.
        assert_eq!(
            pointer_select(&at(PointerKind::Drag, PointerButton::Left, 9, 3), 0),
            Some(SelectOp::Update {
                line: 3,
                col: 9,
                side: SelectSide::Right,
            })
        );
    }

    #[test]
    fn a_bare_left_click_clears_the_selection() {
        assert_eq!(
            pointer_select(&at(PointerKind::Click, PointerButton::Left, 1, 1), 0),
            Some(SelectOp::Clear)
        );
    }

    #[test]
    fn a_release_and_a_move_drive_nothing() {
        // The selection stands on release — copying it is the caller's `copy`
        // — and a hover is nothing to a terminal not reading the mouse.
        for kind in [PointerKind::Release, PointerKind::Move] {
            assert_eq!(
                pointer_select(&at(kind, PointerButton::Left, 1, 1), 0),
                None,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn only_the_left_button_drives_the_selection() {
        for button in [PointerButton::Middle, PointerButton::Right] {
            for kind in [
                PointerKind::Press,
                PointerKind::Release,
                PointerKind::Click,
                PointerKind::Drag,
                PointerKind::Move,
            ] {
                assert_eq!(
                    pointer_select(&at(kind, button, 2, 2), 0),
                    None,
                    "{button:?} {kind:?}"
                );
            }
        }
    }

    proptest! {
        /// The grid line is the visible row minus the scroll offset — the same
        /// mapping the canvas applies — and the column passes through, for any
        /// offset a scrollback can reach.
        #[test]
        fn the_grid_line_is_the_row_minus_the_scroll_offset(
            col in 0u16..500,
            row in 0u16..200,
            offset in 0usize..100_000,
            starts in any::<bool>(),
        ) {
            let kind = if starts { PointerKind::Press } else { PointerKind::Drag };
            let expected_line = i32::from(row) - i32::try_from(offset).expect("fits");
            match pointer_select(&at(kind, PointerButton::Left, col, row), offset) {
                Some(SelectOp::Start { line, col: c, .. })
                | Some(SelectOp::Update { line, col: c, .. }) => {
                    prop_assert_eq!(line, expected_line);
                    prop_assert_eq!(c, usize::from(col));
                }
                other => prop_assert!(false, "expected a Start/Update, got {:?}", other),
            }
        }
    }
}
