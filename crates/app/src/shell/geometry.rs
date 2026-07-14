//! Terminal grid geometry (ARCHITECTURE §8): the layout constants, the pure
//! pane-tree → pixel-rect subdivision, and the `Shell` methods that turn the
//! window bounds into a PTY cols/rows grid and size every pane to its own
//! sub-rect. Split from the shell's state machine so the sizing arithmetic —
//! the single source of truth for both the render and the per-leaf PTY
//! geometry — lives in one place.

use iced::{Rectangle, Task};
use termherd_core::ScrollTarget;
use termherd_core::workspace::{Pane, SessionId, SplitDir};

use super::terminal::cell_size;
use super::{Message, Shell};

/// Grid bounds: a pane never shrinks below a usable terminal nor grows past a
/// sane ceiling, whatever the window or split ratios do.
const MIN_COLS: f32 = 20.0;
const MAX_COLS: f32 = 500.0;
const MIN_ROWS: f32 = 5.0;
const MAX_ROWS: f32 = 200.0;

/// The padding and border a split pane's container spends on each side, shared
/// with `view::terminal_leaf` so the two never drift: the view draws with these,
/// the grid subtracts them. Change here, both follow.
pub(super) const PANE_PAD: f32 = 2.0;
pub(super) const PANE_BORDER: f32 = 2.0;
/// Chrome lost on each axis — padding + border on both sides — subtracted from a
/// leaf's rect before deriving its grid so the PTY is sized to the visible
/// canvas, not the slightly larger slot. A lone (unbordered) pane spends none.
const PANE_CHROME: f32 = 2.0 * (PANE_PAD + PANE_BORDER);

/// Sidebar width and the chrome reserved around the terminal, in logical px.
/// Combined with the zoom-derived cell metrics ([`super::terminal::cell_size`])
/// to size the
/// PTY grid to the window (FR4 resize).
const SIDEBAR_W: f32 = 300.0;
/// Width the collapsed sidebar's "▶" handle occupies; the grid reserves this
/// (not `SIDEBAR_W`) when hidden so the reclaimed space becomes columns. The
/// view pins the handle to it, making it a contract, not a drifting estimate.
pub(super) const HANDLE_W: f32 = 28.0;
const H_CHROME: f32 = 40.0;
const V_CHROME: f32 = 84.0;

impl Shell {
    /// Scroll the focused terminal's viewport (a wheel delta or a top/bottom jump).
    pub(super) fn scroll_focused(&mut self, target: ScrollTarget) -> Task<Message> {
        let Some(session) = self.core.workspace.focused_session() else {
            return Task::none();
        };
        self.scroll_session(session, target)
    }

    /// Scroll a specific session's viewport — the wheel targets the pane under
    /// the pointer, not necessarily the focused one.
    pub(super) fn scroll_session(
        &mut self,
        session: SessionId,
        target: ScrollTarget,
    ) -> Task<Message> {
        let effects = self
            .core
            .apply(termherd_core::Event::ScrollViewport { session, target });
        self.perform(effects)
    }

    /// Size every pane's PTY to its own sub-rect from [`pane_rects`] (FR6): one
    /// `TerminalResized` per leaf. A single-pane tab is the one-leaf case,
    /// resized exactly as before.
    pub(super) fn resize_panes(&mut self) -> Task<Message> {
        let (width, height) = self.content_size();
        let area = Rectangle {
            x: 0.0,
            y: 0.0,
            width,
            height,
        };
        // Resolve the rects before the mutable `self.core` loop: `pane_rects`
        // borrows the tree, so its owned result must outlive that borrow. Only a
        // split's panes are bordered, so only they lose `PANE_CHROME`.
        let (rects, inset) = {
            let Some(tab) = self.core.workspace.tabs.get(self.core.workspace.active) else {
                return Task::none();
            };
            let inset = if matches!(tab.root, Pane::Split { .. }) {
                PANE_CHROME
            } else {
                0.0
            };
            (pane_rects(&tab.root, area), inset)
        };
        let font_size = self.core.font_size();
        let tasks: Vec<_> = rects
            .into_iter()
            .map(|(session, rect)| {
                let (cols, rows) = grid_of(rect.width - inset, rect.height - inset, font_size);
                let effects = self.core.apply(termherd_core::Event::TerminalResized {
                    session,
                    cols,
                    rows,
                });
                self.perform(effects)
            })
            .collect();
        Task::batch(tasks)
    }

    /// Toggle the sidebar, then resize: the reclaimed width must re-derive as
    /// columns, not stretch the existing cells.
    pub(super) fn toggle_sidebar(&mut self) -> Task<Message> {
        let effects = self.core.apply(termherd_core::Event::ToggleSidebar);
        Task::batch([self.perform(effects), self.resize_panes()])
    }

    /// Zoom the terminal font, then resize so the grid re-derives cols/rows for
    /// the new cell box.
    pub(super) fn zoom(&mut self, zoom: termherd_core::Zoom) -> Task<Message> {
        let effects = self.core.apply(termherd_core::Event::Zoom(zoom));
        Task::batch([self.perform(effects), self.resize_panes()])
    }

    /// The pixel area the pane region occupies: the window minus the sidebar
    /// (only reserved while visible) and the fixed chrome. Floored at one cell
    /// so the grid math never goes negative on a tiny window.
    fn content_size(&self) -> (f32, f32) {
        let sidebar = if self.core.sidebar.hidden {
            HANDLE_W
        } else {
            SIDEBAR_W
        };
        let (cell_w, cell_h) = cell_size(self.core.font_size());
        let avail_w = (self.bounds.width - sidebar - H_CHROME).max(cell_w);
        let avail_h = (self.bounds.height - V_CHROME).max(cell_h);
        (avail_w, avail_h)
    }
}

