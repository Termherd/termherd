//! Session lifecycle: launching, splitting, PTY exit, and the running-process
//! predicates the close/quit confirmations share. Also home to the
//! [`Sessions`] registry — the one owner of the live-session map and the id
//! source.

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::time::SystemTime;

use super::snapshot::{identity_of, proves};
use crate::snapshot::SessionKind;
use crate::workspace::SplitDir;
use termherd_claude::session_file::SessionFile;

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
    /// The directory the PTY is in, if known: the one it was launched in until
    /// the shell announces otherwise ([`Event::SessionCwdChanged`]), so every
    /// reader sees where the session *is* rather than where it started.
    pub cwd: Option<String>,
    /// The directory the PTY was *launched* in, never rewritten. `cwd` moves
    /// with every `cd`, so after one it is the only remaining trace of the
    /// project a session belongs to — which is the outermost root a path
    /// printed in this terminal could be relative to.
    pub launch_cwd: Option<String>,
    /// What this terminal is running — a shell or a (possibly resumed) Claude
    /// session. The resumed-id lets the sidebar map a browsed session row to its
    /// live activity (FR8); read it via [`Launch::resume_id`].
    pub launch: Launch,
    /// Activity derived from the OSC stream (FR8).
    pub status: SessionStatus,
    /// The job in front of the shell, as the PTY adapter last reported it
    /// ([`Event::ForegroundJobChanged`]). `None` at the prompt and wherever the
    /// platform has no foreground process group (ConPTY).
    pub foreground: Option<ForegroundJob>,
    /// Claude's session file for the job in front, as the shell last read it
    /// ([`Event::SessionFileRead`]). Only a cache: whether it names this job
    /// is decided on every read, so a stale one names nobody.
    pub session_file: Option<SessionFile>,
    /// When the shell spawned this pane's PTY ([`Event::SessionSpawned`]).
    /// The shell's clock, not core's: core only keeps the stamp.
    pub spawned_at: Option<SystemTime>,
}

/// The job in front of a session's shell, as the PTY adapter reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundJob {
    /// Its process id, the one a Claude session file is named after.
    pub pid: u32,
    /// When that process started, in the form Claude Code stamps its session
    /// file with (`procStart`). `None` when the adapter could not tell.
    pub started: Option<String>,
}

impl LiveSession {
    /// The Claude session id this pane's conversation lives under — the one
    /// accessor every reader of "this pane's transcript" goes through.
    ///
    /// The cached session file wins when it proves the Claude in front (see
    /// [`identity_of`]): Claude rewrites it when `/clear` or a plan-accept
    /// re-keys the conversation, which the launch line cannot follow. Without
    /// that proof, the id the pane was launched under (minted or resumed). The
    /// cache is only as fresh as the shell's last read of the file.
    #[must_use]
    pub fn claude_session_id(&self) -> Option<&str> {
        session_id_by_precedence(self.live_session_id(), self.launch.claude_id())
    }

    /// The session id the cached session file names, if it proves the job in
    /// front of this pane is the Claude that wrote it.
    fn live_session_id(&self) -> Option<&str> {
        self.proven_session_file()?.session_id.as_deref()
    }

    /// The Claude Code version the cached session file states, if it proves
    /// the job in front of this pane is the Claude that wrote it.
    fn live_version(&self) -> Option<&str> {
        self.proven_session_file()?.version.as_deref()
    }

    /// The cached session file, only when it proves the job in front of this
    /// pane is the Claude that wrote it.
    fn proven_session_file(&self) -> Option<&SessionFile> {
        let job = self.foreground.as_ref()?;
        let file = self.session_file.as_ref()?;
        proves(job, file).then_some(file)
    }

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
                Launch::Claude(_) => true,
                Launch::Shell => {
                    matches!(self.status, SessionStatus::Busy | SessionStatus::Attention)
                }
            },
        }
    }
}

/// Which id names a pane's conversation, given the one Claude's live session
/// file states and the one the pane was launched under: the live one, since
/// only it follows a re-key.
fn session_id_by_precedence<'a>(
    live: Option<&'a str>,
    launched: Option<&'a str>,
) -> Option<&'a str> {
    live.or(launched)
}

