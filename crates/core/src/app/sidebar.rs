//! Sidebar read models: the visible/filtered project list, favorites,
//! per-project truncation and fold state, and the search/collision helpers.

use std::collections::HashSet;

use crate::browser::{MatchSnippet, ProjectGroup, SessionRecord, content_snippet, filter_rows};

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
    /// Every project the sidebar could show: the scan's groups, plus an empty
    /// group for each hand-declared repo the scan does not report
    /// (`F-repo-add`). The union is keyed on the path, so a declared repo that
    /// gains its first session stops being empty rather than doubling — the
    /// duplicate-sidebar class FR1 pins.
    ///
    /// Declared paths are sorted before they are appended: they come from a
    /// `HashMap`, whose iteration order would otherwise decide the tie-breaks
    /// of the sort below and make the sidebar reshuffle between two identical
    /// states.
    /// Borrowed form, so the snapshot can walk the same union without cloning
    /// the digests only the rendering path needs.
    pub(super) fn merged_rows(&self) -> Vec<(&str, &[SessionRecord])> {
        let scanned: HashSet<&str> = self
            .sidebar
            .projects
            .iter()
            .map(|group| group.path.as_str())
            .collect();
        let mut declared: Vec<&str> = self
            .repos
            .iter()
            .filter(|(path, meta)| meta.declared && !scanned.contains(path.as_str()))
            .map(|(path, _)| path.as_str())
            .collect();
        declared.sort_unstable();

        let mut rows: Vec<(&str, &[SessionRecord])> = self
            .sidebar
            .projects
            .iter()
            .map(|group| (group.path.as_str(), group.sessions.as_slice()))
            .collect();
        rows.extend(declared.into_iter().map(|path| (path, &[][..])));
        rows
    }

    /// Every repo path the sidebar knows, scanned or declared, before the
    /// search box or the archive toggle narrows anything: membership, not
    /// visibility. Whoever asks "which repos are there" asks here, so the
    /// answer cannot differ from the sidebar's.
    #[must_use]
    pub fn repo_paths(&self) -> Vec<&str> {
        self.merged_rows()
            .into_iter()
            .map(|(path, _)| path)
            .collect()
    }

    /// Whether a sidebar row survives once its sessions have been filtered:
    /// something to show, or a declaration that justifies an empty row
    /// (`F-repo-add`).
    pub(super) fn sidebar_row_shown(&self, path: &str, shown_sessions: usize) -> bool {
        shown_sessions > 0 || self.is_repo_declared(path)
    }

    /// The sidebar's row order as **one** key: a starred repo (`F-favorites`)
    /// outranks a declaration still waiting for its first session, which
    /// outranks recency. Both the rendered list and the snapshot ask for it
    /// here — the two used to hold hand-written copies of this rule, with a
    /// doc-comment asking the next reader to keep them in step.
    ///
    /// `sessions` is the row's **unfiltered** session list, deliberately. Keyed
    /// on the filtered one, typing in the search box would reorder the projects
    /// under the cursor — a search narrows a list, it does not rearrange it —
    /// and a repo whose sessions are merely all archived would be mistaken for
    /// one that never had any, and pinned to the top as if freshly added.
    pub(super) fn sidebar_row_order(
        &self,
        path: &str,
        sessions: &[SessionRecord],
    ) -> (bool, bool, std::cmp::Reverse<Option<std::time::SystemTime>>) {
        (
            !self.is_repo_starred(path),
            !(sessions.is_empty() && self.is_repo_declared(path)),
            std::cmp::Reverse(crate::browser::last_activity(sessions)),
        )
    }

    /// Whether a row has nothing to fold: no session shown *and* none in the
    /// scan at all, which is a declaration waiting for its first session
    /// (`F-repo-add`). A repo whose every session is merely archived has a
    /// history, and calling it empty would be a lie the user can act on.
    #[must_use]
    pub fn sidebar_row_empty(&self, path: &str, shown_sessions: usize) -> bool {
        shown_sessions == 0 && !self.repo_has_sessions(path)
    }

    /// A row's fold state **as it is presented**: an empty row has no session
    /// list, so it cannot be folded whatever the stored flag says.
    ///
    /// Asked here by the renderer and the snapshot alike. Masked in the view
    /// alone, the snapshot reported a fold the screen contradicted, and the
    /// flag went on being toggled invisibly — to surface the day the repo
    /// gained its first session.
    #[must_use]
    pub fn sidebar_row_collapsed(&self, path: &str, shown_sessions: usize) -> bool {
        self.is_collapsed(path) && !self.sidebar_row_empty(path, shown_sessions)
    }

    /// The row for `path` as **membership** sees it — the scan united with the
    /// declarations — carrying its unfiltered session count. `None` when no row
    /// exists at all.
    ///
    /// Deliberately blind to the search box and the archive knob: this answers
    /// "is this repository in the sidebar", which is what an MCP caller asks
    /// after adding one. Read off the filtered list, a search the user happened
    /// to leave in the box reported a successful `add_repo` as a failure.
    #[must_use]
    pub fn sidebar_row(&self, path: &str) -> Option<usize> {
        let sessions = self
            .merged_rows()
            .into_iter()
            .find(|(row, _)| *row == path)
            .map(|(_, sessions)| sessions.len())?;
        self.sidebar_row_shown(path, sessions).then_some(sessions)
    }

    /// Whether the scan reports any session at all for `path`, archived or not
    /// — what tells "added, never used" apart from "everything is filtered out"
    /// for a row the filters emptied.
    #[must_use]
    pub fn repo_has_sessions(&self, path: &str) -> bool {
        self.sidebar
            .projects
            .iter()
            .any(|group| group.path == path && !group.sessions.is_empty())
    }

    /// The sidebar's view of the projects: the scan's groups united with the
    /// declared repos (`F-repo-add`), narrowed to the search matches (FR3),
    /// with the metadata overlay applied (`F-session-metadata`) — archived
    /// sessions hidden unless [`Sidebar::show_archived`], starred sessions
    /// pinned to the top of their group, and groups left with nothing to show
    /// dropped unless a declaration justifies them.
    ///
    /// The order is **one key**, not three sorts: a starred repo
    /// (`F-favorites`) outranks a declaration still waiting for its first
    /// session, which outranks recency. Written as separate sorts, the second
    /// would silently undo the first.
    #[must_use]
    pub fn visible_projects(&self) -> Vec<ProjectGroup> {
        let rows = self.merged_rows();
        let mut groups = filter_rows(&rows, &self.sidebar.search, self.sidebar.search_titles_only);
        for group in &mut groups {
            if !self.sidebar.show_archived {
                group.sessions.retain(|s| !self.is_archived(&s.session_id));
            }
            // Stable sort keeps recency order within each star bucket.
            group
                .sessions
                .sort_by_key(|s| !self.is_starred(&s.session_id));
        }
        groups.retain(|group| self.sidebar_row_shown(&group.path, group.sessions.len()));
        // The order is keyed on the *unfiltered* rows, so a search narrows the
        // list without rearranging it. Stable, so equal keys keep the path order
        // `group_projects` gave them.
        let unfiltered: HashMap<&str, &[SessionRecord]> = rows.iter().copied().collect();
        // Cached: the key costs a map lookup plus a scan of the row's sessions,
        // and this runs on every frame — `sort_by_key` would pay it per
        // comparison rather than per row.
        groups.sort_by_cached_key(|group| {
            let sessions = unfiltered
                .get(group.path.as_str())
                .copied()
                .unwrap_or_default();
            self.sidebar_row_order(&group.path, sessions)
        });
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

    /// The sidebar disambiguators for `group`, keyed by session id. An entry
    /// exists for every session whose resolved [`Self::session_title`] is
    /// shared by another session in the same group — the rows that need help
    /// telling them apart. Collision is checked on the *final* title
    /// (rename/metadata included), so two rows renamed alike still count. The
    /// common, unique case returns an empty map, so callers leave the rows
    /// clean.
    ///
    /// The value is the session's real first-prompt `summary`, but only when
    /// that summary actually *separates* this row from the ones it collides
    /// with: a custom/AI title or rename can mask a completely different
    /// conversation — Claude Code carries a custom title across `/clear` into a
    /// fresh, unrelated session — and there the summary tells them apart by
    /// content, where the last-activity age only tells them apart by time.
    /// `None` means the summary would separate nothing: it is absent, it *is*
    /// the title, or the colliding rows share it too (the same question asked
    /// twice, which the CLI titles alike). The caller then falls back to the
    /// age, which always differs.
    #[must_use]
    pub fn session_disambiguators(&self, group: &ProjectGroup) -> HashMap<String, Option<String>> {
        let rows: Vec<(&str, String, &str)> = group
            .sessions
            .iter()
            .map(|s| {
                (
                    s.session_id.as_str(),
                    self.session_title(s),
                    s.digest.summary.as_str(),
                )
            })
            .collect();
        let mut titles: HashMap<&str, usize> = HashMap::new();
        let mut pairs: HashMap<(&str, &str), usize> = HashMap::new();
        for (_, title, summary) in &rows {
            *titles.entry(title.as_str()).or_default() += 1;
            *pairs.entry((title.as_str(), summary)).or_default() += 1;
        }
        rows.iter()
            .filter(|(_, title, _)| titles.get(title.as_str()).copied().unwrap_or(0) > 1)
            .map(|(id, title, summary)| {
                let separates = !summary.is_empty()
                    && summary != title
                    && pairs.get(&(title.as_str(), *summary)).copied().unwrap_or(0) == 1;
                ((*id).to_owned(), separates.then(|| (*summary).to_owned()))
            })
            .collect()
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
    fn disambiguators_flag_only_shared_titles_and_a_rename_resolves_it() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![
            record("dup1", "/p", "vm tombée"),
            record("dup2", "/p", "vm tombée"),
            record("uniq", "/p", "something else"),
        ]));
        let group = app.sidebar.projects[0].clone();

        let rows = app.session_disambiguators(&group);
        assert_eq!(
            rows.keys().cloned().collect::<HashSet<_>>(),
            HashSet::from(["dup1".to_owned(), "dup2".to_owned()])
        );
        // Both rows show their title *as* their summary, so nothing separates
        // them by content — the caller keeps the age disambiguator.
        assert_eq!(rows.get("dup1"), Some(&None));
        assert_eq!(rows.get("dup2"), Some(&None));

        // Renaming one of the pair to a unique title clears the collision for
        // both — the map is keyed on the resolved title.
        app.apply(Event::RenameSession {
            session: "dup1".into(),
            title: "the original".into(),
        });
        assert!(app.session_disambiguators(&group).is_empty());
    }

    #[test]
    fn a_masked_summary_disambiguates_but_a_shared_one_does_not() {
        let mut app = App::new();
        // Two sessions Claude Code gave the same custom title (the /clear
        // title-carryover), masking two different real first prompts.
        let mut carried = record("clr", "/p", "regardons les soucis du ROR");
        carried.digest.custom_title = Some("login/logout petit souci".into());
        let mut original = record("orig", "/p", "ouvre un worktree auth/login");
        original.digest.custom_title = Some("login/logout petit souci".into());
        app.apply(Event::ScanCompleted(vec![carried, original]));
        let group = app.sidebar.projects[0].clone();

        // Each colliding row surfaces its real summary, so the two are
        // distinguishable by content, not just by age.
        let rows = app.session_disambiguators(&group);
        assert_eq!(
            rows.get("clr"),
            Some(&Some("regardons les soucis du ROR".to_owned()))
        );
        assert_eq!(
            rows.get("orig"),
            Some(&Some("ouvre un worktree auth/login".to_owned()))
        );

        // Renaming one to its own summary makes its title unique: the pair no
        // longer collides at all.
        app.apply(Event::RenameSession {
            session: "clr".into(),
            title: "regardons les soucis du ROR".into(),
        });
        assert!(app.session_disambiguators(&group).is_empty());
    }

    #[test]
    fn the_same_question_asked_twice_falls_back_to_the_age() {
        let mut app = App::new();
        // The same prompt in one project twice: the CLI derives the same
        // ai-title for both, and the summaries match too. Showing the summary
        // would repeat identical text on both rows and separate nothing.
        let mut first = record("a", "/p", "c'est quoi les vignettes ?");
        first.digest.ai_title = Some("Comprendre les vignettes".into());
        let mut second = record("b", "/p", "c'est quoi les vignettes ?");
        second.digest.ai_title = Some("Comprendre les vignettes".into());
        // A third row shares the summary but not the title — it must not make
        // the other two look ambiguous.
        let mut renamed = record("c", "/p", "c'est quoi les vignettes ?");
        renamed.digest.ai_title = Some("Vignettes, la suite".into());
        app.apply(Event::ScanCompleted(vec![first, second, renamed]));
        let group = app.sidebar.projects[0].clone();

        let rows = app.session_disambiguators(&group);
        assert_eq!(
            rows.keys().cloned().collect::<HashSet<_>>(),
            HashSet::from(["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(rows.get("a"), Some(&None));
        assert_eq!(rows.get("b"), Some(&None));
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

    /// The paths the sidebar shows, in order.
    fn shown(app: &App) -> Vec<String> {
        app.visible_projects()
            .into_iter()
            .map(|group| group.path)
            .collect()
    }

    /// A record in `path` with an explicit activity time, so ordering tests
    /// state their own recency instead of relying on `record`'s absent mtime.
    fn active(id: &str, path: &str, at: u64) -> SessionRecord {
        let mut r = record(id, path, "routine work");
        r.modified =
            Some(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + at));
        r
    }

    #[test]
    fn a_declared_repo_appears_as_an_empty_group() {
        let mut app = App::new();
        // Nothing has ever been scanned for this path — the whole point.
        app.apply(Event::DeclareRepo("/fresh".into()));

        let groups = app.visible_projects();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].path, "/fresh");
        assert!(
            groups[0].sessions.is_empty(),
            "a declared repo starts with no sessions; the row is a launch point"
        );
    }

    #[test]
    fn a_declared_repo_that_gains_a_session_stays_one_group() {
        let mut app = App::new();
        app.apply(Event::DeclareRepo("/fresh".into()));
        // The user launches Claude there; the next scan reports it.
        app.apply(Event::ScanCompleted(vec![record("s1", "/fresh", "hello")]));

        // The duplicate-sidebar class FR1 pins: union on the path key, so the
        // declaration and the discovery are the same row.
        assert_eq!(shown(&app), vec!["/fresh"]);
        assert_eq!(app.visible_projects()[0].sessions.len(), 1);
        assert!(
            app.is_repo_declared("/fresh"),
            "a discovery does not undo a declaration"
        );
    }

    #[test]
    fn declaring_a_repo_the_scan_already_reports_is_idempotent() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record("s1", "/known", "hello")]));
        let before = app.visible_projects();

        app.apply(Event::DeclareRepo("/known".into()));
        assert_eq!(app.visible_projects(), before, "nothing visible changes");
        assert!(app.is_repo_declared("/known"), "but the flag is now set");
    }

    #[test]
    fn forgetting_a_declared_repo_removes_its_empty_group() {
        let mut app = App::new();
        app.apply(Event::DeclareRepo("/fresh".into()));
        assert_eq!(shown(&app), vec!["/fresh"]);

        app.apply(Event::ForgetRepo("/fresh".into()));
        assert!(
            shown(&app).is_empty(),
            "nothing else justified the row, so it goes"
        );
    }

    #[test]
    fn forgetting_a_repo_that_has_sessions_keeps_its_group() {
        let mut app = App::new();
        app.apply(Event::DeclareRepo("/fresh".into()));
        app.apply(Event::ScanCompleted(vec![record("s1", "/fresh", "hello")]));

        app.apply(Event::ForgetRepo("/fresh".into()));
        assert_eq!(
            shown(&app),
            vec!["/fresh"],
            "the scan still reports it; forgetting drops the declaration, not the project"
        );
        assert!(!app.is_repo_declared("/fresh"));
    }

    #[test]
    fn the_sidebar_order_is_one_key_star_then_declared_then_activity() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![
            active("s1", "/busy", 100),
            active("s2", "/quiet", 1),
            active("s3", "/starred", 50),
        ]));
        app.apply(Event::ToggleRepoStar("/starred".into()));
        app.apply(Event::DeclareRepo("/fresh".into()));

        // A starred repo outranks a fresh declaration, which outranks activity.
        // Asserted as one order, because it is one sort key: three separate
        // assertions would pass for three sorts that fight each other.
        assert_eq!(shown(&app), vec!["/starred", "/fresh", "/busy", "/quiet"]);
    }

    #[test]
    fn the_declared_pin_disarms_once_the_repo_has_a_session() {
        let mut app = App::new();
        app.apply(Event::DeclareRepo("/fresh".into()));
        app.apply(Event::ScanCompleted(vec![
            active("s1", "/busy", 100),
            // The declared repo's first session is older than /busy's.
            active("s2", "/fresh", 1),
        ]));

        assert_eq!(
            shown(&app),
            vec!["/busy", "/fresh"],
            "the pin was scaffolding until the repo had a session; now recency rules"
        );
    }

    #[test]
    fn a_declared_empty_group_is_filtered_by_the_search_like_any_other() {
        let mut app = App::new();
        app.apply(Event::DeclareRepo("/dev/termherd".into()));

        app.apply(Event::SearchChanged("termherd".into()));
        assert_eq!(shown(&app), vec!["/dev/termherd"], "its path matches");

        app.apply(Event::SearchChanged("nothing-here".into()));
        assert!(
            shown(&app).is_empty(),
            "a declared repo is not exempt from the filter"
        );
    }

    #[test]
    fn declaring_and_forgetting_persist_the_overlay() {
        let mut app = App::new();
        let effects = app.apply(Event::DeclareRepo("/fresh".into()));
        assert!(matches!(effects.as_slice(), [Effect::SaveMetadata(o)]
                if o.repos.get("/fresh").is_some_and(|m| m.declared)));

        let effects = app.apply(Event::ForgetRepo("/fresh".into()));
        assert!(
            matches!(effects.as_slice(), [Effect::SaveMetadata(o)]
                if !o.repos.contains_key("/fresh")),
            "back to defaults, so the entry is dropped rather than persisted as noise"
        );
    }

    #[test]
    fn repo_paths_unite_the_scan_with_declarations_once_each() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![
            active("s1", "/scanned", 10),
            active("s2", "/both", 20),
        ]));
        app.apply(Event::DeclareRepo("/declared-only".into()));
        app.apply(Event::DeclareRepo("/both".into()));

        let mut paths = app.repo_paths();
        paths.sort_unstable();
        assert_eq!(paths, vec!["/both", "/declared-only", "/scanned"]);
    }

    #[test]
    fn repo_paths_ignore_the_search_box_and_the_archive_toggle() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![active("old", "/archived", 1)]));
        app.apply(Event::ToggleArchive("old".into()));
        app.apply(Event::DeclareRepo("/declared".into()));
        app.apply(Event::SearchChanged("matches-nothing".into()));

        let mut paths = app.repo_paths();
        paths.sort_unstable();
        assert_eq!(
            paths,
            vec!["/archived", "/declared"],
            "membership, not visibility"
        );
    }

    #[test]
    fn a_declaration_survives_the_save_that_records_it() {
        // `is_default` decides what gets written; a declaration that counted as
        // default would be deleted by its own save and gone at restart.
        let declared = crate::metadata::RepoMeta {
            starred: false,
            declared: true,
        };
        assert!(!declared.is_default());
    }

    #[test]
    fn searching_narrows_the_sidebar_without_reordering_it() {
        let mut app = App::new();
        // `/busy` leads on its newest session, and that is the session the
        // search excludes. Keyed on the *filtered* list, `/busy` would fall to
        // 1 and drop below `/quiet` as the user types — rows moving under the
        // cursor mid-search.
        let with_summary = |id: &str, path: &str, at: u64, summary: &str| {
            let mut r = active(id, path, at);
            r.digest.summary = summary.to_owned();
            r
        };
        app.apply(Event::ScanCompleted(vec![
            with_summary("fresh", "/busy", 100, "unrelated work"),
            with_summary("stale", "/busy", 1, "keepme, the old one"),
            with_summary("mid", "/quiet", 50, "keepme, the middle one"),
        ]));
        let before = shown(&app);
        assert_eq!(before, vec!["/busy", "/quiet"]);

        app.apply(Event::SearchChanged("keepme".into()));
        let filtered = app.visible_projects();
        assert_eq!(
            filtered.iter().map(|g| g.path.clone()).collect::<Vec<_>>(),
            before,
            "a search narrows the list, it does not rearrange it"
        );
        assert_eq!(filtered[0].sessions.len(), 1, "but it did narrow");
    }

    #[test]
    fn a_declared_repo_whose_sessions_are_all_archived_is_not_treated_as_fresh() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![
            active("old", "/declared", 1),
            active("newer", "/other", 50),
        ]));
        app.apply(Event::DeclareRepo("/declared".into()));
        app.apply(Event::ToggleArchive("old".into()));

        // The row survives — it is declared — but it is a repo with a history,
        // not one waiting for its first session, so it must not jump the queue.
        assert_eq!(
            shown(&app),
            vec!["/other", "/declared"],
            "an archived-away session is still a session"
        );
        assert!(
            app.repo_has_sessions("/declared"),
            "and the view can tell 'added, never used' from 'all filtered out'"
        );
    }

    #[test]
    fn a_declaration_and_a_star_are_independent() {
        let mut app = App::new();
        app.apply(Event::DeclareRepo("/fresh".into()));
        app.apply(Event::ToggleRepoStar("/fresh".into()));
        app.apply(Event::ForgetRepo("/fresh".into()));

        assert!(
            app.is_repo_starred("/fresh"),
            "the star outlives the forget"
        );
        assert!(!app.is_repo_declared("/fresh"));
        assert!(
            shown(&app).is_empty(),
            "a star is not a reason to show a repo with nothing in it"
        );
    }
}
