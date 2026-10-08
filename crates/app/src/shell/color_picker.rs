//! The tab colour picker: `/color`'s palette drawn over the window for the
//! focused tab, opened by the `pick-tab-color` action — which the tab menu's
//! colour entry runs — and driven from the keyboard like the tab menu.
//!
//! Where a pick goes is the pane's [`ColorKeeper`]'s to say. A Claude pane is
//! asked, by typing `/color` behind the confirmation every slash command goes
//! through, and shows the colour once its transcript records it; a shell tab
//! keeps the colour in `core`. No Claude pane is coloured locally, so nothing
//! can disagree with Claude's own prompt bar.

use iced::Task;
use iced::keyboard;
use termherd_core::workspace::SessionId;
use termherd_core::{ClaudeColor, ClaudeCommand, ColorKeeper, Event};

use super::routing::{KeyVerdict, KeyboardOwner};
use super::tab_menu::{ListKey, step};
use super::{Message, Shell};

/// An open picker, anchored on the pane it opened over, as the tab menu is:
/// once that pane no longer holds focus the picker is gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ColorPicker {
    anchor: SessionId,
    selected: usize,
    /// Why the last pick for a Claude pane could not be asked of it, shown in
    /// the picker so the user learns it rather than seeing the pick vanish.
    refused: Option<String>,
}

impl ColorPicker {
    /// What the picker offers, in order: the palette `/color` takes.
    pub(super) fn colors() -> impl Iterator<Item = ClaudeColor> {
        ClaudeColor::ALL.into_iter()
    }

    pub(super) fn selected(&self) -> usize {
        self.selected
    }

    pub(super) fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }
}

impl Shell {
    /// The open picker, if its anchor still holds focus.
    pub(super) fn live_color_picker(&self) -> Option<&ColorPicker> {
        self.color_picker
            .as_ref()
            .filter(|picker| self.core.workspace.focused_session() == Some(picker.anchor))
    }

    /// Open the focused tab's picker on the colour it wears, else the first.
    /// `None` when no pane is focused, or when the pane is a Claude that could
    /// not take `/color` now — busy, or gone and left a shell behind — so the
    /// keymap reports a refusal instead of offering a list that cannot work.
    pub(super) fn open_color_picker(&mut self) -> Option<()> {
        let anchor = self.core.workspace.focused_session()?;
        if self.core.color_keeper(anchor)? == ColorKeeper::Claude {
            self.core
                .claude_command_check(anchor, &self.prompt_input(anchor))
                .ok()?;
        }
        let selected = self
            .core
            .session_color(anchor)
            .and_then(|worn| ColorPicker::colors().position(|color| color == worn))
            .unwrap_or(0);
        self.color_picker = Some(ColorPicker {
            anchor,
            selected,
            refused: None,
        });
        Some(())
    }

    /// The open picker's keys: the arrows move, Enter picks, Escape leaves.
    /// Every other key is swallowed. `None` is the picker's own verdict.
    pub(super) fn color_picker_key(
        &mut self,
        event: &keyboard::Event,
    ) -> (Option<KeyVerdict>, Task<Message>) {
        let key = ListKey::of(event);
        if key == ListKey::Leave {
            self.color_picker = None;
            return (None, Task::none());
        }
        let Some(selected) = self.live_color_picker().map(ColorPicker::selected) else {
            return (None, Task::none());
        };
        let len = ClaudeColor::ALL.len();
        match key {
            ListKey::Up => self.select_color(step(selected, len, false)),
            ListKey::Down => self.select_color(step(selected, len, true)),
            ListKey::Run => return self.pick_color(selected),
            ListKey::Leave | ListKey::Other => {}
        }
        (None, Task::none())
    }

    /// Point the selection at the colour at `position`, as the pointer does.
    pub(super) fn select_color(&mut self, position: usize) {
        if let Some(picker) = self.color_picker.as_mut() {
            picker.selected = position;
        }
    }

    /// Pick the colour at `position` for the picker's pane and close the
    /// picker. A Claude pane gets the `/color` confirmation; when it cannot
    /// take it, the picker stays open naming why, and the verdict says the pick
    /// was refused rather than letting a caller believe it was asked.
    pub(super) fn pick_color(&mut self, position: usize) -> (Option<KeyVerdict>, Task<Message>) {
        let anchor = self.live_color_picker().map(|picker| picker.anchor);
        self.color_picker = None;
        let (Some(anchor), Some(color)) = (anchor, ClaudeColor::ALL.get(position).copied()) else {
            return (None, Task::none());
        };
        if self.core.color_keeper(anchor) == Some(ColorKeeper::Claude) {
            return match self.arm_claude_command(anchor, ClaudeCommand::Color(color)) {
                Ok(_) => (None, Task::none()),
                Err(refusal) => {
                    let reason = refusal.to_string();
                    tracing::info!(%reason, "tab colour not asked of Claude");
                    self.color_picker = Some(ColorPicker {
                        anchor,
                        selected: position,
                        refused: Some(reason.clone()),
                    });
                    let label = KeyboardOwner::ColorPicker.label();
                    (Some(KeyVerdict::Refused(label, reason)), Task::none())
                }
            };
        }
        let index = self.core.workspace.active;
        let effects = self.core.apply(Event::SetTabColor { index, color });
        (None, self.perform(effects))
    }
}
