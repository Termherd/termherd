//! Headless `App` — pure state machine over `Event`/`Effect`.
//!
//! The quality keystone (see `docs/ARCHITECTURE.md` §5). Events and effects
//! grow incrementally with each milestone. M2 adds the terminal lifecycle:
//! launching a session emits a [`Effect::Spawn`]; the runtime (the iced shell
//! plus the `pty` adapter) performs it and feeds bytes/status/exit back as
//! events. The grid itself lives in the adapter's per-session task — `core`
//! holds only the lifecycle and the derived activity status (FR8).
//!
//! `apply(Event) -> Vec<Effect>` is a flat dispatcher; each domain's helpers
//! and read models live in a submodule under `app/` (session, tabs, sidebar,
//! metadata, capture, record, settings, notify).

use std::collections::{HashMap, HashSet};

use crate::browser::{ProjectGroup, group_projects};
use crate::metadata::{RepoMeta, SessionMeta};
use crate::record::Recording;
use crate::workspace::{SessionId, Workspace};

mod capture;
mod effects;
mod events;
mod metadata;
mod notify;
mod record;
mod session;
mod settings;
mod sidebar;
mod tabs;
#[cfg(test)]
mod testsupport;

pub use effects::Effect;
pub use events::Event;
pub use session::{Launch, LaunchSpec, LiveSession, SessionStatus, SpawnSpec};
pub use settings::{DEFAULT_FONT_SIZE, Zoom};
pub use sidebar::SidebarFold;

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
    /// `MAX_CLOSED_TABS` so a long session can't grow it without bound;
    /// closing past the cap drops the oldest entry.
    closed_tabs: Vec<tabs::ClosedTab>,
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

impl App {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply an event, returning the effects the runtime must carry out.
    /// **Pure**: no I/O, no clock, no panic. A flat dispatcher: each arm is a
    /// one-line delegate to a domain helper, or an inline field update that
    /// yields no effect.
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
                self.if_live(session, Effect::Write { session, bytes })
            }
            Event::TerminalResized {
                session,
                cols,
                rows,
            } => self.if_live(
                session,
                Effect::Resize {
                    session,
                    cols,
                    rows,
                },
            ),
            Event::ScrollViewport { session, target } => {
                self.if_live(session, Effect::Scroll { session, target })
            }
            Event::Select { session, op } => self.if_live(session, Effect::Select { session, op }),
            Event::CopyTerminalSelection { session } => {
                self.if_live(session, Effect::CopyTerminalSelection { session })
            }
            Event::StatusChanged { session, status } => {
                if let Some(s) = self.sessions.get_mut(&session)
                    && s.status != SessionStatus::Exited
                {
                    s.status = status;
                }
                Vec::new()
            }
            Event::PtyExited { session, clean } => self.pty_exited(session, clean),
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
            Event::RenameSession { session, title } => self.rename_session(session, title),
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
            Event::OpenUrl(url) => Self::open_url(url),
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

    /// Open a Ctrl/Cmd+clicked link in the OS default handler. Only a
    /// non-blank string reaches the handler; a blank or schemeless one is
    /// dropped rather than shelling out on it.
    fn open_url(url: String) -> Vec<Effect> {
        let url = url.trim();
        if url.is_empty() {
            Vec::new()
        } else {
            vec![Effect::OpenUrl(url.to_owned())]
        }
    }

    /// Whether the OS reports the window focused, so the view can dim an
    /// unfocused window. Starts `false` (unknown) until the first real focus
    /// signal — the same convention notification suppression relies on.
    #[must_use]
    pub const fn window_focused(&self) -> bool {
        self.window_focused
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::*;

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
        ];
        for event in effect_free {
            let label = format!("{event:?}");
            assert!(
                app.apply(event).is_empty(),
                "{label} must stay effect-free (the shell routes it through perform)"
            );
        }
    }
}
