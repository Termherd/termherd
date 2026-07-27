//! The per-session actor: the terminal thread that owns the `alacritty_terminal`
//! grid, the reader thread that feeds it PTY bytes, and the waiter thread that
//! reaps the process. There is no shared `&mut Session` — the control half here
//! only writes to the PTY and sends [`TermCmd`]s; the grid lives on its thread
//! (Q6). Also home to [`PtyResponder`], which answers the cursor-position and
//! colour queries the parser raises.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::term::Config;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::vte::ansi::{NamedColor, Processor, Rgb};
use portable_pty::{Child, ChildKiller, MasterPty};
use termherd_claude::osc::{OscSignal, decode_chunk};
use termherd_core::workspace::SessionId;
use termherd_core::{ScrollTarget, SelectOp, SessionStatus};

use crate::events::{EventSink, PtyEvent};
use crate::grid::{Palette, apply_select, indexed_rgb, snapshot};
use crate::input::wheel_bytes;
use crate::prompt::decode_marks;
use crate::status::{Activity, foreground_leader, foreground_status};

/// Read buffer for the per-session reader thread.
const READ_BUF: usize = 8192;

/// How long the terminal thread keeps draining trailing output after the
/// waiter reported the process exit, before declaring the session over.
const EOF_DRAIN_QUIET: std::time::Duration = std::time::Duration::from_millis(50);

/// The PTY's input side, shared between user writes (the manager) and terminal
/// replies (the parser). A `Mutex` serialises the two so responses never
/// interleave with keystrokes mid-sequence.
pub(crate) type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// The PTY's control side, shared between the manager (resize) and the watcher
/// thread (reading the foreground process group). Neither holds it for long.
pub(crate) type SharedMaster = Arc<Mutex<Box<dyn MasterPty + Send>>>;

/// How often the watcher thread asks the PTY which process group owns it. Short
/// enough that `wait_for_status` on an unintegrated shell settles promptly,
/// long enough to be free: one `tcgetpgrp` per session per tick.
const FOREGROUND_POLL: std::time::Duration = std::time::Duration::from_millis(250);

/// Answers terminal queries the parser raises — chiefly the cursor-position
/// report (`ESC[6n`). **ConPTY blocks startup until it gets that reply**, so
/// dropping it (as `VoidListener` would) hangs every Windows session; this is
/// also what lets programs query the terminal on every platform.
///
/// Colour queries (OSC 4/10/11/12) are answered from the session's palette:
/// a CLI's theme auto-detection asks for the background (OSC 11) and picks
/// light or dark from the reply — unanswered, it assumes dark.
#[derive(Clone)]
struct PtyResponder {
    writer: SharedWriter,
    palette: Palette,
}

impl EventListener for PtyResponder {
    fn send_event(&self, event: Event) {
        let reply = match event {
            Event::PtyWrite(text) => text,
            Event::ColorRequest(index, format) => match query_rgb(index, &self.palette) {
                Some([r, g, b]) => format(Rgb { r, g, b }),
                None => return,
            },
            _ => return,
        };
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(reply.as_bytes());
            let _ = w.flush();
        }
    }
}

/// The palette colour a query index refers to: 0–255 the indexed palette,
/// then the named foreground/background/cursor. The dim variants beyond have
/// no query sequence, so they draw no reply.
fn query_rgb(index: usize, palette: &Palette) -> Option<[u8; 3]> {
    match index {
        0..=255 => Some(indexed_rgb(index as u8, palette)),
        i if i == NamedColor::Foreground as usize => Some(palette.foreground),
        i if i == NamedColor::Background as usize => Some(palette.background),
        i if i == NamedColor::Cursor as usize => Some(palette.cursor),
        _ => None,
    }
}

