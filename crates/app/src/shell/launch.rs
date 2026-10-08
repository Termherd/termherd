//! Launching terminals (FR4): registering a session in `core`, performing the
//! spawn, and the "new in context" / reopen shortcuts that derive a directory
//! from the focused session. Split from the shell's state machine so the
//! spawn-and-focus flow lives in one place.

use iced::Task;
use termherd_core::workspace::SessionId;
use termherd_core::{Effect, Launch, LaunchSpec, Placement};

use super::{Focus, Message, Shell, home_dir};

impl Shell {
    /// Launch a terminal: register it in `core`, perform the spawn, focus it,
    /// and size its PTY to the current pane (FR4).
    pub(super) fn launch(&mut self, cwd: String, launch: Launch) -> Task<Message> {
        self.launch_at(cwd, launch, Placement::Foreground).1
    }

    /// Launch a terminal at `placement`, returning the new session alongside
    /// the spawn. A foreground launch takes focus and drops any pending
    /// prompt; a background one leaves both to the user and sizes only its own
    /// tab, which is drawn at the same area when it is brought forward.
    pub(super) fn launch_at(
        &mut self,
        cwd: String,
        launch: Launch,
        placement: Placement,
    ) -> (Option<SessionId>, Task<Message>) {
        let title = self.core.tab_title(&cwd, &launch);
        let effects = self
            .core
            .apply(termherd_core::Event::LaunchSession(LaunchSpec {
                cwd: Some(cwd),
                launch,
                title,
                placement,
            }));
        let opened = effects.iter().find_map(|effect| match effect {
            Effect::Spawn(spec) => Some(spec.session),
            _ => None,
        });
        let spawn = self.perform(effects);
        let resize = match placement {
            Placement::Foreground => {
                self.focus = Focus::Terminal;
                // Opening another session drops any pending confirmation: a
                // stray Enter in the terminal must not confirm a sidebar prompt
                // that's no longer in view.
                self.closing = None;
                self.archiving = None;
                self.resize_panes()
            }
            Placement::Background => opened
                .and_then(|id| self.core.workspace.tab_of(id))
                .map_or_else(Task::none, |index| self.resize_tab(index)),
        };
        (opened, Task::batch([spawn, resize]))
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
    /// still lands at the repo. `None` when nothing is open — there is no context
    /// to derive a repo from, and a caller is told so rather than told it ran.
    pub(super) fn new_claude_here(&mut self) -> Option<Task<Message>> {
        let cwd = self.focused_cwd()?;
        let root = termherd_scan::repo_root(std::path::Path::new(&cwd))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(cwd);
        Some(self.launch(root, Launch::Claude { resume: None }))
    }

    /// Reopen the most recently closed tab, restoring its mode and
    /// directory. The reopen lives in `core`; here we just perform the spawn and
    /// focus the restored terminal, mirroring [`Self::launch`]. `None` when the
    /// close stack is empty (`core` yields no effects), so a caller learns there
    /// was nothing to reopen instead of being told a tab came back.
    pub(super) fn reopen_closed_tab(&mut self) -> Option<Task<Message>> {
        let effects = self.core.apply(termherd_core::Event::ReopenClosedTab);
        if effects.is_empty() {
            return None;
        }
        let spawn = self.perform(effects);
        self.focus = Focus::Terminal;
        self.closing = None;
        self.archiving = None;
        Some(Task::batch([spawn, self.resize_panes()]))
    }
}
