//! The `view` half of the shell: how `Shell` state is rendered (ARCHITECTURE
//! §8). The session browser sidebar (FR1/FR3) and the focused-terminal main
//! pane with its tab strip (FR5), plus the small status-dot and text helpers
//! shared across them. The confirmation modals (quit / tab-close / archive)
//! live in [`modals`]. No state transitions live here — those are in the
//! parent module.

use std::time::{Duration, SystemTime};

use iced::widget::canvas::Canvas;
use iced::widget::{Column, button, column, container, mouse_area, row, text};
use iced::{Border, Color, Element, Fill, Length, Size};
use termherd_claude::digest::SessionDigest;
use termherd_core::SessionRecord;
use termherd_core::browser::{compact_elapsed, relative_age};
use termherd_core::workspace::{Pane, SessionId, SplitDir};

use super::geometry::{HANDLE_W, PANE_BORDER, PANE_PAD};
use super::ime::ime_area;
use super::terminal::{TerminalView, cell_size};
use super::{Message, Shell};
use crate::strings;

mod doc_editor;
mod modals;
mod settings_panel;
mod sidebar;
mod style;
mod tabs;

use doc_editor::doc_editor;
use modals::modal;
use style::{
    card_secondary_text, card_style, clip, kind_glyph, kind_icon, mix, sidebar_secondary_text,
    status_dot,
};

impl Shell {
    pub(super) fn view(&self) -> Element<'_, Message> {
        // Hiding the sidebar hands its width to the terminal; a slim
        // always-present handle brings it back without needing the shortcut.
        // The handle is pinned to `HANDLE_W` so the grid reserves exactly what
        // it occupies — keeping the pane geometry honest rather than estimating.
        let base: Element<'_, Message> = if self.core.sidebar.hidden {
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
        // The settings panel sits under any confirmation, matching the
        // keyboard ladder: a quit armed while it is open is answered first.
        if let Some((card, on_cancel)) = self.active_confirmation() {
            return modal(base, card, on_cancel);
        }
        if self.settings_open {
            return modal(base, self.settings_panel(), Message::CloseSettings);
        }
        base
    }

    /// The focused terminal: its tab strip, then its grid drawn on a canvas.
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
                    let total: usize = self
                        .core
                        .sidebar
                        .projects
                        .iter()
                        .map(|g| g.sessions.len())
                        .sum();
                    iced::widget::center(
                        column![
                            text("TermHerd").size(40),
                            text(strings::welcome_counts(
                                total,
                                self.core.sidebar.projects.len()
                            ))
                            .size(14),
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
                    hover: self.core.term_hover(),
                    shift: self.shift_modifier,
                    font_size: self.core.font_size(),
                    dimmed: !self.core.window_focused(),
                    gestures: self.gestures,
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

/// What a hover card shows about a session beyond its transcript: the facts
/// only a live pane can tell. The sidebar, which has no pane, passes the
/// default.
#[derive(Debug, Default)]
pub(super) struct CardFacts {
    /// The peer name other Claude sessions address the pane's Claude by.
    pub agent: Option<String>,
    /// The Claude Code version, as [`termherd_core::App::claude_version`]
    /// resolves it.
    pub version: Option<String>,
    /// How long the pane's PTY has run.
    pub running_for: Option<Duration>,
}

/// The dimmed detail lines both hover cards show under their title, in order:
/// agent, model and effort (from the transcript `digest`), version, running
/// time. A fact nobody knows is a line left out rather than a blank one.
pub(super) fn detail_lines(facts: &CardFacts, digest: Option<&SessionDigest>) -> Vec<String> {
    let model = digest.and_then(|d| d.model.as_deref());
    let effort = digest.and_then(|d| d.effort.as_deref());
    [
        facts.agent.as_deref().map(strings::agent_name),
        strings::model_and_effort(model, effort),
        facts.version.as_deref().map(strings::claude_version),
        facts
            .running_for
            .map(|span| strings::running_for(&compact_elapsed(span))),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// One dimmed hover-card line. The title inherits the card's text colour;
/// both colours come from the theme palette (see `card_style`).
pub(super) fn secondary_line(line: String) -> Element<'static, Message> {
    text(line).size(10).style(card_secondary_text).into()
}

/// The frame both hover cards share, so the two surfaces read alike.
pub(super) fn card_frame(card: Column<'static, Message>) -> Element<'static, Message> {
    container(card)
        .padding(8)
        .max_width(360.0)
        .style(card_style)
        .into()
}

/// The hover card for a session row: full title, a muted line with relative
/// last activity and message count, the [`detail_lines`], then the last few
/// transcript lines so a duplicate-looking session is recognisable without
/// opening it.
pub(super) fn session_card(
    title: String,
    facts: &CardFacts,
    session: &SessionRecord,
    now: SystemTime,
) -> Element<'static, Message> {
    let count = session.digest.message_count;

    let age = session
        .modified
        .and_then(|m| now.duration_since(m).ok())
        .map(relative_age);
    let meta = strings::session_meta(age.as_deref(), count);

    let mut card = column![text(title).size(12), secondary_line(meta)].spacing(4);
    for line in detail_lines(facts, Some(&session.digest)) {
        card = card.push(secondary_line(line));
    }
    for line in &session.digest.tail {
        card = card.push(secondary_line(format!("› {line}")));
    }
    card_frame(card)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(model: Option<&str>, effort: Option<&str>) -> SessionDigest {
        SessionDigest {
            model: model.map(str::to_owned),
            effort: effort.map(str::to_owned),
            ..SessionDigest::default()
        }
    }

    fn every_fact() -> CardFacts {
        CardFacts {
            agent: Some("termherd-b0".to_owned()),
            version: Some("2.1.294".to_owned()),
            running_for: Some(Duration::from_secs(3600 + 12 * 60)),
        }
    }

    #[test]
    fn a_card_knowing_everything_shows_every_line_in_order() {
        let known = digest(Some("claude-opus-5-5"), Some("medium"));
        assert_eq!(
            detail_lines(&every_fact(), Some(&known)),
            vec![
                strings::agent_name("termherd-b0"),
                strings::model_and_effort(Some("claude-opus-5-5"), Some("medium")).expect("a line"),
                strings::claude_version("2.1.294"),
                strings::running_for("1h 12m"),
            ]
        );
    }

    #[test]
    fn each_unknown_fact_is_a_line_left_out() {
        let known = || digest(Some("m"), Some("e"));
        let all = detail_lines(&every_fact(), Some(&known()));
        let cases = [
            (
                CardFacts {
                    agent: None,
                    ..every_fact()
                },
                known(),
                0,
            ),
            (every_fact(), digest(None, None), 1),
            (
                CardFacts {
                    version: None,
                    ..every_fact()
                },
                known(),
                2,
            ),
            (
                CardFacts {
                    running_for: None,
                    ..every_fact()
                },
                known(),
                3,
            ),
        ];
        for (facts, digest, missing) in cases {
            let mut expected = all.clone();
            expected.remove(missing);
            assert_eq!(
                detail_lines(&facts, Some(&digest)),
                expected,
                "line {missing}"
            );
        }
    }

    #[test]
    fn a_card_with_no_transcript_still_shows_the_live_facts() {
        assert_eq!(detail_lines(&every_fact(), None).len(), 3);
        assert!(detail_lines(&CardFacts::default(), None).is_empty());
    }
}
