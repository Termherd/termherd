//! A pointer event addressed to a terminal by **cell**, and what it does to the
//! terminal's own text selection when the child is not reading the mouse.
//!
//! Cell-addressed because a terminal is a grid and a grid is what a mouse
//! report carries — so the same event serves a caller that has no pixels (an
//! agent over MCP) and, later, the encoder that forwards it to the child.
//!
//! The rule is split in two on purpose. *Whether* an event drives the local
//! selection ([`PointerEvent::local_gesture`]) depends on the button and the
//! kind alone, so the shell can answer a caller from the event itself. *Where*
//! it lands ([`pointer_select`]) needs the scroll offset, which only the
//! terminal thread holds live — a snapshot's may lag — so only it is handed
//! that half.

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

/// One pointer event at a visible cell of a session's terminal. `col`/`row`
/// are 0-based in the **visible** screen, not the scrollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerEvent {
    pub kind: PointerKind,
    pub col: u16,
    pub row: u16,
    pub button: PointerButton,
}

/// What a pointer event does to the terminal's own selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalGesture {
    /// Begin a selection at the cell.
    Start,
    /// Extend the selection through the cell.
    Extend,
    /// Drop the selection.
    Clear,
}

impl PointerEvent {
    /// A left-button event at a cell — the common case, and the default a
    /// caller gets when it names no button.
    #[must_use]
    pub fn left(kind: PointerKind, col: u16, row: u16) -> Self {
        Self {
            kind,
            col,
            row,
            button: PointerButton::Left,
        }
    }

    /// The gesture this event drives on the terminal's own selection, or
    /// `None` when it drives nothing there: the selection stands on a release
    /// (copying it is the caller's `copy`), a hover is nothing to a terminal
    /// not reading the mouse, and only the left button selects.
    #[must_use]
    pub fn local_gesture(&self) -> Option<LocalGesture> {
        if self.button != PointerButton::Left {
            return None;
        }
        match self.kind {
            PointerKind::Press => Some(LocalGesture::Start),
            PointerKind::Drag => Some(LocalGesture::Extend),
            PointerKind::Click => Some(LocalGesture::Clear),
            PointerKind::Release | PointerKind::Move => None,
        }
    }
}

/// The grid line a visible row names, given how far the viewport is scrolled
/// up into history — the coordinate the emulator anchors a selection to, so
/// the highlight follows the text through scroll.
#[must_use]
pub fn grid_line(row: u16, display_offset: usize) -> i32 {
    i32::from(row) - display_offset as i32
}

/// The selection change a pointer event drives, placed on the grid with the
/// given scroll offset, or `None` when it drives nothing locally.
#[must_use]
pub fn pointer_select(pointer: &PointerEvent, display_offset: usize) -> Option<SelectOp> {
    let line = grid_line(pointer.row, display_offset);
    let col = usize::from(pointer.col);
    // A cell has no halves for a caller addressing it by index, so the sides
    // are chosen to make a forward drag **inclusive** of both cells: the
    // terminal reads a `Left` endpoint as "before this cell" and a `Right`
    // one as "after it".
    pointer.local_gesture().map(|gesture| match gesture {
        LocalGesture::Start => SelectOp::Start {
            line,
            col,
            side: SelectSide::Left,
        },
        LocalGesture::Extend => SelectOp::Update {
            line,
            col,
            side: SelectSide::Right,
        },
        LocalGesture::Clear => SelectOp::Clear,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn at(kind: PointerKind, button: PointerButton, col: u16, row: u16) -> PointerEvent {
        PointerEvent {
            button,
            ..PointerEvent::left(kind, col, row)
        }
    }

    #[test]
    fn a_left_press_starts_a_selection_at_the_cell() {
        assert_eq!(
            pointer_select(&PointerEvent::left(PointerKind::Press, 4, 2), 0),
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
            pointer_select(&PointerEvent::left(PointerKind::Drag, 9, 3), 0),
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
            pointer_select(&PointerEvent::left(PointerKind::Click, 1, 1), 0),
            Some(SelectOp::Clear)
        );
    }

    #[test]
    fn a_release_and_a_move_drive_nothing() {
        for kind in [PointerKind::Release, PointerKind::Move] {
            assert_eq!(
                PointerEvent::left(kind, 1, 1).local_gesture(),
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
                    at(kind, button, 2, 2).local_gesture(),
                    None,
                    "{button:?} {kind:?}"
                );
            }
        }
    }

    proptest! {
        /// The grid line is the visible row minus the scroll offset and the
        /// column passes through, for any offset a scrollback can reach.
        #[test]
        fn the_grid_line_is_the_row_minus_the_scroll_offset(
            col in 0u16..500,
            row in 0u16..200,
            offset in 0usize..100_000,
            starts in any::<bool>(),
        ) {
            let kind = if starts { PointerKind::Press } else { PointerKind::Drag };
            let expected_line = i32::from(row) - i32::try_from(offset).expect("fits");
            match pointer_select(&PointerEvent::left(kind, col, row), offset) {
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
