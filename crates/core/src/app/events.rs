//! The `Event` enum — every input the headless [`App`](super::App) accepts.
//!
//! Kept unified (one enum, not split per domain): `apply(Event) -> Vec<Effect>`
//! is the crate's public contract and its testing model, so the variants live
//! together even though their handlers now fan out across the `app/` submodules.

use std::collections::HashSet;

use crate::browser::SessionRecord;
use crate::metadata::Overlay;
use crate::workspace::{Direction, SessionId, SplitDir};

use super::{LaunchSpec, ScrollTarget, SelectOp, SessionStatus, Zoom};

#[derive(Debug, Clone)]
pub enum Event {
    /// A filesystem scan finished; replaces the whole browser state.
    ScanCompleted(Vec<SessionRecord>),
    /// The search box content changed (FR3).
    SearchChanged(String),
    /// The titles-only search toggle flipped (FR3).
    SearchTitlesOnlyToggled(bool),
    /// The user asked to open a session in a terminal (FR4).
    LaunchSession(LaunchSpec),
    /// The user typed into a terminal; bytes go to its PTY stdin.
    TerminalInput {
        session: SessionId,
        bytes: Vec<u8>,
    },
    /// A terminal pane changed size (in cells); propagate to the PTY (FR4).
    TerminalResized {
        session: SessionId,
        cols: u16,
        rows: u16,
    },
    /// The user changed a terminal's text selection — a press, a drag, or a
    /// clear. Anchored in the terminal grid so the highlight follows the text.
    Select {
        session: SessionId,
        op: SelectOp,
    },
    /// Copy a terminal's current selection to the clipboard. The text is read
    /// from the terminal's own selection (not a snapshot), so it is exact even
    /// right after a fast drag whose highlight has not yet echoed back.
    CopyTerminalSelection {
        session: SessionId,
    },
    /// The user moved a terminal's viewport (FR4 scrollback): a relative wheel
    /// delta, or an absolute jump to the top/bottom of the history.
    ScrollViewport {
        session: SessionId,
        target: ScrollTarget,
    },
    /// The OSC decoder reclassified a session's activity (FR8).
    StatusChanged {
        session: SessionId,
        status: SessionStatus,
    },
    /// A session's PTY process exited. `clean` is true when the adapter saw a
    /// successful completion (exit code 0, no signal); false also covers an
    /// unobservable status.
    PtyExited {
        session: SessionId,
        clean: bool,
    },
    /// The session reported a new title over OSC; relabel its tab.
    SessionTitleChanged {
        session: SessionId,
        title: String,
    },
    /// The user clicked a tab to bring it to the front (FR5).
    ActivateTab(usize),
    /// The user closed a tab (FR5); its sessions' PTYs are killed.
    CloseTab(usize),
    /// The user dragged the tab at `from` to rest at index `to` (FR5). A
    /// pure reorder: no PTY is touched, so it yields no effects.
    MoveTab {
        from: usize,
        to: usize,
    },
    /// Reopen the most recently closed tab, restoring its mode and
    /// directory. A no-op when nothing has been closed.
    ReopenClosedTab,
    /// Give the tab at `index` a manual name, overriding its derived title
    /// (FR5). A blank title clears the override; the manual name is never
    /// clobbered by a later OSC/digest update. A pure relabel — no PTY touched.
    RenameTab {
        index: usize,
        title: String,
    },
    /// Split the focused pane, opening a fresh session beside it (FR6).
    SplitFocused(SplitDir),
    /// Close the focused pane (FR6); its PTY is killed and the split collapses.
    CloseFocusedPane,
    /// Move focus to the next / previous pane in the active tab (FR6).
    FocusNextPane,
    FocusPrevPane,
    /// Move focus to the pane hosting a session (click-to-focus, FR6).
    FocusPane(SessionId),
    /// Move pane focus one step in a spatial direction, cycling within its axis
    /// (FR6).
    FocusDir(Direction),
    /// Persisted metadata loaded at startup (sessions + repos).
    MetadataLoaded(Overlay),
    /// Toggle a session's star, by Claude session id.
    ToggleStar(String),
    /// Toggle a repo's star, by real project path (`F-favorites`, repo-level).
    ToggleRepoStar(String),
    /// Toggle a session's archived flag, by Claude session id.
    ToggleArchive(String),
    /// Set (or clear, when empty) a session's custom title.
    RenameSession {
        session: String,
        title: String,
    },
    /// Show or hide archived sessions in the browser.
    ShowArchivedToggled(bool),
    /// Collapse or restore the session-browser sidebar.
    ToggleSidebar,
    /// Persisted fold state loaded at startup: the folded project paths.
    CollapsedLoaded(HashSet<String>),
    /// Fold or unfold a project's session list in the sidebar, by path.
    ToggleCollapsed(String),
    /// The sidebar session limit from settings: sessions shown per
    /// project before the tail folds behind an expander; `0` shows all.
    SessionLimitLoaded(usize),
    /// Unfold (or refold) a project's truncated session tail, by path.
    ToggleExpanded(String),
    /// The terminal base font size from settings.
    FontSizeLoaded(f32),
    /// Zoom the terminal font in/out/back to base.
    Zoom(Zoom),
    /// The user Ctrl/Cmd+clicked a detected link in a terminal.
    OpenUrl(String),
    /// A session emitted an OSC 9 notification — Claude wants the user.
    /// `body` is the raw payload Claude sent ("needs your attention", a
    /// permission prompt, …). Routed to the OS notification centre on top of
    /// the in-app `Attention` status.
    SessionNotified {
        session: SessionId,
        body: String,
    },
    /// Capture the current state for the AI dev loop (G1). The shell
    /// injects the focused terminal's visible text (the grid lives in the `pty`
    /// adapter, not here); `core` assembles the rest of the dump.
    Capture {
        focused_pty_text: Option<String>,
    },
    /// Start or stop the GIF screencast. Starting carries the frame cap
    /// (`fps × max_seconds`) the app derives from settings; a no-op when the cap
    /// is zero.
    ToggleRecord {
        max_frames: u32,
    },
    /// One frame tick from the app's record timer: capture a frame, and
    /// auto-stop once the cap is reached. A no-op when not recording.
    RecordTick,
    /// The window gained (`true`) or lost (`false`) OS focus. Lets
    /// [`App::notify_session`](super::App) tell a background-tab notification
    /// (surface it) from one on the tab/pane the user is already looking at
    /// (skip the OS banner — the per-window suppression the OS itself applies
    /// when unfocused already covers that case).
    WindowFocusChanged(bool),
}
