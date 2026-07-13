//! Headless `App` — pure state machine over `Event`/`Effect`.
//!
//! The quality keystone (see `docs/ARCHITECTURE.md` §5). Events and effects
//! grow incrementally with each milestone. M2 adds the terminal lifecycle:
//! launching a session emits a [`Effect::Spawn`]; the runtime (the iced shell
//! plus the `pty` adapter) performs it and feeds bytes/status/exit back as
//! events. The grid itself lives in the adapter's per-session task — `core`
//! holds only the lifecycle and the derived activity status (FR8).

use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;

use crate::browser::{
    MatchSnippet, ProjectGroup, SessionRecord, content_snippet, filter_projects, group_projects,
    project_label,
};
use crate::capture::{CaptureDump, CaptureTab};
use crate::metadata::{Overlay, RepoMeta, SessionMeta};
use crate::record::Recording;
use crate::workspace::{Direction, SessionId, SplitDir, Workspace};

/// Cell size a freshly launched PTY starts at, before the widget reports its
/// real geometry via [`Event::TerminalResized`].
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// Shown as the desktop notification body when Claude fires a bare OSC 9 with
/// no text of its own.
const DEFAULT_NOTIFICATION_BODY: &str = "Claude needs your attention";

/// Notification title fallback when a session somehow has no hosting tab;
/// a broken invariant in practice, never the normal path.
const APP_NAME: &str = "TermHerd";

#[derive(Debug, Default)]
pub struct App {
    pub workspace: Workspace,
    /// Sidebar state: projects grouped from the latest scan (FR1).
    pub projects: Vec<ProjectGroup>,
    /// Current search query (FR3); empty means no filtering.
    pub search: String,
    /// FR3 toggle: restrict matching to titles.
    pub search_titles_only: bool,
    /// Live terminal sessions, keyed by their runtime id (FR4/FR7).
    pub sessions: HashMap<SessionId, LiveSession>,
    /// User overlay (star / archive / title) per Claude session id
    /// (`F-session-metadata`); persisted to `~/.termherd`.
    pub metadata: HashMap<String, SessionMeta>,
    /// User overlay (star) per real project path (`F-favorites`, repo-level);
    /// shares `~/.termherd/metadata.json` with [`Self::metadata`].
    pub repos: HashMap<String, RepoMeta>,
    /// Whether archived sessions show in the browser.
    pub show_archived: bool,
    /// Whether the session-browser sidebar is collapsed to give the terminal
    /// the full width. Ephemeral — resets to visible each launch.
    pub sidebar_hidden: bool,
    /// Project paths whose session list is folded shut in the sidebar;
    /// persisted to `~/.termherd` so the fold survives a restart.
    pub collapsed: HashSet<String>,
    /// Sidebar truncation: sessions shown per project before the tail
    /// folds behind an expander. `0` (the default) shows every session; the
    /// user's setting arrives via [`Event::SessionLimitLoaded`].
    pub session_limit: usize,
    /// Projects whose truncated session tail is unfolded. Ephemeral —
    /// unlike `collapsed`, it resets each launch and is never persisted.
    pub expanded: HashSet<String>,
    /// The configured terminal base font size, from settings via
    /// [`Event::FontSizeLoaded`]; `None` until loaded (the built-in
    /// [`DEFAULT_FONT_SIZE`] then applies).
    font_base: Option<f32>,
    /// Zoom steps on top of the base font: ±1 px each, clamped at event
    /// time so surplus presses at a bound don't accumulate as drift.
    /// Ephemeral — resets each launch.
    zoom_steps: i32,
    /// Monotonic source of `SessionId`s; never reused within a run. This is
    /// the structural fix for the `realSessionId` race (Q6) — ids are minted
    /// here, single-threaded, before any PTY exists.
    next_session: u64,
    /// LIFO stack of recently closed tabs, for reopen. Capped at
    /// [`MAX_CLOSED_TABS`] so a long session can't grow it without bound;
    /// closing past the cap drops the oldest entry.
    closed_tabs: Vec<ClosedTab>,
    /// The in-progress GIF screencast, or `None` when not recording. The
    /// frame timer and encoder live in the `app`; `core` only counts frames
    /// against the cap and decides capture/finish.
    recording: Option<Recording>,
    /// Whether the OS says the window has focus, from
    /// [`Event::WindowFocusChanged`]. Starts `false` (unknown) so a session
    /// notification is forwarded to the OS until a real focus signal proves
    /// the user is already looking at it — matching the earlier behaviour.
    window_focused: bool,
}

/// What the expander row under a project's truncated session list should show
/// from [`App::sidebar_sessions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarFold {
    /// The tail is folded: this many more sessions are hidden.
    Truncated(usize),
    /// The tail is unfolded and can be folded back.
    Expanded,
}

/// A zoom request, carried by [`Event::Zoom`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    /// Grow the terminal font one step.
    In,
    /// Shrink the terminal font one step.
    Out,
    /// Back to the configured base size.
    Reset,
}

/// The terminal font size before settings load or when none is configured.
/// Mirrors the historical `FONT_SIZE` constant.
pub const DEFAULT_FONT_SIZE: f32 = 14.0;
/// Bounds for the effective font size — small enough to overview a
/// large scrollback, large enough for a presentation, and both far from
/// degenerate cell geometry.
const FONT_SIZE_RANGE: (f32, f32) = (6.0, 40.0);

/// How many closed tabs the reopen stack remembers. Walking back further
/// than this is rare enough that the unbounded-growth risk outweighs it.
const MAX_CLOSED_TABS: usize = 16;

/// Enough of a closed tab to recreate it on reopen: the kind it ran, the
/// directory it ran in, and the label it carried. A split tab is reduced to its
/// first pane — reopen restores a single terminal, not the whole pane tree.
#[derive(Debug, Clone)]
pub struct ClosedTab {
    pub title: String,
    /// The manual name overlaid on the derived title when the tab was closed, if
    /// any — restored on reopen so a rename round-trips, not just the digest.
    pub custom_title: Option<String>,
    pub cwd: Option<String>,
    pub launch: Launch,
}

/// A terminal session the app is hosting. The PTY handle and terminal grid
/// live in the adapter's task, not here; this is just the lifecycle record.
#[derive(Debug, Clone)]
pub struct LiveSession {
    pub id: SessionId,
    /// Real project path the PTY runs in, if known.
    pub cwd: Option<String>,
    /// What this terminal is running — a shell or a (possibly resumed) Claude
    /// session. The resumed-id lets the sidebar map a browsed session row to its
    /// live activity (FR8); read it via [`Launch::resume_id`].
    pub launch: Launch,
    /// Activity derived from the OSC stream (FR8).
    pub status: SessionStatus,
}

impl LiveSession {
    /// Whether this session still holds a **running foreground process** whose
    /// loss is worth confirming before a close. A Claude session *is* that
    /// process — the `claude` CLI runs in the shell's foreground until it
    /// exits, so any non-exited Claude counts, an idle prompt included. A plain
    /// shell only counts while it is actively working (`Busy`) or flagged for
    /// the user (`Attention`); parked at its prompt (`Idle`/`Starting`) there is
    /// nothing to lose, so it can be closed silently.
    #[must_use]
    pub fn has_running_process(&self) -> bool {
        match self.status {
            SessionStatus::Exited => false,
            _ => match self.launch {
                Launch::Claude { .. } => true,
                Launch::Shell => {
                    matches!(self.status, SessionStatus::Busy | SessionStatus::Attention)
                }
            },
        }
    }
}

/// Per-session activity surfaced in the sidebar and on tabs (FR8). Derived
/// from the terminal OSC stream by `termherd_claude::osc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    /// Spawned; no activity classified yet.
    Starting,
    /// Claude is working (OSC busy / spinner).
    Busy,
    /// Idle, or waiting at a prompt for input.
    Idle,
    /// Blocked needing the user: a permission prompt or an explicit "needs
    /// your attention" notification (OSC 9). Outranks `Idle` — the user must
    /// act — and is cleared only when work resumes (`Busy`).
    Attention,
    /// The PTY process has exited.
    Exited,
}

impl SessionStatus {
    /// Urgency rank for collapsing several sessions into one indicator — the
    /// sidebar dedupe of duplicate live rows and the per-tab badge (FR8). The
    /// status that most wants the user's eyes wins: `Attention` over `Busy`
    /// over `Idle` over `Starting` over `Exited`.
    #[must_use]
    pub fn urgency(self) -> u8 {
        match self {
            SessionStatus::Attention => 4,
            SessionStatus::Busy => 3,
            SessionStatus::Idle => 2,
            SessionStatus::Starting => 1,
            SessionStatus::Exited => 0,
        }
    }
}

/// What to run in a launched terminal (FR4a). The core decides the *kind*; the
/// `pty` adapter decides *how* to start it. `Shell` is a bare login shell;
/// `Claude` starts the CLI — fresh when `resume` is `None`, else
/// `claude --resume <id>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// A plain login shell in the working directory.
    Shell,
    /// A Claude session: fresh (`resume: None`) or resumed (`resume: Some(id)`).
    Claude { resume: Option<String> },
}

impl Launch {
    /// The Claude session id this launch resumes, if any — `None` for a shell
    /// or a fresh Claude session. Lets the sidebar map a `claude_id` back to the
    /// live tab hosting it.
    #[must_use]
    pub fn resume_id(&self) -> Option<&str> {
        match self {
            Launch::Claude { resume: Some(id) } => Some(id),
            _ => None,
        }
    }
}

/// What the user asked to open (FR4): a terminal in `cwd`, running some
/// [`Launch`] kind.
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    /// Working directory for the new terminal (the real project path).
    pub cwd: Option<String>,
    /// What to run in the terminal.
    pub launch: Launch,
    /// Tab title to show.
    pub title: String,
}

/// A spawn request handed to the `pty` adapter. The runtime id is already
/// allocated, so the adapter never invents one.
#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub session: SessionId,
    pub cwd: Option<String>,
    pub launch: Launch,
    pub cols: u16,
    pub rows: u16,
}

/// Where to move a terminal's viewport. One scroll concept covers the
/// mouse wheel's relative nudge and the absolute top/bottom jumps, so the event,
/// effect and `PtyHost::scroll` port all speak it instead of special-casing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollTarget {
    /// The oldest line in the scrollback.
    Top,
    /// The live bottom of the buffer.
    Bottom,
    /// A mouse-wheel turn over a pointer cell (`col`/`row`, 0-based) of `lines`
    /// (positive = up). Carrying the pointer lets a full-screen app with mouse
    /// reporting be handed the wheel as input; the adapter falls back to a
    /// relative scrollback nudge when it isn't one.
    Wheel { col: u16, row: u16, lines: i32 },
}

/// Which edge of a cell a selection endpoint sits on, so a press on a cell's
/// right half extends past it — the terminal's own notion of a selection side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectSide {
    Left,
    Right,
}

/// A change to a session's text selection, applied to the terminal's own
/// grid-anchored selection so the highlight rides the text through scroll. The
/// `line` is an absolute grid line (viewport row minus the scroll offset) and
/// `col` a 0-based column — the coordinate the emulator rotates on every scroll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectOp {
    /// Begin (or restart) a selection at a grid point.
    Start {
        line: i32,
        col: usize,
        side: SelectSide,
    },
    /// Extend the in-progress selection to a grid point.
    Update {
        line: i32,
        col: usize,
        side: SelectSide,
    },
    /// Select a whole range at once, both endpoints given — a double-click word,
    /// whose boundaries the caller resolves (filenames/paths included).
    Range {
        line0: i32,
        col0: usize,
        line1: i32,
        col1: usize,
    },
    /// Drop the selection — a bare click, or an explicit clear.
    Clear,
}

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
    /// A session's PTY process exited.
    PtyExited(SessionId),
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
    /// [`App::notify_session`] tell a background-tab notification (surface
    /// it) from one on the tab/pane the user is already looking at (skip the
    /// OS banner — the per-window suppression the OS itself applies when
    /// unfocused already covers that case).
    WindowFocusChanged(bool),
}

