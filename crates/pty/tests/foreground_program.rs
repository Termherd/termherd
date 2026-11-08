//! A real shell running a program in its foreground must read as running to
//! the close and quit confirmations, even after the terminal's own account of
//! its activity has stopped saying so. Reading the PTY's foreground process
//! group is a Unix notion — ConPTY has none — so the whole file is.
#![cfg(unix)]

use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use termherd_core::ports::PtyHost;
use termherd_core::{App, Effect, Event, Launch, LaunchSpec, Placement, SessionStatus};
use termherd_pty::{EventSink, Palette, PrivateFiles, PtyEvent, PtyManager, Shell};

/// Claude's idle title, which retires the foreground poll as a source of
/// status: what a `claude` typed by hand into this shell leaves behind.
const CLAUDE_IDLE_TITLE: &[u8] = b"printf '\\033]0;\\342\\234\\263 Claude Code\\007'\r";

/// Feed one adapter event to `app`, as the GUI's glue does.
fn apply(app: &mut App, event: PtyEvent) {
    let event = match event {
        PtyEvent::Status { session, status } => Event::StatusChanged { session, status },
        PtyEvent::ForegroundJob { session, job } => Event::ForegroundJobChanged { session, job },
        PtyEvent::AltScreen { session, on } => Event::AltScreenChanged { session, on },
        PtyEvent::Exited { session, clean } => Event::PtyExited { session, clean },
        _ => return,
    };
    app.apply(event);
}

/// Pump events into `app` until `holds` reads true of it, or 15 s pass.
fn pump_until(
    app: &mut App,
    events: &mpsc::Receiver<PtyEvent>,
    holds: impl Fn(&App) -> bool,
) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if holds(app) {
            return true;
        }
        match events.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => apply(app, event),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    holds(app)
}

#[test]
fn a_shell_running_a_foreground_job_reads_as_running_until_it_is_back_at_its_prompt() {
    let (tx, events) = mpsc::channel::<PtyEvent>();
    let sink: EventSink = Arc::new(move |event| {
        let _ = tx.send(event);
    });
    let private = tempfile::tempdir().expect("tempdir");
    let sh = Shell {
        program: "/bin/sh".to_owned(),
        args: Vec::new(),
    };
    let host = PtyManager::new(
        sink,
        Some(sh),
        Palette::default(),
        PrivateFiles::new(private.path()),
    );
    let mut app = App::new();
    let spawn = app.apply(Event::LaunchSession(LaunchSpec {
        cwd: None,
        launch: Launch::Shell,
        title: "sh".to_owned(),
        placement: Placement::Foreground,
    }));
    let spec = spawn
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Spawn(spec) => Some(spec),
            _ => None,
        })
        .expect("a launch spawns a PTY");
    let session = spec.session;
    host.spawn(spec).expect("spawn");

    let shell_in_front = |app: &App| {
        app.sessions
            .get(&session)
            .is_some_and(|live| live.foreground_known && live.foreground.is_none())
    };
    assert!(
        pump_until(&mut app, &events, shell_in_front),
        "the shell at its prompt must be read as in front"
    );
    assert!(!app.tab_has_running_process(0), "a shell at its prompt");

    host.write(session, CLAUDE_IDLE_TITLE).expect("write");
    let idle = |app: &App| {
        app.sessions
            .get(&session)
            .is_some_and(|live| live.status == SessionStatus::Idle)
    };
    assert!(pump_until(&mut app, &events, idle), "the title reads idle");

    host.write(session, b"sleep 30\r").expect("write");
    assert!(
        pump_until(&mut app, &events, |app| app.tab_has_running_process(0)),
        "a shell running `sleep` in its foreground must read as running"
    );
    assert!(app.any_running_process(), "and a quit must confirm too");

    host.write(session, b"\x03").expect("write");
    assert!(
        pump_until(&mut app, &events, |app| !app.tab_has_running_process(0)),
        "back at its prompt the shell must no longer read as running"
    );
    assert!(shell_in_front(&app));

    // `exit 0`: a bare `exit` returns whatever status the startup files left.
    host.write(session, b"exit 0\r").expect("write");
    let exited = |app: &App| !app.sessions.contains_key(&session);
    assert!(pump_until(&mut app, &events, exited), "the shell exits");
    host.kill(session).expect("kill releases the dead session");
}
