//! The orchestration seam: carrying an MCP [`Action`] out against the running
//! workspace. Split from the shell's state machine so the "resolve a handle →
//! apply the existing event(s) → report the resulting focus" flow lives in one
//! place, mirroring how [`launch`](super::launch) owns the spawn-and-focus flow.
//!
//! Every action is a thin wrapper over an existing core
//! [`Event`](termherd_core::Event): the shell owns `core::App` *and* the one
//! effect executor, so it can resolve the stable handle, apply the event, and
//! perform the effects — the read-only `respond` cannot, since it holds only a
//! `&App`.

use std::num::NonZeroU64;

use iced::Task;
use termherd_core::workspace::{SessionId, SplitDir};
use termherd_core::{Event, Launch};

use super::bridge::{Action, ActionOutcome, SessionKind};
use super::{Focus, Message, Shell, home_dir};

impl Shell {
    /// Carry out one MCP [`Action`], returning the outcome to answer the caller
    /// with plus any async follow-up the applied effects need (a PTY spawn's
    /// resize). A handle that resolves to no live session — or an out-of-range
    /// tab — is rejected before any state is touched.
    pub(super) fn perform_action(&mut self, action: Action) -> (ActionOutcome, Task<Message>) {
        match action {
            Action::Open { project, kind } => self.act_open(project, kind),
            Action::Split { pane, dir } => self.act_split(pane, dir),
            Action::Focus { session } => self.act_focus(session),
            Action::Rename { tab, title } => self.act_rename(tab, title),
            Action::Close { pane } => self.act_close(pane),
            Action::Run { session, bytes } => self.act_run(session, bytes),
        }
    }

    /// Open a new session, reusing the shell's own launch path (the same one a
    /// click drives), so the spawn, focus and resize all match. No project falls
    /// back to the home directory, so the tool works from an empty workspace.
    fn act_open(
        &mut self,
        project: Option<String>,
        kind: SessionKind,
    ) -> (ActionOutcome, Task<Message>) {
        let launch = match kind {
            SessionKind::Shell => Launch::Shell,
            SessionKind::Claude => Launch::Claude { resume: None },
        };
        let task = self.launch(project.unwrap_or_else(home_dir), launch);
        (self.applied(), task)
    }

    /// Split a pane, opening a fresh session beside it. With `pane` given, focus
    /// it first so the focus-relative `SplitFocused` acts on it; the new pane
    /// then takes focus. An unknown target is rejected before anything applies.
    fn act_split(&mut self, pane: Option<u64>, dir: SplitDir) -> (ActionOutcome, Task<Message>) {
        let mut effects = match self.retarget(pane) {
            Ok(effects) => effects,
            Err(outcome) => return (outcome, Task::none()),
        };
        effects.extend(self.core.apply(Event::SplitFocused(dir)));
        // A split halves the original pane's area and spawns the new one at a
        // default grid, so both need a resize to their real cells.
        let task = Task::batch([self.perform(effects), self.resize_panes()]);
        (self.applied(), task)
    }

    /// Move focus to the pane hosting `session`, like click-to-focus (it also
    /// hands the keyboard to the terminal). Rejects an unknown handle.
    fn act_focus(&mut self, session: u64) -> (ActionOutcome, Task<Message>) {
        let Some(id) = self.resolve(session) else {
            return (unknown_handle(session), Task::none());
        };
        self.focus = Focus::Terminal;
        let effects = self.core.apply(Event::FocusPane(id));
        (self.applied(), self.perform(effects))
    }

    /// Rename the tab at `tab`. A blank title reverts to the derived name
    /// (core's rule). Rejects an index past the open tabs.
    fn act_rename(&mut self, tab: usize, title: String) -> (ActionOutcome, Task<Message>) {
        if self.core.workspace.tabs.get(tab).is_none() {
            return (
                ActionOutcome::rejected(format!("no tab at index {tab}")),
                Task::none(),
            );
        }
        let effects = self.core.apply(Event::RenameTab { index: tab, title });
        (self.applied(), self.perform(effects))
    }

    /// Close a pane — the focused one, or `pane` when given (focused first). A
    /// lone pane is the whole tab, so core collapses to `close_tab`, killing the
    /// PTY. Rejects an unknown target.
    fn act_close(&mut self, pane: Option<u64>) -> (ActionOutcome, Task<Message>) {
        let mut effects = match self.retarget(pane) {
            Ok(effects) => effects,
            Err(outcome) => return (outcome, Task::none()),
        };
        effects.extend(self.core.apply(Event::CloseFocusedPane));
        let task = Task::batch([self.perform(effects), self.resize_panes()]);
        (self.applied(), task)
    }

    /// Type bytes into a session's PTY without waiting (waiting is a later rung).
    /// Rejects an unknown handle, so a stale target can't misfire into a live
    /// terminal.
    fn act_run(&mut self, session: u64, bytes: Vec<u8>) -> (ActionOutcome, Task<Message>) {
        let Some(id) = self.resolve(session) else {
            return (unknown_handle(session), Task::none());
        };
        let effects = self.core.apply(Event::TerminalInput { session: id, bytes });
        (self.applied(), self.perform(effects))
    }

    /// The shared prelude of the focus-relative actions (split, close): move
    /// focus onto `pane` first when one is named, returning the focus effects to
    /// fold in, or a rejection when the handle is unknown. `None` leaves the
    /// current focus and yields no effects.
    fn retarget(&mut self, pane: Option<u64>) -> Result<Vec<termherd_core::Effect>, ActionOutcome> {
        let Some(handle) = pane else {
            return Ok(Vec::new());
        };
        let Some(id) = self.resolve(handle) else {
            return Err(unknown_handle(handle));
        };
        self.focus = Focus::Terminal;
        Ok(self.core.apply(Event::FocusPane(id)))
    }

    /// Resolve a stable handle to a live [`SessionId`], or `None` when no session
    /// carries it (already closed, or never existed).
    fn resolve(&self, handle: u64) -> Option<SessionId> {
        let id = NonZeroU64::new(handle).map(SessionId)?;
        self.core.sessions.contains_key(&id).then_some(id)
    }

    /// An applied outcome reporting the session that holds focus now.
    fn applied(&self) -> ActionOutcome {
        ActionOutcome::applied(
            self.core
                .workspace
                .focused_session()
                .map(|id| id.0.get().to_string()),
        )
    }
}

/// The rejection for a handle that resolves to no live session — named so an
/// agent sees which id it got wrong.
fn unknown_handle(handle: u64) -> ActionOutcome {
    ActionOutcome::rejected(format!("no live session with handle {handle}"))
}
