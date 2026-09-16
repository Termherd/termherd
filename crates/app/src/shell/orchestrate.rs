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
use termherd_core::{Event, Launch, PointerEvent, PointerRoute};

use super::bridge::{
    Action, ActionDetail, ActionOutcome, PointerOutcome, Press, PressOutcome, PressStep,
    RepoOutcome, SessionKind,
};
use super::input::event_of;
use super::repos::RepoGesture;
use super::routing::KeyVerdict;
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
            Action::Pointer { session, pointer } => self.act_pointer(session, pointer),
            Action::DeclareRepo { path } => self.act_declare_repo(&path),
            Action::ForgetRepo { path } => self.act_forget_repo(&path),
        }
    }

    /// Add a repo to the sidebar. The path is normalised first — the caller may
    /// have passed a subdirectory or a worktree and cannot know the key the
    /// sidebar uses — and a path that does not exist is rejected before
    /// anything applies, since its launch buttons could not work.
    fn act_declare_repo(&mut self, path: &str) -> (ActionOutcome, Task<Message>) {
        let Some(key) = termherd_scan::normalize_repo_path(std::path::Path::new(path)) else {
            return (
                ActionOutcome::rejected(format!("no such directory: {path}")),
                Task::none(),
            );
        };
        let key = key.display().to_string();
        let task = self.declare_repo_key(&key, RepoGesture::Mcp);
        (
            self.applied()
                .with_detail(ActionDetail::Repo(self.repo_outcome(&key))),
            task,
        )
    }

    /// Drop a repo's declaration. Unlike declaring, an unknown path is not an
    /// error: the caller asked for an absence and gets one. The outcome says
    /// whether a row survived on its sessions.
    ///
    /// A path that no longer exists cannot be normalised against the disk, but
    /// it still gets the textual half of the rule — a worktree deleted after it
    /// was declared is filed under its main checkout, and forgetting the raw
    /// path would remove nothing while reporting success.
    fn act_forget_repo(&mut self, path: &str) -> (ActionOutcome, Task<Message>) {
        let key = termherd_scan::normalize_repo_path(std::path::Path::new(path)).map_or_else(
            || termherd_scan::sidebar_key(path),
            |p| p.display().to_string(),
        );
        let task = self.forget_repo_key(&key, RepoGesture::Mcp);
        (
            self.applied()
                .with_detail(ActionDetail::Repo(self.repo_outcome(&key))),
            task,
        )
    }

    /// The sidebar row for `key` as it stands now — **membership**, not what
    /// the screen happens to be showing. Read off the rendered list, a search
    /// the user had left in the box (or the archive knob) reported a successful
    /// `add_repo` back to the agent as a failure.
    fn repo_outcome(&self, key: &str) -> RepoOutcome {
        let row = self.core.sidebar_row(key);
        RepoOutcome {
            path: key.to_owned(),
            declared: self.core.is_repo_declared(key),
            session_count: row.unwrap_or(0),
            visible: row.is_some(),
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

    /// Bring the pane hosting `session` into view — activating its tab when it
    /// lives in another one — and hand the keyboard to the terminal. Rejects a
    /// handle no open pane hosts.
    fn act_focus(&mut self, session: u64) -> (ActionOutcome, Task<Message>) {
        let id = match self.resolve_pane(session) {
            Ok(id) => id,
            Err(outcome) => return (outcome, Task::none()),
        };
        self.focus = Focus::Terminal;
        let effects = self.core.apply(Event::RevealPane(id));
        // A reveal may activate another tab, whose panes were last sized for a
        // different layout — resize like `activate_tab` does.
        let task = Task::batch([self.perform(effects), self.resize_panes()]);
        (self.applied(), task)
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

    /// Type bytes into a session's PTY without waiting; synchronising is a
    /// separate request (`WaitForStatus`), served in `shell::serve`.
    /// Rejects an unknown handle, so a stale target can't misfire into a live
    /// terminal.
    fn act_run(&mut self, session: u64, bytes: Vec<u8>) -> (ActionOutcome, Task<Message>) {
        let Some(id) = self.resolve(session) else {
            return (unknown_handle(session), Task::none());
        };
        let effects = self.core.apply(Event::TerminalInput { session: id, bytes });
        (self.applied(), self.perform(effects))
    }

    /// Place a pointer event at a cell of a session's terminal. Bounded by the
    /// session's last rendered geometry — a cell outside it, or a session that
    /// has not drawn yet, is rejected before anything applies, so a caller
    /// never learns of a bad coordinate from a selection landing elsewhere.
    fn act_pointer(
        &mut self,
        session: u64,
        pointer: PointerEvent,
    ) -> (ActionOutcome, Task<Message>) {
        let Some(id) = self.resolve(session) else {
            return (unknown_handle(session), Task::none());
        };
        let Some(screen) = self.screens.get(&id) else {
            return (
                ActionOutcome::rejected(format!(
                    "session {session} has not rendered yet, so it has no geometry to \
                     place a pointer in"
                )),
                Task::none(),
            );
        };
        if pointer.col >= screen.cols || pointer.row >= screen.rows {
            return (
                ActionOutcome::rejected(format!(
                    "cell ({}, {}) is outside the {}x{} pane",
                    pointer.col, pointer.row, screen.cols, screen.rows
                )),
                Task::none(),
            );
        }
        // Answered off the last screen's mouse mode, as the bounds are off its
        // geometry; the terminal decides again on its live mode and offset.
        let outcome = match pointer.route(screen.mouse_reporting) {
            PointerRoute::Forward => PointerOutcome::Forwarded,
            PointerRoute::Select(_) => PointerOutcome::Selection,
            PointerRoute::Nothing => PointerOutcome::Ignored,
        };
        let effects = self.core.apply(Event::TerminalPointer {
            session: id,
            pointer,
        });
        (
            self.applied().with_detail(ActionDetail::Pointer(outcome)),
            self.perform(effects),
        )
    }

    /// The shared prelude of the focus-relative actions (split, close): reveal
    /// `pane` first when one is named, returning the reveal's effects to fold
    /// in, or a rejection when no open pane hosts the handle. `None` leaves the
    /// current focus and yields no effects.
    ///
    /// The reveal must actually land, because what follows — `SplitFocused`,
    /// `CloseFocusedPane` — reads the focus rather than the handle. A silent
    /// no-op here would split or **kill the wrong pane** while reporting
    /// success, so the target is checked before anything applies.
    fn retarget(&mut self, pane: Option<u64>) -> Result<Vec<termherd_core::Effect>, ActionOutcome> {
        let Some(handle) = pane else {
            return Ok(Vec::new());
        };
        let id = self.resolve_pane(handle)?;
        self.focus = Focus::Terminal;
        Ok(self.core.apply(Event::RevealPane(id)))
    }

    /// Resolve a stable handle to the [`SessionId`] of a **pane that is
    /// actually open**. The session registry is workspace-global and carries no
    /// tab dimension, so it cannot answer this: `tab_of` is the check that
    /// names a pane a focus-relative action can aim at.
    fn resolve_pane(&self, handle: u64) -> Result<SessionId, ActionOutcome> {
        NonZeroU64::new(handle)
            .map(SessionId)
            .filter(|id| self.core.workspace.tab_of(*id).is_some())
            .ok_or_else(|| unhosted_handle(handle))
    }

    /// Resolve a stable handle to a live [`SessionId`], or `None` when no session
    /// carries it (already closed, or never existed).
    pub(super) fn resolve(&self, handle: u64) -> Option<SessionId> {
        let id = NonZeroU64::new(handle).map(SessionId)?;
        self.core.sessions.contains_key(&id).then_some(id)
    }

    /// Press each of `presses` against the app itself, in the order asked,
    /// reporting what the routing ladder did with each plus the focus left
    /// behind.
    ///
    /// Every press runs; none short-circuits. A press the ladder did not act on
    /// is a *reported step*, not a failure — and stopping there would break the
    /// sequence that matters most, where one press opens an overlay and the next
    /// one answers it.
    pub(super) fn perform_presses(&mut self, presses: Vec<Press>) -> (PressOutcome, Task<Message>) {
        let mut steps = Vec::with_capacity(presses.len());
        let mut tasks = Vec::with_capacity(presses.len());
        for press in presses {
            let (step, task) = self.press(press);
            steps.push(step);
            tasks.push(task);
        }
        let outcome = PressOutcome {
            steps,
            focused: self.focused_handle(),
            error: None,
        };
        (outcome, Task::batch(tasks))
    }

    /// Carry out one [`Press`].
    ///
    /// A chord goes in as a *synthesised key event* through [`Shell::on_key`], so
    /// it walks the same ladder a physical press walks — the whole reason an
    /// agent can dismiss an overlay it opened. A named action skips the keymap
    /// but is still gated on the ladder, so neither tool can reach a state the
    /// keyboard cannot.
    fn press(&mut self, press: Press) -> (PressStep, Task<Message>) {
        match press {
            Press::Chord(chord) => match event_of(&chord) {
                Some(event) => {
                    let (verdict, task) = self.on_key(event);
                    (step_of(verdict), task)
                }
                // A chord naming a key no event can carry. A human pressing it
                // gets nothing either, so it reports as nothing happened.
                None => (PressStep::Unbound, Task::none()),
            },
            Press::Command(action) => match self.keyboard_owner() {
                Some(owner) => (PressStep::Overlay(owner.label().to_owned()), Task::none()),
                None => {
                    let (verdict, task) = self.dispatch_action(action);
                    (step_of(verdict), task)
                }
            },
        }
    }

    /// An applied outcome reporting the session that holds focus now.
    fn applied(&self) -> ActionOutcome {
        ActionOutcome::applied(self.focused_handle())
    }

    /// The stable handle of the session holding focus, as an external caller
    /// spells it — `None` when the workspace is empty.
    fn focused_handle(&self) -> Option<String> {
        self.core
            .workspace
            .focused_session()
            .map(|id| id.0.get().to_string())
    }
}

/// The routing ladder's verdict as the wire reports it. Exhaustive, so a new
/// [`KeyVerdict`] case is a compile error here rather than one that silently
/// reports as its neighbour.
fn step_of(verdict: KeyVerdict) -> PressStep {
    match verdict {
        KeyVerdict::Overlay(name) => PressStep::Overlay(name.to_owned()),
        KeyVerdict::Ran(name) => PressStep::Ran(name),
        KeyVerdict::Inert(name, inertia) => PressStep::Inert {
            action: name,
            reason: inertia.label(),
        },
        KeyVerdict::Typed => PressStep::Typed,
        KeyVerdict::Ignored => PressStep::Unbound,
    }
}

/// The rejection for a handle that resolves to no live session — named so an
/// agent sees which id it got wrong.
fn unknown_handle(handle: u64) -> ActionOutcome {
    ActionOutcome::rejected(format!("no live session with handle {handle}"))
}

/// The rejection for a handle no open pane hosts — closed, or never opened. A
/// focus-relative action has nothing to aim at, and must not fall back to
/// whatever happens to hold focus.
fn unhosted_handle(handle: u64) -> ActionOutcome {
    ActionOutcome::rejected(format!("no open pane hosts handle {handle}"))
}
