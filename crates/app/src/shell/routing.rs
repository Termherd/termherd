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

/// Enter confirms, Escape cancels; everything else (and any non-press event) is
/// swallowed so it can't reach the terminal beneath the prompt.
fn classify_confirm(event: &keyboard::Event) -> ConfirmKey {
    match event {
        keyboard::Event::KeyPressed {
            key: Key::Named(Named::Enter),
            ..
        } => ConfirmKey::Confirm,
        keyboard::Event::KeyPressed {
            key: Key::Named(Named::Escape),
            ..
        } => ConfirmKey::Cancel,
        _ => ConfirmKey::Swallow,
    }
}

impl Shell {
    /// Run a keymap [`Action`] (FR9). Clipboard actions become iced tasks; tab
    /// actions drive `core`. Actions without a surface yet are no-ops.
    pub(super) fn run_action(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::Copy => self.copy_selection(),
            Action::Paste => iced::clipboard::read().map(Message::Paste),
            Action::NextTab => self.cycle_tab(1),
            Action::PrevTab => self.cycle_tab(-1),
            Action::CloseFocused => self.close_focused_pane(),
            Action::FocusSearch => {
                self.focus = Focus::Search;
                operate(focusable::focus(search_id()))
            }
            Action::ToggleSidebar => self.toggle_sidebar(),
            Action::ScrollTop => self.scroll_focused(ScrollTarget::Top),
            Action::ScrollBottom => self.scroll_focused(ScrollTarget::Bottom),
            // New shell / Claude session in the focused context, and
            // reopen the last closed tab.
            Action::NewShellHere => self.new_shell_here(),
            Action::NewClaudeSessionHere => self.new_claude_here(),
            Action::ReopenClosedTab => self.reopen_closed_tab(),
            // Capture the current state for the AI dev loop.
            Action::Capture => self.capture(),
            // Start / stop the GIF screencast.
            Action::ToggleRecord => self.toggle_record(),
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
            Action::OpenNewSession => Task::none(),
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
    fn close_focused_pane(&mut self) -> Task<Message> {
        let in_split = self
            .core
            .workspace
            .tabs
            .get(self.core.workspace.active)
            .is_some_and(|tab| tab.sessions().len() > 1);
        if in_split {
            let effects = self.core.apply(termherd_core::Event::CloseFocusedPane);
            Task::batch([self.perform(effects), self.resize_panes()])
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
    /// reaches the focused terminal's PTY.
    pub(super) fn on_key(&mut self, event: keyboard::Event) -> Task<Message> {
        match self.overlay_key(&event) {
            Some(task) => task,
            None => self.terminal_key(event),
        }
    }

    /// The overlays that own the keyboard while open, in precedence order — the
    /// quit modal wins over the tab/archive prompts beneath it. `Some` means the
    /// key was consumed (acted on or swallowed, never leaking to the terminal);
    /// `None` falls through to [`Self::terminal_key`].
    fn overlay_key(&mut self, event: &keyboard::Event) -> Option<Task<Message>> {
        if self.tab_rename.is_some() {
            return Some(self.tab_rename_key(event));
        }
        if self.renaming.is_some() {
            return Some(Task::none());
        }
        if self.quit_pending() {
            return Some(self.quit_confirm_key(event));
        }
        if let Some(index) = self.closing {
            return Some(self.tab_close_confirm_key(event, index));
        }
        if self.archiving.is_some() {
            return Some(self.archive_confirm_key(event));
        }
        if self.open_doc.is_some() {
            return Some(self.open_doc_key(event));
        }
        None
    }

    /// Escape abandons a tab rename; Enter and a blur commit it elsewhere, so
    /// every other key is swallowed.
    fn tab_rename_key(&mut self, event: &keyboard::Event) -> Task<Message> {
        if let keyboard::Event::KeyPressed {
            key: Key::Named(Named::Escape),
            ..
        } = event
        {
            return self.update(Message::CancelTabRename);
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

    /// The doc editor handles its own keys; only the save chord (Cmd/Ctrl+S) is
    /// intercepted.
    fn open_doc_key(&mut self, event: &keyboard::Event) -> Task<Message> {
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
    fn terminal_key(&mut self, event: keyboard::Event) -> Task<Message> {
        let keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            text,
            location,
            ..
        } = event
        else {
            return Task::none();
        };
        if let Some(chord) = chord_of(&key, &physical_key, modifiers)
            && let Some(action) = self.keymap.lookup(&chord)
        {
            return self.run_action(action);
        }
        if self.focus != Focus::Terminal {
            return Task::none();
        }
        let Some(session) = self.core.workspace.focused_session() else {
            return Task::none();
        };
        // With NumLock on, a numpad key reports its un-locked name (`End`,
        // arrows, …) but carries the digit/operator in `text`; type that rather
        // than the navigation sequence the name would otherwise produce.
        let term_key = numpad_char(location, text.as_deref())
            .map(TermKey::Char)
            .or_else(|| to_term_key(&key));
        let Some(term_key) = term_key else {
            return Task::none();
        };
        let Some(bytes) = termherd_pty::key_bytes(term_key, key_mods(modifiers), text.as_deref())
        else {
            return Task::none();
        };
        let effects = self
            .core
            .apply(termherd_core::Event::TerminalInput { session, bytes });
        self.perform(effects)
    }

    /// Whether raw keyboard / IME input should reach the focused terminal: it
    /// holds focus and no overlay (inline rename, close confirmation) is up.
    /// Focus stays `Terminal` while those overlays are open, so they have to be
    /// excluded explicitly — this is the predicate [`Shell::on_key`] enforces
    /// step by step, shared so the IME path can't drift from it.
    pub(super) fn accepts_terminal_input(&self) -> bool {
        self.focus == Focus::Terminal
            && self.renaming.is_none()
            && self.tab_rename.is_none()
            && self.closing.is_none()
            && self.archiving.is_none()
            && self.open_doc.is_none()
            && !self.quit_pending()
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
