//! The session browser sidebar (FR1/FR3): the search box and filters, the
//! cross-project Favorites and Plans & mémoire sections, then the projects by
//! recency — each project folding to a header, each session a resumable row
//! with its activity dot (FR8), rename field and archive control. The shared
//! status-dot, hover-card and text helpers live in the parent [`super`]; the
//! row builders that keep [`Shell::sidebar`] under the length gate live here.

use std::collections::HashMap;
use std::time::SystemTime;

use iced::widget::{
    button, checkbox, column, container, mouse_area, row, rule, scrollable, stack, text,
    text_input, tooltip,
};
use iced::{Element, Fill};
use termherd_core::browser::{ProjectGroup, project_label, relative_age};
use termherd_core::keymap::Action;
use termherd_core::{Provider, SessionKind, SessionRecord, SessionStatus, SidebarFold};

use super::style::row_hover_fill;
use super::{
    Backdrop, CardFacts, clip, color_bar, kind_glyph, session_card, sidebar_secondary_text,
    status_dot,
};
use crate::shell::{Focus, Message, Shell, rename_id, search_id};
use crate::strings;

/// When a sidebar row control shows. At rest a row shows its title, and its
/// ★ when starred, which is state rather than an action; every action shows
/// only under the pointer, in the row redrawn on the hover fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown {
    AtRest,
    OnHover,
}

impl Shown {
    fn star(starred: bool) -> Self {
        if starred { Self::AtRest } else { Self::OnHover }
    }
}

