//! `termherd-pty` — the PTY host adapter (M2).
//!
//! Implements [`termherd_core::ports::PtyHost`]. Each session is owned by its
//! own OS thread that holds the PTY reader and an `alacritty_terminal` grid;
//! the rest of the system talks to it only through this manager's control
//! methods (`write`/`resize`/`kill`) and receives output/exit through a sink
//! given at construction. There is no shared `&mut Session` — the structural
//! fix for the `realSessionId` race (Q6, `docs/PRD.md` §4).
//!
//! Output is a [`Screen`] snapshot of the visible grid: per-cell RGB (xterm
//! 256 palette), the cursor, and a scrolled flag (FR4). Selection is the one
//! FR4 item still pending.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::Config;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor};
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
    /// New terminal screen contents — the visible grid with per-cell colour
    /// and the cursor (FR4).
    Output { session: SessionId, screen: Screen },
    /// Activity reclassified from the OSC stream (FR8).
    Status {
        session: SessionId,
        status: SessionStatus,
    },
    /// The session's PTY process exited.
    Exited { session: SessionId },
}

/// A snapshot of the visible terminal grid handed to the GUI for rendering.
/// Colours are resolved to RGB here so the shell needs no terminal knowledge.
#[derive(Debug, Clone)]
pub struct Screen {
    pub cols: u16,
    pub rows: u16,
    /// Visible rows, top to bottom; each is exactly `cols` cells wide.
    pub lines: Vec<Vec<ScreenCell>>,
    /// Cursor position as `(col, row)` in visible coordinates, if shown.
    pub cursor: Option<(u16, u16)>,
    /// True while the viewport is scrolled up into scrollback history.
    pub scrolled: bool,
    /// True when the application has enabled bracketed paste (DECSET 2004), so
    /// the shell wraps a paste in `ESC[200~`…`ESC[201~` and a multi-line paste
    /// lands as one block instead of submitting line by line (FR4).
    pub bracketed_paste: bool,
}

/// One rendered grid cell: a character and its resolved colours.
#[derive(Debug, Clone, Copy)]
pub struct ScreenCell {
    pub c: char,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub bold: bool,
}

impl ScreenCell {
    const fn blank() -> Self {
        Self {
            c: ' ',
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            bold: false,
        }
    }
}

impl Screen {
    /// Flatten the visible grid to plain text (trailing blanks trimmed) — for
    /// logging and tests.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = String::with_capacity(self.lines.len() * (self.cols as usize + 1));
        for line in &self.lines {
            let row: String = line.iter().map(|cell| cell.c).collect();
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out.trim_end_matches('\n').to_string()
    }
}

/// Default foreground/background when a cell uses the terminal's defaults.
const DEFAULT_FG: [u8; 3] = [0xd0, 0xd0, 0xd0];
const DEFAULT_BG: [u8; 3] = [0x11, 0x13, 0x18];

/// The 16 ANSI colours (classic VGA palette), indices 0–15.
const ANSI16: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00],
    [0xcc, 0x33, 0x33],
    [0x33, 0xcc, 0x33],
    [0xcc, 0xcc, 0x33],
    [0x33, 0x66, 0xcc],
    [0xcc, 0x33, 0xcc],
    [0x33, 0xcc, 0xcc],
    [0xcc, 0xcc, 0xcc],
    [0x66, 0x66, 0x66],
    [0xff, 0x66, 0x66],
    [0x66, 0xff, 0x66],
    [0xff, 0xff, 0x66],
    [0x66, 0x99, 0xff],
    [0xff, 0x66, 0xff],
    [0x66, 0xff, 0xff],
    [0xff, 0xff, 0xff],
];

/// Resolve an xterm 256-colour index to RGB (16 ANSI + 6×6×6 cube + ramp).
fn indexed_rgb(i: u8) -> [u8; 3] {
    match i {
        0..=15 => ANSI16[i as usize],
        16..=231 => {
            let n = i - 16;
            let levels = [0u8, 95, 135, 175, 215, 255];
            [
                levels[(n / 36) as usize],
                levels[((n / 6) % 6) as usize],
                levels[(n % 6) as usize],
            ]
        }
        232..=255 => {
            let v = 8 + 10 * (i - 232);
            [v, v, v]
        }
    }
}

