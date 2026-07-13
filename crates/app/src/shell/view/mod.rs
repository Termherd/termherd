//! The `view` half of the shell: how `Shell` state is rendered (ARCHITECTURE
//! §8). The session browser sidebar (FR1/FR3) and the focused-terminal main
//! pane with its tab strip (FR5), plus the small status-dot and text helpers
//! shared across them. The confirmation modals (quit / tab-close / archive)
//! live in [`modals`]. No state transitions live here — those are in the
//! parent module.

use std::time::SystemTime;

use iced::widget::canvas::Canvas;
use iced::widget::{button, column, container, mouse_area, row, text};
use iced::{Color, Element, Fill, Size};
use termherd_core::SessionRecord;
use termherd_core::SessionStatus;
use termherd_core::browser::relative_age;

use super::geometry::HANDLE_W;
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
        // it occupies — keeping `grid_size` honest rather than estimating.
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
        let screen = focused.and_then(|id| self.screens.get(&id).map(|s| (id, s)));

        let body: Element<'_, Message> = match screen {
            Some((session, screen)) => {
                let canvas = Canvas::new(TerminalView {
                    screen,
                    session,
                    link_modifier: self.link_modifier,
                    font_size: self.core.font_size(),
                })
                .width(Fill)
                .height(Fill);
                // Wrap the grid so the platform IME is on while the terminal is
                // focused — without it dead/accent keys never compose. Off while
                // an overlay (the inline rename field, or a quit / tab-close /
                // archive confirmation modal) is up, so its own field owns
                // composition and a dead key can't leak to the PTY; focus stays
                // `Terminal` underneath those, so they must be excluded
                // explicitly — the same guard `on_key` applies.
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
                mouse_area(composed).on_press(Message::FocusTerminal).into()
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
