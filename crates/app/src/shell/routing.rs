//! The keyboard routing seam (ARCHITECTURE §8): the modal precedence ladder
//! that decides where a key press goes — a keymap [`Action`], an inline
//! rename / confirmation overlay, or the focused terminal's PTY. Split from the
//! shell's state machine so the precedence wiring lives in one auditable place.

use iced::advanced::widget::{operate, operation::focusable};
use iced::{Task, keyboard};
use termherd_core::workspace::{Direction, SplitDir};
use termherd_core::{Action, ScrollTarget};
use termherd_pty::TermKey;

use super::input::{chord_of, key_mods, numpad_char, to_term_key};
use super::{Focus, Message, Shell, search_id};

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
            // Split the focused pane / move focus between panes (FR6).
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

    /// Split the focused pane, spawning a fresh session beside it, then resize
    /// every pane: the split hands the original leaf half its area, and the new
    /// leaf spawns at a default grid that must be corrected to its sub-rect.
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

    /// Route a key press to the focused terminal's PTY (FR4). Ignored unless a
    /// terminal holds focus, so the search box keeps its own typing.
    pub(super) fn on_key(&mut self, event: keyboard::Event) -> Task<Message> {
        // While renaming a tab inline, the text field owns the keyboard —
        // except Escape, which abandons the edit (Enter commits via the field's
        // own submit; a blur commits through `commits_tab_rename`).
        if self.tab_rename.is_some() {
            if let keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            } = &event
            {
                return self.update(Message::CancelTabRename);
            }
            return Task::none();
        }
        // While renaming inline, let the text field own the keyboard.
        if self.renaming.is_some() {
            return Task::none();
        }
        // The quit modal owns the keyboard while it is up: Enter quits, Escape
        // cancels, every other key is swallowed. Checked first so it wins over
        // the tab/archive prompts beneath it.
        if self.quit_pending() {
            if let keyboard::Event::KeyPressed { key, .. } = &event {
                match key {
                    keyboard::Key::Named(keyboard::key::Named::Enter) => {
                        return self.update(Message::ConfirmCloseWindow);
                    }
                    keyboard::Key::Named(keyboard::key::Named::Escape) => {
                        self.closing_window = None;
                    }
                    _ => {}
                }
            }
            return Task::none();
        }
        // A pending close confirmation captures the keyboard: Enter
        // confirms, Escape cancels, and every other key is swallowed so a
        // keystroke can't slip past to the terminal while the prompt is up.
        if let Some(index) = self.closing {
            if let keyboard::Event::KeyPressed { key, .. } = &event {
                match key {
                    keyboard::Key::Named(keyboard::key::Named::Enter) => {
                        return self.close_tab(index);
                    }
                    keyboard::Key::Named(keyboard::key::Named::Escape) => {
                        self.closing = None;
                    }
                    _ => {}
                }
            }
            return Task::none();
        }
        // A pending archive confirmation likewise owns the keyboard:
        // Enter archives, Escape cancels, other keys are swallowed.
        if self.archiving.is_some() {
            if let keyboard::Event::KeyPressed { key, .. } = &event {
                match key {
                    keyboard::Key::Named(keyboard::key::Named::Enter) => {
                        return self.update(Message::ConfirmArchive);
                    }
                    keyboard::Key::Named(keyboard::key::Named::Escape) => {
                        self.archiving = None;
                    }
                    _ => {}
                }
            }
            return Task::none();
        }
        // An open doc owns the keyboard: the text editor handles keys itself, so
        // swallow them here (never leak to the terminal underneath), but honour
        // the save chord (Cmd/Ctrl+S).
        if self.open_doc.is_some() {
            if let keyboard::Event::KeyPressed { key, modifiers, .. } = &event
                && modifiers.command()
                && matches!(key, keyboard::Key::Character(c) if c.as_str() == "s")
            {
                return self.save_open_doc();
            }
            return Task::none();
        }
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
        // A configured shortcut wins over raw terminal input: build the chord
        // and run its action if the keymap binds one (FR9). Resolved before the
        // terminal-focus guard so command chords are global — `mod+T` opens the
        // first shell even from an empty workspace with the search box focused.
        // Run-action handlers that need a session guard for one
        // themselves. Unbound keys fall through to the terminal, so plain Ctrl+C
        // stays the interrupt signal.
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
        // A numpad key with NumLock on reports its un-locked name (`End`, arrows,
        // …) but carries the digit/operator in `text`; type that instead of the
        // navigation sequence its name would otherwise produce. Other keys map
        // by name as usual.
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
