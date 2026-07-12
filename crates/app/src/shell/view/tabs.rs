//! The tab strip (FR5): one chip per open session — its activity dot (FR8), a
//! clipped title and a close button — with drag-to-reorder (the insertion
//! caret) and a hover card that reuses the sidebar's session card. The chip
//! styling and the minimal fallback hover card live here; the shared style
//! helpers stay in the parent [`super`].

use std::time::SystemTime;

use iced::widget::{button, column, container, mouse_area, row, text, text_input, tooltip};
use iced::{Color, Element};
use termherd_core::workspace::Tab;

use super::{card_secondary_text, card_style, clip, session_card, status_color};
use crate::shell::{Message, Shell, tab_rename_id};

impl Shell {
    /// The tab strip (FR5): one chip per open session, the active one
    /// highlighted, each carrying its activity dot (FR8) and a close button.
    /// `None` when nothing is open, so the welcome view keeps the full pane.
    pub(super) fn tab_bar(&self) -> Option<Element<'_, Message>> {
        let tabs = &self.core.workspace.tabs;
        if tabs.is_empty() {
            return None;
        }
        // One wall-clock read per render feeds every tab's relative "last
        // activity" age, matching the sidebar; the app layer owns the clock.
        let now = SystemTime::now();
        let drag = self.tab_drag.map(|d| (d.from, d.over));
        // Where a release would drop the carried tab: `move_tab` lands it *at*
        // index `over`, so the insertion bar sits before `over` when dragging
        // left and after it when dragging right. `None` until the pointer has
        // actually crossed onto another slot.
        let caret_at = drag.and_then(|(from, over)| match over.cmp(&from) {
            std::cmp::Ordering::Greater => Some(over + 1),
            std::cmp::Ordering::Less => Some(over),
            std::cmp::Ordering::Equal => None,
        });
        let mut bar = row![].spacing(4).align_y(iced::Center);
        for (index, tab) in tabs.iter().enumerate() {
            let active = index == self.core.workspace.active;
            // The carried tab fades to a ghost; the drop point is shown by the
            // insertion bar between chips, not on the chip itself.
            let dragging_this = drag.is_some_and(|(from, _)| from == index);

            // Double-clicking a chip opens an inline field over it; while that
            // field is up the chip is the editor, not a draggable button — so it
            // emits no drag messages that could dismiss its own edit.
            let renaming_this = self.tab_rename.as_ref().is_some_and(|(ri, _)| *ri == index);
            let chip: Element<'_, Message> = if renaming_this {
                let buffer = self.tab_rename.as_ref().map_or("", |(_, b)| b.as_str());
                let mut inner = row![].spacing(6).align_y(iced::Center);
                if let Some(status) = self.core.tab_status(index) {
                    inner = inner.push(text("●").size(9).color(status_color(status)));
                }
                inner = inner.push(
                    text_input("", buffer)
                        .id(tab_rename_id())
                        .on_input(Message::TabRenameInput)
                        .on_submit(Message::CommitTabRename)
                        .size(12)
                        .padding(2)
                        .width(140.0),
                );
                container(inner)
                    .padding(6)
                    .style(move |theme: &iced::Theme| tab_chip_style(theme, active, false))
                    .into()
            } else {
                let mut inner = row![].spacing(6).align_y(iced::Center);
                if let Some(status) = self.core.tab_status(index) {
                    inner = inner.push(text("●").size(9).color(status_color(status)));
                }
                inner = inner.push(text(clip(tab.display_title(), 24)).size(12));
                // The × lives inside the chip so it sits on the active tab's fill,
                // and its colour follows the chip's text so it stays legible there.
                inner = inner.push(
                    button(text("×").size(14))
                        .on_press(Message::RequestCloseTab(index))
                        .style(move |theme: &iced::Theme, _status| button::Style {
                            background: None,
                            text_color: tab_chip_text(theme, active),
                            ..button::Style::default()
                        })
                        .padding(0),
                );
                let chip = container(inner)
                    .padding(6)
                    .style(move |theme: &iced::Theme| tab_chip_style(theme, active, dragging_this));
                // A press starts a drag; entering another chip moves the drop
                // slot; a double-click opens the inline rename. Release/cancel are
                // handled by the strip below, so a plain click (press and release
                // on the same chip) still just activates it. The × button captures
                // its own click, so it never starts a drag.
                let chip = mouse_area(chip)
                    .on_press(Message::TabDragStart(index))
                    .on_enter(Message::TabDragOver(index))
                    .on_double_click(Message::StartTabRename {
                        index,
                        current: tab.display_title().to_owned(),
                    });
                // The chip clips the title; hovering reveals the fuller
                // description — the very session card the sidebar shows when the
                // tab resumes a browsed session, else a minimal title + cwd card.
                tooltip(
                    chip,
                    self.tab_hover_card(index, tab, now),
                    tooltip::Position::Bottom,
                )
                .into()
            };
            if caret_at == Some(index) {
                bar = bar.push(insertion_caret());
            }
            bar = bar.push(chip);
        }
        // A drop past the last tab parks the bar at the strip's end.
        if caret_at == Some(tabs.len()) {
            bar = bar.push(insertion_caret());
        }
        // One release anywhere over the strip ends the drag (committing the
        // reorder at the last-hovered slot); leaving the strip abandons it.
        Some(
            mouse_area(bar)
                .on_release(Message::TabDragEnd)
                .on_exit(Message::TabDragCancel)
                .into(),
        )
    }

    /// The hover card for a tab. A tab that resumes a browsed session
    /// shows the *same* [`session_card`] the sidebar does — one derive (the core
    /// resolves the record via [`termherd_core::App::tab_record`]), no divergent
    /// formatting. A shell or a fresh, not-yet-scanned session has no record, so
    /// it falls back to a minimal card with the full title and the working
    /// directory it runs in.
    fn tab_hover_card(
        &self,
        index: usize,
        tab: &Tab,
        now: SystemTime,
    ) -> Element<'static, Message> {
        match self.core.tab_record(index) {
            Some(record) => session_card(self.core.session_title(record), record, now),
            None => {
                let cwd = tab
                    .sessions()
                    .first()
                    .and_then(|id| self.core.sessions.get(id))
                    .and_then(|s| s.cwd.clone());
                tab_card(tab.display_title().to_owned(), cwd)
            }
        }
    }
}

