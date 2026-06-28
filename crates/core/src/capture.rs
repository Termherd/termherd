//! Capture — a pure snapshot of the workspace for the AI dev loop (#108, G1).
//!
//! Rung 0 of the `F-capture` fidelity ladder: a deterministic, diffable model
//! of the current state — tabs, focus, per-tab activity, pane membership — plus
//! the focused terminal's visible text (injected by the shell, since the grid
//! lives in the `pty` adapter, not here). The shell encodes this to JSON and
//! writes it next to the rung-1 PNG. **Pure**: no I/O, no clock, no panic.

use crate::app::SessionStatus;

/// A snapshot of the whole workspace at capture time. The shell serialises it
/// to `capture-<ts>.json`; rung 1 (the PNG) captures the pixels this can't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureDump {
    /// Index of the active tab, or `None` when no tab is open.
    pub active_tab: Option<usize>,
    /// Every open tab, in tab order.
    pub tabs: Vec<CaptureTab>,
    /// The focused terminal's visible grid as text, injected by the shell.
    /// `None` when nothing is focused or its screen has not rendered yet.
    pub focused_pty: Option<String>,
}

/// One tab in a [`CaptureDump`]: its label, derived activity, the sessions it
/// hosts (pane membership, left-to-right), and — for the active tab only — which
/// of them holds focus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureTab {
    /// Whether this is the active tab.
    pub active: bool,
    /// The tab label the user sees.
    pub title: String,
    /// The most urgent activity among the tab's sessions, or `None` if none of
    /// them are still live.
    pub status: Option<SessionStatus>,
    /// Runtime ids of the sessions this tab hosts, in pane order. One id for a
    /// plain tab; several for a split.
    pub sessions: Vec<u64>,
    /// The focused leaf's session id — only set on the active tab, `None`
    /// elsewhere (an inactive tab has no live focus to report).
    pub focus_session: Option<u64>,
}
