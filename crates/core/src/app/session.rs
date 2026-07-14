//! Session lifecycle: launching, splitting, PTY exit, and the running-process
//! predicates the close/quit confirmations share. Also home to the
//! [`Sessions`] registry — the one owner of the live-session map and the id
//! source.

use std::collections::HashMap;
use std::num::NonZeroU64;

use crate::workspace::SplitDir;

use super::*;

/// Cell size a freshly launched PTY starts at, before the widget reports its
/// real geometry via [`Event::TerminalResized`].
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

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

/// How a launched Claude session reaches termherd's in-process MCP server: the
/// loopback url and the per-session bearer token. Opaque plain data — `core`
/// carries it from the adapter that mints it (the shell) to the adapter that
/// consumes it (the pty), holding no url/token of its own and doing no I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConfig {
    /// The loopback MCP server url an `mcpServers` entry points at.
    pub url: String,
    /// The per-session bearer token authorising this session against it.
    pub token: String,
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
    /// The live-bridge endpoint to inject as `mcpServers`, for a Claude launch.
    /// `core` always leaves this `None` — it has no server url or token; the
    /// shell adapter fills it in when performing the spawn (it owns the loopback
    /// endpoint and the token registry).
    pub mcp: Option<McpConfig>,
}

/// The live-session registry: the single owner of the map from runtime id to
/// [`LiveSession`] and the monotonic id source. It names the invariant three
/// clusters used to poke a raw `HashMap` for — *a live session is registered
/// here iff a pane hosts it* — so the terminal, tabs and pane clusters all go
/// through one seam. Ids are minted here, single-threaded, before any PTY
/// exists — the structural fix for the `realSessionId` race (Q6).
#[derive(Debug, Default)]
pub struct Sessions {
    map: HashMap<SessionId, LiveSession>,
    /// Monotonic id counter; never reused within a run.
    next_id: u64,
}

impl Sessions {
    /// Mint the next runtime session id. `None` only on u64 overflow (after
    /// ~1.8e19 launches) — surfaced as a silent no-op upstream, never a panic.
    pub(crate) fn allocate(&mut self) -> Option<SessionId> {
        self.next_id = self.next_id.checked_add(1)?;
        NonZeroU64::new(self.next_id).map(SessionId)
    }

    /// Register a live session under its own id.
    pub(crate) fn insert(&mut self, session: LiveSession) {
        self.map.insert(session.id, session);
    }

    /// Drop a session from the registry (its pane has gone).
    pub(crate) fn remove(&mut self, session: &SessionId) -> Option<LiveSession> {
        self.map.remove(session)
    }

    /// The live session for `id`, if registered.
    #[must_use]
    pub fn get(&self, session: &SessionId) -> Option<&LiveSession> {
        self.map.get(session)
    }

    /// Mutable access to the live session for `id`, if registered.
    pub(crate) fn get_mut(&mut self, session: &SessionId) -> Option<&mut LiveSession> {
        self.map.get_mut(session)
    }

    /// Whether `id` is registered (its pane exists), regardless of PTY status.
    #[must_use]
    pub fn contains_key(&self, session: &SessionId) -> bool {
        self.map.contains_key(session)
    }

    /// Every registered live session.
    pub fn values(&self) -> impl Iterator<Item = &LiveSession> {
        self.map.values()
    }

    /// How many sessions are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether no session is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl std::ops::Index<&SessionId> for Sessions {
    type Output = LiveSession;

    fn index(&self, session: &SessionId) -> &LiveSession {
        &self.map[session]
    }
}

impl App {
    /// Emit `effect` only while `session` is still live; a stale id (its
    /// terminal already closed) drops the effect, so a late input/resize/scroll
    /// can never act on a dead session.
    pub(super) fn if_live(&self, session: SessionId, effect: Effect) -> Vec<Effect> {
        if self.is_live(session) {
            vec![effect]
        } else {
            Vec::new()
        }
    }

    /// Register a launched session, open it as a tab, and ask the runtime to
    /// spawn its PTY. Returns no effects if id allocation overflows (after
    /// ~1.8e19 launches) — surfaced as a silent no-op, never a panic (Q5).
    pub(super) fn launch(&mut self, spec: LaunchSpec) -> Vec<Effect> {
        let Some(id) = self.sessions.allocate() else {
            return Vec::new();
        };
        self.sessions.insert(LiveSession {
            id,
            cwd: spec.cwd.clone(),
            launch: spec.launch.clone(),
            status: SessionStatus::Starting,
        });
        self.workspace.open(id, spec.title);
        vec![Effect::Spawn(SpawnSpec {
            session: id,
            cwd: spec.cwd,
            launch: spec.launch,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            mcp: None,
        })]
    }