/// Side effects the runtime must perform. The iced shell turns these into
/// `pty`-adapter calls (`docs/ARCHITECTURE.md` §8).
#[derive(Debug, Clone)]
pub enum Effect {
    /// Spawn a PTY for a freshly launched session.
    Spawn(SpawnSpec),
    /// Write bytes to a session's PTY stdin.
    Write { session: SessionId, bytes: Vec<u8> },
    /// Resize a session's PTY to the given cell geometry.
    Resize {
        session: SessionId,
        cols: u16,
        rows: u16,
    },
    /// Move a session's viewport: a relative line delta or an absolute jump to
    /// the top/bottom of the scrollback.
    Scroll {
        session: SessionId,
        target: ScrollTarget,
    },
    /// Apply a selection change to a session's terminal grid.
    Select { session: SessionId, op: SelectOp },
    /// Ask a session's terminal to copy its current selection — the text comes
    /// back out-of-band (a PTY event), so it is read from the live selection.
    CopyTerminalSelection { session: SessionId },
    /// Terminate a session's PTY process.
    Kill(SessionId),
    /// Persist the whole metadata overlay (sessions + repos) as one file.
    SaveMetadata(Overlay),
    /// Persist the folded-project set.
    SaveCollapsed(HashSet<String>),
    /// Open a URL in the OS default handler; the shell performs it.
    OpenUrl(String),
    /// Post a desktop notification to the OS notification centre. The
    /// shell performs it; `title` names the session/project that wants the
    /// user, `body` is Claude's message.
    Notify { title: String, body: String },
    /// Write a captured state snapshot for the AI dev loop (G1). The shell
    /// encodes it to `capture-<ts>.json` and takes the companion PNG; `core`
    /// only builds the pure, diffable payload.
    Capture(CaptureDump),
    /// Begin a GIF screencast: the app opens the encoder and starts its
    /// frame timer. `core` has already entered the recording state.
    StartRecording,
    /// Capture one screencast frame: the app screenshots the window and
    /// appends it to the open encoder.
    CaptureFrame,
    /// Finalise the GIF screencast: the app flushes the encoder and writes
    /// `capture-<ts>.gif`. `capped` names *why* it stopped — `true` when the frame
    /// cap auto-stopped it, `false` on a manual ⌘⇧R stop — so the app logs the
    /// reason from `core`'s decision rather than re-deriving it from the effect mix.
    FinishRecording { capped: bool },
    /// Abandon a screencast that captured no frames: the app drops the
    /// encoder without writing a file. Emitted when a recording is stopped before
    /// the first frame.
    CancelRecording,
}

