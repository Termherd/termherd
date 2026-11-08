//! The tab context menu: a list of keymap actions drawn over the window for the
//! focused tab, opened by a right-click on a tab or by the `open-tab-menu`
//! action, and driven from the keyboard like any other overlay.
//!
//! An in-app overlay rather than a native OS menu: a native menu runs its own
//! event loop past [`Shell::on_key`], so neither a keypress nor an MCP press
//! could ever reach it.

use iced::Task;
use iced::keyboard::{self, Key, key::Named};
use termherd_core::workspace::SessionId;
use termherd_core::{Action, CommandRefusal};

use super::routing::{KeyVerdict, KeyboardOwner, is_escape};
use super::{Message, Shell};
use crate::strings;

/// Which tabs an entry is offered on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Offered {
    OnEveryTab,
    /// Only where the focused pane's Claude has a name to copy, read as the
    /// `copy-agent-name` action reads it — so never on Windows, and on a shell
    /// tab whenever a Claude runs in front of it.
    WithAgentName,
    /// On every tab, but runnable only where a colour can be picked for the
    /// focused pane, as the `pick-tab-color` action decides it. Elsewhere it
    /// is listed disabled with `core`'s refusal, so "unavailable now" never
    /// reads as "missing".
    WhenColorPickable,
    /// Only on an unparked tab, which the action would park.
    WhenUnparked,
    /// Only where a git checkout holds the focused pane's directory.
    InRepo,
}

/// What the focused pane allows, read once when the menu opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Offers {
    pub(super) agent_named: bool,
    /// Why no colour can be picked for the pane; `None` when one can.
    pub(super) color_refusal: Option<CommandRefusal>,
    pub(super) unparked: bool,
    pub(super) in_repo: bool,
}

impl Offers {
    /// How `entry` is listed, or `None` when it is not.
    fn line(&self, entry: &'static Entry) -> Option<Line<'_>> {
        let enabled = Line {
            entry,
            refusal: None,
        };
        match entry.offered {
            Offered::OnEveryTab => Some(enabled),
            Offered::WithAgentName => self.agent_named.then_some(enabled),
            Offered::WhenColorPickable => Some(Line {
                entry,
                refusal: self.color_refusal.as_ref(),
            }),
            Offered::WhenUnparked => self.unparked.then_some(enabled),
            Offered::InRepo => self.in_repo.then_some(enabled),
        }
    }
}

/// A listed entry, and why it cannot run now when it cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Line<'m> {
    pub(super) entry: &'static Entry,
    pub(super) refusal: Option<&'m CommandRefusal>,
}

/// One menu line: the keymap action it runs and the label it shows.
#[derive(Debug, PartialEq, Eq)]
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
        action: Action::PickTabColor,
        label: strings::TAB_MENU_COLOR,
        offered: Offered::WhenColorPickable,
    },
    Entry {
        action: Action::CopyAgentName,
        label: strings::TAB_MENU_COPY_AGENT_NAME,
        offered: Offered::WithAgentName,
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
        action: Action::NewClaudeSessionInWorktree,
        label: strings::TAB_MENU_NEW_CLAUDE_WORKTREE,
        offered: Offered::InRepo,
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
        action: Action::TogglePark,
        label: strings::TAB_MENU_PARK,
        offered: Offered::WhenUnparked,
    },
    Entry {
        action: Action::CloseFocused,
        label: strings::TAB_MENU_CLOSE,
        offered: Offered::OnEveryTab,
    },
];

/// The lines a pane allowing `offers` is offered.
pub(super) fn lines(offers: &Offers) -> impl Iterator<Item = Line<'_>> {
    ENTRIES.iter().filter_map(|entry| offers.line(entry))
}

/// What a key does to a list drawn over the window: the tab menu and the
/// lists it leads to answer the same four keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListKey {
    Up,
    Down,
    Run,
    Leave,
    /// Swallowed, so nothing reaches the keymap or the terminal beneath.
    Other,
}

impl ListKey {
    pub(super) fn of(event: &keyboard::Event) -> Self {
        if is_escape(event) {
            return Self::Leave;
        }
        match event {
            keyboard::Event::KeyPressed {
                key: Key::Named(named),
                ..
            } => match named {
                Named::ArrowUp => Self::Up,
                Named::ArrowDown => Self::Down,
                Named::Enter => Self::Run,
                _ => Self::Other,
            },
            _ => Self::Other,
        }
    }
}

/// The selection one entry down (or up) a list of `len`, wrapping at either
/// end. The list is never empty: every tab is offered the entries that need
/// nothing of its pane.
pub(super) fn step(selected: usize, len: usize, down: bool) -> usize {
    if down {
        (selected + 1) % len
    } else {
        (selected + len - 1) % len
    }
}

/// An open menu.
///
/// Its list is decided once, when it opens, so a position into it stays the
/// entry it named. It is anchored on the pane it opened over: once that pane
/// no longer holds focus — closed, exited, or focus moved by an MCP caller —
/// the menu is gone, rather than offering its entries to whatever is focused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TabMenu {
    anchor: SessionId,
    offers: Offers,
    selected: usize,
}

impl TabMenu {
    pub(super) fn selected(&self) -> usize {
        self.selected
    }

    pub(super) fn lines(&self) -> impl Iterator<Item = Line<'_>> {
        lines(&self.offers)
    }
}

