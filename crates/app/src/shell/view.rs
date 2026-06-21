//! The `view` half of the shell: how `Shell` state is rendered (ARCHITECTURE
//! §8). The session browser sidebar (FR1/FR3), the focused-terminal main pane
//! with its tab strip (FR5) and close-confirmation bar (#9), plus the small
//! status-dot and text helpers shared across them. No state transitions live
//! here — those are in the parent module.

use std::collections::HashMap;
use std::time::SystemTime;

use iced::widget::canvas::Canvas;
use iced::widget::{
    button, center, checkbox, column, container, mouse_area, opaque, row, scrollable, stack, text,
    text_editor, text_input, tooltip,
};
use iced::{Color, Element, Fill, Font, Size};
use termherd_core::SessionRecord;
use termherd_core::SessionStatus;
use termherd_core::browser::relative_age;

use super::ime::ime_area;
use super::terminal::{CELL_H, CELL_W, TerminalView};
use super::{DocFeedback, Focus, HANDLE_W, Message, OpenDoc, Shell, rename_id, search_id};
use crate::strings;

impl Shell {
    pub(super) fn view(&self) -> Element<'_, Message> {
        // Hiding the sidebar (#21) hands its width to the terminal; a slim
        // always-present handle brings it back without needing the shortcut.
        // The handle is pinned to `HANDLE_W` so the grid reserves exactly what
        // it occupies — keeping `grid_size` honest rather than estimating (#64).
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
        // A pending quit overlays a modal confirmation, so the about-to-die
        // sessions stay untouchable until the user decides.
        match self.quit_confirmation() {
            Some(card) => modal(base, card, Message::CancelCloseWindow),
            None => base,
        }
    }

    /// The quit-confirmation modal card (shown when a window close is armed and
    /// live sessions would be hard-killed). `None` when no quit is pending.
    fn quit_confirmation(&self) -> Option<Element<'_, Message>> {
        if !self.quit_pending() {
            return None;
        }
        let live = self.live_session_count();
        Some(Self::confirmation_bar(
            strings::quit_prompt(live),
            strings::QUIT,
            button::danger,
            Message::ConfirmCloseWindow,
            Message::CancelCloseWindow,
        ))
    }

    /// The session browser (FR1 + FR3): search box, then projects by recency.
    /// Clicking a project opens a fresh shell; clicking a session resumes it.
    fn sidebar(&self) -> Element<'_, Message> {
        let mut search = text_input(strings::SEARCH_PLACEHOLDER, &self.core.search)
            .id(search_id())
            .size(12)
            .padding(6);
        if self.focus == Focus::Search {
            search = search.on_input(Message::SearchChanged);
        }
        // Clicking the box hands keyboard focus to it (disabling terminal keys).
        let search = mouse_area(search).on_press(Message::FocusSearch);
        let titles_only = checkbox(self.core.search_titles_only)
            .label(strings::TITLES_ONLY)
            .on_toggle(Message::SearchTitlesOnly)
            .text_size(11)
            .size(14);
        let show_archived = checkbox(self.core.show_archived)
            .label(strings::SHOW_ARCHIVED)
            .on_toggle(Message::ShowArchived)
            .text_size(11)
            .size(14);

        // Live activity, keyed by the Claude session id each terminal resumed,
        // so a browsed row can show its current status (FR8). If the same
        // session is open twice, the most urgent status wins.
        let mut live: HashMap<&str, SessionStatus> = HashMap::new();
        for s in self.core.sessions.values() {
            if let Some(resume) = s.resume.as_deref() {
                live.entry(resume)
                    .and_modify(|cur| {
                        if s.status.urgency() > cur.urgency() {
                            *cur = s.status;
                        }
                    })
                    .or_insert(s.status);
            }
        }

        let visible = self.core.visible_projects();
        // One wall-clock read per render drives every relative "last activity"
        // age in the sidebar (row disambiguator + tooltip). The app layer owns
        // the clock; core stays pure.
        let now = std::time::SystemTime::now();
        let mut list = column![].spacing(16).padding(12);
        if let Some(error) = &self.scan_error {
            list = list.push(text(strings::scan_failed(error)).size(12));
        } else if visible.is_empty() {
            let label = if self.core.search.trim().is_empty() {
                strings::NO_SESSIONS
            } else {
                strings::NO_RESULTS
            };
            list = list.push(text(label).size(12));
        }
        // Plans & memory docs (F-plans-memory), above the project list. Its
        // header folds shut too (#22), keyed like a project group.
        if !self.docs.is_empty() {
            let collapsed = self.core.is_collapsed(PLANS_SECTION_KEY);
            let header = row![
                fold_toggle(PLANS_SECTION_KEY, collapsed),
                text(strings::PLANS_AND_MEMORY).size(12)
            ]
            .spacing(6)
            .align_y(iced::Center);
            let mut docs_col = column![header].spacing(4);
            if !collapsed {
                for doc in &self.docs {
                    docs_col = docs_col.push(
                        button(text(clip(&doc.label, 34)).size(11))
                            .on_press(Message::OpenDoc {
                                label: doc.label.clone(),
                                path: doc.path.clone(),
                            })
                            .style(button::text)
                            .padding(0),
                    );
                }
            }
            list = list.push(docs_col);
        }
        for group in &visible {
            let collapsed = self.core.is_collapsed(&group.path);
            // A disclosure triangle folds the session list (#22); the project
            // name keeps its launch-a-terminal click beside it.
            let fold = fold_toggle(&group.path, collapsed);
            let open = button(text(project_label(&group.path).to_owned()).size(14))
                .on_press(Message::LaunchProject(group.path.clone()))
                .style(button::text)
                .padding(0);
            let header = row![fold, open].spacing(6).align_y(iced::Center);
            let mut g = column![header].spacing(4);
            // A folded project shows only its header, hiding the session list.
            if collapsed {
                list = list.push(g);
                continue;
            }
            // Rows whose title repeats within this project get a relative
            // last-activity age appended, so duplicates stay distinguishable
            // (#42). The unique case keeps the clean `{title} · {count}` line.
            let collisions = self.core.colliding_titles(group);
            for s in &group.sessions {
                let id = s.session_id.as_str();
                let starred = self.core.is_starred(id);
                let archived = self.core.is_archived(id);

                // Star toggles the pin; archive hides/shows (F-session-metadata).
                let star = button(text(if starred { "★" } else { "☆" }).size(12))
                    .on_press(Message::ToggleStar(s.session_id.clone()))
                    .style(button::text)
                    .padding(0);

                let mut content = row![].spacing(6).align_y(iced::Center);
                // A coloured dot marks a session already open in TermHerd and
                // carries its live activity (FR8).
                if let Some(status) = live.get(id) {
                    content = content.push(text("●").size(9).color(status_color(*status)));
                }
                let title = self.core.session_title(s);
                let renaming_this = self.renaming.as_ref().is_some_and(|(rid, _)| rid == id);

                // The middle is an edit field while renaming this row, else the
                // clickable title that resumes the session.
                let middle: Element<'_, Message> = if renaming_this {
                    let buffer = self.renaming.as_ref().map_or("", |(_, b)| b.as_str());
                    text_input(strings::RENAME_PLACEHOLDER, buffer)
                        .id(rename_id())
                        .on_input(Message::RenameInput)
                        .on_submit(Message::CommitRename)
                        .size(11)
                        .padding(2)
                        .width(Fill)
                        .into()
                } else {
                    // Colliding rows carry a relative last-activity age so the
                    // duplicate titles stay distinguishable (#42).
                    let mut label = format!("{}  ·  {}", clip(&title, 26), s.digest.message_count);
                    if collisions.contains(id)
                        && let Some(age) = s.modified.and_then(|m| now.duration_since(m).ok())
                    {
                        label.push_str("  ·  ");
                        label.push_str(&relative_age(age));
                    }
                    content = content.push(text(label).size(11));
                    let launch = button(content)
                        .on_press(Message::LaunchSession {
                            cwd: group.path.clone(),
                            resume: s.session_id.clone(),
                        })
                        .style(button::text)
                        .padding(0)
                        .width(Fill);
                    // The narrow row clips the title; hover reveals a richer
                    // card — full title, last activity + message count, and the
                    // last few transcript lines so the session is recognisable
                    // without opening it.
                    tooltip(
                        launch,
                        session_card(title.clone(), s, now),
                        tooltip::Position::Right,
                    )
                    .into()
                };

                // ✎ starts the rename; ✓ commits it.
                let rename = if renaming_this {
                    button(text("✓").size(12))
                        .on_press(Message::CommitRename)
                        .style(button::text)
                        .padding(0)
                } else {
                    button(text("✎").size(12))
                        .on_press(Message::StartRename {
                            session: s.session_id.clone(),
                            current: title.clone(),
                        })
                        .style(button::text)
                        .padding(0)
                };

                // Archiving is deliberate (#20): arm the confirmation bar.
                // Un-archiving is a harmless restore, so it stays one-click.
                let archive_msg = if archived {
                    Message::ToggleArchive(s.session_id.clone())
                } else {
                    Message::RequestArchive(s.session_id.clone())
                };
                let archive = button(text(if archived { "⊞" } else { "⊟" }).size(12))
                    .on_press(archive_msg)
                    .style(button::text)
                    .padding(0);

                g = g.push(
                    row![star, middle, rename, archive]
                        .spacing(6)
                        .align_y(iced::Center),
                );
            }
            list = list.push(g);
        }
        // A handle to collapse the sidebar (#21), mirroring the one that
        // restores it from the main pane.
        let hide = button(text("◀ Masquer le panneau").size(11))
            .on_press(Message::ToggleSidebar)
            .style(button::text)
            .padding(0);
        let mut chrome = column![hide, search, titles_only, show_archived].spacing(8);
        if let Some(confirm) = self.archive_confirmation() {
            chrome = chrome.push(confirm);
        }
        container(chrome.push(scrollable(list).height(Fill)).padding(8))
            .width(300)
            .style(container::rounded_box)
            .into()
    }

    /// The archive-confirmation bar (#20), shown in the sidebar when an archive
    /// is armed: it names the session about to be hidden and offers Archiver
    /// (confirm) / Annuler. `None` when nothing is pending.
    fn archive_confirmation(&self) -> Option<Element<'_, Message>> {
        let session = self.archiving.as_deref()?;
        let title = self
            .core
            .projects
            .iter()
            .flat_map(|group| &group.sessions)
            .find(|s| s.session_id == session)
            .map_or_else(|| session.to_owned(), |s| self.core.session_title(s));
        Some(Self::confirmation_bar(
            strings::archive_prompt(&clip(&title, 24)),
            strings::ARCHIVE,
            button::primary,
            Message::ConfirmArchive,
            Message::CancelArchive,
        ))
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
                })
                .width(Fill)
                .height(Fill);
                // Wrap the grid so the platform IME is on while the terminal is
                // focused — without it dead/accent keys never compose (#34). Off
                // while an overlay (inline rename / close confirmation) is up, so
                // its own field owns composition and a dead key can't leak to the
                // PTY; focus stays `Terminal` underneath those, so they must be
                // excluded explicitly — the same guard `on_key` applies.
                let composed = ime_area(
                    canvas,
                    self.accepts_terminal_input(),
                    screen.cursor,
                    Size::new(CELL_W, CELL_H),
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
        if let Some(confirm) = self.close_confirmation() {
            pane = pane.push(confirm);
        }
        if let Some(status) = focused.and_then(|id| self.core.sessions.get(&id)) {
            pane = pane.push(status_badge(status.status));
        }
        container(pane.push(body)).width(Fill).height(Fill).into()
    }

    /// The close-confirmation bar (#9), shown when a close is armed: it names
    /// the session about to die and offers Fermer (confirm) / Annuler. `None`
    /// when nothing is pending or the armed index has since gone away.
    fn close_confirmation(&self) -> Option<Element<'_, Message>> {
        let index = self.closing?;
        let tab = self.core.workspace.tabs.get(index)?;
        Some(Self::confirmation_bar(
            strings::close_tab_prompt(&clip(&tab.title, 24)),
            strings::CLOSE,
            button::danger,
            Message::CloseTab(index),
            Message::CancelClose,
        ))
    }

    /// A confirmation bar shared by the close (#9) and archive (#20) prompts:
    /// the question, a styled confirm button, and an Annuler cancel, in the
    /// rounded container both use. Keeping one builder stops the two bars
    /// drifting apart.
    fn confirmation_bar<'a>(
        prompt: String,
        confirm_label: &'a str,
        confirm_style: impl Fn(&iced::Theme, button::Status) -> button::Style + 'a,
        on_confirm: Message,
        on_cancel: Message,
    ) -> Element<'a, Message> {
        let prompt = text(prompt).size(12);
        let confirm = button(text(confirm_label).size(12))
            .on_press(on_confirm)
            .style(confirm_style)
            .padding(6);
        let cancel = button(text(strings::CANCEL).size(12))
            .on_press(on_cancel)
            .style(button::text)
            .padding(6);
        container(
            row![prompt, confirm, cancel]
                .spacing(12)
                .align_y(iced::Center),
        )
        .padding(6)
        .style(container::rounded_box)
        .into()
    }

    /// The tab strip (FR5): one chip per open session, the active one
    /// highlighted, each carrying its activity dot (FR8) and a close button.
    /// `None` when nothing is open, so the welcome view keeps the full pane.
    fn tab_bar(&self) -> Option<Element<'_, Message>> {
        let tabs = &self.core.workspace.tabs;
        if tabs.is_empty() {
            return None;
        }
        let mut bar = row![].spacing(4).align_y(iced::Center);
        for (index, tab) in tabs.iter().enumerate() {
            let active = index == self.core.workspace.active;
            let mut label = row![].spacing(6).align_y(iced::Center);
            if let Some(status) = self.core.tab_status(index) {
                label = label.push(text("●").size(9).color(status_color(status)));
            }
            label = label.push(text(clip(&tab.title, 24)).size(12));
            let title = button(label)
                .on_press(Message::ActivateTab(index))
                .padding(6);
            let title = if active {
                title.style(button::primary)
            } else {
                title.style(button::text)
            };
            let close = button(text("×").size(14))
                .on_press(Message::RequestCloseTab(index))
                .style(button::text)
                .padding(4);
            bar = bar.push(row![title, close].align_y(iced::Center));
        }
        Some(bar.into())
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
fn status_color(status: SessionStatus) -> Color {
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

/// Fold key for the Plans & mémoire section (#22). Project groups key their
/// fold by real (always absolute) path; this reserved, non-path key shares the
/// same persisted set without ever colliding with a project.
const PLANS_SECTION_KEY: &str = "plans-memory";

/// The disclosure triangle that folds a sidebar section (#22): ▾ when open, ▸
/// when folded, toggling the fold for `key`. Shared by the project headers and
/// the Plans & mémoire section so the two can't drift apart.
fn fold_toggle(key: &str, collapsed: bool) -> Element<'static, Message> {
    button(text(if collapsed { "▸" } else { "▾" }).size(12))
        .on_press(Message::ToggleCollapsed(key.to_owned()))
        .style(button::text)
        .padding(0)
        .into()
}

