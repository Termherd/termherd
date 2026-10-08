//! The tab strip (FR5): one chip per open session — its activity dot (FR8), a
//! clipped title and a close button — with drag-to-reorder (the insertion
//! caret) and a hover card that reuses the sidebar's session card. The chip
//! styling and the minimal fallback hover card live here; the shared style
//! helpers stay in the parent [`super`].

use std::time::SystemTime;

use iced::widget::{button, column, container, mouse_area, row, text, text_input, tooltip};
use iced::{Color, Element, Fill};
use termherd_core::workspace::Tab;

use termherd_core::ClaudeColor;

use super::modals::modal_card;
use super::{
    COLOR_MARK_WIDTH, CardFacts, card_frame, card_secondary_line, claude_color, clip, detail_lines,
    kind_icon, session_card, status_dot,
};
use crate::shell::color_picker::ColorPicker;
use crate::shell::{Message, Shell, tab_rename_id};
use crate::strings;

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
            let color = self.core.tab_color(index);

            // Double-clicking a chip opens an inline field over it; while that
            // field is up the chip is the editor, not a draggable button — so it
            // emits no drag messages that could dismiss its own edit.
            let renaming_this = self
                .tab_rename
                .as_ref()
                .is_some_and(|(anchor, _)| tab.sessions().contains(anchor));
            // The status dot, then the kind mark — shared by the chip and its
            // rename editor so editing a title never hides what the tab runs.
            let mut inner = row![].spacing(6).align_y(iced::Center);
            if let Some(status) = self.core.tab_status(index) {
                inner = inner.push(status_dot(status));
            }
            if let Some(kind) = self.core.tab_kind(index) {
                inner = inner.push(kind_icon(kind));
            }
            let chip: Element<'_, Message> = if renaming_this {
                let buffer = self.tab_rename.as_ref().map_or("", |(_, b)| b.as_str());
                // The hint is what a blank commit leaves: a shell tab reverts
                // to its derived name, a Claude tab is not renamed at all.
                let blank_leaves = if self.core.tab_names_through_claude(index) {
                    tab.display_title()
                } else {
                    &tab.title
                };
                inner = inner.push(
                    text_input(blank_leaves, buffer)
                        .id(tab_rename_id())
                        .on_input(Message::TabRenameInput)
                        .on_submit(Message::CommitTabRename)
                        .size(12)
                        .padding(2)
                        .width(140.0),
                );
                container(inner)
                    .padding(6)
                    .style(move |theme: &iced::Theme| tab_chip_style(theme, active, false, color))
                    .into()
            } else {
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
                    .style(move |theme: &iced::Theme| {
                        tab_chip_style(theme, active, dragging_this, color)
                    });
                // A press starts a drag; entering another chip moves the drop
                // slot; a double-click opens the inline rename. The release is
                // heard by the shell's window-wide listener, which runs after this
                // press, so a plain click still just activates it. The × captures
                // its own click, so it never starts a drag.
                let chip = mouse_area(chip)
                    .on_press(Message::TabDragStart(index))
                    .on_enter(Message::TabDragOver(index))
                    .on_double_click(Message::StartTabRename(index))
                    .on_right_press(Message::OpenTabMenu(index));
                // The chip clips the title; hovering reveals the fuller
                // description — the sidebar's session card, plus the live pane facts,
                // when the tab resumes a browsed session, else a minimal title +
                // cwd card.
                tooltip(
                    chip,
                    self.tab_hover_card(index, tab, color, now),
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
        // The release that ends a drag is heard window-wide by the shell's
        // subscription, so the strip carries no release or exit handler.
        Some(bar.into())
    }

    /// The open tab menu's card: the focused tab's title, then one line per
    /// entry, the selected one filled. `None` when no menu is open.
    pub(in crate::shell) fn tab_menu_card(&self) -> Option<Element<'_, Message>> {
        let menu = self.live_tab_menu()?;
        let lines = menu
            .entries()
            .map(|entry| text(entry.label).size(12).into());
        Some(list_card(
            vec![list_heading(self.active_tab_title())],
            lines,
            menu.selected(),
            Message::RunTabMenuEntry,
            Message::HoverTabMenuEntry,
        ))
    }

    /// The open colour picker's card: the focused tab's title, why the last
    /// pick was refused when it was, then one line per colour with a swatch
    /// of it beside its name. `None` when no picker is open.
    pub(in crate::shell) fn color_picker_card(&self) -> Option<Element<'_, Message>> {
        let picker = self.live_color_picker()?;
        let mut heading = vec![list_heading(self.active_tab_title())];
        if let Some(reason) = picker.refused() {
            heading.push(card_secondary_line(strings::color_pick_refused(reason)));
        }
        let lines = ColorPicker::colors().iter().map(|&color| {
            row![
                color_swatch(color),
                text(strings::color_choice(color)).size(12)
            ]
            .spacing(8)
            .align_y(iced::Center)
            .into()
        });
        Some(list_card(
            heading,
            lines,
            picker.selected(),
            Message::PickColorPickerEntry,
            Message::HoverColorPickerEntry,
        ))
    }

    /// The focused tab's shown title, the heading of a list drawn over it.
    fn active_tab_title(&self) -> &str {
        self.core
            .workspace
            .tabs
            .get(self.core.workspace.active)
            .map_or("", Tab::display_title)
    }

    /// The hover card for a tab. A tab that resumes a browsed session shows the
    /// [`session_card`] the sidebar does, with the live [`CardFacts`] of its first
    /// pane — one derive (the core resolves the record via
    /// [`termherd_core::App::tab_record`]), no divergent formatting. A shell or a
    /// fresh, not-yet-scanned session has no record, so it falls back to a minimal
    /// card with the full title and the working directory it runs in, under the
    /// same live facts.
    fn tab_hover_card(
        &self,
        index: usize,
        tab: &Tab,
        // The colour the outline shows (the focused pane's), not the record's:
        // in a split the two can differ, and the name is the cue that must
        // match what is drawn.
        color: Option<ClaudeColor>,
        now: SystemTime,
    ) -> Element<'static, Message> {
        let first = tab.first_session();
        let facts = CardFacts {
            agent: self.core.peer_name(first),
            color,
            version: self.core.live_claude_version(first).map(str::to_owned),
            running_for: self
                .core
                .running_since(first)
                .and_then(|spawned| now.duration_since(spawned).ok()),
        };
        match self.core.tab_record(index) {
            Some(record) => session_card(self.core.session_title(record), &facts, record, now),
            None => {
                let cwd = self.core.sessions.get(&first).and_then(|s| s.cwd.clone());
                tab_card(tab.display_title().to_owned(), &facts, cwd)
            }
        }
    }
}

