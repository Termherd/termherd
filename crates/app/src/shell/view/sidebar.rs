//! The session browser sidebar (FR1/FR3): the search box and filters, the
//! cross-project Favorites and Plans & mémoire sections, then the projects by
//! recency — each project folding to a header, each session a resumable row
//! with its activity dot (FR8), rename field and archive control. The shared
//! status-dot, hover-card and text helpers live in the parent [`super`]; the
//! row builders that keep [`Shell::sidebar`] under the length gate live here.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use iced::widget::{
    button, checkbox, column, container, mouse_area, row, rule, scrollable, stack, text,
    text_input, tooltip,
};
use iced::{Element, Fill};
use termherd_core::browser::{ProjectGroup, project_label, relative_age};
use termherd_core::{SessionRecord, SessionStatus, SidebarFold};

use super::{clip, mix, session_card, status_color};
use crate::shell::{Focus, Message, Shell, rename_id, search_id};
use crate::strings;

/// Fold key for the Plans & mémoire section. Project groups key their
/// fold by real (always absolute) path; this reserved, non-path key shares the
/// same persisted set without ever colliding with a project.
const PLANS_SECTION_KEY: &str = "plans-memory";

/// Fold key for the cross-project Favorites section. Like [`PLANS_SECTION_KEY`],
/// a reserved non-path key sharing the persisted fold set without colliding with
/// a real project path.
const FAVORITES_SECTION_KEY: &str = "favorites";