/// A command for the per-session terminal thread, which owns the grid.
pub(crate) enum TermCmd {
    /// Raw bytes read from the PTY.
    Bytes(Vec<u8>),
    /// Resize the grid (the PTY itself is resized by the manager).
    Resize(u16, u16),
    /// Move the viewport: a relative line delta or an absolute top/bottom jump.
    Scroll(ScrollTarget),
    /// Change the grid-anchored text selection (press / drag / clear).
    Select(SelectOp),
    /// Copy the current selection to the clipboard via a `SelectionCopied` event.
    CopySelection,
    /// What the PTY's foreground process group implies about the session's
    /// activity, or `None` when the platform cannot say. The stand-in source
    /// for a shell whose integration did not take (see [`crate::status`]).
    Foreground(Option<SessionStatus>),
    /// The PTY reached end of file; the process is gone. `clean` carries the
    /// reaped exit status (see [`PtyEvent::Exited`]).
    Eof { clean: bool },
}

/// The control-side handle to one session. The grid lives in the terminal
/// thread; this half writes to the PTY, drives resize/scroll and kills.
pub(crate) struct Session {
    pub(crate) master: SharedMaster,
    pub(crate) writer: SharedWriter,
    pub(crate) killer: Box<dyn ChildKiller + Send + Sync>,
    /// Commands to the terminal thread (resize / scroll). The reader thread
    /// holds a clone for `Bytes`, the waiter thread one for `Eof`.
    pub(crate) ctrl: mpsc::Sender<TermCmd>,
    pub(crate) reader: Option<JoinHandle<()>>,
    pub(crate) term: Option<JoinHandle<()>>,
}

/// The PTY reader thread: blocking-reads bytes and forwards them to the
/// terminal thread. Reading is isolated here so the terminal thread can react
/// to resize/scroll immediately, without waiting on a blocked `read` (FR4
/// scrollback). End-of-session is *not* detected here: under ConPTY the output
/// pipe only closes when the pseudo console itself is dropped, so a child's
/// natural exit never surfaces as EOF — the waiter thread owns that signal.
pub(crate) fn spawn_reader(
    session: SessionId,
    mut reader: Box<dyn Read + Send>,
    ctrl: mpsc::Sender<TermCmd>,
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
        })
        .unwrap_or_else(|error| {
            tracing::warn!(
                %error,
                session = session.0.get(),
                "could not spawn the pty reader thread"
            );
            std::thread::spawn(|| {})
        })
}

/// The child-waiter thread: blocks until the session's process exits, reaps it
/// (no zombie), and tells the terminal thread the session is over — with
/// whether it completed successfully, which drives the auto-close on a clean
/// shell exit. Exit must be observed on the process itself, not as reader EOF
/// (see [`spawn_reader`]).
pub(crate) fn spawn_waiter(
    session: SessionId,
    mut child: Box<dyn Child + Send + Sync>,
    ctrl: mpsc::Sender<TermCmd>,
    sink: EventSink,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name(format!("pty-wt-{}", session.0.get()))
        .spawn(move || {
            let clean = child.wait().map(|status| status.success()).unwrap_or(false);
            let _ = ctrl.send(TermCmd::Eof { clean });
        })
        .unwrap_or_else(|_| {
            // Without a waiter the exit would never be observed; declare the
            // session over rather than leaving it undead.
            sink(PtyEvent::Exited {
                session,
                clean: false,
            });
            std::thread::spawn(|| {})
        })
}

/// Emit a [`PtyEvent::Status`] when `activity` has moved off `before`. Both
/// sources — the OSC fold and the foreground poll — report through here, so a
/// status change is logged and published in exactly one place.
fn report_status(session: SessionId, before: SessionStatus, activity: &Activity, sink: &EventSink) {
    if activity.status == before {
        return;
    }
    tracing::debug!(
        session = session.0.get(),
        from = ?before,
        to = ?activity.status,
        "session status changed"
    );
    sink(PtyEvent::Status {
        session,
        status: activity.status,
    });
}

