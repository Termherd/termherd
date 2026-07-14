//! The `view` half of the shell: how `Shell` state is rendered (ARCHITECTURE
//! §8). The session browser sidebar (FR1/FR3) and the focused-terminal main
//! pane with its tab strip (FR5), plus the small status-dot and text helpers
//! shared across them. The confirmation modals (quit / tab-close / archive)
//! live in [`modals`]. No state transitions live here — those are in the
//! parent module.

use std::time::SystemTime;

use iced::widget::canvas::Canvas;
use iced::widget::{button, column, container, mouse_area, row, text};
use iced::{Border, Color, Element, Fill, Length, Size};
use termherd_core::SessionRecord;
use termherd_core::SessionStatus;
use termherd_core::browser::relative_age;
use termherd_core::workspace::{Pane, SessionId, SplitDir};

use super::geometry::{HANDLE_W, PANE_BORDER, PANE_PAD};
use super::ime::ime_area;
use super::terminal::{TerminalView, cell_size};
use super::{Message, Shell};
use crate::strings;

mod doc_editor;
mod modals;
mod sidebar;
mod style;
mod tabs;

use doc_editor::doc_editor;
use modals::modal;
use style::{card_secondary_text, card_style, clip, mix, sidebar_secondary_text, status_color};

impl Shell {
    pub(super) fn view(&self) -> Element<'_, Message> {
        // Hiding the sidebar hands its width to the terminal; a slim
        // always-present handle brings it back without needing the shortcut.
        // The handle is pinned to `HANDLE_W` so the grid reserves exactly what
        // it occupies — keeping the pane geometry honest rather than estimating.
        let base: Element<'_, Message> = if self.core.sidebar_hidden {
            let handle = container(
                button(text("▶").size(12))
                    .on_press(Message::ToggleSidebar)
                    .style(button::text)
                    .padding(4),
            )
            .width(HANDLE_W)
            .padding(4);
            row![handle, self.main_pane()].into()
        } else {
            row![self.sidebar(), self.main_pane()].into()
        };
        // Any armed confirmation — quit, tab-close or archive — overlays the
        // same centred modal, so the about-to-change sessions stay untouchable
        // until the user decides. `active_confirmation` picks the one in force.
        match self.active_confirmation() {
            Some((card, on_cancel)) => modal(base, card, on_cancel),
            None => base,
        }
    }

    /// The focused terminal: a status badge, then its grid drawn on a canvas.
    /// With no session open, a short summary of what the browser found.
    fn main_pane(&self) -> Element<'_, Message> {
        // A plan / memory doc, when one is open, takes over the main pane for
        // viewing/editing (F-plans-memory).
        if let Some(doc) = &self.open_doc {
            return doc_editor(doc);
        }

        let focused = self.core.workspace.focused_session();

        let body: Element<'_, Message> =
            match self.core.workspace.tabs.get(self.core.workspace.active) {
                // A lone terminal needs no focus border — nothing to
                // disambiguate — so only a split renders bordered.
                Some(tab) => {
                    let split = matches!(tab.root, Pane::Split { .. });
                    self.render_pane(&tab.root, focused, split)
                }
                None => {
                    let total: usize = self.core.projects.iter().map(|g| g.sessions.len()).sum();
                    iced::widget::center(
                        column![
                            text("TermHerd").size(40),
                            text(strings::welcome_counts(total, self.core.projects.len())).size(14),
                            text(strings::WELCOME_HINT_OPEN).size(13),
                            text(strings::WELCOME_HINT_RESUME).size(13),
                        ]
                        .spacing(8)
                        .align_x(iced::Center),
                    )
                    .height(Fill)
                    .into()
                }
            };

        let mut pane = column![].spacing(8).padding(8);
        if let Some(bar) = self.tab_bar() {
            pane = pane.push(bar);
        }
        if let Some(indicator) = self.recording_indicator() {
            pane = pane.push(indicator);
        }
        if let Some(status) = focused.and_then(|id| self.core.sessions.get(&id)) {
            pane = pane.push(status_badge(status.status));
        }
        container(pane.push(body)).width(Fill).height(Fill).into()
    }

    /// Render the pane tree (FR6): a leaf is its terminal; a split becomes a
    /// `row!` (vertical divider) or `column!` (horizontal) sharing space at its
    /// ratio. Derived from the `core` tree each frame, so it can't drift.
    fn render_pane(
        &self,
        pane: &Pane,
        focused: Option<SessionId>,
        bordered: bool,
    ) -> Element<'_, Message> {
        match pane {
            Pane::Leaf(session) => {
                self.terminal_leaf(*session, focused == Some(*session), bordered)
            }
            Pane::Split { dir, ratio, a, b } => {
                let a_el = self.render_pane(a, focused, bordered);
                let b_el = self.render_pane(b, focused, bordered);
                let pa = ((ratio * 100.0).round() as u16).clamp(1, 99);
                let pb = 100 - pa;
                // No inter-pane spacing: the per-pane borders already separate
                // them, and a gap here would be geometry `resize_panes` cannot
                // see, drifting the PTY grid from the visible canvas.
                match dir {
                    SplitDir::Vertical => row![
                        container(a_el).width(Length::FillPortion(pa)),
                        container(b_el).width(Length::FillPortion(pb)),
                    ]
                    .into(),
                    SplitDir::Horizontal => column![
                        container(a_el).height(Length::FillPortion(pa)),
                        container(b_el).height(Length::FillPortion(pb)),
                    ]
                    .into(),
                }
            }
        }
    }

    /// One leaf: its terminal, click-to-focus, and (in a split) a focus border.
    /// The focused leaf carries the IME so composition follows the keyboard; a
    /// pane with no output yet holds an empty slot to keep the layout stable.
    fn terminal_leaf(
        &self,
        session: SessionId,
        is_focused: bool,
        bordered: bool,
    ) -> Element<'_, Message> {
        let inner: Element<'_, Message> = match self.screens.get(&session) {
            Some(screen) => {
                let canvas = Canvas::new(TerminalView {
                    screen,
                    session,
                    link_modifier: self.link_modifier,
                    shift: self.shift_modifier,
                    font_size: self.core.font_size(),
                    dimmed: !self.core.window_focused(),
                })
                .width(Fill)
                .height(Fill);
                // IME only on the focused leaf, and only when no overlay owns
                // the keyboard — the same guard `on_key` applies. Others just
                // take a click to focus.
                if is_focused {
                    let composed = ime_area(
                        canvas,
                        self.accepts_terminal_input(),
                        screen.cursor,
                        {
                            let (cw, ch) = cell_size(self.core.font_size());
                            Size::new(cw, ch)
                        },
                        Message::ImeCommit,
                    );
                    mouse_area(composed)
                        .on_press(Message::FocusPane(session))
                        .into()
                } else {
                    mouse_area(canvas)
                        .on_press(Message::FocusPane(session))
                        .into()
                }
            }
            None => container(text("")).width(Fill).height(Fill).into(),
        };
        // A lone pane needs no frame — nothing to distinguish it from.
        if !bordered {
            return inner;
        }
        let window_focused = self.core.window_focused();
        container(inner)
            .width(Fill)
            .height(Fill)
            .padding(PANE_PAD)
            .style(move |theme: &iced::Theme| {
                // Every split pane is outlined so the layout is legible; the
                // focused one gets a thicker, accent-coloured border so which
                // terminal holds the keyboard is unmistakable, the rest a thin
                // muted one. An unfocused window mutes even the focus accent —
                // no pane in it holds the keyboard.
                let palette = theme.extended_palette();
                let (color, width) = if is_focused && window_focused {
                    (palette.primary.strong.color, PANE_BORDER)
                } else {
                    (palette.background.strong.color, PANE_BORDER / 2.0)
                };
                container::Style {
                    border: Border {
                        color,
                        width,
                        radius: 3.0.into(),
                    },
                    ..container::Style::default()
                }
            })
            .into()
    }

    /// The `● REC n/cap` indicator shown while a GIF screencast records,
    /// so the recording state — and how close it is to the auto-stop cap — is
    /// unmistakable. `None` when not recording. Independent of the tab strip, so
    /// it shows even on an empty workspace.
    fn recording_indicator(&self) -> Option<Element<'_, Message>> {
        // The shared alert red (matches `Attention` / the editor error note), so
        // the recording cue never drifts from the rest of the palette.
        const REC: Color = Color::from_rgb(0.95, 0.35, 0.35);
        let (frames, cap) = self.core.recording_progress()?;
        // Show the frame *being captured* (1-based), so the count climbs
        // 1/cap → cap/cap instead of stopping one short — the cap tick captures
        // the final frame and ends the recording in the same step.
        let shown = (frames + 1).min(cap);
        Some(
            container(text(format!("● REC {shown}/{cap}")).size(12).color(REC))
                .padding([2, 8])
                .into(),
        )
    }
}

