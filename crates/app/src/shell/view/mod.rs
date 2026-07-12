//! The `view` half of the shell: how `Shell` state is rendered (ARCHITECTURE
//! §8). The session browser sidebar (FR1/FR3) and the focused-terminal main
//! pane with its tab strip (FR5), plus the small status-dot and text helpers
//! shared across them. The confirmation modals (quit / tab-close / archive)
//! live in [`modals`]. No state transitions live here — those are in the
//! parent module.

use std::time::SystemTime;

use iced::widget::canvas::Canvas;
use iced::widget::{button, column, container, mouse_area, row, text, text_editor};
use iced::{Color, Element, Fill, Font, Size};
use termherd_core::SessionRecord;
use termherd_core::SessionStatus;
use termherd_core::browser::relative_age;

use super::ime::ime_area;
use super::terminal::{TerminalView, cell_size};
use super::{DocFeedback, HANDLE_W, Message, OpenDoc, Shell};
use crate::strings;

mod modals;
mod sidebar;
mod tabs;

use modals::modal;

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

/// Render an open plan/memory doc: a header (label, save state, actions) above
/// the editable text area (F-plans-memory). A read-only doc omits the Save
/// control; the header always carries a close button.
fn doc_editor(doc: &OpenDoc) -> Element<'_, Message> {
    let mut header = row![text(&doc.label).size(13)]
        .spacing(12)
        .align_y(iced::Center);

    if doc.writable {
        // A disabled button (no `on_press`) reads as "nothing to save".
        let mut save = button(text(strings::DOC_SAVE).size(12))
            .style(button::text)
            .padding(0);
        if doc.dirty {
            save = save.on_press(Message::SaveDoc);
        }
        header = header.push(save);
    }

    if let Some(note) = save_note(doc) {
        header = header.push(note);
    }

    header = header.push(
        button(text(strings::DOC_CLOSE).size(12))
            .on_press(Message::CloseDoc)
            .style(button::text)
            .padding(0),
    );

    let editor = text_editor(&doc.content)
        .on_action(Message::DocEdit)
        .font(Font::MONOSPACE)
        .size(12)
        .height(Fill);

    container(column![header, editor].spacing(8).padding(8))
        .width(Fill)
        .height(Fill)
        .into()
}

/// The save-state note for the editor header: the last save outcome if there is
/// one, else a "modified" hint while there are unsaved edits, else nothing.
fn save_note(doc: &OpenDoc) -> Option<Element<'_, Message>> {
    const SAVED: Color = Color::from_rgb(0.3, 0.8, 0.4);
    const ERROR: Color = Color::from_rgb(0.95, 0.35, 0.35);
    const DIRTY: Color = Color::from_rgb(0.6, 0.6, 0.6);

    match &doc.feedback {
        Some(DocFeedback::Saved) => Some(text(strings::DOC_SAVED).size(11).color(SAVED).into()),
        Some(DocFeedback::Error(message)) => {
            Some(text(message.clone()).size(11).color(ERROR).into())
        }
        None if doc.dirty => Some(text(strings::DOC_MODIFIED).size(11).color(DIRTY).into()),
        None => None,
    }
}

/// The dot colour for an activity status (FR8). Shared by the focused-terminal
/// badge and the sidebar's per-session dot so both stay in sync; the matching
/// label lives in [`strings::status_label`].
pub(super) fn status_color(status: SessionStatus) -> Color {
    match status {
        SessionStatus::Starting => Color::from_rgb(0.55, 0.55, 0.6),
        SessionStatus::Busy => Color::from_rgb(0.95, 0.7, 0.2),
        SessionStatus::Idle => Color::from_rgb(0.3, 0.8, 0.4),
        SessionStatus::Attention => Color::from_rgb(0.95, 0.35, 0.35),
        SessionStatus::Exited => Color::from_rgb(0.5, 0.5, 0.5),
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

/// Background for the session hover card — a step away from the surrounding
/// surface (the `strong` palette tier rather than the default `weak`) so the
/// card reads as a distinct floating layer, with a thin border to seal it.
/// Everything is pulled from the theme palette, so it tracks the theme system
/// once that lands rather than baking in a colour.
pub(super) fn card_style(theme: &iced::Theme) -> container::Style {
    let surface = card_surface(theme);
    container::Style {
        background: Some(surface.color.into()),
        text_color: Some(surface.text),
        border: iced::Border {
            color: theme.extended_palette().background.weak.color,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    }
}

pub(super) fn card_secondary_text(theme: &iced::Theme) -> iced::widget::text::Style {
    let surface = card_surface(theme);
    iced::widget::text::Style {
        color: Some(mix(surface.text, surface.color, 0.35)),
    }
}

/// The palette tier the hover card paints on — its surface colour and the text
/// colour meant to sit on it. Single-sourced so the "which tier" choice (and
/// the eventual theme-system wiring) lives in one place.
fn card_surface(theme: &iced::Theme) -> iced::theme::palette::Pair {
    theme.extended_palette().background.strong
}

/// Linear blend from `a` to `b` by `t` in `[0, 1]`.
pub(super) fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
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

/// Collapse newlines to spaces and truncate to `max` characters with an ellipsis.
pub(super) fn clip(s: &str, max: usize) -> String {
    let cleaned: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        let mut out: String = cleaned.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
