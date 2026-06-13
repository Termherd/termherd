//! `termherd-pty` — the PTY host adapter (M2).
//!
//! Implements [`termherd_core::ports::PtyHost`]. Each session is owned by its
//! own OS thread that holds the PTY reader and an `alacritty_terminal` grid;
//! the rest of the system talks to it only through this manager's control
//! methods (`write`/`resize`/`kill`) and receives output/exit through a sink
//! given at construction. There is no shared `&mut Session` — the structural
//! fix for the `realSessionId` race (Q6, `docs/PRD.md` §4).
//!
//! Minimal slice (M2 tranche 2): output is the visible screen rendered to
//! plain text. Colours, cursor, selection and scrollback come next.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::Config;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::vte::ansi::Processor;
use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use termherd_claude::osc::{OscSignal, decode_chunk};
use termherd_core::ports::{PtyError, PtyHost};
use termherd_core::workspace::SessionId;
use termherd_core::{SessionStatus, SpawnSpec};

/// Read buffer for the per-session reader thread.
const READ_BUF: usize = 8192;

/// What the adapter emits back to the runtime, out-of-band. The iced shell
/// maps these onto `core` events.
#[derive(Debug, Clone)]
pub enum PtyEvent {
    /// New terminal screen contents (minimal slice: visible text, no styling).
    Output { session: SessionId, screen: String },
    /// Activity reclassified from the OSC stream (FR8).
    Status {
        session: SessionId,
        status: SessionStatus,
    },
    /// The session's PTY process exited.
    Exited { session: SessionId },
}

/// Fold a chunk's OSC signals into the running activity status (FR8). Only
/// busy/idle markers move it; notifications, bells and alt-screen toggles are
/// surfaced elsewhere and do not change busy-vs-idle here.
fn fold_status(current: SessionStatus, signals: &[OscSignal]) -> SessionStatus {
    let mut status = current;
    for signal in signals {
        match signal {
            OscSignal::Busy => status = SessionStatus::Busy,
            OscSignal::Idle => status = SessionStatus::Idle,
            OscSignal::Notification(_) | OscSignal::AltScreen(_) | OscSignal::Bell => {}
        }
    }
    status
}

/// A sink for [`PtyEvent`]s. Cheap to clone, callable from the reader threads.
pub type EventSink = Arc<dyn Fn(PtyEvent) + Send + Sync + 'static>;

/// The PTY's input side, shared between user writes (the manager) and terminal
/// replies (the parser). A `Mutex` serialises the two so responses never
/// interleave with keystrokes mid-sequence.
type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Answers terminal queries the parser raises — chiefly the cursor-position
/// report (`ESC[6n`). **ConPTY blocks startup until it gets that reply**, so
/// dropping it (as `VoidListener` would) hangs every Windows session; this is
/// also what lets programs query the terminal on every platform.
#[derive(Clone)]
struct PtyResponder {
    writer: SharedWriter,
}

impl EventListener for PtyResponder {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event
            && let Ok(mut w) = self.writer.lock()
        {
            let _ = w.write_all(text.as_bytes());
            let _ = w.flush();
        }
    }
}

/// Hosts every live session's PTY. Construct once in `main()` and inject as a
/// `dyn PtyHost` (no global state, AGENTS.md quality bar).
pub struct PtyManager {
    sessions: Mutex<HashMap<SessionId, Session>>,
    sink: EventSink,
}

/// The control-side handle to one session. The reader half lives in the
/// thread; this half resizes, writes and kills.
struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: SharedWriter,
    killer: Box<dyn ChildKiller + Send + Sync>,
    /// A resize the reader thread should apply to its grid on its next wake.
    pending_resize: Arc<Mutex<Option<(u16, u16)>>>,
    reader: Option<JoinHandle<()>>,
}

impl PtyManager {
    /// `sink` receives every session's output and exit, from the reader
    /// threads. Wrap a channel sender to bridge into the GUI subscription.
    pub fn new(sink: EventSink) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            sink,
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

        // Default login shell, in the project directory, with a sane TERM.
        // Resuming a real Claude session lands in a later slice; the id flows
        // through so the adapter never has to invent one (Q6).
        let mut cmd = CommandBuilder::new_default_prog();
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }
        cmd.env("TERM", "xterm-256color");

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

        let pending_resize = Arc::new(Mutex::new(None));
        let handle = spawn_reader(
            spec.session,
            reader,
            child,
            (spec.cols, spec.rows),
            pending_resize.clone(),
            writer.clone(),
            self.sink.clone(),
        );

        let session = Session {
            master: pair.master,
            writer,
            killer,
            pending_resize,
            reader: Some(handle),
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
        // Tell the reader thread to resize its grid, then resize the PTY
        // (which delivers SIGWINCH / a ConPTY resize and wakes the reader).
        if let Ok(mut pending) = s.pending_resize.lock() {
            *pending = Some((cols, rows));
        }
        s.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Io(e.to_string()))
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
        // Dropping the session drops the master/writer; the reader thread then
        // sees EOF, reaps the child (`wait`) and emits `Exited` on its own.
        drop(s.reader.take());
        // portable-pty's `WinChildKiller::kill` inverts its result — it
        // returns `Err(last_os_error())` when `TerminateProcess` *succeeds*
        // (non-zero return = success on Win32) — so its `Result` is unusable.
        // The terminate is still issued; treat the call as best-effort there.
        #[cfg(windows)]
        {
            let _ = result;
            Ok(())
        }
        #[cfg(not(windows))]
        {
            result.map_err(|e| PtyError::Io(e.to_string()))
        }
    }
}