/// Resolve a named colour to RGB, falling back to the configured defaults.
fn named_rgb(named: NamedColor) -> [u8; 3] {
    use NamedColor::*;
    match named {
        Black => ANSI16[0],
        Red => ANSI16[1],
        Green => ANSI16[2],
        Yellow => ANSI16[3],
        Blue => ANSI16[4],
        Magenta => ANSI16[5],
        Cyan => ANSI16[6],
        White => ANSI16[7],
        BrightBlack => ANSI16[8],
        BrightRed => ANSI16[9],
        BrightGreen => ANSI16[10],
        BrightYellow => ANSI16[11],
        BrightBlue => ANSI16[12],
        BrightMagenta => ANSI16[13],
        BrightCyan => ANSI16[14],
        BrightWhite => ANSI16[15],
        DimBlack => ANSI16[0],
        DimRed => [0x88, 0x22, 0x22],
        DimGreen => [0x22, 0x88, 0x22],
        DimYellow => [0x88, 0x88, 0x22],
        DimBlue => [0x22, 0x44, 0x88],
        DimMagenta => [0x88, 0x22, 0x88],
        DimCyan => [0x22, 0x88, 0x88],
        DimWhite => [0x88, 0x88, 0x88],
        Foreground | BrightForeground => DEFAULT_FG,
        DimForeground => [0x99, 0x99, 0x99],
        Background => DEFAULT_BG,
        Cursor => DEFAULT_FG,
    }
}

fn resolve(color: Color) -> [u8; 3] {
    match color {
        Color::Spec(rgb) => [rgb.r, rgb.g, rgb.b],
        Color::Indexed(i) => indexed_rgb(i),
        Color::Named(named) => named_rgb(named),
    }
}

/// Fold a chunk's OSC signals into the running activity status (FR8).
///
/// Busy/idle titles track work; an OSC 9 notification means the CLI wants the
/// user (a permission prompt or an explicit ping) → [`SessionStatus::Attention`].
/// Attention is sticky: a plain idle prompt does not clear it (the user still
/// has to act); only real work resuming (`Busy`) does. Bells and alt-screen
/// toggles never change the activity status.
fn fold_status(current: SessionStatus, signals: &[OscSignal]) -> SessionStatus {
    let mut status = current;
    for signal in signals {
        status = match signal {
            OscSignal::Busy => SessionStatus::Busy,
            // A pending attention request outranks a bare idle prompt.
            OscSignal::Idle if status == SessionStatus::Attention => SessionStatus::Attention,
            OscSignal::Idle => SessionStatus::Idle,
            OscSignal::Notification(_) => SessionStatus::Attention,
            OscSignal::AltScreen(_) | OscSignal::Bell => status,
        };
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
}

/// A command for the per-session terminal thread, which owns the grid.
enum TermCmd {
    /// Raw bytes read from the PTY.
    Bytes(Vec<u8>),
    /// Resize the grid (the PTY itself is resized by the manager).
    Resize(u16, u16),
    /// Scroll the viewport by a line delta (positive = into history).
    Scroll(i32),
    /// The PTY reached end of file; the process is gone.
    Eof,
}

/// The control-side handle to one session. The grid lives in the terminal
/// thread; this half writes to the PTY, drives resize/scroll and kills.
struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: SharedWriter,
    killer: Box<dyn ChildKiller + Send + Sync>,
    /// Commands to the terminal thread (resize / scroll). The reader thread
    /// holds the other clone for `Bytes`/`Eof`.
    ctrl: mpsc::Sender<TermCmd>,
    reader: Option<JoinHandle<()>>,
    term: Option<JoinHandle<()>>,
}

