//! Session/repo metadata overlay: rename, star/archive edits and their
//! persistence, plus the title and live/browsed lookups they feed.

use crate::browser::SessionRecord;
use crate::metadata::Overlay;

use super::*;

impl App {
    /// Set (or clear, when blank) a session's custom title, persisting the
    /// overlay, and keep a live tab resuming this id in step with the sidebar.
    /// A non-empty rename wins directly; clearing restores the digest-derived
    /// name when the session is still in the last scan.
    pub(super) fn rename_session(&mut self, session: String, title: String) -> Vec<Effect> {
        let trimmed = title.trim().to_owned();
        let effects = self.update_meta(session.clone(), |meta| {
            meta.title = (!trimmed.is_empty()).then(|| trimmed.clone());
        });
        if let Some(live) = self.open_session_for(&session) {
            let next = if trimmed.is_empty() {
                self.record_for(&session)
                    .map(|record| self.session_title(record))
                    .filter(|name| !name.trim().is_empty())
            } else {
                Some(trimmed)
            };
            if let Some(next) = next {
                self.workspace.set_session_title(live, next);
            }
        }
        effects
    }

    /// The title to show for a session: the user's custom title if set, else
    /// the one derived from the digest (`F-session-metadata`).
    #[must_use]
    pub fn session_title(&self, record: &SessionRecord) -> String {
        self.metadata
            .get(&record.session_id)
            .and_then(|meta| meta.title.clone())
            .unwrap_or_else(|| record.digest.display_title(None).to_owned())
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

    /// The live session currently resuming the Claude session `claude_id`, if
    /// one is open. Lets the shell re-focus an existing terminal when its
    /// sidebar row is clicked again, rather than spawning a duplicate (FR4).
    #[must_use]
    pub fn open_session_for(&self, claude_id: &str) -> Option<SessionId> {
        self.sessions
            .values()
            .find(|s| s.launch.resume_id() == Some(claude_id))
            .map(|s| s.id)
    }

    /// The browsed record for the Claude session `claude_id`, if the last scan
    /// found it. The inverse of [`Self::open_session_for`]: it maps a live tab
    /// back to the sidebar entry it resumes, so the tab hover can reuse the same
    /// session card the sidebar shows instead of a second derive. `None`
    /// for a shell or a fresh, not-yet-scanned session.
    #[must_use]
    pub fn record_for(&self, claude_id: &str) -> Option<&SessionRecord> {
        self.projects
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testsupport::*;

    #[test]
    fn open_session_for_finds_a_live_resume_and_ignores_unknowns() {
        let mut app = App::new();
        app.apply(Event::LaunchSession(LaunchSpec {
            cwd: Some("/proj".into()),
            launch: Launch::Claude {
                resume: Some("abc-123".into()),
            },
            title: "proj".into(),
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
        let derived = app.session_title(&app.projects[0].sessions[0].clone());

        app.apply(Event::RenameSession {
            session: "a".into(),
            title: "  My Title  ".into(),
        });
        assert_eq!(
            app.session_title(&app.projects[0].sessions[0].clone()),
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
            app.session_title(&app.projects[0].sessions[0].clone()),
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
            launch: Launch::Claude {
                resume: Some("a".into()),
            },
            title: "derived summary".into(),
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
}
