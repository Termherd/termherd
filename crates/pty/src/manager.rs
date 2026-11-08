//! The PTY host: [`PtyManager`] owns every live session and implements
//! [`termherd_core::ports::PtyHost`]. `spawn` wires a session's three threads
//! (reader / waiter / terminal, see [`crate::session`]) and the control methods
//! forward user actions to it over the command channel. Construct once in
//! `main()` and inject as a `dyn PtyHost` (no global state, AGENTS.md quality
//! bar).

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError, RwLock, mpsc};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use termherd_core::ports::{PtyError, PtyHost};
use termherd_core::workspace::SessionId;
use termherd_core::{PointerEvent, ScrollTarget, SelectOp, SpawnSpec};

use crate::events::EventSink;
use crate::grid::Palette;
use crate::kill::finish_kill;
use crate::launch::{
    PrivateFiles, apply_integration, apply_terminal_env, discard_private_files, launch_command,
    strip_inherited_session_env, write_mcp_config, write_title_settings,
};
use crate::session::{
    SCREEN_INTERVAL, Session, SharedMaster, SharedPalette, SharedWriter, TermCmd, spawn_reader,
    spawn_term, spawn_waiter, spawn_watcher,
};

/// How to launch a session's shell process (FR10). Built from the user's
/// settings and injected into [`PtyManager`]; `None` uses the platform default
/// login shell. Kept a plain type so the adapter never depends on serde.
#[derive(Debug, Clone)]
pub struct Shell {
    /// The program to run, e.g. `pwsh` or `bash`.
    pub program: String,
    /// Arguments passed to the program.
    pub args: Vec<String>,
}

/// Hosts every live session's PTY. Construct once in `main()` and inject as a
/// `dyn PtyHost` (no global state, AGENTS.md quality bar).
pub struct PtyManager {
    sessions: Mutex<HashMap<SessionId, Session>>,
    sink: EventSink,
    /// User-configured shell; `None` falls back to the platform default.
    /// Read when a session spawns, so [`PtyManager::set_shell`] reaches the
    /// next session and leaves the running ones alone.
    shell: RwLock<Option<Shell>>,
    /// The terminal colour scheme every session renders with, shared with
    /// each terminal thread so [`PtyManager::set_palette`] reaches them live.
    palette: SharedPalette,
    /// Where each session's private files are written, and removed from.
    private: PrivateFiles,
}

impl PtyManager {
    /// `sink` receives every session's output and exit, from the reader
    /// threads. Wrap a channel sender to bridge into the GUI subscription.
    /// `shell` is the configured shell to launch, or `None` for the platform
    /// default; `palette` the terminal colour scheme (FR10); `private` where
    /// each session's private files go.
    pub fn new(
        sink: EventSink,
        shell: Option<Shell>,
        palette: Palette,
        private: PrivateFiles,
    ) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            sink,
            shell: RwLock::new(shell),
            palette: Arc::new(RwLock::new(palette)),
            private,
        }
    }

    /// Launch `shell` (or the platform default for `None`) from the next
    /// spawned session on. A running session keeps the program it started.
    pub fn set_shell(&self, shell: Option<Shell>) {
        *self.shell.write().unwrap_or_else(PoisonError::into_inner) = shell;
    }

    /// Switch every session — running and future — to `palette`. Each live
    /// terminal re-emits its screen in the new colours; a later colour query
    /// (OSC 10/11) is answered from it too. A program that asked once at start
    /// (Claude Code picks light or dark that way) keeps its choice until it
    /// asks again.
    pub fn set_palette(&self, palette: Palette) {
        *self.palette.write().unwrap_or_else(PoisonError::into_inner) = palette;
        if let Ok(map) = self.sessions.lock() {
            for session in map.values() {
                let _ = session.ctrl.send(TermCmd::Repaint);
            }
        }
    }

    /// Number of live sessions — for tests and diagnostics.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sessions.lock().map(|s| s.len()).unwrap_or(0)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl PtyManager {
    /// Hand a command to a session's terminal thread.
    fn send(&self, session: SessionId, cmd: TermCmd) -> Result<(), PtyError> {
        let map = self
            .sessions
            .lock()
            .map_err(|_| PtyError::Io("session lock poisoned".into()))?;
        let s = map
            .get(&session)
            .ok_or(PtyError::NoSuchSession(session.0.get()))?;
        s.ctrl
            .send(cmd)
            .map_err(|_| PtyError::Io("terminal thread gone".into()))
    }
}