/// Which Claude Code version a pane runs, given the one its live session file
/// states and the one its transcript last recorded: the live one, since the
/// transcript is only as fresh as the last scan and a fresh pane has none.
fn version_by_precedence<'a>(
    live: Option<&'a str>,
    transcript: Option<&'a str>,
) -> Option<&'a str> {
    live.or(transcript)
}

/// Per-session activity surfaced in the sidebar and on tabs (FR8).
///
/// Derived in the `pty` adapter from whichever source the terminal offers: a
/// Claude session's own OSC stream (`termherd_claude::osc`), a plain shell's
/// OSC 133 shell-integration marks, or — where neither speaks — the PTY's
/// foreground process group. `core` only records the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    /// Spawned; no activity classified yet.
    Starting,
    /// Work is running: Claude's spinner, or a command the shell reported.
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
/// `Claude` starts the CLI, as a [`ClaudeLaunch`] says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// A plain login shell in the working directory.
    Shell,
    /// A Claude session, fresh or resumed.
    Claude(ClaudeLaunch),
}

/// How a Claude launch starts its conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeLaunch {
    /// A new conversation. `Some` is the id the shell minted for it
    /// (`claude --session-id <id>`), so its transcript is known before Claude
    /// has written anything; `None` leaves Claude to pick one, which only its
    /// session file then reveals. `core` never mints one itself: it holds no
    /// source of randomness.
    Fresh(Option<String>),
    /// An existing conversation, resumed by its id (`claude --resume <id>`).
    Resume(String),
}

impl Launch {
    /// The Claude session id this launch resumes, if any — `None` for a shell
    /// or a fresh Claude session, minted id or not. The id a pane's transcript
    /// lives under is [`LiveSession::claude_session_id`].
    #[must_use]
    pub fn resume_id(&self) -> Option<&str> {
        match self {
            Launch::Claude(ClaudeLaunch::Resume(id)) => Some(id),
            _ => None,
        }
    }

    /// The Claude session id this launch starts under: the one it resumes, or
    /// the one minted for a fresh conversation. `None` for a shell, or a fresh
    /// launch nobody minted an id for.
    #[must_use]
    pub fn claude_id(&self) -> Option<&str> {
        match self {
            Launch::Claude(ClaudeLaunch::Resume(id) | ClaudeLaunch::Fresh(Some(id))) => Some(id),
            _ => None,
        }
    }

    /// This launch with a fresh Claude conversation started under `id` in
    /// place of any id it carried; a shell or a resume is returned as is. A
    /// minted id names one conversation only: a second Claude launched under
    /// it would collide with the first one's transcript.
    #[must_use]
    pub fn with_fresh_id(self, id: String) -> Self {
        match self {
            Launch::Claude(ClaudeLaunch::Fresh(_)) => Launch::Claude(ClaudeLaunch::Fresh(Some(id))),
            other => other,
        }
    }

