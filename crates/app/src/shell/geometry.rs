//! Terminal grid geometry (ARCHITECTURE §8): the layout constants and the
//! `Shell` methods that turn the window bounds into a PTY cols/rows grid and
//! keep the focused terminal sized to it. Split from the shell's state machine
//! so the sizing arithmetic — the seam a per-pane split layout extends — lives
//! in one place.

use iced::Task;
use termherd_core::ScrollTarget;
use termherd_core::workspace::SessionId;

use super::terminal::cell_size;
use super::{Message, Shell};

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

    /// Tell the focused session's PTY to match the current pane geometry.
    pub(super) fn resize_focused(&mut self) -> Task<Message> {
        let Some(session) = self.core.workspace.focused_session() else {
            return Task::none();
        };
        let (cols, rows) = self.grid_size();
        let effects = self.core.apply(termherd_core::Event::TerminalResized {
            session,
            cols,
            rows,
        });
        self.perform(effects)
    }

    /// Collapse or restore the sidebar, then resize the focused terminal
    /// so the grid re-derives its column count for the new width — without this
    /// the cells just stretch to fill the reclaimed space. Shared by the
    /// button (`Message::ToggleSidebar`) and the keymap (`Action::ToggleSidebar`).
    pub(super) fn toggle_sidebar(&mut self) -> Task<Message> {
        let _ = self.core.apply(termherd_core::Event::ToggleSidebar);
        self.resize_focused()
    }

    /// Zoom the terminal font, then resize the focused terminal so the
    /// grid re-derives its cols/rows for the new cell box — the same pattern
    /// as [`Self::toggle_sidebar`].
    pub(super) fn zoom(&mut self, zoom: termherd_core::Zoom) -> Task<Message> {
        let _ = self.core.apply(termherd_core::Event::Zoom(zoom));
        self.resize_focused()
    }

    /// The terminal grid size (cols, rows) that fits the current window. The
    /// sidebar's width is only reserved while it's visible; collapsing it
    /// hands that space to the grid as extra columns instead of stretching the
    /// existing cells.
    pub(super) fn grid_size(&self) -> (u16, u16) {
        let sidebar = if self.core.sidebar_hidden {
            HANDLE_W
        } else {
            SIDEBAR_W
        };
        let (cell_w, cell_h) = cell_size(self.core.font_size());
        let avail_w = (self.bounds.width - sidebar - H_CHROME).max(cell_w);
        let avail_h = (self.bounds.height - V_CHROME).max(cell_h);
        let cols = (avail_w / cell_w).floor().clamp(20.0, 500.0) as u16;
        let rows = (avail_h / cell_h).floor().clamp(5.0, 200.0) as u16;
        (cols, rows)
    }
}
