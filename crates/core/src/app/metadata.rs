//! Session/repo metadata overlay: rename, star/archive edits and their
//! persistence, plus the title and live/browsed lookups they feed.

use crate::browser::SessionRecord;
use crate::metadata::Overlay;
use termherd_claude::digest::SessionDigest;

use crate::title::{TitleSources, first_present};

use super::*;

impl App {
    /// Set (or clear, when blank) a session's custom title, persisting the
    /// overlay, and keep a live tab on this conversation in step with the
    /// sidebar: the tab re-resolves its title from the same sources.
    pub(super) fn rename_session(&mut self, session: String, title: String) -> Vec<Effect> {
        let trimmed = title.trim().to_owned();
        let claudes = self
            .record_for(&session)
            .and_then(|record| record.digest.custom_title.clone());
        let effects = self.update_meta(session, |meta| {
            meta.title = (!trimmed.is_empty()).then_some(trimmed);
            meta.title_over = meta.title.is_some().then_some(claudes).flatten();
        });
        self.retitle_tabs();
        effects
    }

    /// The title to show for a session: the user's custom title if set, else
    /// the one derived from the digest (`F-session-metadata`).
    #[must_use]
    pub fn session_title(&self, record: &SessionRecord) -> String {
        let (named, described) = self.recorded_titles(&record.session_id, Some(record));
        recorded_title(named, described)
    }

    /// The title a session would show without termherd's own name for it —
    /// what clearing that name leaves.
    #[must_use]
    pub fn session_title_unnamed_here(&self, record: &SessionRecord) -> String {
        recorded_title(
            record.digest.custom_title.as_deref(),
            described(&record.digest),
        )
    }

