//! Where a session's activity comes from (FR8).
//!
//! Two dialects feed one status, because two kinds of session are hosted. A
//! Claude session announces itself in its own OSC stream (glyph titles, OSC 9
//! notifications — `termherd_claude::osc`). A plain shell says nothing of the
//! sort; it reports prompt and command boundaries in OSC 133, and only if its
//! integration snippet took (`crate::integration`). Both fold into the same
//! [`SessionStatus`] here, so the rest of the system has one notion of activity
//! whatever is running in the terminal.
//!
//! A shell whose integration did not take is not left mute: [`foreground_status`]
//! reads the activity off the PTY's own foreground process group. It is a
//! fallback, subordinate to the marks — see [`Activity`].

use termherd_claude::osc::OscSignal;
use termherd_core::SessionStatus;

use crate::prompt::PromptMark;

/// A session's activity and which source it is entitled to come from.
///
/// The sources are not equals. Signals and marks are the terminal's own account
/// of what it is doing; the foreground process group is an inference termherd
/// makes *about* it, and a coarse one. It would describe a Claude session as
/// permanently busy — the CLI never leaves the foreground — and a shell that
/// backgrounds its job as idle when it is not. So the poll is a stand-in only
/// while the terminal has said nothing for itself: the first signal or mark
/// retires it for the rest of the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Activity {
    /// The status as currently known.
    pub status: SessionStatus,
    /// Whether the terminal has ever classified itself — a Claude OSC signal or
    /// a shell-integration mark. While false, the foreground poll stands in.
    self_reported: bool,
}

impl Activity {
    /// A freshly spawned session: nothing classified yet, no source elected.
    pub(crate) fn starting() -> Self {
        Self {
            status: SessionStatus::Starting,
            self_reported: false,
        }
    }

    /// Fold one output chunk's signals and marks in. Marks are applied first:
    /// within a chunk, a shell's `133;C` is what *starts* the Claude CLI whose
    /// own signals follow it, so the more specific dialect must win the tie.
    pub(crate) fn observe(&mut self, signals: &[OscSignal], marks: &[PromptMark]) {
        self.status = fold_status(fold_marks(self.status, marks), signals);
        // A chunk that classified nothing — plain output, a bare title, a bell
        // — is not the terminal speaking for itself, so it must not retire the
        // stand-in.
        self.self_reported |= !marks.is_empty() || signals.iter().any(classifies);
    }

    /// Fold a foreground-process-group reading in, and report whether it moved
    /// the status. Ignored once the terminal has classified itself, and ignored
    /// when the platform cannot answer (`None`).
    pub(crate) fn poll(&mut self, foreground: Option<SessionStatus>) -> bool {
        let Some(status) = foreground.filter(|_| !self.self_reported) else {
            return false;
        };
        let moved = status != self.status;
        self.status = status;
        moved
    }
}

/// Whether a signal is the terminal classifying its own activity, as opposed to
/// decoration that rides the same wire (a title, a bell, an alt-screen toggle).
fn classifies(signal: &OscSignal) -> bool {
    matches!(
        signal,
        OscSignal::Busy | OscSignal::Idle | OscSignal::Notification(_)
    )
}

/// Fold a chunk's shell-integration marks into the running activity status: a
/// command running is work, a prompt or a finished command is a parked shell.
///
/// A parked shell does not clear a pending [`SessionStatus::Attention`] — the
/// same rule Claude's idle title obeys, and for the same reason: the user still
/// has to act, and a prompt redrawn underneath the request must not drop the
/// badge that says so.
fn fold_marks(current: SessionStatus, marks: &[PromptMark]) -> SessionStatus {
    let mut status = current;
    for mark in marks {
        status = match mark {
            PromptMark::Running => SessionStatus::Busy,
            _ if status == SessionStatus::Attention => SessionStatus::Attention,
            PromptMark::Ready | PromptMark::Done => SessionStatus::Idle,
        };
    }
    status
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
            // The title text drives the tab label, not the status.
            OscSignal::Title(_) | OscSignal::AltScreen(_) | OscSignal::Bell => status,
        };
    }
    status
}