/// The foreground-watcher thread: reports which process group owns the PTY, so
/// a shell whose integration did not take is not left mute (see
/// [`crate::status::Activity`]). It stops as soon as the terminal thread is
/// gone — the send fails — so it never outlives its session.
///
/// A thread rather than a timer on the runtime: the read is a blocking `ioctl`
/// through `portable_pty`, and the terminal thread must stay free to drain
/// output. `shell` is the session's own process id, the thing a foreground
/// reading is compared against.
pub(crate) fn spawn_watcher(
    session: SessionId,
    master: SharedMaster,
    shell: Option<u32>,
    ctrl: mpsc::Sender<TermCmd>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name(format!("pty-fg-{}", session.0.get()))
        .spawn(move || {
            // Where the platform has no foreground process group there is
            // nothing to watch; ticking forever to report `None` would cost a
            // thread per session for no answer.
            if !cfg!(unix) {
                return;
            }
            loop {
                std::thread::sleep(FOREGROUND_POLL);
                let leader = match master.lock() {
                    Ok(master) => foreground_leader(master.as_ref()),
                    // A poisoned lock means the manager panicked mid-resize;
                    // the stand-in is optional, so end quietly.
                    Err(_) => break,
                };
                if ctrl
                    .send(TermCmd::Foreground(foreground_status(leader, shell)))
                    .is_err()
                {
                    break;
                }
            }
        })
        .unwrap_or_else(|error| {
            // Losing the stand-in costs an unintegrated shell its status, not
            // the session — say so and carry on.
            tracing::warn!(
                %error,
                session = session.0.get(),
                "could not spawn the foreground watcher thread"
            );
            std::thread::spawn(|| {})
        })
}

