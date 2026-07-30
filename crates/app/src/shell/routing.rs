//! The keyboard routing seam (ARCHITECTURE §8): the modal precedence ladder
//! that decides where a key press goes — a keymap [`Action`], an inline
//! rename / confirmation overlay, or the focused terminal's PTY. Split from the
//! shell's state machine so the precedence wiring lives in one auditable place.

use iced::advanced::widget::{operate, operation::focusable};
use iced::keyboard::{Key, key::Named};
use iced::{Task, keyboard};
use termherd_core::workspace::{Direction, SplitDir};
use termherd_core::{Action, ScrollTarget};
use termherd_pty::TermKey;

use super::input::{chord_of, key_mods, numpad_char, to_term_key};
use super::{Focus, Message, Shell, search_id};

/// How a confirmation overlay reads a key — the shared shape behind the quit,
/// tab-close and archive prompts.
enum ConfirmKey {
    Confirm,
    Cancel,
    Swallow,
}

/// Which overlay holds the keyboard, in precedence order — the quit modal wins
/// over the prompts beneath it.
///
/// The ladder as *data*, because it has three readers: the key router, the
/// terminal-input guard, and the MCP press tool. Stated once, as a predicate,
/// they cannot drift; stated three times, the first edit that touches one of
/// them is a bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeyboardOwner {
    /// The inline tab-title field.
    TabRename,
    /// The sidebar's inline session-rename field.
    SessionRename,
    /// The macOS Cmd+Q quit confirmation.
    Quit,
    /// The close-confirmation for the tab at this index, which the prompt needs
    /// to act. Carried here rather than re-read on dispatch: a second read
    /// would need a fallback for a state this variant already rules out.
    TabClose(usize),
    /// The archive confirmation.
    Archive,
    /// The document editor, which handles its own keys.
    Doc,
}

impl KeyboardOwner {
    /// Every rung of the ladder, for a caller that must visit all of them —
    /// today, the test asserting each one can be left from the keyboard.
    ///
    /// A hand-written list is only safe because it sits against the exhaustive
    /// `match` below: a new variant fails to compile there, in this file, where
    /// this array is the next thing the author reads.
    #[cfg(test)]
    pub(super) const ALL: [Self; 6] = [
        Self::TabRename,
        Self::SessionRename,
        Self::Quit,
        Self::TabClose(0),
        Self::Archive,
        Self::Doc,
    ];

    /// The name an external caller reads when this overlay consumed its press.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::TabRename => "tab-rename",
            Self::SessionRename => "session-rename",
            Self::Quit => "quit-confirm",
            Self::TabClose(_) => "tab-close-confirm",
            Self::Archive => "archive-confirm",
            Self::Doc => "doc-editor",
        }
    }
}

/// Why running a keymap action changed nothing.
///
/// The two cases call for opposite responses from a caller, which is the whole
/// reason they are not one: a missing surface will never appear, so retrying is
/// pointless; a missing precondition is something the caller can go and create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Inertia {
    /// The action is in the keymap vocabulary and wired to nothing.
    NoSurface,
    /// The action is wired, but refused before acting because a precondition was
    /// absent — no focused session to derive a repo from, no closed tab to
    /// reopen, nothing to scroll.
    ///
    /// Deliberately narrower than "had no visible effect": an action whose event
    /// `core` applies and absorbs (a tab index past the open tabs) *did* run, and
    /// says so. The line is whether the shell refused, not whether the result
    /// was interesting.
    NoContext,
}

impl Inertia {
    /// The reason an external caller reads.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::NoSurface => "no-surface",
            Self::NoContext => "no-context",
        }
    }
}