/// A small per-session activity badge (FR8): a coloured dot + label for the
/// focused terminal. The same dot annotates live rows in the sidebar and each
/// tab in the tab strip.
fn status_badge(status: SessionStatus) -> Element<'static, Message> {
    row![
        text("●").size(13).color(status_color(status)),
        text(strings::status_label(status)).size(13)
    ]
    .spacing(6)
    .align_y(iced::Center)
    .into()
}

/// The hover card for a session row: full title, a muted line with relative
/// last activity and message count, then the last few transcript lines so a
/// duplicate-looking session is recognisable without opening it.
pub(super) fn session_card(
    title: String,
    session: &SessionRecord,
    now: SystemTime,
) -> Element<'static, Message> {
    let count = session.digest.message_count;

    let age = session
        .modified
        .and_then(|m| now.duration_since(m).ok())
        .map(relative_age);
    let meta = strings::session_meta(age.as_deref(), count);

    // Title inherits the card's text colour; secondary lines are dimmed. Both
    // colours come from the theme palette (see `card_style`), never hardcoded.
    let mut card = column![
        text(title).size(12),
        text(meta).size(10).style(card_secondary_text)
    ]
    .spacing(4);
    for line in &session.digest.tail {
        card = card.push(
            text(format!("› {line}"))
                .size(10)
                .style(card_secondary_text),
        );
    }
    container(card)
        .padding(8)
        .max_width(360.0)
        .style(card_style)
        .into()
}
