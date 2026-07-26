//! The bridge-serving seam: how the shell answers one external [`Request`].
//!
//! Split from the shell's state machine because answering is not one thing. A
//! read answers off the state the shell already owns; a mutation needs `&mut`
//! and the effect executor, so it cannot go through the read-only responder.
//! Keeping that fork here — rather than inline in `update` — leaves one
//! auditable place where an external caller meets `core::App`.

use iced::Task;
use iced::window::{self, Screenshot};

use termherd_core::SessionStatus;
use termherd_core::snapshot::tail_lines;
use termherd_core::workspace::SessionId;

use super::bridge::{self, ReplyPort, Request, ShotResult, TerminalRead, WaitOutcome};
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
        // Waiters are settled by status events, so a quiet workspace never
        // sweeps them. Do it here too: this is the one path that runs on every
        // external call, however still the sessions are.
        self.sweep_waiters();
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
            // Pixels come from the window, not from state: this one answers
            // from inside the task it returns, once iced hands the frame over.
            Request::Screenshot { max_width } => Self::serve_screenshot(max_width, reply),
            // The remaining read requests answer straight from owned state.
            read => {
                let inputs = self.snapshot_inputs(&read);
                reply.answer(bridge::respond(&self.core, &read, &inputs));
                Task::none()
            }
        }
    }

    /// Ask iced for the window's pixels and answer `reply` once they arrive.
    ///
    /// The reply port rides along inside the task instead of parking in a
    /// waiter list: unlike a wait, nothing the shell will later observe decides
    /// this answer — it is simply not ready in this `update`. A window-less run
    /// yields no frame and answers with the reason, so the caller learns why
    /// rather than waiting out its bound.
    ///
    /// The fit + encode run inside the async block, so they are off the winit
    /// thread — a multi-megapixel resample and PNG encode there would stall the
    /// very frames the caller is trying to photograph. They are still
    /// synchronous work occupying one bridge-runtime worker for their duration;
    /// the runtime is multi-threaded, so that costs concurrency, not liveness.
    ///
    /// One gap the reason-in-words degradation does not cover: a window that
    /// disappears *between* `latest` and `screenshot` (a quit mid-capture)
    /// leaves the iced oneshot unanswered, so this task never resumes and the
    /// caller falls back on its own `SCREENSHOT_TIMEOUT`. Bounded, never a
    /// hang (Q7) — but a timeout rather than an explanation.
    fn serve_screenshot(max_width: u32, reply: ReplyPort) -> Task<Message> {
        window::latest()
            .then(|window| match window {
                Some(id) => window::screenshot(id).map(Some),
                None => Task::done(None),
            })
            .then(move |shot| {
                let reply = reply.clone();
                Task::future(async move {
                    reply.answer(bridge::Reply::Shot(shot_reply(shot.as_ref(), max_width)));
                })
                .discard()
            })
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
        // Settled already? Either it sits on a target, or it has exited — and
        // `Exited` is terminal in `core`, which refuses to overwrite it, so a
        // wait parked on a dead session could never be woken by a status event.
        if status.is_some_and(|status| settles(&targets, status)) {
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

    /// Settle every waiter watching `session` now that its activity moved, and
    /// sweep the ones whose caller gave up. Called from the two updates that can
    /// move a session's activity: a status change and a PTY exit — a crash emits
    /// no status change, so without the second a caller would wait out its whole
    /// bound on a dead terminal.
    ///
    /// The settling status is read back from `core`, never taken from the
    /// message: `core` refuses to overwrite `Exited`, so a status still in
    /// flight when the exit landed would otherwise report a dead session as
    /// idle. A session gone from the registry (a clean exit closed its pane)
    /// reads as `Exited`.
    pub(super) fn settle_waiters(&mut self, session: SessionId) {
        let status = self.status_of(session);
        self.waiters.retain(|waiter| {
            let settled = waiter.session == session && settles(&waiter.targets, status);
            if settled {
                waiter.reply.answer(bridge::Reply::Waited(WaitOutcome {
                    status: Some(status),
                    error: None,
                }));
            }
            !settled && !waiter.reply.abandoned()
        });
    }

    /// Drop waiters no one is listening for, and settle any whose session has
    /// left the registry — a tab closed from the UI removes its sessions without
    /// a PTY exit ever reaching `update`, which would otherwise leave the caller
    /// parked on a session that no longer exists.
    fn sweep_waiters(&mut self) {
        let vanished: Vec<SessionId> = self
            .waiters
            .iter()
            .map(|waiter| waiter.session)
            .filter(|id| !self.core.sessions.contains_key(id))
            .collect();
        for session in vanished {
            self.settle_waiters(session);
        }
        self.waiters.retain(|waiter| !waiter.reply.abandoned());
    }

    /// A session's activity as `core` records it, or `Exited` when it is no
    /// longer registered — the one place a gone session is read as dead.
    fn status_of(&self, session: SessionId) -> SessionStatus {
        self.core
            .sessions
            .get(&session)
            .map_or(SessionStatus::Exited, |live| live.status)
    }
}

/// Whether `status` settles a wait on `targets`. An exit always does, whatever
/// was asked for: `Exited` is terminal in `core`, so the session will never
/// reach the target and holding the caller would only burn its bound to reach
/// the same conclusion.
fn settles(targets: &[SessionStatus], status: SessionStatus) -> bool {
    targets.contains(&status) || status == SessionStatus::Exited
}

/// Shape a window frame into a [`ShotResult`]: fit it to `max_width`, resample
/// when that shrinks it, and encode PNG bytes. Pure over `Option<Screenshot>`,
/// which is the whole point — the part of the screenshot path that needs a real
/// window is reduced to fetching the frame, and every rule about size,
/// degradation and encoding is testable headlessly.
///
/// `None` is the headless / window-less run: an explanatory result, never a
/// panic, pointing the caller at the text snapshot that still works.
fn shot_reply(shot: Option<&Screenshot>, max_width: u32) -> ShotResult {
    let Some(shot) = shot else {
        return ShotResult::failed("no window to screenshot (a headless or not-yet-mapped run)");
    };
    let (source_width, source_height) = (shot.size.width, shot.size.height);
    // `fit_width` owns "is there an image to make?"; re-deciding it here would
    // be the same invariant in two places, free to drift apart.
    let Some((width, height)) = crate::image::fit_width(source_width, source_height, max_width)
    else {
        return ShotResult::failed("the window reported no pixels");
    };
    // Resample only when the fit actually shrinks the frame — a window already
    // inside the bound is encoded from its own pixels, untouched.
    let fitted;
    let pixels = if (width, height) == (source_width, source_height) {
        &shot.rgba[..]
    } else {
        fitted =
            crate::image::resample_nearest(&shot.rgba, source_width, source_height, width, height);
        &fitted[..]
    };
    match crate::image::encode_png(pixels, width, height) {
        Ok(png) => ShotResult {
            png: Some(png),
            width,
            height,
            error: None,
        },
        Err(error) => ShotResult::failed(format!("could not encode the screenshot: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A solid-colour RGBA frame of `w`×`h`, as iced would hand one over.
    fn frame(w: u32, h: u32) -> Screenshot {
        Screenshot::new(
            vec![0x40u8; (w * h * 4) as usize],
            iced::Size::new(w, h),
            1.0,
        )
    }

    /// The PNG dimensions a decoder reads back out of the encoded bytes — the
    /// only claim that proves the image is really the size we reported.
    fn decoded_dims(png: &[u8]) -> (u32, u32) {
        let reader = png::Decoder::new(png).read_info().expect("decodable png");
        (reader.info().width, reader.info().height)
    }

    #[test]
    fn a_frame_under_the_bound_is_encoded_untouched() {
        let shot = shot_reply(Some(&frame(120, 80)), 1200);
        let png = shot.png.expect("pixels");
        assert_eq!((shot.width, shot.height), (120, 80));
        assert_eq!(
            decoded_dims(&png),
            (120, 80),
            "the encoded image matches the reported size"
        );
        assert_eq!(shot.error, None);
    }

    #[test]
    fn a_frame_over_the_bound_is_downscaled_keeping_its_ratio() {
        let shot = shot_reply(Some(&frame(3000, 2000)), 1200);
        assert_eq!((shot.width, shot.height), (1200, 800));
        assert_eq!(decoded_dims(&shot.png.expect("pixels")), (1200, 800));
    }

    #[test]
    fn a_window_less_run_reports_why_instead_of_panicking() {
        // The headless degradation: no window means no pixels, and the caller
        // must learn that in words — the text snapshot is still available.
        let shot = shot_reply(None, 1200);
        assert!(shot.png.is_none(), "no window, no image");
        let reason = shot.error.expect("a reason");
        assert!(
            reason.contains("window"),
            "the reason should name the missing window, got {reason:?}"
        );
    }

    #[test]
    fn the_debug_form_reports_the_payload_length_not_the_payload() {
        // A `Reply` reaches logs and assertion failures; dumping megabytes of
        // PNG into either is a footgun, so `Debug` must summarise.
        let shot = shot_reply(Some(&frame(8, 8)), 1200);
        let rendered = format!("{shot:?}");
        assert!(
            rendered.contains("png_bytes: Some("),
            "Debug should report the byte count, got {rendered}"
        );
        assert!(
            rendered.len() < 200,
            "Debug must not inline the pixels, got {} chars",
            rendered.len()
        );
    }
}