/// What the routing ladder did with one key press.
///
/// The ladder naming its own outcome, so a caller that needs to *report* what a
/// press did reads the same verdict the press itself produced. Re-deriving it
/// from a second reading of the state would be the same invariant expressed
/// twice — and the two would disagree the first time a rung moved.
///
/// A real keypress discards this; the MCP press tool is what reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum KeyVerdict {
    /// An open overlay consumed it — acted on it or swallowed it. Carries
    /// [`KeyboardOwner::label`].
    Overlay(&'static str),
    /// A bound keymap action ran; carries its config name.
    Ran(String),
    /// A bound keymap action changed nothing, and why. Kept apart from
    /// [`Self::Ran`] because a caller told "ran" would believe a gesture that
    /// never occurred, and apart from [`Self::Ignored`] because the two call for
    /// opposite responses: an ignored press invites another chord, an inert one
    /// will never work however it is bound.
    Inert(String, Inertia),
    /// It reached the focused terminal as input bytes.
    Typed,
    /// Nothing claimed it: no overlay, no binding, and no terminal to type into.
    Ignored,
}

/// Enter confirms, Escape cancels; everything else (and any non-press event) is
/// swallowed so it can't reach the terminal beneath the prompt.
fn classify_confirm(event: &keyboard::Event) -> ConfirmKey {
    if is_escape(event) {
        return ConfirmKey::Cancel;
    }
    match event {
        keyboard::Event::KeyPressed {
            key: Key::Named(Named::Enter),
            ..
        } => ConfirmKey::Confirm,
        _ => ConfirmKey::Swallow,
    }
}

/// Escape, the one key every overlay must answer: it is how a caller with no
/// mouse — which is every MCP caller — gets the keyboard back.
///
/// Modifiers are ignored, so `Shift+Escape` and `Cmd+Escape` leave too. No
/// platform binds them to anything an overlay could mean, and a caller
/// fumbling a modifier while trying to escape should still escape.
fn is_escape(event: &keyboard::Event) -> bool {
    matches!(
        event,
        keyboard::Event::KeyPressed {
            key: Key::Named(Named::Escape),
            ..
        }
    )
}

impl Shell {
    /// Run a keymap [`Action`] (FR9). Clipboard actions become iced tasks; tab
    /// actions drive `core`.
    ///
    /// `Err` means the action changed nothing, and says why (see [`Inertia`]).
    /// This `match` is the only place that knows, so a caller reporting what a
    /// press did cannot over-claim: told `ran` about an action that refused, an
    /// agent would record a gesture that never happened — and, verifying a fix,
    /// read a false pass.
    ///
    /// The handlers that can refuse return `Option` for exactly this reason, so
    /// the knowledge lives at the refusal rather than in a predicate here that
    /// would have to re-derive it.
    pub(super) fn run_action(&mut self, action: Action) -> Result<Task<Message>, Inertia> {
        Ok(match action {
            Action::Copy => self.copy_selection().ok_or(Inertia::NoContext)?,
            Action::Paste => iced::clipboard::read().map(Message::Paste),
            Action::NextTab => self.cycle_tab(1).ok_or(Inertia::NoContext)?,
            Action::PrevTab => self.cycle_tab(-1).ok_or(Inertia::NoContext)?,
            Action::CloseFocused => self.close_focused_pane().ok_or(Inertia::NoContext)?,
            Action::FocusSearch => {
                self.focus = Focus::Search;
                operate(focusable::focus(search_id()))
            }
            Action::ToggleSidebar => self.toggle_sidebar(),
            Action::ScrollTop => self
                .scroll_focused(ScrollTarget::Top)
                .ok_or(Inertia::NoContext)?,
            Action::ScrollBottom => self
                .scroll_focused(ScrollTarget::Bottom)
                .ok_or(Inertia::NoContext)?,
            // New shell / Claude session in the focused context, and
            // reopen the last closed tab.
            Action::NewShellHere => self.new_shell_here(),
            Action::NewClaudeSessionHere => self.new_claude_here().ok_or(Inertia::NoContext)?,
            Action::ReopenClosedTab => self.reopen_closed_tab().ok_or(Inertia::NoContext)?,
            // Capture the current state for the AI dev loop.
            Action::Capture => self.capture(),
            // Start / stop the GIF screencast.
            Action::ToggleRecord => self.toggle_record().ok_or(Inertia::NoContext)?,
            // Zoom re-derives the grid geometry, so the focused terminal is
            // resized like on a window resize; other tabs catch up on
            // focus, the existing convention.
            Action::ZoomIn => self.zoom(termherd_core::Zoom::In),
            Action::ZoomOut => self.zoom(termherd_core::Zoom::Out),
            Action::ZoomReset => self.zoom(termherd_core::Zoom::Reset),
            // Number-row jump straight to a tab. An index past the
            // open tabs is absorbed by `core` as a no-op.
            Action::ActivateTab(index) => self.activate_tab(index),
            // Split the focused pane / move pane focus (FR6).
            Action::SplitHorizontal => self.split_pane(SplitDir::Horizontal),
            Action::SplitVertical => self.split_pane(SplitDir::Vertical),
            Action::FocusNext => self.focus_pane(true),
            Action::FocusPrev => self.focus_pane(false),
            Action::FocusLeft => self.focus_dir(Direction::Left),
            Action::FocusRight => self.focus_dir(Direction::Right),
            Action::FocusUp => self.focus_dir(Direction::Up),
            Action::FocusDown => self.focus_dir(Direction::Down),
            // In the vocabulary since FR9, still unwired: the launch surfaces
            // that exist are `NewShellHere` / `NewClaudeSessionHere`.
            Action::OpenNewSession => return Err(Inertia::NoSurface),
        })
    }