/// Spawn the per-session reader thread: it owns the PTY reader and the grid,
/// feeds bytes through the VTE parser, and pushes a text snapshot per chunk.
fn spawn_reader(
    session: SessionId,
    mut reader: Box<dyn Read + Send>,
    mut child: Box<dyn Child + Send + Sync>,
    size: (u16, u16),
    pending_resize: Arc<Mutex<Option<(u16, u16)>>>,
    writer: SharedWriter,
    sink: EventSink,
) -> JoinHandle<()> {
    let (cols, rows) = size;
    let reader_sink = sink.clone();
    std::thread::Builder::new()
        .name(format!("pty-{}", session.0.get()))
        .spawn(move || {
            let mut term = Term::new(
                Config::default(),
                &TermSize::new(cols as usize, rows as usize),
                PtyResponder { writer },
            );
            let mut parser: Processor = Processor::new();
            let mut buf = [0u8; READ_BUF];
            let mut status = SessionStatus::Starting;
            loop {
                if let Ok(mut pending) = pending_resize.lock()
                    && let Some((c, r)) = pending.take()
                {
                    term.resize(TermSize::new(c as usize, r as usize));
                }
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = &buf[..n];
                        // OSC status comes from the raw bytes — alacritty
                        // consumes the sequences, so decode before parsing.
                        let next =
                            fold_status(status, &decode_chunk(&String::from_utf8_lossy(chunk)));
                        if next != status {
                            status = next;
                            reader_sink(PtyEvent::Status { session, status });
                        }
                        parser.advance(&mut term, chunk);
                        reader_sink(PtyEvent::Output {
                            session,
                            screen: render_screen(&term),
                        });
                    }
                    Err(e) => {
                        tracing::debug!(%e, session = session.0.get(), "pty reader stopped");
                        break;
                    }
                }
            }
            // Reap the child so it does not linger as a zombie.
            let _ = child.wait();
            reader_sink(PtyEvent::Exited { session });
        })
        .unwrap_or_else(|_| {
            // Thread spawn failing is catastrophic and vanishingly rare; emit
            // an immediate exit so the session does not hang half-open.
            sink(PtyEvent::Exited { session });
            std::thread::spawn(|| {})
        })
}

/// Render the visible grid to plain text (minimal slice). Trailing blank
/// cells and lines are trimmed so the GUI shows a tidy screen.
fn render_screen<T: EventListener>(term: &Term<T>) -> String {
    let grid = term.grid();
    let cols = grid.columns();
    let lines = grid.screen_lines();
    let mut out = String::with_capacity(cols * lines);
    for l in 0..lines as i32 {
        let row = &grid[Line(l)];
        let mut line = String::with_capacity(cols);
        for c in 0..cols {
            line.push(row[Column(c)].c);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim_end_matches('\n').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU64;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn sid(n: u64) -> SessionId {
        SessionId(NonZeroU64::new(n).expect("non-zero"))
    }

    fn spec(session: SessionId) -> SpawnSpec {
        SpawnSpec {
            session,
            cwd: None,
            resume: None,
            cols: 80,
            rows: 24,
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
        let mgr = PtyManager::new(sink);
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
                    screen = s;
                    // The command itself echoes the literal; a second line with
                    // just the marker means the shell actually ran it.
                    if screen.matches("TERMHERD_OK").count() >= 2 {
                        saw_marker = true;
                        break;
                    }
                }
                Ok(PtyEvent::Status { .. }) => continue,
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

    #[test]
    fn fold_status_tracks_busy_idle_and_ignores_the_rest() {
        use SessionStatus::*;
        // The last busy/idle marker in the chunk wins.
        assert_eq!(
            fold_status(Starting, &[OscSignal::Busy, OscSignal::Idle]),
            Idle
        );
        assert_eq!(fold_status(Idle, &[OscSignal::Busy]), Busy);
        // Notifications, bells and alt-screen toggles leave it unchanged.
        assert_eq!(
            fold_status(
                Busy,
                &[
                    OscSignal::Notification("x".into()),
                    OscSignal::Bell,
                    OscSignal::AltScreen(true),
                ]
            ),
            Busy
        );
        // No signals at all keeps the current status (e.g. a plain shell).
        assert_eq!(fold_status(Starting, &[]), Starting);
    }

    #[test]
    fn control_methods_reject_unknown_sessions() {
        let sink: EventSink = Arc::new(|_| {});
        let mgr = PtyManager::new(sink);
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