/// The terminal thread: owns the `alacritty_terminal` grid and applies every
/// [`TermCmd`] (bytes → parse + status, resize, scroll), emitting a fresh
/// `Screen` each time. It exits — and reports [`PtyEvent::Exited`] — when the
/// reader signals EOF or every command sender is dropped.
pub(crate) fn spawn_term(
    session: SessionId,
    ctrl_rx: mpsc::Receiver<TermCmd>,
    size: (u16, u16),
    writer: SharedWriter,
    sink: EventSink,
    palette: Palette,
) -> JoinHandle<()> {
    let (cols, rows) = size;
    let term_sink = sink.clone();
    std::thread::Builder::new()
        .name(format!("pty-tm-{}", session.0.get()))
        .spawn(move || {
            // The responder writes cursor-report replies; the loop keeps its own
            // handle to forward wheel input to mouse-mode apps.
            let input = writer.clone();
            let mut term = Term::new(
                Config::default(),
                &TermSize::new(cols as usize, rows as usize),
                PtyResponder {
                    writer,
                    palette: palette.clone(),
                },
            );
            let mut parser: Processor = Processor::new();
            let mut activity = Activity::starting();
            let mut title: Option<String> = None;
            // Stays false when the loop ends without an EOF (every sender
            // dropped) — an unobserved exit is never a clean one.
            let mut clean = false;
            while let Ok(cmd) = ctrl_rx.recv() {
                match cmd {
                    TermCmd::Bytes(bytes) => {
                        // OSC status comes from the raw bytes — alacritty
                        // consumes the sequences, so decode before parsing.
                        let text = String::from_utf8_lossy(&bytes);
                        let signals = decode_chunk(&text);
                        let marks = decode_marks(&text);
                        let before = activity.status;
                        activity.observe(&signals, &marks);
                        report_status(session, before, &activity, &term_sink);
                        // Forward each OSC 9 notification's text to the OS,
                        // independent of the status fold above — the in-app
                        // `Attention` badge and the desktop alert are two
                        // surfaces of the same ping.
                        for signal in &signals {
                            if let OscSignal::Notification(body) = signal {
                                term_sink(PtyEvent::Notification {
                                    session,
                                    body: body.clone(),
                                });
                            }
                        }
                        // Follow Claude's reported title; the last title in
                        // the chunk wins, and only a change is forwarded.
                        if let Some(next) = signals.iter().rev().find_map(|s| match s {
                            OscSignal::Title(t) => Some(t),
                            _ => None,
                        }) && title.as_deref() != Some(next.as_str())
                        {
                            title = Some(next.clone());
                            term_sink(PtyEvent::Title {
                                session,
                                title: next.clone(),
                            });
                        }
                        parser.advance(&mut term, &bytes);
                    }
                    TermCmd::Resize(c, r) => {
                        term.resize(TermSize::new(c as usize, r as usize));
                    }
                    TermCmd::Scroll(target) => {
                        // A wheel turn over a mouse-mode / alt-scroll app is
                        // forwarded as input bytes; otherwise (and for the
                        // absolute jumps) it moves our own scrollback.
                        match target {
                            ScrollTarget::Wheel { col, row, lines } => {
                                match wheel_bytes(*term.mode(), col, row, lines) {
                                    Some(bytes) => {
                                        if let Ok(mut w) = input.lock()
                                            && let Err(error) = w.write_all(&bytes)
                                        {
                                            tracing::debug!(
                                                session = session.0.get(),
                                                %error,
                                                "wheel forward to PTY failed"
                                            );
                                        }
                                    }
                                    None => term.scroll_display(Scroll::Delta(lines)),
                                }
                            }
                            ScrollTarget::Top => term.scroll_display(Scroll::Top),
                            ScrollTarget::Bottom => term.scroll_display(Scroll::Bottom),
                        }
                    }
                    TermCmd::Select(op) => apply_select(&mut term, op),
                    TermCmd::CopySelection => {
                        // Read the text from the live selection, not a snapshot,
                        // so a fast drag's copy is exact. Commands are FIFO, so
                        // any queued Select ops have already been applied here.
                        if let Some(text) = term.selection_to_string() {
                            term_sink(PtyEvent::SelectionCopied { session, text });
                        }
                    }
                    TermCmd::Foreground(status) => {
                        let before = activity.status;
                        if activity.poll(status) {
                            report_status(session, before, &activity, &term_sink);
                        }
                        // A poll changes no pixels; falling through to the
                        // snapshot below would redraw every session four times
                        // a second for nothing.
                        continue;
                    }
                    TermCmd::Eof { clean: c } => {
                        clean = c;
                        // The waiter races the reader: the exit is observed on
                        // the process while the last output chunks may still be
                        // in flight. Give them a short quiet window — a crash
                        // message is exactly what the surviving screen must show.
                        loop {
                            match ctrl_rx.recv_timeout(EOF_DRAIN_QUIET) {
                                Ok(TermCmd::Bytes(bytes)) => parser.advance(&mut term, &bytes),
                                // Anything else — chiefly a foreground poll,
                                // which ticks four times a second — says
                                // nothing about a session that has already
                                // exited, but it must not *end* the drain:
                                // matching only `Bytes` here would abandon the
                                // trailing output this window exists to catch.
                                Ok(_) => continue,
                                Err(_) => break,
                            }
                        }
                        break;
                    }
                }
                term_sink(PtyEvent::Output {
                    session,
                    screen: snapshot(&term, &palette),
                });
            }
            // A final snapshot so any drained bytes reach the screen the tab
            // keeps showing on an unclean exit.
            term_sink(PtyEvent::Output {
                session,
                screen: snapshot(&term, &palette),
            });
            term_sink(PtyEvent::Exited { session, clean });
        })
        .unwrap_or_else(|_| {
            sink(PtyEvent::Exited {
                session,
                clean: false,
            });
            std::thread::spawn(|| {})
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer that banks everything written, so a test can read back the
    /// replies [`PtyResponder`] sends to the child process.
    #[derive(Clone)]
    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("capture lock").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Drive a real terminal thread over `chunks` and collect every status it
    /// reported — the wiring test for "bytes in, activity out", which no pure
    /// fold test can make: it is the terminal thread that decodes both dialects
    /// and decides what is worth emitting.
    fn statuses_reported(chunks: &[&str]) -> Vec<SessionStatus> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let collected = seen.clone();
        let sink: EventSink = Arc::new(move |event| {
            if let PtyEvent::Status { status, .. } = event {
                collected.lock().expect("sink lock").push(status);
            }
        });
        let writer: SharedWriter = Arc::new(Mutex::new(Box::new(CaptureWriter(Arc::new(
            Mutex::new(Vec::new()),
        )))));
        let session = SessionId(std::num::NonZeroU64::new(1).expect("nonzero"));
        let (ctrl, ctrl_rx) = mpsc::channel();
        let term = spawn_term(
            session,
            ctrl_rx,
            (80, 24),
            writer,
            sink,
            Palette::named("solarized-light").expect("known scheme"),
        );
        for chunk in chunks {
            ctrl.send(TermCmd::Bytes(chunk.as_bytes().to_vec()))
                .expect("terminal thread alive");
        }
        drop(ctrl);
        term.join().expect("terminal thread ends");
        seen.lock().expect("sink lock").clone()
    }

    #[test]
    fn a_shell_reporting_its_prompt_reaches_idle_through_the_terminal_thread() {
        // End of the chain the bug broke: a plain shell's integration marks
        // must come out the other side as activity, or `wait_for_status` on a
        // shell can only ever time out.
        assert_eq!(
            statuses_reported(&["\u{1b}]133;A\u{07}"]),
            vec![SessionStatus::Idle]
        );
    }

    #[test]
    fn a_shell_command_cycle_reports_busy_then_idle() {
        assert_eq!(
            statuses_reported(&[
                "\u{1b}]133;A\u{07}",
                "\u{1b}]133;C\u{07}",
                "output\r\n",
                "\u{1b}]133;D;0\u{07}\u{1b}]133;A\u{07}",
            ]),
            vec![
                SessionStatus::Idle,
                SessionStatus::Busy,
                SessionStatus::Idle
            ],
            "only real transitions are emitted — plain output is not one"
        );
    }

    #[test]
    fn claude_titles_still_report_activity_through_the_terminal_thread() {
        // The other dialect, unchanged: a spinner title is work, `✳` is a
        // prompt, and the OSC 9 ping outranks both.
        assert_eq!(
            statuses_reported(&[
                "\u{1b}]0;\u{280B} Thinking…\u{07}",
                "\u{1b}]0;\u{2733} claude\u{07}",
                "\u{1b}]9;allow Bash?\u{07}",
            ]),
            vec![
                SessionStatus::Busy,
                SessionStatus::Idle,
                SessionStatus::Attention
            ]
        );
    }

    #[test]
    fn a_background_colour_query_is_answered_from_the_palette() {
        // A CLI's theme auto-detection (Claude's `theme: auto`) sends OSC 11
        // and picks light or dark from the reply; unanswered, it assumes
        // dark — so a light palette must actually be reported.
        let captured = Arc::new(Mutex::new(Vec::new()));
        let writer: SharedWriter = Arc::new(Mutex::new(Box::new(CaptureWriter(captured.clone()))));
        let palette = Palette::named("solarized-light").expect("known scheme");
        let mut term = Term::new(
            Config::default(),
            &TermSize::new(20, 5),
            PtyResponder {
                writer,
                palette: palette.clone(),
            },
        );
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"\x1b]11;?\x1b\\");
        let reply =
            String::from_utf8(captured.lock().expect("capture lock").clone()).expect("utf8 reply");
        // Solarized-light background is 0xfdf6e3, reported in X11 rgb: form.
        assert!(
            reply.contains("rgb:fdfd/f6f6/e3e3"),
            "OSC 11 must report the palette background, got {reply:?}"
        );
    }

    #[test]
    fn a_foreground_colour_query_is_answered_from_the_palette() {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let writer: SharedWriter = Arc::new(Mutex::new(Box::new(CaptureWriter(captured.clone()))));
        let palette = Palette::named("solarized-light").expect("known scheme");
        let mut term = Term::new(
            Config::default(),
            &TermSize::new(20, 5),
            PtyResponder {
                writer,
                palette: palette.clone(),
            },
        );
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"\x1b]10;?\x1b\\");
        let reply =
            String::from_utf8(captured.lock().expect("capture lock").clone()).expect("utf8 reply");
        // Solarized-light foreground is 0x657b83.
        assert!(
            reply.contains("rgb:6565/7b7b/8383"),
            "OSC 10 must report the palette foreground, got {reply:?}"
        );
    }
}