    /// The two tiers of [`TitleSources`] a scan and the metadata overlay
    /// supply for the Claude conversation `claude_id`: the name it was given,
    /// and what its transcript says it is about (Claude's AI title, else its
    /// first prompt).
    ///
    /// Of the two names — Claude's `/rename` and termherd's own title — the
    /// one given later wins. termherd's title stands while Claude's name is
    /// still the one it was given over (see [`SessionMeta::title_over`]),
    /// and fills in while Claude has none.
    pub(super) fn recorded_titles<'a>(
        &'a self,
        claude_id: &str,
        record: Option<&'a SessionRecord>,
    ) -> (Option<&'a str>, Option<&'a str>) {
        let meta = self.metadata.get(claude_id);
        let digest = record.map(|record| &record.digest);
        let claudes = digest.and_then(|d| d.custom_title.as_deref());
        let local = meta.and_then(|meta| meta.title.as_deref());
        let local_is_later = meta.is_some_and(|meta| meta.title_over.as_deref() == claudes);
        let named = if local_is_later {
            first_present([local, claudes])
        } else {
            first_present([claudes, local])
        };
        (named, digest.and_then(described))
    }

    /// Whether a session (by Claude id) is starred / archived.
    #[must_use]
    pub fn is_starred(&self, session_id: &str) -> bool {
        self.metadata.get(session_id).is_some_and(|m| m.starred)
    }

    #[must_use]
    pub fn is_archived(&self, session_id: &str) -> bool {
        self.metadata.get(session_id).is_some_and(|m| m.archived)
    }

    /// Whether a project (by real path) is starred (`F-favorites`, repo-level).
    #[must_use]
    pub fn is_repo_starred(&self, path: &str) -> bool {
        self.repos.get(path).is_some_and(|m| m.starred)
    }

    /// Whether a project (by real path) was added to the sidebar by hand
    /// (`F-repo-add`). True whether or not the scan also reports sessions for
    /// it — a declaration is not undone by a discovery.
    #[must_use]
    pub fn is_repo_declared(&self, path: &str) -> bool {
        self.repos.get(path).is_some_and(|m| m.declared)
    }

    /// The live session whose Claude conversation is `claude_id` (resumed,
    /// launched under a minted id, or re-keyed to it), if one is open. Lets the shell re-focus an existing terminal when its
    /// sidebar row is clicked again, rather than spawning a duplicate (FR4).
    #[must_use]
    pub fn open_session_for(&self, claude_id: &str) -> Option<SessionId> {
        self.sessions
            .values()
            .find(|s| s.claude_session_id() == Some(claude_id))
            .map(|s| s.id)
    }

    /// The Claude session id of the live pane `session`, as
    /// [`LiveSession::claude_session_id`] decides it. `None` for an unknown
    /// session, or one with no Claude conversation to name.
    #[must_use]
    pub fn claude_session_id(&self, session: SessionId) -> Option<&str> {
        self.sessions.get(&session)?.claude_session_id()
    }

    /// The browsed record for the conversation the live pane `session` holds:
    /// [`Self::claude_session_id`] resolved through [`Self::record_for`]. `None`
    /// for a shell, an unknown pane, or a conversation the last scan has not
    /// found yet.
    #[must_use]
    pub fn session_record(&self, session: SessionId) -> Option<&SessionRecord> {
        self.record_for(self.claude_session_id(session)?)
    }

    /// The browsed record for the Claude session `claude_id`, if the last scan
    /// found it. The inverse of [`Self::open_session_for`]: it maps a live tab
    /// back to the sidebar entry it resumes, so the tab hover can reuse the same
    /// session card the sidebar shows instead of a second derive. `None`
    /// for a shell or a fresh, not-yet-scanned session.
    #[must_use]
    pub fn record_for(&self, claude_id: &str) -> Option<&SessionRecord> {
        self.sidebar
            .projects
            .iter()
            .flat_map(|group| &group.sessions)
            .find(|record| record.session_id == claude_id)
    }

    /// Whether a session id is still on the scanned project list — the guard
    /// the archive confirmation uses against a session a rescan removed while
    /// the prompt was up. Exactly "the last scan has a record for it", so it
    /// tracks [`Self::record_for`].
    #[must_use]
    pub fn is_browsable(&self, session: &str) -> bool {
        self.record_for(session).is_some()
    }

    /// The full overlay to persist — both keyings, cloned as one unit so a save
    /// never drops the other map.
    pub(super) fn overlay(&self) -> Overlay {
        Overlay {
            sessions: self.metadata.clone(),
            repos: self.repos.clone(),
        }
    }

    /// Edit a session's metadata, dropping it when it returns to defaults, and
    /// emit the persistence effect.
    pub(super) fn update_meta(
        &mut self,
        session: String,
        edit: impl FnOnce(&mut SessionMeta),
    ) -> Vec<Effect> {
        let mut meta = self.metadata.get(&session).cloned().unwrap_or_default();
        edit(&mut meta);
        if meta.is_default() {
            self.metadata.remove(&session);
        } else {
            self.metadata.insert(session, meta);
        }
        vec![Effect::SaveMetadata(self.overlay())]
    }

    /// Edit a repo's metadata, dropping it when it returns to defaults, and
    /// emit the persistence effect. Mirrors [`Self::update_meta`].
    pub(super) fn update_repo_meta(
        &mut self,
        path: String,
        edit: impl FnOnce(&mut RepoMeta),
    ) -> Vec<Effect> {
        let mut meta = self.repos.get(&path).cloned().unwrap_or_default();
        edit(&mut meta);
        if meta.is_default() {
            self.repos.remove(&path);
        } else {
            self.repos.insert(path, meta);
        }
        vec![Effect::SaveMetadata(self.overlay())]
    }
}

/// What a transcript says its conversation is about: Claude's AI title, else
/// its first prompt.
fn described(digest: &SessionDigest) -> Option<&str> {
    first_present([digest.ai_title.as_deref(), Some(digest.summary.as_str())])
}