    /// Run a keymap action and say what became of it — the pairing of
    /// [`Self::run_action`] with the verdict it earns, so the ladder and the
    /// control surface report an unwired action the same way.
    pub(super) fn dispatch_action(&mut self, action: Action) -> (KeyVerdict, Task<Message>) {
        let name = action.name();
        match self.run_action(action) {
            Ok(task) => (KeyVerdict::Ran(name), task),
            Err(inertia) => (KeyVerdict::Inert(name, inertia), Task::none()),
        }
    }

    /// Split the focused pane, then resize: the original leaf drops to half its
    /// area and the new one spawns at a default grid, both needing correction.
    fn split_pane(&mut self, dir: SplitDir) -> Task<Message> {
        let effects = self.core.apply(termherd_core::Event::SplitFocused(dir));
        Task::batch([self.perform(effects), self.resize_panes()])
    }

    /// Close the focused pane (FR6). In a split, collapse just that pane and
    /// resize the survivors; a lone pane *is* the whole tab, so fall back to the
    /// tab-close path, which honours the close-confirmation policy for a
    /// still-running session rather than hard-killing it silently.
    /// `None` when there is no tab to close at all, so an empty workspace does
    /// not answer a close with "done".
    fn close_focused_pane(&mut self) -> Option<Task<Message>> {
        let in_split = self
            .core
            .workspace
            .tabs
            .get(self.core.workspace.active)
            .is_some_and(|tab| tab.sessions().len() > 1);
        if in_split {
            let effects = self.core.apply(termherd_core::Event::CloseFocusedPane);
            Some(Task::batch([self.perform(effects), self.resize_panes()]))
        } else {
            self.request_close(self.core.workspace.active)
        }
    }

    /// Move pane focus forward (`next`) or back through the active tab's leaves,
    /// wrapping. Focus alone changes no geometry, so no resize follows.
    fn focus_pane(&mut self, next: bool) -> Task<Message> {
        let event = if next {
            termherd_core::Event::FocusNextPane
        } else {
            termherd_core::Event::FocusPrevPane
        };
        let effects = self.core.apply(event);
        self.perform(effects)
    }

    /// Move pane focus one step in a spatial direction (FR6). Like [`focus_pane`]
    /// it changes no geometry, so no resize follows.
    fn focus_dir(&mut self, dir: Direction) -> Task<Message> {
        let effects = self.core.apply(termherd_core::Event::FocusDir(dir));
        self.perform(effects)
    }