/// The width of a row's glyph control. It is fixed so that a slot left empty
/// at rest lines up with the control the hover layer draws over it.
const CONTROL_WIDTH: f32 = 12.0;

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
        let placeholder = strings::with_shortcut(
            strings::SEARCH_PLACEHOLDER,
            self.shortcut_of(Action::FocusSearch).as_deref(),
        );
        let search = text_input(&placeholder, &self.core.sidebar.search)
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
        let titles_only = checkbox(self.core.sidebar.search_titles_only)
            .label(strings::TITLES_ONLY)
            .on_toggle(Message::SearchTitlesOnly)
            .text_size(11)
            .size(14);
        let show_archived = checkbox(self.core.sidebar.show_archived)
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
            let label = if self.core.sidebar.search.trim().is_empty() {
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
        let hide = chrome_tooltip(
            button(text(strings::SIDEBAR_HIDE).size(11))
                .on_press(Message::ToggleSidebar)
                .style(button::text)
                .padding(0),
            strings::with_shortcut(
                strings::SIDEBAR_HIDE_HINT,
                self.shortcut_of(Action::ToggleSidebar).as_deref(),
            ),
        );
        // Adding a repo sits with the sidebar's own chrome, not inside the
        // project list: it is how the list gets a row it could not discover.
        let add_repo = chrome_tooltip(
            button(text(strings::SIDEBAR_ADD_REPO).size(11))
                .on_press(Message::PickRepoFolder)
                .style(button::text)
                .padding(0),
            strings::SIDEBAR_ADD_REPO_HINT.to_owned(),
        );
        let settings = chrome_tooltip(
            button(text(strings::SIDEBAR_SETTINGS).size(13))
                .on_press(Message::ToggleSettings)
                .style(button::text)
                .padding(0),
            strings::with_shortcut(
                strings::SIDEBAR_SETTINGS_HINT,
                self.shortcut_of(Action::OpenSettings).as_deref(),
            ),
        );
        let mut chrome = column![].spacing(8);
        if !self.load_problems.is_empty() {
            let mut notes = column![].spacing(4);
            for problem in &self.load_problems {
                notes = notes.push(
                    text(strings::config_unusable(
                        &problem.file,
                        problem.kept_as.as_deref(),
                    ))
                    .size(11),
                );
            }
            let dismiss = button(text(strings::DISMISS).size(11))
                .on_press(Message::DismissLoadProblems)
                .style(button::text)
                .padding(0);
            chrome = chrome.push(
                container(row![notes.width(Fill), dismiss].spacing(6))
                    .padding(6)
                    .style(container::bordered_box),
            );
        }
        let chrome = chrome.extend([column![
            row![
                hide,
                iced::widget::Space::new().width(Fill),
                add_repo,
                settings
            ]
            .spacing(6)
            .align_y(iced::Center),
            search,
            titles_only,
            show_archived
        ]
        .spacing(8)
        .into()]);
        container(chrome.push(scrollable(list).height(Fill)).padding(8))
            .width(300)
            .style(container::rounded_box)
            .into()
    }

    /// Live activity, keyed by the Claude session id of each terminal, so a
    /// browsed row can show its current status (FR8). If the same session is
    /// open twice, the most urgent status wins.
    fn live_statuses(&self) -> HashMap<&str, SessionStatus> {
        let mut live: HashMap<&str, SessionStatus> = HashMap::new();
        for s in self.core.sessions.values() {
            if let Some(claude_id) = s.claude_session_id() {
                live.entry(claude_id)
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
                if let Some(dot) = live
                    .get(id)
                    .and_then(|status| status_dot(*status, Backdrop::Chrome))
                {
                    label_row = label_row.push(dot);
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
        // A repo declared by hand and not yet used has nothing to fold and
        // nothing to list: its row exists to be launched from (`F-repo-add`).
        // Both predicates live in `core`, so the snapshot reports the fold the
        // screen is drawing rather than the raw flag behind it.
        let empty = self
            .core
            .sidebar_row_empty(&group.path, group.sessions.len());
        let collapsed = self
            .core
            .sidebar_row_collapsed(&group.path, group.sessions.len());
        // The disclosure triangle and the name both fold the session list —
        // a tree header should fold, not launch. Launching moved
        // to two explicit buttons beside it: `❯` opens a plain shell, `✳` a
        // fresh Claude session, both in the repo dir (FR4a).
        let fold = || -> Element<'_, Message> {
            if empty {
                // Aligned with the triangles above and below it, so an empty
                // row does not shift its whole group left.
                iced::widget::Space::new().width(CONTROL_WIDTH).into()
            } else {
                fold_toggle(&group.path, collapsed)
            }
        };
        // A repo star pins the whole project group to the top of the sidebar
        // (F-favorites), mirroring the per-session star below.
        let repo_starred = self.core.is_repo_starred(&group.path);
        let repo_star = || {
            row_button(
                if repo_starred { "★" } else { "☆" },
                Message::ToggleRepoStar(group.path.clone()),
            )
        };
        let name = || {
            let name = button(text(project_label(&group.path).to_owned()).size(14))
                .style(button::text)
                .padding(0)
                .width(Fill);
            // An empty row has no list to fold, so the click is not merely
            // ineffective — it would store a fold nothing shows, to surface
            // the day the repo gains its first session.
            if empty {
                name
            } else {
                name.on_press(Message::ToggleCollapsed(group.path.clone()))
            }
        };
        let secondary_agents = self.providers.secondary_launchable(Provider::Claude);
        // Only a declared repo can be taken back out; a discovered project has
        // no declaration to drop, and the button must not promise one.
        let declared = self.core.is_repo_declared(&group.path);
        let header = |shown: Shown| {
            let star = match shown {
                Shown::AtRest => slot(Shown::star(repo_starred), repo_star()),
                Shown::OnHover => repo_star().into(),
            };
            let mut header = row![
                fold(),
                star,
                name(),
                launch_button(
                    kind_glyph(SessionKind::Shell),
                    strings::SIDEBAR_LAUNCH_SHELL,
                    Message::LaunchProject(group.path.clone()),
                ),
                launch_button(
                    kind_glyph(SessionKind::Agent(Provider::Claude)),
                    strings::SIDEBAR_LAUNCH_CLAUDE,
                    Message::LaunchClaude(group.path.clone()),
                )
            ]
            .spacing(6)
            .align_y(iced::Center);
            // The other agents' picker is a launch action: always in view.
            if !secondary_agents.is_empty() {
                header = header.push(launch_button(
                    "▾",
                    strings::SIDEBAR_LAUNCH_OTHER,
                    Message::ToggleLaunchPicker(group.path.clone()),
                ));
            }
            if declared {
                let forget: Element<'_, Message> = match shown {
                    Shown::AtRest => iced::widget::Space::new().width(CONTROL_WIDTH).into(),
                    Shown::OnHover => container(launch_button(
                        "✕",
                        strings::SIDEBAR_FORGET_REPO,
                        Message::ForgetRepo(group.path.clone()),
                    ))
                    .width(CONTROL_WIDTH)
                    .center_x(CONTROL_WIDTH)
                    .into(),
                };
                header = header.push(forget);
            }
            header.into()
        };
        let header = with_hover(header(Shown::AtRest), header(Shown::OnHover));
        let mut g = column![header].spacing(4);
        if self.launch_picker.as_deref() == Some(group.path.as_str())
            && !secondary_agents.is_empty()
        {
            let mut picker = row![].spacing(6).align_y(iced::Center);
            for provider in secondary_agents {
                let label = format!(
                    "{} {}",
                    kind_glyph(SessionKind::Agent(provider)),
                    provider.display_name()
                );
                let hint = strings::sidebar_launch_agent(provider.display_name());
                picker = picker.push(tooltip(
                    button(text(label).size(11))
                        .on_press(Message::LaunchAgent {
                            cwd: group.path.clone(),
                            provider,
                        })
                        .style(button::secondary)
                        .padding([2, 6]),
                    text(hint).size(11),
                    tooltip::Position::Bottom,
                ));
            }
            g = g.push(container(picker).padding([2, 0]));
        }
        if empty {
            return g
                .push(
                    text(strings::SIDEBAR_REPO_NO_SESSIONS)
                        .size(11)
                        .style(sidebar_secondary_text),
                )
                .into();
        }
        // A folded project shows only its header, hiding the session list.
        if collapsed {
            return g.into();
        }
        // Rows whose title repeats within this project get a disambiguator
        // appended, so duplicates stay distinguishable. The unique case keeps
        // the clean `{title} · {count}` line.
        let disambiguators = self.core.session_disambiguators(group);
        // Long groups fold their tail behind an expander; search
        // and the user's unfold both surface it (`sidebar_sessions`).
        let (sessions, fold) = self.core.sidebar_sessions(group);
        for s in sessions {
            g = g.push(self.session_row(s, &group.path, &disambiguators, live, now));
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
        disambiguators: &HashMap<String, Option<String>>,
        live: &HashMap<&str, SessionStatus>,
        now: SystemTime,
    ) -> Element<'_, Message> {
        let id = s.session_id.as_str();
        let starred = self.core.is_starred(id);

        // Star toggles the pin; archive hides/shows (F-session-metadata).
        let star = || {
            row_button(
                if starred { "★" } else { "☆" },
                Message::ToggleStar(s.session_id.clone()),
            )
        };

        let title = self.core.session_title(s);
        let renaming_this = self.renaming.as_ref().is_some_and(|(rid, _)| rid == id);
        let actions = || self.session_actions(s, renaming_this, title.clone());

        let entry: Element<'_, Message> = if renaming_this {
            let buffer = self.renaming.as_ref().map_or("", |(_, b)| b.as_str());
            // The hint is what a blank commit leaves: it clears only the
            // sidebar's own name. The row keeps its ✓ in view, away from the
            // pointer, while the name is typed.
            let field = text_input(&self.core.session_title_unnamed_here(s), buffer)
                .id(rename_id())
                .on_input(Message::RenameInput)
                .on_submit(Message::CommitRename)
                .size(11)
                .padding(2)
                .width(Fill);
            row![star(), field, actions()]
                .spacing(6)
                .align_y(iced::Center)
                .into()
        } else {
            let label = row_label(s, &title, disambiguators.get(id), now);
            // The clickable title that resumes the session.
            let resume = || -> Element<'_, Message> {
                let mut content = row![].spacing(6).align_y(iced::Center);
                if let Some(bar) = s.digest.agent_color.and_then(color_bar) {
                    content = content.push(bar);
                }
                // A coloured dot marks a session already open in TermHerd
                // and carries its live activity (FR8).
                if let Some(dot) = live
                    .get(id)
                    .and_then(|status| status_dot(*status, Backdrop::Chrome))
                {
                    content = content.push(dot);
                }
                let launch = button(content.push(text(label.clone()).size(11)))
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
                    session_card(
                        title.clone(),
                        &CardFacts {
                            color: s.digest.agent_color,
                            ..CardFacts::default()
                        },
                        s,
                        now,
                    ),
                    tooltip::Position::Right,
                )
                .into()
            };
            with_hover(
                row![slot(Shown::star(starred), star()), resume()]
                    .spacing(6)
                    .align_y(iced::Center)
                    .into(),
                row![star(), resume(), actions()]
                    .spacing(6)
                    .align_y(iced::Center)
                    .into(),
            )
        };
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
            None => entry,
        }
    }

    /// A session row's actions: reveal the transcript, rename (✎ starts it,
    /// ✓ commits it), and archive.
    fn session_actions(
        &self,
        s: &SessionRecord,
        renaming_this: bool,
        title: String,
    ) -> iced::widget::Row<'_, Message> {
        let rename = if renaming_this {
            row_button("✓", Message::CommitRename)
        } else {
            row_button(
                "✎",
                Message::StartRename {
                    session: s.session_id.clone(),
                    current: title,
                },
            )
        };
        // Archiving is deliberate: arm the confirmation bar.
        // Un-archiving is a harmless restore, so it stays one-click.
        let archive = if self.core.is_archived(&s.session_id) {
            row_button("⊞", Message::ToggleArchive(s.session_id.clone()))
        } else {
            row_button("⊟", Message::RequestArchive(s.session_id.clone()))
        };
        // Shows the file this row was read from: the scan's own path, which a
        // worktree session's collapsed project path could not rebuild.
        let reveal = row_button("↗", Message::RevealRowTranscript(s.transcript.clone()));
        row![reveal, rename, archive]
            .spacing(6)
            .align_y(iced::Center)
    }
}

