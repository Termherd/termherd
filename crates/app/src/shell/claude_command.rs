//! The confirmation between a Claude slash command and the session it is typed
//! into — the one write path for every edit termherd asks Claude to make.
//!
//! Every surface arms the same prompt through [`Shell::arm_claude_command`]:
//! the keymap, the MCP tool, and the rename and colour menus to come. Nothing
//! is typed until the prompt is confirmed, and the confirmation asks `core`
//! again, since the session may have started work while the prompt was up.

use std::fmt;

use iced::Task;
use termherd_core::workspace::SessionId;
use termherd_core::{ClaudeCommand, CommandRefusal, Event};

use super::{Message, Shell};
use crate::strings;

/// A command waiting on the user's confirmation, and the session it is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingCommand {
    pub(super) session: SessionId,
    pub(super) command: ClaudeCommand,
}

/// Why a command could not be armed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            .claude_command_check(session)
            .map_err(ArmRefusal::Session)?;
        let line = command.line();
        tracing::info!(session = session.0.get(), %line, "claude command armed");
        self.claude_command = Some(PendingCommand { session, command });
        Ok(line)
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

    /// Type the armed command, if its session can still take it.
    pub(super) fn confirm_claude_command(&mut self) -> Task<Message> {
        let Some(PendingCommand { session, command }) = self.claude_command.take() else {
            return Task::none();
        };
        let line = command.line();
        let effects = self
            .core
            .apply(Event::SendClaudeCommand { session, command });
        if effects.is_empty() {
            tracing::warn!(session = session.0.get(), %line, "claude command dropped: session no longer idle");
        }
        self.perform(effects)
    }

    /// Ask Claude to rename `session` to `name`, behind the same confirmation
    /// as every command, returning the line armed. A blank name, or the name
    /// the session already shows, asks nothing: neither changes what Claude
    /// calls it.
    ///
    /// # Errors
    ///
    /// Why nothing could be armed, in words — the session cannot take it now,
    /// another prompt is open, or nothing typeable is left of the name.
    pub(super) fn ask_claude_to_rename(
        &mut self,
        session: SessionId,
        name: &str,
        current: &str,
    ) -> Result<Option<String>, String> {
        let name = name.trim();
        if name.is_empty() || name == current.trim() {
            return Ok(None);
        }
        let command = ClaudeCommand::rename(name).map_err(|error| error.to_string())?;
        self.arm_claude_command(session, command)
            .map(Some)
            .map_err(|refusal| refusal.to_string())
    }

    /// [`Self::ask_claude_to_rename`] for a rename the user typed: a refusal
    /// becomes the notice under the tab strip, since the edit they made has
    /// already closed and would otherwise vanish without a word.
    pub(super) fn rename_through_claude(&mut self, session: SessionId, name: &str, current: &str) {
        self.notice = None;
        if let Err(why) = self.ask_claude_to_rename(session, name, current) {
            tracing::warn!(session = session.0.get(), %why, "rename not sent to claude");
            self.notice = Some(strings::rename_refused(&why));
        }
    }

    /// Drop the armed command without typing anything.
    pub(super) fn cancel_claude_command(&mut self) -> Task<Message> {
        self.claude_command = None;
        Task::none()
    }
}
