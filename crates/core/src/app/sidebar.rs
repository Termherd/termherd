//! Sidebar read models: the visible/filtered project list, favorites,
//! per-project truncation and fold state, and the search/collision helpers.

use std::collections::HashSet;

use crate::browser::{MatchSnippet, ProjectGroup, SessionRecord, content_snippet, filter_projects};

use super::*;

/// Session-browser sidebar state: the grouped project list plus the search,
/// fold, truncation, and archive-visibility knobs that shape what it renders
/// (FR1/FR3). Grouped into one struct so the field bag on [`App`] names the
/// sidebar as a domain rather than scattering its eight fields.
#[derive(Debug, Default)]
pub struct Sidebar {
    /// Projects grouped from the latest scan (FR1).
    pub projects: Vec<ProjectGroup>,
    /// Current search query (FR3); empty means no filtering.
    pub search: String,
    /// FR3 toggle: restrict matching to titles.
    pub search_titles_only: bool,
    /// Whether archived sessions show in the browser.
    pub show_archived: bool,
    /// Whether the sidebar is collapsed to give the terminal the full width.
    /// Ephemeral — resets to visible each launch.
    pub hidden: bool,
    /// Project paths whose session list is folded shut; persisted to
    /// `~/.termherd` so the fold survives a restart.
    pub collapsed: HashSet<String>,
    /// Truncation: sessions shown per project before the tail folds behind an
    /// expander. `0` (the default) shows every session; the user's setting
    /// arrives via [`Event::SessionLimitLoaded`].
    pub session_limit: usize,
    /// Projects whose truncated session tail is unfolded. Ephemeral — unlike
    /// `collapsed`, it resets each launch and is never persisted.
    pub expanded: HashSet<String>,
}

/// What the expander row under a project's truncated session list should show
/// from [`App::sidebar_sessions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarFold {
    /// The tail is folded: this many more sessions are hidden.
    Truncated(usize),
    /// The tail is unfolded and can be folded back.
    Expanded,
}

impl App {
    /// The sidebar's view of the projects: search matches (FR3) with the
    /// metadata overlay applied (`F-session-metadata`) — archived sessions
    /// hidden unless [`Sidebar::show_archived`], starred sessions pinned to the
    /// top of their group, starred repos pinned to the top of the sidebar
    /// (`F-favorites`), and emptied groups dropped.
    #[must_use]
    pub fn visible_projects(&self) -> Vec<ProjectGroup> {
        let mut groups = filter_projects(
            &self.sidebar.projects,
            &self.sidebar.search,
            self.sidebar.search_titles_only,
        );
        for group in &mut groups {
            if !self.sidebar.show_archived {
                group.sessions.retain(|s| !self.is_archived(&s.session_id));
            }
            // Stable sort keeps recency order within each star bucket.
            group
                .sessions
                .sort_by_key(|s| !self.is_starred(&s.session_id));
        }
        groups.retain(|group| !group.sessions.is_empty());
        // Stable sort keeps activity order within each repo-star bucket.
        groups.sort_by_key(|group| !self.is_repo_starred(&group.path));
        groups
    }