    /// Route a key press (FR4): an open overlay captures it, otherwise it
    /// reaches the focused terminal's PTY. Reports what the ladder did with it
    /// alongside the work it produced — see [`KeyVerdict`].
    pub(super) fn on_key(&mut self, event: keyboard::Event) -> (KeyVerdict, Task<Message>) {
        match self.keyboard_owner() {
            Some(owner) => (
                KeyVerdict::Overlay(owner.label()),
                self.overlay_key(owner, &event),
            ),
            None => self.terminal_key(event),
        }
    }

    /// Which overlay owns the keyboard right now, if any — the precedence ladder
    /// itself. `None` means a key falls through to [`Self::terminal_key`].
    pub(super) fn keyboard_owner(&self) -> Option<KeyboardOwner> {
        if self.tab_rename.is_some() {
            return Some(KeyboardOwner::TabRename);
        }
        if self.renaming.is_some() {
            return Some(KeyboardOwner::SessionRename);
        }
        if self.quit_pending() {
            return Some(KeyboardOwner::Quit);
        }
        if let Some(index) = self.closing {
            return Some(KeyboardOwner::TabClose(index));
        }
        if self.archiving.is_some() {
            return Some(KeyboardOwner::Archive);
        }
        if self.open_doc.is_some() {
            return Some(KeyboardOwner::Doc);
        }
        None
    }

    /// Hand one key press to the overlay that owns the keyboard. The key is
    /// consumed either way — acted on or swallowed — and never leaks to the
    /// terminal beneath the prompt.
    fn overlay_key(&mut self, owner: KeyboardOwner, event: &keyboard::Event) -> Task<Message> {
        match owner {
            KeyboardOwner::TabRename => self.tab_rename_key(event),
            KeyboardOwner::SessionRename => self.session_rename_key(event),
            KeyboardOwner::Quit => self.quit_confirm_key(event),
            KeyboardOwner::TabClose(index) => self.tab_close_confirm_key(event, index),
            KeyboardOwner::Archive => self.archive_confirm_key(event),
            KeyboardOwner::Doc => self.open_doc_key(event),
        }
    }

    /// Escape abandons a tab rename; Enter and a blur commit it elsewhere, so
    /// every other key is swallowed.
    fn tab_rename_key(&mut self, event: &keyboard::Event) -> Task<Message> {
        if is_escape(event) {
            return self.update(Message::CancelTabRename);
        }
        Task::none()
    }

    /// Escape abandons a session rename — the outcome a blur already gives it,
    /// where a tab rename commits. The field owns every other key: it is a live
    /// text input, and Enter reaches it through the widget.
    fn session_rename_key(&mut self, event: &keyboard::Event) -> Task<Message> {
        if is_escape(event) {
            return self.update(Message::CancelRename);
        }
        Task::none()
    }

    fn quit_confirm_key(&mut self, event: &keyboard::Event) -> Task<Message> {
        match classify_confirm(event) {
            ConfirmKey::Confirm => self.update(Message::ConfirmCloseWindow),
            ConfirmKey::Cancel => {
                self.closing_window = None;
                Task::none()
            }
            ConfirmKey::Swallow => Task::none(),
        }
    }

    fn tab_close_confirm_key(&mut self, event: &keyboard::Event, index: usize) -> Task<Message> {
        match classify_confirm(event) {
            ConfirmKey::Confirm => self.close_tab(index),
            ConfirmKey::Cancel => {
                self.closing = None;
                Task::none()
            }
            ConfirmKey::Swallow => Task::none(),
        }
    }

    fn archive_confirm_key(&mut self, event: &keyboard::Event) -> Task<Message> {
        match classify_confirm(event) {
            ConfirmKey::Confirm => self.update(Message::ConfirmArchive),
            ConfirmKey::Cancel => {
                self.archiving = None;
                Task::none()
            }
            ConfirmKey::Swallow => Task::none(),
        }
    }