/// The command a session's shell is spawned from, before any per-session
/// integration: the configured shell (or the platform default), in `cwd`, with
/// the environment every session gets. Free of the PTY, so the spawn
/// environment is unit-testable.
fn shell_command(configured: Option<&Shell>, cwd: Option<&str>) -> CommandBuilder {
    let mut cmd = match configured {
        Some(shell) => {
            let mut c = CommandBuilder::new(&shell.program);
            for arg in &shell.args {
                c.arg(arg);
            }
            c
        }
        None => CommandBuilder::new_default_prog(),
    };
    if let Some(cwd) = cwd {
        cmd.cwd(cwd);
    }
    strip_inherited_session_env(&mut cmd);
    apply_terminal_env(&mut cmd);
    cmd
}

impl PtyHost for PtyManager {
    fn spawn(&self, spec: SpawnSpec) -> Result<(), PtyError> {
        let size = PtySize {
            rows: spec.rows,
            cols: spec.cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|e| PtyError::Spawn(e.to_string()))?;

        // The configured shell, else the platform default login shell, in the
        // project directory with a sane TERM. Resuming a real Claude session
        // lands in a later slice; the id flows through so the adapter never has
        // to invent one (Q6).
        let configured = self
            .shell
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut cmd = shell_command(configured.as_ref(), spec.cwd.as_deref());
        // Which shell is about to run decides the integration recipe; the
        // default program only resolves to a name here, so ask the builder.
        let program = match &configured {
            Some(shell) => shell.program.clone(),
            None => cmd.get_shell(),
        };
        apply_integration(&self.private, spec.session, &program, &mut cmd);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::Spawn(e.to_string()))?;
        // The slave fd must close in this process so the reader sees EOF when
        // the child exits.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Spawn(e.to_string()))?;
        let writer: SharedWriter = Arc::new(Mutex::new(
            pair.master
                .take_writer()
                .map_err(|e| PtyError::Spawn(e.to_string()))?,
        ));
        let killer = child.clone_killer();
        let shell_pid = child.process_id();
        let master: SharedMaster = Arc::new(Mutex::new(pair.master));

        // Start Claude by typing into the fresh shell (see `launch_command`).
        // Shells buffer stdin, so writing immediately is safe even before the
        // prompt appears. A Claude launch with an mcp endpoint first writes the
        // `mcpServers` config file `--mcp-config` points at.
        let mcp_config_path = spec
            .mcp
            .as_ref()
            .and_then(|config| write_mcp_config(&self.private, spec.session, config));
        // A Claude launch also carries the settings overlay that keeps its OSC
        // title — termherd's only status channel for it — switched on.
        let settings_path = matches!(spec.launch, termherd_core::Launch::Claude(_))
            .then(|| write_title_settings(&self.private, spec.session))
            .flatten();
        if let Some(command) = launch_command(
            &spec.launch,
            mcp_config_path.as_deref(),
            settings_path.as_deref(),
        ) && let Ok(mut w) = writer.lock()
        {
            let _ = w.write_all(command.as_bytes());
            let _ = w.flush();
        }

        let (ctrl, ctrl_rx) = mpsc::channel::<TermCmd>();
        let term = spawn_term(
            spec.session,
            ctrl_rx,
            (spec.cols, spec.rows),
            writer.clone(),
            self.sink.clone(),
            self.palette.clone(),
            SCREEN_INTERVAL,
        );
        let reader = spawn_reader(spec.session, reader, ctrl.clone());
        // Detached from birth: it parks in `wait()` until the process ends,
        // whether by the user's `exit` or a later kill.
        let _ = spawn_waiter(spec.session, child, ctrl.clone(), self.sink.clone());
        // The stand-in status source, for a shell whose integration did not
        // take. It ends itself once the terminal thread is gone.
        let _ = spawn_watcher(spec.session, master.clone(), shell_pid, ctrl.clone());

