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
};
use crate::metadata::SessionMeta;
use crate::workspace::{SessionId, SplitDir, Workspace};

/// Cell size a freshly launched PTY starts at, before the widget reports its
/// real geometry via [`Event::TerminalResized`].
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// Shown as the desktop notification body when Claude fires a bare OSC 9 with
/// no text of its own (#29).
const DEFAULT_NOTIFICATION_BODY: &str = "Claude needs your attention";

/// Notification title fallback when a session somehow has no hosting tab (#29);
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
    /// Whether archived sessions show in the browser.
    pub show_archived: bool,
    /// Whether the session-browser sidebar is collapsed to give the terminal
    /// the full width (#21). Ephemeral — resets to visible each launch.
    pub sidebar_hidden: bool,
    /// Project paths whose session list is folded shut in the sidebar (#22);
    /// persisted to `~/.termherd` so the fold survives a restart.
    pub collapsed: HashSet<String>,
    /// Monotonic source of `SessionId`s; never reused within a run. This is
    /// the structural fix for the `realSessionId` race (Q6) — ids are minted
    /// here, single-threaded, before any PTY exists.
    next_session: u64,
    /// LIFO stack of recently closed tabs, for reopen (#78). Capped at
    /// [`MAX_CLOSED_TABS`] so a long session can't grow it without bound;
    /// closing past the cap drops the oldest entry.
    closed_tabs: Vec<ClosedTab>,
}

/// How many closed tabs the reopen stack (#78) remembers. Walking back further
/// than this is rare enough that the unbounded-growth risk outweighs it.
const MAX_CLOSED_TABS: usize = 16;

/// Enough of a closed tab to recreate it on reopen (#78): the kind it ran, the
/// directory it ran in, and the label it carried. A split tab is reduced to its
/// first pane — reopen restores a single terminal, not the whole pane tree.
#[derive(Debug, Clone)]
pub struct ClosedTab {
    pub title: String,
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

/// Where to move a terminal's viewport (#44). One scroll concept covers the
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
    /// relative scrollback nudge when it isn't one (#98).
    Wheel { col: u16, row: u16, lines: i32 },
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
    /// The user moved a terminal's viewport (FR4 scrollback): a relative wheel
    /// delta, or an absolute jump to the top/bottom of the history (#44).
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
    /// The session reported a new title over OSC (#24); relabel its tab.
    SessionTitleChanged {
        session: SessionId,
        title: String,
    },
    /// The user clicked a tab to bring it to the front (FR5).
    ActivateTab(usize),
    /// The user closed a tab (FR5); its sessions' PTYs are killed.
    CloseTab(usize),
    /// The user dragged the tab at `from` to rest at index `to` (FR5, #25). A
    /// pure reorder: no PTY is touched, so it yields no effects.
    MoveTab {
        from: usize,
        to: usize,
    },
    /// Reopen the most recently closed tab (#78), restoring its mode and
    /// directory. A no-op when nothing has been closed.
    ReopenClosedTab,
    /// Split the focused pane, opening a fresh session beside it (FR6).
    SplitFocused(SplitDir),
    /// Close the focused pane (FR6); its PTY is killed and the split collapses.
    CloseFocusedPane,
    /// Move focus to the next / previous pane in the active tab (FR6).
    FocusNextPane,
    FocusPrevPane,
    /// Persisted metadata loaded at startup (`F-session-metadata`).
    MetadataLoaded(HashMap<String, SessionMeta>),
    /// Toggle a session's star, by Claude session id.
    ToggleStar(String),
    /// Toggle a session's archived flag, by Claude session id.
    ToggleArchive(String),
    /// Set (or clear, when empty) a session's custom title.
    RenameSession {
        session: String,
        title: String,
    },
    /// Show or hide archived sessions in the browser.
    ShowArchivedToggled(bool),
    /// Collapse or restore the session-browser sidebar (#21).
    ToggleSidebar,
    /// Persisted fold state loaded at startup (#22): the folded project paths.
    CollapsedLoaded(HashSet<String>),
    /// Fold or unfold a project's session list in the sidebar, by path (#22).
    ToggleCollapsed(String),
    /// The user Ctrl/Cmd+clicked a detected link in a terminal (#28).
    OpenUrl(String),
    /// A session emitted an OSC 9 notification — Claude wants the user (#29).
    /// `body` is the raw payload Claude sent ("needs your attention", a
    /// permission prompt, …). Routed to the OS notification centre on top of
    /// the in-app `Attention` status.
    SessionNotified {
        session: SessionId,
        body: String,
    },
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
    /// the top/bottom of the scrollback (#44).
    Scroll {
        session: SessionId,
        target: ScrollTarget,
    },
    /// Terminate a session's PTY process.
    Kill(SessionId),
    /// Persist the session metadata overlay (`F-session-metadata`).
    SaveMetadata(HashMap<String, SessionMeta>),
    /// Persist the folded-project set (#22).
    SaveCollapsed(HashSet<String>),
    /// Open a URL in the OS default handler (#28); the shell performs it.
    OpenUrl(String),
    /// Post a desktop notification to the OS notification centre (#29). The
    /// shell performs it; `title` names the session/project that wants the
    /// user, `body` is Claude's message.
    Notify { title: String, body: String },
}

