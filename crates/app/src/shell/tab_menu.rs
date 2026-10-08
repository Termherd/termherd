//! The tab context menu: a list of keymap actions drawn over the window for the
//! focused tab, opened by a right-click on a tab or by the `open-tab-menu`
//! action, and driven from the keyboard like any other overlay.
//!
//! An in-app overlay rather than a native OS menu: a native menu runs its own
//! event loop past [`Shell::on_key`], so neither a keypress nor an MCP press
//! could ever reach it.

use iced::Task;
use iced::keyboard::{self, Key, key::Named};
use termherd_core::{Action, SessionKind};

use super::{Message, Shell};
use crate::strings;

/// Which tabs an entry is offered on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Offered {
    OnEveryTab,
    /// Only where the focused pane runs Claude. The kind is all the menu
    /// checks; whether that Claude has written a name yet stays the action's
    /// own refusal rather than a second copy of it here.
    OnClaude,
}

impl Offered {
    fn on(self, kind: Option<SessionKind>) -> bool {
        match self {
            Self::OnEveryTab => true,
            Self::OnClaude => kind == Some(SessionKind::Claude),
        }
    }
}

/// One menu line: the keymap action it runs and the label it shows.
#[derive(Debug)]
pub(super) struct Entry {
    pub(super) action: Action,
    pub(super) label: &'static str,
    offered: Offered,
}

/// Every entry, in menu order. A new entry is one line here, provided its
/// action is in the keymap.
pub(super) const ENTRIES: &[Entry] = &[
    Entry {
        action: Action::RenameTab,
        label: strings::TAB_MENU_RENAME,
        offered: Offered::OnEveryTab,
    },
    Entry {
        action: Action::CopyAgentName,
        label: strings::TAB_MENU_COPY_AGENT_NAME,
        offered: Offered::OnClaude,
    },
    Entry {
        action: Action::NewShellHere,
        label: strings::TAB_MENU_NEW_SHELL,
        offered: Offered::OnEveryTab,
    },
    Entry {
        action: Action::NewClaudeSessionHere,
        label: strings::TAB_MENU_NEW_CLAUDE,
        offered: Offered::OnEveryTab,
    },
    Entry {
        action: Action::SplitVertical,
        label: strings::TAB_MENU_SPLIT_RIGHT,
        offered: Offered::OnEveryTab,
    },
    Entry {
        action: Action::SplitHorizontal,
        label: strings::TAB_MENU_SPLIT_DOWN,
        offered: Offered::OnEveryTab,
    },
    Entry {
        action: Action::CloseFocused,
        label: strings::TAB_MENU_CLOSE,
        offered: Offered::OnEveryTab,
    },
];

/// The entries offered on a tab whose focused pane runs `kind` (`None` when
/// it runs nothing the registry knows, which leaves the kind-free entries).
pub(super) fn entries(kind: Option<SessionKind>) -> Vec<&'static Entry> {
    ENTRIES
        .iter()
        .filter(|entry| entry.offered.on(kind))
        .collect()
}

/// An open menu: which entry the keyboard and pointer have selected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct TabMenu {
    selected: usize,
}

impl TabMenu {
    pub(super) fn selected(self) -> usize {
        self.selected
    }

    /// Move the selection one entry down a list of `len`, wrapping to the top.
    pub(super) fn next(&mut self, len: usize) {
        if len > 0 {
            self.selected = (self.selected + 1) % len;
        }
    }

    /// Move the selection one entry up a list of `len`, wrapping to the bottom.
    pub(super) fn prev(&mut self, len: usize) {
        if len > 0 {
            self.selected = (self.selected + len - 1) % len;
        }
    }
}

impl Shell {
    /// The entries the open menu lists, for the focused tab's kind.
    pub(super) fn tab_menu_entries(&self) -> Vec<&'static Entry> {
        entries(self.core.tab_kind(self.core.workspace.active))
    }

    /// Open the focused tab's menu on its first entry. `None` when no tab is
    /// open, so there is nothing for the menu to act on.
    pub(super) fn open_tab_menu(&mut self) -> Option<Task<Message>> {
        self.core.workspace.tabs.get(self.core.workspace.active)?;
        self.tab_menu = Some(TabMenu::default());
        Some(Task::none())
    }

    /// A right-click on the tab at `index`: focus it first, since every entry
    /// acts on focus, then open the menu through the action a chord runs.
    pub(super) fn open_tab_menu_at(&mut self, index: usize) -> Task<Message> {
        let focus = self.activate_tab(index);
        let (verdict, open) = self.dispatch_action(Action::OpenTabMenu);
        tracing::debug!(index, ?verdict, "tab menu from a right-click");
        Task::batch([focus, open])
    }

    /// The open menu's keys: the arrows move, Enter runs the selected entry,
    /// Escape leaves. Every other key is swallowed, so nothing reaches the
    /// keymap or the terminal beneath the menu.
    pub(super) fn tab_menu_key(&mut self, event: &keyboard::Event) -> Task<Message> {
        let keyboard::Event::KeyPressed {
            key: Key::Named(named),
            ..
        } = event
        else {
            return Task::none();
        };
        match named {
            Named::Escape => {
                self.tab_menu = None;
                Task::none()
            }
            Named::ArrowUp => self.step_tab_menu(TabMenu::prev),
            Named::ArrowDown => self.step_tab_menu(TabMenu::next),
            Named::Enter => match self.tab_menu {
                Some(menu) => self.run_tab_menu_entry(menu.selected()),
                None => Task::none(),
            },
            _ => Task::none(),
        }
    }

    fn step_tab_menu(&mut self, step: fn(&mut TabMenu, usize)) -> Task<Message> {
        let len = self.tab_menu_entries().len();
        if let Some(menu) = self.tab_menu.as_mut() {
            step(menu, len);
        }
        Task::none()
    }

    /// Point the selection at the entry at `position`, as the pointer does.
    pub(super) fn select_tab_menu_entry(&mut self, position: usize) {
        if let Some(menu) = self.tab_menu.as_mut() {
            menu.selected = position;
        }
    }

    /// Close the menu, then run the entry at `position` through the dispatch a
    /// chord uses. Closing first matters: an entry that opens an overlay of its
    /// own, such as the rename field, must find the keyboard free.
    ///
    /// A position past the list runs nothing: the focused pane's kind can
    /// change under an open menu and shorten it.
    pub(super) fn run_tab_menu_entry(&mut self, position: usize) -> Task<Message> {
        self.tab_menu = None;
        let Some(entry) = self.tab_menu_entries().get(position).copied() else {
            return Task::none();
        };
        let (verdict, task) = self.dispatch_action(entry.action);
        tracing::info!(?verdict, "tab menu entry");
        task
    }
}