/// A glyph button on a sidebar row: a star, or one of a session's actions.
fn row_button(glyph: &'static str, on_press: Message) -> button::Button<'static, Message> {
    button(text(glyph).size(12).width(Fill).center())
        .on_press(on_press)
        .style(button::text)
        .padding(0)
        .width(CONTROL_WIDTH)
}

/// `control` where it shows at rest, else the empty slot the hover layer
/// draws it in.
fn slot<'a>(shown: Shown, control: button::Button<'a, Message>) -> Element<'a, Message> {
    match shown {
        Shown::AtRest => control.into(),
        Shown::OnHover => iced::widget::Space::new().width(CONTROL_WIDTH).into(),
    }
}

/// `rest` with `hovered` drawn over it under the pointer, on the hovered
/// row's opaque fill: the same row with its actions. Opaque, because a
/// translucent tint over the title would lower its contrast.
fn with_hover<'a>(
    rest: Element<'a, Message>,
    hovered: Element<'a, Message>,
) -> Element<'a, Message> {
    iced::widget::hover(
        rest,
        container(hovered).width(Fill).style(|theme: &iced::Theme| {
            container::background(row_hover_fill(theme.extended_palette()))
        }),
    )
}

/// A session row's label: the clipped title and message count, plus a
/// disambiguator when the title repeats within its project — the real summary
/// when it separates the rows by content, else the last-activity age.
fn row_label(
    s: &SessionRecord,
    title: &str,
    disambiguator: Option<&Option<String>>,
    now: SystemTime,
) -> String {
    let mut label = format!("{}  ·  {}", clip(title, 26), s.digest.message_count);
    if let Some(subtitle) = disambiguator {
        let extra = match subtitle {
            Some(summary) => Some(clip(summary, 28)),
            None => s
                .modified
                .and_then(|m| now.duration_since(m).ok())
                .map(relative_age),
        };
        if let Some(extra) = extra {
            label.push_str("  ·  ");
            label.push_str(&extra);
        }
    }
    label
}