impl App {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply an event, returning the effects the runtime must carry out.
    /// **Pure**: no I/O, no clock, no panic.
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
            Event::MetadataLoaded(metadata) => {
                self.metadata = metadata;
                Vec::new()
            }
            Event::ToggleStar(session) => {
                self.update_meta(session, |meta| meta.starred = !meta.starred)
            }
            Event::ToggleArchive(session) => {
                self.update_meta(session, |meta| meta.archived = !meta.archived)
            }
            Event::RenameSession { session, title } => self.update_meta(session, |meta| {
                let trimmed = title.trim();
                meta.title = (!trimmed.is_empty()).then(|| trimmed.to_owned());
            }),
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
        }
    }

    /// The sidebar's view of the projects: search matches (FR3) with the
    /// metadata overlay applied (`F-session-metadata`) — archived sessions
    /// hidden unless [`Self::show_archived`], starred sessions pinned to the
    /// top of their group, and emptied groups dropped.
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
        groups
    }

    /// The located content hit for a session under the current search (#58),
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
    /// disambiguator in the sidebar (#42). Collision is checked on the *final*
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
    /// in its group (#42): the session's real first-prompt `summary` when it
    /// *diverges* from the shown title. A custom/AI title or rename can mask a
    /// completely different conversation — Claude Code carries a custom title
    /// across `/clear` into a fresh, unrelated session (#93), so two rows read
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
    /// session card the sidebar shows (#76) instead of a second derive. `None`
    /// for a shell or a fresh, not-yet-scanned session.
    #[must_use]
    pub fn record_for(&self, claude_id: &str) -> Option<&SessionRecord> {
        self.projects
            .iter()
            .flat_map(|group| &group.sessions)
            .find(|record| record.session_id == claude_id)
    }

    /// The browsed record for the tab at `index` — the sidebar entry its first
    /// pane resumes (#76), so a tab hover can show the same session card. `None`
    /// for an out-of-range index, or a tab whose first pane is a shell or a
    /// fresh, not-yet-scanned session (no resume id / no record).
    #[must_use]
    pub fn tab_record(&self, index: usize) -> Option<&SessionRecord> {
        let tab = self.workspace.tabs.get(index)?;
        let first = tab.sessions().first().copied()?;
        let claude_id = self.sessions.get(&first)?.launch.resume_id()?;
        self.record_for(claude_id)
    }

    /// Whether a project's session list is folded shut in the sidebar (#22).
    #[must_use]
    pub fn is_collapsed(&self, path: &str) -> bool {
        self.collapsed.contains(path)
    }

    /// Flip a project's fold state and emit the persistence effect (#22).
    fn toggle_collapsed(&mut self, path: String) -> Vec<Effect> {
        if !self.collapsed.remove(&path) {
            self.collapsed.insert(path);
        }
        vec![Effect::SaveCollapsed(self.collapsed.clone())]
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
        vec![Effect::SaveMetadata(self.metadata.clone())]
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
    /// Snapshots the tab onto the reopen stack first (#78), so the close can be
    /// undone before its sessions are forgotten.
    fn close_tab(&mut self, index: usize) -> Vec<Effect> {
        self.remember_closed_tab(index);
        let sessions = self.workspace.close_tab(index);
        for id in &sessions {
            self.sessions.remove(id);
        }
        sessions.into_iter().map(Effect::Kill).collect()
    }

    /// Push the tab at `index` onto the reopen stack (#78), capturing the kind,
    /// directory and label needed to recreate it. Reduced to the tab's first
    /// pane — reopen restores one terminal, not a whole split. A no-op for an
    /// out-of-range index or a tab whose first session is no longer live.
    fn remember_closed_tab(&mut self, index: usize) {
        let Some(tab) = self.workspace.tabs.get(index) else {
            return;
        };
        let title = tab.title.clone();
        let Some(first) = tab.sessions().first().copied() else {
            return;
        };
        let Some(session) = self.sessions.get(&first) else {
            return;
        };
        self.closed_tabs.push(ClosedTab {
            title,
            cwd: session.cwd.clone(),
            launch: session.launch.clone(),
        });
        // Keep only the most recent entries; drop the oldest past the cap.
        if self.closed_tabs.len() > MAX_CLOSED_TABS {
            self.closed_tabs.remove(0);
        }
    }

    /// Reopen the most recently closed tab (#78), relaunching it in the mode and
    /// directory it was closed in. Re-closing then reopening walks the stack in
    /// LIFO order. No effects when the stack is empty.
    fn reopen_closed_tab(&mut self) -> Vec<Effect> {
        let Some(closed) = self.closed_tabs.pop() else {
            return Vec::new();
        };
        self.launch(LaunchSpec {
            cwd: closed.cwd,
            launch: closed.launch,
            title: closed.title,
        })
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

    /// Decide whether an OSC 9 notification (#29) reaches the OS notification
    /// centre, and with what title/body. Only live sessions are worth alerting
    /// on — an unknown or exited session has nothing to return to, so it is
    /// dropped. The title is the session's tab label (what the user sees, and
    /// tracks OSC-24 renames); a blank body falls back to a default message.
    fn notify_session(&self, session: SessionId, body: String) -> Vec<Effect> {
        if !self.is_live(session) {
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
        // #76: a live tab's resume id resolves to the sidebar record, so the
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
    fn tab_record_resolves_a_resumed_tab_and_skips_shells_and_unknowns() {
        // #76: a tab resuming a scanned session maps back to its record; a shell
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
        // Regression guard for the number-row jump (issue #26): pressing ⌘5
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
        // #78: closing a Claude tab then reopening relaunches the same kind in
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
    fn reopen_with_nothing_closed_is_a_noop() {
        let mut app = App::new();
        assert!(app.apply(Event::ReopenClosedTab).is_empty());
        // Even after a launch with no close, there is nothing on the stack.
        launch(&mut app, "a");
        assert!(app.apply(Event::ReopenClosedTab).is_empty());
    }

    #[test]
    fn reopen_walks_the_close_stack_in_lifo_order() {
        // #78: closing A then B and reopening twice restores B first, then A.
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
        assert!(matches!(effects.as_slice(), [Effect::SaveMetadata(m)] if m["a"].starred));
        // Starred session now leads its group.
        let group = &app.visible_projects()[0];
        assert_eq!(group.sessions[0].session_id, "a");
        assert!(app.is_starred("a"));
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
        assert!(matches!(effects.as_slice(), [Effect::SaveMetadata(m)] if !m.contains_key("a")));
        assert_eq!(
            app.session_title(&app.projects[0].sessions[0].clone()),
            derived
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
        // title-carryover, #93), masking two different real first prompts.
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

    // ---- #29: OSC 9 notifications forwarded to the OS notification centre ----

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
        // the user, taken from the tab the user sees (#29).
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
        // Claude relabels the tab over OSC (#24); the notification title must
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