/// The terminal grid (cols, rows) that fits a pixel box, clamped to a usable
/// minimum so a pane too small for a real grid still gets a `MIN_COLS`×`MIN_ROWS`
/// terminal rather than a zero-sized one (the split "leaf too small" edge).
pub(super) fn grid_of(width: f32, height: f32, font_size: f32) -> (u16, u16) {
    let (cell_w, cell_h) = cell_size(font_size);
    let cols = (width / cell_w).floor().clamp(MIN_COLS, MAX_COLS) as u16;
    let rows = (height / cell_h).floor().clamp(MIN_ROWS, MAX_ROWS) as u16;
    (cols, rows)
}

/// Subdivide `area` across the tree's leaves at each split's ratio: `Vertical`
/// halves the width (panes side by side), `Horizontal` the height (stacked).
/// Pure, and the single source of truth for both the render and the per-leaf
/// PTY geometry.
pub(super) fn pane_rects(pane: &Pane, area: Rectangle) -> Vec<(SessionId, Rectangle)> {
    let mut out = Vec::new();
    collect_pane_rects(pane, area, &mut out);
    out
}

fn collect_pane_rects(pane: &Pane, area: Rectangle, out: &mut Vec<(SessionId, Rectangle)>) {
    match pane {
        Pane::Leaf(session) => out.push((*session, area)),
        Pane::Split { dir, ratio, a, b } => {
            let (area_a, area_b) = split_area(area, *dir, *ratio);
            collect_pane_rects(a, area_a, out);
            collect_pane_rects(b, area_b, out);
        }
    }
}

/// Cut `area` into the A/B halves for one split node: `Vertical` slices the
/// width (side by side), `Horizontal` slices the height (stacked).
fn split_area(area: Rectangle, dir: SplitDir, ratio: f32) -> (Rectangle, Rectangle) {
    match dir {
        SplitDir::Vertical => {
            let wa = area.width * ratio;
            (
                Rectangle { width: wa, ..area },
                Rectangle {
                    x: area.x + wa,
                    width: area.width - wa,
                    ..area
                },
            )
        }
        SplitDir::Horizontal => {
            let ha = area.height * ratio;
            (
                Rectangle { height: ha, ..area },
                Rectangle {
                    y: area.y + ha,
                    height: area.height - ha,
                    ..area
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU64;
    use termherd_core::workspace::{Pane, SplitDir};

    fn sid(n: u64) -> SessionId {
        SessionId(NonZeroU64::new(n).unwrap_or(NonZeroU64::MIN))
    }

    fn area() -> Rectangle {
        Rectangle {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 600.0,
        }
    }

    fn split(dir: SplitDir, a: Pane, b: Pane) -> Pane {
        Pane::Split {
            dir,
            ratio: 0.5,
            a: Box::new(a),
            b: Box::new(b),
        }
    }

    #[test]
    fn a_lone_leaf_fills_the_whole_area() {
        let rects = pane_rects(&Pane::Leaf(sid(1)), area());
        assert_eq!(rects, vec![(sid(1), area())]);
    }

    #[test]
    fn a_vertical_split_divides_the_width_side_by_side() {
        let tree = split(SplitDir::Vertical, Pane::Leaf(sid(1)), Pane::Leaf(sid(2)));
        let rects = pane_rects(&tree, area());
        assert_eq!(
            rects,
            vec![
                (
                    sid(1),
                    Rectangle {
                        x: 0.0,
                        y: 0.0,
                        width: 500.0,
                        height: 600.0
                    }
                ),
                (
                    sid(2),
                    Rectangle {
                        x: 500.0,
                        y: 0.0,
                        width: 500.0,
                        height: 600.0
                    }
                ),
            ]
        );
    }

    #[test]
    fn a_horizontal_split_divides_the_height_stacked() {
        let tree = split(SplitDir::Horizontal, Pane::Leaf(sid(1)), Pane::Leaf(sid(2)));
        let rects = pane_rects(&tree, area());
        assert_eq!(
            rects,
            vec![
                (
                    sid(1),
                    Rectangle {
                        x: 0.0,
                        y: 0.0,
                        width: 1000.0,
                        height: 300.0
                    }
                ),
                (
                    sid(2),
                    Rectangle {
                        x: 0.0,
                        y: 300.0,
                        width: 1000.0,
                        height: 300.0
                    }
                ),
            ]
        );
    }

    #[test]
    fn a_nested_split_subdivides_recursively() {
        // B side is itself split horizontally: right column stacks 2 over 3.
        let tree = split(
            SplitDir::Vertical,
            Pane::Leaf(sid(1)),
            split(SplitDir::Horizontal, Pane::Leaf(sid(2)), Pane::Leaf(sid(3))),
        );
        let rects = pane_rects(&tree, area());
        assert_eq!(
            rects,
            vec![
                (
                    sid(1),
                    Rectangle {
                        x: 0.0,
                        y: 0.0,
                        width: 500.0,
                        height: 600.0
                    }
                ),
                (
                    sid(2),
                    Rectangle {
                        x: 500.0,
                        y: 0.0,
                        width: 500.0,
                        height: 300.0
                    }
                ),
                (
                    sid(3),
                    Rectangle {
                        x: 500.0,
                        y: 300.0,
                        width: 500.0,
                        height: 300.0
                    }
                ),
            ]
        );
    }

    #[test]
    fn a_tiny_pane_still_gets_a_usable_minimum_grid() {
        // A leaf far too small for a real grid clamps up to the floor rather
        // than collapsing to a zero-sized terminal.
        assert_eq!(grid_of(1.0, 1.0, 14.0), (MIN_COLS as u16, MIN_ROWS as u16));
    }
}