/// A tab chip's text colour: the primary tier on the active (filled) chip, the
/// background tier otherwise, so the label stays legible on either fill.
fn tab_chip_text(theme: &iced::Theme, active: bool) -> Color {
    let palette = theme.extended_palette();
    if active {
        palette.primary.base.text
    } else {
        palette.background.base.text
    }
}

/// A tab chip's look, now a styled container rather than a button (the
/// drag needs `mouse_area` to see press *and* release, which a button would
/// capture). `active` paints the primary fill; `dragging` fades the tab being
/// carried to a ghost. All colours come from the theme palette — never
/// hardcoded.
fn tab_chip_style(theme: &iced::Theme, active: bool, dragging: bool) -> container::Style {
    let palette = theme.extended_palette();
    let bg = active.then_some(palette.primary.base.color);
    let fg = tab_chip_text(theme, active);
    let fade = |c: Color| super::mix(c, palette.background.base.color, 0.55);
    container::Style {
        background: bg.map(|c| iced::Background::Color(if dragging { fade(c) } else { c })),
        text_color: Some(if dragging { fade(fg) } else { fg }),
        border: iced::Border {
            radius: 4.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    }
}

/// The vertical insertion bar shown between chips during a drag — the
/// legible "it drops here" marker, painted in the theme accent.
fn insertion_caret<'a>() -> Element<'a, Message> {
    container(text(""))
        .width(3)
        .height(24)
        .style(|theme: &iced::Theme| container::Style {
            background: Some(theme.extended_palette().primary.strong.color.into()),
            border: iced::Border {
                radius: 2.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// The minimal hover card for a tab with no browsed record — a shell or a fresh
/// session: the full, untruncated title and the working directory it runs
/// in. Styled like [`session_card`] so the two hover surfaces read alike.
fn tab_card(title: String, cwd: Option<String>) -> Element<'static, Message> {
    let mut card = column![text(title).size(12)].spacing(4);
    if let Some(cwd) = cwd {
        card = card.push(text(cwd).size(10).style(card_secondary_text));
    }
    container(card)
        .padding(8)
        .max_width(360.0)
        .style(card_style)
        .into()
}