    /// Starred sessions across all `groups`, most-recent-first — the source for
    /// the cross-project "★ Favorites" section (`F-favorites`). Each carries its
    /// project path so a row can resume it. Derived from `groups` (already
    /// search- and archive-filtered by [`Self::visible_projects`]) so favorites
    /// stay consistent with the list; missing mtimes sort last.
    #[must_use]
    pub fn favorite_sessions<'a>(
        &self,
        groups: &'a [ProjectGroup],
    ) -> Vec<(&'a str, &'a SessionRecord)> {
        let mut favourites: Vec<(&str, &SessionRecord)> = groups
            .iter()
            .flat_map(|group| {
                group
                    .sessions
                    .iter()
                    .map(move |session| (group.path.as_str(), session))
            })
            .filter(|(_, session)| self.is_starred(&session.session_id))
            .collect();
        favourites.sort_by_key(|(_, session)| std::cmp::Reverse(session.modified));
        favourites
    }

    /// The sessions a project row should list: all of them while a
    /// search is active (a hit in the folded tail must surface), when the
    /// limit is unset, or when the group already fits; otherwise the first
    /// `session_limit` (starred pins sort first in [`Self::visible_projects`],
    /// so they stay visible) plus the expander state for the folded tail.
    #[must_use]
    pub fn sidebar_sessions<'a>(
        &self,
        group: &'a ProjectGroup,
    ) -> (&'a [SessionRecord], Option<SidebarFold>) {
        let all = group.sessions.as_slice();
        let searching = !self.sidebar.search.trim().is_empty();
        if searching || self.sidebar.session_limit == 0 || all.len() <= self.sidebar.session_limit {
            return (all, None);
        }
        if self.sidebar.expanded.contains(&group.path) {
            return (all, Some(SidebarFold::Expanded));
        }
        let hidden = all.len() - self.sidebar.session_limit;
        (
            &all[..self.sidebar.session_limit],
            Some(SidebarFold::Truncated(hidden)),
        )
    }

    /// The located content hit for a session under the current search,
    /// or `None` when the row is shown for a title hit (or titles-only mode):
    /// nothing in the content matched, so there is nothing to point at.
    #[must_use]
    pub fn search_snippet(&self, record: &SessionRecord) -> Option<MatchSnippet> {
        if self.sidebar.search_titles_only {
            return None;
        }
        let needle = self.sidebar.search.trim().to_lowercase();
        content_snippet(&record.digest, &needle)
    }

    /// Session ids in `group` whose resolved [`Self::session_title`] is shared
    /// by another session in the same group — the rows that need a
    /// disambiguator in the sidebar. Collision is checked on the *final*
    /// title (rename/metadata included), so two rows renamed alike still count.
    /// The common, unique case returns an empty set, so callers leave it clean.
    #[must_use]
    pub fn colliding_titles(&self, group: &ProjectGroup) -> HashSet<String> {
        let titled: Vec<(&str, String)> = group
            .sessions
            .iter()
            .map(|s| (s.session_id.as_str(), self.session_title(s)))
            .collect();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for (_, title) in &titled {
            *counts.entry(title.as_str()).or_default() += 1;
        }
        titled
            .iter()
            .filter(|(_, title)| counts.get(title.as_str()).copied().unwrap_or(0) > 1)
            .map(|(id, _)| (*id).to_owned())
            .collect()
    }

    /// The content disambiguator for a row whose title collides with another
    /// in its group: the session's real first-prompt `summary` when it
    /// *diverges* from the shown title. A custom/AI title or rename can mask a
    /// completely different conversation — Claude Code carries a custom title
    /// across `/clear` into a fresh, unrelated session, so two rows read
    /// identically while their summaries differ. Surfacing the summary tells
    /// them apart by content, where the last-activity age only tells them apart
    /// by time. `None` when the title *is* the summary (no masking), so the
    /// caller falls back to the age disambiguator.
    #[must_use]
    pub fn collision_subtitle(&self, record: &SessionRecord) -> Option<String> {
        let title = self.session_title(record);
        let summary = record.digest.summary.as_str();
        (!summary.is_empty() && summary != title).then(|| summary.to_owned())
    }

    /// Whether a project's session list is folded shut in the sidebar.
    #[must_use]
    pub fn is_collapsed(&self, path: &str) -> bool {
        self.sidebar.collapsed.contains(path)
    }

    /// Flip a project's fold state and emit the persistence effect.
    pub(super) fn toggle_collapsed(&mut self, path: String) -> Vec<Effect> {
        if !self.sidebar.collapsed.remove(&path) {
            self.sidebar.collapsed.insert(path);
        }
        vec![Effect::SaveCollapsed(self.sidebar.collapsed.clone())]
    }

    /// Unfold (or refold) a project's truncated session tail. Unlike
    /// [`Self::toggle_collapsed`], the state is ephemeral — no save effect.
    pub(super) fn toggle_expanded(&mut self, path: String) -> Vec<Effect> {
        if !self.sidebar.expanded.remove(&path) {
            self.sidebar.expanded.insert(path);
        }
        Vec::new()
    }

    /// Record the configured sidebar session limit, from settings.
    pub(super) fn load_session_limit(&mut self, limit: usize) -> Vec<Effect> {
        self.sidebar.session_limit = limit;
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::*;

    #[test]
    fn scan_completed_rebuilds_projects_and_yields_no_effects() {
        let mut app = App::new();
        let effects = app.apply(Event::ScanCompleted(vec![record("abc", "/p", "hello")]));
        assert!(effects.is_empty());
        assert_eq!(app.sidebar.projects.len(), 1);
        assert_eq!(app.sidebar.projects[0].path, "/p");

        // A later scan replaces, not appends.
        let effects = app.apply(Event::ScanCompleted(vec![]));
        assert!(effects.is_empty());
        assert!(app.sidebar.projects.is_empty());
    }

    #[test]
    fn search_events_drive_visible_projects() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record(
            "abc",
            "/p",
            "fix the login bug",
        )]));
        assert_eq!(app.visible_projects().len(), 1);

        app.apply(Event::SearchChanged("login".into()));
        assert_eq!(app.visible_projects().len(), 1);

        app.apply(Event::SearchChanged("nothing-here".into()));
        assert!(app.visible_projects().is_empty());

        app.apply(Event::SearchChanged(String::new()));
        assert_eq!(app.visible_projects().len(), 1);
    }

    #[test]
    fn sidebar_truncates_to_the_limit_and_folds_the_tail() {
        let mut app = App::new();
        scanned_group(&mut app, 8);
        app.apply(Event::SessionLimitLoaded(5));
        let groups = app.visible_projects();
        let (shown, fold) = app.sidebar_sessions(&groups[0]);
        assert_eq!(shown.len(), 5);
        assert_eq!(fold, Some(SidebarFold::Truncated(3)));
        // The five kept are the freshest.
        assert!(shown.iter().all(|s| s.session_id != "s7"));
    }

    #[test]
    fn no_limit_or_a_fitting_group_shows_every_session() {
        let mut app = App::new();
        scanned_group(&mut app, 8);
        // Default (0): truncation is off.
        let groups = app.visible_projects();
        assert_eq!(
            app.sidebar_sessions(&groups[0]),
            (&groups[0].sessions[..], None)
        );
        // A limit the group fits within changes nothing either.
        app.apply(Event::SessionLimitLoaded(8));
        assert_eq!(
            app.sidebar_sessions(&groups[0]),
            (&groups[0].sessions[..], None)
        );
    }

    #[test]
    fn toggle_expanded_unfolds_the_tail_and_refolds_without_persisting() {
        let mut app = App::new();
        scanned_group(&mut app, 8);
        app.apply(Event::SessionLimitLoaded(5));
        let effects = app.apply(Event::ToggleExpanded("/p".into()));
        assert!(effects.is_empty(), "expanded state is ephemeral");
        let groups = app.visible_projects();
        let (shown, fold) = app.sidebar_sessions(&groups[0]);
        assert_eq!(shown.len(), 8);
        assert_eq!(fold, Some(SidebarFold::Expanded));
        // Toggling again folds the tail back.
        app.apply(Event::ToggleExpanded("/p".into()));
        let (shown, fold) = app.sidebar_sessions(&groups[0]);
        assert_eq!(shown.len(), 5);
        assert_eq!(fold, Some(SidebarFold::Truncated(3)));
    }

    #[test]
    fn search_surfaces_hits_from_the_folded_tail() {
        let mut app = App::new();
        let mut records: Vec<SessionRecord> = (0..7u64)
            .map(|i| {
                let mut r = record(&format!("s{i}"), "/p", "routine work");
                r.modified = Some(
                    std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + i),
                );
                r
            })
            .collect();
        // No mtime → sorts last: the needle lives in the folded tail.
        records.push(record("needle", "/p", "the rare needle"));
        app.apply(Event::ScanCompleted(records));
        app.apply(Event::SessionLimitLoaded(5));

        let groups = app.visible_projects();
        let (shown, _) = app.sidebar_sessions(&groups[0]);
        assert!(shown.iter().all(|s| s.session_id != "needle"));

        // An active query disables truncation, so the tail hit surfaces.
        app.apply(Event::SearchChanged("rare needle".into()));
        let groups = app.visible_projects();
        let (shown, fold) = app.sidebar_sessions(&groups[0]);
        assert_eq!(fold, None);
        assert!(shown.iter().any(|s| s.session_id == "needle"));
    }

    #[test]
    fn colliding_titles_flags_only_shared_titles_and_a_rename_resolves_it() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![
            record("dup1", "/p", "vm tombée"),
            record("dup2", "/p", "vm tombée"),
            record("uniq", "/p", "something else"),
        ]));
        let group = app.sidebar.projects[0].clone();

        let collisions = app.colliding_titles(&group);
        assert_eq!(
            collisions,
            HashSet::from(["dup1".to_owned(), "dup2".to_owned()])
        );

        // Renaming one of the pair to a unique title clears the collision for
        // both — the set is checked on the resolved title.
        app.apply(Event::RenameSession {
            session: "dup1".into(),
            title: "the original".into(),
        });
        assert!(app.colliding_titles(&group).is_empty());
    }

    #[test]
    fn collision_subtitle_surfaces_a_masked_summary_but_not_a_plain_one() {
        let mut app = App::new();
        // Two sessions Claude Code gave the same custom title (the /clear
        // title-carryover), masking two different real first prompts.
        let mut carried = record("clr", "/p", "regardons les soucis du ROR");
        carried.digest.custom_title = Some("login/logout petit souci".into());
        let mut original = record("orig", "/p", "ouvre un worktree auth/login");
        original.digest.custom_title = Some("login/logout petit souci".into());
        app.apply(Event::ScanCompleted(vec![
            carried.clone(),
            original.clone(),
        ]));

        // Each colliding row falls back to its real summary, so the two are
        // distinguishable by content, not just by age.
        assert_eq!(
            app.collision_subtitle(&carried).as_deref(),
            Some("regardons les soucis du ROR")
        );
        assert_eq!(
            app.collision_subtitle(&original).as_deref(),
            Some("ouvre un worktree auth/login")
        );

        // A row whose title *is* its summary (no masking) has nothing extra to
        // show — the caller keeps the age disambiguator.
        let plain = record("plain", "/p", "vm tombée");
        assert_eq!(app.collision_subtitle(&plain), None);

        // A user rename that matches the summary is likewise not a divergence.
        app.apply(Event::RenameSession {
            session: "clr".into(),
            title: "regardons les soucis du ROR".into(),
        });
        assert_eq!(app.collision_subtitle(&carried), None);
    }

    #[test]
    fn toggling_collapse_folds_then_unfolds_and_persists() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record("a", "/p", "only")]));
        assert!(!app.is_collapsed("/p"));

        // First toggle folds the project and persists the set containing it.
        let effects = app.apply(Event::ToggleCollapsed("/p".into()));
        assert!(app.is_collapsed("/p"));
        assert!(matches!(effects.as_slice(), [Effect::SaveCollapsed(c)] if c.contains("/p")));

        // A second toggle unfolds it and persists the now-empty set.
        let effects = app.apply(Event::ToggleCollapsed("/p".into()));
        assert!(!app.is_collapsed("/p"));
        assert!(matches!(effects.as_slice(), [Effect::SaveCollapsed(c)] if !c.contains("/p")));
    }

    #[test]
    fn toggle_sidebar_flips_and_starts_visible() {
        let mut app = App::new();
        assert!(!app.sidebar.hidden, "sidebar is visible on launch");
        assert!(app.apply(Event::ToggleSidebar).is_empty());
        assert!(app.sidebar.hidden);
        app.apply(Event::ToggleSidebar);
        assert!(!app.sidebar.hidden, "a second toggle restores it");
    }

    #[test]
    fn collapsed_state_loads_and_survives_a_rescan() {
        let mut app = App::new();
        app.apply(Event::CollapsedLoaded(HashSet::from(["/p".to_owned()])));
        assert!(app.is_collapsed("/p"));
        // A fold is a sidebar preference, not a property of the scan: a later
        // scan of the same project must keep it folded.
        app.apply(Event::ScanCompleted(vec![record("a", "/p", "only")]));
        assert!(app.is_collapsed("/p"));
    }
}