/// The activity a PTY's foreground process group implies: the shell itself in
/// the foreground means nothing is running (`Idle`); any other process group
/// owns the terminal, so a command is (`Busy`).
///
/// `None` is "the platform cannot say" — ConPTY has no foreground process group,
/// so `portable_pty` reports none on Windows, and a session there stays on
/// whatever its marks said. Reporting an invented status would be worse than
/// reporting none: a caller would wait on it.
pub(crate) fn foreground_status(leader: Option<i32>, shell: Option<u32>) -> Option<SessionStatus> {
    let (leader, shell) = (leader?, i32::try_from(shell?).ok()?);
    Some(if leader == shell {
        SessionStatus::Idle
    } else {
        SessionStatus::Busy
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use SessionStatus::*;

    /// The status an activity reaches after observing one chunk.
    fn after(from: SessionStatus, signals: &[OscSignal], marks: &[PromptMark]) -> SessionStatus {
        let mut activity = Activity {
            status: from,
            self_reported: false,
        };
        activity.observe(signals, marks);
        activity.status
    }

    // --- Claude's dialect (unchanged behaviour, kept under the new seam) ---

    #[test]
    fn fold_status_tracks_busy_idle_attention() {
        // The last busy/idle marker in the chunk wins.
        assert_eq!(
            after(Starting, &[OscSignal::Busy, OscSignal::Idle], &[]),
            Idle
        );
        assert_eq!(after(Idle, &[OscSignal::Busy], &[]), Busy);
        // An OSC 9 notification means the CLI needs the user → Attention.
        assert_eq!(
            after(Busy, &[OscSignal::Notification("x".into())], &[]),
            Attention
        );
        // Attention is sticky against a bare idle prompt, but Busy clears it.
        assert_eq!(after(Attention, &[OscSignal::Idle], &[]), Attention);
        assert_eq!(after(Attention, &[OscSignal::Busy], &[]), Busy);
        // Bells and alt-screen toggles leave the status unchanged.
        assert_eq!(
            after(Busy, &[OscSignal::Bell, OscSignal::AltScreen(true)], &[]),
            Busy
        );
        // No signals at all keeps the current status.
        assert_eq!(after(Starting, &[], &[]), Starting);
    }

    #[test]
    fn notification_still_means_attention_alongside_the_osc9_body_forwarding() {
        // The notification *text* is forwarded to the OS on a separate channel;
        // the status fold must be untouched — an OSC 9 among other signals
        // still resolves to Attention, and its body never leaks into the title.
        let signals = [
            OscSignal::Busy,
            OscSignal::Notification("permission: allow Bash?".into()),
            OscSignal::Title("ignored".into()),
        ];
        assert_eq!(after(Starting, &signals, &[]), Attention);
        // An empty-bodied notification is just as much an attention request.
        assert_eq!(
            after(Idle, &[OscSignal::Notification(String::new())], &[]),
            Attention
        );
    }

    // --- The shell's dialect ---

    #[test]
    fn a_shell_reaching_its_prompt_leaves_starting() {
        // The bug this exists for: a shell sitting at its prompt reported
        // `starting` forever, so `wait_for_status` could only ever time out.
        assert_eq!(after(Starting, &[], &[PromptMark::Ready]), Idle);
    }

    #[test]
    fn a_shell_running_a_command_is_busy_until_it_finishes() {
        assert_eq!(after(Idle, &[], &[PromptMark::Running]), Busy);
        assert_eq!(after(Busy, &[], &[PromptMark::Done]), Idle);
    }

    #[test]
    fn a_whole_command_cycle_in_one_chunk_ends_parked() {
        // Fast commands land their whole cycle in a single read.
        assert_eq!(
            after(
                Idle,
                &[],
                &[PromptMark::Running, PromptMark::Done, PromptMark::Ready]
            ),
            Idle
        );
    }

    #[test]
    fn a_prompt_mark_does_not_clear_a_pending_attention() {
        // Same rule as Claude's idle title: the user still has to act, so only
        // work resuming clears it. Otherwise a shell prompt redrawn under a
        // permission request would silently drop the badge.
        assert_eq!(after(Attention, &[], &[PromptMark::Ready]), Attention);
        assert_eq!(after(Attention, &[], &[PromptMark::Done]), Attention);
        assert_eq!(after(Attention, &[], &[PromptMark::Running]), Busy);
    }

    #[test]
    fn claude_signals_win_over_the_shell_mark_that_started_them() {
        // Launching Claude is a command: the shell writes `133;C`, then the CLI
        // writes its own idle title. Both land in one chunk, and the CLI's
        // account of itself is the specific one.
        assert_eq!(
            after(Starting, &[OscSignal::Idle], &[PromptMark::Running]),
            Idle
        );
    }

    // --- The foreground-process fallback ---

    #[test]
    fn the_shell_in_the_foreground_means_nothing_is_running() {
        assert_eq!(foreground_status(Some(4321), Some(4321)), Some(Idle));
    }

    #[test]
    fn another_process_group_in_the_foreground_means_a_command_is_running() {
        assert_eq!(foreground_status(Some(4399), Some(4321)), Some(Busy));
    }

    #[test]
    fn a_platform_that_cannot_report_a_foreground_group_says_nothing() {
        // ConPTY has no such notion; inventing a status would strand a caller
        // waiting on it.
        assert_eq!(foreground_status(None, Some(4321)), None);
        assert_eq!(foreground_status(Some(4321), None), None);
    }

    #[test]
    fn a_poll_moves_an_unintegrated_session_and_reports_that_it_did() {
        let mut activity = Activity::starting();
        assert!(activity.poll(Some(Busy)), "the status moved");
        assert_eq!(activity.status, Busy);
        assert!(!activity.poll(Some(Busy)), "an unchanged poll is not news");
    }

    #[test]
    fn a_poll_that_cannot_read_the_platform_changes_nothing() {
        let mut activity = Activity::starting();
        assert!(!activity.poll(None));
        assert_eq!(activity.status, Starting);
    }

    #[test]
    fn a_shell_that_has_reported_a_mark_stops_listening_to_the_poll() {
        // The marks are the shell's own account; the poll is a coarse inference
        // about it. Once the precise source speaks, the coarse one must not be
        // able to contradict it — a backgrounded job would otherwise flip the
        // session to idle while the shell is still busy, or the reverse.
        let mut activity = Activity::starting();
        activity.observe(&[], &[PromptMark::Running]);
        assert_eq!(activity.status, Busy);
        assert!(
            !activity.poll(Some(Idle)),
            "the poll is no longer consulted"
        );
        assert_eq!(activity.status, Busy);
    }

    #[test]
    fn a_claude_session_stops_listening_to_the_poll_once_the_cli_speaks() {
        // `claude` never leaves the foreground, so the poll would call every
        // Claude session busy forever — including one blocked on a permission
        // prompt, whose badge the user needs.
        let mut activity = Activity::starting();
        activity.observe(&[OscSignal::Notification("allow?".into())], &[]);
        assert_eq!(activity.status, Attention);
        assert!(!activity.poll(Some(Busy)));
        assert_eq!(activity.status, Attention);
    }

    #[test]
    fn a_chunk_that_classifies_nothing_leaves_the_poll_in_charge() {
        // Plain output, a bare title, a bell: none of them is the terminal
        // saying what it is doing, so the stand-in must survive them.
        let mut activity = Activity::starting();
        activity.observe(&[OscSignal::Title("zsh".into()), OscSignal::Bell], &[]);
        assert!(activity.poll(Some(Busy)), "the poll still stands in");
        assert_eq!(activity.status, Busy);
    }
}