    /// The program kind this launch runs, without the resume id.
    #[must_use]
    pub fn kind(&self) -> SessionKind {
        match self {
            Launch::Shell => SessionKind::Shell,
            Launch::Claude(_) => SessionKind::Claude,
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
            launch_cwd: spec.cwd.clone(),
            launch: spec.launch.clone(),
            status: SessionStatus::Starting,
            foreground: None,
            session_file: None,
            spawned_at: None,
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
            launch_cwd: cwd.clone(),
            launch: Launch::Shell,
            status: SessionStatus::Starting,
            foreground: None,
            session_file: None,
            spawned_at: None,
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

    /// Record `session`'s new activity. An exited session stays exited: a late
    /// report from its dying terminal must not revive it.
    pub(super) fn status_changed(
        &mut self,
        session: SessionId,
        status: SessionStatus,
    ) -> Vec<Effect> {
        if let Some(s) = self.sessions.get_mut(&session)
            && s.status != SessionStatus::Exited
        {
            s.status = status;
        }
        Vec::new()
    }

    /// Record the job now in front of `session`'s shell. Unknown sessions are
    /// ignored, like every other adapter report about one.
    pub(super) fn foreground_job_changed(
        &mut self,
        session: SessionId,
        job: Option<ForegroundJob>,
    ) -> Vec<Effect> {
        if let Some(live) = self.sessions.get_mut(&session) {
            live.foreground = job;
        }
        Vec::new()
    }

    /// Cache the session file the shell just read for `session`'s job, or its
    /// absence. Unknown sessions are ignored.
    pub(super) fn session_file_read(
        &mut self,
        session: SessionId,
        file: Option<SessionFile>,
    ) -> Vec<Effect> {
        if let Some(live) = self.sessions.get_mut(&session) {
            live.session_file = file;
        }
        Vec::new()
    }

    /// The peer name of the Claude in front of `session`, from the cached
    /// session file, by the same proof the snapshot applies.
    #[must_use]
    pub fn peer_name(&self, session: SessionId) -> Option<String> {
        let live = self.sessions.get(&session)?;
        identity_of(live.foreground.as_ref(), live.session_file.as_ref()).peer_name
    }

    /// Stamp `session` with the moment the shell spawned its PTY. Unknown
    /// sessions are ignored.
    pub(super) fn session_spawned(&mut self, session: SessionId, at: SystemTime) -> Vec<Effect> {
        if let Some(live) = self.sessions.get_mut(&session) {
            live.spawned_at = Some(at);
        }
        Vec::new()
    }

    /// When `session`'s PTY was spawned, while it still runs: `None` for an
    /// unknown or exited session, or one the shell never stamped.
    #[must_use]
    pub fn running_since(&self, session: SessionId) -> Option<SystemTime> {
        let live = self.sessions.get(&session)?;
        if live.status == SessionStatus::Exited {
            return None;
        }
        live.spawned_at
    }

    /// The Claude Code version running in `session`: the one its proven
    /// session file states, else the one its transcript last recorded.
    #[must_use]
    pub fn claude_version(&self, session: SessionId) -> Option<&str> {
        let live = self.sessions.get(&session)?;
        let transcript = live
            .claude_session_id()
            .and_then(|id| self.record_for(id))
            .and_then(|record| record.digest.version.as_deref());
        version_by_precedence(live.live_version(), transcript)
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
            // The watcher ends with the PTY and never reports the job gone,
            // and a dead job's pid may be reused by another Claude.
            s.foreground = None;
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
    fn a_pointer_reaches_a_live_terminal_and_is_absorbed_for_a_dead_one() {
        use crate::app::{PointerEvent, PointerKind};
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Shell,
            title: "proj".into(),
        }));
        let id = app.workspace.focused_session().expect("a focused session");
        let pointer = PointerEvent::left(PointerKind::Press, 3, 1);
        match app
            .apply(Event::TerminalPointer {
                session: id,
                pointer,
            })
            .as_slice()
        {
            [
                Effect::TerminalPointer {
                    session,
                    pointer: forwarded,
                },
            ] => {
                assert_eq!(*session, id);
                assert_eq!(*forwarded, pointer);
            }
            other => panic!("expected one TerminalPointer effect, got {other:?}"),
        }
        // A handle nothing owns is absorbed, as every per-session event is.
        let ghost = SessionId(std::num::NonZeroU64::new(999).expect("non-zero"));
        assert!(
            app.apply(Event::TerminalPointer {
                session: ghost,
                pointer,
            })
            .is_empty()
        );
    }