impl Shell {
    /// The session browser (FR1 + FR3): search box, then projects by recency.
    /// Clicking a project opens a fresh shell; clicking a session resumes it.
    /// The per-section builders keep this dispatcher short; each returns the
    /// column it contributes so the length stays a proxy for real complexity.
    pub(super) fn sidebar(&self) -> Element<'_, Message> {
        let search = text_input(strings::SEARCH_PLACEHOLDER, &self.core.search)
            .id(search_id())
            .size(12)
            .padding(6);
        // The box only accepts input while it owns the keyboard; otherwise a
        // typed key must reach the terminal, not the search. But a disabled
        // `text_input` still *captures* any click over it (iced 0.14), so a
        // plain `mouse_area` wrapper never sees the press and clicking the box
        // could not restore focus. A transparent catcher stacked on top wins
        // the press instead (a Stack dispatches topmost-first) and hands the
        // keyboard back.
        let search: Element<'_, Message> = if self.focus == Focus::Search {
            search.on_input(Message::SearchChanged).into()
        } else {
            stack![
                search,
                mouse_area(iced::widget::Space::new().width(Fill).height(Fill))
                    .on_press(Message::FocusSearch)
            ]
            .into()
        };
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

        let live = self.live_statuses();
        let visible = self.core.visible_projects();
        // One wall-clock read per render drives every relative "last activity"
        // age in the sidebar (row disambiguator + tooltip). The app layer owns
        // the clock; core stays pure.
        let now = SystemTime::now();
        // A leading status line (scan error, or "no sessions / no results")
        // stands in for the whole list, so a rule beneath it would read as its
        // own stray divider: it suppresses the first section's top rule below.
        let has_status = self.scan_error.is_some() || visible.is_empty();
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
        // The present sections, in order. The projects are one block so a single
        // rule precedes them rather than one before every group.
        let mut sections: Vec<Element<'_, Message>> = Vec::new();
        if let Some(section) = self.favorites_section(&visible, &live) {
            sections.push(section);
        }
        if let Some(section) = self.plans_section() {
            sections.push(section);
        }
        if !visible.is_empty() {
            let mut projects = column![].spacing(16);
            for group in &visible {
                projects = projects.push(self.project_group(group, &live, now));
            }
            sections.push(projects.into());
        }
        // A thin rule separates adjacent sections; the first section also earns a
        // top rule to set the list apart from the search chrome — unless a status
        // line already sits above it.
        for (i, section) in sections.into_iter().enumerate() {
            if i > 0 || !has_status {
                list = list.push(section_divider());
            }
            list = list.push(section);
        }
        // A handle to collapse the sidebar, mirroring the one that
        // restores it from the main pane.
        let hide = button(text("◀ Masquer le panneau").size(11))
            .on_press(Message::ToggleSidebar)
            .style(button::text)
            .padding(0);
        let chrome = column![hide, search, titles_only, show_archived].spacing(8);
        container(chrome.push(scrollable(list).height(Fill)).padding(8))
            .width(300)
            .style(container::rounded_box)
            .into()
    }

    /// Live activity, keyed by the Claude session id each terminal resumed, so a
    /// browsed row can show its current status (FR8). If the same session is
    /// open twice, the most urgent status wins.
    fn live_statuses(&self) -> HashMap<&str, SessionStatus> {
        let mut live: HashMap<&str, SessionStatus> = HashMap::new();
        for s in self.core.sessions.values() {
            if let Some(resume) = s.launch.resume_id() {
                live.entry(resume)
                    .and_modify(|cur| {
                        if s.status.urgency() > cur.urgency() {
                            *cur = s.status;
                        }
                    })
                    .or_insert(s.status);
            }
        }
        live
    }

    /// Cross-project Favorites (F-favorites): every starred session in one
    /// place, most-recent-first. Coexists with the in-group star pin — a
    /// favourite is a shortcut, not a move. Folds like the other sections.
    /// `None` when nothing is starred, so the section leaves no empty header.
    fn favorites_section(
        &self,
        visible: &[ProjectGroup],
        live: &HashMap<&str, SessionStatus>,
    ) -> Option<Element<'_, Message>> {
        let favorites = self.core.favorite_sessions(visible);
        if favorites.is_empty() {
            return None;
        }
        let collapsed = self.core.is_collapsed(FAVORITES_SECTION_KEY);
        let mut fav_col = column![section_header(
            FAVORITES_SECTION_KEY,
            collapsed,
            strings::FAVORITES
        )]
        .spacing(4);
        if !collapsed {
            for (path, s) in &favorites {
                let id = s.session_id.as_str();
                // ★ (filled) unstars — which also removes it from here.
                let star = button(text("★").size(12))
                    .on_press(Message::ToggleStar(s.session_id.clone()))
                    .style(button::text)
                    .padding(0);
                let mut label_row = row![].spacing(6).align_y(iced::Center);
                if let Some(status) = live.get(id) {
                    label_row = label_row.push(text("●").size(9).color(status_color(*status)));
                }
                let title = self.core.session_title(s);
                // The project label tells cross-project favourites apart.
                label_row = label_row.push(
                    text(format!("{}  ·  {}", clip(&title, 22), project_label(path))).size(11),
                );
                let open = button(label_row)
                    .on_press(Message::LaunchSession {
                        cwd: (*path).to_owned(),
                        resume: s.session_id.clone(),
                    })
                    .style(button::text)
                    .padding(0)
                    .width(Fill);
                fav_col = fav_col.push(row![star, open].spacing(6).align_y(iced::Center));
            }
        }
        Some(fav_col.into())
    }

    /// Plans & memory docs (F-plans-memory), above the project list. Its
    /// header folds shut too, keyed like a project group. `None` when there are
    /// no docs, so the section leaves no empty header.
    fn plans_section(&self) -> Option<Element<'_, Message>> {
        if self.docs.is_empty() {
            return None;
        }
        let collapsed = self.core.is_collapsed(PLANS_SECTION_KEY);
        let mut docs_col = column![section_header(
            PLANS_SECTION_KEY,
            collapsed,
            strings::PLANS_AND_MEMORY
        )]
        .spacing(4);
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
        Some(docs_col.into())
    }

    /// One project group: a foldable header (fold triangle, repo star, name and
    /// the two launch buttons) above its session rows. A folded group shows only
    /// its header; an over-long one folds its tail behind an expander.
    fn project_group(
        &self,
        group: &ProjectGroup,
        live: &HashMap<&str, SessionStatus>,
        now: SystemTime,
    ) -> Element<'_, Message> {
        let collapsed = self.core.is_collapsed(&group.path);
        // The disclosure triangle and the name both fold the session list —
        // a tree header should fold, not launch. Launching moved
        // to two explicit buttons beside it: `$` opens a plain shell, 🤖 a
        // fresh Claude session, both in the repo dir (FR4a).
        let fold = fold_toggle(&group.path, collapsed);
        // A repo star pins the whole project group to the top of the sidebar
        // (F-favorites), mirroring the per-session star below.
        let repo_starred = self.core.is_repo_starred(&group.path);
        let repo_star = button(text(if repo_starred { "★" } else { "☆" }).size(12))
            .on_press(Message::ToggleRepoStar(group.path.clone()))
            .style(button::text)
            .padding(0);
        let name = button(text(project_label(&group.path).to_owned()).size(14))
            .on_press(Message::ToggleCollapsed(group.path.clone()))
            .style(button::text)
            .padding(0)
            .width(Fill);
        let launch_shell = launch_button(
            "$",
            strings::SIDEBAR_LAUNCH_SHELL,
            Message::LaunchProject(group.path.clone()),
        );
        let launch_claude = launch_button(
            "🤖",
            strings::SIDEBAR_LAUNCH_CLAUDE,
            Message::LaunchClaude(group.path.clone()),
        );
        let header = row![fold, repo_star, name, launch_shell, launch_claude]
            .spacing(6)
            .align_y(iced::Center);
        let mut g = column![header].spacing(4);
        // A folded project shows only its header, hiding the session list.
        if collapsed {
            return g.into();
        }
        // Rows whose title repeats within this project get a relative
        // last-activity age appended, so duplicates stay distinguishable
        // The unique case keeps the clean `{title} · {count}` line.
        let collisions = self.core.colliding_titles(group);
        // Long groups fold their tail behind an expander; search
        // and the user's unfold both surface it (`sidebar_sessions`).
        let (sessions, fold) = self.core.sidebar_sessions(group);
        for s in sessions {
            g = g.push(self.session_row(s, &group.path, &collisions, live, now));
        }
        // The expander row under a truncated list: unfold the hidden
        // tail, or fold it back once expanded.
        if let Some(fold) = fold {
            let label = match fold {
                SidebarFold::Truncated(hidden) => strings::sidebar_more(hidden),
                SidebarFold::Expanded => strings::SIDEBAR_SHOW_LESS.to_owned(),
            };
            g = g.push(
                button(text(label).size(11).style(sidebar_secondary_text))
                    .on_press(Message::ToggleExpanded(group.path.clone()))
                    .style(button::text)
                    .padding(0),
            );
        }
        g.into()
    }

    /// One session row: its star pin, the resumable title (an edit field while
    /// renaming), the rename and archive controls, and — under a content search
    /// hit — the matched line in muted text beneath it.
    fn session_row(
        &self,
        s: &SessionRecord,
        group_path: &str,
        collisions: &HashSet<String>,
        live: &HashMap<&str, SessionStatus>,
        now: SystemTime,
    ) -> Element<'_, Message> {
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
            // Colliding rows carry a disambiguator so duplicate titles
            // stay distinguishable. When a custom/AI title masks a
            // different real conversation (the /clear title-carryover)
            // the divergent summary tells them apart by content;
            // otherwise we fall back to the last-activity age.
            let mut label = format!("{}  ·  {}", clip(&title, 26), s.digest.message_count);
            if collisions.contains(id) {
                if let Some(summary) = self.core.collision_subtitle(s) {
                    label.push_str("  ·  ");
                    label.push_str(&clip(&summary, 28));
                } else if let Some(age) = s.modified.and_then(|m| now.duration_since(m).ok()) {
                    label.push_str("  ·  ");
                    label.push_str(&relative_age(age));
                }
            }
            content = content.push(text(label).size(11));
            let launch = button(content)
                .on_press(Message::LaunchSession {
                    cwd: group_path.to_owned(),
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

        // Archiving is deliberate: arm the confirmation bar.
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

        let entry = row![star, middle, rename, archive]
            .spacing(6)
            .align_y(iced::Center);
        // A content hit shows its matched line in muted text beneath the
        // row, so search reveals *what* matched, not merely *that* it did.
        // Title-only hits return `None` and stay single-line. The
        // core windows the line around the hit; we clip to the sidebar.
        match self.core.search_snippet(s) {
            Some(snip) => column![
                entry,
                text(clip(&snip.line, 44))
                    .size(10)
                    .style(sidebar_secondary_text),
            ]
            .spacing(1)
            .into(),
            None => entry.into(),
        }
    }
}