/// Overlay `content` as a centred modal over `base`, dimming everything behind
/// it; a click on the scrim emits `on_blur` to dismiss. The base UI keeps
/// rendering underneath but cannot be interacted with — the inner `opaque`
/// swallows clicks on the card, the outer one blocks the layers below.
fn modal<'a>(
    base: Element<'a, Message>,
    content: Element<'a, Message>,
    on_blur: Message,
) -> Element<'a, Message> {
    stack(vec![
        base,
        opaque(mouse_area(center(opaque(content)).style(modal_backdrop)).on_press(on_blur)),
    ])
    .into()
}

/// Semi-transparent scrim drawn behind a modal so the dialog reads as the only
/// actionable surface.
fn modal_backdrop(_theme: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(
            Color {
                a: 0.6,
                ..Color::BLACK
            }
            .into(),
        ),
        ..container::Style::default()
    }
}

/// Background for the session hover card — a step away from the surrounding
/// surface (the `strong` palette tier rather than the default `weak`) so the
/// card reads as a distinct floating layer, with a thin border to seal it.
/// Everything is pulled from the theme palette, so it tracks the theme system
/// once that lands rather than baking in a colour.
fn card_style(theme: &iced::Theme) -> container::Style {
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

/// Dimmed text for the card's secondary lines (meta + transcript tail): the
/// card's text colour mixed toward its background, so it stays legible and
/// theme-derived on both light and dark palettes.
fn card_secondary_text(theme: &iced::Theme) -> iced::widget::text::Style {
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
fn mix(a: Color, b: Color, t: f32) -> Color {
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
fn session_card(
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

/// Last path component — what the sidebar shows as the project name.
pub(super) fn project_label(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

/// Collapse newlines to spaces and truncate to `max` characters with an ellipsis.
fn clip(s: &str, max: usize) -> String {
    let cleaned: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if cleaned.chars().count() <= max {
        cleaned
    } else {
        let mut out: String = cleaned.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