    /// The doc editor handles its own keys; only the save chord (Cmd/Ctrl+S)
    /// and Escape are intercepted. Escape closes the editor exactly as its own
    /// close button does — unsaved edits included, since a stricter gesture for
    /// the key alone would leave the button's identical hole standing while
    /// looking closed. Discarding a modified doc without asking is a defect on
    /// both paths, tracked on its own.
    fn open_doc_key(&mut self, event: &keyboard::Event) -> Task<Message> {
        if is_escape(event) {
            return self.update(Message::CloseDoc);
        }
        if let keyboard::Event::KeyPressed { key, modifiers, .. } = event
            && modifiers.command()
            && matches!(key, Key::Character(c) if c.as_str() == "s")
        {
            return self.save_open_doc();
        }
        Task::none()
    }

    /// A key no overlay claimed: a bound keymap chord wins over raw input —
    /// resolved before the focus guard so command chords stay global (e.g.
    /// `mod+T` from an empty workspace with the search box focused) — otherwise
    /// it goes to the focused terminal's PTY, leaving plain Ctrl+C as interrupt.
    fn terminal_key(&mut self, event: keyboard::Event) -> (KeyVerdict, Task<Message>) {
        let keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            text,
            location,
            ..
        } = event
        else {
            return (KeyVerdict::Ignored, Task::none());
        };
        if let Some(chord) = chord_of(&key, &physical_key, modifiers)
            && let Some(action) = self.keymap.lookup(&chord)
        {
            return self.dispatch_action(action);
        }
        if self.focus != Focus::Terminal {
            return (KeyVerdict::Ignored, Task::none());
        }
        let Some(session) = self.core.workspace.focused_session() else {
            return (KeyVerdict::Ignored, Task::none());
        };
        // With NumLock on, a numpad key reports its un-locked name (`End`,
        // arrows, …) but carries the digit/operator in `text`; type that rather
        // than the navigation sequence the name would otherwise produce.
        let term_key = numpad_char(location, text.as_deref())
            .map(TermKey::Char)
            .or_else(|| to_term_key(&key));
        let Some(term_key) = term_key else {
            return (KeyVerdict::Ignored, Task::none());
        };
        let Some(bytes) = termherd_pty::key_bytes(term_key, key_mods(modifiers), text.as_deref())
        else {
            return (KeyVerdict::Ignored, Task::none());
        };
        let effects = self
            .core
            .apply(termherd_core::Event::TerminalInput { session, bytes });
        (KeyVerdict::Typed, self.perform(effects))
    }

    /// Whether raw keyboard / IME input should reach the focused terminal: it
    /// holds focus and no overlay (inline rename, close confirmation) is up.
    /// Focus stays `Terminal` while those overlays are open, so the overlay
    /// ladder has to be excluded explicitly — and it is the same ladder
    /// [`Shell::on_key`] walks, read through [`Shell::keyboard_owner`] so the
    /// IME path can't drift from it.
    pub(super) fn accepts_terminal_input(&self) -> bool {
        self.focus == Focus::Terminal && self.keyboard_owner().is_none()
    }

    /// Route IME-composed text (dead/accent keys, CJK) to the focused terminal
    /// as typed bytes. A commit only fires while the terminal accepts
    /// input (see [`Shell::accepts_terminal_input`]), but guard anyway so a
    /// composing overlay (rename / close confirmation) keeps its own typing.
    pub(super) fn on_ime_commit(&mut self, text: String) -> Task<Message> {
        if !self.accepts_terminal_input() || text.is_empty() {
            return Task::none();
        }
        let Some(session) = self.core.workspace.focused_session() else {
            return Task::none();
        };
        let effects = self.core.apply(termherd_core::Event::TerminalInput {
            session,
            bytes: text.into_bytes(),
        });
        self.perform(effects)
    }
}
