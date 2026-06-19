//! The `view` half of the shell: how `Shell` state is rendered (ARCHITECTURE
//! §8). The session browser sidebar (FR1/FR3), the focused-terminal main pane
//! with its tab strip (FR5) and close-confirmation bar (#9), plus the small
//! status-dot and text helpers shared across them. No state transitions live
//! here — those are in the parent module.

use std::collections::HashMap;

use iced::widget::canvas::Canvas;
use iced::widget::{
    button, checkbox, column, container, mouse_area, row, scrollable, text, text_input,
};
use iced::{Color, Element, Fill, Font, Size};
use termherd_core::SessionStatus;

use super::ime::ime_area;
use super::terminal::{CELL_H, CELL_W, TerminalView};
use super::{Focus, Message, Shell, rename_id, search_id};

impl Shell {
    pub(super) fn view(&self) -> Element<'_, Message> {
        row![self.sidebar(), self.main_pane()].into()
    }

    /// The session browser (FR1 + FR3): search box, then projects by recency.
    /// Clicking a project opens a fresh shell; clicking a session resumes it.
    fn sidebar(&self) -> Element<'_, Message> {
        let mut search = text_input("Rechercher…", &self.core.search)
            .id(search_id())
            .size(12)
            .padding(6);
        if self.focus == Focus::Search {
            search = search.on_input(Message::SearchChanged);
        }
        // Clicking the box hands keyboard focus to it (disabling terminal keys).
        let search = mouse_area(search).on_press(Message::FocusSearch);
        let titles_only = checkbox(self.core.search_titles_only)
            .label("Titres uniquement")
            .on_toggle(Message::SearchTitlesOnly)
            .text_size(11)
            .size(14);
        let show_archived = checkbox(self.core.show_archived)
            .label("Afficher les archivées")
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
        let mut list = column![].spacing(16).padding(12);
        if let Some(error) = &self.scan_error {
            list = list.push(text(format!("Scan impossible : {error}")).size(12));
        } else if visible.is_empty() {
            let label = if self.core.search.trim().is_empty() {
                "Aucune session trouvée."
            } else {
                "Aucun résultat."
            };
            list = list.push(text(label).size(12));
        }
        // Plans & memory docs (F-plans-memory), above the project list.
        if !self.docs.is_empty() {
            let mut docs_col = column![text("Plans & mémoire").size(12)].spacing(4);
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
            list = list.push(docs_col);
        }
        for group in &visible {
            let open = button(text(project_label(&group.path).to_owned()).size(14))
                .on_press(Message::LaunchProject(group.path.clone()))
                .style(button::text)
                .padding(0);
            let mut g = column![open].spacing(4);
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
                    content = content.push(text("●").size(9).color(status_style(*status).1));
                }
                let title = self.core.session_title(s);
                let renaming_this = self.renaming.as_ref().is_some_and(|(rid, _)| rid == id);

                // The middle is an edit field while renaming this row, else the
                // clickable title that resumes the session.
                let middle: Element<'_, Message> = if renaming_this {
                    let buffer = self.renaming.as_ref().map_or("", |(_, b)| b.as_str());
                    text_input("titre…", buffer)
                        .id(rename_id())
                        .on_input(Message::RenameInput)
                        .on_submit(Message::CommitRename)
                        .size(11)
                        .padding(2)
                        .width(Fill)
                        .into()
                } else {
                    content = content.push(
                        text(format!(
                            "{}  ·  {}",
                            clip(&title, 26),
                            s.digest.message_count
                        ))
                        .size(11),
                    );
                    button(content)
                        .on_press(Message::LaunchSession {
                            cwd: group.path.clone(),
                            resume: s.session_id.clone(),
                        })
                        .style(button::text)
                        .padding(0)
                        .width(Fill)
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

                let archive = button(text(if archived { "⊞" } else { "⊟" }).size(12))
                    .on_press(Message::ToggleArchive(s.session_id.clone()))
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
        container(
            column![
                search,
                titles_only,
                show_archived,
                scrollable(list).height(Fill)
            ]
            .spacing(8)
            .padding(8),
        )
        .width(300)
        .style(container::rounded_box)
        .into()
    }

    /// The focused terminal: a status badge, then its grid drawn on a canvas.
    /// With no session open, a short summary of what the browser found.
    fn main_pane(&self) -> Element<'_, Message> {
        // A plan / memory doc, when one is open, takes over the main pane
        // read-only (F-plans-memory).
        if let Some((label, content)) = &self.viewing {
            let header = row![
                text(label).size(13),
                button(text("✕ fermer").size(12))
                    .on_press(Message::CloseDoc)
                    .style(button::text)
                    .padding(0),
            ]
            .spacing(12)
            .align_y(iced::Center);
            let body = scrollable(text(content).size(12).font(Font::MONOSPACE)).height(Fill);
            return container(column![header, body].spacing(8).padding(8))
                .width(Fill)
                .height(Fill)
                .into();
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
                        text(format!(
                            "{} session(s) dans {} projet(s)",
                            total,
                            self.core.projects.len()
                        ))
                        .size(14),
                        text("Cliquez un projet pour ouvrir un terminal,").size(13),
                        text("ou une session pour la reprendre.").size(13),
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
        let prompt = text(format!(
            "Fermer « {} » ? La session sera terminée.",
            clip(&tab.title, 24)
        ))
        .size(12);
        let confirm = button(text("Fermer").size(12))
            .on_press(Message::CloseTab(index))
            .style(button::danger)
            .padding(6);
        let cancel = button(text("Annuler").size(12))
            .on_press(Message::CancelClose)
            .style(button::text)
            .padding(6);
        Some(
            container(
                row![prompt, confirm, cancel]
                    .spacing(12)
                    .align_y(iced::Center),
            )
            .padding(6)
            .style(container::rounded_box)
            .into(),
        )
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
                label = label.push(text("●").size(9).color(status_style(status).1));
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

/// The label and dot colour for an activity status (FR8). Shared by the
/// focused-terminal badge and the sidebar's per-session dot so both stay in
/// sync.
fn status_style(status: SessionStatus) -> (&'static str, Color) {
    match status {
        SessionStatus::Starting => ("démarrage", Color::from_rgb(0.55, 0.55, 0.6)),
        SessionStatus::Busy => ("occupé", Color::from_rgb(0.95, 0.7, 0.2)),
        SessionStatus::Idle => ("prêt", Color::from_rgb(0.3, 0.8, 0.4)),
        SessionStatus::Attention => ("attention", Color::from_rgb(0.95, 0.35, 0.35)),
        SessionStatus::Exited => ("terminé", Color::from_rgb(0.5, 0.5, 0.5)),
    }
}

/// A small per-session activity badge (FR8): a coloured dot + label for the
/// focused terminal. The same dot annotates live rows in the sidebar and each
/// tab in the tab strip.
fn status_badge(status: SessionStatus) -> Element<'static, Message> {
    let (label, color) = status_style(status);
    row![text("●").size(13).color(color), text(label).size(13)]
        .spacing(6)
        .align_y(iced::Center)
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