    #[test]
    fn launching_a_resume_records_its_claude_id() {
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Claude(ClaudeLaunch::Resume("abc-123".into())),
            title: "proj".into(),
        }));
        let id = app.workspace.focused_session().expect("a focused session");
        assert_eq!(app.sessions[&id].launch.resume_id(), Some("abc-123"));
    }

    const MINTED: &str = "0b9f2c4e-7d1a-4e8b-9c3f-5a6d7e8f9012";
    const STARTED: &str = "Wed Oct  7 06:48:07 2026";

    fn launch_spec(launch: Launch) -> LaunchSpec {
        LaunchSpec {
            cwd: Some("/proj".into()),
            launch,
            title: "proj".into(),
        }
    }

    fn fresh(id: Option<&str>) -> Launch {
        Launch::Claude(ClaudeLaunch::Fresh(id.map(str::to_owned)))
    }

    fn live(launch: Launch, job: Option<ForegroundJob>, file: Option<SessionFile>) -> LiveSession {
        LiveSession {
            id: SessionId(NonZeroU64::new(1).expect("nonzero")),
            cwd: None,
            launch_cwd: None,
            launch,
            status: SessionStatus::Idle,
            foreground: job,
            session_file: file,
            spawned_at: None,
        }
    }

    fn claude_job(pid: u32) -> ForegroundJob {
        ForegroundJob {
            pid,
            started: Some(STARTED.to_owned()),
        }
    }

    fn file_naming(pid: u32, session_id: &str) -> SessionFile {
        SessionFile {
            pid,
            name: None,
            session_id: Some(session_id.to_owned()),
            proc_start: Some(STARTED.to_owned()),
            version: None,
        }
    }

    #[test]
    fn a_launch_names_its_claude_id_whether_minted_or_resumed() {
        assert_eq!(fresh(Some(MINTED)).claude_id(), Some(MINTED));
        assert_eq!(
            fresh(Some(MINTED)).resume_id(),
            None,
            "a minted id is no resume"
        );
        assert_eq!(
            Launch::Claude(ClaudeLaunch::Resume("abc".into())).claude_id(),
            Some("abc")
        );
        assert_eq!(fresh(None).claude_id(), None);
        assert_eq!(Launch::Shell.claude_id(), None);
    }

    #[test]
    fn a_fresh_id_replaces_only_a_fresh_claude_launch() {
        assert_eq!(
            fresh(None).with_fresh_id(MINTED.into()),
            fresh(Some(MINTED))
        );
        assert_eq!(
            fresh(Some("old")).with_fresh_id(MINTED.into()),
            fresh(Some(MINTED)),
            "a minted id is never launched twice"
        );
        let resume = Launch::Claude(ClaudeLaunch::Resume("abc".into()));
        assert_eq!(resume.clone().with_fresh_id(MINTED.into()), resume);
        assert_eq!(Launch::Shell.with_fresh_id(MINTED.into()), Launch::Shell);
    }

    #[test]
    fn a_fresh_tab_launched_under_a_minted_id_is_known_by_it_at_once() {
        let mut app = App::new();
        app.apply(Event::LaunchSession(launch_spec(fresh(Some(MINTED)))));
        let id = app.workspace.focused_session().expect("a focused session");
        assert_eq!(app.claude_session_id(id), Some(MINTED));
        assert_eq!(app.tab_claude_session_id(0), Some(MINTED));
        assert_eq!(
            app.open_session_for(MINTED),
            Some(id),
            "re-clicking its sidebar row must find the open tab"
        );
    }

    #[test]
    fn a_shell_and_an_unminted_fresh_claude_have_no_session_id() {
        let mut app = App::new();
        app.apply(Event::LaunchSession(launch_spec(Launch::Shell)));
        app.apply(Event::LaunchSession(launch_spec(fresh(None))));
        assert_eq!(app.tab_claude_session_id(0), None);
        assert_eq!(app.tab_claude_session_id(1), None);
        assert_eq!(app.tab_claude_session_id(2), None, "out of range");
    }

    #[test]
    fn the_live_session_file_outranks_the_launch_id_after_a_re_key() {
        let session = live(
            fresh(Some(MINTED)),
            Some(claude_job(42)),
            Some(file_naming(42, "re-keyed")),
        );
        assert_eq!(session.claude_session_id(), Some("re-keyed"));
    }

    #[test]
    fn an_unproven_session_file_never_outranks_the_launch_id() {
        let stale = SessionFile {
            proc_start: Some("another process".into()),
            version: None,
            ..file_naming(42, "stale")
        };
        let session = live(fresh(Some(MINTED)), Some(claude_job(42)), Some(stale));
        assert_eq!(session.claude_session_id(), Some(MINTED));
        let gone = live(fresh(Some(MINTED)), None, Some(file_naming(42, "gone")));
        assert_eq!(
            gone.claude_session_id(),
            Some(MINTED),
            "no job in front, so the cached file proves nothing"
        );
    }

    #[test]
    fn a_shell_pane_running_claude_by_hand_is_known_by_its_session_file() {
        let session = live(
            Launch::Shell,
            Some(claude_job(7)),
            Some(file_naming(7, "typed")),
        );
        assert_eq!(session.claude_session_id(), Some("typed"));
    }

    proptest::proptest! {
        #[test]
        fn a_proven_file_id_wins_else_the_launch_id(
            launched in proptest::option::of("[a-z]{1,8}"),
            resumed in proptest::bool::ANY,
            file_id in proptest::option::of("[A-Z]{1,8}"),
            job_pid in 1u32..4,
            file_pid in 1u32..4,
            same_start in proptest::bool::ANY,
        ) {
            let launch = match (&launched, resumed) {
                (Some(id), true) => Launch::Claude(ClaudeLaunch::Resume(id.clone())),
                (id, _) => fresh(id.as_deref()),
            };
            let file = SessionFile {
                pid: file_pid,
                name: None,
                session_id: file_id.clone(),
                proc_start: Some(if same_start { STARTED.into() } else { "other".into() }),
                version: None,
            };
            let session = live(launch, Some(claude_job(job_pid)), Some(file));
            let proven = job_pid == file_pid && same_start;
            let expected = file_id.as_deref().filter(|_| proven).or(launched.as_deref());
            proptest::prop_assert_eq!(session.claude_session_id(), expected);
        }
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
    fn a_shell_that_announces_a_directory_moves_the_session_into_it() {
        // The launch directory is what the session *started* in; every reader
        // of `cwd` — the MCP snapshot, the capture dump, a split, "new shell
        // here" — asks where it *is*. One field, replaced in place.
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Shell,
            title: "proj".into(),
        }));
        let id = app.workspace.focused_session().expect("a focused session");
        let effects = app.apply(Event::SessionCwdChanged {
            session: id,
            cwd: "/proj/crates/pty".into(),
        });
        assert_eq!(app.sessions[&id].cwd.as_deref(), Some("/proj/crates/pty"));
        assert!(
            effects.is_empty(),
            "recording where a shell is drives no I/O, got {effects:?}"
        );
    }

    #[test]
    fn a_session_launched_without_a_directory_learns_one_from_its_shell() {
        // A shell opened with no known directory reports `None` to every
        // reader; its own announcement is the only thing that can fill it.
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: None,
            launch: Launch::Shell,
            title: "a".into(),
        }));
        let id = app.workspace.focused_session().expect("a focused session");
        app.apply(Event::SessionCwdChanged {
            session: id,
            cwd: "/tmp".into(),
        });
        assert_eq!(app.sessions[&id].cwd.as_deref(), Some("/tmp"));
    }

    #[test]
    fn an_announcement_from_a_session_that_is_gone_changes_nothing() {
        // The PTY thread and the core are not in lockstep: a chunk decoded just
        // before a pane closed still arrives afterwards.
        let mut app = App::new();
        let id = launch(&mut app, "a");
        app.apply(Event::CloseFocusedPane);
        let effects = app.apply(Event::SessionCwdChanged {
            session: id,
            cwd: "/tmp".into(),
        });
        assert!(effects.is_empty());
        assert!(!app.sessions.contains_key(&id));
    }

    #[test]
    fn a_split_inherits_the_directory_the_shell_moved_to() {
        // The inheritance reads `Session.cwd`, so following a `cd` is what
        // makes "split here" land where the user actually is.
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Shell,
            title: "proj".into(),
        }));
        let id = app.workspace.focused_session().expect("a focused session");
        app.apply(Event::SessionCwdChanged {
            session: id,
            cwd: "/proj/docs".into(),
        });
        let effects = app.apply(Event::SplitFocused(SplitDir::Vertical));
        let new = app.workspace.focused_session().expect("focused pane");
        assert_eq!(app.sessions[&new].cwd.as_deref(), Some("/proj/docs"));
        match effects.as_slice() {
            [Effect::Spawn(spec)] => assert_eq!(spec.cwd.as_deref(), Some("/proj/docs")),
            other => panic!("expected one Spawn, got {other:?}"),
        }
    }

    #[test]
    fn a_cd_moves_the_session_without_renaming_the_tab() {
        // The tab is what the user opened — a project, a session. Following the
        // shell into a subdirectory must not relabel it, or a build that cds
        // around would rewrite the workspace under the user's eyes.
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Shell,
            title: "proj".into(),
        }));
        let id = app.workspace.focused_session().expect("a focused session");
        app.apply(Event::SessionCwdChanged {
            session: id,
            cwd: "/proj/crates/core".into(),
        });
        assert_eq!(
            app.workspace.tabs[0].display_title(),
            "proj",
            "a `cd` moves the session, not the tab it lives in"
        );
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
        match app
            .apply(Event::ReopenClosedTab {
                fresh_claude_id: "minted".into(),
            })
            .as_slice()
        {
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

    #[test]
    fn a_spawn_stamp_counts_while_the_session_runs_and_not_after() {
        let mut app = App::new();
        let id = launch(&mut app, "sh");
        let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        assert_eq!(app.running_since(id), None, "nothing stamped yet");
        assert!(
            app.apply(Event::SessionSpawned { session: id, at })
                .is_empty()
        );
        assert_eq!(app.running_since(id), Some(at));
        app.apply(Event::PtyExited {
            session: id,
            clean: false,
        });
        assert_eq!(app.running_since(id), None, "a dead terminal runs no more");
    }

    /// A fresh Claude pane under `MINTED` whose transcript the last scan
    /// found, written by Claude Code `transcript_version`.
    fn claude_pane_with_transcript(transcript_version: Option<&str>) -> (App, SessionId) {
        let mut app = App::new();
        let mut scanned = record(MINTED, "/proj", "a prompt");
        scanned.digest.version = transcript_version.map(str::to_owned);
        app.apply(Event::ScanCompleted(vec![scanned]));
        app.apply(Event::LaunchSession(launch_spec(fresh(Some(MINTED)))));
        let id = app.workspace.focused_session().expect("a focused session");
        (app, id)
    }

    fn read_file(app: &mut App, id: SessionId, started: &str, version: &str) {
        app.apply(Event::ForegroundJobChanged {
            session: id,
            job: Some(claude_job(4399)),
        });
        let file = SessionFile {
            proc_start: Some(started.to_owned()),
            version: Some(version.to_owned()),
            ..file_naming(4399, MINTED)
        };
        app.apply(Event::SessionFileRead {
            session: id,
            file: Some(file),
        });
    }

    #[test]
    fn the_version_comes_from_the_transcript_until_a_session_file_proves_otherwise() {
        let (mut app, id) = claude_pane_with_transcript(Some("2.1.290"));
        assert_eq!(app.claude_version(id), Some("2.1.290"));
        read_file(&mut app, id, STARTED, "2.1.294");
        assert_eq!(
            app.claude_version(id),
            Some("2.1.294"),
            "the running Claude outranks what its transcript last recorded"
        );
        read_file(&mut app, id, "a later process", "9.9.9");
        assert_eq!(
            app.claude_version(id),
            Some("2.1.290"),
            "a file the job in front did not write names nothing"
        );
    }

    #[test]
    fn a_pane_with_no_transcript_yet_takes_its_version_from_the_session_file() {
        let mut app = App::new();
        app.apply(Event::LaunchSession(launch_spec(fresh(Some(MINTED)))));
        let id = app.workspace.focused_session().expect("a focused session");
        assert_eq!(app.claude_version(id), None);
        read_file(&mut app, id, STARTED, "2.1.294");
        assert_eq!(app.claude_version(id), Some("2.1.294"));
    }
}
