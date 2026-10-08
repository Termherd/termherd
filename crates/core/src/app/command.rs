//! Whether a [`ClaudeCommand`] may be typed into a session now, and the writes
//! that type it. One predicate answers both the arming of a confirmation and
//! the send after it, so a session that turned busy while the prompt was up is
//! refused by the same rule that would have refused it at the start.
//!
//! The status alone does not prove the keyboard reaches Claude's text input: a
//! picker or a dialog can be up while Claude reads as idle, and a draft can sit
//! in the prompt. So the check also takes what the screen shows of the prompt
//! ([`PromptInput`]), read by the shell, which holds the screens.

use std::fmt;

use crate::claude_command::{ClaudeCommand, PromptInput};

use super::*;

/// Why a session cannot take a Claude command right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandRefusal {
    /// No live session has this id.
    UnknownSession,
    /// The session runs a shell, which would execute the line, not read it.
    NotClaude,
    /// Claude is not waiting at its prompt: the line would be queued behind
    /// running work, or would answer a permission prompt instead.
    NotIdle(SessionStatus),
    /// Claude's prompt holds a draft, which the command would be typed into.
    /// Carries the draft, so the user can see what is in the way.
    DraftPresent(String),
    /// No input prompt is on screen — a menu or dialog has the keyboard, or
    /// the view is scrolled — so Enter would answer that instead.
    PromptNotVisible,
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
            Self::DraftPresent(draft) => write!(
                f,
                "Claude's prompt holds a draft ({draft:?}); clear or send it first"
            ),
            Self::PromptNotVisible => f.write_str(
                "Claude's input prompt is not on screen (a menu or dialog may be open, \
                 or the view is scrolled)",
            ),
        }
    }
}

impl App {
    /// Whether `session` can take a Claude command now: a Claude launch, idle,
    /// with `prompt` — its input as the screen shows it — empty.
    ///
    /// # Errors
    ///
    /// The [`CommandRefusal`] naming the first condition that fails.
    pub fn claude_command_check(
        &self,
        session: SessionId,
        prompt: &PromptInput,
    ) -> Result<(), CommandRefusal> {
        let live = self
            .sessions
            .get(&session)
            .ok_or(CommandRefusal::UnknownSession)?;
        if !live.runs_claude() {
            return Err(CommandRefusal::NotClaude);
        }
        if live.status != SessionStatus::Idle {
            return Err(CommandRefusal::NotIdle(live.status));
        }
        match prompt {
            PromptInput::Empty => Ok(()),
            PromptInput::Draft(draft) => Err(CommandRefusal::DraftPresent(draft.clone())),
            PromptInput::NotVisible => Err(CommandRefusal::PromptNotVisible),
        }
    }

    /// Type `command` into `session` if it can take one now, else nothing.
    pub(super) fn send_claude_command(
        &self,
        session: SessionId,
        command: &ClaudeCommand,
        prompt: &PromptInput,
    ) -> Vec<Effect> {
        if self.claude_command_check(session, prompt).is_err() {
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

    fn set_status(app: &mut App, session: SessionId, status: SessionStatus) {
        app.apply(Event::StatusChanged { session, status });
    }

    fn send(app: &mut App, session: SessionId) -> Vec<Effect> {
        app.apply(Event::SendClaudeCommand {
            session,
            command: ClaudeCommand::Color(ClaudeColor::Pink),
            prompt: PromptInput::Empty,
        })
    }

    fn check(app: &App, session: SessionId) -> Result<(), CommandRefusal> {
        app.claude_command_check(session, &PromptInput::Empty)
    }

    #[test]
    fn an_idle_claude_with_a_draft_or_no_prompt_on_screen_is_refused() {
        let mut app = App::new();
        let session = launch_claude(&mut app);
        set_status(&mut app, session, SessionStatus::Idle);
        let draft = PromptInput::Draft("fix the\nlogin bug".to_owned());
        assert_eq!(
            app.claude_command_check(session, &draft),
            Err(CommandRefusal::DraftPresent(
                "fix the\nlogin bug".to_owned()
            ))
        );
        assert_eq!(
            app.claude_command_check(session, &PromptInput::NotVisible),
            Err(CommandRefusal::PromptNotVisible)
        );
        for prompt in [draft, PromptInput::NotVisible] {
            let effects = app.apply(Event::SendClaudeCommand {
                session,
                command: ClaudeCommand::Desktop,
                prompt: prompt.clone(),
            });
            assert!(effects.is_empty(), "{prompt:?} sends nothing");
        }
    }

    #[test]
    fn an_idle_claude_takes_the_command_as_two_writes() {
        let mut app = App::new();
        let session = launch_claude(&mut app);
        set_status(&mut app, session, SessionStatus::Idle);

        assert_eq!(check(&app, session), Ok(()));
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
            check(&app, session),
            Err(CommandRefusal::NotIdle(SessionStatus::Starting)),
            "a fresh launch has not reached its prompt"
        );
        for status in [
            SessionStatus::Busy,
            SessionStatus::Attention,
            SessionStatus::Exited,
        ] {
            set_status(&mut app, session, status);
            assert_eq!(check(&app, session), Err(CommandRefusal::NotIdle(status)));
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
        assert_eq!(check(&app, session), Err(CommandRefusal::NotClaude));
        assert!(send(&mut app, session).is_empty());
    }

    #[test]
    fn a_claude_launch_whose_claude_exited_is_refused_as_a_shell() {
        // Its shell's own prompt marks make it idle, and Claude's last frame
        // can still read as an empty prompt: only the foreground tells.
        let mut app = App::new();
        let session = launch_claude(&mut app);
        let job = ForegroundJob {
            pid: 7,
            started: None,
        };
        app.apply(Event::ForegroundJobChanged {
            session,
            job: Some(job),
        });
        set_status(&mut app, session, SessionStatus::Idle);
        assert_eq!(check(&app, session), Ok(()));

        app.apply(Event::ForegroundJobChanged { session, job: None });
        assert_eq!(check(&app, session), Err(CommandRefusal::NotClaude));
        assert!(send(&mut app, session).is_empty());
    }

    #[test]
    fn an_unknown_session_is_refused() {
        let mut app = App::new();
        assert_eq!(check(&app, sid(42)), Err(CommandRefusal::UnknownSession));
        assert!(send(&mut app, sid(42)).is_empty());
    }

    #[test]
    fn a_session_that_turned_busy_after_the_check_is_refused_at_the_send() {
        // The confirmation sits between the two: the send re-asks rather than
        // trusting an answer given before the user decided.
        let mut app = App::new();
        let session = launch_claude(&mut app);
        set_status(&mut app, session, SessionStatus::Idle);
        assert_eq!(check(&app, session), Ok(()));
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
