//! Headless `App` — pure state machine over `Event`/`Effect`.
//!
//! The quality keystone (see `docs/ARCHITECTURE.md` §5). Events and effects
//! grow incrementally with each milestone. M2 adds the terminal lifecycle:
//! launching a session emits a [`Effect::Spawn`]; the runtime (the iced shell
//! plus the `pty` adapter) performs it and feeds bytes/status/exit back as
//! events. The grid itself lives in the adapter's per-session task — `core`
//! holds only the lifecycle and the derived activity status (FR8).

use std::collections::HashMap;
use std::num::NonZeroU64;

use crate::browser::{ProjectGroup, SessionRecord, filter_projects, group_projects};
use crate::workspace::{SessionId, Workspace};

/// Cell size a freshly launched PTY starts at, before the widget reports its
/// real geometry via [`Event::TerminalResized`].
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

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
    /// Monotonic source of `SessionId`s; never reused within a run. This is
    /// the structural fix for the `realSessionId` race (Q6) — ids are minted
    /// here, single-threaded, before any PTY exists.
    next_session: u64,
}

/// A terminal session the app is hosting. The PTY handle and terminal grid
/// live in the adapter's task, not here; this is just the lifecycle record.
#[derive(Debug, Clone)]
pub struct LiveSession {
    pub id: SessionId,
    /// Real project path the PTY runs in, if known.
    pub cwd: Option<String>,
    /// Claude session id this terminal resumed, if any — lets the sidebar map
    /// a browsed session row to its live activity (FR8).
    pub resume: Option<String>,
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

/// What the user asked to open (FR4): a terminal in `cwd`, optionally
/// resuming an existing Claude session.
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    /// Working directory for the new terminal (the real project path).
    pub cwd: Option<String>,
    /// Claude session id to resume/reattach, if any.
    pub resume: Option<String>,
    /// Tab title to show.
    pub title: String,
}

/// A spawn request handed to the `pty` adapter. The runtime id is already
/// allocated, so the adapter never invents one.
#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub session: SessionId,
    pub cwd: Option<String>,
    pub resume: Option<String>,
    pub cols: u16,
    pub rows: u16,
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
    TerminalInput { session: SessionId, bytes: Vec<u8> },
    /// A terminal pane changed size (in cells); propagate to the PTY (FR4).
    TerminalResized {
        session: SessionId,
        cols: u16,
        rows: u16,
    },
    /// The user scrolled a terminal's viewport (FR4 scrollback).
    TerminalScrolled { session: SessionId, delta: i32 },
    /// The OSC decoder reclassified a session's activity (FR8).
    StatusChanged {
        session: SessionId,
        status: SessionStatus,
    },
    /// A session's PTY process exited.
    PtyExited(SessionId),
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
    /// Scroll a session's viewport by a line delta (positive = into history).
    Scroll { session: SessionId, delta: i32 },
    /// Terminate a session's PTY process.
    Kill(SessionId),
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
            Event::TerminalScrolled { session, delta } => {
                if self.is_live(session) {
                    vec![Effect::Scroll { session, delta }]
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
        }
    }

    /// The sidebar's view of the projects: everything, or the search
    /// matches when a query is active (FR3).
    #[must_use]
    pub fn visible_projects(&self) -> Vec<ProjectGroup> {
        filter_projects(&self.projects, &self.search, self.search_titles_only)
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
                resume: spec.resume.clone(),
                status: SessionStatus::Starting,
            },
        );
        self.workspace.open(id, spec.title);
        vec![Effect::Spawn(SpawnSpec {
            session: id,
            cwd: spec.cwd,
            resume: spec.resume,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
        })]
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
            resume: None,
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
            resume: Some("abc-123".into()),
            title: "proj".into(),
        }));
        let id = app.workspace.focused_session().expect("a focused session");
        assert_eq!(app.sessions[&id].resume.as_deref(), Some("abc-123"));
    }

    #[test]
    fn each_launch_gets_a_distinct_id() {
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: None,
            resume: None,
            title: "a".into(),
        }));
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: None,
            resume: None,
            title: "b".into(),
        }));
        assert_eq!(app.sessions.len(), 2);
    }

    #[test]
    fn input_and_resize_target_only_live_sessions() {
        let mut app = App::new();
        let spawn = app.apply(Event::LaunchSession(LaunchSpec {
            cwd: None,
            resume: None,
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

    #[test]
    fn status_changes_are_recorded_but_never_revive_an_exited_session() {
        let mut app = App::new();
        let spawn = app.apply(Event::LaunchSession(LaunchSpec {
            cwd: None,
            resume: None,
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
}