impl PtyManager {
    /// `sink` receives every session's output and exit, from the reader
    /// threads. Wrap a channel sender to bridge into the GUI subscription.
    /// `shell` is the configured shell to launch, or `None` for the platform
    /// default (FR10).
    pub fn new(sink: EventSink, shell: Option<Shell>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            sink,
            shell,
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

        // Resuming a Claude session: type the command into the fresh shell.
        // Shells buffer stdin, so writing it immediately is safe even before
        // the prompt appears, and it keeps `claude` resolution to the user's
        // own shell + PATH (robust across platforms).
        if let Some(resume) = &spec.resume
            && let Ok(mut w) = writer.lock()
        {
            let command = format!("claude --resume {resume}\r");
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
        );
        let reader = spawn_reader(spec.session, reader, child, ctrl.clone(), self.sink.clone());

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

    fn scroll(&self, session: SessionId, delta: i32) -> Result<(), PtyError> {
        let map = self
            .sessions
            .lock()
            .map_err(|_| PtyError::Io("session lock poisoned".into()))?;
        let s = map
            .get(&session)
            .ok_or(PtyError::NoSuchSession(session.0.get()))?;
        s.ctrl
            .send(TermCmd::Scroll(delta))
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
        // Dropping the session drops the master/writer; the reader thread then
        // sees EOF, reaps the child (`wait`) and signals the terminal thread,
        // which emits `Exited` on its own.
        drop(s.reader.take());
        drop(s.term.take());
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

/// The PTY reader thread: blocking-reads bytes and forwards them to the
/// terminal thread, then reaps the child and signals EOF. Reading is isolated
/// here so the terminal thread can react to resize/scroll immediately, without
/// waiting on a blocked `read` (FR4 scrollback).
fn spawn_reader(
    session: SessionId,
    mut reader: Box<dyn Read + Send>,
    mut child: Box<dyn Child + Send + Sync>,
    ctrl: mpsc::Sender<TermCmd>,
    sink: EventSink,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name(format!("pty-rd-{}", session.0.get()))
        .spawn(move || {
            let mut buf = [0u8; READ_BUF];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if ctrl.send(TermCmd::Bytes(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::debug!(%e, session = session.0.get(), "pty reader stopped");
                        break;
                    }
                }
            }
            // Reap the child so it does not linger as a zombie, then tell the
            // terminal thread the session is over.
            let _ = child.wait();
            let _ = ctrl.send(TermCmd::Eof);
        })
        .unwrap_or_else(|_| {
            sink(PtyEvent::Exited { session });
            std::thread::spawn(|| {})
        })
}

/// The terminal thread: owns the `alacritty_terminal` grid and applies every
/// [`TermCmd`] (bytes → parse + status, resize, scroll), emitting a fresh
/// [`Screen`] each time. It exits — and reports [`PtyEvent::Exited`] — when the
/// reader signals EOF or every command sender is dropped.
fn spawn_term(
    session: SessionId,
    ctrl_rx: mpsc::Receiver<TermCmd>,
    size: (u16, u16),
    writer: SharedWriter,
    sink: EventSink,
) -> JoinHandle<()> {
    let (cols, rows) = size;
    let term_sink = sink.clone();
    std::thread::Builder::new()
        .name(format!("pty-tm-{}", session.0.get()))
        .spawn(move || {
            let mut term = Term::new(
                Config::default(),
                &TermSize::new(cols as usize, rows as usize),
                PtyResponder { writer },
            );
            let mut parser: Processor = Processor::new();
            let mut status = SessionStatus::Starting;
            while let Ok(cmd) = ctrl_rx.recv() {
                match cmd {
                    TermCmd::Bytes(bytes) => {
                        // OSC status comes from the raw bytes — alacritty
                        // consumes the sequences, so decode before parsing.
                        let next =
                            fold_status(status, &decode_chunk(&String::from_utf8_lossy(&bytes)));
                        if next != status {
                            status = next;
                            term_sink(PtyEvent::Status { session, status });
                        }
                        parser.advance(&mut term, &bytes);
                    }
                    TermCmd::Resize(c, r) => {
                        term.resize(TermSize::new(c as usize, r as usize));
                    }
                    TermCmd::Scroll(delta) => {
                        term.scroll_display(Scroll::Delta(delta));
                    }
                    TermCmd::Eof => break,
                }
                term_sink(PtyEvent::Output {
                    session,
                    screen: snapshot(&term),
                });
            }
            term_sink(PtyEvent::Exited { session });
        })
        .unwrap_or_else(|_| {
            sink(PtyEvent::Exited { session });
            std::thread::spawn(|| {})
        })
}

/// The bytes a paste sends to the PTY (FR4). Newlines are normalised to the
/// carriage return the terminal expects for Enter; when the application has
/// enabled bracketed paste (see [`Screen::bracketed_paste`]) the text is
/// wrapped in `ESC[200~`…`ESC[201~` so a multi-line paste arrives as one block
/// instead of submitting each line. Terminal byte protocol lives here, in the
/// terminal adapter, not in the GUI shell.
#[must_use]
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let mut out = Vec::with_capacity(normalized.len() + 12);
        out.extend_from_slice(b"\x1b[200~");
        out.extend_from_slice(normalized.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        normalized.into_bytes()
    }
}

/// Snapshot the visible grid into a [`Screen`] with resolved colours and the
/// cursor (FR4). Wide-char spacer cells are dropped; the wide glyph keeps its
/// own column.
fn snapshot<T: EventListener>(term: &Term<T>) -> Screen {
    let cols = term.columns() as u16;
    let rows = term.screen_lines() as u16;
    let mut lines = vec![vec![ScreenCell::blank(); cols as usize]; rows as usize];

    let content = term.renderable_content();
    let first_line = -(content.display_offset as i32);
    let cursor_shape = content.cursor.shape;
    let cursor_point = content.cursor.point;

    for indexed in content.display_iter {
        let row = indexed.point.line.0 - first_line;
        let col = indexed.point.column.0;
        if row < 0 || row as u16 >= rows || col as u16 >= cols {
            continue;
        }
        let cell = indexed.cell;
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let bold = cell.flags.intersects(Flags::BOLD | Flags::DIM_BOLD);
        let mut fg = resolve(cell.fg);
        let mut bg = resolve(cell.bg);
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        let c = if cell.flags.contains(Flags::HIDDEN) {
            ' '
        } else {
            cell.c
        };
        lines[row as usize][col] = ScreenCell { c, fg, bg, bold };
    }

    let cursor = (cursor_shape != CursorShape::Hidden)
        .then(|| {
            let row = cursor_point.line.0 - first_line;
            (row >= 0 && (row as u16) < rows && (cursor_point.column.0 as u16) < cols)
                .then_some((cursor_point.column.0 as u16, row as u16))
        })
        .flatten();

    Screen {
        cols,
        rows,
        lines,
        cursor,
        scrolled: content.display_offset > 0,
        bracketed_paste: term.mode().contains(TermMode::BRACKETED_PASTE),
    }
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
        let mgr = PtyManager::new(sink, None);
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
    fn fold_status_tracks_busy_idle_attention() {
        use SessionStatus::*;
        // The last busy/idle marker in the chunk wins.
        assert_eq!(
            fold_status(Starting, &[OscSignal::Busy, OscSignal::Idle]),
            Idle
        );
        assert_eq!(fold_status(Idle, &[OscSignal::Busy]), Busy);
        // An OSC 9 notification means the CLI needs the user → Attention.
        assert_eq!(
            fold_status(Busy, &[OscSignal::Notification("x".into())]),
            Attention
        );
        // Attention is sticky against a bare idle prompt, but Busy clears it.
        assert_eq!(fold_status(Attention, &[OscSignal::Idle]), Attention);
        assert_eq!(fold_status(Attention, &[OscSignal::Busy]), Busy);
        // Bells and alt-screen toggles leave the status unchanged.
        assert_eq!(
            fold_status(Busy, &[OscSignal::Bell, OscSignal::AltScreen(true)]),
            Busy
        );
        // No signals at all keeps the current status (e.g. a plain shell).
        assert_eq!(fold_status(Starting, &[]), Starting);
    }

    #[test]
    fn paste_normalises_newlines_and_wraps_when_bracketed() {
        // Plain paste: CRLF and LF collapse to the CR a terminal reads as Enter.
        assert_eq!(paste_bytes("a\r\nb\nc", false), b"a\rb\rc".to_vec());
        // Bracketed paste wraps the (normalised) text so it lands as one block.
        assert_eq!(
            paste_bytes("a\nb", true),
            b"\x1b[200~a\rb\x1b[201~".to_vec()
        );
    }

    #[test]
    fn snapshot_tracks_bracketed_paste_mode() {
        use alacritty_terminal::event::VoidListener;
        let mut term = Term::new(Config::default(), &TermSize::new(20, 5), VoidListener);
        let mut parser: Processor = Processor::new();
        assert!(!snapshot(&term).bracketed_paste);
        // DECSET 2004 turns it on; the matching reset turns it off again.
        parser.advance(&mut term, b"\x1b[?2004h");
        assert!(snapshot(&term).bracketed_paste);
        parser.advance(&mut term, b"\x1b[?2004l");
        assert!(!snapshot(&term).bracketed_paste);
    }

    #[test]
    fn colour_resolution_covers_the_256_palette() {
        // ANSI 16.
        assert_eq!(indexed_rgb(0), [0x00, 0x00, 0x00]);
        assert_eq!(indexed_rgb(15), [0xff, 0xff, 0xff]);
        // First cube entry (16) is black; last (231) is white.
        assert_eq!(indexed_rgb(16), [0, 0, 0]);
        assert_eq!(indexed_rgb(231), [255, 255, 255]);
        // Grayscale ramp endpoints.
        assert_eq!(indexed_rgb(232), [8, 8, 8]);
        assert_eq!(indexed_rgb(255), [238, 238, 238]);
        // Spec passes through; named foreground/background hit the defaults.
        assert_eq!(
            resolve(Color::Spec(alacritty_terminal::vte::ansi::Rgb {
                r: 1,
                g: 2,
                b: 3
            })),
            [1, 2, 3]
        );
        assert_eq!(resolve(Color::Named(NamedColor::Background)), DEFAULT_BG);
    }

    #[test]
    fn control_methods_reject_unknown_sessions() {
        let sink: EventSink = Arc::new(|_| {});
        let mgr = PtyManager::new(sink, None);
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