    /// Split the focused pane (FR6): mint a session, inherit the focused pane's
    /// working directory, wrap the leaf into a split, and spawn the new PTY.
    /// Yields no effects on id overflow or if the focus is not on a leaf.
    pub(super) fn split_focused(&mut self, dir: SplitDir) -> Vec<Effect> {
        let Some(id) = self.sessions.allocate() else {
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
        self.sessions.insert(LiveSession {
            id,
            cwd: cwd.clone(),
            launch: Launch::Shell,
            status: SessionStatus::Starting,
        });
        vec![Effect::Spawn(SpawnSpec {
            session: id,
            cwd,
            launch: Launch::Shell,
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
            mcp: None,
        })]
    }

    /// A session's PTY ended. A *clean* exit — the user typed `exit` at a
    /// prompt — leaves nothing worth reading, so its pane closes on its own;
    /// an unclean exit keeps the dead terminal visible: a failure's last
    /// screen is worth reading. This applies to every launch kind: quitting
    /// Claude never raises this event (`claude` is *typed into* a shell, so
    /// its exit returns to the prompt with the PTY alive and the tab open —
    /// see `launch_command` in the `pty` adapter), which means a clean PTY
    /// exit on a Claude tab is that same shell `exit`, closed like any other.
    /// If launching ever `exec`s Claude directly, revisit: the CLI quitting
    /// would then end the PTY cleanly and auto-close a tab worth reviewing.
    pub(super) fn pty_exited(&mut self, session: SessionId, clean: bool) -> Vec<Effect> {
        if clean
            && self.sessions.contains_key(&session)
            && let Some(effects) = self.auto_close_pane(session)
        {
            return effects;
        }
        if let Some(s) = self.sessions.get_mut(&session) {
            s.status = SessionStatus::Exited;
        }
        Vec::new()
    }

    /// Close the pane hosting `session` after its clean exit: the whole tab
    /// (snapshotted onto the reopen stack, like a manual close) when it is the
    /// tab's only pane, else just its leaf, collapsing the split. The emptied
    /// workspace stays open — a clean exit never quits the app. The `Kill`
    /// still goes out for an already-dead process: it releases the adapter's
    /// PTY handles. `None` when no tab hosts the session — the caller falls
    /// back to recording the exit.
    pub(super) fn auto_close_pane(&mut self, session: SessionId) -> Option<Vec<Effect>> {
        let index = self.workspace.tab_of(session)?;
        let only_pane = self
            .workspace
            .tabs
            .get(index)
            .is_some_and(|tab| tab.sessions() == [session]);
        if only_pane {
            return Some(self.close_tab(index));
        }
        self.workspace.close_pane_of(session)?;
        self.sessions.remove(&session);
        Some(vec![Effect::Kill(session)])
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

    /// True if the session exists and its PTY has not exited.
    pub(super) fn is_live(&self, session: SessionId) -> bool {
        self.sessions
            .get(&session)
            .is_some_and(|s| s.status != SessionStatus::Exited)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::*;

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
        app.apply(Event::PtyExited {
            session: id,
            clean: false,
        });
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
    fn a_clean_shell_exit_closes_its_tab_and_kills_the_pty() {
        let mut app = App::new();
        let keep = launch(&mut app, "keep");
        let done = launch(&mut app, "done");
        let effects = app.apply(Event::PtyExited {
            session: done,
            clean: true,
        });
        // The kill releases the adapter's PTY handles for the dead process.
        assert!(matches!(effects.as_slice(), [Effect::Kill(k)] if *k == done));
        assert_eq!(app.workspace.tabs.len(), 1);
        assert!(
            !app.sessions.contains_key(&done),
            "the exited session is forgotten"
        );
        assert!(app.sessions.contains_key(&keep));
    }

    #[test]
    fn a_clean_shell_exit_of_the_last_tab_leaves_the_workspace_open_and_empty() {
        let mut app = App::new();
        let id = launch(&mut app, "only");
        let effects = app.apply(Event::PtyExited {
            session: id,
            clean: true,
        });
        assert!(matches!(effects.as_slice(), [Effect::Kill(k)] if *k == id));
        assert!(
            app.workspace.tabs.is_empty(),
            "the tab closes; the app stays"
        );
        assert!(app.sessions.is_empty());
    }

    #[test]
    fn a_clean_shell_exit_in_a_split_collapses_only_its_pane() {
        let mut app = App::new();
        let first = launch(&mut app, "a");
        let second = match app
            .apply(Event::SplitFocused(SplitDir::Vertical))
            .as_slice()
        {
            [Effect::Spawn(spec)] => spec.session,
            other => panic!("expected Spawn, got {other:?}"),
        };
        let effects = app.apply(Event::PtyExited {
            session: second,
            clean: true,
        });
        assert!(matches!(effects.as_slice(), [Effect::Kill(k)] if *k == second));
        assert_eq!(
            app.workspace.tabs.len(),
            1,
            "the sibling pane keeps the tab"
        );
        assert_eq!(app.workspace.tabs[0].sessions(), vec![first]);
        assert!(!app.sessions.contains_key(&second));
        assert_eq!(app.workspace.focused_session(), Some(first));
    }

    #[test]
    fn an_auto_closed_tab_lands_on_the_reopen_stack() {
        let mut app = App::new();
        let id = match app
            .apply(Event::LaunchSession(LaunchSpec {
                cwd: Some("/proj".into()),
                launch: Launch::Shell,
                title: "shell".into(),
            }))
            .as_slice()
        {
            [Effect::Spawn(spec)] => spec.session,
            other => panic!("expected Spawn, got {other:?}"),
        };
        app.apply(Event::PtyExited {
            session: id,
            clean: true,
        });
        // Reopen restores a shell in the directory the exited one ran in.
        match app.apply(Event::ReopenClosedTab).as_slice() {
            [Effect::Spawn(spec)] => {
                assert_eq!(spec.cwd.as_deref(), Some("/proj"));
                assert_eq!(spec.launch, Launch::Shell);
            }
            other => panic!("expected Spawn, got {other:?}"),
        }
    }

    #[test]
    fn a_dirty_shell_exit_keeps_the_dead_terminal_visible() {
        let mut app = App::new();
        let id = launch(&mut app, "crashed");
        let effects = app.apply(Event::PtyExited {
            session: id,
            clean: false,
        });
        assert!(effects.is_empty());
        assert_eq!(
            app.workspace.tabs.len(),
            1,
            "a failed exit's last screen stays readable"
        );
        assert_eq!(app.sessions[&id].status, SessionStatus::Exited);
    }

    #[test]
    fn a_clean_exit_of_a_claude_tabs_shell_closes_it_too() {
        // Quitting Claude never raises `PtyExited` — the CLI is typed into a
        // shell, so its exit returns to the prompt with the PTY alive (and the
        // tab open for review). A clean PTY exit on a Claude tab is therefore
        // the user typing `exit` at that prompt: close it like any shell.
        let mut app = App::new();
        let id = launch_claude(&mut app);
        let effects = app.apply(Event::PtyExited {
            session: id,
            clean: true,
        });
        assert!(matches!(effects.as_slice(), [Effect::Kill(k)] if *k == id));
        assert!(app.workspace.tabs.is_empty());
    }

    #[test]
    fn a_dirty_claude_exit_keeps_the_dead_terminal_visible() {
        let mut app = App::new();
        let id = launch_claude(&mut app);
        let effects = app.apply(Event::PtyExited {
            session: id,
            clean: false,
        });
        assert!(effects.is_empty());
        assert_eq!(app.workspace.tabs.len(), 1);
        assert_eq!(app.sessions[&id].status, SessionStatus::Exited);
    }

    #[test]
    fn an_exited_tab_has_no_running_process() {
        let mut app = App::new();
        let id = launch_claude(&mut app);
        assert!(app.tab_has_running_process(0));
        app.apply(Event::PtyExited {
            session: id,
            clean: false,
        });
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

        app.apply(Event::PtyExited {
            session: id,
            clean: false,
        });
        app.apply(Event::StatusChanged {
            session: id,
            status: SessionStatus::Idle,
        });
        assert_eq!(app.sessions[&id].status, SessionStatus::Exited);
    }
}
