//! Whether a [`ClaudeCommand`] may be typed into a session now, and the writes
//! that type it. One predicate answers both the arming of a confirmation and
//! the send after it, so a session that turned busy while the prompt was up is
//! refused by the same rule that would have refused it at the start.

use std::fmt;

use crate::claude_command::ClaudeCommand;

use super::*;

/// Why a session cannot take a Claude command right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRefusal {
    /// No live session has this id.
    UnknownSession,
    /// The session runs a shell, which would execute the line, not read it.
    NotClaude,
    /// Claude is not waiting at its prompt: the line would be queued behind
    /// running work, or would answer a permission prompt instead.
    NotIdle(SessionStatus),
}

impl fmt::Display for CommandRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSession => f.write_str("no live session has that handle"),
            Self::NotClaude => f.write_str("the session is not a Claude session"),
            Self::NotIdle(status) => {
                let status = match status {
                    SessionStatus::Starting => "still starting",
                    SessionStatus::Busy => "busy",
                    SessionStatus::Idle => "idle",
                    SessionStatus::Attention => "waiting for an answer",
                    SessionStatus::Exited => "exited",
                };
                write!(f, "the session is {status}, not idle at its prompt")
            }
        }
    }
}

impl App {
    /// Whether `session` can take a Claude command now: a Claude launch, idle
    /// at its prompt.
    ///
    /// # Errors
    ///
    /// The [`CommandRefusal`] naming the first condition that fails.
    pub fn claude_command_check(&self, session: SessionId) -> Result<(), CommandRefusal> {
        let live = self
            .sessions
            .get(&session)
            .ok_or(CommandRefusal::UnknownSession)?;
        if !live.is_claude_launch() {
            return Err(CommandRefusal::NotClaude);
        }
        match live.status {
            SessionStatus::Idle => Ok(()),
            other => Err(CommandRefusal::NotIdle(other)),
        }
    }

    /// Type `command` into `session` if it can take one now, else nothing.
    pub(super) fn send_claude_command(
        &self,
        session: SessionId,
        command: &ClaudeCommand,
    ) -> Vec<Effect> {
        if self.claude_command_check(session).is_err() {
            return Vec::new();
        }
        command
            .keystrokes()
            .into_iter()
            .map(|bytes| Effect::Write { session, bytes })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::*;
    use crate::claude_command::ClaudeColor;

    fn launch_claude(app: &mut App) -> SessionId {
        match app
            .apply(Event::LaunchSession(LaunchSpec {
                cwd: None,
                launch: Launch::Claude(ClaudeLaunch::Fresh(None)),
                title: "claude".into(),
            }))
            .as_slice()
        {
            [Effect::Spawn(spec)] => spec.session,
            other => panic!("expected Spawn, got {other:?}"),
        }
    }

    fn set_status(app: &mut App, session: SessionId, status: SessionStatus) {
        app.apply(Event::StatusChanged { session, status });
    }

    fn send(app: &mut App, session: SessionId) -> Vec<Effect> {
        app.apply(Event::SendClaudeCommand {
            session,
            command: ClaudeCommand::Color(ClaudeColor::Pink),
        })
    }

    #[test]
    fn an_idle_claude_takes_the_command_as_two_writes() {
        let mut app = App::new();
        let session = launch_claude(&mut app);
        set_status(&mut app, session, SessionStatus::Idle);

        assert_eq!(app.claude_command_check(session), Ok(()));
        let writes: Vec<_> = send(&mut app, session)
            .into_iter()
            .map(|effect| match effect {
                Effect::Write { session: to, bytes } => (to, bytes),
                other => panic!("expected a write, got {other:?}"),
            })
            .collect();
        assert_eq!(
            writes,
            vec![
                (session, b"\x15/color pink".to_vec()),
                (session, b"\r".to_vec()),
            ]
        );
    }

    #[test]
    fn a_claude_not_at_its_prompt_is_refused_with_its_status() {
        let mut app = App::new();
        let session = launch_claude(&mut app);
        assert_eq!(
            app.claude_command_check(session),
            Err(CommandRefusal::NotIdle(SessionStatus::Starting)),
            "a fresh launch has not reached its prompt"
        );
        for status in [
            SessionStatus::Busy,
            SessionStatus::Attention,
            SessionStatus::Exited,
        ] {
            set_status(&mut app, session, status);
            assert_eq!(
                app.claude_command_check(session),
                Err(CommandRefusal::NotIdle(status))
            );
            assert!(
                send(&mut app, session).is_empty(),
                "{status:?} sends nothing"
            );
        }
    }

    #[test]
    fn a_shell_is_refused_even_when_idle() {
        // A shell would run `/color pink` as a program, not read it.
        let mut app = App::new();
        let session = launch(&mut app, "sh");
        set_status(&mut app, session, SessionStatus::Idle);
        assert_eq!(
            app.claude_command_check(session),
            Err(CommandRefusal::NotClaude)
        );
        assert!(send(&mut app, session).is_empty());
    }

    #[test]
    fn an_unknown_session_is_refused() {
        let mut app = App::new();
        assert_eq!(
            app.claude_command_check(sid(42)),
            Err(CommandRefusal::UnknownSession)
        );
        assert!(send(&mut app, sid(42)).is_empty());
    }

    #[test]
    fn a_session_that_turned_busy_after_the_check_is_refused_at_the_send() {
        // The confirmation sits between the two: the send re-asks rather than
        // trusting an answer given before the user decided.
        let mut app = App::new();
        let session = launch_claude(&mut app);
        set_status(&mut app, session, SessionStatus::Idle);
        assert_eq!(app.claude_command_check(session), Ok(()));
        set_status(&mut app, session, SessionStatus::Busy);
        assert!(send(&mut app, session).is_empty());
    }

    #[test]
    fn the_refusal_names_the_status_in_words() {
        assert_eq!(
            CommandRefusal::NotIdle(SessionStatus::Busy).to_string(),
            "the session is busy, not idle at its prompt"
        );
        assert_eq!(
            CommandRefusal::NotClaude.to_string(),
            "the session is not a Claude session"
        );
    }
}
