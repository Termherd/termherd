//! The PTY host: [`PtyManager`] owns every live session and implements
//! [`termherd_core::ports::PtyHost`]. `spawn` wires a session's three threads
//! (reader / waiter / terminal, see [`crate::session`]) and the control methods
//! forward user actions to it over the command channel. Construct once in
//! `main()` and inject as a `dyn PtyHost` (no global state, AGENTS.md quality
//! bar).

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex, mpsc};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use termherd_core::ports::{PtyError, PtyHost};
use termherd_core::workspace::SessionId;
use termherd_core::{ScrollTarget, SelectOp, SpawnSpec};

use crate::events::EventSink;
use crate::grid::Palette;
use crate::kill::finish_kill;
use crate::session::{Session, SharedWriter, TermCmd, spawn_reader, spawn_term, spawn_waiter};
use crate::status::{apply_terminal_env, launch_command, write_mcp_config};

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
    shell: Option<Shell>,
    /// The terminal colour scheme every session renders with.
    palette: Palette,
}

impl PtyManager {
    /// `sink` receives every session's output and exit, from the reader
    /// threads. Wrap a channel sender to bridge into the GUI subscription.
    /// `shell` is the configured shell to launch, or `None` for the platform
    /// default; `palette` the terminal colour scheme (FR10).
    pub fn new(sink: EventSink, shell: Option<Shell>, palette: Palette) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            sink,
            shell,
            palette,
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
        let mut cmd = match &self.shell {
            Some(shell) => {
                let mut c = CommandBuilder::new(&shell.program);
                for arg in &shell.args {
                    c.arg(arg);
                }
                c
            }
            None => CommandBuilder::new_default_prog(),
        };
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }
        apply_terminal_env(&mut cmd);

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

        // Start Claude by typing into the fresh shell (see `launch_command`).
        // Shells buffer stdin, so writing immediately is safe even before the
        // prompt appears. A Claude launch with an mcp endpoint first writes the
        // `mcpServers` config file `--mcp-config` points at.
        let mcp_config_path = spec
            .mcp
            .as_ref()
            .and_then(|config| write_mcp_config(spec.session, config));
        if let Some(command) = launch_command(&spec.launch, mcp_config_path.as_deref())
            && let Ok(mut w) = writer.lock()
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
        );
        let reader = spawn_reader(spec.session, reader, ctrl.clone());
        // Detached from birth: it parks in `wait()` until the process ends,
        // whether by the user's `exit` or a later kill.
        let _ = spawn_waiter(spec.session, child, ctrl.clone(), self.sink.clone());

        let session = Session {
            master: pair.master,
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
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Io(e.to_string()))
    }

    fn scroll(&self, session: SessionId, target: ScrollTarget) -> Result<(), PtyError> {
        let map = self
            .sessions
            .lock()
            .map_err(|_| PtyError::Io("session lock poisoned".into()))?;
        let s = map
            .get(&session)
            .ok_or(PtyError::NoSuchSession(session.0.get()))?;
        s.ctrl
            .send(TermCmd::Scroll(target))
            .map_err(|_| PtyError::Io("terminal thread gone".into()))
    }

    fn select(&self, session: SessionId, op: SelectOp) -> Result<(), PtyError> {
        let map = self
            .sessions
            .lock()
            .map_err(|_| PtyError::Io("session lock poisoned".into()))?;
        let s = map
            .get(&session)
            .ok_or(PtyError::NoSuchSession(session.0.get()))?;
        s.ctrl
            .send(TermCmd::Select(op))
            .map_err(|_| PtyError::Io("terminal thread gone".into()))
    }

    fn copy_selection(&self, session: SessionId) -> Result<(), PtyError> {
        let map = self
            .sessions
            .lock()
            .map_err(|_| PtyError::Io("session lock poisoned".into()))?;
        let s = map
            .get(&session)
            .ok_or(PtyError::NoSuchSession(session.0.get()))?;
        s.ctrl
            .send(TermCmd::CopySelection)
            .map_err(|_| PtyError::Io("terminal thread gone".into()))
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
    use termherd_core::Launch;

    fn sid(n: u64) -> SessionId {
        SessionId(NonZeroU64::new(n).expect("non-zero"))
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

    /// Spawn a shell, run a command that prints a unique marker, and assert it
    /// reaches the grid — exercising spawn → write → parse → render → exit.
    #[test]
    fn spawns_writes_and_streams_output() {
        let (tx, rx) = mpsc::channel::<PtyEvent>();
        let sink: EventSink = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let mgr = PtyManager::new(sink, None, Palette::default());
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

    /// Type `exit` into a spawned shell and assert the adapter reaps it as a
    /// *clean* exit — the signal the shell auto-close keys off.
    #[test]
    fn a_typed_exit_reports_a_clean_exit() {
        let (tx, rx) = mpsc::channel::<PtyEvent>();
        let sink: EventSink = Arc::new(move |ev| {
            let _ = tx.send(ev);
        });
        let mgr = PtyManager::new(sink, None, Palette::default());
        let id = sid(2);
        mgr.spawn(spec(id)).expect("spawn");
        mgr.write(id, b"exit\r\n").expect("write");

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
        let mgr = PtyManager::new(sink, None, palette.clone());
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

    #[test]
    fn control_methods_reject_unknown_sessions() {
        let sink: EventSink = Arc::new(|_| {});
        let mgr = PtyManager::new(sink, None, Palette::default());
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
