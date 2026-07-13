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

/// Chrome a split pane's container spends on each axis: 2px padding + a ~2px
/// border on both sides (`view::terminal_leaf`). Subtracted from a leaf's rect
/// before deriving its grid so the PTY is sized to the visible canvas, not the
/// slightly larger slot. A lone (unbordered) pane spends none.
const PANE_CHROME: f32 = 8.0;

/// Sidebar width and the chrome reserved around the terminal, in logical px.
/// Combined with the zoom-derived cell metrics ([`super::terminal::cell_size`])
/// to size the
/// PTY grid to the window (FR4 resize).
const SIDEBAR_W: f32 = 300.0;
/// Width the collapsed sidebar still occupies: just the slim "▶" handle.
/// The grid reserves this instead of `SIDEBAR_W` when hidden, so the reclaimed
/// space becomes columns rather than stretched cells. The view pins the
/// handle to exactly this width (`view::view`), so it is a contract the layout
/// honours, not an estimate that can silently drift.
pub(super) const HANDLE_W: f32 = 28.0;
const H_CHROME: f32 = 40.0;
const V_CHROME: f32 = 84.0;

impl Shell {
    /// Move the focused terminal's viewport: the mouse wheel sends a
    /// relative delta, the scroll-top/bottom shortcuts an absolute jump. Shared
    /// so both paths go through the one `Event::ScrollViewport`.
    pub(super) fn scroll_focused(&mut self, target: ScrollTarget) -> Task<Message> {
        let Some(session) = self.core.workspace.focused_session() else {
            return Task::none();
        };
        self.scroll_session(session, target)
    }

    /// Move a specific session's viewport. The wheel targets the pane under the
    /// pointer, which need not be the focused one in a split layout.
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

    /// Size every pane's PTY to its own sub-rect of the terminal area (FR6).
    /// The pane region is subdivided by [`pane_rects`] from the `core` tree, so
    /// a plain single-pane tab resizes exactly as before (one leaf spanning the
    /// whole area) while a split sizes each leaf independently — one
    /// `TerminalResized` per leaf.
    pub(super) fn resize_panes(&mut self) -> Task<Message> {
        let (width, height) = self.content_size();
        let area = Rectangle {
            x: 0.0,
            y: 0.0,
            width,
            height,
        };
        // Resolve the rects before touching `self.core` mutably: `pane_rects`
        // borrows the tree, so the returned owned Vec must outlive that borrow.
        // A split gives each pane a bordered container the terminal sits inside;
        // a lone pane has none, so only the former loses `PANE_CHROME`.
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

    /// Collapse or restore the sidebar, then resize the panes so the grid
    /// re-derives its column count for the new width — without this the cells
    /// just stretch to fill the reclaimed space. Shared by the button
    /// (`Message::ToggleSidebar`) and the keymap (`Action::ToggleSidebar`).
    pub(super) fn toggle_sidebar(&mut self) -> Task<Message> {
        let _ = self.core.apply(termherd_core::Event::ToggleSidebar);
        self.resize_panes()
    }

    /// Zoom the terminal font, then resize the panes so the grid re-derives its
    /// cols/rows for the new cell box — the same pattern as
    /// [`Self::toggle_sidebar`].
    pub(super) fn zoom(&mut self, zoom: termherd_core::Zoom) -> Task<Message> {
        let _ = self.core.apply(termherd_core::Event::Zoom(zoom));
        self.resize_panes()
    }

    /// The pixel area the pane region occupies: the window minus the sidebar
    /// (only reserved while visible) and the fixed chrome. Floored at one cell
    /// so the grid math never goes negative on a tiny window.
    fn content_size(&self) -> (f32, f32) {
        let sidebar = if self.core.sidebar_hidden {
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

/// Subdivide `area` across every leaf of the pane tree, left-to-right /
/// top-to-bottom, at each split's ratio. A `SplitDir::Vertical` (vertical
/// divider) halves the width so the panes sit side by side; a
/// `SplitDir::Horizontal` halves the height so they stack. Pure: the single
/// source of truth for both the recursive render and the per-leaf PTY geometry,
/// derived from the `core` tree each frame so no rival layout can drift from it.
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
