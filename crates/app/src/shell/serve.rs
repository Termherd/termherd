//! The bridge-serving seam: how the shell answers one external [`Request`].
//!
//! Split from the shell's state machine because answering is not one thing. A
//! read answers off the state the shell already owns; a mutation needs `&mut`
//! and the effect executor, so it cannot go through the read-only responder.
//! Keeping that fork here — rather than inline in `update` — leaves one
//! auditable place where an external caller meets `core::App`.

use iced::Task;

use termherd_core::SessionStatus;
use termherd_core::snapshot::tail_lines;
use termherd_core::workspace::SessionId;

use super::bridge::{self, ReplyPort, Request, TerminalRead, WaitOutcome};
use super::{Message, Shell};

/// One bridge caller parked on a session reaching a target activity. Held by
/// the shell between the `update` that served the request and the one that
/// carries the status change (or the exit) settling it.
pub(super) struct StatusWaiter {
    /// The session being watched.
    pub session: SessionId,
    /// Any of these activities settles the wait.
    pub targets: Vec<SessionStatus>,
    /// Where the answer goes when it does.
    pub reply: ReplyPort,
}

impl Shell {
    /// Answer one bridge request and return any async follow-up it needs.
    pub(super) fn serve(&mut self, request: Request, reply: ReplyPort) -> Task<Message> {
        match request {
            // Actions mutate, so they can't answer off a `&App`: apply them and
            // perform the effects here, where the shell owns both.
            Request::Act(action) => {
                let (outcome, task) = self.perform_action(action);
                reply.answer(bridge::Reply::Acted(outcome));
                task
            }
            // A wait may answer now or park; either way it needs no follow-up.
            Request::WaitForStatus { session, targets } => {
                self.serve_wait(session, targets, reply);
                Task::none()
            }
            // A terminal read needs the `pty` adapter's screens, which the
            // read-only responder over `&App` cannot see.
            Request::ReadTerminal { session, lines } => {
                reply.answer(bridge::Reply::Terminal(self.read_terminal(session, lines)));
                Task::none()
            }
            // The remaining read requests answer straight from owned state.
            read => {
                let inputs = self.snapshot_inputs(&read);
                reply.answer(bridge::respond(&self.core, &read, &inputs));
                Task::none()
            }
        }
    }

    /// Answer a wait now when it is already settled — an unknown handle, or a
    /// session sitting on a target — otherwise park it. Answering an
    /// already-satisfied wait matters: a caller asking "tell me when it is idle"
    /// about an idle session would otherwise wait out its own bound having
    /// missed the transition it asked about.
    fn serve_wait(&mut self, session: u64, targets: Vec<SessionStatus>, reply: ReplyPort) {
        let Some(id) = self.resolve(session) else {
            reply.answer(bridge::Reply::Waited(WaitOutcome {
                status: None,
                error: Some(format!("no live session with handle {session}")),
            }));
            return;
        };
        let status = self.core.sessions.get(&id).map(|live| live.status);
        if status.is_some_and(|status| targets.contains(&status)) {
            reply.answer(bridge::Reply::Waited(WaitOutcome {
                status,
                error: None,
            }));
            return;
        }
        self.waiters.push(StatusWaiter {
            session: id,
            targets,
            reply,
        });
    }

    /// The visible text of one session's terminal, trimmed to its last `lines`.
    /// The three outcomes stay distinct — unknown handle, live-but-unrendered,
    /// and text — because an agent acts differently on each.
    fn read_terminal(&self, session: u64, lines: usize) -> TerminalRead {
        let Some(id) = self.resolve(session) else {
            return TerminalRead {
                text: None,
                error: Some(format!("no live session with handle {session}")),
            };
        };
        TerminalRead {
            // `core` owns the truncation rule; a read borrows it rather than
            // growing a second one that could drift from a snapshot's.
            text: self
                .screens
                .get(&id)
                .map(|screen| tail_lines(&screen.text(), lines)),
            error: None,
        }
    }

    /// Settle every waiter watching `session` now that it reached `status`, and
    /// sweep the ones whose caller gave up. Called from the two updates that can
    /// move a session's activity: a status change and a PTY exit — a crash emits
    /// no status change, so without the second a caller would wait out its whole
    /// bound on a dead terminal.
    pub(super) fn settle_waiters(&mut self, session: SessionId, status: SessionStatus) {
        self.waiters.retain(|waiter| {
            // A target reached settles the wait — and so does an exit, whatever
            // was asked for: a dead session will never reach it, so holding the
            // caller would only burn its bound to reach the same conclusion.
            let settled = waiter.session == session
                && (waiter.targets.contains(&status) || status == SessionStatus::Exited);
            if settled {
                waiter.reply.answer(bridge::Reply::Waited(WaitOutcome {
                    status: Some(status),
                    error: None,
                }));
            }
            !settled && !waiter.reply.abandoned()
        });
    }
}
