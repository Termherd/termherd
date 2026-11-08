//! A pointer event addressed to a terminal by **cell**, and where it goes:
//! to the child when it reads the mouse, to the terminal's own text selection
//! when nothing does.
//!
//! Cell-addressed because a terminal is a grid and a grid is what a mouse
//! report carries — so the same event serves a caller that has no pixels (an
//! agent over MCP), the canvas a human points at, and the encoder that
//! forwards it to the child.
//!
//! The rule is split in two on purpose. *Where the event goes*
//! ([`PointerEvent::route`]) depends on the button, the kind and the mouse
//! reporting the child negotiated — three readers ask it: the terminal thread
//! on its live mode, and the shell and the canvas on the last rendered
//! `Screen`'s. *Where a local gesture lands* ([`pointer_select`]) needs the
//! scroll offset, which only the terminal thread holds live — a snapshot's may
//! lag — so only it is handed that half.

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

/// The mouse reporting a child has switched on — which events it asked the
/// terminal to send it, in the xterm ladder of DECSET 1000 / 1002 / 1003.
/// Each rung includes the ones below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseReporting {
    /// Presses and releases only.
    Click,
    /// Presses, releases and motion with a button held.
    Drag,
    /// Every pointer event, motion with no button included.
    Motion,
}

/// Where a pointer event goes. Under any mouse reporting the mouse belongs
/// to the child: what its mode covers is forwarded and the rest is dropped,
/// so a bare drag never draws a selection over a TUI that is not reading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerRoute {
    /// Encoded and written to the child.
    Forward,
    /// Applied to the terminal's own selection.
    Select,
    /// Dropped.
    Nothing,
}

/// What a pointer event does to the terminal's own selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalGesture {
    /// Begin a selection at the cell.
    Start,
    /// Extend the selection through the cell.
    Extend,
    /// Drop the selection.
    Clear,
}

impl PointerEvent {
    /// An event of `button` at a cell.
    #[must_use]
    pub fn at(kind: PointerKind, button: PointerButton, col: u16, row: u16) -> Self {
        Self {
            kind,
            col,
            row,
            button,
        }
    }

    /// A left-button event at a cell — the common case, and the default a
    /// caller gets when it names no button.
    #[must_use]
    pub fn left(kind: PointerKind, col: u16, row: u16) -> Self {
        Self::at(kind, PointerButton::Left, col, row)
    }

    /// Where this event goes, given what the child asked to be told.
    #[must_use]
    pub fn route(&self, reporting: Option<MouseReporting>) -> PointerRoute {
        let Some(reporting) = reporting else {
            return match self.local_gesture() {
                Some(_) => PointerRoute::Select,
                None => PointerRoute::Nothing,
            };
        };
        let covered = match self.kind {
            PointerKind::Press | PointerKind::Release | PointerKind::Click => true,
            PointerKind::Drag => reporting != MouseReporting::Click,
            PointerKind::Move => reporting == MouseReporting::Motion,
        };
        if covered {
            PointerRoute::Forward
        } else {
            PointerRoute::Nothing
        }
    }

    /// The gesture this event drives on the terminal's own selection, or
    /// `None` when it drives nothing there: the selection stands on a release
    /// (copying it is the caller's `copy`), a hover is nothing to a terminal
    /// not reading the mouse, and only the left button selects.
    fn local_gesture(&self) -> Option<LocalGesture> {
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

    const KINDS: [PointerKind; 5] = [
        PointerKind::Press,
        PointerKind::Release,
        PointerKind::Click,
        PointerKind::Drag,
        PointerKind::Move,
    ];

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
            let event = PointerEvent::left(kind, 1, 1);
            assert_eq!(event.route(None), PointerRoute::Nothing, "{kind:?}");
            assert_eq!(pointer_select(&event, 0), None, "{kind:?}");
        }
    }

    #[test]
    fn only_the_left_button_drives_the_selection() {
        for button in [PointerButton::Middle, PointerButton::Right] {
            for kind in KINDS {
                let event = PointerEvent::at(kind, button, 2, 2);
                assert_eq!(
                    event.route(None),
                    PointerRoute::Nothing,
                    "{button:?} {kind:?}"
                );
                assert_eq!(pointer_select(&event, 0), None, "{button:?} {kind:?}");
            }
        }
    }

    #[test]
    fn without_mouse_reporting_the_route_selects_exactly_when_a_placement_exists() {
        // The two halves of the rule agree: `route` says Select iff
        // `pointer_select` has an op to place.
        for kind in KINDS {
            let event = PointerEvent::left(kind, 1, 1);
            let placed = pointer_select(&event, 0).is_some();
            assert_eq!(
                event.route(None) == PointerRoute::Select,
                placed,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn click_reporting_forwards_the_buttons_and_drops_motion() {
        let mode = Some(MouseReporting::Click);
        for kind in [PointerKind::Press, PointerKind::Release, PointerKind::Click] {
            assert_eq!(
                PointerEvent::left(kind, 1, 1).route(mode),
                PointerRoute::Forward,
                "{kind:?}"
            );
        }
        for kind in [PointerKind::Drag, PointerKind::Move] {
            assert_eq!(
                PointerEvent::left(kind, 1, 1).route(mode),
                PointerRoute::Nothing,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn drag_reporting_forwards_a_drag_but_not_a_bare_move() {
        let mode = Some(MouseReporting::Drag);
        assert_eq!(
            PointerEvent::left(PointerKind::Drag, 1, 1).route(mode),
            PointerRoute::Forward
        );
        assert_eq!(
            PointerEvent::left(PointerKind::Move, 1, 1).route(mode),
            PointerRoute::Nothing
        );
    }

    #[test]
    fn motion_reporting_forwards_everything() {
        for kind in KINDS {
            assert_eq!(
                PointerEvent::left(kind, 1, 1).route(Some(MouseReporting::Motion)),
                PointerRoute::Forward,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn every_button_is_forwarded_alike() {
        // The child gets the button in the report; only the local selection
        // cares that it is the left one.
        for button in [PointerButton::Middle, PointerButton::Right] {
            assert_eq!(
                PointerEvent::at(PointerKind::Press, button, 1, 1)
                    .route(Some(MouseReporting::Click)),
                PointerRoute::Forward,
                "{button:?}"
            );
        }
    }

    proptest! {
        /// Under any mouse reporting the terminal never selects on its own:
        /// the mouse is the child's, whatever the event.
        #[test]
        fn mouse_reporting_never_yields_a_local_selection(
            kind in prop::sample::select(KINDS.to_vec()),
            button in prop::sample::select(vec![
                PointerButton::Left,
                PointerButton::Middle,
                PointerButton::Right,
            ]),
            reporting in prop::sample::select(vec![
                MouseReporting::Click,
                MouseReporting::Drag,
                MouseReporting::Motion,
            ]),
        ) {
            let route = PointerEvent::at(kind, button, 0, 0).route(Some(reporting));
            prop_assert!(
                route != PointerRoute::Select,
                "{:?} {:?} under {:?} selected locally",
                kind, button, reporting
            );
        }

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