/// The disclosure triangle that folds a sidebar section: ▾ when open, ▸
/// when folded, toggling the fold for `key`. Shared by the project headers and
/// the Plans & mémoire section so the two can't drift apart.
fn fold_toggle(key: &str, collapsed: bool) -> Element<'static, Message> {
    button(text(if collapsed { "▸" } else { "▾" }).size(12))
        .on_press(Message::ToggleCollapsed(key.to_owned()))
        .style(button::text)
        .padding(0)
        .into()
}

/// A thin horizontal rule between sidebar sections (Favorites, Plans & mémoire,
/// Projects), so the grouping reads at a glance. Painted in the theme's
/// `background.strong` tier — the calibrated "separator" colour iced's own
/// default rule uses: visible against the surface yet still subtle, never a
/// hardcoded grey.
fn section_divider() -> Element<'static, Message> {
    rule::horizontal(1)
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            rule::Style {
                color: palette.background.strong.color,
                radius: 0.0.into(),
                fill_mode: rule::FillMode::Full,
                snap: true,
            }
        })
        .into()
}

/// A foldable section header: the disclosure triangle and the section title,
/// both toggling the fold for `key`. Clicking the title folds the section, at
/// parity with a project group header — the tiny triangle is not the only
/// target. Shared by the Favorites and Plans & mémoire sections so the two
/// keep the same affordance.
fn section_header(key: &str, collapsed: bool, label: &str) -> Element<'static, Message> {
    row![
        fold_toggle(key, collapsed),
        button(text(label.to_owned()).size(12))
            .on_press(Message::ToggleCollapsed(key.to_owned()))
            .style(button::text)
            .padding(0),
    ]
    .spacing(6)
    .align_y(iced::Center)
    .into()
}

/// An icon button beside a project header that launches a session in the repo
/// dir (FR4a): the glyph is the affordance, the tooltip spells it out.
/// Built once so the `$` (shell) and 🤖 (Claude) buttons can't drift in style.
fn launch_button(
    icon: &'static str,
    tip: &'static str,
    on_press: Message,
) -> Element<'static, Message> {
    tooltip(
        button(text(icon).size(14))
            .on_press(on_press)
            .style(button::text)
            .padding(0),
        container(text(tip).size(12))
            .padding(4)
            .style(container::rounded_box),
        tooltip::Position::Bottom,
    )
    .into()
}

/// Dimmed secondary text for the sidebar — search-match snippets. Mixes
/// the normal text toward the background so it reads muted, theme-aware rather
/// than a hardcoded grey.
fn sidebar_secondary_text(theme: &iced::Theme) -> iced::widget::text::Style {
    let palette = theme.extended_palette();
    iced::widget::text::Style {
        color: Some(mix(
            palette.background.base.text,
            palette.background.base.color,
            0.4,
        )),
    }
}
