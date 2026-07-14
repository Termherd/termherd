//! Launching terminals (FR4): registering a session in `core`, performing the
//! spawn, and the "new in context" / reopen shortcuts that derive a directory
//! from the focused session. Split from the shell's state machine so the
//! spawn-and-focus flow lives in one place.

use iced::Task;
use termherd_core::{Launch, LaunchSpec};

use super::{Focus, Message, Shell, home_dir};

/// Tab-title kind suffixes (presentation): a shell tab shows `$`, a Claude tab
/// 🤖, so the two kinds stay distinguishable at a glance. Kept app-side and
/// handed to the core title policy, so `core` carries no glyph literals.
const SHELL_GLYPH: &str = "$";
const CLAUDE_GLYPH: &str = "🤖";

impl Shell {
    /// Launch a terminal: register it in `core`, perform the spawn, focus it,
    /// and size its PTY to the current pane (FR4).
    pub(super) fn launch(&mut self, cwd: String, launch: Launch) -> Task<Message> {
        let title = self
            .core
            .tab_title(&cwd, &launch, SHELL_GLYPH, CLAUDE_GLYPH);
        let effects = self
            .core
            .apply(termherd_core::Event::LaunchSession(LaunchSpec {
                cwd: Some(cwd),
                launch,
                title,
            }));
        let spawn = self.perform(effects);
        self.focus = Focus::Terminal;
        // Opening another session drops any pending confirmation: a
        // stray Enter in the terminal must not confirm a sidebar prompt that's
        // no longer in view.
        self.closing = None;
        self.archiving = None;
        Task::batch([spawn, self.resize_panes()])
    }

    /// The working directory of the focused session, if one is open and its cwd
    /// is known. The anchor for the "new in context" shortcuts.
    pub(super) fn focused_cwd(&self) -> Option<String> {
        let id = self.core.workspace.focused_session()?;
        self.core.sessions.get(&id)?.cwd.clone()
    }

    /// Open a fresh shell in the focused session's directory, or in the
    /// home directory when nothing is open — so the shortcut still works from an
    /// empty workspace.
    pub(super) fn new_shell_here(&mut self) -> Task<Message> {
        let cwd = self.focused_cwd().unwrap_or_else(home_dir);
        self.launch(cwd, Launch::Shell)
    }

    /// Open a fresh Claude session in the repo containing the focused session.
    /// Walks up to the repo root so a session running in a subdirectory
    /// still lands at the repo. Inert when nothing is open — there is no context
    /// to derive a repo from.
    pub(super) fn new_claude_here(&mut self) -> Task<Message> {
        let Some(cwd) = self.focused_cwd() else {
            return Task::none();
        };
        let root = termherd_scan::repo_root(std::path::Path::new(&cwd))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(cwd);
        self.launch(root, Launch::Claude { resume: None })
    }

    /// Reopen the most recently closed tab, restoring its mode and
    /// directory. The reopen lives in `core`; here we just perform the spawn and
    /// focus the restored terminal, mirroring [`Self::launch`]. A no-op when the
    /// close stack is empty (`core` yields no effects).
    pub(super) fn reopen_closed_tab(&mut self) -> Task<Message> {
        let effects = self.core.apply(termherd_core::Event::ReopenClosedTab);
        if effects.is_empty() {
            return Task::none();
        }
        let spawn = self.perform(effects);
        self.focus = Focus::Terminal;
        self.closing = None;
        self.archiving = None;
        Task::batch([spawn, self.resize_panes()])
    }
}