/// A list's heading: the title of the tab it acts on.
fn list_heading<'a>(title: &str) -> Element<'a, Message> {
    text(clip(title, 32)).size(11).into()
}

/// A square of `color` as the chrome paints it; for `default`, an empty frame,
/// the absence of a colour it stands for.
fn color_swatch<'a>(color: ClaudeColor) -> Element<'a, Message> {
    container(text(""))
        .width(12)
        .height(12)
        .style(move |theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: claude_color(color, palette.is_dark).map(iced::Background::Color),
                border: iced::Border {
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}

/// A list drawn over the window for the focused tab: its title, then one
/// line per entry, the `selected` one filled. A click on a line runs it and
/// hovering selects it, so the pointer moves the selection the arrows move.
fn list_card<'a>(
    heading: Vec<Element<'a, Message>>,
    lines: impl Iterator<Item = Element<'a, Message>>,
    selected: usize,
    on_run: fn(usize) -> Message,
    on_hover: fn(usize) -> Message,
) -> Element<'a, Message> {
    let mut card = column(heading).spacing(2).width(240);
    for (position, label) in lines.enumerate() {
        let style = if position == selected {
            button::primary
        } else {
            button::text
        };
        let line = button(label)
            .on_press(on_run(position))
            .style(style)
            .width(Fill)
            .padding([4, 8]);
        card = card.push(mouse_area(line).on_enter(on_hover(position)));
    }
    modal_card(card)
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
/// carried to a ghost; `color`, the one `/color` set, outlines the chip — an
/// outline rather than a fill, so it reads against the strip whichever fill the
/// chip has. All colours but that one come from the theme palette.
fn tab_chip_style(
    theme: &iced::Theme,
    active: bool,
    dragging: bool,
    color: Option<ClaudeColor>,
) -> container::Style {
    let palette = theme.extended_palette();
    let outline = color.and_then(|c| claude_color(c, palette.is_dark));
    let bg = active.then_some(palette.primary.base.color);
    let fg = tab_chip_text(theme, active);
    let fade = |c: Color| super::mix(c, palette.background.base.color, 0.55);
    let (outline_color, outline_width) = match outline {
        Some(c) => (if dragging { fade(c) } else { c }, COLOR_MARK_WIDTH),
        None => (Color::TRANSPARENT, 0.0),
    };
    container::Style {
        background: bg.map(|c| iced::Background::Color(if dragging { fade(c) } else { c })),
        text_color: Some(if dragging { fade(fg) } else { fg }),
        border: iced::Border {
            color: outline_color,
            width: outline_width,
            radius: 4.0.into(),
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
/// session: the full, untruncated title, the live [`CardFacts`] and the
/// working directory it runs in. Styled like [`session_card`] so the two
/// hover surfaces read alike.
fn tab_card(title: String, facts: &CardFacts, cwd: Option<String>) -> Element<'static, Message> {
    let mut card = column![text(title).size(12)].spacing(4);
    for line in detail_lines(facts, None) {
        card = card.push(card_secondary_line(line));
    }
    if let Some(cwd) = cwd {
        card = card.push(card_secondary_line(cwd));
    }
    card_frame(card)
}