/// The sidebar title of the two recorded tiers, with no live or launch tier.
fn recorded_title(named: Option<&str>, described: Option<&str>) -> String {
    crate::title::resolve(&TitleSources {
        named,
        described,
        ..TitleSources::default()
    })
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::*;

    #[test]
    fn open_session_for_finds_a_live_resume_and_ignores_unknowns() {
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Claude(ClaudeLaunch::Resume("abc-123".into())),
            title: "proj".into(),
            placement: Placement::Foreground,
        }));
        let id = app.workspace.focused_session().expect("a focused session");
        assert_eq!(app.open_session_for("abc-123"), Some(id));
        assert_eq!(app.open_session_for("not-open"), None);
    }

    #[test]
    fn record_for_maps_a_claude_id_back_to_its_browsed_record() {
        // A live tab's resume id resolves to the sidebar record, so the
        // tab hover can reuse the same session card.
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![
            record("abc-123", "/proj", "fix the login bug"),
            record("def-456", "/other", "write the docs"),
        ]));
        assert_eq!(
            app.record_for("def-456").map(|r| r.project_path.as_str()),
            Some("/other")
        );
        assert_eq!(
            app.record_for("abc-123").map(|r| r.digest.summary.as_str()),
            Some("fix the login bug")
        );
        // A shell / fresh session id has no browsed record.
        assert!(app.record_for("not-scanned").is_none());
    }

    #[test]
    fn is_browsable_tracks_the_scanned_list() {
        // The archive-confirm guard: a session is browsable iff the last scan
        // still lists it. A rescan that drops it must un-browse it.
        let mut app = App::new();
        assert!(!app.is_browsable("abc"), "empty app browses nothing");

        app.apply(Event::ScanCompleted(vec![record("abc", "/p", "hi")]));
        assert!(app.is_browsable("abc"), "a scanned session is browsable");
        assert!(!app.is_browsable("gone"), "an unscanned id is not");

        // A rescan without it drops it from the browsable set.
        app.apply(Event::ScanCompleted(vec![]));
        assert!(
            !app.is_browsable("abc"),
            "a session a rescan removed is no longer browsable"
        );
    }

    #[test]
    fn star_pins_a_session_and_persists_metadata() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![
            record("a", "/p", "first"),
            record("b", "/p", "second"),
        ]));
        // "b" is most-recent-first by mtime equal → group order; star "a".
        let effects = app.apply(Event::ToggleStar("a".into()));
        assert!(matches!(effects.as_slice(), [Effect::SaveMetadata(m)] if m.sessions["a"].starred));
        // Starred session now leads its group.
        let group = &app.visible_projects()[0];
        assert_eq!(group.sessions[0].session_id, "a");
        assert!(app.is_starred("a"));
    }

    #[test]
    fn star_pins_a_repo_to_the_top_and_persists() {
        let mut app = App::new();
        // Equal (missing) mtimes → groups fall back to path order: `/busy` first.
        app.apply(Event::ScanCompleted(vec![
            record("q", "/quiet", "q1"),
            record("b", "/busy", "b1"),
        ]));
        assert_eq!(app.visible_projects()[0].path, "/busy");

        // Starring the second repo pins it to the top of the sidebar.
        let effects = app.apply(Event::ToggleRepoStar("/quiet".into()));
        assert!(
            matches!(effects.as_slice(), [Effect::SaveMetadata(m)] if m.repos["/quiet"].starred)
        );
        assert!(app.is_repo_starred("/quiet"));
        let paths: Vec<_> = app
            .visible_projects()
            .iter()
            .map(|g| g.path.clone())
            .collect();
        assert_eq!(paths, vec!["/quiet", "/busy"]);
    }

    #[test]
    fn unstarring_a_repo_drops_its_entry() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record("a", "/p", "only")]));
        app.apply(Event::ToggleRepoStar("/p".into()));
        assert!(app.is_repo_starred("/p"));
        // Toggling back to the default drops the entry rather than persisting it.
        let effects = app.apply(Event::ToggleRepoStar("/p".into()));
        assert!(
            matches!(effects.as_slice(), [Effect::SaveMetadata(m)] if !m.repos.contains_key("/p"))
        );
        assert!(!app.is_repo_starred("/p"));
    }

    #[test]
    fn favorites_aggregate_starred_sessions_across_projects_most_recent_first() {
        let mut app = App::new();
        let mut newer = record("new", "/a", "recent");
        newer.modified = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(100));
        let mut older = record("old", "/b", "stale");
        older.modified = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(10));
        app.apply(Event::ScanCompleted(vec![
            newer,
            older,
            record("plain", "/a", "unstarred"),
        ]));
        app.apply(Event::ToggleStar("new".into()));
        app.apply(Event::ToggleStar("old".into()));

        let groups = app.visible_projects();
        let favs = app.favorite_sessions(&groups);
        let ids: Vec<_> = favs.iter().map(|(_, s)| s.session_id.as_str()).collect();
        assert_eq!(ids, vec!["new", "old"], "cross-project, most-recent-first");
        // Each favourite carries its project path so the row can resume it.
        assert_eq!(favs[0].0, "/a");
        assert_eq!(favs[1].0, "/b");
    }

    #[test]
    fn favorites_are_empty_without_stars() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record("a", "/p", "x")]));
        let groups = app.visible_projects();
        assert!(app.favorite_sessions(&groups).is_empty());
    }

    #[test]
    fn an_archived_starred_session_is_not_a_visible_favorite() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record("a", "/p", "x")]));
        app.apply(Event::ToggleStar("a".into()));
        app.apply(Event::ToggleArchive("a".into()));
        // Hidden by default, so it drops out of the visible groups favorites read.
        let groups = app.visible_projects();
        assert!(app.favorite_sessions(&groups).is_empty());
        // …but it returns once archived sessions are shown.
        app.apply(Event::ShowArchivedToggled(true));
        let groups = app.visible_projects();
        assert_eq!(app.favorite_sessions(&groups).len(), 1);
    }

    #[test]
    fn archived_sessions_hide_unless_shown() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![
            record("a", "/p", "keep"),
            record("b", "/p", "hideme"),
        ]));
        app.apply(Event::ToggleArchive("b".into()));
        // Hidden by default…
        let visible = app.visible_projects();
        assert_eq!(visible[0].sessions.len(), 1);
        assert_eq!(visible[0].sessions[0].session_id, "a");
        // …shown when the toggle is on.
        app.apply(Event::ShowArchivedToggled(true));
        assert_eq!(app.visible_projects()[0].sessions.len(), 2);
    }

    #[test]
    fn archiving_the_only_session_drops_the_empty_group() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record("a", "/solo", "only")]));
        app.apply(Event::ToggleArchive("a".into()));
        assert!(app.visible_projects().is_empty());
    }

    #[test]
    fn rename_overrides_the_title_and_clearing_restores_it() {
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record(
            "a",
            "/p",
            "derived summary",
        )]));
        let derived = app.session_title(&app.sidebar.projects[0].sessions[0].clone());

        app.apply(Event::RenameSession {
            session: "a".into(),
            title: "  My Title  ".into(),
        });
        assert_eq!(
            app.session_title(&app.sidebar.projects[0].sessions[0].clone()),
            "My Title"
        );

        // Clearing (empty title) drops the entry back to the derived title.
        let effects = app.apply(Event::RenameSession {
            session: "a".into(),
            title: "   ".into(),
        });
        assert!(
            matches!(effects.as_slice(), [Effect::SaveMetadata(m)] if !m.sessions.contains_key("a"))
        );
        assert_eq!(
            app.session_title(&app.sidebar.projects[0].sessions[0].clone()),
            derived
        );
    }

    #[test]
    fn renaming_a_session_retitles_its_open_tab_and_clearing_restores_the_name() {
        // Follow-up: a sidebar rename must retitle the live tab too, not
        // just the sidebar row — and clearing it restores the digest name.
        let mut app = App::new();
        app.apply(Event::ScanCompleted(vec![record(
            "a",
            "/p",
            "derived summary",
        )]));
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/p".into()),
            launch: Launch::Claude(ClaudeLaunch::Resume("a".into())),
            title: "derived summary".into(),
            placement: Placement::Foreground,
        }));
        let session = app.workspace.focused_session().expect("a launched tab");

        app.apply(Event::RenameSession {
            session: "a".into(),
            title: "My Title".into(),
        });
        assert_eq!(
            app.workspace.session_title(session),
            Some("My Title"),
            "a sidebar rename retitles the open tab"
        );

        app.apply(Event::RenameSession {
            session: "a".into(),
            title: "  ".into(),
        });
        assert_eq!(
            app.workspace.session_title(session),
            Some("derived summary"),
            "clearing the rename restores the digest name on the open tab"
        );
    }

    #[test]
    fn the_title_unnamed_here_is_what_clearing_the_sidebar_name_leaves() {
        let mut app = App::new();
        let mut r = record("abc", "/repo", "the first prompt");
        r.digest.custom_title = Some("claude's".into());
        app.apply(Event::ScanCompleted(vec![r.clone()]));
        app.apply(Event::RenameSession {
            session: "abc".into(),
            title: "mine".into(),
        });
        assert_eq!(app.session_title(&r), "mine");
        assert_eq!(app.session_title_unnamed_here(&r), "claude's");

        app.apply(Event::RenameSession {
            session: "abc".into(),
            title: String::new(),
        });
        assert_eq!(app.session_title(&r), "claude's");
    }
}