/// The disclosure triangle that folds a sidebar section: ▾ when open, ▸
/// when folded, toggling the fold for `key`. Shared by the project headers and
/// the Plans & mémoire section so the two can't drift apart.
fn fold_toggle(key: &str, collapsed: bool) -> Element<'static, Message> {
    button(text(if collapsed { "▸" } else { "▾" }).size(12))
        .on_press(Message::ToggleCollapsed(key.to_owned()))
        .style(button::text)
        .padding(0)
        .width(CONTROL_WIDTH)
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
/// Built once so the shell and Claude buttons can't drift in style.
/// A sidebar chrome control with its explanation beneath it on hover.
fn chrome_tooltip<'a>(
    control: impl Into<Element<'a, Message>>,
    tip: String,
) -> Element<'a, Message> {
    tooltip(
        control,
        container(text(tip).size(11))
            .padding(6)
            .style(container::rounded_box),
        tooltip::Position::Bottom,
    )
    .into()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn record(messages: u32, modified: Option<SystemTime>) -> SessionRecord {
        SessionRecord {
            session_id: "s1".to_owned(),
            project_path: "/p".to_owned(),
            digest: termherd_claude::digest::SessionDigest {
                message_count: messages,
                ..termherd_claude::digest::SessionDigest::default()
            },
            modified,
            transcript: std::path::PathBuf::new(),
        }
    }

    #[test]
    fn a_row_label_is_the_title_and_count_plus_what_tells_twins_apart() {
        let now = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10_000);
        let two_hours_ago = Some(now - std::time::Duration::from_secs(2 * 3600));
        let row = record(4, two_hours_ago);
        assert_eq!(
            row_label(&row, "Fix the race", None, now),
            "Fix the race  ·  4"
        );
        assert_eq!(
            row_label(
                &row,
                "Fix the race",
                Some(&Some("in the cache".to_owned())),
                now
            ),
            "Fix the race  ·  4  ·  in the cache"
        );
        assert_eq!(
            row_label(&row, "Fix the race", Some(&None), now),
            "Fix the race  ·  4  ·  2h"
        );
        assert_eq!(
            row_label(&record(4, None), "Fix the race", Some(&None), now),
            "Fix the race  ·  4",
            "a twin with no age to show adds nothing"
        );
    }

    #[test]
    fn a_starred_row_keeps_its_star_at_rest_and_an_unstarred_one_on_hover() {
        assert_eq!(Shown::star(true), Shown::AtRest);
        assert_eq!(Shown::star(false), Shown::OnHover);
    }
}