        let session = Session {
            master,
            writer,
            killer,
            ctrl,
            reader: Some(reader),
            term: Some(term),
        };
        if let Ok(mut map) = self.sessions.lock() {
            map.insert(spec.session, session);
        }
        Ok(())
    }

    fn write(&self, session: SessionId, bytes: &[u8]) -> Result<(), PtyError> {
        let mut map = self
            .sessions
            .lock()
            .map_err(|_| PtyError::Io("session lock poisoned".into()))?;
        let s = map
            .get_mut(&session)
            .ok_or(PtyError::NoSuchSession(session.0.get()))?;
        let mut w = s
            .writer
            .lock()
            .map_err(|_| PtyError::Io("writer lock poisoned".into()))?;
        w.write_all(bytes)
            .map_err(|e| PtyError::Io(e.to_string()))?;
        w.flush().map_err(|e| PtyError::Io(e.to_string()))
    }

    fn resize(&self, session: SessionId, cols: u16, rows: u16) -> Result<(), PtyError> {
        let map = self
            .sessions
            .lock()
            .map_err(|_| PtyError::Io("session lock poisoned".into()))?;
        let s = map
            .get(&session)
            .ok_or(PtyError::NoSuchSession(session.0.get()))?;
        // Resize the grid (terminal thread) and the PTY (delivers SIGWINCH /
        // a ConPTY resize so the child redraws at the new size).
        let _ = s.ctrl.send(TermCmd::Resize(cols, rows));
        s.master
            .lock()
            .map_err(|_| PtyError::Io("master lock poisoned".into()))?
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Io(e.to_string()))
    }

    fn scroll(&self, session: SessionId, target: ScrollTarget) -> Result<(), PtyError> {
        self.send(session, TermCmd::Scroll(target))
    }

    fn select(&self, session: SessionId, op: SelectOp) -> Result<(), PtyError> {
        self.send(session, TermCmd::Select(op))
    }

    fn pointer(&self, session: SessionId, pointer: PointerEvent) -> Result<(), PtyError> {
        self.send(session, TermCmd::Pointer(pointer))
    }

    fn copy_selection(&self, session: SessionId) -> Result<(), PtyError> {
        self.send(session, TermCmd::CopySelection)
    }

    fn kill(&self, session: SessionId) -> Result<(), PtyError> {
        let mut map = self
            .sessions
            .lock()
            .map_err(|_| PtyError::Io("session lock poisoned".into()))?;
        let mut s = map
            .remove(&session)
            .ok_or(PtyError::NoSuchSession(session.0.get()))?;
        let result = s.killer.kill();
        // The session's private files — its integration snippet, its mcp config
        // with the bridge token — have no reason to outlive it.
        discard_private_files(&self.private, session);
        // Dropping the session drops the master/writer, which unblocks the
        // reader; the waiter thread reaps the killed child (`wait`) and signals
        // the terminal thread, which emits `Exited` on its own.
        drop(s.reader.take());
        drop(s.term.take());
        // The OS kill outcome's Unix/Windows reconciliation lives in `kill.rs`.
        finish_kill(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PtyEvent;
    use std::num::NonZeroU64;
    use std::time::{Duration, Instant};
    use termherd_core::{Launch, SessionStatus};

    fn sid(n: u64) -> SessionId {
        SessionId(NonZeroU64::new(n).expect("non-zero"))
    }

    /// Session ids restart at 1 per process, so a test writing session files
    /// into the shared temp dir writes — and on teardown deletes — those of
    /// the termherd the developer is running. Reruns this binary under a temp
    /// dir holding a running app's files, which must come through untouched,
    /// and must gain none of the suite's.
    #[test]
    fn the_suite_leaves_a_running_apps_session_files_alone() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let live = plant_live_session_files(tmp.path());
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--skip",
                "the_suite_leaves_a_running_apps_session_files_alone",
            ])
            .env("TMPDIR", tmp.path())
            .env("TMP", tmp.path())
            .env("TEMP", tmp.path())
            .output()
            .expect("rerun the test binary");
        let stdout = String::from_utf8_lossy(&output.stdout);

        let touched: Vec<_> = live
            .iter()
            .filter(|path| std::fs::read_to_string(path).ok().as_deref() != Some("live"))
            .collect();
        assert!(
            touched.is_empty(),
            "tests touched the app's files: {touched:?}"
        );
        let added: Vec<String> = std::fs::read_dir(tmp.path())
            .expect("the temp dir is readable")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| {
                name.starts_with("termherd-")
                    && !LIVE_IDS.flat_map(live_names).any(|live| live == *name)
            })
            .collect();
        assert!(
            added.is_empty(),
            "tests left termherd files there: {added:?}"
        );
        assert!(
            output.status.success() && stdout.contains("test result: ok."),
            "the app's files survived, but the rerun failed, so that proves nothing:\n{stdout}"
        );
    }

    /// The sessions of the running app whose files are planted: its first.
    const LIVE_IDS: std::ops::RangeInclusive<u64> = 1..=8;

    /// The shell dir, mcp config and settings overlay the app names for `id`.
    fn live_names(id: u64) -> [String; 3] {
        [
            format!("termherd-shell-{id}"),
            format!("termherd-mcp-{id}.json"),
            format!("termherd-settings-{id}.json"),
        ]
    }

    /// The running app's files for [`LIVE_IDS`], each holding `live`;
    /// returns every file planted.
    fn plant_live_session_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut planted = Vec::new();
        for id in LIVE_IDS {
            let [shell, mcp, settings] = live_names(id).map(|name| root.join(name));
            std::fs::create_dir(&shell).expect("the app's shell dir");
            planted.extend([shell.join(".zshrc"), mcp, settings]);
        }
        for path in &planted {
            std::fs::write(path, "live").expect("the app's file");
        }
        planted
    }

    /// A manager over the platform's default shell whose private files go to
    /// a temp dir of its own, held for as long as the manager is.
    fn manager(sink: EventSink, palette: Palette) -> (tempfile::TempDir, PtyManager) {
        let private = tempfile::tempdir().expect("tempdir");
        let mgr = PtyManager::new(sink, None, palette, PrivateFiles::new(private.path()));
        (private, mgr)
    }

    /// The root a manager is given is where a real session's files land and
    /// where its teardown removes them from. `bash` named explicitly, because
    /// its recipe always writes a file there; a host without it skips.
    #[test]
    fn a_real_sessions_private_files_live_under_the_root_it_was_given() {
        let Some(bash) = ["/bin/bash", "/usr/bin/bash"]
            .into_iter()
            .find(|path| std::path::Path::new(path).is_file())
        else {
            eprintln!("skipped: no bash on this host");
            return;
        };
        let root = tempfile::tempdir().expect("tempdir");
        let shell = Shell {
            program: bash.to_owned(),
            args: Vec::new(),
        };
        let mgr = PtyManager::new(
            Arc::new(|_| {}),
            Some(shell),
            Palette::default(),
            PrivateFiles::new(root.path()),
        );
        let id = sid(3);
        mgr.spawn(spec(id)).expect("spawn");
        let dir = root.path().join("termherd-shell-3");
        assert!(dir.is_dir(), "the shell's startup files went elsewhere");

        mgr.kill(id).expect("kill");
        assert!(!dir.exists(), "the teardown left them behind");
    }

    fn spec(session: SessionId) -> SpawnSpec {
        SpawnSpec {
            session,
            cwd: None,
            launch: Launch::Shell,
            cols: 80,
            rows: 24,
            mcp: None,
        }
    }

    /// Marks the re-run of this binary that has a parent Claude session seeded
    /// into its real environment.
    const SEEDED_RERUN: &str = "TERMHERD_TEST_SEEDED_CLAUDE_SESSION";

    /// What a Claude Code session exports that names it, against what a user
    /// configures in the same namespace.
    const PARENT_SESSION: &[&str] = &[
        "CLAUDECODE",
        "CLAUDE_CODE_CHILD_SESSION",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_CODE_MESSAGING_SOCKET",
        "CLAUDE_PID",
    ];
    const USER_CONFIGURED: &str = "CLAUDE_CONFIG_DIR";

    /// `CommandBuilder` reads the process environment, which a test cannot
    /// set for itself (`set_var` is `unsafe`, and the workspace denies it), and
    /// on a machine with no Claude session there is nothing to strip. So the
    /// binary reruns its seeded half with the variables put there.
    #[test]
    fn a_session_spawned_from_inside_claude_inherits_none_of_its_identity() {
        let mut rerun = std::process::Command::new(std::env::current_exe().expect("test binary"));
        rerun
            .args([
                "--exact",
                "manager::tests::seeded_shell_command_drops_the_parent_session",
                "--ignored",
            ])
            .env(SEEDED_RERUN, "1")
            .env(USER_CONFIGURED, "/seeded/config");
        for name in PARENT_SESSION {
            rerun.env(name, "parent-session");
        }
        let output = rerun.output().expect("rerun the test binary");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed"),
            "the seeded half failed or did not run:\n{stdout}"
        );
    }

    #[test]
    #[ignore = "run with a seeded environment by \
                a_session_spawned_from_inside_claude_inherits_none_of_its_identity"]
    fn seeded_shell_command_drops_the_parent_session() {
        assert!(
            std::env::var_os(SEEDED_RERUN).is_some(),
            "only meaningful with the parent session seeded; run the test that reruns it"
        );
        let cmd = shell_command(None, None);
        for name in PARENT_SESSION {
            assert!(std::env::var_os(name).is_some(), "{name} was not seeded");
            assert_eq!(cmd.get_env(name), None, "{name} reached the spawn");
        }
        assert_eq!(
            cmd.get_env(USER_CONFIGURED),
            Some(std::ffi::OsStr::new("/seeded/config")),
            "user configuration must still reach the spawn"
        );
    }

    /// Spawn a shell, run a command that prints a unique marker, and assert it
    /// reaches the grid — exercising spawn → write → parse → render → exit.
    #[test]
    fn spawns_writes_and_streams_output() {
        let (tx, rx) = mpsc::channel::<PtyEvent>();
        let sink: EventSink = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let (_private, mgr) = manager(sink, Palette::default());
        let id = sid(1);

        mgr.spawn(spec(id)).expect("spawn");
        assert_eq!(mgr.len(), 1);

        // `echo` exists on every supported shell (cmd, sh, bash, pwsh).
        mgr.write(id, b"echo TERMHERD_OK\r\n").expect("write");

        let deadline = Instant::now() + Duration::from_secs(15);
        let mut screen = String::new();
        let mut saw_marker = false;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(PtyEvent::Output { screen: s, .. }) => {
                    screen = s.text();
                    // The command itself echoes the literal; a second line with
                    // just the marker means the shell actually ran it.
                    if screen.matches("TERMHERD_OK").count() >= 2 {
                        saw_marker = true;
                        break;
                    }
                }
                Ok(
                    PtyEvent::Status { .. }
                    | PtyEvent::Title { .. }
                    | PtyEvent::Notification { .. }
                    | PtyEvent::Cwd { .. }
                    | PtyEvent::ForegroundJob { .. }
                    | PtyEvent::AltScreen { .. }
                    | PtyEvent::SelectionCopied { .. },
                ) => continue,
                Ok(PtyEvent::Exited { .. }) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            }
        }
        assert!(
            saw_marker,
            "expected the echoed marker in the grid, got:\n{screen}"
        );

        mgr.kill(id).expect("kill");
    }

    /// The claim behind the mouse rung, made against a **real child**: a
    /// program that turns mouse reporting on receives a click as the bytes it
    /// negotiated. `cat -v` in a raw, echo-less tty stands in for the TUI —
    /// raw so it reads the bytes as they come and echo off so what lands on
    /// the grid is what the child printed, not the line discipline — and the
    /// mode reading rides back on the `Screen` first, which is what the GUI and
    /// the MCP shell route on.
    ///
    /// Skipped on Windows: the recipe is a POSIX shell line.
    #[test]
    fn a_click_reaches_a_real_child_that_reads_the_mouse() {
        if cfg!(windows) {
            return;
        }
        use termherd_core::ports::PtyHost;
        use termherd_core::{MouseReporting, PointerEvent, PointerKind};
        let (tx, rx) = mpsc::channel::<PtyEvent>();
        let sink: EventSink = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let (_private, mgr) = manager(sink, Palette::default());
        let id = sid(1);
        mgr.spawn(spec(id)).expect("spawn");
        mgr.write(
            id,
            b"stty raw -echo; printf '\\033[?1000h\\033[?1006h'; cat -v\r\n",
        )
        .expect("write");

        let deadline = Instant::now() + Duration::from_secs(15);
        let mut screen = String::new();
        let mut clicked = false;
        let mut saw_report = false;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(PtyEvent::Output { screen: s, .. }) => {
                    screen = s.text();
                    if !clicked && s.mouse_reporting == Some(MouseReporting::Click) {
                        // The screen says the child reads the mouse; click it.
                        mgr.pointer(id, PointerEvent::left(PointerKind::Click, 4, 2))
                            .expect("pointer");
                        clicked = true;
                    }
                    if screen.contains("^[[<0;5;3M^[[<0;5;3m") {
                        saw_report = true;
                        break;
                    }
                }
                Ok(PtyEvent::Exited { .. }) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => continue,
            }
        }
        assert!(clicked, "the screen never reported mouse mode:\n{screen}");
        assert!(
            saw_report,
            "the child never printed the SGR click it was sent:\n{screen}"
        );
        mgr.kill(id).expect("kill");
    }

    /// A **real spawned shell** must leave `Starting` and then report the work
    /// it is doing — the end-to-end claim behind this whole seam, and the one
    /// no unit test can make: whether the integration snippet actually takes on
    /// the machine's shell is only knowable by running it. Either source is
    /// accepted (the marks, or the foreground stand-in); what must not happen
    /// is silence, which is what left `wait_for_status` unusable on a shell.
    ///
    /// Skipped on Windows: ConPTY exposes no foreground process group, and a
    /// shell there may have no recipe, so silence is the documented outcome.
    #[test]
    fn a_real_shell_reports_idle_at_its_prompt_then_busy_while_working() {
        if cfg!(windows) {
            return;
        }
        let (tx, rx) = mpsc::channel::<PtyEvent>();
        let sink: EventSink = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let (_private, mgr) = manager(sink, Palette::default());
        let id = sid(4);
        mgr.spawn(spec(id)).expect("spawn");

        /// Collect statuses until `wanted` shows up, or the deadline passes.
        fn wait_for(
            rx: &mpsc::Receiver<PtyEvent>,
            wanted: SessionStatus,
            seen: &mut Vec<SessionStatus>,
        ) -> bool {
            let deadline = Instant::now() + Duration::from_secs(15);
            while Instant::now() < deadline {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(PtyEvent::Status { status, .. }) => {
                        seen.push(status);
                        if status == wanted {
                            return true;
                        }
                    }
                    Ok(_) => continue,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                }
            }
            false
        }

        let mut seen = Vec::new();
        assert!(
            wait_for(&rx, SessionStatus::Idle, &mut seen),
            "a shell parked at its prompt must report idle, saw {seen:?}"
        );
        // Long enough to outlast a poll tick whichever source is in play.
        mgr.write(id, b"sleep 3\r\n").expect("write");
        assert!(
            wait_for(&rx, SessionStatus::Busy, &mut seen),
            "a shell running a command must report busy, saw {seen:?}"
        );
        mgr.kill(id).expect("kill");
    }

    /// A **real job** in front of a real shell is reported by its pid, and the
    /// shell's return to the front clears it — the seam a Claude pane's
    /// session file is found through. `sleep` stands in for `claude`: both are
    /// a foreground job the shell started. Windows has no foreground process
    /// group, so there only a `claude.exe` below the shell is reported, and a
    /// copy of `PING.EXE` named so stands in: a reported pid can then be
    /// nothing but the program this test typed.
    #[test]
    fn a_job_in_front_of_a_real_shell_is_reported_by_pid_then_cleared() {
        let (tx, rx) = mpsc::channel::<PtyEvent>();
        let sink: EventSink = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let (_private, mgr) = manager(sink, Palette::default());
        let id = sid(5);
        let stand_in = tempfile::tempdir().expect("stand-in dir");
        let job = if cfg!(windows) {
            let claude = stand_in.path().join("claude.exe");
            std::fs::copy(r"C:\Windows\System32\PING.EXE", &claude).expect("copy ping");
            format!("{} -n 4 127.0.0.1\r\n", claude.display())
        } else {
            "sleep 2\r\n".to_owned()
        };
        mgr.spawn(spec(id)).expect("spawn");
        mgr.write(id, job.as_bytes()).expect("write");

        let deadline = Instant::now() + Duration::from_secs(15);
        let mut reported = Vec::new();
        let mut stamps = Vec::new();
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(PtyEvent::ForegroundJob { job, .. }) => {
                    if let Some(started) = job.as_ref().and_then(|job| job.started.clone()) {
                        stamps.push(started);
                    }
                    let pid = job.map(|job| job.pid);
                    reported.push(pid);
                    if pid.is_none() && reported.iter().any(Option::is_some) {
                        break;
                    }
                }
                Ok(PtyEvent::Exited { .. }) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => continue,
            }
        }
        let job = reported.iter().flatten().next();
        assert!(
            job.is_some_and(|&pid| pid > 0),
            "the running job must be reported by its pid, saw {reported:?}"
        );
        assert!(
            !stamps.is_empty(),
            "the running job must carry its start stamp, the check against a reused pid"
        );
        assert_eq!(
            reported.last(),
            Some(&None),
            "the shell back in front must clear the job, saw {reported:?}"
        );
        assert!(
            reported.windows(2).all(|pair| pair[0] != pair[1]),
            "a job is reported on change only, saw {reported:?}"
        );
        mgr.kill(id).expect("kill");
    }

    /// Type `exit` into a spawned shell and assert the adapter reaps it as a
    /// *clean* exit — the signal the shell auto-close keys off.
    #[test]
    fn a_typed_exit_reports_a_clean_exit() {
        let (tx, rx) = mpsc::channel::<PtyEvent>();
        let sink: EventSink = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let (_private, mgr) = manager(sink, Palette::default());
        let id = sid(2);
        mgr.spawn(spec(id)).expect("spawn");
        // `exit 0`: a bare `exit` returns whatever status the startup files left.
        mgr.write(id, b"exit 0\r\n").expect("write");

        let deadline = Instant::now() + Duration::from_secs(15);
        let mut reaped = None;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(PtyEvent::Exited { clean, .. }) => {
                    reaped = Some(clean);
                    break;
                }
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            }
        }
        assert_eq!(reaped, Some(true), "a typed `exit` must reap as clean");
        // The manager holds the dead session's handles until the kill —
        // exactly what the auto-close's `Kill` effect releases.
        mgr.kill(id).expect("kill releases the dead session");
    }

    /// Spawn a real PTY with a custom palette and assert the streamed screens
    /// render in it — exercising manager → terminal thread → snapshot with the
    /// injected colours, the path the GUI consumes.
    #[test]
    fn a_spawned_session_streams_screens_in_the_injected_palette() {
        let palette = Palette {
            foreground: [0xff, 0xcc, 0x00],
            background: [0x3a, 0x0c, 0xa3],
            cursor: [0xff, 0x00, 0x00],
            ..Palette::default()
        };
        let (tx, rx) = mpsc::channel::<PtyEvent>();
        let sink: EventSink = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let (_private, mgr) = manager(sink, palette.clone());
        let id = sid(7);
        mgr.spawn(spec(id)).expect("spawn");

        let deadline = Instant::now() + Duration::from_secs(15);
        let mut verified = false;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(PtyEvent::Output { screen, .. }) => {
                    assert_eq!(screen.default_bg, palette.background);
                    assert_eq!(screen.cursor_color, palette.cursor);
                    // A cell the shell hasn't painted renders in the custom
                    // defaults, so the whole grid follows the scheme.
                    let blank = screen.lines.last().and_then(|l| l.last());
                    if let Some(cell) = blank
                        && cell.c == ' '
                    {
                        assert_eq!(cell.fg, palette.foreground);
                        assert_eq!(cell.bg, palette.background);
                        verified = true;
                        break;
                    }
                }
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            }
        }
        assert!(verified, "expected a screen rendered in the custom palette");

        mgr.kill(id).expect("kill");
    }

    /// A scheme change reaches a session that is already running: its next
    /// screen arrives in the new colours with no respawn and no output of its
    /// own to trigger it.
    #[test]
    fn set_palette_repaints_a_running_session() {
        let next = Palette {
            background: [0x12, 0x34, 0x56],
            ..Palette::default()
        };
        let (tx, rx) = mpsc::channel::<PtyEvent>();
        let sink: EventSink = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let (_private, mgr) = manager(sink, Palette::default());
        let id = sid(8);
        mgr.spawn(spec(id)).expect("spawn");
        mgr.set_palette(next.clone());

        let deadline = Instant::now() + Duration::from_secs(15);
        let mut repainted = false;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(PtyEvent::Output { screen, .. }) if screen.default_bg == next.background => {
                    repainted = true;
                    break;
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            }
        }
        assert!(
            repainted,
            "expected a screen in the palette set after spawn"
        );

        mgr.kill(id).expect("kill");
    }

    #[test]
    fn control_methods_reject_unknown_sessions() {
        let sink: EventSink = Arc::new(|_| {});
        let (_private, mgr) = manager(sink, Palette::default());
        let id = sid(42);
        assert!(matches!(
            mgr.write(id, b"x"),
            Err(PtyError::NoSuchSession(42))
        ));
        assert!(matches!(
            mgr.resize(id, 80, 24),
            Err(PtyError::NoSuchSession(42))
        ));
        assert!(matches!(mgr.kill(id), Err(PtyError::NoSuchSession(42))));
    }
}
