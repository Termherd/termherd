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

use crate::claude_command::{CLEAR_DRAFT, ClaudeCommand, CommandKind, PromptInput, SUBMIT};

use super::*;

/// Why a session cannot take a Claude command right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandRefusal {
    /// No live session has this id.
    UnknownSession,
    /// The session runs a shell, which would execute the line, not read it.
    NotClaude,
    /// Claude is in a status the command does not accept: a dialog would take
    /// the Enter, the session has not started or has exited, or the command
    /// waits for the turn to end (`/desktop`).
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
    /// Whether `session` can take a command of `kind` now: a Claude launch,
    /// in a status the command accepts, with `prompt` — its input as the
    /// screen shows it — empty.
    ///
    /// # Errors
    ///
    /// The [`CommandRefusal`] naming the first condition that fails.
    pub fn claude_command_check(
        &self,
        session: SessionId,
        kind: CommandKind,
        prompt: &PromptInput,
    ) -> Result<(), CommandRefusal> {
        let live = self
            .sessions
            .get(&session)
            .ok_or(CommandRefusal::UnknownSession)?;
        if !live.runs_claude() {
            return Err(CommandRefusal::NotClaude);
        }
        if !kind.accepts(live.status) {
            return Err(CommandRefusal::NotIdle(live.status));
        }
        match prompt {
            // A queue exists only during a turn, so a command that reaches
            // here with one already accepts a busy Claude, which keeps the
            // queued message queued.
            PromptInput::Empty | PromptInput::Queued => Ok(()),
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
        if self
            .claude_command_check(session, command.kind(), prompt)
            .is_err()
        {
            return Vec::new();
        }
        vec![
            Effect::Write {
                session,
                bytes: CLEAR_DRAFT.to_vec(),
            },
            Effect::Paced {
                session,
                writes: vec![command.line().into_bytes(), SUBMIT.to_vec()],
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::*;
    use crate::claude_command::ClaudeColor;
    use proptest::prelude::*;

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
        app.claude_command_check(session, CommandKind::Color, &PromptInput::Empty)
    }

    #[test]
    fn an_idle_claude_with_a_draft_or_no_prompt_on_screen_is_refused() {
        let mut app = App::new();
        let session = launch_claude(&mut app);
        set_status(&mut app, session, SessionStatus::Idle);
        let draft = PromptInput::Draft("fix the\nlogin bug".to_owned());
        assert_eq!(
            app.claude_command_check(session, CommandKind::Color, &draft),
            Err(CommandRefusal::DraftPresent(
                "fix the\nlogin bug".to_owned()
            ))
        );
        assert_eq!(
            app.claude_command_check(session, CommandKind::Color, &PromptInput::NotVisible),
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

    /// The writes `effects` would make, in order, each with whether it is
    /// paced (sent on its own after a pause) or sent at once.
    fn writes_of(effects: &[Effect], session: SessionId) -> Vec<(bool, Vec<u8>)> {
        effects
            .iter()
            .flat_map(|effect| match effect {
                Effect::Write { session: to, bytes } if *to == session => {
                    vec![(false, bytes.clone())]
                }
                Effect::Paced {
                    session: to,
                    writes,
                } if *to == session => writes.iter().map(|w| (true, w.clone())).collect(),
                other => panic!("an effect that is not a write: {other:?}"),
            })
            .collect()
    }

    fn rename_in_idle_claude(name: &str) -> (SessionId, Vec<Effect>) {
        let mut app = App::new();
        let session = launch_claude(&mut app);
        set_status(&mut app, session, SessionStatus::Idle);
        let command = ClaudeCommand::rename(name).expect("a typeable name");
        let effects = app.apply(Event::SendClaudeCommand {
            session,
            command,
            prompt: PromptInput::Empty,
        });
        (session, effects)
    }

    #[test]
    fn an_idle_claude_takes_the_clear_the_line_and_the_enter_each_apart() {
        let mut app = App::new();
        let session = launch_claude(&mut app);
        set_status(&mut app, session, SessionStatus::Idle);

        assert_eq!(check(&app, session), Ok(()));
        let effects = send(&mut app, session);
        assert_eq!(
            writes_of(&effects, session),
            vec![
                (false, b"\x15".to_vec()),
                (true, b"/color pink".to_vec()),
                (true, b"\r".to_vec()),
            ],
            "{effects:?}"
        );
    }

    #[test]
    fn a_rename_long_enough_to_read_as_a_paste_keeps_its_clear_out_of_the_burst() {
        // Live, Claude Code 2.1.319: `\x15` inside a paste burst is stripped
        // ("Removed 1 invisible character") and the line held for review, so
        // the Enter after it only acknowledges the review.
        let name = "a session name long enough to be read as a paste by claude code";
        let line = format!("/rename {name}");
        assert!(line.len() >= 64, "the case the paste rule bites");

        let (session, effects) = rename_in_idle_claude(name);
        assert_eq!(
            writes_of(&effects, session),
            vec![
                (false, b"\x15".to_vec()),
                (true, line.into_bytes()),
                (true, b"\r".to_vec()),
            ],
            "{effects:?}"
        );
    }

    proptest! {
        #[test]
        fn the_clear_goes_alone_and_every_write_after_it_is_paced(name in any::<String>()) {
            prop_assume!(ClaudeCommand::rename(&name).is_ok());
            let (session, effects) = rename_in_idle_claude(&name);
            let writes = writes_of(&effects, session);
            prop_assert!(writes.len() >= 2, "{writes:?}");
            prop_assert!(writes[1..].iter().all(|(paced, _)| *paced), "{writes:?}");
            for (_, bytes) in &writes {
                let control = bytes.iter().any(|b| b.is_ascii_control());
                prop_assert!(
                    !control || bytes.len() == 1,
                    "a control byte rides with text: {bytes:?}"
                );
            }
        }
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
        for status in [SessionStatus::Attention, SessionStatus::Exited] {
            set_status(&mut app, session, status);
            assert_eq!(check(&app, session), Err(CommandRefusal::NotIdle(status)));
            assert!(
                send(&mut app, session).is_empty(),
                "{status:?} sends nothing"
            );
        }
    }

    #[test]
    fn a_busy_claude_takes_a_rename_or_a_colour_but_not_desktop() {
        let mut app = App::new();
        let session = launch_claude(&mut app);
        set_status(&mut app, session, SessionStatus::Busy);

        for kind in [CommandKind::Rename, CommandKind::Color] {
            assert_eq!(
                app.claude_command_check(session, kind, &PromptInput::Empty),
                Ok(()),
                "{kind:?}"
            );
        }
        assert_eq!(
            app.claude_command_check(session, CommandKind::Desktop, &PromptInput::Empty),
            Err(CommandRefusal::NotIdle(SessionStatus::Busy))
        );

        let effects = send(&mut app, session);
        assert_eq!(
            writes_of(&effects, session),
            vec![
                (false, b"\x15".to_vec()),
                (true, b"/color pink".to_vec()),
                (true, b"\r".to_vec()),
            ],
            "{effects:?}"
        );
        let desktop = app.apply(Event::SendClaudeCommand {
            session,
            command: ClaudeCommand::Desktop,
            prompt: PromptInput::Empty,
        });
        assert!(desktop.is_empty(), "{desktop:?}");
    }

    #[test]
    fn a_busy_claude_with_a_queued_message_still_takes_a_rename() {
        // The queue is Claude's, not a draft: `/rename` applies and leaves the
        // queued message where it is (live, Claude Code 2.1.322).
        let mut app = App::new();
        let session = launch_claude(&mut app);
        set_status(&mut app, session, SessionStatus::Busy);
        assert_eq!(
            app.claude_command_check(session, CommandKind::Rename, &PromptInput::Queued),
            Ok(())
        );
        let effects = app.apply(Event::SendClaudeCommand {
            session,
            command: ClaudeCommand::rename("api work").expect("a name"),
            prompt: PromptInput::Queued,
        });
        assert_eq!(
            writes_of(&effects, session),
            vec![
                (false, b"\x15".to_vec()),
                (true, b"/rename api work".to_vec()),
                (true, b"\r".to_vec()),
            ],
            "{effects:?}"
        );
    }

    #[test]
    fn a_busy_claude_with_a_draft_is_still_refused() {
        let mut app = App::new();
        let session = launch_claude(&mut app);
        set_status(&mut app, session, SessionStatus::Busy);
        let draft = PromptInput::Draft("go W7".to_owned());
        assert_eq!(
            app.claude_command_check(session, CommandKind::Rename, &draft),
            Err(CommandRefusal::DraftPresent("go W7".to_owned()))
        );
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
    fn a_session_that_raised_a_dialog_after_the_check_is_refused_at_the_send() {
        // The confirmation sits between the two: the send re-asks rather than
        // trusting an answer given before the user decided.
        let mut app = App::new();
        let session = launch_claude(&mut app);
        set_status(&mut app, session, SessionStatus::Idle);
        assert_eq!(check(&app, session), Ok(()));
        set_status(&mut app, session, SessionStatus::Attention);
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