impl App {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply an event, returning the effects the runtime must carry out.
    /// **Pure**: no I/O, no clock, no panic.
    #[allow(clippy::too_many_lines)] // flat Event dispatcher; one match arm per variant
    pub fn apply(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::ScanCompleted(records) => {
                self.projects = group_projects(records);
                Vec::new()
            }
            Event::SearchChanged(query) => {
                self.search = query;
                Vec::new()
            }
            Event::SearchTitlesOnlyToggled(titles_only) => {
                self.search_titles_only = titles_only;
                Vec::new()
            }
            Event::LaunchSession(spec) => self.launch(spec),
            Event::TerminalInput { session, bytes } => {
                if self.is_live(session) {
                    vec![Effect::Write { session, bytes }]
                } else {
                    Vec::new()
                }
            }
            Event::TerminalResized {
                session,
                cols,
                rows,
            } => {
                if self.is_live(session) {
                    vec![Effect::Resize {
                        session,
                        cols,
                        rows,
                    }]
                } else {
                    Vec::new()
                }
            }
            Event::ScrollViewport { session, target } => {
                if self.is_live(session) {
                    vec![Effect::Scroll { session, target }]
                } else {
                    Vec::new()
                }
            }
            Event::Select { session, op } => {
                if self.is_live(session) {
                    vec![Effect::Select { session, op }]
                } else {
                    Vec::new()
                }
            }
            Event::CopyTerminalSelection { session } => {
                if self.is_live(session) {
                    vec![Effect::CopyTerminalSelection { session }]
                } else {
                    Vec::new()
                }
            }
            Event::StatusChanged { session, status } => {
                if let Some(s) = self.sessions.get_mut(&session)
                    && s.status != SessionStatus::Exited
                {
                    s.status = status;
                }
                Vec::new()
            }
            Event::PtyExited(session) => {
                if let Some(s) = self.sessions.get_mut(&session) {
                    s.status = SessionStatus::Exited;
                }
                Vec::new()
            }
            Event::SessionTitleChanged { session, title } => {
                self.workspace.set_session_title(session, title);
                Vec::new()
            }
            Event::ActivateTab(index) => {
                self.workspace.activate(index);
                Vec::new()
            }
            Event::CloseTab(index) => self.close_tab(index),
            Event::MoveTab { from, to } => {
                self.workspace.move_tab(from, to);
                Vec::new()
            }
            Event::ReopenClosedTab => self.reopen_closed_tab(),
            Event::RenameTab { index, title } => {
                self.workspace.rename_tab(index, &title);
                Vec::new()
            }
            Event::SplitFocused(dir) => self.split_focused(dir),
            Event::CloseFocusedPane => match self.workspace.close_focused() {
                Some(id) => {
                    self.sessions.remove(&id);
                    vec![Effect::Kill(id)]
                }
                None => Vec::new(),
            },
            Event::FocusNextPane => {
                self.workspace.focus_next();
                Vec::new()
            }
            Event::FocusPrevPane => {
                self.workspace.focus_prev();
                Vec::new()
            }
            Event::FocusPane(session) => {
                self.workspace.focus_pane_of(session);
                Vec::new()
            }
            Event::FocusDir(dir) => {
                self.workspace.focus_dir(dir);
                Vec::new()
            }
            Event::MetadataLoaded(overlay) => {
                self.metadata = overlay.sessions;
                self.repos = overlay.repos;
                Vec::new()
            }
            Event::ToggleStar(session) => {
                self.update_meta(session, |meta| meta.starred = !meta.starred)
            }
            Event::ToggleRepoStar(path) => {
                self.update_repo_meta(path, |meta| meta.starred = !meta.starred)
            }
            Event::ToggleArchive(session) => {
                self.update_meta(session, |meta| meta.archived = !meta.archived)
            }
            Event::RenameSession { session, title } => {
                let trimmed = title.trim().to_owned();
                let effects = self.update_meta(session.clone(), |meta| {
                    meta.title = (!trimmed.is_empty()).then(|| trimmed.clone());
                });
                // Keep a live tab in step with the sidebar (follow-up): an
                // open session resuming this id is retitled too. A non-empty
                // rename wins directly; clearing restores the digest-derived name
                // when the session is still in the last scan.
                if let Some(live) = self.open_session_for(&session) {
                    let next = if trimmed.is_empty() {
                        self.record_for(&session)
                            .map(|record| self.session_title(record))
                            .filter(|name| !name.trim().is_empty())
                    } else {
                        Some(trimmed)
                    };
                    if let Some(next) = next {
                        self.workspace.set_session_title(live, next);
                    }
                }
                effects
            }
            Event::ShowArchivedToggled(show) => {
                self.show_archived = show;
                Vec::new()
            }
            Event::ToggleSidebar => {
                self.sidebar_hidden = !self.sidebar_hidden;
                Vec::new()
            }
            Event::CollapsedLoaded(paths) => {
                self.collapsed = paths;
                Vec::new()
            }
            Event::ToggleCollapsed(path) => self.toggle_collapsed(path),
            Event::SessionLimitLoaded(limit) => self.load_session_limit(limit),
            Event::ToggleExpanded(path) => self.toggle_expanded(path),
            Event::FontSizeLoaded(size) => self.load_font_size(size),
            Event::Zoom(zoom) => self.zoom(zoom),
            Event::OpenUrl(url) => {
                let url = url.trim();
                // Only well-formed schemes reach the OS handler; a blank or
                // schemeless string is dropped rather than shelling out on it.
                if url.is_empty() {
                    Vec::new()
                } else {
                    vec![Effect::OpenUrl(url.to_owned())]
                }
            }
            Event::SessionNotified { session, body } => self.notify_session(session, body),
            Event::Capture { focused_pty_text } => {
                vec![Effect::Capture(self.build_capture(focused_pty_text))]
            }
            Event::ToggleRecord { max_frames } => self.toggle_record(max_frames),
            Event::RecordTick => self.record_tick(),
            Event::WindowFocusChanged(focused) => {
                self.window_focused = focused;
                Vec::new()
            }
        }
    }

    /// Start or stop the GIF screencast. Starting from idle enters the
    /// recording state and asks the app to open its encoder; a zero `max_frames`
    /// is a no-op (nothing to record). Stopping finalises the GIF when frames
    /// were captured, or cancels it outright when none were (the zero-frame
    /// guard — a start immediately followed by a stop writes no file).
    fn toggle_record(&mut self, max_frames: u32) -> Vec<Effect> {
        match self.recording.take() {
            None => {
                if max_frames == 0 {
                    return Vec::new();
                }
                self.recording = Some(Recording {
                    frames: 0,
                    max_frames,
                });
                vec![Effect::StartRecording]
            }
            Some(recording) if recording.frames > 0 => {
                vec![Effect::FinishRecording { capped: false }]
            }
            Some(_) => vec![Effect::CancelRecording],
        }
    }

    /// One frame of the screencast: count it and ask the app to capture
    /// it, then auto-stop once the cap is reached. A tick while not recording is
    /// a silent no-op (a stray timer beat after a stop).
    fn record_tick(&mut self) -> Vec<Effect> {
        let Some(recording) = self.recording.as_mut() else {
            return Vec::new();
        };
        recording.frames += 1;
        let mut effects = vec![Effect::CaptureFrame];
        if recording.frames >= recording.max_frames {
            self.recording = None;
            effects.push(Effect::FinishRecording { capped: true });
        }
        effects
    }

    /// Whether a GIF screencast is in progress — the app gates its frame
    /// timer subscription on this.
    #[must_use]
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Screencast progress as `(frames captured, frame cap)` while recording, or
    /// `None` when idle. The shell renders it as the `● REC n/cap`
    /// indicator so the recording state — and how close it is to auto-stop — is
    /// visible at a glance.
    #[must_use]
    pub fn recording_progress(&self) -> Option<(u32, u32)> {
        self.recording.map(|r| (r.frames, r.max_frames))
    }

    /// Assemble the capture snapshot for the AI dev loop. Pure: it reads
    /// the workspace and live-session state and folds in the focused terminal's
    /// text the shell supplied (the grid lives in the `pty` adapter). The result
    /// is the diffable rung-0 payload; the shell adds the rung-1 PNG.
    #[must_use]
    pub fn build_capture(&self, focused_pty_text: Option<String>) -> CaptureDump {
        let active_tab = (!self.workspace.tabs.is_empty()).then_some(self.workspace.active);
        let focused = self.workspace.focused_session();
        let tabs = self
            .workspace
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let active = active_tab == Some(index);
                CaptureTab {
                    active,
                    title: tab.display_title().to_owned(),
                    status: self.tab_status(index),
                    sessions: tab.sessions().into_iter().map(|s| s.0.get()).collect(),
                    // Only the active tab has a live focus to report.
                    focus_session: focused.filter(|_| active).map(|s| s.0.get()),
                }
            })
            .collect();
        CaptureDump {
            active_tab,
            tabs,
            focused_pty: focused_pty_text,
        }
    }

    /// The sidebar's view of the projects: search matches (FR3) with the
    /// metadata overlay applied (`F-session-metadata`) — archived sessions
    /// hidden unless [`Self::show_archived`], starred sessions pinned to the
    /// top of their group, starred repos pinned to the top of the sidebar
    /// (`F-favorites`), and emptied groups dropped.
    #[must_use]
    pub fn visible_projects(&self) -> Vec<ProjectGroup> {
        let mut groups = filter_projects(&self.projects, &self.search, self.search_titles_only);
        for group in &mut groups {
            if !self.show_archived {
                group.sessions.retain(|s| !self.is_archived(&s.session_id));
            }
            // Stable sort keeps recency order within each star bucket.
            group
                .sessions
                .sort_by_key(|s| !self.is_starred(&s.session_id));
        }
        groups.retain(|group| !group.sessions.is_empty());
        // Stable sort keeps activity order within each repo-star bucket.
        groups.sort_by_key(|group| !self.is_repo_starred(&group.path));
        groups
    }

    /// Starred sessions across all `groups`, most-recent-first — the source for
    /// the cross-project "★ Favorites" section (`F-favorites`). Each carries its
    /// project path so a row can resume it. Derived from `groups` (already
    /// search- and archive-filtered by [`Self::visible_projects`]) so favorites
    /// stay consistent with the list; missing mtimes sort last.
    #[must_use]
    pub fn favorite_sessions<'a>(
        &self,
        groups: &'a [ProjectGroup],
    ) -> Vec<(&'a str, &'a SessionRecord)> {
        let mut favourites: Vec<(&str, &SessionRecord)> = groups
            .iter()
            .flat_map(|group| {
                group
                    .sessions
                    .iter()
                    .map(move |session| (group.path.as_str(), session))
            })
            .filter(|(_, session)| self.is_starred(&session.session_id))
            .collect();
        favourites.sort_by_key(|(_, session)| std::cmp::Reverse(session.modified));
        favourites
    }

    /// The sessions a project row should list: all of them while a
    /// search is active (a hit in the folded tail must surface), when the
    /// limit is unset, or when the group already fits; otherwise the first
    /// `session_limit` (starred pins sort first in [`Self::visible_projects`],
    /// so they stay visible) plus the expander state for the folded tail.
    #[must_use]
    pub fn sidebar_sessions<'a>(
        &self,
        group: &'a ProjectGroup,
    ) -> (&'a [SessionRecord], Option<SidebarFold>) {
        let all = group.sessions.as_slice();
        let searching = !self.search.trim().is_empty();
        if searching || self.session_limit == 0 || all.len() <= self.session_limit {
            return (all, None);
        }
        if self.expanded.contains(&group.path) {
            return (all, Some(SidebarFold::Expanded));
        }
        let hidden = all.len() - self.session_limit;
        (
            &all[..self.session_limit],
            Some(SidebarFold::Truncated(hidden)),
        )
    }

    /// The located content hit for a session under the current search,
    /// or `None` when the row is shown for a title hit (or titles-only mode):
    /// nothing in the content matched, so there is nothing to point at.
    #[must_use]
    pub fn search_snippet(&self, record: &SessionRecord) -> Option<MatchSnippet> {
        if self.search_titles_only {
            return None;
        }
        let needle = self.search.trim().to_lowercase();
        content_snippet(&record.digest, &needle)
    }

    /// The title to show for a session: the user's custom title if set, else
    /// the one derived from the digest (`F-session-metadata`).
    #[must_use]
    pub fn session_title(&self, record: &SessionRecord) -> String {
        self.metadata
            .get(&record.session_id)
            .and_then(|meta| meta.title.clone())
            .unwrap_or_else(|| record.digest.display_title(None).to_owned())
    }

    /// Session ids in `group` whose resolved [`Self::session_title`] is shared
    /// by another session in the same group — the rows that need a
    /// disambiguator in the sidebar. Collision is checked on the *final*
    /// title (rename/metadata included), so two rows renamed alike still count.
    /// The common, unique case returns an empty set, so callers leave it clean.
    #[must_use]
    pub fn colliding_titles(&self, group: &ProjectGroup) -> HashSet<String> {
        let titled: Vec<(&str, String)> = group
            .sessions
            .iter()
            .map(|s| (s.session_id.as_str(), self.session_title(s)))
            .collect();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for (_, title) in &titled {
            *counts.entry(title.as_str()).or_default() += 1;
        }
        titled
            .iter()
            .filter(|(_, title)| counts.get(title.as_str()).copied().unwrap_or(0) > 1)
            .map(|(id, _)| (*id).to_owned())
            .collect()
    }

    /// The content disambiguator for a row whose title collides with another
    /// in its group: the session's real first-prompt `summary` when it
    /// *diverges* from the shown title. A custom/AI title or rename can mask a
    /// completely different conversation — Claude Code carries a custom title
    /// across `/clear` into a fresh, unrelated session, so two rows read
    /// identically while their summaries differ. Surfacing the summary tells
    /// them apart by content, where the last-activity age only tells them apart
    /// by time. `None` when the title *is* the summary (no masking), so the
    /// caller falls back to the age disambiguator.
    #[must_use]
    pub fn collision_subtitle(&self, record: &SessionRecord) -> Option<String> {
        let title = self.session_title(record);
        let summary = record.digest.summary.as_str();
        (!summary.is_empty() && summary != title).then(|| summary.to_owned())
    }

    /// Whether a session (by Claude id) is starred / archived.
    #[must_use]
    pub fn is_starred(&self, session_id: &str) -> bool {
        self.metadata.get(session_id).is_some_and(|m| m.starred)
    }

    #[must_use]
    pub fn is_archived(&self, session_id: &str) -> bool {
        self.metadata.get(session_id).is_some_and(|m| m.archived)
    }

    /// Whether a project (by real path) is starred (`F-favorites`, repo-level).
    #[must_use]
    pub fn is_repo_starred(&self, path: &str) -> bool {
        self.repos.get(path).is_some_and(|m| m.starred)
    }

    /// The live session currently resuming the Claude session `claude_id`, if
    /// one is open. Lets the shell re-focus an existing terminal when its
    /// sidebar row is clicked again, rather than spawning a duplicate (FR4).
    #[must_use]
    pub fn open_session_for(&self, claude_id: &str) -> Option<SessionId> {
        self.sessions
            .values()
            .find(|s| s.launch.resume_id() == Some(claude_id))
            .map(|s| s.id)
    }

    /// The browsed record for the Claude session `claude_id`, if the last scan
    /// found it. The inverse of [`Self::open_session_for`]: it maps a live tab
    /// back to the sidebar entry it resumes, so the tab hover can reuse the same
    /// session card the sidebar shows instead of a second derive. `None`
    /// for a shell or a fresh, not-yet-scanned session.
    #[must_use]
    pub fn record_for(&self, claude_id: &str) -> Option<&SessionRecord> {
        self.projects
            .iter()
            .flat_map(|group| &group.sessions)
            .find(|record| record.session_id == claude_id)
    }

    /// Whether a session id is still on the scanned project list — the guard
    /// the archive confirmation uses against a session a rescan removed while
    /// the prompt was up. Exactly "the last scan has a record for it", so it
    /// tracks [`Self::record_for`].
    #[must_use]
    pub fn is_browsable(&self, session: &str) -> bool {
        self.record_for(session).is_some()
    }

    /// The tab title for a new session (FR4): the scanned digest name for a
    /// resumed Claude session — current Claude renders status in-band and emits
    /// no OSC title, so without this every resumed tab in a repo would read
    /// alike — else the kind label `{project} {glyph}`. A fresh or unscanned
    /// session keeps the kind label; an OSC title still wins later. The kind
    /// glyphs are the caller's (view-side constants), so core carries no
    /// presentation literals.
    #[must_use]
    pub fn tab_title(
        &self,
        cwd: &str,
        launch: &Launch,
        shell_glyph: &str,
        claude_glyph: &str,
    ) -> String {
        let label = project_label(cwd);
        match launch {
            Launch::Shell => format!("{label} {shell_glyph}"),
            Launch::Claude {
                resume: Some(claude_id),
            } => self
                .record_for(claude_id)
                .map(|record| self.session_title(record))
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| format!("{label} {claude_glyph}")),
            Launch::Claude { resume: None } => format!("{label} {claude_glyph}"),
        }
    }

    /// The browsed record for the tab at `index` — the sidebar entry its first
    /// pane resumes, so a tab hover can show the same session card. `None`
    /// for an out-of-range index, or a tab whose first pane is a shell or a
    /// fresh, not-yet-scanned session (no resume id / no record).
    #[must_use]
    pub fn tab_record(&self, index: usize) -> Option<&SessionRecord> {
        let tab = self.workspace.tabs.get(index)?;
        let first = tab.sessions().first().copied()?;
        let claude_id = self.sessions.get(&first)?.launch.resume_id()?;
        self.record_for(claude_id)
    }

    /// Whether a project's session list is folded shut in the sidebar.
    #[must_use]
    pub fn is_collapsed(&self, path: &str) -> bool {
        self.collapsed.contains(path)
    }

    /// Flip a project's fold state and emit the persistence effect.
    fn toggle_collapsed(&mut self, path: String) -> Vec<Effect> {
        if !self.collapsed.remove(&path) {
            self.collapsed.insert(path);
        }
        vec![Effect::SaveCollapsed(self.collapsed.clone())]
    }

    /// Unfold (or refold) a project's truncated session tail. Unlike
    /// [`Self::toggle_collapsed`], the state is ephemeral — no save effect.
    fn toggle_expanded(&mut self, path: String) -> Vec<Effect> {
        if !self.expanded.remove(&path) {
            self.expanded.insert(path);
        }
        Vec::new()
    }

    /// Record the configured sidebar session limit, from settings.
    fn load_session_limit(&mut self, limit: usize) -> Vec<Effect> {
        self.session_limit = limit;
        Vec::new()
    }

    /// Record the configured terminal base font size, from settings.
    fn load_font_size(&mut self, size: f32) -> Vec<Effect> {
        self.font_base = Some(size);
        Vec::new()
    }

    /// The effective terminal font size: the configured base (or the
    /// built-in default before settings load) plus the zoom steps, clamped
    /// into [`FONT_SIZE_RANGE`].
    #[must_use]
    pub fn font_size(&self) -> f32 {
        let base = self.font_base.unwrap_or(DEFAULT_FONT_SIZE);
        let (min, max) = FONT_SIZE_RANGE;
        (base + self.zoom_steps as f32).clamp(min, max)
    }

    /// Apply a zoom step. Steps are refused at the bounds rather than
    /// clamped at read, so surplus presses never accumulate as drift — one
    /// zoom-out after many zoom-ins at the cap shrinks immediately.
    fn zoom(&mut self, zoom: Zoom) -> Vec<Effect> {
        let (min, max) = FONT_SIZE_RANGE;
        match zoom {
            Zoom::In if self.font_size() < max => self.zoom_steps += 1,
            Zoom::Out if self.font_size() > min => self.zoom_steps -= 1,
            Zoom::Reset => self.zoom_steps = 0,
            Zoom::In | Zoom::Out => {}
        }
        Vec::new()
    }

    /// The full overlay to persist — both keyings, cloned as one unit so a save
    /// never drops the other map.
    fn overlay(&self) -> Overlay {
        Overlay {
            sessions: self.metadata.clone(),
            repos: self.repos.clone(),
        }
    }

    /// Edit a session's metadata, dropping it when it returns to defaults, and
    /// emit the persistence effect.
    fn update_meta(&mut self, session: String, edit: impl FnOnce(&mut SessionMeta)) -> Vec<Effect> {
        let mut meta = self.metadata.get(&session).cloned().unwrap_or_default();
        edit(&mut meta);
        if meta.is_default() {
            self.metadata.remove(&session);
        } else {
            self.metadata.insert(session, meta);
        }
        vec![Effect::SaveMetadata(self.overlay())]
    }

    /// Edit a repo's metadata, dropping it when it returns to defaults, and
    /// emit the persistence effect. Mirrors [`Self::update_meta`].
    fn update_repo_meta(&mut self, path: String, edit: impl FnOnce(&mut RepoMeta)) -> Vec<Effect> {
        let mut meta = self.repos.get(&path).cloned().unwrap_or_default();
        edit(&mut meta);
        if meta.is_default() {
            self.repos.remove(&path);
        } else {
            self.repos.insert(path, meta);
        }
        vec![Effect::SaveMetadata(self.overlay())]
    }

    /// Register a launched session, open it as a tab, and ask the runtime to
    /// spawn its PTY. Returns no effects if id allocation overflows (after
    /// ~1.8e19 launches) — surfaced as a silent no-op, never a panic (Q5).
    fn launch(&mut self, spec: LaunchSpec) -> Vec<Effect> {
        let Some(id) = self.allocate_session() else {
            return Vec::new();
        };
        self.sessions.insert(
            id,
            LiveSession {
                id,
                cwd: spec.cwd.clone(),
                launch: spec.launch.clone(),
                status: SessionStatus::Starting,
            },
        );
        self.workspace.open(id, spec.title);
        vec![Effect::Spawn(SpawnSpec {
            session: id,
            cwd: spec.cwd,
            launch: spec.launch,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
        })]
    }

    /// Split the focused pane (FR6): mint a session, inherit the focused pane's
    /// working directory, wrap the leaf into a split, and spawn the new PTY.
    /// Yields no effects on id overflow or if the focus is not on a leaf.
    fn split_focused(&mut self, dir: SplitDir) -> Vec<Effect> {
        let Some(id) = self.allocate_session() else {
            return Vec::new();
        };
        // Inherit the cwd before the split moves focus to the new pane.
        let cwd = self
            .workspace
            .focused_session()
            .and_then(|focused| self.sessions.get(&focused))
            .and_then(|session| session.cwd.clone());
        if self.workspace.split(dir, id).is_none() {
            return Vec::new();
        }
        self.sessions.insert(
            id,
            LiveSession {
                id,
                cwd: cwd.clone(),
                launch: Launch::Shell,
                status: SessionStatus::Starting,
            },
        );
        vec![Effect::Spawn(SpawnSpec {
            session: id,
            cwd,
            launch: Launch::Shell,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
        })]
    }

    /// Close a tab (FR5): drop its sessions from the live registry and ask the
    /// runtime to kill each PTY. An out-of-range index yields no effects.
    /// Snapshots the tab onto the reopen stack first, so the close can be
    /// undone before its sessions are forgotten.
    fn close_tab(&mut self, index: usize) -> Vec<Effect> {
        self.remember_closed_tab(index);
        let sessions = self.workspace.close_tab(index);
        for id in &sessions {
            self.sessions.remove(id);
        }
        sessions.into_iter().map(Effect::Kill).collect()
    }

    /// Push the tab at `index` onto the reopen stack, capturing the kind,
    /// directory and label needed to recreate it. Reduced to the tab's first
    /// pane — reopen restores one terminal, not a whole split. A no-op for an
    /// out-of-range index or a tab whose first session is no longer live.
    fn remember_closed_tab(&mut self, index: usize) {
        let Some(tab) = self.workspace.tabs.get(index) else {
            return;
        };
        let title = tab.title.clone();
        let custom_title = tab.custom_title.clone();
        let Some(first) = tab.sessions().first().copied() else {
            return;
        };
        let Some(session) = self.sessions.get(&first) else {
            return;
        };
        self.closed_tabs.push(ClosedTab {
            title,
            custom_title,
            cwd: session.cwd.clone(),
            launch: session.launch.clone(),
        });
        // Keep only the most recent entries; drop the oldest past the cap.
        if self.closed_tabs.len() > MAX_CLOSED_TABS {
            self.closed_tabs.remove(0);
        }
    }

    /// Reopen the most recently closed tab, relaunching it in the mode and
    /// directory it was closed in. Re-closing then reopening walks the stack in
    /// LIFO order. No effects when the stack is empty.
    fn reopen_closed_tab(&mut self) -> Vec<Effect> {
        let Some(closed) = self.closed_tabs.pop() else {
            return Vec::new();
        };
        let custom_title = closed.custom_title;
        let effects = self.launch(LaunchSpec {
            cwd: closed.cwd,
            launch: closed.launch,
            title: closed.title,
        });
        // Restore the manual name on top of the derived title. `launch` opens
        // the reopened tab as the new active one, so its index is `active` — but
        // only when the launch actually opened a tab (empty effects = id
        // overflow, no tab), or we would rename an unrelated tab.
        if !effects.is_empty()
            && let Some(name) = custom_title
        {
            self.workspace.rename_tab(self.workspace.active, &name);
        }
        effects
    }

    /// The activity status to badge on the tab at `index` (FR8): the most
    /// urgent status among the sessions it hosts, or `None` for an unknown
    /// index or a tab whose sessions are no longer live.
    #[must_use]
    pub fn tab_status(&self, index: usize) -> Option<SessionStatus> {
        let tab = self.workspace.tabs.get(index)?;
        tab.sessions()
            .into_iter()
            .filter_map(|id| self.sessions.get(&id).map(|s| s.status))
            .max_by_key(|status| status.urgency())
    }

    /// Whether closing the tab at `index` would kill a running foreground
    /// process, so the GUI must confirm the close first. `false` for a tab
    /// sitting idle (close it silently) and for an unknown index. This single
    /// running-state check is meant to back both the close-tab confirmation and
    /// the quit confirmation, so neither has to re-derive "is a process
    /// running?" for itself.
    #[must_use]
    pub fn tab_has_running_process(&self, index: usize) -> bool {
        self.workspace.tabs.get(index).is_some_and(|tab| {
            tab.sessions().iter().any(|id| {
                self.sessions
                    .get(id)
                    .is_some_and(LiveSession::has_running_process)
            })
        })
    }

    /// Whether any session anywhere still runs a foreground process, so a quit
    /// must confirm before hard-killing them all. The app-wide counterpart to
    /// [`Self::tab_has_running_process`] over the same
    /// [`LiveSession::has_running_process`] predicate, so a close and a quit
    /// never disagree on "is a process running?".
    #[must_use]
    pub fn any_running_process(&self) -> bool {
        self.sessions.values().any(LiveSession::has_running_process)
    }

    /// Count of sessions whose PTY is still running — the ones a quit would
    /// hard-kill. Exited sessions linger in the registry but cost nothing to
    /// drop; the count behind the quit-confirmation modal's summary line.
    #[must_use]
    pub fn live_session_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|s| s.status != SessionStatus::Exited)
            .count()
    }

    /// Decide whether an OSC 9 notification reaches the OS notification
    /// centre, and with what title/body. Only live sessions are worth alerting
    /// on — an unknown or exited session has nothing to return to, so it is
    /// dropped. The title is the session's tab label (what the user sees, and
    /// tracks OSC-24 renames); a blank body falls back to a default message.
    ///
    /// Also dropped: a session that is both the active tab's focused
    /// pane *and* the window has OS focus — the user is already looking at
    /// it, so no banner is needed. Any other live session still gets one,
    /// including a background tab while the window is focused: the OS's own
    /// per-window banner suppression only covers the focused-tab case, and a
    /// background tab needs the effect to reach the OS (or an in-app cue) to
    /// be seen at all.
    fn notify_session(&self, session: SessionId, body: String) -> Vec<Effect> {
        if !self.is_live(session) {
            return Vec::new();
        }
        if self.window_focused && self.workspace.focused_session() == Some(session) {
            return Vec::new();
        }
        // A live session is always hosted by a tab, so `session_title` returns
        // `Some`; the app-name fallback only guards a broken invariant.
        let title = self
            .workspace
            .session_title(session)
            .unwrap_or(APP_NAME)
            .to_owned();
        let body = if body.trim().is_empty() {
            DEFAULT_NOTIFICATION_BODY.to_owned()
        } else {
            body
        };
        vec![Effect::Notify { title, body }]
    }

    /// Mint the next runtime session id. `None` only on u64 overflow.
    fn allocate_session(&mut self) -> Option<SessionId> {
        self.next_session = self.next_session.checked_add(1)?;
        NonZeroU64::new(self.next_session).map(SessionId)
    }

    /// True if the session exists and its PTY has not exited.
    fn is_live(&self, session: SessionId) -> bool {
        self.sessions
            .get(&session)
            .is_some_and(|s| s.status != SessionStatus::Exited)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use termherd_claude::digest::SessionDigest;

    fn record(id: &str, path: &str, summary: &str) -> SessionRecord {
        SessionRecord {
            session_id: id.into(),
            project_path: path.into(),
            digest: SessionDigest {
                summary: summary.into(),
                message_count: 1,
                text_content: String::new(),
                slug: None,
                custom_title: None,
                ai_title: None,
                tail: Vec::new(),
            },
            modified: None,
        }
    }

    #[test]
    fn status_urgency_ranks_attention_highest_and_exited_lowest() {
        use SessionStatus::*;
        let mut ordered = [Exited, Starting, Idle, Busy, Attention];
        ordered.sort_by_key(|s| s.urgency());
        assert_eq!(ordered, [Exited, Starting, Idle, Busy, Attention]);
        assert!(Attention.urgency() > Busy.urgency());
        assert!(Busy.urgency() > Idle.urgency());
        assert!(Idle.urgency() > Starting.urgency());
        assert!(Starting.urgency() > Exited.urgency());
    }

    #[test]
    fn scan_completed_rebuilds_projects_and_yields_no_effects() {
        let mut app = App::new();
        let effects = app.apply(Event::ScanCompleted(vec![record("abc", "/p", "hello")]));
        assert!(effects.is_empty());
        assert_eq!(app.projects.len(), 1);
        assert_eq!(app.projects[0].path, "/p");

        // A later scan replaces, not appends.
        let effects = app.apply(Event::ScanCompleted(vec![]));
        assert!(effects.is_empty());
        assert!(app.projects.is_empty());
    }

    #[test]
    fn search_events_drive_visible_projects() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record(
            "abc",
            "/p",
            "fix the login bug",
        )]));
        assert_eq!(app.visible_projects().len(), 1);

        app.apply(Event::SearchChanged("login".into()));
        assert_eq!(app.visible_projects().len(), 1);

        app.apply(Event::SearchChanged("nothing-here".into()));
        assert!(app.visible_projects().is_empty());

        app.apply(Event::SearchChanged(String::new()));
        assert_eq!(app.visible_projects().len(), 1);
    }

    /// `count` sessions in `/p`, freshest first, applied with a scan.
    fn scanned_group(app: &mut App, count: usize) {
        let records = (0..count)
            .map(|i| {
                let mut r = record(&format!("s{i}"), "/p", "routine work");
                r.modified = Some(
                    std::time::SystemTime::UNIX_EPOCH
                        + std::time::Duration::from_secs(1000 - i as u64),
                );
                r
            })
            .collect();
        app.apply(Event::ScanCompleted(records));
    }

    #[test]
    fn sidebar_truncates_to_the_limit_and_folds_the_tail() {
        let mut app = App::new();
        scanned_group(&mut app, 8);
        app.apply(Event::SessionLimitLoaded(5));
        let groups = app.visible_projects();
        let (shown, fold) = app.sidebar_sessions(&groups[0]);
        assert_eq!(shown.len(), 5);
        assert_eq!(fold, Some(SidebarFold::Truncated(3)));
        // The five kept are the freshest.
        assert!(shown.iter().all(|s| s.session_id != "s7"));
    }

    #[test]
    fn no_limit_or_a_fitting_group_shows_every_session() {
        let mut app = App::new();
        scanned_group(&mut app, 8);
        // Default (0): truncation is off.
        let groups = app.visible_projects();
        assert_eq!(
            app.sidebar_sessions(&groups[0]),
            (&groups[0].sessions[..], None)
        );
        // A limit the group fits within changes nothing either.
        app.apply(Event::SessionLimitLoaded(8));
        assert_eq!(
            app.sidebar_sessions(&groups[0]),
            (&groups[0].sessions[..], None)
        );
    }

    #[test]
    fn toggle_expanded_unfolds_the_tail_and_refolds_without_persisting() {
        let mut app = App::new();
        scanned_group(&mut app, 8);
        app.apply(Event::SessionLimitLoaded(5));
        let effects = app.apply(Event::ToggleExpanded("/p".into()));
        assert!(effects.is_empty(), "expanded state is ephemeral");
        let groups = app.visible_projects();
        let (shown, fold) = app.sidebar_sessions(&groups[0]);
        assert_eq!(shown.len(), 8);
        assert_eq!(fold, Some(SidebarFold::Expanded));
        // Toggling again folds the tail back.
        app.apply(Event::ToggleExpanded("/p".into()));
        let (shown, fold) = app.sidebar_sessions(&groups[0]);
        assert_eq!(shown.len(), 5);
        assert_eq!(fold, Some(SidebarFold::Truncated(3)));
    }

    #[test]
    fn search_surfaces_hits_from_the_folded_tail() {
        let mut app = App::new();
        let mut records: Vec<SessionRecord> = (0..7u64)
            .map(|i| {
                let mut r = record(&format!("s{i}"), "/p", "routine work");
                r.modified = Some(
                    std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + i),
                );
                r
            })
            .collect();
        // No mtime → sorts last: the needle lives in the folded tail.
        records.push(record("needle", "/p", "the rare needle"));
        app.apply(Event::ScanCompleted(records));
        app.apply(Event::SessionLimitLoaded(5));

        let groups = app.visible_projects();
        let (shown, _) = app.sidebar_sessions(&groups[0]);
        assert!(shown.iter().all(|s| s.session_id != "needle"));

        // An active query disables truncation, so the tail hit surfaces.
        app.apply(Event::SearchChanged("rare needle".into()));
        let groups = app.visible_projects();
        let (shown, fold) = app.sidebar_sessions(&groups[0]);
        assert_eq!(fold, None);
        assert!(shown.iter().any(|s| s.session_id == "needle"));
    }

    #[test]
    fn zoom_steps_the_font_from_the_loaded_base_and_resets() {
        let mut app = App::new();
        // Before settings load, the built-in default applies.
        assert!((app.font_size() - DEFAULT_FONT_SIZE).abs() < f32::EPSILON);

        app.apply(Event::FontSizeLoaded(16.0));
        assert!((app.font_size() - 16.0).abs() < f32::EPSILON);

        app.apply(Event::Zoom(Zoom::In));
        app.apply(Event::Zoom(Zoom::In));
        assert!((app.font_size() - 18.0).abs() < f32::EPSILON);

        app.apply(Event::Zoom(Zoom::Out));
        assert!((app.font_size() - 17.0).abs() < f32::EPSILON);

        let effects = app.apply(Event::Zoom(Zoom::Reset));
        assert!(effects.is_empty());
        assert!((app.font_size() - 16.0).abs() < f32::EPSILON);
    }

    #[test]
    fn zoom_refuses_steps_at_the_bounds_without_accumulating_drift() {
        let mut app = App::new();
        app.apply(Event::FontSizeLoaded(38.0));
        // Two steps reach the 40.0 cap; ten more must be refused, not banked.
        for _ in 0..12 {
            app.apply(Event::Zoom(Zoom::In));
        }
        assert!((app.font_size() - 40.0).abs() < f32::EPSILON);
        // One zoom-out shrinks immediately — no surplus presses to unwind.
        app.apply(Event::Zoom(Zoom::Out));
        assert!((app.font_size() - 39.0).abs() < f32::EPSILON);

        // Same at the floor.
        app.apply(Event::FontSizeLoaded(7.0));
        app.apply(Event::Zoom(Zoom::Reset));
        for _ in 0..12 {
            app.apply(Event::Zoom(Zoom::Out));
        }
        assert!((app.font_size() - 6.0).abs() < f32::EPSILON);
        app.apply(Event::Zoom(Zoom::In));
        assert!((app.font_size() - 7.0).abs() < f32::EPSILON);
    }

    #[test]
    fn launch_registers_session_opens_tab_and_spawns() {
        let mut app = App::new();
        let effects = app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Shell,
            title: "proj".into(),
        }));

        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.workspace.tabs.len(), 1);
        let id = app.workspace.focused_session().expect("a focused session");
        assert_eq!(app.sessions[&id].status, SessionStatus::Starting);
        assert_eq!(app.sessions[&id].cwd.as_deref(), Some("/proj"));

        match effects.as_slice() {
            [Effect::Spawn(spec)] => {
                assert_eq!(spec.session, id);
                assert_eq!(spec.cwd.as_deref(), Some("/proj"));
                assert_eq!((spec.cols, spec.rows), (DEFAULT_COLS, DEFAULT_ROWS));
            }
            other => panic!("expected one Spawn, got {other:?}"),
        }
    }

    #[test]
    fn select_on_a_live_session_forwards_the_op_to_its_terminal() {
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Shell,
            title: "proj".into(),
        }));
        let id = app.workspace.focused_session().expect("a focused session");
        let op = SelectOp::Start {
            line: 2,
            col: 4,
            side: SelectSide::Left,
        };
        match app.apply(Event::Select { session: id, op }).as_slice() {
            [
                Effect::Select {
                    session,
                    op: forwarded,
                },
            ] => {
                assert_eq!(*session, id);
                assert_eq!(*forwarded, op);
            }
            other => panic!("expected one Select effect, got {other:?}"),
        }
    }

    #[test]
    fn launching_a_resume_records_its_claude_id() {
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Claude {
                resume: Some("abc-123".into()),
            },
            title: "proj".into(),
        }));
        let id = app.workspace.focused_session().expect("a focused session");
        assert_eq!(app.sessions[&id].launch.resume_id(), Some("abc-123"));
    }

    #[test]
    fn open_session_for_finds_a_live_resume_and_ignores_unknowns() {
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Claude {
                resume: Some("abc-123".into()),
            },
            title: "proj".into(),
        }));
        let id = app.workspace.focused_session().expect("a focused session");
        assert_eq!(app.open_session_for("abc-123"), Some(id));
        assert_eq!(app.open_session_for("not-open"), None);
    }

    #[test]
    fn record_for_maps_a_claude_id_back_to_its_browsed_record() {
        // A live tab's resume id resolves to the sidebar record, so the
        // tab hover can reuse the same session card.
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![
            record("abc-123", "/proj", "fix the login bug"),
            record("def-456", "/other", "write the docs"),
        ]));
        assert_eq!(
            app.record_for("def-456").map(|r| r.project_path.as_str()),
            Some("/other")
        );
        assert_eq!(
            app.record_for("abc-123").map(|r| r.digest.summary.as_str()),
            Some("fix the login bug")
        );
        // A shell / fresh session id has no browsed record.
        assert!(app.record_for("not-scanned").is_none());
    }

    #[test]
    fn is_browsable_tracks_the_scanned_list() {
        // The archive-confirm guard: a session is browsable iff the last scan
        // still lists it. A rescan that drops it must un-browse it.
        let mut app = App::new();
        assert!(!app.is_browsable("abc"), "empty app browses nothing");

        app.apply(Event::ScanCompleted(vec![record("abc", "/p", "hi")]));
        assert!(app.is_browsable("abc"), "a scanned session is browsable");
        assert!(!app.is_browsable("gone"), "an unscanned id is not");

        // A rescan without it drops it from the browsable set.
        app.apply(Event::ScanCompleted(vec![]));
        assert!(
            !app.is_browsable("abc"),
            "a session a rescan removed is no longer browsable"
        );
    }

    #[test]
    fn tab_title_prefers_the_scanned_digest_name() {
        // Glyphs are the caller's (view-side constants), passed in; core owns
        // the digest-name-else-kind-label policy.
        let mut app = App::new();
        // A shell gets the project label with the shell glyph.
        assert_eq!(
            app.tab_title("/home/me/proj", &Launch::Shell, "$", "🤖"),
            "proj $"
        );

        // A fresh Claude session (no resume) gets the Claude glyph.
        assert_eq!(
            app.tab_title("/home/me/proj", &Launch::Claude { resume: None }, "$", "🤖"),
            "proj 🤖"
        );

        // Resuming a *scanned* session takes its digest name (no glyph), so two
        // resumed tabs in one repo don't read alike.
        app.apply(Event::ScanCompleted(vec![record(
            "abc-123",
            "/home/me/proj",
            "fix the login bug",
        )]));
        assert_eq!(
            app.tab_title(
                "/home/me/proj",
                &Launch::Claude {
                    resume: Some("abc-123".into())
                },
                "$",
                "🤖",
            ),
            "fix the login bug"
        );

        // Resuming an *unscanned* session falls back to the kind label.
        assert_eq!(
            app.tab_title(
                "/home/me/proj",
                &Launch::Claude {
                    resume: Some("not-scanned".into())
                },
                "$",
                "🤖",
            ),
            "proj 🤖"
        );
    }

    #[test]
    fn tab_record_resolves_a_resumed_tab_and_skips_shells_and_unknowns() {
        // A tab resuming a scanned session maps back to its record; a shell
        // tab (no resume id) and an out-of-range index map to nothing.
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record(
            "abc-123",
            "/proj",
            "fix the login bug",
        )]));
        // Tab 0: a resumed Claude session that the scan knows.
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Claude {
                resume: Some("abc-123".into()),
            },
            title: "proj 🤖".into(),
        }));
        // Tab 1: a plain shell — no resume id, so no record.
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Shell,
            title: "proj $".into(),
        }));
        assert_eq!(
            app.tab_record(0).map(|r| r.session_id.as_str()),
            Some("abc-123")
        );
        assert!(app.tab_record(1).is_none(), "a shell tab has no record");
        assert!(app.tab_record(9).is_none(), "an out-of-range index is None");
    }

    #[test]
    fn each_launch_gets_a_distinct_id() {
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: None,
            launch: Launch::Shell,
            title: "a".into(),
        }));
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: None,
            launch: Launch::Shell,
            title: "b".into(),
        }));
        assert_eq!(app.sessions.len(), 2);
    }

    #[test]
    fn input_and_resize_target_only_live_sessions() {
        let mut app = App::new();
        let spawn = app.apply(Event::LaunchSession(LaunchSpec {
            cwd: None,
            launch: Launch::Shell,
            title: "a".into(),
        }));
        let id = match spawn.as_slice() {
            [Effect::Spawn(spec)] => spec.session,
            other => panic!("expected Spawn, got {other:?}"),
        };

        let write = app.apply(Event::TerminalInput {
            session: id,
            bytes: b"ls\n".to_vec(),
        });
        assert!(
            matches!(write.as_slice(), [Effect::Write { session, bytes }]
            if *session == id && bytes == b"ls\n")
        );

        let resize = app.apply(Event::TerminalResized {
            session: id,
            cols: 120,
            rows: 40,
        });
        assert!(matches!(
            resize.as_slice(),
            [Effect::Resize { session, cols: 120, rows: 40 }] if *session == id
        ));

        // After exit, no further effects are produced for that session.
        app.apply(Event::PtyExited(id));
        assert_eq!(app.sessions[&id].status, SessionStatus::Exited);
        assert!(
            app.apply(Event::TerminalInput {
                session: id,
                bytes: b"x".to_vec(),
            })
            .is_empty()
        );
    }

    fn launch(app: &mut App, title: &str) -> SessionId {
        match app
            .apply(Event::LaunchSession(LaunchSpec {
                cwd: None,
                launch: Launch::Shell,
                title: title.into(),
            }))
            .as_slice()
        {
            [Effect::Spawn(spec)] => spec.session,
            other => panic!("expected Spawn, got {other:?}"),
        }
    }

    #[test]
    fn activate_tab_brings_an_earlier_session_to_focus() {
        let mut app = App::new();
        let first = launch(&mut app, "a");
        let _second = launch(&mut app, "b");
        assert_eq!(app.workspace.focused_session(), Some(_second));

        let effects = app.apply(Event::ActivateTab(0));
        assert!(effects.is_empty());
        assert_eq!(app.workspace.focused_session(), Some(first));
    }

    #[test]
    fn activate_tab_out_of_range_leaves_the_active_tab_untouched() {
        // Regression guard for the number-row jump: pressing ⌘5
        // with only two tabs open resolves to an out-of-range index, which
        // must be a silent no-op rather than a panic or a focus change.
        let mut app = App::new();
        let _first = launch(&mut app, "a");
        let second = launch(&mut app, "b");
        assert_eq!(app.workspace.active, 1);

        let effects = app.apply(Event::ActivateTab(4));
        assert!(effects.is_empty());
        assert_eq!(app.workspace.active, 1);
        assert_eq!(app.workspace.focused_session(), Some(second));
    }

    #[test]
    fn close_tab_kills_its_session_and_drops_it_from_the_registry() {
        let mut app = App::new();
        let first = launch(&mut app, "a");
        let second = launch(&mut app, "b");

        let effects = app.apply(Event::CloseTab(1));
        assert!(matches!(effects.as_slice(), [Effect::Kill(id)] if *id == second));
        assert_eq!(app.workspace.tabs.len(), 1);
        assert!(!app.sessions.contains_key(&second));
        // The surviving session stays live and focused.
        assert_eq!(app.workspace.focused_session(), Some(first));
        assert!(app.sessions.contains_key(&first));
    }

    #[test]
    fn reopen_restores_a_closed_tab_in_its_mode_and_directory() {
        // Closing a Claude tab then reopening relaunches the same kind in
        // the same directory, with its label.
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/repo".into()),
            launch: Launch::Claude {
                resume: Some("abc".into()),
            },
            title: "repo 🤖".into(),
        }));
        let original = app.workspace.focused_session().expect("focused");
        app.apply(Event::CloseTab(0));
        assert!(app.workspace.tabs.is_empty());

        let effects = app.apply(Event::ReopenClosedTab);
        let spec = match effects.as_slice() {
            [Effect::Spawn(spec)] => spec,
            other => panic!("expected one Spawn, got {other:?}"),
        };
        assert_ne!(spec.session, original, "reopen mints a fresh session id");
        assert_eq!(spec.cwd.as_deref(), Some("/repo"));
        assert_eq!(
            spec.launch,
            Launch::Claude {
                resume: Some("abc".into())
            }
        );
        assert_eq!(app.workspace.tabs.len(), 1);
        assert_eq!(app.workspace.tabs[0].title, "repo 🤖");
    }

    #[test]
    fn reopening_a_renamed_tab_restores_the_custom_title() {
        let mut app = App::new();
        launch(&mut app, "derived");
        app.apply(Event::RenameTab {
            index: 0,
            title: "Prod deploy".into(),
        });
        app.apply(Event::CloseTab(0));

        let effects = app.apply(Event::ReopenClosedTab);
        let new_id = match effects.as_slice() {
            [Effect::Spawn(spec)] => spec.session,
            other => panic!("expected one Spawn, got {other:?}"),
        };
        // The manual name round-trips the close/reopen, laid back over the
        // derived title — not lost, and still a real override.
        assert_eq!(app.workspace.tabs[0].display_title(), "Prod deploy");
        assert_eq!(app.workspace.tabs[0].title, "derived");
        // Being a real override, a later relabel still cannot clobber it.
        app.apply(Event::SessionTitleChanged {
            session: new_id,
            title: "new derived".into(),
        });
        assert_eq!(app.workspace.tabs[0].display_title(), "Prod deploy");
    }

    #[test]
    fn reopen_with_nothing_closed_is_a_noop() {
        let mut app = App::new();
        assert!(app.apply(Event::ReopenClosedTab).is_empty());
        // Even after a launch with no close, there is nothing on the stack.
        launch(&mut app, "a");
        assert!(app.apply(Event::ReopenClosedTab).is_empty());
    }

    #[test]
    fn reopen_walks_the_close_stack_in_lifo_order() {
        // Closing A then B and reopening twice restores B first, then A.
        let mut app = App::new();
        let open = |app: &mut App, dir: &str| {
            app.apply(Event::LaunchSession(LaunchSpec {
                cwd: Some(dir.into()),
                launch: Launch::Shell,
                title: dir.into(),
            }));
        };
        open(&mut app, "/a");
        open(&mut app, "/b");
        // Close the later tab (index 1 = /b) then the remaining one (/a).
        app.apply(Event::CloseTab(1));
        app.apply(Event::CloseTab(0));
        assert!(app.workspace.tabs.is_empty());

        let first = app.apply(Event::ReopenClosedTab);
        let second = app.apply(Event::ReopenClosedTab);
        let cwd_of = |effects: &[Effect]| match effects {
            [Effect::Spawn(spec)] => spec.cwd.clone(),
            other => panic!("expected one Spawn, got {other:?}"),
        };
        // LIFO: the last close (/a) comes back first, then /b.
        assert_eq!(cwd_of(&first).as_deref(), Some("/a"));
        assert_eq!(cwd_of(&second).as_deref(), Some("/b"));
        // Stack drained.
        assert!(app.apply(Event::ReopenClosedTab).is_empty());
    }

    #[test]
    fn session_title_changed_relabels_the_tab() {
        let mut app = App::new();
        let id = launch(&mut app, "old");
        let effects = app.apply(Event::SessionTitleChanged {
            session: id,
            title: "Claude's title".into(),
        });
        assert!(effects.is_empty());
        assert_eq!(app.workspace.tabs[0].title, "Claude's title");
    }

    #[test]
    fn open_url_emits_a_trimmed_open_effect() {
        let mut app = App::new();
        let effects = app.apply(Event::OpenUrl("  https://example.com  ".into()));
        assert!(matches!(
            effects.as_slice(),
            [Effect::OpenUrl(u)] if u == "https://example.com"
        ));
    }

    #[test]
    fn open_url_ignores_a_blank_string() {
        let mut app = App::new();
        assert!(app.apply(Event::OpenUrl("   ".into())).is_empty());
    }

    #[test]
    fn star_pins_a_session_and_persists_metadata() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![
            record("a", "/p", "first"),
            record("b", "/p", "second"),
        ]));
        // "b" is most-recent-first by mtime equal → group order; star "a".
        let effects = app.apply(Event::ToggleStar("a".into()));
        assert!(matches!(effects.as_slice(), [Effect::SaveMetadata(m)] if m.sessions["a"].starred));
        // Starred session now leads its group.
        let group = &app.visible_projects()[0];
        assert_eq!(group.sessions[0].session_id, "a");
        assert!(app.is_starred("a"));
    }

    #[test]
    fn star_pins_a_repo_to_the_top_and_persists() {
        let mut app = App::new();
        // Equal (missing) mtimes → groups fall back to path order: `/busy` first.
        app.apply(Event::ScanCompleted(vec![
            record("q", "/quiet", "q1"),
            record("b", "/busy", "b1"),
        ]));
        assert_eq!(app.visible_projects()[0].path, "/busy");

        // Starring the second repo pins it to the top of the sidebar.
        let effects = app.apply(Event::ToggleRepoStar("/quiet".into()));
        assert!(
            matches!(effects.as_slice(), [Effect::SaveMetadata(m)] if m.repos["/quiet"].starred)
        );
        assert!(app.is_repo_starred("/quiet"));
        let paths: Vec<_> = app
            .visible_projects()
            .iter()
            .map(|g| g.path.clone())
            .collect();
        assert_eq!(paths, vec!["/quiet", "/busy"]);
    }

    #[test]
    fn unstarring_a_repo_drops_its_entry() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record("a", "/p", "only")]));
        app.apply(Event::ToggleRepoStar("/p".into()));
        assert!(app.is_repo_starred("/p"));
        // Toggling back to the default drops the entry rather than persisting it.
        let effects = app.apply(Event::ToggleRepoStar("/p".into()));
        assert!(
            matches!(effects.as_slice(), [Effect::SaveMetadata(m)] if !m.repos.contains_key("/p"))
        );
        assert!(!app.is_repo_starred("/p"));
    }

    #[test]
    fn favorites_aggregate_starred_sessions_across_projects_most_recent_first() {
        let mut app = App::new();
        let mut newer = record("new", "/a", "recent");
        newer.modified = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(100));
        let mut older = record("old", "/b", "stale");
        older.modified = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(10));
        app.apply(Event::ScanCompleted(vec![
            newer,
            older,
            record("plain", "/a", "unstarred"),
        ]));
        app.apply(Event::ToggleStar("new".into()));
        app.apply(Event::ToggleStar("old".into()));

        let groups = app.visible_projects();
        let favs = app.favorite_sessions(&groups);
        let ids: Vec<_> = favs.iter().map(|(_, s)| s.session_id.as_str()).collect();
        assert_eq!(ids, vec!["new", "old"], "cross-project, most-recent-first");
        // Each favourite carries its project path so the row can resume it.
        assert_eq!(favs[0].0, "/a");
        assert_eq!(favs[1].0, "/b");
    }

    #[test]
    fn favorites_are_empty_without_stars() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record("a", "/p", "x")]));
        let groups = app.visible_projects();
        assert!(app.favorite_sessions(&groups).is_empty());
    }

    #[test]
    fn an_archived_starred_session_is_not_a_visible_favorite() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record("a", "/p", "x")]));
        app.apply(Event::ToggleStar("a".into()));
        app.apply(Event::ToggleArchive("a".into()));
        // Hidden by default, so it drops out of the visible groups favorites read.
        let groups = app.visible_projects();
        assert!(app.favorite_sessions(&groups).is_empty());
        // …but it returns once archived sessions are shown.
        app.apply(Event::ShowArchivedToggled(true));
        let groups = app.visible_projects();
        assert_eq!(app.favorite_sessions(&groups).len(), 1);
    }

    #[test]
    fn archived_sessions_hide_unless_shown() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![
            record("a", "/p", "keep"),
            record("b", "/p", "hideme"),
        ]));
        app.apply(Event::ToggleArchive("b".into()));
        // Hidden by default…
        let visible = app.visible_projects();
        assert_eq!(visible[0].sessions.len(), 1);
        assert_eq!(visible[0].sessions[0].session_id, "a");
        // …shown when the toggle is on.
        app.apply(Event::ShowArchivedToggled(true));
        assert_eq!(app.visible_projects()[0].sessions.len(), 2);
    }

    #[test]
    fn toggle_sidebar_flips_and_starts_visible() {
        let mut app = App::new();
        assert!(!app.sidebar_hidden, "sidebar is visible on launch");
        assert!(app.apply(Event::ToggleSidebar).is_empty());
        assert!(app.sidebar_hidden);
        app.apply(Event::ToggleSidebar);
        assert!(!app.sidebar_hidden, "a second toggle restores it");
    }

    #[test]
    fn archiving_the_only_session_drops_the_empty_group() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record("a", "/solo", "only")]));
        app.apply(Event::ToggleArchive("a".into()));
        assert!(app.visible_projects().is_empty());
    }

    #[test]
    fn rename_overrides_the_title_and_clearing_restores_it() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record(
            "a",
            "/p",
            "derived summary",
        )]));
        let derived = app.session_title(&app.projects[0].sessions[0].clone());

        app.apply(Event::RenameSession {
            session: "a".into(),
            title: "  My Title  ".into(),
        });
        assert_eq!(
            app.session_title(&app.projects[0].sessions[0].clone()),
            "My Title"
        );

        // Clearing (empty title) drops the entry back to the derived title.
        let effects = app.apply(Event::RenameSession {
            session: "a".into(),
            title: "   ".into(),
        });
        assert!(
            matches!(effects.as_slice(), [Effect::SaveMetadata(m)] if !m.sessions.contains_key("a"))
        );
        assert_eq!(
            app.session_title(&app.projects[0].sessions[0].clone()),
            derived
        );
    }

    #[test]
    fn renaming_a_session_retitles_its_open_tab_and_clearing_restores_the_name() {
        // Follow-up: a sidebar rename must retitle the live tab too, not
        // just the sidebar row — and clearing it restores the digest name.
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record(
            "a",
            "/p",
            "derived summary",
        )]));
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/p".into()),
            launch: Launch::Claude {
                resume: Some("a".into()),
            },
            title: "derived summary".into(),
        }));
        let session = app.workspace.focused_session().expect("a launched tab");

        app.apply(Event::RenameSession {
            session: "a".into(),
            title: "My Title".into(),
        });
        assert_eq!(
            app.workspace.session_title(session),
            Some("My Title"),
            "a sidebar rename retitles the open tab"
        );

        app.apply(Event::RenameSession {
            session: "a".into(),
            title: "  ".into(),
        });
        assert_eq!(
            app.workspace.session_title(session),
            Some("derived summary"),
            "clearing the rename restores the digest name on the open tab"
        );
    }

    #[test]
    fn colliding_titles_flags_only_shared_titles_and_a_rename_resolves_it() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![
            record("dup1", "/p", "vm tombée"),
            record("dup2", "/p", "vm tombée"),
            record("uniq", "/p", "something else"),
        ]));
        let group = app.projects[0].clone();

        let collisions = app.colliding_titles(&group);
        assert_eq!(
            collisions,
            HashSet::from(["dup1".to_owned(), "dup2".to_owned()])
        );

        // Renaming one of the pair to a unique title clears the collision for
        // both — the set is checked on the resolved title.
        app.apply(Event::RenameSession {
            session: "dup1".into(),
            title: "the original".into(),
        });
        assert!(app.colliding_titles(&group).is_empty());
    }

    #[test]
    fn collision_subtitle_surfaces_a_masked_summary_but_not_a_plain_one() {
        let mut app = App::new();
        // Two sessions Claude Code gave the same custom title (the /clear
        // title-carryover), masking two different real first prompts.
        let mut carried = record("clr", "/p", "regardons les soucis du ROR");
        carried.digest.custom_title = Some("login/logout petit souci".into());
        let mut original = record("orig", "/p", "ouvre un worktree auth/login");
        original.digest.custom_title = Some("login/logout petit souci".into());
        app.apply(Event::ScanCompleted(vec![
            carried.clone(),
            original.clone(),
        ]));

        // Each colliding row falls back to its real summary, so the two are
        // distinguishable by content, not just by age.
        assert_eq!(
            app.collision_subtitle(&carried).as_deref(),
            Some("regardons les soucis du ROR")
        );
        assert_eq!(
            app.collision_subtitle(&original).as_deref(),
            Some("ouvre un worktree auth/login")
        );

        // A row whose title *is* its summary (no masking) has nothing extra to
        // show — the caller keeps the age disambiguator.
        let plain = record("plain", "/p", "vm tombée");
        assert_eq!(app.collision_subtitle(&plain), None);

        // A user rename that matches the summary is likewise not a divergence.
        app.apply(Event::RenameSession {
            session: "clr".into(),
            title: "regardons les soucis du ROR".into(),
        });
        assert_eq!(app.collision_subtitle(&carried), None);
    }

    #[test]
    fn toggling_collapse_folds_then_unfolds_and_persists() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record("a", "/p", "only")]));
        assert!(!app.is_collapsed("/p"));

        // First toggle folds the project and persists the set containing it.
        let effects = app.apply(Event::ToggleCollapsed("/p".into()));
        assert!(app.is_collapsed("/p"));
        assert!(matches!(effects.as_slice(), [Effect::SaveCollapsed(c)] if c.contains("/p")));

        // A second toggle unfolds it and persists the now-empty set.
        let effects = app.apply(Event::ToggleCollapsed("/p".into()));
        assert!(!app.is_collapsed("/p"));
        assert!(matches!(effects.as_slice(), [Effect::SaveCollapsed(c)] if !c.contains("/p")));
    }

    #[test]
    fn collapsed_state_loads_and_survives_a_rescan() {
        let mut app = App::new();
        app.apply(Event::CollapsedLoaded(HashSet::from(["/p".to_owned()])));
        assert!(app.is_collapsed("/p"));
        // A fold is a sidebar preference, not a property of the scan: a later
        // scan of the same project must keep it folded.
        app.apply(Event::ScanCompleted(vec![record("a", "/p", "only")]));
        assert!(app.is_collapsed("/p"));
    }

    #[test]
    fn split_focused_spawns_a_sibling_inheriting_the_cwd() {
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Shell,
            title: "proj".into(),
        }));
        let effects = app.apply(Event::SplitFocused(SplitDir::Vertical));
        // A new session spawns in the same directory and is focused.
        let new = app.workspace.focused_session().expect("focused pane");
        assert_eq!(app.sessions.len(), 2);
        assert_eq!(app.sessions[&new].cwd.as_deref(), Some("/proj"));
        match effects.as_slice() {
            [Effect::Spawn(spec)] => {
                assert_eq!(spec.session, new);
                assert_eq!(spec.cwd.as_deref(), Some("/proj"));
            }
            other => panic!("expected one Spawn, got {other:?}"),
        }
    }

    #[test]
    fn close_focused_pane_kills_only_that_session() {
        let mut app = App::new();
        let first = launch(&mut app, "a");
        app.apply(Event::SplitFocused(SplitDir::Horizontal));
        let split = app.workspace.focused_session().expect("focused pane");

        let effects = app.apply(Event::CloseFocusedPane);
        assert!(matches!(effects.as_slice(), [Effect::Kill(id)] if *id == split));
        assert!(!app.sessions.contains_key(&split));
        // The original session survives and regains focus.
        assert_eq!(app.workspace.focused_session(), Some(first));
        assert!(app.sessions.contains_key(&first));
    }

    #[test]
    fn focus_pane_events_move_the_focused_session() {
        let mut app = App::new();
        let first = launch(&mut app, "a");
        app.apply(Event::SplitFocused(SplitDir::Vertical));
        let second = app.workspace.focused_session().expect("focused pane");
        assert_ne!(first, second);

        app.apply(Event::FocusPrevPane);
        assert_eq!(app.workspace.focused_session(), Some(first));
        app.apply(Event::FocusNextPane);
        assert_eq!(app.workspace.focused_session(), Some(second));
    }

    #[test]
    fn tab_status_reports_the_most_urgent_session_status() {
        let mut app = App::new();
        let id = launch(&mut app, "a");
        assert_eq!(app.tab_status(0), Some(SessionStatus::Starting));

        app.apply(Event::StatusChanged {
            session: id,
            status: SessionStatus::Attention,
        });
        assert_eq!(app.tab_status(0), Some(SessionStatus::Attention));
        // Unknown tab index has no status.
        assert_eq!(app.tab_status(7), None);
    }

    // ---- close-tab confirmation: is a foreground process running? ----

    /// Launch a Claude session and return its id — companion to `launch`, which
    /// spawns a plain shell.
    fn launch_claude(app: &mut App) -> SessionId {
        match app
            .apply(Event::LaunchSession(LaunchSpec {
                cwd: None,
                launch: Launch::Claude { resume: None },
                title: "claude".into(),
            }))
            .as_slice()
        {
            [Effect::Spawn(spec)] => spec.session,
            other => panic!("expected Spawn, got {other:?}"),
        }
    }

    #[test]
    fn an_idle_plain_shell_tab_has_no_running_process() {
        let mut app = App::new();
        let id = launch(&mut app, "shell");
        // Freshly launched it is `Starting`, then settles to `Idle`; in neither
        // state is there foreground work a close would lose.
        assert!(!app.tab_has_running_process(0));
        app.apply(Event::StatusChanged {
            session: id,
            status: SessionStatus::Idle,
        });
        assert!(!app.tab_has_running_process(0));
    }

    #[test]
    fn a_working_or_blocked_shell_tab_has_a_running_process() {
        for status in [SessionStatus::Busy, SessionStatus::Attention] {
            let mut app = App::new();
            let id = launch(&mut app, "shell");
            app.apply(Event::StatusChanged {
                session: id,
                status,
            });
            assert!(
                app.tab_has_running_process(0),
                "a {status:?} shell has foreground work to lose"
            );
        }
    }

    #[test]
    fn a_claude_tab_has_a_running_process_across_every_live_status() {
        let mut app = App::new();
        let id = launch_claude(&mut app);
        // The `claude` process runs in the shell's foreground until it exits, so
        // every live status counts — an idle prompt included.
        for status in [
            SessionStatus::Starting,
            SessionStatus::Idle,
            SessionStatus::Busy,
            SessionStatus::Attention,
        ] {
            app.apply(Event::StatusChanged {
                session: id,
                status,
            });
            assert!(
                app.tab_has_running_process(0),
                "a live Claude ({status:?}) is a running process"
            );
        }
    }

    #[test]
    fn an_exited_tab_has_no_running_process() {
        let mut app = App::new();
        let id = launch_claude(&mut app);
        assert!(app.tab_has_running_process(0));
        app.apply(Event::PtyExited(id));
        assert!(
            !app.tab_has_running_process(0),
            "nothing is left to kill once the PTY has exited"
        );
    }

    #[test]
    fn a_split_tab_is_running_when_any_pane_is() {
        // Two plain shells split into one tab: idle throughout, the tab closes
        // silently; promote either pane to Busy and the whole tab now hosts
        // running work.
        let mut app = App::new();
        let left = launch(&mut app, "left");
        app.apply(Event::SplitFocused(SplitDir::Vertical));
        assert!(
            !app.tab_has_running_process(0),
            "two idle shells have nothing to lose"
        );
        app.apply(Event::StatusChanged {
            session: left,
            status: SessionStatus::Busy,
        });
        assert!(
            app.tab_has_running_process(0),
            "one busy pane makes the whole tab a running tab"
        );
    }

    #[test]
    fn an_unknown_tab_index_has_no_running_process() {
        let mut app = App::new();
        launch_claude(&mut app);
        assert!(
            !app.tab_has_running_process(9),
            "a stale index must never claim a running process"
        );
    }

    #[test]
    fn any_running_process_spans_every_tab() {
        // The app-wide predicate is true iff some session anywhere is running,
        // regardless of which tab hosts it.
        let mut app = App::new();
        assert!(
            !app.any_running_process(),
            "an empty app has nothing running"
        );

        let idle = launch(&mut app, "idle");
        launch(&mut app, "other"); // a second, unrelated tab
        assert!(
            !app.any_running_process(),
            "two idle plain shells: nothing worth confirming a quit over"
        );

        // Promote the first shell to Busy — now the app as a whole is running,
        // even though it lives in a background tab.
        app.apply(Event::StatusChanged {
            session: idle,
            status: SessionStatus::Busy,
        });
        assert!(
            app.any_running_process(),
            "one busy session anywhere makes the app a running app"
        );
    }

    #[test]
    fn live_session_count_excludes_exited_sessions() {
        // The quit-confirm summary counts sessions a quit would hard-kill:
        // everything not yet Exited, whatever its running state.
        let mut app = App::new();
        assert_eq!(app.live_session_count(), 0, "an empty app has none live");

        let a = launch(&mut app, "a");
        launch(&mut app, "b");
        assert_eq!(app.live_session_count(), 2, "two launched shells are live");

        // An idle (but not exited) session still counts — it has a process.
        app.apply(Event::StatusChanged {
            session: a,
            status: SessionStatus::Idle,
        });
        assert_eq!(app.live_session_count(), 2, "idle is still live");

        // Exiting one drops it from the count; the map may still hold it.
        app.apply(Event::PtyExited(a));
        assert_eq!(
            app.live_session_count(),
            1,
            "an exited session no longer counts"
        );
    }

    #[test]
    fn the_state_only_events_yield_no_effects() {
        // The shell routes these through its one executor precisely because they
        // yield nothing today (pure state mutations); if any starts returning
        // effects, the shell now performs them — and this guard flags the change
        // so the new effect is reviewed rather than silently performed.
        let mut app = App::new();
        let session = launch(&mut app, "a");
        app.apply(Event::ScanCompleted(vec![record("abc", "/p", "x")]));
        let effect_free = [
            Event::SearchChanged("q".into()),
            Event::SearchTitlesOnlyToggled(true),
            Event::ToggleSidebar,
            Event::Zoom(Zoom::In),
            Event::ToggleExpanded("/p".into()),
            Event::ShowArchivedToggled(true),
            Event::ActivateTab(0),
            Event::MoveTab { from: 0, to: 0 },
            Event::WindowFocusChanged(false),
            Event::StatusChanged {
                session,
                status: SessionStatus::Idle,
            },
            Event::SessionTitleChanged {
                session,
                title: "t".into(),
            },
            Event::RenameTab {
                index: 0,
                title: "t".into(),
            },
            Event::PtyExited(session),
        ];
        for event in effect_free {
            let label = format!("{event:?}");
            assert!(
                app.apply(event).is_empty(),
                "{label} must stay effect-free (the shell routes it through perform)"
            );
        }
    }

    #[test]
    fn status_changes_are_recorded_but_never_revive_an_exited_session() {
        let mut app = App::new();
        let spawn = app.apply(Event::LaunchSession(LaunchSpec {
            cwd: None,
            launch: Launch::Shell,
            title: "a".into(),
        }));
        let id = match spawn.as_slice() {
            [Effect::Spawn(spec)] => spec.session,
            other => panic!("expected Spawn, got {other:?}"),
        };

        app.apply(Event::StatusChanged {
            session: id,
            status: SessionStatus::Busy,
        });
        assert_eq!(app.sessions[&id].status, SessionStatus::Busy);

        app.apply(Event::PtyExited(id));
        app.apply(Event::StatusChanged {
            session: id,
            status: SessionStatus::Idle,
        });
        assert_eq!(app.sessions[&id].status, SessionStatus::Exited);
    }

    // ---- OSC 9 notifications forwarded to the OS notification centre ----

    /// The single `Effect::Notify` a `SessionNotified` event should produce, or
    /// `None` if the policy dropped it. Panics on any other effect shape so a
    /// regression that emits the wrong effect fails loudly.
    fn notify_effect(effects: &[Effect]) -> Option<(&str, &str)> {
        match effects {
            [] => None,
            [Effect::Notify { title, body }] => Some((title, body)),
            other => panic!("expected at most one Notify, got {other:?}"),
        }
    }

    #[test]
    fn osc9_notification_posts_a_desktop_notification_titled_with_its_session() {
        let mut app = App::new();
        let id = launch(&mut app, "myproj");

        let effects = app.apply(Event::SessionNotified {
            session: id,
            body: "Claude needs your attention".into(),
        });

        // The body is Claude's own message; the title names which session wants
        // the user, taken from the tab the user sees.
        assert_eq!(
            notify_effect(&effects),
            Some(("myproj", "Claude needs your attention"))
        );
    }

    #[test]
    fn a_blank_notification_body_falls_back_to_a_default_message() {
        let mut app = App::new();
        let id = launch(&mut app, "myproj");

        // Claude sometimes fires a bare OSC 9 with no text; the OS notification
        // still has to say something actionable.
        let effects = app.apply(Event::SessionNotified {
            session: id,
            body: "   ".into(),
        });

        assert_eq!(
            notify_effect(&effects),
            Some(("myproj", DEFAULT_NOTIFICATION_BODY))
        );
    }

    #[test]
    fn a_notification_for_an_unknown_session_is_dropped() {
        let mut app = App::new();
        let _present = launch(&mut app, "myproj");

        let effects = app.apply(Event::SessionNotified {
            session: SessionId(NonZeroU64::new(9_999).expect("non-zero")),
            body: "ghost".into(),
        });

        assert_eq!(notify_effect(&effects), None);
    }

    #[test]
    fn a_notification_for_an_exited_session_is_dropped() {
        let mut app = App::new();
        let id = launch(&mut app, "myproj");
        app.apply(Event::PtyExited(id));

        // Nothing to return to — a dead session must not raise a desktop alert.
        let effects = app.apply(Event::SessionNotified {
            session: id,
            body: "too late".into(),
        });

        assert_eq!(notify_effect(&effects), None);
    }

    #[test]
    fn a_notification_follows_the_sessions_latest_tab_title() {
        let mut app = App::new();
        let id = launch(&mut app, "old name");
        // Claude relabels the tab over OSC; the notification title must
        // track that, not the launch label.
        app.apply(Event::SessionTitleChanged {
            session: id,
            title: "renamed".into(),
        });

        let effects = app.apply(Event::SessionNotified {
            session: id,
            body: "ping".into(),
        });

        assert_eq!(notify_effect(&effects), Some(("renamed", "ping")));
    }

    // ---- background-tab notifications while the window keeps focus ----

    #[test]
    fn a_notification_for_the_viewed_session_is_dropped_while_the_window_is_focused() {
        let mut app = App::new();
        let id = launch(&mut app, "myproj");
        app.apply(Event::WindowFocusChanged(true));

        // The user is looking straight at this session; no banner is needed.
        let effects = app.apply(Event::SessionNotified {
            session: id,
            body: "ping".into(),
        });

        assert_eq!(notify_effect(&effects), None);
    }

    #[test]
    fn a_notification_for_a_background_tab_still_posts_while_the_window_is_focused() {
        let mut app = App::new();
        let background = launch(&mut app, "a");
        let _foreground = launch(&mut app, "b");
        assert_eq!(app.workspace.focused_session(), Some(_foreground));
        app.apply(Event::WindowFocusChanged(true));

        // The active tab is "b"; a notification from "a" (a background tab) must
        // still reach the OS — the OS's own per-window suppression only covers
        // the tab the user is actually viewing.
        let effects = app.apply(Event::SessionNotified {
            session: background,
            body: "ping".into(),
        });

        assert_eq!(notify_effect(&effects), Some(("a", "ping")));
    }

    #[test]
    fn a_notification_for_the_viewed_session_still_posts_while_the_window_is_unfocused() {
        let mut app = App::new();
        let id = launch(&mut app, "myproj");
        app.apply(Event::WindowFocusChanged(true));
        app.apply(Event::WindowFocusChanged(false));

        // Termherd itself is out of focus (another app is frontmost); today's
        // OS-suppression behaviour still applies, so the effect must still fire.
        let effects = app.apply(Event::SessionNotified {
            session: id,
            body: "ping".into(),
        });

        assert_eq!(notify_effect(&effects), Some(("myproj", "ping")));
    }

    // ---- capture snapshot for the AI dev loop ----

    /// The single `Effect::Capture` payload a `Capture` event should produce.
    /// Panics on any other effect shape so a regression fails loudly.
    fn capture_dump(effects: &[Effect]) -> &CaptureDump {
        match effects {
            [Effect::Capture(dump)] => dump,
            other => panic!("expected one Capture effect, got {other:?}"),
        }
    }

    #[test]
    fn capture_snapshots_tabs_focus_status_and_pty_text() {
        let mut app = App::new();
        let first = launch(&mut app, "proj $");
        let second = launch(&mut app, "repo 🤖");
        app.apply(Event::StatusChanged {
            session: second,
            status: SessionStatus::Busy,
        });

        let effects = app.apply(Event::Capture {
            focused_pty_text: Some("$ cargo test\nok".to_owned()),
        });
        let dump = capture_dump(&effects);

        // The active tab is the last launched one, carrying its focus.
        assert_eq!(dump.active_tab, Some(1));
        assert_eq!(dump.tabs.len(), 2);
        assert_eq!(dump.focused_pty.as_deref(), Some("$ cargo test\nok"));

        let tab0 = &dump.tabs[0];
        assert!(!tab0.active);
        assert_eq!(tab0.title, "proj $");
        assert_eq!(tab0.status, Some(SessionStatus::Starting));
        assert_eq!(tab0.sessions, vec![first.0.get()]);
        assert_eq!(
            tab0.focus_session, None,
            "only the active tab reports focus"
        );

        let tab1 = &dump.tabs[1];
        assert!(tab1.active);
        assert_eq!(tab1.title, "repo 🤖");
        assert_eq!(tab1.status, Some(SessionStatus::Busy));
        assert_eq!(tab1.sessions, vec![second.0.get()]);
        assert_eq!(tab1.focus_session, Some(second.0.get()));
    }

    #[test]
    fn capture_reports_a_tabs_custom_title_not_its_derived_one() {
        let mut app = App::new();
        launch(&mut app, "derived");
        app.apply(Event::RenameTab {
            index: 0,
            title: "My work".into(),
        });

        let effects = app.apply(Event::Capture {
            focused_pty_text: None,
        });
        let dump = capture_dump(&effects);
        // The dump must match what the user sees on the chip, or an AI reading
        // the state would name the tab wrong.
        assert_eq!(dump.tabs[0].title, "My work");
    }

    #[test]
    fn capture_on_an_empty_workspace_has_no_active_tab() {
        let mut app = App::new();
        let effects = app.apply(Event::Capture {
            focused_pty_text: None,
        });
        let dump = capture_dump(&effects);
        assert_eq!(dump.active_tab, None);
        assert!(dump.tabs.is_empty());
        assert_eq!(dump.focused_pty, None);
    }

    #[test]
    fn capture_lists_split_pane_membership_in_order() {
        // A split tab hosts several sessions; the dump records them in pane
        // order and points focus at the newest pane (layout/state proxy).
        let mut app = App::new();
        let base = launch(&mut app, "proj");
        app.apply(Event::SplitFocused(SplitDir::Vertical));
        let split = app.workspace.focused_session().expect("focused split pane");

        let effects = app.apply(Event::Capture {
            focused_pty_text: None,
        });
        let dump = capture_dump(&effects);
        let tab = &dump.tabs[0];
        assert_eq!(tab.sessions, vec![base.0.get(), split.0.get()]);
        assert_eq!(tab.focus_session, Some(split.0.get()));
    }

    // ---- GIF screencast record state machine ----

    #[test]
    fn toggle_record_starts_then_a_manual_toggle_finishes() {
        let mut app = App::new();
        assert!(!app.is_recording());

        let start = app.apply(Event::ToggleRecord { max_frames: 10 });
        assert!(matches!(start.as_slice(), [Effect::StartRecording]));
        assert!(app.is_recording());

        // Capture a couple of frames, then stop by hand.
        assert!(matches!(
            app.apply(Event::RecordTick).as_slice(),
            [Effect::CaptureFrame]
        ));
        assert!(matches!(
            app.apply(Event::RecordTick).as_slice(),
            [Effect::CaptureFrame]
        ));
        let stop = app.apply(Event::ToggleRecord { max_frames: 10 });
        assert!(matches!(
            stop.as_slice(),
            [Effect::FinishRecording { capped: false }]
        ));
        assert!(!app.is_recording());
    }

    #[test]
    fn the_frame_cap_auto_stops_the_recording() {
        let mut app = App::new();
        app.apply(Event::ToggleRecord { max_frames: 3 });

        // The first two ticks just capture; the third hits the cap and finishes.
        assert!(matches!(
            app.apply(Event::RecordTick).as_slice(),
            [Effect::CaptureFrame]
        ));
        assert!(matches!(
            app.apply(Event::RecordTick).as_slice(),
            [Effect::CaptureFrame]
        ));
        let last = app.apply(Event::RecordTick);
        assert!(
            matches!(
                last.as_slice(),
                [
                    Effect::CaptureFrame,
                    Effect::FinishRecording { capped: true }
                ]
            ),
            "the cap frame is captured, then the recording finishes, got {last:?}"
        );
        assert!(!app.is_recording(), "the cap auto-stops the recording");

        // A stray tick after the auto-stop is a silent no-op.
        assert!(app.apply(Event::RecordTick).is_empty());
    }

    #[test]
    fn stopping_before_any_frame_cancels_without_writing() {
        // The zero-frame guard: start then immediately stop → no file.
        let mut app = App::new();
        app.apply(Event::ToggleRecord { max_frames: 10 });
        let stop = app.apply(Event::ToggleRecord { max_frames: 10 });
        assert!(matches!(stop.as_slice(), [Effect::CancelRecording]));
        assert!(!app.is_recording());
    }

    #[test]
    fn a_zero_cap_record_is_a_noop() {
        let mut app = App::new();
        assert!(app.apply(Event::ToggleRecord { max_frames: 0 }).is_empty());
        assert!(!app.is_recording());
    }

    #[test]
    fn a_record_tick_while_idle_is_a_noop() {
        let mut app = App::new();
        assert!(app.apply(Event::RecordTick).is_empty());
        assert!(!app.is_recording());
    }

    #[test]
    fn recording_progress_tracks_frames_against_the_cap() {
        let mut app = App::new();
        assert_eq!(app.recording_progress(), None, "idle has no progress");

        app.apply(Event::ToggleRecord { max_frames: 3 });
        assert_eq!(app.recording_progress(), Some((0, 3)), "starts at 0/cap");

        app.apply(Event::RecordTick);
        assert_eq!(app.recording_progress(), Some((1, 3)));

        // The cap tick finishes the recording, so progress clears.
        app.apply(Event::RecordTick);
        app.apply(Event::RecordTick);
        assert_eq!(
            app.recording_progress(),
            None,
            "cleared once the cap stops it"
        );
    }

    proptest::proptest! {
        /// For any cap ≥ 1, exactly `max_frames` ticks capture `max_frames`
        /// frames and produce exactly one `FinishRecording`, leaving the app
        /// idle — and `apply` never panics (Q5).
        #[test]
        fn a_recording_captures_exactly_its_cap_then_finishes(max_frames in 1u32..200) {
            let mut app = App::new();
            app.apply(Event::ToggleRecord { max_frames });

            let mut captured = 0u32;
            let mut finishes = 0u32;
            for _ in 0..max_frames {
                for effect in app.apply(Event::RecordTick) {
                    match effect {
                        Effect::CaptureFrame => captured += 1,
                        Effect::FinishRecording { .. } => finishes += 1,
                        other => proptest::prop_assert!(false, "unexpected {:?}", other),
                    }
                }
            }
            proptest::prop_assert_eq!(captured, max_frames);
            proptest::prop_assert_eq!(finishes, 1);
            proptest::prop_assert!(!app.is_recording());
        }
    }

    proptest::proptest! {
        /// For any live session and any body, exactly one notification is
        /// posted, its title is the tab title and its body is preserved
        /// verbatim when non-blank — and `apply` never panics (Q5).
        #[test]
        fn live_session_notifications_preserve_body_and_title(
            title in "[^\u{0}]{0,40}",
            body in "\\PC{1,80}",
        ) {
            let mut app = App::new();
            let id = launch(&mut app, title.as_str());

            let effects = app.apply(Event::SessionNotified { session: id, body: body.clone() });

            let expected_body = if body.trim().is_empty() {
                DEFAULT_NOTIFICATION_BODY.to_owned()
            } else {
                body
            };
            proptest::prop_assert_eq!(
                notify_effect(&effects),
                Some((title.as_str(), expected_body.as_str()))
            );
        }

        /// A notification for a session that was never launched is always
        /// dropped, whatever the body — no panic, no effect.
        #[test]
        fn unknown_session_notifications_are_always_dropped(
            raw_id in 1u64..1_000_000,
            body in "\\PC{0,80}",
        ) {
            let mut app = App::new();
            let id = SessionId(NonZeroU64::new(raw_id).expect("non-zero"));

            let effects = app.apply(Event::SessionNotified { session: id, body });

            proptest::prop_assert_eq!(notify_effect(&effects), None);
        }
    }
}
