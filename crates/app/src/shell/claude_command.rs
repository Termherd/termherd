//! The confirmation between a Claude slash command and the session it is typed
//! into — the one write path for every edit termherd asks Claude to make.
//!
//! Every surface arms the same prompt through [`Shell::arm_claude_command`]:
//! the keymap, the MCP tool, and the rename and colour menus to come. Nothing
//! is typed until the prompt is confirmed, and the confirmation asks `core`
//! again, against the screen as it is then: the session may have started work,
//! or a draft may have appeared, while the prompt was up.

use std::fmt;
use std::time::{Duration, Instant};

use iced::Task;
use iced::keyboard;
use termherd_core::workspace::SessionId;
use termherd_core::{ClaudeCommand, CommandRefusal, Event, PromptInput, read_prompt};

use super::routing::is_enter;
use super::{Message, Shell};

/// How long a prompt armed by a remote caller ignores a physical Enter. The
/// user may be typing in a pane when an agent arms the prompt; without this,
/// the Enter that ends their own line would confirm a command they never read.
const REMOTE_ARM_GRACE: Duration = Duration::from_millis(600);

/// A command waiting on the user's confirmation, and the session it is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingCommand {
    pub(super) session: SessionId,
    pub(super) command: ClaudeCommand,
    /// Until when a physical Enter is ignored; `None` when the user armed it
    /// themselves, and so has already read it.
    pub(super) enter_ignored_until: Option<Instant>,
    /// Why the last confirmation typed nothing, shown in the prompt so the user
    /// learns it rather than seeing the prompt simply vanish.
    pub(super) refused: Option<CommandRefusal>,
}

/// Why a command could not be armed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ArmRefusal {
    /// The session cannot take it now; `core` says why.
    Session(CommandRefusal),
    /// Another prompt holds the keyboard, and arming over it would leave two
    /// questions open with one keyboard to answer them. Carries its label.
    PromptOpen(&'static str),
}

impl fmt::Display for ArmRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(refusal) => refusal.fmt(f),
            Self::PromptOpen(label) => {
                write!(f, "another prompt is open ({label}); answer it first")
            }
        }
    }
}

impl Shell {
    /// Arm the confirmation for typing `command` into `session`, returning the
    /// exact line the prompt shows. Nothing is typed yet.
    pub(super) fn arm_claude_command(
        &mut self,
        session: SessionId,
        command: ClaudeCommand,
    ) -> Result<String, ArmRefusal> {
        if let Some(owner) = self.keyboard_owner() {
            return Err(ArmRefusal::PromptOpen(owner.label()));
        }
        self.core
            .claude_command_check(session, &self.prompt_input(session))
            .map_err(ArmRefusal::Session)?;
        let line = command.line();
        tracing::info!(session = session.0.get(), %line, "claude command armed");
        self.claude_command = Some(PendingCommand {
            session,
            command,
            enter_ignored_until: None,
            refused: None,
        });
        Ok(line)
    }

    /// Mark the armed prompt as armed by a remote caller, so a physical Enter
    /// is ignored for a moment: the user has not read it yet.
    pub(super) fn hold_enter_after_remote_arm(&mut self) {
        if let Some(pending) = self.claude_command.as_mut() {
            pending.enter_ignored_until = Some(Instant::now() + REMOTE_ARM_GRACE);
        }
    }

    /// Arm `command` for the focused pane. `None` when nothing is focused or
    /// the focused pane cannot take it, so the keymap reports the refusal.
    pub(super) fn arm_focused_claude_command(
        &mut self,
        command: ClaudeCommand,
    ) -> Option<Task<Message>> {
        let session = self.core.workspace.focused_session()?;
        self.arm_claude_command(session, command)
            .ok()
            .map(|_| Task::none())
    }

    /// What `session`'s screen shows of Claude's prompt. A session that has
    /// not drawn, or whose view is scrolled back, shows none.
    fn prompt_input(&self, session: SessionId) -> PromptInput {
        match self.screens.get(&session) {
            Some(screen) if !screen.scrolled => read_prompt(&screen.text()),
            _ => PromptInput::NotVisible,
        }
    }

    /// Type the armed command, if its session can still take it. A refusal
    /// keeps the prompt open and names why, so nothing vanishes unexplained;
    /// `escape` then dismisses it.
    pub(super) fn confirm_claude_command(&mut self) -> Result<Task<Message>, CommandRefusal> {
        let Some(session) = self.claude_command.as_ref().map(|pending| pending.session) else {
            return Ok(Task::none());
        };
        let prompt = self.prompt_input(session);
        if let Err(refusal) = self.core.claude_command_check(session, &prompt) {
            if let Some(pending) = self.claude_command.as_mut() {
                tracing::warn!(
                    session = session.0.get(),
                    line = %pending.command,
                    %refusal,
                    "claude command not typed"
                );
                pending.refused = Some(refusal.clone());
            }
            return Err(refusal);
        }
        let Some(pending) = self.claude_command.take() else {
            return Ok(Task::none());
        };
        let effects = self.core.apply(Event::SendClaudeCommand {
            session: pending.session,
            command: pending.command,
            prompt,
        });
        Ok(self.perform(effects))
    }

    /// Drop the armed command without typing anything.
    pub(super) fn cancel_claude_command(&mut self) -> Task<Message> {
        self.claude_command = None;
        Task::none()
    }

    /// Whether a physical key event lands inside a remotely armed prompt's
    /// grace and is an Enter — one the user meant for something else.
    pub(super) fn enter_too_soon(&self, event: &keyboard::Event, now: Instant) -> bool {
        is_enter(event)
            && self
                .claude_command
                .as_ref()
                .and_then(|pending| pending.enter_ignored_until)
                .is_some_and(|until| now < until)
    }
}