impl Shell {
    /// The open menu, if its anchor still holds focus. Every reader goes
    /// through here, so a menu whose pane went away answers nothing even
    /// before [`Self::drop_stale_lists`] clears it.
    pub(super) fn live_tab_menu(&self) -> Option<&TabMenu> {
        self.tab_menu
            .as_ref()
            .filter(|menu| self.core.workspace.focused_session() == Some(menu.anchor))
    }

    /// Forget a menu or a colour picker whose pane lost focus, so focus coming
    /// back to that pane cannot revive a list nobody reopened. Run before each
    /// message and each MCP press: the two ways anything moves focus.
    pub(super) fn drop_stale_lists(&mut self) {
        if self.live_tab_menu().is_none() {
            self.tab_menu = None;
        }
        if self.live_color_picker().is_none() {
            self.color_picker = None;
        }
    }

    /// Forget a menu or a colour picker anchored on `session`, a pane that just
    /// closed. Its id is never reused, so the anchor check alone would hide the
    /// list for good; dropping it here leaves nothing behind to reason about.
    pub(super) fn forget_lists_on(&mut self, session: SessionId) {
        if self
            .tab_menu
            .as_ref()
            .is_some_and(|menu| menu.anchor == session)
        {
            self.tab_menu = None;
        }
        if self
            .color_picker
            .as_ref()
            .is_some_and(|picker| picker.anchor == session)
        {
            self.color_picker = None;
        }
    }

    /// Open the focused tab's menu on its first entry. `None` when no tab is
    /// open, so there is nothing for the menu to act on.
    pub(super) fn open_tab_menu(&mut self) -> Option<()> {
        let anchor = self.core.workspace.focused_session()?;
        let offers = Offers {
            agent_named: self.focused_agent_name().is_some(),
            color_refusal: self.color_refusal(anchor),
            unparked: self
                .core
                .workspace
                .tabs
                .get(self.core.workspace.active)
                .is_some_and(|tab| !tab.is_parked()),
            in_repo: self.focused_repo().is_some(),
        };
        self.tab_menu = Some(TabMenu {
            anchor,
            offers,
            selected: 0,
        });
        Some(())
    }

    /// A right-click on the tab at `index`: focus it first, since every entry
    /// acts on focus, then open the menu through the action a chord runs. A
    /// tab closed between the render and the click opens nothing.
    pub(super) fn open_tab_menu_at(&mut self, index: usize) -> Task<Message> {
        if index >= self.core.workspace.tabs.len() {
            return Task::none();
        }
        let focus = self.activate_tab(index);
        let (verdict, open) = self.dispatch_action(Action::OpenTabMenu);
        tracing::debug!(index, ?verdict, "tab menu from a right-click");
        Task::batch([focus, open])
    }

    /// The open menu's keys: the arrows move, Enter runs the selected entry,
    /// Escape leaves. Every other key is swallowed, so nothing reaches the
    /// keymap or the terminal beneath the menu.
    ///
    /// Enter answers with the verdict of the entry it ran, so a caller learns
    /// whether that action ran or refused; `None` is the menu's own verdict.
    pub(super) fn tab_menu_key(
        &mut self,
        event: &keyboard::Event,
    ) -> (Option<KeyVerdict>, Task<Message>) {
        let key = ListKey::of(event);
        if key == ListKey::Leave {
            self.tab_menu = None;
            return (None, Task::none());
        }
        let Some((selected, len)) = self
            .live_tab_menu()
            .map(|menu| (menu.selected, menu.lines().count()))
        else {
            return (None, Task::none());
        };
        match key {
            ListKey::Up => self.select_tab_menu_entry(step(selected, len, false)),
            ListKey::Down => self.select_tab_menu_entry(step(selected, len, true)),
            ListKey::Run => return self.run_tab_menu_entry(selected),
            ListKey::Leave | ListKey::Other => {}
        }
        (None, Task::none())
    }

    /// Point the selection at the entry at `position`, as the pointer does.
    pub(super) fn select_tab_menu_entry(&mut self, position: usize) {
        if let Some(menu) = self.tab_menu.as_mut() {
            menu.selected = position;
        }
    }

    /// Close the menu, then run the entry at `position` through the dispatch a
    /// chord uses, answering with its verdict (`None` when there is no such
    /// entry). Closing first matters: an entry that opens an overlay of its
    /// own, such as the rename field, must find the keyboard free.
    ///
    /// A disabled entry runs nothing and leaves the menu open, answering with
    /// its refusal, so a caller learns why rather than seeing the menu vanish.
    pub(super) fn run_tab_menu_entry(
        &mut self,
        position: usize,
    ) -> (Option<KeyVerdict>, Task<Message>) {
        let line = self
            .live_tab_menu()
            .and_then(|menu| menu.lines().nth(position))
            .map(|line| (line.entry, line.refusal.cloned()));
        if let Some((_, Some(refusal))) = line {
            tracing::info!(%refusal, "tab menu entry disabled");
            let verdict = KeyVerdict::Refused(KeyboardOwner::TabMenu.label(), refusal.to_string());
            return (Some(verdict), Task::none());
        }
        self.tab_menu = None;
        let Some((entry, None)) = line else {
            return (None, Task::none());
        };
        let (verdict, task) = self.dispatch_action(entry.action);
        tracing::info!(?verdict, "tab menu entry");
        (Some(verdict), task)
    }
}
